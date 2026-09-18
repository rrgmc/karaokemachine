//! Holding a program's most recent log records in memory, for a surface that shows them as they
//! happen.
//!
//! Every program here says what it is doing through `tracing`, and the two places it can end up are
//! a console and — when somebody asks for one — a file. Neither reaches the machine you most want
//! them from. A box under a television has no console and nobody logged into it; a run started by
//! double-clicking its icon on Windows has a null standard output handle and discards every line;
//! and reading a file means finding the folder, over SSH or `adb`, after the moment has passed.
//!
//! So: a third destination that is neither, holding the last [`CAPACITY`] records in memory and
//! handing them to whoever asks. The machine serves them over its API and the development console
//! draws them, which between them answer *"it stopped and I do not know why"* from a browser.
//!
//! # It is always there
//!
//! Unlike the file beside it, this is not asked for by name. The run that most needs a log is the
//! one nobody thought to arm, and a fixed, small buffer is what closes that: by the time somebody
//! wants the last hundred lines it is too late to decide to keep them. Nothing is published by
//! holding them — reaching them is an admin route, and a program that mounts no such route keeps
//! its buffer to itself.
//!
//! **The verbosity ladder still decides what goes in it.** A tap is a destination and `-v` is a
//! volume, and a second filter here would be a second answer to a question `EnvFilter` already
//! answers for the console and the file. The directive in force travels with the tap, through
//! [`LogTap::with_filter`], so a surface can say what this run is keeping rather than leaving
//! somebody to wonder where the `debug` lines went.
//!
//! # A `Layer`, where the two crates beside it are `MakeWriter`s
//!
//! [`km_logfile`](../km_logfile/index.html) and [`km_androidlog`](../km_androidlog/index.html) both
//! take the formatted line `tracing`'s own formatter produces, because a file and logcat both want
//! a line. This wants the *parts* — the level to colour by, the target to filter on, the message
//! apart from the fields — so it has to sit a level lower and visit the event itself. That is the
//! whole of why this crate is shaped differently from its siblings.
//!
//! **A crate rather than code in `km-api`**, which is the one place it would otherwise go.
//! `km-api` carries `tracing-subscriber` as a *dev*-dependency, deliberately: the library emits
//! events and leaves the subscriber to whichever binary is running it. A `Layer` in there would
//! make a subscriber implementation a real dependency of every crate that describes a remote, none
//! of which has any business linking one.
//!
//! Nothing here formats a clock. A record carries milliseconds since the epoch and whoever draws it
//! turns that into a time, which keeps a date library out of a crate that needs nothing else.
//!
//! **No span context is captured.** This feeds line readers, the machine's event stream carries none
//! either, and taking spans would put a `LookupSpan` requirement in the layer's bound to serve
//! something no surface draws.
//!
//! # Nothing here emits an event
//!
//! Not the layer, not [`LogTap::push`], not the visitor, and nothing a caller writes in the loop
//! that drains one. A tap that logs feeds itself: `tracing` drops a nested event silently rather
//! than recursing, so the failure is not a crash but a diagnostic that vanishes with nothing saying
//! why.
//!
//! ```no_run
//! use tracing_subscriber::layer::SubscriberExt as _;
//! use tracing_subscriber::util::SubscriberInitExt as _;
//!
//! let tap = km_logtap::LogTap::new().with_filter("info");
//! tracing_subscriber::registry()
//!     .with(tracing_subscriber::EnvFilter::new("info"))
//!     .with(tracing_subscriber::fmt::layer())
//!     .with(tap.layer())
//!     .init();
//! km_logtap::install(tap);
//! ```

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// How many records are held, and so how far back a surface can look on arrival.
///
/// Five hundred is a few minutes of an ordinary run and the whole of a noisy start, which is the
/// span somebody asking *"what just happened?"* means. The number is bounded rather than generous
/// because a fixed cost is the entire argument for holding this in every run: with the two limits
/// below, the worst case is a couple of megabytes and it cannot grow into anything else.
pub const CAPACITY: usize = 500;

