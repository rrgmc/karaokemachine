//! How the server is set up.
//!
//! Everything here comes from `settings.json`. The defaults are the safe end of each trade rather
//! than the convenient one: bound to loopback, debugging off, and no admin routes reachable.
//!
//! **A shipped machine always has a password**, but that is a fact about `settings.json` rather than
//! about this struct: the settings layer generates a factory PIN at first start and hands the hash
//! down. An [`ApiConfig`] built by hand has none, and then every route under `/api/v1/admin/` simply
//! refuses everybody — which is the right failure, and the one a machine with no password has to
//! make: the alternative is an admin surface wide open to the house.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use crate::auth::DEFAULT_TOKEN_TTL;
use crate::connect::DEFAULT_PORT;

/// The server's configuration.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// What to listen on. `0.0.0.0` to let a phone connect.
    pub bind: SocketAddr,
    /// The `argon2` hash of the admin password.
    ///
    /// `None` only on a hand-built config: a real machine's settings always carry one.
    pub admin_password_hash: Option<String>,
    /// Whether that password is the one the machine generated for itself.
    ///
    /// Reported by `/discover` so an owner's tools can nag, and drawn on the machine's own screen
    /// beside the PIN. **The PIN itself never appears here** — this is the one bit that may travel.
    pub factory_password: bool,
    /// Bumped to invalidate every outstanding admin token. See [`crate::auth::AdminAuth::set_epoch`].
    pub session_epoch: u64,
    /// How long an admin token lasts.
    pub token_ttl: Duration,
    /// Whether the `debug.` section of settings does anything, and whether the two debug routes are
    /// mounted at all.
    ///
    /// Off means they are not there — a 404, not a 401. See `routes::DEBUG_SURFACE`.
    pub debug_enabled: bool,
    /// What to call this machine in a discovery listing.
    pub machine_name: String,
    /// What language this machine speaks.
    ///
    /// **Beside `machine_name` because it is the same kind of fact**: something the owner set about
    /// this machine, not about a request. Every route whose body is prose reads it from here, and
    /// the song book is the only one so far.
    pub locale: km_locale::Locale,
    /// This machine's stable instance id. `km-app` persists it.
    pub instance_id: String,
    /// Whether to advertise over mDNS.
    pub advertise_mdns: bool,
    /// Whether to serve the development remote at `/dev/`.
    pub serve_dev_remote: bool,
    /// Where the development remote's files are.
    pub dev_remote_dir: Option<PathBuf>,
    /// Origins allowed to call the API cross-origin.
    ///
    /// Empty by default, and that is the point: the remote is served from this same origin, so CORS
    /// never enters into it. The setting exists for developing a remote against a separate dev
    /// server, which is a developer's machine and not a product configuration.
    pub cors_origins: Vec<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT),
            admin_password_hash: None,
            factory_password: false,
            session_epoch: 0,
            token_ttl: DEFAULT_TOKEN_TTL,
            debug_enabled: false,
            machine_name: "KaraokeMachine".to_owned(),
            locale: km_locale::Locale::default(),
            instance_id: crate::discover::new_instance_id(),
            advertise_mdns: true,
            // **Off, in every build.** The dev remote is a development tool; serving it would put a
            // control panel on a product surface. It used to follow `debug_assertions`, which said
            // the same thing about a release build and the wrong thing about a debug one — a
            // checkout build is still a machine on somebody's LAN, and a default that differs
            // between the two is a default that gets tested in only one of them.
            serve_dev_remote: false,
            dev_remote_dir: None,
            cors_origins: Vec::new(),
        }
    }
}

impl ApiConfig {
    /// Listens on every interface, so a phone can reach it.
    pub fn on_all_interfaces(mut self, port: u16) -> Self {
        self.bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        self
    }

    /// Listens on an ephemeral loopback port. For tests that want a real server.
    pub fn on_ephemeral_port(mut self) -> Self {
        self.bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        self
    }

