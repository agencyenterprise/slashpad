# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> **If this is a fresh session continuing the Rust rewrite, read `HANDOFF.md` at the repo root first.** It lists what's verified, what's untested, the known gaps, and the manual smoke test. This file covers architecture; `HANDOFF.md` covers status.

## What This Is

Launchpad is a desktop AI command palette (think Raycast) built as a **native Rust binary** using **iced** (GUI) + a **Node.js sidecar** that wraps the Claude Agent SDK. Users press **Ctrl+Space** to summon a floating palette, type a command or natural language prompt, and Claude executes it with tool access.

There is **no webview** and **no React/TypeScript**. The previous Tauri + React implementation was rewritten to be pure Rust — everything except the sidecar.

## Commands

```bash
npm install              # Install sidecar's Node dependencies (@anthropic-ai/claude-agent-sdk)
cargo run                # Development build + run
cargo build --release    # Optimized release binary at target/release/launchpad
cargo check              # Fast type-check feedback loop
```

There are no automated tests.

## Architecture

**Two runtimes:**
- **Rust binary** (`src/`): UI (iced + winit + wgpu), state machine, window management, global hotkey, NSPanel wrapping, sidecar process management.
- **Node.js sidecar** (`agent/runner.mjs`): Runs `@anthropic-ai/claude-agent-sdk` in a spawned child process. Communicates with the Rust side via JSONL on stdin/stdout. **Unchanged from the pre-rewrite version.**

**Module layout** (`src/`):
- `main.rs` — entry point. Builds a tokio runtime, enters it, seeds bundled skills, sets macOS activation policy, starts iced.
- `app.rs` — iced `Application` impl. Central state machine ported from the old `usePalette.ts`. Owns the external event bus (hotkey + sidecar events funneled into iced via `Subscription::run`).
- `state.rs` — `Mode` (Idle/Skills/Chatting/Settings), `ChatMessageView`, `ContentBlock`, `Skill`, `SessionInfo`.
- `hotkey.rs` — `global-hotkey` registration. Runs a blocking thread that forwards presses into an `UnboundedSender`. Has a `parse_hotkey` function matching the old `HotkeyRecorder.tsx` string format.
- `settings.rs` — `~/.launchpad/settings.json` (serde_json-backed).
- `skills.rs` — SKILL.md loader (walks `~/.launchpad/.claude/skills`, parses frontmatter with `serde_yaml`). Also seeds `skill-creator` via `include_dir!("bundled-skills/skill-creator")`.
- `sessions.rs` — One-shot sidecar runs in `list`/`messages` mode to fetch recent sessions and past session messages.
- `fuzzy.rs` — `nucleo-matcher` wrapper replacing Fuse.js.
- `tray.rs` — menu-bar tray icon (`tray-icon` → `NSStatusItem`). Created on the main thread in `main()` before iced starts; click/menu events are forwarded into the `External` bus.
- `sidecar/` — `events.rs` (serde types for runner.mjs JSONL events), `payload.rs` (base64 payload for argv[2]), `process.rs` (`tokio::process::Command` + stdin/stdout pumps), `mod.rs` (public `spawn`/`SpawnedSidecar`/`FollowUp`).
- `platform/` — `macos.rs` for NSPanel wrapping via `objc2` + `objc2-app-kit` (activation policy, style mask, window level, collection behavior, `dispatch_async_f` main-thread hop); `stub.rs` for non-macOS builds.
- `ui/` — iced widgets: `theme.rs`, `command_input.rs`, `skill_list.rs`, `idle_list.rs`, `chat_panel.rs`, `tool_line.rs`, `settings.rs`.

## State machine (ported from usePalette.ts)

Modes: `Idle` → `Skills` (when input starts with `/`) → `Chatting` (after submit) → `Settings` (when input is exactly `/settings`). Port of the original React state machine.

**Key message flows:**
- Hotkey thread → `External::HotkeyPressed` → iced subscription → `Message::HotkeyPressed` → `toggle_palette()`.
- User types → `text_input.on_input` → `Message::InputChanged` → filter skills via `fuzzy::filter_skills`.
- User presses Enter in input → `text_input.on_submit` → `Message::Submit` → spawns sidecar via `spawn_sidecar_chat`.
- Sidecar stdout JSONL → tokio reader task → `External::Sidecar(event)` → iced subscription → `Message::SidecarEvent` → `process_sidecar_event()` which mutates `self.messages`.
- Esc key → `iced::keyboard::on_key_press` subscription → `Message::EscapePressed` → mode-dependent dismiss/back.
- Window blur → `iced::event::listen_with` subscription → `Message::PaletteBlurred` → hide unless mid-chat.

## macOS threading rule

**All NSPanel/NSWindow ops MUST run on the main thread.** iced's event loop runs on the main thread, so Message handlers are safe. But background tasks (tokio::spawn, std::thread::spawn) that need to touch windows must hop to the main thread via `platform::macos::dispatch_main_async(|| { ... })`, which wraps `dispatch_async_f(_dispatch_main_q, ...)`.

The 200ms post-launch NSPanel wrapping hook in `Launchpad::new()` uses `std::thread::spawn` + `dispatch_main_async` to safely apply the style mask once winit has finished creating the window.

## Sidecar IPC schema

The sidecar is spawned as `node agent/runner.mjs <base64-payload>`. The payload is a base64-encoded JSON blob with one of three shapes:

- **chat**: `{ mode: "chat", prompt, systemPrompt, apiKey?, cwd, resume? }` — long-lived; reads stdin for follow-up `{"type":"message","content":"..."}` or `{"type":"close"}` lines.
- **list**: `{ mode: "list", cwd }` — one-shot; emits `{ type: "session", sessionId, summary, lastModified, firstPrompt }` per session then `complete`.
- **messages**: `{ mode: "messages", sessionId, cwd }` — one-shot; emits `{ type: "chat_message", role, content, toolEvents }` per message.

Chat mode emits (one JSON per line on stdout):
- `text_delta { delta }` — streaming text chunk
- `tool_start { tool, args }` / `tool_end { tool, args, result? }`
- `session_id { sessionId }` — emitted once after first response
- `error { error }` / `complete` / `ready`

See `src/sidecar/events.rs` for the canonical serde enum.

## Window behavior

Initial iced window: 720x90, frameless, transparent, always-on-top, `visible: false`. On startup the post-launch hook applies:
- `NSWindowStyleMask::NonactivatingPanel` (bit 7)
- `NSModalPanelWindowLevel` (8)
- Collection behavior: `canJoinAllSpaces | ignoresCycle | fullScreenAuxiliary`
- Transparent background, no shadow

`show_palette()` / `hide_palette()` call `orderFrontRegardless` / `orderOut` on the raw NSWindow pointer.

## Theme

Dark theme defined in `src/ui/theme.rs`. Colors ported 1:1 from the old Tailwind config:
- `SURFACE_0 #0b0b0d`, `SURFACE_1 #161618`, `SURFACE_2 #1f1f23`, `SURFACE_3 #2a2a30`
- `ACCENT #c4a1ff`, `TEXT #f0f0f5`, `MUTED`, `DANGER`, `SUCCESS`

## Things to avoid

- Running NSPanel/NSWindow ops from background threads without `dispatch_main_async`.
- Calling `take()` on `EXTERNAL_RX` outside the subscription stream (it's single-use).
- Modifying `agent/runner.mjs` — it's the pre-rewrite sidecar and the only supported bridge to the Claude Agent SDK.
- Deleting `bundled-skills/skill-creator/` — the `include_dir!` macro in `src/skills.rs` requires it at compile time.
