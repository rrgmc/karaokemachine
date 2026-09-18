//! The seam between the remote's pages and whatever is answering them.
//!
//! Four traits, and the split is by *who owns the answer* rather than by verb — the same reasoning
//! that split `km-api`'s [`Catalog`](km_api::machine::Catalog) from its
//! [`Controller`](km_api::machine::Controller), and for the same payoff: a page that only browses
//! must not be able to fail because the karaoke machine is switched off.
//!
//! * [`Songs`] is the catalog. In the online mode it is `library.sqlite` through the machine's own
//!   catalog; in the offline mode it is the local mirror. Either way it answers with the machine
//!   powered down.
//! * [`Machine`] is playback: the queue, the transport, the settings, and the event stream. This is
//!   the one that can be *absent*, and [`Connection`] is how a page says so.
//! * [`Favorites`] is the offline app's own collection. It has no online implementation at all —
//!   see the `Capabilities` on [`Remote`](crate::Remote) — because a favorites list living on an
//!   appliance under a television would be a shared list nobody owns.
//! * [`Connect`] is *which* machine, and the three ways of changing it. Offline only too, and for a
//!   stronger reason than the last one: online there is nothing to choose between, because the
//!   machine is this process. It is the one trait whose implementation lives entirely outside what
//!   these pages can see — a locator, a mirror and a data directory are `km-remote-core`'s.
//!
//! Every method is `async` and takes `&self`, and the traits are used as `dyn`. That combination is
//! what `async-trait` is here for: the mode is a compile-time fact per binary, but making the
//! handlers generic over it would put a type parameter on every one of them for no gain, since no
//! binary ever holds two.
//!
//! The methods are async even where an implementation does nothing async — SQLite is synchronous in
//! both modes. That is deliberate: it puts the choice of *how* to get off the async runtime inside
//! the implementation, where the thing being got off is known, rather than making every handler
//! remember to wrap a call in `spawn_blocking`.

use std::collections::HashSet;

use km_api::dto::{AddedToQueueDto, QueueDto, SettingsPatchDto, SongDto, StateDto};
use km_api::events::Event;
use km_songcode::SongCode;
use tokio::sync::broadcast;

