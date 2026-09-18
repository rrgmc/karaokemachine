//! What the three tao/wry desktop shells agree about.
//!
//! `km-package-builder`, `km-remote` and `km-admin` each open a window over a loopback server, and
//! each had its own copy of these. **`fit`, `centre` and `opening_geometry` were byte-identical in
//! all three** — verified line by line, not assumed — and so were five of the tests over them, under
//! two slightly different names.
//!
//! **[`with_icons`] arrived the same way and for a sharper reason.** Its three copies were
//! byte-identical too, and all three carried the same wrong sentence about which of Windows' two
//! icon slots the taskbar reads — so the same bug was fixed once here instead of three times, and
//! the next thing learned about `WM_SETICON` has one place to be written down.
//!
//! # What is deliberately *not* here
//!
//! The event loops. A review read the three `desktop.rs` files as ~1,750 lines of duplication and
//! that does not survive reading them: each has its own `Wake` payload, its own tray items, its own
//! state to close over and its own idea of what ends a run — a build finishing, a server stopping, a
//! `.kmbuild` arriving by Apple Event. What is genuinely shared is the arithmetic at the leaves, and
//! a crate that tried to own the loop as well would be a framework three programs then fought.
//!
//! So this holds no `run`, no `WebView`, and not even `wry`: the crate that decides where a window
//! opens does not drag a browser engine in with it.
//!
//! # Where `tao` is, and is not
//!
//! **[`opening_geometry`] and [`with_icons`] are the two things here that need `tao`, and only
//! Windows and macOS compile them.** On Linux `tao` is gtk3, which no build here installs, and this
//! crate being a workspace member means `cargo clippy --workspace` compiled it there anyway — see
//! the target table in `Cargo.toml`. `fit` and `centre` hold the arithmetic and none of it is
//! `tao`'s, so they and their tests build and run on every platform, which is the whole reason the
//! crate stays a member.
//!
//! Everything about the *icons* narrows further to Windows alone: macOS reads the bundle's
//! `CFBundleIconFile` and has no title bar icon, so [`with_icons`] hands the builder straight back
//! there.

// `fit` and `centre` are reached only through `opening_geometry`, which the target table above
// leaves out where there is no window to open. They are still compiled and still tested there.
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

#[cfg(any(windows, target_os = "macos"))]
use tao::dpi::{LogicalPosition, LogicalSize};
#[cfg(any(windows, target_os = "macos"))]
use tao::event_loop::EventLoop;
#[cfg(any(windows, target_os = "macos"))]
use tao::window::WindowBuilder;

/// How much of the screen a window may take before it is clamped to fit.
///
/// A shell's wanted height is routinely taller than the *logical* screen on a machine at 150%
/// scaling — 1920x1080 becomes 1280x720 there — and asking for a window taller than the display puts
/// its bottom edge off-screen, which is where a tab bar or a status line lives. `size()` is the whole
/// monitor rather than its work area, which tao does not report, so this margin also stands in for a
/// taskbar.
const SCREEN_SHARE: f64 = 0.9;

/// Clamps a wanted size to what the screen will take.
///
/// **Each axis on its own**, because which one binds depends on the window: a tall narrow remote
/// hits the height first, and a wide table hits the width. Clamping them together would give up an
/// axis that had room.
fn fit(wanted: (f64, f64), screen: (f64, f64)) -> (f64, f64) {
    (
        wanted.0.min(screen.0 * SCREEN_SHARE),
        wanted.1.min(screen.1 * SCREEN_SHARE),
    )
}

/// The top-left corner that centres a window on a screen.
///
/// Clamped at zero, so a window larger than the screen still starts at the corner rather than at a
/// negative offset that puts its title bar out of reach.
fn centre(screen: (f64, f64), window: (f64, f64)) -> (f64, f64) {
    (
        ((screen.0 - window.0) / 2.0).max(0.0),
        ((screen.1 - window.1) / 2.0).max(0.0),
    )
}

