//! Project picker — Cmd+P list of directories Claude Code has been
//! run in (decoded from `~/.claude/projects/`). Mirrors the layout of
//! `skill_list`: one row per project, keyboard selection highlight,
//! bounded scrollable.

use iced::widget::{button, container, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::projects::ProjectInfo;

pub fn view(
    projects: &[ProjectInfo],
    selected: usize,
    scroll_id: scrollable::Id,
) -> Element<'_, Message> {
    if projects.is_empty() {
        return container(
            text("No projects found — run Claude Code in a directory to see it here.")
                .size(13)
                .color(super::theme::MUTED),
        )
        .padding(16)
        .width(Length::Fill)
        .style(panel_style)
        .into();
    }

    let mut col: Column<'_, Message> = Column::new();
    for (i, project) in projects.iter().enumerate() {
        let is_selected = i == selected;
        let label = text(project.display.clone())
            .size(13)
            .color(super::theme::TEXT)
            .wrapping(iced::widget::text::Wrapping::None);
        let row_body: Element<'_, Message> = if project.is_default {
            // The default row gets a muted "default" tag on the right
            // so the user can tell which entry is the original
            // `~/.launchpad` cwd at a glance.
            row![
                label,
                iced::widget::horizontal_space().width(Length::Fill),
                text("default").size(11).color(super::theme::MUTED),
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
            .direction(super::theme::scrollbar_direction())
            .style(super::theme::scrollbar_style),
    )
    .padding([4, 0])
    .width(Length::Fill)
    .max_height(260.0)
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
