//! Menu-bar tray icon integration.
//!
//! `tray-icon` on macOS uses `NSStatusItem`, which is driven by the main-thread
//! CFRunLoop that iced/winit already pump. We therefore create the tray icon
//! on the main thread in `main()` before handing control to iced, and forward
//! click / menu events into the existing `External` bus via
//! `set_event_handler` callbacks — no extra thread or event-loop polling
//! required.
//!
//! Content mirrors the pre-rewrite Tauri tray: left-click opens the settings
//! panel (which now includes "Show Launcher" and "Quit Launchpad" buttons),
//! and a right-click context menu exposes the same three actions directly.

#[cfg(target_os = "macos")]
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
};

#[cfg(target_os = "macos")]
use crate::app::{external_sender, External};

#[cfg(target_os = "macos")]
const MENU_ID_SHOW: &str = "launchpad.show";
#[cfg(target_os = "macos")]
const MENU_ID_QUIT: &str = "launchpad.quit";

/// Build the menu-bar tray icon. MUST be called on the main thread after
/// `app::init_external_bus()` and before iced takes over the run loop.
///
/// The resulting `TrayIcon` is `!Send` and must live for the entire process;
/// we `Box::leak` it so the underlying `NSStatusItem` stays alive without
/// needing a place to store it.
#[cfg(target_os = "macos")]
pub fn init() {
    // CRITICAL: install the event handlers BEFORE building the tray.
    // `TRAY_EVENT_HANDLER` / `MENU_EVENT_HANDLER` inside tray-icon / muda are
    // `OnceCell`s. Their internal `send()` does `get_or_init(|| None)`, which
    // permanently seals the cell on the first dispatched event. After that,
    // `set_event_handler` silently no-ops via `let _ = .set(...)`. Registering
    // first means our handlers are the value the OnceCell is initialized with.
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            rect,
            ..
        } = event
        {
            // tray-icon reports the icon's rect in PHYSICAL pixels (winit
            // top-left origin). iced's `window::move_to` uses LOGICAL
            // coordinates, so we divide by the primary screen's backing
            // scale factor before forwarding to the update handler.
            let scale = crate::platform::macos::primary_scale_factor().max(1.0);
            let tray_x = rect.position.x / scale;
            let tray_y = rect.position.y / scale;
            let tray_w = (rect.size.width as f64) / scale;
            let tray_h = (rect.size.height as f64) / scale;
            let _ = external_sender().send(External::TrayClicked {
                tray_x,
                tray_y,
                tray_w,
                tray_h,
            });
        }
    }));

    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        let msg = match event.id.0.as_str() {
            MENU_ID_SHOW => External::TrayMenuShow,
            MENU_ID_QUIT => External::TrayMenuQuit,
            _ => return,
        };
        let _ = external_sender().send(msg);
    }));

    let icon = match load_icon() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[launchpad] failed to load tray icon: {e}");
            return;
        }
    };

    let menu = Menu::new();
    if let Err(e) = build_menu(&menu) {
        eprintln!("[launchpad] failed to build tray menu: {e}");
        return;
    }

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_tooltip("Launchpad")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build();

    let tray = match tray {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[launchpad] failed to create tray icon: {e}");
            return;
        }
    };

    // Keep the NSStatusItem alive for the whole process.
    Box::leak(Box::new(tray));
}

#[cfg(not(target_os = "macos"))]
pub fn init() {
    // Other platforms: tray integration is macOS-only for now.
}

#[cfg(target_os = "macos")]
fn load_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let bytes = include_bytes!("../icons/32x32.png");
    let img = image::load_from_memory(bytes)?.into_rgba8();
    let (w, h) = img.dimensions();
    let rgba = img.into_raw();
    Ok(Icon::from_rgba(rgba, w, h)?)
}

#[cfg(target_os = "macos")]
fn build_menu(menu: &Menu) -> Result<(), Box<dyn std::error::Error>> {
    // Right-click menu: Show Launcher + Quit. Settings is deliberately not
    // here — left-click on the tray icon already opens the settings panel,
    // and offering a "Settings…" menu item would fire without a click rect
    // so we couldn't anchor the panel below the icon correctly.
    menu.append(&MenuItem::with_id(
        MENU_ID_SHOW,
        "Show Launcher",
        true,
        None,
    ))?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id(
        MENU_ID_QUIT,
        "Quit Launchpad",
        true,
        None,
    ))?;
    Ok(())
}
