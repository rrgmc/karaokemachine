//! The offline karaoke remote.
//!
//! A web server on loopback holding this phone's own copy of a machine's catalog. Browsing,
//! searching and favorites are answered from that copy, so **they work with the karaoke machine
//! switched off** — which is the whole reason this exists, and the difference between it and the
//! remote the machine serves at its own address.
//!
//! **Almost none of that is in here.** The remote itself is `km-remote-core`, which has no command
//! line, no idea where its files should go, nowhere to print and no opinion about how it is asked
//! to stop — because the same server is going to be an Android application and an iOS one, and
//! those four things are exactly what a phone answers differently. This crate is the desktop's
//! answers: `clap`, `directories`, `km-console` and `tokio::signal`, and a window to put the page
//! in.
//!
//! It is a **library with two binaries hanging off it**, in the arrangement
//! `tools/cmd/km-package-builder` settled on and for the identical reason: `#![windows_subsystem]` is a
//! property of a *binary crate root*, and the two builds disagree about it. Everything is here;
//! each binary is three lines and a [`Shell`].
//!
//! Startup order matters and is the same one `km-package-builder` settled on: **bind before doing
//! anything slow**, so that a browser sent to the address waits on a tab that is loading rather
//! than one that was refused.
//!
//! That is a property of the types rather than of the order of lines in one function:
//! `km_remote_core::Bound` is a listening socket and nothing else, and `Server::open` is everything
//! slow. The banner is printed from `Server::ready()` the moment there is something to say, rather
//! than after both databases are open.

#[cfg(feature = "desktop")]
mod desktop;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use km_console::say;
use km_remote_core::{Bound, Config, Ready, Server, Stop};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// This library's own tracing target, asked of cargo rather than typed out.
///
/// `km_remote` today. Its two companions in the filter string cannot be derived here — see the
/// note in [`Cli::log_filter`] — and export their own [`km_remote_core::LOG_TARGET`] and
/// [`km_remote_core::PAGES_LOG_TARGET`] instead.
const SELF_TARGET: &str = env!("CARGO_CRATE_NAME");

/// What this program calls itself to the ECAppLog viewer, which labels each connection with it.
///
/// **Not `km-remote`, which is what a shell and a package manager call it.** A person picking one of
/// several connected programs out of a list reads this, so it is the `FileDescription` string
/// `build.rs` gives Windows — one product, and this is which program in it. The machine, the package
/// builder and the picture-and-bank tool spell theirs the same way.
const VIEWER_NAME: &str = "KaraokeMachine Remote";

/// The offline karaoke remote.
#[derive(Parser, Debug)]
#[command(name = "km-remote", about, version)]
struct Cli {
    /// Where the karaoke machine is — `192.168.1.5`, or a full URL. Found automatically if omitted.
    #[arg(long)]
    machine: Option<String>,

    /// Where to keep the catalog copy and the favorites. Defaults to the platform's data folder.
    #[arg(long)]
    data_dir: Option<PathBuf>,

    // 8179: one past `km-package-builder`'s 8178, which is one past the machine's own 8177. The
    // number lives in `km-remote-core` so a second shell cannot pick a different one by copying
    // this line and forgetting to.
    /// The port to serve the remote on.
    #[arg(long, default_value_t = km_remote_core::DEFAULT_PORT)]
    port: u16,

    /// Serve on every network interface rather than only to this device.
    #[arg(long)]
    lan: bool,

    // A window satisfies this rather than competing with it: the window *is* the page being shown.
    /// Open the remote in a browser once it is running.
    ///
    /// A build with its own window opens that instead. Use `--browser` for a browser tab.
    #[arg(long)]
    open: bool,

    // Only a build with the `desktop` feature has a window to decline; every Linux build, any build
    // made with `--no-desktop`, and `km-remote-console` behave this way whatever is passed. Accepted
    // everywhere so a script need not know which build it is talking to.
    /// Show the remote in your own browser rather than in a window of its own.
    #[arg(long)]
    browser: bool,

