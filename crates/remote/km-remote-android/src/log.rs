//! Getting this library's `tracing` events somewhere a person can read them.
//!
//! There is no stdout on Android — the process is started by the zygote with its standard streams
//! pointing at `/dev/null` — so without this the app fails in complete silence, which is the worst
//! state to debug anything from. `km-androidlog` does the work; this decides the tag and the filter.

use std::sync::Once;

/// Guards the one-shot installation.
///
/// `start` may be called more than once — an Activity is recreated on a rotation, and Try again
/// calls it directly — and a second `init()` returns an error rather than replacing the subscriber.
static ONCE: Once = Once::new();

/// Installs the subscriber and the panic hook, once.
pub fn install() {
    ONCE.call_once(|| {
        tracing_subscriber::fmt()
            // `adb logcat -s km-remote` then shows this app and not the machine, which is the whole
            // reason `km-androidlog` takes a tag rather than holding one.
            .with_writer(km_androidlog::Logcat::new(c"km-remote"))
            // No terminal on the other end; escape codes would be noise in logcat.
            .with_ansi(false)
            // Nor a clock worth printing: logcat stamps every line already.
            .without_time()
            .with_env_filter(filter())
            .init();
        // After the subscriber, because the hook reports through it.
        km_androidlog::install_panic_logger();
    });
}

/// What a build says out loud.
///
/// **`info` in a release build**, on the same terms as every other shipped binary here: a machine
/// under a television and a phone in somebody's hand are both places where a per-request debug
/// stream is noise somebody else has to scroll past.
///
/// A debug build gets the three crates that make up this application, which is the same grouping
/// `km-remote`'s `-v` offers and for the same reason — a fault is almost never in only one of
/// them. **The build type is what asks**, rather than a flag: `RUST_LOG` cannot be set for an
/// Activity, and there is no command line to put a `-v` on.
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
