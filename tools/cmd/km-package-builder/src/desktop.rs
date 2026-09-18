//! A window of its own, instead of a tab in whatever browser is default — and an icon in the OS icon
//! bar whether or not there is a window.
//!
//! Behind the `desktop` feature, which is off in cargo and on when staging for Windows and macOS.
//! What this module adds is a frame around the page the server already serves: the webview points at
//! `http://127.0.0.1:<port>/`, so every handler, template and route is unchanged and none of them can
//! tell the difference. That is the whole design — this is a *viewer*, not a second front end.
//!
//! Three things here are not obvious and each cost something to learn.
//!
//! **The event loop is no longer conditional on the window.** An icon in the bar needs a platform
//! event loop exactly as a window does, and the runs that most need an icon are the ones with no
//! window: `--browser`, `--lan`, and the fallback taken when a webview will not build. So [`run`] is
//! entered by every windowed-executable run and `window` is a flag inside it. Two consequences:
//!
//! - **The loop has to end when the *server* ends.** The loop owns the main thread, so a Ctrl-C
//!   that stops the server does not end the process: a watcher task reports it as
//!   [`Wake::ServerStopped`] and the loop exits on that. Its `JoinHandle` is kept rather than
//!   dropped, because a dropped one detaches the task — harmless only where closing the window is
//!   the way out, and there are runs with no window to close.
//! - **The webview fallback does not park the thread forever.** A `sleep` loop there lets a Ctrl-C
//!   in a terminal shut the server down and leave the process alive; that fault is recorded in
//!   `crates/remote/km-remote/src/desktop.rs` and is closed here by the same arrangement, because a
//!   windowless run is an ordinary run of the loop rather than a dead end.
//!
//! **The event loop owns the main thread and never returns.** `tao`'s `run` diverges, and on exit it
//! calls `process::exit` — so destructors do not run and any shutdown work has to happen in
//! `Event::LoopDestroyed` rather than after the call. That is why `main` builds the tokio runtime by
//! hand for this path instead of using `#[tokio::main]`: the runtime has to be started, handed the
//! server, and then kept alive by something other than a `block_on` sitting on the main thread.
//!
//! **On macOS a double-clicked document does not arrive in `argv`.** LaunchServices delivers it as a
//! `kAEOpenDocuments` Apple Event, which AppKit turns into `application:openURLs:` on the application
//! delegate — and `tao` installs one, surfacing it as `Event::Opened`. Without an event loop there is
//! nothing to receive it, so on that platform the window is not a convenience but the mechanism. It
//! also means the path arrives *after* the loop has started, so the window is built first and
//! navigates when the event lands.
//!
//! **A second double-click does not start a second process** on macOS: LaunchServices routes it to
//! the running one, as another `Opened`. So swapping the folder in place is a requirement there
//! rather than a nicety, and is one of the reasons `State` holds a slot at all.
//!
//! **The window icons are set here on Windows, and this note has now been wrong twice.** It first
//! said the executable's own resource — which `build.rs` attaches — draws both the taskbar icon and
//! the title-bar one, and the title bar proved otherwise by wearing Windows' default. Then it said
//! filling `ICON_SMALL` was enough because the switcher "already shows the right picture", and the
//! taskbar proved *that* otherwise by going blocky. Both mistakes are the same one: treating two
//! separate slots as if the shell read one of them.
//!
//! What is actually true is in `km_webshell::with_icons`, which all three tool windows now call, and
//! it is not repeated here. The one argument worth keeping from the old note is the one that
//! survived both corrections: reading the compiled-in `.ico` back by ordinal beats decoding a PNG at
//! startup — no decoder, no new dependency, and no second copy of the picture to keep in step.
//!
//! macOS is unchanged and still needs nothing: the bundle's `CFBundleIconFile` is where that
//! platform looks, and there is no title bar icon to set. Linux never gets this feature at all.

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use crate::server::State;

