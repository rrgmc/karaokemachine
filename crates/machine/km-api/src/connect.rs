//! Working out the address a phone should actually type.
//!
//! The bind address is not the reachable address. Bound to `0.0.0.0` the machine has to discover its
//! own LAN addresses, and on a real desktop it has several: the Wi-Fi card, an Ethernet port nobody
//! plugged in, a Hyper-V switch, a VPN tunnel, WSL. Exactly one of them is where the phone is, and
//! showing the wrong one is worse than showing nothing — it sends somebody to a page that will never
//! load and gives them no clue why.
//!
//! So this module ranks rather than guesses, keeps the runners-up so the display can list them, and
//! when there is no answer says which of the several possible reasons applies. The ranking is a pure
//! function over a list of candidates, so the ugly real-world cases are testable with no network.
//!
//! IPv4 only, deliberately. A URL a human has to read off a screen or a QR code has to fit and has
//! to be typeable; an IPv6 literal in brackets is neither, and a link-local one needs a scope id
//! that means nothing on the phone.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde::{Deserialize, Serialize};

/// The port the machine listens on unless told otherwise.
///
/// Unassigned by IANA and easy to say out loud, which matters when somebody is reading it off a
/// television across the room.
pub const DEFAULT_PORT: u16 = 8177;

/// Why the web UI cannot be reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectProblem {
    /// Bound to loopback, so only this machine can connect.
    LoopbackOnly,
    /// No usable network interface.
    NoNetwork,
    /// The machine is not listening: a port already in use, or a socket it is taking again.
    ///
    /// Covers both ends of a server's life, because what the person in the room can do about them is
    /// the same. A port held by something else is the startup case. A socket the system destroyed
    /// under a running process is the other, and [`crate::listener`] is what puts this here while it
    /// takes the port back. The `message` is what separates them.
    ServerFailed {
        /// The actual error, to be shown verbatim. Whoever is standing at the machine is the
        /// person who can fix a port conflict, and a shrug helps them not at all.
        message: String,
    },
}

/// Where the web UI is, and whether it can actually be reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectInfo {
    /// Reachable URLs, best first. Empty when there is no usable address.
    pub urls: Vec<String>,
    /// The address the server bound to, for diagnosis.
    pub bound: Option<String>,
    /// The port, so a client can rebuild a URL for an address it prefers.
    pub port: u16,
    /// Whether a remote can actually connect.
    pub reachable: bool,
    /// Why not, when it cannot.
    pub problem: Option<ConnectProblem>,
    /// Whether the machine is still on the password it generated for itself.
    pub factory_password: bool,
}

impl ConnectInfo {
    /// The address a remote should use.
    pub fn primary_url(&self) -> Option<&str> {
        self.urls.first().map(String::as_str)
    }

    /// The addresses that are not the preferred one — what the display lists small underneath.
    pub fn alternate_urls(&self) -> &[String] {
        self.urls.get(1..).unwrap_or_default()
    }

    /// The machine has no server a phone could reach.
    pub fn server_failed(message: impl Into<String>, port: u16) -> Self {
        Self {
            urls: Vec::new(),
            bound: None,
            port,
            reachable: false,
            problem: Some(ConnectProblem::ServerFailed {
                message: message.into(),
            }),
            factory_password: false,
        }
    }
}

/// One address this machine holds, reduced to what the ranking needs.
///
/// A plain struct rather than `if_addrs::Interface` so the interesting logic can be tested against
/// the awkward configurations — a Hyper-V switch, a VPN tunnel, an unplugged Ethernet port — that
/// are the whole reason this module exists and that no CI machine will ever have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The address.
    pub ip: Ipv4Addr,
    /// The interface's name, used to demote the obvious virtual adapters.
    pub interface: String,
    /// Whether it is point-to-point — a tunnel rather than a network.
    pub p2p: bool,
    /// Whether the interface is operationally up.
    pub up: bool,
}

