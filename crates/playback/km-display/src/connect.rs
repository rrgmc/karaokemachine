//! Telling the user where the web UI is.
//!
//! A remote nobody can find is a remote that does not exist, so the address is a first-class element
//! of the display rather than a line in a log. Two halves: what to say, and a QR code so a phone can
//! reach it without anybody typing an IP address.
//!
//! The failure states matter as much as the success one. A wrong URL on screen is worse than none —
//! it sends somebody to a page that will not load and gives them no idea why — so each way this can
//! fail has its own honest message.

/// Where the web UI is, and whether it can actually be reached.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectInfo {
    /// Reachable URLs, best first. Empty when there is no usable address.
    pub urls: Vec<String>,
    /// The address the server bound to, for diagnosis.
    pub bound: Option<String>,
    /// Whether a remote can actually connect.
    pub reachable: bool,
    /// Why not, when it cannot.
    pub problem: Option<ConnectProblem>,
    /// That generated PIN, when it is still the one in force.
    ///
    /// **On the screen because the screen is the point.** The appliance has no keyboard, so an
    /// owner cannot be told to run a command to find it; what it does have is a television, and
    /// reading a code off that requires being in the room. `None` once somebody has set a password
    /// of their own, and then nothing about it is drawn.
    ///
    /// **`Some` is the whole of what this panel knows about passwords.** The machine keeps the plain
    /// PIN exactly as long as the generated password is the one in force, so this being `Some` is
    /// already "still on the factory password"; a bool beside it would be the same fact twice.
    pub factory_pin: Option<String>,
    /// Which key opens the remote in this computer's own browser, where there is one.
    ///
    /// **The panel's address answers a phone and says nothing to the person sitting at the machine**,
    /// who would have to read a URL off a screen and type it into a browser on the same box. `F11`
    /// is the answer to that, and a key nothing mentions is a key nobody presses.
    ///
    /// **Handed in rather than worked out here**, exactly as [`Self::factory_pin`] is: whether
    /// there is a browser to open, and which spelling of the key reaches it, are facts about the
    /// operating system, and this crate draws for five of them. `karaokemachine`'s
    /// `BROWSER_KEY_HINT` is the one place that decides.
    pub browser_key: Option<BrowserKey>,
}

/// The key the connect panel should name as the way to the remote on this computer.
///
/// **Two spellings of one binding rather than two bindings.** Both press
/// [`DisplayAction::OpenRemote`](crate::DisplayAction::OpenRemote) and both work everywhere; what
/// varies is which one the panel is willing to promise. macOS takes bare `F11` for *Show Desktop*
/// in the window server, so a press there never reaches an application and a panel naming it would
/// be naming a key that does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKey {
    /// `F11`.
    F11,
    /// `Ctrl+F11`.
    CtrlF11,
}

/// Why the web UI cannot be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectProblem {
    /// Bound to loopback, so only this machine can connect.
    LoopbackOnly,
    /// No usable network interface.
    NoNetwork,
    /// The server did not start.
    ServerFailed(String),
}

/// What the connect panel should display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectPanel {
    /// The heading.
    pub headline: String,
    /// The URL to show large, and to encode as a QR code. `None` when unreachable.
    pub url: Option<String>,
    /// A line under the URL: alternative addresses, or an explanation of the problem.
    pub detail: Option<String>,
    /// A line under that, saying which key opens the remote on this computer. `None` where there is
    /// no such key, and on every state with no address.
    ///
    /// **Its own field rather than a third piece of [`Self::detail`].** That one is the address
    /// count and the PIN joined into a sentence about *this machine's* address; this is an
    /// instruction about *this computer's* keyboard. They wrap separately because the panel spends
    /// its rows on the detail first and gives this what is left, and a joined string could not be
    /// cut at the seam between the two.
    pub hint: Option<String>,
    /// Whether this is a problem rather than an address.
    pub is_problem: bool,
}

impl ConnectInfo {
    /// A reachable machine at one or more URLs.
    pub fn reachable(urls: Vec<String>, bound: impl Into<String>) -> Self {
        Self {
            urls,
            bound: Some(bound.into()),
            reachable: true,
            problem: None,
            factory_pin: None,
            browser_key: None,
        }
    }

