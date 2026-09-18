//! The router: every path, and what static files sit alongside.
//!
//! **The URL prefix is the permission.** Everything under `/api/v1/admin/` demands the admin
//! password; everything outside it never does. One middleware over the whole service answers that
//! from the request path, so there is no table of route ids to drift out of step with the routes
//! themselves — which is exactly what the 46-entry ACL this replaced could do, and did.
//!
//! The cost is real and is worth naming: a resource with a public read and an admin write now spans
//! two prefixes, so `GET /api/v1/demo` and `PUT /api/v1/admin/demo` are the same thing filed in two
//! places. That is the price of a permission a reader can see in the URL, and it is cheaper than a
//! lookup table nobody can verify by eye.
//!
//! **The same surface is mounted a second time at [`DEV_API_PREFIX`], where nothing demands a
//! password**, and that follows from the rule rather than bending it: `/dev/api/v1/admin/...` is not
//! under `/api/v1/admin/`. It exists only while both switches [`dev_console_served`] reads are on.

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post, put};

use crate::error::ApiError;
use crate::handlers;
use crate::server::ApiState;

/// The API's version prefix.
pub const API_PREFIX: &str = "/api/v1";

/// Everything below this is an admin action.
pub const ADMIN_PREFIX: &str = "/api/v1/admin";

/// The one path under [`ADMIN_PREFIX`] that does not demand a token.
///
/// It is how a caller gets one, so gating it behind a token would make the password unusable. It is
/// defended by the rate limiter in [`crate::auth`] instead.
pub const ADMIN_LOGIN_PATH: &str = "/api/v1/admin/login";

/// Where the owner's page is, for anything that has to send somebody to it.
///
/// **Unrelated to [`ADMIN_PREFIX`] above, which is the API's**, and the names colliding while the
/// paths do not is exactly why this is spelled once rather than at each place that needs it: this is
/// a page a person opens, and that is a prefix a request is gated by.
///
/// `/admin` and not `/admin/`, which redirects onto this: a browser follows either, and naming the
/// one the router answers on saves a round trip on a television that has just been switched on.
pub const ADMIN_PAGE_PATH: &str = "/admin";

/// The whole API again, under a prefix where nothing demands a password.
///
/// **The development console's own API, and the point of it is that it is a second prefix rather
/// than a second rule.** `/dev/` is one hand-written page of vanilla JavaScript whose value is being
/// unpolished and immediate; once the permission became the prefix, nine of its calls needed a token
/// typed into a box first, and it had already been broken twice by routes moving under it. Mounting
/// the same routes here answers that without touching [`needs_admin_token`]: this prefix is not
/// under [`ADMIN_PREFIX`], so every path below it is open, including the admin ones.
///
/// **What bounds the cost is [`dev_console_served`], not a password.** Both switches have to be on,
/// the pair is off in every build, and the machine draws a marker on its own screen while it is on.
/// See `The development console has an API that needs no password` in docs/decisions/.
pub const DEV_API_PREFIX: &str = "/dev/api/v1";

/// Whether a request path demands a valid admin token.
///
/// **The prefix test is on `"/api/v1/admin/"` with the slash, not on `ADMIN_PREFIX` alone.** Without
/// it `/api/v1/adminfoo` reads as an admin path and gets gated, and — much worse the other way — a
/// future `/api/v1/administration` would be silently protected by a rule nobody wrote down. The
/// slash is what makes "under the prefix" mean under it.
///
/// **Everything under [`DEV_API_PREFIX`] is open, and that falls out of this rule rather than being
/// carved out of it.** `/dev/api/v1/admin/password` does not begin with `/api/v1/admin`, so the
/// `strip_prefix` below already misses it. That is the mechanism and it is deliberate — but it is
/// also invisible, which is why the tests name `DEV_API_PREFIX` explicitly: a future edit that
/// loosened this to a `contains` or an `ends_with` would close the dev API silently, and one that
/// matched `/admin/` anywhere would gate it while every comment here said otherwise.
pub fn needs_admin_token(path: &str) -> bool {
    let Some(rest) = path.strip_prefix(ADMIN_PREFIX) else {
        return false;
    };
    if !rest.starts_with('/') {
        return false;
    }
    path != ADMIN_LOGIN_PATH
}

/// Whether the development console and its API are served at all.
///
/// **Two switches, and both have to be on.** `api.serve_dev_remote` asks for the page;
/// `debug.enabled` is the machine already saying it is a machine being worked on, and it is what
/// the marker on the television and the warning on both admin surfaces are already about. Requiring
/// it means a console whose API needs no password cannot be one switch away from a machine in a
/// living room, and it means the two things an owner has to leave off are the two things the screen
/// is already telling them about.
///
/// One named function rather than the conjunction spelled at each use, so the rule is legible in one
/// place — `router_with` mounts three things on it.
#[must_use]
pub fn dev_console_served(config: &crate::ApiConfig) -> bool {
    config.debug_enabled && config.serve_dev_remote
}

/// Every route mounted unconditionally, as `(method, path)`.
///
/// Documentation that a test can execute: the integration tests drive one request at each entry to
/// prove the surface is actually mounted. Paths are relative to [`API_PREFIX`] and are concrete
/// samples rather than axum patterns, so `{id}` never appears.
pub const SURFACE: &[(&str, &str)] = &[
    ("GET", "/discover"),
    ("GET", "/connect"),
    ("GET", "/state"),
    ("GET", "/songs"),
    ("GET", "/songs/1001"),
    ("GET", "/songs/1001/lyrics"),
    ("GET", "/songs/export"),
    ("GET", "/songs/book.pdf"),
    ("GET", "/queue"),
    ("POST", "/queue"),
    ("DELETE", "/queue"),
    ("DELETE", "/queue/0"),
    ("POST", "/queue/0/move"),
    ("POST", "/transport/play"),
    ("POST", "/transport/pause"),
    ("POST", "/transport/skip"),
    ("POST", "/transport/restart"),
    ("POST", "/transport/stop"),
    ("POST", "/transport/seek"),
    ("GET", "/settings"),
    ("PUT", "/settings"),
    ("GET", "/mics"),
    ("PUT", "/mics/mic1"),
    ("GET", "/audio/outputs"),
    ("GET", "/audio/soundfont"),
    ("GET", "/audio/soundfonts"),
    ("GET", "/wallpapers"),
    ("POST", "/wallpapers/next"),
    ("GET", "/locale"),
    ("GET", "/demo"),
    ("POST", "/demo/start"),
    ("GET", "/packages"),
    ("GET", "/debug"),
    ("GET", "/dev-remote"),
    ("GET", "/performance"),
    ("GET", "/events"),
    // Admin. Every one of these answers 401 without a token, which is what the sweep asserts.
    ("POST", "/admin/login"),
    ("POST", "/admin/logout"),
    ("POST", "/admin/password"),
    ("POST", "/admin/sessions/reset"),
    ("PUT", "/admin/audio/output"),
    ("PUT", "/admin/audio/soundfont"),
    ("POST", "/admin/audio/soundfont/fetch"),
    ("POST", "/admin/audio/soundfonts"),
    ("DELETE", "/admin/audio/soundfonts/generaluser"),
    ("POST", "/admin/wallpapers"),
    ("DELETE", "/admin/wallpapers/sunset-jpg"),
    ("PUT", "/admin/machine/name"),
    ("PUT", "/admin/machine/locale"),
    ("PUT", "/admin/demo"),
    ("PUT", "/admin/demo/delay"),
    ("POST", "/admin/packages"),
    ("POST", "/admin/packages/upload"),
    ("POST", "/admin/packages/rescan"),
    ("DELETE", "/admin/packages/vol1"),
    ("PUT", "/admin/packages/vol1/bank"),
    ("PUT", "/admin/debug"),
    ("PUT", "/admin/dev-remote"),
    ("PUT", "/admin/performance"),
];

