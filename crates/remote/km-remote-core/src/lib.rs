//! The offline karaoke remote, as a library.
//!
//! Everything the remote *is* — this device's own copy of a machine's catalog, the favorites
//! collection, the client for a machine that may be switched off, and the server that puts
//! [`km_remote_pages`]'s pages in front of all three. Everything the remote *runs as* is somewhere else:
//! `km-remote` is the desktop shell over this crate, and the Android and iOS applications will
//! be two more.
//!
//! # Why `core`
//!
//! A code review read this crate as four responsibilities — mirror, favorites, client, discovery —
//! under a name that says none of them, and proposed a rename. It does not survive the first line
//! above: those four are not four things sharing a bag, they are what *one* remote is, and there is
//! no arrangement in which a remote has the catalog but not the client. `core` is the half that is
//! not a host, which is exactly what the next section is about and what four shells over it make it.
//!
//! # Why this is a library and not a `main`
//!
//! **A `main` is the one part of a server that does not travel.** It reads `argv`, works out where
//! its files go, installs a log subscriber, prints a banner, and blocks until somebody presses
//! Ctrl-C — and a phone has none of those five things. So none of them are in here, and the
//! `Cargo.toml` says so by not depending on `clap`, `tracing-subscriber`, `directories` or
//! `km-console`.
//!
//! Four seams carry the differences instead, and each is a field or a phase rather than a
//! `#[cfg(target_os = …)]`. That distinction is the whole design: every one of these differs
//! between *hosts* rather than between platforms, so a `cfg` could not express it even where it
//! guessed the platform right.
//!
//! * **Where the files go is injected.** [`Config::data_dir`] has no default. The `directories`
//!   crate ships `lin.rs`, `mac.rs`, `win.rs` and `wasm.rs` and nothing else, so on Android it
//!   silently takes the Linux XDG path — which depends on `$HOME`, is normally unset there, and
//!   falls back to a working directory of `/`, which is not writable. The host knows the answer
//!   (`ProjectDirs`, `Context.getFilesDir()`, `NSDocumentDirectory`); this crate asks for it.
//! * **The bound address is readable before anything slow happens.** [`Bound`] and [`Server`] are
//!   two types rather than one call, so a port of `0` is answerable and a WebView can be pointed at
//!   a real URL while a cold first import is still running.
//! * **Shutdown is a future the caller supplies.** A closed window, an Activity's `onDestroy` and
//!   Ctrl-C are three hosts' spellings of one idea; `tokio::signal` is one of them and belongs to
//!   the desktop.
//! * **Discovery is a trait.** An mDNS browse on Android sees nothing unless Java is holding a
//!   `WifiManager.MulticastLock` for its duration, and on iOS it needs
//!   `NSLocalNetworkUsageDescription` in the bundle and a permission prompt. See [`Locator`](crate::find::Locator).
//!
//! # Starting one
//!
//! Four phases, each an ordinary `async fn` the caller awaits, so a host can put its own work
//! between any two of them:
//!
//! ```no_run
//! # async fn example() -> anyhow::Result<()> {
//! use km_remote_core::{Bound, Config, Server, Stop};
//!
//! let config = Config::new("/some/where");
//! let bound = Bound::bind(&config).await?;          // 1. listening; the port is now knowable
//! let server = Server::open(bound, config).await?;  // 2. databases open, machine located
//! println!("{}", server.ready().url);
//!
//! let stop = Stop::default();
//! server.spawn_warm_up();                           // 3. catalog refresh, behind the pages
//! server.serve(stop.asked()).await                  // 4. answers until asked to stop
//! # }
//! ```
//!
//! **Phases rather than a callback**, and that is a considered choice rather than a style. A
//! `ready` callback cannot be awaited, forces a `Send + 'static` closure across whatever boundary
//! the host is on, and inverts control for nothing — where a phase boundary is a place a test can
//! stand and a host can do its own work. [`run`] is all four at once, for a caller with nothing to
//! say in between.

pub mod client;
pub mod favdb;
pub mod find;
pub mod link;
pub mod mirror;
pub mod sync;

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use km_remote_pages::machine::Connect;
use km_remote_pages::{Capabilities, Remote};

use crate::client::{Api, MachineClient};
use crate::favdb::{FavDb, FavDbFavorites};
use crate::find::{Found, Locator};
use crate::link::Link;
use crate::mirror::{Mirror, MirrorSongs};