/// How many records a slow reader may fall behind before it starts losing them.
///
/// The same figure the machine's event channel uses, and for the same reason: generous enough that
/// a reader which stalled for a moment catches up rather than being told it missed something.
/// Past this it is told, which is the honest outcome.
///
/// It is a *channel* backlog and not a history. The history is [`LogTap::tail`], which every reader
/// is given when it arrives.
pub const CHANNEL_CAPACITY: usize = 256;

/// The longest message a record keeps.
///
/// A cap rather than a promise to be brief: one `Debug` of an unexpectedly large value is all it
/// takes for a bounded ring to stop being bounded in bytes. Cut on a character boundary and marked,
/// so a surface shows a shortened line rather than replacement characters.
pub const MAX_MESSAGE_BYTES: usize = 2048;

/// The longest run of fields a record keeps, for [`MAX_MESSAGE_BYTES`]' reason.
pub const MAX_FIELDS_BYTES: usize = 1024;

/// What marks a value this crate shortened.
const ELLIPSIS: char = '…';

/// One event, in the parts a surface wants to draw it from.
///
/// **`level` and `target` cost nothing.** `tracing` metadata is `'static`, so both are borrowed
/// rather than copied and the only allocations a record makes are its two strings.
///
/// **The level stays a [`tracing::Level`] rather than becoming a string here.** How a level is
/// spelled on a wire is a wire's business, and this crate has no wire; a consumer that serialises
/// one says so in the type that describes its own protocol.
#[derive(Debug, Clone)]
pub struct Record {
    /// Which record this is, counting from one, for the life of the tap.
    ///
    /// **What makes the tail and the live stream joinable.** A reader is given a tail and a
    /// subscription, and the two deliberately overlap — see [`LogTap::tail_and_subscribe`]. This is
    /// how it drops the overlap instead of showing a line twice.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    ///
    /// A number rather than a formatted time: whoever draws this has a clock and a locale, and this
    /// crate has neither and wants no date library to get them.
    pub at_ms: u64,
    /// How serious it is.
    pub level: tracing::Level,
    /// Which crate or module said it, as a filter directive would name it.
    pub target: &'static str,
    /// The event's own message, without its fields.
    pub message: String,
    /// Everything else the event carried, as `name=value` pairs separated by spaces.
    ///
    /// Empty when there were none, which is the common case. Formatted here rather than kept
    /// structured because every reader of this shows them as text, and a map would cost an
    /// allocation per field to serve none of them.
    pub fields: String,
}

/// The last [`CAPACITY`] records, and a channel carrying the ones still to come.
///
/// Cloning is cheap and shares one buffer: the layer holds one, whatever serves it holds another,
/// and [`install`] keeps a third.
#[derive(Clone)]
pub struct LogTap(Arc<Inner>);

/// The bits behind the [`Arc`]. Separate only so that [`LogTap`] can be cloned freely.
struct Inner {
    /// The ring. Oldest at the front, so a full buffer drops from the front and pushes to the back.
    ///
    /// Records are held behind an [`Arc`] so that the copy in the channel is a pointer rather than
    /// a second payload: a full ring and a full channel are one set of strings, not two.
    recent: Mutex<VecDeque<Arc<Record>>>,
    /// How many the ring may hold.
    capacity: usize,
    /// The last [`Record::seq`] handed out.
    seq: AtomicU64,
    /// How many records have fallen off the front since this tap was made.
    ///
    /// What tells a reader its tail is a tail rather than the whole run.
    dropped: AtomicU64,
    /// Where a live reader is fed from.
    sender: broadcast::Sender<Arc<Record>>,
    /// The filter directive this run is keeping records under, when the caller said.
    ///
    /// A slot rather than a field, so that saying so is one call on a tap that may already have
    /// been cloned rather than a constructor argument every caller has to carry.
    filter: OnceLock<String>,
}

impl Default for LogTap {
    fn default() -> Self {
        Self::new()
    }
}