/// The routes mounted only while debugging mode is on.
///
/// **Not admin routes, and that is what keeps the prefix rule exact.** Off, they are not mounted at
/// all and answer 404; on, they are public. A third state — mounted but admin-gated — would put a
/// password-demanding path outside `/api/v1/admin/`, and then the URL would stop being the
/// permission. See `Debugging is a mode, and the machine says when it is on` in docs/decisions/.
pub const DEBUG_SURFACE: &[(&str, &str)] =
    &[("POST", "/debug/play-file"), ("POST", "/debug/play-upload")];

/// The routes mounted only on a host that can do something about its own power.
///
/// **Admin routes, unlike [`DEBUG_SURFACE`], and the difference is not an inconsistency.** Those two
/// are off-or-public because debugging is a *mode* somebody turns on; these are absent-or-admin
/// because power is a *capability* the host either has or does not. Both obey the same rule —
/// the URL prefix is the permission — and both answer a real 404 rather than mounting a handler
/// that could only refuse. See [`crate::power`].
///
/// Kept out of [`SURFACE`] deliberately: the integration sweep executes every row it holds, and a
/// sweep that called these on every test run would be asking a fake to switch a box off once per
/// suite while also failing against the default harness, which has no power control at all.
pub const POWER_SURFACE: &[(&str, &str)] = &[
    ("GET", "/admin/power"),
    ("POST", "/admin/power/off"),
    ("POST", "/admin/power/restart"),
];

/// The routes mounted only on a machine that keeps its own recent log in memory.
///
/// **Admin, like [`POWER_SURFACE`] and unlike [`DEBUG_SURFACE`].** A log line names file paths,
/// package folders, the address this machine resolved and the name its owner gave it, so reading
/// one is the owner's business — and the URL prefix being the permission means there is no third
/// state to put it in.
///
/// **On the dev mirror, where the power routes are not**, and the difference is not an
/// inconsistency either. That exception is about a change nobody can undo from a page; reading a
/// log is not a change at all, and the mirror already carries `debug/play-file`, which will play
/// any path on the machine's disk to whoever asks. A tail is well inside a bargain that includes
/// that.
///
/// Kept out of [`SURFACE`] for [`POWER_SURFACE`]'s reason: the integration sweep drives every row
/// it holds, and the harness those tests build keeps no log.
pub const LOG_SURFACE: &[(&str, &str)] = &[("GET", "/admin/logs"), ("GET", "/admin/logs/stream")];

/// The route mounted only where the operating system has a mixer to reach.
///
/// Kept out of [`SURFACE`] for [`POWER_SURFACE`]'s reason, and the harness is the same argument in
/// a second form: a default `Controller` refuses this route, so a sweep that drove it would be
/// asserting that every machine has a level rather than that this one does.
///
/// **On the dev mirror.** Moving a level is a change, but it is one the same page can move back,
/// which is the line the power routes sit the other side of.
pub const LEVEL_SURFACE: &[(&str, &str)] = &[("PUT", "/admin/audio/level")];

/// What this machine can do, read while the router is built and fixed for the run.
///
/// **A named struct rather than two `bool` arguments**, on [`Extras`]' argument: a call site reading
/// `api_router(&config, true, false)` says nothing about which is which, and these two are wired
/// *differently* on the mirror — power off it, logs on it. Naming them puts that asymmetry where it
/// happens instead of in a comment above it.
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    /// Whether this host can do something about its own power.
    pub power: bool,
    /// Whether this machine keeps its own recent log in memory.
    pub logs: bool,
    /// Whether this host can reach the operating system's mixer at all.
    ///
    /// A property of the build rather than of the moment, like [`Capabilities::power`]: a Linux,
    /// Windows or macOS binary can always ask, and one built for Android or iOS has nothing to ask
    /// with. Whether
    /// the *active device* has a level is a separate question with its own answer, reported as
    /// `AudioOutputs::level`, because a machine that can reach a mixer can still be playing through
    /// an output that has no level in it.
    pub output_level: bool,
}

/// Builds the whole service: the API, the event stream and the dev remote, with nothing at `/`.
///
/// `/` then serves [`landing`], which is what an owner should see when the singer-facing remote has
/// been switched off — not a bare 404 to somebody who typed the address off the television correctly.
pub fn router(state: ApiState) -> Router {
    router_with(state, Extras::default())
}

