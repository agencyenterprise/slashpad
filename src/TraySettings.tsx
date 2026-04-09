import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { HotkeyRecorder } from "./components/HotkeyRecorder";

export default function TraySettings() {
  const [hotkey, setHotkey] = useState("Ctrl+Space");

  useEffect(() => {
    invoke<string>("get_current_hotkey").then(setHotkey).catch(() => {});
  }, []);

  // Auto-dismiss on blur
  useEffect(() => {
    const handler = () => emit("settings-blur");
    window.addEventListener("blur", handler);
    return () => window.removeEventListener("blur", handler);
  }, []);

  const handleHotkeyChange = async (newShortcut: string) => {
    await invoke("update_hotkey", { oldShortcut: hotkey, newShortcut });
    setHotkey(newShortcut);
  };

  return (
    <div className="p-2 h-screen">
      <div
        className="bg-surface-1 rounded-xl border border-surface-3 p-4 flex flex-col h-full"
      >
        {/* Header */}
        <div className="flex items-center justify-between mb-4">
          <span className="text-[13px] text-white font-mono font-medium">Launchpad</span>
          <span className="text-[11px] text-muted/50 font-mono">v0.1.0</span>
        </div>

        {/* Hotkey */}
        <div className="mb-4">
          <label className="text-[11px] text-muted font-mono uppercase tracking-wider block mb-1.5">
            Global Hotkey
          </label>
          <HotkeyRecorder value={hotkey} onChange={handleHotkeyChange} />
        </div>

        {/* Actions */}
        <div className="border-t border-surface-3 pt-3 mt-auto space-y-0.5">
          <button
            onClick={() => { invoke("show_launcher"); emit("settings-blur"); }}
            className="w-full text-left text-[13px] text-muted hover:text-white font-mono
              py-1.5 px-2 rounded-lg hover:bg-surface-2 transition-colors"
          >
            Show Launcher
          </button>
          <button
            onClick={() => invoke("quit_app")}
            className="w-full text-left text-[13px] text-muted hover:text-white font-mono
              py-1.5 px-2 rounded-lg hover:bg-surface-2 transition-colors"
          >
            Quit Launchpad
          </button>
        </div>
      </div>
    </div>
  );
}
