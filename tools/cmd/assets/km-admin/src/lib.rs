//! KaraokeMachine Admin — finding pictures and SoundFont banks for a machine, and sending those
//! and files you already have to it.
//!
//! **The machine takes files and nothing helped anybody find one.** `/admin/` grew three upload
//! routes; what it cannot do is *search*, because it runs in the machine's process on a box under a
//! television that may have no internet, should not be holding somebody's Pixabay key, and has no
//! business spending a hundred JPEG decodes on a picture while it is meant to be playing a song.
//! This is the other half: a program on a desktop, with the network and the CPU, that produces a
//! file and hands it over.
//!
//! Two things it goes and gets, and the reason they are one program rather than two is that they are
//! one errand — *the machine needs something it has not got*:
//!
//! * **Pictures**, through `km-wallpaper-pack`'s three phases, whose contrast gate is the thing that
//!   makes a photograph safe to put lyrics over. Openverse by default, needing no key; Pixabay and
//!   Pexels only once somebody has pasted their own, under their own agreement with those sites.
//! * **Sound**, from the sixty-three-row bank table in `km-banks`, verified against the digest the
//!   table pins and then uploaded. The machine can fetch a bank itself where it has the network;
//!   this is what serves the machine that has not.
//!
//! **And one it does not go and get, because nobody could.** A package of **songs** is somebody's
//! own and no stock library has it — but a television box has no shell and no file manager that
//! reaches where the machine looks, so a `.kmpkg` on a laptop had no route onto it at all. Sending a
//! file that is already on this computer is the same errand read from the other end, and the Pictures
//! and Sound pages take one too: the `.sf2` somebody bought, the photograph they took.
//!
//! **It keeps what it *made*, lists it, and sends it when told to.** A machine that is switched off,
//! or whose password nobody remembers, is then a delay rather than a dead end — and the second
//! machine in a house costs no second download, because Send is a button on a row rather than
//! something that happened by itself an hour ago at whichever machine was current when the download
//! started. A file that was already on the disk is the exception and is passed straight through: it
//! cost nothing to have, it is still where it was picked from, and a second copy of somebody's
//! library is not this program's business. See [`staging`].
//!
//! ## Shape
//!
//! A library with two thin binaries, `km-package-builder`'s arrangement for
//! `km-package-builder`'s reason: `#![windows_subsystem]` is a property of a binary crate root and
//! the windowed and console builds disagree about it.
//!
//! The browser talks only to this program, on loopback; this program talks to the machine
//! server-to-server. **So CORS never enters it** — `api.cors_origins` ships empty on the machine and
//! stays irrelevant, which is the first thing anybody asks about.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

pub mod bank;
pub mod chosen;
pub mod handlers;
pub mod host;
pub mod job;
pub mod keys;
pub mod machine;
pub mod passwords;
pub mod pictures;
pub mod run;
pub mod server;
pub mod staging;
pub mod views;
pub mod words;

/// The port this serves on when nothing says otherwise.
///
/// One past the offline remote's 8179, which is one past the package builder's 8178, which is one
/// past the machine's 8177. Four programs in this product can be running at once on one desk, and
/// the ports being adjacent is what makes "which one is that?" answerable.
pub const DEFAULT_PORT: u16 = 8180;

/// Find pictures and SoundFont banks for a karaoke machine, and send those and your own files to it.
#[derive(Debug, Parser)]
#[command(name = "km-admin", version, about, long_about = None)]
pub struct Cli {
    // The same ordering `km_remote_core::find::locate` keeps — asked for, then remembered, then the
    // network — and [`crate::chosen`] is the file the middle one lives in.
    /// The machine to send things to: an address or a full URL.
    ///
    /// A bare host or IP uses the machine's default port. Without this, the machine chosen last
    /// time is used, and the page lists machines found on the network.
    ///
    /// Applies to this run only and is not saved.
    #[arg(long, value_name = "ADDR")]
    pub machine: Option<String>,

    // Never the repository's `.wpcache`, and never the shared asset cache a development box keeps:
    // both belong to somebody else's work.
    /// Where downloads, built packs and this program's settings are kept.
    ///
    /// Defaults to this application's folder in the platform's data directory.
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

