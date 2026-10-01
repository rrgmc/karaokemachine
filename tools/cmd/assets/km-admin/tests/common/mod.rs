//! What both test files need in order to be a logged-in program talking to a machine.
//!
//! **Shared rather than copied, and the reason is the bug these tests exist for.** A copy of
//! [`log_in`] would carry its own `/api/v1/admin/login`, and a second spelling of a path written
//! from one belief is exactly how this program came to send every file it had to a route that does
//! not exist. One copy, and it is built from `km_api`'s own constants rather than typed.
//!
//! `wiremock` binds an ephemeral loopback port, which is this repository's rule and not a
//! preference: a test binary's path carries a build hash, so one that bound a non-loopback address
//! would raise a fresh Windows firewall prompt on every rebuild.

// **Each test binary compiles its own copy of this module**, so anything only one of them uses is
// dead code in the other — `upload.rs` needs the upload paths and `machine.rs` does not. The
// alternative is a helper crate for four functions.
#![allow(dead_code)]

use axum::http::{HeaderValue, StatusCode};
use km_admin::server::{State, router};
use km_testkit::http;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The token `log_in` installs, so a test can assert the request carried it.
pub const TOKEN: &str = "a-token";

/// The full path of one [`km_admin::machine::Call`], as it appears on the wire.
///
/// **Computed, never typed.** A literal here would be a second copy of the belief under repair, and
/// a corrected literal proves only that this file and the client agree. What makes the path *true*
/// is `every_call_this_program_makes_is_a_route_the_machine_mounts` in `src/machine.rs` and
/// `the_upload_paths_clients_are_given_are_really_mounted` in `km-api`, which both compare it to the
/// machine's own route table. This just has to follow the client wherever that says it goes.
#[must_use]
pub fn wire_path(call: km_admin::machine::Call<'_>) -> String {
    format!("{}{}", km_api::routes::API_PREFIX, call.path())
}

/// The path a file of this kind is sent to.
#[must_use]
pub fn upload_path(kind: km_api::machine::Upload) -> String {
    wire_path(km_admin::machine::Call::Send(kind))
}

/// A machine standing in for the real one, and this program's state pointed at it.
///
/// The folder is this program's data folder. The caller holds it for the length of the test, as
/// `_dir` when nothing else reads it.
pub async fn machine() -> (MockServer, State, tempfile::TempDir) {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    (server, state, dir)
}

/// The same, signed in with [`TOKEN`], for a test about what happens after the password.
pub async fn signed_in() -> (MockServer, State, tempfile::TempDir) {
    let (server, state, dir) = machine().await;
    log_in(&server, &state).await;
    (server, state, dir)
}

/// `GET`s one of this program's routes.
pub async fn get(state: &State, route: &str) -> (StatusCode, String) {
    let answer = http::send(router(state.clone()), http::get(route)).await;
    (answer.status, answer.text())
}

/// Posts a form to one of this program's own routes.
pub async fn post_form(state: &State, route: &str, body: &str) -> (StatusCode, String) {
    let answer = http::send(router(state.clone()), http::form(route, body.to_owned())).await;
    (answer.status, answer.text())
}

/// Posts a form and reads where it sent the browser next.
///
/// **The shared page reports a refusal as a redirect carrying a notice**, not as a status with a
/// sentence in the body: `good`/`warn`/`bad` and the words ride in the `Location`'s query string,
/// which is `km-admin-pages`' arrangement — a notice that survives a reload and needs no flash
/// cookie. This program's own controls used to answer a `400` or a `401` with the sentence in the
/// body, so a test that asserted a status is now asserting the wrong half.
///
/// `+` is turned back into a space so an assertion can be written in words.
pub async fn post_form_to(state: &State, route: &str, body: &str) -> String {
    let answer = http::send(router(state.clone()), http::form(route, body.to_owned())).await;
    assert!(
        answer.status.is_redirection(),
        "a form post answers with a redirect carrying the notice, not {}",
        answer.status
    );
    answer.location().replace('+', " ")
}

/// Posts a form the way the page does — as htmx, which is what the login box uses.
///
/// Answers with the status, the `HX-Redirect` header if there was one, and the body.
pub async fn post_form_as_htmx(
    state: &State,
    route: &str,
    body: &str,
) -> (StatusCode, Option<String>, String) {
    let mut request = http::form(route, body.to_owned());
    request
        .headers_mut()
        .insert("HX-Request", HeaderValue::from_static("true"));
    let answer = http::send(router(state.clone()), request).await;
    let redirect = answer
        .headers
        .get("hx-redirect")
        .map(|value| value.to_str().expect("a header of text").to_owned());
    let text = answer.text();
    (answer.status, redirect, text)
}

/// Signs this program in, so the routes under test have a token to send.
///
/// **Every send needs this**, which is the visible half of the pre-flight: a send refuses before it
/// reads the body when no password has been typed, so a test that does not log in is testing the
/// refusal rather than the send.
pub async fn log_in(server: &MockServer, state: &State) {
    Mock::given(method("POST"))
        .and(path(wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": TOKEN,
            "expires_in_secs": 43200,
        })))
        .mount(server)
        .await;
    let (status, body) = post_form(state, "/admin/login", "password=first1975").await;
    assert!(status.is_redirection(), "{status} {body}");
}