impl LogTap {
    /// A new, empty tap holding [`CAPACITY`] records.
    ///
    /// **Needs no runtime.** `broadcast::channel` allocates and nothing else, which is what lets a
    /// program build its tap while it is still setting up its subscriber — long before tokio
    /// exists.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(CAPACITY)
    }

    /// The same, holding a stated number of records.
    ///
    /// For a test that wants to watch a ring overflow without pushing five hundred records at it.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self(Arc::new(Inner {
            recent: Mutex::new(VecDeque::new()),
            capacity,
            seq: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            sender,
            filter: OnceLock::new(),
        }))
    }

    /// Records the filter directive this run is keeping records under.
    ///
    /// Carried rather than derived, because the tap cannot see the `EnvFilter` beside it. A surface
    /// that shows records shows this too: a pane with no `debug` lines in it is otherwise
    /// indistinguishable from a machine with nothing to say.
    ///
    /// Said once. A second call is ignored rather than refused: which directive is in force is
    /// settled when the subscriber is built, and nothing later is in a position to know better.
    #[must_use]
    pub fn with_filter(self, filter: impl Into<String>) -> Self {
        let _ = self.0.filter.set(filter.into());
        self
    }

    /// The records held now, oldest first.
    ///
    /// What a surface draws on arrival, so that a pane opened after the interesting moment still
    /// shows it.
    #[must_use]
    pub fn tail(&self) -> Vec<Arc<Record>> {
        self.recent().iter().cloned().collect()
    }

    /// A tail and a subscription, taken together so that nothing falls between them.
    ///
    /// **Both under the ring's lock, which is what leaves no window at all.** [`LogTap::push`] holds
    /// that same lock while it appends, so a subscription taken under it cannot be newer than the
    /// snapshot beside it: every record ends up in the tail, on the reader, or — where a push had
    /// appended and not yet sent — in both.
    ///
    /// The overlap is the deliberate half. A duplicate a reader drops by [`Record::seq`] beats a gap
    /// nobody can see, and the ordering that cannot duplicate is the one that *loses* records taken
    /// while a reader is arriving — which is exactly when a machine is busy enough to be watched.
    #[must_use]
    pub fn tail_and_subscribe(&self) -> (Vec<Arc<Record>>, broadcast::Receiver<Arc<Record>>) {
        let recent = self.recent();
        let reader = self.0.sender.subscribe();
        (recent.iter().cloned().collect(), reader)
    }

    /// A subscription, receiving records taken from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Record>> {
        self.0.sender.subscribe()
    }

    /// How many records the ring holds when it is full.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.0.capacity
    }

    /// How many records have fallen off the front since this tap was made.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.0.dropped.load(Ordering::Relaxed)
    }

    /// The filter directive this run is keeping records under, when the caller said.
    #[must_use]
    pub fn filter(&self) -> Option<&str> {
        self.0.filter.get().map(String::as_str)
    }

    /// How many readers are listening.
    #[must_use]
    pub fn reader_count(&self) -> usize {
        self.0.sender.receiver_count()
    }

    /// The `tracing` layer that fills it.
    ///
    /// Boxed because that is what lets a caller add it, or not, without the two shapes being
    /// different types — the same reason `km_logfile::LogFile::layer` is boxed.
    #[must_use]
    pub fn layer<S>(&self) -> Box<dyn Layer<S> + Send + Sync + 'static>
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        Box::new(TapLayer(self.clone()))
    }

    /// Takes one record: into the ring, and out to whoever is listening.
    ///
    /// **The one door**, used by the layer and by a test seeding a tap. A second way in would be a
    /// code path nothing exercises, which is also why this is public rather than a test double
    /// being offered instead.
    pub fn push(&self, mut record: Record) {
        shorten(&mut record.message, MAX_MESSAGE_BYTES);
        shorten(&mut record.fields, MAX_FIELDS_BYTES);
        record.seq = self.0.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let record = Arc::new(record);

        // **The lock is dropped before the send, and that is load-bearing.** Every thread in the
        // program logs, and holding this across a send would put each reader's wakeup inside a
        // mutex all of them are queueing for.
        {
            let mut recent = self.recent();
            while recent.len() >= self.0.capacity {
                recent.pop_front();
                self.0.dropped.fetch_add(1, Ordering::Relaxed);
            }
            recent.push_back(Arc::clone(&record));
        }

        // **Nobody listening is the ordinary state and is not an error.** It is also the one failure
        // that must never be reported: a tap that says something when it takes a record would take
        // that too, and then say something about it.
        let _ = self.0.sender.send(record);
    }

    /// The ring, with a poisoned lock recovered from rather than panicked on.
    ///
    /// A thread that panicked while holding this must not take logging down with it — the records
    /// already in the ring are exactly what somebody wants after a panic. `km_logfile` recovers a
    /// poisoned writer the same way and for the same reason.
    fn recent(&self) -> std::sync::MutexGuard<'_, VecDeque<Arc<Record>>> {
        self.0.recent.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Prints what it is and how full it is, and never what is in it.
