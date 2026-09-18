//! Shared state, authorization, and getting the thing listening.
//!
//! Binding and serving are separate steps on purpose. `km-app` needs the resolved
//! [`ConnectInfo`] — the address to print and put in a QR code — *before* it starts serving, because
//! the display comes up first and an idle screen with no address on it is the failure this whole
//! mechanism exists to avoid. So [`bind`] returns a [`Listening`] that already knows its address,
//! and [`Listening::serve`] runs until told to stop.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use axum::http::HeaderMap;
use axum::serve::ListenerExt as _;

use crate::auth::{AdminAuth, bearer_token};
use crate::config::ApiConfig;
use crate::connect::{ConnectInfo, resolve};
use crate::discover::{Advert, AdvertAction, Discovery, advert_action, advertisable_addresses};
use crate::error::{ApiError, ApiResult};
use crate::events::{Events, run_state_ticker};
use crate::listener::{HeldListener, Relisten};
use crate::machine::{Catalog, Controller};
use crate::power::Power;

/// How often the reachable address is worked out again.
///
/// DHCP leases renew, Wi-Fi roams between access points, somebody unplugs the Ethernet cable. Any of
/// those changes the address on screen, and thirty seconds of a stale URL is a tolerable window
/// where a permanently stale one is not.
pub const CONNECT_REFRESH: Duration = Duration::from_secs(30);

/// How often the advertisement is compared against the address the refresher last published.
///
/// **Not the same question as [`CONNECT_REFRESH`], and deliberately shorter.** That one costs a
/// `get_if_addrs()` syscall, and thirty seconds of a stale URL on a screen is tolerable. This one is
/// an `RwLock` read and a comparison of two short vectors, and its only job is to avoid being a
/// *second* thirty-second wait stacked behind the first — two tickers of the same period can sit in
/// any phase relative to each other, so the worst case would otherwise be a minute of a machine
/// nobody can discover. Five seconds bounds that at thirty-five.
const ADVERT_REFRESH: Duration = Duration::from_secs(5);

struct Inner {
    catalog: Arc<dyn Catalog>,
    controller: Arc<dyn Controller>,
    events: Events,
    auth: AdminAuth,

    connect: RwLock<ConnectInfo>,
    /// Whether the password in force is the one the machine generated for itself.
    ///
    /// Live rather than read from the config, for [`ApiState::factory_password`]'s reason.
    factory_password: RwLock<bool>,
    /// What this machine is called, which can change while it runs.
    ///
    /// Seeded from [`ApiConfig::machine_name`] and then owned here, for the reason `acl` is: the
    /// config is a startup snapshot and this is a live value. Reading it from the config instead is
    /// exactly the bug that made a rename take effect in `settings.json` and nowhere a phone could
    /// see, so `discovery()` reads this and nothing reads the config field after construction.
    name: RwLock<String>,
    /// What this host can do about its own power, if anything.
    ///
    /// **A `OnceLock` rather than an `RwLock<Option<…>>`, and rather than a constructor argument.**
    /// The three `RwLock`s above are there because they genuinely change while the machine runs; a
    /// capability does not — it is a fact about the box this process started on. And a fifth
    /// constructor would have been the alternative: `new`, `with_events`, `from_machine` and
    /// `from_machine_with_events` already exist and all funnel through one, so an argument would
    /// have had to be threaded through four signatures and every call site that does not have one.
    ///
    /// Empty is the ordinary case and not a degraded one — see [`crate::power`] for why an absent
    /// capability is an unmounted route rather than a route that refuses.
    power: OnceLock<Arc<dyn Power>>,
    /// The machine's own recent log, where the program running it keeps one.
    ///
    /// A `OnceLock` beside [`Inner::power`] for that field's reasons, and absent for the same kind
    /// of reason: a program that installed no tap has nothing to serve, so the two `/admin/logs`
    /// routes are not mounted rather than mounted and empty.
    ///
    /// **No `Arc<dyn …>`, unlike the capability above it.** [`km_logtap::LogTap`] is already a
    /// handle onto a shared buffer, and there is one implementation of a ring of records — a trait
    /// here would be ceremony to abstract over a single type, and a test double would become a
    /// second implementation of the thing under test.
    log_tap: OnceLock<km_logtap::LogTap>,
    /// Whether the machine currently holds its port.
    ///
    /// False only while [`crate::listener::HeldListener`] is between sockets. It is read by
    /// [`run_connect_refresher`], which must not publish a healthy-looking address over the
    /// `ServerFailed` the listener put there: the refresher works out where a phone *would* connect,
    /// which stays true of the interfaces and stops being true of the machine.
    listening: AtomicBool,
    /// Raised by the host when it has reason to think the socket did not survive.
    relisten: Relisten,
    config: ApiConfig,
}

