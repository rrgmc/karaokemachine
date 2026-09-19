//! Browse a folder of karaoke files and hand-curate them into `.kmpkg` packages.
//!
//! `km-pack build` scanning a folder is a bootstrap. Real packages are chosen by hand, and choosing a
//! few thousand songs out of hundreds of thousands of files is an interface problem rather than a
//! command-line one: the titles are truncated filenames, the artists are mostly missing, a good tenth
//! of the files are duplicates of each other, and about a third of the lyric encodings were a guess.
//! This is the tool for that — a local web server, pointed at the folder, with its own database
//! beside the files.
//!
//! It never writes to the source files. It reads them, records what it found, remembers what a person
//! decided, and writes packages.
//!
//! ```sh
//! km-package-builder <root> --init   # create the database and serve
//! km-package-builder <root>          # serve; refuses if there is no database here
//! ```
//!
//! # Why this is a library with two binaries hanging off it
//!
//! On Windows the windowed build is a **GUI-subsystem** executable, so that double-clicking it opens
//! a window and nothing else — no console flashes past on the way. The price of that subsystem is
//! that `--help` and `--version` print nowhere when a person runs it from a terminal, and the price
//! is paid by shipping a second, console-subsystem executable beside it:
//! `km-package-builder-console`, which never opens a window and is the browser-served tool this has
//! always been. See the `Two executables on Windows` decision in `docs/decisions/`.
//!
//! `#![windows_subsystem]` is a property of a *binary crate root*, so the two cannot be one file —
//! and a second crate root can see nothing of this module tree. Hence the library: everything is
//! here, `src/main.rs` and `src/bin/km-package-builder-console.rs` are three lines each, and the only
//! thing they disagree about is the [`Shell`] they pass to [`run`].

/// Talking to the running karaoke machine.
///
/// **The one module here that is `pub`**, and only so that `tests/upload.rs` can drive
/// [`app::Client`] against a real listening machine. That test exists because the seam it covers is
/// invisible from either side: whether the multipart body this writes is the one `km-api` reads. An
/// integration test is a separate crate and cannot see a private module, and the alternative — a
/// `#[cfg(test)]` unit test — could not bind a socket without dragging a server into the library.
pub mod app;
/// What a person typed, in a file they can keep somewhere else. See the module's own note for what
/// is in one and what deliberately is not.
mod backup;
mod browse;
mod build;
mod casing;
/// Which machine this tool would install into, and where that fact is kept. See the module note.
mod chosen;
mod db;
/// The window, when this build has one. See the module's own note for the three surprises in it.
#[cfg(feature = "desktop")]
mod desktop;
mod dupes;
mod fixes;
mod form;
mod handlers;
mod hint;
mod lyric_likeness;
mod model;
mod names;
/// The machine passwords this computer was told to remember. See the module note for why they are
/// not in the curated folder.
mod passwords;
mod recent;
mod register;
mod scan;
mod server;
mod settings;
mod similar;
/// The checklist of steps the Scan page and the Open page both draw. See the module note.
mod step;
/// What the tests need and the tool does not — the scratch directory, in one copy.
#[cfg(test)]
mod testing;
mod version;
mod views;
mod words;
mod workspace;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::db::{DATABASE_NAME, Db};
use crate::server::{DEFAULT_APP_URL, State, router};
use km_console::say;

/// What this program is called **to a person reading it with room**, as against
/// `km-package-builder`, which is what it is called to cargo, to a shell and to a package manager.
///
/// Every page reads this, and so do the window title, the macOS app menu and the tray tooltip —
/// each of which is read one at a time. The console banner, `--help` and the "could not reach …"
/// toast deliberately do *not* read it: there the crate name is the thing somebody typed and the
/// thing they would grep for.
///
/// **[`APP_NAME_SHORT`] is the other half and the two are not interchangeable** — see
/// `What the tool calls itself` in docs/decisions/. An icon in a list of four siblings gets nine
/// characters; a title bar gets the sentence.
///
/// **Two files spell the *short* name out and cannot see a Rust constant**:
/// `tools/platform/macos/Info.package-builder.plist` and `tools/dist/cmd.sh`. They are named here
/// so a rename knows where to go, which is the only thing that has ever kept them in step.
pub const APP_NAME: &str = "KaraokeMachine Package Builder";

/// What this program is called **on an icon**, where nine characters is the whole budget.
///
/// **The launcher, the bundle and the installer, and nothing else.** A Dock, a Start menu, a
/// component list and an `.app` folder read the four siblings side by side and truncate them, and
/// four entries all beginning `KaraokeMachine ` differ only past the width the list will give them.
/// A title bar, a menu bar and a tooltip read one, with room, and get [`APP_NAME`].
///
/// Only [`register::linux`]'s `.desktop` entry and the two error sentences naming
/// `KM Package Builder.app` — a folder on disk — read this from Rust. The rest of the short name's
/// homes cannot see a constant and are listed on [`APP_NAME`].
///
/// The pair is named the way `ports/remote/android/…/values/strings.xml` names it, `app_name`
/// beside `app_name_short` — the platform that states this rule in two files.
pub const APP_NAME_SHORT: &str = "KM Package Builder";

/// Which build this is, for the header chip.
///
/// **`env!` rather than a field on [`views::Chrome`]**, because it is a compile-time constant and a
/// field would be one more thing every constructor and every test fixture has to keep saying — the
/// same reason `APP_NAME` above is reached from the template rather than carried on the view.
///
/// It is the whole repository's version: `version.workspace = true` here and in
/// `tools/cmd/assets/Cargo.toml`'s separate workspace, held equal by
/// `tools/dev/check-version-pin.sh`. So this and the machine it sends songs to report the same
/// number when they are the same build, which is the comparison somebody makes when Install does
/// something they did not expect.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The port this serves on when nothing says otherwise.
///
/// One past the machine's own 8177, and one before the offline remote's 8179 and `km-admin`'s 8180.
/// Four programs in this product can be running at once on one desk, and the ports being adjacent is
/// what makes "which one is that?" answerable.
///
/// A constant rather than a literal on the `--port` attribute, because the other three ports in that
/// ladder are constants — [`km_api::connect::DEFAULT_PORT`], `km_remote_core::DEFAULT_PORT` and
/// `km_admin::DEFAULT_PORT` — and this one was the odd one out, spelled once in the attribute and
/// again, in prose, in the doc comment beside it.
pub const DEFAULT_PORT: u16 = 8178;

/// The curation tool's command line.
#[derive(Parser)]
#[command(
    name = "km-package-builder",
    about = "Browse a folder of karaoke files and curate them into packages",
    version
)]
struct Cli {
    // Optional so that a double-clicked desktop icon, which carries no arguments, lands on the Open
    // page. Giving the `.kmbuild` file is how the operating system invokes this when somebody
    // double-clicks a corpus.
    /// The folder of karaoke files to curate, or its `.kmbuild` file.
    ///
    /// Without this, the tool opens a list of folders you have curated before.
    root: Option<PathBuf>,

    /// Create the database in this folder if it does not exist.
    ///
    /// Required the first time you open a folder. Without it, pointing at the wrong folder reports
    /// an error instead of showing an empty index.
    #[arg(long)]
    init: bool,

    /// Port to listen on. One past the karaoke app's own 8177.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Listen on every network interface rather than loopback only.
    ///
    /// Off by default. There is no password on this tool, and it can read files and write packages
    /// anywhere the user running it can.
    #[arg(long)]
    lan: bool,

    // `--machine` is what `km-remote` and `km-admin` call the same thing, and as in `km-admin` it
    // holds for the run and is not written down — see `State::pin_machine`.
    /// The karaoke machine to test-play songs on and install packages to.
    ///
    /// Used for this run only. It is not saved, and it does not change the machine this folder or
    /// this computer normally uses. Choose one in Settings to keep it.
    #[arg(long)]
    machine: Option<String>,

    /// Start a scan as soon as the server is up.
    #[arg(long)]
    scan: bool,

    /// Save everything you have edited in this folder to a JSON file, then exit.
    ///
    /// Includes titles, artists, languages, encodings, transpositions, the rating, notes, merges
    /// and favorites — everything a re-scan cannot rebuild. Songs are recorded by content hash, so
    /// the file restores onto a rebuilt index or onto another computer.
    ///
    /// Save it somewhere other than the corpus folder. Packages and duplicate verdicts are not
    /// included; use Import on the Packages page for a built package.
    #[arg(long, value_name = "PATH", conflicts_with = "init")]
    backup: Option<PathBuf>,

    /// Restore a backup into this folder's database, then exit.
    ///
    /// Fills in blank fields only, unless you also give `--overwrite`. Nothing is ever cleared.
    ///
    /// Scan the folder first. Songs are matched by content hash, so a song this folder has not
    /// indexed cannot be matched and is listed instead.
    #[arg(long, value_name = "PATH", conflicts_with = "init")]
    restore: Option<PathBuf>,

    /// With `--restore`, replace existing values instead of only filling in blanks.
    #[arg(long, requires = "restore")]
    overwrite: bool,

    /// Read every song again and update what the analysis says, then exit.
    ///
    /// On Windows this is the console program's to run: km-package-builder-console <folder>
    /// --reanalyze. The one with a window hands the prompt back before the work starts, so there
    /// would be nothing to watch and nothing to wait for.
    ///
    /// What to run after an update changes how suitability is worked out. A suitability comes from
    /// the notes, the channels and the lyric timings, and this folder keeps the answer rather than
    /// what it was worked out from, so the songs have to be read again.
    ///
    /// Much quicker than Re-analyze everything on the Scan page: one copy of each song rather than
    /// every duplicate of it, and none of the passes that ask about the whole folder at once. It
    /// finds no songs added since the last scan. Use --scan for those.
    ///
    /// Titles, artists, languages, encodings, transpositions, fixes, chosen melodies, ratings and
    /// notes are yours, and are left exactly as they are.
    #[arg(long, conflicts_with_all = ["init", "backup", "restore", "scan"])]
    reanalyze: bool,

