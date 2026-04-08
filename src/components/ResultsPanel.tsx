import React, { useEffect, useRef } from "react";
import type { ToolEvent, SessionStatus } from "../lib/types";

interface Props {
  events: ToolEvent[];
  result: string;
  status: SessionStatus;
  onCopy: () => void;
  onSaveAsSkill: () => void;
}

function ToolLine({ event }: { event: ToolEvent }) {
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
              → {event.result}
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

export function ResultsPanel({ events, result, status, onCopy, onSaveAsSkill }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [copied, setCopied] = React.useState(false);

  // Auto-scroll as content streams in
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [events, result]);

  const handleCopy = () => {
    onCopy();
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const toolEvents = events.filter(
    (e) => e.type === "tool_start" || e.type === "tool_end" || e.type === "error"
  );

  return (
    <div
      className="mt-1 bg-surface-1 border border-surface-3 rounded-xl overflow-hidden animate-fade-in flex flex-col"
      style={{
        maxHeight: "480px",
      }}
    >
      {/* Tool activity feed */}
      {toolEvents.length > 0 && (
        <div className="px-4 pt-3 pb-2 border-b border-surface-3 space-y-1.5">
          {toolEvents.map((event, i) => (
            <ToolLine key={i} event={event} />
          ))}
        </div>
      )}

      {/* Streaming / final result */}
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto px-4 py-3 min-h-[60px]"
      >
        {status === "running" && !result && toolEvents.length === 0 && (
          <div className="flex items-center gap-2 text-[13px] text-muted font-mono">
            <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse-subtle" />
            Thinking...
          </div>
        )}

        {result && (
          <div className="text-[13px] text-white/90 font-mono leading-relaxed whitespace-pre-wrap break-words">
            {result}
          </div>
        )}
      </div>

      {/* Action bar */}
      {status === "complete" && (
        <div className="flex items-center justify-end gap-2 px-4 py-2.5 border-t border-surface-3">
          <button
            onClick={handleCopy}
            className="flex items-center gap-1.5 text-[11px] font-mono text-muted
              hover:text-white px-2.5 py-1.5 rounded-md hover:bg-surface-3
              transition-colors duration-100"
          >
            {copied ? (
              <>
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                  <path d="M2 6.5L4.5 9L10 3" stroke="#4ade80" strokeWidth="1.5" strokeLinecap="round" />
                </svg>
                Copied
              </>
            ) : (
              <>
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                  <rect x="4" y="4" width="6" height="6" rx="1" stroke="currentColor" strokeWidth="1.2" />
                  <path d="M2 8V2.5C2 2.22 2.22 2 2.5 2H8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
                </svg>
                Copy
              </>
            )}
          </button>

          <button
            onClick={onSaveAsSkill}
            className="flex items-center gap-1.5 text-[11px] font-mono text-muted
              hover:text-accent px-2.5 py-1.5 rounded-md hover:bg-surface-3
              transition-colors duration-100"
          >
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M6 2V10M2 6H10" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
            Save as Skill
          </button>
        </div>
      )}
    </div>
  );
}
