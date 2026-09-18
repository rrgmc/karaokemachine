//! Against a real listening server.
//!
//! `tests/surface.rs` drives the router as a service, which covers routing, extraction and the ACL
//! but never opens a socket. These tests bind a real port, because three things only work — or only
//! fail — on a real connection:
//!
//! * the **WebSocket** event stream, which needs an actual HTTP upgrade;
//! * the **peer address**, which the loopback ACL rule depends on and which a service-level test can
//!   only simulate by injecting an extension;
//! * the **static file layer** for the dev remote, and the singer-facing remote merged at the root.
//!
//! HTTP requests here are written by hand over TCP rather than through a client crate. It is a dozen
//! lines, it adds no dependency, and for `GET /path` there is nothing a client would do better.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use km_api::testing::TestMachine;
use km_api::{ApiConfig, ApiState, Extras, Listening, bind, bind_with};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// A connected event stream.
type EventSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A running server, stopped when this is dropped.
struct Server {
    addr: SocketAddr,
    machine: Arc<TestMachine>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn start(config: ApiConfig) -> Self {
        Self::start_with_remote(config, None).await
    }

    async fn start_with_remote(config: ApiConfig, remote: Option<axum::Router>) -> Self {
        Self::start_with(
            config,
            Extras {
                remote,
                admin: None,
                stream: None,
            },
        )
        .await
    }

    async fn start_with(config: ApiConfig, extras: Extras) -> Self {
        Self::start_tapped(config, extras, None).await
    }

    /// The same, with a log tap installed before the router is built.
    ///
    /// **Before**, because that is when `router_with` reads it — a tap set afterwards would leave a
    /// machine keeping a log with no route saying so, which is the failure `set_log_tap`'s own note
    /// is about.
    async fn start_tapped(
        config: ApiConfig,
        extras: Extras,
        tap: Option<km_logtap::LogTap>,
    ) -> Self {
        let machine = TestMachine::with_catalog(4).shared();
        let state = ApiState::from_machine(machine.clone(), config.on_ephemeral_port());
        if let Some(tap) = tap {
            assert!(state.set_log_tap(tap), "a server installs its tap once");
        }
        let listening: Listening = bind_with(state, extras).await.expect("bind");
        let addr = listening.local_addr;
        let task = tokio::spawn(async move {
            let _ = listening.serve().await;
        });
        Self {
            addr,
            machine,
            task,
        }
    }

    async fn plain() -> Self {
        Self::start(ApiConfig::default().without_mdns()).await
    }