/// The port the remote serves on unless a host says otherwise.
///
/// One past `km-package-builder`'s 8178, which is one past the machine's own 8177. Named here
/// rather than in a `clap` attribute so the number is defined once and a second shell cannot pick a
/// different one by accident.
pub const DEFAULT_PORT: u16 = 8179;

/// This crate's tracing target, for the three shells that name it in a filter string.
///
/// Defined here for the reason `DEFAULT_PORT` is: so the number — or here the name — is written
/// once and a second shell cannot pick a different one by accident. `CARGO_CRATE_NAME` answers for
/// the crate doing the asking, so a host cannot derive this one for itself. See
/// [`km_remote_pages::LOG_TARGET`], which exists for the same reason one crate over.
pub const LOG_TARGET: &str = env!("CARGO_CRATE_NAME");

/// `km-remote-pages`'s tracing target, passed on for the same three shells.
///
/// They all filter on the pages as well as on this crate, and none of them depends on that one —
/// they reach it only through here. Re-exporting the name costs nothing and keeps a dependency edge
/// from being added for the sake of a string.
pub use km_remote_pages::LOG_TARGET as PAGES_LOG_TARGET;

/// Everything the remote needs to start, and nothing a command line had to provide.
///
/// **There is no `Default`, and that absence is the design.** A `Default` would have to invent a
/// [`data_dir`](Self::data_dir), and inventing it is exactly what goes wrong off the desktop: the
/// `directories` crate ships `lin.rs`, `mac.rs`, `win.rs` and `wasm.rs` and nothing else, so on
/// Android it silently takes the Linux XDG path — which depends on `$HOME`, is normally unset
/// there, and falls back to a working directory of `/`, which is not writable. A crate that cannot
/// answer the question should be made to ask it.
pub struct Config {
    /// Where `catalog.sqlite`, `favorites.sqlite` and `last-machine.json` live. Created if absent.
    pub data_dir: PathBuf,

    /// Which interface, and which port.
    ///
    /// `port: 0` is supported and means "whatever is free"; [`Bound::address`] is how the caller
    /// learns what it got. A `--lan` flag does not appear here — choosing loopback or `0.0.0.0` is
    /// the host's decision, and this is where that decision lands rather than a second spelling of
    /// it.
    pub bind: SocketAddr,

    /// An address somebody named, or `None` to try what was remembered and then the network.
    pub machine: Option<String>,

    /// Re-read the whole catalog even when `catalog_version` says nothing has changed.
    pub force_refresh: bool,

    /// How to look for a machine on the network. See [`find::Locator`] for why this is a field.
    pub locator: Arc<dyn Locator>,

    /// What the remote may do.
    ///
    /// [`Capabilities::offline`] is the only value any shell passes today. It is a field so that a
    /// host wanting a reduced remote does not have to fork this crate to get one.
    pub capabilities: Capabilities,

    /// What the browser tab shows, as PNG bytes.
    ///
    /// Beside `capabilities` and for the same reason: it is the *shell's* answer, and a shell that
    /// is not this product would want its own. The default is [`km_remote_pages::ICON_REMOTE_PNG`], the
    /// green microphone, because every shell over this crate is the offline remote — the desktop
    /// window today, an Android and an iOS application later. The machine's own remote does not come
    /// through here at all; it takes `km-remote-pages`'s default, which is the amber one.
    pub icon_png: &'static [u8],
}

impl Config {
    /// The only constructor. The data directory has no default; everything else does.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            bind: SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
            machine: None,
            force_refresh: false,
            #[cfg(feature = "mdns")]
            locator: Arc::new(find::Mdns::new()),
            #[cfg(not(feature = "mdns"))]
            locator: Arc::new(find::NoLocator),
            capabilities: Capabilities::offline(),
            icon_png: km_remote_pages::ICON_REMOTE_PNG,
        }
    }

    /// Which interface and port to bind. See [`bind`](Self::bind).
    #[must_use]
    pub fn with_bind(mut self, bind: SocketAddr) -> Self {
        self.bind = bind;
        self
    }

    /// An address somebody named, skipping discovery entirely.
    #[must_use]
    pub fn with_machine(mut self, machine: Option<String>) -> Self {
        self.machine = machine;
        self
    }

    /// Re-read the catalog whatever its version says.
    #[must_use]
    pub fn with_force_refresh(mut self, force: bool) -> Self {
        self.force_refresh = force;
        self
    }

    /// How to look for a machine — [`find::NoLocator`] to not look at all.
    #[must_use]
    pub fn with_locator(mut self, locator: Arc<dyn Locator>) -> Self {
        self.locator = locator;
        self
    }
}

