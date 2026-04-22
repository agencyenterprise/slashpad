//! macOS platform bits: activation policy, NSPanel conversion, cursor monitor
//! lookup. Ported from `src-tauri/src/lib.rs` but using `objc2` instead of
//! the older `cocoa` + `objc` crates.
//!
//! NSPanel wrapping runs post-creation against an existing NSWindow pointer.
//! iced creates its window through winit, which we can't intercept cleanly —
//! so we set the style mask/level/collection behavior on the active window
//! after iced hands control back to us.

#![allow(non_upper_case_globals)]

use std::ffi::c_void;
use std::sync::OnceLock;

use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSColor, NSScreen, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Dispatches the closure onto the main thread via libdispatch's
/// `dispatch_async_f(dispatch_get_main_queue, ...)`. Needed for NSPanel /
/// NSWindow ops which crash when run on a background thread.
pub fn dispatch_main_async<F: FnOnce() + Send + 'static>(f: F) {
    extern "C" {
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
        static _dispatch_main_q: c_void;
    }

    extern "C" fn trampoline<F: FnOnce()>(context: *mut c_void) {
        let boxed: Box<F> = unsafe { Box::from_raw(context as *mut F) };
        (*boxed)();
    }

    let boxed: Box<F> = Box::new(f);
    let ctx = Box::into_raw(boxed) as *mut c_void;
    unsafe {
        let main_q: *const c_void = &_dispatch_main_q as *const _;
        dispatch_async_f(main_q, ctx, trampoline::<F>);
    }
}

/// `NSModalPanelWindowLevel` raw value — used everywhere for floating palettes.
pub const NS_MODAL_PANEL_WINDOW_LEVEL: i32 = 8;

/// Switch the app to an accessory activation policy: no Dock icon, no menu
/// bar, app only visible through the palette + tray. Must run on the main
/// thread.
pub fn set_accessory_activation_policy() {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[platform/macos] set_accessory_activation_policy called off main thread");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

/// Activate the app and bring it to the front. Needed before showing the
/// palette so it can become the key window immediately.
pub fn activate_ignoring_other_apps() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// Lazily register a custom `SlashpadPanel` subclass of `NSPanel` that
/// overrides `canBecomeKeyWindow` and `canBecomeMainWindow` to return `YES`.
///
/// Why a subclass: a borderless `NSPanel` with `NSWindowStyleMaskNonactivatingPanel`
/// has `canBecomeKeyWindow` returning `NO` in several edge cases (no title bar,
/// fresh class swap against a non-panel base, …), which prevents keyboard
/// events from reaching the first responder chain. Overriding the two
/// methods to unconditionally return `YES` is what `tauri-nspanel` does
/// internally via its `tauri_panel!` macro.
fn slashpad_panel_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = AnyClass::get("NSPanel").expect("NSPanel class must exist");
        let mut builder = ClassBuilder::new("SlashpadPanel", superclass)
            .expect("SlashpadPanel class was already registered");

        // Use `*mut AnyObject` (not `&AnyObject`) as the receiver: objc2 0.5
        // only implements `MessageReceiver` for `&'a T` with a *specific*
        // lifetime, but `add_method` needs the fn pointer type without
        // higher-rank lifetimes. Raw pointers have no lifetime and therefore
        // no HRTB coercion problem.
        extern "C" fn yes(_this: *mut AnyObject, _cmd: Sel) -> Bool {
            Bool::YES
        }

        unsafe {
            builder.add_method(
                sel!(canBecomeKeyWindow),
                yes as extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
            builder.add_method(
                sel!(canBecomeMainWindow),
                yes as extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
        }

        builder.register()
    })
}