    /// Re-read the whole catalog even if nothing has changed.
    #[arg(long)]
    refresh: bool,

    // The case it exists for is the double-click: with the `desktop` feature this is a
    // GUI-subsystem executable with a window and no console, so every line it writes is discarded
    // and a remote that will not start has nothing to show for itself.
    /// Also write this run's log to a file in the data directory's `logs` folder.
    ///
    /// The file holds the same output as the console, uncolored and timestamped. One file per run,
    /// the ten newest kept, in the folder named in the startup banner.
    ///
    /// `KM_LOG_FILE=1` does the same where there is no command line.
    #[arg(long)]
    log_file: bool,

    // The destination somebody watches while the remote runs, where the file above is the one they
    // go and find afterwards. It replaces the console rather than joining it: both would print
    // every line twice.
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
    ecapplog: Option<String>,

    /// Print more detail. Repeat for more: `-v` for this remote, `-vv` for everything.
    ///
    /// `RUST_LOG` overrides this entirely when it is set.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

impl Cli {
    /// The tracing filter this verbosity asks for.
    ///
    /// **The default is plain `info`.** A default of `info,km_remote=debug,km_remote_pages=debug`
    /// makes every shipped build emit this binary's whole debug stream — the mistake the `What a
    /// shipped build says out loud` decision is written about. Debug output is a developer's, and a
    /// release that produces it by default has decided on the owner's behalf that they wanted it.
    ///
    /// It is quieter than `km-app`'s frame report and far from silent: this crate and
    /// `km_remote_pages` between them have eleven `debug!` sites, nearly all on error and reconnect
    /// paths — and one of those, "the event stream is down" in `client.rs`, repeats every 30
    /// seconds forever while the machine is switched off, which is the state this remote is built to
    /// be useful in.
    ///
    /// **`-v` raises all three crates, and that grouping is the point.** `km_remote_pages` is the
    /// templates and handlers and `km_remote_core` is now where every reconnect, import and
    /// discovery message comes from — between them they are almost everything this program has to
    /// say about itself — so raising only `km_remote` would make `-v` quieter than a reader asking
    /// for it expects. The three rise together because all three are this one
    /// program talking about itself; they are separate crates for the sake of Android and iOS, not
    /// because a person watching a log cares where the boundary is.
    ///
    /// The `RUST_LOG` check is spelled out rather than deferred to
    /// `EnvFilter::try_from_default_env`, matching `km-app`: the two differ on a malformed filter,
    /// and one behavior across every shipped binary is what this is for.
    fn log_filter(&self) -> String {
        if let Ok(filter) = std::env::var("RUST_LOG") {
            return filter;
        }
        // Three crates named in one string, and only the first can be asked of cargo here:
        // `CARGO_CRATE_NAME` answers for whoever is compiling. The other two export the name
        // themselves, so a rename cannot leave this filter pointing at a target that is not there —
        // which is silence rather than an error, and therefore the kind of fault that survives.
        let (own, core, pages) = (
            SELF_TARGET,
            km_remote_core::LOG_TARGET,
            km_remote_core::PAGES_LOG_TARGET,
        );
        match self.verbose {
            0 => "info".to_owned(),
            1 => format!("info,{own}=debug,{core}=debug,{pages}=debug"),
            _ => format!("debug,{own}=trace,{core}=trace,{pages}=trace"),
        }
    }
}

/// Which of the two executables is running.
///
/// **Not a flag**, and deliberately not one: a person does not choose this, they choose which icon
/// to double-click or which name to type. `--browser` is the flag that declines a window, and it
/// means exactly what it says — this is the *build* saying whether it has one to decline.
pub enum Shell {
    /// `km-remote` — a window of its own, where the build has one.
    Windowed,
    /// `km-remote-console` — never a window; the browser-served remote this has always been.
    ///
    /// The console twin is compiled from the same library and could open a window as easily as the
    /// other one; what it must not do is *want* to, because it exists so that somebody at a
    /// terminal has a program that talks back to them.
    Console,
}

/// Whether this run will put the page in a window of its own.
///
/// **A window is asked for by the build rather than by the user.** `--browser` is how you decline
/// it, `--lan` declines it too, and the console twin never has one to begin with — that is what it
/// is for.
///
/// `--lan` carries an argument here that it does not in `km-package-builder`. There the reasoning
/// is that a webview on a machine nobody is sitting at is pointless; here there is a second one, and
/// it is sharper: `--lan` already warns that it is serving one person's favorites to the whole
/// network, so a run that did that *and* opened a personal window on this desk would be two
/// contradictory readings of one flag.
fn will_have_a_window(shell: &Shell, cli: &Cli) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(feature = "desktop") && !cli.browser && !cli.lan
}

