//! A window of its own, instead of a tab in whatever browser is default — and an icon in the OS icon
//! bar whether or not there is a window.
//!
//! Behind the `desktop` feature, which is off in cargo and on when staging for Windows and macOS.
//! What this module adds is a frame around the page the server already serves: the webview points at
//! `http://127.0.0.1:<port>/`, so every handler, template and route is unchanged and none of them can
//! tell the difference. That is the whole design — this is a *viewer*, not a second front end.
//!
//! It is `tools/cmd/km-package-builder/src/desktop.rs` with more deleted than added, and the deletions
//! are the interesting part.
//!
//! **The event loop is no longer conditional on the window, and everything else here follows from
//! that.** An icon in the bar needs a platform event loop exactly as a window does, and the runs
//! that most need an icon are the ones with no window: `--browser`, `--lan`, and the fallback taken
//! when a webview will not build. So `run` is now entered by every windowed-executable run, and
//! `window` is a flag inside it. Two consequences worth stating:
//!
//! - **The loop has to end when the *server* ends.** A watcher task reports the server's outcome as
//!   [`Wake::ServerStopped`] and the loop exits on it. Without that, Ctrl-C leaves an event loop
//!   spinning in front of a dead server.
//! - **The webview fallback is not a dead end.** It proceeds with no window, which is the same run
//!   `--browser` asks for and needs no second arrangement; opening a browser and then waiting on
//!   the server on this thread would be a second one.
//!
//! **The event loop owns the main thread and never returns.** `tao`'s `run` diverges, and on exit it
//! calls `process::exit` — so destructors do not run and any shutdown work has to happen in
//! `Event::LoopDestroyed` rather than after the call. That is why `run` in `lib.rs` builds the tokio
//! runtime by hand instead of using `#[tokio::main]`: the runtime has to be started, handed the
//! server, and then kept alive by something other than a `block_on` sitting on the main thread.
//!
//! **There is no `Event::Opened` arm, and its absence is worth stating rather than leaving to be
//! noticed.** In the package builder that arm is the reason `tao` was chosen at all: on macOS a
//! double-clicked `.kmbuild` arrives as a `kAEOpenDocuments` Apple Event and never in `argv`, and a
//! second double-click routes to the already-running process the same way. **None of that applies
//! here.** This program opens no document, registers no file type, has no `--register` and takes no
//! positional argument, so there is nothing for such an event to carry and a second double-click on
//! macOS simply raises the window AppKit already has — which is correct. `tao` is kept for
//! uniformity and for the single lockfile entry, *not* because this crate needs `tao` specifically;
//! if the workspace ever wants a lighter window library, this is the crate that could take it and
//! the package builder is the one that could not.
//!
//! **There is no Quit button on the page, and unlike the package builder there is nowhere to put
//! one.**
//! `km-remote-pages`'s `layout.html` is a phone: a body, a banner, a `<main>`, and a three-item bottom tab
//! bar with `viewport-fit=cover` and a safe-area inset. It has no header at all. This is also the
//! one page in this repository that will literally ship on a phone, so a fourth tab reading "Quit"
//! would be wrong on the device the layout is designed for, and hiding it behind a `Capabilities`
//! flag would mean a field existing for one binary on two platforms — which inverts what
//! `Capabilities` is for. Closing the window is a complete quit: it asks for the same shutdown
//! Ctrl-C asks for. See the `The remote's window` decision in docs/decisions/, which records the one
//! case this leaves uncovered and what to do if it is ever reported.
//!
//! **The tray's Quit is not a second way out, and closing the window is still the quit.** Both call
//! `Stop::ask`, which is the same shutdown Ctrl-C runs; there is deliberately no minimize-to-tray,
//! so this program is never left running behind an icon after somebody closed its window. What the
//! tray adds is the case that had nothing: a run with no window at all. See the `A running server
//! has an icon in the bar` decision in docs/decisions/.
//!
//! **Both Windows icons are set**, by `km_webshell::with_icons`, from this executable's own
//! resources rather than by decoding a PNG. **Both** matters: the taskbar and Alt-Tab fall back to
//! the file on disk only while both of the window's icon slots are empty, so filling just the
//! title bar's ends that fallback one step early and leaves the taskbar stretching a 16-pixel
//! drawing. The whole mechanism is written down on `with_icons` and not repeated here. It is cheap
//! either way: `LoadImageW` by ordinal means no decoder, no new dependency, and no second copy of
//! the picture to keep in step.

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use anyhow::Result;
use km_remote_core::Stop;

