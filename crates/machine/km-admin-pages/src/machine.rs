//! What the owner's page needs from a machine, and nothing about how it gets there.
//!
//! **These traits are the seam that lets one page set serve two hosts.** The machine implements them
//! in its own process, calling `km_api::ops::*` and the `Controller` and `Catalog` trait objects
//! directly; `km-admin` implements them over HTTP against a machine on the network. Neither
//! implementation lives here, and that is the point — this crate holds the markup, the words and the
//! rules about what a page shows, and knows nothing about a wire.
//!
//! It is `km-remote-pages`' arrangement, deliberately: that crate serves an online remote and an
//! offline one from one set of templates behind four `dyn` traits, and the drift this crate was
//! written beside — two admin surfaces with the same settings pane copied into both — is the same
//! problem one layer over. See `Two admin surfaces, one vocabulary` in
//! `docs/decisions/distribution.md`.
//!
//! # Why every method is `async` when one host answers from memory
//!
//! **So that *how to get off the runtime* is the implementation's decision rather than this crate's.**
//! The in-process host reads a catalog behind a mutex that an install holds for **seconds** — and
//! `crates/machine/karaokemachine/src/remote.rs` is the written account of what ignoring that costs:
//! *"every page of this remote would sit on a worker waiting, and with as many workers as the box has
//! cores, the API, the pages and the event stream stop together."* A synchronous trait would force
//! that decision up here, where the crate has no idea which reads are cheap.
//!
//! # Why the methods are grouped by tab
//!
//! One trait per tab, because the tabs *are* the vocabulary decision: a person who has learned one
//! surface reads the other by its tab strip, and a seam shaped the same way can be read against it.
//! `km-remote-pages` groups by capability instead, which is right there — its modes differ by
//! feature — and wrong here, where they differ by which errand a program is for.

use std::borrow::Cow;

