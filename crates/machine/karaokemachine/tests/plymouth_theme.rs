//! That the boot splash is drawn in the machine's own colors.
//!
//! **A copy of a constant in a language that cannot read the constant.** Plymouth's script language
//! has no way to reach `km_display::theme::Theme`, so `linux/plymouth/karaokemachine.script` spells
//! two of its fields out as floats. The whole point of the splash is that the television goes from
//! the boot mark to the machine's idle screen without the ground changing — so the moment those
//! floats drift from `Theme`, the feature is quietly broken in the one way nobody looking at either
//! file would notice, and the one way no test of either half would catch.
//!
//! So this reads the theme's *text* and asserts each float rounds back to the exact byte the
//! `Color` holds. It fails whichever side moved, which is the property worth having: there is no
//! authoritative copy here to fix the other one from, only two that have to agree.
//!
//! Not `#![cfg(unix)]`, unlike the other shell-adjacent test in this directory — nothing here runs a
//! program, it reads a file and parses numbers, so it is as true on Windows as anywhere and there is
//! no reason to give up the coverage on the platform most of the editing happens on.

use std::path::PathBuf;

use km_display::theme::Theme;
use sdl3::pixels::Color;

/// The theme directory as it sits in the source tree, which is what the package installs from.
fn theme_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("linux")
        .join("plymouth")
}

fn script() -> String {
    let path = theme_dir().join("karaokemachine.script");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The three numbers inside the first `call(` in the file.
///
/// Deliberately crude — a `.script` is not a format worth writing a parser for, and a crude reader
/// that panics when it stops recognising the file is the correct failure: it means somebody changed
/// the shape of the line these assertions are about, and they should come back here and say what it
/// now means rather than have the check silently start passing on nothing.
fn floats_after(haystack: &str, call: &str) -> [f64; 3] {
    let start = haystack
        .find(call)
        .unwrap_or_else(|| panic!("the theme script no longer contains `{call}`"));
    let rest = &haystack[start + call.len()..];
    let end = rest
        .find(')')
        .unwrap_or_else(|| panic!("`{call}` is not closed"));

    let parts: Vec<f64> = rest[..end]
        .split(',')
        .map(|p| {
            p.trim()
                .parse()
                .unwrap_or_else(|e| panic!("`{call}` has a non-number in it: {e}"))
        })
        .collect();

    parts
        .try_into()
        .unwrap_or_else(|v: Vec<f64>| panic!("`{call}` takes three numbers, found {}", v.len()))
}

/// One `NAME = 0.5;` assignment.
fn float_named(haystack: &str, name: &str) -> f64 {
    let needle = format!("{name} = ");
    let start = haystack
        .find(&needle)
        .unwrap_or_else(|| panic!("the theme script no longer sets `{name}`"));
    let rest = &haystack[start + needle.len()..];
    let end = rest
        .find(';')
        .unwrap_or_else(|| panic!("`{name}` is not terminated"));
    rest[..end]
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("`{name}` is not a number: {e}"))
}

/// What a Plymouth float has to round back to.
///
/// Plymouth takes 0.0–1.0 and SDL holds 0–255, so the theme cannot spell a color exactly; what it
/// can do is name one that lands on the right byte. Asserting the *byte* rather than comparing
/// floats within some tolerance is what makes this test say something a reader can act on — the
/// failure message names two colors, not two decimals.
fn byte(value: f64) -> u8 {
    assert!(
        (0.0..=1.0).contains(&value),
        "a Plymouth color component is 0.0 to 1.0, found {value}"
    );
    (value * 255.0).round() as u8
}

fn assert_matches(what: &str, floats: [f64; 3], color: Color) {
    let found = Color::RGB(byte(floats[0]), byte(floats[1]), byte(floats[2]));
    assert_eq!(
        (found.r, found.g, found.b),
        (color.r, color.g, color.b),
        "the boot splash's {what} is #{:02X}{:02X}{:02X} and the theme's is #{:02X}{:02X}{:02X}. \
         One of the two moved; they have to agree, because the splash hands straight over to the \
         machine's own screen. The floats are in crates/machine/karaokemachine/linux/plymouth/\
         karaokemachine.script and the fields are in km_display::theme::Theme",
        found.r,
        found.g,
        found.b,
        color.r,
        color.g,
        color.b,
    );
}

