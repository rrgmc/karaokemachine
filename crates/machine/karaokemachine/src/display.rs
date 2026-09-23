//! The window, and the loop that drives it.
//!
//! Deferred out of M4 so the drawing could be built and reviewed headlessly first. What is here is
//! only the parts that genuinely need a screen: creating the window, turning decoded images into
//! textures, pumping SDL events, and calling [`km_display::draw::draw`] once per frame. Every
//! decision about *what* to show was settled in `km-display` and is unit-tested there.
//!
//! Three rules the loop obeys:
//!
//! * **No blocking work on this thread.** Images are decoded and downscaled by the loader thread;
//!   this thread only uploads. A 4K JPEG must never be opened here.
//! * **No locks held across a draw.** Machine state is copied into a snapshot at the top of the
//!   frame, so a slow HTTP request cannot stutter the display.
//! * **SDL stays on the thread that created it.** This runs on the main thread, which macOS
//!   requires, so everything else in the process is a worker.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use km_api::ApiState;
use km_catalog::search::SearchQuery;
use km_display::draw::{Background, Frame, Screen, SongInfo, draw};
use km_display::lyrics::LyricView;
use km_display::numbers::{NumberAction, NumberEntry};
use km_display::text::Fonts;
use km_display::theme::Theme;
use km_display::wallpaper::{Fit, Loader, Playlist, Schedule, Shuffle, WallpaperConfig};
use km_display::{BrowserKey, DisplayAction, action_for, pixel_density, window_to_pixels};
use km_queue::Transport;
use sdl3::event::Event as SdlEvent;
use sdl3::keyboard::Mod;
use sdl3::mouse::MouseUtil;
use sdl3::pixels::PixelFormat;
use sdl3::rect::Rect;
use sdl3::render::{Canvas, ScaleMode, Texture, TextureCreator};
use sdl3::video::{FullscreenType, Window, WindowContext, WindowFlags, WindowPos};

use km_songcode::SongCode;

use crate::dropped::{DropInstaller, DropStatus};
use crate::machine::{CatalogCounts, Machine, SongPreviewLookup};
use crate::settings::WindowRect;
use km_api::machine::{Controller as _, NowPlaying, SettingsPatch, TransportCommand};

/// Which build this is, as the idle screen's corner says it.
///
/// **Resolved here rather than in `km-display`.** One version number covers the whole repository, so
/// that crate reading its own `CARGO_PKG_VERSION` would print the same six characters — and it would
/// be the wrong crate to ask, the way it is the wrong crate to know what a SoundFont bank or a demo
/// is. What the screen states is a fact about this program.
///
/// **The `v` is part of the string and not a label beside it.** There is no room in that corner for
/// a word, and a bare `1.8.0` under a wallpaper is a number with nothing saying what it counts. It
/// is not a catalog string either: a product name and a number are the same in every language, which
/// `Every program says which build it is` in docs/decisions/interface.md already settles for the
/// three programs that print it in their chrome.
pub(crate) const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// How long the connect panel shows itself after startup without being asked.
///
/// The address is useless if nobody sees it, and somebody switching the machine on is exactly who
/// needs it. Eight seconds is long enough to read a URL and point a phone at a QR code.
const CONNECT_GREETING: Duration = Duration::from_secs(8);

/// What the playing screen says while the machine is singing to itself.
///
/// **Two things, and the second is the one that has to be there.** Saying `DEMO` alone explains why
/// music is playing with an empty queue, a state otherwise indistinguishable from something stuck. But it leaves somebody waiting for a song to end so they
/// can have their turn, and a demo song has no queue entry to run out: the next one starts
/// immediately. Queueing is the way through, and nothing else on the screen says so.
///
/// **`QUEUE` and not `SKIP`.** A queue during a demo takes the deck by itself, so naming the skip
/// asks for a press the machine does not need. In capitals because it names an act somebody performs
/// on a keypad or a phone rather than describing one.
///
/// **Short because the television is the narrow screen, which is not the obvious way round.** The
/// row fits 29 characters at the bound `km-display`'s `the_whole_instruction_fits_on_every_screen`
/// measures, so the longer sentence — the one that says the song starts straight away — lives on
/// the remote, where there is room for it and where the queueing actually happens. This is the half
/// that cannot be worked out from anywhere else.
pub(crate) const DEMO_LABEL: &str = "DEMO · QUEUE to sing next";

/// Frame budget for a display that will not pace the loop itself.
///
/// **Vsync is requested now — see [`request_vsync`] — so on any ordinary display this is inert**, and
/// that is the point of it. `present()` blocks until the next refresh, which is a better clock than
/// any constant here could be: it is the display's own rate rather than a guess at it, and it follows
/// a screen that is 50, 60 or 120 Hz without being told.
///
/// This remains for the case the doc has always claimed and never had: a driver that refuses vsync, a
/// software renderer, a headless compositor. Then nothing else limits the loop, and 8 ms keeps it
/// from spinning at four thousand frames a second heating the room.
///
/// **The history is worth one line, because the comment here was wrong for years.** It said vsync was
/// requested when nothing called `SDL_SetRenderVSync`, which was nearly true at idle — a frame costs
/// about 15 ms on the appliance, so the loop settled near 65 fps by itself — and quite false under a
/// video song, where the draw drops to about 5.5 ms and this floor ran the loop at **118 fps into a
/// 60 Hz screen**, the compositor discarding half of it for about a third of a Cortex-A55.
const MIN_FRAME: Duration = Duration::from_millis(8);

/// Asks the renderer to present on the display's refresh, and reports whether it agreed.
///
/// **`sdl3` 0.18.4 wraps no vsync setting for a renderer**, which is why this is FFI rather than a
/// method call. `SwapInterval` in that crate is `SDL_GL_SetSwapInterval` and governs a GL context
/// this code does not own; SDL3 dropped SDL2's `SDL_RENDERER_PRESENTVSYNC` creation flag, and the
/// replacement is either this call or renderer-creation properties the crate does not expose either.
///
/// **`1` rather than `SDL_RENDERER_VSYNC_ADAPTIVE`.** Adaptive vsync tears instead of waiting when a
/// frame runs late, and on a television showing words over a picture a torn frame is worse than a
/// repeated one — the singer is reading. It is also the less widely supported value, and this asks
/// for the one every driver understands.
///
/// **A refusal is not an error.** SDL's own contract is that not every value works on every driver,
/// so this reports what happened and lets [`MIN_FRAME`] do the pacing, which is the fallback that
/// constant exists for. Returning the answer rather than logging it here keeps it on the same line as
/// the backend name, where somebody debugging will read both at once.
#[expect(
    unsafe_code,
    reason = "one FFI call to SDL to set renderer vsync; sdl3-rs wraps only the GL swap interval"
)]
fn request_vsync(canvas: &Canvas<Window>) -> bool {
    // SAFETY: `canvas.raw()` is the live `SDL_Renderer` this canvas owns, valid for as long as the
    // canvas is; the call takes it and an int by value, returns a plain bool, and borrows nothing.
    let granted = unsafe { sdl3::sys::render::SDL_SetRenderVSync(canvas.raw(), 1) };
    if !granted {
        // At `warn` because it silently costs a third of a core on the appliance and the machine will
        // look fine — the sort of fault nobody finds without being told.
        tracing::warn!(
            error = %sdl3::get_error(),
            "the renderer would not take vsync; falling back to the frame budget"
        );
    }
    granted
}

/// How long the transport strip stays on screen after being touched.
///
/// The screen is showing lyrics; a permanent bank of buttons over them would be worse than reaching
/// for a remote. Long enough to press two things in a row, short enough to get out of the way.
const STRIP_LINGER: Duration = Duration::from_secs(6);

/// How long the position bar stays on screen after somebody asks for it.
///
/// How far through the song it is is what somebody who has just pressed a transport key is asking.
/// The bar is a visitor rather than a fixture, because the row it takes is a row of the song's words
/// over a picture and a row a room reads past over the machine's own screen.
///
/// Its own constant beside [`STRIP_LINGER`] rather than the same one: one answers a touch and the
/// other answers a key, and either may move without the other.
const POSITION_LINGER: Duration = Duration::from_secs(6);

/// How much of each end of a song has the position bar drawn with nobody asking.
///
/// A song's length is what a room wants to know as one begins and how much is left is what it wants
/// to know as one ends, so the bar visits both ends and the words have the screen between them.
///
/// **Milliseconds rather than a [`Duration`], because this measures song position and not wall
/// clock.** [`STRIP_LINGER`] and [`POSITION_LINGER`] are added to an [`Instant`] to make a deadline;
/// this is compared against how far into the song the audio has reached, which is the only clock
/// that can answer *how much is left*.
const POSITION_WINDOW_MS: u32 = 30_000;

/// The same window over a song that brought its own words.
///
/// The row the bar takes there is a row of that song's words, and its author chose what goes in it,
/// so the visit is as short as the one a key press buys.
///
/// Its own constant beside [`POSITION_WINDOW_MS`] rather than a fraction of it: the two answer the
/// two screens, and either may move without the other.
const POSITION_WINDOW_PICTURE_MS: u32 = 6_000;

/// Whether the window is allowed to leave fullscreen.
///
/// Android is a fixed-function appliance: the activity is fullscreen by theme (`styles.xml`), there
/// is no desktop to return to, and a remote that dropped the television out of fullscreen would
/// leave nobody a way back. "The appliance is always fullscreen" is a decision rather than an
/// accident of the keycode table, so it is compiled out rather than merely unreachable.
///
/// **iOS answers the same way for the nearer half of that reason.** An application owns the screen
/// and there is no desktop behind it to come back to, so a window that stopped being fullscreen
/// would have nowhere to go. `UIRequiresFullScreen` says the same thing to the system.
///
/// This says nothing about *leaving* the app, which is a separate question that has twice had the
/// wrong answer. First the Android build could not be exited at all. Then `AC_BACK` was bound — but a
/// real Google TV remote sends **`Escape`**, not `AC_BACK`, so it kept quitting on one press. Both
/// keys now mean [`DisplayAction::Back`], and where this constant is set Escape has no fullscreen step
/// to take first.
const FULLSCREEN_IS_FIXED: bool = cfg!(any(target_os = "android", target_os = "ios"));

/// Whether the transport strip says which function key presses each of its buttons.
///
/// **Desktop only.** The hints exist to be *acted on*: `F1` under `PAUSE` is worth a line of text
/// because there is an `F1` within reach, and the strip is transient, so without the hints nobody
/// standing at the machine ever learns those keys are there. On Android there is no keyboard, so the
/// same line would name a key that does not exist and spend a third of every button saying so.
///
/// Compiled out rather than settable, unlike `display.number_pad` next door — and the two are not
/// inconsistent. That setting exists because a pad is the *only* way to type on a device without a
/// keyboard, so getting the platform guess wrong there leaves somebody unable to enter a number at
/// all; getting this one wrong costs a hint on a button that goes on working exactly as it did.
///
/// **iOS is the platform that makes that asymmetry worth having.** An iPad may well have a keyboard
/// attached and mostly has none, so this is a guess rather than a fact — and it is a guess whose
/// cost is one line of text on a button nobody loses.
const KEY_HINTS: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// Whether a package can be installed by dragging it onto the window.
///
/// A separate question from [`KEY_HINTS`] with the same answer today, and named separately because
/// the questions really are different: that one asks whether there is a keyboard, this asks whether
/// there is a file manager to drag out of. Android is windowed and has neither, but a future device
/// could easily have one and not the other.
///
/// The arm compiles everywhere; this exists so that the empty-catalog message does not *offer* the
/// route on a device where it cannot be taken.
///
/// **iOS is where the arm and the constant come apart, and it is worth knowing which is which.**
/// SDL's UIKit delegate turns `application:openURL:` into the very drop event this constant is
/// named after, so a package opened from the Files app arrives at that arm and is installed. What
/// there is no such thing as on a phone is *dragging a file onto a window*, so the message must not
/// suggest it. The constant governs the sentence, never the handler, which is why one is false here
/// and the other still runs. `dropped_path` below is the other half of that route.
pub(crate) const DRAG_AND_DROP: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// The path SDL means by a dropped file.
///
/// **On iOS the payload is a URL, and on every other platform it is a path.** SDL's UIKit delegate
/// answers `application:openURL:` by sending a drop event carrying `url.absoluteString`, so a
/// package reaching the machine through *Open in* arrives as `file:///private/var/…/song.kmpkg`
/// rather than as `/private/var/…/song.kmpkg`. `PathBuf::from` on that produces a relative path
/// beginning `file:` that resolves to nothing, and the install is refused as "not a package" — a
/// message that describes the file rather than the fault.
///
/// Written generically rather than behind a `cfg` because nothing else can reach this branch: no
/// filesystem path starts with `file://`. The percent-decoding matters as much as the prefix, since
/// a person's package is quite likely to have a space in its name.
fn dropped_path(filename: &str) -> PathBuf {
    let Some(rest) = filename.strip_prefix("file://") else {
        return PathBuf::from(filename);
    };
    // The authority component, which for a local file URL is empty or `localhost`. Anything else is
    // a remote address this cannot open, and is left to fail as the path it appears to be.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    // Decoded as bytes rather than as characters, because an escape is one *byte* of UTF-8: a name
    // with an accent in it arrives as two escapes that only mean something together.
    let src = rest.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(src.len());
    let mut at = 0;
    while at < src.len() {
        // A stray or truncated `%` is kept as itself. A name containing one is likelier than a
        // malformed URL, and dropping it would change the path without saying so.
        if src[at] == b'%' && at + 2 < src.len() {
            let hi = char::from(src[at + 1]).to_digit(16);
            let lo = char::from(src[at + 2]).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "two hex digits are one byte by construction"
                )]
                out.push((hi * 16 + lo) as u8);
                at += 3;
                continue;
            }
        }
        out.push(src[at]);
        at += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

/// Whether `F10` shows the packages folder in a file manager.
///
/// **A third question, not a third spelling of the same one.** [`KEY_HINTS`] asks whether there is a
/// keyboard, [`DRAG_AND_DROP`] whether there is somewhere to drag a file *from*, and this asks
/// whether there is a file manager to show a folder *in*. They share an answer today and are named
/// apart because a device could easily change one of them without changing the others — Android is
/// windowed and has none of the three, but a tablet with a keyboard and a Files app would have two.
///
/// Compiled out rather than settable, on the same argument [`KEY_HINTS`] makes: a wrong guess here
/// costs one key, where a wrong guess about the on-screen number pad leaves somebody unable to enter
/// a song number at all. On Android the folder is also the wrong thing to offer — songs reach a
/// device through the *public* directory, and there is no application there that would show it.
///
/// **The Debian appliance is not excluded by this and does not need to be.** A box under a
/// television has no desktop, so it has no `xdg-open` either, and the press comes back with a
/// `NotFound` that goes on the screen. That is the honest answer and a better one than a prediction
/// this constant cannot make: it cannot tell a bare TTY from a Linux desktop, and a machine that
/// refused a key it could actually have honored would be the worse failure of the two.
///
/// **iOS has a Files app and still answers no**, which is the case that shows this is a question
/// about *revealing a folder* rather than about file managers existing. Nothing on the platform
/// takes a directory and shows it to somebody; the container's `Documents` is reachable because the
/// bundle asks for it to be listed, not because an application can be pointed at it.
const FILE_MANAGER: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// Whether `F11` opens the singer's remote in a browser.
///
/// **The fourth question in this family, not a fourth spelling of the same one.** [`KEY_HINTS`] asks
/// whether there is a keyboard, [`DRAG_AND_DROP`] whether there is somewhere to drag a file *from*,
/// [`FILE_MANAGER`] whether there is a file manager to show a folder *in*, and this asks whether
/// there is a browser to hand a URL to.
///
/// It shares its answer with the two above and is named apart for their reason. The mechanism is the
/// same as `FILE_MANAGER`'s and so is the honesty about it: `km_osopen`'s non-Windows, non-macOS arm
/// is `xdg-open`, which Android has not got, so the press would come back `NotFound`. On Android the
/// browser is also the wrong thing to offer — it would open over the full-screen window the machine
/// *is*, on the one platform where the remote and the machine are the same screen.
///
/// **The Debian appliance is not excluded and does not need to be**, exactly as `FILE_MANAGER`
/// records: a box with no desktop has no `xdg-open`, the press comes back with the reason, and that
/// is a better answer than a constant that cannot tell a bare TTY from a Linux desktop pretending
/// otherwise.
///
/// **iOS falls into that same `xdg-open` arm**, which it has not got either, so the press would come
/// back `NotFound`. Android's second reason applies here too and is the stronger of the two: a
/// browser opened over the machine would cover the one screen the machine *is*.
const WEB_BROWSER: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// Which key the connect panel says out loud opens the remote here, if any.
///
/// **The sixth question in this family and the one that is not [`WEB_BROWSER`]**, which is worth
/// saying plainly because the two look like they should be one constant and are not. That one asks
/// whether the press is worth *attempting*, and the honest answer on a Linux box is yes: a desktop
/// opens the page, and a box without `xdg-open` comes back with the reason on the screen. This asks
/// whether to put the offer on the television **before** anybody presses anything, and a line the
/// appliance cannot honor is a line the appliance should not carry.
///
/// The constant cannot tell a bare TTY from a Linux desktop — [`FILE_MANAGER`] records that and
/// [`WEB_BROWSER`] leans on it — so Linux is left out of the promise rather than given one that
/// half of its installations break. Windows and macOS are the two where the press always lands:
/// `km_osopen` reaches `ShellExecute` and `open`, neither of which can be missing.
///
/// **A failed press explains itself and an absent line does not**, which is why the asymmetry runs
/// this way round rather than the other.
///
/// **macOS is offered the modified spelling, because the bare key never arrives there.** The window
/// server takes `F11` for *Show Desktop* before an application is offered the press, so a panel
/// naming it would be naming a key that does nothing on the one platform this line exists to serve.
/// `Ctrl+F11` is the same binding and reaches it everywhere; only what the panel promises splits.
pub(crate) const BROWSER_KEY_HINT: Option<BrowserKey> = if cfg!(target_os = "macos") {
    Some(BrowserKey::CtrlF11)
} else if cfg!(target_os = "windows") {
    Some(BrowserKey::F11)
} else {
    None
};

/// Whether `T` can hold the window in front of the other applications.
///
/// **The fifth question in this family, and the one nearest [`FULLSCREEN_IS_FIXED`] without being
/// it.** That constant asks whether the window may stop being fullscreen; this asks whether there
/// are other windows for it to stand in front of. Android has one application on the screen at a
/// time, so there is no stack to climb and nothing the key could do.
///
/// Named apart from `FULLSCREEN_IS_FIXED` even though they answer together today, because they are
/// answering about different things: a platform could easily fix the window at the full screen and
/// still stack it against something else.
///
/// **iOS answers no for Android's reason.** One application owns the screen, so there is no stack
/// to climb. Split View puts two of them side by side rather than in front of one another, and this
/// machine asks for `UIRequiresFullScreen` in any case.
const WINDOW_STACKS: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// How long an install report stays on screen after it finishes.
///
/// Long enough to look up from a file manager and read one line. It has to outlast the install's own
/// `installing …`, which it replaces, or a small package would report itself in a flicker.
const FLASH_DONE: Duration = Duration::from_secs(6);

/// How long a refused drop stays on screen.
///
/// Longer than [`FLASH_DONE`], because it is the only account anybody gets of why the songs are not
/// there and it carries a reason to read rather than a result to notice.
const FLASH_FAILED: Duration = Duration::from_secs(12);

/// `SDL_TOUCH_MOUSEID`: the `which` on a mouse event SDL invented from a touch.
///
/// SDL delivers **both** for one tap. `SDL_HINT_TOUCH_MOUSE_EVENTS` defaults on, so a finger produces
/// a `FingerDown` *and* a synthetic `MouseButtonDown` at the same point, which this loop handled in
/// two arms and counted twice — every digit on the on-screen keypad arrived doubled on a phone.
///
/// Filtering by id rather than turning the hint off, because the hint is global and useful: it is what
/// lets a touch drive anything that only understands a mouse. What is wrong here is handling the same
/// press through two paths, not the synthesis.
const TOUCH_MOUSEID: u32 = u32::MAX;

/// `SDL_MOUSE_TOUCHID`: the `touch_id` on a touch event SDL invented from a mouse.
///
/// The mirror image of the above, from `SDL_HINT_MOUSE_TOUCH_EVENTS`. Off by default everywhere we
/// run, and filtered anyway: the double-counting would be identical, and a one-line guard is cheaper
/// than rediscovering this from a bug report.
const MOUSE_TOUCHID: u64 = u64::MAX;

/// Goes fullscreen or comes back, and shows the cursor only when there is a window to point at.
///
/// Reported rather than fatal: a machine that will not start because a compositor refused fullscreen
/// is worse than one running in a window.
///
/// **`windowed` is the rect to come back *to*, and it has to be applied by hand.** The obvious
/// reading is that SDL restores the windowed size itself, and it does — but the window this machine
/// creates when it starts fullscreen is created at the *display's* mode (see `run`), so SDL's idea of
/// the windowed size is the whole screen. Without the resize below, leaving fullscreen produces a
/// window exactly the size of the panel and `F` looks like a dead key. The outward trip needs the
/// display's mode and the return trip needs the windowed rect; they are two different numbers.
fn apply_fullscreen(
    canvas: &mut Canvas<Window>,
    mouse: &MouseUtil,
    on: bool,
    windowed: WindowRect,
) {
    if let Err(error) = canvas.window_mut().set_fullscreen(on) {
        tracing::warn!(%error, on, "could not change fullscreen; leaving the window as it is");
    }
    if !on {
        // `SDL_SetWindowFullscreen` is asynchronous wherever a window manager has to agree, and a
        // size set before the change lands is overwritten by it. `SDL_SyncWindow` waits for it.
        canvas.window().sync();
        let WindowRect {
            position,
            width,
            height,
        } = windowed;
        let window = canvas.window_mut();
        if let Err(error) = window.set_size(width, height) {
            tracing::warn!(%error, width, height, "could not restore the windowed size");
        }
        // Where the window was before it went fullscreen, or centered where it has never been
        // anywhere. A window restored to the top-left of a screen it no longer fills reads as a
        // glitch rather than as a window.
        let (x, y) = position.map_or((WindowPos::Centered, WindowPos::Centered), |(x, y)| {
            (WindowPos::Positioned(x), WindowPos::Positioned(y))
        });
        window.set_position(x, y);
    }
    mouse.show_cursor(!on);
}

