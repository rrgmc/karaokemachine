//! The JNI surface: six functions, and nothing that thinks.
//!
//! Everything with a decision in it is in [`crate::state`], which compiles and is tested off
//! Android. What is left here is a string conversion, a guard against unwinding across the boundary,
//! and a call.
//!
//! # Six functions, and why they are these six
//!
//! The surface is deliberately kept to what an iOS `staticlib` could export unchanged, so that shell
//! is a calling convention rather than a redesign:
//!
//! | Java | What it answers |
//! |---|---|
//! | `start(dataDir, machine)` | begins a run; a no-op if one is going |
//! | `port()` | 0 until the server is answering, then the port |
//! | `failure()` | why it stopped, or `null` while it is healthy |
//! | `machine()` | the machine it found, or `null` — which is ordinary |
//! | `songs()` | how many songs this device's copy holds, or -1 |
//! | `stop()` | asks it to stop, and returns at once |
//!
//! `machine` and `songs` exist for exactly one screen — a first run that found nothing and has
//! nothing to show, which is the one moment somebody has to be offered a box to type an address in.
//! They are two lines each, and they are what keeps that screen out of `km-remote-pages`, whose pages are
//! shared with the desktop shell and with the machine.
//!
//! There is deliberately **no progress function**. The Go remote this follows needed one because its
//! catalog import ran before its listener did, so a first run showed nothing for twenty seconds.
//! Here `spawn_warm_up` puts the import behind the pages, so there is nothing to report progress for
//! on a screen anybody is looking at.
//!
//! # Names, not `RegisterNatives`
//!
//! The exports are found by their mangled names. `RegisterNatives` would buy resistance to
//! obfuscation, which this APK does not use — `minifyEnabled` is off — and would cost a table of JNI
//! signature strings checked at *runtime*, which goes quietly wrong when a parameter type changes.
//! A name-mangled export is checked by the linker, and `llvm-readelf --dyn-syms` can confirm it
//! survived — which matters here more than usual, because `.cargo/config.toml` puts a version script
//! on every armv7 build in this workspace.
//!
//! # Nothing is thrown into Java
//!
//! `EnvUnowned::with_env` already wraps its closure in a `catch_unwind`, so a panic cannot unwind
//! into the JVM and abort the process. What it does *next* is a choice, and the choice here is to
//! swallow it: [`settle`] logs and returns a fallback rather than throwing. An exception crossing
//! back into an Activity that is polling every hundred milliseconds would be a crash, and a remote
//! that cannot reach its machine has better things to do than take the app down.

use jni::objects::{JClass, JString};
use jni::sys::jint;
use jni::{Env, EnvUnowned, Outcome};

use km_remote_host as state;

/// Turns an outcome into a value, reporting anything that was not success.
///
/// The fallback is the caller's, because what "nothing" means differs per function: a null string
/// for the two that return one, 0 for a port that is not bound, -1 for a song count nobody knows.
fn settle<T>(outcome: Outcome<T, jni::errors::Error>, what: &str, fallback: T) -> T {
    match outcome {
        Outcome::Ok(value) => value,
        Outcome::Err(error) => {
            tracing::error!(%error, "{what} failed at the JNI boundary");
            fallback
        }
        Outcome::Panic(_) => {
            // The payload itself is dropped here rather than formatted: the panic hook installed by
            // `crate::log` has already put the message and a backtrace in logcat, which is more than
            // the payload carries.
            tracing::error!("{what} panicked; see the panic report above");
            fallback
        }
    }
}

/// Reads a Java string, treating `null` and blank as "not given".
///
/// A `null` really does arrive — it is how the Activity says no address was typed — so it is a case
/// rather than an error.
fn read(env: &Env, value: &JString) -> jni::errors::Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    // `JString::mutf8_chars` rather than `Env::get_string`, which 0.22 deprecates. The `String`
    // conversion is what re-encodes JNI's modified UTF-8 into Rust's.
    let text: String = value.mutf8_chars(env)?.into();
    Ok((!text.trim().is_empty()).then_some(text))
}

/// Starts the server. Returns at once; poll `port` for the result.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the JVM looks up by name; the body touches no raw pointers"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rrgmc_karaokemachine_remote_Native_start<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
    data_dir: JString<'local>,
    machine: JString<'local>,
) {
    let outcome = unowned
        .with_env(|env| -> jni::errors::Result<()> {
            let data_dir = read(env, &data_dir)?;
            let machine = read(env, &machine)?;

            // The subscriber first, so that everything below — including a failure to start — has
            // somewhere to be read. Installing it here rather than in `JNI_OnLoad` keeps the loader
            // callback doing nothing, which is the one thing it should do.
            crate::log::install();

            let Some(data_dir) = data_dir else {
                tracing::error!("no data directory was passed; the remote cannot start");
                return Ok(());
            };
            // **`find::Mdns`, and the decision is made by this crate rather than by a default.**
            // `km-remote-host` takes a locator because its other shell may not multicast at all;
            // this one may, for as long as MainActivity holds the lock. See this crate's `lib.rs`.
            state::start(
                std::path::PathBuf::from(data_dir),
                machine,
                crate::locator(),
            );
            Ok(())
        })
        .into_outcome();
    settle(outcome, "start", ());
}

/// The port the remote is answering on, or 0 until it is.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the JVM looks up by name; the body touches no raw pointers"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rrgmc_karaokemachine_remote_Native_port<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jint {
    let outcome = unowned
        .with_env(|_env| -> jni::errors::Result<jint> { Ok(jint::from(state::port())) })
        .into_outcome();
    settle(outcome, "port", 0)
}

/// How many songs this device's copy of the catalog holds, or -1 if that is not known yet.
///
/// Zero is a real answer and an important one: a first run that found no machine and has nothing to
/// show, which is the case a host has to tell apart from a broken one.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the JVM looks up by name; the body touches no raw pointers"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rrgmc_karaokemachine_remote_Native_songs<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jint {
    let outcome = unowned
        .with_env(|_env| -> jni::errors::Result<jint> { Ok(state::songs()) })
        .into_outcome();
    settle(outcome, "songs", -1)
}

/// Why the remote stopped, or `null` while it is healthy.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the JVM looks up by name; the body only builds a Java string"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rrgmc_karaokemachine_remote_Native_failure<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let outcome = unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            match state::failure() {
                Some(reason) => env.new_string(reason),
                None => Ok(JString::null()),
            }
        })
        .into_outcome();
    settle(outcome, "failure", JString::null())
}

/// The machine this run is talking to, or `null`. `null` is ordinary and is not a failure.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the JVM looks up by name; the body only builds a Java string"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rrgmc_karaokemachine_remote_Native_machine<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let outcome = unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            match state::machine() {
                Some(url) => env.new_string(url),
                None => Ok(JString::null()),
            }
        })
        .into_outcome();
    settle(outcome, "machine", JString::null())
}

/// Asks the server to stop. Returns at once — see [`state::stop`] for why that matters here.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the JVM looks up by name; the body touches no raw pointers"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rrgmc_karaokemachine_remote_Native_stop<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    let outcome = unowned
        .with_env(|_env| -> jni::errors::Result<()> {
            state::stop();
            Ok(())
        })
        .into_outcome();
    settle(outcome, "stop", ());
}
