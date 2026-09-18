//! Serving the singer's remote from the machine itself.
//!
//! `km-remote-pages` is written against three traits and a guard; this is what stands behind them when the
//! machine is the thing being remote-controlled. There is no network here at all — no HTTP client, no
//! socket, no serialization and back — because the catalog and the controller are objects in this
//! process. What the offline app reaches over Wi-Fi, this reaches by calling.
//!
//! Two things it does **not** do, and both are the point.
//!
//! It does not reimplement any operation. Queueing a song is [`km_api::ops::enqueue`], the same
//! function the JSON endpoint calls, so the two cannot come to disagree about what queueing means —
//! and, more to the point, cannot come to disagree about which events it publishes. A remote that
//! queued a song without announcing it would leave every other phone in the room showing a stale
//! queue, and nothing would look broken.
//!
//! And it does not invent an access rule. The question goes straight to
//! [`ApiState::authorize`](km_api::server::ApiState::authorize), with the token out of the page's
//! cookie put back into the header that check expects — and that check is now the URL prefix
//! itself, so there is not even a table to disagree with. **Nothing the remote can do is anything
//! the API would not already have allowed the same caller to do**, which is what makes serving it
//! at `/` a page rather than a permission.

use std::sync::Arc;

use km_api::ApiState;
use km_api::dto::{AddedToQueueDto, QueueDto, SettingsPatchDto, SongDto, StateDto};
use km_api::events::Event;
use km_api::machine::{Catalog, TransportCommand};
use km_catalog::SearchQuery;
use km_remote_pages::machine::{
    ArtistFilter, ArtistRow, BrowseQuery, Connection, LanguageRow, Machine, Miss, Order,
    PackageRow, RemoteError, Resolution, SongPage, SongRef, Songs, TagRow, Transport,
};
use km_songcode::SongCode;
use tokio::sync::broadcast;

/// The machine's own catalog, behind the remote's `Songs` trait.
#[derive(Clone)]
pub struct OnlineSongs(ApiState);

impl OnlineSongs {
    /// Wraps the API's state.
    pub fn new(state: ApiState) -> Self {
        Self(state)
    }

    /// Asks the catalog something, on a blocking thread.
    ///
    /// **Every method below goes through this, and none of them used to.** The catalog is one
    /// SQLite connection behind a `std::sync::Mutex`, and `Songs` is an `async` trait — so a query
    /// written the obvious way runs to completion on a tokio worker, holding it for as long as the
    /// query and the wait for the lock take together. That is survivable while the queries are
    /// small; what makes it not survivable is who else wants that mutex. Installing a package holds
    /// it for the whole transaction — seconds for a large one, and `km_api::handlers::install`
    /// already says so in as many words — so every page of this remote would sit on a worker
    /// waiting, and with as many workers as the box has cores, the API, the pages and the event
    /// stream stop together rather than one at a time.
    ///
    /// `km_api::machine`'s own header says the trait leaves *how* to get off the runtime to the
    /// implementation. `km-remote-core`'s mirror answers that question; this is this side's answer.
    async fn ask<T, F>(&self, query: F) -> Result<T, RemoteError>
    where
        F: FnOnce(&dyn Catalog) -> Result<T, km_api::machine::CatalogError> + Send + 'static,
        T: Send + 'static,
    {
        // An owned handle, because the closure outlives this borrow. `ApiState::catalog_handle`
        // exists for exactly this.
        let catalog = self.0.catalog_handle();
        match tokio::task::spawn_blocking(move || query(catalog.as_ref())).await {
            Ok(result) => result.map_err(catalog_failed),
            // A blocking task fails only by panicking. Said plainly rather than as an empty list:
            // a page reporting no songs because the catalog panicked is a page nobody can debug.
            Err(error) => Err(RemoteError::Failed(format!(
                "the catalog panicked answering that: {error}"
            ))),
        }
    }
}

