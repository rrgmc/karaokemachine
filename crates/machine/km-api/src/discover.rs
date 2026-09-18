//! Being found — over mDNS, and over HTTP for when mDNS is blocked.
//!
//! One payload, two ways out. The [`Discovery`] struct is what `GET /api/v1/discover` returns and
//! what the mDNS TXT records carry, so a phone that found the machine by browsing and a phone that
//! was handed the URL by QR code learn the same things about it.
//!
//! The HTTP endpoint is not a fallback bolted on afterwards, it is the primary one. mDNS is blocked
//! or unreliable on a great many home networks — guest VLANs, "client isolation" on cheap routers,
//! Windows firewall profiles — and a discovery mechanism that works most of the time is a support
//! burden. The QR code on the display is what actually gets a phone connected; mDNS is the nicety
//! that saves the scan when it works.
//!
//! Deliberately not here: a UDP broadcast responder. `docs/ARCHITECTURE.md` records it as a
//! maybe-later, and adding a second homegrown protocol to work around the second one failing is how
//! this ends up with three.

pub mod known;
pub mod watch;

use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::connect::{ConnectInfo, ConnectProblem};

/// The DNS-SD service type.
pub const SERVICE_TYPE: &str = "_karaokemachine._tcp.local.";

/// The payload's own version, so a future client can tell what it is looking at.
pub const PROTOCOL_VERSION: u32 = 1;

/// The API's base path, told to clients rather than assumed by them.
pub const API_BASE: &str = "/api/v1";

/// What this application calls itself in a [`Discovery`].
///
/// **A constant because it is now read as well as written.** It was a literal in one place while the
/// only thing that did anything with it was a test; `km_remote_core::find::Sweep` compares against
/// it, because a sweep meets whatever happens to be listening on port 8177 — a router's admin page,
/// a printer — and "the body parsed as JSON" is not the same claim as "this is a karaoke machine".
/// Two spellings of it could disagree, and the failure would be a remote that remembered a printer.
pub const APP: &str = "karaokemachine";

/// The environment variable that declines the multicast socket.
///
/// The same shape as `KM_LOG_FILE` and `KM_FRAME_STATS`: any value but `0` counts as yes, so
/// `KM_NO_MDNS=1` declines and `KM_NO_MDNS=0` is an explicit no. The `0` spelling is what lets one
/// command ask for mDNS where the variable is set for everything around it, which is a checkout
/// whose `.cargo/config.toml` sets it and a shell with no way to unset a variable for one process.
pub const NO_MDNS_ENV_VAR: &str = "KM_NO_MDNS";

/// What the variable means, apart from the process it is read from.
///
/// Separate so the rule is a unit test rather than a mutation of the environment, which this
/// edition makes `unsafe` and a threaded test harness makes a race.
fn declined_by(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value != "0")
}

/// Whether this process was told to open no mDNS daemon.
///
/// **Read once and held.** [`watch::Watcher::poke`] builds a daemon again on every rescan and the
/// advertiser retries on a timer, so the question is asked repeatedly and one run has to answer it
/// the same way throughout for its behaviour to be describable at all.
#[must_use]
pub fn mdns_declined() -> bool {
    static DECLINED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DECLINED.get_or_init(|| declined_by(std::env::var_os(NO_MDNS_ENV_VAR).as_deref()))
}

/// The one place in this repository that opens an mDNS daemon.
///
/// `None` where there will be no mDNS, and the two reasons are deliberately one state: a network
/// with multicast blocked answers an empty list, and so does a run that declined the socket. A
/// caller that needs to tell them apart asks [`mdns_declined`].
///
/// Every caller goes through here so that a switch which is honoured in three places out of four is
/// not a switch. `mdns_sd::ServiceDaemon::new` binds UDP `0.0.0.0:5353` and `[::]:5353`, so the
/// fourth caller is the one that raises a firewall dialog nobody connected to the setting.
fn daemon() -> Option<mdns_sd::ServiceDaemon> {
    if mdns_declined() {
        tracing::debug!(variable = NO_MDNS_ENV_VAR, "not opening an mDNS daemon");
        return None;
    }
    match mdns_sd::ServiceDaemon::new() {
        Ok(daemon) => Some(daemon),
        Err(error) => {
            tracing::debug!(%error, "cannot open an mDNS daemon on this network");
            None
        }
    }
}