/// What this host's surface can do, as the templates read it.
///
/// **Gates controls and panes *within* a page, and one whole tab.** That is narrower than
/// `km-remote-pages`' `Capabilities`, which gates features across two modes of one product — and the
/// difference is deliberate, because these two surfaces are by standing decision *not* one product:
/// one searches the internet for pictures and the other must never. A tab only one of them draws is
/// therefore **a router the host merges**, not a flag here. See
/// `A fourth program, rather than a fourth tab on the owner's page`.
///
/// **A control whose capability is off is absent, not disabled**, which is the rule that crate states
/// and this one keeps: a grayed button invites a press that can never work. The exception is a
/// control the *machine* refuses for a reason that will pass — see [`AdminError::Busy`] — which is
/// drawn and answered, because a device that cannot be changed now can be changed when the song ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Listing installed packages, removing one, and moving one between blocks of numbers.
    ///
    /// **On for both hosts**, which is `What it is not is a second /admin/`: one page set draws these
    /// controls, so a host rendering them is not a second place to keep right.
    ///
    /// **A host has to implement them, and the flag alone draws nothing.** Every control here needs a
    /// real answer from `Songs` — a table with no rows and a Remove that refuses is what a flag
    /// turned on over stubs produces.
    ///
    /// One thing a host over HTTP cannot show: a package's **size**. `PackageDto` publishes no byte
    /// count, so that column is empty there and holds a number on the machine's own page.
    pub installed_packages: bool,
    /// The wallpaper rotation: what is in it, which is showing, delete, and show the next.
    ///
    /// On for both, on [`Self::installed_packages`]' footing exactly.
    pub rotation: bool,
    /// The banks installed on the machine: the list, *use this one*, and remove.
    ///
    /// On for both, same footing again. **Not the same list as `km-admin`'s own fetch table**, which
    /// is sixty-odd banks it could download; this is the handful the machine already has. The host
    /// deliberately does not pass the machine's `offers` or `fetching` across, or the page would
    /// carry two downloaders.
    pub installed_banks: bool,
    /// The Problems tab, its nav entry and its badge.
    ///
    /// **Machine only, and this one is not a free choice.** Its rows are identified by a path on the
    /// machine, which `PackageProblemDto` refuses to publish. [`Admin::problems`](crate::Admin) is
    /// what answers the tab; this is what draws it.
    pub problems: bool,
    /// The *Screen language* pane.
    ///
    /// **On for both**, on [`Self::installed_packages`]' footing: `GET /locale` reports what the
    /// television draws in and `PUT /admin/machine/locale` sets it, so a host over HTTP has the
    /// same answer the machine has in process.
    ///
    /// **A machine is set up at a desk and stood in front of afterwards.** The language is one of
    /// the first things wrong about a box that arrives speaking the wrong one, and the program an
    /// owner has open while setting one up is this one — the same argument the name and the
    /// password already make, and the pane sits beside both.
    pub screen_language: bool,
    /// Putting the machine back on a freshly generated PIN.
    ///
    /// **Absent in a tool, and the reason is where the PIN goes.** A reset draws the new PIN on the
    /// machine's own television, so the only place the act makes sense is beside the screen that
    /// will then show it. `km-admin` changes the password and does not reset it — *"what is missing
    /// from a box under a television is the first change; the recovery is not missing, it is
    /// somewhere better."*
    pub password_reset: bool,
    /// That this host is *pointed at* a machine rather than being one: the strip's way back to its
    /// front door, and the line on the Machine tab saying whether the program is logged in.
    ///
    /// **A tool only.** A machine has no say over which machine it is and no session to hold, so
    /// its own `/admin/` draws neither half. That page is behind the password already —
    /// [`Self::gate_every_route`] sends a caller without a token to `/admin/login` — where a tool
    /// has no door of its own to be sent through and can write nothing until it holds a token.
    ///
    /// **What this does not draw is the address box or the password field.** Both are on the host's
    /// front door, which is the page a tool opens on and the page every refusal for want of a
    /// password comes back to — see `The front door is where a tool is pointed and let in` in
    /// `docs/decisions/distribution.md`. A pane on a tab is the wrong place for either: the moment
    /// they are wanted is the moment every fact on this tab is blank, because the machine has gone
    /// away.
    pub choose_machine: bool,
    /// Links out to a host's own picture-search and bank-fetching pages.
    ///
    /// # Why a link rather than the thing itself
    ///
    /// **The searching cannot be in this crate**, and that is
    /// `A fourth program, rather than a fourth tab on the owner's page` rather than a limitation: the
    /// machine may have no internet, it must not hold somebody's Pixabay key, and a hundred JPEG
    /// decodes is not what a box should do while playing a song. Nothing here implements it, so the
    /// machine could not serve it if the markup were present.
    ///
    /// It also could not usefully be: the review grid needs a progress sink and `km-wallpaper-pack`'s
    /// own types, and a trait over those with exactly one possible implementation forever is a seam
    /// that buys nothing.
    ///
    /// So the host serves those pages itself — through [`Admin::shell`](crate::Admin::shell), so they
    /// wear the same chrome — and this draws the link that reaches them. **The host must serve
    /// `/admin/pictures/find` and `/admin/sound/fetch`**, which is the one place this crate names a
    /// route it does not own.
    pub searching: bool,
    /// Whether the guard middleware refuses every route but the two open ones.
    ///
    /// # Why this is a capability and not simply always on
    ///
    /// **The machine's guard exists because `/admin/` is reachable from every phone on the LAN.** An
    /// unlisted route there is a control-panel button somebody forgot to gate, and the failure has to
    /// be a refusal rather than an open door.
    ///
    /// A tool on loopback is a different question. It has no password of its own by decision — its
    /// own `--lan` help text says *"this program stores your API keys, can write files as you, and
    /// has no password"* — so a guard there is not protecting *this* surface; it only tracks whether
    /// **the machine** will accept us. Its standing shape is *try, and ask for the password only if
    /// refused*, because reads ship public and a page that opened with a login form would demand a
    /// credential before there was anything to spend it on.
    ///
    /// # What `false` means
    ///
    /// **The middleware is the only caller of [`Guard::allows`](crate::guard::Guard::allows) that
    /// stands between a caller and a write**, and no handler consults it before one. So this flag
    /// chooses between one mechanism and *none*: with it off, nothing in this crate authorizes any
    /// write.
    ///
    /// The Machine tab asks `allows` once per load, and that is a **read** rather than a gate: on a
    /// tool the answer is whether the program holds a token, which is the sentence
    /// [`Self::choose_machine`] draws. Nothing is refused by it.
    ///
    /// **Do not repair that with per-handler checks.** On a tool `allows` answers *do we hold a
    /// token*, a fact about that program's session rather than about the browser making the request.
    /// The authority on whether a write may happen is the machine, which refuses an untokened call
    /// with a 401 that arrives as [`AdminError::Unauthorized`] and is worded here. A local check
    /// before each write could refuse a write the machine would have taken.
    ///
    /// `a_refused_guard_stops_a_write_on_the_machine_and_the_machine_decides_for_a_tool` asserts both
    /// sides: the machine's write must not happen, and a tool's must reach the machine.
    pub gate_every_route: bool,
}

impl Capabilities {
    /// What the machine serves at `/admin/`.
    #[must_use]
    pub fn machine() -> Self {
        Self {
            installed_packages: true,
            rotation: true,
            installed_banks: true,
            problems: true,
            screen_language: true,
            password_reset: true,
            choose_machine: false,
            searching: false,
            gate_every_route: true,
        }
    }

