//! Talking to a karaoke machine.
//!
//! **Server-to-server, and that is what keeps CORS out of this program entirely.** The browser talks
//! only to `km-admin` on loopback; `km-admin` talks to the machine from here. The machine's
//! `api.cors_origins` ships empty and would refuse a cross-origin browser request, which is correct
//! and never comes up.
//!
//! ## Ask for a password where one is about to be needed, and not before
//!
//! **Every machine has a password**, generated as a PIN at first start, so every `/admin/` route
//! wants a token. There is no machine whose privileged routes are public.
//!
//! Reading is free, though: `GET /discover`, `/wallpapers`, `/audio/soundfonts`, `/packages` and
//! `/demo` all ship public, and this program draws a whole machine panel from them holding nothing.
//! So a page that opened with a login form would be demanding a credential before there was anything
//! to spend it on.
//!
//! The order is therefore: show what can be read, and offer the password box where a control that
//! needs one is drawn. [`Client::upload`] returns [`Refused::Unauthorized`] on a 401 or 403 and
//! [`Client::log_in`] exchanges a password for a bearer token.
//!
//! **`/discover` answering is not evidence that nothing needs a password**, and reading it that way
//! was a real bug: the machine panel decided a machine wanted no credential because `/discover` — a
//! public route — had answered, so it never drew the login form at all, and Rename, the demo switch
//! and the debug switch each failed with a 401 and nowhere to type anything.
//!
//! ## The token
//!
//! In memory, for the life of this process. The machine forgets its own tokens on restart and gives
//! them a twelve-hour life, so there is nothing to be gained by writing one down and something to
//! lose: a bearer token in a file is a credential this program was not asked to keep.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use km_api::discover::Discovery;
use km_api::dto::{
    AdminPasswordDto, AdminPasswordRequest, AudioLevelRequest, AudioOutputRequest, AudioOutputsDto,
    BankDto, BankRequest, DebugDto, DemoDto, DevRemoteDto, DevRemoteRequest, LoginRequest,
    LoginResponse, MachineLocaleRequest, MachineNameRequest, PackagesDto, PerformanceDto,
    PerformanceRequest, SetDemoDelayRequest, SetDemoRequest, SoundFontDto, SoundFontRequest,
    SoundFontsDto, UninstallDto, UploadReportDto, WallpapersDto,
};

/// The machine's own default port, so "a bare host means 8177" is not written down twice.
pub use km_api::connect::DEFAULT_PORT as MACHINE_DEFAULT_PORT;

