//! Finding a karaoke machine, and remembering where it was.
//!
//! Three ways, in order of how much anybody has to know: an address somebody named, the address it
//! was found at last time, and the network. The last is what makes the common case need nothing
//! typed at all.
//!
//! **Not finding one is not fatal**, and that rule is the whole point of the offline app. If a mirror
//! exists, the remote starts anyway: browsing, searching and favorites all work, the
//! banner says the machine is away, and the background loop keeps trying. Only with no mirror is a
//! machine genuinely required — it is the only source of one — and then the page says so rather than
//! showing an empty list.
//!
//! **Looking on the network is a [`Locator`] rather than a call to mDNS**, and that is the seam this
//! whole crate exists for. On Android an mDNS browse sees nothing at all unless Java is holding a
//! `WifiManager.MulticastLock` for its duration; on iOS it needs `NSLocalNetworkUsageDescription` in
//! the bundle and a permission prompt that only the application can raise. Both are facts about the
//! *host* rather than about the platform, so a `#[cfg(target_os = …)]` could not express either even
//! where it guessed the platform right — the same Android build wants a real browse when it holds
//! the lock and [`NoLocator`] when it does not.

use std::path::{Path, PathBuf};
use std::time::Duration;

use km_api::discover::Sighting;
use km_api::discover::known::{self, Known, Why};
use km_api::discover::watch::{Observed, Watcher};

pub use km_api::discover::known::normalize;

/// One way of looking for a karaoke machine on the network.
///
/// **Not named `Discovery`**, deliberately: `km_api::discover::Discovery` already exists and is the
/// DTO [`crate::client`] parses out of `GET /api/v1/discover`. Two types called the same thing in
/// one crate's imports is a confusion that costs a reader more than the longer name does.
///
/// **Blocking rather than `async`**, also deliberately. `mdns-sd` is a `recv_timeout` loop and a JNI
/// implementation has to attach a thread anyway, so an `async fn` here would be a wrapper around a
/// blocking call wearing an async hat. [`crate::Server::open`] calls this under
/// `tokio::task::spawn_blocking`, which is a fix as much as an accommodation: the binary this crate
/// came out of called it straight from `#[tokio::main]`, so a browse for a television that is
/// switched off stalled the whole runtime for a second and a half.
pub trait Locator: Send + Sync + 'static {
    /// One look. An empty list is an ordinary answer and never an error — a locked-down network, a
    /// container, a platform with no mDNS, or simply nothing switched on.
    ///
    /// **A list of [`Sighting`]s and not the first URL.** It used to answer `Option<String>`, which
    /// threw away everything an advertisement carries except where to connect — including the
    /// instance id, which is the only thing about a machine that survives it changing address. That
    /// is what `km_api::discover::known::choose` needs and what a bare URL could never supply.
    ///
    /// `The card says which machine, in the owner's words` refused this widening and its reasons
    /// still hold, for the **name**: a browse-time name is a snapshot of a thing that can be
    /// renamed, and carrying one to the shells would cost a seventh C function. An id cannot go
    /// stale, and it stops in this crate — no shell reads one, and the FFI is still six functions.
    fn look(&self) -> Vec<Sighting>;

    /// A registry that fills itself in, for a locator that can genuinely listen.
    ///
    /// `None` is not a failure: it means *ask me again*, and [`Radar`](crate::find::Radar) polls
    /// [`look`](Self::look) instead. [`Sweep`] answers `None` because a unicast walk is a
    /// photograph rather than a subscription, and [`NoLocator`] because it has nothing to say.
    ///
    /// **The registry lives inside the locator that can listen, rather than above the trait.** A
    /// registry above it would make push and poll look alike to a caller while being nothing alike
    /// underneath: a sweep's result folded into one would carry a `last_seen` meaning *the last time
    /// I walked a thousand addresses*, and a subscriber woken by it would be woken by a poll it paid
    /// for itself.
    fn watch(&self) -> Option<Watcher> {
        None
    }

    /// Look for one machine in particular from now on, rather than for any machine.
    ///
    /// **A trait method with a do-nothing default, and not a downcast.** Only [`Sweep`] can act on
    /// it — mDNS hears every machine on the segment whether it wants to or not, so filtering there
    /// would be a filter the caller can apply itself — but a `dyn Locator` cannot be asked whether
    /// it is a `Sweep` without making the whole trait `Any`, and a default is one line against that.
    fn hunt(&self, _id: Option<&str>) {}

    /// Whether looking is a thing this locator can do at all.
    ///
    /// **Not the same question as whether a look succeeded.** [`NoLocator`] answers nothing every
    /// time by construction, and a page that offered a *Rescan* button over one would be offering an
    /// action that can only ever report failure — which is the grayed-out control
    /// `km_remote_pages::Capabilities` exists to avoid. Defaulted to `true`, so a real locator says
    /// nothing and the one that finds nothing says so once.
    fn can_browse(&self) -> bool {
        true
    }
}

/// Never finds anything.
///
/// What a host with no permission to browse passes: an Android build before the `MulticastLock` is
/// wired up, an iOS build before the usage description is in its bundle. Also what the tests use,
/// so that a suite running on somebody's home network cannot be told a different story by whatever
/// happens to be switched on in the next room.
pub struct NoLocator;

impl Locator for NoLocator {
    fn look(&self) -> Vec<Sighting> {
        Vec::new()
    }

    fn can_browse(&self) -> bool {
        false
    }
}

/// mDNS for `_karaokemachine._tcp.local.` — what a desktop uses, and the default.
///
/// **It owns one [`Watcher`] and opens it lazily.** One, because a second daemon would be a second
/// multicast socket answering the same question; lazily, because `Config::new` builds one of these
/// before a caller has had the chance to substitute [`NoLocator`], and a daemon opened there would
/// be a daemon opened in every test — the fault `CONTRIBUTING.md`'s
/// *No test binds a non-loopback address* is about.
#[cfg(feature = "mdns")]
#[derive(Default)]
pub struct Mdns {
    watcher: std::sync::OnceLock<Watcher>,
}

#[cfg(feature = "mdns")]
impl Mdns {
    /// One that has not opened its daemon yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The watcher, opened on the first call and shared from then on.
    fn watcher(&self) -> &Watcher {
        self.watcher.get_or_init(Watcher::start)
    }
}

