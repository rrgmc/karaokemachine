//! The evening: the queue, the singers, their requests, and the rule that joins the three.
//!
//! **The rule is one sentence: the queue holds one song for each singer.** A singer's song is a
//! *request*. The machine accepts it at once when that singer has no entry in the queue, and holds
//! it otherwise. When a singer's entry leaves the queue, that singer's oldest waiting request goes
//! in at the end. So the singers take turns, and nobody is in charge.
//!
//! **Every change to the queue goes through [`Evening`], and each one ends in [`Evening::settle`].**
//! That is what the type is for. The rule reads the queue and the request list together, so a
//! change made to one behind the other's back would leave a singer with two songs or with none.
//! An entry leaves when its song starts, when somebody removes it and when its singer withdraws
//! it, and the rule treats all three alike.
//!
//! **The rule places a song once and never moves an entry afterwards.** An accepted song goes to
//! the end, so no entry moves back for it. Whatever somebody moves by hand stays where they put it.
//!
//! **A song with no singer passes the rule.** It goes straight to the end of the queue and holds
//! back no other song, as [`Evening::add`] does it.
//!
//! Pure state, as [`crate::queue`] and [`crate::seats`] are. The caller hands in the time, so the
//! rule about how long a finished request stays is testable without waiting.
//!
//! **A store may keep an evening.** [`Evening::snapshot`] hands out everything an evening holds,
//! and [`Evening::restore`] builds the evening again from it. The machine does that across a
//! restart when its owner asks for it, and this module opens no file.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use km_songcode::SongCode;
use serde::{Deserialize, Serialize};

use crate::queue::{MAX_QUEUED, Queue, QueueEntry, QueueFull, QueueRequest};
use crate::seats::{Issued, MAX_SEATS, Seat, SeatError, SeatName, SeatRecord, SeatRules, Seats};

/// How many requests the list may hold, finished ones included.
///
/// A cap exists so a phone at the room level cannot fill memory by asking for songs.
pub const MAX_REQUESTS: usize = 1_000;

/// How long a finished request stays in the list.
///
/// Long enough for the phone that asked to read what became of it. A request that stayed for ever
/// would fill the cap with songs already sung.
pub const FINISHED_FOR: Duration = Duration::from_secs(10 * 60);

/// Where a request stands, from the asking to the singing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    /// The singer already has a song in the queue, or the queue is full.
    Waiting,
    /// Accepted. [`Request::entry`] names its queue entry.
    Queued,
    /// The song started.
    Played,
    /// Somebody at the control level took it out.
    Declined,
    /// Its singer took it back.
    Withdrawn,
}

impl RequestState {
    /// Whether nothing further happens to a request in this state.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Played | Self::Declined | Self::Withdrawn)
    }
}

/// One song a singer asked for, through its whole course.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Opaque, stable for the evening. Never reused, and it orders requests by age.
    pub id: u64,
    /// The seat of the singer who asked.
    pub seat: u64,
    /// The catalog code.
    pub number: SongCode,
    /// Title, so a list needs no catalog.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// Where it stands.
    pub state: RequestState,
    /// The queue entry it became, from the moment the machine accepted it.
    ///
    /// Kept after the entry is gone, so one record carries the whole course of a song.
    pub entry: Option<u64>,
}

/// The song a request names, as the catalog describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedSong {
    /// The catalog code.
    pub number: SongCode,
    /// Title.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// Length in milliseconds.
    pub duration_ms: u32,
}

/// What the owner's settings ask of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestRules {
    /// How many requests one singer may have waiting.
    pub max_waiting: u32,
}

impl Default for RequestRules {
    /// Ten, which is more turns than one singer gets in an evening of several.
    fn default() -> Self {
        Self { max_waiting: 10 }
    }
}

/// A song that started this evening, for the record.
///
/// The rule needs no record, and reads none. The record is for a warning on a song somebody
/// already sang, and for a count of how often each singer has sung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sung {
    /// The catalog code.
    pub number: SongCode,
    /// The seat of the singer, or `None` for a song with no singer.
    pub seat: Option<u64>,
}

/// Who is asking, and how they prove it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proof<'a> {
    /// The seat's own token.
    Token(&'a str),
    /// A caller whose level is the proof, acting for any singer.
    Level,
}

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// No seat has that id.
    #[error("no such singer")]
    NoSinger,
    /// The token does not prove the seat.
    #[error("that token does not belong to this singer")]
    WrongToken,
    /// The singer has as many requests waiting as the owner allows.
    #[error("this singer has as many songs waiting as the machine allows")]
    SingerFull,
    /// The list holds as many requests as it may.
    #[error("the machine holds as many requests as it can ({MAX_REQUESTS})")]
    Full,
    /// No request has that id.
    #[error("no such request")]
    NotFound,
    /// The request has already finished.
    #[error("that request has already finished")]
    Finished,
    /// The machine is taking no new requests from a phone.
    #[error("the machine is taking no more requests tonight")]
    Closed,
    /// The request is not waiting, so it cannot be accepted early.
    #[error("that request is not waiting")]
    NotWaiting,
    /// The queue is at its limit, so nothing can be accepted into it.
    #[error("the queue is full ({MAX_QUEUED} songs)")]
    QueueFull,
}

/// Everything an evening holds, as a store keeps it.
///
/// It holds no time. A finished request carries its state alone, and [`Evening::restore`] starts
/// its [`FINISHED_FOR`] again. So two snapshots of an unchanged evening are equal, and a store
/// compares them to learn whether it has anything to write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EveningSnapshot {
    /// The waiting songs, in play order.
    pub queue: Vec<QueueEntry>,
    /// The id the next queue entry takes.
    pub queue_next: u64,
    /// The seats, in the order each registered.
    pub seats: Vec<SeatRecord>,
    /// The id the next seat takes.
    pub seats_next: u64,
    /// The number the last unnamed singer took.
    pub guests_next: u32,
    /// The requests, oldest first.
    pub requests: Vec<Request>,
    /// The id the next request takes.
    pub requests_next: u64,
    /// Every song that started, in order.
    pub sung: Vec<Sung>,
    /// Whether a phone may ask for a song.
    pub accepting: bool,
    /// The entry of the song that is playing, when it came from the queue.
    ///
    /// [`Evening::restore`] puts it back at the front, so a singer loses no turn to a restart.
    pub playing: Option<QueueEntry>,
}

/// Why a snapshot was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RestoreError {
    /// Two records of one kind carry the same id, so an id would name two things.
    #[error("two {0} carry the same id")]
    DuplicateId(&'static str),
}

/// A request as the list holds it.
#[derive(Debug, Clone)]
struct Held {
    request: Request,
    /// When it finished, which starts [`FINISHED_FOR`].
    finished_at: Option<Instant>,
}

/// The order a singer's request takes into the queue.
///
/// **A closed set the owner chooses from**, so another order is one more variant and one more
/// branch in [`Evening::settle`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueOrder {
    /// Every song goes to the end of the queue as it is asked for, named or not.
    ///
    /// **The default**, because a home queues songs one after another and has no turns to keep.
    #[default]
    Arrival,
    /// One song for each singer is in the queue, and the singer's next one waits until it starts.
    ///
    /// A venue chooses it, so one table cannot fill the queue.
    Turns,
}

/// The queue, the singers and their requests.
#[derive(Debug, Clone)]
pub struct Evening {
    queue: Queue,
    seats: Seats,
    requests: Vec<Held>,
    next_request: u64,
    /// Every song that started, in order.
    sung: Vec<Sung>,
    /// Whether a phone may ask for a song.
    accepting: bool,
    /// The order a request takes into the queue.
    order: QueueOrder,
}

