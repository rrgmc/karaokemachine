//! Sending a file somebody already has, against a machine this test controls.
//!
//! **Nothing here touches the network.** `wiremock` binds an ephemeral loopback port, which is also
//! the rule this repository has a scar about: a test that binds a non-loopback address raises a
//! fresh Windows firewall prompt for every rebuild, because a test binary's path carries a build
//! hash. Sixty dead rules accumulated once from a single `0.0.0.0:0`.
//!
//! Three things are worth a test here and only one of them is "it works". The other two are
//! decisions nothing else can observe: **which name reaches the machine**, and **that nothing is
//! left on this disk** whichever way the transfer ended.
//!
//! ## What this file used to assert, and why it proved nothing
//!
//! Every mock below was mounted on the path the *client* sends to, typed out a second time here. So
//! `wiremock` answered 200 to a request a real machine answers 404 — all three uploads were missing
//! the `/admin` every one of the machine's write routes moved behind, and this file agreed with them
//! because it was written from the same belief. The program sent no file anywhere for its whole
//! life, and four green tests said otherwise.
//!
//! **The paths are computed from the client now**, through [`common::upload_path`], so this file can
//! no longer hold an opinion about them at all. What makes them *true* lives where the truth is:
//! `every_call_this_program_makes_is_a_route_the_machine_mounts` in `src/machine.rs` compares every
//! call to `km_api::routes::SURFACE`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TOKEN, log_in, upload_path};
use km_admin::server::{State, router};
use km_api::machine::Upload;
use tower::ServiceExt as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A multipart body with one file part, built by hand.
///
/// There is no multipart *builder* on this side of the wire — `reqwest`'s is for what this program
/// sends, not for what it receives — and four hand-written bodies are clearer than a helper.
fn multipart(file_name: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "----km-admin-upload-test";
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{file_name}\"\r\n\r\n",
        km_api::uploads::FILE_FIELD
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

/// Sends one file, and gives back the status and where the browser was sent next.
///
/// # Both halves, because the answer moved
///
/// **The shared upload route awaits the machine and answers a redirect carrying a notice.** So there
/// is no job to settle and nothing to wait for: the notice in the `Location` *is* the outcome, and it
/// is there by the time the response is. A spawned job answering a progress fragment with a `200`
/// would need a status read and then a poll.
///
/// That is a trade rather than a loss — the job machinery earns its keep on a *download*, where
/// archive.org serves banks at about 30 KB/s, against an upload to a machine on the same network. It
/// also makes these assertions simpler: the machine has been called by the time the post returns.
async fn send(state: &State, route: &str, file_name: &str, bytes: &[u8]) -> (StatusCode, String) {
    let (content_type, body) = multipart(file_name, bytes);
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(route)
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let said = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .replace('+', " ");
    (status, said)
}

// **`settled` went with the job it waited on.** It polled the Songs job for five seconds because a
// send was spawned and the handler answered a progress fragment before the transfer had happened.
// The shared route awaits the machine, so there is nothing to wait for: the notice in the `Location`
// is the outcome, and every assertion here reads it directly. See `send` above.

/// How many files are sitting in the staging folder right now.
fn left_behind(dir: &std::path::Path) -> usize {
    match std::fs::read_dir(km_admin::staging::dir(dir)) {
        Ok(entries) => entries.count(),
        Err(_) => 0,
    }
}

/// The machine is sent the name the browser chose, and a request it can measure.
///
/// **Two decisions in one exchange, and neither is visible from the page.**
///
/// The *name*: the file is staged here under a number this program picked, because nothing is ever
/// written under a name that came off a network. What goes on the wire is the name the person chose,
/// because the machine reads its stem — a bank arrives called whatever the stem says, so sending the
/// staging name would install a SoundFont called `0`.
///
/// The *length*: `Part::stream_with_length` is what lets `Form::compute_length` give the request a
/// real `Content-Length`, which is what lets the machine refuse an oversized upload before reading
/// it. Streaming the browser's field straight through would have produced a chunked request instead,
/// and this header is the only thing that would ever notice.
#[tokio::test]
async fn the_machine_is_sent_the_name_the_browser_chose_and_a_length() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    Mock::given(method("POST"))
        .and(path(upload_path(Upload::Package)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "report": "installed \"Carols 1999\" · 42 songs"
        })))
        .mount(&server)
        .await;

    let (status, said) = send(
        &state,
        "/admin/songs/upload",
        "Carols 1999.kmpkg",
        b"PK\x03\x04not-really",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{said}");
    assert!(said.contains("kind=good"), "{said}");
    assert!(
        said.contains("installed") && said.contains("42 songs"),
        "the machine's own sentence reaches the page, not one this program invented: {said}"
    );

    let wanted = upload_path(Upload::Package);
    let requests = server.received_requests().await.expect("recorded");
    let sent: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == wanted)
        .collect();
    assert_eq!(
        sent.len(),
        1,
        "the package did not reach {wanted} exactly once; it went to {:?}",
        requests.iter().map(|r| r.url.path()).collect::<Vec<_>>()
    );
    let sent = sent[0];

    // **A right path is worth nothing without the token**, and these are admin routes: an upload
    // that arrived unauthenticated would be refused by a real machine and answered 200 by this one.
    assert_eq!(
        sent.headers
            .get("authorization")
            .map(|value| value.to_str().unwrap()),
        Some(format!("Bearer {TOKEN}").as_str()),
        "the package went without the token the route needs"
    );

    let body = String::from_utf8_lossy(&sent.body);
    assert!(
        body.contains(r#"filename="Carols 1999.kmpkg""#),
        "the machine was sent the staging name instead of the chosen one: {body}"
    );
    assert!(
        sent.headers.contains_key("content-length"),
        "the request went chunked; the machine cannot refuse it before reading it"
    );

    assert_eq!(left_behind(dir.path()), 0, "a copy was kept");
}

