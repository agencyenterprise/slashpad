/**
 * Agent runner that executes Claude sessions with tool use.
 *
 * Architecture:
 * - Calls the Anthropic Messages API directly with streaming
 * - Manages the agentic tool-use loop (call → tool_use → tool_result → call...)
 * - Composio tools are registered as tool definitions
 * - Auth comes from Claude CLI config or env var
 *
 * For MVP, tools are defined but stubbed — Composio integration is wired in
 * but actual tool execution requires the Composio SDK in a sidecar (Phase 2).
 */

import { invoke } from "@tauri-apps/api/core";
import { Command } from "@tauri-apps/plugin-shell";
import type { ToolEvent, Session, SessionStatus } from "./types";

const API_URL = "https://api.anthropic.com/v1/messages";
const MODEL = "claude-sonnet-4-20250514";

type EventCallback = (event: ToolEvent) => void;

interface ToolDef {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
}

interface ContentBlock {
  type: string;
  text?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
}

interface Message {
  role: "user" | "assistant";
  content: string | ContentBlock[];
}

/**
 * Try to read the API key from multiple sources:
 * 1. localStorage (user-configured in settings)
 * 2. Claude CLI config file (via Tauri shell)
 */
export async function getApiKey(): Promise<string | null> {
  // Check localStorage first
  const stored = localStorage.getItem("launchpad_api_key");
  if (stored) return stored;

  // Try reading from Claude CLI config
  try {
    const cmd = Command.create("claude-cli", ["config", "get", "apiKey"]);
    const output = await cmd.execute();
    if (output.code === 0 && output.stdout.trim()) {
      return output.stdout.trim();
    }
  } catch {
    // CLI not available or no key configured
  }

  // Try environment variable via Rust
  try {
    const cmd = Command.create("claude-cli", [
      "-c",
      'echo $ANTHROPIC_API_KEY',
    ]);
    const output = await cmd.execute();
    if (output.code === 0 && output.stdout.trim()) {
      return output.stdout.trim();
    }
  } catch {}

  return null;
}

export function setApiKey(key: string) {
  localStorage.setItem("launchpad_api_key", key);
}

/**
 * Define the tools available to the agent.
 * For MVP, we include general-purpose tools.
 * Composio tools get merged in based on the skill's `tools` array.
 */
function getToolDefinitions(_requestedTools: string[]): ToolDef[] {
  // Base tools always available
  const tools: ToolDef[] = [
    {
      name: "run_command",
      description:
        "Execute a shell command on the user's system and return the output. Use for git, file operations, and system tasks.",
      input_schema: {
        type: "object",
        properties: {
          command: {
            type: "string",
            description: "The shell command to execute",
          },
        },
        required: ["command"],
      },
    },
    {
      name: "read_file",
      description: "Read the contents of a file at the given path.",
      input_schema: {
        type: "object",
        properties: {
          path: { type: "string", description: "Absolute file path" },
        },
        required: ["path"],
      },
    },
    {
      name: "write_file",
      description: "Write content to a file, creating it if it doesn't exist.",
      input_schema: {
        type: "object",
        properties: {
          path: { type: "string", description: "Absolute file path" },
          content: { type: "string", description: "File content to write" },
        },
        required: ["path", "content"],
      },
    },
  ];

  // TODO: Phase 2 — merge Composio tool definitions based on requestedTools
  // e.g. if requestedTools includes "composio:gmail", fetch Gmail tool defs
  // from Composio SDK and append them

  return tools;
}

/**
 * Execute a tool call locally.
 * Returns the tool result as a string.
 */
async function executeTool(
  name: string,
  input: Record<string, unknown>
): Promise<string> {
  switch (name) {
    case "run_command": {
      try {
        // Split command for shell execution
        const cmd = Command.create("claude-cli", [
          "-c",
          input.command as string,
        ]);
        const output = await cmd.execute();
        return output.stdout + (output.stderr ? `\nSTDERR: ${output.stderr}` : "");
      } catch (e: any) {
        return `Error: ${e.message}`;
      }
    }

    case "read_file": {
      try {
        const cmd = Command.create("claude-cli", [
          "-c",
          `cat "${input.path}"`,
        ]);
        const output = await cmd.execute();
        return output.stdout;
      } catch (e: any) {
        return `Error reading file: ${e.message}`;
      }
    }

    case "write_file": {
      try {
        const cmd = Command.create("claude-cli", [
          "-c",
          `cat > "${input.path}" << 'LAUNCHPAD_EOF'\n${input.content}\nLAUNCHPAD_EOF`,
        ]);
        const output = await cmd.execute();
        return `File written to ${input.path}`;
      } catch (e: any) {
        return `Error writing file: ${e.message}`;
      }
    }

    default:
      return `Unknown tool: ${name}`;
  }
}

/**
 * Run a streaming agent session.
 *
 * This implements the full agentic loop:
 * 1. Send prompt to Claude with tools
 * 2. If Claude responds with tool_use, execute the tool and continue
 * 3. If Claude responds with text (stop_reason=end_turn), we're done
 * 4. Stream text deltas to the callback as they arrive
 */
