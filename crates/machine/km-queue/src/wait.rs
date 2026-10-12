//! How long a queued song waits before it starts.
//!
//! The wait has two parts. One is the time left in the playing song, which changes every second and
//! which only an observer of the playback holds. The other is the sum of the lengths of the entries
//! ahead, which changes only when the queue does. [`ahead_ms`] gives the second part, and
//! [`Wait::of`] turns the two together into what an entry shows.
//!
//! The figure is rough on purpose. It tells a singer whether to stay near the microphone, so it
//! ignores the time between two songs, a pause and a changed tempo. Each is small beside a figure
//! rounded to the minute.

use crate::queue::QueueEntry;

/// One minute, in milliseconds.
const MINUTE_MS: u64 = 60_000;

/// What an entry shows for its wait.
///
/// Two forms and no third, so the screen and every remote state one wait the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// Less than a minute.
    Soon,
    /// A minute or more, rounded to the minute.
    Minutes(u32),
}

impl Wait {
    /// The wait for an entry, from the time left in the playing song and the lengths ahead of it.
    pub fn of(remaining_ms: u32, ahead_ms: u64) -> Self {
        let wait_ms = u64::from(remaining_ms) + ahead_ms;
        if wait_ms < MINUTE_MS {
            Self::Soon
        } else {
            let minutes = (wait_ms + MINUTE_MS / 2) / MINUTE_MS;
            Self::Minutes(u32::try_from(minutes).unwrap_or(u32::MAX))
        }
    }
}

/// For each entry in play order, the sum of the lengths of the entries before it.
///
/// The first entry's figure is zero, because nothing in the queue is ahead of it.
pub fn ahead_ms<'a>(
    entries: impl IntoIterator<Item = &'a QueueEntry>,
) -> impl Iterator<Item = u64> {
    entries.into_iter().scan(0_u64, |total, entry| {
        let ahead = *total;
        *total += u64::from(entry.duration_ms);
        Some(ahead)
    })
}

/// The time left in a song, from its position and its length.
///
/// Zero where no song plays, and zero where the position has passed the reported length.
pub fn remaining_ms(position_ms: u32, duration_ms: u32) -> u32 {
    duration_ms.saturating_sub(position_ms)
}

#[cfg(test)]
mod tests {
    use km_songcode::SongCode;

    use super::*;

    fn entry(duration_ms: u32) -> QueueEntry {
        QueueEntry {
            id: 0,
            number: SongCode::new(1),
            title: "Song".to_owned(),
            artist: None,
            singer: None,
            duration_ms,
            seat: None,
        }
    }

    #[test]
    fn a_wait_under_a_minute_has_no_figure() {
        assert_eq!(Wait::of(0, 0), Wait::Soon);
        assert_eq!(Wait::of(59_999, 0), Wait::Soon);
        assert_eq!(Wait::of(30_000, 29_999), Wait::Soon);
    }

    #[test]
    fn a_wait_of_a_minute_or_more_rounds_to_the_minute() {
        assert_eq!(Wait::of(60_000, 0), Wait::Minutes(1));
        assert_eq!(Wait::of(89_999, 0), Wait::Minutes(1));
        assert_eq!(Wait::of(90_000, 0), Wait::Minutes(2));
        assert_eq!(Wait::of(45_000, 180_000), Wait::Minutes(4));
    }

    #[test]
    fn the_two_parts_add_without_overflow() {
        assert_eq!(
            Wait::of(u32::MAX, u64::from(u32::MAX) * 200),
            Wait::Minutes(14_388_140)
        );
    }

    #[test]
    fn each_entry_waits_for_the_lengths_before_it() {
        let entries = [entry(180_000), entry(0), entry(240_000), entry(200_000)];
        let ahead: Vec<u64> = ahead_ms(&entries).collect();
        assert_eq!(ahead, vec![0, 180_000, 180_000, 420_000]);
    }

    #[test]
    fn the_time_left_never_goes_below_zero() {
        assert_eq!(remaining_ms(67_000, 222_000), 155_000);
        assert_eq!(remaining_ms(222_500, 222_000), 0);
        assert_eq!(remaining_ms(0, 0), 0);
    }
}
