//! Holding the port, rather than claiming it once.
//!
//! A listening socket is not something a process is handed and keeps. A phone or a tablet has its
//! application suspended when it leaves the screen, and the system destroys the socket while it is
//! away. An appliance loses one with the interface it was bound to. Either way what is left is a
//! machine that runs, draws its screen, shows an address, and answers nobody.
//!
//! [`HeldListener`] stands between that and the URL under the QR code. It is an
//! [`axum::serve::Listener`], so `axum::serve` drives it exactly as it drives a bare
//! [`tokio::net::TcpListener`], and everything that differs is in what happens when accepting stops
//! working: it takes the port again, and for as long as it cannot, the machine says so instead of
//! naming an address.
//!
//! The stock implementation for `TcpListener` cannot do this, and its own documentation says why
//! not in the trait: *if the underlying accept call can return an error, this function must take
//! care of logging and retrying*. It logs and retries for ever, against the same descriptor, which
//! is right for a full file table and wrong for a socket that has been destroyed.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::connect::{ConnectInfo, resolve};
use crate::server::ApiState;

/// How long the first retry waits after a rebind that failed.
const FIRST_WAIT: Duration = Duration::from_millis(200);

/// The longest any retry waits.
///
/// A machine whose network is arriving answers within a second or two of it being there, and a
/// machine whose port is held by another program needs somebody to fix that, so backing off
/// further buys nothing and costs the first case.
const LONGEST_WAIT: Duration = Duration::from_secs(5);

/// Asks the server to take its port again.
///
/// The host holds one of these when it knows something the accept loop cannot see. Returning to the
/// foreground is the case that matters: a system that destroyed the socket while the application
/// was away may leave a descriptor that never errors and never accepts, and no loop can tell that
/// apart from a quiet network.
///
/// **A `watch` channel rather than a `Notify`, and the reason is cancellation.** The accept loop
/// waits on a request and on an incoming connection at once, so whichever loses that race is
/// dropped part-way through. A dropped `Notified` can take its permit with it, which would lose the
/// single request that mattered. A `watch` carries a count, and looking at it and being dropped
/// changes nothing.
#[derive(Debug, Clone)]
pub struct Relisten {
    sender: Arc<watch::Sender<u64>>,
}

impl Relisten {
    /// A handle nobody has asked anything of yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sender: Arc::new(watch::channel(0).0),
        }
    }

    /// Tells the server its socket may not have survived, so it should take the port again.
    ///
    /// Cheap and non-blocking, because the callers are threads that must not wait: a platform's UI
    /// thread reaches this through the machine's watchdog poll.
    pub fn request(&self) {
        self.sender.send_modify(|count| *count += 1);
    }

    /// What [`HeldListener`] waits on.
    pub(crate) fn watcher(&self) -> watch::Receiver<u64> {
        self.sender.subscribe()
    }
}

impl Default for Relisten {
    fn default() -> Self {
        Self::new()
    }
}

/// A listener that reclaims its port when the system takes it away.
pub(crate) struct HeldListener {
    /// `None` only between dropping a socket and binding its replacement.
    listener: Option<TcpListener>,
    /// The address to bind again, which is the one actually held rather than the one asked for.
    ///
    /// **Never [`crate::config::ApiConfig::bind`].** A machine that asked for port 0 was given a
    /// real port, and by now that port is on the television, in a QR code and in whatever a phone
    /// remembered. Binding port 0 a second time answers with a different one, which would be the
    /// same fault this type exists to prevent wearing a better disguise.
    address: SocketAddr,
    state: ApiState,
    relisten: watch::Receiver<u64>,
}

impl HeldListener {
    /// Wraps a bound listener, to be handed straight to `axum::serve`.
    pub(crate) fn new(listener: TcpListener, address: SocketAddr, state: ApiState) -> Self {
        let relisten = state.relisten().watcher();
        Self {
            listener: Some(listener),
            address,
            state,
            relisten,
        }
    }