/// The pages this service is assembled with, beside the API itself.
///
/// **A struct rather than a second `Option<Router>`, because the two mount differently** and that
/// difference deserves a name rather than an argument position: the remote is `merge`d at `/`
/// because its own paths are already root-relative, and the owner's page is `nest`ed under
/// `/admin` because its paths are not.
///
/// Taken as an argument for [`router_with`]'s reason: a `Router` is not `Debug` so it cannot live in
/// [`crate::ApiConfig`], and nothing in a *handler* needs it — it is a fact about how this service is
/// assembled, and this is the function that assembles it.
#[derive(Default)]
pub struct Extras {
    /// The singer's remote, merged at `/`.
    ///
    /// `None` serves [`landing`] there instead, which is what `api.serve_remote = false` asks for.
    pub remote: Option<Router>,
    /// The owner's page, nested under `/admin`.
    ///
    /// Nested **before** the remote is merged, so the remote's own routing can never shadow it.
    pub admin: Option<Router>,
    /// The watch page and the stream beneath it, merged at the root.
    ///
    /// `None` on a machine that draws on a television instead, which is every machine that is not
    /// streaming — so the page and the playlist are genuinely absent rather than answering with an
    /// explanation, the shape `Power is a capability of the host` already settles for a capability
    /// a host may not have.
    ///
    /// Merged rather than nested, like the remote and for the same reason: its paths are already
    /// root-relative, `/watch` and `/stream/...`. Merged **before** the remote, so the remote's
    /// catch-all cannot shadow either. See [`km_api::watch`](crate::watch).
    pub stream: Option<Router>,
}

/// The same, with the singer-facing remote mounted at `/`.
///
/// **A `Router` and not a directory.** M6 reserved a `ServeDir` seam here, on the assumption that
/// what would eventually arrive was a built client-side application to drop in as files. What
/// arrived renders HTML in this same process (`km-remote-pages`), so the seam it was shaped for does
/// not fit it, and keeping both would leave two mechanisms that can disagree about what `/` is.
///
/// The rest of what that seam reserved held up and is used unchanged: same-origin so CORS never
/// enters into it, `cors_origins` for developing against a separate dev server, the password gating
/// exactly as it is, and mDNS plus the on-screen QR code getting a phone here with no typing.
pub fn router_with(state: ApiState, extras: Extras) -> Router {
    let config = state.config().clone();

    let mut app = Router::new()
        // **`state.power()` is read here, while the router is being built**, which is why
        // `set_power` has to be called before serving: a capability installed later would leave a
        // machine that can power itself off with no route saying so.
        .nest(
            API_PREFIX,
            api_router(
                &config,
                Capabilities {
                    power: state.power().is_some(),
                    logs: state.log_tap().is_some(),
                    output_level: state.controller().output_level_supported(),
                },
            ),
        )
        // An unversioned path is a client that guessed at the prefix. Same reasoning.
        .route("/api/{*rest}", axum::routing::any(unknown_endpoint))
        // **The same guard over the mirror, and mounted whether or not the mirror is.** That is the
        // half worth writing down: with the console off, `/dev/api/v1/state` had nothing above it
        // and fell through to the root fallback, so a JSON client got an HTML landing page with a
        // 200 and tried to parse it. Not hypothetical — the console's own address box lets somebody
        // point it at a machine whose console is off, and then *every* call did that. A 404 saying
        // this is not an endpoint is the answer, exactly as for `/api/...`.
        .route("/dev/api/{*rest}", axum::routing::any(unknown_endpoint));

    // **The same routes again, where nothing asks for a password.** Mounted *before* the page below,
    // because that arm may be a `ServeDir` at `/dev` — which claims `/dev/{*rest}` — and a static
    // segment must be declared where axum can prefer it. See `DEV_API_PREFIX`.
    if dev_console_served(&config) {
        tracing::info!("serving the development console's own API at {DEV_API_PREFIX}/");
        // **The power routes are the one part of the surface this mirror does not get**, and the
        // log routes are here to make the shape of that exception plain: everything on this mirror
        // being open is the console's whole point, and what is held back is a poweroff nobody can
        // undo from a page rather than anything merely sensitive. Reading a log is not that.
        app = app.nest(
            DEV_API_PREFIX,
            api_router(
                &config,
                Capabilities {
                    power: false,
                    logs: state.log_tap().is_some(),
                    output_level: state.controller().output_level_supported(),
                },
            ),
        );
    }

    let mut app = app
        // Both pages below point at this, and so will the end-user remote. A route rather than a
        // per-page `data:` URI; see `ICON_PNG`.
        .route(ICON_PATH, get(icon))
        .with_state(state.clone());

    // **The gate goes on the outer router, and that is the one placement that works.** Inside the
    // `nest` above, axum has already stripped `/api/v1` from the path a layer sees, so a middleware
    // there would be testing `/admin/demo` against a rule written about `/api/v1/admin/demo`. Out
    // here the path is whole. It is also harmless to the pages below: they live at `/admin/`, which
    // is not under the API prefix, and they carry their own guard.
    //
    // It sees the dev mirror's paths too, whole, and lets every one of them through — `/dev/api/v1`
    // is not `/api/v1/admin`. The layer needs no exception because the rule already has none.
    app = app.layer(from_fn(move |request: Request, next: Next| {
        let state = state.clone();
        async move {
            let path = request.uri().path().to_owned();
            if let Err(error) = state.authorize(&path, request.headers()) {
                return error.into_response();
            }
            next.run(request).await
        }
    }));

    if dev_console_served(&config) {
        match &config.dev_remote_dir {
            // A directory on disk wins, so the page can be edited and reloaded without rebuilding
            // while somebody is working on it.
            Some(dir) if dir.join("index.html").is_file() => {
                tracing::info!(dir = %dir.display(), "serving the development console at /dev/");
                app = app.nest_service("/dev", tower_http::services::ServeDir::new(dir));
            }
            // Otherwise the copy compiled into the binary. It is one dependency-free file that
            // fetches nothing, so there is nothing else for a folder beside the executable to carry
            // — and a machine that shipped without that folder used to log a warning and serve
            // nothing at the only URL an owner has to drive it from a phone.
            _ => {
                tracing::info!("serving the built-in development console at /dev/");
                app = app
                    .route("/dev", get(dev_remote))
                    .route("/dev/", get(dev_remote))
                    .route("/dev/index.html", get(dev_remote));
            }
        }
    }

    // The owner's page, nested. **Before the remote is merged and not after**, so the remote's
    // routing can never shadow `/admin` -- and it could: it is merged at the root and owns whatever
    // the API did not claim.
    //
    // `nest` and not `merge`, unlike the remote below, because this page's paths are relative to
    // `/admin` rather than to the root. Note it is unrelated to `/api/v1/admin/...`, which is under
    // the API prefix -- the names collide and the paths do not.
    if let Some(admin) = extras.admin {
        tracing::info!("serving the owner's page at /admin/");
        app = app
            .nest(ADMIN_PAGE_PATH, admin)
            // **`nest` matches `/admin` and not `/admin/`**, and the trailing slash is what a person
            // types and what every link ending in a directory produces -- so without this, the URL
            // this feature is described by everywhere answers 404. Measured rather than assumed:
            // `km-admin-pages`' own tests drive both spellings.
            //
            // **Temporary, because a browser keeps a permanent one.** A cached redirect is followed
            // without this server being asked, so a landing path that redirects permanently is a
            // promise about every later build.
            .route(
                &format!("{ADMIN_PAGE_PATH}/"),
                get(|| async { axum::response::Redirect::temporary(ADMIN_PAGE_PATH) }),
            );
    }

    // The watch page and the stream beneath it. Before the remote for the reason `/admin` is:
    // the remote owns whatever the root has not already claimed, and `/watch` is not a song.
    if let Some(stream) = extras.stream {
        tracing::info!(
            "serving the stream at {} and its page at {}",
            crate::watch::STREAM_PREFIX,
            crate::watch::WATCH_PATH,
        );
        app = app.merge(stream);
    }

    // The singer-facing remote, at the root. `merge` and not `nest`, because its own paths are
    // already root-relative -- `/now`, `/queue`, `/static/app.css` -- and because merging is what
    // keeps `/api/...` reachable: the API's routes are declared above and win, and the remote fills
    // in the rest.
    //
    // Its own 404 does *not* become the fallback. `unknown_endpoint` is still what an unknown
    // `/api/...` path gets, which is the one thing a JSON client must not have answered with HTML.
    app = match extras.remote {
        Some(remote) => {
            tracing::info!("serving the remote at /");
            app.merge(remote)
        }
        None => app.fallback(landing),
    };

    if !config.cors_origins.is_empty() {
        // Only when asked for. The remote is served from this same origin, so in the shipped
        // configuration CORS never enters into it; this exists for developing a remote against a
        // separate dev server.
        let mut layer = tower_http::cors::CorsLayer::new()
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any);
        for origin in &config.cors_origins {
            match origin.parse::<axum::http::HeaderValue>() {
                Ok(value) => layer = layer.allow_origin(value),
                Err(_) => tracing::warn!(origin, "ignoring an unparseable CORS origin"),
            }
        }
        app = app.layer(layer);
    }

    app.layer(tower_http::trace::TraceLayer::new_for_http())
}

