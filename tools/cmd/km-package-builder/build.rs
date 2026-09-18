//! Puts the application icon and version information inside the Windows executable.
//!
//! The same job, the same way, as `crates/machine/karaokemachine/build.rs` — see the longer explanation there for why
//! this is the only way Explorer and the taskbar get an icon before the process starts, and why a
//! missing resource compiler is a warning rather than a failure. Duplicated rather than shared
//! because a build script cannot be reached from another crate without inventing a crate to hold it,
//! and it is twenty lines.
//!
//! The curation tool carries the **same microphone in a different color** — the theme's accent blue
//! where the machine has its sung-lyric amber and the offline remote its second accent green.
//! `crates/playback/km-display/examples/icon.rs` renders all three from one drawing, so there is no second
//! design to keep in step.
//!
//! **The argument for one mark shared unchanged — that "two marks would say there are two
//! products" — has a right premise and does not follow.** These two programs are run *side by side
//! on one desktop*, which the machine's other platforms never are, and there two windows, two
//! taskbar buttons and two Explorer entries wearing an identical icon are simply not tellable apart
//! — the icon stops doing the one job an icon has. One shape in two colors keeps the premise:
//! nobody looking at them doubts they are the same product, and nobody has to read a title bar to
//! know which one they are clicking.
//!
//! Before this existed the executable had no resource at all, so Windows reported its version as
//! blank and its icon as the generic one.
//!
//! `ProductName` is the same string as in `crates/machine/karaokemachine/build.rs`, on purpose: the machine and this
//! are two programs in one product, which is the distinction Windows keeps `ProductName` and
//! `FileDescription` apart for. `FileDescription` is what separates them, and it is this program's
//! **name** — the string its window title, its macOS application menu and its tray tooltip already
//! carry — rather than the crate's `description`, which is a sentence written for `cargo`. Neither
//! field is picked up automatically: `winresource` defaults both to the crate name. The machine's
//! build script has the longer form, and this is one of the copies listed in `What the tool calls
//! itself` in `docs/decisions/foundations.md`.

fn main() {
    // Host and target are two questions here; `crates/machine/karaokemachine/build.rs` explains both. In short: the
    // `#[cfg]` is what keeps this compiling where `winresource` is not in the build, and the check
    // below is what keeps a resource compiler away from a non-Windows target.
    #[cfg(windows)]
    attach_icon();
}

#[cfg(windows)]
fn attach_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=../../../icon/km-package-builder.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../../icon/km-package-builder.ico");
    resource.set("ProductName", "KaraokeMachine");
    resource.set("FileDescription", "KaraokeMachine Package Builder");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not attach the application icon: {error}");
        println!(
            "cargo:warning=km-package-builder will build and run with Windows' default icon and \
             no version information. This needs rc.exe from the Windows SDK."
        );
    }
}