/// Swap the Objective-C class of an `NSWindow` pointer to our custom
/// `SlashpadPanel` subclass of `NSPanel`.
///
/// iced/winit creates a plain `NSWindow`; NSPanel-only behaviors
/// (`NSWindowStyleMaskNonactivatingPanel`, floating over full-screen apps)
/// are silently ignored on non-NSPanel instances. `object_setClass` from
/// the Objective-C runtime reinterprets the receiver without moving it in
/// memory; NSPanel shares NSWindow's ivar layout so this is safe, and the
/// operation is idempotent (swapping to the same class is a no-op).
///
/// Safety: `ns_window_ptr` must be a valid live Cocoa window pointer.
unsafe fn convert_nswindow_to_slashpad_panel(ns_window_ptr: *mut c_void) {
    extern "C" {
        fn object_setClass(obj: *mut AnyObject, cls: *const AnyClass) -> *const AnyClass;
    }
    let panel_cls = slashpad_panel_class();
    let _prev = object_setClass(
        ns_window_ptr as *mut AnyObject,
        panel_cls as *const AnyClass,
    );
}

/// Minimal NSPanel treatment for the settings window: swap the Objective-C
/// class to `SlashpadPanel` so `canBecomeKeyWindow` / `canBecomeMainWindow`
/// return `YES`, and nothing else. We explicitly avoid touching the style
/// mask / background color / collection behavior because those paths fire
/// AppKit redraw notifications that have crashed the settings-window open
/// flow repeatedly (the palette is safer because it's created with
/// `visible: false`, so there are no live observers when we class-swap).
///
/// Without this, a borderless iced window can't become key at all on
/// macOS, which means the user can't type into the API-key field AND
/// the window never fires an `Unfocused` event on click-outside — so
/// the blur-close path never triggers.
///
/// Safety: `ns_window_ptr` must be a valid `NSWindow *`.
pub unsafe fn make_window_key_capable(ns_window_ptr: *mut c_void) {
    if ns_window_ptr.is_null() {
        return;
    }
    convert_nswindow_to_slashpad_panel(ns_window_ptr);
}

/// Apply the palette-style floating panel treatment to a raw `NSWindow*`
/// pointer: convert the underlying class to `NSPanel`, set the non-activating
/// style mask, the modal panel window level, and collection behavior that lets
/// it join all spaces + float over full-screen windows.
///
/// Safety: `ns_window_ptr` must be a valid `NSWindow *`. The objc2 calls are
/// all non-null-dereferencing but we rely on the pointer being a live Cocoa
/// object.
pub unsafe fn apply_palette_style(ns_window_ptr: *mut c_void) {
    if ns_window_ptr.is_null() {
        return;
    }

    // First, swap the underlying class from NSWindow to our SlashpadPanel
    // subclass of NSPanel so panel-only behaviors (NonactivatingPanel style,
    // float-over-fullscreen, canBecomeKey override) actually take effect.
    // This must happen before `setStyleMask` for NonactivatingPanel to be
    // honored.
    convert_nswindow_to_slashpad_panel(ns_window_ptr);

    let window: &NSWindow = &*ns_window_ptr.cast::<NSWindow>();

    // Non-activating panel bit (NSWindowStyleMaskNonactivatingPanel = 1 << 7)
    let current_mask: NSWindowStyleMask = window.styleMask();
    let nonactivating = NSWindowStyleMask::from_bits_retain(1 << 7);
    window.setStyleMask(current_mask | nonactivating);

    // NSModalPanelWindowLevel
    window.setLevel(NS_MODAL_PANEL_WINDOW_LEVEL as isize);

    // Keep the palette visible when the app deactivates (e.g. focus hops back
    // to the previous app). Without this the NSPanel hides itself on
    // deactivation, which defeats the float-over-fullscreen behavior.
    window.setHidesOnDeactivate(false);

    // canJoinAllSpaces (1 << 0) | ignoresCycle (1 << 6) | fullScreenAuxiliary (1 << 8)
    let behavior_bits: usize = (1 << 0) | (1 << 6) | (1 << 8);
    let behavior = NSWindowCollectionBehavior::from_bits_retain(behavior_bits);
    window.setCollectionBehavior(behavior);

    // Transparent background so rounded corners + transparent iced theme show through.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let clear = NSColor::colorWithRed_green_blue_alpha(0.0, 0.0, 0.0, 0.0);
    let _ = mtm; // silence unused
    window.setBackgroundColor(Some(&clear));
    window.setOpaque(false);
    window.setHasShadow(false);
}

/// Hide a window by calling `orderOut:` on the NSWindow pointer.
pub unsafe fn order_out(ns_window_ptr: *mut c_void) {
    if ns_window_ptr.is_null() {
        return;
    }
    let window: &NSWindow = &*ns_window_ptr.cast::<NSWindow>();
    window.orderOut(None);
}