/// Every request this program makes of a machine, as the verb and the path together.
///
/// **A type rather than a `&str`, because a `&str` is how this program spent its whole life sending
/// files nowhere.** [`Client::url`] prepends `/api/v1` and each caller supplied the rest, so the
/// three uploads went to `/api/v1/packages/upload`, `/api/v1/wallpapers` and
/// `/api/v1/audio/soundfonts` — none of which the machine mounts, because every route that writes
/// moved under `/api/v1/admin/` (`The URL prefix is the permission`, in
/// `docs/decisions/api-and-network.md`). Nothing this program ever sent arrived.
///
/// **A list beside the literals would not have caught it**, which is why this is a type. The same
/// bug reached `/api/v1/machine/name` once before, and the fix then was to correct one string; a
/// second string written from the same belief was all it took to have it back. With no `&str` door
/// left, a new call site cannot invent a path — it adds a variant, and
/// [`tests::every_call_this_program_makes_is_a_route_the_machine_mounts`] sweeps the lot against
/// [`km_api::routes::SURFACE`].
///
/// **The verb travels with the path deliberately.** `/wallpapers` and `/audio/soundfonts` are real
/// routes — as `GET`. Two of the three uploads were therefore not a missing path but a wrong verb
/// on a right one, which a path-only check calls correct and a machine answers 405 to.
///
/// The three sends delegate to [`km_api::uploads::path_for`] rather than spelling anything, so this
/// program and the machine read one string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call<'a> {
    /// `GET` — what the machine says it is. The one call that works before anything is known.
    Discover,
    /// `POST` — a password for a bearer token.
    Login,
    /// `GET` — the pictures in the rotation.
    Wallpapers,
    /// `GET` — the banks installed.
    SoundFonts,
    /// `GET` — the packages installed.
    Packages,
    /// `GET` — whether the demo is on.
    Demo,
    /// `PUT` — turn the demo on or off.
    SetDemo,
    /// `PUT` — how long the quiet has to last before the demo starts.
    SetDemoDelay,
    /// `GET` — whether debugging mode is on, and what settings say it will be.
    Debug,
    /// `PUT` — turn debugging mode on or off.
    SetDebugging,
    /// `GET` — whether the development console is asked for, and whether it is up.
    DevRemote,
    /// `PUT` — turn the development console on or off.
    SetDevRemote,
    /// `GET` — whether the frame-statistics panel is on the machine's screen.
    Performance,
    /// `PUT` — put the frame-statistics panel on the machine's screen, or take it off.
    SetPerformance,
    /// `GET` — the output devices the machine has, and which one it is using.
    AudioOutputs,
    /// `PUT` — choose the output device.
    SetAudioOutput,
    /// `PUT` — move the level the chosen output runs at.
    SetAudioLevel,
    /// `PUT` — rename the machine.
    Rename,
    /// `GET` — what language the machine's television draws in.
    Locale,
    /// `PUT` — set what language the machine's television draws in.
    ///
    /// **Not this program's own language**, which is a cookie on this program's own origin and
    /// never leaves the browser. The two are named apart everywhere they meet: a *screen* language
    /// belongs to the room the television is in.
    SetLocale,
    /// `POST` — change the machine's password.
    SetPassword,
    /// `POST` — end every outstanding session on the machine, this program's included.
    ///
    /// **Arrived with the shared page rather than with a feature.** *Sign out everywhere* shares the
    /// password pane on the machine's own surface — two different acts, one errand — so serving that
    /// markup means being able to answer both. An owner whose phone went missing should not have to
    /// pick a new password and then tell the house what it is.
    ResetSessions,
    /// `POST` — send a file of one of the three kinds.
    Send(km_api::machine::Upload),
    /// `DELETE` — uninstall a package, by id.
    RemovePackage(&'a str),
    /// `PUT` — move a package's songs to another bank, by id.
    SetPackageBank(&'a str),
    /// `POST` — show the next picture in the rotation.
    ///
    /// **A public route, unlike its neighbours**, and the machine's own reason is that it changes
    /// nothing anybody would mind: the rotation moves on by itself anyway. It is here rather than
    /// under `/admin` because that is where the machine mounts it.
    NextWallpaper,
    /// `DELETE` — take a picture out of the rotation, by id.
    RemoveWallpaper(&'a str),
    /// `PUT` — play through a bank the machine already has, by id in the body.
    ///
    /// Singular, which is the machine's spelling: `/audio/soundfonts` is the list and
    /// `/audio/soundfont` is the one in force — the same split [`Self::SetAudioOutput`] has.
    UseBank,
    /// `DELETE` — delete a bank off the machine, by id.
    RemoveBank(&'a str),
    /// `GET` — which bank is loaded, and whether all of it loaded.
    LoadedBank,
}

/// One path segment, with everything that is not a plain character escaped.
///
/// # Why this is here and not `Url::path_segments_mut`
///
/// **Because an id has to be escaped while it is still a separate value.** Build the whole path with
/// `format!` and then push its `/`-separated pieces through `Url`, and an id containing slashes has
/// already become several segments by the time anything looks at it:
/// `RemoveBank("../../admin/password")` addresses `/api/v1/admin/audio/soundfonts/admin/password`.
/// `an_id_with_something_awkward_in_it_cannot_reshape_a_url` is that bug, kept.
///
/// **These ids are not this program's to trust.** A bank's is a file name, a picture's is a slug of
/// one and a package's comes out of a manifest — so a space, a `?`, a `#` or a `..` is a thing that
/// can genuinely arrive, and each of them means something to a URL.
///
/// Unreserved characters per RFC 3986 pass through, everything else becomes `%XX`. That keeps
/// [`km_api::routes::SURFACE`]'s own sample ids identical to themselves, which is what lets the
/// sweep compare a built path against a declared one.
fn one_segment(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The ids [`Call::ALL`] uses, which are [`km_api::routes::SURFACE`]'s own samples.
///
/// **They have to be the same strings**, because that table is concrete samples rather than axum
/// patterns — its own doc says `{id}` never appears in it — so the sweep compares a built path
/// against a built path. Naming them here rather than inline says out loud that they are not
/// arbitrary, and a change to the machine's table is a compile-time-visible change here.
const SAMPLE_PACKAGE: &str = "vol1";
const SAMPLE_WALLPAPER: &str = "sunset-jpg";
const SAMPLE_BANK: &str = "generaluser";

impl<'a> Call<'a> {
    /// Every variant, for the test that checks them against the machine's own table.
    ///
    /// Written out rather than derived, which is the one place a new variant has to be remembered —
    /// and forgetting it weakens the sweep rather than breaking the program, so the cost is the
    /// right way round.
    pub const ALL: &'static [Call<'static>] = &[
        Call::Discover,
        Call::Login,
        Call::Wallpapers,
        Call::SoundFonts,
        Call::Packages,
        Call::Demo,
        Call::SetDemo,
        Call::SetDemoDelay,
        Call::Debug,
        Call::SetDebugging,
        Call::DevRemote,
        Call::SetDevRemote,
        Call::Performance,
        Call::SetPerformance,
        Call::AudioOutputs,
        Call::SetAudioOutput,
        Call::SetAudioLevel,
        Call::Rename,
        Call::Locale,
        Call::SetLocale,
        Call::SetPassword,
        Call::ResetSessions,
        Call::Send(km_api::machine::Upload::Package),
        Call::Send(km_api::machine::Upload::Wallpaper),
        Call::Send(km_api::machine::Upload::SoundFont),
        Call::RemovePackage(SAMPLE_PACKAGE),
        Call::SetPackageBank(SAMPLE_PACKAGE),
        Call::NextWallpaper,
        Call::RemoveWallpaper(SAMPLE_WALLPAPER),
        Call::UseBank,
        Call::RemoveBank(SAMPLE_BANK),
        Call::LoadedBank,
    ];

    /// The verb the machine mounts this path under.
    #[must_use]
    pub fn method(self) -> reqwest::Method {
        match self {
            Self::Discover
            | Self::Wallpapers
            | Self::SoundFonts
            | Self::Packages
            | Self::Demo
            | Self::Debug
            | Self::DevRemote
            | Self::Performance
            | Self::AudioOutputs
            | Self::Locale
            | Self::LoadedBank => reqwest::Method::GET,
            Self::Login
            | Self::SetPassword
            | Self::ResetSessions
            | Self::Send(_)
            | Self::NextWallpaper => reqwest::Method::POST,
            Self::SetDemo
            | Self::SetDemoDelay
            | Self::SetDebugging
            | Self::SetDevRemote
            | Self::SetPerformance
            | Self::SetAudioOutput
            | Self::SetAudioLevel
            | Self::Rename
            | Self::SetLocale
            | Self::SetPackageBank(_)
            | Self::UseBank => reqwest::Method::PUT,
            Self::RemovePackage(_) | Self::RemoveWallpaper(_) | Self::RemoveBank(_) => {
                reqwest::Method::DELETE
            }
        }
    }

    /// The path, relative to `/api/v1` — the same convention [`km_api::routes::SURFACE`] uses.
    ///
    /// # Why this is a `Cow` and not a `&'static str`
    ///
    /// **Six routes name a thing**, and the id belongs *in* the call rather than beside it. The
    /// alternative was a `url_with(call, id)` taking the id separately, which would have put a
    /// `&str` back on the door this type exists to close: two arguments can be paired wrongly, and
    /// nothing would have caught a removal sent to a wallpaper's path.
    ///
    /// So the id travels as part of the variant, this builds the path, and
    /// `every_call_this_program_makes_is_a_route_the_machine_mounts` still compares a whole path
    /// against the machine's own table — with the *samples that table uses*, which is what
    /// [`SAMPLE_PACKAGE`] and its two neighbours are for.
    ///
    /// **The id is percent-encoded**, in [`Client::url`] rather than here, because what this returns
    /// is compared against a route table and an escaped sample would never match one.
    #[must_use]
    pub fn path(self) -> std::borrow::Cow<'a, str> {
        use std::borrow::Cow;
        match self {
            Self::RemovePackage(id) => Cow::Owned(format!("/admin/packages/{}", one_segment(id))),
            Self::SetPackageBank(id) => {
                Cow::Owned(format!("/admin/packages/{}/bank", one_segment(id)))
            }
            Self::NextWallpaper => Cow::Borrowed("/wallpapers/next"),
            Self::RemoveWallpaper(id) => {
                Cow::Owned(format!("/admin/wallpapers/{}", one_segment(id)))
            }
            Self::UseBank => Cow::Borrowed("/admin/audio/soundfont"),
            Self::RemoveBank(id) => {
                Cow::Owned(format!("/admin/audio/soundfonts/{}", one_segment(id)))
            }
            Self::LoadedBank => Cow::Borrowed("/audio/soundfont"),
            other => Cow::Borrowed(other.fixed_path()),
        }
    }

    /// The path of a call that names nothing, which is most of them.
    fn fixed_path(self) -> &'static str {
        match self {
            Self::Discover => "/discover",
            Self::Login => "/admin/login",
            Self::Wallpapers => "/wallpapers",
            Self::SoundFonts => "/audio/soundfonts",
            Self::Packages => "/packages",
            Self::Demo => "/demo",
            Self::SetDemo => "/admin/demo",
            Self::SetDemoDelay => "/admin/demo/delay",
            Self::Debug => "/debug",
            Self::SetDebugging => "/admin/debug",
            Self::DevRemote => "/dev-remote",
            Self::SetDevRemote => "/admin/dev-remote",
            Self::Performance => "/performance",
            Self::SetPerformance => "/admin/performance",
            // The read is public and the write is not, which is the split every pair here has. The
            // singular on the write is the machine's own spelling: `/audio/outputs` is the list and
            // `/audio/output` is the one in force.
            Self::AudioOutputs => "/audio/outputs",
            Self::SetAudioOutput => "/admin/audio/output",
            Self::SetAudioLevel => "/admin/audio/level",
            Self::Rename => "/admin/machine/name",
            // The read is public and the write is not, as everywhere else here. The write sits
            // under `/machine/` beside the name because both are facts an owner wrote down about
            // this machine; the read does not, `/discover` carrying no locale to hang it off.
            Self::Locale => "/locale",
            Self::SetLocale => "/admin/machine/locale",
            Self::SetPassword => "/admin/password",
            Self::ResetSessions => "/admin/sessions/reset",
            Self::Send(kind) => km_api::uploads::path_for(kind),
            // Unreachable: `path` answers all seven before delegating here. A `match` rather than
            // a catch-all so that adding a variant and forgetting both arms is a compile error.
            Self::RemovePackage(_)
            | Self::SetPackageBank(_)
            | Self::NextWallpaper
            | Self::RemoveWallpaper(_)
            | Self::UseBank
            | Self::RemoveBank(_)
            | Self::LoadedBank => unreachable!("`path` builds these itself"),
        }
    }
}

/// How long to wait on a request that is only asking a question.
const ASK_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait on an upload.
///
/// **Generous rather than tight.** A SoundFont bank runs to a gigabyte, and the machine writes it to
/// disk as it arrives — on a television box that is slow storage over a slow network, and a timeout
/// that fired at five minutes would abandon a transfer that was going perfectly well.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// What went wrong, in the words the page will use.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
    /// No machine has been named yet.
    #[error("no machine selected; set an address first")]
    NoMachine,
    /// The machine did not answer at all.
    #[error(
        "{0} is not answering; check that the machine is switched on and the address is correct"
    )]
    Unreachable(String),
    /// A password is needed, or the one given has expired.
    #[error("this machine requires a password and none has been given")]
    Unauthorized,
    /// The password was wrong, or too many have been tried.
    #[error("{0}")]
    Rejected(String),
    /// The machine answered, and said no.
    #[error("the machine refused it: {0}")]
    Said(String),
    /// Something local went wrong — a file that could not be read.
    #[error("{0}")]
    Local(String),
}