#[cfg(feature = "mdns")]
impl Locator for Mdns {
    /// Whatever the registry holds, waiting only until it holds something.
    ///
    /// **The wait ends at the first machine, not at the timeout**, and getting that wrong cost a
    /// twelvefold slowdown on the one call a person actually waits for. A cold start has an empty
    /// registry by definition — the watcher was opened moments earlier — so sleeping the whole
    /// `timeout` made every first launch pay 1.5 s where the one-shot this replaced returned at the
    /// first announcement in about 130 ms. Measured, before and after, on a real network:
    /// **131 ms against 1575 ms**. The `timeout` is the giving-up point; it was never meant to be
    /// the answer's cost.
    ///
    /// **Returning at the first machine is safe for `resolved_url`'s reason.** An announcement
    /// carries only the addresses on the interface it left by, but it carries the whole TXT set
    /// every time — so the machine's own `url` record, which is what decides the address, is in the
    /// first packet as much as the last. That is the same argument the one-shot's early return
    /// rested on, and it is unchanged.
    ///
    /// **A caller wanting a *complete* list meets a warm registry anyway.** The only moment this is
    /// cold is the first seconds of a process, and by the time somebody presses Rescan the watcher
    /// has been listening throughout. A poll rather than a wait on
    /// [`Watcher::changed`](km_api::discover::watch::Watcher::changed), because that is an `async`
    /// signal and this is a blocking call by design.
    fn look(&self) -> Vec<Sighting> {
        let watcher = self.watcher();
        let snapshot = watcher.snapshot();
        if !snapshot.is_empty() {
            return snapshot;
        }
        watcher.poke();

        // `BROWSE_TIMEOUT` named here rather than taken as a parameter: it is sized for mDNS, this is
        // the mDNS implementation, and every caller passed exactly this constant anyway.
        let deadline = std::time::Instant::now() + BROWSE_TIMEOUT;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(LOOK_POLL);
            let snapshot = watcher.snapshot();
            if !snapshot.is_empty() {
                return snapshot;
            }
        }
        Vec::new()
    }

    fn watch(&self) -> Option<Watcher> {
        Some(self.watcher().clone())
    }

    /// False where the daemon was declined, for the reason the trait's default gives.
    ///
    /// A checkout that sets `KM_NO_MDNS` has an `Mdns` that behaves like a [`NoLocator`], and a
    /// page offering *Rescan* over one offers an action that can only ever report nothing.
    fn can_browse(&self) -> bool {
        !km_api::discover::mdns_declined()
    }
}

// ---------------------------------------------------------------------------------------------
// Finding a machine without multicast.
// ---------------------------------------------------------------------------------------------

/// The port a sweep knocks on.
///
/// **A sweep cannot learn a port the way mDNS does** — an SRV record carries one and a subnet does
/// not — so this is an assumption where [`Mdns`] has a fact. It is not a *new* assumption:
/// [`normalize`] below has always turned a bare `192.168.1.5` into `http://192.168.1.5:8177`.
///
/// Spelled as `km_api::connect::DEFAULT_PORT` and never as a literal, because
/// [`crate::DEFAULT_PORT`] is **8179** — the port *this remote's own* server binds — and the two
/// sit one import apart. A sweep built on the wrong one searches the network for other remotes,
/// finds nothing, and looks exactly like a network with no machine on it.
#[cfg(any(feature = "sweep", test))]
const SWEEP_PORT: u16 = km_api::connect::DEFAULT_PORT;

/// The widest network worth walking, as a count of usable hosts. A /20.
///
/// A /21 is 2,046 addresses and a /16 is 65,534. The ceiling is a judgment about *home* networks:
/// anything wider is an office, where a machine is found by being named rather than by being
/// hunted for. See [`reach`].
pub const MAX_SWEEP_HOSTS: u32 = 4094;

/// How long a sweep gets.
///
/// **Four times [`BROWSE_TIMEOUT`], and that difference is the point.** 1,500 ms is sized for mDNS,
/// where an answer either arrives in the first few hundred milliseconds or is not coming; a sweep
/// cut off there would walk a fraction of a /22 and report "no machine" about a machine that is
/// switched on. So `Sweep` holds itself to this rather than to the shared constant, which would
/// otherwise have to be raised and slow every desktop start for a case only one platform has.
pub const SWEEP_BUDGET: Duration = Duration::from_secs(6);

/// How many probes may be outstanding at once.
///
/// Two reasons, and the second has no counterpart in the UDP sweep this is modeled on. A thousand
/// packets sent back to back looks like a port scan to a consumer access point, and the one it
/// decides to drop is the machine. **And every outstanding TCP connect is a file descriptor**, held
/// for the whole of [`SWEEP_CONNECT_TIMEOUT`] against an address with nothing at it — unbounded,
/// a /22 would exhaust the descriptor table of the process that is *also* running the server the
/// WebView is reading.
#[cfg(any(feature = "sweep", test))]
const SWEEP_IN_FLIGHT: usize = 64;

/// A pause held with the permit, so the packet rate is bounded rather than just the concurrency.
#[cfg(any(feature = "sweep", test))]
const SWEEP_PACE: Duration = Duration::from_millis(20);

/// How long to wait for a TCP handshake.
///
/// **The one that actually matters.** On a LAN a live host answers in single-digit milliseconds and
/// a dead address is silence, so this is what decides how long the sweep spends on the 1,021
/// addresses that are not the machine.
#[cfg(any(feature = "sweep", test))]
const SWEEP_CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

/// How long to wait for the whole request, once something has answered.
#[cfg(any(feature = "sweep", test))]
const SWEEP_PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// Whether a network is one to walk, and why not when it is not.
///
/// **Pure, and separate from the walk that consults it**, on the same reasoning [`RECOVERY_INTERVAL`] is
/// separate from the loop that acts on it: this is the whole of the policy, and testing it needs
/// neither an interface nor a network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Walk it. This many hosts are usable on it, this one included.
    Walk(u32),
    /// A /31 or a /32: there is no other host to find.
    Alone,
    /// An all-zero mask, which some VPN adapters report. It nominally covers the internet.
    Unbounded,
    /// Wider than [`MAX_SWEEP_HOSTS`].
    TooWide(u32),
}

/// Whether a prefix length describes a network a sweep should walk.
#[must_use]
pub fn reach(prefix: u8) -> Reach {
    if prefix == 0 {
        return Reach::Unbounded;
    }
    if prefix >= 31 {
        return Reach::Alone;
    }
    // `prefix` is 1..=30 here, so the shift cannot overflow and the subtraction cannot underflow.
    let hosts = (1_u32 << (32 - prefix)) - 2;
    if hosts > MAX_SWEEP_HOSTS {
        Reach::TooWide(hosts)
    } else {
        Reach::Walk(hosts)
    }
}

/// One of this host's IPv4 networks, reduced to what a sweep needs to judge it.
///
/// **A plain struct rather than an `if_addrs::Interface`**, which is the same bargain
/// `km_api::connect::Candidate` already makes and for the same reason: the configurations that
/// matter — a VPN tunnel, a macOS virtual-machine bridge, an Ethernet port that is up with nothing
/// plugged into it — are ones no CI machine will ever have, and they have to be testable anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Net {
    /// This host's address on the network.
    pub ip: std::net::Ipv4Addr,
    /// The prefix length. `if-addrs` computes this, so nothing here counts mask bits.
    pub prefix: u8,
    /// The interface's name, which is what says whether it is a real adapter.
    pub interface: String,
    /// Whether the interface is up.
    pub up: bool,
    /// Whether it is point-to-point — a tunnel, with nobody else on it.
    pub p2p: bool,
}

