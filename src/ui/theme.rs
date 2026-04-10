//! Dark theme + window configuration.
//!
//! Colors ported from the Tailwind config in the React app:
//!   surface-0: #0b0b0d
//!   surface-1: #161618
//!   surface-2: #1f1f23
//!   surface-3: #2a2a30
//!   accent:    #c4a1ff

use iced::theme::Palette;
use iced::{Color, Theme};

pub const ACCENT: Color = Color::from_rgb(
    0xc4 as f32 / 255.0,
    0xa1 as f32 / 255.0,
    0xff as f32 / 255.0,
);

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

pub fn dark_theme() -> Theme {
    Theme::custom(
        "Launchpad Dark".to_string(),
        Palette {
            background: SURFACE_0,
            text: TEXT,
            primary: ACCENT,
            success: SUCCESS,
            danger: DANGER,
        },
    )
}

/// Initial iced window settings for the palette window.
pub fn palette_window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(720.0, 90.0),
        position: iced::window::Position::Centered,
        min_size: Some(iced::Size::new(720.0, 90.0)),
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
/// The settings window is intentionally NOT put through our `LaunchpadPanel`
/// NSPanel subclass — repeated attempts to class-swap it crashed inside
/// AppKit's redraw notification machinery, and we haven't root-caused the
/// difference from the palette's (working) NSPanel path yet. Relying on
/// iced's native window settings (borderless, transparent, always-on-top)
/// gives us the right visual without the crash risk. Trade-off: the
/// settings window briefly activates the app on show, and it won't float
/// over fullscreen apps. Fixable later.
pub fn settings_window_settings(x: f32, y: f32) -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(340.0, 280.0),
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
