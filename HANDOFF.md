# Handoff — Rust Rewrite Status

This file captures the state of the Rust rewrite of Slashpad so a fresh Claude Code session can pick up without reconstruction. Read this **before** `CLAUDE.md` if you're new to the repo.

## TL;DR

Slashpad was rewritten from **Tauri + React + Node sidecar** to **native Rust (iced + winit + wgpu) + unchanged Node sidecar**. The core interaction — hotkey → palette appears → skills list → keyboard input → float over fullscreen — is now **manually verified working**. The agent/chat side of the app (submit, streaming, tool calls, session resume, settings panel) is still untested and needs a smoke pass.

- **Builds**: `cargo build` — 0 warnings. `cargo clippy -- -D warnings` — passes. Hooks at `.claude/hooks/cargo-{check,clippy}.sh` point at the root `Cargo.toml`.
- **Size**: ~2,400 LOC Rust across 25 files in `src/`.
- **Plan files**: `~/.claude/plans/deep-doodling-shamir.md` (original feasibility) and `~/.claude/plans/floofy-questing-crane.md` (P1 window-behavior pass that landed this session).

## Verified working

Manually smoke-tested end-to-end, works as expected:

- Binary launches, tokio runtime enters, iced event loop starts, no stderr.
- `Ctrl+Space` summons the palette; pressing again hides it.
- Palette appears horizontally centered on the cursor's current monitor, ~20% from the top. Multi-monitor works.
- Palette floats over fullscreen apps (Safari/Xcode in fullscreen Space).
- Custom `SlashpadPanel` subclass of `NSPanel` (registered at runtime via `objc2::declare::ClassBuilder` in `src/platform/macos.rs`) overrides `canBecomeKeyWindow`/`canBecomeMainWindow` → keystrokes reach the iced text input.
- Skill list renders immediately on show (palette opens pre-filled with `/` in `Mode::Skills`).
- Dynamic window resize via `iced::window::resize` in response to mode transitions — base 90px → Skills list height → chat height → back to base on Esc.
- NSPanel class swap + style mask + collection behavior applied both via the 200ms post-launch hook and on every `show_palette()` (self-healing).

## Still untested — smoke test needed

The "front" of the app works; the "back" (anything past Submit) hasn't been driven yet:

1. **Skill filter updates live.** Type `/skil` — does the list narrow? Arrow keys navigate?
2. **Submit spawns sidecar.** Press Enter on a selected skill — does the mode flip to `Chatting`, does a sidecar process actually spawn (`ps aux | grep runner.mjs`), and does text start streaming into the chat panel?
3. **Tool calls render.** When the agent calls a tool (Read/Bash/Grep/Skill), does a tool-line row appear with the tool name and args? See `src/ui/tool_line.rs`.
4. **Session resume.** Close the palette, reopen it with `Ctrl+Space` with empty input — does the recent-sessions list show the session you just created? Does Enter resume it with history? History loads via a one-shot `runner.mjs messages <sessionId>` run; if it fails, the chat appears empty with no banner.
5. **Esc cancels chat.** During a streaming chat, press Esc — does the chat end, sidecar get killed (via `SpawnedSidecar` drop → tokio `kill_on_drop`), and the view return to idle?
6. **Blur auto-hides.** Click outside the palette while not chatting — does it hide? The subscription listens to `iced::window::Event::Unfocused`. During a chat it should NOT hide.
7. **Settings panel.** Type `/settings` — does the panel appear with the current hotkey displayed? Does typing an API key and clicking Save persist to `~/.slashpad/settings.json`?

## TODO — known gaps (ordered by priority)

Each item has enough detail to pick up without asking the user.

### Priority 1 — likely blocking real usability

Items 1–3 **landed this session** (see plan file `~/.claude/plans/floofy-questing-crane.md`). Left: one item.

1. ~~Position palette on the cursor's monitor.~~ ✅ `platform::macos::cursor_palette_position(width)` computes winit-space coords; `show_palette()` batches `iced::window::move_to` on the cached `window_id`.
2. ~~Dynamic window resize based on mode.~~ ✅ `Slashpad::target_height()` ports the `usePalette.ts` heuristic; `resize_task()` is batched into every height-changing update branch.
3. ~~Text input focus after hotkey-show.~~ ✅ `show_palette()` returns a `Task::batch` including `text_input::focus(INPUT_ID.clone())`.
4. **First-show NSPanel race.** Still not fixed. The post-launch hook in `Slashpad::new()` has a hardcoded 200ms sleep, then calls `first_app_window_ptr()` and wraps the window. If iced/winit creates the NSWindow >200ms after `new()` returns, `first_app_window_ptr()` returns null and nothing gets wrapped. The self-healing path is `show_palette()` — which re-applies the style each hotkey press — so in practice the second `Ctrl+Space` always works even if the first missed. If you see a visible flash, either (a) increase the sleep, or (b) use `iced::window::run_with_handle(id, |handle| { ... })` to grab the NSWindow pointer at a guaranteed-safe moment.

