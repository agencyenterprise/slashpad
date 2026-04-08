import React from "react";
import type { PaletteMode } from "../lib/types";

interface Props {
  value: string;
  onChange: (value: string) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  mode: PaletteMode;
  isAgentReady: boolean;
  inputRef: React.RefObject<HTMLInputElement>;
}

export function CommandInput({ value, onChange, onKeyDown, mode, isAgentReady, inputRef }: Props) {
  const isChatting = mode === "chatting";
  const isProcessing = isChatting && !isAgentReady;

  const placeholder = isChatting
    ? isAgentReady
      ? "Send a follow-up..."
      : "Waiting for response..."
    : "Run a command or ask anything...";

  return (
    <div
      data-tauri-drag-region
      className="flex items-center h-[72px] px-5 gap-3 bg-surface-1 border border-surface-3 rounded-2xl"
    >
      {/* Status indicator */}
      <div className="flex-shrink-0 w-5 h-5 flex items-center justify-center">
        {isProcessing ? (
          <div className="w-2.5 h-2.5 rounded-full bg-accent animate-pulse-subtle" />
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
        placeholder={placeholder}
        disabled={isProcessing}
        spellCheck={false}
        autoFocus
        className="flex-1 bg-transparent text-[15px] text-white font-mono
          placeholder:text-muted outline-none caret-accent disabled:opacity-50"
      />

      {/* Hint */}
      <div className="flex-shrink-0 flex items-center gap-1.5">
        {isChatting && isAgentReady && value.length > 0 ? (
          <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
            ↵ reply
          </kbd>
        ) : !isChatting && value.startsWith("/") ? (
          <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
            ↵ run
          </kbd>
        ) : !isChatting && value.length > 0 ? (
          <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
            ↵ send
          </kbd>
        ) : !isChatting && !value ? (
          <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
            / skills
          </kbd>
        ) : null}
        <kbd className="text-[11px] text-muted font-mono bg-surface-3 px-1.5 py-0.5 rounded">
          esc
        </kbd>
      </div>
    </div>
  );
}