impl Default for Evening {
    fn default() -> Self {
        Self {
            queue: Queue::default(),
            seats: Seats::default(),
            requests: Vec::new(),
            next_request: 0,
            sung: Vec::new(),
            accepting: true,
            order: QueueOrder::default(),
        }
    }
}

impl Evening {
    /// An evening with nobody in it, taking requests, in arrival order.
    pub fn new() -> Self {
        Self::default()
    }

    /// An evening with nobody in it, taking requests in `order`.
    pub fn with_order(order: QueueOrder) -> Self {
        Self {
            order,
            ..Self::default()
        }
    }

    /// The order a request takes into the queue.
    pub fn order(&self) -> QueueOrder {
        self.order
    }

    /// Changes the order for the rest of the evening.
    ///
    /// **A change to arrival order empties the waiting list into the queue**, in the order the
    /// songs were asked for, because arrival order has no turn for them to wait for. A change to
    /// turns moves nothing already queued.
    pub fn set_order(&mut self, order: QueueOrder, now: Instant) {
        self.order = order;
        self.settle(now);
    }

    /// Every song that started this evening, in order.
    pub fn sung(&self) -> &[Sung] {
        &self.sung
    }

    /// Whether a phone may ask for a song.
    pub fn accepting(&self) -> bool {
        self.accepting
    }

    /// Opens or closes the evening to new requests from a phone.
    ///
    /// **A request that already waits still enters the queue in its turn**, and a caller whose
    /// level is the proof still adds one. The switch closes the phones and nothing else.
    pub fn set_accepting(&mut self, accepting: bool) {
        self.accepting = accepting;
    }

    /// Starts a new evening without a restart.
    ///
    /// The queue, the requests, the record and every seat go, and the evening takes requests again.
    /// The song that is playing is no part of this, so it plays on. Ids go on counting, so nothing
    /// a phone kept names anything of the new evening.
    pub fn reset(&mut self) {
        self.queue.clear();
        self.requests.clear();
        self.sung.clear();
        self.seats.clear();
        self.accepting = true;
    }

    /// Everything the evening holds, for a store to keep.
    ///
    /// `playing` is the entry of the song that is playing, when it came from the queue. A finished
    /// request whose [`FINISHED_FOR`] has passed is left out.
    pub fn snapshot(&self, playing: Option<QueueEntry>, now: Instant) -> EveningSnapshot {
        EveningSnapshot {
            queue: self.queue.entries().cloned().collect(),
            queue_next: self.queue.next_id(),
            seats: self.seats.records().to_vec(),
            seats_next: self.seats.next_id(),
            guests_next: self.seats.next_guest(),
            requests: self.requests(now).cloned().collect(),
            requests_next: self.next_request,
            sung: self.sung.clone(),
            accepting: self.accepting,
            playing,
        }
    }

    /// Builds the evening a snapshot describes.
    ///
    /// **The song that was playing goes back to the front.** [`Self::pop`] recorded it as sung and
    /// its request as played, and both are put back, because the song starts again.
    ///
    /// **No id is issued twice.** Each counter ends past every id the snapshot holds, whatever the
    /// snapshot says the counter was. A snapshot in which two records share an id is refused.
    ///
    /// The lists are cut to their caps. A queued request whose entry is absent waits again, and the
    /// rule then runs. A finished request stays for [`FINISHED_FOR`] from `now`.
    ///
    /// **`order` comes from the settings and not from the snapshot**, and the rule runs in it. A
    /// venue's waiting songs therefore stay waiting across a restart.
    pub fn restore(
        snapshot: EveningSnapshot,
        order: QueueOrder,
        now: Instant,
    ) -> Result<Self, RestoreError> {
        let EveningSnapshot {
            mut queue,
            queue_next,
            mut seats,
            seats_next,
            guests_next,
            mut requests,
            requests_next,
            mut sung,
            accepting,
            playing,
        } = snapshot;

        if let Some(playing) = playing {
            let record = Sung {
                number: playing.number,
                seat: playing.seat,
            };
            if sung.last() == Some(&record) {
                sung.pop();
            }
            for request in &mut requests {
                if request.entry == Some(playing.id) && request.state == RequestState::Played {
                    request.state = RequestState::Queued;
                }
            }
            queue.insert(0, playing);
        }
        queue.truncate(MAX_QUEUED);
        seats.truncate(MAX_SEATS);
        requests.truncate(MAX_REQUESTS);

        distinct("queue entries", queue.iter().map(|entry| entry.id))?;
        distinct("seats", seats.iter().map(|held| held.seat.id))?;
        distinct("requests", requests.iter().map(|request| request.id))?;

        let queued: HashSet<u64> = queue.iter().map(|entry| entry.id).collect();
        let past_every_request = requests
            .iter()
            .map(|request| request.id + 1)
            .max()
            .unwrap_or(0);
        let requests = requests
            .into_iter()
            .map(|mut request| {
                let in_queue = request.entry.is_some_and(|entry| queued.contains(&entry));
                if request.state == RequestState::Queued && !in_queue {
                    request.state = RequestState::Waiting;
                    request.entry = None;
                }
                let finished_at = request.state.is_finished().then_some(now);
                Held {
                    request,
                    finished_at,
                }
            })
            .collect();

        let mut evening = Self {
            queue: Queue::from_parts(queue, queue_next),
            seats: Seats::from_records(seats, seats_next, guests_next),
            requests,
            next_request: requests_next.max(past_every_request),
            sung,
            accepting,
            order,
        };
        evening.settle(now);
        Ok(evening)
    }

    /// The waiting songs.
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// The singers.
    pub fn seats(&self) -> &Seats {
        &self.seats
    }

    /// Every request the list holds, oldest first.
    ///
    /// A finished one is left out once [`FINISHED_FOR`] has passed, whether or not anything has
    /// swept it yet.
    pub fn requests(&self, now: Instant) -> impl Iterator<Item = &Request> {
        self.requests
            .iter()
            .filter(move |held| !expired(held, now))
            .map(|held| &held.request)
    }

    // -- the queue -------------------------------------------------------------------------------

    /// Adds a song with no singer to the end of the queue.
    ///
    /// It passes the rule, so it holds back no other song.
    pub fn add(&mut self, request: QueueRequest) -> Result<u64, QueueFull> {
        self.queue.add(request)
    }

    /// Takes the next song off the front, because it is starting.
    ///
    /// Its request reads [`RequestState::Played`], and its singer's next request goes in.
    pub fn pop(&mut self, now: Instant) -> Option<QueueEntry> {
        let entry = self.queue.pop()?;
        self.sung.push(Sung {
            number: entry.number,
            seat: entry.seat,
        });
        self.entry_left(entry.id, RequestState::Played, now);
        self.settle(now);
        Some(entry)
    }

    /// Removes an entry, for a caller at the control level.
    ///
    /// Its request reads [`RequestState::Declined`], and its singer's next request goes in.
    pub fn remove(&mut self, id: u64, now: Instant) -> Option<QueueEntry> {
        let entry = self.queue.remove(id)?;
        self.entry_left(entry.id, RequestState::Declined, now);
        self.settle(now);
        Some(entry)
    }

    /// Moves an entry to a position. The rule never moves it again.
    pub fn move_to(&mut self, id: u64, index: usize) -> bool {
        self.queue.move_to(id, index)
    }

