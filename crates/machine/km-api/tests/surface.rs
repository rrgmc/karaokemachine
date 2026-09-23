//! The HTTP surface, driven end to end.
//!
//! These go through the real router — real extractors, real middleware, real serialization — against
//! the in-memory machine from `km_api::testing`. What they are checking is the layer the unit tests
//! cannot: that a route is actually mounted at the path the plan says, that a handler reads the thing
//! it was sent, and that the ACL is applied to every route rather than most of them.

use std::net::SocketAddr;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use km_api::machine::{
    AudioOutput, SoundFontBank, SoundFontBanks, SoundFontChoice, SoundFontStatus, SoundKind,
};
use km_api::routes::{API_PREFIX, LOG_SURFACE, POWER_SURFACE, SURFACE, router};
use km_api::testing::{Faults, Recorded, TestMachine, TestPower};
use km_api::{ApiConfig, ApiState, PowerError};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

/// A machine, its router, and the peer address requests appear to come from.
struct Harness {
    machine: Arc<TestMachine>,
    state: ApiState,
    peer: SocketAddr,
    token: Option<String>,
    /// The host's power control, when this harness has one.
    ///
    /// `None` for every harness above, which is the point: a machine that cannot power itself off
    /// is the ordinary case, and the routes are then not mounted at all.
    power: Option<Arc<TestPower>>,
}

impl Harness {
    /// The machine most of this file drives: a catalog, a password, and a token in hand.
    ///
    /// **It has a password because every machine does.** A `new()` that did not would be testing a
    /// state the product cannot be in, and would refuse every admin route for a reason that has
    /// nothing to do with what most of these tests are about.
    fn new() -> Self {
        Self::passworded()
    }

    fn with_config(config: ApiConfig) -> Self {
        let machine = TestMachine::with_catalog(6).shared();
        let state = ApiState::from_machine(machine.clone(), config);
        // **A token from the first request, whenever the machine has a password.** Every real
        // machine does, so a harness that started tokenless would make three quarters of this file
        // about logging in rather than about what it is testing. `issue` mints one without going
        // through the password, which is what it is there for.
        //
        // A test that wants to see a refusal calls `tokenless()`, and the ones that care about the
        // login exchange itself drive `/admin/login` directly.
        let token = state.auth().issue().map(|grant| grant.token);
        Self {
            machine,
            state,
            // Off-machine by default: the interesting authorization cases are the ones a phone hits.
            peer: "192.168.1.50:51000".parse().expect("addr"),
            token,
            power: None,
        }
    }

    /// The default machine for this file: a catalog, and a password, as a real one has.
    fn passworded() -> Self {
        Self::with_config(
            ApiConfig::default()
                .without_mdns()
                .with_password(HARNESS_PASSWORD)
                .expect("argon2 hashes a password"),
        )
    }

    /// The same machine, with the caller holding no token.
    fn tokenless(mut self) -> Self {
        self.token = None;
        self
    }

    /// A machine in debugging mode, which is what mounts `/debug/play-file` and `/debug/play-upload`.
    ///
    /// **Those two are not admin routes**, so this is not about the token: with the mode off they
    /// are not mounted at all and answer 404. Every test about auditioning a file needs this.
    fn debugging() -> Self {
        let mut config = ApiConfig::default()
            .without_mdns()
            .with_password(HARNESS_PASSWORD)
            .expect("argon2 hashes a password");
        config.debug_enabled = true;
        Self::with_config(config)
    }

    fn empty() -> Self {
        let machine = TestMachine::new().shared();
        let state = ApiState::from_machine(
            machine.clone(),
            ApiConfig::default()
                .without_mdns()
                .with_password(HARNESS_PASSWORD)
                .expect("argon2 hashes a password"),
        );
        let token = state.auth().issue().map(|grant| grant.token);
        Self {
            machine,
            state,
            peer: "192.168.1.50:51000".parse().expect("addr"),
            token,
            power: None,
        }
    }

    /// A machine that keeps its own recent log, which is what mounts the two `/admin/logs` routes.
    ///
    /// **Not the default**, for [`Harness::powered`]'s reason: every other test in this file should
    /// be running against a router where these paths do not exist, so that the absent case is what
    /// is ordinarily exercised.
    ///
    /// A small ring and three records, so a test can watch one fall off the front without pushing
    /// five hundred at it. `push` is what seeds it — a public door on the real type rather than a
    /// double, which is what the concrete-type decision buys.
    fn logging() -> Self {
        let tap = km_logtap::LogTap::with_capacity(2).with_filter("info,km_app=debug");
        for message in ["the oldest", "the middle one", "the newest"] {
            tap.push(km_logtap::Record {
                seq: 0,
                at_ms: 0,
                level: tracing::Level::INFO,
                target: "km_api",
                message: message.to_owned(),
                fields: String::new(),
            });
        }
        let harness = Self::new();
        assert!(
            harness.state.set_log_tap(tap),
            "a harness installs its log tap once"
        );
        harness
    }

    /// A machine whose host can power itself off, which is what mounts the three power routes.
    ///
    /// **Not the default**, unlike the password: a machine that can switch its own box off is the
    /// supervised appliance and nothing else, so every other test in this file should be running
    /// against a router where those paths do not exist.
    fn powered() -> Self {
        Self::new().with_power(TestPower::new())
    }

    /// The same, where the operating system says no.
    fn powered_but_refused(refusal: &str) -> Self {
        Self::new().with_power(TestPower::refusing(PowerError::Refused(refusal.to_owned())))
    }

    fn with_power(mut self, power: TestPower) -> Self {
        let power = Arc::new(power);
        assert!(
            self.state.set_power(power.clone()),
            "a harness installs its power control once"
        );
        self.power = Some(power);
        self
    }

    /// What the host was asked to do, for a harness that has one.
    fn power_recorded(&self) -> Vec<Recorded> {
        self.power
            .as_ref()
            .expect("this harness has no power control")
            .recorded()
    }

    fn at_the_machine(mut self) -> Self {
        self.peer = "127.0.0.1:51000".parse().expect("addr");
        self
    }

    /// A machine with a password already set, which is every real machine.
    fn with_password(password: &str) -> Self {
        Self::with_config(
            ApiConfig::default()
                .without_mdns()
                .with_password(password)
                .expect("argon2 hashes a password"),
        )
    }

    /// A machine on a factory password, as a fresh install is.
    fn on_a_factory_password(password: &str) -> Self {
        let mut config = ApiConfig::default()
            .without_mdns()
            .with_password(password)
            .expect("argon2 hashes a password");
        config.factory_password = true;
        Self::with_config(config)
    }

    /// Exchanges the password for a token and keeps it, for every later request.
    async fn log_in(&mut self, password: &str) {
        let (status, body) = self
            .request(
                Method::POST,
                "/admin/login",
                Some(json!({ "password": password })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "logging in failed: {body}");
        self.token = Some(
            body["token"]
                .as_str()
                .expect("a token comes back")
                .to_owned(),
        );
    }

    fn router(&self) -> Router {
        router(self.state.clone())
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{API_PREFIX}{path}"));
        if let Some(token) = &self.token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        self.send(request).await
    }

    /// Sends a prepared request, attaching the peer address the way a real server would.
    async fn send(&self, mut request: Request<Body>) -> (StatusCode, Value) {
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(self.peer));
        let response = self
            .router()
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read the body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.request(Method::GET, path, None).await
    }

    /// Sends bytes that are not JSON, with the content type that says they are.
    ///
    /// `request` above takes a `Value`, so it can only ever send well-formed JSON — which is exactly
    /// the case a test of malformed bodies cannot use.
    async fn raw_body(&self, method: Method, path: &str, body: &str) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{API_PREFIX}{path}"))
            .header("content-type", "application/json");
        if let Some(token) = &self.token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        self.send(builder.body(Body::from(body.to_owned())).expect("request"))
            .await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.request(Method::POST, path, Some(body)).await
    }

    /// A multipart POST, built by hand.
    ///
    /// `parts` is `(field name, optional file name, bytes)`; a `None` file name is an ordinary text
    /// field. Written out rather than reached for through a crate because this is the only multipart
    /// request in the project and a body is eight lines of it — and because a test that assembles
    /// the bytes itself is a test of what the route really parses.
    async fn post_multipart(
        &self,
        path: &str,
        parts: &[(&str, Option<&str>, &[u8])],
    ) -> (StatusCode, Value) {
        const BOUNDARY: &str = "----kmtestboundary";
        let mut body: Vec<u8> = Vec::new();
        for (name, file_name, bytes) in parts {
            body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
            match file_name {
                Some(file_name) => body.extend_from_slice(
                    format!(
                        "content-disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("content-disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());

        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(format!("{API_PREFIX}{path}"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            );
        if let Some(token) = &self.token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        self.send(builder.body(Body::from(body)).expect("request"))
            .await
    }

    async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.request(Method::PUT, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> (StatusCode, Value) {
        self.request(Method::DELETE, path, None).await
    }

    /// A GET whose answer is not JSON, with its headers.
    ///
    /// The export route answers NDJSON and says the catalog version in a header, and neither
    /// survives [`send`](Self::send)'s parse-or-stringify. Both are the point of that route, so they
    /// need a way in.
    ///
    /// **Text only** — see [`raw_bytes`](Self::raw_bytes) for anything binary.
    async fn raw(&self, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let (status, headers, bytes) = self.raw_bytes(path).await;
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    /// The same, without putting the body through UTF-8.
    ///
    /// The song book is a PDF: its second line is four bytes above `0x7F` by design, and its
    /// content streams carry octal escapes. `from_utf8_lossy` replaces the first and would make any
    /// assertion about the file's shape meaningless.
    async fn raw_bytes(&self, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(format!("{API_PREFIX}{path}"))
            .body(Body::empty())
            .expect("build the request");
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(self.peer));
        let response = self
            .router()
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("read the body");
        (status, headers, bytes.to_vec())
    }

    /// Reads the value a successful call returned, failing loudly otherwise.
    async fn ok(&self, method: Method, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.request(method, path, body).await;
        assert!(status.is_success(), "{path} returned {status}: {value}");
        value
    }
}

fn method_of(name: &str) -> Method {
    match name {
        "GET" => Method::GET,
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        "DELETE" => Method::DELETE,
        other => panic!("unexpected method {other}"),
    }
}

/// A body good enough for each mutating route, so a surface sweep is not defeated by a 422.
///
/// **Keyed by path rather than by route id**, because there are no route ids any more. The paths are
/// the sample ones `SURFACE` carries, so this table and that one move together or the sweep starts
/// sending empty bodies and reading 422 as "mounted".
fn sample_body(path: &str) -> Option<Value> {
    Some(match path {
        "/queue" => json!({ "number": "1001" }),
        "/queue/0/move" => json!({ "to_index": 0 }),
        "/transport/seek" => json!({ "ms": 0 }),
        "/settings" => json!({ "transpose": 0 }),
        "/mics/mic1" => json!({ "muted": false }),
        "/admin/audio/output" => json!({ "id": "system" }),
        "/admin/audio/level" => json!({ "db": -6.0 }),
        "/admin/audio/soundfont" => json!({ "id": "generaluser" }),
        "/admin/audio/soundfont/fetch" => json!({ "id": "generaluser" }),
        "/admin/packages" => json!({ "path": "vol2.kmpkg" }),
        "/admin/packages/vol1/bank" => json!({ "bank": 3 }),
        "/admin/machine/name" => json!({ "name": "Living Room" }),
        "/admin/machine/locale" => json!({ "locale": "en" }),
        "/admin/demo" => json!({ "enabled": false }),
        "/admin/debug" => json!({ "enabled": false }),
        "/admin/login" => json!({ "password": "wrong" }),
        // **The password the sweep already holds, not `null` and not a new one.** `null` now *resets*
        // the password to a fresh PIN rather than clearing it, which would invalidate the token this
        // sweep is carrying and close every remaining admin route to the next request. Setting the
        // same password again is a no-op that still exercises the route -- except that it re-hashes,
        // so the token dies anyway; the sweep logs in again after this row for exactly that reason.
        "/admin/password" => json!({ "password": SWEEP_PASSWORD }),
        "/debug/play-file" => json!({ "path": "fixtures/sample.kar" }),
        _ => return None,
    })
}

/// The password the surface sweep sets up and logs in with.
const SWEEP_PASSWORD: &str = "sweep1975";

/// The password `Harness::new` sets up, so an admin route is reachable at all.
const HARNESS_PASSWORD: &str = "harness1975";

// -- the surface exists --------------------------------------------------------------------------

/// Every route answers a refusal in this API's own shape, whatever is wrong with the request.
///
/// **The drift guard for eighty-one raw extractors against four wrapped ones.** `Body`, `Params` and
/// the `Code` extractor exist so a malformed request is refused as an `ErrorDto` with a stable
/// `error` code — and they were used four times, so `PUT /settings` with a bad body answered
/// `application/json` while `DELETE /queue/nonsense` answered axum's plain text: no code to match
/// on, and not even JSON to parse. Which a client got depended on which extractor the handler had
/// reached for.
///
/// Two shapes of garbage, because they take different paths through the extractors: a body that is
/// not JSON at all, and a path segment that is not the type the route declared.
#[tokio::test]
async fn every_route_refuses_garbage_in_this_apis_own_shape() {
    let mut harness = Harness::with_password(SWEEP_PASSWORD).at_the_machine();
    harness.log_in(SWEEP_PASSWORD).await;

    // A path segment of the wrong type, one per shape of segment the surface has. The method has to
    // be one the route accepts, or a 405 with an empty body proves nothing.
    for (method, path) in [
        (Method::DELETE, "/queue/not-a-number"),
        (Method::GET, "/songs/not-a-code/lyrics"),
    ] {
        let (status, body) = harness.request(method.clone(), path, None).await;
        assert!(
            status.is_client_error(),
            "{method} {path} should be refused, and answered {status}"
        );
        assert!(
            body["error"].is_string() && body["message"].is_string(),
            "{method} {path} has to answer an ErrorDto and answered {body}"
        );
    }

    // ...and a body that is not JSON, on every route that takes one.
    for (method, path) in SURFACE {
        let method = method_of(method);
        // Only the methods that read a body. `SURFACE` lists `GET /queue` beside `POST /queue`, and
        // a GET quite correctly ignores whatever was sent with it.
        if !matches!(method, Method::POST | Method::PUT | Method::PATCH) {
            continue;
        }
        if sample_body(path).is_none() || *path == "/events" {
            continue;
        }
        let (status, body) = harness
            .raw_body(method.clone(), path, "not json at all")
            .await;
        assert!(
            status.is_client_error(),
            "{method} {path} should refuse a body that is not JSON, and answered {status}"
        );
        assert_eq!(
            body["error"], "bad_request",
            "{method} {path} has to refuse it as this API's own bad_request: {body}"
        );
    }
}

#[tokio::test]
async fn every_documented_route_is_actually_mounted() {
    // The check the unit test in `routes` cannot make: that the paths in `SURFACE` resolve. A route
    // renamed in the router but not in the table would answer 404 or 405 here.
    //
    // **Driven with a token, deliberately.** Every admin path answers
    // 401 without one, and a 401 is indistinguishable from a mounted route for this test's purpose —
    // so a sweep with no token would stop proving anything about half the surface.
    let mut harness = Harness::with_password(SWEEP_PASSWORD).at_the_machine();
    harness.log_in(SWEEP_PASSWORD).await;

    for (method, path) in SURFACE {
        if *path == "/events" {
            // A WebSocket route rejects a plain GET, correctly. It is covered by its own test.
            continue;
        }
        let (status, body) = harness
            .request(method_of(method), path, sample_body(path))
            .await;
        assert_ne!(
            body["error"],
            km_api::ApiError::UNKNOWN_ENDPOINT,
            "{method} {path} is not mounted: {body}"
        );
        assert_ne!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} does not accept {method}: {body}"
        );
        assert_ne!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{method} {path} rejected the sample body: {body}"
        );
        // Login is the exception: its sample body is a deliberately wrong password, so a 401 there
        // is the route working rather than the route refusing the sweep.
        if *path != "/admin/login" {
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} refused the sweep's token: {body}"
            );
        }
        // Two rows invalidate the sweep's own token, and the rest of the walk would then read 401
        // as "not mounted". Setting the password re-hashes it, which changes the HMAC key; resetting
        // the sessions moves the epoch, which is mixed into the same MAC. Logging in again after
        // each is cheaper than ordering the table around them.
        if *path == "/admin/password" || *path == "/admin/sessions/reset" {
            harness.log_in(SWEEP_PASSWORD).await;
        }
    }
}

/// Every path under `/api/v1/admin/` refuses a caller with no token, and every other path does not.
///
/// **The whole permission system, swept.** It replaces about a dozen tests that each named one route
/// id and asserted it was admin or public — a table mirroring `acl.rs`'s own. There is no table now:
/// the prefix decides, so the assertion is over the prefix, and a route added under `/admin/` is
/// covered by this the day it is written.
#[tokio::test]
async fn the_admin_prefix_is_exactly_what_needs_a_token() {
    // Genuinely tokenless: the harness mints one by default, which is what every *other* test in
    // this file wants and exactly what this one must not have.
    let harness = Harness::with_password(SWEEP_PASSWORD)
        .at_the_machine()
        .tokenless();

    for (method, path) in SURFACE {
        if *path == "/events" || *path == "/admin/login" {
            continue;
        }
        let (status, body) = harness
            .request(method_of(method), path, sample_body(path))
            .await;
        if path.starts_with("/admin/") {
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} is under /admin/ and let a tokenless caller through: {body}"
            );
        } else {
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} is not under /admin/ and demanded a token: {body}"
            );
        }
    }
}

/// Logging in is reachable without a token, because it is how a caller gets one.
#[tokio::test]
async fn the_login_route_is_the_one_exception_to_the_prefix() {
    let harness = Harness::with_password(SWEEP_PASSWORD);
    let (status, body) = harness
        .request(
            Method::POST,
            "/admin/login",
            Some(json!({ "password": SWEEP_PASSWORD })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["token"].is_string(), "{body}");
}

#[tokio::test]
async fn an_unversioned_api_path_says_where_the_api_is() {
    let harness = Harness::new();
    let request = Request::builder()
        .uri("/api/songs")
        .body(Body::empty())
        .expect("request");
    let (status, body) = harness.send(request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], km_api::ApiError::UNKNOWN_ENDPOINT);
    assert!(body["message"].as_str().expect("text").contains("/api/v1"));
}

#[tokio::test]
async fn the_root_explains_itself_when_no_end_user_remote_is_installed() {
    let harness = Harness::new();
    let request = Request::builder()
        .uri("/")
        .body(Body::empty())
        .expect("request");
    let (status, body) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK);
    // Not JSON — a page. A bare 404 to somebody who typed the address off the screen correctly
    // would look like a broken machine.
    let html = body.as_str().expect("html");
    assert!(html.contains("/api/v1"));
    assert!(html.contains("/dev/"));
}

// -- discovery -----------------------------------------------------------------------------------

#[tokio::test]
async fn discovery_names_the_app_the_api_and_the_catalog_size() {
    let harness = Harness::new();
    let body = harness.ok(Method::GET, "/discover", None).await;
    assert_eq!(body["app"], "karaokemachine");
    assert_eq!(body["api"], "/api/v1");
    assert_eq!(body["v"], 1);
    assert_eq!(body["factory_password"], false);
    assert_eq!(body["song_count"], 6);
}

#[tokio::test]
async fn a_loopback_bound_machine_admits_it_rather_than_offering_a_lan_url() {
    let harness = Harness::new();
    let body = harness.ok(Method::GET, "/connect", None).await;
    assert_eq!(body["reachable"], false);
    assert_eq!(body["problem"]["kind"], "loopback_only");
}

// -- catalog -----------------------------------------------------------------------------------

