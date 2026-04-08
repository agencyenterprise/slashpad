import React, { useState, useRef, useEffect } from "react";

interface Props {
  onSave: (trigger: string) => void;
  onCancel: () => void;
}

export function SaveSkillDialog({ onSave, onCancel }: Props) {
  const [trigger, setTrigger] = useState("/");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && trigger.length > 1) {
      onSave(trigger);
    } else if (e.key === "Escape") {
      onCancel();
    }
  };

  const handleChange = (value: string) => {
    // Enforce /trigger format: lowercase, hyphens, no spaces
    let cleaned = value;
    if (!cleaned.startsWith("/")) cleaned = "/" + cleaned;
    cleaned = cleaned.replace(/[^a-z0-9-/]/gi, "").toLowerCase();
    // Only one leading slash
    cleaned = "/" + cleaned.replace(/\//g, "");
    setTrigger(cleaned);
  };

  return (
    <div
      className="mt-1 bg-surface-1 border border-surface-3 rounded-xl overflow-hidden animate-fade-in"
      style={{ boxShadow: "0 8px 40px rgba(0,0,0,0.5)" }}
    >
      <div className="px-4 py-3">
        <p className="text-[12px] text-muted font-mono mb-2">
          Choose a trigger command for this skill:
        </p>
        <div className="flex items-center gap-2">
          <input
            ref={inputRef}
            type="text"
            value={trigger}
            onChange={(e) => handleChange(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="/my-skill"
            className="flex-1 bg-surface-0 text-[14px] text-accent font-mono font-medium
              px-3 py-2 rounded-lg border border-surface-3 outline-none
              focus:border-accent/40 transition-colors"
          />
          <button
            onClick={() => trigger.length > 1 && onSave(trigger)}
            disabled={trigger.length <= 1}
            className="text-[12px] font-mono text-surface-1 bg-accent
              px-3 py-2 rounded-lg hover:bg-accent-bright
              disabled:opacity-30 disabled:cursor-not-allowed
              transition-colors duration-100"
          >
            Save
          </button>
          <button
            onClick={onCancel}
            className="text-[12px] font-mono text-muted
              px-3 py-2 rounded-lg hover:bg-surface-3
              transition-colors duration-100"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
