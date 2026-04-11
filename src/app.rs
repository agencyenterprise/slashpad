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
use std::sync::{Mutex, OnceLock};

use iced::futures::{SinkExt, Stream};
use iced::widget::{column, container, text_input, Column};
use iced::{Element, Subscription, Task, Theme};
use tokio::sync::mpsc;

use crate::hotkey;
use crate::settings::AppSettings;
use crate::sidecar::{self, FollowUp, Payload, SidecarEvent, SpawnedSidecar};
use crate::skills;
use crate::state::{
    ChatId, ChatMessageView, ChatState, ChatStatus, Mode, SessionInfo, Skill,
};
use crate::ui;

/// A single running (or resumed) chat: its logical state plus, if the
/// sidecar is alive, the process handle. `sidecar` is `None` for a chat
/// that was just resumed-from-disk and hasn't had a follow-up sent yet.
pub struct ChatEntry {
    pub state: ChatState,
    pub sidecar: Option<SpawnedSidecar>,
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
    /// Background-loaded history messages for a resumed session.
    /// Tagged with the `chat_id` of the entry the history belongs to —
    /// multiple resumes can be in flight concurrently.
    HistoryLoaded {
        chat_id: ChatId,
        messages: Vec<ChatMessageView>,
    },
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
    /// Tray context-menu "Quit Launchpad" — graceful shutdown.
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
    EscapePressed,
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
    HistoryLoaded {
        chat_id: ChatId,
        messages: Vec<ChatMessageView>,
    },
    /// Close the settings window (bound to the "esc" button in the
    /// settings panel header).
    CloseSettings,
    ApiKeyInputChanged(String),
    SaveApiKey,
    /// Tray left-click → open a new settings window anchored below the
    /// tray icon at the given logical-pixel rect.
    TrayOpenSettings {
        tray_x: f64,
        tray_y: f64,
        tray_w: f64,
        tray_h: f64,
    },
    /// Tray menu "Quit Launchpad" → drop sidecar, exit process.
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
    /// A user clicked a link inside a rendered markdown message.
    /// Currently a no-op — we just log it. Plumbed because
    /// `iced::widget::markdown::view` returns an `Element<Url>` and
    /// we need a `Message` to map into.
    MarkdownLinkClicked(iced::widget::markdown::Url),
}

/// Root application state.
pub struct Launchpad {
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
    /// sessions — used for up/down nav in Mode::Idle with empty input.
    pub selected_idle_index: usize,

    pub settings: AppSettings,
    pub api_key_input: String,
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
}

impl Launchpad {
    pub fn new() -> (Self, Task<Message>) {
        // `init_external_bus()` is called from `main()` before iced starts,
        // so `external_sender()` is ready for the hotkey forwarder below.

        let settings = AppSettings::load_or_default();
        let all_skills = skills::load_skills().unwrap_or_default();

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
            Err(e) => eprintln!("[launchpad] failed to register hotkey: {e}"),
        }