/// The old spellings are gone, and the two of them fail differently.
///
/// `min_score` and `sort=score` were renamed to say `suitability`, because this project does not
/// score singers and the two readings are one keystroke apart. They were then *also* read for a
/// while, on the reasoning that a query string ends up in bookmarks and shell scripts — true of a
/// released product, and this one has never been released. What the aliases actually bought was a
/// demo script that went on teaching the retired name because it still worked.
///
/// **The asymmetry is the part worth pinning.** `min_score` is an unknown *field*, and
/// `SearchParams` sets no `deny_unknown_fields`, so it is ignored — a filter that silently does not
/// apply. `sort=score` is an unknown *variant*, which serde refuses, so it is a 400. Neither is
/// wrong, but somebody reading only one of them would conclude the wrong thing about the other.
#[tokio::test]
async fn the_old_spellings_of_the_suitability_filter_are_not_read() {
    let harness = Harness::new();

    let filtered = harness
        .ok(Method::GET, "/songs?min_suitability=8", None)
        .await;
    assert_eq!(filtered["songs"].as_array().expect("array").len(), 3);

    let ignored = harness.ok(Method::GET, "/songs?min_score=8", None).await;
    let unfiltered = harness.ok(Method::GET, "/songs", None).await;
    assert_eq!(
        ignored, unfiltered,
        "min_score is an unknown field and is ignored, not honored"
    );
    assert_ne!(
        ignored, filtered,
        "...and ignoring it must not happen to give the filtered answer"
    );

    let (status, _) = harness
        .request(Method::GET, "/songs?sort=score&limit=100", None)
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "sort=score is an unknown variant and is refused"
    );
}

#[tokio::test]
async fn search_applies_every_filter_from_the_query_string() {
    let harness = Harness::new();

    let all = harness.ok(Method::GET, "/songs?limit=100", None).await;
    assert_eq!(all["songs"].as_array().expect("array").len(), 6);
    assert_eq!(all["more"], false);

    let text = harness.ok(Method::GET, "/songs?q=Even", None).await;
    assert_eq!(text["songs"].as_array().expect("array").len(), 3);

    let good = harness
        .ok(Method::GET, "/songs?min_suitability=8", None)
        .await;
    assert_eq!(good["songs"].as_array().expect("array").len(), 3);

    let melodic = harness
        .ok(Method::GET, "/songs?melody_only=true", None)
        .await;
    assert_eq!(melodic["songs"].as_array().expect("array").len(), 3);

    let by_artist = harness
        .ok(Method::GET, "/songs?artist=Odd%20Artist", None)
        .await;
    assert_eq!(by_artist["songs"].as_array().expect("array").len(), 3);

    let by_language = harness.ok(Method::GET, "/songs?language=ja", None).await;
    assert_eq!(by_language["songs"].as_array().expect("array").len(), 3);
    assert!(
        by_language["songs"]
            .as_array()
            .expect("array")
            .iter()
            .all(|song| song["language"] == "ja")
    );

    // Exact, so a partial code is not a prefix search, and an empty parameter is no filter rather
    // than a search for songs with no language.
    let partial = harness.ok(Method::GET, "/songs?language=j", None).await;
    assert!(partial["songs"].as_array().expect("array").is_empty());
    let blank = harness.ok(Method::GET, "/songs?language=", None).await;
    assert_eq!(blank["songs"].as_array().expect("array").len(), 6);
}

/// `?tags=` widens to songs carrying **any** of them, and travels as one comma-joined value.
///
/// The fixture is built so a union and an intersection give different answers: every song is
/// `karaoke`, three are `rock`, two are `brasil`, and exactly one is both. A filter that ANDed would
/// answer 1 to the union question rather than 4.
#[tokio::test]
async fn the_tag_filter_widens_to_the_songs_carrying_any_tag() {
    let harness = Harness::new();
    let count = |body: &serde_json::Value| body["songs"].as_array().expect("array").len();

    let all = harness.ok(Method::GET, "/songs?tags=karaoke", None).await;
    assert_eq!(count(&all), 6);
    let rock = harness.ok(Method::GET, "/songs?tags=rock", None).await;
    assert_eq!(count(&rock), 3);
    let brasil = harness.ok(Method::GET, "/songs?tags=brasil", None).await;
    assert_eq!(count(&brasil), 2);

    let either = harness
        .ok(Method::GET, "/songs?tags=rock,brasil", None)
        .await;
    assert_eq!(count(&either), 4, "OR, not AND");

    // A word nobody has used carries no songs of its own and takes none off the tag beside it.
    let typo = harness
        .ok(Method::GET, "/songs?tags=rock,nobody-typed-this", None)
        .await;
    assert_eq!(count(&typo), 3);

    // Folded on the way in, so what somebody typed reaches the slugs that were stored.
    let shouted = harness.ok(Method::GET, "/songs?tags=ROCK", None).await;
    assert_eq!(count(&shouted), 3);

    // Empty is no filter, the same as `?language=`, and it is the case an `IN` gets wrong.
    let blank = harness.ok(Method::GET, "/songs?tags=", None).await;
    assert_eq!(count(&blank), 6);

    // And the tags ride back on the song, so nothing has to ask a second question.
    assert_eq!(
        either["songs"][0]["tags"],
        serde_json::json!(["brasil", "karaoke", "rock"])
    );
}

#[tokio::test]
async fn a_full_page_says_there_may_be_more_without_counting_the_catalog() {
    let harness = Harness::new();
    let page = harness.ok(Method::GET, "/songs?limit=2", None).await;
    assert_eq!(page["songs"].as_array().expect("array").len(), 2);
    assert_eq!(page["limit"], 2);
    assert_eq!(page["more"], true);

    let second = harness
        .ok(Method::GET, "/songs?limit=2&offset=2", None)
        .await;
    assert_eq!(second["offset"], 2);
    assert_ne!(second["songs"][0]["number"], page["songs"][0]["number"]);
}

#[tokio::test]
async fn an_absurd_page_size_is_capped_and_the_cap_is_reported() {
    let harness = Harness::new();
    let page = harness
        .ok(Method::GET, "/songs?limit=100000000", None)
        .await;
    // Reported rather than echoed, so a client can see it did not get a million rows instead of
    // concluding the catalog is small.
    assert_eq!(page["limit"], km_catalog::search::MAX_LIMIT);
}

#[tokio::test]
async fn a_blank_search_term_is_treated_as_no_term_rather_than_as_a_match_for_nothing() {
    let harness = Harness::new();
    let page = harness.ok(Method::GET, "/songs?q=%20%20", None).await;
    assert_eq!(page["songs"].as_array().expect("array").len(), 6);
}

#[tokio::test]
async fn one_song_by_number() {
    let harness = Harness::new();
    let song = harness.ok(Method::GET, "/songs/1001", None).await;
    assert_eq!(song["number"], "1001");
    assert_eq!(song["title"], "Song 1001");
    assert_eq!(song["melody_available"], true);
    assert_eq!(song["suitability"], 9);

    let (status, body) = harness.get("/songs/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
    assert!(body["message"].as_str().expect("text").contains("999999"));
}

/// Every 404 names what was missing, rather than saying only "not found".
///
/// **The drift guard for a fault that had already been fixed twice in the wrong place.**
/// `CatalogError::NotFound` and `ControlError::NotFound` carried nothing, so `ApiError::from` had no
/// choice but to produce the literal string `"not found"` — and two handlers worked around it by
/// intercepting the variant and re-adding the subject by hand. That left every *other* route on this
/// surface answering a 404 whose whole body said nothing at all.
///
/// Driven through the real router, one route per shape of identifier: a package id, a queue entry
/// number, a microphone name. Each has to appear in the message a client is shown.
#[tokio::test]
async fn a_404_says_what_was_missing() {
    let harness = Harness::new();

    for (method, path, missing) in [
        (
            Method::DELETE,
            "/admin/packages/no-such-package",
            "no-such-package",
        ),
        (Method::DELETE, "/queue/424242", "424242"),
        (Method::PUT, "/mics/no-such-mic", "no-such-mic"),
    ] {
        let (status, body) = harness
            .request(method.clone(), path, Some(serde_json::json!({})))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path} should be a 404");
        assert_eq!(body["error"], "not_found", "{path}");
        let message = body["message"].as_str().expect("a message").to_owned();
        assert!(
            message.contains(missing),
            "the 404 for {path} has to name what was missing, and said {message:?}"
        );
    }
}

#[tokio::test]
async fn a_song_carries_the_first_lines_of_its_words_when_its_package_has_them() {
    let harness = Harness::new();

    let song = harness.ok(Method::GET, "/songs/1001", None).await;
    let preview = song["lyric_preview"].as_array().expect("an array");
    assert_eq!(preview.len(), 2);
    assert_eq!(preview[0], "First line of 1001");

    // A song with none omits the key entirely rather than carrying an empty array — which is what
    // `skip_serializing_if` buys, and it matters at five thousand rows a page in the export.
    let plain = harness.ok(Method::GET, "/songs/1002", None).await;
    assert!(
        plain.get("lyric_preview").is_none(),
        "a song with no words should spend no bytes saying so: {plain}"
    );
}

/// The `serde(default)` on `SongDto::lyric_preview`, stated as a test rather than as a comment.
///
/// `km-remote-core` deserializes the export's NDJSON back into this same struct and refuses a whole
/// page if one line will not parse, and the machine leaves `lyric_preview` out of a song with none —
/// so a row without the key has to parse.
#[test]
fn a_song_row_with_no_preview_key_parses() {
    let older = r#"{"number":"1001","title":"Song 1001","artist":null,"language":null,
        "kind":"midi","duration_ms":180000,"suitability":9,"melody_available":true,
        "default_transpose":0,"package_id":"vol1"}"#;
    let song: km_api::dto::SongDto = serde_json::from_str(older).expect("an older row must parse");
    assert_eq!(song.title, "Song 1001");
    assert!(song.lyric_preview.is_empty());
}