    /// One request, one connection, whole response as text.
    async fn raw(&self, method: &str, path: &str, body: Option<&str>) -> String {
        let mut stream = TcpStream::connect(self.addr).await.expect("connect");
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.addr
        );
        match body {
            Some(body) => {
                request.push_str("content-type: application/json\r\n");
                request.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
                request.push_str(body);
            }
            None => request.push_str("content-length: 0\r\n\r\n"),
        }
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write the request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read the response");
        String::from_utf8_lossy(&response).into_owned()
    }

    async fn get(&self, path: &str) -> String {
        self.raw("GET", path, None).await
    }

    /// The JSON body of a response, after the headers.
    async fn json(&self, method: &str, path: &str, body: Option<&str>) -> Value {
        let text = self.raw(method, path, body).await;
        let (_, payload) = text
            .split_once("\r\n\r\n")
            .unwrap_or_else(|| panic!("no body in: {text}"));
        serde_json::from_str(payload.trim())
            .unwrap_or_else(|error| panic!("not JSON ({error}): {payload}"))
    }

    /// Opens the event stream.
    ///
    /// The concrete type rather than an `impl Trait`: the caller needs `expect` on a send, which
    /// needs the sink error to be `Debug`, and an opaque return type hides that.
    async fn events(&self) -> EventSocket {
        let url = format!("ws://{}/api/v1/events", self.addr);
        let (socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("the event stream accepts a websocket");
        socket
    }

    /// Opens the log stream, through the mirror where it needs no password.
    async fn logs(&self) -> EventSocket {
        let url = format!("ws://{}/dev/api/v1/admin/logs/stream", self.addr);
        let (socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("the log stream accepts a websocket");
        socket
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The next event of a given kind, or a panic if it does not arrive.
async fn next_event(socket: &mut EventSocket, kind: &str) -> Value {
    let deadline = Duration::from_secs(5);
    let found = tokio::time::timeout(deadline, async {
        loop {
            let message = socket.next().await.expect("the stream stays open");
            let Ok(Message::Text(text)) = message else {
                continue;
            };
            let event: Value = serde_json::from_str(&text).expect("an event is JSON");
            if event["event"] == kind {
                return event;
            }
        }
    })
    .await;
    found.unwrap_or_else(|_| panic!("no '{kind}' event within {deadline:?}"))
}

// -- the event stream ----------------------------------------------------------------------------

#[tokio::test]
async fn the_first_message_is_the_current_state() {
    let server = Server::plain().await;
    let mut socket = server.events().await;
    // Sent immediately, not on the next 250 ms tick: a remote that connects between ticks would
    // otherwise render an empty screen for long enough to look broken.
    let event = next_event(&mut socket, "state").await;
    assert_eq!(event["state"]["transport"], "idle");
    assert_eq!(event["state"]["queue_len"], 0);
}

#[tokio::test]
async fn queueing_over_http_reaches_a_websocket_client() {
    let server = Server::plain().await;
    let mut socket = server.events().await;
    next_event(&mut socket, "state").await;

    server
        .json("POST", "/api/v1/queue", Some(r#"{"number":"1001"}"#))
        .await;

    let event = next_event(&mut socket, "queue_changed").await;
    assert_eq!(event["queue"]["len"], 1);
    assert_eq!(event["queue"]["entries"][0]["number"], "1001");
}

#[tokio::test]
async fn playing_announces_the_song_that_started() {
    let server = Server::plain().await;
    server
        .json("POST", "/api/v1/queue", Some(r#"{"number":"1001"}"#))
        .await;

    let mut socket = server.events().await;
    next_event(&mut socket, "state").await;
    server.json("POST", "/api/v1/transport/play", None).await;

    let started = next_event(&mut socket, "song_started").await;
    assert_eq!(started["now_playing"]["origin"]["number"], "1001");
    assert_eq!(started["now_playing"]["title"], "Song 1001");
}

#[tokio::test]
async fn stopping_announces_why_the_song_ended() {
    let server = Server::plain().await;
    server
        .json("POST", "/api/v1/queue", Some(r#"{"number":"1001"}"#))
        .await;
    server.json("POST", "/api/v1/transport/play", None).await;

    let mut socket = server.events().await;
    next_event(&mut socket, "state").await;
    server.json("POST", "/api/v1/transport/stop", None).await;

    let ended = next_event(&mut socket, "song_ended").await;
    // A remote showing "up next" needs to tell "the singer finished" from "somebody hit stop".
    assert_eq!(ended["reason"], "stopped");
}

#[tokio::test]
async fn changing_the_key_reaches_every_connected_remote() {
    let server = Server::plain().await;
    let mut first = server.events().await;
    let mut second = server.events().await;
    next_event(&mut first, "state").await;
    next_event(&mut second, "state").await;

    server
        .json("PUT", "/api/v1/settings", Some(r#"{"transpose":-2}"#))
        .await;

    // Both phones learn about it, which is the whole point of the channel: the person who changed
    // the key is not the only one holding a remote.
    for socket in [&mut first, &mut second] {
        let event = next_event(socket, "settings_changed").await;
        assert_eq!(event["settings"]["transpose"], -2);
    }
}

#[tokio::test]
async fn a_mic_change_and_a_wallpaper_change_both_arrive() {
    let server = Server::plain().await;
    let mut socket = server.events().await;
    next_event(&mut socket, "state").await;

    server
        .json("PUT", "/api/v1/mics/mic1", Some(r#"{"muted":true}"#))
        .await;
    let mics = next_event(&mut socket, "mics_changed").await;
    assert_eq!(mics["mics"]["mics"][0]["muted"], true);

    server.json("POST", "/api/v1/wallpapers/next", None).await;
    let wallpaper = next_event(&mut socket, "wallpaper_changed").await;
    assert!(wallpaper["current"].is_string());
}

#[tokio::test]
async fn the_state_event_carries_the_playback_position() {
    let server = Server::plain().await;
    server
        .json("POST", "/api/v1/queue", Some(r#"{"number":"1001"}"#))
        .await;
    server.json("POST", "/api/v1/transport/play", None).await;
    // Position rides on `state`; per-syllable position is never streamed, which is why a remote has
    // to interpolate from this plus the lyric timeline.
    server.machine.set_position_ms(12_345);

    let mut socket = server.events().await;
    let event = next_event(&mut socket, "state").await;
    assert_eq!(event["state"]["position_ms"], 12_345);
}

#[tokio::test]
async fn the_stream_ignores_what_a_client_sends_rather_than_closing_on_it() {
    let server = Server::plain().await;
    let mut socket = server.events().await;
    next_event(&mut socket, "state").await;

    // Control is over REST. A WebSocket that also accepted commands would need its own
    // authorization story, and one ACL is enough — but a stray message must not kill the stream.
    socket
        .send(Message::Text("please skip the song".into()))
        .await
        .expect("send");

    server
        .json("POST", "/api/v1/queue", Some(r#"{"number":"1002"}"#))
        .await;
    let event = next_event(&mut socket, "queue_changed").await;
    assert_eq!(event["queue"]["len"], 1);
}

#[tokio::test]
async fn a_client_going_away_does_not_disturb_the_others() {
    let server = Server::plain().await;
    let mut staying = server.events().await;
    {
        let mut leaving = server.events().await;
        next_event(&mut leaving, "state").await;
        let _ = leaving.close(None).await;
    }
    next_event(&mut staying, "state").await;

    server
        .json("POST", "/api/v1/queue", Some(r#"{"number":"1001"}"#))
        .await;
    let event = next_event(&mut staying, "queue_changed").await;
    assert_eq!(event["queue"]["len"], 1);
}

/// The event stream is public and stays public, whatever a machine's password is.
///
/// **This inverts the test it replaced**, which set `events.subscribe` to `admin` and asserted the
/// WebSocket upgrade was refused. There is no way to restrict it any more — it sits outside
/// `/api/v1/admin/` and always will — and that is load-bearing rather than incidental: a browser
/// cannot put an `Authorization` header on a WebSocket, so a stream that could be gated would be a
/// stream a page could not open. `km-remote-pages` re-broadcasts as SSE for the same reason.
#[tokio::test]
async fn the_event_stream_stays_open_on_a_machine_with_a_password() {
    let config = ApiConfig::default()
        .without_mdns()
        .with_password("open sesame")
        .expect("hash");
    let server = Server::start(config).await;

    let url = format!("ws://{}/api/v1/events", server.addr);
    assert!(
        tokio_tungstenite::connect_async(&url).await.is_ok(),
        "a password must not close the event stream"
    );
}

// -- a real connection ---------------------------------------------------------------------------

#[tokio::test]
async fn discovery_answers_over_real_http() {
    let server = Server::plain().await;
    let body = server.json("GET", "/api/v1/discover", None).await;
    assert_eq!(body["app"], "karaokemachine");
    assert_eq!(body["song_count"], 4);
    assert_eq!(body["port"], server.addr.port());
}

#[tokio::test]
async fn the_address_refresher_does_not_undo_the_port_that_was_bound() {
    // The test above raced this and lost on Linux and macOS while passing on Windows, which read as
    // flakiness and was not: `tokio::time::interval` fires its first tick at once, and the refresher
    // re-resolved `config.bind` — port 0 here, because the harness asks for an ephemeral port — over
    // the real address `bind` had already worked out. Waiting for that first tick to have happened is
    // what makes the assertion deterministic instead of a coin toss.
    let server = Server::plain().await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let body = server.json("GET", "/api/v1/discover", None).await;
    assert_eq!(
        body["port"],
        server.addr.port(),
        "a remote dialling this number has to reach the machine"
    );
    let connect = server.json("GET", "/api/v1/connect", None).await;
    assert_eq!(connect["port"], server.addr.port());
    assert!(
        connect["bound"]
            .as_str()
            .is_some_and(|bound| bound.ends_with(&server.addr.port().to_string())),
        "the bound address is reported as bound, not as requested: {connect}"
    );
}

/// An admin route is refused over a real socket, from loopback, with no token.
///
/// **Two things this deliberately does not depend on.** Where the request came from is not an
/// input — there is no loopback exemption — and a machine that cannot verify a token refuses rather
/// than admits, so a machine with no password refuses as well.
///
/// It stays a *live* test rather than moving to the service-level file because the thing worth
/// proving over a real socket is that the gate runs before any handler and answers on the wire.
#[tokio::test]
async fn an_admin_route_is_refused_over_a_real_socket_even_from_loopback() {
    let server = Server::plain().await;
    let response = server
        .raw("PUT", "/api/v1/admin/demo", Some(r#"{"enabled":true}"#))
        .await;
    assert!(
        response.starts_with("HTTP/1.1 401"),
        "expected 401, got: {response}"
    );
}

#[tokio::test]
async fn binding_reports_the_address_before_anything_is_served() {
    // `km-app` needs this ordering: the display comes up first, and an idle screen with no address
    // on it is the failure the whole connect panel exists to avoid.
    let state = ApiState::from_machine(
        TestMachine::new().shared(),
        ApiConfig::default().without_mdns().on_ephemeral_port(),
    );
    let listening = bind(state).await.expect("bind");
    assert_ne!(listening.local_addr.port(), 0);
    assert_eq!(listening.connect.port, listening.local_addr.port());
    // Loopback, so it says so rather than offering a LAN URL that would not work.
    assert!(!listening.connect.reachable);
}

// -- static files --------------------------------------------------------------------------------

#[tokio::test]
async fn the_dev_remote_is_served_when_it_is_configured() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tools/dev/remote")
        .canonicalize()
        .expect("the dev remote is in the repository");
    let server = Server::start(ApiConfig::default().without_mdns().with_dev_remote(dir)).await;

    let response = server.get("/dev/index.html").await;
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected the dev remote, got: {}",
        response.lines().next().unwrap_or_default()
    );
    // **The real page, and checked against a marker only the page has.** Asserting `/api/v1` or
    // `KaraokeMachine` would pass against the *landing* page too, which is precisely the thing this
    // exists to catch. `km.dev.base` is the page's own `localStorage` key; the other tests here use
    // it and say why.
    assert!(
        response.contains("km.dev.base"),
        "something other than the dev console was served: {response}"
    );
}

/// The case a staged build is actually in: no directory anywhere, and the page still served.
///
/// The console is served out of the binary, so a distributable that stages no `dev/` directory
/// still answers `/dev/` with it rather than with the landing page — which a reader cannot tell
/// apart from the feature being switched off.
#[tokio::test]
async fn the_dev_remote_is_served_from_the_binary_when_no_directory_exists() {
    let server = Server::start(ApiConfig::default().without_mdns().with_dev_console()).await;

    for path in ["/dev", "/dev/", "/dev/index.html"] {
        let response = server.get(path).await;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "{path} did not answer 200: {}",
            response.lines().next().unwrap_or_default()
        );
        // `km.dev.base` is unique to the page itself. The landing page mentions `/dev/` in prose, so
        // searching for the words would pass against the very thing this is meant to catch.
        assert!(
            response.contains("km.dev.base"),
            "{path} served something other than the dev remote"
        );
    }
}

/// A directory that does not exist must fall back to the built-in copy rather than to the landing
/// page — a stale path in settings should not silently cost somebody their remote.
#[tokio::test]
async fn a_missing_dev_remote_directory_falls_back_to_the_built_in_copy() {
    let server = Server::start(
        ApiConfig::default()
            .without_mdns()
            .with_dev_remote("/definitely/not/here"),
    )
    .await;

    let response = server.get("/dev/index.html").await;
    assert!(response.contains("km.dev.base"), "got: {response}");
}

#[tokio::test]
async fn the_dev_remote_is_absent_when_it_is_turned_off() {
    let mut config = ApiConfig::default().without_mdns();
    config.serve_dev_remote = false;
    let server = Server::start(config).await;
    let response = server.get("/dev/index.html").await;
    // Falls through to the root fallback rather than serving a development tool on a product
    // surface. Checked against a marker unique to the page itself -- the landing page legitimately
    // mentions /dev/, so searching for the words would pass whatever was served.
    assert!(
        !response.contains("km.dev.base"),
        "the dev remote was served with serve_dev_remote off"
    );
    assert!(
        response.contains("No singer-facing remote"),
        "got: {response}"
    );
}

/// **Nobody has to turn it off.**
///
/// Two layers answer this question — `ApiConfig`'s default here and `Settings::serve_dev_remote` —
/// and the shipped answer is the settings one, so the two have to agree. Both say off, and this
/// pins the layer that does not read settings at all: the one every embedder of `km-api` other than
/// the machine gets.
#[tokio::test]
async fn the_dev_remote_is_absent_unless_something_asks_for_it() {
    let server = Server::start(ApiConfig::default().without_mdns()).await;
    let response = server.get("/dev/index.html").await;
    assert!(
        !response.contains("km.dev.base"),
        "the dev remote was served by a default configuration"
    );
}

/// **One switch is not enough, in either direction.**
///
/// The console's own API asks for no password at all, so what keeps it off a machine in a living
/// room is that two separate things have to be true. Both single-switch states are pinned, because
/// only one of them is the obvious one: `serve_dev_remote` alone is what an owner ticking the box
/// produces, and it must not serve.
#[tokio::test]
async fn the_console_needs_both_switches_and_neither_alone_will_do() {
    for (debug, dev_remote) in [(false, false), (true, false), (false, true)] {
        let mut config = ApiConfig::default().without_mdns();
        config.debug_enabled = debug;
        config.serve_dev_remote = dev_remote;
        let server = Server::start(config).await;

        let page = server.get("/dev/index.html").await;
        assert!(
            !page.contains("km.dev.base"),
            "the console was served with debug={debug} dev_remote={dev_remote}"
        );
        // A 404 and not a 401: the mirror is not mounted, so there is nothing there to refuse.
        let mirrored = server.get("/dev/api/v1/state").await;
        assert!(
            mirrored.starts_with("HTTP/1.1 404"),
            "the mirrored API answered with debug={debug} dev_remote={dev_remote}: {}",
            mirrored.lines().next().unwrap_or_default()
        );
    }
}

/// The mirror is mounted beside the page, and neither swallows the other.
///
/// **Written because it is the one thing about this arrangement that could not be settled by
/// reading.** The directory arm is a `ServeDir` nested at `/dev`, which claims `/dev/{*rest}`, and
/// whether a static `/dev/api/v1` declared beside it wins is a property of axum's router rather
/// than of anything here. Both arms are driven — the directory one and the built-in one — because
/// only the first has the wildcard.
#[tokio::test]
async fn the_mirrored_api_and_the_page_share_the_dev_prefix() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tools/dev/remote")
        .canonicalize()
        .expect("the dev console is in the repository");

    for config in [
        ApiConfig::default().without_mdns().with_dev_remote(dir),
        ApiConfig::default().without_mdns().with_dev_console(),
    ] {
        let server = Server::start(config).await;

        let page = server.get("/dev/index.html").await;
        assert!(page.contains("km.dev.base"), "the page: {page}");

        let state = server.get("/dev/api/v1/state").await;
        assert!(
            state.starts_with("HTTP/1.1 200"),
            "the mirrored API: {}",
            state.lines().next().unwrap_or_default()
        );
    }
}

/// **Every admin path is open under the mirror, and refused under the real prefix.**
///
/// The sweep in `tests/surface.rs` asserts the second half against the whole of
/// [`km_api::routes::SURFACE`]; this asserts the first, over a real socket, for the paths that make
/// the point. Not the whole table, because these actually run: a sweep here would rescan packages
/// and reset sessions on the way past.
#[tokio::test]
async fn nothing_under_the_mirror_asks_for_a_password() {
    let server = Server::start(ApiConfig::default().without_mdns().with_dev_console()).await;

    for (method, path) in [
        ("GET", "/packages"),
        ("POST", "/admin/packages/rescan"),
        ("POST", "/admin/sessions/reset"),
    ] {
        let refused = server.raw(method, &format!("/api/v1{path}"), None).await;
        let allowed = server
            .raw(method, &format!("/dev/api/v1{path}"), None)
            .await;

        if path.starts_with("/admin/") {
            assert!(
                refused.starts_with("HTTP/1.1 401"),
                "{method} /api/v1{path} should still want a token: {}",
                refused.lines().next().unwrap_or_default()
            );
        }
        assert!(
            !allowed.starts_with("HTTP/1.1 401"),
            "{method} /dev/api/v1{path} asked for a password: {}",
            allowed.lines().next().unwrap_or_default()
        );
    }
}

/// The root serves whatever router it was handed, and the API keeps its own paths.
///
/// The remote arrives as a `Router` rendering in this process rather than as built files under a
/// `ServeDir`, so what has to be pinned is that merging one in does not cost the API a route.
#[tokio::test]
async fn a_remote_handed_in_is_served_at_the_root_without_disturbing_the_api() {
    let remote = axum::Router::new()
        .route("/", axum::routing::get(|| async { "the singer's remote" }))
        .route("/now", axum::routing::get(|| async { "now playing" }));

    let server = Server::start_with_remote(ApiConfig::default().without_mdns(), Some(remote)).await;

    assert!(server.get("/").await.contains("the singer's remote"));
    assert!(server.get("/now").await.contains("now playing"));

    // The API is untouched by what was merged over the root.
    let discovery = server.json("GET", "/api/v1/discover", None).await;
    assert_eq!(discovery["app"], "karaokemachine");

    // And so is the answer to an API path that does not exist — the one thing a JSON client must
    // never have answered with somebody's HTML.
    let unknown = server.json("GET", "/api/v1/nonsense", None).await;
    assert_eq!(unknown["error"], km_api::ApiError::UNKNOWN_ENDPOINT);
}

/// A page handed in as the owner's is nested under `/admin`, and reachable with or without a slash.
///
/// **Both spellings, because only one of them worked.** `nest("/admin", …)` matches `/admin` and
/// answers 404 to `/admin/` — and the trailing slash is what a person types and what every link
/// ending in a directory produces, so the URL this feature is described by everywhere was the one
/// that did not work. The redirect is what fixes it and this is what keeps it fixed.
#[tokio::test]
async fn a_page_handed_in_as_the_owners_is_nested_under_admin() {
    let admin = axum::Router::new()
        .route("/", axum::routing::get(|| async { "the owner's page" }))
        .route(
            "/songs",
            axum::routing::get(|| async { "what is installed" }),
        );
    let remote =
        axum::Router::new().route("/", axum::routing::get(|| async { "the singer's remote" }));

    let server = Server::start_with(
        ApiConfig::default().without_mdns(),
        Extras {
            remote: Some(remote),
            admin: Some(admin),
            stream: None,
        },
    )
    .await;

    assert!(server.get("/admin").await.contains("the owner's page"));
    assert!(
        server
            .get("/admin/songs")
            .await
            .contains("what is installed")
    );

    // The trailing slash redirects rather than 404ing. Asserted on the response line and the
    // `Location`, because `get` returns the raw response and follows nothing -- and the status is
    // part of it: a permanent redirect is kept by the browser that receives it, which makes a
    // landing path a promise about every later build.
    let slashed = server.get("/admin/").await;
    assert!(slashed.contains("307 Temporary Redirect"), "{slashed}");
    assert!(
        slashed.to_lowercase().contains("location: /admin"),
        "{slashed}"
    );

    // **Nested before the remote is merged**, so the remote -- which owns whatever the API did not
    // claim at the root -- cannot shadow it. Without the ordering this line would read
    // "the singer's remote".
    assert!(!server.get("/admin").await.contains("the singer's remote"));
    assert!(server.get("/").await.contains("the singer's remote"));

    // And the API is untouched by either.
    let discovery = server.json("GET", "/api/v1/discover", None).await;
    assert_eq!(discovery["app"], "karaokemachine");
}

#[tokio::test]
async fn an_unknown_api_path_answers_json_and_not_the_landing_page() {
    let server = Server::plain().await;
    let body = server.json("GET", "/api/v1/nonsense", None).await;
    // A client that got HTML with a 200 here would try to parse it as JSON and report something
    // unrelated to what actually went wrong.
    assert_eq!(body["error"], km_api::ApiError::UNKNOWN_ENDPOINT);
}

/// **What the log stream is for, over a real socket: the tail, and then what happens next.**
///
/// A pane that opened empty and filled up from nothing would be useless for the question this whole
/// feature exists to answer, which is always about a moment that has already passed.
#[tokio::test]
async fn the_log_stream_opens_with_what_was_already_said() {
    let tap = km_logtap::LogTap::new();
    tap.push(record("said before anybody was listening"));

    let server = Server::start_tapped(
        ApiConfig::default().without_mdns().with_dev_console(),
        Extras::default(),
        Some(tap.clone()),
    )
    .await;

    let mut socket = server.logs().await;
    let opening = next_log(&mut socket).await;
    assert_eq!(opening["event"], "record");
    assert_eq!(
        opening["record"]["message"],
        "said before anybody was listening"
    );

    tap.push(record("and this one after"));
    let live = next_log(&mut socket).await;
    assert_eq!(live["record"]["message"], "and this one after");
}

/// **The handler's half of the overlap: every record once, across the join.**
///
/// `tail_and_subscribe` hands the stream a tail and a subscription that overlap on purpose, and
/// `pump_logs` is what drops the overlap by sequence number. This drives a real socket to prove the
/// two halves fit — that a reader is not shown a line twice, and that nothing falls between the
/// records the tail carried and the ones the channel did.
///
/// **The race itself is pinned in `km-logtap`**, by a test with two real threads and a barrier
/// between them. It cannot be pinned from here: a spawned task on the single-threaded runtime a
/// `#[tokio::test]` gets by default never runs at the same time as the test body, so this passes
/// whichever way round the tap takes its snapshot.
#[tokio::test]
async fn a_record_taken_while_a_reader_arrives_is_delivered_exactly_once() {
    let tap = km_logtap::LogTap::new();
    let server = Server::start_tapped(
        ApiConfig::default().without_mdns().with_dev_console(),
        Extras::default(),
        Some(tap.clone()),
    )
    .await;

    // Pushed from another task for as long as it takes a socket to open, so that some of these land
    // in the tail, some on the channel, and some in the gap between them.
    let writer = tokio::spawn({
        let tap = tap.clone();
        async move {
            for index in 0..50 {
                tap.push(record(&format!("line {index}")));
                tokio::task::yield_now().await;
            }
        }
    });

    let mut socket = server.logs().await;
    writer.await.expect("the writer finishes");
    // One more after the writer has stopped, so there is a known last line to read up to.
    tap.push(record("the end"));

    let mut seen = Vec::new();
    loop {
        let frame = next_log(&mut socket).await;
        assert_eq!(frame["event"], "record", "nothing lagged in this test");
        let message = frame["record"]["message"].as_str().expect("a message");
        let done = message == "the end";
        seen.push((
            frame["record"]["seq"].as_u64().expect("a seq"),
            message.to_owned(),
        ));
        if done {
            break;
        }
    }

    let mut seqs: Vec<u64> = seen.iter().map(|(seq, _)| *seq).collect();
    let before = seqs.len();
    seqs.dedup();
    assert_eq!(before, seqs.len(), "a record was delivered twice: {seen:?}");
    assert_eq!(
        seen.len(),
        51,
        "a record went missing between the tail and the stream: {seen:?}"
    );
}

/// A record with an invented target and message, so nothing here names a real machine.
fn record(message: &str) -> km_logtap::Record {
    km_logtap::Record {
        seq: 0,
        at_ms: 0,
        level: tracing::Level::INFO,
        target: "km_api",
        message: message.to_owned(),
        fields: String::new(),
    }
}

/// The next frame of the log stream, or a panic if it does not arrive.
async fn next_log(socket: &mut EventSocket) -> Value {
    let deadline = Duration::from_secs(5);
    tokio::time::timeout(deadline, async {
        loop {
            let message = socket.next().await.expect("the stream stays open");
            let Ok(Message::Text(text)) = message else {
                continue;
            };
            return serde_json::from_str(&text).expect("a frame is JSON");
        }
    })
    .await
    .expect("a frame arrives")
}