/// Why the event loop was woken from outside itself.
///
/// `tao`'s user event, and the reason the loop is built with `with_user_event` at all. Both arms
/// arrive from another thread: the tray's handlers run on the platform's own event thread, and
/// [`Wake::ServerStopped`] comes off the tokio runtime.
#[derive(Debug, Clone, Copy)]
enum Wake {
    /// Somebody picked something from the icon in the bar.
    Tray(km_tray::Command),
    /// The server finished on its own — a Ctrl-C in a terminal, or a failure. Nothing is left for
    /// the window to look at, so the loop ends.
    ServerStopped,
}

/// The mark this program wears in the bar, on macOS.
///
/// The green one, which is this program's own and not the machine's — see the `Application icon`
/// decision in docs/decisions/. Windows never reads it: there the picture comes back out of the
/// executable's own resources, which is the same `.ico` `build.rs` compiled in.
///
/// 256 rather than the 32 the favicon uses, because the menu bar wants 44 physical pixels on a
/// Retina display and downscaling beats upscaling. See `km_tray`'s `decode`.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../../../../icon/km-remote-256.png");

/// What the window is called.
///
/// Static, and deliberately not the machine's address: the machine can go away and come back while
/// the window is open, and a title that tracked it would be a second place the connection state is
/// drawn. The page's own banner is the first, and is where a person actually looks.
///
/// **The whole name, not the `KM Remote` an icon says.** A title bar, the macOS app menu and the
/// tray tooltip — the three things this constant feeds — are each read one at a time and have the
/// room; the abbreviation is for a Dock and a launcher, where four siblings sit side by side. See
/// `What the product is called` in docs/decisions/, and `ports/remote/android`'s `app_name` beside
/// `app_name_short`, which is the same split one platform over.
const TITLE: &str = "KaraokeMachine Remote";

/// The size the window asks for, in logical pixels, before the screen gets a say.
///
/// **Portrait, where `km-package-builder`'s is landscape, and that is the one visible place the
/// analogy between the two windows breaks.** That tool's page is a desktop tool — wide tables, a row
/// ending in five buttons — so it wants 1500x900 and clamps down from there. `km-remote-pages`'s page is a
/// *phone*: a fluid single column with a three-tab bar pinned to the bottom, and no desktop
/// max-width anywhere in its stylesheet. At 1500 wide the three tabs stretch across a monitor and
/// every row becomes a title with half a meter of whitespace after it.
///
/// 520 is a comfortable phone column with room for the longest artist names, and stays clear of the
/// `max-width: 27rem` breakpoint at 432 logical pixels, which exists for real phones and should not
/// fire on a desktop.
const WANTED: (f64, f64) = (520.0, 900.0);