///
/// A log record can hold anything the program was told, so a `Debug` that printed the ring would
/// put the whole buffer into whatever printed the tap.
impl std::fmt::Debug for LogTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogTap")
            .field("held", &self.recent().len())
            .field("capacity", &self.0.capacity)
            .field("dropped", &self.dropped())
            .finish()
    }
}

/// Shortens `text` to `limit` bytes on a character boundary, marking that it was cut.
fn shorten(text: &mut String, limit: usize) {
    if text.len() <= limit {
        return;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push(ELLIPSIS);
}

/// The layer half, kept private so that [`LogTap::layer`] is the only way to make one.
struct TapLayer(LogTap);

impl<S> Layer<S> for TapLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut parts = Parts::default();
        event.record(&mut parts);
        let metadata = event.metadata();
        self.0.push(Record {
            // Replaced by `push`, which is the only thing that may hand one out.
            seq: 0,
            at_ms: now_ms(),
            level: *metadata.level(),
            target: metadata.target(),
            message: parts.message,
            fields: parts.fields,
        });
    }
}

/// Pulls an event apart into the message and everything else.
#[derive(Default)]
struct Parts {
    message: String,
    fields: String,
}

impl Visit for Parts {
    /// **`message` is a field like any other and is separated by name.** `tracing` puts the text of
    /// `info!("...")` in a field called `message`, so a visitor that did not look for it would
    /// produce records whose message was empty and whose fields began `message=...`.
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // `{:?}` on the message field gives the text without quotes around it, the value behind
            // it being a `fmt::Arguments` rather than a `&str`.
            let _ = write!(self.message, "{value:?}");
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        // A write into a `String` cannot fail; the result is discarded rather than unwrapped so
        // that there is no panic anywhere in this crate's hot path.
        let _ = write!(self.fields, "{}={value:?}", field.name());
    }
}

/// Milliseconds since the Unix epoch, or zero on a clock set before it.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The tap this process installed, if it installed one.
fn slot() -> &'static OnceLock<LogTap> {
    static SLOT: OnceLock<LogTap> = OnceLock::new();
    &SLOT
}

/// Remembers the tap this process is filling, so that whatever serves it need not be handed one.
///
/// **A process-wide slot rather than an argument, because one of the two callers has nowhere to put
/// an argument.** A desktop run builds its subscriber on the way through a command line and could
/// pass a tap along it; an Android run reaches the same machine through `SDL_main` with no command
/// line at all, and that is the host with no console, where this is worth the most. One slot answers
/// both, and it is the shape this application already uses for a fact settled at startup.
///
/// Returns whether it took. A second call is a program installing two taps, which is a bug rather
/// than a race, so it is reported instead of replacing a tap that is already being filled.
pub fn install(tap: LogTap) -> bool {
    slot().set(tap).is_ok()
}

