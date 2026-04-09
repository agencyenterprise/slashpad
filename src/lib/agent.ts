/**
 * Agent runner that manages Claude chat sessions via the Agent SDK sidecar.
 *
 * Architecture:
 * - Spawns a Node.js sidecar process running the Claude Agent SDK
 * - Chat sessions are long-lived: the sidecar stays alive for follow-up messages via stdin
 * - Session persistence is handled by the SDK (stored in ~/.claude/projects/<cwd>/)
 * - List and messages modes spawn one-shot processes for data retrieval
 */

import { Command, Child } from "@tauri-apps/plugin-shell";
import { invoke } from "@tauri-apps/api/core";
import type { ToolEvent, ChatMessage, SessionInfo } from "./types";

type EventCallback = (event: ToolEvent) => void;

/**
 * Get an optional API key override from localStorage.
 */
export function getApiKey(): string | null {
  const key = localStorage.getItem("launchpad_api_key");
  if (key && key.startsWith("sk-ant-")) return key;
  if (key) localStorage.removeItem("launchpad_api_key");
  return null;
}

export function setApiKey(key: string) {
  localStorage.setItem("launchpad_api_key", key);
}

async function getRunnerPath(): Promise<string> {
  let projectDir: string;
  try {
    projectDir = await invoke<string>("get_project_dir");
    if (projectDir.endsWith("/src-tauri") || projectDir.endsWith("\\src-tauri")) {
      projectDir = projectDir.replace(/[/\\]src-tauri$/, "");
    }
  } catch {
    projectDir = ".";
  }
  return `${projectDir}/agent/runner.mjs`;
}

async function getLaunchpadDir(): Promise<string> {
  return await invoke<string>("get_launchpad_dir");
}

/**
 * A live chat session backed by a long-lived sidecar process.
 * Send follow-up messages via sendMessage(), kill when done.
 */
export class ChatSession {
  private child: Child | null = null;
  private cmd: Command<string> | null = null;
  private _onEvent: EventCallback;
  private _killed = false;

  constructor(onEvent: EventCallback) {
    this._onEvent = onEvent;
  }

  /** @internal — called by startChatSession */
  async _start(payload: Record<string, unknown>, runnerPath: string): Promise<void> {
    const base64Payload = btoa(Array.from(new TextEncoder().encode(JSON.stringify(payload)), (b) => String.fromCharCode(b)).join(""));
    this.cmd = Command.create("node-agent", [runnerPath, base64Payload]);

    let buffer = "";
    let stderrOutput = "";

    this.cmd.stdout.on("data", (chunk: string) => {
      buffer += chunk;
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const event = JSON.parse(line) as ToolEvent;
          this._onEvent(event);
        } catch {
          // Skip unparseable lines
        }
      }
    });

    this.cmd.stderr.on("data", (chunk: string) => {
      stderrOutput += chunk;
    });

    this.cmd.on("close", (data: { code: number }) => {
      if (!this._killed && data.code !== 0 && data.code !== null) {
        const errMsg = stderrOutput.trim() || `Agent process exited with code ${data.code}`;
        this._onEvent({ type: "error", error: errMsg, timestamp: Date.now() });
      }
      this.child = null;
    });

    this.cmd.on("error", (err: string) => {
      if (!this._killed) {
        this._onEvent({ type: "error", error: err, timestamp: Date.now() });
      }
    });

    this.child = await this.cmd.spawn();
  }

  /**
   * Send a follow-up message to the running session.
   */
  async sendMessage(content: string): Promise<void> {
    if (!this.child) throw new Error("Session not running");
    const msg = JSON.stringify({ type: "message", content }) + "\n";
    await this.child.write(msg);
  }

  /**
   * Kill the session process.
   */
  async kill(): Promise<void> {
    this._killed = true;
    if (this.child) {
      try {
        await this.child.write(JSON.stringify({ type: "close" }) + "\n");
      } catch {
        // Process may already be dead
      }
      try {
        await this.child.kill();
      } catch {
        // Already exited
      }
      this.child = null;
    }
  }
}

/**
 * Start a new chat session or resume an existing one.
 */
export async function startChatSession(
  prompt: string,
  systemPrompt: string,
  onEvent: EventCallback,
  resumeId?: string
): Promise<ChatSession> {
  const [runnerPath, launchpadDir] = await Promise.all([
    getRunnerPath(),
    getLaunchpadDir(),
  ]);

  const payload: Record<string, unknown> = {
    mode: "chat",
    prompt,
    systemPrompt,
    apiKey: getApiKey() || undefined,
    cwd: launchpadDir,
  };
  if (resumeId) {
    payload.resume = resumeId;
  }

  const session = new ChatSession(onEvent);
  await session._start(payload, runnerPath);
  return session;
}

/**
 * List recent sessions from the SDK's session store.
 */