/// The addresses to try on one network, ordered outward from `self_ip`.
///
/// **Alternating up and down, nearest first**, because a machine on a home network is usually a
/// handful of addresses from the phone — which is what turns a 1,022-host walk into an answer in
/// tens of milliseconds rather than a walk. The network address, the broadcast address and
/// `self_ip` are skipped. Empty when [`reach`] refuses.
///
/// The bounds are checked rather than assumed, and that is not defensive style: the Go sweep this
/// follows guards its `selfN - step` with an explicit comparison because its arithmetic is
/// `uint32`, and the direct translation of it panics in a debug build and wraps in a release one
/// the moment the host is the first usable address on its subnet.
#[must_use]
pub fn sweep_order(self_ip: std::net::Ipv4Addr, prefix: u8) -> Vec<std::net::Ipv4Addr> {
    let Reach::Walk(hosts) = reach(prefix) else {
        return Vec::new();
    };
    let self_n = u32::from(self_ip);
    let mask = u32::MAX << (32 - prefix);
    let network = self_n & mask;
    let broadcast = network | !mask;

    let mut out = Vec::with_capacity(hosts as usize);
    let mut step = 1_u32;
    loop {
        let up = self_n.checked_add(step).filter(|n| *n < broadcast);
        let down = self_n.checked_sub(step).filter(|n| *n > network);
        if up.is_none() && down.is_none() {
            return out;
        }
        out.extend(up.map(std::net::Ipv4Addr::from));
        out.extend(down.map(std::net::Ipv4Addr::from));
        step += 1;
    }
}

/// Which of this host's networks to sweep, best first, with the unusable ones dropped.
///
/// Dropped: interfaces that are down, loopback, point-to-point tunnels, link-local `169.254/16`
/// (which means DHCP failed and nothing else is reachable through it), and anything [`reach`]
/// refuses.
///
/// **The ordering is `km_api::connect`'s and not a second copy of it.** That module's
/// `looks_virtual` already knows what a Hyper-V switch, a Docker bridge, `utun`, Tailscale and
/// ZeroTier are called, and its `rank_of_address` already knows that a home router hands out
/// `192.168/16` far more often than anything else. Its own doc says why it was split out — so that
/// a second caller could not "disagree about which private range Hyper-V is on" — and this is that
/// second caller taking it up on the offer.
#[must_use]
pub fn sweep_networks(nets: &[Net]) -> Vec<Net> {
    let mut usable: Vec<Net> = nets
        .iter()
        .filter(|net| {
            net.up
                && !net.p2p
                && !net.ip.is_loopback()
                && !net.ip.is_link_local()
                && matches!(reach(net.prefix), Reach::Walk(_))
        })
        .cloned()
        .collect();
    // Ties broken by address, on the same reasoning `km_api::connect::rank` gives for doing it:
    // two adapters can be equally good by both measures, and a sweep that reordered itself between
    // runs would make "it found it quickly last time" impossible to reason about.
    usable.sort_by_key(|net| {
        (
            km_api::connect::looks_virtual(&net.interface),
            km_api::connect::rank_of_address(net.ip),
            net.ip.octets(),
        )
    });
    usable
}

/// Every address worth probing, across every network, nearest first and without repeats.
///
/// Two interfaces on one subnet is an ordinary thing — a laptop docked and on Wi-Fi at once — and
/// probing each address twice would double the packets for nothing.
#[must_use]
pub fn sweep_targets(nets: &[Net]) -> Vec<std::net::Ipv4Addr> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for net in sweep_networks(nets) {
        for ip in sweep_order(net.ip, net.prefix) {
            if seen.insert(ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// Probes `targets` with bounded concurrency and returns the first that answers.
///
/// **Generic over the probe so that this function is testable without a network**, which is where
/// the value is: the ordering promise and the early stop are the two things a returned address
/// cannot demonstrate on its own. The real caller passes an HTTP probe; a test passes a closure
/// that counts.
///
/// The remaining tasks are canceled by dropping the [`tokio::task::JoinSet`] on the way out, which
/// closes their sockets.
/// **Generic over what the probe *yields*, not merely over the probe.** It used to answer `bool`
/// and hand back the address that said so, which threw away the discovery document the real probe
/// had already parsed — the machine's id and name among it. Answering `Option<T>` costs one type
/// parameter and no logic, and is what lets a sweep return a [`Sighting`] as complete as an mDNS
/// announcement.
#[cfg(any(feature = "sweep", test))]
async fn race<F, Fut, T>(targets: Vec<std::net::Ipv4Addr>, budget: Duration, probe: F) -> Option<T>
where
    F: Fn(std::net::Ipv4Addr) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Option<T>> + Send + 'static,
    T: Send + 'static,
{
    // Fair, so permits are handed out in the order the tasks asked for them and the ordering
    // `sweep_targets` promised survives into the packets.
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(SWEEP_IN_FLIGHT));
    let mut set = tokio::task::JoinSet::new();
    for ip in targets {
        let probe = probe.clone();
        let permits = std::sync::Arc::clone(&permits);
        set.spawn(async move {
            let _permit = permits.acquire_owned().await.ok()?;
            let found = probe(ip).await;
            // Held with the permit rather than between batches: 64 in flight and a pause before
            // each is released is the same packet rate, without a batch boundary to line up on.
            tokio::time::sleep(SWEEP_PACE).await;
            found
        });
    }

    let deadline = tokio::time::Instant::now() + budget;
    loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => return None,
            joined = set.join_next() => match joined {
                None => return None,
                Some(Ok(Some(found))) => return Some(found),
                Some(_) => {}
            },
        }
    }
}

/// A unicast sweep of the local subnet — what a host uses when it may not multicast.
///
/// # Why this exists beside [`Mdns`]
///
/// iOS has required `com.apple.developer.networking.multicast` for multicast and broadcast since
/// version 14, and Apple grants it only after a manually reviewed request. Without it an mDNS
/// browse does not fail: it **finds nothing and reports success**, forever, which is the worst
/// shape a fault can have. Ordinary unicast to a LAN address needs nothing beyond the local-network
/// permission prompt every app gets, so this asks each address in turn instead of asking the
/// network as a whole.
///
/// # What it costs against mDNS, said plainly
///
/// **A machine moved off [`SWEEP_PORT`] is invisible to it.** An SRV record carries a port and a
/// subnet does not. The answer for that machine is to type its address, which is why a host that
/// offers this must also offer somewhere to type one.
///
/// # What counts as an answer
///
/// A 200 whose body parses as a `km_api::discover::Discovery` **naming this application**. Not an
/// open port, and not merely valid JSON: a home network has routers, printers and set-top boxes on
/// it, and the cost of believing one of them is not a retry but a memory — `Server::machine_watch`
/// writes an address down as soon as it reports online, and [`locate`] then prefers what was
/// remembered over looking again. A false positive is a remote that opens on a printer every
/// morning.
#[cfg(feature = "sweep")]
pub struct Sweep {
    budget: Duration,
    wanted: std::sync::RwLock<Option<String>>,
}