/// The tap this process installed, or `None` where nothing did.
///
/// `None` is the ordinary answer in a test, in an example and in any program that has not asked for
/// one — and it is what a surface reads to decide whether it has anything to serve.
#[must_use]
pub fn installed() -> Option<LogTap> {
    slot().get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(message: &str) -> Record {
        Record {
            seq: 0,
            at_ms: 0,
            level: tracing::Level::INFO,
            target: "km_logtap",
            message: message.to_owned(),
            fields: String::new(),
        }
    }

    /// A subscriber that writes into `tap` and nowhere else, installed for this thread only.
    ///
    /// `with_default` rather than `init`, because a global subscriber is set once per process and
    /// these tests share one with every other test in the crate.
    fn with_tap(tap: &LogTap, body: impl FnOnce()) {
        use tracing_subscriber::layer::SubscriberExt as _;
        let subscriber = tracing_subscriber::registry().with(tap.layer());
        tracing::subscriber::with_default(subscriber, body);
    }

    /// The buffer is a ring and not a log: it is bounded, what falls off is the oldest, and the
    /// count of what fell off is what tells a reader so.
    #[test]
    fn the_ring_keeps_the_newest_and_counts_what_fell_off() {
        let tap = LogTap::with_capacity(4);
        for index in 0..10 {
            tap.push(record(&index.to_string()));
        }

        let tail = tap.tail();
        assert_eq!(tail.len(), 4);
        assert_eq!(tail.first().expect("a first record").message, "6");
        assert_eq!(tail.last().expect("a last record").message, "9");
        assert_eq!(tap.dropped(), 6);
        assert_eq!(tap.capacity(), 4);
    }

    /// The sequence is what joins the tail to the live stream, so it has to start at one, never
    /// repeat, and go on counting past what the ring has thrown away.
    #[test]
    fn a_sequence_number_counts_every_record_and_never_repeats() {
        let tap = LogTap::with_capacity(2);
        for index in 0..5 {
            tap.push(record(&index.to_string()));
        }

        let seqs: Vec<u64> = tap.tail().iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![4, 5], "the ring holds the last two of five");
    }

    /// **The window this exists to close.** A record taken between the two halves must arrive
    /// somewhere: in the tail, on the reader, or — where a push is midway — on both, where a
    /// sequence number gets rid of the duplicate. What it must never do is vanish.
    #[test]
    fn a_reader_that_takes_a_tail_and_subscribes_loses_nothing() {
        let tap = LogTap::with_capacity(8);
        tap.push(record("before"));

        let (tail, mut reader) = tap.tail_and_subscribe();
        tap.push(record("after"));

        let mut seen: Vec<String> = tail.iter().map(|record| record.message.clone()).collect();
        let last = tail.last().map_or(0, |record| record.seq);
        while let Ok(record) = reader.try_recv() {
            if record.seq > last {
                seen.push(record.message.clone());
            }
        }

        assert_eq!(seen, vec!["before".to_owned(), "after".to_owned()]);
    }

    /// The same property against a thread that is genuinely running at the same time, which is the
    /// only way to exercise it: taken in sequence on one thread there is no interleaving to get
    /// wrong, and a test that cannot fail is not protecting anything.
    ///
    /// A real `std::thread` and not a task, deliberately. This crate is used before any runtime
    /// exists, and a spawned task on the single-threaded runtime a test gets by default cannot run
    /// concurrently with the test at all — which is exactly how a broken ordering passes.
    ///
    /// Rounds rather than one attempt, because a race is not reproduced by asking once.
    #[test]
    fn a_reader_arriving_mid_flight_still_sees_every_record() {
        for round in 0..200 {
            let tap = LogTap::with_capacity(CAPACITY);
            // Both threads wait here, so the writer's first push and the reader's arrival happen as
            // close together as two threads can manage. Without it the writer has finished long
            // before the reader starts and there is no interleaving to get wrong.
            let gate = Arc::new(std::sync::Barrier::new(2));
            let writer = std::thread::spawn({
                let tap = tap.clone();
                let gate = Arc::clone(&gate);
                move || {
                    gate.wait();
                    for index in 0..64 {
                        tap.push(record(&index.to_string()));
                    }
                }
            });

            gate.wait();
            let (tail, mut reader) = tap.tail_and_subscribe();
            writer.join().expect("the writer finishes");

            let mut seen: Vec<u64> = tail.iter().map(|record| record.seq).collect();
            let last = tail.last().map_or(0, |record| record.seq);
            while let Ok(record) = reader.try_recv() {
                if record.seq > last {
                    seen.push(record.seq);
                }
            }

            // Every sequence number from the first to the last, with none missing in the middle.
            // A gap here is a line the machine said that no reader will ever be shown.
            let expected: Vec<u64> = (1..=64).collect();
            assert_eq!(seen, expected, "round {round} lost or repeated a record");
        }
    }

    /// Nobody listening is the resting state of every one of these programs, and it must cost
    /// nothing and say nothing. A plain test and not a `tokio::test`, deliberately: this is what
    /// pins the property that a tap works before a runtime exists.
    #[test]
    fn a_tap_works_with_no_readers_and_no_runtime() {
        let tap = LogTap::new();
        assert_eq!(tap.reader_count(), 0);
        tap.push(record("into the void"));
        assert_eq!(tap.tail().len(), 1);
    }

    /// A record is taken apart rather than formatted, which is the whole reason this is a `Layer`.
    #[test]
    fn an_event_arrives_in_its_parts() {
        let tap = LogTap::new();
        with_tap(&tap, || {
            tracing::warn!(port = 8377, name = "living room", "could not bind");
        });

        let tail = tap.tail();
        let record = tail.first().expect("one record");
        assert_eq!(record.level, tracing::Level::WARN);
        assert_eq!(record.target, "km_logtap::tests");
        assert_eq!(record.message, "could not bind");
        assert!(record.fields.contains("port=8377"), "{}", record.fields);
        assert!(
            record.fields.contains(r#"name="living room""#),
            "{}",
            record.fields
        );
    }

    /// An event with nothing but a message leaves the fields empty rather than holding a stray
    /// separator, which is what a surface printing the two back to back depends on.
    #[test]
    fn an_event_with_no_fields_carries_no_fields() {
        let tap = LogTap::new();
        with_tap(&tap, || tracing::info!("nothing else to say"));

        let tail = tap.tail();
        let record = tail.first().expect("one record");
        assert_eq!(record.message, "nothing else to say");
        assert!(record.fields.is_empty(), "{}", record.fields);
    }

    /// A bounded ring of unbounded records is not bounded, and a cut in the middle of a character
    /// is what turns a shortened line into replacement characters on the screen.
    #[test]
    fn a_long_value_is_shortened_on_a_character_boundary() {
        let tap = LogTap::new();
        // Three bytes each, so the limit lands mid-character unless something is watching for it.
        tap.push(record(&"é".repeat(MAX_MESSAGE_BYTES)));

        let tail = tap.tail();
        let message = &tail.first().expect("one record").message;
        assert!(message.len() <= MAX_MESSAGE_BYTES + ELLIPSIS.len_utf8());
        assert!(message.ends_with(ELLIPSIS));
        assert!(
            message.chars().all(|c| c == 'é' || c == ELLIPSIS),
            "the cut landed inside a character"
        );
    }

    /// The directive travels with the tap, because the tap cannot see the filter beside it and a
    /// pane with nothing in it needs to say which of the two reasons that is.
    #[test]
    fn a_tap_carries_the_filter_it_was_told_about() {
        assert_eq!(LogTap::new().filter(), None);
        assert_eq!(
            LogTap::new().with_filter("info,km_app=debug").filter(),
            Some("info,km_app=debug")
        );
    }

    /// The stamp is what a surface turns into a time, so it has to be a real one rather than zero.
    #[test]
    fn a_record_is_stamped_with_a_real_moment() {
        // 2020-01-01, comfortably before anything that can run this and comfortably after the epoch.
        assert!(now_ms() > 1_577_836_800_000);
    }

    /// A tap can hold anything the program was told, so printing one must never print the contents.
    #[test]
    fn debugging_a_tap_does_not_print_what_is_in_it() {
        let tap = LogTap::new();
        tap.push(record("the admin password is hunter2"));
        let shown = format!("{tap:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("held: 1"), "{shown}");
    }
}