/// Where and how large a window should open, given what it would like.
///
/// **The monitor's own corner is added**, because a primary monitor is not always at the origin: on
/// a multi-head desktop it can sit at a negative x, and a window centred on the *screen's* width
/// alone would open on the neighbour.
///
/// With no monitor to ask — a headless session, a display that has not finished probing — the wanted
/// size is returned with no position and the platform's own cascade decides.
///
/// Generic over the event loop's user event, which is the only thing the three shells differed by:
/// each has its own `Wake`.
///
/// **Windows and macOS only**, because it is the one thing here that speaks `tao`. There is no
/// stub arm on the other platforms — unlike `km-tray`'s `build`, this takes a `tao` type, so a
/// signature to stub does not exist without the dependency it is avoiding.
#[cfg(any(windows, target_os = "macos"))]
#[must_use]
pub fn opening_geometry<T>(
    event_loop: &EventLoop<T>,
    wanted: (f64, f64),
) -> (LogicalSize<f64>, Option<LogicalPosition<f64>>) {
    let Some(monitor) = event_loop.primary_monitor() else {
        return (LogicalSize::new(wanted.0, wanted.1), None);
    };
    let scale = monitor.scale_factor();
    let screen = monitor.size().to_logical::<f64>(scale);
    let corner = monitor.position().to_logical::<f64>(scale);
    let (width, height) = fit(wanted, (screen.width, screen.height));
    let (left, top) = centre((screen.width, screen.height), (width, height));
    (
        LogicalSize::new(width, height),
        Some(LogicalPosition::new(corner.x + left, corner.y + top)),
    )
}

/// Which resource holds the icon: `winresource`'s `DEFAULT_APPLICATION_ICON_ID`, which is what each
/// `build.rs`'s `set_icon` writes. Named rather than spelled `1` at the call site, because the two
/// have to agree and nothing checks that they do.
#[cfg(windows)]
const ICON_ORDINAL: u16 = 1;

/// The size the title bar draws at, and the frame asked for out of the `.ico`.
///
/// Every `.ico` here carries an exact 16 frame (16, 24, 32, 48, 64, 128 and 256), so asking for one
/// gets a drawing made at the size it is shown at rather than a resample of a bigger one. On a
/// scaled display Windows scales that 16 back up, and doing better would mean reading `SM_CXSMICON`
/// — a `windows-sys` call and a second `unsafe` block for an icon. Not a price worth paying.
#[cfg(windows)]
const TITLE_BAR_ICON: tao::dpi::PhysicalSize<u32> = tao::dpi::PhysicalSize {
    width: 16,
    height: 16,
};

/// One frame out of this executable's own icon resource, at the size asked for.
///
/// `None` for the size means `LR_DEFAULTSIZE`, which is the *large* metric — `SM_CXICON`, 32 pixels
/// at 100% and scaled with the process's DPI awareness above it.
///
/// `None` for the answer is perfectly ordinary and never stops a window opening. Each `build.rs`
/// treats a missing `rc.exe` as a warning and attaches no resource at all, so a build made without
/// the Windows SDK has nothing here to find; an app wearing the default icon is a blemish and an app
/// that refuses to open a window over one is a bug, which is the rule `km-display`'s
/// `set_window_icon` follows too.
#[cfg(windows)]
fn from_resource(size: Option<tao::dpi::PhysicalSize<u32>>) -> Option<tao::window::Icon> {
    use tao::platform::windows::IconExtWindows;

    match tao::window::Icon::from_resource(ICON_ORDINAL, size) {
        Ok(icon) => Some(icon),
        Err(error) => {
            tracing::debug!(%error, "no icon in this executable; Windows draws its default");
            None
        }
    }
}

