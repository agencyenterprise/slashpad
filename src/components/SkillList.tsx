import React from "react";
import type { Skill } from "../lib/types";

interface Props {
  skills: Skill[];
  selectedIndex: number;
}

export function SkillList({ skills, selectedIndex }: Props) {
  if (skills.length === 0) {
    return (
      <div className="mt-1 px-5 py-4 bg-surface-1 border border-surface-3 rounded-xl animate-fade-in"
>
        <p className="text-muted text-[13px] font-mono">No matching skills</p>
      </div>
    );
  }

  return (
    <div
      className="mt-1 bg-surface-1 border border-surface-3 rounded-xl overflow-hidden animate-fade-in"
      role="listbox"
    >
      {skills.map((skill, i) => (
        <div
          key={skill.name}
          role="option"
          aria-selected={i === selectedIndex}
          className={`flex items-center gap-3 px-5 py-3 cursor-default transition-colors duration-75
            ${i === selectedIndex
              ? "bg-surface-3"
              : "bg-transparent hover:bg-surface-2"
            }`}
        >
          {/* Trigger badge */}
          <span className="flex-shrink-0 font-mono text-[13px] text-accent font-medium min-w-[80px]">
            /{skill.name}
          </span>

          {/* Description */}
          <div className="flex-1 min-w-0">
            <span className="text-muted text-[12px] truncate">
              {skill.description}
            </span>
          </div>

          {/* Run hint on selected */}
          {i === selectedIndex && (
            <kbd className="text-[10px] text-muted font-mono bg-surface-0 px-1.5 py-0.5 rounded ml-1">
              ↵
            </kbd>
          )}
        </div>
      ))}
    </div>
  );
}
