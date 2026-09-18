//! The console executable: `karaokemachine-console`.
//!
//! Deliberately unremarkable — no attribute, and the same library the other one runs. It exists
//! because its twin is GUI-subsystem on Windows and so cannot answer `--help`, print a version, list
//! the audio devices, or say why it refused to start. This one can, and `--set-password` is the
//! reason it must: an admin password can be set no other way, and a flag whose output goes nowhere is
//! a flag nobody can trust they have used.
//!
//! It plays songs exactly as its twin does. **It is not a lesser build and not a diagnostic tool** —
//! it is the same machine with a console attached, which is what somebody working over SSH or from a
//! terminal wants. What it has that the other has not is somewhere to print; what the other has is no
//! black window beside the lyrics.
//!
//! See `src/main.rs` for the half of the argument that carries the attribute, and the `Two
//! executables on Windows` decision in `docs/decisions/` for the rest.
//!
//! **Built on every platform, staged only on Windows.** There is no `required-features` gate here,
//! unlike `km-package-builder`'s twin, because there is no feature that means "this build has a
//! window" — this one always does. Where there is no subsystem to choose the two executables are the
//! same program under two names, so `tools/platform/windows/dist.sh` ships the pair and the macOS bundle and
//! the Linux carriers ship one. Deciding that in the staging script rather than in cargo also keeps a
//! plain `cargo build --workspace` producing exactly what the Windows folder holds, which is the
//! property that stops a release discovering at the last step that a binary was never built.

fn main() -> anyhow::Result<()> {
    km_app::cli::main(km_app::cli::Shell::Console)
}
