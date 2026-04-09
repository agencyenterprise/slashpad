import React, { useState, useEffect } from "react";
import { emit } from "@tauri-apps/api/event";
import { usePalette } from "./hooks/usePalette";
import { CommandInput } from "./components/CommandInput";
import { SkillList } from "./components/SkillList";
import { ChatPanel } from "./components/ChatPanel";
import { SessionList } from "./components/SessionList";
import { Settings } from "./components/Settings";

export default function App() {
  const {
    input,
    setInput,
    inputRef,
    filteredSkills,
    selectedIndex,
    mode,
    messages,
    isAgentReady,
    recentSessions,
    selectedSessionIndex,
    handleKeyDown,
    copyResult,
    resumeSession,
  } = usePalette();

  const [showSettings, setShowSettings] = useState(false);

  // Auto-dismiss on blur (clicking outside), but not while chatting
  // (agent tools like composio link can open browser tabs, stealing focus)
  useEffect(() => {
    const handler = () => {
      if (mode !== "chatting") emit("palette-blur");
    };
    window.addEventListener("blur", handler);
    return () => window.removeEventListener("blur", handler);
  }, [mode]);

  // Intercept /settings command
  const handleInputChange = (value: string) => {
    if (value === "/settings") {
      setShowSettings(true);
      setInput("");
      return;
    }
    setShowSettings(false);
    setInput(value);
  };

  return (
    <div className="p-2">
      {/* Command input bar */}
      <CommandInput
        value={input}
        onChange={handleInputChange}
        onKeyDown={(e) => {
          if (e.key === "Escape" && showSettings) {
            setShowSettings(false);
            return;
          }
          handleKeyDown(e);
        }}
        mode={mode}
        isAgentReady={isAgentReady}
        inputRef={inputRef}
      />

      {/* Settings panel */}
      {showSettings && <Settings onClose={() => setShowSettings(false)} />}

      {/* Skill search results */}
      {!showSettings && mode === "skills" && (
        <SkillList skills={filteredSkills} selectedIndex={selectedIndex} />
      )}

      {/* Recent sessions on idle */}
      {!showSettings && mode === "idle" && !input && recentSessions.length > 0 && (
        <SessionList
          sessions={recentSessions}
          selectedIndex={selectedSessionIndex}
          onSelect={resumeSession}
        />
      )}

      {/* Chat panel */}
      {!showSettings && mode === "chatting" && (
        <ChatPanel messages={messages} isAgentReady={isAgentReady} />
      )}
    </div>
  );
}
