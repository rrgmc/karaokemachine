//! The windowed executable: `km-package-builder`.
//!
//! Three lines of code and one attribute, and the attribute is the whole reason this file is separate
//! from `src/bin/km-package-builder-console.rs`. `#![windows_subsystem]` applies to a *binary crate
//! root*, so the two builds it chooses between cannot live in one file — and a second crate root can
//! see nothing of a `main.rs` module tree, which is why everything real is in `src/lib.rs`.
//!
//! **On Windows, with a window, this is a GUI-subsystem executable.** A console-subsystem one is
//! *given* a console by Explorer before `main` runs, and the black window that flashes past on the
//! way to the real one is exactly what the double-click path exists to avoid. The cost is that
//! `--help`, `--version` and every startup line print nowhere when this is run from a terminal, and
//! what pays it is the console twin beside it. See the `Two executables on Windows` decision in
//! `docs/decisions/distribution.md`.
//!
//! The condition is narrow on purpose. Without the `desktop` feature there is no window, so this is
//! the browser-served tool and must keep talking to whoever started it. And macOS is left out because
//! it has no subsystem to choose: a bundled `.app` already has no terminal, a bare executable run
//! from Terminal still prints, and there is no flash on either path.
#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    km_package_builder::run(km_package_builder::Shell::Windowed)
}
