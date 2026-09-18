//! The things this program changes *about* a machine, against a machine this test controls.
//!
//! **These are the tests that would have caught a route that was never right.** Rename posted to
//! `/api/v1/machine/name` for as long as the button existed; the real path is
//! `/api/v1/admin/machine/name`, because `put_machine_name` is registered inside the `admin` router
//! and that router is nested at `/admin`. Every rename from this program answered 404, and the toast
//! it produced read as something wrong with the machine. Nothing here asserted on a path, so nothing
//! noticed.
//!
//! So each of these pins **the path and the token**, which is what a page cannot show you.
//!
//! `wiremock` binds an ephemeral loopback port, which is this repository's rule and not a
//! preference: a test binary's path carries a build hash, so one that bound a non-loopback address
//! would raise a fresh Windows firewall prompt on every rebuild.

mod common;

use axum::http::StatusCode;
use common::{get, log_in, post_form, post_form_to};
use km_admin::server::State;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Signing in is an ordinary form post, and answers an ordinary redirect.
///
/// # The bug this replaces, and why it cannot come back
///
/// `signing_in_from_the_page_is_told_to_navigate_rather_than_handed_a_page` was the test here, and
/// the bug it pinned was htmx's: the login form was the one form in this program with an `hx-post`
/// and no target of its own, so htmx aimed the answer at the form itself — followed the 303, received
/// the whole `/machine` document, and swapped it into the password box. A second nav bar and a second
/// machine panel, drawn below the field somebody had just typed into. `HX-Redirect` was the fix.
///
/// **The shared login page carries no script at all**, which is `No htmx, no script at all` and is
/// asserted by `no_shared_template_carries_a_script` over there. So the collision has no way to
/// happen: there is nothing to aim an answer at. What is left worth pinning is that the plain post
/// still works and still lands the token.
#[tokio::test]
async fn signing_in_is_a_plain_form_post_that_lands_the_token() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    Mock::given(method("POST"))
        .and(path("/api/v1/admin/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "a-token",
            "expires_in_secs": 43200,
        })))
        .mount(&server)
        .await;

    let (status, body) = post_form(&state, "/admin/login", "password=first1975").await;
    assert!(status.is_redirection(), "{status} {body}");

    // The token went where this host keeps it — its own client, not a cookie. `LoggedIn::Kept` is
    // that distinction, and a rename now going through proves the token is really held.
    Mock::given(method("PUT"))
        .and(path("/api/v1/admin/machine/name"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Living Room",
        })))
        .mount(&server)
        .await;
    let (status, body) = post_form(&state, "/admin/machine/name", "name=Living+Room").await;
    assert!(status.is_redirection(), "{status} {body}");
    let sent = server
        .received_requests()
        .await
        .expect("recorded")
        .into_iter()
        .find(|request| request.url.path() == "/api/v1/admin/machine/name")
        .expect("the rename never reached the machine");
    assert_eq!(
        sent.headers
            .get("authorization")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer a-token"),
        "the token this program kept was not sent"
    );
}

/// A rename reaches the admin route, carrying the token.
///
/// **The path is the assertion.** It read `/machine/name` and therefore missed the router it was
/// meant for entirely — a 404 that reached the page as a sentence about the machine.
#[tokio::test]
async fn a_rename_reaches_the_admin_route_with_the_token() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    Mock::given(method("PUT"))
        .and(path("/api/v1/admin/machine/name"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Living Room",
        })))
        .mount(&server)
        .await;

    let (status, body) = post_form(&state, "/admin/machine/name", "name=Living+Room").await;
    assert!(status.is_redirection(), "{status} {body}");

    let sent = server
        .received_requests()
        .await
        .expect("recorded")
        .into_iter()
        .find(|request| request.url.path() == "/api/v1/admin/machine/name")
        .expect("the rename never reached the admin route");
    assert_eq!(
        sent.headers
            .get("authorization")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer a-token"),
        "the rename went without the token it needs"
    );
}

/// The delay reaches `PUT /api/v1/admin/demo/delay`, as a number and with no `persist` beside it.
///
/// **The body is the assertion, not only the path.** A delay is always written down, so a form that
/// sent `persist` would be asking a question this route does not take — and the machine's
/// `deny_unknown_fields` would answer 400 to something the page believed had worked.
#[tokio::test]
async fn the_demo_delay_reaches_its_own_route_and_sends_no_persist() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    Mock::given(method("PUT"))
        .and(path("/api/v1/admin/demo/delay"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "enabled": false,
            "stored": false,
            "delay_secs": 45,
            "min_suitability": 5,
            "playing": false,
            "starts_in_secs": null,
        })))
        .mount(&server)
        .await;

    let (status, body) = post_form(&state, "/admin/machine/demo-delay", "delay_secs=45").await;
    assert!(status.is_redirection(), "{status} {body}");

    let sent = server
        .received_requests()
        .await
        .expect("recorded")
        .into_iter()
        .find(|request| request.url.path() == "/api/v1/admin/demo/delay")
        .expect("the delay never reached the admin route");
    assert_eq!(
        sent.headers
            .get("authorization")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer a-token"),
        "the delay went without the token it needs"
    );
    assert_eq!(
        sent.body_json::<serde_json::Value>().expect("a JSON body"),
        serde_json::json!({ "delay_secs": 45 }),
        "one field, and `persist` is not one of them"
    );

    // Something that is not a number never leaves this program, so the machine is never asked to
    // refuse what a form could have. It says so as a notice rather than a status — the page's
    // convention, not this program's.
    let before = server.received_requests().await.expect("recorded").len();
    let said = post_form_to(&state, "/admin/machine/demo-delay", "delay_secs=a+minute").await;
    assert!(said.contains("kind=bad"), "{said}");
    assert_eq!(
        server.received_requests().await.expect("recorded").len(),
        before,
        "a box holding words must not become a request"
    );
}

