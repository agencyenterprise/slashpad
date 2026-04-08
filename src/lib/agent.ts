/**
 * Agent runner that executes Claude sessions via the Agent SDK sidecar.
 *
 * Architecture:
 * - Spawns a Node.js sidecar process running the Claude Agent SDK
 * - The sidecar handles the Anthropic API, streaming, and tool execution
 * - Auth uses the existing Claude CLI login (no API key needed)
 * - Events are streamed back as JSONL on stdout
 */

import { Command } from "@tauri-apps/plugin-shell";
import { invoke } from "@tauri-apps/api/core";
import type { ToolEvent } from "./types";

type EventCallback = (event: ToolEvent) => void;

/**
 * Get an optional API key override from localStorage.
 * The Agent SDK uses Claude CLI auth by default — this is only
 * needed if the user explicitly sets a key in Settings.
 * Returns null if no valid key is stored.
 */
export function getApiKey(): string | null {
  const key = localStorage.getItem("launchpad_api_key");
  // Only return keys that look like actual Anthropic API keys
  if (key && key.startsWith("sk-ant-")) return key;
  // Clear garbage values
  if (key) localStorage.removeItem("launchpad_api_key");
  return null;
}

export function setApiKey(key: string) {
  localStorage.setItem("launchpad_api_key", key);
}

/**
 * Run a streaming agent session via the Agent SDK sidecar.
 *
 * Spawns a Node.js process that runs agent/runner.mjs with the Agent SDK.
 * The SDK handles the full agentic loop (API calls, tool execution, streaming).
 * Events are emitted as JSONL on stdout and mapped to ToolEvent callbacks.
 */
export async function runSession(
  prompt: string,
  systemPrompt: string,
  _tools: string[],
  onEvent: EventCallback
): Promise<string> {
  let fullResult = "";

  const payload = {
    prompt,
    systemPrompt,
    apiKey: getApiKey() || undefined,
  };

  const base64Payload = btoa(JSON.stringify(payload));

  // Resolve absolute path to the runner script — the Tauri binary's CWD
  // may not be the project root (e.g. src-tauri/target/debug/ during dev)
  let projectDir: string;
  try {
    projectDir = await invoke<string>("get_project_dir");
    // During tauri dev, CWD is src-tauri/ — go up to project root
    if (projectDir.endsWith("/src-tauri") || projectDir.endsWith("\\src-tauri")) {
      projectDir = projectDir.replace(/[/\\]src-tauri$/, "");
    }
  } catch {
    projectDir = ".";
  }
  const runnerPath = `${projectDir}/agent/runner.mjs`;

  try {
    const cmd = Command.create("node-agent", [runnerPath, base64Payload]);

    // Collect complete lines from stdout (JSONL protocol)
    let buffer = "";
    let stderrOutput = "";

    cmd.stdout.on("data", (chunk: string) => {
      buffer += chunk;
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const event = JSON.parse(line) as ToolEvent;
          if (event.type === "text_delta" && event.delta) {
            fullResult += event.delta;
          }
          onEvent(event);
        } catch {
          // Skip unparseable lines
        }
      }
    });

    cmd.stderr.on("data", (chunk: string) => {
      stderrOutput += chunk;
    });

    const child = await cmd.spawn();

    // Wait for the process to exit
    await new Promise<void>((resolve, reject) => {
      cmd.on("close", (data: { code: number }) => {
        // Process any remaining buffer
        if (buffer.trim()) {
          try {
            const event = JSON.parse(buffer) as ToolEvent;
            if (event.type === "text_delta" && event.delta) {
              fullResult += event.delta;
            }
            onEvent(event);
          } catch {
            // ignore
          }
        }

        if (data.code !== 0 && data.code !== null) {
          // Show full stderr for debugging
          const errMsg = stderrOutput.trim() || `Agent process exited with code ${data.code}`;
          reject(new Error(errMsg));
        } else {
          resolve();
        }
      });

      cmd.on("error", (err: string) => {
        reject(new Error(err));
      });
    });
  } catch (e: any) {
    onEvent({
      type: "error",
      error: e?.message || String(e),
      timestamp: Date.now(),
    });
    onEvent({ type: "complete", timestamp: Date.now() });
  }

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
