//! The `karaokemachine` command.
//!
//! A thin front end. Everything that actually runs the machine lives in [`crate::run`], so that
//! Android — which loads a shared library and calls `SDL_main` rather than executing a binary — can
//! reach the same code. What is here is the parts that only make sense with a command line: choosing
//! where data lives, the `--play` debug path, and setting the admin password.
//!
//! # Why this is a module and not `main.rs`
//!
//! **`#![windows_subsystem]` is a property of a binary crate root**, and this command is built as two
//! binaries that disagree about it: `karaokemachine`, GUI-subsystem on Windows so that
//! double-clicking it opens the machine and nothing else, and `karaokemachine-console`, which is not,
//! and is the one to type. A second crate root can see nothing of a `main.rs` module tree, so
//! everything real had to move where both can reach it. See the `Two executables on Windows` decision
//! in `docs/decisions/distribution.md`, and the three-line files in `src/main.rs` and `src/bin/`.
//!
//! **Nothing here may `println!`.** `std::io::_print` panics rather than failing when stdout cannot
//! be written, and a GUI-subsystem process launched from Explorer has a null standard output handle —
//! so the five flags below that print and exit would have aborted the machine every time somebody
//! double-clicked it with a shortcut carrying one, and never once when tested from a shell. Every
//! line goes through [`km_console::say`], which prints where there is somewhere to print and logs
//! where there is not.

use std::io::IsTerminal;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::Parser;
use km_console::say;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::Options;
use crate::settings::{APP_NAME, LoggingSettings, Paths, Settings, WallpaperSource};

#[derive(Debug, Parser)]
#[command(
    name = "karaokemachine",
    about = "Plays karaoke MIDI files with synced lyrics",
    version
)]
struct Cli {
    // Positional and optional, the shape `km-package-builder` uses for the same job: a file manager
    // passes one argument and nothing else, and the same executable launched from an icon passes
    // none. A flag would work from a shell and not from a double-click, which is the case this
    // exists for. `src/handed.rs` has where it goes — copied into the packages folder either way,
    // then handed to the machine already running on this box, or left for this process's own
    // startup scan.
    /// A package to install. A double-clicked `.kmpkg` arrives this way.
    package: Option<PathBuf>,

    // Analyzes the file on the spot to find its melody channel, because packaging has not done it.
    /// Play one MIDI or KAR file directly, without installing a package.
    #[arg(long, value_name = "FILE")]
    play: Option<PathBuf>,

    /// Run without a window: the API, the engine and the catalog only.
    #[arg(long)]
    headless: bool,

    // **A way of running the machine rather than a build of it**, exactly as `--headless` above is:
    // it overrides `display.enabled` for this process and writes nothing back, so an appliance
    // started this way once still opens its television next time.
    //
    // The stream is at `/watch/` and `/stream/live.m3u8` on the same port everything else is on,
    // so a machine reached one way is reached every way.
    /// Draw the screen for an encoder instead of a television, and serve it as one HLS stream.
    #[arg(long, conflicts_with = "headless")]
    stream: bool,

    // The pair exists rather than one flag because the default is off and an installed machine sets
    // it on, so either direction has to be overridable: `--windowed` for a debugging run against an
    // appliance's settings, `--fullscreen` for a checkout build driving a real television. Declaring
    // the run temporary is also what stops such a run turning an appliance's television into a
    // window from then on.
    /// Run full screen, whatever `display.fullscreen` says.
    ///
    /// Applies to this run only. Nothing is written to settings.json, including the window size and
    /// position, which are normally saved when the machine closes. `F` still toggles.
    #[arg(long, conflicts_with = "windowed")]
    fullscreen: bool,

    /// Run in a window, whatever `display.fullscreen` says.
    ///
    /// Applies to this run only. See `--fullscreen`.
    #[arg(long)]
    windowed: bool,

    /// Keep settings and the catalog here instead of in the platform's directories.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    // Moving one process off `api.bind` is what a second machine on one box needs and the one thing
    // the settings file cannot express.
    /// Bind the API to this address for this run: a port alone, or `ADDRESS:PORT`.
    ///
    /// Applies to this run only; nothing is written to settings.json. A port alone keeps the
    /// interface `api.bind` names and changes only the port.
    ///
    /// Use `--data-dir` as well, or two machines on two ports will share one catalog and one
    /// packages folder.
    #[arg(long, value_name = "ADDR")]
    api_bind: Option<String>,

    // The spelling the other three servers use: `km-package-builder`, `km-remote` and `km-admin`
    // all take `--port` and `--lan`, as does `km-api`'s `dev_server` example.
    /// Serve the API on this port for this run. Same as `--api-bind <port>`.
    #[arg(long, value_name = "PORT", conflicts_with = "api_bind")]
    port: Option<u16>,

    // The machine's default is not the other three tools': `api.bind` ships as `0.0.0.0` because a
    // karaoke machine no phone in the room can reach is not one, where the tools bind loopback
    // because they are workstation programs. So this says "the usual thing" explicitly once
    // `--port` has been given, and `--port` alone is the quiet one.
    /// Serve the API on every network interface for this run, not only loopback.
    ///
    /// On Windows this raises the firewall's "allow this app" prompt once per program, port and
    /// network profile. Use `--port` without this for a temporary run.
    #[arg(long, requires = "port")]
    lan: bool,

    // The only screen the ACL editor and the `debug.accept_uploads` switch have, so this is what a
    // curator reaches for when `km-package-builder` is refused an upload.
    /// Serve the development console at `/dev/` for this run, with debugging mode on.
    ///
    /// Applies to this run only; nothing is written to settings.json. Set
    /// `api.serve_dev_remote: true` and `debug.enabled: true` to serve it from every start — the
    /// console needs both, because its own copy of the API asks for no password. It is off by
    /// default because a karaoke machine should not publish a developer console on the network
    /// unasked.
    #[arg(long)]
    dev_remote: bool,

    // Settings hold an `argon2` hash, not a password, so it cannot be typed into the file by hand.
    // There is no first password to set: this and the two surfaces that offer it —
    // `POST /api/v1/admin/password` and the `/admin/` page — all change one that exists.
    /// Change the admin password, and exit.
    ///
    /// A new machine generates its own PIN at first start and shows it on screen, so there is always
    /// a password to change.
    #[arg(long, value_name = "PASSWORD")]
    set_password: Option<String>,

    // Settable only by editing `settings.json` before this, which is no answer on an appliance with
    // no keyboard. 63 bytes is the DNS label limit rather than a preference: the name *is* the
    // DNS-SD instance label. There is no `--clear-name`, because a machine with no name at all is
    // not a state worth reaching.
    /// Set this machine's name on the network, and exit.
    ///
    /// The name phones show in their list of machines. Up to 63 bytes; leading and trailing spaces
    /// are removed and a blank name is refused. To undo, set it back to `KaraokeMachine`.
    #[arg(long, value_name = "NAME")]
    set_name: Option<String>,

    // Reset, not clear: there is no state with no password to clear one to.
    /// Generate a new admin PIN, and exit.
    ///
    /// For an owner who has forgotten the password. The new PIN is printed here and shown on the
    /// machine's screen.
    #[arg(long, conflicts_with = "set_password")]
    reset_password: bool,

    // Bumps the session epoch every token is signed against, so all of them stop verifying at once.
    /// Sign out every phone and browser, and exit.
    ///
    /// The password is not changed.
    #[arg(long)]
    reset_sessions: bool,

    // `HKCU\Software\Classes` on Windows, `~/.local/share` on Linux. On macOS the file type is
    // declared by the `.app` bundle rather than by a command, so this only asks LaunchServices to
    // re-read it. The Windows installer offers it as a task; this is for the tarball and the
    // portable folder.
    /// Open `.kmpkg` files with this machine, and exit.
    ///
    /// Applies to your user account only and needs no administrator rights.
    #[arg(long, conflicts_with = "unregister")]
    register: bool,

    /// Stop opening `.kmpkg` files with this machine, and exit.
    #[arg(long)]
    unregister: bool,

    // Writes `audio.soundfont`, which beats every rule about where banks are found, so unlike the
    // checkout's `local/assets/` overlay it reaches an installed build and the staged `dist/bin`
    // folder as well as a `cargo run`. Fifteen banks were surveyed and four do not load at all;
    // without the check the symptom is a machine that comes up on a sine test tone with the reason
    // in a log nobody reads. `tools/dev/soundfont.sh` — `task soundfont BANK=<name>` — calls this.
    /// Use this `.sf2` sound bank instead of the bundled one, and exit.
    ///
    /// The file is opened and checked before anything is saved, so a bank that will not load is
    /// reported rather than stored.
    #[arg(long, value_name = "FILE")]
    set_soundfont: Option<PathBuf>,

    /// Go back to the bundled sound bank, and exit. Restores the previous `music_volume`.
    #[arg(long, conflicts_with = "set_soundfont")]
    clear_soundfont: bool,

    // What the tick box in the Windows setup program and the macOS package write, so a 261.9 MiB
    // bank does not travel inside a carrier or hold up an install.
    /// Download this sound bank at the next start and use it, then exit.
    ///
    /// Give `recommended`, or a bank id from `task soundfont:list`. Nothing is downloaded now. The
    /// next three starts try, so a machine with no network on first boot still gets it. A bank
    /// already chosen, or already in the folder, is used instead. Undo with `--clear-soundfont`.
    #[arg(long, value_name = "BANK", conflicts_with = "set_soundfont")]
    first_run_soundfont: Option<String>,

    // A bank and a level are one decision rather than two: several banks worth trying exceed full
    // scale at 1.0 — MuseScore_General wants 0.7 and FluidR3_GM 0.6 — so setting the path and
    // leaving the level would hand somebody a bank that clips.
    /// Backing-track level for that bank, 0.0 to 1.0.
    ///
    /// Use with `--set-soundfont`. Some banks distort at 1.0. The previous level is remembered and
    /// `--clear-soundfont` restores it.
    #[arg(long, value_name = "V", requires = "set_soundfont")]
    music_volume: Option<f32>,

    // Slot 1 is fixed so there is one digit that cannot be got wrong and a reference to compare the
    // rest against. Checking every bank matters more for a list than for one path: nine written
    // unchecked would be up to nine keys that each fail in front of a room.
    // `tools/dev/soundfont-debug.sh` — `task soundfont:debug` — finds the banks already here.
    /// Fill the `Ctrl+2`…`Ctrl+9` sound bank slots, and exit.
    ///
    /// Each entry is `<path>`, `<path>=<name>` or `<path>=<name>=<volume>`, where the name is the
    /// on-screen label. Slots are filled in the order given, starting at 2; slot 1 is always the
    /// machine's own bank and cannot be changed.
    ///
    /// Every file is opened and checked before anything is saved. Setting any slot turns the
    /// switcher on. Switching banks with these keys lasts for that run only.
    #[arg(long, value_name = "FILE", num_args = 1.., conflicts_with = "set_soundfont")]
    set_debug_soundfonts: Option<Vec<String>>,

    /// Empty the sound bank slots, and exit. This turns the switcher and its label off.
    #[arg(long, conflicts_with = "set_debug_soundfonts")]
    clear_debug_soundfonts: bool,

