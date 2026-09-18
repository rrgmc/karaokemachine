//! The HTTP surface, in one `router()`.
//!
//! **One function holding every route**, so the whole surface of the program is one screenful. That
//! is `km-package-builder`'s rule and it earns its keep the same way here: the answer to "what can
//! this thing do?" is a page of code rather than a search.
//!
//! **The static files are compiled in.** Five explicit routes rather than a `ServeDir`, because this
//! program is a single executable somebody copies onto a desktop and a `static/` folder that had to
//! arrive beside it is a way to be half-installed. See `static/README.md`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

use crate::{handlers, views};

/// htmx, vendored. See `static/README.md` for why nothing is fetched from a CDN.
const HTMX_JS: &str = include_str!("../static/htmx.min.js");
/// Its license, served because a vendored dependency's terms travel with it.
const HTMX_LICENSE: &str = include_str!("../static/htmx-LICENSE.txt");
/// This program's own script, which is mostly about htmx not swapping a non-2xx response.
const UI_JS: &str = include_str!("../static/ui.js");

/// This program's mark, in the browser tab.
///
/// **Not the machine's, which is the mistake this is easiest to make**: four programs in this
/// product can be open at once and the favicon is what tells two tabs apart. A test asserts these
/// bytes differ from the machine's.
///
/// The magenta is the fourth lead, and it is the theme's `icon_glow` lifted 12% toward white —
/// straight it measured 4.00:1 for the M against the plate, under the 4.5:1 letters floor. See
/// `admin_lead` in `crates/playback/km-display/examples/icon.rs`.
const ICON_PNG: &[u8] = include_bytes!("../../../../../icon/km-admin-32.png");

/// The machine's mark, for a page that is showing *which machine*.
const MACHINE_ICON_PNG: &[u8] = include_bytes!("../../../../../icon/icon-32.png");

/// What every page of this program is called.
///
/// **The page names the product; the command line names the command.** `km-admin` is what somebody
/// typed and what they would grep for; this is what the page, the window title, the macOS app menu
/// and the tray tooltip all say. One constant, because the same string being written out three times
/// in Rust is the drift a constant exists to stop.
///
/// **No short twin here.** The abbreviated `KM Admin` is what an *icon* says — see
/// `What the tool calls itself` in docs/decisions/ — and this program writes no Linux `.desktop`
/// entry, so every place its icon is named is a file that cannot read a Rust constant:
/// `tools/platform/macos/Info.admin.plist`, `bundle_name()` in `tools/dist/cmd.sh`, and the Windows
/// installer. `km-package-builder` carries an `APP_NAME_SHORT` because it does write one.
pub const APP_NAME: &str = "KaraokeMachine Admin";

/// Which build this is, for the chip beside the machine tag.
///
/// **`env!` rather than a field on `Chrome`**, because this is a compile-time constant and a field
/// would be one more thing three constructors and their test fixtures have to keep saying. The
/// template reaches it as `crate::server::VERSION`, exactly as `km-package-builder`'s layout reaches
/// its `APP_NAME`.
///
/// It is the whole repository's version — `version.workspace = true` here and in the root manifest,
/// held equal by `tools/dev/check-version-pin.sh` — so this program and the machine it is pointed at
/// report the same number when they are the same build, which is the comparison somebody makes when
/// a page says something they did not expect.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Puts a job in a section's one slot, unless something is still using it.
///
/// **One at a time per section**, the rule `Downloader::start` keeps: a second is not a feature
/// anybody asked for, and queueing is the part that would need thinking about. A *finished* job is
/// replaced without complaint — it is only being kept so the page can say how it went.
///
/// `busy` is a catalog key rather than a sentence, because no locale is in reach here and the page
/// that draws it has one. It is the section's own words rather than a generic line: a search and a
/// fetch are what those two sections are actually doing.
fn claim(
    slot: &std::sync::Mutex<Option<Arc<crate::job::Job>>>,
    job: Arc<crate::job::Job>,
    busy: &'static str,
) -> Result<Arc<crate::job::Job>, &'static str> {
    let mut slot = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(running) = slot.as_ref()
        && running.view().running
    {
        return Err(busy);
    }
    *slot = Some(Arc::clone(&job));
    Ok(job)
}

/// Everything the handlers share.
#[derive(Clone)]
pub struct State {
    inner: Arc<Inner>,
}

struct Inner {
    /// Where downloads, packs and settings go.
    data_dir: PathBuf,
    /// The machine being talked to, and the token held for it.
    ///
    /// **One client, kept, rather than one built per request** — and that is not an optimization.
    /// The bearer token lives inside the client, so a fresh one per handler would throw the token
    /// away between the login that obtained it and the upload that needs it, and every upload would
    /// meet a 401 no matter how many times somebody typed the password.
    machine: std::sync::Mutex<Option<crate::machine::Client>>,
    /// The Sound section's running job, or the last one to finish.
    ///
    /// **Kept after it finishes**, because that is when it has something to say: which file landed,
    /// or why nothing did. It is replaced when the next one starts.
    sound_job: std::sync::Mutex<Option<Arc<crate::job::Job>>>,
    /// The Pictures section's, on the same terms.
    pictures_job: std::sync::Mutex<Option<Arc<crate::job::Job>>>,
    /// What to search for.
    pictures_settings: std::sync::Mutex<crate::pictures::Settings>,
    /// The provider keys this run is using.
    ///
    /// **In memory**, whether or not they were also written down — see [`crate::keys`]. The file is
    /// read once at startup and written only when somebody ticks the box.
    keys: std::sync::Mutex<km_wallpaper_pack::providers::Keys>,
    /// The machine passwords this computer was told to remember, by machine id.
    ///
    /// **In memory unless the box was ticked**, which is [`crate::keys`]' arrangement one file over
    /// and the same policy: a credential somebody typed into a page is remembered only if they said
    /// so. What is held here is the *password*, never the token — a token expires and the machine
    /// forgets its own on restart, where a standing instruction to log in does neither.
    passwords: std::sync::Mutex<crate::passwords::Remembered>,
    /// The machines advertising themselves, kept current in the background.
    ///
    /// **`None` until the socket is bound**, and that is the guard rather than a `cfg(test)`: this
    /// type is built by a great many tests and `Watcher::start` opens a multicast socket, which is
    /// what `CONTRIBUTING.md`'s *No test binds a non-loopback address* forbids. A test that never
    /// serves therefore never listens, and the Look button answers an empty list — which is the same
    /// answer it gives on a network with no mDNS.
    watcher: std::sync::Mutex<Option<km_api::discover::watch::Watcher>>,
}

impl State {
    /// A fresh state over a data directory.
    ///
    /// **What was asked for, then what was chosen last time, then nothing** — the first two rungs of
    /// `km_remote_core::find::locate`'s ladder. The third is deliberately missing: that one browses
    /// the network and adopts what it finds, and this program may not, because it writes files onto
    /// whatever it is pointed at. The page offers a browse instead, and a person presses a button.
    /// See [`crate::handlers::discover_machines`].
    ///
    /// **`--machine` is not written down here**, and that is the reason this builds the client
    /// directly rather than going through [`set_machine`](Self::set_machine), which persists.
    /// Remembering it would mean a one-off `--machine` for a test quietly replaced the address
    /// somebody normally uses.
    ///
    /// An address that cannot produce a client is treated as no machine: the page then asks for one,
    /// which is the same thing it does when none was given.
    pub fn new(data_dir: PathBuf, machine: Option<String>) -> Self {
        // Normalized on the way out of both rungs, not only the first: what is remembered was
        // normalized before it was written, but the file is one line of text somebody can edit, and
        // a bare `192.168.1.5` in it should mean what it means everywhere else. `normalize` leaves a
        // full URL alone, so this costs nothing on the ordinary path.
        let client = machine
            .or_else(|| crate::chosen::load(&data_dir).map(|known| known.url))
            .map(|address| crate::machine::normalize(&address))
            .and_then(|address| crate::machine::Client::new(address).ok());
        // Read once, here: the environment's keys, then whatever was remembered on this machine.
        let keys = crate::keys::load(&data_dir);
        let settings = crate::pictures::load(&data_dir);
        // The half a `Drop` cannot cover: a process killed mid-upload leaves a staged file behind,
        // and here is the one moment nothing is passing through. See `crate::staging`.
        crate::staging::sweep(&data_dir);
        // Read before the directory moves into the struct, which is `keys` one line over: both come
        // off the same folder and neither is worth a second borrow to reorder.
        let passwords = crate::passwords::load(&data_dir);
        Self {
            inner: Arc::new(Inner {
                data_dir,
                machine: std::sync::Mutex::new(client),
                sound_job: std::sync::Mutex::new(None),
                pictures_job: std::sync::Mutex::new(None),
                pictures_settings: std::sync::Mutex::new(settings),
                keys: std::sync::Mutex::new(keys),
                passwords: std::sync::Mutex::new(passwords),
                watcher: std::sync::Mutex::new(None),
            }),
        }
    }