    /// An unreachable machine, with the reason.
    pub fn unreachable(problem: ConnectProblem, bound: Option<String>) -> Self {
        Self {
            urls: Vec::new(),
            bound,
            reachable: false,
            problem: Some(problem),
            factory_pin: None,
            browser_key: None,
        }
    }

    /// The address a remote should use.
    pub fn primary_url(&self) -> Option<&str> {
        self.urls.first().map(String::as_str)
    }

    /// Whether [`Self::panel`] will have an address to show, without building the panel.
    ///
    /// Every branch of `panel` that reports a problem leaves [`ConnectPanel::url`] `None`, and the
    /// only branch that fills it in is the one reached with no problem and a primary address — so
    /// this is that condition rather than a second opinion about it.
    ///
    /// It exists for the playing screen, which asks once a frame whether the standing panel is going
    /// to draw anything: the two lines along the bottom are shortened to make room for it, and
    /// shortening them for a panel that turns out to be empty is a queue label cut for nothing.
    /// `panel` allocates four strings to answer, and this is on the path of every frame of every
    /// demo song.
    pub fn has_url(&self) -> bool {
        self.problem.is_none() && self.primary_url().is_some()
    }

    /// Works out what to put on screen.
    pub fn panel(&self, locale: km_locale::Locale) -> ConnectPanel {
        let words = crate::words::messages(locale);
        if let Some(problem) = &self.problem {
            return match problem {
                ConnectProblem::LoopbackOnly => ConnectPanel {
                    headline: words.msg(crate::words::CONNECT_LOCAL_ONLY).into_owned(),
                    url: None,
                    detail: Some(
                        words
                            .msg_with(
                                crate::words::CONNECT_LOCAL_ONLY_DETAIL,
                                &[(
                                    "address",
                                    self.bound.as_deref().unwrap_or("127.0.0.1").into(),
                                )],
                            )
                            .into_owned(),
                    ),
                    hint: None,
                    is_problem: true,
                },
                ConnectProblem::NoNetwork => ConnectPanel {
                    headline: words.msg(crate::words::CONNECT_NO_NETWORK).into_owned(),
                    url: None,
                    detail: Some(
                        words
                            .msg(crate::words::CONNECT_NO_NETWORK_DETAIL)
                            .into_owned(),
                    ),
                    hint: None,
                    is_problem: true,
                },
                ConnectProblem::ServerFailed(message) => ConnectPanel {
                    headline: words.msg(crate::words::CONNECT_UNAVAILABLE).into_owned(),
                    url: None,
                    // The actual error, not a shrug. Whoever is standing at the machine is the
                    // person who can fix a port conflict.
                    //
                    // **Untranslated, and that is the honest answer rather than a gap.** It is an
                    // operating system's message about a socket, arriving as a string from a crate
                    // that never saw a catalog; the alternative is a code per errno, which would
                    // say less than the sentence does.
                    detail: Some(message.clone()),
                    hint: None,
                    is_problem: true,
                },
            };
        }

        let Some(primary) = self.primary_url() else {
            // Reachable but with no address is a contradiction; say so rather than showing blank.
            return ConnectPanel {
                headline: words.msg(crate::words::CONNECT_UNAVAILABLE).into_owned(),
                url: None,
                detail: Some(words.msg(crate::words::CONNECT_NO_ADDRESS).into_owned()),
                hint: None,
                is_problem: true,
            };
        };

        let mut detail = Vec::new();
        let others = self.urls.len() - 1;
        if others > 0 {
            // Several interfaces: say how many rather than naming them. `km_api::connect` has
            // already ranked them — it demotes virtual adapters by name and prefers the private
            // range a home router hands out — so the one on screen is the one a phone can reach and
            // the losers are mostly WSL, Hyper-V and Docker. Naming them spends the panel's three
            // wrapped lines on addresses nothing off this machine can use, and can truncate the
            // address the panel came to show. The full list is still in the startup log and in
            // `GET /api/v1/discover`, which is where somebody diagnosing a wrong pick should look.
            detail.push(
                words
                    .msg_with(
                        crate::words::CONNECT_OTHER_ADDRESSES,
                        &[("count", (others as i64).into())],
                    )
                    .into_owned(),
            );
        }
        // **The PIN goes on the screen, and only while it is still the factory one.** Anybody in the
        // room can read it, which is the trust boundary a machine under a television actually has;
        // anybody who is not in the room cannot. Once an owner sets a password of their own this
        // says nothing at all, because there is then nothing here that would help them and something
        // that would help a guest.
        //
        // **A PIN or nothing.** This panel is the singer's, and the singer's remote asks for no
        // password: everything the door guards is under `/api/v1/admin/`, which is the owner's tools
        // and not the remote. A line demanding a password would sit beside the address a phone is
        // meant to scan and read as *you cannot use this without one*. The PIN is a different
        // sentence — the one thing on this screen an owner cannot get anywhere else.
        if let Some(pin) = &self.factory_pin {
            detail.push(
                words
                    .msg_with(
                        crate::words::CONNECT_FACTORY_PIN,
                        &[("pin", pin.as_str().into())],
                    )
                    .into_owned(),
            );
        }

        ConnectPanel {
            headline: words.msg(crate::words::CONNECT_HEADING).into_owned(),
            url: Some(primary.to_owned()),
            detail: (!detail.is_empty()).then(|| detail.join(" ")),
            // **Beside the code and nowhere else.** The three states with no address draw no code
            // either, and each of them already spends its whole panel on the sentence explaining
            // itself — a second instruction under that is one more line in the one place lines are
            // short. That the key still works in those states is true and is not this line's to say.
            hint: self.browser_key.map(|key| {
                words
                    .msg(match key {
                        BrowserKey::F11 => crate::words::CONNECT_BROWSER_KEY,
                        BrowserKey::CtrlF11 => crate::words::CONNECT_BROWSER_KEY_CTRL,
                    })
                    .into_owned()
            }),
            is_problem: false,
        }
    }
}

