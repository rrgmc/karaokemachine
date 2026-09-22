//! The singer-facing remote.
//!
//! One set of templates and handlers, serving two modes:
//!
//! * **Online** — linked into `km-app` and served at `/` by the machine itself. Search, queue, now
//!   playing, and the controls a song allows. Nothing else, and that is the whole design: the
//!   machine is an appliance under a television, so a favorites collection living there would be a
//!   shared list nobody owns.
//! * **Offline** — linked into `km-remote`, which holds its own mirror of the catalog and adds
//!   favorites with folders and the A–Z strip. It browses and searches with the karaoke machine
//!   switched off, which is the reason it exists.
//!
//! The difference between them is [`Capabilities`] and the implementations behind
//! [`machine`]'s three traits. **The templates branch on capabilities, never on a mode**, so moving
//! one feature between the modes is a line here rather than an edit to the markup.
//!
//! This is server-rendered HTML with htmx, not a JSON client — the same shape as
//! `tools/cmd/km-package-builder`, and for the same reasons its handlers module gives. It is a *library* rather than a directory of
//! files dropped into the machine, because both modes render, and two copies of the markup would be
//! two things to keep in step.

/// This crate's tracing target, for the hosts that name it in a filter string.
///
/// Four of them do — `km-remote`, the Android and iOS shells, and the machine's own `-v` ladder
/// — and none of them can ask cargo for it, because `CARGO_CRATE_NAME` answers for the crate doing
/// the asking. Spelling `km_remote_pages` in four places was a rename away from four filters that
/// name a target which does not exist, which is not an error anywhere and shows up only as `-v`
/// having stopped working.
pub const LOG_TARGET: &str = env!("CARGO_CRATE_NAME");

pub mod backup;
pub mod form;
pub mod handlers;
pub mod machine;
pub mod model;
pub mod prefs;
pub mod share;
pub mod sse;
pub mod views;
pub mod words;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};

use crate::machine::{Connect, Favorites, Machine, Songs};
use crate::sse::Hub;

/// The page's script and stylesheet, compiled into whichever binary links this.
///
/// `include_str!` and not a directory beside the executable, per the `Bundling assets` decision. It
/// matters more here than anywhere: this crate ends up inside the appliance's `.deb`, inside a macOS
/// bundle and inside a portable folder, and a stylesheet that failed to travel would leave a page
/// that renders, responds to nothing a singer taps, and reports no error to anybody.
const HTMX_JS: &str = include_str!("../static/htmx.min.js");
const HTMX_LICENSE: &str = include_str!("../static/htmx-LICENSE.txt");
const APP_CSS: &str = include_str!("../static/app.css");
const LIVE_JS: &str = include_str!("../static/live.js");
/// Reading a shared folder's code with the camera, and feeding a picked file into a form.
///
/// **A quarter of a megabyte of it lands in the machine's binary too**, since this crate is linked
/// by `karaokemachine` as well as by `km-remote` and `include_str!` is unconditional. That is the
/// accepted cost: `Bundling assets` admits an exception only for size on
/// the order of the 31 MiB SoundFont or for a license that forbids redistribution, and jsQR is
/// neither — a page the online mode never draws is not one of the two reasons. Gating it behind a
/// cargo feature would make it the first *compile-time* mode split in a crate whose header says the
/// templates branch on capabilities and never on a mode. Only `share_receive.html` loads it, so no
/// other page parses it. Apache-2.0, so the license travels with it; see `static/README.md`.
const JSQR_JS: &str = include_str!("../static/jsqr.js");
const JSQR_LICENSE: &str = include_str!("../static/jsqr-LICENSE.txt");
const SCAN_JS: &str = include_str!("../static/scan.js");
const PICK_JS: &str = include_str!("../static/pick.js");

/// The machine's icon: the amber microphone, for the remote the machine serves at `/`.
///
/// Reaches into the generated `icon/` directory rather than keeping a copy under `static/`, exactly
/// as `km-package-builder` does — one place defines the icon, so the remote cannot end up showing last
/// month's.
pub const ICON_MACHINE_PNG: &[u8] = include_bytes!("../../../../icon/icon-32.png");

