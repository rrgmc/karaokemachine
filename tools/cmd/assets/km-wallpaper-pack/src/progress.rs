//! How far a long phase has got, for a caller that draws rather than logs.
//!
//! **The command line does not need this and does not use it.** `fetch` and `analyze` already say
//! how they are going, through `tracing` every two hundred images — which is the right answer for a
//! terminal, survives being piped to a file, and is deliberately not a progress-bar dependency. That
//! reporting is untouched; a sink is *additional*.
//!
//! What needs more is a program with a page. `km-admin` runs these same phases behind a browser,
//! where "measuring" scrolling past in a log is not available and a phase that takes minutes with no
//! visible movement reads as a hung page rather than as work. It polls a bar, and a bar needs a
//! number.
//!
//! **A `Sink` is `Option<&dyn Fn>` rather than a trait on the caller**, because there is exactly one
//! thing to say — *this many of that many* — and a trait would be a vocabulary to learn for one
//! method. `None` is what every command-line call passes, and it costs a null check per tick.
//!
//! It must be `Sync`: the measuring loop is `rayon`'s `par_iter`, so ticks arrive from every worker
//! thread at once. A sink that wants to count is holding an atomic anyway.

/// How far a phase has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Which phase, in the words a person would be shown.
    pub phase: Phase,
    /// How many units are finished.
    pub done: usize,
    /// How many there are in total, or `0` when that is not yet known.
    ///
    /// **Zero means unknown rather than nothing to do**, which a bar should draw as indeterminate.
    /// `fetch` genuinely does not know how many originals it will download until every search has
    /// answered, and claiming a total it has not established would make the bar jump backwards.
    pub total: usize,
}

/// The long phases, named.
///
/// An enum rather than a `&'static str` so that a caller matching on one cannot mistype a phase and
/// silently draw nothing: these names reach a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Asking the providers, one search term at a time.
    Searching,
    /// Pulling originals into the cache.
    Downloading,
    /// Decoding and measuring what is in the cache.
    Measuring,
    /// Cropping, blurring, encoding and zipping the pack.
    Building,
}

impl Phase {
    /// What to call it on a screen.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Searching => "searching",
            Self::Downloading => "downloading",
            Self::Measuring => "measuring",
            Self::Building => "building",
        }
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Somewhere to send progress, or nothing.
///
/// `None` at a command line, where `tracing` is already the answer.
pub type Sink<'a> = Option<&'a (dyn Fn(Tick) + Sync)>;

/// Sends one tick, if anybody is listening.
pub fn tick(sink: Sink<'_>, phase: Phase, done: usize, total: usize) {
    if let Some(sink) = sink {
        sink(Tick { phase, done, total });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn nothing_happens_without_a_sink() {
        // The command line's case, and the one that must cost nothing.
        tick(None, Phase::Measuring, 1, 10);
    }

    #[test]
    fn a_sink_hears_every_tick_including_from_several_threads() {
        let seen = Mutex::new(Vec::new());
        let sink = |t: Tick| seen.lock().expect("lock").push((t.phase, t.done, t.total));

        // The signature has to be `Sync`, because `analyze` ticks from inside `par_iter`. This is
        // what fails to compile if that is ever loosened.
        std::thread::scope(|scope| {
            for n in 1..=4 {
                scope.spawn(move || tick(Some(&sink), Phase::Measuring, n, 4));
            }
        });

        let mut got = seen.into_inner().expect("into inner");
        got.sort_unstable_by_key(|(_, done, _)| *done);
        assert_eq!(
            got,
            vec![
                (Phase::Measuring, 1, 4),
                (Phase::Measuring, 2, 4),
                (Phase::Measuring, 3, 4),
                (Phase::Measuring, 4, 4),
            ]
        );
    }

    #[test]
    fn every_phase_has_a_distinct_name() {
        let all = [
            Phase::Searching,
            Phase::Downloading,
            Phase::Measuring,
            Phase::Building,
        ];
        let mut names: Vec<&str> = all.iter().map(|phase| phase.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len(), "two phases share a name");
        assert_eq!(Phase::Measuring.to_string(), "measuring");
    }
}