/// A changed password reaches `POST /api/v1/admin/password`, and only ever as a password.
///
/// **`null` is the reset and this must never send it.** `{"password": null}` puts the machine back
/// on a freshly generated PIN, which is destructive and stays on the machine's own `/admin/` page
/// beside the screen that would show the new one.
#[tokio::test]
async fn a_changed_password_reaches_the_admin_route_and_is_never_a_reset() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    Mock::given(method("POST"))
        .and(path("/api/v1/admin/password"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "password_set": true,
            "factory_pin": serde_json::Value::Null,
        })))
        .mount(&server)
        .await;

    let (status, body) = post_form(&state, "/admin/machine/password", "password=carols1975").await;
    assert!(status.is_redirection(), "{status} {body}");

    let sent = server
        .received_requests()
        .await
        .expect("recorded")
        .into_iter()
        .find(|request| request.url.path() == "/api/v1/admin/password")
        .expect("the change never reached the admin route");
    assert_eq!(
        sent.headers
            .get("authorization")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer a-token"),
        "the change went without the token it needs"
    );
    let asked: serde_json::Value = serde_json::from_slice(&sent.body).expect("a JSON body");
    assert_eq!(
        asked["password"].as_str(),
        Some("carols1975"),
        "a change was sent as something other than the password typed: {asked}"
    );
}

/// The token is dropped once the password changes, so the panel asks for the new one.
///
/// Tokens are HMACs keyed on the stored hash, so the machine has just revoked every one of them —
/// this program's included. Holding on to it would leave a page full of controls that each answer
/// 401, which is the state the login form exists to prevent.
#[tokio::test]
async fn changing_the_password_signs_this_program_out() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;
    assert!(
        state.client().expect("a client").has_token(),
        "the login did not take"
    );

    Mock::given(method("POST"))
        .and(path("/api/v1/admin/password"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "password_set": true,
            "factory_pin": serde_json::Value::Null,
        })))
        .mount(&server)
        .await;
    let (status, _) = post_form(&state, "/admin/machine/password", "password=carols1975").await;
    assert!(status.is_redirection(), "{status}");

    assert!(
        !state.client().expect("a client").has_token(),
        "a revoked token was kept, so the page would draw controls that each answer 401"
    );
}

/// A password under the floor is refused here, before anything is sent.
///
/// The machine would refuse it too, in the same words and from the same constant — this only spares
/// the round trip. The assertion worth making is that **nothing reached the machine**: a refusal
/// that still sent the request would be a rate-limit budget spent on a typo.
#[tokio::test]
async fn a_password_under_the_floor_never_leaves_this_program() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    // The refusal is a notice on the page now rather than a status with a body — see
    // `common::post_form_to`. The floor is still named, and it is still `km_api`'s: this page and
    // `POST /api/v1/admin/password` set the same password on the same machine, and a number written
    // down twice is two rules free to disagree — which they did, one counting bytes where the other
    // counted characters, so a two-character CJK password was stored by one and refused by the other.
    let said = post_form_to(&state, "/admin/machine/password", "password=abc").await;
    assert!(said.contains("kind=bad"), "{said}");
    assert!(
        said.contains(&km_api::MIN_PASSWORD_CHARS.to_string()),
        "the refusal should name the floor: {said}"
    );

    assert!(
        !server
            .received_requests()
            .await
            .expect("recorded")
            .iter()
            .any(|request| request.url.path() == "/api/v1/admin/password"),
        "a password this program refused was sent to the machine anyway"
    );
}

