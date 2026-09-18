//! Listening for machines, continuously, instead of asking when somebody presses a button.
//!
//! [`super::browse`] opens a daemon, waits a fixed timeout and shuts it down again. That is the
//! wrong shape for every caller it has: a page pays the timeout on each press, a remote pays it at
//! startup, and between two of them nobody is listening at all — so a machine that comes back on a
//! new address is noticed only by whoever next asks. This module holds the daemon open for the life
//! of the process and keeps the answer ready, so asking costs nothing.
//!
//! # Two types, because only one of them can be tested
//!
//! [`Registry`] is the table and has no socket in it: announcements go in through
//! [`Registry::observe_at`], departures through [`Registry::removed_at`], and the answer comes out
//! of [`Registry::sightings`]. [`Watcher`] is the daemon, the thread and a `Registry` behind a lock.
//!
//! **Nothing in the test suite may construct a [`Watcher`].** `mdns_sd::ServiceDaemon::new` binds
//! `0.0.0.0:5353`, which is exactly what `CONTRIBUTING.md`'s *No test binds a non-loopback address*
//! forbids — and a watcher is worse than the one-shot it replaces, because it would go on
//! retransmitting. Every test here drives a `Registry` by hand.
//!
//! The daemon comes from [`super::daemon`] rather than from `mdns_sd` directly, so a checkout that
//! sets [`super::NO_MDNS_ENV_VAR`] holds the socket shut here as well — including across
//! [`Watcher::poke`], which opens one again whenever somebody presses Rescan.
//!
//! # Two clocks, kept apart
//!
//! The registry measures in [`Instant`]: *how long since this process heard from it*, which is
//! monotonic and meaningless across a restart. Wall-clock staleness — *when did this machine last
//! answer me*, which has to survive being written to a file — is [`super::known`]'s, and is a
//! `SystemTime`. Mixing them would produce a record that reads as fresh after every reboot.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{Announcement, SERVICE_TYPE, Sighting, sighting_url};

/// One machine, as it is being pieced together across the announcements it arrives in.
///
/// **The accumulation is the point.** `mdns-sd` fires `ServiceResolved` the moment it holds one
/// address, and again as more turn up — and it only ever sends the addresses on the subnet of the
/// interface a packet left by, so a machine holding four addresses announces them one per packet.
/// Taking the first event and skipping the rest showed whichever interface answered first, which is
/// not a choice but a race.
#[derive(Debug, Clone)]
struct Seen {
    name: Option<String>,
    id: Option<String>,
    url: Option<String>,
    addresses: Vec<std::net::Ipv4Addr>,
    port: u16,
    /// Every DNS-SD fullname this machine has announced under.
    ///
    /// **Plural, and that is not defensive.** A row is keyed by the `id` TXT record, but a
    /// `ServiceRemoved` carries only `(service_type, fullname)` — so without this a departure could
    /// not find the row it is about. It is a list rather than a field because renaming a machine
    /// re-registers it under a new instance label while its id stays put, and both labels name it
    /// until the old one is withdrawn.
    fullnames: Vec<String>,
    /// Whether it is on the network *now*. See [`Registry::removed_at`].
    present: bool,
    first_seen: Instant,
    last_seen: Instant,
}

impl Seen {
    /// Whether two states differ in any way a caller could see.
    ///
    /// Deliberately does **not** compare the timestamps: a machine re-announcing itself unchanged
    /// every minute is the ordinary case, and waking every subscriber for it would make the change
    /// signal useless. What counts as a change is what [`Registry::sightings`] would show.
    fn differs(&self, other: &Self) -> bool {
        self.name != other.name
            || self.id != other.id
            || self.url != other.url
            || self.addresses != other.addresses
            || self.port != other.port
            || self.present != other.present
    }
}

/// A machine the registry knows about, with everything it knows.
///
/// **A wrapper around [`Sighting`] rather than more fields on it.** `Sighting` is the answer to
/// *what is on the network*, it derives `PartialEq`, and a good deal of this repository compares
/// two of them. Hanging a timestamp off it would make every one of those comparisons depend on
/// when it was taken. So the four fields stay as they are and the bookkeeping lives out here.
#[derive(Debug, Clone)]
pub struct Observed {
    /// What a list would show.
    pub sighting: Sighting,
    /// Whether it is announcing itself now.
    ///
    /// **False is not the same as gone**, and every caller has to know the difference — see
    /// [`Registry::removed_at`].
    pub present: bool,
    /// When this machine was first heard from, this run.
    pub first_seen: Instant,
    /// When it was last heard from.
    pub last_seen: Instant,
}