/// Show + make key a window by its NSWindow pointer.
pub unsafe fn order_front_and_make_key(ns_window_ptr: *mut c_void) {
    if ns_window_ptr.is_null() {
        return;
    }
    let window: &NSWindow = &*ns_window_ptr.cast::<NSWindow>();
    window.orderFrontRegardless();
    window.makeKeyWindow();
    activate_ignoring_other_apps();
}

/// Return the (x, y, width, height) in screen points of whichever screen the
/// cursor is currently on, or the main screen as a fallback. Coordinates are
/// in AppKit's bottom-left-origin space.
pub fn cursor_monitor_frame() -> Option<(f64, f64, f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let mouse = unsafe { mouse_location() };
    let screens = NSScreen::screens(mtm);
    for i in 0..screens.count() {
        let screen = unsafe { screens.objectAtIndex(i) };
        let frame: NSRect = screen.frame();
        if mouse.x >= frame.origin.x
            && mouse.x < frame.origin.x + frame.size.width
            && mouse.y >= frame.origin.y
            && mouse.y < frame.origin.y + frame.size.height
        {
            return Some((
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ));
        }
    }
    let main = NSScreen::mainScreen(mtm)?;
    let frame: NSRect = main.frame();
    Some((
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    ))
}

/// Fingerprint of the screen the cursor is currently on, used to key
/// per-screen palette drag memory. Returns `None` only if AppKit can't
/// be queried (not on the main thread, or no screens attached).
pub fn cursor_screen_key() -> Option<crate::state::ScreenKey> {
    let (x, y, w, h) = cursor_monitor_frame()?;
    Some(crate::state::ScreenKey {
        x: x.round() as i32,
        y: y.round() as i32,
        w: w.round() as i32,
        h: h.round() as i32,
    })
}

/// Given a point in iced/winit's top-left-origin coordinate space
/// (relative to the primary monitor), return the `ScreenKey` of the
/// NSScreen whose frame contains it. Used by `WindowMoved` to attribute
/// a user drag to the correct screen — dragging the palette across
/// monitors updates only the destination screen's remembered position.
pub fn screen_key_for_point(point: iced::Point) -> Option<crate::state::ScreenKey> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    if screens.count() == 0 {
        return None;
    }
    let primary: Retained<NSScreen> = unsafe { screens.objectAtIndex(0) };
    let primary_h = primary.frame().size.height;
    // Convert top-left iced coords back to NS bottom-left.
    let ns_x = point.x as f64;
    let ns_y = primary_h - point.y as f64;
    for i in 0..screens.count() {
        let screen = unsafe { screens.objectAtIndex(i) };
        let frame: NSRect = screen.frame();
        if ns_x >= frame.origin.x
            && ns_x < frame.origin.x + frame.size.width
            && ns_y >= frame.origin.y
            && ns_y < frame.origin.y + frame.size.height
        {
            return Some(crate::state::ScreenKey {
                x: frame.origin.x.round() as i32,
                y: frame.origin.y.round() as i32,
                w: frame.size.width.round() as i32,
                h: frame.size.height.round() as i32,
            });
        }
    }
    None
}

/// Top-left coordinate (in iced/winit logical points, anchored to the primary
/// monitor) at which to place a window of the given `width` so that it is
/// horizontally centered on the cursor's current monitor and offset ~20% from
/// the top of that monitor.
///
/// NSScreen uses a bottom-left-origin global coordinate system; iced/winit
/// uses a top-left-origin system rooted at the primary monitor (the one
/// containing the menu bar, always index 0 in `NSScreen::screens`). This
/// helper does that conversion.
pub fn cursor_palette_position(width: f64) -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    if screens.count() == 0 {
        return None;
    }
    // Index 0 is the primary screen (contains the menu bar).
    let primary: Retained<NSScreen> = unsafe { screens.objectAtIndex(0) };
    let primary_h = primary.frame().size.height;

    let (ns_x, ns_y, ns_w, ns_h) = cursor_monitor_frame()?;
    let target_x = ns_x + (ns_w - width) / 2.0;
    // Convert NS bottom-left y to winit top-left y relative to primary,
    // then drop 20% of the target screen's height as the top offset.
    let target_y = primary_h - ns_y - ns_h + ns_h * 0.20;
    Some((target_x, target_y))
}