/// Why the event loop was woken from outside itself.
///
/// `tao`'s user event, and the reason the loop is built with `with_user_event` at all. Both arms
/// arrive from another thread: the tray's handlers run on the platform's own event thread, and
/// [`Wake::ServerStopped`] comes off the tokio runtime.
#[derive(Debug, Clone)]
enum Wake {
    /// Somebody picked something from the icon in the bar.
    Tray(km_tray::Command),
    /// A link on one of this tool's own pages asked for a new window. The window has no second one to
    /// give, so the page loads in place. See [`open_window`].
    Navigate(String),
    /// The server finished on its own — a Ctrl-C in a terminal, or a failure. Nothing is left for
    /// the window to look at, so the loop ends.
    ServerStopped,
}

/// What the window is called before a folder is open.
const IDLE_TITLE: &str = crate::APP_NAME;

/// The mark this program wears in the bar, on macOS.
///
/// The blue one, which is this tool's own and not the machine's — see the `Application icon`
/// decision in docs/decisions/. Windows never reads it: there the picture comes back out of the
/// executable's own resources, which is the same `.ico` `build.rs` compiled in.
///
/// 256 rather than the 32 `server.rs` serves as a favicon, because the menu bar wants 44 physical
/// pixels on a Retina display and downscaling beats upscaling. See `km_tray`'s `decode`.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../../../../icon/km-package-builder-256.png");

/// The size the window asks for, in logical pixels, before the screen gets a say.
///
/// It was 1280x860, and the browse list did not fit in it. A row ends in five buttons and carries
/// three `select.mini` boxes besides, and `td.actions` is `width: 1%` -- which takes the difference
/// out of Title and Artist rather than out of the buttons, so what actually happens on a narrow
/// window is that the whole table overflows and `.scroll` starts scrolling sideways. The buttons are
/// then present and off the right-hand edge, which is the thing being reported.
///
/// A width and not a pixel-tuned fit: the columns are text in a font this code does not choose, so
/// there is no width that is exactly right, only one with enough slack that the usual case has none
/// of this. 1500 is that with room to spare on a 1920 screen at 100%.
const WANTED: (f64, f64) = (1500.0, 900.0);

