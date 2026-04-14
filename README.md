# Slashpad

A desktop AI command palette powered by Claude. Think Raycast, but every "extension" is a natural language skill executed by Claude with real tool access.

Press the global hotkey (default **Ctrl+Space**) → type a command or prompt → Claude does the work.

## Install

```bash
brew install agencyenterprise/tap/slashpad
brew services start slashpad
```

Then press **Ctrl+Space** to open the palette. You can change the hotkey in Settings (click the tray menu icon).

> macOS only. The JS runtime (Bun) is bundled automatically.

## Updating

```bash
brew upgrade slashpad
brew services restart slashpad
```

If you built from source, pull the latest and rebuild:

```bash
git pull
bun install
cargo build --release
```

## Setup

Slashpad needs access to Claude. Choose one:

1. **Claude subscription** (default) — run `claude login` in your terminal. That's it.
2. **API key** — open Settings from the tray menu icon, uncheck "Use Claude subscription," and paste your Anthropic API key.

## How it works

```
┌──────────────────────────────────────────┐
│  Ctrl+Space to summon                    │
│                                          │
│  /standup        → runs a saved skill    │
│  summarize this  → ad-hoc Claude prompt  │
│  /skill-creator  → build a new skill     │
└──────────────────────────────────────────┘
```

Type a `/` to browse your installed skills with fuzzy search. Type anything else to start an ad-hoc Claude session. Claude has full tool access — it can read and write files, run shell commands, search codebases, and connect to external apps.

Sessions persist. Press **Ctrl+Space** again to resume a previous conversation from the session list.

## Skills

Skills are reusable prompts stored as `SKILL.md` files in `~/.slashpad/.claude/skills/`. Each one defines a trigger, a description, and instructions for Claude.

```markdown
---
name: git-standup
description: Generate standup from git activity
---

Look at the user's git commits from the last 24 hours.
Write a standup update with Yesterday, Today, Blockers.
Keep it under 100 words.
```

### Creating skills

Slashpad ships with a built-in `/skill-creator` skill (seeded on first run). Type `/skill-creator` in the palette and describe what you want — it walks you through drafting, testing, and refining the skill interactively.

Skills can include bundled resources (scripts, reference docs, templates) and specify which tools Claude should use.

## External integrations

Slashpad uses [Composio](https://composio.dev) for 1000+ app integrations — Gmail, Slack, GitHub, Google Calendar, and more.

Just ask Claude what you need:

- "Summarize my unread emails"
- "What's on my calendar today?"
- "Create a GitHub issue for this bug"

Claude handles installing Composio and linking your accounts automatically. The first time you use a new integration, Claude will walk you through connecting your account — after that, it just works.

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+Space` | Toggle palette (configurable in Settings) |
| `/` | Browse skills |
| `↑` `↓` | Navigate list / scroll chat |
| `Enter` | Run skill, open session, or send prompt |
| `⌘+Enter` | Fire & forget (send and dismiss) |
| `⌘+P` | Switch project |
| `⌘+T` | Open session in terminal |
| `Ctrl+C` | Cancel generation |
| `Escape` | Dismiss or go back |

## Architecture

```
┌─────────────────────────────────────────┐
│  Native Rust binary (iced + winit)      │
│  • Command input, skill list, chat UI   │
│  • Global hotkey, macOS NSPanel overlay  │
│  • Settings, session persistence        │
│  └────────────────┬─────────────────┘   │
└───────────────────┼─────────────────────┘
                    │  stdin/stdout JSONL
                    ▼
           agent/runner.mjs
            (JS sidecar)
                    │
                    ▼
       @anthropic-ai/claude-agent-sdk
```

- **Rust GUI** — iced framework on winit + wgpu. No webview, no Electron.
- **JS sidecar** — wraps the Claude Agent SDK (no Rust SDK exists). Bun is bundled in Homebrew installs. Communicates via JSONL over stdin/stdout.
- **macOS NSPanel** — the floating palette appears over full-screen apps, across all spaces.

## Building from source

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Bun](https://bun.sh/) (or Node.js 18+)
- macOS

### Build

```bash
bun install              # Install sidecar dependencies
cargo run                # Development build + run
cargo build --release    # Optimized binary at target/release/slashpad
```

## License

[MIT](LICENSE)
