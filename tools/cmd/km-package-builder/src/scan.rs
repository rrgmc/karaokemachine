//! The corpus scan.
//!
//! Shaped after `km-lyrics scan`, which is this project's existing answer to "parse a whole tree and
//! tabulate it": collect the paths, chunk them across scoped threads, catch a panic per file so one
//! bad file cannot end the run, and merge the results. What is new here is that the results are
//! written rather than printed, and that a second run is nearly free.
//!
//! **Incremental is the default and it matters.** The corpus this was built for holds hundreds of
//! thousands of files. A first scan reads and parses all of them; a second reads none of them,
//! because a path whose size and modification time are unchanged is skipped before it is opened.

use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use km_kmpkg::content_hash;
use km_song::{ParseOptions, Song};
use km_suitability::Analysis;

use crate::db::{DbError, Shared};
use crate::dupes::fingerprint;
#[cfg(feature = "video")]
use crate::model::VideoFacts;
use crate::model::{CdgFacts, MidiFacts, ScanStatus, ScannedFile, ScannedSong, UltraStarFacts};
use crate::step::{Ladder, Step, StepState, StepView, rough_duration};

thread_local! {
    /// Whether a panic on this thread right now is one the scan is deliberately swallowing.
    ///
    /// **The narrowest thing the panic hook can be keyed on.** A corpus holds files that make
    /// `Song::parse` panic, and printing one message per bad file across a whole corpus of them is
    /// useless — but the hook that suppresses those is process-wide, and this process is also
    /// serving a web UI while the scan runs. Keying on a flag set for the duration of one parse
    /// keeps the suppression to exactly the call it was written for.
    ///
    /// Set and cleared around the `catch_unwind` in [`scan_one`], which never propagates, so
    /// control always returns to clear it.
    static PARSING_A_CORPUS_FILE: Cell<bool> = const { Cell::new(false) };
}

/// How many results are written per transaction.
///
/// Large enough that the write is not the bottleneck, small enough that progress on screen keeps
/// moving and an interrupted scan loses very little.
const BATCH: usize = 500;

/// How long a scan stands aside at a batch boundary for whoever is waiting to write.
///
/// **It has to be asked for rather than left to the mutex.** `std::sync::Mutex` makes no fairness
/// promise, so the writer below releasing the connection and immediately taking it again does not
/// hand it over — which is why a scan could go on answering reads while quietly refusing every write
/// for the length of a run. [`Shared::wanted`](crate::db::Shared::wanted) is what is asked, and this
/// is how long the answer is honoured for.
///
/// Bounded both ways, because a write may not stall a corpus read either. A waiter is polling on a
/// far shorter interval than this, so the window is far more than it needs — what it is protecting
/// against is a waiter that went away between the question and the answer.
///
/// **It is nearly free**, and that is the read connection's doing: pages do not come through here at
/// all, so `wanted()` is lit by a star or a rename and not by somebody browsing. Against a batch
/// that commits hundreds of scattered rows, standing aside for this long is a fraction of one.
const YIELD_FOR: Duration = Duration::from_millis(500);

/// How often the stand-off checks whether the waiter has been through.
const YIELD_STEP: Duration = Duration::from_millis(10);

/// Waits for whoever is queued on the writing connection to have their turn.
///
/// Called after the guard is released, never while it is held — the point is to leave the connection
/// free for long enough that an unfair mutex hands it over.
fn stand_off(db: &Shared) {
    let deadline = Instant::now() + YIELD_FOR;
    while db.wanted() && Instant::now() < deadline {
        std::thread::sleep(YIELD_STEP);
    }
}

/// What a scan is doing, as a catalog key.
///
/// **A key rather than a sentence**, because a phase is named on the worker thread, where no request
/// and so no language is in reach. The page writes the words with `{{ progress.phase|t }}` — the
/// filter takes any string — which is how `km-admin`'s job phases reach a page too.
pub mod phase {
    /// Loading what the last scan recorded about every file, which the skip test compares against.
    pub const PREPARING: &str = "scan-phase-preparing";
    /// Finding the files under the root that this tool can read.
    pub const LOOKING: &str = "scan-phase-looking";
    /// Reading and analyzing them, which is the long one.
    pub const READING: &str = "scan-phase-reading";
    /// Building the folder tree the Folders page reads.
    pub const INDEXING: &str = "scan-phase-indexing";
    /// Dropping rows for files that are no longer on disk.
    pub const FORGETTING: &str = "scan-phase-forgetting";
    /// Grouping the files that look like one recording.
    pub const DUPLICATES: &str = "scan-phase-duplicates";
    /// Giving SQLite the statistics its planner needs.
    pub const MEASURING: &str = "scan-phase-measuring";
    /// Ended because somebody asked it to.
    pub const STOPPED: &str = "scan-phase-stopped";
    /// Ended because there was nothing left to do.
    pub const FINISHED: &str = "scan-phase-finished";

    /// Every phase, for the parity tests in [`crate::words`].
    #[cfg(test)]
    pub const ALL: &[&str] = &[
        PREPARING, LOOKING, READING, INDEXING, FORGETTING, DUPLICATES, MEASURING, STOPPED, FINISHED,
    ];
}

/// The steps a run with these options can take, in the order it takes them.
///
/// **Known before the run starts**, which is what lets the page say what is still to come. Three of
/// them depend on whether the run changes a row, which nobody knows until the reading is over, so
/// those are listed with `if_changed` and are skipped at that point if nothing did. A scoped run
/// never lists forgetting or grouping duplicates, because it can never run them.
fn planned_steps(options: &ScanOptions) -> Vec<Step> {
    let whole = options.only.is_none();
    let mut steps = vec![
        Step::waiting(phase::PREPARING, false),
        Step::waiting(phase::LOOKING, false),
        Step::waiting(phase::READING, false),
    ];
    if whole {
        steps.push(Step::waiting(phase::FORGETTING, false));
    }
    if whole && options.force {
        steps.push(Step::waiting(phase::DUPLICATES, true));
    }
    steps.push(Step::waiting(phase::INDEXING, true));
    steps.push(Step::waiting(phase::MEASURING, true));
    steps
}

/// How far back the rate is measured.
///
/// **Recent rather than since the start**, because an incremental scan skips its unchanged files in a
/// burst and then reads the changed ones at a small fraction of that speed. An average over the whole
/// run would promise an end that recedes for as long as the run lasts.
const RATE_WINDOW: Duration = Duration::from_secs(30);

/// How much of the window has to be measured before a rate is shown at all.
const RATE_MINIMUM: Duration = Duration::from_secs(10);

/// How often a sample is taken. Pages poll once a second, and a snapshot is where a sample is taken.
const SAMPLE_EVERY: Duration = Duration::from_secs(1);

/// The rate over the samples in hand, in files a second, and the time `remaining` files take at it.
///
/// `None` until the samples span [`RATE_MINIMUM`], and while nothing is moving: a time left worked
/// out from a second of data, or from a rate of nought, is a number that misleads.
fn estimate(samples: &VecDeque<(Instant, u64)>, remaining: u64) -> Option<(f64, Duration)> {
    let (first_at, first) = samples.front()?;
    let (last_at, last) = samples.back()?;
    let span = last_at.saturating_duration_since(*first_at);
    if span < RATE_MINIMUM {
        return None;
    }
    let rate = last.saturating_sub(*first) as f64 / span.as_secs_f64();
    (rate > 0.0).then(|| (rate, Duration::from_secs_f64(remaining as f64 / rate)))
}

/// Live progress, readable from the web handler while the scan runs.
#[derive(Debug, Default)]
pub struct Progress {
    /// Files found under the root.
    pub total: AtomicU64,
    /// Files dealt with, skipped ones included.
    pub done: AtomicU64,
    /// Files parsed and analyzed this run.
    pub parsed: AtomicU64,
    /// Files that did not parse.
    pub failed: AtomicU64,
    /// Files unchanged since the last run, and so not read at all.
    pub skipped: AtomicU64,
    /// Files whose rows are committed to the database.
    ///
    /// Separate from `done` because the readers run far ahead of the writer: the channel between
    /// them is unbounded, so over a large corpus hundreds of thousands of results can be read and
    /// still be queued. A bar driven by `done` reaches 100% with a quarter of the corpus not yet in
    /// the database, and somebody browsing at that moment sees the song count climb for minutes
    /// after being told the scan had finished.
    pub written: AtomicU64,
    /// Set to ask the run to stop early. Never cleared: a `Progress` belongs to one run.
    ///
    /// The workers check it between files, so a stop takes effect within one file rather than one
    /// corpus. The writer does not check it — it drains whatever has already been read, which is the
    /// whole point of stopping cleanly rather than being killed.
    pub cancel: AtomicBool,
    /// Set when the run stopped because it was asked to, rather than because it ran out of files.
    pub canceled: AtomicBool,
    /// Set when the run has ended, however it ended.
    pub finished: AtomicBool,
    /// The failure that ended the run, if one did.
    pub error: Mutex<Option<String>>,
    /// What the run is doing at the moment, for the status line.
    pub phase: Mutex<String>,
    /// Files found so far by the walk, which is minutes on a large corpus and has no `total` yet.
    pub found: AtomicU64,
    /// Every step of the run, in order, with where each stands and how long each took.
    steps: Ladder,
    /// Settled counts taken over the last [`RATE_WINDOW`], for the rate and the time left.
    samples: Mutex<VecDeque<(Instant, u64)>>,
}

impl Progress {
    /// A snapshot, for rendering.
    pub fn snapshot(&self) -> ProgressView {
        let total = self.total.load(Ordering::Relaxed);
        let done = self.done.load(Ordering::Relaxed);
        let skipped = self.skipped.load(Ordering::Relaxed);
        let written = self.written.load(Ordering::Relaxed);
        // A skipped file was never sent to the writer, so it is settled the moment it is skipped.
        let settled = skipped.saturating_add(written).min(done);
        let now = Instant::now();

        let steps = self.steps.views(now);
        let reading = self.steps.with(|steps| {
            steps
                .iter()
                .any(|step| step.key == phase::READING && step.state == StepState::Running)
        });

        // Sampled only while the files are being read, which is the one step a rate describes.
        let estimate = reading
            .then(|| self.sample(now, settled, total.saturating_sub(settled)))
            .flatten();

        ProgressView {
            total,
            done,
            parsed: self.parsed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            skipped,
            written,
            waiting: done.saturating_sub(settled),
            found: self.found.load(Ordering::Relaxed),
            canceled: self.canceled.load(Ordering::Relaxed),
            stopping: self.stopping(),
            finished: self.finished.load(Ordering::Relaxed),
            error: self.error.lock().ok().and_then(|slot| slot.clone()),
            phase: self
                .phase
                .lock()
                .ok()
                .map(|slot| slot.clone())
                .unwrap_or_default(),
            steps,
            rate: estimate.map(|(rate, _)| rate),
            remaining: estimate.map(|(_, left)| left),
            // Driven by what is in the database, not by what has been read. The bar is the answer to
            // "can I go and browse it yet?", and until a row is committed the answer is no.
            percent: if total == 0 {
                0
            } else {
                ((settled as f64 / total as f64) * 100.0).round() as u32
            },
            // Left for whoever draws it. A snapshot is taken on a worker thread, where no request
            // and so no language is in reach; `ProgressView::say_counts` is what fills these in.
            tally: String::new(),
            failed_said: None,
            waiting_said: None,
            found_said: String::new(),
            rate_said: None,
            remaining_said: None,
        }
    }