/// A shutdown somebody can ask for from elsewhere.
///
/// Clone it and hand one half to whatever can ask — a window's close button, an Activity's
/// `onDestroy`, a Ctrl-C handler. **`notify_one` rather than `notify_waiters`**, because it stores
/// a permit: asking before anybody is waiting still works, and that case is real rather than
/// theoretical, since a window can be closed while the first catalog import is still running.
#[derive(Clone, Default)]
pub struct Stop(Arc<tokio::sync::Notify>);

impl Stop {
    /// Asks the server to stop. Idempotent, and safe to call before anybody is listening.
    pub fn ask(&self) {
        self.0.notify_one();
    }

    /// The future to hand [`Server::serve`]. Resolves once [`ask`](Self::ask) has been called.
    pub async fn asked(self) {
        self.0.notified().await;
    }
}

/// What a run turned out to be, once there is something to say about it.
///
/// Handed back rather than pushed through a callback, so that a host prints it, stores it, or
/// ignores it entirely without this crate knowing which.
#[derive(Debug, Clone)]
pub struct Ready {
    /// What the socket actually bound to. Real even when [`Config::bind`] asked for port 0.
    pub address: SocketAddr,
    /// The loopback URL — see [`Bound::url`].
    pub url: String,
    /// Where the two databases went. Echoed back because a host that computed it wants it in a
    /// banner, and reading it off `Config` would mean keeping the whole struct alive to print one
    /// line.
    pub data_dir: PathBuf,
    /// The machine this run will talk to, if one was named, remembered or found.
    ///
    /// `None` is ordinary and is not a failure: browsing, searching and favorites all answer
    /// without one, which is the entire reason this program exists.
    pub machine: Option<Found>,
    /// How many songs the mirror already holds, or why it could not be counted. A `String` rather
    /// than an error type because the only thing anybody does with it is show it to a person.
    pub songs: Result<usize, String>,
}

/// A listening socket, and nothing else yet.
///
/// **The first phase is separate from the second on purpose.** The operating system accepts into
/// the backlog from the moment a socket is bound, so a browser or a WebView pointed here during a
/// cold first import waits on a tab that is loading rather than one that was refused. It is also
/// the only way to answer "which port?" before the answer is needed, which [`Config::bind`] with a
/// port of 0 makes compulsory.
pub struct Bound {
    listener: tokio::net::TcpListener,
    address: SocketAddr,
    url: String,
}

impl Bound {
    /// Binds, and does nothing else. Everything slow is in [`Server::open`].
    pub async fn bind(config: &Config) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(config.bind)
            .await
            .with_context(|| format!("binding {}", config.bind))?;
        let address = listener
            .local_addr()
            .context("asking the socket what it bound to")?;
        Ok(Self {
            url: format!("http://127.0.0.1:{}/", address.port()),
            listener,
            address,
        })
    }

    /// What the socket really got, port included.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// `http://127.0.0.1:<port>/` — what a browser or a webview should be pointed at.
    ///
    /// **Deliberately not [`address`](Self::address) formatted.** Bound to every interface that
    /// reads `0.0.0.0:8179`, which is a valid thing to bind and not a thing anything can connect to.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// The remote, assembled and not yet answering.
pub struct Server {
    listener: tokio::net::TcpListener,
    remote: Remote,
    ready: Ready,
    /// Which machine, how it was found, and everything that can change that.
    ///
    /// **One field where there were six.** [`Server::machine_watch`] closed over the client, the
    /// locator, the data directory, the mirror, the force-refresh flag and whether a machine was
    /// named; the Now tab's machine card wants exactly the same six, so they are a value with a name
    /// rather than a capture list written out twice. See [`crate::link`].
    link: Arc<Link>,
}

impl Server {
    /// Opens the two databases, finds a machine, and builds the state the pages are served from.
    ///
    /// Everything between the bind and the banner. The machine lookup runs under
    /// `spawn_blocking`: a [`Locator`](crate::find::Locator) is a blocking call by design, and the binary this crate came
    /// out of made it straight from `#[tokio::main]`, so looking for a television that was switched
    /// off stalled the whole runtime for a second and a half.
    pub async fn open(bound: Bound, config: Config) -> Result<Self> {
        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("creating {}", config.data_dir.display()))?;

