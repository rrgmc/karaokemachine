//! Where a long run has got to, said at a steady interval.
//!
//! **A job that reads a corpus is minutes to hours, and the only thing worse than the wait is a wait
//! that says nothing.** A run with no output cannot be told apart from a run that has hung, and the
//! first thing anybody does about that is kill it — which on a pass over hundreds of thousands of
//! files throws away real work.
//!
//! What a person needs in order to decide whether to wait is four things, and a bare count is only
//! the first: how far in, how far there is to go, how fast it is going, and therefore when it ends.
//! [`Meter`] says all four in one line.
//!
//! **Here rather than in each command** because every one of them grew its own: `km-pack` has a
//! carriage-return line for its walk and another for its packaging, and the curation tool's
//! `--reanalyze` had a third. They disagreed about the interval, none of them said a rate, and only
//! one of them could be read at all when the output was a pipe.
//!
//! # Two sinks, and the line is shaped for whichever it has
//!
//! A terminal gets one line that rewrites itself, which is what makes a hundred updates readable.
//! **Anything else gets discrete lines**, because a carriage return in a log file is noise wrapped
//! around a thousand copies of the same sentence — and because [`crate::say`]'s other sink is
//! `tracing`, which has no cursor to move. The choice is made once, from
//! [`std::io::IsTerminal`] and [`crate::nowhere_to_talk`], and never revisited: a run whose output is
//! redirected halfway through is not a thing that happens.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

/// How often a meter speaks, unless it is told otherwise.
///
/// Ten seconds is the interval a four-hour job wants: often enough that the run is visibly alive,
/// rare enough that the whole of it is a hundred and fifty lines rather than a scroll nobody reads.
pub const EVERY: Duration = Duration::from_secs(10);

/// How long a run must have gone on before its speed is worth stating.
const RATE_AFTER: f64 = 2.0;

/// ...and before the speed is worth extrapolating from. Longer, because an estimate is the number
/// somebody acts on and a wrong one costs them the decision.
const ESTIMATE_AFTER: f64 = 5.0;

/// A count, a rate and a finishing time, said at a steady interval.
///
/// Driven by whoever owns the work — `at` is called as often as is convenient and speaks only when
/// the interval has passed, so a caller in a tight loop costs one clock read per item and a caller
/// polling a background thread costs nothing at all.
///
/// **Not thread-safe, and deliberately not.** The count it reports comes from somewhere that already
/// has its own answer — a loop counter, or a set of atomics a worker crew is updating — and a meter
/// that took a lock to print would be a second source of truth about the same number. One thread
/// reads that answer and says it.
pub struct Meter {
    label: String,
    total: u64,
    started: Instant,
    spoke_at: Option<Instant>,
    every: Duration,
    /// Whether the sink has a cursor to move back to the start of the line.
    rewrites: bool,
    /// Whether anything has been drawn that a final line would have to land after.
    drawn: bool,
}

impl Meter {
    /// A meter over a known number of things, labelled with what it is doing to them.
    ///
    /// The label is a verb in the continuous, lower case, because it is read as the middle of a
    /// sentence the count finishes: `reading 42,300 of 150,000`.
    pub fn new(label: impl Into<String>, total: u64) -> Self {
        Self {
            label: label.into(),
            total,
            started: Instant::now(),
            spoke_at: None,
            every: EVERY,
            // A pipe and a log both get whole lines. `nowhere_to_talk` is the stronger of the two
            // tests and is checked first: with no handle at all there is no cursor either, and
            // `IsTerminal` on a null handle is a question with no useful answer.
            rewrites: !crate::nowhere_to_talk() && std::io::stdout().is_terminal(),
            drawn: false,
        }
    }

    /// A different interval, for a job whose whole length is shorter than [`EVERY`].
    #[must_use]
    pub fn every(mut self, interval: Duration) -> Self {
        self.every = interval;
        self
    }

    /// Says where the run has got to, if the interval has passed since it last did.
    pub fn at(&mut self, done: u64) {
        let now = Instant::now();
        if let Some(spoke) = self.spoke_at
            && now.duration_since(spoke) < self.every
        {
            return;
        }
        self.spoke_at = Some(now);
        self.draw(&self.line(done, now));
    }

    /// Takes the line back, so that whatever is said next has the cursor to itself.
    ///
    /// **Call this before printing anything else while a meter is running**, and at the end. A
    /// rewriting line does not end in a newline — that is what lets the next draw overwrite it — so
    /// a line printed on top of one lands in the middle of it.
    ///
    /// **It says nothing of its own**, which is the whole of what was wrong with ending on a summary
    /// line: the caller always has something truer to say at that point, and a meter that insisted on
    /// a last word either repeated the final count or printed a rate for a run that had not started.
    /// Where there is no cursor there is nothing to take back and this does nothing at all.
    pub fn clear(&mut self) {
        if self.rewrites && self.drawn {
            // Blanked rather than newlined: every later line is shorter than the longest one drawn —
            // the rate falls, the estimate loses a digit — so the tail of the old one would stay on
            // screen beside the new one.
            print!("\r{:width$}\r", "", width = self.width());
            let _ = std::io::stdout().flush();
        }
        self.drawn = false;
    }