/// The offline remote's icon: the same microphone in the theme's second accent, green.
///
/// **Both marks live here and the shell picks one**, which is the same seam [`Capabilities`] is: a
/// tab belonging to the machine and a tab belonging to the remote on somebody's phone are two
/// programs, and this crate renders both. Exported so that a shell says which it is rather than
/// carrying a second `include_bytes!` and a second path to keep right — there are three shells
/// coming, and the Android and iOS ones will want the same answer `km-remote-core` already gives.
pub const ICON_REMOTE_PNG: &[u8] = include_bytes!("../../../../icon/km-remote-32.png");

/// What this mode can do.
///
/// The templates read these; nothing reads a mode. A control whose capability is off is **absent**
/// rather than disabled — an offline-only feature is not a thing the online remote is withholding,
/// it is a thing that does not exist there, and a grayed-out star invites a tap that can never work.
///
/// This is not the same as a *song's* capabilities. Transpose being unavailable on a video song is
/// reported per song by `NowPlayingDto`, and that control is drawn and disabled, because there the
/// answer really is "not for this one".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Favorites, folders and the star. Offline only.
    pub favorites: bool,
    /// The A–Z strip. Offline only: it needs an indexed folded-initial column, which the mirror
    /// carries and `library.sqlite` does not.
    pub initial_filter: bool,
    /// The Setup tab's machine card, the offline banner and the catalog refresh. Offline only — in
    /// the online mode the machine is this process, so it cannot be absent.
    ///
    /// **This gates the card, never the tab.** Setup exists in both modes; what the online one has
    /// on it is the singer's name and the language, and it would be a page with a hole in it if the
    /// absence of a machine card took the tab with it. What answers the card is [`Remote::connect`].
    pub connection: bool,
    /// A link to the printable song book. **Online only**, and it is the one capability that is on
    /// in that mode and off in the other.
    ///
    /// The book is `GET /api/v1/songs/book.pdf`, which the machine serves and this crate does not.
    /// In the online mode these pages are mounted at the root of that same router, so the link is
    /// same-origin and needs no address; in the offline mode it would point at nothing this process
    /// has. Absent rather than broken, which is what a capability is for.
    pub song_book: bool,
    /// A link to the owner's page at `/admin/`. **Online only**, for [`Self::song_book`]'s reason
    /// exactly: the machine serves that page and this crate does not, so same-origin in one mode
    /// and nothing at all in the other.
    ///
    /// # Why the singer's remote carries a door it cannot open
    ///
    /// **Nothing linked to `/admin/` from anywhere.** The QR code the television draws points at
    /// `/`, this crate mentioned the page only in guard prose, and the connect panel deliberately
    /// says nothing about passwords — so an owner had to know to type the path. A page reachable
    /// only by people who already know it is there is a page that gets found by whoever read the
    /// README, which is not the same set as whoever owns a machine.
    ///
    /// **It is a link and not a control**, which is what keeps
    /// `km-admin-pages`' *the singer's stays the singer's* intact: what a guest's tap reaches is a
    /// password prompt, and every route behind it demands the token whether it is linked from here
    /// or not. Being unreachable was never the protection; the password is.
    ///
    /// **Last on the tab, not first.** Setup's order is the machine, then this device's
    /// preferences, then the rarest thing — and for most people holding this phone the owner's page
    /// is the rarest thing on it. Leading with a door most viewers cannot open would be the most
    /// prominent possible placement for the one row least of them wants.
    pub owner_page: bool,
}

impl Capabilities {
    /// What `km-app` serves at `/`.
    pub fn online() -> Self {
        Self {
            favorites: false,
            initial_filter: false,
            connection: false,
            song_book: true,
            owner_page: true,
        }
    }

    /// What `km-remote` serves.
    pub fn offline() -> Self {
        Self {
            favorites: true,
            initial_filter: true,
            connection: true,
            // The offline remote's catalog is `km-remote-core`'s own mirror, with its own schema
            // and its own sort key, so serving a book from it would be a second adapter to keep in
            // step with `km_api::book`. And the workflow is not one anybody has: this app exists for
            // browsing with the machine switched off, which a printed book is the *alternative* to.
            song_book: false,
            // The offline app talks to whichever machine it found, and `/admin/` is a page on that
            // machine rather than one this process serves. A link would have to be built from the
            // address the connection currently holds -- and would then be a door onto a box that is
            // switched off half the time this app is open, which is the premise of the app. The
            // machine card already links out to the machine's own remote where reaching it matters.
            owner_page: false,
        }
    }
}