/// Whether this run puts an icon in the OS icon bar.
///
/// **It does not depend on the window, and that is the whole point of it.** A run that declined the
/// window — `--browser`, `--lan`, or a build whose webview would not open — is exactly the run with
/// nothing else to show for itself: no window, and on a double-clicked GUI-subsystem executable no
/// console either. That is the case the icon exists for, so the only questions are whether the
/// platform has a bar and whether this executable is the one somebody double-clicks.
///
/// The console twin is left out for the reason it exists: it has a console in the taskbar and a
/// Ctrl-C that works, so an icon would be a second answer to a question already answered. Linux is
/// left out with the window, on the same terms. See the `A running server has an icon in the bar`
/// decision in docs/decisions/.
///
/// Compiled on every build rather than gated, so that the `#[cfg(not(feature = "desktop"))]` test
/// below runs in CI — which never turns the feature on. Only `start` calls it in earnest, and only
/// under the feature, hence the allowance: `will_have_a_window` escapes the same fate only because
/// `will_open_a_browser` happens to call it.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn will_have_a_tray(shell: &Shell) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(feature = "desktop")
}

/// Whether to hand the address to a browser once the server is listening.
///
/// **`--open` is implied when nobody can read the address.** Double-clicked, there is no console
/// for the URL to have been printed to, so a build that waited to be asked would be a process that
/// starts, serves, and shows nothing at all — indistinguishable from one that failed to start.
///
/// **Unless a window is about to show the page.** A window is the page being shown, so it satisfies
/// `--open` rather than competing with it, and `--browser` is how you ask for a tab *instead of* a
/// window. `nowhere_to_talk` is passed in rather than read from `km_console` so that this is a
/// function of its arguments and can be asserted.
fn will_open_a_browser(shell: &Shell, cli: &Cli, nowhere_to_talk: bool) -> bool {
    !will_have_a_window(shell, cli) && (cli.open || nowhere_to_talk)
}

