//! Iced application — state machine ported from `usePalette.ts`.

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
    PaletteBlurred,
    SidecarEvent(SidecarEvent),
    RecentSessionsLoaded(Vec<SessionInfo>),
    HistoryLoaded(Vec<ChatMessageView>),
    CloseSettings,
    ApiKeyInputChanged(String),
    SaveApiKey,
    /// Resolved once iced creates its initial window — we cache the id so
    /// later `window::resize` / `window::move_to` calls know which window to
    /// target.
    WindowIdResolved(Option<iced::window::Id>),
}

/// Root application state.
pub struct Launchpad {
    pub input: String,
    pub mode: Mode,
    pub visible: bool,

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

    /// Cached iced window id, resolved asynchronously on startup via
    /// `iced::window::get_oldest()`. `None` during the brief window between
    /// `Launchpad::new()` returning and iced processing the first frame.
    pub window_id: Option<iced::window::Id>,
}

impl Launchpad {
    pub fn new() -> (Self, Task<Message>) {
        init_external_bus();

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

        let state = Self {
            input: String::new(),
            mode: Mode::Idle,
            visible: false,
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
            window_id: None,
        };

        // Post-launch hook: apply NSPanel treatment to the palette window once
        // iced/winit has created it. We spawn a std thread that pings the main
        // thread via dispatch_async so the NSPanel ops run where AppKit expects.
        // The delay gives iced/winit enough time to create the NSWindow.
        #[cfg(target_os = "macos")]
        {
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                crate::platform::macos::dispatch_main_async(|| unsafe {
                    let ptr = crate::platform::macos::first_app_window_ptr();
                    crate::platform::macos::apply_palette_style(ptr);
                    crate::platform::macos::order_out(ptr);
                });
            });
        }

        // Resolve the iced window id so subsequent resize / move_to calls can
        // target it. The get_oldest() task resolves once iced has created its
        // first window (typically within the first frame).
        let init = Task::batch([
            text_input::focus(INPUT_ID.clone()),
            iced::window::get_oldest().map(Message::WindowIdResolved),
        ]);
        (state, init)
    }

    pub fn title(&self) -> String {
        "Launchpad".to_string()
    }

    pub fn theme(&self) -> Theme {
        ui::theme::dark_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;
        Subscription::batch([
            Subscription::run(external_subscription_stream),
            iced::keyboard::on_key_press(|key, _modifiers| match key.as_ref() {
                Key::Named(Named::Escape) => Some(Message::EscapePressed),
                Key::Named(Named::ArrowUp) => Some(Message::NavUp),
                Key::Named(Named::ArrowDown) => Some(Message::NavDown),
                _ => None,
            }),
            iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Window(iced::window::Event::Unfocused) => {
                    Some(Message::PaletteBlurred)
                }
                _ => None,
            }),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(value) => {
                // Intercept the /settings command.
                if value == "/settings" {
                    self.mode = Mode::Settings;
                    self.input.clear();
                    return self.resize_task();
                }
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
                    Mode::Settings => {
                        self.mode = Mode::Idle;
                        self.resize_task()
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
                self.mode = Mode::Idle;
                self.resize_task()
            }

            Message::PaletteBlurred => {
                // Only auto-hide if not mid-chat — agent tools (e.g. composio link)
                // may open browser tabs and steal focus.
                if self.mode != Mode::Chatting && self.visible {
                    self.hide_palette()
                } else {
                    Task::none()
                }
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

            Message::WindowIdResolved(id) => {
                self.window_id = id;
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let input = ui::command_input::view(&self.input, self.mode, self.is_agent_ready);

        let mut stack: Column<'_, Message> = column![input].spacing(4);

        match self.mode {
            Mode::Settings => {
                stack = stack.push(ui::settings::view(
                    &self.api_key_input,
                    &self.settings.hotkey,
                    self.recording_hotkey,
                ));
            }
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
        self.messages.push(ChatMessageView::user(user_id, content.clone()));
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
                    msg.blocks.push(ContentBlock::ToolEnd { tool, args, result });
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

    /// Desired palette window height for the current mode + content state.
    /// Ported from the old `usePalette.ts` sizing heuristic.
    fn target_height(&self) -> f32 {
        const BASE: f32 = 90.0;
        const ROW: f32 = 52.0;
        const MAX_LIST: f32 = 260.0;
        const CHAT: f32 = 480.0;
        match self.mode {
            Mode::Chatting => BASE + CHAT,
            Mode::Settings => BASE + CHAT,
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

    /// Emit an `iced::window::resize` task to match the current target height,
    /// or `Task::none()` if the window id hasn't been resolved yet.
    fn resize_task(&self) -> Task<Message> {
        let Some(id) = self.window_id else {
            return Task::none();
        };
        iced::window::resize(id, iced::Size::new(720.0, self.target_height()))
    }

    fn toggle_palette(&mut self) -> Task<Message> {
        if self.visible {
            self.hide_palette()
        } else {
            self.show_palette()
        }
    }

    fn show_palette(&mut self) -> Task<Message> {
        self.visible = true;
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

        #[cfg(target_os = "macos")]
        unsafe {
            let ptr = crate::platform::macos::first_app_window_ptr();
            crate::platform::macos::apply_palette_style(ptr);
            crate::platform::macos::order_front_and_make_key(ptr);
        }

        // Position + resize + focus. Each is independent and safe to dispatch
        // in parallel via Task::batch.
        let mut tasks: Vec<Task<Message>> = Vec::with_capacity(3);
        #[cfg(target_os = "macos")]
        if let Some(id) = self.window_id {
            if let Some((x, y)) = crate::platform::macos::cursor_palette_position(720.0) {
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
        self.visible = false;

        #[cfg(target_os = "macos")]
        unsafe {
            let ptr = crate::platform::macos::first_app_window_ptr();
            crate::platform::macos::order_out(ptr);
        }
        Task::none()
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