    // `tools/dev/soundfont-debug.sh --choose` opens its list with the current slots ticked; the
    // alternative was reading `settings.json` from a shell, which would want a JSON parser this box
    // deliberately does not have.
    /// Print the `Ctrl+2`…`Ctrl+9` sound bank slots, and exit.
    ///
    /// One line per slot, in the same `<path>=<name>=<volume>` form `--set-debug-soundfonts` reads,
    /// so the output can be passed straight back. Prints nothing when the switcher is off.
    #[arg(
        long,
        conflicts_with = "set_debug_soundfonts",
        conflicts_with = "clear_debug_soundfonts"
    )]
    show_debug_soundfonts: bool,

    // This is what `settings.packages` was, and it sits behind `debug.` because of how it fails:
    // every entry is replayed at every pass, so a file moved away becomes a fault reported at every
    // pass until somebody clears it. Nothing here is the machine's to delete —
    // `DELETE /api/v1/packages/{id}` refuses for a package reached through this list, and no API
    // route writes to it.
    /// Install these `.kmpkg` files at every start, and exit.
    ///
    /// Added on top of the packages folders, which are still scanned. A file that is already in a
    /// scanned folder is ignored rather than installed twice. Every file is opened and checked
    /// before anything is saved, and paths are stored absolute.
    ///
    /// For anything other than a one-off, use `settings.package_dirs`: a package removed from a
    /// folder simply stops being installed, where an entry here is reported as missing at every
    /// start until it is cleared.
    #[arg(long, value_name = "FILE", num_args = 1..)]
    set_debug_packages: Option<Vec<String>>,

    /// Empty the extra-packages list, and exit. The folders are unaffected.
    #[arg(long, conflicts_with = "set_debug_packages")]
    clear_debug_packages: bool,

    /// Print the extra `.kmpkg` files installed at every start, and exit.
    ///
    /// One path per line, in the same form `--set-debug-packages` reads. Prints nothing when the
    /// list is empty.
    #[arg(
        long,
        conflicts_with = "set_debug_packages",
        conflicts_with = "clear_debug_packages"
    )]
    show_debug_packages: bool,

    /// Print where settings and the catalog live, and exit.
    #[arg(long)]
    show_paths: bool,

    // The paper half of a karaoke machine, and the one way to find a song that needs neither the
    // screen nor a phone: what a commercial machine ships in a ring binder.
    /// Write a printable song book of everything installed as a PDF, and exit.
    ///
    /// Five columns — artist, number, title, the first line of the words, and the package prefix —
    /// sorted by artist within a section per language.
    ///
    /// Uses the catalog as it stands. Packages are indexed when the machine starts, so a `.kmpkg`
    /// added since the last start is not included. `GET /api/v1/songs/book.pdf` produces the same
    /// document and can narrow it by language or package.
    #[arg(long, value_name = "FILE")]
    song_book: Option<PathBuf>,

    // Whose machine the book belongs to, which is a different question from what the document is
    // called: the centered `SONG LIST` above the columns is unaffected. Two machines in two rooms
    // is the case this exists for. The same string is `?name=` on `GET /api/v1/songs/book.pdf` and
    // `--book-name` on `km-pack book`.
    /// Heading printed at the top left of every song book page.
    ///
    /// Use with `--song-book`. Defaults to this machine's name, such as
    /// `KaraokeMachine - Living Room`.
    #[arg(long, value_name = "NAME")]
    book_name: Option<String>,

    // On an appliance there is no browser and no screen, and the identifiers are not guessable, so
    // this is how you find one over SSH before you can send it.
    /// List the audio output devices, and exit.
    ///
    /// To choose one, use `PUT /api/v1/audio/output` or `audio.output_device` in `settings.json`.
    #[arg(long)]
    list_audio_devices: bool,

    // Six additions and a comparison per frame, so it costs almost nothing to turn on — but it is a
    // line a second forever, which is why it is asked for rather than assumed.
    /// Print frame statistics once a second: fps, mean and worst draw time, mean and worst interval.
    ///
    /// `KM_FRAME_STATS=1` does the same where there is no command line.
    #[arg(long)]
    frame_stats: bool,

    // The machine is normally started by double-clicking it and `karaokemachine.exe` is
    // GUI-subsystem, so there is no console and every line is discarded — which is why "it did not
    // start and I do not know why" had no answer short of finding a terminal.
    /// Also write this run's log to a file in the data directory's `logs` folder.
    ///
    /// The file holds the same output as the console, uncolored and timestamped. One file per run,
    /// the ten newest kept. `--show-paths` says where they are.
    ///
    /// `KM_LOG_FILE=1` does the same where there is no command line, such as a systemd unit or
    /// Android.
    #[arg(long)]
    log_file: bool,

    /// How many runs' logs to keep: a number, or `all` to keep every one of them.
    ///
    /// Naming a count turns the log file on by itself, so a machine being worked on needs this and
    /// nothing else. The names carry the date and time the run started, so `all` leaves that
    /// machine's whole history in the folder in the order it happened. Crash reports are counted
    /// separately and kept to the same number.
    ///
    /// `KM_LOG_KEEP` does the same where there is no command line.
    #[arg(long, value_name = "COUNT", value_parser = km_logfile::parse_keep)]
    log_keep: Option<usize>,

    // The destination a person watches while the run happens, where the file above is the one they
    // go and find afterwards. It replaces the console rather than joining it: both would print
    // every line twice.
    /// Send this run's log to the ECAppLog viewer instead of the console.
    ///
    /// Give an address to reach a viewer on another computer:
    /// `--ecapplog=192.168.1.x:13991`. The viewer does not have to be running yet — lines wait for
    /// it and arrive when it opens.
    ///
    /// `KM_ECAPPLOG=1` does the same for every program at once, and `logging.ecapplog` in
    /// `settings.json` does it for every run of this machine.
    ///
    /// `--log-file` and the machine's own log route are unaffected.
    #[arg(
        long,
        value_name = "ADDR",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = km_ecapplog::DEFAULT_ADDRESS,
        value_parser = km_ecapplog::parse_address,
    )]
    ecapplog: Option<String>,

    /// Print more detail. Repeat for more: `-v` for this app, `-vv` for everything.
    ///
    /// `RUST_LOG` overrides this entirely when it is set, and `logging.level` in settings.json says
    /// the same thing for a machine nobody types a command line at.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

