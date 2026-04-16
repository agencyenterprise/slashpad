//! Iced daemon — multi-window state machine.
//!
//! Two windows live under a single `iced::daemon` application:
//!
//! - **Palette**: the long-lived launcher window. Opened once at startup,
//!   then shown/hidden via raw `orderFrontRegardless` / `orderOut` so the
//!   NSPanel class-swap and style mask persist across hotkey toggles.
//! - **Settings**: a compact, tray-anchored panel. Opened on demand via
//!   `iced::window::open(...)` with a `Position::Specific` under the tray
//!   icon, closed on Esc / blur / CloseSettings, or when the user clicks
//!   Save or Quit. Each open creates a fresh window id; the close_events
//!   subscription drains the cleanup.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, OnceLock};

use iced::futures::{SinkExt, Stream};
use iced::widget::{column, container, mouse_area, scrollable, text_input, Column, Space};
use iced::{Element, Subscription, Task, Theme};

/// Stable scrollable ids so `scrollable::snap_to` can target the skill and
/// idle lists across redraws.
static SKILL_LIST_SCROLL_ID: LazyLock<scrollable::Id> =
    LazyLock::new(scrollable::Id::unique);
static IDLE_LIST_SCROLL_ID: LazyLock<scrollable::Id> =
    LazyLock::new(scrollable::Id::unique);
static PROJECT_PICKER_SCROLL_ID: LazyLock<scrollable::Id> =
    LazyLock::new(scrollable::Id::unique);
/// Stable scrollable id for the chat panel. Only one chat is visible at a
/// time, so a single shared id is fine — it's how we target
/// `scrollable::snap_to` / `scrollable::scroll_to` for autoscroll and
/// keyboard scrolling.
static CHAT_SCROLL_ID: LazyLock<scrollable::Id> =
    LazyLock::new(scrollable::Id::unique);

/// Snap a scrollable so the row at `index` (in a list of `count`) is visible.
/// Uses a fractional offset: index 0 -> top, last index -> bottom.
fn snap_to_selection(id: scrollable::Id, index: usize, count: usize) -> Task<Message> {
    let y = if count <= 1 {
        0.0
    } else {
        index as f32 / (count - 1) as f32
    };
    scrollable::snap_to(id, scrollable::RelativeOffset { x: 0.0, y })
}

/// Tilde-abbreviate `$HOME` in a path for display (e.g.
/// `/Users/foo/.slashpad` → `~/.slashpad`). Used by the hotkeys bar to
/// show the current Claude project directory.
fn display_project_path(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            if let Some(rest) = s.strip_prefix(&home) {
                return format!("~{}", rest);
            }
        }
    }
    s.into_owned()
}

/// Pin the chat scrollable to the bottom. Runs against the *next* laid-out
/// frame, so it sees freshly appended content and lands at y=content_end.
/// Resolve the full path to `brew`. Needed because launchd services
/// don't inherit the user's shell PATH.
fn find_brew() -> Option<&'static str> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .into_iter()
        .find(|path| std::path::Path::new(path).exists())
}

fn snap_chat_to_bottom() -> Task<Message> {
    scrollable::snap_to(
        CHAT_SCROLL_ID.clone(),
        scrollable::RelativeOffset { x: 0.0, y: 1.0 },
    )
}

/// Pixels scrolled per Up/Down keypress while the chat panel is focused.
const CHAT_SCROLL_STEP: f32 = 60.0;
use tokio::sync::mpsc;

use crate::hotkey;
use crate::projects::ProjectInfo;
use crate::settings::{AppSettings, PreferredTerminal};
use crate::sidecar::{self, FollowUp, Payload, SidecarEvent, SpawnedSidecar};
use crate::skills;
use crate::state::{
    ChatId, ChatMessageView, ChatState, ChatStatus, Mode, Pin, SessionInfo, Skill,
};
use crate::ui;

/// A single running (or resumed) chat: its logical state plus, if the
/// sidecar is alive, the process handle. `sidecar` is `None` for a chat
/// that was just resumed-from-disk and hasn't had a follow-up sent yet.
pub struct ChatEntry {
    pub state: ChatState,
    pub sidecar: Option<SpawnedSidecar>,
    /// View-only scroll bookkeeping for the chat panel. Tracks whether the
    /// user wants to pin to the bottom (autoscroll) and the last-known
    /// scrollable geometry so keyboard scrolling can compute clamped
    /// absolute offsets. Not persisted.
    pub scroll: ChatScrollState,
}

/// View-only scroll state for a chat. `autoscroll` defaults to `true` so new
/// chats / newly-opened chats pin to the bottom. The geometry fields are
/// populated by the `ChatScrolled` handler as `on_scroll` fires.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChatScrollState {
    pub autoscroll: bool,
    pub offset: scrollable::AbsoluteOffset,
    pub viewport_h: f32,
    pub content_h: f32,
}

impl ChatScrollState {
    fn new() -> Self {
        Self {
            autoscroll: true,
            offset: scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
            viewport_h: 0.0,
            content_h: 0.0,
        }
    }

    /// Maximum vertical scroll offset given the last-known geometry. Zero
    /// when content fits within the viewport.
    fn max_y(&self) -> f32 {
        (self.content_h - self.viewport_h).max(0.0)
    }
}

/// Unified idle-list selection row used by `handle_submit` and nav.
/// The view layer has its own ref-based `IdleRow` in `ui/idle_list.rs`;
/// this owns its data so it can round-trip through `&mut self`.
#[derive(Debug, Clone)]
enum IdleRowSelection {
    Active(ChatId),
    Past(SessionInfo),
}

/// External events that are produced off the iced thread (hotkey thread, sidecar
/// tasks) and need to be pumped into the iced event loop via a subscription.
#[derive(Debug)]
pub enum External {
    HotkeyPressed,
    Sidecar {
        chat_id: ChatId,
        event: SidecarEvent,
    },
    /// Per-chat sidecar forwarder exited — the runner.mjs process's
    /// stdout closed (exit, crash, external kill). Emitted exactly once
    /// per spawn so a dead chat can flip to `Closed` status instead of
    /// sitting as `Streaming` forever.
    SidecarClosed {
        chat_id: ChatId,
    },
    /// Background-loaded list of recent sessions.
    RecentSessions(Vec<SessionInfo>),
    /// Background-loaded list of directories Claude Code has been run
    /// in. Populates the Cmd+P picker.
    ProjectsLoaded(Vec<ProjectInfo>),
    /// Background-loaded history messages for a resumed session.
    /// Tagged with the `chat_id` of the entry the history belongs to —
    /// multiple resumes can be in flight concurrently.
    HistoryLoaded {
        chat_id: ChatId,
        messages: Vec<ChatMessageView>,
    },
    /// Update check completed. `Some(version)` if newer, `None` if current.
    UpdateAvailable(Option<String>),
    /// Left-click on the menu-bar tray icon — opens the settings window
    /// anchored below the tray icon. Coordinates are logical pixels in
    /// winit's top-left-origin system rooted at the primary monitor.
    TrayClicked {
        tray_x: f64,
        tray_y: f64,
        tray_w: f64,
        tray_h: f64,
    },
    /// Tray context-menu "Show Launcher" — same semantics as the hotkey.
    TrayMenuShow,
    /// Tray context-menu "Quit Slashpad" — graceful shutdown.
    TrayMenuQuit,
}

static EXTERNAL_RX: Mutex<Option<mpsc::UnboundedReceiver<External>>> = Mutex::new(None);
static EXTERNAL_TX: OnceLock<mpsc::UnboundedSender<External>> = OnceLock::new();

/// Initialize the external-event bridge. Call once at startup.
pub fn init_external_bus() {
    let (tx, rx) = mpsc::unbounded_channel::<External>();
    *EXTERNAL_RX.lock().unwrap() = Some(rx);
    EXTERNAL_TX
        .set(tx)
        .map_err(|_| ())
        .expect("init_external_bus called twice");
}

pub fn external_sender() -> mpsc::UnboundedSender<External> {
    EXTERNAL_TX
        .get()
        .cloned()
        .expect("external bus not initialized")
}

#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    Submit,
    /// Cmd+Enter while a skill is locked — spawn the chat but dismiss
    /// the palette immediately instead of switching to the Chatting view.
    FireAndForgetSubmit,
    EscapePressed,
    /// Ctrl+C while a chat turn is in flight — kills the sidecar,
    /// preserving any partial assistant bubble and leaving the chat
    /// ready for a follow-up (respawned via the resume path).
    CancelGeneration,
    /// Cmd+Backspace in the launcher — clears the input. Carries the
    /// window id so the handler can ignore the keystroke when it fired
    /// inside the Settings window (which has its own text input).
    ClearLauncherInput {
        window_id: iced::window::Id,
    },
    NavUp,
    NavDown,
    SelectSkill(usize),
    /// Click on a past (disk) session row in the idle list.
    SelectSession(usize),
    /// Click on an active chat row in the idle list — switches the
    /// palette to the chat view for that chat without spawning a new
    /// sidecar.
    SelectChat(ChatId),
    HotkeyPressed,
    /// An iced window lost focus. Carries the window id so we can dispatch
    /// palette-vs-settings blur handling separately.
    WindowBlurred(iced::window::Id),
    /// An iced window gained focus. Used to re-assert text input focus
    /// when the palette returns from the background.
    WindowFocused(iced::window::Id),
    /// An iced window closed. Fired by the `close_events` subscription so
    /// we can null out `palette_window_id` / `settings_window_id`.
    WindowClosed(iced::window::Id),
    SidecarEvent {
        chat_id: ChatId,
        event: SidecarEvent,
    },
    /// The per-chat sidecar forwarder task exited — reflect that the
    /// runner.mjs process is gone by flipping the chat's status to
    /// `Closed` (unless it was already `Idle`).
    SidecarClosed(ChatId),
    RecentSessionsLoaded(Vec<SessionInfo>),
    /// Project-picker list has finished loading from
    /// `~/.claude/projects/`.
    ProjectsLoaded(Vec<ProjectInfo>),
    /// Cmd+P pressed — switch to the project-picker mode and show the
    /// cached list. No-op in `Mode::Settings`.
    OpenProjectPicker,
    /// User clicked a row in the project picker. Carries the index
    /// into the current `filtered_projects` list.
    SelectProject(usize),
    HistoryLoaded {
        chat_id: ChatId,
        messages: Vec<ChatMessageView>,
    },
    /// Close the settings window (bound to the "esc" button in the
    /// settings panel header).
    CloseSettings,
    ApiKeyInputChanged(String),
    /// Toggle the show/hide state of the API key input so the user can
    /// verify what they pasted.
    ToggleApiKeyVisibility,
    /// Wipe the API key input and remove the saved key from settings.
    ClearApiKey,
    /// The "Use Claude subscription" checkbox was toggled. `true`
    /// means route through `claude login` (no API key forwarded);
    /// `false` reveals the API key input.
    UseSubscriptionToggled(bool),
    /// The "Load user-level Claude settings & skills" checkbox was
    /// toggled. When `true`, the sidecar receives `settingSources:
    /// ["user", "project"]` and the palette skill list is augmented
    /// with skills from `~/.claude/skills/`; when `false`, scope is
    /// limited to `~/.slashpad/`.
    LoadUserSettingsToggled(bool),
    /// Tray left-click → open a new settings window anchored below the
    /// tray icon at the given logical-pixel rect.
    TrayOpenSettings {
        tray_x: f64,
        tray_y: f64,
        tray_w: f64,
        tray_h: f64,
    },
    /// Tray menu "Quit Slashpad" → drop sidecar, exit process.
    QuitRequested,
    /// User clicked the hotkey button in settings → begin listening for a
    /// new chord.
    StartRecordHotkey,
    /// User pressed Esc or clicked the button a second time while
    /// recording → abort and keep the existing hotkey.
    CancelRecordHotkey,
    /// The recording subscription captured a full chord (at least one
    /// non-modifier key). Carries the canonical chord string produced by
    /// `hotkey::format_chord`.
    HotkeyCaptured(String),
    /// Animation tick for the "Working…" spinner in the chat panel.
    /// Emitted by a `time::every` subscription only while a turn is in
    /// flight.
    SpinnerTick,
    /// User clicked the tool-section expand/collapse chevron on an
    /// assistant message. Toggles `tools_expanded` on the identified
    /// message within the active chat.
    ToggleToolsExpanded(u64),
    /// A user clicked a link inside a rendered markdown message.
    /// Currently a no-op — we just log it. Plumbed because
    /// `iced::widget::markdown::view` returns an `Element<Url>` and
    /// we need a `Message` to map into.
    MarkdownLinkClicked(iced::widget::markdown::Url),
    /// The chat panel's `scrollable` fired `on_scroll`. Used to track the
    /// user's scroll position so we can (a) release autoscroll when they
    /// scroll away from the bottom, (b) re-engage it when they scroll
    /// back, and (c) compute clamped absolute offsets for Up/Down keys.
    ChatScrolled(scrollable::Viewport),
    /// Cmd+T pressed while in `Mode::Chatting` — open the active chat's
    /// session in the user's preferred terminal via `claude --resume`.
    /// Silently no-ops if not in Chatting mode or if the active chat
    /// has no `session_id` yet (first response hasn't arrived).
    OpenSessionInTerminal,
    /// The user picked a new terminal from the settings dropdown.
    PreferredTerminalChanged(PreferredTerminal),
    /// User pressed the mouse down inside the invisible drag strip at
    /// the top of the palette — kick off a native OS window drag that
    /// follows the cursor until the mouse button is released.
    DragWindow,
    /// Cmd+Shift+P — toggle the unified pin. Pinning snapshots the
    /// palette's current on-screen position and, if the user is viewing
    /// a chat, that chat id too — so summoning the palette later
    /// restores both. Pressing again fully unpins: the window snaps
    /// back to cursor-center on the next summon and the Skills prefill
    /// returns. See `crate::state::Pin` for lifecycle details.
    TogglePin,
    /// Resolution of the `iced::window::get_position` lookup kicked off
    /// by a fresh pin when the user hadn't already dragged the window.
    /// Arrives after the async runtime reads the palette's actual
    /// on-screen position; we commit it (together with the current
    /// chat id, if any) as `self.pinned`. `None` means the window
    /// wasn't available (shouldn't happen while the palette is
    /// visible, but handled as a no-op rather than panicking).
    CommitPin(Option<iced::Point>),
    /// Window position changed. Fired both by user drags and by our own
    /// programmatic `iced::window::move_to` calls; `update` distinguishes
    /// them via `programmatic_move_pending` so only user drags are
    /// persisted into `dragged_position`.
    WindowMoved {
        window_id: iced::window::Id,
        position: iced::Point,
    },
    /// Background update check completed. `Some(version)` if newer,
    /// `None` if already on the latest.
    UpdateAvailable(Option<String>),
    /// User clicked "Upgrade to vX.Y.Z" — runs `brew upgrade slashpad`.
    UpgradeClicked,
    /// The background `brew upgrade` finished (success or failure).
    UpgradeFinished(Result<(), String>),
}