    /// Drops whatever socket is there and binds the address again, retrying until it works.
    ///
    /// Nothing here gives up. A machine that stops trying is a machine somebody has to notice and
    /// restart, and the whole point of this is that nobody was watching when the socket went.
    async fn take_the_port_again(&mut self, why: &str) {
        tracing::warn!(address = %self.address, why, "taking the port again");
        // Dropped before anything is bound, because one port does not hold two sockets: an explicit
        // request arrives while the old socket is perfectly alive, and binding first would fail on
        // the very case it was raised for.
        drop(self.listener.take());
        self.state.set_listening(false);
        self.publish_failure(why);

        let mut wait = FIRST_WAIT;
        loop {
            match TcpListener::bind(self.address).await {
                Ok(listener) => {
                    self.listener = Some(listener);
                    self.state.set_listening(true);
                    self.republish_address().await;
                    tracing::info!(address = %self.address, "the machine has its port again");
                    return;
                }
                Err(error) => {
                    tracing::warn!(address = %self.address, %error, "the port is not free yet");
                    self.publish_failure(&error.to_string());
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(LONGEST_WAIT);
                }
            }
        }
    }

    /// Puts the reason on the screen, in place of an address that answers nothing.
    fn publish_failure(&self, why: &str) {
        self.state
            .set_connect_info(ConnectInfo::server_failed(why, self.address.port()));
    }

    /// Works the reachable address out again, so the panel stops saying the server failed.
    ///
    /// Off the runtime for the reason [`crate::server::CONNECT_REFRESH`]'s task gives: `resolve`
    /// enumerates the machine's interfaces, which is a blocking syscall and not a cheap one.
    async fn republish_address(&self) {
        let address = self.address;
        let factory_password = self.state.factory_password();
        match tokio::task::spawn_blocking(move || resolve(address, factory_password)).await {
            Ok(resolved) => self.state.set_connect_info(resolved),
            // The refresher publishes a fresh one within its own period, so a panic inside
            // `if_addrs` costs a stale panel rather than a permanent one.
            Err(_) => tracing::warn!("the reachable address could not be worked out again"),
        }
    }
}

impl axum::serve::Listener for HeldListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            // **Biased, so a request is read before a connection.** The two are not equally urgent:
            // a connection arriving on a socket the host has just told us to distrust is one the
            // replacement will get anyway, and the ordering makes the request's effect immediate
            // rather than dependent on the network being quiet.
            tokio::select! {
                biased;

                () = requested(&mut self.relisten) => {
                    self.take_the_port_again("the host asked for the port again").await;
                }

                result = accept_on(self.listener.as_ref()) => match result {
                    Ok(accepted) => return accepted,
                    // One connection died on the way in. The socket is fine and the next caller is
                    // unaffected, so this is not news.
                    Err(error) if is_connection_error(&error) => {}
                    Err(error) => {
                        let why = error.to_string();
                        self.take_the_port_again(&why).await;
                    }
                },
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        match self.listener.as_ref() {
            Some(listener) => listener.local_addr(),
            // Only reachable inside a rebind, and the stored address is the truthful answer there:
            // it is the one being bound, and the one every URL already names.
            None => Ok(self.address),
        }
    }
}