    // Already satisfied by a window, where this build has one: the flag asks to be shown the page,
    // and a window is the page being shown.
    /// Open a browser at the address once the server is listening.
    ///
    /// In a build with its own window, add `--browser` to open a browser tab instead.
    #[arg(long)]
    open: bool,

    /// Show the page in your own browser rather than in a window of its own.
    ///
    /// Only a build with the `desktop` feature has a window to decline — every Linux build, any build
    /// made with `--no-desktop`, and `km-package-builder-console` behave this way whatever is passed.
    /// Accepted everywhere so that a script does not have to know which build it is talking to.
    #[arg(long)]
    browser: bool,

    /// Start on the Open page rather than reopening the folder from last time.
    ///
    /// Only meaningful when no folder is named, which is the case that reopens one. Wanted when the
    /// next corpus is a different one, or when the last one is on a drive that is not plugged in and
    /// waiting for it to fail is slower than saying so.
    #[arg(long)]
    pick: bool,

    /// Make `.kmbuild` files open with this executable, then exit.
    ///
    /// Per-user and needs no elevation. Records wherever this executable is *now*, so a portable
    /// folder registers correctly from whatever directory it was unpacked into — and has to be run
    /// again if the folder moves.
    #[arg(long, conflicts_with = "unregister")]
    register: bool,

    /// Undo `--register`, then exit.
    #[arg(long)]
    unregister: bool,

    /// Also write this run's log to a file in this tool's own data folder.
    ///
    /// The console keeps exactly what it prints without this; the file gets the same stream, never
    /// colored and always timestamped. **The case it exists for is the double-click**: with the
    /// `desktop` feature `km-package-builder.exe` is GUI-subsystem, so it has a window and no
    /// console and every line it writes is discarded — and this is the tool most likely to be
    /// started by double-clicking a `.kmbuild` file rather than by typing its name. One file per
    /// run, the ten newest kept, in the folder the banner names.
    ///
    /// **Not in the corpus folder**, which is where the database lives: the tool can be started
    /// with no folder at all, and a scan that fails to open one still has something to say.
    ///
    /// `KM_LOG_FILE=1` does the same, for a shortcut that has nowhere to put a flag.
    #[arg(long)]
    log_file: bool,

    // The destination somebody watches while a scan runs, where the file above is the one they go
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
    ecapplog: Option<String>,

    /// Say more. Repeat for more still: `-v` for this tool's own detail, `-vv` for everything.
    ///
    /// `RUST_LOG` overrides it entirely when set, which is how you ask for something this ladder
    /// does not offer — one noisy dependency, say. `logging.level` in this tool's settings.json
    /// says the same thing for a run started from an icon rather than a command line.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

impl Cli {
    /// The tracing filter this verbosity asks for.
    ///
    /// **The default is plain `info`.** A default of `info,km_package_builder=debug` makes every
    /// shipped build emit this tool's whole debug stream — the mistake the `What a shipped build
    /// says out loud` decision is written about. Debug output is a developer's, and a release that
    /// produces it by default has decided on the owner's behalf that they wanted it.
    ///
    /// **[`QUIET_DEPENDENCIES`] does more of the work here than the rung does.** This
    /// crate's own debug stream is ten sites on edge paths, so dropping it changes an ordinary run
    /// very little; `symphonia` talking at `info` on every MP3+G pair opened is what actually
    /// buried a scan, and `info` is a level no verbosity change was ever going to reach.
    ///
    /// **`logging.level` is the rung below `-v` and speaks `RUST_LOG`'s grammar**, for the reason
    /// the machine's ladder gives: a tool opened from its icon has no command line to type a flag
    /// on. It replaces the ladder rather than moving along it, exactly as `RUST_LOG` does, and `-v`
    /// beats it so one run is never an edit to the box.
    ///
    /// The `RUST_LOG` check is spelled out rather than deferred to
    /// `EnvFilter::try_from_default_env`, matching `km-app`: the two differ on a malformed filter,
    /// and one behavior across every shipped binary is what this is for.
    fn log_filter(&self, settings: &km_logsettings::LoggingSettings) -> String {
        if let Ok(filter) = std::env::var("RUST_LOG") {
            return filter;
        }
        if self.verbose == 0
            && let Some(level) = settings.level()
        {
            return level.to_owned();
        }
        match self.verbose {
            0 => format!("info,{QUIET_DEPENDENCIES}"),
            1 => format!("info,{SELF_TARGET}=debug,{QUIET_DEPENDENCIES}"),
            _ => format!("debug,{SELF_TARGET}=trace"),
        }
    }
}

/// This library's own tracing target, asked of cargo rather than typed out.
///
/// `km_package_builder`, which is the *library* name and not the package's — a tracing target is a
/// module path, rooted at the library. Derived rather than spelled, because a filter naming a
/// target that does not exist is an error nowhere: `-v` simply stops printing, and nothing says
/// why. The test below still pins the literal, so a rename fails loudly in one place.
const SELF_TARGET: &str = env!("CARGO_CRATE_NAME");

/// Dependencies that speak once per file, held down to errors.
///
/// `symphonia`'s MP3 demuxer announces itself on every file it opens: "estimating duration from
/// bitrate" at `info`, and a lame tag whose CRC disagrees at `warn`. It reaches this binary through
/// `km-cdg`, which is how an MP3+G pair is read, so a scan of a corpus emits a line per pair and
/// buries everything this tool actually has to say. It also lands in the middle of the meter, which
/// rewrites a single line and has no way to know something else wrote to the terminal.
///
/// **Errors, where the machine's copy of this list stops at warnings, and the difference is how many
/// files each opens.** That one opens a file when somebody sings, so a warning is one line and
/// belongs in the journal. This one opens every file in a folder, and how a particular file spells
/// its own tags is not a thing the person running a scan can act on. What they can act on is recorded
/// properly rather than logged: a file that will not read gets a `scan_status` and a place on the
/// failures list, which is this tool's own channel for exactly that and does not depend on a
/// dependency choosing to mention it.
///
/// Duplicated rather than shared: three binaries with no common dependency (`km-package-builder`
/// does not depend on `km-remote-pages`, and neither depends on `km-app`) is not worth a workspace member
/// to hold ten lines. See `crates/machine/karaokemachine/src/main.rs` for the other.
const QUIET_DEPENDENCIES: &str = "symphonia_bundle_mp3=error,symphonia_core=error";

/// The folder a path refers to: itself, or the one holding the `.kmbuild` file it names.
///
/// Both shapes reach this. A person types a folder; an operating system hands over the *document*,
/// because that is what was double-clicked — as `argv` on Windows and Linux, and as an Apple Event
/// on macOS, which lands here through `desktop::run` instead. Resolving it in one place means
/// everything past this point sees the folder it always saw.
///
/// `None` when the path is neither, which is a typo and is reported as one.
fn folder_of(given: &Path) -> Option<PathBuf> {
    if given.is_dir() {
        return Some(crate::model::tidy(given));
    }
    let names_a_database = given
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(db::DATABASE_EXTENSION));
    if given.is_file() && names_a_database {
        return given.parent().map(crate::model::tidy);
    }
    None
}

/// The folder to reopen when none was named, or `None` to start on the picker.
///
/// The newest remembered folder, if it is still a folder and still holds a database this build can
/// open. An older `.sqlite` deliberately does **not** qualify: adopting it is a rename of somebody's
/// whole index and is not a thing to do to them because they double-clicked an icon. The picker
/// offers it with a button and a sentence.
fn folder_to_reopen() -> Option<PathBuf> {
    let recent = crate::recent::Recent::load();
    let newest = recent.folders.first()?;
    if crate::browse::indexed(&newest.path) == crate::browse::Indexed::Yes {
        Some(newest.path.clone())
    } else {
        None
    }
}

/// Which of the two executables is running.
///
/// **Not a flag**, and deliberately not one: a person does not choose this, they choose which icon to
/// double-click or which name to type. `--browser` is the flag that declines a window, and it goes on
/// meaning exactly what it always did — this is the *build* saying whether it has one to decline.
pub enum Shell {
    /// `km-package-builder` — a window of its own, where the build has one.
    Windowed,
    /// `km-package-builder-console` — never a window; the browser-served tool this has always been.
    ///
    /// The console twin is compiled from the same library and could open a window as easily as the
    /// other one; what it must not do is *want* to, because it exists so that somebody at a terminal
    /// has a program that talks back to them.
    Console,
}

/// Whether this run will put the page in a window of its own.
///
/// **A window is asked for by the build rather than by the user.** `--browser` is how you decline it,
/// `--lan` declines it too because a webview on a machine nobody is sitting at is pointless, and the
/// console twin never has one to begin with — that is what it is for.
fn will_have_a_window(shell: &Shell, cli: &Cli) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(feature = "desktop") && !cli.browser && !cli.lan
}