/// This program serves the machine's own page, from the machine's own crate.
///
/// **The claim the whole exercise rests on**, so it is asserted end to end rather than by reading the
/// wiring: a `GET` through this program's router reaches `km-admin-pages`' templates, drawn against a
/// machine over HTTP.
///
/// Three things could each be wired and still wrong, so it checks all three:
///
/// * the **prefix**, because the shared markup writes its links out and a page mounted anywhere else
///   renders with every link pointing at nothing;
/// * the **capabilities**, which cut both ways: *Screen language* is drawn because there is a route
///   behind it, and the password *reset* is not because a PIN is drawn on a television this program
///   is not beside;
/// * the **pane only this host has**: *Different machine* is drawn here and nowhere else.
#[tokio::test]
async fn the_owners_page_is_served_from_the_shared_crate() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    let (status, body) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The shared chrome, at the shared prefix.
    assert!(
        body.contains(r#"<nav class="tabs">"#) && body.contains(r#"href="/admin/sound""#),
        "the strip and its links are the shared crate's: {body}"
    );
    assert!(
        body.contains("Setting up the karaoke machine"),
        "and its words come from the machine's own catalog: {body}"
    );

    // The control only this host draws, because only this host can be pointed elsewhere — in the
    // information panel beside the name and the addresses, which is where it is reachable when the
    // machine those facts describe has gone away.
    assert!(
        body.contains(r#"<div class="choose">"#),
        "choosing a machine is this surface's, and it is in the panel: {body}"
    );

    // *Screen language*, which this host draws because `PUT /admin/machine/locale` is a route it
    // can reach — a box that arrives speaking the wrong language is set right from the program
    // somebody has open while setting it up.
    assert!(
        body.contains(r#"action="/admin/machine/locale""#),
        "the television's language is set from here too: {body}"
    );
    // ...and not the machine's own controls. The password *reset* draws a new PIN on the
    // television, so it belongs beside it.
    assert!(
        !body.contains(r#"name="clear""#),
        "the reset shows a PIN on the machine's screen, so it lives there: {body}"
    );
}

/// The machine's own contents, managed from here: each control reaches the route it should.
///
/// # What this test is for
///
/// **`What it is not is a second /admin/` was rewritten to allow this, and these are the calls that
/// make it real.** The flags were billed as a one-line flip; what was actually behind them was
/// eleven trait methods answering `AdminError::Refused` and four reads answering an empty value, so
/// turning them on alone would have drawn a table with no rows and controls that all refused.
///
/// Each assertion pins the **verb and the path**, which is what this file exists for — the rename
/// bug it opens with was a right path under a wrong prefix, and nothing that only looked at a page
/// could see it. `wire_path` follows `Call` rather than repeating a literal, so a route that moves
/// moves here too, and `every_call_this_program_makes_is_a_route_the_machine_mounts` is what says
/// the path is the machine's rather than merely this file's.
#[tokio::test]
async fn the_machines_own_contents_are_managed_from_here() {
    use km_admin::machine::Call;

    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    // Removing a package answers how many songs went with it, which is what the page says out loud.
    Mock::given(method("DELETE"))
        .and(path(common::wire_path(Call::RemovePackage("vol1"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "package_id": "vol1",
            "songs_removed": 240,
        })))
        .mount(&server)
        .await;
    let said = post_form_to(&state, "/admin/songs/vol1/remove", "").await;
    assert!(
        said.contains("240"),
        "the machine's own count did not reach the page: {said}"
    );

    // Moving one to another block answers how many changed number.
    Mock::given(method("PUT"))
        .and(path(common::wire_path(Call::SetPackageBank("vol1"))))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "package_id": "vol1",
            "bank": 3,
            "songs_renumbered": 240,
        })))
        .mount(&server)
        .await;
    let (status, body) = post_form(&state, "/admin/songs/vol1/bank", "bank=3").await;
    assert!(status.is_redirection(), "{status} {body}");

    // The next picture, and taking one out of the rotation.
    Mock::given(method("POST"))
        .and(path(common::wire_path(Call::NextWallpaper)))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let (status, body) = post_form(&state, "/admin/pictures/next", "").await;
    assert!(status.is_redirection(), "{status} {body}");

    Mock::given(method("DELETE"))
        .and(path(common::wire_path(Call::RemoveWallpaper("sunset-jpg"))))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let (status, body) = post_form(&state, "/admin/pictures/sunset-jpg/remove", "").await;
    assert!(status.is_redirection(), "{status} {body}");

    // Playing through a bank the machine already has, and deleting one off it.
    Mock::given(method("PUT"))
        .and(path(common::wire_path(Call::UseBank)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "path": "/banks/generaluser.sf2",
            "chosen_by": "setting",
            "playing": "soundfont",
            "problem": null,
            "fallback": null,
        })))
        .mount(&server)
        .await;
    let (status, body) = post_form(&state, "/admin/sound/use", "id=generaluser").await;
    assert!(status.is_redirection(), "{status} {body}");

    Mock::given(method("DELETE"))
        .and(path(common::wire_path(Call::RemoveBank("generaluser"))))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let (status, body) = post_form(&state, "/admin/sound/generaluser/remove", "").await;
    assert!(status.is_redirection(), "{status} {body}");

    // **Every one of them carried the token**, which is the other half of what this file pins: an
    // admin route reached without one answers 401, and the page would report a machine that
    // "requires a password" rather than a control that forgot to send it.
    let sent = server.received_requests().await.expect("recorded");
    let writes: Vec<_> = sent
        .iter()
        .filter(|request| request.method.as_str() != "GET")
        // Signing in is the one write with nothing to carry yet.
        .filter(|request| !request.url.path().ends_with("/admin/login"))
        .collect();
    assert!(
        writes.len() >= 6,
        "only {} writes reached the machine: {:?}",
        writes.len(),
        writes.iter().map(|r| r.url.path()).collect::<Vec<_>>()
    );
    for request in writes {
        assert!(
            request.headers.get("authorization").is_some(),
            "{} went without the token",
            request.url.path()
        );
    }
}

/// One page load asks `/discover` once, however many of its parts want it.
///
/// # Why a request count is the assertion
///
/// **Every trait method this program answers is an HTTP request**, and the shared page has no way to
/// know that: in the machine's own process the same calls are field reads. So the heading asks for
/// the machine's name, the information panel asks for its addresses and version, and the
/// factory-password banner asks whether the PIN is still the generated one — three parts, one
/// response, and none of them can see the others.
///
/// Against a machine that answers in a millisecond that is invisible. Against one that does not
/// answer, each of them costs the ask timeout before the page can say so, which is the difference
/// between a page that reports an unreachable machine and a page that looks broken.
///
/// **Counted rather than timed.** What is under test is whether a request was made, and timing it
/// would be a slow way to ask that badly.
#[tokio::test]
async fn one_page_load_asks_discover_once() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let discover = format!(
        "{}{}",
        km_api::routes::API_PREFIX,
        km_admin::machine::Call::Discover.path()
    );
    Mock::given(method("GET"))
        .and(path(discover.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "v": 1,
            "id": "a-machine",
            "name": "Living Room",
            "app": "karaokemachine",
            "version": "1.8.0",
            "api": "/api/v1",
            "factory_password": false,
            "debug_enabled": false,
            "port": 8177,
            "urls": ["http://192.0.2.1:8177"],
            "reachable": true,
        })))
        .mount(&server)
        .await;

    let (status, body) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let asked = server
        .received_requests()
        .await
        .expect("the mock records what it was asked")
        .into_iter()
        .filter(|request| request.url.path() == discover)
        .count();
    assert_eq!(
        asked, 1,
        "the page asked /discover {asked} times; its three readers share one answer"
    );

    // And the answer really did reach the page, or the count above would be one request nobody used.
    assert!(body.contains("Living Room"), "{body}");
}

/// The front door carries the box this program is given the machine's password in.
///
/// # The regression this exists for, and why nothing here caught it
///
/// **Every route this program drives to change a machine is under `/api/v1/admin/`**, so a run
/// holding no token can write nothing: the demo switch, both deletes and the rename each reached the
/// machine, were refused with a 401, and came back as a banner about a password. There was nowhere
/// on any page to type one. `POST /admin/login` was mounted and `Guard::login` implemented, and
/// nothing linked to either -- and because a tool gates none of its own routes, nothing redirected a
/// caller to the login page the way the machine's own surface does.
///
/// **[`common::log_in`] is why the suite stayed green.** It posts to `/admin/login` directly, which
/// is a path no page offered, so every test here logged in by a route a person could not reach. This
/// one asks what a person asks: is the box on the page.
#[tokio::test]
async fn the_front_door_carries_the_box_this_program_logs_in_with() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let (status, body) = get(&state, "/admin/connect").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"action="/admin/connect/use""#),
        "there is nowhere to type the machine's password: {body}"
    );
    assert!(
        body.contains(r#"name="password""#),
        "the door carries no password box: {body}"
    );
}

