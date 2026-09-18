//! The fan-out to open pages.
//!
//! One Server-Sent Events stream per page, opened on load and closed when the page leaves the
//! screen — see the tail of `static/live.js` for why the second half of that sentence is
//! load-bearing rather than tidy. What travels here is
//! **rendered HTML fragments**, not JSON: `static/live.js` replaces the element carrying the matching
//! `data-sse` attribute, so the page does no assembly of its own and every fragment is rendered by
//! the same askama template that rendered it into the page in the first place.
//!
//! Why SSE and not the WebSocket the API already has: a browser cannot put an `Authorization` header
//! on a `WebSocket`, so a page that wanted to follow the machine directly would be stuck the moment
//! anybody moved `events.subscribe` behind the password. The remote holds that one connection on the
//! browser's behalf — in the offline app literally, in the online mode by subscribing in-process —
//! and re-broadcasts it here, where the credentials are a cookie on an ordinary GET.
//!
//! Two things are borrowed wholesale from the Go remote this follows, because both were learned the
//! hard way and neither is guessable:
//!
//! * **The latest value of every state-bearing event is kept and replayed to a new subscriber.**
//!   Without it a page that has just loaded shows nothing until the next change, which on a quiet
//!   machine is never.
//! * **The queue count is a different event from the queue list.** One event cannot be swapped into
//!   two places in a document, and the count is on every page while the list is on one. The same
//!   split separates the tab-bar connection dot from the offline banner.
//!
//! A third thing is this crate's own, and was learned late: **a fragment is rendered once per
//! language and each subscriber is given only its own copies.** What travels here is finished HTML,
//! and the pump that renders it is one task per process with no viewer to ask — so before this the
//! fan-out had no locale anywhere in it and every pushed fragment arrived saying `⟦control-key⟧`.
//! A page was correct as it loaded, through `views::page`, and was overwritten a second later by the
//! replay of a frame nobody had given a catalog to.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use tokio::sync::broadcast;

use km_locale::Locale;

/// How many frames a subscriber may fall behind before it is told to resynchronize.
///
/// A phone that locks its screen mid-song stops reading; the channel must not let that stall the
/// task feeding it. On overflow the subscriber is sent the current state afresh rather than the
/// frames it missed, which is both cheaper and more correct — it wanted the latest, not the history.
///
/// **"The current state" means all of it**, which is what [`Hub::stream`]'s pending queue is for.
/// This branch used to resend the first frame of the snapshot and drop the rest — so the page whose
/// screen had been locked came back with a fresh player card, a stale banner and a stale dot, and
/// nothing ever corrected them: the pump only republishes the connection when it *changes*, so a
/// page that missed the transition never hears about it again.
///
/// **Divided by the number of languages**, which is why it is not 64. Every frame is broadcast once
/// per locale and each subscriber discards all but its own, so a channel of 64 is 32 frames of
/// headroom for a two-language build and would be 16 for a four-language one. The number that
/// matters is how far behind a page may fall in the fragments *it* is being sent.
const CHANNEL_CAPACITY: usize = 64 * Locale::ALL.len();

/// The name of the event carrying the player card.
///
/// Everything on the Now page that is a *control*: the transport row, the steppers, the sliders, the
/// melody toggle, and the song's name. Republished only when the song or the settings change, which
/// is what makes it safe to replace outright — see [`POSITION`].
pub const PLAYER: &str = "player";
/// The name of the event carrying the elapsed time and the progress bar.
///
/// Split from [`PLAYER`] deliberately, and it is the reason this remote needs none of htmx's
/// `hx-preserve` machinery. The machine publishes its state every 250 ms and almost all of what
/// changes is the position; replacing the whole player card at that rate would destroy and rebuild
/// every button roughly four times a second, including the one under somebody's finger. This
/// fragment holds only what actually moves, and the pump sends it at most once a second.
pub const POSITION: &str = "position";
/// The name of the event carrying the Queue page's now bar.
///
/// The same state as [`PLAYER`] and a different fragment — what is playing and the transport row,
/// with no progress bar and none of the steppers. A second event rather than a second target for the
/// first, on the reasoning already given above for the queue list and the queue badge: one event
/// carries one rendered fragment, and one fragment cannot be swapped into two places that want to
/// look different.
///
/// Published at exactly the moments [`PLAYER`] is, which costs one render and is safe for a reason
/// worth stating: the bar is a strict subset of the card, so a card whose markup did not change did
/// not change the bar either, and the pump's existing comparison covers both.
pub const NOWBAR: &str = "nowbar";
/// The name of the event carrying the queue list.
pub const QUEUE: &str = "queue";
/// The name of the event carrying the tab-bar queue badge.
pub const QUEUE_COUNT: &str = "queuecount";
/// The name of the event carrying the tab-bar connection dot.
pub const CONN: &str = "conn";
/// The name of the event carrying the offline banner.
pub const BANNER: &str = "banner";
/// The name of the event carrying the Now tab's machine card.
///
/// Offline only, and published only where a build has a `Remote::connect` — so the machine's own
/// remote emits this event never rather than emitting an empty one.
///
/// **The address input is not in this fragment**, and this is the constant that made that necessary:
/// the pump republishes the card every second, so an input inside it would be replaced under
/// somebody's finger and lose whatever was half typed. The form lives outside the swap target, in
/// `now.html`.
pub const MACHINE: &str = "machine";
/// The name of the event carrying a toast.
pub const TOAST: &str = "toast";