    /// Records the settled count, at most once every [`SAMPLE_EVERY`], and estimates from the window.
    fn sample(&self, now: Instant, settled: u64, remaining: u64) -> Option<(f64, Duration)> {
        let mut samples = self.samples.lock().ok()?;
        if samples
            .back()
            .is_none_or(|(at, _)| now.saturating_duration_since(*at) >= SAMPLE_EVERY)
        {
            samples.push_back((now, settled));
        }
        while samples
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > RATE_WINDOW)
        {
            samples.pop_front();
        }
        estimate(&samples, remaining)
    }

    /// Lists the steps a run with these options will take, all of them waiting.
    fn plan(&self, options: &ScanOptions) {
        self.steps.plan(planned_steps(options));
    }

    /// Names the phase now running, and closes the step before it.
    ///
    /// The status line and the checklist are set together because a phase boundary is the only
    /// moment both are known.
    fn say(&self, phase: &'static str) {
        self.steps.say(phase);
        if let Ok(mut slot) = self.phase.lock() {
            *slot = phase.to_owned();
        }
    }

    /// Marks steps this run will not take, where they have not been reached.
    fn skip(&self, keys: &[&str]) {
        self.steps.skip(keys);
    }

    /// Closes the run: the step still running is over, and a step never reached is skipped.
    ///
    /// The run ends by naming a *state* — `finished` or `stopped` — rather than another step, so this
    /// is separate from [`Progress::say`], which would time how long it took to notice the run was
    /// over.
    fn end(&self, failed: bool) {
        self.steps.end(failed);
        if let Ok(mut slot) = self.phase.lock() {
            *slot = if self.canceled.load(Ordering::Relaxed) {
                phase::STOPPED
            } else {
                phase::FINISHED
            }
            .to_owned();
        }
    }

    /// Every finished step and how long it took, in the order they ran.
    ///
    /// For the corpus measurements, which print them; the page reads the steps themselves.
    #[cfg(test)]
    pub fn timings(&self) -> Vec<(String, Duration)> {
        self.steps.with(|steps| {
            steps
                .iter()
                .filter(|step| step.state == StepState::Done)
                .filter_map(|step| step.took.map(|took| (step.key.to_owned(), took)))
                .collect()
        })
    }

    /// Asks the run to stop after the files already read have been written.
    pub fn ask_to_stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Whether stopping has been asked for.
    pub fn stopping(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A snapshot of a running or finished scan.
#[derive(Debug, Clone, Default)]
pub struct ProgressView {
    /// Files found.
    pub total: u64,
    /// Files dealt with.
    pub done: u64,
    /// Files parsed this run.
    pub parsed: u64,
    /// Files that did not parse.
    pub failed: u64,
    /// Files skipped as unchanged.
    pub skipped: u64,
    /// Files whose rows are committed.
    pub written: u64,
    /// Files read but not yet committed — the writer's backlog.
    pub waiting: u64,
    /// Files the walk has found so far.
    pub found: u64,
    /// Whether the run stopped because it was asked to.
    pub canceled: bool,
    /// Whether stopping has been asked for, whether or not the run has got there yet.
    pub stopping: bool,
    /// Whether the run has ended.
    pub finished: bool,
    /// What went wrong, if anything.
    pub error: Option<String>,
    /// What it is doing now.
    pub phase: String,
    /// Every step of the run, in order, with its duration already written out for reading.
    ///
    /// Formatted here rather than in the template because the template has no arithmetic and a
    /// `Duration` rendered by its `Debug` is `1.153412s`.
    pub steps: Vec<StepView>,
    /// Files settled a second, over the last half minute of reading.
    pub rate: Option<f64>,
    /// How long the files not yet settled take at that rate.
    pub remaining: Option<Duration>,
    /// Completion, 0 to 100.
    pub percent: u32,
    /// The counts, as one sentence. Set by [`Self::say_counts`].
    pub tally: String,
    /// How many did not parse, where any did. Set by [`Self::say_counts`].
    pub failed_said: Option<String>,
    /// How many are read and not yet written, where any are. Set by [`Self::say_counts`].
    pub waiting_said: Option<String>,
    /// How many files the walk has found. Set by [`Self::say_counts`].
    pub found_said: String,
    /// The rate, where there is one. Set by [`Self::say_counts`].
    pub rate_said: Option<String>,
    /// The time left, where there is an estimate. Set by [`Self::say_counts`].
    pub remaining_said: Option<String>,
}

impl ProgressView {
    /// Whether the run reached its end, neither stopped nor failed, which the panel draws in green.
    ///
    /// A failed run also ends on [`phase::FINISHED`], so the phase alone cannot say it.
    pub fn succeeded(&self) -> bool {
        self.finished && !self.canceled && self.error.is_none()
    }

    /// Words the counts, in the language the page asking is being drawn in.
    ///
    /// **Seven numbers in one line**, which is arithmetic and so belongs in Rust rather than in
    /// markup. Called by the handler, because that is the first place a language is in reach: a
    /// snapshot is taken on a worker thread that has none.
    pub fn say_counts(&mut self, locale: km_locale::Locale) {
        let words = crate::words::messages(locale);
        // Saturating rather than wrapping: a corpus of more files than an `i64` holds is not a
        // state this reports correctly either way, and a negative count on screen is the worse of
        // the two wrong answers.
        let n = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
        self.tally = words
            .msg_with(
                "scan-tally",
                &[
                    ("done", n(self.done).into()),
                    ("total", n(self.total).into()),
                    ("percent", i64::from(self.percent).into()),
                    ("written", n(self.written).into()),
                    ("parsed", n(self.parsed).into()),
                    ("skipped", n(self.skipped).into()),
                ],
            )
            .into_owned();
        self.failed_said = (self.failed > 0).then(|| {
            words
                .msg_with("scan-failed", &[("count", n(self.failed).into())])
                .into_owned()
        });
        self.waiting_said = (self.waiting > 0).then(|| {
            words
                .msg_with("scan-waiting", &[("count", n(self.waiting).into())])
                .into_owned()
        });
        self.found_said = words
            .msg_with("scan-found", &[("count", n(self.found).into())])
            .into_owned();
        // One decimal below ten a second, where a video or a slow disk puts it, and whole numbers
        // above, where the decimal is noise.
        self.rate_said = self.rate.map(|rate| {
            let shown = if rate < 10.0 {
                (rate * 10.0).round() / 10.0
            } else {
                rate.round()
            };
            words
                .msg_with("scan-rate", &[("rate", shown.into())])
                .into_owned()
        });
        self.remaining_said = self.remaining.map(|left| {
            words
                .msg_with("scan-remaining", &[("time", rough_duration(left).into())])
                .into_owned()
        });
    }
}

/// How a scan should behave.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Re-read and re-analyze every file, even unchanged ones. Wanted after the analysis heuristics
    /// change, and never otherwise.
    pub force: bool,
    /// Only these paths, relative to the root, rather than everything under it.
    ///
    /// **A scoped run answers a different question from a whole one, and the tail passes are what
    /// tell them apart.** *Which files are gone* is `known - seen`, and a run that looked at four
    /// hundred paths of a whole corpus would answer it *everything else* — so a scoped run
    /// does not ask. It does not group near-duplicates either: that is a whole-corpus pass, and
    /// this saw a corner of one.
    ///
    /// [`Self::force`] rides with it, because re-reading files whose size and time have not moved
    /// is the whole of what a scoped run is for.
    pub only: Option<HashSet<String>>,
    /// Worker threads.
    pub jobs: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            force: false,
            only: None,
            jobs: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        }
    }
}

impl ScanOptions {
    /// Re-reads and re-analyzes exactly these paths, and nothing else.
    pub fn only(paths: HashSet<String>) -> Self {
        Self {
            force: true,
            only: Some(paths),
            ..Self::default()
        }
    }
}

/// Which songs a re-analysis covers, for [`crate::db::Db::paths_matching`].
///
/// **Every version, and that is not what the default asks for.** [`crate::db::Filter`]'s own default
/// collapses a cluster to its representative — `s.duplicate_of IS NULL` — which is right for a page
/// showing one row per recording and wrong here: a song set aside as a version of another keeps its
/// own row and its own suitability, and a change to how one is worked out changed that one too.
///
/// **Here rather than at each caller**, because the Scan page's button and `--reanalyze` are the same
/// operation and a corpus measured two different ways by two spellings of it is the failure this
/// exists to prevent. `paths_matching` picks one path per song, so what comes back is one read per
/// song and not one per copy.
pub fn every_song() -> crate::db::Filter {
    crate::db::Filter {
        versions: crate::db::VersionsFilter::All,
        ..crate::db::Filter::default()
    }
}

/// Runs a scan to completion, writing as it goes.
///
/// Blocking, and meant to be called on a thread of its own. `progress` is updated throughout so a web
/// request can report on it without touching the database.
pub fn run(
    db: &Arc<Shared>,
    options: ScanOptions,
    progress: &Arc<Progress>,
) -> Result<(), DbError> {
    let result = run_inner(db, options, progress);
    if let Err(error) = &result
        && let Ok(mut slot) = progress.error.lock()
    {
        *slot = Some(error.to_string());
    }
    // Before `finished`, so a page that sees the run over also sees every step settled.
    progress.end(result.is_err());
    progress.finished.store(true, Ordering::Relaxed);
    result
}

