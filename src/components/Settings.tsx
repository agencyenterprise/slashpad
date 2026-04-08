import React, { useState, useEffect } from "react";
import { setApiKey, getApiKey } from "../lib/agent";

interface Props {
  onClose: () => void;
}

export function Settings({ onClose }: Props) {
  const [key, setKey] = useState("");
  const [saved, setSaved] = useState(false);
  const [hasExisting, setHasExisting] = useState(false);

  useEffect(() => {
    const k = getApiKey();
    if (k) {
      setHasExisting(true);
      setKey("sk-ant-••••••••••••" + k.slice(-4));
    }
  }, []);

  const handleSave = () => {
    if (key && !key.startsWith("sk-ant-••")) {
      setApiKey(key);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    }
  };

  return (
    <div
      className="mt-1 bg-surface-1 border border-surface-3 rounded-xl overflow-hidden animate-fade-in"
      style={{ boxShadow: "0 8px 40px rgba(0,0,0,0.5)" }}
    >
      <div className="px-4 py-3 border-b border-surface-3 flex items-center justify-between">
        <span className="text-[13px] text-white font-mono font-medium">Settings</span>
        <button
          onClick={onClose}
          className="text-muted hover:text-white transition-colors text-[12px] font-mono"
        >
          esc
        </button>
      </div>

      <div className="px-4 py-3 space-y-3">
        {/* API Key */}
        <div>
          <label className="text-[11px] text-muted font-mono uppercase tracking-wider block mb-1.5">
            Anthropic API Key
          </label>
          <div className="flex gap-2">
            <input
              type="password"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              onFocus={() => {
                if (key.startsWith("sk-ant-••")) setKey("");
              }}
              placeholder="sk-ant-..."
              className="flex-1 bg-surface-0 text-[13px] text-white font-mono
                px-3 py-2 rounded-lg border border-surface-3 outline-none
                focus:border-accent/40 transition-colors"
            />
            <button
              onClick={handleSave}
              className="text-[12px] font-mono px-3 py-2 rounded-lg transition-colors duration-100
                bg-accent text-surface-1 hover:bg-accent-bright"
            >
              {saved ? "✓" : "Save"}
            </button>
          </div>
          <p className="text-[11px] text-muted/60 font-mono mt-1">
            Or run <code className="text-accent-dim">claude login</code> in your terminal
          </p>
        </div>

        {/* Hotkey info */}
        <div>
          <label className="text-[11px] text-muted font-mono uppercase tracking-wider block mb-1.5">
            Global Hotkey
          </label>
          <div className="flex items-center gap-1.5">
            <kbd className="text-[12px] text-white font-mono bg-surface-3 px-2 py-1 rounded">
              ⌥
            </kbd>
            <span className="text-muted text-[11px]">+</span>
            <kbd className="text-[12px] text-white font-mono bg-surface-3 px-2 py-1 rounded">
              Space
            </kbd>
            <span className="text-[11px] text-muted/60 font-mono ml-2">
              (configurable in future)
            </span>
          </div>
        </div>

        {/* Skills dir */}
        <div>
          <label className="text-[11px] text-muted font-mono uppercase tracking-wider block mb-1.5">
            Skills Directory
          </label>
          <code className="text-[12px] text-accent-dim font-mono">
            ~/.launchpad/skills/
          </code>
        </div>
      </div>
    </div>
  );
}