/// The entry point, called by both binaries.
///
/// **Not `#[tokio::main]`, and the reason is the window.** That attribute builds a runtime and parks
/// the main thread inside `block_on` for the life of the process — which is fine for a server and
/// impossible for a desktop shell, because `tao`'s event loop must own the main thread and its `run`
/// never returns. So the runtime is built by hand, the server is *spawned* rather than awaited, and
/// what happens on the main thread afterwards depends on which shape was asked for.
pub fn run(shell: Shell) -> Result<()> {
    // **The command line is read before the subscriber exists**, because the subscriber's level is
    // one of the things it decides. It is also the better order for the other two things clap can
    // do: `--help` and `--version` print and exit while the console is still attached and before
    // anything has been initialized on their behalf.
    let cli = Cli::parse();

    // **And the data directory before it too**, because `--log-file` writes into it — so where it
    // is has to be settled before there is anywhere for a line to go. Resolved here and passed in,
    // rather than inside `start` on the far side of the runtime, so there is exactly one answer.
    //
    // The failure is kept rather than propagated here: it is a platform that will not name a data
    // directory at all, which means there is nowhere to log *and* nothing to log about. Saying it
    // through the subscriber below beats returning into a GUI-subsystem `main` that has no stderr
    // to print it on.
    let data_dir = resolve_data_dir(cli.data_dir.clone());
    let log_file = init_logging(&cli, data_dir.as_ref().ok());
    let data_dir = data_dir.inspect_err(|error| tracing::error!(%error, "no data directory"))?;

    // Before the first line is said, and that ordering is not stylistic: with no console `println!`
    // *panics* rather than failing, so anything printed beforehand would take the process with it.
    // Everything below says its piece through `km_console::say`, which knows which of the two sinks
    // exists.
    //
    // **The twin keeps whatever console it was given**, and that is the one thing `km-console`
    // cannot work out for itself: both executables can be double-clicked and both are handed the
    // same console, so only the build knows which of them exists to be read. Freeing the twin's made
    // a double-click open a window and close it again.
    km_console::decide_where_to_talk(match shell {
        Shell::Windowed => km_console::Console::NotWanted,
        Shell::Console => km_console::Console::Wanted,
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;

    match runtime.block_on(start(cli, data_dir, log_file, shell))? {
        // A desktop shell: the event loop takes the main thread and never gives it back. Whether
        // there is a window inside it is the `window` flag; either way there is an icon in the bar,
        // and an icon needs a loop to be delivered events on.
        #[cfg(feature = "desktop")]
        Started::Shell {
            window,
            url,
            stop,
            serving,
        } => desktop::run(window, url, stop, serving, runtime),
        Started::Serving { serving } => runtime.block_on(serving)?,
    }
}

/// What [`start`] decided to do, once the server is listening.
enum Started {
    /// Hand the main thread to a desktop event loop. The server is already spawned.
    ///
    /// **A shell rather than a window**, because the icon in the bar needs an event loop as much as
    /// a window does — so the loop is not conditional on the window and the window is a flag inside
    /// it.
    #[cfg(feature = "desktop")]
    Shell {
        /// Whether to open a window in it. [`will_have_a_window`]'s answer.
        window: bool,
        /// Where the page is: what to point a webview at, and what the tray offers to open.
        url: String,
        /// How to ask the server to stop when the window is closed or `Quit` is picked.
        stop: Stop,
        /// The spawned server, so the way out can wait briefly for it.
        serving: tokio::task::JoinHandle<Result<()>>,
    },
    /// Nothing on this thread but the server: wait on it here, as this program always did.
    Serving {
        /// The spawned server.
        serving: tokio::task::JoinHandle<Result<()>>,
    },
}

/// Everything up to a listening socket, and then out of the way.
///
/// `data_dir` and `log_file` are settled by [`run`] before the subscriber exists — see the note
/// there — and are passed in rather than worked out again here.
async fn start(
    cli: Cli,
    data_dir: PathBuf,
    log_file: Option<km_logfile::LogFile>,
    shell: Shell,
) -> Result<Started> {
    let bind = if cli.lan {
        SocketAddr::from(([0, 0, 0, 0], cli.port))
    } else {
        SocketAddr::from(([127, 0, 0, 1], cli.port))
    };
    let config = Config::new(data_dir)
        .with_bind(bind)
        .with_machine(cli.machine.clone())
        .with_force_refresh(cli.refresh);

    // Bound before either database is opened, so that a browser sent here during a long first
    // import waits on a tab that is loading rather than one that was refused.
    let bound = Bound::bind(&config).await?;
    let server = Server::open(bound, config).await?;

    print_banner(server.ready(), cli.lan, log_file.as_ref());

    // Behind the pages rather than in front of them. A cold import of a six-figure catalog is a
    // minute, and it used to run before `axum::serve` was called at all — so every request sat in
    // the accept backlog until it finished, which is a slow tab here and a launch that looks like a
    // hang on a phone. What is served meanwhile is whatever the mirror already holds, which is the
    // whole premise of an offline remote.
    server.spawn_warm_up();

    let url = server.ready().url.clone();
    let stop = Stop::default();

    if will_open_a_browser(&shell, &cli, km_console::nowhere_to_talk()) {
        open_browser(&url);
    }

    // Spawned rather than awaited: a window needs the main thread, and a run without one waits on
    // the handle instead, which is the same thing one line later.
    let asked = stop.clone();
    let serving = tokio::spawn(server.serve(async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = asked.asked() => {}
        }
        tracing::info!("stopping");
    }));

    #[cfg(feature = "desktop")]
    if will_have_a_tray(&shell) {
        return Ok(Started::Shell {
            window: will_have_a_window(&shell, &cli),
            url,
            stop,
            serving,
        });
    }

    Ok(Started::Serving { serving })
}