/// The API itself, without the state, so it can be mounted twice.
///
/// **Built by a function rather than cloned, and the reason is the second mount.** `router_with`
/// puts this under [`API_PREFIX`], where the layer above it gates `/admin/`, and again under
/// [`DEV_API_PREFIX`], where nothing does. Cloning a `Router` would do as well; a function says
/// plainly that there is one definition of the surface and two places it is mounted, which is the
/// property that matters — a route added to one and not the other is the drift the dev console has
/// already suffered twice.
/// **`power` is the one thing this does *not* mount identically in both places**, and it is the
/// single exception to the sentence above. Everything else on this surface is a change somebody can
/// undo from the same page; turning the box off cannot be undone from any page, only by walking to
/// the machine and pressing its button. An owner who switched debugging and the console on opted
/// into a diagnostic surface, not into letting anybody on the network end the evening — so the
/// caller passes `false` for the mirror and the power routes exist only where the password does.
/// `the_dev_mirror_does_not_carry_the_power_routes` is what keeps that true.
///
/// The log routes are the same kind of thing and go the other way, which is why
/// [`Capabilities`] names its two fields rather than being a pair of bools: they are read from the
/// same state and mounted differently, and a reader should not have to count arguments to see it.
fn api_router(config: &crate::ApiConfig, capabilities: Capabilities) -> Router<ApiState> {
    let public = Router::new()
        .route("/discover", get(handlers::discover))
        .route("/connect", get(handlers::connect))
        .route("/state", get(handlers::get_state))
        .route("/songs", get(handlers::search_songs))
        // Before `/songs/{number}`: `export` is a word, not a code. Routing prefers a static
        // segment over a parameter, so the order here is documentation rather than a load-bearing
        // detail — but the same mistake in `km-package-builder` cost an afternoon, so it is written
        // down.
        //
        // **Now that a code may begin with letters this is worth one more sentence**, because the
        // obvious worry is that `export` might parse as one. It cannot: a code is letters *then
        // digits*, and `export` has no digits, so `SongCode` refuses it. The static route wins
        // regardless; both facts would have to change together for this to break.
        .route("/songs/export", get(handlers::export_songs))
        // Here for the same reason, and it survives the same test: `book.pdf` is letters and a dot
        // and no digits, so `SongCode::from_str` refuses it outright.
        .route("/songs/book.pdf", get(handlers::song_book))
        .route("/songs/{number}", get(handlers::get_song))
        .route("/songs/{number}/lyrics", get(handlers::get_lyrics))
        .route(
            "/queue",
            get(handlers::get_queue)
                .post(handlers::add_to_queue)
                .delete(handlers::clear_queue),
        )
        .route("/queue/{entry_id}", delete(handlers::remove_from_queue))
        .route("/queue/{entry_id}/move", post(handlers::move_in_queue))
        .route("/transport/play", post(handlers::play))
        .route("/transport/pause", post(handlers::pause))
        .route("/transport/skip", post(handlers::skip))
        .route("/transport/restart", post(handlers::restart))
        .route("/transport/stop", post(handlers::stop))
        .route("/transport/seek", post(handlers::seek))
        // **There is no `/settings/transpose` or `/settings/melody`.** Either would be `PUT /settings`
        // with one field, reaching the same `apply_settings` through a second field spelling —
        // `semitones` for what the patch calls `transpose`, `enabled` for `melody_enabled`. See
        // `One spelling per concept, across every surface` in docs/decisions/.
        //
        // **A key change is public**, beside search and queueing: it is a performance knob a
        // singer reaches for, not installation configuration.
        .route(
            "/settings",
            get(handlers::get_settings).put(handlers::put_settings),
        )
        .route("/mics", get(handlers::get_mics))
        .route("/mics/{id}", put(handlers::put_mic))
        // Reading what the machine's sound is coming out of, and what it is coming out *as*. The
        // three writes that answer the same question live under `/admin/audio/`.
        .route("/audio/outputs", get(handlers::get_audio_outputs))
        .route("/audio/soundfont", get(handlers::get_audio_soundfont))
        .route("/audio/soundfonts", get(handlers::get_audio_soundfonts))
        .route("/wallpapers", get(handlers::get_wallpapers))
        // Anybody in the room may reasonably change the picture; putting a new one into the
        // rotation is the owner's business and lives under `/admin/`.
        .route("/wallpapers/next", post(handlers::next_wallpaper))
        // What language the television draws in. Reading it is public, on the footing `/demo` and
        // `/debug` below share; `PUT /admin/machine/locale` is where it is written.
        .route("/locale", get(handlers::get_locale))
        .route("/demo", get(handlers::get_demo))
        // **A path of its own rather than a `POST /demo`.** That would read as creating demo mode,
        // which is what `PUT /admin/demo` already does; this starts one song and turns nothing on.
        // Public, because asking for one demo song is a guest's business.
        .route("/demo/start", post(handlers::start_demo))
        .route("/packages", get(handlers::get_packages))
        // Reporting whether debugging is on is public; turning it on is not. Using the answer and
        // changing the state are different acts — the same split `/demo` makes one screen over.
        .route("/debug", get(handlers::get_debug))
        // Whether the development console is up, and why it might not be. Public for the same
        // reason, and it carries both switches because one of them alone explains nothing.
        .route("/dev-remote", get(handlers::get_dev_remote))
        // What the frame meter is measuring, if it is drawing. Public read, admin write, the same
        // split as the two switches above -- and the only one of the three that needs no restart.
        .route("/performance", get(handlers::get_performance))
        .route("/events", get(handlers::events));

    // Every path here demands a token, by virtue of where it is. `login` is the documented
    // exception and `needs_admin_token` is what implements it.
    let mut admin = Router::new()
        .route("/login", post(handlers::login))
        .route("/logout", post(handlers::logout))
        .route("/password", post(handlers::set_admin_password))
        // Sign out everywhere. Password-independent on purpose: an owner who wants every phone
        // logged out should not have to change the password and then tell the house the new one.
        .route("/sessions/reset", post(handlers::reset_sessions))
        .route("/audio/output", put(handlers::put_audio_output))
        .route("/audio/soundfont", put(handlers::put_audio_soundfont))
        // Fetching and uploading are the same act from a person's side — "have this bank" — and
        // differ only in where the bytes come from, so they sit together.
        .route(
            "/audio/soundfont/fetch",
            post(handlers::post_audio_soundfont_fetch),
        )
        .route(
            "/audio/soundfonts",
            post(handlers::upload_soundfont)
                .layer(DefaultBodyLimit::max(handlers::MAX_SOUNDFONT_BYTES)),
        )
        // **Removing one hangs off the plural, and that is not a stylistic choice.** The singular
        // `/audio/soundfont` is *the bank in force* while `/audio/soundfonts` is the collection, and
        // deleting a member of a collection belongs on the collection. Hanging it off the singular
        // would also put `{id}` beside the static `fetch` above, so a bank whose file slugged to
        // `fetch` could never be deleted: the static segment wins the match and answers 405.
        // Unlikely, and silent, which is the bad combination.
        .route(
            "/audio/soundfonts/{id}",
            delete(handlers::delete_audio_soundfont),
        )
        .route(
            "/wallpapers",
            post(handlers::upload_wallpaper)
                .layer(DefaultBodyLimit::max(handlers::MAX_WALLPAPER_BYTES)),
        )
        .route("/wallpapers/{id}", delete(handlers::delete_wallpaper))
        // No `GET` twin, and that is not an omission: `/discover` is always public and always
        // carries the name, so the read side was built before the write side and by somebody else.
        .route("/machine/name", put(handlers::put_machine_name))
        // Beside the name because it is the same kind of fact about this machine. Its read side is
        // the public `GET /locale` above, `/discover` carrying no locale.
        .route("/machine/locale", put(handlers::put_machine_locale))
        .route("/demo", put(handlers::put_demo))
        // **A path under the switch rather than a field in its body**, because the two answer
        // different questions: the switch is *tonight or for good* and the delay is installation
        // configuration that is always written down. Admin because zero is a legal delay, so this
        // is the switch reached by another door — see `handlers::put_demo_delay`.
        .route("/demo/delay", put(handlers::put_demo_delay))
        // **Install is admin.** It names a path on the machine's disk and installs whatever is there,
        // which is the one write a stranger must not be able to perform; `km-package-builder`
        // keeps a password of its own, so nothing about it needs this public.
        .route("/packages", post(handlers::install_package))
        // **A path of its own rather than a content-type branch on the route above.** That one means
        // "install what is already at this path"; this means "here are the bytes".
        //
        // **Both static segments sit before `/packages/{id}`**, so neither can be shadowed by a
        // package whose id happens to be `upload` or `rescan`.
        .route(
            "/packages/upload",
            post(handlers::upload_package)
                .layer(DefaultBodyLimit::max(handlers::MAX_PACKAGE_BYTES)),
        )
        .route("/packages/rescan", post(handlers::rescan_packages))
        .route("/packages/{id}", delete(handlers::uninstall_package))
        .route("/packages/{id}/bank", put(handlers::set_package_bank))
        // Turning debugging on, which is what mounts the two routes in `DEBUG_SURFACE`. An owner's
        // act, so it is here; using them is not, so they are not.
        .route("/debug", put(handlers::put_debug))
        // Turning the development console on, which — with the switch above — is what mounts a
        // second copy of this whole surface with no password on any of it.
        .route("/dev-remote", put(handlers::put_dev_remote))
        // Drawing the frame statistics over the picture. Effective on the next frame, so this is
        // the one switch here whose reply a caller should read back.
        .route("/performance", put(handlers::put_performance));

    // Turning the box off, and starting the application again. Mounted only where the host supplied
    // a way to do either, so a machine that cannot answers 404 on all three — the shape the
    // `Controller` trait's own note prescribes for a capability a host does not have, and the one
    // `debug/play-file` already takes.
    //
    // **Read here, while the router is being built**, which is why `set_power` has to be called
    // before serving: a capability installed later would leave a machine that can power itself off
    // with no route saying so.
    if capabilities.power {
        admin = admin
            .route("/power", get(handlers::get_power))
            // Two paths under `/power/` rather than one taking a verb in its body. They are
            // different acts with different consequences — one ends the evening and one interrupts
            // it for ten seconds — and a body field would make the difference invisible in a log.
            .route("/power/off", post(handlers::power_off))
            .route("/power/restart", post(handlers::power_restart));
    }

    // What the machine has said about itself, for a surface that shows it. Mounted only where the
    // program running this installed a tap — and read here, while the router is built, for the
    // reason above.
    //
    // Admin because a log names paths, addresses and the machine's own name. Which is also the
    // standing constraint this route puts on everything else: anything ever written to the log
    // becomes readable by whoever can reach a machine with the development console on, so a line
    // that would say a password out loud is a fault wherever it is written, not here.
    if capabilities.logs {
        admin = admin
            .route("/logs", get(handlers::get_logs))
            // A path of its own rather than an upgrade on `/logs`, so that what a reader gets is
            // decided by the URL it asked for rather than by a header it remembered to send.
            .route("/logs/stream", get(handlers::logs_stream));
    }

    // The level the chosen output runs at, where the operating system has a mixer to move. Mounted
    // for the same reason and in the same place as the two above: a host with nothing to ask
    // answers 404, rather than mounting a route that could only ever refuse.
    //
    // **Whether the active device has a level is a different question**, answered per device by
    // `AudioOutputs::level`, so this gate being open does not promise the route will succeed.
    if capabilities.output_level {
        admin = admin.route("/audio/level", put(handlers::put_audio_level));
    }

    let mut api = public.nest("/admin", admin);

    if config.debug_enabled {
        // The body **is** the song, so the 2 MB axum puts on every request is the wrong limit here:
        // it would refuse every MP3+G pair and every video. The layer sits on this one route rather
        // than on the router, because nothing else on this surface has any business being large.
        api = api
            .route("/debug/play-file", post(handlers::play_file))
            .route(
                "/debug/play-upload",
                post(handlers::play_upload)
                    .layer(DefaultBodyLimit::max(handlers::MAX_AUDITION_BYTES)),
            );
    }

    // Without this, an unknown path under the prefix would fall through to the root fallback and
    // answer a *client* with an HTML page and a 200 — which it would then try to parse as JSON.
    // A distinct code, because "you called a path that does not exist" and "the song you asked
    // for does not exist" are different problems and only one of them is fixable by the client.
    api.fallback(unknown_endpoint)
}

