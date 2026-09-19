//! The endpoints.
//!
//! Each handler does the same four things in the same order: read its input, ask the machine, publish
//! whatever changed, answer. The publishing is the part that is easy to forget and expensive to get
//! wrong — a remote that changed the key and never heard about it shows a stale value until the next
//! quarter-second state tick, and a remote that reordered the queue and never heard about it shows
//! the wrong singer as next until somebody reloads. So mutations go through the small helpers at the
//! bottom of this file rather than each handler remembering.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Json;
use axum::extract::multipart::MultipartRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Multipart, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderName, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use km_catalog::search::SearchQuery;
use km_songcode::SongCode;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast::error::RecvError;

use crate::auth::MIN_PASSWORD_CHARS;
use crate::book::{self, BookFilter, BookQuery};
use crate::connect::ConnectInfo;
use crate::discover::Discovery;
use crate::dto::{
    AddToQueueRequest, AddedToQueueDto, AdminPasswordDto, AdminPasswordRequest, BankDto,
    BankRequest, DebugDto, DebugRequest, ErrorDto, ExportParams, InstallReportDto, InstallRequest,
    LoginRequest, LoginResponse, LyricsDto, MachineLocaleRequest, MachineNameRequest, MicPatchDto,
    MicsDto, MoveRequest, PackageDto, PackagesDto, PlayFileRequest, PowerDto, QueueDto,
    SearchParams, SearchResponse, SeekRequest, SetDemoDelayRequest, SetDemoRequest, SettingsDto,
    SettingsPatchDto, SongDto, StateDto, UninstallDto, UploadReportDto, effective_limit,
    export_limit,
};
use crate::error::{ApiError, ApiResult};
use crate::events::Event;
use crate::machine::{Audition, CatalogError, SettingsPatch, TransportCommand, Upload};
use crate::ops;
use crate::server::ApiState;

/// A song code from a path segment, refused in this API's own shape.
///
/// `Path<SongCode>` would do the parsing just as well, and for most of this crate's life that is
/// what the routes took. What it could not do is *fail* properly: axum answers its own extractor
/// rejections with a plain-text 400 or 422, so a malformed code was the one refusal here that did
/// not arrive as an [`crate::dto::ErrorDto`] — no `error` code to match on, no `message`, not even
/// JSON. That went unnoticed while the only malformed codes were typos like `5A0`; song numbers
/// now stop at [`km_songcode::MAX_NUMBER`], so `1000000` is a value a person can plausibly send and
/// **used** to get a tidy 404 saying no such song.
///
/// The mapping is the obvious one: a `CodeError`
/// is a bad request, and its `Display` is the message.
#[derive(Debug, Clone, Copy)]
pub struct Code(pub SongCode);

impl<S: Send + Sync> FromRequestParts<S> for Code {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(text) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|error| ApiError::BadRequest(error.body_text()))?;
        text.parse()
            .map(Self)
            .map_err(|error: km_songcode::CodeError| ApiError::BadRequest(error.to_string()))
    }
}

/// A JSON body, refused in this API's own shape.
///
/// Same argument as [`Code`], applied to the other place a `SongCode` arrives: `POST /queue` takes
/// one inside its body, so an over-range number fails during deserialization and would otherwise be
/// answered by axum rather than by this crate.
#[derive(Debug, Clone, Copy)]
pub struct Body<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequest<S> for Body<T> {
    type Rejection = ApiError;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| refused(error.status(), error.body_text()))
    }
}

/// Turns an extractor's own rejection into this API's error shape.
///
/// **Always a 400, and that is a decision rather than a simplification.** axum answers 422 when a
/// body is well-formed JSON it cannot make sense of, and 400 when it is not JSON at all. Preserving
/// that split was tried and is wrong here, because this API already answers a *value* error one way:
/// `GET /songs/10000000` is a 400, by way of the [`Code`] extractor, and [`Body`] exists — its own
/// doc says so — precisely so the same over-range number inside `POST /queue`'s body is answered the
/// same way rather than by axum. A client should not get 400 for a number in a path and 422 for the
/// identical number in a body.
///
/// So the split was the accident, not the rule: `/settings` answered 422 only because it still took
/// a raw `Json`, which is the inconsistency this change is about. What these wrappers add is the
/// *shape* — an `ErrorDto` with a stable `error` code — where axum's own rejection is plain text
/// with nothing to match on. The detail is not lost: the message still names the offending field.
///
/// Takes the pieces rather than the rejection, because every axum rejection type has its own
/// inherent `status()` and `body_text()` and they share no trait to be generic over.
fn refused(_status: axum::http::StatusCode, message: String) -> ApiError {
    ApiError::BadRequest(message)
}

/// A query string, refused in this API's own shape. See [`Code`].
#[derive(Debug, Clone, Copy)]
pub struct Params<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> FromRequestParts<S> for Params<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|error| refused(error.status(), error.body_text()))
    }
}

/// A path segment, refused in this API's own shape. See [`Code`].
///
/// **The third of the trio, and the one that was missing.** `Body` and `Params` already existed and
/// were used four times between them against eighty-one raw extractors — so `PUT /settings` with a
/// malformed body answered `application/json` while `DELETE /queue/nonsense` answered axum's
/// plain-text rejection, with no `error` code to match on and not even JSON to parse. Which shape a
/// client got depended on which extractor the handler happened to reach for.
#[derive(Debug, Clone, Copy)]
pub struct Segment<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned + Send> FromRequestParts<S> for Segment<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            // Never a 404, and the distinction is real: `/queue/nonsense` is a malformed request,
            // where `/queue/424242` is a well-formed one about something that is not there. Only
            // the second is a 404, and it is the handler that says so.
            .map_err(|error| refused(error.status(), error.body_text()))
    }
}

/// The client's address, when the server knows it.
///
/// A custom extractor rather than `ConnectInfo<SocketAddr>` directly, because that one *rejects*
/// when the information is absent — and absent is a legitimate state (a test driving the router as a
/// service, a future unix-socket transport). Callers decide what to do with `None`; every current
/// one fails closed.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<SocketAddr>>()
                .map(|connect| connect.0),
        ))
    }
}

impl Peer {
    /// The address to attribute a rate-limited action to.
    ///
    /// An unknown peer shares one bucket rather than getting a free pass, so a proxy that hides
    /// addresses throttles everybody behind it instead of nobody.
    fn rate_limit_key(self) -> std::net::IpAddr {
        self.0
            .map(|addr| addr.ip())
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
    }
}

// -- discovery and state -------------------------------------------------------------------------

/// `GET /api/v1/discover`
pub async fn discover(State(state): State<ApiState>) -> Json<Discovery> {
    Json(state.discovery())
}

/// `GET /api/v1/connect`
///
/// Not in the original endpoint list, and added because the display needs it: `km-app` runs the
/// display in-process and can read [`ApiState::connect_info`] directly, but the dev remote cannot,
/// and "what address does the machine think it is on" is the first question when a phone cannot
/// connect. Recorded in `docs/ARCHITECTURE.md`.
pub async fn connect(State(state): State<ApiState>) -> Json<ConnectInfo> {
    Json(state.connect_info())
}

/// `GET /api/v1/state`
pub async fn get_state(State(state): State<ApiState>) -> Json<StateDto> {
    Json(StateDto::from(&state.controller().snapshot()))
}

// -- catalog -----------------------------------------------------------------------------------

/// `GET /api/v1/songs`
pub async fn search_songs(
    State(state): State<ApiState>,
    Params(params): Params<SearchParams>,
) -> ApiResult<Json<SearchResponse>> {
    let limit = effective_limit(params.limit);
    let query = SearchQuery {
        text: params.q.filter(|text| !text.trim().is_empty()),
        artist: params.artist.filter(|text| !text.trim().is_empty()),
        language: params.language.filter(|text| !text.trim().is_empty()),
        // Split here rather than in `SearchQuery`, so the catalog takes a list and the wire spelling
        // is decided once, at the edge, where every other parameter is parsed.
        tags: params
            .tags
            .as_deref()
            .map(km_kmpkg::tag::parse_list)
            .unwrap_or_default()
            .into_iter()
            .map(km_kmpkg::Tag::into_string)
            .collect(),
        // Hiding a package belongs to one person's remote and is not a parameter of the API.
        exclude_packages: Vec::new(),
        min_suitability: params.min_suitability,
        melody_only: params.melody_only.unwrap_or(false),
        sort: params.sort.unwrap_or_default().into(),
        limit,
        offset: params.offset.unwrap_or(0),
    };
    let songs = state.catalog().search(&query)?;
    Ok(Json(SearchResponse::new(&songs, query.offset, limit)))
}

/// `GET /api/v1/songs/export`
///
/// One page of the whole catalog as **NDJSON** — one [`SongDto`] per line, no wrapping array — so a
/// client can parse it a row at a time instead of holding the page as one JSON document. Paged by
/// `?after=<number>`, the last number of the previous page, and finished when a page comes back
/// shorter than the limit asked for.
///
/// Two headers make a refresh cheap. `X-Km-Catalog-Version` and the `ETag` both carry the number
/// that moves on every install and uninstall, so a client that stored it from last time can compare
/// before downloading anything — on a six-figure catalog that is the difference between a refresh
/// that costs nothing and one that costs a minute of somebody's evening.
///
/// **Deliberately not a streamed body.** At the page sizes this allows, a full catalog is twenty
/// requests; streaming would buy nothing a client can use and would put a long-lived response on a
/// machine whose other job is playing audio.
pub async fn export_songs(
    State(state): State<ApiState>,
    Params(params): Params<ExportParams>,
) -> ApiResult<Response> {
    let limit = export_limit(params.limit);
    let songs = state.catalog().export(params.after, limit)?;
    let version = state.catalog().catalog_version()?;

    let mut body = String::with_capacity(songs.len() * 200);
    for song in &songs {
        // A row that will not serialize is a bug in this crate, not in the catalog, and dropping
        // it silently would hand somebody a song list quietly missing one. There is nothing sensible
        // to do but fail the page.
        let line = serde_json::to_string(&SongDto::from(song)).map_err(|error| {
            ApiError::Internal(format!("serializing song {}: {error}", song.number))
        })?;
        body.push_str(&line);
        body.push('\n');
    }

    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson".to_owned()),
            (header::ETAG, format!("\"{version}\"")),
            (
                HeaderName::from_static("x-km-catalog-version"),
                version.to_string(),
            ),
        ],
        body,
    )
        .into_response())
}

