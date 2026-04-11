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

use std::sync::{Mutex, OnceLock};

use iced::futures::{SinkExt, Stream};
use iced::widget::{column, container, text_input, Column};
use iced::{Element, Subscription, Task, Theme};
use tokio::sync::mpsc;

use crate::hotkey;
use crate::settings::AppSettings;
use crate::sidecar::{self, FollowUp, Payload, SidecarEvent, SpawnedSidecar};
use crate::skills;
use crate::state::{ChatMessageView, ContentBlock, MessageStatus, Mode, Role, SessionInfo, Skill};
use crate::ui;

/// External events that are produced off the iced thread (hotkey thread, sidecar
/// tasks) and need to be pumped into the iced event loop via a subscription.
#[derive(Debug)]
pub enum External {
    HotkeyPressed,
    Sidecar(SidecarEvent),
    /// Background-loaded list of recent sessions.
    RecentSessions(Vec<SessionInfo>),
    /// Background-loaded history messages for a resumed session.
    HistoryLoaded(Vec<ChatMessageView>),
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
    SelectSession(usize),
    HotkeyPressed,
    /// An iced window lost focus. Carries the window id so we can dispatch
    /// palette-vs-settings blur handling separately.
    WindowBlurred(iced::window::Id),
    /// An iced window closed. Fired by the `close_events` subscription so
    /// we can null out `palette_window_id` / `settings_window_id`.
    WindowClosed(iced::window::Id),
    SidecarEvent(SidecarEvent),
    RecentSessionsLoaded(Vec<SessionInfo>),
    HistoryLoaded(Vec<ChatMessageView>),
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

    pub messages: Vec<ChatMessageView>,
    pub is_agent_ready: bool,
    pub session_id: Option<String>,
    pub current_assistant_id: Option<u64>,
    pub next_msg_id: u64,

    pub recent_sessions: Vec<SessionInfo>,
    pub selected_session_index: usize,

    pub settings: AppSettings,
    pub api_key_input: String,
    pub recording_hotkey: bool,

    pub sidecar: Option<SpawnedSidecar>,

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
            messages: Vec::new(),
            is_agent_ready: false,
            session_id: None,
            current_assistant_id: None,
            next_msg_id: 1,
            recent_sessions: Vec::new(),
            selected_session_index: 0,
            settings,
            api_key_input: String::new(),
            recording_hotkey: false,
            sidecar: None,
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

    pub fn subscription(&self) -> Subscription<Message> {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;
        Subscription::batch([
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
        ])
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
                // Settings window takes precedence: close it if it's open.
                // The on_key_press subscription fires regardless of which
                // iced window is focused, so this handles both "Esc while
                // settings is focused" and "Esc while palette is focused
                // with settings still open somewhere".
                if let Some(settings_id) = self.settings_window_id {
                    return iced::window::close(settings_id);
                }
                match self.mode {
                    Mode::Chatting => {
                        self.kill_session();
                        self.mode = Mode::Idle;
                        self.messages.clear();
                        self.input.clear();
                        self.is_agent_ready = false;
                        self.session_id = None;
                        self.current_assistant_id = None;
                        Task::batch([
                            self.refresh_sessions(),
                            self.resize_task(),
                            text_input::focus(INPUT_ID.clone()),
                        ])
                    }
                    _ => self.hide_palette(),
                }
            }