/// The window's rect right now, or `None` where it is not an ordinary window.
///
/// **Fullscreen, maximized and minimized are all skipped**, because none of them is a rect somebody
/// chose: a maximized window closed and reopened at the size of the screen can no longer be
/// un-maximized to anything, and a minimized one reports a place off the desktop on Windows.
fn plain_window_rect(canvas: &Canvas<Window>) -> Option<WindowRect> {
    let window = canvas.window();
    if is_fullscreen(canvas) || window.is_maximized() || window.is_minimized() {
        return None;
    }
    let (width, height) = window.size();
    Some(WindowRect {
        position: Some(window.position()),
        width,
        height,
    })
}

/// How much of a saved window's top edge must land on a display for the position to be used.
///
/// The title bar is what a window is dragged by, so a window whose title bar is off every screen
/// cannot be brought back without a keyboard trick most people do not know.
const REACHABLE_STRIP: (u32, u32) = (100, 32);

/// The saved position, if it would put the window's title bar on a display that is still there.
///
/// **A monitor unplugged since the last close is the case this exists for.** The position was
/// real when it was written; opened on a desk with one screen fewer, it is a window nobody can
/// see. Centered is the answer there, and so is a position with no displays to test it against.
fn reachable_position(rect: WindowRect, displays: &[Rect]) -> Option<(i32, i32)> {
    let (x, y) = rect.position?;
    let strip = Rect::new(
        x,
        y,
        rect.width.max(1),
        REACHABLE_STRIP.1.min(rect.height.max(1)),
    );
    let wanted = REACHABLE_STRIP.0.min(strip.width());
    displays
        .iter()
        .any(|display| {
            display
                .intersection(strip)
                .is_some_and(|shared| shared.width() >= wanted)
        })
        .then_some((x, y))
}

/// What a closing display should write back about where the window was, or `None` for nothing.
///
/// **Only a window closed in a window writes its rect.** One closed fullscreen keeps the rect it
/// had before, which is also the one `F` would have returned it to. There is no flag half, unlike
/// [`fullscreen_to_remember`]: `--fullscreen` and `--windowed` say how the window starts, not where
/// it sits. Android is excluded for [`fullscreen_to_remember`]'s platform reason.
fn window_rect_to_remember(closed_fullscreen: bool, windowed: WindowRect) -> Option<WindowRect> {
    (!FULLSCREEN_IS_FIXED && !closed_fullscreen).then_some(windowed)
}

/// Holds the window in front of the other applications, or lets it back into the stack.
///
/// Reported rather than fatal, on `apply_fullscreen`'s argument: a machine that will not start
/// because a compositor declined to raise it is worse than one behind a browser. Wayland is the
/// case that declines — a client there does not get to place itself in the stack — and the honest
/// outcome is a key that does nothing on that desktop rather than a machine that will not run on
/// it.
///
/// **One FFI call, because sdl3-rs wraps this only as a creation flag.** `WindowFlags::ALWAYS_ON_TOP`
/// can be asked for when the window is built and `PopupWindowBuilder` can set it; neither reaches a
/// window that already exists, and doing it at creation would leave the key with no way to undo
/// itself. `request_vsync` above is the same shape for the same reason.
#[expect(
    unsafe_code,
    reason = "one FFI call to SDL to restack a live window; sdl3-rs wraps only the creation flag"
)]
fn apply_always_on_top(canvas: &mut Canvas<Window>, on: bool) {
    // SAFETY: `window.raw()` is the live `SDL_Window` this canvas owns, valid for as long as the
    // canvas is; the call takes it and a bool by value, returns a plain bool, and borrows nothing.
    let applied =
        unsafe { sdl3::sys::video::SDL_SetWindowAlwaysOnTop(canvas.window_mut().raw(), on) };
    if !applied {
        tracing::warn!(
            error = %sdl3::get_error(),
            on,
            "the window manager would not restack the window; leaving it where it is"
        );
    }
}

/// Whether the window is in front of everything else right now.
///
/// Asked of SDL rather than tracked, exactly as [`is_fullscreen`] is and for its reason: a
/// compositor that refused the change must not leave this out of step with what is on screen. On
/// Wayland that is not a hypothetical.
fn is_always_on_top(canvas: &Canvas<Window>) -> bool {
    WindowFlags::from(canvas.window().window_flags()).contains(WindowFlags::ALWAYS_ON_TOP)
}

/// What a closing display should write back, or `None` for a run that must write nothing.
///
/// **Two rules, and the second is a platform fact rather than a preference.** A run told
/// `--fullscreen` or `--windowed` moves one process and writes nothing down, which is what
/// [`DisplayConfig::remember_fullscreen`] carries. And on Android the window is fullscreen whatever
/// settings say — [`FULLSCREEN_IS_FIXED`] — so writing that back would turn "this platform has no
/// windows" into a preference that then follows the data directory onto one that does.
///
/// A closure rather than a `bool`, so the window is not asked in the cases where the answer is
/// thrown away — and so this is testable, which asking a `Canvas` would not be.
fn fullscreen_to_remember(remember: bool, live: impl FnOnce() -> bool) -> Option<bool> {
    (remember && !FULLSCREEN_IS_FIXED).then(live)
}

/// What a closing display should write back about the window's place in the stack.
///
/// One rule where [`fullscreen_to_remember`] has two, and the missing one is the point: no flag
/// declares a run temporary here, so every run may speak for the machine. What remains is the
/// platform fact — where the window cannot stack, writing an answer back would turn "this platform
/// has one application on the screen" into a preference that then follows the data directory onto
/// one that has windows.
///
/// A closure for [`fullscreen_to_remember`]'s reasons: the window is not asked where the answer is
/// thrown away, and a `Canvas` could not be built in a test.
fn always_on_top_to_remember(live: impl FnOnce() -> bool) -> Option<bool> {
    WINDOW_STACKS.then(live)
}

/// Whether the window is fullscreen right now.
///
/// Asked of SDL rather than tracked, so a compositor that refused a change cannot leave this out of
/// step with what is on screen. `FullscreenType::Desktop` is an SDL2 leftover whose discriminant is
/// `SDL_WINDOW_MODAL` under SDL3, so anything-but-`Off` is the honest test.
fn is_fullscreen(canvas: &Canvas<Window>) -> bool {
    canvas.window().fullscreen_state() != FullscreenType::Off
}

/// Whether the window title has anything new to say.
///
/// The two names rather than the composed title, so the frame that answers *no* — which is every
/// frame but the handful where a song starts or ends — neither allocates nor formats.
fn title_changed(titled: &Option<(String, Option<String>)>, now: Option<&NowPlaying>) -> bool {
    match (titled, now) {
        (None, None) => false,
        (Some((title, artist)), Some(now)) => title != &now.title || artist != &now.artist,
        _ => true,
    }
}

/// What the window says while a song is up.
///
/// **Title, then artist, then the program**, which is the order `km_queue::QueueEntry::label` and
/// the queue overlay already put a song in, so the three surfaces naming one song name it the same
/// way. `base` is the product and its version, and it goes last because a taskbar button and an
/// alt-tab entry truncate from the right: the song is what somebody glancing at the bar is asking
/// for, and the version is what they go looking for deliberately.
///
/// An artist that is present but empty is an artist nobody named — the package builder writes one
/// for a file whose own metadata was junk — so it is dropped rather than drawn as a separator with
/// nothing after it.
fn song_window_title(title: &str, artist: Option<&str>, base: &str) -> String {
    match artist.map(str::trim).filter(|artist| !artist.is_empty()) {
        Some(artist) => format!("{title} — {artist} — {base}"),
        None => format!("{title} — {base}"),
    }
}

/// Every point size a screen of this height asks for.
///
/// Compared rather than the height itself: most of the resize events a drag produces round to the
/// same sizes, and reopening the font files for those would stutter the loop for nothing.
///
/// **Every face `Fonts::load` opens has to be listed here**, the narrow lyric ladder included, or a
/// height change would leave those at the old screen's size. Width is deliberately absent and stays
/// so: the ladder's sizes come from the height like every other face, and only the *choice* between
/// them is about width — which `Fonts::fit_lyric` makes per line, per frame, opening nothing.
fn font_sizes_for(theme: &Theme, height: u32) -> (u16, [u16; 3], u16, u16) {
    (
        theme.lyric_px(height),
        theme.lyric_fallback_px(height),
        theme.text_px(height),
        theme.small_px(height),
    )
}

/// Smooths a counter the audio thread only updates once per callback.
///
/// `km_audio::audio` stores both `position_ms` and `position_ticks` from inside the device callback,
/// so what the display reads is a staircase whose tread is the device's period — 170 ms on the
/// appliance, a clock ticking six times a second behind a renderer measured at a locked sixty. The
/// lyric wipe steps instead of gliding, and a video holds one picture for ten frames and jumps.
///
/// **The step is measured, not calculated**, and that is what makes one type serve both counters.
/// Ticks per callback depends on the song's tempo map and the tempo ratio, and milliseconds per
/// callback depends on the tempo ratio too; deriving either invites getting the arithmetic wrong in a
/// place with no test. Instead the difference between two consecutive reports *is* the step, whatever
/// units it is in and whatever the tempo is doing.
///
/// **It sweeps centered on the report, not forward from it:**
///
/// ```text
///   shown(t) = reported - step/2 + step * (t - t_report)/period
/// ```
///
/// Two properties follow, and both were bought the hard way.
///
/// *It does not move the timing.* The staircase showed `reported` for a whole period, so its mean was
/// `reported`; this sweeps from half a step below to half above, whose mean is also `reported`. The
/// lyric offset judged by eye against a real television stays valid — which matters, because the
/// first attempt at this swept *forward* from `reported` and drew up to a full period early, on top
/// of a report that already leads the sound because it describes the end of a buffer not yet heard.
/// It was reported from the sofa as "maybe a little worse", and it was.
///
/// *It joins up.* At `t_report + period` this shows `reported + step/2`, and the report arriving then
/// is `reported + step`, whose starting value is `reported + step/2` — the same number. No seam at
/// the step it exists to hide.
///
/// The device's true output latency would be better than assuming the timing was already right, but
/// cpal reports it as zero on this backend (`latency_ms=0` beside `period_ms=170` in the log), so
/// there is nothing to use. Hence preserving the timing that was actually judged good.
///
/// That zero is ALSA's answer, not cpal's limitation: CoreAudio reports `latency_ms=194` beside
/// `period_ms=11`. Nothing here uses it — the offset this would inform is a *display* offset judged
/// by eye against a real television, and it is the appliance that has one — but the sentence above
/// reads as though no backend ever answers, and one does.
struct StepSmoother {
    /// The last value the engine reported, and when this loop first saw it.
    anchor: Option<(u32, Instant)>,
    /// How far the last report moved. Zero until two have been seen, which shows the report as-is.
    step: u32,
}

impl StepSmoother {
    fn new() -> Self {
        Self {
            anchor: None,
            step: 0,
        }
    }

    /// `period` is the engine's callback size; zero means no stream has run and nothing is smoothed.
    fn smooth(&mut self, reported: u32, period_ms: u32, playing: bool, now: Instant) -> u32 {
        // Nothing is moving, so inventing movement would be a lie anyone can see. Dropping the anchor
        // is also what stops a pause being counted as song time when play resumes.
        if !playing || period_ms == 0 {
            self.anchor = None;
            self.step = 0;
            return reported;
        }
        match self.anchor {
            Some((at, _)) if at != reported => {
                // A fresh report. The distance it moved is the step from here on -- unless it is not
                // a step at all but a seek, which must land exactly rather than be swept towards.
                self.step = match reported.checked_sub(at) {
                    // Backwards is always a seek. Playback does not run in reverse.
                    None => 0,
                    // Judged against the step already established rather than a constant, because
                    // the unit differs between the two counters this serves: milliseconds for one,
                    // MIDI ticks for the other, and ticks per callback depends on the tempo and the
                    // song's division. A multiple of what this stream has actually been doing is
                    // meaningful in both; a fixed number is meaningful in neither, and let a
                    // 3,830-unit jump through as a step in the test that found this.
                    Some(moved)
                        if self.step != 0
                            && moved > self.step.saturating_mul(Self::SEEK_FACTOR) =>
                    {
                        0
                    }
                    Some(moved) => moved,
                };
                self.anchor = Some((reported, now));
            }
            None => {
                self.anchor = Some((reported, now));
                self.step = 0;
            }
            Some(_) => {}
        }
        let (at, seen) = match self.anchor {
            Some(anchor) => anchor,
            None => return reported,
        };
        let half = self.step / 2;
        let elapsed = now
            .duration_since(seen)
            .as_millis()
            .min(u128::from(period_ms));
        // Integer throughout: the values are milliseconds and MIDI ticks, and a fractional tick is
        // not a thing the lyric view can use.
        let swept = (u64::from(self.step) * elapsed as u64 / u64::from(period_ms)) as u32;
        at.saturating_sub(half).saturating_add(swept)
    }

    /// How many times the established step a report may jump before it is read as a seek.
    ///
    /// Four rather than two, because the step genuinely varies: a tempo change moves ticks per
    /// callback, and the device can hand over a short block. The first delta of a stream is accepted
    /// whatever it is -- there is nothing to compare it against, and the cost of being wrong is one
    /// period of slightly wrong position that the next report corrects.
    const SEEK_FACTOR: u32 = 4;
}

/// Rolling frame statistics, reported once a second when they were asked for.
///
/// **Only built when `--frame-stats` (or `KM_FRAME_STATS`) says so**, and then reported at `info`
/// rather than `debug`. A `debug!` here would ride the default filter, which raises `km_app` to
/// debug — so every shipped build would write this line to its console or its journal once a second
/// forever. A diagnostic is something you ask for by name; it should not be something
/// you discover you have been collecting. And having asked, you should not then have to work out
/// which log level it hides behind — hence `info`.
///
/// Exists because "the screen looks choppy" was reported from a sofa and nothing in this crate could
/// say whether it was true. It reports three different times on purpose:
///
/// * **draw** — how long a frame took to *build*, up to but not including `present`. This is what
///   says whether the machine can keep up: as it approaches the frame interval, it cannot.
/// * **present** — how long `present` took, which under vsync is the wait for the next refresh and
///   therefore the **slack** in the frame. Draw and present together fill one interval, so watching
///   present fall towards zero is watching the machine run out of room.
/// * **interval** — the wall time between frames, which is the other two plus any `MIN_FRAME`
///   padding. `1000 / interval` is the rate somebody watching would count.
///
/// **Draw excluded `present` only from the day vsync was asked for**, and that correction is the
/// reason there are three numbers rather than two. While `present` returned immediately the two were
/// interchangeable; once it blocks, a combined figure sits at the frame interval on every healthy
/// frame and reads exactly like a machine that cannot keep up. A metric whose alarming value is also
/// its resting value cannot report anything.
///
/// The worst of each is carried as well as the mean, because a stutter is a tail property: thirty
/// good frames and one 200 ms frame average out to something that looks fine and is not.
struct FrameMeter {
    window_started: Instant,
    frames: u32,
    draw_total: Duration,
    draw_worst: Duration,
    present_total: Duration,
    present_worst: Duration,
    interval_total: Duration,
    interval_worst: Duration,
    /// The decoder's side of the same second, as cumulative readings differenced per window.
    ///
    /// Here because `draw` and `interval` between them cannot tell a hesitation from a healthy
    /// second: both measure the display, and on the machine this was written for the display was
    /// keeping 60 fps exactly while the picture stopped. What stops is the decoder, and these are
    /// the three ways that shows — the sound running dry, pictures thrown away before anything could
    /// draw them, and pictures arriving after their moment had passed.
    ///
    /// Zero on every MIDI and CD+G song, and on a build without the `video` feature, in which case
    /// the three fields are simply always 0 rather than absent: a log line whose shape depends on
    /// the build is a log line nothing can parse.
    decode: DecodeCounts,
    /// Whether to write the line as well as keep the numbers.
    ///
    /// **This is what `--frame-stats` now means, and it is a narrowing.** The flag used to decide
    /// whether the meter existed at all; `F12` needs the numbers without the log, so the meter is
    /// built whenever *either* asks and this says which of them did. Turning the panel on must not
    /// start writing to somebody's journal, and asking for the log must go on working with nothing
    /// on screen.
    logging: bool,
    /// The last completed window, kept so the screen has something to draw between reports.
    ///
    /// Carried across the reset, like [`Self::decode`] and unlike everything else here: the reset
    /// clears a window's accumulators, and this is the finished answer the window produced. `None`
    /// until the first second has closed, which the panel draws as `measuring...`.
    last: Option<km_display::FrameStats>,
}

/// Cumulative decode-side readings, differenced into a per-window delta.
///
/// Every field is a total-for-the-song rather than a rate, which is why the meter keeps the previous
/// reading rather than a running sum. A song change resets all three at the source, so a window that
/// straddles one sees a negative delta — [`DecodeCounts::since`] saturates it to zero rather than
/// reporting a wrapped number for the one second a song ends in.
#[derive(Clone, Copy, Default)]
struct DecodeCounts {
    starved_ms: u32,
    dropped: u32,
    skipped: u32,
    /// Device underruns, which is the **only** one of these a MIDI song can move.
    ///
    /// The other three all describe a decoder feeding the player from another thread, and a MIDI
    /// song has none — `rustysynth` renders inside the audio callback. So a MIDI song that cannot
    /// hold its deadline shows up here and nowhere else, and on the appliance it has less headroom
    /// than the video decoder did: 82% of one core against a 117 ms period, on a thread that cannot
    /// be given more cores.
    xruns: u32,
}

/// What the machine did to the loaded song, gathered from the two places that know.
///
/// **The levelling and the corrections come from the machine, and everything else from the parsed
/// song the loop already holds.** That split is not arbitrary: a gain is a decision the machine took
/// about this playback, where a truncated track is a fact about the file, and the second kind costs
/// nothing to read because `current_song` was called at the top of the frame anyway.
///
/// `song` being `None` is also what says the loaded song is not a MIDI one, so no song kind has to
/// travel to `km-display` to be asked about. A video and an MP3+G song are told apart by asking the
/// machine for the second, which is the only other thing a non-MIDI song can be.
/// The key and tempo the badges name: the settings for a MIDI song, and none for any other kind.
///
/// **Only a MIDI song's key and tempo move.** The settings go on holding what the next MIDI song
/// will use while a recording plays, and a badge reading "key +2" over one would be a plain lie.
pub(crate) fn drawn_adjustments(
    kind: Option<km_catalog::SongKind>,
    settings: &km_api::machine::Settings,
) -> (i8, f32) {
    match kind {
        Some(kind) if !kind.is_midi() => (0, 1.0),
        _ => (settings.transpose, settings.tempo_ratio),
    }
}

fn song_stats(
    machine: &Machine,
    kind: Option<km_catalog::SongKind>,
    song: Option<&km_song::Song>,
) -> Option<km_display::SongStats> {
    let (gain, gain_source, bank_ignored, muted) = machine.song_levelling()?;
    let is_midi = kind.is_some_and(|kind| kind.is_midi());
    let midi = song.filter(|_| is_midi).map(|song| km_display::SongStats {
        kind: km_display::SongMedia::Midi,
        gain,
        gain_source,
        bank_ignored,
        muted,
        flavor: Some(song.flavor),
        dialect: Some(song.dialect),
        tracks: song.track_count,
        truncated_tracks: song.truncated_tracks.len(),
        missing_tracks: song.missing_tracks,
        repaired_notes: song.repaired_notes,
    });
    Some(midi.unwrap_or(km_display::SongStats {
        kind: if kind.is_some_and(|kind| kind.is_ultrastar()) {
            km_display::SongMedia::UltraStar
        } else if kind.is_some_and(|kind| kind.is_lrc()) {
            km_display::SongMedia::Lrc
        } else if machine.current_cdg().is_some() {
            km_display::SongMedia::Cdg
        } else {
            km_display::SongMedia::Video
        },
        gain,
        gain_source,
        bank_ignored: 0,
        muted: 0,
        flavor: None,
        dialect: None,
        tracks: 0,
        truncated_tracks: 0,
        missing_tracks: 0,
        repaired_notes: 0,
    }))
}

/// What the decoder has done to this song so far.
///
/// Two bodies rather than a `#[cfg]` inside one, for the reason [`crate::video`] gives for its two
/// `mod imp` blocks: the caller then has no `#[cfg]` of its own, and a build without video reads a
/// structure of zeroes rather than a differently shaped log line.
#[cfg(feature = "video")]
fn decode_counts(machine: &Machine) -> DecodeCounts {
    let (dropped, skipped) = machine.current_video().map_or((0, 0), |song| {
        let frames = song.frames();
        let counters = frames.counters();
        (counters.dropped(), counters.skipped())
    });
    DecodeCounts {
        starved_ms: machine.starved_ms(),
        dropped,
        skipped,
        xruns: machine.xruns(),
    }
}

/// The same, in a build that cannot decode video.
///
/// `starved_ms` is still asked for rather than written as 0: it is a `km-audio` reading and
/// `km-audio` has no video feature, so the honest thing is to report what it says. What it says is
/// always 0 here, because only a video song plays from a feed that can run dry.
#[cfg(not(feature = "video"))]
fn decode_counts(machine: &Machine) -> DecodeCounts {
    DecodeCounts {
        starved_ms: machine.starved_ms(),
        xruns: machine.xruns(),
        ..DecodeCounts::default()
    }
}

impl DecodeCounts {
    /// This reading minus the previous one, floored at zero across a song change.
    fn since(self, previous: Self) -> Self {
        Self {
            starved_ms: self.starved_ms.saturating_sub(previous.starved_ms),
            dropped: self.dropped.saturating_sub(previous.dropped),
            skipped: self.skipped.saturating_sub(previous.skipped),
            xruns: self.xruns.saturating_sub(previous.xruns),
        }
    }
}

impl FrameMeter {
    const REPORT_EVERY: Duration = Duration::from_secs(1);

    fn new(logging: bool) -> Self {
        Self {
            window_started: Instant::now(),
            frames: 0,
            draw_total: Duration::ZERO,
            draw_worst: Duration::ZERO,
            present_total: Duration::ZERO,
            present_worst: Duration::ZERO,
            interval_total: Duration::ZERO,
            interval_worst: Duration::ZERO,
            decode: DecodeCounts::default(),
            logging,
            last: None,
        }
    }

    /// The last completed window, or an empty one while the first is still open.
    ///
    /// [`km_display::FrameStats::frames`] is 0 in the second case and the panel draws that as
    /// `measuring...`, which is why this can hand back a default rather than an `Option` the caller
    /// would have to decide about.
    fn snapshot(&self) -> km_display::FrameStats {
        self.last.unwrap_or_default()
    }

