//! Unified idle-list: active chats + recent past sessions.
//!
//! Rows are built by the caller (`Slashpad::build_idle_rows_view`) as
//! borrowed references into `self.chats` / `self.recent_sessions`, so
//! the view lifetime is tied to `&Slashpad`.

use iced::widget::{button, container, row, scrollable, text, text_input, Column};
use iced::{Element, Length};

use crate::app::{ChatEntry, Message, RENAME_INPUT_ID};
use crate::state::{ChatStatus, SessionInfo};

/// One row in the unified idle list.
pub enum IdleRow<'a> {
    /// A chat running (or completed) in this process.
    Active {
        entry: &'a ChatEntry,
        /// True when the row's underlying session is tagged pinned.
        /// Render a 📌 prefix and caller floats it to the top.
        pinned: bool,
    },
    /// A session loaded from disk that isn't also open as an active
    /// chat. Owned because the caller builds a fuzzy-filtered Vec —
    /// borrowing from that Vec's lifetime would escape the view fn.
    Past {
        session: SessionInfo,
        pinned: bool,
    },
}

pub fn view<'a>(
    rows: Vec<IdleRow<'a>>,
    selected: usize,
    spinner_frame: u32,
    scroll_id: scrollable::Id,
    renaming: Option<&'a str>,
    rename_input: &'a str,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new();

    // Track the filtered-past-session index separately: callers of
    // `Message::SelectSession` must pass an index into the past-rows
    // list, not the unified list, to stay in sync with
    // `Slashpad::past_session_rows()`.
    let mut past_index: usize = 0;
    let last = rows.len().saturating_sub(1);

    for (i, row_item) in rows.into_iter().enumerate() {
        let is_selected = i == selected;
        let is_first = i == 0;
        let is_last = i == last;
        // session_id for the row (if any) — used to decide whether this
        // row should render as an inline rename input.
        let row_session_id: Option<&str> = match &row_item {
            IdleRow::Active { entry, .. } => entry.state.session_id.as_deref(),
            IdleRow::Past { session, .. } => Some(session.session_id.as_str()),
        };
        let is_editing = match (renaming, row_session_id) {
            (Some(editing_id), Some(row_id)) => editing_id == row_id,
            _ => false,
        };

        let (row_el, msg) = match row_item {
            IdleRow::Active { entry, pinned } => {
                let status_label = status_text(
                    entry.state.status,
                    spinner_frame,
                    entry.state.last_activity_ms,
                );
                let status_color = status_color(entry.state.status);
                let title_el: Element<'a, Message> = if is_editing {
                    rename_input_widget(rename_input, pinned)
                } else {
                    text(entry.state.title.clone())
                        .size(13)
                        .color(title_color(pinned))
                        .width(Length::FillPortion(4))
                        .into()
                };
                let row = row![
                    title_el,
                    text(status_label)
                        .size(11)
                        .color(status_color)
                        .width(Length::Shrink),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center);
                (row, Message::SelectChat(entry.state.id))
            }
            IdleRow::Past { session, pinned } => {
                let time = format_relative(session.last_modified);
                let title_el: Element<'a, Message> = if is_editing {
                    rename_input_widget(rename_input, pinned)
                } else {
                    text(session.summary.clone())
                        .size(13)
                        .color(title_color(pinned))
                        .width(Length::FillPortion(4))
                        .into()
                };
                let row = row![
                    title_el,
                    text(time)
                        .size(11)
                        .color(super::theme::MUTED)
                        .width(Length::Shrink),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center);
                let idx = past_index;
                past_index += 1;
                (row, Message::SelectSession(idx))
            }
        };

        let row_container = container(row_el).padding([10, 18]).width(Length::Fill).style(
            move |_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(if is_selected {
                    super::theme::SURFACE_3
                } else {
                    iced::Color::TRANSPARENT
                })),
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: selection_radius(is_first, is_last),
                },
                text_color: Some(super::theme::TEXT),
                ..Default::default()
            },
        );

        // While a row is in rename mode, don't wrap it in a button —
        // otherwise clicks inside the text_input bubble up as
        // SelectSession/SelectChat and immediately cancel the edit. The
        // plain container still participates in layout and selection
        // highlighting; the inline text_input handles input.
        if is_editing {
            col = col.push(row_container);
        } else {
            let btn = button(row_container)
                .on_press(msg)
                .padding(0)
                .width(Length::Fill)
                .style(|_theme, _status| iced::widget::button::Style {
                    background: None,
                    text_color: super::theme::TEXT,
                    ..Default::default()
                });
            col = col.push(btn);
        }
    }

    // Window is fixed-height now (see `Slashpad::LAUNCHER_H`), so the
    // list fills whatever space is left after input + keyhints. The
    // inner scrollable takes the same `Length::Fill` so content longer
    // than the available area scrolls inside this bounded container.
    container(
        scrollable(col)
            .id(scroll_id)
            .width(Length::Fill)
            .height(Length::Fill)
            .direction(super::theme::scrollbar_direction())
            .style(super::theme::scrollbar_style),
    )
        .padding([4, 0])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: None,
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        })
        .into()
}