/// Everything a handler needs, cheap to clone.
#[derive(Clone)]
pub struct ApiState {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ApiState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiState")
            .field("bind", &self.inner.config.bind)
            .field("admin_configured", &self.admin_configured())
            .finish_non_exhaustive()
    }
}

impl ApiState {
    /// Builds the state around a machine, with a fresh event channel.
    pub fn new(
        catalog: Arc<dyn Catalog>,
        controller: Arc<dyn Controller>,
        config: ApiConfig,
    ) -> Self {
        Self::with_events(catalog, controller, config, Events::new())
    }

    /// Builds the state around a machine, sharing an existing event channel.
    ///
    /// The machine itself needs to publish: a song ending and the next one starting is not something
    /// any HTTP request caused, so no handler is there to announce it. Creating the channel first and
    /// handing it to both sides is the whole reason this constructor exists — the alternative is the
    /// machine holding a slot that is empty until the API is built, and a window where autonomous
    /// events go nowhere.
    pub fn with_events(
        catalog: Arc<dyn Catalog>,
        controller: Arc<dyn Controller>,
        config: ApiConfig,
        events: Events,
    ) -> Self {
        let auth = match &config.admin_password_hash {
            Some(hash) => AdminAuth::with_hash(hash.clone()),
            None => AdminAuth::disabled(),
        }
        .with_ttl(config.token_ttl)
        .with_epoch(config.session_epoch);

        // Resolved eagerly so `GET /discover` and the display have an answer from the first request,
        // rather than a hole until the refresh task's first tick.
        let factory_password = RwLock::new(config.factory_password);
        let connect = RwLock::new(resolve(config.bind, config.factory_password));
        let name = RwLock::new(config.machine_name.clone());
        Self {
            inner: Arc::new(Inner {
                catalog,
                controller,
                events,
                auth,

                connect,
                factory_password,
                name,
                power: OnceLock::new(),
                log_tap: OnceLock::new(),
                // True before anything is bound, because the only caller that can make it false is
                // the listener itself, and a machine whose API never started publishes its own
                // `ServerFailed` from `bind_with`'s error path instead.
                listening: AtomicBool::new(true),
                relisten: Relisten::new(),
                config,
            }),
        }
    }

    /// Builds the state where one object is both catalog and controller.
    pub fn from_machine<M>(machine: Arc<M>, config: ApiConfig) -> Self
    where
        M: Catalog + Controller,
    {
        Self::new(machine.clone(), machine, config)
    }

    /// Builds the state where one object is both catalog and controller, sharing an event channel.
    pub fn from_machine_with_events<M>(machine: Arc<M>, config: ApiConfig, events: Events) -> Self
    where
        M: Catalog + Controller,
    {
        Self::with_events(machine.clone(), machine, config, events)
    }

    /// The catalog.
    pub fn catalog(&self) -> &dyn Catalog {
        self.inner.catalog.as_ref()
    }

    /// What language this machine speaks, for the routes whose body is prose.
    ///
    /// **One machine, one locale**, unlike a web page a viewer negotiates: the television is in a
    /// room and the room has one language. The song book is the only route that reads this today,
    /// and it lets `?locale=` override it for a book somebody wants in another language.
    pub fn machine_locale(&self) -> km_locale::Locale {
        self.inner.config.locale
    }

    /// The catalog as a shared handle, for work that must leave the async runtime.
    ///
    /// Installing a package indexes every song into SQLite and rebuilds the search index, which for
    /// a large one is seconds — so it runs on a blocking thread and needs an owned handle to take
    /// with it. Same reason as [`ApiState::controller_handle`], different axis: that one outlives
    /// its request, this one must not sit on a worker.
    pub fn catalog_handle(&self) -> Arc<dyn Catalog> {
        self.inner.catalog.clone()
    }

    /// The machine's controls.
    pub fn controller(&self) -> &dyn Controller {
        self.inner.controller.as_ref()
    }

    /// The controller as a shared handle, for tasks that outlive a request.
    pub fn controller_handle(&self) -> Arc<dyn Controller> {
        self.inner.controller.clone()
    }

    /// The event stream, for publishing.
    pub fn events(&self) -> &Events {
        &self.inner.events
    }

    /// Installs what this host can do about its own power.
    ///
    /// **Call it before serving, and once.** Returns whether it took: a second call is a caller
    /// wiring the same machine twice, which is a bug rather than a race, so it is reported instead
    /// of panicking or silently replacing a live capability.
    ///
    /// The router reads [`ApiState::power`] while it is being *built*, so a capability installed
    /// after that would leave a machine that can power itself off with no routes saying so.
    pub fn set_power(&self, power: Arc<dyn Power>) -> bool {
        self.inner.power.set(power).is_ok()
    }