    /// Starts listening for machines. Called once, after the socket is bound.
    pub fn start_watching(&self) {
        self.inner
            .watcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert_with(km_api::discover::watch::Watcher::start);
    }

    /// The machines advertising themselves right now. Empty before [`Self::start_watching`].
    pub fn machines_seen(&self) -> Vec<km_api::discover::Sighting> {
        self.inner
            .watcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(km_api::discover::watch::Watcher::snapshot)
            .unwrap_or_default()
    }

    /// What the network is saying, with presence, as [`km_api::discover::known::choose`] wants it.
    fn machines_observed(&self) -> Vec<km_api::discover::watch::Observed> {
        self.inner
            .watcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(km_api::discover::watch::Watcher::observed)
            .unwrap_or_default()
    }

    /// Ask the network again now, because somebody just pressed the button.
    pub fn look_again(&self) {
        if let Some(watcher) = self
            .inner
            .watcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            watcher.poke();
        }
    }

    /// Records what the machine at the current address said about itself.
    ///
    /// **Not a choice, so it does not break this program's rule.** `chosen`'s header says what is
    /// remembered is what somebody *chose*; the address here is already that, and this only adds
    /// which machine is at it — which is what makes a later move recognizable as the same machine
    /// rather than as a new choice nobody made.
    pub fn machine_answered(&self, id: &str, name: Option<String>) {
        let Some(url) = self.machine() else {
            return;
        };
        crate::chosen::answered(&self.inner.data_dir, &url, id, name);
    }

    /// Follows the chosen machine if it is announcing itself from somewhere else now.
    ///
    /// **Following an id is not choosing a machine**, which is what
    /// `Discovering a machine in the package builder` forbids of every tool that writes to one. The
    /// address moved; the machine did not. Refusing to follow would leave this program sending a
    /// gigabyte to nothing while the box somebody chose sat two meters away announcing itself — and
    /// an identity that cannot change is what makes *this* guarantee stronger than the address rule
    /// it replaces, rather than a softening of it.
    ///
    /// **An address with no id recorded never moves.** That is the guard `--machine` and the address
    /// field need: neither has said what is there yet, and an address somebody named must not
    /// inherit the previous machine's identity and be dragged off after it.
    ///
    /// **The policy is `known::choose` rather than a third copy of it.** There are three follows in
    /// the product — this, `choose` itself and `km-package-builder`'s `moved_to` — and expressing
    /// *may not adopt* by declining to call the policy is how the three drift apart. `adopts: false`
    /// is that rule inside the function, which also brings rule 2 with it: *stay put where the
    /// address in hand is answering and reports the same id*.
    pub fn follow_machine(&self) -> Option<String> {
        let known = crate::chosen::load(&self.inner.data_dir)?;
        let current = self.machine()?;
        let seen = self.machines_observed();
        let km_api::discover::known::Choice::Use { url: moved, why } =
            km_api::discover::known::choose(&km_api::discover::known::Situation {
                pinned: false,
                // This program holds no connection either — every request builds a client — so it
                // has no fact about whether the address is answering, and says so.
                online: false,
                current: Some(&current),
                known: Some(&known),
                seen: &seen,
                answering_id: None,
                stale: km_api::discover::known::is_stale(&known, std::time::SystemTime::now()),
                // This program uploads packages, banks and photographs. See the note above.
                adopts: false,
            })
        else {
            return None;
        };
        crate::chosen::moved(&self.inner.data_dir, &moved);
        let client = crate::machine::Client::new(moved.clone()).ok();
        *self
            .inner
            .machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = client;
        tracing::info!(from = %current, to = %moved, why = why.label(), "following the machine");
        Some(moved)
    }

    /// The provider keys this run is using.
    pub fn keys(&self) -> km_wallpaper_pack::providers::Keys {
        self.inner
            .keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replaces the keys this run is using. Writing them down is [`crate::keys::remember`]'s job.
    pub fn set_keys(&self, keys: km_wallpaper_pack::providers::Keys) {
        *self
            .inner
            .keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = keys;
    }

    /// What the Pictures section is set to search for.
    pub fn pictures_settings(&self) -> crate::pictures::Settings {
        self.inner
            .pictures_settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replaces the search settings, and writes them down.
    pub fn set_pictures_settings(&self, settings: crate::pictures::Settings) {
        crate::pictures::save(&self.inner.data_dir, &settings);
        *self
            .inner
            .pictures_settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
    }

    /// The Pictures section's job, running or just finished.
    pub fn pictures_job(&self) -> Option<Arc<crate::job::Job>> {
        self.inner
            .pictures_job
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Starts a job in the Pictures section, unless one is already running.
    pub fn start_pictures_job(&self, phase: &str) -> Result<Arc<crate::job::Job>, &'static str> {
        claim(
            &self.inner.pictures_job,
            crate::job::Job::new(phase),
            "job-busy-search",
        )
    }

    /// The same slot, for sending a picture somebody already has.
    ///
    /// **Unstoppable**, unlike a search: an upload is one call to the machine with nowhere to notice
    /// the asking. See [`crate::job::Job::unstoppable`]. It shares the section's one slot rather than
    /// taking a second, so a send and a search cannot both be reporting into the same fragment.
    pub fn start_pictures_send(&self, phase: &str) -> Result<Arc<crate::job::Job>, &'static str> {
        claim(
            &self.inner.pictures_job,
            crate::job::Job::unstoppable(phase),
            "job-busy-search",
        )
    }

    /// The Sound section's job, running or just finished.
    pub fn sound_job(&self) -> Option<Arc<crate::job::Job>> {
        self.inner
            .sound_job
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Starts a job in the Sound section, unless one is already running.
    ///
    /// **One at a time**, the rule `Downloader::start` keeps: a second is not a feature anybody
    /// asked for, and queueing is the part that would need thinking about. A *finished* job is
    /// replaced without complaint — it is only being kept so the page can say how it went.
    pub fn start_sound_job(&self, phase: &str) -> Result<Arc<crate::job::Job>, &'static str> {
        claim(
            &self.inner.sound_job,
            crate::job::Job::new(phase),
            "job-busy-fetch",
        )
    }

    /// The same slot, for sending a bank somebody already has. Unstoppable, for `start_pictures_send`'s
    /// reason.
    pub fn start_sound_send(&self, phase: &str) -> Result<Arc<crate::job::Job>, &'static str> {
        claim(
            &self.inner.sound_job,
            crate::job::Job::unstoppable(phase),
            "job-busy-fetch",
        )
    }

    /// Where downloads, packs and settings go.
    pub fn data_dir(&self) -> &std::path::Path {
        &self.inner.data_dir
    }

    /// The client for the machine currently being talked to, if any.
    pub fn client(&self) -> Option<crate::machine::Client> {
        self.inner
            .machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The address of the machine currently being talked to, if any.
    pub fn machine(&self) -> Option<String> {
        self.client().map(|client| client.base().to_owned())
    }

    /// The id of the machine in force, which is `None` until something has answered at its address.
    ///
    /// **The key a password is remembered under**, and the reason a machine that has never replied
    /// cannot be remembered at all: there is no identity yet to key one by. `chosen` records it the
    /// first time a machine answers, which is also what the follow uses, so a machine that moves to
    /// another address keeps its password.
    ///
    /// **The record has to be about the machine this run is pointed at, and a `--machine` run is
    /// the case that makes that worth checking.** That address is never written down, so the record
    /// on disk still names whichever machine was chosen last — and reading an id out of it would
    /// key one machine's password under another's, then offer it to a box it was never given for.
    /// Comparing the record's address with the one in force costs nothing and makes that impossible.
    pub fn machine_id(&self) -> Option<String> {
        let known = crate::chosen::load(&self.inner.data_dir)?;
        let pointed_at = self.machine()?;
        (crate::machine::normalize(&known.url) == pointed_at).then_some(known.id)?
    }

    /// What this computer remembers about the machine in force, or `None` if there is no id to key a
    /// password under.
    ///
    /// **A run started with `--machine` has none, and that is the same rule rather than a gap.**
    /// That address is deliberately not written down — see [`Self::new`] — so there is no record for
    /// an identity to be recorded against, and a one-off address therefore carries no persistent
    /// state of any kind. The box is absent rather than drawn and ignored.
    pub fn remembering(&self) -> Option<km_admin_pages::guard::Remembering> {
        let id = self.machine_id()?;
        Some(km_admin_pages::guard::Remembering {
            on: self
                .inner
                .passwords
                .lock()
                .is_ok_and(|held| held.holds(&id)),
            owner_only: crate::passwords::owner_only(),
        })
    }

    /// Whether a password given now could be written down at all.
    ///
    /// **The record on disk has to be about the machine in force**, because that record is where a
    /// login writes the id a password is keyed under. It is true on the ordinary path before the
    /// first login, the address having been saved when it was chosen; it is false for a `--machine`
    /// run pointed somewhere the record does not name, where a tick would be taken and discarded.
    #[must_use]
    pub fn can_remember(&self) -> bool {
        match (crate::chosen::load(&self.inner.data_dir), self.machine()) {
            (Some(known), Some(pointed_at)) => crate::machine::normalize(&known.url) == pointed_at,
            _ => false,
        }
    }

    /// The password remembered for the machine in force, if this computer was told to remember one.
    pub fn remembered_password(&self) -> Option<String> {
        let id = self.machine_id()?;
        self.inner
            .passwords
            .lock()
            .ok()
            .and_then(|held| held.get(&id).map(str::to_owned))
    }

    /// Remembers a password for the machine in force, or forgets it, writing the file either way.
    ///
    /// **A blank id is nothing to key under**, so a machine that has never answered is not
    /// remembered rather than being remembered under a name that may turn out to be another
    /// machine's.
    pub fn remember_password(&self, password: Option<&str>) {
        let Some(id) = self.machine_id() else {
            return;
        };
        let Ok(mut held) = self.inner.passwords.lock() else {
            return;
        };
        match password {
            Some(password) => held.remember(&id, password),
            None => held.forget(&id),
        }
    }

    /// Whether this program is holding a token the machine will accept.
    ///
    /// **A fact about this program's session and not about the browser asking**, which is the
    /// distinction `Capabilities::gate_every_route` turns on: nothing is refused by this, and the
    /// authority on whether a write may happen is the machine. The front door and the Machine tab
    /// read it to choose a sentence.
    #[must_use]
    pub fn logged_in(&self) -> bool {
        self.client().is_some_and(|client| client.has_token())
    }

    /// Logs in with a remembered password, if there is one and no token is held already.
    ///
    /// **The one lazy path.** A remembered password is a standing instruction to log in rather than a
    /// login that has happened: the token it buys expires and the machine forgets its own on
    /// restart, so the moment to spend it is the moment one is wanted rather than at startup, where
    /// it would talk to the network before anybody had asked for anything.
    ///
    /// **Failure is deliberately silent.** What follows it is the machine's own refusal and the pane
    /// saying how to log in, which is a better answer than a second error about a password somebody
    /// may not remember setting.
    pub async fn log_in_if_remembered(&self, client: &crate::machine::Client) {
        if client.has_token() {
            return;
        }
        let Some(password) = self.remembered_password() else {
            return;
        };
        if let Err(error) = client.log_in(&password).await {
            tracing::debug!("a remembered password was not accepted: {error}");
        }
    }

    /// Points this at a machine, or at none, and writes the choice down.
    ///
    /// **The token goes with the old machine**, because it was never valid for a different one. That
    /// falls out of replacing the client rather than being a step somebody has to remember.
    ///
    /// **It persists because everything that reaches here is a person choosing.** The only caller is
    /// [`crate::handlers::enter`] — the front door's form, whose rows are the address somebody typed
    /// and the machines a browse turned up — so there is no path by which this program points itself
    /// somewhere and then remembers having done so. `--machine` deliberately does not come through
    /// here, and that is enforced at the call site rather than here: the door does not set a machine
    /// it is already pointed at. See [`Self::new`] and `the_command_line_beats_what_was_remembered`.
    ///
    /// Clearing the address field is `None`, and it forgets rather than leaving yesterday's machine
    /// to come back at the next start.
    ///
    /// **Normalized here, which is what makes the address a person types mean what `--machine`
    /// means.** [`crate::machine::Client::new`] documents its argument as a normalized URL and
    /// cannot check it: a base with no scheme builds a request path rather than a URL, so every ask
    /// fails the moment it is made. The page then reports a machine that is not answering while the
    /// machine answers everything else — and a restart appears to cure it, because [`Self::new`]
    /// normalizes what it reads back.
    pub fn set_machine(&self, address: Option<String>) {
        let address = address.map(|address| crate::machine::normalize(&address));
        match &address {
            Some(url) => crate::chosen::save(&self.inner.data_dir, url),
            None => crate::chosen::forget(&self.inner.data_dir),
        }
        let client = address.and_then(|address| crate::machine::Client::new(address).ok());
        *self
            .inner
            .machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = client;
    }
}

