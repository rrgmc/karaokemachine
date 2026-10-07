//! The router and its handlers.
//!
//! **The page is drawn once and its parts are swapped.** Walking the folders swaps the browser,
//! and a Start swaps the editor's state. The words box is outside both, so what somebody pasted
//! is still there after either.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::Deserialize;

use crate::app::{App, Editor, Outcome, Start};
use crate::views;

/// Every sentence a handler says by its key, for the catalog parity tests.
///
/// They reach `msg` as a variable, because [`App::start`] answers with a key and not a sentence.
pub const SAID_KEYS: &[&str] = &[
    "said-busy",
    "said-no-song",
    "said-no-machine",
    "said-no-words",
    "said-box-empty",
    "said-not-midi",
    "said-output-exists",
    "said-reveal-failed",
];

/// How many folders and songs one page of the browser holds.
///
/// Few, because the words box is under the browser and a long page pushes it out of the window.
pub const PAGE_ROWS: usize = 15;

/// The event the page hears when the editor has closed, which draws the browser again.
const EDITOR_ENDED: &str = "editor-ended";

/// The stylesheet.
const STYLE_CSS: &str = include_str!("../static/style.css");

/// The page's own script: whether the words box holds anything.
const UI_JS: &str = include_str!("../static/ui.js");

/// htmx, from the copy the package builder vendors, so the repository holds one.
const HTMX_JS: &str = include_str!("../../km-package-builder/static/htmx.min.js");

/// The favicon.
const ICON_PNG: &[u8] = include_bytes!("../../../../icon/km-song-sync-32.png");

/// Every route.
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/browse", get(browse))
        .route("/start", post(start))
        .route("/editor", get(editor))
        .route("/reveal", post(reveal))
        .route("/locale", post(locale))
        .route("/quit", post(quit))
        .route("/static/style.css", get(|| async { css(STYLE_CSS) }))
        .route("/static/htmx.min.js", get(|| async { script(HTMX_JS) }))
        .route("/static/ui.js", get(|| async { script(UI_JS) }))
        .route("/static/icon.png", get(|| async { png(ICON_PNG) }))
        .layer(axum::middleware::from_fn(refuse_cross_site))
        .with_state(app)
}

/// The language a request is answered in.
fn locale_of(app: &App, headers: &HeaderMap) -> km_locale::Locale {
    app.locale(
        headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok()),
    )
}

/// Tells htmx to draw the page again.
fn refresh() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert("HX-Refresh", HeaderValue::from_static("true"));
    response
}

/// What the browser asks for.
#[derive(Debug, Default, Deserialize)]
struct BrowseQuery {
    /// The folder to list. Absent means wherever [`App::start_folder`] says.
    at: Option<String>,
    /// Show only folders and songs whose name holds this.
    #[serde(default)]
    filter: String,
    /// Which page, as a row index.
    #[serde(default)]
    offset: usize,
    /// List the drives. **A presence flag, read only for being there**: a `bool` would demand the
    /// query spell `true`, and the crumb sends `drives=1`.
    drives: Option<String>,
    /// Answer with the rows and the pager only, for the filter box and a page turn.
    rows: Option<String>,
}

/// One folder read: its listing, what each song on the page holds, and the shortcuts beside it.
///
/// All three read the disk, so all three run on a blocking thread. A folder that cannot be read is a listing
/// with the reason on it.
async fn read(
    app: &Arc<App>,
    query: &BrowseQuery,
) -> Option<(
    km_folders::Listing,
    Vec<crate::rows::Row>,
    Vec<km_folders::Place>,
)> {
    let here = if query.drives.is_some() {
        None
    } else {
        query
            .at
            .as_deref()
            .map(str::trim)
            .filter(|at| !at.is_empty())
            .map(PathBuf::from)
            .or_else(|| app.start_folder())
            // A folder named from where the program was started has no folder above it to walk
            // to, and its songs would be handed to the editor by a name only this program can
            // follow.
            .map(|here| std::path::absolute(&here).unwrap_or(here))
    };
    let ask = km_folders::Ask {
        here,
        filter: query.filter.trim().to_owned(),
        offset: query.offset,
    };
    let app = Arc::clone(app);
    tokio::task::spawn_blocking(move || {
        // The filter narrows a song by its file name, and by the title and artist it states.
        let listing =
            km_folders::list_matching(&ask, PAGE_ROWS, crate::rows::is_song, |song, wanted| {
                app.names.hold(song, wanted)
            });
        // A folder that was read is where the browser is. One that was not is a wrong turn.
        if let (Some(here), None) = (&ask.here, &listing.error) {
            app.remember_folder(here);
        }
        let rows = crate::rows::describe(&listing.files);
        (listing, rows, km_folders::places())
    })
    .await
    .ok()
}

/// `GET /`: the page, with the browser on the folder it starts in.
async fn home(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let locale = locale_of(&app, &headers);
    let words = crate::words::messages(locale);
    let Some((listing, rows, places)) = read(&app, &BrowseQuery::default()).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let machine = app.machine().is_some();
    let state = app.lock().editor.clone();
    views::render(
        &views::HomePage {
            chrome: views::Chrome::new(locale, app.windowed.load(Ordering::Relaxed)),
            machine,
            languages: views::languages(),
            editor: views::editor(&state, None, words),
            browser: views::browser(listing, rows, &places, machine, words),
        },
        locale,
    )
}

