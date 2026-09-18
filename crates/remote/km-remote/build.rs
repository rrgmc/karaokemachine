//! Puts the application icon and version information inside the Windows executable.
//!
//! The same job, the same way, as `crates/machine/karaokemachine/build.rs` and `tools/cmd/km-package-builder/build.rs`
//! — see the longer explanation in the first for why this is the only way Explorer and the taskbar
//! get an icon before the process starts, and why a missing resource compiler is a warning rather
//! than a failure. Duplicated rather than shared because a build script cannot be reached from
//! another crate without inventing a crate to hold it, and it is twenty lines.
//!
//! The remote carries the **same microphone in a different color** — the theme's second accent,
//! green — on exactly the reasoning the curation tool's blue one already had, and which this crate
//! spent two milestones being the exception to. It used to take `karaokemachine.ico` itself, on the
//! grounds that it is the same product; so is the package builder, and the argument that settles it
//! is not about products but about taskbars. This is the one program here that a singer runs *beside*
//! the machine, so two identical icons is two windows nobody can tell apart. `FileDescription` does
//! not help there: nothing shows it until you hover.
//!
//! What it does hold is this program's **name** — the string its window title already carries —
//! rather than the crate's `description`, which is a sentence written for `cargo`. The machine's
//! build script has the longer form, and this is one of the copies listed in `What the tool calls
//! itself` in `docs/decisions/foundations.md`.

fn main() {
    #[cfg(windows)]
    attach_icon();
}

#[cfg(windows)]
fn attach_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=../../../icon/km-remote.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../../icon/km-remote.ico");
    resource.set("ProductName", "KaraokeMachine");
    resource.set("FileDescription", "KaraokeMachine Remote");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not attach the application icon: {error}");
        println!(
            "cargo:warning=km-remote will build and run with Windows' default icon and no \
             version information. This needs rc.exe from the Windows SDK."
        );
    }
}
