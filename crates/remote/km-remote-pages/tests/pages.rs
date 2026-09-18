//! The pages, driven through the real router.
//!
//! The unit tests in this crate check one template or one function; these check the thing a phone
//! actually talks to — real routing, real extraction, the guard layer, and the fragment/page
//! downgrade that only exists at the request level.
//!
//! The machine behind them is a stub rather than either real implementation, and deliberately: what
//! is being checked here is the *pages*, and a stub is the only way to ask what happens when the
//! machine refuses, is away, or wants a password without arranging for a machine that does.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use km_api::dto::{
    AddedToQueueDto, NowPlayingDto, OriginDto, QueueDto, QueueEntryDto, SettingsDto,
    SettingsPatchDto, SongDto, StateDto, TransportDto,
};
use km_api::events::Event;
use km_remote_pages::machine::{
    ArtistRow, BrowseQuery, Connect, Connection, Copied, CopyOutcome, Favorites, FolderRow,
    LanguageRow, Machine, MachineStatus, Miss, Offer, PackageRow, Reconciliation, RemoteError,
    Resolution, Scan, Scanned, SongPage, SongRef, Songs, TagRow, Transport, codes,
};
use km_remote_pages::{Capabilities, Remote};
use km_songcode::SongCode;
use tokio::sync::broadcast;
use tower::ServiceExt;

// -- the stub machine ----------------------------------------------------------------------------

fn song(number: u32, title: &str, artist: Option<&str>) -> SongDto {
    SongDto {
        number: SongCode::new(number),
        title: title.to_owned(),
        artist: artist.map(str::to_owned),
        language: Some("pt".to_owned()),
        kind: km_kmpkg::SongKind::Midi,
        duration_ms: 222_000,
        suitability: Some(9),
        melody_available: true,
        default_transpose: 0,
        package_id: "vol1".to_owned(),
        // Distinct per song and derived from the number, so a stub catalog models the one property
        // the real one guarantees: within a package, one hash names one song.
        content_hash: Some(format!("{number:032x}")),
        lyric_preview: Vec::new(),
        tags: Vec::new(),
    }
}

#[derive(Clone, Default)]
struct StubSongs {
    songs: Vec<SongDto>,
}

#[async_trait::async_trait]
impl Songs for StubSongs {
    async fn search(&self, query: &BrowseQuery) -> Result<SongPage, RemoteError> {
        let needle = query.text.clone().unwrap_or_default().to_lowercase();
        let matching: Vec<SongDto> = self
            .songs
            .iter()
            .filter(|song| needle.is_empty() || song.title.to_lowercase().contains(&needle))
            .filter(|song| !query.hidden_packages.contains(&song.package_id))
            // **Any** tag, not every. A stub that ANDed would let a page test pass over an
            // intersection, which is the one property of this filter worth asserting at all — and
            // no tags at all is no tag filter, where a bare `any` is nothing.
            .filter(|song| {
                query.tags.is_empty()
                    || query
                        .tags
                        .iter()
                        .any(|wanted| song.tags.iter().any(|held| held == wanted))
            })
            .cloned()
            .collect();
        let total = matching.len();
        let page: Vec<SongDto> = matching
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect();
        // **Honestly computed rather than hardcoded `false`**, which is what it was: with a
        // four-song corpus no test could reach a second page, so `Load more` was never drawn and
        // nothing checked what its button asks for. `a_deep_anchor_brings_its_pages_back_with_it`
        // reads exactly that.
        let more = query.offset + page.len() < total;
        Ok(SongPage {
            songs: page,
            total: Some(total),
            more,
        })
    }

    async fn song(&self, number: SongCode) -> Result<Option<SongDto>, RemoteError> {
        Ok(self
            .songs
            .iter()
            .find(|song| song.number == number)
            .cloned())
    }

    async fn songs_by_number(&self, numbers: &[SongCode]) -> Result<Vec<SongDto>, RemoteError> {
        Ok(numbers
            .iter()
            .filter_map(|number| {
                self.songs
                    .iter()
                    .find(|song| song.number == *number)
                    .cloned()
            })
            .collect())
    }

    /// The same three rungs the real catalogs walk, in the same order — a stub that resolved by
    /// number alone would let every test of the other two pass without exercising them.
    async fn resolve(&self, refs: &[SongRef]) -> Result<Vec<Resolution>, RemoteError> {
        Ok(refs
            .iter()
            .map(|asked| {
                let by_pair = asked
                    .package_id
                    .as_deref()
                    .zip(asked.content_hash.as_deref());
                let found = by_pair
                    .and_then(|(package, hash)| {
                        self.songs.iter().find(|song| {
                            song.package_id == package && song.content_hash.as_deref() == Some(hash)
                        })
                    })
                    .or_else(|| {
                        asked.content_hash.as_deref().and_then(|hash| {
                            self.songs
                                .iter()
                                .filter(|song| song.content_hash.as_deref() == Some(hash))
                                .min_by_key(|song| song.number)
                        })
                    })
                    .or_else(|| {
                        self.songs.iter().find(|song| {
                            song.number == asked.code
                                && !asked.contradicted_by(song.content_hash.as_deref())
                        })
                    });
                let outcome = match found {
                    Some(song) => Ok(song.clone()),
                    None => Err(match &asked.package_id {
                        Some(package)
                            if !self.songs.iter().any(|song| song.package_id == *package) =>
                        {
                            Miss::PackageAbsent
                        }
                        _ if asked.is_identified() => Miss::RecordingAbsent,
                        _ => Miss::NumberAbsent,
                    }),
                };
                Resolution {
                    asked: asked.clone(),
                    outcome,
                }
            })
            .collect())
    }

    /// Drawn from the songs this stub holds, exactly as [`Self::tags`] below is.
    ///
    /// **It used to answer one hard-coded row**, which was fine while the artists list had no filter
    /// of its own: nothing could narrow it, so nothing could be wrong about which rows came back. The
    /// A–Z picker reaches it now, and a fixture with one artist cannot tell a filter that works from
    /// one that returns everything.
    ///
    /// Grouped on the name and ordered by it, which is what both real implementations do — the
    /// offline mirror and `km_catalog::Library` both `GROUP BY artist ORDER BY MIN(sort_artist)`.
    /// `contains` is a plain case-insensitive match rather than the accent fold those two use: this
    /// is a stub, and folding here would be a second implementation of a rule with one home.
    async fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<ArtistRow>, RemoteError> {
        let needle = contains.map(str::to_lowercase);
        let mut counted: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for song in self.visible(hidden) {
            let Some(artist) = song.artist.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            if let Some(needle) = &needle
                && !artist.to_lowercase().contains(needle)
            {
                continue;
            }
            *counted.entry(artist.to_owned()).or_default() += 1;
        }
        Ok(counted
            .into_iter()
            .map(|(name, songs)| ArtistRow { name, songs })
            .collect())
    }

    /// Two fixed rows while nothing is hidden, which is what the language-picker tests draw. With a
    /// package hidden, drawn from the songs left, so a test can see a language leave the picker.
    async fn languages(&self, hidden: &[String]) -> Result<Vec<LanguageRow>, RemoteError> {
        if hidden.is_empty() {
            return Ok(vec![LanguageRow::new("pt", 2), LanguageRow::new("ja", 1)]);
        }
        let mut counted: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for song in self.visible(hidden) {
            if let Some(language) = &song.language {
                *counted.entry(language.clone()).or_default() += 1;
            }
        }
        Ok(counted
            .into_iter()
            .map(|(code, songs)| LanguageRow::new(code, songs))
            .collect())
    }

    async fn tags(&self, hidden: &[String]) -> Result<Vec<TagRow>, RemoteError> {
        // Drawn from the songs this stub holds rather than hard-coded, so a test that gives it an
        // untagged catalog gets an empty vocabulary and can assert the picker is not drawn.
        let mut counted: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for song in self.visible(hidden) {
            for tag in &song.tags {
                *counted.entry(tag.clone()).or_default() += 1;
            }
        }
        let mut rows: Vec<TagRow> = counted
            .into_iter()
            .map(|(tag, songs)| TagRow { tag, songs })
            .collect();
        rows.sort_by(|a, b| b.songs.cmp(&a.songs).then(a.tag.cmp(&b.tag)));
        Ok(rows)
    }

    /// One row per package the songs come from, named by its id, in order of id.
    async fn packages(&self) -> Result<Vec<PackageRow>, RemoteError> {
        let mut counted: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for song in &self.songs {
            *counted.entry(song.package_id.clone()).or_default() += 1;
        }
        Ok(counted
            .into_iter()
            .map(|(id, songs)| PackageRow {
                name: format!("Volume {id}"),
                id,
                songs,
            })
            .collect())
    }

    async fn count(&self) -> Result<usize, RemoteError> {
        Ok(self.songs.len())
    }
}

impl StubSongs {
    /// The songs whose package is not hidden.
    fn visible<'a>(&'a self, hidden: &'a [String]) -> impl Iterator<Item = &'a SongDto> {
        self.songs
            .iter()
            .filter(move |song| !hidden.contains(&song.package_id))
    }
}

/// A catalog that answers every question with a fault.
///
/// **A distinct thing from a catalog holding nothing**, which is what a phone that has never reached
/// a machine has and which `known_songs` deliberately treats as "keep everything". Folding a fault
/// into that branch would file codes the catalog could have rejected and report success, so the two
/// need telling apart — and telling them apart needs a stub that fails rather than one that is
/// empty.
#[derive(Clone, Default)]
struct BrokenSongs;

#[async_trait::async_trait]
impl Songs for BrokenSongs {
    async fn search(&self, _query: &BrowseQuery) -> Result<SongPage, RemoteError> {
        Err(broken())
    }

    async fn song(&self, _number: SongCode) -> Result<Option<SongDto>, RemoteError> {
        Err(broken())
    }

    async fn songs_by_number(&self, _numbers: &[SongCode]) -> Result<Vec<SongDto>, RemoteError> {
        Err(broken())
    }

    async fn resolve(&self, _refs: &[SongRef]) -> Result<Vec<Resolution>, RemoteError> {
        Err(broken())
    }

    async fn artists(
        &self,
        _contains: Option<&str>,
        _hidden: &[String],
    ) -> Result<Vec<ArtistRow>, RemoteError> {
        Err(broken())
    }

    async fn languages(&self, _hidden: &[String]) -> Result<Vec<LanguageRow>, RemoteError> {
        Err(broken())
    }

    async fn tags(&self, _hidden: &[String]) -> Result<Vec<TagRow>, RemoteError> {
        Err(broken())
    }

    async fn packages(&self) -> Result<Vec<PackageRow>, RemoteError> {
        Err(broken())
    }

    async fn count(&self) -> Result<usize, RemoteError> {
        Err(broken())
    }
}

fn broken() -> RemoteError {
    RemoteError::Failed("the catalog could not be read".to_owned())
}

#[derive(Clone)]
struct StubMachine {
    queued: Arc<Mutex<Vec<QueueEntryDto>>>,
    events: broadcast::Sender<Event>,
    /// What every command answers with, when it is not to succeed.
    refuse: Option<RemoteError>,
    /// What a transport button alone answers with. See [`StubMachine::refusing_transport`].
    refuse_transport: Option<RemoteError>,
    /// What a move alone answers with. See [`StubMachine::refusing_moves`].
    refuse_move: Option<RemoteError>,
    /// What a demo trigger alone answers with. See [`StubMachine::refusing_demo`].
    refuse_demo: Option<RemoteError>,
    /// How many demo triggers arrived.
    ///
    /// Counted rather than inferred from the state, because a trigger deliberately changes nothing a
    /// client can see: the real machine sets a flag its poll thread reads, and there is no poll
    /// thread here. Without this a page that quietly stopped calling the machine would pass.
    demo_starts: Arc<Mutex<usize>>,
    /// Nothing loaded and nothing playing — a machine sitting under a television in silence.
    ///
    /// The default stub always has a song on the deck, which is the right default for almost
    /// everything here and is exactly the state in which a demo may **not** be started.
    quiet: bool,
    online: bool,
    /// How many times a page has asked for the machine to be tried again.
    ///
    /// An `Arc` because `Harness::with` takes the stub by value: the counter is cloned out before
    /// the machine goes in, which is the only handle a test has on it afterwards.
    wakes: Arc<AtomicUsize>,
}