/// What this program's *own* pages are rendered against.
///
/// # Two states, because there are two kinds of route
///
/// The searching, the jobs and the fragments need [`State`] — this program's data directory, its
/// provider keys, its running job. The pages *also* need the [`km_admin_pages::Admin`] the shared
/// router was built from, because that is what wraps a body in the shared chrome.
///
/// **Two routers, each finalized with its own state, merged.** The alternative was one state
/// carrying both and every job endpoint reaching past a field it has no use for; axum lets a nested
/// router keep its own, so the split costs one struct and says which routes are which.
#[derive(Clone)]
pub struct Pages {
    /// This program's own.
    pub state: State,
    /// The owner page, for [`km_admin_pages::Admin::shell`].
    pub admin: km_admin_pages::Admin,
}

/// A listening socket, before anything slow has happened.
pub struct Bound {
    listener: tokio::net::TcpListener,
    addr: SocketAddr,
}

impl Bound {
    /// The page to hand a browser.
    pub fn front_door(&self) -> String {
        front_door_url(self.addr)
    }
}

/// The address to hand a browser, given what the socket bound to.
///
/// `0.0.0.0` is not an address anything can connect *to*, so a `--lan` run is told to use loopback —
/// the page is on every interface either way.
///
/// **A free function rather than a method, so it can be tested without a socket.** It was a method,
/// and the test for it had to build a real listener and therefore a real reactor, which is a lot of
/// apparatus for formatting a string.
fn browsable_url(addr: SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{addr}")
    }
}

/// The page to open, given what the socket bound to.
///
/// **A shell is handed the door and never the origin.** `/` is a redirect, and a browser that has
/// kept one follows it without asking again — so a program whose landing page can move opens that
/// page itself. See the `/` route on [`router`].
///
/// **A free function for the reason [`browsable_url`] is one**: asserted without a socket, and so
/// without a reactor.
fn front_door_url(addr: SocketAddr) -> String {
    format!("{}{}", browsable_url(addr), views::CONNECT_PAGE)
}