/// Every event whose latest value is worth replaying to a page that has just opened.
///
/// [`TOAST`] is deliberately absent: a toast is a thing that happened, and replaying "Queued: Tempo
/// Perdido" to somebody who has just opened the queue page would be reporting an event they did not
/// cause and cannot act on.
const REPLAYED: [&str; 8] = [
    PLAYER,
    POSITION,
    NOWBAR,
    QUEUE,
    QUEUE_COUNT,
    CONN,
    BANNER,
    MACHINE,
];

/// One rendered fragment, addressed by event name and by the language it is written in.
///
/// **A fragment is rendered once per language, not once.** The pump has no viewer to ask — it is
/// one task per process feeding every open page — while the language is a choice each device made
/// for itself, so one rendered string cannot serve two phones at a party in two languages. The
/// locale rides on the frame and [`Hub::stream`] hands each subscriber only its own.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Which `sse-swap` target it belongs to.
    pub event: &'static str,
    /// Which language this copy is written in.
    pub locale: Locale,
    /// The HTML.
    pub html: String,
}

/// The fan-out.
#[derive(Clone)]
pub struct Hub {
    sender: broadcast::Sender<Frame>,
    latest: Arc<Mutex<HashMap<(Locale, &'static str), String>>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    /// A hub with nothing published yet.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            sender,
            latest: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Publishes a fragment, remembering it if it is one of the replayed kinds.
    ///
    /// A send with no subscribers is not an error — nobody has the page open, which is the usual
    /// state of a machine under a television.
    pub fn publish(&self, locale: Locale, event: &'static str, html: impl Into<String>) {
        let html = html.into();
        if REPLAYED.contains(&event) {
            self.latest
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert((locale, event), html.clone());
        }
        let _ = self.sender.send(Frame {
            event,
            locale,
            html,
        });
    }

    /// Publishes a toast. Never remembered — see [`REPLAYED`].
    pub fn toast(&self, locale: Locale, html: impl Into<String>) {
        self.publish(locale, TOAST, html);
    }

    /// The latest value of every replayed event, in one language, in a stable order.
    fn snapshot(&self, locale: Locale) -> Vec<Frame> {
        let latest = self
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        REPLAYED
            .iter()
            .filter_map(|event| {
                latest.get(&(locale, *event)).map(|html| Frame {
                    event,
                    locale,
                    html: html.clone(),
                })
            })
            .collect()
    }

    /// The fragments one page is owed: everything known now, then everything as it happens.
    ///
    /// Separate from [`Self::stream`] so that a test can drive it. An `Sse` is a response and not a
    /// stream, so a resynchronization defect the whole crate depends on had nowhere to be asserted.
    fn frames(&self, locale: Locale) -> impl Stream<Item = Frame> + use<> {
        let replay = stream::iter(self.snapshot(locale));
        let hub = self.clone();
        // The state is the subscription *and* whatever a lag has left to hand over. `unfold` yields
        // one item per poll, so a catch-up of several frames cannot be returned in one go -- which
        // is exactly how this came to send only the first of them.
        let live = stream::unfold(
            (self.sender.subscribe(), VecDeque::<Frame>::new()),
            move |(mut receiver, mut pending)| {
                let hub = hub.clone();
                async move {
                    if let Some(frame) = pending.pop_front() {
                        return Some((frame, (receiver, pending)));
                    }
                    loop {
                        match receiver.recv().await {
                            // Another language's copy of the same fragment. Every frame reaches
                            // every subscriber and each keeps only its own — the filter is here
                            // rather than on the channel because a broadcast has one queue.
                            Ok(frame) if frame.locale != locale => {}
                            Ok(frame) => return Some((frame, (receiver, pending))),
                            // The subscriber fell behind. Sending the frames it missed would be
                            // both slower and wrong -- it wants the current state, not the history
                            // -- so it gets the current state, **all of it**, and the gap closes.
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::debug!(
                                    skipped,
                                    "a page fell behind; resending the latest state"
                                );
                                pending.extend(hub.snapshot(locale));
                                if let Some(frame) = pending.pop_front() {
                                    return Some((frame, (receiver, pending)));
                                }
                            }
                            Err(broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                }
            },
        );
        replay.chain(live)
    }

    /// The response for `GET /events`, in the language this device reads the remote in.
    ///
    /// The locale comes from the request rather than from the hub because it is the *viewer's*, and
    /// two devices on one machine may differ. See [`Frame`].
    pub fn stream(
        &self,
        locale: Locale,
    ) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>> + use<>> {
        let events = self.frames(locale).map(|frame| {
            Ok::<_, Infallible>(SseEvent::default().event(frame.event).data(frame.html))
        });

        // The keep-alive is not decoration: a phone on mobile data sits behind a NAT that drops an
        // idle connection in a couple of minutes, and a karaoke machine between songs is idle.
        Sse::new(events).keep_alive(KeepAlive::default())
    }

    /// How many pages are listening. For tests and for a log line at shutdown.
    pub fn listeners(&self) -> usize {
        self.sender.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two languages this build has, named once so a test reads as being about the fan-out
    /// rather than about Portuguese.
    const EN: Locale = Locale::English;
    const PT: Locale = Locale::BrazilianPortuguese;

    #[test]
    fn a_new_hub_has_nothing_to_replay() {
        let hub = Hub::new();
        assert!(hub.snapshot(EN).is_empty());
    }

    #[test]
    fn the_latest_value_of_each_state_event_is_kept() {
        let hub = Hub::new();
        hub.publish(EN, PLAYER, "<div>one</div>");
        hub.publish(EN, PLAYER, "<div>two</div>");
        hub.publish(EN, QUEUE, "<ul></ul>");
        let snapshot = hub.snapshot(EN);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].event, PLAYER);
        assert_eq!(snapshot[0].html, "<div>two</div>");
        assert_eq!(snapshot[1].event, QUEUE);
    }

    /// Replaying a toast would report something to somebody who did not cause it and cannot act on
    /// it — the queue page opening and announcing a song queued five minutes ago.
    #[test]
    fn a_toast_is_never_replayed() {
        let hub = Hub::new();
        hub.toast(EN, "<div>Queued: Tempo Perdido</div>");
        assert!(hub.snapshot(EN).is_empty());
    }

    #[test]
    fn the_snapshot_order_is_stable() {
        let hub = Hub::new();
        hub.publish(EN, BANNER, "b");
        hub.publish(EN, CONN, "c");
        hub.publish(EN, PLAYER, "p");
        let names: Vec<&str> = hub.snapshot(EN).iter().map(|f| f.event).collect();
        assert_eq!(names, vec![PLAYER, CONN, BANNER]);
    }

    /// A phone that opens the Queue tab while nothing is happening must still be told what is
    /// playing. The Now tab has always been, through [`PLAYER`]; the bar is the same promise for the
    /// other page, and a state event left out of [`REPLAYED`] shows nothing until the next change,
    /// which on a quiet machine is never.
    #[test]
    fn the_now_bar_is_replayed_to_a_page_that_has_just_opened() {
        let hub = Hub::new();
        hub.publish(EN, NOWBAR, "<div>Corcovado</div>");
        let snapshot = hub.snapshot(EN);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].event, NOWBAR);
    }

