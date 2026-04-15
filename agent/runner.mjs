import { query, listSessions, getSessionMessages, renameSession, tagSession } from "@anthropic-ai/claude-agent-sdk";
import { createInterface } from "readline";
import { existsSync } from "fs";

function findClaudePath() {
  const home = process.env.HOME || "/tmp";
  const candidates = [
    `${home}/.local/bin/claude`,
    "/usr/local/bin/claude",
    "/opt/homebrew/bin/claude",
  ];
  for (const p of candidates) {
    if (existsSync(p)) return p;
  }
  return undefined;
}

function emit(event) {
  process.stdout.write(JSON.stringify(event) + "\n");
}

const payload = JSON.parse(Buffer.from(process.argv[2], "base64").toString());

if (payload.apiKey) {
  process.env.ANTHROPIC_API_KEY = payload.apiKey;
}

const mode = payload.mode || "chat";

if (mode === "list") {
  try {
    const all = await listSessions({ dir: payload.cwd || process.env.HOME });
    const sessions = all.slice(0, 50);
    for (const s of sessions) {
      emit({
        type: "session",
        sessionId: s.sessionId,
        summary: s.summary || s.firstPrompt || "Untitled",
        lastModified: s.lastModified,
        firstPrompt: s.firstPrompt,
      });
    }
    emit({ type: "complete", timestamp: Date.now() });
  } catch (e) {
    emit({ type: "error", error: e.message || String(e), timestamp: Date.now() });
  }
  process.exit(0);
}

if (mode === "messages") {
  try {
    const messages = await getSessionMessages(payload.sessionId, {
      dir: payload.cwd || process.env.HOME,
      includeSystemMessages: false,
    });
    let lastUserTimestamp = null;
    for (const msg of messages) {
      if (msg.type === "user") {
        // Extract text content from user message
        const message = msg.message;
        let content = "";
        if (typeof message === "string") {
          content = message;
        } else if (message?.content) {
          // MessageParam format: content can be string or array of content blocks
          if (typeof message.content === "string") {
            content = message.content;
          } else if (Array.isArray(message.content)) {
            content = message.content
              .filter((b) => b.type === "text")
              .map((b) => b.text)
              .join("");
          }
        }
        if (content) {
          emit({ type: "chat_message", role: "user", content, timestamp: Date.now() });
          lastUserTimestamp = msg.timestamp ? new Date(msg.timestamp).getTime() : null;
        }
      } else if (msg.type === "assistant") {
        const message = msg.message;
        let content = "";
        const toolEvents = [];
        const blocks = message?.content ?? [];
        for (const block of blocks) {
          if (block.type === "text" && block.text) {
            content += block.text;
          } else if (block.type === "tool_use") {
            toolEvents.push({ type: "tool_end", tool: block.name, args: block.input, timestamp: Date.now() });
          }
        }
        if (content || toolEvents.length > 0) {
          const assistantTs = msg.timestamp ? new Date(msg.timestamp).getTime() : null;
          const durationMs = (lastUserTimestamp && assistantTs) ? (assistantTs - lastUserTimestamp) : null;
          emit({ type: "chat_message", role: "assistant", content, toolEvents, durationMs, timestamp: Date.now() });
        }
      }
    }
    emit({ type: "complete", timestamp: Date.now() });
  } catch (e) {
    emit({ type: "error", error: e.message || String(e), timestamp: Date.now() });
  }
  process.exit(0);
}

// Chat mode — long-lived process
const { prompt, cwd } = payload;
let sessionId = payload.resume || null;
let isFirstTurn = true;