/// Takes the socket.
///
/// **Before anything slow**, which is the ordering the package builder learned: bind first and the
/// browser waits in the accept backlog while a long start finishes, rather than meeting a refused
/// connection and a page that says nothing.
pub async fn bind(addr: SocketAddr) -> Result<Bound> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot listen on {addr}. Is something already using it?"))?;
    let addr = listener.local_addr().context("reading the bound address")?;
    Ok(Bound { listener, addr })
}

/// Serves until the process ends.
pub async fn serve(bound: Bound, state: State) -> Result<()> {
    // **After the bind, and never in `State::new`.** Opening a multicast socket is what
    // `CONTRIBUTING.md`'s *No test binds a non-loopback address* forbids, and a great many tests
    // build a `State`. Starting it here means the Look button answers instantly from a registry that
    // has been listening since this program opened, and that a test that never serves never listens.
    state.start_watching();
    axum::serve(bound.listener, router(state).into_make_service())
        .await
        .context("serving the page")
}

/// Serves a compiled-in text file.
///
/// The content type is not a formality: a stylesheet served as `text/plain` is ignored by every
/// browser, and `ServeDir` used to work this out from the extension.
fn embedded(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, content_type)], body)
}

/// The same, for a file that is not text.
fn embedded_bytes(content_type: &'static str, body: &'static [u8]) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, content_type)], body)
}

/// **The whole surface of this program**, in one screenful.
///
/// # The three upload routes each carry their own body limit, and nothing else does
///
/// axum's `DefaultBodyLimit` is **2 MB** unless a route says otherwise, and every other route here
/// takes a small form for which that is exactly the right guard. The three that take a file do not:
/// a package is gigabytes, the smallest bank this project offers is 32 MB, and a photograph is
/// routinely over 2 MB.
///
/// **The numbers come from `km_api::uploads::limit_for`**, never from a constant repeated here —
/// these routes exist to be a second front door onto the machine's own three, and two limits for one
/// operation is the drift that function exists to prevent. No slack is added for the multipart
/// envelope, deliberately: a file of exactly the limit is refused here *and* by the machine, and
/// slack would make this program accept what the machine is going to refuse.
///
/// **It failed in the least helpful way available on the machine's own page**, which is why that
/// paragraph exists rather than three layers with no comment: an 85 MB package uploaded at
/// 2,162,688 bytes and came back as a parser complaint naming neither a size nor a limit. The full
/// account is on `km_admin_pages::router`, which is where those three routes now live.
pub fn router(state: State) -> Router {
    Router::new()
        // **`/` is a redirect rather than a page.** Everything this program serves is under
        // `/admin`, because that is where the shared markup's links point; threading a base through
        // every template to avoid it is the drift one page set exists to remove.
        //
        // **Temporary, because a browser keeps a permanent one.** A cached landing redirect is
        // followed without the server being asked, so where the front door is would stop being this
        // program's to decide. `Bound::front_door` is the other half of that: a shell opens the page
        // and never the origin.
        .route(
            "/",
            get(|| async { axum::response::Redirect::temporary(views::CONNECT_PAGE) }),
        )
        // The static files, at the root and deliberately not under `/admin`: the shared crate serves
        // its own stylesheet and its own mark from there, and two `static/` under one prefix is two
        // things to tell apart for no gain. Five explicit routes, no `ServeDir` — see the module
        // header and `static/README.md`.
        .route(
            "/static/htmx.min.js",
            get(|| async { embedded("application/javascript; charset=utf-8", HTMX_JS) }),
        )
        .route(
            "/static/ui.js",
            get(|| async { embedded("application/javascript; charset=utf-8", UI_JS) }),
        )
        .route(
            "/static/htmx-LICENSE.txt",
            get(|| async { embedded("text/plain; charset=utf-8", HTMX_LICENSE) }),
        )
        .route(
            "/static/icon.png",
            get(|| async { embedded_bytes("image/png", ICON_PNG) }),
        )
        .route(
            "/static/machine.png",
            get(|| async { embedded_bytes("image/png", MACHINE_ICON_PNG) }),
        )
        .merge(owner_pages(state))
}

/// This program's own half — the searching, the jobs and the fragments.
///
/// # Everything here is under `/admin` beside the shared pages, and one path had to move
///
/// **`/sound/{id}/remove` was claimed by both.** The shared router's means *remove this bank from
/// the machine*; this program's meant *remove the copy on this computer*. Two different acts under
/// one path, and axum panics on the overlap rather than picking — which is the right way for that to
/// surface. This program's three row controls moved under `/sound/fetch/`, which also reads better:
/// they belong to the fetching page, and that is where they are drawn.
///
/// **The three upload routes are gone from here**, and that is the deletion this whole exercise was
/// for: `send_package`, `send_wallpaper` and `send_soundfont` posted to this program's own paths and
/// drew this program's own form, beside a machine page that had the same control. There is one now,
/// on the shared page, going through `Uploads::receive`.
fn own_pages(pages: Pages) -> Router {
    let state = pages.state.clone();
    Router::new()
        // This program's front door, which the machine's own page needs no counterpart for: a
        // machine has no say over which machine it is.
        .route("/connect", get(views::connect))
        // The searching, each on its own page under the tab it belongs to. Reached by a link from
        // the shared page — see `Capabilities::searching`.
        .route("/pictures/find", get(views::pictures))
        .route("/sound/fetch", get(views::sound))
        .with_state(pages)
        .merge(
            Router::new()
                // The door's two halves that need only this program's own state: the browse, which
                // arrives as a fragment because it takes three seconds, and the form's answer.
                .route("/connect/found", get(views::found))
                .route("/connect/use", axum::routing::post(handlers::enter))
                // What language this program's own pages are in, which is the door's third
                // question and the only one that is not about a machine.
                .route(
                    "/connect/locale",
                    axum::routing::post(handlers::set_page_locale),
                )
                // The pictures half: what to search for, the keys, the run, and the packs kept here.
                .route("/pictures/progress", get(views::pictures_progress))
                .route(
                    "/pictures/settings",
                    axum::routing::post(handlers::set_search),
                )
                .route(
                    "/pictures/keys/forget",
                    axum::routing::post(handlers::forget_keys),
                )
                .route("/pictures/run", axum::routing::post(handlers::run_pictures))
                .route(
                    "/pictures/stop",
                    axum::routing::post(handlers::stop_pictures),
                )
                .route("/pictures/thumb/{provider}/{id}", get(handlers::thumbnail))
                .route("/pictures/packs", get(views::packs))
                .route(
                    "/pictures/packs/{id}/send",
                    axum::routing::post(handlers::send_pack),
                )
                .route(
                    "/pictures/packs/{id}/remove",
                    axum::routing::post(handlers::remove_pack),
                )
                // The sound half: the sixty-three rows, and what to do with each.
                .route("/sound/progress", get(views::sound_progress))
                .route("/sound/list", get(views::banks))
                .route(
                    "/sound/fetch/{id}/get",
                    axum::routing::post(handlers::get_bank),
                )
                .route(
                    "/sound/fetch/{id}/send",
                    axum::routing::post(handlers::send_bank),
                )
                .route(
                    "/sound/fetch/{id}/remove",
                    axum::routing::post(handlers::remove_bank),
                )
                .route("/sound/stop", axum::routing::post(handlers::stop_sound))
                .with_state(state),
        )
}

