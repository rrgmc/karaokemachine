//! The owner's page, served by the machine at `/admin/`.
//!
//! The questions somebody who *owns* a karaoke machine actually has, one tab each: what the machine
//! is called and who may change it, which songs are on it, what is on the screen behind them, what
//! it sounds like, and what it found and could not use.
//!
//! # Why this exists beside two other pages
//!
//! `/` is the **singer's** remote and is deliberately only that — `Where the banks nobody is offered
//! live` records the cost of having made it so: *"choosing a bank is no longer possible from a phone
//! at all, which is a step back for an owner whose machine is under a television and whose only
//! other device is the one in their hand."* This is what makes that price affordable. The owner gets
//! a page of their own, so the singer's stays the singer's.
//!
//! `/dev/` is the **developer's** console and stays exactly as it is. `The dev remote stays` draws
//! that boundary and this must not blur it: `/dev/` is one hand-written file of vanilla JavaScript
//! that drives raw routes and is useful precisely because it is unpolished. This is an end-user
//! surface with an end-user's vocabulary — it says *pictures* rather than *wallpapers*, and it never
//! shows a route id.
//!
//! # One page set, and the hosts that serve it
//!
//! **This crate is the markup, the words and the rules about what a page shows. It knows nothing
//! about how a machine is reached**, which is [`machine`]'s traits — the machine implements them in
//! its own process ([`in_process`]), and `km-admin` implements them over HTTP against a machine on
//! the network.
//!
//! **Nothing in [`handlers`] or `views` names an `ApiState`**, which is the property that makes a
//! second host possible and the one to check before adding a handler: this crate does not depend on
//! a machine being in its own process, and the compiler holds that rather than a convention —
//! there is no field to reach through. Without it, `Two admin surfaces, one vocabulary` is a rule
//! holding a settings pane, an output picker and a factory-password banner in step across two
//! implementations by hand. See that decision in `docs/decisions/distribution.md`, and
//! `docs/architecture/admin.md` for how the two hosts are wired.
//!
//! # The one thing that is still not like `km-remote-pages`
//!
//! **The guard denies by default**, where that crate's lets an unlisted route through. There, an
//! unlisted route is a favorite or a static file and letting it past is right; here, an unlisted
//! route is a control-panel button somebody forgot to add to the table, and the failure has to be a
//! refusal rather than an open door. It also insures against the one thing nothing could settle by
//! reading: whether axum reports the full or the inner path from `MatchedPath` under `nest`. A wrong
//! guess fails **closed**.

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

/// What every log line in this crate is tagged with.
const LOG_TARGET: &str = env!("CARGO_CRATE_NAME");

pub mod guard;
mod handlers;
pub mod in_process;
/// What this crate needs from a machine, as traits a host implements.
pub mod machine;
pub mod views;
pub mod words;

/// The page's stylesheet, compiled in.
///
/// One file and no framework, exactly as the singer's remote and the project's own web page are.
/// Embedded rather than served from a folder beside the executable for the reason
/// `km-remote-pages` gives: a page that needs a second file is a page that half-works after a copy.
const APP_CSS: &str = include_str!("../static/admin.css");

/// A cache header for the embedded assets.
///
/// A year and `immutable`, which is safe only because every URL carries [`ASSET_VERSION`]: the
/// content is addressed by its own hash, so a stale copy cannot be served for a page that changed.
const STATIC_CACHE: &str = "public, max-age=31536000, immutable";

/// The machine's own mark: the microphone in the theme's amber.
///
/// Reaches into the generated `icon/` directory rather than keeping a copy under `static/`, exactly
/// as `km-remote-pages` and `km-package-builder` do — one place defines the icon, so a page cannot
/// end up showing last month's.
pub const ICON_MACHINE_PNG: &[u8] = include_bytes!("../../../../icon/icon-32.png");

/// KaraokeMachine Admin's mark: the same microphone in the theme's magenta.
///
/// **Both marks live here and the host picks one**, which is `km-remote-pages`' arrangement for the
/// same reason — and here the reason is written down twice over. `Two admin surfaces, one
/// vocabulary` asks the two to look like one product, and `km-admin`'s own server module gives the
/// counterweight: *"four programs in this product can be open at once and the favicon is what tells
/// two tabs apart."* One page set, two tabs, two marks.
pub const ICON_ADMIN_PNG: &[u8] = include_bytes!("../../../../icon/km-admin-32.png");

