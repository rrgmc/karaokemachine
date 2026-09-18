//! One long piece of work, and how far it has got.
//!
//! There are four of these — searching, downloading originals, measuring, and fetching a bank — and
//! all of them take long enough that a page with no sign of movement reads as hung rather than as
//! working.
//!
//! **The shape is `km-package-builder`'s `scan::Progress` rather than the machine's `Fetching`.**
//! Both were candidates. `Fetching` is an enum of states, which is right for one download with one
//! bar; three of these four are multi-item with a phase name and a counter, which is what atomics
//! plus a snapshot already model — and that side also has the htmx polling fragment to copy.
//!
//! **One running job per section**, which is the same rule `Downloader::start` keeps: a second is
//! not a feature anybody asked for, and queueing is the part that would need thinking about.
//!
//! The counters are atomics because a tick can arrive from any `rayon` worker — see
//! `km_wallpaper_pack::progress`, whose `Sink` this is on the other end of.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// The phases a job reports, as catalog keys.
///
/// # Why a key rather than a word
///
/// **A phase is set where no request is in reach.** A job runs detached from the browser that asked
/// for it, so the locale is unknown when a phase is written down and known only when a page is
/// drawn. `km_locale`'s `t` filter takes any `&str`, so `_job.html` writes `{{ it.phase|t }}` and
/// the key becomes a word at render time, in whatever language the poll asking for it speaks.
///
/// **Four of the six come from another crate.** `km-wallpaper-pack`'s `progress::Phase` is an enum
/// of its own whose `as_str` is that program's English for a command line. [`phase_key`] maps it at
/// this program's boundary, which keeps a catalog out of a crate that serves no pages.
pub mod phase {
    /// Started, and not yet doing the thing it started for.
    pub const STARTING: &str = "phase-starting";
    /// Asking the providers, one search term at a time.
    pub const SEARCHING: &str = "phase-searching";
    /// Pulling originals, or a bank, onto this computer.
    pub const DOWNLOADING: &str = "phase-downloading";
    /// Decoding and measuring what was downloaded.
    pub const MEASURING: &str = "phase-measuring";
    /// Cropping, encoding and zipping a pack.
    pub const BUILDING: &str = "phase-building";
    /// Handing a finished file to the machine.
    pub const SENDING: &str = "phase-sending";

    /// Every one, for the test that checks the catalog has them all.
    ///
    /// Written out rather than derived: it is the one place a new phase has to be remembered, and
    /// forgetting it leaves a page drawing `⟦phase-whatever⟧`.
    pub const ALL: &[&str] = &[
        STARTING,
        SEARCHING,
        DOWNLOADING,
        MEASURING,
        BUILDING,
        SENDING,
    ];
}

/// One of `km-wallpaper-pack`'s phases, as a key this program's catalog answers.
///
/// A `match` rather than a string built from the enum's own name: the two vocabularies may differ,
/// and a phase renamed there is then a compile error here rather than a bracketed key on a page.
#[must_use]
pub fn phase_key(phase: km_wallpaper_pack::progress::Phase) -> &'static str {
    use km_wallpaper_pack::progress::Phase;
    match phase {
        Phase::Searching => phase::SEARCHING,
        Phase::Downloading => phase::DOWNLOADING,
        Phase::Measuring => phase::MEASURING,
        Phase::Building => phase::BUILDING,
    }
}

/// A piece of work in progress.
#[derive(Debug)]
pub struct Job {
    /// What it is doing, as a key from [`phase`].
    phase: Mutex<String>,
    /// Units finished.
    done: AtomicU64,
    /// Units in total, or `0` while that is not known.
    total: AtomicU64,
    /// Set when somebody asked it to stop.
    cancel: AtomicBool,
    /// Whether stopping is a thing this job can do at all.
    ///
    /// **A search and a download check a flag between items; a single send has nowhere to check
    /// one.** An upload to the machine is one `reqwest` call with no loop of ours inside it, so a
    /// Stop button on it would be a control that never does anything — and a control that never does
    /// anything teaches somebody to disbelieve the ones that do. The page draws the button only when
    /// this is set, and [`Job::ask_to_stop`] refuses to set `cancel` when it is not, so the two
    /// cannot come apart.
    stoppable: bool,
    /// Set when it has stopped, however it stopped.
    finished: AtomicBool,
    /// Why it stopped, when it stopped badly.
    error: Mutex<Option<String>>,
    /// What it produced, when it produced something.
    outcome: Mutex<Option<String>>,
    /// When it started, for an elapsed time.
    started: Instant,
}

impl Job {
    /// A job that has just begun, and that can be asked to stop.
    pub fn new(phase: impl Into<String>) -> Arc<Self> {
        Self::begin(phase, true)
    }

    /// The same, for work that has nowhere to notice the asking.
    ///
    /// See [`Job::stoppable`]'s field documentation: sending one file is one call, not a loop.
    pub fn unstoppable(phase: impl Into<String>) -> Arc<Self> {
        Self::begin(phase, false)
    }

    fn begin(phase: impl Into<String>, stoppable: bool) -> Arc<Self> {
        Arc::new(Self {
            phase: Mutex::new(phase.into()),
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            stoppable,
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
            outcome: Mutex::new(None),
            started: Instant::now(),
        })
    }

