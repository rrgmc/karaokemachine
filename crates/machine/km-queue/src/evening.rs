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

use std::time::{Duration, Instant};

use km_songcode::SongCode;

use crate::queue::{MAX_QUEUED, Queue, QueueEntry, QueueFull, QueueRequest};
use crate::seats::{Issued, Seat, SeatError, SeatName, SeatRules, Seats};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// A request as the list holds it.
#[derive(Debug, Clone)]
struct Held {
    request: Request,
    /// When it finished, which starts [`FINISHED_FOR`].
    finished_at: Option<Instant>,
}

/// The queue, the singers and their requests.
#[derive(Debug, Clone, Default)]
pub struct Evening {
    queue: Queue,
    seats: Seats,
    requests: Vec<Held>,
    next_request: u64,
}

impl Evening {
    /// An evening with nobody in it.
    pub fn new() -> Self {
        Self::default()
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

    /// Gives a seat a new token. See [`Seats::reissue`].
    pub fn reissue(&mut self, id: u64, token: String) -> Result<Issued, SeatError> {
        self.seats.reissue(id, token)
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
        if let Proof::Token(token) = proof
            && !self.seats.holds(seat, token)
        {
            return Err(RequestError::WrongToken);
        }
        if self.requests.len() >= MAX_REQUESTS {
            return Err(RequestError::Full);
        }
        let waiting = self
            .requests
            .iter()
            .filter(|held| held.request.seat == seat && held.request.state == RequestState::Waiting)
            .count();
        if waiting >= rules.max_waiting as usize {
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

    /// Applies the rule: accepts every waiting request whose singer has no entry in the queue.
    ///
    /// Oldest first, so the request that has waited longest takes the room there is. A full queue
    /// accepts nothing, and its requests wait until an entry leaves.
    fn settle(&mut self, now: Instant) {
        self.sweep(now);
        for index in 0..self.requests.len() {
            let request = &self.requests[index].request;
            if request.state != RequestState::Waiting || self.queue.holds_seat(request.seat) {
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

/// Whether a finished request has stayed its [`FINISHED_FOR`].
fn expired(held: &Held, now: Instant) -> bool {
    held.finished_at
        .is_some_and(|at| now.saturating_duration_since(at) >= FINISHED_FOR)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_first_request_goes_straight_into_the_queue() {
        let now = Instant::now();
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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

    #[test]
    fn the_whole_list_is_capped() {
        let now = Instant::now();
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
        let mut evening = Evening::new();
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
}