/// What this machine is and how to talk to it.
///
/// Public and unauthenticated, always — see the ACL's override rules. It says nothing a QR code on
/// the screen does not already say, and a client that cannot ask this cannot connect at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    /// Payload version.
    pub v: u32,
    /// This machine's instance id — stable across restarts once `km-app` persists it, so a remote
    /// can recognize the machine it talked to yesterday.
    pub id: String,
    /// What to call the machine on a list — "Living Room".
    pub name: String,
    /// The application, so a stray client that stumbled onto the port knows what it found.
    ///
    /// This and the two below are `Cow` rather than `&'static str`, which is what they were and what
    /// they still are on the sending side — every one is built from `Cow::Borrowed` around a
    /// compile-time constant, so nothing allocates to *answer* a discovery request. The reason is
    /// the receiving side: a struct holding a `&'static str` can only be deserialized from input
    /// that is itself `'static`, so `serde_json::from_str` over a response body somebody just read
    /// off a socket does not compile. It is a bound on the derived impl and not a runtime cost, so
    /// it fails at the call site with a lifetime error that says nothing about the real cause.
    /// `km-remote` reads this type, so it has to be readable.
    pub app: Cow<'static, str>,
    /// The build's version.
    pub version: Cow<'static, str>,
    /// Where the API lives.
    pub api: Cow<'static, str>,
    /// Whether this machine is still on the password it generated for itself.
    pub factory_password: bool,
    /// The port.
    pub port: u16,
    /// Reachable URLs, best first.
    pub urls: Vec<String>,
    /// Whether a remote can actually connect.
    pub reachable: bool,
    /// Why not, when it cannot.
    pub problem: Option<ConnectProblem>,
    /// How many songs are catalogd, when the catalog could be asked.
    ///
    /// A remote can show "4,812 songs" before anybody logs in, which is the difference between a
    /// machine that looks alive and one that looks broken.
    pub song_count: Option<usize>,
    /// How many times the catalog has changed, when it could be asked.
    ///
    /// For a client that keeps its own copy: this route is always public and always cheap, so
    /// "should I re-download a hundred thousand songs?" is answerable in one request that transfers
    /// nothing. Meaningless against a different machine, which is why `id` is right beside it — a
    /// mirror stores the pair.
    pub catalog_version: Option<u64>,
    /// Whether this machine will take a song sent to `POST /debug/play-upload`.
    ///
    /// **Here so that a curation tool can ask before it sends, and that is not a convenience.** The
    /// answer is off in a shipped configuration, so refusing without this means refusing *after* the
    /// upload — and a client streaming a video does not reliably get the reply at all, because a
    /// server that answers mid-request and closes leaves the sending half looking like a connection
    /// that dropped. The most useful refusal on this surface would then read as "the machine is not
    /// answering", which is both wrong and unactionable. One cheap question up front instead.
    pub debug_enabled: bool,
}

impl Discovery {
    /// Builds the payload from a resolved address.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        connect: &ConnectInfo,
        song_count: Option<usize>,
    ) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            name: name.into(),
            app: Cow::Borrowed(APP),
            version: Cow::Borrowed(env!("CARGO_PKG_VERSION")),
            api: Cow::Borrowed(API_BASE),
            factory_password: connect.factory_password,
            port: connect.port,
            urls: connect.urls.clone(),
            reachable: connect.reachable,
            problem: connect.problem.clone(),
            song_count,
            catalog_version: None,
            debug_enabled: false,
        }
    }

    /// Records whether this machine will take an uploaded song.
    ///
    /// A builder step for the same reason [`with_catalog_version`](Self::with_catalog_version)
    /// is: it is answering a different question from the rest of the payload, and it comes from the
    /// controller rather than from a resolved address.
    #[must_use]
    pub fn with_debugging(mut self, enabled: bool) -> Self {
        self.debug_enabled = enabled;
        self
    }

    /// Records whether this machine has a password, from the live source rather than a snapshot.
    ///
    /// **[`new`](Self::new) takes it from the [`ConnectInfo`] it is given, and that is no longer
    /// good enough.** A `ConnectInfo` is built when the address is resolved and refreshed when the
    /// address changes; a password can now be *set while the machine runs*, through
    /// `POST /admin/password`, and setting one does not move an address. So the snapshot could say
    /// `none` about a machine whose routes were answering 401, for the rest of that process's life.
    ///
    /// This is a builder step rather than a sixth argument for the reason
    /// [`with_catalog_version`](Self::with_catalog_version) is: it answers a different question
    /// from the rest of the payload, which describes *how to reach* the machine.
    pub fn with_factory_password(mut self, factory_password: bool) -> Self {
        self.factory_password = factory_password;
        self
    }

    /// Records how many times the catalog has changed.
    ///
    /// A builder step rather than a sixth argument to [`new`](Self::new), because it is answering a
    /// different question from everything else here — the rest of this payload describes *how to
    /// reach* the machine, and this describes what is in it. Absent means "could not ask", which is
    /// what a mirror should treat as "re-read to be sure" rather than as "nothing has changed".
    pub fn with_catalog_version(mut self, version: Option<u64>) -> Self {
        self.catalog_version = version;
        self
    }

    /// The mDNS TXT records.
    ///
    /// Kept small on purpose. A TXT record set that grows past a few hundred bytes stops fitting in
    /// one packet, and everything a client actually needs beyond this it can fetch from
    /// `/api/v1/discover` the moment it has an address. So: enough to identify and to decide
    /// whether to bother, not the whole payload.
    ///
    /// **`url` is the exception to that, and it is here because it is the one thing a browsing
    /// client cannot work out for itself.** The address to use is chosen by
    /// [`crate::connect::rank`], which demotes virtual adapters *by interface name* — and a name is
    /// exactly what does not survive the trip: an A record is a bare address. A client is left with
    /// [`crate::connect::rank_of_address`], which cannot tell a VirtualBox host-only `192.168.56.1`
    /// from the Wi-Fi card next to it. Worse, `mdns-sd` sends only the addresses on the subnet of
    /// the interface a packet leaves by, so the addresses arrive one per announcement and the first
    /// to land is not the best one. The machine already knows the answer. It should say it rather
    /// than leave every client to re-derive it from evidence that no longer carries the deciding
    /// fact.
    ///
    /// Absent on a machine with nothing reachable, which is the same thing `urls` being empty says.
    pub fn txt_records(&self) -> Vec<(String, String)> {
        // **`factory_password` is deliberately not here, and the asymmetry is the point.** In
        // `GET /discover` it answers a question a client deliberately asked, which is what lets an
        // owner's own tools nag them into changing it. In a TXT record it would be an unsolicited
        // broadcast to the whole segment announcing *this machine is still unclaimed*. Same bit,
        // very different reach.
        //
        // There is no `auth` record either: every machine has a password, so the bit would be a
        // constant in every advertisement on the network and carry no information at all.
        let mut records = vec![
            ("v".to_owned(), self.v.to_string()),
            ("id".to_owned(), self.id.clone()),
            ("name".to_owned(), self.name.clone()),
            ("api".to_owned(), self.api.clone().into_owned()),
        ];
        if let Some(url) = self.urls.first() {
            records.push(("url".to_owned(), url.clone()));
        }
        records
    }
}

