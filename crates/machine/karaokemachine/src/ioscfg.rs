//! The two directories an iOS application is given, rather than the two it could derive.
//!
//! **The shell creates them and hands them down; nothing here guesses.** `directories` ships no iOS
//! module, and the crate a desktop uses would not merely answer imprecisely but answer with a macOS
//! path outside the container: `~/Library/Application Support/karaokemachine` resolves, and a
//! sandboxed application cannot write to it. The offline remote settled the same question the same
//! way, and [`Where an iPhone keeps its favorites`](../../../docs/decisions/remotes.md) is the entry
//! that argues it.
//!
//! **A static, because `SDL_main` takes no arguments.** SDL calls it with an argv it invented, so the
//! only way from `application(_:didFinishLaunchingWithOptions:)` to [`crate::settings::Paths`] is a
//! value published before SDL starts. That is the arrangement `androidctx` already has for cpal's
//! JavaVM, and for the same reason.
//!
//! Which directory is which:
//!
//! - **Application Support** holds `settings.json`, the catalog and the wallpapers. Backed up, and
//!   invisible to the person, which is right for a database this device can rebuild but a
//!   collection it cannot.
//! - **Documents** is what `UIFileSharingEnabled` exposes in the Files app and in Finder over USB,
//!   so `Documents/packages` is where a `.kmpkg` arrives by hand. It is the counterpart of Android's
//!   public external directory, and `packages_dirs()` already reads it as one.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The directories, once the shell has named them.
static DIRS: OnceLock<Dirs> = OnceLock::new();

/// Where an iOS application keeps its own two kinds of thing.
#[derive(Debug, Clone)]
pub(crate) struct Dirs {
    /// `Library/Application Support/…`: settings, the catalog, the wallpapers.
    pub(crate) support: PathBuf,
    /// `Documents/`: the folder a person can see and drop a package into.
    pub(crate) documents: PathBuf,
}

/// Records the two directories the application container gives us.
///
/// Called from `km_machine_configure` before SDL starts. A second call is ignored rather than a
/// panic: the value is the container's and cannot legitimately change within a run, and aborting an
/// application over a duplicated handoff would be a worse answer than keeping the first one.
pub fn publish(support: impl AsRef<Path>, documents: impl AsRef<Path>) {
    let dirs = Dirs {
        support: support.as_ref().to_path_buf(),
        documents: documents.as_ref().to_path_buf(),
    };
    if DIRS.set(dirs).is_err() {
        tracing::warn!(
            "the container's directories were handed down twice; keeping the first pair"
        );
    }
}

/// Reports a panic through `tracing`, so the tap and the console both see it.
///
/// **Chained rather than replacing**, so the default hook still prints and a debugger still stops.
/// The reason it is needed at all is that `km-machine-ios` catches its own panics at the FFI
/// boundary: an `extern "C"` function that lets one unwind aborts the process, so every export wraps
/// its body, and without this hook a panic inside one would be reported by nothing.
///
/// A backtrace is forced rather than left to `RUST_BACKTRACE`, which nothing sets on a phone.
pub(crate) fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            panic = %info,
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "the machine panicked"
        );
        previous(info);
    }));
}

/// The directories, or `None` where nothing published them.
///
/// `None` is what a unit test sees, and what an iOS build whose shell forgot the handoff sees. The
/// caller falls back to ordinary discovery rather than refusing to start, which on a phone means a
/// path outside the container and a visible failure to write, instead of an invisible one.
pub(crate) fn dirs() -> Option<&'static Dirs> {
    DIRS.get()
}
