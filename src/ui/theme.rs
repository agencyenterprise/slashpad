//! Dark theme + window configuration.
//!
//! Colors ported from the Tailwind config in the React app:
//!   surface-0: #0b0b0d
//!   surface-1: #161618
//!   surface-2: #1f1f23
//!   surface-3: #2a2a30
//!   accent:    #c4a1ff

use std::sync::atomic::{AtomicU32, Ordering};

use iced::theme::Palette;
use iced::{Color, Theme};

use crate::settings::AccentColor;

/// Packed 0x00RRGGBB accent color. Read via `accent()` at render time,
/// written via `set_accent()` when the user picks a new color in
/// Settings. Initialized to the purple default; `Slashpad::new`
/// overrides this from the persisted setting before the first render.
static ACCENT_PACKED: AtomicU32 = AtomicU32::new(0x00c4a1ff);

pub fn accent() -> Color {
    let packed = ACCENT_PACKED.load(Ordering::Relaxed);
    let r = ((packed >> 16) & 0xff) as u8;
    let g = ((packed >> 8) & 0xff) as u8;
    let b = (packed & 0xff) as u8;
    Color::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

pub fn set_accent(c: AccentColor) {
    let (r, g, b) = c.rgb();
    let packed = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    ACCENT_PACKED.store(packed, Ordering::Relaxed);
}

pub const SURFACE_0: Color = Color::from_rgb(
    0x0b as f32 / 255.0,
    0x0b as f32 / 255.0,
    0x0d as f32 / 255.0,
);

pub const SURFACE_1: Color = Color::from_rgb(
    0x16 as f32 / 255.0,
    0x16 as f32 / 255.0,
    0x18 as f32 / 255.0,
);

pub const SURFACE_2: Color = Color::from_rgb(
    0x1f as f32 / 255.0,
    0x1f as f32 / 255.0,
    0x23 as f32 / 255.0,
);

pub const SURFACE_3: Color = Color::from_rgb(
    0x2a as f32 / 255.0,
    0x2a as f32 / 255.0,
    0x30 as f32 / 255.0,
);

pub const TEXT: Color = Color::from_rgb(0.94, 0.94, 0.96);
pub const MUTED: Color = Color::from_rgb(0.55, 0.55, 0.60);
pub const DANGER: Color = Color::from_rgb(0.96, 0.45, 0.45);
pub const SUCCESS: Color = Color::from_rgb(0.5, 0.85, 0.65);

/// Thin vertical scrollbar geometry: narrow track/thumb hugging the right
/// edge. Paired with `scrollbar_style` for a subtle overlay-style scrollbar.
pub fn scrollbar_direction() -> iced::widget::scrollable::Direction {
    use iced::widget::scrollable::{Direction, Scrollbar};
    Direction::Vertical(
        Scrollbar::new()
            .width(6.0)
            .scroller_width(6.0)
            .margin(2.0),
    )
}

/// Shared scrollbar style for all `scrollable` widgets in the palette.
/// Rail is transparent (no track), so the thumb floats over the panel's
/// own background. Resting thumb matches `SURFACE_3` (the panel border
/// color) so it reads as part of the frame; hover/drag tint it with
/// `ACCENT` to mirror the selection language.
pub fn scrollbar_style(
    _theme: &iced::Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    use iced::widget::scrollable::{Rail, Scroller, Status, Style};
    use iced::{Background, Border};

    let scroller_color = match status {
        Status::Active => SURFACE_3,
        Status::Hovered { .. } => Color { a: 0.75, ..accent() },
        Status::Dragged { .. } => accent(),
    };

    let rail = Rail {
        background: Some(Background::Color(Color::TRANSPARENT)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        scroller: Scroller {
            color: scroller_color,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 3.0.into(),
            },
        },
    };

    Style {
        container: iced::widget::container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
    }
}

/// Thin horizontal divider line in SURFACE_3, used between the sections
/// of the unified palette card (input / middle panel / keyhints).
pub fn divider<Message: 'static>() -> iced::Element<'static, Message> {
    iced::widget::horizontal_rule(1)
        .style(|_theme: &iced::Theme| iced::widget::rule::Style {
            color: SURFACE_3,
            width: 1,
            radius: 0.0.into(),
            fill_mode: iced::widget::rule::FillMode::Full,
        })
        .into()
}

pub fn dark_theme() -> Theme {
    Theme::custom(
        "Slashpad Dark".to_string(),
        Palette {
            background: SURFACE_0,
            text: TEXT,
            primary: accent(),
            success: SUCCESS,
            danger: DANGER,
        },
    )
}

/// Initial iced window settings for the palette window.
///
/// The palette is a fixed-size card. We intentionally do NOT resize it in
/// response to mode/content changes any more — every resize hop used to
/// flash the NSPanel, and the UX cost of a small amount of empty space
/// below a short list is strictly lower than that flicker. Internal
/// scroll containers handle overflow.
pub fn palette_window_settings() -> iced::window::Settings {
    let size = iced::Size::new(crate::app::Slashpad::LAUNCHER_W, crate::app::Slashpad::LAUNCHER_H);
    iced::window::Settings {
        size,
        position: iced::window::Position::Centered,
        min_size: Some(size),
        max_size: None,
        visible: false,
        resizable: false,
        decorations: false,
        transparent: true,
        level: iced::window::Level::AlwaysOnTop,
        exit_on_close_request: false,
        ..Default::default()
    }
}

/// Compact settings-panel window settings. Initial position must be supplied
/// by the caller (so the tray-click handler can anchor it under the tray
/// icon). Mirrors the pre-rewrite Tauri `TraySettings` window dimensions.
///
/// The settings window is intentionally NOT put through our `SlashpadPanel`
/// NSPanel subclass — repeated attempts to class-swap it crashed inside
/// AppKit's redraw notification machinery, and we haven't root-caused the
/// difference from the palette's (working) NSPanel path yet. Relying on
/// iced's native window settings (borderless, transparent, always-on-top)
/// gives us the right visual without the crash risk. Trade-off: the
/// settings window briefly activates the app on show, and it won't float
/// over fullscreen apps. Fixable later.
pub fn settings_window_settings(x: f32, y: f32) -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(340.0, 430.0),
        position: iced::window::Position::Specific(iced::Point::new(x, y)),
        min_size: None,
        max_size: None,
        visible: true,
        resizable: false,
        decorations: false,
        transparent: true,
        level: iced::window::Level::AlwaysOnTop,
        exit_on_close_request: true,
        ..Default::default()
    }
}
