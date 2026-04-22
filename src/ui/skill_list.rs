//! Skill search results list.

use iced::widget::{button, container, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::state::Skill;

/// A skill together with its pinned flag, as computed by the app before
/// rendering. Pinned skills float to the top of the filtered list and
/// render their name in the accent color; unpinned skills render in
/// the default text color.
pub struct SkillRow<'a> {
    pub skill: &'a Skill,
    pub pinned: bool,
}

pub fn view<'a>(
    rows: Vec<SkillRow<'a>>,
    selected: usize,
    scroll_id: scrollable::Id,
) -> Element<'a, Message> {
    if rows.is_empty() {
        return container(text("No matching skills").size(13).color(super::theme::MUTED))
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(panel_style)
            .into();
    }

    let mut col: Column<'a, Message> = Column::new();
    let last = rows.len().saturating_sub(1);
    for (i, skill_row) in rows.into_iter().enumerate() {
        let is_selected = i == selected;
        let is_first = i == 0;
        let is_last = i == last;
        let skill = skill_row.skill;
        let name_color = if skill_row.pinned {
            super::theme::accent()
        } else {
            super::theme::TEXT
        };
        let description_cell: Element<'_, Message> = text(truncate(&skill.description, 180))
            .size(12)
            .color(super::theme::MUTED)
            .height(Length::Fixed(32.0))
            .wrapping(iced::widget::text::Wrapping::Word)
            .width(Length::Fill)
            .into();

        let row_inner = row![
            text(format!("/{}", skill.name))
                .size(13)
                .color(name_color)
                .width(Length::Fixed(120.0)),
            description_cell,
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);

        let row_container = container(row_inner)
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
                    radius: selection_radius(is_first, is_last),
                },
                text_color: Some(super::theme::TEXT),
                ..Default::default()
            });

        let btn = button(row_container)
            .on_press(Message::SelectSkill(i))
            .padding(0)
            .width(Length::Fill)
            .style(|_theme, _status| iced::widget::button::Style {
                background: None,
                text_color: super::theme::TEXT,
                ..Default::default()
            });
        col = col.push(btn);
    }

    // Window is fixed-height (see `Slashpad::LAUNCHER_H`), so the list
    // fills whatever space is left after input + keyhints. The inner
    // scrollable takes the same `Length::Fill` so content longer than
    // the available area scrolls inside this bounded container.
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

/// Per-row corner radii. The panel no longer has its own rounded frame
/// (the unified outer container in `app.rs::view()` provides that), so
/// every row draws square selection corners flush with the section
/// dividers above and below.
fn selection_radius(_is_first: bool, _is_last: bool) -> iced::border::Radius {
    0.0.into()
}

/// Single-line description: trim leading whitespace, collapse newlines,
/// and clip to `max_chars` with an ellipsis so rows stay uniform height.
fn truncate(s: &str, max_chars: usize) -> String {
    let flat: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= max_chars {
        flat
    } else {
        let kept: String = flat.chars().take(max_chars).collect();
        format!("{}…", kept.trim_end())
    }
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
