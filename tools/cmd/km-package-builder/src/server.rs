//! The web server: shared state and the router.
//!
//! The state is a slot holding zero or one open folder — a [`Workspace`], which is two connections
//! on one database plus whatever scan happens to be running over it. What the slot adds is that the
//! folder is *chosen on a page* rather than named on the command line, so there is a moment before
//! one is open and there can be a swap to another.
//!
//! **Two connections, because one cannot both commit a batch and draw a page.** The number of people
//! using this tool is one and was never the question: a scan is a writer that holds its connection
//! for as long as committing hundreds of scattered rows takes, and a whole corpus is a great many of
//! those in a row. So a page is drawn through a connection that can only read, where write-ahead
//! logging lets it see the last committed state without waiting; anything that writes queues for the
//! other one, and the scan stands aside between batches so that the queue moves — an unfair mutex
//! would otherwise let the writer keep it for the whole run. A write that still cannot have it is
//! told to ask again rather than left waiting.
//!
//! Every handler that touches SQLite does so inside [`State::reading`] or [`State::blocking`], each
//! of which hops onto a blocking thread first. Locking a `std::sync::Mutex` across an `await` would
//! be a deadlock waiting to happen, and parsing a MIDI file on the async runtime's worker would
//! stall every other request.
//!
//! **Which door a handler takes is decided by what its closure does and not by the method it
//! answers.** A `GET` that rebuilds an index writes; a `POST` that writes a backup file only reads.
//! The signatures catch the obvious half — [`State::reading`] hands out `&Db`, so a closure needing
//! the connection exclusively will not compile there — and the read-only connection catches the
//! rest, because SQLite refuses a write on it outright.
//!
//! Handlers know nothing about the folder being optional. [`require_workspace`] redirects anything
//! that needs one to the Open page, and the single error variant that can still escape it —
//! [`DbError::NoWorkspace`] — becomes the same redirect in `handlers::failure`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use km_api::discover::known::Known;

use crate::app::Client;
use crate::db::{Db, DbError};
use crate::handlers;
use crate::recent::Recent;
use crate::scan::Progress;
use crate::workspace::Workspace;

/// The page's script and stylesheet, compiled into the binary.
///
/// The templates were always compiled in — half of why askama was chosen — but these three traveled
/// as files beside the executable, served by `ServeDir`, and that asymmetry was the tool's most
/// avoidable failure: 58 KB that had to arrive with the exe, could be separated from it by any copy,
/// and whose absence left every button on every page inert. `km-api` embeds its dev remote for
/// exactly this reason, and this is the same rule applied to the same kind of file.
///
/// The vendoring itself is unchanged, and so is the reason for it: nothing is fetched from the
/// network at run time, because a machine with a corpus on it may have no internet and an
/// unreachable CDN would break the page with no explanation. `tools/cmd/km-package-builder/static/README.md`
/// records the htmx version and how to update it.
const HTMX_JS: &str = include_str!("../static/htmx.min.js");
const HTMX_LICENSE: &str = include_str!("../static/htmx-LICENSE.txt");
const STYLE_CSS: &str = include_str!("../static/style.css");

/// The tool's own script — the only one, and the only file in `static/` that is not vendored.
///
/// A hundred lines against htmx's fifty kilobytes, and it does one job: turn a failed request into a
/// line of text on the screen. htmx will not swap a non-2xx response, by design, so without this a
/// page turn that hit a database error changed nothing at all and looked identical to a page turn
/// that had nowhere to go. See the head of `static/ui.js`.
///
/// **What it is allowed to grow into is nothing.** No client-side model, no templating in the
/// browser, no state that the server does not already hold — those are the properties that make
/// every page here one askama template with no second version of the truth in it. This is the
/// exception that was argued for, not the end of the rule.
///
/// **What a gesture remembers is not that state**, and the shift-click that ticks a run of rows is
/// the case worth naming: it holds the box the last press was on, which is the browser's own account
/// of what just happened rather than a second copy of anything the server knows. The selection it
/// helps make is still the ticked boxes themselves, read off the page by `hx-include`.
const UI_JS: &str = include_str!("../static/ui.js");

/// This tool's icon, for the browser tab.
///
/// Not vendored into `static/` — this reaches straight into the generated `icon/` directory, which is
/// the one place the icon is defined. A copy under `static/` would be a second file to remember to
/// update, and the curation tool showing last month's icon is exactly the sort of drift that goes
/// unnoticed for a year.
///
/// **The blue mark, not the machine's amber one.** Somebody curating a corpus usually has the
/// machine's own page open in another tab, and a favicon is most of what a tab is; two identical
/// ones make the tab strip unreadable. See `crates/playback/km-display/examples/icon.rs`.
const ICON_PNG: &[u8] = include_bytes!("../../../../icon/km-package-builder-32.png");

/// The machine's icon, used only to assert this tool's is not it. See the test below.
#[cfg(test)]
const MACHINE_ICON_PNG: &[u8] = include_bytes!("../../../../icon/icon-32.png");

/// Serves one embedded file with the content type a browser needs to honor it.
///
/// The type is not a formality: a stylesheet served as `text/plain` is ignored by every browser, and
/// `ServeDir` used to work this out from the extension.
fn embedded(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, content_type)], body)
}

/// The same, for a file that is not text.
fn embedded_bytes(content_type: &'static str, body: &'static [u8]) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, content_type)], body)
}

// **The machine this tool would install into lives in [`crate::chosen`]**, as one `Known` in the
// workspace database. Three settings here -- `app_url`, `app_machine_id` and `app_machine_name`
// -- would be a record with the type taken off it; that module carries such settings over once and
// then owns the answer. The storage is the workspace's: a `.kmbuild` is a document, and a second
// computer opening the same corpus should reach the same machine.

/// Where the karaoke app is assumed to be until somebody says otherwise.
#[cfg(not(test))]
pub const DEFAULT_APP_URL: &str = "http://127.0.0.1:8177";

/// ...and where a test run looks, which is a port nothing can ever be listening on.
///
/// **A test that drew a page reached the real machine of whoever was building.** Every page that
/// renders the machine panel asks [`crate::app::Client::discover`] first, and an answer is taken:
/// `State::machine_answered` writes the id the *reply* carried over the one the test recorded, so a
/// suite run on a box with a machine at `8177` renders somebody else's identity and the test reads
/// as a broken page. It passes alone, it passes in the next run, and it names an assertion that has
/// nothing to do with the cause — which is the shape that costs an afternoon.
///
/// **Port 0 rather than an unused one**, because an unused port is a race and this is not: 0 is the
/// sentinel that asks the operating system to choose, so nothing can bind it and nothing can answer
/// on it. A connection is refused at once rather than waiting out a timeout, so the guard costs the
/// suite nothing.
///
/// This is [`crate::passwords::path`]'s guard one concern over, with the same reasoning and the same
/// limit: `cfg(test)` is per crate, so an **integration** test links this library without it. Nothing
/// under `tests/` renders a page that discovers today, so the guard holds for every caller there is.
#[cfg(test)]
pub const DEFAULT_APP_URL: &str = "http://127.0.0.1:0";

/// The setting holding the song most recently sent to the karaoke machine.
///
/// A setting rather than a field on the server, so the highlight survives closing the browser and
/// restarting the tool. Curating is a session that spans days, and "which one did I last try?" is a
/// question asked at the start of one as often as in the middle.
pub const LAST_PLAYED_SETTING: &str = "last_played";

/// A matching page's five narrowing controls, in its query string's spellings. Empty is *any*, and
/// for `versions` it is one row per recording.
///
/// One type for both matching pages, because the controls and their spellings are the same decision.
/// Each page keeps its own, because what they open at is not — see [`Self::for_words`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarNarrowing {
    pub suitability: String,
    pub kind: String,
    pub granularity: String,
    pub copies: String,
    pub versions: String,
}

/// Suitability opens at 8–10 and the other four at *any*. A match is looked for to find a better
/// file of a song, and a file under 8 is seldom that one.
impl Default for SimilarNarrowing {
    fn default() -> Self {
        Self {
            suitability: crate::db::SuitabilityFilter::High.as_str().to_owned(),
            kind: String::new(),
            granularity: String::new(),
            copies: String::new(),
            versions: String::new(),
        }
    }
}

impl SimilarNarrowing {
    /// What the same-words page opens at: every control at *any*.
    ///
    /// **Suitability included, which is where the two pages part.** A similar *name* is looked for
    /// among files somebody might play, so it opens at the band worth playing. The same *words*
    /// under a different name is most often a file nothing else could reach — `EARTHW~2`, a rough
    /// transcription, a copy somebody saved badly — and those are the files that score under 8. A
    /// page that opened at 8–10 would hide the matches it exists to find, and say *no other song
    /// sings these words* while one sat behind the band.
    pub fn for_words() -> Self {
        Self {
            suitability: String::new(),
            kind: String::new(),
            granularity: String::new(),
            copies: String::new(),
            versions: String::new(),
        }
    }
}

/// How long a request may wait for the connection that writes before it is told to ask again.
///
/// **The same five seconds as `busy_timeout`, deliberately.** That pragma is how long this program
/// waits for a lock SQLite holds; this is how long it waits for the lock Rust holds, and two
/// different answers to one question — *how long is too long to keep somebody waiting?* — is how
/// they come to disagree.
///
/// Long enough to cover any contention between two page requests, and far short of the batch a scan
/// commits over a whole corpus. So during a scan a write does not silently queue: it either gets in
/// between batches or comes back as [`DbError::Busy`].
const WRITE_WAIT: Duration = Duration::from_secs(5);

/// How long a page request may wait for the connection it is drawn through.
///
/// The same number, because it is the same judgement. A dedicated reading connection is contended
/// only by another page, which is milliseconds; where a database could not have a second connection
/// this is a read queued behind a scan, and it is one of the waiters the scan stands aside for.
const READ_WAIT: Duration = WRITE_WAIT;

/// Everything the handlers share.
///
/// **The open folder is a slot, not a field.** Until the tool learned to be opened without one, the
/// database was built in `main` and lived as long as the process; now it is chosen from a page and
/// can be swapped for another. What that costs is one indirection and one rule, both here:
///
/// `Arc<RwLock<Option<Arc<Workspace>>>>` looks like one wrapper too many and is not. The outer
/// `RwLock` is the slot; the **inner `Arc` is what lets a reader let go of the lock immediately**.
/// Every accessor clones it out under a read guard that is dropped on the next line, so no request
/// ever holds the lock across its SQLite work. Without that, a swap would queue behind a multi-minute
/// migration, and the Scan page's polling would starve the writer indefinitely — `std::sync::RwLock`
/// promises no writer preference. It also means a swap is safe while requests are in flight: they
/// finish against the workspace they started with, and it closes when the last of them lets go.
#[derive(Clone)]
pub struct State {
    /// The folder that is open, if one is.
    open: Arc<RwLock<Option<Arc<Workspace>>>>,
    /// The folder being opened, if one is.
    opening: Arc<Mutex<Option<Arc<Opening>>>>,
    /// The folders this machine has curated before.
    recent: Arc<Mutex<Recent>>,
    /// The admin token for the machine in force, once somebody has signed in.
    ///
    /// **Here rather than in the `Client`, because the client is per request.** [`Self::app_client`]
    /// builds a fresh one each time — it has to, since which address to use is a read of the
    /// database and of what the network is saying — so a token owned by the client would be dropped
    /// between the Settings form that obtained it and the Install button that needs it. Every client
    /// of this run is handed this slot instead; see [`Client::sharing_token`].
    ///
    /// **In memory only, and this is the half that stays that way** now that a password can be
    /// remembered: `crate::passwords` holds the password, and a token is what the password is
    /// exchanged for at the moment one is wanted.
    token: Arc<Mutex<Option<String>>>,
    /// The machine `--machine` named, which holds for this run and is never written down.
    ///
    /// **Ahead of the workspace's record and the per-user seed**, and neither is touched while it is
    /// set: a run started with an address for a test must not replace the machine somebody normally
    /// uses. The Settings form clears it, because an address chosen there is a real choice. See
    /// `Where the package builder keeps the machine it was told about` in
    /// `docs/decisions/curation.md`.
    pinned: Arc<RwLock<Option<Known>>>,
    /// The machine passwords this computer was told to remember, by machine id.
    ///
    /// Read once at startup, as [`Recent`] is, and written through when a box is ticked or a
    /// password forgotten. See [`crate::passwords`] for why this is not in the workspace database.
    passwords: Arc<Mutex<crate::passwords::Remembered>>,
    /// What this curator has told the tool -- the suggested tag vocabulary, and nothing else yet.
    ///
    /// Read at startup and written back only when the Settings page saves -- see
    /// [`crate::settings`]. A `RwLock` and not a `Mutex` because it is read on every render of two
    /// pages and written about once a year.
    settings: Arc<RwLock<crate::settings::Settings>>,
    /// Asked when a page presses Quit.
    shutdown: Arc<tokio::sync::Notify>,
    /// Where the page is being served, so the header can offer to open it somewhere else.
    ///
    /// Set once, in `start`, after the socket is bound — which is also the only moment it is known,
    /// since port 0 means *whatever is free*.
    url: Arc<RwLock<String>>,
    /// Whether the page is inside this tool's own window rather than in a browser.
    ///
    /// **Set by the window itself, not by the flags that asked for one.** The two are not the same:
    /// a build with the `desktop` feature falls back to a browser when the webview cannot be created
    /// — on Windows Server, or an LTSC build with no WebView2 — and that run has to keep the Quit
    /// button, having nowhere else to press Ctrl-C. So `desktop::run` sets this after the webview is
    /// built and never before.
    windowed: Arc<AtomicBool>,
    /// The filter the songs page was last looking through, as a query string.
    ///
    /// **So that leaving the page and coming back is not a reset.** The filter already survived a
    /// reload, a bookmark and the back button, because `/songs/rows` pushes it into the address bar;
    /// what it did not survive was the nav, which is seven bare hrefs. Somebody who narrows a corpus
    /// of hundreds of thousands of songs to one folder, goes to look at a package and comes back was
    /// handed the whole corpus again.
    ///
    /// **In memory and nowhere else**, which is the choice worth writing down. A `.kmbuild` sits
    /// beside the corpus so that a second machine pointed at the same drive picks up where the first
    /// left off (`Curation database`), and where somebody's cursor happens to be is not that kind of
    /// fact. It is not worth a file of its own either: a filter is worth keeping for as long as the
    /// folder it names is open and no longer, which is why [`State::publish`] and
    /// [`State::close_folder`] both empty it.
    ///
    /// **A filter somebody has given a name to is the other kind and is in the database**, in
    /// `saved_filters`. This slot is where the cursor is, and the two are not the same fact: see
    /// `A filter can be given a name, and then it is not the cursor` in `docs/decisions/curation.md`.
    ///
    /// Empty means no filter, which is also what a fresh start says, so there is no `Option` here to
    /// distinguish two states that render identically.
    songs_filter: Arc<RwLock<String>>,
    /// The songs the Songs page was asked to hint, best first, position being the number on the row.
    ///
    /// **A `Vec` rather than a map, because the numbering is the storage.** Position plus one is
    /// what a row draws, so there is nothing to keep in step with the order.
    ///
    /// **In memory rather than in the corpus database**, for [`Self::songs_filter`]'s reason at its
    /// strongest: a filter is worth remembering for as long as the tool is open, and which four
    /// files somebody is about to play through is worth less than that. It is also why swapping the
    /// folder empties it — the ids belong to the corpus that was open.
    quality_hint: Arc<RwLock<Vec<String>>>,
    /// The four narrowing controls the similar-names page was last set to.
    ///
    /// **So a ≈ link opens with them.** Curating is a run through many songs looking for the same kind
    /// of file, and a link carries only the name. **In memory, and kept when the folder changes**,
    /// unlike [`Self::songs_filter`]: a band of suitability or a media type names nothing out of a
    /// corpus, so it means the same in the next one.
    similar_narrowing: Arc<RwLock<SimilarNarrowing>>,
    /// The same five, as the same-words page was last set to.
    ///
    /// **A second cell and not the one above.** The two bars draw the same controls and mean the
    /// same things by them, but they open differently — see [`SimilarNarrowing::for_words`] — and one
    /// record would make the first page opened decide what the other page opens at. Kept for the run
    /// and across a change of folder, for [`Self::similar_narrowing`]'s reason.
    words_narrowing: Arc<RwLock<SimilarNarrowing>>,
    /// The machines advertising themselves, kept current in the background.
    ///
    /// **`None` until [`crate::start`] opens it**, and that is the guard rather than a `cfg(test)`: this
    /// type is built by a great many tests and `Watcher::start` opens a multicast socket, which is
    /// exactly what `CONTRIBUTING.md`'s *No test binds a non-loopback address* forbids. Opening it
    /// after the bind means a test that never serves never listens, and the discover handler answers
    /// an empty list — which is the same answer it gives on a network with no mDNS.
    watcher: Arc<RwLock<Option<km_api::discover::watch::Watcher>>>,
    /// What language to draw in while the settings name none, negotiated from the browser.
    ///
    /// **In memory, and never written.** A tag in `settings.json` is a choice somebody made; this is
    /// what their browser happened to ask for, and persisting it would turn whichever browser opened
    /// the tool first into a decision nobody took. [`require_workspace`] fills it, because that
    /// middleware runs before every route and is the only place a request's headers are in reach —
    /// which is what keeps `Accept-Language` out of eighty render call sites.
    negotiated: Arc<RwLock<km_locale::Locale>>,
}

impl State {
    /// Builds the shared state with a folder already open.
    pub fn new(db: Db) -> Self {
        let state = Self::empty();
        state.publish(Arc::new(Workspace::new(db)));
        state
    }

    /// Builds the shared state with nothing open, which is what the Open page is for.
    pub fn empty() -> Self {
        Self {
            open: Arc::new(RwLock::new(None)),
            opening: Arc::new(Mutex::new(None)),
            recent: Arc::new(Mutex::new(Recent::load())),
            token: Arc::new(Mutex::new(None)),
            pinned: Arc::new(RwLock::new(None)),
            passwords: Arc::new(Mutex::new(crate::passwords::load())),
            settings: Arc::new(RwLock::new(crate::settings::Settings::load())),
            shutdown: Arc::new(tokio::sync::Notify::new()),
            url: Arc::new(RwLock::new(String::new())),
            windowed: Arc::new(AtomicBool::new(false)),
            songs_filter: Arc::new(RwLock::new(String::new())),
            quality_hint: Arc::new(RwLock::new(Vec::new())),
            similar_narrowing: Arc::new(RwLock::new(SimilarNarrowing::default())),
            words_narrowing: Arc::new(RwLock::new(SimilarNarrowing::for_words())),
            watcher: Arc::new(RwLock::new(None)),
            negotiated: Arc::new(RwLock::new(km_locale::Locale::default())),
        }
    }

    /// Starts listening for machines. Called once, from [`crate::start`], after the socket is bound.
    pub fn start_watching(&self) {
        if let Ok(mut slot) = self.watcher.write() {
            slot.get_or_insert_with(km_api::discover::watch::Watcher::start);
        }
    }

    /// The machines advertising themselves right now. Empty before [`Self::start_watching`].
    pub fn machines_seen(&self) -> Vec<km_api::discover::Sighting> {
        self.watcher
            .read()
            .ok()
            .and_then(|slot| {
                slot.as_ref()
                    .map(km_api::discover::watch::Watcher::snapshot)
            })
            .unwrap_or_default()
    }

    /// Ask the network again now, because somebody just pressed the button.
    pub fn look_again(&self) {
        if let Ok(slot) = self.watcher.read()
            && let Some(watcher) = slot.as_ref()
        {
            watcher.poke();
        }
    }

    /// Records the machine answering at the current address, so a move can be recognized later.
    ///
    /// **Written only from a `/discover` that answered at the address in force**, so an address
    /// somebody named cannot inherit the previous machine's identity and be dragged off to wherever
    /// it went. The name and the time travel with it, so the header can name the machine without
    /// asking it and the Settings page can say how long it has been quiet.
    ///
    /// A pinned machine takes the identity in memory and nothing reaches the database.
    pub async fn machine_answered(&self, id: &str, name: &str) {
        let id = id.to_owned();
        let name = name.to_owned();
        {
            let mut pinned = self
                .pinned
                .write()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(known) = pinned.take() {
                *pinned = Some(known.answered(
                    &id,
                    Some(name).filter(|name| !name.is_empty()),
                    std::time::SystemTime::now(),
                ));
                return;
            }
        }
        let _ = self
            .blocking(move |db| {
                let Some(url) = crate::chosen::load(db)?.map(|known| known.url) else {
                    return Ok(());
                };
                crate::chosen::answered(db, &url, &id, Some(name).filter(|name| !name.is_empty()))
            })
            .await;
    }

    /// The machine this workspace was told about, or this computer's last choice for one that never
    /// has been.
    ///
    /// **No network at all**, which is the constraint every caller of this shares: the header is
    /// drawn on every page, and a machine that is switched off is the normal state of affairs here.
    /// **Two steps, because only the first of them can happen on every page.** Reading what a
    /// workspace was told is a read, and the header does it on every render; seeding one that has
    /// never been told is a write, and happens once in a folder's life. Asking for the writing
    /// connection for both would put every page in the tool behind a scan's batch, to answer a
    /// question that is already written down.
    pub async fn chosen_machine(&self) -> Option<Known> {
        if let Some(known) = self.pinned_machine() {
            return Some(known);
        }
        if let Ok(Some(known)) = self.reading(crate::chosen::load).await {
            return Some(known);
        }
        // A workspace that has never been told takes this computer's last hand-set choice, rather
        // than starting on a loopback address with nothing on it. Written down here so the next read
        // is the ordinary path above. See `crate::chosen`.
        let seed = crate::chosen::seed()?;
        self.blocking(move |db| {
            crate::chosen::save(db, &seed.url)?;
            crate::chosen::load(db)
        })
        .await
        .ok()
        .flatten()
    }

    /// Points this run at a machine without writing it anywhere. What `--machine` does.
    pub fn pin_machine(&self, url: &str) {
        let mut pinned = self
            .pinned
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *pinned = Some(Known::at(url, km_api::discover::known::Why::Chosen));
    }

    /// Drops the run's pin, so the workspace's record is in force again.
    pub fn unpin_machine(&self) {
        let mut pinned = self
            .pinned
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *pinned = None;
    }