    /// What this host can do about its own power, if anything.
    ///
    /// `None` is the ordinary answer everywhere except a supervised appliance, and it is what
    /// [`crate::routes::router_with`] reads to decide whether the power routes exist at all.
    pub fn power(&self) -> Option<&dyn Power> {
        self.inner.power.get().map(Arc::as_ref)
    }

    /// Installs the machine's own recent log.
    ///
    /// **Call it before serving, and once**, for [`ApiState::set_power`]'s reasons exactly: the
    /// router reads [`ApiState::log_tap`] while it is being built, so a tap installed after that
    /// would leave a machine keeping a log with no route saying so.
    pub fn set_log_tap(&self, tap: km_logtap::LogTap) -> bool {
        self.inner.log_tap.set(tap).is_ok()
    }

    /// The machine's own recent log, where the program running it keeps one.
    ///
    /// `None` in a test, in the `dev_server` example, and in any host that installed no layer — and
    /// it is what [`crate::routes::router_with`] reads to decide whether the log routes exist.
    pub fn log_tap(&self) -> Option<&km_logtap::LogTap> {
        self.inner.log_tap.get()
    }

    /// The configuration.
    pub fn config(&self) -> &ApiConfig {
        &self.inner.config
    }

    /// Admin tokens and the password.
    pub fn auth(&self) -> &AdminAuth {
        &self.inner.auth
    }

    /// Sets the admin password for this run, and says whose it is.
    ///
    /// **Only the running value.** Writing it down is the controller's business, because only the
    /// implementation knows where settings live -- the same division `set_session_epoch` and
    /// `set_machine_name` draw. The handler persists first and calls this second, so a failed write
    /// never leaves a machine answering to a password its own settings file does not hold.
    ///
    /// **`factory` moves with the hash and is not optional**, for [`Self::factory_password`]'s
    /// reason: the two answer *what* the password is and *whose* it is, and a hash set without the
    /// flag following leaves `/discover` telling every client on the network the wrong one.
    pub fn set_admin_password(&self, hash: Option<String>, factory: bool) {
        self.inner.auth.set_hash(hash);
        *self
            .inner
            .factory_password
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = factory;
    }

    /// Whether a password is set at all.
    pub fn admin_configured(&self) -> bool {
        self.inner.auth.is_configured()
    }

    /// The session epoch, and moving it.
    ///
    /// Bumping invalidates every outstanding token without touching the password — see
    /// [`crate::auth::AdminAuth::set_epoch`]. **Only the running value**; persisting it is the
    /// controller's business, the same division [`Self::set_admin_password`] draws.
    pub fn session_epoch(&self) -> u64 {
        self.inner.auth.epoch()
    }

    /// Replaces the session epoch for this run.
    pub fn set_session_epoch(&self, epoch: u64) {
        self.inner.auth.set_epoch(epoch);
    }

    /// Where the web UI is, as last resolved.
    pub fn connect_info(&self) -> ConnectInfo {
        match self.inner.connect.read() {
            Ok(connect) => connect.clone(),
            // Reporting the failure honestly rather than a blank panel: the display's job is to say
            // something true, and "the server could not tell you" is true.
            Err(_) => ConnectInfo::server_failed(
                "the address could not be read",
                self.inner.config.bind.port(),
            ),
        }
    }

    /// Replaces the resolved address. Called by the refresh task, and by `km-app` if it learns the
    /// server failed to start.
    pub fn set_connect_info(&self, connect: ConnectInfo) {
        *self
            .inner
            .connect
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = connect;
    }

    /// Whether the machine is holding its port at this moment.
    pub fn is_listening(&self) -> bool {
        self.inner.listening.load(Ordering::Acquire)
    }

    /// Says whether the machine is holding its port.
    ///
    /// [`crate::listener::HeldListener`] is the only caller, either side of taking the port again.
    pub(crate) fn set_listening(&self, listening: bool) {
        self.inner.listening.store(listening, Ordering::Release);
    }

    /// The handle that asks the server to take its port again.
    ///
    /// **The host holds this, because the host is what knows.** A system that suspends an
    /// application destroys its listening socket, and what comes back can be a descriptor that
    /// neither accepts nor errors. No accept loop can tell that from a quiet network, so returning
    /// to the screen is a fact only the platform's own event can supply.
    pub fn relisten(&self) -> Relisten {
        self.inner.relisten.clone()
    }

    /// What this machine is called on the network.
    ///
    /// The live value, not [`ApiConfig::machine_name`], which is only the seed — see `Inner::name`.
    pub fn machine_name(&self) -> String {
        self.inner
            .name
            .read()
            .map(|name| name.clone())
            .unwrap_or_else(|_| self.inner.config.machine_name.clone())
    }