/// Whether this executable detaches from the shell that started it.
///
/// **A GUI-subsystem image is one the command processor does not wait for.** `main.rs` asks for that
/// subsystem wherever there is a window to open, which is what keeps a console from flashing past a
/// double-click — and it is also what makes `cmd` return to its prompt the instant the program is
/// typed, with the run carrying on behind it. A second of that is a backup nobody noticed finishing.
/// Twenty minutes of it is a job reporting into a prompt that has moved on, with no status for a
/// script to read.
///
/// **It says nothing about whether a window opens.** `--browser` and `--lan` decline one and the
/// subsystem is unmoved, because that is fixed when the executable is linked. The console twin is
/// the same library without the attribute, and is the one a person or a script can wait for.
fn detaches_from_the_shell(shell: &Shell) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(all(windows, feature = "desktop"))
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
/// under the feature, hence the allowance: [`will_have_a_window`] escapes the same fate only
/// because [`will_open_a_browser`] happens to call it.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn will_have_a_tray(shell: &Shell) -> bool {
    matches!(shell, Shell::Windowed) && cfg!(feature = "desktop")
}

/// Whether to hand the address to a browser once the server is listening.
///
/// **`--open` is implied when nobody can read the address.** Double-clicked, there is no console for
/// the URL to have been printed to, so a build that waited to be asked would be a process that starts,
/// serves, and shows nothing at all — indistinguishable from one that failed to start.
///
/// **Unless a window is about to show the page, and that clause is the whole of the fix.** This used
/// to be decided beside the printing and the window a hundred lines further down, neither knowing
/// about the other, so a double-clicked windowed build opened the window *and* a browser tab on the
/// same URL. An explicit `--open` reached it by the shorter road and did the same.
///
/// A window is the page being shown, so it satisfies `--open` rather than competing with it, and
/// `--browser` is how you ask for a tab **instead of** a window. `nowhere_to_talk` is passed in
/// rather than read from [`km_console`] so that this is a function of its arguments and can be asserted.
fn will_open_a_browser(shell: &Shell, cli: &Cli, nowhere_to_talk: bool) -> bool {
    !will_have_a_window(shell, cli) && (cli.open || nowhere_to_talk)
}

/// Whether a folder named on the command line is opened before the server answers, or handed to the
/// Open page as a job.
///
/// **What decides it is where a refusal would be read.** A folder that will not open is the ordinary
/// way a double-click fails — a database written by a newer build, two of them in one folder, an
/// index that has gone — and an eager open reports it by returning out of `main`. In a window that is
/// nothing at all: no console, null handles, and a process that exits while the screen does not
/// change. Opened as a job, the same sentence lands on the Open page in red with the chooser beside
/// it, which is the answer [`State::begin_open`] has always given the folder it reopens.
///
/// **A window is not enough on its own, and `nowhere_to_talk` is the clause that makes this a rule
/// rather than a list.** Standard handles are inherited whatever the subsystem — which is the fact
/// [`km_console`] is written around — so `km-package-builder <folder>` typed at a prompt is a
/// windowed run with a perfectly good stderr, and somebody is waiting on both the sentence and the
/// exit status. Deferring there would take the status with it and a script would stop seeing
/// failures. What is left in the deferred set is the registered command, `"<exe>" "%1"`, and nothing
/// else.
///
/// **The flags that name a corpus they cannot act on until it is open keep the eager path**:
/// `--init` creates the database, `--scan` starts work on it, and `--backup`/`--restore`/`--reanalyze` are it. Each was typed. So is every console run, every
/// `--browser` and `--lan` run, and every build with no window to offer.
fn opens_eagerly(shell: &Shell, cli: &Cli, nowhere_to_talk: bool) -> bool {
    !will_have_a_window(shell, cli)
        || !nowhere_to_talk
        || cli.init
        || cli.scan
        || cli.backup.is_some()
        || cli.restore.is_some()
        || cli.reanalyze
}

/// The entry point, called by both binaries.
///
/// **Not `#[tokio::main]`, and the reason is the window.** That attribute builds a runtime and parks
/// the main thread inside `block_on` for the life of the process — which is fine for a server and
/// impossible for a desktop shell, because `tao`'s event loop must own the main thread and its `run`
/// never returns. So the runtime is built by hand, the server is *spawned* on it rather than awaited,
/// and what happens on the main thread afterwards depends on which shape was asked for: the event
/// loop, or a plain wait on the server.
///
/// The runtime is then handed to whichever of the two takes over, because dropping it would stop the
/// server they are both there to look at.
pub fn run(shell: Shell) -> Result<()> {
    // **The command line is read before the subscriber exists**, because the subscriber's level is
    // one of the things it decides. It is also the better order for the other two things clap can
    // do: `--help` and `--version` print and exit while the console is still attached and before
    // anything has been initialized on their behalf.
    let cli = Cli::parse();

    let log_file = init_logging(&cli);
    // Drains the viewer's queue on the way out of every exit this function has. The windowed shell
    // is the one it does not reach -- `tao`'s loop calls `process::exit` -- so `desktop.rs` drains
    // by name where it does the rest of its shutdown.
    let _drain = km_ecapplog::flush_on_drop();

    // Before the first line is printed, and that ordering is not stylistic: with no console,
    // `println!` panics rather than failing, so anything printed beforehand would take the process
    // with it. Everything below says its piece through `km_console::say`, which knows which of the two
    // sinks exists.
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

    // Read before `shell` is handed over, because what it decides is what happens if `start` fails.
    let windowed = matches!(shell, Shell::Windowed);

    let started = match runtime.block_on(start(cli, log_file, shell)) {
        Ok(started) => started,
        Err(error) => return Err(fatal(error, windowed)),
    };
    match started {
        // A desktop shell: the event loop takes the main thread and never gives it back. Whether
        // there is a window inside it is the `window` flag; either way there is an icon in the bar,
        // and an icon needs a loop to be delivered events on.
        #[cfg(feature = "desktop")]
        Started::Shell {
            window,
            state,
            url,
            serving,
        } => desktop::run(window, state, url, serving, runtime),
        Started::Serving { serving, state } => {
            runtime.block_on(serving)?;
            finish(&state);
            Ok(())
        }
        Started::Done => Ok(()),
    }
}

/// What a failure on the way up does with itself, before it is returned.
///
/// **Logged first, and unconditionally.** Returning an error out of `main` prints it through `std`'s
/// own `Termination`, which writes to a standard error handle a windowed run does not have — so a
/// startup failure was the one thing `--log-file` could not capture, in the build whose
/// documentation says the double-click is the case it exists for.
///
/// **Then shown, where there is nothing to read it.** [`km_console::nowhere_to_talk`] is the whole
/// test: it is true exactly when this process was launched with no console behind it, which is the
/// double-click and the Start Menu shortcut. A console twin, a terminal, and every build without a
/// window return the error the way they always have — each of those has somewhere the sentence
/// already goes.
///
/// The error comes back either way. Nothing here changes what the process exits with.
fn fatal(error: anyhow::Error, windowed: bool) -> anyhow::Error {
    tracing::error!("{error:#}");

    #[cfg(feature = "desktop")]
    if windowed && km_console::nowhere_to_talk() {
        desktop::show_error(&failure_html(&format!("{error:#}")));
    }
    // Which of the two executables this is matters only to the arm above, and that arm does not
    // exist in a build with no window. The workspace's lints refuse an unused binding, and the
    // parameter is worth keeping in both shapes so the signature does not move with the feature.
    let _ = windowed;
    error
}

/// The one-sentence page the failure window shows.
///
/// Self-contained, because the server that serves this tool's stylesheet is the thing that did not
/// start. Both themes are painted rather than inherited: a page naming no background borrows the
/// webview's, which is white on a desktop set to dark. `pre-wrap` keeps any newline a refusal writes
/// into its own message.
///
/// **Here rather than beside the window it is for**, for the reason [`will_have_a_tray`] gives:
/// `desktop.rs` is behind a feature `tools/setup/features.sh` deliberately never turns on, so a test
/// written there is a test that never runs. What is left in that module is the loop and the window,
/// which no test can assert anyway.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn failure_html(message: &str) -> String {
    format!(
        r#"<!doctype html>
<meta charset="utf-8">
<title>{title}</title>
<style>
  :root {{ color-scheme: light dark; }}
  body {{
    margin: 0; padding: 2rem;
    font: 15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif;
    background: #fbfbfb; color: #1c1c1c;
  }}
  h1 {{ margin: 0 0 0.75rem; font-size: 1.15rem; font-weight: 600; }}
  p {{ margin: 0; white-space: pre-wrap; }}
  @media (prefers-color-scheme: dark) {{
    body {{ background: #1e1e1e; color: #e8e8e8; }}
  }}
</style>
<h1>{heading}</h1>
<p>{message}</p>
"#,
        title = escape_html(APP_NAME),
        // The settings read directly, because this runs before there is a server to have read them
        // — and before there is a page to draw, which is why this window exists at all.
        heading = escape_html(
            &crate::words::messages(settings::Settings::load().locale().unwrap_or_default(),)
                .msg_with("window-could-not-start", &[("program", APP_NAME.into())])
        ),
        message = escape_html(message),
    )
}

/// The three characters that would otherwise end the element they are sitting in.
///
/// **The escaping is the reason [`failure_html`] is a function at all.** What arrives is an anyhow
/// chain carrying a path somebody chose, and a path is a string with `&` and `<` as available in it
/// as anywhere else. A replace rather than a template, because this page is built on the one path
/// where nothing else worked.
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What `start` decided to do, once the server is listening.
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
        /// The shared state, for the title and for the folder a document names.
        state: State,
        /// Where the page is: what to point a webview at, and what the tray offers to open.
        url: String,
        /// The server, so the loop can end when it does.
        ///
        /// **Not dropped here.** Dropping it detaches the task, which is harmless only if closing
        /// the window is the one way out of a run — and a windowless run has no window to close, so
        /// a Ctrl-C that stops the server has to be able to end the loop as well.
        serving: tokio::task::JoinHandle<()>,
    },
    /// Serve until something asks it to stop, with the page in a browser or nowhere.
    Serving {
        /// The server, spawned and running.
        serving: tokio::task::JoinHandle<()>,
        /// The shared state, for the shutdown.
        state: State,
    },
    /// The command did its one job and there is nothing to serve — `--register` and its opposite.
    Done,
}