/// A QR code as a grid of dark and light modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrMatrix {
    /// Modules per side.
    pub size: usize,
    /// Row-major, `true` where the module is dark.
    pub modules: Vec<bool>,
}

impl QrMatrix {
    /// Encodes a string, or `None` if it will not fit in a QR code.
    pub fn encode(data: &str) -> Option<Self> {
        let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
        let size = code.width();
        let modules = code
            .to_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect();
        Some(Self { size, modules })
    }

    /// Whether the module at a position is dark. Out-of-range positions read as light.
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        if x >= self.size || y >= self.size {
            return false;
        }
        self.modules
            .get(y * self.size + x)
            .copied()
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use km_locale::Locale;

    use super::*;

    const EN: Locale = Locale::English;
    const PT: Locale = Locale::BrazilianPortuguese;

    /// The cheap answer and the real one agree, on every state there is.
    ///
    /// [`ConnectInfo::has_url`] exists so the playing screen need not build a panel once a frame,
    /// and a shortcut that quietly stopped matching what it is a shortcut for would shorten the
    /// bottom lines for a panel that draws nothing — or, worse, leave a QR code running under them.
    #[test]
    fn has_url_answers_what_the_panel_would() {
        let bound = Some("127.0.0.1:8177".to_owned());
        for info in [
            ConnectInfo::reachable(vec!["http://192.168.1.42:8177".to_owned()], "0.0.0.0:8177"),
            ConnectInfo::unreachable(ConnectProblem::LoopbackOnly, bound.clone()),
            ConnectInfo::unreachable(ConnectProblem::NoNetwork, None),
            ConnectInfo::unreachable(ConnectProblem::ServerFailed("in use".to_owned()), bound),
            // Reachable with nothing to be reachable at — the contradiction `panel` reports rather
            // than drawing blank, and the one case that is neither of the two obvious ones.
            ConnectInfo::reachable(Vec::new(), "0.0.0.0:8177"),
            ConnectInfo::default(),
        ] {
            assert_eq!(
                info.has_url(),
                info.panel(EN).url.is_some(),
                "has_url disagrees with the panel for {info:?}"
            );
        }
    }

    #[test]
    fn a_reachable_machine_shows_its_url() {
        let info =
            ConnectInfo::reachable(vec!["http://192.168.1.42:8177".to_owned()], "0.0.0.0:8177");
        let panel = info.panel(EN);
        assert!(!panel.is_problem);
        assert_eq!(panel.url.as_deref(), Some("http://192.168.1.42:8177"));
        assert_eq!(panel.detail, None);
    }

    #[test]
    fn one_extra_address_is_counted_rather_than_named() {
        let info = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://10.0.0.5:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        let panel = info.panel(EN);
        assert_eq!(panel.url.as_deref(), Some("http://192.168.1.42:8177"));
        let detail = panel.detail.expect("the count should be shown");
        assert_eq!(detail, "1 other address", "singular, and no address in it");
        // The whole point: the runner-up is not on screen. It is in the startup log and in
        // `GET /api/v1/discover`, both of which still carry every address.
        assert!(!detail.contains("10.0.0.5"), "got {detail}");
    }

