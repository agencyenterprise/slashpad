#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(not(target_os = "macos"))]
pub mod stub;

#[cfg(not(target_os = "macos"))]
pub use stub as macos;