    fn record(
        &mut self,
        draw: Duration,
        present: Duration,
        interval: Duration,
        decode: DecodeCounts,
    ) {
        self.frames += 1;
        self.draw_total += draw;
        self.draw_worst = self.draw_worst.max(draw);
        self.present_total += present;
        self.present_worst = self.present_worst.max(present);
        self.interval_total += interval;
        self.interval_worst = self.interval_worst.max(interval);

        if self.window_started.elapsed() < Self::REPORT_EVERY {
            return;
        }
        // Guarded rather than assumed: a window can in principle close on zero frames, and dividing
        // by that would take the display down over a diagnostic.
        let finished = (self.frames > 0).then(|| {
            let elapsed = self.window_started.elapsed();
            let per = f32::from(u16::try_from(self.frames.min(u32::from(u16::MAX))).unwrap_or(1));
            let ms = |d: Duration| d.as_secs_f32() * 1000.0;
            let delta = decode.since(self.decode);
            km_display::FrameStats {
                frames: self.frames,
                fps: per / elapsed.as_secs_f32(),
                draw_ms: ms(self.draw_total) / per,
                draw_worst_ms: ms(self.draw_worst),
                present_ms: ms(self.present_total) / per,
                present_worst_ms: ms(self.present_worst),
                interval_ms: ms(self.interval_total) / per,
                interval_worst_ms: ms(self.interval_worst),
                starved_ms: delta.starved_ms,
                dropped: delta.dropped,
                late: delta.skipped,
                xruns: delta.xruns,
            }
        });

        // **The window is measured whatever happens to it, and only the writing is optional.** The
        // panel and the log read the same numbers, taken the same way over the same second, which is
        // what lets somebody diagnose from the sofa and then ask for the log without wondering
        // whether the two agree.
        if let Some(stats) = finished
            && self.logging
        {
            tracing::info!(
                fps = format_args!("{:.1}", stats.fps),
                draw_ms = format_args!("{:.1}", stats.draw_ms),
                draw_worst_ms = format_args!("{:.1}", stats.draw_worst_ms),
                present_ms = format_args!("{:.1}", stats.present_ms),
                present_worst_ms = format_args!("{:.1}", stats.present_worst_ms),
                interval_ms = format_args!("{:.1}", stats.interval_ms),
                interval_worst_ms = format_args!("{:.1}", stats.interval_worst_ms),
                starved_ms = stats.starved_ms,
                pictures_dropped = stats.dropped,
                pictures_late = stats.late,
                xruns = stats.xruns,
                "frames"
            );
        }

        let logging = self.logging;
        // Kept rather than cleared when a window closes on no frames at all: the previous second is
        // a better thing to leave on screen than a panel that blinks back to `measuring...`.
        let last = finished.or(self.last);
        *self = Self::new(logging);
        // Carried across the reset, unlike everything else here: these are cumulative readings, and
        // a window that started from zero would report the whole song again every second.
        self.decode = decode;
        self.last = last;
    }
}

/// The message across the top of the screen, and when it stops being there.
///
/// **The clock lives here rather than in `km-display`**, which is handed a string and a color once
/// a frame and owns no state at all. It is one of five deadlines of this shape in this loop, beside
/// `CONNECT_GREETING`, `strip_until`, `position_until` and the keypad line's own, and it works the
/// same way: a value
/// compared against `Instant::now()` while the frame is being built, never a timer and never a
/// thread. The keypad's is the one exception and is a countdown rather than a deadline, because it
/// lives in `NumberEntry`, which is pure state that no test may hand a clock.
struct Flashed {
    text: String,
    kind: km_display::FlashKind,
    /// When it goes away, or `None` while the work is still going.
    ///
    /// Work in progress is replaced by its own result rather than timed out. An install of four
    /// thousand songs takes seconds and could take longer on a slow disk, and an `installing …` that
    /// vanished halfway would leave the machine looking as though it had quietly dropped the file.
    until: Option<Instant>,
}

impl Flashed {
    /// Whether it has been on screen long enough.
    fn expired(&self) -> bool {
        self.until.is_some_and(|until| Instant::now() >= until)
    }
}

impl From<DropStatus> for Flashed {
    fn from(status: DropStatus) -> Self {
        match status {
            DropStatus::Working(text) => Self {
                text,
                kind: km_display::FlashKind::Working,
                until: None,
            },
            DropStatus::Done(text) => Self {
                text,
                kind: km_display::FlashKind::Done,
                until: Some(Instant::now() + FLASH_DONE),
            },
            DropStatus::Failed(text) => Self {
                text,
                kind: km_display::FlashKind::Failed,
                until: Some(Instant::now() + FLASH_FAILED),
            },
        }
    }
}

/// The other thing that arrives while somebody is watching: a bank a setup program was asked to
/// fetch, downloading on the first start after an install.
///
/// Drawn exactly like a dropped package and deliberately not folded into [`DropStatus`] — see
/// [`crate::firstrun::Notice`]. The `Working` arm has no deadline for the same reason that one does
/// not: a percentage that vanished halfway would leave the machine looking as though it had given
/// up, and 261.9 MiB takes a while.
impl From<crate::firstrun::Notice> for Flashed {
    fn from(notice: crate::firstrun::Notice) -> Self {
        match notice {
            crate::firstrun::Notice::Working(text) => Self {
                text,
                kind: km_display::FlashKind::Working,
                until: None,
            },
            crate::firstrun::Notice::Done(text) => Self {
                text,
                kind: km_display::FlashKind::Done,
                until: Some(Instant::now() + FLASH_DONE),
            },
            crate::firstrun::Notice::Failed(text) => Self {
                text,
                kind: km_display::FlashKind::Failed,
                until: Some(Instant::now() + FLASH_FAILED),
            },
        }
    }
}

/// The keypad, rebuilt only when something it depends on changes.
///
/// Laying it out is cheap, but it allocates, and a display loop that allocates twelve keys sixty
/// times a second for no reason is the sort of thing that shows up later as jitter.
struct KeypadCache {
    keypad: km_display::Keypad,
    width: u32,
    height: u32,
    playing: bool,
    melody_available: bool,
    visible: bool,
}

/// The idle screen's catalog summary, recomputed only when the catalog moves.
///
/// The same bargain [`KeypadCache`] makes, against a different cost. Counting rows is cheap, but it
/// takes the library mutex, and this is the display thread — so the read is a `try_lock` through
/// [`Machine::catalog_counts`], and the answer is kept between frames rather than asked for again.
/// What decides whether to re-count is `catalog_version`, one indexed row from `meta` that moves if
/// and only if a package was installed or uninstalled.
///
/// **A busy library keeps the last answer on screen.** That is the point of holding it: an install
/// is the one moment the counts are both changing and unreadable, and blanking the line for its
/// duration would draw attention to the one second nobody should be looking at.
pub(crate) struct SummaryCache {
    /// The catalog version the counts below were taken at.
    version: Option<u64>,
    /// The last answer, or `None` before the first successful read.
    summary: Option<km_display::CatalogSummary>,
}

impl SummaryCache {
    pub(crate) fn new() -> Self {
        Self {
            version: None,
            summary: None,
        }
    }

    /// The summary to draw, re-counting only if the catalog changed.
    pub(crate) fn get(&mut self, machine: &Machine) -> Option<km_display::CatalogSummary> {
        match machine.catalog_counts(self.version) {
            CatalogCounts::Counted {
                version,
                songs,
                packages,
            } => {
                self.version = Some(version);
                self.summary = Some(km_display::CatalogSummary::new(songs, packages));
            }
            // Nothing to do for either: `Unchanged` means what is held is still right, and `Busy`
            // means nothing was learned. Neither is a reason to stop drawing what is on screen.
            CatalogCounts::Unchanged | CatalogCounts::Busy => {}
        }
        self.summary
    }
}

impl KeypadCache {
    pub(crate) fn new() -> Self {
        Self {
            keypad: km_display::Keypad::empty(),
            width: 0,
            height: 0,
            playing: false,
            melody_available: false,
            visible: false,
        }
    }

    /// The keypad as last laid out.
    ///
    /// What an event handler must hit-test against: it is what was actually *drawn* on the frame the
    /// user was looking at when they pressed. Rebuilding first would test against a layout nobody
    /// has seen yet.
    fn keypad(&self) -> &km_display::Keypad {
        &self.keypad
    }

    /// Moves the D-pad highlight. Returns whether it moved.
    fn move_focus(&mut self, direction: km_display::Direction) -> bool {
        self.keypad.move_focus(direction)
    }

    /// Drops the D-pad highlight.
    ///
    /// Called when somebody touches or clicks: the highlight then shows where the *remote* is, which
    /// is no longer where the attention is, and a stale highlight is worse than none.
    fn clear_focus(&mut self) {
        self.keypad.restore_focus(None);
    }

    /// Returns the keypad for these conditions, rebuilding it only if they changed.
    ///
    /// `theme` is not among the conditions because it is fixed for the run: the reserve the number
    /// pad leaves for the build number under it is a function of the theme and the size, and only
    /// the size can move while the machine is on.
    fn get(
        &mut self,
        theme: &Theme,
        width: u32,
        height: u32,
        playing: bool,
        melody_available: bool,
        visible: bool,
    ) -> &km_display::Keypad {
        let same = self.width == width
            && self.height == height
            && self.playing == playing
            && self.melody_available == melody_available
            && self.visible == visible;
        if !same {
            // Carried across the rebuild. Losing the D-pad's position on a resize, or when a song
            // starts, would be maddening with a remote in your hand; `restore_focus` drops it only
            // if the new layout is too small to hold it.
            let focus = self.keypad.focus();
            self.keypad = match (visible, playing) {
                (false, _) => km_display::Keypad::empty(),
                (true, false) => km_display::Keypad::idle(
                    width as f32,
                    height as f32,
                    km_display::version_reserve(theme, width as f32, height as f32),
                ),
                (true, true) => km_display::Keypad::playing(
                    width as f32,
                    height as f32,
                    melody_available,
                    KEY_HINTS,
                    km_display::position_reserve(height as f32),
                ),
            };
            self.keypad.restore_focus(focus);
            self.width = width;
            self.height = height;
            self.playing = playing;
            self.melody_available = melody_available;
            self.visible = visible;
        }
        &self.keypad
    }
}

/// How the window is set up.
#[derive(Debug, Clone)]
pub struct DisplayConfig {
    /// Start fullscreen.
    pub fullscreen: bool,
    /// Whether to write the window's fullscreen state back to settings when the display closes.
    ///
    /// **False for a run that was told what to do.** `F` and `Escape` toggle fullscreen at the
    /// machine, and a person who leaves it one way means it — losing that at every restart is the
    /// bug this closes. But `--fullscreen` and `--windowed` are documented as moving one process and
    /// writing nothing down, and that is worth keeping: a debugging run against an appliance's own
    /// `--data-dir` must not turn its television into a window from then on.
    ///
    /// So the flag decides whether this run is a statement about the machine or only about itself,
    /// and [`crate::run`] sets this from whether one was given. Android never sets it: fullscreen is
    /// fixed there, and there is nothing to remember.
    pub remember_fullscreen: bool,
    /// Start in front of the other applications.
    ///
    /// No `remember_` counterpart, because no flag sets this: the write-back at close is
    /// unconditional wherever the window stacks at all. See [`always_on_top_to_remember`].
    pub always_on_top: bool,
    /// Where the window goes when it is not fullscreen, and where it came back to last time.
    ///
    /// Written back at close whenever the window closed in a window, on any run. See
    /// [`window_rect_to_remember`].
    pub window: WindowRect,
    /// A font file to prefer.
    pub font: Option<PathBuf>,
    /// The bundled font, if this install has one. Tried after `font` and before system fonts.
    pub bundled_font: Option<PathBuf>,
    /// A font to stand behind the others for Han, Kana and Hangul.
    ///
    /// For a platform `km_display::text::find_cjk_fonts`'s list does not cover. Named alone when it
    /// is given: an answer plus guesses is not a better answer. See
    /// [`crate::settings::DisplaySettings::font_cjk`].
    pub font_cjk: Option<PathBuf>,
    /// Whether to show the on-screen keypad at all — both the number pad and the transport strip.
    ///
    /// On a touch device this is the only way to work the machine, so it defaults on. An owner
    /// driving it entirely from a keyboard or a phone can turn it off, and then a press lands
    /// nowhere. See [`crate::settings::DisplaySettings::keypad`].
    pub keypad: bool,
    /// Whether the idle screen carries the number pad. On by default only on Android.
    ///
    /// Subordinate to [`Self::keypad`]: this narrows what is drawn, it does not turn anything on.
    /// See [`crate::settings::DisplaySettings::number_pad`] for why the platform decides it.
    pub number_pad: bool,
    /// The wallpaper cycle.
    pub wallpaper: WallpaperConfig,
    /// **Write** frame statistics to the log once a second. Off unless somebody asked.
    ///
    /// See [`FrameMeter`]. This says only whether the meter *writes a line*: `F12` draws the same
    /// numbers on screen, so the meter is built whenever either of the two asks for it. Asking for the panel must not start filling somebody's journal, and
    /// asking for the log must go on working with nothing on screen.
    pub frame_stats: bool,
    /// Whether `F11` has a singer's remote to open.
    ///
    /// From `api.serve_remote`, which is the same value that decides whether `km-remote-pages` is
    /// merged at `/`. Carried here rather than asked of the API because the API cannot answer it:
    /// the router was built before the display started, and a machine serving the landing page looks
    /// from the outside exactly like one serving the remote.
    pub remote_served: bool,
}

/// One wallpaper, as a texture plus the name to report.
struct Wall {
    texture: Texture,
    name: String,
}

/// Frees a wallpaper's texture.
///
/// Explicit for the same reason as in `km_display::text`: the `unsafe_textures` feature is what lets
/// this struct hold a `Texture` at all without a lifetime tying it to the `TextureCreator`, and the
/// price is that dropping a `Wall` frees the `String` and leaks the texture. A wallpaper is only a few
/// megabytes and only changes every thirty seconds, so this leaks far more slowly than the per-frame
/// text did — slowly enough to have gone unnoticed, which is the reason to name it here.
#[expect(
    unsafe_code,
    reason = "Texture::destroy is unsafe only because it must precede its Canvas, and the render \
              loop that owns the canvas is what calls this"
)]
fn retire(wall: Option<Wall>) {
    if let Some(wall) = wall {
        // SAFETY: only ever called from the render loop, which holds the canvas that owns this
        // texture, so the renderer is alive — the single condition `destroy` requires.
        unsafe { wall.texture.destroy() }
    }
}

/// The video picture on screen, as a streaming texture.
///
/// One texture per song rather than per frame: `update_yuv` writes the decoder's three planes into
/// it in place, and the GPU does the YUV to RGB conversion while it draws. That is the whole reason
/// frames are never converted on the CPU — see `km_video::Frame`.
/// Shared by both kinds of song that put a picture where the wallpaper goes: a video's `IYUV`
/// planes, and an MP3+G song's `ARGB8888` CD+G surface. Only the format and the upload call differ.
struct PictureTexture {
    texture: Texture,
    width: u32,
    height: u32,
}

/// Frees a picture texture, for the same reason and under the same conditions as [`retire`].
///
/// **Not optional bookkeeping**: M8 found a GPU texture leak here that affected every platform, and
/// this is the lesson that came out of it.
#[expect(
    unsafe_code,
    reason = "Texture::destroy is unsafe only because it must precede its Canvas, and the render \
              loop that owns the canvas is what calls this"
)]
fn retire_picture(picture: Option<PictureTexture>) {
    if let Some(picture) = picture {
        // SAFETY: only ever called from the render loop, which holds the canvas that owns this
        // texture, so the renderer is alive — the single condition `destroy` requires.
        unsafe { picture.texture.destroy() }
    }
}

/// Which folder the wallpapers should be coming from, asked again.
///
/// The same question `WallpaperSettings::to_config` answers at startup, and asked the same way, so
/// the two cannot drift: an explicit `wallpaper.dir` in settings wins outright, and otherwise
/// `Paths::wallpaper_dir` chooses by contents between the owner's folder, an overlay and the shipped
/// set.
///
/// **It exists because that choice is not stable for the life of a run.** Adding the first picture
/// to an empty owner folder changes the answer, which is exactly what an upload does, and a display
/// holding the startup answer would watch the shipped folder for ever with the new picture sitting
/// unread in another one.
fn settings_wallpaper_dir(machine: &Machine) -> PathBuf {
    let settings = machine.settings();
    settings
        .wallpaper
        .dir
        .clone()
        .unwrap_or_else(|| machine.paths().wallpaper_dir().0)
}

/// The wallpaper cycle's state on the render thread.
struct Walls {
    playlist: Playlist,
    schedule: Schedule,
    loader: Loader,
    current: Option<Wall>,
    outgoing: Option<Wall>,
    /// A request is in flight, so the cycle does not ask twice for the same image.
    pending: bool,
    fit: Fit,
    dim: f32,
}

/// A seed for the wallpaper order, from the OS.
///
/// **The randomness is drawn here rather than in `km-display`**, which is the seam that crate keeps
/// on purpose: a display type that reached for entropy itself would be one no test could pin to an
/// order. `getrandom` is this workspace's entropy source everywhere else, and an evening that opened
/// on the same photograph as the last one is the whole of what a bad seed costs — so a failure is
/// worth a fixed number and a warning rather than a machine that will not start.
fn seed() -> u64 {
    let mut bytes = [0u8; 8];
    if let Err(error) = getrandom::fill(&mut bytes) {
        tracing::warn!(%error, "no entropy for the wallpaper order; using a fixed one");
        return 0x9E37_79B9_7F4A_7C15;
    }
    u64::from_ne_bytes(bytes)
}

impl Walls {
    fn start(config: &WallpaperConfig) -> Self {
        let mut playlist = Playlist::scan(&config.dir, &config.extra);
        if config.shuffle {
            playlist = playlist.shuffled(Shuffle::seeded(seed()));
        }
        tracing::info!(
            dir = %config.dir.display(),
            images = playlist.len(),
            shuffle = config.shuffle,
            "wallpaper folder"
        );
        Self {
            playlist,
            schedule: Schedule::new(config.interval, config.crossfade),
            loader: Loader::start(),
            current: None,
            outgoing: None,
            pending: false,
            fit: config.fit,
            dim: config.dim,
        }
    }

    /// Asks the loader for whatever should be shown next.
    ///
    /// Returns whether an image is actually on its way. `false` is an empty folder or a loader that
    /// has stopped: no result will arrive, so nothing will call `start_crossfade`, and a caller
    /// that is waiting on one has to stop waiting itself. A request already in flight answers
    /// `true` — its result is what ends the wait, and asking twice would spend two images on one
    /// change.
    fn request(&mut self, width: u32, height: u32) -> bool {
        if self.pending {
            return true;
        }
        let Some(source) = self.playlist.current().cloned() else {
            return false;
        };
        self.pending = self.loader.request(source, width, height, self.fit);
        self.pending
    }

    /// A problem worth putting on screen, when there is one.
    fn problem(&self) -> Option<String> {
        if self.playlist.is_empty() {
            Some("no images in the wallpaper folder".to_owned())
        } else {
            None
        }
    }
}