    #[test]
    fn several_extra_addresses_are_counted_in_the_plural() {
        // The developer's machine this exists for: a real Wi-Fi address in front of WSL and Docker.
        let info = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://172.28.0.1:8177".to_owned(),
                "http://172.17.0.1:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        let detail = info.panel(EN).detail.expect("the count should be shown");
        assert_eq!(detail, "2 other addresses");
    }

    #[test]
    fn the_count_and_the_pin_are_both_said() {
        let mut info = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://10.0.0.5:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        info.factory_pin = Some("482913".to_owned());
        let detail = info.panel(EN).detail.expect("detail");
        assert_eq!(detail, "1 other address · PIN 482913");
    }

    #[test]
    fn a_portuguese_machine_says_all_of_this_in_portuguese() {
        // Including the plural, which Portuguese forms by changing the noun rather than by adding a
        // letter to it — the case the hand-written `if others == 1 { "" } else { "es" }` could not
        // have expressed.
        let mut info = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://10.0.0.5:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        info.factory_pin = Some("482913".to_owned());
        let panel = info.panel(PT);
        assert_eq!(panel.headline, "Controle remoto");
        assert_eq!(
            panel.detail.expect("detail"),
            "mais 1 endereço · PIN 482913"
        );

        let many = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://172.28.0.1:8177".to_owned(),
                "http://172.17.0.1:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        assert_eq!(many.panel(PT).detail.expect("detail"), "mais 2 endereços");
    }

    #[test]
    fn a_server_failure_keeps_the_operating_systems_own_words() {
        // The one thing on this panel that is deliberately not translated: it is an OS message
        // about a socket, and a code per errno would say less than the sentence does.
        let info = ConnectInfo::unreachable(
            ConnectProblem::ServerFailed("Address already in use (os error 98)".to_owned()),
            Some("0.0.0.0:8177".to_owned()),
        );
        let panel = info.panel(PT);
        assert_eq!(panel.headline, "Controle remoto indisponível");
        assert_eq!(
            panel.detail.expect("detail"),
            "Address already in use (os error 98)"
        );
    }

    /// The panel offers a PIN or says nothing, and never tells a singer a password is wanted.
    ///
    /// The remote this panel points a phone at asks for no password — the door is on
    /// `/api/v1/admin/`, which is the owner's tools — so a line demanding one beside the QR code
    /// would answer a question nobody asked. A PIN is the opposite case: it is the one fact about
    /// the machine that exists nowhere an owner can reach.
    #[test]
    fn a_machine_with_no_pin_to_give_says_nothing_about_a_password() {
        let mut info = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://10.0.0.5:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        for locale in [EN, PT] {
            let detail = info.panel(locale).detail.expect("the count is still said");
            assert!(
                !detail.to_lowercase().contains("password") && !detail.contains("senha"),
                "got {detail}"
            );
        }