impl Candidate {
    /// A plain, up, non-tunnel interface.
    pub fn new(ip: Ipv4Addr, interface: impl Into<String>) -> Self {
        Self {
            ip,
            interface: interface.into(),
            p2p: false,
            up: true,
        }
    }
}

/// Interface-name fragments that mark an adapter as virtual.
///
/// Matched case-insensitively as substrings. These are the adapters that are up, have a perfectly
/// valid private address, and are never where the phone is. On a Windows developer's machine there
/// are usually three of them and they sort before the Wi-Fi card, which is how "the app showed me an
/// address that does not work" happens.
const VIRTUAL_MARKERS: &[&str] = &[
    "vethernet",
    "hyper-v",
    "virtualbox",
    "vmware",
    "vmnet",
    "docker",
    "wsl",
    "tailscale",
    "zerotier",
    "utun",
    "tun",
    "tap",
    // macOS names its virtual-machine bridges `bridge100`, `bridge101`, and Windows names a manual
    // one "Network Bridge". Both are up, both carry a perfectly valid `192.168` address, and
    // neither is where the phone is. **Found by the second caller of this list rather than the
    // first**: `km_remote_core::find::sweep_networks` ranks the networks it will walk, and on a Mac
    // with a VM installed it put `bridge100` ahead of the Wi-Fi card. Note this deliberately does
    // not match Linux's `br0`, which is very often the real LAN on a hypervisor host.
    "bridge",
    "loopback",
    "pseudo-interface",
];