/// Why something the remote asked for did not happen.
///
/// Coarser than the API's two error enums, because a page has fewer things it can usefully say than
/// a JSON client does. The one distinction that earns its place and has no counterpart in `km-api`
/// is [`Offline`](RemoteError::Offline): the machine not being there is the normal state of affairs
/// for the offline app, not a fault, and the pages treat it as such.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteError {
    /// The machine is not reachable, as one of [`codes`].
    ///
    /// **A code and not a sentence.** Finished US English composed in `km-remote-core` is what puts
    /// `The karaoke machine is not answering.` inside a Portuguese page — see [`codes`] for why
    /// in-process is not the same as translated.
    #[error("machine offline: {0}")]
    Offline(&'static str),
    /// The machine took the command but never acknowledged it inside the timeout.
    ///
    /// **Not a failure.** The command was almost certainly obeyed; what is missing is the
    /// confirmation. The pages say "sent" rather than "queued" and do not color it as an error,
    /// because reporting a red failure for a song that is in fact in the queue is worse than saying
    /// less.
    #[error("no acknowledgment")]
    NotAcknowledged,
    /// No song, queue entry or folder with that identifier.
    #[error("not found")]
    NotFound,
    /// The queue is at its limit.
    #[error("the queue is full")]
    QueueFull,
    /// The machine cannot do this to what is loaded — transposing a video song, a melody toggle on a
    /// file where detection abstained.
    ///
    /// **The one variant that carries a code**, because it is the one a singer meets and the one
    /// with several distinguishable meanings. The page renders from the code, out of its own
    /// catalog, in the language that viewer is reading — the machine's `message` is a fallback and a
    /// log line, never the page's only source.
    ///
    /// A `String` rather than a `&'static str` because it arrives over HTTP for the offline remote,
    /// which may be talking to a machine of a different version. A code this build does not know is
    /// not an error: it renders as the generic refusal, which is exactly the sentence every one of
    /// these produced before any of them had a name.
    #[error("{message}")]
    Unavailable {
        /// The machine's stable name for the refusal.
        code: String,
        /// What the machine said, for the log and for a surface that shows diagnostics.
        message: String,
    },
    /// This device refused the request itself, as one of [`codes`] — an empty folder name, a
    /// duplicate one, an empty address box.
    ///
    /// Distinct from [`Rejected`](RemoteError::Rejected), which is the *machine* refusing: this one
    /// was decided in this process, and there is a message for every value it can take.
    #[error("refused: {0}")]
    Refused(&'static str),
    /// The machine refused the request, in its own words.
    ///
    /// **Never rendered.** A 400 is aimed at whatever built the request rather than at the person
    /// holding the phone, and it arrives in the machine's English — so the page says the generic
    /// failure and this goes to the log, exactly as [`Unavailable`](RemoteError::Unavailable)'s
    /// message does.
    #[error("{0}")]
    Rejected(String),
    /// A password is needed, or the one held has expired.
    #[error("not authorized")]
    Unauthorized,
    /// Anything else.
    #[error("{0}")]
    Failed(String),
}

/// The remote's own refusal, for a capability this host does not have.
///
/// **Not a code the machine sends.** The online remote is served by the machine itself and keeps no
/// favorites — a collection living on an appliance under a television would be a shared list nobody
/// owns — so asking one for a folder is refused here, before anything is sent. It shares the code
/// vocabulary because it shares the way it is rendered: a page looks up a sentence in its own
/// catalog, whoever refused.
pub const NO_FAVORITES: &str = "no_favorites";

/// The remote's own refusal for a host with no demo song to start.
///
/// The default body of `start_demo`, so an implementation that predates the route still compiles
/// and still says something true.
pub const NO_DEMO: &str = "no_demo";

/// The stable names a refusal or a connection state travels under, between the crate that decides
/// one and the crate that words it.
///
/// **The same bargain [`RemoteError::Unavailable`] already makes with the machine, made one crate
/// closer.** `km-remote-core` decides that no machine has been found and that a folder name is
/// taken; it has no catalog and no viewer to ask, so for as long as it sent finished sentences its
/// US English landed inside otherwise translated pages. It sends a code now and
/// [`crate::words::code_key`] turns it into a message id.
///
/// Spelled here rather than in `km-remote-core`, because this is the crate that has to have a
/// message for each — and because the dependency runs this way: the core implements these traits.
pub mod codes {
    /// The machine was there and has stopped answering.
    pub const NOT_ANSWERING: &str = "not-answering";
    /// Nothing has ever been found to talk to.
    pub const NONE_FOUND: &str = "none-found";
    /// Looking on the network, having never found anything.
    pub const LOOKING: &str = "looking";
    /// An address is in hand and the first request has not come back.
    pub const CONNECTING: &str = "connecting";
    /// The address box was submitted empty.
    pub const ADDRESS_NEEDED: &str = "address-needed";
    /// A folder was named with nothing but spaces.
    pub const FOLDER_NEEDS_NAME: &str = "folder-needs-name";
    /// A folder of that name is already there.
    pub const FOLDER_NAME_TAKEN: &str = "folder-name-taken";
    /// The last folder cannot be deleted — the star's flow has nowhere to lead without one.
    pub const ONLY_FOLDER: &str = "only-folder";
    /// The machine closed the event stream politely. It has still gone.
    pub const STREAM_CLOSED: &str = "stream-closed";

    /// Every code above, for the test that checks each reaches a message.
    pub const ALL: &[&str] = &[
        NOT_ANSWERING,
        NONE_FOUND,
        LOOKING,
        CONNECTING,
        ADDRESS_NEEDED,
        FOLDER_NEEDS_NAME,
        FOLDER_NAME_TAKEN,
        ONLY_FOLDER,
        STREAM_CLOSED,
    ];
}

/// What a move or a refresh actually did.
///
/// **Facts, not a sentence.** `Connect::connect_to`, `use_found` and `refresh` used to answer
/// `Ok(String)` — `Now using {url}. Copied {n} songs from the machine.` — composed in a crate with
/// no catalog, so that sentence appeared verbatim on a page in any language. The url and the count
/// travel; [`crate::handlers`] writes the words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// The address now in use, where this was a move rather than a refresh of the one in hand.
    pub moved_to: Option<String>,
    /// How the copy of the catalog ended up.
    pub outcome: CopyOutcome,
}

/// What became of this device's copy of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyOutcome {
    /// It was already current, and holds this many songs.
    AlreadyCurrent(usize),
    /// This many songs were copied from the machine.
    Imported(usize),
    /// The machine did not answer. **Not a failure**: the address is held and will be used as soon
    /// as it does, which is the normal state of a machine under a television that is switched off.
    NotAnswering,
}

impl RemoteError {
    /// Whether this is worth coloring red.
    ///
    /// [`NotAcknowledged`](RemoteError::NotAcknowledged) is the reason this exists — see its own
    /// documentation.
    pub fn is_fault(&self) -> bool {
        !matches!(self, RemoteError::NotAcknowledged)
    }
}

/// How an artist name is matched.
///
/// Two genuinely different questions wearing one field in the API's search: typing "jobim" into the
/// box should find every artist whose name contains it, while opening an artist from the list must
/// show *that* artist and not everybody whose name contains theirs. The API only offers the first,
/// which is right for a search box and wrong for a drill-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtistFilter {
    /// Substring, case-insensitive — what a search box means.
    Contains(String),
    /// The whole name — what opening an artist means.
    Exactly(String),
}

/// How a list of songs is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// Relevance when there is a query, then the folded title. Never the raw title: SQLite's default
    /// collation sorts every accented character after `Z`, which buries a Portuguese corpus.
    #[default]
    Best,
    /// Folded title, then number.
    Title,
    /// Artist, then title.
    Artist,
    /// Song number — the order a printed book would be in.
    Number,
}