    /// Removes every entry, for a caller at the control level.
    ///
    /// Each one's request reads [`RequestState::Declined`]. The rule then runs, so each singer's
    /// next waiting request goes in.
    pub fn clear(&mut self, now: Instant) -> Vec<QueueEntry> {
        let removed = self.queue.clear();
        for entry in &removed {
            self.entry_left(entry.id, RequestState::Declined, now);
        }
        self.settle(now);
        removed
    }

    // -- the singers -----------------------------------------------------------------------------

    /// Registers a singer. See [`Seats::register`].
    ///
    /// A seat holds a song while one of its songs is queued or waiting.
    pub fn register(
        &mut self,
        device: &str,
        name: Option<SeatName>,
        token: String,
        rules: SeatRules,
    ) -> Result<Issued, SeatError> {
        let Self {
            queue,
            seats,
            requests,
            ..
        } = self;
        seats.register(device, name, token, rules, |seat| {
            queue.holds_seat(seat)
                || requests.iter().any(|held| {
                    held.request.seat == seat && held.request.state == RequestState::Waiting
                })
        })
    }

    /// Gives a seat a name, or another name. See [`Seats::rename`].
    ///
    /// The seat's queued songs take the name too, because the screen draws it from the entry.
    pub fn rename(&mut self, id: u64, token: &str, name: SeatName) -> Result<Seat, SeatError> {
        let seat = self.seats.rename(id, token, name)?;
        self.queue.relabel_seat(seat.id, seat.name.as_deref());
        Ok(seat)
    }

    /// Opens a seat for another phone to take. See [`Seats::open`].
    pub fn open_seat(&mut self, id: u64) -> Result<Seat, SeatError> {
        self.seats.open(id)
    }

    /// Creates a singer who has no phone. See [`Seats::create`].
    pub fn create_singer(&mut self, name: Option<SeatName>) -> Result<Seat, SeatError> {
        self.seats.create(name)
    }

    /// Removes a singer who went home: the seat, its queued songs and its waiting ones.
    ///
    /// Each of its requests reads [`RequestState::Declined`]. The rule then runs, because the
    /// entries that left made room.
    pub fn remove_singer(&mut self, id: u64, now: Instant) -> Result<Seat, SeatError> {
        let seat = self.seats.remove(id).ok_or(SeatError::NotFound)?;
        self.queue.remove_seat(id);
        for held in &mut self.requests {
            if held.request.seat == id && !held.request.state.is_finished() {
                held.request.state = RequestState::Declined;
                held.finished_at = Some(now);
            }
        }
        self.settle(now);
        Ok(seat)
    }

    /// Joins two seats that are one person: `from` goes, and `into` takes what it held.
    ///
    /// The requests, the queue entries and the record all move. The rule may then find two entries
    /// for the one singer. It leaves both, and accepts no third while either is there.
    pub fn merge(&mut self, from: u64, into: u64) -> Result<Seat, SeatError> {
        if from == into {
            return Err(SeatError::NotFound);
        }
        let kept = self.seats.get(into).cloned().ok_or(SeatError::NotFound)?;
        self.seats.remove(from).ok_or(SeatError::NotFound)?;
        self.queue.reseat(from, into, kept.name.as_deref());
        for held in &mut self.requests {
            if held.request.seat == from {
                held.request.seat = into;
            }
        }
        for sung in &mut self.sung {
            if sung.seat == Some(from) {
                sung.seat = Some(into);
            }
        }
        Ok(kept)
    }

    // -- the requests ----------------------------------------------------------------------------

    /// Takes a singer's request, and accepts it at once when the rule allows.
    ///
    /// The answer carries the state it reached: [`RequestState::Queued`] with its entry, or
    /// [`RequestState::Waiting`].
    pub fn request(
        &mut self,
        seat: u64,
        proof: Proof<'_>,
        song: RequestedSong,
        rules: RequestRules,
        now: Instant,
    ) -> Result<Request, RequestError> {
        self.sweep(now);
        if self.seats.get(seat).is_none() {
            return Err(RequestError::NoSinger);
        }
        if let Proof::Token(token) = proof {
            if !self.seats.holds(seat, token) {
                return Err(RequestError::WrongToken);
            }
            if !self.accepting {
                return Err(RequestError::Closed);
            }
        }
        if self.requests.len() >= MAX_REQUESTS {
            return Err(RequestError::Full);
        }
        let waiting = self
            .requests
            .iter()
            .filter(|held| held.request.seat == seat && held.request.state == RequestState::Waiting)
            .count();
        // The cap is on songs that wait. A song the rule accepts at once waits for nothing, so a
        // cap of zero still gives each singer one song in the queue.
        let would_wait = waiting > 0
            || (self.order == QueueOrder::Turns && self.queue.holds_seat(seat))
            || self.queue.len() >= MAX_QUEUED;
        if would_wait && waiting >= rules.max_waiting as usize {
            return Err(RequestError::SingerFull);
        }

        let id = self.next_request;
        self.next_request += 1;
        self.requests.push(Held {
            request: Request {
                id,
                seat,
                number: song.number,
                title: song.title,
                artist: song.artist,
                duration_ms: song.duration_ms,
                state: RequestState::Waiting,
                entry: None,
            },
            finished_at: None,
        });
        self.settle(now);
        Ok(self.held(id).expect("just added").request.clone())
    }

    /// Takes a request back, for its singer. The seat's token is the proof.
    ///
    /// The same token takes the song out of the queue after the machine accepted it, so a guest who
    /// asked for the wrong song needs no control code.
    pub fn withdraw(
        &mut self,
        id: u64,
        token: &str,
        now: Instant,
    ) -> Result<Request, RequestError> {
        self.sweep(now);
        let held = self.held(id).ok_or(RequestError::NotFound)?;
        let (seat, state, entry) = (held.request.seat, held.request.state, held.request.entry);
        if !self.seats.holds(seat, token) {
            return Err(RequestError::WrongToken);
        }
        if state.is_finished() {
            return Err(RequestError::Finished);
        }
        if let Some(entry) = entry {
            self.queue.remove(entry);
        }
        self.finish(id, RequestState::Withdrawn, now);
        self.settle(now);
        Ok(self.held(id).expect("still held").request.clone())
    }

    /// Refuses a request, for a caller at the control level. Waiting or queued, it reads
    /// [`RequestState::Declined`], and a queued one leaves the queue.
    pub fn decline(&mut self, id: u64, now: Instant) -> Result<Request, RequestError> {
        self.sweep(now);
        let held = self.held(id).ok_or(RequestError::NotFound)?;
        let (state, entry) = (held.request.state, held.request.entry);
        if state.is_finished() {
            return Err(RequestError::Finished);
        }
        if let Some(entry) = entry {
            self.queue.remove(entry);
        }
        self.finish(id, RequestState::Declined, now);
        self.settle(now);
        Ok(self.held(id).expect("still held").request.clone())
    }

