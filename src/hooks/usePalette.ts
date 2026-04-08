import { useState, useCallback, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import Fuse from "fuse.js";
import type { Skill, ToolEvent, SessionStatus } from "../lib/types";
import { loadSkills, saveSkill, buildSkillFromSession } from "../lib/skills";
import { runSession, SYSTEM_PROMPT, SKILL_CREATION_PROMPT } from "../lib/agent";

const INPUT_HEIGHT = 90;
const MAX_RESULTS_HEIGHT = 480;

export function usePalette() {
  const [input, setInput] = useState("");
  const [skills, setSkills] = useState<Skill[]>([]);
  const [filteredSkills, setFilteredSkills] = useState<Skill[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [mode, setMode] = useState<"idle" | "skills" | "running" | "result">("idle");
  const [events, setEvents] = useState<ToolEvent[]>([]);
  const [result, setResult] = useState("");
  const [status, setStatus] = useState<SessionStatus>("idle");
  const [showSaveDialog, setShowSaveDialog] = useState(false);
  const [toolsUsed, setToolsUsed] = useState<string[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const resultRef = useRef("");

  // Fuse.js for fuzzy matching skills
  const fuse = useRef<Fuse<Skill>>(
    new Fuse([], {
      keys: ["trigger", "name", "description"],
      threshold: 0.4,
    })
  );

  // Load skills on mount
  useEffect(() => {
    loadSkills().then((loaded) => {
      setSkills(loaded);
      fuse.current = new Fuse(loaded, {
        keys: ["trigger", "name", "description"],
        threshold: 0.4,
      });
    });
  }, []);

  // Listen for palette show/hide events from Rust
  useEffect(() => {
    const unlisten = listen("palette-shown", () => {
      // Reset state when palette opens
      setInput("");
      setMode("idle");
      setEvents([]);
      setResult("");
      setStatus("idle");
      setShowSaveDialog(false);
      setSelectedIndex(0);
      resultRef.current = "";
      setTimeout(() => inputRef.current?.focus(), 50);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Resize window based on content
  useEffect(() => {
    let height = INPUT_HEIGHT;
    if (mode === "skills") {
      const count = filteredSkills.length || skills.length;
      height += Math.min(Math.max(count, 1) * 52, 260);
    } else if (mode === "running" || mode === "result") {
      height += MAX_RESULTS_HEIGHT;
    }
    invoke("resize_palette", { height });
  }, [mode, filteredSkills.length, skills.length, events.length]);

  // Filter skills as user types
  useEffect(() => {
    if (input.startsWith("/")) {
      const query = input.slice(1);
      if (query.length === 0) {
        setFilteredSkills(skills);
      } else {
        const results = fuse.current.search(query);
        setFilteredSkills(results.map((r) => r.item));
      }
      setMode("skills");
      setSelectedIndex(0);
    } else if (mode === "skills") {
      setMode("idle");
      setFilteredSkills([]);
    }
  }, [input, skills]);

  const dismiss = useCallback(() => {
    invoke("hide_palette");
  }, []);

  const runSkill = useCallback(
    async (skill: Skill) => {
      setMode("running");
      setStatus("running");
      setEvents([]);
      setResult("");
      resultRef.current = "";
      setToolsUsed([]);

      const usedTools: string[] = [];

      const finalResult = await runSession(
        skill.prompt,
        SYSTEM_PROMPT,
        skill.tools,
        (event) => {
          setEvents((prev) => [...prev, event]);
          if (event.type === "text_delta" && event.delta) {
            resultRef.current += event.delta;
            setResult(resultRef.current);
          }
          if (event.type === "tool_start" && event.tool) {
            usedTools.push(event.tool);
          }
          if (event.type === "complete") {
            setStatus("complete");
            setMode("result");
          }
          if (event.type === "error") {
            setStatus("error");
            setMode("result");
          }
        }
      );

      setToolsUsed(usedTools);
    },
    []
  );

  const runAdHoc = useCallback(
    async (prompt: string) => {
      setMode("running");
      setStatus("running");
      setEvents([]);
      setResult("");
      resultRef.current = "";
      setToolsUsed([]);

      const isSkillCreation =
        prompt.toLowerCase().includes("create a skill") ||
        prompt.toLowerCase().includes("make a skill") ||
        prompt.toLowerCase().includes("new skill");

      const systemPrompt = isSkillCreation
        ? SYSTEM_PROMPT + "\n\n" + SKILL_CREATION_PROMPT
        : SYSTEM_PROMPT;

      const usedTools: string[] = [];

      await runSession(prompt, systemPrompt, [], (event) => {
        setEvents((prev) => [...prev, event]);
        if (event.type === "text_delta" && event.delta) {
          resultRef.current += event.delta;
          setResult(resultRef.current);
        }
        if (event.type === "tool_start" && event.tool) {
          usedTools.push(event.tool);
        }
        if (event.type === "complete") {
          setStatus("complete");
          setMode("result");
        }
        if (event.type === "error") {
          setStatus("error");
          setMode("result");
        }
      });

      setToolsUsed(usedTools);
    },
    []
  );

  const handleSubmit = useCallback(() => {
    if (mode === "skills" && filteredSkills.length > 0) {
      const skill = filteredSkills[selectedIndex];
      if (skill) {
        runSkill(skill);
      }
    } else if (input.trim() && !input.startsWith("/")) {
      runAdHoc(input.trim());
    }
  }, [input, mode, filteredSkills, selectedIndex, runSkill, runAdHoc]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "Escape":
          if (mode === "running" || mode === "result") {
            setMode("idle");
            setInput("");
            setEvents([]);
            setResult("");
          } else {
            dismiss();
          }
          break;
        case "ArrowDown":
          e.preventDefault();
          setSelectedIndex((i) =>
            Math.min(i + 1, filteredSkills.length - 1)
          );
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex((i) => Math.max(i - 1, 0));
          break;
        case "Enter":
          e.preventDefault();
          handleSubmit();
          break;
      }
    },
    [mode, filteredSkills.length, handleSubmit, dismiss]
  );

  const copyResult = useCallback(() => {
    navigator.clipboard.writeText(result);
  }, [result]);

  const handleSaveAsSkill = useCallback(
    async (trigger: string) => {
      const skill = buildSkillFromSession(trigger, input, toolsUsed);
      await saveSkill(skill);
      const updated = await loadSkills();
      setSkills(updated);
      fuse.current = new Fuse(updated, {
        keys: ["trigger", "name", "description"],
        threshold: 0.4,
      });
      setShowSaveDialog(false);
    },
    [input, toolsUsed]
  );

  return {
    input,
    setInput,
    inputRef,
    skills,
    filteredSkills,
    selectedIndex,
    mode,
    events,
    result,
    status,
    showSaveDialog,
    setShowSaveDialog,
    handleKeyDown,
    handleSubmit,
    handleSaveAsSkill,
    copyResult,
    dismiss,
  };
}
