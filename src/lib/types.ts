export interface Skill {
  name: string;
  trigger: string;
  description: string;
  prompt: string;
  tools: string[];
  createdAt?: string;
  lastUsed?: string;
}

export type SessionStatus = "idle" | "running" | "complete" | "error";

export interface ToolEvent {
  type: "tool_start" | "tool_end" | "text_delta" | "error" | "complete";
  tool?: string;
  args?: Record<string, unknown>;
  result?: string;
  delta?: string;
  error?: string;
  timestamp: number;
}

export interface Session {
  id: string;
  prompt: string;
  skill?: Skill;
  status: SessionStatus;
  events: ToolEvent[];
  result: string;
  startedAt: number;
}