/// The door says what this computer already holds, and puts the box behind that.
///
/// # What this is about
///
/// **The page was read as asking for a password it already had.** The box was drawn on every launch
/// whatever this computer held, and the sentence that answered *do I have to type this again* sat
/// below the submit button, worded as a fact rather than as an answer. So the three things a person
/// wanted before pressing anything — is it saved, for which machine, and what happens if I type
/// nothing — were a sentence away from the control they were about, or nowhere.
#[tokio::test]
async fn the_door_says_it_already_has_the_password_and_which_machine_for() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    // Chosen rather than handed over on a command line, an identity being what a password is keyed
    // by and a chosen machine being the only kind that has a record to record one against.
    state.set_machine(Some(server.uri()));
    log_in(&server, &state).await;
    answers_discover(&server).await;
    post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=first1975&remember=yes",
    )
    .await;

    let (status, body) = get(&state, "/admin/connect").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("has the password for Living Room"),
        "the door does not say what it holds, or which machine for: {body}"
    );
    // The way out, beside the sentence saying there is something to forget rather than under the
    // button. `form=` because a form inside a form is not a document.
    assert!(
        body.contains(r#"form="forget-password""#),
        "no way to stop remembering: {body}"
    );
    // ...and the box is still there, one press away rather than gone: a machine whose password has
    // been changed is answered by typing the new one here.
    let Some(behind) = body.split_once(r#"<details class="retype""#) else {
        panic!("the box is not behind a summary: {body}");
    };
    assert!(
        behind.1.contains(r#"name="password""#),
        "the box is outside the summary it should be behind: {body}"
    );
    assert!(
        !behind.0.contains(r#"name="password""#),
        "a second password box above the summary: {body}"
    );
}

/// A refusal opens the box it asks somebody to use.
///
/// Every bad notice this page draws is answered by typing a password — an address that is not one, a
/// machine that did not answer, one that refused the password, a write refused three tabs away — so
/// pointing at a closed box would be the page asking for a press before the press it wants.
#[tokio::test]
async fn a_refusal_opens_the_box_rather_than_pointing_at_it() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    let (status, body) = get(&state, "/admin/connect").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"<details class="retype">"#),
        "the box opens with nothing having been refused: {body}"
    );

    let (status, body) = get(&state, "/admin/connect?kind=bad&said=nope").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"<details class="retype" open>"#),
        "a refusal left the box it names shut: {body}"
    );
}

/// A token already held is a password the machine accepted, and the door takes it.
///
/// # The bug this pins
///
/// **The page said one thing and the handler did another.** *This program is already logged in…
/// leave the box empty to stay that way* was drawn whenever a token was held, but a blank box was
/// only ever spent against a *remembered* password — so somebody who logged in without ticking the
/// box, went to a tab and came back was told to leave it empty and then refused for want of a
/// password they had already given.
#[tokio::test]
async fn a_token_already_held_opens_the_door_with_the_box_blank() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    // Logged in and nothing written down, which is what leaving the tick clear leaves behind.
    log_in(&server, &state).await;
    assert_eq!(state.remembered_password(), None);

    let landed = post_form_to(&state, "/admin/connect/use", "row=chosen&password=").await;
    assert_eq!(landed, "/admin/machine", "{landed}");

    // And the machine was not asked a second time: what was spent is the token this run already
    // had, which is the point — there was no password to spend.
    let logins = server
        .received_requests()
        .await
        .expect("recorded")
        .iter()
        .filter(|request| request.url.path() == common::wire_path(km_admin::machine::Call::Login))
        .count();
    assert_eq!(
        logins, 1,
        "the door logged in again with nothing to log in with"
    );
}

/// Once the token is held, the Machine tab says so.
#[tokio::test]
async fn the_machine_tab_says_so_once_this_program_is_logged_in() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    let (status, body) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("This program is logged in."),
        "a logged-in program does not say so: {body}"
    );
    // Debugging opens, being the pane that is never empty.
    assert!(body.contains(r#"id="machine-tab-debug" checked"#), "{body}");
    // **And nothing on this tab logs in.** What is here is the sentence saying whether the token has
    // been bought and a link to the page that buys it; the *Password* pane's boxes are a different
    // errand — changing the machine's password, which needs that token first.
    assert!(
        !body.contains(r#"action="/admin/login""#),
        "a second place to log this program in: {body}"
    );
    assert!(
        body.contains(r#"href="/admin/connect""#),
        "and no way back to the page that does: {body}"
    );
}

/// Logging in from the door enters the machine.
///
/// Choosing a machine and being let in to it are one errand, so one form does both and the answer is
/// the tab somebody came to use.
#[tokio::test]
async fn logging_in_from_the_door_enters_the_machine() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "a-token",
            "expires_in_secs": 43200,
        })))
        .mount(&server)
        .await;

    let said = post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=first1975",
    )
    .await;
    assert_eq!(said, "/admin/machine", "{said}");
}

/// A password the machine refuses comes back to the door and is not written down.
#[tokio::test]
async fn a_password_the_machine_refuses_comes_back_to_the_door() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            serde_json::json!({"error": "unauthorized", "message": "wrong password"}),
        ))
        .mount(&server)
        .await;

    let said = post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=wrong&remember=yes",
    )
    .await;
    assert!(said.starts_with("/admin/connect?"), "{said}");
    assert!(said.contains("kind=bad"), "{said}");
    assert_eq!(
        state.remembered_password(),
        None,
        "a password the machine refused was written to this computer"
    );
}

/// A write refused for want of a password lands on the door that mends it.
///
/// **This is the reported failure, end to end.** Saving the demo switch with no token answered a
/// 401, and what a person saw was a page that had closed the pane they pressed on and said something
/// about a password with nowhere to type one.
#[tokio::test]
async fn a_write_refused_for_want_of_a_password_lands_on_the_door() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    Mock::given(method("PUT"))
        .and(path(common::wire_path(km_admin::machine::Call::SetDemo)))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            serde_json::json!({"error": "unauthorized", "message": "send an admin token"}),
        ))
        .mount(&server)
        .await;

    let said = post_form_to(&state, "/admin/machine/demo", "enabled=yes").await;
    assert!(said.starts_with("/admin/connect?"), "{said}");
    assert!(said.contains("kind=bad"), "{said}");

    // And the page that `Location` names really does carry the box.
    let (status, body) = get(&state, "/admin/connect").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"name="password""#),
        "the refusal named a box the page it sent somebody to does not draw: {body}"
    );
}

