//! The window, and the icon in the bar. Behind the `desktop` feature.
//!
//! **A viewer over the page this program already serves**, and nothing else — there is no second
//! front end, and the webview loads the same loopback URL a browser would. That is the arrangement
//! `km-package-builder` and `km-remote` both use, and it is what keeps one set of templates
//! answering for the window and the browser alike.
//!
//! **`tao`'s event loop owns the main thread and its `run` never returns**, which is the whole
//! reason `crate::run` builds its runtime by hand rather than wearing `#[tokio::main]`. The runtime
//! is moved in here and held for the life of the process: dropping it would stop the server the
//! window exists to look at.
//!
//! **Never on Linux.** `wry` links libwebkit2gtk at load time, so a Linux build carrying this
//! feature does not *start* on a machine without it — a failure in the dynamic loader, before
//! `main`, that `--browser` cannot rescue. The tray rides on the same feature for the same kind of
//! reason: `tray-icon`'s Linux backend links `libayatana-appindicator`.
//!
//! **The window icons are `km_webshell::with_icons`' job**, both of them: `tao` clears the window's
//! two Windows icon slots rather than leaving them alone, and a window that fills only the title
//! bar's gets a taskbar button that stretches a 16-pixel drawing to 24. That was this tool's own
//! symptom. The argument, and why reading the compiled-in `.ico` back by ordinal is the cheap way to
//! do it, is on that function; all three tool windows call it and none of them repeats it.

use anyhow::Result;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use crate::server::APP_NAME;

/// The size to open at, before the screen is consulted.
///
/// Wider than tall because the review grid is the page that matters here: a hundred and twenty
/// thumbnails in rows, beside a column of thresholds.
const WANTED: (f64, f64) = (1400.0, 900.0);

/// This program's mark, for the icon in the bar.
///
/// Read on macOS and ignored on Windows, which takes the picture from the executable's own resource
/// — see `build.rs`.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../../../../../icon/km-admin-256.png");

/// Why the loop was woken from somewhere that is not the window.
enum Wake {
    /// The icon in the bar was used.
    Tray(km_tray::Command),
}

/// Opens the window and runs the event loop. Never returns.
pub fn run(url: &str, runtime: tokio::runtime::Runtime, wants_a_tray: bool) -> Result<()> {
    let event_loop: EventLoop<Wake> = EventLoopBuilder::with_user_event().build();

    let window = build_window(&event_loop, url);
    let has_a_window = window.is_some();

    let proxy = event_loop.create_proxy();

    // **Held rather than dropped**, both of them: dropping the tray takes the icon out of the bar,
    // and dropping the runtime stops the server the window exists to look at.
    let _tray = wants_a_tray
        .then(|| build_tray(url, has_a_window, &proxy))
        .flatten();
    let _runtime = runtime;

    // **The menu bar.** On macOS ⌘Q is a key equivalent on the application menu and not a key the
    // window is sent, so a shell with no menu bar has no ⌘Q at all — nor ⌘C and ⌘V in the page's own
    // fields. `tao` never makes one.
    //
    // **Not behind `wants_a_tray`.** The icon is something this run can be asked to go without; the
    // menu bar is not, being how the platform expects any application to be quit. That they are two
    // calls rather than one is what lets them differ — and what lets a failure of either say which
    // of the two was lost. A failure is a blemish rather than a reason to stop serving, exactly as
    // the icon's is.
    let _app_menu = km_tray::install_app_menu(APP_NAME, {
        let proxy = proxy.clone();
        move |command| {
            let _ = proxy.send_event(Wake::Tray(command));
        }
    })
    .inspect_err(|error| tracing::debug!(%error, "no menu bar; the tool is running all the same"))
    .ok();

    if !has_a_window {
        // A window that could not open is not a reason to stop serving. Say where the page is and
        // let whoever started this open it themselves.
        //
        // `km-admin` rather than `APP_NAME`: this is the console, which is the audience `What the
        // tool calls itself` gives the crate name, beside the startup banner it sits under. The
        // product's name belongs on the window that did not open.
        km_console::say(format!(
            "could not open a window; km-admin is still serving at {url}"
        ));
    }

    let opened = url.to_owned();
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            // **One arm and an inner `match`, rather than an arm per command.** Written per command
            // beside the loop's own `_ => {}`, a command added to `km_tray` compiled here and
            // reached nothing — the outer wildcard is for the dozen `tao` events this shell does not
            // care about, and it was silently covering the tray's as well. This way the inner match
            // is exhaustive and a new command is a compile error.
            Event::UserEvent(Wake::Tray(command)) => match command {
                km_tray::Command::Quit => *control_flow = ControlFlow::Exit,
                km_tray::Command::OpenInBrowser => {
                    let _ = km_osopen::open_url(&opened);
                }
                km_tray::Command::Show => {
                    if let Some((window, _)) = window.as_ref() {
                        window.set_visible(true);
                        window.set_focus();
                    }
                }
                // The karaoke machine's three, and this is not the machine: `Pages::JustThisOne`
                // means the menu never offers them. Spelled rather than left to a wildcard so that a
                // fourth page is a compile error here.
                km_tray::Command::OpenTheRemote
                | km_tray::Command::OpenTheWatchPage
                | km_tray::Command::OpenTheSetupPage => {}
            },
            // `run` calls `process::exit` after this, so nothing after the loop ever executes and
            // no destructor runs. This program has nothing else to do on the way out; draining the
            // viewer's queue is the one thing, and a queue that dies with the process loses exactly
            // the entries a person watching is there for. Without `--ecapplog` it does nothing.
            Event::LoopDestroyed => km_ecapplog::flush(),
            _ => {}
        }
    });
}

