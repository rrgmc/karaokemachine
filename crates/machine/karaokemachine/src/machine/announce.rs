//! When an announced song starts playing, and when its card leaves.
//!
//! **Arithmetic only.** The machine owns the clock and the transport, and the display draws the
//! card. What is here decides one thing: how much of the card's time the song spends in silence.
//!
//! A card that always held the song would add its whole length to every song. Most songs open with
//! an introduction nobody sings over, so the card spends that time and the room hears no gap. The
//! decision is `The machine announces a song before it starts` in `docs/decisions/interface.md`.

use std::time::{Duration, Instant};

/// How one song's announcement is timed, from the moment the song is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Plan {
    /// How long the card stays on the screen.
    pub(super) card: Duration,
    /// How long the song waits in silence before it plays. Zero plays it at once, under the card.
    pub(super) play_after: Duration,
}

/// Times an announcement of `card` for a song whose first line appears `reveal` into it.
///
/// **The card ends when the first line appears, or later.** A song with a longer introduction
/// plays at once and the card leaves part way through it. A shorter one waits for the difference,
/// so its first line arrives as the card goes.
///
/// `None` is a song with no word times: a video, an MP3+G song, or a MIDI file with no lyrics.
/// Nothing says when its singer starts, so it waits for the whole card.
pub(super) fn plan(card: Duration, reveal: Option<Duration>) -> Plan {
    Plan {
        card,
        play_after: card.saturating_sub(reveal.unwrap_or(Duration::ZERO)),
    }
}

/// An announcement in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Announce {
    /// When the card went up.
    pub(super) shown_at: Instant,
    /// When the card comes down.
    pub(super) card_until: Instant,
    /// When the held song plays, while it has not. `None` once it is playing.
    pub(super) play_at: Option<Instant>,
}

impl Announce {
    /// Starts an announcement at `now`. The second value is whether the song plays at once.
    pub(super) fn begin(plan: Plan, now: Instant) -> (Self, bool) {
        let plays_now = plan.play_after.is_zero();
        (
            Self {
                shown_at: now,
                card_until: now + plan.card,
                play_at: (!plays_now).then(|| now + plan.play_after),
            },
            plays_now,
        )
    }

    /// Whether the held song is due to play at `now`.
    pub(super) fn play_is_due(&self, now: Instant) -> bool {
        self.play_at.is_some_and(|at| at <= now)
    }

    /// Whether nothing is left of this announcement at `now`: the card is down and the song plays.
    pub(super) fn is_over(&self, now: Instant) -> bool {
        self.play_at.is_none() && now >= self.card_until
    }

    /// How long the card has been up at `now` and how long it stays, while it is up.
    pub(super) fn card(&self, now: Instant) -> Option<(Duration, Duration)> {
        (now < self.card_until).then(|| {
            (
                now.saturating_duration_since(self.shown_at),
                self.card_until.saturating_duration_since(self.shown_at),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: Duration = Duration::from_secs(5);

    /// An introduction as long as the card, or longer, costs the room no silence at all.
    #[test]
    fn a_long_introduction_plays_under_the_card() {
        for reveal in [5_000, 5_001, 20_000, 240_000] {
            let plan = plan(CARD, Some(Duration::from_millis(reveal)));
            assert_eq!(plan.play_after, Duration::ZERO, "{reveal} ms");
            assert_eq!(plan.card, CARD, "the card keeps its length");
        }
    }

    /// A shorter one waits for the difference, so the first line arrives as the card goes.
    #[test]
    fn a_short_introduction_waits_only_for_what_is_missing() {
        let plan = plan(CARD, Some(Duration::from_millis(1_800)));
        assert_eq!(plan.play_after, Duration::from_millis(3_200));
        assert_eq!(plan.play_after + Duration::from_millis(1_800), plan.card);

        // A song whose first line is on the screen from its first tick waits for the whole card.
        assert_eq!(plan_for_ms(0).play_after, CARD);
    }

    fn plan_for_ms(reveal: u64) -> Plan {
        plan(CARD, Some(Duration::from_millis(reveal)))
    }

    /// A song with no word times waits for the whole card, because nothing says when it starts.
    #[test]
    fn a_song_with_no_word_times_waits_for_the_whole_card() {
        assert_eq!(plan(CARD, None).play_after, CARD);
    }

    /// A held song has a moment to play at, and one that plays at once has none.
    #[test]
    fn an_announcement_holds_a_play_only_for_a_song_that_waits() {
        let now = Instant::now();

        let (held, plays_now) = Announce::begin(plan(CARD, None), now);
        assert!(!plays_now);
        assert_eq!(held.play_at, Some(now + CARD));
        assert!(!held.play_is_due(now + Duration::from_millis(4_999)));
        assert!(held.play_is_due(now + CARD));

        let (under, plays_now) = Announce::begin(plan_for_ms(9_000), now);
        assert!(plays_now);
        assert_eq!(under.play_at, None);
        assert!(!under.play_is_due(now + CARD));
    }

    /// The announcement is over when the card is down and the song is playing, and not before.
    #[test]
    fn an_announcement_is_over_when_the_card_is_down_and_the_song_plays() {
        let now = Instant::now();
        let (mut held, _) = Announce::begin(plan(CARD, None), now);

        assert!(!held.is_over(now + CARD), "the song has not played yet");
        held.play_at = None;
        assert!(
            !held.is_over(now + Duration::from_secs(4)),
            "the card is up"
        );
        assert!(held.is_over(now + CARD));
    }

    /// The card reports how far through it is, and reports nothing once it is down.
    #[test]
    fn the_card_says_how_far_through_it_is_until_it_comes_down() {
        let now = Instant::now();
        let (announce, _) = Announce::begin(plan_for_ms(9_000), now);

        assert_eq!(announce.card(now), Some((Duration::ZERO, CARD)));
        assert_eq!(
            announce.card(now + Duration::from_secs(2)),
            Some((Duration::from_secs(2), CARD))
        );
        assert_eq!(announce.card(now + CARD), None);
    }
}