/// Starts the tracing subscriber, with a file beside the console when one was asked for.
///
/// The same shape `km-app`'s and `km-remote`'s do, and the same three properties: the console layer
/// prints exactly what it printed before this existed; the file layer is never colored and always
/// timestamped, because nothing else will stamp it; and a file that will not open is warned about
/// rather than fatal — a curation tool whose log cannot be written still curates.
///
/// **`--ecapplog` is the one destination that displaces another**: the console layer is not built at
/// all when it is on, because a viewer offering a tab per crate and a level to filter on is a better
/// console than a console and two of them would print every line twice. The file is untouched. See
/// `A log that goes to a viewer instead of a console` in docs/decisions/.
fn init_logging(cli: &Cli) -> Option<km_logfile::LogFile> {
    // **Three sources, narrowest first**: what was typed, then what the environment was given, then
    // what this box was set up to do. A run says what this run does, and neither a variable nor a
    // settings file may override it.
    //
    // Naming a count is asking for a history, and a history nobody is writing is not one -- so a
    // `keep` in the file turns the log file on by itself, exactly as `KM_LOG_KEEP` does.
    let settings = crate::settings::peek_logging();
    let keep = km_logfile::keep_wanted(None).or_else(|| settings.keep());
    let wanted = km_logfile::asked_for(cli.log_file) || settings.file || keep.is_some();
    let keep = keep.unwrap_or(km_logfile::KEEP);
    let (file, failure) = match wanted.then(logs_dir).map(|dir| {
        dir.map(|dir| km_logfile::LogFile::open_keeping(dir, "km-package-builder", keep))
    }) {
        Some(Some(Ok(file))) => (Some(file), None),
        Some(Some(Err(error))) => (None, Some(error.to_string())),
        Some(None) => (None, Some("this platform has no data directory".to_owned())),
        None => (None, None),
    };

    // **Three sources, narrowest first**, the same ladder the file above climbs and for the same
    // reason: the variable is what a checkout sets once for every program in it, and the file is
    // what reaches a run started by double-clicking an icon.
    let (viewer, viewer_failure) = match km_ecapplog::asked_for(cli.ecapplog.as_deref()) {
        Ok(asked) => (asked.or_else(|| settings.ecapplog()), None),
        Err(reason) => (None, Some(reason)),
    };
    let viewer = viewer.map(|address| km_ecapplog::EcAppLog::open(&address, APP_NAME));

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
    if let Some(file) = &file {
        tracing::info!(path = %file.path().display(), "writing this run's log here");
    }
    file
}

/// Where `--log-file` writes, which is this tool's own per-user data folder.
///
/// **Not the corpus folder**, where the `.kmbuild` document lives, and that is the one decision in
/// here: this tool can be started with no folder at all, and the run that most wants a log is the
/// one that could not open the folder it was given.
///
/// The qualifier is the one [`crate::recent`] and `desktop::webview_data_dir` already use, so this
/// program has a single identity under `directories`.
fn logs_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "km-package-builder")?;
    Some(dirs.data_dir().join(km_logfile::SUBDIR))
}

/// `--backup` and `--restore`, which do their job and leave.
///
/// **Backup before restore when both are given**, and that ordering is the whole reason they are one
/// branch rather than two: it is what makes restoring the wrong file recoverable. Anybody who reaches
/// for `--restore` is already in the situation where being wrong twice would be expensive.
///
/// [`Db::open`] and never `Db::create`: restoring into a folder with no database has no songs to
/// rejoin to, and `require_database` above has already refused it with the right sentence — which
/// names `--init` and is the one this tool gives everywhere else.
///
/// Everything is reported through `say`, like the banner and the refusals, so a run launched by a
/// scheduled task with nothing reading its console does not panic trying to print.
fn backup_and_exit(cli: &Cli, root: Option<&Path>) -> Result<Started> {
    let Some(root) = root else {
        bail!(
            "--backup and --restore need the folder to work on:\n  \
             km-package-builder <folder> --backup <file>"
        );
    };
    let mut db = Db::open(root)?;

    if let Some(out) = &cli.backup {
        let counts = backup::write(&db, out)?;
        say(format!(
            "  backed up  {} song(s) and {} favorite(s) to {}",
            counts.songs,
            counts.favorites,
            out.display()
        ));
    }

    if let Some(path) = &cli.restore {
        let file = backup::Backup::read(path)?;
        if file.format > backup::FORMAT {
            say(format!(
                "  note       written by a later build (format {}); reading what this one \
                 understands",
                file.format
            ));
        }
        let policy = if cli.overwrite {
            backup::Policy::Overwrite
        } else {
            backup::Policy::FillBlanks
        };
        let report = backup::restore(&mut db, &file, policy)?;
        say(format!(
            "  restored   {} song(s), {} favorite(s), {} membership(s), {} merge(s)",
            report.songs_applied,
            report.favorites_created,
            report.memberships_applied,
            report.merges_applied
        ));
        if !report.songs_unmatched.is_empty() {
            say(format!(
                "  unmatched  {} song(s) are in the file and not in this folder -- scan first?",
                report.songs_unmatched.len()
            ));
        }
        for line in report.rejected.iter().take(10) {
            say(format!("  refused    {line}"));
        }
        if report.rejected.len() > 10 {
            say(format!(
                "  refused    ... and {} more",
                report.rejected.len() - 10
            ));
        }
    }

    Ok(Started::Done)
}

/// How often the run looks at where the scan has got to.
///
/// **Not how often it says so**, which is [`km_console::Meter`]'s own interval. Polling more often
/// than the meter speaks is what keeps the two independent: the meter is told a number that is at
/// most a second old whenever it decides to draw, rather than one that is as stale as the gap
/// between drawings. It is an atomic read against a background thread, so a second costs nothing.
const REANALYZE_POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// `--reanalyze`: every song read again, and what the analysis says written back.
///
/// **A scan of a named set of files, and the set is one path per song.** The copies of a song are
/// byte-identical by construction — the id *is* the hash — so a second copy would produce the same
/// answer for the same row at the cost of reading the file again. That is the whole of why this is
/// quicker than the Scan page's *Re-analyze everything*, which is a walk of every file there is.
///
/// **It re-reads rather than recomputing**, because there is nothing stored to recompute from: a
/// suitability is derived from notes, channels and lyric timings, and the database keeps the
/// conclusion rather than the evidence. Nothing here can be made to skip that.
///
/// Through [`crate::workspace::Workspace`] rather than calling the scanner directly, for the reason
/// `Workspace::start_scan` gives: it is the one place a scan is spawned, so this and the button on
/// the page cannot end up with different ideas. Dropping it joins the thread and closes the
/// database, which is the checkpoint a run that wrote hundreds of thousands of rows needs and a
/// backup never did.
async fn reanalyze_and_exit(root: Option<&Path>) -> Result<Started> {
    let Some(root) = root else {
        bail!(
            "--reanalyze needs the folder to work on:\n  \
             km-package-builder <folder> --reanalyze"
        );
    };
    let db = Db::open(root)?;

    let paths = db.paths_matching(&crate::scan::every_song())?;
    if paths.is_empty() {
        say("  nothing indexed yet -- scan this folder first");
        return Ok(Started::Done);
    }
    let total = paths.len() as u64;
    say(format!("  re-reading  {total} song(s)"));

    let workspace = crate::workspace::Workspace::new(db);
    let progress =
        workspace.start_scan(crate::scan::ScanOptions::only(paths.into_iter().collect()));

    // One Ctrl-C ends it cleanly and keeps what has been read; the watchdog spawned above this
    // branch takes the second and leaves at once. Both see every signal, because tokio notifies
    // every listener — so the first press arms the watchdog's second await as well as this.
    let asked_to_stop = std::sync::Arc::clone(&progress);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        say("\n  stopping; what has already been read will be written.");
        asked_to_stop.ask_to_stop();
    });

    let locale = settings::Settings::load().locale().unwrap_or_default();
    let words = crate::words::messages(locale);
    // Counted in songs read rather than in [`ProgressView::percent`], which is the share *written* —
    // a number that sits at zero through the first batch and would read as a run doing nothing.
    let mut meter = km_console::Meter::new(words.msg("scan-meter-reading"), total);
    let mut said_phase = String::new();
    loop {
        let mut view = progress.snapshot();
        // The phase is a catalog key named on a worker thread, where no language is in reach. This
        // is the first place one is, exactly as a page handler is for the Scan page.
        let phase = words.msg(&view.phase).into_owned();
        if phase != said_phase {
            // Ahead of the phase line, so a rewriting meter does not have the phase printed into the
            // middle of it.
            meter.clear();
            say(format!("  {phase}"));
            said_phase = phase;
        }
        if view.finished {
            meter.clear();
            view.say_counts(locale);
            say(format!("  {}", view.tally));
            if let Some(failed) = &view.failed_said {
                say(format!("  {failed}"));
            }
            if let Some(error) = &view.error {
                bail!("the re-analysis failed: {error}");
            }
            break;
        }
        meter.at(view.done);
        tokio::time::sleep(REANALYZE_POLL_EVERY).await;
    }

    Ok(Started::Done)
}