export async function runSession(
  prompt: string,
  systemPrompt: string,
  tools: string[],
  onEvent: EventCallback
): Promise<string> {
  const apiKey = await getApiKey();
  if (!apiKey) {
    onEvent({
      type: "error",
      error: "No API key found. Set one in settings or run `claude login`.",
      timestamp: Date.now(),
    });
    return "";
  }

  const toolDefs = getToolDefinitions(tools);
  const messages: Message[] = [{ role: "user", content: prompt }];
  let fullResult = "";
  let loopCount = 0;
  const MAX_LOOPS = 10;

  while (loopCount < MAX_LOOPS) {
    loopCount++;

    // Make streaming API call
    const response = await fetch(API_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": "2023-06-01",
      },
      body: JSON.stringify({
        model: MODEL,
        max_tokens: 4096,
        system: systemPrompt,
        messages,
        tools: toolDefs.length > 0 ? toolDefs : undefined,
        stream: true,
      }),
    });

    if (!response.ok) {
      const err = await response.text();
      onEvent({
        type: "error",
        error: `API error ${response.status}: ${err}`,
        timestamp: Date.now(),
      });
      return fullResult;
    }

    // Parse SSE stream
    const reader = response.body!.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let currentToolUse: {
      id: string;
      name: string;
      inputJson: string;
    } | null = null;
    let stopReason: string | null = null;
    const contentBlocks: ContentBlock[] = [];

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.startsWith("data: ")) continue;
        const data = line.slice(6).trim();
        if (data === "[DONE]") continue;

        try {
          const event = JSON.parse(data);

          switch (event.type) {
            case "content_block_start":
              if (event.content_block?.type === "tool_use") {
                currentToolUse = {
                  id: event.content_block.id,
                  name: event.content_block.name,
                  inputJson: "",
                };
                onEvent({
                  type: "tool_start",
                  tool: event.content_block.name,
                  timestamp: Date.now(),
                });
              }
              break;

            case "content_block_delta":
              if (event.delta?.type === "text_delta") {
                const text = event.delta.text;
                fullResult += text;
                onEvent({
                  type: "text_delta",
                  delta: text,
                  timestamp: Date.now(),
                });
              } else if (event.delta?.type === "input_json_delta") {
                if (currentToolUse) {
                  currentToolUse.inputJson += event.delta.partial_json;
                }
              }
              break;

            case "content_block_stop":
              if (currentToolUse) {
                const toolInput = JSON.parse(currentToolUse.inputJson || "{}");
                contentBlocks.push({
                  type: "tool_use",
                  id: currentToolUse.id,
                  name: currentToolUse.name,
                  input: toolInput,
                });

                // Execute the tool
                const result = await executeTool(
                  currentToolUse.name,
                  toolInput
                );
                onEvent({
                  type: "tool_end",
                  tool: currentToolUse.name,
                  args: toolInput,
                  result:
                    result.length > 200
                      ? result.slice(0, 200) + "..."
                      : result,
                  timestamp: Date.now(),
                });

                // We'll add tool results after the full response
                currentToolUse = null;
              }
              break;

            case "message_delta":
              if (event.delta?.stop_reason) {
                stopReason = event.delta.stop_reason;
              }
              break;
          }
        } catch {
          // Skip unparseable events
        }
      }
    }

    // If stop reason is end_turn, we're done
    if (stopReason === "end_turn" || stopReason === "stop") {
      break;
    }

    // If there were tool uses, add assistant message and tool results, then loop
    if (contentBlocks.some((b) => b.type === "tool_use")) {
      // Reconstruct assistant message with both text and tool_use blocks
      const assistantContent: ContentBlock[] = [];
      if (fullResult) {
        assistantContent.push({ type: "text", text: fullResult });
      }
      assistantContent.push(
        ...contentBlocks.filter((b) => b.type === "tool_use")
      );

      messages.push({ role: "assistant", content: assistantContent });

      // Add tool results
      const toolResults = [];
      for (const block of contentBlocks.filter((b) => b.type === "tool_use")) {
        const result = await executeTool(
          block.name!,
          block.input as Record<string, unknown>
        );
        toolResults.push({
          type: "tool_result" as const,
          tool_use_id: block.id,
          content: result,
        });
      }

      messages.push({ role: "user", content: toolResults as any });
      fullResult = ""; // Reset for next iteration's text
    } else {
      break;
    }
  }

  onEvent({ type: "complete", timestamp: Date.now() });
  return fullResult;
}

/**
 * System prompt for the Launchpad agent.
 */
export const SYSTEM_PROMPT = `You are Launchpad, a fast personal AI assistant running as a desktop command palette.
You help the user by executing tasks quickly and concisely.

Guidelines:
- Be extremely concise. This is a command palette, not a chat.
- When using tools, do so without asking for confirmation unless destructive.
- Format output as clean markdown.
- If creating a skill definition, output valid YAML.
- Prioritize speed and directness over politeness.
- When the user asks to "create a skill", generate a YAML skill definition with: name, trigger, description, prompt, and tools fields.`;

/**
 * System prompt addition when creating skills.
 */
export const SKILL_CREATION_PROMPT = `The user wants to create a new Launchpad skill.
Generate a complete skill definition in YAML format with these fields:
- name: Human-readable name
- trigger: Short /command trigger (lowercase, hyphens)
- description: One-line description
- prompt: The full prompt that will be sent to Claude when this skill runs
- tools: Array of tool identifiers needed (e.g., composio:gmail, composio:github)

Output ONLY the YAML, no explanation.`;