/// One browse request, in the remote's own terms.
///
/// Almost `km_catalog::SearchQuery`, and the differences are the point.
///
/// `artist` is richer here (see [`ArtistFilter`]) and `initial` is a browsing affordance that needs an
/// indexed folded-initial column only the mirror carries. Everything else lines up, `language`
/// included — that one is matched exactly on both sides, which the catalog settled when it started
/// storing ISO 639-1 codes.
///
/// **What is deliberately absent is any notion of a list of ids.** A favorites folder is a bounded,
/// personal collection — tens to a few hundred songs — and it is assembled by asking for those songs
/// by number and then filtering and paging in Rust. Pushing an `id IN (...)` down into the catalog
/// would put the remote's furniture inside the machine's search, for a query that is never large
/// enough to need SQL's help.
///
/// What *is* shared is the part that is subtle: both implementations turn `text` into an FTS5 MATCH
/// string with [`km_catalog::fts_match_query`], so neither reinvents the escaping that stops
/// `AC/DC` being a syntax error and `rock 'n' roll` being three operators.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowseQuery {
    /// Free text over title and artist.
    pub text: Option<String>,
    /// Restrict to an artist.
    pub artist: Option<ArtistFilter>,
    /// Restrict to one language, as an ISO 639-1 code. Matched exactly, as the catalog matches it.
    pub language: Option<String>,
    /// Restrict to songs carrying **any** one of these tags.
    ///
    /// OR rather than AND, matching `km_catalog::SearchQuery::tags`: the vocabulary is open, so one
    /// kind of song is filed under several words by different hands, and a person picking both means
    /// *either of these*.
    ///
    /// Slugs by the time they arrive here — the handler folds what came off the query string
    /// through `km_kmpkg::Tag`, so both the online and the offline path compare like with like.
    pub tags: Vec<String>,
    /// Restrict to titles starting with this initial; `#` means "a digit".
    ///
    /// Offline only — see `Capabilities::initial_filter`.
    pub initial: Option<char>,
    /// Leave out songs from these packages, by package id.
    ///
    /// The packages the person holding this phone hid. They come from that person's cookie on every
    /// request, so they are never part of a link or of where the person was.
    pub hidden_packages: Vec<String>,
    /// How to order what is left.
    pub order: Order,
    /// Page size.
    pub limit: usize,
    /// How far in.
    pub offset: usize,
}

/// One page of songs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SongPage {
    /// The rows.
    pub songs: Vec<SongDto>,
    /// How many match in total, when that is cheap to know.
    ///
    /// `None` where counting would mean a second full scan. The page then says how many it is
    /// showing rather than inventing a total, which is the honest version of the API's `more` flag.
    pub total: Option<usize>,
    /// Whether asking for the next page is worth it.
    pub more: bool,
}

/// A package the catalog holds songs from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRow {
    /// The package id, which is what a hide is stored under.
    pub id: String,
    /// What its packager named it, or the id where the catalog holds no name for it.
    pub name: String,
    /// How many songs.
    pub songs: usize,
}

/// An artist, and how many songs are filed under them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistRow {
    /// The name as the catalog stores it. There is no artist id — the column is text.
    pub name: String,
    /// How many songs.
    pub songs: usize,
}

/// A language, and how many songs are in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageRow {
    /// The ISO 639-1 code the catalog stores — `pt`, `ja`, or `und` for "nobody could tell".
    pub code: String,
    /// What to show somebody.
    pub name: String,
    /// How many songs.
    pub songs: usize,
}

impl LanguageRow {
    /// Names a code, using the compiled-in ISO 639-1 table `km-kmpkg` already carries.
    ///
    /// A second table here would be a second thing to disagree with the one packaging writes codes
    /// from, and the disagreement would show up as a picker offering a language nothing is filed
    /// under. A code the table does not have is shown as itself rather than dropped — a catalog
    /// built by an older packager can hold anything, and a filter that silently omits a language is
    /// worse than one that spells it oddly.
    pub fn new(code: impl Into<String>, songs: usize) -> Self {
        let code = code.into();
        let name = km_kmpkg::language::Language::parse(&code)
            .map(|language| language.name().to_owned())
            .unwrap_or_else(|| code.clone());
        Self { code, name, songs }
    }
}

/// A tag, and how many songs carry it.
///
/// **No `name` beside the slug, where [`LanguageRow`] has one**, and the absence is the whole
/// difference between the two filters. A language is a row in a compiled-in table, so `pt` can be
/// shown as `Portuguese`; a tag is a word somebody typed, so the slug *is* what they typed, folded.
/// There is nothing to look it up in and nothing that could be more readable than itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    /// The slug, as stored — `rock`, `anos-80`.
    pub tag: String,
    /// How many songs carry it.
    pub songs: usize,
}

/// A song as a favorite remembers it: the number it was filed under, and what it was.
///
/// **The number alone is not enough, and that is the whole reason this type exists.** A number is
/// `bank * 1000 + slot`, and the bank is assigned by the machine rather than carried by the package
/// — `Library::set_package_bank` re-keys every song in a package, two machines can bank the same
/// package differently, and a rebuild re-flows the slots. Any of those leaves a collection pointing
/// at numbers that have moved, and pointing at *some other package's* songs is the bad half of that:
/// the folder still lists something, and it is the wrong song.
///
/// The two identifying fields both come out of the package and neither moves. They are `Option`
/// because a collection built before this existed has neither, and because a package may predate the
/// manifest field — see [`Songs::resolve`] for what is done about that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongRef {
    /// The number the favorite is filed under.
    pub code: SongCode,
    /// The package the song came from.
    pub package_id: Option<String>,
    /// What the package said the song's content hashes to.
    pub content_hash: Option<String>,
}

