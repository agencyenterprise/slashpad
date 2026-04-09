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

  // Load recent sessions
  const refreshSessions = useCallback(async () => {
    try {
      const sessions = await listRecentSessions();
      setRecentSessions(sessions);
    } catch {
      setRecentSessions([]);
    }
  }, []);

  useEffect(() => {
    refreshSessions();
  }, [refreshSessions]);

  // Kill active session helper
  const killSession = useCallback(async () => {
    if (chatSessionRef.current) {
      await chatSessionRef.current.kill();
      chatSessionRef.current = null;
    }
  }, []);

  // Listen for palette show/hide events from Rust
  useEffect(() => {
    const unlisten = listen("palette-shown", () => {
      // Kill any active session and reset — prefill "/" to show skills
      killSession();
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
  }, [killSession, refreshSessions, skills]);

  // Refocus input when agent becomes ready (disabled input drops focus)
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

  // Filter skills as user types (only when not in chatting mode)
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

  // Shared event handler for chat sessions
  const makeOnEvent = useCallback(() => {
    return (event: ToolEvent) => {
      if (event.type === "session_id" && event.sessionId) {
        setSessionId(event.sessionId);
        return;
      }

      if (event.type === "ready") {
        setIsAgentReady(true);
        return;
      }

      if (event.type === "text_delta" || event.type === "tool_start" || event.type === "tool_end") {
        // Ensure an assistant message exists for this turn
        setMessages((prev) => {
          const lastMsg = prev[prev.length - 1];
          if (!lastMsg || lastMsg.role !== "assistant" || lastMsg.status !== "streaming") {
            const newId = nextMsgId();
            currentAssistantIdRef.current = newId;
            const newMsg: ChatMessage = {
              id: newId,
              role: "assistant",
              content: "",
              toolEvents: [],
              blocks: [],
              timestamp: Date.now(),
              status: "streaming",
            };
            return [...prev, newMsg];
          }
          return prev;
        });
      }

      if (event.type === "text_delta" && event.delta) {
        setMessages((prev) =>
          prev.map((m) => {
            if (m.id !== currentAssistantIdRef.current) return m;
            const lastBlock = m.blocks[m.blocks.length - 1];
            const updatedBlocks = lastBlock && lastBlock.type === "text"
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
          const hasAssistant = currentAssistantIdRef.current != null &&
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
            m.id === currentAssistantIdRef.current
              ? { ...m, status: "complete" as const }
              : m
          )
        );
        // Reset for next turn
        currentAssistantIdRef.current = null;
      }
    };
  }, []);

  // Start a new chat session
  const startChat = useCallback(
    async (prompt: string, skill?: Skill) => {
      await killSession();

      setMode("chatting");
      setIsAgentReady(false);
      setSessionId(null);
      currentAssistantIdRef.current = null;

      // Add user message
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

      const onEvent = makeOnEvent();

      try {
        const session = await startChatSession(
          prompt,
          SYSTEM_PROMPT,
          onEvent
        );
        chatSessionRef.current = session;
      } catch (e: any) {
        onEvent({ type: "error", error: e?.message || String(e), timestamp: Date.now() });
        onEvent({ type: "ready", timestamp: Date.now() });
      }
    },
    [killSession, makeOnEvent]
  );

  // Resume a past session
  const resumeSession = useCallback(
    async (info: SessionInfo) => {
      await killSession();

      setMode("chatting");
      setIsAgentReady(true); // Ready for input — sidecar not spawned yet
      setSessionId(info.sessionId);
      setInput("");
      currentAssistantIdRef.current = null;

      try {
        const loaded = await loadSessionMessages(info.sessionId);
        setMessages(loaded);
      } catch {
        setMessages([]);
      }
    },
    [killSession]
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

      // If we have a live session, send via stdin
      if (chatSessionRef.current) {
        try {
          await chatSessionRef.current.sendMessage(content.trim());
        } catch (e: any) {
          const onEvent = makeOnEvent();
          onEvent({ type: "error", error: e?.message || String(e), timestamp: Date.now() });
          onEvent({ type: "ready", timestamp: Date.now() });
        }
      } else if (sessionId) {
        // Resumed session — need to spawn sidecar with resume
        const onEvent = makeOnEvent();
        try {
          const session = await startChatSession(
            content.trim(),
            SYSTEM_PROMPT,
            onEvent,
            sessionId
          );
          chatSessionRef.current = session;
        } catch (e: any) {
          onEvent({ type: "error", error: e?.message || String(e), timestamp: Date.now() });
          onEvent({ type: "ready", timestamp: Date.now() });
        }
      }
    },
    [sessionId, makeOnEvent]
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
      // Select a session from the list
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
    // Copy the last assistant message
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
