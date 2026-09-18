//! Driving the offline remote's server from a host that is not a `main`.
//!
//! **The whole crate compiles and is tested on every platform**, which is the point of it existing
//! apart from any one shell. Everything here is the part that is easy to get wrong — an idempotent
//! start, a stop that must not block the caller, and a superseded run that must not overwrite the
//! state of the run that replaced it — and none of it needs a phone to exercise. Each shell's `ffi`
//! is then thin enough to read: a string conversion, a guard against unwinding, and a call in here.
//!
//! # A crate rather than a module, because there are two mobile shells
//!
//! This was `km-remote-android`'s `state.rs` while there was one of them. `km-remote-ios` needs the
//! identical thing — the same four phases, the same generation counter, the same non-blocking stop
//! — and the only honest ways to give it that were a second copy or a crate. The same bargain
//! `km-androidlog` made when there were two Android applications, and the one
//! `km-songbook` and `km-tray` made before it.
//!
//! **What the shells keep** is a calling convention and nothing else: JNI on one side, C on the
//! other, over six functions that mean the same six things.
//!
//! # The one thing a host must decide: which [`Locator`]
//!
//! [`start`] takes one rather than letting [`Config::new`] pick, and that is the seam this crate
//! could not have expressed as a `#[cfg(target_os = …)]`. Android passes a real `find::Mdns`,
//! because Java holds a `WifiManager.MulticastLock` for as long as the Activity is on screen. iOS
//! passes `find::Sweep`, because Apple grants the multicast entitlement only after a manual review
//! and a browse without it finds nothing *and reports success*. A test passes `find::NoLocator`.
//!
//! It is also why [`serve`] needs no `#[cfg(test)]` line substituting `NoLocator` to keep
//! `cargo test` from asking a Windows developer's firewall on every relink. That fix wears a `cfg`
//! it should never have needed: **the host chooses**, and a test is a host.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use km_remote_core::find::Locator;
use km_remote_core::{Bound, Config, Server, Stop};

/// How long a stopping runtime is given to wind down before its threads are abandoned.
///
/// Five seconds is the same allowance the desktop shell gives its own shutdown. It is a ceiling
/// rather than a wait: [`stop`] returns immediately either way, because the only caller is an
/// Activity's `onDestroy` on the main looper.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Loopback, and a port the operating system picks.
///
/// **Not configurable, and not `0.0.0.0`.** The server inside a phone exists for that phone's
/// WebView; binding the Wi-Fi interface would put an unauthenticated remote control on the network
/// for anyone in range to find. Port 0 because a fixed one collides with whatever else is listening,
/// and [`Bound::address`] is how the answer gets back — which is the second of the core's four
/// seams, and the reason this shell has no port to configure.
const BIND: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);

/// What a run owns, and what a later run has to be able to take away from it.
#[derive(Default)]
struct State {
    /// `None` while nothing is running. Dropping this blocks, so it is never dropped under the lock.
    runtime: Option<tokio::runtime::Runtime>,
    stop: Option<Stop>,
    /// Counts starts. A run compares the generation it was born with against this before writing
    /// anything, so a run that was abandoned rather than stopped cannot clear its successor's state.
    generation: u64,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

/// The bound port, or 0 until the server is answering on it.
///
/// An atomic rather than a field on [`State`], because the host polls this every hundred
/// milliseconds while [`start`] may be holding the lock.
static PORT: AtomicU16 = AtomicU16::new(0);

/// How many songs the mirror holds, or -1 if it could not be counted.
static SONGS: AtomicI32 = AtomicI32::new(-1);

fn failure_slot() -> &'static Mutex<Option<String>> {
    static FAILURE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    FAILURE.get_or_init(Mutex::default)
}

/// The client for the machine, which is how [`machine`] answers.
///
/// **A client rather than the address `Server::open` found, and that distinction is the whole
/// point.** An `Option<String>` written once by [`publish`] out of `Ready::machine` is a snapshot of
/// what phase 2 turned up; `Server::machine_watch` re-points the client every time it finds a
/// machine afterwards, and none of that reaches a snapshot, so the answer is frozen for the life of
/// the run.
///
/// What that costs is the "no machine found" screen on both mobile shells: each polls [`machine`]
/// waiting for one to appear, and each waits for ever while its own log shows the server finding a
/// machine twenty seconds later, mirroring the catalog and connecting. Seen on an iPad and
/// confirmed on Android — the same fault, in the one place both shells share, which is the argument
/// for this crate existing.
fn client_slot() -> &'static Mutex<Option<km_remote_core::client::MachineClient>> {
    static MACHINE: OnceLock<Mutex<Option<km_remote_core::client::MachineClient>>> =
        OnceLock::new();
    MACHINE.get_or_init(Mutex::default)
}