impl SongRef {
    /// A reference carrying nothing but a number — what every favorite held before this.
    pub fn code(code: SongCode) -> Self {
        Self {
            code,
            package_id: None,
            content_hash: None,
        }
    }

    /// Whether anything but the number is known.
    pub fn is_identified(&self) -> bool {
        self.content_hash.is_some()
    }

    /// Whether a song found by its number is provably not this favorite's.
    ///
    /// **The number rung is the loosest of the three and this is what bounds it.** A number names a
    /// song on the machine that assigned it and something else on a machine that banked the same
    /// package elsewhere, so following one onto a second machine is how a folder comes to list the
    /// wrong song in the right place with nothing on screen to say so. Where both sides carry a
    /// hash, the catalog can settle it: two different hashes are two different recordings, whatever
    /// number they share.
    ///
    /// **A missing hash on either side means unknown, never different** — the rule
    /// [`Songs::resolve`] already applies to the favorite, applied to the candidate as well. A
    /// package built before the manifest carried the field mirrors a null here, and refusing those
    /// would strand every favorite of one behind a test it cannot pass.
    pub fn contradicted_by(&self, candidate_hash: Option<&str>) -> bool {
        match (&self.content_hash, candidate_hash) {
            (Some(asked), Some(found)) => asked != found,
            _ => false,
        }
    }
}

/// Why a favorite could not be placed.
///
/// **Three cases because the remedy differs**, which is [`crate::share::ShareError`]'s reason for
/// being a variant rather than a sentence: a package that is not installed is something the owner
/// can go and install, a recording that is not in a package they *do* have is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    /// Nothing in this catalog comes from that package.
    PackageAbsent,
    /// The package is here, and does not hold that recording.
    RecordingAbsent,
    /// Nothing was carried but a number, and no song has it.
    NumberAbsent,
}

/// What asking the catalog about one favorite produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    /// What was asked for.
    pub asked: SongRef,
    /// The song it landed on, or why it did not land.
    pub outcome: Result<SongDto, Miss>,
}

impl Resolution {
    /// The song, when there is one.
    pub fn song(&self) -> Option<&SongDto> {
        self.outcome.as_ref().ok()
    }

    /// Whether the song is now under a different number than the favorite was filed under.
    ///
    /// The signal the caller writes back: a `true` here is a re-banked package caught before it
    /// could show somebody the wrong song.
    pub fn moved(&self) -> bool {
        self.song()
            .is_some_and(|song| song.number != self.asked.code)
    }
}

/// The catalog, whichever one it is.
#[async_trait::async_trait]
pub trait Songs: Send + Sync + 'static {
    /// One page of songs.
    async fn search(&self, query: &BrowseQuery) -> Result<SongPage, RemoteError>;

    /// One song by its number.
    async fn song(&self, number: SongCode) -> Result<Option<SongDto>, RemoteError>;

    /// Several songs by number, in the order given, skipping any that are not there.
    ///
    /// Used where a list of ids has to become a list of songs — a favorites folder. Skipping rather
    /// than failing is deliberate: a folder is allowed to outlive the package a song came from.
    async fn songs_by_number(&self, numbers: &[SongCode]) -> Result<Vec<SongDto>, RemoteError>;

    /// Several favorites, resolved to the songs they name — in the order given, one answer each.
    ///
    /// **Three rungs, each looser than the one above, and the order is the whole design:**
    ///
    /// 1. `(package_id, content_hash)`. A candidate key, and not by luck: two songs in one package
    ///    with the same hash are a `ManifestProblem::DuplicateContent`, which `km_kmpkg` refuses
    ///    both when opening a package and when writing one. Neither half moves when a bank does, so
    ///    this rung is what makes a favorite survive a re-banking, a re-slotting, and a move to
    ///    another machine.
    /// 2. `content_hash` alone. The same *recording* filed in some other package — a re-packaging,
    ///    or a friend's machine that got it from elsewhere. Legitimately several rows, because two
    ///    packages holding one recording is reported rather than refused, so this rung must **pick**
    ///    rather than assume: lowest number, so that the same catalog always answers the same way.
    /// 3. The number. What every favorite held before the two fields above existed, and what a song
    ///    whose package never recorded a hash still has.
    ///
    /// **A missing hash falls through rather than failing.** `Manifest::problems` skips a song with
    /// no hash in its own duplicate check, so a null here means *unknown*, never *different*, and a
    /// resolver that treated the two alike would strand every favorite of a package built before the
    /// field.
    ///
    /// Unlike [`Self::songs_by_number`] this does not skip what it cannot find: a caller that wants
    /// to *list* the unplaceable needs to be told, and one that wants the old behavior takes
    /// [`Resolution::song`] and drops the rest.
    async fn resolve(&self, refs: &[SongRef]) -> Result<Vec<Resolution>, RemoteError>;

    /// The artists, with counts, optionally filtered by name.
    ///
    /// `hidden` leaves out the songs of the packages this person hid, so an artist found only there
    /// is not listed. It means the same in [`Self::languages`] and [`Self::tags`].
    async fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<ArtistRow>, RemoteError>;

    /// The languages present, with counts.
    async fn languages(&self, hidden: &[String]) -> Result<Vec<LanguageRow>, RemoteError>;

    /// The tags present, with counts, commonest first.
    ///
    /// The whole vocabulary — there is no table of legal tags anywhere, so what the catalog is
    /// filed under is the only statement of what exists, and it is what the picker is drawn from.
    async fn tags(&self, hidden: &[String]) -> Result<Vec<TagRow>, RemoteError>;

    /// The packages the songs come from, with names and counts, for the list on Setup that a person
    /// hides packages from.
    async fn packages(&self) -> Result<Vec<PackageRow>, RemoteError>;

    /// How many songs there are.
    async fn count(&self) -> Result<usize, RemoteError>;
}