/// How long to wait for the server to finish on the way out.
///
/// Bounded, and short. Nothing here *must* happen — the mirror is rebuildable by definition and a
/// committed favorites transaction is durable in the WAL whether or not the connection is dropped
/// tidily — so this is only so that the graceful shutdown gets to close the connections it is
/// holding rather than having them cut. Two seconds is far longer than that takes and far shorter
/// than a person would notice.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Runs the desktop shell. Never returns.
///
/// `window` is [`crate::will_have_a_window`]'s answer; with `false` this is a `--browser` or `--lan`
/// run, which now comes here too so that it gets an icon in the bar. `runtime` is moved into the
/// event loop's closure and kept there: the server was spawned on it, and dropping it would stop
/// answering the very requests the webview is about to make.
pub fn run(
    window: bool,
    url: String,
    stop: Stop,
    serving: tokio::task::JoinHandle<Result<()>>,
    runtime: tokio::runtime::Runtime,
) -> ! {
    let event_loop: EventLoop<Wake> = EventLoopBuilder::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // **The server's own ending has to reach the loop**, and this is the only thing that carries it.
    // The loop owns the main thread, so a Ctrl-C that stops the server does not end the process:
    // without this the loop would go on spinning in front of a server that had already stopped. The outcome goes down a channel so `LoopDestroyed` can still
    // wait briefly for a shutdown it asked for.
    let (finished, outcome) = tokio::sync::oneshot::channel();
    runtime.spawn({
        let proxy = proxy.clone();
        async move {
            let _ = finished.send(serving.await);
            let _ = proxy.send_event(Wake::ServerStopped);
        }
    });

    let window = window.then(|| open_window(&event_loop, &url)).flatten();

    let mut outcome = Some(outcome);
    let mut tray = None;

    // **The menu bar, and it goes in here rather than at `StartCause::Init` where the icon does.**
    // On macOS ⌘Q is a key equivalent on the application menu and not a key the window is sent, so a
    // shell with no menu bar has no ⌘Q at all — nor ⌘C and ⌘V in the page's own fields. `tao` never
    // makes one. The icon's later timing is `tray-icon`'s rule about status items; a menu needs only
    // an `NSApplication`, which exists by now.
    //
    // Unconditional, like the icon and for the icon's reason: a `--browser` or `--lan` run has no
    // window to close and is still a Dock application somebody will press ⌘Q at. A failure is a
    // blemish rather than a reason to stop serving.
    let app_menu = km_tray::install_app_menu(TITLE, {
        let proxy = proxy.clone();
        move |command| {
            let _ = proxy.send_event(Wake::Tray(command));
        }
    })
    .inspect_err(|error| tracing::debug!(%error, "no menu bar; the remote is running all the same"))
    .ok();

    event_loop.run(move |event, _, control_flow| {
        // Held by the closure for its whole life. Named with an underscore because nothing reads it:
        // its only job is to not be dropped, which would stop the server the window is looking at.
        let _runtime = &runtime;
        // The same arrangement, for the same reason: the menu belongs to the run, not to the moment
        // it was built.
        let _app_menu = &app_menu;
        *control_flow = ControlFlow::Wait;

        match event {
            // **The icon is created here and not before the loop**, which is `tray-icon`'s own
            // requirement on macOS: an icon made before the loop is running misbehaves against
            // full-screen applications, and `StartCause::Init` is the earliest safe moment. Windows
            // does not care, and one arm for both beats two spellings of the same thing.
            Event::NewEvents(StartCause::Init) => {
                tray = build_tray(&url, window.is_some(), &proxy);
            }

            Event::UserEvent(Wake::Tray(command)) => match command {
                km_tray::Command::Show => match &window {
                    Some((window, _)) => {
                        window.set_visible(true);
                        window.set_focus();
                    }
                    // Offered only when there is a window, so this is unreachable in practice --
                    // and doing the other useful thing beats an unreachable panic.
                    None => open_browser(&url),
                },
                km_tray::Command::OpenInBrowser => open_browser(&url),
                // The karaoke machine's three, and this is not the machine: `Pages::JustThisOne`
                // means the menu never offers them. Spelled rather than left to a wildcard so that a
                // fourth page is a compile error here.
                km_tray::Command::OpenTheRemote
                | km_tray::Command::OpenTheWatchPage
                | km_tray::Command::OpenTheSetupPage => {}
                // The identical shutdown closing the window asks for, and the identical one Ctrl-C
                // runs. Not a second way out.
                km_tray::Command::Quit => {
                    stop.ask();
                    *control_flow = ControlFlow::Exit;
                }
            },

            // Nothing left to look at. The server has already finished, so there is no shutdown to
            // wait for and `LoopDestroyed` will find the outcome ready.
            Event::UserEvent(Wake::ServerStopped) => *control_flow = ControlFlow::Exit,

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                // Closing the window is what quitting means here — there is no second window, and no
                // Quit button on the page. It asks the server to stop rather than exiting outright,
                // so this is the identical shutdown Ctrl-C runs. **The icon in the bar and the menu
                // bar do not change this**: there is no minimize-to-tray, deliberately, and the
                // menu's Quit reaches the arm above rather than standing beside it. `Stop::ask` is
                // idempotent, so ⌘Q pressed again during the grace wait below is a no-op.
                stop.ask();
                *control_flow = ControlFlow::Exit;
            }

            // `run` calls `process::exit` after this, so nothing after the loop ever executes and no
            // destructor runs. Everything that must happen on the way out happens here — which for
            // this program is only waiting briefly for the shutdown asked for above. Dropping the
            // tray here is what takes the icon out of the bar rather than leaving a ghost of it.
            Event::LoopDestroyed => {
                tray.take();
                if let Some(outcome) = outcome.take() {
                    let waited = runtime
                        .block_on(async { tokio::time::timeout(SHUTDOWN_GRACE, outcome).await });
                    match waited {
                        Ok(Ok(Ok(Err(error)))) => {
                            km_console::say(format!("  the remote stopped: {error}"));
                        }
                        Ok(Ok(Err(error))) => {
                            km_console::say(format!("  the remote stopped: {error}"));
                        }
                        Err(_) => tracing::debug!("the server was still finishing; going anyway"),
                        _ => {}
                    }
                }
                // Last, because the lines above are among the ones it is here to save: a queue that
                // dies with the process loses exactly the entries a person is watching for.
                km_ecapplog::flush();
            }

            _ => {}
        }
    })
}