/// Answers a path that is not an endpoint.
///
/// The code is [`ApiError::UNKNOWN_ENDPOINT`], distinct from the `not_found` a real missing song or
/// queue entry produces, so a client — and the surface sweep in the tests — can tell "this route
/// does not exist" from "the thing you named does not exist".
async fn unknown_endpoint(method: axum::http::Method, uri: axum::http::Uri) -> Response {
    // **The prefix the caller used, not always `API_PREFIX`.** A path under the dev mirror that is
    // not a route was being told the API lives somewhere it had not asked about — which is unhelpful
    // in the one case it comes up most: the console pointed at a machine whose console is off, where
    // every call lands here and the answer *is* the diagnosis.
    let path = uri.path();
    let prefix = if path.starts_with(DEV_API_PREFIX) {
        DEV_API_PREFIX
    } else {
        API_PREFIX
    };
    ApiError::UnknownEndpoint(format!(
        "{method} {path} is not an endpoint; the API lives under {prefix}"
    ))
    .into_response()
}

/// The development remote, compiled in.
///
/// Embedded rather than shipped as a file because it is exactly the kind of thing that should not be
/// losable: one self-contained HTML page, no dependencies, nothing fetched from the network. A build
/// that forgot to stage it served the landing page at `/dev/` instead, which looks like the feature
/// is off rather than like a file is missing.
///
/// 30 KB against a 12 MB executable, and `cargo` rebuilds when the file changes.
const DEV_REMOTE_HTML: &str = include_str!("../../../../tools/dev/remote/index.html");

