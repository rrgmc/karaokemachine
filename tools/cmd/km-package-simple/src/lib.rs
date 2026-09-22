//! `km-package-simple`: a `.kmpkg` from a folder, without a curation database.
//!
//! A web page on loopback, in a window of its own on Windows and macOS. Choose a folder, look down
//! the songs it holds, rename a few, leave some out, and build. Every package it writes carries the
//! `uncurated` flag, because nobody reviewed it. See `A package can be built straight from a folder`
//! in `docs/decisions/curation.md`.
//!
//! **The work is `km-pack`'s.** `describe` walks the folder, and `build` writes each package. What
//! this crate adds is a page to change the description on, and the division into volumes past 999
//! songs.
//!
//! It is a **library with two binaries hanging off it**, in the arrangement `km-remote` and
//! `km-package-builder` use: `#![windows_subsystem]` belongs to a binary's root, and the two builds
//! disagree about it.
//!
//! **Bind before anything slow**, so a browser sent to the address waits on a page that is loading
//! rather than one that was refused.

pub mod app;
#[cfg(feature = "desktop")]
mod desktop;
pub mod server;
pub mod session;
pub mod settings;
pub mod views;
pub mod words;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use km_console::say;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::app::App;

/// The port this tool asks for first.
///
/// The machine is 8177, the package builder 8178, the offline remote 8179 and `km-admin` 8180. A
/// second copy of this tool finds 8181 taken and takes whatever port the system offers.
pub const DEFAULT_PORT: u16 = 8181;

/// This library's own tracing target.
const SELF_TARGET: &str = env!("CARGO_CRATE_NAME");

/// Build a karaoke package straight from a folder, marked uncurated.
#[derive(Parser, Debug)]
#[command(name = "km-package-simple", about, version)]
struct Cli {
    /// A folder to read as soon as the page opens.
    folder: Option<PathBuf>,

    /// The port to serve the page on.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    // A window satisfies this rather than competing with it: the window is the page being shown.
    /// Open the page in a browser once it is running.
    ///
    /// A build with its own window opens that instead. Use `--browser` for a browser tab.
    #[arg(long)]
    open: bool,

    /// Show the page in your own browser rather than in a window of its own.
    #[arg(long)]
    browser: bool,

    /// Also write this run's log to a file, in the folder the startup lines name.
    ///
    /// `KM_LOG_FILE=1` does the same where there is no command line.
    #[arg(long)]
    log_file: bool,

    /// Print more detail. Repeat for more: `-v` for this tool and `km-pack`, `-vv` for everything.
    ///
    /// `RUST_LOG` overrides this entirely when it is set.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

impl Cli {
    /// The tracing filter this verbosity asks for. Plain `info` by default, so a shipped build
    /// says nothing at `debug` unless asked.
    fn log_filter(&self) -> String {
        if let Ok(filter) = std::env::var("RUST_LOG") {
            return filter;
        }
        match self.verbose {
            0 => "info".to_owned(),
            1 => format!("info,{SELF_TARGET}=debug,km_pack=debug"),
            _ => format!("debug,{SELF_TARGET}=trace,km_pack=trace"),
        }
    }
}

/// Which of the two executables is running.
pub enum Shell {
    /// `km-package-simple`: a window of its own, where the build has one.
    Windowed,
    /// `km-package-simple-console`: never a window.
    Console,
}

/// Whether this run puts the page in a window of its own.
fn will_have_a_window(shell: &Shell, cli: &Cli) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(feature = "desktop") && !cli.browser
}

/// Whether this run puts an icon in the OS icon bar. It follows the executable, not the window.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn will_have_a_tray(shell: &Shell) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(feature = "desktop")
}

/// Whether to hand the address to a browser once the server listens.
///
/// Implied when nobody can read a printed address, unless a window is about to show the page.
fn will_open_a_browser(shell: &Shell, cli: &Cli, nowhere_to_talk: bool) -> bool {
    !will_have_a_window(shell, cli) && (cli.open || nowhere_to_talk)
}