    /// The port to serve the page on.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    pub port: u16,

    /// Serve on every network interface rather than loopback only.
    ///
    /// Not recommended. This program stores your API keys, can write files as you, and has no
    /// password. It prints a warning when this is used.
    #[arg(long)]
    pub lan: bool,

    /// Open the page in a browser once the server is running.
    ///
    /// Assumed when there is no console to print the address to.
    #[arg(long)]
    pub open: bool,

    /// Use a browser rather than this program's own window.
    #[arg(long)]
    pub browser: bool,

    /// Also write the log to a file under the data directory.
    #[arg(long)]
    pub log_file: bool,

    // The destination somebody watches while the tool runs, where the file above is the one they go
    // and find afterwards. It replaces the console rather than joining it: both would print every
    // line twice.
    /// Send this run's log to the ECAppLog viewer instead of the console.
    ///
    /// Give an address to reach a viewer on another computer:
    /// `--ecapplog=192.168.1.x:13991`. The viewer does not have to be running yet — lines wait for
    /// it and arrive when it opens.
    ///
    /// `KM_ECAPPLOG=1` does the same for every program at once.
    ///
    /// `--log-file` is unaffected.
    #[arg(
        long,
        value_name = "ADDR",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = km_ecapplog::DEFAULT_ADDRESS,
        value_parser = km_ecapplog::parse_address,
    )]
    pub ecapplog: Option<String>,

    /// More logging. Repeat for more still.
    ///
    /// `RUST_LOG` overrides it entirely when set, and `logging.level` in this program's
    /// settings.json says the same thing for a run started from an icon.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl Cli {
    /// The tracing filter this verbosity asks for.
    ///
    /// `RUST_LOG` wins when it is set, which is what makes a one-off "show me everything the HTTP
    /// client is doing" possible without adding a flag for it.
    ///
    /// **`logging.level` is the rung below `-v` and speaks `RUST_LOG`'s grammar**, for the reason
    /// the machine's ladder gives: a program with a window is one people start from an icon. It
    /// replaces the ladder rather than moving along it, and `-v` beats it so one run is never an
    /// edit to the box.
    pub fn log_filter(&self, settings: &km_logsettings::LoggingSettings) -> String {
        if let Ok(filter) = std::env::var("RUST_LOG") {
            return filter;
        }
        if self.verbose == 0
            && let Some(level) = settings.level()
        {
            return level.to_owned();
        }
        match self.verbose {
            0 => "km_admin=info,km_wallpaper_pack=info".to_owned(),
            1 => "km_admin=debug,km_wallpaper_pack=debug".to_owned(),
            _ => "km_admin=trace,km_wallpaper_pack=trace,reqwest=debug".to_owned(),
        }
    }

    /// What to listen on.
    pub fn bind(&self) -> SocketAddr {
        let host = if self.lan {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        SocketAddr::new(host, self.port)
    }
}

/// Which of the two binaries is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// `km-admin` — a window of its own, where the build has one.
    Windowed,
    /// `km-admin-console` — never a window.
    ///
    /// Compiled from the same library and could open a window as easily as the other one; what it
    /// must not do is *want* to, because it exists so that somebody at a terminal has a program that
    /// talks back to them.
    Console,
}

/// Whether this run will put the page in a window of its own.
///
/// **A window is asked for by the build rather than by the user.** `--browser` declines it, `--lan`
/// declines it too because a webview on a machine nobody is sitting at is pointless, and the console
/// twin never wants one.
fn will_have_a_window(shell: Shell, cli: &Cli) -> bool {
    window_matrix(shell, cli, cfg!(feature = "desktop"))
}

