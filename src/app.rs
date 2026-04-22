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

use std::collections::{BTreeMap, HashMap};
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

/// Current wall-clock time as unix millis. Used as the pin timestamp
/// on skill/project pin toggles — same ordering semantics as
/// `state::new_pin_tag` (oldest pin sorts first within the pinned
/// block).
fn now_unix_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Resolve the current ⌘K menu target from `self.mode`. The menu is
/// keyboard-navigable across Idle (sessions), Skills, and ProjectPicker
/// modes; row counts and labels differ per target.
fn menu_target_for_mode(mode: Mode) -> Option<ui::session_options_menu::MenuTarget> {
    match mode {
        Mode::Idle => Some(ui::session_options_menu::MenuTarget::Session),
        Mode::Skills => Some(ui::session_options_menu::MenuTarget::Skill),
        Mode::ProjectPicker => Some(ui::session_options_menu::MenuTarget::Project),
        _ => None,
    }
}

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
/// Return the `.app` bundle directory path (e.g. `/Applications/Slashpad.app`).
fn app_bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    // Walk up from .../Slashpad.app/Contents/MacOS/slashpad to .../Slashpad.app
    exe.parent()?.parent()?.parent().map(|p| p.to_path_buf())
}

/// Extract the downloaded .app zip and replace the current bundle.
fn replace_app_bundle(
    zip_path: &std::path::Path,
    bundle_path: Option<&std::path::Path>,
) -> Result<(), String> {
    let bundle = bundle_path.ok_or("cannot determine .app bundle path")?;
    eprintln!("[slashpad] replacing app bundle at {}", bundle.display());

    let tmp_dir = zip_path
        .parent()
        .unwrap_or(std::path::Path::new("/tmp"))
        .join("extracted");
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Use `unzip` to extract — avoids adding a zip crate dependency.
    eprintln!("[slashpad] extracting zip...");
    let status = std::process::Command::new("unzip")
        .args(["-q", "-o"])
        .arg(zip_path)
        .arg("-d")
        .arg(&tmp_dir)
        .status()
        .map_err(|e| format!("failed to run unzip: {e}"))?;
    if !status.success() {
        return Err("unzip failed".to_string());
    }

    // The zip contains Slashpad.app/ at its root.
    let new_app = tmp_dir.join("Slashpad.app");
    if !new_app.exists() {
        return Err("Slashpad.app not found in downloaded zip".to_string());
    }

    // Atomic-ish replacement: rename old to .bak, move new into place.
    let backup = bundle.with_extension("app.bak");
    let _ = std::fs::remove_dir_all(&backup);
    eprintln!("[slashpad] moving old bundle to backup...");
    std::fs::rename(bundle, &backup)
        .map_err(|e| format!("failed to move old bundle to backup: {e}"))?;
    eprintln!("[slashpad] moving new bundle into place...");
    if let Err(e) = std::fs::rename(&new_app, bundle) {
        // Rollback: try to restore the backup.
        eprintln!("[slashpad] move failed, rolling back: {e}");
        let _ = std::fs::rename(&backup, bundle);
        return Err(format!("failed to move new bundle into place: {e}"));
    }

    // Clean up backup and temp files.
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let _ = std::fs::remove_file(zip_path);

    eprintln!("[slashpad] bundle replacement complete");
    Ok(())
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
    is_archived, is_pinned, new_pin_tag, pin_tag_with, pin_timestamp, ChatId, ScreenKey,
    ChatMessageView, ChatState, ChatStatus, Mode, SessionInfo, Skill, TAG_ARCHIVED,
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
    /// Update check completed. `Some(info)` if newer, `None` if current.
    UpdateAvailable(Option<crate::updates::UpdateInfo>),
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
    /// User toggled the "Launch at login" checkbox (.app installs only).
    LaunchAtLoginToggled(bool),
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
    /// Window position changed. Fired both by user drags and by our own
    /// programmatic `iced::window::move_to` calls; `update` distinguishes
    /// them via `programmatic_move_pending` so only user drags are
    /// persisted into `dragged_positions`.
    WindowMoved {
        window_id: iced::window::Id,
        position: iced::Point,
    },
    /// Background update check completed.
    UpdateAvailable(Option<crate::updates::UpdateInfo>),
    /// User clicked "Upgrade to vX.Y.Z".
    UpgradeClicked,
    /// The background upgrade finished (success or failure).
    UpgradeFinished(Result<(), String>),
    /// ⌘K in the idle list — open or close the floating actions menu
    /// over the selected row. No-op when no row is selected or when
    /// the selected row has no `session_id` to tag.
    ToggleSessionMenu,
    /// ↵ while the options menu is open with Archive highlighted —
    /// tag the selected row's session as archived and (for active
    /// chats) close the chat.
    ArchiveSelectedRow,
    /// ↵ while the options menu is open with Pin highlighted — toggle
    /// the pinned tag on the selected row's session.
    TogglePinSelectedRow,
    /// ⌘⇧↑ in Idle when the selected row is pinned — slide the pin up
    /// one slot within the pinned block by swapping timestamps with
    /// the pinned neighbor above it. No-op when not pinned or already
    /// at the top of the pinned block.
    MovePinnedUp,
    /// ⌘⇧↓ in Idle when the selected row is pinned — slide the pin
    /// down one slot within the pinned block by swapping timestamps
    /// with the pinned neighbor below it. No-op when not pinned or
    /// already at the bottom of the pinned block.
    MovePinnedDown,
    /// Async result from `sessions::tag_session`. `Ok` triggers a
    /// refresh of the session list; `Err` restores the row.
    SessionTagged(Result<(), String>),
    /// ↵ while the options menu is open with Rename highlighted — swaps
    /// the selected row's title for an inline `text_input` pre-filled
    /// with the current title.
    BeginRenameSelectedRow,
    /// Typing into the inline rename text_input.
    RenameInputChanged(String),
    /// ↵ in the inline rename input — persist the new title via the SDK
    /// and exit edit mode.
    CommitRename,
    /// Esc while the inline rename input is active — abort without saving.
    CancelRename,
    /// Async result from `sessions::rename_session`. Triggers a
    /// `refresh_sessions` so past rows pick up the SDK-persisted summary.
    SessionRenamed(Result<(), String>),
    /// ↵ while the options menu is open over a Skill row with `Delete
    /// skill` highlighted — swaps the menu into its confirmation panel.
    BeginDeleteSkill,
    /// ↵ in the delete confirmation panel with `Delete` highlighted —
    /// removes the skill's directory from disk and reloads the list.
    ConfirmDeleteSkill,
    /// ↵ in the delete confirmation panel with `Cancel` highlighted, or
    /// Esc while the confirmation panel is open — returns the menu to
    /// the action list without deleting.
    CancelDeleteSkill,
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
    /// True when the floating actions submenu (⌘K) is open over the
    /// idle list. While open, `↵` runs the highlighted action and
    /// `esc` / `⌘K` dismiss the menu without changing the selection.
    pub session_menu_open: bool,
    /// Which row within the options menu is highlighted. 0 = Pin/Unpin,
    /// 1 = Archive, 2 = Rename. Reset to 0 on menu open/close.
    pub session_menu_selected: usize,
    /// When true, the options menu (open under `MenuTarget::Skill`) has
    /// been swapped into its two-step delete-confirmation panel. Rows
    /// are `[0 = Cancel, 1 = Delete]`; `session_menu_selected` is
    /// reinterpreted against that pair while this is set. Reset on
    /// menu open, cancel, confirm, or menu close.
    pub skill_delete_confirm: bool,
    /// Session id currently in inline-rename mode on the idle list.
    /// `Some(id)` means the row matching `id` renders a `text_input`
    /// in place of its title; all other rows render normally. Cleared
    /// on commit / cancel / mode transitions away from Idle.
    pub renaming_session_id: Option<String>,
    /// Draft value of the inline rename input, pre-filled with the row's
    /// current title when rename mode is entered.
    pub rename_input: String,
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

    /// Per-screen positions the user has dragged the palette window
    /// to. Keyed by `ScreenKey` (an NSScreen frame fingerprint). When
    /// `show_palette()` runs, it looks up the cursor's current screen
    /// and reuses the stored position if one exists; otherwise the
    /// palette cursor-centers on that screen. Raycast-style: dragging
    /// on monitor A doesn't affect monitor B's position. In-memory only.
    pub dragged_positions: HashMap<ScreenKey, iced::Point>,
    /// Armed right before we issue `iced::window::move_to` so the
    /// `window::Event::Moved` fired by that programmatic move isn't
    /// mistaken for a user drag. Cleared by the first `WindowMoved`
    /// observed after arming.
    pub programmatic_move_pending: bool,
    /// When true, the next `show_palette()` forces the palette back to
    /// `Mode::Skills` with `/` prefilled instead of restoring the
    /// previous view. Set when the palette hides while in Skills mode
    /// or when a skill is fired-and-forgotten (Cmd+Enter) — both cases
    /// where bringing back the same view would be stale. Cleared on
    /// open after being consumed.
    pub reset_to_skills_on_next_open: bool,

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
    /// A newer version exists. `download_url` is set for `.app` installs.
    Available {
        version: String,
        download_url: Option<String>,
    },
    /// Upgrade in progress (downloading and replacing the .app bundle).
    Upgrading,
}

