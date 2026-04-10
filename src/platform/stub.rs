//! No-op platform stubs for non-macOS targets. The current MVP targets macOS
//! because NSPanel-style overlays are core to the UX, but the stubs keep the
//! crate compiling on linux/windows so cargo check works cross-platform.

use std::ffi::c_void;

pub fn set_accessory_activation_policy() {}

pub fn dispatch_main_async<F: FnOnce() + Send + 'static>(_f: F) {}

pub unsafe fn first_app_window_ptr() -> *mut c_void {
    std::ptr::null_mut()
}

pub unsafe fn apply_palette_style(_ptr: *mut c_void) {}

pub unsafe fn order_out(_ptr: *mut c_void) {}

pub unsafe fn order_front_and_make_key(_ptr: *mut c_void) {}
