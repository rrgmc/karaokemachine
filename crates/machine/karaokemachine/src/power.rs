//! Turning this box off, where this box is one that should be turned off.
//!
//! `km-api` describes the capability and mounts routes for it; this decides whether there is one.
//! Same division as the catalog and the audio device, and the reason is the same: only the binary
//! knows what it is running on.
//!
//! **The whole file is one predicate and two commands.** The predicate is where the judgement is.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use km_api::power::Power;

// Only the supervised-Linux half refuses anything or touches the flag, so these would be dead
// imports everywhere else.
#[cfg(target_os = "linux")]
use km_api::power::PowerError;
#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;

/// The power controls this machine actually has, or `None`.
///
/// **The gate is "am I the appliance", not "would the operating system let me".** Those come apart
/// exactly where it matters: a developer running this from a terminal on a Linux desktop is inside
/// their own active logind session, so logind *would* power their desktop off — and offering that
/// on the owner's page of an application somebody installed on their laptop is the same category
/// error as `systemctl enable` in a `postinst`, which this project already refuses.
///
/// `INVOCATION_ID` is what tells them apart. systemd sets it in every unit's environment and a
/// `cargo run` never has one, so it answers *"I am supervised"* — which is simultaneously the
/// question `restart_application` needs answered, since ending the process only starts it again if
/// something is watching.
///
/// **One capability covering both actions rather than two flags.** A restart is meaningless without
/// a supervisor and a shutdown should be refused without one, so the two conditions coincide; two
/// flags would be structure for a variation with no instances, which is the trade `Controller`'s own
/// documentation argues against one file over.
///
/// **Deliberately not probed here**: whether the session is Active, and whether polkit will allow
/// it. Both are true at one moment and false at the next — somebody switches virtual terminal — and
/// both come back from `systemctl` as a readable sentence. Reporting them as *unavailability* would
/// hide a button because of a condition that no longer holds, where the page's own rule is that a
/// control which can only be refused is left out and one that merely *might* be refused is offered.
#[cfg(target_os = "linux")]
pub fn detect(shutdown: Arc<AtomicBool>) -> Option<Arc<dyn Power>> {
    if std::env::var_os("INVOCATION_ID").is_none() {
        tracing::debug!(
            "no INVOCATION_ID: this is not a supervised machine, so it offers no power controls"
        );
        return None;
    }
    tracing::info!("supervised by systemd: the machine can be shut down and restarted remotely");
    Some(Arc::new(Systemd { shutdown }))
}

/// No power controls anywhere but a supervised Linux box.
///
/// Windows and macOS run this as somebody's application, and Android has a power button of its own
/// that this has no business duplicating. The `shutdown` flag is taken and dropped so the two
/// signatures match and the caller needs no `cfg` of its own.
#[cfg(not(target_os = "linux"))]
pub fn detect(_shutdown: Arc<AtomicBool>) -> Option<Arc<dyn Power>> {
    None
}

/// A machine running as a systemd unit.
#[cfg(target_os = "linux")]
struct Systemd {
    /// The one stop flag the display loop and the watchdog already watch.
    ///
    /// A restart sets *this* rather than inventing a second way out, which is what keeps the
    /// shutdown block at the end of [`crate::run`] the only exit there is.
    shutdown: Arc<AtomicBool>,
}

#[cfg(target_os = "linux")]
impl Power for Systemd {
    fn shut_down(&self) -> Result<(), PowerError> {
        // **`systemctl poweroff` rather than a D-Bus call, and it is not the lazier of the two.**
        // It calls `org.freedesktop.login1.Manager.PowerOff` on the system bus — the same method a
        // client would, checked against the same `org.freedesktop.login1.power-off` polkit action,
        // with our session resolved from the child's PID, which inherits our cgroup. So the only
        // thing a `zbus` dependency would buy is a typed error, at the price of some twenty crates
        // in a workspace that has no D-Bus stack at all, for one call made at most once per process
        // lifetime. `km-osopen` reaches `xdg-open` the same way, for the same reason.
        //
        // **Nothing here stops the machine.** systemd notices the box going down and stops this
        // unit the way it stops it for any other reason, which runs the one shutdown path there is
        // — the same one the physical power button takes.
        run("poweroff")
    }

    fn restart_application(&self) -> Result<(), PowerError> {
        // **Not `systemctl restart karaokemachine`**, which would be denied: managing a unit is
        // `org.freedesktop.systemd1.manage-units`, which an unprivileged account does not get,
        // where exiting needs no privilege whatsoever.
        //
        // So this is an ordinary clean exit and `Restart=always` does the rest. `run()` returns
        // `Ok(())`, `main` turns that into exit 0, and systemd starts it again after `RestartSec`.
        // Nothing distinguishes exit reasons anywhere and nothing needs to — `Restart=always`
        // restarts after any exit and after none that `systemctl stop` caused.
        self.shutdown.store(true, Ordering::Release);
        Ok(())
    }
}

/// Runs one `systemctl` verb and turns how it went into a sentence.
///
/// **`output()` rather than `status()`, because the stderr *is* the diagnosis.** *"Interactive
/// authentication required."* is a line somebody can search for and act on; a bare exit code is
/// not, and on the appliance the journal is the only place anybody will look.
#[cfg(target_os = "linux")]
fn run(verb: &str) -> Result<(), PowerError> {
    let output = std::process::Command::new("systemctl")
        .arg(verb)
        .output()
        .map_err(|error| PowerError::Failed(format!("could not run systemctl {verb}: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(PowerError::Refused(if said.is_empty() {
        // A non-zero exit with nothing on stderr is said plainly, because an empty refusal would
        // render as a blank message.
        format!("systemctl {verb} failed and said nothing")
    } else {
        said
    }))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// The gate, from the side that matters: a developer's machine must not offer to switch itself
    /// off. Not the other direction — `INVOCATION_ID` cannot be set for a test without setting it
    /// for the whole process, and a test that leaked it would arm every later test in the binary.
    #[test]
    fn an_unsupervised_run_has_no_power_controls() {
        // `cargo test` is not a systemd unit, which is this test's premise rather than its subject.
        // Skipped rather than failed if it ever is one — a suite run from inside a unit would then
        // report a red test about its own launcher instead of about this code.
        if std::env::var_os("INVOCATION_ID").is_some() {
            return;
        }
        assert!(detect(Arc::new(AtomicBool::new(false))).is_none());
    }
}
