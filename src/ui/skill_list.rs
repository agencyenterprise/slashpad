//! Skill search results list.

use iced::widget::{button, container, row, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::state::Skill;

pub fn view(skills: &[Skill], selected: usize) -> Element<'_, Message> {
    if skills.is_empty() {
        return container(text("No matching skills").size(13).color(super::theme::MUTED))
            .padding(16)
            .width(Length::Fill)
            .style(panel_style)
            .into();
    }

    let mut col: Column<'_, Message> = Column::new();
    for (i, skill) in skills.iter().enumerate() {
        let is_selected = i == selected;
        let row = row![
            text(format!("/{}", skill.name))
                .size(13)
                .color(super::theme::ACCENT)
                .width(Length::Fixed(120.0)),
            text(skill.description.clone())
                .size(12)
                .color(super::theme::MUTED),
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

    container(col)
        .padding(0)
        .width(Length::Fill)
        .style(panel_style)
        .into()
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
