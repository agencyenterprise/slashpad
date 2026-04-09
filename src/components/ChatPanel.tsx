import React, { useEffect, useRef } from "react";
import type { ChatMessage } from "../lib/types";
import { ToolLine } from "./ToolLine";

interface Props {
  messages: ChatMessage[];
  isAgentReady: boolean;
}

function UserMessage({ message }: { message: ChatMessage }) {
  return (
    <div className="flex justify-end animate-fade-in">
      <div className="max-w-[85%] bg-surface-3 rounded-xl px-3.5 py-2.5">
        <div className="text-[13px] text-white/90 font-mono leading-relaxed whitespace-pre-wrap break-words">
          {message.content}
        </div>
      </div>
    </div>
  );
}

function AssistantMessage({ message }: { message: ChatMessage }) {
  const blocks = message.blocks;

  return (
    <div className="animate-fade-in space-y-2">
      {blocks.map((block, i) =>
        block.type === "text" ? (
          <div key={i} className="text-[13px] text-white/90 font-mono leading-relaxed whitespace-pre-wrap break-words">
            {block.content}
          </div>
        ) : (
          <ToolLine key={i} event={block} />
        )
      )}

      {/* Streaming indicator */}
      {message.status === "streaming" && blocks.length === 0 && (
        <div className="flex items-center gap-2 text-[13px] text-muted font-mono">
          <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse-subtle" />
          Thinking...
        </div>
      )}

      {message.status === "streaming" && blocks.some((b) => b.type === "text") && (
        <span className="inline-block w-1.5 h-3 bg-accent/60 animate-pulse-subtle ml-0.5 -mb-0.5" />
      )}
    </div>
  );
}

export function ChatPanel({ messages, isAgentReady }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll as content streams in
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, messages[messages.length - 1]?.content]);

  return (
    <div
      className="mt-1 bg-surface-1 border border-surface-3 rounded-xl overflow-hidden animate-fade-in flex flex-col"
      style={{ maxHeight: "480px" }}
    >
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto px-4 py-3 space-y-4 min-h-[60px]"
      >
        {messages.map((msg) =>
          msg.role === "user" ? (
            <UserMessage key={msg.id} message={msg} />
          ) : (
            <AssistantMessage key={msg.id} message={msg} />
          )
        )}
      </div>
    </div>
  );
}