        let mirror = Mirror::open(&config.data_dir).context("opening the catalog copy")?;
        let songs = MirrorSongs::new(mirror);
        let held = songs.handle();

        let favorites = FavDb::open(&config.data_dir).context("opening the favorites")?;
        favorites.seed().context("making the first folder")?;
        let favorites = FavDbFavorites::new(favorites);

        let machine = MachineClient::new();

        // **The radar comes first, so it is already listening while everything below runs.** That
        // is the whole change of shape: a look used to start when somebody wanted an answer, so the
        // answer was always as old as the question. With a watcher the registry has been filling
        // itself in since the process began, and the third rung of `locate` costs nothing.
        let radar = Arc::new(find::Radar::new(Arc::clone(&config.locator)));

        let known = find::known(&config.data_dir);
        let stale = known.as_ref().is_some_and(|known| {
            km_api::discover::known::is_stale(known, std::time::SystemTime::now())
        });

        let found = {
            let asked = config.machine.clone();
            let known = known.clone();
            let radar = Arc::clone(&radar);
            tokio::task::spawn_blocking(move || {
                find::locate(asked.as_deref(), known.as_ref(), &radar)
            })
            .await
            .context("looking for a machine")?
        };
        // **Pointed at, but not written down.** Remembering here is what made a dead address
        // permanent: `locate` had verified nothing, so an address that has not answered since August
        // was rewritten on every start, and because a remembered address short-circuits the browse
        // it could never be replaced by the machine actually on the network. The address is
        // remembered by [`Server::machine_watch`] once something answers at it instead.
        if let Some(found) = &found {
            machine.point_at(Api::new(&found.url));
        }

        // **A record older than `STALE_AFTER` gets the network asked at once**, and that is the
        // whole of what age changes. The address above is still used immediately either way — the
        // trade `A remote looks again when its machine goes quiet` bought, that opening instantly
        // beats opening in a second and a half, is untouched. What being stale buys is that the
        // watch does not wait for the connection to fail before correcting the address.
        if stale {
            radar.poke().await;
        }

        let link = Arc::new(Link::new(
            machine.clone(),
            Arc::clone(&radar),
            config.data_dir.clone(),
            Arc::clone(&held),
            config.force_refresh,
            link::Origin {
                pinned: config.machine.is_some(),
                how: found.as_ref().map(|found| found.how),
                known,
            },
        ));

        let remote = Remote::new(
            Arc::new(songs.clone()),
            Arc::new(machine),
            Some(Arc::new(favorites)),
            config.capabilities,
        )
        .with_icon(config.icon_png)
        // What makes the Now tab's machine card appear. The online remote passes no such thing, and
        // `Capabilities::connection` is what its templates read to know that.
        .with_connect(Arc::clone(&link) as Arc<dyn Connect>);

        let count = {
            use km_remote_pages::machine::Songs;
            songs.count().await.map_err(|error| error.to_string())
        };