/// A save that works comes back to the pane it was made on.
///
/// The demo switch is the one this was reported against: every press on this tab reloads the page,
/// so a notice read underneath a debug switch is a notice about a page somebody left.
#[tokio::test]
async fn saving_the_demo_switch_stays_on_the_demo_pane() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    log_in(&server, &state).await;

    Mock::given(method("PUT"))
        .and(path(common::wire_path(km_admin::machine::Call::SetDemo)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "enabled": true,
            "stored": true,
            "delay_secs": 90,
            "min_suitability": serde_json::Value::Null,
            "playing": false,
            "starts_in_secs": serde_json::Value::Null,
        })))
        .mount(&server)
        .await;

    let said = post_form_to(&state, "/admin/machine/demo", "enabled=yes&persist=yes").await;
    assert!(said.contains("pane=demo"), "{said}");
    assert!(said.contains("kind=good"), "{said}");
}

/// A machine that answers `/discover`, so a page load learns which machine it is pointed at.
///
/// The id is what a remembered password and the follow are both keyed by, so a test about either
/// needs a machine that has answered at least once.
async fn answers_discover(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(common::wire_path(km_admin::machine::Call::Discover)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "v": 1,
            "id": "test-machine-id",
            "name": "Living Room",
            "app": "karaokemachine",
            "version": "1.10.0",
            "api": "/api/v1",
            "factory_password": false,
            "debug_enabled": false,
            "port": 8177,
            "urls": ["http://192.0.2.1:8177"],
            "reachable": true,
        })))
        .mount(server)
        .await;
}

/// A machine that answers is recorded, and that is what a remembered password is keyed by.
///
/// # The regression this exists for
///
/// **`machine_answered` had no callers at all.** `/discover` is what says which machine is at the
/// address somebody chose, and the page that reads it stopped recording what it learned -- so the
/// follow, which keys on the id, could no longer recognize *that machine moved* rather than a
/// stranger, and nothing could be remembered against a machine with no identity.
#[tokio::test]
async fn a_machine_that_answers_is_recorded_by_its_id() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    answers_discover(&server).await;
    // Chosen rather than handed over with `--machine`, which is deliberately not written down -- so
    // there would be no record for an identity to be recorded against. See `State::remembering`.
    state.set_machine(Some(server.uri()));

    assert_eq!(
        state.machine_id(),
        None,
        "nothing has answered, so there is no identity yet"
    );
    // And so there is nothing to offer a box against.
    assert!(
        state.remembering().is_none(),
        "a machine with no identity cannot be remembered"
    );

    let (status, body) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(
        state.machine_id().as_deref(),
        Some("test-machine-id"),
        "the page read /discover and threw away which machine answered"
    );
    let remembering = state.remembering().expect("an identity to key one under");
    assert!(!remembering.on, "nothing was asked to be remembered");
}

/// Nothing is keyed under a machine that has not said which machine it is.
///
/// **The rule is about storing, and the door had to stop expressing it by not drawing the box.**
/// One row shared by every anonymous machine is a password handed to whichever answers next, so a
/// password may only be written down under an id — but this page asks the machine nothing, by
/// design, so on a first login there is no id yet and a box gated on one could never be ticked. The
/// id is recorded by `handlers::enter` at the moment a login proves the machine is up, and the
/// password is stored after that. This asserts both halves: a machine that never answers keeps
/// nothing, and one that does is remembered under its own id.
#[tokio::test]
async fn a_password_is_remembered_only_under_a_machine_that_named_itself() {
    // A machine that takes the password and will not say who it is: no `/discover` mock at all.
    let silent = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "a-token",
            "expires_in_secs": 43200,
        })))
        .mount(&silent)
        .await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(silent.uri()));
    state.set_machine(Some(silent.uri()));

    // The box is offered, because a password typed here can be checked.
    let (status, body) = get(&state, "/admin/connect").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"name="remember""#), "{body}");

    post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=first1975&remember=yes",
    )
    .await;
    assert!(
        state.remembering().is_none(),
        "a password was keyed under a machine that never named itself"
    );
    assert_eq!(state.remembered_password(), None);

    // And one that does answer is remembered, on the login that learned the identity.
    let server = MockServer::start().await;
    answers_discover(&server).await;
    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "a-token",
            "expires_in_secs": 43200,
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("temp dir");
    let answering = State::new(dir.path().to_path_buf(), Some(server.uri()));
    answering.set_machine(Some(server.uri()));

    post_form_to(
        &answering,
        "/admin/connect/use",
        "row=chosen&password=first1975&remember=yes",
    )
    .await;
    assert!(
        answering.remembering().expect("an identity").on,
        "the machine named itself and the password was not kept"
    );
}

/// Ticking the box remembers the password, and the next run spends it without being asked.
///
/// **A remembered password is a standing instruction to log in, not a login that has happened.** It
/// is spent at the moment a token is wanted, which is what lets it survive a token expiring and the
/// machine forgetting its own on restart.
#[tokio::test]
async fn a_remembered_password_is_spent_when_a_token_is_wanted() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    answers_discover(&server).await;
    // Chosen rather than handed over with `--machine`, which is deliberately not written down -- so
    // there would be no record for an identity to be recorded against. See `State::remembering`.
    state.set_machine(Some(server.uri()));

    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "a-token",
            "expires_in_secs": 43200,
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(common::wire_path(km_admin::machine::Call::Rename)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Living Room",
        })))
        .mount(&server)
        .await;

    // One load, so the machine has an identity to key a password under.
    let (status, _) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);

    let said = post_form_to(&state, "/admin/login", "password=first1975&remember=yes").await;
    assert!(said.contains("kind=good"), "{said}");
    assert!(
        state.remembering().expect("an identity").on,
        "a ticked box did not remember"
    );
    assert_eq!(
        state.remembered_password().as_deref(),
        Some("first1975"),
        "the password the machine accepted was not the one kept"
    );

    // A fresh run over the same data directory holds no token, and the first write logs itself in.
    let next = State::new(dir.path().to_path_buf(), Some(server.uri()));
    assert!(
        !next.client().expect("a client").has_token(),
        "a token was carried across a restart, which is not what is remembered"
    );
    let said = post_form_to(&next, "/admin/machine/name", "name=Living+Room").await;
    assert!(
        said.contains("kind=good"),
        "a remembered password was not spent: {said}"
    );
    assert!(
        next.client().expect("a client").has_token(),
        "the write went through without a token being obtained"
    );
}