#[cfg(feature = "sweep")]
impl Default for Sweep {
    fn default() -> Self {
        Self {
            budget: SWEEP_BUDGET,
            wanted: std::sync::RwLock::new(None),
        }
    }
}

#[cfg(feature = "sweep")]
impl Sweep {
    /// A sweep with the default budget.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A sweep given longer, or shorter, than [`SWEEP_BUDGET`].
    #[must_use]
    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    /// Hunt one machine by its instance id, rather than taking the first one that answers.
    ///
    /// **This is the iOS half of identity-first resolution**, and it is a real change of behavior
    /// rather than a filter. A sweep meets whatever is on the subnet, so in a house with two
    /// machines it used to adopt whichever answered first — a coin toss dressed as a decision.
    /// Given the id somebody is actually connected to, it walks past the other one.
    ///
    /// The cost is that hunting a machine which is switched off spends the whole [`SWEEP_BUDGET`]
    /// rather than stopping at the first karaoke machine it meets. That is bounded at six seconds,
    /// happens only while the remote is already offline, and is worth strictly more than the coin
    /// toss it replaces.
    ///
    /// Interior mutability because the wanted machine changes while the process runs — a `Locator`
    /// is behind an `Arc` and shared, and the id arrives from `/discover` long after construction.
    fn want(&self, id: Option<&str>) {
        if let Ok(mut wanted) = self.wanted.write() {
            *wanted = id.map(std::borrow::ToOwned::to_owned);
        }
    }
}

#[cfg(feature = "sweep")]
impl Locator for Sweep {
    /// One sweep of every network this host is on.
    ///
    /// **This implementation owns its budget**, where the trait used to hand one in. It was always
    /// [`BROWSE_TIMEOUT`], sized for mDNS, and a sweep held to that would walk a tenth of a /22 and
    /// call the result "nothing found" -- so this took the larger of the two and the parameter was
    /// advisory here and ignored entirely by `NoLocator`. A number that one of three implementations
    /// refuses and no caller ever varies is worse than no number: a reader has to discover that what
    /// they passed does not apply. The cost is bounded on both sides -- [`known()`] short-circuits
    /// every launch after the first, and the four phases have the pages up while this runs.
    fn look(&self) -> Vec<Sighting> {
        let budget = self.budget;

        // `try_current` rather than `current`: a `Sweep` built by a host with no runtime, or in a
        // test, should answer "nothing found" rather than panicking. Blocking here is correct —
        // `Server::open` calls a locator inside `spawn_blocking`, and a blocking-pool thread holds
        // the runtime's handle without being marked as an async context.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("no runtime on this thread; a sweep cannot run from here");
            return Vec::new();
        };

        let nets = interfaces();
        let targets = sweep_targets(&nets);
        if targets.is_empty() {
            // Every ordinary reason lands here: no usable interface, a tunnel only, a /32, a
            // network too wide to walk. **Not a guessed /24** — deriving one from the local address
            // covers a quarter of the owner's /22 and misses three times in four, and the recovery
            // loop would go on asking every twenty seconds and go on being wrong.
            tracing::debug!("no network worth sweeping");
            return Vec::new();
        }
        tracing::debug!(addresses = targets.len(), "sweeping for a machine");

        let http = reqwest::Client::builder()
            .connect_timeout(SWEEP_CONNECT_TIMEOUT)
            .timeout(SWEEP_PROBE_TIMEOUT)
            // Nothing here is spoken to twice, and a pool of idle sockets to a thousand addresses
            // that did not answer is a thousand descriptors held for nothing.
            .pool_max_idle_per_host(0)
            // Without this a thousand probes go to whatever proxy the system happens to name, which
            // is neither what was meant nor kind to the proxy.
            .no_proxy()
            .build();
        let Ok(http) = http else {
            return Vec::new();
        };

        let wanted = self.wanted.read().ok().and_then(|wanted| wanted.clone());
        let found = handle.block_on(race(targets, budget, move |ip| {
            let http = http.clone();
            let wanted = wanted.clone();
            async move {
                let sighting = probe(&http, ip).await?;
                // A hunt walks past a machine that is not the one being looked for; see
                // [`Sweep::hunt`]. With nothing wanted, the first machine is the answer as before.
                match wanted {
                    Some(wanted) if sighting.id.as_deref() != Some(wanted.as_str()) => None,
                    _ => Some(sighting),
                }
            }
        }));
        // **One machine and not a list, and that is honest rather than lazy.** A sweep stops at its
        // first answer by design — walking every remaining address to complete a list would spend
        // the whole budget every time, on a subnet, for a second machine almost nobody has.
        found.into_iter().collect()
    }

    fn hunt(&self, id: Option<&str>) {
        self.want(id);
    }
}

/// The machine answering at `ip`, if one is.
///
/// **It answers a [`Sighting`] where it used to answer `bool`**, which costs nothing: the discovery
/// document was already fetched and already parsed, and its id, name and auth mode were thrown away
/// one line later. Keeping them is what lets a sweep feed the same identity-anchored decision an
/// mDNS announcement does — on the one platform that may not multicast at all.
#[cfg(feature = "sweep")]
async fn probe(http: &reqwest::Client, ip: std::net::Ipv4Addr) -> Option<Sighting> {
    let url = format!(
        "http://{ip}:{SWEEP_PORT}{}/discover",
        km_api::discover::API_BASE
    );
    let response = http.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let discovery = response.json::<km_api::discover::Discovery>().await.ok()?;
    // Not an open port, and not merely valid JSON: a home network has routers, printers and set-top
    // boxes on it, and the cost of believing one is a remote that opens on a printer every morning.
    if discovery.app != km_api::discover::APP {
        return None;
    }
    Some(Sighting {
        name: discovery.name,
        // An empty id is a machine too old to have one, which is `None` rather than `Some("")` —
        // otherwise two such machines would look like one another's identity.
        id: Some(discovery.id).filter(|id| !id.is_empty()),
        // **The address that answered, and deliberately not the machine's own `urls.first()`.**
        // `The advert names the address the machine chose` prefers the machine's ranking over a
        // client's guess, and that is right for an *announcement*, where the A records arrive one
        // interface at a time and none of them is known to work. A sweep is the opposite case: this
        // address was just probed from this device and replied, which is proof rather than a
        // ranking — and the machine's preferred address may be on a subnet this phone cannot reach.
        // Preferring an unverified address over a verified one would be a regression dressed as
        // consistency.
        url: format!("http://{ip}:{SWEEP_PORT}"),
    })
}