    /// **One event name, two remembered fragments.** The replay is what a page sees a second after
    /// it loads, so a hub that kept one copy per event would hand a Portuguese phone whichever
    /// language the last render happened to be in — which is the shape the bug had before the
    /// locale was on the frame.
    #[test]
    fn each_language_is_remembered_and_replayed_on_its_own() {
        let hub = Hub::new();
        hub.publish(EN, PLAYER, "<div>Nothing playing</div>");
        hub.publish(PT, PLAYER, "<div>Nada tocando</div>");

        let english = hub.snapshot(EN);
        assert_eq!(english.len(), 1, "{english:?}");
        assert_eq!(english[0].html, "<div>Nothing playing</div>");

        let portuguese = hub.snapshot(PT);
        assert_eq!(portuguese.len(), 1, "{portuguese:?}");
        assert_eq!(portuguese[0].html, "<div>Nada tocando</div>");
    }

    #[tokio::test]
    async fn a_published_frame_reaches_a_subscriber() {
        let hub = Hub::new();
        let mut receiver = hub.sender.subscribe();
        hub.publish(EN, QUEUE_COUNT, "<span>3</span>");
        let frame = receiver.recv().await.expect("a frame");
        assert_eq!(frame.event, QUEUE_COUNT);
        assert_eq!(frame.locale, EN);
        assert_eq!(frame.html, "<span>3</span>");
    }