    /// What `km-admin` serves on a desktop.
    ///
    /// **The three `installed_*` flags are on**, which is `What it is not is a second /admin/`: a
    /// tool that sends a package sees the package it sent, corrects the bank it landed in, and takes
    /// it off again.
    ///
    /// What is off is what a tool cannot answer or should not: the Problems tab, whose rows are
    /// identified by a path the API refuses to publish, and the password *reset*, which draws a new
    /// PIN on a television this program is not standing in front of.
    #[must_use]
    pub fn desktop() -> Self {
        Self {
            installed_packages: true,
            rotation: true,
            installed_banks: true,
            problems: false,
            screen_language: true,
            password_reset: false,
            choose_machine: true,
            searching: true,
            // Loopback, and no password of its own. See the field.
            gate_every_route: false,
        }
    }
}

/// Why an operation did not happen, in a form either host can produce.
///
/// **`km-admin-pages` needed no error vocabulary while it had one host**, and its own `words` module
/// said so: *"These pages are served by the machine itself, in its own process, so nothing arrives
/// from across a wire needing a code to be rendered from."* A second host makes that false, and this
/// is `km-remote-pages`' `RemoteError` for the same reason it exists there — a page has to say what
/// went wrong in the reader's language, and a code is the only thing that crosses a wire without
/// picking one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminError {
    /// The machine could not be reached at all. Carries a code, not a sentence.
    ///
    /// Only a host that talks over a network can produce this; the in-process one is the machine.
    #[error("the machine is not answering: {0}")]
    Offline(&'static str),
    /// The machine refused for want of a password, or the token has gone stale.
    ///
    /// The page answers with its login form. **Not the same as the guard refusing** — that happens
    /// before a handler runs; this is the machine changing its mind mid-session, which a remote host
    /// finds out about only by being told no.
    #[error("the machine wants its password")]
    Unauthorized,
    /// The thing named is not there — a package, a bank, a picture.
    #[error("not found")]
    NotFound,
    /// The machine refused, and the refusal is **the machine's own sentence**.
    ///
    /// **Passed through and never re-worded.** The machine is the authority on what a name may be and
    /// on what a `.kmpkg` contains, and a second opinion composed here would be a second thing to
    /// keep in agreement with it — which is the rule `km-admin` already keeps by unwrapping the error
    /// body to its `message`. So this variant is deliberately *not* localized: it arrives in the
    /// machine's words or not at all.
    #[error("{0}")]
    Refused(String),
    /// The machine is busy and the same request would work later. **A warning, not an error.**
    ///
    /// **Kept apart from [`Self::Refused`] because the page says something different about it**, and
    /// flattening the two is the kind of thing a careless refactor does silently. Choosing an output
    /// device while a song is loaded is the case: the player lives inside the stream a change has to
    /// drop, so the machine answers *busy* — and *busy* is `warn`, where *there is no such device* is
    /// `bad`. One reads as "try again when the song ends" and the other as "something is wrong with
    /// what you sent".
    ///
    /// Carries the machine's own sentence, as `Refused` does. A host over HTTP produces this from a
    /// **409**.
    #[error("{0}")]
    Busy(String),
    /// Something else broke, in words for the log rather than for a page.
    #[error("{0}")]
    Failed(String),
}

/// Every message key an [`AdminError`] can render as.
///
/// **Listed rather than scanned, and this is the one place this crate departs from `words`' habit.**
/// That module's scanner finds `msg("…")` and `msg_with("…")` calls in `handlers.rs`, which works
/// because every key there *is* such a call. These are not: they are a `match` mapping variants to
/// keys, and a scanner looking for a call shape would report all three as messages nothing asks for.
///
/// The list is what `no_message_is_left_unused` reads, and [`AdminError::key`] is what produces them
/// — [`every_error_key_is_listed`](self) asserts the two agree, so a new variant cannot quietly add a
/// key no catalog has.
pub const ERROR_KEYS: &[&str] = &["error-offline", "error-unauthorized", "error-not-found"];

impl AdminError {
    /// The message key a page should render this as, or `None` to print [`Self::Refused`]'s sentence.
    ///
    /// **A key rather than a sentence**, which is the whole shape of this type: the host that
    /// produced the fault may be a program with no Fluent catalog talking to a machine in another
    /// language, and the page doing the rendering is the one that knows what the reader speaks. The
    /// same division `km-remote-core` keeps with `words::code_key`.
    #[must_use]
    pub fn key(&self) -> Option<&'static str> {
        match self {
            Self::Offline(_) => Some("error-offline"),
            Self::Unauthorized => Some("error-unauthorized"),
            Self::NotFound => Some("error-not-found"),
            // These carry words already. `Refused` and `Busy` carry the machine's, which is the
            // whole point of them; `Failed` carries a developer's, which a page shows rather than
            // hides because the alternative is a control that silently does nothing.
            Self::Refused(_) | Self::Busy(_) | Self::Failed(_) => None,
        }
    }

    /// Which kind of banner this deserves: `warn` for a fault that will pass, `bad` for one that
    /// will not.
    ///
    /// **One place decides**, so a handler cannot get the distinction wrong by forgetting it exists.
    /// [`Self::Busy`] is the whole reason there is a choice — see that variant.
    #[must_use]
    pub fn severity(&self) -> &'static str {
        match self {
            Self::Busy(_) => "warn",
            _ => "bad",
        }
    }

    /// Whether this is the machine asking for its password, which a page answers with a form.
    #[must_use]
    pub fn wants_password(&self) -> bool {
        matches!(self, Self::Unauthorized)
    }
}