/// A fresh instance id.
///
/// Random rather than derived from the hostname or a MAC address: two machines on one LAN can share
/// a hostname, and a MAC is an identifier the owner did not agree to broadcast. `km-app` persists
/// whatever this returns, so it is generated once in the machine's life.
///
/// # Panics
///
/// If the operating system will not produce entropy. A machine that cannot name itself has nothing
/// to advertise, so there is no degraded answer to return here.
pub fn new_instance_id() -> String {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).expect("the OS must be able to produce entropy for an instance id");
    let mut hex = String::with_capacity(16);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Why advertising failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AdvertError {
    /// The mDNS daemon could not start or could not register the service.
    #[error("mDNS advertisement failed: {0}")]
    Mdns(String),
    /// There is no address to advertise.
    #[error("no reachable address to advertise")]
    NoAddress,
    /// This process was told to open no mDNS daemon. See [`NO_MDNS_ENV_VAR`].
    #[error("mDNS declined by the environment")]
    Declined,
}

/// A live mDNS advertisement.
///
/// Withdrawn on drop, so a machine that shuts down cleanly stops appearing in a phone's list
/// instead of lingering until the record's TTL runs out.
pub struct Advert {
    daemon: mdns_sd::ServiceDaemon,
    fullname: String,
}

impl std::fmt::Debug for Advert {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Advert")
            .field("fullname", &self.fullname)
            .finish_non_exhaustive()
    }
}

impl Advert {
    /// Starts advertising this machine.
    ///
    /// `addresses` are the ones to publish; pass the ranked list from
    /// [`crate::connect::resolve`] so a browsing client is handed the same address the screen shows.
    pub fn start(
        discovery: &Discovery,
        addresses: &[IpAddr],
        port: u16,
    ) -> Result<Self, AdvertError> {
        if addresses.is_empty() {
            return Err(AdvertError::NoAddress);
        }
        if mdns_declined() {
            return Err(AdvertError::Declined);
        }
        let daemon =
            daemon().ok_or_else(|| AdvertError::Mdns("the daemon would not open".to_owned()))?;

        // The instance name is what a phone shows in a list, so it is the machine's name and not
        // its id. The id goes in TXT, where a client can use it to recognize the machine even after
        // somebody renames it.
        let instance = sanitise_instance_name(&discovery.name);
        let host = format!("{}.local.", sanitise_instance_name(&discovery.id));
        let properties = discovery.txt_records();

        let info = mdns_sd::ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host,
            addresses,
            port,
            &properties[..],
        )
        .map_err(|error| AdvertError::Mdns(error.to_string()))?;
        let fullname = info.get_fullname().to_owned();
        daemon
            .register(info)
            .map_err(|error| AdvertError::Mdns(error.to_string()))?;
        tracing::info!(service = %fullname, "advertising over mDNS");
        Ok(Self { daemon, fullname })
    }

    /// The service's full DNS-SD name.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

/// A machine seen advertising itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    /// The machine's name — "Living Room". What a list shows.
    pub name: String,
    /// Its instance id, from TXT. Stable across a rename, which the name is not.
    pub id: Option<String>,
    /// The base URL to talk to it on: `http://192.168.1.5:8177`.
    pub url: String,
}

/// The address to talk to a sighted machine on.
///
/// **The `url` TXT record wins when there is one**, because it is the machine's own answer, arrived
/// at with the interface names that the advertisement does not carry. See
/// [`Discovery::txt_records`] for why that matters. Failing one — a machine with no reachable
/// address — the fallback is the best-ranked announced address by
/// [`crate::connect::rank_of_address`], ties broken by octets so that repeated looks agree.
///
/// **A TXT record is shape-checked before it is trusted, and only shape-checked.** It has to be
/// `http://<IPv4>:<port>`: a hostname would send a client somewhere DNS decides, and another scheme
/// or port would let an advertisement redirect it off the machine entirely. What it deliberately
/// does *not* do is require the address to be one of the announced ones — `mdns-sd` sends only the
/// addresses on the subnet of the interface a packet leaves by, so a client legitimately never sees
/// the A record for the address the machine picked. Anybody who can write this TXT record can
/// already write an A record, so the check exists to keep that reach from widening, not to close it.
///
/// A pure function over what came off the wire, so the awkward cases are testable with no network —
/// the same split [`crate::connect::resolve_from`] makes.
pub fn sighting_url(
    txt_url: Option<&str>,
    addresses: &[std::net::Ipv4Addr],
    port: u16,
) -> Option<String> {
    if let Some(url) = txt_url {
        if let Some(rest) = url.strip_prefix("http://")
            && let Ok(socket) = rest.parse::<SocketAddr>()
            && socket.is_ipv4()
            && !socket.ip().is_loopback()
        {
            return Some(url.to_owned());
        }
        tracing::debug!(%url, "ignoring an advertised url that is not http://<IPv4>:<port>");
    }
    let best = addresses
        .iter()
        .min_by_key(|ip| (crate::connect::rank_of_address(**ip), ip.octets()))?;
    Some(format!("http://{best}:{port}"))
}

