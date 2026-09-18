//! The C surface: six functions, and nothing that thinks.
//!
//! The same six the Android shell exports over JNI, which is what
//! `km-remote-android`'s own `ffi.rs` meant when it said the surface was "deliberately kept to what
//! an iOS `staticlib` could export unchanged, so that shell is a calling convention rather than a
//! redesign". It was, and this is the convention:
//!
//! | C | What it answers |
//! |---|---|
//! | `km_remote_start(data_dir, machine)` | begins a run; a no-op if one is going |
//! | `km_remote_port()` | 0 until the server is answering, then the port |
//! | `km_remote_failure()` | why it stopped, or `NULL` while it is healthy |
//! | `km_remote_machine()` | the machine it found, or `NULL` — which is ordinary |
//! | `km_remote_songs()` | how many songs this device's copy holds, or -1 |
//! | `km_remote_stop()` | asks it to stop, and returns at once |
//!
//! Its pair is `include/km_remote.h`, which is hand-written and committed. `tests/header_matches.rs`
//! is what stops the two drifting: a C symbol has no mangling, so a changed *signature* still links
//! and would go wrong only on a device.
//!
//! # Three things JNI did for free
//!
//! **`catch_unwind` on every export, by hand.** `EnvUnowned::with_env` wrapped the Android bodies;
//! nothing wraps these. An `extern "C"` function is `-unwind` in this edition, so a panic crossing
//! it aborts the process — and an abort is what somebody sees as the app vanishing while a `Timer`
//! polls it ten times a second. Each body catches and returns the caller's own fallback instead,
//! which is exactly the policy `settle` applies on the other side.
//!
//! **The inbound strings are copied before anything is spawned.** Swift's `withCString` frees its
//! buffer the moment the closure returns, so a pointer kept for later reads freed memory. This is
//! the worst bug the Go remote this follows ever had, and it is worth knowing the shape of it: the
//! freed bytes read back as a *plausible* address, so discovery was skipped and a dial ran to a
//! timeout — presenting as a slow first run rather than as a crash. It was invisible in the
//! simulator, where the freed bytes happened to begin with a NUL and so read as the empty string,
//! which is the "discover it" case.
//!
//! **The outbound strings belong to this library.** Swift polls at ten hertz, so a fresh allocation
//! per call would leak on every one; instead each of the two string functions keeps its answer in a
//! static and hands out a pointer into it. The header says the caller must not free it, and must
//! copy it before the next call. See [`publish`].
//!
//! # Nothing is thrown, because there is nothing to throw
//!
//! There is no exception mechanism on a C boundary, which makes the Android decision — swallow,
//! log, return a fallback — the only one available rather than a choice. It was the right choice
//! there for a reason that holds here too: a remote that cannot reach its machine has better things
//! to do than take the app down.

use std::ffi::{CStr, CString, c_char};
use std::sync::{Mutex, OnceLock};

use km_remote_host as state;

/// Runs a body, returning `fallback` if it panics.
///
/// The counterpart of the Android shell's `settle`, and it takes a fallback for the same reason:
/// what "nothing" means differs per function — a null pointer for the two that return a string, 0
/// for a port that is not bound, -1 for a song count nobody knows.
///
/// The payload is dropped rather than formatted: the panic hook installed by [`crate::log`] has
/// already written the message and a backtrace to stderr, which is more than the payload carries.
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
/// A `NULL` really does arrive — it is how the view controller says no address was typed — so it is
/// a case rather than an error. **The `String` is owned before this returns**, which is the whole
/// point: see the module header.
///
/// # Safety
///
/// `value` must be `NULL` or a valid NUL-terminated string that stays alive for this call.
#[expect(
    unsafe_code,
    reason = "reading the two strings Swift passes; the only pointer dereference in this crate"
)]
unsafe fn read(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // Lossy rather than strict: a path or an address that is not UTF-8 should reach the log as
    // something a person can read, not vanish into an error branch that says nothing.
    let text = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    (!text.trim().is_empty()).then_some(text)
}

