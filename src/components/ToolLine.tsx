import React from "react";
import type { ToolEvent } from "../lib/types";

export function ToolLine({ event }: { event: ToolEvent }) {
  if (event.type === "tool_start") {
    return (
      <div className="flex items-center gap-2 text-[12px] font-mono animate-fade-in">
        <span className="w-4 h-4 flex items-center justify-center">
          <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse-subtle" />
        </span>
        <span className="text-muted">
          Running <span className="text-accent-dim">{event.tool}</span>...
        </span>
      </div>
    );
  }

  if (event.type === "tool_end") {
    return (
      <div className="flex items-center gap-2 text-[12px] font-mono animate-fade-in">
        <span className="w-4 h-4 flex items-center justify-center text-success">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            <path d="M1 5.5L3.5 8L9 2" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </span>
        <span className="text-muted">
          <span className="text-accent-dim">{event.tool}</span>
          {event.result && (
            <span className="text-muted/60 ml-2 truncate max-w-[400px] inline-block align-bottom">
              {event.result}
            </span>
          )}
        </span>
      </div>
    );
  }

  if (event.type === "error") {
    return (
      <div className="flex items-center gap-2 text-[12px] font-mono animate-fade-in">
        <span className="w-4 h-4 flex items-center justify-center text-danger">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            <path d="M2 2L8 8M8 2L2 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </span>
        <span className="text-danger">{event.error}</span>
      </div>
    );
  }

  return null;
}