/// `GET /api/v1/songs/book.pdf`
///
/// The whole catalog as a printable song book — the same thing `karaokemachine --song-book`
/// writes, from the same code, so the two cannot disagree about what a book looks like.
///
/// **Public**, like the export it is a rendering of, and now by construction rather than by an
/// entry in a table: it is not under `/api/v1/admin/`, which is the whole of the rule. See
/// [`crate::routes::needs_admin_token`].
pub async fn song_book(
    State(state): State<ApiState>,
    Params(mut query): Params<BookQuery>,
) -> ApiResult<Response> {
    // **The machine's own name is the default for `?name=`, applied before anything reads it.** A
    // book found on a table in a house with a machine in two rooms should say which one it is a
    // list for, and the masthead is the field that exists for exactly that. Written into `query`
    // rather than passed alongside it so the `ETag` below follows through `BookQuery::name_tag`:
    // renaming the machine has to change the validator, or a cache serves the old book.
    if query.name.is_none() {
        query.name = book::book_name_for(&state.machine_name());
    }
    let query = query;

    let songs = book::collect(|after, limit| state.catalog().export(after, limit))?;
    let version = state.catalog().catalog_version()?;
    let filter = query.filter();

    // **Rendered off the runtime.** A 233-page book is tens to low hundreds of milliseconds of
    // straight CPU, and this process is also feeding a WebSocket event stream to whatever remotes
    // are open. `export_songs` has no such block and needs none; it is an order of magnitude
    // smaller.
    let filter_for_task = filter.clone();
    let name_for_task = query.name.clone();
    // `?locale=` decides, falling back to what the machine speaks. The machine has one locale and
    // this route reads it from one place, so when that becomes a setting this line is where it
    // arrives.
    let locale = query.locale(state.machine_locale());
    let pdf = tokio::task::spawn_blocking(move || {
        book::render(
            songs,
            &filter_for_task,
            version,
            name_for_task.as_deref(),
            locale,
        )
        .render()
    })
    .await
    .map_err(|error| ApiError::Internal(format!("rendering the song book: {error}")))?;

    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                content_disposition(&book_filename(&filter, &state.machine_name(), locale)),
            ),
            // **Not the bare catalog version, unlike `songs.export`.** That route's parameters are
            // a cursor and a page size over a stable set, so the version alone identifies what comes
            // back. Here the *body* changes with the query, and a bare version would let a client
            // that fetched `?language=pt` be handed the Portuguese book when it asked for English,
            // out of its own cache.
            //
            // **The locale is in here for that same reason and is easy to forget**, because it is
            // the one parameter that changes no song in the book — only every word around them. Two
            // books of identical rows with `ARTIST` and `ARTISTA` at the top are different bodies.
            //
            // `?name=` varies the body too, and is the first free text on this route — so it is
            // *hashed* rather than interpolated. See [`BookQuery::name_tag`]: a `"` in a name would
            // otherwise close the entity tag early.
            (
                header::ETAG,
                format!(
                    "\"{version}-{}-{}-{}-{}-{}\"",
                    filter.language.as_deref().unwrap_or("all"),
                    filter.package.as_deref().unwrap_or("all"),
                    // Safe to interpolate where the name is not: these are slugs `Tag::parse`
                    // produced, so neither a `"` nor a `-` can be in one.
                    //
                    // **Wrapped in the name of what the filter does with them**, because a version
                    // of this machine that read the same list a different way would render a
                    // different book from the same query — and `version` above is the catalog's,
                    // which an upgrade does not move. An entity tag that named only the words would
                    // hand that reader a body nothing in the request asked for.
                    if filter.tags.is_empty() {
                        "all".to_owned()
                    } else {
                        format!("any({})", filter.tags.join(","))
                    },
                    query.name_tag(),
                    locale.tag()
                ),
            ),
            (
                HeaderName::from_static("x-km-catalog-version"),
                version.to_string(),
            ),
        ],
        pdf,
    )
        .into_response())
}

/// What the browser will call the downloaded book.
///
/// **`KaraokeMachine - Living Room - Song Book (Portuguese).pdf`**, and the same string minus
/// whichever halves are absent: an unnamed machine drops the second segment, an unfiltered book the
/// parenthesis. A flat `songbook.pdf` is what a folder full of books off three machines looks like
/// when every one of them is called the same thing.
///
/// **The machine name is composed by [`book::book_name_for`]**, the one function that decides what
/// the masthead reads — so the file is named what the page it contains is named, and a machine still
/// called `KaraokeMachine` gets it once rather than twice.
///
/// **It follows `state.machine_name()` and not `?name=`**, which is the one part of this worth
/// arguing. A caller who renames a book for one download is titling *that copy*; the file on disk is
/// still a book out of this machine, and the invariant below is worth more than honouring the
/// override in two places.
///
/// **Every word of it is in the book's own locale**, which is one rule rather than two: the language
/// through [`book::named_language`], the same function the section dividers use, and the document's
/// own name through `book-filename`, the catalog's filename-cased twin of `book-title`. A book
/// headed `LISTA DE MÚSICAS` arriving in a file called `Song Book (Portuguese)` is the kind of small
/// lie that makes somebody check whether they downloaded the right thing, and half-translating it
/// was the same lie told quieter. `?locale=` is already in the `ETag`, so a cache cannot cross the
/// two.
///
/// **`Lista de Músicas` is not ASCII and that is [`content_disposition`]'s problem, not this
/// one's** — it returns both spellings, and the plain `filename` an old client reads carries
/// `M_sicas`. Which is the same bargain the machine name already made.
///
/// **Nothing a caller controls reaches the header, still.** The language is a code the compiled-in
/// ISO table knows and the locale is one of a closed set; a package id — arbitrary text out of a
/// manifest — is deliberately not in the name at all. What *is* new is the machine name, which an
/// owner types: see [`content_disposition`] for what that costs and how it is paid.
fn book_filename(filter: &BookFilter, machine: &str, locale: km_locale::Locale) -> String {
    let mut name = book::book_name_for(machine).unwrap_or_else(|| book::PRODUCT.to_owned());
    name.push_str(" - ");
    name.push_str(&book::messages(locale).msg("book-filename"));
    if let Some(code) = filter
        .language
        .as_deref()
        .and_then(km_kmpkg::Language::parse)
    {
        name.push_str(" (");
        name.push_str(&book::named_language(code, locale));
        name.push(')');
    }
    name.push_str(".pdf");
    name
}

/// The `Content-Disposition` value for a downloaded book, in both spellings.
///
/// **A header value is visible ASCII and a machine name is not**, which is the whole of why this
/// function exists. `Sala de Estar` is fine; `Salão` is a `HeaderValue` that will not build, and the
/// route would answer 500 for no reason a reader of the log could guess. Before the machine name was
/// in the filename every part of it came from a closed compiled-in set and the question could not
/// arise.
///
/// So RFC 6266's two-parameter form: `filename` carries an ASCII rendering for anything that reads
/// only that, and `filename*` carries the real one, percent-encoded UTF-8 per RFC 8187. Every
/// browser this machine is driven from prefers `filename*`; the fallback is what a script with a
/// naive parser gets, and `Salao` is a better answer for it than a 500.
fn content_disposition(filename: &str) -> String {
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_fallback(filename),
        rfc8187(filename)
    )
}