/// A stamp over the embedded assets, so a changed stylesheet cannot be served from a cache.
///
/// The same FNV-1a-in-a-`const fn` `km-remote-pages` uses, and it is worth copying rather than
/// sharing for one reason: what it hashes is *this crate's* assets, so a shared helper would need
/// the bytes passed in and the call site would be this line anyway.
pub const ASSET_VERSION: &str = asset_version();

/// FNV-1a over the one embedded file, rendered as hex.
///
/// Written out by hand for `km-remote-pages`' reason: it has to run in a `const`, no hasher in
/// `std` is `const`, and a `OnceLock<String>` filled on first use would make the stamp a runtime
/// value for no gain. Eight hex digits is ample — it is not defending against anything, it only has
/// to differ when the bytes differ.
const fn asset_version() -> &'static str {
    const HASH: u64 = fnv1a(APP_CSS.as_bytes(), FNV_OFFSET);
    const DIGITS: [u8; 8] = hex8(HASH);
    // Safe by construction: `hex8` writes only ASCII hex digits.
    match std::str::from_utf8(&DIGITS) {
        Ok(text) => text,
        Err(_) => "00000000",
    }
}

/// FNV-1a's 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a over `bytes`, continuing from `hash`.
const fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// The eight hex digits of `value`, as ASCII.
const fn hex8(value: u64) -> [u8; 8] {
    let digits = b"0123456789abcdef";
    let mut out = [b'0'; 8];
    let mut index = 0;
    while index < 8 {
        let nibble = ((value >> (4 * (7 - index))) & 0xf) as usize;
        out[index] = digits[nibble];
        index += 1;
    }
    out
}