/// Everything from the command line to a listening server.
///
/// Split out of `main` so that the main thread is free afterwards. The ordering inside it is
/// unchanged and is load-bearing; the comments say why in place.
///
/// The command line arrives already parsed, because [`run`] needs it first — the log filter comes
/// from `-v`, and a subscriber cannot be started before the flag that sets its level is read. The
/// log file comes with it for the same reason, opened by [`init_logging`] and passed here only so
/// that the banner can name it.
async fn start(cli: Cli, log_file: Option<km_logfile::LogFile>, shell: Shell) -> Result<Started> {
    // Before anything is bound or opened: these do one thing and leave. Reported through the same
    // `say` as everything else, so a `--register` run that was launched by double-clicking an
    // installer shortcut does not panic trying to print its result.
    if cli.register {
        register::register()?;
        return Ok(Started::Done);
    }
    if cli.unregister {
        register::unregister()?;
        return Ok(Started::Done);
    }

    // A folder, the `.kmbuild` file inside one, or nothing at all.
    let root = match cli.root.as_deref() {
        None => None,
        Some(given) => match folder_of(given) {
            Some(folder) => Some(folder),
            None => bail!("{} is not a folder or a .kmbuild file", given.display()),
        },
    };

    // **With no folder named, reopen the one from last time.** Most people curate one corpus, and
    // making them pick it out of a list every single start is the friction this milestone set out to
    // remove rather than relocate. It is only ever the *most recent* entry, and only when it is still
    // there and still openable: a folder on an unplugged drive, or one whose database has gone,
    // falls through to the picker, which is where both of those are explained. `--pick` asks for
    // the picker regardless.
    //
    // Nothing is opened *here* — `begin_open` starts the job and the Open page shows it finishing,
    // which is the same path every other way of opening a folder takes.
    let reopen = match (&root, cli.pick) {
        (None, false) => folder_to_reopen(),
        _ => None,
    };

    // Asked once and carried, because three decisions below turn on it and it cannot change while
    // this runs: whether anything printed will be read is settled by how the process was launched.
    let nowhere_to_talk = km_console::nowhere_to_talk();

    // **Everything that does not need the database happens first, and that ordering is the point.**
    // Opening a corpus can take minutes — a migration to run, indexes to build — and until this was
    // reordered every one of those minutes was spent with no port open and nothing on screen. Now the
    // socket is listening and the URL is printed before any of it starts, so a browser pointed at the
    // tool waits in the accept backlog instead of being refused. Nothing can be *served* early by
    // accident: `axum::serve` is not called until the open has returned, and `State::new` takes the
    // `Db` by value, so no handler can reach a database that does not exist yet.
    //
    // The one thing that must not move behind the bind is the refusal, hence this: printing a URL for
    // a server that is about to exit because the folder holds no database would be a worse message
    // than the one it is meant to give.
    //
    // **Only when a folder was named.** With none, there is nothing to refuse — the Open page is the
    // whole point of starting that way — and the same check runs again inside `begin_open` when a
    // folder is finally chosen, which is where a wrong one has to be reported anyway.
    //
    // **And only when the refusal has somewhere to be read.** A run that will draw the Open page
    // reports this there instead, for the reason in [`opens_eagerly`]: a folder holding two
    // databases is reachable by double-clicking one of them, and a refusal returned out of a
    // windowed run is a process that exits while the screen does not change.
    if let Some(root) = &root
        && !cli.init
        && opens_eagerly(&shell, &cli, nowhere_to_talk)
    {
        db::require_database(root)?;
    }

    // Two more that do one job and leave — but unlike `--register` at the top these need a folder,
    // so they branch here instead: after the root has been resolved and the database is known to be
    // there, and still before anything is bound. A run-and-exit path that had already printed a URL
    // would be a tool announcing a server it is about to not run.
    if cli.backup.is_some() || cli.restore.is_some() {
        return backup_and_exit(&cli, root.as_deref());
    }

    // A second Ctrl-C leaves at once. The first one waits for the scan's writer to put down what it
    // is holding, which is bounded to a few batches and so takes well under a second — but a tool
    // somebody cannot get out of is worse than one that loses a batch, and this costs nothing.
    // Both awaits see the same signal because tokio notifies every listener.
    //
    // Installed before the database is opened rather than after, so the long one-time migration is
    // interruptible too. It resumes on the next open, so leaving costs nothing but the time.
    tokio::spawn(async {
        let _ = tokio::signal::ctrl_c().await;
        let _ = tokio::signal::ctrl_c().await;
        say("\n  leaving now; the scan's last batch is lost.");
        std::process::exit(130);
    });

    // The third that does one job and leaves, and the only one placed *after* the watchdog. It reads
    // the whole corpus, which is twenty minutes rather than the second a backup takes, so it needs
    // both halves of what the two branches above return before: a Ctrl-C that ends it, and the
    // checkpoint [`Workspace`] writes on the way out. Still ahead of the bind, for the reason the
    // others are.
    if cli.reanalyze {
        // **The one flag the windowed executable refuses**, and the refusal is the feature. This is
        // a long run whose whole worth is the progress it prints and the status it ends on, and a
        // GUI-subsystem image hands its prompt back before either exists — so the run carries on
        // behind a prompt that has moved on, reporting to nobody. The other run-and-exit flags are
        // over in a second and lose nothing by it. Naming the twin rather than dropping the flag:
        // somebody typed the right thing into the wrong one of two executables, and a sentence is
        // what tells them that.
        if detaches_from_the_shell(&shell) {
            bail!(
                "--reanalyze reads for many minutes and reports as it goes, and this program hands \
                 the prompt back before it starts.\n  \
                 Either use Re-analyze the songs on the Scan page, which is the same work with a \
                 progress bar,\n  \
                 or run the console one, which waits and prints:\n    \
                 km-package-builder-console <folder> --reanalyze"
            );
        }
        return reanalyze_and_exit(root.as_deref()).await;
    }

    let address = SocketAddr::new(
        if cli.lan {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        },
        cli.port,
    );
    // **The address being taken is the one bind failure worth its own sentence**, because it is the
    // one a person causes and the one they can undo — a corpus double-clicked while a copy is up.
    // `binding 127.0.0.1:8178: os error 10048` names the operating system's complaint rather than
    // the situation. The kind is what is matched and not a number, so `WSAEADDRINUSE` and
    // `EADDRINUSE` arrive here as the same thing.
    //
    // **What is listening is deliberately not claimed.** Any program at all can hold that port, so
    // the sentence says what is true and offers the way out for each case. Every other kind keeps
    // the generic context: they are rare, and a wrong guess reads worse than no guess.
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            bail!(
                "something is already listening on {address}. If it is a package builder, use the \
                 window it already has — its Open page opens another folder — or quit it first. \
                 Otherwise start this one with --port."
            );
        }
        Err(error) => return Err(anyhow::Error::new(error).context(format!("binding {address}"))),
    };
    let bound = listener.local_addr().unwrap_or(address);
    // Bound to 0.0.0.0 the socket address is `0.0.0.0:8178`, which is not an address anything can
    // connect to. The loopback form is the one to print and the one to hand a browser.
    let url = format!("http://127.0.0.1:{}/", bound.port());

    say("km-package-builder");
    match &root {
        Some(root) => {
            say(format!("  curating   {}", root.display()));
            say(format!(
                "  database   {}",
                db::database_in(root)
                    .unwrap_or_else(|| root.join(DATABASE_NAME))
                    .display()
            ));
        }
        // Not an apology. Starting with no folder is the ordinary way in now, and the line says what
        // is about to happen rather than reporting an absence.
        None => match &reopen {
            Some(folder) => say(format!(
                "  reopening  {} — --pick for the list",
                folder.display()
            )),
            None => say("  curating   no folder yet — choose one on the page"),
        },
    }
    // Only when there is one, exactly as `km-remote` does it: a line saying "off" belongs in a
    // report somebody asked for and not in the block this prints on every start.
    // The one line this tool says about a destination it is no longer printing to. Said here rather
    // than where the connection opens, which is before this process has settled whether it has a
    // console at all.
    if let Some(address) = km_ecapplog::address() {
        say(format!("  log        ECAppLog at {address}"));
    }
    if let Some(file) = &log_file {
        say(format!("  log        {}", file.path().display()));
    }
    say(format!("  open       {url}"));
    if cli.lan {
        say(
            "\n  WARNING: listening on every interface. There is no password on this tool, and it \
             can open files and write packages as you.",
        );
    }

    // The OS accepts into the backlog even before `serve` starts draining it, so the browser opens on
    // a tab that waits rather than one that fails — which during a migration is the difference between
    // "it is working" and "it is broken". Best-effort: a machine with no default browser is a normal
    // thing and no reason to refuse to start, and the address was just printed.
    //
    let open_a_browser = will_open_a_browser(&shell, &cli, nowhere_to_talk);
    if open_a_browser && let Err(error) = km_osopen::open_url(&url) {
        say(format!(
            "  (could not open a browser: {error} — open {url} yourself)"
        ));
    }

    let state = match &root {
        // A folder named on the command line, in a run with a window to draw the refusal in. The
        // job is the same one the Open page starts for a folder pressed on it, so a corpus that
        // takes minutes to migrate shows its progress here too rather than holding the window blank.
        // See [`opens_eagerly`].
        Some(root) if !opens_eagerly(&shell, &cli, nowhere_to_talk) => {
            let state = State::empty();
            if let Err(error) = state.begin_open(root.clone(), false) {
                say(format!("  (could not open it: {error})"));
                // The refusals `begin_open` makes without starting a job — two databases in the
                // folder, one under a name this build does not open — leave nothing for the page to
                // poll. This is the whole reason they would otherwise be silent.
                state.report_failed_open(root, &error.to_string());
            }
            state
        }
        Some(root) => {
            let db = if cli.init {
                Db::create(root).with_context(|| {
                    format!(
                        "creating {}",
                        db::database_in(root)
                            .unwrap_or_else(|| root.join(DATABASE_NAME))
                            .display()
                    )
                })?
            } else {
                Db::open(root)?
            };

            let app_url = match &cli.machine {
                Some(url) => url.trim().to_owned(),
                None => chosen::load(&db)?
                    .map(|known| known.url)
                    .unwrap_or_else(|| DEFAULT_APP_URL.to_owned()),
            };

            let counts = db.counts()?;
            let state = State::new(db);
            // A folder opened by name is as much "the one I was working on" as one picked off the
            // page, so it goes in the recent list too. Without this the list only ever recorded
            // folders chosen through the picker, and somebody whose habit is to name the folder would
            // be offered an empty list and never have the next start reopen anything.
            state.remember(root, counts.songs, counts.files);

            say(format!(
                "  indexed    {} song(s) across {} file(s)",
                counts.songs, counts.files
            ));
            say(format!("  karaoke    {app_url}"));
            // Only when nothing is going to happen about it. Telling somebody to "restart with
            // --scan" in the same breath as starting the scan they asked for reads like the flag was
            // ignored.
            if counts.files == 0 && !cli.scan {
                say("\n  Nothing indexed yet — open the Scan page, or restart with --scan.");
            }
            if cli.scan {
                say("\n  Scanning now. Watch it on the Scan page.");
                state.start_scan(scan::ScanOptions::default());
            }
            state
        }
        None => {
            // `--scan` names a corpus that has not been chosen yet. Said now rather than silently
            // dropped: a flag that does nothing is worth one line, and the Open page is about to make
            // the choice it was waiting on.
            if cli.scan {
                say("\n  --scan needs a folder; choose one and it is on the page.");
            }
            let state = State::empty();
            // Started as a job, exactly as the page's own button starts one, so a corpus that takes
            // minutes to migrate shows its progress instead of holding the first request open. A
            // failure here is not fatal: it lands on the Open page with the reason on it, which is a
            // better answer than refusing to start.
            if let Some(folder) = reopen
                && let Err(error) = state.begin_open(folder.clone(), false)
            {
                say(format!("  (could not reopen it: {error})"));
                state.report_failed_open(&folder, &error.to_string());
            }
            state
        }
    };

    // The address is a fact only the socket knows -- `--port 0` means whatever is free -- and the
    // header's *Open in browser* button has to be able to name it. Recorded here rather than passed
    // to the router, so a handler asks the state for it the way it asks for everything else.
    state.set_url(&url);

    // Before the first request, in every branch above: a pin needs no folder.
    if let Some(machine) = &cli.machine {
        state.pin_machine(machine.trim());
    }

    // **After the bind, and never in `State::empty`.** Opening a multicast socket is what
    // `CONTRIBUTING.md`'s *No test binds a non-loopback address* forbids, and a great many tests
    // build a `State`. Starting it here means the Discover button answers instantly from a registry
    // that has been listening since the tool opened, and that a test that never serves never
    // listens.
    state.start_watching();

    // **Spawned rather than awaited**, so that the main thread is free for whatever takes over — the
    // event loop, in a desktop build. Everything about the server itself is unchanged.
    let quit = state.clone();
    let serving = tokio::spawn(async move {
        let served = axum::serve(listener, router(quit.clone()).into_make_service())
            .with_graceful_shutdown(async move {
                // Three ways out now, and they run the identical shutdown. Ctrl-C is the one a
                // terminal has; Quit is the one a page has; closing the window is the third, and it
                // asks through the same signal. The second two are not niceties — a tool started by
                // double-clicking its corpus has no console to press Ctrl-C in, so without them there
                // would be no way to stop it but the task manager.
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    () = quit.quit_requested() => {}
                }
                // The first thing printed after the key is pressed, before anything slow starts. What
                // follows can take seconds on a corpus this size, and a tool that goes silent for
                // seconds after Ctrl-C is one somebody presses Ctrl-C at again.
                say("\n  stopping...");
            })
            .await;
        if let Err(error) = served {
            tracing::error!("serving stopped: {error}");
        }
    });

    // The same question `will_open_a_browser` asked above, and asking it twice rather than binding it
    // once is deliberate: without the `desktop` feature this arm does not exist, and a binding only
    // this arm reads is an unused variable — which the workspace's lints refuse.
    #[cfg(feature = "desktop")]
    if will_have_a_tray(&shell) {
        return Ok(Started::Shell {
            window: will_have_a_window(&shell, &cli),
            state,
            url,
            serving,
        });
    }

    Ok(Started::Serving { serving, state })
}