    /// Accepts a waiting request early, for a caller at the control level.
    ///
    /// Its singer then has a second song in the queue, at the end or next. That makes a second
    /// round, so a new singer's first song can follow this singer's second. The rule leaves both
    /// entries where they are, and accepts no third while either is there.
    pub fn accept(&mut self, id: u64, next: bool, now: Instant) -> Result<Request, RequestError> {
        self.sweep(now);
        let index = self
            .requests
            .iter()
            .position(|held| held.request.id == id)
            .ok_or(RequestError::NotFound)?;
        let request = &self.requests[index].request;
        match request.state {
            RequestState::Waiting => {}
            state if state.is_finished() => return Err(RequestError::Finished),
            _ => return Err(RequestError::NotWaiting),
        }
        let entry = self
            .queue
            .add(QueueRequest {
                number: request.number,
                title: request.title.clone(),
                artist: request.artist.clone(),
                singer: self
                    .seats
                    .get(request.seat)
                    .and_then(|seat| seat.name.clone()),
                duration_ms: request.duration_ms,
                seat: Some(request.seat),
            })
            .map_err(|QueueFull::Full| RequestError::QueueFull)?;
        if next {
            self.queue.move_to(entry, 0);
        }
        let request = &mut self.requests[index].request;
        request.state = RequestState::Queued;
        request.entry = Some(entry);
        Ok(request.clone())
    }

    /// Applies the rule: accepts every waiting request whose singer has no entry in the queue, or,
    /// in arrival order, every waiting request.
    ///
    /// Oldest first, so the request that has waited longest takes the room there is. A full queue
    /// accepts nothing, and its requests wait until an entry leaves.
    fn settle(&mut self, now: Instant) {
        self.sweep(now);
        for index in 0..self.requests.len() {
            let request = &self.requests[index].request;
            // In turns a singer's song waits while another of theirs is queued. In arrival order
            // nothing waits but for room in the queue.
            let turn_taken = self.order == QueueOrder::Turns && self.queue.holds_seat(request.seat);
            if request.state != RequestState::Waiting || turn_taken {
                continue;
            }
            if self.queue.len() >= MAX_QUEUED {
                return;
            }
            let singer = self
                .seats
                .get(request.seat)
                .and_then(|seat| seat.name.clone());
            let entry = self.queue.add(QueueRequest {
                number: request.number,
                title: request.title.clone(),
                artist: request.artist.clone(),
                singer,
                duration_ms: request.duration_ms,
                seat: Some(request.seat),
            });
            let Ok(entry) = entry else {
                return;
            };
            let request = &mut self.requests[index].request;
            request.state = RequestState::Queued;
            request.entry = Some(entry);
        }
    }

    /// Records that a queue entry left, on the request it came from. A song with no singer has
    /// none.
    fn entry_left(&mut self, entry: u64, state: RequestState, now: Instant) {
        let id = self
            .requests
            .iter()
            .find(|held| {
                held.request.entry == Some(entry) && held.request.state == RequestState::Queued
            })
            .map(|held| held.request.id);
        if let Some(id) = id {
            self.finish(id, state, now);
        }
    }

    fn finish(&mut self, id: u64, state: RequestState, now: Instant) {
        if let Some(held) = self.requests.iter_mut().find(|held| held.request.id == id) {
            held.request.state = state;
            held.finished_at = Some(now);
        }
    }

    fn held(&self, id: u64) -> Option<&Held> {
        self.requests.iter().find(|held| held.request.id == id)
    }

    /// Drops every finished request whose time has passed.
    fn sweep(&mut self, now: Instant) {
        self.requests.retain(|held| !expired(held, now));
    }
}

/// Refuses a list in which two records share an id.
fn distinct(what: &'static str, ids: impl Iterator<Item = u64>) -> Result<(), RestoreError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(RestoreError::DuplicateId(what));
        }
    }
    Ok(())
}

