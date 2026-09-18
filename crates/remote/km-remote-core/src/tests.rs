//! Tests for the entry point — the four phases, and the seams that make them portable.
//!
//! Every one of these runs with [`find::NoLocator`], deliberately. A suite that browsed the network
//! would be told a different story by whatever happened to be switched on in the next room, and the
//! machine-is-absent path is the one this program exists for anyway.
//!
//! **They all bind loopback, too**, which is not a style preference: a test binary that listens on
//! `0.0.0.0` raises a Windows Firewall dialog on every rebuild, because the rule that dialog writes
//! names a path whose hash has already changed. See "No test binds a non-loopback address" in
//! docs/ARCHITECTURE.md.

use super::*;
use std::{net::Ipv6Addr, path::Path};

use crate::testing::Scratch;

/// A config that never goes near the network, on a port the operating system picks.
fn offline(dir: &Path) -> Config {
    Config::new(dir)
        .with_bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .with_locator(Arc::new(find::NoLocator))
}

/// The data directory is the one thing with no default, and the rest have one.
///
/// There is no `Config::default()` to call — the type enforces that, and this records *why*, since
/// a later reader's obvious tidy-up is to add one.
#[test]
fn the_data_directory_is_the_only_thing_without_a_default() {
    let config = Config::new("/some/where");
    assert_eq!(config.data_dir, PathBuf::from("/some/where"));
    assert_eq!(config.bind.port(), DEFAULT_PORT);
    assert!(config.bind.ip().is_loopback());
    assert!(config.machine.is_none());
    assert!(!config.force_refresh);
}

/// Binding reports the port the socket really got, which is the whole point of phase one.
///
/// A port of 0 is what a phone wants — two copies of an app cannot argue over a fixed number — and
/// it is unanswerable unless the bind is separate from the serve.
#[tokio::test]
async fn binding_reports_the_port_the_socket_really_got() {
    let scratch = Scratch::new("bind");
    let bound = Bound::bind(&offline(&scratch.0)).await.expect("bind");

    assert_ne!(bound.address().port(), 0, "the socket kept a placeholder");
    assert_eq!(
        bound.url(),
        format!("http://127.0.0.1:{}/", bound.address().port())
    );
}

/// The URL is not the socket's address formatted, which is the whole point of [`Bound::url`].
///
/// Bound to `0.0.0.0` — the `--lan` case — the formatted address reads `0.0.0.0:8179`, which is a
/// perfectly good thing to bind and not a thing anything can connect to, so a webview handed it
/// would show an error page.
///
/// **The wildcard address is deliberately not what this binds**, and the reason is not about this
/// crate: on Windows a listener on a non-loopback address raises a Firewall prompt, and the rule
/// that prompt writes is keyed on the *full image path* — which for a test binary is
/// `target/debug/deps/<crate>-<hash>.exe`, a new path after every rebuild. So answering it buys
/// nothing and the dialog comes back for ever, once per rebuild and once per worktree. `::1` is
/// loopback, so nothing prompts, and it proves the same property.
///
/// It has to be some address, though, and that is why the loopback test above cannot do this job:
/// bound to `127.0.0.1`, the right answer and the bug — `format!("http://{address}/")` — are
/// character for character the same string.
#[tokio::test]
async fn the_url_is_not_the_socket_address_formatted() {
    let scratch = Scratch::new("not-the-address");
    let config = offline(&scratch.0).with_bind(SocketAddr::from((Ipv6Addr::LOCALHOST, 0)));
    let bound = Bound::bind(&config).await.expect("bind");

    assert!(bound.address().is_ipv6(), "the socket did not take ::1");
    assert_ne!(
        bound.url(),
        format!("http://{}/", bound.address()),
        "the url is the address formatted, so `--lan` would send a webview to 0.0.0.0"
    );
    assert!(
        bound.url().starts_with("http://127.0.0.1:"),
        "a browser was going to be sent to {}",
        bound.url()
    );
}

/// A first run with no machine and no mirror still opens.
///
/// **This is the offline app's central promise and nothing asserted it before.** Browsing,
/// searching and favorites all have to work with the karaoke machine switched off; if this ever
/// starts returning an error, the program has stopped being the thing it was built to be.
#[tokio::test]
async fn a_run_with_no_machine_and_no_mirror_still_opens() {
    let scratch = Scratch::new("cold");
    let config = offline(&scratch.0);
    let bound = Bound::bind(&config).await.expect("bind");
    let server = Server::open(bound, config).await.expect("open");

    let ready = server.ready();
    assert!(ready.machine.is_none(), "found a machine that cannot exist");
    assert_eq!(ready.songs, Ok(0));
    assert_eq!(ready.data_dir, scratch.0);
}

/// The data directory is created rather than required to exist.
#[tokio::test]
async fn the_data_directory_is_made_if_it_is_not_there() {
    let scratch = Scratch::new("mkdir");
    let nested = scratch.0.join("not").join("yet");
    let config = offline(&nested);
    let bound = Bound::bind(&config).await.expect("bind");
    let _server = Server::open(bound, config).await.expect("open");

    assert!(nested.join(crate::mirror::MIRROR_FILE).exists());
}

/// An injected shutdown stops the server, with no signal handler anywhere.
///
/// This is the seam a phone needs: a window closing, an `onDestroy` and a Ctrl-C are three hosts'
/// spellings of one idea, and only one of them is a signal.
#[tokio::test]
async fn an_injected_shutdown_stops_the_server() {
    let scratch = Scratch::new("stop");
    let config = offline(&scratch.0);
    let bound = Bound::bind(&config).await.expect("bind");
    let address = bound.address();
    let server = Server::open(bound, config).await.expect("open");

    let stop = Stop::default();
    let serving = tokio::spawn(server.serve(stop.clone().asked()));

    // It is really answering before it is asked to stop, or this would pass against a server that
    // never started.
    let page = reqwest::get(format!("http://127.0.0.1:{}/", address.port()))
        .await
        .expect("the remote should answer");
    assert!(page.status().is_success(), "got {}", page.status());

    stop.ask();
    tokio::time::timeout(std::time::Duration::from_secs(10), serving)
        .await
        .expect("the server did not stop when asked")
        .expect("the serving task panicked")
        .expect("serving failed");
}

/// Asking to stop before anybody is listening still stops it.
///
/// The reason `Stop` uses `notify_one` and not `notify_waiters`: the permit is stored. A window
/// closed during a long first import is exactly this case, and with `notify_waiters` it would hang.
#[tokio::test]
async fn a_stop_asked_for_too_early_is_not_lost() {
    let scratch = Scratch::new("early-stop");
    let config = offline(&scratch.0);
    let bound = Bound::bind(&config).await.expect("bind");
    let server = Server::open(bound, config).await.expect("open");

    let stop = Stop::default();
    stop.ask(); // before `serve` exists at all

    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        server.serve(stop.asked()),
    )
    .await
    .expect("a stop asked for before serving began was dropped")
    .expect("serving failed");
}

/// Warming up with no machine to talk to is a no-op rather than an error.
#[tokio::test]
async fn warming_up_without_a_machine_does_nothing_and_says_nothing() {
    let scratch = Scratch::new("warm");
    let config = offline(&scratch.0);
    let bound = Bound::bind(&config).await.expect("bind");
    let server = Server::open(bound, config).await.expect("open");

    server.warm_up().await;
    assert_eq!(server.ready().songs, Ok(0));
}