#[test]
fn the_splash_ground_is_the_machines_ground() {
    let script = script();
    let theme = Theme::default();

    assert_matches(
        "top of the ground",
        floats_after(&script, "Window.SetBackgroundTopColor("),
        theme.background,
    );
    assert_matches(
        "bottom of the ground",
        floats_after(&script, "Window.SetBackgroundBottomColor("),
        theme.background,
    );
}

/// Flat, not a gradient.
///
/// Separate from the test above because it is a different claim: that one says each end is the right
/// color, this one says there is no gradient to notice. The machine's own ground is flat, and a
/// splash that fades top to bottom would be a second design that the first frame then wipes.
#[test]
fn the_splash_ground_is_flat() {
    let script = script();
    assert_eq!(
        floats_after(&script, "Window.SetBackgroundTopColor("),
        floats_after(&script, "Window.SetBackgroundBottomColor("),
        "the boot splash's ground is a gradient; the machine's is flat"
    );
}

#[test]
fn the_splash_text_is_the_machines_pending_lyric() {
    let script = script();
    assert_matches(
        "prompt text",
        [
            float_named(&script, "TEXT_RED"),
            float_named(&script, "TEXT_GREEN"),
            float_named(&script, "TEXT_BLUE"),
        ],
        Theme::default().lyric_pending,
    );
}

/// Where the theme says its own files are.
///
/// `ImageDir` and `ScriptFile` are absolute paths into `/usr/share/plymouth/themes`, so they are a
/// promise about where the package installs — and the package's asset list is three directories
/// away in `Cargo.toml`. Plymouth resolves `Image("logo.png")` against `ImageDir`, so a mismatch is
/// a splash that comes up as a bare ground with no mark on it, on a box that has already rebuilt its
/// initramfs.
#[test]
fn the_theme_points_at_where_the_package_installs_it() {
    let path = theme_dir().join("karaokemachine.plymouth");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

    let installed = "/usr/share/plymouth/themes/karaokemachine";
    for line in [
        "ModuleName=script",
        &format!("ImageDir={installed}"),
        &format!("ScriptFile={installed}/karaokemachine.script"),
    ] {
        assert!(
            text.lines().any(|l| l.trim() == line),
            "karaokemachine.plymouth has no `{line}` line"
        );
    }
}

/// No comment in the theme file may name a key.
///
/// **This one is a bug that happened**, and it is worth the test because nothing about the failure
/// points at the file that caused it. Debian's `plymouth-set-default-theme` does not parse the
/// theme; it runs `grep "ModuleName *= *"` over the whole thing, comments included. A comment block
/// that opened with the words `ModuleName=script` was therefore a second match, the two lines went
/// into a `[ ! -e … ]`, and the script failed with `[: too many arguments` — naming its own line
/// number and nothing else.
///
/// The keys are checked one at a time rather than as a set, because the message that matters is
/// *which* key a rewording brought back.
#[test]
fn no_key_appears_twice_in_the_theme_file() {
    let path = theme_dir().join("karaokemachine.plymouth");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

    // `grep "Key *= *"` matches anywhere in a line, not only at its start — that is the whole
    // reason a *comment* could break it, and a test anchored at the start would pass on the exact
    // file that failed. The word-boundary condition is what keeps `Name` from matching inside
    // `ModuleName`, which grep would do and which is not the hazard being described.
    fn assigned_here(line: &str, key: &str) -> bool {
        line.match_indices(key).any(|(at, _)| {
            let boundary = at == 0
                || !line[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
            boundary
                && line[at + key.len()..]
                    .trim_start_matches(' ')
                    .starts_with('=')
        })
    }

    for key in [
        "ModuleName",
        "ImageDir",
        "ScriptFile",
        "Name",
        "Description",
    ] {
        let matches: Vec<&str> = text.lines().filter(|l| assigned_here(l, key)).collect();
        assert_eq!(
            matches.len(),
            1,
            "`{key}` is assigned on {} lines of karaokemachine.plymouth, and Debian's \
             plymouth-set-default-theme greps for it without skipping comments:\n{}",
            matches.len(),
            matches.join("\n")
        );
    }
}
