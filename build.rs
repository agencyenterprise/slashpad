fn main() {
    // Link ServiceManagement.framework for SMAppService. Without this
    // the class loads as a forwarding stub and `+[SMAppService mainApp]`
    // raises `unrecognized selector sent to class`, crashing the app
    // when the Settings "Launch at login" checkbox is toggled.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=ServiceManagement");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=objc");

        // Compile the login-item Obj-C shim. Doing the @try/@catch in
        // real Obj-C avoids relying on objc2's Rust-side exception
        // catch, which has been unreliable under LTO + strip in
        // release builds.
        println!("cargo:rerun-if-changed=src/platform/login_item.m");
        cc::Build::new()
            .file("src/platform/login_item.m")
            .flag("-fobjc-arc")
            .flag("-fobjc-exceptions")
            .compile("slashpad_login_item");
    }
}
