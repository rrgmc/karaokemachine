//! Writing a program's log to a file, for the runs nobody is watching a console for.
//!
//! Every program here says what it is doing through `tracing`, and every one of them says it to
//! stdout. That is the right answer for a run somebody typed — and it is no answer at all for the
//! runs these three programs are mostly *used* in. `karaokemachine.exe` is GUI-subsystem on Windows,
//! so a double-click hands it a null standard output handle and every line it writes goes nowhere;
//! `km-package-builder` and `km-remote` are the same the moment they are started by their icon
//! rather than by typing their name. The lines are not suppressed, they are *discarded* — which is
//! why nothing was ever visibly wrong, and why "it did not start and I do not know why" has had no
//! answer short of finding a terminal and running it again.
//!
//! So: `--log-file` (or `KM_LOG_FILE=1`, for the places with no command line) puts the same stream
//! into a file under the application's own data directory, beside the catalog and the packages
//! rather than somewhere only this crate knows about.
//!
//! # It is asked for by name
//!
//! Off by default, on when asked, and it does not move with the verbosity ladder — which is the
//! `What a shipped build says out loud` decision in `docs/decisions/` applied one more time. A log
//! level says *how much detail* you want; whether the program writes a file is a different question
//! and should not be answered by a filter directive. `-v` and `KM_LOG_FILE` compose exactly as you
//! would expect: the flag decides whether there is a file, the ladder decides what goes in it.
//!
//! # A crate rather than three copies
//!
//! `QUIET_DEPENDENCIES` and the `-v` ladder are deliberately duplicated across the same three
//! binaries, and `docs/ARCHITECTURE.md` says why: ten lines of `match` in three crates with no common
//! dependency is cheaper than a workspace member existing to hold them. The axis it names is not
//! size but **how badly a divergence would hurt**, and this falls the other side of it. Opening a
//! file, naming it so two runs cannot collide, deleting the old ones, and handing `tracing` a writer
//! that will not panic when the disk is full are four things to get right rather than ten lines to
//! copy — and three copies of a retention policy is three answers to "how many are kept?".
//!
//! [`km_logsettings`](../km_logsettings/index.html) is the same test applied to the settings key
//! that turns this on: a section whose keys each take either of two shapes, and each answer a bad
//! value with no opinion rather than a refusal, is a grammar. The ladder stays duplicated beside it,
//! because a rung names a different crate in every program and a grammar does not.
//!
//! # No dependencies but the one it implements against
//!
//! [`km_androidlog`](../km_androidlog/index.html) made the same trade for the same reason: a rolling
//! -file crate would be a dependency, a version to track and a policy to configure, in exchange for
//! the hundred lines below. `tracing-appender` in particular does not do what this does — it rotates
//! on the clock (hourly, daily, never) and this rotates per **run**, which is the unit somebody
//! actually asks about. "Send me the log from when it broke" is one file here and a guess there.
//!
//! ```no_run
//! # fn main() -> std::io::Result<()> {
//! use tracing_subscriber::layer::SubscriberExt as _;
//! use tracing_subscriber::util::SubscriberInitExt as _;
//!
//! let file = km_logfile::LogFile::open("/var/lib/karaoke/logs", "karaokemachine")?;
//! tracing_subscriber::registry()
//!     .with(tracing_subscriber::EnvFilter::new("info"))
//!     .with(tracing_subscriber::fmt::layer())
//!     .with(file.layer())
//!     .init();
//! # Ok(())
//! # }
//! ```

use std::fs::File;
use std::io::{self, Write};
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::registry::LookupSpan;

/// The environment variable that turns file logging on where there is no command line to put a flag
/// on.
///
/// The same shape as `KM_FRAME_STATS`, and for the same reason: a systemd unit and an Android
/// activity both have somewhere to put an environment variable and nowhere to put an argument. Any
/// value but `0` counts as yes, so `KM_LOG_FILE=1` and `KM_LOG_FILE=yes` both work and
/// `KM_LOG_FILE=0` is an explicit no.
pub const ENV_VAR: &str = "KM_LOG_FILE";

