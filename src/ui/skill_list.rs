//! Skill search results list.

use iced::widget::{button, container, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::state::Skill;

pub fn view(
    skills: &[Skill],
    selected: usize,
    scroll_id: scrollable::Id,
) -> Element<'_, Message> {
    if skills.is_empty() {
        return container(text("No matching skills").size(13).color(super::theme::MUTED))
            .padding(16)
            .width(Length::Fill)
            .style(panel_style)
            .into();
    }

    let mut col: Column<'_, Message> = Column::new();
    let last = skills.len().saturating_sub(1);
    for (i, skill) in skills.iter().enumerate() {
        let is_selected = i == selected;
        let is_first = i == 0;
        let is_last = i == last;
        let row = row![
            text(format!("/{}", skill.name))
                .size(13)
                .color(super::theme::ACCENT)
                .width(Length::Fixed(120.0)),
            text(truncate(&skill.description, 90))
                .size(12)
                .color(super::theme::MUTED)
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);

        let row_container = container(row)
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

    // `max_height` bounds the scrollable so it scrolls once content
    // exceeds the cap, while allowing the panel to shrink to content
    // (no dead space) for short lists. Matches `MAX_LIST` in
    // `Launchpad::target_height`.
    container(scrollable(col).id(scroll_id))
        .padding(0)
        .width(Length::Fill)
        .max_height(260.0)
        .style(panel_style)
        .into()
}

/// Per-row corner radii that match the panel's 12px border on the outer
/// edges. Without this, a highlighted first/last row draws sharp corners
/// that poke outside the rounded panel.
fn selection_radius(is_first: bool, is_last: bool) -> iced::border::Radius {
    // Panel radius (see `panel_style`) minus the 1px border so the
    // selection fill sits just inside the stroke.
    let r = 11.0;
    iced::border::Radius {
        top_left: if is_first { r } else { 0.0 },
        top_right: if is_first { r } else { 0.0 },
        bottom_left: if is_last { r } else { 0.0 },
        bottom_right: if is_last { r } else { 0.0 },
    }
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
        background: Some(iced::Background::Color(super::theme::SURFACE_1)),
        border: iced::Border {
            color: super::theme::SURFACE_3,
            width: 1.0,
            radius: 12.0.into(),
        },
        text_color: Some(super::theme::TEXT),
        ..Default::default()
    }
}