fn run_inner(
    db: &Arc<Shared>,
    options: ScanOptions,
    progress: &Arc<Progress>,
) -> Result<(), DbError> {
    progress.plan(&options);
    progress.say(phase::PREPARING);
    let (root, known) = {
        let db = db.lock();
        (db.root().to_path_buf(), db.known_files()?)
    };

    progress.say(phase::LOOKING);
    let mut paths = Vec::new();
    // Every kind in one walk. MIDI, video and MP3+G all belong in this list — a folder can hold all
    // three and the point of curating them together is that a mixed package is built from one place
    // — and until this was `collect_songs` it was three calls that each opened every directory
    // under the root and stated every entry, so the corpus was walked three times to answer one
    // question.
    //
    // **Both halves of an MP3+G pair are collected**, not just the audio: a paired `.cdg` needs its
    // own `files` row so the count in the folder and the count in the tool reconcile, and an
    // unpaired one needs a row to say so. `scan_one` is what keeps that from becoming two songs —
    // it files the pair under the audio and gives the `.cdg` a row with no song attached.
    let walked = km_pack::collect_songs_observed(&root, &mut paths, &mut |found| {
        progress.found.store(found as u64, Ordering::Relaxed);
        if progress.stopping() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    // **A walk that was stopped found part of the folder**, so nothing may be read or concluded from
    // its list — least of all which files are gone. Nothing was written either, so there is no
    // folder tree to bring up to date.
    if walked.is_break() {
        progress.canceled.store(true, Ordering::Relaxed);
        return Ok(());
    }
    // Narrowed after the walk rather than instead of it, so a scoped run reaches a file by the same
    // route a whole one does and cannot disagree with it about what is there. The `.cdg` half of a
    // pair falls out of the set — a song's path is its audio — and keeps the row it has, which is
    // right: `scan_one` folds the graphics into the audio's own numbers as it reads them.
    if let Some(only) = &options.only {
        paths.retain(|path| only.contains(&relative_path(&root, path)));
    }
    paths.sort();
    progress.total.store(paths.len() as u64, Ordering::Relaxed);
    progress.say(phase::READING);

    // A corpus scan would otherwise print a panic message per bad file. Recorded and counted
    // instead, exactly as `km-lyrics scan` does.
    //
    // **Filtered rather than silenced, and the difference is that this process is also a web
    // server.** `set_hook` is process-wide, a scan is minutes to hours, and the server answers
    // throughout — so a blanket `|_| {}` meant that for the whole of a scan, a panic in any axum
    // handler on any tokio worker printed nothing at all. Nothing else would have said so either:
    // `State::blocking` recovers poisoned mutexes by design, so there was no poisoned-lock symptom
    // to notice, and what reached the person was a 500 with no line anywhere explaining it.
    //
    // The flag is set only around the one `catch_unwind` this exists for — see `scan_one` — so the
    // silence covers parsing a corpus file and nothing else, not even the rest of these threads.
    let previous_hook: Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send> =
        Arc::from(std::panic::take_hook());
    let while_scanning = Arc::clone(&previous_hook);
    std::panic::set_hook(Box::new(move |info| {
        if !PARSING_A_CORPUS_FILE.get() {
            while_scanning(info);
        }
    }));

    let seen: Vec<String> = paths
        .iter()
        .map(|path| relative_path(&root, path))
        .collect();

    // Bounded, where this was once unbounded. The readers are far faster than the single writer, so
    // an unbounded channel let them run the whole corpus into memory: on a full-corpus run the queue
    // reached hundreds of thousands of parsed songs, which cost memory nobody had budgeted, made `done`
    // a claim about reading rather than about the database, and — the reason it is bounded now —
    // meant stopping cleanly would have taken minutes of draining. A few batches of slack is all a
    // pipeline needs to keep the writer busy; past that the readers should wait.
    let (sender, receiver) = mpsc::sync_channel::<ScannedFile>(BATCH * 4);
    let writer_db = Arc::clone(db);
    let writer_progress = Arc::clone(progress);

    // One writer thread owning the connection. `rusqlite::Connection` cannot be shared, and batching
    // in one place is both simpler and faster than contending a mutex per row.
    let writer = std::thread::spawn(move || -> Result<(), DbError> {
        let now = timestamp();
        let mut batch = Vec::with_capacity(BATCH);
        for file in receiver {
            match file.status {
                ScanStatus::Ok => writer_progress.parsed.fetch_add(1, Ordering::Relaxed),
                _ => writer_progress.failed.fetch_add(1, Ordering::Relaxed),
            };
            batch.push(file);
            if batch.len() >= BATCH {
                {
                    let mut db = writer_db.lock();
                    db.write_scanned(&batch, &now)?;
                }
                writer_progress
                    .written
                    .fetch_add(batch.len() as u64, Ordering::Relaxed);
                batch.clear();
                // **Between batches, with the connection released.** This is the one point in the
                // reading phase where a write somebody asked for can get in, so it is where the
                // scan asks whether anybody is waiting. See `stand_off`.
                stand_off(&writer_db);
            }
        }
        if !batch.is_empty() {
            let mut db = writer_db.lock();
            db.write_scanned(&batch, &now)?;
            writer_progress
                .written
                .fetch_add(batch.len() as u64, Ordering::Relaxed);
        }
        Ok(())
    });

    let jobs = options.jobs.max(1);
    let chunk_size = paths.len().div_ceil(jobs).max(1);
    std::thread::scope(|scope| {
        for chunk in paths.chunks(chunk_size) {
            let sender = sender.clone();
            let progress = Arc::clone(progress);
            let known = &known;
            let root = &root;
            scope.spawn(move || {
                for path in chunk {
                    // Between files, not inside one: a single file is milliseconds, and stopping
                    // half-way through parsing one would gain nothing and complicate everything.
                    if progress.stopping() {
                        return;
                    }
                    let relative = relative_path(root, path);
                    let (mut size, mut mtime) = stat(path);

                    // **For a paired song the unit scanned is the pair, so the skip test has to
                    // cover both halves.** An MP3+G song is filed under its audio, and replacing
                    // only the `.cdg` leaves that audio's size and modification time untouched — so
                    // without this the song keeps the graphics facts of a file that is no longer
                    // there, and a re-scan silently does nothing. Folding the graphics into the
                    // audio's own numbers needs no schema change and no second lookup.
                    if km_pack::is_audio(path)
                        && let Some(graphics) = km_pack::pair_for(path)
                    {
                        let (graphics_size, graphics_mtime) = stat(&graphics);
                        size = size.saturating_add(graphics_size);
                        mtime = mtime.max(graphics_mtime);
                    } else if (km_pack::is_audio(path) || km_pack::is_video(path))
                        && let Some(text) = km_pack::ultrastar_naming(path)
                    {
                        // **The same fold for a file an UltraStar song claims**, and it does a second
                        // job: a row that read this MP3 as half a pair, or this video as a song, was
                        // written before its `.txt` counted, so the size differs and the file is read
                        // again once. Only an unpaired MP3 or a video pays the folder read.
                        let (text_size, text_mtime) = stat(&text);
                        size = size.saturating_add(text_size);
                        mtime = mtime.max(text_mtime);
                    }

                    // **Unchanged means both the bytes and the answer.** The size and the time say
                    // the file is the one that was read; the revision says this build would write
                    // the same row about it. A heuristic that moved leaves every row stale while
                    // every file is untouched, and before the revision was here there was nothing in
                    // a scan that could notice — the only correction available was to re-read the
                    // whole corpus, hours of it, chosen by hand.
                    if !options.force
                        && let Some(known) = known.get(&relative)
                        && known.size == size
                        && known.mtime == mtime
                        && known.analysis_revision == Some(km_suitability::ANALYSIS_REVISION)
                    {
                        progress.skipped.fetch_add(1, Ordering::Relaxed);
                        progress.done.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }

                    let scanned = scan_one(path, relative, size, mtime);
                    // Counted here rather than in the writer: `done` means "dealt with", and a batch
                    // that fails to write must not leave the bar reading zero after the whole corpus
                    // has been read.
                    progress.done.fetch_add(1, Ordering::Relaxed);
                    // A closed receiver means the writer died; there is nothing useful to do with
                    // the rest of the chunk, so stop.
                    if sender.send(scanned).is_err() {
                        return;
                    }
                }
            });
        }
    });
    drop(sender);

    let write_result = writer
        .join()
        .unwrap_or_else(|_| Err(DbError::Rejected("the scan writer thread died".to_owned())));
    // A wrapper around the original rather than the original box, because installing the filter
    // above had to share it. Indistinguishable in behaviour: the flag is false everywhere by the
    // time this runs, and nothing outside `scan_one` ever sets it.
    std::panic::set_hook(Box::new(move |info| previous_hook(info)));
    write_result?;

    // A stopped run has read only part of the corpus, so none of what follows is a conclusion it is
    // entitled to draw. Forgetting would in fact be safe — the walk finishes before the reading
    // starts, so `seen` is every path found on disk whatever the run got through, and the `gone` set
    // below is therefore complete however early this stopped — but recording `last_scan` would claim
    // pairs from a half-filled table, and recording `last_scan` would claim the folder had been read
    // when it had not. The rows that were written stay written, and the next scan picks up the rest
    // for free.
    if progress.stopping() {
        // The folder tree is the one tail pass a partial read *is* entitled to run: it is not a
        // conclusion about the corpus, it is a description of the rows that were written, and rows
        // were written. Left alone, the Folders page would rebuild it on the next visit anyway —
        // slowly, while somebody waited.
        progress.skip(&[phase::FORGETTING, phase::DUPLICATES, phase::MEASURING]);
        progress.say(phase::INDEXING);
        {
            let db = db.lock();
            db.rebuild_folders()?;
        }
        progress.canceled.store(true, Ordering::Relaxed);
        return Ok(());
    }

    // **Only a run that looked at the whole corpus may conclude anything about the whole corpus**,
    // which is the one rule a scoped run turns on. The set difference below is `known - seen`, and
    // a run handed four hundred paths of a whole corpus has `seen` of four hundred — so
    // asking it *which files are gone* answers *everything else*, and the next two statements would
    // delete the corpus.
    let mut removed = 0usize;
    if options.only.is_none() {
        progress.say(phase::FORGETTING);
        // **The set difference is computed here, in memory, rather than by the database.** `known`
        // is every path the `files` table held when this run started and `seen` is every path the
        // walk found, so the rows to delete are `known - seen` and both sides are already in hand.
        // Handing all of the walk's paths to SQLite instead — a temp table filled a row at a time,
        // then two anti-joins scanning `files` and `songs` whole — is minutes of the tool frozen,
        // and on the ordinary scan where nothing has been deleted, minutes to delete nothing.
        let present: HashSet<&str> = seen.iter().map(String::as_str).collect();
        let gone: Vec<String> = known
            .keys()
            .filter(|path| !present.contains(path.as_str()))
            .cloned()
            .collect();
        // `known` is a snapshot from before the first row was written, which is safe rather than
        // merely tolerable: every row this run wrote is for a path in `seen`, so no row it wrote
        // can be in `gone`.
        for chunk in gone.chunks(BATCH) {
            // Re-locked per chunk, and stood aside between them, exactly as the writer above does
            // per batch. The tail is the only stretch of a scan that holds the writing connection
            // for longer than one batch, so without this a curation action arriving during it waits
            // out the whole of a whole-corpus delete.
            {
                let mut db = db.lock();
                let (files, _) = db.forget_missing(chunk)?;
                removed += files;
            }
            stand_off(db);
        }
        if removed > 0 {
            let db = db.lock();
            db.forget_orphaned_songs()?;
        }
    }

    // **Nothing below this line can change a thing unless a row did**, and until now all of it ran
    // on every completed scan regardless. A re-scan of an untouched corpus read the whole `songs`
    // table to compute suggestions identical to the stored ones, rebuilt a folder tree from
    // unchanged paths, and measured a corpus whose shape had not moved.
    let written = progress.written.load(Ordering::Relaxed);
    let changed = written > 0 || removed > 0;
    if !changed {
        progress.skip(&[phase::DUPLICATES, phase::INDEXING, phase::MEASURING]);
    }

    // Unconditional, unlike everything around it: this records that the folder was *read*, which is
    // true of a scan that found nothing to do — and untrue of a scoped run, which read a corner of
    // it and must not leave a stamp claiming otherwise.
    if options.only.is_none() {
        let db = db.lock();
        db.set_setting("last_scan", &timestamp())?;
    }

    // **A full re-analysis ends by looking for near-duplicates; a changed-file scan does not.**
    // Bucketing hundreds of thousands of fingerprints is a whole-`songs` read, so what decides where
    // it belongs is who asked for one. A scan that found a file that moved did not, and the
    // suggestions it would produce are almost exactly the stored ones — the button on the
    // Duplicates page is where that is asked for. A forced scan did: it has just rewritten every
    // fingerprint in the corpus, so the stored grouping stands on shapes that are no longer there,
    // and 10.4 s over the whole corpus is nothing beside the read it is the tail of.
    //
    // **The suggesting runs outside the lock**, between two short holds, exactly as the button's
    // handler does it. It is pure CPU over a vector it owns and touches no database, and the tool
    // answers pages throughout a scan.
    // **A scoped run is not one of the two things that ask**, however forced it is: it re-read a
    // corner of the corpus, and the pass reads all of it.
    if options.force && changed && options.only.is_none() {
        progress.say(phase::DUPLICATES);
        let prints = db.lock().fingerprints()?;
        let pairs = crate::dupes::suggest(&prints);
        stand_off(db);
        let mut db = db.lock();
        db.store_candidates(&pairs)?;
        // Grouping is what the rest of the tool reads — the pairs are only ever an input to it — so
        // a suggestion stored without being grouped would change nothing anybody can see.
        db.cluster()?;
    }

    if changed {
        // After `last_scan`, not before: the folder index stamps itself with that timestamp to know
        // whether it is still current, so rebuilding first would leave it marked stale and rebuild
        // again on the first visit to the Folders page.
        progress.say(phase::INDEXING);
        {
            let db = db.lock();
            db.rebuild_folders()?;
        }
        // Between the two whole-corpus holds that end a scan. Neither can be taken in chunks — a
        // folder tree is one pass and `ANALYZE` is one statement — so standing aside between them is
        // the only place a write arriving in the tail can be let through.
        stand_off(db);

        // A scan is the one thing that changes the shape of this database rather than its contents,
        // so it is the moment the planner's statistics are most out of date — and on a first scan
        // they describe an empty table. Without this the browse page would sort the whole corpus
        // until the next start, which is precisely the session somebody has just filled a corpus and
        // wants to look at it.
        progress.say(phase::MEASURING);
        {
            let db = db.lock();
            db.refresh_statistics();
        }
    }

    Ok(())
}

/// Probes one video file, in a build that can read them.
///
/// There is no analysis here and there is nothing to add: a video has no channels to separate, no
/// lyric timeline to measure and no melody to find, so what the database learns is what the
/// container says about itself. No `catch_unwind` either, unlike the MIDI path — that exists because
/// this project's own parser is fed a whole corpus of hostile files, whereas ffmpeg reports a bad
/// file as an error.
#[cfg(feature = "video")]
fn scan_video(path: &Path, relative: String, size: u64, mtime: i64, hash: String) -> ScannedFile {
    match km_video::probe(path) {
        Ok(info) => ScannedFile {
            size,
            mtime,
            content_hash: Some(hash.clone()),
            status: ScanStatus::Ok,
            error: None,
            song: Some(ScannedSong {
                id: hash,
                // The container's own tags, when it has any. A video downloaded with its metadata
                // embedded carries both, so it arrives in curation already named; one fetched
                // without that usually carries neither and falls back to `stem`, which is the same
                // situation the MIDI corpus is in and is settled the same way.
                //
                // Through the gate every detected name passes, so a tag that is a row of marks is
                // no more a title here than inside a MIDI file.
                det_title: info.title.as_deref().and_then(km_song::clean_meta_name),
                det_artist: info.artist.as_deref().and_then(km_song::clean_meta_name),
                // Still nothing. No container records the language of the *singing*, and inferring
                // it from a title would be a guess wearing a fact's clothes.
                det_language: None,
                stem: km_pack::file_stem(Path::new(&relative)),
                duration_ms: info.duration_ms,
                // Never any: the words are pixels in somebody else's picture, which is the whole
                // reason a video song is played rather than transcribed.
                lyrics: None,
                // The near-duplicate suggester reads a MIDI file's structure, and there is no
                // counterpart for a video. Byte-identical copies still land on one row, because that
                // is the content hash and not this.
                fingerprint: String::new(),
                // Its own length stands in for how much of it is sung: a video's words are pixels,
                // so there is no span to read out of it.
                suitability: crate::model::SuitabilityFacts::purpose_made(info.duration_ms),
                midi: None,
                video: Some(VideoFacts {
                    width: info.width,
                    height: info.height,
                    frame_rate_milli: info.frame_rate_milli,
                    video_codec: info.video_codec,
                    audio_codec: info.audio_codec,
                }),
                cdg: None,
                ultrastar: None,
            }),
            path: relative,
        },
        Err(error) => ScannedFile {
            path: relative,
            size,
            mtime,
            content_hash: Some(hash),
            status: ScanStatus::NotVideo,
            error: Some(error.to_string()),
            song: None,
        },
    }
}

/// Records a video file as unreadable, in a build with no `video` feature.
///
/// Said out loud rather than passed over: a folder of MP4s that indexes as nothing at all, with no
/// explanation, looks like a broken scan. The row appears with a reason a person can act on.
#[cfg(not(feature = "video"))]
fn scan_video(_path: &Path, relative: String, size: u64, mtime: i64, hash: String) -> ScannedFile {
    ScannedFile {
        path: relative,
        size,
        mtime,
        content_hash: Some(hash),
        status: ScanStatus::VideoUnsupported,
        error: Some("rebuild with `--features video` to scan and package video songs".to_owned()),
        song: None,
    }
}

/// Probes an MP3+G pair, from the audio half.
///
/// **No `#[cfg]` pair, unlike [`scan_video`] above**, and the absence is the point: `km-cdg` is pure
/// Rust, so there is no build of this tool that can find a pair and be unable to read it.
///
/// The identity is the hash of **both** files, because the same backing track under two different
/// `.cdg` files is two different karaoke songs. That is why the caller's hash of the audio alone is
/// discarded here rather than reused.
fn scan_cdg(audio: &Path, relative: String, size: u64, mtime: i64) -> ScannedFile {
    let fail = |status, error: String| ScannedFile {
        path: relative.clone(),
        size,
        mtime,
        content_hash: None,
        status,
        error: Some(error),
        song: None,
    };

    let Some(graphics) = km_pack::pair_for(audio) else {
        // **The audio half of an UltraStar song is not half a pair.** Its song is filed under the
        // `.txt` that names it, so this row says only that the file is accounted for.
        if km_pack::ultrastar_naming(audio).is_some() {
            return claimed(relative, size, mtime);
        }
        return fail(
            ScanStatus::MissingGraphics,
            "no .cdg beside it, so it has no words to sing".to_owned(),
        );
    };

    // **Both halves are read exactly once, and everything below works from those bytes.** The song's
    // identity is the hash of the two files together, so they have to be in memory whatever else
    // happens; probing from a path as well read the MP3 a second time and the `.cdg` a second time,
    // for bytes already in hand. Reading first also means the hash is taken before the `Vec` is
    // handed to symphonia, which is what lets the audio be moved rather than copied.
    let (Ok(audio_bytes), Ok(graphics_bytes)) = (std::fs::read(audio), std::fs::read(&graphics))
    else {
        return fail(
            ScanStatus::Unreadable,
            "one half of the pair could not be read".to_owned(),
        );
    };
    let hash = km_kmpkg::pair_content_hash(&audio_bytes, &graphics_bytes);

    let audio_name = audio.display().to_string();
    let graphics_name = graphics.display().to_string();
    let info = match km_cdg::probe_from(
        std::io::Cursor::new(audio_bytes),
        &audio_name,
        &graphics_bytes,
        &graphics_name,
    ) {
        Ok(info) => info,
        Err(error) => {
            // Which half failed decides which status, because they send a person to different
            // places: a bad MP3 is replaced, a bad `.cdg` is re-ripped.
            let status = match &error {
                km_cdg::CdgError::NoPackets { .. } => ScanStatus::BadGraphics,
                _ => ScanStatus::NotAudio,
            };
            return fail(status, error.to_string());
        }
    };

    if info.graphics.tiles_written == 0 {
        return fail(
            ScanStatus::BadGraphics,
            "the .cdg never draws a tile, so the pair has no words in it".to_owned(),
        );
    }

    ScannedFile {
        size,
        mtime,
        content_hash: Some(hash.clone()),
        status: ScanStatus::Ok,
        error: None,
        song: Some(ScannedSong {
            id: hash,
            // What the file said, so a person can see it and overrule it the same way they overrule
            // a parsed MIDI title -- and, like one, only where it says something. Which tag is
            // *believed* over which stem stays `km-pack`'s business; whether a string is a name at
            // all is settled once, for every kind of song, by the gate below.
            det_title: info
                .audio
                .title
                .as_deref()
                .and_then(km_song::clean_meta_name),
            det_artist: info
                .audio
                .artist
                .as_deref()
                .and_then(km_song::clean_meta_name),
            // CD+G carries no text at all — the words are one-bit tiles — and ID3's `TLAN` frame
            // appeared in none of the measured corpus. Somebody has to say.
            det_language: None,
            stem: km_pack::file_stem(Path::new(&relative)),
            // From the audio, always, and counted rather than read from its header: one corpus file
            // overstates its own length sevenfold.
            duration_ms: info.audio.duration_ms,
            // The words exist and are not text. Nothing to index.
            lyrics: None,
            // Structural fingerprinting reads a MIDI file's notes; there is no counterpart. Exact
            // copies still land on one row, through the pair hash above.
            fingerprint: String::new(),
            // The audio's length, for the same reason a video's stands in: CD+G words are one-bit
            // tiles with no timing to read. Never `graphics_ms`, which starts at a title card.
            suitability: crate::model::SuitabilityFacts::purpose_made(info.audio.duration_ms),
            midi: None,
            video: None,
            ultrastar: None,
            cdg: Some(CdgFacts {
                graphics_path: graphics
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                sample_rate: info.audio.sample_rate,
                channels: info.audio.channels,
                packets: info.graphics.packets,
                graphics_ms: info.graphics.duration_ms,
                graphics_short_by_ms: info.graphics_short_by_ms(),
                tiles_written: info.graphics.tiles_written,
                unknown_instructions: info.graphics.unknown_instructions,
            }),
        }),
        path: relative,
    }
}

/// A row for a file that belongs to a song filed under another file, as a paired `.cdg` does.
///
/// Every file the scan walked gets a row, which is what makes the count in the folder and the count
/// in the tool reconcilable; this one is not a failure and is not a song.
fn claimed(relative: String, size: u64, mtime: i64) -> ScannedFile {
    ScannedFile {
        path: relative,
        size,
        mtime,
        content_hash: None,
        status: ScanStatus::Ok,
        error: None,
        song: None,
    }
}

/// Reads one `.txt`: an UltraStar song, a file an UltraStar song refuses, or a text file that is
/// neither.
///
/// **Filed under the `.txt`**, because it is the half that names the other. The identity is the hash
/// of the audio and the text together, as `km-pack` records it, so a song's timing fixed in its
/// `.txt` is a different song from the one it replaced.
fn scan_ultrastar(text: &Path, relative: String, size: u64, mtime: i64) -> ScannedFile {
    let fail = |status, error: String| ScannedFile {
        path: relative.clone(),
        size,
        mtime,
        content_hash: None,
        status,
        error: Some(error),
        song: None,
    };

    let source = match km_pack::read_ultrastar(text) {
        Ok(source) => source,
        // A readme is not a song and not a failure. Song folders hold them.
        Err(km_pack::UltraStarRefusal::NotUltraStar) => return claimed(relative, size, mtime),
        Err(error @ km_pack::UltraStarRefusal::Unreadable(_)) => {
            return fail(ScanStatus::Unreadable, error.to_string());
        }
        Err(error @ km_pack::UltraStarRefusal::Refused(_)) => {
            return fail(ScanStatus::BadUltraStar, error.to_string());
        }
        Err(
            error @ (km_pack::UltraStarRefusal::VideoOnly(_)
            | km_pack::UltraStarRefusal::NotMp3(_)
            | km_pack::UltraStarRefusal::AudioMissing(_)),
        ) => return fail(ScanStatus::UltraStarAudio, error.to_string()),
    };

    let info = match km_cdg::probe_audio(&source.audio) {
        Ok(info) => info,
        Err(error) => return fail(ScanStatus::NotAudio, error.to_string()),
    };
    let Ok(hash) = km_kmpkg::pair_content_hash_of(&source.audio, text) else {
        return fail(
            ScanStatus::Unreadable,
            "the MP3 it names could not be read".to_owned(),
        );
    };

    let song = &source.song;
    let plain = song.timeline.plain_text();
    ScannedFile {
        size,
        mtime,
        content_hash: Some(hash.clone()),
        status: ScanStatus::Ok,
        error: None,
        song: Some(ScannedSong {
            id: hash,
            det_title: km_song::clean_meta_name(&song.title),
            det_artist: song.artist.as_deref().and_then(km_song::clean_meta_name),
            // A name such as `English`, which `Language::detect` reads as the declared language.
            det_language: song.language.clone(),
            stem: km_pack::file_stem(Path::new(&relative)),
            duration_ms: info.duration_ms,
            lyrics: (!plain.trim().is_empty()).then_some(plain),
            fingerprint: String::new(),
            // The one media kind whose span is read rather than stood in for: a person timed these
            // words to this recording, so the file says how much of it is sung.
            suitability: crate::model::SuitabilityFacts::purpose_made(
                km_suitability::sung_span_ms(&km_song::ultrastar::song_from_timeline(
                    song.timeline.clone(),
                )),
            ),
            midi: None,
            video: None,
            cdg: None,
            ultrastar: Some(UltraStarFacts {
                line_count: u32::try_from(song.timeline.line_count()).unwrap_or(u32::MAX),
                syllable_count: u32::try_from(song.timeline.syllable_count()).unwrap_or(u32::MAX),
                det_encoding: song.decoder.name().to_owned(),
                det_encoding_source: format!("{:?}", song.decoder.source()).to_lowercase(),
            }),
        }),
        path: relative,
    }
}

/// Reads and analyzes one file.
///
/// Every failure mode ends up as a status on the row rather than an error out of the function: over a
/// real corpus, a scan that stops at the first unreadable file is a scan that never finishes.
pub fn scan_one(path: &Path, relative: String, size: u64, mtime: i64) -> ScannedFile {
    // **Before the read, not after.** An MP3+G pair is scanned from its audio half, so each pair
    // produces exactly one row and the `.cdg` is reached by rule; and its identity is the hash of
    // *both* files, which `scan_cdg` computes for itself. So the whole-file read and hash below are
    // work this branch cannot use — reading every MP3 in the corpus to throw the bytes away. A
    // `.cdg` that nothing claimed gets a row of its own further down, saying so: the two halves of
    // "this folder is not what you think it is" are different sentences.
    if km_pack::is_audio(path) {
        return scan_cdg(path, relative, size, mtime);
    }
    if km_pack::is_ultrastar_candidate(path) {
        return scan_ultrastar(path, relative, size, mtime);
    }
    // A singing game's music video is not a video song: it has no words in its picture, and the
    // UltraStar file that names it is the song.
    if km_pack::is_video(path) && km_pack::ultrastar_naming(path).is_some() {
        return claimed(relative, size, mtime);
    }

    let Ok(bytes) = std::fs::read(path) else {
        return ScannedFile {
            path: relative,
            size,
            mtime,
            content_hash: None,
            status: ScanStatus::Unreadable,
            error: Some("could not be read".to_owned()),
            song: None,
        };
    };

    // Hashing before parsing, as `km-pack build` does: it is far cheaper, and it is what makes
    // identical copies land on one song row with no grouping pass to get wrong.
    let hash = content_hash(&bytes);

    // A video is probed rather than parsed, and by ffmpeg from the path rather than from the bytes
    // already in hand — reading a 50 MB file twice, which is the price of `km_video::probe` taking a
    // path. Unlike the audio branch above this read is not wasted: the hash *is* the song's identity
    // here, so the bytes have to be gone through either way. Cheap in practice: an incremental scan
    // reads a file at most once ever, because size and modification time settle it thereafter.
    if km_pack::is_video(path) {
        return scan_video(path, relative, size, mtime, hash);
    }

    if km_pack::is_graphics(path) {
        let orphan = km_pack::pair_for(path).is_none();
        return ScannedFile {
            path: relative,
            size,
            mtime,
            content_hash: Some(hash),
            // A paired `.cdg` is not a failure, it is simply not the file the song is filed under.
            // It still gets a row, because every file the scan walked gets one — that is what makes
            // the count in the folder and the count in the tool reconcilable.
            status: if orphan {
                ScanStatus::OrphanGraphics
            } else {
                ScanStatus::Ok
            },
            error: orphan.then(|| "no audio beside it to sing over".to_owned()),
            song: None,
        };
    }

    // The one call the scan's panic hook is allowed to be quiet about. Cleared immediately after:
    // `catch_unwind` returns rather than propagating, so there is no unwind path around this that
    // would leave the flag set and silence a later, real panic on this thread.
    PARSING_A_CORPUS_FILE.set(true);
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        Song::parse(&bytes, &ParseOptions::default())
    }));
    PARSING_A_CORPUS_FILE.set(false);

    match parsed {
        Ok(Ok(song)) => {
            let analysis = Analysis::of(&song);
            ScannedFile {
                size,
                mtime,
                content_hash: Some(hash.clone()),
                status: ScanStatus::Ok,
                error: None,
                // Before `path`, so the stem can be taken from `relative` while it is still owned
                // here. Field order in a struct literal is evaluation order.
                song: Some(describe(hash, &relative, &song, &analysis)),
                path: relative,
            }
        }
        Ok(Err(error)) => ScannedFile {
            path: relative,
            size,
            mtime,
            content_hash: Some(hash),
            status: ScanStatus::NotMidi,
            error: Some(error.to_string()),
            song: None,
        },
        Err(_) => ScannedFile {
            path: relative,
            size,
            mtime,
            content_hash: Some(hash),
            status: ScanStatus::Panicked,
            error: Some("the parser panicked on this file".to_owned()),
            song: None,
        },
    }
}