/// The window and the webview in it, or `None` having said why and opened a browser instead.
///
/// **Both failures fall back to the browser rather than to nothing**, and neither ends the run any
/// more. WebView2 is present on every Windows 11 and on all but a small number of Windows 10
/// machines — but not on Windows Server, and not on some LTSC builds. Detecting that in advance
/// would be a version check that goes stale; letting the build fail and carrying on windowless is a
/// few lines and covers every case, including ones that do not exist yet. What is left is exactly a
/// `--browser` run, icon in the bar included.
///
/// The webview is returned with the window because dropping it would take the page away.
fn open_window(
    event_loop: &EventLoop<Wake>,
    url: &str,
) -> Option<(tao::window::Window, wry::WebView)> {
    let (size, position) = km_webshell::opening_geometry(event_loop, WANTED);
    let mut builder = WindowBuilder::new().with_title(TITLE).with_inner_size(size);
    if let Some(position) = position {
        builder = builder.with_position(position);
    }

    let window = match km_webshell::with_icons(builder).build(event_loop) {
        Ok(window) => window,
        Err(error) => return fall_back(url, format!("could not open a window: {error}")),
    };

    // **The profile is named rather than left to the platform**, or WebView2 puts an
    // `km-remote.exe.WebView2` folder beside the executable and a distributed folder starts
    // growing browser profiles inside itself. See [`webview_data_dir`].
    //
    // The context is a local and is dropped when this function returns, which is correct rather
    // than an oversight: `build` unifies this borrow with the window's own and hands back a
    // `WebView` carrying no lifetime at all, so both end here. What wry warns a live `WebContext`
    // is still needed for is a custom protocol on macOS, and this program registers none.
    let mut context = wry::WebContext::new(webview_data_dir());

    let webview = match WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_new_window_req_handler(|url, _features| {
            open_outside(&url);
            wry::NewWindowResponse::Deny
        })
        // **The camera, for the share page's scanner, and nothing else.**
        //
        // The same grant both phone shells make, in the seam this one has for it. Without it the
        // engine asks — WebView2 puts its own bar up, and WKWebView asks on *every* call because a
        // webview never remembers the answer — so a page that starts its camera on load would
        // prompt on every visit.
        //
        // `Microphone` is refused explicitly rather than by falling through, because that is a
        // standing product decision and not an oversight: mic audio is mixed in hardware and
        // nothing here opens an input stream. Everything else is refused because nothing here asks
        // for it, and `Default` — which would hand the question back to the engine — is exactly the
        // prompt-on-every-visit this exists to remove.
        //
        // **No origin to check, unlike the iOS delegate.** wry hands over the kind alone, so the
        // guard that delegate makes by comparing a host is made here by the window only ever
        // loading `http://127.0.0.1:<port>/` — which `run` builds and nothing else can change.
        .with_permission_handler(|kind| match kind {
            wry::PermissionKind::Camera => wry::PermissionResponse::Allow,
            _ => wry::PermissionResponse::Deny,
        })
        // **Where a saved backup lands, said out loud.**
        //
        // wry's default handler allows every download "to match browser behavior", so the export
        // already worked before this — but the destination was whatever the platform backend chose
        // and nothing said what that was. A person who has just saved their collection and cannot
        // find it is the failure worth spending four lines on.
        .with_download_started_handler(|url, path| {
            tracing::info!(%url, file = %path.display(), "saving a download");
            true
        })
        .with_download_completed_handler(|url, path, success| match (success, path) {
            (true, Some(path)) => {
                km_console::say(format!("  saved {}", path.display()));
                tracing::info!(%url, file = %path.display(), "saved");
            }
            (true, None) => tracing::info!(%url, "saved, and the engine did not say where"),
            (false, _) => tracing::warn!(%url, "that download did not finish"),
        })
        .build(&window)
    {
        Ok(webview) => webview,
        Err(error) => return fall_back(url, format!("could not open a webview: {error}")),
    };

    Some((window, webview))
}