/// Starts the tracing subscriber, with a file beside the console when one was asked for.
///
/// The same shape `km-app`'s `init_logging` has, and the same three properties: the console layer
/// prints exactly what it printed before this existed; the file layer is never colored and always
/// timestamped, because nothing else will stamp it; and a file that will not open is warned about
/// rather than fatal — an offline remote whose log cannot be written is still a working remote.
///
/// `data_dir` is `None` only when the platform named no data directory, which is the case [`run`]
/// is about to fail on anyway. Asking for a log file there is answered with the warning rather than
/// with silence, because "I passed `--log-file` and got no file" deserves a reason.
fn init_logging(cli: &Cli, data_dir: Option<&PathBuf>) -> Option<km_logfile::LogFile> {
    let wanted = km_logfile::asked_for(cli.log_file);
    let dir = data_dir.map(|dir| dir.join(km_logfile::SUBDIR));
    let (file, failure) = match dir
        .as_ref()
        .filter(|_| wanted)
        .map(|dir| km_logfile::LogFile::open(dir, "km-remote"))
    {
        Some(Ok(file)) => (Some(file), None),
        Some(Err(error)) => (None, Some(error.to_string())),
        None if wanted => (None, Some("this platform has no data directory".to_owned())),
        None => (None, None),
    };

    // **Two sources, narrowest first.** There is no settings file here to be the third rung the
    // machine has, and the variable is what a checkout sets once for every program in it.
    let (viewer, viewer_failure) = match km_ecapplog::asked_for(cli.ecapplog.as_deref()) {
        Ok(asked) => (asked, None),
        Err(reason) => (None, Some(reason)),
    };
    let viewer = viewer.map(|address| km_ecapplog::EcAppLog::open(&address, VIEWER_NAME));

    tracing_subscriber::registry()
        .with(EnvFilter::new(cli.log_filter()))
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
    if let Some(reason) = failure {
        tracing::warn!(
            reason,
            "could not open a log file; the log goes to the console only"
        );
    }
    if let Some(file) = &file {
        tracing::info!(path = %file.path().display(), "writing this run's log here");
    }
    file
}

/// Where this phone's copy lives.
///
/// The platform's own data folder, so the collection survives the binary being replaced — and so it
/// is somewhere a person could find it. `--data-dir` is what a second instance uses, and what a test
/// run uses to stay out of the real one.
fn resolve_data_dir(asked_for: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = asked_for {
        return Ok(dir);
    }
    let dirs = directories::ProjectDirs::from("", "", "km-remote")
        .context("this platform has no data directory; pass --data-dir")?;
    Ok(dirs.data_dir().to_path_buf())
}

