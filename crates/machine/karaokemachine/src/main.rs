//! The machine: `karaokemachine`.
//!
//! Three lines of code and one attribute, and the attribute is the whole reason this file is separate
//! from `src/bin/karaokemachine-console.rs`. `#![windows_subsystem]` applies to a *binary crate root*,
//! so the two builds it chooses between cannot live in one file — and a second crate root can see
//! nothing of a `main.rs` module tree, which is why everything real is in `src/cli.rs`.
//!
//! **On Windows this is a GUI-subsystem executable.** A console-subsystem one is *given* a console by
//! Explorer before `main` runs, so the machine opened with a black window beside it that stayed for
//! the evening — under a television, next to a fullscreen lyric screen, which is where this was
//! reported from. Freeing the console cannot fix that: it is allocated at process creation, so the
//! best that could do is make a window that has already appeared go away again. The subsystem is the
//! only thing that stops it being created.
//!
//! The cost is that `--help`, `--version`, `--show-paths`, `--list-audio-devices` and
//! `--set-password` print nowhere when this one is run from a terminal with no redirection, and
//! `--set-password` is how the admin password is set from a desktop — the other way,
//! `POST /api/v1/admin/password`, needs a client already on the network. What pays it is the console
//! twin beside it. See the `Two executables on Windows` decision in `docs/decisions/`.
//!
//! **The condition is `windows` alone, with no feature under it**, and that is the one place this
//! differs from `km-package-builder`, which asks for `all(windows, feature = "desktop")`. That tool
//! has builds with no window at all, and one of those must keep talking to whoever started it. This
//! one always has a window — `--headless` is a way of running the machine, not a build of it — so
//! there is no Windows build of this executable that wants a console. macOS and Linux are left out
//! because neither has a subsystem to choose: a `.app` already has no terminal, a `.desktop` entry
//! sets `Terminal=false`, and a bare executable run from a shell still prints on both.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    km_app::cli::main(km_app::cli::Shell::Windowed)
}
