# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Launchpad is a desktop AI command palette (think Raycast) built with **Tauri v2** (Rust backend + React/TypeScript frontend). Users press **Alt+Space** to summon a floating palette, type a command or natural language prompt, and Claude executes it with tool access.

## Commands

```bash
npm install              # Install frontend dependencies
npm run tauri dev        # Development mode (hot-reload, Vite on :1420)
npm run tauri build      # Production build (creates native app bundle)
```

There are no tests or linting configured.

## Architecture

**Three runtimes:**
- **Rust** (`src-tauri/src/lib.rs`): Window management, global hotkey (`Ctrl+Space`), multi-monitor cursor detection, palette toggle/resize, skills directory creation, project dir resolution. Tauri commands: `hide_palette`, `resize_palette`, `get_skills_dir`, `get_project_dir`.
- **TypeScript/React** (`src/`): UI and state management run in the Tauri webview.
- **Node.js sidecar** (`agent/runner.mjs`): Runs the Claude Agent SDK in a spawned process. Communicates with the webview via JSONL on stdout.

**Agent architecture** (`src/lib/agent.ts` + `agent/runner.mjs`):
- `agent.ts` spawns a Node.js sidecar via Tauri shell plugin (`Command.create("node-agent", [...])`)
- The sidecar runs `@anthropic-ai/claude-agent-sdk` `query()` with built-in tools: Read, Write, Bash, Glob, Grep
- Auth uses existing Claude CLI login (no API key needed). Optional API key override via localStorage.
- Events stream as JSONL on stdout, mapped to `ToolEvent` types for the UI
- `get_project_dir` Rust command resolves CWD; `agent.ts` strips `src-tauri/` suffix to find project root

**State management** (`src/hooks/usePalette.ts`):
- Single custom hook manages all palette state (input, mode, skills, events, results)
- Fuse.js for fuzzy skill search (threshold 0.4)
- Modes: `idle` -> `skills` (when input starts with `/`) -> `running` -> `result`
- Window dynamically resizes via Rust `resize_palette` command based on mode

**Skills system** (`src/lib/skills.ts`):
- YAML files stored in `~/.launchpad/skills/`
- Loaded via Tauri FS plugin, parsed with js-yaml
- Example skills seeded on first run (emails, standup, PRs)
- Skills can be created via prompt ("create a skill...") or saved from ad-hoc sessions

**Window behavior** (configured in `src-tauri/tauri.conf.json`):
- 720x72 frameless, transparent, always-on-top, hidden by default, skip taskbar
- macOS private API enabled for advanced window management

## Key Patterns

- Path alias: `@/*` maps to `src/*`
- Styling: Tailwind CSS with custom dark theme (surface-0/1/2/3 colors, accent `#c4a1ff`)
- Animations: Framer Motion (fadeIn, slideDown, pulseSubtle keyframes)
- Fonts: Berkeley Mono, JetBrains Mono, Inter
- Components are flat (App.tsx -> CommandInput, SkillList, ResultsPanel, SaveSkillDialog, Settings), props-only, no context providers
- Agent sidecar is spawned via `Command.create("node-agent", [runnerPath, base64Payload])` — the `node-agent` scope maps to `node` in Tauri capabilities
