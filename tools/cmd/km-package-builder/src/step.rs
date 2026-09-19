//! A checklist of the steps a long job takes, with where each one stands and how long it took.
//!
//! **Two pages draw one of these.** The Scan page lists a run's steps, and the Open page lists the
//! rungs an open climbs. Both answer the same question — *which step, and what is left* — and a name
//! for the running step alone answers neither: a step that has shown the same sentence for five
//! minutes looks exactly like a step that has stopped.
//!
//! **A step is a catalog key, not a sentence.** Both jobs run on a worker thread where no request
//! and so no language is in reach, so the page writes the words from the key through `|t`.
//!
//! The two owners differ in one thing, which is how a step that does not run is marked. A scan knows
//! its skips at the gate that decides them and names them with [`Ladder::skip`]. An open's gates are
//! `if` statements inside `Db::prepare`, so reaching a later rung is the only thing that says an
//! earlier one did not run, and that is [`Ladder::advance_to`].

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Where one step of a job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    /// Not reached yet.
    Waiting,
    /// Going on now.
    Running,
    /// Over, and it went through.
    Done,
    /// Not run: this job did not need it, or ended before reaching it.
    Skipped,
    /// The step the job failed in.
    Failed,
}

impl StepState {
    /// The class the page draws the step's mark with.
    pub fn class(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

/// One step of a job, as the checklist draws it.
#[derive(Debug, Clone)]
pub struct Step {
    /// The phase key, which is also the step's name on the page.
    pub key: &'static str,
    pub state: StepState,
    /// When it started running.
    pub started: Option<Instant>,
    /// How long it ran, once it is over.
    pub took: Option<Duration>,
    /// Whether it runs only when the job changed something.
    pub if_changed: bool,
}

impl Step {
    /// A step nothing has reached.
    pub fn waiting(key: &'static str, if_changed: bool) -> Self {
        Self {
            key,
            state: StepState::Waiting,
            started: None,
            took: None,
            if_changed,
        }
    }
}

/// A step as the page draws it.
#[derive(Debug, Clone)]
pub struct StepView {
    /// The phase key, which the page words.
    pub key: String,
    /// The class for its mark: `waiting`, `running`, `done`, `skipped` or `failed`.
    pub state: &'static str,
    /// How long it took, once it is over; how long it has been running, while it runs.
    pub took: Option<String>,
    /// Whether it runs only when the job changed something.
    pub if_changed: bool,
}

/// The steps of one job, in the order it takes them.
///
/// **Listed before they run**, so a page can say what is still to come as well as what is over.
///
/// **Timed, because the first question about a slow job is *which step*.** A scan is up to seven
/// steps and an open eleven, several of them whole-corpus passes over a database of most of a
/// gigabyte, and a name alone does not say which of them the time went into.
#[derive(Debug, Default)]
pub struct Ladder {
    steps: Mutex<Vec<Step>>,
}

impl Ladder {
    /// A ladder whose rungs are these keys, all waiting, in this order.
    pub fn of(keys: &[&'static str]) -> Self {
        Self {
            steps: Mutex::new(keys.iter().map(|key| Step::waiting(key, false)).collect()),
        }
    }

    /// Replaces the list with the steps this job will take, all of them waiting.
    pub fn plan(&self, steps: Vec<Step>) {
        if let Ok(mut slot) = self.steps.lock() {
            *slot = steps;
        }
    }

    /// Opens `key`'s step, and closes the one before it.
    ///
    /// The two are one operation on purpose: a phase boundary is the only moment both are known,
    /// and a `say` that forgot to stop the clock would leave a timing that silently belonged to two
    /// steps at once. A step the plan did not list is added at the end, so it is still shown.
    pub fn say(&self, key: &'static str) {
        let now = Instant::now();
        if let Ok(mut steps) = self.steps.lock() {
            close_running(&mut steps, now, StepState::Done);
            match steps.iter_mut().find(|step| step.key == key) {
                Some(step) => {
                    step.state = StepState::Running;
                    step.started = Some(now);
                }
                None => steps.push(Step {
                    state: StepState::Running,
                    started: Some(now),
                    ..Step::waiting(key, false)
                }),
            }
        }
    }

    /// Opens `key`'s rung, and marks every rung still waiting above it as skipped.
    ///
    /// **The difference from [`Self::say`], and why both exist.** A scan names its own skips at the
    /// gate that decides them. An open's gates are `if` statements inside `Db::prepare` with no page
    /// in reach — missing indexes, a revision that moved, a database with no statistics — so
    /// reaching a later rung is the only thing that says an earlier one did not run.
    ///
    /// **A rung already running is left where it is.** The two counted rungs report on every chunk
    /// they write, and restarting the clock at each of them would draw a step that has run for
    /// minutes as one that has just begun.
    pub fn advance_to(&self, key: &'static str) {
        let now = Instant::now();
        if let Ok(mut steps) = self.steps.lock() {
            let Some(at) = steps.iter().position(|step| step.key == key) else {
                return;
            };
            if steps[at].state == StepState::Running {
                return;
            }
            close_running(&mut steps, now, StepState::Done);
            for step in steps[..at]
                .iter_mut()
                .filter(|step| step.state == StepState::Waiting)
            {
                step.state = StepState::Skipped;
            }
            steps[at].state = StepState::Running;
            steps[at].started = Some(now);
        }
    }

    /// Marks steps this job will not take, where they have not been reached.
    pub fn skip(&self, keys: &[&str]) {
        if let Ok(mut steps) = self.steps.lock() {
            for step in steps.iter_mut() {
                if step.state == StepState::Waiting && keys.contains(&step.key) {
                    step.state = StepState::Skipped;
                }
            }
        }
    }

    /// Closes the job: the step still running is over, and a step never reached is skipped.
    ///
    /// A job ends by naming a *state* rather than another step, so this is separate from
    /// [`Self::say`], which would time how long it took to notice the job was over.
    pub fn end(&self, failed: bool) {
        let now = Instant::now();
        if let Ok(mut steps) = self.steps.lock() {
            close_running(
                &mut steps,
                now,
                if failed {
                    StepState::Failed
                } else {
                    StepState::Done
                },
            );
            for step in steps.iter_mut() {
                if step.state == StepState::Waiting {
                    step.state = StepState::Skipped;
                }
            }
        }
    }

    /// Every step as the page draws it, with its duration already written out for reading.
    ///
    /// Formatted here rather than in the template because the template has no arithmetic and a
    /// `Duration` rendered by its `Debug` is `1.153412s`.
    pub fn views(&self, now: Instant) -> Vec<StepView> {
        self.with(|steps| {
            steps
                .iter()
                .map(|step| StepView {
                    key: step.key.to_owned(),
                    state: step.state.class(),
                    took: match step.state {
                        StepState::Running => step
                            .started
                            .map(|started| human_duration(now.saturating_duration_since(started))),
                        _ => step.took.map(human_duration),
                    },
                    if_changed: step.if_changed,
                })
                .collect()
        })
    }

    /// Reads the steps under the lock, or answers as if there were none.
    ///
    /// A poisoned lock is a page with no checklist on it rather than a page that will not render:
    /// what is behind it is a list drawn for reading, and nothing else reads it back.
    pub fn with<T: Default>(&self, read: impl FnOnce(&[Step]) -> T) -> T {
        self.steps
            .lock()
            .map(|steps| read(&steps))
            .unwrap_or_default()
    }
}

/// Ends the running step, if there is one, in `state`, and logs how long it took.
fn close_running(steps: &mut [Step], now: Instant, state: StepState) {
    for step in steps
        .iter_mut()
        .filter(|step| step.state == StepState::Running)
    {
        let took = step
            .started
            .map(|started| now.saturating_duration_since(started))
            .unwrap_or_default();
        tracing::info!(phase = step.key, seconds = took.as_secs_f32(), "phase");
        step.state = state;
        step.took = Some(took);
    }
}

/// A phase duration, at the precision somebody reading it can act on.
///
/// Four bands rather than one format, because the numbers this reports genuinely span them: a
/// gated phase is microseconds, `ANALYZE` is about a second, and a forced pass over a real corpus is
/// hours. Milliseconds on the last of those would be noise, and `0 s` on the first would hide the
/// very thing the timings are there to show.
pub fn human_duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    let whole = elapsed.as_secs();
    if seconds < 1.0 {
        format!("{} ms", elapsed.as_millis())
    } else if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else if whole < 3600 {
        format!("{} m {:02} s", whole / 60, whole % 60)
    } else {
        format!("{} h {:02} m", whole / 3600, (whole % 3600) / 60)
    }
}

/// A time still to come, at the precision an estimate has.
///
/// Coarser than [`human_duration`] on purpose: an estimate moves with every poll, and a seconds
/// figure that changes every second on a two-hour estimate claims a precision it does not have.
pub fn rough_duration(left: Duration) -> String {
    let whole = left.as_secs();
    if whole < 60 {
        format!("{whole} s")
    } else if whole < 3600 {
        format!("{} m", whole.div_ceil(60))
    } else {
        format!("{} h {:02} m", whole / 3600, (whole % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_is_shown_at_the_precision_it_has() {
        assert_eq!(human_duration(Duration::from_millis(250)), "250 ms");
        assert_eq!(human_duration(Duration::from_millis(1_500)), "1.5 s");
        assert_eq!(human_duration(Duration::from_secs(125)), "2 m 05 s");
        assert_eq!(human_duration(Duration::from_secs(7_500)), "2 h 05 m");
        assert_eq!(rough_duration(Duration::from_secs(42)), "42 s");
        assert_eq!(rough_duration(Duration::from_secs(61)), "2 m");
        assert_eq!(rough_duration(Duration::from_secs(7_500)), "2 h 05 m");
    }

    /// Reaching a rung is what says the rungs above it did not run.
    #[test]
    fn a_rung_that_was_passed_is_marked_skipped() {
        let ladder = Ladder::of(&["one", "two", "three", "four"]);
        ladder.advance_to("three");

        let states = ladder.with(|steps| {
            steps
                .iter()
                .map(|step| step.state.class())
                .collect::<Vec<_>>()
        });
        assert_eq!(states, vec!["skipped", "skipped", "running", "waiting"]);
    }

    /// **A rung that reports on every chunk keeps one clock.** Restarting it at each report would
    /// draw a step that has run for minutes as one that has just begun.
    #[test]
    fn a_rung_reached_twice_keeps_the_clock_it_started_with() {
        let ladder = Ladder::of(&["one", "two"]);
        ladder.advance_to("two");
        let started = ladder.with(|steps| steps[1].started);
        ladder.advance_to("two");

        assert_eq!(ladder.with(|steps| steps[1].started), started);
        assert_eq!(ladder.with(|steps| steps[1].state.class()), "running");
    }
}