/// The three developer switches and demo mode, as one snapshot.
///
/// **One read for all of them, because the Debugging pane draws them together** and the console's
/// state is only legible beside debugging's — the console needs *both* switches, so a pane that
/// reported them separately would let somebody turn one on, see nothing happen, and have nowhere to
/// find out why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SwitchState {
    /// Whether debugging is on **in this run**.
    pub debug_running: bool,
    /// Whether the settings file says debugging is on, and so whether it survives a restart.
    ///
    /// **The stored value is what a switch draws from, not the running one.** A switch drawn from
    /// the running value read *Turn debugging on* both before the press and after it, because the
    /// running value is a snapshot taken when the router was built and debugging decides which
    /// routes get mounted. That bug is why this pair is two fields.
    pub debug_stored: bool,
    /// Whether the settings file asks for the development console at `/dev/`.
    pub dev_remote_stored: bool,
    /// Whether it is actually being served, which needs debugging on as well.
    pub dev_remote_served: bool,
    /// Whether the frame-statistics panel is on the machine's screen.
    ///
    /// The one switch here that takes effect at once and is never written down: it mounts nothing,
    /// so the next frame draws it.
    pub performance_overlay: bool,
    /// Whether demo mode is on for this run.
    pub demo_enabled: bool,
    /// Whether the settings file says so, and so whether it survives a restart.
    pub demo_stored: bool,
    /// Seconds of silence before a demo song starts. `0` is "as soon as the machine is idle".
    pub demo_delay_secs: u32,
}

/// The facts the information panel draws, which are the machine saying what it is.
///
/// **Built entirely from reads a caller needs no password for**, which is what lets `km-admin` draw
/// this panel before anybody has logged in — and is why the panel sits above that surface's tab strip
/// rather than in it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Identity {
    /// Every address this machine can be reached on, in its own order of preference.
    pub urls: Vec<String>,
    /// How many songs are installed, or `None` when the machine could not count them.
    ///
    /// **`None` is a real answer and not a zero.** A catalog that would not open is a machine with
    /// something wrong rather than a machine with no songs, and the panel says
    /// *could not be counted* — which `songs-uncounted` exists for.
    pub songs: Option<usize>,
    /// Which build is answering.
    pub version: String,
    /// What language the television draws in.
    ///
    /// **The parsed locale rather than the tag it arrived as**, so the picker cannot be handed a
    /// string no `<option>` matches. A remote host reads a tag off the wire and resolves it with
    /// `km_locale::Locale::parse`; a tag it does not know is a machine speaking a language this build
    /// has no catalog for, which is `Locale::default()`'s case rather than an error worth a page.
    pub locale: km_locale::Locale,
    /// Whether this host can be switched off and restarted from a page.
    ///
    /// A capability of the *host* rather than a method on the machine: a desktop build has no
    /// business shutting the computer down, and the appliance does.
    pub power: bool,
}

