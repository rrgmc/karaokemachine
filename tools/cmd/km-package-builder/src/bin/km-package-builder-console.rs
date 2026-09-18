//! The console executable: `km-package-builder-console`.
//!
//! Deliberately unremarkable — no attribute, no window, and the same library the windowed one runs.
//! It exists because its twin is GUI-subsystem on Windows and so cannot answer `--help`, print a
//! version, or say why it refused to start. This one can, and it serves the page in a browser exactly
//! as this tool always did. See `src/main.rs` for the half of the argument that carries the
//! attribute, and the `Two executables on Windows` decision in `docs/decisions/` for the rest.
//!
//! Built only when the `desktop` feature is on (`required-features` in `Cargo.toml`): without it the
//! other executable already *is* this one, and shipping the same program twice under two names would
//! be a folder that makes people choose between identical things.

fn main() -> anyhow::Result<()> {
    km_package_builder::run(km_package_builder::Shell::Console)
}
