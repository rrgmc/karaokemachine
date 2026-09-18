//! What `tools/platform/linux/grub-appliance-edit.sh` does to `/etc/default/grub`.
//!
//! **The second shell script in this repository with a test, and it is earned for a harder reason
//! than the first.** `wait-for-drm.sh` decides whether the appliance comes up with a picture; this
//! one edits a bootloader's configuration, and the failure mode at the bottom of its range is a box
//! that does not come up at all — on hardware that is under a television, has no keyboard plugged
//! in, and is reached over ssh that will not answer until it has booted.
//!
//! The script is a `stdin`→`stdout` filter precisely so that it can be tried against a dozen
//! starting files here rather than once, destructively, over there. Everything below is a file that
//! exists somewhere in the wild.
//!
//! Unix only: it runs `sh`, so this covers the Linux container `tools/platform/linux/check.sh` uses
//! and macOS, and is compiled out on Windows. The theme test beside this one is not, because it
//! parses text and starts no process.
#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The script, three directories up out of this crate.
///
/// It lives under `tools/` rather than in the package because nothing installed ever runs it — it
/// is part of setting a box up, not part of how the machine starts, which is the line
/// `wait-for-drm.sh` falls on the other side of. Reaching up is the cost of putting each file where
/// it belongs, and it is cheaper than shipping a script into `/opt` that the package never calls.
fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tools/platform/linux/grub-appliance-edit.sh")
}

fn filter(input: &str) -> String {
    let mut child = Command::new("sh")
        .arg(script())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run grub-appliance-edit.sh");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write the starting file");

    let out = child.wait_with_output().expect("wait for the filter");
    assert!(
        out.status.success(),
        "the filter exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("the filter wrote non-UTF-8")
}

/// The value of one assignment in the output, or `None` if it is not there.
///
/// Reads the *last* one, which is the one the shell that sources this file will end up with — so a
/// test cannot pass on an assignment that a later line overrides. That is not hypothetical: the
/// script appends what it did not find, and appending a key that was already present further up is
/// exactly the bug this notices.
fn value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').to_string())
        .next_back()
}