/// How many runs' logs are kept in a directory, this one included.
///
/// The oldest beyond this are deleted when a new one is opened. Ten is an evening's worth of
/// starting and stopping while something is being sorted out, which is when anybody looks — and the
/// files are small enough that the number is not really about space: it is about the directory
/// staying readable, so that "the newest one" is a thing you can see rather than a thing you have to
/// sort for.
pub const KEEP: usize = 10;

/// Keeps every file there has ever been, rather than the newest few.
///
/// What a machine being worked on wants: the names carry the date and time they started, so a
/// directory that is never pruned is the whole history of that machine in the order it happened.
/// [`prune`] compares a count against this and stops, so nothing is ever read or sorted for it.
pub const KEEP_ALL: usize = usize::MAX;

/// The environment variable that says how many to keep.
///
/// `all`, or a count. Read wherever a file is opened, so **one variable covers every program**
/// rather than each of the four growing a flag of its own -- and the appliance, which is started by
/// a systemd unit, has somewhere to put a variable and nowhere to put an argument. A program with a
/// command line of its own may offer a flag that wins over it; the machine does.
pub const KEEP_ENV_VAR: &str = "KM_LOG_KEEP";

/// The subdirectory of an application's data directory that logs go in.
///
/// Named here so the three callers cannot disagree about it, on exactly the argument
/// `km_songcode::MAX_SLOT` is shared by: one constant beats three literals that are equal today.
pub const SUBDIR: &str = "logs";

/// What a run's log is called.
const LOG_EXTENSION: &str = "log";

/// What a panic's report is called.
///
/// **A second extension rather than a second folder or a second stem.** [`prune`] counts by
/// extension, so the two kinds retire on their own clocks in one directory: an evening of starting
/// and stopping cannot push out the report of the panic that ended one of those runs. A folder of
/// its own would be a second path for every caller to learn, and a stem of its own would collide
/// with [`prune`]'s prefix match.
const CRASH_EXTENSION: &str = "crash";

/// Whether this run should write a log file.
///
/// `flag` is the program's own `--log-file`; the environment variable is checked second so that a
/// command line always wins over an inherited setting rather than being overridden by one.
#[must_use]
pub fn asked_for(flag: bool) -> bool {
    flag || std::env::var_os(ENV_VAR).is_some_and(|value| value != "0")
}

/// How many files to keep, or `None` when nobody has said.
///
/// `flag` is the program's own `--log-keep`; the environment variable is checked second so that a
/// command line always wins over an inherited setting rather than being overridden by one.
///
/// **`None` is not the same as [`KEEP`]**, and the caller wants the difference: asking to keep a
/// history is asking for a history to be written, so a run that names a count is a run that wants
/// a file even without `--log-file` beside it.
#[must_use]
pub fn keep_wanted(flag: Option<usize>) -> Option<usize> {
    flag.or_else(|| {
        std::env::var(KEEP_ENV_VAR)
            .ok()
            .and_then(|value| parse_keep(&value).ok())
    })
}

/// Reads `all` or a count.
///
/// Shared with the machine's argument parser so that the flag and the variable cannot disagree
/// about what they accept, and so the refusal is written once.
///
/// # Errors
///
/// If the value is neither `all` nor a number.
pub fn parse_keep(value: &str) -> Result<usize, String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("all") {
        return Ok(KEEP_ALL);
    }
    value
        .parse()
        .map_err(|_| format!("expected a number of files to keep, or `all`, not `{value}`"))
}

