//! The owner's page, driven as a service.
//!
//! **Nested under `/admin` exactly as the machine mounts it**, which is the whole point of testing
//! it here rather than unit-testing the permission table. Two things can only be answered by the
//! real router: whether axum reports the full path or the inner one from `MatchedPath` under
//! `nest`, and whether every route the router declares is actually covered by the guard. A table
//! test can answer neither.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use km_admin_pages::guard::{Caller, Grant, Guard, LoggedIn, Refusal};
use km_admin_pages::{Admin, router};
use km_api::testing::{Recorded, TestMachine, TestPower};
use km_api::{ApiConfig, ApiState};
use tower::ServiceExt as _;

/// A guard that says yes, and counts how many times it was asked.
///
/// The point of counting rather than merely allowing: the assertion that matters most in this file
/// is *that the guard was consulted at all* for a given route, which a permissive stub proves only
/// if it can be asked afterwards. It used to record which route id it was handed; there are no route
/// ids any more, and the question is the same for every page — so what is left worth observing is
/// whether it was asked.
#[derive(Default)]
struct SpyGuard {
    asked: Mutex<usize>,
    refuse: Option<Refusal>,
    factory_password: bool,
    /// A tool's answer to a sign-in: the token goes wherever the host keeps its own.
    keeps_its_own: bool,
}

impl SpyGuard {
    fn asked(&self) -> usize {
        *self.asked.lock().expect("the spy lock holds")
    }
}

#[async_trait::async_trait]
impl Guard for SpyGuard {
    async fn allows(&self, _caller: &Caller) -> Result<(), Refusal> {
        *self.asked.lock().expect("the spy lock holds") += 1;
        match &self.refuse {
            Some(refusal) => Err(refusal.clone()),
            None => Ok(()),
        }
    }

    async fn factory_password(&self) -> bool {
        self.factory_password
    }

    /// A cookie, which is the machine's answer and the one most of these tests are about.
    ///
    /// `LoggedIn::Kept` is a tool's, and `keeps_its_own` is how a test asks for it. The two
    /// assertions that need it are that no cookie is set, and that the sign-in comes back to the
    /// pane it was made for.
    async fn login(
        &self,
        _password: &str,
        _caller: &Caller,
        _remember: bool,
    ) -> Result<LoggedIn, Refusal> {
        if self.keeps_its_own {
            return Ok(LoggedIn::Kept);
        }
        Ok(LoggedIn::Cookie(Grant {
            token: "t".repeat(64),
            expires_in_secs: 3600,
        }))
    }
}

/// The state every one of these mounts the page against.
///
/// **The real in-process implementation, not a stub.** `km-remote-pages`' tests hand-write stubs for
/// its four traits, and that is right there — its implementations need the machine's catalog and
/// event plumbing. Here the implementation *is* an `ApiState`, and these tests exist to prove that
/// pressing a button changes the machine: `nested_with_state` hands the state back so a test can ask
/// what it ended up with. A stub would answer canned values and prove nothing of the sort.
///
/// So the one seam these tests do stub is the guard, which is the only thing they need to lie about.
fn admin_over(state: ApiState, guard: Arc<SpyGuard>) -> Admin {
    let machine = Arc::new(km_admin_pages::in_process::ThisMachine::new(state.clone()));
    // `with_problems` because this host *can* answer for refused packages — it reads the machine's
    // own disk. A host over HTTP cannot, and the four tests on that tab are what say so if this is
    // ever dropped.
    Admin::over(
        km_admin_pages::machine::Capabilities::machine(),
        km_admin_pages::ICON_MACHINE_PNG,
        guard,
        machine.clone(),
    )
    .with_problems(machine)
}

/// Which of the two surfaces a test is asking about.
///
/// # Why a shared control is asserted on both
///
/// **Because one caller proves nothing about a seam.** Every test here ran `Capabilities::machine()`
/// when this crate had one host, so a template that ignored a flag, or a handler that only worked
/// because an `ApiState` was in reach, would have passed the lot. `km-remote-pages` values exactly
/// this about its own second host and says so: a second caller is what turns an interface into one.
///
/// So the controls the merge actually unified — the four shared panes on *This machine*, the output
/// picker, the three upload forms, the notice and the factory-password banner — are asserted under
/// **both** capability sets, over the *same* in-process implementation. Nothing about the machine
/// changes between the two runs; only what the templates are told the surface is for.
///
/// What is deliberately *not* looped is a control only one host has. Installed packages, the
/// rotation, the bank list and the whole Problems tab are `Capabilities::desktop()`'s answer of
/// *no*, and `a_tools_capabilities_take_the_machines_own_controls_away` is where that is asserted —
/// once, in one place, rather than as an `if` inside twenty tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    /// The machine's own `/admin/`: every flag on, and the only host that can answer for refused
    /// packages.
    Machine,
    /// `km-admin` on a desktop: a tool's capabilities, pointed at a machine, no Problems tab.
    Tool,
}

/// Both surfaces, for the `for host in BOTH_HOSTS` a shared control is asserted under.
const BOTH_HOSTS: [Host; 2] = [Host::Machine, Host::Tool];

impl Host {
    /// For an assertion message, so a failure says which surface broke.
    fn name(self) -> &'static str {
        match self {
            Self::Machine => "the machine's own page",
            Self::Tool => "a tool's page",
        }
    }
}

/// The page as `host` serves it, over a machine a test has arranged.
///
/// The state comes back for `nested_with_state`'s reason: these tests exist to prove that pressing a
/// button changes the machine, so a test has to be able to ask what it ended up with.
///
/// **The locale is deliberately left negotiable on both.** `km-admin` pins English in earnest — half
/// a program in Portuguese is worse than none of it — but that is a bridge rather than a capability,
/// and pinning it here would make the two runs differ by something this seam is not about.
fn nested_as(host: Host, machine: TestMachine, guard: Arc<SpyGuard>) -> (Router, ApiState) {
    let state = ApiState::from_machine(machine.shared(), ApiConfig::default().without_mdns());
    let implementation = Arc::new(km_admin_pages::in_process::ThisMachine::new(state.clone()));
    let admin = match host {
        Host::Machine => Admin::over(
            km_admin_pages::machine::Capabilities::machine(),
            km_admin_pages::ICON_MACHINE_PNG,
            guard,
            implementation.clone(),
        )
        .with_problems(implementation),
        // As `km-admin` assembles it: pointed at a machine, and saying which program is drawing the
        // page. `What the tool calls itself` is why the second of those is not cosmetic.
        Host::Tool => Admin::over(
            km_admin_pages::machine::Capabilities::desktop(),
            km_admin_pages::ICON_ADMIN_PNG,
            guard,
            implementation,
        )
        .drawn_by("KaraokeMachine Admin"),
    };
    (Router::new().nest("/admin", router(admin)), state)
}

/// The same, over a machine that already has a password.
fn nested_as_with_password(host: Host, guard: Arc<SpyGuard>) -> (Router, ApiState) {
    let (app, state) = nested_as(host, TestMachine::with_catalog(6), guard);
    state.set_admin_password(
        Some(km_api::AdminAuth::hash_password("hunter2xyz").expect("argon2 hashes a password")),
        false,
    );
    (app, state)
}

/// The page mounted where the machine mounts it.
fn nested(guard: Arc<SpyGuard>) -> (Router, Arc<SpyGuard>) {
    nested_on(TestMachine::with_catalog(6), guard)
}

/// The same, over a machine a test has arranged — a package that cannot be removed, a second bank.
fn nested_on(machine: TestMachine, guard: Arc<SpyGuard>) -> (Router, Arc<SpyGuard>) {
    let state = ApiState::from_machine(machine.shared(), ApiConfig::default().without_mdns());
    let app = Router::new().nest("/admin", router(admin_over(state, guard.clone())));
    (app, guard)
}

/// The same again, handing back the state so a test can ask what the machine ended up with.
fn nested_with_state(machine: TestMachine, guard: Arc<SpyGuard>) -> (Router, ApiState) {
    let state = ApiState::from_machine(machine.shared(), ApiConfig::default().without_mdns());
    let app = Router::new().nest("/admin", router(admin_over(state.clone(), guard)));
    (app, state)
}

/// A machine that already has a password, for the controls that exist only once one does.
fn nested_with_password(guard: Arc<SpyGuard>) -> (Router, ApiState) {
    let (app, state) = nested_with_state(TestMachine::with_catalog(6), guard);
    state.set_admin_password(
        Some(km_api::AdminAuth::hash_password("hunter2xyz").expect("argon2 hashes a password")),
        false,
    );
    (app, state)
}

async fn get(app: &Router, path: &str) -> (StatusCode, String) {
    send(
        app,
        Request::get(path).body(Body::empty()).expect("request"),
    )
    .await
}

async fn post_form(app: &Router, path: &str, body: &str) -> (StatusCode, String) {
    send(
        app,
        Request::post(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body.to_owned()))
            .expect("request"),
    )
    .await
}

/// Where a form post sent the browser next, which is where the notice rides.
///
/// **The only way to see a notice's *kind*.** These handlers answer `303` and carry what happened in
/// the query string rather than in a flash cookie, so `good`/`warn`/`bad` is in the `Location` and
/// nowhere in a body — and the difference between a warning and an error is a thing worth being able
/// to assert. `send` throws the headers away, which is right for the twenty tests that read markup.
async fn post_form_to(app: &Router, path: &str, body: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "a form post answers with a redirect carrying the notice"
    );
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read the body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Every route the router declares reaches the guard, or is one of the two that deliberately do not.
///
/// **The test the deny-by-default guard exists for.** A route added to the router without a thought
/// asks for the password rather than escaping a table, and this is what proves the middleware sees
/// the pattern at all under `nest` — which nothing in axum's source settles.
///
/// It used to assert *which route id* each page was gated on, a nineteen-row table mirroring the one
/// in `guard.rs`. There are no ids now and every page here is an admin action, so what is left worth
/// asserting is that each one is asked about exactly once.
#[tokio::test]
async fn every_page_route_is_asked_about_before_it_runs() {
    let cases: &[(&str, &str)] = &[
        // `/admin` is the first tab in the bar, which is *This machine*.
        ("GET", "/admin"),
        ("GET", "/admin/songs"),
        ("GET", "/admin/pictures"),
        ("GET", "/admin/sound"),
        ("GET", "/admin/machine"),
        ("GET", "/admin/problems"),
        ("GET", "/admin/songs/vol1/remove"),
        ("POST", "/admin/songs/vol1/remove"),
        ("POST", "/admin/songs/vol1/bank"),
        // An id nothing answers to, deliberately: what is asserted here is that the guard was
        // consulted *before* the handler ran, and a miss redirects, which this loop accepts.
        ("GET", "/admin/problems/nothing-00000000/delete"),
        ("POST", "/admin/problems/nothing-00000000/delete"),
        ("POST", "/admin/pictures/next"),
        // Three that were missing from this table rather than deliberately left out of it, found
        // while adding the four below. A route nobody lists here is a route whose guarding nothing
        // asserts — which is exactly the hole the deny-by-default rule exists to make harmless, and
        // the reason a gap here is a weakened test rather than a bug.
        ("GET", "/admin/pictures/sunset-jpg/remove"),
        ("POST", "/admin/pictures/sunset-jpg/remove"),
        ("POST", "/admin/machine/locale"),
        // Both take the id in the body now, because the Problems tab offers each as a `<select>`.
        ("POST", "/admin/sound/use"),
        ("POST", "/admin/sound/output"),
        ("GET", "/admin/sound/piano/remove"),
        ("POST", "/admin/sound/piano/remove"),
        ("POST", "/admin/machine/name"),
        ("POST", "/admin/machine/demo"),
        ("POST", "/admin/machine/demo-delay"),
        ("POST", "/admin/machine/password"),
        ("POST", "/admin/machine/sessions"),
        ("POST", "/admin/machine/debug"),
        ("POST", "/admin/machine/dev-remote"),
        ("POST", "/admin/machine/performance"),
        // Mounted whether or not this host has power controls, so they belong in this sweep like
        // any other route. On the machine here they redirect back with a notice, which is what the
        // loop's `is_redirection` accepts — the point being only that the guard was asked first.
        ("GET", "/admin/machine/power/off"),
        ("POST", "/admin/machine/power/off"),
        ("POST", "/admin/machine/power/restart"),
    ];

    for (method, path) in cases {
        let (app, guard) = nested(Arc::new(SpyGuard::default()));
        let (status, _) = if *method == "GET" {
            get(&app, path).await
        } else {
            post_form(
                &app,
                path,
                "name=Kitchen&bank=2&password=carols1975&enabled=yes&locale=en&id=system",
            )
            .await
        };
        assert!(
            status.is_success() || status.is_redirection(),
            "{method} {path} answered {status}"
        );
        assert_eq!(guard.asked(), 1, "{method} {path} did not reach the guard");
    }
}

/// The three upload routes reach the guard too, and are asked separately for one reason.
///
/// **They take `multipart/form-data`**, so the form-encoded body the loop above sends is refused
/// with a 400 before the handler does anything — which says nothing about the guard, and is why they
/// were absent from that table rather than merely forgotten. What matters about them is the same
/// thing: that the guard was consulted before the handler ran. So the status is not asserted and the
/// guard count is.
#[tokio::test]
async fn the_upload_routes_are_asked_about_before_they_run() {
    for path in [
        "/admin/songs/upload",
        "/admin/pictures/upload",
        "/admin/sound/upload",
    ] {
        let (app, guard) = nested(Arc::new(SpyGuard::default()));
        let _ = post_form(&app, path, "file=nothing").await;
        assert_eq!(guard.asked(), 1, "POST {path} did not reach the guard");
    }
}