fn words(text: &str) -> Vec<String> {
    value(text, "GRUB_CMDLINE_LINUX_DEFAULT")
        .expect("a command line")
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Everything the script promises, on every input.
fn assert_appliance(output: &str) {
    assert_eq!(
        value(output, "GRUB_TIMEOUT_STYLE").as_deref(),
        Some("hidden"),
        "the menu is still drawn"
    );

    // Above zero, and the number is not what is being asserted — that a UEFI box has *some* window
    // in which a keypress reaches the menu is. `keystatus` does not exist under EFI, so a zero here
    // is a machine with no way back short of a live USB.
    let timeout: u32 = value(output, "GRUB_TIMEOUT")
        .expect("a timeout")
        .parse()
        .expect("the timeout is a number");
    assert!(
        timeout > 0,
        "a hidden menu with a zero timeout has no way in"
    );

    let words = words(output);
    for wanted in [
        "quiet",
        "splash",
        "loglevel=3",
        "vt.global_cursor_default=0",
        "systemd.show_status=false",
    ] {
        assert!(
            words.iter().any(|w| w == wanted),
            "`{wanted}` is missing from {words:?}"
        );
    }

    assert_eq!(
        value(output, "GRUB_GFXPAYLOAD_LINUX").as_deref(),
        Some("keep")
    );
}

/// The file a Debian box actually ships, which is the one input that matters most.
const DEBIAN: &str = "\
# If you change this file, run 'update-grub' afterwards to update
# /boot/grub/grub.cfg.
GRUB_DEFAULT=0
GRUB_TIMEOUT=5
GRUB_DISTRIBUTOR=`( . /etc/os-release && echo ${NAME} )`
GRUB_CMDLINE_LINUX_DEFAULT=\"quiet\"
GRUB_CMDLINE_LINUX=\"\"
";

#[test]
fn a_stock_debian_file_becomes_an_appliances() {
    assert_appliance(&filter(DEBIAN));
}

/// **The property the caller depends on and cannot check for itself.**
///
/// `appliance-boot.sh` has no way to know whether a box has been through this before — a re-run
/// after a Debian upgrade rewrote the file, a second operator, a `--revert` that was never
/// followed through. A filter that appends a little more each time is how a kernel command line
/// ends up reading `quiet quiet quiet`.
#[test]
fn running_it_twice_changes_nothing() {
    let once = filter(DEBIAN);
    assert_eq!(filter(&once), once, "the second pass is not the first");
}

/// Somebody else's settings survive, in their own order.
#[test]
fn what_it_does_not_recognise_it_does_not_touch() {
    let input = "\
GRUB_DEFAULT=saved
GRUB_SAVEDEFAULT=true
GRUB_BADRAM=0x01234567,0xfefefefe
GRUB_CMDLINE_LINUX=\"consoleblank=0\"
GRUB_TERMINAL=console
";
    let output = filter(input);
    assert_appliance(&output);

    for kept in [
        ("GRUB_DEFAULT", "saved"),
        ("GRUB_SAVEDEFAULT", "true"),
        ("GRUB_BADRAM", "0x01234567,0xfefefefe"),
        ("GRUB_CMDLINE_LINUX", "consoleblank=0"),
        ("GRUB_TERMINAL", "console"),
    ] {
        assert_eq!(
            value(&output, kept.0).as_deref(),
            Some(kept.1),
            "{} was changed",
            kept.0
        );
    }
}

/// A commented-out assignment is documentation, not a setting.
///
/// Every distribution's shipped file is half comments, and several of them are commented-out
/// examples of the very keys this script sets. Uncommenting one would be the script answering a
/// question nobody asked it — and, for `#GRUB_TIMEOUT_STYLE=hidden`, doing so twice.
#[test]
fn commented_lines_stay_commented() {
    let input = "\
#GRUB_TIMEOUT_STYLE=hidden
# GRUB_TIMEOUT=0
#GRUB_GFXPAYLOAD_LINUX=text
GRUB_TIMEOUT=5
GRUB_CMDLINE_LINUX_DEFAULT=\"quiet\"
";
    let output = filter(input);
    assert_appliance(&output);

    for line in [
        "#GRUB_TIMEOUT_STYLE=hidden",
        "# GRUB_TIMEOUT=0",
        "#GRUB_GFXPAYLOAD_LINUX=text",
    ] {
        assert!(
            output.lines().any(|l| l == line),
            "`{line}` did not survive verbatim"
        );
    }
}

/// A setting the box already has, with the wrong value.
///
/// `loglevel` is the one that matters: appending `loglevel=3` beside an existing `loglevel=7` gives
/// a kernel command line where the last one wins. That happens to produce the right answer and is
/// still a bug — it reads as a mistake to everybody who sees it afterwards, and the day the
/// precedence is the other way round it stops working. The token is replaced where it stands.
#[test]
fn a_setting_with_the_wrong_value_is_replaced_in_place() {
    let output = filter("GRUB_CMDLINE_LINUX_DEFAULT=\"loglevel=7 nomodeset splash\"\n");
    assert_appliance(&output);

    let words = words(&output);
    assert_eq!(
        words.iter().filter(|w| w.starts_with("loglevel=")).count(),
        1,
        "two loglevels in {words:?}"
    );
    assert_eq!(
        words.iter().filter(|w| *w == "splash").count(),
        1,
        "splash was added beside itself in {words:?}"
    );

    // Not ours to remove, and worth being explicit that we leave it: `nomodeset` will stop the
    // machine drawing at all, which `appliance-boot.sh` warns about rather than silently fixing.
    // Deleting a word somebody put on their own kernel command line is a bigger liberty than any
    // this script takes.
    assert!(words.iter().any(|w| w == "nomodeset"), "{words:?}");

    // Position kept. Reordering a command line is a change nobody asked for and nobody can see the
    // reason for in a diff.
    assert_eq!(words[0], "loglevel=3");
    assert_eq!(words[1], "nomodeset");
}

/// Both other ways a value gets quoted.
#[test]
fn single_quoted_and_bare_values_are_read() {
    for input in [
        "GRUB_CMDLINE_LINUX_DEFAULT='quiet'\n",
        "GRUB_CMDLINE_LINUX_DEFAULT=quiet\n",
    ] {
        let output = filter(input);
        assert_appliance(&output);
        assert_eq!(
            words(&output).iter().filter(|w| *w == "quiet").count(),
            1,
            "from {input:?}"
        );
    }
}

/// Nothing to work from at all.
///
/// An empty or absent `/etc/default/grub` is legal — every key has a default. The script has to
/// produce a whole appliance configuration rather than a partial one, because there is no earlier
/// line for it to have merged into.
#[test]
fn an_empty_file_becomes_a_whole_configuration() {
    assert_appliance(&filter(""));
}

/// The file it produced, handed to it again after a distribution upgrade rewrote parts of it.
#[test]
fn its_own_output_with_a_key_reverted_is_repaired() {
    let once = filter(DEBIAN);
    let broken = once.replace("GRUB_TIMEOUT_STYLE=hidden", "GRUB_TIMEOUT_STYLE=menu");
    assert_ne!(broken, once, "the fixture did not change anything");
    assert_appliance(&filter(&broken));
}