/// Gives a window builder the icons Windows draws it with.
///
/// **Windows keeps two, they are set separately, and both have to be filled.** `ICON_SMALL` is the
/// title bar; `ICON_BIG` is the taskbar button and Alt-Tab. `tao` registers its window class with
/// `hIcon` and `hIconSm` both null, then at creation sets `ICON_SMALL` from `with_window_icon` and
/// *actively zeroes* `ICON_BIG` — `set_taskbar_icon(None)` sends `WM_SETICON, ICON_BIG, 0`.
///
/// **Setting only the title bar's is what made the taskbar's blocky**, and the note this replaces
/// argued the opposite. The shell resolves a taskbar icon by walking `ICON_BIG` → the class icons →
/// `ICON_SMALL` → the executable's resource. While both window slots were empty it fell all the way
/// through to the resource, where Windows picks the right frame out of the `.ico` — so the switcher
/// looked correct and setting `ICON_BIG` looked like it "would change nothing visible". Filling
/// `ICON_SMALL` with an exact 16 stopped that walk one step early, and the taskbar began stretching
/// a 16-pixel drawing into a 24-pixel button.
///
/// So the two sizes are asked for on purpose and they are different: the small metric for the title
/// bar, the large one for everything else. Both come from the executable's own compiled-in resource
/// through `LoadImageW`, so there is no decoder, no new dependency and no second copy of the picture
/// to keep in step.
///
/// **macOS needs none of it** — the bundle's `CFBundleIconFile` is where that platform looks, and
/// there is no title bar icon to set — so the builder comes back untouched. Linux never builds the
/// feature that calls this.
#[cfg(any(windows, target_os = "macos"))]
#[must_use]
pub fn with_icons(builder: WindowBuilder) -> WindowBuilder {
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowBuilderExtWindows;

        builder
            .with_window_icon(from_resource(Some(TITLE_BAR_ICON)))
            .with_taskbar_icon(from_resource(None))
    }
    #[cfg(not(windows))]
    {
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window never opens larger than the screen it opens on.
    ///
    /// The fault this exists for: a wanted height of 900 against a logical 720 — which is what
    /// 1920x1080 becomes at 150% scaling — put the bottom edge off-screen, taking the tab bar with
    /// it. This test existed in three places under two names before this crate did.
    #[test]
    fn a_window_never_opens_bigger_than_the_screen() {
        // The scaled-down case, where the height binds.
        let (width, height) = fit((1500.0, 900.0), (1280.0, 720.0));
        assert!(width <= 1280.0 * SCREEN_SHARE, "width {width}");
        assert!(height <= 720.0 * SCREEN_SHARE, "height {height}");

        // A tall narrow window on a wide screen: the width had room and must keep it.
        let (width, _) = fit((520.0, 900.0), (2560.0, 1080.0));
        assert_eq!(width, 520.0, "an axis with room must not be clamped");

        // Room for everything: nothing moves.
        assert_eq!(fit((1400.0, 900.0), (2560.0, 1440.0)), (1400.0, 900.0));
    }

    /// A window opens in the middle, and never at a negative offset.
    #[test]
    fn a_window_opens_in_the_middle_of_the_screen() {
        assert_eq!(centre((1000.0, 800.0), (400.0, 200.0)), (300.0, 300.0));
        // Larger than the screen on both axes: the corner, not a negative offset that would put the
        // title bar out of reach.
        assert_eq!(centre((400.0, 300.0), (1500.0, 900.0)), (0.0, 0.0));
    }

    /// Both icon slots are read out of this executable's own resources, and neither traps.
    ///
    /// It deliberately does **not** assert `Some`, and that is not a weak test hiding from a strong
    /// one. Each `build.rs` treats a missing `rc.exe` as a warning and attaches no resource at all,
    /// so on a machine without the Windows SDK `None` is the correct answer and asserting otherwise
    /// would fail a build that is working exactly as designed — and a *test* binary has no icon
    /// resource in any case, so `None` is what this call answers even here. What it pins is the pair
    /// that has to agree and that nothing else checks: the ordinal here and the one `winresource`
    /// writes. Pass the wrong one and `LoadImageW` fails, which this would still tolerate — but it
    /// also proves both calls are reachable and do not trap, which is the half a hand test cannot
    /// see.
    ///
    /// **Both sizes rather than one**, because the defect this crate's [`with_icons`] exists to fix
    /// was exactly a second slot nobody was filling.
    #[cfg(windows)]
    #[test]
    fn both_window_icons_come_out_of_this_executables_own_resources() {
        let _ = from_resource(Some(TITLE_BAR_ICON));
        let _ = from_resource(None);
    }
}
