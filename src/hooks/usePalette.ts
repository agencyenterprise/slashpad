import { useState, useCallback, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import Fuse from "fuse.js";
import type { Skill, ToolEvent, ChatMessage, SessionInfo, PaletteMode } from "../lib/types";
import { loadSkills } from "../lib/skills";
import {
  startChatSession,
  listRecentSessions,
  loadSessionMessages,
  ChatSession,
  SYSTEM_PROMPT,
} from "../lib/agent";
import { sessionManager } from "../lib/sessionManager";

const INPUT_HEIGHT = 90;
const MAX_RESULTS_HEIGHT = 480;

let msgIdCounter = 0;
function nextMsgId() {
  return `msg-${Date.now()}-${msgIdCounter++}`;
}

export function usePalette() {
  const [input, setInput] = useState("");
  const [skills, setSkills] = useState<Skill[]>([]);
  const [filteredSkills, setFilteredSkills] = useState<Skill[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [mode, setMode] = useState<PaletteMode>("idle");

  // Chat state
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isAgentReady, setIsAgentReady] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const chatSessionRef = useRef<ChatSession | null>(null);
  const currentAssistantIdRef = useRef<string | null>(null);

  // Ref for stale-closure-safe message access
  const messagesRef = useRef<ChatMessage[]>([]);
  messagesRef.current = messages;

  // Session list
  const [recentSessions, setRecentSessions] = useState<SessionInfo[]>([]);
  const [selectedSessionIndex, setSelectedSessionIndex] = useState(0);

  const inputRef = useRef<HTMLInputElement>(null);

  // Fuse.js for fuzzy matching skills
  const fuse = useRef<Fuse<Skill>>(
    new Fuse([], {
      keys: ["name", "description"],
      threshold: 0.4,
    })
  );

  // Load skills on mount
  useEffect(() => {
    loadSkills().then((loaded) => {
      setSkills(loaded);
      fuse.current = new Fuse(loaded, {
        keys: ["name", "description"],
        threshold: 0.4,
      });
    });
  }, []);

  /**
   * Process an event into React state. Called by self-routing handlers
   * when the session is active (displayed in the UI).
   */
  const processReactEvent = useCallback((event: ToolEvent) => {
    if (event.type === "session_id" && event.sessionId) {
      setSessionId(event.sessionId);
      // Also update the session manager's record
      if (chatSessionRef.current) {
        const state = sessionManager.getState(chatSessionRef.current);
        if (state) state.sessionId = event.sessionId;
      }
      return;
    }

    if (event.type === "ready") {
      setIsAgentReady(true);
      return;
    }

    if (event.type === "text_delta" || event.type === "tool_start" || event.type === "tool_end") {
      setMessages((prev) => {
        const lastMsg = prev[prev.length - 1];
        if (!lastMsg || lastMsg.role !== "assistant" || lastMsg.status !== "streaming") {
          const newId = nextMsgId();
          currentAssistantIdRef.current = newId;
          return [
            ...prev,
            {
              id: newId,
              role: "assistant" as const,
              content: "",
              toolEvents: [],
              blocks: [],
              timestamp: Date.now(),
              status: "streaming" as const,
            },
          ];
        }
        return prev;
      });
    }

    if (event.type === "text_delta" && event.delta) {
      setMessages((prev) =>
        prev.map((m) => {
          if (m.id !== currentAssistantIdRef.current) return m;
          const lastBlock = m.blocks[m.blocks.length - 1];
          const updatedBlocks =
            lastBlock && lastBlock.type === "text"
              ? [...m.blocks.slice(0, -1), { type: "text" as const, content: lastBlock.content + event.delta }]
              : [...m.blocks, { type: "text" as const, content: event.delta! }];
          return { ...m, content: m.content + event.delta, blocks: updatedBlocks };
        })
      );
    }

    if ((event.type === "tool_start" || event.type === "tool_end") && event.tool) {
      setMessages((prev) =>
        prev.map((m) =>
          m.id === currentAssistantIdRef.current
            ? { ...m, toolEvents: [...m.toolEvents, event], blocks: [...m.blocks, event] }
            : m
        )
      );
    }

    if (event.type === "error") {
      const errorContent = event.error || "An error occurred";
      setMessages((prev) => {
        const hasAssistant =
          currentAssistantIdRef.current != null &&
          prev.some((m) => m.id === currentAssistantIdRef.current);
        if (hasAssistant) {
          return prev.map((m) =>
            m.id === currentAssistantIdRef.current
              ? { ...m, status: "error" as const, content: m.content || errorContent }
              : m
          );
        }
        const newId = nextMsgId();
        currentAssistantIdRef.current = newId;
        return [
          ...prev,
          {
            id: newId,
            role: "assistant" as const,
            content: errorContent,
            toolEvents: [],
            blocks: [{ type: "text" as const, content: errorContent }],
            timestamp: Date.now(),
            status: "error" as const,
          },
        ];
      });
    }

    if (event.type === "complete") {
      setMessages((prev) =>
        prev.map((m) =>
          m.id === currentAssistantIdRef.current ? { ...m, status: "complete" as const } : m
        )
      );
      currentAssistantIdRef.current = null;
    }
  }, []);

  /**
   * Create a self-routing event handler for a chat session.
   * The handler checks sessionManager.activeSession at dispatch time:
   *   - If active → processReactEvent (updates UI)
   *   - If not active → sessionManager.bufferEvent (stores for later)
   *
   * The `sessSlot` pattern handles the async gap: the handler is passed
   * to the ChatSession constructor before `await` resolves, so we use a
   * mutable local that's set immediately after. JS is single-threaded,
   * so no events can arrive in that gap.
   */
  const createSessionHandler = useCallback(
    (sessSlot: { current: ChatSession | null }) => {
      return (event: ToolEvent) => {
        const sess = sessSlot.current;
        if (!sess) return;
        if (sessionManager.activeSession === sess) {
          processReactEvent(event);
        } else {
          sessionManager.bufferEvent(sess, event);
        }
      };
    },
    [processReactEvent]
  );

  // Load recent sessions, merging in any managed sessions
  const refreshSessions = useCallback(async () => {
    try {
      const diskSessions = await listRecentSessions();
      const managed = sessionManager.getAll();
      const runningIds = sessionManager.getRunningIds();
      const seen = new Set<string>();
      const merged: SessionInfo[] = [];

      // Managed sessions with known sessionId go first
      for (const { session, state } of managed) {
        if (state.sessionId) {
          seen.add(state.sessionId);
          merged.push({
            sessionId: state.sessionId,
            summary: state.summary,
            lastModified: Date.now(),
            firstPrompt: state.summary,
            isRunning: session.isAlive,
            _managed: session,
          });
        }
      }
      // Disk sessions (the alive-fallback in resumeSession handles matching
      // for managed sessions whose sessionId hasn't arrived yet)
      for (const ds of diskSessions) {
        if (!seen.has(ds.sessionId)) {
          merged.push({ ...ds, isRunning: runningIds.has(ds.sessionId) });
        }
      }
      setRecentSessions(merged);
    } catch {
      setRecentSessions([]);
    }
  }, []);

  useEffect(() => {
    refreshSessions();
  }, [refreshSessions]);

  // Kill active session
  const killSession = useCallback(async () => {
    const session = chatSessionRef.current;
    if (session) {
      sessionManager.remove(session);
      await session.kill();
      chatSessionRef.current = null;
    }
  }, []);

  // Deactivate current session — snapshot messages, flip activeSession to null.
  // Safe to call any number of times, at any point.
  const detachSession = useCallback(() => {
    sessionManager.deactivate([...messagesRef.current]);
    chatSessionRef.current = null;
  }, []);

  // Listen for palette show/hide events from Rust
  useEffect(() => {
    const unlisten = listen("palette-shown", () => {
      detachSession();
      setInput("/");
      setMode("skills");
      setMessages([]);
      setIsAgentReady(false);
      setSessionId(null);
      setFilteredSkills(skills);
      setSelectedIndex(0);
      setSelectedSessionIndex(0);
      currentAssistantIdRef.current = null;
      refreshSessions();
      setTimeout(() => inputRef.current?.focus(), 50);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [detachSession, refreshSessions, skills]);

  // Kill all sessions on app quit
  useEffect(() => {
    const cleanup = () => sessionManager.killAll();
    window.addEventListener("beforeunload", cleanup);
    return () => window.removeEventListener("beforeunload", cleanup);
  }, []);

  // Refocus input when agent becomes ready
  useEffect(() => {
    if (isAgentReady && mode === "chatting") {
      inputRef.current?.focus();
    }
  }, [isAgentReady, mode]);

  // Resize window based on content
  useEffect(() => {
    let height = INPUT_HEIGHT;
    if (mode === "skills") {
      const count = filteredSkills.length || skills.length;
      height += Math.min(Math.max(count, 1) * 52, 260);
    } else if (mode === "chatting") {
      height += MAX_RESULTS_HEIGHT;
    } else if (mode === "idle" && recentSessions.length > 0 && !input) {
      height += Math.min(Math.max(recentSessions.length, 1) * 52, 260);
    }
    invoke("resize_palette", { height });
  }, [mode, filteredSkills.length, skills.length, recentSessions.length, input, messages.length]);

  // Filter skills as user types
  useEffect(() => {
    if (mode === "chatting") return;
    if (input.startsWith("/")) {
      const query = input.slice(1);
      if (query.length === 0) {
        setFilteredSkills(skills);
      } else {
        const results = fuse.current.search(query);
        setFilteredSkills(results.map((r) => r.item));
      }
      setMode("skills");
      setSelectedIndex(0);
    } else if (mode === "skills") {
      setMode("idle");
      setFilteredSkills([]);
    }
  }, [input, skills, mode]);

  const dismiss = useCallback(() => {
    invoke("hide_palette");
  }, []);

  // Start a new chat session
  const startChat = useCallback(
    async (prompt: string, skill?: Skill) => {
      detachSession();

      setMode("chatting");
      setIsAgentReady(false);
      setSessionId(null);
      currentAssistantIdRef.current = null;

      const userMsg: ChatMessage = {
        id: nextMsgId(),
        role: "user",
        content: prompt,
        toolEvents: [],
        blocks: [{ type: "text" as const, content: prompt }],
        timestamp: Date.now(),
        status: "complete",
      };
      setMessages([userMsg]);
      setInput("");

      // Mutable slot — set after await so the handler can reference the session
      const sessSlot = { current: null as ChatSession | null };
      const handler = createSessionHandler(sessSlot);

      try {
        const session = await startChatSession(prompt, SYSTEM_PROMPT, handler);
        sessSlot.current = session;
        sessionManager.register(session, [userMsg], prompt);
        sessionManager.activate(session);
        chatSessionRef.current = session;
      } catch (e: any) {
        processReactEvent({ type: "error", error: e?.message || String(e), timestamp: Date.now() });
        processReactEvent({ type: "ready", timestamp: Date.now() });
      }
    },
    [detachSession, createSessionHandler, processReactEvent]
  );

  // Resume a past session
  const resumeSession = useCallback(
    async (info: SessionInfo) => {
      // Find the managed session. Three strategies (in priority order):
      // 1. _managed ref attached during refreshSessions
      // 2. sessionId lookup in the manager
      // 3. Any alive managed session (handles the case where session_id
      //    event hasn't arrived yet — it's emitted AFTER tool events)
      let session: ChatSession | undefined;
      let state = undefined as ReturnType<typeof sessionManager.getState>;

      const managed = info._managed as ChatSession | undefined;
      if (managed) {
        state = sessionManager.getState(managed);
        if (state) session = managed;
      }
      if (!state) {
        const bg = sessionManager.getBySessionId(info.sessionId);
        if (bg) { session = bg.session; state = bg.state; }
      }
      if (!state) {
        const all = sessionManager.getAll();
        const alive = all.find(({ session: s }) => s.isAlive);
        if (alive) { session = alive.session; state = alive.state; }
      }

      if (session && state) {
        detachSession();
        chatSessionRef.current = session;
        sessionManager.activate(session);

        setMode("chatting");
        setSessionId(info.sessionId);
        setMessages(
          state.messages.map((m) => ({
            ...m,
            toolEvents: [...m.toolEvents],
            blocks: m.blocks.map((b) => (b.type === "text" ? { ...b } : { ...b })),
          }))
        );
        currentAssistantIdRef.current = state.currentAssistantId;
        setIsAgentReady(state.status !== "running");
        setInput("");
      } else {
        // No managed session — load from disk
        detachSession();
        setMode("chatting");
        setIsAgentReady(true);
        setSessionId(info.sessionId);
        setInput("");
        currentAssistantIdRef.current = null;
        try {
          const loaded = await loadSessionMessages(info.sessionId);
          setMessages(loaded);
        } catch {
          setMessages([]);
        }
      }
    },
    [detachSession]
  );

  // Send a follow-up in active session
  const sendFollowUp = useCallback(
    async (content: string) => {
      if (!content.trim()) return;

      const userMsg: ChatMessage = {
        id: nextMsgId(),
        role: "user",
        content: content.trim(),
        toolEvents: [],
        blocks: [{ type: "text" as const, content: content.trim() }],
        timestamp: Date.now(),
        status: "complete",
      };
      setMessages((prev) => [...prev, userMsg]);
      setInput("");
      setIsAgentReady(false);
      currentAssistantIdRef.current = null;

      if (chatSessionRef.current) {
        try {
          await chatSessionRef.current.sendMessage(content.trim());
        } catch (e: any) {
          processReactEvent({ type: "error", error: e?.message || String(e), timestamp: Date.now() });
          processReactEvent({ type: "ready", timestamp: Date.now() });
        }
      } else if (sessionId) {
        // Resumed session with dead sidecar — spawn new one
        const sessSlot = { current: null as ChatSession | null };
        const handler = createSessionHandler(sessSlot);
        try {
          const session = await startChatSession(content.trim(), SYSTEM_PROMPT, handler, sessionId);
          sessSlot.current = session;
          sessionManager.register(session, [...messagesRef.current], content.trim());
          sessionManager.activate(session);
          chatSessionRef.current = session;
        } catch (e: any) {
          processReactEvent({ type: "error", error: e?.message || String(e), timestamp: Date.now() });
          processReactEvent({ type: "ready", timestamp: Date.now() });
        }
      }
    },
    [sessionId, createSessionHandler, processReactEvent]
  );

  const handleSubmit = useCallback(() => {
    if (mode === "chatting" && isAgentReady && input.trim()) {
      sendFollowUp(input);
    } else if (mode === "skills" && filteredSkills.length > 0) {
      const skill = filteredSkills[selectedIndex];
      if (skill) {
        startChat(`/${skill.name}`, skill);
      }
    } else if (mode === "idle" && !input && recentSessions.length > 0) {
      const session = recentSessions[selectedSessionIndex];
      if (session) {
        resumeSession(session);
      }
    } else if (input.trim() && !input.startsWith("/")) {
      startChat(input.trim());
    }
  }, [input, mode, isAgentReady, filteredSkills, selectedIndex, recentSessions, selectedSessionIndex, startChat, sendFollowUp, resumeSession]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "Escape":
          if (mode === "chatting") {
            killSession();
            setMode("idle");
            setMessages([]);
            setInput("");
            setIsAgentReady(false);
            setSessionId(null);
            currentAssistantIdRef.current = null;
            refreshSessions();
          } else {
            dismiss();
          }
          break;
        case "ArrowDown":
          e.preventDefault();
          if (mode === "skills") {
            setSelectedIndex((i) => Math.min(i + 1, filteredSkills.length - 1));
          } else if (mode === "idle" && !input && recentSessions.length > 0) {
            setSelectedSessionIndex((i) => Math.min(i + 1, recentSessions.length - 1));
          }
          break;
        case "ArrowUp":
          e.preventDefault();
          if (mode === "skills") {
            setSelectedIndex((i) => Math.max(i - 1, 0));
          } else if (mode === "idle" && !input && recentSessions.length > 0) {
            setSelectedSessionIndex((i) => Math.max(i - 1, 0));
          }
          break;
        case "Enter":
          e.preventDefault();
          handleSubmit();
          break;
      }
    },
    [mode, input, filteredSkills.length, recentSessions.length, handleSubmit, dismiss, killSession, refreshSessions]
  );

  const copyResult = useCallback(() => {
    const lastAssistant = [...messages].reverse().find((m) => m.role === "assistant");
    if (lastAssistant) {
      navigator.clipboard.writeText(lastAssistant.content);
    }
  }, [messages]);

  return {
    input,
    setInput,
    inputRef,
    skills,
    filteredSkills,
    selectedIndex,
    mode,
    messages,
    isAgentReady,
    recentSessions,
    selectedSessionIndex,
    handleKeyDown,
    handleSubmit,
    copyResult,
    dismiss,
    resumeSession,
  };
}