/// One `ServiceResolved` event, reduced to what a sighting is built from.
///
/// A plain struct rather than a `mdns_sd::ResolvedService` for the reason
/// [`crate::connect::Candidate`] is one: the interesting logic is the merge, and the merge should be
/// testable against the awkward announcement orders without a network to produce them.
#[derive(Debug, Clone, Default)]
pub struct Announcement {
    /// What identifies the machine across its announcements — the `id` TXT record, or the DNS-SD
    /// fullname when there is none.
    pub key: String,
    /// The DNS-SD fullname this one arrived under.
    ///
    /// Kept even when it is not the key, because a `ServiceRemoved` carries nothing else — see
    /// [`watch::Registry::removed_at`]. Throwing it away whenever an `id` is present is what makes
    /// a departure impossible to match to a row.
    pub fullname: String,
    /// What its owner calls it.
    pub name: Option<String>,
    /// Its instance id, from the `id` TXT record.
    pub id: Option<String>,
    /// The `url` TXT record, unchecked. [`sighting_url`] is what decides whether to believe it.
    pub url: Option<String>,
    /// The addresses this announcement carried — only the ones on the interface it left by.
    pub addresses: Vec<std::net::Ipv4Addr>,
    /// The port from the SRV record.
    pub port: u16,
}

impl Announcement {
    /// Reads one off the wire.
    fn from_resolved(info: &mdns_sd::ResolvedService) -> Self {
        let property = |key: &str| {
            info.get_property_val_str(key)
                .map(std::borrow::ToOwned::to_owned)
        };
        let id = property("id");
        Self {
            key: id.clone().unwrap_or_else(|| info.get_fullname().to_owned()),
            fullname: info.get_fullname().to_owned(),
            // The instance name is the machine's name; TXT carries it too, and is preferred because
            // DNS-SD escaping mangles a name with a dot or a space in it.
            name: property("name").or_else(|| {
                Some(info.get_fullname().split_once('.').map_or_else(
                    || info.get_fullname().to_owned(),
                    |(first, _)| first.to_owned(),
                ))
            }),
            id,
            url: property("url"),
            // IPv4 only, matching what `Advert` publishes and what a home network routes without
            // anybody having thought about it.
            addresses: info.get_addresses_v4().into_iter().collect(),
            port: info.get_port(),
        }
    }
}

/// The URL to talk to one resolved advertisement on.
///
/// The per-event half of [`browse`], exported so that `km_remote_core::find::Mdns` — which wants
/// *a* machine rather than a list, and so returns at the first one — reads an advertisement exactly
/// the way this does. One place knows how to read one, for the reason this module owns
/// [`SERVICE_TYPE`]: a second copy would be free to disagree.
///
/// **The `url` TXT record is what makes an early return safe.** The complete answer is in every
/// packet, so a caller stopping at the first event is not stopping at one interface's answer.
pub fn resolved_url(info: &mdns_sd::ResolvedService) -> Option<String> {
    let announcement = Announcement::from_resolved(info);
    sighting_url(
        announcement.url.as_deref(),
        &announcement.addresses,
        announcement.port,
    )
}

/// Every machine advertising itself on this network right now.
///
/// The counterpart of [`Advert`], and it lives here for the reason the service type does: one place
/// knows what a karaoke machine calls itself on a network, and a second copy of
/// `_karaokemachine._tcp.local.` in another crate is the drift this module exists to prevent.
///
/// **It waits the whole timeout and collects, where a client looking for *a* machine stops at the
/// first.** The two are different questions: `km-remote-core`'s [`Locator`](../../km_remote_core)
/// wants to start talking to something as soon as possible, and a person being shown a list wants
/// the list to be complete. So this one does not return early, and callers should expect it to take
/// the timeout it was given.
///
/// **Blocking**, because `mdns-sd` is a `recv_timeout` loop. Call it under `spawn_blocking` from an
/// async context.
///
/// **One row per machine, not one per address**, and that distinction is the whole difference
/// between a useful list and a confusing one. A Windows machine with WSL and Hyper-V on it
/// advertises three addresses — a real LAN one, plus something like `172.17.0.1` and
/// `172.28.0.1`, the last two being virtual adapters nothing outside that computer can reach.
/// Listing all three shows the same machine three times under the same name with two entries that
/// silently do not work. So machines are collapsed on the `id` TXT record, which is what identifies
/// a machine across a rename, and each keeps the one address [`sighting_url`] chooses.
///
/// **Every announcement of a machine is folded in, not just the first.** See
/// [`watch::Registry`]: the addresses arrive one packet at a time, so a machine is assembled over
/// the whole timeout rather than read off whichever interface answered first.
///
/// A machine advertising no `id` falls back to being identified by its DNS-SD
/// fullname, which is stable per service. That replaces keying on the URL, which cannot identify a
/// machine any more now that the URL is the thing being worked out.
///
/// Sorted by name so the list does not reshuffle between looks.
///
/// **The merge is [`watch::Registry`]'s and not a second copy of it.** This function is the
/// one-shot that predates the watcher and is on its way out; what it still owns is the *waiting*,
/// which is the only thing a snapshot of a live registry cannot do.
pub fn browse(timeout: std::time::Duration) -> Vec<Sighting> {
    let Some(daemon) = daemon() else {
        // No mDNS is an ordinary state, not a fault: a locked-down network, a container, a machine
        // with the service disabled, a checkout that declined the socket. An empty list says the
        // same thing to a caller.
        return Vec::new();
    };
    let Ok(receiver) = daemon.browse(SERVICE_TYPE) else {
        return Vec::new();
    };

    let mut registry = watch::Registry::new();
    let deadline = std::time::Instant::now() + timeout;
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        let Ok(event) = receiver.recv_timeout(remaining) else {
            break;
        };
        match event {
            mdns_sd::ServiceEvent::ServiceResolved(info) => {
                registry.observe_at(
                    Announcement::from_resolved(&info),
                    std::time::Instant::now(),
                );
            }
            mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                registry.removed_at(&fullname, std::time::Instant::now());
            }
            _ => {}
        }
    }
    let _ = daemon.shutdown();
    registry.sightings()
}

