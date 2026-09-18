//! Carrying the reachable address from the API to the display.
//!
//! Both crates have a `ConnectInfo`, deliberately. `km-api` owns
//! *resolution* — enumerating interfaces, ranking them, deciding what is reachable. `km-display` owns
//! *presentation* — what words go on screen, and the QR code. Neither may depend on the other:
//! pointing the display at the API drags `tokio` and `axum` into the render crate, and pointing the
//! API at the display drags in SDL.
//!
//! So the mapping lives here, in the one crate that already depends on both, and
//! `docs/ARCHITECTURE.md` records the trade.
//!
//! [`own_url`] and [`reachable_url`] are two further questions neither of those crates should
//! answer, and they are not one question. `F11` opens a browser on this box, so it asks *how does
//! this box reach its own server*. The icon in the bar names an address for a phone in somebody's
//! hand, so it asks *where is the machine* and takes the first only where there is no answer to the
//! second. Both live here because both are that same fact read a further way.

use std::net::{Ipv4Addr, SocketAddr};

/// The address to use for this machine's own server, from a machine standing on it.
///
/// **A different question from the one [`to_display`] answers, and the difference is who is
/// asking.** The panel on the television is read by somebody holding a phone, so it names an
/// address that phone can route to; this is read by a browser on the box itself, where the shortest
/// true answer is loopback and the LAN address may not exist at all.
///
/// Three cases, and the middle one is why this is not simply `127.0.0.1`:
///
/// * **Unspecified** (`0.0.0.0`, the shipped default) — the listener is on every interface,
///   loopback included, so `127.0.0.1` reaches it and reaches it without touching the network.
/// * **A specific address** — the listener is on that one and **nothing is listening on loopback**,
///   so `127.0.0.1` would be refused. `--api-bind 192.168.1.42:8177` is a real thing somebody
///   types, and the shipped `api.bind` is a settings key an owner may narrow.
/// * **Loopback already** — itself, unchanged.
///
/// Deliberately not `ConnectInfo::primary_url`, which is the *phone's* answer: on `0.0.0.0` with no
/// network its `urls` are empty and it returns `None`, which would refuse a browser that could have
/// been opened on a perfectly working machine.
pub fn reachable_from_here(bound: SocketAddr) -> SocketAddr {
    if bound.ip().is_unspecified() {
        SocketAddr::from((Ipv4Addr::LOCALHOST, bound.port()))
    } else {
        bound
    }
}

/// This machine's own base URL, or `None` when there is no server to open.
///
/// `None` only ever means [`km_api::ConnectInfo::bound`] is absent, which happens in exactly one
/// place — `ConnectInfo::server_failed`, where the listener never bound at all. That is a refusal
/// worth making rather than papering over: the caller has the problem text to say why.
pub fn own_url(api: &km_api::ConnectInfo) -> Option<String> {
    let bound: SocketAddr = api.bound.as_ref()?.parse().ok()?;
    Some(format!("http://{}", reachable_from_here(bound)))
}

/// The address to hand somebody who is not standing at this machine, or `None` when there is no
/// server to name.
///
/// **[`own_url`]'s question the other way round, and the icon in the bar is why.** That one is read
/// by a browser on this box, where loopback is the shortest true answer; this one is read by
/// somebody about to type it into a phone, or about to open the singer's remote for a room the
/// machine is streaming to — and `127.0.0.1` is no use to either.
///
/// So [`km_api::ConnectInfo::primary_url`] first, which is the ranked address the television's panel
/// and the QR code already name, and `own_url` beneath it. **The fallback is the whole reason this
/// is not `primary_url` alone**: a machine with no network has empty `urls` and would leave the menu
/// naming nothing, where an address only this box can use is at least an address.
///
/// **The icon in the bar is the only caller, and it is behind two features**, so a build with
/// neither has this and no use for it — which is the same shape `km_tray::tooltip` carries on the
/// platforms with no icon bar. The rule belongs beside `own_url` whatever a build turns on: the two
/// are one question asked from two sides, and splitting them across files is how they come to
/// disagree.
#[cfg_attr(not(all(feature = "video", feature = "tray")), allow(dead_code))]
pub fn reachable_url(api: &km_api::ConnectInfo) -> Option<String> {
    api.primary_url()
        .map(str::to_owned)
        .or_else(|| own_url(api))
}