/// Whether an interface name looks like a virtual adapter.
pub fn looks_virtual(interface: &str) -> bool {
    let lowered = interface.to_ascii_lowercase();
    VIRTUAL_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// How good an address is on the strength of its number alone. Lower is better.
///
/// Home routers hand out 192.168/16 far more often than anything else, so that is the address a
/// phone on the same Wi-Fi almost certainly shares a subnet with. 10/8 is next (larger homes, some
/// ISP routers), then 172.16/12 — which is last among the private ranges precisely because Docker
/// and Hyper-V live there.
///
/// **Split out from [`rank_of`] so that a caller holding nothing but an address can use it.** That
/// is [`crate::discover::browse`]: mDNS hands back a set of addresses with no interface names
/// attached, so the `p2p` and virtual-adapter parts of the full ranking are unavailable there and
/// this is the whole of what can be judged. The judgment itself lives here, once — a second copy in
/// `discover` would be free to disagree about which private range Hyper-V is on.
pub fn rank_of_address(ip: Ipv4Addr) -> u32 {
    match ip.octets() {
        [192, 168, _, _] => 0,
        [10, _, _, _] => 1,
        [172, second, _, _] if (16..=31).contains(&second) => 2,
        // A public address on the machine itself. Legitimate on some setups, but a phone reaching
        // it is not the common case, so it goes last.
        _ => 3,
    }
}

/// How good an address is as "the one to put on screen". Lower is better.
fn rank_of(candidate: &Candidate) -> u32 {
    let class = rank_of_address(candidate.ip);
    // The multipliers keep the tiers from crossing: a virtual 192.168 address must still lose to a
    // real 172.16 one, because the phone is on the real network.
    class
        + if candidate.p2p { 100 } else { 0 }
        + if looks_virtual(&candidate.interface) {
            10
        } else {
            0
        }
}

/// Orders candidates best-first and drops the unusable ones.
///
/// Dropped: interfaces that are down, loopback, and link-local (`169.254/16`, which means DHCP
/// failed and nothing else can reach it).
pub fn rank(candidates: &[Candidate]) -> Vec<Ipv4Addr> {
    let mut usable: Vec<&Candidate> = candidates
        .iter()
        .filter(|candidate| candidate.up)
        .filter(|candidate| !candidate.ip.is_loopback())
        .filter(|candidate| !candidate.ip.is_link_local())
        .filter(|candidate| !candidate.ip.is_unspecified())
        .collect();
    // Ties broken by address, so the order is stable across runs and the display does not reshuffle
    // itself every thirty seconds.
    usable.sort_by_key(|candidate| (rank_of(candidate), candidate.ip.octets()));
    usable.dedup_by_key(|candidate| candidate.ip);
    usable.into_iter().map(|candidate| candidate.ip).collect()
}

/// Every IPv4 address this machine holds.
pub fn interfaces() -> Vec<Candidate> {
    let Ok(found) = if_addrs::get_if_addrs() else {
        // An enumeration failure and a machine with no network are the same thing to the person
        // looking at the screen: no address to show. It is reported as `NoNetwork`.
        return Vec::new();
    };
    found
        .into_iter()
        .filter_map(|interface| {
            let IpAddr::V4(ip) = interface.ip() else {
                return None;
            };
            Some(Candidate {
                ip,
                p2p: interface.is_p2p,
                up: interface.is_oper_up(),
                interface: interface.name,
            })
        })
        .collect()
}

/// Resolves the reachable address of a live server.
pub fn resolve(bound: SocketAddr, factory_password: bool) -> ConnectInfo {
    resolve_from(bound, &interfaces(), factory_password)
}

/// The resolution, against a supplied candidate list.
///
/// Split out so the awkward cases are testable. Three shapes:
///
/// * bound to loopback — say so and say how to fix it, never invent a LAN URL;
/// * bound to one specific address — that address is the answer, whatever else the machine holds;
/// * bound to `0.0.0.0` — rank what we found, or report no network.
pub fn resolve_from(
    bound: SocketAddr,
    candidates: &[Candidate],
    factory_password: bool,
) -> ConnectInfo {
    let port = bound.port();
    let bound_text = Some(bound.to_string());

    if bound.ip().is_loopback() {
        return ConnectInfo {
            // A loopback URL is still the truth, and on a machine used as its own remote it works.
            // It is listed, but `reachable` is false so the display leads with the explanation
            // rather than an address a phone cannot use.
            urls: vec![format!("http://{}:{port}", bound.ip())],
            bound: bound_text,
            port,
            reachable: false,
            problem: Some(ConnectProblem::LoopbackOnly),
            factory_password,
        };
    }

    if !bound.ip().is_unspecified() {
        return ConnectInfo {
            urls: vec![format!("http://{}:{port}", bound.ip())],
            bound: bound_text,
            port,
            reachable: true,
            problem: None,
            factory_password,
        };
    }

    let ranked = rank(candidates);
    if ranked.is_empty() {
        return ConnectInfo {
            urls: Vec::new(),
            bound: bound_text,
            port,
            reachable: false,
            problem: Some(ConnectProblem::NoNetwork),
            factory_password,
        };
    }

    ConnectInfo {
        urls: ranked
            .into_iter()
            .map(|ip| format!("http://{ip}:{port}"))
            .collect(),
        bound: bound_text,
        port,
        reachable: true,
        problem: None,
        factory_password,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(ip: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::from(ip), port))
    }

    fn any(port: u16) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))
    }

    #[test]
    fn a_home_wifi_address_beats_everything_else_on_a_developers_machine() {
        // The configuration that motivates this module: a real Wi-Fi card behind three virtual
        // adapters, all up, all with valid private addresses.
        let candidates = [
            Candidate::new(Ipv4Addr::new(172, 22, 176, 1), "vEthernet (WSL)"),
            Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi"),
            Candidate::new(Ipv4Addr::new(172, 17, 0, 1), "docker0"),
            Candidate {
                ip: Ipv4Addr::new(100, 84, 1, 9),
                interface: "Tailscale".to_owned(),
                p2p: true,
                up: true,
            },
        ];
        let ranked = rank(&candidates);
        assert_eq!(ranked[0], Ipv4Addr::new(192, 168, 1, 42));
        // The tunnel sorts last, since a phone on the living-room Wi-Fi is not on it.
        assert_eq!(ranked.last(), Some(&Ipv4Addr::new(100, 84, 1, 9)));
    }

    #[test]
    fn a_real_interface_in_a_late_range_still_beats_a_virtual_one_in_an_early_range() {
        let candidates = [
            Candidate::new(
                Ipv4Addr::new(192, 168, 56, 1),
                "VirtualBox Host-Only Network",
            ),
            Candidate::new(Ipv4Addr::new(172, 20, 10, 3), "Wi-Fi"),
        ];
        // 172.16/12 ranks below 192.168/16 on address alone, but the 192.168 one is a VirtualBox
        // adapter, and the phone is on the Wi-Fi. This is the case a naive sort gets wrong.
        assert_eq!(rank(&candidates)[0], Ipv4Addr::new(172, 20, 10, 3));
    }

    #[test]
    fn ten_dot_beats_one_seventy_two() {
        let candidates = [
            Candidate::new(Ipv4Addr::new(172, 16, 5, 5), "eth1"),
            Candidate::new(Ipv4Addr::new(10, 0, 0, 7), "eth0"),
        ];
        assert_eq!(rank(&candidates)[0], Ipv4Addr::new(10, 0, 0, 7));
    }

    #[test]
    fn unusable_addresses_are_dropped_rather_than_ranked_low() {
        let candidates = [
            Candidate::new(Ipv4Addr::LOCALHOST, "Loopback"),
            // DHCP failed. Nothing can reach this.
            Candidate::new(Ipv4Addr::new(169, 254, 13, 9), "Ethernet"),
            // An unplugged Ethernet port keeps its last address on some systems.
            Candidate {
                ip: Ipv4Addr::new(192, 168, 9, 9),
                interface: "Ethernet 2".to_owned(),
                p2p: false,
                up: false,
            },
            Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi"),
        ];
        assert_eq!(rank(&candidates), [Ipv4Addr::new(192, 168, 1, 42)]);
    }

    #[test]
    fn the_same_address_on_two_interfaces_is_listed_once() {
        let candidates = [
            Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi"),
            Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi 2"),
        ];
        assert_eq!(rank(&candidates).len(), 1);
    }

    /// The case this ranking exists for: a Windows box with WSL and Hyper-V on it.
    ///
    /// Such a machine advertises three addresses over mDNS — a real LAN one plus `172.17.0.1` and
    /// `172.28.0.1`, the last two virtual adapters nothing outside the computer can reach — and
    /// `discover::browse` has only the numbers to go on, no interface names. This is the whole of
    /// what it can judge, so it is worth stating as a test rather than leaving to `rank_of`'s tests,
    /// which all supply an interface.
    #[test]
    fn a_real_lan_address_outranks_a_virtual_adapters_one_on_number_alone() {
        let lan = rank_of_address(Ipv4Addr::new(192, 168, 1, 42));
        for virtual_adapter in [Ipv4Addr::new(172, 17, 0, 1), Ipv4Addr::new(172, 28, 0, 1)] {
            assert!(
                lan < rank_of_address(virtual_adapter),
                "{virtual_adapter} should not beat a 192.168 address"
            );
        }
    }

    #[test]
    fn ranking_is_stable_so_the_display_does_not_reshuffle() {
        let candidates = [
            Candidate::new(Ipv4Addr::new(192, 168, 1, 90), "b"),
            Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "a"),
        ];
        assert_eq!(rank(&candidates), rank(&candidates));
        assert_eq!(rank(&candidates)[0], Ipv4Addr::new(192, 168, 1, 42));
    }

    #[test]
    fn virtual_adapter_names_are_recognized_however_they_are_cased() {
        assert!(looks_virtual("vEthernet (Default Switch)"));
        assert!(looks_virtual("VMware Network Adapter VMnet8"));
        assert!(looks_virtual("Tailscale"));
        assert!(looks_virtual("utun4"));
        assert!(looks_virtual("bridge100"), "macOS names a VM bridge this");
        assert!(
            looks_virtual("Network Bridge"),
            "and Windows names one this"
        );
        assert!(!looks_virtual("Wi-Fi"));
        assert!(!looks_virtual("Ethernet"));
        assert!(!looks_virtual("en0"));
        assert!(
            !looks_virtual("br0"),
            "a Linux bridge is very often the real LAN on a hypervisor host"
        );
    }

    #[test]
    fn binding_to_loopback_says_so_instead_of_inventing_a_lan_url() {
        let info = resolve_from(
            addr([127, 0, 0, 1], DEFAULT_PORT),
            &[Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi")],
            false,
        );
        assert!(!info.reachable);
        assert_eq!(info.problem, Some(ConnectProblem::LoopbackOnly));
        // Crucially: the LAN address the machine happens to hold is NOT offered, because the server
        // is not listening on it.
        assert_eq!(info.urls, ["http://127.0.0.1:8177"]);
    }

    #[test]
    fn binding_to_one_address_makes_that_the_answer() {
        let info = resolve_from(
            addr([192, 168, 1, 42], 9000),
            // Even with a "better" candidate present: the server is not listening on it.
            &[Candidate::new(Ipv4Addr::new(10, 0, 0, 1), "eth0")],
            true,
        );
        assert!(info.reachable);
        assert_eq!(info.urls, ["http://192.168.1.42:9000"]);
        assert!(info.factory_password);
        assert_eq!(info.problem, None);
    }

    #[test]
    fn binding_to_everything_with_no_network_reports_no_network() {
        let info = resolve_from(any(DEFAULT_PORT), &[], false);
        assert!(!info.reachable);
        assert_eq!(info.problem, Some(ConnectProblem::NoNetwork));
        assert!(info.urls.is_empty());
        // The bound address is still reported: it is what diagnosis starts from.
        assert_eq!(info.bound.as_deref(), Some("0.0.0.0:8177"));
    }

    #[test]
    fn only_loopback_and_link_local_present_is_still_no_network() {
        let info = resolve_from(
            any(DEFAULT_PORT),
            &[
                Candidate::new(Ipv4Addr::LOCALHOST, "Loopback"),
                Candidate::new(Ipv4Addr::new(169, 254, 1, 1), "Ethernet"),
            ],
            false,
        );
        assert_eq!(info.problem, Some(ConnectProblem::NoNetwork));
    }

    #[test]
    fn the_runners_up_are_kept_for_the_display_to_list() {
        let info = resolve_from(
            any(DEFAULT_PORT),
            &[
                Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi"),
                Candidate::new(Ipv4Addr::new(10, 0, 0, 7), "Ethernet"),
            ],
            false,
        );
        assert_eq!(info.primary_url(), Some("http://192.168.1.42:8177"));
        assert_eq!(info.alternate_urls(), ["http://10.0.0.7:8177"]);
    }

    #[test]
    fn one_address_has_no_alternates() {
        let info = resolve_from(
            any(DEFAULT_PORT),
            &[Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi")],
            false,
        );
        assert!(info.alternate_urls().is_empty());
    }

    #[test]
    fn a_failed_server_reports_the_actual_error() {
        let info = ConnectInfo::server_failed("address already in use", DEFAULT_PORT);
        assert!(!info.reachable);
        assert_eq!(
            info.problem,
            Some(ConnectProblem::ServerFailed {
                message: "address already in use".to_owned()
            })
        );
        assert!(info.urls.is_empty());
        assert_eq!(info.port, DEFAULT_PORT);
    }

    #[test]
    fn enumerating_this_machine_never_panics() {
        // Whatever this machine's network looks like, resolution produces an answer with a stated
        // reason. Asserting anything about the addresses themselves would be asserting about CI.
        let info = resolve(any(DEFAULT_PORT), false);
        assert_eq!(info.reachable, info.problem.is_none());
        if info.reachable {
            assert!(
                info.primary_url()
                    .is_some_and(|url| url.starts_with("http://"))
            );
        }
    }
}
