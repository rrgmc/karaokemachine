//! The page a television opens, and the stream beneath it.
//!
//! **Two paths and no JSON.** `/watch` is a page and `/stream/` is a directory of files a media
//! player fetches, so neither belongs on the versioned API surface: nothing here answers questions
//! about the machine, and a client is a `<video>` element rather than something parsing a reply.
//!
//! # The playlist is the interface
//!
//! `/stream/live.m3u8` is what this exists to serve, and the page above it is a convenience.
//! Anything that follows a URL plays the stream — a television's own media pipeline, VLC, Kodi, a
//! player on a set-top box — without knowing that a page, a browser or this project exists. That is
//! what makes "as many clients as possible" achievable rather than a pile of per-device work, and it
//! is why the page must never become the only way in.
//!
//! # The page's faster path
//!
//! `/stream/live.ws` carries the same encoded stream as fragmented MP4 over a WebSocket, for the
//! page to append straight into its `<video>`. A playlist puts a client three or four seconds
//! behind, and this puts the page under one. It is mounted only when the machine hands over a
//! [`Live`], and the playlist beside it is unchanged.
//!
//! # Public, on the same terms as the queue
//!
//! Outside `/api/v1/admin/`, so no token is asked for, which is the judgment `Network reach` already
//! makes about this LAN: anybody in the room can queue a song, and anybody in the room can watch the
//! screen they are queueing it onto.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};

/// Where the page lives, and the prefix its relative links resolve against.
pub const WATCH_PATH: &str = "/watch";

/// Where the playlist and its segments are served from.
pub const STREAM_PREFIX: &str = "/stream";

/// The page itself.
///
/// Compiled in rather than read from disk, for the reason the development console's built-in copy
/// gives: a machine under a television has whatever its executable carries, and no folder anybody
/// can reach to put a file in.
const WATCH_HTML: &str = include_str!("../static/watch.html");

/// The one library the page needs, and only on the browsers that have no HLS of their own.
const HLS_JS: &str = include_str!("../static/hls.light.min.js");

/// hls.js's license, because a vendored dependency's terms travel with it.
const HLS_LICENSE: &str = include_str!("../static/hls-LICENSE.txt");

/// Where the page's socket is, beside the playlist.
pub const SOCKET: &str = "live.ws";

/// Serves the page, its library, and the directory the encoder is writing into.
///
/// **`dir` is where the segments are**, which is the machine's business rather than this crate's —
/// the same division `play_file`'s allowed roots make. Nothing here creates it: a directory that is
/// not there yet is a stream that has not started, and the playlist 404s until it has.
///
/// **`live` mounts the page's socket**, and a machine that hands none over has no such path. The
/// page then finds nothing there and plays the playlist.
pub fn router(dir: &Path, live: Option<Live>) -> Router {
    let routes = Router::new();
    let routes = match live {
        Some(live) => routes.route(
            &format!("{STREAM_PREFIX}/{SOCKET}"),
            get(move |upgrade: WebSocketUpgrade| async move {
                upgrade.on_upgrade(move |socket| pump_fragments(socket, live))
            }),
        ),
        None => routes,
    };
    routes
        // **The trailing slash is the page's real address, and the other spelling redirects onto
        // it.** The library beside the page is named relatively — `hls.light.min.js` — and a
        // relative name resolves against the *directory* of the current URL, so on `/watch` it is
        // sought at `/hls.light.min.js` and on `/watch/` at `/watch/hls.light.min.js`. Serving the
        // same page at both spellings would therefore work on one and quietly fail on the other,
        // and the failure is the one nobody sees coming: the page loads, the picture never starts,
        // and only a browser with no HLS of its own is affected.
        //
        // **The playlist is named absolutely and is outside all of this**, being served at the root
        // rather than beneath the page. See `the_page_asks_for_a_playlist_this_router_serves`.
        .route(WATCH_PATH, get(to_slash))
        .route(&format!("{WATCH_PATH}/"), get(page))
        .route(&format!("{WATCH_PATH}/hls.light.min.js"), get(hls_js))
        .route(&format!("{WATCH_PATH}/hls-LICENSE.txt"), get(hls_license))
        // **The playlist is served by hand and its segments by the file server**, which is one route
        // for one file and worth it for what it carries. A file server names a type from the
        // extension, and for `.m3u8` that is the legacy `audio/x-mpegurl`; a television's own media
        // pipeline may decline a playlist that is not `application/vnd.apple.mpegurl`, and declining
        // is all it does — there is no error for anybody to read. It also refuses caching, because a
        // playlist is rewritten every segment and a client holding an old one follows a window that
        // has moved.
        .route(&format!("{STREAM_PREFIX}/{PLAYLIST}"), get(playlist))
        .with_state(PathBuf::from(dir))
        .nest_service(STREAM_PREFIX, segments(dir))
}