/// A password the machine refused is never written down.
#[tokio::test]
async fn a_refused_password_is_not_remembered() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    answers_discover(&server).await;
    // Chosen rather than handed over with `--machine`, which is deliberately not written down -- so
    // there would be no record for an identity to be recorded against. See `State::remembering`.
    state.set_machine(Some(server.uri()));

    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            serde_json::json!({"error": "unauthorized", "message": "incorrect password"}),
        ))
        .mount(&server)
        .await;

    let (status, _) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = post_form(&state, "/admin/login", "password=wrongone&remember=yes").await;
    assert!(
        !status.is_redirection(),
        "a refused login answered a redirect"
    );
    assert!(
        !state.remembering().expect("an identity").on,
        "a password the machine refused was written down"
    );
}

/// An unticked box forgets, and so does the Forget button.
///
/// **The box is a statement about what this computer should be remembering** rather than an act taken
/// once, so logging in with it clear removes whatever was there. The button is the way out that does
/// not run through logging in, which is the state somebody trying to stop is already in.
#[tokio::test]
async fn an_unticked_box_forgets_and_so_does_the_button() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    answers_discover(&server).await;
    // Chosen rather than handed over with `--machine`, which is deliberately not written down -- so
    // there would be no record for an identity to be recorded against. See `State::remembering`.
    state.set_machine(Some(server.uri()));

    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "a-token",
            "expires_in_secs": 43200,
        })))
        .mount(&server)
        .await;

    let (status, _) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);

    post_form_to(&state, "/admin/login", "password=first1975&remember=yes").await;
    assert!(state.remembering().expect("an identity").on);

    // Logging in again with the box clear.
    post_form_to(&state, "/admin/login", "password=first1975").await;
    assert!(
        !state.remembering().expect("an identity").on,
        "an unticked box left a password remembered"
    );

    // And the button, from a state where one is remembered.
    post_form_to(&state, "/admin/login", "password=first1975&remember=yes").await;
    assert!(state.remembering().expect("an identity").on);
    let said = post_form_to(&state, "/admin/machine/password/forget", "").await;
    assert!(said.starts_with("/admin/connect?"), "{said}");
    assert!(
        !state.remembering().expect("an identity").on,
        "the button left the password remembered"
    );
}

/// A remembered password lands in the folder this run was given, and nowhere else.
///
/// # The regression this exists for
///
/// **A test suite created a config directory in the owner's own profile.** The store's location was
/// a process-global path guarded by `cfg!(test)`, and `cfg(test)` is per crate: an integration test
/// links the library compiled *without* it, so the guard was off in exactly the tests that drive the
/// whole program. Every one of these that remembered a password was writing into
/// `%APPDATA%/km-admin/`, which is the owner's state and the one place this program's own notes say
/// nothing a test starts may touch.
///
/// The repair is that the path arrives as an argument. `--data-dir` then covers a run somebody starts
/// by hand for the same reason a scratch directory covers a test, which is the arrangement
/// `crate::keys` already had for provider keys.
#[tokio::test]
async fn a_remembered_password_lands_in_the_folder_this_run_was_given() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    answers_discover(&server).await;
    state.set_machine(Some(server.uri()));

    Mock::given(method("POST"))
        .and(path(common::wire_path(km_admin::machine::Call::Login)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "a-token",
            "expires_in_secs": 43200,
        })))
        .mount(&server)
        .await;

    let (status, _) = get(&state, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);

    let file = dir.path().join("machine-passwords.json");
    assert!(!file.exists(), "nothing was asked to be remembered yet");

    post_form_to(&state, "/admin/login", "password=first1975&remember=yes").await;
    assert!(
        file.is_file(),
        "the password went somewhere other than the folder this run was given"
    );
    // Beside the provider keys and the settings, its own file: deleting it is a whole answer to
    // *forget my password*, and two files cannot become one decision by accident.
    let held = std::fs::read_to_string(&file).expect("the store reads back");
    assert!(held.contains("first1975"), "{held}");

    // And forgetting deletes it rather than leaving `{}`, which would read as one being remembered.
    post_form_to(&state, "/admin/machine/password/forget", "").await;
    assert!(
        !file.exists(),
        "an emptied file says a password is remembered"
    );
}

/// A send this program refuses for want of a password lands on the page that holds one.
///
/// # The bug this pins
///
/// **A form navigates, so a status with a sentence in it becomes the whole document.** The two send
/// controls answered `401 text/plain`, on the reading that htmx's error listener would word it —
/// and neither control is an htmx control. What a person saw was one line of unstyled monospace on
/// white: no heading, no strip, no link, and a sentence naming a tab they could not get to.
///
/// So the assertion is about the *shape* of the answer and not only its words: a redirect, to the
/// door, and the door drawing a box to type a password into.
#[tokio::test]
async fn a_send_with_no_password_lands_on_the_door_that_holds_one() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    // A pack on this computer, so the send gets past the "there is no such pack" floor and reaches
    // the token check, which is the one under test.
    let pack = km_admin::pictures::packs_dir(dir.path()).join("wallpapers-aaaaaaaa");
    std::fs::create_dir_all(&pack).expect("a pack folder");
    std::fs::write(pack.join("wallpapers-aaaaaaaa.zip"), b"PK").expect("a zip");

    let landed = post_form_to(&state, "/admin/pictures/packs/wallpapers-aaaaaaaa/send", "").await;
    assert!(
        landed.starts_with("/admin/connect?"),
        "a refused send goes to the door, not to {landed}"
    );
    assert!(landed.contains("kind=bad"), "{landed}");
    // The sentence names where the box is, and the box is here rather than on the *This machine*
    // tab, which carries a status banner about the machine's own password and no field at all.
    assert!(landed.contains("Type it here"), "{landed}");

    // ...and the door it goes to is a page, with the control the sentence asks somebody to use.
    // `post_form_to` reads `+` back as a space for legibility, so the query is not re-requestable;
    // what this asks of the destination is that it is the door and that the box is on it.
    let (status, body) = get(&state, "/admin/connect").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains(r#"name="password""#),
        "the door drew no password box"
    );

    assert!(
        server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty(),
        "a send with no password reached the machine anyway"
    );
}