/// The last of the shutdown, once the server has stopped.
///
/// Runs after nothing is asking the database for a page, so the scan is not racing a checkpoint.
/// Killing the thread instead — which is what simply returning from `main` does, since the process
/// exits and the scan is never joined — throws away the batch in hand and everything queued behind
/// it.
///
/// **The work itself lives in `Workspace::drop`**, because a folder can also be closed while the tool
/// goes on running and that path needs the same stopping and the same checkpoint. What is here is the
/// timing, which is the part to keep: on the real corpus this is where the several seconds between
/// Ctrl-C and the prompt coming back actually go, and a wait with no sentence in front of it reads as
/// a hang. See `Db::close`.
fn finish(state: &State) {
    let started = Instant::now();
    state.close_folder();
    let waited = started.elapsed();
    if waited.as_secs_f32() > 0.5 {
        say(format!(
            "  wrote the journal back into the database in {:.1}s.",
            waited.as_secs_f32()
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    use crate::testing::Scratch;

    /// `--overwrite` is the one flag here that can take work away, so it must never be a word
    /// somebody typed that did nothing. clap's `requires` is what makes it an error rather than a
    /// silent no-op, and this pins it: the attribute is one line to lose in a refactor and the
    /// failure it prevents is invisible.
    /// The viewer is off unless asked for, and naming one takes an `=`.
    ///
    /// **The `=` is what keeps the positional folder.** Written with a space, an optional-value flag
    /// swallows whatever follows it — so the corpus somebody named would be read as a viewer's
    /// address and the tool would open nothing.
    #[test]
    fn the_viewer_is_off_unless_asked_for() {
        use clap::Parser as _;

        assert_eq!(Cli::parse_from(["km-package-builder"]).ecapplog, None);
        assert_eq!(
            Cli::parse_from(["km-package-builder", "--ecapplog"]).ecapplog,
            Some(km_ecapplog::DEFAULT_ADDRESS.to_owned())
        );
        assert_eq!(
            Cli::parse_from(["km-package-builder", "--ecapplog=1.2.3.4:99"]).ecapplog,
            Some("1.2.3.4:99".to_owned())
        );
        assert!(Cli::try_parse_from(["km-package-builder", "--ecapplog=nope"]).is_err());

        let cli = Cli::parse_from(["km-package-builder", "--ecapplog", "D:/tunes/karaoke"]);
        assert_eq!(cli.ecapplog, Some(km_ecapplog::DEFAULT_ADDRESS.to_owned()));
        assert_eq!(cli.root, Some(PathBuf::from("D:/tunes/karaoke")));
    }

    #[test]
    fn overwrite_without_restore_is_refused_by_the_command_line() {
        use clap::Parser as _;

        assert!(
            Cli::try_parse_from(["km-package-builder", "/tunes/karaoke", "--overwrite"]).is_err(),
            "--overwrite on its own has nothing to overwrite from"
        );
        let paired = Cli::try_parse_from([
            "km-package-builder",
            "/tunes/karaoke",
            "--restore",
            "kept.json",
            "--overwrite",
        ])
        .expect("--overwrite beside --restore is the whole point of it");
        assert!(paired.overwrite);
        assert_eq!(paired.restore.as_deref(), Some(Path::new("kept.json")));
    }

    /// `--reanalyze` reads the whole corpus and writes over every detected column, so pairing it
    /// with a flag that has its own idea of what the run is for would leave the answer to whichever
    /// branch `start` reaches first. Refused at the command line, where the sentence names the two.
    #[test]
    fn reanalyze_refuses_to_share_a_run() {
        use clap::Parser as _;

        let alone = Cli::try_parse_from(["km-package-builder", "/tunes/karaoke", "--reanalyze"])
            .expect("a folder and the flag is the whole invocation");
        assert!(alone.reanalyze);

        for other in [
            vec!["--init"],
            vec!["--scan"],
            vec!["--backup", "before.json"],
            vec!["--restore", "kept.json"],
        ] {
            let mut argv = vec!["km-package-builder", "/tunes/karaoke", "--reanalyze"];
            argv.extend(other.iter().copied());
            assert!(
                Cli::try_parse_from(&argv).is_err(),
                "--reanalyze with {} must be refused",
                other.join(" ")
            );
        }
    }

    /// The console twin is the one a long run can be typed into, and the rule is the subsystem.
    ///
    /// **Written as a rule over `Shell` rather than read off the platform**, so the test runs on all
    /// three of them. What it pins is that the twin never refuses: it carries no
    /// `windows_subsystem` attribute wherever it is built, so whatever the other one does, this one
    /// waits — which is the whole reason it exists.
    #[test]
    fn only_the_windowed_executable_hands_its_prompt_back() {
        assert!(
            !detaches_from_the_shell(&Shell::Console),
            "the console twin waits, on every platform"
        );
        assert_eq!(
            detaches_from_the_shell(&Shell::Windowed),
            cfg!(all(windows, feature = "desktop")),
            "the windowed one detaches exactly where main.rs asks for a GUI subsystem"
        );
    }

    /// The two flags are one branch so that a backup is taken before a restore reads over it. Both
    /// at once therefore has to *parse*, or that ordering is a comment about something impossible.
    #[test]
    fn backing_up_and_restoring_in_one_run_is_allowed() {
        use clap::Parser as _;

        let both = Cli::try_parse_from([
            "km-package-builder",
            "/tunes/karaoke",
            "--backup",
            "before.json",
            "--restore",
            "kept.json",
        ])
        .expect("taking a backup before restoring over it is the safe order, not a conflict");
        assert_eq!(both.backup.as_deref(), Some(Path::new("before.json")));
        assert_eq!(both.restore.as_deref(), Some(Path::new("kept.json")));
    }

    #[test]
    fn a_canonical_path_loses_its_windows_prefix() {
        // `tidy` falls through to the input when the path does not exist, which is what makes this
        // testable without touching a real directory.
        let path = PathBuf::from("/definitely/not/here");
        assert_eq!(crate::model::tidy(&path), path);
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
                std::iter::once("km-package-builder".to_owned())
                    .chain((0..verbose).map(|_| "-v".to_owned())),
            )
            .log_filter(&km_logsettings::LoggingSettings::default())
        };
        // The one that matters: a shipped build says nothing at `debug` unless asked.
        assert_eq!(filter(0), format!("info,{QUIET_DEPENDENCIES}"));
        assert_eq!(
            filter(1),
            format!("info,km_package_builder=debug,{QUIET_DEPENDENCIES}")
        );
        // `-vv` drops the quietening too — somebody asking for everything asked for the chatter.
        assert_eq!(filter(2), "debug,km_package_builder=trace");
        // Beyond the ladder is the top of it, not a panic.
        assert_eq!(filter(5), "debug,km_package_builder=trace");
        // The thing it is actually there to hold down, named rather than implied: symphonia's MP3
        // demuxer announces itself on every pair this tool scans, at `info` and at `warn` both, so
        // `warn` here would leave a line per pair still going to the terminal.
        assert!(QUIET_DEPENDENCIES.contains("symphonia_bundle_mp3=error"));
        assert!(QUIET_DEPENDENCIES.contains("symphonia_core=error"));
    }

    /// The settings file is the rung below the flag, and a directive replaces the ladder.
    ///
    /// `RUST_LOG` is left out for the reason the test above gives: it lives in an environment this
    /// binary's tests share, and setting it would be a race rather than a test.
    #[test]
    fn a_settings_file_says_the_level_where_nobody_can_type_one() {
        if std::env::var_os("RUST_LOG").is_some() {
            return;
        }
        let filter = |verbose: u8, level: &str| {
            let settings = km_logsettings::LoggingSettings {
                level: Some(level.to_owned()),
                ..km_logsettings::LoggingSettings::default()
            };
            Cli::parse_from(
                std::iter::once("km-package-builder".to_owned())
                    .chain((0..verbose).map(|_| "-v".to_owned())),
            )
            .log_filter(&settings)
        };
        // Nobody typed a flag, so the file speaks -- and it speaks whole, taking the place of the
        // rung rather than being appended to it.
        assert_eq!(filter(0, "debug"), "debug");
        assert_eq!(
            filter(0, "warn,km_package_builder=trace"),
            "warn,km_package_builder=trace"
        );
        // One run typed at a keyboard is never an edit to the box.
        assert_eq!(
            filter(1, "debug"),
            format!("info,km_package_builder=debug,{QUIET_DEPENDENCIES}")
        );
        // A directive nobody can read leaves the ladder standing.
        assert_eq!(filter(0, "="), format!("info,{QUIET_DEPENDENCIES}"));
    }

    #[test]
    fn the_cli_parses_the_shape_the_docs_promise() {
        let cli = Cli::try_parse_from([
            "km-package-builder",
            "D:/tunes/karaoke",
            "--init",
            "--port",
            "9000",
        ])
        .expect("parse");
        assert_eq!(cli.root, Some(PathBuf::from("D:/tunes/karaoke")));
        assert!(cli.init);
        assert_eq!(cli.port, 9000);
        assert!(!cli.lan);
        assert!(!cli.open);
    }

    /// Starting with no folder is now a documented way to run this, not a usage error.
    ///
    /// It has to be: a double-clicked application is handed no arguments, and the Open page is what
    /// answers that. The assertion is on clap accepting it, because making `root` optional is the
    /// one change that could silently turn a typo into "start the picker" — and it does not, since
    /// anything that is not a folder or a `.kmbuild` file is still refused by name in `main`.
    #[test]
    fn no_folder_at_all_is_accepted_and_means_the_picker() {
        let cli = Cli::try_parse_from(["km-package-builder"]).expect("parse");
        assert_eq!(cli.root, None);
        assert!(!cli.init);
    }

    /// A double-clicked `.kmbuild` arrives as a file path, and the folder is its parent.
    #[test]
    fn a_database_path_resolves_to_the_folder_around_it() {
        let scratch = Scratch::new("folder-of");
        let folder = scratch.0.clone();
        let tidied = crate::model::tidy(&folder);

        let database = folder.join(db::DATABASE_NAME);
        std::fs::write(&database, b"a stand-in; this never gets opened").expect("write");

        // The document resolves to the folder around it — the shape a double-click delivers.
        assert_eq!(folder_of(&database).as_deref(), Some(tidied.as_path()));
        // A folder resolves to itself — the shape a person types.
        assert_eq!(folder_of(&folder).as_deref(), Some(tidied.as_path()));

        // Case is not significant: Windows and macOS hand back whatever case the file was made with,
        // and a `.KMBUILD` that would not open is baffling.
        let shouted = folder.join("CORPUS.KMBUILD");
        std::fs::write(&shouted, b"x").expect("write");
        assert_eq!(folder_of(&shouted).as_deref(), Some(tidied.as_path()));

        // Anything else is a typo, and is reported as one rather than quietly starting the picker —
        // which is the failure the optional argument could most easily have introduced.
        let stray = folder.join("notes.txt");
        std::fs::write(&stray, b"x").expect("write");
        assert_eq!(folder_of(&stray), None);
        assert_eq!(folder_of(Path::new("/definitely/not/here")), None);
    }

    /// A run's shape, from the flags a person actually typed.
    fn shape(shell: Shell, args: &[&str], nowhere_to_talk: bool) -> (bool, bool) {
        let cli =
            Cli::try_parse_from(std::iter::once("km-package-builder").chain(args.iter().copied()))
                .expect("parse");
        (
            will_have_a_window(&shell, &cli),
            will_open_a_browser(&shell, &cli, nowhere_to_talk),
        )
    }

    /// **The reported bug: a double-clicked windowed build opened the window and a browser tab.**
    ///
    /// Double-clicked there is no console, so `nowhere_to_talk` is true and `--open` is implied —
    /// which is right for a build with no window and was being applied to one that had.
    #[cfg(feature = "desktop")]
    #[test]
    fn a_window_is_shown_instead_of_a_browser_and_never_as_well_as_one() {
        // Double-clicked: a window, and nothing else.
        assert_eq!(shape(Shell::Windowed, &[], true), (true, false));
        // ...and from a terminal, where the address was printed, likewise.
        assert_eq!(shape(Shell::Windowed, &[], false), (true, false));

        // **`--open` is satisfied by the window rather than competing with it.** This is the case
        // `run-package-builder.local.bat` hits, and honoring the flag literally would have left that
        // launcher opening two views of the same page.
        assert_eq!(shape(Shell::Windowed, &["--open"], false), (true, false));

        // `--browser` is how you ask for a tab instead — one view either way, never two.
        assert_eq!(
            shape(Shell::Windowed, &["--browser"], false),
            (false, false)
        );
        assert_eq!(
            shape(Shell::Windowed, &["--browser", "--open"], false),
            (false, true)
        );
        assert_eq!(shape(Shell::Windowed, &["--browser"], true), (false, true));

        // Serving the house has nobody at this machine to show a window to.
        assert_eq!(shape(Shell::Windowed, &["--lan"], false), (false, false));

        // The console twin never has a window, so the implication is exactly what it always was.
        // The first row is rarer than it used to look: the twin now keeps the console it is
        // double-clicked into, so "nowhere to talk" is a twin started with no console at all rather
        // than a twin somebody double-clicked. See `km_console::Console`.
        assert_eq!(shape(Shell::Console, &[], true), (false, true));
        assert_eq!(shape(Shell::Console, &["--open"], false), (false, true));
        assert_eq!(shape(Shell::Console, &[], false), (false, false));
    }

    /// The failure window escapes the path it is naming.
    ///
    /// **What goes into that page is an anyhow chain carrying a path somebody chose**, and a path is
    /// a string like any other: a folder with an `&` in its name would otherwise put a broken
    /// document in front of somebody whose tool had already failed once.
    #[test]
    fn the_failure_window_escapes_the_message_it_is_given() {
        let page = failure_html("D:\\tunes\\rock & <roll>\\x.kmbuild is not a folder");

        assert!(page.contains("rock &amp; &lt;roll&gt;"), "{page}");
        assert!(
            !page.contains("<roll>"),
            "the raw angle brackets reached the document: {page}"
        );
        // The path itself survives being escaped — an unreadable message is no better than none.
        assert!(page.contains("D:\\tunes\\"), "{page}");
        assert!(page.contains("x.kmbuild is not a folder"), "{page}");
        // And the page is a whole document, since nothing is going to wrap it.
        assert!(page.starts_with("<!doctype html>"), "{page}");
    }

    /// An address already taken names the tool that took it, not the operating system's number.
    ///
    /// **The second double-click is what this is for.** Nothing here is a single-instance mechanism
    /// — a second copy still starts and still leaves — but what it leaves behind is a sentence
    /// somebody can act on rather than `os error 10048`.
    ///
    /// `--pick` so that no recent list is read: the folder this would otherwise reopen belongs to
    /// whoever is running the suite. `--port 0` is not available here for the obvious reason, so the
    /// port is taken first and its number asked for by name. The listener is held for the whole
    /// test; dropping it early would free the address this is asserting is busy.
    #[tokio::test]
    async fn an_address_already_taken_says_what_is_on_it() {
        let held = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("take a port");
        let port = held.local_addr().expect("the port taken").port();

        let cli = Cli::parse_from(["km-package-builder", "--pick", "--port", &port.to_string()]);
        // A match rather than `expect_err`: what `start` hands back on success holds a running
        // server and a live state, neither of which is `Debug` and neither of which this can reach.
        let Err(error) = start(cli, None, Shell::Console).await else {
            panic!("the address is taken, so this cannot bind");
        };

        let said = format!("{error:#}");
        assert!(
            said.contains("already listening"),
            "the refusal names the situation: {said}"
        );
        // **What is holding the port is not claimed**, only offered as the likely case. Anything at
        // all can be on it, and this test is the pin on not saying otherwise.
        assert!(
            !said.contains("binding "),
            "the refusal is not the socket call: {said}"
        );
        assert!(
            said.contains(&port.to_string()),
            "the refusal names the address: {said}"
        );
        drop(held);
    }

    /// Where a folder that will not open is reported, from the flags a person actually typed.
    ///
    /// **The row that matters is the first one**: a windowed run with no flags is the double-click,
    /// and an eager open there returns the refusal into a process with no console and null handles
    /// — which is the reported bug, a corpus that opens nothing and says nothing. Lazy, the same
    /// sentence lands on the Open page with the chooser beside it.
    ///
    /// The flags that stay eager each name a corpus they cannot act on until it is open, and each
    /// was typed at a prompt that is still there to print to.
    #[test]
    fn a_folder_opens_as_a_job_only_where_a_refusal_would_be_seen() {
        let decide = |shell: Shell, args: &[&str], nowhere_to_talk: bool| {
            let cli =
                Cli::parse_from(std::iter::once("km-package-builder").chain(args.iter().copied()));
            opens_eagerly(&shell, &cli, nowhere_to_talk)
        };

        // **The double-click, and the only row in the table that defers.** It needs the feature: a
        // build with no window has no page to put the refusal on.
        assert_eq!(
            decide(Shell::Windowed, &[], true),
            !cfg!(feature = "desktop")
        );

        // **The same executable and the same folder, typed at a prompt.** Standard handles are
        // inherited whatever the subsystem, so the sentence prints and the exit status is read —
        // and deferring here would take both away from a script.
        assert!(decide(Shell::Windowed, &[], false));

        // Each of these names a corpus it cannot act on until the database is open.
        for flags in [
            &["--init"][..],
            &["--scan"][..],
            &["--backup", "out.json"][..],
            &["--restore", "in.json"][..],
            &["--reanalyze"][..],
        ] {
            assert!(decide(Shell::Windowed, flags, true), "{flags:?}");
        }

        // No window means a browser or a console, and both are somewhere the sentence already goes.
        assert!(decide(Shell::Windowed, &["--browser"], true));
        assert!(decide(Shell::Windowed, &["--lan"], true));
        assert!(decide(Shell::Console, &[], true));
        assert!(decide(Shell::Console, &[], false));
        assert!(decide(Shell::Console, &["--scan"], true));
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
        let decide = |shell: Shell, args: &[&str]| {
            let cli =
                Cli::parse_from(std::iter::once("km-package-builder").chain(args.iter().copied()));
            (will_have_a_tray(&shell), will_have_a_window(&shell, &cli))
        };

        // A window and an icon beside it.
        assert_eq!(decide(Shell::Windowed, &[]), (true, true));
        // No window, and therefore the case the icon is for.
        assert_eq!(decide(Shell::Windowed, &["--browser"]), (true, false));
        assert_eq!(decide(Shell::Windowed, &["--lan"]), (true, false));
        // The console twin has a console and a Ctrl-C; it needs neither.
        assert_eq!(decide(Shell::Console, &[]), (false, false));
        assert_eq!(decide(Shell::Console, &["--browser"]), (false, false));
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

    /// Without the feature there is no window to prefer, so nothing about this changed.
    ///
    /// Worth asserting rather than assuming: the fix reads `cfg!(feature = "desktop")` at runtime, so
    /// a build that has no window must still open a browser when nobody can read the address —
    /// otherwise a double-clicked Linux or `--no-desktop` build would show nothing at all.
    #[cfg(not(feature = "desktop"))]
    #[test]
    fn a_build_with_no_window_still_opens_a_browser_for_a_double_click() {
        assert_eq!(shape(Shell::Windowed, &[], true), (false, true));
        assert_eq!(shape(Shell::Windowed, &["--open"], false), (false, true));
        assert_eq!(shape(Shell::Windowed, &[], false), (false, false));
        // `--browser` is accepted here and changes nothing, which is what its help says.
        assert_eq!(shape(Shell::Windowed, &["--browser"], true), (false, true));
    }

    /// `--pick` is what asks for the list when there is a folder to reopen.
    ///
    /// The default is to reopen, because most people curate one corpus and choosing it from a list
    /// every start is the friction this milestone removes rather than relocates.
    #[test]
    fn pick_is_accepted_and_is_not_the_default() {
        let plain = Cli::try_parse_from(["km-package-builder"]).expect("parse");
        assert!(!plain.pick, "reopening the last folder is the default");

        let asked = Cli::try_parse_from(["km-package-builder", "--pick"]).expect("parse");
        assert!(asked.pick);
        assert_eq!(asked.root, None);
    }

    /// A remembered folder that is no longer openable is not reopened behind somebody's back.
    ///
    /// An unplugged drive and a folder whose database has gone both fall through, and the picker
    /// explains each rather than acting on either silently.
    #[test]
    fn only_a_folder_that_is_really_there_is_reopened() {
        let scratch = Scratch::new("reopen-rule");
        let folder = scratch.0.clone();

        assert_eq!(
            crate::browse::indexed(&folder),
            crate::browse::Indexed::No,
            "an empty folder is not something to reopen"
        );

        std::fs::write(folder.join(db::DATABASE_NAME), b"x").expect("write");
        assert_eq!(crate::browse::indexed(&folder), crate::browse::Indexed::Yes);

        assert_eq!(
            crate::browse::indexed(Path::new("/definitely/not/here")),
            crate::browse::Indexed::No,
            "a drive that is not plugged in falls through to the picker"
        );
    }

    /// Every flag named in `CLAUDE.md`, in `dist-tools.sh`'s closing line, and in the README that
    /// ships beside the binary, accepted in one go.
    ///
    /// `--open` was documented in all three and implemented in none of them, so the very first
    /// command the README tells somebody to run failed with `unexpected argument '--open' found`.
    /// A flag that only exists in prose is worse than no flag at all.
    #[test]
    fn every_documented_flag_exists() {
        let cli = Cli::try_parse_from([
            "km-package-builder",
            "D:/tunes/karaoke",
            "--init",
            "--scan",
            "--open",
            "--lan",
            "--port",
            "8178",
            "--machine",
            "http://127.0.0.1:8177",
        ])
        .expect("every documented flag must parse");

        assert!(cli.init);
        assert!(cli.scan);
        assert!(cli.open);
        assert!(cli.lan);
        assert_eq!(cli.port, 8178);
        assert_eq!(cli.machine.as_deref(), Some("http://127.0.0.1:8177"));
    }

    /// The exact line `dist-tools.sh` prints for km-package-builder, and the one the README opens
    /// with.
    #[test]
    fn the_command_the_readme_tells_people_to_run_works() {
        Cli::try_parse_from([
            "km-package-builder",
            "./songs",
            "--init",
            "--scan",
            "--open",
        ])
        .expect("the README's first command must parse");
    }

    /// Every option the staged README lists, accepted.
    ///
    /// The README is written in `tools/dist/cmd.sh` and nothing else checks it — which is exactly
    /// how `--open` came to be documented in three places and implemented in none, so that the first
    /// command it told people to run failed with `unexpected argument`. These four are the new ones.
    #[test]
    fn every_option_the_shipped_readme_lists_is_accepted() {
        Cli::try_parse_from(["km-package-builder", "--pick"]).expect("--pick");
        Cli::try_parse_from(["km-package-builder", "--browser"]).expect("--browser");
        Cli::try_parse_from(["km-package-builder", "--register"]).expect("--register");
        Cli::try_parse_from(["km-package-builder", "--unregister"]).expect("--unregister");
        Cli::try_parse_from(["km-package-builder", "--reanalyze"]).expect("--reanalyze");

        // ...and the two that undo each other cannot be asked for together, which clap enforces
        // rather than the code having to decide which one wins.
        assert!(
            Cli::try_parse_from(["km-package-builder", "--register", "--unregister"]).is_err(),
            "asking to register and unregister at once has no sensible answer"
        );
    }

    // No argument is not a usage error — it means the picker, which
    // `no_folder_at_all_is_accepted_and_means_the_picker` above asserts. What is still enforced is
    // that a *wrong* path is refused rather than silently ignored, in `main` rather than in clap,
    // because it takes a look at the filesystem to tell a folder from a `.kmbuild` from a typo.
}