/// How many fragments a viewer may fall behind before it is sent away to start again.
///
/// **About three seconds.** The muxer writes a fragment per packet, which at thirty frames a second
/// and 48 kHz sound is under eighty a second. A viewer further behind than this is better restarted
/// at a keyframe than fed a stream it can never catch up with.
const FRAGMENTS_KEPT: usize = 256;

/// The stream as fragments, for the page's socket.
///
/// **Handed in by the machine and fed from its stream thread.** [`Live::publish_init`] and
/// [`Live::publish_fragment`] are synchronous and never wait, so a thread with a frame deadline can
/// call them. What they take is a copy of what the encoder already produced, and this crate never
/// links an encoder to get it.
#[derive(Clone)]
pub struct Live {
    inner: Arc<Shared>,
}

struct Shared {
    /// The initialisation segment. Every viewer is sent this first, and one arriving before the
    /// stream has started waits for it.
    init: watch::Sender<Option<Bytes>>,
    fragments: broadcast::Sender<Fragment>,
}

/// One fragment, and whether a viewer may start on it.
#[derive(Clone, Debug)]
struct Fragment {
    bytes: Bytes,
    key: bool,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("viewers", &self.inner.fragments.receiver_count())
            .finish_non_exhaustive()
    }
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

impl Live {
    /// A stream with nothing in it yet.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Shared {
                init: watch::Sender::new(None),
                fragments: broadcast::Sender::new(FRAGMENTS_KEPT),
            }),
        }
    }

    /// Sets the initialisation segment every viewer is sent first.
    pub fn publish_init(&self, bytes: impl Into<Bytes>) {
        self.inner.init.send_replace(Some(bytes.into()));
    }

    /// Sends one fragment to every viewer. `key` says a viewer may start on it.
    ///
    /// Nobody watching is not an error: the stream is written whether or not anybody is.
    pub fn publish_fragment(&self, bytes: impl Into<Bytes>, key: bool) {
        let _ = self.inner.fragments.send(Fragment {
            bytes: bytes.into(),
            key,
        });
    }
}

/// What a viewer's socket does with the next thing off the ring.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Send(Bytes),
    Skip,
    Close,
}

/// **A viewer starts on a keyframe, and one that falls behind is sent away.** A decoder handed a
/// fragment that depends on one it never saw shows nothing until the next keyframe. A viewer that
/// missed fragments has a gap in its timeline, and a `<video>` stops at a gap and waits there.
/// Closing is what the page reconnects from, and a fresh start is at a keyframe again.
fn step(started: &mut bool, received: Result<Fragment, RecvError>) -> Step {
    match received {
        Ok(fragment) if *started || fragment.key => {
            *started = true;
            Step::Send(fragment.bytes)
        }
        Ok(_) => Step::Skip,
        Err(RecvError::Lagged(_) | RecvError::Closed) => Step::Close,
    }
}

/// Feeds one viewer until it goes away or falls behind.
async fn pump_fragments(socket: WebSocket, live: Live) {
    let (mut sink, mut incoming) = socket.split();
    // Subscribed before the initialisation segment is awaited, so nothing published in between is
    // lost. What arrives before the first keyframe is skipped anyway.
    let mut fragments = live.inner.fragments.subscribe();
    let mut ready = live.inner.init.subscribe();
    let init = loop {
        tokio::select! {
            message = incoming.next() => match message {
                None | Some(Err(_) | Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            },
            init = ready.wait_for(Option::is_some) => match init {
                Ok(init) => break init.clone(),
                Err(_) => return,
            },
        }
    };
    let Some(init) = init else { return };
    if sink.send(Message::Binary(init)).await.is_err() {
        return;
    }

    let mut started = false;
    loop {
        tokio::select! {
            // Read only to notice the viewer leaving. The page sends nothing.
            message = incoming.next() => match message {
                None | Some(Err(_) | Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            },
            received = fragments.recv() => match step(&mut started, received) {
                Step::Send(bytes) => {
                    if sink.send(Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                Step::Skip => {}
                Step::Close => {
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code: close_code::AGAIN,
                            reason: "fell behind; reconnect to start at a keyframe".into(),
                        })))
                        .await;
                    break;
                }
            },
        }
    }
}

/// The name the muxer writes and a client asks for.
///
/// Spelled here rather than taken from `km-stream`, because this crate serves streams it does not
/// produce and must not link an encoder to name a file.
pub const PLAYLIST: &str = "live.m3u8";

/// The playlist, with the media type a strict player insists on.
async fn playlist(axum::extract::State(dir): axum::extract::State<PathBuf>) -> Response {
    match tokio::fs::read(dir.join(PLAYLIST)).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        // Absent rather than empty: a machine that is not streaming has no playlist, and a client
        // told so retries where one told "here is nothing" would play silence.
        Err(_) => (
            axum::http::StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "no stream is running",
        )
            .into_response(),
    }
}