### Priority 2 — polish

5. **Tray icon integration.** `src/tray.rs` is a stub. The `tray-icon` crate expects to run on the same main thread as the windowing event loop. With iced owning the winit loop via `iced::application`, you can't just instantiate a tray-icon in `main()` — the menu events won't be pumped. Options:
   - Drive the iced loop manually via `iced_winit` (not `iced::application`), allowing you to poll tray-icon events in the event handler.
   - Use a separate std::thread that runs its own minimal AppKit event loop for the tray, and pipe click events through the `External` bus.
   The old app only used the tray for a settings-open click, so the feature is low-value compared to the work.

6. **Hotkey recorder widget.** `src/ui/settings.rs` shows the hotkey as a `text` widget. To make it editable, add:
   - A button that sets `state.recording_hotkey = true`
   - While `recording_hotkey`, the keyboard subscription should intercept keypresses and build a chord string matching `HotkeyRecorder.tsx`'s format (e.g., `Ctrl+Shift+Space`)
   - On first non-modifier keypress, call `hotkey::update_hotkey(&chord)` and persist via `AppSettings::save`
   - The old JS implementation is in the deleted `src_react_legacy/components/HotkeyRecorder.tsx` — check `git log` for the file if you need it.

7. **Rich markdown rendering in chat panel.** `src/markdown.rs` flattens everything to plain text. Build a proper renderer that walks `pulldown-cmark::Parser` events and emits an `iced::Element` tree: headings → larger bold text, code blocks → monospace container with `SURFACE_0` background, inline code → inline monospace, lists → indented bullets, links → clickable (iced has no native link widget; use a button styled like text).

8. **Fix runner.mjs path for release builds.** `src/sidecar/process.rs::runner_path()` resolves `agent/runner.mjs` relative to `std::env::current_dir()`. This works with `cargo run` but will break when the binary is installed to `/Applications/Slashpad.app/Contents/MacOS/slashpad`. Fix: resolve relative to the executable path via `std::env::current_exe()?.parent().unwrap().join("agent/runner.mjs")`, and bundle `agent/` alongside the binary in the `.app`.

9. **Graceful shutdown.** Old app had `sessionManager.killAll()` on `beforeunload`. Rust app relies on tokio `kill_on_drop` which fires when the `SpawnedSidecar` struct is dropped. This works on SIGINT but not on Cmd+Q from the dock (if there were a dock icon). Consider handling `iced::window::Event::CloseRequested` in the subscription and sending `FollowUp::Close` to the sidecar before exiting.

### Priority 3 — nice-to-have

10. **Packaging as `.app`.** Install `cargo-bundle`, add a `[package.metadata.bundle]` section to `Cargo.toml` with:
    - `name = "Slashpad"`
    - `identifier = "com.slashpad.app"`
    - `icon = ["icons/icon.icns"]`
    - `category = "Productivity"`
    - `osx_info_plist_exts = ["NSApp-LSUIElement: true"]` (to match the `NSApplicationActivationPolicyAccessory` runtime setting)
    Test that `cargo bundle --release` produces a working `.app` and that `agent/` + `node_modules/` are included or resolvable.

11. **Copy last response.** Old app had a `copyResult` action that put the last assistant message on the clipboard. Add a `Ctrl+C` or `Cmd+Shift+C` shortcut via the keyboard subscription that calls `iced::clipboard::write(msg.flat_text())`.

12. **Esc during hotkey recording** should cancel recording. Depends on TODO #6 landing first.

13. **Tune `target_height` for Settings mode.** Currently `Mode::Settings` uses the same `BASE + CHAT = 570` height as Chatting. The Settings form is probably smaller; measure and adjust in `src/app.rs::Slashpad::target_height()`.

## Gotchas — non-obvious things to know

1. **macOS main thread rule.** All NSPanel/NSWindow operations MUST run on the main thread. The iced update loop runs on the main thread, so Message handlers are safe. Background tasks (`tokio::spawn`, `std::thread::spawn`) that touch windows must hop to the main thread via `platform::macos::dispatch_main_async(|| { ... })`. If you forget this, you get a crash inside AppKit with no Rust backtrace.