/// Every machine heard from, merged.
///
/// Split from the socket loop so that "the same machine four times with one address each" is a unit
/// test rather than a house with a Hyper-V switch in it.
#[derive(Debug, Default)]
pub struct Registry {
    /// Insertion-ordered, because the number of machines on a home network is not a number that
    /// wants a hash map. Keyed by the `id` TXT record, or by the DNS-SD fullname without one.
    machines: Vec<(String, Seen)>,
}

impl Registry {
    /// An empty one.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds one resolved announcement in. Answers whether anything a caller could see changed.
    pub fn observe_at(&mut self, announcement: Announcement, now: Instant) -> bool {
        let Announcement {
            key,
            fullname,
            name,
            id,
            url,
            addresses,
            port,
        } = announcement;

        let position = self.machines.iter().position(|(seen, _)| *seen == key);
        let index = match position {
            Some(index) => index,
            None => {
                self.machines.push((
                    key,
                    Seen {
                        name: None,
                        id: None,
                        url: None,
                        addresses: Vec::new(),
                        port: 0,
                        fullnames: Vec::new(),
                        present: false,
                        first_seen: now,
                        last_seen: now,
                    },
                ));
                self.machines.len() - 1
            }
        };
        let before = self.machines[index].1.clone();
        let seen = &mut self.machines[index].1;

        // Later announcements are the more complete ones, so they win where they say anything;
        // where they are silent, what is already known stands.
        if name.is_some() {
            seen.name = name;
        }
        if id.is_some() {
            seen.id = id;
        }
        if url.is_some() {
            seen.url = url;
        }
        seen.port = port;
        for address in addresses {
            if !seen.addresses.contains(&address) {
                seen.addresses.push(address);
            }
        }
        if !fullname.is_empty() && !seen.fullnames.contains(&fullname) {
            seen.fullnames.push(fullname);
        }
        seen.present = true;
        seen.last_seen = now;

        seen.differs(&before)
    }

    /// Records that a machine has withdrawn its advertisement.
    ///
    /// **It marks the row absent and removes nothing**, which is the single most important rule in
    /// this module and the one that looks most like a leak.
    ///
    /// The reason is Android. `MainActivity` takes the `WifiManager.MulticastLock` in `onStart` and
    /// releases it in `onStop`, so a remote in somebody's pocket hears nothing at all — and
    /// `mdns-sd` will duly expire the record by TTL and report a `ServiceRemoved` for a machine that
    /// is switched on, announcing itself, and two meters away. Dropping the row there would turn
    /// *the phone was in a pocket* into *the machine went away*, which is the fault
    /// `A remote looks again when its machine goes quiet` was written against, arriving from the
    /// other end.
    ///
    /// So an absent row keeps everything known about it and goes on being a usable cache of an
    /// address. What it may not do is *cause* anything: the rule on the other side, in
    /// [`super::known::choose`], is that only a **present** sighting may move a connection. Absence
    /// is never evidence.
    pub fn removed_at(&mut self, fullname: &str, now: Instant) -> bool {
        for (_, seen) in &mut self.machines {
            if seen.fullnames.iter().any(|known| known == fullname) {
                let changed = seen.present;
                seen.present = false;
                seen.last_seen = now;
                return changed;
            }
        }
        false
    }

    /// The machines on the network now, best address each.
    ///
    /// Sorted by name so the list does not reshuffle between looks. Absent machines are left out —
    /// this is the answer to *what is on the network*, and a row for something that has gone is a
    /// row that does not work.
    #[must_use]
    pub fn sightings(&self) -> Vec<Sighting> {
        let mut found: Vec<Sighting> = self
            .observed()
            .into_iter()
            .filter(|observed| observed.present)
            .map(|observed| observed.sighting)
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.url.cmp(&b.url)));
        found
    }

    /// Everything heard from this run, present or not, with the bookkeeping.
    #[must_use]
    pub fn observed(&self) -> Vec<Observed> {
        self.machines
            .iter()
            .filter_map(|(key, seen)| {
                let url = sighting_url(seen.url.as_deref(), &seen.addresses, seen.port)?;
                Some(Observed {
                    sighting: Sighting {
                        name: seen.name.clone().unwrap_or_else(|| key.clone()),
                        id: seen.id.clone(),
                        url,
                    },
                    present: seen.present,
                    first_seen: seen.first_seen,
                    last_seen: seen.last_seen,
                })
            })
            .collect()
    }

    /// One machine by its instance id, present or not.
    ///
    /// The caller decides what to do about absence, which is the whole point of
    /// [`Observed::present`] being on the answer rather than a filter applied here.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<Observed> {
        self.observed()
            .into_iter()
            .find(|observed| observed.sighting.id.as_deref() == Some(id))
    }
}