impl Slashpad {
    pub fn new() -> (Self, Task<Message>) {
        // `init_external_bus()` is called from `main()` before iced starts,
        // so `external_sender()` is ready for the hotkey forwarder below.

        let settings = AppSettings::load_or_default();

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

        let all_skills =
            skills::load_skills(Some(&project_path), settings.load_user_settings)
                .unwrap_or_default();

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
            session_menu_open: false,
            session_menu_selected: 0,
            skill_delete_confirm: false,
            renaming_session_id: None,
            rename_input: String::new(),
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
            dragged_positions: HashMap::new(),
            programmatic_move_pending: false,
            // First-ever launch: default to the Skills prompt so the
            // user sees the picker immediately. Future opens restore
            // whatever view they were in when the palette was hidden.
            reset_to_skills_on_next_open: true,
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
            //
            // `Cmd+Shift+Arrow` is reserved for pinned-row reordering
            // (see `decode_shortcut`); let it fall through to
            // `listen_with` so Nav doesn't also fire.
            iced::keyboard::on_key_press(|key, modifiers| {
                if modifiers.command() && modifiers.shift() {
                    return None;
                }
                match key.as_ref() {
                    Key::Named(Named::ArrowUp) => Some(Message::NavUp),
                    Key::Named(Named::ArrowDown) => Some(Message::NavDown),
                    _ => None,
                }
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

        // Cmd+Shift+Arrow reorders pinned rows within the pinned block.
        // Checked before the Cmd+letter arms since arrows are Named, not
        // Character — routing is unambiguous either way, but keeping
        // these up here makes the intent obvious.
        if modifiers.command() && modifiers.shift() {
            if matches!(key.as_ref(), Key::Named(Named::ArrowUp)) {
                return Some(Message::MovePinnedUp);
            }
            if matches!(key.as_ref(), Key::Named(Named::ArrowDown)) {
                return Some(Message::MovePinnedDown);
            }
        }

        // Cmd+letter shortcuts. `ShortcutFilter` prevents the letter
        // from leaking into text_input; we don't need post-hoc stripping.
        if modifiers.command() && matches!(key.as_ref(), Key::Character("p")) {
            return Some(Message::OpenProjectPicker);
        }
        if modifiers.command() && matches!(key.as_ref(), Key::Character("t")) {
            return Some(Message::OpenSessionInTerminal);
        }
        if modifiers.command() && matches!(key.as_ref(), Key::Character("k")) {
            return Some(Message::ToggleSessionMenu);
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
                    self.apply_pinned_project_sort();
                    let max = self.filtered_projects.len().saturating_sub(1);
                    if self.selected_project_index > max {
                        self.selected_project_index = 0;
                    }
                    return Task::none();
                }
                if let Some(query) = value.strip_prefix('/') {
                    // Re-scan skills from disk every time we enter skill
                    // mode so newly created skills appear without
                    // reopening the palette.
                    if self.mode != Mode::Skills {
                        self.all_skills = skills::load_skills(
                            Some(&self.project_path),
                            self.settings.load_user_settings,
                        )
                        .unwrap_or_default();
                    }
                    self.filtered_skills = if query.is_empty() {
                        self.all_skills.clone()
                    } else {
                        crate::fuzzy::filter_skills(&self.all_skills, query)
                    };
                    self.apply_pinned_skill_sort();
                    self.mode = Mode::Skills;
                    self.selected_skill_index = 0;
                } else {
                    let entering_idle = self.mode == Mode::Skills;
                    if entering_idle {
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
                    if entering_idle {
                        return snap_to_selection(
                            IDLE_LIST_SCROLL_ID.clone(),
                            0,
                            self.idle_row_count(),
                        );
                    }
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
                snap_to_selection(IDLE_LIST_SCROLL_ID.clone(), 0, self.idle_row_count())
            }

            Message::Submit => {
                // Inline rename input steals ↵ — commit the new title.
                // Takes precedence over every other Submit behavior so
                // Enter doesn't also open/send while editing.
                if self.renaming_session_id.is_some() {
                    return self.dispatch_update(Message::CommitRename);
                }
                // While the options menu is open, ↵ is rebound to the
                // highlighted action — never the normal Submit path.
                // This lets the menu feel like a modal overlay without
                // a separate key-handling layer.
                //
                // Session target rows: 0 = Rename, 1 = Pin/Unpin, 2 = Archive.
                // Skill / Project targets only have Pin at index 0.
                if self.session_menu_open {
                    // Delete-confirm panel shadows the normal action
                    // routing: row 0 = Cancel, row 1 = Delete.
                    if self.skill_delete_confirm {
                        return self.dispatch_update(match self.session_menu_selected {
                            1 => Message::ConfirmDeleteSkill,
                            _ => Message::CancelDeleteSkill,
                        });
                    }
                    return self.dispatch_update(match (self.mode, self.session_menu_selected) {
                        (Mode::Idle, 0) => Message::BeginRenameSelectedRow,
                        (Mode::Idle, 1) => Message::TogglePinSelectedRow,
                        (Mode::Idle, 2) => Message::ArchiveSelectedRow,
                        (Mode::Skills, 1) => Message::BeginDeleteSkill,
                        _ => Message::TogglePinSelectedRow,
                    });
                }
                self.handle_submit()
            }

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
                // If a row is in inline-rename mode, Esc aborts the rename
                // without saving and restores focus to the main input.
                // Takes precedence over every other Esc behavior so the
                // palette stays open while the user backs out of editing.
                if self.renaming_session_id.is_some() {
                    return self.dispatch_update(Message::CancelRename);
                }
                // If the options menu is open, Esc closes it and keeps
                // the underlying idle-list selection intact. Takes
                // precedence over every other Esc behavior.
                //
                // Exception: when the delete-confirmation panel is
                // showing, Esc steps back to the action list (one
                // level) rather than closing the whole menu — so the
                // user can back out of a confirm without losing the
                // menu they just opened.
                if self.session_menu_open {
                    if self.skill_delete_confirm {
                        return self.dispatch_update(Message::CancelDeleteSkill);
                    }
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                }
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
                    return Task::batch([
                        text_input::focus(INPUT_ID.clone()),
                        snap_to_selection(IDLE_LIST_SCROLL_ID.clone(), 0, self.idle_row_count()),
                    ]);
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
                // While the options menu is open, arrow keys move
                // within the menu rows (0 = Pin/Unpin, 1 = Archive)
                // and never leak through to the idle list behind it.
                if self.session_menu_open {
                    if self.session_menu_selected > 0 {
                        self.session_menu_selected -= 1;
                    }
                    return Task::none();
                }
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
                if self.session_menu_open {
                    let max = if self.skill_delete_confirm {
                        1 // Cancel / Delete
                    } else {
                        menu_target_for_mode(self.mode)
                            .map(|t| t.row_count().saturating_sub(1))
                            .unwrap_or(0)
                    };
                    if self.session_menu_selected < max {
                        self.session_menu_selected += 1;
                    }
                    return Task::none();
                }
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
                // Clicking a row while another row is in rename mode
                // abandons the edit — the user clearly moved on.
                self.renaming_session_id = None;
                self.rename_input.clear();
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
                self.renaming_session_id = None;
                self.rename_input.clear();
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
                    self.apply_pinned_project_sort();
                    self.selected_project_index = 0;
                }
                Task::none()
            }

            Message::UpdateAvailable(info) => {
                self.update_status = match info {
                    Some(i) => UpdateStatus::Available {
                        version: i.version,
                        download_url: i.download_url,
                    },
                    None => UpdateStatus::UpToDate,
                };
                Task::none()
            }

            Message::UpgradeClicked => {
                if matches!(self.update_status, UpdateStatus::Upgrading) {
                    return Task::none();
                }

                // Capture the download URL before we overwrite the status.
                let download_url = match &self.update_status {
                    UpdateStatus::Available { download_url, .. } => download_url.clone(),
                    _ => None,
                };

                self.update_status = UpdateStatus::Upgrading;

                let Some(url) = download_url else {
                    eprintln!("[slashpad] upgrade clicked but no download URL available");
                    self.update_status = UpdateStatus::Idle;
                    return Task::none();
                };

                // Download the new .app zip, replace the current bundle, relaunch.
                let bundle_path = app_bundle_path();
                Task::perform(
                    async move {
                        let zip_path = crate::updates::download_update(&url).await?;
                        // replace_app_bundle is blocking (runs unzip),
                        // so move it off the async executor.
                        tokio::task::spawn_blocking(move || {
                            replace_app_bundle(&zip_path, bundle_path.as_deref())
                        })
                        .await
                        .map_err(|e| format!("spawn_blocking failed: {e}"))?
                    },
                    Message::UpgradeFinished,
                )
            }

            Message::UpgradeFinished(result) => {
                match result {
                    Ok(()) => {
                        self.chats.clear();
                        // Relaunch the .app via `open -n` so launchd
                        // always spawns a fresh instance — without
                        // `-n`, Launch Services sees this still-running
                        // process and just "activates" it, which does
                        // nothing once we exit below. A short sleep
                        // gives `open` time to complete the Launch
                        // Services handshake before the current
                        // process tears down.
                        if let Some(path) = app_bundle_path() {
                            let _ = std::process::Command::new("open")
                                .arg("-n")
                                .arg(path)
                                .spawn();
                            std::thread::sleep(std::time::Duration::from_millis(400));
                        }
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("[slashpad] upgrade failed: {e}");
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
                self.apply_pinned_project_sort();
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
                        // event so it isn't logged as a user drag.
                        self.programmatic_move_pending = false;
                    } else {
                        // Attribute the drag to whichever screen now
                        // contains the palette. Walking NSScreen::screens
                        // from a top-left point lets dragging across
                        // monitors update the correct entry.
                        #[cfg(target_os = "macos")]
                        if let Some(key) =
                            crate::platform::macos::screen_key_for_point(position)
                        {
                            self.dragged_positions.insert(key, position);
                        }
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
                    skills::load_skills(Some(&self.project_path), enabled).unwrap_or_default();
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
                self.apply_pinned_skill_sort();
                self.selected_skill_index = 0;
                Task::none()
            }

            Message::LaunchAtLoginToggled(enabled) => {
                // Do the SMAppService call first so the persisted value
                // tracks the actual login-item state. If registration
                // fails (unsigned dev build, missing bundle, etc.) we
                // leave the checkbox unchecked rather than silently
                // claiming success.
                #[cfg(target_os = "macos")]
                let applied = {
                    if enabled {
                        crate::platform::macos::register_login_item()
                    } else {
                        crate::platform::macos::unregister_login_item()
                    }
                };
                #[cfg(not(target_os = "macos"))]
                let applied = enabled;

                self.settings.launch_at_login = if enabled { applied } else { false };
                if let Err(e) = self.settings.save() {
                    eprintln!("[slashpad] failed to save launchAtLogin: {e}");
                }
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

            Message::ToggleSessionMenu => {
                // The ⌘K menu opens in three modes: Idle (sessions),
                // Skills (skill pins), ProjectPicker (project pins).
                // Ignores the chord elsewhere so it feels dead rather
                // than surprising.
                if self.session_menu_open {
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                }
                let can_open = match self.mode {
                    Mode::Idle => {
                        !self.input.starts_with('/')
                            && self.idle_row_count() > 0
                            && self.idle_selection_active
                            && self.selected_row_session_id().is_some()
                    }
                    Mode::Skills => {
                        !self.filtered_skills.is_empty()
                            && self.selected_skill_index < self.filtered_skills.len()
                    }
                    Mode::ProjectPicker => {
                        !self.filtered_projects.is_empty()
                            && self.selected_project_index < self.filtered_projects.len()
                    }
                    _ => false,
                };
                if !can_open {
                    return Task::none();
                }
                self.session_menu_open = true;
                self.session_menu_selected = 0;
                self.skill_delete_confirm = false;
                Task::none()
            }

            Message::ArchiveSelectedRow => {
                // Resolve the selected row's session_id and (if active)
                // chat_id up front, while the `build_idle_rows` borrow
                // is still alive — so we can drop it before mutating
                // self.chats / self.recent_sessions below.
                let rows = self.build_idle_rows();
                let selected = rows.get(self.selected_idle_index).cloned();
                let Some(row) = selected else {
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                };
                let (session_id, archived_chat_id) = match &row {
                    IdleRowSelection::Active(chat_id) => {
                        let sid = self
                            .chats
                            .get(chat_id)
                            .and_then(|e| e.state.session_id.clone());
                        (sid, Some(*chat_id))
                    }
                    IdleRowSelection::Past(info) => (Some(info.session_id.clone()), None),
                };
                let Some(session_id) = session_id else {
                    // Active chat without a session_id yet — archive is
                    // impossible. Close the menu so the user knows the
                    // action didn't fire.
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                };

                // Optimistic UI: hide the row immediately so the list
                // collapses before tagSession returns. `refresh_sessions`
                // after success reconciles against the SDK's view.
                self.recent_sessions
                    .retain(|s| s.session_id != session_id);
                if let Some(chat_id) = archived_chat_id {
                    // Drop the ChatEntry — its SpawnedSidecar has
                    // `kill_on_drop` on the child process, so
                    // runner.mjs exits immediately.
                    self.chats.remove(&chat_id);
                    if self.active_chat_id == Some(chat_id) {
                        self.active_chat_id = None;
                    }
                }

                // Clamp selection into the new row count.
                let count = self.idle_row_count();
                if count == 0 {
                    self.selected_idle_index = 0;
                    self.idle_selection_active = false;
                } else if self.selected_idle_index >= count {
                    self.selected_idle_index = count - 1;
                }

                self.session_menu_open = false;
                self.session_menu_selected = 0;

                let cwd = self.project_path.clone();
                let sid_for_task = session_id.clone();
                Task::perform(
                    async move {
                        crate::sessions::tag_session(
                            &cwd,
                            &sid_for_task,
                            Some(TAG_ARCHIVED),
                        )
                        .await
                        .map_err(|e| e.to_string())
                    },
                    Message::SessionTagged,
                )
            }

            Message::TogglePinSelectedRow => {
                // Skills / ProjectPicker fork off to their own local
                // pin-toggle paths; only Idle goes through the SDK
                // `tag_session` round-trip below.
                if self.mode == Mode::Skills {
                    return self.toggle_pin_selected_skill();
                }
                if self.mode == Mode::ProjectPicker {
                    return self.toggle_pin_selected_project();
                }
                let rows = self.build_idle_rows();
                let selected = rows.get(self.selected_idle_index).cloned();
                let Some(row) = selected else {
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                };
                let session_id = match &row {
                    IdleRowSelection::Active(chat_id) => self
                        .chats
                        .get(chat_id)
                        .and_then(|e| e.state.session_id.clone()),
                    IdleRowSelection::Past(info) => Some(info.session_id.clone()),
                };
                let Some(session_id) = session_id else {
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                };

                // Read current pin state from the recent_sessions cache
                // — active chats borrow their tag from whichever past
                // session shares their session_id. If the SDK list
                // hasn't returned yet for an in-flight chat, treat it
                // as unpinned (pin will set it on the next refresh).
                let currently_pinned = self
                    .recent_sessions
                    .iter()
                    .find(|s| s.session_id == session_id)
                    .map(|s| is_pinned(s.tag.as_deref()))
                    .unwrap_or(false);
                // Timestamp the pin so the pinned block sorts newest
                // pin to the bottom. Passing `None` clears the tag
                // and unpins.
                let new_tag: Option<String> = if currently_pinned {
                    None
                } else {
                    Some(new_pin_tag())
                };

                // Optimistic local update so the 📌 + reordering happen
                // before the sidecar round-trip. `refresh_sessions()`
                // on SessionTagged reconciles against the SDK.
                if let Some(info) = self
                    .recent_sessions
                    .iter_mut()
                    .find(|s| s.session_id == session_id)
                {
                    info.tag = new_tag.clone();
                }

                self.session_menu_open = false;
                self.session_menu_selected = 0;

                let cwd = self.project_path.clone();
                let sid_for_task = session_id.clone();
                let tag_for_task = new_tag;
                Task::perform(
                    async move {
                        crate::sessions::tag_session(
                            &cwd,
                            &sid_for_task,
                            tag_for_task.as_deref(),
                        )
                        .await
                        .map_err(|e| e.to_string())
                    },
                    Message::SessionTagged,
                )
            }

            Message::BeginDeleteSkill => {
                // Only valid when the options menu is open over a skill
                // row. Swap the menu into its two-row confirm panel with
                // `Cancel` highlighted by default.
                if !self.session_menu_open || self.mode != Mode::Skills {
                    return Task::none();
                }
                if self
                    .filtered_skills
                    .get(self.selected_skill_index)
                    .is_none()
                {
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                }
                self.skill_delete_confirm = true;
                self.session_menu_selected = 0;
                Task::none()
            }

            Message::CancelDeleteSkill => {
                // Step back from the confirm panel to the regular action
                // list. Keeps the menu open so the user can reach for a
                // different action without re-summoning ⌘K.
                self.skill_delete_confirm = false;
                self.session_menu_selected = 0;
                Task::none()
            }

            Message::ConfirmDeleteSkill => {
                let Some(skill) = self
                    .filtered_skills
                    .get(self.selected_skill_index)
                    .cloned()
                else {
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    self.skill_delete_confirm = false;
                    return Task::none();
                };
                if let Err(e) = skills::delete_skill(&skill) {
                    eprintln!(
                        "[slashpad] failed to delete skill {:?}: {e}",
                        skill.name
                    );
                }
                // Reload the canonical list and rebuild the filtered view
                // against the current `/query` so the deleted row drops
                // out (or a permission error is reflected by the skill
                // staying visible after the reload round-trip).
                self.all_skills = skills::load_skills(
                    Some(&self.project_path),
                    self.settings.load_user_settings,
                )
                .unwrap_or_default();
                let query = self.input.strip_prefix('/').unwrap_or("");
                self.filtered_skills = if query.is_empty() {
                    self.all_skills.clone()
                } else {
                    crate::fuzzy::filter_skills(&self.all_skills, query)
                };
                self.apply_pinned_skill_sort();
                let count = self.filtered_skills.len();
                if self.selected_skill_index >= count {
                    self.selected_skill_index = count.saturating_sub(1);
                }
                self.session_menu_open = false;
                self.session_menu_selected = 0;
                self.skill_delete_confirm = false;
                Task::none()
            }

            Message::MovePinnedUp => match self.mode {
                Mode::Skills => self.move_pinned_skill_row(-1),
                Mode::ProjectPicker => self.move_pinned_project_row(-1),
                _ => self.move_pinned_row(-1),
            },
            Message::MovePinnedDown => match self.mode {
                Mode::Skills => self.move_pinned_skill_row(1),
                Mode::ProjectPicker => self.move_pinned_project_row(1),
                _ => self.move_pinned_row(1),
            },

            Message::SessionTagged(result) => {
                match result {
                    Ok(()) => {
                        // Re-fetch to pick up the SDK-persisted tag.
                        // The filter in `past_session_rows` keeps
                        // archived entries out of the idle list.
                        self.refresh_sessions()
                    }
                    Err(e) => {
                        // Tag call failed — log and refetch so the UI
                        // can fall back to reality (the optimistic
                        // removal gets undone by the fresh list).
                        eprintln!("[slashpad] tagSession failed: {e}");
                        self.refresh_sessions()
                    }
                }
            }

            Message::BeginRenameSelectedRow => {
                // Resolve the selected row's session_id and current
                // displayed title so we can pre-fill the inline input.
                let rows = self.build_idle_rows();
                let Some(row) = rows.get(self.selected_idle_index).cloned() else {
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                };
                let (session_id, title) = match &row {
                    IdleRowSelection::Active(chat_id) => {
                        let entry = self.chats.get(chat_id);
                        let sid = entry.and_then(|e| e.state.session_id.clone());
                        let title = entry.map(|e| e.state.title.clone()).unwrap_or_default();
                        (sid, title)
                    }
                    IdleRowSelection::Past(info) => {
                        (Some(info.session_id.clone()), info.summary.clone())
                    }
                };
                let Some(session_id) = session_id else {
                    // Active chat without a session_id yet — rename is
                    // impossible. Close the menu so the user can see the
                    // action didn't fire.
                    self.session_menu_open = false;
                    self.session_menu_selected = 0;
                    return Task::none();
                };

                self.renaming_session_id = Some(session_id);
                self.rename_input = title;
                self.session_menu_open = false;
                self.session_menu_selected = 0;
                text_input::focus(RENAME_INPUT_ID.clone())
            }

            Message::RenameInputChanged(value) => {
                self.rename_input = value;
                Task::none()
            }

            Message::CommitRename => {
                let Some(session_id) = self.renaming_session_id.take() else {
                    return Task::none();
                };
                let new_title = self.rename_input.trim().to_string();
                self.rename_input.clear();

                // Empty or whitespace-only title is a no-op — clearing a
                // session name isn't a supported SDK operation, so we
                // treat this the same as Esc-cancel.
                if new_title.is_empty() {
                    return text_input::focus(INPUT_ID.clone());
                }

                // Optimistic UI: update any active chat's in-memory title
                // and the corresponding SessionInfo summary so the row
                // re-renders immediately. refresh_sessions on SessionRenamed
                // reconciles against the SDK.
                for entry in self.chats.values_mut() {
                    if entry.state.session_id.as_deref() == Some(session_id.as_str()) {
                        entry.state.title = new_title.clone();
                    }
                }
                if let Some(info) = self
                    .recent_sessions
                    .iter_mut()
                    .find(|s| s.session_id == session_id)
                {
                    info.summary = new_title.clone();
                }

                let cwd = self.project_path.clone();
                let sid_for_task = session_id;
                let title_for_task = new_title;
                Task::batch([
                    text_input::focus(INPUT_ID.clone()),
                    Task::perform(
                        async move {
                            crate::sessions::rename_session(
                                &cwd,
                                &sid_for_task,
                                &title_for_task,
                            )
                            .await
                            .map_err(|e| e.to_string())
                        },
                        Message::SessionRenamed,
                    ),
                ])
            }

            Message::CancelRename => {
                self.renaming_session_id = None;
                self.rename_input.clear();
                text_input::focus(INPUT_ID.clone())
            }

            Message::SessionRenamed(result) => {
                match result {
                    Ok(()) => self.refresh_sessions(),
                    Err(e) => {
                        eprintln!("[slashpad] renameSession failed: {e}");
                        // Re-sync from truth so the optimistic title
                        // update rolls back if the SDK rejected the rename.
                        self.refresh_sessions()
                    }
                }
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
                self.settings.launch_at_login,
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
        let mut body: Column<'_, Message> = column![drag_strip, input]
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
                    body = body.push(ui::theme::divider());
                    let skill_rows: Vec<ui::skill_list::SkillRow<'_>> = self
                        .filtered_skills
                        .iter()
                        .map(|s| ui::skill_list::SkillRow {
                            skill: s,
                            pinned: self.skill_is_pinned(&s.name),
                        })
                        .collect();
                    body = body.push(ui::skill_list::view(
                        skill_rows,
                        self.selected_skill_index,
                        SKILL_LIST_SCROLL_ID.clone(),
                    ));
                    has_fill_middle = true;
                }
            }
            Mode::ProjectPicker => {
                body = body.push(ui::theme::divider());
                let project_rows: Vec<ui::project_picker::ProjectRow<'_>> = self
                    .filtered_projects
                    .iter()
                    .map(|p| ui::project_picker::ProjectRow {
                        project: p,
                        pinned: self
                            .project_is_pinned(p.path.to_string_lossy().as_ref()),
                    })
                    .collect();
                body = body.push(ui::project_picker::view(
                    project_rows,
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
                // Build pin-first ordering that matches the selection
                // list from `build_idle_rows()` exactly so arrow-key
                // indices stay aligned with what the user sees.
                // Within pinned, sort oldest-pin-first so the most
                // recently pinned item lands at the bottom.
                let visible_chat_ids = self.visible_active_chat_ids();
                let visible_past = self.visible_past_session_rows();
                let capacity = visible_chat_ids.len() + visible_past.len();
                let mut pinned_rows: Vec<(i64, ui::idle_list::IdleRow<'_>)> =
                    Vec::with_capacity(capacity);
                let mut rest_rows: Vec<ui::idle_list::IdleRow<'_>> =
                    Vec::with_capacity(capacity);
                for id in &visible_chat_ids {
                    if let Some(entry) = self.chats.get(id) {
                        let sid = entry.state.session_id.as_deref();
                        let is_pin = sid.map(|s| self.session_is_pinned(s)).unwrap_or(false);
                        let ts = sid.and_then(|s| self.session_pin_timestamp(s)).unwrap_or(0);
                        let row = ui::idle_list::IdleRow::Active { entry, pinned: is_pin };
                        if is_pin {
                            pinned_rows.push((ts, row));
                        } else {
                            rest_rows.push(row);
                        }
                    }
                }
                for session in visible_past {
                    let is_pin = is_pinned(session.tag.as_deref());
                    let ts = pin_timestamp(session.tag.as_deref()).unwrap_or(0);
                    let row = ui::idle_list::IdleRow::Past { session, pinned: is_pin };
                    if is_pin {
                        pinned_rows.push((ts, row));
                    } else {
                        rest_rows.push(row);
                    }
                }
                pinned_rows.sort_by_key(|(ts, _)| *ts);
                let mut rows: Vec<ui::idle_list::IdleRow<'_>> =
                    pinned_rows.into_iter().map(|(_, r)| r).collect();
                rows.extend(rest_rows);
                let selected = if self.idle_selection_active {
                    self.selected_idle_index
                } else {
                    usize::MAX
                };
                body = body.push(ui::theme::divider());
                body = body.push(ui::idle_list::view(
                    rows,
                    selected,
                    self.spinner_frame,
                    IDLE_LIST_SCROLL_ID.clone(),
                    self.renaming_session_id.as_deref(),
                    &self.rename_input,
                ));
                has_fill_middle = true;
            }
            Mode::Chatting => {
                if let Some(entry) = self.active_chat() {
                    let is_generating = !matches!(
                        entry.state.status,
                        ChatStatus::Idle | ChatStatus::Closed | ChatStatus::Error
                    );
                    body = body.push(ui::theme::divider());
                    body = body.push(ui::chat_panel::view(
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
            body = body.push(iced::widget::vertical_space().height(iced::Length::Fill));
        }

        body = body.push(ui::theme::divider());
        let can_open_options = match self.mode {
            Mode::Idle => {
                !self.input.starts_with('/')
                    && self.idle_selection_active
                    && self.idle_row_count() > 0
                    && self.selected_row_session_id().is_some()
            }
            Mode::Skills => !self.filtered_skills.is_empty(),
            Mode::ProjectPicker => !self.filtered_projects.is_empty(),
            _ => false,
        };
        let selected_is_pinned = match self.mode {
            Mode::Idle => self.selected_row_is_pinned(),
            Mode::Skills => self
                .filtered_skills
                .get(self.selected_skill_index)
                .map(|s| self.skill_is_pinned(&s.name))
                .unwrap_or(false),
            Mode::ProjectPicker => self
                .filtered_projects
                .get(self.selected_project_index)
                .map(|p| self.project_is_pinned(p.path.to_string_lossy().as_ref()))
                .unwrap_or(false),
            _ => false,
        };
        let pinned_count = match self.mode {
            Mode::Idle => self
                .build_idle_rows()
                .iter()
                .filter(|r| match r {
                    IdleRowSelection::Active(id) => self
                        .chats
                        .get(id)
                        .and_then(|e| e.state.session_id.as_deref())
                        .map(|s| self.session_is_pinned(s))
                        .unwrap_or(false),
                    IdleRowSelection::Past(info) => is_pinned(info.tag.as_deref()),
                })
                .count(),
            Mode::Skills => self
                .filtered_skills
                .iter()
                .take_while(|s| self.skill_is_pinned(&s.name))
                .count(),
            Mode::ProjectPicker => self
                .filtered_projects
                .iter()
                .take_while(|p| self.project_is_pinned(p.path.to_string_lossy().as_ref()))
                .count(),
            _ => 0,
        };
        body = body.push(ui::keyhints::view(
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
                session_menu_open: self.session_menu_open,
                can_open_options,
                session_menu_selected: self.session_menu_selected,
                renaming: self.renaming_session_id.is_some(),
                selected_is_pinned,
                pinned_count,
            },
        ));

        // Always wrap the body in a two-layer stack so the widget-tree
        // path of the idle list's scrollable stays stable across
        // menu-open/close transitions. Swapping the root widget type
        // (Column ↔ Stack) makes iced throw away child state keyed by
        // tree position — in particular the scrollable's scroll offset,
        // which caused the list to jump to the top when ⌘K toggled.
        // Layer 1 is the menu when open, or a zero-size Space that
        // doesn't intercept clicks when closed.
        let overlay: Element<'_, Message> = if self.session_menu_open {
            let target = menu_target_for_mode(self.mode)
                .unwrap_or(ui::session_options_menu::MenuTarget::Session);
            let (can_act, is_pinned) = match target {
                ui::session_options_menu::MenuTarget::Session => (
                    self.selected_row_session_id().is_some(),
                    self.selected_row_is_pinned(),
                ),
                ui::session_options_menu::MenuTarget::Skill => {
                    let pinned = self
                        .filtered_skills
                        .get(self.selected_skill_index)
                        .map(|s| self.skill_is_pinned(&s.name))
                        .unwrap_or(false);
                    (true, pinned)
                }
                ui::session_options_menu::MenuTarget::Project => {
                    let pinned = self
                        .filtered_projects
                        .get(self.selected_project_index)
                        .map(|p| {
                            self.project_is_pinned(p.path.to_string_lossy().as_ref())
                        })
                        .unwrap_or(false);
                    (true, pinned)
                }
            };
            let confirming_name: Option<String> =
                if self.skill_delete_confirm && target == ui::session_options_menu::MenuTarget::Skill {
                    self.filtered_skills
                        .get(self.selected_skill_index)
                        .map(|s| s.name.clone())
                } else {
                    None
                };
            ui::session_options_menu::view(
                target,
                self.session_menu_selected,
                can_act,
                is_pinned,
                confirming_name.as_deref(),
            )
        } else {
            Space::new(iced::Length::Fixed(0.0), iced::Length::Fixed(0.0)).into()
        };
        let layered: Element<'_, Message> = iced::widget::stack![body, overlay]
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into();

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
        let card = container(layered)
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
                    self.apply_pinned_skill_sort();
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
                // Reload skills for the new project: project-scoped
                // skills live under `<project>/.claude/skills`, so the
                // palette's skill list differs between projects.
                self.all_skills = skills::load_skills(
                    Some(&self.project_path),
                    self.settings.load_user_settings,
                )
                .unwrap_or_default();
                // If Cmd+P was pressed while in Skills mode (input was
                // `/...`), drop back into Skills mode in the new
                // project with that query preserved. Otherwise land
                // on a clean Idle view.
                let prev_input = self.input_before_picker.take().unwrap_or_default();
                let stay_in_skills = prev_input.starts_with('/');
                self.active_chat_id = None;
                self.filtered_projects.clear();
                self.selected_project_index = 0;
                if stay_in_skills {
                    self.input = prev_input;
                    let query = self.input.strip_prefix('/').unwrap_or("");
                    self.filtered_skills = if query.is_empty() {
                        self.all_skills.clone()
                    } else {
                        crate::fuzzy::filter_skills(&self.all_skills, query)
                    };
                    self.apply_pinned_skill_sort();
                    self.mode = Mode::Skills;
                    self.selected_skill_index = 0;
                } else {
                    self.mode = Mode::Idle;
                    self.input.clear();
                    self.selected_idle_index = 0;
                    self.idle_selection_active = false;
                }
                // Past-sessions list is scoped per-cwd, so a project
                // switch needs a re-fetch. Also reset the idle scroll
                // offset: iced retains scrollable position by id, so
                // without this the new project's list would display
                // mid-scrolled if the old one was scrolled down.
                self.recent_sessions.clear();
                Task::batch([
                    snap_to_selection(IDLE_LIST_SCROLL_ID.clone(), 0, self.idle_row_count()),
                    self.refresh_sessions(),
                ])
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
    /// active chat (by `session_id` match) and any tagged as archived.
    /// Returned as owned clones because borrow-lifetime rules for
    /// `&self` + later mutations through `handle_submit` etc. are
    /// simpler with owned rows.
    fn past_session_rows(&self) -> Vec<SessionInfo> {
        let active_ids: std::collections::HashSet<&str> = self
            .chats
            .values()
            .filter_map(|c| c.state.session_id.as_deref())
            .collect();
        self.recent_sessions
            .iter()
            .filter(|s| !active_ids.contains(s.session_id.as_str()))
            .filter(|s| !is_archived(s.tag.as_deref()))
            .cloned()
            .collect()
    }

    /// Session id of the currently-selected idle-list row, if any.
    /// `None` for an active chat that hasn't received its `session_id`
    /// from the sidecar yet — in that state archive / options are not
    /// yet available.
    fn selected_row_session_id(&self) -> Option<String> {
        let rows = self.build_idle_rows();
        match rows.get(self.selected_idle_index)? {
            IdleRowSelection::Active(chat_id) => self
                .chats
                .get(chat_id)
                .and_then(|e| e.state.session_id.clone()),
            IdleRowSelection::Past(info) => Some(info.session_id.clone()),
        }
    }

    /// True when the selected row's session is tagged as pinned.
    /// Used by the options menu to swap between "Pin session" and
    /// "Unpin session" as the top action.
    fn selected_row_is_pinned(&self) -> bool {
        let Some(sid) = self.selected_row_session_id() else {
            return false;
        };
        self.session_is_pinned(&sid)
    }

    /// Lookup a session's pinned state from `recent_sessions`. Active
    /// chats share their tag with the corresponding past SessionInfo
    /// by session_id match. Returns false if the SDK list hasn't
    /// returned that session yet (treat as unpinned).
    fn session_is_pinned(&self, session_id: &str) -> bool {
        self.recent_sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .map(|s| is_pinned(s.tag.as_deref()))
            .unwrap_or(false)
    }

    /// Pin timestamp (unix millis) for a session, if its tag carries
    /// one. Used to sort within the pinned block — oldest-pin-first
    /// so a freshly-pinned item lands at the bottom.
    fn session_pin_timestamp(&self, session_id: &str) -> Option<i64> {
        pin_timestamp(
            self.recent_sessions
                .iter()
                .find(|s| s.session_id == session_id)
                .and_then(|s| s.tag.as_deref()),
        )
    }

    /// Move the selected idle-list row up (direction=-1) or down
    /// (direction=+1) within the pinned block by swapping pin
    /// timestamps with the pinned neighbor in that direction.
    ///
    /// Since pinned-block order is determined by the unix-millis
    /// timestamp embedded in the `pinned:<ts>` tag (ascending = top
    /// to bottom), swapping two neighbors' timestamps is equivalent
    /// to swapping their display positions — no other pins are
    /// affected. The update is applied optimistically to
    /// `recent_sessions` and persisted via two `tag_session` calls;
    /// the follow-up refresh reconciles against the SDK.
    fn move_pinned_row(&mut self, direction: i32) -> Task<Message> {
        if self.mode != Mode::Idle
            || !self.idle_selection_active
            || self.input.starts_with('/')
            || self.session_menu_open
        {
            return Task::none();
        }

        let rows = self.build_idle_rows();
        let Some(selected_row) = rows.get(self.selected_idle_index).cloned() else {
            return Task::none();
        };
        let selected_sid = match &selected_row {
            IdleRowSelection::Active(chat_id) => self
                .chats
                .get(chat_id)
                .and_then(|e| e.state.session_id.clone()),
            IdleRowSelection::Past(info) => Some(info.session_id.clone()),
        };
        let Some(selected_sid) = selected_sid else {
            return Task::none();
        };
        if !self.session_is_pinned(&selected_sid) {
            return Task::none();
        }

        // The pinned block is the contiguous prefix of `rows` — walk
        // until we hit the first unpinned row. Each pinned row keeps
        // its unified-list index so the selection can follow the
        // moved row to its new position.
        let mut pinned_rows: Vec<(usize, String)> = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let sid = match r {
                IdleRowSelection::Active(chat_id) => self
                    .chats
                    .get(chat_id)
                    .and_then(|e| e.state.session_id.clone()),
                IdleRowSelection::Past(info) => Some(info.session_id.clone()),
            };
            let Some(sid) = sid else { break };
            if !self.session_is_pinned(&sid) {
                break;
            }
            pinned_rows.push((i, sid));
        }

        let Some(my_pos) = pinned_rows.iter().position(|(_, s)| s == &selected_sid) else {
            return Task::none();
        };
        let target = my_pos as i32 + direction;
        if target < 0 || target >= pinned_rows.len() as i32 {
            return Task::none();
        }
        let (neighbor_row_idx, neighbor_sid) = pinned_rows[target as usize].clone();

        let my_ts = self.session_pin_timestamp(&selected_sid).unwrap_or(0);
        let neighbor_ts = self.session_pin_timestamp(&neighbor_sid).unwrap_or(0);
        if my_ts == neighbor_ts {
            // Two pins with identical timestamps (e.g. both legacy
            // bare `pinned` tags sharing ts=0) would swap into a
            // no-op. Give the moved row a ±1ms offset to force an
            // observable reorder.
            let bumped = match direction {
                d if d < 0 => neighbor_ts.saturating_sub(1),
                _ => neighbor_ts.saturating_add(1),
            };
            let new_tag = pin_tag_with(bumped);
            if let Some(info) = self
                .recent_sessions
                .iter_mut()
                .find(|s| s.session_id == selected_sid)
            {
                info.tag = Some(new_tag.clone());
            }
            self.selected_idle_index = neighbor_row_idx;
            let cwd = self.project_path.clone();
            let sid = selected_sid.clone();
            return Task::batch([
                snap_to_selection(
                    IDLE_LIST_SCROLL_ID.clone(),
                    self.selected_idle_index,
                    self.idle_row_count(),
                ),
                Task::perform(
                    async move {
                        crate::sessions::tag_session(&cwd, &sid, Some(&new_tag))
                            .await
                            .map_err(|e| e.to_string())
                    },
                    Message::SessionTagged,
                ),
            ]);
        }

        let my_new_tag = pin_tag_with(neighbor_ts);
        let neighbor_new_tag = pin_tag_with(my_ts);

        for info in self.recent_sessions.iter_mut() {
            if info.session_id == selected_sid {
                info.tag = Some(my_new_tag.clone());
            } else if info.session_id == neighbor_sid {
                info.tag = Some(neighbor_new_tag.clone());
            }
        }

        self.selected_idle_index = neighbor_row_idx;

        let cwd = self.project_path.clone();
        let sid1 = selected_sid.clone();
        let tag1 = my_new_tag.clone();
        let sid2 = neighbor_sid.clone();
        let tag2 = neighbor_new_tag.clone();
        Task::batch([
            snap_to_selection(
                IDLE_LIST_SCROLL_ID.clone(),
                self.selected_idle_index,
                self.idle_row_count(),
            ),
            Task::perform(
                async move {
                    let r1 = crate::sessions::tag_session(&cwd, &sid1, Some(&tag1))
                        .await
                        .map_err(|e| e.to_string());
                    let r2 = crate::sessions::tag_session(&cwd, &sid2, Some(&tag2))
                        .await
                        .map_err(|e| e.to_string());
                    r1.and(r2)
                },
                Message::SessionTagged,
            ),
        ])
    }

    /// True when the named skill is currently pinned by the user. Pin
    /// state lives in `AppSettings::pinned_skills`.
    fn skill_is_pinned(&self, name: &str) -> bool {
        self.settings.pinned_skills.contains_key(name)
    }

    /// Pin timestamp for a skill, if pinned. Same ASC-ordering
    /// semantics as `session_pin_timestamp`.
    fn skill_pin_timestamp(&self, name: &str) -> Option<i64> {
        self.settings.pinned_skills.get(name).copied()
    }

    /// True when the given absolute project path is pinned.
    fn project_is_pinned(&self, path: &str) -> bool {
        self.settings.pinned_projects.contains_key(path)
    }

    /// Pin timestamp for a project, if pinned.
    fn project_pin_timestamp(&self, path: &str) -> Option<i64> {
        self.settings.pinned_projects.get(path).copied()
    }

    /// Partition `filtered_skills` into pinned-first + rest, sorted
    /// within the pinned block by pin timestamp ASC (oldest pin at
    /// the top, newest pin just above the unpinned block). Mirrors
    /// the behavior of `build_idle_rows` for session pins.
    fn apply_pinned_skill_sort(&mut self) {
        let mut pinned: Vec<(i64, Skill)> = Vec::with_capacity(self.filtered_skills.len());
        let mut rest: Vec<Skill> = Vec::with_capacity(self.filtered_skills.len());
        for skill in std::mem::take(&mut self.filtered_skills) {
            match self.skill_pin_timestamp(&skill.name) {
                Some(ts) => pinned.push((ts, skill)),
                None => rest.push(skill),
            }
        }
        pinned.sort_by_key(|(ts, _)| *ts);
        let mut out: Vec<Skill> =
            pinned.into_iter().map(|(_, s)| s).collect();
        out.extend(rest);
        self.filtered_skills = out;
    }

    /// Same partition/sort as `apply_pinned_skill_sort`, but against
    /// `filtered_projects`. Also promotes the built-in `~/.slashpad`
    /// project (flagged via `is_default`) to the top of the unpinned
    /// block so it always sits flush with the pinned block above —
    /// no other unpinned rows can slot between the last pin and the
    /// default.
    fn apply_pinned_project_sort(&mut self) {
        let mut pinned: Vec<(i64, crate::projects::ProjectInfo)> =
            Vec::with_capacity(self.filtered_projects.len());
        let mut rest: Vec<crate::projects::ProjectInfo> =
            Vec::with_capacity(self.filtered_projects.len());
        for project in std::mem::take(&mut self.filtered_projects) {
            let key = project.path.to_string_lossy().to_string();
            match self.project_pin_timestamp(&key) {
                Some(ts) => pinned.push((ts, project)),
                None => rest.push(project),
            }
        }
        pinned.sort_by_key(|(ts, _)| *ts);
        // If the default lives in `rest`, promote it to position 0
        // so it always anchors the unpinned block. If it's in
        // `pinned`, leave it to sort normally by its pin timestamp.
        if let Some(pos) = rest.iter().position(|p| p.is_default) {
            if pos != 0 {
                let default = rest.remove(pos);
                rest.insert(0, default);
            }
        }
        let mut out: Vec<crate::projects::ProjectInfo> =
            pinned.into_iter().map(|(_, p)| p).collect();
        out.extend(rest);
        self.filtered_projects = out;
    }

    /// Toggle the pinned state of the currently-selected skill row.
    /// Persists to `settings.json` and rebuilds `filtered_skills` so
    /// the list re-sorts immediately.
    fn toggle_pin_selected_skill(&mut self) -> Task<Message> {
        let Some(skill) = self.filtered_skills.get(self.selected_skill_index).cloned() else {
            self.session_menu_open = false;
            self.session_menu_selected = 0;
            return Task::none();
        };
        let was_pinned = self.skill_is_pinned(&skill.name);
        if was_pinned {
            self.settings.pinned_skills.remove(&skill.name);
        } else {
            self.settings
                .pinned_skills
                .insert(skill.name.clone(), now_unix_millis());
        }
        if let Err(e) = self.settings.save() {
            eprintln!("[slashpad] failed to save pinned_skills: {e}");
        }
        self.apply_pinned_skill_sort();
        // Track the skill across the reorder so the selection follows
        // it to its new position — same UX as session pinning.
        if let Some(new_idx) = self
            .filtered_skills
            .iter()
            .position(|s| s.name == skill.name)
        {
            self.selected_skill_index = new_idx;
        }
        self.session_menu_open = false;
        self.session_menu_selected = 0;
        snap_to_selection(
            SKILL_LIST_SCROLL_ID.clone(),
            self.selected_skill_index,
            self.filtered_skills.len(),
        )
    }

    /// Toggle the pinned state of the currently-selected project row.
    fn toggle_pin_selected_project(&mut self) -> Task<Message> {
        let Some(project) = self.filtered_projects.get(self.selected_project_index).cloned()
        else {
            self.session_menu_open = false;
            self.session_menu_selected = 0;
            return Task::none();
        };
        let key = project.path.to_string_lossy().to_string();
        let was_pinned = self.project_is_pinned(&key);
        if was_pinned {
            self.settings.pinned_projects.remove(&key);
        } else {
            self.settings
                .pinned_projects
                .insert(key.clone(), now_unix_millis());
        }
        if let Err(e) = self.settings.save() {
            eprintln!("[slashpad] failed to save pinned_projects: {e}");
        }
        self.apply_pinned_project_sort();
        if let Some(new_idx) = self
            .filtered_projects
            .iter()
            .position(|p| p.path == project.path)
        {
            self.selected_project_index = new_idx;
        }
        self.session_menu_open = false;
        self.session_menu_selected = 0;
        snap_to_selection(
            PROJECT_PICKER_SCROLL_ID.clone(),
            self.selected_project_index,
            self.filtered_projects.len(),
        )
    }

    /// Reorder within the pinned skill block by swapping pin
    /// timestamps with the neighbor in the given direction. Mirrors
    /// the shape of `move_pinned_row` for sessions; the only
    /// differences are the storage (settings map vs SDK tag) and the
    /// persistence call (settings.save vs tag_session).
    fn move_pinned_skill_row(&mut self, direction: i32) -> Task<Message> {
        if self.mode != Mode::Skills
            || self.session_menu_open
            || self.filtered_skills.is_empty()
        {
            return Task::none();
        }
        let Some(selected) = self.filtered_skills.get(self.selected_skill_index).cloned()
        else {
            return Task::none();
        };
        if !self.skill_is_pinned(&selected.name) {
            return Task::none();
        }
        // Contiguous pinned prefix of the filtered list.
        let mut pinned_rows: Vec<(usize, String)> = Vec::new();
        for (i, s) in self.filtered_skills.iter().enumerate() {
            if !self.skill_is_pinned(&s.name) {
                break;
            }
            pinned_rows.push((i, s.name.clone()));
        }
        let Some(my_pos) = pinned_rows.iter().position(|(_, n)| n == &selected.name) else {
            return Task::none();
        };
        let target = my_pos as i32 + direction;
        if target < 0 || target >= pinned_rows.len() as i32 {
            return Task::none();
        }
        let (neighbor_row_idx, neighbor_name) = pinned_rows[target as usize].clone();

        let my_ts = self.skill_pin_timestamp(&selected.name).unwrap_or(0);
        let neighbor_ts = self.skill_pin_timestamp(&neighbor_name).unwrap_or(0);
        if my_ts == neighbor_ts {
            let bumped = match direction {
                d if d < 0 => neighbor_ts.saturating_sub(1),
                _ => neighbor_ts.saturating_add(1),
            };
            self.settings
                .pinned_skills
                .insert(selected.name.clone(), bumped);
        } else {
            self.settings
                .pinned_skills
                .insert(selected.name.clone(), neighbor_ts);
            self.settings
                .pinned_skills
                .insert(neighbor_name.clone(), my_ts);
        }
        if let Err(e) = self.settings.save() {
            eprintln!("[slashpad] failed to save pinned_skills: {e}");
        }
        self.apply_pinned_skill_sort();
        self.selected_skill_index = neighbor_row_idx;
        snap_to_selection(
            SKILL_LIST_SCROLL_ID.clone(),
            self.selected_skill_index,
            self.filtered_skills.len(),
        )
    }

    /// Reorder within the pinned project block. Same semantics as
    /// `move_pinned_skill_row`.
    fn move_pinned_project_row(&mut self, direction: i32) -> Task<Message> {
        if self.mode != Mode::ProjectPicker
            || self.session_menu_open
            || self.filtered_projects.is_empty()
        {
            return Task::none();
        }
        let Some(selected) = self.filtered_projects.get(self.selected_project_index).cloned()
        else {
            return Task::none();
        };
        let selected_key = selected.path.to_string_lossy().to_string();
        if !self.project_is_pinned(&selected_key) {
            return Task::none();
        }
        let mut pinned_rows: Vec<(usize, String)> = Vec::new();
        for (i, p) in self.filtered_projects.iter().enumerate() {
            let key = p.path.to_string_lossy().to_string();
            if !self.project_is_pinned(&key) {
                break;
            }
            pinned_rows.push((i, key));
        }
        let Some(my_pos) = pinned_rows.iter().position(|(_, k)| k == &selected_key) else {
            return Task::none();
        };
        let target = my_pos as i32 + direction;
        if target < 0 || target >= pinned_rows.len() as i32 {
            return Task::none();
        }
        let (neighbor_row_idx, neighbor_key) = pinned_rows[target as usize].clone();

        let my_ts = self.project_pin_timestamp(&selected_key).unwrap_or(0);
        let neighbor_ts = self.project_pin_timestamp(&neighbor_key).unwrap_or(0);
        if my_ts == neighbor_ts {
            let bumped = match direction {
                d if d < 0 => neighbor_ts.saturating_sub(1),
                _ => neighbor_ts.saturating_add(1),
            };
            self.settings
                .pinned_projects
                .insert(selected_key.clone(), bumped);
        } else {
            self.settings
                .pinned_projects
                .insert(selected_key.clone(), neighbor_ts);
            self.settings
                .pinned_projects
                .insert(neighbor_key.clone(), my_ts);
        }
        if let Err(e) = self.settings.save() {
            eprintln!("[slashpad] failed to save pinned_projects: {e}");
        }
        self.apply_pinned_project_sort();
        self.selected_project_index = neighbor_row_idx;
        snap_to_selection(
            PROJECT_PICKER_SCROLL_ID.clone(),
            self.selected_project_index,
            self.filtered_projects.len(),
        )
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
    ///
    /// Pinned rows float to the top. Within the pinned block, order
    /// is oldest-pin-first / newest-pin-last (so the item the user
    /// just pinned lands at the bottom of the pinned group). Unpinned
    /// rows keep their original order (active chats first, then past
    /// sessions by recent activity).
    fn build_idle_rows(&self) -> Vec<IdleRowSelection> {
        let capacity = self.idle_row_count();
        let mut pinned: Vec<(i64, IdleRowSelection)> = Vec::with_capacity(capacity);
        let mut rest: Vec<IdleRowSelection> = Vec::with_capacity(capacity);
        for id in self.visible_active_chat_ids() {
            let sid = self
                .chats
                .get(&id)
                .and_then(|e| e.state.session_id.as_deref())
                .map(|s| s.to_string());
            let pin_ts = sid.as_deref().and_then(|s| self.session_pin_timestamp(s));
            let is_pin = sid
                .as_deref()
                .map(|s| self.session_is_pinned(s))
                .unwrap_or(false);
            if is_pin {
                pinned.push((pin_ts.unwrap_or(0), IdleRowSelection::Active(id)));
            } else {
                rest.push(IdleRowSelection::Active(id));
            }
        }
        for session in self.visible_past_session_rows() {
            if is_pinned(session.tag.as_deref()) {
                let ts = pin_timestamp(session.tag.as_deref()).unwrap_or(0);
                pinned.push((ts, IdleRowSelection::Past(session)));
            } else {
                rest.push(IdleRowSelection::Past(session));
            }
        }
        pinned.sort_by_key(|(ts, _)| *ts);
        let mut out: Vec<IdleRowSelection> = pinned.into_iter().map(|(_, r)| r).collect();
        out.extend(rest);
        out
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
        self.all_skills = skills::load_skills(
            Some(&self.project_path),
            self.settings.load_user_settings,
        )
        .unwrap_or_default();

        // Decide what view to restore. The default is "show whatever
        // the user was last looking at" (Chatting, Idle, or Skills with
        // preserved input). The reset flag overrides this and forces
        // the Skills picker with `/` prefilled — set by `hide_palette`
        // when the user dismissed while in Skills mode, or when a
        // skill was fire-and-forgotten.
        let force_reset = self.reset_to_skills_on_next_open
            || matches!(self.mode, Mode::Settings | Mode::ProjectPicker)
            || (self.mode == Mode::Chatting
                && self
                    .active_chat_id
                    .map(|id| !self.chats.contains_key(&id))
                    .unwrap_or(true));
        self.reset_to_skills_on_next_open = false;

        if force_reset {
            self.mode = Mode::Skills;
            self.input = "/".to_string();
            self.filtered_skills = self.all_skills.clone();
            self.apply_pinned_skill_sort();
            self.selected_skill_index = 0;
        }
        // Otherwise leave `self.mode`, `self.input`, `self.active_chat_id`,
        // and selections alone — the last view is restored as-is.

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
        // Per-screen drag memory: if the user has previously dragged
        // the palette on the screen where the cursor currently is,
        // reuse that position. Otherwise cursor-center on that screen.
        // Raycast-style: dragging on monitor A doesn't affect monitor
        // B's position.
        #[cfg(target_os = "macos")]
        {
            let cursor_key = crate::platform::macos::cursor_screen_key();
            let remembered = cursor_key.and_then(|k| self.dragged_positions.get(&k).copied());
            if let Some(point) = remembered {
                self.programmatic_move_pending = true;
                tasks.push(iced::window::move_to(id, point));
            } else if let Some((x, y)) =
                crate::platform::macos::cursor_palette_position(Self::LAUNCHER_W as f64)
            {
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
        // If the user was browsing skills at dismiss time (input starts
        // with `/`, skill list showing), next open should re-prompt
        // with `/` rather than restoring the half-typed filter. Also
        // catches the fire-and-forget-skill path: that handler invokes
        // from Skills mode and calls into `hide_palette()` at the end.
        if self.mode == Mode::Skills {
            self.reset_to_skills_on_next_open = true;
        }
        // Drop any dangling rename state so the row isn't still in edit
        // mode the next time the palette is summoned.
        self.renaming_session_id = None;
        self.rename_input.clear();
        self.session_menu_open = false;
        self.session_menu_selected = 0;
        self.skill_delete_confirm = false;
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

        // Kick off an update check every time settings opens — unless
        // an upgrade is already in progress.
        if !matches!(self.update_status, UpdateStatus::Upgrading) {
            self.update_status = UpdateStatus::Checking;
            let tx = external_sender();
            tokio::spawn(async move {
                let result = crate::updates::check_for_update().await;
                let _ = tx.send(External::UpdateAvailable(result));
            });
        }

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

/// Stable iced text_input ID used for focusing the inline rename input
/// when a session row is in edit mode.
pub(crate) static RENAME_INPUT_ID: std::sync::LazyLock<text_input::Id> =
    std::sync::LazyLock::new(|| text_input::Id::new("slashpad-rename-input"));