/// Makes a panic leave a file behind, whether or not this run asked for a log.
///
/// **The file above is asked for; this one is not, and that is the difference that matters.** A
/// panic is the one thing a person cannot decide to have recorded in advance, because by the time
/// they know they wanted it the process is gone. Nothing is written until one happens, so a machine
/// that never panics never writes a byte here -- which is what keeps this on the right side of the
/// argument that made the log a flag rather than a setting.
///
/// **Neither the console nor the log file reaches a panic on its own.** The default hook writes to
/// stderr, and a GUI-subsystem executable has a null handle there, so the message goes nowhere at
/// all; and it does not travel through `tracing`, so a run with `--log-file` on has a log that stops
/// mid-sentence at the last ordinary event. This installs both destinations: the report as a file,
/// and the same facts as a `tracing` event for whoever is watching a console.
///
/// The previous hook still runs afterwards, so a terminal prints exactly what it printed before.
///
/// Best effort throughout, and it has to be: a hook that can fail is a second panic inside the
/// first, which Rust answers by aborting -- taking with it the report this exists to write.
pub fn report_panics(dir: impl Into<PathBuf>, stem: &str, keep: usize) {
    let dir = dir.into();
    let stem = stem.to_owned();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "an unknown place".to_owned(), Location::to_string);
        let message = info
            .payload_as_str()
            .unwrap_or("a payload that is not text");
        // The file first, because it is the destination that survives having nobody watching, and
        // because a subscriber is one more thing that could itself panic.
        let written = write_crash(&dir, &stem, &location, message, keep);
        tracing::error!(%location, message, "the program panicked");
        if let Some(path) = &written {
            tracing::error!(path = %path.display(), "the panic was written here");
        }
        previous(info);
    }));
}

/// Writes one crash report, answering where it went.
///
/// The backtrace is forced rather than left to `RUST_BACKTRACE`, since the runs this exists for are
/// the ones nobody set an environment variable before starting.
fn write_crash(
    dir: &Path,
    stem: &str,
    location: &str,
    message: &str,
    keep: usize,
) -> Option<PathBuf> {
    let (mut file, path) = create_dated(dir, stem, CRASH_EXTENSION, keep).ok()?;
    // This crate's version, which is the caller's: `One version number for the whole repository`
    // makes them the same number, and the report wants the number somebody would quote.
    let version = env!("CARGO_PKG_VERSION");
    let trace = std::backtrace::Backtrace::force_capture();
    let report = format!(
        "{stem} {version} panicked at {location}\n\n{message}\n\nthread: {}\n\n{trace}\n",
        std::thread::current().name().unwrap_or("unnamed"),
    );
    file.write_all(report.as_bytes()).ok()?;
    // Explicit, because a panic that is on its way to aborting may not run the drop that would
    // otherwise flush this.
    file.flush().ok()?;
    Some(path)
}

/// An open log file, and the `tracing` layer that writes to it.
///
/// Cloneable and cheap to clone — the handle is shared, not the file. It has to be `'static` and
/// `Clone` to be a [`MakeWriter`], which is the only reason for the [`Arc`].
#[derive(Clone)]
pub struct LogFile(Arc<Open>);

/// The bits behind the [`Arc`]. Separate only so that [`LogFile`] can be cloned freely.
struct Open {
    /// Held under a mutex because `tracing` will write to it from every thread the program has, and
    /// an event arrives as several `write` calls: without the lock two threads' lines interleave
    /// mid-word. The guard is held for one whole event — see [`LogFile::make_writer`].
    file: Mutex<File>,
    /// Where it is, so a caller can say so.
    path: PathBuf,
}

impl LogFile {
    /// Opens this run's log in `dir`, making the directory and pruning older runs.
    ///
    /// The name is `<stem>-<UTC timestamp>.log`; [`create_dated`] owns its shape. Retention is
    /// [`KEEP_ENV_VAR`]'s if it is set and [`KEEP`] if it is not, so a program with no flag of its
    /// own still keeps what a machine being worked on was told to keep.
    ///
    /// # Errors
    ///
    /// If the directory cannot be made or no name in it can be created. A caller should report that
    /// and carry on without a file — a log that cannot be written is not a reason to refuse to run.
    pub fn open(dir: impl AsRef<Path>, stem: &str) -> io::Result<Self> {
        Self::open_keeping(dir, stem, keep_wanted(None).unwrap_or(KEEP))
    }

    /// The same, for a program whose own command line said how many to keep.
    ///
    /// # Errors
    ///
    /// As [`LogFile::open`].
    pub fn open_keeping(dir: impl AsRef<Path>, stem: &str, keep: usize) -> io::Result<Self> {
        let (file, path) = create_dated(dir.as_ref(), stem, LOG_EXTENSION, keep)?;
        Ok(Self(Arc::new(Open {
            file: Mutex::new(file),
            path,
        })))
    }