/// Why the display did not run.
///
/// **Two kinds, because only one of them is worth trying again.** [`DisplayError::Unavailable`]
/// means there was no screen to draw on *at that moment* — on the appliance that is a television
/// switched off at the wall, or a connector i915 has not finished probing. [`DisplayError::Failed`]
/// is everything else: no usable font, SDL_ttf missing, a renderer that would not create. Retrying
/// the second is a loop that never ends and a log that never stops.
///
/// **A type rather than a look at the message text**, so the boundary is structural: a `?` added
/// inside the body later cannot accidentally become retryable, because everything past the video
/// subsystem lives in a function that cannot return `Unavailable` at all.
#[derive(Debug, thiserror::Error)]
pub enum DisplayError {
    /// There is no video device. Nothing has been built, so trying again later costs one probe.
    #[error("no video device: {0}")]
    Unavailable(String),
    /// Something that will not fix itself by waiting.
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// Runs the display until the user quits or `shutdown` is set.
///
/// Setting `shutdown` on the way out is deliberate: closing the window is how somebody stops the
/// machine, and the rest of the process has to hear about it.
///
/// Returning [`DisplayError::Unavailable`] is not a failure of the machine — the caller keeps the
/// API, the queue and the audio running and asks again later. See the retry loop in [`crate::run`].
pub fn run(
    machine: Arc<Machine>,
    api: ApiState,
    config: DisplayConfig,
    shutdown: Arc<AtomicBool>,
) -> Result<(), DisplayError> {
    // **This is what actually holds the machine in landscape, and the manifest is not.**
    //
    // `AndroidManifest.xml` says `screenOrientation="userLandscape"`, and it is obeyed right up until
    // SDL creates the window: `Android_CreateWindow` calls `setOrientation(w, h, resizable,
    // SDL_GetHint(SDL_HINT_ORIENTATIONS))`, and `SDLActivity.setOrientationBis` ends every path with
    // an unconditional `setRequestedOrientation(...)` that replaces the manifest value for the life
    // of the activity. With no hint set, the branch it takes is chosen by **resizability, not by
    // width and height** — and our window is `.resizable()` below — so it lands on
    // `SCREEN_ORIENTATION_FULL_USER`: every orientation allowed, subject only to the device's
    // rotation lock. A Galaxy S23 duly came up 1080x2340, portrait, reporting
    // `requestedOrientation=13 resizable=true hint=`.
    //
    // **A television hides this completely.** A Google TV Streamer has one fixed landscape mode and
    // no rotation sensor, so `FULL_USER` is indistinguishable there from being locked — which is why
    // this survived being run on one.
    //
    // Naming both landscapes gives SDL `SCREEN_ORIENTATION_USER_LANDSCAPE`, which is the manifest's
    // own value: landscape either way up, never portrait. Set before `SDL_Init` because the hint's
    // documentation asks for that, and read by the Android and iOS backends alone — inert
    // everywhere else, so it needs no `cfg`.
    sdl3::hint::set(
        sdl3::hint::names::ORIENTATIONS,
        "LandscapeLeft LandscapeRight",
    );

    // `SDL_Init` with no subsystem flags touches no video driver at all, so a failure here is not a
    // display that is merely absent — it is SDL itself refusing, and no amount of waiting mends it.
    let sdl = sdl3::init().map_err(|error| anyhow::anyhow!("SDL would not start: {error}"))?;
    // **This is the appliance's race, and the only retryable line in the whole function.** SDL's
    // kmsdrm backend needs a connector that is connected *and* has a mode; during the second or so
    // that i915 takes to finish probing, and for as long as the television is switched off, there is
    // none and this is what it says.
    let video = sdl
        .video()
        .map_err(|error| DisplayError::Unavailable(error.to_string()))?;
    run_with(sdl, video, machine, api, config, shutdown).map_err(DisplayError::Failed)
}

/// Everything from the moment a display exists.
///
/// Split out so that the retryable boundary is a *signature* and not a convention: nothing reachable
/// from here can return [`DisplayError::Unavailable`], so nothing here can accidentally be retried
/// for ever.
///
/// # Why this is one long function
///
/// It is ~190 lines of setup and a ~780-line frame loop over 27 `let mut` bindings, which is the
/// largest single scope in the workspace and looks exactly like something that wants a `Screen`
/// struct with a `fn frame(&mut self)`. **The borrows permit that**:
/// [`km_display::text::TextCache`] owns its texture creator, and `canvas.texture_creator()` hands
/// out an independent handle to the same context, so one struct may hold the canvas and the cache
/// together in safe Rust. [`km_display::Offscreen`] is such a struct, for the windowless path.
///
/// What holds this shape is the 27 bindings rather than the borrow checker. [`StepSmoother`],
/// [`FrameMeter`], [`KeypadCache`], [`SummaryCache`] and [`Walls`] are separate types already, and
/// what remains in scope is per-frame state that a `Screen` would hold as fields without making any
/// of it easier to follow. Extracting phases as free functions is no better — every one of them
/// would take a dozen parameters out of this scope.
fn run_with(
    sdl: sdl3::Sdl,
    video: sdl3::VideoSubsystem,
    machine: Arc<Machine>,
    api: ApiState,
    // `mut` only for the wallpaper folder, which is re-resolved at each rescan rather than frozen
    // at startup. Everything else in here is read once and never written.
    mut config: DisplayConfig,
    shutdown: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let ttf =
        sdl3::ttf::init().map_err(|error| anyhow::anyhow!("SDL_ttf would not start: {error}"))?;

    // The rect this window comes back to from fullscreen, and the one written down at close. The
    // saved position is tested against the displays that are connected now, before anything is
    // placed with it. Kept up to date as the window is moved and resized.
    let mut windowed = {
        let displays: Vec<Rect> = video
            .displays()
            .map(|all| all.iter().filter_map(|d| d.get_bounds().ok()).collect())
            .unwrap_or_default();
        let position = reachable_position(config.window, &displays);
        if config.window.position.is_some() && position.is_none() {
            tracing::info!(
                rect = ?config.window,
                "the saved window position is on no connected display; centering it"
            );
        }
        WindowRect {
            position,
            ..config.window
        }
    };

    // A window that is about to go fullscreen is created at the *display's* size, not at
    // `config.window` -- that describes the windowed rect and nothing else.
    //
    // This is not cosmetic. On kmsdrm there is no desktop for "fullscreen desktop" to mean anything,
    // so SDL sets a video mode near the window's own size. A window at the windowed size therefore
    // drives the screen at that size: a 4K television runs at 720p and upscales, and a screen whose
    // aspect ratio is not 16:9 stretches on top of that. The first costs sharpness and the second
    // costs shape, and neither is visible on the 16:9 desktop most testing happens on. `modetest`
    // reports the mode the CRTC is actually scanning out, which is what settles it.
    //
    // `Display::get_mode` is SDL_GetDesktopDisplayMode, which on kmsdrm reports the connector's
    // preferred mode. Falling back to the configured size keeps a headless or odd video backend
    // working rather than refusing to open at all.
    let (window_width, window_height) = if config.fullscreen {
        match video.get_primary_display().and_then(|d| d.get_mode()) {
            Ok(mode) if mode.w > 0 && mode.h > 0 => {
                let (w, h) = (mode.w as u32, mode.h as u32);
                tracing::debug!(
                    width = w,
                    height = h,
                    "sizing the window to the display's mode"
                );
                (w, h)
            }
            Ok(_) => (config.window.width, config.window.height),
            Err(error) => {
                tracing::warn!(%error, "could not read the display mode; using the configured size");
                (config.window.width, config.window.height)
            }
        }
    } else {
        (config.window.width, config.window.height)
    };

    // **The version is on the window and in the idle screen's corner**, and the two are for two
    // different people. This is the surface an operator working at the box already looks at, and it
    // is the one that is not the show — so it carries the number in full, beside the product's name.
    // The corner is for the hosts with no window at all: an Android television and the appliance on
    // a bare TTY draw no taskbar, no dock and no title, and on those the screen is the only place a
    // build number can be read from. See `VERSION` and `Every program says which build it is` in
    // docs/decisions/interface.md.
    //
    // **This is the whole title while nothing is playing, and the tail of it while something is** —
    // see [`song_window_title`] for which way round the two go and why. The window opens on an idle
    // machine, so the bare form is what it opens with.
    let base_title = format!("KaraokeMachine {}", env!("CARGO_PKG_VERSION"));
    let mut builder = video.window(&base_title, window_width, window_height);
    // Where it was left, when that is still on a screen. A fullscreen start is centered whatever
    // was saved: the display's mode is the size, and the saved corner belongs to the smaller rect.
    match windowed.position {
        Some((x, y)) if !config.fullscreen => builder.position(x, y),
        _ => builder.position_centered(),
    };
    let mut window = builder
        .resizable()
        // Ask for the real pixels. Without this a Retina panel renders the whole display at
        // logical resolution and upscales it, which costs nothing but looks soft. Inert on
        // Android and Windows, whose window coordinates are physical pixels already.
        .high_pixel_density()
        .build()?;

    // Before the canvas takes the window: the taskbar, the dock and the alt-tab switcher all read
    // this, and none of them is reached by the icon an installer writes to disk. Inert on Android,
    // which has no window furniture to put one in.
    km_display::set_window_icon(&mut window);

    let mut canvas = window.into_canvas();
    // Which backend SDL actually chose and whether it will pace us, said once and together. Both are
    // decided by probing at run time, so the only way to know is to ask -- and they are the first two
    // questions any "why is this choppy?" has to answer: a software renderer is slow, and a renderer
    // that would not take vsync is drawing frames the screen will never show. On the appliance this
    // should say `opengles2`, which is what kmsdrm reaches through EGL/GBM.
    tracing::info!(
        renderer = %canvas.renderer_name,
        vsync = request_vsync(&canvas),
        "the SDL renderer backend"
    );
    let creator = canvas.texture_creator();
    // **Text rendered once and kept, rather than rebuilt sixty times a second.** Profiled on the
    // appliance, making text textures was about 38% of the app's CPU — two thirds of that the upload
    // into the GPU, not the glyph rasterising. It lives out here because a cache inside the frame
    // would be a cache of nothing.
    let mut text_cache = km_display::text::TextCache::new(canvas.texture_creator());
    apply_fullscreen(&mut canvas, &sdl.mouse(), config.fullscreen, windowed);
    // After the window exists rather than as a creation flag, so that starting in front and being
    // put there by `T` are one code path. Skipped where it would mean nothing, so a platform with
    // no window stack does not log a refusal every start.
    if WINDOW_STACKS && config.always_on_top {
        apply_always_on_top(&mut canvas, true);
    }
    let (mut width, mut height) = canvas
        .output_size()
        .unwrap_or((config.window.width, config.window.height));
    // Window coordinates to backbuffer pixels. Everything laid out below is in pixels; a mouse
    // event is not, on a display where these differ.
    let mut density = pixel_density(canvas.window().pixel_density());

    let theme = Theme::default();
    // **Without CJK faces to start with**, whatever this machine's catalog holds. Opening them is
    // twelve more `TTF_Font`s over files of 8 to 20 MB, and the corpus scan puts CJK at 0.3% of it —
    // so the machine that pays for them should be the one that has just been asked to draw one. The
    // rebuild below is what does that, and it is the same seam a resize already goes through.
    let mut with_cjk = false;
    let mut fonts = Fonts::discover(
        &ttf,
        config.font.as_deref(),
        config.bundled_font.as_deref(),
        config.font_cjk.as_deref(),
        with_cjk,
        &theme,
        height,
    )
    .map_err(|error| {
        anyhow::anyhow!("no usable font: {error}. Set display.font in settings.json")
    })?;
    // What `fonts` was built for. Compared on a resize so the common case — a drag that moves the
    // height without moving any rounded point size — does no work.
    let mut font_sizes = font_sizes_for(&theme, height);

    let mut walls = Walls::start(&config.wallpaper);
    // **Which folder the wallpapers come from is not fixed for the run.**
    // `Paths::wallpaper_dir` chooses between the owner's folder, an overlay and the shipped set *by
    // contents*, and `to_config` resolves it once — so on a machine whose `wallpapers/` is empty at
    // boot, which is every machine before its first upload, this loop would watch the shipped
    // folder and go on watching it. `take_wallpaper_dir_stale` is what asks for the
    // question to be put again.
    let mut wallpaper_dir = config.wallpaper.dir.clone();
    #[cfg(feature = "video")]
    let mut video: Option<PictureTexture> = None;
    // No `cfg`: an MP3+G song draws in every build.
    let mut cdg: Option<PictureTexture> = None;
    walls.request(width, height);
    // Which of the four rules produced the folder in use. Held in the loop rather than asked for at
    // each report, because it is re-decided at the rescan seam below and both reporters have to
    // agree with what that decided.
    let mut wall_source = machine.wallpaper_folder().1;
    machine.set_wallpaper_state(None, walls.playlist.len(), walls.problem(), wall_source);

    let mut event_pump = sdl
        .event_pump()
        .map_err(|error| anyhow::anyhow!("no event pump: {error}"))?;

    // **A watch rather than an arm in the loop below, and it has to be.** On Android the activity
    // stopping is what parks this very thread: SDL's Java calls `pauseNativeThread()`, and
    // `poll_iter()` does not run again until the app comes back — so an `AppWillEnterBackground`
    // arm in the `match` would be read on return, minutes after it mattered, which is exactly no
    // use. A watch is called synchronously on the thread that pushed the event, before the parking
    // takes effect, which is why SDL documents this as the way to handle these four.
    //
    // That thread is Android's UI thread, so the callback may not block: `set_foreground` is an
    // atomic store and deliberately nothing else, and [`Machine::poll`] on the watchdog thread does
    // the work fifty milliseconds later.
    //
    // Both are bound to locals for the life of the loop, and neither binding is tidiness:
    // `EventWatch` deregisters when it drops, and the subsystem it came from should outlive it.
    // A `let _ = ...` here would drop the watch on the spot and remove it again.
    let event_subsystem = sdl
        .event()
        .map_err(|error| anyhow::anyhow!("no event subsystem: {error}"))?;
    let _visibility_watch = {
        let machine = Arc::clone(&machine);
        event_subsystem.add_event_watch(move |event| match event {
            SdlEvent::AppWillEnterBackground { .. } => machine.set_foreground(false),
            SdlEvent::AppDidEnterForeground { .. } => machine.set_foreground(true),
            _ => {}
        })
    };

    let mut entry = NumberEntry::new();
    let mut view = LyricView::for_ticks_per_quarter(480);
    let mut show_connect = false;
    let mut show_queue = false;
    let started = Instant::now();
    let mut last_frame = Instant::now();
    // **Once, before the loop, and that is not laziness.** Both switches behind this take effect at
    // the next start — what they change is which routes get mounted — so the *running* answer cannot
    // move for the life of this process. The pages draw the stored value, and say which is which.
    let developer_mode = developer_mode(api.config());

    // Built when *either* the flag or the key asks for it. `F12` turns it on and off below; the
    // flag is what decides whether it also writes a line.
    let mut frames = config.frame_stats.then(|| FrameMeter::new(true));
    // **`show_perf` was a `bool` this loop owned and is now the machine's**, read fresh inside the
    // loop rather than carried across iterations. Three pages can move it — the two admin surfaces
    // and `/dev/` — because the appliance has no keyboard to press `F12` on, which is the machine
    // this diagnostic is most for. A local that persisted and a flag would be two answers.
    let mut ms_clock = StepSmoother::new();
    let mut tick_clock = StepSmoother::new();
    let mut loaded_beat_ticks = 0_u16;
    let mut keypad_cache = KeypadCache::new();
    let mut summary_cache = SummaryCache::new();
    // When the transport strip stops showing. `None` means it is not up.
    let mut strip_until: Option<Instant> = None;
    // Whether `Ctrl+F12` has taken the timer away. A local, and one that dies with the process:
    // see `DisplayAction::ToggleStripPin`, whose whole argument is that nothing is written down.
    let mut strip_pinned = false;
    // When the position bar stops standing over a song that brought its own words. `None` means it
    // is waiting for somebody to want it, which over such a song is most of the song.
    //
    // **A second deadline rather than `strip_until` read again**, because the two answer different
    // presses. Only a D-pad move and a touch raise the strip, so a desktop keyboard raises it never
    // and a bar that followed it would be absent from the machine the owner is sitting at.
    let mut position_until: Option<Instant> = None;
    // Packages dragged onto the window, installed off this thread, and whatever the last one had to
    // say. Started whether or not anything is ever dropped: one idle worker costs nothing, and the
    // alternative is deciding on the first drop whether the machine is somewhere drops happen.
    let mut installer = DropInstaller::spawn(Arc::clone(&machine));
    let mut flash: Option<Flashed> = None;
    // What the window title was last built from, rather than the title itself. `SDL_SetWindowTitle`
    // is a round trip to the window manager, so it is worth making only when the song changes —
    // and comparing the two names a song is drawn from costs nothing, where composing the string to
    // compare it would allocate on every one of sixty frames a second. A local of `run_with` and
    // not a field, because [`run`] opens a second window after a display goes away.
    let mut titled: Option<(String, Option<String>)> = None;
    // What `F10` had to say, which is only ever a failure -- see [`show_packages_folder`]. A channel
    // rather than a return value because the opener is not run on this thread, and one channel for
    // the life of the loop rather than one per press because a `Receiver` has to outlive the frame
    // the press happened in.
    let (opened_tx, opened_rx) = std::sync::mpsc::channel::<String>();

    // `width`/`height` are backbuffer pixels, so on a high-density screen they are larger than the
    // window: 1280x720 at density 2 logs 2560x1440. Worth printing, because a click landing in the
    // wrong place is almost always this number being unexpected.
    tracing::info!(width, height, density, "the display is up");

    'running: loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        // **The panel's flag, reconciled once per frame rather than on the keypress.** `F12` is no
        // longer the only thing that can move it, so the meter has to be built and dropped where
        // *any* change is noticed. One atomic load per frame, which is what `foreground` already
        // costs and is nothing beside a frame.
        //
        // The meter is dropped again unless `--frame-stats` asked for it, so a machine nobody is
        // diagnosing goes back to doing no arithmetic per frame -- and one that *was* asked for the
        // log keeps writing it.
        let show_perf = machine.performance_overlay_on();
        if show_perf && frames.is_none() {
            frames = Some(FrameMeter::new(false));
        } else if !show_perf && !config.frame_stats {
            frames = None;
        }

        for event in event_pump.poll_iter() {
            match event {
                SdlEvent::Quit { .. } => {
                    // Worth naming, because "the display is closing" alone does not say who asked.
                    // On Android this is the system having finished the activity — a BACK that SDL's
                    // Java passed to `super.onBackPressed()` rather than delivering to us as a key.
                    tracing::debug!(
                        "SDL delivered a quit event; something outside asked us to stop"
                    );
                    break 'running;
                }
                // Follows the window as it is dragged and resized, so both the return from
                // fullscreen and the write at close have the rect somebody last chose. Asked of the
                // window rather than taken from the event, because the event may describe a
                // maximize or a fullscreen change that has already landed.
                SdlEvent::Window {
                    win_event:
                        sdl3::event::WindowEvent::Moved(..) | sdl3::event::WindowEvent::Resized(..),
                    ..
                } => {
                    if let Some(rect) = plain_window_rect(&canvas) {
                        windowed = rect;
                    }
                }
                SdlEvent::Window {
                    win_event: sdl3::event::WindowEvent::PixelSizeChanged(new_width, new_height),
                    ..
                } => {
                    width = new_width.max(1) as u32;
                    height = new_height.max(1) as u32;
                    // A move to a display of a different density changes this, not just a resize.
                    density = pixel_density(canvas.window().pixel_density());
                    // The images on screen were downscaled for the old size, so ask again.
                    walls.request(width, height);
                    // Rebuilding opens font files, which this thread is otherwise careful never to
                    // do — but a resize is rare, and the guard means a drag does it once rather
                    // than per frame. The alternative is visible: leaving fullscreen would draw
                    // glyphs sized for the big screen into the small one.
                    let wanted = font_sizes_for(&theme, height);
                    if wanted != font_sizes {
                        match Fonts::discover(
                            &ttf,
                            config.font.as_deref(),
                            config.bundled_font.as_deref(),
                            config.font_cjk.as_deref(),
                            with_cjk,
                            &theme,
                            height,
                        ) {
                            Ok(rebuilt) => {
                                fonts = rebuilt;
                                font_sizes = wanted;
                                // **The one thing the cache cannot work out for itself.** Its keys
                                // carry the address of the font that drew each string, and dropping
                                // the old faces frees addresses that the new ones can be handed
                                // straight back — so without this the next frame would draw the old
                                // size's glyphs from a stale entry and look almost right. See
                                // `TextCache`'s `Key`.
                                text_cache.clear();
                            }
                            // Keep the old fonts. Wrongly sized text beats ending the singing.
                            Err(error) => {
                                tracing::warn!(%error, height, "could not resize the fonts");
                            }
                        }
                    }
                }
                SdlEvent::KeyDown {
                    keycode: Some(key),
                    keymod,
                    repeat,
                    ..
                } => {
                    // Either Control key. SDL has no combined constant, and the two are the same
                    // key as far as any binding here is concerned.
                    let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
                    let Some(action) = action_for(key, ctrl) else {
                        // Named rather than silently dropped: a remote whose button arrives as an
                        // unexpected keycode is indistinguishable from a dead button otherwise, and
                        // that is exactly the confusion the TV's BACK caused.
                        tracing::debug!(?key, "no binding for this key");
                        continue;
                    };
                    // Auto-repeat is for a key somebody means to hold. None of these is: a held `F`
                    // toggles fullscreen at the repeat rate and lands on whichever parity it stopped
                    // on -- often the one it started from, which looks exactly like the key not
                    // working -- and a held Escape or BACK walks past the leave-fullscreen step into
                    // `break 'running`, ending the evening. A held `F10` is the same fault with the
                    // loudest symptom of the four: thirty file-manager windows, none of which this
                    // process can close again. A held `Ctrl+F10` is the quietest and the most
                    // expensive: thirty rescans queued behind one another, each re-indexing every
                    // package in the folders, and the cap then refusing the one the owner meant.
                    // A held `D` is the fullscreen fault with a worse ending than a wrong parity:
                    // each repeat that lands on "on" also asks for a song, so a catalog would be
                    // drawn from thirty times and the mode would settle wherever the key was let
                    // go. A held `Ctrl+2` is worse than any of them: each repeat drops the audio
                    // stream and rebuilds the synthesizer, so the song would stutter for as long as
                    // the key was down and might not survive it -- three failed opens and the
                    // engine gives up on the song altogether. Every other binding is either
                    // idempotent or worth repeating, so this stays a short list rather than a
                    // blanket filter.
                    if repeat
                        && matches!(
                            action,
                            DisplayAction::ToggleFullscreen
                                // A held `T` and a held `Ctrl+F12` are the fullscreen fault
                                // exactly: both toggle at the repeat rate and settle on whichever
                                // parity the key was let go on, which is most often the one it
                                // started from — a key that looks broken.
                                | DisplayAction::ToggleAlwaysOnTop
                                | DisplayAction::ToggleStripPin
                                | DisplayAction::Escape
                                | DisplayAction::Back
                                | DisplayAction::OpenPackagesFolder
                                | DisplayAction::OpenRemote
                                | DisplayAction::TogglePerformance
                                | DisplayAction::RescanPackages
                                | DisplayAction::SelectSoundFont(_)
                                | DisplayAction::ToggleDemo
                                // Held `Ctrl+Q` would ask to shut down once per repeat. The first
                                // one is the only one that can mean anything and the rest arrive at
                                // an operating system already going down.
                                | DisplayAction::Quit
                        )
                    {
                        continue;
                    }
                    // **Any bound key means somebody is standing there**, so the position bar comes
                    // back over a song that brought its own words. Here rather than on the transport
                    // actions alone: every key on this machine is somebody working it, and a bar
                    // that appeared for `PAUSE` and not for `KEY +` would be a rule nobody could
                    // learn. Before the `match` below, because several of its arms `continue`.
                    position_until = Some(Instant::now() + POSITION_LINGER);
                    // A television remote's D-pad arrives here as arrow keys, and its OK button as
                    // Return — SDL translates both. So the remote needs no handling of its own; it
                    // needs the arrows to move a highlight and Return to press what is highlighted.
                    let action = match action {
                        DisplayAction::Focus(direction) => {
                            // **Move first, then wake, and the order is the behavior.** While the
                            // strip is hidden the cache holds `Keypad::empty()`, so this call does
                            // nothing and the press only reveals the strip; the next one moves the
                            // highlight. That is the same reveal-then-act rule `press` states for
                            // touch, and for the same reason — a remote in a lap is as brushable as
                            // a screen, and acting on the first press would skip a song. Confirmed
                            // on a Google TV remote: three presses to press a button.
                            //
                            // So do not "fix" this by waking before moving, and do not let a change
                            // to the cache quietly drop it. It reads like an off-by-one and is a
                            // policy.
                            keypad_cache.move_focus(direction);
                            // Moving the highlight is the whole action. Also wake the transport
                            // strip, so the D-pad can reach it during a song.
                            strip_until = Some(Instant::now() + STRIP_LINGER);
                            continue;
                        }
                        // With something highlighted, OK presses it. With nothing highlighted —
                        // a desktop, where nobody has touched an arrow — Return still submits the
                        // number that was typed.
                        DisplayAction::Submit => keypad_cache.keypad().activate().unwrap_or(action),
                        // Needs the window, like the two below it and for the same reason.
                        DisplayAction::ToggleAlwaysOnTop => {
                            if WINDOW_STACKS {
                                let on = is_always_on_top(&canvas);
                                apply_always_on_top(&mut canvas, !on);
                            } else {
                                tracing::debug!("this build has no window stack to climb");
                            }
                            continue;
                        }
                        // **Pinning is the loop's own business and nothing else's.** The flag lives
                        // here rather than on the machine — unlike the frame overlay's, which
                        // `/admin/` can also move — because the strip is drawn by this loop, is
                        // reached from the keyboard in front of it, and is a diagnostic that
                        // deliberately does not outlive the process.
                        DisplayAction::ToggleStripPin => {
                            strip_pinned = !strip_pinned;
                            tracing::debug!(strip_pinned, "the transport strip's timer");
                            continue;
                        }
                        // Both of these need the window, which `handle` has no access to.
                        DisplayAction::ToggleFullscreen => {
                            if FULLSCREEN_IS_FIXED {
                                tracing::debug!("this build is always fullscreen");
                            } else {
                                let on = is_fullscreen(&canvas);
                                // Read on the way out rather than trusting the last move event,
                                // which may still be in the queue behind this key.
                                if let Some(rect) = plain_window_rect(&canvas) {
                                    windowed = rect;
                                }
                                apply_fullscreen(&mut canvas, &sdl.mouse(), !on, windowed);
                            }
                            continue;
                        }
                        // The escape hatch: out of fullscreen first, and quit only from a window, so
                        // one keypress cannot end a song by accident.
                        //
                        // On an appliance that safeguard does not exist, because there is no
                        // fullscreen to leave — and it turns out that is exactly where it was needed.
                        // **A Google TV remote's BACK button arrives here as `Escape`, not as
                        // `AC_BACK`**, so Escape was quitting outright on the first press, mid-song,
                        // with none of `Back`'s level logic. Verified on the device: an injected
                        // `KEYCODE_ESCAPE` reproduces the exact log signature a real BACK press
                        // leaves. So where fullscreen is fixed, Escape *is* the remote's back button
                        // and must mean the same thing.
                        DisplayAction::Escape => {
                            if !FULLSCREEN_IS_FIXED {
                                if is_fullscreen(&canvas) {
                                    apply_fullscreen(&mut canvas, &sdl.mouse(), false, windowed);
                                    continue;
                                }
                                break 'running;
                            }
                            DisplayAction::Back
                        }
                        // **`Ctrl+Q` means two different things and they are the same intention.**
                        // On a desktop it ends the application, which is what quit means where
                        // there is a desktop to go back to. On the appliance there is not one: the
                        // machine *is* the box, so ending the application would leave a television
                        // showing a console for the two seconds it takes `Restart=always` to bring
                        // it back — a quit that visibly does not quit. There, it turns the box off,
                        // which is what somebody at the keyboard meant.
                        //
                        // Which of the two is not a `cfg`: `FULLSCREEN_IS_FIXED` is Android, not
                        // Linux, and there is no compile-time notion of "appliance" here. It is the
                        // same runtime capability the admin page reads, so a `cargo run` on a Linux
                        // desktop quits and does not switch the developer's machine off.
                        //
                        // **It does not also break the loop.** Asking logind to power off ends this
                        // process the ordinary way, through the SIGTERM systemd sends — one exit
                        // rather than a race between two. The same reasoning as the API route's.
                        DisplayAction::Quit => {
                            if let Some(power) = api.power() {
                                if let Err(error) = power.shut_down() {
                                    // The operating system's own sentence, on the band the rest of
                                    // this loop reports refusals on: the machine is still up, so
                                    // there is a screen to put it on.
                                    tracing::error!(%error, "the machine could not shut down");
                                    flash =
                                        Some(Flashed::from(DropStatus::Failed(error.to_string())));
                                }
                                continue;
                            }
                            break 'running;
                        }
                        // Needs the machine's paths and the loop's own `flash`, neither of which
                        // `handle` is given -- the same reason the two arms above are here.
                        DisplayAction::OpenPackagesFolder => {
                            show_packages_folder(&machine, &opened_tx);
                            continue;
                        }
                        // Needs the meter, which is this loop's own and is deliberately not shared:
                        // it is written once a frame from here and read nowhere else.
                        // Needs the API and the settings flag, neither of which `handle` is given.
                        DisplayAction::OpenRemote => {
                            show_remote(&api, config.remote_served, &opened_tx);
                            continue;
                        }
                        // Handed to the installer's worker rather than run here: a rescan holds the
                        // library's mutex for as long as installing every package in the folders
                        // takes, which on the display thread is the picture stopping. Same queue as
                        // a drop, so the two cannot run into each other's transaction.
                        DisplayAction::RescanPackages => {
                            // A refusal comes straight back, as a drop's does, so that it is on
                            // screen rather than waiting behind the queue it was refused for.
                            if let Some(refused) = installer.rescan() {
                                flash = Some(Flashed::from(refused));
                            }
                            continue;
                        }
                        // Needs the loop's own `flash`, which `handle` is not given -- the same
                        // reason `RescanPackages` above is here.
                        //
                        // **The flash band rather than `NumberEntry::show_message`, and that is
                        // not a style choice.** That line is failures only, painted `theme.alert`
                        // with no kind to choose by, so good news sent through it arrives in the
                        // color of bad news, which is why a queued song's title
                        // does not go through it.
                        //
                        // It also needs a band at all, where a queued song does not. The usual
                        // answer here is that a success has the screen itself to speak with, and
                        // for a *song* that is true: it starts. A *mode* has nothing to show.
                        // `DEMO · QUEUE to sing next` is drawn for the length of any demo song
                        // whether the mode is on or off, so the screen cannot say which -- and
                        // switching it off mid-demo changes nothing visible whatsoever.
                        DisplayAction::ToggleDemo => {
                            flash = Some(toggle_demo(machine.as_ref()));
                            continue;
                        }
                        // **The key sets the flag and nothing else.** Building and dropping the
                        // meter happens once per frame below, where it also catches a page having
                        // moved the same flag -- which is the whole reason this stopped being a
                        // `bool` the loop owned.
                        DisplayAction::TogglePerformance => {
                            machine.set_performance_overlay(!machine.performance_overlay_on());
                            continue;
                        }
                        other => other,
                    };
                    if handle(
                        &machine,
                        &mut entry,
                        &mut walls,
                        &mut show_connect,
                        &mut show_queue,
                        action,
                    ) {
                        break 'running;
                    }
                }
                // The keypad is laid out in backbuffer pixels, and every path into it has to arrive
                // in that space. Mouse coordinates are window coordinates, which are pixels only
                // where the density is 1 — so on a Retina screen an unscaled click lands at half
                // the height it looks like and misses the pad entirely...
                // `which` filtered so a tap is not also counted as a click; see `TOUCH_MOUSEID`.
                SdlEvent::MouseButtonDown { x, y, which, .. } if which != TOUCH_MOUSEID => {
                    let (x, y) = window_to_pixels(x, y, density);
                    // Somebody at the machine, whether or not the press lands on a touch target —
                    // so the position bar comes back even where `display.keypad` has left `press`
                    // nothing to hit.
                    position_until = Some(Instant::now() + POSITION_LINGER);
                    if let Some(action) =
                        press(keypad_cache.keypad(), x, y, &mut strip_until, config.keypad)
                        && handle(
                            &machine,
                            &mut entry,
                            &mut walls,
                            &mut show_connect,
                            &mut show_queue,
                            action,
                        )
                    {
                        break 'running;
                    }
                }
                // ...while touch coordinates are normalized 0..1 instead, which is the whole reason
                // these are two arms. Treating a finger's x as a pixel would put every touch in the
                // top-left corner, and on Android touch is the only input there is.
                SdlEvent::FingerDown { x, y, touch_id, .. } if touch_id != MOUSE_TOUCHID => {
                    keypad_cache.clear_focus();
                    // The mouse arm's reasoning, one input over.
                    position_until = Some(Instant::now() + POSITION_LINGER);
                    if let Some(action) = press(
                        keypad_cache.keypad(),
                        x * width as f32,
                        y * height as f32,
                        &mut strip_until,
                        config.keypad,
                    ) && handle(
                        &machine,
                        &mut entry,
                        &mut walls,
                        &mut show_connect,
                        &mut show_queue,
                        action,
                    ) {
                        break 'running;
                    }
                }
                // A package dragged onto the window. SDL sends one of these per file, bracketed by
                // `DropBegin` and `DropComplete`, and neither bracket is needed here: each file is
                // handled on its own and the worker takes them in order.
                //
                // Nothing is installed on this thread. `Catalog::install` holds the library's
                // mutex for a whole transaction, which is seconds for a large package — long enough
                // to stop the picture mid-song. The worker does the work and this loop reads what it
                // has to say once a frame.
                SdlEvent::DropFile { filename, .. } => {
                    let path = dropped_path(&filename);
                    tracing::info!(path = %path.display(), "a file was dropped on the window");
                    // A refusal comes straight back — a file that is not a package, or too many at
                    // once — so that it is on screen before the queue ahead of it has moved.
                    if let Some(refused) = installer.submit(path) {
                        flash = Some(Flashed::from(refused));
                    }
                }
                _ => {}
            }
        }

        // -- packages dropped on the window --------------------------------------------------------
        //
        // Drained rather than read once: an install reports twice (started, then finished) and a
        // burst of drops reports twice each, and only the newest is worth screen space. Never blocks
        // — `poll` is a `try_recv`.
        while let Some(status) = installer.poll() {
            flash = Some(Flashed::from(status));
        }

        // -- and the bank a setup program asked for -----------------------------------------------
        //
        // At most one sentence per frame and usually none: the machine's poll loop leaves the newest
        // here and this takes it. Only ever set on the first start after an install where somebody
        // ticked the box, so on every other run this is one lock and nothing.
        if let Some(notice) = machine.take_first_run_notice() {
            flash = Some(Flashed::from(notice));
        }

        // -- and F10 ------------------------------------------------------------------------------
        //
        // Only failures arrive here: a folder that opened is a file manager window in front of the
        // machine, which says it better than a line of text could. `try_recv` in a loop, for the
        // same reason the drain above is one -- and never `recv`, which would park the display
        // thread for the length of a song.
        while let Ok(reason) = opened_rx.try_recv() {
            flash = Some(Flashed {
                text: reason,
                kind: km_display::FlashKind::Failed,
                until: Some(Instant::now() + FLASH_FAILED),
            });
        }

        if flash.as_ref().is_some_and(Flashed::expired) {
            flash = None;
        }

        // -- wallpaper ---------------------------------------------------------------------------
        while let Some(result) = walls.loader.poll() {
            walls.pending = false;
            match result {
                Ok(image) => match texture_from_rgba(&creator, &image) {
                    Ok(texture) => {
                        // Names a zipped image as `archive.zip/photo.jpg`, so the API says where it
                        // came from rather than just what it is called inside its archive.
                        let name = image.source.name();
                        // Whatever was still fading out has had its turn; without this it would be
                        // dropped and its texture left behind.
                        retire(walls.outgoing.take());
                        walls.outgoing = walls.current.take();
                        walls.current = Some(Wall { texture, name });
                        walls.schedule.start_crossfade();
                        machine.set_wallpaper_state(
                            walls.current.as_ref().map(|wall| wall.name.clone()),
                            walls.playlist.len(),
                            walls.problem(),
                            wall_source,
                        );
                    }
                    Err(error) => {
                        tracing::warn!(%error, "could not upload a wallpaper");
                        walls.schedule.abandon_change();
                    }
                },
                Err(error) => {
                    tracing::warn!(%error, "could not decode a wallpaper");
                    walls.schedule.abandon_change();
                }
            }
        }

        let now = Instant::now();
        let delta = now.duration_since(last_frame);
        last_frame = now;

        // The keypad line's own deadline, and the fourth of this shape in this loop. It is a delta
        // rather than an `Instant` because `NumberEntry` is pure state and its tests say so exactly;
        // the wallpaper's schedule below takes the same one. Without it a message stayed until
        // somebody pressed a key that happened to clear it, which is how a refusal about a melody
        // channel was still on an idle screen long after the song it was about had ended.
        entry.tick(delta);

        // Before the tick below rather than inside it: the folder has to be right *before* anything
        // rescans, or the first refresh after an upload reads the old one and the picture appears a
        // cycle late.
        if machine.take_wallpaper_dir_stale() {
            let resolved = settings_wallpaper_dir(&machine);
            if resolved != wallpaper_dir {
                tracing::info!(
                    from = %wallpaper_dir.display(),
                    to = %resolved.display(),
                    "the wallpaper folder changed under the machine"
                );
                wallpaper_dir = resolved;
            }
        }

        // **Both are evaluated, and that is what `||` would not do.** Short-circuiting left the
        // request flag set on any frame the timer had already fired, so the next frame took it and
        // advanced the playlist a second time for one change — rare while only a person could ask,
        // and ordinary now that a song starting asks.
        let asked = machine.take_wallpaper_request();
        let due = walls.schedule.tick(delta);
        if due || asked {
            // Stops the interval timing for as long as the decode is in flight. `tick` does this
            // for itself when it fires; a request from anywhere else needs telling.
            walls.schedule.change_now();
            // **The folder *choice* is re-made here, not only its contents rescanned**, and that is
            // a fault this fixes rather than a refinement. `Paths::wallpaper_dir` picks between the
            // owner's folder, the checkout overlay and the bundled set **by contents** — and it used
            // to be asked once, at startup. So on every machine before its first picture, where
            // `Paths::create` makes the owner's folder empty on purpose, dropping an image in
            // scanned a folder that had already lost the argument: the contents were live and the
            // choice was frozen. It also has to be able to move *back*, because `Where the owner's
            // own wallpapers live` promises that emptying the folder returns the shipped set.
            //
            // Only the directory, deliberately: `interval` and `crossfade` are baked into
            // `Schedule::new` and `fit`/`dim` into `Walls::start`, so re-reading those here would be
            // a behavior change beyond the bug.
            let (dir, source) = machine.wallpaper_folder();
            wall_source = source;
            if dir != config.wallpaper.dir {
                tracing::info!(
                    from = %config.wallpaper.dir.display(),
                    to = %dir.display(),
                    rule = source.describe(),
                    "the wallpaper folder has changed"
                );
                config.wallpaper.dir = dir;
            }
            config.wallpaper.extra = machine.debug_wallpapers();
            // Rescanned each cycle so images can be added while the machine runs.
            walls
                .playlist
                .refresh(&config.wallpaper.dir, &config.wallpaper.extra);
            walls.playlist.advance();
            if !walls.request(width, height) {
                // Nothing was asked for — an empty folder, or a loader that has stopped. No result
                // is coming, so nothing will end the wait, and the rescan above lives inside this
                // same `if`: images dropped into an empty folder while the machine runs would never
                // be noticed again.
                walls.schedule.abandon_change();
            }
        }
        let presentation = walls.schedule.presentation();
        if !presentation.crossfading {
            retire(walls.outgoing.take());
        }

        // -- state -------------------------------------------------------------------------------
        // Copied once, at the top of the frame. Nothing below holds a lock.
        let mut snapshot = machine.snapshot();
        // Smoothed here and nowhere else: the engine, the API and the `lyric_line` events go on
        // reporting exactly what the audio has really reached. Same division as the lyric offset.
        let period_ms = machine.period_ms();
        let advancing = snapshot.transport == Transport::Playing;
        snapshot.position_ms = ms_clock.smooth(snapshot.position_ms, period_ms, advancing, now);
        let song = machine.current_lyric_song();
        let now_kind = snapshot.now_playing.as_ref().map(|now| now.kind);

        // **The window says what is playing**, which is the one thing about this machine an
        // operating system will draw for a program that is behind another window. Pushed from here
        // because the window belongs to this loop and nothing outside it holds a handle.
        if title_changed(&titled, snapshot.now_playing.as_ref()) {
            let wanted = match snapshot.now_playing.as_ref() {
                Some(now) => song_window_title(&now.title, now.artist.as_deref(), &base_title),
                None => base_title.clone(),
            };
            // Warned about rather than propagated, as `set_window_icon` and `apply_fullscreen`
            // treat their own failures: the only way this fails is a title carrying an interior
            // NUL, and a machine that stops singing over its own window furniture is worse than one
            // whose taskbar button is a version behind.
            if let Err(error) = canvas.window_mut().set_title(&wanted) {
                tracing::warn!(%error, "could not put the song on the window title");
            }
            titled = snapshot
                .now_playing
                .as_ref()
                .map(|now| (now.title.clone(), now.artist.clone()));
            // **A song beginning raises the bar, and this raise is what makes the window at the
            // start of a song true.** `now_playing` arrives here from the locked state on the frame
            // a song is loaded, while `position_ms` comes from an atomic the audio callback alone
            // writes — so until the device reports, the frame holds the new song's length beside
            // the position of the song before it, and a window read off that pair answers about
            // neither. Six seconds covers it: a queue advance costs one device period, and a device
            // reopening after an idle gap has four seconds of `OPEN_RETRIES` to get there.
            //
            // `None` where the deck emptied, so a deadline never outlives the song that raised it.
            //
            // Its blind spot is a second song with the same title and artist, whose bar waits for a
            // key press — cheaper than a second notion of *a song changed* in this loop.
            position_until = snapshot
                .now_playing
                .is_some()
                .then(|| Instant::now() + POSITION_LINGER);
        }

        let queue = machine.queue();
        // The PIN comes from settings rather than from the API's `ConnectInfo`, because it is
        // deliberately not on the wire: `/discover` says only *that* a machine is on a factory
        // password, never which one. The screen is the one place it belongs.
        let factory_pin = machine.factory_pin();
        let connect = crate::connect::to_display(&api.connect_info(), factory_pin.as_deref());

        if let Some(song) = &song
            && song.beat_ticks() != loaded_beat_ticks
        {
            // The lyric view's lead-in is measured in beats, so it has to be rebuilt per song. A
            // beat rather than a quarter note: a timecode file and an UltraStar song have none.
            loaded_beat_ticks = song.beat_ticks();
            view = LyricView::for_ticks_per_quarter(song.beat_ticks());
        }

        let info = snapshot.now_playing.as_ref().map(|now| SongInfo {
            number: match &now.origin {
                km_api::machine::Origin::Catalog { number, .. } => Some(*number),
                km_api::machine::Origin::File { .. } => None,
                // Shown, so somebody who likes what the machine picked for itself can write the
                // number down — which is the whole reason a demo names its songs at all.
                km_api::machine::Origin::Demo { number } => Some(*number),
            },
            title: now.title.clone(),
            artist: now.artist.clone(),
            // Turned into a word here rather than in `km-display`, which would otherwise need a
            // dependency on `km-kmpkg` purely to look up a name. A code this build does not know
            // — from a package written by a later one — resolves to `None` and is simply not drawn,
            // which is better than putting `xx` on a television.
            language: now
                .language
                .as_deref()
                .and_then(km_kmpkg::Language::parse)
                .map(|language| language.name().to_owned()),
        });
        let next_up = queue.first().map(km_queue::QueueEntry::label);
        // Resolved here rather than in `km-display`, the same seam `language` and the bank label
        // use. The sentence has to carry the instruction as well as the state: a demo song has no
        // queue entry to run out, so waiting for it to end is not the answer and skipping is.
        let demo = snapshot
            .now_playing
            .as_ref()
            .filter(|now| matches!(now.origin, km_api::machine::Origin::Demo { .. }))
            .map(|_| DEMO_LABEL);
        let lyrics = match (&song, snapshot.transport) {
            (Some(song), Transport::Playing | Transport::Paused) => {
                // The one place the display offset is applied. It shifts the tick this screen draws
                // from and nothing else: the engine's position, the audio and the `lyric_line`
                // events the API publishes all keep the true one. At the default of 0 this is the
                // published tick unchanged.
                // Smoothed for the same reason and by the same means as `position_ms` above: this
                // is the counter the wipe actually rides on, and it steps once per audio callback.
                // An UltraStar song's clock is the audio's, already smoothed, which its millisecond
                // tempo map turns into ticks; its tempo is never changed, so the offset is not
                // scaled by a ratio meant for the next MIDI song.
                let (ticks, tempo_ratio) = if now_kind.is_some_and(|kind| kind.is_midi()) {
                    let ticks = tick_clock.smooth(
                        machine.engine().position_ticks(),
                        period_ms,
                        advancing,
                        now,
                    );
                    (ticks, snapshot.settings.tempo_ratio)
                } else {
                    (song.tempo_map.ms_to_tick(snapshot.position_ms), 1.0)
                };
                let tick = km_display::lyrics::shift_ticks(
                    &song.tempo_map,
                    ticks,
                    snapshot.settings.lyric_offset_ms,
                    tempo_ratio,
                );
                view.frame(&song.lyrics, tick)
            }
            _ => Default::default(),
        };

        // The number pad stands on the idle screen the way a front panel does, but only where there
        // is no keyboard to type a number on — Android by default, and anywhere `display.number_pad`
        // asks for it. The transport strip is not conditioned on that and appears on every platform,
        // for a few seconds after a press, because it sits over the lyrics.
        let playing = snapshot.now_playing.is_some();
        // Whether the song brings its own words with it: a video or an MP3+G song, whose words are
        // in its picture. A MIDI or an UltraStar song has a timeline the machine draws instead.
        let has_own_picture = now_kind.is_some_and(|kind| !kind.draws_words());
        let melody_available = snapshot
            .now_playing
            .as_ref()
            .is_some_and(|now| now.melody_channel.is_some());
        // One instant for both deadlines, so a single frame cannot answer two questions from two
        // clocks.
        let deadline_now = Instant::now();
        let strip_visible = strip_visible(strip_pinned, strip_until, deadline_now);
        // Asked each frame rather than hoisted before the loop, because unlike the font paths and
        // `frame_stats` this one changes while the machine runs — that is the whole point of it.
        // `None` on every machine whose `debug.soundfonts` is empty, which costs one uncontended
        // lock and no allocation.
        let soundfont_label = machine.debug_soundfont_label();
        // **Both areas are named at once**, which is what retired the arbitration that used to
        // stand here: the slot held one sentence, so a package problem had to beat a sound one to
        // reach it and a machine with both said only half of what was wrong. A count and a list have
        // room for both.
        //
        // Not drawn with a flash, though. They share the top of the idle screen, and two sentences
        // in one place read as one sentence — the same reason a dialled title is never drawn beside
        // an error. The flash wins because it is the newer of the two and it is about something the
        // person standing there has just done; this line is standing and comes back when the flash
        // goes.
        //
        // It is also the honest ordering for the case that matters most: a package that will not
        // install produces a *flash* saying why, and leaving the standing line above it would put a
        // count and a reason on screen about the same file, in two different voices.
        let faults = if flash.is_none() {
            faults(&machine)
        } else {
            km_display::Faults::default()
        };
        // Only while idle, because that is the only screen it is drawn on: during a song this is a
        // library read per frame for something nobody can see.
        let catalog = (!playing).then(|| summary_cache.get(&machine)).flatten();
        let keypad = keypad_cache.get(
            &theme,
            width,
            height,
            playing,
            melody_available,
            keypad_visible(config.keypad, config.number_pad, playing, strip_visible),
        );

        // Read once: the bar's own length and the window that decides whether it is drawn are the
        // same fact, and two readings of it could disagree about the song within one frame.
        let duration_ms = snapshot
            .now_playing
            .as_ref()
            .map_or(0, |now| now.duration_ms);
        let adjustments = drawn_adjustments(now_kind, &snapshot.settings);

        let frame = Frame {
            // Read per frame rather than off `config`, so the /admin/ picker takes effect on the
            // next frame rather than on the next start. `Machine::locale` exists for this.
            locale: machine.locale(),
            screen: if playing {
                Screen::Playing
            } else {
                Screen::Idle
            },
            song: info.as_ref(),
            timeline: song.as_ref().map(|song| &song.lyrics),
            lyrics,
            picture: has_own_picture,
            show_position: position_visible(
                position_wanted(strip_pinned, position_until, deadline_now),
                // A song that has stopped keeps its bar for as long as it is stopped: the position
                // stops advancing with it, so the window cannot answer for it, and a room that
                // paused to fetch a drink is the room most likely to be asking. Gated on `playing`
                // so the idle screen is not reported as wanting a bar it has no row for.
                playing && !advancing,
                has_own_picture,
                snapshot.position_ms,
                duration_ms,
            ),
            position_ms: snapshot.position_ms,
            duration_ms,
            transpose: adjustments.0,
            tempo_ratio: adjustments.1,
            // `None` hides the indicator entirely, which is the point: a melody channel that was
            // never confidently detected must not be presented as a control.
            melody: snapshot
                .now_playing
                .as_ref()
                .and_then(|now| now.melody_channel.map(|_| snapshot.settings.melody_enabled)),
            lyrics_hidden: snapshot
                .now_playing
                .as_ref()
                .is_some_and(|now| now.lyrics_hidden),
            connect: Some(&connect),
            catalog,
            number_entry: &entry,
            next_up: next_up.as_deref(),
            demo,
            show_connect_overlay: show_connect || started.elapsed() < CONNECT_GREETING,
            queue: &queue,
            show_queue_overlay: show_queue,
            faults,
            flash: flash.as_ref().map(|flash| km_display::Flash {
                text: &flash.text,
                kind: flash.kind,
            }),
            keypad: (!keypad.is_empty()).then_some(keypad),
            soundfont_label: soundfont_label.as_deref(),
            developer_mode,
            // `unwrap_or_default` rather than `?`, so pressing the key gets a panel immediately:
            // the meter may have been built this very frame and have nothing to report yet, and
            // `frames: 0` is what the panel draws as `measuring...`.
            performance: show_perf.then(|| {
                frames
                    .as_ref()
                    .map_or_else(km_display::FrameStats::default, FrameMeter::snapshot)
            }),
            // **Gated on the key, like the meter beside it**, so the state lock this takes is a cost
            // only a machine somebody is diagnosing pays. `and_then` rather than `then`: the panel
            // goes up on the idle screen too, and there is no song to describe there.
            song_stats: show_perf
                .then(|| song_stats(&machine, now_kind, song.as_deref()))
                .flatten(),
            // Unconditional, because it is true whatever the machine is doing. The idle screen is
            // the only one that draws it.
            version: Some(VERSION),
        };

        // -- picture -----------------------------------------------------------------------------
        // Whichever kind of song puts its own picture where the wallpaper goes. Take the one that
        // belongs at the position the *audio* has reached, and upload it. Nothing here waits: when
        // no new picture is ready the last one stays on screen, which is the ordinary case because
        // the display draws faster than a video's frame rate or a disc's redraws.
        #[cfg(feature = "video")]
        {
            if let Some(song) = machine.current_video() {
                let frames = song.frames();
                if let Some(picture) = frames.take_frame_for(snapshot.position_ms) {
                    // A texture per song, remade only if the picture's size changes — which within
                    // one file it does not.
                    if video.as_ref().is_none_or(|held| {
                        held.width != picture.width || held.height != picture.height
                    }) {
                        retire_picture(video.take());
                        // IYUV rather than RGB: this is the decoder's own layout, so the three
                        // planes upload untouched and the GPU converts while it draws.
                        match creator.create_texture_streaming(
                            PixelFormat::IYUV,
                            picture.width,
                            picture.height,
                        ) {
                            Ok(texture) => {
                                video = Some(PictureTexture {
                                    texture,
                                    width: picture.width,
                                    height: picture.height,
                                });
                            }
                            Err(error) => {
                                tracing::error!(%error, "could not make a video texture");
                            }
                        }
                    }
                    if let Some(held) = video.as_mut() {
                        let (y, y_pitch) = picture.y();
                        let (u, u_pitch) = picture.u();
                        let (v, v_pitch) = picture.v();
                        if let Err(error) = held
                            .texture
                            .update_yuv(None, y, y_pitch, u, u_pitch, v, v_pitch)
                        {
                            tracing::warn!(%error, "could not upload a video frame");
                        }
                    }
                    // Straight back to the decoder's pool, so the next picture reuses these buffers.
                    frames.recycle(picture);
                }
            } else {
                // The song ended or was skipped; let go of the picture with it.
                retire_picture(video.take());
            }
        }

        // The MP3+G branch, and **no `#[cfg]` on it**: this is the whole point of rendering CD+G in
        // Rust rather than pre-rendering each pair to an MP4. It draws in a `--no-video` build and
        // on Android, where the block above is compiled out entirely.
        if let Some(song) = machine.current_cdg() {
            let frames = song.frames();
            if let Some(picture) = frames.take_frame_for(snapshot.position_ms) {
                if cdg.as_ref().is_none_or(|held| {
                    held.width != picture.width() || held.height != picture.height()
                }) {
                    retire_picture(cdg.take());
                    // `ARGB8888` rather than the video path's `IYUV`: a CD+G surface is a
                    // sixteen-color palette resolved on the CPU, and the whole picture is 221 KB,
                    // so there is nothing for a GPU color conversion to save.
                    match creator.create_texture_streaming(
                        PixelFormat::ARGB8888,
                        picture.width(),
                        picture.height(),
                    ) {
                        Ok(mut texture) => {
                            // Nearest, against SDL's bilinear default. A karaoke machine's picture
                            // is meant to look blocky; 288x192 smoothed up to a television looks
                            // like a mistake rather than like a disc.
                            texture.set_scale_mode(ScaleMode::Nearest);
                            cdg = Some(PictureTexture {
                                texture,
                                width: picture.width(),
                                height: picture.height(),
                            });
                        }
                        Err(error) => {
                            tracing::error!(%error, "could not make a CD+G texture");
                        }
                    }
                }
                if let Some(held) = cdg.as_mut() {
                    let (pixels, pitch) = picture.pixels();
                    if let Err(error) = held.texture.update(None, pixels, pitch) {
                        tracing::warn!(%error, "could not upload a CD+G picture");
                    }
                }
                frames.recycle(picture);
            }
        } else {
            retire_picture(cdg.take());
        }

        #[cfg(feature = "video")]
        let picture_layer = video
            .as_mut()
            // A video's pixels are square, so its pixel size *is* its shape.
            .map(|held| (&mut held.texture, (held.width, held.height)));
        #[cfg(not(feature = "video"))]
        let picture_layer: Option<(&mut Texture, (u32, u32))> = None;
        // A CD+G picture passes 4:3 and not its 288x192, because its pixels are not square.
        let picture_layer = picture_layer.or_else(|| {
            cdg.as_mut()
                .map(|held| (&mut held.texture, km_cdg::DISPLAY_ASPECT))
        });

        text_cache.begin_frame();
        draw(
            &mut canvas,
            &mut text_cache,
            &fonts,
            &theme,
            &frame,
            // A video or MP3+G song's picture *is* the background: it goes exactly where the
            // wallpaper goes, which is why every overlay — the keypad, the queue, the connect panel,
            // the progress bar — keeps working over it with no change at all. No scrim, because the
            // 45% dim exists to keep lyrics legible over an arbitrary photograph and there are no
            // lyrics being drawn here; darkening the words would only make them harder to sing.
            match picture_layer {
                Some((texture, shape)) => Background {
                    current: Some(texture),
                    outgoing: None,
                    fade: 1.0,
                    dim: 0.0,
                    shape: Some(shape),
                },
                None => Background {
                    current: walls.current.as_mut().map(|wall| &mut wall.texture),
                    outgoing: walls.outgoing.as_mut().map(|wall| &mut wall.texture),
                    fade: presentation.fade_in,
                    dim: walls.dim,
                    shape: None,
                },
            },
        );
        // **The clock stops here, before `present`, and that split is the whole point.** With vsync
        // granted `present` blocks until the next refresh, so a frame that took two milliseconds to
        // build and one that took twelve both come out of it at the same moment. Timed together they
        // would read as one number pinned to the frame interval, which is exactly the reading that
        // says "this machine cannot keep up" -- and it would say so on a machine with nothing wrong.
        let built = Instant::now();
        canvas.present();
        let present = built.elapsed();
        let build = built.duration_since(last_frame);

        // **The first Japanese title this machine has ever been asked to draw**, which the cache
        // noticed because every string on screen goes through it. Rebuilding is the only way to
        // answer it: SDL_ttf caches rasterized glyphs per face, so attaching a fallback to the faces
        // already open would return success and go on drawing the boxes it had drawn once.
        //
        // After `present`, so the frame that discovered it is already on screen and the pause lands
        // between frames rather than inside one. That frame shows boxes; the next does not.
        if !with_cjk && text_cache.saw_cjk() {
            with_cjk = true;
            match Fonts::discover(
                &ttf,
                config.font.as_deref(),
                config.bundled_font.as_deref(),
                config.font_cjk.as_deref(),
                with_cjk,
                &theme,
                height,
            ) {
                Ok(rebuilt) => {
                    let found = rebuilt.has_cjk();
                    fonts = rebuilt;
                    // The same obligation the resize rebuild has, for the same reason: the cache's
                    // keys carry the address of the font that drew each string.
                    text_cache.clear();
                    if found {
                        tracing::info!(
                            "a song wants CJK glyphs; the fonts now stand on a CJK face"
                        );
                    } else {
                        // `with_cjk` stays true, so this is said once rather than every frame of
                        // every song. Boxes are what the screen shows, and the log is the only place
                        // that can say why.
                        tracing::warn!(
                            "a song wants CJK glyphs and no CJK font was found; \
                             set display.font_cjk in settings.json"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not rebuild the fonts with a CJK face");
                }
            }
        }

        // The padding below is decided on both together: the question it asks is whether the whole
        // loop body came in under the floor, and on a display that paces us it never does.
        let spent = build + present;
        if let Some(meter) = frames.as_mut() {
            // Read only when the meter exists, so a build with `--frame-stats` off does not so much
            // as load the atomics. That is the same bargain the meter itself makes.
            meter.record(build, present, delta, decode_counts(&machine));
        }
        if spent < MIN_FRAME {
            std::thread::sleep(MIN_FRAME - spent);
        }
    }

    tracing::info!("the display is closing");
    // **Asked of the window rather than tracked**, exactly as `ToggleFullscreen` asks it: a
    // compositor that refused a change must not have us write down the state it declined to enter.
    //
    // Only where this run is entitled to speak for the machine — see
    // `DisplayConfig::remember_fullscreen` — and only when the answer differs from what is already
    // written, so an ordinary close does not rewrite settings.json for nothing. `FULLSCREEN_IS_FIXED`
    // is checked because on Android the window is fullscreen whatever settings say, and writing that
    // back would turn a fact about the platform into a preference that follows the data directory.
    if let Some(state) =
        fullscreen_to_remember(config.remember_fullscreen, || is_fullscreen(&canvas))
    {
        machine.remember_fullscreen(state);
    }
    // And where the window was left in the stack. No flag half here — nothing declares this run
    // temporary — so the only question is whether the platform stacks windows at all.
    if let Some(state) = always_on_top_to_remember(|| is_always_on_top(&canvas)) {
        machine.remember_always_on_top(state);
    }
    // And where the window was, when it closed in a window. Read once more here, for a drag whose
    // last move event never reached the loop.
    if let Some(rect) = plain_window_rect(&canvas) {
        windowed = rect;
    }
    if let Some(rect) = window_rect_to_remember(is_fullscreen(&canvas), windowed) {
        machine.remember_window_rect(rect);
    }
    shutdown.store(true, Ordering::Release);
    Ok(())
}

/// Whether any keypad is drawn this frame — which decides *which* one, since the screen does.
///
/// A named function rather than an expression in the frame loop so that a test can assert the real
/// composition instead of a copy of it. The two conditions are different in kind and that is the
/// whole of it: `number_pad` is a standing platform decision about the idle screen, `strip_visible`
/// is a six-second timer over a song. `keypad` is the master switch over both.
fn keypad_visible(keypad: bool, number_pad: bool, playing: bool, strip_visible: bool) -> bool {
    keypad && if playing { strip_visible } else { number_pad }
}

/// Whether the transport strip is up this frame.
///
/// A named function beside [`keypad_visible`] and for its reason: a test can assert the real
/// composition rather than a copy of it.
///
/// **The pin wins over the timer and does not touch it.** `Ctrl+F12` holds the strip up without
/// clearing `until`, so unpinning hands back whatever the timer had left rather than a strip that
/// vanishes the instant it stops being pinned — and a press while pinned still pushes the deadline
/// out, so the strip that was up stays up for the usual six seconds afterwards.
///
/// `now` is passed rather than read here so the test does not have to wait six seconds.
fn strip_visible(pinned: bool, until: Option<Instant>, now: Instant) -> bool {
    pinned || before(until, now)
}

/// Whether a deadline is still ahead. `None` is a deadline nobody set.
///
/// Shared by [`strip_visible`] and [`position_visible`], so the arithmetic is written once.
fn before(until: Option<Instant>, now: Instant) -> bool {
    until.is_some_and(|until| now < until)
}

/// How much of each end of a song draws the bar unasked, by which screen the song brings.
fn position_window_ms(picture: bool) -> u32 {
    if picture {
        POSITION_WINDOW_PICTURE_MS
    } else {
        POSITION_WINDOW_MS
    }
}

/// Whether the song is inside the window at one end or the other.
///
/// **A length is required, and that is the whole of what the first term is for.** A bar with no
/// length cannot move, so unasked it would take a row to say nothing; a song reporting zero is a
/// video container that declares no duration, or a MIDI file whose last event is at tick zero.
/// Asking for the bar still gets the empty track, because a key press deserves a visible answer.
///
/// `saturating_sub` because [`Frame::position_ms`] and [`Frame::duration_ms`] can describe two
/// different songs for the first frames of one — see [`position_visible`].
fn within_position_window(picture: bool, position_ms: u32, duration_ms: u32) -> bool {
    let window = position_window_ms(picture);
    duration_ms > 0 && (position_ms < window || duration_ms.saturating_sub(position_ms) <= window)
}

/// Whether somebody is asking for the position bar this instant.
///
/// The same shape as [`strip_visible`] over a *different* deadline, and the difference is the point:
/// only a D-pad move and a touch raise the strip, so a desktop keyboard raises it never and a bar
/// that followed it would be absent from the machine somebody is sitting at.
fn position_wanted(pinned: bool, until: Option<Instant>, now: Instant) -> bool {
    pinned || before(until, now)
}

/// Whether the position bar is drawn this frame.
///
/// A named function beside [`strip_visible`] and [`keypad_visible`], for their reason: a test can
/// assert the real composition rather than a copy of it.
///
/// **The window is the ambient rule and the other two terms are people.** A song draws its bar
/// unasked at either end; between them `wanted` answers somebody standing at the machine, and `held`
/// answers a song that has stopped, where how far through it is is the only thing left to say.
///
/// **The window is decided here rather than in `km-display` because only this loop can tell a fresh
/// position from the previous song's.** `Snapshot` takes `now_playing` from the locked state and
/// `position_ms` from an engine atomic that the audio callback alone writes, so through the frames
/// between a song being loaded and the device reporting, a frame carries one song's position beside
/// another song's length. `position_until` is what covers those frames, which is why the raise on a
/// song change is load-bearing rather than a courtesy.
///
/// **It does not read [`keypad_visible`].** That master switch decides whether this machine has
/// touch targets at all, and a machine with none still has a song whose length the room wants to
/// know. Reading it would take the bar off every appliance with the pad switched off.
fn position_visible(
    wanted: bool,
    held: bool,
    picture: bool,
    position_ms: u32,
    duration_ms: u32,
) -> bool {
    wanted || held || within_position_window(picture, position_ms, duration_ms)
}

/// Turns a press at a point into an action, and keeps the transport strip alive.
///
/// The reveal-then-press behavior is deliberate and is how every phone video player works: while the
/// strip is hidden, the first touch brings it up and does nothing else. Acting on that touch would
/// mean skipping a song because somebody brushed the screen.
fn press(
    keypad: &km_display::Keypad,
    x: f32,
    y: f32,
    strip_until: &mut Option<Instant>,
    enabled: bool,
) -> Option<DisplayAction> {
    if !enabled {
        return None;
    }
    // Any press means somebody is interacting, so hold the strip open whether they hit a key or not.
    let action = keypad.hit(x, y);
    *strip_until = Some(Instant::now() + STRIP_LINGER);
    action
}

/// Shows the packages folder in the operating system's file manager. `F10`.
///
/// **The folder is the singular [`Paths::packages_dir`](crate::settings::Paths::packages_dir)**, not every entry of `packages_dirs`. That
/// one is the folder [`Paths::create`](crate::settings::Paths::create) makes and `--show-paths` calls the answer to *where do I put
/// my songs?*; the others are Android's public directory — which this build cannot reach anyway, see
/// [`FILE_MANAGER`] — and whatever the owner named in `settings.package_dirs`, and one key opening
/// three windows is a surprise rather than a convenience. Somebody who set those knows where they
/// are.
///
/// **Off the display thread, always.** `km_osopen::open` waits for the child to exit, and both
/// `xdg-open` consulting a desktop and `cmd /c start` spawning a shell take long enough to drop
/// frames in the middle of a song. `km-package-builder` wraps the identical call in `spawn_blocking`
/// and `km-remote` in a thread of its own for exactly this reason; there is no runtime here, so it is
/// a thread. It is not joined: the press is finished as far as this loop is concerned, and what the
/// thread has to say comes back through the channel.
///
/// **Only a failure is sent.** A folder that opened put a window in front of the machine, which says
/// so better than a band across the lyrics would; a failure is the case with nothing else to show
/// for it, and on a machine with no opener at all it is the difference between a key that explains
/// itself and a key that looks dead. Both outcomes are logged, on the standing rule that a press
/// which produces nothing must not also be silent.
/// The folder `F10` shows, made if it is not there.
///
/// **Making it is not belt-and-braces.** `Paths::create` makes it at every start, so the only way it
/// is missing is that somebody deleted it while the machine was running — and it is still the folder
/// the machine scans, so making it again is the answer rather than an error to report.
///
/// It also removes the one Windows failure that would never have reached the screen. **Measured**:
/// `cmd /c start "" <path that does not exist>` puts up a *modal message box* and does not return
/// until somebody dismisses it, so the failure never comes back as an exit code at all — nothing
/// would have flashed, and the thread would have sat there until the dialog was clicked. Cheap
/// either way: `create_dir_all` on a directory that is already there is one syscall.
///
/// Split out from [`show_packages_folder`] so it can be asserted without launching anything, which
/// is the same reason `km_osopen::command_for` is split from its own caller.
fn folder_to_show(paths: &crate::settings::Paths) -> std::io::Result<PathBuf> {
    let folder = paths.packages_dir();
    std::fs::create_dir_all(&folder)?;
    Ok(folder)
}

fn show_packages_folder(machine: &Machine, failures: &std::sync::mpsc::Sender<String>) {
    if !FILE_MANAGER {
        tracing::debug!("this build has nothing to show a folder in");
        return;
    }
    let folder = match folder_to_show(machine.paths()) {
        Ok(folder) => folder,
        Err(error) => {
            tracing::warn!(%error, "could not make the packages folder");
            let _ = failures.send(format!("could not make the packages folder: {error}"));
            return;
        }
    };
    let failures = failures.clone();
    // Named, because a thread that dies with the receiver already gone should be legible in a log
    // rather than a surprise.
    let spawned = std::thread::Builder::new()
        .name("km-open-folder".into())
        .spawn(move || {
            match km_osopen::open(&folder) {
                Ok(()) => tracing::debug!(folder = %folder.display(), "opened the packages folder"),
                Err(error) => {
                    tracing::warn!(folder = %folder.display(), %error, "could not open the packages folder");
                    // A send that fails means the display has gone, which is the machine closing.
                    let _ = failures.send(format!("could not open the packages folder: {error}"));
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the thread that opens the packages folder");
    }
}

/// The URL `F11` should open, or the reason there is not one.
///
/// Split out from [`show_remote`] so it can be asserted without launching a browser, which is the
/// reason [`folder_to_show`] is split from its own caller and the reason `km_osopen::command_for` is
/// split from its.
///
/// Two refusals, and they are different failures worth different sentences. **The remote turned
/// off** is a working machine doing what it was told: `api.serve_remote` is a settings key, and with
/// it false `/` serves the API's diagnostic landing page — so opening it would put an index of
/// endpoints in front of somebody expecting a list of songs, which is worse than saying no. **No
/// server at all** is the port conflict the connect panel already exists for, and it carries the
/// same words that panel does rather than inventing a second phrasing for one fault.
fn remote_url(api: &ApiState, remote_served: bool) -> Result<String, String> {
    if !remote_served {
        return Err(
            "the singer's remote is turned off in settings (api.serve_remote), so there is \
             nothing to open"
                .to_owned(),
        );
    }
    let info = api.connect_info();
    crate::connect::own_url(&info).ok_or_else(|| match &info.problem {
        Some(km_api::ConnectProblem::ServerFailed { message }) => {
            format!("the web server did not start, so there is nothing to open: {message}")
        }
        _ => "the web server did not start, so there is nothing to open".to_owned(),
    })
}

/// Opens the singer's remote in whatever this computer uses for web pages. `F11`.
///
/// The same shape as [`show_packages_folder`] in every respect that matters, and for the same
/// reasons: **off this thread**, because `km_osopen::open_url` blocks until the child process exits
/// and a browser cold-starting is seconds of frozen picture; **failures down a channel**, because
/// the thread has no way to draw; and **silence on success**, because a browser window standing in
/// front of the machine says it better than a band over the lyrics could.
///
/// It shares [`show_packages_folder`]'s channel rather than taking one of its own. Both are keys
/// that reach outside the machine, both can only ever report a failure, and only one of them can be
/// mid-flight in any frame that matters — a second channel would be a second `try_recv` in the loop
/// for no property gained.
fn show_remote(api: &ApiState, remote_served: bool, failures: &std::sync::mpsc::Sender<String>) {
    if !WEB_BROWSER {
        tracing::debug!("this build has nothing to open a web page in");
        return;
    }
    let url = match remote_url(api, remote_served) {
        Ok(url) => url,
        Err(reason) => {
            tracing::warn!(%reason, "the remote could not be opened");
            let _ = failures.send(reason);
            return;
        }
    };
    let failures = failures.clone();
    let spawned = std::thread::Builder::new()
        .name("km-open-remote".into())
        .spawn(move || match km_osopen::open_url(&url) {
            Ok(()) => tracing::debug!(%url, "opened the remote"),
            Err(error) => {
                tracing::warn!(%url, %error, "could not open the remote");
                // A send that fails means the display has gone, which is the machine closing.
                let _ = failures.send(format!("could not open a browser: {error}"));
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the thread that opens the remote");
    }
}

/// Flips demo mode, asks for a song when it went on, and says which happened.
///
/// **`persist: false`, always.** The machine keeps two switches — `demo_enabled` for this run and
/// `settings.demo.enabled` for what a restart finds — and this moves only the first. A key pressed
/// at the machine is the evening's switch; `settings.json`, the owner's `/admin/` page and
/// `PUT /api/v1/admin/demo` remain the ways to make it hold.
///
/// **Turning it on asks for a song rather than moving the deadline**, which is the whole reason the
/// key is worth having. `Controller::start_demo_song` sets the machine's `demo_once` one-shot and
/// the poll thread loads the song fifty milliseconds later — a press is a person saying *now*, not
/// a deadline arriving early. Left to the clock instead, a press in a room that has just been
/// singing would do nothing at all for the whole delay and read as a key that does not work.
///
/// **A refusal is reported as `Done`, not `Failed`, and that is deliberate.** The four refusals in
/// `why_no_demo` — a song loaded, a queue to play first, no sound, off the screen — all mean *not
/// yet*, and the mode is on regardless and will take the deck the moment it clears. Only
/// `set_demo` failing is a failure, and it is the one that gets the red band.
///
/// **Switching off leaves the song playing**, which is the same bargain the API route makes: this
/// is a mode, not a transport command, and `N` is one key away for somebody who meant stop.
///
/// Generic over the trait rather than taking `&Machine`, purely so the three outcomes can be tested
/// against `km_api::testing::TestMachine` — starting a real one needs an audio device, a catalog and
/// a window, none of which a unit test has.
fn toggle_demo<C: km_api::machine::Controller>(machine: &C) -> Flashed {
    let on = !machine.demo().enabled;
    if let Err(error) = machine.set_demo(on, false) {
        tracing::warn!(%error, "demo mode could not be switched");
        return Flashed {
            text: error.to_string(),
            kind: km_display::FlashKind::Failed,
            until: Some(Instant::now() + FLASH_FAILED),
        };
    }
    let text = if !on {
        "demo mode off".to_owned()
    } else {
        match machine.start_demo_song() {
            Ok(_) => "demo mode on".to_owned(),
            // The machine's own sentence, rather than a second wording for the same four facts.
            Err(reason) => format!("demo mode on · {reason}"),
        }
    };
    tracing::info!(on, %text, "demo mode was switched from the keyboard");
    Flashed {
        text,
        kind: km_display::FlashKind::Done,
        until: Some(Instant::now() + FLASH_DONE),
    }
}

/// Acts on a key. Returns whether the machine should stop.
fn handle(
    machine: &Machine,
    entry: &mut NumberEntry,
    walls: &mut Walls,
    show_connect: &mut bool,
    show_queue: &mut bool,
    action: DisplayAction,
) -> bool {
    match action {
        DisplayAction::Digit(digit) => {
            if let NumberAction::Submit(number) = entry.push_digit(digit) {
                queue_number(machine, entry, number);
            }
            look_up_preview(machine, entry);
        }
        DisplayAction::Submit => {
            if let NumberAction::Submit(number) = entry.submit(machine.locale()) {
                queue_number(machine, entry, number);
            }
        }
        DisplayAction::Backspace => {
            entry.backspace();
            look_up_preview(machine, entry);
        }
        DisplayAction::Clear => {
            entry.clear();
        }
        DisplayAction::TogglePause => {
            let playing = machine.snapshot().transport == Transport::Playing;
            let command = if playing {
                TransportCommand::Pause
            } else {
                TransportCommand::Play
            };
            report(entry, machine.transport(command));
        }
        DisplayAction::Skip => report(entry, machine.transport(TransportCommand::Skip)),
        DisplayAction::Restart => report(entry, machine.transport(TransportCommand::Restart)),
        DisplayAction::SeekBy(seconds) => seek_by(machine, entry, seconds),
        DisplayAction::TransposeUp => nudge_transpose(machine, entry, 1),
        DisplayAction::TransposeDown => nudge_transpose(machine, entry, -1),
        DisplayAction::TransposeReset => set_transpose(machine, entry, 0),
        DisplayAction::ToggleMelody => {
            let enabled = machine.snapshot().settings.melody_enabled;
            let result = machine.update_settings(&SettingsPatch {
                melody_enabled: Some(!enabled),
                ..Default::default()
            });
            report_settings(entry, result);
        }
        DisplayAction::NextWallpaper => {
            walls.playlist.advance();
            walls.pending = false;
        }
        DisplayAction::Focus(_) => {
            // Intercepted in the event loop, where the keypad lives. Unreachable here, and an arm
            // rather than a catch-all so adding an action stays a compile error.
        }
        DisplayAction::ToggleConnect => *show_connect = !*show_connect,
        DisplayAction::ToggleQueue => *show_queue = !*show_queue,
        DisplayAction::SelectSoundFont(slot) => {
            // Straight to the machine, like every other arm here: `handle` already holds one, and
            // the switch needs nothing the event loop has. A machine with an empty
            // `debug.soundfonts` returns `Ok` without doing anything, so an unconfigured build
            // shows no message for a key somebody pressed by accident.
            if let Err(error) = machine.switch_debug_soundfont(slot) {
                entry.show_message(error.to_string());
            }
        }
        DisplayAction::Back => {
            // Up one level, and out from the top. Four levels, in this order, because each is a
            // thing on screen that a person would expect back to undo:
            //
            //   1. the queue overlay     -> close it
            //   2. a number being typed  -> clear it
            //   3. a song loaded         -> stop it, which unloads and returns to the idle screen
            //   4. nothing               -> leave
            //
            // The typed-number level matters more than it looks: without it, mistyping a number and
            // reaching for back would close the machine instead of correcting the digits.
            //
            // No confirmation at any level — Android's TV guidelines forbid gating back behind one —
            // and every press does something visible, so back is never a dead key and never loops.
            // At most four presses leave from anywhere.
            //
            // Logged because one press must produce exactly one of these. A remote that delivers a
            // button twice — which is how the touch keypad's doubled digits turned out — would show
            // up here as two lines and traverse two levels, which from a sofa looks indistinguishable
            // from "back just exits".
            if *show_queue {
                tracing::debug!("back: closing the queue");
                *show_queue = false;
            } else if entry.is_active() {
                tracing::debug!("back: clearing the number being typed");
                entry.clear();
            } else if machine.snapshot().transport == Transport::Idle {
                tracing::debug!("back: nothing left to leave, stopping the machine");
                return true;
            } else {
                tracing::debug!("back: stopping the song");
                // `Stop` unloads rather than merely pausing, so the next press sees `Idle` and quits.
                report(entry, machine.transport(TransportCommand::Stop));
            }
        }
        DisplayAction::Escape
        | DisplayAction::ToggleFullscreen
        | DisplayAction::ToggleAlwaysOnTop => {
            // Intercepted in the event loop, where the window is. Unreachable here, and arms
            // rather than a catch-all so adding an action stays a compile error.
        }
        DisplayAction::ToggleStripPin => {
            // Likewise: the pin is a local of the loop that draws the strip, and deliberately
            // reaches no further than that. See the arm in the event loop.
        }
        DisplayAction::OpenPackagesFolder
        | DisplayAction::OpenRemote
        | DisplayAction::TogglePerformance => {
            // Intercepted in the event loop too, where the paths, the flash and the meter are. Same
            // reasoning as the arm above, and the same reason they are written out.
        }
        DisplayAction::RescanPackages => {
            // Likewise: it needs the installer's worker and the loop's own `flash`, neither of
            // which reaches here.
        }
        DisplayAction::ToggleDemo => {
            // Likewise: it answers on the flash band, because a mode has nothing on screen of its
            // own to speak with. See the arm in the event loop.
        }
        DisplayAction::Quit => return true,
    }
    false
}

/// What is wrong with this machine, counted by area.
///
/// The catalog on this machine is what somebody is looking at when an album is missing, and until a
/// line above the title existed the only account of why was a line in a log — which, on a box under
/// a television with no console and no browser, nobody reads.
///
/// **What it says is how many and where, and never why.** It quoted the first package's reason
/// until 2026-09-08, back when the screen was the only surface a refusal reached. It is not any
/// more: `/admin/`'s Problems tab lists every one of them and offers to delete the file behind each,
/// `GET /packages` carries the whole of it, and so does the online remote. What was left here was a
/// diagnostic reproduced in the one place with the least room for it — two lines above the title,
/// wrapping on whitespace — so the half that got cut was always the informative half. See
/// `A clash warns rather than only logging`.
///
/// The two areas, and both are faults nobody would otherwise see standing in front of the machine:
///
/// * **Packages.** A file that would not open, or a bank another package already holds. A missing
///   album with nothing on screen accounting for it.
/// * **Sound.** A stale `audio.soundfont` playing the bundled bank instead, a bank that resolved and
///   would not parse so the instruments are a test tone, or no audio device at all. A machine that
///   sounds wrong is not obviously the machine's fault, and the person who could act on it is the
///   one standing in front of it.
///
/// Cheap on a healthy machine, which is the ordinary case: one lock and a length for the packages,
/// one enum read for the sound.
pub(crate) fn faults(machine: &Machine) -> km_display::Faults {
    km_display::Faults {
        packages: machine.package_problems().len(),
        // `SoundFontStatus::complaint` rather than this module's own reading of `problem` and
        // `fallback`: the owner's Problems tab says the same thing through the same function, and
        // one machine must not word one fault two ways. It is asked whether it has something to say
        // rather than what — three states collapse to one count here, and the tab is where the three
        // are told apart.
        sound: usize::from(machine.soundfont().complaint().is_some()),
    }
}

/// What this machine has open that a machine in a living room should not.
///
/// **The screen's half of a rule the pages already state**, and what makes it owed is that both
/// switches are things somebody turns on to get something done and then has no reason to think about
/// again. Debugging mode lets anybody on the network play a file off this machine's disk; the
/// development console goes further, and serves the whole API a second time with no password on any
/// of it. Neither left a mark anywhere a person in the room could see.
///
/// **The state, not the words** — see `km_display::Frame::developer_mode`. Two fixed states need no
/// value from here, so `km-display` says them in the language it is already drawing the screen in.
///
/// **The running values and not the stored ones**, which is the whole of what this function decides.
/// The pages draw a switch, and a switch has to show what the next start will do; the screen has to
/// show what *this* machine is doing, or a marker would appear the moment somebody pressed a switch
/// and before the surface it warns about existed.
fn developer_mode(api: &km_api::ApiConfig) -> Option<km_display::DeveloperMode> {
    if km_api::routes::dev_console_served(api) {
        return Some(km_display::DeveloperMode::Console);
    }
    api.debug_enabled
        .then_some(km_display::DeveloperMode::Debugging)
}

/// Names the song being dialled, so it is on screen before anybody presses OK.
///
/// Called once per key press and never per frame: [`NumberEntry::pending_lookup`] answers `None`
/// once the current input has been asked about, so a held digit or a still screen costs nothing.
/// **No debounce** — a timer would delay exactly the feedback this exists to give, and the cost
/// being deferred is one indexed row read.
///
/// A number nobody has is recorded as an answer of `None` rather than as an error: see the module
/// documentation of `km_display::numbers` for why a miss draws nothing while typing.
fn look_up_preview(machine: &Machine, entry: &mut NumberEntry) {
    let Some(key) = entry.pending_lookup() else {
        return;
    };
    // Only a well-formed number is worth asking about. Anything else simply stays unanswered, and
    // the next key press asks again.
    let Ok(number) = key.parse::<SongCode>() else {
        return;
    };
    match machine.song_preview(number) {
        SongPreviewLookup::Found { title, artist } => {
            entry.set_lookup(&key, Some(km_display::SongPreview { title, artist }));
        }
        // Recording the miss is what stops the same number being asked about again.
        SongPreviewLookup::Missing => entry.set_lookup(&key, None),
        // Nothing was learned, so nothing is recorded and the next key press asks again.
        SongPreviewLookup::Busy => {}
    }
}

/// Looks a number up and queues it, reporting only a failure on the keypad.
///
/// **Success says nothing, which is the same judgment [`show_packages_folder`] makes**: a song that
/// was queued is on the screen already — it starts playing, or it appears as `next: …`, which is
/// permanent where a message lingers eight seconds. A failure is the case with nothing else to show
/// for it.
///
/// Showing the song's title on success is a real fault rather than clutter: every entry message is
/// drawn in `theme.alert`, so the confirmation that a song was queued arrives in the color reserved
/// for things going wrong — a red strip naming the song, as reported from a sofa. **Having no
/// success case is what makes that color correct** — every caller of `show_message` is a failure, so
/// the one channel carries one meaning.
fn queue_number(machine: &Machine, entry: &mut NumberEntry, number: SongCode) {
    use km_api::machine::Catalog as _;

    match machine.song(number) {
        Ok(Some(song)) => {
            let request = km_queue::QueueRequest {
                number,
                title: song.title,
                artist: song.artist,
                singer: None,
            };
            match machine.queue_add(request) {
                Ok(_) => {
                    entry.clear();
                }
                Err(error) => entry.show_message(error.to_string()),
            }
        }
        // The message a real machine gives when somebody punches in a number nobody has.
        Ok(None) => entry.show_message(format!("no song {number}")),
        Err(error) => entry.show_message(error.to_string()),
    }
}

/// Moves the position within the song that is playing.
///
/// Clamped here for the same reason a transpose is: the display knows the delta, the machine knows
/// the song, and holding the key down should stop at the end rather than flash a refusal at every
/// press. The last second is kept back deliberately — seeking exactly to the end would trip the
/// watchdog that advances the queue, so `+10s` near the end would look like NEXT.
fn seek_by(machine: &Machine, entry: &mut NumberEntry, seconds: i32) {
    let snapshot = machine.snapshot();
    let Some(now) = snapshot.now_playing.as_ref() else {
        entry.show_message("nothing is playing".to_owned());
        return;
    };
    let duration = now.duration_ms;
    let delta = seconds.saturating_mul(1_000);
    let target = (snapshot.position_ms as i64 + i64::from(delta)).max(0) as u64;
    let ceiling = u64::from(duration.saturating_sub(1_000));
    let ms = target.min(ceiling) as u32;
    report(entry, machine.transport(TransportCommand::Seek { ms }));
}

fn nudge_transpose(machine: &Machine, entry: &mut NumberEntry, by: i8) {
    let current = machine.snapshot().settings.transpose;
    set_transpose(machine, entry, current.saturating_add(by));
}

fn set_transpose(machine: &Machine, entry: &mut NumberEntry, semitones: i8) {
    // Clamped here rather than letting the machine reject it, so holding the key down stops at the
    // limit instead of flashing an error at every press.
    let semitones = semitones.clamp(-km_queue::MAX_TRANSPOSE, km_queue::MAX_TRANSPOSE);
    let result = machine.update_settings(&SettingsPatch {
        transpose: Some(semitones),
        ..Default::default()
    });
    report_settings(entry, result);
}

fn report(entry: &mut NumberEntry, result: Result<(), km_api::machine::ControlError>) {
    if let Err(error) = result {
        entry.show_message(error.to_string());
    }
}

fn report_settings(
    entry: &mut NumberEntry,
    result: Result<km_api::machine::Settings, km_api::machine::ControlError>,
) {
    if let Err(error) = result {
        entry.show_message(error.to_string());
    }
}

/// Uploads decoded pixels as a texture.
///
/// The loader hands over tightly packed RGBA; SDL's `ARGB8888` is BGRA in memory on a little-endian
/// machine, so the channels are swapped on the way in. Done here, on already-downscaled pixels,
/// rather than in the decoder, so the loader stays independent of SDL's pixel layout.
fn texture_from_rgba(
    creator: &TextureCreator<WindowContext>,
    image: &km_display::wallpaper::LoadedImage,
) -> Result<Texture, String> {
    let format = PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_ARGB8888)
        .map_err(|error| error.to_string())?;
    let mut surface = sdl3::surface::Surface::new(image.width, image.height, format)
        .map_err(|error| error.to_string())?;
    surface.with_lock_mut(|bytes| {
        for (index, pixel) in image.rgba.as_chunks::<4>().0.iter().enumerate() {
            let offset = index * 4;
            if offset + 3 < bytes.len() {
                bytes[offset] = pixel[2];
                bytes[offset + 1] = pixel[1];
                bytes[offset + 2] = pixel[0];
                bytes[offset + 3] = pixel[3];
            }
        }
    });
    creator
        .create_texture_from_surface(&surface)
        .map_err(|error| error.to_string())
}

/// The catalog lookup the keypad performs, exposed so `main` can report a cold catalog.
pub fn catalog_is_empty(machine: &Machine) -> bool {
    use km_api::machine::Catalog as _;

    machine
        .search(&SearchQuery {
            limit: 1,
            ..Default::default()
        })
        .map(|songs| songs.is_empty())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key or tempo badge is drawn over a MIDI song, and over nothing else that is playing.
    #[test]
    fn only_a_midi_song_draws_its_key_and_tempo() {
        let settings = km_api::machine::Settings {
            transpose: 2,
            tempo_ratio: 1.1,
            ..km_api::machine::Settings::default()
        };
        assert_eq!(
            drawn_adjustments(Some(km_catalog::SongKind::Midi), &settings),
            (2, 1.1)
        );
        assert_eq!(drawn_adjustments(None, &settings), (2, 1.1));
        for kind in [
            km_catalog::SongKind::UltraStar,
            km_catalog::SongKind::Lrc,
            km_catalog::SongKind::Cdg,
            km_catalog::SongKind::Video,
        ] {
            assert_eq!(
                drawn_adjustments(Some(kind), &settings),
                (0, 1.0),
                "{kind:?}"
            );
        }
    }

    /// A desktop drop is a path and is passed through untouched.
    #[test]
    fn a_dropped_path_is_left_alone() {
        assert_eq!(
            dropped_path("/tunes/karaoke/party.kmpkg"),
            PathBuf::from("/tunes/karaoke/party.kmpkg"),
        );
        assert_eq!(
            dropped_path(r"D:\tunes\karaoke\party.kmpkg"),
            PathBuf::from(r"D:\tunes\karaoke\party.kmpkg"),
        );
    }

    /// iOS hands over a URL, and it becomes the path it names.
    ///
    /// The percent-decoding is the half worth asserting: a package named by a person is quite
    /// likely to have a space in it, and `%20` reaching `Catalog::install` is refused as a file that
    /// is not there.
    #[test]
    fn a_dropped_url_becomes_the_path_it_names() {
        assert_eq!(
            dropped_path("file:///var/mobile/Documents/party.kmpkg"),
            PathBuf::from("/var/mobile/Documents/party.kmpkg"),
        );
        assert_eq!(
            dropped_path("file:///var/mobile/Documents/party%20night.kmpkg"),
            PathBuf::from("/var/mobile/Documents/party night.kmpkg"),
        );
        assert_eq!(
            dropped_path("file://localhost/var/mobile/party.kmpkg"),
            PathBuf::from("/var/mobile/party.kmpkg"),
        );
    }

    /// An escape is a byte of UTF-8, not a character.
    ///
    /// Two escapes that mean one letter have to be decoded together, which is why the decoder works
    /// on bytes: taken one at a time they would come out as two wrong letters and the file would not
    /// be found.
    #[test]
    fn a_dropped_url_keeps_a_name_that_is_not_ascii() {
        assert_eq!(
            dropped_path("file:///var/mobile/can%C3%A7%C3%B5es.kmpkg"),
            PathBuf::from("/var/mobile/canções.kmpkg"),
        );
    }

    /// A stray `%` is kept rather than swallowed.
    #[test]
    fn a_dropped_url_keeps_a_percent_that_escapes_nothing() {
        assert_eq!(
            dropped_path("file:///var/mobile/100%.kmpkg"),
            PathBuf::from("/var/mobile/100%.kmpkg"),
        );
        assert_eq!(
            dropped_path("file:///var/mobile/50%zz.kmpkg"),
            PathBuf::from("/var/mobile/50%zz.kmpkg"),
        );
    }

    /// What `D` does, driven through the real [`km_api::machine::Controller`] trait.
    ///
    /// Against `TestMachine` rather than a `Machine`, which is the whole reason [`toggle_demo`] is
    /// generic: a real one needs an audio device, a catalog and a window. The double records what
    /// it was asked, so these assert the two calls actually made and not merely the sentence drawn.
    /// What the operating system is told the machine is doing.
    ///
    /// Composed by a function of its own so it can be asserted without a window: the loop that
    /// pushes it needs a display, and what goes wrong here is the shape of a string.
    mod window_title {
        use super::*;

        const BASE: &str = "KaraokeMachine 1.8.0";

        #[test]
        fn a_song_leads_and_the_version_trails() {
            assert_eq!(
                song_window_title("Exagerado", Some("Cazuza"), BASE),
                "Exagerado — Cazuza — KaraokeMachine 1.8.0"
            );
        }

        #[test]
        fn a_song_nobody_named_a_performer_for_draws_no_empty_separator() {
            assert_eq!(
                song_window_title("Exagerado", None, BASE),
                "Exagerado — KaraokeMachine 1.8.0"
            );
            // The package builder's *Title from file name* writes exactly this, so it is not a
            // shape that only a malformed file can produce.
            assert_eq!(
                song_window_title("Exagerado", Some(""), BASE),
                "Exagerado — KaraokeMachine 1.8.0"
            );
            assert_eq!(
                song_window_title("Exagerado", Some("   "), BASE),
                "Exagerado — KaraokeMachine 1.8.0"
            );
        }

        /// The guard that keeps `SDL_SetWindowTitle` off sixty frames a second.
        #[test]
        fn only_a_change_of_song_asks_for_a_new_title() {
            let playing = |title: &str, artist: Option<&str>| NowPlaying {
                origin: km_api::machine::Origin::File {
                    path: "/tunes/karaoke/a.kar".into(),
                },
                title: title.to_owned(),
                artist: artist.map(ToOwned::to_owned),
                language: None,
                singer: None,
                kind: km_catalog::SongKind::Midi,
                duration_ms: 1_000,
                melody_channel: None,
                has_lyrics: true,
                lyrics_hidden: false,
            };

            let idle: Option<(String, Option<String>)> = None;
            assert!(!title_changed(&idle, None), "an idle machine stays idle");
            assert!(title_changed(&idle, Some(&playing("Exagerado", None))));

            let up = Some(("Exagerado".to_owned(), Some("Cazuza".to_owned())));
            assert!(
                !title_changed(&up, Some(&playing("Exagerado", Some("Cazuza")))),
                "the same song frame after frame asks for nothing"
            );
            assert!(title_changed(&up, Some(&playing("Wave", Some("Cazuza")))));
            // The artist alone moves while a machine plays two takes of one title, and a window
            // still owes the room the difference.
            assert!(title_changed(&up, Some(&playing("Exagerado", Some("Ney")))));
            assert!(title_changed(&up, None), "a song ending clears it");
        }
    }

    mod demo_key {
        use km_api::testing::{Recorded, TestMachine};

        use super::*;

        #[test]
        fn turning_it_on_asks_for_a_song_rather_than_waiting_out_the_clock() {
            let machine = TestMachine::new();

            let flash = toggle_demo(&machine);

            assert!(machine.demo().enabled, "the mode is on");
            assert_eq!(flash.text, "demo mode on");
            assert_eq!(flash.kind, km_display::FlashKind::Done);
            // The point of the key, and the half a plain `set_demo` would miss: the clock counts
            // idleness, so without this the room stays silent for another whole delay.
            assert!(
                machine.recorded().contains(&Recorded::DemoStarted),
                "turning the mode on must ask for a song, not just move the switch"
            );
            // And never written down: `settings.json` is the owner's switch, not this one.
            assert!(machine.recorded().contains(&Recorded::SetDemo {
                enabled: true,
                persist: false,
            }));
            assert!(!machine.demo().stored, "a key press must not persist");
        }

        #[test]
        fn turning_it_off_says_so_and_asks_for_nothing() {
            let machine = TestMachine::new();
            let _ = toggle_demo(&machine);
            machine.clear_recorded();

            let flash = toggle_demo(&machine);

            assert!(!machine.demo().enabled);
            assert_eq!(flash.text, "demo mode off");
            assert_eq!(flash.kind, km_display::FlashKind::Done);
            assert!(
                !machine.recorded().contains(&Recorded::DemoStarted),
                "switching the mode off must not start a song"
            );
        }

        /// A refusal means *not yet*, and the band has to say that rather than report a failure.
        #[test]
        fn a_refused_song_still_leaves_the_mode_on_and_is_not_a_failure() {
            let machine = TestMachine::new();
            machine
                .queue_add(km_queue::QueueRequest {
                    number: "1001".parse().expect("a song number"),
                    title: "Something".to_owned(),
                    artist: None,
                    singer: None,
                })
                .expect("the queue takes it");

            let flash = toggle_demo(&machine);

            assert!(
                machine.demo().enabled,
                "the mode goes on regardless -- it takes the deck once the queue clears"
            );
            assert_eq!(
                flash.kind,
                km_display::FlashKind::Done,
                "not yet is not a failure, and must not get the red band"
            );
            // The machine's own sentence, rather than a second wording for the same fact.
            assert_eq!(
                flash.text,
                "demo mode on · there are songs in the queue to play first"
            );
        }
    }

    /// A closing window writes back what it was left as — unless this run was told, or is Android.
    ///
    /// The window is asked only where the answer is kept, which is the other half of what this
    /// pins: a `Canvas` that has already been dropped, or a platform with nothing to ask, must not
    /// be reached for to produce a value that is then discarded.
    #[test]
    fn only_a_run_that_chose_nothing_remembers_how_it_was_left() {
        let mut asked = false;
        let live = |asked: &mut bool| {
            *asked = true;
            true
        };

        // A run given `--fullscreen` or `--windowed` writes nothing, and does not even look.
        assert_eq!(fullscreen_to_remember(false, || live(&mut asked)), None);
        assert!(!asked, "the window was asked for an answer nobody wanted");

        // A run given neither writes back whatever the window ended as — except on Android, where
        // fullscreen is the platform rather than a choice.
        let remembered = fullscreen_to_remember(true, || live(&mut asked));
        if FULLSCREEN_IS_FIXED {
            assert_eq!(
                remembered, None,
                "Android has no window state worth keeping"
            );
            assert!(!asked);
        } else {
            assert_eq!(remembered, Some(true));
            assert!(asked);
            assert_eq!(fullscreen_to_remember(true, || false), Some(false));
        }
    }

    /// A window closed in a window writes its rect; one closed fullscreen keeps the old one.
    #[test]
    fn only_a_window_closed_in_a_window_remembers_where_it_was() {
        let rect = WindowRect {
            position: Some((200, 150)),
            width: 1000,
            height: 600,
        };
        assert_eq!(window_rect_to_remember(true, rect), None);
        let windowed = window_rect_to_remember(false, rect);
        if FULLSCREEN_IS_FIXED {
            assert_eq!(windowed, None, "Android has no window to place");
        } else {
            assert_eq!(windowed, Some(rect));
        }
    }

    /// A saved position is used only while its title bar lands on a display that is still there.
    #[test]
    fn a_window_saved_on_a_missing_monitor_is_centered() {
        let rect = |x, y| WindowRect {
            position: Some((x, y)),
            width: 1000,
            height: 600,
        };
        // Two screens, the left one at a negative x, which is how a desk with the primary on the
        // right reports itself.
        let primary = Rect::new(0, 0, 1920, 1080);
        let left = Rect::new(-1920, 0, 1920, 1080);

        assert_eq!(
            reachable_position(rect(200, 100), &[primary]),
            Some((200, 100))
        );
        assert_eq!(
            reachable_position(rect(-1500, 100), &[primary, left]),
            Some((-1500, 100))
        );
        // The left monitor has gone.
        assert_eq!(reachable_position(rect(-1500, 100), &[primary]), None);
        // A title bar with only a sliver on screen cannot be grabbed.
        assert_eq!(reachable_position(rect(1900, 100), &[primary]), None);
        // Above the top edge, where the bar is off screen and the body is not.
        assert_eq!(reachable_position(rect(200, -200), &[primary]), None);
        // Nothing to test against, and nothing saved.
        assert_eq!(reachable_position(rect(200, 100), &[]), None);
        let centered = WindowRect {
            position: None,
            ..rect(0, 0)
        };
        assert_eq!(reachable_position(centered, &[primary]), None);
    }

    /// `F10` makes the folder it is about to show, if somebody deleted it since the machine started.
    ///
    /// The measured Windows behavior is what makes this worth a test rather than a line: `start` on
    /// a path that does not exist blocks on a modal dialog instead of failing, so the branch this
    /// removes is one that could not have reported itself. Asserted here rather than by pressing the
    /// key, because [`folder_to_show`] is exactly the part that launches nothing.
    #[test]
    fn f12_makes_the_packages_folder_if_it_is_missing() {
        let scratch = std::env::temp_dir().join(format!(
            "km-f12-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        let paths = crate::settings::Paths::data_rooted_at(scratch.clone());

        let folder = folder_to_show(&paths).expect("the folder is made");
        assert_eq!(folder, paths.packages_dir());
        assert!(
            folder.is_dir(),
            "F10 must not be pointed at a folder that is not there"
        );

        // Twice, because the ordinary case is a folder that already exists and `create_dir_all` has
        // to be quiet about that rather than an error somebody sees on every other press.
        assert_eq!(folder_to_show(&paths).expect("still fine"), folder);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The idle pad is a platform decision and the transport strip is not.
    ///
    /// Driven through the real [`keypad_visible`] and into a real [`KeypadCache`], rather than
    /// re-spelling the condition here: a test carrying its own copy of the expression would keep
    /// passing after somebody simplified the loop back to `!playing || strip_visible`, which still
    /// reads sensibly and silently returns the number pad to every desktop.
    #[test]
    fn the_number_pad_is_conditional_and_the_transport_strip_is_not() {
        let mut cache = KeypadCache::new();
        // `true` here is `melody_available`, which only decides whether the strip has a MELODY key.
        let drawn = |cache: &mut KeypadCache, keypad, number_pad, playing, strip| {
            let visible = keypad_visible(keypad, number_pad, playing, strip);
            !cache
                .get(&Theme::default(), 1920, 1080, playing, true, visible)
                .is_empty()
        };

        // Idle. The desktop declines the pad; Android asks for it.
        assert!(!drawn(&mut cache, true, false, false, false));
        assert!(drawn(&mut cache, true, true, false, false));

        // Playing. The strip appears while its timer is alive whatever the platform decided about
        // the pad, and goes when the timer lapses. This pair is what pins "the strip is unchanged".
        assert!(drawn(&mut cache, true, false, true, true));
        assert!(!drawn(&mut cache, true, false, true, false));

        // And the master switch still overrides both.
        assert!(!drawn(&mut cache, false, true, false, false));
        assert!(!drawn(&mut cache, false, false, true, true));
    }

    /// The pin holds the strip up, and hands the timer back untouched.
    ///
    /// The second half is the one worth a test. Unpinning restores whatever the timer had left
    /// rather than dropping the strip on the spot, which is what makes `Ctrl+F12` usable twice in
    /// a row while somebody works on the layout.
    #[test]
    fn pinning_the_strip_beats_its_timer_without_consuming_it() {
        let now = Instant::now();
        let alive = Some(now + STRIP_LINGER);
        let lapsed = Some(now - Duration::from_secs(1));

        // The timer alone, which is the behavior that must not change.
        assert!(strip_visible(false, alive, now));
        assert!(!strip_visible(false, lapsed, now));
        assert!(!strip_visible(false, None, now));

        // The pin alone, with no press behind it: the strip comes up over a song nobody has
        // touched, which is the whole point of the key.
        assert!(strip_visible(true, None, now));
        assert!(strip_visible(true, lapsed, now));

        // And unpinning gives back the deadline rather than the strip.
        assert!(strip_visible(false, alive, now));
    }

    /// A song three minutes long, for the tests that want a middle to be hidden in.
    const LONG_SONG_MS: u32 = 180_000;

    /// The bar visits both ends of a song and the words have the middle.
    ///
    /// Both screens, because the rule is now one rule and only the width of the window differs.
    #[test]
    fn the_position_bar_visits_both_ends_of_a_song() {
        for picture in [false, true] {
            let window = position_window_ms(picture);
            let unasked =
                |position| position_visible(false, false, picture, position, LONG_SONG_MS);

            assert!(unasked(0), "the bar is up as the song begins");
            assert!(unasked(window - 1), "and to the end of the first window");
            assert!(
                !unasked(window),
                "then the words have the screen for the middle of the song"
            );
            assert!(!unasked(LONG_SONG_MS / 2));
            assert!(
                !unasked(LONG_SONG_MS - window - 1),
                "up to the frame before the last window"
            );
            assert!(unasked(LONG_SONG_MS - window), "which it is drawn through");
            assert!(unasked(LONG_SONG_MS), "and at the end");
        }
    }

    /// The two windows differ by screen, and the machine's own is the wider.
    ///
    /// One assertion, and it is the one that would go quietly if somebody folded the two constants
    /// into one: the row a picture song's bar takes is a row of that song's words, and the row it
    /// takes on the machine's own screen holds nothing else.
    #[test]
    fn the_wider_window_belongs_to_the_machines_own_screen() {
        assert!(position_window_ms(false) > position_window_ms(true));
    }

    /// A song shorter than two windows keeps its bar the whole way through.
    ///
    /// The two ends overlap, and a song that is all beginning and all end is drawn as both rather
    /// than as neither.
    #[test]
    fn a_short_song_keeps_its_bar_throughout() {
        for picture in [false, true] {
            let window = position_window_ms(picture);
            let duration = window + window / 2;
            for position in [0, window / 2, window, duration - 1, duration] {
                assert!(
                    position_visible(false, false, picture, position, duration),
                    "a song of {duration}ms has no middle to hide in at {position}ms"
                );
            }
        }
    }

    /// A song with no length draws no bar unasked, and draws the empty track when asked.
    ///
    /// A bar with no length cannot move, so unasked it would take a row to say nothing. A key press
    /// is answered anyway: somebody who pressed something has to see the machine heard it.
    #[test]
    fn a_song_with_no_length_draws_no_bar_unasked() {
        for picture in [false, true] {
            assert!(
                !position_visible(false, false, picture, 0, 0),
                "a length of zero is the one thing the window cannot answer about"
            );
            assert!(position_visible(true, false, picture, 0, 0));
            assert!(position_visible(false, true, picture, 0, 0));
        }
    }

    /// A stopped song keeps its bar for as long as it is stopped.
    ///
    /// The position stops advancing with the song, so the window cannot answer for it, and this is
    /// the one state where how far through the song is is all there is left to say. It outlasts the
    /// key that stopped the song, which is the half worth a test: the deadline is lapsed here.
    #[test]
    fn a_stopped_song_keeps_its_bar_past_the_key_that_stopped_it() {
        let middle = LONG_SONG_MS / 2;
        for picture in [false, true] {
            assert!(
                !position_visible(false, false, picture, middle, LONG_SONG_MS),
                "the middle of a playing song has no bar"
            );
            assert!(
                position_visible(false, true, picture, middle, LONG_SONG_MS),
                "and the middle of a stopped one does"
            );
        }
    }

    /// A song change shows the bar while the position still reports the song before it.
    ///
    /// **The regression test for the raise on a song change.** `now_playing` reaches the frame from
    /// the locked state as soon as a song is loaded, while `position_ms` waits on the audio
    /// callback — so a frame in between holds the new song's length beside the old song's position,
    /// which is past both windows and can land inside the *end* one when the song before was the
    /// longer. Nothing the window can compute from that pair is about either song, and the deadline
    /// is what carries the start of a song until the device reports.
    #[test]
    fn a_song_change_shows_the_bar_before_the_audio_reports() {
        let now = Instant::now();
        let wanted = position_wanted(false, Some(now + POSITION_LINGER), now);

        // The song before was the shorter, so its position lands in this song's middle.
        let hidden = 100_000;
        // The song before was the longer, so `duration - position` saturates to zero and the
        // window reports the *end* of a song that has not started.
        let ending = LONG_SONG_MS + 20_000;

        for picture in [false, true] {
            assert!(
                !within_position_window(picture, hidden, LONG_SONG_MS),
                "read alone this pair hides the bar as the song begins"
            );
            assert!(
                within_position_window(picture, ending, LONG_SONG_MS),
                "and this one calls the beginning of the song its end"
            );

            // Neither answer is about either song, and neither is reached: the deadline carries
            // both frames until the device reports a position belonging to this song.
            assert!(position_visible(
                wanted,
                false,
                picture,
                hidden,
                LONG_SONG_MS
            ));
            assert!(position_visible(
                wanted,
                false,
                picture,
                ending,
                LONG_SONG_MS
            ));
        }
    }

    /// The pin holds the bar, and a lapsed deadline does not.
    ///
    /// The bar's deadline is its own and not the strip's, so this drives the real predicate: only a
    /// D-pad move and a touch raise the strip, and a bar that followed it would be absent from the
    /// machine somebody is sitting at with a keyboard.
    #[test]
    fn the_position_bar_follows_its_own_deadline_and_the_pin() {
        let now = Instant::now();
        let alive = Some(now + POSITION_LINGER);
        let lapsed = Some(now - Duration::from_secs(1));

        assert!(position_wanted(false, alive, now));
        assert!(!position_wanted(false, lapsed, now));
        assert!(!position_wanted(false, None, now));

        // The pin with no press behind it, so somebody working on the layout sees the bar and the
        // strip together for as long as they need to.
        assert!(position_wanted(true, None, now));
        assert!(position_wanted(true, lapsed, now));
    }

    /// A machine with no transport strip keeps a picture song's bar.
    ///
    /// **This is the case the composition exists to protect.** `display.keypad` off means there are
    /// no touch targets and so no strip at all, and a bar that had waited for one would wait for
    /// ever. Driven through both real functions rather than re-spelling either: a test carrying its
    /// own copy would keep passing after somebody simplified the loop to read `keypad_visible`,
    /// which reads sensibly and takes the bar off every appliance with the pad switched off.
    #[test]
    fn a_machine_with_no_transport_strip_keeps_a_picture_songs_bar() {
        let now = Instant::now();
        let alive = Some(now + POSITION_LINGER);

        assert!(
            !keypad_visible(false, true, true, true),
            "with the master switch off there is no strip to wait for"
        );
        assert!(
            position_visible(
                position_wanted(false, alive, now),
                false,
                true,
                LONG_SONG_MS / 2,
                LONG_SONG_MS
            ),
            "and the bar comes up anyway, because it follows its own deadline"
        );
        assert!(
            position_visible(false, false, true, 0, LONG_SONG_MS),
            "and a song beginning needs no strip either"
        );
    }

    /// The appliance's measured period: 8192 frames at 48 kHz.
    const P: u32 = 170;
    /// A plausible per-callback step in whatever unit is being smoothed.
    const STEP: u32 = 170;

    /// Feeds `reported` at `t`, having already established a step of `STEP`.
    fn primed(now: Instant) -> (StepSmoother, Instant) {
        let mut c = StepSmoother::new();
        c.smooth(1_000, P, true, now);
        let next = now + Duration::from_millis(u64::from(P));
        c.smooth(1_000 + STEP, P, true, next);
        (c, next)
    }

    #[test]
    fn the_first_report_is_shown_unchanged_because_no_step_is_known_yet() {
        let mut c = StepSmoother::new();
        assert_eq!(c.smooth(1_000, P, true, Instant::now()), 1_000);
    }

    #[test]
    fn once_a_step_is_known_a_report_is_shown_half_a_step_early() {
        let (mut c, t) = primed(Instant::now());
        // Re-reading the same report at the same instant: the sweep has not started.
        assert_eq!(c.smooth(1_000 + STEP, P, true, t), 1_000 + STEP - STEP / 2);
    }

    #[test]
    fn it_sweeps_forward_between_reports() {
        let (mut c, t) = primed(Instant::now());
        let mid = t + Duration::from_millis(u64::from(P) / 2);
        assert_eq!(c.smooth(1_000 + STEP, P, true, mid), 1_000 + STEP);
    }

    /// The property the design rests on: no seam where the staircase used to step.
    #[test]
    fn the_sweep_joins_up_exactly_when_the_next_report_arrives() {
        let (mut c, t) = primed(Instant::now());
        let due = t + Duration::from_millis(u64::from(P));
        let last = c.smooth(1_000 + STEP, P, true, due);
        let first = c.smooth(1_000 + 2 * STEP, P, true, due);
        assert_eq!(
            last, first,
            "a seam here is the judder this exists to remove"
        );
    }

    #[test]
    fn the_average_matches_the_staircase_so_the_offset_judged_by_eye_still_holds() {
        let (mut c, t) = primed(Instant::now());
        let mut total: u64 = 0;
        for step in 0..P {
            total += u64::from(c.smooth(
                1_000 + STEP,
                P,
                true,
                t + Duration::from_millis(u64::from(step)),
            ));
        }
        let mean = total / u64::from(P);
        let staircase = u64::from(1_000 + STEP);
        assert!(
            (mean as i64 - staircase as i64).abs() <= 1,
            "mean drifted to {mean}, staircase was {staircase}"
        );
    }

    #[test]
    fn it_never_sweeps_past_one_step_when_the_engine_goes_quiet() {
        let (mut c, t) = primed(Instant::now());
        let far = t + Duration::from_secs(30);
        assert_eq!(
            c.smooth(1_000 + STEP, P, true, far),
            1_000 + STEP + STEP / 2
        );
    }

    #[test]
    fn a_seek_backwards_lands_exactly_rather_than_being_swept_towards() {
        let (mut c, t) = primed(Instant::now());
        assert_eq!(c.smooth(500, P, true, t + Duration::from_millis(10)), 500);
    }

    #[test]
    fn a_forward_jump_far_larger_than_the_established_step_is_read_as_a_seek() {
        let (mut c, t) = primed(Instant::now());
        let jumped = 1_000 + STEP + STEP * StepSmoother::SEEK_FACTOR + 1;
        assert_eq!(
            c.smooth(jumped, P, true, t + Duration::from_millis(10)),
            jumped
        );
    }

    #[test]
    fn a_step_that_merely_varies_is_still_a_step() {
        let (mut c, t) = primed(Instant::now());
        // Twice the usual -- a tempo change or a long block, not a seek.
        let next = 1_000 + STEP + STEP * 2;
        assert_eq!(
            c.smooth(next, P, true, t + Duration::from_millis(10)),
            next - STEP
        );
    }

    #[test]
    fn a_paused_position_is_never_invented() {
        let (mut c, t) = primed(Instant::now());
        assert_eq!(
            c.smooth(1_000 + STEP, P, false, t + Duration::from_millis(200)),
            1_000 + STEP
        );
    }

    #[test]
    fn a_pause_is_not_counted_as_song_time_when_play_resumes() {
        let (mut c, t) = primed(Instant::now());
        c.smooth(1_000 + STEP, P, false, t + Duration::from_secs(10));
        // Ten seconds paused must not become ten seconds of song: the step is forgotten with the
        // anchor, so the first frame back shows exactly what the engine says.
        assert_eq!(
            c.smooth(1_000 + STEP, P, true, t + Duration::from_secs(10)),
            1_000 + STEP
        );
    }

    #[test]
    fn nothing_is_smoothed_before_a_stream_has_ever_run() {
        let mut c = StepSmoother::new();
        let t = Instant::now();
        assert_eq!(c.smooth(1_000, 0, true, t), 1_000);
        assert_eq!(
            c.smooth(1_000, 0, true, t + Duration::from_millis(500)),
            1_000
        );
    }
}
