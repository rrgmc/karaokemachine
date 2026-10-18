//! Measuring how loud each MP3+G song is on other threads, ahead of the build that adds it.
//!
//! **The measurement is nearly the whole cost of a build of MP3+G pairs.** It decodes the whole
//! MP3, about half a second a song, where probing and hashing the pair take about a hundredth. The
//! songs are added in order on one thread, but each measurement depends on its own file alone. So
//! workers measure the songs ahead of the add, and the add takes the answer.
//!
//! **The answer is the one the add would have measured**: the same function on the same file. A
//! package built this way is the package built one song at a time.
//!
//! **Stopping is dropping.** A build that ends early drops its [`MeasureAhead`], which tells the
//! workers to take no further song. A worker part-way through a song finishes it and then exits,
//! and nothing waits for it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// One song's measurement, as [`km_cdg::measure_loudness`] answers, with the error as text.
pub type Measurement = Result<Option<km_loudness::Loudness>, String>;

/// Workers measuring a list of songs, and the answers that have arrived.
pub struct MeasureAhead {
    answers: Receiver<(usize, Measurement)>,
    arrived: HashMap<usize, Measurement>,
    stop: Arc<AtomicBool>,
}

impl MeasureAhead {
    /// Starts measuring `jobs`, each a song's position in the build and the audio file to measure.
    ///
    /// The workers take the songs in the order given, so the song the build needs next is the one
    /// most likely to be ready.
    pub fn start(jobs: Vec<(usize, PathBuf)>) -> Self {
        let (send, answers) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let jobs = Arc::new(jobs);
        let next = Arc::new(AtomicUsize::new(0));
        let workers = workers_for(std::thread::available_parallelism().map_or(1, usize::from))
            .min(jobs.len());
        // Each worker owns a sender, and this function drops the first one on return. So the
        // receiver ends once every worker has, and a worker that panics cannot leave the build
        // waiting for an answer that never comes.
        for _ in 0..workers {
            let (jobs, next, stop) = (Arc::clone(&jobs), Arc::clone(&next), Arc::clone(&stop));
            let send = send.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Some((index, path)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    let measured =
                        km_cdg::measure_loudness(path).map_err(|error| error.to_string());
                    if send.send((*index, measured)).is_err() {
                        break;
                    }
                }
            });
        }
        Self {
            answers,
            arrived: HashMap::new(),
            stop,
        }
    }

    /// The measurement of the song at `index`, waiting for it if a worker has it in hand.
    ///
    /// `None` when no worker will answer for it: it was not in the list, or every worker has
    /// ended. The add then measures the song itself.
    pub fn take(&mut self, index: usize) -> Option<Measurement> {
        loop {
            if let Some(measured) = self.arrived.remove(&index) {
                return Some(measured);
            }
            let (arrived, measured) = self.answers.recv().ok()?;
            if arrived == index {
                return Some(measured);
            }
            self.arrived.insert(arrived, measured);
        }
    }
}

/// The most workers a build measures with, whatever the processor: what one spinning disk feeds.
///
/// A worker reads a pair of about 7 MB in about half a second, so about 14 MB/s. Eight of them read
/// about 110 MB/s, near what a spinning disk delivers, and a ninth would mostly wait for it.
const MAX_WORKERS: usize = 8;

/// How many workers measure at once on a processor with `logical` cores.
///
/// **Half the logical cores**, which is about one a physical core where each core runs two
/// threads. The other half stays free, because a build of a whole collection runs for hours on a
/// machine somebody is using. At least one, and at most [`MAX_WORKERS`].
fn workers_for(logical: usize) -> usize {
    (logical / 2).clamp(1, MAX_WORKERS)
}

impl Drop for MeasureAhead {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_the_cores_measure_and_never_none_or_more_than_the_disk_feeds() {
        let workers: Vec<usize> = [1, 2, 4, 8, 12, 16, 24, 64]
            .into_iter()
            .map(workers_for)
            .collect();
        assert_eq!(workers, [1, 1, 2, 4, 6, 8, 8, 8]);
    }

    #[test]
    fn an_answer_is_the_one_the_add_would_measure_and_arrives_in_any_order() {
        let scratch = km_testkit::Scratch::new("measure-ahead");
        let tone = scratch.join("tone.mp3");
        std::fs::write(&tone, crate::zipped_tests::TONE).expect("write");
        let broken = scratch.join("broken.mp3");
        std::fs::write(&broken, b"not audio").expect("write");
        let expected = km_cdg::measure_loudness(&tone).map_err(|error| error.to_string());

        let jobs = (0..8)
            .map(|index| {
                let path = if index == 3 { &broken } else { &tone };
                (index * 2, path.clone())
            })
            .collect();
        let mut ahead = MeasureAhead::start(jobs);
        // The last first, so every earlier answer has to be kept until it is asked for.
        assert_eq!(ahead.take(14), Some(expected.clone()));
        for index in [0, 2, 4, 8, 10, 12] {
            assert_eq!(ahead.take(index), Some(expected.clone()), "song {index}");
        }
        assert!(
            matches!(ahead.take(6), Some(Err(_))),
            "a decode failure is an answer"
        );
        assert_eq!(
            ahead.take(1),
            None,
            "nobody measures a song not in the list"
        );
    }
}