/// The `filename` half: printable ASCII, with anything else standing in as `_`.
///
/// Not a transliteration — `ç` becomes `_` and not `c`. A machine named in a script with no Latin
/// rendering at all would transliterate to nothing useful anyway, and `filename*` is what actually
/// gets used; this only has to be safe, unambiguous and non-empty.
fn ascii_fallback(filename: &str) -> String {
    filename
        .chars()
        .map(|c| match c {
            '"' | '\\' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect()
}

/// The `filename*` half: RFC 8187 percent-encoded UTF-8.
///
/// Hand-rolled rather than a dependency, for the reason `km-songbook` takes none: it is one table
/// and a loop, and the set is small and fixed. Everything outside RFC 8187's `attr-char` is escaped,
/// which is stricter than it has to be and cannot be wrong.
fn rfc8187(filename: &str) -> String {
    const ATTR_CHAR: &str = "!#$&+-.^_`|~";
    let mut out = String::with_capacity(filename.len());
    for byte in filename.bytes() {
        if byte.is_ascii_alphanumeric() || ATTR_CHAR.contains(byte as char) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `GET /api/v1/songs/{number}`
pub async fn get_song(
    State(state): State<ApiState>,
    Code(number): Code,
) -> ApiResult<Json<SongDto>> {
    let song = state
        .catalog()
        .song(number)?
        .ok_or_else(|| ApiError::not_found(format!("song {number}")))?;
    Ok(Json(SongDto::from(&song)))
}

/// `GET /api/v1/songs/{number}/lyrics`
pub async fn get_lyrics(
    State(state): State<ApiState>,
    Code(number): Code,
) -> ApiResult<Json<LyricsDto>> {
    let song = state
        .catalog()
        .load(number)?
        .ok_or_else(|| ApiError::not_found(format!("song {number}")))?;
    Ok(Json(LyricsDto::from_song(Some(number), &song)))
}

// -- queue ---------------------------------------------------------------------------------------

/// `GET /api/v1/queue`
pub async fn get_queue(State(state): State<ApiState>) -> Json<QueueDto> {
    Json(QueueDto::new(&state.controller().queue()))
}

/// `POST /api/v1/queue`
pub async fn add_to_queue(
    State(state): State<ApiState>,
    Body(body): Body<AddToQueueRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<AddedToQueueDto>)> {
    // Off the runtime: queueing onto an idle machine loads the song, which for a video is ffmpeg
    // opening a decoder. See `ops::off_runtime`.
    let singer = body.singer.clone();
    let number = body.number;
    let added = ops::off_runtime(&state, move |state| {
        ops::enqueue(state, number, singer.as_deref())
    })
    .await?;
    Ok((axum::http::StatusCode::CREATED, Json(added)))
}

/// `DELETE /api/v1/queue/{entry_id}`
pub async fn remove_from_queue(
    State(state): State<ApiState>,
    Segment(entry_id): Segment<u64>,
) -> ApiResult<Json<QueueDto>> {
    Ok(Json(ops::dequeue(&state, entry_id)?))
}

/// `POST /api/v1/queue/{entry_id}/move`
pub async fn move_in_queue(
    State(state): State<ApiState>,
    Segment(entry_id): Segment<u64>,
    Body(body): Body<MoveRequest>,
) -> ApiResult<Json<QueueDto>> {
    Ok(Json(ops::move_entry(&state, entry_id, body.to_index)?))
}

/// `DELETE /api/v1/queue`
pub async fn clear_queue(State(state): State<ApiState>) -> ApiResult<Json<QueueDto>> {
    Ok(Json(ops::clear_queue(&state)?))
}

// -- transport -----------------------------------------------------------------------------------

/// `POST /api/v1/transport/play`
pub async fn play(State(state): State<ApiState>) -> ApiResult<Json<StateDto>> {
    run_transport(&state, TransportCommand::Play).await
}

/// `POST /api/v1/transport/pause`
pub async fn pause(State(state): State<ApiState>) -> ApiResult<Json<StateDto>> {
    run_transport(&state, TransportCommand::Pause).await
}

/// `POST /api/v1/transport/skip`
pub async fn skip(State(state): State<ApiState>) -> ApiResult<Json<StateDto>> {
    run_transport(&state, TransportCommand::Skip).await
}

/// `POST /api/v1/transport/restart`
pub async fn restart(State(state): State<ApiState>) -> ApiResult<Json<StateDto>> {
    run_transport(&state, TransportCommand::Restart).await
}

/// `POST /api/v1/transport/stop`
pub async fn stop(State(state): State<ApiState>) -> ApiResult<Json<StateDto>> {
    run_transport(&state, TransportCommand::Stop).await
}

/// `POST /api/v1/transport/seek`
pub async fn seek(
    State(state): State<ApiState>,
    Body(body): Body<SeekRequest>,
) -> ApiResult<Json<StateDto>> {
    run_transport(&state, TransportCommand::Seek { ms: body.ms }).await
}

// -- settings ------------------------------------------------------------------------------------

/// `GET /api/v1/settings`
pub async fn get_settings(State(state): State<ApiState>) -> Json<SettingsDto> {
    Json(SettingsDto::from(state.controller().snapshot().settings))
}

/// `PUT /api/v1/settings`
pub async fn put_settings(
    State(state): State<ApiState>,
    Body(body): Body<SettingsPatchDto>,
) -> ApiResult<Json<SettingsDto>> {
    apply_settings(&state, body.to_patch())
}

// -- microphones ---------------------------------------------------------------------------------

/// `GET /api/v1/mics`
pub async fn get_mics(State(state): State<ApiState>) -> Json<MicsDto> {
    Json(MicsDto::new(&state.controller().mics()))
}

/// `PUT /api/v1/mics/{id}`
pub async fn put_mic(
    State(state): State<ApiState>,
    Segment(id): Segment<String>,
    Body(body): Body<MicPatchDto>,
) -> ApiResult<Json<MicsDto>> {
    state
        .controller()
        // See `ops::dequeue`: the refusal carries its own subject, so a future reason to refuse a
        // mic patch will no longer be reported as the microphone not existing.
        .update_mic(&id, &body.to_patch())?;
    let mics = MicsDto::new(&state.controller().mics());
    state
        .events()
        .publish(Event::MicsChanged { mics: mics.clone() });
    Ok(Json(mics))
}

// -- the audio output ----------------------------------------------------------------------------

/// `GET /api/v1/audio/outputs`
pub async fn get_audio_outputs(
    State(state): State<ApiState>,
) -> ApiResult<Json<crate::dto::AudioOutputsDto>> {
    let outputs = state.controller().audio_outputs()?;
    Ok(Json(crate::dto::AudioOutputsDto::from(&outputs)))
}

/// `GET /api/v1/audio/soundfont`
///
/// Which bank the instruments come from — the HTTP form of the `soundfont` line `--show-paths`
/// prints, and the only thing on this surface that can explain a machine playing sine waves.
pub async fn get_audio_soundfont(
    State(state): State<ApiState>,
) -> ApiResult<Json<crate::dto::SoundFontDto>> {
    let soundfont = state.controller().soundfont();
    Ok(Json(crate::dto::SoundFontDto::from(&soundfont)))
}

/// `GET /api/v1/audio/soundfonts`
///
/// Every bank that could be chosen. The companion to the route above, which says which one is
/// sounding: during an A/B through the debug slots those two genuinely disagree, and that is the
/// point of having both.
///
/// `?all=true` widens `offers` from the nine banks the machine offers to the whole catalog it
/// knows about, which is sixty-odd. Off by default, because the default caller is a phone.
pub async fn get_audio_soundfonts(
    State(state): State<ApiState>,
    Params(params): Params<crate::dto::SoundFontsParams>,
) -> ApiResult<Json<crate::dto::SoundFontsDto>> {
    let banks = state.controller().soundfonts(params.all.unwrap_or(false));
    Ok(Json(crate::dto::SoundFontsDto::from(&banks)))
}

/// `PUT /api/v1/admin/audio/soundfont`
///
/// Publishes `settings_changed`, unlike `PUT /audio/output` beside it — and the difference is the
/// whole reason this route needed thinking about rather than copying. The output device is set once
/// when the machine is installed; a bank is chosen while people are listening, takes effect on the
/// song already playing, and there may be a second phone open on the same page. A remote that had to
/// poll to find out its own machine had changed bank would be the one thing this feature cannot
/// afford, since the change is audible before it is visible.
pub async fn put_audio_soundfont(
    State(state): State<ApiState>,
    Body(body): Body<crate::dto::SoundFontRequest>,
) -> ApiResult<Json<crate::dto::SoundFontsDto>> {
    Ok(Json(ops::set_soundfont(&state, &body.id)?))
}

/// `POST /api/v1/admin/audio/soundfont/fetch`
///
/// Starts a download and answers with the list as it now stands, `fetching` filled in. **Answers
/// when the download has started, not when it has finished**: these banks run to a gigabyte, and a
/// request held open for that long is one every proxy and phone in the path would give up on.
/// Progress is read back from `GET /audio/soundfonts`.
///
/// Publishes no event. What changes while a download runs is a byte count, and a machine that
/// announced it four times a second to every connected phone would be spending the event stream on
/// the least interesting thing on it.
pub async fn post_audio_soundfont_fetch(
    State(state): State<ApiState>,
    Body(body): Body<crate::dto::SoundFontFetchRequest>,
) -> ApiResult<Json<crate::dto::SoundFontsDto>> {
    state.controller().fetch_soundfont(&body.id)?;
    // The shortlist, whatever width the list was asked at: what changed is `fetching`, and a caller
    // that wants the catalog back re-reads the route that takes the parameter.
    let banks = state.controller().soundfonts(false);
    Ok(Json(crate::dto::SoundFontsDto::from(&banks)))
}

/// `DELETE /api/v1/admin/audio/soundfonts/{id}`
///
/// Removes an installed bank and answers with the list as it now stands.
///
/// **The id goes in the path rather than a body**, which is the one place this route does not copy
/// its neighbors: `PUT` and `POST` above both take JSON. A `DELETE` with a body is the awkward
/// case — proxies and HTTP clients are entitled to drop it — and `DELETE /packages/{id}` beside it
/// already establishes what a delete-by-id looks like here.
///
/// Publishes `settings_changed` through [`ops::delete_soundfont`], because deleting the bank the
/// setting names falls back to the bundled one and that moves the level with it.
pub async fn delete_audio_soundfont(
    State(state): State<ApiState>,
    Segment(id): Segment<String>,
) -> ApiResult<Json<crate::dto::SoundFontsDto>> {
    Ok(Json(ops::delete_soundfont(&state, &id)?))
}

/// `PUT /api/v1/admin/audio/output`
///
/// Publishes no event, deliberately. `settings_changed` carries the performance knobs a remote
/// redraws four times a second, and the output device is not one of them — it is set once when the
/// machine is installed. The response is the whole answer.
pub async fn put_audio_output(
    State(state): State<ApiState>,
    Body(body): Body<crate::dto::AudioOutputRequest>,
) -> ApiResult<Json<crate::dto::AudioOutputsDto>> {
    let outputs = state.controller().set_audio_output(&body.id)?;
    Ok(Json(crate::dto::AudioOutputsDto::from(&outputs)))
}

/// `PUT /api/v1/admin/audio/level`
///
/// Publishes no event, for the reason above it: this is the gain between the machine and the
/// amplifier, set once when a room is balanced.
///
/// **Off the runtime**, unlike its neighbour, because reaching a level means opening the operating
/// system's mixer and that is blocking I/O on a thread the request shares with everything else the
/// machine is answering. See the invariant in `CONTRIBUTING.md`.
pub async fn put_audio_level(
    State(state): State<ApiState>,
    Body(body): Body<crate::dto::AudioLevelRequest>,
) -> ApiResult<Json<crate::dto::AudioOutputsDto>> {
    // Hundredths of a decibel crossing into the domain, where the type is comparable. A request
    // carrying a NaN would otherwise arrive as an arbitrary integer, so it is answered as the
    // quietest the control goes and clamped from there.
    let db_centi = if body.db.is_nan() {
        i32::MIN
    } else {
        (body.db * 100.0)
            .round()
            .clamp(f32::from(i16::MIN) * 100.0, f32::from(i16::MAX) * 100.0) as i32
    };
    ops::off_runtime(&state, move |state| {
        let outputs = state.controller().set_output_level(db_centi)?;
        Ok(Json(crate::dto::AudioOutputsDto::from(&outputs)))
    })
    .await
}

// -- wallpapers ----------------------------------------------------------------------------------

/// `GET /api/v1/wallpapers`
pub async fn get_wallpapers(State(state): State<ApiState>) -> Json<crate::dto::WallpapersDto> {
    let controller = state.controller();
    Json(crate::dto::WallpapersDto::with_pictures(
        &controller.wallpapers(),
        controller.wallpaper_pictures(),
    ))
}

/// `DELETE /api/v1/admin/wallpapers/{id}`
///
/// **On the plural, beside `GET` and the upload**, which is the rule the SoundFont routes settled:
/// removing hangs off the collection. Note the trap that rule exists to avoid is live here —
/// `/wallpapers/next` is a static segment sitting exactly where `{id}` goes — and what keeps them
/// apart is the method rather than the path, since `next` is a `POST` and this is a `DELETE`. A
/// picture named `next.jpg` therefore stays deletable, which a test pins because the failure would
/// be a 405 and silent.
pub async fn delete_wallpaper(
    State(state): State<ApiState>,
    Segment(id): Segment<String>,
) -> ApiResult<Json<crate::dto::WallpapersDto>> {
    state.controller().delete_wallpaper(&id)?;
    let controller = state.controller();
    let wallpapers = controller.wallpapers();
    // The picture on screen changes as a result — `delete_wallpaper` asks for the next one — so this
    // is the same event `next_wallpaper` publishes, for the same reason: every phone holding the
    // page learns, not only whoever pressed Remove.
    state.events().publish(Event::WallpaperChanged {
        current: wallpapers.current.clone(),
    });
    Ok(Json(crate::dto::WallpapersDto::with_pictures(
        &wallpapers,
        controller.wallpaper_pictures(),
    )))
}

/// `POST /api/v1/wallpapers/next`
pub async fn next_wallpaper(
    State(state): State<ApiState>,
) -> ApiResult<Json<crate::dto::WallpapersDto>> {
    state.controller().next_wallpaper()?;
    let wallpapers = state.controller().wallpapers();
    state.events().publish(Event::WallpaperChanged {
        current: wallpapers.current.clone(),
    });
    Ok(Json(crate::dto::WallpapersDto::from(&wallpapers)))
}

// -- demo mode -----------------------------------------------------------------------------------

/// `GET /api/v1/demo`
///
/// Public, unlike its `PUT` twin: a remote has to know whether what it can hear is a demo before it
/// can tell a singer that skipping — not waiting — is what lets them sing.
pub async fn get_demo(State(state): State<ApiState>) -> Json<crate::dto::DemoDto> {
    Json(crate::dto::DemoDto::from(&state.controller().demo()))
}

/// `PUT /api/v1/admin/demo`
///
/// Turns demo mode on or off, for this run or for good — see [`SetDemoRequest::persist`]. Admin by
/// default, because it makes the machine play music by itself in somebody's house -- which under
/// the prefix rule is said by the path: `PUT /api/v1/admin/demo`. See [`crate::routes::ADMIN_PREFIX`].
///
/// **Answers with what the machine now says rather than echoing what was asked**, the same
/// distinction the uploads switch draws for the same reason: a client that turned demo mode on
/// and read `enabled: true` back out of the controller has been told it took effect, not merely that
/// the request parsed. It is also the only way to learn `starts_in_secs` without a second request.
///
/// Synchronous, unlike the routes that can load a song. This only moves a deadline; the machine's
/// own poll thread notices within [`POLL_INTERVAL`] and starts the song there, off the runtime by
/// construction.
///
/// [`POLL_INTERVAL`]: https://docs.rs/karaokemachine
pub async fn put_demo(
    State(state): State<ApiState>,
    Body(body): Body<SetDemoRequest>,
) -> ApiResult<Json<crate::dto::DemoDto>> {
    let demo = state.controller().set_demo(body.enabled, body.persist)?;
    Ok(Json(crate::dto::DemoDto::from(&demo)))
}

/// `PUT /api/v1/admin/demo/delay`
///
/// Sets how long the machine waits before performing for itself, and writes it down.
///
/// **A route of its own rather than a field on [`put_demo`], and `persist` is the whole reason** —
/// the argument is in [`Controller::set_demo_delay`]. Admin for its neighbour's reason and said by
/// the path, and admin for one of its own on top: the smallest delay is zero, so somebody who found
/// the machine on the network could otherwise arrange for a house they are not in to start singing
/// the moment it goes quiet, which is `PUT /admin/demo` reached by another door.
///
/// Answers with the machine's own state, as its neighbour does and for the same reason — and here
/// the answer carries the number that was actually stored, so a form that sent a value it should not
/// have does not have to guess what happened to it.
///
/// Refuses with 400 `rejected` above the cap. Synchronous: like `put_demo` this only moves a
/// deadline.
///
/// [`Controller::set_demo_delay`]: crate::machine::Controller::set_demo_delay
pub async fn put_demo_delay(
    State(state): State<ApiState>,
    Body(body): Body<SetDemoDelayRequest>,
) -> ApiResult<Json<crate::dto::DemoDto>> {
    let demo = state.controller().set_demo_delay(body.delay_secs)?;
    Ok(Json(crate::dto::DemoDto::from(&demo)))
}

/// `POST /api/v1/demo/start`
///
/// Plays one song the machine chooses, now, whether or not demo mode is on. Bodyless, like
/// [`next_wallpaper`]: there is nothing to say beyond the press.
///
/// **Public where its `PUT` neighbor is admin**, and the whole of that argument is one song against
/// a mode, and under the prefix rule the two paths say it themselves: `POST /api/v1/demo/start`
/// against `PUT /api/v1/admin/demo`. Refuses with 409 `unavailable` when something is
/// loaded, when the queue is not empty, or when the machine has no sound, so the sentence a remote
/// puts on screen is the machine's own.
///
/// Synchronous for [`put_demo`]'s reason and one more. It sets a one-shot flag and the machine's poll
/// thread starts the song a moment later, on the one thread that is allowed to start demo songs —
/// which is what makes this both free of [`ops::off_runtime`] and unable to race that thread into
/// starting two.
///
/// **The answer describes the machine as it stands, not as it will be in fifty milliseconds.**
/// `playing` is false in it and `starts_in_secs` is `None`, and neither is a fault: what a caller
/// wants from this body is `enabled`, which is the difference between the one song it just asked for
/// and a machine that will now keep going by itself.
pub async fn start_demo(State(state): State<ApiState>) -> ApiResult<Json<crate::dto::DemoDto>> {
    Ok(Json(crate::dto::DemoDto::from(&ops::start_demo(&state)?)))
}

// -- packages ------------------------------------------------------------------------------------

/// `GET /api/v1/packages`
pub async fn get_packages(State(state): State<ApiState>) -> ApiResult<Json<PackagesDto>> {
    let packages = state.catalog().packages()?;
    Ok(Json(PackagesDto {
        packages: packages
            .iter()
            .map(|package| {
                PackageDto::new(
                    package,
                    state.catalog().why_not_removable(package).is_none(),
                )
            })
            .collect(),
        song_count: state.catalog().song_count()?,
        problems: state
            .catalog()
            .package_problems()
            .iter()
            .map(crate::dto::PackageProblemDto::from)
            .collect(),
    }))
}

/// `POST /api/v1/admin/packages/rescan`
///
/// Reads the packages folders again, so a `.kmpkg` copied in by hand takes effect without a
/// restart. `spawn_blocking` for the same reason [`install_package`] uses it, and more so: a rescan
/// is N installs.
///
/// **No event is published.** A catalog change is learned from `catalog_version`, which is
/// exactly what a mirror already polls; the event stream is about a performance.
pub async fn rescan_packages(
    State(state): State<ApiState>,
) -> ApiResult<Json<crate::dto::RescanReportDto>> {
    let catalog = state.catalog_handle();
    let report = tokio::task::spawn_blocking(move || catalog.rescan())
        .await
        .map_err(|error| CatalogError::Failed(format!("the rescan did not finish: {error}")))??;
    Ok(Json(crate::dto::RescanReportDto::from(&report)))
}

/// `POST /api/v1/admin/packages`
pub async fn install_package(
    State(state): State<ApiState>,
    Body(body): Body<InstallRequest>,
) -> ApiResult<Json<InstallReportDto>> {
    let path = PathBuf::from(&body.path);
    // Off the runtime, and the only `spawn_blocking` in the workspace. Installing a package indexes
    // every song into SQLite and rebuilds the search index, which for a four-thousand-song package
    // is seconds. Held on a tokio worker that stalls the API, the singer's remote at `/` and the
    // event stream for the whole of it — the machine looking broken while it is working.
    //
    // It does **not** read the media, even though the media now lives in the package: installing
    // reads the manifest and the MIDI, and a video is opened at play time and never before. So this
    // is bounded by the song count rather than by the package's size, which is worth knowing before
    // anybody reaches for a progress report it does not need.
    let catalog = state.catalog_handle();
    let report = tokio::task::spawn_blocking(move || catalog.install_copied(&path))
        .await
        .map_err(|error| CatalogError::Failed(format!("the install did not finish: {error}")))??;
    Ok(Json(InstallReportDto::from(&report)))
}

/// `DELETE /api/v1/admin/packages/{id}`
///
/// **This deletes the `.kmpkg` file.** Admin by default for that reason, and it is why the pages
/// that offer it ask first — the route does not, because a 200 is a 200 and a JSON caller cannot be
/// asked a question.
///
/// **Three failures, three answers.** A blanket `|_| not_found(…)` answers `404 package '<id>'`
/// for a package the machine refuses to delete and for a disk that will not let go of the file —
/// the one status that says "it is not here", false in both cases, and it throws away the machine's
/// own sentence explaining why. `From<CatalogError>` already turns `Rejected` into a 400 and
/// `Failed` into a 500, so only the `NotFound` arm needs spelling out, and it is spelled out to
/// keep the id in the sentence.
///
/// So a client treating 404 as "already gone, fine" sees a 400 or a 500 when the delete genuinely
/// failed. That is the point of it.
pub async fn uninstall_package(
    State(state): State<ApiState>,
    Segment(id): Segment<String>,
) -> ApiResult<Json<UninstallDto>> {
    // No `map_err` here any more: `CatalogError::NotFound` carries what was missing, so the
    // subject reaches the client from wherever the refusal was raised rather than being put back
    // by hand at the two call sites somebody happened to notice.
    let songs_removed = state.catalog().uninstall(&id)?;
    Ok(Json(UninstallDto {
        package_id: id,
        songs_removed,
    }))
}

/// `PUT /api/v1/admin/packages/{id}/bank`
///
/// Moves a package to another block of a thousand. **Admin by default**, and refused with a 409
/// while anything is playing or queued — every song in the package changes its number, so a queue
/// holding the old ones would be a queue of songs that no longer exist.
///
/// **Bank 0 is refused here as well as in the catalog**, and the sentence is the point: the catalog
/// holds the invariant, and this is where somebody who typed a number reads why it was not taken.
/// A 400 and no error code, per `A refusal travels as a code` — this route is the owner's.
pub async fn set_package_bank(
    State(state): State<ApiState>,
    Segment(id): Segment<String>,
    Body(body): Body<BankRequest>,
) -> ApiResult<Json<BankDto>> {
    if body.bank == 0 {
        return Err(ApiError::BadRequest(
            "bank 0 is the machine's own and cannot hold a package; banks run from 1".to_owned(),
        ));
    }
    if body.bank > km_songcode::MAX_BANK {
        return Err(ApiError::BadRequest(format!(
            "bank {} is above the highest bank, {}",
            body.bank,
            km_songcode::MAX_BANK
        )));
    }
    // See `uninstall_package`: the subject travels with the refusal now.
    let songs = state.catalog().set_package_bank(&id, body.bank)?;
    // No event, for the same reason installing and uninstalling publish none: a catalog change is
    // learned from `catalog_version`, which this bumps, and the event stream is about a
    // performance rather than about what songs exist.
    Ok(Json(BankDto {
        package_id: id,
        bank: body.bank,
        songs_renumbered: songs,
    }))
}

// -- admin ---------------------------------------------------------------------------------------

/// `POST /api/v1/admin/login`
pub async fn login(
    State(state): State<ApiState>,
    peer: Peer,
    Body(body): Body<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let grant = state.auth().login(peer.rate_limit_key(), &body.password)?;
    tracing::info!(peer = ?peer.0, "admin logged in");
    Ok(Json(LoginResponse {
        token: grant.token,
        expires_in_secs: grant.expires_in_secs,
    }))
}

/// `POST /api/v1/admin/logout`
///
/// **This clears the caller's cookie and nothing else, and the wording says so.** A token is an HMAC
/// over the stored password hash and the session epoch rather than a row in a table, so there is no
/// row to delete: the browser stops presenting it, and a copy taken off the wire stays valid until it
/// expires or `POST /api/v1/admin/sessions/reset` moves the epoch. Pretending otherwise here would be
/// worse than the limitation.
pub async fn logout(State(_state): State<ApiState>) -> Json<ErrorDto> {
    // Always 200. Reaching this route already required a valid token.
    Json(ErrorDto::new(
        "ok",
        "logged out on this device; use /admin/sessions/reset to end every session".to_owned(),
    ))
}

/// `POST /api/v1/admin/sessions/reset`
///
/// Sign out everywhere. Bumps the session epoch, which every outstanding token is signed against, so
/// all of them stop verifying at once — including the one that made this call.
///
/// **Deliberately not folded into changing the password.** They are different acts: an owner who
/// wants every phone logged out should not have to pick a new password and then tell the house what
/// it is.
pub async fn reset_sessions(State(state): State<ApiState>) -> ApiResult<Json<ErrorDto>> {
    let next = state.session_epoch().saturating_add(1);
    // Persist first, then move the running value — the order `set_admin_password` uses, so a failed
    // write never leaves a machine enforcing an epoch its settings file does not hold.
    state.controller().set_session_epoch(next)?;
    state.set_session_epoch(next);
    tracing::info!(epoch = next, "every admin session was ended");
    Ok(Json(ErrorDto::new(
        "ok",
        "every admin session has been ended; log in again".to_owned(),
    )))
}

// -- power ---------------------------------------------------------------------------------------

/// How long the answer is given to leave before the machine acts on it.
///
/// **Not the mechanism that makes the response arrive** — `serve_with_shutdown` is graceful and the
/// body is written before any of this reaches the socket layer. It is the margin that makes the
/// question uninteresting, and it is short because somebody is standing in front of the box: a
/// person who has just pressed *Shut down* and sees nothing happen presses it again.
const POWER_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

/// `GET /api/v1/admin/power`
///
/// What this machine can do about its own power. Mounted only where it can do anything at all, so
/// the interesting answer is the **404** a machine without power control gives — see
/// [`crate::power`] for why that is the honest shape rather than a route that always refuses.
///
/// Under `/admin/` with its two siblings rather than public beside `/debug`, and the difference is
/// worth naming: whether debugging is on is a fact a curation tool needs before it offers a Play
/// button, where whether this box can be switched off remotely is not something the LAN needs to be
/// told before it has a password.
pub async fn get_power(State(state): State<ApiState>) -> ApiResult<Json<PowerDto>> {
    power_of(&state)?;
    Ok(Json(PowerDto {
        shutdown: true,
        restart: true,
    }))
}

/// `POST /api/v1/admin/power/off`
///
/// Turns the box off. The same thing the physical power button does, reached from a phone.
///
/// **202 rather than 200, and it means it**: the machine has not powered off when this is written,
/// and the caller will get no later word from a box that is going dark. The work is deliberately
/// *not* awaited — see [`crate::power::Power::shut_down`] for why nothing here also stops the
/// machine.
pub async fn power_off(State(state): State<ApiState>) -> ApiResult<Response> {
    act_on_power(&state, PowerAction::Off)
}

/// `POST /api/v1/admin/power/restart`
///
/// Ends the application so that whatever supervises it starts it again. The box stays on.
///
/// This is the answer to *"I changed a setting that is only read when the machine starts"* — the
/// same restart [`put_debug`] tells an owner they need, on a box with no keyboard.
pub async fn power_restart(State(state): State<ApiState>) -> ApiResult<Response> {
    act_on_power(&state, PowerAction::Restart)
}

/// Which of the two a request asked for.
#[derive(Debug, Clone, Copy)]
enum PowerAction {
    Off,
    Restart,
}

/// This machine's power control, or the 404 that says it has none.
///
/// `UnknownEndpoint` rather than `NotFound`: the two are distinguished throughout this crate by
/// whether the *path* exists or the *thing* does, and here it is the path. A machine with no power
/// control never mounts these routes at all, so the only way to reach this is a race between a
/// router built without one and a request already in flight — but the answer must be the same one
/// the unmounted router gives, or a client would see two different 404s for one condition.
fn power_of(state: &ApiState) -> ApiResult<&dyn crate::power::Power> {
    state
        .power()
        .ok_or_else(|| ApiError::UnknownEndpoint("this machine has no power control".to_owned()))
}

/// Answers first, then does it.
///
/// The spawned task is what makes the two orderings independent: whatever the action does to this
/// process — and shutting down eventually ends it — happens after the response has been handed to
/// the connection.
fn act_on_power(state: &ApiState, action: PowerAction) -> ApiResult<Response> {
    // Resolved before answering, so a machine with no power control answers 404 rather than 202 and
    // then silently doing nothing. This is the only part that cannot be deferred.
    power_of(state)?;
    let (said, log) = match action {
        PowerAction::Off => (
            "the machine is shutting down",
            "powering the machine off at the owner's request",
        ),
        PowerAction::Restart => (
            "the machine is restarting",
            "restarting the machine at the owner's request",
        ),
    };
    tracing::warn!("{log}");

    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(POWER_GRACE).await;
        // On a blocking thread because the real implementation forks a process, which is exactly
        // what a runtime worker must not sit in. Nothing awaits the outcome — by the time there is
        // one there may be no process left to hear it — so a failure is reported to the journal,
        // which on the appliance is the only place anybody would look for it anyway.
        let outcome = tokio::task::spawn_blocking(move || {
            let power = state
                .power()
                .expect("power control cannot be uninstalled once set");
            match action {
                PowerAction::Off => power.shut_down(),
                PowerAction::Restart => power.restart_application(),
            }
        })
        .await;
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(%error, "the machine could not act on a power request")
            }
            Err(error) => tracing::error!(%error, "a power request did not run"),
        }
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(ErrorDto::new("ok", said.to_owned())),
    )
        .into_response())
}