/// Every IPv4 network this host is on.
///
/// The only line in the sweep that talks to the operating system, which is what the `sweep` feature
/// is gating: `if-addrs` is already a direct dependency of `km-api`, so this costs no new crate,
/// and it handles the `sockaddr` differences between Apple and everything else — which is exactly
/// where a hand-rolled `getifaddrs` goes wrong, and would have needed an `unsafe` block in a crate
/// that has none.
#[cfg(feature = "sweep")]
fn interfaces() -> Vec<Net> {
    let Ok(found) = if_addrs::get_if_addrs() else {
        tracing::debug!("cannot list this host's network interfaces");
        return Vec::new();
    };
    found
        .into_iter()
        .filter_map(|interface| {
            let up = interface.is_oper_up();
            let p2p = interface.is_p2p();
            let name = interface.name.clone();
            match interface.addr {
                if_addrs::IfAddr::V4(v4) => Some(Net {
                    ip: v4.ip,
                    prefix: v4.prefixlen,
                    interface: name,
                    up,
                    p2p,
                }),
                if_addrs::IfAddr::V6(_) => None,
            }
        })
        .collect()
}

/// Where the machine this device knows is written down.
const MACHINE_FILE: &str = "last-machine.json";

/// How long to wait for an mDNS answer before giving up and starting anyway.
///
/// Short on purpose. A remote that took ten seconds to open because it was looking for a television
/// that is switched off would be a remote nobody opens.
///
/// **It is a deadline and not a cost, and this doc said otherwise for a while.** It claimed the
/// one-shot this replaced "used to be paid on every single look", which was never true — that one
/// returned at the first announcement and reached the timeout only when nothing answered. Written
/// as though the old code were slower than it was, it excused a `look` that really did sleep the
/// whole 1.5 s on an empty registry, and made a cold start twelve times slower than the thing it
/// replaced. Measured: 131 ms before, 1575 ms after, 150 ms once [`Mdns::look`] went back to
/// stopping at the first machine.
///
/// So: it is reached when nothing is there, on a cold registry, and never otherwise. With something
/// on the network a look costs what mDNS costs; with a warm registry it costs nothing.
pub const BROWSE_TIMEOUT: Duration = Duration::from_millis(1500);

/// How often a cold look re-reads the registry while it waits.
///
/// **Granularity, not a delay.** It bounds how much later than the first announcement an answer can
/// be, and twenty-five milliseconds is far below anything a person perceives while being coarse
/// enough that a wait costs a handful of wake-ups rather than a spin. It is only ever reached on a
/// registry that is still empty, which is the first seconds of a process and no other time.
///
/// **Gated, where [`BROWSE_TIMEOUT`] above needs no gate**, and the difference is only that this one
/// is private: its single use is inside `impl Locator for Mdns`, so a build with `mdns` off — which
/// is what `km-remote-ios` asks for, since Apple grants multicast only after a manual review — has
/// a constant nothing reads and says so. Nothing on CI compiles that combination, and `task check`
/// lints the desktop feature set where `mdns` is on, so the warning appears only in the iOS build.
#[cfg(feature = "mdns")]
const LOOK_POLL: Duration = Duration::from_millis(25);

/// How often a remote that cannot reach its machine looks again.
///
/// **This is now the fallback tick rather than the mechanism**, and it stays for three reasons that
/// are easy to miss once updates arrive by push: [`Sweep`] has nothing to push and must be polled;
/// writing the record down once a connection answers is a timer's job and always was; and a
/// [`Watcher`] whose daemon would not open needs something to call `poke` on it.
///
/// Twenty seconds is short enough that somebody who has just switched a machine on does not wait for
/// it and long enough that a remote left open all day is not a nuisance on the network. What changed
/// is that the interesting case — the machine reappearing at a new address — now fires in under a
/// second instead of up to twenty.
pub const RECOVERY_INTERVAL: Duration = Duration::from_secs(20);

/// The address a run should use, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The base URL — `http://192.168.1.5:8177`.
    pub url: String,
    /// How it was found, for the startup line and the machine card.
    pub how: Why,
}

/// Where the record lives beside a data directory.
#[must_use]
pub fn machine_path(dir: &Path) -> PathBuf {
    dir.join(MACHINE_FILE)
}

/// The machine this device knows about.
#[must_use]
pub fn known(dir: &Path) -> Option<Known> {
    known::read(&machine_path(dir))
}

/// How often a connection that has not otherwise changed refreshes its timestamp.
///
/// **The record is written when the machine or its address changes, and otherwise at most this
/// often.** Both halves matter. Writing on every refresh would be a write a second for an evening;
/// writing *only* on a change would freeze `last_connected` at the first connection, so a machine in
/// continuous use would read as [stale](known::is_stale) six hours later — which is exactly
/// backwards. An hour is fine grain for a measurement made in hours.
pub const REMEMBER_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Writes the machine down.
///
/// **The remote only ever calls this for a machine that answered**, which is the rule
/// `A remote looks again when its machine goes quiet` turns on: an address recorded without having
/// been reached is one a look can never get past, and that is how a remote spent an evening on a
/// machine switched off in another room. `km-admin` deliberately keeps the opposite rule, for
/// reasons its own module records.
pub fn write_known(dir: &Path, record: &Known) {
    known::write(&machine_path(dir), record);
}

/// Looks for a machine: what was asked for, then what is known, then the network.
///
/// **Nothing here waits on the network in the ordinary case, and now it does not wait in the cold
/// one either.** The remembered address is preferred because opening instantly beats opening in a
/// second and a half, and it is not verified here — checking would mean a request, and the event
/// stream is about to make one anyway. What that reasoning used to miss is that a stale address was
/// then never replaced; [`Radar`](crate::find::Radar) and `Server::machine_watch` are the other half, looking in
/// the background rather than making this function slower.
///
/// The third rung reads the registry rather than starting a browse, so a watcher that has been
/// listening since the process began answers it for nothing.
#[must_use]
pub fn locate(asked_for: Option<&str>, known: Option<&Known>, radar: &Radar) -> Option<Found> {
    if let Some(url) = asked_for {
        return Some(Found {
            url: normalize(url),
            how: Why::AskedFor,
        });
    }
    if let Some(known) = known {
        return Some(Found {
            url: normalize(&known.url),
            how: Why::Remembered,
        });
    }
    radar.look_now().into_iter().next().map(|sighting| Found {
        url: sighting.url,
        how: Why::Adopted,
    })
}

/// What the network is saying, however this host is able to hear it.
///
/// **One place decides "push where we can, poll where we must".** [`Mdns`] hands over a [`Watcher`]
/// that fills itself in; [`Sweep`] and [`NoLocator`] cannot, so their answers are whatever the last
/// look produced. Every consumer asks this and none of them has to know which.
pub struct Radar {
    locator: std::sync::Arc<dyn Locator>,
    /// Present only for a locator that can listen.
    watcher: Option<Watcher>,
    /// The last poll's answer, for one that cannot.
    polled: std::sync::RwLock<Vec<Observed>>,
}