    /// The one line, for whichever sink.
    fn line(&self, done: u64, now: Instant) -> String {
        let elapsed = now.duration_since(self.started).as_secs_f64();
        let rate = if elapsed > 0.0 {
            done as f64 / elapsed
        } else {
            0.0
        };
        let mut line = format!(
            "  {} {} of {}",
            self.label,
            thousands(done),
            thousands(self.total)
        );
        // A total nobody knows yet is a count on its own rather than a division by zero.
        if let Some(percent) = (done.min(self.total) * 100).checked_div(self.total) {
            line.push_str(&format!("  {percent}%"));
        }
        // **Nothing is said about speed until there has been some.** A count divided by the first
        // fraction of a second is a number in the tens of thousands that has measured nothing, and
        // the first thing a meter draws is exactly when that happens — the draw is immediate so the
        // run is visibly alive, and the rate at that moment is noise.
        if elapsed >= RATE_AFTER && rate >= 0.05 {
            line.push_str(&format!("  {}/s", thousands(rate.round() as u64)));
            // No estimate once it is over, and none before there is enough to estimate from: a
            // remaining time worked out from the first second of a four-hour job is a number that
            // will be wrong by hours and read as though it were not.
            if done < self.total && elapsed >= ESTIMATE_AFTER {
                let left = (self.total - done) as f64 / rate;
                line.push_str(&format!("  {} left", roughly(left)));
            }
        }
        line
    }

    /// Puts the line where it goes, rewriting in place where there is a cursor to do it with.
    fn draw(&mut self, line: &str) {
        if !self.rewrites {
            crate::say(line);
            return;
        }
        // `print!` rather than `say`: this branch has already established that there is a terminal
        // to write to, and `say`'s other sink is a log with no cursor. The flush is not optional —
        // stdout is line-buffered and this line has no newline in it to trigger one.
        print!("\r{line:width$}", width = self.width());
        let _ = std::io::stdout().flush();
        self.drawn = true;
    }

    /// Wide enough that a shorter line covers whatever the last one left behind.
    fn width(&self) -> usize {
        // The longest shape this prints, with room for a corpus of ten million: label, two counts, a
        // percentage, a rate and an estimate.
        self.label.len() + 56
    }
}

/// A count with the separators a person reads it by.
///
/// Grouped in threes with a comma, which is what every other number this program prints to a console
/// does. Pages have a catalog and a locale; a console line has neither.
fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A duration at the precision somebody deciding whether to wait can act on.
///
/// **Rounded hard on purpose.** An estimate is a guess from an average rate, and printing it to the
/// second claims an accuracy it does not have — a corpus read gets faster and slower as it crosses
/// folders, and the number it ends on is not the number it started with.
fn roughly(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    if seconds <= 89 {
        return format!("{seconds}s");
    }
    // Rounded to minutes first, and the hour band chosen from *that* rather than from the seconds:
    // picking it from the seconds prints an hour as `60m`, which is the right number in the wrong
    // unit and reads as a mistake.
    let minutes = (seconds + 30) / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    match minutes % 60 {
        0 => format!("{hours}h"),
        rest => format!("{hours}h {rest}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_grouped_the_way_it_is_read() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(150_000), "150,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    /// Seconds while the end is in sight, minutes through the middle of a job, hours at the start of
    /// a long one — each band as precise as a guess from an average rate can support.
    #[test]
    fn an_estimate_is_rounded_to_what_it_can_support() {
        assert_eq!(roughly(0.0), "0s");
        assert_eq!(roughly(45.0), "45s");
        assert_eq!(roughly(89.0), "89s");
        assert_eq!(roughly(90.0), "2m");
        assert_eq!(roughly(600.0), "10m");
        // An hour is an hour in both directions across the boundary, and never `60m`.
        assert_eq!(roughly(3_570.0), "1h");
        assert_eq!(roughly(3_600.0), "1h");
        assert_eq!(roughly(5_400.0), "1h 30m");
        assert_eq!(roughly(16_240.0), "4h 31m");
    }

    /// The whole line, because its parts are assembled conditionally and the conditions are the
    /// point: no percentage without a total, no rate before there is one, no estimate at the end.
    #[test]
    fn the_line_says_what_it_can_and_no_more() {
        let mut meter = Meter::new("reading", 150_000);
        meter.rewrites = false;
        meter.started = Instant::now() - Duration::from_secs(1_000);
        let now = Instant::now();

        let line = meter.line(20_000, now);
        assert!(line.contains("reading 20,000 of 150,000"), "{line}");
        assert!(line.contains("13%"), "{line}");
        assert!(line.contains("20/s"), "{line}");
        assert!(line.contains("left"), "{line}");

        // Finished: a count equal to the total has nothing left to wait for.
        let done = meter.line(150_000, now);
        assert!(done.contains("100%"), "{done}");
        assert!(!done.contains("left"), "{done}");
    }

    /// A job that has only just started knows neither its speed nor its end, and must not invent
    /// either. The first draw happens immediately so that the run is visibly alive, and that draw is
    /// precisely the one where a count over a fraction of a second reads as tens of thousands a
    /// second.
    #[test]
    fn nothing_is_measured_from_the_first_moment() {
        let mut meter = Meter::new("reading", 1_000_000);
        meter.rewrites = false;

        meter.started = Instant::now();
        let opening = meter.line(2, Instant::now());
        assert!(opening.contains("reading 2 of 1,000,000"), "{opening}");
        assert!(
            !opening.contains("/s"),
            "a rate from no time at all: {opening}"
        );

        // Long enough to know the speed, not long enough to extrapolate it.
        meter.started = Instant::now() - Duration::from_secs(3);
        let early = meter.line(30, Instant::now());
        assert!(early.contains("10/s"), "{early}");
        assert!(!early.contains("left"), "{early}");
    }

    /// A total nobody knows yet is a count on its own rather than a division by zero.
    #[test]
    fn a_meter_with_no_total_still_counts() {
        let mut meter = Meter::new("reading", 0);
        meter.rewrites = false;
        meter.started = Instant::now() - Duration::from_secs(10);
        let line = meter.line(40, Instant::now());
        assert!(line.contains("reading 40 of 0"), "{line}");
        assert!(!line.contains('%'), "{line}");
    }
}