/// Turns a catalog failure into something a page can show.
fn catalog_failed(error: km_api::machine::CatalogError) -> RemoteError {
    match error {
        km_api::machine::CatalogError::NotFound(_) => RemoteError::NotFound,
        km_api::machine::CatalogError::Rejected(why) => RemoteError::Rejected(why),
        // "not now" rather than "not ever", which is a different sentence on a page.
        km_api::machine::CatalogError::Unavailable(refusal) => RemoteError::Unavailable {
            code: refusal
                .code
                .unwrap_or(km_api::ApiError::UNAVAILABLE)
                .to_owned(),
            message: refusal.message,
        },
        km_api::machine::CatalogError::Failed(why) => RemoteError::Failed(why),
    }
}

/// Turns an API failure into something a page can show.
///
/// Every control this serves goes through [`km_api::ops`], which has already turned a
/// `ControlError` into an `ApiError` — so there is one translation here and not two, and the page
/// says the same thing the JSON client would have been told.
fn api_failed(error: km_api::ApiError) -> RemoteError {
    use km_api::ApiError;
    match error {
        ApiError::NotFound(_) | ApiError::UnknownEndpoint(_) => RemoteError::NotFound,
        ApiError::Unauthorized(_) | ApiError::Forbidden(_) => RemoteError::Unauthorized,
        ApiError::BadRequest(why) => RemoteError::Rejected(why),
        // A 409 is several distinguishable things, which is why it carries a code: the queue being
        // full and a song refusing a control are different sentences on a page.
        ApiError::Conflict { code, message } => match code {
            "queue_full" => RemoteError::QueueFull,
            _ => RemoteError::Unavailable {
                code: code.to_owned(),
                message,
            },
        },
        other => RemoteError::Failed(other.to_string()),
    }
}

#[async_trait::async_trait]
impl Songs for OnlineSongs {
    async fn search(&self, query: &BrowseQuery) -> Result<SongPage, RemoteError> {
        // The letter filter is absent from this mode's capabilities, so it never arrives here —
        // `library.sqlite` has no indexed folded initial to serve one with. Everything else maps
        // straight across, `language` included, which the catalog matches exactly on both sides.
        let artist = match &query.artist {
            // The catalog's own artist filter is a substring match, which is right for a search box
            // and wrong for a drill-down. Narrowing an exact one afterwards costs nothing at the size
            // an artist's discography is.
            Some(ArtistFilter::Contains(name) | ArtistFilter::Exactly(name)) => Some(name.clone()),
            None => None,
        };
        let search = SearchQuery {
            text: query.text.clone(),
            artist,
            language: query.language.clone(),
            // Straight across: both sides are slugs, folded by `BrowseParams::tags` before they
            // reached the query, and the catalog folds again on its own way into SQL.
            tags: query.tags.clone(),
            exclude_packages: query.hidden_packages.clone(),
            min_suitability: None,
            melody_only: false,
            sort: match query.order {
                Order::Best => km_catalog::SortOrder::Relevance,
                Order::Title => km_catalog::SortOrder::Title,
                Order::Artist => km_catalog::SortOrder::Artist,
                Order::Number => km_catalog::SortOrder::Number,
            },
            limit: query.limit,
            offset: query.offset,
        };
        let mut songs: Vec<SongDto> = self
            .ask(move |catalog| catalog.search(&search))
            .await?
            .iter()
            .map(SongDto::from)
            .collect();

        if let Some(ArtistFilter::Exactly(name)) = &query.artist {
            songs.retain(|song| song.artist.as_deref() == Some(name.as_str()));
        }

        // No total: counting would be a second query over the whole catalog for a number nobody
        // scrolls to. `SongPage::total` is `Option` precisely so this can say so rather than guess.
        let more = songs.len() == query.limit;
        Ok(SongPage {
            songs,
            total: None,
            more,
        })
    }

    async fn song(&self, number: SongCode) -> Result<Option<SongDto>, RemoteError> {
        Ok(self
            .ask(move |catalog| catalog.song(number))
            .await?
            .map(|song| SongDto::from(&song)))
    }

