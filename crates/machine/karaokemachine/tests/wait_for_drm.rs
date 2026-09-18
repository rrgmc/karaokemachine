//! What `linux/wait-for-drm.sh` waits for.
//!
//! **A shell script with a test, which is unusual here and is earned.** This one decides whether the
//! appliance comes up with a picture, it is exercised on exactly one machine in the world, and its
//! first version was wrong in a way nothing could catch until a television stayed black — the
//! predicate was "the device node exists", and the node existed. The interesting cases are all
//! states of `/sys/class/drm` that are awkward to produce on demand and trivial to fabricate, so the
//! script takes `KM_DRM_SYSFS` and `KM_DRM_DRI_DIR` and this points them at a directory.
//!
//! **Every case asserts exit status 0.** That is the invariant that must never break, whatever the
//! script decides: a machine with no screen still has an API, a queue and a catalog, and an
//! `ExecStartPre` that fails is one that stops the machine from starting at all. It is `-` prefixed
//! in the unit as well, so this is belt and braces — but the belt is what is tested.
//!
//! Unix only: it runs `sh`, so it covers the Linux container `tools/platform/linux/check.sh` uses and
//! macOS, and is compiled out on Windows.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch directory that cleans up after itself.
///
/// Hand-rolled rather than `tempfile`, which this workspace does not depend on anywhere; the same
/// shape as `Scratch` in `settings.rs`.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "km-wait-for-drm-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    /// A connector directory: `<sysfs>/card0-HDMI-A-1/{status,modes}`.
    ///
    /// `modes` is written even when empty, because an empty `modes` beside a `connected` status is
    /// the exact state that produced the fault this script now waits past.
    fn connector(&self, name: &str, status: &str, modes: &str) {
        let dir = self.0.join("sys").join(name);
        std::fs::create_dir_all(&dir).expect("the connector directory");
        std::fs::write(dir.join("status"), format!("{status}\n")).expect("status");
        std::fs::write(dir.join("modes"), modes).expect("modes");
    }

    /// A card node under the fake `/dev/dri`.
    fn node(&self, card: &str) {
        let dir = self.0.join("dri");
        std::fs::create_dir_all(&dir).expect("the dri directory");
        std::fs::write(dir.join(card), b"").expect("the node");
    }

    fn sysfs(&self) -> PathBuf {
        self.0.join("sys")
    }

    fn dri(&self) -> PathBuf {
        self.0.join("dri")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs the script against a fabricated tree, with a short timeout so a "no display" case is quick.
fn run(scratch: &Scratch, timeout_ds: &str) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("linux")
        .join("wait-for-drm.sh");
    Command::new("sh")
        .arg(&script)
        .env("KM_DRM_SYSFS", scratch.sysfs())
        .env("KM_DRM_DRI_DIR", scratch.dri())
        .env("KM_DRM_TIMEOUT_DS", timeout_ds)
        .output()
        .expect("run wait-for-drm.sh")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_connected_connector_with_a_mode_is_what_it_waits_for() {
    let scratch = Scratch::new("connected");
    scratch.connector("card0-HDMI-A-1", "connected", "2560x1080\n1920x1080\n");
    scratch.node("card0");

    let output = run(&scratch, "20");
    assert!(output.status.success(), "must always exit 0");
    let said = stderr(&output);
    assert!(
        said.contains("card0-HDMI-A-1"),
        "names the connector: {said}"
    );
    assert!(said.contains("2560x1080"), "names the mode: {said}");
}

/// The fault, and the case the first version of this script could not see.
///
/// `connected` with an empty `modes` is what i915 reports for the ~1.1 s between creating the node
/// and finishing the connector probe. The old predicate — the node exists — was satisfied throughout
/// it, which is how the appliance came up black with `card0 appeared after 300ms` in the journal.
#[test]
fn connected_with_no_mode_yet_is_not_a_display() {
    let scratch = Scratch::new("no-modes");
    scratch.connector("card0-HDMI-A-1", "connected", "");
    scratch.node("card0");

    let output = run(&scratch, "3");
    assert!(output.status.success(), "must always exit 0");
    assert!(
        stderr(&output).contains("no connected display"),
        "waits rather than accepting it: {}",
        stderr(&output)
    );
}

/// A television switched off at the wall drops hotplug detect, and this is what that looks like.
///
/// Timing out and starting anyway is correct — the machine still has an API and a queue, and the
/// retry in `lib.rs` is what takes the screen when somebody switches it on.
#[test]
fn a_disconnected_connector_times_out_and_starts_anyway() {
    let scratch = Scratch::new("disconnected");
    scratch.connector("card0-HDMI-A-1", "disconnected", "");
    scratch.node("card0");

    let output = run(&scratch, "3");
    assert!(output.status.success(), "must always exit 0");
    assert!(stderr(&output).contains("no connected display"));
}

/// A box with an integrated and a discrete GPU finds the connector that is actually lit, on
/// whichever card it is on.
#[test]
fn the_connector_may_be_on_a_second_card() {
    let scratch = Scratch::new("two-cards");
    scratch.connector("card0-DP-1", "disconnected", "");
    scratch.connector("card1-HDMI-A-1", "connected", "1920x1080\n");
    scratch.node("card0");
    scratch.node("card1");

    let output = run(&scratch, "20");
    assert!(output.status.success());
    let said = stderr(&output);
    assert!(
        said.contains("card1-HDMI-A-1"),
        "picks the lit card: {said}"
    );
}

/// A connected connector whose card node has not appeared yet is not usable either — SDL has to
/// `open` the node, so that is a second thing to wait for and not the only one.
#[test]
fn a_connector_whose_node_is_missing_is_not_accepted() {
    let scratch = Scratch::new("no-node");
    scratch.connector("card0-HDMI-A-1", "connected", "1920x1080\n");
    // deliberately no `scratch.node("card0")`
    std::fs::create_dir_all(scratch.dri()).expect("an empty dri directory");

    let output = run(&scratch, "3");
    assert!(output.status.success(), "must always exit 0");
    assert!(stderr(&output).contains("no connected display"));
}

/// A machine with no DRM at all — no `/sys/class/drm`, so the glob matches nothing and stays literal.
///
/// This pins the unmatched-glob handling: the script must time out cleanly rather than emit shell
/// errors about a file named `card*-*`.
#[test]
fn no_drm_subsystem_at_all_is_a_clean_timeout() {
    let scratch = Scratch::new("no-drm");
    // Neither directory is created.

    let output = run(&scratch, "3");
    assert!(output.status.success(), "must always exit 0");
    let said = stderr(&output);
    assert!(said.contains("no connected display"), "{said}");
    assert!(
        !said.contains("card*"),
        "an unmatched glob must not reach a message: {said}"
    );
    assert!(
        !said.to_lowercase().contains("no such file"),
        "no shell errors: {said}"
    );
}