/// What the machine is, and the two names an owner gives it.
///
/// Everything on the *This machine* tab that is not a switch: the information panel, the name, the
/// screen language, the password and the sessions it grants.
#[async_trait::async_trait]
pub trait Machine: Send + Sync + 'static {
    /// What this machine calls itself, for the heading every page carries.
    ///
    /// **Separate from [`Self::identity`] and cheap on purpose.** The heading is on every page and
    /// the panel is on one, so folding the two together would make every page load ask a machine for
    /// its address list — over a network, in one of the two hosts.
    async fn name(&self) -> Result<String, AdminError>;

    /// The information panel's facts.
    async fn identity(&self) -> Result<Identity, AdminError>;

    /// Rename it, so the name survives a restart.
    ///
    /// **What a name may be is the machine's rule and not this page's.** A refusal comes back as
    /// [`AdminError::Refused`] carrying the machine's own sentence, because a second opinion composed
    /// here would be a second thing to keep in agreement with it.
    async fn set_name(&self, name: &str) -> Result<(), AdminError>;

    /// Set what language the television draws in.
    ///
    /// **Not what language this page is in**, which is the reader's own choice and rides in a cookie.
    /// The two are genuinely different questions: the person setting a machine up may not be the
    /// person who will sing at it.
    async fn set_locale(&self, tag: &str) -> Result<(), AdminError>;

    /// Change the admin password, or with `None` put the machine back on a fresh PIN.
    ///
    /// **`None` is the reset and only one host offers it.** A machine put back on a generated PIN
    /// shows that PIN on its own television, so the only place the act makes sense is beside the
    /// screen that will then display it.
    ///
    /// **A plaintext password, and the hashing is the implementation's.** Three things happen behind
    /// this in the machine's own process — argon2, a write to `settings.json`, and moving the running
    /// hash that the token check is keyed on — and a host over HTTP does one `POST` and none of them.
    /// The page knows only that it has a password somebody typed.
    ///
    /// **What a password may be is checked before this is called**, against
    /// `km_api::MIN_PASSWORD_CHARS`, because both surfaces and the JSON route set the same password on
    /// the same machine: a floor written down twice is two rules free to disagree, and they did — one
    /// counted bytes where the API counts characters, so a two-character CJK password was stored by
    /// one and refused by the other.
    async fn set_password(&self, password: Option<&str>) -> Result<(), AdminError>;

    /// End every outstanding session, including the caller's own.
    async fn reset_sessions(&self) -> Result<(), AdminError>;

    /// What the room may do with no code, and whether each code is set.
    ///
    /// **Defaulted to a refusal**, so a host that cannot answer draws no Access card rather than a
    /// wrong one. The machine's own page answers from its state.
    async fn access(&self) -> Result<km_api::dto::AccessDto, AdminError> {
        Err(AdminError::NotFound)
    }

    /// Set what the room may do with no code.
    async fn set_room_access(&self, _room: km_api::Access) -> Result<(), AdminError> {
        Err(AdminError::NotFound)
    }

    /// Set the code for the queue or the control level, or with `None` clear it.
    async fn set_access_code(
        &self,
        _level: km_api::Access,
        _code: Option<&str>,
    ) -> Result<(), AdminError> {
        Err(AdminError::NotFound)
    }

    /// Switch the machine off.
    ///
    /// **Refused rather than absent where the host has no power controls**, which is the one place
    /// this seam prefers a refusal to a capability: [`Identity::power`] is what decides whether the
    /// control is *drawn*, and this is the belt to that braces — a `POST` arriving anyway, from a
    /// stale page or a hand-made request, gets a sentence rather than a panic.
    async fn shut_down(&self) -> Result<(), AdminError>;

    /// Restart it. **Mends itself in ten seconds**, which is why it asks nothing first where
    /// [`Self::shut_down`] does.
    async fn restart(&self) -> Result<(), AdminError>;
}

/// One installed package, as the Songs tab shows it.
///
/// **The package and the reason it cannot be removed travel together, because they are one
/// consistent view.** Both are asked in the same breath, so a listing cannot show a button the
/// machine would then refuse — which is `A control that can only be refused is left out, not grayed`
/// applied to a list rather than to a switch.
///
/// # A named handful of fields, and not `km_catalog::InstalledPackage`
///
/// This is the one place the seam does *not* pass a machine type through, and the reason is the
/// workspace boundary rather than taste. `InstalledPackage` is `km-catalog`'s, which means
/// **rusqlite** — and the second host is a desktop program with no catalog of its own, reaching into
/// the main workspace by path under a rule that keeps what it takes small. `AudioOutputs` and
/// `SoundFontBanks` cross unchanged because they cost nothing to name; a SQLite dependency for a
/// handful of fields is a different bargain.
///
/// So these are the ones the page actually reads. One more is a compile error at both hosts, which
/// is the right way round to find out it is wanted.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// What the routes name it by.
    pub id: String,
    /// What it calls itself, which may be empty — the page then shows the id.
    pub name: String,
    /// Which build of it is installed.
    ///
    /// **The file on disk cannot say**, because the machine names it `<name>-<id>.kmpkg` so that a
    /// rebuild lands on its predecessor — see `What an installed package file is called`. So the
    /// row is where the owner who has just sent a volume finds out which build arrived.
    pub version: String,
    /// Which thousand its songs are numbered in.
    pub bank: u16,
    /// How many songs it holds.
    pub song_count: usize,
    /// The machine's own sentence, or `None` when the package is the machine's to delete.
    pub why_not_removable: Option<String>,
    /// How large the `.kmpkg` is, and **only [`Songs::package`] fills this in**.
    ///
    /// **Not carried on every row, deliberately.** Measuring a file is cheap once and a thousand
    /// times is a thousand stats; the one moment the difference between 34 KiB and twenty gigabytes
    /// matters is the page asking whether to delete it, and that page is one somebody just asked
    /// for. So the listing leaves it `None` and the confirmation reads it.
    pub bytes: Option<u64>,
    /// The package's flags by name, such as `uncurated`, each drawn as a badge.
    ///
    /// **Names rather than the word**, because a name is what the page prints and a bit it has no
    /// label for is nothing it can say. The machine names them through `PackageFlags::names`, and
    /// `PackageDto::flag_names` carries the same list to the desktop host.
    pub flag_names: Vec<String>,
}

