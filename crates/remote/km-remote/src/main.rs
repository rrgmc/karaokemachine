//! The windowed executable: `km-remote`.
//!
//! Three lines and one attribute, and the attribute is the whole reason this file is separate from
//! `src/bin/km-remote-console.rs`. `#![windows_subsystem]` is a property of a **binary crate
//! root**, so the only way to have one executable that opens no console and one that does is to
//! have two crate roots over one library. See the `Two executables on Windows` decision in
//! docs/decisions/distribution.md.
//!
//! **The condition is narrow on purpose.** Without the `desktop` feature this is still the
//! browser-served remote it has always been, and such a build must be able to print; and macOS and
//! Linux have no subsystem to choose, so naming one there would be a promise the platform does not
//! keep.

#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    km_remote::run(km_remote::Shell::Windowed)
}