/// Brings the server up and returns at once.
///
/// A no-op if one is already running, which is what makes it safe to call from an Activity that may
/// be recreated — a rotation, a return from the background — without checking first.
///
/// `machine` is an address somebody typed, or `None` to try what was remembered and then the
/// network. **Naming one pins it**: the core never wanders away from a machine it was told about,
/// however long it stays silent, so a host offering this must also offer a way to clear it.
///
/// `locator` is how this run looks on the network, and it is the host's decision rather than this
/// crate's — see the module header. It is taken even when `machine` names an address, because a
/// pinned machine is a `find::locate` short-circuit rather than a promise never to browse.
pub fn start(data_dir: PathBuf, machine: Option<String>, locator: Arc<dyn Locator>) {
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    if guard.runtime.is_some() {
        tracing::debug!("a server is already running; leaving it alone");
        return;
    }

    // Cleared here rather than on the way out, so that the failure of the previous run stays
    // readable right up until somebody asks for another one. A host showing a failure screen is
    // reading this while it waits for the user to press Try again.
    *failure_slot().lock().unwrap_or_else(|e| e.into_inner()) = None;
    PORT.store(0, Ordering::SeqCst);
    SONGS.store(-1, Ordering::SeqCst);
    *client_slot().lock().unwrap_or_else(|e| e.into_inner()) = None;

    // **Multi-thread and `enable_all`, and neither is a style choice.** Every SQLite call in the
    // mirror and the favorites goes through `spawn_blocking`, and so does every `Locator::browse`,
    // so the runtime must have a blocking pool; and the server needs the IO and time drivers.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            record_failure(format!("could not start the async runtime: {error}"));
            return;
        }
    };

    guard.generation = guard.generation.wrapping_add(1);
    let generation = guard.generation;
    let stop = Stop::default();
    guard.stop = Some(stop.clone());

    runtime.spawn(serve(data_dir, machine, locator, stop, generation));
    guard.runtime = Some(runtime);
}

/// The four phases, in order, on the runtime [`start`] just built.
///
/// Split out so `start` reads as bookkeeping and this reads as the sequence the core's own
/// documentation describes.
async fn serve(
    data_dir: PathBuf,
    machine: Option<String>,
    locator: Arc<dyn Locator>,
    stop: Stop,
    generation: u64,
) {
    // The locator is the host's, never `Config::new`'s default. Android hands over a real
    // `find::Mdns`, because Java holds the multicast lock for as long as the Activity is on screen;
    // iOS hands over `find::Sweep`, because it may not multicast at all; a test hands over
    // `find::NoLocator`, which is what a suite running on somebody's home network needs so that
    // whatever is switched on in the next room cannot tell it a different story.
    let config = Config::new(data_dir)
        .with_bind(BIND)
        .with_machine(machine)
        .with_locator(locator);

    // Phase 1. The port is knowable from here, but nothing answers on it until phase 4 — so it is
    // deliberately *not* published yet. A WebView pointed at a bound-but-unserved port gets a
    // connection it cannot explain.
    let bound = match Bound::bind(&config).await {
        Ok(bound) => bound,
        Err(error) => {
            return finish(
                generation,
                format!("could not listen on loopback: {error:#}"),
            );
        }
    };
    let port = bound.address().port();

    // Phase 2. The databases, and one look for a machine. Milliseconds, unless the browse runs.
    let server = match Server::open(bound, config).await {
        Ok(server) => server,
        Err(error) => return finish(generation, format!("could not open the remote: {error:#}")),
    };

    {
        let ready = server.ready();
        tracing::info!(
            url = %ready.url,
            data_dir = %ready.data_dir.display(),
            machine = ready.machine.as_ref().map_or("none", |m| m.url.as_str()),
            "the remote is ready"
        );
        if !publish(generation, port, ready, server.machine().clone()) {
            return;
        }
    }

    // Phase 3. The catalog refresh, behind the pages rather than in front of them. This is what
    // makes a first run usable while it is still importing, and it is why this shell needs no
    // progress bar and no long startup deadline.
    server.spawn_warm_up();

    // Phase 4.
    if let Err(error) = server.serve(stop.asked()).await {
        finish(generation, format!("the remote stopped: {error:#}"));
    }
}