        Ok(Self {
            ready: Ready {
                address: bound.address,
                url: bound.url,
                data_dir: config.data_dir,
                machine: found,
                songs: count,
            },
            listener: bound.listener,
            remote,
            link,
        })
    }

    /// What a host prints, stores, or ignores.
    pub fn ready(&self) -> &Ready {
        &self.ready
    }

    /// The state the pages are served from, for a host that wants to reach past this crate.
    pub fn remote(&self) -> &Remote {
        &self.remote
    }

    /// The client for the machine — **live**, unlike [`Ready::machine`].
    ///
    /// [`Ready`] is a report of what [`Server::open`] found, and it is right to be a snapshot: it is
    /// what a host prints at startup. This is the other question, and a host that shows a screen
    /// about the machine has to ask it instead: [`Server::machine_watch`] re-points this client
    /// every time it finds one, so `machine().api()` is the only thing that answers "what are we
    /// talking to *now*".
    ///
    /// **Added because both mobile shells got that wrong in the same way**, which is what a shared
    /// `km-remote-host` made visible: each showed a "no machine found" screen, polled the snapshot
    /// waiting for it to change, and waited for ever while the log showed the server finding a
    /// machine, mirroring the catalog and connecting. The screen was a dead end on a first run on
    /// a quiet network — the exact case the polling was written to rescue.
    pub fn machine(&self) -> &MachineClient {
        self.link.machine()
    }

    /// Which machine, how it was found, and the three ways of changing that.
    ///
    /// The same value the Now tab's card is drawn from — a host that wants to offer the choice
    /// somewhere of its own asks this rather than reassembling it out of [`Server::machine`] and a
    /// guess at the rest.
    pub fn link(&self) -> &Arc<Link> {
        &self.link
    }

    /// Brings the catalog copy up to date.
    ///
    /// Awaitable for a host that wants to finish before serving; see
    /// [`spawn_warm_up`](Self::spawn_warm_up) for the usual case, which does not.
    pub async fn warm_up(&self) {
        self.warming().await;
    }

    /// The same work, detached, so that pages are answered while it runs.
    ///
    /// **This is what a first run needs and what the binary this came from did not do.** A cold
    /// import of a six-figure catalog is a minute, and it used to happen before `axum::serve` was
    /// called at all — so every request sat in the accept backlog until it finished. On a desktop
    /// that is a slow tab; on a phone it is a launch indistinguishable from a hang. The pages are
    /// served from whatever the mirror already holds, which is the offline app's whole premise.
    pub fn spawn_warm_up(&self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.warming())
    }

    /// The body of both, holding only what it needs so that it can outlive the borrow.
    fn warming(&self) -> impl Future<Output = ()> + Send + 'static {
        let api = self.link.machine().api();
        let link = Arc::clone(&self.link);
        async move {
            if let Some(api) = api {
                refresh_catalog(&api, &link, link.force_refresh()).await;
            }
        }
    }

    /// Looks for a machine again when the one in hand stops answering, and writes down the one that
    /// does.
    ///
    /// **Two jobs, one loop, because they are the same question asked either way round.** A remote
    /// that cannot reach its machine should look for another; a remote that can should record where
    /// it is. Splitting them would mean two timers reading the same connection.
    ///
    /// Why this exists: [`find::locate`] runs **once**, at [`Server::open`], and prefers a
    /// remembered address over the network so that opening is instant. That is the right trade for
    /// the common case and it has no way back — a machine that moved, or a remembered address that
    /// was never reachable, could not be replaced by the one actually advertising itself, and the
    /// event stream would retry the dead address for as long as the process lived. Recovering here
    /// rather than at startup keeps the instant open and still ends up on the right machine.
    ///
    /// **Nothing is remembered until it answers.** The address is written down when the connection
    /// is *online*, not when it is chosen, which is what stops an address that has never worked
    /// being rewritten on every start and short-circuiting the browse for ever.
    fn machine_watch(&self) -> impl Future<Output = ()> + Send + 'static {
        let link = Arc::clone(&self.link);
        async move {
            let machine = link.machine().clone();
            // `connection` is a trait method, brought in here rather than at the top of the file for
            // the same reason `Songs` is in `open`: one use, one scope.
            use km_remote_pages::machine::Machine;

            loop {
                // **Push where there is a push, and a tick underneath it.** The watcher wakes this
                // the moment a machine appears, disappears or changes address, so the case the whole
                // feature is about now fires in under a second instead of up to twenty. The tick
                // stays for three reasons that are easy to miss: `Sweep` has nothing to push and
                // must be polled, a watcher whose daemon would not open needs somebody to call
                // `poke` on it, and a poll is how a host with no watcher looks at all.
                let attention = machine.attention();
                tokio::select! {
                    () = link.radar().changed() => {}
                    () = attention.notified() => {
                        // Somebody is looking at a page again. On Android that means the multicast
                        // lock has just been retaken, so this is the moment a query is worth
                        // sending; see `MachineClient::wake`.
                        link.radar().poke().await;
                    }
                    () = tokio::time::sleep(find::RECOVERY_INTERVAL) => {}
                }

                let online = machine.connection().online;
                let current = machine.api().map(|api| api.base().to_owned());

                // A machine somebody named is not one to wander away from; see [`Link::pinned`].
                //
                // **Read every tick rather than captured once**, because the pin is not only a
                // command-line fact: typing an address into the Now tab's card sets it and *Rescan*
                // clears it, and a loop holding the value from startup would go on obeying an
                // instruction somebody has since withdrawn.
                if link.pinned() {
                    continue;
                }

                // A host with no watcher learns nothing without being asked, and only while it is
                // worth asking: 1,021 TCP connects every twenty seconds from a phone in a pocket
                // that is happily connected would be a battery bug rather than a feature.
                if !online && link.radar().watches_by_itself() {
                    // Nothing to do: the registry is current by construction.
                } else if !online {
                    let _ = link.look().await;
                }

                let known = link.known();
                let answering = link.answering_id();
                let seen = link.radar().seen();
                let stale = known.as_ref().is_some_and(|known| {
                    km_api::discover::known::is_stale(known, std::time::SystemTime::now())
                });
                let situation = km_api::discover::known::Situation {
                    pinned: false,
                    online,
                    current: current.as_deref(),
                    known: known.as_ref(),
                    seen: &seen,
                    answering_id: answering.as_deref(),
                    stale,
                    // **The remote may adopt, and the two tools that write to a machine may not.**
                    // The worst thing that happens here is mirroring the wrong catalog, which
                    // `A remote looks again when its machine goes quiet` weighs against a remote
                    // that sits idle beside a working machine; the worst thing that happens to
                    // `km-package-builder` is a package installed on somebody else's box.
                    adopts: true,
                };
                let km_api::discover::known::Choice::Use { url, why } =
                    km_api::discover::known::choose(&situation)
                else {
                    continue;
                };

                tracing::info!(machine = %url, why = why.label(), "pointing at a machine");
                link.point_at(&url, why);

                // The catalog is fetched from whatever machine is in hand, so switching machines
                // means the copy is of the wrong one — or, after the migration that discards a
                // catalog written before song codes, of nothing at all. The event stream will
                // reconnect on its own; this is the half it does not do.
                let api = Api::new(&url);
                refresh_catalog(&api, &link, link.force_refresh()).await;
            }
        }
    }

    /// Answers until `shutdown` resolves.
    ///
    /// **No `tokio::signal` in here.** Ctrl-C is one host's idea of "stop"; a closed window and an
    /// Activity's `onDestroy` are two others, and neither is reachable from a signal handler. The
    /// future is the seam, and [`Stop`] is a ready-made one.
    pub async fn serve(self, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<()> {
        tokio::spawn(self.machine_watch());
        client::spawn_event_stream(self.link.machine().clone());
        km_remote_pages::handlers::spawn_pump(self.remote.clone());

        axum::serve(
            self.listener,
            km_remote_pages::router(self.remote).into_make_service(),
        )
        .with_graceful_shutdown(shutdown)
        .await
        .context("serving the remote")
    }
}

