//! The machine as an iOS application.
//!
//! An entry point and a paths handoff over `karaokemachine`'s own library. Everything that thinks is
//! in there; what belongs here is two `extern "C"` functions, a `catch_unwind` around each, and a
//! string copy.
//!
//! # The four things `main.m` does, in order
//!
//! 1. Creates `Library/Application Support/karaokemachine` and `Documents/packages`.
//! 2. Configures `AVAudioSession` for playback, which cpal's CoreAudio backend does not do.
//! 3. Calls `km_machine_configure` with those two directories.
//! 4. Hands `SDL_main` to `SDL_RunApp`, which does not return.
//!
//! **Objective-C rather than Swift, where the offline remote's shell is Swift with `@main`.** SDL
//! owns `UIApplicationMain` here: `SDL_RunApp` calls it with SDL's own delegate, which makes the
//! window and the event pump the machine draws through, so a second `@main` beside it would be two
//! applications competing for one process.
//!
//! Steps 1 and 3 are the seam: `directories` ships no iOS module, and the answer a desktop would
//! give is a path outside the container that resolves and cannot be written to. See
//! [`What the machine *is*, on iOS`](../../../docs/decisions/distribution.md).
//!
//! # Why there is no `log.rs` here
//!
//! The offline remote's iOS shell installs its own subscriber, because the library underneath it
//! installs none. The machine's library installs one for both mobile platforms, and a second
//! `init()` is a panic rather than a no-op. So `karaokemachine::install_logging` is idempotent and
//! both callers reach it: this crate before it complains about a missing directory, and
//! `run_on_phone` because on Android nothing else would.
//!
//! What that shares is the decision as well as the code. **iOS has a working stderr** and Xcode's
//! console shows it, which is why neither shell has a `km-ioslog` beside `km-androidlog`. The day to
//! write one is the day a device has to say something with Xcode not attached, because `os_log` is
//! what reaches Console.app and `devicectl --console`.

#[cfg(target_os = "ios")]
mod ffi;

/// Whether this build carries the video decoder.
///
/// Outside the `cfg(target_os = "ios")` gate deliberately, so a desktop `cargo km-test` still
/// compiles and type-checks something in this crate. Everything else here needs the iOS target to
/// exist at all, which would otherwise leave a change to this crate's surface undiscovered until
/// somebody built for a phone. `km-remote-ios` keeps one `pub fn` visible for the same reason.
#[must_use]
pub const fn has_video() -> bool {
    cfg!(feature = "video")
}