/// NSEvent.mouseLocation — global cursor position in AppKit coordinates.
/// objc2-app-kit doesn't expose a safe binding for this class method directly,
/// so we use a raw objc2 message send.
unsafe fn mouse_location() -> NSPoint {
    let cls = AnyClass::get("NSEvent").expect("NSEvent class should exist");
    let pt: NSPoint = msg_send![cls, mouseLocation];
    pt
}

/// Extract the `NSWindow *` pointer from an iced `WindowHandle`. Used with
/// `iced::window::run_with_handle(id, |handle| ...)` so we can apply NSPanel
/// treatment to a specific iced window by id, rather than guessing via
/// `NSApp.windows()` (which breaks in multi-window setups because the tray
/// icon's `NSStatusBarWindow` and both of our own windows all show up).
///
/// Returns null if the handle isn't an AppKit one (shouldn't happen on macOS).
///
/// Safety: `handle` must refer to a live winit window. The returned pointer
/// is only valid while that window is alive; callers should finish whatever
/// NSWindow ops they need before the closure running this returns.
pub unsafe fn ns_window_from_handle<H: HasWindowHandle>(handle: &H) -> *mut c_void {
    let Ok(window_handle) = handle.window_handle() else {
        return std::ptr::null_mut();
    };
    let RawWindowHandle::AppKit(appkit) = window_handle.as_raw() else {
        return std::ptr::null_mut();
    };
    // `handle.ns_view` is a `NonNull<c_void>` pointing at an NSView. Ask it
    // for its parent NSWindow via `[nsView window]`.
    let view_ptr: *mut AnyObject = appkit.ns_view.as_ptr().cast();
    let window_ptr: *mut AnyObject = msg_send![view_ptr, window];
    window_ptr as *mut c_void
}

/// Backing scale factor of the primary screen (menu-bar screen). Used to
/// convert `tray-icon`'s physical-pixel `Rect` into iced/winit logical
/// coordinates. Returns `2.0` as a Retina-typical fallback if the primary
/// screen can't be resolved (extremely unlikely inside a live app).
pub fn primary_scale_factor() -> f64 {
    let Some(mtm) = MainThreadMarker::new() else {
        return 2.0;
    };
    NSScreen::mainScreen(mtm)
        .map(|screen| screen.backingScaleFactor())
        .unwrap_or(2.0)
}

/// Retrieve the NSWindow pointer for iced's palette window.
///
/// Iterates `NSApp.windows()` and returns the first entry whose Objective-C
/// class name ends in `WinitWindow` (iced/winit's window class) or
/// `SlashpadPanel` (our custom NSPanel subclass, after the class swap).
///
/// Two subtleties:
///
/// 1. **KVO dynamic subclasses**: when something registers a KVO observer on
///    the winit window, Cocoa swaps the instance's class to an automatically-
///    generated subclass named `NSKVONotifying_WinitWindow` (prefix added by
///    AppKit). An exact-match check for `"WinitWindow"` misses this. We match
///    on the suffix instead — the original class name is always at the end.
///
/// 2. **tray-icon's NSStatusBarWindow**: the `tray-icon` crate adds an
///    `NSStatusBarWindow` to `NSApp.windows()` as soon as the menu-bar tray is
///    created. That window is a private AppKit NSWindow subclass; swapping
///    its class to `SlashpadPanel` + calling `setBackgroundColor` crashes
///    AppKit with a `viewNeedsDisplayInRectNotification:` unrecognized-selector
///    exception. The suffix filter naturally skips it.
///
/// Returns null if iced's window hasn't been created yet (callers already
/// handle null — `apply_palette_style` is a no-op on a null pointer, and the
/// self-healing `show_palette()` path re-runs the style application on every
/// hotkey press).
pub unsafe fn first_app_window_ptr() -> *mut c_void {
    extern "C" {
        fn object_getClassName(obj: *const AnyObject) -> *const i8;
    }

    let Some(mtm) = MainThreadMarker::new() else {
        return std::ptr::null_mut();
    };
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    let count = windows.count();
    for i in 0..count {
        let window: Retained<NSWindow> = unsafe { windows.objectAtIndex(i) };
        let obj_ptr = (&*window as *const NSWindow) as *const AnyObject;
        let name_ptr = object_getClassName(obj_ptr);
        if name_ptr.is_null() {
            continue;
        }
        let name_bytes = std::ffi::CStr::from_ptr(name_ptr).to_bytes();
        let Ok(name_str) = std::str::from_utf8(name_bytes) else {
            continue;
        };
        if name_str.ends_with("WinitWindow") || name_str.ends_with("SlashpadPanel") {
            let ptr: *const NSWindow = &*window;
            return ptr as *mut c_void;
        }
    }
    std::ptr::null_mut()
}

