use tauri::{
    AppHandle, Emitter, Listener, Manager, LogicalSize, PhysicalPosition,
    WebviewWindow,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use include_dir::{include_dir, Dir};

#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel,
    ManagerExt as PanelManagerExt,
    WebviewWindowExt as PanelWindowExt,
    Panel,
    builder::CollectionBehavior,
};

static SKILL_CREATOR_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/bundled-skills/skill-creator");

/// Seed a bundled skill into the user's skills directory if it doesn't already exist.
fn seed_bundled_skill(skills_dir: &str, skill_name: &str, dir: &Dir<'static>) {
    let dest = format!("{}/{}", skills_dir, skill_name);
    if std::path::Path::new(&dest).exists() {
        return;
    }
    if let Err(e) = extract_dir(dir, &std::path::Path::new(&dest)) {
        eprintln!("Failed to seed skill '{}': {}", skill_name, e);
    }
}

fn extract_dir(dir: &Dir<'static>, dest: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for file in dir.files() {
        if let Some(name) = file.path().file_name() {
            std::fs::write(dest.join(name), file.contents())?;
        }
    }
    for sub_dir in dir.dirs() {
        if let Some(name) = sub_dir.path().file_name() {
            extract_dir(sub_dir, &dest.join(name))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(PalettePanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true
        }
    })
}

/// Reposition the palette window centered on whichever monitor the cursor is on.
fn center_on_cursor_monitor(window: &WebviewWindow) -> Result<(), String> {
    let monitors = window.available_monitors().map_err(|e| e.to_string())?;
    let cursor = window.cursor_position().map_err(|e| e.to_string())?;

    let target_monitor = monitors
        .iter()
        .find(|m| {
            let pos = m.position();
            let size = m.size();
            let mx = pos.x as f64;
            let my = pos.y as f64;
            let mw = size.width as f64;
            let mh = size.height as f64;

            cursor.x >= mx
                && cursor.x < mx + mw
                && cursor.y >= my
                && cursor.y < my + mh
        })
        .or_else(|| monitors.first());

    if let Some(monitor) = target_monitor {
        let mon_pos = monitor.position();
        let mon_size = monitor.size();
        let win_size = window.outer_size().map_err(|e| e.to_string())?;

        let x = mon_pos.x as f64 + (mon_size.width as f64 - win_size.width as f64) / 2.0;
        let y = mon_pos.y as f64 + (mon_size.height as f64 * 0.22);

        window
            .set_position(PhysicalPosition::new(x as i32, y as i32))
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn toggle_palette(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        if let Ok(panel) = app.get_webview_panel("palette") {
            if panel.is_visible() {
                panel.hide();
                let _ = app.emit("palette-hidden", ());
            } else {
                if let Some(window) = app.get_webview_window("palette") {
                    let _ = center_on_cursor_monitor(&window);

                    let ns_win = window.ns_window().unwrap() as cocoa::base::id;
                    unsafe {
                        use cocoa::appkit::NSWindow;
                        use cocoa::base::NO;

                        ns_win.setLevel_(8); // NSModalPanelWindowLevel
                        ns_win.setHidesOnDeactivate_(NO);
                        let behavior: u64 = (1 << 0) | (1 << 6) | (1 << 8);
                        ns_win.setCollectionBehavior_(std::mem::transmute(behavior));
                    }
                }

                panel.show();
                panel.order_front_regardless();
                panel.make_key_window();

                unsafe {
                    use cocoa::appkit::NSApplication;
                    use cocoa::base::YES;
                    cocoa::appkit::NSApp().activateIgnoringOtherApps_(YES);
                }

                let _ = app.emit("palette-shown", ());
            }
            return;
        }
    }

    // Non-macOS fallback
    if let Some(window) = app.get_webview_window("palette") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            let _ = app.emit("palette-hidden", ());
        } else {
            let _ = center_on_cursor_monitor(&window);
            let _ = window.show();
            let _ = window.set_focus();
            let _ = app.emit("palette-shown", ());
        }
    }
}

#[tauri::command]
async fn hide_palette(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(panel) = app.get_webview_panel("palette") {
            panel.hide();
            return Ok(());
        }
    }

    if let Some(window) = app.get_webview_window("palette") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn resize_palette(app: AppHandle, height: f64) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("palette") {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let current = window.outer_size().map_err(|e| e.to_string())?;
        let logical_width = current.width as f64 / scale;
        window
            .set_size(LogicalSize::new(logical_width, height))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn get_launchpad_dir() -> Result<String, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = format!("{}/.launchpad", home);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[tauri::command]
async fn get_project_dir() -> Result<String, String> {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_fs::init());

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .invoke_handler(tauri::generate_handler![
            hide_palette,
            resize_palette,
            get_launchpad_dir,
            get_project_dir,
        ])
        .setup(|app| {
            let shortcut: Shortcut = "Ctrl+Space".parse().unwrap();

            let handle = app.handle().clone();
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |_app, _shortcut, event| {
                        if event.state() == ShortcutState::Pressed {
                            toggle_palette(&handle);
                        }
                    })
                    .build(),
            )?;

            app.global_shortcut().register(shortcut)?;

            // Listen for blur events from the frontend to auto-hide
            let handle2 = app.handle().clone();
            app.listen("palette-blur", move |_| {
                #[cfg(target_os = "macos")]
                {
                    if let Ok(panel) = handle2.get_webview_panel("palette") {
                        panel.hide();
                        return;
                    }
                }

                if let Some(window) = handle2.get_webview_window("palette") {
                    let _ = window.hide();
                }
            });

            // macOS: convert window to NSPanel for full-screen overlay
            #[cfg(target_os = "macos")]
            {
                unsafe {
                    use cocoa::appkit::{NSApplication, NSApplicationActivationPolicyAccessory};
                    cocoa::appkit::NSApp().setActivationPolicy_(NSApplicationActivationPolicyAccessory);
                }

                if let Some(window) = app.get_webview_window("palette") {
                    let _ = window.to_panel::<PalettePanel>();

                    if let Ok(panel) = app.get_webview_panel("palette") {
                        panel.set_level(8); // NSModalPanelWindowLevel
                        panel.set_floating_panel(true);

                        let behavior = CollectionBehavior::new()
                            .can_join_all_spaces()
                            .ignores_cycle()
                            .full_screen_auxiliary();
                        panel.set_collection_behavior(behavior.value());
                    }

                    let ns_window = window.ns_window().unwrap() as cocoa::base::id;
                    unsafe {
                        use cocoa::appkit::{NSColor, NSWindow};
                        use cocoa::base::nil;
                        use objc::{msg_send, sel, sel_impl};

                        // NSNonactivatingPanelMask — critical for full-screen overlay
                        let current_mask: u64 = msg_send![ns_window, styleMask];
                        let _: () = msg_send![ns_window, setStyleMask: current_mask | (1u64 << 7)];

                        ns_window.setLevel_(8);
                        let behavior: u64 = (1 << 0) | (1 << 6) | (1 << 8);
                        ns_window.setCollectionBehavior_(std::mem::transmute(behavior));

                        let bg_color = NSColor::colorWithRed_green_blue_alpha_(
                            nil, 0.0, 0.0, 0.0, 0.0,
                        );
                        ns_window.setBackgroundColor_(bg_color);
                    }
                }
            }

            // Create launchpad project directory with .claude/skills
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let skills_dir = format!("{}/.launchpad/.claude/skills", home);
            let _ = std::fs::create_dir_all(&skills_dir);

            seed_bundled_skill(&skills_dir, "skill-creator", &SKILL_CREATOR_DIR);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Launchpad");
}
