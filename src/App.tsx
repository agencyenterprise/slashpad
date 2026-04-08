import React, { useState } from "react";
import { usePalette } from "./hooks/usePalette";
import { CommandInput } from "./components/CommandInput";
import { SkillList } from "./components/SkillList";
import { ResultsPanel } from "./components/ResultsPanel";
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
    events,
    result,
    status,
    showSaveDialog,
    setShowSaveDialog,
    handleKeyDown,
    handleSaveAsSkill,
    copyResult,
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
        status={status}
        inputRef={inputRef}
      />

      {/* Settings panel */}
      {showSettings && <Settings onClose={() => setShowSettings(false)} />}

      {/* Skill search results */}
      {!showSettings && mode === "skills" && (
        <SkillList skills={filteredSkills} selectedIndex={selectedIndex} />
      )}

      {/* Running / Result panel */}
      {!showSettings && (mode === "running" || mode === "result") && !showSaveDialog && (
        <ResultsPanel
          events={events}
          result={result}
          status={status}
          onCopy={copyResult}
          onSaveAsSkill={() => setShowSaveDialog(true)}
        />
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
