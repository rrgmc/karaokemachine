//! A window of its own, and an icon in the OS icon bar whether or not there is a window.
//!
//! Behind the `desktop` feature, which is on when staging for Windows and macOS and never on
//! Linux. It is `km-remote`'s `desktop.rs` with the camera and the downloads taken out, and that
//! file carries the reasoning at length: why the event loop is not conditional on the window, why
//! it ends when the server ends, and why the webview falling back to a browser is not a dead end.
//!
//! **Closing the window is the quit.** It asks for the same stop Ctrl-C and the tray's Quit ask
//! for, and there is no minimize-to-tray.

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use std::sync::Arc;

use anyhow::Result;

use crate::app::App;

/// Why the event loop was woken from outside itself.
#[derive(Debug, Clone, Copy)]
enum Wake {
    /// Somebody picked something from the icon in the bar.
    Tray(km_tray::Command),
    /// The server finished on its own, so there is nothing left to look at.
    ServerStopped,
}

/// The mark this program wears in the bar on macOS: the package builder's, until it has its own.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../../../../icon/km-package-builder-256.png");

/// What the window is called.
const TITLE: &str = "KaraokeMachine Simple Package Builder";

/// The size the window asks for. Landscape, because the song list is a wide table.
const WANTED: (f64, f64) = (1300.0, 900.0);

/// How long to wait for the server to finish on the way out.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Runs the desktop shell. Never returns.
pub fn run(
    window: bool,
    app: Arc<App>,
    serving: tokio::task::JoinHandle<Result<()>>,
    runtime: tokio::runtime::Runtime,
) -> ! {
    let url = app.url.clone();
    let stop = app.stop.clone();
    let event_loop: EventLoop<Wake> = EventLoopBuilder::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let (finished, outcome) = tokio::sync::oneshot::channel();
    runtime.spawn({
        let proxy = proxy.clone();
        async move {
            let _ = finished.send(serving.await);
            let _ = proxy.send_event(Wake::ServerStopped);
        }
    });

    let window = window.then(|| open_window(&event_loop, &url)).flatten();
    // A webview that would not build left a browser tab instead, and that page has no window to
    // close, so it keeps its Quit.
    app.windowed
        .store(window.is_some(), std::sync::atomic::Ordering::Relaxed);

    let mut outcome = Some(outcome);
    let mut tray = None;

    let app_menu = km_tray::install_app_menu(TITLE, {
        let proxy = proxy.clone();
        move |command| {
            let _ = proxy.send_event(Wake::Tray(command));
        }
    })
    .inspect_err(|error| tracing::debug!(%error, "no menu bar; the tool is running all the same"))
    .ok();

    event_loop.run(move |event, _, control_flow| {
        let _runtime = &runtime;
        let _app_menu = &app_menu;
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                tray = build_tray(&url, window.is_some(), &proxy);
            }

            Event::UserEvent(Wake::Tray(command)) => match command {
                km_tray::Command::Show => match &window {
                    Some((window, _)) => {
                        window.set_visible(true);
                        window.set_focus();
                    }
                    None => open_browser(&url),
                },
                km_tray::Command::OpenInBrowser => open_browser(&url),
                km_tray::Command::OpenTheRemote
                | km_tray::Command::OpenTheWatchPage
                | km_tray::Command::OpenTheSetupPage => {}
                km_tray::Command::Quit => {
                    stop.ask();
                    *control_flow = ControlFlow::Exit;
                }
            },

            Event::UserEvent(Wake::ServerStopped) => *control_flow = ControlFlow::Exit,

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                stop.ask();
                *control_flow = ControlFlow::Exit;
            }

            // `run` calls `process::exit` after this, so everything on the way out happens here.
            Event::LoopDestroyed => {
                tray.take();
                if let Some(outcome) = outcome.take() {
                    let waited = runtime
                        .block_on(async { tokio::time::timeout(SHUTDOWN_GRACE, outcome).await });
                    match waited {
                        Ok(Ok(Ok(Err(error)))) => {
                            km_console::say(format!("  the tool stopped: {error}"));
                        }
                        Ok(Ok(Err(error))) => {
                            km_console::say(format!("  the tool stopped: {error}"));
                        }
                        Err(_) => tracing::debug!("the server was still finishing; going anyway"),
                        _ => {}
                    }
                }
            }

            _ => {}
        }
    })
}

/// The window and its webview, or `None` having opened a browser instead.
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

    // Named, or WebView2 grows a profile folder beside the executable. See `webview_data_dir`.
    let mut context = wry::WebContext::new(webview_data_dir());

    let webview = match WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_new_window_req_handler(|url, _features| {
            open_outside(&url);
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
fn build_tray(
    url: &str,
    has_a_window: bool,
    proxy: &tao::event_loop::EventLoopProxy<Wake>,
) -> Option<km_tray::Tray> {
    let spec = km_tray::Spec {
        title: TITLE,
        url: url.to_owned(),
        icon_png: TRAY_ICON_PNG,
        icon_is_template: false,
        icon_ordinal: km_tray::DEFAULT_ICON_ORDINAL,
        has_a_window,
        pages: km_tray::Pages::JustThisOne,
    };
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
fn fall_back(url: &str, reason: String) -> Option<(tao::window::Window, wry::WebView)> {
    km_console::say(format!("  (no window: {reason})"));
    km_console::say(format!("  opening {url} in a browser instead"));
    open_browser(url);
    None
}

/// A link that asked for a window of its own goes to the real browser. Only `http` and `https`.
fn open_outside(url: &str) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        tracing::warn!(%url, "declined to open: not a web address");
        return;
    }
    let url = url.to_owned();
    std::thread::spawn(move || {
        if let Err(error) = km_osopen::open_url(&url) {
            tracing::warn!(%error, %url, "could not open it");
        }
    });
}

/// Hands the address to a browser, and says so when it cannot.
fn open_browser(url: &str) {
    if let Err(error) = km_osopen::open_url(url) {
        km_console::say(format!("  (could not open a browser: {error})"));
        km_console::say(format!("  the tool is running — open {url} yourself"));
    }
}

/// Where WebView2 keeps this program's browser profile: the per-user cache folder, never beside
/// the executable. `None` is wry's own default, and the window still opens.
#[cfg(windows)]
fn webview_data_dir() -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "km-package-simple")?;
    let dir = dirs.cache_dir().join("webview");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::debug!(%error, dir = %dir.display(), "no webview profile directory");
        return None;
    }
    Some(dir)
}

/// Only WebView2 reads a data directory, so elsewhere there is nothing to name.
#[cfg(not(windows))]
fn webview_data_dir() -> Option<std::path::PathBuf> {
    None
}