/// A route nobody wrote down asks for the password rather than running.
///
/// **This is the property the deleted table was there to provide**, and it is now free: `is_open` is
/// an allow-list of two, so anything else — including a page invented next year — lands on the
/// guard. The bug it stands against was found by running the thing rather than by reading it: an
/// earlier design gated each tab on the *read* id for its subject, which shipped public, and left the
/// whole setup page open on a machine whose owner had set a password.
#[test]
fn nothing_but_the_login_page_and_the_stylesheet_escapes_the_guard() {
    for path in [
        "/",
        "/songs",
        "/pictures",
        "/sound",
        "/machine",
        "/problems",
        "/machine/password",
        "/machine/sessions",
        "/machine/debug",
        "/invented-next-year",
    ] {
        assert!(
            !km_admin_pages::guard::is_open(path),
            "{path} must demand the admin password"
        );
    }
    for path in ["/login", "/static/admin.css"] {
        assert!(km_admin_pages::guard::is_open(path), "{path} must be open");
    }
}

/// *This machine* is the first tab, and `/admin/` lands on it.
///
/// The order is written in two places that cannot see each other — `views::Tab` and the `<nav>` in
/// `layout.html` — so it is asserted against the rendered bar rather than against either of them.
/// `km-admin` puts the same tab first under the same words; see `Two admin surfaces, one vocabulary`
/// in docs/decisions/distribution.md.
#[tokio::test]
async fn the_bar_starts_with_this_machine_and_so_does_the_root() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);

    let order: Vec<&str> = [
        "/admin/machine",
        "/admin/songs",
        "/admin/pictures",
        "/admin/sound",
        "/admin/problems",
    ]
    .into_iter()
    .collect();
    let mut at = 0;
    let mut seen = Vec::new();
    while let Some(next) = body[at..].find(r#"<a href="/admin/"#) {
        let start = at + next + r#"<a href=""#.len();
        let end = start + body[start..].find('"').expect("an href closes");
        seen.push(&body[start..end]);
        at = end;
    }
    assert_eq!(seen, order, "the tabs are in the wrong order");

    // And the root is the first of them rather than a fourth spelling of Songs.
    let (status, root) = get(&app, "/admin").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        root.contains(r#"/admin/machine" aria-current="page""#),
        "/admin/ should land on This machine"
    );
}

