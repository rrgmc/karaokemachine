//! Who may open the owner's page.
//!
//! **Every page in this crate is an admin action, so there is one question rather than a table of
//! thirty-odd rows.** Everything the machine treats as privileged lives under `/api/v1/admin/` and
//! every control on this page drives one of those, so the guard asks *may this caller act as the
//! owner at all* rather than *which route id is this page about to exercise*. A per-route table
//! would have to be gated on the right row, and a page gated on the wrong one is a control panel
//! that opens.
//!
//! # It still denies by default, and that is the property worth keeping
//!
//! [`is_open`] is an allow-list of the routes that need no password — the login page, which is where
//! somebody without a token is sent, and the stylesheet and the favicon, which touch nothing and
//! which a browser fetches before anybody could have signed in. **Everything else demands a token,
//! including a route nobody remembered to think about.** A page added to the
//! router without a thought is a page that asks for the password, which is the safe direction to
//! fail; the old table's fall-through had to be a refusal to achieve the same thing, and now it is
//! the only behaviour there is.
//!
//! That also disposes of something nothing could settle by reading the source: whether axum reports
//! the full path or the inner one from `MatchedPath` under `nest`. [`is_open`] handles both
//! spellings, and every other answer — including a third one a future version might invent — lands
//! on "ask for the password".

use std::net::SocketAddr;

/// Who is asking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    /// The admin token this browser is holding, from its cookie.
    ///
    /// A cookie and not an `Authorization` header, because these are ordinary page loads and form
    /// posts — a browser cannot be told to put a header on a link. The guard turns it back into the
    /// header the API's own check expects, so there is one authorization path and not two.
    pub token: Option<String>,
    /// Where the request came from, for the login rate limiter.
    pub peer: Option<SocketAddr>,
}

/// The caller's address, when the server knows it.
///
/// A custom extractor rather than `ConnectInfo<SocketAddr>` directly, because that one **rejects**
/// when the information is absent — and absent is legitimate: a test driving the router as a service
/// has none. `km-api`'s `Peer` exists for the same reason and was written after the same 500.
#[derive(Debug, Clone, Copy, Default)]
pub struct PeerAddr(pub Option<SocketAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for PeerAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<SocketAddr>>()
                .map(|connect| connect.0),
        ))
    }
}

/// A token and how long it lasts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The token to hold.
    pub token: String,
    /// Seconds until it stops working.
    pub expires_in_secs: u64,
}

/// What a successful login left behind.
///
/// # Why this is not simply a [`Grant`]
///
/// **The two hosts hold the token in different places, and only one of them is the browser's.** On
/// the machine a token is *this browser's*: it rides in an `HttpOnly` cookie, one per browser, because
/// a browser following a link cannot be told to send an `Authorization` header. In a tool on loopback
/// the token is *the program's* — it lives in the kept `reqwest::Client`, which is why that client is
/// kept at all rather than built per request.
///
/// So a `Grant` here would have forced the tool to invent a cookie value for a cookie nothing reads,
/// and a page setting one would be claiming a session it is not the keeper of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggedIn {
    /// Hand this to the browser. The machine's answer.
    Cookie(Grant),
    /// **Already put where it belongs; set no cookie.** A tool's answer.
    Kept,
}

/// What a host remembers about the machine's password, for the box beside the login field.
///
/// **`None` from [`Guard::remembering`] means there is nothing to offer**, and there are two ways to
/// arrive at it: a host that keeps no password at all, and a machine that has never answered. The
/// second is the interesting one — a password is remembered under the machine's *id*, so that a
/// machine which moves to another address is still recognized, and a machine nothing has heard from
/// has no id to key one under. The box is then left out rather than drawn and ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Remembering {
    /// Whether a password is remembered for this machine now, which is the box's state.
    pub on: bool,
    /// Whether the file can be made owner-only on this platform.
    ///
    /// The page says what is true rather than claiming a protection it did not apply: a unix file is
    /// `0600`, and a Windows one is protected by the profile directory and nothing more.
    pub owner_only: bool,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// No token, or one that has expired. The answer is the login page.
    #[error("{0}")]
    NeedsPassword(String),
    /// The machine could not answer.
    #[error("{0}")]
    Failed(String),
}