impl Cli {
    /// The tracing filter this verbosity asks for.
    ///
    /// **The default is plain `info`.** A default of `info,km_app=debug` would make every shipped
    /// build emit this crate's whole debug stream — a frame report every second, a line per unbound
    /// keypress, the back-button commentary — into a Windows console or the appliance's journal.
    /// Debug output is a developer's, and a release that produces it by default has decided on the
    /// owner's behalf that they wanted it.
    ///
    /// `RUST_LOG` wins when it is set, which is what makes a one-off "show me everything from the
    /// HTTP layer" possible without adding a flag for it.
    ///
    /// **`logging.level` is the rung below `-v` and speaks `RUST_LOG`'s grammar**, because the
    /// machine that most needs the level changed is the box under a television, where there is no
    /// command line to type a flag on and no unit file to put a variable in. It replaces the ladder
    /// rather than moving along it, exactly as `RUST_LOG` does: a rung is a fixed pair of names and
    /// a directive is the thing that can say `km_api=trace` and nothing else. `-v` beats it, so one
    /// run typed at a keyboard is never an edit to the machine.
    ///
    /// Dependencies that talk at `info` are quietened by name — see [`QUIET_DEPENDENCIES`]. A
    /// directive stands alone, for `RUST_LOG`'s reason: somebody naming their own targets has said
    /// what they want to hear.
    fn log_filter(&self, settings: &LoggingSettings) -> String {
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

    /// What `--fullscreen` and `--windowed` between them asked for, or `None` for neither.
    ///
    /// **Two flags folded into one answer**, for the reason `--port` is folded into `--api-bind`
    /// where `Options` is built: two bools can say four things, only three of them mean anything,
    /// and clap has already refused the fourth. The one that matters most is `None` — "nobody
    /// said", which is what leaves `display.fullscreen` in charge and is a state a pair of bools
    /// has no room for once they are read separately.
    ///
    /// A method rather than a `match` at the call site so the test below asks the same question
    /// `main` does, instead of a copy of it that can drift.
    fn fullscreen_asked(&self) -> Option<bool> {
        match (self.fullscreen, self.windowed) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }
}

/// This library's own tracing target, asked of cargo rather than typed out.
///
/// **`km_app`, not `karaokemachine` — the package name is not the target.** The package is named
/// `karaokemachine` but the library is `km_app`, and a tracing target is the module path, which is
/// rooted at the *library* name. `karaokemachine=debug` therefore matches nothing at all. It cost a
/// round of Android debugging to notice, because the symptom is silence rather than an error; see
/// the same note in `lib.rs`.
///
/// Taking it from `CARGO_CRATE_NAME` is what stops that happening a second time. A filter naming a
/// target that does not exist is not an error anywhere — not at compile time, not at run time, not
/// in a test that asserts the string — so a crate rename that missed this line would quietly turn
/// `-v` into a flag that does nothing. The test below still pins the literal, deliberately: derived
/// here and spelled there, a rename fails loudly in exactly one place instead of silently in none.
const SELF_TARGET: &str = env!("CARGO_CRATE_NAME");

/// What this program calls itself to the ECAppLog viewer, which labels each connection with it.
///
/// **Not [`APP_NAME`], which is the file stem.** That one names a log file and a crash report, so it
/// is lowercase and safe in a path; this one is read by a person picking one of several connected
/// programs out of a list. It is the `FileDescription` string `build.rs` gives Windows, and the same
/// division: one product, and this is which program in it.
const VIEWER_NAME: &str = "KaraokeMachine";

/// Dependencies whose own `info` output is noise here, held down to warnings.
///
/// **This is the `What a shipped build says out loud` decision applied to somebody else's crate.**
/// That decision is about the machine being quiet by default and diagnostics being asked for by
/// name, and it is just as true of a library that announces itself. `symphonia`'s MP3 demuxer logs
/// "estimating duration from bitrate" at `info` **every time a file is opened** — which for this
/// application is every time somebody sings — and it is not even a fact about this machine's
/// behavior: the duration it is talking about is one we deliberately do not use, because a real
/// corpus file overstates its own length sevenfold.
///
/// Named individually rather than as a blanket `warn` for everything-but-us, because the interesting
/// case is the opposite one: when `cpal` or `symphonia` has something to say at `info` about a
/// device or a file that will not open, that belongs in the journal. Only the crates measured to be
/// chatty are listed, and the `-vv` ladder above drops this entirely.
const QUIET_DEPENDENCIES: &str = "symphonia_bundle_mp3=warn,symphonia_core=warn";

/// Starts the tracing subscriber, formatted for wherever the output is actually going.
///
/// Two things are decided here beyond the level, and they are decided separately because they are
/// not the same question:
///
/// * **Color** follows whether stdout is a terminal. Down a pipe or into a file, ANSI escapes are
///   bytes somebody has to read around; in a terminal they stay. It answers false for a
///   GUI-subsystem process launched from Explorer, which has no stdout at all — and answering that
///   question is all it does there, because `is_terminal` reads a handle rather than writing to it.
///   The Windows portable build does not open a console of its own; see the
///   `Two executables on Windows` decision in `docs/decisions/`.
/// * **Timestamps** are dropped only when systemd has connected stdout to the journal, which it
///   announces by setting `JOURNAL_STREAM`. journald stamps every line it receives, so ours was the
///   second one on every line of the appliance's log. Deliberately *not* keyed on "is a terminal":
///   a plain `> log.txt` is not a terminal either and very much wants the time.
///
/// The two are boxed rather than assigned to one variable because `.without_time()` changes the
/// layer's type.
///
/// **The writer stays stdout even when there is nowhere to write, and `log_internal_errors` must
/// never be turned on.** The first half is free: `tracing_subscriber` discards its writer's errors,
/// which is the property [`km_console::say`] is built on, so a GUI-subsystem run with a null handle
/// loses its log lines silently and costs nothing. The second half is the trap — that setting reports
/// a failed write with `eprintln!`, which *panics* when there is no stderr either. Switching it on
/// would turn every lost log line in a double-clicked build into an abort, which is the same fault
/// `say` exists to prevent, arriving by the one door `say` does not guard. It is the reason
/// [`km_logfile`] can hand a file to the same subscriber safely: a full disk costs log lines.
///
/// # The file, when one is asked for
///
/// `--log-file` (or `KM_LOG_FILE`) adds a **second layer** rather than a second writer, and the
/// difference is the whole point: the console goes on printing exactly what it printed before, and
/// the file gets the same events formatted for a file — never colored, and timestamped even under
/// systemd, which drops the console's clock because journald supplies one and a file has nobody to
/// supply it.
///
/// The file is opened *before* the subscriber exists, so a failure has nowhere to be reported yet;
/// it is carried past `init` and warned about through the console layer that has just been
/// installed. A log file that will not open is not a reason to refuse to start a karaoke machine.
///
/// # The tap, which nobody asks for
///
/// A third layer, and the only one of the three that is unconditional. [`km_logtap`] keeps the most
/// recent records in memory so that `GET /admin/logs` and the pane on `/dev/` can show them, and it
/// is always there because the run that most needs a log is the one nobody armed: by the time
/// somebody wants the last hundred lines it is too late to start keeping them. What that costs is a
/// bounded ring, which is why it can be paid on every run.
///
/// It rides this same filter, exactly as the file does — a level says how much detail and where the
/// detail goes is a different question, which here has no switch at all. Holding records publishes
/// nothing: the routes that serve them are admin routes, and a program mounting none keeps its
/// buffer to itself.
///
/// **It cannot reach the trap above.** The tap is not a `fmt` layer, has no `MakeWriter` and writes
/// to no handle, so there is no failed write for `log_internal_errors` to turn into an `eprintln!`
/// on a null stderr.
///
/// # The viewer, which takes the console's place
///
/// `--ecapplog` sends the same events to the [ECAppLog](https://github.com/RangelReale/ecapplog)
/// viewer, live, with a tab per crate and a level to colour by. It is the one destination here that
/// *displaces* another: the console layer is not built at all when it is on, because a window
/// offering a filter is a better console than a console and two of them would print every line
/// twice. The file and the tap are untouched.
///
/// It rides this same filter for the reason the two above do. The viewer need not be running — lines
/// queue and arrive when it opens — so nothing here waits on it and nothing fails when it is absent;
/// see [`km_ecapplog`].
///
/// # The crash report, which nobody asks for
///
/// Neither destination above reaches a panic: the default hook writes to stderr, which is the same
/// null handle stdout is here, and it does not travel through `tracing` — so a run with `--log-file`
/// on has a log that stops mid-sentence at the last ordinary event and looks complete. The hook goes
/// in whatever the flags say, because a person cannot decide in advance to record the one event they
/// will want. Nothing is written until a panic happens.
fn init_logging(cli: &Cli, paths: &Paths) -> Option<km_logfile::LogFile> {
    // **Three sources, narrowest first**: what was typed, then what the environment was given, then
    // what this machine was set up to do. The order is the one every other flag here uses -- a run
    // says what this run does, and neither a variable nor a settings file may override it.
    //
    // Naming a count is asking for a history, and a history nobody is writing is not one -- so
    // `--log-keep all` alone is enough, and `--log-file` beside it is allowed rather than required.
    let settings = Settings::peek_logging(paths);
    let keep = km_logfile::keep_wanted(cli.log_keep).or_else(|| settings.keep());
    let wanted = km_logfile::asked_for(cli.log_file) || settings.file || keep.is_some();
    let keep = keep.unwrap_or(km_logfile::KEEP);
    // Installed before the file is opened, so a panic while opening it is still reported. The hook
    // needs no subscriber: it writes its own file, and the event it also emits is a bonus for
    // whoever has a console.
    km_logfile::report_panics(paths.logs_dir(), APP_NAME, keep);
    let (file, failure) =
        match wanted.then(|| km_logfile::LogFile::open_keeping(paths.logs_dir(), APP_NAME, keep)) {
            Some(Ok(file)) => (Some(file), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };

    // **The viewer takes the console's place rather than standing beside it**, which is the one
    // destination here that displaces another: a window with a tab per crate and a level to filter
    // on is a better console than a console, and two of them would print every line twice. The file
    // and the tap are untouched, because how much detail there is and where it goes are different
    // questions -- see `A log that goes to a viewer instead of a console` in docs/decisions/.
    //
    // **Three sources, narrowest first**, the same ladder the file above climbs and for the same
    // reason: a machine being worked on is a standing state rather than an evening, and neither a
    // variable nor a settings file may override what this run was told to do.
    let (viewer, viewer_failure) = match km_ecapplog::asked_for(cli.ecapplog.as_deref()) {
        Ok(asked) => (asked.or_else(|| settings.ecapplog()), None),
        Err(reason) => (None, Some(reason)),
    };
    let viewer = viewer.map(|address| km_ecapplog::EcAppLog::open(&address, VIEWER_NAME));
    let ansi = std::io::stdout().is_terminal();
    let console = viewer.is_none().then(|| {
        if std::env::var_os("JOURNAL_STREAM").is_some() {
            tracing_subscriber::fmt::layer()
                .with_ansi(ansi)
                .without_time()
                .boxed()
        } else {
            tracing_subscriber::fmt::layer().with_ansi(ansi).boxed()
        }
    });

    // Resolved once and used twice: the filter decides what every layer below keeps, and the tap
    // carries the directive so that a surface drawing records can say what this run is keeping.
    let filter = cli.log_filter(&settings);
    let tap = km_logtap::LogTap::new().with_filter(filter.clone());

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(filter))
        .with(console)
        .with(viewer.as_ref().map(|viewer| viewer.layer()))
        .with(file.as_ref().map(|file| file.layer()))
        .with(tap.layer())
        .init();

    if let Some(viewer) = viewer {
        // Remembered rather than handed along, for `km_logtap::install`'s reason one line down: the
        // place a run ends is not the place it built its subscriber, and `SDL_main` has no argument
        // to put a handle in either.
        km_ecapplog::install(viewer);
    }

    // Remembered here rather than handed along, because `SDL_main` has no argument to put it in.
    km_logtap::install(tap);

    // Said through the console layer that has just been installed, which is there precisely because
    // the viewer this names could not be opened.
    if let Some(reason) = viewer_failure {
        tracing::warn!(%reason, "could not read where the ECAppLog viewer is; this run's log goes to the console");
    }
    if let Some(error) = failure {
        tracing::warn!(
            dir = %paths.logs_dir().display(),
            %error,
            "could not open a log file; this run's log goes to the console only"
        );
    }
    if let Some(file) = &file {
        // At `info`, so that asking for the file is enough to be told where it went — and into the
        // file itself, which is how a log somebody sends on says which run it was.
        tracing::info!(path = %file.path().display(), "writing this run's log here");
    }
    file
}

/// Which of the two executables is running.
///
/// **Not a flag**, exactly as it is not one in `km-package-builder` and `km-remote`: a person
/// does not choose this, they choose which icon to double-click or which name to type. It exists for
/// one question — whether a console this process was handed is one it wanted — and the machine is
/// the last of the three programs in this shape to be asked it.
///
/// It carries no `#[cfg]`. Off Windows the two executables are the same program under two names, and
/// answering honestly there costs nothing because `km-console`'s non-Windows half ignores it.
pub enum Shell {
    /// `karaokemachine` — GUI-subsystem on Windows, so it is never given a console at all.
    Windowed,
    /// `karaokemachine-console` — the one to type, and the only way to reach `--set-password`.
    Console,
}

/// Everything the two executables have in common, which is everything except the subsystem.
///
/// The order of the first lines is load-bearing and is the same order `km-package-builder`
/// settles on. `Cli::parse` is first because `--help` and `--version` are answered inside it and
/// exit; the subscriber is next, so that anything the rest of this says has somewhere to go; and
/// [`km_console::decide_where_to_talk`] is third, because it may hand back a console nobody asked for
/// and must run before the first [`say`] and not after it.
///
/// **[`Paths`] moved in front of the subscriber**, which is a change to that order and the one thing
/// to preserve about it: `--log-file` writes into the data directory, so where the data directory is
/// has to be settled before there is anywhere to write. It is safe to move because path discovery
/// says nothing — it reads the platform's directories and the executable's own folder and logs
/// none of it — so nothing is lost to the subscriber not existing yet.
pub fn main(shell: Shell) -> anyhow::Result<()> {
    let cli = Cli::parse();

    let paths = match &cli.data_dir {
        // `data_rooted_at` and NOT `rooted_at`: this flag moves the data, and assets are not data.
        // They ship with the build rather than accumulating with use, so a scratch run reads the
        // same SoundFont, font and wallpapers as any other -- which is what stops `--data-dir` from
        // quietly producing a test tone over a plain gradient.
        Some(dir) => Paths::data_rooted_at(dir),
        None => Paths::discover(),
    };

    // Held for the life of the run. The subscriber has a clone of its own, so this is not what keeps
    // the file open -- it is what lets `--show-paths` say whether this run is writing one.
    let log_file = init_logging(&cli, &paths);
    // Drains the viewer's queue on the way out, whichever of this function's many exits is taken.
    // Nothing happens without `--ecapplog`; with it, the alternative is losing the last lines of
    // every run, which are the ones somebody watching is there for.
    let _drain = km_ecapplog::flush_on_drop();
    // Which sink the lines below have, and the point at which an unwanted console is let go of. The
    // machine is normally started by double-clicking it or by systemd, neither of which is reading;
    // the flags that print are typed, and those runs have a console or a pipe.
    //
    // **The twin keeps whatever console it was given.** `karaokemachine.exe` is GUI-subsystem on
    // Windows and so is never handed one — the count is zero and this arm cannot fire for it — which
    // is why the answer here is honest rather than merely harmless: get the two arms the wrong way
    // round and the only executable freeing a console is the one whose whole job is to print.
    km_console::decide_where_to_talk(match shell {
        Shell::Windowed => km_console::Console::NotWanted,
        Shell::Console => km_console::Console::Wanted,
    });

    // First in the chain, and deliberately ahead of `Settings::load` below: registering a file type
    // touches nothing of the machine's own, and loading settings *writes* a `settings.json` for a
    // machine that has never run. Somebody who registers before their first start should not have a
    // data directory made for them by it. Mirrors `km-package-builder`'s own ordering.
    if cli.register {
        return crate::register::register();
    }
    if cli.unregister {
        return crate::register::unregister();
    }

    if cli.show_paths {
        say(format!("settings   {}", paths.settings_file().display()));
        say(format!("catalog  {}", paths.library_file().display()));
        // The answer to "where do I put my songs?", which is why it is printed even before it
        // exists: the machine makes it on its first proper start, and somebody reading this may not
        // have had one yet.
        let packages = paths.packages_dir();
        say(format!(
            "packages   {}{}",
            packages.display(),
            if packages.is_dir() {
                ""
            } else {
                "   (not made yet — start the machine once)"
            }
        ));
        // The second, public folder — Android only, and printed only where there is one. It matters
        // more than its one line suggests: it is the folder somebody can actually reach with a file
        // manager or `adb push`, and nobody would guess
        // `/storage/emulated/0/Android/data/<package>/files/packages` unaided.
        if let Some(extra) = paths.packages_dirs().get(1) {
            say(format!(
                "packages   {}{}   (shared — put songs here)",
                extra.display(),
                if extra.is_dir() {
                    ""
                } else {
                    "   (not made yet — start the machine once)"
                }
            ));
        }
        // ...and any the owner named in `package_dirs`. Read straight off the file rather than from
        // `paths`, because a `Paths` here has never been joined to a settings file — see
        // `peek_settings`. Same question as the two lines above ("where do I put my songs?"), and a
        // folder the owner added is exactly the one they may be checking the spelling of.
        //
        // **Each says it can be deleted from**, because naming a folder here does two things and
        // only one of them is obvious. `Paths::is_mine_to_delete` treats every scanned folder as the
        // machine's own territory, so uninstalling a package removes its file from one of these as
        // readily as from the machine's own folder. Somebody who pointed this at a wide folder is
        // owed the sentence.
        for dir in settings_package_dirs(&paths) {
            say(format!(
                "packages   {}   (from settings.package_dirs — uninstall deletes from here){}",
                dir.display(),
                if dir.is_dir() {
                    ""
                } else {
                    "   (missing — nothing will be scanned here)"
                }
            ));
        }
        // The extra files `debug.packages` names, which are not in any of the folders above and are
        // therefore the one thing the lines above cannot account for. "Where did that package come
        // from?" has no other answer.
        //
        // **Prefixed rather than a fourth `packages` line**, and that is not cosmetic:
        // `tools/platform/linux/deploy.sh` reads this output with an `awk` matching `/^packages/`,
        // so a new line starting with that word would be taken for a folder to copy songs into.
        for file in settings_debug_packages(&paths) {
            say(format!(
                "debug pkg  {}   (from debug.packages){}",
                file.display(),
                if file.is_file() {
                    ""
                } else {
                    "   (missing — this will be reported at every start)"
                }
            ));
        }
        // The wallpapers, which this command did not name at all until the owner got a folder of
        // their own to put some in. Two lines rather than one, and the second is the useful half:
        // the first says what is on screen, the second says where to put something else. "I dropped
        // my pictures in and nothing changed" is almost always a folder the machine was never going
        // to look in, and that is a question no other output here can answer.
        //
        // `wallpaper.dir` in settings beats all of this, so say so rather than printing a resolved
        // path that a setting is quietly overriding.
        //
        // Asked of `WallpaperSettings::folder` rather than restated here, which is what makes the
        // claim that there is *one* definition of this rule true: `Settings::wallpaper_config` and
        // the display loop's re-resolution ask the same function.
        let wallpaper = peek_settings(&paths)
            .map(|settings| settings.wallpaper)
            .unwrap_or_default();
        let (shown, source) = wallpaper.folder(&paths);
        say(format!(
            "wallpapers {}   ({})",
            shown.display(),
            source.describe()
        ));
        // Where to put your own, unless that is already what is being shown — and suppressed for a
        // folder named by the setting too, because that beats this one outright, so pointing at it
        // would be an invitation the machine would then ignore.
        if !matches!(source, WallpaperSource::Owner | WallpaperSource::Setting) {
            let own = paths.wallpapers_dir();
            say(format!(
                "           {}   ({})",
                own.display(),
                if own.is_dir() {
                    "drop your own in here — they replace the set above"
                } else {
                    "not made yet; start the machine once, then drop your own in here to \
                     replace the set above"
                }
            ));
        }
        // The extra files `debug.wallpapers` names, shown *as well as* whatever folder won and so
        // not accounted for by the lines above. Prefixed for the same reason the packages one is:
        // `tools/platform/linux/deploy.sh` reads this output with an awk anchored on a word.
        for file in wallpaper_extras(&paths) {
            say(format!(
                "debug wall {}   (from debug.wallpapers){}",
                file.display(),
                if file.is_file() {
                    ""
                } else {
                    "   (missing — it will simply not appear)"
                }
            ));
        }
        // Worth printing, and worth marking when it is not there: a missing asset directory is
        // exactly why a machine plays a test tone instead of instruments and shows a gradient
        // instead of a wallpaper, and it is not otherwise visible from outside.
        let assets = &paths.asset_dir;
        say(format!(
            "assets     {}{}",
            assets.display(),
            if assets.is_dir() {
                ""
            } else {
                "   (does not exist — run tools/setup/fetch-assets.sh)"
            }
        ));
        // Only ever present in a checkout, and the only visible sign that the machine is playing a
        // developer's own bank rather than the bundled one. Nothing deletes this folder -- see the
        // `Local assets in a checkout` decision -- so naming it is what makes `rm -rf` an option
        // somebody can act on rather than a directory they have to go looking for.
        if let Some(overlay) = &paths.overlay_asset_dir {
            say(format!(
                "overlay    {}   (local and gitignored — files here are used instead of the bundled ones)",
                overlay.display()
            ));
        }
        // **Which bank actually won**, which the three lines above only imply. It was missing for
        // long enough to be worth a sentence: `assets` and `overlay` name directories, and the
        // answer to "why does this sound wrong?" is a file — chosen by `audio.soundfont` if that is
        // set, and otherwise by the first of `SOUNDFONT_SUBPATHS` that exists across both
        // directories. `resolve_soundfont` is that rule, so it is called rather than restated.
        let configured = peek_settings(&paths).and_then(|s| s.audio.soundfont);
        let selected = crate::soundfont::resolve(&paths, configured.as_deref());
        match crate::engine::resolve_soundfont(selected.path.as_deref(), &paths) {
            Ok(bank) => {
                let note = crate::soundfont::read(&paths);
                say(format!(
                    "soundfont  {}   ({})",
                    bank.display(),
                    match (&configured, &selected.missing, &note) {
                        // The case that most needs saying: a name that matches no bank. The
                        // machine is playing the bundled one and the picker agrees, but the setting
                        // still says otherwise and only this can point at the gap.
                        (Some(id), Some(_), _) => {
                            format!("bundled — audio.soundfont says \"{id}\", which is not there")
                        }
                        (Some(id), None, Some(note)) => format!(
                            "audio.soundfont {id}; music_volume {}, was {}",
                            note.applied_music_volume, note.previous_music_volume
                        ),
                        (Some(id), None, _) => format!("audio.soundfont {id}"),
                        (None, _, _) => "bundled".to_owned(),
                    }
                ));
            }
            // The failing case is the one this line is most worth printing for: a machine with no
            // bank plays a sine test tone, and nothing else here says so.
            Err(error) => say(format!("soundfont  none — {error}")),
        }
        // And where to put another one, which is the wallpapers block's second line made again for
        // the same reason: the line above says what is playing, this one says where to put something
        // else. Printed always rather than only when the bundled bank won, because unlike the
        // wallpapers case there is no "these are already yours" state that would make it noise.
        //
        // **Every folder that is scanned, not just the first.** On Android there are two, and the
        // second is the only one anybody can copy a file into — a report that named only the private
        // directory would be answering "where do I put my banks?" with the one path that cannot be
        // written to. See [`crate::settings::Paths::soundfonts_dirs`].
        for banks in paths.soundfonts_dirs() {
            say(format!(
                "           {}   ({})",
                banks.display(),
                if banks.is_dir() {
                    "drop your own banks in here — they can be chosen from the remote"
                } else {
                    "not made yet; start the machine once, then drop your own banks in here"
                }
            ));
        }

        // **A bank a setup program asked for and the machine has not fetched yet**, printed only
        // while there is one. It is the answer to a question nothing else here can be asked: a first
        // start that is about to spend several minutes downloading 261.9 MiB looks, from outside,
        // like a machine doing nothing in particular. Naming the file is also the way to call it off
        // — deleting it is the undo, and it is not a path anybody would guess.
        if let Some(request) = crate::firstrun::read(&paths) {
            say(format!(
                "requested  {}   ({})",
                crate::firstrun::file(&paths).display(),
                match request.resolve() {
                    Some(row) => format!(
                        "the next start will download {} — {}, {} attempt(s) so far",
                        row.name, row.size, request.attempts
                    ),
                    None => format!("'{}' is not a bank this build knows", request.bank),
                }
            ));
        }

        // **And where a log would go, with whether this run is writing one.** The folder is the
        // answer to "send me the log" and the parenthesis is the answer to the question that comes
        // straight after it, which is why one line carries both: a folder named with nothing in it
        // reads as a fault rather than as an option nobody has turned on.
        say(format!(
            "logs       {}   ({})",
            paths.logs_dir().display(),
            match &log_file {
                Some(file) => format!(
                    "this run is writing {}",
                    file.path()
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ),
                // Three ways in, and the settings key is named last because it is the one that
                // outlives the run — somebody reading this line wants the cheapest answer first.
                None =>
                    "off — ask with --log-file, KM_LOG_FILE=1, or logging.file in settings.json"
                        .to_owned(),
            }
        ));

        // **And the viewer, only when there is one.** It has no folder to name, so unlike the line
        // above there is nothing to print when it is off — a row saying "off" for a destination with
        // no location is an answer to a question nobody asked.
        if let Some(address) = km_ecapplog::address() {
            say(format!(
                "viewer     ECAppLog at {address}   (this run's log, in place of the console)"
            ));
        }
        return Ok(());
    }

    // **Before `Settings::load`, and it is the only one-shot that needs to be.** It reads the
    // catalog and nothing else, and loading settings would *write* a `settings.json` as a side
    // effect — so `--song-book` against a fresh `--data-dir` would leave an install behind for a
    // command that only wanted to print a list.
    if let Some(out) = &cli.song_book {
        return write_song_book(&paths, out, cli.book_name.as_deref());
    }

    let (mut settings, _) = Settings::load(&paths);

    if cli.list_audio_devices {
        let saved = settings.audio.output_device.as_deref();
        match km_audio::device::list_outputs(saved) {
            Ok(devices) => {
                // Measured rather than guessed: an ALSA pcm name is around thirty characters and a
                // WASAPI one is a GUID nearer sixty, so any fixed column is wrong on one platform.
                // Over every device, so the two blocks below line up with each other.
                let width = devices.iter().map(|d| d.id.len()).max().unwrap_or(0);
                let line = |device: &km_audio::device::OutputDevice| {
                    let mut marks = Vec::new();
                    if device.usb {
                        marks.push("usb");
                    }
                    if device.system_default {
                        marks.push("the system default today");
                    }
                    if saved == Some(device.id.as_str()) {
                        marks.push("saved");
                    }
                    if !device.available {
                        marks.push("not present");
                    }
                    say(format!(
                        "{:<width$}  {}{}",
                        device.id,
                        device.name,
                        if marks.is_empty() {
                            String::new()
                        } else {
                            format!("   [{}]", marks.join("] ["))
                        }
                    ));
                };

                // One row per physical output first. Nothing is hidden from an operator -- this is
                // the appliance's only way to see the list at all, and a diagnostic that omits
                // something is worse than a long one -- but the useful lines have to come first, or
                // the answer to "which one is my headphone jack" is thirty rows down.
                //
                // No `[preferred]` mark: the block a row is in already says it, and a mark that is
                // true of every line in a block is noise.
                for device in devices.iter().filter(|device| device.preferred) {
                    line(device);
                }
                if devices.iter().any(|device| !device.preferred) {
                    say(
                        "\nOther names for those same outputs. ALSA spells one output several ways \
                         and gives every\nspelling the card's own description, so the list above is \
                         one row per output and each of\nthese is another way of saying one of \
                         them. Any of them can be saved; the ones above are\nthe ones worth saving.",
                    );
                    for device in devices.iter().filter(|device| !device.preferred) {
                        line(device);
                    }
                }
                if saved.is_none() {
                    // Nothing is marked because nobody has chosen, and an operator left to wonder is
                    // what this line prevents. It deliberately does not say the machine records
                    // its pick: the preference is re-applied every start, so the line stays true
                    // tomorrow.
                    say(
                        "\nNothing chosen yet, so the machine follows the system — on Linux, \
                         preferring a USB\ninterface if there is one. It picks again every start \
                         until you choose here.",
                    );
                }
            }
            Err(error) => say(format!("no audio devices could be listed: {error}")),
        }
        return Ok(());
    }

    if let Some(password) = &cli.set_password {
        settings.api.admin_password_hash = Some(
            km_api::AdminAuth::hash_password(password)
                .map_err(|error| anyhow::anyhow!("could not hash the password: {error}"))?,
        );
        // Cleared in the same write as the hash: a machine holding a PIN it no longer answers to
        // would go on showing it on its own screen.
        settings.api.admin_factory_pin = None;
        settings.save(&paths)?;
        say(
            "Admin password changed. Everything under /api/v1/admin/ now wants it, and\n\
             every signed-in phone and browser has been signed out.\n\
             The password itself is not stored — only an argon2 hash of it.",
        );
        return Ok(());
    }
    if cli.reset_password {
        let pin = km_api::auth::generate_factory_pin();
        settings.api.admin_password_hash = Some(
            km_api::AdminAuth::hash_password(&pin)
                .map_err(|error| anyhow::anyhow!("could not hash the password: {error}"))?,
        );
        settings.api.admin_factory_pin = Some(pin.clone());
        settings.save(&paths)?;
        say(format!(
            "The admin password is now {pin}, and the machine shows it on its own screen\n\
             until you change it. Every signed-in phone and browser has been signed out."
        ));
        return Ok(());
    }
    if cli.reset_sessions {
        settings.api.session_epoch = settings.api.session_epoch.saturating_add(1);
        settings.save(&paths)?;
        say(
            "Every admin session has ended. The password is unchanged — sign in again\n\
             wherever you need to.",
        );
        return Ok(());
    }

    if let Some(name) = &cli.set_name {
        let name = km_api::discover::tidy_name(name)
            .ok_or_else(|| anyhow::anyhow!("a machine's name cannot be blank"))?;
        settings.machine.name = name.clone();
        settings.save(&paths)?;
        say(format!(
            "This machine is now called \"{name}\".
             That is what a phone shows in its list of machines, and what the network advert says."
        ));
        return Ok(());
    }

    if let Some(bank) = &cli.set_soundfont {
        return set_soundfont(&paths, &mut settings, bank, cli.music_volume);
    }
    if cli.clear_soundfont {
        return clear_soundfont(&paths, &mut settings);
    }
    if let Some(bank) = &cli.first_run_soundfont {
        return first_run_soundfont(&paths, bank);
    }
    if let Some(banks) = &cli.set_debug_soundfonts {
        return set_debug_soundfonts(&paths, &mut settings, banks);
    }
    if cli.clear_debug_soundfonts {
        return clear_debug_soundfonts(&paths, &mut settings);
    }
    if cli.show_debug_soundfonts {
        return show_debug_soundfonts(&settings);
    }
    if let Some(packages) = &cli.set_debug_packages {
        return set_debug_packages(&paths, &mut settings, packages);
    }
    if cli.clear_debug_packages {
        return clear_debug_packages(&paths, &mut settings);
    }
    if cli.show_debug_packages {
        return show_debug_packages(&settings);
    }

    // Last in the chain, and it is the only one of these that does not always end the process. It
    // needs settings, because deciding whether a machine is already running here means knowing which
    // port to ask -- so it cannot go with `--register` at the top.
    if let Some(package) = cli.package.clone() {
        match crate::handed::take(&paths, &settings, &package)? {
            crate::handed::Next::Done => return Ok(()),
            // Falls through to `crate::run` below rather than returning: a document open is an
            // application launch, and the startup scan installs what is now in the folder.
            crate::handed::Next::StartNormally => {}
        }
    }

    // Resolved here rather than by clap, because a bare port means "the interface settings already
    // name" and clap parses arguments long before there are any settings to ask.
    //
    // `--port` is folded into `--api-bind` rather than carried beside it, so there is one path to
    // the bound address and not two that could disagree. clap has already refused the two together.
    let api_bind = match (&cli.api_bind, cli.port) {
        (Some(value), _) => Some(resolve_api_bind(value, settings.api.socket_addr())?),
        (None, Some(port)) if cli.lan => {
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))
        }
        (None, Some(port)) => Some(SocketAddr::new(settings.api.socket_addr().ip(), port)),
        (None, None) => None,
    };

    // Read before the struct below moves `cli.play` out from under it.
    let fullscreen = cli.fullscreen_asked();

    let outcome = crate::run(
        paths,
        settings,
        Options {
            play: cli.play,
            headless: cli.headless,
            stream: cli.stream,
            fullscreen,
            frame_stats: cli.frame_stats,
            api_bind,
            dev_remote: cli.dev_remote,
        },
    );
    // **Logged as well as returned, because otherwise a failed double-click says nothing anywhere.**
    // Returning `Err` from `main` prints `Error: ...` through `std::io::attempt_print_to_stderr`,
    // which — as the name says — does not panic when there is no stderr, and also does not print.
    // So the GUI-subsystem build's one report of why it would not start went to a null handle, while
    // the log, which had a subscriber and a journal behind it, was told nothing. That is the console
    // window's real loss and this is what pays for it: `-v` is now a diagnosis and not just detail,
    // and on the appliance the reason lands in the journal exactly as every other failure does.
    //
    // Still returned rather than swallowed: the exit code is what a script reads, and a run from a
    // terminal should print the error the ordinary way rather than have it whispered at `error!`.
    if let Err(error) = &outcome {
        tracing::error!("{error:#}");
    }
    outcome
}