/// Root application state.
pub struct Slashpad {
    pub input: String,
    pub mode: Mode,
    /// Whether the palette window is currently being displayed on screen.
    /// The palette NSWindow is long-lived and hidden via `orderOut`; this
    /// flag tracks the logical visibility state so `toggle_palette()` knows
    /// whether to show or hide it.
    pub palette_visible: bool,

    pub all_skills: Vec<Skill>,
    pub filtered_skills: Vec<Skill>,
    pub selected_skill_index: usize,

    /// All chats the user has started or resumed in this process —
    /// live (with an active `sidecar`), resumed-from-disk (no sidecar
    /// until first follow-up), completed, errored, or closed. Keyed by
    /// locally-allocated `ChatId` because the Claude `session_id` isn't
    /// known until after the first turn's `result` event.
    pub chats: BTreeMap<ChatId, ChatEntry>,
    /// Which chat is currently rendered in the chat panel. `None` means
    /// the palette is in the idle list view (or skills picker) — the
    /// other entries in `chats` keep streaming in the background.
    pub active_chat_id: Option<ChatId>,
    pub next_chat_id: ChatId,

    /// Monotonically incrementing animation frame counter for the
    /// "Working…" spinner shown in the chat panel while a turn is in
    /// flight and for animated status pills on idle-list rows. Driven
    /// by a `time::every` subscription that only runs while at least
    /// one chat is in a non-terminal state.
    pub spinner_frame: u32,

    pub recent_sessions: Vec<SessionInfo>,
    /// Unified selection index walking active chats first, then past
    /// sessions — used for up/down nav in Mode::Idle.
    pub selected_idle_index: usize,
    /// Has the user moved into the idle list via ↑/↓? Controls whether
    /// Enter with a non-empty input resumes the highlighted row (true)
    /// or starts a new chat with the typed text (false). Always true
    /// when the input is empty, so the historical empty-input Enter
    /// behavior is preserved.
    pub idle_selection_active: bool,

    pub settings: AppSettings,
    pub api_key_input: String,
    /// Whether the API key input is currently rendering in plaintext
    /// (show/hide toggle in the settings panel). Always starts `false`
    /// on each settings-window open.
    pub api_key_visible: bool,
    pub recording_hotkey: bool,
    /// Last error surfaced from `hotkey::update_hotkey`, shown under the
    /// hotkey button in the settings window. Cleared when recording starts
    /// or succeeds.
    pub hotkey_error: Option<String>,

    /// The persistent launcher window. Known as soon as
    /// `iced::window::open(...)` returns; the window itself is created
    /// asynchronously when the daemon processes the Open action.
    pub palette_window_id: Option<iced::window::Id>,
    /// The tray-anchored settings window. `Some` only while a settings
    /// window is currently open; set when the tray is clicked and cleared
    /// by the `WindowClosed` subscription when the user dismisses it.
    pub settings_window_id: Option<iced::window::Id>,
    /// Timestamp of the last settings-window open. Used to debounce the
    /// `Unfocused` event that AppKit fires during the initial activation
    /// hand-off — without NSPanel non-activating treatment, a fresh
    /// settings window activates the app briefly, then immediately blurs,
    /// which would close it before the user can interact. Any blur event
    /// within `SETTINGS_BLUR_GRACE_MS` of open is ignored.
    pub settings_opened_at: Option<std::time::Instant>,

    /// Directory Claude Code runs in (`cwd` passed to the sidecar).
    /// Loaded from `settings.selected_project_path` at startup (falling
    /// back to `~/.slashpad` if unset or missing), and updated by the
    /// Cmd+P project picker. New chats spawn with this as their cwd;
    /// in-flight chats keep whatever cwd they were spawned with.
    pub project_path: std::path::PathBuf,

    /// All directories Claude Code has been run in, loaded once at
    /// startup from `~/.claude/projects/` and sorted most-recent first.
    /// Source of truth for the Cmd+P picker.
    pub all_projects: Vec<ProjectInfo>,
    /// Current fuzzy-filtered view of `all_projects`. Rebuilt on every
    /// `InputChanged` while in `Mode::ProjectPicker`.
    pub filtered_projects: Vec<ProjectInfo>,
    /// Highlighted row in the picker. Clamped into `filtered_projects`
    /// by the nav / InputChanged handlers.
    pub selected_project_index: usize,
    /// Input saved when the user opens the picker — restored on Esc so
    /// backing out of Cmd+P doesn't destroy whatever they were typing.
    pub input_before_picker: Option<String>,

    /// Position the user has dragged the palette window to. When `Some`,
    /// `show_palette()` skips the cursor-center `move_to` and lets the
    /// NSPanel reappear wherever the user last left it. Reset to `None`
    /// whenever `start_chat` / `start_chat_detached` creates a fresh
    /// chat, so each new chat opens at the cursor again.
    pub dragged_position: Option<iced::Point>,
    /// The user's current pin (Cmd+Shift+P). Unified "stay put" state:
    /// captures the palette's on-screen position *and*, if pinned while
    /// viewing a chat, the chat id to reopen on every future summon.
    /// See `crate::state::Pin` for lifecycle details. In-memory only.
    pub pinned: Option<Pin>,
    /// Armed right before we issue `iced::window::move_to` so the
    /// `window::Event::Moved` fired by that programmatic move isn't
    /// mistaken for a user drag. Cleared by the first `WindowMoved`
    /// observed after arming.
    pub programmatic_move_pending: bool,

    /// Update check / upgrade lifecycle state. Driven by the settings
    /// title row — checked on each settings-window open.
    pub update_status: UpdateStatus,
}

#[derive(Debug, Clone, Default)]
pub enum UpdateStatus {
    /// No check performed yet (or settings haven't been opened).
    #[default]
    Idle,
    /// GitHub API request in flight.
    Checking,
    /// Running version is the latest.
    UpToDate,
    /// A newer version exists.
    Available(String),
    /// `brew update && brew upgrade slashpad` in progress.
    Upgrading,
}

impl Slashpad {
    pub fn new() -> (Self, Task<Message>) {
        // `init_external_bus()` is called from `main()` before iced starts,
        // so `external_sender()` is ready for the hotkey forwarder below.

        let settings = AppSettings::load_or_default();
        let all_skills = skills::load_skills(settings.load_user_settings).unwrap_or_default();

        // Resolve the starting project path from persisted settings,
        // falling back to `~/.slashpad` if unset or if the saved path
        // has been deleted since last run.
        let project_path: std::path::PathBuf = settings
            .selected_project_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| {
                sidecar::slashpad_home().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });

        // Spin up the hotkey manager — forwards presses into the external bus.
        match hotkey::spawn(&settings.hotkey) {
            Ok(mut rx) => {
                let tx = external_sender();
                tokio::spawn(async move {
                    while rx.recv().await.is_some() {
                        let _ = tx.send(External::HotkeyPressed);
                    }
                });
            }
            Err(e) => eprintln!("[slashpad] failed to register hotkey: {e}"),
        }

        // Kick off a background load of recent sessions for the idle
        // view — scoped to the currently-selected project.
        let tx = external_sender();
        let sessions_cwd = project_path.clone();
        tokio::spawn(async move {
            let sessions = crate::sessions::list_recent(&sessions_cwd)
                .await
                .unwrap_or_default();
            let _ = tx.send(External::RecentSessions(sessions));
        });

        // Kick off a background scan of `~/.claude/projects/` so the
        // Cmd+P picker has its list ready by the time the user opens it.
        let tx = external_sender();
        tokio::spawn(async move {
            let projects = crate::projects::list_known().await.unwrap_or_default();
            let _ = tx.send(External::ProjectsLoaded(projects));
        });

        // `iced::daemon` starts with no windows. Open the palette here; the
        // id is known immediately (even before the window exists), and
        // `open_palette_task` drives iced to actually create it.
        let (palette_id, open_palette_task) =
            iced::window::open(ui::theme::palette_window_settings());

        let state = Self {
            input: String::new(),
            mode: Mode::Idle,
            palette_visible: false,
            filtered_skills: all_skills.clone(),
            all_skills,
            selected_skill_index: 0,
            chats: BTreeMap::new(),
            active_chat_id: None,
            next_chat_id: 1,
            spinner_frame: 0,
            recent_sessions: Vec::new(),
            selected_idle_index: 0,
            idle_selection_active: true,
            api_key_input: String::new(),
            settings,
            api_key_visible: false,
            recording_hotkey: false,
            hotkey_error: None,
            palette_window_id: Some(palette_id),
            settings_window_id: None,
            settings_opened_at: None,
            project_path,
            all_projects: Vec::new(),
            filtered_projects: Vec::new(),
            selected_project_index: 0,
            input_before_picker: None,
            dragged_position: None,
            pinned: None,
            programmatic_move_pending: false,
            update_status: UpdateStatus::Idle,
        };

        // Tray icon creation must happen on the main thread AFTER the
        // NSApplication event loop is running; tray-icon's docs are
        // explicit about this. `dispatch_main_async` enqueues onto the
        // main dispatch queue, which is drained by NSApp's CFRunLoop once
        // `run_with` hands control to winit.
        #[cfg(target_os = "macos")]
        crate::platform::macos::dispatch_main_async(|| {
            crate::tray::init();
        });