/// Deciding whether a caller may act as the owner, and getting them a token if not.
#[async_trait::async_trait]
pub trait Guard: Send + Sync + 'static {
    /// Whether this caller is holding a valid admin token.
    ///
    /// **One question, not one per route.** Every page here is an admin action; there is nothing a
    /// token opens some of and not the rest.
    ///
    /// **Asked by the middleware before a write, and by the Machine tab as a read.** On a host whose
    /// [`Capabilities::gate_every_route`](crate::machine::Capabilities::gate_every_route) is `true`
    /// the middleware asks it for every route, and a caller it refuses never reaches a handler. On a
    /// host where that is `false` nothing authorizes a write here at all — the machine is the
    /// authority on whether its own write may happen, which that field argues — and what this
    /// answers instead is whether the *program* holds a token, which is what decides whether the
    /// Machine tab draws a password box or a sentence.
    ///
    /// So the answer is worth getting right on either kind of host, and the two questions are not
    /// the same one: on the machine it is *may this browser act as the owner*, and on a tool it is
    /// *will the machine accept us*.
    async fn allows(&self, caller: &Caller) -> Result<(), Refusal>;

    /// Whether this machine is still on the password it generated for itself.
    ///
    /// **Not "is a password set" any more, because one always is.** What the page does with this is
    /// nag: a banner on every tab saying the machine is still on its factory PIN, linking to the tab
    /// that changes it. The old banner said "anyone on this network can change these settings" and
    /// linked nowhere, because there was nothing to press — that state does not exist now.
    async fn factory_password(&self) -> bool;

    /// Exchange a password for a token, wherever this host keeps one.
    ///
    /// `caller` is the browser asking, and a tool ignores it entirely: it is there for the machine's
    /// rate limiter, which keys on the address a guess came from.
    ///
    /// `remember` is the box beside the field, and a host that keeps no password ignores it. **It is
    /// spent here rather than by a second call**, because here is the one moment the password is in
    /// hand: a host keeps the token it buys, not the password, so a tick applied afterwards would
    /// have nothing to store. **An unticked box forgets** whatever was remembered before, the box
    /// being a statement about what this computer should be remembering rather than an action taken
    /// once.
    async fn login(
        &self,
        password: &str,
        caller: &Caller,
        remember: bool,
    ) -> Result<LoggedIn, Refusal>;

    /// What this host remembers about the machine's password, or `None` where it offers no box.
    ///
    /// Default `None`, which is the machine's own answer: the surface a password is *for* has no use
    /// for remembering one.
    async fn remembering(&self) -> Option<Remembering> {
        None
    }

    /// Forget whatever was remembered for this machine.
    ///
    /// **The way out that does not run through logging in**, which is the state somebody trying to
    /// stop is already in. Nothing on a host that remembers nothing.
    async fn forget_password(&self) {}
}

/// The routes that need no password, as an allow-list rather than a fall-through.
///
/// The login page has to be reachable without a token — it is what somebody without one is sent to —
/// and the stylesheet and the favicon touch nothing. **Everything else demands one**, including a
/// route that is not written down anywhere.
pub fn is_open(matched: &str) -> bool {
    let inner = matched.strip_prefix("/admin").unwrap_or(matched);
    // The favicon joins the stylesheet on exactly its terms: it touches nothing, and a mark behind a
    // login would be a browser asked for a password before it could draw a tab.
    matches!(inner, "/login" | "/static/admin.css" | "/static/icon.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both spellings, because which one `MatchedPath` reports under `nest` is not something the
    /// source settles.
    #[test]
    fn the_login_page_and_the_stylesheet_are_open_under_either_spelling() {
        for path in [
            "/login",
            "/admin/login",
            "/static/admin.css",
            "/admin/static/admin.css",
            // A browser asks for this before anybody could have signed in, and it touches nothing.
            "/static/icon.png",
            "/admin/static/icon.png",
        ] {
            assert!(is_open(path), "{path} must need no password");
        }
    }

    /// The property the deleted table existed to provide, now free: anything not named is refused.
    #[test]
    fn everything_else_is_refused_including_a_route_nobody_wrote_down() {
        for path in [
            "/",
            "/admin",
            "/machine",
            "/admin/songs",
            "/admin/songs/upload",
            "/admin/pictures",
            "/admin/sound",
            "/admin/problems",
            "/admin/machine/password",
            "/admin/something-invented-next-year",
            "/admin/loginx",
            "/admin/static/other.css",
        ] {
            assert!(!is_open(path), "{path} must demand the password");
        }
    }
}