/// The extra package folders the settings file names, on the same terms.
///
/// Empty covers all three of missing, unreadable and unparseable, which is what the caller wants to
/// print: nothing is adding a folder to the rules.
fn settings_package_dirs(paths: &Paths) -> Vec<PathBuf> {
    peek_settings(paths)
        .map(|s| s.package_dirs)
        .unwrap_or_default()
}

fn wallpaper_extras(paths: &Paths) -> Vec<PathBuf> {
    peek_settings(paths)
        .map(|s| s.debug.wallpapers)
        .unwrap_or_default()
}

fn settings_debug_packages(paths: &Paths) -> Vec<PathBuf> {
    peek_settings(paths)
        .map(|s| s.debug.packages)
        .unwrap_or_default()
}

/// The settings file as it stands, or `None` — **without loading it**.
///
/// The shared half of the two readers above, and the reason for both is in
/// [`settings_wallpaper_dir`]'s own doc: `--show-paths` must be able to say where things are without
/// bringing an install into being, and [`Settings::load`] writes.
fn peek_settings(paths: &Paths) -> Option<Settings> {
    let text = std::fs::read_to_string(paths.settings_file()).ok()?;
    serde_json::from_str::<Settings>(&text).ok()
}

/// `--set-soundfont`: check the bank opens, remember the level, write both.
///
/// The order matters and is the point of the function. Nothing is written until the bank has been
/// through the synthesizer, so a refused bank leaves the machine exactly as it was rather than
/// pointing it at a file that will produce a test tone at the next start.
fn set_soundfont(
    paths: &Paths,
    settings: &mut Settings,
    bank: &Path,
    volume: Option<f32>,
) -> anyhow::Result<()> {
    // Absolute, because a relative path in `settings.json` would be resolved against whatever
    // working directory the machine is next started in -- which for a double-click or a systemd unit
    // is not this one. The same reason `Paths::discover_asset_dirs` returns absolute paths.
    let bank = std::path::absolute(bank).unwrap_or_else(|_| bank.to_path_buf());
    if !bank.is_file() {
        anyhow::bail!("{} is not a file", bank.display());
    }
    if let Some(volume) = volume
        && !(0.0..=1.0).contains(&volume)
    {
        anyhow::bail!("--music-volume {volume} is outside 0.0 to 1.0");
    }
    let defects = match crate::soundfont::check_plays(&bank) {
        Ok(defects) => defects,
        // Named, and not fallen back from. This is the message people need: four of the
        // fifteen banks surveyed are refused by `rustysynth` where other players accept
        // them, and a bank that will not open is a fact about this synthesizer rather than about the
        // file being broken.
        Err(error) => anyhow::bail!(
            "{} will not play: {error}\n\
             The bank was not changed. This machine's synthesizer is stricter than most — see\n\
             crates/machine/km-banks/data/soundfont-banks.conf, which records for every\n\
             surveyed bank whether it opens.",
            bank.display()
        ),
    };

    let was = settings.audio.music_volume;
    let existing = crate::soundfont::read(paths);
    // **What the level would be with no override at all**, which is the right thing to fall back to
    // when the new bank asks for nothing. Switching a bank that needed 0.8 for a bank that does not
    // must not leave the first bank's reduction behind: that number was a property of the bank, not
    // a choice somebody made. `restore_to` is the same rule `--clear-soundfont` uses, including its
    // guard — a level edited by hand since is the owner's and is kept.
    let baseline = crate::soundfont::restore_to(existing.as_ref(), was).unwrap_or(was);
    let applied = volume.unwrap_or(baseline);
    let note = crate::soundfont::stash(existing, was, applied);

    // **Installed into the SoundFont folder, then chosen by id.** The setting names a bank rather
    // than a file now, and the folder is what says which banks there are — so a bank that stayed in
    // a download folder or a build cache would be chosen and then not found. This is the same
    // copy-then-install reasoning the drop route makes for a package, applied to a bank: taking a
    // file means putting it where the machine looks.
    let installed = install_bank(paths, &bank)?;
    let id = crate::soundfont::bank_id(&installed).with_context(|| {
        format!(
            "{} has no name a bank can be chosen by",
            installed.display()
        )
    })?;

    settings.audio.soundfont = Some(id.clone());
    settings.audio.music_volume = applied;
    settings.save(paths)?;
    crate::soundfont::write(paths, &note)?;

    say(format!("SoundFont set. {id} — {}", installed.display()));
    // Said here and not only in the journal, because this is the moment somebody is choosing a bank
    // and the only moment they are in a position to choose a different one. It is not a refusal: the
    // bank plays, and dropping the bad record instead of the file is the whole reason a bank with
    // defects loads at all. What it is not is something to discover from a song where an instrument
    // never arrives.
    if !defects.is_empty() {
        say(format!(
            "{defects}.\n\
             The bank plays and is now set. Records the synthesizer cannot make sense of are\n\
             dropped rather than costing the whole file, so an instrument may be missing."
        ));
    }
    if applied != was {
        say(match volume {
            Some(volume) => format!(
                "music_volume {volume} — this bank exceeds full scale at 1.0. Was {was}, and\n\
                 --clear-soundfont puts back {}.",
                note.previous_music_volume
            ),
            // Reached by switching away from a bank that wanted a reduction to one that does not.
            None => format!("music_volume back to {applied} — this bank needs no reduction."),
        });
    }
    say(
        "This is settings.json, so it reaches every build on this machine — a cargo run, a\nstaged folder and an installed one alike. Undo it with --clear-soundfont.",
    );
    Ok(())
}