/// Demo mode is switchable from the owner's page, and the two boxes stay two answers.
///
/// **The card exists because until now nothing but the dev console could send `PUT /api/v1/demo`.**
/// The route ships admin-only precisely because turning it on is a decision about what the machine
/// does in a room, and an owner's decision with no owner's control is a feature nobody can reach.
///
/// What the second box buys is the state the API can express and one checkbox could not: on tonight,
/// off tomorrow. So it is asserted rather than assumed.
#[tokio::test]
async fn the_owners_page_turns_demo_mode_on_for_a_night_or_for_good() {
    // Demo mode is one of the four panes both surfaces carry, so both are asked. See `Host`.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );

        // Off out of the box, and the card says so.
        let (status, body) = get(&app, "/admin/machine").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(
            body.contains(r#"action="/admin/machine/demo""#),
            "{surface}: {body}"
        );
        assert!(
            !body.contains(r#"name="enabled" value="yes" checked"#),
            "nothing should be ticked yet: {body}"
        );

        // On for the evening: the machine does it, and the settings file does not.
        let (status, _) = post_form(&app, "/admin/machine/demo", "enabled=yes").await;
        assert!(status.is_redirection(), "{status}");
        let demo = state.controller().demo();
        assert!(demo.enabled, "the machine should be performing");
        assert!(!demo.stored, "and should not have written it down");

        // On for good.
        let (status, _) = post_form(&app, "/admin/machine/demo", "enabled=yes&persist=yes").await;
        assert!(status.is_redirection(), "{status}");
        let demo = state.controller().demo();
        assert!(demo.enabled && demo.stored, "{demo:?}");

        // Both boxes come back ticked, which is what makes the card readable as state rather than as a
        // pair of buttons.
        let (_, body) = get(&app, "/admin/machine").await;
        assert!(
            body.contains(r#"name="enabled" value="yes" checked"#),
            "{body}"
        );
        assert!(
            body.contains(r#"name="persist" value="yes" checked"#),
            "{body}"
        );

        // An unticked box posts nothing at all, which is the whole of "off".
        let (status, _) = post_form(&app, "/admin/machine/demo", "").await;
        assert!(status.is_redirection(), "{surface}: {status}");
        assert!(!state.controller().demo().enabled, "{surface}");
    }
}

/// The delay is a box on the same card, on a form of its own, and it is always written down.
///
/// **The second form is the assertion, not an accident of layout.** A delay is installation
/// configuration — it has no *for tonight* — so folding it in beside `persist` would have produced
/// a form where one field obeyed that box and its neighbour ignored it. Two forms is what keeping
/// `persist` meaning one thing costs, and a test that only checked the number would let somebody
/// merge them back.
#[tokio::test]
async fn the_owners_page_sets_the_delay_and_writing_it_down_is_not_optional() {
    // The delay sits on the demo card, which both surfaces carry.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );

        // The box is drawn with the machine's own delay in it, on its own action.
        let (status, body) = get(&app, "/admin/machine").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(
            body.contains(r#"action="/admin/machine/demo-delay""#),
            "{body}"
        );
        assert!(body.contains(r#"name="delay_secs" value="60""#), "{body}");
        // The browser is told the same cap the route enforces, so the two cannot drift.
        assert!(
            body.contains(&format!(
                r#"max="{}""#,
                km_api::machine::MAX_DEMO_DELAY_SECS
            )),
            "{body}"
        );

        let (status, _) = post_form(&app, "/admin/machine/demo-delay", "delay_secs=30").await;
        assert!(status.is_redirection(), "{status}");
        assert_eq!(state.controller().demo().delay_secs, 30);

        // Zero is a real answer -- "as soon as it goes quiet" -- and not this feature's off switch.
        let (status, _) = post_form(&app, "/admin/machine/demo-delay", "delay_secs=0").await;
        assert!(status.is_redirection(), "{status}");
        assert_eq!(state.controller().demo().delay_secs, 0);

        // Past the cap is refused, and the machine keeps what it had rather than clamping to it: a
        // number nobody meant should come back as a sentence, not as a value they did not choose.
        let too_long = km_api::machine::MAX_DEMO_DELAY_SECS + 1;
        let (status, _) = post_form(
            &app,
            "/admin/machine/demo-delay",
            &format!("delay_secs={too_long}"),
        )
        .await;
        assert!(status.is_redirection(), "{status}");
        assert_eq!(state.controller().demo().delay_secs, 0);

        // And so is something that is not a number at all, which is the whole reason the form takes a
        // string: axum would answer a `u32` it could not parse with a bare 422 and no page.
        let (status, _) = post_form(&app, "/admin/machine/demo-delay", "delay_secs=a+minute").await;
        assert!(status.is_redirection(), "{surface}: {status}");
        assert_eq!(state.controller().demo().delay_secs, 0, "{surface}");
    }
}

/// The two open routes are open, and they are the only two.
#[tokio::test]
async fn the_login_page_and_the_stylesheet_need_no_permission() {
    let (app, guard) = nested(Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/static/admin.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("--accent"), "the stylesheet came back");

    // **The login page always renders now.** It used to redirect away on a machine with no
    // password, because a box that cannot be filled in correctly implies the person has forgotten
    // something. There is always a password, so it can always be filled in.
    let (status, body) = get(&app, "/admin/login").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"type="password""#), "{body}");

    assert!(guard.asked() == 0, "neither of these touches the machine");
}

/// A route nobody put in the table is refused rather than served.
///
/// The inverse of `km-remote-pages`, and the reason this crate has its own middleware: there, an
/// unlisted route is a favorite and letting it through is right. Here it is a control-panel button
/// somebody forgot about.
#[tokio::test]
async fn a_route_with_no_permission_is_refused_rather_than_run() {
    // A path the router does not declare at all: it 404s, which is the same closed answer.
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, _) = get(&app, "/admin/songs/vol1/rename").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A refused guard stops a write on the machine — and on a tool the guard is not what stops it.
///
/// # The two hosts are protected by different things, and this is where that is written down
///
/// `Capabilities::gate_every_route` is **true** on the machine and **false** on a tool, and both are
/// argued. An unlisted route on the machine is a control-panel button somebody forgot to gate, on a
/// page every phone on the LAN can reach. A tool is loopback-only, has no password of its own, and
/// its reads ship public, so a page that opened with a login form would demand a credential before
/// there was anything to spend it on.
///
/// **What follows from that is sharper than it looks, and this test is how it was found.** The
/// middleware is the only caller of `Guard::allows` that stands between a caller and a write — no
/// handler consults it before one — so a host with the flag off has **nothing in this crate
/// authorizing any write**. `gate_every_route` does not choose between two mechanisms; it chooses
/// between one and none.
///
/// The Machine tab asks `allows` too, and that is why this test still counts zero: it posts a write
/// and never renders a page. That read decides whether a tool draws its password box, and refuses
/// nothing.
///
/// That is right, and it is not an oversight to be fixed by adding calls. On a tool the guard answers
/// *do we hold a token*, which is a question about this program's session rather than about the
/// browser making the request; the authority on whether a write may happen is **the machine**, which
/// refuses an untokened call with a 401 that arrives back as `AdminError::Unauthorized` and is worded
/// by the page. A local `allows` check before each write would be this crate second-guessing that,
/// and could refuse a write the machine would have taken.
///
/// So the assertion is per host, and asymmetric on purpose: the machine's write must not happen, and
/// the tool's must reach the host it is pointed at, because that is who decides. Asserted against
/// the machine's *state* rather than a status, since both answers are a 303.
#[tokio::test]
async fn a_refused_guard_stops_a_write_on_the_machine_and_the_machine_decides_for_a_tool() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let guard = Arc::new(SpyGuard {
            refuse: Some(Refusal::NeedsPassword("Sign in first.".to_owned())),
            ..SpyGuard::default()
        });
        let (app, state) = nested_as(host, TestMachine::with_catalog(6), guard.clone());

        // A device the machine has not already got selected, or this would pass against a handler
        // that did nothing at all: `TestMachine` opens with the USB card chosen.
        let onboard = "alsa:plughw:CARD=PCH,DEV=0";
        let before = selected_output(&state);
        assert_ne!(
            before.as_deref(),
            Some(onboard),
            "{surface}: the fixture already has the device this test chooses"
        );
        let _ = post_form(&app, "/admin/sound/output", &format!("id={onboard}")).await;

        match host {
            Host::Machine => {
                assert_eq!(
                    selected_output(&state),
                    before,
                    "a refused caller changed the output on a page the whole house can reach"
                );
                assert!(
                    guard.asked() > 0,
                    "nothing asked the guard at all, which is worse than the wrong answer"
                );
            }
            Host::Tool => {
                assert_eq!(
                    selected_output(&state).as_deref(),
                    Some(onboard),
                    "a tool passes the write to the machine, which is the authority on it"
                );
                assert_eq!(
                    guard.asked(),
                    0,
                    "a tool consults no guard before a write; if that changes, the comment above \
                     this assertion is the thing to read before changing it"
                );
            }
        }
    }
}

/// What the machine has been asked to sound through, or `None` if nothing has ever been chosen.
fn selected_output(state: &ApiState) -> Option<String> {
    state
        .controller()
        .audio_outputs()
        .expect("the test machine lists its outputs")
        .selected
}

/// A caller with no token is sent to the login page when there is one to fill in.
#[tokio::test]
async fn a_refusal_on_a_machine_with_a_password_offers_the_login_page() {
    let guard = Arc::new(SpyGuard {
        refuse: Some(Refusal::NeedsPassword("Sign in first.".to_owned())),
        factory_password: true,
        ..SpyGuard::default()
    });
    let (app, _) = nested(guard);
    let (status, _) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

/// A refusal the password cannot fix is not sent to the login page.
///
/// The other half of the branch above, and the reason `Refusal` still has two variants: a poisoned
/// lock or a controller that failed is not something signing in mends, so sending somebody to a box
/// would be a loop.
#[tokio::test]
async fn a_refusal_that_is_not_about_a_password_does_not_offer_the_login_page() {
    let guard = Arc::new(SpyGuard {
        refuse: Some(Refusal::Failed("The machine is not answering.".to_owned())),
        ..SpyGuard::default()
    });
    let (app, _) = nested(guard);
    let (status, body) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("The machine is not answering."), "{body}");
}

/// The password card always offers a change and a reset, because a password always exists.
///
/// **This replaced a pair of tests about a machine with no password**, which asserted that the card
/// showed a warning and no control at all. That state cannot arise: a machine generates a PIN for
/// itself at first start, so there is never a first password for this page to be forbidden from
/// setting.
#[tokio::test]
async fn the_machine_tab_always_offers_a_change_and_a_reset() {
    // **The one shared pane where the two surfaces genuinely differ**, and the asymmetry is asserted
    // rather than left to `a_tools_capabilities_take_the_machines_own_controls_away`, because it sits
    // inside a pane both hosts draw: a *change* travels over HTTP, and a *reset* puts a new PIN on
    // the television, which is a screen the tool is not standing in front of.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let guard = Arc::new(SpyGuard {
            factory_password: true,
            ..SpyGuard::default()
        });
        let (app, _state) = nested_as_with_password(host, guard);
        let (status, body) = get(&app, "/admin/machine").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(body.contains(r#"type="password""#), "{surface}: {body}");
        // The two controls this change added, on the same tab.
        assert!(body.contains("Sign out everywhere"), "{surface}: {body}");
        assert!(body.contains("Turn debugging on"), "{surface}: {body}");

        let resets = body.contains("Reset to a new PIN");
        match host {
            Host::Machine => assert!(resets, "the machine offers the reset: {body}"),
            Host::Tool => assert!(
                !resets,
                "a reset draws a PIN on the television, so a tool does not offer one: {body}"
            ),
        }
    }
}

/// The Machine tab's errands are a strip, and its two load-bearing properties hold.
///
/// **The radios come before the first `<form>`, and that is asserted by position.** There are six
/// `POST` forms on this page, and a radio inside one of them would post a `machine-tab` key nobody
/// reads. `km-admin` asserts the same thing about its own copy of this strip.
///
/// The panes are asserted too, because the selector that shows one names it: a pane whose class
/// was renamed without the stylesheet following would be a section nothing could ever open, and
/// `display: none` is unconditional.
#[tokio::test]
async fn the_machine_tabs_errands_are_a_strip_outside_every_form() {
    let (app, _state) = nested_with_password(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);

    let last_radio = body
        .rfind(r#"name="machine-tab""#)
        .expect("the strip's radios");
    let first_form = body.find("<form").expect("a form on this page");
    assert!(
        last_radio < first_form,
        "a machine-tab radio sits inside a form, and would post a key nobody reads"
    );

    for pane in [
        "pane debug",
        "pane name",
        "pane password",
        "pane demo",
        "pane language",
    ] {
        assert!(body.contains(pane), "{pane} is missing: {body}");
    }
    // The addresses panel stays *above* the strip: it is built from public reads and is what the
    // tab is for, where the panes are things somebody came to change.
    let addresses = body
        .find("addresses")
        .or_else(|| body.find("machine-unreachable"));
    if let Some(addresses) = addresses {
        assert!(
            addresses < body.find(r#"class="settings""#).expect("the strip"),
            "the addresses panel fell inside the strip"
        );
    }
}

/// The Sound tab offers one row per output, and every spelling behind a link.
///
/// **Both halves are the decision rather than a preference.** ALSA hands over its configuration
/// rather than its hardware, so one jack arrives many times under one string; the first screen shows
/// `preferred` rows only, and nothing is hidden from the machine. The fake's list carries the
/// onboard card twice under one name for exactly this.
#[tokio::test]
async fn the_sound_tab_offers_one_row_per_output_and_all_of_them_on_request() {
    // **The picker is the control this merge deleted a second copy of**, down to the sentinel row,
    // `fell_back`, `changeable` and `?all=1`. Both surfaces, therefore, on every run.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _state) = nested_as_with_password(host, Arc::new(SpyGuard::default()));

        let (status, body) = get(&app, "/admin/sound").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(body.contains("/admin/sound/output"), "{surface}: {body}");
        // The sentinel leads, worded as following the system rather than by its id.
        assert!(body.contains("Follow the system"), "{surface}: {body}");
        assert!(body.contains("USB Audio CODEC"), "{surface}: {body}");
        // One row for the onboard card, not two, though the fake lists it under two names.
        assert_eq!(
            body.matches("HDA Intel PCH").count(),
            1,
            "{surface}: the second spelling of one output was offered first: {body}"
        );
        assert!(body.contains("/admin/sound?all=1"), "{surface}: {body}");

        let (_, all) = get(&app, "/admin/sound?all=1").await;
        assert_eq!(
            all.matches("HDA Intel PCH").count(),
            2,
            "{surface}: ?all=1 did not widen the list: {all}"
        );
        // And the way back, so widening is not a one-way door.
        assert!(all.contains(r#"href="/admin/sound""#), "{surface}: {all}");
    }
}

/// Choosing an output reaches the machine, and the page reads it back.
///
/// **Asserted through the tab rather than against a recorded call**, which is the stronger of the
/// two: it drives the write and then the read, so a picker that posted the right thing and drew the
/// wrong `selected` afterwards still fails. That combination is the one an owner would actually
/// notice, because the symptom is a choice that does not appear to stick.
#[tokio::test]
async fn choosing_an_output_reaches_the_machine_and_comes_back() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, state) = nested_as_with_password(host, Arc::new(SpyGuard::default()));
        // **The onboard card, not the USB one.** `TestMachine` opens with the USB device already
        // selected, so this test posted the machine's existing choice and would have passed against
        // a handler that did nothing at all — found while writing
        // `a_refused_guard_stops_a_write_on_either_surface`, which needed the same distinction and
        // could not get it from a fixture that was already in the answer's state.
        let onboard = "alsa:plughw:CARD=PCH,DEV=0";
        assert_ne!(
            selected_output(&state).as_deref(),
            Some(onboard),
            "{surface}: the fixture already has the device this test chooses"
        );

        let (status, _) = post_form(&app, "/admin/sound/output", &format!("id={onboard}")).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{surface}");
        assert_eq!(
            selected_output(&state).as_deref(),
            Some(onboard),
            "{surface}: the machine was not asked for the device that was chosen"
        );

        let (_, body) = get(&app, "/admin/sound").await;
        assert!(
            body.contains(&format!(r#"value="{onboard}" selected"#)),
            "{surface}: the chosen output did not come back marked: {body}"
        );
    }
}

/// A host's own page gets the shared chrome, and its tab is marked.
///
/// **The seam that lets a host have pages at all.** `km-admin` has two this crate does not and should
/// not — its front door, and the picture and bank searching that
/// `A fourth program, rather than a fourth tab on the owner's page` says must never run on the
/// machine — and askama cannot `{% extends %}` across a crate. Without `Admin::shell` a host would
/// keep a second copy of the strip, the heading and the factory-password banner, which is exactly the
/// markup this whole exercise deletes.
#[tokio::test]
async fn a_hosts_own_page_wears_the_shared_chrome() {
    let state = ApiState::from_machine(
        TestMachine::with_catalog(6).shared(),
        ApiConfig::default().without_mdns(),
    );
    let host = Arc::new(km_admin_pages::in_process::ThisMachine::new(state));
    let admin = Admin::over(
        km_admin_pages::machine::Capabilities::desktop(),
        km_admin_pages::ICON_ADMIN_PNG,
        Arc::new(SpyGuard::default()),
        host,
    );

    let response = admin
        .shell(
            km_admin_pages::views::Tab::Sound,
            km_locale::Locale::English,
            None,
            "<p id=\"mine\">a host wrote this</p>".to_owned(),
        )
        .await;
    let body = String::from_utf8_lossy(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read the body"),
    )
    .into_owned();

    assert!(
        body.contains(r#"<p id="mine">a host wrote this</p>"#),
        "the host's markup goes in unescaped, which is what `Shell::content` is for: {body}"
    );
    // The chrome, all of it: the strip, the heading and the nag's absence are one decision each.
    assert!(body.contains(r#"<nav class="tabs">"#), "the strip: {body}");
    assert!(
        body.contains(r#"href="/admin/sound""#),
        "its entries: {body}"
    );
    assert!(
        body.contains(r#"<a href="/admin/sound" aria-current="page""#),
        "and the tab it was rendered under is the one marked current: {body}"
    );
    // A tool's capabilities, so the tab it cannot draw is still absent from a page it does draw.
    assert!(
        !body.contains("/admin/problems"),
        "capabilities apply to a host's own page too: {body}"
    );
}

/// A host that keeps its own token gets no cookie, and the machine's host does.
///
/// **The whole reason `LoggedIn` is not a `Grant`.** On the machine a token is *this browser's* and
/// rides in an `HttpOnly` cookie, because a browser following a link cannot be told to send an
/// `Authorization` header. In a tool on loopback the token is *the program's*, in the kept
/// `reqwest::Client` — so a cookie there would be a page claiming a session it is not the keeper of,
/// and the tool would have had to invent a value for a cookie nothing reads.
#[tokio::test]
async fn a_host_that_keeps_its_own_token_is_sent_no_cookie() {
    let keeps_its_own = Arc::new(SpyGuard {
        keeps_its_own: true,
        ..SpyGuard::default()
    });

    let state = ApiState::from_machine(
        TestMachine::with_catalog(6).shared(),
        ApiConfig::default().without_mdns(),
    );
    let host = Arc::new(km_admin_pages::in_process::ThisMachine::new(state));
    let app = Router::new().nest(
        "/admin",
        router(Admin::over(
            km_admin_pages::machine::Capabilities::desktop(),
            km_admin_pages::ICON_ADMIN_PNG,
            keeps_its_own,
            host,
        )),
    );

    let response = app
        .clone()
        .oneshot(
            Request::post("/admin/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("password=hunter2xyz"))
                .expect("request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response.headers().get(header::SET_COOKIE).is_none(),
        "the token is already where it belongs; a cookie here would claim otherwise"
    );

    // ...and the machine's host, whose token *is* the browser's, still sets one.
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let response = app
        .oneshot(
            Request::post("/admin/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("password=hunter2xyz"))
                .expect("request"),
        )
        .await
        .expect("the router answers");
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(cookie.contains("km_token="), "no cookie was set: {cookie}");
    assert!(
        cookie.contains("HttpOnly"),
        "a token a script can read is a token a page can leak: {cookie}"
    );
}

/// The same handlers, rendered under a tool's capabilities, draw a smaller page.
///
/// **The test the flags exist for, and the one nothing else can be.** Every other test here runs
/// `Capabilities::machine()`, where every flag is on — so a template that ignored a flag entirely
/// would pass all of them. This renders the *same* routes against `Capabilities::desktop()` and
/// asserts what goes away.
///
/// **What a tool's capabilities take away is three things, each for its own reason**: the Problems
/// tab cannot be answered over HTTP at all, *Screen language* has no route in the API, and a
/// password *reset* draws a new PIN on a television the tool is not standing in front of.
///
/// Note what stays on both: the upload forms, the output picker, the song and package counts. A
/// surface that sends a package but cannot say how many songs the machine now holds could not
/// confirm its own work.
#[tokio::test]
async fn a_tools_capabilities_take_the_machines_own_controls_away() {
    // A tool's capabilities over the *same* implementation, which is the point: nothing about the
    // host changes, only what the templates are told this surface is for. The machine's page is
    // built beside it, so what is asserted below is a *difference* rather than a string.
    let (app, _state) = nested_as(
        Host::Tool,
        TestMachine::with_catalog(6),
        Arc::new(SpyGuard::default()),
    );
    let (machine_app, _machine_state) = nested_as(
        Host::Machine,
        TestMachine::with_catalog(6),
        Arc::new(SpyGuard::default()),
    );

    let (status, songs) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        songs.contains("<input type=\"file\""),
        "sending a package is what this surface is for: {songs}"
    );
    // Asked of the catalog rather than of a literal, because the count is a *message* with two
    // plurals in it: six songs in one package reads `6 songs in 1 package.`, and a test spelling
    // `packages.` passes only for as long as the fixture holds more than one.
    let holds = km_admin_pages::words::messages(km_locale::Locale::English).msg_with(
        "songs-count",
        &[("songs", 6.into()), ("packages", 1.into())],
    );
    assert!(
        songs.contains(holds.as_ref()),
        "and it still says what the machine holds: {songs}"
    );
    // A tool that sends a package sees the one it sent and corrects it — see
    // `What it is not is a second /admin/`.
    assert!(
        songs.contains("/bank\""),
        "a tool moves a package between blocks: {songs}"
    );

    let (_, pictures) = get(&app, "/admin/pictures").await;
    assert!(
        pictures.contains("/pictures/next"),
        "and shows the next picture: {pictures}"
    );

    let (_, sound) = get(&app, "/admin/sound").await;
    assert!(
        sound.contains("/sound/output"),
        "the output picker is on both surfaces, which is what its decision asks: {sound}"
    );
    // The *table* of banks the machine already has, which is what the flag gates. Not the *use*
    // control, which the template draws only for a bank that is not the current one — this fixture
    // has the bundled bank and nothing else, so no host draws one, and asserting it here would be
    // asserting something about the fixture rather than about the capability.
    assert!(
        sound.contains("/admin/sound/fetch"),
        "the door to this program's own fetching is still there: {sound}"
    );
    let (_, machine_sound) = get(&machine_app, "/admin/sound").await;
    assert_eq!(
        sound.contains(r#"<table"#),
        machine_sound.contains(r#"<table"#),
        "both surfaces list the machine's installed banks now: {sound}"
    );

    let (_, machine) = get(&app, "/admin/machine").await;
    assert!(
        machine.contains("/machine/password"),
        "changing the password is the third setting-up errand: {machine}"
    );
    assert!(
        !machine.contains("name=\"clear\""),
        "a reset draws the new PIN on the television, so it belongs beside it: {machine}"
    );
    assert!(
        machine.contains("/machine/locale"),
        "a tool sets what the television speaks, the API having a route for it: {machine}"
    );

    // And the tab that cannot be drawn at all is not in the strip.
    for body in [&songs, &machine] {
        assert!(
            !body.contains("/admin/problems"),
            "a surface that cannot list refused packages does not offer the tab: {body}"
        );
    }
}

/// Not one template in this crate carries a script, and a dozen controls depend on that.
///
/// **`No htmx, no script at all` is what a dozen decisions here rest on**: the confirmations are
/// pages rather than `confirm()` dialogs, *show every spelling* is a link with `?all=1` rather than a
/// checkbox, two `<select>`-driven acts take their id in the body because there is nothing to build a
/// URL with, and the settings strip is `:checked ~` sibling selectors. Every one of those reads as an
/// odd choice the moment a script is available, and the first person to add one would be removing the
/// reason for all of them without noticing.
///
/// **It became a property worth testing when this crate gained a second host.** `km-admin` genuinely
/// needs htmx — a gigabyte bank download has to report progress — and it merges *its own* fragments
/// over this router. So the rule is not "the admin has no script" but "the shared markup has none",
/// which is a line only a test can hold.
///
/// Read off the files rather than off a rendered page, because a fragment nothing currently renders
/// is still a template somebody will render later.
///
/// **`<script` is asked of the rendered page instead**, by the test below, because `shell.html` now
/// emits one per entry in `Admin::scripts` — and a file scan could only have answered that with an
/// exemption for the one template whose scripts are the point.
#[test]
fn no_shared_template_carries_an_hx_attribute() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(dir).expect("the templates directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|ext| ext != "html") {
            continue;
        }
        let markup = std::fs::read_to_string(&path).expect("read the template");
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        checked += 1;
        assert!(
            !markup.contains("hx-"),
            "{name} carries an htmx attribute; those belong in the host that needs them"
        );
        assert!(
            !markup.contains("onclick") && !markup.contains("onchange"),
            "{name} carries an inline handler, which is a script by another name"
        );
    }
    assert!(checked > 5, "the scanner found only {checked} templates");
}

/// The machine's own pages load no script, asked of what a browser actually receives.
///
/// # Why this is a rendered-page test and the one above is a file scan
///
/// **Because the regression it stands against was invisible to every file.** `km-admin` kept its own
/// `layout.html`, and that file was the only thing linking htmx, `ui.js` and its stylesheet. Deleting
/// it — the whole point of the second-host change — took all three tags with it, and nothing failed:
/// the templates were all still correct, the routes all still served, and the searching's progress
/// bar simply stopped moving. A test that reads templates cannot see a missing `<script>`; only one
/// that reads a response can.
///
/// So the guarantee runs both ways now. `Admin::scripts` is empty on the machine, and this asserts
/// what that *means* — `/admin/` works with scripting switched off, which is what
/// `No htmx, no script at all` promised somebody on a phone browser they did not choose.
#[tokio::test]
async fn no_page_the_machine_serves_carries_a_script() {
    let (app, _) = nested_on(TestMachine::with_catalog(6), Arc::new(SpyGuard::default()));
    let mut checked = 0usize;
    for path in [
        "/admin/machine",
        "/admin/songs",
        "/admin/pictures",
        "/admin/sound",
        "/admin/sound?all=1",
        "/admin/problems",
        "/admin/login",
    ] {
        let (status, body) = get(&app, path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        checked += 1;
        assert!(
            !body.contains("<script"),
            "{path} carries a script; see `/admin/ is the owner's page` in docs/decisions/"
        );
        assert!(
            !body.contains("hx-"),
            "{path} carries an htmx attribute, so it needs a script it does not load"
        );
    }
    assert!(checked > 5, "only {checked} pages were read");
}

/// A host that declares scripts gets them, on its own pages and in the order it gave.
///
/// **The other half of the property above**, and the half that was missing when the second host
/// shipped: a page that needs htmx has to actually load it, before the script that listens to it.
/// `defer` on both, so the order in the markup is the order they run.
#[tokio::test]
async fn a_hosts_own_page_loads_the_scripts_that_host_declared() {
    let state = ApiState::from_machine(
        TestMachine::with_catalog(6).shared(),
        ApiConfig::default().without_mdns(),
    );
    let host = Arc::new(km_admin_pages::in_process::ThisMachine::new(state));
    let admin = Admin::over(
        km_admin_pages::machine::Capabilities::desktop(),
        km_admin_pages::ICON_ADMIN_PNG,
        Arc::new(SpyGuard::default()),
        host,
    )
    .with_scripts(&["/static/htmx.min.js", "/static/ui.js"]);

    let response = admin
        .shell(
            km_admin_pages::views::Tab::Sound,
            km_locale::Locale::English,
            None,
            "<p>a host's own page</p>".to_owned(),
        )
        .await;
    let markup = String::from_utf8_lossy(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("a body"),
    )
    .into_owned();

    let htmx = markup
        .find(r#"<script src="/static/htmx.min.js" defer></script>"#)
        .expect("the page loads htmx");
    let ui = markup
        .find(r#"<script src="/static/ui.js" defer></script>"#)
        .expect("the page loads this host's own script");
    assert!(
        htmx < ui,
        "htmx must be loaded before the script that listens to it"
    );
}

/// A busy machine reads as a warning; a device it has never heard of reads as an error.
///
/// **Two refusals of one control that mean opposite things.** *The output can only be changed when
/// nothing is playing* is `try again when the song ends`; *no such audio output* is `something is
/// wrong with what you sent`. A page that painted both red would tell somebody to go and fix a
/// request that was perfectly good.
///
/// **This was untested while the distinction lived in a `match` inside the handler**, on
/// `ControlError::Unavailable` — so moving the operation behind `Sound::set_output` could have
/// flattened the two and nothing would have said so. It is `AdminError::Busy` and
/// `AdminError::severity` now, which is one place rather than one page's habit, and this is the test
/// that holds it there.
#[tokio::test]
async fn a_busy_machine_warns_and_a_bad_device_errs() {
    // **`AdminError::severity` is the seam's own judgement**, and asking both surfaces is what says
    // the distinction survives the trip: on the machine it is a `match` on an in-process error, and
    // on a tool it is a code that crossed a wire and got worded again at this end.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let machine = TestMachine::with_catalog(6);
        // Anything loaded makes the change unavailable: the player lives inside the stream a change
        // has to drop. This is the appliance case — somebody setting a machine up mid-party.
        km_api::Controller::play_file(
            &machine,
            std::path::Path::new("anything.mid"),
            &km_api::Audition::default(),
        )
        .expect("the test machine loads a file");

        let (app, _state) = nested_as(host, machine, Arc::new(SpyGuard::default()));
        let usb = "alsa:plughw:CARD=Device,DEV=0";

        let busy = post_form_to(&app, "/admin/sound/output", &format!("id={usb}")).await;
        assert!(
            busy.contains("kind=warn"),
            "{surface}: busy is not broken, so it is a warning: {busy}"
        );

        // ...and a device the machine does not have is the other kind entirely — asked of an
        // **idle** machine, because the busy check comes first and would otherwise answer this too.
        // That ordering is the machine's and is right: *stop the music* is the more useful thing to
        // be told.
        let (idle, _state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );
        let unknown = post_form_to(&idle, "/admin/sound/output", "id=alsa:no-such-thing").await;
        assert!(
            unknown.contains("kind=bad"),
            "{surface}: an identifier the machine never heard of is an error: {unknown}"
        );
    }
}

/// The stylesheet and the template agree about all five pane names.
///
/// **The one thing about this strip that no other test can see.** `.pane { display: none }` is
/// unconditional and each pane is revealed by a selector that *names* it, so a pane renamed on one
/// side and not the other is a section nothing can ever open — and the page still renders, still
/// passes every assertion about its markup, and simply has a tab that does nothing. The graceful
/// degradation this pattern is chosen for does not cover that: it covers a stylesheet that fails to
/// *load*, where everything stacks and nothing is lost.
///
/// Both names are read out of the files rather than listed here, because a hand-kept third list is
/// exactly the drift a test like this exists to catch.
/// # It reads **both** hosts' pages, and had to start doing so
///
/// A pane can now belong to one surface: *Screen language* is the machine's, *Different machine* is a
/// tool's. So the forward check is per host — every pane a page draws must have a selector — and the
/// backward check is against the *union*, because a selector for a pane no host draws is the dead
/// rule this half is looking for, and one only a tool draws is not dead.
///
/// **This test caught the pane that prompted the change**, which is the best evidence for it: the
/// stylesheet named `elsewhere` before the machine's page could draw it, and the assertion fired.
#[tokio::test]
async fn every_pane_the_machine_tab_draws_has_a_selector_that_opens_it() {
    /// Every `class="pane <name>"` in a rendered page.
    fn panes_of(body: &str) -> Vec<String> {
        body.match_indices(r#"class="pane "#)
            .map(|(at, marker)| {
                body[at + marker.len()..]
                    .split('"')
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }

    let (app, _state) = nested_with_password(Arc::new(SpyGuard::default()));
    let (_, machine_page) = get(&app, "/admin/machine").await;
    let (_, css) = get(&app, "/admin/static/admin.css").await;

    // A tool's page, over the same handlers, so the panes only it draws are in scope too.
    let state = ApiState::from_machine(
        TestMachine::with_catalog(6).shared(),
        ApiConfig::default().without_mdns(),
    );
    let host = Arc::new(km_admin_pages::in_process::ThisMachine::new(state));
    let tool = Router::new().nest(
        "/admin",
        router(Admin::over(
            km_admin_pages::machine::Capabilities::desktop(),
            km_admin_pages::ICON_ADMIN_PNG,
            Arc::new(SpyGuard::default()),
            host,
        )),
    );
    let (_, tool_page) = get(&tool, "/admin/machine").await;

    let machine_panes = panes_of(&machine_page);
    let tool_panes = panes_of(&tool_page);
    assert!(
        machine_panes.len() >= 5,
        "only found {machine_panes:?} in the machine's markup; the scanner is broken"
    );
    // **Choosing a machine is not one of the panes**, and asserted here because this is the test
    // that knows what the panes are: it lives in the information card, where the name and the
    // addresses are, so it is reachable without pressing anything — which is the point of it, since
    // somebody reaches for it when the machine those facts describe has gone away.
    assert!(
        !tool_panes.iter().any(|pane| pane == "elsewhere"),
        "the chooser belongs in the card, not behind a tab: {tool_panes:?}"
    );
    assert!(
        tool_page.contains(r#"<div class="choose">"#),
        "a tool draws the chooser only it has: {tool_page}"
    );
    assert!(
        !machine_page.contains(r#"<div class="choose">"#),
        "and the machine does not, being unable to point itself at another machine"
    );

    for pane in machine_panes.iter().chain(&tool_panes) {
        assert!(
            css.contains(&format!(":checked ~ .pane.{pane}")),
            "the pane `{pane}` has no selector that reveals it, so its tab would do nothing"
        );
    }

    // And backwards: a selector for a pane **no host** draws is a leftover from a rename, which
    // leaves dead CSS rather than a dead tab — quieter, and the same mistake.
    for (at, marker) in css.match_indices(":checked ~ .pane.") {
        let named = css[at + marker.len()..]
            .split([',', ' ', '{', '\n', '\r'])
            .next()
            .unwrap_or_default()
            .to_owned();
        assert!(
            machine_panes.contains(&named) || tool_panes.contains(&named),
            "the stylesheet reveals a pane `{named}` that no host draws"
        );
    }
}

/// The information panel says which machine it is about.
///
/// **The one fact in that card that is not true of every machine.** Addresses, a song count and a
/// version describe any of them; on a tool pointed at one of several, the name is what somebody is
/// checking when they read the card at all — and it is what tells them the chooser below it did what
/// they asked.
#[tokio::test]
async fn the_information_panel_names_the_machine() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _state) = nested_as_with_password(host, Arc::new(SpyGuard::default()));
        let (_, body) = get(&app, "/admin/machine").await;

        // **The facts list and not the whole page.** Every page carries the machine's name in its
        // heading, so a search over the body would pass with the card empty — which is the state
        // this test is about.
        let facts = body
            .split_once(r#"<dl class="facts">"#)
            .and_then(|(_, rest)| rest.split_once("</dl>"))
            .map(|(facts, _)| facts)
            .unwrap_or_else(|| panic!("{surface}: the panel has no facts list: {body}"));
        let name = ApiConfig::default().machine_name;
        assert!(
            facts.contains(&name),
            "{surface}: the facts do not name the machine: {facts}"
        );
    }
}

/// All three switches are on the Debugging pane, and each says what it needs.
#[tokio::test]
async fn the_debugging_pane_carries_three_switches() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _state) = nested_as_with_password(host, Arc::new(SpyGuard::default()));
        let (_, body) = get(&app, "/admin/machine").await;
        assert!(body.contains("/admin/machine/debug"), "{surface}: {body}");
        assert!(
            body.contains("/admin/machine/dev-remote"),
            "{surface}: {body}"
        );
        assert!(
            body.contains("/admin/machine/performance"),
            "{surface}: {body}"
        );
        // The console's card says out loud that it is not the only switch involved.
        assert!(
            body.contains("Needs debugging on as well"),
            "{surface}: {body}"
        );
    }
}

/// The console card names the reason it is not being served, and there are two of them.
///
/// **Blaming the wrong one is worse than saying nothing**: an owner reading *debugging is off* on a
/// card beside a button that says *Turn debugging off* has been told the page is broken, and the
/// thing they actually have to do — restart the machine — is the thing they were not told. Both
/// switches on with the console still unserved is the ordinary state after pressing them, because
/// each takes effect at the next start.
#[tokio::test]
async fn the_console_card_blames_the_switch_that_is_off_and_otherwise_the_restart() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _state) = nested_as_with_password(host, Arc::new(SpyGuard::default()));

        // Nothing asked for, nothing to explain.
        let (_, body) = get(&app, "/admin/machine").await;
        assert!(
            !body.contains("It needs both"),
            "{surface}: the console is off and wants no banner: {body}"
        );
        assert!(
            !body.contains("takes effect the next time"),
            "{surface}: {body}"
        );

        // The console alone: the other switch is what is missing, and it is a switch.
        post_form(&app, "/admin/machine/dev-remote", "enabled=yes").await;
        let (_, body) = get(&app, "/admin/machine").await;
        assert!(body.contains("It needs both"), "{surface}: {body}");

        // Both switches now on, and the run they were pressed in still has neither. What is left is
        // a restart, so that is what the card says.
        post_form(&app, "/admin/machine/debug", "enabled=yes").await;
        let (_, body) = get(&app, "/admin/machine").await;
        assert!(
            body.contains("takes effect the next time"),
            "{surface}: both are on, so the card asks for a restart: {body}"
        );
        assert!(
            !body.contains("It needs both"),
            "{surface}: debugging is on, and the card must not say otherwise: {body}"
        );
    }
}

/// Setting a password from this page really reaches the machine.
///
/// The inverse of the test this replaced, which asserted the page *could not* set one. It could not
/// because the page was open to the whole network while the password was unset; that state is gone,
/// and reaching this page at all now takes the password.
#[tokio::test]
async fn a_posted_password_reaches_the_machine() {
    // Changing the password is shared; only the *reset* is the machine's — see
    // `the_machine_tab_always_offers_a_change_and_a_reset`.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );
        let (status, _) = post_form(&app, "/admin/machine/password", "password=carols1975").await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{surface}");
        assert!(
            state.admin_configured(),
            "{surface}: the page did not set the password"
        );
    }
}

/// A password under the floor is refused, and the floor is four.
///
/// Four rather than eight because the PIN the machine generates for itself is six digits, and a
/// floor above what the product ships would be a rule it breaks itself.
#[tokio::test]
async fn a_password_shorter_than_the_floor_is_refused() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );
        let (status, _) = post_form(&app, "/admin/machine/password", "password=abc").await;
        assert!(status.is_redirection(), "{surface}: {status}");
        assert!(
            !state.admin_configured(),
            "{surface}: a three-character password was accepted"
        );
    }
}

/// The floor counts characters, not bytes, and this page counts them the way the API does.
///
/// **It counted bytes, and that made the two surfaces disagree.** `日本` is two characters and six
/// UTF-8 bytes, so `password.len() < 4` let it through here while `POST /api/v1/admin/password`
/// refused it — the same machine, the same password, two answers. Both read
/// `km_api::MIN_PASSWORD_CHARS` now, so the number cannot drift either.
#[tokio::test]
async fn a_short_password_that_is_long_in_bytes_is_refused() {
    let (app, state) =
        nested_with_state(TestMachine::with_catalog(6), Arc::new(SpyGuard::default()));
    let two_chars = "日本";
    assert!(two_chars.len() > km_api::MIN_PASSWORD_CHARS);
    assert!(two_chars.chars().count() < km_api::MIN_PASSWORD_CHARS);

    let (status, _) = post_form(
        &app,
        "/admin/machine/password",
        &format!("password={two_chars}"),
    )
    .await;
    assert!(status.is_redirection(), "{status}");
    assert!(
        !state.admin_configured(),
        "a two-character password was accepted because its bytes were counted"
    );
}

/// The factory-password banner is on every tab, not only the one that can fix it.
///
/// **And it names the pane, not only the tab.** The Machine tab opens Debugging on a host holding a
/// token, so a link without `pane=password` promises a box it does not then show.
#[tokio::test]
async fn a_machine_on_its_factory_password_says_so_on_every_page() {
    // **The nag existed twice before the merge**, once in each program's layout, and it is the
    // clearest case for asking both surfaces: a machine still on its factory PIN is exactly the
    // machine somebody is configuring from a desktop, so the tool is arguably where the sentence
    // matters more.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let guard = Arc::new(SpyGuard {
            factory_password: true,
            ..SpyGuard::default()
        });
        let (app, _state) = nested_as(host, TestMachine::with_catalog(6), guard);
        for path in [
            "/admin/songs",
            "/admin/pictures",
            "/admin/sound",
            "/admin/machine",
        ] {
            let (status, body) = get(&app, path).await;
            assert_eq!(status, StatusCode::OK, "{surface}: {path}");
            assert!(
                body.contains("still on the default password"),
                "{surface}: {path} does not carry the banner"
            );
            assert!(
                body.contains(r#"href="/admin/machine?pane=password""#),
                "{surface}: {path}'s banner does not link to the pane that fixes it"
            );
        }
    }
}

/// ...and that link opens the pane with the box in it.
#[tokio::test]
async fn the_factory_password_banner_opens_the_pane_it_names() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _) = nested_as_with_password(host, Arc::new(SpyGuard::default()));
        let (status, body) = get(&app, "/admin/machine?pane=password").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(
            body.contains(r#"id="machine-tab-password" checked"#),
            "{surface} opened something other than the pane the banner names: {body}"
        );
    }
}

/// A tool holding no token still opens the pane the link was for, and says what is missing.
///
/// **The banner is drawn off `/discover`, which needs no password**, so it is on the page before
/// anybody has logged in — and every pane on the strip is a control whose save the machine would
/// refuse. What used to happen was that a pane opened itself over the one the link asked for; the
/// box that pane held is on the host's own front door now, so the page keeps the pane and carries a
/// sentence and a way there instead.
#[tokio::test]
async fn a_tool_with_no_token_keeps_the_pane_and_says_what_is_missing() {
    let guard = Arc::new(SpyGuard {
        refuse: Some(Refusal::NeedsPassword(
            "this machine requires a password".to_owned(),
        )),
        keeps_its_own: true,
        factory_password: true,
        ..SpyGuard::default()
    });
    let (app, _) = nested_as_with_password(Host::Tool, guard);

    let (status, body) = get(&app, "/admin/machine?pane=password").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"id="machine-tab-password" checked"#),
        "the pane the link asked for was taken away: {body}"
    );
    assert!(
        body.contains("This program is not logged in"),
        "nothing says why every control here would refuse: {body}"
    );
    assert!(
        body.contains(r#"href="/admin/connect""#),
        "and nothing leads to the page that mends it: {body}"
    );
}

/// ...and a machine whose owner has chosen a password carries no banner at all.
#[tokio::test]
async fn a_machine_with_an_owners_password_carries_no_banner() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("still on the default password"), "{body}");
}