/// A notice meant for one of this program's own pages is drawn on it.
///
/// **Without this the redirect above lands somewhere silent.** `Admin::shell` and `Admin::door`
/// took no notice, so a page reached by `?kind=&said=` rendered the query string and nothing else;
/// the door looked right only because its own template drew a second copy of the banner. Both of
/// this program's shelled pages are swept, because the banner is chrome and either one can be the
/// destination of a refusal.
#[tokio::test]
async fn a_notice_meant_for_this_programs_own_page_is_drawn_on_it() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    for page in [
        "/admin/pictures/find",
        "/admin/sound/fetch",
        "/admin/connect",
    ] {
        let (status, body) = get(&state, &format!("{page}?kind=bad&said=nope")).await;
        assert_eq!(status, StatusCode::OK, "{page}: {body}");
        assert!(
            body.contains("banner-bad") && body.contains("nope"),
            "{page} drew no banner"
        );
        assert_eq!(
            body.matches("nope").count(),
            1,
            "{page} drew the notice twice"
        );
    }
}

/// Every control this program owns answers a refusal on the page it was pressed on.
///
/// **One test over the whole class, because they are one bug and not eight.** Each of these
/// answered a status with a sentence in it, which a form post turns into the whole document. The
/// fault arranged here is the cheapest one each control has — an id nothing matches — and what is
/// asserted is the shape every one of them must now keep: a redirect, to that control's own
/// section, carrying a notice.
#[tokio::test]
async fn a_refused_control_lands_on_its_own_page_with_a_notice() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let cases = [
        ("/admin/sound/fetch/nosuchbank/get", "/admin/sound/fetch"),
        ("/admin/sound/fetch/nosuchbank/send", "/admin/sound/fetch"),
        ("/admin/sound/fetch/nosuchbank/remove", "/admin/sound/fetch"),
        (
            "/admin/pictures/packs/nosuchpack/send",
            "/admin/pictures/find",
        ),
        (
            "/admin/pictures/packs/nosuchpack/remove",
            "/admin/pictures/find",
        ),
    ];

    for (route, page) in cases {
        let landed = post_form_to(&state, route, "").await;
        assert!(
            landed.starts_with(&format!("{page}?")),
            "{route} went to {landed} rather than to {page}"
        );
        assert!(landed.contains("kind=bad"), "{route}: {landed}");
        assert!(landed.contains("said="), "{route} said nothing: {landed}");
    }
}

/// ...and the one that does not, because nothing it answers is a document.
///
/// **A thumbnail is an `<img src>`.** A `Location` in an image slot fetches a page into a picture
/// frame; the browser's own alt text is the failure a reader can see. This is the counter-assertion
/// to the sweep above, so that "everything redirects" is not read into it.
#[tokio::test]
async fn a_thumbnail_that_is_missing_stays_a_status() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let (status, _) = get(&state, "/admin/pictures/thumb/openverse/nosuchpicture").await;
    assert!(
        !status.is_redirection(),
        "a thumbnail answered a redirect: {status}"
    );
    assert!(
        status.is_client_error() || status.is_server_error(),
        "{status}"
    );
}

/// The name typed beside the search button reaches the run.
///
/// # The bug this pins
///
/// **The name box and the search button were on two different forms.** The box lived in the
/// settings form, which *Save* posts; *Search, measure and build* was a form of its own with no
/// fields in it, and the run read only what had already been saved. So typing a name and pressing
/// the button next to it built a pack called `wallpapers-<hash>.zip`, and nothing on the page said
/// that Save had to be pressed first.
///
/// The assertion is on what got stored rather than on a file name, so it needs no build and no
/// network: what was broken was the trip from the box to the settings.
#[tokio::test]
async fn the_name_typed_beside_the_search_button_reaches_the_run() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let form = "name=Praias+do+Sul&provider=openverse&terms=beach&pages=2&target_count=40\
                &target_contrast=4.5&min_source_width=1920&min_decoded_width=1280";
    let landed = post_form_to(&state, "/admin/pictures/run", form).await;
    assert!(
        landed.starts_with("/admin/pictures/find"),
        "the run went to {landed}"
    );

    assert_eq!(
        state.pictures_settings().name.as_deref(),
        Some("praias-do-sul"),
        "the name on the form never reached the settings the run reads"
    );
}

/// A search with no name is refused on the page, and starts nothing.
///
/// The `required` attribute is a courtesy a stale page can post around, so the rule is the server's.
#[tokio::test]
async fn a_search_with_no_name_is_refused_and_starts_no_job() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let form = "provider=openverse&terms=beach&pages=2&target_count=40\
                &target_contrast=4.5&min_source_width=1920&min_decoded_width=1280";
    let landed = post_form_to(&state, "/admin/pictures/run", form).await;
    assert!(landed.starts_with("/admin/pictures/find?"), "{landed}");
    assert!(landed.contains("kind=bad"), "{landed}");
    assert!(
        state.pictures_job().is_none(),
        "a search with no name started anyway"
    );
}