async function runTurn(userPrompt) {
  emit({ type: "turn_start", timestamp: Date.now() });
  let emittedText = false;

  const claudePath = findClaudePath();
  const options = {
    cwd: cwd || process.env.HOME,
    ...(claudePath && { pathToClaudeCodeExecutable: claudePath }),
    // Use the claude_code preset. `settingSources` controls which on-disk
    // Claude settings the SDK loads:
    //   - ["project"] always loads ~/.slashpad/CLAUDE.md (seeded from
    //     bundled-prompts/CLAUDE.md by Rust), which is how Slashpad's
    //     system prompt is customized.
    //   - ["user", "project"] additionally loads ~/.claude/ — the user's
    //     personal CLAUDE.md, skills, and hooks — when the Settings
    //     "Load user-level Claude settings & skills" checkbox is on.
    systemPrompt: { type: "preset", preset: "claude_code" },
    allowedTools: ["Read", "Write", "Bash", "Glob", "Grep", "Skill"],
    settingSources: payload.loadUserSettings ? ["user", "project"] : ["project"],
    permissionMode: "bypassPermissions",
    allowDangerouslySkipPermissions: true,
    includePartialMessages: true,
  };

  if (sessionId) {
    options.resume = sessionId;
  }

  // Track active tool_use blocks by content-block index so we can
  // accumulate streamed input_json_delta chunks and emit tool_end
  // with the complete parsed args once the block closes.
  const toolNames = new Map();   // index -> tool name
  const toolInputs = new Map();  // index -> accumulated JSON string

  try {
    for await (const message of query({ prompt: userPrompt, options })) {
      // Emit the session id as soon as the SDK exposes it (first yielded
      // message — e.g. the `system` init — already carries it). Rust
      // needs this early so it can resume the session if the user
      // interrupts the turn before it completes.
      if (message.session_id && message.session_id !== sessionId) {
        sessionId = message.session_id;
        emit({ type: "session_id", sessionId, timestamp: Date.now() });

        if (isFirstTurn && !payload.resume) {
          const dir = cwd || process.env.HOME;
          const title = userPrompt.length > 80 ? userPrompt.slice(0, 77) + "..." : userPrompt;
          tagSession(sessionId, "slashpad", { dir }).catch(() => {});
          renameSession(sessionId, title, { dir }).catch(() => {});
          isFirstTurn = false;
        }
      }

      if (message.type === "stream_event") {
        const event = message.event;

        if (event.type === "content_block_start") {
          const cb = event.content_block;
          if (cb.type === "tool_use") {
            toolNames.set(event.index, cb.name);
            toolInputs.set(event.index, "");
            emit({ type: "tool_start", tool: cb.name, timestamp: Date.now() });
          }
        } else if (event.type === "content_block_delta") {
          const delta = event.delta;
          if (delta.type === "text_delta") {
            emit({ type: "text_delta", delta: delta.text, timestamp: Date.now() });
            emittedText = true;
          } else if (delta.type === "input_json_delta") {
            const prev = toolInputs.get(event.index) || "";
            toolInputs.set(event.index, prev + delta.partial_json);
          }
        } else if (event.type === "content_block_stop") {
          const name = toolNames.get(event.index);
          if (name) {
            let args = {};
            try { args = JSON.parse(toolInputs.get(event.index) || "{}"); } catch {}
            emit({ type: "tool_end", tool: name, args, timestamp: Date.now() });
            toolNames.delete(event.index);
            toolInputs.delete(event.index);
          }
        }
      } else if ("result" in message) {
        if (message.result && !emittedText) {
          emit({ type: "text_delta", delta: message.result, timestamp: Date.now() });
        }
        emit({
          type: "complete",
          durationMs: message.duration_ms ?? null,
          numTurns: message.num_turns ?? null,
          totalCostUsd: message.total_cost_usd ?? null,
          timestamp: Date.now(),
        });
      }
      // AssistantMessage is skipped — content already streamed above.
    }

  } catch (e) {
    emit({ type: "error", error: e.message || String(e), timestamp: Date.now() });
  }

  emit({ type: "ready", timestamp: Date.now() });
}

// Run first turn
try {
  await runTurn(prompt);
} catch (e) {
  emit({ type: "error", error: e.message || String(e), timestamp: Date.now() });
  process.exit(1);
}

// Listen for follow-up messages on stdin
const rl = createInterface({ input: process.stdin });

rl.on("line", async (line) => {
  if (!line.trim()) return;
  try {
    const msg = JSON.parse(line);
    if (msg.type === "close") {
      process.exit(0);
    }
    if (msg.type === "message" && msg.content) {
      await runTurn(msg.content);
    }
  } catch {
    // Skip unparseable lines
  }
});

rl.on("close", () => {
  process.exit(0);
});