/// Serves the built-in development remote.
async fn dev_remote() -> Response {
    Html(DEV_REMOTE_HTML).into_response()
}

/// The application icon, compiled in, for the browser tab.
///
/// The same file `km-display` embeds as the window icon, at favicon size. Served as a route rather
/// than inlined into each page as a `data:` URI, which was the other way to keep the dev remote a
/// single self-contained file: a `data:` URI would have added a kilobyte of base64 to a page that is
/// meant to stay readable, and a missing favicon is a benign failure in a way a missing stylesheet is
/// not. It is still compiled in, so no build can ship without it.
const ICON_PNG: &[u8] = include_bytes!("../../../../icon/icon-32.png");

/// Where that icon is, for the pages that link to it.
///
/// **One spelling, because a page naming a path this router does not serve is a fault nothing else
/// reports**: a browser asks for a favicon, gets a 404, and draws its blank sheet. The landing page
/// below spells it out instead, `concat!` taking literals and nothing else.
pub const ICON_PATH: &str = "/icon.png";

/// Serves the icon for whatever page asked for it.
///
/// Public and unauthenticated, like the landing page: a browser fetches a favicon before anybody has
/// logged in to anything, and an icon behind a password is an icon nothing ever shows.
async fn icon() -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, "image/png"),
            // It changes only when the app is rebuilt, and a tab icon that flickers on every
            // navigation looks like a fault.
            (axum::http::header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        ICON_PNG,
    )
        .into_response()
}