/// Runs the desktop shell. Never returns.
///
/// `runtime` is moved into the event loop's closure and kept there: the server was spawned on it, and
/// dropping it would stop answering the very requests the webview is about to make.
pub fn run(
    window: bool,
    state: State,
    url: String,
    serving: tokio::task::JoinHandle<()>,
    runtime: tokio::runtime::Runtime,
) -> ! {
    let event_loop: EventLoop<Wake> = EventLoopBuilder::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // **The server's own ending has to reach the loop**, and this is the only thing that carries it.
    // The loop owns the main thread, so a Ctrl-C that stops the server does not end the process:
    // without this the loop would go on spinning in front of a server that had already stopped.
    runtime.spawn({
        let proxy = proxy.clone();
        async move {
            let _ = serving.await;
            let _ = proxy.send_event(Wake::ServerStopped);
        }
    });

    let window = window
        .then(|| open_window(&event_loop, &state, &url, &proxy))
        .flatten();

    // **Here, and not where the window was asked for.** A run that ends up in a browser after all
    // has to keep its Quit button — being a run with no window to close and, if it was started by a
    // double-click, no console to press Ctrl-C in. Where there *is* a window, the header can stop
    // offering a second way to do what closing it does and start offering the one thing a webview
    // cannot: a real browser. See `State::windowed`.
    //
    // The icon in the bar does not change this. It offers Quit in both shapes, and a page that also
    // offered it would be two buttons for one act in the case that already had one.
    if window.is_some() {
        state.set_windowed();
    }

    let open_page = format!("{}open", url.trim_end_matches('/').to_owned() + "/");
    let mut tray = None;

    // **The menu bar, and it goes in here rather than at `StartCause::Init` where the icon does.**
    // On macOS ⌘Q is a key equivalent on the application menu and not a key the window is sent, so a
    // shell with no menu bar has no ⌘Q at all — nor ⌘C and ⌘V in the page's own fields. `tao` never
    // makes one. The icon's later timing is `tray-icon`'s rule about status items; a menu needs only
    // an `NSApplication`, which exists by now.
    //
    // Held rather than dropped, and unconditional: a `--browser` run has no window to close and is
    // still a Dock application somebody will press ⌘Q at. A failure is a blemish, not a reason to
    // stop serving — the same rule the icon follows.
    let app_menu = km_tray::install_app_menu(crate::APP_NAME, {
        let proxy = proxy.clone();
        move |command| {
            let _ = proxy.send_event(Wake::Tray(command));
        }
    })
    .inspect_err(|error| tracing::debug!(%error, "no menu bar; the tool is running all the same"))
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
                // **Where the window actually is, not where it started.** The Songs page pushes its
                // filter into the address bar, so after twenty seconds of narrowing a corpus the
                // start address is the one place the person is *not*; handing that over threw the
                // work away and looked like the menu item not working.
                km_tray::Command::OpenInBrowser => {
                    open_browser(&showing(window.as_ref().map(|(_, webview)| webview), &url))
                }
                // The karaoke machine's three, and this is not the machine: `Pages::JustThisOne`
                // means the menu never offers them. Spelled rather than left to a wildcard so that a
                // fourth page is a compile error here.
                km_tray::Command::OpenTheRemote
                | km_tray::Command::OpenTheWatchPage
                | km_tray::Command::OpenTheSetupPage => {}
                // The identical shutdown closing the window asks for, and the identical one Ctrl-C
                // and the page's Quit button run. Not a fourth way out, a fourth door onto the same
                // one.
                km_tray::Command::Quit => {
                    state.ask_to_quit();
                    *control_flow = ControlFlow::Exit;
                }
            },

            // Here rather than in the new-window handler, which runs on the platform's own event
            // thread before the webview it would load into has been returned.
            Event::UserEvent(Wake::Navigate(page)) => {
                if let Some((_, webview)) = &window
                    && let Err(error) = webview.load_url(&page)
                {
                    tracing::warn!(%error, %page, "could not show the page");
                }
            }

            // Nothing left to look at, and nothing to ask for: the server has already stopped.
            Event::UserEvent(Wake::ServerStopped) => *control_flow = ControlFlow::Exit,
            // macOS only, and the reason `tao` is here rather than a plain window library: this is a
            // double-clicked `.kmbuild`, arriving as an Apple Event because LaunchServices does not
            // pass documents in `argv`. It is also how a *second* double-click reaches an already
            // running copy, which on that platform is the only way it can.
            Event::Opened { urls } => {
                for url in &urls {
                    let Ok(path) = url.to_file_path() else {
                        tracing::warn!("asked to open {url}, which is not a file");
                        continue;
                    };
                    let Some(folder) = crate::folder_of(&path) else {
                        tracing::warn!("{} is not a corpus", path.display());
                        continue;
                    };
                    tracing::info!("asked to open {}", folder.display());
                    if let Err(error) = state.begin_open(folder, false) {
                        tracing::warn!("could not open it: {error}");
                    }
                    // Straight to the picker, which shows the progress and navigates on its own when
                    // the folder is ready — the same path its own buttons take, rather than a second
                    // arrangement that could disagree with the first.
                    //
                    // **Only where there is a webview to send there**, which is new: a `--browser`
                    // run now reaches this loop too, and the folder still swaps under whatever tab
                    // is open. There is nothing useful to do about that tab from here — a second
                    // browser window on the same URL would be worse than leaving it to notice — and
                    // the folder having changed is what the person asked for either way.
                    if let Some((_, webview)) = &window
                        && let Err(error) = webview.load_url(&open_page)
                    {
                        tracing::warn!("could not show the Open page: {error}");
                    }
                }
            }

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                // Closing the window is what quitting means here — there is no other window to go
                // back to. It asks the server to stop rather than exiting outright, so this is the
                // same shutdown Ctrl-C, the Quit button, the tray's Quit and macOS's ⌘Q run. **The
                // icon in the bar and the menu bar do not change this**: there is no
                // minimize-to-tray, deliberately, and the menu's Quit reaches the arm above rather
                // than standing beside it.
                state.ask_to_quit();
                *control_flow = ControlFlow::Exit;
            }

            // `run` calls `process::exit` after this, so nothing after the loop ever executes and no
            // destructor runs. Everything that must happen on the way out happens here — which for
            // this tool means stopping the scan and checkpointing, both of which are `Workspace`'s
            // `Drop` and are reached by closing the folder.
            //
            // Dropping the tray here is what takes the icon out of the bar rather than leaving a
            // ghost of it, and it is done first because closing the folder is the slow half.
            Event::LoopDestroyed => {
                tray.take();
                state.close_folder();
                tracing::debug!("closed the folder on the way out");
                // Last, because the line above is one of the ones it is here to save: a queue that
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
///
/// **A link that asks for a new window lands in one of two places.** An address on this server loads
/// in place, through [`Wake::Navigate`]: the page asked for a tab so that a browser keeps the list
/// behind it, and a webview has no tab to give, so the next best is the page itself rather than a
/// browser opened onto the tool's own loopback address. Anything else leaves through
/// [`open_outside`]. Both answer `Deny`, because a webview has nowhere to put a second window.
fn open_window(
    event_loop: &EventLoop<Wake>,
    state: &State,
    url: &str,
    proxy: &tao::event_loop::EventLoopProxy<Wake>,
) -> Option<(tao::window::Window, wry::WebView)> {
    let (size, position) = km_webshell::opening_geometry(event_loop, WANTED);
    let mut builder = WindowBuilder::new()
        .with_title(title_for(state))
        .with_inner_size(size);
    if let Some(position) = position {
        builder = builder.with_position(position);
    }

    let window = match km_webshell::with_icons(builder).build(event_loop) {
        Ok(window) => window,
        Err(error) => return fall_back(url, format!("could not open a window: {error}")),
    };

    // **The profile is named rather than left to the platform**, or WebView2 puts a
    // `km-package-builder.exe.WebView2` folder beside the executable and a distributed folder
    // starts growing browser profiles inside itself. See [`webview_data_dir`].
    //
    // The context is a local and is dropped when this function returns, which is correct rather
    // than an oversight: `build` unifies this borrow with the window's own and hands back a
    // `WebView` carrying no lifetime at all, so both end here. What wry warns a live `WebContext`
    // is still needed for is a custom protocol on macOS, and this program registers none --
    // `load_url` in the `Event::Opened` arm is all it asks of the webview later.
    let mut context = wry::WebContext::new(webview_data_dir());

    let base = url.to_owned();
    let proxy = proxy.clone();
    let webview = match WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_new_window_req_handler(move |url, _features| {
            if is_ours(&url, &base) {
                let _ = proxy.send_event(Wake::Navigate(url));
            } else {
                open_outside(&url);
            }
            wry::NewWindowResponse::Deny
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
        title: IDLE_TITLE,
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
            tracing::debug!(%error, "no icon in the bar; the tool is running all the same");
            None
        }
    }
}
/// Gives up on the window and hands the page to a browser instead. Always `None`.
///
/// **It used to park this thread in a `sleep` loop forever**, which meant a Ctrl-C in a terminal
/// shut the server down and left the process alive — tokio's handler having suppressed the default
/// terminate. `km-remote`'s twin of this function recorded that fault and said this one could
/// take the same fix; what actually fixed it was the tray, because a windowless run is now an
/// ordinary run of the event loop rather than a shape of its own.
fn fall_back(url: &str, reason: String) -> Option<(tao::window::Window, wry::WebView)> {
    km_console::say(format!("  (no window: {reason})"));
    km_console::say(format!("  opening {url} in a browser instead"));
    open_browser(url);
    None
}

/// The size the failure window asks for, in logical pixels.
///
/// Small, because it holds one sentence and is read once. It goes through the same
/// `km_webshell::opening_geometry` as the real window so that a screen too small for it clamps it
/// rather than putting it half off the edge.
const ERROR_WANTED: (f64, f64) = (620.0, 340.0);

/// Shows a startup failure in a window of its own, then leaves with status 1.
///
/// **The case this exists for is the double-click.** A GUI-subsystem executable has no console and
/// null standard handles, so a failure on the way up is returned into nothing: the process exits and
/// the screen does not change, which is indistinguishable from the association being broken. What
/// reaches somebody who started this from a file manager is a window, and this is the smallest one
/// that carries a sentence.
///
/// **Only what has no page to land on comes here.** A folder that will not open is reported on the
/// Open page, chooser and all — see `opens_eagerly` — because that failure has somewhere better to
/// be and something to do about it. What is left is everything that happens before a server exists:
/// the address already taken, and a path that names neither a folder nor a `.kmbuild`.
///
/// **`wry` rather than a message box.** The platform's own dialog is an `unsafe` call with no safe
/// wrapper, and the workspace denies `unsafe_code` outside the one allowance `km-console` holds —
/// so a message box here would be a second exception, in the builds that already link a browser
/// engine for the window this is standing in for. See the `Unsafe code, once` decision in
/// `docs/decisions/`.
///
/// **Closing the window is the only control, deliberately.** A Close button inside the page would
/// need an IPC handler and a user event to reach the loop, which is machinery for a second way to do
/// what the title bar already does.
///
/// A window or webview that will not build leaves the same way without one. There is nothing further
/// to fall back to — a browser needs a server, and the server is what failed — and the caller has
/// already logged the reason.
///
/// **The page arrives built**, from `failure_html` in `lib.rs`, which is where the escaping that
/// matters can be tested: this module is behind a feature the test commands never turn on. No
/// `WebContext` either — a page with no network and no storage needs no profile beside it.
///
/// This is the only `EventLoop` the process will ever build. `tao` allows one and panics on a
/// second, and the path here is `start` having failed, which is before [`run`] can have been
/// entered.
pub fn show_error(html: &str) -> ! {
    let event_loop: EventLoop<()> = EventLoop::new();
    let (size, position) = km_webshell::opening_geometry(&event_loop, ERROR_WANTED);
    let mut builder = WindowBuilder::new()
        .with_title(IDLE_TITLE)
        .with_inner_size(size);
    if let Some(position) = position {
        builder = builder.with_position(position);
    }

    let window = match km_webshell::with_icons(builder).build(&event_loop) {
        Ok(window) => window,
        Err(error) => {
            tracing::error!(%error, "no window to show the startup failure in");
            std::process::exit(1);
        }
    };

    // Held by the closure below for its whole life: dropping it would take the page away.
    let webview = match WebViewBuilder::new().with_html(html).build(&window) {
        Ok(webview) => webview,
        Err(error) => {
            tracing::error!(%error, "no webview to show the startup failure in");
            std::process::exit(1);
        }
    };

    event_loop.run(move |event, _, control_flow| {
        let _webview = &webview;
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            // **Not `Exit`, which is status 0.** `tao`'s loop ends the process itself, so this is
            // the only place left to say that the run failed — and a double-click whose exit code
            // says success is one the installer's own `--register` check would believe.
            *control_flow = ControlFlow::ExitWithCode(1);
        }
    })
}