// -- debugging mode ------------------------------------------------------------------------------

/// `GET /api/v1/debug`
///
/// Public: reporting whether debugging is on is a different act from turning it on, the same split
/// `/demo` makes. A curation tool asks this before it offers a Play button.
pub async fn get_debug(State(state): State<ApiState>) -> Json<DebugDto> {
    Json(DebugDto {
        enabled: state.config().debug_enabled,
        stored: state.controller().developer_switches().debug,
    })
}

/// `GET /api/v1/dev-remote`
///
/// Whether the development console is asked for, and whether it is actually up. Public for
/// `GET /debug`'s reason: reporting the state of a surface is not the same act as changing it, and
/// the two admin pages both need the read before anybody has typed a password.
pub async fn get_dev_remote(State(state): State<ApiState>) -> Json<crate::dto::DevRemoteDto> {
    Json(crate::dto::DevRemoteDto {
        enabled: state.controller().developer_switches().dev_remote,
        served: crate::routes::dev_console_served(state.config()),
    })
}

/// `GET /api/v1/performance`
///
/// Whether the frame-statistics panel is on the machine's own screen. Public, like the two switches
/// beside it: reporting a state is not changing it.
pub async fn get_performance(State(state): State<ApiState>) -> Json<crate::dto::PerformanceDto> {
    Json(crate::dto::PerformanceDto {
        enabled: state.controller().performance_overlay(),
    })
}

