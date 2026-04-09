# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Launchpad is a desktop AI command palette (think Raycast) built with **Tauri v2** (Rust backend + React/TypeScript frontend). Users press **Ctrl+Space** to summon a floating palette, type a command or natural language prompt, and Claude executes it with tool access.

## Commands

```bash
npm install              # Install frontend dependencies
npm run tauri dev        # Development mode (hot-reload, Vite on :1420)
npm run tauri build      # Production build (creates native app bundle)
```

There are no tests or linting configured.

## Architecture

**Three runtimes:**
- **Rust** (`src-tauri/src/lib.rs`): Window management, global hotkey (`Ctrl+Space`), multi-monitor cursor detection, palette toggle/resize, launchpad directory creation, project dir resolution. Tauri commands: `hide_palette`, `resize_palette`, `get_launchpad_dir`, `get_project_dir`.
- **TypeScript/React** (`src/`): UI and state management run in the Tauri webview.
- **Node.js sidecar** (`agent/runner.mjs`): Runs the Claude Agent SDK in a spawned process. Communicates with the webview via JSONL on stdout.

**Agent architecture** (`src/lib/agent.ts` + `agent/runner.mjs`):
- `agent.ts` exposes a `ChatSession` class that manages a long-lived Node.js sidecar process for multi-turn conversations
- The sidecar runs `@anthropic-ai/claude-agent-sdk` with built-in tools: Read, Write, Bash, Glob, Grep, Skill
- `runner.mjs` supports three modes: `"chat"` (new/resumed session), `"list"` (list recent sessions), `"messages"` (load past session messages)
- Session persistence & resumption via Claude Agent SDK's built-in store; sessions tagged with "launchpad"
- Auth uses existing Claude CLI login (no API key needed). Optional API key override via localStorage.
- Events stream as JSONL on stdout, mapped to event types for the UI
- `get_project_dir` Rust command resolves CWD; `agent.ts` strips `src-tauri/` suffix to find project root

**State management** (`src/hooks/usePalette.ts`):
- Single custom hook manages all palette state (input, mode, skills, events, results)
- Fuse.js for fuzzy skill search (threshold 0.4)
- Modes: `idle` (shows recent sessions) -> `skills` (when input starts with `/`) -> `chatting` (active conversation)
- Session resumption: idle mode loads recent sessions for quick resume
- Window dynamically resizes via Rust `resize_palette` command based on mode

**Skills system** (`src/lib/skills.ts`):
- SKILL.md files with YAML frontmatter stored in `~/.launchpad/.claude/skills/`
- Loaded via Tauri FS plugin, parsed with js-yaml
- Only `skill-creator` is bundled and seeded on first run (from `src-tauri/bundled-skills/`)
- Skills can be created via prompt ("create a skill...") using the bundled skill-creator

**Window behavior** (configured in `src-tauri/tauri.conf.json`):
- 720x90 frameless, transparent, always-on-top, hidden by default, skip taskbar
- macOS private API enabled for advanced window management

## macOS Threading Rule

**All NSPanel/window UI operations (show, hide, setLevel, etc.) MUST run on the main thread.** Tauri `async fn` commands run on a thread pool and will crash. Use `fn` (not `async fn`) for Tauri commands that touch windows, and wrap `app.listen` callbacks in `run_on_main_thread` if they call panel/window methods.

## Key Patterns

- Path alias: `@/*` maps to `src/*`
- Styling: Tailwind CSS with custom dark theme (surface-0/1/2/3 colors, accent `#c4a1ff`)
- Animations: Framer Motion (fadeIn, slideDown, pulseSubtle keyframes)
- Fonts: Berkeley Mono, JetBrains Mono, Inter
- Components are flat (App.tsx -> CommandInput, ChatPanel, ToolLine, SkillList, SessionList, Settings), props-only, no context providers
- Agent sidecar is spawned via `Command.create("node-agent", [runnerPath, base64Payload])` — the `node-agent` scope maps to `node` in Tauri capabilities