/// What the pages are rendered against.
#[derive(Clone)]
pub struct Admin {
    /// Who is allowed to do what.
    ///
    /// Not `Option`, unlike `km-remote-pages`' — a page with no guard is an open control panel, and
    /// this crate refuses to be assembled that way rather than logging about it later.
    pub guard: Arc<dyn guard::Guard>,
    /// What the machine is, and the two names an owner gives it.
    pub machine: Arc<dyn machine::Machine>,
    /// The Debugging pane's switches, and demo mode.
    pub switches: Arc<dyn machine::Switches>,
    /// Where the sound comes out, and which bank supplies the instruments.
    pub sound: Arc<dyn machine::Sound>,
    /// Which packages are installed, and where their numbers sit.
    pub songs: Arc<dyn machine::Songs>,
    /// What is on the screen behind the words.
    pub pictures: Arc<dyn machine::Pictures>,
    /// Taking a file a browser sent.
    pub uploads: Arc<dyn machine::Uploads>,
    /// The packages the machine found and could not use, where a host can answer for them.
    ///
    /// **`Option`, and the only one of these that is.** The refused list identifies its rows by a
    /// path on the machine, which `PackageProblemDto` deliberately does not publish — *"the directory
    /// layout of the machine under the television is nobody's business but the owner's."* So a host
    /// reaching a machine over HTTP has no way to answer this, and `None` is that fact rather than an
    /// empty list, which would be a claim.
    ///
    /// The same shape `km-remote-pages` gives `Favorites`, for the same reason: a capability one mode
    /// has no implementation of at all.
    pub problems: Option<Arc<dyn machine::Problems>>,
    /// What this host's surface can do. See [`machine::Capabilities`].
    pub capabilities: machine::Capabilities,
    /// Which program is drawing this page, where that is not the machine itself.
    ///
    /// # Why the shared page has to be able to say
    ///
    /// **`What the tool calls itself` requires it.** That decision says `km-admin`'s *pages* say
    /// *KaraokeMachine Admin* while its `--help` and its banner say `km-admin` — two audiences, two spellings —
    /// and a shared layout that only ever showed the machine's name would have quietly dropped the
    /// first. It was caught by `the_page_renders`, which asserted the product's name was on the page
    /// and stopped being true the moment this crate drew it.
    ///
    /// `None` on the machine, whose pages *are* the machine's: a label there would be the box
    /// introducing itself to somebody already looking at its own address.
    pub program: Option<&'static str>,
    /// The mark in a browser tab, compiled in.
    ///
    /// # A field rather than a route on the machine
    ///
    /// The layout asked for `/icon.png`, which is **the machine's** route and exists in no other
    /// host. `km-remote-pages` had the same problem and answers it the same way: the marks live in
    /// the crate and the host picks one.
    ///
    /// **It has to differ per host**, which `km-admin`'s own server module already argued before
    /// there was a seam to state it on: *"four programs in this product can be open at once and the
    /// favicon is what tells two tabs apart."* A tool wearing the machine's amber would be the one
    /// tab you cannot find.
    pub icon_png: &'static [u8],
    /// The language every page is drawn in, or `None` to ask the reader's browser.
    ///
    /// # Why a host may pin it
    ///
    /// **`None` is right for the machine** and is what `words::locale` was written for: the cookie
    /// the singer's remote writes, then `Accept-Language`, so a viewer who chose Portuguese at `/`
    /// does not meet English at `/admin/`.
    ///
    /// **`Some` is for a host whose own pages have no catalog**, because half a program in one
    /// language is worse than none of it: a host drawing untranslated markup of its own asks for the
    /// shared pages in that same language rather than letting a page read as half broken.
    ///
    /// No host needs it — `km-admin` keeps a catalog beside its own templates — and it stays for the
    /// next one, at the cost of an `Option` and one `unwrap_or_else`.
    ///
    /// Deliberately **not** a [`machine::Capabilities`] field: what language a page is in is not
    /// something a surface can or cannot do.
    pub fixed_locale: Option<km_locale::Locale>,
    /// Scripts this host's own pages load, in order. Empty on the machine.
    ///
    /// # Why the scriptless page has a field for scripts
    ///
    /// **Because a host's own pages are not the machine's pages, and one of them needs htmx.**
    /// `/admin/ is the owner's page` argues at length that these pages carry no script — *"it works
    /// with scripting off, on a phone browser nobody chose"* — and that is unchanged and enforced:
    /// the machine passes nothing here, and `no_page_the_machine_serves_carries_a_script` renders
    /// every route it has and asserts so.
    ///
    /// `km-admin` is a different program. Its picture searching and its bank fetching run jobs that
    /// take minutes, and `_job.html` replaces itself once a second to report one — the single thing
    /// on those pages a `<form>` cannot do. It declares htmx and its own `ui.js`, which exists
    /// because htmx will not swap a non-2xx response and so a failure with no handling looks like a
    /// control that does nothing.
    ///
    /// **These reach only [`views::Shell`]**, the page a host renders its own body into. So the tabs
    /// this crate draws are script-free whichever host is serving them, rather than only on the one
    /// that declares nothing — which is a stronger property than the decision asks for and free to
    /// keep.
    ///
    /// # The regression this replaces
    ///
    /// `km-admin` had its own `layout.html` and it was the only thing linking htmx, `ui.js` and that
    /// program's stylesheet. Deleting it — the point of the second-host change — took all three
    /// `<link>` and `<script>` tags with it, and nothing noticed: every test drives a router and
    /// asserts on markup, and not one of them asked whether a page loads what it needs. The
    /// searching still ran and still finished; its progress bar simply never moved again.
    pub scripts: &'static [&'static str],
}

impl Admin {
    /// The page, against one host that answers for every tab.
    ///
    /// **One object behind all of them, fanned out here rather than at the call site.** The seam is
    /// split by *tab* so the strip and the traits can be read against each other — not because a
    /// host might answer them from different places, and a host that could is not a thing this
    /// product has. Passing the same `Arc` five times was the alternative, and it read as though the
    /// five might differ.
    pub fn over<H>(
        capabilities: machine::Capabilities,
        icon_png: &'static [u8],
        guard: Arc<dyn guard::Guard>,
        host: Arc<H>,
    ) -> Self
    where
        H: machine::Machine
            + machine::Switches
            + machine::Sound
            + machine::Songs
            + machine::Pictures
            + machine::Uploads,
    {
        Self {
            guard,
            machine: host.clone(),
            switches: host.clone(),
            sound: host.clone(),
            songs: host.clone(),
            pictures: host.clone(),
            uploads: host,
            // Not part of the bound: a host that can answer for refused packages says so with
            // `with_problems`, and one that cannot is the ordinary case rather than an incomplete
            // one. `km-remote-pages`' `with_connect` is the same builder step for the same reason.
            problems: None,
            capabilities,
            icon_png,
            // The reader's browser decides, which is right for the machine. A host with an
            // untranslated half of its own says otherwise with `with_fixed_locale`.
            program: None,
            fixed_locale: None,
            // None, which is the machine's answer and the decision's: these pages work with
            // scripting switched off. A host with a job to report says otherwise with
            // `with_scripts`.
            scripts: &[],
        }
    }