/// Whether the machine is there, and what to say when it is not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Connection {
    /// Whether commands will work.
    pub online: bool,
    /// Where the machine is, for the banner. `None` when nothing has ever been found.
    pub address: Option<String>,
    /// What the machine calls itself, where it has said.
    ///
    /// **Learned from `/discover` on the last refresh, not from the browse that found it.** The two
    /// differ after a rename, and only this one is right: a name taken when the machine was found
    /// would go on being shown for as long as the process lived. `None` until something has
    /// answered, and `None` for a machine advertising nothing worth showing — a card falls back to
    /// the address, which it has anyway, rather than drawing an empty heading.
    pub name: Option<String>,
    /// Why it is not reachable, as one of [`codes`].
    ///
    /// **A code and not a sentence**, for [`RemoteError::Offline`]'s reason: the banner that prints
    /// this is drawn in the viewer's language and the client that sets it has no idea who is
    /// reading. `None` where there is nothing to add to *not reachable*.
    pub reason: Option<&'static str>,
}

impl Connection {
    /// The online mode's answer: the machine is this process.
    pub fn in_process() -> Self {
        Self {
            online: true,
            address: None,
            // The machine is this process, so there is no card and nothing to head. The online mode
            // draws no connection block at all — see `Capabilities::connection`.
            name: None,
            reason: None,
        }
    }
}

/// What a transport button does.
///
/// A local enum rather than `km_api`'s `TransportCommand`, because that one carries `Seek(ms)` and
/// `PlayFile(path)`, and neither is a thing a singer's remote offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Start, or resume.
    Play,
    /// Hold, keeping the song loaded.
    Pause,
    /// Give up on this one and take the next.
    Skip,
    /// From the top.
    Restart,
    /// Stop and unload.
    Stop,
}