/// The page the webview is on, or `base` when there is no useful answer.
///
/// Three ways there is not one, and all three fall back rather than fail: no window at all (a
/// `--browser` run, or one whose webview would not build), a platform that will not say, and an
/// answer that is not on this server. The last is the one worth keeping — it is a prefix check
/// against our own loopback address, and it is what stops whatever the webview happens to be showing
/// from becoming an argument to `cmd /c start`.
fn showing(webview: Option<&wry::WebView>, base: &str) -> String {
    showing_or(webview.and_then(|webview| webview.url().ok()), base)
}

/// The choice [`showing`] makes, with the webview taken out of it so it can be asserted.
///
/// A `wry::WebView` cannot be built without a window and an event loop, so the platform call stays
/// in the caller and the decision lives here.
fn showing_or(showing: Option<String>, base: &str) -> String {
    showing
        .filter(|showing| is_ours(showing, base))
        .unwrap_or_else(|| base.to_owned())
}

/// Whether an address is a page on this server.
///
/// A prefix check against `base`, which ends in `/`: that slash is what stops `127.0.0.1:81780`
/// passing for `127.0.0.1:8178`.
fn is_ours(url: &str, base: &str) -> bool {
    url.starts_with(base)
}

/// A link that asked for a window of its own goes to the real browser instead.
///
/// **This is what makes `target="_blank"` work at all in here**, and until now it did not: a webview
/// has nowhere to put a second window, so WebView2 and WKWebView both raise a *new window requested*
/// event and, with nothing answering it, the click did nothing whatsoever. The YouTube button on a
/// browse row and on a song's own page were the two visible casualties. Denying the window and
/// handing the address to the platform is the same thing the Android and iOS shells already do with
/// `shouldOverrideUrlLoading` and `decidePolicyFor`, so all three hosts now agree: a link out of the
/// tool leaves the tool.
///
/// **Only `http` and `https`.** What arrives is whatever the page asked to open, and it ends up as an
/// argument to `cmd /c start` — which will happily open a file, a folder or a program. The page is
/// this tool's own, so this is a guard rather than a fix for anything known; it is one line and the
/// alternative is trusting a rendering engine's idea of a URL.
///
/// Off the event thread, because this runs on the platform's own and `km_osopen` waits for the opener
/// to exit. Nothing is reported back: there is no window of ours to report it in, and the log is
/// where a failure belongs.
fn open_outside(url: &str) {
    if !is_web(url) {
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

/// Whether an address is one the platform's browser should be handed. See [`open_outside`].
fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Hands the address to whatever the platform calls a browser, and says so when it cannot.
///
/// Shared by the fallback above and by the tray's `Open in browser`, so both report a failure the
/// same way. It is `km_osopen` and not `km_console`: see the standing rule in that crate.
fn open_browser(url: &str) {
    if let Err(error) = km_osopen::open_url(url) {
        km_console::say(format!("  (could not open a browser: {error})"));
        km_console::say(format!("  the tool is running — open {url} yourself"));
    }
}

/// Where WebView2 keeps this program's browser profile, or `None` to let it choose.
///
/// **Named rather than left to the platform, because the platform's answer is a folder beside the
/// executable.** WebView2 defaults its user data folder to `<exe name>.WebView2` in the executable's
/// own directory, so a staged folder somebody unzips starts growing browser profiles inside itself
/// the first time each program in it is run. `dist/bin/windows` is the sharpest case, holding this
/// program and the offline remote side by side; an installed build whose `{app}` is under
/// `C:\Program Files` is the sharper one still, having nowhere writable to put it at all.
///
/// **`cache_dir` and not `data_dir`**, which on Windows is the roaming `%APPDATA%`: this is
/// hundreds of megabytes of browser cache with no business following somebody onto another machine.
/// It is also honestly disposable — this tool's page is server-rendered htmx with no `localStorage`
/// — so deleting the folder is a repair rather than a loss. Note that it is emphatically **not**
/// where any curation state goes: that lives in the corpus's own `.kmbuild`, and the one per-user
/// file this tool keeps is `recent.json` in the config directory. See [`crate::recent`].
///
/// `None` never stops the window opening, on the rule `km_webshell::with_icons` already follows: it
/// means wry's own default, which is what every build before this one used.
#[cfg(windows)]
fn webview_data_dir() -> Option<std::path::PathBuf> {
    // The qualifier `recent.rs` already uses, so this program has one identity under `directories`
    // rather than two.
    let dirs = directories::ProjectDirs::from("", "", "km-package-builder")?;
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

/// The window title: the folder being curated, or the product's name before one is chosen.
fn title_for(state: &State) -> String {
    match state.root() {
        Some(root) => match root.file_name() {
            Some(name) => format!("{} — {IDLE_TITLE}", name.to_string_lossy()),
            None => root.display().to_string(),
        },
        None => IDLE_TITLE.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The title names the folder, and says the product's name when there is none.
    #[test]
    fn the_title_names_the_folder() {
        assert_eq!(title_for(&State::empty()), IDLE_TITLE);
    }

    /// The webview profile is somewhere per-user and absolute, and never beside the executable.
    ///
    /// It does **not** assert `Some`: a platform that will not name a home directory is a
    /// legitimate `None`, and wry's default is then the right answer. What this pins is the part
    /// that would silently regress — a relative path, or a
    /// directory under the executable, would both still *work* and would both put the profile back
    /// where this function exists to move it from.
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

    /// *Open in browser* hands over the page the window is on, not the one it started on.
    ///
    /// The filter lives in the address bar — htmx pushes it — so the start address is precisely the
    /// place a person narrowing a corpus is *not*, and opening it threw the narrowing away. The
    /// three fallbacks are the three ways there is no better answer, and the last is the one that
    /// matters: whatever a webview reports becomes an argument to the platform's opener, so an
    /// address that is not ours is refused rather than passed on.
    #[test]
    fn open_in_browser_takes_the_page_the_window_is_showing() {
        let base = "http://127.0.0.1:8178/";

        assert_eq!(
            showing_or(
                Some(format!("{base}songs?artist=Queen&sort=suitability")),
                base
            ),
            "http://127.0.0.1:8178/songs?artist=Queen&sort=suitability"
        );

        // No window, or a platform that would not say.
        assert_eq!(showing_or(None, base), base);
        // Somewhere else entirely. Not ours to open.
        assert_eq!(
            showing_or(Some("https://example.com/".to_owned()), base),
            base
        );
        // And the near miss the trailing slash on `base` exists to catch.
        assert_eq!(
            showing_or(Some("http://127.0.0.1:81780/evil".to_owned()), base),
            base
        );
    }

    /// A new window asked for on this server stays in the window, and nothing else does.
    #[test]
    fn only_this_servers_pages_load_in_place() {
        let base = "http://127.0.0.1:8178/";

        assert!(is_ours("http://127.0.0.1:8178/songs/abc", base));
        assert!(is_ours(
            "http://127.0.0.1:8178/similar?title=Wave&artist=Tom+Jobim",
            base
        ));

        assert!(!is_ours(
            "https://www.youtube.com/results?search_query=Tom+Jobim",
            base
        ));
        assert!(!is_ours("http://127.0.0.1:81780/songs/abc", base));
        assert!(!is_ours("javascript:alert(1)", base));
    }

    /// A link out of the window reaches the browser, and only a web address does.
    ///
    /// The guard is not about anything the tool's own pages do — it is about what the string ends up
    /// being: an argument to `cmd /c start`, which opens a program as readily as a page.
    #[test]
    fn only_a_web_address_leaves_the_window() {
        assert!(is_web(
            "https://www.youtube.com/results?search_query=Tom+Jobim"
        ));
        assert!(is_web("http://127.0.0.1:8178/songs"));

        assert!(!is_web("file:///C:/Windows/notepad.exe"));
        // A bare path, which is what `cmd /c start` is happiest to accept. The drive is the
        // documentation sample; `tools/dev/check-no-local-refs.sh` matches by shape and is right to.
        assert!(!is_web("D:\\tunes\\karaoke\\notasong.exe"));
        assert!(!is_web("javascript:alert(1)"));
        assert!(!is_web(""));
    }
}