/// [`will_have_a_window`]'s rule, with the build's own answer handed in.
///
/// **Split out for the reason `nowhere_to_talk` is a parameter of [`will_open_a_browser`]**: so the
/// rule is a function of its arguments and can be asserted. It could not be before. `cfg!(feature =
/// "desktop")` read inside the function meant the desktop half of the matrix was only testable on a
/// build with that feature — and on Linux that feature links libwebkit2gtk, which this project
/// deliberately never installs, so CI runs the assets workspace without it and always will.
///
/// The test for the desktop half was therefore written, marked `#[cfg(feature = "desktop")]`, and
/// **never once executed**. Its sibling's doc even said so: *"the matrix, on a build without the
/// feature — which is every build CI runs."* It was failing, and it was right to: it caught
/// `--browser` opening nothing.
fn window_matrix(shell: Shell, cli: &Cli, desktop_built: bool) -> bool {
    matches!(shell, Shell::Windowed) && desktop_built && !cli.browser && !cli.lan
}

/// Whether this run puts an icon in the OS icon bar.
///
/// **It does not depend on the window, and that is the point.** A run that declined the window is
/// exactly the run with nothing else to show for itself: no window, and on a double-clicked
/// GUI-subsystem executable no console either.
///
/// Compiled on every build rather than gated, so the `#[cfg(not(feature = "desktop"))]` test below
/// runs in CI — which never turns the feature on.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn will_have_a_tray(shell: Shell) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(feature = "desktop")
}

/// Whether to hand the address to a browser once the server is listening.
///
/// **`--open` is implied when nobody can read the address.** Double-clicked there is no console for
/// the URL to have been printed to, so a build that waited to be asked would start, serve, and show
/// nothing at all — indistinguishable from one that failed to start.
///
/// **Unless a window is about to show the page.** A window *is* the page being shown, so it
/// satisfies `--open` rather than competing with it, and `--browser` is how you ask for a tab
/// instead of a window. `nowhere_to_talk` is passed in rather than read from the console module so
/// that this is a function of its arguments and can be asserted.
///
/// **`cli.browser` is in the disjunction, and its absence was a bug.** The flag's own help says
/// *"use a browser rather than this program's own window"* — so it has to open one. It did not:
/// `--browser` declined the window in [`will_have_a_window`] and then failed the `--open ||
/// nowhere_to_talk` test, so a `--browser` run from a console served the page and opened nothing at
/// all. There is a test that says so, and it had never been run — see
/// `the_window_matrix_holds_either_way`.
fn will_open_a_browser(shell: Shell, cli: &Cli, nowhere_to_talk: bool) -> bool {
    !will_have_a_window(shell, cli) && (cli.open || cli.browser || nowhere_to_talk)
}

/// The entry point, called by both binaries.
///
/// **Not `#[tokio::main]`, and the reason is the window.** That attribute builds a runtime and parks
/// the main thread inside `block_on` for the life of the process — fine for a server and impossible
/// for a desktop shell, because `tao`'s event loop must own the main thread and its `run` never
/// returns. So the runtime is built by hand and the server is *spawned* on it rather than awaited.
pub fn run(shell: Shell) -> Result<()> {
    // **The command line is read before the subscriber exists**, because the subscriber's level is
    // one of the things it decides — and because `--help` and `--version` should print and exit
    // while the console is still attached and before anything has been initialized on their behalf.
    let cli = Cli::parse();

    let data_dir = resolve_data_dir(&cli)?;
    let log_file = init_logging(&cli, &data_dir);
    // Drains the viewer's queue on the way out of every exit this function has. The windowed shell
    // is the one it does not reach -- `tao`'s loop calls `process::exit` -- so `desktop.rs` drains
    // by name where it does the rest of its shutdown.
    let _drain = km_ecapplog::flush_on_drop();

    // **Which executable this is decides whether the console it was handed matters**, and only the
    // build knows: on Windows both binaries can be double-clicked and both are given the same
    // console, so the twin that exists to be *read* must keep it and the other must let it go.
    // Freeing the wrong one made a double-click open a window and close it again.
    km_console::decide_where_to_talk(match shell {
        Shell::Windowed => km_console::Console::NotWanted,
        Shell::Console => km_console::Console::Wanted,
    });
    let nowhere_to_talk = km_console::nowhere_to_talk();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;

    let state = server::State::new(data_dir.clone(), cli.machine.clone());
    let bound = runtime.block_on(server::bind(cli.bind()))?;
    let door = bound.front_door();

    print_banner(&door, &cli, log_file.as_ref().map(|file| file.path()));

    let server = runtime.spawn(server::serve(bound, state));

    if will_open_a_browser(shell, &cli, nowhere_to_talk) {
        let opening = door.clone();
        runtime.spawn(async move {
            if let Err(error) = tokio::task::spawn_blocking(move || km_osopen::open_url(&opening))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()))
            {
                km_console::say(format!("could not open a browser: {error}"));
            }
        });
    }

    // Whatever happens next owns the main thread, and both shapes need the runtime kept alive:
    // dropping it would stop the server they are both there to look at.
    #[cfg(feature = "desktop")]
    if will_have_a_window(shell, &cli) {
        return desktop::run(&door, runtime, will_have_a_tray(shell));
    }

    runtime
        .block_on(server)
        .context("the server task ended unexpectedly")?
}

