export interface Skill {
  name: string;
  description: string;
  path: string;
  args?: { name: string; description: string; required: boolean }[];
}

export interface ToolEvent {
  type: "tool_start" | "tool_end" | "text_delta" | "error" | "complete" | "ready" | "session_id";
  tool?: string;
  args?: Record<string, unknown>;
  result?: string;
  delta?: string;
  error?: string;
  sessionId?: string;
  timestamp: number;
}

export type ContentBlock =
  | { type: "text"; content: string }
  | ToolEvent;

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  toolEvents: ToolEvent[];
  blocks: ContentBlock[];
  timestamp: number;
  status: "streaming" | "complete" | "error";
}

export interface SessionInfo {
  sessionId: string;
  summary: string;
  lastModified: number;
  firstPrompt?: string;
}

export type PaletteMode = "idle" | "skills" | "chatting";