    /// Where this run's log is being written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0.path
    }

    /// The `tracing` layer that writes formatted events into it.
    ///
    /// **Never colored and always timestamped**, whatever the console layer beside it does. Those
    /// are two separate corrections of what a single-writer arrangement would produce. ANSI escapes
    /// are for a terminal and are bytes somebody has to read around in a file; and the machine drops
    /// its timestamp when systemd says it owns stdout, which is right for the journal — journald
    /// stamps every line itself — and wrong for a file, where nothing else will.
    ///
    /// Boxed because that is what lets a caller add it, or not, without the two shapes being
    /// different types.
    #[must_use]
    pub fn layer<S>(&self) -> Box<dyn Layer<S> + Send + Sync + 'static>
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(self.clone())
            .boxed()
    }
}

impl std::fmt::Debug for LogFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogFile")
            .field("path", &self.0.path)
            .finish()
    }
}

/// One event's worth of bytes, on their way to the file.
///
/// It is a lock guard, so the whole of one event is written before another thread's begins.
pub struct Handle<'a>(MutexGuard<'a, File>);

impl Write for Handle<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for LogFile {
    type Writer = Handle<'a>;

    /// **A failed write is discarded, and that is load-bearing rather than lazy.**
    /// `tracing_subscriber` throws its writer's errors away unless `log_internal_errors` is on, and
    /// that setting reports through `eprintln!`, which *panics* when there is no stderr — which is
    /// exactly the GUI-subsystem double-click this crate exists for. A full disk must cost log lines
    /// and never the program. `crates/machine/karaokemachine/src/cli.rs` carries the same warning
    /// about the console writer.
    ///
    /// A poisoned mutex is recovered from for the same reason: a thread that panicked mid-line must
    /// not take logging down with it, and the worst it can leave behind is a truncated line.
    fn make_writer(&'a self) -> Self::Writer {
        Handle(self.0.file.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The highest collision suffix already used for this stem and second, or 0 if there is none.
///
/// The unsuffixed name counts as 1, so that the first run of a second gets it and every run after
/// gets `_02` upwards. Anything unparseable is ignored rather than guessed at: the cost of missing
/// one is a `create_new` that loses and is retried, which the loop in [`LogFile::open`] already does.
/// Creates `dir/<stem>-<UTC timestamp>.<extension>`, making the directory and pruning older ones.
///
/// **UTC, and the `Z` says so** — a local-time name would need a time zone database, which is a
/// dependency this crate is written to avoid, and a stamp that silently means one thing on the
/// appliance and another on the box that read the file is worse than one that is plainly universal.
/// Every file manager shows the local modification time beside the name anyway.
///
/// Two files wanted inside one second get `_02`, `_03` and so on rather than sharing one. That is
/// not hypothetical here: `--api-bind` without `--data-dir` is a documented way to end up with two
/// machines sharing one data directory, and on Windows the second would simply fail to open the
/// first's file.
///
/// **The suffix is `_` and is zero-padded, and both halves are load-bearing** — [`prune`] keeps the
/// newest by sorting names, so a name that sorts wrongly is a file deleted out of turn. `_` (0x5F)
/// sorts *after* the `.` that begins the extension (0x2E), so a collision comes after the file it
/// collided with rather than before it; and the padding is what puts `_02` before `_11` instead of
/// after it. Found by running the machine thirteen times inside one second, which pruned the two
/// newest.
///
/// # Errors
///
/// If the directory cannot be made or no name in it can be created.
fn create_dated(
    dir: &Path,
    stem: &str,
    extension: &str,
    keep: usize,
) -> io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(dir)?;

    let stamp = stamp(SystemTime::now());
    let mut last = None;
    // **From the highest name in use rather than from the first one free**, which is the second
    // thing a same-second batch got wrong. `prune` runs at the end of this function and deletes
    // the *lowest*-sorting names — so a first-free scan handed the freed `…Z.log` back to the
    // next run a moment later, and that run then held the oldest-sorting name in the folder and
    // was the first thing deleted. Counting up from what is there cannot reuse a name, because
    // pruning keeps the newest and therefore always leaves the highest suffix behind.
    let first = highest_in_use(dir, stem, &stamp, extension) + 1;
    for attempt in first..first.saturating_add(64) {
        let name = if attempt == 1 {
            format!("{stem}-{stamp}.{extension}")
        } else {
            format!("{stem}-{stamp}_{attempt:02}.{extension}")
        };
        let path = dir.join(name);
        // `create_new`, so that losing a race means trying the next name rather than two
        // processes writing into one file.
        match File::options().write(true).create_new(true).open(&path) {
            Ok(file) => {
                // After ours exists, so the count includes it and `keep` means what it says.
                prune(dir, stem, extension, keep);
                return Ok((file, path));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no free file name")))
}

fn highest_in_use(dir: &Path, stem: &str, stamp: &str, extension: &str) -> u32 {
    let prefix = format!("{stem}-{stamp}");
    let suffix = format!(".{extension}");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| {
            let rest = name.strip_prefix(&prefix)?.strip_suffix(&suffix)?;
            match rest.strip_prefix('_') {
                None if rest.is_empty() => Some(1),
                None => None,
                Some(digits) => digits.parse().ok(),
            }
        })
        .max()
        .unwrap_or(0)
}

/// Deletes all but the newest `keep` of `dir`'s `<stem>-*.<extension>` files.
///
/// **The extension is what separates the two kinds of file in one folder.** Runs and crashes are
/// counted apart, so an evening of starting and stopping cannot push out the report of the panic
/// that ended one of those runs -- which is the one file anybody in that folder is looking for.
///
/// **Best effort throughout.** A file another instance still has open cannot be removed on Windows,
/// and an unreadable directory is not a reason to refuse to log — in both cases the right outcome is
/// one more file than intended, not a program that will not start.
///
/// Sorting by name is sorting by time, because [`stamp`] is fixed width and starts with the year —
/// and, for a same-second collision, because of the two properties [`LogFile::open`] describes the
/// `_02` suffix as having. That equivalence is the whole reason this can delete by name rather than
/// by asking the filesystem for modification times, and it is worth keeping rather than taking on
/// faith: `the_newest_survive_a_same_second_batch` is what pins it.
fn prune(dir: &Path, stem: &str, extension: &str, keep: usize) {
    let prefix = format!("{stem}-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut ours: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|found| found == extension)
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect();
    if ours.len() <= keep {
        return;
    }
    ours.sort();
    let doomed = ours.len() - keep;
    for path in &ours[..doomed] {
        let _ = std::fs::remove_file(path);
    }
}

/// `YYYYMMDDThhmmssZ` for a moment, in UTC.
///
/// A time before the epoch is not a thing this is ever asked for — it would mean a clock set to the
/// 1960s — and it answers `19700101T000000Z` rather than carrying signed arithmetic through
/// [`civil_from_days`] for a case that cannot arise.
fn stamp(at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rest / 3_600, (rest / 60) % 60, rest % 60);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// The calendar date `days` after 1970-01-01, in the proleptic Gregorian calendar.
///
/// Howard Hinnant's `civil_from_days`, which is the standard way to do this without a date library
/// and is what every one of them does inside. It shifts the year to start in March so that the leap
/// day is the last day of it, which is what removes every special case: the four-, hundred- and
/// four-hundred-year rules all fall out of the era arithmetic instead of being written down.
///
/// Unsigned throughout because [`stamp`] never passes it a date before the epoch.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // 719_468 is 1970-01-01 counted from 0000-03-01, the start of the first era.
    let z = days + 719_468;
    let era = z / 146_097; // 146_097 days is 400 years exactly.
    let doe = z - era * 146_097; // day of era, [0, 146_096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of the March-based year, [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month, [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stamp is what stops two runs sharing a file and what makes the pruning order right, so
    /// the arithmetic behind it is worth pinning at the awkward dates rather than only at one.
    #[test]
    fn stamps_known_moments() {
        let at = |secs| stamp(UNIX_EPOCH + std::time::Duration::from_secs(secs));
        assert_eq!(at(0), "19700101T000000Z");
        // The famous one: 10^9 seconds.
        assert_eq!(at(1_000_000_000), "20010909T014640Z");
        // A leap day in a year divisible by 100 *and* 400, so it is one.
        assert_eq!(at(951_782_400), "20000229T000000Z");
        // 2100 is divisible by 100 and not by 400, so it is not a leap year: this is 1 March, and a
        // naive four-year rule would call it 29 February.
        assert_eq!(at(4_107_542_400), "21000301T000000Z");
        assert_eq!(at(86_399), "19700101T235959Z");
    }

    /// Before the epoch cannot happen and must not panic if it somehow does.
    #[test]
    fn a_clock_set_before_the_epoch_does_not_panic() {
        let at = UNIX_EPOCH - std::time::Duration::from_secs(60);
        assert_eq!(stamp(at), "19700101T000000Z");
    }

    #[test]
    fn the_environment_variable_is_a_second_way_to_ask() {
        // The flag alone is enough, which is the half that needs no environment.
        assert!(asked_for(true));
    }

    #[test]
    fn a_run_gets_its_own_file_and_the_directory_is_made() {
        let dir = scratch("own-file");
        let log = LogFile::open(dir.join("logs"), "karaokemachine").expect("open");
        assert!(log.path().exists());
        assert!(
            log.path()
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("karaokemachine-") && name.ends_with(".log"))
        );
    }

    /// Two runs in the same second must not share a file — see [`LogFile::open`].
    #[test]
    fn two_runs_in_one_second_get_two_files() {
        let dir = scratch("same-second");
        let first = LogFile::open(&dir, "km-remote").expect("first");
        let second = LogFile::open(&dir, "km-remote").expect("second");
        assert_ne!(first.path(), second.path());
    }

    /// **The one that was got wrong first.** [`prune`] keeps the newest by sorting names, so a batch
    /// of runs inside one second is only pruned correctly if the collision suffix sorts in creation
    /// order. It did not: `-11` sorts before `-2`, and both sorted before the unsuffixed name, so
    /// thirteen starts of the machine inside one second deleted the two most recent of them.
    ///
    /// This asserts the property rather than the fix, so it goes on holding if the spelling changes.
    #[test]
    fn the_newest_survive_a_same_second_batch() {
        let dir = scratch("batch");
        let made: Vec<PathBuf> = (0..13)
            .map(|_| {
                LogFile::open(&dir, "karaokemachine")
                    .expect("open")
                    .path()
                    .to_path_buf()
            })
            .collect();

        // Every name is distinct, and sorting them puts them back in the order they were made —
        // which is exactly what lets `prune` choose by name.
        let mut sorted = made.clone();
        sorted.sort();
        assert_eq!(sorted, made, "names must sort in creation order");

        prune(&dir, "karaokemachine", LOG_EXTENSION, KEEP);
        let left: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .map(|entry| entry.path())
            .collect();
        assert_eq!(left.len(), KEEP);
        for newest in &made[made.len() - KEEP..] {
            assert!(left.contains(newest), "{} was deleted", newest.display());
        }
    }

    /// What is written comes back, and comes back whole: the point of the mutex is that an event is
    /// not interleaved with another thread's.
    #[test]
    fn what_is_written_reaches_the_file() {
        let dir = scratch("written");
        let log = LogFile::open(&dir, "km-package-builder").expect("open");
        {
            let mut writer = log.make_writer();
            writer.write_all(b"a line\n").expect("write");
            writer.flush().expect("flush");
        }
        let text = std::fs::read_to_string(log.path()).expect("read");
        assert_eq!(text, "a line\n");
    }

    /// The retention policy, which is the thing three copies of this would have disagreed about.
    #[test]
    fn only_the_newest_are_kept() {
        let dir = scratch("pruned");
        std::fs::create_dir_all(&dir).expect("dir");
        for second in 0..12 {
            std::fs::write(
                dir.join(format!("km-remote-20260901T0000{second:02}Z.log")),
                b"x",
            )
            .expect("write");
        }
        // A file that is not a log, and one belonging to another program, both survive: this deletes
        // its own runs and nothing else in the folder.
        std::fs::write(dir.join("km-remote-20260901T000000Z.txt"), b"x").expect("write");
        std::fs::write(dir.join("karaokemachine-20260101T000000Z.log"), b"x").expect("write");

        prune(&dir, "km-remote", LOG_EXTENSION, 3);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "karaokemachine-20260101T000000Z.log".to_owned(),
                "km-remote-20260901T000000Z.txt".to_owned(),
                "km-remote-20260901T000009Z.log".to_owned(),
                "km-remote-20260901T000010Z.log".to_owned(),
                "km-remote-20260901T000011Z.log".to_owned(),
            ]
        );
    }

    /// `all` and a count are both answers; anything else is refused rather than guessed at.
    #[test]
    fn a_retention_setting_is_a_count_or_the_word_all() {
        assert_eq!(parse_keep("all"), Ok(KEEP_ALL));
        assert_eq!(parse_keep(" ALL "), Ok(KEEP_ALL));
        assert_eq!(parse_keep("200"), Ok(200));
        assert!(parse_keep("everything").is_err());
        assert!(parse_keep("").is_err());
        // Nobody having said is not the same as somebody having said ten: the caller turns the first
        // into a default and reads the second as a reason to write a file at all.
        assert_eq!(keep_wanted(Some(4)), Some(4));
    }

    /// Keeping everything is a number `prune` never reaches, so a full directory stays full.
    #[test]
    fn keeping_all_of_them_deletes_none() {
        let dir = scratch("kept");
        std::fs::create_dir_all(&dir).expect("dir");
        for second in 0..12 {
            std::fs::write(
                dir.join(format!("karaokemachine-20260901T0000{second:02}Z.log")),
                b"x",
            )
            .expect("write");
        }

        prune(&dir, "karaokemachine", LOG_EXTENSION, KEEP_ALL);

        assert_eq!(std::fs::read_dir(&dir).expect("read").count(), 12);
    }

    /// The report a panic leaves, written by hand.
    ///
    /// **Deliberately not an actual panic**: the hook is process-wide and unwinding out of a test
    /// would take the harness's own reporting with it. What is asserted is the part that has to be
    /// right when nobody is watching — that the file appears, is named for the moment it happened,
    /// and holds enough to act on.
    #[test]
    fn a_panic_leaves_a_file_naming_where_and_what() {
        let dir = scratch("crashed");

        let path = write_crash(
            &dir,
            "karaokemachine",
            "text.rs:434:9",
            "a NUL got in",
            KEEP,
        )
        .expect("a report");

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        assert!(name.starts_with("karaokemachine-"), "{name}");
        assert!(name.ends_with(".crash"), "{name}");
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("text.rs:434:9"), "{written}");
        assert!(written.contains("a NUL got in"), "{written}");
        assert!(written.contains("karaokemachine"), "{written}");
    }