/// Playback: the queue, the transport, the settings, and the events.
#[async_trait::async_trait]
pub trait Machine: Send + Sync + 'static {
    /// Everything the Now page draws.
    async fn state(&self) -> Result<StateDto, RemoteError>;

    /// The queue as it stands.
    async fn queue(&self) -> Result<QueueDto, RemoteError>;

    /// Put a song at the back.
    ///
    /// Playing something *next* is this followed by [`move_entry`](Machine::move_entry) to index 0,
    /// done in the handler rather than here — two API calls either way, and keeping the trait to the
    /// operations the API actually has means neither implementation has to fake one.
    async fn enqueue(
        &self,
        number: SongCode,
        singer: Option<&str>,
    ) -> Result<AddedToQueueDto, RemoteError>;

    /// Take an entry out, by the id it was given. Never by position — positions shift underneath a
    /// page that has been open for a minute.
    async fn dequeue(&self, entry_id: u64) -> Result<QueueDto, RemoteError>;

    /// Move an entry to a position.
    async fn move_entry(&self, entry_id: u64, to_index: usize) -> Result<QueueDto, RemoteError>;

    /// Empty the queue.
    async fn clear_queue(&self) -> Result<QueueDto, RemoteError>;

    /// Press a transport button.
    async fn transport(&self, command: Transport) -> Result<StateDto, RemoteError>;

    /// Ask the machine to play something of its own choosing, now.
    ///
    /// **`()` rather than a `StateDto`, unlike every other write here, and that is deliberate.** The
    /// song starts on the machine's poll thread a moment after this answers, so a state read taken
    /// here would be stale by construction — it would say `Nothing playing` about a machine that is
    /// about to. The card the caller draws is re-read anyway, and republished over the event stream
    /// when the song actually starts.
    ///
    /// Refused unless the deck is empty and nothing is queued, in a sentence the machine writes; see
    /// `Controller::start_demo_song`. Defaulted to a refusal so an implementation that predates the
    /// route still compiles and still says something true.
    async fn start_demo(&self) -> Result<(), RemoteError> {
        Err(RemoteError::Unavailable {
            code: NO_DEMO.to_owned(),
            message: "this machine cannot start a demo song".to_owned(),
        })
    }

    /// Change what can be changed while a song plays.
    async fn update_settings(&self, patch: &SettingsPatchDto) -> Result<StateDto, RemoteError>;

    /// Subscribe to the machine's events.
    ///
    /// Synchronous, and a `broadcast::Receiver` rather than a stream, because both modes genuinely
    /// have one: online it is `km_api::events::Events::subscribe`, and offline it is a channel fed
    /// by the task holding the WebSocket. The remote's own fan-out to browsers sits on top of this,
    /// so the two modes differ in where events come from and in nothing after that.
    fn subscribe(&self) -> broadcast::Receiver<Event>;

    /// Whether the machine is reachable, for the banner and the connection dot.
    fn connection(&self) -> Connection;

    /// Somebody is looking at a page again — try the machine now rather than on the next backoff.
    ///
    /// **Defaulted to nothing, and free for the online mode**, which is the shape
    /// [`package_problems`](Machine::package_problems) below already has: there the machine is this
    /// process and there is no link to retry. Offline it resets the reconnection loop's wait.
    ///
    /// Sync, cheap, non-blocking and infallible, exactly like [`connection`](Machine::connection)
    /// and [`subscribe`](Machine::subscribe) beside it. A caller may say this whenever it likes and
    /// need not know whether it will help.
    ///
    /// **Who says it: a page opening an event stream.** A phone that has been in a pocket froze the
    /// whole process with its screen, so the wait it resumes was measured before the interruption —
    /// which is the reported fault, a red strip for ten seconds while a machine that was up went
    /// unasked. The precedent is the `/dev/` remote's Reconnect button, whose decision puts it
    /// better than this can: *somebody pressing it is saying they think the machine is back, which
    /// is a better guess than the interval a run of failures had arrived at.*
    fn wake(&self) {}

    /// Packages the machine could not install, as sentences to put in a banner.
    ///
    /// **Free for the online remote and deliberately empty for the offline one.** `km-app` serves
    /// these pages from the machine itself and already holds the list; a remote on somebody's phone
    /// holds a *mirror of the catalog*, which by construction cannot describe a package that
    /// never got into it. Fetching them would mean an extra request against `/packages` on every
    /// page render, and the person who can act on this is next to the machine, where the idle screen
    /// says it anyway.
    async fn package_problems(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Which machine the remote is talking to, and how that was decided.
///
/// Distinct from [`Connection`], which answers only *is it there*. This is the rest of the question:
/// which address, how it came to be that one, whether it is pinned, and how much of a catalog this
/// device holds of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineStatus {
    /// Whether it answers, and what to say when it does not.
    pub connection: Connection,
    /// How the address was arrived at, as a stable code: `adopted`, `remembered`, `asked-for`,
    /// `machine-moved` or `chosen`.
    ///
    /// `None` before anything has been found at all. **A code and not a sentence**, because the card
    /// it goes on is drawn in the viewer's language and the process that decided this has no idea
    /// who is reading — the same bargain a refusal makes. [`crate::words::how_key`] turns it into a
    /// message id, and a code this build does not know draws nothing rather than the wrong sentence.
    pub how: Option<String>,
    /// Whether this machine will be kept whatever the network offers.
    pub pinned: bool,
    /// How many songs this device's own copy of the catalog holds.
    pub songs: usize,
    /// Whether looking on the network is a thing this build can do at all.
    ///
    /// False under a locator that finds nothing by construction. A Rescan button that can only ever
    /// answer "nothing found" is the grayed-out star [`Capabilities`](crate::Capabilities) exists to
    /// avoid, so where this is false the button is **absent** rather than disabled.
    pub can_browse: bool,
}

/// What a look at the network turned up, and what was done about it.
///
/// [`Connect::rescan`] used to answer `Option<String>` — the address it had *already moved to*, or
/// nothing. That shape could not express the case this type exists for: a remote that is talking to
/// a working machine and has just been shown another one. Moving to it is a decision, and a button
/// press that browses is not the same press as a decision to switch.
///
/// **The pin comes off in every one of these**, including [`Self::Nothing`]. That is the promise
/// `Naming a machine from the remote` makes about this button and it is independent of whether the
/// connection moves — un-pinned means the background watch may recover to something better the next
/// time this machine goes quiet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scanned {
    /// Nothing answered. The address in hand is still the address in hand.
    Nothing,
    /// Found the machine this remote belongs on and moved to it, there being no working connection
    /// to disturb.
    ///
    /// **Its own machine, or any machine where it knows of none.** A remote that has met one follows
    /// that one wherever it has got to and takes no other; a remote that has met none has nothing to
    /// lose by trying what the network offered, which is how a first run finds the machine in the
    /// room at all.
    Using(String),
    /// The machine in hand is among what answered.
    ///
    /// Worth its own case rather than folding into [`Self::Kept`]: offering *use this one* for the
    /// address printed on the card above it reads as a fault in the page. It is **not** a reason to
    /// say nothing about the other machines beside it — those travel in [`Scan::others`], and the
    /// two treated as exclusive is exactly the fault this variant exists to keep apart.
    Already(String),
    /// Nothing moved, and the machine in hand did not answer the browse.
    ///
    /// **The answer a remote gets when its own machine is switched off and somebody else's is not.**
    /// A remote that knows which machine is its own moves to that one and to no other, so a look
    /// that turns up only strangers ends here with all of them in [`Scan::others`], waiting for a
    /// press. See `A remote looks again when its machine goes quiet` in docs/decisions/remotes.md.
    ///
    /// The url lives in [`Scan::others`] rather than here: a `Found(url)` beside `others[0]` would be
    /// two spellings of one fact.
    Kept,
}