/// The Songs tab: which packages are installed and where their numbers sit.
#[async_trait::async_trait]
pub trait Songs: Send + Sync + 'static {
    /// Every installed package, each with the reason it cannot be removed if there is one.
    ///
    /// **The implementation decides how to get off the runtime**, and in the machine's own process it
    /// must: installing a package holds the catalog mutex for *seconds*, so a page load that blocked
    /// a worker behind one would stall every other request too. That is the whole reason these
    /// methods are `async` when one host answers from memory.
    async fn packages(&self) -> Result<Vec<Listing>, AdminError>;

    /// One package by id, for the question a removal asks first.
    ///
    /// **Its own method rather than a filter over [`Self::packages`]**, because a host over HTTP can
    /// ask for one and should not fetch a thousand to find it.
    async fn package(&self, id: &str) -> Result<Listing, AdminError>;

    /// Uninstall it, which deletes the `.kmpkg`.
    ///
    /// **The page asks before this is called.** It may be tens of gigabytes and may be the only
    /// copy, and on Android no file manager can reach the folder — which is what makes the
    /// confirmation owed rather than a courtesy.
    ///
    /// Answers with how many songs went, which the notice quotes.
    async fn remove(&self, id: &str) -> Result<usize, AdminError>;

    /// Move a package's thousand songs to another block of numbers.
    ///
    /// Answers with how many moved, which the notice quotes beside the new first number. **What a
    /// block may be is checked before this is called**, against `km_songcode::MAX_BANK`, for the
    /// reason the password floor is: the same limit written down twice is two rules free to
    /// disagree.
    async fn set_bank(&self, id: &str, bank: u16) -> Result<usize, AdminError>;
}

/// The Pictures tab: what is on the screen behind the words.
#[async_trait::async_trait]
pub trait Pictures: Send + Sync + 'static {
    /// What the rotation holds, which one is showing, and whose pictures they are.
    async fn rotation(&self) -> Result<km_api::machine::WallpaperState, AdminError>;

    /// The files in the folder, one row each.
    ///
    /// **Files, not pictures**, so an archive is one row saying how many it puts in the rotation —
    /// the same unit a package is. Separate from [`Self::rotation`] because the confirmation page
    /// needs only this, and a machine showing the bundled set has a rotation and no rows.
    async fn files(&self) -> Result<Vec<km_api::machine::Picture>, AdminError>;

    /// Show the next one now.
    async fn next(&self) -> Result<(), AdminError>;

    /// Delete a file from the folder.
    async fn delete(&self, id: &str) -> Result<(), AdminError>;
}

/// One package file the machine found and could not use.
///
/// **The folder is here and is not in `PackageProblemDto`**, which drops it deliberately: *"the
/// directory layout of the machine under the television is nobody's business but the owner's."* This
/// page runs inside the machine and already prints paths in a refusal's sentence, so it may show it
/// — and it has to, because two `carols.kmpkg` rows with the same reason and two different Delete
/// links was an observed bug. The rule is `the page prints the sentence, a remote gets the flag`.
#[derive(Debug, Clone, Default)]
pub struct Refused {
    /// What the delete route names it by.
    pub id: String,
    /// The file's own name.
    pub file: String,
    /// Which folder it was found in.
    pub folder: String,
    /// The machine's own reason.
    pub reason: String,
    /// Why the machine will not delete it, or `None` when it will.
    pub why_not_removable: Option<String>,
    /// How large it is.
    pub bytes: Option<u64>,
}

/// The Problems tab's own half: the packages the machine found and could not use.
///
/// **Only the refused list.** The rest of that tab — the bank complaint, the output fallback, the
/// empty rotation — is composed from [`Sound`] and [`Pictures`], because each of those already knows
/// it and a second way to ask would be the drift this seam exists to stop. See `handlers::faults`.
///
/// **`Option` on the state**, because it is the one part of this page a host over HTTP has no way to
/// answer: the list identifies its rows by a *path on the machine*, which the API deliberately does
/// not publish.
#[async_trait::async_trait]
pub trait Problems: Send + Sync + 'static {
    /// Every package file the machine found and could not install.
    async fn refused(&self) -> Result<Vec<Refused>, AdminError>;

    /// Delete one of those files.
    ///
    /// `id` is resolved against a **freshly read** list rather than trusted from the page, because
    /// the page a person is looking at may be minutes old and the thing it named may have gone.
    async fn delete(&self, id: &str) -> Result<(), AdminError>;
}