/// A live mDNS browse, held open.
///
/// Cheap to clone — every clone is the same daemon and the same table, and the daemon is shut down
/// when the last of them goes.
#[derive(Clone)]
pub struct Watcher {
    inner: Arc<Inner>,
}

struct Inner {
    registry: Mutex<Registry>,
    /// Bumped whenever [`Registry::sightings`] would answer differently.
    generation: tokio::sync::watch::Sender<u64>,
    /// `None` where mDNS would not start. [`Watcher::poke`] tries again.
    daemon: Mutex<Option<Running>>,
}

/// A daemon and the thread reading it.
struct Running {
    daemon: mdns_sd::ServiceDaemon,
    /// Cleared before this daemon is deliberately shut down.
    ///
    /// The drain thread ends when its channel closes, and that happens for two very different
    /// reasons: this watcher replaced the daemon on purpose, or it lost it. Only the second is worth
    /// a warning, and without a flag they are indistinguishable — which is how a dead watcher went
    /// unreported once already.
    live: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Watcher")
            .field("machines", &self.snapshot().len())
            .finish_non_exhaustive()
    }
}

impl Watcher {
    /// Starts listening.
    ///
    /// **It cannot fail**, on the reasoning [`super::browse`] already gives for answering with an
    /// empty list: no mDNS is an ordinary state, not a fault — a locked-down network, a container, a
    /// machine with the service switched off. A watcher that could not open a daemon answers
    /// nothing and lets [`poke`](Self::poke) try again later, which is the one thing a one-shot
    /// browse could never do: a network that was hostile at start-up recovers by itself.
    #[must_use]
    pub fn start() -> Self {
        let (generation, _) = tokio::sync::watch::channel(0);
        let watcher = Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(Registry::new()),
                generation,
                daemon: Mutex::new(None),
            }),
        };
        watcher.open();
        watcher
    }

    /// Opens the daemon and starts the thread, if there is not one already.
    fn open(&self) {
        let mut held = self
            .inner
            .daemon
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if held.is_some() {
            return;
        }
        let Some(daemon) = super::daemon() else {
            return;
        };
        let receiver = match daemon.browse(SERVICE_TYPE) {
            Ok(receiver) => receiver,
            Err(error) => {
                tracing::debug!(%error, "cannot browse for machines on this network");
                let _ = daemon.shutdown();
                return;
            }
        };
        let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
        *held = Some(Running {
            daemon,
            live: Arc::clone(&live),
        });
        drop(held);

        // **A thread and not a task.** `mdns-sd`'s receiver is a blocking `flume` one — its `async`
        // feature is off in the default build — this crate must not assume anything about the
        // runtime it is linked into, and `run_advertiser` next door already keeps its socket work
        // off the executor.
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("km-discovery-watch".to_owned())
            .spawn(move || drain(&inner, &receiver, &live));
        if let Err(error) = spawned {
            tracing::debug!(%error, "cannot start the discovery watcher");
        }
    }

    /// Throws the daemon away and starts a fresh one, keeping everything already known.
    ///
    /// **This is what a poke does, and a lighter touch was tried first and did not work.** Neither
    /// re-browsing (which clobbers the listener) nor
    /// [`verify`](mdns_sd::ServiceDaemon::verify) (which asks a daemon whose sockets may no longer
    /// be receiving) got a phone back onto a machine that had moved while the application was
    /// backgrounded — measured at a minute and counting, with the machine advertising throughout.
    ///
    /// A daemon binds its sockets per interface when it starts. On a phone that is the one thing
    /// most likely to have gone stale: the multicast lock was released and retaken, the Wi-Fi may
    /// have dropped, the network may be a different one entirely. Rebuilding asks all of those
    /// questions at once and costs a socket set and a thread, at the handful of moments a person has
    /// actually asked to look again.
    ///
    /// **The registry is deliberately kept.** What this device knows about a machine — its identity,
    /// and the address it was last at — is not invalidated by rebuilding a socket, and throwing it
    /// away would lose the cache that lets a remote open instantly.
    fn reopen(&self) {
        {
            let mut held = self
                .inner
                .daemon
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(running) = held.take() {
                // Marked dead first, so the thread it is about to end knows the ending was intended.
                running
                    .live
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                let _ = running.daemon.shutdown();
            }
        }
        self.open();
    }

    /// The machines on the network now. Instant, and never touches the network.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Sighting> {
        self.registry().sightings()
    }

    /// Everything heard from this run, present or not.
    #[must_use]
    pub fn observed(&self) -> Vec<Observed> {
        self.registry().observed()
    }

    /// One machine by its instance id, present or not.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<Observed> {
        self.registry().find(id)
    }

    /// A counter that moves whenever [`snapshot`](Self::snapshot) would answer differently.
    ///
    /// A counter rather than a stream of events, because no subscriber wants the events: they want
    /// *look again, something moved*. A re-announcement that changes nothing does not move it.
    #[must_use]
    pub fn changed(&self) -> tokio::sync::watch::Receiver<u64> {
        self.inner.generation.subscribe()
    }

    /// Ask the network again, now.
    ///
    /// What a button press, a resumed application or a connection that has just dropped is worth.
    /// `mdns-sd` backs its retransmissions off as a browse ages, so this is what puts a fresh query
    /// on the wire at the moment somebody has a reason to want one — and on Android it is what makes
    /// the multicast lock, retaken moments earlier, worth anything.
    ///
    /// It also re-opens a daemon that would not open before.
    ///
    /// # It re-verifies, and it must never re-browse
    ///
    /// **Calling `browse` a second time silently destroys this watcher.** `mdns-sd` keeps
    /// `service_queriers` as one listener *per service type* and `browse` **overwrites** it, so a
    /// second call replaces the sender feeding [`drain`]'s receiver. Dropping the throwaway receiver
    /// then closes the channel, the drain loop's `recv` fails, the thread exits — and the registry is
    /// frozen for the life of the process, answering from whatever it had heard before the first
    /// poke. Nothing reports it: snapshots keep working, they just stop changing.
    ///
    /// That was shipped, and a phone found it. Backgrounded, the multicast lock is released and the
    /// application is blind; the machine restarted on a new address in that window; on return the
    /// remote poked, killed its own watcher, and sat on the dead address for as long as it was
    /// watched. The desktop had passed the same feature because its tests never needed the registry
    /// to learn anything *after* a poke.
    ///
    /// So a poke asks the one question a live listener cannot answer for itself:
    /// [`verify`](mdns_sd::ServiceDaemon::verify), which is RFC 6762 §10.4 record verification —
    /// re-ask about a known instance by name, and flush it (with a `ServiceRemoved`) if nothing
    /// answers. **Machines this process has never heard of need no poke**: one that has just been
    /// switched on announces itself unsolicited, and the listener — now never clobbered — hears it.
    pub fn poke(&self) {
        self.reopen();
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.inner
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Best effort, and not waited on: this runs on the way out, and blocking a shutdown on a
        // multicast round trip is not a trade worth making — the same posture `Advert` takes.
        if let Ok(mut held) = self.daemon.lock()
            && let Some(running) = held.take()
        {
            running
                .live
                .store(false, std::sync::atomic::Ordering::Relaxed);
            let _ = running.daemon.shutdown();
        }
    }
}