/// Turns a parsed song and its analysis into the row the database stores.
///
/// `relative` is the file's path under the root, and is here only for its stem: most of a real corpus
/// has no title meta event, and the file's own name is the only name those songs have. It is kept
/// apart from `det_title`, which stays answerable to "what did the file say?".
fn describe(id: String, relative: &str, song: &Song, analysis: &Analysis) -> ScannedSong {
    let warnings: Vec<crate::model::StoredWarning> = analysis
        .suitability
        .warnings
        .iter()
        .map(|warning| crate::model::StoredWarning {
            code: km_pack::warning_code(warning.code),
            message: warning.message.clone(),
        })
        .collect();
    let suitability = crate::model::SuitabilityFacts {
        value: analysis.suitability.value,
        breakdown: (
            analysis.suitability.breakdown.lyrics,
            analysis.suitability.breakdown.sync,
            analysis.suitability.breakdown.channels,
            analysis.suitability.breakdown.arrangement,
        ),
        warnings: serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".to_owned()),
    };

    ScannedSong {
        id,
        det_title: song.meta.title.clone(),
        det_artist: song.meta.artist.clone(),
        det_language: song.meta.language.clone(),
        // The same helper `km-pack` uses when it builds a package, so what the browse list calls a
        // song and what lands in the manifest cannot disagree.
        stem: km_pack::file_stem(Path::new(relative)),
        duration_ms: song.duration_ms(),
        // The one place the words themselves are kept. `plain_text` has said "for search indexing"
        // in its doc comment since km-song was written and until now nothing indexed anything.
        // `None` rather than `Some("")` for an instrumental, so the FTS trigger skips it.
        lyrics: {
            let text = song.lyrics.plain_text();
            (!text.trim().is_empty()).then_some(text)
        },
        fingerprint: fingerprint(song),
        suitability,
        midi: Some(MidiFacts {
            flavor: format!("{:?}", song.flavor).to_lowercase(),
            granularity: format!("{:?}", song.lyrics.granularity()).to_lowercase(),
            note_count: song.note_count() as u32,
            channel_count: song.sounding_channels().len() as u32,
            line_count: song.lyrics.line_count() as u32,
            syllable_count: song.lyrics.syllable_count() as u32,
            det_encoding: song.decoder.name().to_owned(),
            det_encoding_source: format!("{:?}", song.decoder.source()).to_lowercase(),
            melody_channel: analysis.melody_channel(),
            melody_confidence: analysis.melody.channel().map(|melody| melody.confidence),
            melody_abstained: km_pack::melody_abstained(analysis),
        }),
        video: None,
        cdg: None,
        ultrastar: None,
    }
}

