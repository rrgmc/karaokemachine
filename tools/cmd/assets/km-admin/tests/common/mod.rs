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

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use km_admin::server::{State, router};
use tower::ServiceExt as _;
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

/// `GET`s one of this program's routes.
pub async fn get(state: &State, route: &str) -> (StatusCode, String) {
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(route)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 512 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Posts a form to one of this program's own routes.
pub async fn post_form(state: &State, route: &str, body: &str) -> (StatusCode, String) {
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(route)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
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
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(route)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("response");
    assert!(
        response.status().is_redirection(),
        "a form post answers with a redirect carrying the notice, not {}",
        response.status()
    );
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .replace('+', " ")
}

/// Posts a form the way the page does — as htmx, which is what the login box uses.
///
/// Answers with the status, the `HX-Redirect` header if there was one, and the body.
pub async fn post_form_as_htmx(
    state: &State,
    route: &str,
    body: &str,
) -> (StatusCode, Option<String>, String) {
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(route)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let redirect = response
        .headers()
        .get("hx-redirect")
        .map(|value| value.to_str().expect("a header of text").to_owned());
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    (
        status,
        redirect,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
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