/// Sends `/watch` to `/watch/`, so the page's relative links resolve beneath it.
///
/// **Temporary, because a browser keeps a permanent one** — the same reasoning `/admin/`'s redirect
/// carries. A cached redirect is followed without this server being asked, so a permanent one is a
/// promise about every later build.
async fn to_slash() -> Response {
    Redirect::temporary(&format!("{WATCH_PATH}/")).into_response()
}

/// A file server over the encoder's directory.
///
/// **No directory listing and no following of links out of it**, which `ServeDir` gives by default:
/// what is served is what the muxer wrote, and a client only ever asks for names the playlist gave
/// it.
fn segments(dir: &Path) -> tower_http::services::ServeDir {
    tower_http::services::ServeDir::new(PathBuf::from(dir))
        // A playlist is rewritten every segment, so a client that cached one would follow a window
        // that has already moved on. The segments themselves are named uniquely and never change,
        // but they are also deleted as they age out, so nothing is gained by letting either sit in
        // a cache.
        .append_index_html_on_directories(false)
}

/// The page, with caching refused.
///
/// A television that kept this would keep the script tag and the playlist name it points at, which
/// is exactly what an upgrade needs to be able to change.
async fn page() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        WATCH_HTML,
    )
        .into_response()
}

/// The library, which may be cached: it is pinned to a version and changes only with the build.
async fn hls_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        HLS_JS,
    )
        .into_response()
}