export async function listRecentSessions(): Promise<SessionInfo[]> {
  const [runnerPath, launchpadDir] = await Promise.all([
    getRunnerPath(),
    getLaunchpadDir(),
  ]);

  const payload = {
    mode: "list",
    cwd: launchpadDir,
  };

  const base64Payload = btoa(Array.from(new TextEncoder().encode(JSON.stringify(payload)), (b) => String.fromCharCode(b)).join(""));
  const cmd = Command.create("node-agent", [runnerPath, base64Payload]);

  return new Promise((resolve, reject) => {
    const sessions: SessionInfo[] = [];
    let buffer = "";

    cmd.stdout.on("data", (chunk: string) => {
      buffer += chunk;
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const event = JSON.parse(line);
          if (event.type === "session") {
            sessions.push({
              sessionId: event.sessionId,
              summary: event.summary,
              lastModified: event.lastModified,
              firstPrompt: event.firstPrompt,
            });
          }
        } catch {
          // Skip unparseable
        }
      }
    });

    cmd.on("close", () => {
      resolve(sessions);
    });

    cmd.on("error", (err: string) => {
      reject(new Error(err));
    });

    cmd.spawn();
  });
}

/**
 * Load messages from a past session for display.
 */
export async function loadSessionMessages(sessionId: string): Promise<ChatMessage[]> {
  const [runnerPath, launchpadDir] = await Promise.all([
    getRunnerPath(),
    getLaunchpadDir(),
  ]);

  const payload = {
    mode: "messages",
    sessionId,
    cwd: launchpadDir,
  };

  const base64Payload = btoa(Array.from(new TextEncoder().encode(JSON.stringify(payload)), (b) => String.fromCharCode(b)).join(""));
  const cmd = Command.create("node-agent", [runnerPath, base64Payload]);

  return new Promise((resolve, reject) => {
    const messages: ChatMessage[] = [];
    let buffer = "";
    let idCounter = 0;

    cmd.stdout.on("data", (chunk: string) => {
      buffer += chunk;
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const event = JSON.parse(line);
          if (event.type === "chat_message") {
            messages.push({
              id: `loaded-${idCounter++}`,
              role: event.role,
              content: event.content || "",
              toolEvents: event.toolEvents || [],
              timestamp: event.timestamp,
              status: "complete",
            });
          }
        } catch {
          // Skip unparseable
        }
      }
    });

    cmd.on("close", () => {
      resolve(messages);
    });

    cmd.on("error", (err: string) => {
      reject(new Error(err));
    });

    cmd.spawn();
  });
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
- Prioritize speed and directness over politeness.

## Composio — External App Integrations

You have access to 1000+ app integrations through the Composio CLI.
Bias toward action: run \`composio search <task>\`, then \`composio execute <slug>\`.
Input validation, auth checks, and error messages are built in — just try it.

### Installation
If \`composio\` is not found or errors on startup, install it:
  curl -fsSL https://composio.dev/install | bash
Then authenticate: \`composio login\`

### Core Commands

**search** — Find tools. Use this first — describe what you need in natural language.
  composio search <query> [--toolkits text]

**execute** — Run a tool. Handles input validation and auth checks automatically.
  If auth is missing, the error tells you what to run. Use aggressively.
  composio execute <slug> [-d, --data text] [--dry-run] [--get-schema]

**link** — Connect an account. Only needed when execute tells you to — don't preemptively link.
  composio link <toolkit> [--no-wait]

**run** — Run inline TS/JS code with shimmed CLI commands; injected execute(), search(), proxy(), experimental_subAgent(), and z (zod).
  composio run <code> [-- ...args] | run [-f, --file text] [-- ...args] [--dry-run]

**proxy** — curl-like access to any toolkit API through Composio using the linked account.
  composio proxy <url> --toolkit text [-X method] [-H header]... [-d data]

**tools** — Inspect known tools.
  composio tools info <slug>
  composio tools list <toolkit>

**artifacts** — Inspect the cwd-scoped session artifact directory and history.
  composio artifacts cwd

### Workflow
search → execute. If execute fails with an auth error, run link, then retry.

### Examples
  # Find a tool
  composio search "create github issue"
  # → returns GITHUB_CREATE_ISSUE

  # Execute it (will error if not linked — that's fine)
  composio execute GITHUB_CREATE_ISSUE -d '{ "repo": "owner/repo", "title": "Bug" }'
  # → if auth missing: "Run \`composio link github\` first"

  # Link only when told to
  composio link github

  # Raw API access when no tool exists
  composio proxy https://gmail.googleapis.com/gmail/v1/users/me/profile --toolkit gmail

  # Run a script with injected helpers
  composio run 'const me = await execute("GITHUB_GET_THE_AUTHENTICATED_USER"); console.log(me)'

### Getting Started
When the user asks you to do something with an external app:
1. composio search "<what they want done>"
2. composio execute <slug from search> -d '<params>'
3. If auth error → composio link <toolkit>, then retry step 2.

Do not assume we lack coverage. Search first — we likely support it.
Do not preemptively link accounts or ask the user what to connect.
Just try. Auth and validation errors are self-descriptive.`;