/// Per-row corner radii. The panel no longer has its own rounded frame
/// (the unified outer container in `app.rs::view()` provides that), so
/// every row draws square selection corners flush with the section
/// dividers above and below.
fn selection_radius(_is_first: bool, _is_last: bool) -> iced::border::Radius {
    0.0.into()
}

/// Title color for an idle-list row. Pinned rows render in accent
/// purple — the only visual cue that a row is pinned, replacing the
/// earlier inline pin glyph so the timestamp/status column stays
/// intact for every row.
fn title_color(pinned: bool) -> iced::Color {
    if pinned {
        super::theme::ACCENT
    } else {
        super::theme::TEXT
    }
}

/// Inline rename text_input, styled to blend with the row's title so the
/// swap between static text and editable field is visually minimal.
/// Pre-filled with the row's current title by the caller. Enter/Esc are
/// routed by the shared `decode_shortcut` subscription in `app.rs`:
/// while `renaming_session_id` is set, `Submit` short-circuits to
/// `CommitRename` and `EscapePressed` to `CancelRename`. We intentionally
/// do NOT set `.on_submit` here — it would race the subscription and
/// cause Enter to fire twice (once as CommitRename, once as Submit that
/// falls through to opening the session).
fn rename_input_widget<'a>(value: &'a str, pinned: bool) -> Element<'a, Message> {
    let value_color = title_color(pinned);
    text_input("Rename session…", value)
        .id(RENAME_INPUT_ID.clone())
        .on_input(Message::RenameInputChanged)
        .size(13)
        .padding(0)
        .width(Length::FillPortion(4))
        .style(move |_theme: &iced::Theme, _status| iced::widget::text_input::Style {
            background: iced::Background::Color(iced::Color::TRANSPARENT),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            icon: super::theme::MUTED,
            placeholder: super::theme::MUTED,
            value: value_color,
            selection: iced::Color {
                a: 0.35,
                ..super::theme::ACCENT
            },
        })
        .into()
}

fn status_text(status: ChatStatus, spinner_frame: u32, last_activity_ms: i64) -> String {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    let glyph = FRAMES[(spinner_frame as usize) % FRAMES.len()];
    match status {
        ChatStatus::Initializing => format!("{glyph} starting"),
        ChatStatus::Streaming => format!("{glyph} streaming"),
        ChatStatus::Idle | ChatStatus::Error | ChatStatus::Closed => {
            format_relative(last_activity_ms)
        }
    }
}

fn status_color(status: ChatStatus) -> iced::Color {
    match status {
        ChatStatus::Initializing | ChatStatus::Streaming => super::theme::ACCENT,
        ChatStatus::Idle | ChatStatus::Error | ChatStatus::Closed => super::theme::MUTED,
    }
}

fn format_relative(unix_millis: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let diff_ms = now - unix_millis;
    if diff_ms < 0 {
        return "just now".to_string();
    }
    let minutes = diff_ms / 60_000;
    if minutes < 1 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else {
        let hours = minutes / 60;
        if hours < 24 {
            format!("{hours}h ago")
        } else {
            let days = hours / 24;
            format!("{days}d ago")
        }
    }
}