/// The aligned block a person reads once and then ignores.
///
/// The same shape `km-package-builder` prints, and it answers the same four questions: where the data is,
/// which machine, how much of it there is, and what to open.
/// Every line goes through [`km_console::say`], and that is the difference between a flag that
/// prints nowhere and a process that aborts: on a GUI-subsystem executable that was double-clicked
/// the standard output handle is null, and `std::io::_print` *panics* on a failed write. `say`
/// prints where there is somewhere to print and logs where there is not.
fn print_banner(ready: &Ready, lan: bool, log_file: Option<&km_logfile::LogFile>) {
    say("km-remote");
    say(format!("  data       {}", ready.data_dir.display()));
    // Only when there is one. A line saying "off" belongs in a report somebody asked for -- which is
    // what `--show-paths` is on the machine -- and not in the four lines this prints every start.
    if let Some(file) = log_file {
        say(format!("  log        {}", file.path().display()));
    }
    // The one line this program says about a destination it is no longer printing to. Said here
    // rather than where the connection opens, which is before this process has settled whether it
    // has a console at all.
    if let Some(address) = km_ecapplog::address() {
        say(format!("  log        ECAppLog at {address}"));
    }
    match &ready.machine {
        Some(found) => say(format!(
            "  machine    {} ({})",
            found.url,
            found.how.label()
        )),
        None => say("  machine    none found — browsing still works"),
    }
    match &ready.songs {
        Ok(0) => say("  songs      none copied yet"),
        Ok(count) => say(format!("  songs      {count}")),
        Err(error) => say(format!("  songs      unreadable: {error}")),
    }
    say(format!("  open       {}", ready.url));
    if lan {
        // The machine's own remote is deliberately open to a home LAN; this one is not, because it
        // holds somebody's personal collection. Saying so is the least that is owed to anybody who
        // passes the flag without thinking about it.
        say(format!(
            "  WARNING    serving on every interface (port {}) — your favorites are",
            ready.address.port()
        ));
        say("             readable by anything on this network");
    }
}