/// Nothing is kept, whichever way it goes.
///
/// **This is the test that proves "no copy".** The file was already on the owner's disk, so keeping
/// a second one here would be a gigabyte of somebody's library with nothing to retry that the
/// original cannot — and the guard that arranges that lives inside the spawned task, which is
/// exactly the sort of thing a refactor moves out without noticing.
#[tokio::test]
async fn nothing_is_kept_whichever_way_it_goes() {
    // **The two rows now differ in *who words the refusal*, which is the seam's whole shape.** A 401
    // becomes `AdminError::Unauthorized` — a code, which the page renders from its own catalog,
    // because a status line carries no sentence a page could show and this host may be talking to a
    // machine in another language. Anything else keeps the machine's own words: it is the authority
    // on what it is refusing, and a second opinion here would be a second thing to keep in agreement
    // with it. See `host::fault` and `AdminError`.
    // **A 401 also differs in *where it lands*.** It is the one refusal this program can do
    // something about, so it goes to the front door, which holds the password box, rather than back
    // to the tab the send was made from -- a message naming the fix on a page that does not carry it
    // is a message somebody then has to go and find. See `AdminError::wants_password`.
    for (status, body, expected, door) in [
        (
            401_u16,
            serde_json::json!({"error": "unauthorized", "message": "incorrect password"}),
            // The catalog's, not the machine's `incorrect password`.
            "Type it here.",
            true,
        ),
        (
            500,
            serde_json::json!({"error": "internal", "message": "the machine is busy"}),
            "the machine is busy",
            false,
        ),
    ] {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().expect("temp dir");
        let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
        log_in(&server, &state).await;

        Mock::given(method("POST"))
            .and(path(upload_path(Upload::Package)))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;

        let (code, said) = send(&state, "/admin/songs/upload", "carols.kmpkg", b"anything").await;
        assert_eq!(code, StatusCode::SEE_OTHER, "{said}");
        // The machine's refusal reaches the page as a notice rather than through a job's `error`.
        assert!(said.contains("kind=bad"), "{status} produced: {said}");
        assert!(said.contains(expected), "{status} produced: {said}");
        if door {
            assert!(
                said.starts_with("/admin/connect?"),
                "a {status} has to land on the page that mends it: {said}"
            );
        } else {
            assert!(
                !said.starts_with("/admin/connect?"),
                "a {status} is not something the password box can mend: {said}"
            );
        }
        assert_eq!(
            left_behind(dir.path()),
            0,
            "a {status} left the staged file behind"
        );
    }
}