/// Turns a typed address into a URL.
///
/// A bare host or IP gets the machine's own default port, which is what somebody who has changed
/// nothing wants. **It is `km-remote`'s function and not a copy of it**, so the two tools cannot
/// disagree about where a port goes.
pub use km_api::discover::known::normalize;

/// A machine, and whatever token this program holds for it.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: Arc<Mutex<Option<String>>>,
    recent: Arc<Mutex<Recent>>,
}

/// What the last few moments of talking to this machine came to.
///
/// # Why a page needs this at all
///
/// **One page draws itself out of eleven requests**, and against a machine that is not there each
/// one waits [`ASK_TIMEOUT`] before saying so. Three of the eleven are the same `/discover`, asked
/// by the heading, the information panel and the factory-password banner, none of which knows the
/// others exist — and every page after the first asks the whole set again.
///
/// So two things are remembered, each for a different half of the problem: a [`Discovery`] that has
/// just been read, so the three callers within one render share one request; and the fact that the
/// machine did not answer, so the page after this one does not pay for finding that out twice.
///
/// **It rides on the [`Client`], which is what makes it per-machine.** Choosing a machine builds a
/// new client, so nothing here can outlive the machine it is about, and nobody has to remember to
/// clear it.
#[derive(Debug, Default)]
struct Recent {
    /// The last answer to `/discover`, and when it arrived.
    discovery: Option<(Instant, Discovery)>,
    /// When the machine last failed to answer, while that is still worth assuming.
    silent_since: Option<Instant>,
}

/// How long a `/discover` answer is reused rather than asked for again.
///
/// **Long enough for one page and no longer.** The three callers within a render are microseconds
/// apart; a second that somebody spends reading the page is a second in which the machine may have
/// been renamed, and the next load should say so.
const DISCOVERY_HELD: Duration = Duration::from_millis(750);

/// How long a machine that did not answer is assumed to still not be answering.
///
/// **A page's own asks skip the wait; a button's do not.** What somebody presses is always tried
/// for real — a machine that has just come back must not refuse the press that proves it — and its
/// answer, either way, is what this is set or cleared from.
const SILENCE_HELD: Duration = Duration::from_secs(5);

/// Who a request is being made for, which decides whether silence is waited on again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asking {
    /// A page drawing itself. Refused at once while the machine is assumed silent.
    ForAPage,
    /// Something somebody pressed. Always tried, and its answer is what settles the assumption.
    ForAPerson,
}

