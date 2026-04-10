/**
 * Session manager — centralized registry for all chat sessions.
 *
 * Each ChatSession gets a permanent event handler at construction that
 * self-routes: if the session is "active" (being displayed), events go
 * to React state; otherwise they buffer here. No handler swapping needed.
 *
 * Keyed by ChatSession instance to avoid sessionId timing races.
 */

import type { ChatSession } from "./agent";
import type { ChatMessage, ToolEvent } from "./types";

export interface SessionState {
  messages: ChatMessage[];
  currentAssistantId: string | null;
  status: "running" | "ready" | "error";
  sessionId: string | null;
  summary: string;
  startedAt: number;
}

let bgIdCounter = 0;
function nextBgMsgId() {
  return `bg-${Date.now()}-${bgIdCounter++}`;
}

class SessionManager {
  private sessions = new Map<ChatSession, SessionState>();

  /** The session currently displayed in the UI. null = no active session. */
  activeSession: ChatSession | null = null;

  /**
   * Register a new session in the registry.
   */
  register(
    session: ChatSession,
    initialMessages: ChatMessage[],
    summary: string
  ): void {
    this.sessions.set(session, {
      messages: [...initialMessages],
      currentAssistantId: null,
      status: "running",
      sessionId: null,
      summary,
      startedAt: Date.now(),
    });
  }

  /**
   * Deactivate the current session — snapshot React messages into the
   * buffer so future buffered events append to the right state.
   * Safe to call multiple times or when no session is active.
   */
  deactivate(currentMessages: ChatMessage[]): void {
    if (this.activeSession) {
      const state = this.sessions.get(this.activeSession);
      if (state) {
        // Snapshot: deep-clone React messages into the buffer
        state.messages = currentMessages.map((m) => ({
          ...m,
          toolEvents: [...m.toolEvents],
          blocks: m.blocks.map((b) => (b.type === "text" ? { ...b } : { ...b })),
        }));
        // Detect streaming assistant in progress
        const lastMsg = state.messages[state.messages.length - 1];
        if (lastMsg && lastMsg.role === "assistant" && lastMsg.status === "streaming") {
          state.currentAssistantId = lastMsg.id;
        } else {
          state.currentAssistantId = null;
        }
      }
    }
    this.activeSession = null;
  }

  /**
   * Activate a session — events from it will now route to React state.
   */
  activate(session: ChatSession): void {
    this.activeSession = session;
  }

  /**
   * Buffer an event for a non-active session.
   * Called by the self-routing handler when the session isn't displayed.
   */
  bufferEvent(session: ChatSession, event: ToolEvent): void {
    const state = this.sessions.get(session);
    if (!state) return;

    if (event.type === "session_id" && event.sessionId) {
      state.sessionId = event.sessionId;
      return;
    }

    if (event.type === "ready") {
      state.status = "ready";
      return;
    }

    if (event.type === "text_delta" || event.type === "tool_start" || event.type === "tool_end") {
      const lastMsg = state.messages[state.messages.length - 1];
      if (!lastMsg || lastMsg.role !== "assistant" || lastMsg.status !== "streaming") {
        const newId = nextBgMsgId();
        state.currentAssistantId = newId;
        state.status = "running";
        state.messages.push({
          id: newId,
          role: "assistant",
          content: "",
          toolEvents: [],
          blocks: [],
          timestamp: Date.now(),
          status: "streaming",
        });
      }
    }

    if (event.type === "text_delta" && event.delta) {
      const msg = state.messages.find((m) => m.id === state.currentAssistantId);
      if (msg) {
        msg.content += event.delta;
        const lastBlock = msg.blocks[msg.blocks.length - 1];
        if (lastBlock && lastBlock.type === "text") {
          (lastBlock as { type: "text"; content: string }).content += event.delta;
        } else {
          msg.blocks.push({ type: "text", content: event.delta });
        }
      }
    }

    if ((event.type === "tool_start" || event.type === "tool_end") && event.tool) {
      const msg = state.messages.find((m) => m.id === state.currentAssistantId);
      if (msg) {
        msg.toolEvents.push(event);
        msg.blocks.push(event);
      }
    }

    if (event.type === "error") {
      const errorContent = event.error || "An error occurred";
      const msg = state.messages.find((m) => m.id === state.currentAssistantId);
      if (msg) {
        msg.status = "error";
        msg.content = msg.content || errorContent;
      } else {
        const newId = nextBgMsgId();
        state.currentAssistantId = newId;
        state.messages.push({
          id: newId,
          role: "assistant",
          content: errorContent,
          toolEvents: [],
          blocks: [{ type: "text", content: errorContent }],
          timestamp: Date.now(),
          status: "error",
        });
      }
      state.status = "error";
    }

    if (event.type === "complete") {
      const msg = state.messages.find((m) => m.id === state.currentAssistantId);
      if (msg) {
        msg.status = "complete";
      }
      state.currentAssistantId = null;
    }
  }

  /** Look up a session by its SDK sessionId. */
  getBySessionId(sessionId: string): { session: ChatSession; state: SessionState } | undefined {
    for (const [session, state] of this.sessions) {
      if (state.sessionId === sessionId) return { session, state };
    }
    return undefined;
  }

  /** Get state for a session by instance. */
  getState(session: ChatSession): SessionState | undefined {
    return this.sessions.get(session);
  }

  /** All tracked sessions. */
  getAll(): { session: ChatSession; state: SessionState }[] {
    return Array.from(this.sessions.entries()).map(([session, state]) => ({ session, state }));
  }

  /** Set of sessionIds for sessions with alive sidecars. */
  getRunningIds(): Set<string> {
    const ids = new Set<string>();
    for (const [session, state] of this.sessions) {
      if (state.sessionId && session.isAlive) ids.add(state.sessionId);
    }
    return ids;
  }

  /** Remove a session from the registry. */
  remove(session: ChatSession): void {
    if (this.activeSession === session) this.activeSession = null;
    this.sessions.delete(session);
  }

  /** Kill all tracked sessions (app quit cleanup). */
  killAll(): void {
    for (const session of this.sessions.keys()) {
      session.kill();
    }
    this.sessions.clear();
    this.activeSession = null;
  }
}

export const sessionManager = new SessionManager();