    /// The machine `--machine` named, if this run was given one and nobody has chosen since.
    pub fn pinned_machine(&self) -> Option<Known> {
        self.pinned
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Which machine this tool would install into, as the header prints it.
    ///
    /// `None` for a workspace that has never been told one, which is what the header says as *no
    /// machine set*. The address is whatever is written down; the name is what that machine last
    /// said about itself, absent until something has answered.
    ///
    /// It deliberately does **not** follow a move the way [`Self::app_client`] does. That follow
    /// writes, and a header rendered on every page is not a place to be writing; the next request
    /// that actually talks to the machine will do it, and the header is then right.
    pub async fn machine_shown(&self) -> Option<(String, Option<String>)> {
        let known = self.chosen_machine().await?;
        Some((known.url, known.name.filter(|name| !name.trim().is_empty())))
    }

    /// Records where the page is being served. Called once, from `start`.
    pub fn set_url(&self, url: impl Into<String>) {
        if let Ok(mut slot) = self.url.write() {
            *slot = url.into();
        }
    }

    /// Where the page is being served, or empty before the socket is bound.
    pub fn url(&self) -> String {
        self.url.read().map(|url| url.clone()).unwrap_or_default()
    }

    /// Says that the page is in this tool's own window. See the field's own note for who calls it.
    ///
    /// Behind the feature because `desktop::run` is its only caller and is the only thing that can
    /// honestly call it — there is no window without the feature, so a build that could set this has
    /// nothing to set it about. The *reader* is not gated: every build draws the header, and a build
    /// with no window answers `false` for the plain reason that it has none.
    #[cfg(feature = "desktop")]
    pub fn set_windowed(&self) {
        self.windowed.store(true, Ordering::Relaxed);
    }

    /// Whether the page is in this tool's own window rather than a browser.
    ///
    /// `Relaxed` because this is one flag written once, before the window's first navigation, and
    /// read only to decide which of two buttons the header draws. Nothing is ordered against it.
    pub fn is_windowed(&self) -> bool {
        self.windowed.load(Ordering::Relaxed)
    }

    /// The open workspace, if there is one.
    ///
    /// Clones the `Arc` out and drops the guard — see the note on [`State`] for why that ordering is
    /// the whole design and not a detail.
    pub fn workspace(&self) -> Option<Arc<Workspace>> {
        // A poisoned lock means a handler panicked while holding it. The data behind it is a slot
        // holding an `Arc`, which a panic cannot leave half-written, so recovering is strictly better
        // than making every later request fail too. In a window with no console, the alternative is an
        // application that is silently dead.
        let guard = self.open.read().unwrap_or_else(|error| error.into_inner());
        guard.clone()
    }

    /// The folder being curated, if one is.
    pub fn root(&self) -> Option<PathBuf> {
        self.workspace().map(|ws| ws.root().to_path_buf())
    }

    /// Puts a workspace in the slot, closing whatever was there.
    ///
    /// Closing is [`Workspace`]'s own `Drop`, which is why this is an assignment and not a sequence.
    ///
    /// **The songs filter is dropped rather than carried**, and that is not tidiness. A filter names
    /// a folder and a favorite id out of the corpus it was built against; carried into the next one
    /// it describes rows that do not exist, so the Songs tab would lead somewhere empty and the
    /// chips would explain it in the vocabulary of a corpus nobody is looking at.
    fn publish(&self, workspace: Arc<Workspace>) {
        self.remember_songs_filter(String::new());
        // The hint is a list of content hashes out of the folder being closed, and a hash that
        // happened to exist in both corpora would draw a number on a row nobody ticked.
        self.clear_quality_hint();
        let mut guard = self.open.write().unwrap_or_else(|error| error.into_inner());
        *guard = Some(workspace);
    }

    /// What the songs page was last looking through, as a query string with no leading `?`.
    ///
    /// Empty for a tool that has not been to the page yet, and for one that has cleared its filter.
    pub fn songs_filter(&self) -> String {
        self.songs_filter
            .read()
            .map(|filter| filter.clone())
            .unwrap_or_default()
    }

    /// Writes down the filter the songs page is looking through.
    ///
    /// A poisoned lock is recovered rather than propagated, for the reason [`State::workspace`]
    /// gives — and this one is a convenience on top of that: losing a remembered filter is not worth
    /// failing a request over.
    pub fn remember_songs_filter(&self, filter: String) {
        let mut guard = self
            .songs_filter
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *guard = filter;
    }

    /// What the similar-names page was last narrowed by. Every field empty until somebody sets one.
    pub fn similar_narrowing(&self) -> SimilarNarrowing {
        self.similar_narrowing
            .read()
            .map(|narrowing| narrowing.clone())
            .unwrap_or_default()
    }

    /// Writes down what the similar-names page is narrowed by, empty fields included, so a control
    /// set back to *any* stays that way.
    /// What the same-words page was last narrowed by.
    pub fn words_narrowing(&self) -> SimilarNarrowing {
        self.words_narrowing
            .read()
            .map(|narrowing| narrowing.clone())
            .unwrap_or_else(|_| SimilarNarrowing::for_words())
    }

    /// Writes down what the same-words page is narrowed by, empty fields included.
    pub fn remember_words_narrowing(&self, narrowing: SimilarNarrowing) {
        let mut guard = self
            .words_narrowing
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *guard = narrowing;
    }

    pub fn remember_similar_narrowing(&self, narrowing: SimilarNarrowing) {
        let mut guard = self
            .similar_narrowing
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *guard = narrowing;
    }

    /// The songs this run was asked to hint, best first. Empty until somebody asks.
    pub fn quality_hint(&self) -> Vec<String> {
        self.quality_hint
            .read()
            .map(|hint| hint.clone())
            .unwrap_or_default()
    }

    /// Replaces the hint with a new one, and answers with what it replaced.
    ///
    /// The old list is what the clearing marks are sent for: a row that carried a number and is no
    /// longer in the hint has to be told, and only the list being replaced knows which rows those
    /// are.
    pub fn set_quality_hint(&self, hinted: Vec<String>) -> Vec<String> {
        let mut guard = self
            .quality_hint
            .write()
            .unwrap_or_else(|error| error.into_inner());
        std::mem::replace(&mut guard, hinted)
    }

    /// Takes the hint away, answering with what it held.
    pub fn clear_quality_hint(&self) -> Vec<String> {
        self.set_quality_hint(Vec::new())
    }

    /// Writes each row's place in the hint onto it, for a page about to be drawn.
    ///
    /// **Here rather than in the browse query**, because a hint is a fact about this run and not
    /// about the corpus — the same reason the filter it sits beside is not a column either. A row
    /// in no hint keeps `None` and draws an empty badge.
    pub fn mark_hints(&self, rows: &mut [crate::model::SongRow]) {
        let hinted = self.quality_hint();
        if hinted.is_empty() {
            return;
        }
        for row in rows.iter_mut() {
            row.hint = hinted
                .iter()
                .position(|id| *id == row.id)
                .map(|at| at as u32 + 1);
        }
    }

    /// Words each row's four sentences that carry a value, for a page about to be drawn.
    ///
    /// [`Self::mark_hints`]'s sibling, and beside it for the same reason: what a row *says* is a
    /// fact about the page rather than about the corpus, and the query that reads a row has no
    /// language in reach. `picking` is whether the favorite chooser is open, which changes what the
    /// favorites button offers.
    pub fn say_rows(&self, rows: &mut [crate::model::SongRow], picking: bool) {
        let locale = self.locale();
        for row in rows.iter_mut() {
            row.say(locale, picking);
        }
    }

    /// The same, for a row swapped back on its own after an edit.
    ///
    /// Without it, scoring a song would rub out the number the hint had put on it — which reads as
    /// the edit having done something it did not.
    pub fn mark_hint(&self, row: &mut crate::model::SongRow) {
        self.mark_hints(std::slice::from_mut(row));
    }

    /// [`Self::say_rows`] for one row, which the three single-row routes draw.
    pub fn say_row(&self, row: &mut crate::model::SongRow, picking: bool) {
        self.say_rows(std::slice::from_mut(row), picking);
    }

    /// Runs a closure against the database on a blocking thread, through the **writing** connection.
    ///
    /// The lock is taken and released inside the closure, so it is never held across an `await`.
    ///
    /// The signature is unchanged from when there was always a database, which is what let this
    /// become swappable without touching any of the forty-odd call sites: the one new outcome,
    /// [`DbError::NoWorkspace`], travels the error path every one of them already has.
    ///
    /// **Anything that only reads belongs on [`Self::reading`] instead.** A scan holds this
    /// connection for a whole batch, so what stays here waits for one.
    ///
    /// **And waits only so long**: past [`WRITE_WAIT`] it gives up with [`DbError::Busy`], which
    /// travels the same error path `NoWorkspace` does. A write that cannot have the connection has
    /// to say so, because the alternative is what this whole arrangement exists to remove — a
    /// request that never answers and a browser that runs out of sockets holding it open.
    pub async fn blocking<T, F>(&self, work: F) -> Result<T, DbError>
    where
        F: FnOnce(&mut Db) -> Result<T, DbError> + Send + 'static,
        T: Send + 'static,
    {
        let workspace = self.workspace().ok_or(DbError::NoWorkspace)?;
        let db = Arc::clone(&workspace.db);
        tokio::task::spawn_blocking(move || {
            let mut guard = db.lock_within(WRITE_WAIT).ok_or(DbError::Busy)?;
            work(&mut guard)
        })
        .await
        .unwrap_or_else(|error| {
            Err(DbError::Rejected(format!(
                "the worker thread died: {error}"
            )))
        })
    }

    /// The same, through the **reading** connection, for a closure that only reads.
    ///
    /// **This is what a page uses, and why a scan no longer stops the tool.** A scan's writer holds
    /// [`Self::blocking`]'s connection for a whole batch; this one is a second connection on the same
    /// file, so a render sees the last committed state and waits for nothing.
    ///
    /// **`&Db` and not `&mut Db`**, which is the part that keeps the split honest. Every method that
    /// needs a transaction takes `&mut self`, so a closure that writes one will not compile here and
    /// has to say so by asking for `blocking` instead. It is a filter rather than a proof — a handful
    /// of writes go through `&self` — and the connection itself is the proof: SQLite refuses a write
    /// on it outright, so a mis-routed closure fails loudly on the first attempt rather than quietly
    /// doing the wrong thing.
    pub async fn reading<T, F>(&self, work: F) -> Result<T, DbError>
    where
        F: FnOnce(&Db) -> Result<T, DbError> + Send + 'static,
        T: Send + 'static,
    {
        let workspace = self.workspace().ok_or(DbError::NoWorkspace)?;
        let db = Arc::clone(workspace.reader());
        tokio::task::spawn_blocking(move || {
            let guard = db.lock_within(READ_WAIT).ok_or(DbError::Busy)?;
            work(&guard)
        })
        .await
        .unwrap_or_else(|error| {
            Err(DbError::Rejected(format!(
                "the worker thread died: {error}"
            )))
        })
    }

    /// A client for the karaoke app, at whatever address is configured — or wherever it has moved.
    ///
    /// **The follow is `known::choose`, and this tool no longer has a policy of its own.** It had
    /// one — a `moved_to` that followed the remembered id whenever it appeared at a different
    /// address — and `km-admin` had a third. Three implementations of one rule is exactly the drift
    /// `choose` was written to remove, and two of the three disagreed with it about something real:
    /// rule 2 *stays put* when the address in hand is answering and reports the same id, which is
    /// one machine with two addresses and the one in hand working.
    ///
    /// **`adopts: false` is the whole of *listing is not setting*.** Rule 3 and the nothing-in-hand
    /// tail are the two answers that would point this tool at a machine nobody named, and this tool
    /// installs packages. Following an id is not adopting and runs regardless — the exception
    /// `Discovering a machine in the package builder` already grants.
    ///
    /// **`online: false` and `answering_id: None` are honest rather than lazy.** This tool holds no
    /// connection: every request builds a fresh client, so it has no fact about whether the address
    /// is answering, and rule 2 collapses to the same *take it* the old `moved_to` gave.
    ///
    /// The follow rides on the read this was already doing, against an in-memory snapshot, so the
    /// common case — nothing has moved — costs what it always did.
    ///
    /// **It holds no connection and now does hold a token**, which is not a contradiction of the
    /// paragraph above. The client is still fresh every time; what it is handed is [`Self::token`],
    /// so a sign-in outlives the request that performed it. A machine that has *moved* keeps the
    /// token, because it is the same machine and the token is its; a machine somebody *chose* clears
    /// it, in [`Self::forget_token`].
    pub async fn app_client(&self) -> Client {
        // A pin is an address the command line named, so nothing follows it anywhere.
        if let Some(pinned) = self.pinned_machine() {
            return Client::sharing_token(&pinned.url, Arc::clone(&self.token));
        }
        let Some(known) = self.chosen_machine().await else {
            return Client::sharing_token(DEFAULT_APP_URL, Arc::clone(&self.token));
        };
        let seen = self.machines_observed();
        let choice = km_api::discover::known::choose(&km_api::discover::known::Situation {
            pinned: false,
            online: false,
            current: Some(&known.url),
            known: Some(&known),
            seen: &seen,
            answering_id: None,
            stale: km_api::discover::known::is_stale(&known, std::time::SystemTime::now()),
            adopts: false,
        });

        let url = match choice {
            km_api::discover::known::Choice::Use { url, why } => {
                let write = url.clone();
                if self
                    .blocking(move |db| crate::chosen::moved(db, &write))
                    .await
                    .is_ok()
                {
                    tracing::info!(from = %known.url, to = %url, why = why.label(), "following the machine");
                    url
                } else {
                    known.url
                }
            }
            km_api::discover::known::Choice::Stay => known.url,
        };
        Client::sharing_token(&url, Arc::clone(&self.token))
    }

    /// Drops the admin token, which is what choosing another machine means.
    ///
    /// **Choosing, not moving.** `chosen::moved` follows one machine to a new address and the token
    /// is still that machine's, so it is kept; `chosen::save` is somebody naming an address, which
    /// says nothing about what is at it — and a token sent to a machine that did not issue it is a
    /// 401 with a confusing sentence attached rather than a prompt to sign in.
    pub fn forget_token(&self) {
        if let Ok(mut held) = self.token.lock() {
            *held = None;
        }
    }

    /// Whether somebody has signed in to the machine in force.
    pub fn signed_in(&self) -> bool {
        self.token.lock().is_ok_and(|held| held.is_some())
    }

    /// The password remembered for a machine, if this computer was told to remember one.
    pub fn remembered_password(&self, machine_id: &str) -> Option<String> {
        self.passwords
            .lock()
            .ok()
            .and_then(|held| held.get(machine_id).map(str::to_owned))
    }

    /// Whether anything is remembered for a machine.
    pub fn remembers(&self, machine_id: &str) -> bool {
        self.passwords
            .lock()
            .is_ok_and(|held| held.holds(machine_id))
    }

    /// Remembers a password for a machine, or forgets it, writing the file either way.
    pub fn remember_password(&self, machine_id: &str, password: Option<&str>) {
        let Ok(mut held) = self.passwords.lock() else {
            return;
        };
        match password {
            Some(password) => held.remember(machine_id, password),
            None => held.forget(machine_id),
        }
    }

    /// The id of the machine in force, which is `None` until something has answered at its address.
    ///
    /// The key `crate::passwords` is written under, and the reason a machine that has never replied
    /// cannot be remembered: there is no identity yet to key it by.
    pub async fn machine_id(&self) -> Option<String> {
        self.chosen_machine().await.and_then(|known| known.id)
    }

    /// Signs in with a remembered password, if there is one and nothing is signed in already.
    ///
    /// **The one lazy path**, called by the two handlers that need a token. A remembered password is
    /// a standing instruction to sign in, not a sign-in that has happened: the token it buys expires
    /// and the machine may have been restarted, so the moment to spend it is the moment one is
    /// wanted rather than at startup, where it would talk to the network before anybody had asked
    /// for anything.
    ///
    /// Failure is deliberately silent. The caller goes on to be refused by the machine and to show
    /// the sentence that says how to sign in, which is a better answer than a second error about a
    /// password the person may not remember setting.
    pub async fn sign_in_if_remembered(&self, client: &Client) {
        if client.logged_in() {
            return;
        }
        let Some(id) = self.machine_id().await else {
            return;
        };
        let Some(password) = self.remembered_password(&id) else {
            return;
        };
        if let Err(error) = client.log_in(&password).await {
            tracing::debug!("a remembered password was not accepted: {error}");
        }
    }

    /// What the network is saying, with presence, as [`km_api::discover::known::choose`] wants it.
    fn machines_observed(&self) -> Vec<km_api::discover::watch::Observed> {
        self.watcher
            .read()
            .ok()
            .and_then(|slot| {
                slot.as_ref()
                    .map(km_api::discover::watch::Watcher::observed)
            })
            .unwrap_or_default()
    }

    /// Whether a scan is running right now.
    ///
    /// False when nothing is open, which is the honest answer rather than a special case: the Scan
    /// page is behind the redirect and cannot be reached without a folder.
    pub fn scan_running(&self) -> bool {
        self.workspace().is_some_and(|ws| ws.scan_running())
    }

    /// The current or last scan's progress.
    pub fn scan_progress(&self) -> crate::scan::ProgressView {
        self.workspace()
            .map(|ws| ws.scan_progress())
            .unwrap_or_default()
    }

    /// Starts a scan over the open folder, and returns its progress handle.
    ///
    /// `None` when nothing is open. The scan itself belongs to the [`Workspace`] — see that type for
    /// why a scan must not be able to outlive the folder it is scanning.
    pub fn start_scan(&self, options: crate::scan::ScanOptions) -> Option<Arc<Progress>> {
        self.workspace().map(|ws| ws.start_scan(options))
    }

    /// Asks the open folder's scan to stop, without waiting for it. False when none is running.
    ///
    /// Not the `stop_scan` the note below rules out: that one joins the thread, and joining belongs
    /// to `Workspace::drop`. This only sets the flag the run already checks between files, so it
    /// closes nothing and leaves the run to end the way a finished one does.
    pub fn ask_scan_to_stop(&self) -> bool {
        self.workspace().is_some_and(|ws| ws.ask_scan_to_stop())
    }

    /// Whether a package is being built right now.
    pub fn build_running(&self) -> bool {
        self.workspace().is_some_and(|ws| ws.build_running())
    }

    /// The current or last build's progress, if there has been one.
    pub fn build_progress(&self) -> Option<crate::build::BuildProgressView> {
        self.workspace().and_then(|ws| ws.build_progress())
    }

    /// Starts a build of one package, and returns its progress handle.
    ///
    /// `None` when nothing is open. Like the scan, the build belongs to the [`Workspace`] and cannot
    /// outlive the folder it is reading — which for a build matters more, not less: it writes to the
    /// database when it finishes.
    pub fn start_build(
        &self,
        package_id: String,
        volume: u32,
        out: std::path::PathBuf,
        write_listing: bool,
    ) -> Option<Arc<crate::build::BuildProgress>> {
        self.workspace()
            .map(|ws| ws.start_build(package_id, volume, out, write_listing))
    }

    /// Starts a build of every volume of a package. `None` when nothing is open.
    pub fn start_build_all(
        &self,
        package_id: String,
        folder: Option<std::path::PathBuf>,
        write_listing: bool,
    ) -> Option<Arc<crate::build::BuildProgress>> {
        self.workspace()
            .map(|ws| ws.start_build_all(package_id, folder, write_listing))
    }

    // No `stop_build` here either, and for the reason spelled out below about the scan: stopping
    // belongs to `Workspace::drop`, so a folder cannot be closed without its build being stopped
    // first — which for a build is the difference between a recorded result and a lost one.

    // There is deliberately no `stop_scan` here. Calling one by hand on the way out of `main`, with
    // the stop available on the shared state, invites the bug this arrangement closes: a scan
    // stopped through one path and a folder swapped through another, with no rule about which
    // happens first. Stopping belongs to `Workspace::drop`, so it is impossible to close a folder
    // *without* stopping its scan, whether that close comes from Quit, Ctrl-C, or opening a
    // different corpus.

    /// How the folder currently being opened is getting on, if one is.
    pub fn opening(&self) -> Option<OpeningView> {
        let guard = self
            .opening
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.as_ref().map(|o| o.snapshot())
    }

    /// The folders this machine has curated before.
    pub fn recent(&self) -> Recent {
        self.recent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// What this curator has told the tool.
    ///
    /// Cloned rather than borrowed, so the lock is never held across an `await` — the discipline
    /// `State::blocking` keeps for the database, applied to the one other shared thing here.
    pub fn settings(&self) -> crate::settings::Settings {
        self.settings
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// What language to draw a page in.
    ///
    /// The setting where there is one, and what the browser asked for while there is not. Its own
    /// accessor rather than a read through [`Self::settings`], which clones the whole struct so that
    /// no lock is held across an `await`: every render asks this, and cloning a tag vector to read
    /// one `Copy` field is waste.
    pub fn locale(&self) -> km_locale::Locale {
        if let Some(chosen) = self
            .settings
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .locale()
        {
            return chosen;
        }
        *self
            .negotiated
            .read()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Records what this request's browser asked for, while no setting overrides it.
    ///
    /// Called from [`require_workspace`]. A header naming nothing this build has leaves the slot
    /// alone, which is `km_locale::negotiate` declining rather than guessing.
    fn negotiate(&self, header: Option<&str>) {
        let Some(asked) = header.and_then(km_locale::negotiate) else {
            return;
        };
        let mut slot = self
            .negotiated
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *slot = asked;
    }

    /// Replaces the language these pages are drawn in and writes it back.
    pub fn set_locale(&self, locale: km_locale::Locale) {
        self.settings
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .set_locale(locale);
    }

    /// Replaces the suggested tag list and writes it back. Returns what was stored.
    pub fn set_default_tags(&self, raw: &str) -> Vec<String> {
        self.settings
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .set_default_tags(raw)
    }

    /// Records a folder as the most recently opened one.
    ///
    /// Called by **both** ways of opening: the job behind the Open page, and `main` when a folder was
    /// named on the command line. The second is easy to forget and was — which made the whole
    /// reopen-what-you-had-last-time behavior dead for anyone whose habit is to name the folder,
    /// since nothing they did ever wrote the list they were going to be offered.
    ///
    /// Deliberately not inside `State::new`, tempting though that is. That constructor is what every
    /// test builds its state with, and the recent list is a real file in the user's own config
    /// directory: a test suite that remembered its scratch folders would rewrite it on every run.
    ///
    /// **Keeping it out of the constructor turned out not to be enough**, and the second half of the
    /// guard is in [`crate::recent::path`]. `begin_open` records the folder it opened, a test calls
    /// `begin_open` for what it refuses rather than for what it opens, and the spawned thread wrote
    /// the real list anyway — so the list a test holds now has nowhere to save to at all. That same
    /// function is where a run which is *not* a test and *not* curation — a screenshot pipeline, a
    /// smoke test — is sent somewhere else, by [`crate::recent::ENV_VAR`].
    pub fn remember(&self, root: &Path, songs: u32, files: u32) {
        self.recent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remember(root, songs, files);
    }

    /// Takes a folder out of the recent list.
    pub fn forget_recent(&self, root: &Path) {
        self.recent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .forget(root);
    }

    /// Begins opening a folder, on a thread, and returns at once.
    ///
    /// **Opening is a job rather than a request, and that is not gold-plating.** `Db::prepare` runs
    /// the migrations, creates the browse indexes, runs three backfills and finishes with an
    /// unbounded `ANALYZE`; on a large real corpus the songs rebuild is documented in minutes and
    /// the `ANALYZE` alone is minutes more against a cold disk. `main` has a long comment why none of
    /// that may happen before the socket is listening — a browser pointed at a tool that has not
    /// bound yet is refused, and reads as broken. Doing it inside a POST handler would reproduce
    /// exactly that fault one level up: the request would hang for minutes with nothing on screen,
    /// and on a desktop build there is no console for the migration notice to appear on either.
    ///
    /// So it runs on a `std::thread` — not `spawn_blocking`, for the same reason the scan does not:
    /// minutes of blocking CPU has no business on a runtime worker — and the page polls
    /// [`State::opening`] the way the Scan page already polls a scan.
    ///
    /// Returns the error immediately only for what can be known immediately.
    pub fn begin_open(&self, root: PathBuf, create: bool) -> Result<(), DbError> {
        if !root.is_dir() {
            return Err(DbError::Rejected(format!(
                "{} is not a folder",
                root.display()
            )));
        }
        // Whether there is a database to open is answerable now, and answering it now is what keeps
        // "you pointed at the wrong folder" an error rather than a job that starts and then fails.
        if !create {
            crate::db::require_database(&root)?;
        }

        let progress = Arc::new(Opening::new(&root));
        {
            let mut slot = self
                .opening
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(current) = slot.as_ref()
                && !current.finished()
            {
                // **Asking again for the folder already being opened is not a mistake.** It is what
                // clicking a recent folder does while the startup reopen is still loading that very
                // folder, and it is what a double-click sends. The honest answer is the job's
                // progress, which is what every caller renders on `Ok`.
                if current.path == root {
                    return Ok(());
                }
                return Err(DbError::Rejected(format!(
                    "already opening {}",
                    current.root
                )));
            }
            *slot = Some(Arc::clone(&progress));
        }

        let state = self.clone();
        std::thread::spawn(move || {
            // Whatever happens below, including a panic, ends the job. See [`EndsTheJob`].
            let job = EndsTheJob(progress);
            tracing::debug!("opening {}", root.display());
            // Where the page's message comes from while the open runs. `Db::prepare` speaks at the
            // two points it already tells a console about, so a build with no console — the windowed
            // one, which is what a double-click starts — stops being the one told least.
            let opening = Arc::clone(&job.0);
            let phase = move |saying| opening.set_phase(saying);
            // **The folder being left is closed before the next one is opened**, so this process
            // holds one connection per file. Opening a folder that is already open is an ordinary
            // act — the Recent list offers it and a second double-click sends it — and closing
            // afterwards instead would run the whole migration on a second connection to the same
            // file, then close the first with a `wal_checkpoint(TRUNCATE)` at the moment the list is
            // first asked for. A read meeting that writer answers "database is locked", and a second
            // press cures it, which is the shape a fault hides in.
            //
            // Here rather than in `publish`, and this early, for the two things it buys: the new
            // connection is alone on the file, and the close's own checkpoint is paid inside the
            // window the Open page is already reporting. The price is that an open failing *during*
            // the migration leaves nothing open — which is where a failed open lands anyway, and is
            // the honest state for a folder whose index has just been half-rewritten. Everything
            // refusable without touching a database has been refused above, before this runs.
            //
            // Said rather than done silently: the checkpoint this pays is the size of whatever the
            // folder being left had rewritten, which on a corpus is hundreds of megabytes, and the
            // seed sentence names the *next* folder's database — so a page left on it through this
            // is describing work that has not started.
            if state.root().is_some() {
                phase(crate::db::OpeningPhase::ClosingPrevious);
            }
            state.close_folder();
            // **Said here, by the caller, rather than by `Db::prepare`.** Closing the folder that
            // was open is the rung below this one and happens on this thread, so a job that starts
            // already on `Database` would have to climb backwards to report it. `Db::prepare` opens
            // a connection it was handed and has nothing to say about this.
            phase(crate::db::OpeningPhase::Database);
            let opened = if create {
                Db::create_saying(&root, &phase)
            } else {
                Db::open_saying(&root, &phase)
            };
            tracing::debug!("opened {}: ok={}", root.display(), opened.is_ok());
            match opened {
                Ok(db) => {
                    let counts = db.counts().unwrap_or_default();
                    // Published *before* the recent list is written, so the page's next poll finds a
                    // folder to go to even if writing a convenience file fails.
                    state.publish(Arc::new(Workspace::new(db)));
                    state.remember(&root, counts.songs, counts.files);
                    job.0.finish(None);
                }
                Err(error) => job.0.finish(Some(error.to_string())),
            }
        });
        Ok(())
    }

    /// Leaves a reason where the Open page draws it, for a refusal that had no request to answer.
    ///
    /// **What [`Self::begin_open`] refuses without touching a database it refuses by returning** —
    /// a folder that is not one, a folder holding no database, a folder holding two. A page that
    /// posted the folder renders that return; a start has nowhere to put it, and in a window with
    /// no console nowhere is exactly where it goes. So it is written into the slot the page already
    /// polls, as a job that is over before anybody sees it.
    ///
    /// **A job still running is left alone**, being the one thing in the slot somebody is watching.
    /// [`Self::begin_open`] already replaces a *finished* slot, so a reason left here never stands
    /// in the way of the next open.
    pub fn report_failed_open(&self, root: &Path, reason: &str) {
        let mut slot = self
            .opening
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot.as_ref().is_some_and(|current| !current.finished()) {
            return;
        }
        // Finished before the `Arc` exists, so no reader can meet a job that is half a report.
        let progress = Opening::new(root);
        progress.finish(Some(reason.to_owned()));
        *slot = Some(Arc::new(progress));
    }

    /// Closes the open folder, leaving nothing open.
    ///
    /// The checkpoint and the scan join happen in [`Workspace`]'s `Drop`, once the last reference to
    /// it goes — this thread's, or a request still in flight. The slot is empty either way before
    /// this returns, so nothing else is waiting on the work.
    pub fn close_folder(&self) {
        // A filter is of the folder it was built against, so it goes with it. Nothing reads it
        // while nothing is open, and the next folder gets its own empty one either way.
        self.remember_songs_filter(String::new());
        // **Taken out under the lock and dropped outside it.** Assigning `None` into the slot would
        // drop the workspace *while the write guard is held*, putting a `wal_checkpoint(TRUNCATE)`
        // of a migration-sized journal in front of every request waiting on `State::workspace` —
        // the Open page's own progress poll among them, which is the one page that has to keep
        // answering while a folder is being swapped.
        let leaving = {
            let mut guard = self.open.write().unwrap_or_else(|error| error.into_inner());
            guard.take()
        };
        drop(leaving);
    }

    /// Asks the server to stop, as the Quit control does.
    ///
    /// The counterpart of Ctrl-C, and needed because a desktop build has no console to press it in
    /// and a browser build may have been started by a double-click. `main` waits on the same signal,
    /// so both routes run the identical shutdown.
    pub fn ask_to_quit(&self) {
        self.shutdown.notify_waiters();
    }

    /// Waits for the Quit control to be pressed.
    pub async fn quit_requested(&self) {
        self.shutdown.notified().await;
    }
}

/// How the opening of one folder is getting on.
///
/// Deliberately much smaller than the scan's [`Progress`]: a scan has files, parses, failures and a
/// backlog to report, whereas opening has a phase and an outcome. Sharing the scan's type would have
/// meant a page full of counters that are structurally zero.
pub struct Opening {
    /// The folder being opened, for comparing against another request for the same one.
    ///
    /// Beside `root` rather than instead of it: the page wants a string and the guard above wants a
    /// path, and `Path::new(&self.root)` would be reconstructing one from its own display form.
    path: PathBuf,
    /// The folder being opened, for the message.
    root: String,
    /// What it is doing now.
    phase: Mutex<crate::db::OpeningPhase>,
    /// When it started.
    ///
    /// **The half of the panel that cannot stop moving.** A phase is a claim about a step, and the
    /// longest step here is minutes long, so between two of them there is nothing on the page to say
    /// the tool is alive. A count of seconds is the same proof of life the console's `done — N
    /// seconds` gives, and it survives an animation a browser has throttled, disabled or never
    /// painted.
    started: Instant,
    /// Every rung of the open, in order, with where each stands and how long each took.
    ///
    /// **The half the sentence and the seconds cannot cover.** A name for the running step says
    /// neither what is still to come nor whether a step showing the same words for five minutes is
    /// working, and an open's longest rung is exactly that shape. See [`crate::db::OPENING_LADDER`].
    steps: crate::step::Ladder,
    /// Whether it has ended.
    finished: AtomicBool,
    /// What went wrong, if anything.
    error: Mutex<Option<String>>,
}

impl Opening {
    /// Starts one.
    ///
    /// **No rung is open yet.** The first one is named by the worker, so that a job which closes a
    /// folder before opening one has its rungs in the order they happen. The phase behind the
    /// sentence starts at `Database` because that is what an open with nothing to close does first.
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            path: root.to_path_buf(),
            root: root.display().to_string(),
            phase: Mutex::new(crate::db::OpeningPhase::Database),
            started: Instant::now(),
            steps: crate::step::Ladder::of(crate::db::OPENING_LADDER),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }

    /// Whether it has ended, however it ended.
    fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Says what it is doing now.
    ///
    /// The one field on this type that changes between being built and being finished, and for most
    /// of this type's life nothing wrote it at all: the page polled a sentence fixed at construction
    /// and so reported "opening the database" for however many minutes an open ran. `Db::prepare`
    /// knew better the whole time and was telling the console. See `db::OpeningPhase`.
    pub(crate) fn set_phase(&self, phase: crate::db::OpeningPhase) {
        let mut slot = self.phase.lock().unwrap_or_else(|error| error.into_inner());
        *slot = phase;
        // `advance_to` and not `say`, because an open names only the rungs it takes: the ones it
        // had nothing to do are marked by being climbed past. See `crate::step::Ladder`.
        self.steps.advance_to(phase.step());
    }

    /// Records the outcome, and only the first one.
    ///
    /// **Idempotent on purpose**, because two things end a job now: the worker itself, and
    /// [`EndsTheJob`]'s `Drop` behind it. Without this, the guard would overwrite a real result and
    /// a successful open would end up carrying "opening stopped unexpectedly".
    ///
    /// The error is written *before* `finished` is published, so a page that sees a finished job
    /// sees its reason with it. The lock is what serializes the two callers, which is why no second
    /// atomic is needed to claim the right to record.
    pub(crate) fn finish(&self, error: Option<String>) {
        let mut slot = self.error.lock().unwrap_or_else(|error| error.into_inner());
        if self.finished.load(Ordering::Acquire) {
            return;
        }
        // The checklist is closed inside the lock that already decides which caller wins, so the
        // rung a failed open stopped at cannot be overwritten by the guard behind it.
        self.steps.end(error.is_some());
        *slot = error;
        self.finished.store(true, Ordering::Release);
    }

    /// A snapshot for the page.
    pub(crate) fn snapshot(&self) -> OpeningView {
        let phase = *self.phase.lock().unwrap_or_else(|error| error.into_inner());
        OpeningView {
            root: self.root.clone(),
            phase,
            elapsed_secs: self.started.elapsed().as_secs(),
            steps: self.steps.views(Instant::now()),
            // Rounded down, so a bar reaches full width only when the step it draws is over rather
            // than when it is near enough.
            percent: phase.counted().and_then(|(done, total)| {
                (total > 0).then(|| (done.min(total) * 100 / total) as u32)
            }),
            finished: self.finished(),
            error: self
                .error
                .lock()
                .map(|e| e.clone())
                .unwrap_or_else(|error| error.into_inner().clone()),
        }
    }
}

/// Ends the job however its thread leaves.
///
/// Set `finished` only on the two paths through the worker and a panic anywhere in `Db::open`,
/// `Workspace::new`, `publish` or `remember` leaves the slot *in flight for the life of the
/// process* — every later open answering `already opening …`, with no timeout, no cancel and
/// nothing to clear it but restarting the tool. In a window with no console that is an application
/// that will not open anything again and does not say why.
///
/// A `Drop` rather than a `catch_unwind` because it covers any future early return out of that
/// closure as well, and needs no `AssertUnwindSafe` over [`State`].
struct EndsTheJob(Arc<Opening>);

impl Drop for EndsTheJob {
    fn drop(&mut self) {
        self.0
            .finish(Some("opening stopped unexpectedly".to_owned()));
    }
}

/// A snapshot of a folder being opened.
#[derive(Debug, Clone, Default)]
pub struct OpeningView {
    /// The folder.
    pub root: String,
    /// What it is doing now.
    pub phase: crate::db::OpeningPhase,
    /// How long it has been going, in seconds.
    pub elapsed_secs: u64,
    /// Every rung, in order, with where each stands.
    pub steps: Vec<crate::step::StepView>,
    /// How far through the running rung, where that rung counts what it does.
    pub percent: Option<u32>,
    /// Whether it has ended.
    pub finished: bool,
    /// What went wrong, if anything.
    pub error: Option<String>,
}

/// Where a request goes when nothing is open.
pub const OPEN_PATH: &str = "/open";

/// Refuses a state-changing request that came from another site.
///
/// **This tool has no login, and on a loopback port that is the right call — but it is not a reason
/// to be reachable from any page the person happens to have open.** Every form here posts
/// `application/x-www-form-urlencoded`, which is a CORS *simple request*: a browser sends it
/// cross-origin without a preflight and without asking anyone. So any tab, on any site, could
/// `fetch('http://127.0.0.1:8178/quit', {method:'POST', mode:'no-cors', …})` and be obeyed — and
/// `/quit` is the mild end of it. The reachable set included `/packages/{id}/delete`,
/// `/settings/restore`, the bulk language and tag writes over a whole corpus, and `/settings/backup`,
/// which writes a file to a path taken from the form.
///
/// The machine's own surfaces already answer this: `km-admin-pages` and `km-remote-pages` both set
/// their cookies `SameSite=Lax`, which is exactly this refusal spelled in the place a cookie makes
/// it available. This tool keeps no cookie to hang it on, so it is a header check instead.
///
/// **`Sec-Fetch-Site` is the whole of it where the browser sends one**, which is every browser this
/// tool is opened in — it is sent on every request, it cannot be set by script, and `same-origin` is
/// precisely the question being asked. `Origin` is the fallback for anything that sends one and not
/// the other, and its absence is allowed: a `curl` or a script driving this tool deliberately sends
/// neither, and refusing those would break the dev remote and every command-line check for no gain,
/// because an attacker's lever here is somebody's *browser*, which always sends them.
///
/// A remembered filter with `favorite=` dropped when no favorite in the corpus carries that id.
///
/// Works on the query string rather than on a parsed [`crate::handlers::FilterQuery`], because the
/// two shapes have to agree about every key and going through the parse would make this a second
/// place that has to know them all. The key is compared before the `=` and so needs no decoding, and
/// the value is a row id, which percent-encoding leaves as it found it.
pub(crate) fn without_missing_favorite(
    filter: &str,
    known: &[crate::model::FavoriteNode],
) -> String {
    filter
        .split('&')
        .filter(|pair| match pair.strip_prefix("favorite=") {
            Some(id) => known.iter().any(|node| node.id.to_string() == id),
            None => true,
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// GET and HEAD are not gated. They change nothing, and gating them would refuse an ordinary link.
fn is_cross_site(request: &axum::extract::Request) -> bool {
    if matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        return false;
    }
    let headers = request.headers();
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        // `none` is a user typing the address or opening a bookmark; `same-origin` is this tool's
        // own pages. Everything else — `cross-site`, `same-site` — is another origin asking.
        return !matches!(site.trim(), "same-origin" | "none");
    }
    let Some(origin) = headers.get(axum::http::header::ORIGIN) else {
        return false;
    };
    let Ok(origin) = origin.to_str() else {
        return true;
    };
    // Compared against `Host` rather than against a configured address, because the tool is reached
    // as `127.0.0.1:<port>` or `localhost:<port>` depending on what was typed, and both are correct.
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        != Some(host)
}

/// Sends a request that needs a folder to the Open page when there is not one.
///
/// **A plain 302 is wrong here, and would have been wrong invisibly.** Every page in this tool is
/// htmx: a button issues an XHR and swaps the response into a target element. A browser follows a 302
/// transparently, so htmx would receive the *Open page* and swap that whole document into whatever
/// `<div>` the button aimed at — a picker nested inside a table cell, with no error anywhere. So an
/// htmx request is answered with `HX-Redirect`, which htmx understands as "navigate the window", and
/// only an ordinary navigation gets the 302.
///
/// It goes on `.layer` rather than `.route_layer` so that a request to a path that no longer matches
/// anything is caught too.
async fn require_workspace(
    axum::extract::State(state): axum::extract::State<State>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // **Which language this browser asked for**, taken here because this is the one place a
    // request's headers are in reach before every route. It only ever matters while the settings
    // name no language of their own; see `State::negotiate`.
    state.negotiate(
        request
            .headers()
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok()),
    );

    // Before the workspace test rather than after, and before every route: a request from another
    // site is refused whether or not a folder is open, and whether or not the path it named exists.
    // See `is_cross_site`.
    if is_cross_site(&request) {
        tracing::warn!(
            path = %request.uri().path(),
            "refused a state-changing request that came from another site"
        );
        return (
            StatusCode::FORBIDDEN,
            "this request came from another site and was refused",
        )
            .into_response();
    }

    if state.workspace().is_some() {
        return next.run(request).await;
    }

    let path = request.uri().path();
    // The Open page itself, everything it posts to, and the embedded assets every page needs in
    // order to render at all. Without the last of these the picker arrives unstyled and scriptless,
    // which is a worse first impression than the error it replaced.
    //
    // **And the two controls that are about the *program* rather than about a folder**, which is the
    // reported bug: the picker's own header carries Quit, and pressing it posted to `/quit`, met this
    // middleware, and was answered with `HX-Redirect: /open`. htmx navigated to the page it was
    // already on and the tool went on running — a button that visibly did nothing, on the one page
    // where it is the only way out. Neither of these touches a workspace; requiring one of them was
    // never a rule, only the default falling through.
    if path == OPEN_PATH
        || path.starts_with("/open/")
        || path.starts_with("/static/")
        || path == "/quit"
        || path == "/browser"
    {
        return next.run(request).await;
    }

    if request.headers().contains_key("hx-request") {
        return ([("hx-redirect", OPEN_PATH)], StatusCode::NO_CONTENT).into_response();
    }
    axum::response::Redirect::to(OPEN_PATH).into_response()
}

/// Builds the router.
///
/// Every route is here rather than spread across modules, so the whole surface of the tool is one
/// screenful — the same reason `km-api` keeps its `SURFACE` table in one place.
pub fn router(state: State) -> Router {
    Router::new()
        .route("/", get(handlers::index))
        .route(OPEN_PATH, get(handlers::open_page))
        .route("/open/list", get(handlers::open_list))
        .route("/open/progress", get(handlers::open_progress))
        .route("/open/open", post(handlers::open_folder))
        .route("/open/forget", post(handlers::open_forget))
        .route("/open/close", post(handlers::open_close))
        .route("/quit", post(handlers::quit))
        .route("/browser", post(handlers::open_in_browser))
        .route("/songs", get(handlers::songs))
        .route("/songs/rows", get(handlers::song_rows))
        .route("/songs/{id}", get(handlers::song))
        .route("/songs/{id}/row", get(handlers::song_row))
        .route("/songs/{id}/rename", post(handlers::rename))
        // Not `/songs/language-bulk`'s singular: that one writes to a whole filter and confirms
        // first, this one is a select in a row and answers with the row.
        .route("/songs/{id}/language", post(handlers::set_song_language))
        .route("/songs/{id}/lyrics", get(handlers::lyrics))
        .route("/songs/{id}/raw", get(handlers::raw))
        .route("/songs/{id}/download", get(handlers::download))
        .route("/songs/{id}/edit", post(handlers::edit_song))
        .route("/songs/{id}/corrections", post(handlers::save_corrections))
        .route("/songs/{id}/user-score", post(handlers::user_score))
        .route(
            "/songs/{id}/lyrics-hidden",
            post(handlers::set_lyrics_hidden),
        )
        .route("/songs/{id}/favorites", post(handlers::song_favorites))
        // Before the `{favorite}` route: `new` is a word, not an id, and axum would otherwise try to
        // parse it as one.
        .route(
            "/songs/{id}/favorites/new",
            post(handlers::favorite_into_new),
        )
        .route(
            "/songs/{id}/favorites/{favorite}",
            post(handlers::toggle_favorite),
        )
        .route("/songs/{id}/pin-encoding", post(handlers::pin_encoding))
        .route("/songs/{id}/play", post(handlers::play))
        .route("/songs/{id}/reveal", post(handlers::reveal))
        .route("/songs/{id}/unmerge", post(handlers::unmerge))
        .route(
            "/favorites",
            get(handlers::favorites).post(handlers::create_favorite),
        )
        // Static segments, so they sit beside `/songs/{id}` above without either shadowing the
        // other — `/songs/tag-bulk/cancel` and `/songs/{id}/row` already prove the arrangement.
        .route("/songs/saved-filters", post(handlers::save_filter))
        .route(
            "/songs/saved-filters/cancel",
            get(handlers::save_filter_cancel),
        )
        .route(
            "/songs/saved-filters/{id}/delete",
            post(handlers::delete_saved_filter),
        )
        .route(
            "/songs/saved-filters/{id}/update",
            post(handlers::update_saved_filter),
        )
        .route(
            "/songs/saved-filters/{id}/chip",
            get(handlers::saved_filter_chip),
        )
        .route(
            "/songs/saved-filters/{id}/rename",
            post(handlers::rename_saved_filter),
        )
        .route("/songs/language-bulk", post(handlers::bulk_language))
        .route("/songs/tag-bulk", post(handlers::bulk_tag))
        .route("/songs/tag-bulk/cancel", get(handlers::bulk_tag_cancel))
        .route("/songs/delete-bulk", post(handlers::bulk_delete))
        .route(
            "/songs/delete-bulk/cancel",
            get(handlers::bulk_delete_cancel),
        )
        .route("/songs/favorite-bulk", post(handlers::bulk_favorite))
        .route(
            "/songs/favorite-bulk/cancel",
            get(handlers::bulk_favorite_cancel),
        )
        .route("/songs/reanalyze", post(handlers::reanalyze))
        .route("/songs/reanalyze/cancel", get(handlers::reanalyze_cancel))
        .route("/songs/quality-hint", post(handlers::quality_hint))
        .route(
            "/songs/quality-hint/clear",
            post(handlers::quality_hint_clear),
        )
        .route("/songs/{id}/tags", post(handlers::song_tags))
        .route(
            "/songs/language-bulk/cancel",
            get(handlers::bulk_language_cancel),
        )
        .route(
            "/songs/titles-from-filename",
            post(handlers::titles_from_filename),
        )
        .route("/songs/fix-name-case", post(handlers::fix_name_case))
        .route(
            "/songs/split-artist-from-title",
            post(handlers::split_artist_from_title),
        )
        .route("/favorites/add", post(handlers::add_to_favorite))
        .route("/favorites/{id}/rename", post(handlers::rename_favorite))
        .route("/favorites/{id}/delete", post(handlers::delete_favorite))
        .route("/favorites/{id}/tidy", post(handlers::tidy_favorite))
        .route(
            "/favorites/{id}/temporary",
            post(handlers::set_favorite_temporary),
        )
        // Not under `/songs`: this searches the words, not the songs, and `/songs/{id}/lyrics` is
        // already the words of one song. Two nouns, two paths.
        .route("/lyrics", get(handlers::lyric_search))
        .route("/lyrics/hits", get(handlers::lyric_hits))
        .route("/similar", get(handlers::similar))
        .route("/similar/hits", get(handlers::similar_hits))
        .route("/similar-words", get(handlers::similar_words))
        .route("/similar-words/hits", get(handlers::similar_words_hits))
        .route("/folders", get(handlers::folders))
        .route("/songs/{id}/release", post(handlers::release))
        .route("/duplicates", get(handlers::duplicates))
        .route("/duplicates/suggest", post(handlers::suggest_duplicates))
        .route(
            "/songs/{id}/not-the-same/{other}",
            post(handlers::not_the_same),
        )
        .route(
            "/packages",
            get(handlers::packages).post(handlers::create_package),
        )
        .route("/packages/add", post(handlers::add_to_package))
        .route("/packages/from-filter", post(handlers::package_from_filter))
        .route("/packages/import", post(handlers::import_package))
        .route("/packages/{id}", get(handlers::package))
        .route("/packages/{id}/delete", post(handlers::delete_package))
        .route("/packages/{id}/settings", post(handlers::package_settings))
        .route(
            "/packages/{id}/volume",
            post(handlers::package_volume_settings),
        )
        .route("/packages/{id}/number", post(handlers::package_number))
        .route("/packages/{id}/remove", post(handlers::package_remove))
        .route("/packages/replace", post(handlers::package_replace))
        .route("/packages/{id}/renumber", post(handlers::package_renumber))
        .route("/packages/{id}/held/fill", post(handlers::held_fill))
        .route("/packages/{id}/held/release", post(handlers::held_release))
        .route(
            "/packages/{id}/sources/add",
            post(handlers::add_package_source),
        )
        .route(
            "/packages/{id}/sources/remove",
            post(handlers::remove_package_source),
        )
        .route("/packages/{id}/sync", post(handlers::sync_package))
        .route("/packages/{id}/build", post(handlers::build_package))
        .route(
            "/packages/{id}/build/progress",
            get(handlers::build_progress),
        )
        .route("/packages/{id}/build/out", get(handlers::build_out))
        .route("/packages/{id}/build/pane", get(handlers::build_pane))
        .route("/packages/{id}/build/all", post(handlers::build_all))
        .route("/packages/{id}/spec", post(handlers::write_package_spec))
        .route("/packages/{id}/install", post(handlers::install_package))
        .route("/scan", get(handlers::scan_page).post(handlers::start_scan))
        .route("/scan/progress", get(handlers::scan_progress))
        .route("/scan/stop", post(handlers::stop_scan))
        .route("/scan/failures/remove", post(handlers::remove_failures))
        .route("/scan/failures/restore", post(handlers::restore_failures))
        .route(
            "/settings",
            get(handlers::settings).post(handlers::save_settings),
        )
        // POST rather than GET: it puts multicast traffic on the network, which is an action even
        // though it changes nothing here.
        .route("/settings/tags", post(handlers::save_default_tags))
        // Its own route for the same reason the tags have one: this writes a preference into the
        // person's config directory, where `POST /settings` writes an address into the corpus's
        // database.
        .route("/settings/locale", post(handlers::save_locale))
        .route("/settings/discover", post(handlers::discover_machines))
        // The machine's own password, exchanged for a token this run holds. Three routes rather
        // than one because they are three different sentences: signing in, forgetting, and the
        // switch that mounts the machine's two play routes.
        .route("/settings/login", post(handlers::log_in_to_machine))
        .route("/settings/logout", post(handlers::log_out_of_machine))
        .route("/settings/debugging", post(handlers::set_debugging))
        // Under `/settings/` rather than at the top level, following `/settings/discover`: the whole
        // surface of the tool is meant to stay the one screenful this function is.
        .route("/settings/backup", post(handlers::write_backup))
        .route("/settings/restore", post(handlers::restore_backup))
        // The same three URLs `ServeDir` used to answer, so no template changes with the move.
        .route(
            "/static/htmx.min.js",
            get(|| async { embedded("application/javascript; charset=utf-8", HTMX_JS) }),
        )
        .route(
            "/static/ui.js",
            get(|| async { embedded("application/javascript; charset=utf-8", UI_JS) }),
        )
        .route(
            "/static/style.css",
            get(|| async { embedded("text/css; charset=utf-8", STYLE_CSS) }),
        )
        .route(
            "/static/htmx-LICENSE.txt",
            get(|| async { embedded("text/plain; charset=utf-8", HTMX_LICENSE) }),
        )
        .route(
            "/static/icon.png",
            get(|| async { embedded_bytes("image/png", ICON_PNG) }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_workspace,
        ))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Scratch;

    /// The page's script and stylesheet are really in the binary, and are really the right files.
    ///
    /// `include_str!` makes a *missing* file a build failure, which is most of the guarantee. What it
    /// cannot catch is a file that is present and empty or wrong — and a stylesheet that silently
    /// became zero bytes would leave a page that renders, so nothing would look broken enough to
    /// investigate.
    #[test]
    fn the_static_files_are_embedded_and_not_empty() {
        assert!(
            HTMX_JS.contains("htmx"),
            "the embedded script does not look like htmx"
        );
        assert!(
            HTMX_JS.len() > 10_000,
            "htmx is 50 KB, not {}",
            HTMX_JS.len()
        );
        assert!(
            STYLE_CSS.contains('{'),
            "the embedded stylesheet has no rules in it"
        );
        assert!(
            HTMX_LICENSE.to_lowercase().contains("permission"),
            "the license must ship with the code it covers"
        );
        assert!(
            ICON_PNG.starts_with(b"\x89PNG\r\n\x1a\n"),
            "the embedded favicon is not a PNG"
        );
        // The two icons must not be the same file. This tool served the machine's favicon for most
        // of its life and the change is one character of `include_bytes!` to undo by accident --
        // a stale path, or a merge taking the wrong side. Bytes rather than pixels because the whole
        // point is that they are different renderings, and comparing them needs no decoder here;
        // that the blue one is actually blue is asserted in `km_display::icon`, where the artwork's
        // colors are pinned and an image decoder is already in the build.
        assert_ne!(
            ICON_PNG, MACHINE_ICON_PNG,
            "the package builder is serving the machine's favicon again"
        );
        // Named events rather than a length, because the file's whole reason to exist is that it
        // hears these. A `ui.js` that still parsed but had lost the listener would leave every
        // failure silent again, which is the exact fault it was written to fix and the one nothing
        // else here can see.
        for listener in ["htmx:responseError", "htmx:sendError"] {
            assert!(
                UI_JS.contains(listener),
                "ui.js no longer listens for {listener}, so a failed request would say nothing"
            );
        }
        // The attribute a form uses to ask for its page back, spelled as the DOM hands it over. A
        // rename on either side alone parses, runs, and quietly drops the reader at the top of the
        // list — which is the fault, and it is invisible to every other test here because the copy
        // happens in the browser. `the_titles_actions_ask_for_their_page_back` in `views` holds the
        // markup end of the same rule.
        assert!(
            UI_JS.contains("keepsThePage"),
            "ui.js no longer reads the attribute a form asks for its page back with"
        );
    }

    /// Fetches one page through the real router, as a browser would.
    async fn get(state: &State, uri: &str) -> (axum::http::StatusCode, String) {
        use tower::ServiceExt;

        let request = axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .expect("request");
        let response = router(state.clone())
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read the body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Fetches one page for its status, keeping only as much of the body as an assertion can print.
    ///
    /// **A page whose size is a property of the machine rather than of the code needs this**, and
    /// the folder picker draws one: its crumb bar links every folder above the one being listed, the
    /// topmost of which is the system temp folder. That folder belongs to every program on the
    /// computer — a suite of another crate leaving scratch directories behind is enough — so a test
    /// that reads the whole listing is a test that fails on how full a folder it does not own
    /// happens to be. [`get`]'s cap is right for a page this crate decides the size of.
    async fn answered(state: &State, uri: &str) -> (axum::http::StatusCode, String) {
        use tower::ServiceExt;

        let request = axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .expect("request");
        let response = router(state.clone())
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let body = match axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(error) => format!("[a body this test does not need: {error}]"),
        };
        (status, body)
    }

    /// Fetches one page and reads where it was sent, if it was sent anywhere.
    ///
    /// [`get`] reads the body and drops the headers, which is the wrong half for a route whose whole
    /// answer is a `Location`.
    async fn sent_to(state: &State, uri: &str) -> (axum::http::StatusCode, Option<String>) {
        use tower::ServiceExt;

        let request = axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .expect("request");
        let response = router(state.clone())
            .oneshot(request)
            .await
            .expect("the router answers");
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        (response.status(), location)
    }

    /// Posts a form body carrying the fetch-metadata header a browser would send.
    ///
    /// `None` is the no-header case: `curl`, a script, the dev remote.
    async fn post_from(
        state: &State,
        uri: &str,
        site: Option<&str>,
    ) -> (axum::http::StatusCode, String) {
        use tower::ServiceExt;

        let mut builder = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri(uri)
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            );
        if let Some(site) = site {
            builder = builder.header("sec-fetch-site", site);
        }
        let response = router(state.clone())
            .oneshot(builder.body(axum::body::Body::empty()).expect("request"))
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read the body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Another site cannot press this tool's buttons.
    ///
    /// **The reachable set was the whole mutating surface**, because a form post is a CORS simple
    /// request and this tool has no login to stand in the way. `/quit` is the one that is easiest to
    /// demonstrate and the least of it — `/packages/{id}/delete`, `/settings/restore` and the bulk
    /// writes over a whole corpus were all equally open to any tab the person had.
    #[tokio::test]
    async fn a_post_from_another_site_is_refused() {
        let state = State::empty();
        for site in ["cross-site", "same-site"] {
            let (status, _) = post_from(&state, "/quit", Some(site)).await;
            assert_eq!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "a `{site}` post reached /quit"
            );
        }
    }

    /// ...and this tool's own pages, and a bare `curl`, still can.
    ///
    /// The second half matters as much as the first: refusing a request that carries no fetch
    /// metadata would break every command-line check and the dev remote, and would buy nothing —
    /// the lever this guards against is somebody's browser, which always sends the header.
    #[tokio::test]
    async fn this_tools_own_pages_and_a_bare_curl_are_not_refused() {
        let state = State::empty();
        for site in [Some("same-origin"), Some("none"), None] {
            let (status, _) = post_from(&state, "/quit", site).await;
            assert_ne!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "a {site:?} post was refused as cross-site"
            );
        }
    }

    /// A GET is not gated, or an ordinary link into the tool would be refused.
    #[tokio::test]
    async fn a_cross_site_get_is_not_refused() {
        use tower::ServiceExt;

        let request = axum::http::Request::builder()
            .uri("/static/style.css")
            .header("sec-fetch-site", "cross-site")
            .body(axum::body::Body::empty())
            .expect("request");
        let response = router(State::empty())
            .oneshot(request)
            .await
            .expect("the router answers");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    /// Posts a form body through the real router, the way htmx does.
    ///
    /// The content type is set because htmx sets it, and because a handler that ever reached for
    /// `axum::Form` would answer 415 to a request without one — a button that does nothing, with no
    /// message anywhere.
    async fn post(state: &State, uri: &str, body: &str) -> (axum::http::StatusCode, String) {
        use tower::ServiceExt;

        let request = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri(uri)
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(axum::body::Body::from(body.to_owned()))
            .expect("request");
        let response = router(state.clone())
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read the body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Three distinct songs, the two under `brasil/` tagged `bossa`, for the tests that narrow the
    /// list to part of the corpus.
    ///
    /// Three *different* fixtures on purpose: identical bytes are one song with two files, which is
    /// correct behavior and would quietly make a test that counts songs count something else.
    fn two_folders(name: &str) -> (Scratch, State) {
        let corpus = Scratch::new(name);
        std::fs::create_dir_all(corpus.0.join("brasil")).expect("make a folder");
        std::fs::create_dir_all(corpus.0.join("ingles")).expect("make a folder");
        for (path, bytes) in [
            ("brasil/A.kar", km_song::testing::soft_karaoke()),
            ("brasil/B.kar", km_song::testing::lyric_events()),
            ("ingles/C.kar", km_song::testing::named_text_track()),
        ] {
            std::fs::write(corpus.0.join(path), bytes).expect("write a fixture");
        }
        let state = State::new(Db::open_in_memory(&corpus.0).expect("open"));
        crate::scan::run(
            &state.workspace().expect("a folder is open").db,
            crate::scan::ScanOptions::default(),
            &std::sync::Arc::new(Progress::default()),
        )
        .expect("scan");
        {
            let db = state.workspace().expect("a folder is open").db.clone();
            let db = db.lock();
            let brasil: Vec<String> = db
                .songs(&crate::db::Filter::default())
                .expect("browse")
                .iter()
                .map(|row| row.id.clone())
                .filter(|id| {
                    db.song(id)
                        .expect("detail")
                        .files
                        .iter()
                        .any(|file| file.path.starts_with("brasil/"))
                })
                .collect();
            assert_eq!(brasil.len(), 2, "two songs under brasil/");
            let bossa = km_kmpkg::Tag::parse("bossa").expect("a tag");
            db.add_tag_of(&brasil, &bossa).expect("tag them");
        }
        (corpus, state)
    }

    /// A scan leaves the folder tree current, and the Folders page lists what it scanned.
    ///
    /// The scan builds the tree in its own tail so that the page never has to pay for a whole pass
    /// over `files` while somebody waits for it.
    #[tokio::test]
    async fn the_folders_page_lists_the_tree_a_scan_built() {
        let (_corpus, state) = two_folders("folders-page");
        let current = state
            .reading(|db| db.folder_index_is_current())
            .await
            .expect("marker");
        assert!(current, "the scan left the folder tree behind");

        let (status, html) = get(&state, "/folders").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains(r#"href="/folders?path=brasil/""#), "{html}");
        assert!(html.contains(r#"href="/songs?folder=ingles/""#), "{html}");
    }

    /// The body the filter bar actually submits, with the tags set to what is asked for.
    ///
    /// Every control, including the empty ones — which is the shape that matters. A form submits all
    /// of its fields, so the filter arrives as a dozen empty strings and one set value, and a reader
    /// that only ever saw `rebuild`'s output (which leaves empty fields out) would not be tested on
    /// what a browser sends.
    fn bar(tags: &str) -> String {
        format!(
            "q=&artist=&sort=title&initial=&suitability=&user_score=\
             &melody=&kind=&language=&copies=&added=&granularity=&encoding_source=&favorite=\
             &tags={tags}"
        )
    }

    /// The action's own `hx-post`, read back out of the confirmation it answered with.
    ///
    /// Taken from the markup rather than rebuilt by hand, because half of what these tests are for
    /// is that the handler and the fragment it renders agree about the round trip.
    fn confirm_url(html: &str) -> String {
        let after = html.split("hx-post=\"").nth(1).expect("a confirm button");
        after
            .split('"')
            .next()
            .expect("a quoted url")
            // askama escapes the ampersands; a URL is what the browser would send back.
            .replace("&#38;", "&")
            .replace("&amp;", "&")
    }

    /// Making a package out of "every matching song" has to mean the filter that is on screen.
    ///
    /// The bug, exactly as reported: filter the list to a fraction of the corpus, press *Make*, and
    /// the confirmation offers the whole corpus and says so out loud. The filter was carried by a
    /// query string rendered into the form's `hx-post` at page load, and the filter bar never
    /// re-renders the page — so the attribute described a page that had stopped existing the moment
    /// anybody used the bar.
    ///
    /// Through the router with **no query string at all**, which is what the page now sends.
    #[tokio::test]
    async fn a_package_from_the_filter_counts_the_bar_and_not_the_corpus() {
        let (_corpus, state) = two_folders("from-filter");

        let (status, html) = post(
            &state,
            "/packages/from-filter",
            &format!("name=Vol+1&{}", bar("bossa")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("<strong>2 songs</strong>"),
            "two songs, not three:\n{html}"
        );
        assert!(
            html.contains("tag: bossa"),
            "the chip names the filter:\n{html}"
        );
        // The fingerprint of the bug. It was a true sentence about what the handler had been asked,
        // which is what made it so hard to read as a fault in the page.
        assert!(
            !html.contains("the whole corpus"),
            "the filter reached the handler, so it must not say this:\n{html}"
        );

        // And with nothing narrowing it, the warning is still right and still appears.
        let (_, all) = post(
            &state,
            "/packages/from-filter",
            &format!("name=All&{}", bar("")),
        )
        .await;
        assert!(all.contains("<strong>3 songs</strong>"), "{all}");
        assert!(all.contains("the whole corpus"), "{all}");
    }

    /// The confirmed write goes to the set that was counted, whatever the bar says by then.
    ///
    /// The other half of the rule, and the reason the two phases read the filter from two different
    /// places. A confirmation can sit on screen for as long as somebody leaves it there; what gets
    /// written has to be what they were shown, so phase two takes the query string phase one handed
    /// back and ignores a body that has since been cleared.
    #[tokio::test]
    async fn the_confirmed_write_takes_the_set_that_was_counted() {
        let (_corpus, state) = two_folders("from-filter-confirm");

        let (_, offered) = post(
            &state,
            "/packages/from-filter",
            &format!("name=Vol+1&{}", bar("bossa")),
        )
        .await;
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");

        // The bar has since been cleared. The body says so, and it must make no difference.
        let (status, made) = post(
            &state,
            &confirm_url(&offered),
            &format!("name=Vol+1&{}", bar("")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(made.contains("Made Vol 1"), "{made}");

        let packages = state
            .workspace()
            .expect("a folder is open")
            .db
            .lock()
            .packages()
            .expect("packages");
        let made = packages
            .iter()
            .find(|p| p.name == "Vol 1")
            .expect("the package");
        // A name-shaped id is one `Manifest::problems` refuses, so a package made here could not be
        // built.
        assert!(
            km_kmpkg::PackageMeta::is_generated_id(&made.id),
            "the id is generated: {}",
            made.id
        );
        assert_eq!(
            made.song_count, 2,
            "the write took the counted set, not the cleared bar"
        );

        // A second name making the same file name is refused before anything is counted, since the
        // two builds would write one `.kmpkg`.
        let (_, again) = post(
            &state,
            "/packages/from-filter",
            &format!("name=vol-1&{}", bar("bossa")),
        )
        .await;
        assert!(
            again.contains("There is already a package called vol-1"),
            "{again}"
        );
    }

    /// A package to add songs to, made through the router the way the Packages page makes one.
    ///
    /// `start_number` is a parameter because the room a package has left is what the filter-wide add
    /// has to reason about, and a package starting at 1 has 999 of them — more than any fixture here
    /// can fill.
    async fn empty_package(state: &State, id: &str, start: u32) -> String {
        let (status, _) = post(
            state,
            "/packages",
            &format!("id={id}&name=Box&start_number={start}"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        id.to_owned()
    }

    /// Adding *every matching song* to a package that exists counts the bar, then writes what it
    /// counted.
    ///
    /// The same two halves `a_package_from_the_filter_counts_the_bar_and_not_the_corpus` and
    /// `the_confirmed_write_takes_the_set_that_was_counted` assert for the form one control up, over
    /// the form that adds to a package rather than making one. Both halves in one test because the
    /// round trip is the thing: the count has to reach the confirmation and the confirmation's own
    /// url has to reach the write.
    #[tokio::test]
    async fn adding_a_whole_filter_counts_the_bar_and_writes_what_it_counted() {
        let (_corpus, state) = two_folders("add-matching");
        let id = empty_package(&state, "box", 1).await;

        let (status, offered) = post(
            &state,
            "/packages/add?as=toast",
            &format!("package_id={id}&scope=matching&{}", bar("bossa")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            offered.contains("<strong>2 songs</strong>"),
            "two songs, not three:\n{offered}"
        );
        assert!(
            offered.contains("tag: bossa"),
            "the chip names the filter:\n{offered}"
        );
        assert!(
            !offered.contains("the whole corpus"),
            "the filter reached the handler, so it must not say this:\n{offered}"
        );

        // The bar has since been cleared, which would widen the write to the whole corpus if the
        // confirmation read it rather than the query string it was handed.
        let (status, done) = post(
            &state,
            &confirm_url(&offered),
            &format!("package_id={id}&scope=matching&{}", bar("")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(done.contains("Added 2"), "{done}");

        let db = state.workspace().expect("a folder is open").db.clone();
        let db = db.lock();
        let held = db.package_members(&id, 1).expect("members");
        assert_eq!(
            held.len(),
            2,
            "the write took the counted set, not the cleared bar"
        );
    }

    /// Replacing a song asks first, naming the song that leaves the number, and then writes.
    #[tokio::test]
    async fn a_replacement_names_the_song_leaving_before_it_writes() {
        let (_corpus, state) = two_folders("replace-confirm");
        let id = empty_package(&state, "swap", 1).await;
        let songs = {
            let db = state.workspace().expect("a folder is open").db.clone();
            let mut db = db.lock();
            let songs = db
                .matching_ids(&crate::db::Filter::default(), None)
                .expect("ids");
            db.add_to_package(&id, &songs[..2], "t").expect("add");
            songs
        };

        let (_, page) = get(&state, &format!("/songs/{}", songs[2])).await;
        assert!(
            page.contains(&format!("value=\"1#{id}\"")),
            "the song page offers the volume:\n{page}"
        );

        let body = format!("slot=1%23{id}&number=1&song_id={}", songs[2]);
        let (status, offered) = post(&state, "/packages/replace", &body).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(offered.contains("Number 1 in Box is"), "{offered}");
        {
            let db = state.workspace().expect("a folder is open").db.clone();
            let db = db.lock();
            let held = db.package_members(&id, 1).expect("members");
            assert_eq!(held[0].song_id, songs[0], "asking writes nothing");
        }

        let (_, done) = post(&state, &confirm_url(&offered), &body).await;
        assert!(done.contains("is now number 1 in Box"), "{done}");
        let db = state.workspace().expect("a folder is open").db.clone();
        let db = db.lock();
        let held = db.package_members(&id, 1).expect("members");
        assert_eq!(
            (held[0].number, held[0].song_id.as_str()),
            (1, songs[2].as_str())
        );
    }

    /// A filter matching more songs than the package has numbers left adds what fits and says so.
    ///
    /// **The sentence is the half worth pinning.** The count is otherwise read as what the filter
    /// found, so a package with room for one, handed a filter matching three, would report adding one
    /// and look like a filter that matched exactly one.
    #[tokio::test]
    async fn a_filter_bigger_than_the_package_adds_what_fits_and_says_so() {
        let (_corpus, state) = two_folders("add-matching-full");
        // Numbers 999 and nothing above it, so exactly one song fits.
        let id = empty_package(&state, "brim", u32::from(km_songcode::MAX_SLOT)).await;

        let (_, offered) = post(
            &state,
            "/packages/add?as=toast",
            &format!("package_id={id}&scope=matching&{}", bar("")),
        )
        .await;
        assert!(offered.contains("<strong>3 songs</strong>"), "{offered}");
        assert!(
            offered.contains("room for 1"),
            "the confirmation has to say what will not fit:\n{offered}"
        );

        let (_, done) = post(
            &state,
            &confirm_url(&offered),
            &format!("package_id={id}&scope=matching&{}", bar("")),
        )
        .await;
        assert!(done.contains("Added 1"), "{done}");
        assert!(
            done.contains("more songs than this package has room for"),
            "a count alone reads as what the filter found:\n{done}"
        );
    }

    /// A body with no `scope` adds the ticked rows and nothing else.
    ///
    /// **This is what a song's own page and the Lyrics page send**, neither of which has a filter bar
    /// to match against. The scope select is the browse page's alone, so the absent key has to go on
    /// meaning *ticked* — and it must not confirm, because there is nothing on either page to put a
    /// question into.
    #[tokio::test]
    async fn a_body_with_no_scope_adds_the_ticked_rows() {
        let (_corpus, state) = two_folders("add-ticked");
        let id = empty_package(&state, "ticked", 1).await;
        let one = {
            let db = state.workspace().expect("a folder is open").db.clone();
            let db = db.lock();
            db.matching_ids(&crate::db::Filter::default(), None)
                .expect("ids")
                .first()
                .cloned()
                .expect("a song")
        };

        let (status, done) = post(
            &state,
            "/packages/add?as=toast",
            &format!("package_id={id}&song_id={one}"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            done.contains("Added 1"),
            "no confirmation, just the write:\n{done}"
        );

        let db = state.workspace().expect("a folder is open").db.clone();
        let db = db.lock();
        assert_eq!(db.package_members(&id, 1).expect("members").len(), 1);
    }

    /// The language set also writes the counted set, not whatever the bar says by the time it lands.
    ///
    /// **The test the `BulkAction` extractor made cheap, and the reason it exists.** This invariant
    /// was restated by hand in all three bulk handlers and asserted for exactly one of them, so the
    /// other two held it only because somebody had copied the lines correctly. Now they hold it
    /// because there is one copy — and this is what says so for a second handler.
    #[tokio::test]
    async fn the_language_set_also_writes_the_set_that_was_counted() {
        let (_corpus, state) = two_folders("language-confirm");

        let (_, offered) = post(
            &state,
            "/songs/language-bulk",
            &format!("set_language=pt&scope=matching&{}", bar("bossa")),
        )
        .await;
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");

        // The bar has since been cleared, which would widen the write to the whole corpus if the
        // confirmation read it rather than the query string it was handed.
        let (status, done) = post(
            &state,
            &confirm_url(&offered),
            &format!("set_language=pt&scope=matching&{}", bar("")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(done.contains('2'), "it should report two songs: {done}");

        let db = state.workspace().expect("a folder is open").db.clone();
        let db = db.lock();
        let counts = db.counts().expect("counts");
        assert_eq!(
            db.count_matching(&crate::db::Filter {
                language: crate::db::LanguageFilter::Unset,
                ..Default::default()
            })
            .expect("unset"),
            counts.songs - 2,
            "only the two counted songs may have been given a language"
        );
    }

    /// Filing a whole filter into a favorite, and then taking the same set back out of it.
    ///
    /// The two directions over one set, because the pair is the point: the control offers *take out*
    /// beside *put into*, and what makes that safe is that the direction is named rather than read
    /// off whether each song happens to be filed already.
    #[tokio::test]
    async fn a_favorite_takes_a_whole_filter_and_gives_it_back() {
        let (_corpus, state) = two_folders("favorite-bulk");
        let favorite = {
            let db = state.workspace().expect("a folder is open").db.clone();
            let db = db.lock();
            db.create_favorite("Bossa").expect("make a favorite")
        };
        let filed = |db: &crate::db::Db| {
            db.count_matching(&crate::db::Filter {
                favorite: Some(favorite),
                ..Default::default()
            })
            .expect("count")
        };

        let body = format!("favorite_id={favorite}&scope=matching&{}", bar("bossa"));
        let (_, offered) = post(&state, "/songs/favorite-bulk", &body).await;
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");
        assert!(
            offered.contains("Bossa"),
            "the favorite is named: {offered}"
        );

        let (status, done) = post(&state, &confirm_url(&offered), &body).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(done.contains("Filed 2"), "{done}");
        {
            let db = state.workspace().expect("a folder is open").db.clone();
            let db = db.lock();
            assert_eq!(filed(&db), 2);
        }

        // The same set, the other way. `remove` is a word in the body and never a reading of where
        // each song already is.
        let out = format!("favorite_action=remove&{body}");
        let (_, offered) = post(&state, "/songs/favorite-bulk", &out).await;
        assert!(offered.contains("out of"), "{offered}");
        let (_, done) = post(&state, &confirm_url(&offered), &out).await;
        assert!(done.contains("Took out 2"), "{done}");

        let db = state.workspace().expect("a folder is open").db.clone();
        let db = db.lock();
        assert_eq!(filed(&db), 0, "the favorite is empty again");
    }

    /// The favorite write also takes the counted set, not whatever the bar says by the time it lands.
    ///
    /// The third handler to say so, and it holds for the same reason the other two do: `BulkAction`
    /// swaps the live bar for the frozen query string on the confirmed pass, in one place.
    #[tokio::test]
    async fn the_favorite_write_takes_the_set_that_was_counted() {
        let (_corpus, state) = two_folders("favorite-confirm");
        let favorite = {
            let db = state.workspace().expect("a folder is open").db.clone();
            let db = db.lock();
            db.create_favorite("Bossa").expect("make a favorite")
        };

        let (_, offered) = post(
            &state,
            "/songs/favorite-bulk",
            &format!("favorite_id={favorite}&scope=matching&{}", bar("bossa")),
        )
        .await;
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");

        // The bar has since been cleared, which would file the whole corpus if the confirmation read
        // it rather than the query string it was handed.
        let (status, done) = post(
            &state,
            &confirm_url(&offered),
            &format!("favorite_id={favorite}&scope=matching&{}", bar("")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(done.contains("Filed 2"), "{done}");

        let db = state.workspace().expect("a folder is open").db.clone();
        let db = db.lock();
        assert_eq!(
            db.count_matching(&crate::db::Filter {
                favorite: Some(favorite),
                ..Default::default()
            })
            .expect("count"),
            2,
            "only the two counted songs may have been filed"
        );
    }

    /// Recalculating counts the files it would read, and the confirmed press starts the job.
    ///
    /// It answers with where to watch rather than with a result, because the work is a scan: the
    /// request returns in milliseconds and the reading goes on for as long as it takes.
    #[tokio::test]
    async fn recalculating_counts_the_files_and_then_starts_a_scan() {
        let (_corpus, state) = two_folders("reanalyze");

        let body = format!("scope=matching&{}", bar("bossa"));
        let (status, offered) = post(&state, "/songs/reanalyze", &body).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(offered.contains("<strong>2 files</strong>"), "{offered}");
        assert!(
            !state.scan_running(),
            "counting must not have started anything"
        );

        let (status, done) = post(&state, &confirm_url(&offered), &body).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(done.contains("Re-reading 2"), "{done}");
        assert!(done.contains("Scan page"), "it says where to watch: {done}");

        // Joined rather than polled: the scan is a detached thread, and a test that asserted on the
        // database while it was still writing would be a test of timing. Whether it finished or was
        // stopped part way makes no difference to what is being asserted — a scoped run has no pass
        // that can take a row away at either point.
        state.workspace().expect("a folder is open").stop_scan();
        let db = state.workspace().expect("a folder is open").db.clone();
        let db = db.lock();
        assert_eq!(
            db.counts().expect("counts").files,
            3,
            "the song outside the filter still has its file row"
        );
    }

    /// The Stop button asks a running scan to stop and answers without waiting for it.
    ///
    /// A folder this small can finish before the press arrives, so what is asserted holds either
    /// way: once the answer is back, the run has been asked to stop or had already ended.
    #[tokio::test]
    async fn stop_asks_a_running_scan_to_stop_and_answers_at_once() {
        let (_corpus, state) = two_folders("stop-scan");

        let (status, idle) = post(&state, "/scan/stop", "").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            !idle.contains(r#"hx-post="/scan/stop""#),
            "nothing to stop: {idle}"
        );

        let progress = state
            .start_scan(crate::scan::ScanOptions::default())
            .expect("a folder is open");
        let (status, _) = post(&state, "/scan/stop", "").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            progress.stopping() || progress.snapshot().finished,
            "a scan still running after the press was not asked to stop"
        );
        state.workspace().expect("a folder is open").stop_scan();
    }

    /// The bulk language set reads the bar, and its own select is not one of the bar's.
    ///
    /// `#filters` has a `language` select and this form sets a language, so the two would be one key
    /// arriving twice — which serde answers with `duplicate_field` and a 400, i.e. a button that
    /// does nothing. Hence `set_language`. Rename it back and this test goes red.
    ///
    /// `scope=matching` is what asks for the filter-wide write; without it the action is over the
    /// ticked rows, which is the default and the smaller act.
    #[tokio::test]
    async fn the_bulk_language_set_reads_the_bar_beside_its_own_select() {
        let (_corpus, state) = two_folders("bulk-language");

        let (status, offered) = post(
            &state,
            "/songs/language-bulk",
            &format!("set_language=pt&scope=matching&{}", bar("bossa")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");
        assert!(offered.contains("Portuguese"), "{offered}");
        assert!(!offered.contains("the whole corpus"), "{offered}");

        let (_, done) = post(
            &state,
            &confirm_url(&offered),
            "set_language=pt&scope=matching",
        )
        .await;
        assert!(done.contains("2 songs"), "{done}");
    }

    /// The bulk tag write reads the bar beside its own box, and the field is `set_tag`.
    ///
    /// The twin of the language test above, and it goes red for the same reason if the field is
    /// renamed to `tags`: `#filters` carries a `tags` field of its own, so the two would be one key
    /// arriving twice — which `serde_urlencoded` answers with a 400, i.e. a button that does
    /// nothing and says nothing.
    ///
    /// It also asserts the thing the language control has no need to: the confirmation shows the
    /// **slug**, so somebody finds out `Rock & Roll` became `rock-roll` before the write and not
    /// after it.
    #[tokio::test]
    async fn the_bulk_tag_write_reads_the_bar_beside_its_own_box() {
        let (_corpus, state) = two_folders("bulk-tag");

        let (status, offered) = post(
            &state,
            "/songs/tag-bulk",
            &format!(
                "set_tag=Rock%20%26%20Roll&tag_action=add&scope=matching&{}",
                bar("bossa")
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");
        assert!(
            offered.contains("rock-roll"),
            "the confirmation shows the slug it will store: {offered}"
        );
        assert!(!offered.contains("the whole corpus"), "{offered}");

        let (_, done) = post(
            &state,
            &confirm_url(&offered),
            "set_tag=Rock%20%26%20Roll&tag_action=add&scope=matching",
        )
        .await;
        assert!(done.contains("2 songs"), "{done}");

        // And the write landed where the filter said, not over the whole corpus.
        let db = state.workspace().expect("open");
        let db = db.db.lock();
        assert_eq!(
            db.tags_present().expect("vocabulary"),
            ["bossa", "rock-roll"]
        );
        assert_eq!(
            db.count_matching(&crate::db::Filter {
                tags: vec!["rock-roll".to_owned()],
                ..crate::db::Filter::default()
            })
            .expect("count"),
            2
        );
    }

    /// Deleting counts and asks first, then writes and comes back on the page it was pressed on.
    ///
    /// **Four things the other bulk actions have no equivalent of.** The first pass writes nothing
    /// and names how many of the set a package holds. The confirmed pass answers with `#rows`
    /// rather than a toast alone, because this one takes songs *out* of the list they were ticked
    /// in. The page it was pressed on rides in the frozen query string, so the confirm button
    /// carries it without `ui.js` reading anything. And that answer empties the confirmation's own
    /// slot out of band, which is what aiming elsewhere costs.
    #[tokio::test]
    async fn deleting_asks_first_and_comes_back_on_the_page_it_was_pressed_on() {
        let (_corpus, state) = two_folders("bulk-delete");

        // Read first, so the element the answer below sends back is the element the page draws and
        // the two cannot drift apart.
        let (_, page) = get(&state, "/songs").await;
        assert!(
            page.contains(r#"<span id="bulk-delete-result"></span>"#),
            "{page}"
        );

        let (status, offered) = post(
            &state,
            "/songs/delete-bulk?offset=0",
            &format!("delete_action=delete&scope=matching&{}", bar("bossa")),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");
        assert!(!offered.contains("the whole corpus"), "{offered}");
        // The button that goes ahead redraws the list, which no other confirmation here does.
        assert!(offered.contains("hx-target=\"#rows\""), "{offered}");

        // Nothing was written by the asking.
        {
            let db = state.workspace().expect("open");
            let db = db.db.lock();
            assert_eq!(
                db.count_matching(&crate::db::Filter::default())
                    .expect("count"),
                3
            );
        }

        let (_, done) = post(
            &state,
            &confirm_url(&offered),
            "delete_action=delete&scope=matching",
        )
        .await;
        // The table, not a sentence on its own — and the songs that went are not in it.
        assert!(done.contains("id=\"rows\""), "{done}");
        assert!(done.contains("2 songs"), "{done}");
        // The confirmation goes with it. A row left behind offers a button over songs that have
        // gone, and its hidden `song_id` boxes ride in `#bulk-delete` into the next press.
        assert!(
            done.contains(r#"<span id="bulk-delete-result" hx-swap-oob="true"></span>"#),
            "{done}"
        );
        let db = state.workspace().expect("open");
        let db = db.db.lock();
        assert_eq!(
            db.count_matching(&crate::db::Filter::default())
                .expect("count"),
            1,
            "the folder that was not named keeps its song"
        );
        assert_eq!(
            db.count_matching(&crate::db::Filter {
                deleted: crate::db::DeletedFilter::Only,
                ..crate::db::Filter::default()
            })
            .expect("count"),
            2,
            "and the two that went are reachable through the box that asks for them"
        );
    }

    /// A discarded song says so on its row, and a live one carries no such chip.
    ///
    /// **The row is the only thing that can say it.** Browsing hides these, so a row reached
    /// through *only deleted* is otherwise identical to a live one — same star, same *add to a
    /// package*, same play button — while every list, search and build leaves the song out.
    #[tokio::test]
    async fn a_discarded_song_says_so_on_its_row() {
        let (_corpus, state) = two_folders("deleted-chip");

        let (status, live) = get(&state, "/songs").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            !live.contains(r#"class="tag deleted""#),
            "nothing on the browse list is discarded: {live}"
        );

        let ticked = {
            let db = state.workspace().expect("open");
            let db = db.db.lock();
            db.songs(&crate::db::Filter::default()).expect("browse")[0]
                .id
                .clone()
        };
        let (_, _) = post(
            &state,
            "/songs/delete-bulk?offset=0&confirm=1",
            &format!("delete_action=delete&scope=ticked&song_id={ticked}"),
        )
        .await;

        // The list it is reachable through, and the chip that tells it from the rows beside it.
        let (status, only) = get(&state, "/songs?deleted=only").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(only.contains(r#"class="tag deleted""#), "{only}");
        assert_eq!(
            only.matches(r#"class="tag deleted""#).count(),
            1,
            "one chip, on the one song that was thrown away"
        );

        // And browsing still says nothing, because browsing has nothing to say it about.
        let (_, live) = get(&state, "/songs?deleted=").await;
        assert!(!live.contains(r#"class="tag deleted""#), "{live}");
    }

    /// The default is the ticked rows, and *only songs with no language yet* narrows the write.
    ///
    /// **Both halves matter.** A control that can only write filter-wide makes setting three songs
    /// a matter of describing them in the filter bar first — and *only songs with no language yet*
    /// is not the same as setting the bar's `language=unset`, which would take the rows being
    /// looked at off the page rather than narrowing the write.
    #[tokio::test]
    async fn the_language_set_takes_the_ticked_rows_and_can_spare_the_ones_already_said() {
        let (_corpus, state) = two_folders("bulk-ticked");
        let ids: Vec<String> = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .songs(&crate::db::Filter::default())
            .expect("browse")
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert!(ids.len() >= 2, "two folders hold more than one song");
        let ticked = |ids: &[String]| {
            ids.iter()
                .map(|id| format!("song_id={id}"))
                .collect::<Vec<_>>()
                .join("&")
        };

        // Nothing ticked is a refusal rather than a silent write over the corpus.
        let (_, refused) = post(&state, "/songs/language-bulk", "set_language=pt").await;
        assert!(refused.contains("Nothing is ticked."), "{refused}");

        // Two ticked, and the confirmation counts them rather than the filter.
        let two = &ids[..2];
        let body = format!("set_language=pt&{}", ticked(two));
        let (_, offered) = post(&state, "/songs/language-bulk", &body).await;
        assert!(offered.contains("<strong>2 songs</strong>"), "{offered}");
        assert!(offered.contains("2 ticked"), "{offered}");
        assert!(!offered.contains("the whole corpus"), "{offered}");

        let (_, done) = post(&state, &confirm_url(&offered), &body).await;
        assert!(done.contains("2 songs"), "{done}");

        // Now they have a language, so *only songs with no language yet* leaves them alone — which
        // is the whole of what that box is for.
        let spared = format!("set_language=it&only_unset=1&{}", ticked(two));
        let (_, refused) = post(&state, "/songs/language-bulk", &spared).await;
        assert!(
            refused.contains("Nothing ticked has an empty language."),
            "{refused}"
        );
    }

    /// One song's language, set from its row, and the row that comes back showing it.
    ///
    /// The counterpart of the bulk set above: that one writes to a whole filter and confirms first,
    /// this one is a select in a row and answers with the row. Both go through `edit_song`, so the
    /// canonicalising and the refusal of an unknown tag are the same on both paths.
    #[tokio::test]
    async fn a_row_can_set_one_songs_language() {
        let (_corpus, state) = two_folders("row-language");
        let id = state
            .workspace()
            .expect("a folder is open")
            .db
            .lock()
            .songs(&crate::db::Filter::default())
            .expect("browse")
            .first()
            .expect("a song")
            .id
            .clone();

        let (status, row) = post(&state, &format!("/songs/{id}/language"), "row_language=pt").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(row.contains(&format!("id=\"row-{id}\"")), "a row: {row}");
        let flat = row.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("value=\"pt\" selected"), "{flat}");

        // `more` is the short list's escape hatch and is not a language: it writes nothing and comes
        // back as the full picker. Asserted because the alternative — `Language::parse` refusing it —
        // would be an error message on an ordinary click.
        let (status, picker) = post(
            &state,
            &format!("/songs/{id}/language"),
            "row_language=more",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(picker.contains("value=\"cy\""), "the standard: {picker}");
        assert!(!picker.contains("value=\"more\""), "{picker}");

        // ...and the language it did not write is still what it was.
        let (_, row) = post(&state, &format!("/songs/{id}/language"), "row_language=").await;
        let flat = row.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            !flat.contains("value=\"pt\" selected"),
            "an empty choice clears it: {flat}"
        );
    }

    /// The row selects ride in the same body as the filter bar without breaking it.
    ///
    /// *Title from file name* sends `hx-include="#rows, #filters"`, so every row's language select is
    /// in that body beside the bar's own `language`. They are two different keys on purpose, and this
    /// is the end of the wire where getting it wrong shows up: a 400 on a button, rather than a
    /// deserializer test.
    #[tokio::test]
    async fn the_rows_language_selects_do_not_break_the_filter_bar() {
        let (_corpus, state) = two_folders("row-language-bar");
        let ids: Vec<String> = state
            .workspace()
            .expect("a folder is open")
            .db
            .lock()
            .songs(&crate::db::Filter::default())
            .expect("browse")
            .iter()
            .map(|row| row.id.clone())
            .collect();

        let ticked: String = ids
            .iter()
            .map(|id| format!("song_id={id}&row_language=pt&"))
            .collect();
        let (status, body) = post(
            &state,
            "/songs/titles-from-filename",
            &format!("{ticked}{}", bar("bossa")),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "a repeated `row_language` is ignored, a repeated `language` would be a 400: {body}"
        );
    }

    /// Every song of a scanned corpus, in browse order, for the hint tests below.
    fn all_ids(state: &State) -> Vec<String> {
        state
            .workspace()
            .expect("a folder is open")
            .db
            .lock()
            .songs(&crate::db::Filter::default())
            .expect("browse")
            .iter()
            .map(|row| row.id.clone())
            .collect()
    }

    /// The ticked rows come back numbered, and the badges are addressed to the rows themselves.
    ///
    /// The response is nothing but out-of-band swaps, so what is asserted is that each one names a
    /// row's own badge — the element `song_row.html` draws whether or not there is a number in it.
    #[tokio::test]
    async fn the_quality_hint_numbers_the_ticked_rows_best_first() {
        let (_corpus, state) = two_folders("quality-hint");
        let ids = all_ids(&state);
        let ticked: String = ids.iter().map(|id| format!("song_id={id}&")).collect();

        let (status, body) = post(&state, "/songs/quality-hint", &ticked).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(body.contains("Numbered 3 songs"), "{body}");
        for id in &ids {
            assert!(
                body.contains(&format!("id=\"hint-{id}\"")),
                "every ticked row gets a badge: {body}"
            );
        }
        assert_eq!(state.quality_hint().len(), 3);
    }

    /// The numbers survive a page turn, because the hint is the server's and not the response's.
    ///
    /// This is the whole reason the hint is held in [`State`] rather than being only the swap the
    /// button answered with: leaving the page and coming back to it is what curating a corpus of
    /// several hundred thousand files consists of.
    #[tokio::test]
    async fn a_hinted_row_draws_its_number_again_when_the_rows_are_redrawn() {
        let (_corpus, state) = two_folders("quality-hint-redraw");
        let ids = all_ids(&state);
        let ticked: String = ids.iter().map(|id| format!("song_id={id}&")).collect();
        post(&state, "/songs/quality-hint", &ticked).await;

        let hinted = state.quality_hint();
        let (status, rows) = get(&state, "/songs/rows").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        for (at, id) in hinted.iter().enumerate() {
            assert!(
                rows.contains(&format!(
                    "<span id=\"hint-{id}\" class=\"place\">{}</span>",
                    at + 1
                )),
                "a redrawn row carries the number the hint gave it: {rows}"
            );
        }
    }

    /// The similar-names page offers the hint, and a redrawn list of matches keeps its numbers.
    #[tokio::test]
    async fn the_similar_names_page_offers_the_hint_and_draws_its_numbers() {
        let (_corpus, state) = a_corpus_of("similar-hint", 3);

        let (status, html) = get(&state, "/similar?title=Song%200000&from=song-0000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains(r##"hx-post="/songs/quality-hint" hx-include="#hits""##),
            "{html}"
        );

        let ids = all_ids(&state);
        let ticked: String = ids.iter().map(|id| format!("song_id={id}&")).collect();
        post(&state, "/songs/quality-hint", &ticked).await;
        let hinted = state.quality_hint();
        assert!(!hinted.is_empty());

        let (status, hits) = get(&state, "/similar/hits?title=Song%200000&suitability=").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let listed: Vec<_> = hinted
            .iter()
            .enumerate()
            .filter(|(_, id)| hits.contains(&format!("id=\"row-{id}\"")))
            .collect();
        assert!(listed.len() >= 2, "the matches include hinted rows: {hits}");
        for (at, id) in listed {
            assert!(
                hits.contains(&format!(
                    "<span id=\"hint-{id}\" class=\"place\">{}</span>",
                    at + 1
                )),
                "a match carries the number the hint gave it: {hits}"
            );
        }
    }

    /// The similar-names page's actions are the Songs page's tabs, each form in its own panel, and
    /// the list of matches carries the box that ticks all of them.
    #[tokio::test]
    async fn the_similar_names_page_draws_its_actions_as_tabs_and_a_box_that_ticks_every_match() {
        let (_corpus, state) = a_corpus_of("similar-tabs", 3);

        let (status, html) = get(&state, "/similar?title=Song%200000&from=song-0000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains(r#"<nav class="tabstrip">"#), "{html}");
        // Quality is the first tab and the open one.
        assert!(html.contains(r#"id="curate-quality" checked>"#), "{html}");
        let strip = html
            .split(r#"<nav class="tabstrip">"#)
            .nth(1)
            .expect("a tab strip");
        assert!(
            strip.find("curate-quality") < strip.find("curate-favorites"),
            "{strip}"
        );
        for (tab, form) in [
            ("favorites", "favorite-file"),
            ("quality", "quality-hint"),
            ("titles", "titles-from-filename"),
            ("titles", "fix-name-case"),
        ] {
            assert!(html.contains(&format!(r#"id="curate-{tab}""#)), "{html}");
            let panel = html
                .split(&format!(r#"<div class="panel {tab}">"#))
                .nth(1)
                .unwrap_or_else(|| panic!("a {tab} panel: {html}"));
            let panel = panel
                .split(r#"<div class="panel "#)
                .next()
                .expect("a panel");
            assert!(
                panel.contains(&format!(r#"id="{form}""#)),
                "{form} sits in the {tab} panel: {panel}"
            );
        }

        let (_, hits) = get(&state, "/similar/hits?title=Song%200000&suitability=").await;
        let head = hits.split("</thead>").next().expect("a table head");
        assert!(head.contains(r#"class="select-all""#), "{head}");
        assert!(
            !head.contains("name="),
            "the head's box must not be submitted:\n{head}"
        );
    }

    /// Clearing takes every number away and says how many it took.
    ///
    /// An empty badge per row that had one, because a row keeps whatever markup it was last given —
    /// so a clear that sent nothing would leave the numbers on screen while the hint behind them
    /// was gone.
    #[tokio::test]
    async fn clearing_the_hint_rubs_out_every_number_it_put_there() {
        let (_corpus, state) = two_folders("quality-hint-clear");
        let ids = all_ids(&state);
        let ticked: String = ids.iter().map(|id| format!("song_id={id}&")).collect();
        post(&state, "/songs/quality-hint", &ticked).await;

        let (status, body) = post(&state, "/songs/quality-hint/clear", "").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(body.contains("Cleared 3 numbers"), "{body}");
        for id in &ids {
            assert!(
                body.contains(&format!("id=\"hint-{id}\"")),
                "every row that had a number is told it no longer has one: {body}"
            );
        }
        assert!(state.quality_hint().is_empty());
        for id in &ids {
            assert!(
                body.contains(&format!(
                    "<span id=\"hint-{id}\" hx-swap-oob=\"true\" class=\"place\"></span>"
                )),
                "the badge is emptied, which is what `.place:empty` takes off the row: {body}"
            );
        }
    }

    /// Ticking nothing says so, rather than answering with an empty hint.
    #[tokio::test]
    async fn a_hint_over_nothing_says_nothing_is_ticked() {
        let (_corpus, state) = two_folders("quality-hint-empty");
        let (status, body) = post(&state, "/songs/quality-hint", &bar("")).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(body.contains("Nothing is ticked"), "{body}");
        assert!(state.quality_hint().is_empty());
    }

    /// The hint form sends only ticks, so nothing in it can collide with the filter bar.
    ///
    /// It is `hx-include="#rows"` and not `"#filters, #rows"`, so the bar is not in the body at all
    /// — and `song_id` is a key `FilterQuery` does not know, which is what makes a hundred of them
    /// legal. This is the end of the wire where getting either wrong shows up as a button that 400s.
    #[tokio::test]
    async fn the_hint_form_posts_no_key_the_filter_bar_owns() {
        let (_corpus, state) = two_folders("quality-hint-keys");
        let ids = all_ids(&state);
        // The row's own controls ride along in `#rows`, exactly as they do for every other action
        // that includes it: a repeated `row_language` and a repeated `score` must both be ignored.
        let ticked: String = ids
            .iter()
            .map(|id| format!("song_id={id}&row_language=pt&score=&"))
            .collect();
        let (status, body) = post(&state, "/songs/quality-hint", &ticked).await;
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "the row's own keys are ignored rather than read: {body}"
        );
    }

    /// A filter that cannot be read stops the action, rather than becoming no filter at all.
    ///
    /// The tempting `unwrap_or_default()` here writes a package holding the whole corpus, so what is
    /// asserted is not only the message but that nothing was created.
    #[tokio::test]
    async fn an_unreadable_filter_refuses_instead_of_taking_everything() {
        let (_corpus, state) = two_folders("unreadable-filter");

        // One key, twice — which is what a second form reusing one of the bar's names would send.
        let (status, html) = post(
            &state,
            "/packages/from-filter",
            "name=Vol+1&artist=A&artist=B",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("could not read the filter"), "{html}");

        let packages = state
            .workspace()
            .expect("a folder is open")
            .db
            .lock()
            .packages()
            .expect("packages");
        assert!(packages.is_empty(), "nothing may be created: {packages:?}");
    }

    /// The rows bring the chips strip back with them, filtered or not.
    ///
    /// Both halves matter. With a filter, the strip has to say so — it is the page's only readout of
    /// what is narrowing the list, and it used to go on describing the page as it loaded. With none,
    /// the container still has to be *there*: htmx drops an out-of-band swap whose id is not on the
    /// page, so a strip that exists only when a filter is set is a strip that can never appear.
    #[tokio::test]
    async fn the_rows_bring_the_chips_strip_back_with_them() {
        let (_corpus, state) = two_folders("oob-chips");

        let (status, filtered) = get(&state, "/songs/rows?tags=bossa&filename=1").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(filtered.contains("id=\"rows\""), "{filtered}");
        assert!(
            filtered.contains("id=\"filter-chips\" hx-swap-oob=\"true\""),
            "the strip has to ride out of band, being outside #rows:\n{filtered}"
        );
        assert!(filtered.contains("tag: bossa"), "{filtered}");

        let (_, plain) = get(&state, "/songs/rows?filename=1").await;
        assert!(
            plain.contains("id=\"filter-chips\""),
            "the container is unconditional, or the swap has nowhere to land:\n{plain}"
        );
        assert!(!plain.contains(">showing<"), "{plain}");

        // And the page draws the same strip, once, so the two cannot drift.
        let (_, page) = get(&state, "/songs?tags=bossa").await;
        assert_eq!(page.matches("id=\"filter-chips\"").count(), 1, "{page}");
        assert!(page.contains("tag: bossa"), "{page}");
    }

    /// A filter change puts the filter in the address bar, so a reload keeps it.
    ///
    /// The third thing outside `#rows` that a filter change ought to move, and the only one no swap
    /// can reach. Before this, narrowing a corpus of hundreds of thousands of files and pressing F5
    /// landed back on all of them.
    #[tokio::test]
    async fn a_filter_change_is_pushed_into_the_address_bar() {
        use tower::ServiceExt;

        let (_corpus, state) = two_folders("push-url");
        let response = router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .uri("/songs/rows?tags=bossa&filename=1")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("the router answers");
        let pushed = response
            .headers()
            .get("HX-Push-Url")
            .and_then(|value| value.to_str().ok())
            .expect("a url to push");
        // `/songs`, not `/songs/rows`: what goes in the address bar has to be a page somebody can
        // reload into, not the fragment route that answered.
        assert!(pushed.starts_with("/songs?"), "{pushed}");
        assert!(pushed.contains("tags=bossa"), "{pushed}");
        // Never the carried count. A back button landing on a stale `total` would label the page
        // with a number the rows underneath it disagree with.
        assert!(!pushed.contains("total="), "{pushed}");
    }

    /// Clicking an artist narrows the list to that artist, end to end.
    ///
    /// Through the router because the parts that can be wrong are the ones a unit test does not
    /// reach: the link a row renders, the query parameter a page reads, and the clause the database
    /// runs all have to be the same filter. The link is taken **out of the rendered page** rather
    /// than written by hand here, which is what makes this a test of the round trip rather than of
    /// two guesses agreeing.
    #[tokio::test]
    async fn clicking_an_artist_narrows_the_list_to_that_artist() {
        let corpus = Scratch::new("artist-filter");
        for (path, bytes) in [
            ("A.kar", km_song::testing::soft_karaoke()),
            ("B.kar", km_song::testing::lyric_events()),
            ("C.kar", km_song::testing::named_text_track()),
        ] {
            std::fs::write(corpus.0.join(path), bytes).expect("write a fixture");
        }
        let state = State::new(Db::open_in_memory(&corpus.0).expect("open"));
        crate::scan::run(
            &state.workspace().expect("a folder is open").db,
            crate::scan::ScanOptions::default(),
            &std::sync::Arc::new(Progress::default()),
        )
        .expect("scan");

        // Two of the three are given one artist and the third another, by hand — the fixtures carry
        // whatever their own bytes say, and this test is about the filter rather than about parsing.
        let ids: Vec<String> = {
            let workspace = state.workspace().expect("a folder is open");
            let db = workspace.db.lock();
            let rows = db.songs(&crate::db::Filter::default()).expect("browse");
            rows.iter().map(|row| row.id.clone()).collect()
        };
        assert_eq!(ids.len(), 3, "the fixtures did not all land");
        {
            let workspace = state.workspace().expect("a folder is open");
            let db = workspace.db.lock();
            for (id, artist) in [
                (&ids[0], "Tom Jobim"),
                (&ids[1], "TOM JOBIM"),
                (&ids[2], "Dire Straits"),
            ] {
                db.edit_song(
                    id,
                    &crate::db::SongEdit {
                        artist: Some(Some(artist.to_owned())),
                        ..crate::db::SongEdit::default()
                    },
                )
                .expect("set an artist");
            }
        }

        // The link is the one the page draws, encoded the way a query string wants it.
        let (_, page) = get(&state, "/songs").await;
        assert!(
            page.contains(r#"href="/songs?artist=Tom+Jobim""#),
            "a row did not offer its artist as a link: {page}"
        );

        // And following it narrows to that artist — both spellings of it, and only them.
        let (_, filtered) = get(&state, "/songs?artist=Tom+Jobim").await;
        assert!(filtered.contains(&ids[0]), "{filtered}");
        assert!(
            filtered.contains(&ids[1]),
            "the other spelling of one artist was left out; the fold is not being applied"
        );
        assert!(
            !filtered.contains(&ids[2]),
            "a different artist survived the filter"
        );
        // The chip is the only thing on the page that admits the click narrowed anything, because
        // the link carries no other filter — the rule `SongRow::artist_url` records.
        assert!(
            filtered.contains("by Tom Jobim"),
            "no chip named the artist: {filtered}"
        );
    }

    /// The lyric search, end to end: scan a folder, then find a song by a line of its words.
    ///
    /// Through the router rather than through `Db`, because the parts that can be wrong here are the
    /// parts a unit test does not reach — a route that was never registered, a fragment whose target
    /// id does not match what the form aims at, a template that does not compile against its struct.
    #[tokio::test]
    async fn a_song_can_be_found_by_a_line_of_its_lyrics() {
        let corpus = Scratch::new("lyric-page");
        // Sings "Mary had a little lamb"; its name says none of that.
        std::fs::write(corpus.0.join("X1.mid"), km_song::testing::lyric_events())
            .expect("write a fixture");
        std::fs::write(corpus.0.join("X2.kar"), km_song::testing::soft_karaoke())
            .expect("write a fixture");

        let state = State::new(Db::open_in_memory(&corpus.0).expect("open"));
        let workspace = state.workspace().expect("a folder is open");
        crate::scan::run(
            &workspace.db,
            crate::scan::ScanOptions::default(),
            &std::sync::Arc::new(Progress::default()),
        )
        .expect("scan");

        let (status, html) = get(&state, "/lyrics?q=fleece").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("X1"),
            "the song that sings it is missing:\n{html}"
        );
        assert!(
            !html.contains("X2"),
            "a song that does not sing it came back:\n{html}"
        );
        assert!(
            html.contains("<mark>"),
            "the passage is not highlighted:\n{html}"
        );
        // The box keeps what was typed, or every page turn empties it.
        assert!(html.contains("value=\"fleece\""), "{html}");

        // The htmx fragment answers on its own, and its root element is what the form aims at.
        let (status, fragment) = get(&state, "/lyrics/hits?q=fleece").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(fragment.contains("id=\"hits\""), "{fragment}");
        assert!(
            !fragment.contains("<html"),
            "a fragment must not be a page:\n{fragment}"
        );

        // A word nobody sings finds nothing, and says so as *no match* rather than *not indexed*.
        let (_, none) = get(&state, "/lyrics?q=xyzzy").await;
        assert!(none.contains("Not found"), "{none}");

        // And the tab is reachable from every page, which is the only thing that makes it findable.
        let (_, songs) = get(&state, "/songs").await;
        assert!(
            songs.contains("href=\"/lyrics\""),
            "the nav has no Lyrics tab"
        );
    }

    /// A row's ≈ button opens the songs with a similar name, and that list is headed by the row's own
    /// song, marked.
    /// A name nothing is like keeps the song searched from, under the sentence that says so.
    ///
    /// The ≋ page's rule over the other index: the page answers about one song, and a sentence with
    /// no row under it leaves the reader without the song they asked about.
    #[tokio::test]
    async fn a_name_nothing_is_like_keeps_the_song_searched_from() {
        let (_corpus, state) = a_corpus_of("similar-no-match", 3);

        let (status, html) = get(
            &state,
            "/similar?title=Nothing%20Like%20It&from=song-0000&suitability=&kind=&granularity=&copies=",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let words = crate::words::messages(km_locale::Locale::English);
        assert!(html.contains(&*words.msg("similar-not-found")), "{html}");
        assert!(
            html.contains(r#"id="row-song-0000""#),
            "the song searched from is there: {html}"
        );
        assert!(html.contains("searched-from"), "{html}");
        assert!(
            !html.contains(r#"id="row-song-0001""#),
            "and nothing else is: {html}"
        );
    }

    #[tokio::test]
    async fn a_row_leads_to_the_songs_with_a_similar_name() {
        let (_corpus, state) = a_corpus_of("similar-page", 3);

        let (status, songs) = get(&state, "/songs").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let link = "/similar?title=Song+0000&#38;from=song-0000";
        assert!(songs.contains(link), "no ≈ link on the row:\n{songs}");

        let (status, html) = get(
            &state,
            "/similar?title=Song%200000&from=song-0000&suitability=",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("value=\"Song 0000\""), "{html}");
        assert!(html.contains("/songs/song-0001"), "{html}");
        let own = html
            .find("id=\"row-song-0000\"")
            .expect("the row's own song is listed");
        let other = html
            .find("id=\"row-song-0001\"")
            .expect("the other song is listed");
        assert!(own < other, "the row's own song is not first:\n{html}");
        assert!(html.contains("searched-from"), "{html}");
        // The likeness is a cell of the row, and a row whose redraw cannot know it keeps the one on
        // the page.
        assert!(html.contains("<table class=\"similar\">"), "{html}");
        assert!(
            html.contains(r#"id="likeness-song-0001" hx-preserve>50%</td>"#),
            "{html}"
        );
        let (_, row) = get(&state, "/songs/song-0001/row").await;
        assert!(
            row.contains(r#"id="likeness-song-0001" hx-preserve></td>"#),
            "{row}"
        );

        let (status, fragment) = get(&state, "/similar/hits?title=Song%200000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(fragment.contains("id=\"hits\""), "{fragment}");
        assert!(!fragment.contains("<html"), "{fragment}");

        // A filter narrows the matches, keeps the song searched from, and is drawn chosen.
        let (status, html) = get(
            &state,
            "/similar?title=Song%200000&from=song-0000&kind=video&copies=1",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("id=\"row-song-0000\""), "{html}");
        assert!(!html.contains("id=\"row-song-0001\""), "{html}");
        assert!(
            html.contains(r#"<option value="video" selected>"#),
            "{html}"
        );
        assert!(html.contains(r#"<option value="1" selected>"#), "{html}");

        // A ≈ link names no filter, so the next page opens narrowed as the last one was left.
        let (_, html) = get(&state, "/similar?title=Song%200001&from=song-0001").await;
        assert!(
            html.contains(r#"<option value="video" selected>"#),
            "{html}"
        );
        assert!(html.contains(r#"<option value="1" selected>"#), "{html}");

        // Every version is remembered like the rest, and drawn ticked.
        let (_, _) = get(
            &state,
            "/similar/hits?title=Song%200001&suitability=&kind=&granularity=&copies=&versions=all",
        )
        .await;
        let (_, html) = get(&state, "/similar?title=Song%200001&from=song-0001").await;
        assert!(
            html.contains(r#"name="versions" value="all" checked"#),
            "{html}"
        );

        // A control set back to *any* stays there: the bar sends every field, empty ones included,
        // and a cleared checkbox sends nothing.
        let (_, _) = get(
            &state,
            "/similar/hits?title=Song%200001&suitability=&kind=&granularity=&copies=",
        )
        .await;
        assert_eq!(
            state.similar_narrowing(),
            SimilarNarrowing {
                suitability: String::new(),
                kind: String::new(),
                granularity: String::new(),
                copies: String::new(),
                versions: String::new(),
            }
        );
        let (_, html) = get(&state, "/similar?title=Song%200000&from=song-0000").await;
        assert!(html.contains("id=\"row-song-0001\""), "{html}");
    }

    /// A fresh run opens the similar-names page at suitability 8–10, and the song searched from heads
    /// the list whatever its suitability.
    #[tokio::test]
    async fn a_fresh_run_opens_the_similar_names_page_at_the_high_band() {
        let (_corpus, state) = a_corpus_of("similar-high-band", 3);

        let (status, html) = get(&state, "/similar?title=Song%200000&from=song-0000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains(r#"<option value="8-10" selected>"#), "{html}");
        assert!(html.contains("id=\"row-song-0000\""), "{html}");
        assert!(!html.contains("id=\"row-song-0001\""), "{html}");
    }

    /// Leaving waits for the scan instead of killing it.
    ///
    /// This is the path Ctrl-C takes once the server has stopped: ask the run to stop, then *join*
    /// the thread. What it replaces is nothing at all — the scan was spawned detached and never
    /// joined, so `main` returning ended the process with the thread still holding a batch. The
    /// keystroke itself is the operating system's job and is not what this pins down; everything the
    /// keystroke reaches is.
    #[test]
    fn leaving_waits_for_a_running_scan_rather_than_killing_it() {
        let corpus = Scratch::new("stop-scan");
        for i in 0..30 {
            std::fs::write(
                corpus.0.join(format!("{i}.kar")),
                km_song::testing::soft_karaoke(),
            )
            .expect("write a fixture");
        }
        let db = Db::open_in_memory(&corpus.0).expect("open");
        let state = State::new(db);

        let workspace = state.workspace().expect("a folder is open");
        let progress = state
            .start_scan(crate::scan::ScanOptions::default())
            .expect("a folder is open, so a scan starts");
        assert!(workspace.stop_scan(), "there was a scan to stop");

        // `stop_scan` returning means the thread is joined, so the run is over by the time anything
        // reads it — no sleeping, no polling, no flake.
        let view = progress.snapshot();
        assert!(view.finished, "the thread was joined, so the run has ended");
        assert!(
            workspace.scan_thread_is_clear(),
            "the handle is consumed by the join"
        );

        // Whatever it managed to read is committed, because a batch is a transaction. It may have
        // read all thirty or none of them; either is correct and the count must simply agree.
        let counts = workspace.db.lock().counts().expect("counts");
        assert_eq!(u64::from(counts.files), view.written);

        // Stopping twice is harmless, which matters because Ctrl-C is a key somebody presses twice.
        assert!(!workspace.stop_scan(), "nothing left to stop");
    }

    /// With no folder open, everything that needs one goes to the picker instead of failing.
    ///
    /// The redirect is the whole reason handlers did not have to learn about the `Option`, so it is
    /// worth a test that drives the real router rather than trusting the middleware by inspection.
    #[tokio::test]
    async fn with_nothing_open_the_pages_redirect_and_the_picker_does_not() {
        use tower::ServiceExt;

        let state = State::empty();

        for path in ["/", "/songs", "/packages", "/scan", "/settings", "/lyrics"] {
            let response = router(state.clone())
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                axum::http::StatusCode::SEE_OTHER,
                "{path} should go to the picker"
            );
            assert_eq!(
                response
                    .headers()
                    .get(axum::http::header::LOCATION)
                    .and_then(|value| value.to_str().ok()),
                Some(OPEN_PATH)
            );
        }

        // The picker itself, and the assets every page needs to render at all. Without the second of
        // these the picker would arrive unstyled and scriptless — a worse first impression than the
        // error it replaced.
        for path in [
            OPEN_PATH,
            "/static/style.css",
            "/static/htmx.min.js",
            "/static/ui.js",
        ] {
            let response = router(state.clone())
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                axum::http::StatusCode::OK,
                "{path} must answer without a folder"
            );
        }
    }

    /// Every link the picker draws is a link the picker answers.
    ///
    /// **The crumb bar's ⏶ was a 400 for as long as it existed.** It sends `?drives=1`, `OpenQuery`
    /// bound `drives` as a plain `bool`, and serde's bool deserializer takes only `true` or `false` —
    /// so axum refused the request before the handler ran and htmx swapped
    /// `Failed to deserialize query string` into the listing. Nothing caught it: the route answered
    /// `/open/list` and `/open/list?at=…` perfectly well, and no test had ever pressed the one crumb
    /// that carries a flag.
    ///
    /// So this reads the URLs out of the rendered listing rather than naming them, because a test
    /// that hard-codes `?drives=1` stops testing the template the moment somebody edits it. What it
    /// asserts is the property the bug broke: a query string this tool *writes* is a query string
    /// this tool *reads*.
    #[tokio::test]
    async fn every_link_the_picker_draws_is_one_the_picker_answers() {
        let (corpus, state) = two_folders("picker-links");

        let root = urlencode(&corpus.0.display().to_string());
        let (status, html) = get(&state, &format!("/open/list?at={root}")).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{html}");

        // `hx-get` only. The listing's other buttons are `hx-post` with an `hx-vals` body, and the
        // song pages' links are templated with ids that need a live row; this is the one fragment
        // whose links are all self-contained GETs.
        let links: Vec<String> = html
            .split("hx-get=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .map(|link| link.replace("&amp;", "&"))
            .collect();

        // Without these the test passes on a template that draws nothing at all.
        assert!(
            links.iter().any(|link| link.contains("drives=")),
            "the ⏶ crumb is gone from the listing: {html}"
        );
        assert!(
            links.iter().any(|link| link.contains("at=")),
            "the walker's own links are gone from the listing: {html}"
        );

        for link in &links {
            let (status, body) = answered(&state, link).await;
            assert!(
                status.is_success(),
                "the listing draws {link}, and asking for it answered {status}: {body}"
            );
        }
    }

    /// An htmx request gets `HX-Redirect`, not a 302.
    ///
    /// The distinction is invisible in a browser and fatal in this tool: htmx follows a 302
    /// transparently and swaps the *result* into whatever element the button aimed at, so a 302 here
    /// would paint the whole picker inside a table cell with no error anywhere.
    #[tokio::test]
    async fn an_htmx_request_with_nothing_open_is_told_to_navigate() {
        use tower::ServiceExt;

        let state = State::empty();
        let response = router(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri("/songs/rows")
                    .header("hx-request", "true")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("hx-redirect")
                .and_then(|value| value.to_str().ok()),
            Some(OPEN_PATH),
            "htmx must be told to navigate rather than handed a page to swap"
        );
        assert!(
            response
                .headers()
                .get(axum::http::header::LOCATION)
                .is_none(),
            "a Location header would make htmx follow it and swap the picker"
        );
    }

    /// **No machine is a normal state, and every page has to draw in it.**
    ///
    /// The one thing this tool must never do is wait on a machine that is switched off — which is
    /// the ordinary case, since a corpus is curated for weeks and installed from once. Every page
    /// draws a header that names the machine, so a header that probed one would put a five-second
    /// timeout in front of every page in the tool.
    ///
    /// Asserted as a *timed* run rather than merely a passing one, because the failure this guards
    /// against is a page that works and is slow: `State::machine_shown` reading two settings and
    /// `State::app_client` reading one are both instant, and anything that grew a network call into
    /// either would show up here as seconds.
    #[tokio::test]
    async fn every_page_draws_with_no_machine_and_nothing_on_the_network() {
        use tower::ServiceExt;

        let (_corpus, state) = two_folders("no-machine");
        // Nothing has been told to this workspace, and `State::new` opened no watcher, so the
        // network is empty by construction — which is what a `cargo test` build always is.
        assert_eq!(state.chosen_machine().await, None);

        // **Settings is deliberately not in this list.** That page's job is to say whether the
        // machine is reachable, so it is the one place a probe belongs — and the one page somebody
        // opens *because* they are asking about the machine.
        for path in [
            "/",
            "/songs",
            "/lyrics",
            "/favorites",
            "/duplicates",
            "/packages",
            "/scan",
        ] {
            let started = std::time::Instant::now();
            let response = router(state.clone())
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert!(
                response.status().is_success() || response.status().is_redirection(),
                "{path} answered {} with no machine",
                response.status()
            );
            // Generous against the five-second timeout a probe would cost, so this cannot fail for
            // being run on a busy machine — and tight enough that a probe cannot hide in it.
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "{path} took {:?} with no machine, which means something asked the network",
                started.elapsed()
            );
        }

        // And Settings still answers rather than failing, which is the other half of *no machine is
        // a normal state*.
        let response = router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .uri("/settings")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert!(response.status().is_success(), "{}", response.status());
    }

    /// **Quit works on the picker, which is the one page it is the only way out of.**
    ///
    /// The reported fault: with no folder open, `POST /quit` fell through to `require_workspace`,
    /// which saw an htmx request and answered `HX-Redirect: /open` — so the button navigated to the
    /// page it was already on and the tool went on running. A control that visibly does nothing, on
    /// the page somebody reaches by double-clicking a corpus and has no console for.
    ///
    /// Neither of these two touches a workspace. `/browser` is here for the same reason and because
    /// the windowed build's picker offers it in Quit's place.
    #[tokio::test]
    async fn quitting_works_with_nothing_open() {
        use tower::ServiceExt;

        for path in ["/quit", "/browser"] {
            let state = State::empty();
            let response = router(state)
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri(path)
                        .header("hx-request", "true")
                        .body(axum::body::Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");

            assert_eq!(
                response.status(),
                axum::http::StatusCode::OK,
                "{path} answered {} with nothing open",
                response.status()
            );
            assert!(
                response.headers().get("hx-redirect").is_none(),
                "{path} was redirected to the picker instead of being run"
            );
        }
    }

    /// Asking again for the folder already being opened is answered, not scolded.
    ///
    /// This is the request the reported fault produced: the startup reopen is loading last time's
    /// corpus, the picker offers it in the Recent list, and clicking it used to answer
    /// `already opening N:\…` — a refusal for asking for exactly what was already happening. A
    /// *different* folder is still refused, because that one really is two answers to one question.
    ///
    /// The slot is seeded by hand rather than by racing a real open: what is under test is the guard,
    /// and a test that had to win a race against a database opening in a temp folder would be a
    /// test that passes on a fast machine.
    #[test]
    fn asking_for_the_folder_already_being_opened_is_not_an_error() {
        let corpus = Scratch::new("already-opening");
        let other = Scratch::new("already-opening-other");
        for folder in [&corpus, &other] {
            std::fs::write(folder.0.join(crate::db::DATABASE_NAME), b"")
                .expect("a database for `require_database` to find");
        }

        let state = State::empty();
        *state
            .opening
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(Opening::new(&corpus.0)));

        state
            .begin_open(corpus.0.clone(), false)
            .expect("the folder already being opened is answered with its progress");

        let Err(error) = state.begin_open(other.0.clone(), false) else {
            panic!("a second folder must not be opened while the first is still loading");
        };
        let said = error.to_string();
        assert!(
            said.contains("already opening") && said.contains(&corpus.0.display().to_string()),
            "the refusal does not name the folder that is holding it up: {said}"
        );

        // ...and once that job has ended, the same second folder is allowed. Without this the slot
        // would be a one-way door, which is what a missing `finished` check would look like.
        state
            .opening
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .expect("a job is in the slot")
            .finish(Some("stopped".to_owned()));
        state
            .begin_open(other.0.clone(), false)
            .expect("a finished job holds nothing up");

        // **That last call is the only one here that really starts a job**, and it opens a database
        // inside `other`. Left to itself it outlives the test: `Scratch`'s `Drop` runs while SQLite
        // still holds the file, Windows refuses the removal, and the temp folder stays for good —
        // sixteen of them had accumulated before anybody looked. So the test waits for the job it
        // started and closes what the job published, which is what lets the directory go.
        wait_for_the_open(&state);
        state.close_folder();
    }

    /// Opening a folder closes the one that was open before the new connection is made.
    ///
    /// This is the reported fault: a folder that is already open is an ordinary thing to click, and
    /// opening it used to run the whole migration on a second connection to the same file and then
    /// close the first with a `wal_checkpoint(TRUNCATE)` — which met the first page render as
    /// "database is locked".
    ///
    /// **The failing open is what pins it**, because it is the one arrangement where the two
    /// orderings give different answers. A zero-byte `.kmbuild` gets past `require_database`, which
    /// only looks for the file, and then fails inside `Db::prepare` — so a job that reached the
    /// database at all has closed what was open, and a job that never did has not.
    #[test]
    fn opening_a_folder_closes_the_one_that_was_open() {
        let corpus = Scratch::new("close-before-open");
        let broken = Scratch::new("close-before-open-broken");
        std::fs::write(broken.0.join(crate::db::DATABASE_NAME), b"not a database")
            .expect("a file for `require_database` to find");

        let state = State::new(Db::open_in_memory(&corpus.0).expect("open"));
        assert!(
            state.workspace().is_some(),
            "a folder is open to begin with"
        );

        // Refused before the thread is spawned, so nothing is closed: what can be answered cheaply
        // still is, and it costs the folder somebody is looking at nothing.
        assert!(
            state.begin_open(corpus.0.join("nowhere"), false).is_err(),
            "a folder that is not there is refused outright"
        );
        assert!(
            state.workspace().is_some(),
            "a refusal that never reached a database closed the folder anyway"
        );

        state
            .begin_open(broken.0.clone(), false)
            .expect("a database that is there is a job, and its unreadability is the job's answer");
        wait_for_the_open(&state);

        assert!(
            state
                .opening()
                .expect("the job is in the slot")
                .error
                .is_some(),
            "the open was supposed to fail on the way through the file"
        );
        assert!(
            state.workspace().is_none(),
            "the folder that was open is still open, so its connection was live while the next \
             database was being read"
        );
    }

    /// Closing hands the slot back before the checkpoint is paid.
    ///
    /// `*guard = None` drops the workspace *while the write guard is held*, so on a migration-sized
    /// journal every request waiting on [`State::workspace`] waited out a `wal_checkpoint(TRUNCATE)`
    /// — the Open page's own progress poll among them, which is the one page that has to keep
    /// answering while a folder is being swapped.
    #[test]
    fn closing_empties_the_slot_before_it_pays_for_the_close() {
        let corpus = Scratch::new("close-outside-the-lock");
        let state = State::new(Db::open_in_memory(&corpus.0).expect("open"));

        // Held from outside, so the drop cannot happen inside `close_folder` at all and what is left
        // is only the question this test asks: is the slot empty when it returns?
        let held = state.workspace().expect("a folder is open");
        state.close_folder();
        assert!(
            state.workspace().is_none(),
            "the slot still holds a workspace after closing"
        );
        drop(held);
    }

    /// *Open in browser* answers in the tray, like every other action that is over when it is said.
    ///
    /// The slot it would otherwise fill sits in the header, and a sentence there widens the one strip
    /// every page is measured against. The refusal is the reachable half — the success opens a real
    /// browser — and both go the same way, because a button that toasts when it works and reflows the
    /// header when it does not is two controls.
    #[tokio::test]
    async fn open_in_browser_says_so_in_the_toast_tray() {
        let state = State::empty();

        let (status, body) = post(&state, "/browser", "url=http://127.0.0.1:8178/songs").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            body.contains(r#"id="toasts""#) && body.contains("hx-swap-oob"),
            "not an out-of-band toast: {body}"
        );
        assert!(body.contains("toast-bad"), "{body}");
        assert!(
            !body.contains("class=\"message"),
            "a message fragment as well as a toast: {body}"
        );
    }

    /// Waits for the job in the opening slot to end, however it ends.
    ///
    /// Polled rather than joined because `begin_open` detaches its thread on purpose — the page
    /// polls it too, and a handle kept for a test would be a handle the tool does not need. The
    /// deadline is generous and its expiry is a failure: a job that never finishes is the fault
    /// [`EndsTheJob`] exists to prevent, so a test hanging quietly instead of reporting it would
    /// hide exactly the thing worth knowing.
    /// **The reported bug, end to end and with no window in it.**
    ///
    /// A corpus last opened by a newer build is what a double-click most often meets, and the
    /// refusal `migrate` makes for it has to reach a screen: in a process with no console, a reason
    /// that only gets returned reaches nothing. What this pins is the whole route from that refusal
    /// to the string the Open page draws in red.
    #[test]
    fn a_corpus_from_a_newer_build_says_so_on_the_open_page() {
        let corpus = Scratch::new("from-a-newer-build");
        {
            let db = Db::create(&corpus.0).expect("make a current database");
            db.as_if_from_a_newer_build();
        }

        let state = State::empty();
        state
            .begin_open(corpus.0.clone(), false)
            .expect("the database is there, so its version is the job's answer and not the call's");
        wait_for_the_open(&state);

        let job = state.opening().expect("the job is in the slot");
        let reason = job
            .error
            .expect("a database this build cannot read is a failure");
        assert!(
            reason.contains("newer build"),
            "the page is handed the refusal itself: {reason}"
        );
        assert!(
            state.workspace().is_none(),
            "a refused database was published anyway"
        );
    }

    /// A refusal made before any job could start is still left where the page looks.
    ///
    /// **The gap this closes is the synchronous half of [`State::begin_open`]**: a folder holding
    /// two databases is refused by returning, which a page that posted the folder renders and a
    /// start cannot. Without somewhere to put it, that refusal is the silence all over again.
    #[test]
    fn a_refusal_with_no_request_to_answer_is_left_on_the_page() {
        let corpus = Scratch::new("two-databases");
        std::fs::write(corpus.0.join("one.kmbuild"), b"x").expect("write");
        std::fs::write(corpus.0.join("two.kmbuild"), b"x").expect("write");

        let state = State::empty();
        let error = state
            .begin_open(corpus.0.clone(), false)
            .expect_err("two databases in one folder is refused without starting a job");
        assert!(
            state.opening().is_none(),
            "nothing started, so nothing is in the slot yet"
        );

        state.report_failed_open(&corpus.0, &error.to_string());
        let job = state.opening().expect("the reason is in the slot now");
        assert!(job.finished, "a report is a job that is already over");
        assert!(
            job.error.is_some_and(|reason| reason.contains("databases")),
            "the reason the call gave is the reason the page draws"
        );
    }

    /// A report never displaces a job somebody is watching.
    ///
    /// The one ordering that would matter: an open running while something else decides to leave a
    /// reason behind would replace a live progress panel with a dead sentence.
    #[test]
    fn a_report_leaves_a_running_open_alone() {
        let corpus = Scratch::new("report-waits-its-turn");
        let state = State::empty();
        state
            .begin_open(corpus.0.clone(), true)
            .expect("creating a database is a job");

        state.report_failed_open(&corpus.0, "a reason from somewhere else");
        wait_for_the_open(&state);

        let job = state.opening().expect("the job is in the slot");
        assert_eq!(
            job.error, None,
            "the report overwrote the open that was running"
        );
    }

    fn wait_for_the_open(state: &State) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            if state.opening().is_none_or(|job| job.finished) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the open never finished, so nothing here can say whether it worked");
    }

    /// A job ends once, whoever ends it — and something always does.
    ///
    /// Set `finished` only on the worker's own two paths and a panic in `Db::open` leaves the slot
    /// in flight for the life of the process, with *every* later open answering `already opening …`
    /// and nothing but a restart to clear it. [`EndsTheJob`] closes that; the first half here is
    /// what stops it introducing a worse fault, by overwriting the real outcome of a job that ended
    /// normally.
    #[test]
    fn a_job_ends_once_and_a_dropped_guard_ends_it_anyway() {
        let job = Arc::new(Opening::new(Path::new("/tunes/karaoke")));
        job.finish(None);
        drop(EndsTheJob(Arc::clone(&job)));

        let view = job.snapshot();
        assert!(view.finished, "the job ended");
        assert_eq!(
            view.error, None,
            "the guard overwrote the outcome of a job that had already succeeded"
        );

        // The case the guard exists for: nothing ever called `finish`, as after a panic.
        let abandoned = Arc::new(Opening::new(Path::new("/tunes/karaoke")));
        drop(EndsTheJob(Arc::clone(&abandoned)));
        let view = abandoned.snapshot();
        assert!(
            view.finished,
            "an abandoned job stays in flight, and every later open is refused"
        );
        assert!(
            view.error.is_some(),
            "an abandoned job ends silently, so the page says it opened"
        );
    }

    /// What the open says reaches the page, which is the half `db`'s own tests cannot see.
    ///
    /// Those prove `prepare` speaks; this proves somebody is listening. Between them sits the
    /// closure `begin_open` hands down, and the failure it guards against is the one this type spent
    /// its whole life in: a `phase` field that everything reads and nothing writes.
    #[test]
    fn what_the_open_says_is_what_the_page_shows() {
        let job = Opening::new(Path::new("/tunes/karaoke"));
        assert_eq!(
            job.snapshot().phase,
            crate::db::OpeningPhase::Database,
            "the step an open starts on"
        );

        job.set_phase(crate::db::OpeningPhase::Indexing { missing: 1 });
        assert_eq!(
            job.snapshot().phase,
            crate::db::OpeningPhase::Indexing { missing: 1 },
            "the page is still showing the step the job was built with"
        );

        // Replaced rather than appended to — the field is what it is doing *now*.
        job.set_phase(crate::db::OpeningPhase::Finishing);
        assert_eq!(job.snapshot().phase, crate::db::OpeningPhase::Finishing);

        // And the checklist beside it moved with every one of those, which is the half a field
        // holding only the current step cannot answer: what is over, and what there was nothing to
        // do. Nothing reported the six rungs between indexing and finishing, so they were passed.
        let states = |view: &OpeningView| {
            view.steps
                .iter()
                .map(|step| step.state)
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            states(&job.snapshot()),
            "skipped skipped skipped done skipped skipped skipped skipped skipped skipped running",
            "the rungs do not follow what the job was told"
        );

        job.finish(None);
        assert!(
            !states(&job.snapshot()).contains("running"),
            "a job that has ended leaves a rung claiming to be going on"
        );
    }

    /// The clock reaches the page too, and it runs from the job rather than from the poll.
    ///
    /// **It is the half that keeps moving when the phase cannot.** The longest step in an open is
    /// minutes inside one sentence, so without this the panel has a five-hundred-millisecond poll
    /// putting an identical fragment on the screen over and over. A snapshot taken from a job that
    /// started in the past must say so; one taken from a job that has just started must not.
    #[test]
    fn how_long_it_has_been_going_reaches_the_page() {
        let fresh = Opening::new(Path::new("/tunes/karaoke"));
        assert_eq!(
            fresh.snapshot().elapsed_secs,
            0,
            "a job that has just started claims time it has not spent"
        );

        let mut running = Opening::new(Path::new("/tunes/karaoke"));
        running.started = Instant::now() - std::time::Duration::from_secs(137);
        assert_eq!(
            running.snapshot().elapsed_secs,
            137,
            "the page shows nought however long an open has been running, so nothing on it moves"
        );
    }

    /// The picker really is handed the job, over the route a browser asks on.
    ///
    /// The template test in `views.rs` proves the markup is right *given* an `OpeningView`; what it
    /// cannot see is whether anything puts one there. That wiring is precisely what was missing —
    /// `OpenPage` had no such field, so the page was correct about a job it was never told about —
    /// and it is one forgotten line in `open_page` away from being missing again.
    #[tokio::test]
    async fn the_open_page_is_told_about_a_job_that_started_before_it() {
        let corpus = Scratch::new("picker-shows-opening");
        let state = State::empty();

        let (_, quiet) = get(&state, OPEN_PATH).await;
        assert!(
            !quiet.contains("/open/progress"),
            "the picker polls with nothing opening: {quiet}"
        );

        *state
            .opening
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(Opening::new(&corpus.0)));

        let (status, busy) = get(&state, OPEN_PATH).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            busy.contains(r#"hx-get="/open/progress""#),
            "the picker was not told about the folder already being opened: {busy}"
        );
    }

    /// The picker offers every language this build has, in each language's own name.
    #[tokio::test]
    async fn the_settings_page_offers_every_language_in_its_own_name() {
        let (_corpus, state) = two_folders("settings-locale");

        let (status, html) = get(&state, "/settings").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains(r#"hx-post="/settings/locale""#),
            "no language picker on the settings page: {html}"
        );
        for locale in km_locale::Locale::ALL {
            assert!(
                html.contains(locale.endonym()),
                "the picker does not offer {locale} by its own name: {html}"
            );
        }
    }

    /// Choosing a language stores it and asks for the whole page back.
    ///
    /// **`HX-Refresh` rather than a swap**, because every word on the document changes — and that is
    /// exactly what the second half asserts: the nav, which no `hx-target` on that form could have
    /// reached, comes back in the language just chosen.
    #[tokio::test]
    async fn choosing_a_language_redraws_the_whole_page_in_it() {
        use tower::ServiceExt;

        let (_corpus, state) = two_folders("locale-chosen");
        assert_eq!(state.locale(), km_locale::Locale::English);

        let request = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri("/settings/locale")
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(axum::body::Body::from("locale=pt-BR"))
            .expect("request");
        let response = router(state.clone())
            .oneshot(request)
            .await
            .expect("the router answers");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("hx-refresh")
                .map(|v| v.to_str().ok()),
            Some(Some("true")),
            "nothing asked the browser for the page back"
        );
        assert_eq!(state.locale(), km_locale::Locale::BrazilianPortuguese);

        let (_, html) = get(&state, "/settings").await;
        let portuguese = crate::words::messages(km_locale::Locale::BrazilianPortuguese);
        assert!(
            html.contains(portuguese.msg("nav-songs").as_ref()),
            "the nav did not follow the choice: {html}"
        );
    }

    /// A tag this build has no catalog for changes nothing, rather than refusing the page.
    #[tokio::test]
    async fn a_language_this_build_does_not_have_is_ignored() {
        let (_corpus, state) = two_folders("locale-unknown");

        let (status, _) = post(&state, "/settings/locale", "locale=de").await;

        assert_eq!(status, axum::http::StatusCode::NO_CONTENT);
        assert_eq!(state.locale(), km_locale::Locale::English);
    }

    /// No page draws a bracketed key, in either language.
    ///
    /// **The sweep that says the job is done.** A key missing from either catalog renders `⟦key⟧`
    /// rather than failing, which is `km-locale`'s deliberate answer — legible from across a room and
    /// quotable over a telephone — so nothing but a test stops one reaching a curator. The parity
    /// tests in `words` compare the catalogs against each other; this is what compares them against
    /// the pages.
    #[tokio::test]
    async fn no_page_draws_a_key_in_either_language() {
        let (_corpus, state) = a_corpus_of("every-page-every-language", 3);

        for locale in km_locale::Locale::ALL {
            state.set_locale(*locale);
            // Every page, and the fragments a page is redrawn out of. A song's own page and a
            // package's take an id, so they are asked for below.
            for path in [
                "/songs",
                "/songs/rows",
                "/lyrics",
                "/lyrics/hits?q=a",
                "/similar",
                "/similar?title=Song&from=song-0000",
                "/similar/hits?title=Song",
                "/similar-words",
                "/similar-words?from=song-0000",
                "/similar-words/hits?from=song-0000",
                "/favorites",
                "/duplicates",
                "/packages",
                "/scan",
                "/scan/progress",
                "/settings",
                "/open",
                "/open/list",
            ] {
                let (status, html) = get(&state, path).await;
                assert!(
                    status.is_success() || status.is_redirection(),
                    "{path} answered {status} in {locale}"
                );
                assert!(
                    !html.contains('⟦'),
                    "{path} draws an untranslated key in {locale}: {html}"
                );
            }

            // ...and the two pages that name something, plus the row fragment in each of its three
            // states, which is where most of a row's own words are.
            let id = first_song_id(&state).await;
            for path in [
                format!("/songs/{id}"),
                format!("/songs/{id}/row"),
                format!("/songs/{id}/row?editing=1"),
                format!("/songs/{id}/row?picking=1"),
            ] {
                let (status, html) = get(&state, &path).await;
                assert!(status.is_success(), "{path} answered {status} in {locale}");
                assert!(
                    !html.contains('⟦'),
                    "{path} draws an untranslated key in {locale}: {html}"
                );
            }
        }
    }

    /// Any song in the corpus, for a test that only needs a page that names one.
    async fn first_song_id(state: &State) -> String {
        state
            .blocking(|db| db.songs(&crate::db::Filter::default()))
            .await
            .expect("the corpus reads")
            .first()
            .expect("a song")
            .id
            .clone()
    }

    /// The panel is the only way most people will ever take a backup, so its two forms have to be on
    /// the page and have to post where the router listens.
    #[tokio::test]
    async fn the_settings_page_offers_a_backup_and_a_restore() {
        let (_corpus, state) = two_folders("settings-backup");

        let (status, html) = get(&state, "/settings").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains(r#"hx-post="/settings/backup""#),
            "no backup form on the settings page"
        );
        assert!(
            html.contains(r#"hx-post="/settings/restore""#),
            "no restore form on the settings page"
        );
        assert!(
            html.contains("kmbackup.json"),
            "the backup form does not offer a default path: {html}"
        );
        // The count is of songs somebody has touched, not of the corpus -- three songs were scanned
        // and nothing has been typed on any of them.
        assert!(
            html.contains("0 songs carry something you typed"),
            "the panel is counting the corpus rather than the corrections: {html}"
        );
    }

    /// This folder says how much is in the discard pile, which no other page does.
    ///
    /// The browse list shows what has been thrown away only to somebody who asked for it, so
    /// without this row a corpus holding discarded songs reads exactly like one holding none. Drawn
    /// at nought as well, because a page somebody opened to read facts answers rather than nags.
    #[tokio::test]
    async fn the_settings_page_says_how_much_has_been_thrown_away() {
        let (_corpus, state) = a_corpus_of("settings-discard-pile", 3);

        let (status, html) = get(&state, "/settings").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("Thrown away"),
            "the folder panel does not name the discard pile: {html}"
        );

        state
            .blocking(|db| {
                db.set_deleted_of(&["song-0001".to_owned()], true)
                    .map(|_| ())
            })
            .await
            .expect("throw one away");

        let (_, html) = get(&state, "/settings").await;
        let pile = html
            .split("Thrown away")
            .nth(1)
            .expect("the row is on the page");
        assert!(
            pile.starts_with("</dt><dd>1</dd>"),
            "the discard pile is not counted: {pile}"
        );
    }

    /// The password box is on the page, because three refusals send people to it by name.
    ///
    /// **This is the half that was missing for as long as the sentence existed.** `app.rs`'s
    /// `explain_upload_refusal` has told curators to type the password into a box on the machine
    /// panel since installing became an admin action, and `explain_uploads` has pointed at a
    /// Debugging button beside it; neither control had ever been built, and `Client::log_in` was
    /// reachable only from a test. An assertion here is what stops the two drifting apart again.
    #[tokio::test]
    async fn the_settings_page_offers_the_password_box_the_refusals_name() {
        let (_corpus, state) = two_folders("settings-password");

        let (status, html) = get(&state, "/settings").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains(r#"hx-post="/settings/login""#),
            "no password form on the settings page: {html}"
        );
        assert!(
            html.contains(r#"type="password""#),
            "the form has no password box: {html}"
        );
        // Signed out, so the controls a token pays for are not drawn at all rather than drawn and
        // refused.
        assert!(
            !html.contains(r#"hx-post="/settings/debugging""#),
            "the Debugging switch needs a token and must not be offered without one: {html}"
        );
        // Nothing has answered here, so there is no identity to remember a password under.
        assert!(
            !html.contains(r#"name="remember""#),
            "a machine that has never answered cannot be remembered: {html}"
        );
    }

    /// A test run looks for a machine at a port nothing can be listening on.
    ///
    /// The property the `cfg(test)` guard on [`DEFAULT_APP_URL`] exists for, asserted in the crate
    /// the guard is in — [`crate::passwords`] pins its own the same way. Every page drawing the
    /// machine panel discovers first and takes the id the reply carries, so a suite reaching a real
    /// machine renders somebody else's identity into tests that never mention one.
    #[test]
    fn a_test_run_looks_for_a_machine_where_none_can_be() {
        let port = DEFAULT_APP_URL.rsplit(':').next().expect("a port");
        assert_eq!(
            port, "0",
            "a test run must reach a port nothing can bind, and 0 is the only one"
        );
    }

    /// A password this computer has saved is said so, and the box goes behind a summary.
    ///
    /// **This is the state most launches open in.** A remembered password is a standing instruction
    /// to sign in rather than a sign-in that has happened, so the token is bought at the moment one
    /// is wanted — which means the panel is drawn signed out with the password already on this
    /// computer. A box on its own there asks for something this program is holding, and the answer
    /// to *do I have to type this again* was nowhere on the signed-out half at all.
    #[tokio::test]
    async fn a_saved_password_is_said_so_with_the_box_behind_a_summary() {
        let (_corpus, state) = two_folders("settings-password-saved");
        // A machine that has answered, which is what gives a password an id to be keyed under.
        state
            .blocking(|db| {
                crate::chosen::save(db, DEFAULT_APP_URL)?;
                crate::chosen::answered(
                    db,
                    DEFAULT_APP_URL,
                    "abc123",
                    Some("Living Room".to_owned()),
                )
            })
            .await
            .expect("the machine is recorded");
        state.remember_password("abc123", Some("first1975"));

        let (status, html) = get(&state, "/settings").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(!state.signed_in(), "the token is bought later, not here");
        assert!(
            // Not the whole sentence: an apostrophe reaches the page as an entity.
            html.contains("This computer has this machine"),
            "the panel does not say what this computer holds: {html}"
        );
        // The way out, beside the sentence rather than only in the signed-in half.
        assert!(
            html.contains(r#"hx-post="/settings/logout""#),
            "no way to stop remembering without signing in first: {html}"
        );
        // The box is one press away rather than the first thing to fill in...
        assert!(
            html.contains(r#"<details class="retype">"#),
            "the box is still asking for a password this computer has: {html}"
        );
        // ...and it cannot be `required` there: a required field inside a closed disclosure is a
        // form the browser refuses to submit and cannot say why.
        assert!(
            !html.contains("current-password\" required"),
            "a required box behind a summary blocks Sign in: {html}"
        );
    }

    /// A blank box with nothing saved is turned away rather than sent to the machine.
    ///
    /// Blank means *spend what this computer has saved*, so a pass with nothing to fall back on is
    /// answered here — the machine has no part in it, and an empty password on the wire would come
    /// back as a refusal about the password rather than about the box.
    #[tokio::test]
    async fn a_blank_box_with_nothing_saved_asks_for_the_password() {
        let (_corpus, state) = two_folders("settings-password-blank");

        let (status, html) = post(&state, "/settings/login", "password=").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(!state.signed_in());
        assert!(
            // Not the whole sentence: an apostrophe reaches the page as an entity.
            html.contains("password first."),
            "a blank box was not answered on the page: {html}"
        );
    }

    /// A password the machine will not take leaves the panel exactly where it was.
    ///
    /// The address in a fresh workspace is the loopback default with nothing on it, so this is the
    /// unreachable case rather than the wrong-password one — which is the same requirement of the
    /// handler: answer the fragment with the reason, still signed out, rather than an error page.
    #[tokio::test]
    async fn a_sign_in_that_fails_says_so_and_stays_signed_out() {
        let (_corpus, state) = two_folders("settings-login-fails");

        let (status, html) = post(&state, "/settings/login", "password=hunter2").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            !state.signed_in(),
            "nothing answered, so nothing is signed in"
        );
        assert!(
            html.contains(r#"hx-post="/settings/login""#),
            "the box has to still be there to try again: {html}"
        );
        assert!(
            html.contains("class=\"message error\""),
            "the reason is not shown: {html}"
        );
    }

    /// Two requests share one token, which is the whole reason it is on `State`.
    ///
    /// **The bug this is written against is not hypothetical**: `app_client` builds a fresh `Client`
    /// per request, so a token owned by the client would be dropped between the Settings form that
    /// obtained it and the Install button that needs it — and the box would have looked like it
    /// worked, once, on the page that took the password.
    #[tokio::test]
    async fn the_token_outlives_the_client_that_obtained_it() {
        let (_corpus, state) = two_folders("settings-token-shared");

        let first = state.app_client().await;
        let second = state.app_client().await;
        assert!(!first.logged_in() && !second.logged_in());

        // Reaching past the network, which is what a unit test here can do: the slot is the
        // contract, and `log_in` is the only thing that fills it.
        state
            .token
            .lock()
            .expect("the token slot")
            .replace("a-token".to_owned());
        assert!(first.logged_in(), "a client built before the sign-in");
        assert!(second.logged_in(), "and one built after it");
        assert!(state.app_client().await.logged_in(), "and one built since");

        // Choosing a machine signs out of the last one; moving to a new address would not.
        state.forget_token();
        assert!(!first.logged_in() && !state.signed_in());
    }

    /// A message rather than a broken button. `MessageFragment::failed` is deliberately a 200
    /// because htmx will not swap a non-2xx, so a 500 here is a Restore button that does nothing
    /// visible at all.
    #[tokio::test]
    async fn restoring_from_a_file_that_is_not_there_says_so_rather_than_failing_the_request() {
        let (_corpus, state) = two_folders("settings-restore-missing");

        let (status, html) = post(&state, "/settings/restore", "path=nowhere.json").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("nowhere.json"),
            "the failure has to name the file it could not read: {html}"
        );
    }

    /// The round trip through the two routes a person actually presses.
    #[tokio::test]
    async fn a_backup_written_from_the_page_restores_from_the_page() {
        let (corpus, state) = two_folders("settings-round-trip");
        let id = {
            let workspace = state.workspace().expect("a folder is open");
            let db = workspace.db.lock();
            let id = db
                .songs(&crate::db::Filter::default())
                .expect("songs")
                .first()
                .expect("a song")
                .id
                .clone();
            db.edit_song(
                &id,
                &crate::db::SongEdit {
                    title: Some(Some("Corcovado".to_owned())),
                    ..crate::db::SongEdit::default()
                },
            )
            .expect("edit");
            id
        };

        let out = crate::db::data_dir(&corpus.0).join("kept.kmbackup.json");
        let (status, html) = post(
            &state,
            "/settings/backup",
            &format!("out={}", urlencode(&out.display().to_string())),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("1 song"),
            "one song was corrected and one should be in the file: {html}"
        );

        // Undo the correction, then ask for it back.
        {
            let workspace = state.workspace().expect("a folder is open");
            let db = workspace.db.lock();
            db.edit_song(
                &id,
                &crate::db::SongEdit {
                    title: Some(None),
                    ..crate::db::SongEdit::default()
                },
            )
            .expect("clear it");
        }

        let (status, html) = post(&state, "/settings/restore", "path=kept.kmbackup.json").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("Restored 1 song,"), "{html}");

        let workspace = state.workspace().expect("a folder is open");
        let db = workspace.db.lock();
        assert_eq!(
            db.song(&id).expect("song").title.as_deref(),
            Some("Corcovado"),
            "a relative path found the file and the title came back"
        );
    }

    /// Form encoding for a test body, for a Windows path full of backslashes.
    fn urlencode(value: &str) -> String {
        form_urlencoded::byte_serialize(value.as_bytes()).collect()
    }

    /// A filter lives as long as the folder it names is open.
    ///
    /// The way out is `close_folder`, where `crate::finish` and the event loop both arrive, and a
    /// start is `State::new` doing a publish — so a folder opened again is opened whole, and a
    /// morning's work is come back to by the name it was saved under.
    #[test]
    fn a_filter_does_not_outlive_the_folder_it_names() {
        let corpus = Scratch::new("filter-not-outliving");
        let state = State::new(Db::open_in_memory(&corpus.0).expect("open"));
        state.remember(&corpus.0, 0, 0);
        state.remember_songs_filter("language=pt&sort=updated".to_owned());

        state.close_folder();
        assert_eq!(state.songs_filter(), "");

        state.publish(Arc::new(Workspace::new(
            Db::open_in_memory(&corpus.0).expect("reopen"),
        )));
        assert_eq!(state.songs_filter(), "");
    }

    /// Another corpus opens on the whole corpus, whatever the last one was filtered to.
    ///
    /// The fault this closes is the one `publish` describes: a filter names a folder and a favorite
    /// id out of one corpus, so carried into another it points at rows that do not exist and the
    /// chips explain the empty page in the vocabulary of a corpus nobody is looking at.
    #[test]
    fn another_corpus_does_not_inherit_a_filter() {
        let corpus = Scratch::new("filter-not-inherited");
        let other = Scratch::new("filter-not-inherited-other");
        let state = State::new(Db::open_in_memory(&corpus.0).expect("open"));
        state.remember(&corpus.0, 0, 0);
        state.remember_songs_filter("language=pt".to_owned());

        state.publish(Arc::new(Workspace::new(
            Db::open_in_memory(&other.0).expect("open the other one"),
        )));
        assert_eq!(state.songs_filter(), "");
    }

    #[test]
    fn only_a_favorite_nothing_carries_is_taken_out_of_a_filter() {
        let live = vec![crate::model::FavoriteNode {
            id: 7,
            name: "Bossa".to_owned(),
            song_count: 0,
            second_copies: 0,
            temporary: false,
        }];

        assert_eq!(
            without_missing_favorite("language=pt&favorite=7&sort=title", &live),
            "language=pt&favorite=7&sort=title",
            "a favorite that is still there is still a filter"
        );
        assert_eq!(
            without_missing_favorite("language=pt&favorite=9&sort=title", &live),
            "language=pt&sort=title"
        );
        assert_eq!(
            without_missing_favorite("favorite=9", &live),
            "",
            "and a filter that was only that reads as the whole corpus"
        );
        // `favorited` is a different filter whose name begins the same way, and it is not an id.
        assert_eq!(
            without_missing_favorite("favorited=in", &live),
            "favorited=in"
        );
    }

    /// A bare `/songs` is answered with the filter the folder was left on.
    ///
    /// **The one test that drives a request**, and the reason the restore was invisible without it:
    /// every other test here asserts on `State::songs_filter`, which is written correctly at startup
    /// and then overwritten by the first render of a page that arrived with nothing in its address.
    /// A start goes `/` → `/songs`, so that render is the first thing anybody sees.
    #[tokio::test]
    async fn a_bare_songs_address_is_answered_with_the_remembered_filter() {
        let (_corpus, state) = two_folders("bare-songs-address");
        state.remember_songs_filter("language=pt&offset=50".to_owned());

        let (status, location) = sent_to(&state, "/songs").await;
        assert_eq!(status, axum::http::StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/songs?language=pt&offset=50"));
        assert_eq!(
            state.songs_filter(),
            "language=pt&offset=50",
            "the redirect must not spend what it is redirecting to"
        );

        // And `/` arrives at the same place, one hop earlier.
        let (_, index) = sent_to(&state, "/").await;
        assert_eq!(index.as_deref(), Some("/songs"));
    }

    /// *clear all* clears, and is not handed back what it just took off.
    ///
    /// The two addresses differ by one character: the bar's *clear all* is `/songs?` once the last
    /// filter comes off, and everything that means *the songs page* with nothing to say sends
    /// `/songs`. That is the whole distinction, so it is the whole test.
    #[tokio::test]
    async fn clearing_the_bar_is_not_answered_with_the_filter_it_cleared() {
        let (_corpus, state) = two_folders("clear-all");
        state.remember_songs_filter("language=pt".to_owned());

        let (status, location) = sent_to(&state, "/songs?").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(location, None);
        // **What must not survive a clear is a filter, and a total is not one.** The render writes
        // down the count it just paid for, so that arriving at the tab again does not pay it twice;
        // `FilterQuery::total` decides no row and no button. So the assertion is on the filters
        // being gone rather than on the string being empty.
        let remembered = state.songs_filter();
        assert!(
            !remembered.contains("language="),
            "the clear has to stick, and {remembered} still narrows"
        );
        assert_eq!(remembered, "total=3", "and what is left is only the count");
    }

    // -- filters somebody named ------------------------------------------------------------------

    /// Saving sends no filter, and the right one is written down anyway.
    ///
    /// **This is the test for the whole design.** The save control deliberately does not
    /// `hx-include="#filters"`, on the grounds that `/songs/rows` has already written the canonical
    /// query string into the state — so what this pins is that claim, `offset` included. It fails
    /// the moment a route that redraws `#rows` stops writing the filter down.
    #[tokio::test]
    async fn saving_a_filter_writes_down_what_is_on_screen() {
        let (_corpus, state) = two_folders("save-filter");
        let (status, _) = get(&state, "/songs/rows?language=pt&sort=updated").await;
        assert_eq!(status, axum::http::StatusCode::OK);

        let (status, said) = post(
            &state,
            "/songs/saved-filters",
            "saved_name=Brasil&keep_page=1",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(said.contains("Saved as Brasil"), "{said}");

        let saved = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .saved_filters()
            .expect("read");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "Brasil");
        assert_eq!(saved[0].query, "language=pt&sort=updated");
    }

    /// Every sentence the saved strip says is a toast, the save box's included.
    ///
    /// The slot under the bar has nobody watching it and nothing to clear it, so a sentence left
    /// there reads an hour later as something that has just happened — and it sits above a page of
    /// rows, where the button that produced it may be off the screen. What still belongs in the slot
    /// is the collision confirmation, which is a fragment carrying buttons rather than a sentence,
    /// and the test below holds that half.
    #[tokio::test]
    async fn the_save_box_answers_with_a_toast() {
        let (_corpus, state) = two_folders("save-filter-toast");
        let (_, said) = post(&state, "/songs/saved-filters", "saved_name=Brasil").await;
        assert!(said.contains(r#"hx-swap-oob="afterbegin""#), "{said}");
        assert!(said.contains("toast-good"), "{said}");
        assert!(
            !said.contains(r#"class="message"#),
            "a sentence in the slot outlives the act it describes:\n{said}"
        );

        // A refusal goes the same way, and takes the slot's contents with it.
        let (_, refused) = post(&state, "/songs/saved-filters", "saved_name=").await;
        assert!(refused.contains("toast-bad"), "{refused}");
        assert!(!refused.contains(r#"class="message"#), "{refused}");
    }

    /// A name that is taken still answers in the slot, because a confirmation is not a sentence.
    #[tokio::test]
    async fn a_name_that_is_taken_answers_with_the_confirmation_in_the_slot() {
        let (_corpus, state) = two_folders("save-filter-confirm");
        post(&state, "/songs/saved-filters", "saved_name=Brasil").await;

        let (_, again) = post(&state, "/songs/saved-filters", "saved_name=Brasil").await;
        assert!(again.contains(r#"id="saved-filter-confirm""#), "{again}");
        assert!(
            !again.contains("toast-"),
            "a confirmation carries buttons and has to stay on the page:\n{again}"
        );
    }

    /// The page travels with it, and the box is what takes it off.
    #[tokio::test]
    async fn leaving_the_page_out_saves_everything_else() {
        let (_corpus, state) = two_folders("save-filter-page");
        state.remember_songs_filter("language=pt&sort=updated&offset=150".to_owned());

        post(
            &state,
            "/songs/saved-filters",
            "saved_name=Kept&keep_page=1",
        )
        .await;
        // An unticked checkbox sends nothing at all, which is the whole of how it is read.
        post(&state, "/songs/saved-filters", "saved_name=Dropped").await;

        let saved = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .saved_filters()
            .expect("read");
        let query = |name: &str| {
            saved
                .iter()
                .find(|filter| filter.name == name)
                .map(|filter| filter.query.clone())
                .expect("saved")
        };
        assert_eq!(query("Kept"), "language=pt&sort=updated&offset=150");
        assert_eq!(query("Dropped"), "language=pt&sort=updated");
    }

    /// A name that is taken is a question, not a write.
    #[tokio::test]
    async fn a_name_that_is_taken_asks_before_it_replaces() {
        let (_corpus, state) = two_folders("save-filter-taken");
        state.remember_songs_filter("kind=video".to_owned());
        post(
            &state,
            "/songs/saved-filters",
            "saved_name=Videos&keep_page=1",
        )
        .await;

        state.remember_songs_filter("kind=midi".to_owned());
        let (status, asked) = post(
            &state,
            "/songs/saved-filters",
            "saved_name=Videos&keep_page=1",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(asked.contains("confirm=1"), "it has to ask: {asked}");
        assert!(asked.contains("already saved"), "{asked}");

        let still = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .saved_filters()
            .expect("read");
        assert_eq!(still.len(), 1, "and write nothing while it asks");
        assert_eq!(still[0].query, "kind=video");
    }

    /// …and the replacement writes what the confirmation promised, not the bar as it now stands.
    ///
    /// The bar is live while a confirmation sits on screen, so somebody can narrow the list between
    /// reading the sentence and pressing the button. The sentence is what has to be kept.
    #[tokio::test]
    async fn the_replacement_writes_what_the_confirmation_showed() {
        let (_corpus, state) = two_folders("save-filter-confirm");
        state.remember_songs_filter("kind=video".to_owned());
        post(
            &state,
            "/songs/saved-filters",
            "saved_name=Videos&keep_page=1",
        )
        .await;

        // What the confirmation was shown, and then the bar moving underneath it.
        state.remember_songs_filter("kind=midi".to_owned());
        post(
            &state,
            "/songs/saved-filters",
            "saved_name=Videos&keep_page=1",
        )
        .await;
        state.remember_songs_filter("language=ja".to_owned());

        let (status, said) = post(
            &state,
            "/songs/saved-filters?confirm=1",
            "saved_name=Videos&saved_query=kind%3Dmidi",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(said.contains("Saved as Videos"), "{said}");

        let saved = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .saved_filters()
            .expect("read");
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved[0].query, "kind=midi",
            "not the filter that arrived later"
        );
    }

    /// A blank name writes nothing and says so.
    #[tokio::test]
    async fn a_saved_filter_with_no_name_is_refused() {
        let (_corpus, state) = two_folders("save-filter-blank");
        state.remember_songs_filter("kind=video".to_owned());

        let (status, said) = post(&state, "/songs/saved-filters", "saved_name=%20%20").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(said.contains("Name it first"), "{said}");
        assert!(
            state
                .workspace()
                .expect("open")
                .db
                .lock()
                .saved_filters()
                .expect("read")
                .is_empty()
        );
    }

    /// A favorite that has gone is taken off the link, and left in the row.
    ///
    /// The scrub is at render, because restoring is a bare `<a href>` and there is no handler to put
    /// one in. Rewriting the row instead would be the tool editing something somebody typed.
    #[tokio::test]
    async fn a_saved_filter_that_names_a_gone_favorite_is_offered_without_it() {
        let (_corpus, state) = two_folders("save-filter-stale");
        let favorite = {
            let workspace = state.workspace().expect("open");
            let db = workspace.db.lock();
            let id = db.create_favorite("Bossa").expect("favorite");
            db.save_filter(
                "Filed",
                &format!("favorite={id}&language=pt"),
                "2026-09-11T10:00:00Z",
            )
            .expect("save");
            id
        };

        let (_, before) = get(&state, "/songs?").await;
        // `&#38;` and not `&`: askama escapes the attribute, as it does for every chip link on the
        // page.
        assert!(
            before.contains(&format!("/songs?favorite={favorite}&#38;language=pt")),
            "while the favorite is there it is offered whole: {before}"
        );

        state
            .workspace()
            .expect("open")
            .db
            .lock()
            .delete_favorite(favorite)
            .expect("delete");

        let (_, after) = get(&state, "/songs?").await;
        assert!(
            after.contains(r#"href="/songs?language=pt""#),
            "the link drops it: {after}"
        );
        assert!(
            !after.contains(&format!("favorite={favorite}")),
            "and names it nowhere: {after}"
        );
        let stored = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .saved_filters()
            .expect("read");
        assert_eq!(
            stored[0].query,
            format!("favorite={favorite}&language=pt"),
            "the row itself is untouched"
        );
    }

    /// A saved whole-corpus filter restores to the corpus and not to the cursor.
    ///
    /// One character, and it is the same one
    /// [`clearing_the_bar_is_not_answered_with_the_filter_it_cleared`] turns on: a bare `/songs` is
    /// answered with the remembered filter, so the link has to carry its `?` whether or not anything
    /// follows it.
    #[tokio::test]
    async fn a_saved_whole_corpus_filter_restores_to_the_corpus() {
        let (_corpus, state) = two_folders("save-filter-everything");
        state
            .workspace()
            .expect("open")
            .db
            .lock()
            .save_filter("Everything", "", "2026-09-11T10:00:00Z")
            .expect("save");

        let (_, html) = get(&state, "/songs?").await;
        assert!(html.contains(r#"href="/songs?""#), "{html}");
    }

    /// Forgetting one takes it off the page, and brings the strip back with the answer.
    #[tokio::test]
    async fn forgetting_a_saved_filter_takes_it_off_the_page() {
        let (_corpus, state) = two_folders("save-filter-forget");
        let id = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .save_filter("Videos", "kind=video", "2026-09-11T10:00:00Z")
            .expect("save");

        let (status, said) = post(&state, &format!("/songs/saved-filters/{id}/delete"), "").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(said.contains("Forgotten"), "{said}");
        assert!(
            said.contains(r#"id="saved-filters""#) && said.contains("hx-swap-oob"),
            "the strip rides along: {said}"
        );

        let (_, html) = get(&state, "/songs?").await;
        assert!(!html.contains("Videos"), "{html}");
    }

    /// Rewriting a named filter takes what is on screen, and says so by name.
    ///
    /// The sentence names the filter rather than either query: both are lines of `key=value` that
    /// mean nothing read back, and the row on screen is the one that was pressed.
    #[tokio::test]
    async fn rewriting_a_saved_filter_takes_the_filter_on_screen() {
        let (_corpus, state) = two_folders("saved-rewrite");
        state.remember_songs_filter("language=pt".to_owned());
        post(&state, "/songs/saved-filters", "saved_name=Curating").await;
        let id = only_saved(&state).id;

        state.remember_songs_filter("language=pt&favorited=out".to_owned());
        let (status, said) = post(&state, &format!("/songs/saved-filters/{id}/update"), "").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(said.contains("Curating was updated."), "{said}");
        assert!(
            !said.contains("favorited=out&#38;") && !said.contains("no longer"),
            "neither query is spelled at somebody: {said}"
        );
        assert!(
            said.contains(r#"id="saved-filters""#) && said.contains("hx-swap-oob"),
            "the strip rides along: {said}"
        );
        assert!(
            said.contains(r#"id="toasts""#),
            "and the sentence is a toast, which goes: {said}"
        );

        let held = only_saved(&state);
        assert_eq!(held.id, id, "the same row");
        assert_eq!(held.name, "Curating");
        assert_eq!(held.query, "language=pt&favorited=out");
    }

    /// **The page comes or does not come by what the row already held.**
    ///
    /// A chip has nowhere to put the save box's *keep the page* tick, so the rule is read off the
    /// filter being rewritten: one saved as a place stays a place, and one saved as a question is
    /// not turned into a place by being brought up to date.
    #[tokio::test]
    async fn rewriting_keeps_a_saved_filter_the_kind_of_filter_it_was() {
        let (_corpus, state) = two_folders("saved-rewrite-page");
        state.remember_songs_filter("language=pt&offset=150".to_owned());
        post(
            &state,
            "/songs/saved-filters",
            "saved_name=Place&keep_page=1",
        )
        .await;
        let place = only_saved(&state).id;

        state.remember_songs_filter("kind=video".to_owned());
        post(&state, "/songs/saved-filters", "saved_name=Question").await;

        let question = saved_named(&state, "Question").id;
        state.remember_songs_filter("language=pt&offset=800".to_owned());
        post(&state, &format!("/songs/saved-filters/{place}/update"), "").await;
        post(
            &state,
            &format!("/songs/saved-filters/{question}/update"),
            "",
        )
        .await;

        assert_eq!(saved_named(&state, "Place").query, "language=pt&offset=800");
        assert_eq!(saved_named(&state, "Question").query, "language=pt");
    }

    /// The chip opens for renaming and closes again, and neither touches the row.
    #[tokio::test]
    async fn a_chip_opens_for_renaming_and_comes_back_closed() {
        let (_corpus, state) = two_folders("saved-rename-chip");
        state.remember_songs_filter("kind=video".to_owned());
        post(&state, "/songs/saved-filters", "saved_name=Videos").await;
        let id = only_saved(&state).id;

        let (status, open) = get(
            &state,
            &format!("/songs/saved-filters/{id}/chip?renaming=1"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(open.contains(r#"value="Videos""#), "{open}");
        assert!(
            open.contains(&format!("/songs/saved-filters/{id}/rename")),
            "{open}"
        );

        let (_, shut) = get(&state, &format!("/songs/saved-filters/{id}/chip")).await;
        assert!(shut.contains(r#"href="/songs?kind=video""#), "{shut}");
        assert!(!shut.contains(r#"name="saved_name""#), "{shut}");

        assert_eq!(only_saved(&state).name, "Videos", "neither drew a write");
    }

    /// A rename keeps the query, and a name that is taken is refused in words.
    #[tokio::test]
    async fn renaming_a_saved_filter_keeps_its_query_and_refuses_a_collision() {
        let (_corpus, state) = two_folders("saved-rename");
        state.remember_songs_filter("kind=video".to_owned());
        post(&state, "/songs/saved-filters", "saved_name=Videos").await;
        let id = only_saved(&state).id;
        state.remember_songs_filter("language=pt".to_owned());
        post(&state, "/songs/saved-filters", "saved_name=Brasil").await;

        let (status, refused) = post(
            &state,
            &format!("/songs/saved-filters/{id}/rename"),
            "saved_name=Brasil",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(refused.contains("already saved"), "{refused}");
        assert_eq!(saved_named(&state, "Videos").query, "kind=video");

        let (_, said) = post(
            &state,
            &format!("/songs/saved-filters/{id}/rename"),
            "saved_name=Clips",
        )
        .await;
        assert!(said.contains("Renamed to Clips"), "{said}");
        assert!(
            said.contains(r#"id="saved-filters""#) && said.contains("hx-swap-oob"),
            "the strip rides along, because a rename reorders it: {said}"
        );
        assert_eq!(saved_named(&state, "Clips").query, "kind=video");
    }

    // -- a favorite that is a working list -------------------------------------------------------

    /// The box on the Favorites page says what kind of list one is, and the row's star follows.
    ///
    /// **An unticked box sends nothing at all**, which is the whole of how the handler reads it —
    /// the same reading `keep_page` relies on, and the reason there is no hidden `temporary=0`
    /// beside the box: a repeated known key is a 400, so ticking it would fail.
    #[tokio::test]
    async fn marking_a_favorite_a_working_list_takes_the_gold_off_its_songs() {
        let (_corpus, state) = two_folders("working-list");
        let song = a_song(&state);
        let favorite = {
            let workspace = state.workspace().expect("open");
            let db = workspace.db.lock();
            let id = db.create_favorite("to-check").expect("favorite");
            db.set_favorite(&song, id, true).expect("file it");
            id
        };

        let (_, filed) = get(&state, &format!("/songs/{song}/row")).await;
        assert!(filed.contains(r#"class="star filed""#), "{filed}");

        let (status, said) = post(
            &state,
            &format!("/favorites/{favorite}/temporary"),
            "temporary=1",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(said.contains("to-check is a working list"), "{said}");

        let (_, aside) = get(&state, &format!("/songs/{song}/row")).await;
        assert!(aside.contains("&#9733;"), "still in a list: {aside}");
        assert!(
            !aside.contains(r#"class="star filed""#),
            "and filed in none: {aside}"
        );

        // An empty body is what an unticked box sends, and it is the whole of the undo.
        let (_, back) = post(&state, &format!("/favorites/{favorite}/temporary"), "").await;
        assert!(back.contains("is a filing again"), "{back}");
        let (_, refiled) = get(&state, &format!("/songs/{song}/row")).await;
        assert!(refiled.contains(r#"class="star filed""#), "{refiled}");
    }

    /// The Favorites page offers the box, and offers it per list.
    #[tokio::test]
    async fn the_favorites_page_says_which_lists_are_working_lists() {
        let (_corpus, state) = two_folders("working-list-page");
        {
            let workspace = state.workspace().expect("open");
            let db = workspace.db.lock();
            let id = db.create_favorite("to-check").expect("favorite");
            db.create_favorite("Bossa").expect("favorite");
            db.set_favorite_temporary(id, true).expect("set");
        }
        let (status, html) = get(&state, "/favorites").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("Working list"), "{html}");
        assert_eq!(
            html.matches(r#"name="temporary""#).count(),
            2,
            "one box per list: {html}"
        );
        assert_eq!(
            html.matches("checked").count(),
            1,
            "and only the one that is: {html}"
        );
        // The working list comes after the filing, below the rule naming the group.
        let rule = html.find("rule-row").expect("a rule");
        let bossa = html.find(r#"value="Bossa""#).expect("the filing");
        let check = html.find(r#"value="to-check""#).expect("the working list");
        assert!(bossa < rule && rule < check, "{html}");
    }

    /// A page of filings alone draws no rule, since there is nothing below it to separate.
    #[tokio::test]
    async fn the_favorites_page_draws_no_rule_without_a_working_list() {
        let (_corpus, state) = two_folders("working-list-no-rule");
        state
            .workspace()
            .expect("open")
            .db
            .lock()
            .create_favorite("Bossa")
            .expect("favorite");
        let (_, html) = get(&state, "/favorites").await;
        assert!(!html.contains("rule-row"), "{html}");
    }

    // -- the song page's tabs --------------------------------------------------------------------

    /// Every tab on the song page has the four parts a tab is made of, and they agree about its
    /// name.
    ///
    /// A radio, a label pointing at it, a pane, and the stylesheet's arm naming all three.
    /// `.songtabs .pane { display: none }` is unconditional, so a name that agrees in three of the
    /// four is a tab that draws itself, takes the click and shows nothing at all.
    ///
    /// The stylesheet is asserted against the shipped file, as the Songs page's own tab test does:
    /// one arm per tab has to be written out because a selector cannot ask which radio is checked
    /// without naming it, and none of those places is reachable from a template.
    #[tokio::test]
    async fn every_song_page_tab_has_a_radio_a_label_a_pane_and_a_rule() {
        let (_corpus, state) = two_folders("song-tabs");
        let (status, html) = get(&state, &format!("/songs/{}", a_song(&state))).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let css = include_str!("../static/style.css");
        for tab in ["details", "files", "filing", "lyrics"] {
            assert!(
                html.contains(&format!(r#"id="song-tab-{tab}""#)),
                "no radio for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"for="song-tab-{tab}""#)),
                "no label for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"class="pane {tab}""#)),
                "no pane for the {tab} tab:\n{html}"
            );
            assert!(
                css.contains(&format!("#song-tab-{tab}:checked ~ .pane.{tab}")),
                "nothing shows the {tab} pane, so the tab is a label over an empty page"
            );
            assert!(
                css.contains(&format!(
                    r#"#song-tab-{tab}:checked ~ .tabstrip label[for="song-tab-{tab}"]"#
                )),
                "the {tab} tab is not underlined when it is the open one"
            );
            assert!(
                css.contains(&format!(
                    r#"#song-tab-{tab}:focus-visible ~ .tabstrip label[for="song-tab-{tab}"]"#
                )),
                "the {tab} tab takes no focus ring, and its radio is hidden"
            );
        }
    }

    /// Every radio comes before every pane, because `~` only reaches forward.
    ///
    /// The rule that shows a pane is `#song-tab-x:checked ~ .pane.x`, and the general sibling
    /// combinator matches nothing preceding the element it starts from. So moving each input inside
    /// its own label in the strip breaks this silently and completely: every pane would be
    /// `display: none` with nothing able to turn one back on.
    #[tokio::test]
    async fn the_song_page_radios_come_before_every_pane() {
        let (_corpus, state) = two_folders("song-tab-order");
        let (_, html) = get(&state, &format!("/songs/{}", a_song(&state))).await;
        let last_radio = html.rfind(r#"name="song-tab""#).expect("a radio");
        let first_pane = html.find(r#"class="pane "#).expect("a pane");
        assert!(
            last_radio < first_pane,
            "a pane before the last radio is a pane nothing can show"
        );
    }

    // -- the settings page's tabs ------------------------------------------------------------------

    /// Every tab on the settings page has the four parts a tab is made of, and its rule.
    ///
    /// The same assertion as the song page's, over the same `.songtabs` arrangement: a name that
    /// agrees in three of the four is a tab that draws itself, takes the click and shows nothing,
    /// because `.songtabs .pane { display: none }` is unconditional.
    #[tokio::test]
    async fn every_settings_tab_has_a_radio_a_label_a_pane_and_a_rule() {
        let (_corpus, state) = two_folders("settings-tabs");
        let (status, html) = get(&state, "/settings").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let css = include_str!("../static/style.css");
        for tab in ["machine", "backup", "locale", "tags", "folder"] {
            assert!(
                html.contains(&format!(r#"id="settings-tab-{tab}""#)),
                "no radio for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"for="settings-tab-{tab}""#)),
                "no label for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"class="pane {tab}""#)),
                "no pane for the {tab} tab:\n{html}"
            );
            assert!(
                css.contains(&format!("#settings-tab-{tab}:checked ~ .pane.{tab}")),
                "nothing shows the {tab} pane, so the tab is a label over an empty page"
            );
            assert!(
                css.contains(&format!(
                    r#"#settings-tab-{tab}:checked ~ .tabstrip label[for="settings-tab-{tab}"]"#
                )),
                "the {tab} tab is not underlined when it is the open one"
            );
            assert!(
                css.contains(&format!(
                    r#"#settings-tab-{tab}:focus-visible ~ .tabstrip label[for="settings-tab-{tab}"]"#
                )),
                "the {tab} tab takes no focus ring, and its radio is hidden"
            );
        }
    }

    /// Every radio comes before every pane, and the slot four of them answer into is before them all.
    ///
    /// `~` reaches forward only, so a pane above the last radio is a pane nothing can show. The slot
    /// is the other half: four of these five panels swap `#action-result`, and one inside a pane is
    /// a button that appears to do nothing whenever another tab is open.
    #[tokio::test]
    async fn the_settings_radios_come_before_every_pane_and_the_slot_before_them() {
        let (_corpus, state) = two_folders("settings-tab-order");
        let (_, html) = get(&state, "/settings").await;
        let slot = html.find(r#"id="action-result""#).expect("a slot");
        let first_radio = html.find(r#"name="settings-tab""#).expect("a radio");
        let last_radio = html.rfind(r#"name="settings-tab""#).expect("a radio");
        let first_pane = html.find(r#"class="pane "#).expect("a pane");
        assert!(
            last_radio < first_pane,
            "a pane before the last radio is a pane nothing can show"
        );
        assert!(
            slot < first_radio,
            "what a button answers with is inside a tab, so it shows only while that tab is open"
        );
    }

    /// What answers a button has to land somewhere that is on the screen.
    ///
    /// Almost every control on this page targets `#action-result`, and a slot inside a pane would be
    /// a button that appeared to do nothing whenever a different tab was open. The banners above it
    /// are outside for the same reason: a song hidden behind a merge has to say so however the page
    /// was left.
    #[tokio::test]
    async fn the_song_pages_answer_slot_is_outside_every_tab() {
        let (_corpus, state) = two_folders("song-tab-result");
        let (_, html) = get(&state, &format!("/songs/{}", a_song(&state))).await;
        let slot = html.find(r#"id="action-result""#).expect("the slot");
        let tabs = html.find(r#"class="songtabs""#).expect("the tabs");
        assert!(slot < tabs, "the slot is above the tabs:\n{html}");
    }

    /// The Files tab is drawn for a song nothing else looks like.
    ///
    /// A tab is drawn whether or not it has anything to offer, which is the rule the Songs page's
    /// own tabs already keep — a box that vanishes leaves a tab that is sometimes one card wide and
    /// sometimes two.
    #[tokio::test]
    async fn other_versions_is_drawn_for_a_song_that_has_none() {
        let (_corpus, state) = two_folders("song-tab-versions");
        let (_, html) = get(&state, &format!("/songs/{}", a_song(&state))).await;
        assert!(html.contains("Other versions"), "{html}");
        assert!(
            html.contains("No other file here looks like this recording"),
            "{html}"
        );
    }

    /// The first song the corpus fixture holds.
    fn a_song(state: &State) -> String {
        let workspace = state.workspace().expect("open");
        let db = workspace.db.lock();
        db.songs_page(&crate::db::Filter::default())
            .expect("rows")
            .0
            .first()
            .expect("a song")
            .id
            .clone()
    }

    /// A rating set from the song page answers in a toast, and the page asks for one.
    #[tokio::test]
    async fn a_rating_from_the_song_page_is_a_toast() {
        let (_corpus, state) = two_folders("rating-toast");
        let id = a_song(&state);
        let (_, html) = get(&state, &format!("/songs/{id}")).await;
        assert!(html.contains("/user-score?as=toast"), "{html}");

        let (status, said) = post(
            &state,
            &format!("/songs/{id}/user-score?as=toast"),
            "score=9",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{said}");
        assert!(
            said.contains("hx-swap-oob=\"afterbegin\"") && said.contains("toast-good"),
            "{said}"
        );
        assert!(said.contains("Rated 9/10."), "{said}");
    }

    /// The three answers about a song's words, and the column each one leaves behind.
    ///
    /// **`show` storing a 0 rather than a NULL is the assertion that matters.** NULL is nobody
    /// having said, and a build reads it as *take whatever the analysis concludes* — so a person
    /// who has just overruled the analysis and a person who has never opened the page would leave
    /// the same row, and the next build would undo the first one's answer.
    #[tokio::test]
    async fn the_three_answers_about_a_songs_words_store_three_different_rows() {
        let (_corpus, state) = two_folders("lyrics-hidden-states");
        let id = a_song(&state);

        let stored = |state: &State, id: &str| -> Option<bool> {
            let workspace = state.workspace().expect("open");
            let db = workspace.db.lock();
            db.song(id).expect("song").lyrics_hidden
        };
        assert_eq!(stored(&state, &id), None, "nobody has said yet");

        for (posted, expected) in [
            ("lyrics_hidden=hide", Some(true)),
            ("lyrics_hidden=show", Some(false)),
            ("lyrics_hidden=auto", None),
        ] {
            let (status, said) = post(
                &state,
                &format!("/songs/{id}/lyrics-hidden?as=toast"),
                posted,
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK, "{said}");
            assert_eq!(stored(&state, &id), expected, "after {posted}");
        }
    }

    /// The Advanced tab is offered for every song whose words the machine draws, and no others.
    #[tokio::test]
    async fn the_advanced_tab_follows_the_words_rather_than_the_channels() {
        let (_corpus, state) = two_folders("advanced-tab-words");
        let id = a_song(&state);
        let (_, html) = get(&state, &format!("/songs/{id}")).await;
        assert!(
            html.contains("song-tab-advanced\">") || html.contains("for=\"song-tab-advanced\""),
            "a MIDI song is offered the tab: {html}"
        );
        assert!(
            html.contains("/lyrics-hidden?as=toast"),
            "and the words control is on it: {html}"
        );
    }

    /// Corrections are written by their own route, and the details form does not touch them.
    ///
    /// **Two forms on two tabs write one column, and neither may write the other's fields.** The
    /// details form sends the title, the artist and the language from what came back, so a save from
    /// the channel table posting into it would clear all three; a route of its own is what keeps
    /// them apart, and this is the test that says so.
    #[tokio::test]
    async fn corrections_and_the_details_form_do_not_write_over_each_other() {
        let (_corpus, state) = two_folders("corrections-route");
        let id = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .songs(&crate::db::Filter::default())
            .expect("browse")
            .into_iter()
            .map(|row| row.id)
            .next()
            .expect("a song");

        let (status, _) = post(
            &state,
            &format!("/songs/{id}/edit"),
            "title=Chosen&artist=&language=&transpose=&notes=",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);

        let (status, said) = post(
            &state,
            &format!("/songs/{id}/corrections"),
            "fix=mute_channel:2&fix=force_program:0:52",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{said}");
        assert!(
            said.contains("hx-swap-oob=\"afterbegin\"") && said.contains("toast-good"),
            "a saved list is a toast: {said}"
        );

        let detail = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .song(&id)
            .expect("the song");
        assert_eq!(detail.title.as_deref(), Some("Chosen"), "the title stands");
        assert_eq!(
            crate::fixes::stored(detail.fixes.as_deref()),
            Some(vec![
                km_fixes::Fix::ForceProgram {
                    channel: 0,
                    program: 52
                },
                km_fixes::Fix::MuteChannel { channel: 2 },
            ])
        );

        // And the details form leaves the corrections alone on its way past.
        post(
            &state,
            &format!("/songs/{id}/edit"),
            "title=Chosen&artist=Queen&language=&transpose=&notes=",
        )
        .await;
        let detail = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .song(&id)
            .expect("the song");
        assert_eq!(detail.artist.as_deref(), Some("Queen"));
        assert!(
            crate::fixes::stored(detail.fixes.as_deref()).is_some_and(|fixes| fixes.len() == 2),
            "the corrections are untouched: {:?}",
            detail.fixes
        );
    }

    /// The one saved filter a test has made.
    fn only_saved(state: &State) -> crate::model::SavedFilter {
        let held = state
            .workspace()
            .expect("open")
            .db
            .lock()
            .saved_filters()
            .expect("read");
        assert_eq!(held.len(), 1, "one saved filter");
        held.into_iter().next().expect("saved")
    }

    /// The saved filter under a name, for a test that has made more than one.
    fn saved_named(state: &State, name: &str) -> crate::model::SavedFilter {
        state
            .workspace()
            .expect("open")
            .db
            .lock()
            .saved_filter_named(name)
            .expect("read")
            .expect("saved")
    }

    /// Taking a title from a file name redraws the rows where they were, and writes that down.
    ///
    /// The action changes no filter, so page three still means page three; the offset reaches the
    /// body because `static/ui.js` copies it off `#rows`. The write-down is the other half — the
    /// nav's Songs link and a filter saved next both read it, so a redraw that did not say where it
    /// landed would send them to a page nobody is on.
    #[tokio::test]
    async fn a_title_from_a_file_name_stays_on_the_page_it_was_pressed_on() {
        let (_corpus, state) = a_corpus_of("titles-keep-the-page", 120);
        state.remember_songs_filter("offset=50".to_owned());

        let (status, html) = post(
            &state,
            "/songs/titles-from-filename",
            "offset=50&song_id=song-0050",
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("Took the title of 1 song"),
            "the write happened: {html}"
        );
        assert!(
            html.contains(r#"data-offset="50""#),
            "and the rows came back on the page the button was pressed on: {html}"
        );
        assert_eq!(
            state.songs_filter(),
            "offset=50",
            "and the record says the page being shown"
        );
    }

    /// Fixing the capitals answers with the rows, on the page the button was pressed on.
    ///
    /// The other Titles action's rule, asserted through the second route that now has to keep it.
    /// The two share `redraw_over_the_write`, and this is what would fail if one of them stopped.
    #[tokio::test]
    async fn fixing_the_capitals_stays_on_the_page_it_was_pressed_on() {
        let (_corpus, state) = a_shouting_corpus("capitals-keep-the-page", 120);
        state.remember_songs_filter("offset=50".to_owned());

        let (status, html) = post(
            &state,
            "/songs/fix-name-case",
            "offset=50&song_id=song-0050",
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("Fixed the capitals of 1 song."),
            "the write happened: {html}"
        );
        assert!(
            html.contains(r#"data-offset="50""#),
            "and the rows came back where they were: {html}"
        );
        assert_eq!(state.songs_filter(), "offset=50");
    }

    /// The Titles actions on the similar-names page answer with its matches, redrawn for the search
    /// in its bar, and leave the Songs page's filter alone.
    #[tokio::test]
    async fn the_titles_actions_redraw_the_similar_names_matches() {
        let (_corpus, state) = a_shouting_corpus("capitals-similar", 3);
        state.remember_songs_filter("offset=50".to_owned());

        let (status, page) = get(
            &state,
            "/similar?title=CANCAO%20NUMERO%200000&from=song-0000",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        for route in [
            "/songs/fix-name-case?as=hits",
            "/songs/titles-from-filename?as=hits",
            "/songs/split-artist-from-title?as=hits",
        ] {
            assert!(page.contains(&format!("hx-post=\"{route}\"")), "{page}");
        }

        // The bar comes first; a row open for renaming would send a `title` of its own after it.
        let bar = "title=CANCAO+NUMERO+0000&artist=&from=song-0000&suitability=&kind=&granularity=&copies=";
        let (status, html) = post(
            &state,
            "/songs/fix-name-case?as=hits",
            &format!("{bar}&song_id=song-0001&title=elsewhere"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("Fixed the capitals of 1 song."), "{html}");
        assert!(
            html.contains("id=\"hits\""),
            "the matches came back: {html}"
        );
        assert!(
            html.contains("Cancao Numero 0001"),
            "with the new name: {html}"
        );
        assert!(html.contains("id=\"row-song-0000\""), "{html}");
        assert!(
            !html.contains("data-offset"),
            "and not the Songs rows: {html}"
        );
        assert_eq!(state.songs_filter(), "offset=50");

        let (status, html) = post(
            &state,
            "/songs/titles-from-filename?as=hits",
            &format!("{bar}&song_id=song-0002"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("Took the title of 1 song"), "{html}");
        assert!(html.contains("id=\"hits\""), "{html}");
    }

    /// A name that already has capitals of its own is reported, not silently dropped.
    ///
    /// The count short of the number ticked is the whole of what a person needs to know here: a
    /// button that said "Fixed the capitals of 1 song" over three ticked rows reads as two failures.
    #[tokio::test]
    async fn fixing_the_capitals_says_how_many_already_had_them() {
        let (_corpus, state) = a_shouting_corpus("capitals-already-decided", 3);
        let (status, html) = post(
            &state,
            "/songs/fix-name-case",
            "song_id=song-0000&song_id=decided",
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("1 already had capitals of their own"),
            "{html}"
        );
    }

    /// Ticking nothing is refused by every Titles action, and by name.
    #[tokio::test]
    async fn fixing_the_capitals_with_nothing_ticked_says_so() {
        let (_corpus, state) = a_shouting_corpus("capitals-nothing-ticked", 1);
        let (_, html) = post(&state, "/songs/fix-name-case", "").await;
        assert!(html.contains("Nothing was ticked."), "{html}");
    }

    /// A corpus that shouts, plus one song somebody has already cased.
    ///
    /// [`a_corpus_of`] writes `Song 0000`, which is already an answer and which *Fix the capitals*
    /// therefore leaves alone — so a test about this button needs names with the defect in them.
    fn a_shouting_corpus(name: &str, songs: u32) -> (Scratch, State) {
        let corpus = Scratch::new(name);
        let mut db = Db::open_in_memory(&corpus.0).expect("open");
        for number in 0..songs {
            crate::db::tests::add(
                &mut db,
                &format!("song-{number:04}"),
                Some(&format!("CANCAO NUMERO {number:04}")),
                &format!("folder/SONG{number:04}.kar"),
            );
        }
        crate::db::tests::add(&mut db, "decided", Some("Tom Jobim"), "folder/tj.kar");
        (corpus, State::new(db))
    }

    /// A corpus where two files sing one song under names that share nothing, and one sings another.
    ///
    /// [`a_corpus_of`] gives every song a lyric of its own, which is what the Lyrics page wants and
    /// the opposite of what this one does.
    fn a_singing_corpus(name: &str) -> (Scratch, State) {
        let corpus = Scratch::new(name);
        let mut db = Db::open_in_memory(&corpus.0).expect("open");
        for (id, title, lyrics) in [
            (
                "song-0000",
                "Dancing In The Dark",
                crate::db::tests::verses(0, 5),
            ),
            ("song-0001", "EARTHW~2", crate::db::tests::verses(0, 5)),
            (
                "song-0002",
                "Something Else",
                crate::db::tests::verses(20, 25),
            ),
        ] {
            crate::db::tests::add_scanned(&mut db, id, |song| {
                song.det_title = Some(title.to_owned());
                song.lyrics = Some(lyrics);
            });
        }
        crate::db::tests::add_scanned(&mut db, "instrumental", |song| {
            song.det_title = Some("No Words At All".to_owned());
            song.lyrics = None;
        });
        (corpus, State::new(db))
    }

    /// A row leads to the songs that sing what it sings, and a file no name could reach is there.
    #[tokio::test]
    async fn a_row_leads_to_the_songs_that_sing_the_same_words() {
        let (_corpus, state) = a_singing_corpus("words-from-a-row");

        let (status, rows) = get(&state, "/songs").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            rows.contains("/similar-words?from=song-0000"),
            "a song with words carries the button: {rows}"
        );

        let (status, html) = get(&state, "/similar-words?from=song-0000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains(r#"id="row-song-0001""#),
            "the file under an unrelated name is a match: {html}"
        );
        assert!(
            !html.contains(r#"id="row-song-0002""#),
            "and the song that sings something else is not: {html}"
        );
        assert!(html.contains("searched-from"), "{html}");
    }

    /// The fragment route answers with the matches alone, as the bar asks for them.
    #[tokio::test]
    async fn the_same_words_hits_are_a_fragment() {
        let (_corpus, state) = a_singing_corpus("words-fragment");

        let (status, hits) = get(&state, "/similar-words/hits?from=song-0000&suitability=").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(hits.contains(r#"<div id="hits">"#), "{hits}");
        assert!(!hits.contains("<html"), "a fragment and not a page: {hits}");
    }

    /// The two matching bars are remembered apart, and the same-words one opens at every band.
    ///
    /// **The defect this pins is that the page would find nothing.** The files it exists to reach are
    /// the ones no name could — a rough transcription under a name like `EARTHW~2` — and those score
    /// under 8. One shared record, opening where the names page opens, would have hidden them behind
    /// the band and said no other song sings these words.
    #[tokio::test]
    async fn the_two_matching_bars_are_remembered_apart() {
        let (_corpus, state) = a_singing_corpus("words-own-bar");

        // A bare link to either page, which carries no narrowing and takes what each remembers.
        let (status, _) = get(&state, "/similar-words?from=song-0000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(state.words_narrowing(), SimilarNarrowing::for_words());
        assert_eq!(
            state.similar_narrowing(),
            SimilarNarrowing::default(),
            "and the names page's own record is untouched"
        );

        // The same-words bar speaking narrows only itself.
        let (status, _) = get(
            &state,
            "/similar-words/hits?from=song-0000&suitability=0-4&kind=&granularity=&copies=",
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(state.words_narrowing().suitability, "0-4");
        assert_eq!(
            state.similar_narrowing(),
            SimilarNarrowing::default(),
            "the bar next door did not move"
        );
    }

    /// A song with no words draws no button, and its page says so rather than reporting no match.
    ///
    /// Two different things to be told, because they have two different answers: one is *this file
    /// has nothing to compare*, the other is *your corpus holds nothing like it*.
    #[tokio::test]
    async fn a_song_with_no_words_says_so_rather_than_finding_nothing() {
        let (_corpus, state) = a_singing_corpus("words-instrumental");

        let (status, rows) = get(&state, "/songs").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            !rows.contains("/similar-words?from=instrumental"),
            "an instrumental carries no button: {rows}"
        );

        let (status, html) = get(&state, "/similar-words?from=instrumental").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let words = crate::words::messages(km_locale::Locale::English);
        assert!(html.contains(&*words.msg("words-too-few")), "{html}");
        assert!(!html.contains(&*words.msg("words-not-found")), "{html}");
    }

    /// A song nothing else sings keeps its own row, under the sentence that says so.
    ///
    /// **The defect this pins is a page that looks broken.** The page answers about one song, and a
    /// sentence with no row under it leaves the reader without the song they asked about.
    #[tokio::test]
    async fn a_song_nothing_else_sings_keeps_its_own_row() {
        let (_corpus, state) = a_singing_corpus("words-no-match");

        let (status, html) = get(&state, "/similar-words?from=song-0002").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let words = crate::words::messages(km_locale::Locale::English);
        assert!(html.contains(&*words.msg("words-not-found")), "{html}");
        assert!(
            html.contains(r#"id="row-song-0002""#),
            "the song searched from is there: {html}"
        );
        assert!(html.contains("searched-from"), "{html}");
        assert!(
            !html.contains(r#"id="row-song-0000""#),
            "and nothing else is: {html}"
        );
    }

    /// Taking the artist out of the title answers with the rows, on the page it was pressed on.
    ///
    /// The third Titles action's half of the rule the other two hold, asserted through the route
    /// that now has to keep it as well.
    #[tokio::test]
    async fn splitting_the_artist_stays_on_the_page_it_was_pressed_on() {
        let (_corpus, state) = a_corpus_with_seams("split-keep-the-page", 120);
        state.remember_songs_filter("offset=50".to_owned());

        let (status, html) = post(
            &state,
            "/songs/split-artist-from-title",
            "offset=50&song_id=song-0050",
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("Took the artist out of the title of 1 song."),
            "the write happened: {html}"
        );
        assert!(
            html.contains(r#"data-offset="50""#),
            "and the rows came back where they were: {html}"
        );
        assert_eq!(state.songs_filter(), "offset=50");
    }

    /// A row that names an artist already is reported, not silently dropped.
    ///
    /// The count short of the number ticked is rows there was nothing to take out of, which is what
    /// the sentence says — for the reason the capitals button's does.
    #[tokio::test]
    async fn splitting_the_artist_says_how_many_had_nothing_to_split() {
        let (_corpus, state) = a_corpus_with_seams("split-nothing-to-split", 3);

        let (status, html) = post(
            &state,
            "/songs/split-artist-from-title",
            "song_id=song-0000&song_id=credited&song_id=whole",
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains("Took the artist out of the title of 1 song; 2 named an artist already"),
            "{html}"
        );
    }

    /// Ticking nothing is refused by this action too, and by name.
    #[tokio::test]
    async fn splitting_the_artist_with_nothing_ticked_says_so() {
        let (_corpus, state) = a_corpus_with_seams("split-nothing-ticked", 1);
        let (_, html) = post(&state, "/songs/split-artist-from-title", "").await;
        assert!(html.contains("Nothing was ticked."), "{html}");
    }

    /// A corpus filing the artist inside the title, plus a row that names one and a title with no
    /// seam.
    ///
    /// [`a_corpus_of`] and [`a_shouting_corpus`] both write a title holding no `-`, which this
    /// button leaves alone — so a test about it needs names with the defect in them.
    fn a_corpus_with_seams(name: &str, songs: u32) -> (Scratch, State) {
        let corpus = Scratch::new(name);
        let mut db = Db::open_in_memory(&corpus.0).expect("open");
        for number in 0..songs {
            crate::db::tests::add(
                &mut db,
                &format!("song-{number:04}"),
                Some(&format!("Bob Seger-Cancao numero {number:04}")),
                &format!("folder/song{number:04}.kar"),
            );
        }
        crate::db::tests::add_with_artist(
            &mut db,
            "credited",
            "Bob Seger-Mainstreet",
            "Bob Seger",
            "folder/bs.kar",
        );
        crate::db::tests::add(&mut db, "whole", Some("Corcovado"), "folder/c.kar");
        (corpus, State::new(db))
    }

    /// A page the write emptied comes back as the last real one.
    ///
    /// Keeping the page is safe on an action that writes because `rows_for` clamps. A filter that
    /// reads titles or artists can match fewer songs after a retitle than the offset asked for, and
    /// the alternative is an empty table under a working *previous* button. A hundred and twenty
    /// songs and page eleven is the shape of it.
    #[tokio::test]
    async fn a_title_from_a_file_name_lands_on_a_page_that_is_there() {
        let (_corpus, state) = a_corpus_of("titles-clamp-the-page", 120);

        let (status, html) = post(
            &state,
            "/songs/titles-from-filename",
            "offset=500&song_id=song-0000",
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            html.contains(r#"data-offset="100""#),
            "the last page, not an empty one: {html}"
        );
        assert_eq!(
            state.songs_filter(),
            "offset=100",
            "and what is written down is the page being shown"
        );
    }

    /// A corpus of `songs` rows, written straight in rather than scanned.
    ///
    /// A page is fifty rows, so anything about paging needs more distinct songs than there are
    /// fixtures to build them out of — and what a page turn does is the same whether the row came
    /// from a MIDI file or from `write_scanned`.
    fn a_corpus_of(name: &str, songs: u32) -> (Scratch, State) {
        let corpus = Scratch::new(name);
        let mut db = Db::open_in_memory(&corpus.0).expect("open");
        for number in 0..songs {
            crate::db::tests::add(
                &mut db,
                &format!("song-{number:04}"),
                Some(&format!("Song {number:04}")),
                &format!("folder/SONG{number:04}.kar"),
            );
        }
        (corpus, State::new(db))
    }

    /// The page somebody is on is written down along with the filter.
    #[tokio::test]
    async fn the_page_travels_with_the_filter() {
        let (_corpus, state) = a_corpus_of("page-travels", 120);

        let (status, _) = get(&state, "/songs?sort=title&offset=50").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(
            state.songs_filter(),
            "sort=title&offset=50&total=120",
            "a page turn is part of where somebody was"
        );

        // The route the paging buttons themselves call, which is the ordinary way a page changes.
        let (status, _) = get(&state, "/songs/rows?sort=title&offset=100&total=120").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(
            state.songs_filter(),
            "sort=title&offset=100",
            "and `total` is a count this render carried, not part of what was asked for"
        );
    }

    /// A page above the end of the corpus draws the last page rather than nothing.
    ///
    /// A page number outlives the run that set it, so it comes back to a corpus a scan may have
    /// taken rows out of. Sixty songs and page twenty-one is the shape of that, and of a bookmark.
    #[tokio::test]
    async fn a_page_above_the_end_comes_back_as_the_last_page() {
        let (_corpus, state) = a_corpus_of("page-past-the-end", 60);

        let (status, html) = get(&state, "/songs?offset=1000").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("page 2 of 2 (60 songs)"), "{html}");
        assert_eq!(
            state.songs_filter(),
            "offset=50&total=60",
            "and what is written down is the page being shown"
        );
    }

    // -- a page is drawn while the corpus is being written to ------------------------------------

    /// **The reported fault, as a test.** A scan holds the writing connection for the length of a
    /// batch, and a page drawn through that same connection had to win a gap between batches — which
    /// over a whole corpus is a wait with no bound on it, spent with a browser socket held open.
    ///
    /// A held lock stands in for the batch. On a database that took WAL the page is drawn through a
    /// connection of its own and answers regardless; before that it waited here for ever.
    ///
    /// The holder is a thread rather than a guard taken in this function, so that no lock is held
    /// across an `await`.
    #[tokio::test]
    async fn a_page_is_drawn_while_the_writing_connection_is_held() {
        let corpus = Scratch::new("page-while-writing");
        let mut db = Db::create(&corpus.0).expect("create");
        crate::db::tests::add(&mut db, "song-0000", Some("Cabeça"), "folder/SONG.kar");
        let state = State::new(db);

        let workspace = state.workspace().expect("a folder is open");
        assert!(
            !Arc::ptr_eq(workspace.reader(), &workspace.db),
            "the test is meaningless without a second connection"
        );

        let (say_held, held) = std::sync::mpsc::channel::<()>();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let writing = Arc::clone(&workspace.db);
        let holder = std::thread::spawn(move || {
            let _guard = writing.lock();
            say_held.send(()).expect("say it is held");
            let _ = released.recv();
        });
        held.recv().expect("the writing connection is held");

        let (status, html) = get(&state, "/songs?").await;

        release.send(()).expect("let the connection go");
        holder.join().expect("the holder finished");

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(html.contains("Cabeça"), "{html}");
    }

    /// A request gives the connection a while and then says it could not have it.
    ///
    /// Over [`crate::db::Shared`] directly, with a deadline a test can afford: what [`WRITE_WAIT`]
    /// should be is a judgement about a person's patience, and pinning the mechanism does not need
    /// to sit through it.
    #[test]
    fn a_wait_for_the_writing_connection_gives_up_at_its_deadline() {
        let db = crate::db::Shared::new(Db::open_in_memory(Path::new("/corpus")).expect("memory"));

        assert!(
            db.lock_within(Duration::from_millis(50)).is_some(),
            "an idle connection is handed over at once"
        );
        assert!(!db.wanted(), "and nobody is left counted as waiting");

        let guard = db.lock();
        let started = Instant::now();
        assert!(
            db.lock_within(Duration::from_millis(50)).is_none(),
            "a held connection is given up on rather than waited for"
        );
        assert!(
            started.elapsed() >= Duration::from_millis(50),
            "and it is given the whole deadline first"
        );
        drop(guard);
        assert!(
            !db.wanted(),
            "a wait that gave up stops being counted, or a scan would stand aside for ever"
        );
    }

    /// A waiter is visible to whoever is holding the connection, which is what a scan asks.
    #[test]
    fn a_request_waiting_for_the_writing_connection_says_so() {
        let db = Arc::new(crate::db::Shared::new(
            Db::open_in_memory(Path::new("/corpus")).expect("memory"),
        ));
        let guard = db.lock();
        assert!(!db.wanted(), "nothing is waiting yet");

        let waiting = Arc::clone(&db);
        let waiter = std::thread::spawn(move || waiting.lock_within(WRITE_WAIT).is_some());

        // The waiter polls, so it has to be given a moment to enter the queue before it is asked
        // about — the alternative is asserting on a race.
        let mut seen = false;
        for _ in 0..100 {
            if db.wanted() {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            seen,
            "a request waiting on the connection has to be visible"
        );

        drop(guard);
        assert!(waiter.join().expect("the waiter finished"), "and gets in");
        assert!(!db.wanted(), "and stops being counted once it has");
    }

    // -- a refused navigation is a page and a refused fragment is a sentence ----------------------

    /// **The reported fault, as a test.** A navigation that is refused comes back with the nav on it.
    ///
    /// The window this tool draws is a webview: no address bar, no Back, no reload. Answered with a
    /// sentence, a refused navigation left it holding one line of text, and closing the window was
    /// the only way out of it. A missing song is the cheap refusal to ask for; what is asserted is
    /// the way out, which every refusal shares.
    #[tokio::test]
    async fn a_refused_navigation_comes_back_as_a_page_with_the_nav_on_it() {
        let (_corpus, state) = a_corpus_of("refused-navigation", 1);

        let (status, html) = get(&state, "/songs/no-such-song").await;

        assert_eq!(status, axum::http::StatusCode::NOT_FOUND, "{html}");
        assert!(
            html.contains("<nav>") && html.contains("href=\"/scan\""),
            "a refused page carries the way to every other one: {html}"
        );
        assert!(
            html.contains("error-try-again") || html.contains("Try again"),
            "and something to press: {html}"
        );
        assert!(
            !html.contains("http-equiv=\"refresh\""),
            "a song that is not there will not be there in fifteen seconds either: {html}"
        );
    }

    /// A page asked for while the corpus is being written to says so, and offers the way out.
    ///
    /// **An in-memory database is the case that reaches this**, and it is not a contrivance: a
    /// folder only gets a reading connection where its database took write-ahead logging, and
    /// `Workspace::new` falls back to the writing one where it did not. On that fallback every page
    /// queues behind the scan's batch, which is the state the fault was reported from.
    ///
    /// **The navigation and the fragment are asked at once**, which is the contrast this change is
    /// about and costs one `READ_WAIT` rather than two: `/songs` is a page somebody went to, and
    /// `/songs/rows` is the same refusal arriving at a page that is still on the screen.
    ///
    /// The holder is a thread, so no lock is held across an `await`. It sits through `READ_WAIT`,
    /// which is what the person reporting this sat through.
    #[tokio::test]
    async fn a_navigation_while_the_corpus_is_written_to_offers_the_way_out() {
        let (_corpus, state) = a_corpus_of("busy-navigation", 1);

        let workspace = state.workspace().expect("a folder is open");
        assert!(
            Arc::ptr_eq(workspace.reader(), &workspace.db),
            "the test needs the one-connection fallback, which is what an in-memory database takes"
        );

        let (say_held, held) = std::sync::mpsc::channel::<()>();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let writing = Arc::clone(&workspace.db);
        let holder = std::thread::spawn(move || {
            let _guard = writing.lock();
            say_held.send(()).expect("say it is held");
            let _ = released.recv();
        });
        held.recv().expect("the writing connection is held");

        let ((status, html), (fragment_status, fragment)) =
            tokio::join!(get(&state, "/songs"), get(&state, "/songs/rows?"));

        release.send(()).expect("let the connection go");
        holder.join().expect("the holder finished");

        assert_eq!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "the status goes on saying the corpus was not read: {html}"
        );
        assert!(
            html.contains("<nav>") && html.contains("href=\"/scan\""),
            "and the page carries the way back to the scan that is holding it: {html}"
        );
        assert!(
            html.contains("http-equiv=\"refresh\""),
            "a busy corpus is the one refusal that asks again by itself: {html}"
        );
        assert!(
            !html.contains("class=\"counts\""),
            "a header that could not count says nothing rather than four zeroes: {html}"
        );

        // **The half that must not change.** htmx will not swap a failed response, so the page the
        // rows are already on stays as it is and `static/ui.js` puts the reason over it. A page sent
        // back here would be a whole document handed to an element expecting table rows.
        assert_eq!(
            fragment_status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "{fragment}"
        );
        assert!(
            !fragment.contains("<nav>"),
            "a fragment is answered with the sentence, not with a page: {fragment}"
        );
    }

    /// `--machine` is what the tool talks to for the run, and the workspace keeps what it was told.
    ///
    /// A test run pointed at a machine of its own must not replace the address somebody normally
    /// uses, which is the whole reason the pin exists.
    #[tokio::test]
    async fn a_pinned_machine_is_used_and_not_written_down() {
        let (_corpus, state) = two_folders("pinned-not-written");
        state
            .blocking(|db| crate::chosen::save(db, "http://192.168.1.42:8177"))
            .await
            .expect("save");

        state.pin_machine("http://127.0.0.1:8277");
        assert_eq!(
            state
                .chosen_machine()
                .await
                .map(|known| known.url)
                .as_deref(),
            Some("http://127.0.0.1:8277")
        );
        assert_eq!(
            state.machine_shown().await.map(|(url, _)| url).as_deref(),
            Some("http://127.0.0.1:8277")
        );

        state.machine_answered("abc123", "Test").await;
        assert_eq!(state.machine_id().await.as_deref(), Some("abc123"));

        let kept = state
            .reading(crate::chosen::load)
            .await
            .expect("load")
            .expect("the record is still there");
        assert_eq!(kept.url, "http://192.168.1.42:8177");
        assert_eq!(kept.id, None, "the pin's answer reached the database");
    }

    /// Saving the form on the pinned address writes nothing; saving another one is a real choice.
    #[tokio::test]
    async fn the_settings_form_unpins_only_for_another_address() {
        use axum::extract::State as AxumState;

        let (_corpus, state) = two_folders("pinned-settings");
        state.pin_machine("http://127.0.0.1:9");

        let _ = crate::handlers::save_settings(
            AxumState(state.clone()),
            "machine=http%3A%2F%2F127.0.0.1%3A9".to_owned(),
        )
        .await;
        assert!(
            state.pinned_machine().is_some(),
            "the same address unpinned"
        );
        assert_eq!(
            state.reading(crate::chosen::load).await.expect("load"),
            None
        );

        let _ = crate::handlers::save_settings(
            AxumState(state.clone()),
            "machine=http%3A%2F%2F127.0.0.1%3A19".to_owned(),
        )
        .await;
        assert_eq!(state.pinned_machine(), None);
        assert_eq!(
            state
                .reading(crate::chosen::load)
                .await
                .expect("load")
                .map(|known| known.url)
                .as_deref(),
            Some("http://127.0.0.1:19")
        );
    }
}