    /// Sets the admin password, hashing it.
    pub fn with_password(mut self, password: &str) -> Result<Self, argon2::password_hash::Error> {
        self.admin_password_hash = Some(crate::auth::AdminAuth::hash_password(password)?);
        Ok(self)
    }

    /// Turns debugging mode on, mounting the two debug routes.
    pub fn with_debugging(mut self) -> Self {
        self.debug_enabled = true;
        self
    }

    /// Turns the mDNS advertisement off.
    pub fn without_mdns(mut self) -> Self {
        self.advertise_mdns = false;
        self
    }

    /// Serves the development console from a directory.
    ///
    /// **Turns debugging on as well**, because the console needs both switches — see
    /// [`crate::routes::dev_console_served`]. A builder whose name says *with the dev console* and
    /// that produced a config not serving it would be a trap, and it was one for exactly as long as
    /// it took to write this: the test that drives the directory arm asserted against a string the
    /// landing page also carries, so it passed while serving the landing page.
    pub fn with_dev_remote(mut self, dir: impl Into<PathBuf>) -> Self {
        self.serve_dev_remote = true;
        self.debug_enabled = true;
        self.dev_remote_dir = Some(dir.into());
        self
    }

    /// Serves the built-in copy of the development console, with no directory anywhere.
    ///
    /// The state a staged build is in, and the twin of [`Self::with_dev_remote`] for the arm that
    /// has no folder to point at.
    pub fn with_dev_console(mut self) -> Self {
        self.serve_dev_remote = true;
        self.debug_enabled = true;
        self.dev_remote_dir = None;
        self
    }

    /// Whether a password is set at all.
    ///
    /// **Not a permission input.** A route under `/api/v1/admin/` demands a token whatever this
    /// says, and a machine with no hash refuses every login rather than opening every door.
    pub fn admin_configured(&self) -> bool {
        self.admin_password_hash.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_closed_to_the_network_rather_than_open() {
        let config = ApiConfig::default();
        assert!(config.bind.ip().is_loopback());
        assert_eq!(config.bind.port(), DEFAULT_PORT);
    }

    /// The state that used to open every admin route now closes them all. A config with no hash
    /// cannot verify a token, so `/api/v1/admin/...` refuses everybody rather than admitting
    /// everybody — which was the whole point of the change.
    #[test]
    fn a_config_with_no_password_shuts_the_admin_routes_rather_than_opening_them() {
        let config = ApiConfig::default();
        assert!(!config.admin_configured());
        let auth = crate::auth::AdminAuth::default();
        assert!(!auth.verify("anything at all"));
        assert!(auth.issue().is_none());
    }

    #[test]
    fn debugging_is_off_until_somebody_asks_for_it() {
        assert!(!ApiConfig::default().debug_enabled);
        assert!(ApiConfig::default().with_debugging().debug_enabled);
    }

    #[test]
    fn a_hand_built_config_does_not_claim_a_factory_password() {
        assert!(!ApiConfig::default().factory_password);
        assert_eq!(ApiConfig::default().session_epoch, 0);
    }

    #[test]
    fn opening_to_the_lan_is_one_deliberate_call() {
        let config = ApiConfig::default().on_all_interfaces(9000);
        assert!(config.bind.ip().is_unspecified());
        assert_eq!(config.bind.port(), 9000);
    }

    #[test]
    fn the_password_itself_is_nowhere_in_the_config() {
        let config = ApiConfig::default()
            .with_password("let me in")
            .expect("hash");
        assert!(config.admin_configured());
        let hash = config.admin_password_hash.expect("a hash");
        assert!(!hash.contains("let me in"));
        assert!(hash.starts_with("$argon2"));
    }

    #[test]
    fn two_machines_do_not_share_an_instance_id() {
        assert_ne!(
            ApiConfig::default().instance_id,
            ApiConfig::default().instance_id
        );
    }

    #[test]
    fn cors_is_off_until_somebody_asks_for_it() {
        assert!(ApiConfig::default().cors_origins.is_empty());
    }
}