    /// The broadcast carries every language and the subscriber keeps one. A page reading Portuguese
    /// must never be handed the English copy — it would be swapped straight into the document.
    #[tokio::test]
    async fn a_page_is_never_handed_another_languages_fragment() {
        let hub = Hub::new();
        let mut frames = Box::pin(hub.frames(PT));

        hub.publish(EN, PLAYER, "<div>Nothing playing</div>");
        hub.publish(PT, PLAYER, "<div>Nada tocando</div>");

        let frame = frames.next().await.expect("a frame");
        assert_eq!(frame.locale, PT);
        assert_eq!(frame.html, "<div>Nada tocando</div>");
    }

    #[test]
    fn publishing_with_nobody_listening_is_not_an_error() {
        let hub = Hub::new();
        assert_eq!(hub.listeners(), 0);
        hub.publish(EN, PLAYER, "<div></div>");
    }

    /// **The whole state comes back after a lag, not the first frame of it.**
    ///
    /// The case is the one [`CHANNEL_CAPACITY`] is documented against — a phone that locks its
    /// screen mid-song stops reading — and it is the reported fault: switching away from the
    /// Android remote and back showed a stale banner that nothing ever corrected, because the pump
    /// republishes the connection only when it *changes*. This branch used to hand over
    /// `snapshot().next()`, so the page was resynchronized with a player card and nothing else.
    #[tokio::test]
    async fn a_page_that_fell_behind_gets_every_state_event_back() {
        let hub = Hub::new();
        hub.publish(EN, PLAYER, "<div>Tempo Perdido</div>");
        hub.publish(EN, CONN, "<span>on</span>");
        hub.publish(EN, BANNER, "<div></div>");

        let mut frames = Box::pin(hub.frames(EN));
        // The three the stream opens with. Asking for a fourth here would wait for ever.
        for _ in 0..3 {
            frames.next().await.expect("the replay");
        }

        // Overrun the channel without reading a thing, which is what a locked screen does.
        hub.publish(
            EN,
            BANNER,
            "<div class=\"banner\">The machine is off.</div>",
        );
        hub.publish(EN, CONN, "<span>off</span>");
        for _ in 0..CHANNEL_CAPACITY {
            hub.publish(EN, POSITION, "<div>0:01</div>");
        }

        // Whatever the snapshot holds is handed over one frame at a time before the live feed
        // resumes, so reading exactly that many gets the catch-up and no more.
        let expected = REPLAYED
            .iter()
            .filter(|event| [PLAYER, POSITION, CONN, BANNER].contains(event))
            .count();
        let mut seen = Vec::new();
        for _ in 0..expected {
            seen.push(frames.next().await.expect("the catch-up").event);
        }

        for event in [PLAYER, POSITION, CONN, BANNER] {
            assert!(
                seen.contains(&event),
                "{event} was dropped from the resynchronization: {seen:?}"
            );
        }
    }

    /// The dot and the banner are the two the old behavior lost, and losing them is what a person
    /// sees: a red strip that will not go away on a machine that is answering perfectly.
    #[tokio::test]
    async fn the_catch_up_carries_the_latest_html_and_not_the_frame_that_was_missed() {
        let hub = Hub::new();
        hub.publish(
            EN,
            BANNER,
            "<div class=\"banner\">The machine is off.</div>",
        );

        let mut frames = Box::pin(hub.frames(EN));
        frames.next().await.expect("the replay");

        // It came back while nobody was reading.
        hub.publish(EN, BANNER, "<div data-sse=\"banner\"></div>");
        for _ in 0..CHANNEL_CAPACITY {
            hub.publish(EN, POSITION, "<div>0:02</div>");
        }

        let mut banners = Vec::new();
        for _ in 0..2 {
            let frame = frames.next().await.expect("the catch-up");
            if frame.event == BANNER {
                banners.push(frame.html);
            }
        }
        assert_eq!(
            banners,
            vec![r#"<div data-sse="banner"></div>"#.to_owned()],
            "the page is caught up to now, not walked through the history"
        );
    }

    /// A catch-up is the viewer's own language too. The lag branch reaches for the snapshot, and a
    /// snapshot that ignored the locale would hand a Portuguese phone the English state at exactly
    /// the moment it had lost track of everything.
    #[tokio::test]
    async fn a_catch_up_is_in_the_language_the_page_is_reading() {
        let hub = Hub::new();
        hub.publish(PT, BANNER, "<div>A máquina está desligada.</div>");

        let mut frames = Box::pin(hub.frames(PT));
        frames.next().await.expect("the replay");

        hub.publish(EN, BANNER, "<div>The machine is off.</div>");
        for _ in 0..CHANNEL_CAPACITY {
            hub.publish(EN, POSITION, "<div>0:03</div>");
        }

        let frame = frames.next().await.expect("the catch-up");
        assert_eq!(frame.locale, PT);
        assert_eq!(frame.html, "<div>A máquina está desligada.</div>");
    }
}