    /// ...and every page says which program is drawing it.
    ///
    /// For a host that is not the machine. See [`Self::program`].
    #[must_use]
    pub fn drawn_by(mut self, program: &'static str) -> Self {
        self.program = Some(program);
        self
    }

    /// ...and every page is drawn in one language, whatever the reader's browser asks for.
    ///
    /// For a host whose own pages have no catalog. See [`Self::fixed_locale`], including why it stays
    /// with nothing calling it.
    #[must_use]
    pub fn with_fixed_locale(mut self, locale: km_locale::Locale) -> Self {
        self.fixed_locale = Some(locale);
        self
    }

    /// ...and this host's own pages load these scripts, in this order.
    ///
    /// For a host whose pages do something a form cannot. See [`Self::scripts`], which is also
    /// where the reason the machine calls this and never will is written down.
    #[must_use]
    pub fn with_scripts(mut self, scripts: &'static [&'static str]) -> Self {
        self.scripts = scripts;
        self
    }

    /// A page of this host's own, inside the shared chrome.
    ///
    /// # The seam a host with its own pages needs
    ///
    /// `km-admin` has two pages this crate does not and should not have: its front door, and the
    /// picture and bank *searching* that
    /// `A fourth program, rather than a fourth tab on the owner's page` says must never run on the
    /// machine. Both need the shared strip, heading and factory-password banner — and askama cannot
    /// `{% extends %}` across a crate, so without this a host would keep a second copy of exactly
    /// the markup this seam exists to delete.
    ///
    /// `tab` marks the entry in the strip, so a host's page looks like the page you are on.
    ///
    /// **`content` is markup, and the contract is in [`views::Shell::content`]:** render a template
    /// and pass its output, never build the string. A request's value formatted into it would be an
    /// injection nothing here could catch.
    /// `notice` is what a redirect asked this load to say, and it is drawn where every other
    /// banner on this surface is drawn. **The banner is chrome**, so a host hands one over rather
    /// than drawing a second copy inside its own body: a refusal that redirects to a host's page
    /// has nowhere else to put its sentence, and two banners in two places is the drift this seam
    /// exists to delete.
    pub async fn shell(
        &self,
        tab: views::Tab,
        locale: km_locale::Locale,
        notice: Option<views::Notice>,
        content: String,
    ) -> axum::response::Response {
        let chrome = handlers::chrome(self, tab, locale).await;
        views::page(
            &views::Shell {
                chrome,
                notice,
                content,
            },
            locale,
        )
    }

    /// What language to draw a page in, given what this request said.
    ///
    /// **One place asks, so a handler cannot forget the host may have pinned it.** There are
    /// thirty-odd page handlers and each needs a locale; a host with an untranslated half of its own
    /// would otherwise be relying on every one of them to check a field.
    #[must_use]
    pub fn locale(&self, headers: &axum::http::HeaderMap) -> km_locale::Locale {
        self.fixed_locale.unwrap_or_else(|| words::locale(headers))
    }

    /// ...and this host can also list the packages the machine refused.
    ///
    /// A builder step rather than a seventh argument, because it is the one capability a host may
    /// genuinely lack. See [`Self::problems`].
    #[must_use]
    pub fn with_problems(mut self, problems: Arc<dyn machine::Problems>) -> Self {
        self.problems = Some(problems);
        self
    }

    /// A host's front door: its own body, in this crate's `<head>` and stylesheet, and no chrome.
    ///
    /// # Why a second wrapper rather than a `Tab` of its own
    ///
    /// **The page this draws is the page somebody is on before there is a machine**, so every part
    /// of the chrome is either blank or wrong there. The heading is the *machine's* name, the banner
    /// warns about the *machine's* factory password, the badge counts the *machine's* problems, and
    /// the strip's entries all lead to tabs that can do nothing yet. A strip drawn over that is a
    /// row of controls that answer *which machine?* with five ways to look at one.
    ///
    /// **What it buys is measured rather than tidy: this page makes no trait call at all.**
    /// [`handlers::chrome`] `try_join!`s the machine's name, its factory-password flag and its
    /// problem count, and on a host over HTTP each is a request that a machine which is not
    /// answering charges the full ask timeout for. The front door is the page somebody reaches
    /// *because* the machine is off, so paying three timeouts to draw a heading naming nothing is
    /// the exact wrong bargain. [`views::Chrome::door`] fills in the four fields the `<head>` needs
    /// and asks nobody anything.
    ///
    /// **`content` carries [`views::Shell::content`]'s contract unchanged**: render a template and
    /// pass its output, never build the string.
    #[must_use]
    pub fn door(
        &self,
        locale: km_locale::Locale,
        notice: Option<views::Notice>,
        content: String,
    ) -> axum::response::Response {
        views::page(
            &views::Shell {
                chrome: views::Chrome::door(self.capabilities, self.program, locale, self.scripts),
                notice,
                content,
            },
            locale,
        )
    }
}

