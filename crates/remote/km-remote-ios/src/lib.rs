//! The offline remote as an iOS application.
//!
//! A `staticlib` linked into the app binary, holding the same `km-remote-core` server the desktop
//! shell runs. The view controller is a `WKWebView` on `http://127.0.0.1:<port>/` and nothing else:
//! there is no native interface to build, because the interface already exists — the same pages,
//! over loopback rather than the LAN.
//!
//! # The four seams, and what this host answers
//!
//! `km-remote-core`'s own documentation names four things a host has to supply. This is the third
//! host to do it, and each answer is one line:
//!
//! | Seam | This host |
//! |---|---|
//! | `Config::data_dir` | `Library/Application Support/km-remote`, created by Swift and passed in through `ffi`. **`directories` is never reached**, and here it would not even guess wrongly — it ships `lin.rs`, `mac.rs`, `win.rs` and `wasm.rs`, and on iOS would hand back a macOS path that is not inside the container. |
//! | The port before the slow work | `Bound::bind` is phase 1, so a port exists in milliseconds. It is deliberately **not published until phase 4** — see `km-remote-host`. |
//! | Shutdown as a future | `Stop`, asked from `applicationWillTerminate`. |
//! | Discovery as a trait | `find::Sweep`, because this platform will not let us multicast. |
//!
//! # Why a sweep, and not Bonjour
//!
//! iOS has required the `com.apple.developer.networking.multicast` entitlement for multicast and
//! broadcast since version 14, and Apple grants it only after a manually reviewed request. Without
//! it an mDNS browse **finds nothing and reports success** — the worst shape a fault can have, and
//! indistinguishable from a house with the machine switched off.
//!
//! The other way through is Apple's own Bonjour APIs, `NWBrowser` with `NSBonjourServices`
//! declared, which need no entitlement. That is rejected here for one reason: it
//! puts discovery in Swift, which means a seventh function to hand the answer back down, and it
//! makes the one part of this crate that is genuinely hard to get right the one part that cannot be
//! tested by `cargo test`. `find::Sweep` is ordinary unicast, needs nothing but the local-network
//! permission every app is prompted for, and is `km-remote-core`'s code with `km-remote-core`'s
//! tests. It is also what the owner's Go remote settled on for the same platform.
//!
//! The cost is written down where it belongs, on `find::Sweep` itself: a sweep assumes port 8177,
//! where an SRV record would have carried one.
//!
//! # What thinks, and where it lives
//!
//! **Not here.** The four phases, the singleton, the generation counter and the non-blocking stop
//! are `km-remote-host`, shared with the Android shell and tested by an ordinary `cargo test` on
//! the machine doing the building. This crate is `ffi` and `log`: a string conversion, a
//! `catch_unwind`, and a call.
//!
//! # What this crate deliberately does not carry
//!
//! Each of these is a thing the owner's Go remote has and this one does not, checked rather than
//! assumed:
//!
//! - **No `fork`, and nothing to `exec`.** That design is a *Go* constraint — a cgo-free Go binary
//!   cannot be loaded as a library — and iOS forbids it outright anyway, which is why that project
//!   builds a `c-archive` here and a forked child on Android. Rust has the problem on neither.
//! - **No suspend and no resume.** Those exist there because the karaoke unit serves five clients
//!   and an abandoned session holds a slot until it times out. Nothing here is being held: the
//!   machine serves any number of callers, this remote never names itself to one, and its link is
//!   an HTTP client plus an event stream that reconnects with backoff. A suspension freezes the
//!   server with the app and thaws it again; the loopback port does not move.
//! - **No progress function.** `spawn_warm_up` puts the catalog import *behind* the pages, so
//!   there is nothing to report progress for on a screen anybody is looking at.
//! - **No time-zone plumbing.** Go hardcodes `time.Local` to UTC on iOS and has no platform
//!   zoneinfo at all, so that project passes a zone down through the environment and reads it with
//!   `C.getenv` because the Go runtime caches its own copy at process start. Rust has neither
//!   problem, and the only wall clock in the core is SQLite's `datetime('now')`, which is UTC
//!   everywhere by definition and is read only to sort by.
//! - **No event-stream resume shim.** `km-remote-pages`'s `static/live.js` uses a plain `EventSource`,
//!   whose reconnection is the browser's own. The Go remote needs one because it drives htmx's SSE
//!   extension, where the browser reconnects the stream and leaves every swap target listening to
//!   the dead one — which is how an offline banner there outlived the reconnection it was
//!   reporting.
//! - **No asset unpacking.** `km-remote-pages` compiles every template, stylesheet and script in and
//!   serves them from routes, so there is no bundle resource to read and nothing to unpack.

// Everything iOS-specific, and nothing else, lives behind this gate. A desktop
// `cargo build --workspace` therefore compiles an empty archive that nothing links.
#[cfg(target_os = "ios")]
mod ffi;
#[cfg(target_os = "ios")]
mod log;

/// Which [`Locator`](km_remote_core::find::Locator) this shell hands to the core.
///
/// **Outside the target gate on purpose**, for the reason its Android twin records: everything else
/// in this crate needs an iOS target, so a change to a locator's constructor would compile clean on
/// a desktop and be found by whoever next built the port. It cost exactly that once, on the other
/// shell. One `pub fn` on this side of the gate puts the decision in the desktop build.
///
/// **A sweep, because this platform may not multicast at all.** iOS has required
/// `com.apple.developer.networking.multicast` since 14 and Apple grants it only after a manual
/// review — and without it an mDNS browse does not fail, it finds nothing and reports success, which
/// is the worst shape a fault can have. [`Sweep`](km_remote_core::find::Sweep) asks each address on
/// the local subnet instead.
/// No `cfg` on this: `sweep` is not a feature of *this* crate, it is one this crate turns on in its
/// dependency — see the manifest. Gating on a feature that never exists here would have made the
/// guard above compile to nothing, which is the failure it exists to prevent, one level in.
#[must_use]
pub fn locator() -> std::sync::Arc<dyn km_remote_core::find::Locator> {
    std::sync::Arc::new(km_remote_core::find::Sweep::new())
}
