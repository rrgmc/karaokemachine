//! Puts the application icon and version information inside the Windows executable.
//!
//! The same job, the same way, as `crates/machine/karaokemachine/build.rs` and
//! `tools/cmd/km-package-builder/build.rs`. Duplicated rather than shared because a build script
//! cannot be reached from another crate without inventing a crate to hold it, and it is twenty
//! lines. See the machine's for the longer explanation of why this is the only way Explorer and the
//! taskbar get an icon before the process starts, and why a missing resource compiler is a warning
//! rather than a failure.
//!
//! **The fourth mark, led by the theme's magenta.** One drawing under four palettes now — amber for
//! the machine, blue for the package builder, green for the offline remote, magenta for this — with
//! every color still out of `km_display::theme::Theme`. The magenta is `icon_glow` lifted 12%
//! toward white, because straight it gave the M 4.00:1 against the plate under a 4.5:1 floor; see
//! `admin_lead` in `crates/playback/km-display/examples/icon.rs`.
//!
//! Carrying the machine's `.ico` instead is the one thing that must not happen. These programs are
//! run side by side on one desktop, and two taskbar buttons wearing an identical icon are not
//! tellable apart — which is the whole argument that produced four palettes rather than one.
//!
//! `ProductName` is the same string as in the other three on purpose: these are programs in one
//! product, which is the distinction Windows keeps `ProductName` and `FileDescription` apart for.
//! `FileDescription` is what separates them, and it is this program's **name** — the string its
//! pages, its window title and its tray tooltip already carry — rather than the crate's
//! `description`, which is a sentence written for `cargo`. Neither field is picked up
//! automatically: `winresource` defaults both to the crate name. The machine's build script has the
//! longer form, and this is one of the copies listed in `What the tool calls itself` in
//! `docs/decisions/foundations.md`.

fn main() {
    // Host and target are two questions here. The `#[cfg]` is what keeps this compiling where
    // `winresource` is not in the build; the check inside is what keeps a resource compiler away
    // from a non-Windows target.
    #[cfg(windows)]
    attach_resources();
}

#[cfg(windows)]
fn attach_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=../../../../icon/km-admin.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../../../icon/km-admin.ico");
    resource.set("ProductName", "KaraokeMachine");
    resource.set("FileDescription", "KaraokeMachine Admin");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not attach the application icon: {error}");
        println!(
            "cargo:warning=km-admin will build and run with Windows' default icon and no version \
             information. This needs rc.exe from the Windows SDK."
        );
    }
}