/// A code that is not a code refuses in this API's shape, rather than in axum's.
///
/// The over-range case is the one this test exists for. `10000000` would be an ordinary song number
/// that simply nobody had if the ceiling were one digit higher, and would come back as the tidy 404
/// above; because numbers stop at `MAX_NUMBER` it fails during extraction instead, and without the
/// crate's own extractors that would arrive as a plain-text 422 with no `error` field for a client
/// to match on.
#[tokio::test]
async fn a_code_that_is_not_a_code_is_a_bad_request_in_json() {
    let harness = Harness::new();

    for path in ["/songs/10000000", "/songs/BR5A0", "/songs/10000000/lyrics"] {
        let (status, body) = harness.get(path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(body["error"], "bad_request", "{path}");
        assert!(body["message"].is_string(), "{path}");
    }

    // And in the body of a queue request, which is the other place a code arrives.
    let (status, body) = harness
        .post("/queue", serde_json::json!({ "number": "10000000" }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "bad_request");

    // The boundary itself is still an ordinary code, and so still an ordinary 404.
    let (status, _) = harness
        .get(&format!("/songs/{}", km_songcode::MAX_NUMBER))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn lyrics_come_back_in_milliseconds_with_syllables_inside_their_lines() {
    let harness = Harness::new();
    let lyrics = harness.ok(Method::GET, "/songs/1001/lyrics", None).await;
    assert_eq!(lyrics["number"], "1001");
    let lines = lyrics["lines"].as_array().expect("lines");
    assert!(!lines.is_empty());
    for line in lines {
        let start = line["start_ms"].as_u64().expect("start");
        let end = line["end_ms"].as_u64().expect("end");
        assert!(end >= start);
        // The whole-line text is exactly the syllables joined, so a client that only displays and a
        // client that follows the singing agree about what the words are.
        let joined: String = line["syllables"]
            .as_array()
            .expect("syllables")
            .iter()
            .map(|syllable| syllable["text"].as_str().expect("text"))
            .collect();
        assert_eq!(line["text"].as_str().expect("text"), joined);
    }
}

#[tokio::test]
async fn a_catalog_failure_is_a_500_and_not_an_empty_result() {
    let harness = Harness::new();
    harness.machine.set_faults(Faults {
        search: Some("the index is corrupt".to_owned()),
        ..Default::default()
    });
    let (status, body) = harness.get("/songs").await;
    // Silently answering "no songs" would look to a remote exactly like an empty catalog.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "internal");
}

// -- queue ---------------------------------------------------------------------------------------

#[tokio::test]
async fn queueing_a_number_that_does_not_exist_fails_before_anything_is_queued() {
    let harness = Harness::new();
    let (status, body) = harness.post("/queue", json!({ "number": "999999" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
    // Nothing was queued: punching a wrong number in should be a rejection, not an entry that
    // fails at play time.
    assert!(harness.machine.queue_ids().is_empty());
}

#[tokio::test]
async fn queueing_resolves_the_title_so_a_remote_need_not_re_fetch() {
    let harness = Harness::new();
    let (status, added) = harness
        .post("/queue", json!({ "number": "1002", "singer": " Ana " }))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(added["title"], "Song 1002");
    assert_eq!(added["position"], 0);

    let queue = harness.ok(Method::GET, "/queue", None).await;
    assert_eq!(queue["len"], 1);
    // The singer's name is trimmed rather than stored with the spaces a phone keyboard added.
    assert_eq!(queue["entries"][0]["singer"], "Ana");
    assert_eq!(queue["entries"][0]["id"], added["entry_id"]);
    assert_eq!(queue["capacity"], km_queue::queue::MAX_QUEUED);
}

#[tokio::test]
async fn an_empty_singer_is_stored_as_absent_rather_than_as_an_empty_name() {
    let harness = Harness::new();
    harness
        .post("/queue", json!({ "number": "1001", "singer": "   " }))
        .await;
    let queue = harness.ok(Method::GET, "/queue", None).await;
    assert!(queue["entries"][0]["singer"].is_null());
}

#[tokio::test]
async fn reordering_is_by_entry_id_so_a_stale_position_cannot_move_the_wrong_song() {
    let harness = Harness::new();
    let mut ids = Vec::new();
    for number in ["1001", "1002", "1003"] {
        let (_, added) = harness.post("/queue", json!({ "number": number })).await;
        ids.push(added["entry_id"].as_u64().expect("id"));
    }

    // Move the last to the front.
    let queue = harness
        .ok(
            Method::POST,
            &format!("/queue/{}/move", ids[2]),
            Some(json!({ "to_index": 0 })),
        )
        .await;
    assert_eq!(queue["entries"][0]["id"], ids[2]);
    assert_eq!(queue["entries"][1]["id"], ids[0]);

    // An index past the end clamps instead of failing: a remote acting on a stale view asking for
    // position 9 of 3 means "put it last", not "error".
    let queue = harness
        .ok(
            Method::POST,
            &format!("/queue/{}/move", ids[2]),
            Some(json!({ "to_index": 99 })),
        )
        .await;
    assert_eq!(queue["entries"][2]["id"], ids[2]);
}

#[tokio::test]
async fn removing_a_queue_entry_twice_is_a_404_the_second_time() {
    let harness = Harness::new();
    let (_, added) = harness.post("/queue", json!({ "number": "1001" })).await;
    let id = added["entry_id"].as_u64().expect("id");

    let queue = harness
        .ok(Method::DELETE, &format!("/queue/{id}"), None)
        .await;
    assert_eq!(queue["len"], 0);

    let (status, body) = harness.delete(&format!("/queue/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["message"]
            .as_str()
            .expect("text")
            .contains(&id.to_string())
    );
}

#[tokio::test]
async fn a_full_queue_is_a_409_a_client_can_tell_apart_from_a_missing_song() {
    let harness = Harness::new();
    harness.machine.set_faults(Faults {
        queue_full: true,
        ..Default::default()
    });
    let (status, body) = harness.post("/queue", json!({ "number": "1001" })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "queue_full");
}

#[tokio::test]
async fn clearing_the_queue_empties_it() {
    let harness = Harness::new();
    for number in [1001, 1002] {
        harness.post("/queue", json!({ "number": number })).await;
    }
    let queue = harness.ok(Method::DELETE, "/queue", None).await;
    assert_eq!(queue["len"], 0);
    assert!(queue["entries"].as_array().expect("array").is_empty());
}

// -- transport -----------------------------------------------------------------------------------

#[tokio::test]
async fn playing_a_queued_song_takes_it_off_the_queue_and_reports_it_as_now_playing() {
    let harness = Harness::new();
    let (_, added) = harness
        .post("/queue", json!({ "number": "1001", "singer": "Ana" }))
        .await;

    let state = harness.ok(Method::POST, "/transport/play", None).await;
    assert_eq!(state["transport"], "playing");
    assert_eq!(state["queue_len"], 0);
    assert_eq!(state["now_playing"]["origin"]["kind"], "catalog");
    assert_eq!(state["now_playing"]["origin"]["number"], "1001");
    assert_eq!(
        state["now_playing"]["origin"]["entry_id"],
        added["entry_id"]
    );
    assert_eq!(state["now_playing"]["singer"], "Ana");
    assert_eq!(state["now_playing"]["melody_available"], true);
}

#[tokio::test]
async fn playing_with_nothing_queued_is_a_409_rather_than_a_silent_success() {
    let harness = Harness::empty();
    let (status, body) = harness.post("/transport/play", Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "unavailable");
}

/// Skip into silence is a demo song while the mode is on, and a 409 while it is off.
///
/// **The refusal is the same one whichever way the press goes wrong**, which is what this asserts
/// alongside the success: a client that renders `nothing_playing` for this press needs nothing new,
/// and the demo's own reasons stay with `POST /demo/start`.
#[tokio::test]
async fn a_skip_into_silence_starts_a_demo_only_while_the_mode_is_on() {
    let harness = Harness::new();

    let (status, body) = harness.post("/transport/skip", Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "unavailable");
    assert!(
        !harness.machine.recorded().contains(&Recorded::DemoStarted),
        "nothing may ask for a demo with the mode off"
    );

    harness
        .ok(Method::PUT, "/admin/demo", Some(json!({ "enabled": true })))
        .await;
    harness.machine.clear_recorded();

    let (status, _) = harness.post("/transport/skip", Value::Null).await;
    assert!(
        status.is_success(),
        "a skip into silence is answered rather than refused: {status}"
    );
    assert!(
        harness.machine.recorded().contains(&Recorded::DemoStarted),
        "and it has to reach the machine as a demo press"
    );
}

#[tokio::test]
async fn each_transport_command_reaches_the_machine_as_itself() {
    let harness = Harness::new();
    harness.post("/queue", json!({ "number": "1001" })).await;
    harness.machine.clear_recorded();

    for path in ["play", "pause", "restart", "skip", "stop"] {
        harness
            .post(&format!("/transport/{path}"), Value::Null)
            .await;
    }
    harness.post("/transport/seek", json!({ "ms": 1000 })).await;

    use km_api::machine::TransportCommand as Command;
    let expected = [
        Recorded::Transport(Command::Play),
        Recorded::Transport(Command::Pause),
        Recorded::Transport(Command::Restart),
        Recorded::Transport(Command::Skip),
        Recorded::Transport(Command::Stop),
        Recorded::Transport(Command::Seek { ms: 1000 }),
    ];
    assert_eq!(harness.machine.recorded(), expected);
}

#[tokio::test]
async fn a_seek_lands_where_it_was_asked_to() {
    let harness = Harness::new();
    harness.post("/queue", json!({ "number": "1001" })).await;
    harness.post("/transport/play", Value::Null).await;
    let state = harness
        .ok(
            Method::POST,
            "/transport/seek",
            Some(json!({ "ms": 42_000 })),
        )
        .await;
    assert_eq!(state["position_ms"], 42_000);
}

// -- settings ------------------------------------------------------------------------------------

#[tokio::test]
async fn transposing_changes_what_a_later_read_reports() {
    let harness = Harness::new();
    let settings = harness
        .ok(Method::PUT, "/settings", Some(json!({ "transpose": -3 })))
        .await;
    assert_eq!(settings["transpose"], -3);
    let read_back = harness.ok(Method::GET, "/settings", None).await;
    assert_eq!(read_back["transpose"], -3);
}

#[tokio::test]
async fn a_transpose_beyond_the_engines_range_is_a_400_and_changes_nothing() {
    let harness = Harness::new();
    let (status, body) = harness.put("/settings", json!({ "transpose": 99 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "bad_request");
    let settings = harness.ok(Method::GET, "/settings", None).await;
    assert_eq!(settings["transpose"], 0);
}

#[tokio::test]
async fn a_partial_settings_patch_leaves_the_other_values_alone() {
    let harness = Harness::new();
    harness.put("/settings", json!({ "transpose": 2 })).await;
    let settings = harness
        .ok(
            Method::PUT,
            "/settings",
            Some(json!({ "tempo_ratio": 0.9 })),
        )
        .await;
    assert_eq!(settings["transpose"], 2);
    assert!((settings["tempo_ratio"].as_f64().expect("ratio") - 0.9).abs() < 0.001);
}

#[tokio::test]
async fn a_misspelled_settings_field_is_rejected_rather_than_ignored() {
    let harness = Harness::new();
    let (status, body) = harness.put("/settings", json!({ "transpoze": 2 })).await;
    // A request that succeeded and changed nothing is the worst outcome: the client believes it
    // worked.
    //
    // **A 400 rather than the 422 axum would have chosen**, because this route takes `Body` like
    // every other one now. See `handlers::refused`: an over-range song number is a 400 in a path and
    // has to be a 400 in a body too, and axum cannot tell that apart from a misspelled field. The
    // message still names the field, which is the part a client acts on.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "bad_request");
    assert!(
        body["message"]
            .as_str()
            .expect("text")
            .contains("transpoze"),
        "the refusal has to name the field: {body}"
    );
}

#[tokio::test]
async fn enabling_the_melody_on_a_song_where_detection_abstained_is_a_409() {
    let harness = Harness::new();
    // 1002 is one of the songs melody detection abstained on.
    harness.post("/queue", json!({ "number": "1002" })).await;
    harness.post("/transport/play", Value::Null).await;

    let state = harness.ok(Method::GET, "/state", None).await;
    assert_eq!(state["now_playing"]["melody_available"], false);
    assert!(state["now_playing"]["melody_channel"].is_null());

    let (status, body) = harness
        .put("/settings", json!({ "melody_enabled": true }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "unavailable");
}

// -- the audio output device ---------------------------------------------------------------------

#[tokio::test]
async fn the_output_devices_are_listed_with_the_system_default_among_them() {
    let harness = Harness::new();
    let (status, body) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);

    let outputs = body["outputs"].as_array().expect("an array");
    // ALSA.s own enumeration cannot produce the default device, so the sentinel has to be
    // synthesised. A list without it would leave "follow the system" unsayable.
    assert!(outputs.iter().any(|o| o["id"] == "system"));
    // ...and the flag marks the real device the sentinel resolves to, which is a different fact and
    // the one somebody choosing it needs.
    let default_row = outputs
        .iter()
        .find(|o| o["system_default"] == true)
        .expect("something is the system default");
    assert_ne!(default_row["id"], "system");
    assert!(outputs.iter().any(|o| o["usb"] == true));
    assert_eq!(body["selected"], "alsa:plughw:CARD=Device,DEV=0");
    assert_eq!(body["active_name"], "USB Audio CODEC");
    assert_eq!(body["fell_back"], false);
}

#[tokio::test]
async fn the_alternate_spellings_are_marked_rather_than_hidden() {
    // A backend can name one physical output many ways and describe every one of them identically:
    // on the appliance a single headphone jack arrives about ten times and the list runs past
    // thirty rows. The machine says which row it would put in front of somebody and sends the rest
    // anyway, because any of them is a legal choice and one of them may be what settings name.
    let harness = Harness::new();
    let (status, body) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);
    let outputs = body["outputs"].as_array().expect("an array");

    let alternate = outputs
        .iter()
        .find(|o| o["id"] == "alsa:front:CARD=PCH,DEV=0")
        .expect("the other spelling is still listed");
    assert_eq!(alternate["preferred"], false);

    let offered = outputs
        .iter()
        .find(|o| o["id"] == "alsa:plughw:CARD=PCH,DEV=0")
        .expect("the offered spelling");
    assert_eq!(offered["preferred"], true);
    // The two are the same hardware under two names, which is what makes one of them redundant.
    assert_eq!(alternate["name"], offered["name"]);

    let sentinel = outputs
        .iter()
        .find(|o| o["id"] == "system")
        .expect("the sentinel");
    assert_eq!(sentinel["preferred"], true);
}

#[tokio::test]
async fn a_saved_device_that_is_gone_is_listed_as_absent_rather_than_forgotten() {
    // The appliance's actual failure: the USB interface unplugged, the machine following the system
    // default, and the setting deliberately untouched so it comes back on its own.
    let harness = Harness::new();
    harness.machine.set_audio_outputs(
        vec![
            AudioOutput {
                id: "system".to_owned(),
                name: "Follow the system default".to_owned(),
                system_default: true,
                usb: false,
                available: true,
                preferred: true,
            },
            AudioOutput {
                id: "alsa:plughw:CARD=Device,DEV=0".to_owned(),
                name: "USB Audio CODEC".to_owned(),
                system_default: false,
                usb: true,
                available: false,
                preferred: true,
            },
        ],
        Some("alsa:plughw:CARD=Device,DEV=0".to_owned()),
    );

    let (status, body) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);
    // Still named as the selection...
    assert_eq!(body["selected"], "alsa:plughw:CARD=Device,DEV=0");
    // ...while the sound actually comes out of somewhere else, and the response says both.
    assert_eq!(body["active_id"], "system");
    assert_eq!(body["fell_back"], true);
    let absent = body["outputs"]
        .as_array()
        .expect("an array")
        .iter()
        .find(|o| o["id"] == "alsa:plughw:CARD=Device,DEV=0")
        .expect("the saved device is still listed");
    assert_eq!(absent["available"], false);
    assert_eq!(absent["selected"], true);
}

#[tokio::test]
async fn choosing_an_output_device_reaches_the_machine() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .put(
            "/admin/audio/output",
            json!({ "id": "alsa:plughw:CARD=PCH,DEV=0" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["selected"], "alsa:plughw:CARD=PCH,DEV=0");
    assert_eq!(body["active_name"], "HDA Intel PCH");
    assert_eq!(
        harness.machine.recorded(),
        vec![Recorded::SetAudioOutput(
            "alsa:plughw:CARD=PCH,DEV=0".to_owned()
        )]
    );
}

#[tokio::test]
async fn following_the_system_default_is_asked_for_by_id_like_anything_else() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .put("/admin/audio/output", json!({ "id": "system" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    // Stored rather than cleared: "follow the system" is a choice, and storing it is what stops
    // the USB preference overriding it on the next start.
    assert_eq!(body["selected"], "system");
    assert_eq!(body["fell_back"], false);
}

#[tokio::test]
async fn an_unknown_output_device_is_not_found() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .put(
            "/admin/audio/output",
            json!({ "id": "alsa:plughw:CARD=Nope,DEV=0" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn the_output_device_cannot_be_changed_while_a_song_is_loaded() {
    // The player lives inside the audio stream a change has to drop, so this is a refusal and not a
    // delay. 409 rather than 400: the request is perfectly well formed.
    let harness = Harness::new().at_the_machine();
    harness.post("/queue", json!({ "number": "1001" })).await;
    harness.post("/transport/play", json!({})).await;

    let (status, body) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["changeable"], false,
        "a remote should be able to gray the control out rather than collect a 409"
    );

    let (status, body) = harness
        .put(
            "/admin/audio/output",
            json!({ "id": "alsa:plughw:CARD=PCH,DEV=0" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "unavailable");
    // ...and it really did not change.
    let (_, body) = harness.get("/audio/outputs").await;
    assert_eq!(body["selected"], "alsa:plughw:CARD=Device,DEV=0");
}

#[tokio::test]
async fn a_queued_song_also_blocks_a_change() {
    // An idle machine with a queued song is a machine about to start one: the poll loop loads the
    // next entry the moment the transport goes idle. Transport alone is not the whole test.
    let harness = Harness::new().at_the_machine();
    harness.post("/queue", json!({ "number": "1001" })).await;
    let (status, body) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["changeable"], false);
}

#[tokio::test]
async fn choosing_an_output_device_is_open_while_no_password_is_set() {
    // The shipped ACL's only admin route outside `acl.write` and `admin.*`. A guest may see where
    // the sound is going, and whether they may move it depends entirely on whether a password has
    // been set -- with none, the `admin` mark is dormant. See the `A machine with no password has
    // no door` decision in docs/decisions/.
    let harness = Harness::new();
    let (status, _) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);

    // No password configured, so the mark is dormant and this behaves as a public route.
    let (status, _) = harness
        .put("/admin/audio/output", json!({ "id": "system" }))
        .await;
    assert_ne!(status, StatusCode::FORBIDDEN);
}

// -- the output's own level ----------------------------------------------------------------------

/// A control like the appliance's USB interface: 1 dB steps from −128 dB to unity, sitting 20 dB
/// down, which is the state this whole surface exists because of.
fn attenuated() -> km_api::machine::OutputLevel {
    km_api::machine::OutputLevel {
        db_centi: -2000,
        db_min_centi: -12800,
        db_max_centi: 0,
        step_centi: 100,
    }
}

#[tokio::test]
async fn an_output_with_no_level_reports_none_rather_than_zero() {
    let harness = Harness::new();
    let (status, body) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);
    // Absent, not `0` — a card at unity and a card with no control are different things, and a
    // reader that saw `0.0` could not tell them apart.
    assert!(body["level"].is_null());
}

#[tokio::test]
async fn the_level_is_reported_in_decibels() {
    let harness = Harness::new();
    harness.machine.set_output_level_range(attenuated());
    let (status, body) = harness.get("/audio/outputs").await;
    assert_eq!(status, StatusCode::OK);
    // Decibels on the wire, so the number beside the slider is the number `amixer` prints.
    assert_eq!(body["level"]["db"], -20.0);
    assert_eq!(body["level"]["db_min"], -128.0);
    assert_eq!(body["level"]["db_max"], 0.0);
    assert_eq!(body["level"]["step_db"], 1.0);
}

#[tokio::test]
async fn moving_the_level_reaches_the_machine() {
    let harness = Harness::new();
    harness.machine.set_output_level_range(attenuated());
    let (status, body) = harness
        .put("/admin/audio/level", json!({ "db": 0.0 }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["level"]["db"], 0.0);
    // Hundredths crossing the seam, which is what keeps the domain type comparable.
    assert_eq!(
        harness.machine.recorded(),
        vec![Recorded::SetOutputLevel(0)]
    );
}

#[tokio::test]
async fn a_level_past_either_end_is_clamped_rather_than_refused() {
    let harness = Harness::new();
    harness.machine.set_output_level_range(attenuated());

    // A client drawing a slider from an earlier reading can be a step out of date without being
    // wrong, so the answer is the nearest legal level and not a 400.
    let (status, body) = harness
        .put("/admin/audio/level", json!({ "db": 12.0 }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["level"]["db"], 0.0);

    let (status, body) = harness
        .put("/admin/audio/level", json!({ "db": -400.0 }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["level"]["db"], -128.0);
}

#[tokio::test]
async fn an_output_with_no_level_refuses_the_change() {
    let harness = Harness::new();
    // No level arranged, which is a machine playing through HDMI: the receiver holds the volume.
    let (status, body) = harness
        .put("/admin/audio/level", json!({ "db": 0.0 }))
        .await;
    // A 409 and not a 400: the request was well formed and the machine declined it.
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "no_output_level");
}

#[tokio::test]
async fn the_level_is_not_refused_while_a_song_is_playing() {
    let harness = Harness::new().at_the_machine();
    harness.machine.set_output_level_range(attenuated());
    harness.post("/queue", json!({ "number": "1001" })).await;
    harness.post("/transport/play", json!({})).await;

    // The device cannot be changed now, which is the contrast this test exists for.
    let (_, body) = harness.get("/audio/outputs").await;
    assert_eq!(body["changeable"], false);

    // ...and the level can. It belongs to the sound card rather than to the audio stream, so there
    // is nothing for a song to be in the way of.
    let (status, _) = harness
        .put("/admin/audio/level", json!({ "db": -10.0 }))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_bank_that_is_playing_is_named_along_with_what_chose_it() {
    let harness = Harness::new();
    let (status, body) = harness.get("/audio/soundfont").await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(body["playing"], "soundfont");
    // The full path, not a file name: a bundled bank and an override can share a name, and the
    // whole reason to ask is to tell which one is loaded.
    assert!(
        body["path"]
            .as_str()
            .expect("a path")
            .ends_with("GeneralUser-GS.sf2")
    );
    assert_eq!(body["chosen_by"], "bundled");
    assert!(body["problem"].is_null());
}

#[tokio::test]
async fn a_machine_on_a_test_tone_says_so_and_says_why() {
    // The state this endpoint exists for. Everything else on the surface reports a machine that is
    // working: a device is open, songs play, the lyrics scroll in time -- and every instrument is a
    // sine wave. Reporting the absence without the reason would leave somebody with nowhere to go.
    let harness = Harness::new();
    harness.machine.set_soundfont(SoundFontStatus {
        path: None,
        chosen_by: None,
        playing: SoundKind::TestTone,
        problem: Some("D:/banks/Broken.sf2 is not a file".to_owned()),
        fallback: None,
    });

    let (status, body) = harness.get("/audio/soundfont").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["playing"], "test_tone");
    assert!(body["path"].is_null());
    // Nothing chose a bank, so neither answer to "what chose it" is true. Saying `bundled` here
    // would read as a bundled bank playing.
    assert!(body["chosen_by"].is_null());
    assert!(
        body["problem"]
            .as_str()
            .expect("a reason")
            .contains("Broken.sf2")
    );
}

#[tokio::test]
async fn a_configured_bank_is_reported_as_configured() {
    let harness = Harness::new();
    harness.machine.set_soundfont(SoundFontStatus {
        path: Some("D:/tunes/karaoke/MuseScore_General.sf2".to_owned()),
        chosen_by: Some(SoundFontChoice::Setting),
        playing: SoundKind::SoundFont,
        problem: None,
        fallback: None,
    });

    let body = harness.ok(Method::GET, "/audio/soundfont", None).await;
    assert_eq!(body["chosen_by"], "setting");
    assert_eq!(body["path"], "D:/tunes/karaoke/MuseScore_General.sf2");
    assert!(
        body["fallback"].is_null(),
        "the setting resolved, so there is no caveat"
    );
}

/// A stale bank id is a working machine with a caveat, and is reported as neither of the other two.
///
/// The state the id change made reachable and the older shape could not describe: `audio.soundfont`
/// names a bank the folder no longer holds, so the bundled one is playing. `playing` is `soundfont`
/// because it is; `problem` is null because nothing is wrong with the sound; and `chosen_by` is
/// `fallback` rather than `bundled`, because a machine on the bundled bank *despite* a choice is not
/// the same machine as one on it because nobody chose.
#[tokio::test]
async fn a_stale_bank_setting_is_reported_as_a_fallback_rather_than_a_problem() {
    let harness = Harness::new();
    harness.machine.set_soundfont(SoundFontStatus {
        path: Some("/opt/karaokemachine/assets/soundfont/GeneralUser-GS.sf2".to_owned()),
        chosen_by: Some(SoundFontChoice::Fallback),
        playing: SoundKind::SoundFont,
        problem: None,
        fallback: Some(
            "the chosen SoundFont \"musescore\" is not in the SoundFont folder any more, so the              bundled bank is playing"
                .to_owned(),
        ),
    });

    let body = harness.ok(Method::GET, "/audio/soundfont", None).await;
    assert_eq!(body["playing"], "soundfont", "the machine is working");
    assert_eq!(body["chosen_by"], "fallback");
    assert!(
        body["problem"].is_null(),
        "a working machine must not be reported as broken"
    );
    assert!(
        body["fallback"]
            .as_str()
            .expect("a caveat")
            .contains("musescore")
    );
}

#[tokio::test]
async fn which_bank_is_playing_is_readable_without_a_password() {
    // It shares `audio.read` with the device list, which ships public: knowing why the instruments
    // sound wrong is not a privilege, and only `audio.write` -- moving the sound elsewhere -- is.
    let harness = Harness::new();
    let (status, _) = harness.get("/audio/soundfont").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_bank_list_offers_the_bundled_one_even_when_nothing_has_been_added() {
    // The ordinary machine, and the case a picker is most likely to be written without: one row,
    // already selected, and nothing to choose. It must still be a list rather than an error.
    let harness = Harness::new();
    let body = harness.ok(Method::GET, "/audio/soundfonts", None).await;

    assert_eq!(body["selected"], "bundled");
    let banks = body["banks"].as_array().expect("a list of banks");
    assert_eq!(banks.len(), 1);
    assert_eq!(banks[0]["id"], "bundled");
    assert_eq!(banks[0]["selected"], true);
    // The row a remote must not offer a delete control on.
    assert_eq!(banks[0]["bundled"], true);
}

/// Two banks to choose between, which the default machine does not have.
fn two_banks() -> SoundFontBanks {
    SoundFontBanks {
        banks: vec![
            SoundFontBank {
                id: "bundled".to_owned(),
                name: "Bundled".to_owned(),
                bytes: 32_319_396,
                bundled: true,
                why_not_removable: Some(
                    "the bundled SoundFont ships with the machine and cannot be removed".to_owned(),
                ),
            },
            SoundFontBank {
                id: "roland-sc-55-v3-7".to_owned(),
                name: "Roland SC-55 v3.7".to_owned(),
                bytes: 108_424_522,
                bundled: false,
                why_not_removable: None,
            },
        ],
        selected: "bundled".to_owned(),
        // This double answers about what is installed; what could be fetched is a different test.
        offers: Vec::new(),
        fetching: None,
    }
}

#[tokio::test]
async fn choosing_a_bank_moves_the_selection() {
    let harness = Harness::new();
    harness.machine.set_soundfonts(two_banks());

    let body = harness
        .ok(
            Method::PUT,
            "/admin/audio/soundfont",
            Some(json!({ "id": "roland-sc-55-v3-7" })),
        )
        .await;

    // The response is the whole new list, so a remote redraws from one answer rather than choosing
    // and then asking what it chose.
    assert_eq!(body["selected"], "roland-sc-55-v3-7");
    let banks = body["banks"].as_array().expect("a list of banks");
    assert_eq!(banks[0]["selected"], false);
    assert_eq!(banks[1]["selected"], true);
}

#[tokio::test]
async fn choosing_a_bank_is_open_while_no_password_is_set() {
    // It shares `audio.write` with the output device, which is admin-marked because both are
    // installation configuration rather than performance knobs. With no password the mark is
    // dormant and this behaves as a public route -- the `A machine with no password has no door`
    // decision, and the same shape as `choosing_an_output_device_is_open_while_no_password_is_set`.
    let harness = Harness::new();
    harness.machine.set_soundfonts(two_banks());
    let (status, _) = harness
        .put(
            "/admin/audio/soundfont",
            json!({ "id": "roland-sc-55-v3-7" }),
        )
        .await;
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_ne!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_bank_that_is_not_in_the_list_is_refused_rather_than_guessed_at() {
    // Also the traversal case, and the reason ids are looked up in the list rather than turned back
    // into a filename: an id shaped like a path matches no row, so it never reaches the filesystem.
    let harness = Harness::new();
    harness.machine.set_soundfonts(two_banks());
    for id in ["nonesuch", "../../etc/passwd", ""] {
        let (status, body) = harness
            .put("/admin/audio/soundfont", json!({ "id": id }))
            .await;
        assert!(
            !status.is_success(),
            "id {id:?} should be refused, not accepted: {body}"
        );
    }
    // And the selection did not move on the way past.
    let body = harness.ok(Method::GET, "/audio/soundfonts", None).await;
    assert_eq!(body["selected"], "bundled");
}

#[tokio::test]
async fn deleting_a_bank_removes_it_from_the_list() {
    let harness = Harness::new();
    harness.machine.set_soundfonts(two_banks());

    let body = harness
        .ok(
            Method::DELETE,
            "/admin/audio/soundfonts/roland-sc-55-v3-7",
            None,
        )
        .await;

    // The whole new list comes back, as it does from choosing one, so a page redraws from one
    // answer rather than deleting and then asking what is left.
    let banks = body["banks"].as_array().expect("a list of banks");
    assert_eq!(banks.len(), 1);
    assert_eq!(banks[0]["id"], "bundled");
}

#[tokio::test]
async fn deleting_the_selected_bank_falls_back_to_the_bundled_one() {
    // The case the route exists for. On a television the selected bank may *be* the mistake being
    // undone, so this must not be a refusal -- and what it must not leave behind is a selection
    // naming a bank that is gone.
    let harness = Harness::new();
    harness.machine.set_soundfonts(two_banks());
    harness
        .ok(
            Method::PUT,
            "/admin/audio/soundfont",
            Some(json!({ "id": "roland-sc-55-v3-7" })),
        )
        .await;

    let body = harness
        .ok(
            Method::DELETE,
            "/admin/audio/soundfonts/roland-sc-55-v3-7",
            None,
        )
        .await;

    assert_eq!(body["selected"], "bundled");
    let banks = body["banks"].as_array().expect("a list of banks");
    assert_eq!(banks.len(), 1);
    assert_eq!(banks[0]["selected"], true);
}

#[tokio::test]
async fn the_bundled_bank_cannot_be_deleted() {
    // It is an unpacked asset: removing the file would succeed and it would be back on the next
    // launch, so the refusal is the honest answer rather than a protective one.
    let harness = Harness::new();
    harness.machine.set_soundfonts(two_banks());

    let (status, body) = harness.delete("/admin/audio/soundfonts/bundled").await;
    assert!(
        !status.is_success(),
        "the bundled bank should be refused: {body}"
    );

    let body = harness.ok(Method::GET, "/audio/soundfonts", None).await;
    assert_eq!(body["banks"].as_array().expect("a list of banks").len(), 2);
}

#[tokio::test]
async fn deleting_a_bank_that_is_not_in_the_list_is_refused_rather_than_guessed_at() {
    // The traversal case again, and for the same reason as choosing: an id shaped like a path
    // matches no row, so it never reaches the filesystem. Worth its own test here because this is
    // the route that ends in `remove_file`.
    let harness = Harness::new();
    harness.machine.set_soundfonts(two_banks());
    for id in ["nonesuch", "..%2F..%2Fetc%2Fpasswd", "GeneralUser-GS.sf2"] {
        let (status, body) = harness
            .delete(&format!("/admin/audio/soundfonts/{id}"))
            .await;
        assert!(
            !status.is_success(),
            "id {id:?} should be refused, not accepted: {body}"
        );
    }
    let body = harness.ok(Method::GET, "/audio/soundfonts", None).await;
    assert_eq!(body["banks"].as_array().expect("a list of banks").len(), 2);
}

#[tokio::test]
async fn fetching_a_bank_the_machine_does_not_offer_is_refused() {
    // The default `TestMachine` offers nothing and downloads nothing, which is also every host that
    // is not the machine itself. Asking must be a refusal rather than a silent success.
    let harness = Harness::new();
    let (status, body) = harness
        .post("/admin/audio/soundfont/fetch", json!({ "id": "musescore" }))
        .await;
    assert!(
        !status.is_success(),
        "a machine with nothing to fetch refuses: {body}"
    );
}

/// One row of the survey that is not on the shortlist — what `?all=true` is for.
fn an_unranked_offer() -> km_api::machine::SoundFontOffer {
    km_api::machine::SoundFontOffer {
        id: "timgm6mb".to_owned(),
        name: "TimGM6mb.sf2".to_owned(),
        size: "5.9 MiB".to_owned(),
        bytes: 6_143_868,
        license: "GPL, with the samples' own terms under it".to_owned(),
        note: "the small one".to_owned(),
        fetchable: true,
        page: None,
        recommended: false,
        offered: false,
    }
}

#[tokio::test]
async fn the_bank_list_is_the_shortlist_until_the_whole_catalog_is_asked_for() {
    // The two widths of one route. A phone gets the nine the machine offers; a page for working on
    // the machine asks for the survey and gets it, with every row saying which kind it is.
    let harness = Harness::new();
    harness.machine.set_soundfonts(SoundFontBanks {
        offers: vec![km_api::machine::SoundFontOffer {
            id: "musescore".to_owned(),
            name: "MuseScore_General.sf2".to_owned(),
            size: "205.6 MiB".to_owned(),
            bytes: 215_614_036,
            license: "MIT, attribution required".to_owned(),
            note: "the best measured".to_owned(),
            fetchable: true,
            page: None,
            recommended: true,
            offered: true,
        }],
        ..two_banks()
    });
    harness
        .machine
        .set_unranked_soundfont_offers(vec![an_unranked_offer()]);

    for path in ["/audio/soundfonts", "/audio/soundfonts?all=false"] {
        let body = harness.ok(Method::GET, path, None).await;
        let offers = body["offers"].as_array().expect("a list of offers");
        assert_eq!(offers.len(), 1, "{path} widened the list: {body}");
        assert_eq!(offers[0]["id"], "musescore");
        assert_eq!(offers[0]["offered"], true);
    }

    let body = harness
        .ok(Method::GET, "/audio/soundfonts?all=true", None)
        .await;
    let offers = body["offers"].as_array().expect("a list of offers");
    assert_eq!(offers.len(), 2, "the catalog was not widened: {body}");
    // A widening, not a different list: the shortlist is still there and still first.
    assert_eq!(offers[0]["id"], "musescore");
    assert_eq!(offers[1]["id"], "timgm6mb");
    assert_eq!(offers[1]["offered"], false);
    // And the installed banks are the same answer at either width.
    assert_eq!(body["banks"].as_array().expect("a list").len(), 2);
}

#[tokio::test]
async fn the_whole_catalog_needs_no_password() {
    // It is a width, not a permission: `audio.read` ships public, and `POST .../fetch` was never
    // gated by rank either, so refusing to *name* a bank while agreeing to fetch it would be a line
    // drawn where there is no difference in what somebody may do.
    let harness = Harness::new();
    let (status, _) = harness.get("/audio/soundfonts?all=true").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_query_parameter_this_version_has_not_heard_of_is_ignored() {
    // The rule `SearchParams` and `ExportParams` already keep: a client built against a later
    // version passes something extra and still gets its answer.
    let harness = Harness::new();
    let (status, body) = harness.get("/audio/soundfonts?all=true&nonesuch=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_bank_list_carries_offers_and_what_a_download_is_doing() {
    // Both are absent on a plain machine, and absent has to be a shape a client can read rather
    // than a missing key.
    let harness = Harness::new();
    let body = harness.ok(Method::GET, "/audio/soundfonts", None).await;
    assert!(body["offers"].is_array(), "offers is a list: {body}");
    assert_eq!(body["offers"].as_array().expect("a list").len(), 0);
    assert!(
        body["fetching"].is_null(),
        "nothing is being fetched: {body}"
    );
}

// -- microphones, wallpapers, packages -----------------------------------------------------------

#[tokio::test]
async fn mics_report_that_they_apply_no_processing() {
    let harness = Harness::new();
    let mics = harness.ok(Method::GET, "/mics", None).await;
    assert_eq!(mics["applies_dsp"], false);
    assert_eq!(mics["mics"].as_array().expect("array").len(), 2);
}

#[tokio::test]
async fn a_mic_patch_touches_only_what_it_names_and_clamps_what_it_does() {
    let harness = Harness::new();
    let mics = harness
        .ok(
            Method::PUT,
            "/mics/mic1",
            Some(json!({ "gain": 99.0, "muted": true })),
        )
        .await;
    let first = &mics["mics"][0];
    assert_eq!(first["gain"], km_queue::mics::MAX_GAIN);
    assert_eq!(first["muted"], true);
    // Effects untouched, and the second mic untouched.
    assert_eq!(first["reverb"], 0.0);
    assert_eq!(mics["mics"][1]["muted"], false);
}

#[tokio::test]
async fn a_blank_device_hint_takes_the_hint_away_and_a_missing_one_keeps_it() {
    let harness = Harness::new();
    let named = harness
        .ok(
            Method::PUT,
            "/mics/mic1",
            Some(json!({ "device_hint": "USB Audio" })),
        )
        .await;
    assert_eq!(named["mics"][0]["device_hint"], "USB Audio");

    // A patch that says nothing about the hint keeps it, which is why a blank one has to mean
    // something: there would otherwise be no way to say the mic is no longer on that input.
    let muted = harness
        .ok(Method::PUT, "/mics/mic1", Some(json!({ "muted": true })))
        .await;
    assert_eq!(muted["mics"][0]["device_hint"], "USB Audio");

    let cleared = harness
        .ok(
            Method::PUT,
            "/mics/mic1",
            Some(json!({ "device_hint": "" })),
        )
        .await;
    assert!(cleared["mics"][0]["device_hint"].is_null());
}

#[tokio::test]
async fn patching_a_mic_that_does_not_exist_is_a_404() {
    let harness = Harness::new();
    let (status, body) = harness.put("/mics/mic9", json!({ "muted": true })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["message"].as_str().expect("text").contains("mic9"));
}

#[tokio::test]
async fn advancing_the_wallpaper_reaches_the_machine() {
    let harness = Harness::new();
    let before = harness.ok(Method::GET, "/wallpapers", None).await;
    assert_eq!(before["count"], 3);
    let after = harness.ok(Method::POST, "/wallpapers/next", None).await;
    assert_ne!(after["current"], before["current"]);
    assert!(
        harness
            .machine
            .recorded()
            .contains(&Recorded::NextWallpaper)
    );
}

/// The folder lists one row per **file**, so a zip is one row saying how many pictures it holds.
///
/// That is the package rule read across: you remove the package, never a song inside it. A listing
/// that flattened archives into their entries would offer a Remove control that cannot exist,
/// because taking one picture out of a zip means rewriting an archive the machine did not make.
#[tokio::test]
async fn the_wallpaper_folder_lists_files_and_a_zip_is_one_of_them() {
    let harness = Harness::new();
    let state = harness.ok(Method::GET, "/wallpapers", None).await;
    let pictures = state["pictures"].as_array().expect("pictures");
    assert_eq!(pictures.len(), 3);

    let zip = pictures
        .iter()
        .find(|p| p["name"] == "beach.zip")
        .expect("the archive is one row");
    assert_eq!(zip["images"], 2, "a zip says how many pictures it holds");
    assert_eq!(
        zip["removable"], true,
        "the whole file is the removable unit"
    );

    // The sentence stays on the machine: it names a path, and a path is the operator's filesystem
    // layout. Only the boolean crosses.
    let refused = pictures
        .iter()
        .find(|p| p["name"] == "pinned.jpg")
        .expect("the refused row is still listed");
    assert_eq!(refused["removable"], false);
    assert!(
        !state.to_string().contains("debug.wallpapers"),
        "the refusal's wording must not go on the wire: {state}"
    );
}

/// Removing a picture takes it off the list, off the count, and off the screen.
///
/// The last is the one worth asserting: `The wallpaper folder is chosen again, not once` asks that
/// such a control never leave the removed picture showing, because a picture that stays on the
/// television after being removed reads as a broken button.
#[tokio::test]
async fn removing_a_wallpaper_takes_it_off_the_list_and_off_the_screen() {
    let harness = Harness::new();
    let before = harness.ok(Method::GET, "/wallpapers", None).await;
    assert_eq!(before["count"], 3);
    assert_eq!(before["current"], "sunset.jpg");

    let after = harness
        .ok(Method::DELETE, "/admin/wallpapers/beach-zip", None)
        .await;
    assert_eq!(
        after["pictures"].as_array().expect("pictures").len(),
        2,
        "the row is gone from the answer, not only from a later read"
    );
    assert_eq!(
        after["count"], 1,
        "both of the zip's pictures left the cycle"
    );
    assert_ne!(
        after["current"], before["current"],
        "the cycle was asked to move on"
    );
    assert!(
        harness
            .machine
            .recorded()
            .contains(&Recorded::DeleteWallpaper("beach-zip".to_owned())),
        "the request has to reach the machine, not merely return 200"
    );
}

/// A picture the machine will not delete is refused, and the refusal names the file.
#[tokio::test]
async fn a_wallpaper_that_is_not_the_machines_to_delete_is_refused_by_name() {
    let harness = Harness::new();
    let (status, body) = harness
        .request(Method::DELETE, "/admin/wallpapers/pinned-jpg", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("pinned.jpg"),
        "the refusal has to name the file: {body}"
    );

    let state = harness.ok(Method::GET, "/wallpapers", None).await;
    assert_eq!(state["pictures"].as_array().expect("pictures").len(), 3);
}

/// An unknown id is refused rather than reaching the filesystem.
#[tokio::test]
async fn an_unknown_wallpaper_id_is_refused() {
    let harness = Harness::new();
    let (status, _) = harness
        .request(
            Method::DELETE,
            "/admin/wallpapers/..%2F..%2Fetc%2Fpasswd",
            None,
        )
        .await;
    assert_ne!(status, StatusCode::OK);
}

/// A picture called `next.jpg` is deletable, and the *id shape* is what makes it so.
///
/// `/wallpapers/next` is a static segment sitting exactly where `{id}` goes. **Axum matches the path
/// before the method**, so that literal wins for every verb and answers 405 to a `DELETE` — one
/// never falls through to the parameter. This route was written the other way round, on the
/// assumption that the method separated them, and this test is what caught it: precisely the
/// "unlikely, and silent" trap the SoundFont routes' own comment warns about.
///
/// What actually keeps them apart is that no picture id can *be* `next`: an id is slugged from the
/// whole file name including its extension, and `Playlist::scan` only takes files that have an image
/// or archive one. So `next.jpg` is `next-jpg`. The 405 below is therefore asserted rather than
/// worked around — it is the right answer to a path no picture can claim.
#[tokio::test]
async fn a_picture_called_next_is_not_shadowed_by_the_next_route() {
    let harness = Harness::new();
    harness.machine.set_pictures(vec![km_api::machine::Picture {
        id: "next-jpg".to_owned(),
        name: "next.jpg".to_owned(),
        images: 1,
        bytes: 1000,
        why_not_removable: None,
    }]);

    // The file is reachable under the id it really has...
    harness
        .ok(Method::DELETE, "/admin/wallpapers/next-jpg", None)
        .await;
    assert!(
        harness
            .machine
            .recorded()
            .contains(&Recorded::DeleteWallpaper("next-jpg".to_owned()))
    );

    // ...and `POST /wallpapers/next` still advances the picture, from the *public* prefix.
    //
    // **Why the pair is asserted together.** axum matches the path before the method, so `next` and
    // `{id}` in one router would make `/wallpapers/next` reach the static route for every verb — a
    // 405 for the delete, and a picture whose id slugs to `next` undeletable. Advancing is public
    // and deleting is admin, so the two sit in different prefixes and cannot shadow each other at
    // all. The id carries its extension, which is the other half of what keeps them apart.
    harness.ok(Method::POST, "/wallpapers/next", None).await;
    let (status, _) = harness
        .request(Method::DELETE, "/admin/wallpapers/next", None)
        .await;
    assert_ne!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "nothing static sits beside {{id}} under /admin/wallpapers/ any more"
    );
}

#[tokio::test]
async fn an_empty_wallpaper_folder_is_reported_as_a_problem_not_as_zero_images() {
    let harness = Harness::new();
    harness
        .machine
        .set_wallpapers(km_api::machine::WallpaperState {
            current: None,
            count: 0,
            interval_secs: 30,
            shuffle: false,
            on_song_change: true,
            problem: Some("the wallpaper folder is empty".to_owned()),
            source: km_api::machine::WallpaperSource::Owner,
        });
    let state = harness.ok(Method::GET, "/wallpapers", None).await;
    assert_eq!(state["count"], 0);
    assert_eq!(state["problem"], "the wallpaper folder is empty");
    // Which rule chose the folder, which is the half a count of zero cannot answer: an owner's own
    // folder with nothing in it is a different situation from the shipped set being what is showing.
    assert_eq!(state["source"], "owner");
}

/// The rule that chose the folder reaches the wire, and the folder's path does not.
///
/// `bundled` beside a count is the whole answer to *why isn't my picture on the screen* — the
/// owner's folder held nothing when it was last looked at. The path stays off: that is the
/// operator's filesystem layout, which `current` already refuses to carry for the same reason.
#[tokio::test]
async fn the_wallpaper_folders_rule_is_reported_and_its_path_is_not() {
    let harness = Harness::new();
    harness
        .machine
        .set_wallpapers(km_api::machine::WallpaperState {
            current: Some("01-dusk.png".to_owned()),
            count: 4,
            interval_secs: 30,
            shuffle: true,
            on_song_change: true,
            problem: None,
            source: km_api::machine::WallpaperSource::Bundled,
        });
    let state = harness.ok(Method::GET, "/wallpapers", None).await;
    assert_eq!(state["source"], "bundled");
    assert_eq!(state["count"], 4);
    assert_eq!(
        state["current"], "01-dusk.png",
        "a bare file name, never a path"
    );
    let text = state.to_string();
    assert!(
        !text.contains('/') && !text.contains('\\'),
        "no filesystem layout on the wire: {text}"
    );
}

#[tokio::test]
async fn installing_a_package_reports_duplicates_rather_than_refusing() {
    let harness = Harness::new();
    let report = harness
        .ok(
            Method::POST,
            "/admin/packages",
            Some(json!({ "path": "D:/packages/vol2.kmpkg" })),
        )
        .await;
    assert_eq!(report["package_id"], "vol2");
    // Beside the id, because a client wording this for a person quotes the name — the builder does,
    // and so does a machine handed the file by a double-click.
    assert_eq!(report["package_name"], "vol2");
    assert_eq!(report["songs_added"], 2);
    assert_eq!(report["replaced_existing"], false);
    // Two packages legitimately containing the same recording is a catalog smell the owner should
    // see, not an error that blocks the install.
    assert_eq!(
        report["duplicate_content"].as_array().expect("array").len(),
        1
    );
    assert_eq!(report["duplicate_content"][0]["existing_number"], "1001");
}

#[tokio::test]
async fn a_package_listing_does_not_leak_the_operators_filesystem() {
    let harness = Harness::new();
    let listing = harness.ok(Method::GET, "/packages", None).await;
    assert_eq!(listing["song_count"], 6);
    let text = listing.to_string();
    assert!(
        !text.contains("D:/packages"),
        "the archive path leaked: {text}"
    );

    // The same rule for a package that would *not* install, which is the harder case: the file has
    // to be identifiable so somebody can go and fix it, and the directory it sits in still must not
    // reach a phone. Found by running it — the first version of this route sent the whole path.
    harness
        .machine
        .set_package_problems(vec![km_api::machine::PackageProblem {
            path: "D:\\packages\\vol2.kmpkg".to_owned(),
            package_id: None,
            reason: "could not open it".to_owned(),
        }]);
    let listing = harness.ok(Method::GET, "/packages", None).await;
    assert_eq!(listing["problems"][0]["file"], "vol2.kmpkg");
    let text = listing.to_string();
    assert!(
        !text.contains("packages\\\\vol2") && !text.contains("D:"),
        "the refused package's path leaked: {text}"
    );
}

#[tokio::test]
async fn a_package_listing_names_what_would_not_install_and_why() {
    let harness = Harness::new();

    // Nothing wrong: the key is absent rather than present and empty, so a healthy machine's
    // response carries no `problems` at all.
    let healthy = harness.ok(Method::GET, "/packages", None).await;
    assert!(healthy.get("problems").is_none(), "{healthy}");

    // **One sentence per fault.** Two packages cannot claim one number — each is in a thousand of
    // its own — so what is left to say is that a bank is taken or that a file will not open, and
    // each is one remedy.
    harness
        .machine
        .set_package_problems(vec![km_api::machine::PackageProblem {
            path: "D:/packages/vol2.kmpkg".to_owned(),
            package_id: Some("brasil-vol2".to_owned()),
            reason: "bank 3 already belongs to the package vol1".to_owned(),
        }]);

    let listing = harness.ok(Method::GET, "/packages", None).await;
    let problem = &listing["problems"][0];
    assert_eq!(problem["package_id"], "brasil-vol2");
    assert_eq!(
        problem["reason"],
        "bank 3 already belongs to the package vol1"
    );
}

#[tokio::test]
async fn a_bank_outside_the_range_is_refused() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .put(
            "/admin/packages/vol1/bank",
            json!({ "bank": km_songcode::MAX_BANK + 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Bank 0 is the machine's own, and this is the one route that could have given it away.
    let (status, body) = harness
        .put("/admin/packages/vol1/bank", json!({ "bank": 0 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("bank 0"),
        "it should say which bank: {body}"
    );
}

#[tokio::test]
async fn a_bank_change_is_refused_while_anything_is_queued() {
    let harness = Harness::new().at_the_machine();

    // With nothing playing and nothing waiting, it goes through and says what it re-keyed. The
    // fixtures are slots 1..6 in bank 1, so bank 5 moves them to 5001 upwards.
    let done = harness
        .ok(
            Method::PUT,
            "/admin/packages/vol1/bank",
            Some(json!({ "bank": 5 })),
        )
        .await;
    assert_eq!(done["bank"], 5);
    assert_eq!(done["songs_renumbered"], 6);
    // And the songs really answer to the new numbers.
    let (status, _) = harness.get("/songs/5001").await;
    assert!(status.is_success());
    let (status, _) = harness.get("/songs/1001").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the old number is gone");

    // Queue something, and the same call is refused: every number in the package would move under a
    // queue that names the old ones.
    harness.post("/queue", json!({ "number": "5001" })).await;
    let (status, body) = harness
        .put("/admin/packages/vol1/bank", json!({ "bank": 7 }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"], "unavailable",
        "a 409 a client can tell apart"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("queued"),
        "the refusal should say why: {body}"
    );
}

#[tokio::test]
async fn moving_a_package_needs_the_admin_password_where_queueing_does_not() {
    // Re-keying a whole package is reconfiguration, not a performance knob. This is the shipped
    // default and `PUT /acl` can still open it.
    let mut harness = Harness::with_password("hunter2").tokenless();
    let (status, _) = harness
        .put("/admin/packages/vol1/bank", json!({ "bank": 3 }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // ...and queueing, next to it, is still open to anybody in the room.
    let (status, _) = harness.post("/queue", json!({ "number": "1001" })).await;
    assert!(status.is_success(), "queueing must stay public");

    let login = harness
        .ok(
            Method::POST,
            "/admin/login",
            Some(json!({ "password": "hunter2" })),
        )
        .await;
    harness.token = login["token"].as_str().map(str::to_owned);
    let (status, _) = harness
        .put("/admin/packages/vol1/bank", json!({ "bank": 3 }))
        .await;
    assert_ne!(status, StatusCode::UNAUTHORIZED, "the password opens it");
}

#[tokio::test]
async fn turning_demo_mode_on_needs_the_admin_password_where_reading_it_does_not() {
    // Making the machine play music by itself in somebody's house is an owner's act. Knowing that
    // it is doing so is not: a remote has to read this to tell a singer that skipping — not waiting
    // — is what lets them sing, and it says nothing anybody in the room cannot already hear.
    let mut harness = Harness::with_password("hunter2").tokenless();
    let (status, _) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = harness.get("/demo").await;
    assert!(status.is_success(), "reading demo mode must stay public");
    assert_eq!(body["enabled"], false, "and the refusal changed nothing");

    let login = harness
        .ok(
            Method::POST,
            "/admin/login",
            Some(json!({ "password": "hunter2" })),
        )
        .await;
    harness.token = login["token"].as_str().map(str::to_owned);
    let (status, body) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert!(status.is_success(), "the password opens it");
    assert_eq!(body["enabled"], true);
}

/// `persist` is the only thing separating an evening from a permanent change, so it has to travel.
#[tokio::test]
async fn a_demo_switch_lasts_the_run_unless_it_is_asked_to_persist() {
    let harness = Harness::new();

    // The plain body is the safer of the two: it changes the running machine and nothing else.
    let body = harness
        .ok(Method::PUT, "/admin/demo", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(body["enabled"], true);
    assert_eq!(
        body["stored"], false,
        "a switch nobody asked to keep must not reach the settings file"
    );
    assert_eq!(
        harness.machine.recorded(),
        vec![Recorded::SetDemo {
            enabled: true,
            persist: false
        }]
    );

    // And saying so writes it down.
    let body = harness
        .ok(
            Method::PUT,
            "/admin/demo",
            Some(json!({ "enabled": true, "persist": true })),
        )
        .await;
    assert_eq!(body["stored"], true);

    // The answer is read back out of the machine rather than echoed, so it carries the rest of the
    // state a client would otherwise need a second request for.
    assert!(body["delay_secs"].is_number());
    assert_eq!(body["playing"], false, "nothing is loaded");
}

/// The delay has a route of its own, it is admin, and it never asks about `persist`.
///
/// **The shape is the assertion.** `PUT /admin/demo` is *tonight or for good* because a party is a
/// run; a delay is installation configuration and is always written down, so it is a path under the
/// switch rather than a third field in its body — a body where one field obeyed `persist` and its
/// neighbour ignored it would make the flag mean two things. And it is admin for the switch's reason
/// with one of its own on top: the smallest legal delay is zero, so a stranger who could set it
/// could arrange for a house they are not in to sing the moment it goes quiet.
#[tokio::test]
async fn the_demo_delay_is_its_own_admin_route_and_is_always_written_down() {
    let mut harness = Harness::with_password("hunter2").tokenless();
    let (status, _) = harness
        .put("/admin/demo/delay", json!({ "delay_secs": 30 }))
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "zero is a legal delay, so this is the switch by another door"
    );

    let login = harness
        .ok(
            Method::POST,
            "/admin/login",
            Some(json!({ "password": "hunter2" })),
        )
        .await;
    harness.token = login["token"].as_str().map(str::to_owned);

    let body = harness
        .ok(
            Method::PUT,
            "/admin/demo/delay",
            Some(json!({ "delay_secs": 30 })),
        )
        .await;
    assert_eq!(body["delay_secs"], 30);
    assert_eq!(
        harness.machine.recorded().last(),
        Some(&Recorded::SetDemoDelay { delay_secs: 30 }),
        "the number has to reach the machine, not merely parse"
    );

    // `persist` is not a field here, and `deny_unknown_fields` says so rather than ignoring it --
    // a client that sent one would otherwise believe it had asked for something.
    let (status, _) = harness
        .put(
            "/admin/demo/delay",
            json!({ "delay_secs": 30, "persist": true }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{status}");

    // Past the cap is a 400: it says *fix what you sent*, which is aimed at whatever built the
    // request rather than at somebody holding a phone.
    let (status, _) = harness
        .put(
            "/admin/demo/delay",
            json!({ "delay_secs": km_api::machine::MAX_DEMO_DELAY_SECS + 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{status}");
    let body = harness.ok(Method::GET, "/demo", None).await;
    assert_eq!(body["delay_secs"], 30, "a refusal changes nothing");
}

/// Starting one demo song is anybody's, where turning the mode on is an owner's.
///
/// **The difference is one song against a mode**, and the pair to compare it with is one screen over:
/// `wallpapers.next` ships public and `wallpapers.upload` ships admin for the same reason. A trigger
/// that first needed the admin password would be useless to the room it exists for — the point of it
/// is that somebody standing in a silent house can make the box show what it holds.
#[tokio::test]
async fn starting_a_demo_song_needs_no_password_where_turning_the_mode_on_does() {
    let harness = Harness::with_password("hunter2").tokenless();

    let (status, _) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the mode is an owner's");

    let (status, body) = harness.post("/demo/start", json!(null)).await;
    assert!(status.is_success(), "one song is not: {body}");
    assert_eq!(
        harness.machine.recorded(),
        vec![Recorded::DemoStarted],
        "and the press reached the machine"
    );

    // And it turned nothing on. Whether the machine will keep going by itself afterwards is what a
    // caller reads this body for, and the answer here is no.
    assert_eq!(body["enabled"], false);
}

/// A trigger may not interrupt anybody, which is what makes it safe to hand to a guest.
///
/// Both refusals are 409 `unavailable` carrying the machine's own sentence, because a remote puts
/// that sentence on screen rather than inventing one.
#[tokio::test]
async fn a_demo_cannot_be_started_over_a_song_or_over_somebody_waiting_in_the_queue() {
    let harness = Harness::new();

    harness
        .ok(Method::POST, "/queue", Some(json!({ "number": "1001" })))
        .await;
    let (status, body) = harness.post("/demo/start", json!(null)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "unavailable");
    assert!(
        body["message"]
            .as_str()
            .expect("a sentence")
            .contains("queue"),
        "the sentence has to be usable as it stands: {body}"
    );

    harness.ok(Method::POST, "/transport/play", None).await;
    let (status, body) = harness.post("/demo/start", json!(null)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["message"]
            .as_str()
            .expect("a sentence")
            .contains("already playing"),
        "{body}"
    );
}

#[tokio::test]
async fn uninstalling_removes_the_package_and_says_how_many_songs_went() {
    let harness = Harness::new();
    let report = harness
        .ok(Method::DELETE, "/admin/packages/vol1", None)
        .await;
    assert_eq!(report["package_id"], "vol1");
    assert_eq!(report["songs_removed"], 6);

    let (status, _) = harness.delete("/admin/packages/vol1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A package the machine will not delete answers **400 with the sentence**, not 404.
///
/// The bug this pins: the mapping was `|_| not_found(…)`, so every failure came back as
/// `404 package '<id>'` — the one status that says "it is not here", which was false, and which
/// threw away the machine's own account of why. A client treating 404 as "already gone, fine" sees
/// the difference, and that is the point.
#[tokio::test]
async fn a_package_the_machine_refuses_to_delete_says_why_rather_than_answering_404() {
    let harness = Harness::new();
    harness.machine.set_faults(Faults {
        uninstall: Some(km_api::machine::CatalogError::Rejected(
            "it is named in debug.packages: take it out with --clear-debug-packages".to_owned(),
        )),
        ..Default::default()
    });

    let (status, body) = harness.delete("/admin/packages/vol1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "bad_request");
    assert!(
        body["message"]
            .as_str()
            .expect("a message")
            .contains("--clear-debug-packages"),
        "the machine's own sentence survives: {body}"
    );
}

/// A file the machine could not remove is a 500, and it says nothing was uninstalled.
///
/// The disk failed, not the request — so it is neither a 404 nor a 400.
#[tokio::test]
async fn an_uninstall_that_could_not_delete_the_file_is_a_500() {
    let harness = Harness::new();
    harness.machine.set_faults(Faults {
        uninstall: Some(km_api::machine::CatalogError::Failed(
            "\"vol1.kmpkg\" could not be removed, so nothing was uninstalled: access denied"
                .to_owned(),
        )),
        ..Default::default()
    });

    let (status, _) = harness.delete("/admin/packages/vol1").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// A package that is not the machine's to delete is listed as such, without saying why.
///
/// **The flag ships and the reason does not.** Every refusal names a full path, and the directory
/// layout of the machine under the television is nobody's business but the owner's — who reads the
/// sentence on their own page, in process.
#[tokio::test]
async fn a_package_that_is_not_the_machines_to_delete_is_listed_as_not_removable() {
    let harness = Harness::new();
    harness.machine.set_faults(Faults {
        unremovable: vec![(
            "vol1".to_owned(),
            "\"D:/private/place/vol1.kmpkg\" is named in debug.packages".to_owned(),
        )],
        ..Default::default()
    });

    let body = harness.ok(Method::GET, "/packages", None).await;
    let package = &body["packages"][0];
    assert_eq!(package["id"], "vol1");
    assert_eq!(package["removable"], false);

    let json = body.to_string();
    assert!(!json.contains("debug.packages"), "the reason stays behind");
    assert!(!json.contains("private"), "and so does the path");
}

#[tokio::test]
async fn playing_a_file_directly_bypasses_the_catalog() {
    let harness = Harness::debugging();
    let state = harness
        .ok(
            Method::POST,
            "/debug/play-file",
            Some(json!({ "path": "fixtures/generated/lyric_events.mid" })),
        )
        .await;
    assert_eq!(state["now_playing"]["origin"]["kind"], "file");
    assert_eq!(state["transport"], "playing");
    assert!(harness.machine.recorded().iter().any(|entry| matches!(
        entry,
        Recorded::PlayFile(path) if path.ends_with("lyric_events.mid")
    )));
}

#[tokio::test]
async fn a_refused_debug_path_is_the_controllers_decision_and_becomes_a_400() {
    let harness = Harness::debugging();
    harness.machine.set_faults(Faults {
        play_file: Some("that path is outside the allowed folders".to_owned()),
        ..Default::default()
    });
    let (status, body) = harness
        .post("/debug/play-file", json!({ "path": "/etc/passwd" }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .expect("text")
            .contains("outside the allowed folders")
    );
}

/// The route the whole feature exists for: a machine that cannot see the curator's disk.
#[tokio::test]
async fn an_uploaded_song_is_staged_and_played_under_the_name_it_was_sent_with() {
    let harness = Harness::debugging();
    let (status, state) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"Sultans of Swing"),
                ("primary", Some("whatever.kar"), b"MThd fake but not empty"),
            ],
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{state}");
    assert_eq!(state["now_playing"]["origin"]["kind"], "file");
    assert_eq!(state["transport"], "playing");
    // Named from the `stem` field and the part's *extension*, never from its filename.
    assert!(
        harness.machine.recorded().iter().any(|entry| matches!(
            entry,
            Recorded::PlayAudition(name) if name == "Sultans of Swing.kar"
        )),
        "{:?}",
        harness.machine.recorded()
    );
}

/// The pair is the reason an upload is staged in a folder rather than streamed at a decoder.
#[tokio::test]
async fn both_halves_of_an_mp3_g_song_are_staged_under_one_stem() {
    let harness = Harness::debugging();
    let (status, _) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"Perfidia"),
                ("primary", Some("PERFIDIA.MP3"), b"ID3 audio"),
                // Deliberately a differently-spelled filename, and deliberately one with a trailing
                // space in the stem: neither may reach the disk, because both halves are named from
                // the `stem` field. A pair named from two filenames can end up unable to find each
                // other, which is the hazard `km_kmpkg::pair_for` documents.
                ("partner", Some("Perfidia .Cdg"), b"CDG graphics"),
            ],
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    let names: Vec<String> = harness
        .machine
        .recorded()
        .iter()
        .filter_map(|entry| match entry {
            Recorded::PlayAudition(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    // One call, naming the half sent first — the machine finds the other beside it.
    assert_eq!(names, vec!["Perfidia.mp3".to_owned()]);
}

/// The words of a small UltraStar song, as the timeline a sender reads out of its `.txt`.
fn ultrastar_words() -> km_song::LyricTimeline {
    km_song::ultrastar::parse(b"#TITLE:Song\n#MP3:Song.mp3\n#BPM:300\n: 0 4 0 Hel\n: 4 2 0 lo\nE\n")
        .expect("an UltraStar song")
        .timeline
}

/// An UltraStar song crosses as its MP3 and its words, and the machine is never sent the `.txt`.
#[tokio::test]
async fn an_uploaded_ultrastar_song_is_its_mp3_with_the_words_beside_it() {
    let harness = Harness::debugging();
    let words = ultrastar_words();
    let lyrics = serde_json::to_vec(&words).expect("encodes");
    let (status, body) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"Ace Of Spades"),
                ("lyrics", None, lyrics.as_slice()),
                ("primary", Some("Ace Of Spades.mp3"), b"ID3 audio"),
            ],
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let recorded = harness.machine.recorded();
    assert!(
        recorded.iter().any(|entry| matches!(
            entry,
            Recorded::PlayAudition(name) if name == "Ace Of Spades.mp3"
        )),
        "{recorded:?}"
    );
    assert!(
        recorded.iter().any(|entry| matches!(
            entry,
            Recorded::Decided(decided) if decided.lyrics.as_ref() == Some(&words)
        )),
        "{recorded:?}"
    );
}

/// An LRC song crosses as an UltraStar song does, with its kind named beside the words.
#[tokio::test]
async fn an_uploaded_lrc_song_names_its_kind_beside_the_words() {
    let harness = Harness::debugging();
    let words = km_song::lrc::parse(b"[00:01.00]First line\n[00:04.00]Second line\n")
        .expect("an LRC song")
        .timeline;
    let lyrics = serde_json::to_vec(&words).expect("encodes");
    let (status, body) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"Ace Of Spades"),
                ("lyrics", None, lyrics.as_slice()),
                ("lyrics_kind", None, b"lrc"),
                ("primary", Some("Ace Of Spades.mp3"), b"ID3 audio"),
            ],
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let recorded = harness.machine.recorded();
    assert!(
        recorded.iter().any(|entry| matches!(
            entry,
            Recorded::Decided(decided)
                if decided.lyrics.as_ref() == Some(&words)
                    && decided.lyrics_kind == Some(km_catalog::SongKind::Lrc)
        )),
        "{recorded:?}"
    );
}

/// A kind that carries no words is refused, rather than words played as a song they do not belong to.
#[tokio::test]
async fn an_uploaded_lyrics_kind_that_carries_no_words_is_a_400() {
    let harness = Harness::debugging();
    let (status, body) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"Ace Of Spades"),
                ("lyrics_kind", None, b"video"),
                ("primary", Some("Ace Of Spades.mp3"), b"ID3 audio"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .expect("text")
            .contains("lyrics_kind"),
        "{body}"
    );
}

/// Words that will not read are refused, rather than an UltraStar song playing with none.
#[tokio::test]
async fn uploaded_words_that_will_not_read_are_a_400() {
    let harness = Harness::debugging();
    let (status, body) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"Ace Of Spades"),
                ("lyrics", None, b"not a timeline"),
                ("primary", Some("Ace Of Spades.mp3"), b"ID3 audio"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().expect("text").contains("lyrics"),
        "{body}"
    );
}

/// The path route carries an UltraStar song's words as the upload route does.
#[tokio::test]
async fn an_ultrastar_songs_words_travel_with_a_path() {
    let harness = Harness::debugging();
    let words = ultrastar_words();
    harness
        .ok(
            Method::POST,
            "/debug/play-file",
            Some(json!({
                "path": "fixtures/generated/lyric_events.mid",
                "lyrics": words,
            })),
        )
        .await;
    assert!(harness.machine.recorded().iter().any(
        |entry| matches!(entry, Recorded::Decided(decided) if decided.lyrics.as_ref() == Some(&words))
    ));
}

#[tokio::test]
async fn an_uploaded_stem_cannot_climb_out_of_the_staging_folder() {
    for stem in [
        "../../authorized_keys",
        "..\\..\\windows\\system32\\evil",
        "/etc/passwd",
        "..",
        "   ",
    ] {
        let harness = Harness::debugging();
        let (status, body) = harness
            .post_multipart(
                "/debug/play-upload",
                &[
                    ("stem", None, stem.as_bytes()),
                    ("primary", Some("x.kar"), b"MThd"),
                ],
            )
            .await;
        // Either the stem is refused outright, or it is reduced to a bare name — never a path.
        if status == StatusCode::OK {
            let played = harness
                .machine
                .recorded()
                .iter()
                .find_map(|entry| match entry {
                    Recorded::PlayAudition(name) => Some(name.clone()),
                    _ => None,
                })
                .expect("something was played");
            assert_eq!(
                std::path::Path::new(&played).file_name(),
                Some(played.as_ref()),
                "{stem:?} produced {played:?}, which is not a bare name"
            );
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{stem:?}: {body}");
        }
    }
}

#[tokio::test]
async fn an_upload_that_is_not_a_kind_of_song_is_refused_before_anything_is_written() {
    let harness = Harness::debugging();
    let (status, body) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"payload"),
                ("primary", Some("payload.exe"), b"MZ"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().expect("text").contains("play"),
        "{body}"
    );
    assert!(
        !harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::PlayAudition(_))),
        "nothing should have been played"
    );
}

#[tokio::test]
async fn the_stem_has_to_arrive_before_the_files_it_names() {
    let harness = Harness::debugging();
    let (status, _) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("primary", Some("x.kar"), b"MThd"),
                ("stem", None, b"too late"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_upload_with_no_file_in_it_is_a_400_rather_than_a_silent_success() {
    let harness = Harness::debugging();
    let (status, _) = harness
        .post_multipart("/debug/play-upload", &[("stem", None, b"lonely")])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A machine that will not take an upload says which switch would change that.
///
/// **The wording is load-bearing**: `km-package-builder` recognises it to explain the refusal rather
/// than showing a bare 400. It names `debug.enabled` now, where it used to name
/// `debug.accept_uploads` — one switch replaced the other.
#[tokio::test]
async fn a_machine_that_takes_no_uploads_says_which_setting_would_change_that() {
    let harness = Harness::debugging();
    harness.machine.set_faults(Faults {
        uploads: Some(
            "this machine is not in debugging mode; turn it on from the owner's page, or set \
             debug.enabled in settings, to audition an uploaded song"
                .to_owned(),
        ),
        ..Default::default()
    });
    let (status, body) = harness
        .post_multipart(
            "/debug/play-upload",
            &[
                ("stem", None, b"anything"),
                ("primary", Some("x.kar"), b"MThd"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"]
            .as_str()
            .expect("text")
            .contains("debug.enabled"),
        "{body}"
    );
}

/// Discovery says whether this machine is in debugging mode, which is what a curator asks first.
///
/// **Asked before a video is sent, so the refusal arrives as a sentence rather than as a dropped
/// connection.** A server that answers mid-request and closes leaves the sending half looking like a
/// connection that dropped, so the most useful refusal is the one least likely to arrive. It was
/// `accepts_uploads`; the field is `debug_enabled` now, and it answers a wider question with the
/// same one bit.
#[tokio::test]
async fn discovery_says_whether_this_machine_is_in_debugging_mode() {
    let on = Harness::debugging();
    let (_, body) = on.get("/discover").await;
    assert_eq!(body["debug_enabled"], true, "{body}");
    assert!(
        body.get("accepts_uploads").is_none(),
        "the old field should be gone: {body}"
    );

    let off = Harness::new();
    let (_, body) = off.get("/discover").await;
    assert_eq!(body["debug_enabled"], false, "{body}");
}

/// The switch, and the round trip that makes it worth having.
///
/// This is the route that exists so a curation tool can open the debug surface on a machine with no
/// keyboard — an Android device or the appliance, where "edit settings.json and restart" means `adb`
/// or nothing. So the thing to prove is not that a field was written but that the *machine's own
/// answer* moved, which is why the assertions read `/discover` rather than the response body.
#[tokio::test]
async fn debugging_can_be_turned_off_and_on_again_over_the_api() {
    let harness = Harness::debugging();
    let (_, before) = harness.get("/discover").await;
    assert_eq!(before["debug_enabled"], true, "{before}");

    let (status, body) = harness
        .put("/admin/debug", json!({ "enabled": false }))
        .await;
    assert_eq!(status, StatusCode::OK);
    // **`stored` moved and `enabled` did not, and that is the point of the pair.** The two debug
    // routes are mounted at router-construction time, so this run stays exactly as debuggable as it
    // was; what changed is what the next start will do. A reply echoing the request into `enabled`
    // would have been telling a caller the surface had closed while it was still open.
    assert_eq!(body["stored"], false, "{body}");
    assert_eq!(body["enabled"], true, "{body}");
    assert!(
        harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::DebugEnabledSet(false))),
        "the switch did not reach the controller"
    );

    let (status, body) = harness
        .put("/admin/debug", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["stored"], true, "{body}");
}

/// A misspelled field is refused rather than silently changing nothing.
///
/// `deny_unknown_fields`, as every other request body on this surface has: a client that sent
/// `{"accept": true}` — the field this route used to take — and got a 200 would have every reason to
/// believe debugging was on. 422 and not 400, matching
/// `a_misspelled_settings_field_is_rejected_rather_than_ignored` above: the body parsed as JSON and
/// was refused for what it said, which is the distinction that code draws.
#[tokio::test]
async fn the_debug_switch_refuses_a_body_it_does_not_understand() {
    let harness = Harness::debugging();
    let (status, _) = harness.put("/admin/debug", json!({ "accept": true })).await;
    // 400 rather than 422 since this route takes `Body`; see `handlers::refused`.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, discovery) = harness.get("/discover").await;
    assert_eq!(discovery["debug_enabled"], true, "{discovery}");
}

/// Renaming the machine moves what the machine says about itself, not just what it stored.
///
/// The assertion reads `/discover` for `uploads_can_be_turned_off_and_on_again_over_the_api`'s
/// reason: `/discover` is what a phone actually sees, and a rename that reached `settings.json` and
/// not the running state would be a rename that visibly did not happen. The recorded call is the
/// other half — the durable one — because `ApiState` holds the name for this run only.
#[tokio::test]
async fn renaming_the_machine_changes_what_discovery_says_and_is_written_down() {
    let harness = Harness::debugging().at_the_machine();
    let (_, before) = harness.get("/discover").await;
    assert_eq!(before["name"], "KaraokeMachine", "{before}");

    let (status, body) = harness
        .put("/admin/machine/name", json!({ "name": "  Living Room  " }))
        .await;
    assert_eq!(status, StatusCode::OK);
    // Answered with what the machine now says, not with what was asked: the name was tidied on the
    // way in, so an echo would tell a client nothing about the rules it just met.
    assert_eq!(body["name"], "Living Room", "{body}");

    let (_, after) = harness.get("/discover").await;
    assert_eq!(after["name"], "Living Room", "{after}");
    assert_eq!(
        harness.machine.recorded(),
        vec![Recorded::MachineRenamed("Living Room".to_owned())]
    );
}

/// A name cut to the DNS label limit is reported back at the length that was kept.
///
/// The reply exists to answer this case. Sixty-three bytes is the DNS-SD instance label limit, so
/// the tidying is not a preference a client could have applied itself, and a client that had to
/// guess would draw a name the network never carried.
#[tokio::test]
async fn a_name_too_long_for_a_dns_label_comes_back_at_the_length_that_was_kept() {
    let harness = Harness::debugging().at_the_machine();
    let long = "a".repeat(200);
    let (status, body) = harness
        .put("/admin/machine/name", json!({ "name": long }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let kept = body["name"].as_str().expect("a name came back");
    assert_eq!(kept.len(), km_api::discover::MAX_NAME_BYTES, "{body}");

    let (_, discovery) = harness.get("/discover").await;
    assert_eq!(discovery["name"], kept, "{discovery}");
}

/// A blank name is refused rather than stored, so no client has to decide what an empty name means.
#[tokio::test]
async fn a_blank_name_is_refused_and_the_machine_keeps_the_one_it_had() {
    let harness = Harness::debugging().at_the_machine();
    let (status, body) = harness
        .put("/admin/machine/name", json!({ "name": "   " }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().expect("text").contains("blank"),
        "{body}"
    );
    // And nothing moved.
    let (_, discovery) = harness.get("/discover").await;
    assert_eq!(discovery["name"], "KaraokeMachine", "{discovery}");
    assert!(harness.machine.recorded().is_empty());
}

/// A misspelled field is refused rather than silently renaming nothing.
///
/// `deny_unknown_fields`, as every other request body on this surface has. 422 and not 400, for
/// `the_upload_switch_refuses_a_body_it_does_not_understand`'s reason: the body parsed as JSON and
/// was refused for what it said.
#[tokio::test]
async fn the_rename_route_refuses_a_body_it_does_not_understand() {
    let harness = Harness::debugging().at_the_machine();
    let (status, _) = harness
        .put("/admin/machine/name", json!({ "machine_name": "Kitchen" }))
        .await;
    // 400 rather than 422 since this route takes `Body`; see `handlers::refused`.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, discovery) = harness.get("/discover").await;
    assert_eq!(discovery["name"], "KaraokeMachine", "{discovery}");
}

// -- what language the television draws in ---------------------------------------------------------

/// The locale goes round: what is set is what is read back, and it is written down.
///
/// **The read is public and the write is not**, which is the split this pair exists to draw — so
/// the assertion is made without a token as well as with one. A tool across the room reads the
/// machine's language to open its picker on the right row, and it holds a password before it can
/// move it.
#[tokio::test]
async fn what_the_television_speaks_can_be_read_by_anybody_and_set_by_an_owner() {
    let harness = Harness::debugging().at_the_machine();
    let (status, before) = harness.get("/locale").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["locale"], "en", "{before}");

    let (status, body) = harness
        .put("/admin/machine/locale", json!({ "locale": "pt-BR" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["locale"], "pt-BR", "{body}");

    let (_, after) = harness.get("/locale").await;
    assert_eq!(after["locale"], "pt-BR", "{after}");
}

/// A tag this build has no catalog for is refused, and the machine goes on drawing what it drew.
///
/// **`best_match` first, so `pt` and `pt-PT` are answers rather than refusals.** A client that
/// asked for either gets `pt-BR` back, so the reply is the tag the machine settled on rather than
/// an echo. What is left over is a language this build does not have at all, and a 200 there would
/// report a change nobody could see.
#[tokio::test]
async fn a_language_this_build_does_not_have_is_refused_and_a_near_one_is_resolved() {
    let harness = Harness::debugging().at_the_machine();

    let (status, body) = harness
        .put("/admin/machine/locale", json!({ "locale": "pt-PT" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["locale"], "pt-BR", "{body}");

    let (status, body) = harness
        .put("/admin/machine/locale", json!({ "locale": "de" }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().expect("text").contains("de"),
        "{body}"
    );

    // And nothing moved.
    let (_, after) = harness.get("/locale").await;
    assert_eq!(after["locale"], "pt-BR", "{after}");
}

// -- what an owner sends from a browser ------------------------------------------------------------

/// A package sent from a browser reaches the machine, bytes and all.
///
/// **The assertion is on what the controller was handed, not on the status code**, because the half
/// of an upload route that fails silently is the streaming: a handler that answered 200 having
/// written nothing would pass every check but this one. `TestMachine` reads the staged file's size
/// before recording, so the byte count in its answer is proof the body arrived.
#[tokio::test]
async fn a_package_uploaded_from_a_browser_reaches_the_machine() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .post_multipart(
            "/admin/packages/upload",
            &[("file", Some("carols.kmpkg"), b"PK\x03\x04 a stand-in")],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["report"]
            .as_str()
            .expect("a sentence")
            .contains("15 bytes"),
        "the bytes were streamed through, not dropped: {body}"
    );
    assert_eq!(
        harness.machine.recorded(),
        vec![Recorded::UploadAccepted(
            km_api::machine::Upload::Package,
            "carols.kmpkg".to_owned()
        )]
    );
}

/// The name is taken apart and rebuilt, so nothing a client wrote reaches a path.
///
/// Three things at once, and each is a real client: `..` and separators are the traversal attempt,
/// the trailing dot is what Win32 silently strips, and the extension is checked against a list and
/// replaced by one of a fixed set of `&'static str`. What lands is `passwd.kmpkg` in the staging
/// folder and nowhere else.
#[tokio::test]
async fn an_uploaded_filename_never_reaches_a_path_as_the_client_wrote_it() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .post_multipart(
            "/admin/packages/upload",
            &[("file", Some("../../../etc/passwd..kmpkg"), b"PK a stand-in")],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let Some(Recorded::UploadAccepted(_, name)) = harness.machine.recorded().first().cloned()
    else {
        panic!("the upload was not accepted");
    };
    assert_eq!(name, "passwd.kmpkg", "only the last component, and no dots");
}

/// A file the route does not take is refused before a byte is written.
#[tokio::test]
async fn a_package_route_refuses_something_that_is_not_a_package() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .post_multipart(
            "/admin/packages/upload",
            &[("file", Some("song.kar"), b"MThd")],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().expect("text").contains("song.kar"),
        "{body}"
    );
    assert!(
        harness.machine.recorded().is_empty(),
        "nothing was accepted"
    );
}

/// Each of the three routes takes its own kind and says which one it took.
#[tokio::test]
async fn each_upload_route_names_the_kind_it_accepted() {
    for (path, file, kind) in [
        (
            "/admin/wallpapers",
            "beaches.zip",
            km_api::machine::Upload::Wallpaper,
        ),
        (
            "/admin/audio/soundfonts",
            "piano.sf2",
            km_api::machine::Upload::SoundFont,
        ),
    ] {
        let harness = Harness::new().at_the_machine();
        let (status, body) = harness
            .post_multipart(path, &[("file", Some(file), b"bytes")])
            .await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(
            harness.machine.recorded(),
            vec![Recorded::UploadAccepted(kind, file.to_owned())],
            "{path}"
        );
    }
}

/// A file past the route's limit is a 413 naming the limit, not a parser complaint.
///
/// **The failure this is against named neither a size nor a limit.** Every one of these routes reads
/// its body through axum's `Multipart`, so a `DefaultBodyLimit` trip surfaces as a `MultipartError`
/// — and formatting one with `Display` yields the fixed string *"Error parsing `multipart/form-data`
/// request"* whatever actually went wrong. An 85 MB package came back with exactly that once, for a
/// file that was perfectly well formed; `km_admin_pages::router` records the episode. `status()` and
/// `body_text()` are the accessors that tell the cases apart, and this is what proves they are the
/// ones being used.
///
/// Sixty-four mebibytes and one byte: the smallest body that trips the smallest of the three limits.
/// It is a real allocation and worth the second it costs, because nothing smaller reaches the code
/// path at all.
#[tokio::test]
async fn a_file_past_the_limit_is_a_413_that_names_the_limit() {
    let harness = Harness::new().at_the_machine();
    let too_big = vec![b'x'; km_api::uploads::limit_for(km_api::machine::Upload::Wallpaper) + 1];
    let (status, body) = harness
        .post_multipart(
            "/admin/wallpapers",
            &[("file", Some("beach.jpg"), &too_big)],
        )
        .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["error"], "too_large");
    let said = body["message"].as_str().expect("text");
    assert!(
        said.contains("64 MB"),
        "the refusal has to name a number somebody can act on: {said}"
    );
    assert!(
        !said.contains("Error parsing"),
        "that is axum's `Display`, which names nothing: {said}"
    );
    assert!(harness.machine.recorded().is_empty(), "nothing was staged");
}

/// The words the machine refuses with are the words a page offers with.
#[test]
fn a_limit_reads_the_same_wherever_it_is_quoted() {
    // `km-admin` prints "Up to 64 MB." above its file chooser from this same function. A page and a
    // refusal describing one number two different ways is the drift it exists to stop.
    assert_eq!(
        km_api::uploads::limit_in_words(km_api::machine::Upload::Wallpaper),
        "64 MB"
    );
    assert_eq!(
        km_api::uploads::limit_in_words(km_api::machine::Upload::SoundFont),
        "1 GB"
    );
    // Two gibibytes less a byte, rounded **up**: a ceiling quoted as a smaller number than the thing
    // it refuses would be worse than no number at all.
    assert_eq!(
        km_api::uploads::limit_in_words(km_api::machine::Upload::Package),
        "2 GB"
    );
}

/// What a file chooser offers is derived from the list the route checks against.
///
/// Both admin surfaces read this one function, so the property under test is that the string a
/// person picks a file from cannot come to disagree with the list their file is then measured
/// against — which is what a typed-out `accept` attribute could always do, and did not only because
/// nobody had changed the table yet.
#[test]
fn a_file_chooser_offers_what_the_route_takes() {
    use km_api::machine::Upload;

    for kind in [Upload::Package, Upload::Wallpaper, Upload::SoundFont] {
        let accept = km_api::uploads::accept_for(kind);
        for extension in km_api::uploads::extensions_for(kind) {
            assert!(
                accept
                    .split(',')
                    .any(|entry| entry == format!(".{extension}")),
                "{kind:?} takes .{extension}, so its chooser has to offer it: {accept}"
            );
        }
    }

    // **A wallpaper is a pack, so the chooser offers a pack.** An `image/*` here would open a
    // phone's camera roll and put somebody one tap from a file the route refuses.
    let pictures = km_api::uploads::accept_for(Upload::Wallpaper);
    assert_eq!(pictures, ".zip", "a pack and not a picture");

    // None of the three carries a media type at all -- none has one a picker knows, so a wildcard
    // would either match nothing or offer everything. See `accept_for`'s own note.
    for kind in [Upload::Package, Upload::Wallpaper, Upload::SoundFont] {
        let accept = km_api::uploads::accept_for(kind);
        assert!(
            !accept.contains('/'),
            "{kind:?} has no media type worth naming: {accept}"
        );
    }
}

/// An upload with no `file` part is refused rather than silently doing nothing.
#[tokio::test]
async fn an_upload_with_no_file_part_says_so() {
    let harness = Harness::new().at_the_machine();
    let (status, body) = harness
        .post_multipart("/admin/wallpapers", &[("notes", None, b"hello")])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().expect("text").contains("file"),
        "{body}"
    );
}

// -- the door ---------------------------------------------------------------------------------------

/// A machine with no password refuses every admin route rather than opening every one.
///
/// **This is the inversion the whole change is about.** The old rule made an `admin` route public
/// when no password was set, on the reasoning that an owner who had not asked for a door should not
/// have one. Settings now generate a PIN at first start, so the state cannot arise in a running
/// machine — and if it somehow does, refusing is the safe answer rather than the open one.
#[tokio::test]
async fn a_machine_with_no_password_is_shut_rather_than_open() {
    // Deliberately no password at all -- the state a hand-edited settings file could produce.
    let harness = Harness::with_config(ApiConfig::default().without_mdns()).at_the_machine();
    let (status, body) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // Logging in cannot help, because there is nothing to log in with. A 404 rather than a 401,
    // deliberately: it does not confirm or deny that a password exists.
    let (status, _) = harness
        .request(
            Method::POST,
            "/admin/login",
            Some(json!({ "password": "anything" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // ...and the public half is untouched, which is what keeps a phone working.
    let (status, _) = harness.post("/queue", json!({ "number": "1001" })).await;
    assert_eq!(status, StatusCode::CREATED);
}

/// Setting a password writes a hash, never the password, and hands it to the controller.
#[tokio::test]
async fn setting_a_password_stores_a_hash_and_never_the_password() {
    let mut harness = Harness::with_password("first1975").at_the_machine();
    harness.log_in("first1975").await;

    let (status, body) = harness
        .post("/admin/password", json!({ "password": "carols1975" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["password_set"], true);
    assert!(
        body.get("password").is_none(),
        "the reply must never echo the password: {body}"
    );
    assert!(
        body.get("factory_pin").is_none(),
        "an owner's own password is not a factory PIN: {body}"
    );

    let stored = harness
        .machine
        .recorded()
        .into_iter()
        .find_map(|entry| match entry {
            Recorded::AdminPasswordSet(hash, pin) => Some((hash, pin)),
            _ => None,
        })
        .expect("the password reached the controller");
    let hash = stored.0.expect("a hash was stored");
    assert!(hash.starts_with("$argon2"), "{hash}");
    assert!(!hash.contains("carols1975"), "{hash}");
    assert_eq!(stored.1, None, "an owner's password clears the factory PIN");
}

/// Changing the password ends every session, including the one that changed it.
///
/// **A property of the construction rather than a step taken anywhere.** A token is an HMAC keyed on
/// the stored hash, so a different hash cannot produce the same MAC. The old implementation cleared
/// a map to achieve this; nothing clears anything now and the guarantee is stronger for it.
#[tokio::test]
async fn changing_the_password_ends_every_session() {
    let mut harness = Harness::with_password("first1975").at_the_machine();
    harness.log_in("first1975").await;
    let (status, _) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::OK, "the token works to begin with");

    harness
        .post("/admin/password", json!({ "password": "carols1975" }))
        .await;
    let (status, _) = harness
        .put("/admin/demo", json!({ "enabled": false }))
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the old token still opened an admin route"
    );
}

/// `null` resets to a freshly generated PIN, and the reply carries it.
///
/// **It used to clear the password entirely, and there is no such state now.** What an owner wants
/// under that button is "I have forgotten mine" — so the machine invents a new PIN, puts it on its
/// own screen, and hands it back here because a caller resetting remotely cannot go and read the
/// television.
#[tokio::test]
async fn a_null_password_resets_to_a_generated_pin() {
    let mut harness = Harness::with_password("first1975").at_the_machine();
    harness.log_in("first1975").await;

    let (status, body) = harness
        .post("/admin/password", json!({ "password": null }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["password_set"], true);
    let pin = body["factory_pin"].as_str().expect("a PIN comes back");
    assert_eq!(pin.len(), 6, "{pin}");
    assert!(pin.chars().all(|c| c.is_ascii_digit()), "{pin}");
    assert!(!pin.starts_with('0'), "{pin} would be mangled as a number");

    // The controller was told it is a factory PIN, which is what makes the screen show it.
    let recorded = harness
        .machine
        .recorded()
        .into_iter()
        .find_map(|entry| match entry {
            Recorded::AdminPasswordSet(_, pin) => Some(pin),
            _ => None,
        })
        .expect("the reset reached the controller");
    assert_eq!(recorded.as_deref(), Some(pin));
}

/// A password under the floor is refused, and nothing reaches the machine.
///
/// Four rather than eight: the PIN a machine generates for itself is six digits, and a floor above
/// what the product ships would be a rule it breaks itself.
#[tokio::test]
async fn a_password_too_short_to_protect_anything_is_refused() {
    let mut harness = Harness::with_password("first1975").at_the_machine();
    harness.log_in("first1975").await;

    let (status, body) = harness
        .post("/admin/password", json!({ "password": "abc" }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().is_some_and(|m| m.contains('4')),
        "the refusal should name the floor: {body}"
    );
    assert!(
        !harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::AdminPasswordSet(..))),
        "a refused password still reached the machine"
    );
}

/// A password of exactly the floor is accepted, which is the half the refusal above cannot state.
///
/// **The pair is the point.** A test that only asserts what is refused passes just as well against a
/// floor of eight, or of forty — so it says nothing about where the floor actually is. This one
/// counts *characters* rather than bytes, using a four-character password six bytes long, which is
/// the case that was legal on the owner's page and refused here while the two counted differently.
#[tokio::test]
async fn a_password_of_exactly_the_floor_is_accepted() {
    let mut harness = Harness::with_password("first1975").at_the_machine();
    harness.log_in("first1975").await;

    let short = "açaí";
    assert_eq!(short.chars().count(), km_api::MIN_PASSWORD_CHARS);
    assert!(short.len() > km_api::MIN_PASSWORD_CHARS);

    let (status, body) = harness
        .post("/admin/password", json!({ "password": short }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::AdminPasswordSet(..))),
        "an accepted password never reached the machine"
    );
}

/// Signing out everywhere ends every session without changing the password.
///
/// **The revocation a stateless token would otherwise cost.** There is no table to delete a row
/// from, so a session epoch is mixed into every token's MAC; moving it invalidates all of them at
/// once. It answers a phone left in a taxi, which changing the password would also do — at the cost
/// of telling the whole house a new one.
#[tokio::test]
async fn signing_out_everywhere_ends_every_session_but_keeps_the_password() {
    let mut harness = Harness::with_password("carols1975").at_the_machine();
    harness.log_in("carols1975").await;
    let (status, _) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = harness.post("/admin/sessions/reset", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _) = harness
        .put("/admin/demo", json!({ "enabled": false }))
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the token outlived a reset"
    );

    // The epoch was handed to the controller, or a restart would un-revoke everything.
    assert!(
        harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::SessionEpochSet(1))),
        "the new epoch was not persisted"
    );

    // ...and the password still works, which is the half that distinguishes this from a change.
    harness.log_in("carols1975").await;
    let (status, _) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::OK);
}

/// A token minted against one machine's hash does not open another's.
///
/// The property that makes an HMAC token safe to hand out without a table behind it: it is only
/// valid against the exact stored hash it was signed with, and every machine's argon2 salt differs.
#[tokio::test]
async fn a_token_from_one_machine_does_not_open_another() {
    let mut first = Harness::with_password("carols1975").at_the_machine();
    first.log_in("carols1975").await;
    let stolen = first.token.clone().expect("a token");

    let mut second = Harness::with_password("carols1975").at_the_machine();
    second.token = Some(stolen);
    let (status, _) = second.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// -- admin mode ----------------------------------------------------------------------------------

/// Logging in yields a token that opens an admin route.
#[tokio::test]
async fn logging_in_yields_a_token_that_opens_an_admin_route() {
    let mut harness = Harness::with_password("carols1975").tokenless();

    let (status, _) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "closed before logging in");

    harness.log_in("carols1975").await;
    let (status, _) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::OK, "open afterwards");
}

/// The wrong password is a 401, and enough of them is a lockout that the right one also waits.
#[tokio::test]
async fn the_wrong_password_is_a_401_and_a_lockout_follows() {
    let harness = Harness::with_password("carols1975");
    for _ in 0..km_api::auth::MAX_FAILED_LOGINS {
        let (status, _) = harness
            .post("/admin/login", json!({ "password": "wrong" }))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, body) = harness
        .post("/admin/login", json!({ "password": "carols1975" }))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "too_many_requests");
}

/// Logging out clears the caller's own session and says exactly that.
///
/// **It cannot revoke, and the wording does not pretend to.** A token is an HMAC rather than a row,
/// so there is nothing to delete: the browser stops presenting it, and a copy taken off the wire
/// lives until it expires or the epoch moves. Saying "logged out" flatly would be the lie.
#[tokio::test]
async fn logging_out_is_about_this_device_and_says_so() {
    let mut harness = Harness::with_password("carols1975");
    harness.log_in("carols1975").await;

    let (status, body) = harness.post("/admin/logout", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("sessions/reset")),
        "logout should point at what does revoke: {body}"
    );
}

#[tokio::test]
async fn a_made_up_token_does_not_work() {
    let mut harness = Harness::with_password("carols1975");
    harness.token = Some("v1.99999999999.aaaaaaaa.".to_owned() + &"0".repeat(32));
    let (status, body) = harness.put("/admin/demo", json!({ "enabled": true })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("log in again")),
        "{body}"
    );
}

/// `/discover` is public on a machine with a password, and says whether the PIN is still the
/// machine's own — but never what it is.
#[tokio::test]
async fn discover_reports_a_factory_password_without_ever_carrying_the_pin() {
    let factory = Harness::on_a_factory_password("123456");
    let body = factory.ok(Method::GET, "/discover", None).await;
    assert_eq!(body["factory_password"], true);
    let text = body.to_string();
    assert!(!text.contains("123456"), "the PIN reached the wire: {text}");
    assert!(
        body.get("auth").is_none(),
        "`auth` became a constant and was removed: {body}"
    );

    let owned = Harness::with_password("carols1975");
    let body = owned.ok(Method::GET, "/discover", None).await;
    assert_eq!(body["factory_password"], false);
}

/// Debugging mode is reported publicly and changed only with the password.
#[tokio::test]
async fn reading_the_debug_switch_is_public_and_moving_it_is_not() {
    let mut harness = Harness::with_password("carols1975")
        .at_the_machine()
        .tokenless();

    let body = harness.ok(Method::GET, "/debug", None).await;
    assert_eq!(body["enabled"], false);

    let (status, _) = harness
        .put("/admin/debug", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "moving it needs a token");

    harness.log_in("carols1975").await;
    let (status, body) = harness
        .put("/admin/debug", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Written down, and this run still not debugging: see the pair's own test above.
    assert_eq!(body["stored"], true);
    assert_eq!(body["enabled"], false);
    assert!(
        harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::DebugEnabledSet(true))),
        "the switch did not reach the controller"
    );
}

/// The frame-statistics panel, switched from a page rather than from a keyboard.
///
/// **The one switch of the three whose read answers the new value immediately**, because there is no
/// route to mount — just a flag the next frame draws from. So unlike its two neighbours this really
/// is a round trip, and asserting it is what stops the read half rotting while the write half works.
#[tokio::test]
async fn the_performance_panel_is_public_to_read_and_admin_to_switch() {
    let mut harness = Harness::with_password("carols1975")
        .at_the_machine()
        .tokenless();

    let body = harness.ok(Method::GET, "/performance", None).await;
    assert_eq!(body["enabled"], false);

    let (status, _) = harness
        .put("/admin/performance", json!({ "enabled": true }))
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "switching it needs a token"
    );

    harness.log_in("carols1975").await;
    let (status, body) = harness
        .put("/admin/performance", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], true, "{body}");
    assert!(
        harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::PerformanceOverlaySet(true))),
        "the switch did not reach the controller"
    );

    // The read follows at once. No restart, and no `stored` field to disagree with.
    let body = harness.ok(Method::GET, "/performance", None).await;
    assert_eq!(body["enabled"], true, "{body}");

    let (status, body) = harness
        .put("/admin/performance", json!({ "enabled": false }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], false, "{body}");
}

/// The console's switch is public to read and admin to move, exactly as debugging's is.
///
/// **And it reports `served: false` while it is on**, which is the state this pair exists for: the
/// harness has no debugging mode, so ticking this box changes nothing this run and the page has to
/// be able to say so rather than leaving somebody at a 404.
#[tokio::test]
async fn reading_the_console_switch_is_public_and_moving_it_is_not() {
    let mut harness = Harness::with_password("carols1975")
        .at_the_machine()
        .tokenless();

    let body = harness.ok(Method::GET, "/dev-remote", None).await;
    assert_eq!(body["enabled"], false);
    assert_eq!(body["served"], false);

    let (status, _) = harness
        .put("/admin/dev-remote", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "moving it needs a token");

    harness.log_in("carols1975").await;
    let (status, body) = harness
        .put("/admin/dev-remote", json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], true, "{body}");
    assert_eq!(
        body["served"], false,
        "debugging is off here, so nothing is being served: {body}"
    );
    assert!(
        harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::DevRemoteEnabledSet(true))),
        "the switch did not reach the controller"
    );

    // Same `deny_unknown_fields` as its neighbour, so a client sending the reply's own field back
    // is refused rather than quietly changing nothing.
    let (status, _) = harness
        .put("/admin/dev-remote", json!({ "served": true }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The debug routes are not mounted at all while debugging is off.
///
/// **A 404 and not a 401, which is what keeps the prefix rule exact.** They are public when they
/// exist; a third state, mounted but password-gated, would put a token-demanding path outside
/// `/api/v1/admin/` and the URL would stop being the permission.
#[tokio::test]
async fn the_debug_routes_are_absent_until_debugging_is_on() {
    let off = Harness::with_password("carols1975").at_the_machine();
    let (status, body) = off
        .post("/debug/play-file", json!({ "path": "fixtures/sample.kar" }))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], km_api::ApiError::UNKNOWN_ENDPOINT);

    let mut config = ApiConfig::default()
        .without_mdns()
        .with_password("carols1975")
        .expect("hash");
    config.debug_enabled = true;
    let on = Harness::with_config(config).at_the_machine();
    let (status, body) = on
        .post("/debug/play-file", json!({ "path": "fixtures/sample.kar" }))
        .await;
    assert_ne!(status, StatusCode::NOT_FOUND, "{body}");
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "the debug routes are public when they exist: {body}"
    );
}

// -- the catalog export ------------------------------------------------------------------------

/// The shape a mirroring client depends on: one JSON object per line, and **no wrapping array**, so
/// it can be parsed a row at a time rather than held whole.
#[tokio::test]
async fn the_export_answers_one_song_per_line_as_ndjson() {
    let harness = Harness::new();
    let (status, headers, body) = harness.raw("/songs/export").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/x-ndjson");
    assert!(!body.starts_with('['), "an array, not lines: {body}");

    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 6, "the test catalog holds six songs");
    for line in &lines {
        let song: Value = serde_json::from_str(line).expect("each line parses on its own");
        assert!(song["number"].is_string(), "{line}");
        assert!(song["title"].is_string(), "{line}");
    }
    assert!(
        body.ends_with('\n'),
        "a final newline, so appending is safe"
    );
}

/// The property the whole design rests on: following `after` with the last number seen covers the
/// catalog exactly once. A boundary that dropped or repeated a song would leave somebody's phone
/// quietly missing one, which nothing downstream could notice.
#[tokio::test]
async fn paging_the_export_covers_every_song_exactly_once() {
    let harness = Harness::new();

    let mut seen: Vec<String> = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let path = match &after {
            Some(number) => format!("/songs/export?limit=2&after={number}"),
            None => "/songs/export?limit=2".to_owned(),
        };
        let (status, _, body) = harness.raw(&path).await;
        assert_eq!(status, StatusCode::OK);
        if body.trim().is_empty() {
            break;
        }
        for line in body.lines() {
            let song: Value = serde_json::from_str(line).expect("parse");
            seen.push(song["number"].as_str().expect("a code").to_owned());
        }
        after = seen.last().cloned();
    }

    assert_eq!(seen, vec!["1001", "1002", "1003", "1004", "1005", "1006"]);
}

/// The cap is on the route, not on the client's imagination. Asking for a million rows gets a page,
/// not a refusal and not a million rows.
#[tokio::test]
async fn an_absurd_export_limit_is_clamped_rather_than_refused() {
    let harness = Harness::new();
    let (status, _, body) = harness.raw("/songs/export?limit=99999999").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.lines().count(), 6);
}

/// What makes a refresh cheap: the version is on the response and on `/discover`, so a client that
/// stored it can decide whether to download anything at all.
#[tokio::test]
async fn the_export_and_discovery_agree_about_the_catalog_version() {
    let harness = Harness::new();

    let (_, headers, _) = harness.raw("/songs/export?limit=1").await;
    let from_header = headers["x-km-catalog-version"]
        .to_str()
        .expect("a version header")
        .to_owned();
    assert_eq!(headers["etag"], format!("\"{from_header}\""));

    let discovery = harness.ok(Method::GET, "/discover", None).await;
    assert_eq!(
        discovery["catalog_version"].as_u64().expect("a version"),
        from_header.parse::<u64>().expect("a number"),
        "a client checks discovery and then trusts the export's header"
    );
}

/// Installing something has to move the number, or a mirror is told there is nothing new when there
/// is. This is the half of the mechanism that fails silently if it is wrong.
#[tokio::test]
async fn installing_a_package_moves_the_catalog_version() {
    let harness = Harness::new();
    let before = harness.ok(Method::GET, "/discover", None).await["catalog_version"]
        .as_u64()
        .expect("a version");

    harness
        .ok(
            Method::POST,
            "/admin/packages",
            Some(json!({ "path": "/tmp/vol2.kmpkg" })),
        )
        .await;

    let after = harness.ok(Method::GET, "/discover", None).await["catalog_version"]
        .as_u64()
        .expect("a version");
    assert!(after > before, "{before} -> {after}");
}

/// The export stays public on a machine with a password, and that is now permanent.
///
/// **This inverts the test it replaced**, which closed `songs.export` and asserted it refused. There
/// is no way to close it any more: it sits outside `/api/v1/admin/`, and the reasoning that used to
/// make it its own ACL id — the difference between a guest looking a song up and a guest walking off
/// with the index — is a distinction the product no longer offers to draw. Worth a test either way,
/// because it is the mirror a client keeps its catalog with.
#[tokio::test]
async fn the_export_stays_open_on_a_machine_with_a_password() {
    let harness = Harness::with_password("carols1975");

    let (status, _, _) = harness.raw("/songs/export").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = harness.get("/songs").await;
    assert_eq!(status, StatusCode::OK, "and so is search");
}

// -- the song book -------------------------------------------------------------------------------

/// A browser asking for this must be handed a file it will save, not a page it will try to show.
#[tokio::test]
async fn the_song_book_is_a_pdf_a_browser_will_download() {
    let harness = Harness::new();
    let (status, headers, body) = harness.raw_bytes("/songs/book.pdf").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/pdf");
    let disposition = headers["content-disposition"].to_str().expect("ascii");
    assert!(disposition.contains("attachment"), "{disposition}");
    // The harness machine is called `KaraokeMachine`, so the name segment collapses — see
    // `book_name_for`. An unfiltered book carries no language parenthesis either.
    assert!(
        disposition.contains("KaraokeMachine - Song Book.pdf"),
        "{disposition}"
    );
    assert!(body.starts_with(b"%PDF-1."), "not a PDF");
    assert!(body.ends_with(b"%%EOF\n"), "truncated");
    assert!(headers.contains_key("x-km-catalog-version"));
}

/// The book is a *rendering of the catalog*, so it changes exactly when the catalog does — which
/// is what makes the version a usable validator in the first place.
#[tokio::test]
async fn the_song_book_reports_the_catalog_version_discovery_reports() {
    let harness = Harness::new();
    let (_, headers, _) = harness.raw_bytes("/songs/book.pdf").await;
    let from_book = headers["x-km-catalog-version"].to_str().expect("ascii");
    let discovered = harness.ok(Method::GET, "/discover", None).await;
    assert_eq!(from_book, discovered["catalog_version"].to_string());
}

/// **The bug this route would have inherited by copying the export's header block.** There the
/// parameters are a cursor and a page size over a stable set, so the bare version identifies the
/// answer. Here the body changes with the query — so a shared `ETag` would let a client that asked
/// for Portuguese be served the Japanese book out of its own cache.
#[tokio::test]
async fn two_filters_are_two_documents_and_two_etags() {
    let harness = Harness::new();
    let (_, portuguese, _) = harness.raw_bytes("/songs/book.pdf?language=pt").await;
    let (_, japanese, _) = harness.raw_bytes("/songs/book.pdf?language=ja").await;
    let (_, everything, _) = harness.raw_bytes("/songs/book.pdf").await;

    assert_ne!(portuguese["etag"], japanese["etag"]);
    assert_ne!(portuguese["etag"], everything["etag"]);
}

#[tokio::test]
async fn a_filtered_book_is_smaller_and_says_which_language_it_is() {
    let harness = Harness::new();
    let (status, headers, filtered) = harness.raw_bytes("/songs/book.pdf?language=pt").await;
    let (_, _, everything) = harness.raw_bytes("/songs/book.pdf").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        filtered.len() < everything.len(),
        "the filter narrowed nothing"
    );
    let disposition = headers["content-disposition"].to_str().expect("ascii");
    // Spelled out, and in the book's own locale — English here, because `?locale=` was not given
    // and the harness machine speaks English. The parenthesis is the only part of the filename a
    // filter touches.
    assert!(
        disposition.contains("KaraokeMachine - Song Book (Portuguese).pdf"),
        "{disposition}"
    );
}

/// A Portuguese book downloaded in Portuguese arrives under no English word at all.
///
/// Both halves are localised and for one reason: the language through `named_language`, the way the
/// section dividers say it, and the document's own name through `book-filename`. A name like
/// `Song Book (Português)` tells the same lie more quietly rather than not telling it.
#[tokio::test]
async fn the_filename_says_the_language_in_the_books_own_locale() {
    let harness = Harness::new();
    let (status, headers, _) = harness
        .raw_bytes("/songs/book.pdf?language=pt&locale=pt-BR")
        .await;

    assert_eq!(status, StatusCode::OK);
    let disposition = headers["content-disposition"].to_str().expect("ascii");
    // Neither `Músicas` nor `Português` is ASCII, so the readable form is in `filename*` and the
    // plain `filename` carries the underscored fallback. Both are asserted: the fallback existing
    // is what stops the header from failing to build at all.
    assert!(
        disposition.contains("KaraokeMachine - Lista de M_sicas (Portugu_s).pdf"),
        "{disposition}"
    );
    assert!(disposition.contains("filename*=UTF-8''"), "{disposition}");
    assert!(
        disposition.contains("Lista%20de%20M%C3%BAsicas"),
        "{disposition}"
    );
    assert!(disposition.contains("Portugu%C3%AAs"), "{disposition}");
}

/// The name an owner typed reaches the filename, which is the whole point of the change — and it is
/// the *machine's* name rather than `?name=`, which stays out of the header entirely.
#[tokio::test]
async fn a_named_machine_is_named_in_the_filename() {
    let mut config = ApiConfig::default().without_mdns();
    config.machine_name = "Living Room".to_owned();
    let harness = Harness::with_config(config);
    let (status, headers, _) = harness.raw_bytes("/songs/book.pdf").await;

    assert_eq!(status, StatusCode::OK);
    let disposition = headers["content-disposition"].to_str().expect("ascii");
    assert!(
        disposition.contains("KaraokeMachine - Living Room - Song Book.pdf"),
        "{disposition}"
    );
}

/// A machine name is the first free text an owner types that reaches a header, and a header value
/// is visible ASCII — so this is the case that answers 500 without the encoding.
#[tokio::test]
async fn a_machine_named_outside_ascii_still_downloads() {
    let mut config = ApiConfig::default().without_mdns();
    config.machine_name = "Salão".to_owned();
    let harness = Harness::with_config(config);
    let (status, headers, body) = harness.raw_bytes("/songs/book.pdf").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with(b"%PDF-1."), "not a PDF");
    let disposition = headers["content-disposition"].to_str().expect("ascii");
    assert!(
        disposition.contains("KaraokeMachine - Sal_o - Song Book.pdf"),
        "{disposition}"
    );
    assert!(disposition.contains("Sal%C3%A3o"), "{disposition}");
}

/// A download that legibly says "nothing matched" beats a JSON error a browser renders as a blank
/// tab — and the same reasoning as search, which answers an empty list rather than a 404.
#[tokio::test]
async fn a_filter_that_matches_nothing_is_still_a_book() {
    let harness = Harness::new();
    for query in ["?language=xx", "?package=no-such-package"] {
        let (status, headers, body) = harness.raw_bytes(&format!("/songs/book.pdf{query}")).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert_eq!(headers["content-type"], "application/pdf", "{query}");
        assert!(body.starts_with(b"%PDF-1."), "{query}");
        // One page, saying so. Never zero pages: that is an invalid PDF.
        assert!(
            body.windows(9).any(|window| window == b"/Count 1 "),
            "{query} should be a single page"
        );
    }
}

/// What the whole book off an unnamed machine downloads as, in both spellings.
///
/// Written out rather than built, so a change to either half of the header has to be typed here
/// deliberately. The harness machine is called `KaraokeMachine`, which is why there is no second
/// name segment; nothing is filtered, so there is no language parenthesis.
const UNFILTERED_BOOK_DISPOSITION: &str = "attachment; filename=\"KaraokeMachine - Song Book.pdf\"; \
     filename*=UTF-8''KaraokeMachine%20-%20Song%20Book.pdf";

/// An unrecognized language cannot reach the header, because only a code the compiled-in ISO table
/// knows is ever named there.
///
/// **Still true with an owner's machine name in the filename**, which is the invariant worth
/// re-reading rather than assuming: what an owner typed is not what a *caller* typed, and `?name=`
/// — the one thing a caller can put in a masthead — is deliberately not read here at all.
#[tokio::test]
async fn nothing_a_caller_types_reaches_the_filename() {
    let harness = Harness::new();
    for query in [
        "?language=xx",
        "?language=%22pt%22",
        "?package=a%22b%0D%0Ax",
    ] {
        let (status, headers, _) = harness.raw_bytes(&format!("/songs/book.pdf{query}")).await;
        let Some(disposition) = headers.get("content-disposition") else {
            // Refused outright is also an acceptable answer, and is what a header-splitting
            // attempt gets: what must never happen is a 200 carrying the caller's text.
            assert!(
                !status.is_success(),
                "{query} answered {status} with no disposition"
            );
            continue;
        };
        assert_eq!(
            disposition.to_str().expect("ascii"),
            UNFILTERED_BOOK_DISPOSITION,
            "{query} leaked into the header"
        );
    }
}

/// Two books that differ only in what they are called are still two books.
///
/// `?name=` changes the body without changing which songs are in it, so it is the one parameter a
/// reader might assume is cosmetic. It is not: a shared validator would hand a cached copy of one
/// machine's book to somebody who asked for another's.
#[tokio::test]
async fn a_name_is_part_of_what_identifies_a_book() {
    let harness = Harness::new();
    let (_, plain, _) = harness.raw_bytes("/songs/book.pdf").await;
    let (_, sitting_room, body) = harness
        .raw_bytes("/songs/book.pdf?name=Sala%20de%20Estar")
        .await;
    let (_, kitchen, _) = harness.raw_bytes("/songs/book.pdf?name=Cozinha").await;

    assert_ne!(plain["etag"], sitting_room["etag"]);
    assert_ne!(sitting_room["etag"], kitchen["etag"]);
    // The same name twice is the same book, or the ETag would be worthless.
    let (_, again, _) = harness
        .raw_bytes("/songs/book.pdf?name=Sala%20de%20Estar")
        .await;
    assert_eq!(sitting_room["etag"], again["etag"]);

    // …and it is on the page, not merely in the header.
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("(Sala de Estar)"), "the name is drawn");
    assert!(!text.contains("(KaraokeMachine)"), "instead of the default");

    // The name narrows nothing, so the filename is untouched by it — unlike `?language=`.
    let disposition =
        harness.raw_bytes("/songs/book.pdf?name=Cozinha").await.1["content-disposition"]
            .to_str()
            .expect("ascii")
            .to_owned();
    assert_eq!(disposition, UNFILTERED_BOOK_DISPOSITION);
}

/// A name is the first free text on this route, so it is the first that could split a header.
///
/// It is hashed into the `ETag` rather than interpolated, which is what makes this pass: a `"` in a
/// name would otherwise close the entity tag early, and a CRLF would end the header.
#[tokio::test]
async fn a_name_cannot_break_out_of_the_headers() {
    let harness = Harness::new();
    for query in [
        "?name=%22quoted%22",
        "?name=a%22b%0D%0AX-Injected%3A+yes",
        "?name=%0D%0A%0D%0A",
    ] {
        let (status, headers, _) = harness.raw_bytes(&format!("/songs/book.pdf{query}")).await;
        let Some(etag) = headers.get("etag") else {
            assert!(
                !status.is_success(),
                "{query} answered {status} with no etag"
            );
            continue;
        };
        let etag = etag.to_str().expect("ascii");
        assert!(
            etag.starts_with('"')
                && etag.ends_with('"')
                && etag[1..etag.len() - 1].matches('"').count() == 0,
            "{query} produced the malformed etag {etag}"
        );
        assert!(
            headers.get("x-injected").is_none(),
            "{query} injected a header"
        );
        assert_eq!(
            headers["content-disposition"].to_str().expect("ascii"),
            UNFILTERED_BOOK_DISPOSITION,
            "{query} leaked into the disposition"
        );
    }
}

/// Public, permanently, on a machine with a password.
///
/// The prefix settles it: the book is not under `/api/v1/admin/`, so there is nothing to close. An
/// option to close it would be thin anyway — a printed song list is the most public artifact a
/// karaoke machine has.
#[tokio::test]
async fn the_song_book_stays_open_on_a_machine_with_a_password() {
    let harness = Harness::with_password("carols1975");

    let (status, _, _) = harness.raw_bytes("/songs/book.pdf").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = harness.get("/songs").await;
    assert_eq!(status, StatusCode::OK, "search is untouched");
    let (status, _, _) = harness.raw("/songs/export").await;
    assert_eq!(status, StatusCode::OK, "and so is the export");
}

// -- power ---------------------------------------------------------------------------------------

/// The gate, in both directions and in one test, because the pair is the assertion.
///
/// A machine with no power control does not merely refuse these — it does not have them. That is
/// the whole design: `Controller`'s own note says an unmounted route is the honest shape for a
/// capability a host lacks, and this is what proves it stayed that way.
#[tokio::test]
async fn the_power_routes_exist_only_on_a_host_that_can_do_something_about_its_power() {
    let without = Harness::new();
    for (method, path) in POWER_SURFACE {
        let (status, body) = without
            .request(
                Method::from_bytes(method.as_bytes()).expect("method"),
                path,
                None,
            )
            .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} should not exist here: {body}"
        );
        assert_eq!(
            body["error"], "unknown_endpoint",
            "{method} {path} must read as a path that is not there, not a thing that is missing"
        );
    }

    let with = Harness::powered();
    for (method, path) in POWER_SURFACE {
        let (status, body) = with
            .request(
                Method::from_bytes(method.as_bytes()).expect("method"),
                path,
                None,
            )
            .await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} should be mounted here: {body}"
        );
    }
}

/// The most important test in the change: switching the television off must never be reachable
/// without the password, on the one machine where the routes exist at all.
#[tokio::test]
async fn every_power_route_demands_a_token() {
    let harness = Harness::powered().tokenless();
    for (method, path) in POWER_SURFACE {
        let (status, body) = harness
            .request(
                Method::from_bytes(method.as_bytes()).expect("method"),
                path,
                None,
            )
            .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} answered without a token: {body}"
        );
    }
    assert!(
        harness.power_recorded().is_empty(),
        "a tokenless request reached the host"
    );
}

#[tokio::test]
async fn shutting_down_is_accepted_rather_than_done_and_reaches_the_host() {
    let harness = Harness::powered();
    let (status, body) = harness
        .request(Method::POST, "/admin/power/off", None)
        .await;
    // 202 and not 200: the box has not gone off when this is written, and the caller will get no
    // later word from a machine that is going dark.
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["error"], "ok");

    // The action is deliberately deferred past the response, so the assertion has to wait for it.
    // Anything else would be asserting on the ordering this handler exists to guarantee.
    for _ in 0..100 {
        if !harness.power_recorded().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(harness.power_recorded(), vec![Recorded::ShutDown]);
}

#[tokio::test]
async fn restarting_reaches_the_host_as_a_restart_and_not_as_a_shutdown() {
    // The distinction the two paths exist to keep visible: one ends the evening, the other
    // interrupts it for ten seconds, and a body field carrying a verb would have made them look
    // the same in a log.
    let harness = Harness::powered();
    let (status, body) = harness
        .request(Method::POST, "/admin/power/restart", None)
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    for _ in 0..100 {
        if !harness.power_recorded().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(harness.power_recorded(), vec![Recorded::RestartApplication]);
}

/// A refusal arrives after the 202 has gone, so it cannot be a status code — it is a journal line.
///
/// What this asserts instead is that the machine stays up and says so, which is the behaviour that
/// matters: a `systemctl` that answers *Interactive authentication required.* must not leave a box
/// that has half-stopped.
#[tokio::test]
async fn a_host_that_refuses_leaves_the_machine_running() {
    let harness = Harness::powered_but_refused("Interactive authentication required.");
    let (status, _) = harness
        .request(Method::POST, "/admin/power/off", None)
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        harness.power_recorded().is_empty(),
        "a refused request must not record as done"
    );
    let (status, _) = harness.get("/state").await;
    assert_eq!(status, StatusCode::OK, "the machine is still answering");
}

/// **The one part of the surface the development console's mirror does not get.**
///
/// That mirror re-mounts the whole API at `/dev/api/v1`, where nothing asks for a password — which
/// is its entire point and is bounded by both switches being off by default. The power routes are
/// held back from it because they are the only ones whose consequence cannot be undone from a page:
/// everything else there changes something somebody can change again, where this ends the evening
/// and needs a person to walk to the box. An owner who switched debugging and the console on opted
/// into a diagnostic surface, not into letting the network switch the television off.
#[tokio::test]
async fn the_dev_mirror_does_not_carry_the_power_routes() {
    let harness = Harness::with_config(
        ApiConfig::default()
            .without_mdns()
            .with_password(HARNESS_PASSWORD)
            .expect("argon2 hashes a password")
            .with_dev_console(),
    )
    .with_power(TestPower::new());

    for (_, path) in POWER_SURFACE {
        // Mounted under the real prefix, where the password is.
        let (status, _) = harness.request(Method::POST, path, None).await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "/api/v1{path} should be mounted on a machine with power control"
        );

        // And absent under the mirror, rather than mounted there with nothing in front of it.
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("/dev/api/v1{path}"))
            .body(Body::empty())
            .expect("request");
        let (status, _) = harness.send(request).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "/dev/api/v1{path} is reachable without a password"
        );
    }

    // The mirror is really there, so the assertion above is about the power routes and not about a
    // console that failed to mount at all.
    let request = Request::builder()
        .method(Method::GET)
        .uri("/dev/api/v1/state")
        .body(Body::empty())
        .expect("request");
    let (status, _) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK, "the dev mirror is not mounted");

    assert!(
        harness.power_recorded().is_empty(),
        "a request through the mirror reached the host"
    );
}