/// Everything the handlers share.
///
/// Cheap to clone — axum hands a copy to every handler — so the contents are behind `Arc`.
#[derive(Clone)]
pub struct Remote {
    /// The catalog.
    pub songs: Arc<dyn Songs>,
    /// Playback, which may be unreachable.
    pub machine: Arc<dyn Machine>,
    /// The collection, in the offline mode only.
    pub favorites: Option<Arc<dyn Favorites>>,
    /// Which machine this device talks to, in the offline mode only.
    ///
    /// `None` online, where the machine is this process and there is nothing to choose between. See
    /// [`Capabilities::connection`], which is the flag the templates read; this is the thing that
    /// answers once they do.
    pub connect: Option<Arc<dyn Connect>>,
    /// What this mode offers.
    pub capabilities: Capabilities,
    // **There is no guard here and no `/login` page.** Nothing the singer's remote serves is an
    // admin action, and a phone opens at the room level. Each write handler checks the phone's
    // level through `Machine::access` before it acts. See `handlers::permit`.
    /// What the browser tab shows, as PNG bytes served at `/static/icon.png`.
    ///
    /// Defaults to [`ICON_MACHINE_PNG`], so the mode that is *inside* the machine needs to say
    /// nothing; [`with_icon`](Self::with_icon) is how the offline remote asks for its own.
    pub icon_png: &'static [u8],
    /// The fan-out to open pages.
    pub hub: Hub,
}

impl Remote {
    /// Builds the shared state.
    pub fn new(
        songs: Arc<dyn Songs>,
        machine: Arc<dyn Machine>,
        favorites: Option<Arc<dyn Favorites>>,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            songs,
            machine,
            favorites,
            connect: None,
            capabilities,
            icon_png: ICON_MACHINE_PNG,
            hub: Hub::new(),
        }
    }

    /// Serves a different icon in the browser tab — [`ICON_REMOTE_PNG`], for a shell that is not the
    /// machine.
    ///
    /// A builder step rather than a fifth argument to [`new`](Self::new): the machine's own remote
    /// wants the default and should not
    /// have to name it. It is not folded into [`Capabilities`] because that is a `Copy` set of what
    /// this mode can *do*, and which picture a tab shows is not one of those.
    pub fn with_icon(mut self, icon_png: &'static [u8]) -> Self {
        self.icon_png = icon_png;
        self
    }

    /// Says which machine this device is talking to, and lets a page change it.
    ///
    /// A builder step for the same reason as the two above: the machine's own remote is not offered
    /// this and should not have to pass a `None` to say so. Installing it is what makes the Now
    /// tab's machine card appear; [`Capabilities::connection`] is what the templates ask.
    pub fn with_connect(mut self, connect: Arc<dyn Connect>) -> Self {
        self.connect = Some(connect);
        self
    }
}

/// What a `/static/` response tells the browser about keeping it.
///
/// A year, and `immutable` so a reload does not revalidate either. That is only safe because the
/// URLs carry [`ASSET_VERSION`]: the bytes at `/static/app.css?v=<stamp>` really are immutable, and
/// a rebuild that changes them changes the stamp with them.
///
/// **It is worth more here than the byte count suggests.** These four files were served with a
/// content type and nothing else, so a browser had no freshness information and re-fetched all of
/// them on every navigation — and the tabs are ordinary links, so every tab press is a navigation.
/// Five requests where one would do is four of a browser's six connections to this host, on a page
/// that is also holding one of them open for the event stream. See the head of `static/live.js`.
const STATIC_CACHE: &str = "public, max-age=31536000, immutable";