2. **`EXTERNAL_RX.take()` is single-use.** The subscription stream takes the receiver once from a `Mutex<Option<UnboundedReceiver>>` and drains it forever. If iced ever restarts the subscription (it shouldn't for a fn-pointer subscription, but watch for changes), the second `take()` panics. Symptom would be a panic on the first hotkey press after some state change.

3. **`include_dir!("bundled-skills/skill-creator")` is compile-time.** If you move, rename, or delete `bundled-skills/skill-creator/`, the build breaks. Don't touch it without updating `src/skills.rs`.

4. **Sidecar requires `node` + `node_modules/@anthropic-ai/claude-agent-sdk`.** `npm install` must be run once after checkout. `node` must be on PATH at runtime. The Rust side calls `Command::new("node")`, which searches PATH.

5. **Tokio runtime is entered in `main`.** `main.rs` creates a multi-thread runtime and holds a `_guard` for the entire program lifetime so `tokio::spawn` works everywhere. Don't remove this guard — it'll break the sidecar.

6. **iced 0.13 API specifics:**
   - `iced::stream::channel(100, |output| async {...})` for subscription streams
   - `Subscription::run(fn_pointer)` — note: requires a plain `fn`, not a closure, for stable identity
   - `iced::event::listen_with(|event, _status, _window| ...)` for window events (blur, close, etc.)
   - `text_input::Id` + `text_input::focus(id)` Task for focus control
   - `iced::Padding` uses a builder: `Padding::from(0.0).top(4.0).left(24.0)` — no 4-element array constructor
   - `.style(|theme, status| iced::widget::button::Style { ... })` takes a closure
   Do not blindly upgrade to iced 0.14 — the API is different.

7. **The hotkey parser is incomplete.** `src/hotkey.rs::parse_hotkey` handles letters, digits, F-keys, arrow keys, and common punctuation. It does NOT handle IME/Unicode keys or distinguish left/right modifiers. Sufficient for Ctrl+Space but fragile for exotic chords.

8. **NSPanel wrapping re-applies on every show.** `show_palette()` calls `apply_palette_style` + `order_front_and_make_key` each time. This is intentional self-healing if the initial 200ms hook missed the window. Don't optimize it away.

9. **Session resume from disk can fail silently.** `resume_session` spawns a one-shot `messages` mode sidecar. If it returns empty or errors, the chat panel is empty with no banner. Consider adding an error indicator to `SessionInfo` loading.

10. **Custom `SlashpadPanel` NSPanel subclass.** `src/platform/macos.rs::slashpad_panel_class()` registers a subclass of `NSPanel` at runtime via `objc2::declare::ClassBuilder`, overriding `canBecomeKeyWindow` and `canBecomeMainWindow` to return `YES`. `apply_palette_style` uses `object_setClass` to swap iced/winit's plain `NSWindow` into this subclass. **This is load-bearing**: without it, (a) `NSWindowStyleMaskNonactivatingPanel` is silently ignored because iced's window isn't an NSPanel, (b) the palette won't float over fullscreen apps, and (c) keyboard input stops reaching the first responder because borderless nonactivating panels refuse to become key by default. `tauri-nspanel` does the same thing in the old codebase via its `tauri_panel!` macro. Don't revert it.

11. **objc2 0.5 `MethodImplementation` HRTB gotcha.** When registering Objective-C methods via `ClassBuilder::add_method`, the receiver type must be `*mut AnyObject` (raw pointer), not `&AnyObject`. The reference form triggers a `for<'a>` higher-rank lifetime bound that objc2 0.5's `MethodImplementation` impl cannot satisfy (the impl is for a single `'0`, not HRTB). Raw pointers have no lifetime and work cleanly. See `slashpad_panel_class()` for the canonical pattern.

12. **`window_id` is cached, not guaranteed.** `Slashpad::window_id: Option<iced::window::Id>` is populated asynchronously via `iced::window::get_oldest().map(Message::WindowIdResolved)` in the init task. There's a theoretical race where a hotkey press before iced's first frame would show the palette without iced-level resize/move (degraded but functional; the next press works). If you see first-show sizing glitches, investigate here.

13. **cargo-watch rebuild lifecycle on macOS.** Plain `cargo watch -x run` leaves orphan binaries because iced/winit's NSApplication event loop ignores SIGHUP (cargo-watch's default kill signal). The orphan holds the global hotkey registration, so the newly-built binary fails `manager.register()` silently and pressing Ctrl+Space still triggers the stale binary. Use `cargo watch --signal SIGINT -x run` instead — winit treats SIGINT as a close request and exits cleanly. Eventual real fix is TODO #9 (graceful shutdown).

10. **The `External` bus carries three event kinds.** Hotkey presses, sidecar events, and background-loaded data (recent sessions, history). Adding a new async-loaded thing means adding a new variant to `External` and a matching case in `external_subscription_stream`.

## Architecture recap (the short version)

Read `CLAUDE.md` for the full tour. The essentials:

- **Single binary**, `cargo run` launches it.
- **Three layers**: iced UI (`src/app.rs`, `src/ui/`) → platform services (`src/hotkey.rs`, `src/platform/macos.rs`, `src/settings.rs`, `src/skills.rs`) → sidecar client (`src/sidecar/`).
- **State machine** (`src/app.rs::Slashpad`) is the direct port of `usePalette.ts`. Mode: `Idle` → `Skills` → `Chatting` → `Settings`.
- **External event bus** (`src/app.rs::External` + the `EXTERNAL_RX`/`EXTERNAL_TX` statics) bridges non-iced threads (hotkey, sidecar) into the iced update loop via a `Subscription::run`.
- **Node sidecar** (`agent/runner.mjs`) is **unchanged** and is the only way to talk to the Claude Agent SDK. Don't touch it.

## Development loop

```bash
# Fast edit-time feedback
cargo check                  # ~0.5s after first build

# Full build + run
cargo run

# Strict lint (same as the stop hook)
cargo clippy --all-targets --all-features -- -D warnings

# Automated rebuild-on-save (recommended)
cargo install cargo-watch
cargo watch -x run
```

**Hooks** (`.claude/hooks/`):
- `cargo-check.sh` — runs after `.rs` file edits, blocks if check fails
- `cargo-clippy.sh` — runs as a stop hook, blocks if clippy fails under `-D warnings`

Both point at the repo-root `Cargo.toml`. If you move the manifest, update both scripts.

**The user's CLAUDE.md rule**: "Never run dev, build, test, or any other scripts without explicit instruction." This instance had explicit permission ("do everything yourself"). A fresh session should re-ask before running `cargo run` / `npm install`.

## File map

```
slashpad/
├── Cargo.toml, Cargo.lock
├── src/
│   ├── main.rs               # entry point, tokio runtime, iced::application
│   ├── app.rs                # Slashpad state + Message enum + update/view/subscription
│   ├── state.rs              # Mode, Skill, ChatMessageView, ContentBlock, SessionInfo
│   ├── hotkey.rs             # global-hotkey parse + register + poller thread
│   ├── settings.rs           # ~/.slashpad/settings.json serde I/O
│   ├── skills.rs             # SKILL.md loader + bundled skill seeding
│   ├── sessions.rs           # list_recent() + load_messages() via sidecar one-shots
│   ├── fuzzy.rs              # nucleo-matcher wrapper for skill filter
│   ├── markdown.rs           # pulldown-cmark → plain text (TODO: rich rendering)
│   ├── tray.rs               # STUB
│   ├── platform/
│   │   ├── mod.rs
│   │   ├── macos.rs          # NSPanel wrap, dispatch_main_async, cursor monitor
│   │   └── stub.rs           # non-macOS no-ops
│   ├── sidecar/
│   │   ├── mod.rs            # public SpawnedSidecar + slashpad_home()
│   │   ├── events.rs         # SidecarEvent serde enum (matches runner.mjs JSONL)
│   │   ├── payload.rs        # Chat/List/Messages payload + base64 encoding + SYSTEM_PROMPT
│   │   └── process.rs        # tokio::process + stdin writer + stdout reader tasks
│   └── ui/
│       ├── mod.rs
│       ├── theme.rs          # dark palette + palette_window_settings()
│       ├── command_input.rs  # the top bar
│       ├── skill_list.rs
│       ├── session_list.rs
│       ├── chat_panel.rs     # scrollable messages
│       ├── tool_line.rs      # single tool call row
│       └── settings.rs       # API key + hotkey display
├── agent/
│   └── runner.mjs            # UNCHANGED — the Node sidecar
├── bundled-skills/
│   └── skill-creator/        # include_dir! — must exist at build time
├── icons/                    # tray/dock icons (currently unused)
├── package.json              # slimmed to just @anthropic-ai/claude-agent-sdk
├── node_modules/             # MUST be populated (npm install)
├── README.md                 # user-facing
├── CLAUDE.md                 # architecture for Claude Code sessions
└── HANDOFF.md                # ← this file
```

## Git state when handed off

Nothing has been committed. `git status` should show:
- All deleted: `src-tauri/`, `src_react_legacy/` (temporarily existed), `index.html`, `vite.config.ts`, `tsconfig.json`, `tailwind.config.js`, `postcss.config.js`
- All added: `Cargo.toml`, `Cargo.lock`, `src/` (new Rust tree), `HANDOFF.md`
- Modified: `README.md`, `CLAUDE.md`, `package.json`, `.claude/hooks/cargo-*.sh`
- Moved (git will see as delete+add): `bundled-skills/`, `icons/`

Run `git status` before deciding what to commit. The rewrite is one logical change — a single commit makes sense, but the user should drive that call.
