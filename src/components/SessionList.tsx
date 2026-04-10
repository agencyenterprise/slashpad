import React from "react";
import type { SessionInfo } from "../lib/types";

interface Props {
  sessions: SessionInfo[];
  selectedIndex: number;
  onSelect: (session: SessionInfo) => void;
}

function timeAgo(timestamp: number): string {
  const diff = Date.now() - timestamp;
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

export function SessionList({ sessions, selectedIndex, onSelect }: Props) {
  if (sessions.length === 0) return null;

  return (
    <div className="mt-1 bg-surface-1 border border-surface-3 rounded-xl overflow-hidden animate-fade-in">
      {sessions.map((session, i) => (
        <button
          key={session.sessionId}
          onClick={() => onSelect(session)}
          className={`w-full flex items-center gap-3 px-4 h-[52px] transition-colors duration-75 text-left
            ${i === selectedIndex ? "bg-surface-2" : "hover:bg-surface-2/50"}`}
        >
          <span className="w-4 h-4 flex items-center justify-center flex-shrink-0">
            <span className={`w-1.5 h-1.5 rounded-full ${
              session.isRunning ? "bg-accent animate-pulse-subtle"
              : i === selectedIndex ? "bg-accent" : "bg-muted/40"
            }`} />
          </span>
          <span className="flex-1 min-w-0">
            <span className="text-[13px] font-mono text-white/80 truncate block">
              {session.summary}
            </span>
          </span>
          <span className="text-[11px] font-mono text-muted flex-shrink-0">
            {session.isRunning
              ? <span className="text-accent">running</span>
              : timeAgo(session.lastModified)}
          </span>
        </button>
      ))}
    </div>
  );
}
