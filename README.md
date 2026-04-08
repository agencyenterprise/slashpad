# Launchpad

A desktop AI command palette powered by Claude. Think Raycast, but every "extension" is a natural language skill executed by Claude with real tool access.

**Alt+Space** → type a command or prompt → Claude does the work → copy the result.

## How it works

```
┌──────────────────────────────────────────┐
│  Alt+Space to summon                     │
│                                          │
│  /standup        → runs saved skill      │
│  summarize my    → ad-hoc Claude session │
│  create a skill  → generates new skill   │
│  /settings       → configure API key     │
└──────────────────────────────────────────┘
```

**Skills** are YAML files stored in `~/.launchpad/skills/`. Each one defines a trigger (`/emails`), a prompt, and the tools it needs. You create skills by telling Claude what you want — or run something ad-hoc and hit "Save as Skill" to keep it.

**Tools** are provided by Composio (Gmail, GitHub, Slack, Calendar, etc.) and local system access (shell, files). Claude executes them autonomously during a session.

## Architecture

```
┌─────────────────────────────────────┐
│            Tauri Shell              │
│  ┌───────────┐  ┌───────────────┐  │
│  │   Rust    │  │  React UI     │  │
│  │  backend  │  │  (WebView)    │  │
│  │           │  │               │  │
│  │ • hotkey  │  │ • input bar   │  │
│  │ • window  │  │ • skill list  │  │
│  │ • monitor │  │ • results     │  │
│  │   detect  │  │ • streaming   │  │
│  └───────────┘  └──────┬────────┘  │
│                        │           │
│              ┌─────────▼─────────┐ │
│              │  Agent Runner     │ │
│              │                   │ │
│              │ Anthropic API     │ │
│              │ (streaming +      │ │
│              │  tool use loop)   │ │
│              │                   │ │
│              │ Composio Tools    │ │
│              │ (Gmail, GitHub,   │ │
│              │  Slack, etc.)     │ │
│              └───────────────────┘ │
└─────────────────────────────────────┘
```

**Key design decisions:**
- **Tauri v2** for the shell — small binary, native performance, Rust handles window management
- **Multi-monitor aware** — palette appears centered on whichever screen the mouse cursor is on
- **Anthropic Messages API** with streaming for the agent loop (not Claude Code CLI)
- **Composio MCP** for third-party integrations (Phase 2)
- **YAML skills** stored locally — portable, editable, version-controllable

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 18+
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)
- An Anthropic API key, **or** the Claude CLI logged in (`claude login`)

## Setup

```bash
# Install dependencies
npm install

# Run in development mode (hot-reload)
npm run tauri dev

# Build for production
npm run tauri build
```

## Configuration

### API Key

Three ways to provide your Anthropic API key (checked in order):

1. **Settings UI** — type `/settings` in the palette
2. **Claude CLI** — run `claude login` in your terminal
3. **Environment variable** — set `ANTHROPIC_API_KEY`

### Skills

Skills live in `~/.launchpad/skills/` as YAML files:

```yaml
name: Git Standup
trigger: /standup
description: Generate standup from git activity
prompt: |
  Look at my git commits from the last 24 hours.
  Write a standup update with Yesterday, Today, Blockers.
  Keep it under 100 words.
tools:
  - composio:github
```

**Creating skills:**
1. **By hand** — create a `.yaml` file in `~/.launchpad/skills/`
2. **By prompt** — type "create a skill that..." in the palette
3. **By saving** — run an ad-hoc command, then click "Save as Skill"

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Alt+Space` | Toggle palette |
| `/` | Browse skills |
| `↑↓` | Navigate skill list |
| `Enter` | Run skill or send prompt |
| `Escape` | Dismiss or go back |

## Roadmap

- [x] Core palette UI with global hotkey
- [x] Multi-monitor cursor-aware positioning
- [x] Skill search with fuzzy matching
- [x] Streaming agent sessions with tool use
- [x] Save ad-hoc sessions as skills
- [ ] Composio tool integration (Phase 2)
- [ ] Background/scheduled skills
- [ ] Conversation continuations
- [ ] Skill sharing / import from URL
- [ ] Custom themes

## Project Structure

```
launchpad/
├── src/                    # React frontend
│   ├── components/
│   │   ├── CommandInput    # Main text input bar
│   │   ├── SkillList       # Fuzzy search skill dropdown
│   │   ├── ResultsPanel    # Streaming output + tool activity
│   │   ├── SaveSkillDialog # Name a new skill
│   │   └── Settings        # API key + preferences
│   ├── hooks/
│   │   └── usePalette      # Core state management
│   ├── lib/
│   │   ├── agent.ts        # Anthropic API + agent loop
│   │   ├── skills.ts       # YAML skill loading/saving
│   │   └── types.ts        # TypeScript types
│   └── App.tsx
├── src-tauri/              # Rust backend
│   └── src/
│       └── lib.rs          # Window mgmt, hotkey, multi-monitor
├── skills/                 # Example skill definitions
└── package.json
```