    async fn songs_by_number(&self, numbers: &[SongCode]) -> Result<Vec<SongDto>, RemoteError> {
        // The whole loop in one visit rather than one visit per number: the numbers are a favorites
        // folder, so the list is short but not one, and a hop onto a blocking thread per song would
        // cost more than the queries do.
        let numbers = numbers.to_vec();
        self.ask(move |catalog| {
            let mut found = Vec::with_capacity(numbers.len());
            for number in numbers {
                if let Some(song) = catalog.song(number)? {
                    found.push(SongDto::from(&song));
                }
            }
            Ok(found)
        })
        .await
    }

    async fn resolve(&self, refs: &[SongRef]) -> Result<Vec<Resolution>, RemoteError> {
        // One visit for the whole folder, for the reason `songs_by_number` above gives.
        let refs = refs.to_vec();
        self.ask(move |catalog| {
            let mut out = Vec::with_capacity(refs.len());
            for asked in refs {
                // The three rungs, in the order and for the reasons `Songs::resolve` sets out.
                let mut found = None;
                if let (Some(package), Some(hash)) = (&asked.package_id, &asked.content_hash) {
                    found = catalog.song_in_package(package, hash)?;
                }
                if found.is_none()
                    && let Some(hash) = &asked.content_hash
                {
                    found = catalog.song_by_content(hash)?;
                }
                if found.is_none() {
                    found = catalog
                        .song(asked.code)?
                        .filter(|song| !asked.contradicted_by(song.content_hash.as_deref()));
                }
                let outcome = match found {
                    Some(song) => Ok(SongDto::from(&song)),
                    None => Err(match &asked.package_id {
                        Some(package) if !catalog.has_package(package)? => Miss::PackageAbsent,
                        _ if asked.is_identified() => Miss::RecordingAbsent,
                        _ => Miss::NumberAbsent,
                    }),
                };
                out.push(Resolution { asked, outcome });
            }
            Ok(out)
        })
        .await
    }

    async fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<ArtistRow>, RemoteError> {
        let contains = contains.map(str::to_owned);
        let hidden = hidden.to_vec();
        Ok(self
            .ask(move |catalog| catalog.artists(contains.as_deref(), &hidden))
            .await?
            .into_iter()
            .map(|(name, songs)| ArtistRow { name, songs })
            .collect())
    }

    async fn languages(&self, hidden: &[String]) -> Result<Vec<LanguageRow>, RemoteError> {
        let hidden = hidden.to_vec();
        Ok(self
            .ask(move |catalog| catalog.languages(&hidden))
            .await?
            .into_iter()
            .map(|(code, songs)| LanguageRow::new(code, songs))
            .collect())
    }

    async fn tags(&self, hidden: &[String]) -> Result<Vec<TagRow>, RemoteError> {
        let hidden = hidden.to_vec();
        Ok(self
            .ask(move |catalog| catalog.tags(&hidden))
            .await?
            .into_iter()
            .map(|(tag, songs)| TagRow { tag, songs })
            .collect())
    }

    async fn packages(&self) -> Result<Vec<PackageRow>, RemoteError> {
        let mut rows: Vec<PackageRow> = self
            .ask(|catalog| catalog.packages())
            .await?
            .into_iter()
            .map(|package| PackageRow {
                id: package.id,
                name: package.name,
                songs: package.song_count,
            })
            .collect();
        // By name, as the offline mirror lists them, so both remotes draw one order.
        rows.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(rows)
    }

    async fn count(&self) -> Result<usize, RemoteError> {
        self.ask(|catalog| catalog.song_count()).await
    }
}

/// The machine's own controls, behind the remote's `Machine` trait.
#[derive(Clone)]
pub struct OnlineMachine(ApiState);

impl OnlineMachine {
    /// Wraps the API's state.
    pub fn new(state: ApiState) -> Self {
        Self(state)
    }
}

#[async_trait::async_trait]
impl Machine for OnlineMachine {
    async fn state(&self) -> Result<StateDto, RemoteError> {
        Ok(StateDto::from(&self.0.controller().snapshot()))
    }

    async fn queue(&self) -> Result<QueueDto, RemoteError> {
        Ok(QueueDto::new(&self.0.controller().queue()))
    }