/// Publishes a ready server's details, unless this run has already been superseded.
///
/// Returns whether it is still the current run — a `false` means everything below the caller belongs
/// to whoever replaced it.
fn publish(
    generation: u64,
    port: u16,
    ready: &km_remote_core::Ready,
    client: km_remote_core::client::MachineClient,
) -> bool {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    if guard.generation != generation {
        tracing::debug!("superseded before serving; leaving the current run's state alone");
        return false;
    }
    SONGS.store(
        ready
            .songs
            .as_ref()
            .map_or(-1, |n| i32::try_from(*n).unwrap_or(i32::MAX)),
        Ordering::SeqCst,
    );
    // The **client**, not `ready.machine`. See [`client_slot`] for what reading the snapshot cost.
    *client_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(client);
    // Last, and that ordering is the contract: a host polls the port and reads the rest once it is
    // non-zero, so everything it might read has to be in place before the port appears.
    PORT.store(port, Ordering::SeqCst);
    true
}

/// Records why a run ended, unless it had already been replaced.
fn finish(generation: u64, reason: String) {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    if guard.generation != generation {
        // Superseded while winding down. Saying nothing here is the whole job: a late failure from
        // an abandoned run that cleared the live run's port would leave the host looking at a server
        // it had already been told about.
        tracing::debug!(reason, "a superseded run ended; not reporting it");
        return;
    }
    drop(guard);
    tracing::error!(reason, "the remote is not running");
    record_failure(reason);
    PORT.store(0, Ordering::SeqCst);
}

fn record_failure(reason: String) {
    *failure_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
}

/// Asks the server to stop, and **returns without waiting for it**.
///
/// The waiting is what this deliberately does not do. Dropping a `tokio` runtime blocks until its
/// blocking tasks finish, and what is on a blocking thread here is a network browse or an HTTP
/// request to a machine that may have been switched off — so a caller on Android's main looper would
/// be frozen for as long as that takes, at the exact moment the user asked the app to go away. The
/// runtime is handed to a thread of its own instead, which gives it [`SHUTDOWN_GRACE`] and then
/// abandons whatever is left.
///
/// The bookkeeping *does* happen on the calling thread, and that part is not optional: the port must
/// read 0 before this returns, or a host coming straight back can load a page against a server that
/// is being torn down.
pub fn stop() {
    let (runtime, stop) = {
        let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
        // Bumped even though nothing is replacing this run: it is what tells the four phases, if
        // they are still in flight, that their results belong to nobody.
        guard.generation = guard.generation.wrapping_add(1);
        (guard.runtime.take(), guard.stop.take())
    };
    PORT.store(0, Ordering::SeqCst);
    SONGS.store(-1, Ordering::SeqCst);
    *client_slot().lock().unwrap_or_else(|e| e.into_inner()) = None;

    let Some(runtime) = runtime else { return };
    if let Some(stop) = stop {
        stop.ask();
    }
    // A thread of its own, which has to outlive the Android component that asked for the stop —
    // that is the whole reason the runtime is not simply dropped here.
    //
    // The result is discarded on purpose. If a thread cannot be spawned the closure is dropped
    // inside `spawn`, taking the runtime with it, and this call blocks for as long as the wind-down
    // takes. That is the degenerate case and it is still the right one: blocking briefly beats
    // leaking a listening socket, and a process that cannot spawn a thread has larger problems.
    let spawned = std::thread::Builder::new()
        .name("km-remote-shutdown".into())
        .spawn(move || {
            runtime.shutdown_timeout(SHUTDOWN_GRACE);
            tracing::info!("the remote has stopped");
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "no thread for the shutdown; it happened inline");
    }
}

/// The port the remote is answering on, or 0 until it is.
#[must_use]
pub fn port() -> u16 {
    PORT.load(Ordering::SeqCst)
}

/// How many songs this device's copy of the catalog holds, or -1 if it is not known yet.
///
/// Zero is a real answer and an important one: it is a first run that has found no machine and has
/// nothing to show, which is the one moment a host has to offer somewhere to type an address.
#[must_use]
pub fn songs() -> i32 {
    SONGS.load(Ordering::SeqCst)
}

/// The machine this run is talking to **now**, or `None` — which is ordinary, not a failure.
///
/// Asked of the live client on every call rather than read out of a slot somebody filled in once.
/// A host polls this while it is showing a "no machine found" screen, so the whole value of the
/// function is that the answer can *change*; see [`client_slot`] for what it cost when it could
/// not.
#[must_use]
pub fn machine() -> Option<String> {
    let client = client_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()?;
    client.api().map(|api| api.base().to_owned())
}