        // Kick off a background load of recent sessions for the idle view.
        let tx = external_sender();
        tokio::spawn(async move {
            let sessions = crate::sessions::list_recent().await.unwrap_or_default();
            let _ = tx.send(External::RecentSessions(sessions));
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
            settings,
            api_key_input: String::new(),
            recording_hotkey: false,
            hotkey_error: None,
            palette_window_id: Some(palette_id),
            settings_window_id: None,
            settings_opened_at: None,
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

        // Palette NSPanel treatment: swap the class to `LaunchpadPanel`,
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
            "Launchpad Settings".to_string()
        } else {
            "Launchpad".to_string()
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
            iced::keyboard::on_key_press(|key, _modifiers| match key.as_ref() {
                Key::Named(Named::ArrowUp) => Some(Message::NavUp),
                Key::Named(Named::ArrowDown) => Some(Message::NavDown),
                _ => None,
            }),
            // Escape must use listen_with (not on_key_press) because iced's
            // text_input widget captures Escape when focused — it clears its
            // own focus and returns Status::Captured, which hides the event
            // from on_key_press. listen_with receives events regardless of
            // capture status, so we see the first press.
            iced::event::listen_with(|event, _status, window_id| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key, ..
                }) => match key.as_ref() {
                    Key::Named(Named::Escape) => Some(Message::EscapePressed),
                    _ => None,
                },
                iced::Event::Window(iced::window::Event::Unfocused) => {
                    Some(Message::WindowBlurred(window_id))
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

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(value) => {
                self.input = value.clone();

                // Skill filtering
                if self.mode == Mode::Chatting {
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
                } else if self.mode == Mode::Skills {
                    self.mode = Mode::Idle;
                    self.filtered_skills.clear();
                }
                self.resize_task()
            }

            Message::Submit => self.handle_submit(),

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
                // Esc always dismisses the palette now. In Chatting mode
                // it *preserves* the chat — the sidecar keeps streaming
                // in the background and the entry stays in `self.chats`
                // so the user can return to it via the idle list next
                // time they summon the palette.
                if self.mode == Mode::Chatting {
                    self.active_chat_id = None;
                    self.mode = Mode::Idle;
                    self.input.clear();
                }
                self.hide_palette()
            }

            Message::NavUp => {
                match self.mode {
                    Mode::Skills => {
                        if self.selected_skill_index > 0 {
                            self.selected_skill_index -= 1;
                        }
                    }
                    Mode::Idle if self.input.is_empty() && self.idle_row_count() > 0 => {
                        if self.selected_idle_index > 0 {
                            self.selected_idle_index -= 1;
                        }
                    }
                    _ => {}
                }
                Task::none()
            }

            Message::NavDown => {
                match self.mode {
                    Mode::Skills => {
                        let max = self.filtered_skills.len().saturating_sub(1);
                        if self.selected_skill_index < max {
                            self.selected_skill_index += 1;
                        }
                    }
                    Mode::Idle if self.input.is_empty() && self.idle_row_count() > 0 => {
                        let max = self.idle_row_count().saturating_sub(1);
                        if self.selected_idle_index < max {
                            self.selected_idle_index += 1;
                        }
                    }
                    _ => {}
                }
                Task::none()
            }

            Message::SelectSkill(i) => {
                self.selected_skill_index = i;
                self.handle_submit()
            }

            Message::SelectSession(session_index) => {
                // `session_index` is an index into the *past sessions*
                // portion of the idle list (after filtering out dupes
                // of active chats). The caller already passes the
                // correct filtered index from the view builder.
                let past = self.past_session_rows();
                if let Some(session) = past.get(session_index).cloned() {
                    self.resume_session(session)
                } else {
                    Task::none()
                }
            }

            Message::SelectChat(chat_id) => {
                if self.chats.contains_key(&chat_id) {
                    self.active_chat_id = Some(chat_id);
                    self.mode = Mode::Chatting;
                    self.input.clear();
                    Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())])
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
                // Height may change if we're idle with empty input — the
                // session list just became populated.
                self.resize_task()
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
                Task::none()
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
                    // Palette lost focus → hide. Chats keep streaming
                    // in the background; if a Claude tool call opens a
                    // browser tab, the palette just hides and the
                    // user re-summons to see results — strictly better
                    // than the pre-multi-chat behavior, which left a
                    // blurred palette hovering mid-interaction.
                    if self.mode == Mode::Chatting {
                        self.active_chat_id = None;
                        self.mode = Mode::Idle;
                        self.input.clear();
                    }
                    self.hide_palette()
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

            Message::ApiKeyInputChanged(v) => {
                self.api_key_input = v;
                Task::none()
            }