/// Where downloads, packs and settings go.
///
/// **Its own folder, never the machine's and never a checkout's.** `.wpcache` in a repository is a
/// developer's, and `~/.cache/karaokemachine/assets` is a development box's shared bank cache with
/// gigabytes of somebody's survey in it — a shipped program writing into either would be a surprise.
fn resolve_data_dir(cli: &Cli) -> Result<PathBuf> {
    if let Some(dir) = &cli.data_dir {
        return Ok(dir.clone());
    }
    let dirs = directories::ProjectDirs::from("", "", APP_DIR)
        .context("no data directory on this platform; pass --data-dir")?;
    Ok(dirs.data_dir().to_path_buf())
}

/// What this program calls itself to `directories`.
///
/// The lowercase spelling is a different name from the product's -- see `What the product is called`
/// in docs/decisions/foundations.md -- so this is the crate name and follows a rename of it.
const APP_DIR: &str = "km-admin";

/// Starts tracing, and opens a log file if one was asked for.
///
/// **A log file that could not be opened is a warning, never a refusal to start.** The run that most
/// wants a log is the one where something else is already wrong.
fn init_logging(cli: &Cli, data_dir: &std::path::Path) -> Option<km_logfile::LogFile> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    // **Three sources, narrowest first**: what was typed, then what the environment was given, then
    // what this box was set up to do. `asked_for` covers `KM_LOG_FILE=1` as well as the flag, so
    // the two spellings agree across every program in this product, and the file is the rung that
    // reaches a run started from an icon.
    //
    // Naming a count is asking for a history, and a history nobody is writing is not one -- so a
    // `keep` in the file turns the log file on by itself, exactly as `KM_LOG_KEEP` does.
    let settings = crate::pictures::peek_logging(data_dir);
    let keep = km_logfile::keep_wanted(None).or_else(|| settings.keep());
    let wanted = km_logfile::asked_for(cli.log_file) || settings.file || keep.is_some();
    let keep = keep.unwrap_or(km_logfile::KEEP);
    let (file, failure) = if wanted {
        match km_logfile::LogFile::open_keeping(data_dir.join(km_logfile::SUBDIR), "km-admin", keep)
        {
            Ok(file) => (Some(file), None),
            Err(error) => (None, Some(error.to_string())),
        }
    } else {
        (None, None)
    };

    // **The viewer takes the console's place rather than standing beside it**, which is the one
    // destination here that displaces another: a window with a tab per crate and a level to filter
    // on is a better console than a console, and two of them would print every line twice. The file
    // is untouched -- see `A log that goes to a viewer instead of a console` in docs/decisions/.
    // **Three sources, narrowest first**, the same ladder the file above climbs and for the same
    // reason: the variable is what a checkout sets once for every program in it, and the file is
    // what reaches a run started from an icon.
    let (viewer, viewer_failure) = match km_ecapplog::asked_for(cli.ecapplog.as_deref()) {
        Ok(asked) => (asked.or_else(|| settings.ecapplog()), None),
        Err(reason) => (None, Some(reason)),
    };
    let viewer =
        viewer.map(|address| km_ecapplog::EcAppLog::open(&address, crate::server::APP_NAME));

    tracing_subscriber::registry()
        .with(EnvFilter::new(cli.log_filter(&settings)))
        .with(viewer.is_none().then(tracing_subscriber::fmt::layer))
        .with(viewer.as_ref().map(|viewer| viewer.layer()))
        .with(file.as_ref().map(|file| file.layer()))
        .init();

    if let Some(viewer) = viewer {
        km_ecapplog::install(viewer);
    }

    // Said through the console layer that has just been installed, which is there precisely because
    // the viewer this names could not be opened.
    if let Some(reason) = viewer_failure {
        tracing::warn!(
            reason,
            "could not read where the ECAppLog viewer is; this run's log goes to the console"
        );
    }
    // Here rather than beside the peek above, for the same reason: a key nobody can read is
    // reported through the log it failed to configure, which is the only place there is.
    settings.warn_about_unread();
    if let Some(reason) = failure {
        tracing::warn!(
            reason,
            "could not open a log file; the log goes to the console only"
        );
    }
    file
}

