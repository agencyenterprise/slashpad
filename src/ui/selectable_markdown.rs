//! Markdown renderer that uses `SelectableRichText` so chat messages
//! keep their formatting AND let the user select/copy text.
//!
//! Structure mirrors `iced::widget::markdown::view`
//! (at `iced_widget-0.13.4/src/markdown.rs`, ~lines 616–702) — only the
//! inner `rich_text(...)` calls are swapped for `selectable_rich_text(...)`.

use iced::advanced;
use iced::font::Font;
use iced::widget::markdown::{Catalog, Item, Settings, Style, Url};
use iced::widget::{column, container, row, scrollable, text};
use iced::{padding, Element, Length, Pixels};

use super::selectable_rich_text::selectable_rich_text;

pub fn view<'a, Theme, Renderer>(
    items: impl IntoIterator<Item = &'a Item>,
    settings: Settings,
    style: Style,
) -> Element<'a, Url, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: advanced::text::Renderer<Font = Font> + 'a,
{
    let Settings {
        text_size,
        h1_size,
        h2_size,
        h3_size,
        h4_size,
        h5_size,
        h6_size,
        code_size,
    } = settings;

    let spacing = text_size * 0.625;

    let blocks = items.into_iter().enumerate().map(|(i, item)| match item {
        Item::Heading(level, heading) => container(
            selectable_rich_text(heading.spans(style)).size(match level {
                iced::widget::markdown::HeadingLevel::H1 => h1_size,
                iced::widget::markdown::HeadingLevel::H2 => h2_size,
                iced::widget::markdown::HeadingLevel::H3 => h3_size,
                iced::widget::markdown::HeadingLevel::H4 => h4_size,
                iced::widget::markdown::HeadingLevel::H5 => h5_size,
                iced::widget::markdown::HeadingLevel::H6 => h6_size,
            }),
        )
        .padding(padding::top(if i > 0 {
            text_size / 2.0
        } else {
            Pixels::ZERO
        }))
        .into(),
        Item::Paragraph(paragraph) => {
            selectable_rich_text(paragraph.spans(style))
                .size(text_size)
                .into()
        }
        Item::List { start: None, items } => {
            column(items.iter().map(|items| {
                row![
                    text("•").size(text_size),
                    view(items, settings, style)
                ]
                .spacing(spacing)
                .into()
            }))
            .spacing(spacing)
            .into()
        }
        Item::List {
            start: Some(start),
            items,
        } => column(items.iter().enumerate().map(|(i, items)| {
            row![
                text(format!("{}.", i as u64 + *start)).size(text_size),
                view(items, settings, style)
            ]
            .spacing(spacing)
            .into()
        }))
        .spacing(spacing)
        .into(),
        Item::CodeBlock(code) => container(
            scrollable(
                container(
                    selectable_rich_text(code.spans(style))
                        .font(Font::MONOSPACE)
                        .size(code_size),
                )
                .padding(spacing.0 / 2.0),
            )
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default()
                    .width(spacing.0 / 2.0)
                    .scroller_width(spacing.0 / 2.0),
            )),
        )
        .width(Length::Fill)
        .padding(spacing.0 / 2.0)
        .class(<Theme as Catalog>::code_block())
        .into(),
    });

    Element::new(column(blocks).width(Length::Fill).spacing(text_size))
}