/// Opens the remote in whatever the platform calls a browser.
///
/// Best effort — a machine with no browser is a normal thing and no reason to refuse to start — but
/// **said out loud, which is what its two siblings do**: `km-package-builder` and `km-admin` both
/// print the failure beside the address. A `tracing::debug!` here reached nobody, the default level
/// being `info`, and the run it goes silent on is the one where it matters: a Linux box with no
/// `xdg-open` opens nothing, says nothing, and leaves a line reading `open <url>` that looks like a
/// report of something that happened. Measured on a Linux session carrying no `xdg-utils`.
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
    use clap::Parser;

    #[test]
    fn the_command_the_readme_tells_people_to_run_works() {
        let cli = Cli::parse_from(["km-remote"]);
        assert_eq!(cli.port, 8179);
        assert!(!cli.lan);
        assert!(cli.machine.is_none());
        assert!(!cli.browser);
    }

    /// The viewer is off unless asked for, and naming one takes an `=`.
    ///
    /// The space form is what `require_equals` refuses, and it is refused here rather than merely
    /// discouraged: this program takes no positional argument today, and a flag whose spelling
    /// depends on whether one has been added yet is a flag that breaks when one is.
    #[test]
    fn the_viewer_is_off_unless_asked_for() {
        assert_eq!(Cli::parse_from(["km-remote"]).ecapplog, None);
        assert_eq!(
            Cli::parse_from(["km-remote", "--ecapplog"]).ecapplog,
            Some(km_ecapplog::DEFAULT_ADDRESS.to_owned())
        );
        assert_eq!(
            Cli::parse_from(["km-remote", "--ecapplog=1.2.3.4:99"]).ecapplog,
            Some("1.2.3.4:99".to_owned())
        );
        assert!(Cli::try_parse_from(["km-remote", "--ecapplog=nope"]).is_err());
        assert!(Cli::try_parse_from(["km-remote", "--ecapplog", "1.2.3.4:99"]).is_err());
    }

    /// The port `clap` defaults to is the one the library defines, not a second copy of the number.
    #[test]
    fn the_default_port_is_the_librarys_and_not_a_second_copy_of_it() {
        assert_eq!(
            Cli::parse_from(["km-remote"]).port,
            km_remote_core::DEFAULT_PORT
        );
    }

    /// What each combination of build, executable and flag decides.
    ///
    /// `(window, browser)` for each. The pair matters more than either half: the bug this replaced
    /// in `km-package-builder` was not that a window failed to open but that a window **and** a
    /// browser tab both did, the two decisions having been made a hundred lines apart with neither
    /// aware of the other.
    #[cfg(feature = "desktop")]
    #[test]
    fn a_window_is_shown_instead_of_a_browser_and_never_as_well_as_one() {
        let decide = |args: &[&str], shell: Shell, nowhere: bool| {
            let cli = Cli::parse_from(std::iter::once("km-remote").chain(args.iter().copied()));
            (
                will_have_a_window(&shell, &cli),
                will_open_a_browser(&shell, &cli, nowhere),
            )
        };

        // Double-clicked: nobody can read a printed address, and the window is what shows the page.
        assert_eq!(decide(&[], Shell::Windowed, true), (true, false));
        // From a terminal: the same, because the window is not conditional on being unable to print.
        assert_eq!(decide(&[], Shell::Windowed, false), (true, false));
        // `--open` asks to be shown the page, and the window is the page being shown.
        assert_eq!(decide(&["--open"], Shell::Windowed, false), (true, false));

        // `--browser` declines the window. On its own that is a run with nothing asking for a tab.
        assert_eq!(
            decide(&["--browser"], Shell::Windowed, false),
            (false, false)
        );
        // ...and with `--open`, a tab instead of a window, which is what the flag is for.
        assert_eq!(
            decide(&["--browser", "--open"], Shell::Windowed, false),
            (false, true)
        );
        // Declined and double-clicked: a tab, because otherwise nothing at all is shown.
        assert_eq!(decide(&["--browser"], Shell::Windowed, true), (false, true));

        // `--lan` declines it too. Serving one person's favorites to the whole network *and*
        // opening a personal window on this desk would be two readings of one flag.
        assert_eq!(decide(&["--lan"], Shell::Windowed, false), (false, false));

        // The console twin never has a window, whatever is passed.
        assert_eq!(decide(&[], Shell::Console, false), (false, false));
        assert_eq!(decide(&["--open"], Shell::Console, false), (false, true));
        // Nowhere to talk *and* the twin: rare rather than impossible, and not what a double-click
        // looks like — the twin keeps the console it is handed, so double-clicking it prints the
        // address instead of opening a tab and vanishing. What reaches this row is a twin started
        // with no console at all and its output going nowhere.
        assert_eq!(decide(&[], Shell::Console, true), (false, true));
    }

    /// The icon in the bar belongs to the executable, not to the window.
    ///
    /// The pairing is what this pins: every row where `will_have_a_window` is false but a tray is
    /// still wanted is a run that would otherwise have nothing at all to show for itself, and those
    /// rows are the reason the feature exists. A test that only checked the windowed case would
    /// pass against an implementation that got exactly the wrong half.
    #[cfg(feature = "desktop")]
    #[test]
    fn the_icon_in_the_bar_does_not_depend_on_there_being_a_window() {
        let decide = |args: &[&str], shell: Shell| {
            let cli = Cli::parse_from(std::iter::once("km-remote").chain(args.iter().copied()));
            (will_have_a_tray(&shell), will_have_a_window(&shell, &cli))
        };

        // A window and an icon beside it.
        assert_eq!(decide(&[], Shell::Windowed), (true, true));
        // No window, and therefore the case the icon is for.
        assert_eq!(decide(&["--browser"], Shell::Windowed), (true, false));
        assert_eq!(decide(&["--lan"], Shell::Windowed), (true, false));
        // The console twin has a console and a Ctrl-C; it needs neither.
        assert_eq!(decide(&[], Shell::Console), (false, false));
        assert_eq!(decide(&["--browser"], Shell::Console), (false, false));
    }

    /// Without the feature there is no bar to put anything in, whatever is asked for.
    ///
    /// **This is the one that runs in CI**, which never turns the feature on, and it is the whole
    /// Linux story besides — the same bargain the browser test below makes.
    #[cfg(not(feature = "desktop"))]
    #[test]
    fn a_build_with_no_window_has_no_icon_in_the_bar_either() {
        assert!(!will_have_a_tray(&Shell::Windowed));
        assert!(!will_have_a_tray(&Shell::Console));
    }

    /// A build with no window still opens a browser for a double-click.
    ///
    /// **This is the one that runs in CI**, which never turns the feature on — and it is the whole
    /// Linux story besides. Without it the `desktop`-shaped logic above would be unexercised
    /// everywhere except a developer's Windows machine.
    #[cfg(not(feature = "desktop"))]
    #[test]
    fn a_build_with_no_window_still_opens_a_browser_for_a_double_click() {
        let decide = |args: &[&str], shell: Shell, nowhere: bool| {
            let cli = Cli::parse_from(std::iter::once("km-remote").chain(args.iter().copied()));
            (
                will_have_a_window(&shell, &cli),
                will_open_a_browser(&shell, &cli, nowhere),
            )
        };

        // There is no window to have, even asking for the windowed shell.
        assert_eq!(decide(&[], Shell::Windowed, false), (false, false));
        // So a double-click must open a tab, or the run shows nothing at all.
        assert_eq!(decide(&[], Shell::Windowed, true), (false, true));
        assert_eq!(decide(&["--open"], Shell::Windowed, false), (false, true));
        // `--browser` is accepted and changes nothing, which is why it is accepted everywhere.
        assert_eq!(
            decide(&["--browser", "--open"], Shell::Windowed, false),
            (false, true)
        );
    }

    #[test]
    fn every_documented_flag_parses() {
        let cli = Cli::parse_from([
            "km-remote",
            "--machine",
            "192.168.1.5",
            "--data-dir",
            "./scratch",
            "--port",
            "9000",
            "--lan",
            "--open",
            "--browser",
            "--refresh",
        ]);
        assert_eq!(cli.machine.as_deref(), Some("192.168.1.5"));
        assert_eq!(
            cli.data_dir.as_deref(),
            Some(std::path::Path::new("./scratch"))
        );
        assert_eq!(cli.port, 9000);
        assert!(cli.lan);
        assert!(cli.open);
        assert!(cli.browser);
        assert!(cli.refresh);
    }

    /// The ladder, without touching the process environment.
    ///
    /// `RUST_LOG` is deliberately not exercised here, for the reason `km-app`'s twin of this test
    /// gives: it is read from the environment every other test in the binary shares, so setting it
    /// would be a race rather than a test. What this pins is the default rung, which is the one a
    /// shipped build gets.
    #[test]
    fn verbosity_chooses_a_filter() {
        if std::env::var_os("RUST_LOG").is_some() {
            // Whoever is running the suite asked for something; the arms below cannot be reached.
            return;
        }
        let filter = |verbose: u8| {
            Cli::parse_from(
                std::iter::once("km-remote".to_owned())
                    .chain((0..verbose).map(|_| "-v".to_owned())),
            )
            .log_filter()
        };
        // The one that matters: a shipped build says nothing at `debug` unless asked.
        assert_eq!(filter(0), "info");
        // And all three crates come up together — between them `km_remote_core` and `km_remote_pages` are
        // almost everything this program has to say about itself, so raising only the shell would
        // make `-v` quieter than the old default in a way nobody asked for.
        assert_eq!(
            filter(1),
            "info,km_remote=debug,km_remote_core=debug,km_remote_pages=debug"
        );
        assert_eq!(
            filter(2),
            "debug,km_remote=trace,km_remote_core=trace,km_remote_pages=trace"
        );
        // Beyond the ladder is the top of it, not a panic.
        assert_eq!(
            filter(5),
            "debug,km_remote=trace,km_remote_core=trace,km_remote_pages=trace"
        );
    }

    #[test]
    fn an_explicit_data_dir_wins_over_the_platforms() {
        let dir = resolve_data_dir(Some(PathBuf::from("./scratch"))).expect("resolve");
        assert_eq!(dir, PathBuf::from("./scratch"));
    }
}
