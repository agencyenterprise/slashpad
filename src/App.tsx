import React, { useState } from "react";
import { usePalette } from "./hooks/usePalette";
import { CommandInput } from "./components/CommandInput";
import { SkillList } from "./components/SkillList";
import { ChatPanel } from "./components/ChatPanel";
import { SessionList } from "./components/SessionList";
import { SaveSkillDialog } from "./components/SaveSkillDialog";
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
    showSaveDialog,
    setShowSaveDialog,
    handleKeyDown,
    handleSaveAsSkill,
    copyResult,
    resumeSession,
  } = usePalette();

  const [showSettings, setShowSettings] = useState(false);

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
      {!showSettings && mode === "chatting" && !showSaveDialog && (
        <ChatPanel messages={messages} isAgentReady={isAgentReady} />
      )}

      {/* Save as skill dialog */}
      {showSaveDialog && (
        <SaveSkillDialog
          onSave={handleSaveAsSkill}
          onCancel={() => setShowSaveDialog(false)}
        />
      )}
    </div>
  );
}