/// The owner's page, as this program hosts it.
///
/// # Why `/admin` and not this program's root
///
/// The shared templates write their links out — `/admin/machine`, `/admin/sound?all=1`,
/// `/admin/songs/{id}/remove`. The alternative was a `base` field on the chrome, prefixed at every
/// link and every redirect, which is forty handlers and every template threading a value that is
/// empty in one host and `/admin` in the other. **Mounting at the same prefix is one line and costs
/// nothing**: this program is a year old, loopback-only, and has no bookmarks in the world to break.
///
/// `Capabilities::desktop` is what makes the page this program's rather than the machine's — the
/// installed-package table, the rotation and the bank list are absent, and the *Different machine*
/// pane and the two doors to the searching are present. `ICON_ADMIN_PNG` is the magenta mark, because
/// four programs here can be open at once and the favicon is what tells two tabs apart.
///
/// **No pinned language.** Both halves of every page have a catalog — this program's in
/// [`crate::words`] and the shared pages' in `km-admin-pages` — so `Admin::locale` reads the
/// request, and a viewer who chose Portuguese at `/` on the machine meets Portuguese here.
fn owner_pages(state: State) -> Router {
    let host = std::sync::Arc::new(crate::host::RemoteMachine::new(state.clone()));
    let state2 = state.clone();
    let admin = km_admin_pages::Admin::over(
        km_admin_pages::machine::Capabilities::desktop(),
        km_admin_pages::ICON_ADMIN_PNG,
        host.clone(),
        host.clone(),
    )
    .drawn_by(APP_NAME)
    // **The two scripts this program's own pages need, and the machine's host declares none.**
    // `_job.html` replaces itself once a second while a search or a download runs, which is the one
    // thing on these pages a form cannot do; `ui.js` is there because htmx will not swap a non-2xx
    // response, so a refusal with no handling looks like a control that does nothing.
    //
    // Absolute paths to this program's own root-mounted `static/`, which is deliberately not under
    // `/admin` — see `router`. The shared crate serves its stylesheet and its mark from
    // `/admin/static/`, and two `static/` under one prefix is two things to tell apart for no gain.
    .with_scripts(&["/static/htmx.min.js", "/static/ui.js"]);

    // **The shared pages and this program's own, under one prefix.** Two routers because they have
    // two states — see `Pages` — and merged rather than nested twice so that a link from one to the
    // other is an ordinary relative path.
    let mine = own_pages(Pages {
        state,
        admin: admin.clone(),
    });
    // **Outside the nest, not inside it.** `nest` rewrites the URI it hands the inner router, so a
    // layer applied within sees `/connect` where this one sees `/admin/connect` — and a prefix test
    // against the wrong spelling of the path would redirect the door to itself forever.
    Router::new()
        .nest("/admin", km_admin_pages::router(admin).merge(mine))
        .layer(axum::middleware::from_fn_with_state(state2, to_the_door))
}

