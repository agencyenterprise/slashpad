import React, { useState, useEffect } from "react";

interface Props {
  value: string;
  onChange: (shortcut: string) => Promise<void>;
}

function codeToKeyName(code: string): string | null {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (code.startsWith("F") && !isNaN(Number(code.slice(1)))) return code;
  const map: Record<string, string> = {
    Space: "Space", Enter: "Enter", Tab: "Tab",
    Backspace: "Backspace", Delete: "Delete",
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]",
    Backslash: "\\", Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/",
    Backquote: "`",
  };
  return map[code] || null;
}

function buildShortcutString(e: KeyboardEvent): string | null {
  const key = codeToKeyName(e.code);
  if (!key) return null;

  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");

  if (mods.length === 0) return null;

  return [...mods, key].join("+");
}

function formatShortcut(shortcut: string): { symbol: string }[] {
  const isMac = navigator.platform.includes("Mac");
  return shortcut.split("+").map((part) => {
    switch (part) {
      case "Ctrl": return { symbol: isMac ? "⌃" : "Ctrl" };
      case "Alt": return { symbol: isMac ? "⌥" : "Alt" };
      case "Shift": return { symbol: isMac ? "⇧" : "Shift" };
      case "Super": return { symbol: isMac ? "⌘" : "Win" };
      default: return { symbol: part };
    }
  });
}

export function HotkeyRecorder({ value, onChange }: Props) {
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!recording) return;

    const handler = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return;

      if (e.key === "Escape") {
        setRecording(false);
        return;
      }

      const shortcut = buildShortcutString(e);
      if (!shortcut) return;

      setRecording(false);
      setError(null);
      onChange(shortcut).catch((err) => setError(String(err)));
    };

    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [recording, onChange]);

  const parts = formatShortcut(value);

  return (
    <div>
      <button
        onClick={() => { setRecording(true); setError(null); }}
        className={`flex items-center gap-1.5 px-3 py-2 rounded-lg border transition-colors ${
          recording
            ? "border-accent bg-accent/10"
            : "border-surface-3 bg-surface-0 hover:border-accent/30"
        }`}
      >
        {recording ? (
          <span className="text-[12px] text-accent font-mono animate-pulse-subtle">
            Press a key combination...
          </span>
        ) : (
          parts.map((part, i) => (
            <React.Fragment key={i}>
              {i > 0 && <span className="text-muted text-[11px]">+</span>}
              <kbd className="text-[12px] text-white font-mono bg-surface-3 px-2 py-0.5 rounded">
                {part.symbol}
              </kbd>
            </React.Fragment>
          ))
        )}
      </button>
      {error && (
        <p className="text-[11px] text-danger font-mono mt-1">{error}</p>
      )}
    </div>
  );
}
