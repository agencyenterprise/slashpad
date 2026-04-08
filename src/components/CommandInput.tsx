import React from "react";
import type { SessionStatus } from "../lib/types";

interface Props {
  value: string;
  onChange: (value: string) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  status: SessionStatus;
  inputRef: React.RefObject<HTMLInputElement>;
}

export function CommandInput({ value, onChange, onKeyDown, status, inputRef }: Props) {
  return (
    <div
      data-tauri-drag-region
      className="flex items-center h-[72px] px-5 gap-3 bg-surface-1 border border-surface-3 rounded-2xl"
    >
      {/* Status indicator */}
      <div className="flex-shrink-0 w-5 h-5 flex items-center justify-center">
        {status === "running" ? (
          <div className="w-2.5 h-2.5 rounded-full bg-accent animate-pulse-subtle" />
        ) : status === "complete" ? (
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <path
              d="M2 7.5L5.5 11L12 3"
              stroke="#4ade80"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        ) : status === "error" ? (
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <path
              d="M3 3L11 11M11 3L3 11"
              stroke="#f87171"
              strokeWidth="2"
              strokeLinecap="round"
            />
          </svg>
        ) : (
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" className="opacity-40">
            <path
              d="M2 5L8 11L14 5"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              className="text-white"
            />
          </svg>
        )}
      </div>

      {/* Input */}
      <input
        ref={inputRef as React.RefObject<HTMLInputElement>}
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder="Run a command or ask anything..."
        spellCheck={false}
        autoFocus
        className="flex-1 bg-transparent text-[15px] text-white font-mono
          placeholder:text-muted outline-none caret-accent"
      />

      {/* Hint */}
      <div className="flex-shrink-0 flex items-center gap-1.5">
        {value.startsWith("/") ? (
          <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
            ↵ run
          </kbd>
        ) : value.length > 0 ? (
          <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
            ↵ send
          </kbd>
        ) : (
          <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
            / skills
          </kbd>
        )}
        <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
          esc
        </kbd>
      </div>
    </div>
  );
}