/// Puts a bank into the SoundFont folder, so it can be chosen by id, and says where it went.
///
/// **Already in a scanned folder means leave it there**, which is the case `task soundfont` reaches
/// once a bank has been installed before, and the case an owner reaches by putting their own bank in
/// the folder and then naming it.
///
/// Hard-linked where the filesystem allows it and copied otherwise. A bank is 30–300 MiB and the
/// cache a development build fetches into is usually on the same volume as the data directory, so
/// the link costs nothing and no worktree carries its own copy; the copy is what crosses a volume,
/// which is the normal case on Android and for a `--data-dir` somewhere else.
///
/// **A different file already there under that name is refused, naming both**, and deliberately not
/// given a `-2` the way a dropped package is. A package is small enough that a second copy is a
/// tidying job; a bank is hundreds of megabytes and two confusable rows in the picker, and this
/// route has a terminal to say so on where a drop has nobody to talk to.
fn install_bank(paths: &Paths, bank: &Path) -> anyhow::Result<PathBuf> {
    let already = paths
        .soundfonts_dirs()
        .into_iter()
        .any(|dir| bank.parent() == Some(dir.as_path()));
    if already {
        return Ok(bank.to_path_buf());
    }

    let dir = paths.soundfonts_write_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("the SoundFont folder {} could not be made", dir.display()))?;
    let name = bank
        .file_name()
        .context("that bank has no file name")?
        .to_owned();
    let destination = dir.join(&name);

    if destination.exists() {
        // The same file under the same name is this command being run twice, which is fine.
        if crate::soundfont::same_file(&destination, bank) {
            return Ok(destination);
        }
        anyhow::bail!(
            "{} is already a different bank.\n\
             Nothing was changed. Rename one of them, or delete the one there:\n\
             {}",
            destination.display(),
            bank.display()
        );
    }

    // `.part` then rename, the same rule a dropped package follows: an interrupted copy must not
    // leave something in the folder that the next scan reads as a bank.
    let partial = destination.with_extension("part");
    if std::fs::hard_link(bank, &destination).is_err() {
        std::fs::copy(bank, &partial).map_err(|error| {
            let _ = std::fs::remove_file(&partial);
            anyhow::anyhow!(
                "{} could not be copied into {}: {error}",
                bank.display(),
                dir.display()
            )
        })?;
        std::fs::rename(&partial, &destination).map_err(|error| {
            let _ = std::fs::remove_file(&partial);
            anyhow::anyhow!(
                "{} could not be put in place: {error}",
                destination.display()
            )
        })?;
    }
    Ok(destination)
}

/// `--clear-soundfont`: back to the bundled bank, and back to the level that was found.
fn clear_soundfont(paths: &Paths, settings: &mut Settings) -> anyhow::Result<()> {
    let had = settings.audio.soundfont.take();
    let note = crate::soundfont::read(paths);
    let restored = crate::soundfont::restore_to(note.as_ref(), settings.audio.music_volume);
    if let Some(volume) = restored {
        settings.audio.music_volume = volume;
    }
    settings.save(paths)?;
    crate::soundfont::remove(paths)?;

    match had {
        Some(id) => say(format!("SoundFont cleared. Was {id}")),
        None => say("No sound bank was set; the bundled bank was already in use."),
    }
    match (restored, note) {
        (Some(volume), _) => say(format!("music_volume back to {volume}")),
        // Said out loud rather than silently left: somebody who tuned the level while an
        // override was in force should be told their number survived, not left wondering.
        (None, Some(_)) => say(format!(
            "music_volume left at {} — it has been changed by hand since, so it is yours\nrather than this command's to put back.",
            settings.audio.music_volume
        )),
        (None, None) => {}
    }
    Ok(())
}

/// `--first-run-soundfont`: ask the next start to fetch a bank, and fetch nothing here.
///
/// **The bank is resolved before anything is written**, which is the same discipline
/// [`set_soundfont`] keeps for a different reason: that one refuses a file that will not play, and
/// this one refuses a name nothing will ever be able to act on. A request naming a bank that is not
/// in the table would sit in the config directory being read and discarded at every start.
///
/// What it cannot check is whether the download will work, and that is the whole difference between
/// the two commands — this one is a note for a program that has not run yet.
fn first_run_soundfont(paths: &Paths, bank: &str) -> anyhow::Result<()> {
    let request = crate::firstrun::Request::new(bank);
    let Some(row) = request.resolve() else {
        anyhow::bail!(
            "there is no SoundFont called '{bank}'. Name one from `task soundfont:list`, or\n\
             `{}` for whichever the table recommends.",
            crate::firstrun::RECOMMENDED
        );
    };
    if row.url.is_none() {
        // The reason is the absence of a direct address, not anything about the terms -- which is
        // what this said until the terms column stopped carrying an editorial suffix that happened
        // to read as one. See `Where a bank may be fetched from` in docs/decisions/repository.md.
        anyhow::bail!(
            "{} has no direct download address, so the machine cannot fetch it.\n{}",
            row.id,
            match row.page {
                Some(page) => format!("Get it from {page}, then: --set-soundfont <the file>"),
                None => "Fetch it by hand, then: --set-soundfont <the file>".to_owned(),
            }
        );
    }
    crate::firstrun::write(paths, &request)?;

    say(format!(
        "The next start will download {} ({}) and play it.",
        row.name, row.size
    ));
    // The terms travel with the offer everywhere else this bank is named -- the tick box's Ready
    // page, the `.pkg`'s description, the row on the SoundFont page -- and a shell is not the place
    // to make an exception. See `Where a bank may be fetched from` in docs/decisions/repository.md.
    say(format!("License: {}", row.license));
    say(format!(
        "Three starts try before it gives up. Undo it by deleting\n{}",
        crate::firstrun::file(paths).display()
    ));
    Ok(())
}