#[tokio::test]
async fn asking_what_the_power_controls_are_answers_both() {
    let harness = Harness::powered();
    let (status, body) = harness.get("/admin/power").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["shutdown"], true);
    assert_eq!(body["restart"], true);
}

// -- the machine's own log -----------------------------------------------------------------------

/// The gate, in both directions and in one test, exactly as the power routes have it: a machine
/// keeping no log does not refuse these, it does not have them.
#[tokio::test]
async fn the_log_routes_exist_only_on_a_machine_that_keeps_one() {
    let without = Harness::new();
    for (_, path) in LOG_SURFACE {
        let (status, body) = without.get(path).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "GET {path} should not exist here: {body}"
        );
        assert_eq!(
            body["error"], "unknown_endpoint",
            "GET {path} must read as a path that is not there, not a thing that is missing"
        );
    }

    let with = Harness::logging();
    let (status, body) = with.get("/admin/logs").await;
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "the tail should be mounted here: {body}"
    );
}

/// A log names paths, addresses and the name somebody gave the machine, so it is the owner's.
#[tokio::test]
async fn every_log_route_demands_a_token() {
    let harness = Harness::logging().tokenless();
    for (_, path) in LOG_SURFACE {
        let (status, body) = harness.get(path).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "GET {path} answered without a token: {body}"
        );
    }
}