/// Waits until the host asks for the port again.
///
/// Waits for ever once nothing holds the other end, rather than resolving: the request can never
/// come, and a ready arm would spin the accept loop.
async fn requested(watcher: &mut watch::Receiver<u64>) {
    if watcher.changed().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Accepts on the socket, or waits for ever when there is not one.
///
/// The `None` arm cannot be reached through [`axum::serve::Listener::accept`], which replaces the
/// socket before it yields. It is written as a wait rather than an `expect` because the alternative
/// to a wait here is a panic in the one place whose entire job is surviving a socket that went
/// away.
async fn accept_on(listener: Option<&TcpListener>) -> std::io::Result<(TcpStream, SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}

/// Whether the error belongs to one arriving connection rather than to the socket.
///
/// The same set `axum` and `hyper` treat as harmless, kept here because theirs is private.
fn is_connection_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionRefused | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
    )
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use axum::serve::Listener as _;

    use super::*;
    use crate::config::ApiConfig;
    use crate::testing::TestMachine;

    fn state() -> ApiState {
        ApiState::from_machine(
            TestMachine::with_catalog(1).shared(),
            ApiConfig::default().on_ephemeral_port(),
        )
    }

    /// Binds loopback on a port the OS chooses, which is the rule every test in this crate follows:
    /// a test that binds a routable address raises a firewall prompt per build on Windows.
    async fn bound() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("loopback on an ephemeral port binds");
        let address = listener
            .local_addr()
            .expect("a bound socket has an address");
        (listener, address)
    }

    /// Connects, so `accept` has something to return.
    async fn knock(address: SocketAddr) {
        tokio::spawn(async move {
            let _ = TcpStream::connect(address).await;
        });
    }

    #[tokio::test]
    async fn a_request_takes_the_same_port_again() {
        let (listener, address) = bound().await;
        let state = state();
        let relisten = state.relisten();
        let mut held = HeldListener::new(listener, address, state.clone());

        relisten.request();
        knock(address).await;
        let (_stream, _peer) = held.accept().await;

        // The whole reason the bound address is carried rather than the requested one. A rebind
        // that moved the port would leave every QR code already on screen pointing nowhere.
        assert_eq!(
            held.local_addr().expect("the replacement is bound"),
            address
        );
        assert!(state.is_listening());
    }

    #[tokio::test]
    async fn a_socket_that_went_away_is_replaced_and_serves_again() {
        let (listener, address) = bound().await;
        let state = state();
        let mut held = HeldListener::new(listener, address, state.clone());

        // What the system does to a suspended application, done deliberately: the socket is gone
        // and the address is free.
        drop(held.listener.take());

        state.relisten().request();
        knock(address).await;
        let (_stream, _peer) = held.accept().await;

        assert_eq!(
            held.local_addr().expect("the replacement is bound"),
            address
        );
        assert!(state.is_listening());
    }

    #[tokio::test]
    async fn the_screen_says_the_server_failed_while_the_port_is_gone() {
        let (listener, address) = bound().await;
        let state = state();
        let mut held = HeldListener::new(listener, address, state.clone());

        // Held by something else, so the rebind cannot succeed and the machine stays down.
        let squatter = {
            drop(held.listener.take());
            TcpListener::bind(address).await.expect("the port is free")
        };

        let rebinding = tokio::spawn(async move {
            held.take_the_port_again("the socket went away").await;
            held
        });

        // The panel has to be telling the truth while this is going on, which is the half of the
        // fault that made it invisible.
        let failed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let info = state.connect_info();
                if matches!(
                    info.problem,
                    Some(crate::ConnectProblem::ServerFailed { .. })
                ) {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the failure reaches the panel");

        assert!(!failed.reachable);
        assert!(failed.urls.is_empty());
        assert_eq!(failed.port, address.port());
        assert!(!state.is_listening());

        // Let go, and it comes back on its own without anybody restarting anything.
        drop(squatter);
        let held = tokio::time::timeout(Duration::from_secs(10), rebinding)
            .await
            .expect("the rebind finishes once the port is free")
            .expect("the task did not panic");

        assert_eq!(
            held.local_addr().expect("the replacement is bound"),
            address
        );
        assert!(state.is_listening());
        // Not "no problem at all": these tests bind loopback, and a machine on loopback reports
        // `LoopbackOnly` for ever, which is the honest answer and a different sentence entirely.
        // What has to be gone is the failure.
        assert!(!matches!(
            state.connect_info().problem,
            Some(crate::ConnectProblem::ServerFailed { .. })
        ));
    }

    #[tokio::test]
    async fn one_refused_connection_is_not_a_reason_to_rebind() {
        let (listener, address) = bound().await;
        let state = state();
        let mut held = HeldListener::new(listener, address, state.clone());

        assert!(is_connection_error(&std::io::Error::from(
            ErrorKind::ConnectionReset
        )));
        assert!(!is_connection_error(&std::io::Error::from(
            ErrorKind::InvalidInput
        )));

        // The socket is untouched, so an ordinary connection still arrives on the original.
        knock(address).await;
        let (_stream, _peer) = held.accept().await;
        assert_eq!(held.local_addr().expect("still bound"), address);
    }
}