impl StubMachine {
    fn new() -> Self {
        let (events, _) = broadcast::channel(16);
        Self {
            queued: Arc::new(Mutex::new(Vec::new())),
            events,
            refuse: None,
            refuse_transport: None,
            refuse_move: None,
            refuse_demo: None,
            demo_starts: Arc::new(Mutex::new(0)),
            quiet: false,
            online: true,
            wakes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A machine that refuses a demo trigger the way a real one does — with a sentence.
    fn refusing_demo(mut self, error: RemoteError) -> Self {
        self.refuse_demo = Some(error);
        self
    }

    /// A machine with nothing on the deck, which is the only state a demo may be started from.
    fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// The counter behind [`Machine::wake`], to be taken before this is handed to a `Harness`.
    fn wakes(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.wakes)
    }

    fn refusing(mut self, error: RemoteError) -> Self {
        self.refuse = Some(error);
        self
    }

    /// A machine that queues and reorders happily and will not press a transport button.
    ///
    /// Separate from [`refusing`](Self::refusing) because `play now` is three calls, and what the
    /// pages have to get right is the *partial* result: the song is at the front and did not start.
    /// A machine that refused everything could never produce that.
    fn refusing_transport(mut self, error: RemoteError) -> Self {
        self.refuse_transport = Some(error);
        self
    }

    /// A machine that will not move a queue entry, which is what an **idle** one looks like from
    /// here: the song was queued, `advance()` took it straight to the deck, and the entry id the
    /// enqueue handed back names nothing any more. `state()` reports song 1001 as loaded, so that is
    /// the number for which this reads as "it got there" and any other reads as a real refusal.
    fn refusing_moves(mut self, error: RemoteError) -> Self {
        self.refuse_move = Some(error);
        self
    }

    fn away(mut self) -> Self {
        self.online = false;
        self.refuse = Some(RemoteError::Offline(codes::NOT_ANSWERING));
        self
    }

    fn check(&self) -> Result<(), RemoteError> {
        match &self.refuse {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn queue_dto(&self) -> QueueDto {
        let entries = self.queued.lock().expect("lock").clone();
        QueueDto {
            len: entries.len(),
            capacity: 200,
            entries,
        }
    }
}

#[async_trait::async_trait]
impl Machine for StubMachine {
    async fn state(&self) -> Result<StateDto, RemoteError> {
        self.check()?;
        if self.quiet {
            return Ok(StateDto {
                transport: TransportDto::Idle,
                now_playing: None,
                position_ms: 0,
                queue_len: self.queued.lock().expect("lock").len(),
                settings: SettingsDto {
                    transpose: 0,
                    tempo_ratio: 1.0,
                    melody_enabled: false,
                    music_volume: 0.8,
                    lyric_offset_ms: 0,
                },
            });
        }
        Ok(StateDto {
            transport: TransportDto::Playing,
            now_playing: Some(NowPlayingDto {
                origin: OriginDto::Catalog {
                    number: SongCode::new(1001),
                    entry_id: 1,
                },
                title: "Tempo Perdido".to_owned(),
                artist: Some("Legião Urbana".to_owned()),
                language: Some("pt".to_owned()),
                singer: Some("Ana".to_owned()),
                kind: km_kmpkg::SongKind::Midi,
                duration_ms: 222_000,
                melody_channel: Some(4),
                melody_available: true,
                transpose_available: true,
                tempo_available: true,
                has_lyrics: true,
            }),
            position_ms: 67_000,
            queue_len: self.queued.lock().expect("lock").len(),
            settings: SettingsDto {
                transpose: 2,
                tempo_ratio: 1.0,
                melody_enabled: true,
                music_volume: 0.8,
                lyric_offset_ms: 0,
            },
        })
    }

    async fn queue(&self) -> Result<QueueDto, RemoteError> {
        Ok(self.queue_dto())
    }

    async fn enqueue(
        &self,
        number: SongCode,
        singer: Option<&str>,
    ) -> Result<AddedToQueueDto, RemoteError> {
        self.check()?;
        let mut queued = self.queued.lock().expect("lock");
        let position = queued.len();
        queued.push(QueueEntryDto {
            id: u64::from(number.number()),
            position,
            number,
            title: format!("Song {number}"),
            artist: None,
            singer: singer.map(str::to_owned),
        });
        Ok(AddedToQueueDto {
            entry_id: u64::from(number.number()),
            position,
            title: format!("Song {number}"),
            artist: None,
        })
    }

    async fn dequeue(&self, entry_id: u64) -> Result<QueueDto, RemoteError> {
        self.check()?;
        self.queued
            .lock()
            .expect("lock")
            .retain(|e| e.id != entry_id);
        Ok(self.queue_dto())
    }

    async fn move_entry(&self, _entry_id: u64, _to_index: usize) -> Result<QueueDto, RemoteError> {
        self.check()?;
        if let Some(error) = &self.refuse_move {
            return Err(error.clone());
        }
        Ok(self.queue_dto())
    }

    async fn clear_queue(&self) -> Result<QueueDto, RemoteError> {
        self.check()?;
        self.queued.lock().expect("lock").clear();
        Ok(self.queue_dto())
    }

    async fn transport(&self, _command: Transport) -> Result<StateDto, RemoteError> {
        self.check()?;
        if let Some(error) = &self.refuse_transport {
            return Err(error.clone());
        }
        self.state().await
    }

    async fn start_demo(&self) -> Result<(), RemoteError> {
        self.check()?;
        if let Some(error) = &self.refuse_demo {
            return Err(error.clone());
        }
        *self.demo_starts.lock().expect("lock") += 1;
        Ok(())
    }

    async fn update_settings(&self, _patch: &SettingsPatchDto) -> Result<StateDto, RemoteError> {
        self.check()?;
        self.state().await
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn connection(&self) -> Connection {
        Connection {
            online: self.online,
            address: Some("http://192.168.1.5:8177".to_owned()),
            name: None,
            reason: (!self.online).then_some(codes::NOT_ANSWERING),
        }
    }

    fn wake(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}

// -- the stub link -------------------------------------------------------------------------------

/// Which machine, and what the three actions did.
///
/// It records rather than acts, because what these tests are about is the *page*: whether a typed
/// address arrives intact, whether a fruitless browse is reported as an ordinary thing, and whether a
/// build that cannot browse draws the button at all. **Normalizing is deliberately not checked here**
/// — `find::normalize` lives in `km-remote-core` and this crate cannot see it, which is the whole
/// point of the seam; `link.rs`'s own tests cover it.
#[derive(Default)]
struct StubConnectState {
    address: Option<String>,
    /// What the machine says it is called, or `None` for one that advertises no name.
    name: Option<String>,
    pinned: bool,
    /// Every address `connect_to` was handed, verbatim.
    typed: Vec<String>,
    /// Every address `use_found` was handed — an offer somebody accepted.
    picked: Vec<String>,
}

struct StubConnect {
    state: Mutex<StubConnectState>,
    /// What a browse turns up, or `None` for a network with nothing on it.
    found: Vec<String>,
    /// Whether the machine in hand is answering.
    ///
    /// The one thing a rescan branches on, so it is a field rather than derived from `address`: a
    /// remote holding an address that is *not* answering is the case where a browse still moves.
    online: bool,
    can_browse: bool,
    songs: usize,
}

impl StubConnect {
    fn new() -> Self {
        Self {
            state: Mutex::new(StubConnectState {
                address: Some("http://192.168.1.9:8177".to_owned()),
                name: Some("Living Room".to_owned()),
                pinned: false,
                typed: Vec::new(),
                picked: Vec::new(),
            }),
            found: vec!["http://192.168.1.42:8177".to_owned()],
            online: true,
            can_browse: true,
            songs: 1204,
        }
    }

    /// The same stub for a machine that advertises no name — every build before names existed.
    fn with_no_name(self) -> Self {
        self.state.lock().expect("the stub lock holds").name = None;
        self
    }

    /// The same stub holding an address nothing answers at. A rescan moves in this state.
    fn offline(mut self) -> Self {
        self.online = false;
        self
    }

    fn picked(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("the stub lock holds")
            .picked
            .clone()
    }

    fn address(&self) -> Option<String> {
        self.state
            .lock()
            .expect("the stub lock holds")
            .address
            .clone()
    }

    fn typed(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("the stub lock holds")
            .typed
            .clone()
    }
}

#[async_trait::async_trait]
impl Connect for StubConnect {
    async fn status(&self) -> MachineStatus {
        let held = self.state.lock().expect("the stub lock holds");
        MachineStatus {
            connection: Connection {
                online: held.address.is_some(),
                address: held.address.clone(),
                name: held.name.clone(),
                reason: None,
            },
            how: Some("remembered".to_owned()),
            pinned: held.pinned,
            songs: self.songs,
            can_browse: self.can_browse,
        }
    }

    async fn connect_to(&self, address: &str) -> Result<Copied, RemoteError> {
        let mut held = self.state.lock().expect("the stub lock holds");
        held.typed.push(address.to_owned());
        held.address = Some(address.to_owned());
        held.pinned = true;
        Ok(Copied {
            moved_to: Some(address.to_owned()),
            outcome: CopyOutcome::AlreadyCurrent(self.songs),
        })
    }

    /// The same answers `Link` gives, because the page's job is to render each of them.
    ///
    /// `found` is a list now rather than one address, which is the whole of the change this stub is
    /// tracking: an answer about the machine in hand, and everything else that was out there.
    async fn rescan(&self) -> Result<Scan, RemoteError> {
        let mut held = self.state.lock().expect("the stub lock holds");
        // Cleared whichever branch is taken below, which is the property `Link` states and the one
        // worth a stub honoring: the pin comes off even when nothing moves.
        held.pinned = false;
        if self.found.is_empty() {
            return Ok(Scan {
                outcome: Scanned::Nothing,
                others: Vec::new(),
            });
        }
        let here = held.address.clone();
        let offers = |skip: Option<&str>| {
            self.found
                .iter()
                .filter(|url| Some(url.as_str()) != skip)
                .map(|url| Offer {
                    url: url.clone(),
                    name: None,
                })
                .collect::<Vec<_>>()
        };
        if let Some(here) = here.filter(|here| self.found.iter().any(|url| url == here)) {
            let others = offers(Some(&here));
            return Ok(Scan {
                outcome: Scanned::Already(here),
                others,
            });
        }
        if self.online {
            return Ok(Scan {
                outcome: Scanned::Kept,
                others: offers(None),
            });
        }
        let taken = self.found[0].clone();
        held.address = Some(taken.clone());
        Ok(Scan {
            outcome: Scanned::Using(taken.clone()),
            others: offers(Some(&taken)),
        })
    }

    async fn use_found(&self, url: &str) -> Result<Copied, RemoteError> {
        let mut held = self.state.lock().expect("the stub lock holds");
        held.address = Some(url.to_owned());
        held.picked.push(url.to_owned());
        Ok(Copied {
            moved_to: Some(url.to_owned()),
            outcome: CopyOutcome::AlreadyCurrent(self.songs),
        })
    }

    async fn refresh(&self) -> Result<Copied, RemoteError> {
        Ok(Copied {
            moved_to: None,
            outcome: CopyOutcome::Imported(self.songs),
        })
    }
}

/// One folder in the stub collection.
///
/// **Named and holding its own songs**, rather than one flat set behind a folder row invented on
/// demand. The share screens are the reason: a code carries the *name* of the folder it came from,
/// and the page's job is to show that a code from `Party` is going into `Rock` — which a stub with
/// one folder called `Favorites` cannot express at all.
#[derive(Clone)]
struct StubFolder {
    id: i64,
    name: String,
    songs: HashSet<SongCode>,
    /// What each filed code is known to be, where anything is known.
    ///
    /// The stub's version of the two nullable columns beside `favorite.song_code`. Absent for a
    /// code nothing has reconciled yet, which is every code a test files by hand — the same state a
    /// real collection is in until a folder has been drawn once.
    identity: HashMap<SongCode, SongRef>,
}

#[derive(Clone)]
struct StubFavorites {
    folders: Arc<Mutex<Vec<StubFolder>>>,
}

impl Default for StubFavorites {
    fn default() -> Self {
        Self::with_folders(&[(1, "Favorites")])
    }
}

impl StubFavorites {
    /// A collection with these folders, all empty.
    fn with_folders(folders: &[(i64, &str)]) -> Self {
        Self {
            folders: Arc::new(Mutex::new(
                folders
                    .iter()
                    .map(|(id, name)| StubFolder {
                        id: *id,
                        name: (*name).to_owned(),
                        songs: HashSet::new(),
                        identity: HashMap::new(),
                    })
                    .collect(),
            )),
        }
    }

    /// Files songs directly, for a test arranging a folder rather than exercising a route.
    fn fill(&self, folder: i64, songs: &[SongCode]) {
        let mut held = self.folders.lock().expect("lock");
        if let Some(found) = held.iter_mut().find(|f| f.id == folder) {
            found.songs.extend(songs.iter().copied());
        }
    }

    /// What is filed in a folder, for a test asserting on what a merge or a restore wrote.
    fn songs_in(&self, folder: i64) -> Vec<SongCode> {
        let held = self.folders.lock().expect("lock");
        let mut songs: Vec<SongCode> = held
            .iter()
            .find(|f| f.id == folder)
            .map(|f| f.songs.iter().copied().collect())
            .unwrap_or_default();
        songs.sort_unstable();
        songs
    }

    /// Every folder's name, for a test asserting that a restore created one.
    fn names(&self) -> Vec<String> {
        let held = self.folders.lock().expect("lock");
        let mut names: Vec<String> = held.iter().map(|f| f.name.clone()).collect();
        names.sort();
        names
    }

    fn row(folder: &StubFolder) -> FolderRow {
        FolderRow {
            id: folder.id,
            name: folder.name.clone(),
            songs: folder.songs.len(),
        }
    }
}

#[async_trait::async_trait]
impl Favorites for StubFavorites {
    async fn folders(&self) -> Result<Vec<FolderRow>, RemoteError> {
        let held = self.folders.lock().expect("lock");
        let mut rows: Vec<FolderRow> = held.iter().map(Self::row).collect();
        // The real one is `ORDER BY name COLLATE NOCASE`, and a page that lists folders shows them
        // in that order — so a stub that returned insertion order would let a test pass over an
        // ordering the collection does not have.
        rows.sort_by_key(|row| row.name.to_lowercase());
        Ok(rows)
    }

    async fn folder(&self, id: i64) -> Result<Option<FolderRow>, RemoteError> {
        let held = self.folders.lock().expect("lock");
        Ok(held.iter().find(|f| f.id == id).map(Self::row))
    }

    async fn ensure_folder(&self, name: &str) -> Result<(FolderRow, bool), RemoteError> {
        let mut held = self.folders.lock().expect("lock");
        // Case-insensitively, because the real column is `COLLATE NOCASE UNIQUE` and a restore of a
        // file naming both `Party` and `party` has to land in one folder.
        if let Some(found) = held
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
            .map(Self::row)
        {
            return Ok((found, false));
        }
        let id = held.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        held.push(StubFolder {
            id,
            name: name.to_owned(),
            songs: HashSet::new(),
            identity: HashMap::new(),
        });
        Ok((
            FolderRow {
                id,
                name: name.to_owned(),
                songs: 0,
            },
            true,
        ))
    }

    async fn rename_folder(&self, _id: i64, _name: &str) -> Result<(), RemoteError> {
        Ok(())
    }

    async fn delete_folder(&self, _id: i64) -> Result<(), RemoteError> {
        Ok(())
    }

    async fn song_ids(&self, folder: i64) -> Result<Vec<SongCode>, RemoteError> {
        let held = self.folders.lock().expect("lock");
        Ok(held
            .iter()
            .find(|f| f.id == folder)
            .map(|f| f.songs.iter().copied().collect())
            .unwrap_or_default())
    }

    async fn song_refs(&self, folder: i64) -> Result<Vec<SongRef>, RemoteError> {
        let held = self.folders.lock().expect("lock");
        Ok(held
            .iter()
            .find(|f| f.id == folder)
            .map(|f| {
                f.songs
                    .iter()
                    .map(|code| {
                        f.identity
                            .get(code)
                            .cloned()
                            .unwrap_or_else(|| SongRef::code(*code))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// **Across every folder, as the real one is**, and moving the code where it moved: a stub that
    /// only recorded the identity would let a test of the repair pass without the repair happening.
    async fn reconcile(&self, rows: &[Reconciliation]) -> Result<(), RemoteError> {
        let mut held = self.folders.lock().expect("lock");
        for folder in held.iter_mut() {
            for row in rows {
                if !folder.songs.remove(&row.was) {
                    continue;
                }
                folder.identity.remove(&row.was);
                folder.songs.insert(row.now);
                folder.identity.insert(
                    row.now,
                    SongRef {
                        code: row.now,
                        package_id: row.package_id.clone(),
                        content_hash: row.content_hash.clone(),
                    },
                );
            }
        }
        Ok(())
    }

    async fn toggle(&self, folder: i64, song: SongCode) -> Result<bool, RemoteError> {
        let mut held = self.folders.lock().expect("lock");
        let Some(found) = held.iter_mut().find(|f| f.id == folder) else {
            return Err(RemoteError::NotFound);
        };
        if found.songs.remove(&song) {
            return Ok(false);
        }
        found.songs.insert(song);
        Ok(true)
    }

    async fn add_songs(&self, folder: i64, songs: &[SongCode]) -> Result<usize, RemoteError> {
        if songs.is_empty() {
            return Ok(0);
        }
        let mut held = self.folders.lock().expect("lock");
        let Some(found) = held.iter_mut().find(|f| f.id == folder) else {
            return Err(RemoteError::NotFound);
        };
        // `insert` answers whether the value was new, which is exactly what `INSERT OR IGNORE`'s
        // changed-row count answers — so this stub reports the same number the real one does, and
        // `a_merge_twice_adds_nothing_the_second_time` really observes idempotence.
        Ok(songs
            .iter()
            .filter(|song| found.songs.insert(**song))
            .count())
    }

    async fn remove(&self, folder: i64, song: SongCode) -> Result<(), RemoteError> {
        let mut held = self.folders.lock().expect("lock");
        if let Some(found) = held.iter_mut().find(|f| f.id == folder) {
            found.songs.remove(&song);
        }
        Ok(())
    }

    async fn folders_for_song(&self, song: SongCode) -> Result<Vec<i64>, RemoteError> {
        let held = self.folders.lock().expect("lock");
        Ok(if held.iter().any(|f| f.songs.contains(&song)) {
            vec![1]
        } else {
            Vec::new()
        })
    }

    async fn favorited(&self, songs: &[SongCode]) -> Result<HashSet<SongCode>, RemoteError> {
        let held = self.folders.lock().expect("lock");
        Ok(songs
            .iter()
            .copied()
            // Filed *anywhere*, which is what the star asks: `SELECT DISTINCT song_code FROM
            // favorite` does not care which folder a song is in.
            .filter(|song| held.iter().any(|folder| folder.songs.contains(song)))
            .collect())
    }
}

// -- the harness ---------------------------------------------------------------------------------

struct Harness {
    remote: Remote,
}

impl Harness {
    fn offline() -> Self {
        Self::with(StubMachine::new(), Capabilities::offline())
    }

    fn online() -> Self {
        Self::with(StubMachine::new(), Capabilities::online())
    }

    /// The offline remote with a link whose answers this test chose.
    ///
    /// Returns the stub as well, because half of what these tests assert is what the *page handed
    /// the link* — a typed address arriving intact is not visible in the markup.
    fn with_connect(connect: StubConnect) -> (Self, Arc<StubConnect>) {
        let connect = Arc::new(connect);
        let mut harness = Self::offline();
        harness.remote = harness
            .remote
            .clone()
            .with_connect(Arc::clone(&connect) as Arc<dyn Connect>);
        (harness, connect)
    }

    /// An offline remote over a corpus big enough to page through.
    ///
    /// The four-song list every other test uses cannot reach a second page, so it cannot say
    /// anything about how many rows a restore renders. Titles are `Song NNNN` in code order, which
    /// keeps `Order::Title` and the stub's own order the same thing and makes "the row at index 137"
    /// a number a test can name.
    fn with_corpus(count: usize) -> Self {
        let songs = StubSongs {
            songs: (0..count)
                .map(|index| {
                    let number = 1000 + index as u32;
                    song(number, &format!("Song {number}"), None)
                })
                .collect(),
        };
        Self {
            remote: Remote::new(
                Arc::new(songs),
                Arc::new(StubMachine::new()),
                None,
                Capabilities::online(),
            ),
        }
    }

    /// An offline remote whose songs carry tags, for the filter and its picker.
    ///
    /// Separate from [`Self::offline`] rather than tagging the four songs everybody uses, because
    /// *no tags at all* is the case most of those tests are implicitly asserting — a picker that
    /// appeared in every one of them would be a change to a dozen fixtures and would lose the one
    /// assertion worth keeping, which is that an untagged catalog draws no tag control.
    fn with_tags() -> Self {
        let tagged = |number: u32, title: &str, tags: &[&str]| {
            let mut dto = song(number, title, None);
            dto.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
            dto
        };
        let songs = StubSongs {
            songs: vec![
                tagged(1001, "Both", &["brasil", "rock"]),
                tagged(1002, "Rock only", &["rock"]),
                tagged(1003, "Neither", &[]),
            ],
        };
        Self {
            remote: Remote::new(
                Arc::new(songs),
                Arc::new(StubMachine::new()),
                None,
                Capabilities::offline(),
            ),
        }
    }

    /// An offline remote whose collection this test arranged, and the collection itself.
    ///
    /// The default stub has one folder called `Favorites`, which cannot express the thing the share
    /// screens are *about*: a code carries the name of the folder it came from, and what the confirm
    /// screen has to show is that a code from `Party` is going into `Rock`. Handing the stub back is
    /// what lets a test file songs without going through a route, and read back what a merge wrote.
    fn with_folders(folders: &[(i64, &str)]) -> (Self, Arc<StubFavorites>) {
        let favorites = Arc::new(StubFavorites::with_folders(folders));
        let mut harness = Self::offline();
        harness.remote.favorites = Some(Arc::clone(&favorites) as Arc<dyn Favorites>);
        (harness, favorites)
    }

    /// A remote over a named catalog, keeping a collection that already exists.
    ///
    /// **This is how a catalog refresh is modelled**, and it is exactly what one is: the mirror is
    /// replaced wholesale while `favorites.sqlite` beside it is untouched. Handing the same
    /// `StubFavorites` to a second harness is a phone whose machine has re-banked a package since
    /// the last time anybody opened a folder.
    fn with_catalog(songs: Vec<SongDto>, favorites: Arc<StubFavorites>) -> Self {
        let capabilities = Capabilities::offline();
        let remote = Remote::new(
            Arc::new(StubSongs { songs }),
            Arc::new(StubMachine::new()),
            Some(Arc::clone(&favorites) as Arc<dyn Favorites>),
            capabilities,
        );
        Self { remote }
    }

    fn with(machine: StubMachine, capabilities: Capabilities) -> Self {
        let songs = StubSongs {
            songs: vec![
                song(1001, "Tempo Perdido", Some("Legião Urbana")),
                song(1002, "Águas de Março", Some("Tom Jobim")),
                song(1003, "Exagerado", None),
                // A title that is really the file's own truncated name, which the corpus is full of.
                // Here so that the YouTube link has something to decline — see
                // [`a_song_links_out_to_youtube_unless_its_title_is_a_filename`].
                song(1004, "CORCOVAD", None),
            ],
        };
        let favorites: Option<Arc<dyn Favorites>> = capabilities
            .favorites
            .then(|| Arc::new(StubFavorites::default()) as Arc<dyn Favorites>);
        let remote = Remote::new(Arc::new(songs), Arc::new(machine), favorites, capabilities);
        Self { remote }
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
        let response = km_remote_pages::router(self.remote.clone())
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read the body");
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    /// The offline shell's answer to which mark its tab shows. See
    /// [`the_shell_chooses_which_mark_the_tab_shows`].
    fn with_icon(mut self, icon_png: &'static [u8]) -> Self {
        self.remote = self.remote.clone().with_icon(icon_png);
        self
    }

    /// A response body that is not text. `get` would hand back a lossy `String`, which is the wrong
    /// tool for asking whether two PNGs are the same bytes.
    async fn bytes(&self, path: &str) -> Vec<u8> {
        let request = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("build");
        let response = km_remote_pages::router(self.remote.clone())
            .oneshot(request)
            .await
            .expect("the router answers");
        to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read the body")
            .to_vec()
    }

    async fn get(&self, path: &str) -> String {
        let request = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("build");
        self.send(request).await.2
    }

    /// A navigation with the headers kept, and a cookie sent if there is one.
    ///
    /// The preference cookies are only observable this way: what the ⋯ toggle does is set one, and
    /// what it remembers is read back off the next request.
    async fn get_full(
        &self,
        path: &str,
        cookie: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut request = Request::builder().uri(path);
        if let Some(cookie) = cookie {
            request = request.header("Cookie", cookie);
        }
        self.send(request.body(Body::empty()).expect("build")).await
    }

    /// A navigation from a browser that asked for a language.
    async fn get_in(&self, path: &str, accept_language: &str) -> String {
        let request = Request::builder()
            .uri(path)
            .header("Accept-Language", accept_language)
            .body(Body::empty())
            .expect("build");
        self.send(request).await.2
    }

    async fn htmx(&self, path: &str) -> String {
        let request = Request::builder()
            .uri(path)
            .header("HX-Request", "true")
            .body(Body::empty())
            .expect("build");
        self.send(request).await.2
    }

    /// An htmx GET with the status and headers kept.
    ///
    /// The ⋯ preference needs all three at once: it is only honored for an htmx request, it answers
    /// `204`, and everything it does is in a `Set-Cookie`.
    async fn htmx_full(&self, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let request = Request::builder()
            .uri(path)
            .header("HX-Request", "true")
            .body(Body::empty())
            .expect("build");
        self.send(request).await
    }

    async fn post(&self, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let request = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("HX-Request", "true")
            .body(Body::empty())
            .expect("build");
        self.send(request).await
    }

    /// A form post. **No `Content-Type`, deliberately** — `form::field` reads the body as a string
    /// precisely so that a client which does not send one is not answered with a 415 htmx will not
    /// swap, and a test that sent the header would never exercise that.
    async fn post_form(&self, path: &str, body: &'static str) -> (StatusCode, String) {
        self.post_owned(path, body.to_owned()).await
    }

    /// The same, for a body a test had to build — a share code, or a backup document.
    ///
    /// Separate from [`Self::post_form`] rather than widening it, because every existing caller
    /// passes a literal and `&'static str` is the tighter signature for those.
    async fn post_owned(&self, path: &str, body: String) -> (StatusCode, String) {
        let request = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("HX-Request", "true")
            .body(Body::from(body))
            .expect("build");
        let (status, _, text) = self.send(request).await;
        (status, text)
    }

    /// A navigation, with the response's headers kept.
    async fn get_headers(&self, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        self.get_full(path, None).await
    }
}

/// Every `Set-Cookie` on a response, joined — a response carries several, and which one is which is
/// not worth a test knowing.
fn set_cookie(headers: &axum::http::HeaderMap) -> String {
    headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join("; ")
}

// -- the pages -----------------------------------------------------------------------------------

#[tokio::test]
async fn every_tab_renders() {
    let harness = Harness::offline();
    for (path, marker) in [
        ("/", "Tempo Perdido"),
        ("/now", "now-title"),
        ("/queue", "nowbar"),
        ("/setup", "Singing as"),
    ] {
        let page = harness.get(path).await;
        assert!(page.starts_with("\n<!doctype html>") || page.contains("<!doctype html>"));
        assert!(page.contains(marker), "{path} is missing {marker}");
        assert!(page.contains("tabbar"), "{path} has no way back");

        // **No template comment reaches a browser**, which is a real fault caught by running the
        // thing rather than by any test here: a stray `#}` inside a long `{# … #}` closes it early,
        // and everything after it becomes literal output. Every row of the Songs tab printed a
        // paragraph of this repository's own prose where the artist should have been, and the
        // templates still parsed, still rendered, and still passed every assertion above.
        //
        // A leading `#}` is what a swallowed comment ends with, so it is the cheapest thing to look
        // for and it cannot occur in anything this project legitimately prints.
        assert!(
            !page.contains("#}"),
            "{path} is printing a template comment"
        );
    }
}

/// The bank picker is `/dev/`'s, and the singer's remote has no `/settings` page at all — see
/// `Choosing a bank` in docs/decisions/audio.md.
///
/// **Not the Setup tab, which is `/setup` and is about the machine and this device.** Which bank the
/// synthesizer loads is the owner's decision made once, and it is not on either of them.
///
/// Worth a test rather than trusting the deletion, because the failure it guards is quiet in both
/// directions: a route left behind is a page nothing links to and nobody would find, and a link left
/// behind is a tab that 404s under somebody's thumb.
#[tokio::test]
async fn the_singers_remote_has_no_bank_picker() {
    for harness in [Harness::offline(), Harness::online()] {
        let (status, _, _) = harness.get_full("/settings", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "the page is gone");

        for path in ["/", "/now", "/queue", "/setup"] {
            let page = harness.get(path).await;
            assert!(
                !page.contains("href=\"/settings\""),
                "a tab leading nowhere on {path}: {page}"
            );
            assert!(
                !page.to_lowercase().contains("instrument bank"),
                "{path} still offers a bank: {page}"
            );
        }
    }
}

/// Somebody reading the queue is exactly the person who wants to skip what is playing, so the four
/// transport buttons are on that page — and the progress bar deliberately is not. See the `The Queue
/// page carries the transport` decision in docs/decisions/.
///
/// They are in the markup and folded away behind the gear, which is `The transport is behind a gear,
/// and only on the Queue tab`: hidden by CSS rather than by not being rendered, precisely so that the
/// pump can replace this fragment without the strip closing itself. That half is a rule in
/// `app.css`, which no test here can read.
#[tokio::test]
async fn the_queue_page_says_what_is_playing_and_can_stop_it() {
    for harness in [Harness::offline(), Harness::online()] {
        let page = harness.get("/queue").await;

        assert!(page.contains("Tempo Perdido"), "what is on: {page}");
        assert!(page.contains("Legião Urbana"), "and who by");
        assert!(page.contains("for Ana"), "and who asked for it");

        for control in [
            "/control/pause",
            "/control/restart",
            "/control/skip",
            "/control/stop",
        ] {
            assert!(page.contains(control), "no {control} on the queue page");
        }

        // The one thing that moves. The Now tab draws it; a second copy would mean a second fragment
        // republished every second for something nobody reading a queue is watching.
        assert!(
            !page.contains("class=\"progress\""),
            "the bar carries no progress: {page}"
        );
        // Nor any of the Now tab's own controls.
        assert!(!page.contains("control-row"), "no steppers here: {page}");
    }
}

/// The bar and the card are one state rendered twice, so a press from either page has to come back
/// as the fragment that page has room for. `?fragment=nowbar` is what says which, and getting it
/// wrong would swap the whole card — steppers, sliders and all — into the top of the queue.
#[tokio::test]
async fn a_control_pressed_from_the_queue_answers_with_the_bar_and_not_the_card() {
    let harness = Harness::offline();

    let (status, _, bar) = harness.post("/control/skip?fragment=nowbar").await;
    assert_eq!(status, StatusCode::OK);
    assert!(bar.contains("id=\"nowbar\""), "{bar}");
    assert!(bar.contains("Tempo Perdido"), "{bar}");
    assert!(!bar.contains("id=\"player\""), "not the card: {bar}");
    assert!(!bar.contains("control-row"), "not the steppers: {bar}");
    assert!(!bar.contains("class=\"progress\""), "not the bar: {bar}");

    // And the Now tab is untouched by any of it.
    let (status, _, card) = harness.post("/control/skip").await;
    assert_eq!(status, StatusCode::OK);
    assert!(card.contains("id=\"player\""), "{card}");
    assert!(card.contains("control-row"), "the steppers are still there");
    assert!(card.contains("class=\"progress\""), "so is the position");
    assert!(!card.contains("id=\"nowbar\""), "{card}");
}

/// The transport is on the Queue tab and nowhere else, and it names the element it replaces.
///
/// A mismatched target is an htmx error and a dead button, which nothing else here would catch.
/// The Now tab's half of this asserts the four buttons are *absent* there, which is `The transport
/// is behind a gear, and only on the Queue tab`.
#[tokio::test]
async fn the_transport_is_on_the_queue_tab_and_nowhere_else() {
    let harness = Harness::offline();

    let now = harness.get("/now").await;
    for action in ["play", "pause", "skip", "restart", "stop"] {
        assert!(
            !now.contains(&format!("/control/{action}\"")),
            "the Now tab must carry no transport: {action}"
        );
    }
    assert!(
        now.contains("/control/transpose-up"),
        "the four control rows stay — they adjust a performance rather than ending one"
    );

    let queue = harness.get("/queue").await;
    assert!(
        queue.contains("hx-post=\"/control/skip?fragment=nowbar\""),
        "{queue}"
    );
    assert!(queue.contains("hx-target=\"#nowbar\""), "{queue}");
    assert!(!queue.contains("hx-target=\"#player\""), "{queue}");
}

/// The ▾ beside the song's name, and the one property of it that a refactor could silently lose.
///
/// **The checkbox holding the open state has to be outside `#nowbar`.** That fragment is republished
/// whenever the song or the settings change, so state inside it would fold the strip away mid-song —
/// which is the fault the sibling combinator in `app.css` exists to avoid, and which no rendering
/// test could notice after the fact. Being outside is also what makes a tab switch close it, since
/// every tab is an ordinary link and arriving here renders the element afresh.
///
/// **The mark is a chevron and not ⋯, and not a gear either.** ⋯ on a queue row means *there is
/// more you can do to this*; this one means *this panel has a second half*, and it is the only
/// control on the page that has an open state to report. Both glyphs are in the markup and
/// `app.css` hides one, so the assertion below is on the pair rather than on either.
///
/// **Asserted inside the now bar and not across the page.** A glyph looked for anywhere in the
/// response is satisfied by the queue rows' own ⋯ as well, so an assertion written that way goes on
/// passing after this label loses its mark entirely.
#[tokio::test]
async fn the_controls_toggle_holds_its_state_outside_the_fragment_the_pump_replaces() {
    let harness = Harness::offline();

    let queue = harness.get("/queue").await;
    let toggle = queue
        .find(r#"class="icon-btn controls-toggle""#)
        .expect("the queue page carries the controls toggle");
    let label = &queue[toggle..];
    let label = &label[..label.find("</label>").expect("the toggle's label closes") + 8];
    assert!(
        label.contains(r#"<span class="when-closed">&#9662;</span>"#)
            && label.contains(r#"<span class="when-open">&#9652;</span>"#),
        "the toggle is a chevron that flips: {label}"
    );
    // And the stylesheet holds up its half. Without it both glyphs draw at once, which is a
    // rendering fault no template assertion can see.
    let css = include_str!("../static/app.css");
    assert!(
        css.contains("#nowbar-controls:not(:checked) ~ #nowbar .controls-toggle .when-open,")
            && css.contains(
                "#nowbar-controls:checked ~ #nowbar .controls-toggle .when-closed \
                 { display: none; }"
            ),
        "the rule that picks one chevron is not in the stylesheet"
    );
    // The only gear on this page is the one in the tab bar, which is every page's.
    let (content, _) = queue
        .split_once(r#"<nav class="tabbar">"#)
        .expect("the tab bar");
    assert!(
        !content.contains("&#9881;"),
        "the gear is the Setup tab's and nothing in the page body wears one: {content}"
    );

    let checkbox = queue
        .find(r#"id="nowbar-controls""#)
        .expect("the queue page carries the toggle");
    let bar = queue
        .find(r#"id="nowbar""#)
        .expect("the queue page carries the now bar");
    assert!(
        checkbox < bar,
        "the toggle must precede the bar, or the `~` combinator cannot reach it"
    );

    // And what the pump sends back on its own never contains it, which is the same statement made
    // where it would actually break.
    let (_, _, republished) = harness.post("/control/skip?fragment=nowbar").await;
    assert!(republished.contains("nowbar-controls"), "the label stays");
    assert!(
        !republished.contains(r#"id="nowbar-controls""#),
        "but the checkbox itself is not in the swapped fragment: {republished}"
    );
    assert!(
        republished.contains(r#"class="transport""#),
        "so do the buttons"
    );
}

/// The demo button: present always, enabled only when there is nothing to interrupt.
///
/// **Disabled and not absent**, which is the rule at the top of `_player.html` — absent is for what a
/// *mode* does not have, and this condition flips on every song start and every queue add. A control
/// coming and going that often reads as a fault.
#[tokio::test]
async fn the_player_card_offers_a_demo_song_only_when_there_is_nothing_to_interrupt() {
    // Something on the deck: drawn, and unpressable.
    let busy = Harness::offline().get("/now").await;
    let button = demo_button(&busy);
    assert!(button.contains("disabled"), "a song is playing: {button}");

    // Nothing on the deck: the same button, live.
    let quiet = Harness::with(StubMachine::new().quiet(), Capabilities::offline())
        .get("/now")
        .await;
    let button = demo_button(&quiet);
    assert!(!button.contains("disabled"), "{button}");

    // A queued song is somebody's answer to "what next", so the machine has no business talking over
    // it — the same condition the machine itself applies.
    let harness = Harness::with(StubMachine::new().quiet(), Capabilities::offline());
    harness.post("/song/1001/queue").await;
    let waiting = harness.get("/now").await;
    assert!(
        demo_button(&waiting).contains("disabled"),
        "somebody is waiting: {waiting}"
    );
}

/// The demo button's own tag, so an assertion about `disabled` cannot accidentally read another
/// control's.
fn demo_button(page: &str) -> String {
    let start = page
        .find(r#"hx-post="/control/demo""#)
        .expect("the card offers a demo song");
    let open = page[..start].rfind('<').expect("a tag");
    let end = page[open..].find('>').expect("a tag") + open;
    page[open..=end].to_owned()
}

/// Pressing it asks the machine, and says a song is coming — which the card itself cannot.
///
/// **The card that comes back says `Nothing playing`, and that is not a bug.** The machine sets a
/// flag and its own poll thread starts the song a moment later, so the state read to draw this
/// answer was taken while the deck was still empty. The toast is what fills the gap until
/// `SongStarted` replaces the card over the event stream.
#[tokio::test]
async fn pressing_the_demo_button_asks_the_machine_and_says_a_song_is_coming() {
    let machine = StubMachine::new().quiet();
    let presses = Arc::clone(&machine.demo_starts);
    let harness = Harness::with(machine, Capabilities::offline());

    let (status, _, body) = harness.post("/control/demo").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(*presses.lock().expect("lock"), 1, "the press arrived");
    assert!(body.contains("Starting a song"), "{body}");
    assert!(body.contains(r#"id="player""#), "and the card comes back");
}

/// A skip that succeeded on an empty deck says a song is coming, and one over a song says nothing.
///
/// **The card cannot show either outcome for itself**, which is what the toast is for: a skip into
/// silence started a demo the machine's poll thread has not loaded yet, so the state this page reads
/// back was taken while the deck was still empty. A skip over a song needs no toast, because the
/// card that comes back is the answer.
#[tokio::test]
async fn a_skip_that_starts_a_demo_says_a_song_is_coming() {
    let quiet = Harness::with(StubMachine::new().quiet(), Capabilities::offline());
    let (status, _, body) = quiet.post("/control/skip").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Starting a song"), "{body}");

    let busy = Harness::offline();
    let (status, _, body) = busy.post("/control/skip").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.contains("Starting a song"),
        "a skip over a song took a turn rather than starting a demo: {body}"
    );
}

/// A refusal is the machine's own sentence, and it does not blank the card.
///
/// The machine has the last word on the one condition no client can see — whether it has sound at
/// all — so the button is live and the refusal arrives as a 409 that `check()` has already turned
/// into a sentence. Putting a client's guess there instead would mean inventing a reason.
#[tokio::test]
async fn a_machine_that_refuses_a_demo_says_why_without_blanking_the_card() {
    let harness = Harness::with(
        StubMachine::new()
            .quiet()
            .refusing_demo(RemoteError::Unavailable {
                code: "no_sound".to_owned(),
                message: "this machine has no sound".to_owned(),
            }),
        Capabilities::offline(),
    );

    let (status, _, body) = harness.post("/control/demo").await;
    assert_eq!(status, StatusCode::OK, "htmx would ignore anything else");
    // **The page's own sentence, not the machine's.** The stub said "this machine has no sound";
    // what a singer reads is what this crate's catalog says about the code beside it, in whatever
    // language they are reading. The machine's words went to the log.
    assert!(body.contains("not making sound"), "{body}");
    assert!(
        !body.contains("this machine has no sound"),
        "the machine's own sentence reached the page: {body}"
    );
    assert!(body.contains(r#"id="player""#), "the card stays: {body}");
    assert!(!body.contains("Starting a song"), "{body}");
}

/// A row's ↑ ↓ ✕ are folded away behind ⋯, and the state is outside the fragment the pump replaces.
///
/// Same shape as the gear above and outside for the same reason — a state inside `#queue` would fold
/// the buttons away under somebody's finger the moment anybody queued a song — but a different
/// question, so this asserts the second checkbox exists rather than trusting the first to stand for
/// both.
#[tokio::test]
async fn the_queue_rows_fold_their_controls_away_behind_a_toggle() {
    let harness = Harness::offline();

    // A row to fold: the stub's queue starts empty, and an empty queue has nothing to reveal.
    harness.post("/song/1001/queue").await;

    let queue = harness.get("/queue").await;
    let checkbox = queue
        .find(r#"id="queue-actions""#)
        .expect("the queue page carries the row toggle");
    let list = queue
        .find(r#"id="queue""#)
        .expect("the queue page carries the list");
    assert!(
        checkbox < list,
        "the toggle must precede the list, or the `~` combinator cannot reach it"
    );
    assert!(
        queue.contains(r#"<label for="queue-actions""#),
        "and the label is what a finger presses: {queue}"
    );

    // The pump's own fragment carries the label and never the box, which is the half that would
    // break silently.
    let (_, _, republished) = harness.post("/queue/1001/up").await;
    assert!(republished.contains(r#"for="queue-actions""#), "the label");
    assert!(
        !republished.contains(r#"id="queue-actions""#),
        "but not the state: {republished}"
    );

    // And the stylesheet is the other half. Without this rule the buttons are on every row all the
    // time, which is the thing the toggle exists to prevent.
    let css = harness.get("/static/app.css").await;
    assert!(
        css.contains("#queue-actions:not(:checked) ~ #queue .song-actions > .icon-btn"),
        "{css}"
    );
}

/// Taking a song out of the queue, which had no test at all and did not work in either phone app.
///
/// **`hx-confirm` is why, and it is gone.** It calls `window.confirm()`, and a WebView with no
/// `WebChromeClient` (Android) or `WKUIDelegate` (iOS) answers `false` with nothing on screen — so
/// htmx read "the person said no" and never sent the request. The ↑ and ↓ beside it worked, which is
/// exactly the shape of the report. Both apps now answer the page's dialogs, and this button no
/// longer asks one: the ⋯ above the list is the guard, per `The transport is behind a gear`.
#[tokio::test]
async fn taking_a_song_out_of_the_queue_asks_nothing_and_says_what_the_queue_is_now() {
    let harness = Harness::offline();

    harness.post("/song/1001/queue").await;

    let queue = harness.get("/queue").await;
    assert!(queue.contains(r#"hx-post="/queue/1001/remove""#), "{queue}");
    assert!(
        !queue.contains("hx-confirm"),
        "a dialog no phone app could answer is not a guard: {queue}"
    );

    let (status, _, body) = harness.post("/queue/1001/remove").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"id="queue""#),
        "the list comes back: {body}"
    );
    assert!(
        body.contains("No song queued"),
        "and the song is actually gone: {body}"
    );
}

/// A page that opens on a quiet machine must still be told what is playing, and the bar is a second
/// event rather than a second target for `player` — so it needs its own listener or the Queue tab
/// silently never moves.
#[tokio::test]
async fn the_live_script_listens_for_the_now_bar() {
    let harness = Harness::offline();
    let script = harness.get("/static/live.js").await;
    assert!(script.contains("\"nowbar\""), "{script}");
}

/// The whole reason the downgrade exists: `hx-push-url` puts `fragment=list` in the address bar, so a
/// reload asks for it as a navigation and must not get a bare fragment with no layout.
#[tokio::test]
async fn the_same_url_is_a_fragment_for_htmx_and_a_page_for_a_reload() {
    let harness = Harness::offline();

    let fragment = harness.htmx("/?q=Tempo&fragment=list").await;
    assert!(fragment.contains("id=\"list\""));
    assert!(!fragment.contains("<html"), "a fragment must not be a page");

    let page = harness.get("/?q=Tempo&fragment=list").await;
    assert!(page.contains("<html"), "a reload must get the whole page");
    assert!(page.contains("tabbar"));
}

#[tokio::test]
async fn searching_narrows_the_list() {
    let harness = Harness::offline();
    let all = harness.htmx("/?fragment=rows").await;
    assert_eq!(all.matches("class=\"song-row\"").count(), 4);

    let narrowed = harness.htmx("/?q=Exagerado&fragment=rows").await;
    assert_eq!(narrowed.matches("class=\"song-row\"").count(), 1);
    assert!(narrowed.contains("Exagerado"));
}

/// The box has said "Song or number" since M14 and only now means it.
///
/// Typing a number used to run an FTS match over title and artist, so `1002` found songs with those
/// digits in their *name* — which for a catalog numbered in the thousands is usually nothing at
/// all. A code has its own grammar now, so it can simply be tried first.
#[tokio::test]
async fn searching_for_a_song_code_finds_that_song() {
    let harness = Harness::offline();

    let by_code = harness.htmx("/?q=1002&fragment=rows").await;
    assert_eq!(by_code.matches("class=\"song-row\"").count(), 1);
    assert!(by_code.contains("Águas de Março"), "{by_code}");

    // A code nobody has falls through to the ordinary search rather than answering "no such song":
    // the text may still be a title, and the box is a search box first.
    let miss = harness.htmx("/?q=999999&fragment=rows").await;
    assert_eq!(miss.matches("class=\"song-row\"").count(), 0);
}

/// Absent, not disabled. An offline-only feature is not something the online remote is withholding.
#[tokio::test]
async fn the_online_remote_has_no_star_and_no_letter_picker() {
    let online = Harness::online().get("/").await;
    assert!(!online.contains("Add to favorites"), "no star");
    assert!(!online.contains("mode=favorites"), "no favorites");
    assert!(!online.contains("id=\"initial\""), "no A–Z picker");

    let offline = Harness::offline().get("/").await;
    assert!(offline.contains("Add to favorites"));
    assert!(offline.contains("mode=favorites"));
    assert!(offline.contains("id=\"initial\""));
}

/// The A–Z filter is a `<select>` and not 27 links, and the digit bucket keeps the **value** it has
/// always had while showing a label somebody can act on. Changing the value would silently orphan
/// every `km_browse` cookie and every bookmark that carries `initial=%23`.
#[tokio::test]
async fn the_letter_picker_is_a_select_whose_digit_bucket_still_says_hash() {
    let page = Harness::offline().get("/").await;
    assert!(page.contains(r#"<select id="initial""#), "{page}");
    // `r##` because the needle itself ends in `"#` — the digit bucket's value is a hash.
    assert!(page.contains(r##"<option value="#""##), "the digit bucket");
    assert!(
        page.contains("0\u{2013}9"),
        "labeled as a bucket, not as a hash"
    );
    // `All`, and not `A–Z`: clearing the filter gives back every song, the `0–9` bucket among them,
    // so the label a picker wears while it filters nothing may not name a range that excludes one.
    assert!(
        page.contains(r#"selected>All</option>"#),
        "the option that means no filter: {page}"
    );
    assert!(
        !page.contains("A\u{2013}Z"),
        "a range that leaves the digits out is not what no filter means"
    );
    assert!(!page.contains("class=\"initials\""), "the strip is gone");
}

/// A initial arrives lower-case from a hand-typed URL and comes back selected, and an **empty**
/// `initial=` — which is what the picker submits for `All` — means no filter rather than a filter on
/// nothing.
#[tokio::test]
async fn a_chosen_letter_comes_back_selected_and_an_empty_one_means_all() {
    let harness = Harness::offline();

    let chosen = harness.get("/?initial=j").await;
    assert!(chosen.contains(r#"<option value="J" selected"#), "{chosen}");

    let cleared = harness.get("/?initial=").await;
    assert!(
        cleared.contains(r#"<option value="" selected"#),
        "{cleared}"
    );
    assert!(!cleared.contains(" selected>J<"), "nothing else is chosen");
}

/// The ⋯ toggle, which is a checkbox and a stylesheet rather than a re-render.
///
/// **The rows now always carry `↑ next` and `▶ now`**, and what the preference decides is whether
/// `app.css` draws them — so this asserts the box rather than the buttons. It used to swap the whole
/// `#browse` block, which came back as page one and threw away everything `Load more` had appended;
/// see `_song_actions.html`.
#[tokio::test]
async fn the_extra_row_actions_are_revealed_by_a_checkbox_the_stylesheet_reads() {
    let harness = Harness::offline();

    // Named in full: `/now` on its own is the Now tab's link, which is on every page.
    let plain = harness.get("/").await;
    assert!(
        plain.contains("/song/1001/next"),
        "always rendered: {plain}"
    );
    assert!(plain.contains("/song/1001/now"), "always rendered");
    assert!(
        plain.contains(r#"class="icon-btn extra-action""#),
        "{plain}"
    );
    assert!(
        plain.contains(r#"id="extra-actions""#),
        "the state: {plain}"
    );
    assert!(
        !checkbox_is_checked(&plain),
        "off until a phone says otherwise: {plain}"
    );

    // The label is what a finger presses, and it swaps nothing at all -- no `hx-target`, and the box
    // beside it is the only thing on the page that talks to the server about this.
    assert!(plain.contains(r#"<label for="extra-actions""#), "{plain}");

    // Remembered: the cookie checks the box, and the stylesheet does the rest.
    let (_, _, remembered) = harness.get_full("/", Some("km_extra=1")).await;
    assert!(checkbox_is_checked(&remembered), "{remembered}");

    // The other half, pinned here because each is inert alone: without this rule every row would
    // show `↑ next` and `▶ now` to everybody, all evening, which is the thing the toggle exists to
    // prevent. Same reason `a_badge_is_added_to_the_row_and_takes_itself_away` names its four.
    let css = harness.get("/static/app.css").await;
    assert!(
        css.contains("#extra-actions:not(:checked) ~ #browse .extra-action { display: none; }"),
        "{css}"
    );
}

/// Whether the ⋯ box came back ticked.
fn checkbox_is_checked(page: &str) -> bool {
    let Some(start) = page.find(r#"id="extra-actions""#) else {
        return false;
    };
    let tail = &page[start..];
    let end = tail.find('>').unwrap_or(tail.len());
    tail[..end].contains("checked")
}

/// A press writes the cookie and answers with nothing to draw.
///
/// **Presence is the value.** htmx sends a ticked checkbox's `actions=1` and omits an unticked one,
/// so a request carrying no `actions` is a phone saying *off* — which is what makes a second press
/// correct where a value baked into a button's own URL was not.
#[tokio::test]
async fn pressing_the_extra_toggle_stores_the_preference_and_draws_nothing() {
    let harness = Harness::offline();

    let (status, headers, body) = harness.htmx_full("/?fragment=extra&actions=1").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty(), "nothing to swap: {body}");
    assert!(set_cookie(&headers).contains("km_extra=1"));

    // Unticked, so the box sends nothing. Stored as `0` rather than cleared — `Prefs::read` tests
    // for `"1"`, so a phone that has said no keeps saying it.
    let (status, headers, _) = harness.htmx_full("/?fragment=extra").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(set_cookie(&headers).contains("km_extra=0"));
}

/// The preference is only honored for an htmx request, like every other fragment on this route.
///
/// Typed into an address bar it is an ordinary page load, which is the rule that stops a reload
/// rendering a bare fragment with no layout — and here it stops a bookmark quietly rewriting a
/// preference as a side effect of being opened.
#[tokio::test]
async fn the_extra_preference_typed_into_an_address_bar_is_an_ordinary_page() {
    let (status, headers, body) = Harness::offline()
        .get_full("/?fragment=extra&actions=1", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<!doctype html>"), "a whole page: {body}");
    assert!(!set_cookie(&headers).contains("km_extra"));
}

/// A page load that did not press the toggle must not re-send the preference: it is a header on
/// every request of an evening, saying what the phone already knows.
#[tokio::test]
async fn an_ordinary_page_load_does_not_rewrite_the_preference() {
    let (_, headers, _) = Harness::offline().get_full("/", Some("km_extra=1")).await;
    assert!(!set_cookie(&headers).contains("km_extra"));
}

/// Three calls, and the badge says which of them actually happened.
#[tokio::test]
async fn playing_a_song_now_says_so() {
    let harness = Harness::offline();
    let (status, _, body) = harness.post("/song/1001/now").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("playing"), "{body}");
    assert!(body.contains("Playing now"), "{body}");
}

/// **A failed move is not a failure when the machine was idle**, and this is not an edge case — it
/// is what happens every time somebody uses a machine that is not already playing. Queueing onto an
/// empty queue wakes it and `advance()` takes the song straight to the deck, so the entry id names
/// nothing by the time the move is asked for.
///
/// Found by running it rather than by a test: with a real machine idle, tapping ▶ answered *queued,
/// but it could not be moved up* for a song that was at that moment playing.
#[tokio::test]
async fn a_song_the_idle_machine_took_straight_to_the_deck_is_not_a_failed_move() {
    let idle = || {
        Harness::with(
            StubMachine::new().refusing_moves(RemoteError::NotFound),
            Capabilities::offline(),
        )
    };

    // 1001 is what the stub reports as loaded, so the move failed because the song got there.
    let (_, _, now) = idle().post("/song/1001/now").await;
    assert!(now.contains("playing"), "{now}");
    assert!(!now.contains("could not be moved up"), "{now}");

    let (_, _, next) = idle().post("/song/1001/next").await;
    assert!(next.contains("next up"), "{next}");
    assert!(next.contains("Playing next"), "{next}");

    // 1002 is not loaded, so the move really was refused and the song really is only queued.
    let (_, _, other) = idle().post("/song/1002/now").await;
    assert!(other.contains("could not be moved up"), "{other}");
    let (_, _, other_next) = idle().post("/song/1002/next").await;
    assert!(other_next.contains("could not be moved up"), "{other_next}");
}

/// The transport refusing is **not** the same as the queueing failing. The song really is at the
/// front, so saying `playing` would be a lie about a television the phone cannot see.
#[tokio::test]
async fn a_song_that_could_not_be_started_is_still_next() {
    let harness = Harness::with(
        StubMachine::new().refusing_transport(RemoteError::Unavailable {
            code: "nothing_playing".to_owned(),
            message: "no".to_owned(),
        }),
        Capabilities::offline(),
    );
    let (_, _, body) = harness.post("/song/1001/now").await;
    assert!(body.contains("next up"), "{body}");
    assert!(body.contains("could not be started"), "{body}");
}

/// A search box needs something that empties it *and* the rows. The native ✕ on `type="search"` does
/// the first and fires nothing, so the list would go on showing results for words that are gone.
#[tokio::test]
async fn the_clear_button_drops_the_query_and_keeps_where_you_are() {
    let harness = Harness::offline();

    // `&#38;` because askama escapes an ampersand numerically; a browser reads it as `&` either way.
    let plain = harness.get("/?q=Tempo").await;
    assert!(
        plain.contains(r#"hx-get="/?mode=songs&#38;fragment=browse""#),
        "{plain}"
    );

    // Inside an artist it clears the words and stays inside the artist — the drill-down is where you
    // are, not what you searched for.
    let nested = harness.get("/?mode=artists&artist=Tom%20Jobim&q=Mar").await;
    assert!(
        nested.contains("artist=Tom%20Jobim&#38;fragment=browse"),
        "{nested}"
    );
}

/// **The two buttons in the browse bar were dead in a browser and every test here passed.**
///
/// They sit inside the search `<form>`, which carried `hx-vals='{"fragment": "list"}'`. htmx
/// inherits `hx-vals` from every ancestor — the collector walks parents with a plain `getAttribute`,
/// so `hx-disinherit` does not reach it — and for a GET it appends what it collected onto the
/// `hx-get` URL without looking at what the query string already says. Both buttons name
/// `fragment=browse` in their own URLs, so each press asked for the key twice, `Query<BrowseParams>`
/// answered 400, and htmx does not swap on an error. Nothing happened, and nothing said why.
///
/// No test in this crate can see that, because there is no browser here. So this guards the
/// *attribute*, in the same spirit as `the_live_script_processes_what_it_inserts`: the fragment a
/// control wants belongs in that control's own URL, and nothing on the form may be inheritable.
///
/// **The ⋯ has since stopped being one of the two**, which takes it out of the collision rather than
/// guarding it: it is a `<label>` with no `hx-*` at all, and the checkbox it drives is outside the
/// form. The ✕ is the last button in the bar that builds a URL.
#[tokio::test]
async fn the_search_form_carries_nothing_its_buttons_can_inherit() {
    let page = Harness::offline().get("/").await;

    assert!(page.contains(r#"<form hx-get="/?fragment=list""#), "{page}");
    assert!(
        !page.contains("hx-vals"),
        "htmx inherits hx-vals from every ancestor and hx-disinherit does not stop it, so anything \
         set on the form is appended to the URL of every control in the bar: {page}"
    );

    // `&#38;` because askama escapes what `clear_href()` produced.
    assert!(
        page.contains(r#"hx-get="/?mode=songs&#38;fragment=browse""#),
        "{page}"
    );

    // And the toggle asks for nothing, so there is nothing for it to inherit into.
    let label = page
        .split(r#"<label for="extra-actions""#)
        .nth(1)
        .expect("the toggle is drawn");
    let label = &label[..label.find('>').expect("a tag")];
    assert!(!label.contains("hx-"), "the ⋯ swaps nothing: {label}");
}

/// The URL htmx used to build, kept as the record of why the collision was fatal rather than merely
/// untidy.
///
/// A duplicate key is refused before a handler sees it, and that is the better of the two failures:
/// last-wins would have swapped `#browse` for a bare list fragment and taken the whole bar off
/// screen, which reads like a layout bug rather than a request one. If an axum or serde bump ever
/// turns this into a 200, the reasoning in `_browse.html`'s comments needs revisiting.
#[tokio::test]
async fn a_query_key_that_arrives_twice_is_refused_rather_than_guessed_at() {
    let (status, _, _) = Harness::offline()
        .get_full(
            "/?actions=1&fragment=browse&mode=songs&q=&fragment=list",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The URL the ✕ actually builds, answered.
///
/// On its own this would not have caught the bug — nobody writes these out without already knowing
/// the shape — but until now only the *form's* URLs had server-side coverage, so a change to
/// `clear_href()` could break it without failing anything.
#[tokio::test]
async fn the_url_the_clear_button_builds_is_answered_with_the_browse_block() {
    let harness = Harness::offline();

    // `clear_href()` plus the live `#language` and `#initial`, which is all htmx sends for a GET from a
    // button: the closest form is only included automatically for a non-GET.
    let cleared = harness
        .htmx("/?mode=songs&fragment=browse&language=&initial=")
        .await;
    assert!(cleared.contains(r#"id="browse""#), "{cleared}");
    assert!(
        !cleared.contains("<html"),
        "a fragment, not a page: {cleared}"
    );
}

/// The A–Z picker is on the artists list too, and narrows it by the artist's own letter.
///
/// **The list was assumed short and a real corpus disagrees**: twelve thousand songs are several
/// thousand artists, and scrolling to `T` for Tom Jobim is the exact thing the songs list already had
/// a letter for.
///
/// An artist files under the letter their *name sorts under*, because the filter reads
/// `km_song::text::initial` — the same function the mirror stores for song titles. So this is one
/// alphabet rather than a second one that happens to agree most of the time.
#[tokio::test]
async fn the_letter_narrows_the_artists_list_by_the_artists_own_initial() {
    let harness = Harness::offline();

    let all = harness.get("/?mode=artists").await;
    assert!(all.contains(r#"<select id="initial""#), "{all}");
    assert!(
        all.contains("Legião Urbana") && all.contains("Tom Jobim"),
        "{all}"
    );

    let l = harness.get("/?mode=artists&initial=L").await;
    assert!(l.contains("Legião Urbana"), "{l}");
    assert!(!l.contains("Tom Jobim"), "{l}");
    assert!(
        l.contains(r#"value="L" selected"#),
        "the picker shows the letter it is on: {l}"
    );

    let t = harness.get("/?mode=artists&initial=T").await;
    assert!(t.contains("Tom Jobim"), "{t}");
    assert!(!t.contains("Legião Urbana"), "{t}");
}

/// A letter with no artists under it says so, rather than saying nobody has any.
///
/// **The wildcard arm would have swallowed this**, and did until the picker reached this list: the
/// `("artists", true, _, _)` arm answers "No artists — is a package installed?", which is a wrong and
/// alarming thing to tell somebody who pressed `Q` on a corpus with several thousand of them.
#[tokio::test]
async fn a_letter_with_no_artists_under_it_names_the_letter() {
    let harness = Harness::offline();

    let empty = harness.get("/?mode=artists&initial=Q").await;
    assert!(empty.contains("No artist starts with Q"), "{empty}");
    assert!(!empty.contains("is a package installed"), "{empty}");

    let searched = harness.get("/?mode=artists&initial=Q&q=jobim").await;
    assert!(
        searched.contains("No artist starting with Q matches"),
        "{searched}"
    );
}

/// Online, the artists list has no picker — for the same reason the songs list has none.
///
/// The A–Z needs the indexed folded-initial column the mirror carries and `library.sqlite` does not,
/// so `Capabilities::initial_filter` is off there. Drawing it on one of the two lists and not the
/// other would read as a bug on whichever list somebody looked at second.
#[tokio::test]
async fn the_online_remote_draws_no_a_to_z_on_the_artists_list_either() {
    let harness = Harness::online();

    let artists = harness.get("/?mode=artists").await;
    assert!(!artists.contains(r#"<select id="initial""#), "{artists}");
    let songs = harness.get("/").await;
    assert!(!songs.contains(r#"<select id="initial""#), "{songs}");
}

/// **Clearing a search inside an artist used to drop the initial**, which is the very thing the
/// hidden field above the search box exists to prevent.
///
/// The ✕ leaves the initial out of `clear_href()` on purpose and lets the live value ride along
/// through `hx-include="#language, #initial"` — right for the `<select>`, and wrong everywhere the picker
/// is not drawn. Inside an artist or a folder `shows_initials()` is false, so the initial travels as a
/// hidden `<input name="initial">`, and that input had **no id** — so `#initial` matched nothing and the
/// initial was silently dropped. It is a real filter in both places (`BrowseQuery::initial` for an
/// artist, the in-memory predicate for a folder), so the rows came back wrong, not merely different.
///
/// The two are mutually exclusive by construction — the `<select>` is drawn when `shows_initials()`
/// and the hidden field when it does not — so one id on both is one element either way, never two.
#[tokio::test]
async fn the_letter_is_addressable_wherever_it_travels() {
    let harness = Harness::offline();

    // The picker is drawn: the initial is the select's own value.
    let listing = harness.get("/?initial=J").await;
    assert!(listing.contains(r#"<select id="initial""#), "{listing}");
    assert!(
        !listing.contains(r#"type="hidden" name="initial""#),
        "{listing}"
    );

    // Inside an artist it is not drawn, and the hidden field carrying it has to answer to the same
    // selector the ✕ asks for.
    let nested = harness
        .get("/?mode=artists&artist=Legi%C3%A3o%20Urbana&initial=J&q=Tempo")
        .await;
    assert!(!nested.contains(r#"<select id="initial""#), "{nested}");
    assert!(
        nested.contains(r#"id="initial" type="hidden" name="initial" value="J""#),
        "the ✕ finds the initial by id, so the hidden field needs one: {nested}"
    );

    // And the ✕ still asks for it by that name, which is the other half of the pair. `#tags` is
    // beside it for the same reason one step on: the chosen tags are a hidden field in every view,
    // so clearing a search must send what is on screen rather than what the last render said.
    assert!(
        nested.contains(r##"hx-include="#language, #initial, #tags""##),
        "{nested}"
    );
    assert!(
        nested.contains(r#"id="tags" type="hidden" name="tags""#),
        "the chosen tags travel in every view, drawn picker or not: {nested}"
    );
}

/// A search URL and never a stored one — nothing in the workspace holds a link. The judgment about
/// which titles are worth searching for is `km_remote_pages::model`'s, and matches the package builder's.
#[tokio::test]
async fn a_song_links_out_to_youtube_unless_its_title_is_a_filename() {
    let page = Harness::offline().get("/").await;
    assert!(
        page.contains("https://www.youtube.com/results?search_query=Legi"),
        "{page}"
    );
    assert!(page.contains("Find on YouTube"));
    // `CORCOVAD` with no artist is the file's own name, and a search for it finds nothing.
    assert!(!page.contains("search_query=CORCOVAD"), "{page}");
}

/// Taking a song out of a folder was nested inside `{% if extra %}`, and `extra` was never true — so
/// this could not be done at all. It is not one of the extras: it edits this phone's own list, it
/// asks first, and it reorders nobody's evening.
#[tokio::test]
async fn the_folder_x_is_not_behind_the_toggle() {
    let harness = Harness::offline();
    harness.post("/favorites/folders/1/toggle/1001").await;

    let folder = harness.get("/?mode=favorites&folder=1").await;
    assert!(
        folder.contains("/favorites/folders/1/remove/1001"),
        "{folder}"
    );
}

/// Which mark the browser tab shows is the **shell's** answer, not the mode's.
///
/// `km-app` takes the default and `km-remote-core` calls `with_icon`, so this crate serves the amber
/// microphone inside the machine and the green one in the standalone remote. Pinned here because the
/// mistake it guards against is a quiet one: a constant creeping back into the route would put the
/// machine's mark on the offline remote's tab, which looks entirely plausible.
#[tokio::test]
async fn the_shell_chooses_which_mark_the_tab_shows() {
    assert_ne!(
        km_remote_pages::ICON_MACHINE_PNG,
        km_remote_pages::ICON_REMOTE_PNG,
        "the two marks are two pictures"
    );

    let machine = Harness::online().bytes("/static/icon.png").await;
    assert_eq!(
        machine,
        km_remote_pages::ICON_MACHINE_PNG,
        "the remote the machine serves keeps the machine's mark, without asking"
    );

    let standalone = Harness::offline()
        .with_icon(km_remote_pages::ICON_REMOTE_PNG)
        .bytes("/static/icon.png")
        .await;
    assert_eq!(standalone, km_remote_pages::ICON_REMOTE_PNG);
}

/// A link shared from the offline app, opened on the machine's own remote.
#[tokio::test]
async fn a_mode_this_build_lacks_falls_back_to_the_songs_list() {
    let page = Harness::online().get("/?mode=favorites").await;
    assert!(page.contains("Tempo Perdido"), "landed on the songs list");
}

#[tokio::test]
async fn queueing_answers_with_a_badge_and_a_toast() {
    let harness = Harness::offline();
    let (status, _, body) = harness.post("/song/1001/queue").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("queued"), "{body}");
    assert!(body.contains("hx-swap-oob"), "the toast rides out of band");

    let queue = harness.get("/queue").await;
    assert!(queue.contains("Song 1001"));
}

/// htmx does not swap on an error response, so a refusal answered with a 409 would leave the button
/// dead and the reason nowhere. Every ordinary refusal is a 200 carrying a toast.
#[tokio::test]
async fn a_full_queue_is_a_toast_and_not_an_error_status() {
    let harness = Harness::with(
        StubMachine::new().refusing(RemoteError::QueueFull),
        Capabilities::offline(),
    );
    let (status, _, body) = harness.post("/song/1001/queue").await;
    assert_eq!(status, StatusCode::OK, "htmx would ignore anything else");
    assert!(body.contains("queue is full"), "{body}");
}

/// The four halves of "the badge goes away again", each in the file that owns it. Pinning them
/// together is the point: a badge that is added but never hidden behind, or hidden behind but never
/// removed, is a row nobody can press a second time — which is the fault all of this replaced.
#[tokio::test]
async fn a_badge_is_added_to_the_row_and_takes_itself_away() {
    let harness = Harness::offline();

    // 1. The press adds to the slot rather than replacing it, so the buttons are still underneath.
    let rows = harness.get("/").await;
    assert!(
        rows.contains(r##"hx-target="#song-1001" hx-swap="afterbegin""##),
        "the row's buttons add their answer rather than standing aside for it: {rows}"
    );
    assert!(
        !rows.contains(r##"hx-target="#song-1001" hx-swap="innerHTML""##),
        "`innerHTML` here is the swap that leaves one permanent green box: {rows}"
    );

    // 2. The badge carries the class the other two halves are keyed on.
    let (_, _, badge) = harness.post("/song/1001/queue").await;
    assert!(badge.contains(r#"class="badge flash"#), "{badge}");

    // 3. The stylesheet hides the buttons for as long as it is there.
    let css = harness.get("/static/app.css").await;
    assert!(
        css.contains(".song-actions:has(.badge.flash) > .icon-btn"),
        "{css}"
    );
    assert!(
        css.contains("@keyframes flash"),
        "the animation is the clock"
    );

    // 4. And the script takes it out again when the clock runs down. Without this the rule above
    //    never stops matching and the buttons never come back.
    let script = harness.get("/static/live.js").await;
    assert!(script.contains("animationend"), "{script}");
    assert!(
        script.contains(r#"classList.contains("flash")"#),
        "{script}"
    );
}

/// A refusal never went anywhere near the buttons, so it has nothing to put back — and what it used
/// to put back was a second `<div class="song-actions" id="song-1001">`, which gave the document two
/// elements with one id and dropped the folder ✕ along the way.
#[tokio::test]
async fn a_refused_action_answers_with_the_toast_alone() {
    let harness = Harness::with(
        StubMachine::new().refusing(RemoteError::QueueFull),
        Capabilities::offline(),
    );
    let (_, _, body) = harness.post("/song/1001/queue").await;
    assert!(body.contains("queue is full"), "the reason is said: {body}");
    assert!(
        !body.contains("song-actions"),
        "the slot is not sent back, because it never left: {body}"
    );
    assert!(
        !body.contains(r#"id="song-1001""#),
        "and so cannot arrive as a duplicate id: {body}"
    );
}

/// The command almost certainly arrived; what is missing is the confirmation. Saying "sent" beats
/// coloring a song that is in fact queued as a failure.
#[tokio::test]
async fn an_unacknowledged_command_says_sent_rather_than_failed() {
    let harness = Harness::with(
        StubMachine::new().refusing(RemoteError::NotAcknowledged),
        Capabilities::offline(),
    );
    let (_, _, body) = harness.post("/song/1001/queue").await;
    assert!(body.contains("sent"), "{body}");
    assert!(
        !body.contains("toast-bad"),
        "not colored as a failure: {body}"
    );
}

/// The offline app spends most of its life here, and it is not an error page.
#[tokio::test]
async fn a_machine_that_is_away_still_renders_every_page() {
    let harness = Harness::with(StubMachine::new().away(), Capabilities::offline());

    let browse = harness.get("/").await;
    assert!(browse.contains("Tempo Perdido"), "browsing still works");
    assert!(
        browse.contains("The karaoke machine is not answering."),
        "the banner says so"
    );

    let now = harness.get("/now").await;
    assert!(
        now.contains("Nothing playing"),
        "an idle card, not an error"
    );

    // The same for the Queue tab's bar, and for the same reason: the offline app spends most of its
    // life here, and a bar that vanished would make "the television is off" look like a fault.
    let queue = harness.get("/queue").await;
    assert!(
        queue.contains("id=\"nowbar\"") && queue.contains("Nothing playing"),
        "an idle bar, not a missing one: {queue}"
    );

    assert!(
        browse.contains("&mdash; retrying http://192.168.1.5:8177")
            || browse.contains("— retrying http://192.168.1.5:8177"),
        "and which machine it is failing to reach: {browse}"
    );
}

/// **The strip says it is trying before it says it failed**, and it arrives in that phase.
///
/// The machine going quiet for a moment is common and mostly means nothing — a stalled heartbeat on
/// the Android machine, a phone whose radio has just woken — and the remote is already trying again
/// within about a second. Reported as a red bar appearing over and over through an evening.
///
/// The reason is in the markup from the start rather than fetched later: the phase is a class, and
/// `static/live.js` takes it off after `BANNER_GRACE_MS`.
#[tokio::test]
async fn the_banner_says_it_is_trying_before_it_says_the_machine_is_unreachable() {
    let harness = Harness::with(StubMachine::new().away(), Capabilities::offline());
    let browse = harness.get("/").await;

    assert!(
        browse.contains(r#"class="banner banner-trying""#),
        "the strip starts in its quiet phase: {browse}"
    );
    assert!(
        browse.contains("Reconnecting"),
        "...and says what it is doing while it is in it: {browse}"
    );
    assert!(
        browse.contains("The karaoke machine is not answering."),
        "...with the reason already there, for the script to reveal: {browse}"
    );
}

/// One phase, spelled in three files that cannot see each other.
///
/// The template writes the class, the stylesheet decides what it looks like and the script takes it
/// off again — so a rename in one place is a strip that either never turns red or never stops being
/// quiet, with nothing failing to say so. This is the cheapest guard against that and the only one
/// available: there is no browser in this file's loop.
#[tokio::test]
async fn the_banners_first_phase_is_named_the_same_in_the_template_the_style_and_the_script() {
    let harness = Harness::with(StubMachine::new().away(), Capabilities::offline());

    for (what, body) in [
        ("the rendered page", harness.get("/").await),
        ("the stylesheet", harness.get("/static/app.css").await),
        ("the script", harness.get("/static/live.js").await),
    ] {
        assert!(
            body.contains("banner-trying"),
            "{what} does not know the phase by that name"
        );
    }

    let script = harness.get("/static/live.js").await;
    assert!(
        script.contains("BANNER_GRACE_MS") && script.contains("BANNER_LINGER_MS"),
        "both phases have a named duration: {script}"
    );
}

/// The banner's other half, which nothing covered: a machine that is *there* still renders the
/// element, empty and unstyled.
///
/// It is a swap target before it is a message. An absent one is where the next `banner` event would
/// have landed, and `static/live.js` hides a spent one rather than removing it for the same reason —
/// so "no banner" has to mean an empty div, never no div.
#[tokio::test]
async fn a_machine_that_is_there_leaves_the_banner_empty_rather_than_absent() {
    let harness = Harness::offline();

    let browse = harness.get("/").await;
    assert!(
        browse.contains(r#"<div data-sse="banner"></div>"#),
        "the swap target survives: {browse}"
    );
    assert!(
        !browse.contains(r#"data-sse="banner" class="banner""#),
        "and says nothing while the machine is there: {browse}"
    );
}

#[tokio::test]
async fn the_star_files_a_song_and_comes_back_filled() {
    let harness = Harness::offline();
    let (_, _, body) = harness.post("/favorites/folders/1/toggle/1001").await;
    assert!(
        body.contains("hx-swap-oob"),
        "the star updates behind the sheet"
    );
    assert!(body.contains("&#9733;"), "and it is filled: {body}");
    assert!(body.contains("Added to Favorites"), "{body}");
}

/// A field and the button beside it, on four rows, and none of them may push the button off screen.
///
/// **Reported in Portuguese, and that is the whole of why it was not found sooner.** `Salvar` is
/// wider than `Save` and `Adicionar` is wider than `Add`, so the singer's row on the Queue tab drew
/// its word outside its own button while the English one had been sitting just inside the same edge.
/// An `<input>` is `width: 100%`, which as a flex item is a basis of the whole row, and a flex item
/// does not shrink below its content minimum without `min-width` — so the field conceded nothing,
/// and `.icon-btn`'s explicit `min-width: 2.1rem` had replaced the floor that would have stopped the
/// button being squeezed under its own text. Measured: a 43px button around a 48px word.
///
/// Four rows are this shape and only `.machine-change` ever carried the fix. They share one class
/// now, and this asserts both halves: that the rule is in the stylesheet, and that every one of the
/// four wears it. A fifth written as a bare flex `<form>` is what the second half is here to catch.
#[tokio::test]
async fn a_field_row_lets_its_field_shrink_so_the_button_is_not_pushed_off() {
    let harness = Harness::offline();

    let css = harness.get("/static/app.css").await;
    assert!(
        css.contains(".field-row input { flex: 1; min-width: 6rem; }"),
        "the field gives way: {css}"
    );
    assert!(
        css.contains(".field-row { display: flex; flex-wrap: wrap;"),
        "and the button has somewhere to go: {css}"
    );

    // The singer's name and the language picker, which is where it was seen. Both on the Setup tab
    // now; the Queue tab carries neither.
    let setup = harness.get("/setup").await;
    assert_eq!(
        setup.matches(r#"class="pref-row field-row""#).count(),
        2,
        "both rows on the Setup tab: {setup}"
    );
    let queue = harness.get("/queue").await;
    assert!(
        !queue.contains(r#"class="pref-row field-row""#),
        "and neither is left behind on the Queue tab: {queue}"
    );

    // The search box and its ✕, on the Songs tab.
    let browse = harness.get("/").await;
    assert!(browse.contains(r#"<div class="field-row">"#), "{browse}");

    // The new-folder box and *Add*, in the sheet.
    let sheet = harness.get("/favorites/sheet/1001").await;
    assert!(sheet.contains(r#"class="field-row""#), "{sheet}");

    // The machine's address and *Use*, behind the disclosure on the Setup tab. Offline only, and it
    // needs a link that has found something, which is what `with_connect` is for.
    let (linked, _connect) = Harness::with_connect(StubConnect::new());
    let setup = linked.get("/setup").await;
    assert!(setup.contains(r#"<form class="field-row""#), "{setup}");
}

#[tokio::test]
async fn the_stylesheet_is_served() {
    let harness = Harness::online();
    let request = Request::builder()
        .uri("/static/app.css")
        .body(Body::empty())
        .expect("build");
    let (status, headers, body) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/css; charset=utf-8");
    assert!(body.contains("--tabbar-height"));
}

/// Every fragment `live.js` inserts must be handed to `htmx.process`.
///
/// A guard rather than a real test, and it is here because a real one is not available: htmx binds
/// `hx-*` triggers only in `htmx.process()` and carries no `MutationObserver`, so a fragment that
/// file inserts without processing renders perfectly and is completely inert. That is exactly what
/// happened — the hub replays the latest `player` frame as a stream opens, so the thirteen `hx-post`
/// buttons in `_player.html` were replaced by dead copies within milliseconds of the page loading,
/// and the Now tab had never worked for anybody. Nothing in this file could see it, because there is
/// no browser here: the router is driven as a service and every assertion is over an HTML string.
///
/// So this asserts the one thing that *is* checkable without one — that the call has not gone away
/// again. See "Why htmx's SSE extension is not vendored" in `static/README.md`.
#[tokio::test]
async fn the_live_script_processes_what_it_inserts() {
    let harness = Harness::online();
    let request = Request::builder()
        .uri("/static/live.js")
        .body(Body::empty())
        .expect("build");
    let (status, _, body) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("htmx.process"),
        "live.js must hand every fragment it inserts to htmx.process, or every control in a \
         server-pushed fragment is inert"
    );
}

/// The stream must be released when its page stops being the one on screen.
///
/// The same kind of guard as [`the_live_script_processes_what_it_inserts`] above, and here for the
/// same reason: there is no browser in this file, so what a socket does across a navigation cannot
/// be observed here at all. What can be asserted is that the two halves of the fix are still in the
/// script.
///
/// What it is guarding against: the three tabs are ordinary links, so every tab press replaces the
/// document. A document that opened a stream and never closed it left the socket held while the
/// browser cached the page, and a browser allows about six connections to one host — so a few tab
/// presses spent the budget and the next navigation queued behind a page nobody was looking at.
/// With `hx-sync="body:queue last"` on the body, the first request to queue stopped every later tap
/// too. It presented as the remote freezing for half a minute with nothing wrong at either end.
#[tokio::test]
async fn the_live_script_releases_its_stream_when_the_page_leaves() {
    let harness = Harness::online();
    let request = Request::builder()
        .uri("/static/live.js")
        .body(Body::empty())
        .expect("build");
    let (status, _, body) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"addEventListener("pagehide""#) && body.contains(".close()"),
        "live.js must close its EventSource on pagehide, or an abandoned page goes on holding one \
         of the browser's six connections to this host"
    );
    assert!(
        body.contains(r#"addEventListener("pageshow""#),
        "...and reopen it on pageshow, or a page restored from the browser's cache never updates \
         again"
    );
}

/// The other half of the same pair, and the half a WebView needs.
///
/// **Backgrounding the Android remote is not a navigation**, so `pagehide` and `pageshow` never
/// fire: the stream is neither closed nor reopened, no replay happens, and the page keeps whatever
/// fragment it was last pushed — a red banner for a machine that has since come back. Reopening on
/// the way back is what brings it up to date, and the handler turns that same reopen into a request
/// to retry the machine now.
///
/// Asserted against the served text because there is no browser anywhere in this file's loop; see
/// `static/README.md`.
#[tokio::test]
async fn the_live_script_reopens_its_stream_when_the_page_comes_back_on_screen() {
    let script = Harness::offline().get("/static/live.js").await;
    assert!(
        script.contains(r#"addEventListener("visibilitychange""#)
            && script.contains(r#"visibilityState === "visible""#),
        "live.js must take a fresh stream when the page is in front of somebody again: {script}"
    );
}

/// And it must **only** wake on becoming visible, never close on becoming hidden.
///
/// The reason `visibilitychange` was rejected outright for years: it fires when the window merely
/// loses focus, and a queue left up on a second screen while somebody works in another window is a
/// real use. Closing there would freeze a page that is being looked at.
#[tokio::test]
async fn the_live_script_never_drops_its_stream_merely_for_losing_focus() {
    let script = Harness::offline().get("/static/live.js").await;
    assert!(
        !script.contains(r#""hidden""#),
        "nothing in live.js may act on the hidden state -- a queue on a second screen is a real \
         use, and closing its stream would freeze the page somebody is watching: {script}"
    );
}

// -- coming back to the row you were reading ------------------------------------------------------

/// The list's own stamp, read off the page rather than computed.
///
/// A test cannot spell the tag without new public API and should not need to: reading it back and
/// feeding it in is what pins the round trip, which is the thing that can actually break.
fn list_tag_of(body: &str) -> String {
    let marker = r#"id="rows" data-list=""#;
    let start = body.find(marker).expect("the rows carry a list tag") + marker.len();
    let rest = &body[start..];
    rest[..rest.find('"').expect("the tag is closed")].to_owned()
}

/// The whole point: a bare tab press comes back with the row named on it.
#[tokio::test]
async fn a_tab_press_comes_back_to_the_row_you_were_reading() {
    let harness = Harness::online();
    let tag = list_tag_of(&harness.get("/").await);

    let (status, _, body) = harness
        .get_full("/", Some(&format!("km_at=row=1003&at=2&list={tag}")))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"data-anchor="1003""#), "{body}");
    // And the row it names is on the page for the script to find.
    assert!(body.contains(r#"data-row="1003""#), "{body}");
}

/// The search case, and the reason the anchor carries a stamp of its own.
///
/// `_browse.html` swaps `#list` without a navigation, so an anchor captured before a search outlives
/// the list it belonged to — and `km_browse`, which is rewritten by that same swap, cannot be used
/// to notice.
#[tokio::test]
async fn an_anchor_captured_in_another_list_is_ignored() {
    let harness = Harness::online();

    let (_, _, body) = harness
        .get_full("/", Some("km_at=row=1003&at=2&list=deadbeef"))
        .await;

    assert!(!body.contains("data-anchor"), "{body}");
}

/// A URL that says something is itself the answer — the rule `km_browse` already keeps.
#[tokio::test]
async fn only_a_bare_tab_press_restores() {
    let harness = Harness::online();
    let tag = list_tag_of(&harness.get("/").await);

    let (_, _, body) = harness
        .get_full(
            "/?q=tempo",
            Some(&format!("km_at=row=1003&at=2&list={tag}")),
        )
        .await;

    assert!(!body.contains("data-anchor"), "{body}");
}

/// An htmx fragment must never carry one, which is what leaves the deliberate jump to the top after
/// a search alone.
#[tokio::test]
async fn an_htmx_fragment_never_carries_the_anchor() {
    let harness = Harness::online();
    let tag = list_tag_of(&harness.get("/").await);

    let request = Request::builder()
        .uri("/?fragment=list")
        .header("HX-Request", "true")
        .header("Cookie", format!("km_at=row=1003&at=2&list={tag}"))
        .body(Body::empty())
        .expect("build");
    let (_, _, body) = harness.send(request).await;

    assert!(!body.contains("data-anchor"), "{body}");
}

/// Rows past the first fifty exist only because somebody pressed `Load more`, and nothing on the
/// server records that — so a restore has to render them again or the row it names is not there.
#[tokio::test]
async fn a_deep_anchor_brings_its_pages_back_with_it() {
    let harness = Harness::with_corpus(600);
    let tag = list_tag_of(&harness.get("/").await);

    let (_, _, body) = harness
        .get_full("/", Some(&format!("km_at=row=1137&at=137&list={tag}")))
        .await;

    assert!(body.contains(r#"data-anchor="1137""#), "{body}");
    assert!(
        body.contains(r#"data-row="1137""#),
        "the row itself is missing: {body}"
    );
    // Rounded up to a whole page, so the row has at most a page below it and the button carries on
    // from a page boundary rather than from 138: index 137 lives in the third page, so three of them.
    assert!(body.contains("offset=150"), "{body}");
}

/// And it stops rather than rendering a corpus into a phone.
#[tokio::test]
async fn a_restore_is_capped_rather_than_rendering_a_whole_corpus() {
    let harness = Harness::with_corpus(600);
    let tag = list_tag_of(&harness.get("/").await);

    let (_, _, body) = harness
        .get_full("/", Some(&format!("km_at=row=1590&at=590&list={tag}")))
        .await;

    // Ten pages and no more. The row it asked for is past that, so nothing scrolls and the top of
    // the list is what you get — which is what this tab did before any of this existed.
    assert!(body.contains("offset=500"), "{body}");
    assert!(!body.contains(r#"data-row="1590""#), "{body}");
}

/// The button is a trigger as well: reaching it is what asks for the page after it.
///
/// `click` is asserted beside `revealed` because naming a trigger replaces the implicit one a
/// `<button>` carries, so dropping it would trade the press for the scroll rather than adding to
/// it, and the press is what carries on a list whose automatic fetch `hx-sync` dropped.
///
/// Asserted against the same element as the URL, which is the whole of what there is to get wrong:
/// there is no browser here, so a trigger on the wrong control reads exactly like one on the right
/// control.
#[tokio::test]
async fn the_next_page_is_asked_for_by_being_scrolled_to() {
    let harness = Harness::with_corpus(600);

    let body = harness.htmx("/?fragment=rows").await;

    assert!(
        body.contains(r#"&amp;fragment=rows" hx-trigger="revealed, click""#),
        "{body}"
    );
}

/// And a list with nothing after it carries no trigger at all.
///
/// htmx binds `revealed` by starting a scroll listener and a timer that live as long as the page,
/// so a list that cannot page is a list that never starts them.
#[tokio::test]
async fn a_list_with_no_next_page_is_not_scrolled_to() {
    let harness = Harness::offline();

    let body = harness.htmx("/?fragment=rows").await;

    assert_eq!(body.matches("class=\"song-row\"").count(), 4);
    assert!(!body.contains("revealed"), "{body}");
    assert!(!body.contains("load-more"), "{body}");
}

/// Anybody can edit a cookie, and the answer to a bad one is to render the page without it.
#[tokio::test]
async fn a_malformed_anchor_is_ignored_rather_than_refused() {
    let harness = Harness::online();

    let (status, _, body) = harness
        .get_full("/", Some("km_at=row=hello&at=-1&list="))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("data-anchor"), "{body}");
}

/// The one cookie the server reads and never writes.
///
/// Pins the rule rather than the behavior: a Rust setter for this would have to be the first
/// non-`HttpOnly` cookie the remote emits, and `km_token` is why that default is worth keeping
/// absolute.
#[tokio::test]
async fn nothing_on_the_server_ever_writes_the_anchor_cookie() {
    let harness = Harness::online();
    let (_, headers, _) = harness.get_full("/", None).await;

    let said = set_cookie(&headers);
    assert!(
        said.contains("km_browse"),
        "the browse cookie still lands: {said}"
    );
    assert!(!said.contains("km_at"), "{said}");
}

/// A filter is remembered across a break in an evening and not across the night.
///
/// The header a browser actually receives, rather than the function that builds it: what expires the
/// filter is the browser reading this `Max-Age`, so the two hours are worth asserting where they are
/// delivered. The restore itself is the other half — `only_a_bare_tab_press_restores` covers the
/// cookie losing to a URL, and the state a bare tab press comes back to is `handlers`' own tests.
#[tokio::test]
async fn a_filter_is_remembered_for_two_hours() {
    let harness = Harness::online();
    let (_, headers, _) = harness.get_full("/?language=pt", None).await;

    let said = set_cookie(&headers);
    assert!(said.contains("km_browse=language%3Dpt"), "{said}");
    assert!(said.contains("Max-Age=7200"), "{said}");
}

/// The capture half, asserted against the served text — there is no browser in this file.
#[tokio::test]
async fn the_live_script_remembers_which_row_was_on_screen() {
    let script = Harness::offline().get("/static/live.js").await;

    assert!(script.contains("km_at"), "{script}");
    assert!(script.contains("document.cookie"), "{script}");
    assert!(
        script.contains(r#"addEventListener("pagehide""#),
        "{script}"
    );
}

/// The restore half, and the negative that keeps two other statements honest: `desktop.rs` says the
/// remote's pages use no `localStorage`, and this position is a row rather than a saved pixel.
#[tokio::test]
async fn the_live_script_scrolls_to_a_row_and_never_to_a_pixel() {
    let script = Harness::offline().get("/static/live.js").await;

    assert!(script.contains("dataset.anchor"), "{script}");
    assert!(script.contains("scrollBy"), "{script}");
    for forbidden in ["localStorage", "sessionStorage", "scrollRestoration"] {
        assert!(
            !script.contains(forbidden),
            "live.js must not reach for {forbidden}: {script}"
        );
    }
}

// -- a stream opening is a request to try the machine again --------------------------------------

/// Opens `GET /events` and answers with the status, **without reading the body**.
///
/// **A test that reads this body never returns.** `Hub::stream` is an SSE response with a keep-alive
/// and no end, and `Harness::send` — like `get` and `htmx` above it — calls `to_bytes` on whatever
/// it gets. There was no test of this route until now, so nothing warned anybody; reach for this
/// helper rather than the harness.
///
/// The harness must also be unguarded: `/events` is gated on `events.subscribe`, so a guarded one
/// answers 303 and the handler never runs.
async fn open_stream(harness: &Harness) -> StatusCode {
    let request = Request::builder()
        .uri("/events")
        .body(Body::empty())
        .expect("build");
    let response = km_remote_pages::router(harness.remote.clone())
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    drop(response);
    status
}

/// **The reported fault, at the seam that fixes it.** Coming back to the Android remote showed a red
/// strip for ten seconds while a machine that was up went unasked: backgrounding a WebView is not a
/// navigation, so the page's stream was neither closed nor reopened, and the reconnection loop was
/// waiting out a backoff measured before the process was frozen.
///
/// A stream opening is the signal, and it needs no route: a page opens one on load, on every tab
/// press, and — since `live.js` learned to — on coming back on screen.
#[tokio::test]
async fn opening_an_event_stream_asks_an_absent_machine_to_try_again_now() {
    let machine = StubMachine::new().away();
    let wakes = machine.wakes();
    let harness = Harness::with(machine, Capabilities::offline());

    assert_eq!(open_stream(&harness).await, StatusCode::OK);
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        1,
        "a page coming back is somebody saying they think the machine is there"
    );
}

/// And the other branch, which is why the gate is in the handler rather than in the trait method:
/// there is nothing to retry about a connection that is up, and a page load should not disturb one.
#[tokio::test]
async fn opening_an_event_stream_does_not_disturb_a_machine_that_is_answering() {
    let machine = StubMachine::new();
    let wakes = machine.wakes();
    let harness = Harness::with(machine, Capabilities::offline());

    assert_eq!(open_stream(&harness).await, StatusCode::OK);
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
}

// -- the asset budget ----------------------------------------------------------------------------
//
// The other half of the same fault. These four files were served with a content type and nothing
// else, so a browser had no freshness information for them and re-fetched all four on every
// navigation — which, the tabs being links, is every tab press. Five requests where one would do,
// against a budget of about six connections that is already one short because the event stream is
// holding one.

/// Every static file says it may be kept, and kept for a long time.
#[tokio::test]
async fn every_static_file_may_be_cached_for_a_year() {
    let harness = Harness::online();
    for path in [
        "/static/app.css",
        "/static/live.js",
        "/static/htmx.min.js",
        "/static/icon.png",
    ] {
        let request = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("build");
        let (status, headers, _) = harness.send(request).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let cache = headers
            .get("cache-control")
            .unwrap_or_else(|| panic!("{path} carries no Cache-Control"))
            .to_str()
            .expect("ascii");
        assert!(cache.contains("max-age=31536000"), "{path}: {cache}");
        // `immutable` is what stops a reload revalidating, and it is only honest because the URLs
        // carry a stamp — see the test below.
        assert!(cache.contains("immutable"), "{path}: {cache}");
    }
}

/// ...and every reference to one carries the stamp that makes `immutable` true.
///
/// Both halves matter and neither is any use alone: caching for a year without a stamp serves a
/// stale stylesheet after the next build, and a stamp without caching is a longer URL for the same
/// five requests.
#[tokio::test]
async fn every_asset_url_carries_the_version_stamp() {
    let stamp = km_remote_pages::ASSET_VERSION;
    assert_eq!(stamp.len(), 8, "the stamp is eight hex digits: {stamp}");
    assert!(
        stamp.chars().all(|character| character.is_ascii_hexdigit()),
        "the stamp goes into a URL unescaped: {stamp}"
    );

    // Any full page: the references are in `layout.html`, which all three extend.
    let page = Harness::online().get("/queue").await;
    for file in ["app.css", "live.js", "htmx.min.js", "icon.png"] {
        assert!(
            page.contains(&format!("/static/{file}?v={stamp}")),
            "{file} is referenced without the stamp, so it cannot be cached: {page}"
        );
    }
    // Nothing may reference one bare, or that file alone goes on being re-fetched and the fault
    // survives in miniature.
    for file in ["app.css", "live.js", "htmx.min.js"] {
        assert!(
            !page.contains(&format!("/static/{file}\"")),
            "{file} is referenced without a stamp somewhere in the page"
        );
    }
}

// -- which machine -------------------------------------------------------------------------------

/// The card is on the Setup tab in the offline mode, and the address is on it.
#[tokio::test]
async fn the_offline_remote_shows_which_machine_it_is_using() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new());
    let body = harness.get("/setup").await;

    assert!(
        body.contains(r#"id="machine""#),
        "the card is on the Setup tab"
    );
    assert!(
        body.contains("http://192.168.1.9:8177"),
        "the address is the one thing somebody came to this card to read"
    );
    assert!(
        body.contains("Living Room"),
        "the name the owner gave the machine leads the card"
    );
    assert!(body.contains("1204 songs in this copy"));
    assert!(body.contains("remembered"), "how it was arrived at");
    assert!(
        body.contains("/machine/rescan") && body.contains("/machine/refresh"),
        "both actions are offered"
    );
}

/// A machine that advertises no name is shown by its address, with no empty heading above it.
///
/// The reader's half of the naming rules, from the card's side. Every machine built before names
/// could be set is this case, and so is one whose settings file was edited by hand — so it is the
/// state the card has to draw well, not an edge.
#[tokio::test]
async fn a_machine_with_no_name_is_shown_by_its_address_alone() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new().with_no_name());
    let body = harness.get("/setup").await;

    assert!(
        body.contains("http://192.168.1.9:8177"),
        "the address is still there and is the whole answer"
    );
    assert!(
        !body.contains("machine-name"),
        "no heading element at all, rather than an empty one"
    );
}

/// Absent online, not disabled — the `Capabilities` rule.
///
/// Two reasons for it rather than one, which is why this is worth asserting: the capability is off
/// *and* `Remote::connect` is `None`, because online the machine is this process and there is nothing
/// for either half to point at.
#[tokio::test]
async fn the_online_remote_has_no_machine_card_at_all() {
    let harness = Harness::online();
    let body = harness.get("/setup").await;
    assert!(
        !body.contains(r#"id="machine""#),
        "the online remote is inside the machine; there is nothing to choose"
    );
    assert!(!body.contains("/machine/rescan"));
}

/// The address form is **outside** the fragment the pump replaces.
///
/// The one structural claim on this page that is not cosmetic: the pump republishes `#machine` every
/// second, so an input inside it would be replaced under somebody's finger and lose a half-typed
/// address. Asserted by position, because position is the whole of the protection.
#[tokio::test]
async fn the_address_box_is_not_inside_the_fragment_the_pump_replaces() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new());
    let body = harness.get("/setup").await;

    let card = body.find(r#"id="machine""#).expect("the card is drawn");
    let closes = body[card..].find("</div>").expect("the card closes") + card;
    let input = body.find(r#"name="address""#).expect("the box is drawn");
    assert!(
        input > closes,
        "the address input must sit outside `#machine`, which the pump replaces once a second"
    );
}

/// What somebody typed reaches the link verbatim.
///
/// **Not normalized here, and that is the seam working.** `find::normalize` lives in
/// `km-remote-core`, which this crate cannot see; the page's job is to pass the string through
/// intact, and `link.rs` tests what becomes of it.
#[tokio::test]
async fn a_typed_address_reaches_the_link_unchanged() {
    let (harness, connect) = Harness::with_connect(StubConnect::new());

    let (status, body) = harness
        .post_form("/machine/connect", "address=192.168.1.5")
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(connect.typed(), vec!["192.168.1.5".to_owned()]);
    assert!(
        body.contains(r#"id="machine""#),
        "the answer is the re-rendered card, so the page updates itself"
    );
    assert!(
        body.contains("Now using 192.168.1.5."),
        "and a toast saying so"
    );
    assert!(
        body.contains("pinned"),
        "a typed address is kept, and the card says so"
    );
}

/// An empty box is a toast and no re-point — the same answer for a box somebody cleared and one they
/// never filled, which is what `form::field` folding empty into absent already gives.
#[tokio::test]
async fn an_empty_address_changes_nothing_and_says_so() {
    let (harness, connect) = Harness::with_connect(StubConnect::new());

    let (status, body) = harness.post_form("/machine/connect", "address=").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "never a 4xx — htmx does not swap one"
    );
    assert!(connect.typed().is_empty(), "nothing was pointed anywhere");
    assert!(body.contains("Enter an address first."));
    assert_eq!(
        connect.address().as_deref(),
        Some("http://192.168.1.9:8177"),
        "the machine in hand is untouched"
    );
}

/// A rescan that finds nothing is an ordinary answer, not a failure — and it must not lose the
/// address in hand.
#[tokio::test]
async fn a_rescan_that_finds_nothing_keeps_the_card_and_says_what_happened() {
    let (harness, connect) = Harness::with_connect(StubConnect {
        found: Vec::new(),
        ..StubConnect::new()
    });

    let (status, body) = harness.post_form("/machine/rescan", "").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("No machines answered on the network."));
    assert!(
        body.contains("http://192.168.1.9:8177"),
        "a fruitless browse leaves the remote where it was, and the card still says where"
    );
    assert_eq!(
        connect.address().as_deref(),
        Some("http://192.168.1.9:8177")
    );
}

/// A rescan while connected offers what it found and does not take it.
///
/// **The fault this is about is a working evening interrupted by a button pressed to look.** A
/// browse takes the first machine to resolve, which in a house with two has nothing to do with which
/// one the room is using — so the answer arrives as an offer in `#machine-found`, and the card still
/// names the machine this device has.
#[tokio::test]
async fn a_rescan_on_a_connected_remote_offers_rather_than_switches() {
    let (harness, connect) = Harness::with_connect(StubConnect::new());

    let (status, body) = harness.post_form("/machine/rescan", "").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        connect.address().as_deref(),
        Some("http://192.168.1.9:8177"),
        "the machine in hand is untouched by a look at the network"
    );
    assert!(body.contains("Still using the same machine."), "{body}");
    assert!(
        body.contains("One more available"),
        "the count of what else is out there is the fact the button used to drop: {body}"
    );
    assert!(
        body.contains(r#"id="machine-found""#) && body.contains(r#"hx-swap-oob="true""#),
        "the offer swaps itself, because it lives outside the card the pump republishes"
    );
    assert!(
        body.contains("http://192.168.1.42:8177") && body.contains("/machine/use"),
        "the offer names the address it found and carries the button that takes it"
    );
}

/// **The reported fault, at the page.**
///
/// Two machines answered and one of them is the one in hand. The old shape had one slot for an
/// offer and `Already` filled none of it, so this answered *"the machine you are already using is
/// the one on the network"* and gave no way at all to reach the second machine.
#[tokio::test]
async fn a_rescan_offers_the_other_machine_even_when_it_finds_the_one_in_hand() {
    let (harness, connect) = Harness::with_connect(StubConnect {
        found: vec![
            "http://192.168.1.9:8177".to_owned(),
            "http://192.168.1.42:8177".to_owned(),
        ],
        ..StubConnect::new()
    });

    let (status, body) = harness.post_form("/machine/rescan", "").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("is the one you are already using"), "{body}");
    assert!(body.contains("One more available"), "{body}");
    assert!(
        body.contains(r#"value="http://192.168.1.42:8177""#),
        "the other machine has to be pressable: {body}"
    );
    assert!(
        !body.contains(r#"value="http://192.168.1.9:8177""#),
        "and the one on the card above must not be offered back: {body}"
    );
    assert_eq!(
        connect.address().as_deref(),
        Some("http://192.168.1.9:8177"),
        "nothing moved"
    );
}

/// Every machine that answered gets its own button.
#[tokio::test]
async fn a_rescan_offers_a_button_for_every_other_machine() {
    let (harness, _connect) = Harness::with_connect(StubConnect {
        found: vec![
            "http://192.168.1.9:8177".to_owned(),
            "http://192.168.1.42:8177".to_owned(),
            "http://192.168.1.77:8177".to_owned(),
        ],
        ..StubConnect::new()
    });

    let (_, body) = harness.post_form("/machine/rescan", "").await;

    assert!(body.contains("2 more available"), "{body}");
    assert_eq!(
        body.matches(r#"hx-post="/machine/use""#).count(),
        2,
        "one form per machine offered: {body}"
    );
}

/// The same press on a remote whose machine is not answering still moves, as it always did.
///
/// This is the half that must not regress: there is nothing to disturb, so a browse that finds
/// something is unambiguously good news and waiting to be asked would be a worse remote.
#[tokio::test]
async fn a_rescan_with_nothing_answering_still_moves_by_itself() {
    let (harness, connect) = Harness::with_connect(StubConnect::new().offline());

    let (status, body) = harness.post_form("/machine/rescan", "").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        connect.address().as_deref(),
        Some("http://192.168.1.42:8177"),
        "with nothing to lose, a browse that answers is taken"
    );
    assert!(body.contains("Found"));
    assert!(
        connect.picked().is_empty(),
        "it moved on its own; nobody pressed the offer"
    );
}

/// The offer moves the machine and takes itself off the page.
///
/// The second half is the one worth asserting: an offer that outlived the press that acted on it
/// would sit there recommending a machine this device is already using.
#[tokio::test]
async fn accepting_an_offer_moves_the_machine_and_clears_the_offer() {
    let (harness, connect) = Harness::with_connect(StubConnect::new());

    let (status, body) = harness
        .post_form("/machine/use", "address=http%3A%2F%2F192.168.1.42%3A8177")
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        connect.picked(),
        vec!["http://192.168.1.42:8177".to_owned()],
        "the address arrives intact, decoded"
    );
    assert_eq!(
        connect.address().as_deref(),
        Some("http://192.168.1.42:8177")
    );
    assert!(
        !body.contains("/machine/use"),
        "the offer block comes back empty, so nothing is left to press twice"
    );
}

/// A browse that turns up the machine already in use says so instead of offering it.
#[tokio::test]
async fn a_rescan_that_finds_the_machine_in_hand_offers_nothing() {
    let (harness, connect) = Harness::with_connect(StubConnect {
        found: vec!["http://192.168.1.9:8177".to_owned()],
        ..StubConnect::new()
    });

    let (status, body) = harness.post_form("/machine/rescan", "").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("is the one you are already using"));
    assert!(
        !body.contains("/machine/use"),
        "offering the address printed on the card above reads as a fault in the page"
    );
    assert_eq!(
        connect.address().as_deref(),
        Some("http://192.168.1.9:8177")
    );
}

/// Two lines: the title with the number hard right, then the artist with the buttons hard right.
///
/// **The number may not go back onto the artist's line**, which is the fault this shape exists
/// because of: `.song-sub` is one `nowrap` line with an ellipsis, so an artist name of any length cut
/// off the one thing on the row somebody needs in order to dial the song at the machine's own keypad.
/// It spent a while on a third line of its own above the buttons, which fixed that and cost every row
/// in the list a line of height.
///
/// A stylesheet cannot be asserted from here; what can is which element each thing is inside.
#[tokio::test]
async fn a_row_is_two_lines_with_the_number_beside_the_title() {
    let harness = Harness::offline();

    let body = harness.get("/").await;

    // The number shares the title's line, and is a sibling of the name rather than inside it — so a
    // long title wraps and the number stays whole beside the first line of it.
    let head = body
        .split(r#"<div class="song-head">"#)
        .nth(1)
        .expect("a row has a head")
        .split("</div>")
        .next()
        .expect("a head ends");
    assert!(
        head.contains(r#"<span class="song-name">Tempo Perdido</span>"#),
        "{head}"
    );
    assert!(
        head.contains(r#"<span class="song-number">#1001</span>"#),
        "{head}"
    );

    // The sub line ends at the duration. Anything after it on that line is inside the ellipsis.
    for chunk in body.split(r#"<span class="song-sub">"#).skip(1) {
        let line = chunk.split("</span>").next().unwrap_or_default();
        assert!(
            !line.contains('#'),
            "the number is back inside the truncating line: {line}"
        );
    }

    // And nothing labels the media type. A `video` pill sat after the title and cost the row width
    // for something a singer does not choose a song by; the controls a video refuses are already
    // drawn grayed out on the player card, which is where it matters.
    assert!(!body.contains(r#"class="tag""#), "{body}");
}

/// *Open in browser* opens the **machine**, at the address the card is already printing.
///
/// The destination is the other remote, which is worth reaching from any host — a phone included.
/// Opening this remote's own home page would make the control meaningful only in a run that had a
/// window to escape from.
#[tokio::test]
async fn open_in_browser_goes_to_the_machines_own_remote() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new());
    let body = harness.get("/setup").await;

    assert!(
        body.contains(r#"<a href="http://192.168.1.9:8177" target="_blank""#),
        "the machine's address, not this remote's: {body}"
    );
    assert!(body.contains("Open in browser"), "{body}");
    assert!(
        !body.contains("/browser"),
        "there is no host seam left to post to: {body}"
    );
}

/// The song book is linked out to the machine's own route, not built from this device's mirror.
///
/// `The song book` in docs/decisions/interface.md said the offline remote does not link to a book,
/// on an argument about *serving* one: this device's catalog is `km-remote-core`'s mirror, with a
/// different schema and a different sort key, so a book built here would be a second adapter.
/// Linking out is not serving — it is the *Open in browser* anchor beside it, read a second time.
#[tokio::test]
async fn the_card_offers_the_machines_own_song_book() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new());
    let body = harness.get("/setup").await;

    assert!(
        body.contains(r#"href="http://192.168.1.9:8177/api/v1/songs/book.pdf""#),
        "the machine's route, not one of this remote's: {body}"
    );

    // And a device with no machine has nothing to link to, exactly as with the anchor beside it.
    let mut connect = StubConnect::new();
    connect
        .state
        .get_mut()
        .expect("the stub lock holds")
        .address = None;
    let (harness, _connect) = Harness::with_connect(connect);
    let body = harness.get("/setup").await;
    assert!(!body.contains("book.pdf"), "{body}");
}

/// `target="_blank"` is what makes it leave, so it is asserted rather than assumed.
///
/// Every webview this runs in routes a foreign URL out to the real browser off a *new window*
/// request — wry's `with_new_window_req_handler`, and the two phones' navigation policies. Dropping
/// the attribute would navigate the app's own window to the machine and strand somebody there.
#[tokio::test]
async fn the_link_out_asks_for_a_new_window_so_the_app_does_not_navigate_away() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new());
    let body = harness.get("/setup").await;
    let link = body
        .split("Open in browser")
        .next()
        .expect("the markup before the label");
    let anchor = link.rsplit("<a ").next().expect("the anchor");
    assert!(anchor.contains(r#"target="_blank""#), "{anchor}");
    assert!(anchor.contains(r#"rel="noopener""#), "{anchor}");
}

/// No machine, no address, nothing to open — absent rather than disabled, per `Two remotes`.
#[tokio::test]
async fn a_remote_that_has_found_no_machine_offers_nothing_to_open() {
    let mut connect = StubConnect::new();
    connect
        .state
        .get_mut()
        .expect("the stub lock holds")
        .address = None;
    let (harness, _connect) = Harness::with_connect(connect);

    let body = harness.get("/setup").await;
    assert!(body.contains("No machine selected"), "{body}");
    assert!(
        !body.contains("Open in browser"),
        "a link that could only ever go nowhere: {body}"
    );
}

/// A machine that is not answering keeps its link.
///
/// The banner prints that same address as *retrying*, so hiding it here would have the card
/// pretending the remote does not know where the machine is.
#[tokio::test]
async fn a_machine_that_is_not_answering_can_still_be_opened() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new().offline());
    let body = harness.get("/setup").await;
    assert!(body.contains(r#"href="http://192.168.1.9:8177""#), "{body}");
}

/// A build that cannot browse draws no Rescan button — absent, not disabled.
#[tokio::test]
async fn a_remote_that_cannot_browse_offers_no_rescan() {
    let (harness, _connect) = Harness::with_connect(StubConnect {
        can_browse: false,
        ..StubConnect::new()
    });
    let body = harness.get("/setup").await;

    assert!(
        !body.contains("/machine/rescan"),
        "a button that can only ever report failure is the grayed-out star Capabilities avoids"
    );
    assert!(
        body.contains("/machine/refresh"),
        "refreshing the song list is unaffected — it needs no locator"
    );
}

/// Refreshing the song list answers with the card and what it did.
#[tokio::test]
async fn refreshing_the_song_list_reports_what_it_copied() {
    let (harness, _connect) = Harness::with_connect(StubConnect::new());

    let (status, body) = harness.post_form("/machine/refresh", "").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Copied 1204 songs from the machine."));
    assert!(body.contains(r#"id="machine""#));
}

/// A build with no link at all answers rather than rendering an empty card.
///
/// Unreachable through the shipped shells — these routes are only useful where a `connect` is
/// installed — but a handler that assumed one would panic on the machine's own remote, which mounts
/// this same router.
#[tokio::test]
async fn the_machine_routes_answer_even_where_nothing_can_choose_a_machine() {
    let harness = Harness::online();
    for route in [
        "/machine/connect",
        "/machine/rescan",
        "/machine/use",
        "/machine/refresh",
    ] {
        let (status, body) = harness.post_form(route, "address=10.0.0.1").await;
        assert_eq!(status, StatusCode::OK, "{route} answers");
        assert!(
            body.contains("cannot change which machine it uses"),
            "{route} says why it did nothing"
        );
    }
}

// -- what language the page is in ----------------------------------------------------------------

/// A guest who has never used this remote gets their own language on the first load.
///
/// **The whole reason negotiation happens before any cookie exists.** A phone handed round at a
/// party has no history with the machine, and asking somebody to find a language picker in a
/// language they cannot read is asking them to give up.
#[tokio::test]
async fn a_browser_that_asks_for_portuguese_gets_a_portuguese_page() {
    let harness = Harness::offline();
    let body = harness.get_in("/setup", "pt-BR,pt;q=0.9,en;q=0.8").await;
    assert!(body.contains(r#"<html lang="pt-BR">"#), "{body}");
    assert!(body.contains("Músicas"), "the tab bar: {body}");
    assert!(body.contains("Cantando como"), "the singer field: {body}");
    assert!(!body.contains(">Songs<"), "English survived: {body}");
}

/// **A whole page in Portuguese has no English left in it, and no keys either.**
///
/// The reader-facing complement to `no_fragment_the_fan_out_pushes_is_ever_missing_its_words`. That
/// one checks the fan-out; this one checks the three documents a singer actually loads, and it is
/// the check that would have caught the quiet half of the reported fault — sentences composed in
/// Rust, which came out in English rather than in brackets and so looked deliberate.
///
/// The English list is short on purpose: every entry is a string that used to survive a Portuguese
/// render, and each is distinctive enough that a Portuguese page cannot contain it innocently. It is
/// not a substitute for reading the page, and it is not meant to grow into one.
#[tokio::test]
async fn a_portuguese_page_has_no_english_and_no_untranslated_keys() {
    // With a `connect`, so the machine card — which is where the report's own screenshot had four
    // English words on it — is really on the page rather than absent and vacuously passing.
    let (harness, _connect) = Harness::with_connect(StubConnect::new());
    for path in ["/", "/now", "/queue", "/setup"] {
        let body = harness.get_in(path, "pt-BR").await;
        assert!(
            !body.contains('\u{27E6}'),
            "{path} shows a message id rather than a message: {body}"
        );
        for english in [
            ">Songs<",
            "Song or number",
            "Nothing playing",
            "Up next",
            "shown<",
            "Rescan",
            "Refresh song list",
            "songs in this copy",
            "Connected",
            "is not reachable",
            "No songs",
            "Nothing matches",
        ] {
            assert!(
                !body.contains(english),
                "{path} still says `{english}`: {body}"
            );
        }
    }

    // And the pages really do hold the things being checked for, so the loop above is not passing
    // because it rendered nothing.
    let setup = harness.get_in("/setup", "pt-BR").await;
    assert!(setup.contains("Conectado"), "the machine card: {setup}");
    assert!(
        setup.contains("lembrada"),
        "how the machine was arrived at: {setup}"
    );
    assert!(
        setup.contains("músicas nesta cópia"),
        "the copy count: {setup}"
    );
    let browse = harness.get_in("/", "pt-BR").await;
    assert!(
        browse.contains("Música ou número"),
        "the search box: {browse}"
    );
    assert!(browse.contains("mostrando"), "the list count: {browse}");
}

/// European Portuguese is served far better by Brazilian Portuguese than by English.
#[tokio::test]
async fn a_region_this_build_does_not_have_falls_back_to_the_language() {
    let harness = Harness::offline();
    let body = harness.get_in("/queue", "pt-PT").await;
    assert!(body.contains(r#"<html lang="pt-BR">"#), "{body}");
}

/// A language nothing here speaks gets English rather than a blank or a refusal.
#[tokio::test]
async fn a_language_this_build_does_not_have_gets_the_source_language() {
    let harness = Harness::offline();
    let body = harness.get_in("/setup", "de,fr;q=0.8").await;
    assert!(body.contains(r#"<html lang="en">"#), "{body}");
    assert!(body.contains("Singing as"), "{body}");
}

/// The cookie wins over the header: one is a choice made on this device, the other is what the
/// browser happened to be installed with.
#[tokio::test]
async fn a_chosen_language_outlives_what_the_browser_asks_for() {
    let harness = Harness::offline();
    let (_, _, body) = harness.get_full("/queue", Some("km_locale=pt-BR")).await;
    assert!(body.contains(r#"<html lang="pt-BR">"#), "{body}");
}

/// The picker names each language in itself, which is the one label that must not be translated.
#[tokio::test]
async fn the_language_picker_names_each_language_in_its_own_words() {
    let harness = Harness::offline();
    let body = harness.get_in("/setup", "pt-BR").await;
    assert!(body.contains("Português (Brasil)"), "{body}");
    assert!(body.contains(">English<"), "{body}");
}

/// A catalog nobody has tagged draws no tag control at all.
///
/// Which is most catalogs, because nothing detects a tag — so this is the ordinary case rather than
/// an edge one, and a picker with nothing in it would be a permanent empty row on every phone.
#[tokio::test]
async fn an_untagged_catalog_draws_no_tag_picker() {
    let harness = Harness::offline();
    let body = harness.get("/").await;
    assert!(!body.contains(r#"id="add-tag""#), "{body}");
    assert!(!body.contains("filter-chips"), "{body}");
    // The hidden field is there regardless: it is how a tag survives a search inside an artist.
    assert!(body.contains(r#"id="tags" type="hidden""#), "{body}");
}

/// The picker offers what is not chosen, the chips show what is, and each ✕ drops exactly one.
#[tokio::test]
async fn the_tag_picker_offers_what_is_not_chosen_and_a_chip_drops_exactly_one() {
    let harness = Harness::with_tags();

    let bare = harness.get("/").await;
    assert!(bare.contains(r#"id="add-tag""#), "{bare}");
    assert!(
        bare.contains(">rock (2)<"),
        "commonest first, with counts: {bare}"
    );
    assert!(bare.contains(">brasil (1)<"), "{bare}");
    assert!(!bare.contains("filter-chips"), "nothing chosen yet: {bare}");

    let one = harness.get("/?tags=rock").await;
    assert!(one.contains("filter-chips"), "{one}");
    assert!(
        one.contains(r#"value="rock""#),
        "the set rides as a field: {one}"
    );
    assert!(
        !one.contains(">rock (2)<"),
        "a chosen tag must not be offered again — adding it twice changes nothing: {one}"
    );
    assert!(one.contains(">brasil (1)<"), "{one}");

    // Two chosen: the picker has nothing left to add and disappears; each ✕ drops one of the two.
    let both = harness.get("/?tags=brasil,rock").await;
    assert!(!both.contains(r#"id="add-tag""#), "{both}");
    assert!(both.contains("tags=rock&#38;fragment=browse"), "{both}");
    assert!(both.contains("tags=brasil&#38;fragment=browse"), "{both}");
    // And the last chip's ✕ leaves no `tags=` at all rather than an empty one.
    let last = harness.get("/?tags=rock").await;
    assert!(
        last.contains(r#"hx-get="/?mode=songs&#38;fragment=browse""#),
        "{last}"
    );
}

/// The strip says what is on, offers the way out of all of it, and sits last in the browse bar.
///
/// **The clear-all is the fix, and the reason it was needed is what this pins.** Nothing else could
/// do it: the picker's first option is a placeholder that `BrowseParams::tags` never merges, and the
/// search box's ✕ deliberately keeps the tags because it clears a *search*. Four tags on meant four
/// presses, with the one control that looked like it might do it doing nothing.
///
/// **The position is asserted because it moved once and must not drift back.** It was tried above
/// the search box, on the argument that what the list has been narrowed to is the first thing to
/// know about it — and the strip's height varies with the number of tags, so up there it shoved the
/// search box down the page on every add and every remove. On a phone that box is the most used
/// control there is. Below the pickers it grows into a list that is already scrolling and moves
/// nothing.
#[tokio::test]
async fn the_chip_strip_is_last_in_the_bar_and_offers_a_way_out_of_every_tag() {
    let harness = Harness::with_tags();

    let both = harness.get("/?tags=brasil,rock").await;
    let chips = both.find("filter-chips").expect("the strip is drawn");
    let search = both
        .find(r#"type="search""#)
        .expect("the search box is drawn");
    let list = both.find(r#"id="list""#).expect("the list is drawn");
    assert!(
        search < chips,
        "a strip that varies in height must not sit above the search box: {both}"
    );
    assert!(
        chips < list,
        "the strip goes against the list it describes: {both}"
    );

    // The picker's own place in that order, checked with one tag on rather than two — this fixture
    // has exactly two tags, so choosing both leaves the picker nothing to offer and it is not drawn.
    let one = harness.get("/?tags=rock").await;
    let picker = one.find(r#"id="add-tag""#).expect("the picker is drawn");
    let chips = one.find("filter-chips").expect("the strip is drawn");
    assert!(
        picker < chips,
        "the strip goes under the pickers, not among them: {one}"
    );

    // Clear all: this mode, and no `tags=` at all rather than an empty one.
    assert!(both.contains("Clear all"), "{both}");
    assert!(both.contains("chip-clear"), "{both}");
    assert!(
        both.contains(r#"hx-get="/?mode=songs&#38;fragment=browse""#),
        "clear all asks for this mode with no tags at all, not with an empty one: {both}"
    );

    // Offered with one tag too. It duplicates that chip's ✕, and that is the point: a control which
    // only appears once somebody is already lost teaches nobody it is there.
    let one = harness.get("/?tags=rock").await;
    assert!(one.contains("chip-clear"), "{one}");

    // And never when there is nothing to clear.
    let bare = harness.get("/").await;
    assert!(!bare.contains("chip-clear"), "{bare}");
}

/// Adding a tag asks for the whole browse bar, because it changes the strip and not only the rows.
///
/// **Every test here rendered the right markup while the feature did not work in a browser**, which
/// is the reason this one asserts an attribute rather than a page. The picker rode the enclosing
/// form, whose `hx-get` targets `#list` — the rows and nothing else — so the server was asked for,
/// and correctly returned, a fragment that could not contain the strip. Three symptoms, one cause:
/// no chip appeared, the picker went on offering the tag it had just applied, and the hidden `tags`
/// field kept the previous set so a second choice **replaced** the first.
///
/// The rule this restores is the one `tag_remove_href` already states: a control that only refilters
/// the list swaps `#list`, and one that changes the bar swaps `#browse`. Removing a tag had it and
/// adding one did not.
#[tokio::test]
async fn the_tag_picker_swaps_the_browse_bar_and_not_just_the_rows() {
    let harness = Harness::with_tags();
    let body = harness.get("/").await;

    // The picker carries its own request…
    assert!(
        body.contains(r##"hx-target="#browse""##),
        "the picker must ask for the bar it changes: {body}"
    );
    let picker = &body[body.find(r#"id="add-tag""#).expect("the picker is drawn")..];
    let picker = &picker[..picker.find("</select>").expect("a closed picker")];
    for attribute in [
        r#"hx-get="/?mode=songs&#38;fragment=browse""#,
        r##"hx-target="#browse""##,
        r#"hx-swap="outerHTML""#,
        r##"hx-include="#language, #initial""##,
    ] {
        assert!(
            picker.contains(attribute),
            "the picker is missing {attribute}: {picker}"
        );
    }

    // …and is no longer one of the form's triggers, or it would fire twice and one of the two would
    // swap the wrong element.
    assert!(
        !body.contains("change from:#add-tag"),
        "the picker is still on the form's trigger list: {body}"
    );
    // The three that genuinely only narrow are still on it.
    assert!(
        body.contains("change from:#language, change from:#initial"),
        "{body}"
    );
}

/// A tag chosen while another is already on **adds**; it does not replace.
///
/// This is what the picker looked like it was doing, and the reason it looked that way is that the
/// bar was never re-rendered, so the hidden `tags` field the form sent was always the set from
/// before. `tag_add_href` bakes the held tags into the picker's own URL instead, so the request
/// carries them however stale the field is.
#[tokio::test]
async fn choosing_a_second_tag_adds_it_to_the_first() {
    let harness = Harness::with_tags();

    // With `rock` on, the picker's URL already carries it — so htmx appending `add_tag=brasil`
    // asks for both rather than for `brasil` alone.
    let one = harness.get("/?tags=rock").await;
    let picker = &one[one.find(r#"id="add-tag""#).expect("the picker is drawn")..];
    assert!(
        picker.contains(r#"hx-get="/?mode=songs&#38;tags=rock&#38;fragment=browse""#),
        "the tags already held must be in the picker's URL: {picker}"
    );

    // And the merge is real: the list holds the rows filed under either word. A picker that
    // replaced would answer with `brasil` alone, which is `Both` and nothing else.
    let either = harness.get("/?tags=rock&add_tag=brasil").await;
    assert!(either.contains("Both"), "{either}");
    assert!(
        either.contains("Rock only"),
        "adding brasil to rock must keep rock's rows, not replace them: {either}"
    );
}

/// Clearing every tag keeps the search and the list it was narrowing.
///
/// It clears *tags*, exactly as the box's ✕ clears a *search* and keeps the tags. Getting this
/// backwards would make the strip's way out a way out of the whole page.
#[tokio::test]
async fn clearing_the_tags_keeps_the_search_and_the_list_you_are_in() {
    let harness = Harness::with_tags();

    let searching = harness.get("/?tags=rock&q=both").await;
    assert!(
        searching.contains(r#"hx-get="/?mode=songs&#38;q=both&#38;fragment=browse""#),
        "the words survive the clear: {searching}"
    );
}

/// Picking a tag narrows the rows to the ones filed under it, and two give the union.
#[tokio::test]
async fn the_tag_filter_narrows_the_list_and_two_tags_unite() {
    let harness = Harness::with_tags();

    let all = harness.get("/").await;
    assert!(all.contains("Both") && all.contains("Rock only") && all.contains("Neither"));

    let rock = harness.get("/?tags=rock").await;
    assert!(
        rock.contains("Both") && rock.contains("Rock only"),
        "{rock}"
    );
    assert!(!rock.contains(">Neither<"), "{rock}");

    let either = harness.get("/?tags=rock,brasil").await;
    assert!(either.contains("Both"), "{either}");
    assert!(either.contains("Rock only"), "OR, not AND: {either}");
    // A filter is still a filter: the untagged song stays out of a list two tags asked for.
    assert!(!either.contains(">Neither<"), "{either}");
}

/// The picker adds through `add_tag`, and the answer comes back with it merged and the picker clear.
///
/// Two names for one control, and the reason is a 400: the chosen set rides the same form as a
/// hidden `tags` field, so a picker also called `tags` would put the key in twice and
/// `serde_urlencoded` refuses that — silently, because htmx does not swap on an error.
#[tokio::test]
async fn adding_a_tag_merges_it_into_the_set_and_clears_the_picker() {
    let harness = Harness::with_tags();
    let body = harness.get("/?tags=brasil&add_tag=rock").await;
    assert!(
        body.contains(r#"name="tags" value="brasil,rock""#),
        "the added tag joins the set, sorted: {body}"
    );
    assert!(
        !body.contains(r#"id="add-tag""#),
        "both tags are chosen now, so there is nothing left to offer: {body}"
    );
    assert!(
        body.contains(">Both<"),
        "and the rows are narrowed by both: {body}"
    );
}

// -- sharing a folder ----------------------------------------------------------------------------

/// The code for a folder holding these songs, built the way the sending page builds it.
fn code_for(name: &str, songs: &[u32]) -> String {
    km_remote_pages::share::encode(&km_remote_pages::share::Folder {
        name: name.to_owned(),
        songs: songs.iter().copied().map(SongCode::new).collect(),
    })
    .expect("encode")
}

/// **Sharing is reachable only from inside a folder**, which is the one constraint that deletes both
/// "which folder?" questions from the flow. Asserted from the markup rather than the route table,
/// because a route nothing links to is a route nobody can reach.
#[tokio::test]
async fn sharing_is_offered_only_from_inside_a_folder() {
    let harness = Harness::offline();

    let list = harness.get("/?mode=favorites").await;
    assert!(
        !list.contains("/favorites/share/"),
        "the folder list offers no share link: {list}"
    );

    let inside = harness.get("/?mode=favorites&folder=1").await;
    assert!(
        inside.contains("/favorites/share/1"),
        "inside a folder it does: {inside}"
    );

    // An artist shares the same `page-head` and has nothing to share.
    let artist = harness
        .get("/?mode=artists&artist=Legi%C3%A3o+Urbana")
        .await;
    assert!(
        !artist.contains("/favorites/share/"),
        "an artist is not a folder: {artist}"
    );
}

#[tokio::test]
async fn the_chooser_offers_both_directions_for_the_folder_it_is_in() {
    let harness = Harness::offline();
    let body = harness.get("/favorites/share/1").await;
    assert!(body.contains("/favorites/share/1/send"), "{body}");
    assert!(body.contains("/favorites/share/1/receive"), "{body}");
    // The folder is named, so neither side has to ask which.
    assert!(body.contains("Favorites"), "{body}");
}

/// A real SVG, and never cached — the folder changes as songs are filed and the URL does not.
#[tokio::test]
async fn a_folders_code_is_served_as_one_image_that_is_never_cached() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001), SongCode::new(2005)]);

    let (status, headers, body) = harness.get_headers("/favorites/share/1/code.svg").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "image/svg+xml; charset=utf-8"
    );
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "no-store"
    );
    assert!(body.contains("<svg"), "{body}");
    // A vector, so there is no pixel scale to get wrong: the viewBox is the module count.
    assert!(body.contains("viewBox"), "{body}");
}

/// An `<img src>` has no page to word a refusal into, so it refuses with a status and a log line.
#[tokio::test]
async fn the_image_route_refuses_a_folder_it_cannot_draw() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let (status, _, _) = harness.get_headers("/favorites/share/1/code.svg").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty folder has no code to draw"
    );
}

/// The sending page must not link an image the handler will refuse — that is a broken icon and no
/// reason. It says so and opens the text box instead.
#[tokio::test]
async fn a_folder_with_nothing_in_it_says_so_rather_than_linking_a_broken_image() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let body = harness.get("/favorites/share/1/send").await;
    assert!(
        !body.contains("code.svg"),
        "no image is linked when there is no code: {body}"
    );
    assert!(
        body.contains("no songs in this folder"),
        "and it says why: {body}"
    );
}

#[tokio::test]
async fn the_sending_page_shows_the_code_and_the_text_behind_a_disclosure() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001), SongCode::new(2005)]);

    let body = harness.get("/favorites/share/1/send").await;
    assert!(body.contains("/favorites/share/1/code.svg"), "{body}");
    assert!(body.contains(&code_for("Rock", &[1001, 2005])), "{body}");
    assert!(body.contains("scan it?"), "{body}");
}

/// **The code is never drawn on the receiving side.** Showing it was most of what made this look
/// like a tool rather than a task, so it rides in a hidden field and nowhere else.
#[tokio::test]
async fn a_scanned_code_is_never_drawn_on_the_page() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let code = code_for("Party", &[1001]);

    let (status, body) = harness
        .post_owned("/favorites/share/1/receive", format!("code={code}"))
        .await;
    assert_eq!(status, StatusCode::OK);

    let hidden = format!(r#"<input type="hidden" name="code" value="{code}">"#);
    assert!(body.contains(&hidden), "it rides in a hidden field: {body}");
    assert_eq!(
        body.matches(code.as_str()).count(),
        1,
        "and appears nowhere else: {body}"
    );
}

/// Not an error — merging one folder into another is a fair thing to want — but names not matching
/// is also what a mis-scan looks like, so it is said out loud before anything is written.
#[tokio::test]
async fn the_confirm_screen_names_the_folder_the_code_came_from() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);

    let (_, mismatched) = harness
        .post_owned(
            "/favorites/share/1/receive",
            format!("code={}", code_for("Party", &[1001])),
        )
        .await;
    assert!(
        mismatched.contains("came from") && mismatched.contains("Party"),
        "{mismatched}"
    );

    let (_, agreeing) = harness
        .post_owned(
            "/favorites/share/1/receive",
            format!("code={}", code_for("Rock", &[1001])),
        )
        .await;
    assert!(
        !agreeing.contains("came from"),
        "nothing to say when the names agree: {agreeing}"
    );
}

/// The confirm screen chooses nothing: the folder is settled and the only control adds.
#[tokio::test]
async fn the_confirm_screen_chooses_nothing() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let (_, body) = harness
        .post_owned(
            "/favorites/share/1/receive",
            format!("code={}", code_for("Party", &[1001])),
        )
        .await;
    assert!(body.contains("/favorites/share/1/merge"), "{body}");
    assert!(
        !body.contains("<select"),
        "there is no folder to pick: {body}"
    );
}

#[tokio::test]
async fn a_code_that_will_not_decode_comes_back_to_the_camera_with_a_sentence() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let (status, body) = harness
        .post_form("/favorites/share/1/receive", "code=nonsense")
        .await;
    assert_eq!(status, StatusCode::OK, "not a status htmx would refuse");
    assert!(body.contains("not a favorites code"), "{body}");
    assert!(
        body.contains("scan-form"),
        "and the camera is still there: {body}"
    );
}

#[tokio::test]
async fn a_merge_adds_and_says_what_it_added() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);

    let (status, body) = harness
        .post_owned(
            "/favorites/share/1/merge",
            format!("code={}", code_for("Party", &[1001, 1002, 1003])),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("2 songs added"), "{body}");
    assert!(body.contains("1 already here"), "{body}");
    assert_eq!(
        favorites.songs_in(1),
        vec![
            SongCode::new(1001),
            SongCode::new(1002),
            SongCode::new(1003)
        ]
    );
}

/// The guarantee both features rest on, observed through the router.
#[tokio::test]
async fn a_merge_twice_adds_nothing_the_second_time() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    let body = format!("code={}", code_for("Party", &[1001, 1002]));

    harness
        .post_owned("/favorites/share/1/merge", body.clone())
        .await;
    let (_, second) = harness.post_owned("/favorites/share/1/merge", body).await;
    assert!(second.contains("0 songs added"), "{second}");
    assert!(second.contains("2 already here"), "{second}");
    assert_eq!(favorites.songs_in(1).len(), 2);
}

/// A merge cannot take anything away, whatever the code says.
#[tokio::test]
async fn a_merge_takes_nothing_away() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1004)]);

    harness
        .post_owned(
            "/favorites/share/1/merge",
            format!("code={}", code_for("Party", &[1001])),
        )
        .await;
    assert!(
        favorites.songs_in(1).contains(&SongCode::new(1004)),
        "a song the code did not mention has to survive"
    );
}

/// Dropped **before** the write, not stored and hidden: a folder's count comes from the collection
/// while its listing is filtered through the catalog, so an unshowable code would make the folder
/// claim more songs than it lists.
#[tokio::test]
async fn a_merge_drops_songs_this_device_cannot_show_and_counts_them() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);

    let (_, body) = harness
        .post_owned(
            "/favorites/share/1/merge",
            // 1001 is in the stub catalog; 9001 is not.
            format!("code={}", code_for("Party", &[1001, 9001])),
        )
        .await;
    assert!(body.contains("1 song added"), "{body}");
    assert!(body.contains("left out"), "and says one was: {body}");
    assert_eq!(favorites.songs_in(1), vec![SongCode::new(1001)]);
}

/// **The case that would lose somebody their whole collection.** A phone that has never reached a
/// machine has an empty mirror, which is this app's normal starting state — filtering against it
/// would discard everything and report the loss as a count.
#[tokio::test]
async fn a_merge_onto_a_device_with_no_catalog_keeps_everything() {
    let favorites = Arc::new(StubFavorites::with_folders(&[(1, "Rock")]));
    let mut harness = Harness::with(StubMachine::new(), Capabilities::offline());
    harness.remote.songs = Arc::new(StubSongs::default()); // nothing copied yet
    harness.remote.favorites = Some(Arc::clone(&favorites) as Arc<dyn Favorites>);

    let (_, body) = harness
        .post_owned(
            "/favorites/share/1/merge",
            format!("code={}", code_for("Party", &[1001, 9001])),
        )
        .await;
    assert!(body.contains("2 songs added"), "{body}");
    assert!(
        !body.contains("left out"),
        "nothing is left out when there is no catalog to check against: {body}"
    );
    assert_eq!(favorites.songs_in(1).len(), 2);
}

/// jsQR has to be parsed before `scan.js` runs, or the scanner finds no `window.jsQR` and hides its
/// own button — which looks exactly like a device with no camera.
#[tokio::test]
async fn the_receive_screen_loads_the_decoder_before_the_scanner() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let body = harness.get("/favorites/share/1/receive").await;
    let decoder = body.find("jsqr.js").expect("the decoder is loaded");
    let scanner = body.find("scan.js").expect("the scanner is loaded");
    assert!(decoder < scanner, "the decoder must come first: {body}");
}

/// **The scanner's sentences come off the page.** A static file cannot go through `|t`, so English
/// inside `scan.js` would show on a Portuguese page and no test in this crate would see it.
#[tokio::test]
async fn the_scanner_takes_its_sentences_from_the_page_rather_than_the_script() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let body = harness.get("/favorites/share/1/receive").await;
    for attribute in [
        "data-starting=",
        "data-got-it=",
        "data-refused=",
        "data-none=",
        "data-failed=",
    ] {
        assert!(body.contains(attribute), "{attribute} is missing: {body}");
    }

    let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/static/scan.js"))
        .expect("read scan.js");
    for english in [
        "Starting the camera",
        "Code read",
        "was not allowed",
        "No camera was found",
    ] {
        assert!(
            !script.contains(english),
            "`{english}` is prose inside the script, where no catalog can reach it"
        );
    }
}

/// Both flows run several screens deep, so every screen carries the way out.
///
/// **The ✕ is the same on both flows and the ‹ is not.** Leaving means being finished with the
/// favorites, which is one place whichever flow you were in; backing out means the screen you came
/// from, and the backup flow is now entered from the Setup tab — see the test below.
#[tokio::test]
async fn every_share_and_backup_screen_leaves_straight_to_the_folder_list() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let code = code_for("Party", &[1001]);

    let mut screens = vec![
        harness.get("/favorites/share/1").await,
        harness.get("/favorites/share/1/send").await,
        harness.get("/favorites/share/1/receive").await,
        harness.get("/favorites/backup").await,
        harness.get("/favorites/backup/restore").await,
    ];
    screens.push(
        harness
            .post_owned("/favorites/share/1/receive", format!("code={code}"))
            .await
            .1,
    );
    screens.push(
        harness
            .post_owned("/favorites/share/1/merge", format!("code={code}"))
            .await
            .1,
    );

    for screen in &screens {
        assert!(
            screen.contains(r#"href="/?mode=favorites""#),
            "a screen with no way out: {screen}"
        );
    }
}

/// The backup flow backs out to the tab it is reached from, and sharing to the folder it is about.
///
/// A step-back that goes somewhere you were never is worse than none at all. The two flows are
/// entered from different places now, so this is the one thing about them that is no longer common.
#[tokio::test]
async fn the_backup_flow_backs_out_to_setup_and_sharing_to_the_folder() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);

    let backup = harness.get("/favorites/backup").await;
    assert!(
        backup.contains(r#"<a class="icon-btn" href="/setup""#),
        "the backup was reached from Setup: {backup}"
    );

    let share = harness.get("/favorites/share/1").await;
    assert!(
        share.contains(r#"<a class="icon-btn" href="/?mode=favorites&#38;folder=1""#),
        "sharing was reached from inside the folder: {share}"
    );
}

/// A folder deleted in another tab is not a fault worth a page of its own.
#[tokio::test]
async fn a_stale_folder_goes_back_to_the_folder_list_rather_than_erroring() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let (status, headers, _) = harness.get_headers("/favorites/share/99").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers
            .get("location")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "/?mode=favorites"
    );
}

/// Neither feature is a thing the machine's own remote is withholding — it keeps no collection at
/// all, so there is nothing to share and nothing to save.
#[tokio::test]
async fn neither_feature_exists_on_the_machines_own_remote() {
    let harness = Harness::online();
    for path in [
        "/favorites/share/1",
        "/favorites/share/1/send",
        "/favorites/share/1/receive",
        "/favorites/backup",
        "/favorites/backup.json",
        "/favorites/backup/restore",
    ] {
        let body = harness.get(path).await;
        assert!(
            body.contains("does not keep favorites"),
            "{path} answered as if it did: {body}"
        );
    }

    let list = harness.get("/?mode=songs").await;
    assert!(
        !list.contains("/favorites/backup") && !list.contains("/favorites/share/"),
        "and neither control is drawn: {list}"
    );
}

// -- carrying the collection to a file -----------------------------------------------------------

/// **A backup is offered on the Setup tab and sharing only inside a folder.** One rule read from
/// both ends: a screen never asks a question its own path already answered, and it never offers a
/// control whose question its path has already answered *differently*.
///
/// Sharing is inside a folder because being in one is what answers *which folder?*. A backup is the
/// whole collection, and every screen under Songs has already narrowed which part of it you mean —
/// so it is offered where nothing has, which is Setup.
#[tokio::test]
async fn a_backup_is_offered_on_the_setup_tab_and_nowhere_under_songs() {
    let harness = Harness::offline();

    let setup = harness.get("/setup").await;
    assert!(setup.contains("/favorites/backup"), "{setup}");

    for path in [
        "/?mode=favorites",
        "/?mode=favorites&folder=1",
        "/?mode=songs",
    ] {
        let body = harness.get(path).await;
        assert!(
            !body.contains("/favorites/backup"),
            "{path} narrows the collection, so it must not offer a backup of all of it: {body}"
        );
    }
}

/// The `Content-Disposition` is what both phone shells key off to turn this into a file, and what
/// makes an ordinary browser save rather than render.
#[tokio::test]
async fn the_export_is_an_attachment_named_for_the_day() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);

    let (status, headers, body) = harness.get_headers("/favorites/backup.json").await;
    assert_eq!(status, StatusCode::OK);
    let disposition = headers
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(disposition.starts_with("attachment;"), "{disposition}");
    assert!(disposition.contains("km-favorites-"), "{disposition}");
    assert!(disposition.ends_with(".json\""), "{disposition}");
    assert!(
        disposition.is_ascii(),
        "no RFC 5987 encoding needed: {disposition}"
    );
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "no-store"
    );
    assert!(body.contains("km-remote-favorites"), "{body}");
}

/// The titles are what make a backup worth opening a year later.
#[tokio::test]
async fn the_export_carries_every_folder_and_the_titles_in_it() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock"), (2, "Festa")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    favorites.fill(2, &[SongCode::new(1002)]);

    let body = harness.get("/favorites/backup.json").await;
    assert!(body.contains("\"Rock\""), "{body}");
    assert!(body.contains("\"Festa\""), "{body}");
    assert!(body.contains("Tempo Perdido"), "{body}");
    assert!(body.contains("Legi"), "and the artist: {body}");
}

/// There is nothing in an empty folder to carry, and a page promising four folders that writes
/// three is worse than one that says three.
#[tokio::test]
async fn an_empty_folder_is_in_neither_the_file_nor_the_count() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock"), (2, "Empty")]);
    favorites.fill(1, &[SongCode::new(1001)]);

    let file = harness.get("/favorites/backup.json").await;
    assert!(!file.contains("\"Empty\""), "{file}");

    let page = harness.get("/favorites/backup").await;
    assert!(page.contains("1 folder"), "counted over the file: {page}");
    assert!(!page.contains("2 folders"), "{page}");
}

#[tokio::test]
async fn a_collection_with_nothing_in_it_offers_nothing_to_save() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let body = harness.get("/favorites/backup").await;
    assert!(body.contains("nothing to save"), "{body}");
    assert!(
        !body.contains("/favorites/backup.json"),
        "and does not link a file with nothing in it: {body}"
    );
}

/// An htmx swap would read the response as markup and put the JSON in the page.
#[tokio::test]
async fn the_export_link_is_not_an_htmx_swap() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);

    let body = harness.get("/favorites/backup").await;
    let at = body
        .find("/favorites/backup.json")
        .expect("the link is there");
    let anchor = &body[at.saturating_sub(120)..at];
    assert!(!anchor.contains("hx-get"), "{anchor}");
    assert!(
        !anchor.contains("download"),
        "the header carries the name; WebKit ignores the attribute anyway: {anchor}"
    );
}

/// The whole point of the feature: a collection comes back onto another device.
#[tokio::test]
async fn a_restore_creates_the_folders_it_does_not_find() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let file = harness.get("/favorites/backup.json").await;

    let (fresh, into) = Harness::with_folders(&[(1, "Somewhere else")]);
    let (status, body) = fresh
        .post_owned(
            "/favorites/backup/restore",
            format!("document={}", km_remote_pages::prefs::encode(&file)),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("1 song added"), "{body}");
    assert!(body.contains("1 folder created"), "{body}");
    assert_eq!(
        into.names(),
        vec!["Rock".to_owned(), "Somewhere else".to_owned()]
    );
}

#[tokio::test]
async fn a_restore_twice_adds_nothing_the_second_time() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001), SongCode::new(1002)]);
    let file = harness.get("/favorites/backup.json").await;
    let body = format!("document={}", km_remote_pages::prefs::encode(&file));

    let (fresh, into) = Harness::with_folders(&[(1, "Other")]);
    fresh
        .post_owned("/favorites/backup/restore", body.clone())
        .await;
    let (_, second) = fresh.post_owned("/favorites/backup/restore", body).await;
    assert!(second.contains("0 songs added"), "{second}");
    assert!(second.contains("2 already here"), "{second}");
    assert_eq!(into.songs_in(2).len(), 2, "and the folder is not doubled");
}

#[tokio::test]
async fn a_restore_takes_nothing_away() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let file = harness.get("/favorites/backup.json").await;

    let (fresh, into) = Harness::with_folders(&[(1, "Rock")]);
    into.fill(1, &[SongCode::new(1004)]);
    fresh
        .post_owned(
            "/favorites/backup/restore",
            format!("document={}", km_remote_pages::prefs::encode(&file)),
        )
        .await;
    assert_eq!(
        into.songs_in(1),
        vec![SongCode::new(1001), SongCode::new(1004)],
        "what was already filed survives"
    );
}

/// The same guard the merge has, and for the same reason.
#[tokio::test]
async fn a_restore_onto_a_device_with_no_catalog_keeps_everything() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001), SongCode::new(1002)]);
    let file = harness.get("/favorites/backup.json").await;

    let into = Arc::new(StubFavorites::with_folders(&[(1, "Other")]));
    let mut fresh = Harness::with(StubMachine::new(), Capabilities::offline());
    fresh.remote.songs = Arc::new(StubSongs::default());
    fresh.remote.favorites = Some(Arc::clone(&into) as Arc<dyn Favorites>);

    let (_, body) = fresh
        .post_owned(
            "/favorites/backup/restore",
            format!("document={}", km_remote_pages::prefs::encode(&file)),
        )
        .await;
    assert!(body.contains("2 songs added"), "{body}");
    assert!(!body.contains("left out"), "{body}");
}

#[tokio::test]
async fn a_file_this_build_cannot_read_says_so_above_the_picker() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);

    for (body, expected) in [
        ("document=not+json+at+all", "could not be read"),
        (
            "document=%7B%22folders%22%3A%5B%5D%7D",
            "not a favorites file",
        ),
        ("document=", "No file was chosen"),
    ] {
        let (status, page) = harness
            .post_owned("/favorites/backup/restore", body.to_owned())
            .await;
        assert_eq!(status, StatusCode::OK, "not a status htmx would refuse");
        assert!(page.contains(expected), "{body} did not say so: {page}");
        assert!(
            page.contains("pick-file"),
            "and the picker is still there: {page}"
        );
    }
}

/// A song in a named package, under a named hash — for the re-banking tests below.
fn song_in(number: u32, package: &str, hash: &str, title: &str) -> SongDto {
    SongDto {
        package_id: package.to_owned(),
        content_hash: Some(hash.to_owned()),
        ..song(number, title, Some("Legião Urbana"))
    }
}

/// **The bug this whole thing exists for.** A favorite is filed at 1001; the owner re-banks `vol1`
/// so its song is 2001 now, and `vol2` moves into the bank `vol1` left, so 1001 is a real number
/// belonging to somebody else's song. Resolving by code alone would draw the wrong song and say
/// nothing about it.
#[tokio::test]
async fn a_favorite_survives_its_package_being_re_banked() {
    let (before, favorites) = Harness::with_folders(&[(1, "Rock")]);
    // Filed while `vol1` was in bank 1, and drawn once so the identity is recorded.
    favorites.fill(1, &[SongCode::new(1001)]);
    let listed = before.get("/?mode=favorites&folder=1").await;
    assert!(listed.contains("Tempo Perdido"), "{listed}");

    // The mirror refreshes and everything has moved.
    let after = Harness::with_catalog(
        vec![
            song_in(2001, "vol1", &format!("{:032x}", 1001), "Tempo Perdido"),
            song_in(1001, "vol2", "somebody-elses", "Exagerado"),
        ],
        Arc::clone(&favorites),
    );
    let listed = after.get("/?mode=favorites&folder=1").await;

    assert!(
        listed.contains("Tempo Perdido"),
        "the favorite still names its own song: {listed}"
    );
    assert!(
        !listed.contains("Exagerado"),
        "and not whoever moved into the number it used to hold: {listed}"
    );
    assert_eq!(
        favorites.songs_in(1),
        vec![SongCode::new(2001)],
        "and the row was refiled, so the next queue press sends the right number"
    );
}

/// The repair is not a read-time patch that has to happen again tomorrow: it is written back, so a
/// folder drawn once stays right even if the catalog later loses the song entirely.
#[tokio::test]
async fn a_re_banked_favorite_is_written_back_rather_than_patched_each_time() {
    let (before, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let _ = before.get("/?mode=favorites&folder=1").await;

    let after = Harness::with_catalog(
        vec![song_in(
            2001,
            "vol1",
            &format!("{:032x}", 1001),
            "Tempo Perdido",
        )],
        Arc::clone(&favorites),
    );
    let _ = after.get("/?mode=favorites&folder=1").await;
    assert_eq!(favorites.songs_in(1), vec![SongCode::new(2001)]);

    // A third catalog that has never heard of any of it. The row stays where the repair put it.
    let gone = Harness::with_catalog(Vec::new(), Arc::clone(&favorites));
    let _ = gone.get("/?mode=favorites&folder=1").await;
    assert_eq!(
        favorites.songs_in(1),
        vec![SongCode::new(2001)],
        "a favorite is never removed for being hard to find"
    );
}

/// A phone carried to a second machine keeps its collection and lists what that machine has. The
/// folder's count comes from the collection and its rows come from the catalog, so the two disagree
/// — and the sentence under them is what makes the difference readable instead of looking like
/// songs that have gone.
#[tokio::test]
async fn a_folder_says_which_of_its_songs_the_machine_in_hand_does_not_have() {
    let (home, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001), SongCode::new(1002)]);
    let listed = home.get("/?mode=favorites&folder=1").await;
    assert!(listed.contains("Tempo Perdido"), "{listed}");

    // A second machine, holding `vol1` and not the recording either favorite is about.
    let away = Harness::with_catalog(
        vec![song_in(4001, "vol1", "ffffffff", "Outra")],
        Arc::clone(&favorites),
    );
    let body = away.get("/?mode=favorites&folder=1").await;

    assert!(
        body.contains("2 songs in this folder are not on this machine"),
        "{body}"
    );
    assert!(
        body.contains("not in any song pack here"),
        "the remedy line names what would fix it: {body}"
    );
    assert_eq!(
        favorites.songs_in(1),
        vec![SongCode::new(1001), SongCode::new(1002)],
        "and nothing was taken out of the collection for being unreachable today"
    );
}

/// The number rung does not answer for a favorite the catalog can prove it is not about. A second
/// machine that banked another package into the number a favorite was filed under would otherwise
/// list a stranger's song in the right folder, saying nothing.
#[tokio::test]
async fn a_folder_lists_nothing_rather_than_the_wrong_song_after_a_change_of_machine() {
    let (home, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let _ = home.get("/?mode=favorites&folder=1").await;

    // `vol2` holds 1001 here, and `vol1` is installed but does not hold that recording.
    let away = Harness::with_catalog(
        vec![
            song_in(1001, "vol2", "ffffffff", "Somebody Else"),
            song_in(3001, "vol1", "eeeeeeee", "Outra"),
        ],
        Arc::clone(&favorites),
    );
    let body = away.get("/?mode=favorites&folder=1").await;

    assert!(
        !body.contains("Somebody Else"),
        "a number is not an identity across machines: {body}"
    );
    assert!(
        body.contains("One song in this folder is not on this machine"),
        "{body}"
    );
}

/// A collection restored onto a machine that banks the same package differently lands on the right
/// songs — which is what carrying the identity in the file is for.
#[tokio::test]
async fn a_backup_restores_onto_a_machine_that_numbers_its_catalog_differently() {
    let (source, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let file = source.get("/favorites/backup.json").await;
    assert!(file.contains("\"content_hash\""), "{file}");

    // The other machine put `vol1` in bank 7, so every number in that file is wrong here.
    let (_, landed) = Harness::with_folders(&[(1, "Rock")]);
    let fresh = Harness::with_catalog(
        vec![song_in(
            7001,
            "vol1",
            &format!("{:032x}", 1001),
            "Tempo Perdido",
        )],
        Arc::clone(&landed),
    );
    let (_, body) = fresh
        .post_owned(
            "/favorites/backup/restore",
            format!("document={}", km_remote_pages::prefs::encode(&file)),
        )
        .await;

    assert!(body.contains("1 song added"), "{body}");
    assert_eq!(
        landed.songs_in(1),
        vec![SongCode::new(7001)],
        "filed under this machine's number for that song, not the one in the file"
    );
}

/// Listed rather than only counted, and grouped by what would fix it: a pack that can be installed
/// is a different answer from a recording that is nowhere here.
#[tokio::test]
async fn a_restore_says_which_songs_it_could_not_place_and_why() {
    let (source, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let file = source.get("/favorites/backup.json").await;

    // A catalog holding a different package entirely, so `vol1` is absent rather than merely
    // lacking that recording. Not empty — an empty mirror keeps everything, deliberately.
    let (_, landed) = Harness::with_folders(&[(1, "Rock")]);
    let fresh = Harness::with_catalog(
        vec![song_in(9001, "vol9", "unrelated", "Something Else")],
        Arc::clone(&landed),
    );
    let (_, body) = fresh
        .post_owned(
            "/favorites/backup/restore",
            format!("document={}", km_remote_pages::prefs::encode(&file)),
        )
        .await;

    assert!(
        body.contains("song pack this device does not have"),
        "the remedy, not just a count: {body}"
    );
    assert!(body.contains("1001"), "and which song: {body}");
}

/// A format number is compared and **reported**, never enforced: a file refused by an older build
/// is a recovery that did not happen.
#[tokio::test]
async fn a_file_from_a_newer_version_restores_and_says_so() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    // From the constant, not from the number it currently is: the point of this test is a file
    // *newer* than what this build writes, and a hardcoded number makes the replacement a no-op the
    // day the format moves.
    let file = harness.get("/favorites/backup.json").await.replace(
        &format!("\"format\": {}", km_remote_pages::backup::FORMAT),
        "\"format\": 99",
    );

    let (fresh, _) = Harness::with_folders(&[(1, "Other")]);
    let (_, body) = fresh
        .post_owned(
            "/favorites/backup/restore",
            format!("document={}", km_remote_pages::prefs::encode(&file)),
        )
        .await;
    assert!(body.contains("1 song added"), "{body}");
    assert!(body.contains("newer version"), "{body}");
}

/// The two caps are not redundant: the layer lets a plausible file *reach* the handler so this
/// sentence can be the one a person sees.
#[tokio::test]
async fn a_document_past_the_cap_is_worded_rather_than_dropped() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    let huge = "x".repeat(km_remote_pages::handlers::MAX_DOCUMENT + 1);
    let (status, body) = harness
        .post_owned("/favorites/backup/restore", format!("document={huge}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("too large"), "{body}");
}

/// These are pages, not fragments: a swap would put a whole document inside the browse list.
#[tokio::test]
async fn the_share_and_backup_screens_are_ordinary_navigations() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    for path in [
        "/favorites/share/1",
        "/favorites/share/1/receive",
        "/favorites/backup",
        "/favorites/backup/restore",
    ] {
        let body = harness.get(path).await;
        assert!(
            body.contains("<!doctype html>"),
            "{path} is not a whole page: {body}"
        );
        assert!(
            body.contains(r#"<nav class="tabbar">"#),
            "{path} lost the tab bar: {body}"
        );
    }
}

/// **Neither flow adds a tab of its own.** The bar is Songs, Now, Queue and Setup, and it is four
/// wherever you are: sharing is a screen under Songs and the backup is a screen under Setup.
///
/// The count is the assertion. A screen that wanted its own tab would be a screen that had stopped
/// being a step in a flow, and the bar is where that would show first.
#[tokio::test]
async fn neither_feature_adds_a_tab_of_its_own() {
    let (harness, _) = Harness::with_folders(&[(1, "Rock")]);
    for path in ["/favorites/share/1", "/favorites/backup"] {
        let body = harness.get(path).await;
        let bar = body
            .split_once(r#"<nav class="tabbar">"#)
            .and_then(|(_, rest)| rest.split_once("</nav>"))
            .map(|(bar, _)| bar.to_owned())
            .expect("the tab bar");
        assert_eq!(
            bar.matches("<a href=").count(),
            4,
            "{path} changed the tab bar: {bar}"
        );
    }
}

/// The fourth tab is a gear with no word under it, and it still has a name.
///
/// Both halves matter and only one of them is visible. The other three tabs take their accessible
/// name from the label under the mark; this one has no label, so without the attributes a screen
/// reader announces the link by its URL. The narrow rule is what makes it read as a utility rather
/// than a fourth place to be.
#[tokio::test]
async fn the_setup_tab_is_a_gear_with_no_label_and_still_has_a_name() {
    for harness in [Harness::offline(), Harness::online()] {
        let body = harness.get("/setup").await;
        let bar = body
            .split_once(r#"<nav class="tabbar">"#)
            .and_then(|(_, rest)| rest.split_once("</nav>"))
            .map(|(bar, _)| bar.to_owned())
            .expect("the tab bar");

        assert!(bar.contains("&#9881;"), "the gear: {bar}");
        assert!(
            bar.contains(r#"aria-label="Setup""#) && bar.contains(r#"title="Setup""#),
            "a mark with no word under it still needs a name: {bar}"
        );
        assert!(
            !bar.contains("<span>Setup</span>"),
            "and it is not drawn as a word: {bar}"
        );
        assert!(
            bar.contains(r#"class="tab-setup active""#),
            "the tab knows it is the current one: {bar}"
        );
    }

    let css = Harness::offline().get("/static/app.css").await;
    assert!(
        css.contains(".tabbar a.tab-setup { flex: 0 0 3.4rem; }"),
        "and it is the narrow one: {css}"
    );
}

/// Setup is on both remotes, and the two do not hold the same things.
///
/// The tab is never gated — `Capabilities::connection` gates the machine card inside it. A build
/// that took the tab away with the card would leave the online remote no way to set a name or a
/// language, both of which are about the person holding the phone and exist in both modes.
#[tokio::test]
async fn the_setup_tab_is_on_both_remotes_and_carries_what_each_one_has() {
    let (offline, _connect) = Harness::with_connect(StubConnect::new());
    let body = offline.get("/setup").await;
    assert!(body.contains("Singing as"), "the name: {body}");
    assert!(body.contains(r#"id="locale""#), "the language: {body}");
    assert!(body.contains(r#"id="machine""#), "the machine card: {body}");
    assert!(
        body.contains(r#"href="/favorites/backup""#),
        "the backup: {body}"
    );
    // And not the owner's page: `/admin/` is a page on whichever machine this app found, not one
    // this process serves — and half the time this app is open that box is switched off, which is
    // the premise of the app. Absent rather than a door onto nothing.
    assert!(
        !body.contains(r#"href="/admin""#),
        "the owner's page is the machine's own surface: {body}"
    );

    let body = Harness::online().get("/setup").await;
    assert!(body.contains("Singing as"), "the name: {body}");
    assert!(body.contains(r#"id="locale""#), "the language: {body}");
    assert!(
        !body.contains(r#"id="machine""#),
        "the machine is this process: {body}"
    );
    assert!(
        !body.contains("/favorites/backup"),
        "and there are no favorites to back up: {body}"
    );
    // Same-origin here, and the only thing anywhere in this product that points at that page.
    assert!(
        body.contains(r#"href="/admin""#),
        "the owner's door, on the machine's own remote: {body}"
    );
    assert!(
        body.contains("Needs the machine&#39;s password"),
        "a link that always ends in a prompt says so before the tap: {body}"
    );
}

/// The gear is the tab bar's, on every page and in both modes.
///
/// `The gear is this tab's, and the Queue tab's transport disclosure gave it up` refuses one mark
/// meaning two things in one application, and the Queue tab's ⋯ is that decision already paid for.
/// **The trap is that the obvious icon for a row about settings is a gear**, so the Setup tab is
/// where this gets broken — a gear inside the gear tab, which reads as a mark that means nothing in
/// particular. The owner's-page row wears a pencil for exactly this reason.
///
/// Asserted over every page rather than one, because the page that breaks it next is not the page
/// that broke it last.
#[tokio::test]
async fn the_only_gear_anywhere_is_the_setup_tabs_own() {
    let (offline, _connect) = Harness::with_connect(StubConnect::new());
    let online = Harness::online();

    for page in ["/", "/now", "/queue", "/setup"] {
        for body in [offline.get(page).await, online.get(page).await] {
            let (content, bar) = body
                .split_once(r#"<nav class="tabbar">"#)
                .unwrap_or_else(|| panic!("{page} has no tab bar"));
            assert!(
                !content.contains("&#9881;"),
                "{page} wears a gear in its body, which is the Setup tab's mark: {content}"
            );
            assert!(
                bar.contains("&#9881;"),
                "...and the bar is where the one gear belongs: {bar}"
            );
        }
    }
}

/// The machine leads, then the preferences, then the backup — with a rule between and none on top.
///
/// **Order is the content here.** A remote talking to the wrong machine is what brings anybody to
/// this tab, so the card is what the page opens with; the backup is the rarest of the three and is
/// last. The separator is drawn by the group that *follows* another rather than by every group,
/// which is the only shape that survives the machine card being absent — online, and in a build
/// whose host passes no locator. A rule under the heading with nothing above it is a rule about
/// nothing.
#[tokio::test]
async fn the_setup_tab_leads_with_the_machine_and_rules_off_each_group_after_it() {
    let (offline, _connect) = Harness::with_connect(StubConnect::new());
    let body = offline.get("/setup").await;

    let card = body.find(r#"class="machine-card""#).expect("the card");
    let prefs = body.find(r#"id="singer""#).expect("the name");
    let language = body.find(r#"id="locale""#).expect("the language");
    let backup = body.find("/favorites/backup").expect("the backup");
    assert!(
        card < prefs && prefs < language && language < backup,
        "machine, then preferences, then backup: {body}"
    );

    // The two groups that follow one are wrapped; the machine card is not, because nothing precedes
    // it for a rule to sit between.
    assert_eq!(
        body.matches(r#"class="setup-group""#).count(),
        2,
        "the preferences and the backup are each a group: {body}"
    );

    let css = offline.get("/static/app.css").await;
    assert!(
        css.contains(".setup-group { padding: 0.9rem 0; }"),
        "the line stands off the rows on both sides: {css}"
    );
    assert!(
        css.contains(".machine-card + .setup-group,\n.setup-group + .setup-group { border-top:"),
        "and only a group with something before it draws one: {css}"
    );
    assert!(
        !css.contains(".machine-card { padding: 0.9rem 0 0.3rem; border-top:"),
        "the card leads now, so it carries no rule of its own: {css}"
    );

    // Online there is no machine card, so the preferences lead and nothing draws a rule above them
    // — and the owner's door follows them, being the rarest row on this tab for whoever is holding
    // the phone. Two groups, one rule, and it is the second that draws it.
    let body = Harness::online().get("/setup").await;
    assert!(!body.contains(r#"class="machine-card""#), "{body}");
    assert_eq!(
        body.matches(r#"class="setup-group""#).count(),
        2,
        "the preferences and the owner's page are each a group: {body}"
    );
    let prefs = body.find(r#"id="singer""#).expect("the name");
    let owner = body.find(r#"href="/admin""#).expect("the owner's page");
    assert!(
        prefs < owner,
        "this device's preferences come before a door most viewers cannot open: {body}"
    );
}

/// The Now tab is the player card and nothing else.
///
/// The whole point of the Setup tab: what is playing and what runs the appliance stopped sharing a
/// page. Asserted in both modes, because the online one never had the card and would pass either
/// way — it is the offline one that had to lose it.
#[tokio::test]
async fn the_now_tab_carries_the_player_and_nothing_about_the_machine() {
    let (offline, _connect) = Harness::with_connect(StubConnect::new());
    for body in [
        offline.get("/now").await,
        Harness::online().get("/now").await,
    ] {
        assert!(
            body.contains(r#"data-sse="player""#),
            "the player card stays: {body}"
        );
        assert!(
            !body.contains(r#"id="machine""#) && !body.contains("/machine/rescan"),
            "and the machine card is gone: {body}"
        );
        assert!(
            !body.contains(r#"name="address""#),
            "including the address box: {body}"
        );
    }
}

/// Setting a name answers with a toast and no queue.
///
/// It used to answer with the queue block, which looked like it was keeping the rows in step and was
/// not: a row prints the singer the machine recorded on that entry, never this cookie. The swap was
/// invisible, and once the field moved to a page with no `#queue` on it there was nothing for htmx
/// to put it in either.
#[tokio::test]
async fn setting_a_name_answers_with_a_toast_and_not_the_queue() {
    let harness = Harness::offline();
    let (status, body) = harness.post_form("/singer", "singer=Ana+Carolina").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Ana Carolina"),
        "the toast says the name: {body}"
    );
    assert!(
        !body.contains(r#"id="queue""#),
        "and it is not carrying a queue: {body}"
    );
}

/// A folder in a hand-edited file whose every code is unreadable must not be created.
///
/// The export leaves an empty folder out of the file and out of its count; this is that rule read
/// backwards. Without it a restore invents a folder nobody asked for and reports it under *folders
/// created*, which is a change with no visible cause.
#[tokio::test]
async fn a_restored_folder_with_nothing_readable_in_it_is_not_created() {
    let (harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    favorites.fill(1, &[SongCode::new(1001)]);
    let file = harness.get("/favorites/backup.json").await;

    // A second folder whose only row is a code no parser will take — a letter O for a zero, which
    // is what a person editing this by hand actually types.
    let meddled = file.replace(
        "\"folders\": [",
        "\"folders\": [\n    { \"name\": \"Typo\", \"songs\": [ { \"code\": \"10O1\" } ] },",
    );

    let (fresh, into) = Harness::with_folders(&[(1, "Other")]);
    let (_, body) = fresh
        .post_owned(
            "/favorites/backup/restore",
            format!("document={}", km_remote_pages::prefs::encode(&meddled)),
        )
        .await;

    assert_eq!(
        into.names(),
        vec!["Other".to_owned(), "Rock".to_owned()],
        "`Typo` carried nothing readable, so it must not exist"
    );
    assert!(
        body.contains("1 folder created"),
        "and only the one that carried something is counted: {body}"
    );
    assert!(
        body.contains("1 read from 1 folder"),
        "the folder count is over what was written, not what was named: {body}"
    );
    assert!(
        body.contains("not a song number"),
        "the unreadable row is still reported: {body}"
    );
}

/// A catalog that fails to answer is a fault, not an empty catalog.
///
/// `known_songs` short-circuits on a count of zero — the phone that has never reached a machine —
/// and folding an error into that branch would silently skip the filter and say nothing.
#[tokio::test]
async fn a_catalog_that_will_not_answer_fails_the_merge_rather_than_skipping_the_filter() {
    let favorites = Arc::new(StubFavorites::with_folders(&[(1, "Rock")]));
    let mut harness = Harness::with(StubMachine::new(), Capabilities::offline());
    harness.remote.songs = Arc::new(BrokenSongs);
    harness.remote.favorites = Some(Arc::clone(&favorites) as Arc<dyn Favorites>);

    let (status, body) = harness
        .post_owned(
            "/favorites/share/1/merge",
            format!("code={}", code_for("Party", &[1001])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "a page, not a status htmx refuses");
    assert!(
        !body.contains("songs added"),
        "nothing may be filed on a catalog that did not answer: {body}"
    );
    assert!(favorites.songs_in(1).is_empty(), "and nothing was written");
}

// -- hiding a package ----------------------------------------------------------------------------

const KEPT: &str = "1f4a9c8e2b7d0356";
const HIDDEN: &str = "a1b2c3d4e5f60789";

/// A catalog of two packages, one song each. The artist and the language of the second song belong
/// to no other song.
fn two_packages(capabilities: Capabilities) -> Harness {
    let mut harness = Harness::with(StubMachine::new(), capabilities);
    let kept = SongDto {
        package_id: KEPT.to_owned(),
        ..song(1001, "Tempo Perdido", Some("Legião Urbana"))
    };
    let hidden = SongDto {
        package_id: HIDDEN.to_owned(),
        language: Some("ja".to_owned()),
        ..song(2001, "Kawaii Song", Some("Only Hidden"))
    };
    harness.remote.songs = Arc::new(StubSongs {
        songs: vec![kept, hidden],
    });
    harness
}

/// A post to the hide form, with the cookie the phone already holds.
fn hide_request(body: String, cookie: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/packages/hidden")
        .header("HX-Request", "true");
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie.to_owned());
    }
    request.body(Body::from(body)).expect("build")
}

/// Both remotes, because a hide is about the person holding the phone. The hidden package leaves the
/// song list, the artists and the language picker, and a typed song number still finds its song.
#[tokio::test]
async fn a_hidden_package_leaves_the_list_and_the_pickers_on_both_remotes() {
    for capabilities in [Capabilities::offline(), Capabilities::online()] {
        let harness = two_packages(capabilities);

        let setup = harness.get("/setup").await;
        assert!(
            setup.contains(r#"href="/setup/packages""#),
            "two packages draw the Packages tab: {setup}"
        );
        assert!(
            !setup.contains("/packages/hidden"),
            "the list is on its own tab: {setup}"
        );

        let packages = harness.get("/setup/packages").await;
        assert!(packages.contains(&format!("Volume {HIDDEN}")), "{packages}");
        assert!(
            packages.contains(&format!(r#"name="show" value="{HIDDEN}" checked"#)),
            "shown until somebody unticks it: {packages}"
        );
        assert!(
            packages.contains(r#"hx-post="/packages/hidden" hx-trigger="change" hx-swap="none""#),
            "the answer is only a toast, so swapping it in would empty the form: {packages}"
        );

        let (status, headers, _) = harness
            .send(hide_request(
                format!("listed={KEPT}&listed={HIDDEN}&show={KEPT}"),
                None,
            ))
            .await;
        assert_eq!(status, StatusCode::OK);
        let written = set_cookie(&headers);
        assert!(
            written.contains(&format!("km_hidden={HIDDEN};")),
            "{written}"
        );
        let cookie = format!("km_hidden={HIDDEN}");
        let cookie = Some(cookie.as_str());

        let (_, _, songs) = harness.get_full("/", cookie).await;
        assert!(songs.contains("Tempo Perdido"), "{songs}");
        assert!(!songs.contains("Kawaii Song"), "{songs}");
        assert!(
            !songs.contains(r#"value="ja""#),
            "a language only the hidden package holds leaves the picker: {songs}"
        );

        let (_, _, artists) = harness.get_full("/?mode=artists", cookie).await;
        assert!(artists.contains("Legião Urbana"), "{artists}");
        assert!(!artists.contains("Only Hidden"), "{artists}");

        let (_, _, by_number) = harness.get_full("/?q=2001", cookie).await;
        assert!(
            by_number.contains("Kawaii Song"),
            "a song number still finds it: {by_number}"
        );

        let (_, _, empty) = harness.get_full("/?q=Kawaii", cookie).await;
        assert!(
            empty.contains("Packages you hid under Setup, Packages are not searched."),
            "an empty search says where the rest went: {empty}"
        );

        let (_, _, packages) = harness.get_full("/setup/packages", cookie).await;
        assert!(
            packages.contains(&format!(r#"name="show" value="{HIDDEN}">"#)),
            "the Packages tab shows it unticked: {packages}"
        );
    }
}

/// A favorite is a song somebody chose, so a folder still lists it when its package is hidden.
#[tokio::test]
async fn a_favorites_folder_still_lists_a_song_from_a_hidden_package() {
    let (mut harness, favorites) = Harness::with_folders(&[(1, "Rock")]);
    harness.remote.songs = two_packages(Capabilities::offline()).remote.songs;
    favorites.fill(1, &[SongCode::new(2001)]);

    let cookie = format!("km_hidden={HIDDEN}");
    let (_, _, folder) = harness
        .get_full("/?mode=favorites&folder=1", Some(&cookie))
        .await;
    assert!(folder.contains("Kawaii Song"), "{folder}");
}

/// An id the form did not list keeps its place in the cookie, and showing everything clears it.
#[tokio::test]
async fn saving_the_list_keeps_a_hidden_package_the_form_did_not_show() {
    let harness = two_packages(Capabilities::offline());
    let every_box = format!("listed={KEPT}&listed={HIDDEN}&show={KEPT}&show={HIDDEN}");
    let away = "0123456789abcdef";

    let held = format!("km_hidden={away}.{HIDDEN}");
    let (_, headers, _) = harness
        .send(hide_request(every_box.clone(), Some(&held)))
        .await;
    let written = set_cookie(&headers);
    assert!(written.contains(&format!("km_hidden={away};")), "{written}");

    let held = format!("km_hidden={HIDDEN}");
    let (_, headers, _) = harness.send(hide_request(every_box, Some(&held))).await;
    let written = set_cookie(&headers);
    assert!(
        written.contains("km_hidden=;") && written.contains("Max-Age=0"),
        "{written}"
    );
}