impl Client {
    /// A client against one machine.
    ///
    /// `base` is a normalized URL — see [`normalize`].
    pub fn new(base: String) -> Result<Self, Refused> {
        // Shares `km-wallpaper-pack`'s `reqwest`, and therefore its missing rustls provider: without
        // this, `Client::new()` *panics*. The install is idempotent and public for exactly this.
        km_wallpaper_pack::providers::install_crypto_provider();

        let http = reqwest::Client::builder()
            .user_agent(concat!("km-admin/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| Refused::Local(format!("could not build an HTTP client: {error}")))?;
        Ok(Self {
            http,
            base,
            token: Arc::new(Mutex::new(None)),
            recent: Arc::new(Mutex::new(Recent::default())),
        })
    }

    /// The one place a request is put on the wire, and the one place silence is noticed.
    ///
    /// **`asking` is what separates a page from a person.** A page filling itself is refused
    /// immediately while [`SILENCE_HELD`] stands, because it would only be waiting to be told what
    /// this already knows; a button is tried for real whatever this thinks, or a machine that has
    /// come back could not be reached by the press that would prove it.
    ///
    /// Either way the outcome is recorded, so one successful press clears the assumption for
    /// everything else on the page.
    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        asking: Asking,
    ) -> Result<reqwest::Response, Refused> {
        if asking == Asking::ForAPage && self.lately_silent() {
            return Err(Refused::Unreachable(self.base.clone()));
        }
        match request.send().await {
            Ok(response) => {
                self.recent().silent_since = None;
                Ok(response)
            }
            Err(_) => {
                self.recent().silent_since = Some(Instant::now());
                Err(Refused::Unreachable(self.base.clone()))
            }
        }
    }

    /// Whether this machine has failed to answer recently enough to assume it still will.
    fn lately_silent(&self) -> bool {
        self.recent()
            .silent_since
            .is_some_and(|since| since.elapsed() < SILENCE_HELD)
    }

    /// The memo, with a poisoned lock recovered rather than panicked on.
    ///
    /// A cache is the last thing worth taking a program down over: the recovered value is at worst
    /// one stale answer, and the rule this follows is `A poisoned lock is recovered, never panicked
    /// on` in `CONTRIBUTING.md`.
    fn recent(&self) -> std::sync::MutexGuard<'_, Recent> {
        self.recent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The machine this is pointed at.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The full address of one [`Call`].
    ///
    /// **`km_api::routes::API_PREFIX` rather than a literal**, so the version this program speaks is
    /// the version the crate it decodes DTOs from serves. The path half is [`Call`]'s, and there is
    /// no overload taking a `&str` — that is the whole point of the type.
    ///
    /// **The id is already escaped** by the time it gets here: [`Call::path`] puts it through
    /// [`one_segment`], which is the only place that can do it safely — see that function for the
    /// traversal this arrangement replaces.
    fn url(&self, call: Call<'_>) -> String {
        format!("{}{}{}", self.base, km_api::routes::API_PREFIX, call.path())
    }

    fn token(&self) -> Option<String> {
        self.token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Whether this program is holding a token for the machine.
    pub fn has_token(&self) -> bool {
        self.token().is_some()
    }

    /// Puts a token in place without a round trip, for a test whose machine does not answer.
    ///
    /// **Not a shortcut around [`Self::log_in`]** — it exists because the send routes now refuse
    /// before they read the body when no password has been typed, and the in-crate tests that
    /// exercise the *other* gates point at a deliberately dead address, which is nothing to log in
    /// to. `#[cfg(test)]`, so it is not a way in that a shipped binary carries.
    /// Marks this machine silent without waiting for one to be, for a test about the memo.
    ///
    /// `#[cfg(test)]` for [`Self::pretend_signed_in`]'s reason: what it stands in for is a timeout,
    /// and a test that waited one out to reach the state under test would be spending
    /// [`ASK_TIMEOUT`] to arrange a `bool`.
    #[cfg(test)]
    pub(crate) fn pretend_silent(&self) {
        self.recent().silent_since = Some(Instant::now());
    }

    #[cfg(test)]
    pub(crate) fn pretend_signed_in(&self) {
        *self
            .token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some("a-test-token".to_owned());
    }

    /// What the machine says it is.
    ///
    /// Always public, on every machine, whatever its password — so this is the one call that can be
    /// made before anything is known.
    /// **Answered from [`Recent`] when one has just arrived**, because three separate callers ask
    /// this to draw one page — the heading wants the name, the information panel wants the addresses
    /// and the version, and the banner wants `factory_password` — and none of them can see the
    /// others. Three requests for one response was the shape of it, and against a machine that is
    /// not there, three waits.
    pub async fn discover(&self) -> Result<Discovery, Refused> {
        if let Some((read, discovery)) = self.recent().discovery.as_ref()
            && read.elapsed() < DISCOVERY_HELD
        {
            return Ok(discovery.clone());
        }

        let response = self
            .send(
                self.http.get(self.url(Call::Discover)).timeout(ASK_TIMEOUT),
                Asking::ForAPage,
            )
            .await?;
        let response = self.check(response).await?;
        let discovery: Discovery = response
            .json()
            .await
            .map_err(|error| Refused::Said(format!("its answer made no sense: {error}")))?;
        self.recent().discovery = Some((Instant::now(), discovery.clone()));
        Ok(discovery)
    }

    /// Whether this machine is still on the password it generated for itself.
    ///
    /// **Not "does it have one" any more, because every machine does.** What is worth asking is
    /// whether the owner has changed it -- so this tool can say so rather than leave a machine on a
    /// PIN that is written on its own television.
    pub async fn on_a_factory_password(&self) -> Result<bool, Refused> {
        Ok(self.discover().await?.factory_password)
    }

    /// Turns debugging mode on or off on the machine.
    ///
    /// **The switch that makes a curator's Play button work against a shipped machine.** The two
    /// `debug/play-*` routes are not mounted at all while it is off — a 404, not a refusal — so
    /// this is what a tool holding the password can do about that without anybody going to the box.
    pub async fn set_debugging(&self, enabled: bool) -> Result<(), Refused> {
        self.json_with::<_, serde_json::Value>(
            Call::SetDebugging,
            &serde_json::json!({ "enabled": enabled }),
        )
        .await
        .map(drop)
    }

    /// Exchanges the admin password for a bearer token, and keeps it.
    ///
    /// The machine rate-limits this — five failures per address per minute — and says so in its own
    /// words, which are passed through rather than replaced.
    pub async fn log_in(&self, password: &str) -> Result<(), Refused> {
        let response = self
            .send(
                self.http
                    .post(self.url(Call::Login))
                    .timeout(ASK_TIMEOUT)
                    .json(&LoginRequest {
                        password: password.to_owned(),
                    }),
                Asking::ForAPerson,
            )
            .await?;

        if !response.status().is_success() {
            let said = response.text().await.unwrap_or_default();
            return Err(Refused::Rejected(
                sentence(&said).unwrap_or_else(|| "that password was not accepted".to_owned()),
            ));
        }

        let granted: LoginResponse = response
            .json()
            .await
            .map_err(|error| Refused::Said(format!("its answer made no sense: {error}")))?;
        *self
            .token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(granted.token);
        Ok(())
    }

    /// What pictures the machine has.
    ///
    /// **A count and the one on screen, which is all the API offers.** There is no route that lists
    /// wallpapers by name and none that deletes one, so a page here can say "42 pictures, showing
    /// `beach.jpg`" and must not imply it could offer to replace them.
    pub async fn wallpapers(&self) -> Result<WallpapersDto, Refused> {
        self.get_json(Call::Wallpapers).await
    }

    /// What banks the machine has installed.
    pub async fn soundfonts(&self) -> Result<SoundFontsDto, Refused> {
        self.get_json(Call::SoundFonts).await
    }

    /// What packages the machine has installed, and how many songs they come to.
    ///
    /// `packages.read` ships public, like the two above, so this is the same bet they make: asked
    /// for and allowed to fail, leaving a line blank rather than stating something untrue.
    pub async fn packages(&self) -> Result<PackagesDto, Refused> {
        self.get_json(Call::Packages).await
    }

    /// Whether the machine is performing for itself, and whether that survives a restart.
    ///
    /// `demo.read` ships **public**, like the three above, so this is asked for on every machine
    /// page and allowed to fail — the card is drawn without it rather than not at all.
    pub async fn demo(&self) -> Result<DemoDto, Refused> {
        self.get_json(Call::Demo).await
    }

    /// Whether debugging mode is on this run, and what the settings file says.
    ///
    /// **The stored half is why this call exists.** `/discover` already reports the running value,
    /// and that is a snapshot taken when the machine started and cannot move — so a switch drawn
    /// from it said *Turn debugging on* both before the press and after it. Public, like the reads
    /// beside it.
    pub async fn debug(&self) -> Result<DebugDto, Refused> {
        self.get_json(Call::Debug).await
    }

    /// Whether the development console is asked for, and whether it is being served.
    ///
    /// Two facts because one explains nothing: the console needs debugging mode on as well, so a
    /// switch reporting only its own position would leave somebody who had turned it on looking at
    /// a `/dev/` that answers 404. Public, so it is asked for on every machine page.
    pub async fn dev_remote(&self) -> Result<DevRemoteDto, Refused> {
        self.get_json(Call::DevRemote).await
    }

    /// Turns the development console on or off.
    ///
    /// **What this opens is the machine's whole API again with no password on any of it**, at
    /// `/dev/api/v1`, so it is an admin write and the page above it says so in as many words.
    pub async fn set_dev_remote(&self, enabled: bool) -> Result<DevRemoteDto, Refused> {
        self.json_with(Call::SetDevRemote, &DevRemoteRequest { enabled })
            .await
    }

    /// Whether the frame-statistics panel is on the machine's screen.
    pub async fn performance(&self) -> Result<PerformanceDto, Refused> {
        self.get_json(Call::Performance).await
    }

    /// Puts the frame-statistics panel on the machine's screen, or takes it off.
    ///
    /// **The one switch on that page that needs no restart**, because it mounts nothing: the next
    /// frame draws it. This program is often the only thing that *can* press it — the appliance has
    /// no keyboard, so `F12` is not available on the machine that most wants this.
    pub async fn set_performance(&self, enabled: bool) -> Result<PerformanceDto, Refused> {
        self.json_with(Call::SetPerformance, &PerformanceRequest { enabled })
            .await
    }

    /// The output devices the machine has, and which one is sounding.
    ///
    /// Public, so the Sound page draws the picker before anybody has logged in — and the write
    /// below is what wants the password, which is the same shape every pair here has.
    pub async fn audio_outputs(&self) -> Result<AudioOutputsDto, Refused> {
        self.get_json(Call::AudioOutputs).await
    }

    /// Chooses the output device.
    ///
    /// **Refused with a 409 while anything is playing or queued**, because the player lives inside
    /// the audio stream a change has to drop. That is the machine's condition rather than a
    /// permission, and its own sentence is passed through rather than reworded.
    pub async fn set_audio_output(&self, id: &str) -> Result<AudioOutputsDto, Refused> {
        self.json_with(
            Call::SetAudioOutput,
            &AudioOutputRequest { id: id.to_owned() },
        )
        .await
    }

    /// Moves the level the chosen output runs at.
    ///
    /// **Not refused while a song plays**, unlike the device above it: the level belongs to the
    /// sound card rather than to the audio stream. Refused where the output has no level of its
    /// own, which is what an HDMI path answers, and the machine's sentence is passed through.
    pub async fn set_audio_level(&self, db: f32) -> Result<AudioOutputsDto, Refused> {
        self.json_with(Call::SetAudioLevel, &AudioLevelRequest { db })
            .await
    }

    /// Turns demo mode on or off, for this run or for good.
    ///
    /// `demo.write` ships **admin**, so this is one of the two calls on this page that can want a
    /// password — see [`Self::rename`].
    pub async fn set_demo(&self, enabled: bool, persist: bool) -> Result<DemoDto, Refused> {
        self.json_with(Call::SetDemo, &SetDemoRequest { enabled, persist })
            .await
    }

    /// Sets how long the machine waits before performing for itself, and it is always written down.
    ///
    /// **A second call rather than a third argument to [`Self::set_demo`]**, because the machine
    /// draws the same line: that route is *tonight or for good* and a delay is installation
    /// configuration with no run-only half to ask about. Admin, like its neighbour.
    pub async fn set_demo_delay(&self, delay_secs: u32) -> Result<DemoDto, Refused> {
        self.json_with(Call::SetDemoDelay, &SetDemoDelayRequest { delay_secs })
            .await
    }

    /// Renames the machine.
    ///
    /// **One of the things this program changes about a machine rather than adding to it**, and it is
    /// here because this is the program somebody has open while setting a machine up — see
    /// `A fourth program, rather than a fourth tab on the owner's page` in
    /// docs/decisions/distribution.md.
    ///
    /// The machine tidies and validates the name; whatever it says about one it refuses is passed
    /// through rather than second-guessed here, exactly as an upload's refusal is.
    ///
    /// **`/admin/machine/name`, and the prefix is not decoration.** `put_machine_name` is registered
    /// inside the `admin` router, which is nested at `/admin`, so the full path is
    /// `/api/v1/admin/machine/name`. This asked for `/api/v1/machine/name` and got a 404 that reached
    /// the page as a toast about the machine rather than about the route — every rename from this
    /// program failed, quietly, for as long as the button existed. The test below is what would now
    /// notice.
    pub async fn rename(&self, name: &str) -> Result<(), Refused> {
        let _: MachineNameRequest = self
            .json_with(
                Call::Rename,
                &MachineNameRequest {
                    name: name.to_owned(),
                },
            )
            .await?;
        Ok(())
    }

    /// What language the machine's television draws in.
    ///
    /// Public, like the three reads above, so the *Screen language* pane is drawn on a machine this
    /// program has not logged in to — and the picker shows the machine's real answer rather than a
    /// guess. A machine speaking a locale this build has no catalog for reads as
    /// `Locale::default()`: the tag crosses as a string precisely so that it can be read at all.
    pub async fn locale(&self) -> Result<km_locale::Locale, Refused> {
        let answer: MachineLocaleRequest = self.get_json(Call::Locale).await?;
        Ok(km_locale::Locale::parse(&answer.locale).unwrap_or_default())
    }

    /// Sets what language the machine's television draws in.
    ///
    /// **The television's language and not this page's.** The pane says which of the two it means,
    /// and the confirmation is worded in the language just chosen — so somebody who picked the
    /// wrong one finds out at the browser rather than by walking to the machine.
    ///
    /// A tag the machine has no catalog for comes back as its refusal rather than being silently
    /// resolved here, for [`Self::rename`]'s reason: the machine is the one that knows.
    pub async fn set_locale(&self, locale: km_locale::Locale) -> Result<(), Refused> {
        let _: MachineLocaleRequest = self
            .json_with(
                Call::SetLocale,
                &MachineLocaleRequest {
                    locale: locale.tag().to_owned(),
                },
            )
            .await?;
        Ok(())
    }

    /// Changes the machine's admin password.
    ///
    /// **The third thing this program changes about a machine rather than adds to it**, and it is
    /// here for the reason the other two are: this is the program somebody has open while a machine
    /// is being set up, and a box still answering to the PIN printed on its own television is the
    /// thing this program is best placed to notice — it is where the machine was found.
    ///
    /// **It only ever sets a password, never `None`.** `{"password": null}` resets the machine to a
    /// freshly generated PIN, which is destructive and belongs where it already is: the machine's own
    /// `/admin/` page, and the television that then shows the new one. What is missing from a box
    /// under a set is the *first change*, and that is all this does.
    ///
    /// **It logs this program out, and the caller has to expect that.** Tokens are HMACs keyed on the
    /// stored hash, so changing the password revokes every one of them, this program's included. The
    /// token is dropped here rather than left to fail on the next request, so the page comes back
    /// offering the login form instead of a control that would answer 401.
    pub async fn set_password(&self, password: &str) -> Result<(), Refused> {
        let outcome = self
            .json_with::<_, AdminPasswordDto>(
                Call::SetPassword,
                &AdminPasswordRequest {
                    password: Some(password.to_owned()),
                },
            )
            .await;
        if outcome.is_ok() {
            self.forget_token();
        }
        outcome.map(drop)
    }

    /// Ends every session on the machine, this program's included.
    ///
    /// **Password-independent, and that is the point** — the same argument the machine's own page
    /// makes: an owner who wants every phone in the house logged out should not have to change the
    /// password and then tell the house the new one.
    ///
    /// **The token goes with it**, for [`Self::set_password`]'s reason exactly: a session epoch that
    /// has moved invalidates every outstanding token, so holding one here would only produce a 401 on
    /// the next request. Dropped now, so the page comes back offering the login form rather than a
    /// control that answers 401.
    ///
    /// No body, and none is read back: this is an act, and its answer is that it happened.
    pub async fn reset_sessions(&self) -> Result<(), Refused> {
        let outcome = self.act(Call::ResetSessions).await;
        if outcome.is_ok() {
            self.forget_token();
        }
        outcome
    }

    /// Which bank is loaded, and whether all of it loaded.
    pub async fn loaded_bank(&self) -> Result<SoundFontDto, Refused> {
        self.get_json(Call::LoadedBank).await
    }

    /// Uninstalls a package, and answers how many songs went with it.
    ///
    /// **The count comes back from the machine rather than being remembered here.** The page says
    /// *"Removed vol1 and its 240 songs"*, and the only honest source for that number is the machine
    /// that did the removing — a count read before the call would be a claim about what it was about
    /// to do.
    pub async fn remove_package(&self, id: &str) -> Result<usize, Refused> {
        let removed: UninstallDto = self.get_json(Call::RemovePackage(id)).await?;
        Ok(removed.songs_removed)
    }

    /// Moves a package's songs into another bank, and answers how many changed number.
    pub async fn set_package_bank(&self, id: &str, bank: u16) -> Result<usize, Refused> {
        let moved: BankDto = self
            .json_with(Call::SetPackageBank(id), &BankRequest { bank })
            .await?;
        Ok(moved.songs_renumbered)
    }

    /// Shows the next picture in the rotation.
    pub async fn next_wallpaper(&self) -> Result<(), Refused> {
        self.act(Call::NextWallpaper).await
    }

    /// Takes a picture out of the rotation.
    pub async fn remove_wallpaper(&self, id: &str) -> Result<(), Refused> {
        self.act(Call::RemoveWallpaper(id)).await
    }

    /// Plays through a bank the machine already has.
    ///
    /// **The id goes in the body and not the path**, which is the machine's own shape for this one:
    /// `PUT /admin/audio/soundfont` replaces *which bank is in force*, so the thing being addressed
    /// is the setting rather than the bank.
    pub async fn use_bank(&self, id: &str) -> Result<(), Refused> {
        self.json_with::<_, SoundFontDto>(Call::UseBank, &SoundFontRequest { id: id.to_owned() })
            .await
            .map(drop)
    }

    /// Deletes a bank off the machine.
    pub async fn remove_bank(&self, id: &str) -> Result<(), Refused> {
        self.act(Call::RemoveBank(id)).await
    }

    /// Drops the bearer token, so the next page asks for the new password.
    fn forget_token(&self) {
        *self
            .token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// One call carrying a JSON body.
    ///
    /// **The verb is not an argument.** A `put_json`/`post_json` pair over this function would let a
    /// caller choose the verb and the path independently — a name is `PUT` because it replaces one, a
    /// password is `POST` because it is an act — and that is one of the two ways a request can miss:
    /// a right path under a wrong verb, which nothing in this file could notice. [`Call::method`]
    /// answers it from the same value that answers the path, so the pair is chosen once.
    async fn json_with<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        call: Call<'_>,
        body: &B,
    ) -> Result<T, Refused> {
        let mut request = self
            .http
            .request(call.method(), self.url(call))
            .timeout(ASK_TIMEOUT)
            .json(body);
        if let Some(token) = self.token() {
            request = request.bearer_auth(token);
        }
        // `ForAPerson`: every one of these is a `PUT` or a `POST`, which is something pressed.
        let response = self.send(request, Asking::ForAPerson).await?;
        let response = self.check(response).await?;
        response
            .json()
            .await
            .map_err(|error| Refused::Said(format!("its answer made no sense: {error}")))
    }

    /// One call with no body and no answer to read.
    ///
    /// **Its own helper rather than `json_with::<(), ()>`**, which would send the four bytes `null`
    /// as a JSON body and then insist on parsing a reply. The routes this suits are acts: what they
    /// answer is that they happened, and [`Self::check`] is what turns anything else into a
    /// [`Refused`] carrying the machine's own sentence.
    async fn act(&self, call: Call<'_>) -> Result<(), Refused> {
        let mut request = self
            .http
            .request(call.method(), self.url(call))
            .timeout(ASK_TIMEOUT);
        if let Some(token) = self.token() {
            request = request.bearer_auth(token);
        }
        // An act is something pressed, so it is tried whatever the last one came to.
        let response = self.send(request, Asking::ForAPerson).await?;
        self.check(response).await.map(drop)
    }

    /// One read, for a page filling itself.
    ///
    /// **`ForAPage`, and this is the helper that makes the memo worth having.** The Debugging pane
    /// alone is four of these, and the page around it is more — so against a machine that is not
    /// answering, this is where a load's whole wait used to accumulate.
    async fn get_json<T: serde::de::DeserializeOwned>(&self, call: Call<'_>) -> Result<T, Refused> {
        let mut request = self
            .http
            .request(call.method(), self.url(call))
            .timeout(ASK_TIMEOUT);
        if let Some(token) = self.token() {
            request = request.bearer_auth(token);
        }
        let response = self.send(request, Asking::ForAPage).await?;
        let response = self.check(response).await?;
        response
            .json()
            .await
            .map_err(|error| Refused::Said(format!("its answer made no sense: {error}")))
    }

    /// Sends a file to one of the machine's upload routes, under the name it has on disk.
    ///
    /// The part is named `file`, which is `km_api::uploads::FILE_FIELD` for all three of them.
    pub async fn upload(&self, call: Call<'_>, file: &std::path::Path) -> Result<String, Refused> {
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| Refused::Local(format!("{} has no filename", file.display())))?;
        self.upload_as(call, file, &name).await
    }

    /// The same, under a name the caller chose.
    ///
    /// **Two names, because the file on this disk and the file on the wire are not the same thing.**
    /// A file somebody picked in a browser is staged here under a name this program invented — the
    /// rule `bank.rs` states, that nothing is ever written under a name the network chose — while
    /// the machine needs the name the *person* chose: it reads the extension to decide what the file
    /// is, and the stem becomes the bank's name or the picture's. Sending the staging name would put
    /// a SoundFont called `7` on somebody's machine. The machine sanitises what it is given through
    /// its own `safe_stem`, which is why this passes it through untouched rather than growing a
    /// second, disagreeing copy of that function.
    pub async fn upload_as(
        &self,
        call: Call<'_>,
        file: &std::path::Path,
        wire_name: &str,
    ) -> Result<String, Refused> {
        // **The floor under every send, and it is here rather than only at the call sites.** All
        // three upload routes are admin routes, and a 401 arriving part-way through a multipart
        // stream reads as a dropped connection to the sending half — so the refusal somebody most
        // needs is the one least likely to arrive. `km-package-builder` pre-flights its token for
        // this reason; the difference is that this is the *floor*, reached by a caller that forgot,
        // which is the failure this whole file is being repaired for.
        //
        // It does not cover a token that has **expired** — that still fails on the wire, and
        // `check` drops the stale one so the next page draw asks again.
        if !self.has_token() {
            return Err(Refused::Unauthorized);
        }

        let handle = tokio::fs::File::open(file)
            .await
            .map_err(|error| Refused::Local(format!("could not read {wire_name}: {error}")))?;
        let length = handle
            .metadata()
            .await
            .map_err(|error| Refused::Local(format!("could not measure {wire_name}: {error}")))?
            .len();

        // **`stream_with_length` rather than `Part::bytes`**, which read the whole file into memory:
        // a package may be two gibibytes and a bank one. The length is given rather than left to
        // chunked encoding, which is what `Form::compute_length` needs in order to give the request
        // a real `Content-Length` — and that is what lets the machine's own `DefaultBodyLimit`
        // refuse an oversized upload before reading any of it.
        //
        // Not `Part::file`, which would take the name from the staging path and guess a media type
        // from it. The name is the caller's business here and the machine does not read the type.
        let part = reqwest::multipart::Part::stream_with_length(handle, length)
            .file_name(wire_name.to_owned());
        let form = reqwest::multipart::Form::new().part(km_api::uploads::FILE_FIELD, part);

        let mut request = self
            .http
            .post(self.url(call))
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form);
        if let Some(token) = self.token() {
            request = request.bearer_auth(token);
        }

        // A file somebody chose, so it is sent whatever the last read came to.
        let response = self.send(request, Asking::ForAPerson).await?;
        let response = self.check(response).await?;
        let report: UploadReportDto = response
            .json()
            .await
            .map_err(|error| Refused::Said(format!("its answer made no sense: {error}")))?;
        Ok(report.report)
    }