    /// Renames the machine for this run.
    ///
    /// **Only the running value.** Writing it down is the controller's business, because only the
    /// implementation knows where settings live — the same division `acl_changed` draws. The handler
    /// persists first and calls this second, so a failed write never leaves the machine answering to
    /// a name its own settings file does not hold.
    pub fn set_machine_name(&self, name: impl Into<String>) {
        *self
            .inner
            .name
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = name.into();
    }

    /// The discovery payload, including the catalog size when it can be had.
    pub fn discovery(&self) -> Discovery {
        Discovery::new(
            &self.inner.config.instance_id,
            self.machine_name(),
            &self.connect_info(),
            // A catalog that cannot be counted is reported as unknown rather than as zero. "0
            // songs" and "we could not ask" look identical to a remote and mean different things.
            self.inner.catalog.song_count().ok(),
        )
        // Same rule: unknown rather than zero. A mirror that read `0` would conclude the catalog
        // had never changed and skip a refresh it needed.
        .with_catalog_version(self.inner.catalog.catalog_version().ok())
        // So a curation tool can ask before it sends a video it is about to be refused — see the
        // field's own note for why asking afterwards does not reliably work at all.
        .with_debugging(self.inner.config.debug_enabled)
        // **Asked of `AdminAuth` rather than taken from `ConnectInfo`, and that is a fix.**
        // `ConnectInfo` carries an `auth` too, but it is a *snapshot*: it is built when the address
        // is resolved and refreshed only when the address changes. Since `POST /admin/password` made
        // the password settable while the machine runs, that snapshot has been able to say `none`
        // about a machine whose routes are answering 401 — for the rest of the process's life, since
        // setting a password does not move an address.
        //
        // What that costs is precisely the thing every client does first: `discover` is public on
        // every machine and is how a remote asks *should I expect to need a password*. An owner who
        // set one from their phone left every client believing there was none until the next restart.
        .with_factory_password(self.factory_password())
    }

    /// Whether this machine is still on the password it generated for itself.
    ///
    /// **A live value behind a lock, not the config's snapshot, and that distinction has bitten this
    /// exact field before under another name.** `ApiConfig` is built at startup; a password can be
    /// changed while the machine runs, through `POST /api/v1/admin/password` or the owner's page, and
    /// changing one does not restart anything. Reading the snapshot left `/discover` reporting
    /// `factory_password: true` about a machine whose owner had just set their own — the routes
    /// behaving correctly the whole time, which is the worse half, because a client that trusts the
    /// announcement stops asking.
    ///
    /// Found by driving a real machine over a real socket. Every unit test in the crate passed: they
    /// build a state and ask it, which is the snapshot being correct at the only moment they look.
    pub fn factory_password(&self) -> bool {
        self.inner
            .factory_password
            .read()
            .map(|factory| *factory)
            .unwrap_or(false)
    }

    /// Decides whether a request may proceed.
    ///
    /// **The path is the whole input, and the caller's address is deliberately not one.** A path
    /// under `/api/v1/admin/` wants a token; every other path is public. There is no map to consult
    /// and nothing an owner can have configured, which is the point: the permission is visible in
    /// the URL and cannot drift away from the router. See [`crate::routes::needs_admin_token`].
    ///
    /// **A machine with no password refuses rather than opens.** That is the inversion this replaced
    /// — the old rule made an `admin` route public when no password was set, on the reasoning that
    /// an owner who had not asked for a door should not have one. Every machine has a password now,
    /// so the state cannot arise in normal running; if it somehow does, refusing is the safe answer.
    pub fn authorize(&self, path: &str, headers: &HeaderMap) -> ApiResult<()> {
        if !crate::routes::needs_admin_token(path) {
            return Ok(());
        }
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token);
        match token {
            Some(token) if self.inner.auth.verify(token) => Ok(()),
            Some(_) => Err(ApiError::Unauthorized(
                "that token is not valid; log in again".to_owned(),
            )),
            None => Err(ApiError::Unauthorized(format!(
                "'{path}' needs the admin password; send an admin token"
            ))),
        }
    }
}

/// A bound but not yet serving API.
///
/// Holds the listener, so the port is claimed and the address known before anything else starts.
pub struct Listening {
    /// The address actually bound. With port 0 requested, this is the port the OS chose.
    pub local_addr: SocketAddr,
    /// Where a remote should connect, resolved against this machine's interfaces.
    pub connect: ConnectInfo,
    state: ApiState,
    listener: tokio::net::TcpListener,
    router: axum::Router,
    /// Shared with [`run_advertiser`], which replaces it when the address changes.
    ///
    /// Shared rather than owned because the advertisement is no longer decided once: the task keeps
    /// it matching the reachable address, and `serve_with_shutdown` still has to be able to withdraw
    /// it deterministically on the way out.
    advert: Arc<Mutex<Option<Advert>>>,
}

