//! What the machine tells its remotes, as it happens.
//!
//! One `tokio` broadcast channel, one WebSocket per remote. Every event is tagged so a client can
//! switch on `event` and ignore what it does not care about.
//!
//! **Per-syllable position is never streamed.** That is the one rule this module exists to enforce.
//! A song has thousands of syllables; pushing one message each would mean a thousand-fold increase
//! in traffic to tell a remote something it can work out itself. Instead the position rides on the
//! periodic `state` event at [`STATE_INTERVAL`], and a remote that wants to follow the words fetches
//! `/songs/{number}/lyrics` once and interpolates locally against its own clock. The display does
//! not use this channel at all — it reads the engine's atomics directly, every frame.
//!
//! `lyric_line` is sent, at line granularity, because a remote showing "now singing" needs a cue it
//! cannot derive when the song was paused, seeked or skipped.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::dto::{MicsDto, NowPlayingDto, QueueDto, SettingsDto, StateDto};
use crate::machine::Controller;

/// How often the periodic state event goes out.
///
/// 4 Hz: fast enough that a progress bar does not visibly step, slow enough that ten phones cost
/// nothing. A remote wanting smoother than this interpolates — it knows the tempo and the position.
pub const STATE_INTERVAL: Duration = Duration::from_millis(250);

/// How many events a slow subscriber may fall behind before it starts losing them.
///
/// Generous, because the expensive case is a phone that locked its screen mid-song and comes back:
/// it should catch up rather than be told it desynced. Past this it is told, which is the honest
/// outcome — see [`Event::Desync`].
pub const CHANNEL_CAPACITY: usize = 256;

/// Something a remote should know about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The periodic snapshot, carrying the playback position.
    State {
        /// The whole state, so a client that missed everything else recovers from this alone.
        state: StateDto,
    },
    /// The queue changed — added, removed, reordered, or a song was taken off the front.
    QueueChanged {
        /// The queue as it now stands.
        queue: QueueDto,
    },
    /// A song started playing.
    SongStarted {
        /// What started.
        now_playing: NowPlayingDto,
    },
    /// A song finished, was skipped or was stopped.
    SongEnded {
        /// Why it ended, so a remote can distinguish "next singer" from "somebody hit stop".
        reason: EndReason,
    },
    /// The current lyric line advanced.
    LyricLine {
        /// Which line, indexed into the timeline the lyrics endpoint returned.
        index: usize,
        /// When it starts, so a remote can align it against its own interpolation.
        start_ms: u32,
        /// When it ends.
        end_ms: u32,
        /// The words, so a remote that never fetched the lyrics can still show them.
        text: String,
    },
    /// Playback settings changed.
    SettingsChanged {
        /// The settings as they now stand.
        settings: SettingsDto,
    },
    /// A microphone's state changed.
    MicsChanged {
        /// Every channel, since a remote showing a mixer wants them all anyway.
        mics: MicsDto,
    },
    /// The wallpaper changed.
    WallpaperChanged {
        /// The file name now on screen.
        current: Option<String>,
    },
    /// This subscriber fell far enough behind that events were dropped.
    ///
    /// Sent instead of silently continuing, because a remote holding a queue it believes is current
    /// and is not will show the wrong singer as next. On receiving this a client should re-fetch.
    Desync {
        /// How many events went missing.
        skipped: u64,
    },
}

/// Why a song stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// It reached its end.
    Finished,
    /// Somebody skipped it.
    Skipped,
    /// Somebody stopped playback.
    Stopped,
    /// A demo song gave the deck up to a song somebody queued.
    ///
    /// **Its own reason rather than [`Self::Skipped`], because nobody skipped.** Each of the three
    /// above names who ended the song, and this one is the machine standing aside — the only case
    /// in which a song stops with nobody having asked for that song to stop. A remote that reports
    /// *skipped* here would be telling a room somebody took a turn away, when what happened is that
    /// a turn began.
    ///
    /// Only ever seen for a demo song. See `What the machine does when nobody is singing`.
    Yielded,
}

/// The publishing end of the event stream.
///
/// Cloneable and cheap: the handlers hold one, the state ticker holds one, and `km-app`'s control
/// thread holds one so engine events reach remotes without going through HTTP.
#[derive(Debug, Clone)]
pub struct Events {
    sender: broadcast::Sender<Event>,
}

impl Default for Events {
    fn default() -> Self {
        Self::new()
    }
}

impl Events {
    /// A new channel.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { sender }
    }

    /// Publishes an event.
    ///
    /// Returns how many subscribers it reached. Failure is not an error and is not logged as one:
    /// nobody subscribed is the normal state of a karaoke machine with no phone connected.
    pub fn publish(&self, event: Event) -> usize {
        self.sender.send(event).unwrap_or(0)
    }

    /// A new subscription, receiving events published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// How many remotes are listening.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// Publishes a `state` event every [`STATE_INTERVAL`] for as long as the future is polled.