/// Both buttons post the one form, and nothing suggests a name.
///
/// **What stops the form being split again**, which is what the bug was. The run button reaches back
/// to the settings form with `form=` because the *Forget the keys* form sits between them.
#[tokio::test]
async fn both_buttons_post_one_form_and_no_name_is_suggested() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let (status, body) = get(&state, "/admin/pictures/find").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert!(
        body.contains(r#"form="pack-search" formaction="/admin/pictures/run""#),
        "the search button does not carry the form's fields"
    );
    assert!(
        !body.contains(r#"<form method="post" action="/admin/pictures/run">"#),
        "the search button is back on a form of its own"
    );
    assert!(
        body.contains(r#"name="name""#) && body.contains("required"),
        "the name box is not required"
    );
    assert!(
        !body.contains("placeholder=\"beaches\"") && !body.contains("wallpapers-beaches-"),
        "the page still suggests a name"
    );
}

/// The door does not open on a blank password with nothing remembered.
///
/// **The requirement, stated once.** A program let in without a password refuses every write on
/// every tab behind this page, which reads as a broken machine rather than as a question nobody
/// answered.
#[tokio::test]
async fn the_door_does_not_open_without_a_password() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));

    let landed = post_form_to(&state, "/admin/connect/use", "row=chosen&password=").await;
    assert!(landed.starts_with("/admin/connect?"), "{landed}");
    assert!(landed.contains("kind=bad"), "{landed}");
    assert!(
        !state.logged_in(),
        "a blank password let this program in anyway"
    );
    // Nothing was asked of the machine either: there was nothing to ask with.
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty(),
        "the door called the machine with no password"
    );
}

/// A machine that does not answer is told apart from a password it refused.
///
/// Two different things to do about it — switch the machine on, or retype — so two sentences.
#[tokio::test]
async fn a_machine_that_does_not_answer_says_so_rather_than_blaming_the_password() {
    let dir = tempfile::tempdir().expect("temp dir");
    // A port nothing is listening on. Loopback, which is this repository's rule for a test.
    let state = State::new(dir.path().to_path_buf(), Some("127.0.0.1:1".to_owned()));

    let landed = post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=first1975",
    )
    .await;
    assert!(landed.contains("kind=bad"), "{landed}");
    assert!(
        landed.contains("did not answer"),
        "an unreachable machine was reported as a refused password: {landed}"
    );
}

/// A remembered password opens the door with the box left blank.
///
/// **Blank means *spend what this computer remembers***, which is the other half of requiring one:
/// somebody who ticked the box does not type it again every launch.
#[tokio::test]
async fn a_remembered_password_opens_the_door_with_the_box_blank() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    // Chosen rather than handed over on a command line: only a chosen machine has a record for an
    // identity to be written against, and an identity is what a password is keyed by.
    state.set_machine(Some(server.uri()));
    log_in(&server, &state).await;
    answers_discover(&server).await;

    // Typed once, with the box ticked.
    let landed = post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=first1975&remember=yes",
    )
    .await;
    assert_eq!(landed, "/admin/machine", "{landed}");
    assert!(state.remembered_password().is_some(), "nothing was stored");

    // ...and not typed again.
    let landed = post_form_to(&state, "/admin/connect/use", "row=chosen&password=").await;
    assert_eq!(landed, "/admin/machine", "{landed}");
}

/// Spending a remembered password keeps it.
///
/// # The bug this pins
///
/// **The tick is spent on the password in hand, and a blank box has none.** The door draws no
/// checkbox where nothing is being typed — there is nothing for it to be a statement about — so an
/// entry that read its absence as *forget* would delete a credential as a side effect of using it,
/// on the one press the page exists for. Using a thing is not asking for it to be thrown away.
///
/// The route out is *Forget it*, which is beside the sentence saying there is something to forget,
/// and `an_unticked_box_forgets_and_so_does_the_button` is where both of those are pinned.
#[tokio::test]
async fn spending_a_remembered_password_keeps_it() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let state = State::new(dir.path().to_path_buf(), Some(server.uri()));
    // Chosen rather than handed over on a command line: only a chosen machine has a record for an
    // identity to be written against, and an identity is what a password is keyed by.
    state.set_machine(Some(server.uri()));
    log_in(&server, &state).await;
    answers_discover(&server).await;

    post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=first1975&remember=yes",
    )
    .await;
    assert!(state.remembered_password().is_some(), "nothing was stored");

    // The box blank and no tick in the body, which is every entry the saved state draws.
    let landed = post_form_to(&state, "/admin/connect/use", "row=chosen&password=").await;
    assert_eq!(landed, "/admin/machine", "{landed}");
    assert_eq!(
        state.remembered_password().as_deref(),
        Some("first1975"),
        "using the remembered password threw it away"
    );

    // ...and typing one with the tick cleared still forgets, the box being drawn on that pass.
    let landed = post_form_to(
        &state,
        "/admin/connect/use",
        "row=chosen&password=first1975",
    )
    .await;
    assert_eq!(landed, "/admin/machine", "{landed}");
    assert!(
        state.remembered_password().is_none(),
        "an unticked box kept a password that was typed beside it"
    );
}

/// A run pointed elsewhere does not borrow the recorded machine's password.
///
/// # The bug this pins
///
/// **A password is keyed by machine id, and the id was read out of the record unconditionally.** A
/// `--machine` run is never written down, so the record still names whichever machine was chosen
/// last — and the id from it would key one machine's password and then offer it to another box
/// entirely, on the first write that wanted a token.
#[tokio::test]
async fn a_run_pointed_elsewhere_does_not_borrow_another_machines_password() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("temp dir");

    // Machine A: chosen, logged into, remembered.
    let chosen = State::new(dir.path().to_path_buf(), Some(server.uri()));
    chosen.set_machine(Some(server.uri()));
    log_in(&server, &chosen).await;
    answers_discover(&server).await;
    post_form_to(
        &chosen,
        "/admin/connect/use",
        "row=chosen&password=first1975&remember=yes",
    )
    .await;
    assert!(
        chosen.remembered_password().is_some(),
        "the machine that was chosen remembers nothing"
    );

    // Machine B: the same data directory, a different address on the command line.
    let elsewhere = State::new(dir.path().to_path_buf(), Some("127.0.0.1:1".to_owned()));
    assert_eq!(
        elsewhere.remembered_password(),
        None,
        "a run pointed elsewhere was handed another machine's password"
    );
    assert!(
        !elsewhere.can_remember(),
        "a run pointed elsewhere offers a box whose tick it would discard"
    );
}
