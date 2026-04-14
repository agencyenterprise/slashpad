import { query, listSessions, getSessionMessages, renameSession, tagSession } from "@anthropic-ai/claude-agent-sdk";
import { createInterface } from "readline";

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
            toolEvents.push({ type: "tool_start", tool: block.name, args: block.input, timestamp: Date.now() });
          }
        }
        if (content || toolEvents.length > 0) {
          emit({ type: "chat_message", role: "assistant", content, toolEvents, timestamp: Date.now() });
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
  let emittedText = false;

  const options = {
    cwd: cwd || process.env.HOME,
    // Use the claude_code preset. `settingSources` controls which on-disk
    // Claude settings the SDK loads:
    //   - ["project"] always loads ~/.launchpad/CLAUDE.md (seeded from
    //     bundled-prompts/CLAUDE.md by Rust), which is how Launchpad's
    //     system prompt is customized.
    //   - ["user", "project"] additionally loads ~/.claude/ — the user's
    //     personal CLAUDE.md, skills, and hooks — when the Settings
    //     "Load user-level Claude settings & skills" checkbox is on.
    systemPrompt: { type: "preset", preset: "claude_code" },
    allowedTools: ["Read", "Write", "Bash", "Glob", "Grep", "Skill"],
    settingSources: payload.loadUserSettings ? ["user", "project"] : ["project"],
    permissionMode: "bypassPermissions",
    allowDangerouslySkipPermissions: true,
    maxTurns: 10,
  };

  if (sessionId) {
    options.resume = sessionId;
  }

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
          tagSession(sessionId, "launchpad", { dir }).catch(() => {});
          renameSession(sessionId, title, { dir }).catch(() => {});
          isFirstTurn = false;
        }
      }

      if ("result" in message) {
        if (message.result && !emittedText) {
          emit({ type: "text_delta", delta: message.result, timestamp: Date.now() });
        }
        emit({ type: "complete", timestamp: Date.now() });
      } else if (message.type === "assistant") {
        for (const block of message.message?.content ?? []) {
          if (block.type === "text" && block.text) {
            emit({ type: "text_delta", delta: block.text, timestamp: Date.now() });
            emittedText = true;
          } else if (block.type === "tool_use") {
            emit({ type: "tool_start", tool: block.name, args: block.input, timestamp: Date.now() });
            emit({ type: "tool_end", tool: block.name, timestamp: Date.now() });
          }
        }
      }
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