/// The entry point, called by both binaries.
///
/// The runtime is built by hand rather than by `#[tokio::main]`, because a window's event loop
/// must own the main thread.
pub fn run(shell: Shell) -> Result<()> {
    let cli = Cli::parse();
    let log_file = init_logging(&cli);

    km_console::decide_where_to_talk(match shell {
        Shell::Windowed => km_console::Console::NotWanted,
        Shell::Console => km_console::Console::Wanted,
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;

    match runtime.block_on(start(cli, log_file, shell))? {
        #[cfg(feature = "desktop")]
        Started::Shell {
            window,
            app,
            serving,
        } => desktop::run(window, app, serving, runtime),
        Started::Serving { serving } => runtime.block_on(serving)?,
    }
}

/// What [`start`] decided, once the server listens.
enum Started {
    /// Hand the main thread to a desktop event loop.
    #[cfg(feature = "desktop")]
    Shell {
        /// Whether to open a window in it.
        window: bool,
        /// The state, for its address and its stop.
        app: Arc<App>,
        /// The spawned server.
        serving: tokio::task::JoinHandle<Result<()>>,
    },
    /// Wait on the server on this thread.
    Serving {
        /// The spawned server.
        serving: tokio::task::JoinHandle<Result<()>>,
    },
}

/// Everything up to a listening socket, and then out of the way.
async fn start(cli: Cli, log_file: Option<km_logfile::LogFile>, shell: Shell) -> Result<Started> {
    let listener = bind(cli.port).await?;
    let address = listener.local_addr().context("reading the bound address")?;
    let url = format!("http://{address}/");

    let app = App::new(settings::default_path(), url.clone());
    app.windowed.store(
        will_have_a_window(&shell, &cli),
        std::sync::atomic::Ordering::Relaxed,
    );
    if let Some(folder) = &cli.folder
        && let Err(key) = app.read_folder(folder.clone())
    {
        say(format!(
            "  {}",
            words::messages(km_locale::Locale::English).msg(key)
        ));
    }

    print_banner(&url, log_file.as_ref());

    if will_open_a_browser(&shell, &cli, km_console::nowhere_to_talk()) {
        open_browser(&url);
    }

    let router = server::router(Arc::clone(&app));
    let stop = app.stop.clone();
    let serving = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    () = stop.asked() => {}
                }
                tracing::info!("stopping");
            })
            .await
            .context("serving the page")
    });

    #[cfg(feature = "desktop")]
    if will_have_a_tray(&shell) {
        return Ok(Started::Shell {
            window: will_have_a_window(&shell, &cli),
            app,
            serving,
        });
    }

    Ok(Started::Serving { serving })
}

/// Binds loopback on `port`, or on any free port when that one is taken.
async fn bind(port: u16) -> Result<tokio::net::TcpListener> {
    let wanted = SocketAddr::from(([127, 0, 0, 1], port));
    match tokio::net::TcpListener::bind(wanted).await {
        Ok(listener) => Ok(listener),
        Err(error) => {
            tracing::info!(%error, port, "the port is taken; taking any free one");
            tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
                .await
                .context("binding a loopback port")
        }
    }
}

/// Starts the tracing subscriber, with a file beside the console when one was asked for.
fn init_logging(cli: &Cli) -> Option<km_logfile::LogFile> {
    let wanted = km_logfile::asked_for(cli.log_file);
    let dir = directories::ProjectDirs::from("", "", "km-package-simple")
        .map(|dirs| dirs.data_dir().join(km_logfile::SUBDIR));
    let (file, failure) = match dir
        .as_ref()
        .filter(|_| wanted)
        .map(|dir| km_logfile::LogFile::open(dir, "km-package-simple"))
    {
        Some(Ok(file)) => (Some(file), None),
        Some(Err(error)) => (None, Some(error.to_string())),
        None if wanted => (None, Some("this platform has no data directory".to_owned())),
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(EnvFilter::new(cli.log_filter()))
        .with(tracing_subscriber::fmt::layer())
        .with(file.as_ref().map(|file| file.layer()))
        .init();

    if let Some(reason) = failure {
        tracing::warn!(
            reason,
            "could not open a log file; the log goes to the console only"
        );
    }
    file
}

/// The lines a person reads once: where the log is and what to open.
fn print_banner(url: &str, log_file: Option<&km_logfile::LogFile>) {
    say("km-package-simple");
    if let Some(file) = log_file {
        say(format!("  log        {}", file.path().display()));
    }
    say(format!("  open       {url}"));
}

/// Opens the page in whatever the platform calls a browser, and says so when it cannot.
fn open_browser(url: &str) {
    if let Err(error) = km_osopen::open_url(url) {
        say(format!(
            "  (could not open a browser: {error} — open {url} yourself)"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_parse_and_a_folder_is_optional() {
        let cli = Cli::parse_from(["km-package-simple"]);
        assert_eq!(cli.port, DEFAULT_PORT);
        assert!(cli.folder.is_none());
        let cli = Cli::parse_from(["km-package-simple", "/tunes/karaoke", "--browser"]);
        assert_eq!(cli.folder, Some(PathBuf::from("/tunes/karaoke")));
        assert!(cli.browser);
    }

    /// A window instead of a browser, never both; and a double-click with no window opens a tab.
    #[test]
    fn a_window_is_shown_instead_of_a_browser_and_never_as_well_as_one() {
        let decide = |args: &[&str], shell: Shell, nowhere: bool| {
            let cli =
                Cli::parse_from(std::iter::once("km-package-simple").chain(args.iter().copied()));
            (
                will_have_a_window(&shell, &cli),
                will_open_a_browser(&shell, &cli, nowhere),
            )
        };
        let windowed = cfg!(feature = "desktop");
        assert_eq!(decide(&[], Shell::Windowed, true), (windowed, !windowed));
        assert_eq!(
            decide(&["--browser", "--open"], Shell::Windowed, false),
            (false, true)
        );
        assert_eq!(decide(&[], Shell::Console, false), (false, false));
        assert_eq!(decide(&["--open"], Shell::Console, false), (false, true));
    }

    #[test]
    fn verbosity_chooses_a_filter() {
        if std::env::var_os("RUST_LOG").is_some() {
            return;
        }
        assert_eq!(Cli::parse_from(["km-package-simple"]).log_filter(), "info");
        assert_eq!(
            Cli::parse_from(["km-package-simple", "-v"]).log_filter(),
            "info,km_package_simple=debug,km_pack=debug"
        );
    }
}