/// Brings the mirror up to date with one machine, saying what happened and swallowing what did not.
///
/// Shared by the warm-up and by [`Server::machine_watch`], which needs exactly this after switching
/// to a machine found on the network: the copy in hand was of a different one.
///
/// **Takes the link rather than its three parts** because a refresh now brings something back as
/// well as pushing something in: `/discover` names the machine, and this is the only place on the
/// warm-up path that asks. Recording it here is what makes the name appear on a remote that was
/// opened cold, rather than only after somebody pressed Rescan.
async fn refresh_catalog(api: &Api, link: &Link, force: bool) {
    match sync::refresh(api, link.mirror(), force).await {
        Ok(refreshed) => {
            link.machine().set_machine_name(refreshed.name.clone());
            // ...and the identity, on the same call. This is the warm-up path, so it is what
            // anchors the record on a remote that was opened cold and never touched afterwards.
            if let Some(id) = &refreshed.id {
                link.answered(id, refreshed.name);
            }
            match refreshed.outcome {
                sync::Outcome::AlreadyCurrent { songs } => {
                    tracing::info!(songs, "the song list is already up to date");
                }
                sync::Outcome::Imported { songs, version } => {
                    tracing::info!(songs, version, "copied the song list from the machine");
                }
            }
        }
        // Not fatal, and deliberately: a mirror from last week is a working remote, and refusing to
        // start because a television is off would defeat the point of the thing.
        Err(error) => {
            tracing::warn!(%error, "could not refresh the song list; using the copy already held");
        }
    }
}

/// All four phases, for a host with nothing to say in between.
///
/// The catalog refresh runs behind the pages; see [`Server::spawn_warm_up`].
pub async fn run(
    config: Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let bound = Bound::bind(&config).await?;
    let server = Server::open(bound, config).await?;
    server.spawn_warm_up();
    server.serve(shutdown).await
}

/// What the tests need and the remote does not — the scratch directory, in one copy.
#[cfg(test)]
mod testing;

#[cfg(test)]
mod tests;