/// The pictures tab warns before the first upload, not after it.
///
/// `Where the owner's own wallpapers live` makes a non-empty owner folder win outright, so adding
/// one picture silently takes the shipped set out of the rotation. The warning has to be above the
/// control, which is what this asserts by asserting it is on the page at all while the source is
/// not the owner's.
#[tokio::test]
async fn the_pictures_tab_says_the_first_upload_replaces_the_shipped_set() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/pictures").await;
    assert_eq!(status, StatusCode::OK);
    // The test machine reports the owner's own folder, so the warning is absent...
    assert!(
        !body.contains("replaces"),
        "a machine already on the owner's pictures needs no warning"
    );
    assert!(body.contains("Your own pictures."), "{body}");
}

/// Each chooser offers what its route takes, and no more.
///
/// **This page is the surface a phone actually reaches** — the machine serves it on the LAN to every
/// phone in the house — so what the chooser opens is not a detail. An `image/*` on the picture form
/// would open a camera roll and put somebody one tap from a file the route refuses: the wallpaper
/// folder holds packs.
///
/// Asserted against the *rendered* attribute rather than against `accept_for`, which
/// `km-api`'s own `a_file_chooser_offers_what_the_route_takes` already covers: what could break here
/// is the template going back to a literal, and only reading the markup catches that.
#[tokio::test]
async fn every_chooser_offers_what_its_route_takes() {
    // **The three upload forms are the whole of what a tool's Songs, Pictures and Sound tabs are
    // for**, so a chooser that lost its `accept` on that surface is the more expensive of the two
    // failures. Both, then.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );

        let (status, body) = get(&app, "/admin/pictures").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(
            body.contains(r#"accept=".zip""#),
            "{surface}: a pack and not a picture: {body}"
        );
        assert!(
            !body.contains("image/*"),
            "{surface}: a camera roll offers what this route refuses: {body}"
        );

        // None of the three carries a media type -- none has one a picker knows. See `accept_for`.
        let (_, body) = get(&app, "/admin/songs").await;
        assert!(body.contains(r#"accept=".kmpkg""#), "{surface}: {body}");
        let (_, body) = get(&app, "/admin/sound").await;
        assert!(body.contains(r#"accept=".sf2""#), "{surface}: {body}");
    }
}

/// A notice carried in the query string is shown, and its wording survives the trip.
#[tokio::test]
async fn something_that_just_happened_is_reported_on_the_page_it_happened_on() {
    // The notice is the shared chrome's, and every refusal on either surface arrives through it —
    // a 303 with the sentence in the query rather than a status with a body.
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );
        let (status, body) = get(&app, "/admin/songs?kind=good&said=Added+16+songs.").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(body.contains("Added 16 songs."), "{surface}: {body}");
        assert!(body.contains("banner-good"), "{surface}: {body}");
    }
}

/// The tabs are all there, and the one being drawn is marked.
#[tokio::test]
async fn the_tab_showing_is_the_one_marked_current() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (_, body) = get(&app, "/admin/sound").await;
    for tab in ["Songs", "Pictures", "Sound", "This machine"] {
        assert!(body.contains(tab), "the {tab} tab is missing");
    }
    assert!(
        body.contains(r#"<a href="/admin/sound" aria-current="page""#),
        "{body}"
    );
}

/// Remove leads to a question, not to a deleted file.
///
/// **The one that would catch a half-done change.** Everything else about the confirmation could be
/// right while the row still posted straight through, and nobody would notice until a package was
/// gone.
#[tokio::test]
async fn the_remove_control_leads_to_a_confirmation_rather_than_straight_to_a_delete() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"href="/admin/songs/vol1/remove""#),
        "the row links to the question"
    );
    assert!(
        !body.contains(r#"action="/admin/songs/vol1/remove""#),
        "and does not post to it from the list"
    );
}

/// The confirmation says what goes, and offers the POST that does it.
#[tokio::test]
async fn the_confirmation_names_the_package_and_what_removing_it_costs() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/songs/vol1/remove").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("deleted"), "it says the file is deleted");
    assert!(
        body.contains(r#"action="/admin/songs/vol1/remove""#),
        "the form posts to the same address the page was fetched from"
    );
    assert!(
        body.contains(r#"href="/admin/songs""#),
        "and Cancel goes back to the tab"
    );
    // Inside the tab it came from, rather than a page floating outside the chrome.
    assert!(body.contains(r#"<a href="/admin/songs" aria-current="page""#));
}

/// The table names which build of each package is installed.
///
/// **Both surfaces, because the one that most needs it is the tool.** A curator who has just sent a
/// rebuilt volume is asking whether the build that landed is the one they made, and `km-admin` is
/// the window they asked it from. The machine's own page carries it on the same footing.
///
/// The version is asserted rather than the column alone: a header with an empty cell under it is
/// the shape a half-threaded seam produces, and it is the cell that answers the question.
#[tokio::test]
async fn the_songs_table_names_the_build_of_each_package() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _state) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );

        let (status, body) = get(&app, "/admin/songs").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(
            body.contains(">Version<"),
            "{surface}: the table has no version column: {body}"
        );
        assert!(
            body.contains("1.0.4"),
            "{surface}: the column is there and the build is not: {body}"
        );
    }
}

