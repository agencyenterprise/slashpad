# Slashpad

A desktop AI command palette powered by Claude. Think Raycast, but every "extension" is a natural language skill executed by Claude with real tool access.

**Ctrl+Space** → type a command or prompt → Claude does the work → copy the result.

## How it works

```
┌──────────────────────────────────────────┐
│  Ctrl+Space to summon                    │
│                                          │
│  /standup        → runs saved skill      │
│  summarize my    → ad-hoc Claude session │
│  create a skill  → generates new skill   │
│  /settings       → configure API key     │
└──────────────────────────────────────────┘
```

**Skills** are SKILL.md files (with YAML frontmatter) stored in `~/.slashpad/.claude/skills/`. Each one defines a trigger (`/emails`), a description, and what it does. You create skills by telling Claude what you want — the bundled `skill-creator` skill walks you through it.

**Tools** are provided by the Claude Agent SDK (Read, Write, Bash, Glob, Grep, Skill) plus the Composio CLI for external app integrations (Gmail, GitHub, Slack, Calendar, etc.). Claude executes them autonomously during a session.

## Architecture

```
┌─────────────────────────────────────────┐
│  Native Rust binary (iced + winit)      │
│  ┌──────────────────────────────────┐   │
│  │  UI                              │   │
│  │  • Command input bar             │   │
│  │  • Skill list (fuzzy search)     │   │
│  │  • Streaming chat panel          │   │
│  │  • Recent sessions (resume)      │   │
│  │  • Settings panel                │   │
│  └──────────────────────────────────┘   │
│  ┌──────────────────────────────────┐   │
│  │  Platform                        │   │
│  │  • Global hotkey                 │   │
│  │  • macOS NSPanel overlay         │   │
│  │  • Settings persistence          │   │
│  └──────────────────────────────────┘   │
│  ┌──────────────────────────────────┐   │
│  │  Sidecar client (tokio::process) │   │
│  └────────────────┬─────────────────┘   │
└───────────────────┼─────────────────────┘
                    │  stdin/stdout JSONL
                    ▼
           agent/runner.mjs
           (Node.js sidecar)
                    │
                    ▼
       @anthropic-ai/claude-agent-sdk
```

**Key design decisions:**
- **iced GUI** running on winit + wgpu — native Rust, no webview
- **Node.js sidecar** wraps the Claude Agent SDK (no Rust SDK exists)
- **macOS NSPanel** for the floating palette window (appears over full-screen apps)
- **Multi-monitor aware** — palette appears centered on the cursor's current screen
- **Session persistence** via the Claude Agent SDK's built-in store
- **SKILL.md skills** stored under `~/.slashpad/.claude/skills/`

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 18+ — required for the sidecar process
- An Anthropic API key, **or** the Claude CLI logged in (`claude login`)

## Setup

```bash
# Install the sidecar's Node dependencies
npm install

# Run in development mode
cargo run

# Build an optimized release binary
cargo build --release
```

The release binary lives at `target/release/slashpad`.

## Configuration

### API Key

Two ways to provide your Anthropic API key:

1. **Settings panel** — type `/settings` in the palette
2. **Claude CLI** — run `claude login` in your terminal (subscription-based auth, no API key needed)

### Skills

Skills live at `~/.slashpad/.claude/skills/<skill-name>/SKILL.md` with YAML frontmatter:

```markdown
---
name: git-standup
description: Generate standup from git activity
---

Look at the user's git commits from the last 24 hours.
Write a standup update with Yesterday, Today, Blockers.
Keep it under 100 words.
```

The bundled `skill-creator` skill (seeded on first run) can generate new skills interactively — run `/skill-creator` in the palette.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+Space` | Toggle palette (customizable in `/settings`) |
| `/` | Browse skills |
| `↑ ↓` | Navigate skill / session list |
| `Enter` | Run skill or send prompt |
| `Escape` | Dismiss or cancel chat |

## Project Structure

```
slashpad/
├── Cargo.toml              # Rust crate manifest
├── src/
│   ├── main.rs             # Entry point
│   ├── app.rs              # iced Application + state machine
│   ├── state.rs            # Message/Skill/Session types
│   ├── hotkey.rs           # global-hotkey registration
│   ├── settings.rs         # ~/.slashpad/settings.json I/O
│   ├── skills.rs           # SKILL.md loader + bundled skill seeding
│   ├── sessions.rs         # list/resume session history
│   ├── fuzzy.rs            # nucleo-matcher skill filter
│   ├── markdown.rs         # pulldown-cmark → plain text
│   ├── tray.rs             # (stub — tray not yet integrated)
│   ├── platform/
│   │   └── macos.rs        # NSPanel wrapping, activation policy
│   ├── sidecar/
│   │   ├── events.rs       # JSONL event serde types
│   │   ├── payload.rs      # base64 payload for runner.mjs
│   │   └── process.rs      # tokio::process + stdin/stdout pumps
│   └── ui/
│       ├── theme.rs        # dark theme + window settings
│       ├── command_input.rs
│       ├── skill_list.rs
│       ├── chat_panel.rs
│       ├── tool_line.rs
│       ├── session_list.rs
│       └── settings.rs
├── agent/
│   └── runner.mjs          # Node.js Claude Agent SDK sidecar
├── bundled-skills/
│   └── skill-creator/      # Seeded on first run
├── icons/                  # App icons (tray, dock fallback)
├── package.json            # Node deps for the sidecar (runner.mjs)
└── CLAUDE.md               # Guidance for Claude Code sessions
```

## Roadmap

- [x] Core palette UI with global hotkey
- [x] Multi-monitor cursor-aware positioning
- [x] Skill search with fuzzy matching
- [x] Streaming agent sessions with tool use
- [x] Session resume from history
- [x] Pure native Rust (iced, no webview)
- [ ] System tray (stubbed — needs iced event loop integration)
- [ ] Dynamic window resize based on content
- [ ] Hotkey rebinding UI (reader works; recorder widget not yet built)
- [ ] Rich markdown rendering in chat panel (currently plain-text flattened)
- [ ] `.app` bundle packaging for macOS distribution