/// Translates the API's resolved address into what the display should show.
pub fn to_display(api: &km_api::ConnectInfo, pin: Option<&str>) -> km_display::ConnectInfo {
    km_display::ConnectInfo {
        urls: api.urls.clone(),
        bound: api.bound.clone(),
        reachable: api.reachable,
        problem: api.problem.as_ref().map(to_display_problem),
        // The PIN itself never crosses the network -- this is read from the machine's own settings,
        // beside it, which is why the display can show what `/discover` deliberately will not.
        //
        // **`api.factory_password` deliberately does not come across.** It is the same fact as this
        // being `Some` -- the machine keeps the plain PIN exactly as long as the generated password
        // is the one in force -- and the panel has nothing to say about a door the singer's remote
        // does not have. The bit has its jobs on `/discover` and in the owner's own tools.
        factory_pin: pin.map(str::to_owned),
        // Whether there is a browser to open is an operating system's fact and `km-display` draws
        // for five of them, so it is handed the answer rather than asked for one. See
        // `crate::display::BROWSER_KEY_HINT`, which is narrower than `WEB_BROWSER` on purpose.
        browser_key: crate::display::BROWSER_KEY_HINT,
    }
}

fn to_display_problem(problem: &km_api::ConnectProblem) -> km_display::ConnectProblem {
    match problem {
        km_api::ConnectProblem::LoopbackOnly => km_display::ConnectProblem::LoopbackOnly,
        km_api::ConnectProblem::NoNetwork => km_display::ConnectProblem::NoNetwork,
        km_api::ConnectProblem::ServerFailed { message } => {
            km_display::ConnectProblem::ServerFailed(message.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use km_api::connect::{Candidate, resolve_from};

    use super::*;

    fn bound(ip: Ipv4Addr, port: u16) -> SocketAddr {
        SocketAddr::from((ip, port))
    }

    #[test]
    fn a_reachable_machine_carries_every_url_in_order() {
        let api = resolve_from(
            bound(Ipv4Addr::UNSPECIFIED, 8177),
            &[
                Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi"),
                Candidate::new(Ipv4Addr::new(10, 0, 0, 7), "Ethernet"),
            ],
            false,
        );
        let display = to_display(&api, None);
        assert!(display.reachable);
        assert_eq!(display.primary_url(), Some("http://192.168.1.42:8177"));
        // The runners-up survive the mapping. The panel no longer names them — it says how many
        // there are — but they are still what the API publishes and what the startup log lists, and
        // dropping them here would take them out of both.
        assert_eq!(display.urls.len(), 2);
        assert_eq!(display.factory_pin, None);
    }

    #[test]
    fn loopback_only_becomes_the_panel_that_says_so() {
        let api = resolve_from(bound(Ipv4Addr::LOCALHOST, 8177), &[], false);
        let panel = to_display(&api, None).panel(km_locale::Locale::English);
        assert!(panel.is_problem);
        assert_eq!(panel.url, None);
        assert!(panel.headline.contains("local-only"));
        // The fix is on screen, not in a log nobody reads.
        assert!(
            panel
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("0.0.0.0"))
        );
    }

    #[test]
    fn no_network_becomes_the_panel_that_says_that_instead() {
        let api = resolve_from(bound(Ipv4Addr::UNSPECIFIED, 8177), &[], false);
        let panel = to_display(&api, None).panel(km_locale::Locale::English);
        assert!(panel.is_problem);
        assert!(panel.headline.contains("No network"));
    }

    #[test]
    fn a_server_that_would_not_start_carries_its_error_verbatim() {
        let api =
            km_api::ConnectInfo::server_failed("Address already in use (os error 10048)", 8177);
        let panel = to_display(&api, None).panel(km_locale::Locale::English);
        // Whoever is standing at the machine is the person who can fix a port conflict; a shrug
        // helps them not at all.
        assert_eq!(
            panel.detail.as_deref(),
            Some("Address already in use (os error 10048)")
        );
    }

    /// The PIN is what crosses to the screen, and a machine on a factory password without one says
    /// nothing rather than demanding a password the remote will never ask for.
    #[test]
    fn a_pin_is_carried_through_and_a_bare_password_is_not() {
        let api = resolve_from(bound(Ipv4Addr::new(192, 168, 1, 42), 8177), &[], true);

        let named = to_display(&api, Some("482913"));
        assert_eq!(named.factory_pin.as_deref(), Some("482913"));
        let detail = named
            .panel(km_locale::Locale::English)
            .detail
            .expect("the PIN is on the screen");
        assert!(detail.contains("482913"), "got {detail}");

        // `api.factory_password` is true here and buys nothing: one address, no PIN, no detail.
        assert_eq!(
            to_display(&api, None)
                .panel(km_locale::Locale::English)
                .detail,
            None
        );
    }

    #[test]
    fn a_machine_on_every_interface_reaches_itself_over_loopback() {
        // The shipped default, and the case that makes loopback the right answer rather than a
        // guess: the listener is on `127.0.0.1` too, so this needs no network at all.
        let api = resolve_from(
            bound(Ipv4Addr::UNSPECIFIED, 8177),
            &[Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi")],
            false,
        );
        assert_eq!(own_url(&api).as_deref(), Some("http://127.0.0.1:8177"));
    }

    #[test]
    fn a_machine_on_one_address_is_opened_at_that_address() {
        // The case loopback gets wrong, and it is not exotic: `--api-bind 192.168.1.42:8177` is a
        // thing somebody types, and nothing is listening on 127.0.0.1 afterwards.
        let api = resolve_from(bound(Ipv4Addr::new(192, 168, 1, 42), 9000), &[], false);
        assert_eq!(own_url(&api).as_deref(), Some("http://192.168.1.42:9000"));
    }

    #[test]
    fn a_loopback_machine_is_opened_where_it_is() {
        let api = resolve_from(bound(Ipv4Addr::LOCALHOST, 8177), &[], false);
        assert_eq!(own_url(&api).as_deref(), Some("http://127.0.0.1:8177"));
    }

    #[test]
    fn a_machine_with_no_network_can_still_be_opened_on_this_box() {
        // The whole reason this is not `primary_url`. There is no reachable address for a phone, so
        // `urls` is empty and the panel says "No network" -- and the server is running perfectly
        // well, so a browser on this box has somewhere to go.
        let api = resolve_from(bound(Ipv4Addr::UNSPECIFIED, 8177), &[], false);
        assert_eq!(api.primary_url(), None);
        assert_eq!(own_url(&api).as_deref(), Some("http://127.0.0.1:8177"));
    }

    #[test]
    fn a_server_that_never_bound_has_nothing_to_open() {
        let api = km_api::ConnectInfo::server_failed("Address already in use", 8177);
        assert_eq!(own_url(&api), None);
    }

    /// The case the icon in the bar exists for, and `own_url`'s answer to the same machine is
    /// loopback -- which is the pair of assertions worth keeping side by side.
    #[test]
    fn the_address_to_hand_over_is_the_one_a_phone_can_route_to() {
        let api = resolve_from(
            bound(Ipv4Addr::UNSPECIFIED, 8177),
            &[Candidate::new(Ipv4Addr::new(192, 168, 1, 42), "Wi-Fi")],
            false,
        );
        assert_eq!(
            reachable_url(&api).as_deref(),
            Some("http://192.168.1.42:8177")
        );
        assert_eq!(own_url(&api).as_deref(), Some("http://127.0.0.1:8177"));
    }

    /// The fallback, and why this is not `primary_url` alone: there is no address a phone can use,
    /// and a menu naming nothing is worse than one naming the machine's own.
    #[test]
    fn a_machine_with_no_network_falls_back_to_its_own_address() {
        let api = resolve_from(bound(Ipv4Addr::UNSPECIFIED, 8177), &[], false);
        assert_eq!(api.primary_url(), None);
        assert_eq!(
            reachable_url(&api).as_deref(),
            Some("http://127.0.0.1:8177")
        );
    }

    #[test]
    fn a_machine_on_one_address_is_handed_over_at_that_address() {
        let api = resolve_from(bound(Ipv4Addr::new(192, 168, 1, 42), 9000), &[], false);
        assert_eq!(
            reachable_url(&api).as_deref(),
            Some("http://192.168.1.42:9000")
        );
    }

    /// A loopback bind has one address and it is both answers, so the two agree here by arithmetic
    /// rather than by choice.
    #[test]
    fn a_loopback_machine_hands_over_the_only_address_it_has() {
        let api = resolve_from(bound(Ipv4Addr::LOCALHOST, 8177), &[], false);
        assert_eq!(
            reachable_url(&api).as_deref(),
            Some("http://127.0.0.1:8177")
        );
    }

    #[test]
    fn a_server_that_never_bound_has_nothing_to_hand_over_either() {
        let api = km_api::ConnectInfo::server_failed("Address already in use", 8177);
        assert_eq!(reachable_url(&api), None);
    }
}