/// A path relative to the root, with forward slashes.
///
/// Stored this way so the corpus folder can move, or be reached from another machine, without every
/// row becoming wrong.
pub fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Size and modification time, both zero when the file cannot be stat'ed.
///
/// A file that cannot be stat'ed will fail to be read a moment later anyway, and zeroes guarantee the
/// incremental check treats it as changed rather than silently skipping it forever.
fn stat(path: &Path) -> (u64, i64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (0, 0);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    (meta.len(), mtime)
}

/// An ISO-8601 timestamp to the second, in UTC.
///
/// Hand-rolled for the same reason `km-app` hand-rolls one: the only thing this needs a wall clock
/// for is stamping a row, and a date library for that is not worth the dependency.
pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    let (hour, minute, second) = (time / 3600, (time % 3600) / 60, time % 60);

    // Civil-from-days, the standard algorithm, with the epoch shifted to 0000-03-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Scratch;

    #[test]
    fn a_relative_path_uses_forward_slashes() {
        assert_eq!(
            relative_path(Path::new("/corpus"), Path::new("/corpus/a/b.kar")),
            "a/b.kar"
        );
    }

    #[test]
    fn a_path_outside_the_root_is_kept_whole_rather_than_mangled() {
        assert_eq!(
            relative_path(Path::new("/corpus"), Path::new("/elsewhere/b.kar")),
            "/elsewhere/b.kar"
        );
    }

    #[test]
    fn a_missing_file_is_recorded_rather_than_fatal() {
        let scanned = scan_one(
            Path::new("/definitely/not/here.kar"),
            "here.kar".to_owned(),
            0,
            0,
        );
        assert_eq!(scanned.status, ScanStatus::Unreadable);
        assert!(scanned.song.is_none());
    }

    /// Every file in an UltraStar song folder is accounted for, and only a refused song is a failure.
    ///
    /// The MP3 an UltraStar file names is not half a pair, and a readme beside it is not a song. The
    /// song itself needs audio that decodes, which no synthetic fixture here carries, so a song that
    /// reads is refused on its audio: the status names the MP3 rather than the words.
    #[test]
    fn an_ultrastar_folder_reads_as_one_song_and_no_failures_but_its_own() {
        let scratch = Scratch::new("ultrastar-folder");
        scratch.write(
            "Someone - Song.txt",
            b"#TITLE:Song\n#ARTIST:Someone\n#MP3:Someone - Song.mp3\n#BPM:300\n: 0 4 0 la\nE\n",
        );
        scratch.write("Someone - Song.mp3", b"not really audio");
        scratch.write("ReadMe!.txt", b"Thanks for downloading.");
        scratch.write(
            "duet.txt",
            b"#TITLE:Duet\n#MP3:Someone - Song.mp3\n#BPM:300\nP1\n: 0 4 0 me\nP2\n: 4 4 0 you\nE\n",
        );
        let scan = |name: &str| scan_one(&scratch.0.join(name), name.to_owned(), 1, 0);

        let audio = scan("Someone - Song.mp3");
        assert_eq!((audio.status, audio.song.is_none()), (ScanStatus::Ok, true));
        let readme = scan("ReadMe!.txt");
        assert_eq!(
            (readme.status, readme.song.is_none()),
            (ScanStatus::Ok, true)
        );
        assert_eq!(scan("duet.txt").status, ScanStatus::BadUltraStar);
        assert_eq!(scan("Someone - Song.txt").status, ScanStatus::NotAudio);
    }

    #[test]
    fn a_timestamp_is_iso_8601_to_the_second() {
        let stamp = timestamp();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
        // Sanity: this tool did not exist before 2026 and the year field is four digits.
        let year: i64 = stamp[..4].parse().expect("a year");
        assert!((2024..3000).contains(&year), "{stamp}");
    }

    /// A tagged video reaches the database already knowing what it is.
    ///
    /// The point of a download writing container tags, proved on the scanning side: without this,
    /// a video song's title is its file name and its artist is blank until somebody types one in.
    /// The fixture is `km-video`'s, borrowed rather than copied — one tagged MP4 in the workspace
    /// is enough, and two would drift.
    #[cfg(feature = "video")]
    #[test]
    fn a_videos_container_tags_become_its_detected_title_and_artist() {
        let tagged = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/playback/km-video/fixtures/tagged.mp4");
        let bytes = std::fs::read(&tagged).expect("the km-video fixture is where it always was");

        let scratch = Scratch::new("video-tags");
        // Named nothing like its title on purpose: if the stem were leaking through instead of the
        // tags, this test would still pass with a name like `Howl - Aeng Moo Sae.mp4`.
        scratch.write("clip-0001.mp4", &bytes);

        let scanned = scan_one(
            &scratch.0.join("clip-0001.mp4"),
            "clip-0001.mp4".to_owned(),
            bytes.len() as u64,
            0,
        );

        assert_eq!(scanned.status, ScanStatus::Ok);
        let song = scanned.song.expect("a video that probes is a song");
        assert_eq!(song.det_title.as_deref(), Some("Aeng Moo Sae"));
        assert_eq!(song.det_artist.as_deref(), Some("Howl"));
        assert_eq!(song.stem, "clip-0001", "the stem is still the fallback");
        assert_eq!(
            song.det_language, None,
            "no container says what language the singing is in"
        );
    }

    /// Removing a reason accepts the files failing that way now, and a later one is still news.
    ///
    /// **The second half is the whole design.** Dismissing the reason itself would be one line
    /// less code and would make the ninth bad file silent — which is the thing the list exists to
    /// prevent, and it would fail silently, because nothing about a message that never appears
    /// looks wrong.
    ///
    /// The red badge in the bar is asserted alongside, because it reads the same count from a
    /// different query: a removal that cleared the table and left the badge lit would leave
    /// somebody no way at all to put it out.
    #[test]
    fn removing_a_reason_accepts_the_files_failing_that_way_now() {
        let scratch = Scratch::new("dismiss-failures");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());
        scratch.write("junk.kar", b"this is not a MIDI file at all");
        scratch.write("junk2.kar", b"nor is this one");

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        let rescan =
            || run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("scan");
        rescan();

        let guard = db.lock();
        let failures = guard.failure_tally().expect("tally");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].count, 2);
        assert_eq!(guard.counts().expect("counts").failed, 2);
        let status = failures[0].status.clone();

        // Removing accepts both, so nothing is left to show and the badge goes out.
        assert_eq!(
            guard
                .dismiss_failures(&status, &timestamp())
                .expect("remove"),
            2
        );
        assert!(guard.failure_tally().expect("tally").is_empty());
        assert_eq!(guard.counts().expect("counts").failed, 0);

        // ...and they are offered back, counted, so the line below the table can name them.
        let dismissed = guard.dismissed_tally().expect("dismissed");
        assert_eq!(dismissed.len(), 1);
        assert_eq!(dismissed[0].count, 2);
        assert_eq!(dismissed[0].status, status);
        drop(guard);

        // A rescan finds the same two files failing the same way, and they stay accepted.
        rescan();
        {
            let guard = db.lock();
            assert!(
                guard.failure_tally().expect("tally").is_empty(),
                "a rescan must not undo a verdict somebody passed"
            );
            assert_eq!(guard.counts().expect("counts").failed, 0);
        }

        // A third bad file is news, and it is counted on its own rather than joining a total
        // nobody would look at again.
        scratch.write("junk3.kar", b"a new one nobody has seen");
        rescan();
        {
            let guard = db.lock();
            let failures = guard.failure_tally().expect("tally");
            assert_eq!(failures.len(), 1);
            assert_eq!(
                failures[0].count, 1,
                "only the file nobody has accepted: got {failures:?}"
            );
            assert!(failures[0].example.ends_with("junk3.kar"));
            assert_eq!(guard.counts().expect("counts").failed, 1);

            // Restoring puts every file accepted under that reason back.
            assert_eq!(guard.restore_failures(&status).expect("restore"), 2);
            assert_eq!(guard.failure_tally().expect("tally")[0].count, 3);
            assert!(guard.dismissed_tally().expect("dismissed").is_empty());
            assert_eq!(guard.counts().expect("counts").failed, 3);
        }
    }

    /// A file that starts failing a different way is counted again.
    ///
    /// A dismissal is a verdict on one failure, not a permanent exemption for the file: a `.kar`
    /// accepted as unreadable that later fails to parse a different way is a different thing to
    /// know, and staying quiet about it would be answering a question nobody asked.
    #[test]
    fn a_file_that_fails_a_different_way_is_counted_again() {
        let scratch = Scratch::new("dismiss-restatus");
        scratch.write("junk.kar", b"this is not a MIDI file at all");

        let db = crate::db::Db::open_in_memory(&scratch.0).expect("open");
        let shared = Arc::new(Shared::new(db));
        run(
            &shared,
            ScanOptions::default(),
            &Arc::new(Progress::default()),
        )
        .expect("scan");

        let guard = shared.lock();
        let status = guard.failure_tally().expect("tally")[0].status.clone();
        guard
            .dismiss_failures(&status, &timestamp())
            .expect("remove");
        assert!(guard.failure_tally().expect("tally").is_empty());

        // The same file, recorded under a different failure. The dismissal names the status it was
        // passed on, so it no longer matches.
        guard
            .execute_for_test("UPDATE files SET scan_status = 'panicked'")
            .expect("restatus");
        let failures = guard.failure_tally().expect("tally");
        assert_eq!(failures.len(), 1, "got {failures:?}");
        assert_eq!(failures[0].status, "panicked");
        assert!(
            guard.dismissed_tally().expect("dismissed").is_empty(),
            "the old verdict counts for nothing once the failure is a different one"
        );
    }

    /// A whole scan against a real folder, through the real database.
    ///
    /// This exists because the first version of the schema shipped a trigger that used
    /// external-content FTS5 syntax on a plain table. Every unit test passed — none of them had ever
    /// inserted a row — and the failure only appeared as "SQL logic error" when a real corpus was
    /// scanned. A test that writes is the only kind that would have caught it.
    #[test]
    fn a_scan_writes_songs_files_and_the_search_index() {
        let scratch = Scratch::new("scan");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());
        scratch.write("nested/b.mid", &km_song::testing::lyric_events());
        // The same bytes under a second name: one song, two files, with no grouping pass involved.
        scratch.write("nested/a-copy.kar", &km_song::testing::soft_karaoke());
        scratch.write("junk.kar", b"this is not a MIDI file at all");

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        let progress = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &progress).expect("the scan must succeed");

        let view = progress.snapshot();
        assert!(view.finished);
        assert_eq!(view.error, None, "the scan reported an error");
        assert_eq!(view.total, 4);
        assert_eq!(view.done, 4);
        assert_eq!(view.parsed, 3);
        assert_eq!(
            view.failed, 1,
            "the non-MIDI file must be counted, not fatal"
        );

        let guard = db.lock();
        let counts = guard.counts().expect("counts");
        assert_eq!(counts.files, 4, "every file gets a row, failures included");
        assert_eq!(counts.songs, 2, "the doubled fixture is one song");
        assert_eq!(counts.failed, 1);

        // The search index is written by a trigger, so this is what proves the trigger runs. The
        // term is taken from a row that was just written rather than hard-coded, so the test says
        // "searching finds what was indexed" instead of pinning a fixture's title.
        let all = guard
            .songs(&crate::db::Filter::default())
            .expect("browse everything");
        let titled = all
            .iter()
            .find(|row| !row.title.trim().is_empty())
            .expect("at least one song has a title");
        let word = titled
            .title
            .split_whitespace()
            .next()
            .expect("a first word")
            .to_owned();

        let found = guard
            .songs(&crate::db::Filter {
                query: Some(word.clone()),
                ..crate::db::Filter::default()
            })
            .expect("search");
        assert!(
            found.iter().any(|row| row.id == titled.id),
            "searching for {word:?} did not find {:?}, so the FTS trigger did not fire",
            titled.title
        );

        // The duplicate is visible as two paths on one song — which is the whole of what the tool
        // says about byte-identical files now that the Duplicates page has gone: an id *is* the hash
        // of the bytes, so copies land on one row by construction and the count is a column on it.
        // `2-10` is the bar's own bucket for that, where `AtLeastTwo` was a fourth arm only that page
        // could reach.
        let duplicated = guard
            .songs(&crate::db::Filter {
                copies: crate::db::CopiesFilter::AtLeastTwo,
                ..crate::db::Filter::default()
            })
            .expect("duplicates");
        assert_eq!(duplicated.len(), 1);
        assert_eq!(duplicated[0].file_count, 2);
    }

    /// A scan writes the words, and the words can be searched.
    ///
    /// Written as a whole scan for the reason the test above it gives: the lyric index is filled by
    /// a trigger, and a trigger that is never fired is a trigger every unit test agrees with and no
    /// real corpus does. This also pins the property the whole page rests on — that a song is found
    /// by a line nobody typed anywhere in its title.
    #[test]
    fn a_scan_indexes_the_words_and_they_can_be_searched() {
        let scratch = Scratch::new("lyric-search");
        // Sings "Mary had a little lamb / Its fleece was white as snow" and is called nothing of the
        // sort, so a hit can only have come from the words.
        scratch.write("nested/UNTITLED1.mid", &km_song::testing::lyric_events());
        scratch.write("twinkle.kar", &km_song::testing::soft_karaoke());
        scratch.write("silent.mid", &km_song::testing::untitled_instrumental());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("scan");

        let guard = db.lock();
        assert!(
            guard.lyrics_indexed().expect("indexed"),
            "a scan that read a file with lyrics must leave something in the index"
        );

        let search = |words: &str| {
            guard
                .lyric_search(&crate::db::LyricSearch {
                    query: words.to_owned(),
                    limit: 25,
                    offset: 0,
                })
                .expect("lyric search")
        };

        let hits = search("fleece");
        assert_eq!(hits.len(), 1, "one song sings about a fleece");
        assert!(hits[0].song.path.ends_with("UNTITLED1.mid"));
        assert_eq!(
            guard
                .lyric_search_count(&crate::db::LyricSearch {
                    query: "fleece".to_owned(),
                    limit: 25,
                    offset: 0,
                })
                .expect("count"),
            1,
            "the count and the rows have to agree, or the paging lies"
        );

        // The passage is the point of the page, not a detail of it: a hit with no words shown is a
        // song list, and a song list is what the Songs page already is.
        let runs = crate::views::highlight(&hits[0].passage);
        assert!(
            runs.iter()
                .any(|(text, matched)| *matched && text.to_lowercase().contains("fleece")),
            "the matched word must come back marked, got {runs:?}"
        );

        // Every term has to appear, which is what makes a remembered line narrow rather than widen.
        assert_eq!(search("fleece twinkle").len(), 0);
        assert_eq!(search("fleece snow").len(), 1);

        // An empty box is no search at all, not a search matching everything.
        assert!(search("   ").is_empty());
        // ...and neither is punctuation, which would otherwise be an FTS5 syntax error.
        assert!(search("\"*(").is_empty());

        // The instrumental is not in the index. Most of a real corpus is instrumental, and several
        // hundred thousand empty rows would be space spent recording that they say nothing.
        let indexed: i64 = guard
            .count_for_test("SELECT COUNT(*) FROM lyrics_fts")
            .expect("count the index");
        assert_eq!(indexed, 2, "only the two files with words are indexed");
    }

    /// Deleting a song takes its words out of the index with it.
    ///
    /// The insert trigger is conditional and the delete trigger is not, which is the pairing that
    /// makes this work; a symmetric `WHEN` on both would leave the row behind forever.
    #[test]
    fn removing_a_song_removes_its_words() {
        let scratch = Scratch::new("lyric-drop");
        scratch.write("gone.mid", &km_song::testing::lyric_events());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("scan");

        {
            let guard = db.lock();
            assert_eq!(
                guard
                    .count_for_test("SELECT COUNT(*) FROM lyrics_fts")
                    .expect("count"),
                1
            );
        }

        std::fs::remove_file(scratch.0.join("gone.mid")).expect("delete the file");
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("re-scan");

        let guard = db.lock();
        assert_eq!(guard.counts().expect("counts").songs, 0);
        assert_eq!(
            guard
                .count_for_test("SELECT COUNT(*) FROM lyrics_fts")
                .expect("count"),
            0,
            "a song that is gone must not still be findable by its words"
        );
        assert!(!guard.lyrics_indexed().expect("indexed"));
    }

    /// A file with no metadata is shown under its own name, and is findable by it.
    ///
    /// Pointed at the top of a real corpus this is not an edge case but the common one: most of the
    /// files are plain `.mid` with no title meta event at all, and the browse table used to render
    /// them as an empty, unclickable link — hundreds of thousands of rows with nothing in them, all
    /// sorted to the front because the empty string sorts first.
    #[test]
    fn a_song_with_no_metadata_is_titled_and_searchable_by_its_file_name() {
        let scratch = Scratch::new("stem");
        scratch.write(
            "nested/CORCOVAD.mid",
            &km_song::testing::untitled_instrumental(),
        );
        scratch.write("titled.kar", &km_song::testing::soft_karaoke());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("scan");

        let guard = db.lock();
        let all = guard
            .songs(&crate::db::Filter::default())
            .expect("browse everything");
        let nameless = all
            .iter()
            .find(|row| row.path.ends_with("CORCOVAD.mid"))
            .expect("the instrumental was scanned");

        assert_eq!(
            nameless.title, "CORCOVAD",
            "a file with no title must fall back to its own name, without the extension"
        );
        assert!(
            nameless.from_filename,
            "the row has to say the title is only the file's name, or a curator cannot tell it \
             apart from one somebody checked"
        );
        assert_eq!(nameless.artist, None, "no artist is ever invented");

        // The detected column is untouched: `det_title` answers "what did the file say?", and the
        // edit form's "(the file says nothing)" placeholder depends on that staying true.
        let detail = guard.song(&nameless.id).expect("the detail page");
        assert_eq!(detail.det_title, None);
        assert_eq!(detail.effective_title(), "CORCOVAD");
        assert!(detail.title_is_filename());

        // And the search box finds it, which is the whole reason the stem is a column rather than a
        // display-time flourish.
        let found = guard
            .songs(&crate::db::Filter {
                query: Some("corcovad".to_owned()),
                ..crate::db::Filter::default()
            })
            .expect("search");
        assert!(
            found.iter().any(|row| row.id == nameless.id),
            "searching for the file's name must find it"
        );

        // Sorting by title now interleaves it with the real ones instead of clumping every nameless
        // song at the front.
        let by_title = guard
            .songs(&crate::db::Filter {
                sort: crate::db::Sort::Title,
                ..crate::db::Filter::default()
            })
            .expect("sort by title");
        let titles: Vec<&str> = by_title.iter().map(|row| row.title.as_str()).collect();
        let mut sorted = titles.clone();
        sorted.sort_unstable();
        assert_eq!(
            titles, sorted,
            "the order must match the titles being shown"
        );
    }

    /// The bar counts what is in the database, not what has been read off the disk.
    ///
    /// The readers run ahead of the single writer through an unbounded channel. Over a whole corpus
    /// that gap reached hundreds of thousands of rows: the Scan page said 100% while browsing kept
    /// finding new songs for minutes afterwards, which reads as a broken counter rather than as a
    /// scan that has not finished.
    #[test]
    fn progress_reflects_rows_committed_rather_than_files_read() {
        let progress = Progress::default();
        progress.total.store(100, Ordering::Relaxed);
        progress.done.store(100, Ordering::Relaxed);

        let view = progress.snapshot();
        assert_eq!(view.percent, 0, "nothing is written yet");
        assert_eq!(view.waiting, 100);

        progress.written.store(60, Ordering::Relaxed);
        let view = progress.snapshot();
        assert_eq!(view.percent, 60);
        assert_eq!(view.waiting, 40);

        progress.written.store(100, Ordering::Relaxed);
        let view = progress.snapshot();
        assert_eq!(view.percent, 100);
        assert_eq!(view.waiting, 0, "no backlog once everything is committed");
    }

    /// A file skipped as unchanged never reaches the writer, so it is settled the moment it is
    /// skipped — otherwise a second scan would sit at 0% forever.
    #[test]
    fn a_skipped_file_counts_as_settled_without_being_written() {
        let progress = Progress::default();
        progress.total.store(10, Ordering::Relaxed);
        progress.done.store(10, Ordering::Relaxed);
        progress.skipped.store(10, Ordering::Relaxed);

        let view = progress.snapshot();
        assert_eq!(view.percent, 100);
        assert_eq!(view.waiting, 0);
        assert_eq!(view.written, 0);
    }

    /// Each step's key and state, for asserting on a whole checklist at once.
    fn states(view: &ProgressView) -> Vec<(&str, &str)> {
        view.steps
            .iter()
            .map(|step| (step.key.as_str(), step.state))
            .collect()
    }

    /// A finished run leaves no step waiting, and says which ones it did not need.
    ///
    /// **The unchanged rescan is the case that matters**: it is the ordinary scan, and its tail steps
    /// are skipped because nothing moved. A page that left them *waiting* would say the run still had
    /// work to do after it had ended.
    #[test]
    fn a_finished_scan_settles_every_step_and_skips_what_nothing_needed() {
        let scratch = Scratch::new("steps");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        let first = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &first).expect("first scan");
        assert_eq!(
            states(&first.snapshot()),
            [
                (phase::PREPARING, "done"),
                (phase::LOOKING, "done"),
                (phase::READING, "done"),
                (phase::FORGETTING, "done"),
                (phase::INDEXING, "done"),
                (phase::MEASURING, "done"),
            ]
        );
        assert_eq!(first.snapshot().found, 1);
        assert!(
            first
                .snapshot()
                .steps
                .iter()
                .all(|step| step.took.is_some()),
            "a step that ran says how long it took"
        );
        assert_eq!(first.timings().len(), 6);

        let second = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &second).expect("second scan");
        assert_eq!(
            states(&second.snapshot()),
            [
                (phase::PREPARING, "done"),
                (phase::LOOKING, "done"),
                (phase::READING, "done"),
                (phase::FORGETTING, "done"),
                (phase::INDEXING, "skipped"),
                (phase::MEASURING, "skipped"),
            ]
        );

        // A forced run lists the grouping of duplicates, and a scoped one lists neither it nor
        // forgetting, because it can never run them.
        let forced = Arc::new(Progress::default());
        run(
            &db,
            ScanOptions {
                force: true,
                ..ScanOptions::default()
            },
            &forced,
        )
        .expect("forced scan");
        assert!(states(&forced.snapshot()).contains(&(phase::DUPLICATES, "done")));

        let scoped = Arc::new(Progress::default());
        run(
            &db,
            ScanOptions::only(HashSet::from(["a.kar".to_owned()])),
            &scoped,
        )
        .expect("scoped run");
        let view = scoped.snapshot();
        let keys: Vec<&str> = states(&view).into_iter().map(|(key, _)| key).collect();
        assert!(!keys.contains(&phase::FORGETTING), "{keys:?}");
        assert!(!keys.contains(&phase::DUPLICATES), "{keys:?}");
    }

    /// The plan is on the page before the run takes its first step.
    #[test]
    fn the_steps_to_come_are_listed_before_they_run() {
        let progress = Progress::default();
        progress.plan(&ScanOptions::default());
        progress.say(phase::PREPARING);

        let view = progress.snapshot();
        assert_eq!(view.steps[0].state, "running");
        assert!(
            view.steps[0].took.is_some(),
            "a running step shows its clock"
        );
        assert!(view.steps[1..].iter().all(|step| step.state == "waiting"));
        assert!(
            view.steps
                .iter()
                .filter(|step| step.if_changed)
                .map(|step| step.key.as_str())
                .eq([phase::INDEXING, phase::MEASURING]),
            "{:?}",
            view.steps
        );
    }

    /// A run that fails marks the step it failed in, and the steps it never reached as skipped.
    #[test]
    fn a_failed_run_marks_the_step_it_failed_in() {
        let progress = Progress::default();
        progress.plan(&ScanOptions::default());
        progress.say(phase::PREPARING);
        progress.say(phase::LOOKING);
        progress.end(true);

        let view = progress.snapshot();
        assert_eq!(view.steps[0].state, "done");
        assert_eq!(view.steps[1].state, "failed");
        assert!(view.steps[2..].iter().all(|step| step.state == "skipped"));
        assert_eq!(view.phase, phase::FINISHED);
    }

    /// No estimate from too little, and the right one from a steady rate.
    #[test]
    fn the_time_left_is_estimated_from_the_recent_rate() {
        let start = Instant::now();
        let at = |seconds: u64, settled: u64| (start + Duration::from_secs(seconds), settled);

        assert_eq!(estimate(&VecDeque::new(), 100), None);
        assert_eq!(
            estimate(&VecDeque::from([at(0, 0), at(5, 50)]), 100),
            None,
            "five seconds is too little to estimate from"
        );
        assert_eq!(
            estimate(&VecDeque::from([at(0, 10), at(20, 10)]), 100),
            None,
            "nothing moving is no rate at all"
        );

        let (rate, left) =
            estimate(&VecDeque::from([at(0, 0), at(10, 100), at(20, 200)]), 600).expect("a rate");
        assert!((rate - 10.0).abs() < 1e-9, "{rate}");
        assert_eq!(left, Duration::from_secs(60));
    }

    /// The rate is taken only while files are being read, and is worded with the time left.
    #[test]
    fn a_rate_is_said_only_while_reading() {
        let progress = Progress::default();
        progress.plan(&ScanOptions::default());
        progress.say(phase::READING);
        progress.total.store(1_000, Ordering::Relaxed);
        progress.done.store(100, Ordering::Relaxed);
        progress.skipped.store(100, Ordering::Relaxed);
        {
            let mut samples = progress.samples.lock().expect("samples");
            let back = Instant::now() - Duration::from_secs(20);
            samples.push_back((back, 0));
        }

        let mut view = progress.snapshot();
        assert_eq!(view.rate.map(f64::round), Some(5.0));
        view.say_counts(km_locale::Locale::English);
        assert!(view.rate_said.is_some() && view.remaining_said.is_some());

        progress.say(phase::INDEXING);
        let view = progress.snapshot();
        assert_eq!(
            view.rate, None,
            "a rate describes reading and nothing after it"
        );
    }

    /// A scan asked to stop writes what it read and abandons the rest, rather than being killed.
    ///
    /// Leaving the tool used to kill the scan thread wherever it stood — the process exits when
    /// `main` returns, and nothing ever joined it — so the batch in hand and everything queued
    /// behind it were lost. Stopping before the first file even starts is the extreme case and the
    /// easy one to assert on: nothing is read, nothing is claimed, and the database is untouched but
    /// valid.
    #[test]
    fn a_scan_asked_to_stop_before_it_starts_reads_nothing_and_claims_nothing() {
        let scratch = Scratch::new("stop-early");
        for i in 0..20 {
            scratch.write(&format!("{i}.kar"), &km_song::testing::soft_karaoke());
        }

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        let progress = Arc::new(Progress::default());
        progress.ask_to_stop();
        run(&db, ScanOptions::default(), &progress).expect("stopping is not a failure");

        let view = progress.snapshot();
        assert!(view.finished);
        assert!(view.canceled, "the run must own up to having been stopped");
        assert_eq!(view.phase, phase::STOPPED, "and must not say it finished");
        assert_eq!(view.done, 0);
        assert_eq!(view.error, None);

        let guard = db.lock();
        // The tail passes are conclusions a partial read is not entitled to draw. `last_scan`
        // staying unset is the one that matters: it is what tells somebody the folder still needs
        // reading.
        assert_eq!(guard.setting("last_scan").expect("setting"), None);
    }

    /// Stopping partway keeps every row already committed, and the next run finishes the job.
    #[test]
    fn a_stopped_scan_keeps_what_it_wrote_and_the_next_one_carries_on() {
        let scratch = Scratch::new("stop-resume");
        for i in 0..8 {
            scratch.write(&format!("{i}.kar"), &km_song::testing::soft_karaoke());
            scratch.write(&format!("m{i}.mid"), &km_song::testing::lyric_events());
        }

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));

        // Stop it the moment it has begun. How far it gets is a race and is not asserted on — what
        // matters is that whatever it got is committed and consistent.
        let stopped = Arc::new(Progress::default());
        let watcher = Arc::clone(&stopped);
        let stopper = std::thread::spawn(move || {
            while watcher.snapshot().total == 0 && !watcher.snapshot().finished {
                std::thread::yield_now();
            }
            watcher.ask_to_stop();
        });
        run(&db, ScanOptions::default(), &stopped).expect("stopping is not a failure");
        stopper.join().expect("the stopper thread");

        {
            let guard = db.lock();
            let counts = guard.counts().expect("counts");
            // Whatever was read was written: no row is half-there, because a batch is a transaction.
            assert_eq!(
                u64::from(counts.files),
                stopped.snapshot().written,
                "every file the writer took must be in the database"
            );
        }

        // A second run, not asked to stop, completes the folder.
        let rest = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &rest).expect("second scan");
        let view = rest.snapshot();
        assert!(!view.canceled);
        assert_eq!(view.phase, phase::FINISHED);

        let guard = db.lock();
        assert_eq!(guard.counts().expect("counts").files, 16);
        assert!(
            guard.setting("last_scan").expect("setting").is_some(),
            "a run that reached the end records that it did"
        );
    }

    /// The second run must read nothing, which is what makes a large corpus workable.
    #[test]
    fn a_second_scan_skips_everything_unchanged() {
        let scratch = Scratch::new("rescan");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());
        scratch.write("b.mid", &km_song::testing::lyric_events());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        let first = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &first).expect("first scan");
        assert_eq!(first.snapshot().parsed, 2);
        assert_eq!(first.snapshot().skipped, 0);

        let second = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &second).expect("second scan");
        let view = second.snapshot();
        assert_eq!(view.skipped, 2, "unchanged files must not be re-read");
        assert_eq!(view.parsed, 0);

        // ...unless asked, which is what `--force` is for after the analysis changes.
        let forced = Arc::new(Progress::default());
        run(
            &db,
            ScanOptions {
                force: true,
                ..ScanOptions::default()
            },
            &forced,
        )
        .expect("forced scan");
        assert_eq!(forced.snapshot().parsed, 2);
        assert_eq!(forced.snapshot().skipped, 0);
    }

    /// A run scoped to one file re-reads that file and concludes nothing about the rest.
    ///
    /// **The assertion that matters is the second one.** *Which files are gone* is `known - seen`,
    /// so a scoped run that ran the pass would have `seen` of one path and would answer *everything
    /// else* — deleting a corpus to recompute one song's suitability.
    #[test]
    fn a_scoped_run_reads_what_it_was_given_and_forgets_nothing() {
        let scratch = Scratch::new("scoped");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());
        scratch.write("b.mid", &km_song::testing::lyric_events());
        scratch.write("c.kar", &km_song::testing::high_quality_song());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("first scan");
        let stamped = {
            let guard = db.lock();
            assert_eq!(guard.counts().expect("counts").files, 3);
            guard.setting("last_scan").expect("setting")
        };

        let scoped = Arc::new(Progress::default());
        run(
            &db,
            ScanOptions::only(HashSet::from(["a.kar".to_owned()])),
            &scoped,
        )
        .expect("scoped run");

        let view = scoped.snapshot();
        assert_eq!(view.total, 1, "it looked at the one file it was given");
        assert_eq!(view.parsed, 1, "and read it again, unchanged though it is");
        assert_eq!(view.skipped, 0);

        let guard = db.lock();
        assert_eq!(
            guard.counts().expect("counts").files,
            3,
            "the files it was not given are still here"
        );
        assert_eq!(
            guard.setting("last_scan").expect("setting"),
            stamped,
            "a corner of the corpus is not a reading of the corpus, so the stamp does not move"
        );
    }

    /// A scan re-reads a song this build would answer differently, though nothing on disk moved.
    ///
    /// **The round trip is the whole feature**, so it is asserted as one: a second scan of an
    /// untouched folder reads nothing, the same folder as an older build left it reads everything,
    /// and the scan after that reads nothing again. Miss the middle step and a changed heuristic
    /// never reaches a corpus; miss either of the others and every scan is a reading of the whole
    /// corpus, which on the real one is hours.
    ///
    /// `analysis_revision = NULL` is exactly what a row written before the column existed carries,
    /// so this is the shape every existing corpus arrives in rather than an invented one.
    #[test]
    fn a_song_an_older_analysis_decided_is_read_again_though_its_file_has_not_moved() {
        let scratch = Scratch::new("stale-analysis");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());
        scratch.write("b.mid", &km_song::testing::lyric_events());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("first scan");

        let unchanged = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &unchanged).expect("second scan");
        assert_eq!(
            unchanged.snapshot().skipped,
            2,
            "nothing moved and nothing moved on"
        );
        assert_eq!(unchanged.snapshot().parsed, 0);

        db.lock()
            .execute_for_test("UPDATE songs SET analysis_revision = NULL")
            .expect("age the rows");

        let stale = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &stale).expect("third scan");
        assert_eq!(
            stale.snapshot().parsed,
            2,
            "a row nothing can vouch for is read again, whatever its file says"
        );
        assert_eq!(stale.snapshot().skipped, 0);

        let settled = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &settled).expect("fourth scan");
        assert_eq!(
            settled.snapshot().skipped,
            2,
            "and having been read, it is current: the cost is paid once per revision, not per scan"
        );
    }

    /// A scan stores the suitability, and a song too short to be worth choosing lands under the
    /// default band.
    ///
    /// **The browse list, the band filter and the sort all read the stored column**, so what a scan
    /// writes is what a curator sees and what the `WHERE` clause means. The two files here have the
    /// same words, the same timing and the same arrangement, and differ only in how long they run.
    #[test]
    fn a_scan_stores_what_a_song_is_worth_and_a_short_one_falls_below_the_band() {
        let scratch = Scratch::new("stored-suitability");
        scratch.write("long.kar", &km_song::testing::high_quality_song());
        scratch.write("short.kar", &km_song::testing::a_complete_short_song());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("scan");

        let guard = db.lock();
        let stored = |path: &str| {
            guard
                .count_for_test(&format!(
                    "SELECT suitability FROM songs
                      WHERE id = (SELECT song_id FROM files WHERE path = '{path}')"
                ))
                .expect("read the suitability")
        };
        assert_eq!(stored("long.kar"), 10);
        // Three rather than four: forty seconds is under the length a song is plausibly played for
        // as well, so the arrangement loses its second point to a separate rule about the backing.
        assert_eq!(stored("short.kar"), 3);

        let band = |filter: crate::db::SuitabilityFilter| {
            guard
                .songs(&crate::db::Filter {
                    suitability: filter,
                    ..crate::db::Filter::default()
                })
                .expect("browse")
                .len()
        };
        assert_eq!(band(crate::db::SuitabilityFilter::High), 1);
        assert_eq!(
            band(crate::db::SuitabilityFilter::Low),
            1,
            "the short one is where somebody looking for what is wrong would find it"
        );
    }

    /// A row climbs through every revision that cannot reach it and stops at the first that can.
    ///
    /// Revision 2 reaches a song with syllables and revision 3 one with 8 lyric lines, so a row
    /// short of both climbs past them. **Where it stops is the newest revision that reaches it**, and
    /// the newest of all reaches every song, so the climb is what this asserts rather than a scan
    /// avoided, and the count of songs still stale afterwards is all of them.
    #[test]
    fn a_song_climbs_to_the_first_revision_that_can_change_it() {
        let scratch = Scratch::new("revision-reach");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());
        scratch.write("b.mid", &km_song::testing::lyric_events());
        scratch.write("c.kar", &km_song::testing::high_quality_song());
        scratch.write("d.kar", &km_song::testing::chord_names_as_lyrics());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("first scan");

        {
            let guard = db.lock();
            for (path, revision, line_count, syllable_count) in [
                // Short, from the build before the chord-chart revision: promoted.
                ("a.kar", "2", "3", "30"),
                // No counts, from the first build: climbs every revision with a bound on one.
                ("b.mid", "1", "NULL", "NULL"),
                // Long enough to be a chord chart: stays.
                ("c.kar", "2", "20", "200"),
                // Short but sung, from the first build: the melody revision reaches it, so it stays.
                ("d.kar", "1", "3", "40"),
            ] {
                guard
                    .execute_for_test(&format!(
                        "UPDATE songs SET analysis_revision = {revision}, line_count = {line_count},
                                          syllable_count = {syllable_count}
                          WHERE id = (SELECT song_id FROM files WHERE path = '{path}')"
                    ))
                    .expect("shape the row");
            }
            assert_eq!(guard.stale_analysis_count().expect("count"), 4);

            guard.promote_unreached_revisions().expect("promote");
            guard
                .promote_unreached_revisions()
                .expect("and again, changing nothing");

            // Where each row stopped, which is the first revision that could change it.
            let revision = |path: &str| {
                guard
                    .count_for_test(&format!(
                        "SELECT analysis_revision FROM songs
                          WHERE id = (SELECT song_id FROM files WHERE path = '{path}')"
                    ))
                    .expect("read the revision")
            };
            assert_eq!(revision("a.kar"), 5, "short of every count, so it climbed");
            assert_eq!(
                revision("b.mid"),
                5,
                "no counts at all, so every bound clears it"
            );
            assert_eq!(revision("c.kar"), 2, "long enough for the chord-chart rule");
            assert_eq!(revision("d.kar"), 1, "the melody revision reaches it");

            assert_eq!(
                guard.stale_analysis_count().expect("count"),
                4,
                "the newest revision reaches every song, so every one is still stale"
            );
        }

        let progress = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &progress).expect("second scan");
        assert_eq!(progress.snapshot().parsed, 4, "every song");
        assert_eq!(progress.snapshot().skipped, 0);
        assert_eq!(db.lock().stale_analysis_count().expect("count"), 0);
    }

    /// A row no build vouches for is not promoted: nothing says which revision it would climb from.
    #[test]
    fn a_song_with_no_revision_is_not_promoted() {
        let scratch = Scratch::new("revision-reach-null");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("first scan");

        let guard = db.lock();
        guard
            .execute_for_test("UPDATE songs SET analysis_revision = NULL, line_count = 0")
            .expect("age the rows");
        guard.promote_unreached_revisions().expect("promote");
        assert_eq!(guard.stale_analysis_count().expect("count"), 1);
    }

    /// A file with no song is skipped by the next scan, like a song is.
    ///
    /// A MIDI file that does not parse, a readme `.txt` and an orphan `.cdg` are the three shapes of
    /// a row with no song. Asking only the song's revision made every one of them fail the skip test,
    /// so a changed-file scan read all of them again, every time.
    #[test]
    fn a_file_with_no_song_is_not_read_again() {
        let scratch = Scratch::new("songless-skip");
        scratch.write("a.kar", &km_song::testing::soft_karaoke());
        scratch.write("broken.mid", b"not a midi file");
        scratch.write("readme.txt", b"These songs came from a friend.");
        scratch.write("orphan.cdg", &[0u8; 96]);

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("first scan");

        let unchanged = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &unchanged).expect("second scan");
        let view = unchanged.snapshot();
        assert_eq!(view.skipped, 4, "nothing moved, song or no song");
        assert_eq!(view.parsed + view.failed, 0);

        db.lock()
            .execute_for_test("UPDATE files SET analysis_revision = NULL")
            .expect("age the rows");
        let stale = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &stale).expect("third scan");
        let view = stale.snapshot();
        assert_eq!(
            (view.skipped, view.parsed + view.failed),
            (1, 3),
            "a song-less row nothing vouches for is read again; the song answers from its own row"
        );

        {
            let guard = db.lock();
            guard
                .execute_for_test("UPDATE files SET analysis_revision = 1 WHERE song_id IS NULL")
                .expect("age the rows");
            guard.promote_unreached_revisions().expect("promote");
        }
        let promoted = Arc::new(Progress::default());
        run(&db, ScanOptions::default(), &promoted).expect("fourth scan");
        assert_eq!(
            promoted.snapshot().skipped,
            4,
            "a revision with a limited reach cannot reach a file with no song"
        );
    }

    /// A changed-file scan leaves the grouping where it stands; a full re-analysis redoes it.
    #[test]
    fn only_a_forced_scan_looks_for_near_duplicates() {
        const CREDIT: &[u8] = b"(c) 2026 nobody";

        let scratch = Scratch::new("near-duplicates");
        let first = km_song::testing::soft_karaoke();
        // The same song re-saved with a different credit: different bytes and therefore a different
        // song, with the same shape and the same title. That is the pair the pass exists to find,
        // and the credit is the one field in the fixture that changes neither.
        let second = {
            let mut bytes = first.clone();
            let at = bytes
                .windows(CREDIT.len())
                .position(|window| window == CREDIT)
                .expect("the fixture's credit");
            bytes[at..at + CREDIT.len()].copy_from_slice(b"(c) 2026 nobodz");
            bytes
        };
        scratch.write("a.kar", &first);
        scratch.write("b.kar", &second);

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("first scan");
        {
            let guard = db.lock();
            assert_eq!(guard.counts().expect("counts").files, 2);
            assert_eq!(
                guard.cluster_counts().expect("counts").clusters,
                0,
                "a scan that reads changed files does not pay for a whole-corpus pass"
            );
        }

        let forced = Arc::new(Progress::default());
        run(
            &db,
            ScanOptions {
                force: true,
                ..ScanOptions::default()
            },
            &forced,
        )
        .expect("forced scan");
        assert!(
            forced
                .timings()
                .iter()
                .any(|(phase, _)| phase == phase::DUPLICATES),
            "the pass is a phase of the scan, so it is timed and reported like one"
        );

        let guard = db.lock();
        let counts = guard.cluster_counts().expect("counts");
        assert_eq!(counts.clusters, 1, "the two files are one recording");
        assert_eq!(
            counts.set_aside, 1,
            "one of them is kept and the other hidden"
        );
    }

    /// A re-analysis measures every song, and a song set aside as a version of another is a song.
    ///
    /// The default filter collapses a cluster to the row a page shows, which is the right answer for
    /// a page and the wrong one here: the hidden row keeps its own suitability, and a rubric that has
    /// changed has changed it too. `--reanalyze` therefore asks for [`VersionsFilter::All`], and this
    /// is what says the two answers differ — without it the flag would quietly measure a fraction of
    /// the corpus and report having finished.
    #[test]
    fn a_version_set_aside_is_still_a_song_to_re_analyze() {
        const CREDIT: &[u8] = b"(c) 2026 nobody";

        let scratch = Scratch::new("reanalyze-versions");
        let first = km_song::testing::soft_karaoke();
        let second = {
            let mut bytes = first.clone();
            let at = bytes
                .windows(CREDIT.len())
                .position(|window| window == CREDIT)
                .expect("the fixture's credit");
            bytes[at..at + CREDIT.len()].copy_from_slice(b"(c) 2026 nobodz");
            bytes
        };
        scratch.write("a.kar", &first);
        scratch.write("b.kar", &second);

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        // Forced, because clustering is the tail of a forced scan and a cluster is the whole point.
        run(
            &db,
            ScanOptions {
                force: true,
                ..ScanOptions::default()
            },
            &Arc::new(Progress::default()),
        )
        .expect("scan");

        let guard = db.lock();
        assert_eq!(
            guard.cluster_counts().expect("counts").set_aside,
            1,
            "the corpus this test needs is one where a row is hidden"
        );

        let collapsed = guard
            .paths_matching(&crate::db::Filter::default())
            .expect("collapsed");
        let every = guard
            .paths_matching(&crate::db::Filter {
                versions: crate::db::VersionsFilter::All,
                ..crate::db::Filter::default()
            })
            .expect("every version");

        assert_eq!(collapsed.len(), 1, "a page shows one row per recording");
        assert_eq!(
            every.len(),
            2,
            "both rows carry a suitability to bring up to date"
        );
    }

    /// A file that disappears is forgotten; one a package still names is kept and flagged instead.
    #[test]
    fn a_deleted_file_is_forgotten_unless_a_package_names_it() {
        let scratch = Scratch::new("forget");
        scratch.write("keep.kar", &km_song::testing::soft_karaoke());
        scratch.write("drop.mid", &km_song::testing::lyric_events());

        let db = Arc::new(Shared::new(
            crate::db::Db::open_in_memory(&scratch.0).expect("open"),
        ));
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("first scan");

        // Put the one about to vanish into a package, so it has a reason to survive.
        let doomed = {
            let guard = db.lock();
            let rows = guard.songs(&crate::db::Filter::default()).expect("songs");
            let doomed = rows
                .iter()
                .find(|row| row.path.ends_with("drop.mid"))
                .expect("the file about to be deleted")
                .id
                .clone();
            drop(guard);
            let mut guard = db.lock();
            guard
                .create_package(
                    &crate::model::PackageRow {
                        id: "vol1".to_owned(),
                        name: "Vol 1".to_owned(),
                        version: "1.0.0".to_owned(),
                        publisher: None,
                        start_number: 101,
                        default_language: None,
                        out_path: None,
                        built_at: None,
                        song_count: 0,
                        ..crate::model::PackageRow::new("", "")
                    },
                    "now",
                )
                .expect("create the package");
            guard
                .add_to_package("vol1", std::slice::from_ref(&doomed), "now")
                .expect("add");
            doomed
        };

        std::fs::remove_file(scratch.0.join("drop.mid")).expect("delete");
        run(&db, ScanOptions::default(), &Arc::new(Progress::default())).expect("second scan");

        let guard = db.lock();
        assert_eq!(guard.counts().expect("counts").files, 1, "the row is gone");
        // The song survives, with no files, because a package still names it.
        let detail = guard.song(&doomed).expect("the song is kept");
        assert!(detail.files.is_empty());
    }

    // -- standing aside for a write ---------------------------------------------------------------

    fn shared() -> Arc<Shared> {
        Arc::new(Shared::new(
            crate::db::Db::open_in_memory(Path::new("/corpus")).expect("open"),
        ))
    }

    /// With nobody waiting, a batch boundary costs nothing.
    ///
    /// The common case by far, and the one that must not slow a scan down: pages are drawn through a
    /// connection of their own, so nothing lights this unless somebody is actually curating.
    #[test]
    fn standing_aside_for_nobody_returns_at_once() {
        let db = shared();
        let started = Instant::now();
        stand_off(&db);
        assert!(
            started.elapsed() < YIELD_FOR,
            "an unwanted connection is not waited on: {:?}",
            started.elapsed()
        );
    }

    /// A waiting write gets the connection, which an unfair mutex would not have given it.
    ///
    /// **This is the test for the whole mechanism.** Releasing and immediately re-taking a
    /// `std::sync::Mutex` does not hand it over, so the scan has to be told to keep its hands off
    /// for a moment — and the shape below is the writer's loop: take it, work, release, stand aside,
    /// take it again.
    #[test]
    fn a_write_waiting_at_a_batch_boundary_gets_in() {
        let db = shared();
        let waiter = Arc::clone(&db);
        let got_in = Arc::new(AtomicBool::new(false));
        let reported = Arc::clone(&got_in);

        let held = db.lock();
        // The flag is set while the writer holds the connection, so the scan taking it back below
        // cannot see the connection free and the flag unset.
        let writing = std::thread::spawn(move || {
            if let Some(_connection) = waiter.lock_within(Duration::from_secs(5)) {
                reported.store(true, Ordering::Relaxed);
            }
        });
        // Let it queue, so what follows is not a race against the thread starting.
        for _ in 0..100 {
            if db.wanted() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(db.wanted(), "the write should be queued by now");

        // The batch boundary: the guard goes, and the scan stands aside rather than taking it again.
        // The scan then carries on, which means taking the connection again. `wanted()` goes false
        // the moment the writer is handed the lock, before it has used it, so the flag is read under
        // the lock rather than straight after `stand_off`.
        drop(held);
        stand_off(&db);
        let carried_on = db.lock();

        assert!(
            got_in.load(Ordering::Relaxed),
            "the waiting write has to have been through before the scan carries on"
        );
        drop(carried_on);
        writing.join().expect("the writer finished");
    }
}
