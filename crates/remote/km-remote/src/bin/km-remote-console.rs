//! The console executable: `km-remote-console`.
//!
//! The same library as `km-remote`, with no `#![windows_subsystem]` attribute and therefore a
//! console. **It is not a lesser build**: what it has that the other cannot is the ability to answer
//! `--help`, print a version, and say why it refused to start — a GUI-subsystem executable can do
//! none of those, because there is nowhere for the text to go.
//!
//! It exists only where there is a window to be the alternative to, which is why the manifest gates
//! it on `required-features = ["desktop"]`. Without that feature the executable beside it already
//! is this one, and shipping the same program twice under two names is a choice nobody could make
//! correctly.

fn main() -> anyhow::Result<()> {
    km_remote::run(km_remote::Shell::Console)
}