/// Puts the icon in the bar, and says nothing if it cannot.
///
/// `None` never stops the run, on the rule `km_webshell::with_icons` already follows: an icon that
/// could not be created is a blemish, and a program that refuses to serve over one is a bug.
fn build_tray(
    url: &str,
    has_a_window: bool,
    proxy: &tao::event_loop::EventLoopProxy<Wake>,
) -> Option<km_tray::Tray> {
    let spec = km_tray::Spec {
        title: TITLE,
        url: url.to_owned(),
        icon_png: TRAY_ICON_PNG,
        // **A tile, so not a template.** macOS would draw its silhouette, which is the tile: a
        // solid square. Only a mark drawn as a silhouette may say `true`, and the machine's
        // streaming icon is the one that is.
        icon_is_template: false,
        icon_ordinal: km_tray::DEFAULT_ICON_ORDINAL,
        has_a_window,
        // One page, and the address above it in the menu already names it.
        pages: km_tray::Pages::JustThisOne,
    };

    // Forwarded to the loop rather than acted on where it arrives: the handler runs on the
    // platform's own event thread, which is not a good place to close a window from.
    let proxy = proxy.clone();
    match km_tray::build(spec, move |command| {
        let _ = proxy.send_event(Wake::Tray(command));
    }) {
        Ok(tray) => Some(tray),
        Err(error) => {
            tracing::debug!(%error, "no icon in the bar; the remote is running all the same");
            None
        }
    }
}

/// Gives up on the window and hands the page to a browser instead. Always `None`.
///
/// **It used to end the run and now it does not**, which is the change the tray paid for. It waited
/// on the server here and exited; a windowless run is now an ordinary run of the event loop — the
/// same one `--browser` gets, icon in the bar included — so there is one shape instead of two and
/// the `ServerStopped` arm covers the ending for both.
fn fall_back(url: &str, reason: String) -> Option<(tao::window::Window, wry::WebView)> {
    km_console::say(format!("  (no window: {reason})"));
    km_console::say(format!("  opening {url} in a browser instead"));
    open_browser(url);
    None
}

/// A link that asked for a window of its own goes to the real browser instead.
///
/// **What makes `target="_blank"` work in here**, and until now nothing did: a webview has nowhere
/// to put a second window, so the platform raises a *new window requested* event and, with nothing
/// answering it, the click did nothing at all. The casualty here is the YouTube link on a song row
/// (`km-remote-pages`'s `_rows.html`), which is the one thing on this page that deliberately leaves
/// it. The Android and iOS shells over the same pages have always done this at the native layer —
/// `shouldOverrideUrlLoading` and `decidePolicyFor` — so this is the desktop catching up rather than
/// a new rule, and `km-package-builder`'s window carries the twin of this function.
///
/// **Only `http` and `https`.** What arrives is whatever the page asked to open and it ends up as an
/// argument to the platform's opener, which will open a file or a program as readily as a page.
///
/// Off the event thread, because this runs on the platform's own and `km_osopen` waits for the opener
/// to exit. `km_console` is not used: a windowed run may have no console, and unlike the two callers
/// below this is not answering something the person typed.
fn open_outside(url: &str) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        tracing::warn!(%url, "declined to open: not a web address");
        return;
    }
    tracing::debug!(%url, "opening outside the window");
    let url = url.to_owned();
    std::thread::spawn(move || {
        if let Err(error) = km_osopen::open_url(&url) {
            tracing::warn!(%error, %url, "could not open it");
        }
    });
}