/// What the root serves when no end-user remote is installed.
async fn landing() -> Response {
    // Deliberately plain and self-contained: no fonts, no scripts, nothing to fetch. Whoever is
    // looking at this is diagnosing something.
    (
        StatusCode::OK,
        Html(concat!(
            "<!doctype html><meta charset=utf-8>",
            "<meta name=viewport content='width=device-width,initial-scale=1'>",
            "<title>KaraokeMachine</title>",
            "<link rel=icon href=/icon.png>",
            "<style>body{font:16px/1.5 system-ui,sans-serif;margin:3rem auto;max-width:34rem;",
            "padding:0 1.5rem}code{background:#0001;padding:.15em .4em;border-radius:.25em}",
            "@media(prefers-color-scheme:dark){body{background:#111;color:#eee}",
            "code{background:#fff2}}</style>",
            "<h1>KaraokeMachine</h1>",
            "<p>This machine is running and reachable. No singer-facing remote is installed here ",
            "yet.</p>",
            "<ul>",
            "<li>The API is at <code>/api/v1</code> — start with <code>/api/v1/discover</code>.</li>",
            "<li>The owner's page is at <code>/admin/</code>.</li>",
            "<li>The development console is off unless it is asked for, by ",
            "<code>--dev-remote</code> for one run or <code>api.serve_dev_remote</code> in ",
            "settings. It is then at <code>/dev/</code>.</li>",
            "</ul>",
            "<p>Queue a song at the machine itself by typing its number.</p>",
        )),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three paths this crate hands to a client are paths this crate actually mounts.
    ///
    /// **The half of the guard that lives where a route moves.** `km_api::uploads::path_for` exists
    /// so `km-admin` and `km-package-builder` stop spelling these by hand, and a constant nobody
    /// checks is only a fourth spelling — so the table that proves the surface is mounted proves
    /// the client's copy of it too, on every push to this workspace.
    ///
    /// **The verb is not decoration.** `/wallpapers` and `/audio/soundfonts` are both in [`SURFACE`]
    /// as `GET`, so a test asking only whether the path appears would pass while two of the three
    /// went to a route that answers 405. The pair is the assertion.
    #[test]
    fn the_upload_paths_clients_are_given_are_really_mounted() {
        for kind in [
            crate::machine::Upload::Package,
            crate::machine::Upload::Wallpaper,
            crate::machine::Upload::SoundFont,
        ] {
            let path = crate::uploads::path_for(kind);
            assert!(
                SURFACE.contains(&("POST", path)),
                "{kind:?} is sent to {path}, which is not mounted as a POST"
            );
            assert!(
                needs_admin_token(&format!("{API_PREFIX}{path}")),
                "{kind:?} is sent to {path}, which is outside the admin prefix"
            );
        }
    }

    /// The whole permission system, in one table. If this is wrong, everything is.
    #[test]
    fn the_prefix_decides_and_login_is_the_one_exception() {
        for path in [
            "/api/v1/admin/demo",
            "/api/v1/admin/password",
            "/api/v1/admin/packages",
            "/api/v1/admin/packages/vol1/bank",
            "/api/v1/admin/sessions/reset",
            "/api/v1/admin/logout",
        ] {
            assert!(needs_admin_token(path), "{path} must demand a token");
        }
        for path in [
            ADMIN_LOGIN_PATH,
            "/api/v1/discover",
            "/api/v1/queue",
            "/api/v1/settings",
            "/api/v1/demo",
            "/api/v1/debug",
            "/api/v1/debug/play-file",
            "/admin/songs",
            "/",
        ] {
            assert!(!needs_admin_token(path), "{path} must not demand a token");
        }
    }

    /// A prefix test without the trailing slash would gate this, and would silently gate a future
    /// `/api/v1/administration` that nobody had decided to protect.
    #[test]
    fn a_path_that_merely_starts_with_the_word_admin_is_not_an_admin_path() {
        assert!(!needs_admin_token("/api/v1/adminfoo"));
        assert!(!needs_admin_token("/api/v1/administration"));
        assert!(!needs_admin_token("/api/v1/admin"));
    }

    /// **Nothing under the mirror asks for a token, and this is the assertion that says so out
    /// loud.**
    ///
    /// It is already true by arithmetic — `/dev/api/v1/admin/...` does not begin with
    /// `/api/v1/admin` — and that is exactly why it needs pinning: the behaviour rests on the shape
    /// of a `strip_prefix` rather than on anything a reader would notice. Loosen the rule above to
    /// a `contains("/admin/")` and the whole point of the mirror closes with every other test still
    /// green.
    ///
    /// The whole of [`SURFACE`] is swept, because a route added under `/admin/` tomorrow must land
    /// open here without anybody remembering to come back. [`LOG_SURFACE`] is swept with it, being
    /// the conditional surface that *is* mirrored — [`POWER_SURFACE`] is deliberately absent.
    #[test]
    fn no_path_under_the_dev_mirror_demands_a_token() {
        for (_, path) in SURFACE.iter().chain(DEBUG_SURFACE).chain(LOG_SURFACE) {
            let mirrored = format!("{DEV_API_PREFIX}{path}");
            assert!(
                !needs_admin_token(&mirrored),
                "{mirrored} demands a token; the mirror is supposed to have no permissions at all"
            );
        }
        // Spelled concretely as well as swept, so a reader of this test can see what it is about
        // without resolving a constant against a table.
        assert!(!needs_admin_token("/dev/api/v1/admin/password"));
        assert!(!needs_admin_token("/dev/api/v1/admin/login"));
        // And the real prefix is untouched, which is the other half of the claim.
        assert!(needs_admin_token("/api/v1/admin/password"));
    }

    /// Both switches, and the truth table written out.
    ///
    /// Cheap to the point of looking redundant, and it is the rule the whole no-password surface
    /// rests on: an edit turning the `&&` into an `||` would open the mirror on a machine whose
    /// owner had only ever ticked one box.
    #[test]
    fn the_console_is_served_only_when_both_switches_are_on() {
        let mut config = crate::ApiConfig::default();
        for (debug, dev_remote, expected) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, true),
        ] {
            config.debug_enabled = debug;
            config.serve_dev_remote = dev_remote;
            assert_eq!(
                dev_console_served(&config),
                expected,
                "debug={debug} dev_remote={dev_remote}"
            );
        }
    }

    /// Every call the development console makes is a route this crate mounts.
    ///
    /// **The guard `km-admin` has had for months and the console has never had, and both drift
    /// episodes it would have caught were found by hand.** Install and two package routes moved
    /// under `/admin/` and the page kept calling the old paths; a later reading found three more,
    /// including two sub-paths of `/settings` that had stopped existing altogether. Nothing failed —
    /// the page just logged a 404 or a 405 into its own pane, on a surface whose whole job is to be
    /// the thing you check the API with.
    ///
    /// The page's paths are string literals in HTML, so this parses them out rather than asking a
    /// type: `api("METHOD", "/path")` and its backtick form. Two shapes need normalising, and both
    /// are in the file today — a `${…}` interpolation is a path parameter, and a literal ending in
    /// `/` is a segment concatenated on afterwards (`"/transport/" + action`). Both become "any
    /// segment", matched against `SURFACE`'s concrete samples position by position.
    #[test]
    fn every_call_the_development_console_makes_is_a_route_this_crate_mounts() {
        /// Whether a page segment matches a sample segment.
        fn segment_matches(from_page: &str, sample: &str) -> bool {
            // A parameter, or a segment built by concatenation: anything the sample has will do.
            from_page.is_empty() || from_page.contains("${") || from_page == sample
        }

        let calls = console_calls(DEV_REMOTE_HTML);
        // A parser that silently matched nothing would make this test pass forever. The count is a
        // floor rather than an equality, so adding a call to the page is not a failure here.
        assert!(
            calls.len() > 30,
            "only {} calls were parsed out of the console; the parser is broken, not the page",
            calls.len()
        );

        for (method, path) in &calls {
            let wanted: Vec<&str> = path.split('/').collect();
            let found = SURFACE
                .iter()
                .chain(DEBUG_SURFACE)
                .chain(LOG_SURFACE)
                .chain(LEVEL_SURFACE)
                .any(|(m, sample)| {
                    m == method
                        && sample.split('/').count() == wanted.len()
                        && sample
                            .split('/')
                            .zip(&wanted)
                            .all(|(sample, page)| segment_matches(page, sample))
                });
            assert!(
                found,
                "the console calls {method} {path}, which is in none of SURFACE, DEBUG_SURFACE, LOG_SURFACE or LEVEL_SURFACE"
            );
        }
    }

    /// The `(method, path)` pairs in the console's `api(...)` calls.
    ///
    /// Hand-written rather than a regex, which would be a dependency for twenty lines. A path is
    /// truncated at `?` — a query string is not part of a route — and a trailing `/` is kept as an
    /// empty final segment, which is what says "a segment is concatenated on here".
    fn console_calls(html: &str) -> Vec<(String, String)> {
        let mut calls = Vec::new();
        let mut rest = html;
        while let Some(at) = rest.find("api(\"") {
            rest = &rest[at + "api(\"".len()..];
            let Some((method, after)) = rest.split_once('"') else {
                break;
            };
            rest = after;
            let Some(after_comma) = after.strip_prefix(", ") else {
                continue;
            };
            let mut characters = after_comma.chars();
            let Some(quote) = characters.next() else {
                break;
            };
            if quote != '"' && quote != '`' {
                // `api(method, path)` with the path in a variable. There are none today; if one
                // arrives it is invisible to this sweep, which is worth knowing rather than
                // guessing at.
                continue;
            }
            let body = &after_comma[quote.len_utf8()..];
            let Some(end) = body.find(quote) else {
                break;
            };
            let path = &body[..end];
            let path = path.split('?').next().unwrap_or(path);
            if path.starts_with('/') {
                calls.push((method.to_owned(), path.to_owned()));
            }
        }
        calls
    }

    #[test]
    fn every_surface_entry_names_a_real_method_and_an_absolute_path() {
        for (method, path) in SURFACE
            .iter()
            .chain(DEBUG_SURFACE)
            .chain(POWER_SURFACE)
            .chain(LOG_SURFACE)
        {
            assert!(
                ["GET", "POST", "PUT", "DELETE"].contains(method),
                "{path}: odd method {method}"
            );
            assert!(path.starts_with('/'), "{path} is not absolute");
            assert!(!path.contains('{'), "{path} still has a placeholder");
        }
    }

    /// The surface and the gate have to agree, and this is what makes them. A route added under
    /// `/admin/` is gated by construction; one added outside it is public by construction; and this
    /// asserts that the sample paths in `SURFACE` really do land the way their prefix says.
    #[test]
    fn the_surface_agrees_with_the_gate_about_every_path() {
        for (_, path) in SURFACE {
            let full = format!("{API_PREFIX}{path}");
            let under_admin = path.starts_with("/admin/");
            let gated = needs_admin_token(&full);
            if full == ADMIN_LOGIN_PATH {
                assert!(!gated, "login must stay reachable without a token");
                continue;
            }
            assert_eq!(
                under_admin, gated,
                "{full}: filed under /admin/ is {under_admin}, gated is {gated}"
            );
        }
    }

    /// The debug routes are public when they exist at all. If one of these ever moved under
    /// `/admin/` the mode would stop meaning anything, because the password would gate it anyway.
    #[test]
    fn the_debug_routes_are_never_admin_routes() {
        for (_, path) in DEBUG_SURFACE {
            let full = format!("{API_PREFIX}{path}");
            assert!(
                !needs_admin_token(&full),
                "{full} must be public when mounted"
            );
        }
    }

    /// The mirror image of the assertion above it, and the reason both are written out rather than
    /// derived: the two conditional surfaces are conditional for opposite reasons. Debugging is a
    /// mode, so its routes are public when they exist; power is a capability, so its routes are the
    /// owner's when they exist. A power route that slipped out from under `/admin/` would let
    /// anybody on the LAN switch the television off.
    #[test]
    fn the_power_routes_are_always_admin_routes() {
        for (_, path) in POWER_SURFACE {
            let full = format!("{API_PREFIX}{path}");
            assert!(
                needs_admin_token(&full),
                "{full} must demand a token when mounted"
            );
        }
    }

    /// The third conditional surface, and the third reason for being one. Debugging is a mode, so
    /// its routes are public when they exist; power is a capability of the box, so its routes are
    /// the owner's; a log is a capability too, and what it holds is the owner's business — paths,
    /// addresses and the name they gave the machine.
    #[test]
    fn the_log_routes_are_always_admin_routes() {
        for (_, path) in LOG_SURFACE {
            let full = format!("{API_PREFIX}{path}");
            assert!(
                needs_admin_token(&full),
                "{full} must demand a token when mounted"
            );
        }
    }

    /// **The one place the two capabilities' difference is asserted rather than described.** Both
    /// are absent-or-admin, and they part company on the mirror: a poweroff nobody can undo from a
    /// page is held back from it, and reading a log is not that. Written out because the difference
    /// lives in two `Capabilities` literals that a later edit could quietly make agree.
    #[test]
    fn the_dev_mirror_carries_the_log_routes_and_not_the_power_routes() {
        for (_, path) in LOG_SURFACE {
            assert!(
                !needs_admin_token(&format!("{DEV_API_PREFIX}{path}")),
                "{path} is on the mirror, where nothing asks for a password"
            );
        }
        // Not a statement about the gate — nothing under the mirror is gated — but about what is
        // mounted there at all. `the_dev_mirror_does_not_carry_the_power_routes` drives real
        // requests to prove it; this is the reminder beside its opposite.
        assert!(
            POWER_SURFACE
                .iter()
                .all(|(_, path)| !LOG_SURFACE.iter().any(|(_, log)| log == path)),
            "the two conditional admin surfaces must not share a path"
        );
    }

    #[test]
    fn the_api_prefix_is_the_one_discovery_advertises() {
        assert_eq!(API_PREFIX, crate::discover::API_BASE);
    }

    #[test]
    fn the_admin_prefix_sits_under_the_api_prefix() {
        assert!(ADMIN_PREFIX.starts_with(API_PREFIX));
        assert!(ADMIN_LOGIN_PATH.starts_with(ADMIN_PREFIX));
    }
}
