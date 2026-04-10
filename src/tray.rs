//! Tray icon stub. The `tray-icon` crate's event loop needs to share threads
//! with the main window event loop on macOS, which is non-trivial inside an
//! iced `Application` that hides winit from us. For the MVP we skip the tray
//! and drive everything through the global hotkey. This module exists so
//! future work can hook the tray here without changing main.rs.

pub fn init() {
    // TODO: integrate tray-icon with iced's event loop (may require a custom
    // winit event loop, or using the iced daemon/runtime APIs directly).
}