/// Why the remote stopped, or `None` while it is healthy.
#[must_use]
pub fn failure() -> Option<String> {
    failure_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data directory of this test's own, cleaned up on the way out — or on the way in, next time.
    ///
    /// The same shape as `km-remote-core`'s own `Scratch`, and for the same reason: nothing in this
    /// workspace depends on `tempfile`, and one struct is cheaper than a dependency.
    ///
    /// **The one test here cannot be cleaned up by its own `Drop`**, which is why there is a sweep as
    /// well. The directory is handed to [`start`], so the runtime holds it and releases its SQLite
    /// files when that shuts down — after every local in the test body has gone. Windows refuses to
    /// remove a file another handle still has open, so the removal fails and the directory waits for
    /// a later process to take it. The sweep is what makes the growth one run's worth rather than
    /// unbounded.
    struct Scratch(PathBuf);

    /// The prefix these are named with, and what the sweep recognizes its own by.
    const SCRATCH_PREFIX: &str = "km-remote-host-";

    /// How old a directory has to be before the sweep will take it: longer than any run of this
    /// suite, so a checkout building beside this one keeps its own.
    const SCRATCH_STALE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

    impl Scratch {
        fn new(name: &str) -> Self {
            sweep_once();
            let dir = std::env::temp_dir().join(format!(
                "{SCRATCH_PREFIX}{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Takes the directories an earlier run could not remove, once per process.
    fn sweep_once() {
        static SWEPT: std::sync::Once = std::sync::Once::new();
        SWEPT.call_once(|| {
            let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
                return;
            };
            for entry in entries.flatten() {
                let stale = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .map(|at| at.elapsed().unwrap_or_default() > SCRATCH_STALE)
                    .unwrap_or(false);
                if stale
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(SCRATCH_PREFIX)
                {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        });
    }

    /// What a test looks for on the network, which is nothing.
    ///
    /// **A host chooses its locator, and a test is a host — there is nothing here for a
    /// `#[cfg(test)]` inside `serve` to decide.** `find::Mdns::browse` reaches
    /// `mdns_sd::ServiceDaemon::new()`, which binds `0.0.0.0:5353` and `[::]:5353` on every
    /// interface — so a Windows developer is asked by the Firewall on every relink, the rule it
    /// writes naming a `deps/km_remote_host-<hash>.exe` whose hash has already changed.
    fn quiet() -> Arc<dyn Locator> {
        Arc::new(km_remote_core::find::NoLocator)
    }

    /// Waits for the port, or for a failure, the way a host does.
    fn await_port() -> u16 {
        for _ in 0..600 {
            let port = port();
            if port != 0 {
                return port;
            }
            assert!(failure().is_none(), "the server failed: {:?}", failure());
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the server never bound");
    }

    /// The whole life cycle, and the fact that it can be lived twice.
    ///
    /// **These share one process-wide state, so they are one test rather than several.** Separate
    /// `#[test]` functions run on threads of one binary and would each be starting and stopping the
    /// same singleton; the interleaving would be the thing under test rather than the behavior.
    #[test]
    fn a_server_starts_stops_and_starts_again() {
        let scratch = Scratch::new("lifecycle");

        start(scratch.0.clone(), None, quiet());
        let first = await_port();
        assert_ne!(first, 0, "a started server must report a port");
        assert!(failure().is_none(), "a healthy start reports no failure");

        // Idempotent: the second call must leave the first run alone rather than build a second
        // runtime, which is what makes it safe to call from an Activity that gets recreated.
        start(scratch.0.clone(), None, quiet());
        assert_eq!(port(), first, "a second start must not disturb the first");

        // The mirror is empty and no machine was found, which is the ordinary first-run state and
        // emphatically not a failure — it is the case the host has to tell apart from a broken one.
        assert_eq!(songs(), 0, "an empty mirror counts zero songs");
        assert!(machine().is_none(), "no machine is ordinary");

        stop();
        assert_eq!(port(), 0, "the port must read zero before stop returns");
        assert_eq!(songs(), -1, "a stopped server knows nothing");

        // The one that matters: a stop must leave the shell startable. The Go remote this follows
        // had a guard against double-starting become a guard against ever starting again, and Try
        // again then did nothing for the rest of the process's life.
        start(scratch.0.clone(), None, quiet());
        let second = await_port();
        assert_ne!(second, 0, "the shell must still be startable after a stop");
        stop();

        // Stopping what is already stopped is a no-op rather than a panic. Reached in practice by
        // an `onDestroy` for an Activity whose `start` never got anywhere.
        stop();
        assert_eq!(port(), 0);
    }
}
