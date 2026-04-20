//! Project picker — Cmd+P list of directories Claude Code has been
//! run in (decoded from `~/.claude/projects/`). Mirrors the layout of
//! `skill_list`: one row per project, keyboard selection highlight,
//! bounded scrollable.

use iced::widget::{button, container, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::projects::ProjectInfo;

/// A project plus its pinned flag. Pinned projects float to the top
/// and render their path in ACCENT — same visual language as pinned
/// sessions in the idle list.
pub struct ProjectRow<'a> {
    pub project: &'a ProjectInfo,
    pub pinned: bool,
}

pub fn view<'a>(
    rows: Vec<ProjectRow<'a>>,
    selected: usize,
    scroll_id: scrollable::Id,
) -> Element<'a, Message> {
    if rows.is_empty() {
        return container(
            text("No projects found — run Claude Code in a directory to see it here.")
                .size(13)
                .color(super::theme::MUTED),
        )
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(panel_style)
        .into();
    }

    let mut col: Column<'a, Message> = Column::new();
    for (i, project_row) in rows.into_iter().enumerate() {
        let is_selected = i == selected;
        let project = project_row.project;
        let label_color = if project_row.pinned {
            super::theme::ACCENT
        } else {
            super::theme::TEXT
        };
        let label = text(project.display.clone())
            .size(13)
            .color(label_color)
            .wrapping(iced::widget::text::Wrapping::None);
        // Right-side tag(s): pinned/default, muted so they don't
        // dominate the row but give a non-color indicator of status.
        let tag_text: Option<&'static str> = match (project_row.pinned, project.is_default) {
            (true, true) => Some("pinned · default"),
            (true, false) => Some("pinned"),
            (false, true) => Some("default"),
            (false, false) => None,
        };
        let row_body: Element<'_, Message> = if let Some(tag) = tag_text {
            row![
                label,
                iced::widget::horizontal_space().width(Length::Fill),
                text(tag).size(11).color(super::theme::MUTED),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            label.into()
        };
        let row_container = container(row_body)
            .padding([10, 18])
        .width(Length::Fill)
        .style(move |_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(if is_selected {
                super::theme::SURFACE_3
            } else {
                iced::Color::TRANSPARENT
            })),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        });

        let btn = button(row_container)
            .on_press(Message::SelectProject(i))
            .padding(0)
            .width(Length::Fill)
            .style(|_theme, _status| iced::widget::button::Style {
                background: None,
                text_color: super::theme::TEXT,
                ..Default::default()
            });
        col = col.push(btn);
    }

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
    .style(panel_style)
    .into()
}

fn panel_style(_theme: &iced::Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: None,
        border: iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        text_color: Some(super::theme::TEXT),
        ..Default::default()
    }
}
