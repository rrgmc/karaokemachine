//! The windowed executable: `km-package-simple`.
//!
//! The attribute is why this file is separate from `src/bin/km-package-simple-console.rs`:
//! `#![windows_subsystem]` applies to a binary crate root, and everything real is in `src/lib.rs`.
//! `km-package-builder`'s `main.rs` states the whole argument, and the `Two executables on Windows`
//! decision in `docs/decisions/distribution.md` is the rule.
#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    km_package_simple::run(km_package_simple::Shell::Windowed)
}
