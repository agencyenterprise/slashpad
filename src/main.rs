#![allow(dead_code)]

mod app;
mod fuzzy;
mod hotkey;
mod platform;
mod secrets;
mod sessions;
mod settings;
mod sidecar;
mod skills;
mod state;
mod terminal;
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

    // Seed the default CLAUDE.md so the user can customize the system prompt.
    if let Err(e) = sidecar::seed_default_claude_md() {
        eprintln!("[launchpad] Failed to seed default CLAUDE.md: {e}");
    }

    // Use an Accessory activation policy on macOS so no Dock icon appears.
    #[cfg(target_os = "macos")]
    platform::macos::set_accessory_activation_policy();

    // Initialize the external event bus BEFORE iced::application — both
    // `Launchpad::new()`'s hotkey forwarder and the tray handlers push
    // events through it, and the iced subscription drains it.
    app::init_external_bus();

    // Note: tray::init() is deliberately NOT called here. tray-icon's macOS
    // docs require the NSApplication event loop to be running before the
    // NSStatusItem is created, and calling it before `run_with` leaves the
    // tray in a state where click events don't dispatch. See the dispatch
    // hook in `Launchpad::new()` that creates the tray once iced's run loop
    // starts draining the main dispatch queue.

    // Multi-window daemon: the launcher palette and the tray-anchored
    // settings window are each their own iced window. `iced::daemon` starts
    // with no windows; `Launchpad::new()` returns a task that opens the
    // palette via `iced::window::open(...)`.
    //
    // Font bundling: iced's built-in Fira Sans has very narrow glyph
    // coverage — no dingbats (● ✓ ✗), no bullet (•), no Braille, and
    // nothing in the emoji ranges — so any of those characters render
    // as tofu in the chat panel. We bundle two fonts and let
    // cosmic-text/iced_glyphon pick the right one per glyph:
    //
    //   - DejaVu Sans (primary, 739KB): covers Latin + extended,
    //     symbols, dingbats, geometric shapes, Braille — everything
    //     the tool-line icons and markdown bullets need.
    //   - Noto Color Emoji (fallback, 10MB CBDT): covers the
    //     supplementary emoji plane (U+1F300+) that Claude emits in
    //     responses. iced_glyphon 0.6 handles `SwashContent::Color`
    //     via `ContentType::Color`, so the CBDT color bitmaps render
    //     as real color emoji, not monochrome silhouettes.
    const DEJAVU_SANS: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");
    const NOTO_COLOR_EMOJI: &[u8] = include_bytes!("../fonts/NotoColorEmoji.ttf");
    iced::daemon(Launchpad::title, Launchpad::update, Launchpad::view)
        .subscription(Launchpad::subscription)
        .theme(Launchpad::theme)
        .style(Launchpad::style)
        .antialiasing(true)
        .font(DEJAVU_SANS)
        .font(NOTO_COLOR_EMOJI)
        .default_font(iced::Font::with_name("DejaVu Sans"))
        .run_with(Launchpad::new)
}