///
/// Runs unconditionally rather than only while something is playing. An idle machine's remote still
/// needs to learn that the queue emptied or that somebody changed the key at the machine itself,
/// and a 4 Hz tick costs nothing when there are no subscribers — `publish` on an empty channel is a
/// single atomic read.
pub async fn run_state_ticker<C: Controller + ?Sized>(controller: Arc<C>, events: Events) {
    let mut ticker = tokio::time::interval(STATE_INTERVAL);
    // The default `Burst` behavior would fire back-to-back ticks to catch up after the process was
    // descheduled, which is exactly wrong for a heartbeat: nobody wants four identical states at
    // once, they want the current one.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let snapshot = controller.snapshot();
        events.publish(Event::State {
            state: StateDto::from(&snapshot),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Snapshot;

    #[test]
    fn events_are_tagged_so_a_client_can_switch_on_them() {
        let event = Event::State {
            state: StateDto::from(&Snapshot::default()),
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["event"], "state");
        assert_eq!(json["state"]["transport"], "idle");
    }

    #[test]
    fn a_lyric_line_carries_its_words_and_its_timing() {
        let json = serde_json::to_value(Event::LyricLine {
            index: 3,
            start_ms: 1000,
            end_ms: 2500,
            text: "Hello darkness".to_owned(),
        })
        .expect("serialize");
        assert_eq!(json["event"], "lyric_line");
        assert_eq!(json["index"], 3);
        assert_eq!(json["text"], "Hello darkness");
    }

    #[test]
    fn an_ending_says_which_kind_it_was() {
        for (reason, expected) in [
            (EndReason::Finished, "finished"),
            (EndReason::Skipped, "skipped"),
            (EndReason::Stopped, "stopped"),
        ] {
            let json = serde_json::to_value(Event::SongEnded { reason }).expect("serialize");
            assert_eq!(json["event"], "song_ended");
            assert_eq!(json["reason"], expected);
        }
    }

    #[test]
    fn every_planned_event_name_is_spelled_the_way_the_plan_spells_it() {
        // The names are API: a client matches on them. This pins the eight from
        // `docs/ARCHITECTURE.md` plus the desync notice this module adds.
        let names: Vec<String> = [
            Event::State {
                state: StateDto::from(&Snapshot::default()),
            },
            Event::QueueChanged {
                queue: QueueDto::new(&[]),
            },
            Event::SongEnded {
                reason: EndReason::Finished,
            },
            Event::LyricLine {
                index: 0,
                start_ms: 0,
                end_ms: 0,
                text: String::new(),
            },
            Event::SettingsChanged {
                settings: crate::machine::Settings::default().into(),
            },
            Event::MicsChanged {
                mics: MicsDto::new(&[]),
            },
            Event::WallpaperChanged { current: None },
            Event::Desync { skipped: 1 },
        ]
        .iter()
        .map(|event| {
            serde_json::to_value(event).expect("serialize")["event"]
                .as_str()
                .expect("tagged")
                .to_owned()
        })
        .collect();
        assert_eq!(
            names,
            [
                "state",
                "queue_changed",
                "song_ended",
                "lyric_line",
                "settings_changed",
                "mics_changed",
                "wallpaper_changed",
                "desync",
            ]
        );
    }

    #[tokio::test]
    async fn a_published_event_reaches_every_subscriber() {
        let events = Events::new();
        let mut first = events.subscribe();
        let mut second = events.subscribe();
        assert_eq!(events.subscriber_count(), 2);

        assert_eq!(events.publish(Event::Desync { skipped: 7 }), 2);
        assert_eq!(
            first.recv().await.expect("received"),
            Event::Desync { skipped: 7 }
        );
        assert_eq!(
            second.recv().await.expect("received"),
            Event::Desync { skipped: 7 }
        );
    }

    #[test]
    fn publishing_with_nobody_listening_is_not_an_error() {
        let events = Events::new();
        assert_eq!(events.publish(Event::Desync { skipped: 1 }), 0);
        assert_eq!(events.subscriber_count(), 0);
    }

    #[test]
    fn a_subscriber_sees_only_what_is_published_after_it_joined() {
        let events = Events::new();
        events.publish(Event::Desync { skipped: 1 });
        let mut late = events.subscribe();
        // Nothing buffered from before: a WebSocket client's first `state` tick is what syncs it,
        // which is at most 250 ms away.
        assert!(late.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_subscriber_that_falls_too_far_behind_is_told_it_lagged() {
        let events = Events::new();
        let mut receiver = events.subscribe();
        for skipped in 0..(CHANNEL_CAPACITY as u64 + 10) {
            events.publish(Event::Desync { skipped });
        }
        // The channel reports the overflow rather than quietly renumbering, which is what the
        // WebSocket handler turns into a `desync` event for the client.
        assert!(matches!(
            receiver.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn the_state_ticker_publishes_at_four_hertz() {
        let events = Events::new();
        let mut receiver = events.subscribe();
        let machine = Arc::new(crate::testing::TestMachine::new());
        let handle = tokio::spawn(run_state_ticker(machine, events.clone()));

        // With the clock paused, tokio advances it whenever every task is idle, so awaiting the
        // events costs no real time and the virtual elapsed time is exact. `interval` fires once
        // immediately, so five events span four intervals -- one second of song time.
        let start = tokio::time::Instant::now();
        for _ in 0..5 {
            let event = receiver.recv().await.expect("the ticker publishes");
            assert!(matches!(event, Event::State { .. }));
        }
        let elapsed = start.elapsed();
        handle.abort();
        assert_eq!(elapsed, STATE_INTERVAL * 4, "the tick rate drifted");
    }
}
