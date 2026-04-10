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

/// Lazily register a custom `LaunchpadPanel` subclass of `NSPanel` that
/// overrides `canBecomeKeyWindow` and `canBecomeMainWindow` to return `YES`.
///
/// Why a subclass: a borderless `NSPanel` with `NSWindowStyleMaskNonactivatingPanel`
/// has `canBecomeKeyWindow` returning `NO` in several edge cases (no title bar,
/// fresh class swap against a non-panel base, …), which prevents keyboard
/// events from reaching the first responder chain. Overriding the two
/// methods to unconditionally return `YES` is what `tauri-nspanel` does
/// internally via its `tauri_panel!` macro.
fn launchpad_panel_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = AnyClass::get("NSPanel").expect("NSPanel class must exist");
        let mut builder = ClassBuilder::new("LaunchpadPanel", superclass)
            .expect("LaunchpadPanel class was already registered");

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
/// `LaunchpadPanel` subclass of `NSPanel`.
///
/// iced/winit creates a plain `NSWindow`; NSPanel-only behaviors
/// (`NSWindowStyleMaskNonactivatingPanel`, floating over full-screen apps)
/// are silently ignored on non-NSPanel instances. `object_setClass` from
/// the Objective-C runtime reinterprets the receiver without moving it in
/// memory; NSPanel shares NSWindow's ivar layout so this is safe, and the
/// operation is idempotent (swapping to the same class is a no-op).
///
/// Safety: `ns_window_ptr` must be a valid live Cocoa window pointer.
unsafe fn convert_nswindow_to_launchpad_panel(ns_window_ptr: *mut c_void) {
    extern "C" {
        fn object_setClass(obj: *mut AnyObject, cls: *const AnyClass) -> *const AnyClass;
    }
    let panel_cls = launchpad_panel_class();
    let _prev = object_setClass(
        ns_window_ptr as *mut AnyObject,
        panel_cls as *const AnyClass,
    );
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

    // First, swap the underlying class from NSWindow to our LaunchpadPanel
    // subclass of NSPanel so panel-only behaviors (NonactivatingPanel style,
    // float-over-fullscreen, canBecomeKey override) actually take effect.
    // This must happen before `setStyleMask` for NonactivatingPanel to be
    // honored.
    convert_nswindow_to_launchpad_panel(ns_window_ptr);

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

/// Best-effort retrieval of the first NSWindow owned by the current app.
/// iced + winit create exactly one window for our use case, so the first
/// entry in `NSApp.windows` is the palette window.
pub unsafe fn first_app_window_ptr() -> *mut c_void {
    let Some(mtm) = MainThreadMarker::new() else {
        return std::ptr::null_mut();
    };
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    if windows.count() == 0 {
        return std::ptr::null_mut();
    }
    let window: Retained<NSWindow> = unsafe { windows.objectAtIndex(0) };
    let ptr: *const NSWindow = &*window;
    ptr as *mut c_void
}

/// Shim that suppresses an unused import on non-macos targets.
#[allow(dead_code)]
fn _unused(_: &AnyObject) {}