/// Puts the icon in the bar, and says nothing if it cannot.
///
/// An icon that could not be created is a blemish; a program that refuses to serve over one is a
/// bug — so this answers `None` and the run carries on.
fn build_tray(
    url: &str,
    has_a_window: bool,
    proxy: &tao::event_loop::EventLoopProxy<Wake>,
) -> Option<km_tray::Tray> {
    let spec = km_tray::Spec {
        title: APP_NAME,
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
    // platform's own event thread, which is not a good place to touch a window from.
    let proxy = proxy.clone();
    match km_tray::build(spec, move |command| {
        let _ = proxy.send_event(Wake::Tray(command));
    }) {
        Ok(tray) => Some(tray),
        Err(error) => {
            tracing::debug!(%error, "no icon in the bar; the program is running all the same");
            None
        }
    }
}

/// Builds the window and the webview in it, or says why not.
fn build_window(
    event_loop: &EventLoop<Wake>,
    url: &str,
) -> Option<(tao::window::Window, wry::WebView)> {
    let (size, position) = km_webshell::opening_geometry(event_loop, WANTED);
    let mut builder = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_inner_size(size);
    if let Some(position) = position {
        builder = builder.with_position(position);
    }
    let window = match km_webshell::with_icons(builder).build(event_loop) {
        Ok(window) => window,
        Err(error) => {
            tracing::warn!(%error, "could not open a window");
            return None;
        }
    };

    // **The profile is named rather than left to the platform**, or WebView2 puts a
    // `km-admin.exe.WebView2` folder beside the executable and a distributed folder starts growing
    // browser profiles inside itself.
    let mut context = wry::WebContext::new(webview_data_dir());

    let webview = match WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        // A license page, a provider's terms, a bank's own site: all of those are links this page
        // offers and none of them belongs inside this window.
        .with_new_window_req_handler(|url, _features| {
            let _ = km_osopen::open_url(&url);
            wry::NewWindowResponse::Deny
        })
        .build(&window)
    {
        Ok(webview) => webview,
        Err(error) => {
            tracing::warn!(%error, "could not open a webview");
            return None;
        }
    };

    Some((window, webview))
}

/// Where the webview keeps its profile.
///
/// Under this program's own data directory, so a distributed folder does not grow one beside the
/// executable.
fn webview_data_dir() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("", "", "km-admin").map(|dirs| dirs.data_dir().join("webview"))
}