/// What the tail says, and the two numbers beside it that make it readable.
///
/// `dropped` is what tells a reader this is a tail rather than the whole run, and `filter` is what
/// stops a quiet pane being a mystery on a machine started without `-v`.
#[tokio::test]
async fn the_tail_is_what_the_ring_kept_and_says_what_it_lost() {
    let harness = Harness::logging();
    let (status, body) = harness.get("/admin/logs").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let records = body["records"].as_array().expect("records");
    assert_eq!(records.len(), 2, "the ring holds two of the three pushed");
    assert_eq!(records[0]["message"], "the middle one");
    assert_eq!(records[1]["message"], "the newest");
    assert_eq!(body["capacity"], 2);
    assert_eq!(body["dropped"], 1, "the oldest fell off and was counted");
    assert_eq!(body["filter"], "info,km_app=debug");

    // The sequence is what a reader joins the tail to the stream with, so it has to travel.
    assert!(records[0]["seq"].as_u64() < records[1]["seq"].as_u64());
    // An event with nothing but a message says so with a null rather than an empty string.
    assert!(records[0]["fields"].is_null(), "{}", records[0]);
    assert_eq!(records[0]["level"], "info");
    assert_eq!(records[0]["target"], "km_api");
}

/// **The console reaches this without a password, and that is the point of the mirror.**
///
/// The opposite of `the_dev_mirror_does_not_carry_the_power_routes`, and the pair is what says the
/// exception there is about a change nobody can undo rather than about anything merely sensitive.
#[tokio::test]
async fn the_dev_mirror_carries_the_log_routes() {
    let harness = Harness::with_config(
        ApiConfig::default()
            .without_mdns()
            .with_password(HARNESS_PASSWORD)
            .expect("argon2 hashes a password")
            .with_dev_console(),
    );
    let tap = km_logtap::LogTap::new().with_filter("info");
    tap.push(km_logtap::Record {
        seq: 0,
        at_ms: 0,
        level: tracing::Level::WARN,
        target: "km_api",
        message: "something worth reading".to_owned(),
        fields: String::new(),
    });
    assert!(harness.state.set_log_tap(tap));

    let request = Request::builder()
        .method(Method::GET)
        .uri("/dev/api/v1/admin/logs")
        .body(Body::empty())
        .expect("request");
    let (status, body) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["records"][0]["message"], "something worth reading");

    // And the same path through the real prefix still wants the password it always did.
    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/admin/logs")
        .body(Body::empty())
        .expect("request");
    let (status, _) = harness.send(request).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the mirror must not open the route it mirrors"
    );
}