/// One `--set-debug-soundfonts` argument, as it is written on the command line.
///
/// `<path>`, `<path>=<name>`, or `<path>=<name>=<volume>`. Split on `=` rather than taking three
/// flags because the three belong to one bank and nine banks would otherwise be twenty-seven
/// arguments in an order nothing could check.
fn parse_debug_bank(spec: &str) -> anyhow::Result<crate::settings::DebugBank> {
    let mut parts = spec.splitn(3, '=');
    let path = parts.next().unwrap_or_default().trim();
    if path.is_empty() {
        anyhow::bail!("'{spec}' names no file");
    }
    let path = PathBuf::from(path);
    // Absolute for the same reason `--set-soundfont` absolutises: settings are read by a process
    // whose working directory is not this one.
    let path = std::path::absolute(&path).unwrap_or(path);

    let name = match parts.next().map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_owned(),
        // The file's stem is a poor name and a fair default — `GeneralUser-GS` reads well enough,
        // and a per-bank cache folder's `gm.sf2` does not, which is why the script passes one.
        None => path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    };

    let music_volume = match parts.next().map(str::trim).filter(|v| !v.is_empty()) {
        Some(text) => {
            let volume: f32 = text
                .parse()
                .map_err(|_| anyhow::anyhow!("'{text}' in '{spec}' is not a level"))?;
            if !(0.0..=1.0).contains(&volume) {
                anyhow::bail!("the level {volume} in '{spec}' is outside 0.0 to 1.0");
            }
            Some(volume)
        }
        None => None,
    };

    Ok(crate::settings::DebugBank {
        path,
        name,
        music_volume,
    })
}

/// `--set-debug-soundfonts`: fill the switcher's slots, having opened every bank first.
fn set_debug_soundfonts(
    paths: &Paths,
    settings: &mut Settings,
    specs: &[String],
) -> anyhow::Result<()> {
    // Nine slots because there are nine keys. A tenth bank could be written and never reached,
    // which is a worse outcome than being told.
    const MAX_SLOTS: usize = 8;
    if specs.len() > MAX_SLOTS {
        anyhow::bail!(
            "{} banks were given and there are only {MAX_SLOTS} slots to put them in: Ctrl+1 is \
             always the bundled bank, so Ctrl+2 to Ctrl+9 are what is left.",
            specs.len()
        );
    }

    let mut banks = Vec::with_capacity(specs.len());
    let mut notes = Vec::new();
    for spec in specs {
        let bank = parse_debug_bank(spec)?;
        if !bank.path.is_file() {
            anyhow::bail!("{} is not a file", bank.path.display());
        }
        // Every one of them, before a single line is written. A partly written list would be a set
        // of keys that mostly work, which is the hardest kind to diagnose from in front of a room.
        match crate::soundfont::check_plays(&bank.path) {
            Ok(defects) if defects.is_empty() => {}
            Ok(defects) => notes.push(format!("  {} — {defects}", bank.name)),
            Err(error) => anyhow::bail!(
                "{} will not play: {error}\n\
                 No slot was changed. This machine's synthesizer is stricter than most — see\n\
                 crates/machine/km-banks/data/soundfont-banks.conf, which records for every\n\
                 surveyed bank whether it opens.",
                bank.path.display()
            ),
        }
        banks.push(bank);
    }

    let count = banks.len();
    settings.debug.soundfonts = banks;
    // A slot number means whatever the list under it says, so replacing the list makes a remembered
    // slot name a different bank. Clearing it is the only honest answer: the next start comes up on
    // slot 1, which is what somebody who has just re-chosen the slots would expect.
    settings.debug.soundfont_slot = None;
    settings.save(paths)?;

    say(format!(
        "SoundFont slots set: {count} bank{} in Ctrl+2 to Ctrl+{}.",
        if count == 1 { "" } else { "s" },
        count + 1
    ));
    say(
        "Ctrl+1 is the bundled bank and is always there. The name of whichever bank is\nplaying is drawn on screen for as long as any slot is filled.",
    );
    // The property that makes this safe to leave configured, said where somebody is deciding
    // whether to leave it configured.
    say(
        "Switching never writes audio.soundfont, so emptying the slots puts the machine back\non its own bank whatever you pressed. What it does remember is which slot you were\non, so a restart resumes the comparison. --clear-debug-soundfonts empties both.",
    );
    if !notes.is_empty() {
        say(format!(
            "These play, with records the synthesizer could not make sense of dropped:\n{}",
            notes.join("\n")
        ));
    }
    Ok(())
}

/// `--show-debug-soundfonts`: print the slots in the form `--set-debug-soundfonts` reads.
///
/// The two are a pair, and the round trip is the contract: every line printed here is a valid
/// argument there, and feeding the whole output back changes nothing. `tools/dev/soundfont-debug.sh
/// --choose` relies on that to open its list with the current slots ticked — the alternative was
/// reading `debug.soundfonts` out of `settings.json` in shell, and there is no JSON parser on the
/// development box by policy.
///
/// **Silence means the switcher is off.** Not a message saying so: this output is read by a script
/// far more often than by a person, and a script that has to recognize a sentence is a script that
/// breaks when the sentence is reworded.
fn show_debug_soundfonts(settings: &Settings) -> anyhow::Result<()> {
    for bank in &settings.debug.soundfonts {
        say(debug_bank_spec(bank));
    }
    Ok(())
}

/// One slot written the way [`parse_debug_bank`] reads it. The other half of the round trip.
///
/// The level is the field that most wants to survive the journey: a bank re-set without it is a
/// bank that clips, and several in the table exceed full scale at `1.0`.
fn debug_bank_spec(bank: &crate::settings::DebugBank) -> String {
    let mut spec = format!("{}={}", bank.path.display(), bank.name);
    if let Some(volume) = bank.music_volume {
        spec.push_str(&format!("={volume}"));
    }
    spec
}

/// `--clear-debug-soundfonts`: empty the slots, which turns the switcher and its label off.
fn clear_debug_soundfonts(paths: &Paths, settings: &mut Settings) -> anyhow::Result<()> {
    let had = std::mem::take(&mut settings.debug.soundfonts).len();
    // The remembered slot is an index into the list that has just gone, so it goes with it. Leaving
    // it would be harmless — `Settings::restored_soundfont_slot` answers `None` for an empty list —
    // and would still be a number in a file describing nothing.
    settings.debug.soundfont_slot = None;
    settings.save(paths)?;
    match had {
        0 => say("No sound bank slots were set; the switcher was already off."),
        count => say(format!(
            "SoundFont slots cleared — {count} removed. The switcher and its on-screen label\nare off. Nothing else changed: switching never wrote audio.soundfont."
        )),
    }
    Ok(())
}

/// Writes `debug.packages`, having opened every package first.
///
/// **Every one of them before a single line is written**, exactly as `set_debug_soundfonts` does and
/// for the same reason: a list written unchecked is several startup failures met one at a time, and
/// finding out which entry is which means restarting the machine.
///
/// No cap on the count, unlike the SoundFont slots. Those are limited because there are eight keys
/// to reach them with; packages have no keys and a tenth is as reachable as the first.
fn set_debug_packages(
    paths: &Paths,
    settings: &mut Settings,
    given: &[String],
) -> anyhow::Result<()> {
    let mut packages = Vec::with_capacity(given.len());
    let mut lines = Vec::new();
    for spec in given {
        // Absolute, because a relative path would name a different file depending on where the
        // machine happened to be started from — and this list is read at every pass.
        let path = std::path::absolute(spec)
            .with_context(|| format!("could not work out the full path of {spec}"))?;
        if !path.is_file() {
            anyhow::bail!("{} is not a file", path.display());
        }
        let package = km_kmpkg::Package::open(&path).with_context(|| {
            format!(
                "{} is not a package this machine can read. Nothing was changed.",
                path.display()
            )
        })?;
        let manifest = package.manifest();
        lines.push(format!(
            "  {} — {} ({} songs)",
            manifest.package.id,
            manifest.package.name,
            manifest.songs.len()
        ));
        packages.push(path);
    }

    let count = packages.len();
    settings.debug.packages = packages;
    settings.save(paths)?;
    let mut report = match count {
        1 => "One extra package will be installed at every start:".to_owned(),
        n => format!("{n} extra packages will be installed at every start:"),
    };
    for line in lines {
        report.push('\n');
        report.push_str(&line);
    }
    report.push_str(
        "\n\nThe packages folders are still scanned as well — this is added to what they hold, \
         never\ninstead of it. Nothing here is the machine's to delete: uninstalling one of these \
         over the\nAPI is refused, and --clear-debug-packages is what takes it out.",
    );
    say(report);
    Ok(())
}

fn clear_debug_packages(paths: &Paths, settings: &mut Settings) -> anyhow::Result<()> {
    let had = std::mem::take(&mut settings.debug.packages).len();
    settings.save(paths)?;
    match had {
        0 => say("No extra packages were named; nothing changed."),
        count => say(format!(
            "Extra packages cleared — {count} removed. **No file was deleted**: this list only ever\nnamed them, and the packages folders are unaffected. Anything that was also sitting in\na scanned folder is still installed."
        )),
    }
    Ok(())
}

fn show_debug_packages(settings: &Settings) -> anyhow::Result<()> {
    // Nothing when the list is empty, which is the shell-friendly way of saying so — and one bare
    // path per line, in exactly the form `--set-debug-packages` reads, so the two round-trip.
    for path in &settings.debug.packages {
        println!("{}", path.display());
    }
    Ok(())
}

/// `--api-bind`'s value, read against the address settings would otherwise have used.
///
/// Two spellings. A bare port keeps `from_settings`' interface and moves only the port, which is
/// the case worth making short: the reason to use this flag at all is almost always that something
/// else already has 8177. A full `ADDRESS:PORT` says both, and is how you also shut a second
/// machine in to loopback while the first one serves the house.
///
/// **A value that will not read is an error and not a fallback.** Every other malformed input on
/// this command line has a sensible default to retreat to; this one does not, because retreating
/// means binding the default port — which is the exact collision the flag was typed to avoid, and a
/// failure that would present as the *first* machine mysteriously losing its remote.
fn resolve_api_bind(value: &str, from_settings: SocketAddr) -> anyhow::Result<SocketAddr> {
    let value = value.trim();
    if let Ok(port) = value.parse::<u16>() {
        return Ok(SocketAddr::new(from_settings.ip(), port));
    }
    value.parse::<SocketAddr>().map_err(|_| {
        anyhow::anyhow!(
            "could not read '{value}' as somewhere to bind: give a port on its own, like `8277`, \
             or a full address, like `0.0.0.0:8277`"
        )
    })
}

/// What a finished song book has to say for itself.
///
/// A struct so that [`song_book_report`] can be tested without a catalog and without writing a
/// file — the wording is the part that rots, and the arithmetic in it is the part that misleads.
struct SongBookReport {
    path: PathBuf,
    songs: usize,
    pages: usize,
    /// Songs whose package carries no first line for them.
    wordless: usize,
    /// Characters no base-14 font can draw, and a sample of them.
    replaced: km_songbook::Replacements,
}

