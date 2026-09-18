//! The offline remote as an Android application.
//!
//! A `cdylib` the APK loads with `System.loadLibrary`, holding the same `km-remote-core` server the
//! desktop shell runs. The Activity is a `WebView` on `http://127.0.0.1:<port>/` and nothing else:
//! there is no native interface to build, because the interface already exists — the same pages,
//! over loopback rather than the LAN.
//!
//! # The four seams, and what this host answers
//!
//! `km-remote-core`'s own documentation names four things a host has to supply. This is the second
//! host to do it, and each answer is one line:
//!
//! | Seam | This host |
//! |---|---|
//! | `Config::data_dir` | `Context.getFilesDir()`, passed in through `ffi`. **`directories` is never reached**, which matters: it ships `lin.rs`, `mac.rs`, `win.rs` and `wasm.rs` only, so on Android it would silently take the Linux XDG path, which depends on a `$HOME` that is normally unset and falls back to `/`. |
//! | The port before the slow work | `Bound::bind` is phase 1, so a port exists in milliseconds. It is deliberately **not published until phase 4** — see `km_remote_host`'s `state`. |
//! | Shutdown as a future | `Stop`, asked from the Activity's `onDestroy`. |
//! | Discovery as a trait | `find::Mdns`, handed to [`km_remote_host::start`] — see below. |
//!
//! # Why there is no `fork`, and no line to scrape
//!
//! The Go remote this product's owner already ships packages its server as a file named `lib*.so`,
//! extracts it, `exec`s it as a child process and scrapes the child's stdout for the port it bound.
//! **That is a Go constraint, not an Android one**: a cgo-free Go binary cannot be loaded as a
//! library, and `nativeLibraryDir` is the one directory an app may still execute from. Rust has no
//! such problem. This is one process, the server runs on a `tokio` runtime inside it, and the port
//! is read straight across the JNI boundary.
//!
//! Two Gradle settings follow from that and must **not** be copied from the Go project:
//! `useLegacyPackaging` and `extractNativeLibs` exist to put a real, uncompressed file on disk for
//! something to `exec`. `System.loadLibrary` maps a library straight out of the APK, so setting them
//! here would keep two copies on the device for no reason.
//!
//! # What thinks, and where it lives
//!
//! **Not here.** The four phases, the singleton, the generation counter and the non-blocking stop
//! are `km-remote-host`, which is an ordinary library shared with the iOS shell and tested by an
//! ordinary `cargo test` on the machine doing the building. This crate is `ffi` and `log` and
//! nothing else: a string conversion, a guard against unwinding into the JVM, and a call.
//!
//! It was this crate's own `state.rs` while there was one mobile shell. The second one needed the
//! identical thing, and a copy of the part that is easy to get wrong is not a thing to keep two of.
//!
//! # Why Java holds the multicast lock
//!
//! An mDNS browse on Android sees nothing unless Java holds a `WifiManager.MulticastLock` for its
//! duration — sending multicast is unrestricted, which is why the machine's own app advertises
//! without one and this one cannot browse without one.
//!
//! The lock is therefore held by the Activity across `onStart`/`onStop`, and this crate passes the
//! ordinary `find::Mdns`. The alternative — a `Locator` that calls back into Java around each browse
//! — is more precise about battery and considerably worse in every other way, and one detail is
//! the one whoever optimizes this later should know about first: `Locator::browse`
//! runs on a `spawn_blocking` thread, and `FindClass` from a thread JNI has attached cannot see
//! *application* classes, because that thread's class loader is the bootstrap one. The lookup would
//! have to be cached in `JNI_OnLoad`, and the failure without it appears only on a first run, only
//! when nothing has been remembered.
//!
//! Holding it for the foreground costs little: `find::remembered` means the second launch normally
//! does not browse at all, and the background loop browses only while the machine is unreachable.
//! Not holding it while backgrounded is *correct* rather than a compromise — a browse then returns
//! nothing, and `find::recovery` answers `Stay`.
//!
//! # What this crate deliberately does not carry
//!
//! No asset unpacking: `km-remote-pages` compiles every template, stylesheet and script in and serves them
//! from routes, so there is no `MANIFEST` and no counterpart to `km-app`'s `androidassets`. No time
//! zone plumbing either — the Go remote had to pass one down because Go hardcodes `time.Local` to
//! UTC on Android, and Rust has no such thing; the only wall clock in the core is SQLite's
//! `datetime('now')`, which is UTC everywhere by definition and is read only to sort by.

// Everything Android-specific, and nothing else, lives behind this gate. A desktop
// `cargo build --workspace` therefore compiles neither `jni` nor `km-androidlog`.
#[cfg(target_os = "android")]
mod ffi;
#[cfg(target_os = "android")]
mod log;

/// Which [`Locator`](km_remote_core::find::Locator) this shell hands to the core.
///
/// **Outside the target gate on purpose, and it is the only line of this crate's decisions that is.**
/// Everything else here needs `jni`, so it is compiled by an Android build and by nothing else —
/// which means a change to a locator's constructor compiles clean on a desktop, passes CI, merges,
/// and is discovered by whoever next builds the port. That happened: `Mdns` grew a field, every
/// caller in the workspace was updated, `cargo check --workspace` was clean, and this crate did not
/// build. A `pub fn` on this side of the gate puts that one line in the desktop build, so the next
/// such change breaks where somebody is looking.
///
/// **A real browse, because Java holds the multicast lock.** `MainActivity` takes a
/// `WifiManager.MulticastLock` across `onStart`/`onStop`, so this build may browse for as long as
/// the application is on screen — which is what makes the *same* build want `Mdns` while it holds
/// the lock and nothing while it does not, and why this is a value rather than a `#[cfg]`.
#[must_use]
pub fn locator() -> std::sync::Arc<dyn km_remote_core::find::Locator> {
    std::sync::Arc::new(km_remote_core::find::Mdns::new())
}