/// What the program says once it is listening.
///
/// **Through `km_console::say` and never `println!`.** A GUI-subsystem executable on Windows has a
/// null stdout handle, where `println!` panics — which is the whole reason that crate exists.
fn print_banner(url: &str, cli: &Cli, log_file: Option<&std::path::Path>) {
    // The crate name and neither display name: `What the tool calls itself` puts the banner,
    // `--help` and the console lines beside them on the side of the shell, where the thing somebody
    // typed is the thing they would grep for. `APP_NAME` is the page's name, and this is the one
    // place that is not a page.
    km_console::say(format!("km-admin is at {url}"));
    if cli.lan {
        km_console::say(
            "Serving on every interface. There is no password on this program, it holds whatever \
             API keys you give it, and it writes files as you.",
        );
    }
    // The one line this tool says about a destination it is no longer printing to. Said here rather
    // than where the connection opens, which is before this process has settled whether it has a
    // console at all.
    if let Some(address) = km_ecapplog::address() {
        km_console::say(format!("Logging to ECAppLog at {address}"));
    }
    if let Some(path) = log_file {
        km_console::say(format!("Logging to {}", path.display()));
    }
}

#[cfg(feature = "desktop")]
mod desktop;

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        let mut argv = vec!["km-admin"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).expect("parse")
    }

    #[test]
    fn the_command_line_is_valid() {
        // clap's own audit: a duplicate flag, a bad default, a short that collides.
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    /// The viewer is off unless asked for, and naming one takes an `=`.
    #[test]
    fn the_viewer_is_off_unless_asked_for() {
        assert_eq!(cli(&[]).ecapplog, None);
        assert_eq!(
            cli(&["--ecapplog"]).ecapplog,
            Some(km_ecapplog::DEFAULT_ADDRESS.to_owned())
        );
        assert_eq!(
            cli(&["--ecapplog=1.2.3.4:99"]).ecapplog,
            Some("1.2.3.4:99".to_owned())
        );
        assert!(Cli::try_parse_from(["km-admin", "--ecapplog=nope"]).is_err());
        assert!(Cli::try_parse_from(["km-admin", "--ecapplog", "1.2.3.4:99"]).is_err());
    }

    #[test]
    fn the_default_port_is_one_past_the_offline_remotes() {
        // 8177 machine, 8178 package builder, 8179 remote, 8180 this. Asserted by literal on
        // purpose: four programs on one desk, and the ports being adjacent is what makes one
        // identifiable from a URL.
        assert_eq!(DEFAULT_PORT, 8180);
        assert_eq!(cli(&[]).port, 8180);
        assert_eq!(cli(&["--port", "9000"]).port, 9000);
    }

    #[test]
    fn loopback_unless_lan_is_asked_for() {
        // A program with no password and somebody's API keys in it does not go on the network
        // because a default said so.
        assert_eq!(cli(&[]).bind().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            cli(&["--lan"]).bind().ip(),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
    }

    #[test]
    fn verbosity_raises_the_filter_and_rust_log_beats_it() {
        // Not asserted against a live RUST_LOG, which a developer may well have set.
        if std::env::var("RUST_LOG").is_err() {
            let none = km_logsettings::LoggingSettings::default();
            assert!(cli(&[]).log_filter(&none).contains("km_admin=info"));
            assert!(cli(&["-vv"]).log_filter(&none).contains("trace"));
        }
    }

    /// The settings file is the rung below the flag, and a directive replaces the ladder.
    #[test]
    fn a_settings_file_says_the_level_where_nobody_can_type_one() {
        if std::env::var("RUST_LOG").is_ok() {
            return;
        }
        let asking = |level: &str| km_logsettings::LoggingSettings {
            level: Some(level.to_owned()),
            ..km_logsettings::LoggingSettings::default()
        };
        // Nobody typed a flag, so the file speaks -- and it speaks whole.
        assert_eq!(cli(&[]).log_filter(&asking("debug")), "debug");
        // One run typed at a keyboard is never an edit to the box.
        assert!(
            cli(&["-vv"])
                .log_filter(&asking("debug"))
                .contains("km_admin=trace")
        );
        // A directive nobody can read leaves the ladder standing.
        assert!(cli(&[]).log_filter(&asking("=")).contains("km_admin=info"));
    }

    /// Both halves of the window matrix, on **any** build.
    ///
    /// **Both halves in one test rather than a `#[cfg]`-split pair, of which one would never run.**
    /// A desktop half gated on `feature = "desktop"` is a test CI never executes: that feature links
    /// libwebkit2gtk on Linux, this project deliberately never installs it, and the assets workspace
    /// is therefore built without the feature every time.
    ///
    /// It was failing when it was finally run, and it was right to: `--browser` declined the window
    /// and then failed `will_open_a_browser`'s test, so a `--browser` run from a console served the
    /// page and opened nothing at all. Taking `desktop_built` as an argument — the treatment
    /// `nowhere_to_talk` already had, and for the same stated reason — is what makes both halves
    /// assertable everywhere.
    #[test]
    fn the_window_matrix_holds_either_way() {
        // Without the feature there is never a window, whatever is asked for.
        for shell in [Shell::Windowed, Shell::Console] {
            assert!(!window_matrix(shell, &cli(&[]), false));
        }

        // With it, a window is the default and three things decline it.
        assert!(window_matrix(Shell::Windowed, &cli(&[]), true));
        assert!(!window_matrix(Shell::Windowed, &cli(&["--browser"]), true));
        assert!(!window_matrix(Shell::Windowed, &cli(&["--lan"]), true));
        assert!(!window_matrix(Shell::Console, &cli(&[]), true));
    }

    /// What opens a browser, on the build this one is.
    ///
    /// `will_open_a_browser` reads the feature through `will_have_a_window`, so this half stays
    /// honest about the build it runs on rather than pretending: the first two assertions hold
    /// either way, and the last pair is written for whichever build is compiling it.
    #[test]
    fn a_browser_opens_when_nothing_else_will_show_the_page() {
        // `--browser` says so outright. **This is the assertion that was failing**, on the half of
        // the matrix nothing ever compiled.
        assert!(will_open_a_browser(
            Shell::Windowed,
            &cli(&["--browser"]),
            false
        ));
        // Nothing asked and nothing to print to: the page would otherwise be invisible.
        assert!(will_open_a_browser(
            Shell::Windowed,
            &cli(&["--browser"]),
            true
        ));

        // A window IS the page being shown, so it satisfies `--open` rather than competing with it;
        // without one, `--open` is the only thing left that can honour the request. Getting this
        // wrong opened a window and a browser tab on the same URL.
        assert_eq!(
            will_open_a_browser(Shell::Windowed, &cli(&["--open"]), false),
            !cfg!(feature = "desktop")
        );
        assert_eq!(
            will_open_a_browser(Shell::Windowed, &cli(&[]), true),
            !cfg!(feature = "desktop")
        );
        // Nothing asked for, and a console to print the address to.
        assert!(!will_open_a_browser(Shell::Windowed, &cli(&[]), false));
    }

    /// The tray does not depend on the window: the run that declined one is the run with nothing
    /// else to show for itself.
    #[test]
    fn the_tray_is_the_windowed_shells_and_not_the_console_one() {
        assert_eq!(will_have_a_tray(Shell::Windowed), cfg!(feature = "desktop"));
        assert!(!will_have_a_tray(Shell::Console));
    }
}