/// What a look at the network turned up: what it did about the machine in hand, and what else
/// answered.
///
/// **An outcome plus a list, rather than one answer carrying one address.** The reported fault
/// this shape exists for: two machines on a LAN, and *Rescan* saying "the machine you are already
/// using is the one on the network" with no way to reach the second. One answer cannot express
/// *the one you are on, plus these others*, so a handler has nowhere to put them and drops them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// What happened to the machine in hand. Only [`Scanned::Using`] moves anything.
    pub outcome: Scanned,
    /// Every other machine the look turned up, sorted and deduplicated.
    ///
    /// Never contains the machine in hand — recognized by **id** where both ends have one and by
    /// address otherwise, so a machine that has merely moved is not offered back to itself as
    /// though it were a stranger. Following such a move is the background watch's job; see `The
    /// remote follows its machine, and looks without being asked` in docs/decisions/remotes.md.
    pub others: Vec<Offer>,
}

/// One machine on the network that is not the one in hand.
///
/// **It names the machine as well as the address**, which the `Rescan offers a machine rather than
/// taking one` decision argues against. Its cost — "a seventh C function and nineteen places in
/// prose" — is one this type does not pay: `km_api::discover::Sighting` carries a name, the
/// registry fills it, this type never crosses the FFI, and no host implements a locator. Its other
/// reason, that a browse-time name is a snapshot of a renameable thing, is thin for a value that
/// lives for one button press and is replaced by `/discover`'s name the moment the offer is
/// accepted.
///
/// With **one** offer an address is a complete answer. With three, three bare
/// `http://192.168.1.x:8177` strings are precisely the "which machine is this?" failure `The card
/// says which machine, in the owner's words` was written against — and the person is being asked to
/// choose between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The address to point at, which is what the form posts.
    pub url: String,
    /// What it calls itself, where that is worth showing.
    ///
    /// `None` for a machine that advertises no name — and for one whose "name" is its own id, which
    /// is what the registry falls back to and which would be worse on a card than no name at all.
    pub name: Option<String>,
}

/// Which machine this device talks to, and the four ways of changing that.
///
/// **Absent in the online mode**, where the machine is this process — see
/// [`Capabilities::connection`](crate::Capabilities::connection). It is a fourth trait beside
/// [`Songs`], [`Machine`] and [`Favorites`] for the same reason those three are separate: this
/// crate cannot see `km-remote-core`, where a locator, a mirror and a data directory live, and the
/// online mode has none of them to lend it.
///
/// **Nothing here fails for an ordinary "that did not work."** A browse that finds nothing is
/// `Ok(None)`, and an address nothing answers at is still an address this device is now holding.
/// [`RemoteError`] is for a refusal, not for a machine being switched off — which is the normal
/// state of affairs here, exactly as it is for [`Connection`].
#[async_trait::async_trait]
pub trait Connect: Send + Sync + 'static {
    /// Which machine, how it was arrived at, and how much of it is held here.
    async fn status(&self) -> MachineStatus;

    /// Points at what somebody typed, and pins it.
    ///
    /// Pinning is what makes this the same instruction as `--machine`: *that one*, and do not wander
    /// off it because it went quiet. [`rescan`](Self::rescan) is how the pin comes off. Answers with what happened,
    /// as facts rather than as a sentence — see [`Copied`].
    async fn connect_to(&self, address: &str) -> Result<Copied, RemoteError>;

    /// Looks on the network now, un-pinning first, and says what it found.
    ///
    /// **It moves only where there is nothing to disturb, and only to the machine this remote
    /// belongs on.** A remote with a machine answering keeps it and offers what turned up; so does
    /// one whose own machine is switched off while a stranger is not — see [`Scanned`] for why that
    /// is a different act from pressing the button in the first place. A remote that knows of no
    /// machine takes what it finds.
    ///
    /// **Everything else that answered comes back too**, in [`Scan::others`], whatever happened to
    /// the machine in hand. A `Using` carries them as well as an `Already` does: a remote that had
    /// nothing, took machine A and never mentioned B was the same fault in its second location.
    async fn rescan(&self) -> Result<Scan, RemoteError>;

    /// Moves to an address a browse turned up, without pinning it.
    ///
    /// **Deliberately not [`connect_to`](Self::connect_to).** Pinning means *that one, whatever
    /// happens*, which is what somebody typing an address into the box is saying; accepting what the
    /// network offered is not saying that, and a remote pinned to it could never be recovered from
    /// by the background watch. The address arrives already normalized, since it came from a browse
    /// rather than from a person.
    async fn use_found(&self, url: &str) -> Result<Copied, RemoteError>;

    /// Re-reads the catalog from whichever machine is in hand.
    async fn refresh(&self) -> Result<Copied, RemoteError>;
}

/// A named list a song is in.
///
/// There is no other kind of favorite — the same conclusion `km-package-builder` reached in the `What a
/// favorite is` decision, arrived at here for the same reason: a star that sets a boolean has no
/// answer to *which favorite?*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRow {
    /// Its id.
    pub id: i64,
    /// Its name.
    pub name: String,
    /// How many songs are in it.
    pub songs: usize,
}