/// `PUT /api/v1/admin/performance`
///
/// Putting the frame-statistics panel on the machine's screen. **Effective on the next frame**,
/// which makes it the one switch on these pages that does not wait for a restart — and the reply is
/// therefore the machine's own state rather than an echo, because for once those agree.
///
/// Under `/admin/` because it draws over the picture in front of a room. That it changes nothing
/// about what the machine *does* is what earned `F12` its key without reopening the front-end scope;
/// it is not an argument for a stranger on the LAN being able to do it.
pub async fn put_performance(
    State(state): State<ApiState>,
    Body(body): Body<crate::dto::PerformanceRequest>,
) -> ApiResult<Json<crate::dto::PerformanceDto>> {
    state.controller().set_performance_overlay(body.enabled)?;
    tracing::info!(
        enabled = body.enabled,
        "the frame statistics panel was switched"
    );
    Ok(Json(crate::dto::PerformanceDto {
        enabled: state.controller().performance_overlay(),
    }))
}

/// `PUT /api/v1/admin/dev-remote`
///
/// Turning the development console on or off. Under `/admin/` because it is an owner's act, and
/// because what it opens is the entire API again with no password on any of it.
///
/// **It takes effect on the next start**, exactly as `PUT /admin/debug` does and for the same
/// reason: the routes are mounted at router-construction time so that a machine with the console off
/// answers 404 — genuinely not there — rather than carrying a disabled surface.
///
/// **It does not turn debugging on as a side effect.** Two switches is the decision, and a switch
/// that silently flipped the other one would be one switch wearing a disguise. The reply's `served`
/// field is what says whether anything actually happened, and both pages draw the sentence from it.
pub async fn put_dev_remote(
    State(state): State<ApiState>,
    Body(body): Body<crate::dto::DevRemoteRequest>,
) -> ApiResult<Json<crate::dto::DevRemoteDto>> {
    state.controller().set_dev_remote_enabled(body.enabled)?;
    tracing::info!(
        enabled = body.enabled,
        "the development console switch was changed"
    );
    Ok(Json(crate::dto::DevRemoteDto {
        enabled: body.enabled,
        // Unchanged by this call: the console is mounted or not for the life of the process.
        served: crate::routes::dev_console_served(state.config()),
    }))
}

/// `PUT /api/v1/admin/debug`
///
/// Turning debugging on or off. Under `/admin/` because it is an owner's act, and because what it
/// opens is a route that plays any file the machine can read.
///
/// **It takes effect on the next start, and the response says so.** The two debug routes are mounted
/// at router-construction time rather than checked per request, so that a machine with debugging off
/// answers 404 — genuinely not there — instead of carrying a disabled handler. That is the trade:
/// a clearer surface for a restart.
pub async fn put_debug(
    State(state): State<ApiState>,
    Body(body): Body<DebugRequest>,
) -> ApiResult<Json<DebugDto>> {
    state.controller().set_debug_enabled(body.enabled)?;
    tracing::info!(enabled = body.enabled, "debugging mode was changed");
    Ok(Json(DebugDto {
        // The running value, which this call did not change and cannot.
        enabled: state.config().debug_enabled,
        stored: body.enabled,
    }))
}

// -- debug ---------------------------------------------------------------------------------------

/// `POST /api/v1/debug/play-file`
///
/// The debug path from the brief: load one MIDI file directly, bypassing the catalog. Which
/// directories are allowed is the controller's decision, not this crate's — only `km-app` knows
/// where the operator keeps their files.
pub async fn play_file(
    State(state): State<ApiState>,
    Body(body): Body<PlayFileRequest>,
) -> ApiResult<Json<StateDto>> {
    // Off the runtime, for `add_to_queue`'s reason and more of it: this reaches the same
    // `play_path` as an audition does, which parses a MIDI file, opens an ffmpeg decoder or reads
    // both halves of an MP3+G pair before it returns. See `ops::off_runtime`.
    let path = PathBuf::from(&body.path);
    let PlayFileRequest {
        fixes,
        title,
        artist,
        transpose,
        melody,
        lyrics,
        lyrics_hidden,
        ..
    } = body;
    if let Some(Some(channel)) = melody {
        melody_channel_in_range(channel)?;
    }
    ops::off_runtime(&state, move |state| {
        state.controller().play_file(
            &path,
            &Audition {
                title: title.as_deref(),
                artist: artist.as_deref(),
                transpose,
                fixes: fixes.as_deref(),
                melody,
                lyrics: lyrics.as_ref(),
                lyrics_hidden,
            },
        )?;
        Ok(())
    })
    .await?;
    let snapshot = state.controller().snapshot();
    if let Some(now) = &snapshot.now_playing {
        state.events().publish(Event::SongStarted {
            now_playing: crate::dto::NowPlayingDto::from(now),
        });
    }
    Ok(Json(StateDto::from(&snapshot)))
}

/// The largest song this endpoint will take.
///
/// A MIDI is kilobytes and an MP3+G pair is single-digit megabytes, so this number is here for the
/// third kind. A karaoke video at the profile packaging already enforces — H.264, 1080p30, in MP4 —
/// runs to a few hundred megabytes at the long end, and a limit that refuses one is a limit that
/// makes this endpoint useless for exactly the songs hardest to judge without hearing them.
pub const MAX_AUDITION_BYTES: usize = 1024 * 1024 * 1024;

/// The field carrying the name both halves of the song will be staged under.
pub const STEM_FIELD: &str = "stem";

/// The optional text part naming the corrections to play the upload with, as a JSON array.
///
/// A part rather than a second endpoint, and optional rather than required, because an older
/// curation tool sends nothing and must go on working: the parts loop ignores what it does not
/// recognize, so this costs a client that does not send it exactly nothing.
pub const FIXES_FIELD: &str = "fixes";

/// The optional text part naming the title to show, in place of the file's own.
pub const TITLE_FIELD: &str = "title";

/// The optional text part naming the performer to show, in place of the file's own.
pub const ARTIST_FIELD: &str = "artist";

/// The optional text part naming the key to play in, in semitones.
///
/// A value that will not read as a number is ignored rather than refused, unlike the corrections
/// beside it: a transposition that failed to arrive is a song in the wrong key, which anybody
/// listening for a key hears immediately, where a correction that failed to arrive is a difference
/// they may not be able to place.
pub const TRANSPOSE_FIELD: &str = "transpose";

/// The optional text part naming the melody channel: `none`, or a 0-based channel number.
///
/// Refused when it will not read, like the corrections and unlike the key: a melody channel that
/// failed to arrive is a guide-melody toggle missing or silencing the wrong part, which is hard to
/// tell from the curator's choice being wrong.
pub const MELODY_FIELD: &str = "melody";

/// The optional text part carrying an UltraStar song's words, as the JSON timeline a package stores.
///
/// Sent beside the song's MP3, because the machine never reads an UltraStar file. Refused when it
/// will not read, for the corrections' reason: a song playing with no words looks like a fault in
/// the song rather than in the request.
pub const LYRICS_FIELD: &str = "lyrics";

/// The optional text part saying whether to draw the song's words: `true` or `false`.
///
/// Refused when it will not read, like the corrections and the melody channel: a curator sends this
/// having just disagreed with what the machine would measure, and a value that failed to arrive
/// shows them the answer they were overruling while telling them it is theirs.
pub const LYRICS_HIDDEN_FIELD: &str = "lyrics_hidden";

/// Refuses a melody channel outside the sixteen a MIDI file has.
fn melody_channel_in_range(channel: u8) -> ApiResult<()> {
    if usize::from(channel) < km_fixes::CHANNELS {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "melody channel {channel} is not one of the {} a MIDI file has",
            km_fixes::CHANNELS
        )))
    }
}

/// Reads [`MELODY_FIELD`]'s spelling: `none` for no melody channel, else a channel number.
fn parse_melody_field(text: &str) -> ApiResult<Option<u8>> {
    let text = text.trim();
    if text == "none" {
        return Ok(None);
    }
    let channel = text.parse::<u8>().map_err(|_| {
        ApiError::BadRequest(format!(
            "the {MELODY_FIELD} is neither `none` nor a channel number: {text}"
        ))
    })?;
    melody_channel_in_range(channel)?;
    Ok(Some(channel))
}

/// The longest staged stem, in characters.
///
/// Windows keeps `MAX_PATH` at 260 for a path that is not `\\?\`-prefixed, and the data directory
/// plus a staging folder has already spent some of it. A corpus filename is nowhere near this long;
/// a hostile one is as long as it likes.
const MAX_STEM_CHARS: usize = 80;

/// The most files one audition may carry: an MP3+G song is a pair, and nothing here is a trio.
const MAX_AUDITION_FILES: usize = 2;