/// A file of the wrong kind never reaches the machine at all.
///
/// Asserting the status code alone would pass with the gate deleted, because a machine that is not
/// there refuses everything too. What this asserts is that nothing was *sent*.
#[tokio::test]
async fn a_wrong_kind_never_reaches_the_machine() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    // **Signed in on purpose**, or this would be testing the wrong gate: the password check is step
    // 2 and the extension check step 3, so a program holding no token refuses a `.txt` with a 401
    // for a reason that has nothing to do with its being a `.txt`.
    log_in(&server, &state).await;

    Mock::given(method("POST"))
        .and(path(upload_path(Upload::Package)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"report": "no"})))
        .mount(&server)
        .await;

    let (code, said) = send(
        &state,
        "/admin/songs/upload",
        "notes.txt",
        b"just some notes",
    )
    .await;
    assert_eq!(code, StatusCode::SEE_OTHER, "{said}");
    assert!(said.contains("kind=bad"), "{said}");
    assert!(
        !sent_to(&server, &upload_path(Upload::Package)).await,
        "a .txt was offered to the machine"
    );
    assert_eq!(left_behind(dir.path()), 0);
}

/// A send with no password behind it refuses here, and the machine never hears about it.
///
/// **The pre-flight's own assertion**, and the reason it is worth one: all three upload routes are
/// admin routes, and a 401 arriving part-way through a multipart stream reads as a dropped
/// connection to the sending half — so the refusal somebody most needs would be the one least
/// likely to arrive. Refusing first means the two gibibytes are never pushed across loopback at all.
///
/// The same shape as `a_password_under_the_floor_never_leaves_this_program` one file over.
#[tokio::test]
async fn a_send_with_no_password_never_leaves_this_program() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"report": "no"})))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    for (route, file_name) in [
        ("/admin/songs/upload", "carols.kmpkg"),
        ("/admin/pictures/upload", "beaches.zip"),
        ("/admin/sound/upload", "piano.sf2"),
    ] {
        let (code, said) = send(&state, route, file_name, b"bytes").await;
        assert_eq!(code, StatusCode::SEE_OTHER, "{said}");
        // **The pre-flight still refuses before a byte is read**, which is the whole point — see
        // `stage_and_send`'s `has_token` floor. What changed is only that it says so as a notice.
        assert!(
            said.contains("kind=bad"),
            "{route} tried to send without a token: {said}"
        );
    }
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty(),
        "a send with no password reached the machine anyway"
    );
    assert_eq!(left_behind(dir.path()), 0, "a refused send staged a file");
}

/// The other two routes go to the other two places, which is the only thing that maps them.
///
/// **This is the test that was meant to catch the bug and instead recorded it.** It mounted its mock
/// on `/api/v1/wallpapers` and `/api/v1/audio/soundfonts` — the client's own two strings, typed
/// again — so it asserted that the client sends where the client sends, which is true of any client
/// and says nothing about any machine. Both were real routes, as `GET`; the sends were POSTs to
/// them, and a real machine answered 405 to every picture and every bank this program ever offered.
///
/// It now takes the path from the client rather than restating it, which makes it a test of the
/// mapping only — Pictures to the wallpaper route, Sound to the bank route — and leaves *whether
/// those routes exist* to the sweep in `src/machine.rs` that reads the machine's own table.
#[tokio::test]
async fn each_section_sends_to_its_own_route() {
    for (route, file_name, kind) in [
        ("/admin/pictures/upload", "beaches.zip", Upload::Wallpaper),
        ("/admin/sound/upload", "piano.sf2", Upload::SoundFont),
    ] {
        let expected = upload_path(kind);
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().expect("temp dir");
        let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
        log_in(&server, &state).await;

        Mock::given(method("POST"))
            .and(path(expected.clone()))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"report": "took it"})),
            )
            .mount(&server)
            .await;

        let (code, said) = send(&state, route, file_name, b"bytes").await;
        assert_eq!(code, StatusCode::SEE_OTHER, "{said}");
        assert!(said.contains("kind=good"), "{said}");

        // **No waiting.** The shared route awaits the machine, so by the time the post has answered
        // the request has been made — where a spawned job would need the mock polled for seconds.
        assert!(
            sent_to(&server, &expected).await,
            "{route} did not reach {expected}"
        );
    }
}

/// Whether anything has yet arrived at one path on the mock.
///
/// A path rather than a count, because signing in is a request too — every test here now makes at
/// least two, and `received_requests().len()` stopped meaning "the upload happened".
async fn sent_to(server: &MockServer, wanted: &str) -> bool {
    server
        .received_requests()
        .await
        .expect("recorded")
        .iter()
        .any(|request| request.url.path() == wanted)
}