impl Drop for Advert {
    fn drop(&mut self) {
        // Best effort. Both calls return a receiver for the outcome and we do not wait on it: this
        // runs on the way out, and blocking a shutdown on a multicast round trip to announce our
        // own departure is not a trade worth making.
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// The longest name that can be advertised.
///
/// **63 bytes, and it is the DNS label limit rather than a preference.** A DNS-SD instance name is
/// one label, and `Advert::start` makes the name into exactly that. It matters a second time for
/// [`Discovery::txt_records`], whose test pins a 400-byte ceiling so the whole record set fits in
/// one packet — an unbounded name would blow through it, and both failures happen on the network
/// where no test would see them.
///
/// Bytes and not characters, because that is what the limit counts: "Sala de Estar" is thirteen of
/// each and "Karaokê da Sala" is not.
pub const MAX_NAME_BYTES: usize = 63;

/// Reduces a typed name to something that can be advertised, or `None` for nothing usable.
///
/// The **writer's** half of the naming rules: what `--set-name` and the API accept. Trimmed, cut to
/// [`MAX_NAME_BYTES`] on a character boundary so a multi-byte name is never cut into invalid UTF-8,
/// and `None` for a name that is only whitespace — which is a refusal worth making at the point
/// somebody types it, rather than storing an empty string and letting every client decide what to do
/// about it.
pub fn tidy_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut end = trimmed.len().min(MAX_NAME_BYTES);
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    // Trimmed again: cutting mid-name can leave a trailing space.
    let cut = trimmed[..end].trim_end();
    (!cut.is_empty()).then(|| cut.to_owned())
}

/// What to show for a name that came off the wire, or `None` when there is nothing worth showing.
///
/// The **reader's** half, and it exists because [`tidy_name`] cannot be the only rule. A machine
/// whose `settings.json` was edited by hand can advertise an empty name or a whitespace one — so every client needs one answer for that, and the answer is to fall
/// back to the address rather than draw an empty heading.
///
/// `KaraokeMachine`, the shipped default, is deliberately **not** filtered out here. It is a real
/// answer to "what is this machine called" for somebody who has one; a client listing several is the
/// one that should be showing the address as well, and it has both.
pub fn display_name(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Makes a name safe for a DNS-SD instance label.
///
/// DNS-SD instance names are generous — UTF-8 is allowed — but dots separate labels and would split
/// the name in two, and an empty label is invalid. Everything else is left alone, so "Sala de
/// Estar" stays readable on the phone.
fn sanitise_instance_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            '.' => '-',
            other if other.is_control() => '-',
            other => other,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "karaokemachine".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The advertisement's addresses, from a resolved [`ConnectInfo`].
pub fn advertisable_addresses(connect: &ConnectInfo) -> Vec<IpAddr> {
    connect
        .urls
        .iter()
        .filter_map(|url| {
            let rest = url.strip_prefix("http://")?;
            let authority = rest.split('/').next()?;
            authority.parse::<SocketAddr>().ok().map(|addr| addr.ip())
        })
        .filter(|ip| !ip.is_loopback())
        .collect()
}

/// What the advertiser should do about the addresses it has just been handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvertAction {
    /// Nothing has changed. Leave whatever is up alone.
    Keep,
    /// Take the advertisement down: there is no longer anything reachable to point it at.
    Withdraw,
    /// Take down whatever is up, and publish this name and these addresses instead.
    ///
    /// The name is carried because it is published — as the `name` TXT record and as the DNS-SD
    /// instance label — so the advertiser has to remember what it last said in order to notice a
    /// rename. See [`advert_action`].
    Publish(String, Vec<IpAddr>),
}

/// Decides what to do, given what is published and what should be.
///
/// **A pure function, separate from the task that acts on it, because this is the whole of the
/// policy.** A loop around a timer and a socket is not the part worth testing; deciding whether a
/// change of address is a change worth re-registering for is.
///
/// **The tail is compared as a set and the head is compared in place**, and that split is the
/// subtle half. [`advertisable_addresses`] preserves [`crate::connect::resolve`]'s *ranking*, so a
/// Wi-Fi interface coming up beside an Ethernet one — or simply a different answer from
/// `get_if_addrs()` — can reorder the list without changing a single address anybody could reach.
/// Re-registering for that would send a goodbye and a fresh announcement for no reason at all, and
/// on a machine whose interfaces flap it would do so every few seconds. So the set comparison
/// stays, and it is why.
///
/// **The first address is exempt from that, because it is more than an ordering.** It is what
/// [`Discovery::txt_records`] publishes as `url`, and what every browsing client uses in preference
/// to guessing. A reorder that moves a different address to the front is therefore a
/// change of published fact, and leaving it alone would strand a stale URL in the advert until
/// something else happened to change the set. The anti-flap property is kept for the tail, which is
/// where it was earned: the list's order below the first entry is not published anywhere.
///
/// **The name is compared in place too, and it is here for the same reason the first address is.**
/// It is published twice — as the `name` TXT record and as the DNS-SD instance label — so a rename
/// is a change of published fact and not a reordering. Without it, renaming a machine changes
/// `settings.json` and `/discover` immediately and leaves every phone on the network calling it by
/// the previous name until the process restarts, which is a rename that visibly did not happen.
pub fn advert_action(
    published: Option<(&str, &[IpAddr])>,
    wanted: (&str, &[IpAddr]),
) -> AdvertAction {
    let (wanted_name, wanted) = wanted;
    let republish = || AdvertAction::Publish(wanted_name.to_owned(), wanted.to_vec());
    match published {
        // Not advertising. Start, unless there is still nothing to say.
        None if wanted.is_empty() => AdvertAction::Keep,
        None => republish(),
        // Advertising, but the address has gone: the cable is out, or the lease lapsed. Better to
        // vanish from the network than to keep answering for an address nobody can reach.
        Some(_) if wanted.is_empty() => AdvertAction::Withdraw,
        Some((current_name, current)) => {
            if current_name != wanted_name || current.first() != wanted.first() {
                return republish();
            }
            let mut current: Vec<IpAddr> = current.to_vec();
            let mut next = wanted.to_vec();
            current.sort_unstable();
            next.sort_unstable();
            if current == next {
                AdvertAction::Keep
            } else {
                republish()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use crate::connect::{Candidate, DEFAULT_PORT, resolve_from};

    fn reachable_info() -> ConnectInfo {
        resolve_from(
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_PORT)),
            &[
                Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi"),
                Candidate::new(Ipv4Addr::new(10, 0, 0, 7), "Ethernet"),
            ],
            false,
        )
    }

    /// The variable's rule, read off values rather than off this process.
    ///
    /// Mutating the environment is `unsafe` in this edition and races the other tests in this
    /// binary besides, which is why [`declined_by`] takes the value.
    #[test]
    fn any_value_but_zero_declines_the_multicast_socket() {
        use std::ffi::OsStr;

        assert!(!declined_by(None), "unset asks for mDNS");
        assert!(!declined_by(Some(OsStr::new("0"))), "0 is an explicit no");
        assert!(declined_by(Some(OsStr::new("1"))));
        assert!(declined_by(Some(OsStr::new("yes"))));
        assert!(
            declined_by(Some(OsStr::new(""))),
            "set and empty is set: this is a switch rather than a value"
        );
    }

    #[test]
    fn the_payload_names_the_app_and_its_api() {
        let discovery = Discovery::new("abc123", "Living Room", &reachable_info(), Some(4812));
        let json = serde_json::to_value(&discovery).expect("serialize");
        assert_eq!(json["app"], "karaokemachine");
        assert_eq!(json["api"], "/api/v1");
        assert_eq!(json["v"], 1);
        assert_eq!(json["factory_password"], false);
        assert_eq!(json["song_count"], 4812);
        assert_eq!(json["urls"][0], "http://192.168.1.42:8177");
        assert_eq!(json["reachable"], true);
    }

    /// Pins the `Cow` on `app`, `version` and `api`.
    ///
    /// This reads a body the way a client actually gets one — off a socket, into a `String` whose
    /// lifetime is the function it was read in. With `&'static str` fields the derived impl demands
    /// `'de: 'static` and this does not *compile*, which is a failure no runtime test can catch and
    /// whose error message names a lifetime rather than the field that caused it. So the test is the
    /// call, and the assertion is almost incidental.
    #[test]
    fn a_client_can_parse_a_discovery_body_it_just_read_off_a_socket() {
        let discovery = Discovery::new("abc123", "Living Room", &reachable_info(), Some(4812));
        let body: String = serde_json::to_string(&discovery).expect("serialize");
        let parsed: Discovery = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed, discovery);
    }

    #[test]
    fn an_unreachable_machine_still_answers_and_says_why() {
        let info = resolve_from(
            SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)),
            &[],
            false,
        );
        let discovery = Discovery::new("abc123", "Living Room", &info, None);
        let json = serde_json::to_value(&discovery).expect("serialize");
        assert_eq!(json["reachable"], false);
        assert_eq!(json["problem"]["kind"], "loopback_only");
        assert!(json["song_count"].is_null());
    }

    #[test]
    fn txt_records_stay_small_enough_for_one_packet() {
        let discovery = Discovery::new("abc123", "Living Room", &reachable_info(), Some(4812));
        let records = discovery.txt_records();
        let keys: Vec<&str> = records.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["v", "id", "name", "api", "url"]);
        let total: usize = records
            .iter()
            .map(|(key, value)| key.len() + value.len() + 2)
            .sum();
        assert!(total < 400, "TXT records grew to {total} bytes");
    }

    /// The record this whole change exists for: the machine states the address *it* chose.
    #[test]
    fn the_advert_names_the_address_the_machine_chose() {
        let discovery = Discovery::new("abc123", "Living Room", &reachable_info(), None);
        let url = discovery
            .txt_records()
            .into_iter()
            .find(|(key, _)| key == "url")
            .expect("url is present");
        // Not `10.0.0.7`, which is the other address it advertises and which a client ranking on
        // numbers alone would have had no way to reject had the Wi-Fi been on a later range.
        assert_eq!(url.1, "http://192.168.1.42:8177");
        assert_eq!(url.1, discovery.urls[0]);
    }

    #[test]
    fn a_machine_with_nothing_reachable_advertises_no_url() {
        let info = resolve_from(
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_PORT)),
            &[],
            false,
        );
        let discovery = Discovery::new("abc123", "Living Room", &info, None);
        assert!(!discovery.txt_records().iter().any(|(key, _)| key == "url"));
    }

    /// **The one bit that must not be broadcast.** `/discover` answers a question a client asked;
    /// a TXT record is shouted at the whole segment, and "this machine is still unclaimed" is not a
    /// thing to shout. A regression here would be silent and would look like a feature.
    #[test]
    fn a_factory_password_is_reported_over_http_and_never_advertised() {
        let mut info = reachable_info();
        info.factory_password = true;
        let discovery = Discovery::new("abc123", "Living Room", &info, None);

        let json = serde_json::to_value(&discovery).expect("serialize");
        assert_eq!(json["factory_password"], true);

        let records = discovery.txt_records();
        assert!(
            !records
                .iter()
                .any(|(key, _)| key == "factory_password" || key == "auth"),
            "the factory-password flag must not reach the mDNS advertisement: {records:?}"
        );
        // And nothing else leaked it in a value, either.
        assert!(
            !records.iter().any(|(_, value)| value.contains("password")),
            "no TXT value may mention a password: {records:?}"
        );
    }

    #[test]
    fn instance_ids_are_random_and_hex() {
        let first = new_instance_id();
        let second = new_instance_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 16);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_name_with_dots_does_not_split_into_dns_labels() {
        assert_eq!(sanitise_instance_name("Sala.de.Estar"), "Sala-de-Estar");
    }

    #[test]
    fn accented_names_survive_because_dns_sd_allows_them() {
        assert_eq!(sanitise_instance_name("Karaokê da Sala"), "Karaokê da Sala");
    }

    #[test]
    fn an_empty_name_falls_back_to_something_findable() {
        assert_eq!(sanitise_instance_name("   "), "karaokemachine");
        assert_eq!(sanitise_instance_name(""), "karaokemachine");
    }

    #[test]
    fn a_typed_name_is_trimmed_and_a_blank_one_is_refused() {
        assert_eq!(tidy_name("  Living Room  ").as_deref(), Some("Living Room"));
        // Refused at the point somebody types it, rather than stored empty for every client to
        // decide about separately.
        assert_eq!(tidy_name("   "), None);
        assert_eq!(tidy_name(""), None);
    }

    #[test]
    fn a_long_name_is_cut_to_the_dns_label_limit() {
        let long = "a".repeat(200);
        let tidied = tidy_name(&long).expect("a name of letters is usable");
        assert_eq!(tidied.len(), MAX_NAME_BYTES);
    }

    #[test]
    fn cutting_a_long_name_never_splits_a_character() {
        // 63 is not a multiple of 2, so a string of two-byte characters lands mid-character at the
        // limit -- which is the whole reason the cut walks back to a boundary. Slicing there would
        // panic rather than produce a short name, so this is a crash test as much as a length one.
        let long = "ê".repeat(100);
        let tidied = tidy_name(&long).expect("usable");
        assert!(tidied.len() <= MAX_NAME_BYTES);
        assert!(tidied.chars().all(|c| c == 'ê'));
        // ...and the whole record set still fits in one packet, which is the second limit this
        // serves and the one no unit test would otherwise reach.
        let discovery = Discovery::new("abc123", &tidied, &reachable_info(), Some(4812));
        let bytes: usize = discovery
            .txt_records()
            .iter()
            .map(|(key, value)| key.len() + value.len() + 2)
            .sum();
        assert!(bytes < 400, "{bytes} bytes of TXT records");
    }

    #[test]
    fn a_machine_that_advertises_no_name_is_shown_by_its_address_instead() {
        // The reader's half. `tidy_name` refuses a blank name on the way in, but a hand-edited
        // settings file can still put one on the wire, and every client needs one
        // answer for that rather than an empty heading each.
        assert_eq!(display_name(""), None);
        assert_eq!(display_name("   "), None);
        assert_eq!(display_name(" Living Room "), Some("Living Room"));
        // The shipped default is a real answer and is deliberately not filtered out here.
        assert_eq!(display_name("KaraokeMachine"), Some("KaraokeMachine"));
    }

    /// The four transitions, and they are the whole of the advertiser's policy.
    #[test]
    fn the_advertiser_publishes_withdraws_and_otherwise_leaves_well_alone() {
        let one: IpAddr = Ipv4Addr::new(192, 168, 1, 5).into();
        let two: IpAddr = Ipv4Addr::new(10, 0, 0, 9).into();
        let name = "Living Room";

        // Nothing advertised and nothing to advertise: the state of a machine whose network has not
        // come up yet, which must not be mistaken for something to do.
        assert_eq!(advert_action(None, (name, &[])), AdvertAction::Keep);
        // ...and the moment it does come up. This is the transition the appliance never made.
        assert_eq!(
            advert_action(None, (name, &[one])),
            AdvertAction::Publish(name.to_owned(), vec![one])
        );
        // Steady state, which is almost every tick for the life of the machine.
        assert_eq!(
            advert_action(Some((name, &[one])), (name, &[one])),
            AdvertAction::Keep
        );
        // A lease that moved the machine. Re-registering is the point.
        assert_eq!(
            advert_action(Some((name, &[one])), (name, &[two])),
            AdvertAction::Publish(name.to_owned(), vec![two])
        );
        // The cable came out.
        assert_eq!(
            advert_action(Some((name, &[one])), (name, &[])),
            AdvertAction::Withdraw
        );
    }

    /// A rename is a change worth re-registering for, and nothing else about the machine moved.
    ///
    /// The bug this is against is the one that makes a rename look like it did nothing: the name is
    /// published twice — as the `name` TXT record and as the DNS-SD instance label — so comparing
    /// addresses alone answered `Keep`, and every phone on the network went on showing the old name
    /// until the process restarted. Meanwhile `settings.json` and `/discover` had both changed, so
    /// the two halves of the machine disagreed with each other.
    #[test]
    fn renaming_the_machine_republishes_even_though_the_addresses_did_not_move() {
        let one: IpAddr = Ipv4Addr::new(192, 168, 1, 5).into();
        assert_eq!(
            advert_action(Some(("Living Room", &[one])), ("Kitchen", &[one])),
            AdvertAction::Publish("Kitchen".to_owned(), vec![one])
        );
        // And a machine with nothing to advertise is still not something to do: a rename on a box
        // whose network has not come up must not start an advert for an address nobody can reach.
        assert_eq!(advert_action(None, ("Kitchen", &[])), AdvertAction::Keep);
    }

    /// Reordering the *tail* is not a change. Reordering the head is.
    ///
    /// `advertisable_addresses` preserves `resolve`'s ranking, so a second interface appearing can
    /// reorder the list without changing what anybody can reach. Re-registering for that would send
    /// a goodbye and a fresh announcement for nothing — which is why the comparison is by set. The
    /// first entry is the exception, because it is published as the `url` TXT record and a stale
    /// one sends every browsing client to the wrong address.
    #[test]
    fn reordering_the_tail_is_not_a_change_but_moving_the_head_is() {
        let one: IpAddr = Ipv4Addr::new(192, 168, 1, 5).into();
        let two: IpAddr = Ipv4Addr::new(10, 0, 0, 9).into();
        let three: IpAddr = Ipv4Addr::new(172, 17, 0, 1).into();

        let name = "Living Room";

        // Same head, tail shuffled: nothing anybody can see changed.
        assert_eq!(
            advert_action(Some((name, &[one, two, three])), (name, &[one, three, two])),
            AdvertAction::Keep
        );
        // The same addresses, but a different one is now preferred. The advert says so.
        assert_eq!(
            advert_action(Some((name, &[one, two])), (name, &[two, one])),
            AdvertAction::Publish(name.to_owned(), vec![two, one])
        );
        // ...and losing one of them is a change however the rest are ordered.
        assert_eq!(
            advert_action(Some((name, &[one, two])), (name, &[one])),
            AdvertAction::Publish(name.to_owned(), vec![one])
        );
    }

    #[test]
    fn advertisable_addresses_come_from_the_resolved_urls() {
        let addresses = advertisable_addresses(&reachable_info());
        assert_eq!(
            addresses,
            [
                IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)),
            ]
        );
    }

    #[test]
    fn a_loopback_only_machine_advertises_nothing() {
        let info = resolve_from(
            SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)),
            &[],
            false,
        );
        assert!(advertisable_addresses(&info).is_empty());
        let discovery = Discovery::new("abc", "Living Room", &info, None);
        // ...and refusing is better than registering a service pointing at 127.0.0.1, which every
        // phone that found it would fail to reach.
        assert!(matches!(
            Advert::start(&discovery, &[], DEFAULT_PORT),
            Err(AdvertError::NoAddress)
        ));
    }

    #[test]
    fn the_service_type_is_the_one_the_plan_names() {
        assert_eq!(SERVICE_TYPE, "_karaokemachine._tcp.local.");
    }

    // -----------------------------------------------------------------------------------------
    // Reading an advertisement.
    // -----------------------------------------------------------------------------------------

    /// The case a client cannot get right on its own, and the reason `url` is advertised at all.
    #[test]
    fn the_advertised_url_beats_an_address_that_only_looks_better() {
        // A VirtualBox host-only adapter and a Wi-Fi card on a phone hotspot. On numbers alone the
        // VirtualBox one wins outright — 192.168/16 ranks above 172.16/12 — and this is the same
        // pair `connect::rank`'s own
        // `a_real_interface_in_a_late_range_still_beats_a_virtual_one_in_an_early_range` gets right
        // by reading the interface name, which is exactly the fact an A record does not carry.
        let addresses = [
            Ipv4Addr::new(192, 168, 56, 1),
            Ipv4Addr::new(172, 20, 10, 3),
        ];
        assert_eq!(
            sighting_url(Some("http://172.20.10.3:8177"), &addresses, 8177),
            Some("http://172.20.10.3:8177".to_owned())
        );
        // Without the record there is nothing to go on but the numbers, and the numbers are wrong.
        // The wrong answer is asserted deliberately: it is the whole cost of a machine that does
        // not advertise one.
        assert_eq!(
            sighting_url(None, &addresses, 8177),
            Some("http://192.168.56.1:8177".to_owned())
        );
    }

    #[test]
    fn without_an_advertised_url_the_best_ranked_address_is_used() {
        let addresses = [
            Ipv4Addr::new(172, 17, 0, 1),
            Ipv4Addr::new(192, 168, 1, 42),
            Ipv4Addr::new(10, 0, 0, 7),
        ];
        assert_eq!(
            sighting_url(None, &addresses, 8177),
            Some("http://192.168.1.42:8177".to_owned())
        );
        // Nothing at all to say is not an answer, and a caller must not invent one.
        assert_eq!(sighting_url(None, &[], 8177), None);
    }

    /// A TXT record is written by whoever is on the network, so it is shape-checked before it is
    /// believed. Every one of these falls back to the addresses rather than being followed.
    #[test]
    fn an_advertised_url_that_is_not_a_plain_http_address_is_not_followed() {
        let addresses = [Ipv4Addr::new(192, 168, 1, 42)];
        let fallback = Some("http://192.168.1.42:8177".to_owned());
        for bad in [
            "https://evil.example.com/",
            // A hostname would send the client wherever DNS decides.
            "http://evil.example.com:8177",
            // No port: not a socket address, and the port is not ours to guess.
            "http://192.168.1.9",
            // Loopback would point every client at itself.
            "http://127.0.0.1:8177",
            "http://[::1]:8177",
            "",
            "not a url at all",
        ] {
            assert_eq!(
                sighting_url(Some(bad), &addresses, 8177),
                fallback,
                "{bad} should not have been followed"
            );
        }
    }
}