/// `POST /api/v1/debug/play-upload`
///
/// [`play_file`]'s answer for a curator whose machine is somewhere else. That one names a path and
/// the machine opens it **on its own disk**, which is right on one box and meaningless on two: the
/// folder a corpus is being curated from does not exist on the appliance under the television.
///
/// So the body is the song itself. It is staged in a folder the controller opens and then played
/// through the ordinary loose-file path, which is what keeps every song kind working without a
/// second copy of the logic that chooses between them — the video decoder wants bytes it can seek,
/// and an MP3+G song wants its partner beside it, and a folder on disk is what gives both.
///
/// **The name comes from a form field and not from the parts' filenames**, which is the part worth
/// reading twice. A multipart filename is a header the client wrote, so it arrives carrying every
/// one of `..`, a separator, a Windows-illegal character and a trailing space, and its encoding
/// across the wire is not something this can rely on. The stem arrives in the body as ordinary
/// UTF-8, is reduced to something safe here, and is then given to **both** halves of a pair. That
/// last part is not tidiness: `km_kmpkg::pair_for` documents a measured pair whose stem ends in a
/// space, which Win32 strips from a path component — two halves named from two filenames can end up
/// unable to find each other, and two named from one stem cannot.
///
/// `stem` comes before the files, so nothing has to be buffered while the name is still unknown.
pub async fn play_upload(
    State(state): State<ApiState>,
    form: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<StateDto>> {
    // Axum's own rejection is plain text with no code in it, and every other failure on this surface
    // carries one. A client that got the content type wrong should read the same shape as a client
    // that got the song wrong.
    let mut form =
        form.map_err(|rejection| ApiError::BadRequest(rejection.body_text().to_string()))?;

    // Refuses here when the machine does not take uploads, in its own words and naming its own
    // setting — the same shape as the allowed-folders refusal `play_file` gives.
    //
    // Off the runtime, and this one is the least obvious of the three: opening an audition creates
    // the folder and then *sweeps* the older ones, which is a `read_dir` and a `remove_dir_all`
    // over staging trees that hold whole videos. All of that ran on a tokio worker before a single
    // byte of the upload had arrived.
    let staged = Staged::new(
        ops::off_runtime(&state, |state| Ok(state.controller().open_audition()?)).await?,
    );

    let mut stem: Option<String> = None;
    let mut fixes: Option<Vec<km_fixes::Fix>> = None;
    // Both left absent when a part is blank rather than stored as an empty string: a curation tool
    // with nothing typed in the box has said nothing, and the file's own name is then the honest
    // answer rather than a blank line on a television.
    let mut title: Option<String> = None;
    let mut artist: Option<String> = None;
    let mut transpose: Option<i8> = None;
    let mut melody: Option<Option<u8>> = None;
    let mut lyrics: Option<km_song::LyricTimeline> = None;
    let mut lyrics_hidden: Option<bool> = None;
    let mut names: Vec<String> = Vec::new();
    let mut total: usize = 0;

    while let Some(mut field) = form
        .next_field()
        .await
        .map_err(|error| multipart_failure(&error, MAX_AUDITION_BYTES))?
    {
        // A part with no filename is not a file. The stem is the one wanted; anything else is
        // ignored rather than refused, so a client may send ordinary form fields beside the song.
        if field.file_name().is_none() {
            if field.name() == Some(STEM_FIELD) {
                let text = field.text().await.map_err(|error| {
                    ApiError::BadRequest(format!(
                        "the {STEM_FIELD} could not be read: {}",
                        error.body_text()
                    ))
                })?;
                stem = Some(safe_stem(&text)?);
            } else if field.name() == Some(FIXES_FIELD) {
                let text = field.text().await.map_err(|error| {
                    ApiError::BadRequest(format!(
                        "the {FIXES_FIELD} could not be read: {}",
                        error.body_text()
                    ))
                })?;
                // Refused rather than ignored. A list that will not read is a curator about to be
                // told a song sounds like this with their corrections on, while hearing it with
                // them off — and a silent wrong answer is the failure this whole control exists to
                // let somebody check.
                fixes = Some(serde_json::from_str(&text).map_err(|error| {
                    ApiError::BadRequest(format!("the {FIXES_FIELD} are not a list: {error}"))
                })?);
            } else if field.name() == Some(TITLE_FIELD) {
                title = field
                    .text()
                    .await
                    .ok()
                    .filter(|text| !text.trim().is_empty());
            } else if field.name() == Some(ARTIST_FIELD) {
                artist = field
                    .text()
                    .await
                    .ok()
                    .filter(|text| !text.trim().is_empty());
            } else if field.name() == Some(TRANSPOSE_FIELD) {
                transpose = field
                    .text()
                    .await
                    .ok()
                    .and_then(|text| text.trim().parse::<i8>().ok());
            } else if field.name() == Some(MELODY_FIELD) {
                let text = field.text().await.map_err(|error| {
                    ApiError::BadRequest(format!(
                        "the {MELODY_FIELD} could not be read: {}",
                        error.body_text()
                    ))
                })?;
                melody = Some(parse_melody_field(&text)?);
            } else if field.name() == Some(LYRICS_FIELD) {
                let text = field.text().await.map_err(|error| {
                    ApiError::BadRequest(format!(
                        "the {LYRICS_FIELD} could not be read: {}",
                        error.body_text()
                    ))
                })?;
                lyrics = Some(serde_json::from_str(&text).map_err(|error| {
                    ApiError::BadRequest(format!("the {LYRICS_FIELD} are not a timeline: {error}"))
                })?);
            } else if field.name() == Some(LYRICS_HIDDEN_FIELD) {
                let text = field.text().await.map_err(|error| {
                    ApiError::BadRequest(format!(
                        "the {LYRICS_HIDDEN_FIELD} could not be read: {}",
                        error.body_text()
                    ))
                })?;
                lyrics_hidden = Some(text.trim().parse::<bool>().map_err(|_| {
                    ApiError::BadRequest(format!(
                        "the {LYRICS_HIDDEN_FIELD} is neither `true` nor `false`: {text}"
                    ))
                })?);
            }
            continue;
        }

        let Some(stem) = stem.as_deref() else {
            return Err(ApiError::BadRequest(format!(
                "the {STEM_FIELD} field has to arrive before the files it names"
            )));
        };
        if names.len() == MAX_AUDITION_FILES {
            return Err(ApiError::BadRequest(format!(
                "an audition takes at most {MAX_AUDITION_FILES} files"
            )));
        }
        let name = format!(
            "{stem}.{}",
            audition_extension(field.file_name().unwrap_or_default())?
        );
        if names.contains(&name) {
            return Err(ApiError::BadRequest(format!(
                "{name} arrived twice; an MP3+G pair is two different halves"
            )));
        }

        let path = staged.dir.join(&name);
        // `total` carries across the parts, because an MP3+G pair is two of them and the limit is
        // over the upload rather than over each half.
        total = stream_field_to_file(&mut field, &path, &name, MAX_AUDITION_BYTES, total).await?;
        names.push(name);
    }

    // The first file is the one to play. A second is the other half of a pair, and the machine finds
    // it beside the first for itself — which is the whole reason an upload is staged in a folder.
    let Some(primary) = names.first() else {
        return Err(ApiError::BadRequest(
            "the upload carried no song file".to_owned(),
        ));
    };

    // Off the runtime, for `play_file`'s reason at its worst: the file this is about to parse is one
    // the request itself just uploaded, so it is as large as the machine allows.
    let primary = primary.clone();
    ops::off_runtime(&state, move |state| {
        state.controller().play_audition(
            &primary,
            &Audition {
                title: title.as_deref(),
                artist: artist.as_deref(),
                transpose,
                fixes: fixes.as_deref(),
                melody,
                lyrics: lyrics.as_ref(),
                lyrics_hidden,
            },
        )?;
        Ok(())
    })
    .await?;
    // Only now: the folder has to outlive the request, because what is playing is reading out of it.
    // Every path that returned before this point took the staged files with it.
    staged.keep();

    let snapshot = state.controller().snapshot();
    if let Some(now) = &snapshot.now_playing {
        state.events().publish(Event::SongStarted {
            now_playing: crate::dto::NowPlayingDto::from(now),
        });
    }
    Ok(Json(StateDto::from(&snapshot)))
}

/// `PUT /api/v1/admin/machine/name` — rename the machine.
///
/// **A route of its own rather than a `SettingsPatchDto` field**, which is the rule
/// `docs/architecture/persistence.md` states and the third time it has been followed rather than
/// bent: the settings route carries the knobs that belong to a *performance* and ride the 4 Hz state
/// broadcast, and a name is installation configuration that does neither.
///
/// **No `GET` twin.** `/discover` already carries the name to every caller, publicly and cheaply,
/// which is exactly the argument `PUT /debug/uploads` makes for having none either.
///
/// **Persisted first, applied second.** The controller writes `settings.json` and only then does the
/// running machine start answering to the new name — so a failed write leaves the machine and its
/// settings file agreeing, rather than a machine calling itself something no restart would restore.
///
/// The advert catches up on its own: `run_advertiser` compares the published name against this one
/// every five seconds, which is what `advert_action` grew a name for.
pub async fn put_machine_name(
    State(state): State<ApiState>,
    Body(body): Body<MachineNameRequest>,
) -> ApiResult<Json<MachineNameRequest>> {
    let name = crate::discover::tidy_name(&body.name)
        .ok_or_else(|| ApiError::BadRequest("a machine's name cannot be blank".to_owned()))?;
    state.controller().set_machine_name(&name)?;
    state.set_machine_name(name);
    Ok(Json(MachineNameRequest {
        name: state.machine_name(),
    }))
}

/// `GET /api/v1/locale` — what language the television draws in.
///
/// **Public, and its `PUT` twin is not**, which is the split `/demo` and `/debug` already draw:
/// reading how a machine is set up is anybody's business and changing it is the owner's.
///
/// **A route of its own rather than a field on `/discover`.** That payload is what a remote reads
/// before it knows anything, over multicast and on every poll, and what a screen in another room
/// says changes nothing a remote does — a phone follows its own reader. See
/// `A viewer chooses the remote's language, and the machine does not choose it for them` in
/// `docs/decisions/remotes.md`.
pub async fn get_locale(State(state): State<ApiState>) -> Json<MachineLocaleRequest> {
    Json(MachineLocaleRequest {
        locale: state.controller().machine_locale().tag().to_owned(),
    })
}

/// `PUT /api/v1/admin/machine/locale` — set what language the television draws in.
///
/// **Beside `/machine/name` because it is the same kind of fact** — installation configuration the
/// owner wrote down about this machine, rather than a knob that belongs to a performance and rides
/// the state broadcast. That is the rule `docs/architecture/persistence.md` states, and the name
/// route is the entry above this one.
///
/// **A tag no catalog answers to is a 400 rather than a fallback.** `Locale::best_match` reaches
/// `pt-BR` from `pt` and `pt-PT`, so what gets here unmatched is a language this build does not
/// have at all — and a machine that answered 200 while going on drawing English would be reporting
/// a change nobody could see.
pub async fn put_machine_locale(
    State(state): State<ApiState>,
    Body(body): Body<MachineLocaleRequest>,
) -> ApiResult<Json<MachineLocaleRequest>> {
    let chosen = km_locale::Locale::best_match(&body.locale).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "this machine has no {:?} to draw in",
            body.locale.as_str()
        ))
    })?;
    state.controller().set_machine_locale(chosen)?;
    Ok(Json(MachineLocaleRequest {
        locale: chosen.tag().to_owned(),
    }))
}