            Message::SaveApiKey => {
                if self.api_key_input.starts_with("sk-ant-") {
                    self.settings.api_key = Some(self.api_key_input.clone());
                    let _ = self.settings.save();
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
                    eprintln!("[launchpad] failed to open link {url}: {e}");
                }
                Task::none()
            }
        }
    }

    pub fn view(&self, window_id: iced::window::Id) -> Element<'_, Message> {
        // Settings window: always shows the settings panel.
        if Some(window_id) == self.settings_window_id {
            return container(ui::settings::view(
                &self.api_key_input,
                &self.settings.hotkey,
                self.recording_hotkey,
                self.hotkey_error.as_deref(),
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
        let mut stack: Column<'_, Message> = column![input].spacing(4);

        match self.mode {
            Mode::Skills => {
                stack = stack.push(ui::skill_list::view(
                    &self.filtered_skills,
                    self.selected_skill_index,
                ));
            }
            Mode::Idle if self.input.is_empty() && self.idle_row_count() > 0 => {
                // Build view-layer rows that borrow from `self`. Past
                // sessions are filtered to exclude session_ids already
                // represented by an active chat.
                let active_session_ids: std::collections::HashSet<&str> = self
                    .chats
                    .values()
                    .filter_map(|c| c.state.session_id.as_deref())
                    .collect();
                let mut rows: Vec<ui::idle_list::IdleRow<'_>> =
                    Vec::with_capacity(self.idle_row_count());
                for entry in self.chats.values() {
                    rows.push(ui::idle_list::IdleRow::Active(entry));
                }
                for session in &self.recent_sessions {
                    if !active_session_ids.contains(session.session_id.as_str()) {
                        rows.push(ui::idle_list::IdleRow::Past(session));
                    }
                }
                stack = stack.push(ui::idle_list::view(
                    &rows,
                    self.selected_idle_index,
                    self.spinner_frame,
                ));
            }
            Mode::Chatting => {
                if let Some(entry) = self.active_chat() {
                    let ready = matches!(entry.state.status, ChatStatus::Idle);
                    stack = stack.push(ui::chat_panel::view(
                        &entry.state.messages,
                        ready,
                        self.spinner_frame,
                    ));
                }
            }
            _ => {}
        }

        container(stack)
            .padding(8)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    // --- helpers ---

    fn handle_submit(&mut self) -> Task<Message> {
        match self.mode {
            Mode::Chatting if !self.input.trim().is_empty() => {
                // Only dispatch a follow-up if the active chat is
                // actually ready; otherwise silently drop — today's
                // `command_input` placeholder already reflects this
                // state so the user sees "Waiting for response...".
                let ready = self
                    .active_chat()
                    .map(|e| matches!(e.state.status, ChatStatus::Idle))
                    .unwrap_or(false);
                if ready {
                    let content = self.input.trim().to_string();
                    self.send_follow_up(content)
                } else {
                    Task::none()
                }
            }
            Mode::Skills if !self.filtered_skills.is_empty() => {
                if let Some(skill) = self.filtered_skills.get(self.selected_skill_index).cloned() {
                    self.start_chat(format!("/{}", skill.name))
                } else {
                    Task::none()
                }
            }
            Mode::Idle if self.input.is_empty() && self.idle_row_count() > 0 => {
                // Dispatch based on which row is selected in the
                // unified (active chats + past sessions) list.
                let rows = self.build_idle_rows();
                match rows.get(self.selected_idle_index) {
                    Some(IdleRowSelection::Active(chat_id)) => {
                        let cid = *chat_id;
                        // Enter the chat view without spawning anything.
                        if self.chats.contains_key(&cid) {
                            self.active_chat_id = Some(cid);
                            self.mode = Mode::Chatting;
                            self.input.clear();
                            Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())])
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
                    },
                );
                self.active_chat_id = Some(chat_id);
                self.mode = Mode::Chatting;
                self.input.clear();
                return Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())]);
            }
        };

        let state = ChatState::new(chat_id, &prompt);
        self.chats.insert(
            chat_id,
            ChatEntry {
                state,
                sidecar: Some(spawned),
            },
        );
        self.active_chat_id = Some(chat_id);
        self.mode = Mode::Chatting;
        self.input.clear();

        Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())])
    }

    fn resume_session(&mut self, info: SessionInfo) -> Task<Message> {
        // If we already have an active chat tracking this session id,
        // just switch to it instead of creating a duplicate entry.
        if let Some((&existing, _)) = self
            .chats
            .iter()
            .find(|(_, e)| e.state.session_id.as_deref() == Some(info.session_id.as_str()))
        {
            self.active_chat_id = Some(existing);
            self.mode = Mode::Chatting;
            self.input.clear();
            return Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())]);
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
            },
        );
        self.active_chat_id = Some(chat_id);
        self.mode = Mode::Chatting;
        self.input.clear();

        // Load session history in the background via a one-shot "messages" sidecar.
        let session_id = info.session_id.clone();
        let tx = external_sender();
        tokio::spawn(async move {
            let msgs = crate::sessions::load_messages(&session_id)
                .await
                .unwrap_or_default();
            let _ = tx.send(External::HistoryLoaded {
                chat_id,
                messages: msgs,
            });
        });

        Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())])
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
        // hand off to the sidecar.
        if let Some(entry) = self.chats.get_mut(&chat_id) {
            let user_id = entry.state.alloc_msg_id();
            entry
                .state
                .messages
                .push(ChatMessageView::user(user_id, content.clone()));
            entry.state.current_assistant_id = None;
            entry.state.status = ChatStatus::Streaming;
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
        Task::none()
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
        let home = sidecar::launchpad_home()?;
        let payload = Payload::chat(
            prompt,
            home.to_string_lossy().to_string(),
            self.settings.api_key.clone(),
            resume,
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
        let prev_status = entry.state.status;
        entry.state.apply_event(event);
        let status_changed = entry.state.status != prev_status;

        // If the palette is currently showing the idle list and a
        // background chat's status just flipped (e.g. Initializing →
        // Streaming or Streaming → Idle), we may need to resize: the
        // row content changed but the row count didn't. `resize_task`
        // is cheap, so just dispatch it whenever the status flipped
        // and the palette is visible in idle.
        if status_changed
            && self.palette_visible
            && self.mode == Mode::Idle
            && self.input.is_empty()
        {
            self.resize_task()
        } else {
            Task::none()
        }
    }

    fn process_sidecar_closed(&mut self, chat_id: ChatId) -> Task<Message> {
        let Some(entry) = self.chats.get_mut(&chat_id) else {
            return Task::none();
        };
        entry.sidecar = None;
        if !matches!(entry.state.status, ChatStatus::Idle | ChatStatus::Error) {
            entry.state.status = ChatStatus::Closed;
        }
        if self.palette_visible && self.mode == Mode::Idle && self.input.is_empty() {
            self.resize_task()
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
    /// session_id. Used by nav bounds and resize sizing.
    fn idle_row_count(&self) -> usize {
        self.chats.len() + self.past_session_rows().len()
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

    /// Build the unified idle-list selection list. Used by
    /// `handle_submit` to map `selected_idle_index` to either a
    /// ChatId or a past SessionInfo. The analogous `build_idle_rows`
    /// produces the view-layer rows with references.
    fn build_idle_rows(&self) -> Vec<IdleRowSelection> {
        let mut rows: Vec<IdleRowSelection> = Vec::with_capacity(self.idle_row_count());
        // Active chats first, in insertion order (BTreeMap iterates by id).
        for (&id, _) in self.chats.iter() {
            rows.push(IdleRowSelection::Active(id));
        }
        for session in self.past_session_rows() {
            rows.push(IdleRowSelection::Past(session));
        }
        rows
    }

    fn refresh_sessions(&self) -> Task<Message> {
        let tx = external_sender();
        tokio::spawn(async move {
            let sessions = crate::sessions::list_recent().await.unwrap_or_default();
            let _ = tx.send(External::RecentSessions(sessions));
        });
        Task::none()
    }

    /// Palette launcher width. Fixed; settings has its own window.
    const LAUNCHER_W: f32 = 720.0;

    /// Debounce window for the spurious `Unfocused` event that AppKit
    /// fires right after we open the settings window (our Accessory
    /// activation policy means the app deactivates the moment it
    /// activates). Blurs within this window are ignored; anything after
    /// is a genuine user-initiated click-outside and closes settings.
    const SETTINGS_BLUR_GRACE_MS: u64 = 300;

    /// Desired palette window height for the current mode + content state.
    /// Ported from the old `usePalette.ts` sizing heuristic.
    fn target_height(&self) -> f32 {
        const BASE: f32 = 90.0;
        const ROW: f32 = 52.0;
        const MAX_LIST: f32 = 260.0;
        const CHAT: f32 = 480.0;
        match self.mode {
            Mode::Chatting => BASE + CHAT,
            // Palette never enters Settings mode now (settings is a
            // separate window); return BASE as a safe fallback if it ever
            // somehow does.
            Mode::Settings => BASE,
            Mode::Skills => {
                let n = self.filtered_skills.len().max(1) as f32;
                BASE + (n * ROW).min(MAX_LIST)
            }
            Mode::Idle => {
                if self.input.is_empty() && self.idle_row_count() > 0 {
                    let n = self.idle_row_count().max(1) as f32;
                    BASE + (n * ROW).min(MAX_LIST)
                } else {
                    BASE
                }
            }
        }
    }

    /// Emit an `iced::window::resize` task to match the current target
    /// palette size, or `Task::none()` if the palette window id hasn't
    /// been created yet.
    fn resize_task(&self) -> Task<Message> {
        let Some(id) = self.palette_window_id else {
            return Task::none();
        };
        iced::window::resize(id, iced::Size::new(Self::LAUNCHER_W, self.target_height()))
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
        // When no chats exist, keep the pre-multi-chat behavior of
        // opening directly into the skills picker with "/" prefilled —
        // users who never leave a chat running see zero regression.
        // With chats present (running, completed, resumed-from-disk),
        // land in Idle with an empty input so the unified list of
        // active chats + past sessions is visible.
        if self.chats.is_empty() {
            self.mode = Mode::Skills;
            self.input = "/".to_string();
            self.filtered_skills = self.all_skills.clone();
            self.selected_skill_index = 0;
        } else {
            self.mode = Mode::Idle;
            self.input.clear();
            self.selected_idle_index = 0;
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
        #[cfg(target_os = "macos")]
        if let Some((x, y)) =
            crate::platform::macos::cursor_palette_position(Self::LAUNCHER_W as f64)
        {
            tasks.push(iced::window::move_to(
                id,
                iced::Point::new(x as f32, y as f32),
            ));
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
        // the class to `LaunchpadPanel` was crashing iced's close
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
    std::sync::LazyLock::new(|| text_input::Id::new("launchpad-command-input"));