            Message::NavUp => {
                match self.mode {
                    Mode::Skills => {
                        if self.selected_skill_index > 0 {
                            self.selected_skill_index -= 1;
                        }
                    }
                    Mode::Idle if self.input.is_empty() && !self.recent_sessions.is_empty() => {
                        if self.selected_session_index > 0 {
                            self.selected_session_index -= 1;
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
                    Mode::Idle if self.input.is_empty() && !self.recent_sessions.is_empty() => {
                        let max = self.recent_sessions.len().saturating_sub(1);
                        if self.selected_session_index < max {
                            self.selected_session_index += 1;
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

            Message::SelectSession(i) => {
                self.selected_session_index = i;
                if let Some(session) = self.recent_sessions.get(i).cloned() {
                    self.resume_session(session)
                } else {
                    Task::none()
                }
            }

            Message::HotkeyPressed => self.toggle_palette(),

            Message::SidecarEvent(event) => {
                self.process_sidecar_event(event);
                Task::none()
            }

            Message::RecentSessionsLoaded(sessions) => {
                self.recent_sessions = sessions;
                if self.selected_session_index >= self.recent_sessions.len() {
                    self.selected_session_index = 0;
                }
                // Height may change if we're idle with empty input — the
                // session list just became populated.
                self.resize_task()
            }

            Message::HistoryLoaded(messages) => {
                self.messages = messages;
                self.is_agent_ready = true;
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
                } else if Some(window_id) == self.palette_window_id {
                    // Palette lost focus → hide (unless chatting; agent
                    // tools can steal focus by opening browser tabs).
                    if self.mode != Mode::Chatting && self.palette_visible {
                        self.hide_palette()
                    } else {
                        Task::none()
                    }
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
                // Drop the sidecar (tokio kill_on_drop handles child cleanup)
                // and exit the process. Bypasses iced's graceful shutdown
                // because that path is fiddly with our NSPanel wrapping.
                self.sidecar = None;
                std::process::exit(0)
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
            ))
            .padding(8)
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into();
        }

        // Palette window: command input + mode-dependent content below.
        let input = ui::command_input::view(&self.input, self.mode, self.is_agent_ready);
        let mut stack: Column<'_, Message> = column![input].spacing(4);

        match self.mode {
            Mode::Skills => {
                stack = stack.push(ui::skill_list::view(
                    &self.filtered_skills,
                    self.selected_skill_index,
                ));
            }
            Mode::Idle if self.input.is_empty() && !self.recent_sessions.is_empty() => {
                stack = stack.push(ui::session_list::view(
                    &self.recent_sessions,
                    self.selected_session_index,
                ));
            }
            Mode::Chatting => {
                stack = stack.push(ui::chat_panel::view(&self.messages, self.is_agent_ready));
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
            Mode::Chatting if self.is_agent_ready && !self.input.trim().is_empty() => {
                let content = self.input.trim().to_string();
                self.send_follow_up(content)
            }
            Mode::Skills if !self.filtered_skills.is_empty() => {
                if let Some(skill) = self.filtered_skills.get(self.selected_skill_index).cloned() {
                    self.start_chat(format!("/{}", skill.name))
                } else {
                    Task::none()
                }
            }
            Mode::Idle if self.input.is_empty() && !self.recent_sessions.is_empty() => {
                if let Some(session) = self
                    .recent_sessions
                    .get(self.selected_session_index)
                    .cloned()
                {
                    self.resume_session(session)
                } else {
                    Task::none()
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
        self.mode = Mode::Chatting;
        self.is_agent_ready = false;
        self.session_id = None;
        self.current_assistant_id = None;

        let user_id = self.alloc_id();
        let user_msg = ChatMessageView::user(user_id, prompt.clone());
        self.messages.clear();
        self.messages.push(user_msg);
        self.input.clear();

        if let Err(e) = self.spawn_sidecar_chat(prompt, None) {
            self.push_error(format!("Failed to start agent: {e}"));
        }

        Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())])
    }

    fn resume_session(&mut self, info: SessionInfo) -> Task<Message> {
        self.kill_session();
        self.mode = Mode::Chatting;
        self.is_agent_ready = true;
        self.session_id = Some(info.session_id.clone());
        self.input.clear();
        self.current_assistant_id = None;
        self.messages.clear();

        // Load session history in the background via a one-shot "messages" sidecar.
        let session_id = info.session_id.clone();
        let tx = external_sender();
        tokio::spawn(async move {
            let msgs = crate::sessions::load_messages(&session_id)
                .await
                .unwrap_or_default();
            let _ = tx.send(External::HistoryLoaded(msgs));
        });

        Task::batch([self.resize_task(), text_input::focus(INPUT_ID.clone())])
    }

    fn send_follow_up(&mut self, content: String) -> Task<Message> {
        let user_id = self.alloc_id();
        self.messages
            .push(ChatMessageView::user(user_id, content.clone()));
        self.input.clear();
        self.is_agent_ready = false;
        self.current_assistant_id = None;

        if let Some(sidecar) = self.sidecar.as_ref() {
            let _ = sidecar.follow_up_tx.send(FollowUp::Message(content));
        } else if let Some(resume_id) = self.session_id.clone() {
            // Resumed from disk — spawn a fresh sidecar with the resume ID.
            if let Err(e) = self.spawn_sidecar_chat(content, Some(resume_id)) {
                self.push_error(format!("Failed to restart agent: {e}"));
            }
        }
        Task::none()
    }

    fn spawn_sidecar_chat(&mut self, prompt: String, resume: Option<String>) -> anyhow::Result<()> {
        let home = sidecar::launchpad_home()?;
        let payload = Payload::chat(
            prompt,
            home.to_string_lossy().to_string(),
            self.settings.api_key.clone(),
            resume,
        );
        let mut spawned = sidecar::spawn(payload)?;

        // Forward sidecar events into the external bus.
        let tx = external_sender();
        let mut rx = std::mem::replace(&mut spawned.event_rx, mpsc::unbounded_channel().1);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if tx.send(External::Sidecar(event)).is_err() {
                    break;
                }
            }
        });

        self.sidecar = Some(spawned);
        Ok(())
    }

    fn kill_session(&mut self) {
        if let Some(sidecar) = self.sidecar.take() {
            let _ = sidecar.follow_up_tx.send(FollowUp::Close);
            // tokio Child has kill_on_drop; dropping the struct kills the process.
            drop(sidecar);
        }
    }

    fn process_sidecar_event(&mut self, event: SidecarEvent) {
        match event {
            SidecarEvent::Ready { .. } => {
                self.is_agent_ready = true;
            }
            SidecarEvent::SessionId { session_id, .. } => {
                self.session_id = Some(session_id);
            }
            SidecarEvent::TextDelta { delta, .. } => {
                self.ensure_streaming_assistant();
                if let Some(msg) = self.current_assistant_mut() {
                    match msg.blocks.last_mut() {
                        Some(ContentBlock::Text(buf)) => buf.push_str(&delta),
                        _ => msg.blocks.push(ContentBlock::Text(delta)),
                    }
                }
            }
            SidecarEvent::ToolStart { tool, args, .. } => {
                self.ensure_streaming_assistant();
                let args = args.unwrap_or_default();
                if let Some(msg) = self.current_assistant_mut() {
                    msg.blocks.push(ContentBlock::ToolStart { tool, args });
                }
            }
            SidecarEvent::ToolEnd {
                tool, args, result, ..
            } => {
                self.ensure_streaming_assistant();
                let args = args.unwrap_or_default();
                if let Some(msg) = self.current_assistant_mut() {
                    msg.blocks
                        .push(ContentBlock::ToolEnd { tool, args, result });
                }
            }
            SidecarEvent::Error { error, .. } => {
                let err = error.unwrap_or_else(|| "An error occurred".to_string());
                if let Some(msg) = self.current_assistant_mut() {
                    msg.status = MessageStatus::Error;
                    msg.blocks.push(ContentBlock::Error(err));
                } else {
                    self.push_error(err);
                }
            }
            SidecarEvent::Complete { .. } => {
                if let Some(msg) = self.current_assistant_mut() {
                    msg.status = MessageStatus::Complete;
                }
                self.current_assistant_id = None;
            }
            SidecarEvent::Session { .. } | SidecarEvent::ChatMessage { .. } => {
                // These arrive from list/messages modes and are consumed by
                // sessions.rs directly, not through this stream.
            }
        }
    }

    fn ensure_streaming_assistant(&mut self) {
        let needs_new = match self.messages.last() {
            Some(m) => m.role != Role::Assistant || m.status != MessageStatus::Streaming,
            None => true,
        };
        if needs_new {
            let id = self.alloc_id();
            self.current_assistant_id = Some(id);
            self.messages.push(ChatMessageView::assistant_streaming(id));
        }
    }

    fn current_assistant_mut(&mut self) -> Option<&mut ChatMessageView> {
        let id = self.current_assistant_id?;
        self.messages.iter_mut().find(|m| m.id == id)
    }

    fn push_error(&mut self, error: String) {
        let id = self.alloc_id();
        self.current_assistant_id = Some(id);
        let mut msg = ChatMessageView::assistant_streaming(id);
        msg.status = MessageStatus::Error;
        msg.blocks.push(ContentBlock::Error(error));
        self.messages.push(msg);
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_msg_id;
        self.next_msg_id += 1;
        id
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
                if self.input.is_empty() && !self.recent_sessions.is_empty() {
                    let n = self.recent_sessions.len().max(1) as f32;
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
        // Input is prefilled with "/", so we land directly in Skills mode —
        // the view only renders the skill list when `mode == Skills`.
        self.mode = Mode::Skills;
        self.input = "/".to_string();
        self.filtered_skills = self.all_skills.clone();
        self.selected_skill_index = 0;
        self.selected_session_index = 0;
        self.messages.clear();
        self.is_agent_ready = false;
        self.session_id = None;
        self.current_assistant_id = None;

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
                External::Sidecar(ev) => Message::SidecarEvent(ev),
                External::RecentSessions(s) => Message::RecentSessionsLoaded(s),
                External::HistoryLoaded(m) => Message::HistoryLoaded(m),
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
