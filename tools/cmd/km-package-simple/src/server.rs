//! The router and its handlers.
//!
//! **A press redraws the page.** Every button answers with `HX-Refresh`, and the page is drawn
//! again from the state. The two exceptions are a rename, which moves nothing else, and keeping a
//! song or leaving it out, which redraws the list because every number after it moves.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::Deserialize;

use crate::app::{App, Phase};
use crate::session::PackageForm;
use crate::views;

/// Every sentence a handler says through `App::lock().error`, for the catalog parity tests.
///
/// They reach `msg` as a variable, because `App` and [`refresh_saying`] take a key rather than a
/// sentence.
pub const SAID_KEYS: &[&str] = &[
    "said-busy",
    "said-no-folder",
    "said-nothing-kept",
    "said-no-name",
    "said-reveal-failed",
];

/// The stylesheet.
const STYLE_CSS: &str = include_str!("../static/style.css");

/// The page's own script: the shift-click that keeps or leaves out a run of songs.
const UI_JS: &str = include_str!("../static/ui.js");

/// htmx, from the copy the package builder vendors, so the repository holds one.
const HTMX_JS: &str = include_str!("../../km-package-builder/static/htmx.min.js");

/// The favicon. The package builder's mark until this program has one of its own.
const ICON_PNG: &[u8] = include_bytes!("../../../../icon/km-package-builder-32.png");

/// Every route.
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/progress", get(progress))
        .route("/folder", post(read_folder))
        .route("/songs/{index}", post(rename))
        .route("/songs/keep", post(keep))
        .route("/build", post(build))
        .route("/back", post(back))
        .route("/close", post(close))
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

/// Puts a sentence where the next page shows it, and draws the page again.
fn refresh_saying(app: &App, headers: &HeaderMap, key: &str) -> Response {
    let said = crate::words::messages(locale_of(app, headers))
        .msg(key)
        .into_owned();
    app.lock().error = Some(said);
    refresh()
}

/// Which page of the song list to draw.
#[derive(Debug, Default, Deserialize)]
struct PageQuery {
    #[serde(default)]
    page: usize,
}

/// `GET /`: whatever the state calls for.
async fn home(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Response {
    let locale = locale_of(&app, &headers);
    let windowed = app.windowed.load(Ordering::Relaxed);
    let mut inner = app.lock();
    let page = views::page(&inner, locale, windowed, query.page);
    // Shown once: a reload after it has been read should not say it again.
    inner.error = None;
    page
}

/// `GET /progress`: the running job, or a redraw once it has ended.
async fn progress(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let locale = locale_of(&app, &headers);
    let inner = app.lock();
    match &inner.phase {
        phase @ (Phase::Reading { .. } | Phase::Building { .. }) => views::render(
            &views::progress(phase, crate::words::messages(locale)),
            locale,
        ),
        _ => refresh(),
    }
}

#[derive(Debug, Deserialize)]
struct FolderForm {
    path: String,
}

/// `POST /folder`: read a folder.
async fn read_folder(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(form): Form<FolderForm>,
) -> Response {
    match app.read_folder(std::path::PathBuf::from(form.path.trim())) {
        Ok(()) => refresh(),
        Err(key) => refresh_saying(&app, &headers, key),
    }
}

#[derive(Debug, Deserialize)]
struct RenameForm {
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
}

/// `POST /songs/{index}`: rename a song. Nothing else on the page moves, so nothing is drawn.
async fn rename(
    State(app): State<Arc<App>>,
    Path(index): Path<usize>,
    Form(form): Form<RenameForm>,
) -> Response {
    let mut inner = app.lock();
    let renamed = inner
        .session
        .as_mut()
        .is_some_and(|session| session.rename(index, &form.title, &form.artist));
    if renamed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

#[derive(Debug, Deserialize)]
struct KeepForm {
    from: usize,
    to: usize,
    keep: Option<String>,
}

/// `POST /songs/keep`: keep the songs from `from` to `to` or leave them out, and redraw the list.
async fn keep(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    Form(form): Form<KeepForm>,
) -> Response {
    let locale = locale_of(&app, &headers);
    let mut inner = app.lock();
    let kept = form.keep.is_some();
    let changed = inner
        .session
        .as_mut()
        .is_some_and(|session| session.keep(form.from, form.to, kept));
    if !changed {
        return StatusCode::NOT_FOUND.into_response();
    }
    views::render(
        &views::song_list(&inner, crate::words::messages(locale), query.page),
        locale,
    )
}

#[derive(Debug, Deserialize)]
struct BuildForm {
    name: String,
    version: String,
    #[serde(default)]
    publisher: String,
    language: String,
    out_dir: String,
}

/// `POST /build`: write the packages.
async fn build(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(form): Form<BuildForm>,
) -> Response {
    let publisher = form.publisher.trim();
    let form = PackageForm {
        name: form.name.trim().to_owned(),
        version: form.version.trim().to_owned(),
        publisher: (!publisher.is_empty()).then(|| publisher.to_owned()),
        language: form.language.trim().to_owned(),
        out_dir: std::path::PathBuf::from(form.out_dir.trim()),
    };
    match app.build(form) {
        Ok(()) => refresh(),
        Err(key) => refresh_saying(&app, &headers, key),
    }
}

/// `POST /back`: from the written packages to the song list.
async fn back(State(app): State<Arc<App>>) -> Response {
    app.back_to_songs();
    refresh()
}

/// `POST /close`: forget the folder.
async fn close(State(app): State<Arc<App>>) -> Response {
    app.close();
    refresh()
}

/// `POST /reveal`: open the folder the packages were written to.
async fn reveal(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let folder = app.lock().form.as_ref().map(|form| form.out_dir.clone());
    let Some(folder) = folder else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let opened = tokio::task::spawn_blocking(move || km_osopen::open(&folder)).await;
    match opened {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        _ => refresh_saying(&app, &headers, "said-reveal-failed"),
    }
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
/// The rule `km-package-builder` follows, for its reason: every form here is a CORS simple
/// request, so any page open in the same browser could otherwise read a folder or write packages
/// through this tool. See `A page on another site cannot press this tool's buttons` in
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
