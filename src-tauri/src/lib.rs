use tauri::{
    AppHandle, Emitter, Listener, Manager, LogicalSize, PhysicalPosition,
    WebviewWindow,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// Reposition the palette window centered on whichever monitor the cursor is on.
/// Window height is dynamic — we only center horizontally and position ~25% from top.
fn center_on_cursor_monitor(window: &WebviewWindow) -> Result<(), String> {
    let monitors = window.available_monitors().map_err(|e| e.to_string())?;
    let cursor = window.cursor_position().map_err(|e| e.to_string())?;

    // Find which monitor contains the cursor
    let target_monitor = monitors
        .iter()
        .find(|m| {
            let pos = m.position();
            let size = m.size();

            // Monitor bounds in physical pixels
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

        // Center horizontally on the monitor, position ~22% from top vertically
        let x = mon_pos.x as f64 + (mon_size.width as f64 - win_size.width as f64) / 2.0;
        let y = mon_pos.y as f64 + (mon_size.height as f64 * 0.22);

        window
            .set_position(PhysicalPosition::new(x as i32, y as i32))
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn toggle_palette(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("palette") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            let _ = app.emit("palette-hidden", ());
        } else {
            // Reposition before showing
            let _ = center_on_cursor_monitor(&window);
            let _ = window.show();
            let _ = window.set_focus();
            let _ = app.emit("palette-shown", ());
        }
    }
}

#[tauri::command]
async fn hide_palette(app: AppHandle) -> Result<(), String> {
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
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            hide_palette,
            resize_palette,
            get_launchpad_dir,
            get_project_dir,
        ])
        .setup(|app| {
            // Register global shortcut with handler: Option+Space (macOS) / Alt+Space (Windows/Linux)
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
                if let Some(window) = handle2.get_webview_window("palette") {
                    let _ = window.hide();
                }
            });

            // Make webview background transparent on macOS
            #[cfg(target_os = "macos")]
            {
                use cocoa::appkit::{NSColor, NSWindow};
                use cocoa::base::{id, nil};

                if let Some(window) = app.get_webview_window("palette") {
                    let ns_window = window.ns_window().unwrap() as id;
                    unsafe {
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Launchpad");
}
