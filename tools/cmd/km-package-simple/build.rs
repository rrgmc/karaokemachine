//! Attaches the icon and the version information to the Windows executable.
//!
//! The same arrangement as `km-package-builder`'s `build.rs`, which says why the `#[cfg]` and the
//! target check are two different questions.

fn main() {
    #[cfg(windows)]
    attach_icon();
}

#[cfg(windows)]
fn attach_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // The builder's blue with a bolt. `crates/playback/km-display/examples/icon.rs` draws it.
    println!("cargo:rerun-if-changed=../../../icon/km-package-simple.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../../icon/km-package-simple.ico");
    resource.set("ProductName", "KaraokeMachine");
    resource.set("FileDescription", "KaraokeMachine Simple Package Builder");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not attach the application icon: {error}");
        println!(
            "cargo:warning=km-package-simple will build and run with Windows' default icon and \
             no version information. This needs rc.exe from the Windows SDK."
        );
    }
}