        // Palette NSPanel treatment: swap the class to `SlashpadPanel`,
        // set the non-activating style mask / modal panel level /
        // fullscreen-auxiliary collection behavior, then orderOut so the
        // window starts hidden. Runs as an iced task so we can target the
        // exact window id via `run_with_handle` — no NSApp.windows()
        // guessing, which breaks as soon as a second iced window exists.
        let palette_style_task: Task<Message> =
            iced::window::run_with_handle(palette_id, |handle| {
                #[cfg(target_os = "macos")]
                unsafe {
                    let ns_window = crate::platform::macos::ns_window_from_handle(&handle);
                    crate::platform::macos::apply_palette_style(ns_window);
                    crate::platform::macos::order_out(ns_window);
                }
                #[cfg(not(target_os = "macos"))]
                let _ = handle;
            })
            .discard();

        // Sequencing matters: the palette NSWindow must exist before
        // `run_with_handle` fires, and the text_input focus targets the
        // palette's command input so it wants a live window too.
        // `.chain()` runs the next task only after the previous one
        // resolves; `open_palette_task` resolves when the Open action
        // has been fully processed by the runtime.
        let init = open_palette_task
            .discard::<Message>()
            .chain(palette_style_task)
            .chain(text_input::focus(INPUT_ID.clone()));
        (state, init)
    }

    pub fn title(&self, window_id: iced::window::Id) -> String {
        if Some(window_id) == self.settings_window_id {
            "Slashpad Settings".to_string()
        } else {
            "Slashpad".to_string()
        }
    }

    pub fn theme(&self, _window_id: iced::window::Id) -> Theme {
        ui::theme::dark_theme()
    }

    /// Root `Appearance` for the iced daemon. Overrides the default
    /// (which clears to `palette.background.base.color` — our SURFACE_0)
    /// with a fully transparent clear so the rounded inner containers
    /// are the only thing drawn onto the transparent NSWindow.
    pub fn style(&self, _theme: &Theme) -> iced::daemon::Appearance {
        iced::daemon::Appearance {
            background_color: iced::Color::TRANSPARENT,
            text_color: ui::theme::TEXT,
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;
        let mut subs: Vec<Subscription<Message>> = vec![
            Subscription::run(external_subscription_stream),
            // ArrowUp/Down propagate normally through text_input (it returns
            // Status::Ignored for them), so on_key_press is fine here.
            // Enter is handled in listen_with below so Cmd+Enter can
            // dispatch FireAndForgetSubmit without racing on_submit.
            iced::keyboard::on_key_press(|key, _modifiers| match key.as_ref() {
                Key::Named(Named::ArrowUp) => Some(Message::NavUp),
                Key::Named(Named::ArrowDown) => Some(Message::NavDown),
                _ => None,
            }),
            // Shortcut detection + window events. `listen_with` (not
            // `on_key_press`) because iced's focused `text_input` returns
            // `Status::Captured` for character keys and Escape — which
            // hides them from `on_key_press`. `listen_with` sees events
            // regardless of capture status, so we get the first press.
            //
            // All shortcut detection is centralized in
            // `Slashpad::decode_shortcut`; leak prevention for modifier+
            // letter chords is handled by `ui::shortcut_filter::ShortcutFilter`,
            // which wraps the launcher `text_input` and drops matching
            // `KeyPressed` events before the widget can insert the letter.
            iced::event::listen_with(|event, _status, window_id| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key, modifiers, ..
                }) => Self::decode_shortcut(&key, modifiers, window_id),
                iced::Event::Window(iced::window::Event::Unfocused) => {
                    Some(Message::WindowBlurred(window_id))
                }
                iced::Event::Window(iced::window::Event::Focused) => {
                    Some(Message::WindowFocused(window_id))
                }
                iced::Event::Window(iced::window::Event::Moved(point)) => {
                    Some(Message::WindowMoved {
                        window_id,
                        position: point,
                    })
                }
                _ => None,
            }),
            iced::window::close_events().map(Message::WindowClosed),
        ];

        // Spinner animation: tick while *any* chat is non-terminal, so
        // both the active-chat "Working…" spinner and animated status
        // pills on idle-list rows stay live. Gated off when all chats
        // are at rest (Idle / Error / Closed) or when there are no
        // chats at all, to avoid pointless redraws.
        let any_chat_live = self.chats.values().any(|c| {
            matches!(
                c.state.status,
                ChatStatus::Initializing | ChatStatus::Streaming
            )
        });
        if any_chat_live {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(80))
                    .map(|_| Message::SpinnerTick),
            );
        }

        // Hotkey recorder: only active while the user is capturing a new
        // chord. `listen_with` (not `on_key_press`) so it sees events even
        // when the API-key text_input is focused and would otherwise
        // capture letter presses.
        if self.recording_hotkey {
            subs.push(iced::event::listen_with(|event, _status, _window_id| {
                match event {
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key,
                        modifiers,
                        ..
                    }) => {
                        // Let the existing Esc handler route this to
                        // EscapePressed, which cancels recording.
                        if matches!(key, Key::Named(Named::Escape)) {
                            return None;
                        }
                        hotkey::format_chord(&key, modifiers).map(Message::HotkeyCaptured)
                    }
                    _ => None,
                }
            }));
        }

        Subscription::batch(subs)
    }

    /// Central keyboard shortcut decoder. Single source of truth for
    /// every app-bound keypress. Pairs with
    /// `should_filter_launcher_keypress` — adding a `Cmd`+letter arm
    /// here automatically enables `ShortcutFilter` to drop the
    /// corresponding event before the launcher `text_input` sees it
    /// (otherwise iced's text_input would insert the letter and render
    /// a one-frame flash before we could strip it).
    fn decode_shortcut(
        key: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
        window_id: iced::window::Id,
    ) -> Option<Message> {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;

        if matches!(key.as_ref(), Key::Named(Named::Enter)) {
            return Some(if modifiers.command() {
                Message::FireAndForgetSubmit
            } else {
                Message::Submit
            });
        }
        if matches!(key.as_ref(), Key::Named(Named::Escape)) {
            return Some(Message::EscapePressed);
        }
        if modifiers.command() && matches!(key.as_ref(), Key::Named(Named::Backspace)) {
            return Some(Message::ClearLauncherInput { window_id });
        }
        if modifiers.control() && matches!(key.as_ref(), Key::Character("c")) {
            return Some(Message::CancelGeneration);
        }

        // Cmd+letter shortcuts. `ShortcutFilter` prevents the letter
        // from leaking into text_input; we don't need post-hoc stripping.
        // Order matters: Cmd+Shift+P must be checked before plain Cmd+P
        // since the shifted chord would otherwise fall through to the
        // project picker.
        if modifiers.command()
            && modifiers.shift()
            && matches!(key.as_ref(), Key::Character("p") | Key::Character("P"))
        {
            return Some(Message::TogglePin);
        }
        if modifiers.command() && matches!(key.as_ref(), Key::Character("p")) {
            return Some(Message::OpenProjectPicker);
        }
        if modifiers.command() && matches!(key.as_ref(), Key::Character("t")) {
            return Some(Message::OpenSessionInTerminal);
        }
        None
    }

    /// Predicate used by `ui::shortcut_filter::ShortcutFilter` on the
    /// launcher `text_input`: returns `true` when the keypress matches
    /// a `Cmd`+letter shortcut that, if allowed through, would leak its
    /// letter into the text buffer and render a one-frame flash.
    ///
    /// Derived from `decode_shortcut` so the filter stays in lockstep —
    /// adding a new `Cmd`+letter arm there automatically activates
    /// filtering here, with no separate registry to maintain.
    ///
    /// Returns `false` for unbound `Cmd`+letter combos (e.g. `Cmd+C`,
    /// `Cmd+V`, `Cmd+X`, `Cmd+A`) so text_input's built-in clipboard
    /// and select-all handling keeps working. Returns `false` for
    /// non-character keys (Enter/Escape/Backspace/arrows), which
    /// text_input never inserts regardless of modifier state.
    pub fn should_filter_launcher_keypress(
        key: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> bool {
        use iced::keyboard::Key;
        if !modifiers.command() || !matches!(key.as_ref(), Key::Character(_)) {
            return false;
        }
        Self::decode_shortcut(key, modifiers, iced::window::Id::unique()).is_some()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        self.dispatch_update(message)
    }

    fn dispatch_update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(value) => {
                // `ShortcutFilter` (wrapping the launcher text_input)
                // drops modifier+letter shortcuts before text_input can
                // insert the letter, so there's no leaked character to
                // strip here — InputChanged reflects real user typing.
                self.input = value.clone();

                // Skill filtering
                if self.mode == Mode::Chatting {
                    return Task::none();
                }
                // Project picker owns the input while it's open: every
                // keystroke rebuilds the fuzzy-filtered list. Exits via
                // Esc or Submit; we never fall through to the Skills /
                // Idle logic below while the picker is up.
                if self.mode == Mode::ProjectPicker {
                    self.filtered_projects = if self.input.is_empty() {
                        self.all_projects.clone()
                    } else {
                        crate::fuzzy::filter_projects(&self.all_projects, &self.input)
                    };
                    let max = self.filtered_projects.len().saturating_sub(1);
                    if self.selected_project_index > max {
                        self.selected_project_index = 0;
                    }
                    return Task::none();
                }
                if let Some(query) = value.strip_prefix('/') {
                    self.filtered_skills = if query.is_empty() {
                        self.all_skills.clone()
                    } else {
                        crate::fuzzy::filter_skills(&self.all_skills, query)
                    };
                    self.mode = Mode::Skills;
                    self.selected_skill_index = 0;
                } else {
                    if self.mode == Mode::Skills {
                        self.mode = Mode::Idle;
                        self.filtered_skills.clear();
                    }
                    // Fuzzy-filtering the idle list in place. Empty
                    // input → selection active (Enter resumes top
                    // row, matching the original behavior). Typed
                    // input → selection *not* active, so Enter
                    // starts a new chat with the typed text unless
                    // the user arrow-keys into the filtered list.
                    self.selected_idle_index = 0;
                    self.idle_selection_active = self.input.is_empty();
                }
                Task::none()
            }

            Message::ClearLauncherInput { window_id } => {
                // The Settings window has its own API-key text_input — let
                // Cmd+Backspace do whatever the OS/iced defaults do there
                // instead of stomping on it.
                if Some(window_id) == self.settings_window_id {
                    return Task::none();
                }
                if self.input.is_empty() {
                    return Task::none();
                }
                self.input.clear();
                // Mirror the Skills → Idle transition in `InputChanged`
                // so dismissing `/foo` via Cmd+Backspace also collapses
                // the skills list.
                if self.mode == Mode::Skills {
                    self.mode = Mode::Idle;
                    self.filtered_skills.clear();
                }
                // Back to an empty-input idle state: re-engage the
                // idle selection so Enter resumes the top row.
                self.selected_idle_index = 0;
                self.idle_selection_active = true;
                Task::none()
            }

            Message::Submit => self.handle_submit(),

            Message::FireAndForgetSubmit => {
                let prompt = self.input.trim().to_string();
                match self.mode {
                    Mode::Skills if self.locked_skill().is_some() => {
                        self.start_chat_detached(prompt)
                    }
                    Mode::Idle
                        if !prompt.is_empty() && !prompt.starts_with('/') =>
                    {
                        self.start_chat_detached(prompt)
                    }
                    _ => Task::none(),
                }
            }

            Message::EscapePressed => {
                // If a hotkey recording is in progress, Esc cancels the
                // recording rather than closing the settings window.
                if self.recording_hotkey {
                    self.recording_hotkey = false;
                    self.hotkey_error = None;
                    return Task::none();
                }
                // Settings window takes precedence: close it if it's open.
                // The on_key_press subscription fires regardless of which
                // iced window is focused, so this handles both "Esc while
                // settings is focused" and "Esc while palette is focused
                // with settings still open somewhere".
                if let Some(settings_id) = self.settings_window_id {
                    return iced::window::close(settings_id);
                }
                // ProjectPicker → back to Idle, restoring whatever the
                // user had typed before Cmd+P. Does NOT dismiss the
                // palette, matching the Skills/Chatting back pattern.
                if self.mode == Mode::ProjectPicker {
                    self.mode = Mode::Idle;
                    self.input = self.input_before_picker.take().unwrap_or_default();
                    self.filtered_projects.clear();
                    self.selected_project_index = 0;
                    self.selected_idle_index = 0;
                    self.idle_selection_active = self.input.is_empty();
                    return text_input::focus(INPUT_ID.clone());
                }
                // In Chatting mode, Esc steps back to the idle thread
                // list instead of dismissing. The sidecar keeps streaming
                // in the background and the entry stays in `self.chats`,
                // so the user can pick a different chat (or re-enter this
                // one) from the list. A second Esc from the idle list
                // dismisses the palette.
                if self.mode == Mode::Chatting {
                    self.active_chat_id = None;
                    self.mode = Mode::Idle;
                    self.input.clear();
                    self.selected_idle_index = 0;
                    return Task::batch([
                        text_input::focus(INPUT_ID.clone()),
                        snap_to_selection(IDLE_LIST_SCROLL_ID.clone(), 0, self.idle_row_count()),
                    ]);
                }
                self.hide_palette()
            }

            Message::CancelGeneration => {
                let Some(chat_id) = self.active_chat_id else {
                    return Task::none();
                };
                let Some(entry) = self.chats.get_mut(&chat_id) else {
                    return Task::none();
                };
                if !matches!(
                    entry.state.status,
                    ChatStatus::Initializing | ChatStatus::Streaming
                ) {
                    return Task::none();
                }
                // Dropping the SpawnedSidecar triggers kill_on_drop on
                // the Child and closes stdin, which ends the writer
                // task. The reader task exits when stdout closes.
                // `session_id` (if already assigned) is preserved on
                // ChatState, so the next submit respawns via the
                // existing resume path in `send_follow_up`.
                entry.sidecar = None;
                entry.state.mark_cancelled();
                Task::none()
            }

            Message::NavUp => {
                match self.mode {
                    Mode::Skills => {
                        if self.selected_skill_index > 0 {
                            self.selected_skill_index -= 1;
                        }
                        snap_to_selection(
                            SKILL_LIST_SCROLL_ID.clone(),
                            self.selected_skill_index,
                            self.filtered_skills.len(),
                        )
                    }
                    Mode::ProjectPicker => {
                        if self.selected_project_index > 0 {
                            self.selected_project_index -= 1;
                        }
                        snap_to_selection(
                            PROJECT_PICKER_SCROLL_ID.clone(),
                            self.selected_project_index,
                            self.filtered_projects.len(),
                        )
                    }
                    Mode::Idle if !self.input.starts_with('/') && self.idle_row_count() > 0 => {
                        let count = self.idle_row_count();
                        // First arrow press while typing engages the
                        // selection without moving it (lands on row 0).
                        if !self.idle_selection_active {
                            self.idle_selection_active = true;
                            self.selected_idle_index = 0;
                        } else if self.selected_idle_index > 0 {
                            self.selected_idle_index -= 1;
                        }
                        if self.selected_idle_index >= count {
                            self.selected_idle_index = count.saturating_sub(1);
                        }
                        snap_to_selection(
                            IDLE_LIST_SCROLL_ID.clone(),
                            self.selected_idle_index,
                            count,
                        )
                    }
                    Mode::Chatting => self.chat_scroll_by(-CHAT_SCROLL_STEP),
                    _ => Task::none(),
                }
            }

            Message::NavDown => {
                match self.mode {
                    Mode::Skills => {
                        let max = self.filtered_skills.len().saturating_sub(1);
                        if self.selected_skill_index < max {
                            self.selected_skill_index += 1;
                        }
                        snap_to_selection(
                            SKILL_LIST_SCROLL_ID.clone(),
                            self.selected_skill_index,
                            self.filtered_skills.len(),
                        )
                    }
                    Mode::ProjectPicker => {
                        let max = self.filtered_projects.len().saturating_sub(1);
                        if self.selected_project_index < max {
                            self.selected_project_index += 1;
                        }
                        snap_to_selection(
                            PROJECT_PICKER_SCROLL_ID.clone(),
                            self.selected_project_index,
                            self.filtered_projects.len(),
                        )
                    }
                    Mode::Idle if !self.input.starts_with('/') && self.idle_row_count() > 0 => {
                        let count = self.idle_row_count();
                        let max = count.saturating_sub(1);
                        // First arrow press while typing engages the
                        // selection without moving it (lands on row 0).
                        if !self.idle_selection_active {
                            self.idle_selection_active = true;
                            self.selected_idle_index = 0;
                        } else if self.selected_idle_index < max {
                            self.selected_idle_index += 1;
                        }
                        if self.selected_idle_index > max {
                            self.selected_idle_index = max;
                        }
                        snap_to_selection(
                            IDLE_LIST_SCROLL_ID.clone(),
                            self.selected_idle_index,
                            count,
                        )
                    }
                    Mode::Chatting => self.chat_scroll_by(CHAT_SCROLL_STEP),
                    _ => Task::none(),
                }
            }

            Message::SelectSkill(i) => {
                self.selected_skill_index = i;
                self.handle_submit()
            }

            Message::SelectSession(session_index) => {
                // `session_index` is an index into the *past sessions*
                // portion of the idle list (post fuzzy-filter + dupe
                // removal). The caller already passes the correct
                // filtered index from the view builder.
                let past = self.visible_past_session_rows();
                if let Some(session) = past.get(session_index).cloned() {
                    self.resume_session(session)
                } else {
                    Task::none()
                }
            }

            Message::SelectChat(chat_id) => {
                if let Some(entry) = self.chats.get_mut(&chat_id) {
                    entry.scroll.autoscroll = true;
                    self.active_chat_id = Some(chat_id);
                    self.mode = Mode::Chatting;
                    self.input.clear();
                    Task::batch([
                        text_input::focus(INPUT_ID.clone()),
                        snap_chat_to_bottom(),
                    ])
                } else {
                    Task::none()
                }
            }

            Message::HotkeyPressed => self.toggle_palette(),

            Message::SidecarEvent { chat_id, event } => self.process_sidecar_event(chat_id, event),

            Message::SidecarClosed(chat_id) => self.process_sidecar_closed(chat_id),

            Message::RecentSessionsLoaded(sessions) => {
                self.recent_sessions = sessions;
                let max = self.idle_row_count().saturating_sub(1);
                if self.selected_idle_index > max {
                    self.selected_idle_index = 0;
                }
                Task::none()
            }

            Message::ProjectsLoaded(projects) => {
                self.all_projects = projects;
                // If the picker happens to be open when results arrive
                // (e.g. the user opened it immediately after launch,
                // before the scan finished), replay the current input
                // through the new list so they see filtered results.
                if self.mode == Mode::ProjectPicker {
                    self.filtered_projects = if self.input.is_empty() {
                        self.all_projects.clone()
                    } else {
                        crate::fuzzy::filter_projects(&self.all_projects, &self.input)
                    };
                    self.selected_project_index = 0;
                }
                Task::none()
            }

            Message::UpdateAvailable(version) => {
                // `None` means the running version is already the latest.
                self.update_status = match version {
                    Some(v) => UpdateStatus::Available(v),
                    None => UpdateStatus::UpToDate,
                };
                Task::none()
            }

            Message::UpgradeClicked => {
                if matches!(self.update_status, UpdateStatus::Upgrading) {
                    return Task::none();
                }
                self.update_status = UpdateStatus::Upgrading;
                Task::perform(
                    async {
                        let brew = find_brew().ok_or_else(|| {
                            "brew not found — install Homebrew or upgrade manually".to_string()
                        })?;

                        // Fetch the latest formulae so brew knows about
                        // the new release.
                        let update = tokio::process::Command::new(brew)
                            .arg("update")
                            .output()
                            .await
                            .map_err(|e| e.to_string())?;
                        if !update.status.success() {
                            return Err(String::from_utf8_lossy(&update.stderr).to_string());
                        }

                        let upgrade = tokio::process::Command::new(brew)
                            .args(["upgrade", "slashpad"])
                            .output()
                            .await
                            .map_err(|e| e.to_string())?;
                        if !upgrade.status.success() {
                            return Err(String::from_utf8_lossy(&upgrade.stderr).to_string());
                        }

                        Ok(())
                    },
                    Message::UpgradeFinished,
                )
            }

            Message::UpgradeFinished(result) => {
                match result {
                    Ok(()) => {
                        // The new binary is installed. Restart via brew
                        // services so the updated version takes over.
                        self.chats.clear();
                        if let Some(brew) = find_brew() {
                            let _ = std::process::Command::new(brew)
                                .args(["services", "restart", "slashpad"])
                                .spawn();
                        }
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("[slashpad] brew upgrade failed: {e}");
                        // Fall back to showing the update button again.
                        self.update_status = UpdateStatus::Idle;
                    }
                }
                Task::none()
            }

            Message::OpenProjectPicker => {
                // Settings window open → skip; ProjectPicker already
                // open → no-op (don't reset selection/input on a
                // double-press).
                if self.settings_window_id.is_some() || self.mode == Mode::ProjectPicker {
                    return Task::none();
                }
                self.input_before_picker = Some(std::mem::take(&mut self.input));
                self.filtered_projects = self.all_projects.clone();
                self.selected_project_index = 0;
                self.mode = Mode::ProjectPicker;
                // `ShortcutFilter` drops the Cmd+P event before
                // text_input can insert the 'p' — no leak to clean up.
                text_input::focus(INPUT_ID.clone())
            }

            Message::SelectProject(i) => {
                self.selected_project_index = i;
                self.handle_submit()
            }

            Message::HistoryLoaded { chat_id, messages } => {
                if let Some(entry) = self.chats.get_mut(&chat_id) {
                    // Reset the per-chat message id counter to walk
                    // above whatever ids the loader handed us.
                    let max_id = messages.iter().map(|m| m.id).max().unwrap_or(0);
                    entry.state.messages = messages;
                    entry.state.next_msg_id = max_id + 1;
                    entry.state.status = ChatStatus::Idle;
                }
                // If the resumed chat is what the user is looking at,
                // pin to the bottom now that the history has landed.
                if self.active_chat_id == Some(chat_id) && self.mode == Mode::Chatting {
                    snap_chat_to_bottom()
                } else {
                    Task::none()
                }
            }

            Message::CloseSettings => {
                if let Some(id) = self.settings_window_id {
                    iced::window::close(id)
                } else {
                    Task::none()
                }
            }

            Message::WindowBlurred(window_id) => {
                if Some(window_id) == self.settings_window_id {
                    // Settings window: close on blur, matching the Tauri
                    // behavior — BUT ignore the spurious Unfocused event
                    // AppKit fires during initial activation hand-off,
                    // within `SETTINGS_BLUR_GRACE_MS` of open.
                    let grace = std::time::Duration::from_millis(Self::SETTINGS_BLUR_GRACE_MS);
                    let within_grace = self
                        .settings_opened_at
                        .map(|t| t.elapsed() < grace)
                        .unwrap_or(false);
                    if within_grace {
                        Task::none()
                    } else {
                        iced::window::close(window_id)
                    }
                } else if Some(window_id) == self.palette_window_id && self.palette_visible {
                    // Palette lost focus → hide, unless we're mid-chat.
                    // Agent tool calls can steal focus (e.g. opening a
                    // browser tab), and we don't want that to dismiss
                    // the palette out from under the user. Esc and the
                    // hotkey toggle still dismiss normally; both paths
                    // preserve chat state.
                    if self.mode == Mode::Chatting {
                        Task::none()
                    } else {
                        self.hide_palette()
                    }
                } else {
                    Task::none()
                }
            }

            Message::WindowFocused(window_id) => {
                if Some(window_id) == self.palette_window_id && self.palette_visible {
                    text_input::focus(INPUT_ID.clone())
                } else {
                    Task::none()
                }
            }

            Message::WindowClosed(window_id) => {
                if Some(window_id) == self.settings_window_id {
                    self.settings_window_id = None;
                    self.settings_opened_at = None;
                } else if Some(window_id) == self.palette_window_id {
                    self.palette_window_id = None;
                }
                Task::none()
            }

            Message::DragWindow => {
                if let Some(id) = self.palette_window_id {
                    iced::window::drag(id)
                } else {
                    Task::none()
                }
            }

            Message::WindowMoved {
                window_id,
                position,
            } => {
                // Only the palette is user-draggable; the settings window
                // is positioned relative to the tray icon on open and
                // doesn't participate in the restore-last-position dance.
                if Some(window_id) == self.palette_window_id {
                    if self.programmatic_move_pending {
                        // Our own `move_to` in `show_palette` — eat the
                        // event without updating `dragged_position` so
                        // the next cursor-center open doesn't look like
                        // a user drag.
                        self.programmatic_move_pending = false;
                    } else {
                        self.dragged_position = Some(position);
                        // If the user drags while pinned, the pin follows:
                        // otherwise they'd have to unpin-drag-repin to
                        // relocate a pinned palette, which is annoying.
                        if let Some(pin) = &mut self.pinned {
                            pin.position = position;
                        }
                    }
                }
                Task::none()
            }

            Message::TogglePin => {
                if self.pinned.is_some() {
                    // Unpin → full reset to the default cursor-centered
                    // behavior. Clear both stored positions and actively
                    // move the window back now so the user sees the
                    // effect immediately (not on next hide/show). The
                    // chat portion of the pin (if any) is released at
                    // the same time — unpin is atomic.
                    self.pinned = None;
                    self.dragged_position = None;
                    #[cfg(target_os = "macos")]
                    if let Some(id) = self.palette_window_id {
                        if let Some((x, y)) = crate::platform::macos::cursor_palette_position(
                            Self::LAUNCHER_W as f64,
                        ) {
                            self.programmatic_move_pending = true;
                            return iced::window::move_to(
                                id,
                                iced::Point::new(x as f32, y as f32),
                            );
                        }
                    }
                } else {
                    // Pin → capture chat (if viewing one) and position.
                    let chat_id = if self.mode == Mode::Chatting {
                        self.active_chat_id
                    } else {
                        None
                    };
                    if let Some(position) = self.dragged_position {
                        self.pinned = Some(Pin { position, chat_id });
                    } else if let Some(id) = self.palette_window_id {
                        // User hasn't dragged yet — pin wherever the palette
                        // currently sits on screen. We don't track position
                        // continuously (the `programmatic_move_pending` flag
                        // swallows the initial cursor-center `Moved` event
                        // so it isn't mistaken for a drag), so read it now
                        // via `get_position`. The resolved Task dispatches
                        // `CommitPin` which reads the chat id fresh at
                        // commit time and finalises the pin.
                        return iced::window::get_position(id).map(Message::CommitPin);
                    }
                }
                Task::none()
            }

            Message::CommitPin(point) => {
                if let Some(position) = point {
                    // Only commit if the user hasn't managed to drag or
                    // toggle in the tiny window between kicking off the
                    // async read and this handler firing.
                    if self.pinned.is_none() && self.dragged_position.is_none() {
                        let chat_id = if self.mode == Mode::Chatting {
                            self.active_chat_id
                        } else {
                            None
                        };
                        self.pinned = Some(Pin { position, chat_id });
                    }
                }
                Task::none()
            }

            Message::ApiKeyInputChanged(v) => {
                self.api_key_input = v;
                // Save-on-change: persist whatever is currently in the
                // field. Empty clears the saved key; non-empty stores
                // it verbatim. Any sk-ant- prefix validation happens
                // in the sidecar at run-time — the settings panel
                // shouldn't silently drop input that doesn't match.
                //
                // Storage is the OS keychain (see `secrets` module),
                // not settings.json — the key is never written to
                // disk in plaintext.
                let result = if self.api_key_input.is_empty() {
                    crate::secrets::delete_api_key()
                } else {
                    crate::secrets::set_api_key(&self.api_key_input)
                };
                if let Err(e) = result {
                    eprintln!("[slashpad] failed to persist api key: {e}");
                }
                Task::none()
            }

            Message::ToggleApiKeyVisibility => {
                self.api_key_visible = !self.api_key_visible;
                Task::none()
            }

            Message::ClearApiKey => {
                self.api_key_input.clear();
                if let Err(e) = crate::secrets::delete_api_key() {
                    eprintln!("[slashpad] failed to clear api key: {e}");
                }
                self.api_key_visible = false;
                Task::none()
            }

            Message::UseSubscriptionToggled(enabled) => {
                self.settings.use_subscription = enabled;
                let _ = self.settings.save();
                // Hide the key again whenever we flip modes so toggling
                // off → on → off doesn't leak plaintext.
                if enabled {
                    self.api_key_visible = false;
                }
                Task::none()
            }

            Message::LoadUserSettingsToggled(enabled) => {
                self.settings.load_user_settings = enabled;
                if let Err(e) = self.settings.save() {
                    eprintln!("[slashpad] failed to save loadUserSettings: {e}");
                }
                // Reload the palette's skill list so the new scope
                // takes effect immediately without restarting the app.
                // New chat sidecars pick up the flag on spawn via the
                // payload; already-running chats keep their original
                // scope until restarted, which is fine.
                self.all_skills =
                    skills::load_skills(enabled).unwrap_or_default();
                self.filtered_skills = if self.mode == Mode::Skills {
                    if let Some(query) = self.input.strip_prefix('/') {
                        if query.is_empty() {
                            self.all_skills.clone()
                        } else {
                            crate::fuzzy::filter_skills(&self.all_skills, query)
                        }
                    } else {
                        self.all_skills.clone()
                    }
                } else {
                    self.all_skills.clone()
                };
                self.selected_skill_index = 0;
                Task::none()
            }

            Message::TrayOpenSettings {
                tray_x,
                tray_y,
                tray_w,
                tray_h,
            } => self.open_settings_window(tray_x, tray_y, tray_w, tray_h),

            Message::QuitRequested => {
                // Drop every chat entry — each holds an owned
                // `SpawnedSidecar` whose `Child` has `kill_on_drop:
                // true`, so clearing the map kills all runner.mjs
                // children. Bypasses iced's graceful shutdown because
                // that path is fiddly with our NSPanel wrapping.
                self.chats.clear();

                // Unload the launchctl service so `brew services start`
                // works again without hitting "Bootstrap failed: 5".
                // Only runs when managed by brew services (plist exists).
                if let Some(home) = std::env::var_os("HOME") {
                    let plist = std::path::Path::new(&home)
                        .join("Library/LaunchAgents/homebrew.mxcl.slashpad.plist");
                    if plist.exists() {
                        let uid = std::process::Command::new("id")
                            .arg("-u")
                            .output()
                            .ok()
                            .and_then(|o| String::from_utf8(o.stdout).ok())
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        if !uid.is_empty() {
                            let _ = std::process::Command::new("/bin/launchctl")
                                .args(["bootout", &format!("gui/{uid}"), &plist.to_string_lossy()])
                                .status();
                        }
                    }
                }

                std::process::exit(0)
            }

            Message::StartRecordHotkey => {
                self.recording_hotkey = true;
                self.hotkey_error = None;
                Task::none()
            }

            Message::CancelRecordHotkey => {
                self.recording_hotkey = false;
                self.hotkey_error = None;
                Task::none()
            }

            Message::HotkeyCaptured(chord) => {
                match hotkey::update_hotkey(&chord) {
                    Ok(()) => {
                        self.settings.hotkey = chord;
                        let _ = self.settings.save();
                        self.hotkey_error = None;
                    }
                    Err(e) => {
                        self.hotkey_error = Some(e.to_string());
                    }
                }
                self.recording_hotkey = false;
                Task::none()
            }

            Message::SpinnerTick => {
                self.spinner_frame = self.spinner_frame.wrapping_add(1);
                Task::none()
            }

            Message::ToggleToolsExpanded(msg_id) => {
                if let Some(chat) = self
                    .active_chat_id
                    .and_then(|cid| self.chats.get_mut(&cid))
                {
                    if let Some(msg) = chat.state.messages.iter_mut().find(|m| m.id == msg_id) {
                        msg.tools_expanded = !msg.tools_expanded;
                    }
                }
                Task::none()
            }

            Message::MarkdownLinkClicked(url) => {
                // Hand off to the macOS `open` command, which routes
                // http(s) URLs to the user's default browser. Spawning
                // (not waiting) so the palette stays responsive, and
                // best-effort — if the spawn fails for some reason we
                // just log it.
                if let Err(e) = std::process::Command::new("open")
                    .arg(url.as_str())
                    .spawn()
                {
                    eprintln!("[slashpad] failed to open link {url}: {e}");
                }
                Task::none()
            }

            Message::OpenSessionInTerminal => {
                // `ShortcutFilter` drops the Cmd+T event before
                // text_input can insert the 't' — no leak to clean up.
                //
                // Only meaningful from the chat view. Fire in Idle /
                // Skills / Settings is a silent no-op so the shortcut
                // feels dead outside its scope rather than doing
                // something surprising.
                if self.mode != Mode::Chatting {
                    return Task::none();
                }
                let session_id = match self
                    .active_chat()
                    .and_then(|e| e.state.session_id.clone())
                {
                    Some(id) => id,
                    None => {
                        // First turn hasn't completed yet — the
                        // sidecar assigns a session id with its first
                        // response. Keyhint is suppressed in this
                        // case too, so this branch is mostly defense
                        // against a racey keypress.
                        eprintln!(
                            "[slashpad] Cmd+T ignored: active chat has no session_id yet"
                        );
                        return Task::none();
                    }
                };
                let cwd = self.project_path.to_string_lossy().into_owned();
                if let Err(e) = crate::terminal::open_claude_resume(
                    self.settings.preferred_terminal,
                    &cwd,
                    &session_id,
                ) {
                    eprintln!(
                        "[slashpad] failed to open session {session_id} in {:?}: {e}",
                        self.settings.preferred_terminal
                    );
                    return Task::none();
                }
                self.hide_palette()
            }

            Message::PreferredTerminalChanged(term) => {
                self.settings.preferred_terminal = term;
                if let Err(e) = self.settings.save() {
                    eprintln!("[slashpad] failed to save preferred terminal: {e}");
                }
                Task::none()
            }

            Message::ChatScrolled(viewport) => {
                // Record the new viewport geometry on the active chat and
                // recompute `autoscroll`: on when the user is sitting at
                // (or below — rounding) the bottom, off otherwise. The
                // zero-overflow case (content shorter than viewport) also
                // counts as "at the bottom" so autoscroll stays engaged
                // for short chats.
                if let Some(chat_id) = self.active_chat_id {
                    if let Some(entry) = self.chats.get_mut(&chat_id) {
                        let offset = viewport.absolute_offset();
                        let viewport_h = viewport.bounds().height;
                        let content_h = viewport.content_bounds().height;
                        let max_y = (content_h - viewport_h).max(0.0);
                        // 2px slack so a near-bottom rounded offset still
                        // counts as "at bottom".
                        let at_bottom = max_y - offset.y <= 2.0;
                        entry.scroll.offset = offset;
                        entry.scroll.viewport_h = viewport_h;
                        entry.scroll.content_h = content_h;
                        entry.scroll.autoscroll = at_bottom;
                    }
                }
                Task::none()
            }
        }
    }

    /// Scroll the active chat panel by `delta` pixels (positive = down,
    /// negative = up), clamped to the scrollable range. No-op when there
    /// is no active chat, or when the content fits within the viewport.
    /// Returns a `scroll_to` task; the resulting `on_scroll` callback
    /// updates `autoscroll` for us.
    fn chat_scroll_by(&mut self, delta: f32) -> Task<Message> {
        let Some(chat_id) = self.active_chat_id else {
            return Task::none();
        };
        let Some(entry) = self.chats.get_mut(&chat_id) else {
            return Task::none();
        };
        let max_y = entry.scroll.max_y();
        if max_y <= 0.0 {
            return Task::none();
        }
        let new_y = (entry.scroll.offset.y + delta).clamp(0.0, max_y);
        entry.scroll.offset.y = new_y;
        scrollable::scroll_to(
            CHAT_SCROLL_ID.clone(),
            scrollable::AbsoluteOffset { x: 0.0, y: new_y },
        )
    }

    pub fn view(&self, window_id: iced::window::Id) -> Element<'_, Message> {
        // Settings window: always shows the settings panel.
        if Some(window_id) == self.settings_window_id {
            return container(ui::settings::view(
                &self.api_key_input,
                self.api_key_visible,
                self.settings.use_subscription,
                &self.settings.hotkey,
                self.recording_hotkey,
                self.hotkey_error.as_deref(),
                self.settings.preferred_terminal,
                self.settings.load_user_settings,
                &self.update_status,
            ))
            .padding(8)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into();
        }

        // Palette window: command input + mode-dependent content below.
        // `is_agent_ready` in the placeholder/hint text now reflects
        // the *active* chat when there is one — a background chat
        // being busy shouldn't gray out the input for a different
        // chat you're viewing.
        let is_agent_ready = self
            .active_chat()
            .map(|e| matches!(e.state.status, ChatStatus::Idle))
            .unwrap_or(true);
        let input = ui::command_input::view(&self.input, self.mode, is_agent_ready);
        // Invisible 8px drag handle at the very top of the card. Clicks
        // here fire `Message::DragWindow` which starts a native OS window
        // drag (iced::window::drag → winit → performWindowDragWithEvent:).
        // Not mode-gated: "drag the window by its top edge" works in every
        // mode uniformly. The strip sits above `input` so text-input focus
        // clicks stay untouched — the input has its own hit rect below.
        let drag_strip: Element<'_, Message> = mouse_area(
            Space::new(iced::Length::Fill, iced::Length::Fixed(8.0)),
        )
        .on_press(Message::DragWindow)
        .into();
        // `Column` defaults to `Length::Shrink` for height — but we rely on
        // the mode-specific middle elements (lists / chat panel / spacer)
        // being `Length::Fill` to anchor the keyhints bar to the bottom of
        // the fixed-height window. `Length::Fill` inside a `Shrink` parent
        // collapses to 0, so we have to make the whole vertical chain
        // (container → card → stack) `Length::Fill` end-to-end.
        let mut stack: Column<'_, Message> = column![drag_strip, input]
            .spacing(0)
            .height(iced::Length::Fill);

        // Track whether the mode pushed a `Length::Fill` middle element
        // (list or chat panel). If it didn't, we push a `vertical_space`
        // filler before the keyhints so the bar stays pinned to the
        // bottom of the fixed-height window instead of floating up.
        let mut has_fill_middle = false;

        match self.mode {
            Mode::Skills => {
                // Once the input has locked onto a concrete skill
                // (`/<name>` or `/<name> ...`), hide the filter panel.
                // Continuing to render it would either show a stale
                // list or — worse — "No matching skills" as soon as
                // the user starts typing a natural-language argument.
                if self.locked_skill().is_none() {
                    stack = stack.push(ui::theme::divider());
                    stack = stack.push(ui::skill_list::view(
                        &self.filtered_skills,
                        self.selected_skill_index,
                        SKILL_LIST_SCROLL_ID.clone(),
                    ));
                    has_fill_middle = true;
                }
            }
            Mode::ProjectPicker => {
                stack = stack.push(ui::theme::divider());
                stack = stack.push(ui::project_picker::view(
                    &self.filtered_projects,
                    self.selected_project_index,
                    PROJECT_PICKER_SCROLL_ID.clone(),
                ));
                has_fill_middle = true;
            }
            Mode::Idle if !self.input.starts_with('/') && self.idle_row_count() > 0 => {
                // Build view-layer rows that borrow from `self`. Both
                // active chats and past sessions are passed through the
                // live fuzzy filter (idle_filter_query); past sessions
                // also exclude any session_id already represented by an
                // active chat.
                let visible_chat_ids = self.visible_active_chat_ids();
                let visible_past = self.visible_past_session_rows();
                let mut rows: Vec<ui::idle_list::IdleRow<'_>> =
                    Vec::with_capacity(visible_chat_ids.len() + visible_past.len());
                for id in &visible_chat_ids {
                    if let Some(entry) = self.chats.get(id) {
                        rows.push(ui::idle_list::IdleRow::Active(entry));
                    }
                }
                for session in visible_past {
                    rows.push(ui::idle_list::IdleRow::Past(session));
                }
                let selected = if self.idle_selection_active {
                    self.selected_idle_index
                } else {
                    usize::MAX
                };
                stack = stack.push(ui::theme::divider());
                stack = stack.push(ui::idle_list::view(
                    rows,
                    selected,
                    self.spinner_frame,
                    IDLE_LIST_SCROLL_ID.clone(),
                ));
                has_fill_middle = true;
            }
            Mode::Chatting => {
                if let Some(entry) = self.active_chat() {
                    let is_generating = !matches!(
                        entry.state.status,
                        ChatStatus::Idle | ChatStatus::Closed | ChatStatus::Error
                    );
                    stack = stack.push(ui::theme::divider());
                    stack = stack.push(ui::chat_panel::view(
                        &entry.state.messages,
                        is_generating,
                        entry.state.turn_submitted_at,
                        self.spinner_frame,
                        CHAT_SCROLL_ID.clone(),
                    ));
                    has_fill_middle = true;
                }
            }
            _ => {}
        }

        if !has_fill_middle {
            // No middle panel in this mode (empty Idle, locked Skills,
            // Chatting with no active chat, Settings fallback). Push a
            // flexible spacer so the keyhints bar stays anchored to the
            // bottom edge of the fixed-size window.
            stack = stack.push(iced::widget::vertical_space().height(iced::Length::Fill));
        }

        stack = stack.push(ui::theme::divider());
        stack = stack.push(ui::keyhints::view(
            self.mode,
            ui::keyhints::KeyhintContext {
                has_rows: self.idle_row_count() > 0,
                selection_active: self.idle_selection_active,
                has_session_id: self
                    .active_chat()
                    .and_then(|e| e.state.session_id.as_deref())
                    .is_some(),
                is_generating: self
                    .active_chat()
                    .map(|e| matches!(
                        e.state.status,
                        ChatStatus::Initializing | ChatStatus::Streaming
                    ))
                    .unwrap_or(false),
                skill_locked: self.locked_skill().is_some(),
                project_path_display: display_project_path(&self.project_path),
                pinned: self.pinned.is_some(),
            },
        ));

        // Unified rounded card: one SURFACE_1 surface + one SURFACE_3
        // border around the whole stack. Individual sections (input,
        // middle panel, keyhints) render with no border/background —
        // `ui::theme::divider()` rows draw the thin section lines.
        //
        // Explicit `height(Fill)` matters: without it the container
        // defaults to `Shrink`, which collapses every `Length::Fill`
        // child inside the stack (lists, chat panel, keyhints-anchor
        // spacer) to 0px. The rendered card then shrinks to just
        // `input + keyhints`, leaving a tiny palette floating inside a
        // 500px transparent window — which is the "wrong height" the
        // user sees on first summon.
        let card = container(stack)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .style(|_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(ui::theme::SURFACE_1)),
                border: iced::Border {
                    color: ui::theme::SURFACE_3,
                    width: 1.0,
                    radius: 14.0.into(),
                },
                text_color: Some(ui::theme::TEXT),
                ..Default::default()
            })
            .clip(true);

        container(card)
            .padding(8)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    // --- helpers ---

    /// Returns the skill whose name the current input has already
    /// "committed" to — i.e. `/<name>` exactly, or `/<name> ...`. Used
    /// to drive the two-step skill-submission UX: when this returns
    /// `Some`, Enter runs the skill and the skill-list panel is hidden
    /// (so natural-language arguments don't trigger a "No matching
    /// skills" message from the fuzzy list).
    fn locked_skill(&self) -> Option<&Skill> {
        let rest = self.input.strip_prefix('/')?;
        self.all_skills.iter().find(|s| {
            rest == s.name || rest.starts_with(&format!("{} ", s.name))
        })
    }

    fn handle_submit(&mut self) -> Task<Message> {
        match self.mode {
            Mode::Chatting if !self.input.trim().is_empty() => {
                let content = self.input.trim().to_string();
                let status = self.active_chat().map(|e| e.state.status);

                match status {
                    Some(ChatStatus::Idle) => {
                        // Agent is ready — normal follow-up path.
                        self.send_follow_up(content)
                    }
                    Some(ChatStatus::Initializing | ChatStatus::Streaming) => {
                        // Agent is busy — interrupt then follow up.
                        let chat_id = self.active_chat_id.unwrap();
                        let has_session_id = self
                            .chats
                            .get(&chat_id)
                            .and_then(|e| e.state.session_id.clone())
                            .is_some();

                        // Kill the running sidecar and seal partial output.
                        if let Some(entry) = self.chats.get_mut(&chat_id) {
                            entry.sidecar = None;
                            entry.state.mark_cancelled();
                        }

                        if has_session_id {
                            // Session exists — send_follow_up will respawn
                            // with resume, continuing the conversation.
                            self.send_follow_up(content)
                        } else {
                            // No session_id yet (interrupted during init) —
                            // can't resume, so start a brand new chat.
                            self.start_chat(content)
                        }
                    }
                    _ => Task::none(),
                }
            }
            Mode::Skills => {
                // Two-step submission: if the input has already
                // committed to a skill (`/<name>` or `/<name> ...`),
                // Enter runs it. Otherwise Enter autocompletes the
                // currently-selected filter match into the input.
                if self.locked_skill().is_some() {
                    self.start_chat(self.input.trim().to_string())
                } else if let Some(skill) =
                    self.filtered_skills.get(self.selected_skill_index).cloned()
                {
                    self.input = format!("/{} ", skill.name);
                    // Keep `filtered_skills` consistent with the new
                    // input value, mirroring the InputChanged handler.
                    let query = format!("{} ", skill.name);
                    self.filtered_skills =
                        crate::fuzzy::filter_skills(&self.all_skills, &query);
                    self.selected_skill_index = 0;
                    text_input::focus(INPUT_ID.clone())
                } else {
                    Task::none()
                }
            }
            Mode::ProjectPicker if !self.filtered_projects.is_empty() => {
                let Some(picked) = self
                    .filtered_projects
                    .get(self.selected_project_index)
                    .cloned()
                else {
                    return Task::none();
                };
                // Commit the new project: update the in-memory cwd,
                // persist it to settings.json so it survives restarts,
                // restore whatever the user had typed before Cmd+P,
                // and drop back to the Idle list.
                self.project_path = picked.path.clone();
                self.settings.selected_project_path =
                    Some(picked.path.to_string_lossy().into_owned());
                if let Err(e) = self.settings.save() {
                    eprintln!("[slashpad] failed to persist selected project: {e}");
                }
                self.mode = Mode::Idle;
                // Always land on a clean Idle view for the new
                // project: no residual query (it was scoped to the
                // old project), no pre-selected row (so a reflex
                // Enter doesn't resume the top session by accident),
                // and no lingering `active_chat_id` pointing at a
                // chat that belongs to the previous cwd.
                self.input.clear();
                self.input_before_picker = None;
                self.active_chat_id = None;
                self.filtered_projects.clear();
                self.selected_project_index = 0;
                self.selected_idle_index = 0;
                self.idle_selection_active = false;
                // Past-sessions list is scoped per-cwd, so a project
                // switch needs a re-fetch.
                self.recent_sessions.clear();
                self.refresh_sessions()
            }
            // In picker mode with no matches, Enter is a silent no-op
            // — don't fall through to "start chat with typed text".
            Mode::ProjectPicker => Task::none(),
            Mode::Idle if self.idle_selection_active && self.idle_row_count() > 0 => {
                // Dispatch based on which row is selected in the
                // unified (active chats + past sessions) list. The
                // typed input (if any) is discarded — the user opted
                // into a resume by arrow-keying onto a row. To start
                // a new chat with the typed text, press Enter without
                // arrow-keying first.
                let rows = self.build_idle_rows();
                match rows.get(self.selected_idle_index) {
                    Some(IdleRowSelection::Active(chat_id)) => {
                        let cid = *chat_id;
                        // Enter the chat view without spawning anything.
                        if let Some(entry) = self.chats.get_mut(&cid) {
                            entry.scroll.autoscroll = true;
                            self.active_chat_id = Some(cid);
                            self.mode = Mode::Chatting;
                            self.input.clear();
                            Task::batch([
                                text_input::focus(INPUT_ID.clone()),
                                snap_chat_to_bottom(),
                            ])
                        } else {
                            Task::none()
                        }
                    }
                    Some(IdleRowSelection::Past(session)) => {
                        let s = session.clone();
                        self.resume_session(s)
                    }
                    None => Task::none(),
                }
            }
            _ if !self.input.trim().is_empty() && !self.input.starts_with('/') => {
                let prompt = self.input.trim().to_string();
                self.start_chat(prompt)
            }
            _ => Task::none(),
        }
    }

    fn start_chat(&mut self, prompt: String) -> Task<Message> {
        // Fresh chat → forget any position the user dragged the palette
        // to during a previous chat. The next `show_palette()` will
        // cursor-center again, matching the "new task, fresh location"
        // UX the user confirmed.
        self.dragged_position = None;
        let chat_id = self.alloc_chat_id();
        let spawned = match self.spawn_sidecar_chat(chat_id, prompt.clone(), None) {
            Ok(s) => s,
            Err(e) => {
                // Build an error-state entry so the user sees what went
                // wrong in the chat panel (and the chat still shows in
                // the idle list next time they summon the palette).
                let mut state = ChatState::new(chat_id, &prompt);
                state.push_error(format!("Failed to start agent: {e}"));
                state.status = ChatStatus::Error;
                self.chats.insert(
                    chat_id,
                    ChatEntry {
                        state,
                        sidecar: None,
                        scroll: ChatScrollState::new(),
                    },
                );
                self.active_chat_id = Some(chat_id);
                self.mode = Mode::Chatting;
                self.input.clear();
                return Task::batch([
                    text_input::focus(INPUT_ID.clone()),
                    snap_chat_to_bottom(),
                ]);
            }
        };

        let state = ChatState::new(chat_id, &prompt);
        self.chats.insert(
            chat_id,
            ChatEntry {
                state,
                sidecar: Some(spawned),
                scroll: ChatScrollState::new(),
            },
        );
        self.active_chat_id = Some(chat_id);
        self.mode = Mode::Chatting;
        self.input.clear();

        Task::batch([
            text_input::focus(INPUT_ID.clone()),
            snap_chat_to_bottom(),
        ])
    }

    /// Spawn a sidecar chat without switching to the Chatting view.
    /// Used by the Cmd+Enter "fire & forget" flow: the chat streams in
    /// the background and the palette is dismissed immediately.
    fn start_chat_detached(&mut self, prompt: String) -> Task<Message> {
        // Same reset as `start_chat` — Cmd+Enter "fire and forget" is
        // still a new chat, so the next palette summon should cursor-
        // follow rather than snap back to the previous drag position.
        self.dragged_position = None;
        let chat_id = self.alloc_chat_id();
        match self.spawn_sidecar_chat(chat_id, prompt.clone(), None) {
            Ok(spawned) => {
                let state = ChatState::new(chat_id, &prompt);
                self.chats.insert(
                    chat_id,
                    ChatEntry {
                        state,
                        sidecar: Some(spawned),
                        scroll: ChatScrollState::new(),
                    },
                );
            }
            Err(e) => {
                let mut state = ChatState::new(chat_id, &prompt);
                state.push_error(format!("Failed to start agent: {e}"));
                state.status = ChatStatus::Error;
                self.chats.insert(
                    chat_id,
                    ChatEntry {
                        state,
                        sidecar: None,
                        scroll: ChatScrollState::new(),
                    },
                );
            }
        }
        self.input.clear();
        self.hide_palette()
    }

    fn resume_session(&mut self, info: SessionInfo) -> Task<Message> {
        // If we already have an active chat tracking this session id,
        // just switch to it instead of creating a duplicate entry.
        if let Some((&existing, _)) = self
            .chats
            .iter()
            .find(|(_, e)| e.state.session_id.as_deref() == Some(info.session_id.as_str()))
        {
            if let Some(entry) = self.chats.get_mut(&existing) {
                entry.scroll.autoscroll = true;
            }
            self.active_chat_id = Some(existing);
            self.mode = Mode::Chatting;
            self.input.clear();
            return Task::batch([
                text_input::focus(INPUT_ID.clone()),
                snap_chat_to_bottom(),
            ]);
        }

        let chat_id = self.alloc_chat_id();
        let title = info
            .first_prompt
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| info.summary.clone());
        let state = ChatState::resumed(chat_id, info.session_id.clone(), title);
        self.chats.insert(
            chat_id,
            ChatEntry {
                state,
                sidecar: None,
                scroll: ChatScrollState::new(),
            },
        );
        self.active_chat_id = Some(chat_id);
        self.mode = Mode::Chatting;
        self.input.clear();

        // Load session history in the background via a one-shot
        // "messages" sidecar. Scoped to the current project's cwd so
        // the runner finds the session's JSONL under
        // `~/.claude/projects/<mangled-cwd>/`.
        let session_id = info.session_id.clone();
        let cwd = self.project_path.clone();
        let tx = external_sender();
        tokio::spawn(async move {
            let msgs = crate::sessions::load_messages(&cwd, &session_id)
                .await
                .unwrap_or_default();
            let _ = tx.send(External::HistoryLoaded {
                chat_id,
                messages: msgs,
            });
        });

        // The snap here covers the fast-path where history may already
        // be cached / very short; the `HistoryLoaded` handler will emit
        // another snap once the background load completes.
        Task::batch([
            text_input::focus(INPUT_ID.clone()),
            snap_chat_to_bottom(),
        ])
    }

    fn send_follow_up(&mut self, content: String) -> Task<Message> {
        let Some(chat_id) = self.active_chat_id else {
            return Task::none();
        };
        // Take the existing follow-up tx (if any) out of the entry
        // *before* we need to mutate `state`, to avoid an immutable
        // borrow of the entry overlapping with `entry.state` writes.
        let follow_up_tx = self
            .chats
            .get(&chat_id)
            .and_then(|e| e.sidecar.as_ref().map(|s| s.follow_up_tx.clone()));
        let needs_respawn_resume_id = self.chats.get(&chat_id).and_then(|e| {
            if e.sidecar.is_none() {
                e.state.session_id.clone()
            } else {
                None
            }
        });

        // Push the user bubble + flip status to Streaming before we
        // hand off to the sidecar. Submitting a follow-up implicitly
        // re-engages autoscroll — the user wants to see their own
        // message (and the reply) land at the bottom.
        if let Some(entry) = self.chats.get_mut(&chat_id) {
            let user_id = entry.state.alloc_msg_id();
            entry
                .state
                .messages
                .push(ChatMessageView::user(user_id, content.clone()));
            entry.state.current_assistant_id = None;
            entry.state.status = ChatStatus::Streaming;
            entry.scroll.autoscroll = true;
        }
        self.input.clear();

        if let Some(tx) = follow_up_tx {
            let _ = tx.send(FollowUp::Message(content));
        } else if let Some(resume_id) = needs_respawn_resume_id {
            // Resumed-from-disk first follow-up: spawn a fresh sidecar
            // with the resume id and write it back into the *same*
            // chat entry (don't allocate a new chat_id — that would
            // create a ghost duplicate).
            match self.spawn_sidecar_chat(chat_id, content, Some(resume_id)) {
                Ok(spawned) => {
                    if let Some(entry) = self.chats.get_mut(&chat_id) {
                        entry.sidecar = Some(spawned);
                    }
                }
                Err(e) => {
                    if let Some(entry) = self.chats.get_mut(&chat_id) {
                        entry.state.push_error(format!("Failed to restart agent: {e}"));
                        entry.state.status = ChatStatus::Error;
                    }
                }
            }
        }
        Task::batch([snap_chat_to_bottom(), text_input::focus(INPUT_ID.clone())])
    }

    /// Spawn a sidecar for `chat_id` and return the handle. Sets up a
    /// per-chat forwarder task that tags each event with `chat_id` as
    /// it flows into the global `EXTERNAL_TX` bus, and emits a
    /// `SidecarClosed` sentinel when the sidecar's stdout closes.
    fn spawn_sidecar_chat(
        &self,
        chat_id: ChatId,
        prompt: String,
        resume: Option<String>,
    ) -> anyhow::Result<SpawnedSidecar> {
        // Subscription mode: don't forward a key, let the Agent SDK
        // fall back to the user's `claude login` session. API-key
        // mode: forward whatever is saved in the OS keychain.
        let api_key = if self.settings.use_subscription {
            None
        } else {
            crate::secrets::get_api_key()
        };
        let payload = Payload::chat(
            prompt,
            self.project_path.to_string_lossy().to_string(),
            api_key,
            resume,
            self.settings.load_user_settings,
        );
        let mut spawned = sidecar::spawn(payload)?;

        // Forward sidecar events into the external bus, tagged with
        // the chat id so the main loop can route them to the correct
        // entry. When the reader task on the sidecar side finishes
        // (stdout closed → runner.mjs exited), the receiver returns
        // `None`, the loop exits, and we emit a `SidecarClosed`
        // sentinel so the chat's status can flip to `Closed`.
        let tx = external_sender();
        let mut rx = std::mem::replace(&mut spawned.event_rx, mpsc::unbounded_channel().1);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if tx.send(External::Sidecar { chat_id, event }).is_err() {
                    return;
                }
            }
            let _ = tx.send(External::SidecarClosed { chat_id });
        });

        Ok(spawned)
    }

    fn process_sidecar_event(&mut self, chat_id: ChatId, event: SidecarEvent) -> Task<Message> {
        let Some(entry) = self.chats.get_mut(&chat_id) else {
            return Task::none();
        };
        // `TextDelta`, `ToolStart`, `ToolEnd`, and `Error` all append to
        // `messages` / `blocks`. `SessionId`, `Ready`, and `Complete`
        // don't add visible content so they don't need an autoscroll.
        let grows_content = matches!(
            event,
            SidecarEvent::TextDelta { .. }
                | SidecarEvent::ToolStart { .. }
                | SidecarEvent::ToolEnd { .. }
                | SidecarEvent::Error { .. }
        );
        entry.state.apply_event(event);
        let autoscroll = entry.scroll.autoscroll;

        let mut tasks: Vec<Task<Message>> = Vec::new();
        // Autoscroll: pin the chat view to the bottom whenever the
        // active chat just grew and the user hasn't scrolled away.
        if grows_content
            && autoscroll
            && self.mode == Mode::Chatting
            && self.active_chat_id == Some(chat_id)
        {
            tasks.push(snap_chat_to_bottom());
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn process_sidecar_closed(&mut self, chat_id: ChatId) -> Task<Message> {
        let Some(entry) = self.chats.get_mut(&chat_id) else {
            return Task::none();
        };
        // If a new sidecar has already been spawned (e.g. after an
        // interrupt-and-follow-up), this event is from the old
        // forwarder — ignore it so we don't stomp the live sidecar.
        if entry.sidecar.is_some() {
            return Task::none();
        }
        entry.sidecar = None;
        if !matches!(entry.state.status, ChatStatus::Idle | ChatStatus::Error) {
            entry.state.status = ChatStatus::Closed;
        }
        if self.palette_visible
            && self.mode == Mode::Chatting
            && self.active_chat_id == Some(chat_id)
        {
            text_input::focus(INPUT_ID.clone())
        } else {
            Task::none()
        }
    }

    fn alloc_chat_id(&mut self) -> ChatId {
        let id = self.next_chat_id;
        self.next_chat_id += 1;
        id
    }

    pub fn active_chat(&self) -> Option<&ChatEntry> {
        self.active_chat_id.and_then(|id| self.chats.get(&id))
    }

    /// How many rows the idle list currently contains — active chats
    /// first, then past sessions that aren't dupes of an active chat's
    /// session_id. Respects the live fuzzy filter on `self.input`. Used
    /// by keyboard nav bounds.
    fn idle_row_count(&self) -> usize {
        self.visible_active_chat_ids().len() + self.visible_past_session_rows().len()
    }

    /// Past sessions filtered to exclude any already represented as an
    /// active chat (by `session_id` match). Returned as owned clones
    /// because borrow-lifetime rules for `&self` + later mutations
    /// through `handle_submit` etc. are simpler with owned rows.
    fn past_session_rows(&self) -> Vec<SessionInfo> {
        let active_ids: std::collections::HashSet<&str> = self
            .chats
            .values()
            .filter_map(|c| c.state.session_id.as_deref())
            .collect();
        self.recent_sessions
            .iter()
            .filter(|s| !active_ids.contains(s.session_id.as_str()))
            .cloned()
            .collect()
    }

    /// The current fuzzy-filter query for the idle list. Empty string
    /// when the input is empty, `/`-prefixed (skills mode), or we're
    /// not in Idle. Callers pass this into `filter_sessions` and the
    /// chat-title filter so filtering is a no-op outside Idle.
    fn idle_filter_query(&self) -> &str {
        if self.mode == Mode::Idle && !self.input.starts_with('/') {
            self.input.trim()
        } else {
            ""
        }
    }

    /// Past sessions visible in the idle list, after the fuzzy filter.
    /// Falls back to `past_session_rows()` when the filter query is
    /// empty.
    fn visible_past_session_rows(&self) -> Vec<SessionInfo> {
        let all = self.past_session_rows();
        let q = self.idle_filter_query();
        if q.is_empty() {
            all
        } else {
            crate::fuzzy::filter_sessions(&all, q)
        }
    }

    /// Active chat ids visible in the idle list, after the fuzzy
    /// filter. Ranked by title-match score when the filter is active;
    /// otherwise returned in BTreeMap (id) order.
    fn visible_active_chat_ids(&self) -> Vec<ChatId> {
        let q = self.idle_filter_query();
        if q.is_empty() {
            return self.chats.keys().rev().copied().collect();
        }
        let mut scored: Vec<(u32, ChatId)> = Vec::new();
        for (&id, entry) in self.chats.iter() {
            if let Some(score) = crate::fuzzy::fuzzy_score(&entry.state.title, q) {
                scored.push((score, id));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, id)| id).collect()
    }

    /// Build the unified idle-list selection list. Used by
    /// `handle_submit` to map `selected_idle_index` to either a
    /// ChatId or a past SessionInfo. The analogous `build_idle_rows`
    /// produces the view-layer rows with references.
    fn build_idle_rows(&self) -> Vec<IdleRowSelection> {
        let mut rows: Vec<IdleRowSelection> = Vec::with_capacity(self.idle_row_count());
        for id in self.visible_active_chat_ids() {
            rows.push(IdleRowSelection::Active(id));
        }
        for session in self.visible_past_session_rows() {
            rows.push(IdleRowSelection::Past(session));
        }
        rows
    }

    fn refresh_sessions(&self) -> Task<Message> {
        let tx = external_sender();
        let cwd = self.project_path.clone();
        tokio::spawn(async move {
            let sessions = crate::sessions::list_recent(&cwd).await.unwrap_or_default();
            let _ = tx.send(External::RecentSessions(sessions));
        });
        Task::none()
    }

    /// Palette launcher width. Fixed; settings has its own window.
    pub const LAUNCHER_W: f32 = 720.0;

    /// Palette launcher height. Fixed — picked to sit between the idle-list
    /// max (~390px) and the chat detail max (~610px). Keeping the window
    /// size constant across every mode eliminates the NSPanel resize
    /// flicker that used to fire on keystrokes, mode transitions, and
    /// background chat-status updates.
    pub const LAUNCHER_H: f32 = 500.0;

    /// Debounce window for the spurious `Unfocused` event that AppKit
    /// fires right after we open the settings window (our Accessory
    /// activation policy means the app deactivates the moment it
    /// activates). Blurs within this window are ignored; anything after
    /// is a genuine user-initiated click-outside and closes settings.
    const SETTINGS_BLUR_GRACE_MS: u64 = 300;

    /// One-shot resize to the canonical launcher size. Called only from
    /// `show_palette()` so winit has a deterministic size when the NSPanel
    /// gets wrapped. All other code paths that used to resize on content
    /// change have been removed — the window is intentionally fixed-size
    /// now to eliminate flicker.
    fn resize_task(&self) -> Task<Message> {
        let Some(id) = self.palette_window_id else {
            return Task::none();
        };
        iced::window::resize(id, iced::Size::new(Self::LAUNCHER_W, Self::LAUNCHER_H))
    }

    fn toggle_palette(&mut self) -> Task<Message> {
        if self.palette_visible {
            self.hide_palette()
        } else {
            self.show_palette()
        }
    }

    fn show_palette(&mut self) -> Task<Message> {
        self.palette_visible = true;

        // Re-scan the skills directory every time the palette is shown so
        // newly created skills appear without restarting the app.
        self.all_skills =
            skills::load_skills(self.settings.load_user_settings).unwrap_or_default();

        // If the user has pinned a chat (Cmd+Shift+P while viewing one)
        // and that chat is still alive, re-enter it directly. Otherwise
        // fall through to the default "open into Skills with `/`
        // prefilled" behavior. A position-only pin goes through the
        // same fall-through — it only affects window placement.
        let pinned_chat_id = self
            .pinned
            .and_then(|p| p.chat_id)
            .filter(|id| self.chats.contains_key(id));

        if let Some(chat_id) = pinned_chat_id {
            self.mode = Mode::Chatting;
            self.active_chat_id = Some(chat_id);
            self.input.clear();
        } else {
            // Defensive: if the pinned chat has since been closed, drop
            // just the chat portion of the pin while keeping the pinned
            // position intact.
            if let Some(pin) = &mut self.pinned {
                if pin.chat_id.is_some() {
                    pin.chat_id = None;
                }
            }
            // Default: open into the skills picker with "/" prefilled.
            // Users reach the unified Idle list (active chats + past
            // sessions) by backspacing the "/" away —
            // `Message::InputChanged` flips the mode to Idle automatically
            // when the leading "/" is removed.
            self.mode = Mode::Skills;
            self.input = "/".to_string();
            self.filtered_skills = self.all_skills.clone();
            self.selected_skill_index = 0;
        }
        // Never clear `self.chats` or `self.active_chat_id` — those
        // carry the state the user expects to come back to.

        let Some(id) = self.palette_window_id else {
            return Task::none();
        };

        // NSPanel style + orderFrontAndMakeKey on the specific palette
        // window, via run_with_handle. This is the multi-window-safe
        // replacement for the old `first_app_window_ptr()` guessing.
        let style_task: Task<Message> = iced::window::run_with_handle(id, |handle| {
            #[cfg(target_os = "macos")]
            unsafe {
                let ns_window = crate::platform::macos::ns_window_from_handle(&handle);
                crate::platform::macos::apply_palette_style(ns_window);
                crate::platform::macos::order_front_and_make_key(ns_window);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = handle;
        })
        .discard();

        let mut tasks: Vec<Task<Message>> = Vec::with_capacity(4);
        tasks.push(style_task);
        // Skip cursor-centering if the user has previously dragged the
        // palette in this session (or pinned it permanently): the
        // NSPanel retains its frame across `orderOut`/`orderFrontRegardless`,
        // so letting it reappear where they put it is the "just show it
        // where I left it" experience. `dragged_position` is cleared by
        // `start_chat` / `start_chat_detached` so each fresh chat cursor-
        // follows; the pin is NOT cleared there — that's the whole
        // point of pinning.
        #[cfg(target_os = "macos")]
        if self.dragged_position.is_none() && self.pinned.is_none() {
            if let Some((x, y)) =
                crate::platform::macos::cursor_palette_position(Self::LAUNCHER_W as f64)
            {
                // Arm the flag so the `Moved` event fired by this
                // programmatic move doesn't get logged as a user drag.
                self.programmatic_move_pending = true;
                tasks.push(iced::window::move_to(
                    id,
                    iced::Point::new(x as f32, y as f32),
                ));
            }
        }
        tasks.push(self.resize_task());
        tasks.push(text_input::focus(INPUT_ID.clone()));
        Task::batch(tasks)
    }

    fn hide_palette(&mut self) -> Task<Message> {
        self.palette_visible = false;
        let Some(id) = self.palette_window_id else {
            return Task::none();
        };
        iced::window::run_with_handle(id, |handle| {
            #[cfg(target_os = "macos")]
            unsafe {
                let ns_window = crate::platform::macos::ns_window_from_handle(&handle);
                crate::platform::macos::order_out(ns_window);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = handle;
        })
        .discard()
    }

    /// Open (or toggle closed) the settings window anchored below the
    /// tray icon. Matches the Tauri `toggle_settings_window` behavior.
    fn open_settings_window(
        &mut self,
        tray_x: f64,
        tray_y: f64,
        tray_w: f64,
        tray_h: f64,
    ) -> Task<Message> {
        // Toggle: if settings is already open, a second tray click
        // dismisses it rather than being a no-op.
        if let Some(id) = self.settings_window_id {
            return iced::window::close(id);
        }

        // Kick off an update check every time settings opens.
        self.update_status = UpdateStatus::Checking;
        let tx = external_sender();
        tokio::spawn(async move {
            let result = crate::updates::check_for_update().await;
            let _ = tx.send(External::UpdateAvailable(result));
        });

        // Each fresh open starts masked and re-synced with whatever is
        // actually stored in the keychain (covers the case where the
        // user saved, closed, and came back — we want the saved key
        // visible as bullets, not whatever stale string is still in
        // the input field).
        self.api_key_visible = false;
        if !self.settings.use_subscription {
            self.api_key_input = crate::secrets::get_api_key().unwrap_or_default();
        }

        let settings_w = 340.0_f64;
        let x = (tray_x + tray_w / 2.0 - settings_w / 2.0) as f32;
        let y = (tray_y + tray_h + 4.0) as f32;

        let (settings_id, open_task) = iced::window::open(ui::theme::settings_window_settings(x, y));
        self.settings_window_id = Some(settings_id);
        // Stamp the open time so the blur handler can ignore the
        // activation-flicker Unfocused that AppKit fires in the next
        // few milliseconds.
        self.settings_opened_at = Some(std::time::Instant::now());

        // No class swap! winit's own `WinitWindow` class already
        // overrides `canBecomeKeyWindow` to return `true`, so the
        // settings window is key-eligible out of the box. Swapping
        // the class to `SlashpadPanel` was crashing iced's close
        // path — its internal cleanup tries to reach methods that
        // live on `WinitWindow`'s class, and the swap hides them.
        //
        // `gain_focus` explicitly calls `makeKeyAndOrderFront` on
        // the new window, which makes it key, which in turn enables
        // the `Unfocused` (blur-close) path when the user clicks
        // outside.
        open_task
            .discard::<Message>()
            .chain(iced::window::gain_focus::<Message>(settings_id))
    }
}

/// Stream that drains the external event bus into iced Messages.
fn external_subscription_stream() -> impl Stream<Item = Message> {
    iced::stream::channel(100, |mut output| async move {
        let mut rx = EXTERNAL_RX
            .lock()
            .unwrap()
            .take()
            .expect("external rx should be available exactly once");
        while let Some(event) = rx.recv().await {
            let msg = match event {
                External::HotkeyPressed => Message::HotkeyPressed,
                External::Sidecar { chat_id, event } => {
                    Message::SidecarEvent { chat_id, event }
                }
                External::SidecarClosed { chat_id } => Message::SidecarClosed(chat_id),
                External::RecentSessions(s) => Message::RecentSessionsLoaded(s),
                External::ProjectsLoaded(p) => Message::ProjectsLoaded(p),
                External::HistoryLoaded { chat_id, messages } => {
                    Message::HistoryLoaded { chat_id, messages }
                }
                External::TrayClicked {
                    tray_x,
                    tray_y,
                    tray_w,
                    tray_h,
                } => Message::TrayOpenSettings {
                    tray_x,
                    tray_y,
                    tray_w,
                    tray_h,
                },
                External::UpdateAvailable(v) => Message::UpdateAvailable(v),
                External::TrayMenuShow => Message::HotkeyPressed,
                External::TrayMenuQuit => Message::QuitRequested,
            };
            if output.send(msg).await.is_err() {
                break;
            }
        }
    })
}

/// Stable iced text_input ID used for focusing the palette input.
pub(crate) static INPUT_ID: std::sync::LazyLock<text_input::Id> =
    std::sync::LazyLock::new(|| text_input::Id::new("slashpad-command-input"));