/// The owner's page, to be nested under `/admin`.
///
/// **Every route here demands the admin password except the [`guard::is_open`] names**, and the
/// middleware refuses the rest — so a route added here is admin by default rather than public by
/// accident, which is the way round that fails safely.
/// `tests/pages.rs` drives the nested router and asserts exactly that.
///
/// # Each upload route needs its own body limit, and the default is not a limit anybody chose
///
/// axum's `DefaultBodyLimit` is **2 MB** unless a route says otherwise, and every one of these three
/// forms carries a file far larger than that: a package is tens of megabytes once it holds a video
/// or an MP3+G pair, the smallest SoundFont this project offers is 32 MB, and a photograph is
/// routinely over 2 MB. Without these layers the forms are not slow or awkward — they are
/// **unusable for their real payloads**, and this page is the one surface a television with no
/// keyboard has for adding any of them.
///
/// The limits are `km_api::handlers`' own constants rather than numbers repeated here, because these
/// routes exist to be the browser's way to the same operations the API exposes: `/songs/upload`
/// against `POST /admin/packages/upload`, and so on. Two limits for one operation is the drift
/// naming the constants prevents.
///
/// **It failed in the least helpful way available**, which is why this is written down. An 85 MB
/// package uploaded at 2,162,688 bytes and came back as *"the upload stopped early: Error parsing
/// `multipart/form-data` request"* — a parser complaint, naming neither a size nor a limit, for a
/// file that was perfectly well formed. A small package installed from the same form seconds later.
/// Nothing in the code said 2 MB, because nothing in the code said anything.
///
/// **The message half of that has since been fixed and the limits are still what matter.**
/// `km_api::handlers::multipart_failure` reads axum's `status()` and `body_text()` rather than its
/// `Display`, so a limit trip now answers **413** with *"the limit is 64 MB"* instead of the parser
/// complaint. That makes a missing layer here survivable rather than baffling — it would refuse at
/// 2 MB and say so — but it does not make it correct, and the file would still be one nobody could
/// send.
pub fn router(state: Admin) -> Router {
    let router = Router::new()
        // `/admin/` lands on the first tab in the bar, which is *This machine*.
        .route("/", get(handlers::machine_page))
        .route("/songs", get(handlers::songs_page))
        .route(
            "/songs/upload",
            post(handlers::upload_package)
                .layer(DefaultBodyLimit::max(km_api::handlers::MAX_PACKAGE_BYTES)),
        )
        // The question and the answer at one address: `GET` asks, `POST` does. Same spelling as
        // `/login` below, and it means the confirmation adds no new URL vocabulary — nothing that
        // links to a remove has to learn a second path.
        .route(
            "/songs/{id}/remove",
            get(handlers::confirm_remove_package).post(handlers::remove_package),
        )
        .route("/songs/{id}/bank", post(handlers::set_bank))
        .route("/pictures", get(handlers::pictures_page))
        .route(
            "/pictures/upload",
            post(handlers::upload_wallpaper)
                .layer(DefaultBodyLimit::max(km_api::handlers::MAX_WALLPAPER_BYTES)),
        )
        .route("/pictures/next", post(handlers::next_picture))
        // `{id}` cannot collide with the `next` above: a picture id carries its file extension, so
        // `next.jpg` is `next-jpg`. See the same pair in `km_api::routes`, where the trap is
        // spelled out.
        .route(
            "/pictures/{id}/remove",
            get(handlers::confirm_remove_picture).post(handlers::remove_picture),
        )
        .route("/sound", get(handlers::sound_page))
        .route(
            "/sound/upload",
            post(handlers::upload_soundfont)
                .layer(DefaultBodyLimit::max(km_api::handlers::MAX_SOUNDFONT_BYTES)),
        )
        // **Both take a body field rather than a path segment**, because the Problems tab offers each
        // of these as a `<select>` and this page has no script to build a URL with. `use` and
        // `output` are static segments sitting where `{id}` used to, which is safe: axum prefers a
        // static segment, and there is no `{id}` left under `/sound/` but the two below.
        .route("/sound/use", post(handlers::use_bank))
        .route("/sound/output", post(handlers::use_output))
        // A static segment beside the two above, for the same reason: the control is a slider and
        // this page has no script to build a URL with. The confirmation a large rise raises posts
        // back here too, with the value in the query.
        .route("/sound/level", post(handlers::use_level))
        .route(
            "/sound/{id}/remove",
            get(handlers::confirm_remove_bank).post(handlers::remove_bank),
        )
        .route("/problems", get(handlers::problems_page))
        // The same `GET` asks, `POST` does pair as `/songs/{id}/remove`, and for the same reason:
        // this deletes a file, and this page has no script to raise a `confirm()` with.
        .route(
            "/problems/{id}/delete",
            get(handlers::confirm_delete_problem).post(handlers::delete_problem),
        )
        .route("/machine", get(handlers::machine_page))
        .route("/machine/name", post(handlers::set_name))
        .route("/machine/locale", post(handlers::set_machine_locale))
        .route("/machine/demo", post(handlers::set_demo))
        .route("/machine/demo-delay", post(handlers::set_demo_delay))
        .route("/machine/password", post(handlers::set_password))
        .route("/machine/sessions", post(handlers::reset_sessions))
        // Beside the password pane's own route and not under it: `POST /machine/password` sets one on
        // the machine, and this forgets one on this computer. Two different acts on two different
        // disks, which is why the path says `forget` rather than the verb carrying the difference.
        .route("/machine/password/forget", post(handlers::forget_password))
        .route("/machine/debug", post(handlers::set_debug))
        // The other two switches on the same pane. The console needs the one above as well; the
        // panel needs neither and takes effect on the next frame.
        .route("/machine/dev-remote", post(handlers::set_dev_remote))
        .route("/machine/performance", post(handlers::set_performance))
        // Mounted unconditionally, unlike the API's twins, and the asymmetry is deliberate. There
        // the URL is a client's contract and an absent capability should read as 404; here both
        // paths are only ever reached from a card this page does not draw, so what a typed URL
        // deserves on a machine with no power control is the page saying so rather than the bare
        // 404 a browser renders as "this address is wrong".
        //
        // The same `GET` asks, `POST` does pair as every removal above.
        .route(
            "/machine/power/off",
            get(handlers::confirm_shut_down).post(handlers::shut_down),
        )
        // No confirmation, and that is the argued half: it interrupts the evening for ten seconds
        // and mends itself, which puts it beside "show the next picture" rather than beside a
        // delete. See `confirm_shut_down`.
        .route(
            "/machine/power/restart",
            post(handlers::restart_application),
        )
        .route("/login", get(handlers::login_page).post(handlers::login))
        .route("/static/admin.css", get(handlers::stylesheet))
        // The mark in the browser tab. A route rather than the machine's own `/icon.png`, which is
        // the only host that has one — see `Admin::icon_png`.
        .route("/static/icon.png", get(handlers::favicon));

    // **The guard layer is the machine's, and a loopback tool serves the same router without it.**
    // Not because that surface is less careful but because the question is different: this middleware
    // asks *may this browser act as the owner*, which is worth asking when every phone on the LAN can
    // reach the page and is not the question a program on 127.0.0.1 with no password of its own has.
    // There, nothing in this crate authorizes a write: the machine is the authority on whether its
    // own write may happen, and refuses an untokened one with a 401 that arrives as
    // `AdminError::Unauthorized`. What `guard.allows` answers on that surface is whether the
    // *program* holds a token, which is a read the Machine tab makes once to decide whether to draw
    // its login pane — see `Capabilities::gate_every_route` and `Capabilities::log_in`.
    let router = if state.capabilities.gate_every_route {
        router.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            handlers::authorize,
        ))
    } else {
        router
    };

    router.with_state(state)
}
