//! The C surface: two functions, and nothing that thinks.
//!
//! | C | What it does |
//! |---|---|
//! | `km_machine_configure(support, documents)` | records the container's two directories |
//! | `SDL_main(argc, argv)` | runs the machine, and does not return until it stops |
//!
//! Its pair is `include/km_machine.h`, which is hand-written and committed. `tests/header_matches.rs`
//! is what stops the two drifting: a C symbol has no mangling, so a changed *signature* still links
//! and would go wrong only on a device.
//!
//! **Two, where the offline remote's shell has six**, and the difference is which side owns the
//! screen. That shell polls a state machine so Swift can draw a view; here SDL draws everything and
//! the shell's whole job is to name two directories and start it. There is no state to poll and
//! no string to hand back, so the ownership rules that shell needed do not arise.
//!
//! # What the boundary still costs
//!
//! **`catch_unwind` on every export, by hand.** An `extern "C"` function is `-unwind` in this
//! edition, so a panic crossing it aborts the process, and an abort is what somebody sees as the
//! application vanishing. Each body catches and returns instead. `SDL_main` is the one that matters:
//! a panic anywhere in the machine would otherwise take the process down without a word, where
//! catching it lets the panic hook's report reach the log first.
//!
//! **The inbound strings are copied before anything else runs.** The buffer belongs to the caller
//! and has a shorter life than it looks: `fileSystemRepresentation` is valid only while the
//! autorelease pool holding its `NSString` is, and Swift's `withCString` frees on the closure's
//! return. A pointer kept for later reads freed memory. The offline remote's shell records the
//! shape of that fault: the freed bytes read back as a *plausible* path, so the failure presents as
//! the wrong directory rather than as a crash.

use std::ffi::{CStr, c_char, c_int};

/// Runs a body, returning `fallback` if it panics.
///
/// The payload is dropped rather than formatted: the panic hook
/// `km_app::install_logging` puts in place has already written the message and a backtrace,
/// which is more than the payload carries.
fn guard<T>(what: &str, fallback: T, body: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => {
            tracing::error!("{what} panicked; see the panic report above");
            fallback
        }
    }
}

/// Reads a C string, treating `NULL` and blank as "not given".
///
/// **The `String` is owned before this returns**, which is the whole point: see the module header.
///
/// # Safety
///
/// `value` must be `NULL` or a valid NUL-terminated string that stays alive for this call.
#[expect(
    unsafe_code,
    reason = "reading the two paths the shell passes; the only pointer dereference in this crate"
)]
unsafe fn read(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // Lossy rather than strict: a container path that is not UTF-8 should reach the log as something
    // a person can read, not vanish into an error branch that says nothing.
    let text = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    (!text.trim().is_empty()).then_some(text)
}

/// Records the two directories the application container gives us.
///
/// Call this before `SDL_main`. Both paths must exist: this library creates neither, because a
/// sandboxed container's directories are the platform's to make and the shell has already asked for
/// them by the time it can pass them here.
///
/// # Safety
///
/// Both arguments must be `NULL` or valid NUL-terminated strings alive for the call.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the shell links against, and reading the two paths it passes"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn km_machine_configure(support: *const c_char, documents: *const c_char) {
    guard("configure", (), || {
        let support = unsafe { read(support) };
        let documents = unsafe { read(documents) };

        // The subscriber first, so that a complaint about either path has somewhere to be read.
        // Idempotent: `run_on_phone` calls the same function, and whichever arrives second is a
        // no-op rather than the panic a second `init()` would be.
        km_app::install_logging();

        let (Some(support), Some(documents)) = (support, documents) else {
            // Not fatal, and deliberately so: `Paths::discover` falls back to ordinary discovery,
            // which on a phone is a path outside the container. That fails visibly on the first
            // write, where refusing to start here would fail with no screen to say why.
            tracing::error!(
                "the container's directories were not both passed; the machine will look for them \
                 where a desktop would"
            );
            return;
        };
        tracing::info!(%support, %documents, "the container named its directories");
        km_app::ioscfg::publish(&support, &documents);
    });
}

/// Runs the machine. Does not return until it stops.
///
/// `SDL_RunApp` in `main.m` calls this, which is what makes the calling thread the one SDL owns.
/// The arguments are SDL's own and are ignored: there is no command line on a phone.
///
/// **Defined here rather than re-exported from `karaokemachine`**, because a `no_mangle` symbol in
/// an upstream rlib is not guaranteed to reach a `staticlib` that never references it. The failure
/// that shape produces is an undefined `_SDL_main` when Xcode links the app.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol SDL_RunApp calls; the signature is SDL's and the body touches \
              no raw pointers"
)]
#[unsafe(no_mangle)]
pub extern "C" fn SDL_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    guard("SDL_main", 1, km_app::run_on_phone)
}