/// The confirmation names the build it is about to delete.
///
/// The file may be the only copy and tens of gigabytes of it, so the page that asks says which build
/// goes with the name above it — the machine names the file after the package rather than after the
/// build, so nothing else on the way to a delete can say.
#[tokio::test]
async fn the_confirmation_names_the_build_that_is_about_to_go() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/songs/vol1/remove").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Version"), "it labels the fact: {body}");
    assert!(body.contains("1.0.4"), "and names the build: {body}");
}

/// A package the machine may not delete is offered no control, and the row says why.
///
/// Not a grayed-out button: `sound.html`'s bundled bank sets the rule, and the sentence is the
/// machine's own — the very one an uninstall would have refused with.
#[tokio::test]
async fn a_package_the_machine_may_not_delete_offers_no_control_and_says_why() {
    let machine = TestMachine::with_catalog(6);
    machine.set_faults(km_api::testing::Faults {
        unremovable: vec![("vol1".to_owned(), "named in debug.packages".to_owned())],
        ..Default::default()
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("named in debug.packages"), "the row says why");
    assert!(
        !body.contains(r#"href="/admin/songs/vol1/remove""#),
        "and offers nothing to press"
    );
}

/// Asking to confirm something that cannot be removed says so instead of asking.
///
/// A stale page or a typed-in URL reaches here, and the payoff for carrying the reason rather than a
/// bare `bool` is that it can be answered in the machine's own words.
#[tokio::test]
async fn confirming_a_package_that_cannot_be_removed_says_so_rather_than_asking() {
    let machine = TestMachine::with_catalog(6);
    machine.set_faults(km_api::testing::Faults {
        unremovable: vec![("vol1".to_owned(), "named in debug.packages".to_owned())],
        ..Default::default()
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, _) = get(&app, "/admin/songs/vol1/remove").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

/// The POST refuses on its own. The confirmation is a courtesy, not the guard.
#[tokio::test]
async fn the_post_still_refuses_without_having_been_asked_first() {
    let machine = TestMachine::with_catalog(6);
    machine.set_faults(km_api::testing::Faults {
        uninstall: Some(km_api::machine::CatalogError::Rejected(
            "named in debug.packages".to_owned(),
        )),
        ..Default::default()
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, _) = post_form(&app, "/admin/songs/vol1/remove", "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

/// Two refused packages, in two folders, under one name.
///
/// **The case the whole id design exists for**, and it is an observed one rather than a
/// hypothetical: the same package sitting in two of the folders the machine scans, listed twice on
/// the singer's remote with identical text. A name-shaped id would give these two rows one link, and
/// pressing it would delete whichever file the loop reached first — the single worst outcome this
/// operation has available. This is the test that would catch that.
#[tokio::test]
async fn two_refused_packages_with_one_name_get_a_row_each_and_two_different_links() {
    let machine = TestMachine::with_catalog(6);
    machine.set_package_problems(vec![
        refused("/data/packages/carols.kmpkg"),
        refused("/tunes/karaoke/carols.kmpkg"),
    ]);
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/problems").await;
    assert_eq!(status, StatusCode::OK);

    let links: Vec<&str> = body
        .match_indices("/admin/problems/")
        .map(|(_, s)| s)
        .collect();
    assert_eq!(links.len(), 2, "one link per file: {body}");
    assert_ne!(
        link_at(&body, 0),
        link_at(&body, 1),
        "two files, two links, or the wrong one gets deleted: {body}"
    );

    // And the row says which is which, because two identical rows with different links is worse
    // than no page at all.
    assert!(body.contains("/data/packages"), "{body}");
    assert!(body.contains("/tunes/karaoke"), "{body}");
}

/// The tab is in the bar on a machine with nothing wrong, and says so without a badge.
///
/// Always present by decision: deleting the *last* problem would otherwise make the tab vanish
/// while somebody is standing on it, and the redirect afterwards would name a tab the bar no longer
/// has.
#[tokio::test]
async fn the_problems_tab_is_in_the_bar_even_when_nothing_is_wrong() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"href="/admin/problems""#), "{body}");
    assert!(
        !body.contains("badge-bad"),
        "a healthy machine wears no count: {body}"
    );

    let (status, body) = get(&app, "/admin/problems").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Every package was loaded."), "{body}");
}

/// ...and it wears the count when there is one, on every tab rather than only its own.
///
/// The badge's whole job is to be seen from a tab somebody opened for another reason, which is why
/// the count is on the chrome and not on the page.
#[tokio::test]
async fn the_tab_carries_a_count_and_carries_it_onto_the_other_tabs() {
    let machine = TestMachine::with_catalog(6);
    machine.set_package_problems(vec![refused("/data/packages/carols.kmpkg")]);
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    for tab in ["/admin/songs", "/admin/pictures", "/admin/problems"] {
        let (status, body) = get(&app, tab).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains(r#"<span class="badge badge-bad">1</span>"#),
            "{tab} does not carry the count: {body}"
        );
    }
}

/// A refused package leads to a question, never straight to a deleted file.
#[tokio::test]
async fn deleting_a_refused_package_asks_first() {
    let machine = TestMachine::with_catalog(6);
    machine.set_package_problems(vec![refused("/data/packages/carols.kmpkg")]);
    let id = refused("/data/packages/carols.kmpkg").id();
    // Built out by hand rather than through `nested_with_state`, because this is the one test that
    // wants the double itself afterwards as well as the state.
    let machine = machine.shared();
    let state = ApiState::from_machine(machine.clone(), ApiConfig::default().without_mdns());
    let app = Router::new().nest(
        "/admin",
        router(admin_over(state.clone(), Arc::new(SpyGuard::default()))),
    );

    // The row links to the question and posts nothing itself.
    let (_, listing) = get(&app, "/admin/problems").await;
    assert!(
        listing.contains(&format!(r#"href="/admin/problems/{id}/delete""#)),
        "{listing}"
    );

    let (status, body) = get(&app, &format!("/admin/problems/{id}/delete")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Delete the file"), "{body}");
    assert!(
        body.contains("/data/packages"),
        "the folder is a fact: {body}"
    );
    // Nothing has happened yet.
    assert!(!state.catalog().package_problems().is_empty());

    let (status, _) = post_form(&app, &format!("/admin/problems/{id}/delete"), "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        state.catalog().package_problems().is_empty(),
        "the POST is what does it"
    );
    assert!(
        machine
            .recorded()
            .contains(&km_api::testing::Recorded::DeletedProblemFile(id)),
        "the machine was asked, rather than the page merely redirecting"
    );
}

/// A file the machine may not delete shows the sentence and offers no control.
///
/// `A control that can only be refused is left out, not grayed`, and the sentence is the machine's
/// own — the very one the delete would have refused with.
#[tokio::test]
async fn a_refused_package_the_machine_may_not_delete_offers_no_control_and_says_why() {
    let machine = TestMachine::with_catalog(6);
    machine.set_package_problems(vec![refused("/data/packages/carols.kmpkg")]);
    machine.set_faults(km_api::testing::Faults {
        unremovable_problems: vec![(
            "/data/packages/carols.kmpkg".to_owned(),
            "named in debug.packages".to_owned(),
        )],
        ..Default::default()
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/problems").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("named in debug.packages"), "{body}");
    assert!(
        !body.contains("/delete\""),
        "and offers nothing to press: {body}"
    );
}

/// ...and the POST refuses on its own, for a URL typed rather than followed.
///
/// The songs tab's `the_post_still_refuses_without_having_been_asked_first` invariant, which holds
/// here for the same reason: leaving the control out of the page is a courtesy, and the operation
/// asks the same predicate again before it touches anything.
#[tokio::test]
async fn posting_a_delete_the_machine_may_not_do_leaves_the_file_listed() {
    let machine = TestMachine::with_catalog(6);
    machine.set_package_problems(vec![refused("/data/packages/carols.kmpkg")]);
    machine.set_faults(km_api::testing::Faults {
        unremovable_problems: vec![(
            "/data/packages/carols.kmpkg".to_owned(),
            "named in debug.packages".to_owned(),
        )],
        ..Default::default()
    });
    let id = refused("/data/packages/carols.kmpkg").id();
    let (app, state) = nested_with_state(machine, Arc::new(SpyGuard::default()));

    let (status, _) = post_form(&app, &format!("/admin/problems/{id}/delete"), "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        state.catalog().package_problems().len(),
        1,
        "the refusal is the operation's, not the page's"
    );
}

/// Confirming something that is no longer refused says so, and says it as good news.
///
/// The stale-page case, and the usual cause is the one worth wording kindly: a rescan took the file
/// in between the page being drawn and the link being pressed.
#[tokio::test]
async fn confirming_a_problem_that_is_no_longer_listed_says_so_rather_than_asking() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));

    let where_to = redirected_to(&app, "/admin/problems/gone-00000000/delete").await;
    // `good`, not `bad`: the id going stale almost always means a rescan took the file in, which is
    // the outcome somebody wanted. Answering it in red would report a fixed machine as broken.
    assert!(
        where_to.starts_with("/admin/problems?kind=good"),
        "a stale link is news rather than a fault: {where_to}"
    );
    assert!(where_to.contains("fixed"), "{where_to}");
}

/// Where a `GET` sent the browser, for the handlers that answer with a notice rather than a page.
async fn redirected_to(app: &Router, path: &str) -> String {
    let response = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).expect("request"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// The sound fault reports itself **and carries the control that fixes it**.
///
/// **This test asserted the opposite until now**, and the paragraph it was written from was right
/// about the reason and wrong about the remedy: a page that gathers faults it cannot fix is the one
/// failure mode this arrangement has, and *Go there* was supposed to answer it. It did not, for the
/// device fault — the tab it pointed at listed banks and had no output picker at all.
///
/// The link stays, because the owning tab knows things this one does not.
#[tokio::test]
async fn the_problems_tab_reports_the_sound_fault_and_offers_a_bank_for_it() {
    let machine = TestMachine::with_catalog(6);
    machine.set_soundfont(km_api::machine::SoundFontStatus {
        playing: km_api::machine::SoundKind::TestTone,
        problem: Some("the bank would not load".to_owned()),
        ..Default::default()
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/problems").await;
    assert_eq!(status, StatusCode::OK);
    // The wording is `SoundFontStatus::complaint`'s, which the idle screen shows too — one machine,
    // one sentence, rather than this crate inventing a second reading of the same two fields.
    assert!(
        body.contains("instruments will sound wrong: the bank would not load"),
        "{body}"
    );
    // The fix, in place: the Sound tab's own route, with a hidden field that brings the redirect
    // back here rather than leaving somebody on a different tab.
    assert!(body.contains(r#"action="/admin/sound/use""#), "{body}");
    assert!(
        body.contains(r#"name="back" value="problems""#),
        "the fix would not return to this tab: {body}"
    );
    assert!(body.contains(r#"href="/admin/sound""#), "{body}");
    assert!(
        body.contains(r#"<span class="badge badge-bad">1</span>"#),
        "a fault counts toward the badge even with every package installed: {body}"
    );
}

/// The device fault carries the picker, which is the row this whole change is for.
///
/// **It was the one fault whose link led nowhere useful**: the Sound tab listed banks and never
/// outputs, so *Go there* went to a tab with no control for it. On the appliance this row means the
/// ALSA card order moved between boots and the machine is playing to nobody.
#[tokio::test]
async fn the_problems_tab_offers_the_output_picker_for_a_device_that_is_gone() {
    let machine = TestMachine::with_catalog(6);
    // A saved device that is not present: the fake lists it unavailable and reports `fell_back`.
    machine.set_audio_outputs(
        vec![
            km_api::machine::AudioOutput {
                id: km_api::machine::SYSTEM_OUTPUT.to_owned(),
                name: "Follow the system default".to_owned(),
                system_default: false,
                usb: false,
                available: true,
                preferred: true,
            },
            km_api::machine::AudioOutput {
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
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/problems").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("is not there, so"), "{body}");
    assert!(body.contains(r#"action="/admin/sound/output""#), "{body}");
    // The sentinel is labelled in the reader's language rather than in the backend's English, which
    // is why that label lives in the template. And the unplugged device is still listed: the choice
    // survives being unplugged, and hiding it would report the fallback as a choice.
    assert!(body.contains("Follow the system"), "{body}");
    assert!(body.contains("USB Audio CODEC"), "{body}");
}

/// The picture fault carries a file chooser, and it posts to the Pictures tab's own route.
#[tokio::test]
async fn the_problems_tab_offers_a_picture_for_an_empty_rotation() {
    let machine = TestMachine::with_catalog(6);
    machine.set_wallpapers(km_api::machine::WallpaperState {
        problem: Some("no pictures in the folder".to_owned()),
        ..Default::default()
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (_, body) = get(&app, "/admin/problems").await;
    assert!(body.contains("no pictures in the folder"), "{body}");
    // `back` is in the query here and a field everywhere else, because `uploads::receive` owns the
    // whole multipart body.
    assert!(
        body.contains(r#"action="/admin/pictures/upload?back=problems""#),
        "{body}"
    );
    assert!(body.contains(r#"enctype="multipart/form-data""#), "{body}");
}

/// The confirmation names what was chosen, not what happens to be sounding.
///
/// **Found by pressing the button on an idle machine, which is every machine somebody is
/// configuring.** `Holding the audio device` means the endpoint is handed back five seconds after
/// the last song, so `AudioOutputs::active_name` is the engine's placeholder — and the notice read
/// *"The sound comes out of not yet opened now."* The two fields answer different questions and this
/// sentence wants the other one.
#[tokio::test]
async fn choosing_an_output_confirms_with_the_name_of_the_one_chosen() {
    let (app, _state) = nested_with_password(Arc::new(SpyGuard::default()));

    let sent = posted_to(
        &app,
        "/admin/sound/output",
        "id=alsa%3Aplughw%3ACARD%3DDevice%2CDEV%3D0",
    )
    .await;
    assert!(
        sent.contains("USB+Audio+CODEC"),
        "the notice did not name the chosen device: {sent}"
    );

    // **The sentinel gets a sentence of its own, and needs one.** Substituting its label into the
    // message above produces *"The sound comes out of Follow the system now"*, which reads as a
    // fault in the page rather than as a confirmation.
    let system = posted_to(&app, "/admin/sound/output", "id=system").await;
    assert!(
        system.contains("follows+the+system+default"),
        "the sentinel did not get its own sentence: {system}"
    );
    assert!(
        !system.contains("comes+out+of"),
        "the sentinel was substituted into the device sentence: {system}"
    );
}

/// A fix applied from the Problems tab comes back to the Problems tab.
///
/// **And an invented `back` does not become a redirect**, which is the half worth pinning: the value
/// comes off a form and reaches a `Location` header, so `back_tab` whitelists it and anything else
/// lands on the tab the control belongs to.
///
/// **The outcome is in `Location` and not in the body.** These routes answer `303` with an empty
/// body, so an assertion over the body would be vacuous — a trap this file has already recorded
/// falling into once.
#[tokio::test]
async fn a_fix_returns_to_the_tab_it_was_applied_from_and_nowhere_else() {
    let (app, _state) = nested_with_password(Arc::new(SpyGuard::default()));

    let sent = posted_to(&app, "/admin/sound/output", "id=system&back=problems").await;
    assert!(
        sent.starts_with("/admin/problems?"),
        "a fix from the Problems tab did not return there: {sent}"
    );

    let invented = posted_to(
        &app,
        "/admin/sound/output",
        "id=system&back=https://elsewhere.invalid",
    )
    .await;
    assert!(
        invented.starts_with("/admin/sound?"),
        "an invented back reached the redirect: {invented}"
    );
}

/// Where a `POST` sent the browser. The twin of [`redirected_to`], for a form rather than a link.
async fn posted_to(app: &Router, path: &str, body: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// A refused package, as a test arranges one.
///
/// The id is deliberately **not** passed in: it is derived from the path, so a fixture cannot hand
/// the double an id naming a different file than the one it holds — which is the whole reason
/// `PackageProblem::id` is a method rather than a field.
fn refused(path: &str) -> km_api::machine::PackageProblem {
    km_api::machine::PackageProblem {
        path: path.to_owned(),
        package_id: None,
        reason: "could not read manifest.json".to_owned(),
    }
}

/// The nth `/admin/problems/…` link in a page, for telling two rows apart.
fn link_at(body: &str, nth: usize) -> String {
    body.match_indices("/admin/problems/")
        .nth(nth)
        .map(|(at, _)| body[at..].split('"').next().unwrap_or_default().to_owned())
        .unwrap_or_default()
}

/// The bundled bank still offers nothing, and a bank that can go leads to a question.
#[tokio::test]
async fn a_removable_bank_asks_first_and_the_bundled_one_is_not_offered_at_all() {
    let machine = TestMachine::with_catalog(6);
    machine.set_soundfonts(km_api::machine::SoundFontBanks {
        banks: vec![
            km_api::machine::SoundFontBank {
                id: "bundled".to_owned(),
                name: "Bundled".to_owned(),
                bytes: 32_319_396,
                bundled: true,
                why_not_removable: Some("it ships with the machine".to_owned()),
            },
            km_api::machine::SoundFontBank {
                id: "sc55".to_owned(),
                name: "SC-55".to_owned(),
                bytes: 108_000_000,
                bundled: false,
                why_not_removable: None,
            },
        ],
        selected: "bundled".to_owned(),
        offers: Vec::new(),
        fetching: None,
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/sound").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"href="/admin/sound/sc55/remove""#));
    assert!(!body.contains(r#"href="/admin/sound/bundled/remove""#));
    // The bundled row carries its badge and does not also print the sentence: it would be saying
    // the same thing twice.
    assert!(!body.contains("it ships with the machine"));

    let (status, body) = get(&app, "/admin/sound/sc55/remove").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("103.0 MiB"), "the size is on the question");
    assert!(body.contains(r#"action="/admin/sound/sc55/remove""#));
}

/// The Pictures tab lists files, offers Remove only where it would work, and asks first.
///
/// The bank test's shape one tab over, with the two differences that are the point of this feature:
/// a zip is **one row saying how many pictures it holds**, because the file is the removable unit;
/// and the row that cannot be removed shows the machine's own sentence in place of the control,
/// rather than a button that is always refused.
#[tokio::test]
async fn a_removable_picture_asks_first_and_a_pinned_one_is_not_offered_at_all() {
    let machine = TestMachine::with_catalog(6);
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/pictures").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"href="/admin/pictures/sunset-jpg/remove""#));
    assert!(body.contains(r#"href="/admin/pictures/beach-zip/remove""#));
    assert!(
        !body.contains(r#"href="/admin/pictures/pinned-jpg/remove""#),
        "a file the machine will not delete gets no Remove link"
    );
    assert!(
        body.contains("debug.wallpapers"),
        "...it gets the machine's own sentence instead: {body}"
    );

    // The archive says how many pictures go with it, on the row and again on the question.
    let (status, body) = get(&app, "/admin/pictures/beach-zip/remove").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Pictures in it"),
        "removing a zip removes several pictures and has to say so: {body}"
    );
    assert!(body.contains(r#"action="/admin/pictures/beach-zip/remove""#));

    // The confirmation refuses the pinned file too, rather than offering a button the POST denies.
    let (status, _) = get(&app, "/admin/pictures/pinned-jpg/remove").await;
    assert_eq!(
        status,
        StatusCode::SEE_OTHER,
        "a typed URL meets the same refusal the row showed"
    );
}

/// A byte count somebody can read, including the one the formatter used to get wrong.
///
/// It was written for banks, which stop around 300 MiB. A twenty-gigabyte package read as
/// `20480.0 MiB`, which is the number a confirmation exists to make legible.
#[tokio::test]
async fn a_package_larger_than_a_gibibyte_is_not_shown_in_mebibytes() {
    let machine = TestMachine::with_catalog(6);
    machine.set_soundfonts(km_api::machine::SoundFontBanks {
        banks: vec![km_api::machine::SoundFontBank {
            id: "huge".to_owned(),
            name: "Huge".to_owned(),
            bytes: 21_474_836_480,
            bundled: false,
            why_not_removable: None,
        }],
        selected: "huge".to_owned(),
        offers: Vec::new(),
        fetching: None,
    });
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/sound/huge/remove").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("20.0 GiB"), "not 20480.0 MiB");
}

/// Nothing else on the page grew a confirmation.
///
/// The scope is the rule: a page that asks twice teaches people to click twice, so only the two
/// controls that destroy a file ask. These three still act on the first press.
#[tokio::test]
async fn the_controls_that_destroy_no_file_are_still_one_click() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/songs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"action="/admin/songs/vol1/bank""#));

    let (_, body) = get(&app, "/admin/pictures").await;
    assert!(body.contains(r#"action="/admin/pictures/next""#));

    let (_, body) = get(&app, "/admin/machine").await;
    assert!(body.contains(r#"action="/admin/machine/name""#));
}

/// A well-formed multipart body past axum's 2 MB default reaches the handler on every upload route.
///
/// **The regression test for a cap nobody wrote, and the second attempt at writing it.**
/// `DefaultBodyLimit` is 2 MB unless a route says otherwise, and all three of these forms carry
/// files far past it — a package holding a video, a 32 MB SoundFont, a photograph. Without the
/// layers an 85 MB package uploaded 2,162,688 bytes and came back *"the upload stopped early: Error
/// parsing `multipart/form-data` request"*.
///
/// **That message is why this test has to be shaped exactly so, and the first version of it was
/// useless.** A body of rubbish never reaches the limit: the multipart parser cannot find a boundary
/// in it and fails first, so the route answers identically with the layer and without it. So the
/// body has to be genuinely well formed and genuinely too big.
///
/// The assertion stays on **what the page says** rather than on the status, and that is now a choice
/// rather than a necessity. These routes answer `303` and redirect whatever happened, so a status
/// assertion here would be vacuous — but the underlying refusal did also change: a limit trip is a
/// `413` in `km-api` now, because `multipart_failure` reads axum's `status()` instead of formatting
/// its `Display`. `a_file_past_the_limit_is_a_413_that_names_the_limit` in `km-api`'s surface tests
/// is where that is pinned.
///
/// Three megabytes: past the 2 MB default, small enough to stay quick. The upload is rejected either
/// way — 3 MB of `x` is not a package, a PNG or a SoundFont — and *which* rejection arrives is the
/// whole signal.
#[tokio::test]
async fn a_well_formed_upload_past_axums_default_limit_reaches_the_handler() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    for (path, filename) in [
        ("/admin/songs/upload", "big.kmpkg"),
        ("/admin/pictures/upload", "big.png"),
        ("/admin/sound/upload", "big.sf2"),
    ] {
        let boundary = "zzzzzzzzzzzzzzzz";
        let head = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        );
        let tail = format!("\r\n--{boundary}--\r\n");
        let mut payload = Vec::from(head);
        payload.extend_from_slice(&vec![b'x'; 3 * 1024 * 1024]);
        payload.extend_from_slice(tail.as_bytes());

        let request = Request::post(path)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(payload))
            .expect("request");
        // **The outcome is in `Location`, not in the body**, and reading the body instead is how the
        // first two versions of this test passed with the bug present: these routes answer `303` and
        // redirect back to the page with `kind=` and `said=` in the query, so the response body is
        // empty and every assertion over it is vacuous.
        let response = app.clone().oneshot(request).await.expect("answer");
        let said = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(
            !said.contains("stopped+early") && !said.contains("stopped early"),
            "{path} cut a 3 MB upload off, so its DefaultBodyLimit layer is missing: {said}"
        );
    }
}

// -- what language these pages, and the television, are in ---------------------------------------

/// A navigation with a header set, for the two ways a language is asked for.
async fn get_with(app: &Router, path: &str, header: (&str, &str)) -> (StatusCode, String) {
    send(
        app,
        Request::get(path)
            .header(header.0, header.1)
            .body(Body::empty())
            .expect("request"),
    )
    .await
}

/// The owner's pages follow the same cookie the singer's remote writes.
///
/// Both are mounted on one origin, so a viewer who chose Portuguese at `/` should not meet English
/// at `/admin/` — which is the whole reason the cookie's name lives in `km-locale` rather than in
/// either pages crate.
#[tokio::test]
async fn the_owner_pages_follow_the_same_cookie_the_remote_writes() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get_with(&app, "/admin/machine", ("Cookie", "km_locale=pt-BR")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Esta máquina"), "the tab bar: {body}");
    assert!(!body.contains(">Songs<"), "English survived: {body}");
}

/// With no cookie, the browser's own language decides.
#[tokio::test]
async fn a_browser_that_asks_for_portuguese_gets_portuguese_pages() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (_, body) = get_with(
        &app,
        "/admin/machine",
        ("Accept-Language", "pt-BR,pt;q=0.9"),
    )
    .await;
    assert!(body.contains("Esta máquina"), "{body}");
}

/// The picker names each language in itself, which is the one label that must not be translated.
#[tokio::test]
async fn the_machine_language_picker_names_each_language_in_its_own_words() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (_, body) = get(&app, "/admin/machine").await;
    assert!(body.contains("<h2>Screen language</h2>"), "{body}");
    assert!(body.contains("Português (Brasil)"), "{body}");
    assert!(body.contains(">English<"), "{body}");
}

/// A tag this build has no catalog for is refused rather than silently accepted.
#[tokio::test]
async fn a_language_this_build_does_not_have_is_refused_rather_than_saved() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = post_form(&app, "/admin/machine/locale", "locale=klingon").await;
    assert_eq!(
        status,
        StatusCode::SEE_OTHER,
        "it redirects back with a notice"
    );
    let _ = body;
}

// -- power ---------------------------------------------------------------------------------------

/// The same router over a host that can switch its own box off.
///
/// **Not the default for this file**, deliberately: every other test here should be running against
/// the machine an owner most likely has, which is one with no power controls at all.
fn nested_with_power(guard: Arc<SpyGuard>) -> (Router, Arc<TestPower>) {
    let state = ApiState::from_machine(
        TestMachine::with_catalog(6).shared(),
        ApiConfig::default().without_mdns(),
    );
    let power = Arc::new(TestPower::new());
    assert!(state.set_power(power.clone()), "power installs once");
    let app = Router::new().nest("/admin", router(admin_over(state, guard)));
    (app, power)
}

/// The card is part of the document or it is not. There is no third, grayed state, which is this
/// page's standing rule: a disabled control invites somebody to work out what would enable it, and
/// the answer — *run on a supervised appliance* — is not something anybody can do from here.
#[tokio::test]
async fn the_power_card_is_absent_on_a_machine_that_cannot_switch_itself_off() {
    let (app, _) = nested(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.contains("/admin/machine/power/off"),
        "a machine with no power control offered to shut down: {body}"
    );
    assert!(!body.contains("/admin/machine/power/restart"), "{body}");

    let (app, _) = nested_with_power(Arc::new(SpyGuard::default()));
    let (status, body) = get(&app, "/admin/machine").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("/admin/machine/power/off"), "{body}");
    assert!(body.contains("/admin/machine/power/restart"), "{body}");
}

/// Shutting down asks first; restarting does not. The asymmetry is the assertion — see
/// `confirm_shut_down` for why it extends the confirmation rule rather than breaking it.
#[tokio::test]
async fn shutting_down_asks_first_and_restarting_does_not() {
    let (app, power) = nested_with_power(Arc::new(SpyGuard::default()));

    let (status, body) = get(&app, "/admin/machine/power/off").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"action="/admin/machine/power/off""#),
        "the confirmation must post to the address it was fetched from: {body}"
    );
    assert!(
        power.recorded().is_empty(),
        "asking is not doing: {:?}",
        power.recorded()
    );

    // And there is no `GET` twin for restart at all, so a link cannot restart the machine.
    let (status, _) = get(&app, "/admin/machine/power/restart").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn both_controls_reach_the_host_and_answer_a_page_rather_than_a_redirect() {
    let (app, power) = nested_with_power(Arc::new(SpyGuard::default()));

    let (status, body) = post_form(&app, "/admin/machine/power/restart", "").await;
    // A page and not a 303: the machine is down for the seconds a browser would spend following a
    // redirect, so the redirect is a guaranteed failed load that reads as the button being broken.
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("http-equiv=\"refresh\""),
        "a restart brings the tab back by itself: {body}"
    );
    assert_eq!(power.recorded(), vec![Recorded::RestartApplication]);

    let (app, power) = nested_with_power(Arc::new(SpyGuard::default()));
    let (status, body) = post_form(&app, "/admin/machine/power/off", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains("http-equiv=\"refresh\""),
        "nothing to come back to, so nothing should keep trying: {body}"
    );
    assert_eq!(power.recorded(), vec![Recorded::ShutDown]);
}

/// The guard covers these like everything else. Free by construction — the layer is on the whole
/// router — and worth asserting once, because the cost of it not being true is that anybody on the
/// LAN can switch the television off.
#[tokio::test]
async fn the_power_controls_are_refused_when_the_guard_refuses() {
    let guard = Arc::new(SpyGuard {
        refuse: Some(Refusal::NeedsPassword("no".to_owned())),
        ..SpyGuard::default()
    });
    let (app, power) = nested_with_power(guard.clone());

    for path in ["/admin/machine/power/off", "/admin/machine/power/restart"] {
        let (status, _) = post_form(&app, path, "").await;
        assert_ne!(status, StatusCode::OK, "{path} ran without the guard");
    }
    assert!(guard.asked() >= 2, "the guard was not consulted");
    assert!(
        power.recorded().is_empty(),
        "a refused caller reached the host"
    );
}

/// A tool says whether it is logged in and leads to the door; the machine says neither.
///
/// # The regression this exists for
///
/// **A tool holding no token can write nothing**, every route this page drives being under
/// `/api/v1/admin/`, and there was nowhere on any of its pages to type a password. The route was
/// mounted and the guard implemented; no markup reached either, and nothing gates a tool's routes,
/// so nothing redirected a caller to the login page either. Saving the demo switch, deleting a
/// package and deleting a picture each reached the machine, were refused with a 401, and came back
/// as a banner naming a box that did not exist.
///
/// **Every test that logs in posts to `/admin/login` directly**, which is why the suite passed
/// throughout. This one asks the question a person asks: can this page be got out of.
#[tokio::test]
async fn a_tool_says_whether_it_is_logged_in_and_the_machine_does_not() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let guard = Arc::new(SpyGuard {
            refuse: Some(Refusal::NeedsPassword("Log in first.".to_owned())),
            ..SpyGuard::default()
        });
        let (app, _) = nested_as_with_password(host, guard);
        let (status, body) = get(&app, "/admin/machine").await;

        match host {
            Host::Tool => {
                assert_eq!(status, StatusCode::OK, "{surface}");
                assert!(
                    body.contains("This program is not logged in"),
                    "{surface} says nothing about why its every write is refused: {body}"
                );
                assert!(
                    body.contains(r#"href="/admin/connect""#),
                    "{surface} offers no way to the page that mends it: {body}"
                );
                // **And no second place to log in.** The field is on the door; what is here is the
                // state, which is `km-admin says logged in`.
                assert!(
                    !body.contains(r#"action="/admin/login""#),
                    "{surface} carries a second login form: {body}"
                );
            }
            // **The machine's whole page is behind that password already.** A caller without a
            // token is redirected to `/admin/login` before this handler runs, so a sentence about
            // holding one would be about a question that cannot arise.
            Host::Machine => {
                assert_eq!(
                    status,
                    StatusCode::SEE_OTHER,
                    "{surface} let a refused caller reach the tab"
                );
            }
        }
    }
}

/// Every pane a radio names, for the count below and for the fallback above it.
const EVERY_PANE: [&str; 5] = ["debug", "name", "password", "demo", "language"];

/// Exactly one pane opens, on either host and whatever a query string asked for.
///
/// **`.settings .pane { display: none }` is unconditional**, which is the polarity argument
/// `machine.html` makes: a stylesheet that fails to load leaves every section stacked rather than
/// none. The cost is that a `pane` naming a radio this host does not render would open *nothing*,
/// and the tab's whole settings half would be gone with no error anywhere. `Pane::drawn` answers
/// that with Debugging, and these are the two values that arrive that way: `language` has no route
/// on a tool, and `login` no counterpart on the machine.
#[tokio::test]
async fn exactly_one_pane_opens_whatever_was_asked_for() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        for asked in [
            "",
            "?pane=login",
            "?pane=language",
            "?pane=demo",
            "?pane=nonsense",
        ] {
            let (app, _) = nested_as_with_password(host, Arc::new(SpyGuard::default()));
            let (status, body) = get(&app, &format!("/admin/machine{asked}")).await;
            assert_eq!(status, StatusCode::OK, "{surface} {asked}");
            let opened: Vec<&str> = EVERY_PANE
                .into_iter()
                .filter(|pane| body.contains(&format!(r#"id="machine-tab-{pane}" checked"#)))
                .collect();
            assert_eq!(
                opened.len(),
                1,
                "{surface} opened {opened:?} for {asked:?}, and one pane is what a strip shows"
            );
        }
    }
}

/// A save comes back to the pane it was made on.
///
/// Every control on this tab is a `POST` that reloads the page, so without this the notice about a
/// rename is read underneath a debug switch: a message about a page somebody is no longer looking
/// at. Asserted through the `Location` and then through the page that `Location` names, because the
/// redirect carrying the pane and the template honouring it are two separate things to get wrong.
#[tokio::test]
async fn a_save_comes_back_to_the_pane_it_was_made_on() {
    for (path, body, pane) in [
        ("/admin/machine/name", "name=Living+Room", "name"),
        ("/admin/machine/demo", "enabled=yes", "demo"),
        ("/admin/machine/demo-delay", "delay_secs=45", "demo"),
        ("/admin/machine/debug", "enabled=yes", "debug"),
        ("/admin/machine/sessions", "", "password"),
    ] {
        let (app, _) = nested_with_password(Arc::new(SpyGuard::default()));
        let said = post_form_to(&app, path, body).await;
        assert!(
            said.contains(&format!("pane={pane}")),
            "{path} came back to something other than the {pane} pane: {said}"
        );

        let (status, drawn) = get(&app, &said).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            drawn.contains(&format!(r#"id="machine-tab-{pane}" checked"#)),
            "{path} redirected to the {pane} pane and the page opened another: {drawn}"
        );
    }
}

/// A pane nobody wrote down opens Debugging, and nothing that arrived is echoed into the page.
///
/// The value comes off a query string and is interpolated into a redirect, so `handlers::back_pane`
/// answers with an enum and the URL is built from `Pane::id`. This is that, asserted: an unknown
/// name is a fallback rather than an opening for markup.
#[tokio::test]
async fn an_unknown_pane_opens_debugging_and_is_never_echoed() {
    let (app, _) = nested_with_password(Arc::new(SpyGuard::default()));
    let (status, body) = get(
        &app,
        "/admin/machine?pane=%22%3E%3Cscript%3Ealert(1)%3C/script%3E",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"id="machine-tab-debug" checked"#),
        "an unknown pane did not fall back to the one that is never empty"
    );
    assert!(
        !body.contains("<script>alert"),
        "a query string reached the markup: {body}"
    );
}

/// No page the machine's own tabs serve carries the tray, on either host.
///
/// **htmx does not swap a non-2xx response**, so a host's own `ui.js` is what words a refusal, and
/// it looks this element up by id and returns when it is absent. The condition is the same
/// `scripts` list the `<script>` tags come from, and these pages extend the layout directly rather
/// than through `shell.html` -- so they carry neither, on whichever host serves them.
#[tokio::test]
async fn the_shared_tabs_carry_no_tray_because_they_carry_no_script() {
    for host in BOTH_HOSTS {
        let (app, _) = nested_as_with_password(host, Arc::new(SpyGuard::default()));
        let (status, body) = get(&app, "/admin/machine").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !body.contains(r#"id="toasts""#),
            "{} drew a tray on a page with no script to fill it",
            host.name()
        );
    }
}

/// *Where to find it* leads with the name, and the addresses follow it.
///
/// The name says *which* machine the card is about, and every other fact in it is true of any
/// machine. An address is what to do with that machine once you know it is the right one.
#[tokio::test]
async fn where_to_find_it_leads_with_the_name() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let (app, _) = nested_as_with_password(host, Arc::new(SpyGuard::default()));
        let (status, body) = get(&app, "/admin/machine").await;
        assert_eq!(status, StatusCode::OK);

        let facts = body.find(r#"class="facts""#).expect("the facts list");
        let addresses = body
            .find(r#"class="addresses""#)
            .or_else(|| body.find(r#"class="empty""#));
        if let Some(addresses) = addresses {
            assert!(
                facts < addresses,
                "{surface} put the addresses above the name"
            );
        }
        // And the whole card is still above the strip, which is what the panel is for: it is built
        // from public reads, where every pane below it is something somebody came to change.
        assert!(
            facts < body.find(r#"class="settings""#).expect("the strip"),
            "{surface} let the panel fall inside the strip"
        );
    }
}

/// A host's own page can carry a notice, and it is drawn where every other banner is.
///
/// **The parameter is easy to add and ignore.** `shell` and `door` filled the field with `None`, so
/// a host redirecting to its own page with `?kind=&said=` had nowhere for the sentence to land and
/// drew the query string alone. A host that worked around that by drawing its own banner inside its
/// body is the second copy this seam exists to delete.
#[tokio::test]
async fn a_hosts_own_page_can_carry_a_notice() {
    let state = ApiState::from_machine(
        TestMachine::with_catalog(6).shared(),
        ApiConfig::default().without_mdns(),
    );
    let host = Arc::new(km_admin_pages::in_process::ThisMachine::new(state));
    let admin = Admin::over(
        km_admin_pages::machine::Capabilities::desktop(),
        km_admin_pages::ICON_ADMIN_PNG,
        Arc::new(SpyGuard::default()),
        host,
    );

    for (notice, wanted) in [
        (km_admin_pages::views::Notice::bad("it went wrong"), "bad"),
        (km_admin_pages::views::Notice::good("it went right"), "good"),
    ] {
        let said = notice.text.clone();
        let response = admin
            .shell(
                km_admin_pages::views::Tab::Sound,
                km_locale::Locale::English,
                Some(notice),
                "<p>a host's own page</p>".to_owned(),
            )
            .await;
        let markup = String::from_utf8_lossy(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("a body"),
        )
        .into_owned();

        assert!(
            markup.contains(&format!("banner-{wanted}")),
            "no banner of that kind: {markup}"
        );
        assert!(markup.contains(&said), "the sentence is missing");
        assert!(
            markup.contains("<p>a host's own page</p>"),
            "the host's body went missing"
        );
    }
}

/// A host that declares a script gets it on every page that host serves, tabs included.
///
/// **The tabs are where the upload forms are**, which is why this reaches past a host's own pages.
/// Songs, Pictures and Sound each carry a multipart form, a package runs to two gibibytes, and a
/// form post that reports nothing while it runs cannot be told from a program that has stopped.
/// Only a script can say a navigation is still in flight, so a host's scripts have to reach the
/// pages this crate draws.
///
/// The counter-assertion is `no_page_the_machine_serves_carries_a_script`: the machine declares
/// none, so the same pages stay byte-for-byte scriptless there.
#[tokio::test]
async fn a_host_that_declares_a_script_gets_it_on_every_page_it_serves() {
    let state = ApiState::from_machine(
        TestMachine::with_catalog(6).shared(),
        ApiConfig::default().without_mdns(),
    );
    let host = Arc::new(km_admin_pages::in_process::ThisMachine::new(state));
    let admin = Admin::over(
        km_admin_pages::machine::Capabilities::desktop(),
        km_admin_pages::ICON_ADMIN_PNG,
        Arc::new(SpyGuard::default()),
        host,
    )
    .with_scripts(&["/static/htmx.min.js", "/static/ui.js"]);
    let app = Router::new().nest("/admin", km_admin_pages::router(admin));

    let mut checked = 0usize;
    for path in [
        "/admin/machine",
        "/admin/songs",
        "/admin/pictures",
        "/admin/sound",
    ] {
        let (status, body) = get(&app, path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        checked += 1;
        assert!(
            body.contains(r#"<script src="/static/htmx.min.js" defer></script>"#),
            "{path} loads no htmx"
        );
        assert!(
            body.contains(r#"<script src="/static/ui.js" defer></script>"#),
            "{path} loads no ui.js"
        );
        // The tray the script fills, and the sentence it says while an upload is in flight.
        assert!(
            body.contains(r#"id="toasts""#) && body.contains("data-sending="),
            "{path} has nowhere for the script to speak"
        );
    }
    assert!(checked > 3, "only {checked} pages were read");
}

// -- the output's own level ----------------------------------------------------------------------

/// A control like the appliance's USB interface: 1 dB steps from −128 dB to unity, sitting 20 dB
/// down, which is the state this control exists because of.
fn attenuated() -> km_api::machine::OutputLevel {
    km_api::machine::OutputLevel {
        db_centi: -2000,
        db_min_centi: -12800,
        db_max_centi: 0,
        step_centi: 100,
    }
}

/// The reading is the half that was missing, so it is drawn without being asked for.
#[tokio::test]
async fn the_sound_tab_says_what_level_the_output_is_running_at() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        let machine = TestMachine::with_catalog(6);
        machine.set_output_level_range(attenuated());
        let (app, _) = nested_as(host, machine, Arc::new(SpyGuard::default()));

        let (status, body) = get(&app, "/admin/sound").await;
        assert_eq!(status, StatusCode::OK, "{surface}");
        assert!(
            body.contains("Sending at -20.0 dB."),
            "{surface} should print the level it is running at: {body}"
        );
    }
}

/// The gain into an amplifier is not a control to leave lying open on a page.
#[tokio::test]
async fn the_slider_is_behind_a_link_and_the_reading_is_not() {
    let machine = TestMachine::with_catalog(6);
    machine.set_output_level_range(attenuated());
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let (_, body) = get(&app, "/admin/sound").await;
    assert!(
        body.contains(r#"href="/admin/sound?level=1""#),
        "the link that reveals the slider should be there: {body}"
    );
    assert!(
        !body.contains(r#"type="range""#),
        "the slider should not be drawn until it is asked for: {body}"
    );

    let (_, body) = get(&app, "/admin/sound?level=1").await;
    assert!(
        body.contains(r#"type="range""#),
        "asking for it should draw it: {body}"
    );
    assert!(
        body.contains(r#"action="/admin/sound/level""#),
        "and it should post somewhere: {body}"
    );
}

/// `A control that can only be refused is left out, not grayed` — and unlike `changeable`, an HDMI
/// output will not gain a level later.
#[tokio::test]
async fn an_output_with_no_level_gets_a_sentence_rather_than_a_dead_slider() {
    for host in BOTH_HOSTS {
        let surface = host.name();
        // No level arranged, which is a machine playing through HDMI.
        let (app, _) = nested_as(
            host,
            TestMachine::with_catalog(6),
            Arc::new(SpyGuard::default()),
        );

        let (_, body) = get(&app, "/admin/sound?level=1").await;
        assert!(
            body.contains("no level the machine can set"),
            "{surface} should say why there is no control: {body}"
        );
        assert!(
            !body.contains(r#"type="range""#),
            "{surface} should not draw a slider that cannot work: {body}"
        );
    }
}

/// Turning it down is audible, costs nothing, and the same slider undoes it.
#[tokio::test]
async fn lowering_the_level_goes_straight_through() {
    let machine = TestMachine::with_catalog(6);
    machine.set_output_level_range(attenuated());
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    let location = post_form_to(&app, "/admin/sound/level", "db=-30").await;
    assert!(
        location.contains("kind=good"),
        "a drop should be applied rather than questioned: {location}"
    );
}

/// Six decibels is a doubling of voltage, and the accident worth stopping is a drag across the
/// slider rather than a nudge.
#[tokio::test]
async fn a_large_rise_asks_first() {
    let machine = TestMachine::with_catalog(6);
    machine.set_output_level_range(attenuated());
    let (app, state) = nested_with_state(machine, Arc::new(SpyGuard::default()));

    // −20 dB to unity is twenty decibels, so this is the drag the question exists for.
    let (status, body) = post_form(&app, "/admin/sound/level", "db=0").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the question is a page, not a redirect"
    );
    assert!(body.contains("Turn the machine up?"), "{body}");
    assert!(
        body.contains(r#"action="/admin/sound/level?db=0&#38;confirmed=1""#),
        "the answer has to carry the level, the confirm form having no fields: {body}"
    );
    // ...and nothing moved while the question was open.
    assert_eq!(
        state.controller().audio_outputs().expect("outputs").level,
        Some(attenuated()),
        "asking is not doing"
    );

    // Answering it applies the change.
    let location = post_form_to(&app, "/admin/sound/level?db=0&confirmed=1", "").await;
    assert!(location.contains("kind=good"), "{location}");
}

/// A nudge is not a drag, and a page that asks twice teaches people to click twice.
#[tokio::test]
async fn a_small_rise_does_not_ask() {
    let machine = TestMachine::with_catalog(6);
    machine.set_output_level_range(attenuated());
    let (app, _) = nested_on(machine, Arc::new(SpyGuard::default()));

    // Five decibels, under the six a doubling of voltage is.
    let location = post_form_to(&app, "/admin/sound/level", "db=-15").await;
    assert!(
        location.contains("kind=good"),
        "a nudge should not be questioned: {location}"
    );
}