    /// A crash report is not a run's log, and an evening of runs must not push one out.
    #[test]
    fn runs_and_crashes_retire_on_separate_clocks() {
        let dir = scratch("both");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("karaokemachine-20260101T000000Z.crash"), b"x").expect("write");
        for second in 0..12 {
            std::fs::write(
                dir.join(format!("karaokemachine-20260901T0000{second:02}Z.log")),
                b"x",
            )
            .expect("write");
        }

        prune(&dir, "karaokemachine", LOG_EXTENSION, 3);

        assert!(
            dir.join("karaokemachine-20260101T000000Z.crash").exists(),
            "the oldest file in the folder, and the one worth keeping"
        );
    }

    /// A report is best effort: a directory that cannot be made costs the file, not the process.
    #[test]
    fn a_report_that_cannot_be_written_is_not_a_second_panic() {
        let blocked = scratch("blocked");
        std::fs::create_dir_all(blocked.parent().expect("parent")).expect("dir");
        // A file where the directory would have to go, so `create_dir_all` cannot succeed.
        std::fs::write(&blocked, b"x").expect("write");

        assert_eq!(
            write_crash(&blocked, "karaokemachine", "somewhere", "something", KEEP),
            None
        );
    }

    /// A directory of this test's own, removed first so a rerun starts empty.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("km-logfile-tests").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