/// The lines `--song-book` prints.
///
/// The last one is not decoration. Packages are installed by `install_startup_packages` inside
/// [`crate::run`], so a book taken straight after dropping a `.kmpkg` into the packages folder will
/// not hold it — and the failure looks exactly like a package that did not work.
fn song_book_report(report: &SongBookReport) -> Vec<String> {
    let mut lines = vec![
        format!("song book  {}", report.path.display()),
        format!(
            "{} song{} over {} page{}",
            report.songs,
            if report.songs == 1 { "" } else { "s" },
            report.pages,
            if report.pages == 1 { "" } else { "s" }
        ),
    ];
    if report.wordless > 0 {
        lines.push(format!(
            "{} {} no first line recorded — a video, an MP3+G pair, or a package built before \
             previews existed",
            report.wordless,
            if report.wordless == 1 {
                "song has"
            } else {
                "songs have"
            }
        ));
    }
    if report.replaced.count > 0 {
        let sample: String = report.replaced.sample.iter().collect();
        lines.push(format!(
            "{} character{} could not be drawn and became '?': {sample}",
            report.replaced.count,
            if report.replaced.count == 1 { "" } else { "s" }
        ));
        lines.push(
            "   the book uses the PDF's built-in Helvetica, which covers Western European text only"
                .to_owned(),
        );
    }
    lines.push(
        "read from the catalog as it stands — start the machine once if a package has just been \
         added"
            .to_owned(),
    );
    lines
}