/// The stamp on every `/static/` URL, and the whole reason [`STATIC_CACHE`] can say a year.
///
/// A hash of the bytes actually compiled in, computed at build time — so it changes when and only
/// when one of the files changes, which is the property a hand-bumped constant does not have and
/// the crate version does not have either (these files change without a release, and a release
/// happens without them changing).
///
/// Both icons go into it although a given shell serves only one. Hashing what is *in the binary*
/// rather than what this instance chose keeps it a constant; the cost is that changing the machine's
/// mark re-stamps the remote's stylesheet too, which costs one download.
pub const ASSET_VERSION: &str = asset_version();

/// FNV-1a over the four embedded files, rendered as hex.
///
/// Written out by hand because it has to run in a `const`: no hasher in `std` is `const`, and the
/// alternative — a `OnceLock<String>` filled on first use — would make the stamp a runtime value
/// for no gain. Eight hex digits is ample for a cache key: it is not defending against anything,
/// it only has to differ when the bytes differ.
const fn asset_version() -> &'static str {
    const HASH: u64 = fnv1a(
        PICK_JS.as_bytes(),
        fnv1a(
            SCAN_JS.as_bytes(),
            fnv1a(
                JSQR_JS.as_bytes(),
                fnv1a(
                    ICON_REMOTE_PNG,
                    fnv1a(
                        ICON_MACHINE_PNG,
                        fnv1a(
                            LIVE_JS.as_bytes(),
                            fnv1a(APP_CSS.as_bytes(), fnv1a(HTMX_JS.as_bytes(), FNV_OFFSET)),
                        ),
                    ),
                ),
            ),
        ),
    );
    const DIGITS: [u8; 8] = hex8(HASH);
    // Safe by construction: `hex8` writes only ASCII hex digits.
    match std::str::from_utf8(&DIGITS) {
        Ok(text) => text,
        Err(_) => "00000000",
    }
}

/// FNV-1a's 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a over `bytes`, continuing from `hash` so several files fold into one value.
const fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// The low eight hex digits of `value`, as ASCII.
const fn hex8(value: u64) -> [u8; 8] {
    let digits = b"0123456789abcdef";
    let mut out = [b'0'; 8];
    let mut index = 0;
    while index < 8 {
        // Most significant of the eight first, so the text reads the way the number does.
        let nibble = ((value >> (4 * (7 - index))) & 0xf) as usize;
        out[index] = digits[nibble];
        index += 1;
    }
    out
}

/// Serves one embedded file with the content type a browser needs to honor it.
///
/// The type is not a formality: a stylesheet served as `text/plain` is ignored by every browser.
fn embedded(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, STATIC_CACHE),
        ],
        body,
    )
}

/// The same, for a file that is not text.
fn embedded_bytes(content_type: &'static str, body: &'static [u8]) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, STATIC_CACHE),
        ],
        body,
    )
}