/// Reduces the client's stem to something safe to join onto a folder and open on any platform.
///
/// Three rules, and the first is the one that matters. **Only the last component survives**, so
/// `../../authorized_keys` and a rooted `\windows\system32\evil` both come out bare — `Path::join`
/// would otherwise honor an absolute path by discarding the folder it was joined to. Both
/// separators are cut regardless of this platform, because the client is not necessarily on it.
///
/// The second is Windows: `: * ? " < > |` and the control characters cannot appear in a name at all,
/// and Win32 *strips* a component's trailing spaces and dots rather than refusing them — which is
/// the mechanism behind the trailing-space pair in `km_kmpkg::pair_for`'s own documentation. Dealt
/// with here, once, instead of surprising a decoder later.
///
/// The third is length, for [`MAX_STEM_CHARS`]' reason.
pub(crate) fn safe_stem(stem: &str) -> Result<String, ApiError> {
    let refuse = || {
        ApiError::BadRequest(format!(
            "{stem:?} is not a usable name for an uploaded song"
        ))
    };

    let last = stem.rsplit(['/', '\\']).next().ok_or_else(refuse)?;
    let cleaned: String = last
        .chars()
        .take(MAX_STEM_CHARS)
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Leading dots as well as trailing ones: `..` is only the obvious member of a family that also
    // holds `...` and `. `, and none of them is a name anybody meant to send.
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if cleaned.is_empty() {
        return Err(refuse());
    }
    Ok(cleaned.to_owned())
}

/// The extension to stage a part under, taken from its filename and checked against what plays.
///
/// The filename is read for **this and nothing else**, which is what makes reading it safe: the
/// answer is one of a fixed set of `&'static str` or the request is refused, so no byte the client
/// wrote reaches the disk. Lower-cased on the way in, because a real corpus holds `.KAR` and `.Mp3`
/// and a staged pair should be spelled consistently rather than depend on `pair_for`'s tolerance.
fn audition_extension(file_name: &str) -> Result<&'static str, ApiError> {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    audition_extensions()
        .into_iter()
        .find(|known| *known == extension)
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "{file_name} is not a kind of song this machine can play"
            ))
        })
}

/// Every extension an audition may be staged under.
///
/// **Built from `km_kmpkg`'s own lists rather than typed out**, so this cannot come to disagree with
/// what `play_path` will accept — which is the failure that would present as a file uploaded
/// successfully and then refused by the decoder. The MIDI extensions are the one part with no
/// constant to borrow: `km_song` parses by content and has no list of names to offer.
fn audition_extensions() -> impl IntoIterator<Item = &'static str> {
    ["mid", "midi", "kar"]
        .into_iter()
        .chain(km_kmpkg::VIDEO_EXTENSIONS)
        .chain(km_kmpkg::AUDIO_EXTENSIONS)
        .chain([km_kmpkg::GRAPHICS_EXTENSION])
}

/// A staging folder that removes itself unless whatever was uploaded actually landed.
///
/// Every early return in [`play_upload`] and in the three owner uploads drops one of these, so a
/// refused name, an oversized body and a client that hung up all leave nothing behind. The audition
/// path calls [`Staged::keep`] because what is playing is reading out of the folder; the owner
/// uploads deliberately do **not**, because `Controller::accept_upload` moves the file out and the
/// folder should go with the request that made it.
struct Staged {
    dir: PathBuf,
    keep: bool,
}

impl Staged {
    fn new(dir: PathBuf) -> Self {
        Self { dir, keep: false }
    }

    fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

// -- what an owner sends from a browser ------------------------------------------------------------

/// The most a package may be, in bytes.
///
/// **Two gibibytes, and the number is a consequence rather than a judgment.** `DefaultBodyLimit`
/// takes a `usize` and this workspace builds for a 32-bit Android target, where `usize` is four
/// bytes — so anything larger is not expressible on every platform this compiles for. A package of
/// karaoke video runs to a few hundred megabytes at the long end, so the cap is generous either way,
/// and saying where it comes from stops somebody raising it and breaking the Android build.
pub const MAX_PACKAGE_BYTES: usize = 2 * 1024 * 1024 * 1024 - 1;

/// The most a SoundFont bank may be.
///
/// A gibibyte, matching [`MAX_AUDITION_BYTES`]: the offered banks reach that, which
/// `Controller::fetch_soundfont`'s own documentation records, and a limit that refuses the banks
/// this machine itself offers to download would be a strange one.
pub const MAX_SOUNDFONT_BYTES: usize = 1024 * 1024 * 1024;

/// The most a wallpaper or a pack of them may be.
///
/// Sixty-four mebibytes. Wallpapers are photographs and a zip of them is the shipped set's own
/// shape — seven CC0 photographs — so this is roomy for the thing it is for and small enough that a
/// misdirected upload fails quickly.
pub const MAX_WALLPAPER_BYTES: usize = 64 * 1024 * 1024;

/// What to say about a multipart failure, and with which status.
///
/// **`status()` and `body_text()`, never `Display`.** `MultipartError`'s `Display` is the fixed
/// string *"Error parsing `multipart/form-data` request"* whatever went wrong — a parser complaint
/// naming neither a size nor a limit, for a file that was perfectly well formed. That is the sentence
/// an 85 MB package came back with when the owner's page was missing its `DefaultBodyLimit` layers,
/// and it cost a debugging session; `km_admin_pages::router` documents the whole episode. axum's own
/// accessors distinguish the cases: `status()` is 413 for a length trip and 400 for a malformed body,
/// and `body_text()` is multer's real message rather than that one string.
///
/// A length trip gets **this machine's own sentence**, because axum's is *"Request payload is too
/// large"* and names no number somebody could act on. Everything else keeps the 400 it has always
/// had and simply says something true — including axum's own 500 case, a stream that failed to read,
/// which is very nearly always a client that hung up mid-upload and is not this machine's fault to
/// log as one.
pub(crate) fn multipart_failure(
    error: &axum::extract::multipart::MultipartError,
    limit: usize,
) -> ApiError {
    if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        return too_large(limit);
    }
    ApiError::BadRequest(format!("the upload stopped early: {}", error.body_text()))
}

/// The one sentence about size, naming the limit in words somebody can read.
pub(crate) fn too_large(limit: usize) -> ApiError {
    ApiError::PayloadTooLarge(format!(
        "that is larger than this machine accepts — the limit is {}",
        bytes_in_words(limit as u64)
    ))
}

/// Streams one multipart field into a file, refusing it the moment it passes `limit`.
///
/// **A chunk at a time and never the whole body**, which is the whole reason this is not a
/// `field.bytes().await`: these are the two routes with limits measured in gigabytes, and a video
/// read into memory first would be a gigabyte of it. The count is checked as it goes, so a client
/// that lies about its size is stopped mid-stream rather than after the disk has taken it.
///
/// `already` is what previous fields of the same request have contributed, because
/// `play_upload` sends a pair and the limit is over the pair rather than over each half. It comes
/// back updated.
///
/// Extracted because this existed twice — `play_upload` and the owner uploads — as thirty-odd
/// identical lines differing only in the wording of their error messages. `what` supplies that
/// wording, which is the only thing the two callers ever disagreed about.
pub(crate) async fn stream_field_to_file(
    field: &mut axum::extract::multipart::Field<'_>,
    path: &std::path::Path,
    what: &str,
    limit: usize,
    already: usize,
) -> ApiResult<usize> {
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|error| ApiError::Internal(format!("could not stage {what}: {error}")))?;
    let mut total = already;
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|error| multipart_failure(&error, limit))?
    {
        total = total.saturating_add(chunk.len());
        if total > limit {
            return Err(too_large(limit));
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| ApiError::Internal(format!("could not write {what}: {error}")))?;
    }
    file.flush()
        .await
        .map_err(|error| ApiError::Internal(format!("could not finish {what}: {error}")))?;
    Ok(total)
}

/// A byte count for a person: `64 MB`, `2 GB`.
///
/// **Rounded up, because it is quoting a ceiling.** A limit of 2 GiB − 1 shown as "1 GB" would be a
/// smaller number than the thing it refuses. The unit is the binary one spelled the way a file
/// manager spells it, which is what the person reading it will compare against.
#[must_use]
pub fn bytes_in_words(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= GIB {
        format!("{} GB", bytes.div_ceil(GIB))
    } else {
        format!("{} MB", bytes.div_ceil(MIB))
    }
}

/// The field carrying the file, in all three owner uploads.
///
/// One name for three routes because there is nothing to tell apart: each takes exactly one file
/// and the route says what kind it is. `play_upload` needs two fields because a song can be a pair.
pub const FILE_FIELD: &str = "file";

/// Streams one uploaded file into a fresh folder and hands it to the machine.
///
/// **The whole of the shape `play_upload` established, minus the parts that are about auditions.**
/// The rejection is remapped so a wrong content type carries an error code like every other failure
/// here; the body is streamed through `Field::chunk` and never held; a running total is checked as
/// well as the per-route `DefaultBodyLimit`, because the layer bounds the request and this bounds
/// what reaches the disk; and [`Staged`] removes the folder on every early return.
///
/// **The name never comes from the part's filename** except for its extension, which is checked
/// against a list and replaced by one of a fixed set of `&'static str`. So the only client bytes
/// that reach a path are the stem, and that has been through [`safe_stem`] — `..`, separators,
/// Windows-illegal characters, control characters, leading and trailing dots and length are all
/// dealt with there rather than four times here.
pub(crate) async fn receive_upload(
    state: &ApiState,
    form: Result<Multipart, MultipartRejection>,
    kind: Upload,
) -> ApiResult<String> {
    let (limit, extensions) = limits_for(kind);
    let mut form =
        form.map_err(|rejection| ApiError::BadRequest(rejection.body_text().to_string()))?;

    let staged = Staged::new(state.controller().open_upload()?);
    let mut landed: Option<PathBuf> = None;

    while let Some(mut field) = form
        .next_field()
        .await
        .map_err(|error| multipart_failure(&error, limit))?
    {
        if field.name() != Some(FILE_FIELD) {
            continue;
        }
        if landed.is_some() {
            return Err(ApiError::BadRequest("send one file at a time".to_owned()));
        }

        let file_name = field.file_name().unwrap_or_default().to_owned();
        let extension = allowed_extension(&file_name, extensions)?;
        let stem = std::path::Path::new(&file_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let stem = safe_stem(stem)?;

        let destination = staged.dir.join(format!("{stem}.{extension}"));
        // One part per request here, so this starts from zero where `play_upload` carries a running
        // total across a pair. The file is closed inside the helper, which is what the `drop` this
        // replaced was for.
        stream_field_to_file(&mut field, &destination, "the upload", limit, 0).await?;
        landed = Some(destination);
    }

    let Some(landed) = landed else {
        return Err(ApiError::BadRequest(format!(
            "there is no {FILE_FIELD} part in that upload"
        )));
    };

    // Off the runtime: installing a package holds the catalog mutex for seconds, and moving a
    // gigabyte of SoundFont across a mount point is not a thing to do on a worker thread.
    let controller = state.controller_handle();
    let said = tokio::task::spawn_blocking(move || controller.accept_upload(kind, &landed))
        .await
        .map_err(|error| {
            ApiError::Internal(format!("the upload could not be finished: {error}"))
        })??;
    Ok(said)
}

/// How large each kind may be, and what it may be called.
///
/// **Keyed on the kind rather than passed in at each route**, so the owner's page and the JSON API
/// cannot come to disagree about what a wallpaper is — they are two front doors onto one function
/// and this is the table both read.
pub(crate) fn limits_for(kind: Upload) -> (usize, &'static [&'static str]) {
    match kind {
        Upload::Package => (MAX_PACKAGE_BYTES, &["kmpkg"]),
        // A pack and never a picture. The wallpaper folder holds zips, so a route that took a `.jpg`
        // would accept a file the display then passes over — an upload that says it worked and
        // changes nothing on the screen. See `A wallpaper is a pack` in
        // `docs/decisions/interface.md`.
        Upload::Wallpaper => (MAX_WALLPAPER_BYTES, &["zip"]),
        Upload::SoundFont => (MAX_SOUNDFONT_BYTES, &["sf2"]),
    }
}