/// Taking a file a browser sent, whichever kind it is.
///
/// **One trait with a `kind` rather than a method per tab**, because `km_api::uploads` already treats
/// the three as one operation — one field name, one size table, one extension list, one streaming
/// path. Three methods here would be three chances to disagree with it.
#[async_trait::async_trait]
pub trait Uploads: Send + Sync + 'static {
    /// Stream one uploaded file to the machine and say what happened, in a sentence.
    ///
    /// **Takes an already-extracted `Multipart`** so a page's handler keeps axum's own rejection for
    /// its own error page rather than this crate's, which is the shape `km_api::uploads::receive`
    /// chose for the same reason.
    ///
    /// The sentence is the machine's — *"Added 16 songs."* — and is shown as it arrives. What each
    /// kind may be and how large it may be are the machine's rules too, read from
    /// `km_api::uploads::{accept_for, limit_in_words}` before a file is ever chosen.
    async fn receive(
        &self,
        kind: km_api::machine::Upload,
        form: axum::extract::Multipart,
    ) -> Result<String, AdminError>;
}

/// The Debugging pane's three switches, and demo mode.
///
/// Grouped away from [`Machine`] because they are settings a machine *runs under* rather than facts
/// about what it is — and because all four share one read.
#[async_trait::async_trait]
pub trait Switches: Send + Sync + 'static {
    /// Every switch at once. See [`SwitchState`] for why it is one call.
    async fn read(&self) -> Result<SwitchState, AdminError>;

    /// Turn debugging mode on or off. Takes effect at the machine's next start.
    async fn set_debug(&self, on: bool) -> Result<(), AdminError>;

    /// Ask for the development console at `/dev/`, or stop asking. Needs debugging on as well.
    async fn set_dev_remote(&self, on: bool) -> Result<(), AdminError>;

    /// Put the frame-statistics panel on the machine's screen, or take it off. Immediate.
    async fn set_performance(&self, on: bool) -> Result<(), AdminError>;

    /// Turn demo mode on or off, for this run or for good.
    async fn set_demo(&self, enabled: bool, persist: bool) -> Result<(), AdminError>;

    /// How long the machine waits before starting a demo song, answering with what it stored.
    ///
    /// **The machine's own number back, not the one that was asked for**, because the route caps it
    /// — a page that echoed the request would tell somebody their 9999 had been saved.
    async fn set_demo_delay(&self, secs: u32) -> Result<u32, AdminError>;
}

/// The Sound tab: where the sound comes out, and which bank supplies the instruments.
///
/// # The shapes are `km_api`'s own, and that is on purpose
///
/// `AudioOutputs` and `SoundFontBanks` cross this seam unchanged rather than being copied into
/// parallel row types. **Both hosts can already name them** — the machine because they are its own,
/// and `km-admin` because it takes `km-api` for exactly this reason, stated in its manifest: *"The
/// machine's DTOs and its mDNS discovery, so the shapes this talks to are stated once. It carries no
/// HTTP client of its own, which is what makes it safe to take from here."*
///
/// A third set of row types here would be one more thing to keep in step with the machine's, on a
/// page whose whole job is to show what the machine says. [`crate::views`]' own row types stay, and
/// those earn their place: they hold rendered sizes, composed sentences and resolved labels.
#[async_trait::async_trait]
pub trait Sound: Send + Sync + 'static {
    /// Every output device that could be chosen, and which is in use.
    ///
    /// `all` widens the list from one row per physical output to every way of naming them — ALSA
    /// hands over its *configuration* rather than its hardware, so one jack arrives ten times under
    /// one string and the full list runs past thirty rows.
    async fn outputs(&self, all: bool) -> Result<km_api::machine::AudioOutputs, AdminError>;

    /// Choose the output device, and remember the choice.
    ///
    /// Refused with [`AdminError::Busy`] while anything is loaded or queued: the player lives inside
    /// the stream a change has to drop. **The control is drawn anyway** — `A control that can only be
    /// refused is left out, not grayed` names `changeable` as the precedent for *having* the flag
    /// rather than for spending it, because a device that cannot be changed now can be changed when
    /// the song ends.
    ///
    /// **Answers with the device list, which the confirmation needs.** It names the device that was
    /// *chosen*, and the only place its name can be looked up is the list the machine answered with
    /// — not `active_name`, which is what is *sounding*. `Holding the audio device` means that on an
    /// idle machine nothing is: the endpoint is handed back five seconds after the last song, so the
    /// engine reports the placeholder `not yet opened`, and the confirmation once read *"The sound
    /// comes out of not yet opened now."* Found by pressing the button on an idle machine, which is
    /// every machine somebody is configuring.
    async fn set_output(&self, id: &str) -> Result<km_api::machine::AudioOutputs, AdminError>;

    /// Move the level the active output runs at, in hundredths of a decibel.
    ///
    /// **Not refused while a song plays**, unlike the device beside it: the level belongs to the
    /// sound card rather than to the stream, so moving it disturbs nothing and there is no
    /// `changeable` to consult.
    ///
    /// Refused where the active output has no level of its own, which is an HDMI or S/PDIF path
    /// handing the volume to a receiver. **The page leaves the control out in that case rather than
    /// drawing it dead**, so this refusal is the answer to a request nothing on the page offered —
    /// a second tab, or a device that changed underneath.
    ///
    /// Answers with the device list, for [`Sound::set_output`]'s reason: the confirmation names
    /// where the level landed, which a control with coarse steps will have rounded.
    async fn set_level(&self, db_centi: i32) -> Result<km_api::machine::AudioOutputs, AdminError>;

    /// Every bank that could be chosen, and which one the setting names.
    async fn banks(&self) -> Result<km_api::machine::SoundFontBanks, AdminError>;

    /// What is actually loaded, and what is wrong with it if anything.
    ///
    /// Separate from [`Self::banks`] because the two answer different questions: that one is *what
    /// could play*, this is *what is playing and whether it is whole*. The Problems tab reads only
    /// this one.
    async fn loaded(&self) -> Result<km_api::machine::SoundFontStatus, AdminError>;

    /// Choose a bank and keep the choice, putting it in force without stopping the song.
    async fn use_bank(&self, id: &str) -> Result<(), AdminError>;

    /// Delete a bank's file.
    ///
    /// **On Android this is 301 MiB no file manager can reach**, which is the case that makes this
    /// page's confirmation owed rather than a courtesy.
    async fn delete_bank(&self, id: &str) -> Result<(), AdminError>;
}