/// Folds events into the registry until the daemon goes away.
///
/// **It says so when it stops**, which is the whole of what the previous version lacked. This loop
/// ending is indistinguishable from a quiet network — snapshots go on answering, they just stop
/// changing — so a `poke` that closed the channel froze discovery for the life of the process and
/// reported nothing. One line at `warn` is the difference between that and a log somebody can read.
fn drain(
    inner: &Arc<Inner>,
    receiver: &mdns_sd::Receiver<mdns_sd::ServiceEvent>,
    live: &Arc<std::sync::atomic::AtomicBool>,
) {
    while let Ok(event) = receiver.recv() {
        let changed = match event {
            mdns_sd::ServiceEvent::ServiceResolved(info) => {
                let announcement = Announcement::from_resolved(&info);
                let mut registry = inner
                    .registry
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                registry.observe_at(announcement, Instant::now())
            }
            mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                let mut registry = inner
                    .registry
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                registry.removed_at(&fullname, Instant::now())
            }
            _ => false,
        };
        if changed {
            inner.generation.send_modify(|generation| *generation += 1);
        }
    }
    if live.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!(
            "the discovery watcher has stopped listening; machines will not be noticed again in this run"
        );
    } else {
        tracing::debug!("the discovery watcher was replaced");
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use crate::connect::DEFAULT_PORT;

    /// One announcement from a machine with an `id`, carrying the addresses named.
    fn announced(id: &str, name: &str, addresses: &[Ipv4Addr]) -> Announcement {
        Announcement {
            key: id.to_owned(),
            fullname: format!("{name}._karaokemachine._tcp.local."),
            name: Some(name.to_owned()),
            id: Some(id.to_owned()),
            url: None,
            addresses: addresses.to_vec(),
            port: DEFAULT_PORT,
        }
    }

    fn now() -> Instant {
        Instant::now()
    }

    /// The reported bug, as a test with no network in it.
    ///
    /// A machine holding four addresses on four interfaces announces them one per packet, because
    /// `mdns-sd` only sends the addresses on the subnet a packet leaves by. `browse` used to take
    /// the first announcement and skip the rest, so the address shown was whichever interface
    /// answered first — here, the WSL switch.
    #[test]
    fn a_machine_announcing_one_address_at_a_time_is_still_one_machine() {
        let mut registry = Registry::new();
        // The order they arrive in is a race, so the awkward one is put first on purpose.
        for address in [
            Ipv4Addr::new(172, 28, 0, 1),
            Ipv4Addr::new(203, 0, 113, 9),
            Ipv4Addr::new(192, 168, 1, 42),
            Ipv4Addr::new(172, 17, 0, 1),
        ] {
            registry.observe_at(announced("abc123", "Living Room", &[address]), now());
        }
        let found = registry.sightings();
        assert_eq!(found.len(), 1, "one machine, not four");
        assert_eq!(found[0].url, "http://192.168.1.42:8177");
        assert_eq!(found[0].name, "Living Room");
        assert_eq!(found[0].id.as_deref(), Some("abc123"));
    }

    /// ...and the same machine's `url` record settles it whatever the addresses say.
    #[test]
    fn an_advertised_url_survives_being_merged_across_announcements() {
        let mut registry = Registry::new();
        registry.observe_at(
            Announcement {
                url: Some("http://192.168.1.42:8177".to_owned()),
                ..announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 42)])
            },
            now(),
        );
        // A later announcement over the WSL switch, carrying its address, no name and no url of its
        // own. What it does not say must not unsay what is already known.
        registry.observe_at(
            Announcement {
                name: None,
                ..announced("abc123", "Living Room", &[Ipv4Addr::new(172, 28, 0, 1)])
            },
            now(),
        );
        let found = registry.sightings();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "http://192.168.1.42:8177");
        assert_eq!(found[0].name, "Living Room");
    }

    #[test]
    fn two_machines_stay_two_machines_and_sort_by_name() {
        let mut registry = Registry::new();
        registry.observe_at(
            announced("zzz", "Sala", &[Ipv4Addr::new(192, 168, 1, 9)]),
            now(),
        );
        registry.observe_at(
            announced("aaa", "Bedroom", &[Ipv4Addr::new(192, 168, 1, 8)]),
            now(),
        );
        let found = registry.sightings();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Bedroom");
        assert_eq!(found[1].name, "Sala");
    }

    /// An older build advertising no `id` is keyed by its DNS-SD fullname instead, which is stable
    /// per service. Keying on the URL is not available here: the URL is the thing being worked out.
    #[test]
    fn a_machine_with_no_id_is_still_collapsed_across_its_announcements() {
        let mut registry = Registry::new();
        for address in [Ipv4Addr::new(172, 28, 0, 1), Ipv4Addr::new(192, 168, 1, 42)] {
            registry.observe_at(
                Announcement {
                    key: "Living Room._karaokemachine._tcp.local.".to_owned(),
                    id: None,
                    ..announced("unused", "Living Room", &[address])
                },
                now(),
            );
        }
        let found = registry.sightings();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "http://192.168.1.42:8177");
        assert_eq!(found[0].id, None);
    }

    /// A machine that resolved but had nothing usable in it is not a row.
    #[test]
    fn a_machine_with_no_address_and_no_url_is_not_listed() {
        let mut registry = Registry::new();
        registry.observe_at(announced("abc", "Living Room", &[]), now());
        assert!(registry.sightings().is_empty());
    }

    /// A departure takes the machine off the list and forgets nothing about it.
    #[test]
    fn a_service_removed_marks_a_machine_absent_and_keeps_what_is_known() {
        let mut registry = Registry::new();
        registry.observe_at(
            announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 42)]),
            now(),
        );
        assert!(registry.removed_at("Living Room._karaokemachine._tcp.local.", now()));

        assert!(registry.sightings().is_empty(), "not on the network now");
        let observed = registry.find("abc123").expect("still known about");
        assert!(!observed.present);
        assert_eq!(observed.sighting.url, "http://192.168.1.42:8177");
        assert_eq!(observed.sighting.name, "Living Room");
    }

    /// The bookkeeping that makes the test above possible: rows are keyed by id and departures
    /// arrive by fullname, so the fullname has to be kept.
    #[test]
    fn a_removal_finds_its_row_even_though_rows_are_keyed_by_id() {
        let mut registry = Registry::new();
        registry.observe_at(
            announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 42)]),
            now(),
        );
        assert!(
            !registry.removed_at("Bedroom._karaokemachine._tcp.local.", now()),
            "a departure for a machine we never saw changes nothing"
        );
        assert_eq!(registry.sightings().len(), 1);
    }

    /// Renaming a machine re-registers it under a new label with the same id, and a departure for
    /// the old label must still land on the row.
    #[test]
    fn a_renamed_machine_is_known_by_both_of_its_labels() {
        let mut registry = Registry::new();
        registry.observe_at(
            announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 42)]),
            now(),
        );
        registry.observe_at(
            announced("abc123", "Sala", &[Ipv4Addr::new(192, 168, 1, 42)]),
            now(),
        );
        assert_eq!(registry.sightings().len(), 1, "still one machine");
        assert!(
            registry.removed_at("Living Room._karaokemachine._tcp.local.", now()),
            "the label it used to answer to still names it"
        );
    }

    /// A machine that comes back is present again, and is not a second row.
    #[test]
    fn a_removal_followed_by_an_announcement_is_present_again() {
        let mut registry = Registry::new();
        let announcement = || announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 42)]);
        registry.observe_at(announcement(), now());
        registry.removed_at("Living Room._karaokemachine._tcp.local.", now());
        assert!(registry.observe_at(announcement(), now()), "a change");

        assert_eq!(registry.sightings().len(), 1);
        assert!(registry.find("abc123").expect("known").present);
    }

    /// The DHCP shuffle, which is the whole reason any of this exists.
    #[test]
    fn an_id_reappearing_at_a_new_address_is_one_row_at_the_new_address() {
        let mut registry = Registry::new();
        registry.observe_at(
            Announcement {
                url: Some("http://192.168.1.42:8177".to_owned()),
                ..announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 42)])
            },
            now(),
        );
        registry.observe_at(
            Announcement {
                url: Some("http://192.168.1.77:8177".to_owned()),
                ..announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 77)])
            },
            now(),
        );

        let found = registry.sightings();
        assert_eq!(found.len(), 1, "one machine that moved, not two machines");
        assert_eq!(found[0].url, "http://192.168.1.77:8177");
    }

    /// **The test that pins the decision to keep time off `Sighting`.** A machine re-announcing
    /// itself unchanged moves the clock and nothing else, so nothing that compares sightings — and
    /// nothing subscribed to the change signal — is disturbed by it.
    #[test]
    fn last_seen_moves_forward_without_changing_the_sighting() {
        let mut registry = Registry::new();
        let announcement = || announced("abc123", "Living Room", &[Ipv4Addr::new(192, 168, 1, 42)]);

        let earlier = Instant::now();
        assert!(registry.observe_at(announcement(), earlier), "the first is");
        let before = registry.sightings();
        let first_seen = registry.find("abc123").expect("known").first_seen;

        let later = earlier + std::time::Duration::from_secs(60);
        assert!(
            !registry.observe_at(announcement(), later),
            "saying the same thing again is not a change"
        );

        assert_eq!(before, registry.sightings(), "the sighting is untouched");
        let observed = registry.find("abc123").expect("known");
        assert_eq!(observed.first_seen, first_seen, "first_seen does not move");
        assert_eq!(observed.last_seen, later, "last_seen does");
    }

    /// Nothing heard from is not an error and does not wait.
    #[test]
    fn an_empty_registry_answers_at_once() {
        let registry = Registry::new();
        assert!(registry.sightings().is_empty());
        assert!(registry.observed().is_empty());
        assert!(registry.find("abc123").is_none());
    }
}