/// The extension to stage an owner upload under, taken from its filename and checked against a list.
///
/// [`audition_extension`]'s sibling and the same rule for the same reason: the filename is read for
/// this and nothing else, so the answer is one of a fixed set of `&'static str` or the request is
/// refused. Lower-cased, so `PHOTO.JPG` and `photo.jpg` land the same way.
fn allowed_extension(file_name: &str, allowed: &[&'static str]) -> Result<&'static str, ApiError> {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    allowed
        .iter()
        .copied()
        .find(|known| *known == extension)
        .ok_or_else(|| {
            ApiError::BadRequest(format!("{file_name} is not something this route takes"))
        })
}

/// `POST /api/v1/admin/packages/upload` — send a package to the machine.
///
/// **A route of its own rather than a content-type branch on `POST /packages`.** That one means
/// "install what is already at this path", which is what `km-package-builder` says to a machine
/// sharing its filesystem; this means "here are the bytes". Overloading one path would make it mean
/// two things to two clients and force one ACL id to cover both — and the two need different ones,
/// since the first must stay public for the builder and this one ships closed.
pub async fn upload_package(
    State(state): State<ApiState>,
    form: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<UploadReportDto>> {
    let said = receive_upload(&state, form, Upload::Package).await?;
    Ok(Json(UploadReportDto { report: said }))
}

/// `POST /api/v1/admin/wallpapers` — add a picture, or a pack of them, to the rotation.
///
/// A `.zip` is accepted whole and is the natural unit for a set: `Zipped wallpapers` already has the
/// playlist reading images out of an archive without unpacking it.
pub async fn upload_wallpaper(
    State(state): State<ApiState>,
    form: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<UploadReportDto>> {
    let said = receive_upload(&state, form, Upload::Wallpaper).await?;
    Ok(Json(UploadReportDto { report: said }))
}

/// `POST /api/v1/admin/audio/soundfonts` — add a bank.
///
/// The upload twin of `POST /admin/audio/soundfont/fetch`, which downloads one of the offered banks. Both
/// end with a file in the SoundFont folder and `soundfont::installed` reading the directory again.
pub async fn upload_soundfont(
    State(state): State<ApiState>,
    form: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<UploadReportDto>> {
    let said = receive_upload(&state, form, Upload::SoundFont).await?;
    Ok(Json(UploadReportDto { report: said }))
}

/// `POST /api/v1/admin/password` — set or clear the admin password.
///
/// **The route that turns the whole admin mechanism on, and until now nothing but a command line
/// could reach it.** `A machine with no password has no door` calls setting a password "the single
/// act that turns the whole mechanism on", and `--set-password` is the only thing that could perform
/// it — on an appliance under a television that is `ssh` or nothing, and the phone in the room is
/// the only interface that box has.
///
/// **The prefix rule is what makes this safe to add.** `admin.password.write` is privileged by
/// `Acl::requirement`'s `admin.` test rather than by the map, so it cannot be opened up by
/// `PUT /acl`, and on a machine with a password it demands a token like every other admin route —
/// which means changing a password requires the current one. On a machine *without* one it is
/// public, which is the whole point: that is the state it exists to get somebody out of.
pub async fn set_admin_password(
    State(state): State<ApiState>,
    Body(body): Body<AdminPasswordRequest>,
) -> ApiResult<Json<AdminPasswordDto>> {
    // **`null` no longer clears the password; it resets it to a freshly generated PIN.** A machine
    // always has one, so there is nothing for "no password" to mean — and the act an owner actually
    // wants under that button is "I have forgotten mine, give me a new one off the screen".
    let (hash, factory_pin) = match body.password.as_deref().map(str::trim) {
        None => {
            let pin = crate::auth::generate_factory_pin();
            let hash = crate::AdminAuth::hash_password(&pin).map_err(|error| {
                ApiError::Internal(format!("the password could not be stored: {error}"))
            })?;
            (hash, Some(pin))
        }
        Some(password) => {
            if password.chars().count() < MIN_PASSWORD_CHARS {
                return Err(ApiError::BadRequest(format!(
                    "a password wants at least {MIN_PASSWORD_CHARS} characters"
                )));
            }
            let hash = crate::AdminAuth::hash_password(password).map_err(|error| {
                ApiError::Internal(format!("the password could not be stored: {error}"))
            })?;
            (hash, None)
        }
    };

    // Persist first, then move the running value, so a failed write never leaves a machine
    // answering to a password its own settings file does not hold.
    state
        .controller()
        .set_admin_password(Some(hash.clone()), factory_pin.clone())?;
    state.set_admin_password(Some(hash), factory_pin.is_some());
    Ok(Json(AdminPasswordDto {
        password_set: true,
        // **The PIN is echoed only when the machine just invented it**, and never on a password an
        // owner typed. A caller that asked for a reset has to be told the new one somehow, and it is
        // already on the television; a caller that set their own knows it.
        factory_pin,
    }))
}

// -- the event stream ----------------------------------------------------------------------------

/// `GET /api/v1/events` — the WebSocket upgrade.
pub async fn events(upgrade: WebSocketUpgrade, State(state): State<ApiState>) -> Response {
    upgrade.on_upgrade(move |socket| pump_events(socket, state))
}

/// Feeds one remote until it goes away.
async fn pump_events(socket: WebSocket, state: ApiState) {
    let (mut sink, mut incoming) = socket.split();
    let mut receiver = state.events().subscribe();

    // Send the current state before anything else, so a client is synchronized the instant it
    // connects rather than up to a quarter-second later. Without this, a remote that connects
    // between ticks renders an empty screen for long enough to look broken.
    let opening = Event::State {
        state: StateDto::from(&state.controller().snapshot()),
    };
    if send_event(&mut sink, &opening).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            // Read the client's half only to notice it closing. Control happens over REST: a
            // WebSocket that also accepted commands would need its own authorization story, and one
            // ACL is enough.
            message = incoming.next() => match message {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            },
            event = receiver.recv() => match event {
                Ok(event) => {
                    if send_event(&mut sink, &event).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "a remote fell behind the event stream");
                    if send_event(&mut sink, &Event::Desync { skipped }).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
            },
        }
    }
}

async fn send_event<S>(sink: &mut S, event: &Event) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let Ok(text) = serde_json::to_string(event) else {
        // Our own type failing to serialize is a bug, not a client problem. Drop the event rather
        // than the connection.
        tracing::error!("an event could not be serialized");
        return Ok(());
    };
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

// -- the machine's own log -----------------------------------------------------------------------
//
// **Nothing below may emit a `tracing` event, at any level.** A line said here would be taken by
// the tap, broadcast to the reader that provoked it, and provoke another — and for the lag arm in
// particular that loop feeds exactly the reader least able to keep up. `pump_events` logs on its
// own lag and is right to, because an event is not a log record; the same line here is the one
// mistake this section exists to warn about. It is also why `send_frame` below is not `send_event`,
// which reports a serialization failure at `error!`.

/// This machine's log, or the 404 that says it keeps none.
///
/// `UnknownEndpoint` for [`power_of`]'s reason: what is missing is the path, not the thing. A
/// program that installed no tap never mounts these routes, so the only way here is a race between
/// a router built without one and a request already in flight, and both must answer alike.
fn log_tap_of(state: &ApiState) -> ApiResult<&km_logtap::LogTap> {
    state
        .log_tap()
        .ok_or_else(|| ApiError::UnknownEndpoint("this machine keeps no log in memory".to_owned()))
}

/// `GET /api/v1/admin/logs`
///
/// What the machine has kept, oldest first. The stream below opens with the same records, so this
/// exists for the reader that is not a browser: `curl`, and the walkthrough script beside the
/// console.
pub async fn get_logs(State(state): State<ApiState>) -> ApiResult<Json<crate::dto::LogTailDto>> {
    let tap = log_tap_of(&state)?;
    Ok(Json(crate::dto::LogTailDto {
        records: tap
            .tail()
            .iter()
            .map(|record| record.as_ref().into())
            .collect(),
        capacity: tap.capacity(),
        dropped: tap.dropped(),
        filter: tap.filter().map(str::to_owned),
    }))
}

/// `GET /api/v1/admin/logs/stream` — the WebSocket upgrade.
pub async fn logs_stream(upgrade: WebSocketUpgrade, State(state): State<ApiState>) -> Response {
    // Resolved before the upgrade, so a machine with no tap answers 404 rather than accepting a
    // socket it has nothing to put on.
    let tap = match log_tap_of(&state) {
        Ok(tap) => tap.clone(),
        Err(error) => return error.into_response(),
    };
    upgrade.on_upgrade(move |socket| pump_logs(socket, tap))
}

/// Feeds one reader until it goes away.
async fn pump_logs(socket: WebSocket, tap: km_logtap::LogTap) {
    let (mut sink, mut incoming) = socket.split();

    // **Subscribed before the tail is read, and the overlap is deliberate.** Taken the other way
    // round, a record arriving between the two is in neither — and that window is exactly when a
    // machine is busy enough for somebody to be watching. `seq` is what drops the duplicate.
    let (tail, mut reader) = tap.tail_and_subscribe();
    let mut last = 0;
    for record in tail {
        last = record.seq;
        let frame = crate::dto::LogFrameDto::Record {
            record: record.as_ref().into(),
        };
        if send_frame(&mut sink, &frame).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            // Read the client's half only to notice it closing, as the event stream does: there is
            // nothing a reader of a log can ask for.
            message = incoming.next() => match message {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            },
            record = reader.recv() => match record {
                Ok(record) => {
                    // The tail's own records come back down the channel when they were taken after
                    // the subscription; drawn twice they would read as the machine repeating itself.
                    if record.seq <= last {
                        continue;
                    }
                    last = record.seq;
                    let frame = crate::dto::LogFrameDto::Record {
                        record: record.as_ref().into(),
                    };
                    if send_frame(&mut sink, &frame).await.is_err() {
                        break;
                    }
                }
                // Said to the reader and nowhere else. See the warning at the head of this section.
                Err(RecvError::Lagged(skipped)) => {
                    if send_frame(&mut sink, &crate::dto::LogFrameDto::Lagged { skipped })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
            },
        }
    }
}

/// Sends one frame, silently.
///
/// Its own function rather than [`send_event`] because that one reports a serialization failure at
/// `error!`, which on this socket is a line about the log going into the log. A frame that will not
/// serialize is dropped here instead: our own type failing is a bug, and the reader is not the place
/// it can be reported.
async fn send_frame<S>(sink: &mut S, frame: &crate::dto::LogFrameDto) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let Ok(text) = serde_json::to_string(frame) else {
        return Ok(());
    };
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

// -- shared bits ---------------------------------------------------------------------------------

/// Runs a transport command and publishes what changed.
///
/// A thin wrapper over [`ops::transport`], kept because every transport handler reads better ending
/// in one call than in a `Json(...)` around one. The operation itself lives in `ops` because the
/// remote this machine serves runs it too — see that module's header.
///
/// Off the runtime, because two of the six commands can load a song: `Play` on an idle machine and
/// `Skip` both reach `advance`, which reads the package and opens a decoder. All six go the same
/// way rather than only those two — a `spawn_blocking` around a channel send costs microseconds,
/// and a wrapper that took one route for some commands and another for the rest is a thing somebody
/// has to keep right when a seventh command arrives.
async fn run_transport(state: &ApiState, command: TransportCommand) -> ApiResult<Json<StateDto>> {
    let state = ops::off_runtime(state, move |state| ops::transport(state, command)).await?;
    Ok(Json(state))
}

/// Applies a settings patch and publishes the result.
fn apply_settings(state: &ApiState, patch: SettingsPatch) -> ApiResult<Json<SettingsDto>> {
    Ok(Json(ops::apply_settings(state, patch)?))
}