    /// Maps a response's status onto the refusals a page knows how to act on.
    ///
    /// **401 and 403 are one case here**, because the page's answer to both is the same: show a
    /// password field. The machine distinguishes "no token" from "token not good enough"; nothing
    /// this program can do differs between them.
    async fn check(&self, response: reqwest::Response) -> Result<reqwest::Response, Refused> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // A token that has expired is indistinguishable from never having had one, and the
            // answer is the same, so the stale one goes rather than being retried into a loop.
            *self
                .token
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            return Err(Refused::Unauthorized);
        }
        let said = response.text().await.unwrap_or_default();
        Err(Refused::Said(
            sentence(&said).unwrap_or_else(|| format!("it answered {status}")),
        ))
    }
}

/// The machine's own words out of a refusal body.
///
/// **Every error the API produces is `{"error": "<code>", "message": "<a sentence>"}`**, and the
/// sentence is written for a person — *"incorrect password"*, *"'admin.password.write' is
/// admin-only; send an admin token"*. Passing the whole body through instead puts a JSON blob in a
/// toast, which is what this program did until somebody read one.
///
/// `None` for a body with nothing usable in it, so the caller can supply its own words. A body that
/// is not JSON at all is still worth showing if it is short — axum's own multipart rejection is
/// plain text, and so is anything a proxy in the way might answer with.
fn sentence(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(message) = value.get("message").and_then(|message| message.as_str())
    {
        return Some(message.to_owned());
    }
    // Not JSON, or JSON without a message. Anything long is a document rather than an explanation.
    (trimmed.len() <= 300 && !trimmed.starts_with('<')).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page's asks skip a machine assumed silent; a person's are tried anyway, and settle it.
    ///
    /// **The three properties the memo has to have, and the middle one is the trap.** Short-circuiting
    /// everything would mean a machine that has come back could not be reached by the press that
    /// would prove it, and nothing would ever clear the assumption but waiting.
    ///
    /// Asserted on the mock's request count rather than on how long a call took: what is under test
    /// is whether a request was made, and a timing assertion would be a slow way to ask that
    /// question badly.
    #[tokio::test]
    async fn a_page_does_not_wait_on_a_silent_machine_and_a_person_still_can() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "{}{}",
                km_api::routes::API_PREFIX,
                Call::Login.path()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "a-token",
                "expires_in_secs": 43200,
            })))
            .mount(&server)
            .await;

        let client = Client::new(normalize(&server.uri())).expect("a client");
        client.pretend_silent();

        // A page's ask: refused out of the memo, with nothing put on the wire.
        assert!(
            matches!(client.discover().await, Err(Refused::Unreachable(_))),
            "a page's ask is answered from the memo"
        );
        assert_eq!(
            server.received_requests().await.map(|seen| seen.len()),
            Some(0),
            "and it asked the machine nothing"
        );

        // A person's: tried, and it is what settles the question either way.
        client.log_in("first1975").await.expect("the mock answers");
        assert_eq!(
            server.received_requests().await.map(|seen| seen.len()),
            Some(1),
            "a press is not refused out of the memo"
        );

        // Cleared by that answer, so the rest of the page stops being refused.
        assert!(
            client.discover().await.is_err(),
            "this mock mounts no /discover, so the ask reaches it and 404s"
        );
        assert_eq!(
            server.received_requests().await.map(|seen| seen.len()),
            Some(2),
            "which is only possible because the memo was cleared"
        );
    }

    #[test]
    fn a_bare_address_gets_the_machines_own_port() {
        // The port comes from `km-api`, so this asserts the rule rather than the number.
        assert_eq!(
            normalize("192.168.1.5"),
            format!("http://192.168.1.5:{MACHINE_DEFAULT_PORT}")
        );
        assert_eq!(
            normalize("  living-room  "),
            format!("http://living-room:{MACHINE_DEFAULT_PORT}")
        );
    }

    #[test]
    fn a_stated_port_or_scheme_is_left_alone() {
        assert_eq!(normalize("192.168.1.5:9000"), "http://192.168.1.5:9000");
        assert_eq!(normalize("http://box:8177/"), "http://box:8177");
        assert_eq!(
            normalize("https://box"),
            format!("https://box:{MACHINE_DEFAULT_PORT}")
        );
    }

    #[test]
    fn the_api_prefix_is_added_once() {
        let client = Client::new("http://box:8177".to_owned()).expect("client");
        assert_eq!(
            client.url(Call::Discover),
            "http://box:8177/api/v1/discover"
        );
        assert_eq!(
            client.url(Call::SoundFonts),
            "http://box:8177/api/v1/audio/soundfonts"
        );
        assert_eq!(
            client.url(Call::Send(km_api::machine::Upload::Package)),
            "http://box:8177/api/v1/admin/packages/upload"
        );
    }

    /// Every call this program can make is a route the machine actually mounts, under that verb.
    ///
    /// **This is the test that was missing, and its absence cost the program every file it ever
    /// sent.** `tests/upload.rs` drove the three uploads against a `wiremock` built from the same
    /// constants the client sends to, so it asserted the client against itself and answered 200 to a
    /// request a real machine 404s. The truth is [`km_api::routes::SURFACE`] — the table the
    /// machine's own integration tests drive a request at every entry of — and this is the only
    /// thing in this program that consults it.
    ///
    /// **The verb is half the assertion.** `/wallpapers` and `/audio/soundfonts` are in `SURFACE` as
    /// `GET`, so a check for the path alone would have called two of the three broken sends correct.
    ///
    /// Run against the code as it was, this fails on three of thirteen: `("POST",
    /// "/packages/upload")` is not in the table at all, and `("POST", "/wallpapers")` and `("POST",
    /// "/audio/soundfonts")` are not in it as POSTs.
    #[test]
    fn every_call_this_program_makes_is_a_route_the_machine_mounts() {
        for call in Call::ALL {
            let (method, path) = (call.method(), call.path());
            // **The six calls that name a thing carry `SURFACE`'s own sample id**, because that
            // table is concrete samples rather than axum patterns — its own doc says `{id}` never
            // appears in it. So this compares a whole built path against a whole declared one, and
            // a route whose *shape* changed (a segment added, `/bank` renamed) fails here.
            // **`LEVEL_SURFACE` as well, because that route is mounted only where the host has a
            // mixer to reach.** It is kept out of `SURFACE` so the machine's own sweep does not
            // drive it against a harness that has no level; a program calling it still has to find
            // it declared somewhere, and this is where the two tables are read as one.
            let declared = km_api::routes::SURFACE
                .iter()
                .chain(km_api::routes::LEVEL_SURFACE)
                .any(|(m, p)| *m == method.as_str() && *p == path.as_ref());
            assert!(
                declared,
                "{call:?} asks for {method} {path}, which the machine does not mount"
            );
        }
    }

    /// The id in a path is escaped, and the sample ids are not what makes that true.
    ///
    /// **A bank's id is a file name and a picture's is a slug of one**, neither of this program's
    /// choosing, so a space or a `?` would otherwise build a request nobody asked for and a `../` a
    /// different route entirely. Asserted on the URL rather than on the path, because
    /// [`Call::path`] deliberately leaves it raw so the sweep above can match a route table.
    #[test]
    fn an_id_with_something_awkward_in_it_cannot_reshape_a_url() {
        let client = Client::new("http://machine.invalid:8177".to_owned()).expect("a client");
        let url = client.url(Call::RemoveBank("../../admin/password"));
        assert!(
            !url.contains("/admin/password"),
            "a crafted id reached another route: {url}"
        );
        let spaced = client.url(Call::RemoveWallpaper("holiday snap.jpg"));
        assert!(
            !spaced.contains(' '),
            "a space survived into a URL: {spaced}"
        );
        assert!(
            spaced.contains("holiday%20snap.jpg") || spaced.contains("holiday%20snap"),
            "the name did not survive its escaping: {spaced}"
        );
    }

    #[test]
    fn a_refusal_shows_the_machines_sentence_and_not_its_json() {
        // What the machine actually answers a bad password with. Showing the whole body put
        // `{"error":"unauthorized","message":"incorrect password"}` in a toast.
        assert_eq!(
            sentence(r#"{"error":"unauthorized","message":"incorrect password"}"#).as_deref(),
            Some("incorrect password")
        );
        // Plain text still comes through — axum's own multipart rejection is not JSON.
        assert_eq!(
            sentence("Failed to parse the request body").as_deref(),
            Some("Failed to parse the request body")
        );
        // Nothing usable, so the caller supplies its own words.
        assert_eq!(sentence("   "), None);
        assert_eq!(sentence("<html><body>502 Bad Gateway</body></html>"), None);
        assert_eq!(sentence(&"x".repeat(400)), None);
        // JSON without a message is not a sentence either, so it falls through to the length rule.
        assert_eq!(
            sentence(r#"{"error":"teapot"}"#).as_deref(),
            Some(r#"{"error":"teapot"}"#)
        );
    }

    #[test]
    fn a_fresh_client_holds_no_token() {
        // The state a machine with no password leaves it in for ever, which is most machines.
        let client = Client::new("http://box:8177".to_owned()).expect("client");
        assert!(!client.has_token());
    }
}