        // And with one, the PIN is what appears — not the word.
        info.factory_pin = Some("482913".to_owned());
        let detail = info.panel(EN).detail.expect("detail");
        assert!(detail.contains("482913"), "got {detail}");
        assert!(!detail.to_lowercase().contains("password"), "got {detail}");
    }

    #[test]
    fn loopback_only_says_so_instead_of_showing_an_unreachable_url() {
        let info = ConnectInfo::unreachable(
            ConnectProblem::LoopbackOnly,
            Some("127.0.0.1:8177".to_owned()),
        );
        let panel = info.panel(EN);
        assert!(panel.is_problem);
        assert_eq!(
            panel.url, None,
            "a URL that will not work must not be shown"
        );
        let detail = panel.detail.expect("detail");
        assert!(detail.contains("127.0.0.1"));
        assert!(
            detail.contains("0.0.0.0"),
            "the fix should be stated: {detail}"
        );
    }

    #[test]
    fn no_network_explains_what_to_do() {
        let panel = ConnectInfo::unreachable(ConnectProblem::NoNetwork, None).panel(EN);
        assert!(panel.is_problem);
        assert_eq!(panel.url, None);
        assert!(panel.detail.expect("detail").contains("Wi-Fi"));
    }

    #[test]
    fn a_failed_server_shows_the_actual_error() {
        let panel = ConnectInfo::unreachable(
            ConnectProblem::ServerFailed("address already in use (port 8177)".to_owned()),
            None,
        )
        .panel(EN);
        assert!(panel.is_problem);
        assert_eq!(
            panel.detail.as_deref(),
            Some("address already in use (port 8177)"),
            "whoever is at the machine is the person who can fix it"
        );
    }

    #[test]
    fn reachable_with_no_address_is_reported_rather_than_shown_blank() {
        let info = ConnectInfo {
            urls: Vec::new(),
            bound: Some("0.0.0.0:8177".to_owned()),
            reachable: true,
            problem: None,
            factory_pin: None,
            browser_key: None,
        };
        let panel = info.panel(EN);
        assert!(panel.is_problem);
        assert_eq!(panel.url, None);
    }

    #[test]
    fn a_machine_with_a_browser_key_says_which_key_it_is() {
        let mut info =
            ConnectInfo::reachable(vec!["http://192.168.1.42:8177".to_owned()], "0.0.0.0:8177");
        assert_eq!(info.panel(EN).hint, None, "no key, nothing to offer");

        info.browser_key = Some(BrowserKey::F11);
        assert_eq!(
            info.panel(EN).hint.as_deref(),
            Some("F11 opens the remote in a browser")
        );
        assert_eq!(
            info.panel(PT).hint.as_deref(),
            Some("F11 abre o controle remoto no navegador")
        );
        // The line is not folded into the detail, which stays the address count and the PIN.
        assert_eq!(info.panel(EN).detail, None);

        // The other spelling, which is what macOS is offered because the bare key never reaches an
        // application there. Same sentence, same place, one key named differently.
        info.browser_key = Some(BrowserKey::CtrlF11);
        assert_eq!(
            info.panel(EN).hint.as_deref(),
            Some("Ctrl+F11 opens the remote in a browser")
        );
        assert_eq!(
            info.panel(PT).hint.as_deref(),
            Some("Ctrl+F11 abre o controle remoto no navegador")
        );
    }

    /// The key still works in each of these states and the panel still does not mention it.
    ///
    /// The three of them have no address to encode and no code to draw, and each already spends its
    /// whole panel on the sentence explaining itself. This is the claim `ConnectPanel::hint`'s doc
    /// makes, asserted where it can be — a state that started carrying the line would push the
    /// explanation a row further down in the one place rows are short.
    #[test]
    fn no_state_without_an_address_offers_the_key() {
        let bound = Some("127.0.0.1:8177".to_owned());
        for problem in [
            ConnectProblem::LoopbackOnly,
            ConnectProblem::NoNetwork,
            ConnectProblem::ServerFailed("in use".to_owned()),
        ] {
            let mut info = ConnectInfo::unreachable(problem, bound.clone());
            info.browser_key = Some(BrowserKey::CtrlF11);
            let panel = info.panel(EN);
            assert_eq!(panel.url, None);
            assert_eq!(
                panel.hint, None,
                "{panel:?} has no code to put a line under"
            );
        }

        let mut nothing = ConnectInfo::reachable(Vec::new(), "0.0.0.0:8177");
        nothing.browser_key = Some(BrowserKey::F11);
        assert_eq!(nothing.panel(EN).hint, None);
    }

    #[test]
    fn a_url_encodes_to_a_qr_code() {
        let qr = QrMatrix::encode("http://192.168.1.42:8177").expect("should encode");
        assert!(qr.size >= 21, "the smallest QR version is 21 modules");
        assert_eq!(qr.modules.len(), qr.size * qr.size);
        // The top-left finder pattern is dark at its corner.
        assert!(qr.is_dark(0, 0));
    }

    #[test]
    fn out_of_range_qr_positions_read_as_light_rather_than_panicking() {
        let qr = QrMatrix::encode("http://example.test").expect("should encode");
        assert!(!qr.is_dark(qr.size, 0));
        assert!(!qr.is_dark(0, qr.size));
        assert!(!qr.is_dark(usize::MAX, usize::MAX));
    }

    #[test]
    fn an_unencodable_payload_returns_none_rather_than_panicking() {
        // Far past the capacity of any QR version.
        let huge = "x".repeat(10_000);
        assert_eq!(QrMatrix::encode(&huge), None);
    }
}