/// `GET /browse`: the browser on a folder, or one page of its rows.
async fn browse(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<BrowseQuery>,
) -> Response {
    let locale = locale_of(&app, &headers);
    let words = crate::words::messages(locale);
    let Some((listing, rows, places)) = read(&app, &query).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let machine = app.machine().is_some();
    if query.rows.is_some() {
        views::render(&views::browser_rows(listing, rows, machine, words), locale)
    } else {
        views::render(
            &views::browser(listing, rows, &places, machine, words),
            locale,
        )
    }
}

/// What a Start button posts: its own row's fields, and the words box beside the browser.
#[derive(Debug, Deserialize)]
struct StartForm {
    song: String,
    #[serde(default)]
    words: String,
    /// Present when the words box is ticked for use.
    use_words: Option<String>,
    /// Present when the box is ticked.
    resume: Option<String>,
    /// Present when the row's box is ticked.
    force: Option<String>,
    /// The three names the panel holds, each starting as the song's own.
    title: Option<String>,
    artist: Option<String>,
    language: Option<String>,
}

/// `POST /start`: open the editor on a song, and answer with the editor's state.
///
/// A refused start is answered with the reason on the same panel, and changes nothing.
async fn start(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(form): Form<StartForm>,
) -> Response {
    let locale = locale_of(&app, &headers);
    let words = crate::words::messages(locale);
    let asked = Start {
        song: PathBuf::from(form.song),
        pasted: form.use_words.is_some().then_some(form.words),
        resume: form.resume.is_some(),
        force: form.force.is_some(),
        title: form.title,
        artist: form.artist,
        language: form.language,
    };
    // The checks read the song and the folder around it.
    let started = {
        let app = Arc::clone(&app);
        tokio::task::spawn_blocking(move || app.start(asked)).await
    };
    let refused = match started {
        Ok(Ok(())) => None,
        Ok(Err(key)) => Some(key),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let state = app.lock().editor.clone();
    views::render(&views::editor(&state, refused, words), locale)
}

/// `GET /editor`: the editor's state, polled while it is open.
///
/// The answer that says it has closed also tells the page to draw the browser again, because the
/// synced copy is in the folder now.
async fn editor(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let locale = locale_of(&app, &headers);
    let state = app.lock().editor.clone();
    let mut response = views::render(
        &views::editor(&state, None, crate::words::messages(locale)),
        locale,
    );
    if matches!(state, Editor::Ended(_)) {
        response
            .headers_mut()
            .insert("HX-Trigger", HeaderValue::from_static(EDITOR_ENDED));
    }
    response
}

/// `POST /reveal`: open the folder the synced copy was written to.
async fn reveal(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let folder = match &app.lock().editor {
        Editor::Ended(Outcome::Saved { out }) => out.parent().map(PathBuf::from),
        _ => None,
    };
    let Some(folder) = folder else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let opened = tokio::task::spawn_blocking(move || km_osopen::open(&folder)).await;
    if matches!(opened, Ok(Ok(()))) {
        return StatusCode::NO_CONTENT.into_response();
    }
    let locale = locale_of(&app, &headers);
    let state = app.lock().editor.clone();
    views::render(
        &views::editor(
            &state,
            Some("said-reveal-failed"),
            crate::words::messages(locale),
        ),
        locale,
    )
}

#[derive(Debug, Deserialize)]
struct LocaleForm {
    locale: String,
}

/// `POST /locale`: speak another language, and redraw.
async fn locale(State(app): State<Arc<App>>, Form(form): Form<LocaleForm>) -> Response {
    match km_locale::Locale::parse(&form.locale) {
        Some(locale) => {
            app.set_locale(locale);
            refresh()
        }
        None => StatusCode::BAD_REQUEST.into_response(),
    }
}

/// `POST /quit`: stop, the same stop Ctrl-C asks for.
async fn quit(State(app): State<Arc<App>>) -> Response {
    app.stop.ask();
    StatusCode::NO_CONTENT.into_response()
}

fn css(body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], body).into_response()
}

fn script(body: &'static str) -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        body,
    )
        .into_response()
}

fn png(body: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, "image/png")], body).into_response()
}

/// Refuses a state-changing request another site's page sent.
///
/// The rule `km-package-builder` follows, and it matters more here: `POST /start` starts a
/// program. Every form on the page is a CORS simple request, so any page open in the same browser
/// could otherwise press Start. See `A page on another site cannot press this tool's buttons` in
/// `docs/decisions/curation.md`.
async fn refuse_cross_site(request: Request, next: Next) -> Response {
    if is_cross_site(&request) {
        tracing::warn!(path = %request.uri().path(), "refused a request from another site");
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

/// GET and HEAD are not gated: they change nothing.
fn is_cross_site(request: &Request) -> bool {
    if matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        return false;
    }
    let headers = request.headers();
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        return !matches!(site.trim(), "same-origin" | "none");
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return false;
    };
    let Ok(origin) = origin.to_str() else {
        return true;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        != Some(host)
}