/// ...and its license beside it.
async fn hls_license() -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        HLS_LICENSE,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    /// Drives one request at a router over `dir` and hands back the parts worth asserting on.
    async fn get_path(dir: &Path, path: &str) -> (StatusCode, String, String) {
        let response = router(dir, None)
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("a request with no body"),
            )
            .await
            .expect("the router answers every request");
        let status = response.status();
        let header_value = |name: header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        let content_type = header_value(header::CONTENT_TYPE);
        let location = header_value(header::LOCATION);
        (status, content_type, location)
    }

    /// A directory with a playlist in it, for the tests that need one.
    fn with_playlist() -> tempdir::Dir {
        let dir = tempdir::Dir::new();
        std::fs::write(dir.path().join(PLAYLIST), "#EXTM3U\n").expect("writing a playlist");
        dir
    }

    /// The bare spelling redirects rather than serving, so relative links resolve beneath it.
    ///
    /// Serving the page at both would work on one spelling and fail on the other, and the failure
    /// is invisible: the page loads, the script beside it 404s, and only a browser with no HLS of
    /// its own shows nothing.
    #[tokio::test]
    async fn the_bare_path_redirects_onto_the_slash() {
        let dir = tempdir::Dir::new();
        let (status, _, location) = get_path(dir.path(), WATCH_PATH).await;
        assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(location, "/watch/");
    }

    /// ...and everything the page asks for afterwards is there beneath it.
    #[tokio::test]
    async fn the_page_and_what_it_loads_are_served() {
        let dir = tempdir::Dir::new();
        for (path, expected) in [
            ("/watch/", "text/html; charset=utf-8"),
            ("/watch/hls.light.min.js", "text/javascript; charset=utf-8"),
            ("/watch/hls-LICENSE.txt", "text/plain; charset=utf-8"),
        ] {
            let (status, content_type, _) = get_path(dir.path(), path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(content_type, expected, "{path}");
        }
    }

    /// The playlist carries the media type a strict player insists on.
    ///
    /// A file server would name `audio/x-mpegurl` from the extension, and a television's own media
    /// pipeline may decline that and say nothing about why.
    #[tokio::test]
    async fn the_playlist_is_typed_for_a_television() {
        let dir = with_playlist();
        let (status, content_type, _) = get_path(dir.path(), "/stream/live.m3u8").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/vnd.apple.mpegurl");
    }

    /// The address the page asks the playlist for is one this router answers.
    ///
    /// **The two halves are written in different languages in different files, and nothing else
    /// compares them.** Every other test here drives a path spelled in the test, so the page could
    /// name anything at all and they would still pass — which is how a relative name that resolved
    /// to `/watch/stream/live.m3u8` went unnoticed. The failure it produces is silent in the worst
    /// way: the page loads, the picture never starts, and the machine's side looks perfect.
    #[tokio::test]
    async fn the_page_asks_for_a_playlist_this_router_serves() {
        let asked_for = playlist_the_page_asks_for();
        assert!(
            asked_for.starts_with('/'),
            "the page names `{asked_for}`, which resolves against `/watch/` rather than the root"
        );

        let dir = with_playlist();
        let (status, content_type, _) = get_path(dir.path(), &asked_for).await;
        assert_eq!(status, StatusCode::OK, "{asked_for}");
        assert_eq!(content_type, "application/vnd.apple.mpegurl");
    }

    /// The tab wears the machine's mark, and it is the one this product actually serves.
    ///
    /// **Not this router's, which is the point of asserting it**: the icon is `routes.rs`'s, mounted
    /// on the outer router beside everything else a run offers, so nothing here would notice the
    /// page naming a path that is not there — a browser asks, gets a 404, and draws a blank sheet.
    #[test]
    fn the_page_wears_the_mark_the_machine_serves() {
        const NAMED: &str = "<link rel=\"icon\" href=\"";
        let href = WATCH_HTML
            .split_once(NAMED)
            .expect("the page links an icon")
            .1
            .split_once('"')
            .expect("the href is a closed string")
            .0;
        assert_eq!(href, crate::routes::ICON_PATH);
    }

    /// The path out of the page's own `PLAYLIST`, which is the one thing it fetches by name.
    ///
    /// Read rather than restated: a copy here would be a second spelling that can drift from the
    /// page exactly as the router's did.
    fn playlist_the_page_asks_for() -> String {
        named_in_page("PLAYLIST")
    }

    /// The string a `const` in the page's script holds.
    fn named_in_page(constant: &str) -> String {
        let named = format!("const {constant} = \"");
        let after = WATCH_HTML
            .split_once(&named)
            .unwrap_or_else(|| panic!("the page has a `const {constant}` it can be read out of"))
            .1;
        after
            .split_once('"')
            .expect("the name is a closed string literal")
            .0
            .to_owned()
    }

    /// A GET for `path` on a router that has, or has not, been handed the stream as fragments.
    async fn status_of(path: &str, live: Option<Live>) -> StatusCode {
        let dir = tempdir::Dir::new();
        router(dir.path(), live)
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("a request with no body"),
            )
            .await
            .expect("the router answers every request")
            .status()
    }

    /// The page's socket is a path the router mounts, and only when it has a stream to put on it.
    ///
    /// A plain GET is refused for not being an upgrade, which is still an answer from the route.
    /// A 404 is the file server beneath it, saying the route is not there.
    #[tokio::test]
    async fn the_page_asks_for_a_socket_mounted_only_with_a_stream_to_carry() {
        let asked_for = named_in_page("SOCKET");
        assert_eq!(asked_for, format!("{STREAM_PREFIX}/{SOCKET}"));
        assert_ne!(
            status_of(&asked_for, Some(Live::new())).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(status_of(&asked_for, None).await, StatusCode::NOT_FOUND);
    }

    fn fragment(key: bool) -> Result<Fragment, RecvError> {
        Ok(Fragment {
            bytes: Bytes::from_static(if key { b"key" } else { b"delta" }),
            key,
        })
    }

    #[test]
    fn a_viewer_starts_on_a_keyframe_and_then_takes_everything() {
        let mut started = false;
        assert_eq!(step(&mut started, fragment(false)), Step::Skip);
        assert_eq!(
            step(&mut started, fragment(true)),
            Step::Send(Bytes::from_static(b"key"))
        );
        assert_eq!(
            step(&mut started, fragment(false)),
            Step::Send(Bytes::from_static(b"delta"))
        );
    }

    /// A viewer that missed fragments would stop at the gap, so it is sent away to start again.
    #[test]
    fn a_viewer_that_falls_behind_is_closed_even_mid_stream() {
        let mut started = true;
        assert_eq!(step(&mut started, Err(RecvError::Lagged(3))), Step::Close);
        assert_eq!(step(&mut started, Err(RecvError::Closed)), Step::Close);
    }

    /// A machine that is not streaming has no playlist, rather than an empty one.
    ///
    /// A client told "here is nothing" plays silence; one told the playlist is absent retries.
    #[tokio::test]
    async fn a_machine_that_is_not_streaming_has_no_playlist() {
        let dir = tempdir::Dir::new();
        let (status, ..) = get_path(dir.path(), "/stream/live.m3u8").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Nothing outside the directory is reachable through the segment path.
    #[tokio::test]
    async fn the_segment_path_does_not_climb_out() {
        let dir = with_playlist();
        let (status, ..) = get_path(dir.path(), "/stream/../../secrets.txt").await;
        assert_ne!(status, StatusCode::OK);
    }

    /// A directory that lasts as long as the test, and takes itself away afterwards.
    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct Dir(PathBuf);

        impl Dir {
            pub fn new() -> Self {
                // The process id and a counter, so two tests running at once never meet. Nothing
                // here needs unpredictability — this is a scratch directory, not a secret.
                static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let path =
                    std::env::temp_dir().join(format!("km-watch-{}-{n}", std::process::id()));
                std::fs::create_dir_all(&path).expect("a scratch directory");
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