// ── Launch at login (user LaunchAgent) ───────────────────────────
//
// Rather than depend on SMAppService — whose +mainApp selector turned
// out to be missing on at least one macOS runtime we ship to — we
// register a user LaunchAgent plist at
// `~/Library/LaunchAgents/dev.slashpad.LoginAgent.plist`. launchd
// loads every plist in that directory at login and, with
// `RunAtLoad=true`, runs the ProgramArguments command. We invoke
// `/usr/bin/open -a <bundle>` so LaunchServices handles activation
// policy and .app lifecycle correctly.
//
// No Obj-C or framework dependency — just filesystem writes.

const LOGIN_AGENT_LABEL: &str = "dev.slashpad.LoginAgent";

fn app_bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    // Expect .../Foo.app/Contents/MacOS/<binary> — walk back to the .app.
    let app = exe.parent()?.parent()?.parent()?.to_path_buf();
    if app
        .extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("app"))
        .unwrap_or(false)
    {
        Some(app)
    } else {
        None
    }
}

fn launch_agent_plist_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LOGIN_AGENT_LABEL}.plist")),
    )
}

/// True when this build of slashpad supports the Launch-at-login
/// toggle. Requires the running binary to be inside an `.app` bundle
/// — otherwise we have nothing stable to point the plist at.
pub fn login_item_supported() -> bool {
    app_bundle_path().is_some() && launch_agent_plist_path().is_some()
}

/// Register the app as a Login Item by writing a user LaunchAgent
/// plist. launchd picks it up at next login via RunAtLoad.
pub fn register_login_item() -> bool {
    let Some(bundle) = app_bundle_path() else {
        eprintln!("[platform/macos] register_login_item: no .app bundle path resolvable");
        return false;
    };
    let Some(plist_path) = launch_agent_plist_path() else {
        eprintln!("[platform/macos] register_login_item: no HOME");
        return false;
    };
    if let Some(parent) = plist_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "[platform/macos] failed to create {}: {e}",
                parent.display()
            );
            return false;
        }
    }
    let bundle_str = bundle.display().to_string();
    // `open -a <bundle>` routes through LaunchServices so the app
    // comes up with its normal activation policy / NSApplicationMain.
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>-a</string>
        <string>{bundle_xml}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        label = LOGIN_AGENT_LABEL,
        bundle_xml = xml_escape(&bundle_str),
    );
    if let Err(e) = std::fs::write(&plist_path, plist) {
        eprintln!(
            "[platform/macos] failed to write {}: {e}",
            plist_path.display()
        );
        return false;
    }
    eprintln!(
        "[platform/macos] wrote LaunchAgent plist at {}",
        plist_path.display()
    );
    true
}

/// Unregister the Login Item by removing the plist.
pub fn unregister_login_item() -> bool {
    let Some(plist_path) = launch_agent_plist_path() else {
        return false;
    };
    if !plist_path.exists() {
        return true;
    }
    match std::fs::remove_file(&plist_path) {
        Ok(()) => {
            eprintln!(
                "[platform/macos] removed LaunchAgent plist at {}",
                plist_path.display()
            );
            true
        }
        Err(e) => {
            eprintln!(
                "[platform/macos] failed to remove {}: {e}",
                plist_path.display()
            );
            false
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Shim that suppresses an unused import on non-macos targets.
#[allow(dead_code)]
fn _unused(_: &AnyObject) {}