impl Radar {
    /// Wraps a locator, taking its watcher if it has one.
    #[must_use]
    pub fn new(locator: std::sync::Arc<dyn Locator>) -> Self {
        let watcher = locator.watch();
        Self {
            locator,
            watcher,
            polled: std::sync::RwLock::new(Vec::new()),
        }
    }

    /// Whether looking is a thing this host can do at all.
    #[must_use]
    pub fn can_browse(&self) -> bool {
        self.locator.can_browse()
    }

    /// Whether the registry keeps itself current, or has to be asked.
    ///
    /// True for mDNS, false for a sweep — which is what tells the watch loop whether a poll is work
    /// it has to do or work that has already been done for it.
    #[must_use]
    pub fn watches_by_itself(&self) -> bool {
        self.watcher.is_some()
    }

    /// What is known right now, without going near the network.
    #[must_use]
    pub fn seen(&self) -> Vec<Observed> {
        if let Some(watcher) = &self.watcher {
            return watcher.observed();
        }
        self.polled
            .read()
            .map(|polled| polled.clone())
            .unwrap_or_default()
    }

    /// The machines on the network now, for a list somebody is looking at.
    #[must_use]
    pub fn sightings(&self) -> Vec<Sighting> {
        let mut found: Vec<Sighting> = self
            .seen()
            .into_iter()
            .filter(|observed| observed.present)
            .map(|observed| observed.sighting)
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.url.cmp(&b.url)));
        found
    }

    /// Ask now, and answer with what that produced.
    ///
    /// **Blocking**, because a [`Locator`] is: `Mdns` waits only on a cold registry and `Sweep`
    /// walks a subnet. Call it from [`Radar::poke`](crate::find::Radar::poke) or under `spawn_blocking`.
    fn look_now(&self) -> Vec<Sighting> {
        let found = self.locator.look();
        self.record(&found);
        found
    }

    /// Keeps what a poll saw, when there is no watcher holding it instead.
    ///
    /// **Extracted because it was written twice**, once in `look_now` and once in `look`, differing
    /// only in whether the call around it was blocking. Two copies of the construction of an
    /// `Observed` is two places to update when it gains a field, and only one of them would fail to
    /// compile if the other were forgotten.
    ///
    /// Everything a poll returns is `present` by definition -- it answered a moment ago -- and both
    /// timestamps are now, because a poll has no memory of when it first saw anything.
    fn record(&self, found: &[Sighting]) {
        if self.watcher.is_some() {
            // The watcher is the thing holding it; a poll's snapshot would only compete.
            return;
        }
        let now = std::time::Instant::now();
        let observed = found
            .iter()
            .cloned()
            .map(|sighting| Observed {
                sighting,
                present: true,
                first_seen: now,
                last_seen: now,
            })
            .collect();
        *self
            .polled
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = observed;
    }

    /// Ask the network again, off the runtime.
    pub async fn poke(&self) {
        if let Some(watcher) = &self.watcher {
            watcher.poke();
            return;
        }
        // Nothing to poke, so a poll is the only way to learn anything new.
        self.look().await;
    }

    /// One look, off the runtime, answering what it found.
    pub async fn look(&self) -> Vec<Sighting> {
        let locator = std::sync::Arc::clone(&self.locator);
        let found = tokio::task::spawn_blocking(move || locator.look())
            .await
            .unwrap_or_default();
        self.record(&found);
        found
    }

    /// Resolves when the network has said something new.
    ///
    /// Never, for a locator with no watcher — the caller's fallback tick is what drives those, and a
    /// future that resolves immediately would turn that tick into a spin.
    pub async fn changed(&self) {
        let Some(watcher) = &self.watcher else {
            std::future::pending::<()>().await;
            return;
        };
        let mut changes = watcher.changed();
        let _ = changes.changed().await;
    }

    /// Tell the locator which machine to look for. Nothing, for one that hears them all anyway.
    pub fn hunt(&self, id: Option<&str>) {
        self.locator.hunt(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // **The policy lives in `km_api::discover::known::choose`**, where its five cases are asserted
    // — a record with no id has to behave exactly as a bare address does. A second policy function
    // here, free to disagree with that one, is exactly the drift the single definition prevents.
    // What is here is the *looking*: the locators, and the ladder that decides which rung answers.

    /// A locator that answers whatever it was told to, and counts how often it was asked.
    ///
    /// The count is the interesting half: what [`locate`] promises is not only that it finds a
    /// machine but that it does **not** go looking when it already has an answer, and only a
    /// counter can tell the two apart.
    struct StubLocator {
        answer: Vec<Sighting>,
        asked: AtomicUsize,
        wanted: std::sync::Mutex<Option<String>>,
    }

    impl StubLocator {
        fn new(answer: Option<&str>) -> Self {
            Self::seeing(answer.into_iter().map(|url| sighting(None, url)).collect())
        }

        fn seeing(answer: Vec<Sighting>) -> Self {
            Self {
                answer,
                asked: AtomicUsize::new(0),
                wanted: std::sync::Mutex::new(None),
            }
        }

        fn asked(&self) -> usize {
            self.asked.load(Ordering::Relaxed)
        }

        fn hunting(&self) -> Option<String> {
            self.wanted.lock().expect("not poisoned").clone()
        }
    }

    impl Locator for StubLocator {
        fn look(&self) -> Vec<Sighting> {
            self.asked.fetch_add(1, Ordering::Relaxed);
            self.answer.clone()
        }

        fn hunt(&self, id: Option<&str>) {
            *self.wanted.lock().expect("not poisoned") = id.map(str::to_owned);
        }
    }

    fn sighting(id: Option<&str>, url: &str) -> Sighting {
        Sighting {
            name: "Living Room".to_owned(),
            id: id.map(str::to_owned),
            url: url.to_owned(),
        }
    }

    fn radar_over(locator: StubLocator) -> (Radar, std::sync::Arc<StubLocator>) {
        let locator = std::sync::Arc::new(locator);
        (
            Radar::new(std::sync::Arc::clone(&locator) as std::sync::Arc<dyn Locator>),
            locator,
        )
    }

    #[test]
    fn what_a_person_types_becomes_a_url() {
        assert_eq!(normalize("192.168.1.5"), "http://192.168.1.5:8177");
        assert_eq!(normalize("192.168.1.5:9000"), "http://192.168.1.5:9000");
        assert_eq!(
            normalize("http://192.168.1.5:8177/"),
            "http://192.168.1.5:8177"
        );
        assert_eq!(normalize("  karaoke.local  "), "http://karaoke.local:8177");
    }

    /// The scheme's own colon must not be mistaken for a port, or every typed address would be left
    /// without one.
    #[test]
    fn the_schemes_colon_is_not_a_port() {
        assert_eq!(
            normalize("http://karaoke.local"),
            "http://karaoke.local:8177"
        );
    }

    /// A directory of this test's own. Held by the caller, because it goes when the value does.
    fn scratch(name: &str) -> crate::testing::Scratch {
        crate::testing::Scratch::new(name)
    }

    #[test]
    fn a_machine_is_remembered_and_read_back() {
        let dir = scratch("remember");

        assert_eq!(known(&dir), None);
        write_known(
            &dir,
            &Known::at("http://192.168.1.5:8177", Why::Adopted).answered(
                "abc123",
                Some("Living Room".to_owned()),
                std::time::SystemTime::now(),
            ),
        );

        let record = known(&dir).expect("remembered");
        assert_eq!(record.url, "http://192.168.1.5:8177");
        assert_eq!(record.id.as_deref(), Some("abc123"));
        assert!(record.last_connected.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tried before the network, which is the difference between opening instantly and opening in a
    /// second and a half.
    ///
    /// The locator is asserted **never to have been asked** in either arm, which is the half of the
    /// promise a returned value cannot show: an implementation that browsed first and then threw the
    /// answer away would satisfy every assertion about `found` and still cost the second and a half
    /// this ordering exists to save.
    #[test]
    fn what_was_asked_for_beats_what_was_remembered() {
        let dir = scratch("order");
        write_known(&dir, &Known::at("http://remembered:8177", Why::Remembered));
        let record = known(&dir);

        let (radar, locator) = radar_over(StubLocator::new(Some("http://network:8177")));

        let found = locate(Some("192.168.1.9"), record.as_ref(), &radar).expect("asked for");
        assert_eq!(found.url, "http://192.168.1.9:8177");
        assert_eq!(found.how, Why::AskedFor);

        let found = locate(None, record.as_ref(), &radar).expect("remembered");
        assert_eq!(found.url, "http://remembered:8177");
        assert_eq!(found.how, Why::Remembered);

        assert_eq!(
            locator.asked(),
            0,
            "the network was consulted despite an answer already being to hand"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With nothing asked for and nothing remembered, the locator is what answers — and finding
    /// nothing is an ordinary result rather than a failure.
    #[test]
    fn the_network_is_the_last_resort_and_may_find_nothing() {
        let (radar, locator) = radar_over(StubLocator::new(Some("http://network:8177")));
        let located = locate(None, None, &radar).expect("found on the network");
        assert_eq!(located.url, "http://network:8177");
        assert_eq!(located.how, Why::Adopted);
        assert_eq!(locator.asked(), 1);

        // Nothing switched on, no mDNS at all, a container: all the same answer, and none of them
        // an error. This is what every mobile host passes before it has the permission to browse.
        let blind = Radar::new(std::sync::Arc::new(NoLocator) as std::sync::Arc<dyn Locator>);
        assert!(locate(None, None, &blind).is_none());
        assert!(!blind.can_browse());
    }

    /// A locator that cannot listen has its answer kept, so asking the registry between polls is
    /// not the same as having found nothing.
    #[test]
    fn a_poll_is_remembered_until_the_next_one() {
        let (radar, _locator) = radar_over(StubLocator::new(Some("http://network:8177")));
        assert!(radar.seen().is_empty(), "nothing asked yet");
        assert!(!radar.watches_by_itself(), "a stub has nothing to push");

        let _ = locate(None, None, &radar);
        let seen = radar.seen();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].present, "it answered a moment ago");
        assert_eq!(radar.sightings()[0].url, "http://network:8177");
    }

    /// The id a remote is anchored to is passed down to the locator, which is what lets a sweep walk
    /// past a machine that is not the one being looked for.
    #[test]
    fn the_wanted_machine_reaches_the_locator() {
        let (radar, locator) = radar_over(StubLocator::seeing(vec![sighting(
            Some("abc123"),
            "http://192.168.1.42:8177",
        )]));
        assert_eq!(locator.hunting(), None);
        radar.hunt(Some("abc123"));
        assert_eq!(locator.hunting().as_deref(), Some("abc123"));
        radar.hunt(None);
        assert_eq!(locator.hunting(), None);
    }

    // -- the sweep ------------------------------------------------------------------------------
    //
    // None of this is behind the `sweep` feature, and that is deliberate: everything below is
    // arithmetic and policy, so it compiles and runs under the everyday test command rather than
    // under a flag a desktop has no reason to turn on. What the feature gates is the one call that
    // asks the operating system for its interfaces, and the HTTP probe.

    use std::net::Ipv4Addr;

    fn net(ip: &str, prefix: u8, interface: &str) -> Net {
        Net {
            ip: ip.parse().expect("address"),
            prefix,
            interface: interface.to_owned(),
            up: true,
            p2p: false,
        }
    }

    /// Every boundary of the ceiling, including both sides of it.
    #[test]
    fn a_network_is_walked_only_when_it_is_worth_walking() {
        assert_eq!(
            reach(32),
            Reach::Alone,
            "a /32 is this host and nothing else"
        );
        assert_eq!(reach(31), Reach::Alone, "a /31 is a point-to-point pair");
        assert_eq!(
            reach(0),
            Reach::Unbounded,
            "an all-zero mask is what some VPN adapters report"
        );
        assert_eq!(reach(24), Reach::Walk(254));
        assert_eq!(reach(22), Reach::Walk(1022), "the reference home network");
        assert_eq!(
            reach(20),
            Reach::Walk(MAX_SWEEP_HOSTS),
            "the widest accepted"
        );
        assert_eq!(reach(19), Reach::TooWide(8190), "one step past it");
        assert_eq!(reach(16), Reach::TooWide(65534));
    }

    /// The order, not merely the set.
    ///
    /// **Asserting membership would pass on a walk that went `.1` upwards**, and that walk is four
    /// times slower to find a machine sitting a few addresses away — which is the entire reason
    /// this is ordered rather than enumerated.
    #[test]
    fn the_walk_goes_outward_from_here_alternating() {
        let order = sweep_order("192.168.1.5".parse().unwrap(), 24);
        assert_eq!(
            &order[..4],
            &[
                Ipv4Addr::new(192, 168, 1, 6),
                Ipv4Addr::new(192, 168, 1, 4),
                Ipv4Addr::new(192, 168, 1, 7),
                Ipv4Addr::new(192, 168, 1, 3),
            ],
        );
        assert_eq!(order.len(), 253, "254 usable, less this host");
        assert!(
            !order.contains(&Ipv4Addr::new(192, 168, 1, 0)),
            "no network address"
        );
        assert!(
            !order.contains(&Ipv4Addr::new(192, 168, 1, 255)),
            "no broadcast"
        );
        assert!(
            !order.contains(&Ipv4Addr::new(192, 168, 1, 5)),
            "not this host"
        );
    }

    /// The edge the Go original guards and a direct translation does not.
    ///
    /// Its walk is `uint32` arithmetic behind an explicit `selfN >= step`; in Rust the same
    /// subtraction panics in a debug build and wraps in a release one, and the address it wraps to
    /// is `255.255.255.255`. Both ends of the subnet, because the same mistake is available going
    /// up as going down.
    #[test]
    fn a_host_at_either_end_of_its_subnet_does_not_run_off_it() {
        let low = sweep_order("192.168.1.1".parse().unwrap(), 24);
        assert_eq!(low.first(), Some(&Ipv4Addr::new(192, 168, 1, 2)));
        assert_eq!(low.last(), Some(&Ipv4Addr::new(192, 168, 1, 254)));
        assert_eq!(low.len(), 253);

        let high = sweep_order("192.168.1.254".parse().unwrap(), 24);
        assert_eq!(high.first(), Some(&Ipv4Addr::new(192, 168, 1, 253)));
        assert_eq!(high.len(), 253);

        for order in [low, high] {
            assert!(!order.contains(&Ipv4Addr::UNSPECIFIED));
            assert!(!order.contains(&Ipv4Addr::BROADCAST));
        }

        // And the same at the very top of the address space, which is where `checked_add` is what
        // saves it rather than the broadcast comparison.
        let top = sweep_order("255.255.255.254".parse().unwrap(), 24);
        assert_eq!(top.first(), Some(&Ipv4Addr::new(255, 255, 255, 253)));
        assert_eq!(top.len(), 253);
        assert!(!top.contains(&Ipv4Addr::BROADCAST));
    }

    /// A refused network yields nothing rather than something wrong.
    #[test]
    fn a_network_that_is_refused_is_not_walked_anyway() {
        assert!(sweep_order("10.8.0.2".parse().unwrap(), 32).is_empty());
        assert!(sweep_order("10.0.0.1".parse().unwrap(), 0).is_empty());
        assert!(sweep_order("10.0.0.1".parse().unwrap(), 16).is_empty());
    }

    /// The configurations that are the whole reason this is a list rather than one address.
    ///
    /// Every kind of entry is real: `bridge100` is macOS's own virtual-machine bridge and is up
    /// with a perfectly valid private address on it, `utun3` is a VPN, and a `169.254` address
    /// means DHCP failed. None of them is where the karaoke machine is.
    ///
    /// The *numbers* are the documented sample set rather than the ones a real Mac uses -- see the
    /// `What a committed file may say about the machine it was written on` decision in
    /// docs/decisions/repository.md, which `tools/dev/check-no-local-refs.sh` enforces by shape. Nothing here depends
    /// on the exact octets: what the ranking reads is the interface name and the private range.
    #[test]
    fn the_real_adapter_is_swept_first_and_the_others_not_at_all() {
        let mut lo = net("127.0.0.1", 8, "lo0");
        lo.up = true;
        let mut tunnel = net("10.8.0.2", 32, "utun3");
        tunnel.p2p = true;
        let mut unplugged = net("192.168.1.20", 24, "en5");
        unplugged.up = false;

        let nets = vec![
            lo,
            tunnel,
            net("192.168.56.1", 24, "bridge100"),
            net("192.168.1.37", 22, "en0"),
            net("169.254.7.7", 16, "en1"),
            unplugged,
        ];

        let chosen = sweep_networks(&nets);
        assert_eq!(
            chosen
                .iter()
                .map(|n| n.interface.as_str())
                .collect::<Vec<_>>(),
            vec!["en0", "bridge100"],
            "the real adapter first, the VM bridge behind it, and nothing else at all"
        );
    }

    /// Two interfaces on one subnet is a docked laptop, not a mistake.
    #[test]
    fn one_address_is_probed_once_however_many_interfaces_reach_it() {
        let nets = vec![
            net("192.168.1.37", 24, "en0"),
            net("192.168.1.38", 24, "en1"),
        ];
        let targets = sweep_targets(&nets);
        let unique: std::collections::HashSet<_> = targets.iter().copied().collect();
        assert_eq!(targets.len(), unique.len(), "no address is probed twice");
        assert!(
            targets.contains(&Ipv4Addr::new(192, 168, 1, 37)),
            "each is the other's neighbor"
        );
        assert!(targets.contains(&Ipv4Addr::new(192, 168, 1, 38)));
    }

    /// The two things a returned address cannot demonstrate: the order, and the stop.
    ///
    /// **This is what the probe is a parameter for.** A sweep that found the machine by walking all
    /// 1,022 addresses would pass any assertion about its return value and would still be the slow,
    /// noisy thing this design exists to avoid.
    #[tokio::test]
    async fn a_sweep_stops_at_the_first_answer() {
        let nets = vec![net("192.168.1.5", 22, "en0")];
        let targets = sweep_targets(&nets);
        assert_eq!(targets.len(), 1021, "a /22, less this host");

        let machine: Ipv4Addr = "192.168.1.9".parse().unwrap();
        let asked = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&asked);

        let found = race(targets.clone(), Duration::from_secs(5), move |ip| {
            let counter = std::sync::Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                (ip == machine).then_some(ip)
            }
        })
        .await;

        assert_eq!(found, Some(machine));
        assert!(
            asked.load(Ordering::SeqCst) < targets.len() / 2,
            "a sweep that asked {} of {} addresses did not stop early",
            asked.load(Ordering::SeqCst),
            targets.len(),
        );
    }

    /// Nothing out there is an ordinary answer, and it arrives within the budget.
    #[tokio::test]
    async fn a_sweep_that_finds_nothing_says_so() {
        let nets = vec![net("192.168.1.5", 24, "en0")];
        let found = race(sweep_targets(&nets), Duration::from_secs(5), |_ip| async {
            None::<std::net::Ipv4Addr>
        })
        .await;
        assert_eq!(found, None);
    }

    /// **A sweep looking for one machine walks past the others.** The whole of the iOS half of
    /// identity-first resolution, and it needs no network to show: the probe is a parameter, so the
    /// two machines are a closure.
    #[tokio::test]
    async fn a_sweep_walks_past_a_machine_that_is_not_the_one_it_wants() {
        let nets = vec![net("192.168.1.5", 24, "en0")];
        let stranger: Ipv4Addr = "192.168.1.6".parse().unwrap();
        let wanted: Ipv4Addr = "192.168.1.9".parse().unwrap();

        let found = race(
            sweep_targets(&nets),
            Duration::from_secs(5),
            move |ip| async move {
                let id = if ip == wanted {
                    "abc123"
                } else if ip == stranger {
                    "someone-else"
                } else {
                    return None;
                };
                // What `Sweep::look` does with `wanted` set, written out.
                (id == "abc123").then(|| sighting(Some(id), &format!("http://{ip}:8177")))
            },
        )
        .await;

        let found = found.expect("the machine it was looking for");
        assert_eq!(found.id.as_deref(), Some("abc123"));
        assert_eq!(found.url, "http://192.168.1.9:8177");
    }
}