/// A refusal, in the reader's language where it has a key and in the machine's words where it does not.
///
/// **The one function that decides which**, so no handler has to remember the rule. `Refused` carries
/// the machine's own sentence and is shown as it arrived; everything else is a code this crate has a
/// catalog entry for.
#[must_use]
pub fn say(error: &AdminError, locale: km_locale::Locale) -> Cow<'static, str> {
    match error.key() {
        Some(key) => crate::words::messages(locale).msg(key),
        None => Cow::Owned(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key `key()` can produce is in [`ERROR_KEYS`], and every one of those is reachable.
    ///
    /// **Both directions, because each catches a different mistake.** A variant whose key is missing
    /// from the list is a message `no_message_is_left_unused` will call unused and a translator will
    /// delete; a key in the list that nothing produces is the leftover that test exists to find,
    /// hiding behind the one exemption this crate grants.
    #[test]
    fn every_error_key_is_listed() {
        let every = [
            AdminError::Offline("timeout"),
            AdminError::Unauthorized,
            AdminError::NotFound,
            AdminError::Refused("the machine said so".to_owned()),
            AdminError::Failed("a lock was poisoned".to_owned()),
        ];

        let mut produced = Vec::new();
        for error in &every {
            if let Some(key) = error.key() {
                assert!(
                    ERROR_KEYS.contains(&key),
                    "`{key}` is produced and not listed, so the catalog test will call it unused"
                );
                produced.push(key);
            }
        }
        for key in ERROR_KEYS {
            assert!(
                produced.contains(key),
                "`{key}` is listed and nothing produces it"
            );
        }

        // The two that carry words already. `Refused` is the machine's own sentence, which is the
        // whole point of the variant; `Failed` is a developer's, shown rather than swallowed.
        assert!(AdminError::Refused(String::new()).key().is_none());
        assert!(AdminError::Failed(String::new()).key().is_none());
    }

    /// A refusal in the machine's words arrives unchanged, whatever language the page is in.
    #[test]
    fn the_machines_own_sentence_is_not_translated() {
        let refusal = AdminError::Refused("that name is already taken".to_owned());
        for locale in km_locale::Locale::ALL {
            assert_eq!(say(&refusal, *locale), "that name is already taken");
        }
    }

    /// ...and a coded one is, which is the other half of why codes exist.
    #[test]
    fn a_coded_refusal_is_rendered_in_the_readers_language() {
        let english = say(&AdminError::Unauthorized, km_locale::Locale::English);
        let portuguese = say(
            &AdminError::Unauthorized,
            km_locale::Locale::BrazilianPortuguese,
        );
        assert_ne!(
            english, portuguese,
            "a code exists so the sentence can differ; these did not"
        );
        for said in [&english, &portuguese] {
            assert!(
                !said.starts_with('\u{2e22}'),
                "a key the catalog does not have renders in brackets: {said}"
            );
        }
    }
}