/// The corrections a curation tool has decided on reach the machine with the path.
///
/// Absent is not empty, and both have to cross as themselves: a song whose corrections were turned
/// off must not sound like a song nobody has touched, which is the distinction the person listening
/// is trying to hear.
///
/// A correction's arguments cross with it, which the forced instrument is what proves: a preview
/// that carried the name and not the program would play a different song from the one on the page.
#[tokio::test]
async fn a_files_corrections_travel_with_its_path() {
    let harness = Harness::debugging();
    harness
        .ok(
            Method::POST,
            "/debug/play-file",
            Some(json!({
                "path": "fixtures/generated/lyric_events.mid",
                "fixes": [
                    {"fix": "mute_channel", "channel": 2},
                    {"fix": "force_program", "channel": 4, "program": 52},
                ],
            })),
        )
        .await;
    assert!(harness.machine.recorded().iter().any(|entry| matches!(
        entry,
        Recorded::Decided(decided)
            if decided.fixes.as_deref() == Some(&[
                km_fixes::Fix::MuteChannel { channel: 2 },
                km_fixes::Fix::ForceProgram { channel: 4, program: 52 },
            ][..])
    )));
}

/// A correction this build knows the shape of and cannot honour is a bad request, not a fix to
/// carry on. A program change has seven bits, so nothing above 127 could have come from a file.
#[tokio::test]
async fn an_instrument_no_program_change_could_carry_is_refused() {
    let harness = Harness::debugging();
    let (status, _) = harness
        .request(
            Method::POST,
            "/debug/play-file",
            Some(json!({
                "path": "fixtures/generated/lyric_events.mid",
                "fixes": [{"fix": "force_program", "channel": 4, "program": 128}],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_file_with_nothing_decided_about_it_is_left_to_detection() {
    let harness = Harness::debugging();
    harness
        .ok(
            Method::POST,
            "/debug/play-file",
            Some(json!({ "path": "fixtures/generated/lyric_events.mid" })),
        )
        .await;
    assert!(
        harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::Decided(decided) if decided.fixes.is_none()))
    );
}

/// An empty list is a decision the wire has to carry as one.
#[tokio::test]
async fn deciding_on_no_corrections_is_not_the_same_as_deciding_nothing() {
    let harness = Harness::debugging();
    harness
        .ok(
            Method::POST,
            "/debug/play-file",
            Some(json!({ "path": "fixtures/generated/lyric_events.mid", "fixes": [] })),
        )
        .await;
    assert!(harness.machine.recorded().iter().any(|entry| matches!(
        entry,
        Recorded::Decided(decided) if decided.fixes.as_deref() == Some(&[][..])
    )));
}

/// The key a curator chose crosses with the song, and lands where a package's own would.
///
/// A preview that played the file's own key would answer the wrong question for the one curator who
/// is listening for a key: somebody deciding whether two semitones down is enough hears the song
/// they are about to package, not the song as it was found.
#[tokio::test]
async fn a_curators_key_travels_with_the_song() {
    let harness = Harness::debugging();
    harness
        .ok(
            Method::POST,
            "/debug/play-file",
            Some(json!({
                "path": "fixtures/generated/lyric_events.mid",
                "transpose": -2,
            })),
        )
        .await;
    assert!(
        harness.machine.recorded().iter().any(
            |entry| matches!(entry, Recorded::Decided(decided) if decided.transpose == Some(-2))
        )
    );
}

/// A song nobody has transposed sends nothing, and plays in the key its file is written in.
#[tokio::test]
async fn a_song_nobody_has_transposed_sends_no_key() {
    let harness = Harness::debugging();
    harness
        .ok(
            Method::POST,
            "/debug/play-file",
            Some(json!({ "path": "fixtures/generated/lyric_events.mid" })),
        )
        .await;
    assert!(
        harness.machine.recorded().iter().any(
            |entry| matches!(entry, Recorded::Decided(decided) if decided.transpose.is_none())
        )
    );
}

/// The melody channel a curator chose crosses in all three of its states.
///
/// Absent detects, `null` says there is none, and a number names one. A route that folded `null`
/// into absence would let a detector overrule the curator who said the song has no melody.
#[tokio::test]
async fn a_curators_melody_channel_travels_in_all_three_states() {
    for (body, expected) in [
        (
            json!({ "path": "fixtures/generated/lyric_events.mid" }),
            None,
        ),
        (
            json!({ "path": "fixtures/generated/lyric_events.mid", "melody": null }),
            Some(None),
        ),
        (
            json!({ "path": "fixtures/generated/lyric_events.mid", "melody": 2 }),
            Some(Some(2)),
        ),
    ] {
        let harness = Harness::debugging();
        harness
            .ok(Method::POST, "/debug/play-file", Some(body.clone()))
            .await;
        assert!(
            harness.machine.recorded().iter().any(
                |entry| matches!(entry, Recorded::Decided(decided) if decided.melody == expected)
            ),
            "{body}"
        );
    }
}

/// A melody channel outside the sixteen is refused, and nothing plays.
#[tokio::test]
async fn a_melody_channel_past_sixteen_is_a_400() {
    let harness = Harness::debugging();
    let (status, _) = harness
        .post(
            "/debug/play-file",
            json!({ "path": "fixtures/generated/lyric_events.mid", "melody": 16 }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        !harness
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::PlayFile(_)))
    );
}