    async fn package_problems(&self) -> Vec<String> {
        self.0
            .catalog()
            .package_problems()
            .iter()
            .map(|problem| {
                // The file's name, never its path: this reaches a banner on every phone in the
                // room. See `PackageProblemDto::file`.
                let name = problem.package_id.clone().unwrap_or_else(|| {
                    problem
                        .path
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&problem.path)
                        .to_owned()
                });
                format!("\"{name}\" was not installed: {}", problem.reason)
            })
            .collect()
    }

    async fn enqueue(
        &self,
        number: SongCode,
        singer: Option<&str>,
    ) -> Result<AddedToQueueDto, RemoteError> {
        // Off the runtime, like the JSON route beside it: queueing onto an idle machine loads the
        // song, and for a video that is ffmpeg opening a decoder. See `km_api::ops::off_runtime`.
        let singer = singer.map(str::to_owned);
        km_api::ops::off_runtime(&self.0, move |state| {
            km_api::ops::enqueue(state, number, singer.as_deref())
        })
        .await
        .map_err(api_failed)
    }

    async fn dequeue(&self, entry_id: u64) -> Result<QueueDto, RemoteError> {
        km_api::ops::dequeue(&self.0, entry_id).map_err(api_failed)
    }

    async fn move_entry(&self, entry_id: u64, to_index: usize) -> Result<QueueDto, RemoteError> {
        km_api::ops::move_entry(&self.0, entry_id, to_index).map_err(api_failed)
    }

    async fn clear_queue(&self) -> Result<QueueDto, RemoteError> {
        km_api::ops::clear_queue(&self.0).map_err(api_failed)
    }

    async fn transport(&self, command: Transport) -> Result<StateDto, RemoteError> {
        let command = match command {
            Transport::Play => TransportCommand::Play,
            Transport::Pause => TransportCommand::Pause,
            Transport::Skip => TransportCommand::Skip,
            Transport::Restart => TransportCommand::Restart,
            Transport::Stop => TransportCommand::Stop,
        };
        km_api::ops::off_runtime(&self.0, move |state| km_api::ops::transport(state, command))
            .await
            .map_err(api_failed)
    }

    /// No `off_runtime`, unlike `enqueue` and `transport` beside it: this sets a flag and the poll
    /// thread does the loading. See `km_api::handlers::start_demo`.
    async fn start_demo(&self) -> Result<(), RemoteError> {
        km_api::ops::start_demo(&self.0)
            .map(|_| ())
            .map_err(api_failed)
    }

    async fn update_settings(&self, patch: &SettingsPatchDto) -> Result<StateDto, RemoteError> {
        // The settings route answers with the settings; the card wants the whole state. Re-reading
        // rather than patching a copy is what makes a clamped value show what is really set.
        km_api::ops::apply_settings(&self.0, patch.to_patch())
            .map_err(api_failed)
            .map(|_| StateDto::from(&self.0.controller().snapshot()))
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        // The machine's own channel, not a second one. `ApiState::with_events` exists so that the
        // engine, the API and now this all publish into and read from the same place — a song ending
        // and the next one starting is not something any request caused, so nothing else would ever
        // announce it.
        self.0.events().subscribe()
    }

    fn connection(&self) -> Connection {
        // The machine is this process. It cannot be away, and the banner is never drawn.
        Connection::in_process()
    }
}

/// Everything the machine needs to serve its own remote.
///
/// Returns the state the pump reads and the router to merge at `/`. Built together because the two
/// have to share one `Remote` — the pump publishes into the same hub the pages subscribe to, and two
/// of those would be a page listening to a fan-out nothing feeds.
pub fn build(state: ApiState) -> (km_remote_pages::Remote, axum::Router) {
    let remote = km_remote_pages::Remote::new(
        Arc::new(OnlineSongs::new(state.clone())),
        Arc::new(OnlineMachine::new(state.clone())),
        // No favorites. The machine is an appliance under a television, so a collection living here
        // would be a shared list nobody owns — see the `Two remotes` entry in `docs/decisions/`.
        None,
        km_remote_pages::Capabilities::online(),
    );
    let router = km_remote_pages::router(remote.clone());
    (remote, router)
}