/// With no machine chosen, every page but the front door is the front door.
///
/// **Not a guard, and it must not be read as one.** This program is loopback-only and has no
/// password of its own; what this enforces is that the tabs are about *a machine*, and there is
/// nothing for them to be about until one is picked. A tab drawn over no machine is a page of blank
/// facts and controls that every one of them refuses.
///
/// **On this program's merged router and not inside `km_admin_pages`**, because the shared crate
/// serves the machine's own `/admin/` too — where there is always a machine and this question cannot
/// arise.
///
/// Two paths are let through whatever the state: the door itself with its fragment and its form, and
/// the static files a page needs to look like a page. A redirect that took the stylesheet with it
/// would leave the door unstyled.
async fn to_the_door(
    axum::extract::State(state): axum::extract::State<State>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path();
    // **Only the owner's pages, and a path this router does not mount is still a 404.** The layer
    // sits on the merged router, so it sees every request — and a blanket redirect would answer a
    // path that is simply absent with the door, which `a_path_with_no_route_is_seen_as_missing`
    // catches by asking whether the sweep can still tell the two apart.
    let mine = path.starts_with("/admin/") || path == "/admin";
    let open = path.starts_with(views::CONNECT_PAGE) || path.starts_with("/admin/static/");
    if mine && !open && state.machine().is_none() {
        return axum::response::Redirect::to(views::CONNECT_PAGE).into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    /// A state over a folder of its own.
    ///
    /// **Never `PathBuf::from(".")`.** The data directory is read from and written to —
    /// `machine.json` lives in it — so a state over the working directory would let one test's
    /// chosen machine reach the next one, and would drop a file in the crate folder of whoever ran
    /// `cargo test`.
    ///
    /// The `TempDir` is returned rather than dropped here, because dropping it deletes the folder.
    fn state() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let state = State::new(dir.path().to_path_buf(), None);
        (dir, state)
    }

    /// The same, already pointed at a machine.
    ///
    /// **Most pages here are behind [`to_the_door`]**, which redirects everything but the front door
    /// while no machine has been chosen — so a test about a page's markup has to say which machine
    /// the page is about first. Nothing answers at this address and nothing has to: the pages read
    /// what the record says, and a machine that does not reply is the state this program is built
    /// around.
    fn state_pointed_at_a_machine() -> (tempfile::TempDir, State) {
        let (dir, state) = state();
        state.set_machine(Some("127.0.0.1:8177".to_owned()));
        (dir, state)
    }

    async fn get_path(path: &str) -> (StatusCode, String, Vec<u8>) {
        let (_dir, state) = state_pointed_at_a_machine();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec();
        (status, content_type, body)
    }

    #[tokio::test]
    async fn the_page_renders() {
        let (status, content_type, body) = get_path("/admin/connect").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"), "{content_type}");
        let html = String::from_utf8(body).expect("utf-8");
        assert!(html.contains(APP_NAME), "the page names the product");
    }

    #[tokio::test]
    async fn every_static_file_is_really_embedded_and_typed() {
        // `include_str!` makes a *missing* file a build failure, which is most of the guarantee.
        // What it cannot catch is a route serving the wrong content type, and a stylesheet sent as
        // `text/plain` is silently ignored by every browser.
        for (path, wanted, must_contain) in [
            ("/static/htmx.min.js", "application/javascript", "htmx"),
            ("/static/ui.js", "application/javascript", ""),
            ("/static/htmx-LICENSE.txt", "text/plain", "Zero-Clause"),
        ] {
            let (status, content_type, body) = get_path(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(content_type.starts_with(wanted), "{path}: {content_type}");
            assert!(!body.is_empty(), "{path} is empty");
            let text = String::from_utf8(body).expect("utf-8");
            assert!(text.contains(must_contain), "{path} is not what it claims");
        }
    }

    #[tokio::test]
    async fn this_programs_mark_is_not_the_machines() {
        // Four programs in this product can be open at once, and the favicon is what tells two tabs
        // apart. One character of `include_bytes!` undoes that by accident, and the offline remote
        // has worn the machine's icon that way.
        let (status, content_type, mine) = get_path("/static/icon.png").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "image/png");
        let (_, _, machine) = get_path("/static/machine.png").await;
        assert!(!mine.is_empty() && !machine.is_empty());
        assert_ne!(
            mine, machine,
            "this program must not wear the machine's icon"
        );
    }

    /// A fixed boundary and one file part, which is all these tests need to build by hand.
    fn multipart(file_name: &str, bytes: usize) -> (String, Vec<u8>) {
        const BOUNDARY: &str = "----km-admin-test";
        let head = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{file_name}\"\r\n\r\n",
            km_api::uploads::FILE_FIELD
        );
        let mut body = head.into_bytes();
        body.extend(std::iter::repeat_n(b'x', bytes));
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={BOUNDARY}"), body)
    }

    async fn post(state: State, path: &str, kind: &str, body: Vec<u8>) -> (StatusCode, String) {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::CONTENT_TYPE, kind)
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The same, reading the `Location` rather than the body.
    ///
    /// **The shared upload route answers a redirect and carries what happened in the query string**,
    /// which is `km-admin-pages`' arrangement — a notice that survives a reload and needs no flash
    /// cookie. So a refusal from it is a `303` naming the reason in a URL, where this program's own
    /// send control used to answer a `400` naming it in a body. The sentence is the same; where to
    /// read it is not.
    async fn post_for_location(state: State, path: &str, kind: &str, body: Vec<u8>) -> String {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::CONTENT_TYPE, kind)
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "an upload answers with a redirect carrying the notice"
        );
        // Percent-decoded enough to read: the notice rides as `said=` with spaces as `+`.
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .replace('+', " ")
    }

    /// A state with a machine that is not there, so a network attempt would be visible.
    ///
    /// Port 1 on loopback: nothing binds it, and **nothing in this repository may bind a
    /// non-loopback address in a test** — a test binary's path carries a build hash, so each rebuild
    /// would raise a fresh Windows firewall prompt and leave a dead rule behind.
    fn state_with_a_dead_machine(dir: &std::path::Path) -> State {
        State::new(dir.to_path_buf(), Some("127.0.0.1:1".to_owned()))
    }

    /// The same, holding a token — for a test about a gate that sits *after* the password check.
    ///
    /// A send refuses before it reads the body when no password has been typed, so without this a
    /// test about the extension gate asserts the password gate instead, and would keep passing with
    /// the extension gate deleted.
    fn state_with_a_dead_machine_signed_in(dir: &std::path::Path) -> State {
        let state = state_with_a_dead_machine(dir);
        state
            .client()
            .expect("a client for the address above")
            .pretend_signed_in();
        state
    }

    /// Writes a pack folder of the shape a finished build leaves behind.
    fn a_pack(data_dir: &std::path::Path, id: &str) -> std::path::PathBuf {
        let dir = crate::pictures::packs_dir(data_dir).join(id);
        std::fs::create_dir_all(&dir).expect("the pack folder");
        std::fs::write(dir.join(format!("{id}.zip")), b"PK\x03\x04").expect("the zip");
        dir
    }

    async fn post_form(state: State, path: &str) -> (StatusCode, String) {
        post(state, path, "application/x-www-form-urlencoded", Vec::new()).await
    }

    /// **The whole point of the change, at the route level.** A pack outlives the build that made
    /// it, is listed, and can be sent — which is what makes the second machine in a house cost no
    /// second build.
    #[tokio::test]
    async fn a_built_pack_is_listed_and_can_be_removed() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let kept = a_pack(dir.path(), "wallpapers-e36b9929");
        let state = State::new(dir.path().to_path_buf(), Some("127.0.0.1:8177".to_owned()));

        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/admin/pictures/packs")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let html = String::from_utf8(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body")
                .to_vec(),
        )
        .expect("utf-8");
        assert!(html.contains("wallpapers-e36b9929.zip"), "{html}");
        // A machine is chosen — `to_the_door` sends every page away until one is — so the pack
        // offers both of the things that can be done with it: sent to that machine, or taken off
        // this computer.
        assert!(html.contains("/send"), "{html}");
        assert!(
            html.contains("/admin/pictures/packs/wallpapers-e36b9929/remove"),
            "{html}"
        );

        let (status, _) =
            post_form(state, "/admin/pictures/packs/wallpapers-e36b9929/remove").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert!(!kept.exists(), "the folder is gone");
    }

    /// **A pack id comes off a URL and a bank id does not**, which is why only this one needs
    /// saying: a bank's path is built from `km_banks`' own table, and a pack's from a name handed
    /// back to a route.
    #[tokio::test]
    async fn a_pack_id_that_is_a_path_reaches_nothing() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let banks = crate::bank::banks_dir(dir.path());
        std::fs::create_dir_all(&banks).expect("the banks folder");
        std::fs::write(banks.join("GeneralUser-GS.sf2"), b"not a bank").expect("a bank");

        for id in ["..%2F..%2Fbanks", "%2E%2E", "..", "%2Fetc"] {
            // **A refusal is a redirect too, so the status alone no longer separates the two.** What
            // says it was refused is the notice the redirect carries: a removal that happened goes
            // back to the page with nothing to report.
            let landed = post_for_location(
                State::new(dir.path().to_path_buf(), Some("127.0.0.1:8177".to_owned())),
                &format!("/admin/pictures/packs/{id}/remove"),
                "application/x-www-form-urlencoded",
                Vec::new(),
            )
            .await;
            assert!(landed.contains("kind=bad"), "{id} was acted on: {landed}");
        }
        assert!(
            banks.join("GeneralUser-GS.sf2").is_file(),
            "nothing outside the packs folder was touched"
        );
    }

    /// **Fetching a bank that is already here is work with no result**, a bank running to a
    /// gibibyte. Nothing starts, which is what "no job" asserts.
    #[tokio::test]
    async fn a_bank_that_is_already_here_is_not_fetched_again() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let bank = km_banks::bank("generaluser").expect("a row in the table");
        let banks = crate::bank::banks_dir(dir.path());
        std::fs::create_dir_all(&banks).expect("the banks folder");
        std::fs::write(banks.join(bank.name), b"already here").expect("a bank");

        let state = State::new(dir.path().to_path_buf(), None);
        let (status, _) = post_form(state.clone(), "/admin/sound/fetch/generaluser/get").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert!(
            state.sound_job().is_none(),
            "nothing was started, so nothing was downloaded"
        );
    }

    /// Sending needs a machine, and lands on the page that picks one.
    ///
    /// **`to_the_door` answers before the handler does**, so a press that needs a machine puts
    /// somebody on the page that picks one rather than in front of a sentence saying they need one.
    /// The handler keeps its own guard, [`crate::handlers::stage_and_send`] being called from more
    /// than a route, and a browser does not meet it.
    #[tokio::test]
    async fn sending_something_that_is_here_lands_on_the_door_until_a_machine_is_chosen() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        a_pack(dir.path(), "wallpapers-e36b9929");
        let state = State::new(dir.path().to_path_buf(), None);

        let said = post_for_location(
            state.clone(),
            "/admin/pictures/packs/wallpapers-e36b9929/send",
            "application/x-www-form-urlencoded",
            Vec::new(),
        )
        .await;
        assert_eq!(said, "/admin/connect", "{said}");
        assert!(state.pictures_job().is_none(), "no job was left running");
    }

    // **Two tests went with the pages they were about.** `the_songs_page_renders` and
    // `the_nav_marks_the_page_it_is_on` asserted a Songs page and a nav strip this program no longer
    // draws — `km-admin-pages` does, and its own suite covers both:
    // `the_tab_showing_is_the_one_marked_current` for the strip, and
    // `a_tools_capabilities_take_the_machines_own_controls_away` for what this surface's Songs page
    // holds. `the_owners_page_is_served_from_the_shared_crate` in `tests/machine.rs` is the one that
    // proves this program is the host of them.
    //
    // `Chrome::section` was the string those two guarded against `layout.html`'s, and both are gone.

    /// The front door wears the shared `<head>` and none of the shared chrome.
    ///
    /// **Both halves matter and the second is the one that would rot quietly.** The page has to look
    /// like this product — the stylesheet, the mark, the heading — and it must not carry a tab strip,
    /// because every entry in that strip leads to a page about a machine and this is the page
    /// somebody is on before there is one. See `Admin::door`.
    #[tokio::test]
    async fn the_front_door_wears_the_head_and_not_the_strip() {
        let (status, content_type, body) = get_path("/admin/connect").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"), "{content_type}");
        let html = String::from_utf8(body).expect("utf-8");
        assert!(
            html.contains(r#"<link rel="stylesheet" href="/admin/static/admin.css"#),
            "the shared frame wraps it: {html}"
        );
        assert!(html.contains(APP_NAME), "the heading names the product");
        assert!(
            !html.contains(r#"<nav class="tabs">"#),
            "no strip over a page with no machine: {html}"
        );
    }

    /// The door offers a language for this program, and choosing one writes the cookie both halves
    /// of every page read.
    ///
    /// **The whole point of the control is the cookie**, so that is what is asserted rather than
    /// the `<select>` alone. This program is a fourth program on a loopback port of its own: no
    /// remote is mounted beside it to write `km_locale`, so before this door there was nothing on
    /// this origin that could — and its pages followed `Accept-Language` with no way to disagree.
    ///
    /// The path is checked too, because the router declares `/connect/locale` and the template
    /// spells the `/admin` prefix by hand.
    #[tokio::test]
    async fn the_door_offers_a_language_and_choosing_one_is_remembered() {
        let (_status, _content_type, body) = get_path("/admin/connect").await;
        let html = String::from_utf8(body).expect("utf-8");
        assert!(
            html.contains(r#"action="/admin/connect/locale""#),
            "the picker posts to the route the router mounts: {html}"
        );
        assert!(
            html.contains("Português (Brasil)"),
            "and each language names itself rather than being translated: {html}"
        );

        let (_dir, state) = state_pointed_at_a_machine();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/connect/locale")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("locale=pt-BR"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(
            cookie.starts_with(&format!("{}=pt-BR;", km_locale::COOKIE)),
            "the choice is remembered on this device: {cookie}"
        );
        assert!(
            cookie.contains("Path=/") && cookie.contains("HttpOnly"),
            "and it reaches every page here, out of reach of script: {cookie}"
        );
    }

    /// With no machine chosen, every page but the door is the door.
    ///
    /// **Not a guard and not a password**, which the middleware's own doc says at length: this
    /// program is loopback-only and has none. What it enforces is that a tab is about a machine, and
    /// a tab drawn over no machine is blank facts and controls that all refuse.
    #[tokio::test]
    async fn nothing_but_the_door_answers_until_a_machine_is_chosen() {
        let (_dir, state) = state();
        for path in ["/admin/machine", "/admin/songs", "/admin/sound/fetch"] {
            let response = router(state.clone())
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
            assert_eq!(
                response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok()),
                Some("/admin/connect"),
                "{path}"
            );
        }
        // ...and the door itself answers, or there would be nowhere to go.
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/admin/connect")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Every page this program draws itself loads the stylesheet and the two scripts it needs.
    ///
    /// # The regression this exists for
    ///
    /// **This program stopped loading all three and every test still passed.** Its own
    /// `templates/layout.html` carried the one `<link rel="stylesheet">` and the two `<script>`
    /// tags; deleting that file was the point of moving to the shared pages, and nothing here
    /// noticed the tags going with it. The searching still ran, still finished and still wrote its
    /// files — `_job.html` simply never replaced itself again, so a download that took nine hours
    /// reported the first second of itself for the whole of it, and a refusal drew no toast at all.
    ///
    /// So this asks the three questions no other test here asks: is the sheet linked, are the
    /// scripts loaded, and is what they point at actually served. Every one of them is about a page
    /// *fetching* something rather than about markup, which is the gap the whole suite had.
    #[tokio::test]
    async fn this_programs_own_pages_load_what_they_need() {
        for path in [
            "/admin/connect",
            "/admin/pictures/find",
            "/admin/sound/fetch",
        ] {
            let (status, _, body) = get_path(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            let html = String::from_utf8(body).expect("utf-8");
            assert!(
                html.contains(r#"<link rel="stylesheet" href="/admin/static/admin.css"#),
                "{path} draws no stylesheet"
            );
            assert!(
                html.contains(r#"<script src="/static/htmx.min.js" defer></script>"#),
                "{path} does not load htmx, so its job fragment cannot report progress"
            );
            assert!(
                html.contains(r#"<script src="/static/ui.js" defer></script>"#),
                "{path} does not load this program's own script, so a refusal shows nothing"
            );
            // **The element that script needs, and a script without it is worse than no script.**
            // `ui.js` looks the tray up by id and returns when it is absent, so every refusal it
            // words goes nowhere -- a download reporting its first second for an hour, and no way
            // to tell that from a server that never answered. Asserted on a **rendered** page,
            // because an element that is missing is exactly what a file scan cannot see.
            assert!(
                html.contains(r#"<div id="toasts" aria-live="polite" data-sending="#),
                "{path} has nowhere to put a message, so ui.js discards every one"
            );
        }

        // And the three are really there to be fetched. `every_static_file_is_really_embedded_and_typed`
        // asks this of the scripts; the stylesheet is the shared crate's route, so it is asked here.
        for path in [
            "/static/htmx.min.js",
            "/static/ui.js",
            "/admin/static/admin.css",
        ] {
            let (status, _, body) = get_path(path).await;
            assert_eq!(status, StatusCode::OK, "{path} is linked but not served");
            assert!(!body.is_empty(), "{path} is empty");
        }
    }

    /// The root names the door, and names it temporarily.
    ///
    /// **The `Location` is half the assertion and the status is the other half.** A redirect a
    /// browser keeps is followed without the server being asked, so a door that moves leaves every
    /// browser that has been here pointed at a path nothing mounts.
    #[tokio::test]
    async fn the_root_sends_a_browser_to_the_front_door() {
        let (_dir, state) = state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some(views::CONNECT_PAGE),
        );
    }

    #[tokio::test]
    async fn each_upload_route_takes_more_than_the_default() {
        // **The regression the machine's own page paid for.** Without a per-route
        // `DefaultBodyLimit`, axum's 2 MB default makes every one of these unusable for its real
        // payload, and the failure names neither a size nor a limit. Three megabytes is over the
        // default and under all three real caps, so the only thing being asserted is that the layer
        // is present at all — not which refusal a machineless state goes on to produce.
        let dir = tempfile::tempdir().expect("a temporary folder");
        for (path, name) in [
            ("/admin/songs/upload", "carols.kmpkg"),
            ("/admin/pictures/upload", "beach.jpg"),
            ("/admin/sound/upload", "piano.sf2"),
        ] {
            let (kind, body) = multipart(name, 3 * 1024 * 1024);
            let (status, said) =
                post(state_with_a_dead_machine(dir.path()), path, &kind, body).await;
            assert_ne!(
                status,
                StatusCode::PAYLOAD_TOO_LARGE,
                "{path} still has axum's 2 MB default: {said}"
            );
        }
    }

    #[tokio::test]
    async fn the_default_limit_is_still_on_every_other_route() {
        // The converse, and the reason the layers are per route rather than on the router: a form
        // that takes an address has no business accepting three megabytes of it.
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (_, body) = multipart("beach.jpg", 3 * 1024 * 1024);
        let (status, _) = post(
            State::new(dir.path().to_path_buf(), Some("127.0.0.1:8177".to_owned())),
            "/admin/connect/use",
            "application/x-www-form-urlencoded",
            body,
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn a_file_of_the_wrong_kind_is_refused_before_anything_is_read() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (kind, body) = multipart("notes.txt", 32);
        let said = post_for_location(
            state_with_a_dead_machine_signed_in(dir.path()),
            "/admin/songs/upload",
            &kind,
            body,
        )
        .await;
        assert!(said.contains("kind=bad"), "and it is an error: {said}");
        assert!(
            said.contains(".kmpkg"),
            "the refusal says what it takes: {said}"
        );
        assert!(said.contains(".txt"), "and what it was given: {said}");
        // Nothing was staged, because nothing was accepted.
        assert!(
            !crate::staging::dir(dir.path()).exists()
                || std::fs::read_dir(crate::staging::dir(dir.path()))
                    .expect("the folder")
                    .count()
                    == 0
        );
    }

    /// An upload with no machine chosen never reaches the body, and lands on the door.
    ///
    /// **The body being unread is the half worth keeping.** A package is gigabytes and the answer
    /// does not depend on a byte of it, so a refusal that read it first would spend minutes to say
    /// something it knew at once. `to_the_door` runs before any extractor.
    #[tokio::test]
    async fn an_upload_with_no_machine_lands_on_the_door_without_reading_the_body() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (kind, body) = multipart("carols.kmpkg", 32);
        let said = post_for_location(
            State::new(dir.path().to_path_buf(), None),
            "/admin/songs/upload",
            &kind,
            body,
        )
        .await;
        assert_eq!(said, "/admin/connect", "{said}");
        assert!(
            !crate::staging::dir(dir.path()).exists(),
            "nothing was staged, so nothing was read"
        );
    }

    /// The ladder: what was asked for, then what was chosen last time.
    ///
    /// **And `--machine` does not overwrite what is remembered**, which is the half worth pinning: a
    /// one-off `--machine` for a test must not quietly replace the address somebody normally uses.
    /// It is `km_remote_core::find::locate`'s ordering, minus the rung that adopts whatever the
    /// network answers — this program writes files onto what it is pointed at, so it may not.
    #[test]
    fn the_command_line_beats_what_was_remembered_without_replacing_it() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        crate::chosen::save(dir.path(), "http://192.168.1.42:8177");

        let asked = State::new(dir.path().to_path_buf(), Some("192.168.1.9".to_owned()));
        assert_eq!(asked.machine().as_deref(), Some("http://192.168.1.9:8177"));
        assert_eq!(
            crate::chosen::load(dir.path()).map(|known| known.url),
            Some("http://192.168.1.42:8177".to_owned()),
            "the run's own address must not have been written down"
        );

        // ...and with nothing asked for, the remembered one is what comes back.
        let remembered = State::new(dir.path().to_path_buf(), None);
        assert_eq!(
            remembered.machine().as_deref(),
            Some("http://192.168.1.42:8177")
        );
    }

    /// Entering on the address this run already holds writes nothing down.
    ///
    /// **This is what keeps the rule above true now that the front door opens every launch.** The
    /// door pre-selects the machine this run is pointed at, which under `--machine` is the command
    /// line's address — so pressing its button on a `--machine` run would otherwise write down for
    /// good the address that was meant for one run. [`crate::handlers::enter`] skips the write when
    /// the address is the one already held, which also keeps a re-entry after a refused write from
    /// throwing away the token it already has along with the client.
    #[tokio::test]
    async fn entering_on_the_machine_this_run_already_holds_writes_nothing_down() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        crate::chosen::save(dir.path(), "http://192.168.1.42:8177");
        let state = State::new(dir.path().to_path_buf(), Some("192.168.1.9".to_owned()));

        let said = post_for_location(
            state.clone(),
            "/admin/connect/use",
            "application/x-www-form-urlencoded",
            b"row=chosen".to_vec(),
        )
        .await;
        // The door turns this away for want of a password, which is a later rule than the one
        // under test: the address is settled before anything is asked of a machine.
        assert!(said.starts_with("/admin/connect?"), "{said}");
        assert_eq!(
            crate::chosen::load(dir.path()).map(|known| known.url),
            Some("http://192.168.1.42:8177".to_owned()),
            "the run's own address was written down after all"
        );
        assert_eq!(state.machine().as_deref(), Some("http://192.168.1.9:8177"));
    }

    /// A typed address is written down, and is what the next run opens on.
    #[tokio::test]
    async fn a_machine_typed_on_the_door_is_remembered() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let state = State::new(dir.path().to_path_buf(), None);

        let said = post_for_location(
            state,
            "/admin/connect/use",
            "application/x-www-form-urlencoded",
            b"row=typed&typed=192.168.1.50".to_vec(),
        )
        .await;
        // Turned away for want of a password, as every entry now is — and the address is written
        // down all the same, because what is remembered is what somebody chose.
        assert!(said.starts_with("/admin/connect?"), "{said}");
        assert_eq!(
            State::new(dir.path().to_path_buf(), None)
                .machine()
                .as_deref(),
            Some("http://192.168.1.50:8177"),
        );
    }

    /// Submitting the typed row with nothing in it comes back to the door rather than forgetting.
    #[tokio::test]
    async fn an_empty_address_comes_back_to_the_door() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        crate::chosen::save(dir.path(), "http://192.168.1.42:8177");
        let state = State::new(dir.path().to_path_buf(), None);

        let said = post_for_location(
            state,
            "/admin/connect/use",
            "application/x-www-form-urlencoded",
            b"row=typed&typed=%20%20".to_vec(),
        )
        .await;
        assert!(said.starts_with("/admin/connect?"), "{said}");
        assert!(said.contains("kind=bad"), "{said}");
        assert_eq!(
            crate::chosen::load(dir.path()).map(|known| known.url),
            Some("http://192.168.1.42:8177".to_owned()),
            "a blank box forgot the machine somebody had chosen"
        );
    }

    /// The browse lists what it found, and pre-selects none of it.
    ///
    /// **Listing is not setting**, which on a page of radios is exactly this assertion: the only row
    /// that may open selected is the machine somebody already chose, and a discovered one has to be
    /// pressed. See `Discovering a machine in the package builder`.
    #[tokio::test]
    async fn a_discovered_row_is_never_preselected() {
        // No watcher in a test — see `Inner::watcher` — so the list is empty and what is asserted is
        // the markup's shape rather than a sighting. The fragment is what would carry a `checked`.
        let (_dir, state) = state_pointed_at_a_machine();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/admin/connect/found")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body")
                .to_vec(),
        )
        .expect("utf-8");
        assert!(
            !html.contains("checked"),
            "a machine the browse turned up opened selected: {html}"
        );
    }

    /// A machine chosen on the page is there again next run, and clearing the field forgets it.
    ///
    /// *No machine* is the state a fresh install is in, and an address held in memory alone puts
    /// every start back into it.
    #[test]
    fn a_machine_chosen_on_the_page_comes_back_next_run() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        assert_eq!(State::new(dir.path().to_path_buf(), None).machine(), None);

        State::new(dir.path().to_path_buf(), None)
            .set_machine(Some("http://192.168.1.42:8177".to_owned()));
        assert_eq!(
            State::new(dir.path().to_path_buf(), None)
                .machine()
                .as_deref(),
            Some("http://192.168.1.42:8177")
        );

        // Clearing the address field is a choice too, and it has to stick the same way.
        State::new(dir.path().to_path_buf(), None).set_machine(None);
        assert_eq!(State::new(dir.path().to_path_buf(), None).machine(), None);
    }

    /// An address somebody types is normalized before it becomes a client.
    ///
    /// **The failure this holds down is a machine that answers everything and a page that says it
    /// does not.** `Client::new` documents a normalized URL and cannot check for one, and a base
    /// with no scheme makes every request a relative path — so the ask fails the instant it is made,
    /// the page reports an unreachable machine, and restarting the program cures it, because
    /// [`State::new`] normalizes what it reads back out of the record. The chooser is the one
    /// control whose whole job is to be typed into, and this is the path it takes.
    #[test]
    fn a_typed_address_becomes_a_client_with_a_scheme_and_a_port() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let state = State::new(dir.path().to_path_buf(), None);
        state.set_machine(Some("192.168.1.5".to_owned()));
        assert_eq!(
            state.machine().as_deref(),
            Some("http://192.168.1.5:8177"),
            "the client this run will use"
        );
        // And what was written down, or the next start would normalize a second time to reach the
        // same place and the record would disagree with the page that showed it.
        assert_eq!(
            crate::chosen::load(dir.path()).map(|known| known.url),
            Some("http://192.168.1.5:8177".to_owned())
        );

        // A bare host:port keeps its port, which is the form the chooser's own rows carry.
        state.set_machine(Some("127.0.0.1:8477".to_owned()));
        assert_eq!(state.machine().as_deref(), Some("http://127.0.0.1:8477"));
    }

    /// A remembered address is normalized on the way back in.
    ///
    /// The record is a file somebody can edit, so a bare `192.168.1.5` in its `url` should mean what
    /// it means everywhere else rather than producing a URL with no scheme and no port.
    #[test]
    fn a_hand_edited_address_is_normalized_like_a_typed_one() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(
            crate::chosen::path(dir.path()),
            r#"{"v":1,"url":"192.168.1.5"}"#,
        )
        .expect("write");
        assert_eq!(
            State::new(dir.path().to_path_buf(), None)
                .machine()
                .as_deref(),
            Some("http://192.168.1.5:8177")
        );
    }

    #[test]
    fn a_lan_run_is_told_to_use_loopback() {
        // `0.0.0.0` is not an address anything can connect to, and it is what a browser would be
        // handed if this formatted the bound address verbatim.
        assert_eq!(
            browsable_url("0.0.0.0:8180".parse().expect("addr")),
            "http://127.0.0.1:8180"
        );
        assert_eq!(
            browsable_url("127.0.0.1:8180".parse().expect("addr")),
            "http://127.0.0.1:8180"
        );
    }

    /// A shell is handed the door, and the origin is not the door.
    ///
    /// **The pin on the whole arrangement.** `/` is a redirect, a browser keeps the answer it was
    /// given there, and so a shell opening the origin opens wherever it was told last rather than
    /// wherever the door is.
    #[test]
    fn what_a_shell_opens_is_the_front_door() {
        for bound in ["0.0.0.0:8180", "127.0.0.1:8180"] {
            assert_eq!(
                front_door_url(bound.parse().expect("addr")),
                "http://127.0.0.1:8180/admin/connect",
                "{bound}"
            );
        }
    }
}