impl std::fmt::Debug for Listening {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Listening")
            .field("local_addr", &self.local_addr)
            .field("urls", &self.connect.urls)
            .field(
                "advertising",
                &self.advert.lock().is_ok_and(|slot| slot.is_some()),
            )
            .finish_non_exhaustive()
    }
}

impl Listening {
    /// The shared state, so `km-app` can publish events into the same channel.
    pub fn state(&self) -> &ApiState {
        &self.state
    }

    /// Serves until the process ends.
    pub async fn serve(self) -> std::io::Result<()> {
        self.serve_with_shutdown(std::future::pending()).await
    }

    /// Serves until `shutdown` resolves.
    ///
    /// The state ticker and the address refresher run alongside, and stop with it: they are spawned
    /// here rather than by the caller so that a `km-api` server always honors the 4 Hz state
    /// contract and always re-resolves its address, whatever `km-app` remembers to do.
    pub async fn serve_with_shutdown<S>(self, shutdown: S) -> std::io::Result<()>
    where
        S: Future<Output = ()> + Send + 'static,
    {
        let Self {
            state,
            listener,
            router,
            advert,
            local_addr,
            ..
        } = self;

        let ticker = tokio::spawn(run_state_ticker(
            state.controller_handle(),
            state.events().clone(),
        ));
        // The *bound* address, for the reason `bind` gives: with port 0 the requested address and
        // the real one differ, and the refresher must not undo what `bind` resolved.
        let refresher = tokio::spawn(run_connect_refresher(state.clone(), local_addr));
        let advertiser = tokio::spawn(run_advertiser(
            state.clone(),
            local_addr.port(),
            Arc::clone(&advert),
        ));

        // **Wrapped rather than handed over, and that is what keeps the machine reachable.** A
        // socket is not held for the life of a process: a platform that suspends an application
        // destroys it while the application is away, and an interface going down takes one with it.
        // `axum::serve` cannot help with either, because the stock listener treats every accept
        // failure as transient and retries against the same descriptor for ever. See
        // [`crate::listener`].
        //
        // **`tap_io` with an empty closure, and it is load-bearing.** A handler asks for the caller's
        // address through `ConnectInfo<SocketAddr>`, and `axum` supplies that for its own
        // `TcpListener` and for any listener wrapped in `TapIo`, by two impls it owns. The obvious
        // third impl cannot be written here: `Connected` and `SocketAddr` are both foreign, and a
        // local type buried in a foreign one's parameters does not satisfy the orphan rule. Wrapping
        // reaches the blanket impl that already exists, and the closure runs once per accepted
        // connection and does nothing.
        let held = HeldListener::new(listener, local_addr, state.clone()).tap_io(|_| {});
        let result = axum::serve(
            held,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await;

        ticker.abort();
        refresher.abort();
        // **Awaited, and not merely aborted.** `abort()` asks; it does not wait. The advertiser can
        // be sitting inside `Advert::start` with a finished `Advert` in a local, and that local is
        // dropped whenever the runtime next gets to the task — possibly after this function has
        // returned, by which point there may be no runtime left to send a goodbye on. Awaiting the
        // handle resolves only once the task's frame really is gone, which is what lets the take
        // below be the last word.
        advertiser.abort();
        let _ = advertiser.await;
        // Withdrawing the advertisement is the last thing, so a phone browsing the network stops
        // seeing a machine that has gone.
        //
        // Taken out of the shared slot and dropped here, rather than left to a dropped `Arc`: the
        // task holds a handle too, and an `Arc` going out of scope withdraws nothing until the
        // *last* one does. Dropping the `Advert` itself is what sends the goodbye.
        let withdrawn = advert
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        drop(withdrawn);
        result
    }
}

/// Claims the port and works out the reachable address.
///
/// Fails rather than falling back to another port: a port conflict is something the operator must
/// see, and silently moving to 8178 would make every QR code and every remembered bookmark wrong.
pub async fn bind(state: ApiState) -> std::io::Result<Listening> {
    bind_with(state, crate::routes::Extras::default()).await
}

/// The same, with the singer-facing remote mounted at `/`.
///
/// Separate from [`bind`] rather than an argument on it, so that every caller who does not serve a
/// remote — the dev server, most tests — reads exactly as it did.
pub async fn bind_with(
    state: ApiState,
    extras: crate::routes::Extras,
) -> std::io::Result<Listening> {
    let config = state.config().clone();
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let local_addr = listener.local_addr()?;

    // Resolve against the address actually bound, not the one requested: with port 0 they differ,
    // and the URL on screen has to name the port a phone can reach.
    let connect = resolve(local_addr, config.factory_password);
    state.set_connect_info(connect.clone());

    // **Binding no longer decides whether this machine is advertised.** It used to: a single
    // `Advert::start` here, and on failure a `warn!` and `None` for the life of the process. That is
    // exactly the shape of the fault it caused — on every cold boot of the appliance the machine
    // bound its port about three seconds before dhcpcd had a lease, so there was no address to
    // advertise, and the machine then served HTTP happily while announcing nothing at all. Nothing
    // ever looked again, so `km-remote` could not find it until somebody restarted the service by
    // hand.
    //
    // [`run_advertiser`] owns it now, start to finish, and this is deliberately not a *second* place
    // that can open one: two owners would be two error paths and a window in which both hold an
    // `Advert` for the same name. Nothing is lost by waiting for the task — `tokio::time::interval`
    // fires its first tick immediately, so a machine whose network is already up begins advertising
    // microseconds into `serve` rather than microseconds before it.
    let advert = Arc::new(Mutex::new(None));

    let router = crate::routes::router_with(state.clone(), extras);
    Ok(Listening {
        local_addr,
        connect,
        state,
        listener,
        router,
        advert,
    })
}

/// Re-resolves the reachable address every [`CONNECT_REFRESH`].
///
/// `bound` is the address the listener actually holds, **not** `config.bind`. Resolving the requested
/// one again looks equivalent and is not: a server asked for port 0 was bound to a real port by
/// `bind`, which resolved and stored it — and `tokio::time::interval` fires its first tick
/// immediately, so this task would overwrite that with port 0 within microseconds of the server
/// starting. Which end of the race won depended on the machine: it passed on Windows, failed in CI on
/// Linux and macOS, and reads as a flaky test rather than as the address being wrong. A remote reading
/// `GET /discover` and dialling port 0 gets nowhere, and the URL under the QR code would be as wrong
/// as the number in the test.
async fn run_connect_refresher(state: ApiState, bound: SocketAddr) {
    let mut ticker = tokio::time::interval(CONNECT_REFRESH);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let bind = bound;

    loop {
        ticker.tick().await;
        // **Read per tick, not once before the loop.** It is a live value: an owner who changes the
        // password while the machine runs must not leave the panel on screen still offering a PIN
        // that no longer works.
        let factory_password = state.factory_password();
        // Off the runtime: `resolve` enumerates the machine's network interfaces, which is
        // `GetAdaptersAddresses` on Windows and `getifaddrs` elsewhere — a blocking syscall, and not
        // a cheap one on a box carrying Hyper-V, WSL and a VPN adapter. It cannot hold a request
        // open by itself, being a task of its own, but it can take a worker away from one every
        // thirty seconds for no reason at all.
        let Ok(resolved) =
            tokio::task::spawn_blocking(move || resolve(bind, factory_password)).await
        else {
            // The only way that fails is a panic inside `if_addrs`. Try again on the next tick
            // rather than ending the task: an address that stops being re-resolved is a QR code
            // that goes quietly wrong hours later.
            tracing::warn!("could not re-resolve the reachable address; trying again shortly");
            continue;
        };
        // **Nothing is published while the machine is between sockets.** This task answers where a
        // phone *would* connect, which stays true of the interfaces while it stops being true of the
        // machine, so publishing here would paint a healthy address over the `ServerFailed` the
        // listener put on the screen and hide the one fault worth seeing.
        if !state.is_listening() {
            continue;
        }
        if resolved != state.connect_info() {
            tracing::info!(urls = ?resolved.urls, "the reachable address changed");
            state.set_connect_info(resolved);
        }
    }
}

/// Keeps the mDNS advertisement matching the address the machine is actually reachable at.
///
/// **A separate task from [`run_connect_refresher`] rather than a branch inside it**, for three
/// reasons that all point the same way: it needs only that function's *published* result, which is
/// `state.connect_info()`; a slow `get_if_addrs()` and a slow service registration should not queue
/// behind one another; and working out an address and announcing one are two different jobs.
///
/// **It polls, and polling is the honest answer here rather than a shortcut.** There is no watch or
/// notify on [`ApiState`] — `connect` is an `RwLock` that every synchronous caller reads — and
/// adding one would buy a few seconds of latency at the cost of reshaping the most contended field
/// in this file. A timer is needed regardless: a `ServiceDaemon` that would not open is worth trying
/// again even when nothing about the address has changed, and a notify would never fire for that.
async fn run_advertiser(state: ApiState, port: u16, slot: Arc<Mutex<Option<Advert>>>) {
    if !state.config().advertise_mdns || crate::discover::mdns_declined() {
        // Returning rather than ticking to no purpose. `without_mdns()` is what the test harness
        // sets, so this is the common case in this workspace, and a task waking twelve times a
        // minute to decide it has nothing to do is a cost every test would pay.
        //
        // The environment is read here as well as in `Advert::start` so that a declined run is
        // quiet rather than merely unsuccessful: without this the timer wakes, asks for a daemon it
        // will not get, and complains once about a machine that was told not to advertise.
        return;
    }
    let mut ticker = tokio::time::interval(ADVERT_REFRESH);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // What is currently registered, as opposed to what is reachable. `None` is "not advertising".
    // The name is remembered beside the addresses because it is published too, so a rename has to be
    // noticed here or it never reaches the network -- see `advert_action`.
    let mut published: Option<(String, Vec<IpAddr>)> = None;
    let mut complained = false;
    loop {
        ticker.tick().await;
        // **The loopback guard, and it is the same one it has always been.**
        // `advertisable_addresses` drops loopback, so a server bound to `127.0.0.1` yields an empty
        // list, `advert_action` answers `Keep`, and this task never reaches `ServiceDaemon::new()`.
        // See "No test binds a non-loopback address" in docs/ARCHITECTURE.md: being bound to loopback
        // *is* the guard, and putting the advertisement on a timer is precisely the change that
        // could have lost it — before this, a test could only open a daemon by getting `bind` wrong
        // once, and now there is something that would try again every five seconds.
        let wanted = advertisable_addresses(&state.connect_info());
        let wanted_name = state.machine_name();
        let current = published
            .as_ref()
            .map(|(name, addresses)| (name.as_str(), addresses.as_slice()));
        match advert_action(current, (&wanted_name, &wanted)) {
            AdvertAction::Keep => {}
            AdvertAction::Withdraw => {
                let gone = slot
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                drop(gone);
                published = None;
                tracing::debug!("withdrew the mDNS advertisement — no reachable address");
            }
            AdvertAction::Publish(name, addresses) => {
                // **The old advertisement is dropped before the new one is made, and the order is
                // load-bearing.** `Advert::drop` unregisters *by fullname*, and a replacement
                // carries the same fullname — so registering first and dropping afterwards would
                // send a goodbye for the record just published, and every phone browsing the
                // network would drop the machine it had only just found.
                let previous = slot
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                drop(previous);
                // Built after the address rather than before it: `discovery()` reads
                // `connect_info`, and the whole point here is that it has just changed.
                let discovery = state.discovery();
                match Advert::start(&discovery, &addresses, port) {
                    Ok(advert) => {
                        *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                            Some(advert);
                        published = Some((name, addresses));
                        complained = false;
                    }
                    Err(error) => {
                        published = None;
                        // Not fatal, and deliberately so: mDNS is blocked on plenty of networks, and
                        // the QR code on screen is what actually connects a phone. Said once and
                        // then not again, because this ticks every five seconds for the life of the
                        // machine and a blocked network would otherwise fill a journal. The latch
                        // clears on the next success, so a *later* failure is heard.
                        if complained {
                            tracing::debug!(%error, "still not advertising over mDNS");
                        } else {
                            tracing::warn!(%error, "continuing without an mDNS advertisement");
                            complained = true;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::header::AUTHORIZATION;

    use super::*;
    use crate::routes::{ADMIN_LOGIN_PATH, API_PREFIX};
    use crate::testing::TestMachine;

    const ADMIN_ROUTE: &str = "/api/v1/admin/demo";
    const PUBLIC_ROUTE: &str = "/api/v1/queue";

    fn state_with(config: ApiConfig) -> ApiState {
        ApiState::from_machine(TestMachine::with_catalog(3).shared(), config)
    }

    fn passworded() -> ApiState {
        state_with(
            ApiConfig::default()
                .with_password("hunter2")
                .expect("hashing works"),
        )
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {token}").parse().expect("header"),
        );
        headers
    }

    fn log_in(state: &ApiState) -> String {
        state
            .auth()
            .login(std::net::Ipv4Addr::LOCALHOST.into(), "hunter2")
            .expect("the password is right")
            .token
    }

    #[test]
    fn a_public_route_needs_nothing() {
        let state = passworded();
        assert!(state.authorize(PUBLIC_ROUTE, &HeaderMap::new()).is_ok());
        assert!(
            state
                .authorize("/api/v1/settings", &HeaderMap::new())
                .is_ok()
        );
    }

    #[test]
    fn an_admin_route_without_a_token_is_a_401() {
        let state = passworded();
        let error = state
            .authorize(ADMIN_ROUTE, &HeaderMap::new())
            .expect_err("an admin route must refuse");
        assert!(matches!(error, ApiError::Unauthorized(_)));
        assert!(error.to_string().contains(ADMIN_ROUTE));
    }

    #[test]
    fn a_valid_token_opens_an_admin_route() {
        let state = passworded();
        let token = log_in(&state);
        assert!(state.authorize(ADMIN_ROUTE, &bearer(&token)).is_ok());
    }

    #[test]
    fn a_made_up_token_does_not() {
        let state = passworded();
        let error = state
            .authorize(ADMIN_ROUTE, &bearer(&"0".repeat(64)))
            .expect_err("a forged token must refuse");
        assert!(error.to_string().contains("log in again"));
    }

    /// The inversion at the heart of the change. This state used to open every admin route; now it
    /// shuts them, because a machine that cannot verify a token cannot admit anybody.
    #[test]
    fn with_no_password_an_admin_route_is_shut_rather_than_open() {
        let state = state_with(ApiConfig::default());
        assert!(!state.admin_configured());
        assert!(state.authorize(ADMIN_ROUTE, &HeaderMap::new()).is_err());
        assert!(
            state
                .authorize("/api/v1/admin/packages", &HeaderMap::new())
                .is_err()
        );
        // ...and the public half is unaffected, which is what keeps a phone working.
        assert!(state.authorize(PUBLIC_ROUTE, &HeaderMap::new()).is_ok());
    }

    #[test]
    fn login_stays_reachable_because_it_is_the_way_in() {
        let state = passworded();
        assert!(state.authorize(ADMIN_LOGIN_PATH, &HeaderMap::new()).is_ok());
    }

    #[test]
    fn discover_is_reachable_from_anywhere() {
        let state = passworded();
        let path = format!("{API_PREFIX}/discover");
        assert!(state.authorize(&path, &HeaderMap::new()).is_ok());
    }

    /// Sign-out-everywhere, driven through the state the way a handler drives it.
    #[test]
    fn bumping_the_session_epoch_shuts_every_token_out() {
        let state = passworded();
        let token = log_in(&state);
        assert!(state.authorize(ADMIN_ROUTE, &bearer(&token)).is_ok());

        state.set_session_epoch(state.session_epoch() + 1);
        assert!(state.authorize(ADMIN_ROUTE, &bearer(&token)).is_err());
    }

    #[test]
    fn changing_the_password_shuts_every_token_out() {
        let state = passworded();
        let token = log_in(&state);
        assert!(state.authorize(ADMIN_ROUTE, &bearer(&token)).is_ok());

        state.set_admin_password(
            Some(crate::auth::AdminAuth::hash_password("something else").expect("hashing works")),
            false,
        );
        assert!(state.authorize(ADMIN_ROUTE, &bearer(&token)).is_err());
    }

    /// A token minted before a restart has to keep working, and the only thing that carries across
    /// one is the stored hash. Two states built from the same config is what a restart looks like
    /// from here.
    #[test]
    fn a_token_outlives_the_state_that_issued_it() {
        let config = ApiConfig::default()
            .with_password("hunter2")
            .expect("hashing works");
        let token = {
            let before = state_with(config.clone());
            log_in(&before)
        };
        let after = state_with(config);
        assert!(after.authorize(ADMIN_ROUTE, &bearer(&token)).is_ok());
    }

    /// Setting a password of the owner's own stops `factory_password` being true, *at once*.
    ///
    /// **The regression test for a bug the whole unit suite missed**, found by driving a real machine
    /// over a real socket: `factory_password` was read from `ApiConfig`, which is a startup snapshot,
    /// so `/discover` went on reporting `true` about a machine whose owner had just changed the
    /// password — with the routes behaving correctly the whole time, which is the worse half.
    ///
    /// It is the same class of bug the machine's *name* had, one field over, and the note in
    /// `discovery()` warning about exactly this had been written before the field existed.
    #[test]
    fn changing_the_password_stops_the_machine_calling_it_a_factory_one() {
        let mut config = ApiConfig::default()
            .with_password("123456")
            .expect("hashing works");
        config.factory_password = true;
        let state = state_with(config);
        assert!(state.factory_password());
        assert!(state.discovery().factory_password);

        state.set_admin_password(
            Some(crate::auth::AdminAuth::hash_password("carols1975").expect("hashing works")),
            false,
        );
        assert!(!state.factory_password(), "the live value did not move");
        assert!(
            !state.discovery().factory_password,
            "discovery is still reading a snapshot"
        );
    }

    #[test]
    fn a_machine_reports_whether_it_is_still_on_its_factory_password() {
        let mut config = ApiConfig::default()
            .with_password("123456")
            .expect("hashing works");
        config.factory_password = true;
        assert!(state_with(config.clone()).factory_password());
        config.factory_password = false;
        assert!(!state_with(config).factory_password());
    }
}