    /// Records how far along it is.
    pub fn progress(&self, phase: &str, done: u64, total: u64) {
        {
            let mut slot = self
                .phase
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *slot != phase {
                phase.clone_into(&mut slot);
            }
        }
        self.done.store(done, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }

    /// Asks it to stop at the next place it looks.
    ///
    /// A no-op on a job that cannot stop, so a typed-in URL to `/{section}/stop` cannot leave the
    /// page saying "stopping" about work that is going to run to the end regardless.
    pub fn ask_to_stop(&self) {
        if self.stoppable {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Whether it has been asked to stop.
    pub fn stopping(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Marks it done, with what it produced.
    pub fn done_with(&self, outcome: impl Into<String>) {
        *self
            .outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(outcome.into());
        self.finished.store(true, Ordering::Relaxed);
    }

    /// Marks it failed, with why — already worded for a person.
    pub fn failed_with(&self, why: impl Into<String>) {
        *self
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(why.into());
        self.finished.store(true, Ordering::Relaxed);
    }

    /// A consistent-enough picture for one render.
    ///
    /// **Not a lock over the whole job**, deliberately: the fields are read one at a time and a
    /// counter may move between two of them. What that can produce is a bar one tick behind its
    /// label, which nobody can see; what a global lock would produce is a `rayon` worker waiting on
    /// a browser poll.
    pub fn view(&self) -> View {
        let done = self.done.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        View {
            phase: self
                .phase
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            done,
            total,
            percent: percent(done, total),
            running: !self.finished.load(Ordering::Relaxed),
            stopping: self.stopping(),
            stoppable: self.stoppable,
            error: self
                .error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            outcome: self
                .outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            elapsed_secs: self.started.elapsed().as_secs(),
        }
    }
}

/// A job as one render of the page sees it.
#[derive(Debug, Clone)]
pub struct View {
    /// What it is doing.
    pub phase: String,
    /// Units finished.
    pub done: u64,
    /// Units in total, `0` when unknown.
    pub total: u64,
    /// How far along, 0 to 100. Meaningless while `total` is 0 — see [`View::indeterminate`].
    pub percent: u64,
    /// Whether it is still going.
    pub running: bool,
    /// Whether it has been asked to stop.
    pub stopping: bool,
    /// Whether asking it to stop would do anything, and so whether to draw the button.
    pub stoppable: bool,
    /// Why it stopped badly.
    pub error: Option<String>,
    /// What it produced.
    pub outcome: Option<String>,
    /// How long it has been going.
    pub elapsed_secs: u64,
}

impl View {
    /// Whether the bar should be drawn as "working" rather than as a proportion.
    ///
    /// **A total of zero means *not known yet*, never *nothing to do*.** `fetch` cannot know how many
    /// originals it will pull until every search has answered, and a bar that guessed would jump
    /// backwards when the real number arrived.
    pub fn indeterminate(&self) -> bool {
        self.total == 0
    }
}

/// How far along, as a percentage, with no division by zero.
fn percent(done: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    (done.min(total) * 100) / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_that_has_not_started_is_running_and_indeterminate() {
        let job = Job::new("waiting");
        let view = job.view();
        assert!(view.running);
        assert!(view.indeterminate(), "nothing is known about the size yet");
        assert_eq!(view.percent, 0);
        assert!(view.error.is_none() && view.outcome.is_none());
    }

    #[test]
    fn progress_is_reported_as_a_proportion_once_the_total_is_known() {
        let job = Job::new("downloading");
        job.progress("downloading", 1, 4);
        let view = job.view();
        assert!(!view.indeterminate());
        assert_eq!(view.percent, 25);
        assert_eq!(view.phase, "downloading");

        job.progress("downloading", 4, 4);
        assert_eq!(job.view().percent, 100);
    }

    #[test]
    fn a_count_past_the_total_does_not_go_past_a_hundred() {
        // A download that reports more bytes than the table said is a real case — the size check
        // refuses the wild ones and tolerates the near ones — and a bar at 140% is nonsense.
        assert_eq!(percent(140, 100), 100);
        assert_eq!(percent(1, 0), 0, "no division by zero");
    }

    #[test]
    fn finishing_stops_it_running_and_says_which_way_it_went() {
        let good = Job::new("x");
        good.done_with("GeneralUser-GS.sf2 is on the machine");
        let view = good.view();
        assert!(!view.running);
        assert_eq!(
            view.outcome.as_deref(),
            Some("GeneralUser-GS.sf2 is on the machine")
        );
        assert!(view.error.is_none());

        let bad = Job::new("x");
        bad.failed_with("the download does not match its digest");
        let view = bad.view();
        assert!(!view.running);
        assert!(view.outcome.is_none());
        assert!(view.error.is_some());
    }

    #[test]
    fn a_stop_is_visible_to_the_work_and_to_the_page() {
        let job = Job::new("downloading");
        assert!(!job.stopping());
        job.ask_to_stop();
        assert!(job.stopping(), "the loop checks this");
        assert!(job.view().stopping, "and the page says so");
        // Asking to stop is not the same as having stopped: the loop is still between two chunks.
        assert!(job.view().running);
    }

    #[test]
    fn work_with_nowhere_to_stop_is_never_offered_a_stop_button() {
        // Sending one file to the machine is a single call with no loop of ours inside it. A button
        // that set a flag nothing reads would be worse than no button: it would look as though the
        // upload had been called off.
        let job = Job::unstoppable("sending");
        assert!(!job.view().stoppable, "the page draws no button");
        job.ask_to_stop();
        assert!(!job.stopping(), "and asking anyway changes nothing");
        assert!(!job.view().stopping);
        assert!(job.view().running);

        assert!(Job::new("searching").view().stoppable, "a search still can");
    }
}