/// Writes the song book and says what went into it.
///
/// The **first** one-shot on this command line to open the catalog, which is why it is the only
/// one that can fail for a reason that is not the command line's fault.
fn write_song_book(paths: &Paths, out: &std::path::Path, name: Option<&str>) -> anyhow::Result<()> {
    // **Checked before opening, because `Library::open` would otherwise create one.** A book of a
    // catalog that has never existed is an empty book, and the honest answer to asking for one is
    // that the machine has not started yet — not a blank page, and not a SQLite error either, which
    // is what a missing parent directory produces.
    let catalog = paths.library_file();
    if !catalog.is_file() {
        say(format!("no catalog yet at {}", catalog.display()));
        say("start the machine once — it builds the catalog from the packages folder on startup");
        return Ok(());
    }
    let library = km_catalog::Library::open(&catalog).map_err(|error| {
        anyhow::anyhow!(
            "could not open the catalog at {}: {error}",
            catalog.display()
        )
    })?;
    let songs = km_api::book::collect(|after, limit| library.export_after(after, limit))?;
    if songs.is_empty() {
        // An empty book is a valid PDF and this could write one, but writing a page that says "no
        // songs" to somebody who has not installed any is answering the wrong question.
        say("No songs are installed, so there is nothing to print.");
        say(format!(
            "put a .kmpkg in {} and start the machine once",
            paths.packages_dir().display()
        ));
        return Ok(());
    }

    let wordless = songs
        .iter()
        .filter(|song| song.lyric_preview.is_empty())
        .count();
    let version = library.catalog_version().unwrap_or(0);
    let filter = km_api::book::BookFilter::default();
    // Read on its own rather than through `Settings::load`, which repairs and whose caller saves —
    // see `settings::machine_locale`. A printed book is in the machine's language, the same one the
    // television is in.
    let locale = crate::settings::machine_locale(paths);
    // `--book-name` wins; without one, the machine's own name composes the masthead exactly as it
    // does for `GET /songs/book.pdf`, through the one function that decides what that reads.
    let named = name.map(str::to_owned).or_else(|| {
        crate::settings::machine_name(paths)
            .and_then(|machine| km_api::book::book_name_for(&machine))
    });
    let book = km_api::book::render(songs, &filter, version, named.as_deref(), locale);

    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| anyhow::anyhow!("could not make {}: {error}", parent.display()))?;
    }
    std::fs::write(out, book.render())
        .map_err(|error| anyhow::anyhow!("could not write {}: {error}", out.display()))?;

    for line in song_book_report(&SongBookReport {
        path: out.to_path_buf(),
        songs: book.row_count(),
        pages: book.page_count(),
        wordless,
        replaced: book.replaced().clone(),
    }) {
        say(line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder, without touching the process environment.
    ///
    /// `RUST_LOG` is deliberately not exercised here: it is read from the environment this test
    /// shares with every other test in the binary, and setting it would be a race rather than a
    /// test. The branch is one line and the assertion below is the part that has ever been wrong --
    /// the target name. `km_app`, not `karaokemachine`.
    #[test]
    fn verbosity_chooses_a_filter() {
        if std::env::var_os("RUST_LOG").is_some() {
            // Whoever is running the suite asked for something; the arms below cannot be reached.
            return;
        }
        let filter = |verbose| {
            Cli::parse_from(
                std::iter::once("karaokemachine".to_owned())
                    .chain((0..verbose).map(|_| "-v".to_owned())),
            )
            .log_filter(&LoggingSettings::default())
        };
        // The quiet-dependency clause rides along on the two lower rungs and not on the top one,
        // where somebody asking for `-vv` has asked for everything including the chatter.
        assert_eq!(filter(0), format!("info,{QUIET_DEPENDENCIES}"));
        assert_eq!(filter(1), format!("info,km_app=debug,{QUIET_DEPENDENCIES}"));
        assert_eq!(filter(2), "debug,km_app=trace");
        // Beyond the ladder is the top of it, not a panic.
        assert_eq!(filter(5), "debug,km_app=trace");
        // The thing it is actually there to hold down, named rather than implied: symphonia's MP3
        // demuxer announces itself at `info` on every single file it opens.
        assert!(QUIET_DEPENDENCIES.contains("symphonia_bundle_mp3=warn"));
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
        let filter = |verbose, level: &str| {
            let settings = LoggingSettings {
                level: Some(level.to_owned()),
                ..LoggingSettings::default()
            };
            Cli::parse_from(
                std::iter::once("karaokemachine".to_owned())
                    .chain((0..verbose).map(|_| "-v".to_owned())),
            )
            .log_filter(&settings)
        };
        // Nobody typed a flag, so the file speaks -- and it speaks whole, taking the place of the
        // rung rather than being appended to it.
        assert_eq!(filter(0, "debug"), "debug");
        assert_eq!(filter(0, "info,km_api=trace"), "info,km_api=trace");
        // One run typed at a keyboard is never an edit to the machine.
        assert_eq!(
            filter(1, "debug"),
            format!("info,km_app=debug,{QUIET_DEPENDENCIES}")
        );
        assert_eq!(filter(2, "debug"), "debug,km_app=trace");
        // A directive nobody can read leaves the ladder standing, and `warn_about_unread_settings`
        // is what says so.
        assert_eq!(filter(0, "="), format!("info,{QUIET_DEPENDENCIES}"));
    }

    /// A bare port moves the port and keeps the interface settings chose.
    ///
    /// The half that matters is the interface: `0.0.0.0` is the shipped default and a second
    /// machine that quietly came up on `127.0.0.1` instead would be unreachable from the phone
    /// being used to test it, which is a long way to look for a one-word bug.
    #[test]
    fn a_bare_port_keeps_the_interface() {
        let from_settings = "0.0.0.0:8177".parse().expect("should parse");
        assert_eq!(
            resolve_api_bind("8277", from_settings).expect("should resolve"),
            "0.0.0.0:8277".parse::<SocketAddr>().expect("should parse")
        );
        // Whatever `api.bind` says, not a hardcoded `0.0.0.0`.
        let shut_in = "127.0.0.1:8177".parse().expect("should parse");
        assert_eq!(
            resolve_api_bind("8277", shut_in).expect("should resolve"),
            "127.0.0.1:8277"
                .parse::<SocketAddr>()
                .expect("should parse")
        );
    }

    /// A full address says both halves, and nonsense is refused rather than defaulted.
    #[test]
    fn a_full_address_wins_and_a_bad_one_is_an_error() {
        let from_settings = "0.0.0.0:8177".parse().expect("should parse");
        assert_eq!(
            resolve_api_bind("127.0.0.1:8277", from_settings).expect("should resolve"),
            "127.0.0.1:8277"
                .parse::<SocketAddr>()
                .expect("should parse")
        );
        // The port alone is out of range for a `u16`, so it is not a port; it is not an address
        // either. Falling back to 8177 here is the one outcome that must not happen.
        assert!(resolve_api_bind("82777", from_settings).is_err());
        assert!(resolve_api_bind("nowhere", from_settings).is_err());
        assert!(resolve_api_bind("0.0.0.0:", from_settings).is_err());
    }

    /// `--port` is the other three tools' spelling of `--api-bind <port>`, and clap keeps the two
    /// from being given together.
    ///
    /// The conflict is the part worth pinning. Two flags that both name the bound address, with no
    /// rule about which wins, is the shape that produces a machine listening somewhere neither flag
    /// asked for — so they are declared mutually exclusive and this asserts clap enforces it rather
    /// than trusting the attribute to still be there. `--lan` needs `--port` for the same reason:
    /// alone it would silently mean nothing, since `--api-bind` already says the interface.
    #[test]
    fn port_and_api_bind_are_two_spellings_and_cannot_be_given_together() {
        assert!(Cli::try_parse_from(["karaokemachine", "--port", "8277"]).is_ok());
        assert!(Cli::try_parse_from(["karaokemachine", "--port", "8277", "--lan"]).is_ok());
        assert!(
            Cli::try_parse_from(["karaokemachine", "--api-bind", "127.0.0.1:8277"]).is_ok(),
            "the older spelling still parses"
        );
        assert!(
            Cli::try_parse_from([
                "karaokemachine",
                "--port",
                "8277",
                "--api-bind",
                "127.0.0.1:8277",
            ])
            .is_err(),
            "two names for the bound address must not both be given"
        );
        assert!(
            Cli::try_parse_from(["karaokemachine", "--lan"]).is_err(),
            "--lan alone says nothing --api-bind does not already say"
        );
    }

    /// The two spellings of the fullscreen override, and the third thing they can say together.
    ///
    /// The conflict is pinned for the same reason `--port` and `--api-bind`'s is: two flags naming
    /// one thing with no rule about which wins is a machine that does what neither asked for. The
    /// `None` case is pinned as well, because it is the one that carries meaning by *absence* — it
    /// is what leaves `display.fullscreen` in charge, and a refactor that turned the pair into two
    /// bools would lose it silently.
    #[test]
    fn fullscreen_and_windowed_are_one_override_and_cannot_both_be_given() {
        let asked = |argv: &[&str]| {
            Cli::try_parse_from(std::iter::once("karaokemachine").chain(argv.iter().copied()))
                .expect("parses")
                .fullscreen_asked()
        };
        assert_eq!(asked(&["--fullscreen"]), Some(true));
        assert_eq!(asked(&["--windowed"]), Some(false));
        // **And `None` is now two facts rather than one.** It leaves `display.fullscreen` in charge
        // of how the window opens, and it is also what entitles the run to write back how the window
        // was left — `run` sets `DisplayConfig::remember_fullscreen` from exactly this being absent,
        // so a flag run stays the one process it is documented to be.
        assert_eq!(
            asked(&[]),
            None,
            "neither given leaves display.fullscreen in charge, and lets the close write it back"
        );
        assert!(
            Cli::try_parse_from(["karaokemachine", "--fullscreen", "--windowed"]).is_err(),
            "a run cannot be asked for both"
        );
    }

    /// Both routes to the frame meter, and neither of them a log level.
    #[test]
    fn frame_stats_is_off_unless_asked_for() {
        assert!(!Cli::parse_from(["karaokemachine"]).frame_stats);
        assert!(Cli::parse_from(["karaokemachine", "--frame-stats"]).frame_stats);
    }

    /// The log file is off unless asked for, on exactly the argument the meter above is: whether a
    /// program writes a file is not a question about how much detail you want.
    ///
    /// `KM_LOG_FILE` is the other route and is not tested here, for the reason `RUST_LOG` is not
    /// tested in [`verbosity_chooses_a_filter`]: it is read from an environment every other test in
    /// this binary shares, so setting it would be a race rather than a test.
    #[test]
    fn the_log_file_is_off_unless_asked_for() {
        assert!(!Cli::parse_from(["karaokemachine"]).log_file);
        assert!(Cli::parse_from(["karaokemachine", "--log-file"]).log_file);
    }

    /// The viewer is off unless asked for, and naming one takes an `=`.
    #[test]
    fn the_viewer_is_off_unless_asked_for() {
        assert_eq!(Cli::parse_from(["karaokemachine"]).ecapplog, None);
        assert_eq!(
            Cli::parse_from(["karaokemachine", "--ecapplog"]).ecapplog,
            Some(km_ecapplog::DEFAULT_ADDRESS.to_owned())
        );
        assert_eq!(
            Cli::parse_from(["karaokemachine", "--ecapplog=1.2.3.4:99"]).ecapplog,
            Some("1.2.3.4:99".to_owned())
        );
        assert!(Cli::try_parse_from(["karaokemachine", "--ecapplog=nope"]).is_err());
    }

    /// **The `=` is what keeps the positional argument.** Written with a space, an optional-value
    /// flag swallows whatever follows it — so the package a person double-clicked would be read as
    /// a viewer's address and nothing would open. The space form is refused instead.
    #[test]
    fn a_viewer_address_cannot_eat_the_package() {
        let cli = Cli::parse_from(["karaokemachine", "--ecapplog", "songs.kmpkg"]);
        assert_eq!(cli.ecapplog, Some(km_ecapplog::DEFAULT_ADDRESS.to_owned()));
        assert_eq!(cli.package, Some(PathBuf::from("songs.kmpkg")));
    }

    /// A retention setting is read at the command line, and a bad one is refused there.
    #[test]
    fn how_many_logs_to_keep_is_a_count_or_all() {
        assert_eq!(Cli::parse_from(["karaokemachine"]).log_keep, None);
        assert_eq!(
            Cli::parse_from(["karaokemachine", "--log-keep", "200"]).log_keep,
            Some(200)
        );
        assert_eq!(
            Cli::parse_from(["karaokemachine", "--log-keep", "all"]).log_keep,
            Some(km_logfile::KEEP_ALL)
        );
        assert!(Cli::try_parse_from(["karaokemachine", "--log-keep", "lots"]).is_err());
    }

    /// Every option the staged Windows README lists, accepted.
    ///
    /// **The README is written in `tools/platform/windows/dist.sh` and nothing else checks it.** That is the
    /// hole `km-package-builder` fell into once — `--open` was documented in three places and
    /// implemented in none, so the first command its README told anybody to run failed with
    /// `unexpected argument` — and this crate's Options block has just been rewritten, which is
    /// exactly when a list and a parser drift apart. It closes the same hole for the same reason.
    ///
    /// `try_parse_from` rather than `parse_from`: the latter exits the process on a parse error, so
    /// a failure would kill the test harness instead of failing this test.
    #[test]
    fn every_option_the_readme_lists_is_accepted() {
        for args in [
            vec!["--show-paths"],
            vec!["--set-password", "hunter2"],
            vec!["--set-name", "Living Room"],
            vec!["vol1.kmpkg"],
            vec!["--register"],
            vec!["--unregister"],
            vec!["--headless"],
            vec!["--fullscreen"],
            vec!["--windowed"],
            vec!["--data-dir", "."],
            vec!["--play", "song.kar"],
            vec!["-v"],
            vec!["-vv"],
            vec!["--frame-stats"],
            vec!["--log-file"],
            vec!["--log-keep", "all"],
            vec!["--api-bind", "8277"],
            vec!["--version"],
            vec!["--help"],
            vec!["--song-book", "songbook.pdf"],
            vec![
                "--song-book",
                "songbook.pdf",
                "--book-name",
                "Sala de Estar",
            ],
            // Not in the Options block, but named in the prose above it and just as able to rot.
            vec!["--list-audio-devices"],
            vec!["--reset-password"],
            vec!["--reset-sessions"],
            vec!["--set-soundfont", "bank.sf2"],
            vec!["--set-soundfont", "bank.sf2", "--music-volume", "0.8"],
            vec!["--clear-soundfont"],
            vec!["--first-run-soundfont", "recommended"],
            vec!["--first-run-soundfont", "colombogmgs2"],
            vec!["--set-debug-soundfonts", "bank.sf2"],
            // All three spellings of an entry, and several at once — `num_args = 1..` is the part
            // that would rot silently, since a single value parses either way.
            vec![
                "--set-debug-soundfonts",
                "a.sf2=GeneralUser GS",
                "b.sf2=MuseScore General=0.8",
                "c.sf2",
            ],
            vec!["--clear-debug-soundfonts"],
            vec!["--show-debug-soundfonts"],
        ] {
            let argv = std::iter::once("karaokemachine-console").chain(args.iter().copied());
            match Cli::try_parse_from(argv) {
                Ok(_) => {}
                // `--help` and `--version` are "errors" carrying the text to print and an exit code
                // of zero; anything else is a flag the README promises and the parser does not have.
                Err(error) => assert!(
                    matches!(
                        error.kind(),
                        clap::error::ErrorKind::DisplayHelp
                            | clap::error::ErrorKind::DisplayVersion
                    ),
                    "the README lists {args:?}, which the command line rejects: {error}"
                ),
            }
        }
    }

    #[test]
    fn a_debug_bank_spec_reads_all_three_of_its_forms() {
        let bare = parse_debug_bank("some/bank.sf2").expect("a bare path");
        // The stem, because a bank with no name given still has to be called something on screen.
        assert_eq!(bare.name, "bank");
        assert_eq!(bare.music_volume, None);
        assert!(
            bare.path.is_absolute(),
            "the path is absolutised on the way in"
        );

        let named = parse_debug_bank("some/gm.sf2=MuseScore General").expect("a named path");
        assert_eq!(named.name, "MuseScore General");
        assert_eq!(named.music_volume, None);

        let leveled = parse_debug_bank("some/gm.sf2=Arachno=0.7").expect("a leveled path");
        assert_eq!(leveled.name, "Arachno");
        assert_eq!(leveled.music_volume, Some(0.7));
    }

    /// What `--show-debug-soundfonts` prints is what `--set-debug-soundfonts` reads, both ways
    /// round.
    ///
    /// This is the contract `tools/dev/soundfont-debug.sh --choose` is built on: it asks a machine
    /// what is in the slots so it can open its list with them ticked, and re-writes whatever the
    /// person did not change. If the two ever stopped agreeing the symptom would be a picker that
    /// silently drops a bank's level, or a slot that comes back named after its filename — both of
    /// which look like the picker being wrong.
    #[test]
    fn a_slot_printed_is_a_slot_that_can_be_set_again() {
        for spec in [
            "some/bank.sf2=GeneralUser GS",
            "some/bank.sf2=MuseScore General=0.8",
        ] {
            let parsed = parse_debug_bank(spec).expect("the spec parses");
            let printed = debug_bank_spec(&parsed);
            let again = parse_debug_bank(&printed).expect("what was printed parses");
            assert_eq!(again.path, parsed.path, "{printed}");
            assert_eq!(again.name, parsed.name, "{printed}");
            assert_eq!(again.music_volume, parsed.music_volume, "{printed}");
        }
    }

    /// There are exactly three fields, and the third is the level — so a fourth `=` is a mistake
    /// rather than part of a name.
    ///
    /// Worth a test because the alternative reading is tempting: the level could be split off the
    /// end instead, which would let a name hold an `=`. No bank in
    /// `tools/setup/soundfont-banks.sh` has one, and the cost of allowing it is that a mistyped
    /// level becomes part of the name and is silently ignored — a bank playing at the wrong level
    /// with no message anywhere.
    #[test]
    fn a_fourth_field_is_a_mistake_rather_than_part_of_a_name() {
        let error = parse_debug_bank("gm.sf2=a=b=c").expect_err("three fields, not four");
        assert!(
            error.to_string().contains("is not a level"),
            "the message should name the level as the field at fault: {error}"
        );
    }

    #[test]
    fn a_debug_bank_spec_refuses_nonsense_rather_than_defaulting() {
        assert!(parse_debug_bank("").is_err(), "no file named");
        assert!(parse_debug_bank("=name").is_err(), "no file named");
        assert!(
            parse_debug_bank("gm.sf2=name=1.5").is_err(),
            "a level above full scale"
        );
        assert!(
            parse_debug_bank("gm.sf2=name=-0.1").is_err(),
            "a level below silence"
        );
        assert!(
            parse_debug_bank("gm.sf2=name=loud").is_err(),
            "not a number"
        );
    }

    /// The two halves of the pair cannot be asked for at once, exactly as the password pair cannot.
    #[test]
    fn setting_and_clearing_the_soundfont_conflict() {
        let error = Cli::try_parse_from([
            "karaokemachine-console",
            "--set-soundfont",
            "bank.sf2",
            "--clear-soundfont",
        ])
        .expect_err("asking to set and to clear at once is not a coherent request");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    /// Asking for a bank now and asking for one at the next start are two answers to one question.
    ///
    /// They would not even fail usefully together: `--set-soundfont` returns first, so the request
    /// would be written by a command that had already exited, or not written at all depending on
    /// which branch moved. Refusing in the parser is the only place that cannot drift.
    #[test]
    fn setting_a_soundfont_and_requesting_one_conflict() {
        let error = Cli::try_parse_from([
            "karaokemachine-console",
            "--set-soundfont",
            "bank.sf2",
            "--first-run-soundfont",
            "recommended",
        ])
        .expect_err("a bank now and a bank at the next start are not one request");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    /// A double-clicked package arrives as one bare argument and nothing else.
    ///
    /// The whole contract with every file manager on three platforms, and it is one line: no flag,
    /// no subcommand, one path. A `#[arg(long)]` here would have worked from a shell and from
    /// nowhere a person actually double-clicks, which is the case the feature exists for.
    #[test]
    fn a_package_is_taken_as_a_bare_argument_the_way_a_file_manager_passes_one() {
        let cli = Cli::try_parse_from(["karaokemachine", r"D:\tunes\karaoke\vol1.kmpkg"])
            .expect("one path and nothing else is how a double-click arrives");
        assert_eq!(
            cli.package.as_deref(),
            Some(Path::new(r"D:\tunes\karaoke\vol1.kmpkg"))
        );
    }

    /// ...and no argument at all is equally normal: the same executable launched from its icon.
    #[test]
    fn starting_with_no_package_is_the_ordinary_case_and_not_an_error() {
        let cli = Cli::try_parse_from(["karaokemachine"]).expect("an icon passes no arguments");
        assert!(cli.package.is_none());
    }

    /// Registering and unregistering are two answers to one question.
    #[test]
    fn registering_and_unregistering_conflict() {
        let error = Cli::try_parse_from(["karaokemachine-console", "--register", "--unregister"])
            .expect_err("claiming the file type and giving it up are not one request");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    /// **`--music-volume` is not a general volume control**, and saying so in the parser rather than
    /// in prose is what stops it becoming one: on its own it would look like a way to set the level
    /// permanently, which is `audio.music_volume` in settings.json and not a flag.
    #[test]
    fn a_music_volume_on_its_own_is_refused() {
        let error = Cli::try_parse_from(["karaokemachine-console", "--music-volume", "0.8"])
            .expect_err("a level with no bank to attach it to is not what this flag is for");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    fn report(songs: usize, pages: usize, wordless: usize, lost: &[char]) -> Vec<String> {
        let mut replaced = km_songbook::Replacements::default();
        for ch in lost {
            replaced.count += 1;
            replaced.sample.insert(*ch);
        }
        song_book_report(&SongBookReport {
            path: PathBuf::from("book.pdf"),
            songs,
            pages,
            wordless,
            replaced,
        })
    }

    /// The two conditional lines say nothing when there is nothing to say — a report that always
    /// mentions what went wrong trains somebody to stop reading it.
    #[test]
    fn a_book_with_nothing_to_apologize_for_apologizes_for_nothing() {
        let lines = report(12, 3, 0, &[]);
        assert_eq!(lines.len(), 3, "{lines:#?}");
        assert!(lines[0].contains("book.pdf"));
        assert_eq!(lines[1], "12 songs over 3 pages");
        // The one line that is always there, because the failure it heads off looks exactly like a
        // package that did not install.
        assert!(lines[2].contains("start the machine once"), "{lines:#?}");
    }

    #[test]
    fn one_of_a_thing_is_never_pluralised() {
        let lines = report(1, 1, 1, &['東']);
        assert_eq!(lines[1], "1 song over 1 page");
        assert!(
            lines[2].starts_with("1 song has no first line"),
            "{lines:#?}"
        );
        assert!(
            lines[3].starts_with("1 character could not be drawn"),
            "{lines:#?}"
        );
    }

    #[test]
    fn what_could_not_be_drawn_is_said_out_loud_with_a_sample() {
        let lines = report(10, 1, 0, &['東', 'ア']);
        let complaint = lines
            .iter()
            .find(|line| line.contains("could not be drawn"));
        let complaint = complaint.expect("the replacement line");
        assert!(complaint.contains('東'), "{complaint}");
        assert!(
            lines.iter().any(|line| line.contains("Western European")),
            "the reason has to be given, not just the count: {lines:#?}"
        );
    }
}