/// Hands out a pointer to a string this library owns.
///
/// **Replaced only when the value changes**, which is what makes a pointer stable across the polls
/// that return the same answer — and those are almost all of them. A caller that copies within the
/// call, as `String(cString:)` does, is safe either way; this only narrows the window.
///
/// The lock is released before the pointer is returned, and that is sound: the pointer is into a
/// `CString` owned by the static, not into the guard.
fn publish(slot: &'static Mutex<Option<CString>>, value: Option<String>) -> *const c_char {
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    let Some(text) = value else {
        *guard = None;
        return std::ptr::null();
    };
    // An interior NUL cannot be expressed in C at all. Empty is the honest rendering of it, and a
    // failure reason containing one is not a thing that happens.
    let wanted = CString::new(text).unwrap_or_default();
    if guard.as_deref() != Some(wanted.as_c_str()) {
        *guard = Some(wanted);
    }
    guard
        .as_ref()
        .map_or(std::ptr::null(), |held| held.as_ptr())
}

fn failure_text() -> &'static Mutex<Option<CString>> {
    static SLOT: OnceLock<Mutex<Option<CString>>> = OnceLock::new();
    SLOT.get_or_init(Mutex::default)
}

fn machine_text() -> &'static Mutex<Option<CString>> {
    static SLOT: OnceLock<Mutex<Option<CString>>> = OnceLock::new();
    SLOT.get_or_init(Mutex::default)
}

/// Starts the server. Returns at once; poll `km_remote_port` for the result.
///
/// # Safety
///
/// Both arguments must be `NULL` or valid NUL-terminated strings alive for the call.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol Swift links against, and reading the two strings it passes"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn km_remote_start(data_dir: *const c_char, machine: *const c_char) {
    guard("start", (), || {
        let data_dir = unsafe { read(data_dir) };
        let machine = unsafe { read(machine) };

        // The subscriber first, so that everything below — including a failure to start — has
        // somewhere to be read.
        crate::log::install();

        let Some(data_dir) = data_dir else {
            tracing::error!("no data directory was passed; the remote cannot start");
            return;
        };
        // **`find::Sweep`, and the decision is made by this crate rather than by a default.**
        // `km-remote-host` takes a locator because its two shells cannot both browse; this one
        // cannot. See this crate's `lib.rs`, which is also where the one line naming it lives, on
        // the desktop-visible side of the target gate.
        state::start(
            std::path::PathBuf::from(data_dir),
            machine,
            crate::locator(),
        );
    });
}

/// The port the remote is answering on, or 0 until it is.
#[expect(unsafe_code, reason = "exporting the C symbol Swift links against")]
#[unsafe(no_mangle)]
pub extern "C" fn km_remote_port() -> i32 {
    guard("port", 0, || i32::from(state::port()))
}

/// How many songs this device's copy of the catalog holds, or -1 if that is not known yet.
///
/// Zero is a real answer and an important one: a first run that found no machine and has nothing to
/// show, which is the case a host has to tell apart from a broken one.
#[expect(unsafe_code, reason = "exporting the C symbol Swift links against")]
#[unsafe(no_mangle)]
pub extern "C" fn km_remote_songs() -> i32 {
    guard("songs", -1, state::songs)
}

/// Why the remote stopped, or `NULL` while it is healthy.
///
/// The returned pointer belongs to this library. Do not free it, and copy it before the next call.
#[expect(unsafe_code, reason = "exporting the C symbol Swift links against")]
#[unsafe(no_mangle)]
pub extern "C" fn km_remote_failure() -> *const c_char {
    guard("failure", std::ptr::null(), || {
        publish(failure_text(), state::failure())
    })
}

/// The machine this run is talking to, or `NULL`. `NULL` is ordinary and is not a failure.
///
/// The returned pointer belongs to this library. Do not free it, and copy it before the next call.
#[expect(unsafe_code, reason = "exporting the C symbol Swift links against")]
#[unsafe(no_mangle)]
pub extern "C" fn km_remote_machine() -> *const c_char {
    guard("machine", std::ptr::null(), || {
        publish(machine_text(), state::machine())
    })
}

/// Asks the server to stop. Returns at once — see `km_remote_host::stop` for why that matters.
#[expect(unsafe_code, reason = "exporting the C symbol Swift links against")]
#[unsafe(no_mangle)]
pub extern "C" fn km_remote_stop() {
    guard("stop", (), state::stop);
}
