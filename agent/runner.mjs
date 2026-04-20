import { query, listSessions, getSessionMessages, tagSession } from "@anthropic-ai/claude-agent-sdk";
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
        tag: s.tag ?? null,
      });
    }
    emit({ type: "complete", timestamp: Date.now() });
  } catch (e) {
    emit({ type: "error", error: e.message || String(e), timestamp: Date.now() });
  }
  process.exit(0);
}

if (mode === "tag") {
  try {
    await tagSession(payload.sessionId, payload.tag ?? null, {
      dir: payload.cwd || process.env.HOME,
    });
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
    // Collect tool_result blocks from user messages so we can retroactively
    // attach them to the assistant message that invoked the tool, keyed by
    // tool_use_id. This lets historical views show which tool calls errored.
    const pendingToolResults = new Map(); // tool_use_id -> { content, isError }
    for (const msg of messages) {
      if (msg.type === "user") {
        const message = msg.message;
        let content = "";
        let hasToolResult = false;
        if (typeof message === "string") {
          content = message;
        } else if (message?.content) {
          // MessageParam format: content can be string or array of content blocks
          if (typeof message.content === "string") {
            content = message.content;
          } else if (Array.isArray(message.content)) {
            for (const block of message.content) {
              if (block?.type === "text" && typeof block.text === "string") {
                content += block.text;
              } else if (block?.type === "tool_result" && block.tool_use_id) {
                hasToolResult = true;
                let resultText = "";
                if (typeof block.content === "string") {
                  resultText = block.content;
                } else if (Array.isArray(block.content)) {
                  resultText = block.content
                    .filter((b) => b?.type === "text" && typeof b.text === "string")
                    .map((b) => b.text)
                    .join("\n");
                }
                pendingToolResults.set(block.tool_use_id, {
                  content: resultText,
                  isError: block.is_error === true,
                });
              }
            }
          }
        }
        // Only emit a user chat_message for real user input — skip synthetic
        // messages that are purely tool_result envelopes.
        if (content && !hasToolResult) {
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
            const pending = pendingToolResults.get(block.id);
            toolEvents.push({
              type: "tool_end",
              tool: block.name,
              toolUseId: block.id,
              args: block.input,
              result: pending?.content ?? null,
              timestamp: Date.now(),
            });
            if (pending) {
              toolEvents.push({
                type: "tool_result",
                toolUseId: block.id,
                content: pending.content,
                isError: pending.isError,
                timestamp: Date.now(),
              });
              pendingToolResults.delete(block.id);
            }
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
    // --- Permissions ---
    //
    // The Agent SDK evaluates a tool call in this order (see
    // https://code.claude.com/en/agent-sdk/permissions):
    //   1. Hooks
    //   2. Deny rules (disallowedTools + settings.json deny)
    //   3. Permission mode — "auto" uses a model classifier to
    //      approve or deny each tool call
    //   4. Allow rules (allowedTools + settings.json allow)
    //   5. canUseTool callback
    //
    // We use "auto" mode so the model classifier handles approvals
    // without blanket bypass. disallowedTools still blocks tools
    // that don't belong in a command palette.
    disallowedTools: ["EnterPlanMode", "ExitPlanMode", "AskUserQuestion"],
    // Widen the filesystem scope beyond cwd so Claude can stage in
    // /tmp (skill-creator does this) and reach the user's other
    // project directories without tripping the scope check.
    additionalDirectories: [
      process.env.HOME,
      "/tmp",
      "/private/tmp", // macOS resolves /tmp to /private/tmp in some contexts
    ].filter(Boolean),
    // `settingSources` controls which on-disk Claude settings the SDK
    // loads (this is also where deny rules from .claude/settings.json
    // would come from):
    //   - ["project"] always loads ~/.slashpad/CLAUDE.md (seeded from
    //     bundled-prompts/CLAUDE.md by Rust), which is how Slashpad's
    //     system prompt is customized.
    //   - ["user", "project"] additionally loads ~/.claude/ — the user's
    //     personal CLAUDE.md, skills, and hooks — when the Settings
    //     "Load user-level Claude settings & skills" checkbox is on.
    settingSources: payload.loadUserSettings ? ["user", "project"] : ["project"],
    permissionMode: "auto",
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
  const toolIds = new Map();     // index -> tool_use_id (for linking tool_result events)

  try {
    for await (const message of query({ prompt: userPrompt, options })) {
      // Emit the session id as soon as the SDK exposes it (first yielded
      // message — e.g. the `system` init — already carries it). Rust
      // needs this early so it can resume the session if the user
      // interrupts the turn before it completes.
      if (message.session_id && message.session_id !== sessionId) {
        sessionId = message.session_id;
        emit({ type: "session_id", sessionId, timestamp: Date.now() });
      }

      if (message.type === "stream_event") {
        const event = message.event;

        if (event.type === "content_block_start") {
          const cb = event.content_block;
          if (cb.type === "tool_use") {
            toolNames.set(event.index, cb.name);
            toolInputs.set(event.index, "");
            toolIds.set(event.index, cb.id);
            emit({ type: "tool_start", tool: cb.name, toolUseId: cb.id, timestamp: Date.now() });
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
            const toolUseId = toolIds.get(event.index);
            emit({ type: "tool_end", tool: name, toolUseId, args, timestamp: Date.now() });
            toolNames.delete(event.index);
            toolInputs.delete(event.index);
            toolIds.delete(event.index);
          }
        }
      } else if (message.type === "user") {
        // Tool results come back as `user` messages with tool_result blocks
        // (type: "tool_result", tool_use_id, content, is_error). Forward
        // them so the UI can render errored tool calls distinctly instead
        // of showing a misleading "✓" for blocked/failed operations.
        const blocks = message.message?.content;
        if (Array.isArray(blocks)) {
          for (const block of blocks) {
            if (block?.type !== "tool_result") continue;
            let resultText = "";
            if (typeof block.content === "string") {
              resultText = block.content;
            } else if (Array.isArray(block.content)) {
              resultText = block.content
                .filter((b) => b?.type === "text" && typeof b.text === "string")
                .map((b) => b.text)
                .join("\n");
            }
            emit({
              type: "tool_result",
              toolUseId: block.tool_use_id,
              content: resultText,
              isError: block.is_error === true,
              timestamp: Date.now(),
            });
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
