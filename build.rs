fn main() {
    // Link ServiceManagement.framework for SMAppService. Without this
    // the class loads as a forwarding stub and `+[SMAppService mainApp]`
    // raises `unrecognized selector sent to class`, crashing the app
    // when the Settings "Launch at login" checkbox is toggled.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=ServiceManagement");
}