/// Whether a finished request has stayed its [`FINISHED_FOR`].
fn expired(held: &Held, now: Instant) -> bool {
    held.finished_at
        .is_some_and(|at| now.saturating_duration_since(at) >= FINISHED_FOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A restart of a venue's evening, which keeps turns.
    fn restore_turns(snapshot: EveningSnapshot, now: Instant) -> Result<Evening, RestoreError> {
        Evening::restore(snapshot, QueueOrder::Turns, now)
    }

    /// A phone with one singer on it.
    struct Singer {
        seat: u64,
        token: String,
    }

    fn singer(evening: &mut Evening, name: &str) -> Singer {
        let issued = evening
            .register(
                &format!("device-{name}"),
                Some(SeatName {
                    shown: name.to_owned(),
                    key: name.to_lowercase(),
                }),
                format!("token-{name}"),
                SeatRules::default(),
            )
            .expect("the seat registers");
        Singer {
            seat: issued.seat.id,
            token: issued.token,
        }
    }

    fn song(number: u32) -> RequestedSong {
        RequestedSong {
            number: SongCode::new(number),
            title: format!("Song {number}"),
            artist: None,
            duration_ms: 180_000,
        }
    }

    fn ask(evening: &mut Evening, who: &Singer, number: u32, now: Instant) -> Request {
        evening
            .request(
                who.seat,
                Proof::Token(&who.token),
                song(number),
                RequestRules::default(),
                now,
            )
            .expect("the request is taken")
    }

    /// The queue as song numbers, in play order.
    fn numbers(evening: &Evening) -> Vec<u32> {
        evening
            .queue()
            .entries()
            .map(|entry| entry.number.number())
            .collect()
    }

    fn state_of(evening: &Evening, id: u64, now: Instant) -> RequestState {
        evening
            .requests(now)
            .find(|request| request.id == id)
            .expect("the request is listed")
            .state
    }

    /// Arrival order queues every song as it is asked for, whoever asks and however many.
    #[test]
    fn arrival_order_queues_every_song_in_the_order_asked() {
        let now = Instant::now();
        let mut evening = Evening::new();
        assert_eq!(evening.order(), QueueOrder::Arrival, "the default");
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");

        for number in [1, 2, 3] {
            assert_eq!(
                ask(&mut evening, &ana, number, now).state,
                RequestState::Queued
            );
        }
        ask(&mut evening, &beto, 4, now);
        for number in [5, 6, 7, 8, 9, 10, 11, 12, 13, 14] {
            ask(&mut evening, &ana, number, now);
        }

        assert_eq!(
            numbers(&evening),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
            "no turns: ten of one singer's songs stand in a row"
        );
    }

    /// A singer with no name queues the same way, because the order does not depend on a name.
    #[test]
    fn arrival_order_takes_an_unnamed_singer_the_same_way() {
        let now = Instant::now();
        let mut evening = Evening::new();
        let issued = evening
            .register("device-x", None, "token-x".to_owned(), SeatRules::default())
            .expect("an unnamed singer");
        let guest = Singer {
            seat: issued.seat.id,
            token: "token-x".to_owned(),
        };
        ask(&mut evening, &guest, 1, now);
        ask(&mut evening, &guest, 2, now);
        assert_eq!(numbers(&evening), vec![1, 2]);
    }

    /// Changing to arrival order mid-evening moves every waiting song into the queue, oldest first.
    #[test]
    fn a_change_to_arrival_order_empties_the_waiting_list_into_the_queue() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        ask(&mut evening, &ana, 1, now);
        ask(&mut evening, &ana, 2, now);
        ask(&mut evening, &beto, 3, now);
        ask(&mut evening, &ana, 4, now);
        assert_eq!(numbers(&evening), vec![1, 3], "turns keep 2 and 4 waiting");

        evening.set_order(QueueOrder::Arrival, now);
        assert_eq!(numbers(&evening), vec![1, 3, 2, 4]);
    }

    /// A restart keeps the order the settings give, so a venue's waiting songs stay waiting.
    #[test]
    fn a_restart_applies_the_order_it_is_given() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 1, now);
        ask(&mut evening, &ana, 2, now);
        let snapshot = evening.snapshot(None, now);

        let turns = Evening::restore(snapshot.clone(), QueueOrder::Turns, now).expect("restored");
        assert_eq!(numbers(&turns), vec![1]);
        let arrival = Evening::restore(snapshot, QueueOrder::Arrival, now).expect("restored");
        assert_eq!(numbers(&arrival), vec![1, 2]);
    }

    #[test]
    fn a_first_request_goes_straight_into_the_queue() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");

        let request = ask(&mut evening, &ana, 1, now);

        assert_eq!(request.state, RequestState::Queued);
        let entry = evening.queue().peek().expect("a queue entry");
        assert_eq!(request.entry, Some(entry.id));
        assert_eq!(entry.seat, Some(ana.seat));
        assert_eq!(entry.singer.as_deref(), Some("Ana"));
    }

    #[test]
    fn a_second_request_waits_while_the_first_is_queued() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 1, now);

        let second = ask(&mut evening, &ana, 2, now);

        assert_eq!(second.state, RequestState::Waiting);
        assert_eq!(second.entry, None);
        assert_eq!(numbers(&evening), vec![1]);
    }

    /// Three singers with three requests each sing in turn, whatever order the requests came in.
    #[test]
    fn the_singers_take_turns() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        let caio = singer(&mut evening, "Caio");
        // Ana asks for all of hers before anybody else asks for one.
        for number in [11, 12, 13] {
            ask(&mut evening, &ana, number, now);
        }
        for number in [21, 22, 23] {
            ask(&mut evening, &beto, number, now);
        }
        for number in [31, 32, 33] {
            ask(&mut evening, &caio, number, now);
        }
        assert_eq!(numbers(&evening), vec![11, 21, 31], "one round");

        let mut sung = Vec::new();
        while let Some(entry) = evening.pop(now) {
            sung.push(entry.number.number());
        }
        assert_eq!(sung, vec![11, 21, 31, 12, 22, 32, 13, 23, 33]);
    }

    /// A singer's next request goes in when that singer's song starts, at the end of the queue.
    #[test]
    fn the_next_request_goes_in_when_the_song_starts_and_no_entry_moves_back() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        let first = ask(&mut evening, &ana, 11, now);
        let next = ask(&mut evening, &ana, 12, now);
        ask(&mut evening, &beto, 21, now);
        assert_eq!(numbers(&evening), vec![11, 21]);

        let started = evening.pop(now).expect("Ana's song starts");

        assert_eq!(started.number.number(), 11);
        assert_eq!(
            numbers(&evening),
            vec![21, 12],
            "behind the song already there"
        );
        assert_eq!(state_of(&evening, first.id, now), RequestState::Played);
        assert_eq!(state_of(&evening, next.id, now), RequestState::Queued);
    }

    #[test]
    fn a_new_singer_waits_one_round_at_most() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        for number in [11, 12, 13] {
            ask(&mut evening, &ana, number, now);
        }
        for number in [21, 22, 23] {
            ask(&mut evening, &beto, number, now);
        }
        evening.pop(now);

        let dani = singer(&mut evening, "Dani");
        ask(&mut evening, &dani, 41, now);

        assert_eq!(numbers(&evening), vec![21, 12, 41]);
    }

    /// A song with no singer goes straight to the end and holds back no other song.
    #[test]
    fn a_song_with_no_singer_passes_the_rule() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 11, now);
        for number in [901, 902] {
            evening
                .add(QueueRequest {
                    number: SongCode::new(number),
                    title: "Generic".to_owned(),
                    artist: None,
                    singer: None,
                    duration_ms: 0,
                    seat: None,
                })
                .expect("add");
        }
        assert_eq!(numbers(&evening), vec![11, 901, 902]);
        assert_eq!(evening.requests(now).count(), 1, "and it is no request");
    }

    #[test]
    fn withdrawing_a_waiting_request_needs_the_seats_token() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        ask(&mut evening, &ana, 11, now);
        let waiting = ask(&mut evening, &ana, 12, now);

        assert_eq!(
            evening.withdraw(waiting.id, &beto.token, now),
            Err(RequestError::WrongToken)
        );
        let withdrawn = evening
            .withdraw(waiting.id, &ana.token, now)
            .expect("her own request");
        assert_eq!(withdrawn.state, RequestState::Withdrawn);
        assert_eq!(numbers(&evening), vec![11]);

        assert_eq!(
            evening.withdraw(waiting.id, &ana.token, now),
            Err(RequestError::Finished)
        );
        assert_eq!(
            evening.withdraw(9_999, &ana.token, now),
            Err(RequestError::NotFound)
        );
    }

    /// The same token takes the song out of the queue, and the singer's next request takes its turn.
    #[test]
    fn withdrawing_a_queued_song_lets_the_next_one_in() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        let wrong = ask(&mut evening, &ana, 11, now);
        ask(&mut evening, &beto, 21, now);
        ask(&mut evening, &ana, 12, now);

        evening
            .withdraw(wrong.id, &ana.token, now)
            .expect("her own song");

        assert_eq!(numbers(&evening), vec![21, 12]);
        assert_eq!(state_of(&evening, wrong.id, now), RequestState::Withdrawn);
    }

    #[test]
    fn an_entry_somebody_removes_reads_declined_and_the_rule_runs() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let first = ask(&mut evening, &ana, 11, now);
        let second = ask(&mut evening, &ana, 12, now);

        evening
            .remove(first.entry.expect("queued"), now)
            .expect("the entry");

        assert_eq!(state_of(&evening, first.id, now), RequestState::Declined);
        assert_eq!(state_of(&evening, second.id, now), RequestState::Queued);
        assert_eq!(numbers(&evening), vec![12]);
    }

    #[test]
    fn clearing_the_queue_lets_each_singers_next_request_in() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        ask(&mut evening, &ana, 11, now);
        ask(&mut evening, &ana, 12, now);
        ask(&mut evening, &beto, 21, now);

        let removed = evening.clear(now);

        assert_eq!(removed.len(), 2);
        assert_eq!(numbers(&evening), vec![12], "Beto has nothing else waiting");
    }

    /// An entry moved by hand stays where it was put, and the rule adds behind it.
    #[test]
    fn the_rule_never_moves_an_entry_somebody_moved() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        ask(&mut evening, &ana, 11, now);
        let moved = ask(&mut evening, &beto, 21, now);
        ask(&mut evening, &beto, 22, now);

        assert!(evening.move_to(moved.entry.expect("queued"), 0));
        assert_eq!(numbers(&evening), vec![21, 11]);
        evening.pop(now);
        assert_eq!(numbers(&evening), vec![11, 22]);
    }

    #[test]
    fn a_singer_may_have_only_so_many_requests_waiting() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let rules = RequestRules { max_waiting: 2 };
        let mut ask =
            |number| evening.request(ana.seat, Proof::Token(&ana.token), song(number), rules, now);
        // One in the queue, and two waiting.
        for number in [11, 12, 13] {
            ask(number).expect("under the cap");
        }
        assert_eq!(ask(14), Err(RequestError::SingerFull));
    }

    /// A cap of zero is one song at a time: the song that goes straight in is never refused.
    #[test]
    fn a_cap_of_no_waiting_songs_still_takes_the_song_that_goes_straight_in() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let rules = RequestRules { max_waiting: 0 };
        let mut ask =
            |number| evening.request(ana.seat, Proof::Token(&ana.token), song(number), rules, now);

        let first = ask(11).expect("nothing of hers is queued, so it goes in");
        assert_eq!(first.state, RequestState::Queued);
        assert_eq!(
            ask(12),
            Err(RequestError::SingerFull),
            "the second would wait"
        );

        // Her song starts, and the next one goes straight in again.
        evening.pop(now);
        let next = evening
            .request(ana.seat, Proof::Token(&ana.token), song(12), rules, now)
            .expect("the queue holds none of hers");
        assert_eq!(next.state, RequestState::Queued);
    }

    #[test]
    fn the_whole_list_is_capped() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let rules = RequestRules {
            max_waiting: u32::MAX,
        };
        for number in 0..MAX_REQUESTS {
            evening
                .request(
                    ana.seat,
                    Proof::Token(&ana.token),
                    song(number as u32 + 1),
                    rules,
                    now,
                )
                .expect("under the cap");
        }
        let one_more = evening.request(ana.seat, Proof::Token(&ana.token), song(9_999), rules, now);
        assert_eq!(one_more, Err(RequestError::Full));
    }

    /// A full queue accepts nothing, and the oldest request takes the first room there is.
    #[test]
    fn a_full_queue_holds_every_request_until_an_entry_leaves() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        for number in 0..MAX_QUEUED {
            evening
                .add(QueueRequest {
                    number: SongCode::new(number as u32 + 1),
                    title: "Generic".to_owned(),
                    artist: None,
                    singer: None,
                    duration_ms: 0,
                    seat: None,
                })
                .expect("add");
        }
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        let first = ask(&mut evening, &ana, 5_001, now);
        let second = ask(&mut evening, &beto, 5_002, now);
        assert_eq!(first.state, RequestState::Waiting);
        assert_eq!(second.state, RequestState::Waiting);

        evening.pop(now);

        assert_eq!(state_of(&evening, first.id, now), RequestState::Queued);
        assert_eq!(state_of(&evening, second.id, now), RequestState::Waiting);
    }

    #[test]
    fn a_request_needs_a_seat_and_its_token_or_a_callers_level() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let rules = RequestRules::default();

        assert_eq!(
            evening.request(9_999, Proof::Level, song(1), rules, now),
            Err(RequestError::NoSinger)
        );
        assert_eq!(
            evening.request(ana.seat, Proof::Token("nope"), song(1), rules, now),
            Err(RequestError::WrongToken)
        );
        // An operator adds a song for any singer, and the rule treats it as a phone's.
        let first = evening
            .request(ana.seat, Proof::Level, song(1), rules, now)
            .expect("the level is the proof");
        let second = evening
            .request(ana.seat, Proof::Level, song(2), rules, now)
            .expect("the level is the proof");
        assert_eq!(first.state, RequestState::Queued);
        assert_eq!(second.state, RequestState::Waiting);
    }

    #[test]
    fn a_finished_request_leaves_the_list_after_a_fixed_time() {
        let start = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let played = ask(&mut evening, &ana, 11, start);
        let queued = ask(&mut evening, &ana, 12, start);
        evening.pop(start);

        let just_before = start + FINISHED_FOR - Duration::from_secs(1);
        assert_eq!(
            state_of(&evening, played.id, just_before),
            RequestState::Played
        );

        let after = start + FINISHED_FOR;
        let listed: Vec<u64> = evening.requests(after).map(|request| request.id).collect();
        assert_eq!(listed, vec![queued.id], "the one still in the queue stays");
    }

    /// A seat holds a song while one is queued or waiting, which is what holds its name.
    #[test]
    fn a_name_is_held_while_its_seat_has_a_request() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let request = ask(&mut evening, &ana, 11, now);
        let reclaim = |evening: &mut Evening| {
            evening.register(
                "another-device",
                Some(SeatName {
                    shown: "Ana".to_owned(),
                    key: "ana".to_owned(),
                }),
                "token-new".to_owned(),
                SeatRules::default(),
            )
        };

        assert_eq!(reclaim(&mut evening), Err(SeatError::NameTaken));
        evening
            .withdraw(request.id, &ana.token, now)
            .expect("her own song");
        assert!(
            reclaim(&mut evening).is_ok(),
            "nothing queued and nothing waiting"
        );
    }

    #[test]
    fn a_rename_reaches_the_seats_queued_songs() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 11, now);

        evening
            .rename(
                ana.seat,
                &ana.token,
                SeatName {
                    shown: "Ana Paula".to_owned(),
                    key: "ana paula".to_owned(),
                },
            )
            .expect("a free name");

        let entry = evening.queue().peek().expect("a queue entry");
        assert_eq!(entry.singer.as_deref(), Some("Ana Paula"));
    }

    /// A hand-written slip, entered by somebody at the control level.
    fn slip(evening: &mut Evening, who: &Singer, number: u32, now: Instant) -> Request {
        evening
            .request(
                who.seat,
                Proof::Level,
                song(number),
                RequestRules::default(),
                now,
            )
            .expect("the level is the proof")
    }

    #[test]
    fn a_declined_request_never_plays_and_the_next_one_takes_the_turn() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let queued = ask(&mut evening, &ana, 11, now);
        let unwelcome = ask(&mut evening, &ana, 12, now);
        let third = ask(&mut evening, &ana, 13, now);

        let declined = evening
            .decline(unwelcome.id, now)
            .expect("a waiting request");
        assert_eq!(declined.state, RequestState::Declined);
        assert_eq!(numbers(&evening), vec![11], "nothing in the queue moved");

        // A queued one leaves the queue, and her next waiting song goes in.
        evening.decline(queued.id, now).expect("a queued request");
        assert_eq!(numbers(&evening), vec![13]);
        assert_eq!(state_of(&evening, third.id, now), RequestState::Queued);

        assert_eq!(evening.decline(queued.id, now), Err(RequestError::Finished));
        assert_eq!(evening.decline(9_999, now), Err(RequestError::NotFound));
    }

    /// An early accept gives a singer a second song in the queue, at the end or next.
    #[test]
    fn accepting_early_gives_a_second_song_and_the_rule_adds_no_third() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        let first = ask(&mut evening, &ana, 11, now);
        ask(&mut evening, &beto, 21, now);
        let second = ask(&mut evening, &ana, 12, now);
        let third = ask(&mut evening, &ana, 13, now);

        let accepted = evening.accept(second.id, false, now).expect("it waits");
        assert_eq!(accepted.state, RequestState::Queued);
        assert_eq!(numbers(&evening), vec![11, 21, 12], "at the end");

        let next = evening.accept(third.id, true, now).expect("it waits");
        assert_eq!(numbers(&evening), vec![13, 11, 21, 12], "next");
        assert_eq!(next.entry, evening.queue().peek().map(|entry| entry.id));

        assert_eq!(
            evening.accept(first.id, false, now),
            Err(RequestError::NotWaiting),
            "a queued request has nothing to accept"
        );

        // One of hers starts, and she still has two there, so nothing more of hers goes in.
        let fourth = ask(&mut evening, &ana, 14, now);
        evening.pop(now);
        assert_eq!(state_of(&evening, fourth.id, now), RequestState::Waiting);
    }

    #[test]
    fn a_full_queue_refuses_an_early_accept() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 5_001, now);
        let waiting = ask(&mut evening, &ana, 5_002, now);
        for number in 0..MAX_QUEUED - 1 {
            evening
                .add(QueueRequest {
                    number: SongCode::new(number as u32 + 1),
                    title: "Generic".to_owned(),
                    artist: None,
                    singer: None,
                    duration_ms: 0,
                    seat: None,
                })
                .expect("add");
        }
        assert_eq!(
            evening.accept(waiting.id, false, now),
            Err(RequestError::QueueFull)
        );
        assert_eq!(state_of(&evening, waiting.id, now), RequestState::Waiting);
    }

    /// The switch closes the phones. What already waits goes in, and the host still adds.
    #[test]
    fn a_closed_evening_refuses_a_phone_and_keeps_what_waits() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 11, now);
        let waiting = ask(&mut evening, &ana, 12, now);
        assert!(evening.accepting());

        evening.set_accepting(false);
        let refused = evening.request(
            ana.seat,
            Proof::Token(&ana.token),
            song(13),
            RequestRules::default(),
            now,
        );
        assert_eq!(refused, Err(RequestError::Closed));

        evening.pop(now);
        assert_eq!(state_of(&evening, waiting.id, now), RequestState::Queued);
        let last = slip(&mut evening, &ana, 14, now);
        assert_eq!(
            last.state,
            RequestState::Waiting,
            "the host's song is taken"
        );

        evening.set_accepting(true);
        ask(&mut evening, &ana, 15, now);
    }

    /// A singer who went home: the seat goes, and so does every song of theirs.
    #[test]
    fn removing_a_singer_takes_their_songs_and_frees_their_name() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        let queued = ask(&mut evening, &ana, 11, now);
        let waiting = ask(&mut evening, &ana, 12, now);
        ask(&mut evening, &beto, 21, now);

        let gone = evening.remove_singer(ana.seat, now).expect("the seat");
        assert_eq!(gone.id, ana.seat);
        assert_eq!(numbers(&evening), vec![21]);
        assert_eq!(state_of(&evening, queued.id, now), RequestState::Declined);
        assert_eq!(state_of(&evening, waiting.id, now), RequestState::Declined);
        assert!(evening.seats().get(ana.seat).is_none());
        assert_eq!(
            evening.remove_singer(ana.seat, now),
            Err(SeatError::NotFound)
        );
        // The name is free for whoever comes next.
        singer(&mut evening, "Ana");
    }

    /// One person registered two names. The merge leaves one singer with both songs in the queue,
    /// and the rule accepts no third.
    #[test]
    fn merging_two_singers_moves_everything_to_one() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let dave = singer(&mut evening, "Dave");
        let david = singer(&mut evening, "David");
        ask(&mut evening, &dave, 11, now);
        ask(&mut evening, &david, 21, now);
        let waiting = ask(&mut evening, &dave, 12, now);

        let kept = evening.merge(dave.seat, david.seat).expect("both exist");
        assert_eq!(kept.id, david.seat);
        assert!(evening.seats().get(dave.seat).is_none());
        let seats: Vec<Option<u64>> = evening.queue().entries().map(|entry| entry.seat).collect();
        assert_eq!(seats, vec![Some(david.seat), Some(david.seat)]);
        let names: Vec<Option<&str>> = evening
            .queue()
            .entries()
            .map(|entry| entry.singer.as_deref())
            .collect();
        assert_eq!(names, vec![Some("David"), Some("David")]);

        evening.pop(now);
        assert_eq!(
            state_of(&evening, waiting.id, now),
            RequestState::Waiting,
            "one of his is still in the queue"
        );
        evening.pop(now);
        assert_eq!(state_of(&evening, waiting.id, now), RequestState::Queued);
        assert_eq!(
            evening.sung(),
            &[
                Sung {
                    number: SongCode::new(11),
                    seat: Some(david.seat)
                },
                Sung {
                    number: SongCode::new(21),
                    seat: Some(david.seat)
                },
            ]
        );
        assert_eq!(
            evening.merge(david.seat, david.seat),
            Err(SeatError::NotFound)
        );
        assert_eq!(evening.merge(9_999, david.seat), Err(SeatError::NotFound));
    }

    /// The record is every song that started, with its singer where it had one.
    #[test]
    fn the_record_holds_every_song_that_started() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 11, now);
        evening
            .add(QueueRequest {
                number: SongCode::new(901),
                title: "Generic".to_owned(),
                artist: None,
                singer: None,
                duration_ms: 0,
                seat: None,
            })
            .expect("add");
        assert!(evening.sung().is_empty());

        evening.pop(now);
        evening.pop(now);
        let sung: Vec<(u32, Option<u64>)> = evening
            .sung()
            .iter()
            .map(|sung| (sung.number.number(), sung.seat))
            .collect();
        assert_eq!(sung, vec![(11, Some(ana.seat)), (901, None)]);
    }

    /// A new evening starts as the machine starts, and takes requests again.
    #[test]
    fn a_new_evening_clears_everything_and_ids_go_on_counting() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let before = ask(&mut evening, &ana, 11, now);
        ask(&mut evening, &ana, 12, now);
        evening.pop(now);
        evening.set_accepting(false);

        evening.reset();

        assert!(evening.queue().is_empty());
        assert!(evening.seats().is_empty());
        assert_eq!(evening.requests(now).count(), 0);
        assert!(evening.sung().is_empty());
        assert!(evening.accepting());

        let again = singer(&mut evening, "Ana");
        assert!(again.seat > ana.seat, "a kept seat id names nobody now");
        let after = ask(&mut evening, &again, 11, now);
        assert!(after.id > before.id);
    }

    /// An operator's singer has no phone, and sings in turn like any other.
    #[test]
    fn an_operators_singer_takes_turns_with_no_phone() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let hosted = evening
            .create_singer(Some(SeatName {
                shown: "Caio".to_owned(),
                key: "caio".to_owned(),
            }))
            .expect("a free name");
        let caio = Singer {
            seat: hosted.id,
            token: String::new(),
        };
        assert_eq!(
            slip(&mut evening, &caio, 31, now).state,
            RequestState::Queued
        );
        assert_eq!(
            slip(&mut evening, &caio, 32, now).state,
            RequestState::Waiting
        );
        // No token proves the seat, so no phone asks for him.
        assert_eq!(
            evening.request(
                caio.seat,
                Proof::Token(""),
                song(33),
                RequestRules::default(),
                now
            ),
            Err(RequestError::WrongToken)
        );
        // He holds a song, so his name is refused to a phone until the operator opens the seat.
        let phone = |evening: &mut Evening| {
            evening.register(
                "phone-c",
                Some(SeatName {
                    shown: "Caio".to_owned(),
                    key: "caio".to_owned(),
                }),
                "token-c".to_owned(),
                SeatRules::default(),
            )
        };
        assert_eq!(phone(&mut evening), Err(SeatError::NameTaken));
        evening.open_seat(caio.seat).expect("the seat");
        let taken = phone(&mut evening).expect("an opened seat");
        assert_eq!(taken.seat.id, caio.seat, "with his songs");
    }

    /// An evening with two singers, one song sung, two queued and one waiting.
    fn a_busy_evening(now: Instant) -> (Evening, Singer, Singer) {
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        ask(&mut evening, &ana, 11, now);
        ask(&mut evening, &beto, 21, now);
        ask(&mut evening, &ana, 12, now);
        ask(&mut evening, &ana, 13, now);
        evening.pop(now);
        evening.set_accepting(false);
        (evening, ana, beto)
    }

    fn generic(number: u32) -> QueueRequest {
        QueueRequest {
            number: SongCode::new(number),
            title: "Generic".to_owned(),
            artist: None,
            singer: None,
            duration_ms: 0,
            seat: None,
        }
    }

    #[test]
    fn a_restored_evening_holds_what_the_snapshot_held() {
        let now = Instant::now();
        let (evening, ana, beto) = a_busy_evening(now);

        let snapshot = evening.snapshot(None, now);
        let restored = restore_turns(snapshot.clone(), now).expect("a sound snapshot");

        assert_eq!(numbers(&restored), vec![21, 12]);
        assert_eq!(restored.sung(), evening.sung());
        assert!(!restored.accepting());
        assert!(restored.seats().holds(ana.seat, &ana.token));
        assert!(restored.seats().holds(beto.seat, &beto.token));
        let requests: Vec<Request> = restored.requests(now).cloned().collect();
        assert_eq!(requests, evening.requests(now).cloned().collect::<Vec<_>>());
        assert_eq!(
            restored.snapshot(None, now),
            snapshot,
            "nothing changed on the way through"
        );
    }

    #[test]
    fn a_snapshot_of_an_unchanged_evening_equals_the_one_before() {
        let now = Instant::now();
        let (evening, ..) = a_busy_evening(now);
        let later = now + Duration::from_secs(60);
        assert_eq!(evening.snapshot(None, now), evening.snapshot(None, later));
    }

    #[test]
    fn a_snapshot_survives_its_written_form() {
        let now = Instant::now();
        let (mut evening, ..) = a_busy_evening(now);
        let playing = evening.pop(now);
        let snapshot = evening.snapshot(playing, now);

        let written = serde_json::to_string(&snapshot).expect("it serializes");
        let read: EveningSnapshot = serde_json::from_str(&written).expect("it parses");

        assert_eq!(read, snapshot);
    }

    #[test]
    fn a_restored_evening_issues_no_id_twice() {
        let now = Instant::now();
        let (evening, ana, beto) = a_busy_evening(now);
        let before = evening.snapshot(None, now);
        let mut restored = restore_turns(before.clone(), now).expect("a sound snapshot");
        restored.set_accepting(true);

        let caio = singer(&mut restored, "Caio");
        let request = ask(&mut restored, &caio, 31, now);

        assert!(caio.seat > ana.seat.max(beto.seat));
        assert!(before.requests.iter().all(|old| old.id < request.id));
        let entry = request.entry.expect("it went straight in");
        assert!(before.queue.iter().all(|old| old.id < entry));
    }

    /// The song that was playing starts again, so it is first, unsung, and its request is queued.
    #[test]
    fn the_song_that_was_playing_goes_back_to_the_front() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let beto = singer(&mut evening, "Beto");
        let first = ask(&mut evening, &ana, 11, now);
        ask(&mut evening, &beto, 21, now);
        let playing = evening.pop(now).expect("Ana's song starts");
        assert_eq!(evening.sung().len(), 1);

        let restored = restore_turns(evening.snapshot(Some(playing.clone()), now), now)
            .expect("a sound snapshot");

        assert_eq!(numbers(&restored), vec![11, 21]);
        let front = restored.queue().peek().expect("a queue entry");
        assert_eq!(front, &playing, "with its id and its seat");
        assert!(restored.sung().is_empty(), "it has not been sung");
        assert_eq!(state_of(&restored, first.id, now), RequestState::Queued);
    }

    /// A record that is not the playing song's stays, as after two seats were joined.
    #[test]
    fn a_record_of_another_song_stays_when_the_playing_song_returns() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        ask(&mut evening, &ana, 11, now);
        let mut playing = evening.pop(now).expect("Ana's song starts");
        playing.seat = None;

        let restored =
            restore_turns(evening.snapshot(Some(playing), now), now).expect("a sound snapshot");

        assert_eq!(numbers(&restored), vec![11]);
        assert_eq!(restored.sung().len(), 1);
    }

    #[test]
    fn a_snapshot_with_one_id_on_two_records_is_refused() {
        let now = Instant::now();
        let (evening, ..) = a_busy_evening(now);

        let mut entries = evening.snapshot(None, now);
        entries.queue[1].id = entries.queue[0].id;
        assert_eq!(
            restore_turns(entries, now).err(),
            Some(RestoreError::DuplicateId("queue entries"))
        );

        let mut seats = evening.snapshot(None, now);
        seats.seats[1].seat.id = seats.seats[0].seat.id;
        assert_eq!(
            restore_turns(seats, now).err(),
            Some(RestoreError::DuplicateId("seats"))
        );

        let mut requests = evening.snapshot(None, now);
        requests.requests[1].id = requests.requests[0].id;
        assert_eq!(
            restore_turns(requests, now).err(),
            Some(RestoreError::DuplicateId("requests"))
        );
    }

    #[test]
    fn a_counter_behind_its_ids_is_raised() {
        let now = Instant::now();
        let (evening, ..) = a_busy_evening(now);
        let mut snapshot = evening.snapshot(None, now);
        snapshot.queue_next = 0;
        snapshot.seats_next = 0;
        snapshot.guests_next = 0;
        snapshot.requests_next = 0;
        let highest_entry = snapshot.queue.iter().map(|entry| entry.id).max();
        let highest_seat = snapshot.seats.iter().map(|held| held.seat.id).max();
        let highest_request = snapshot.requests.iter().map(|request| request.id).max();

        let raised = restore_turns(snapshot, now)
            .expect("a sound snapshot")
            .snapshot(None, now);

        assert!(Some(raised.queue_next) > highest_entry);
        assert!(Some(raised.seats_next) > highest_seat);
        assert!(Some(raised.requests_next) > highest_request);
    }

    /// A queued request with no entry would never play, so it waits and the rule places it.
    #[test]
    fn a_queued_request_with_no_entry_waits_again() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        let ana = singer(&mut evening, "Ana");
        let request = ask(&mut evening, &ana, 11, now);
        let mut snapshot = evening.snapshot(None, now);
        snapshot.queue.clear();

        let restored = restore_turns(snapshot, now).expect("a sound snapshot");

        assert_eq!(numbers(&restored), vec![11], "the rule accepted it again");
        let again = restored
            .requests(now)
            .find(|held| held.id == request.id)
            .expect("the request is listed");
        assert_eq!(again.state, RequestState::Queued);
        assert_ne!(again.entry, request.entry, "under a new entry");
    }

    #[test]
    fn a_restored_finished_request_stays_for_the_fixed_time_again() {
        let start = Instant::now();
        let (evening, ..) = a_busy_evening(start);
        let late = start + FINISHED_FOR - Duration::from_secs(1);
        let snapshot = evening.snapshot(None, late);
        assert_eq!(snapshot.requests.len(), 4, "the played one is still listed");

        let restored = restore_turns(snapshot, late).expect("a sound snapshot");

        let just_before = late + FINISHED_FOR - Duration::from_secs(1);
        assert_eq!(restored.requests(just_before).count(), 4);
        assert_eq!(restored.requests(late + FINISHED_FOR).count(), 3);
        assert_eq!(
            evening.snapshot(None, start + FINISHED_FOR).requests.len(),
            3,
            "and a snapshot leaves out one whose time has passed"
        );
    }

    #[test]
    fn a_snapshot_past_a_cap_is_cut_to_it() {
        let now = Instant::now();
        let mut evening = Evening::with_order(QueueOrder::Turns);
        for number in 0..MAX_QUEUED {
            evening.add(generic(number as u32 + 1)).expect("add");
        }
        let playing = evening.pop(now).expect("a song starts");
        evening.add(generic(9_000)).expect("room for one");

        let restored =
            restore_turns(evening.snapshot(Some(playing), now), now).expect("a sound snapshot");

        assert_eq!(restored.queue().len(), MAX_QUEUED);
        assert_eq!(numbers(&restored)[0], 1, "the playing song is first");
    }
}