/// Hands the address to whatever the platform calls a browser, and says so when it cannot.
///
/// Shared by the fallback above and by the tray's `Open in browser`, so both report a failure the
/// same way. It is `km_osopen` and not `km_console`: see the standing rule in that crate.
fn open_browser(url: &str) {
    if let Err(error) = km_osopen::open_url(url) {
        km_console::say(format!("  (could not open a browser: {error})"));
        km_console::say(format!("  the remote is running — open {url} yourself"));
    }
}

/// Where WebView2 keeps this program's browser profile, or `None` to let it choose.
///
/// **Named rather than left to the platform, because the platform's answer is a folder beside the
/// executable.** WebView2 defaults its user data folder to `<exe name>.WebView2` in the executable's
/// own directory, so a staged folder somebody unzips starts growing browser profiles inside itself
/// the first time each program in it is run. `dist/bin/windows` is the sharpest case, holding this
/// program and the package builder side by side; an installed build whose `{app}` is under
/// `C:\Program Files` is the sharper one still, having nowhere writable to put it at all.
///
/// **`cache_dir` and not `data_dir`**, which on Windows is the roaming `%APPDATA%`: this is
/// hundreds of megabytes of browser cache with no business following somebody onto another machine.
/// It is also honestly disposable — `km-remote-pages`'s pages are server-rendered htmx with no
/// `localStorage` — so deleting the folder is a repair rather than a loss.
///
/// **Not governed by `--data-dir`**, which names the catalog copy and the favorites. Those are
/// what a scratch run wants kept out of the way; a regenerable browser cache is not, and tying the
/// two would mean a flag about somebody's collection quietly deciding where a cache goes.
///
/// `None` never stops the window opening, on the rule `km_webshell::with_icons` already follows: it
/// means wry's own default, which is what every build before this one used.
#[cfg(windows)]
fn webview_data_dir() -> Option<std::path::PathBuf> {
    // The qualifier `resolve_data_dir` already uses, so this program has one identity under
    // `directories` rather than two.
    let dirs = directories::ProjectDirs::from("", "", "km-remote")?;
    let dir = dirs.cache_dir().join("webview");

    // Created here rather than left to WebView2, so that a failure becomes this function's `None`
    // — a wry default, and a window — instead of a webview that will not build and a run that
    // falls back to a browser it did not need to.
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::debug!(
            %error,
            dir = %dir.display(),
            "no webview profile directory; letting WebView2 choose"
        );
        return None;
    }

    tracing::debug!(dir = %dir.display(), "webview profile");
    Some(dir)
}

/// Everywhere else the platform already keeps this somewhere sensible, so there is nothing to name.
///
/// Only WebView2 reads a [`wry::WebContext`]'s data directory. WKWebView ignores the field outright
/// and keeps its state in the application's own container, and Linux never builds this feature at
/// all — so a path returned here would create a directory nothing would ever open.
#[cfg(not(windows))]
fn webview_data_dir() -> Option<std::path::PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The webview profile is somewhere per-user and absolute, and never beside the executable.
    ///
    /// It does **not** assert `Some`: a platform that will not name a home directory is a
    /// legitimate `None`, and wry's default is then the right answer. What this pins is the part
    /// that would silently regress — a relative path, or a directory under the executable, would
    /// both still *work* and would both put the profile back where this function exists to move it
    /// from.
    #[cfg(windows)]
    #[test]
    fn the_webview_profile_is_per_user_and_not_beside_the_executable() {
        let Some(dir) = webview_data_dir() else {
            return;
        };

        assert!(dir.is_absolute(), "not absolute: {}", dir.display());
        assert!(
            dir.ends_with("webview"),
            "not the profile: {}",
            dir.display()
        );

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
        if let Some(exe_dir) = exe_dir {
            assert!(
                !dir.starts_with(&exe_dir),
                "{} is beside the executable, which is what this exists to avoid",
                dir.display()
            );
        }
    }
}