/// Every route the remote answers.
///
/// All of them here rather than spread across modules, so the whole surface is one screenful — the
/// same reason `km-package-builder` keeps its router in one place and `km-api` keeps its `SURFACE`
/// table in one.
///
/// The paths are deliberately short and free of a prefix. In the online mode this is merged at the
/// root of the machine's own router, where `/api` is already taken and nothing else is; in the
/// offline app it is the whole server.
pub fn router(state: Remote) -> Router {
    Router::new()
        .route("/", get(handlers::browse))
        .route("/now", get(handlers::now))
        .route("/queue", get(handlers::queue_page))
        .route("/setup", get(handlers::setup))
        .route("/setup/packages", get(handlers::setup_packages))
        .route("/events", get(handlers::events))
        .route("/singer", post(handlers::set_singer))
        .route("/access", post(handlers::set_access))
        .route("/locale", post(handlers::set_locale))
        .route("/packages/hidden", post(handlers::set_hidden_packages))
        // Which machine this device talks to. Useful only in a build with a `connect`, and harmless
        // in one without — each answers with a toast saying so.
        .route("/machine/connect", post(handlers::connect_machine))
        .route("/machine/rescan", post(handlers::rescan_machine))
        .route("/machine/use", post(handlers::use_machine))
        .route("/machine/refresh", post(handlers::refresh_machine))
        .route("/song/{number}/{action}", post(handlers::song_action))
        .route("/queue/{entry}/{action}", post(handlers::queue_action))
        .route("/control/{action}", post(handlers::control))
        .route("/favorites/sheet/{number}", get(handlers::sheet))
        .route("/favorites/sheet/close", get(handlers::sheet_close))
        .route("/favorites/folders", post(handlers::create_folder))
        .route(
            "/favorites/folders/{id}/rename",
            post(handlers::rename_folder),
        )
        .route(
            "/favorites/folders/{id}/delete",
            post(handlers::delete_folder),
        )
        .route(
            "/favorites/folders/{id}/toggle/{number}",
            post(handlers::toggle_favorite),
        )
        .route(
            "/favorites/folders/{id}/remove/{number}",
            post(handlers::remove_favorite),
        )
        // Carrying a folder to another phone, and the whole collection to a file. Registered
        // unconditionally like `/machine/connect` above, and each guarded on `state.favorites` so
        // the online mode answers with `NO_FAVORITES` rather than the router having two shapes.
        .route("/favorites/share/{folder}", get(handlers::share_choose))
        .route("/favorites/share/{folder}/send", get(handlers::share_send))
        .route(
            "/favorites/share/{folder}/code.svg",
            get(handlers::share_image),
        )
        .route(
            "/favorites/share/{folder}/receive",
            get(handlers::share_receive).post(handlers::share_scanned),
        )
        .route(
            "/favorites/share/{folder}/merge",
            post(handlers::share_merge),
        )
        .route("/favorites/backup", get(handlers::backup_choose))
        .route("/favorites/backup.json", get(handlers::backup_export))
        // **The one route here that needs its own body limit**, and it goes on the method router
        // rather than the `Router`, or every route above it would silently get the same allowance.
        // `DefaultBodyLimit` is 2 MB for a `String` body and answers `413`, which htmx will not
        // swap — so without this a plausible file would leave the Restore button visually dead and
        // the reason nowhere on screen. The worded refusal is `handlers::MAX_DOCUMENT`, well inside
        // this; see its comment for why the two are not redundant.
        .route(
            "/favorites/backup/restore",
            get(handlers::backup_restore_page)
                .post(handlers::backup_restore)
                .layer(DefaultBodyLimit::max(handlers::RESTORE_LIMIT)),
        )
        .route(
            "/static/app.css",
            get(|| async { embedded("text/css; charset=utf-8", APP_CSS) }),
        )
        .route(
            "/static/live.js",
            get(|| async { embedded("text/javascript; charset=utf-8", LIVE_JS) }),
        )
        .route(
            "/static/htmx.min.js",
            get(|| async { embedded("text/javascript; charset=utf-8", HTMX_JS) }),
        )
        .route(
            "/static/htmx-LICENSE.txt",
            get(|| async { embedded("text/plain; charset=utf-8", HTMX_LICENSE) }),
        )
        .route(
            "/static/jsqr.js",
            get(|| async { embedded("text/javascript; charset=utf-8", JSQR_JS) }),
        )
        // Apache-2.0 requires the license travel with the code, which is why this is a route and
        // not merely a file in `static/`.
        .route(
            "/static/jsqr-LICENSE.txt",
            get(|| async { embedded("text/plain; charset=utf-8", JSQR_LICENSE) }),
        )
        .route(
            "/static/scan.js",
            get(|| async { embedded("text/javascript; charset=utf-8", SCAN_JS) }),
        )
        .route(
            "/static/pick.js",
            get(|| async { embedded("text/javascript; charset=utf-8", PICK_JS) }),
        )
        // The one static file that is not the same in both modes: which mark a tab shows is the
        // shell's answer, so it comes out of the state rather than out of a constant.
        .route(
            "/static/icon.png",
            get(
                |axum::extract::State(state): axum::extract::State<Remote>| async move {
                    embedded_bytes("image/png", state.icon_png)
                },
            ),
        )
        // **There is no authorization layer here.** A layer calling the machine's own check for the
        // route each page is about to exercise would have nothing to decide: none of these routes
        // can be admin, since they are all outside `/api/v1/admin/`. The machine refuses anything
        // it should; this page carries no permission of its own.
        .with_state(state)
}
