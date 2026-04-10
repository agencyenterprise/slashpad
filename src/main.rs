#![allow(dead_code)]

mod app;
mod fuzzy;
mod hotkey;
mod markdown;
mod platform;
mod sessions;
mod settings;
mod sidecar;
mod skills;
mod state;
mod tray;
mod ui;

use app::Launchpad;

fn main() -> iced::Result {
    // Enter a tokio runtime for the whole process lifetime so `tokio::spawn`
    // calls made from inside iced callbacks find an active reactor.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let _guard = runtime.enter();

    // Seed bundled skills so the skills directory always has skill-creator.
    if let Err(e) = skills::seed_bundled_skills() {
        eprintln!("[launchpad] Failed to seed bundled skills: {e}");
    }

    // Use an Accessory activation policy on macOS so no Dock icon appears.
    #[cfg(target_os = "macos")]
    platform::macos::set_accessory_activation_policy();

    iced::application(Launchpad::title, Launchpad::update, Launchpad::view)
        .subscription(Launchpad::subscription)
        .theme(Launchpad::theme)
        .window(ui::theme::palette_window_settings())
        .antialiasing(true)
        .run_with(Launchpad::new)
}