/// One favorite, and what the catalog said it was.
///
/// Produced from a [`Resolution`] that found something, and fed to [`Favorites::reconcile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    /// The number the row is filed under now.
    pub was: SongCode,
    /// The number it should be filed under. Equal to [`Self::was`] in the ordinary case.
    pub now: SongCode,
    /// The package the song came from.
    pub package_id: Option<String>,
    /// What the package said the song's content hashes to.
    pub content_hash: Option<String>,
}

impl Reconciliation {
    /// What to write back for a favorite that resolved.
    ///
    /// `None` when there is nothing to write: the song was found under the number it was already
    /// filed under, and it already carries the hash that found it. Returning an `Option` here is
    /// what keeps the common case — a folder viewed twice in a row — from writing every row back a
    /// second time.
    pub fn of(resolution: &Resolution) -> Option<Self> {
        let song = resolution.song()?;
        let asked = &resolution.asked;
        let unchanged = song.number == asked.code
            && asked.content_hash == song.content_hash
            && asked.package_id.as_deref() == Some(song.package_id.as_str());
        (!unchanged).then(|| Self {
            was: asked.code,
            now: song.number,
            package_id: Some(song.package_id.clone()),
            content_hash: song.content_hash.clone(),
        })
    }
}

/// The offline app's own collection. Absent in the online mode.
#[async_trait::async_trait]
pub trait Favorites: Send + Sync + 'static {
    /// Every folder, with counts, ordered by name.
    async fn folders(&self) -> Result<Vec<FolderRow>, RemoteError>;

    /// One folder.
    async fn folder(&self, id: i64) -> Result<Option<FolderRow>, RemoteError>;

    /// Make a folder, or find the one that is already called this.
    ///
    /// Create-or-find rather than create-or-fail, so that filing a song into a new folder is one
    /// action even when somebody made that folder a moment ago in another tab. Returns whether it
    /// was made.
    async fn ensure_folder(&self, name: &str) -> Result<(FolderRow, bool), RemoteError>;

    /// Rename one.
    async fn rename_folder(&self, id: i64, name: &str) -> Result<(), RemoteError>;

    /// Delete one, and everything filed in it. Refuses the last folder.
    async fn delete_folder(&self, id: i64) -> Result<(), RemoteError>;

    /// The song numbers in a folder, most recently added first.
    async fn song_ids(&self, folder: i64) -> Result<Vec<SongCode>, RemoteError>;

    /// The same rows, carrying what each favorite knows about its song.
    ///
    /// What [`Songs::resolve`] is fed. [`Self::song_ids`] stays because plenty of callers want only
    /// the numbers, and because it is the cheaper query.
    async fn song_refs(&self, folder: i64) -> Result<Vec<SongRef>, RemoteError>;

    /// Writes back what the catalog said these favorites turned out to be.
    ///
    /// **Two jobs in one write, because they are the same write.** It fills in the package and hash
    /// of a favorite filed before those were recorded — which is what arms the repair, long before
    /// anything needs repairing — and it moves a row whose song is now under a different number,
    /// which is the repair itself.
    ///
    /// **Not a removal, and there is no argument here that could express one.** A favorite the
    /// catalog could not place is left exactly as it is: this phone may simply be pointed at a
    /// machine that does not have that package today, and the same folder on the same phone
    /// tomorrow is the case that must not have been quietly emptied. That is the guarantee
    /// [`Self::add_songs`] states, and it does not stop applying because a row was hard to find.
    async fn reconcile(&self, rows: &[Reconciliation]) -> Result<(), RemoteError>;

    /// File a song, or take it back out. Returns whether it is now in.
    async fn toggle(&self, folder: i64, song: SongCode) -> Result<bool, RemoteError>;

    /// File several songs at once, adding only.
    ///
    /// **The one write in this trait that cannot take anything away**, and that is what the two
    /// features built on it rest on: a folder carried between two phones as a code, and a whole
    /// collection restored from a file, need no confirmation and no undo, because running either
    /// twice is the same insert again. The guarantee is the shape of this API rather than a promise
    /// in a comment — there is no argument here that could express a removal.
    ///
    /// Returns how many were **new**, which is the only number a page can report honestly: the rest
    /// were already there, and the difference between the two is what somebody watching wants to
    /// know.
    ///
    /// One transaction, so a folder half-filed cannot exist, and `INSERT OR IGNORE` per row, so a
    /// song already filed costs nothing and raises nothing. **The folder is checked first**, because
    /// `OR IGNORE` does not cover a foreign key: a `folder_id` naming a folder that is gone is
    /// refused by the constraint rather than ignored, so the whole transaction would fail on the
    /// first row with a database fault where [`RemoteError::NotFound`] is what a page can say.
    async fn add_songs(&self, folder: i64, songs: &[SongCode]) -> Result<usize, RemoteError>;

    /// Take a song out of a folder, whether or not it was in.
    async fn remove(&self, folder: i64, song: SongCode) -> Result<(), RemoteError>;

    /// Which folders a song is in.
    async fn folders_for_song(&self, song: SongCode) -> Result<Vec<i64>, RemoteError>;

    /// Which of these songs are a favorite of anything.
    ///
    /// One query for a page of rows rather than one per star — the shape `favdb.FavoritedIDs` landed
    /// on, and the difference between one round trip and fifty.
    async fn favorited(&self, songs: &[SongCode]) -> Result<HashSet<SongCode>, RemoteError>;
}
