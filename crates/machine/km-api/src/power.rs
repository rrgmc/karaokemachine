//! Turning the machine off, and starting it again.
//!
//! **A capability the host supplies rather than a method on [`Controller`], and the split is the
//! point.** [`Controller`] is playback, the queue and the settings that belong to a performance —
//! things every host can do, which is why it has no defaulted methods. Powering a box off is not
//! like that: an Android television cannot, a desktop must not, and only a supervised appliance
//! both can and should. That is a capability that genuinely varies, and the trait's own note says
//! what to do with one — *"the honest shape is a route that is **not mounted**"*. So a machine with
//! no [`Power`] answers 404 on these paths, exactly as a machine with debugging off answers 404 on
//! `debug/play-file`, rather than mounting a route that can only ever refuse.
//!
//! **This crate implements none of it.** `km-api` has no `cfg` for an operating system and runs on
//! Android, Windows, Linux and an in-memory test double; the binary crate decides whether there is
//! a power button to press and what pressing it runs. Same division as the catalog and the audio
//! device, and for the same reason.
//!
//! [`Controller`]: crate::machine::Controller

/// What a host can do about its own power.
///
/// Installed once at startup through [`ApiState::set_power`](crate::server::ApiState::set_power) and
/// never swapped, which is why the state holds it in a `OnceLock`: a capability is a fact about the
/// machine this process is running on, and that fact does not change while it runs.
///
/// **Both methods return as soon as the request is *accepted*, not when it has happened.** Neither
/// can report how it went, because by the time it has gone the process asking is not there to hear.
/// A handler therefore answers `202 Accepted` and means it.
pub trait Power: Send + Sync + 'static {
    /// Asks the operating system to power the whole box off.
    ///
    /// **Nothing here stops the machine itself, deliberately.** The supervisor notices the box going
    /// down and stops this unit the way it stops it for any other reason, which runs the one
    /// shutdown path that already exists and is already tested — the same path a physical power
    /// button takes. Setting a shutdown flag *as well* would race that: the process would start
    /// persisting settings and dropping the audio device while systemd was separately stopping it,
    /// and the two orderings would interleave differently every time.
    fn shut_down(&self) -> Result<(), PowerError>;

    /// Stops this application cleanly so that whatever supervises it starts it again.
    ///
    /// The box stays on. This is what answers *"I changed a setting that is only read when the
    /// machine starts"* — `api.serve_dev_remote` being the live example — from a phone, on a box
    /// with no keyboard and nobody logged into it.
    ///
    /// **A restart is an exit, not a request to the supervisor**, and that is not a stylistic
    /// choice: asking systemd to restart a unit is `org.freedesktop.systemd1.manage-units`, which
    /// is not granted to an unprivileged account, where exiting needs no privilege whatsoever and
    /// `Restart=always` does the rest.
    fn restart_application(&self) -> Result<(), PowerError>;
}

/// Why a power request did not happen.
///
/// Two variants and no `NotFound`: a host that cannot do this at all has no `Power` and therefore
/// no route to reach, so "unavailable" is spelled by the absence of the endpoint rather than by an
/// error inside it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PowerError {
    /// The machine asked and the operating system said no.
    ///
    /// **Carries the operating system's own words**, because they are the diagnosis. *"Interactive
    /// authentication required."* is a sentence somebody can search for and act on; "power off
    /// failed" is not. This is the same reasoning [`ControlError::Unavailable`] follows in carrying
    /// a `Refusal` rather than a bare discriminant.
    ///
    /// [`ControlError::Unavailable`]: crate::machine::ControlError::Unavailable
    #[error("{0}")]
    Refused(String),
    /// Something went wrong that is not the caller's fault — the tool is missing, or would not run.
    #[error("{0}")]
    Failed(String),
}
