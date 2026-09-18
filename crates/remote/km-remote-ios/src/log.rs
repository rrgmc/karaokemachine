//! Getting this library's `tracing` events somewhere a person can read them.
//!
//! # Why there is no `km-ioslog` crate
//!
//! `km-androidlog` exists because Android has no standard output at all: the zygote starts every
//! process with its streams pointing at `/dev/null`, so without a logcat bridge the app fails in
//! complete silence. **iOS has a working stderr**, and Xcode's console shows it, so the equivalent
//! crate here would be a wrapper around `std::io::stderr` — a dependency to write down a decision
//! that `tracing_subscriber` already makes correctly.
//!
//! What it would buy, and the reason to keep this paragraph rather than delete it: `os_log` reaches
//! Console.app and `devicectl --console`, which is what you want when the app is running on a
//! device with Xcode *not* attached. That is the day to write the crate. Until then a build run
//! from Xcode says everything, and a build run without it says nothing to nobody.
//!
//! The owner's Go remote made the same call for the same reason, and its own notes say so: "the
//! server writes to stderr, which Xcode's console shows. There is no `os_log` bridge."

use std::sync::Once;

/// Guards the one-shot installation.
///
/// `start` may be called more than once — *Try again* on the failure screen calls it directly — and
/// a second `init()` returns an error rather than replacing the subscriber.
static ONCE: Once = Once::new();

/// Installs the subscriber and the panic hook, once.
pub fn install() {
    ONCE.call_once(|| {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            // No terminal on the other end; escape codes are noise in Xcode's console.
            .with_ansi(false)
            .with_env_filter(filter())
            .init();
        // After the subscriber, because the hook reports through it.
        install_panic_logger();
    });
}

/// What a build says out loud.
///
/// **`info` in a release build**, on the same terms as every other shipped binary here: a machine
/// under a television and a phone in somebody's hand are both places where a per-request debug
/// stream is noise somebody else has to scroll past.
///
/// A debug build gets the three crates that make up this application, which is the same grouping
/// `km-remote`'s `-v` offers and the same one the Android shell picks. **The build type is what
/// asks**, rather than a flag: there is no command line to put a `-v` on, and while `RUST_LOG`
/// could in principle be set with a scheme's environment variables, a build somebody is debugging
/// in Xcode is exactly the build that should already be talking.
fn filter() -> tracing_subscriber::EnvFilter {
    // Derived rather than typed: `CARGO_CRATE_NAME` answers for this crate, and the two libraries
    // below export their own. A filter naming a target that does not exist is not an error
    // anywhere -- it is simply a debug build that says nothing.
    let own = env!("CARGO_CRATE_NAME");
    let (core, pages) = (km_remote_core::LOG_TARGET, km_remote_core::PAGES_LOG_TARGET);
    let default = if cfg!(debug_assertions) {
        format!("info,{own}=debug,{core}=debug,{pages}=debug")
    } else {
        "info".to_owned()
    };
    tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| default.into())
}

/// Reports a panic through `tracing` before the default hook runs.
///
/// **Chained rather than replacing**, so whatever the runtime would have printed still happens.
/// The backtrace is forced on: `RUST_BACKTRACE` is not a thing anybody sets for an app launched
/// from a home screen, and a panic report with no frames in it is a report that costs a rebuild.
///
/// This matters more here than it looks. Every export catches its own panic and returns a fallback,
/// so **without this a panic would be completely silent** — the app would go on polling a port that
/// never appears, and the only symptom would be a starting screen that never leaves.
fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(location = %info.location().map_or_else(
            || "unknown".to_owned(),
            std::string::ToString::to_string,
        ), "panic: {info}\n{backtrace}");
        previous(info);
    }));
}
