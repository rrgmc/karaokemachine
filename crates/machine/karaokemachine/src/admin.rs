//! Wiring the owner's page into this machine.
//!
//! `crates/machine/km-admin-pages` renders the pages and knows which API route each control is
//! about to exercise; this says whether the caller may exercise it. The division is the one
//! `remote.rs` already draws for the singer's remote, and the implementation is the same shape —
//! deliberately, because a second authorization path is the thing worth not having.

use std::sync::Arc;

use axum::http::HeaderMap;
use km_admin_pages::guard::{Caller, Grant, Guard, LoggedIn, Refusal};
use km_api::ApiState;

/// The path this guard asks the API about.
///
/// Never fetched — it exists so the question "may this caller act as the owner?" is asked of the
/// same code that answers it for the JSON API, rather than of a second implementation. Any path
/// under `/api/v1/admin/` would do; this one is real, which keeps it from looking like a placeholder
/// somebody could delete.
const PROBE_PATH: &str = "/api/v1/admin/password";

/// Authorizes the owner's page against the machine's own admin password.
///
/// Every decision is [`ApiState::authorize`]'s, unchanged, so nothing new becomes reachable by there
/// being a page for it. The singer's remote needs no twin of this: nothing it serves is an admin
/// action.
pub struct AdminAclGuard(ApiState);

impl AdminAclGuard {
    /// Against this machine's access list.
    pub fn new(state: ApiState) -> Self {
        Self(state)
    }

    /// The token from the browser's cookie, put back into the header the API's check expects.
    ///
    /// **This is what makes it one authorization path rather than two.** `ApiState::authorize`
    /// reads an `Authorization: Bearer` header, and a browser following a link cannot be told to
    /// send one — so the token rides in a cookie and is turned back into the header here, rather
    /// than the ACL growing a second way to be consulted.
    fn headers_of(caller: &Caller) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(token) = &caller.token
            && let Ok(value) = format!("Bearer {token}").parse()
        {
            headers.insert(axum::http::header::AUTHORIZATION, value);
        }
        headers
    }
}

#[async_trait::async_trait]
impl Guard for AdminAclGuard {
    async fn allows(&self, caller: &Caller) -> Result<(), Refusal> {
        // **A representative admin path rather than the page's own.** The check is `does this
        // caller hold a valid admin token`, and the API answers that from the path's prefix — so
        // any path under `/api/v1/admin/` asks exactly the question this page needs answered, and
        // asking it about the real page path would mean mapping page routes to API routes again,
        // which is the table this change deleted.
        self.0
            .authorize(PROBE_PATH, &Self::headers_of(caller))
            .map_err(|error| match error {
                km_api::ApiError::Unauthorized(message) => Refusal::NeedsPassword(message),
                other => Refusal::Failed(other.to_string()),
            })
    }

    async fn factory_password(&self) -> bool {
        self.0.factory_password()
    }

    /// `remember` is ignored, and `Guard::remembering` answering `None` is why nothing ever sets it:
    /// the machine is the surface a password is *for*, so remembering one of its own would be a
    /// credential kept beside the door it opens.
    async fn login(
        &self,
        password: &str,
        caller: &Caller,
        _remember: bool,
    ) -> Result<LoggedIn, Refusal> {
        // Through `AdminAuth::login`, so this page meets the same rate limiter the JSON route does.
        // An unknown peer folds into `0.0.0.0` rather than getting a free pass -- the same call
        // `remote.rs` makes, and for the same reason: a caller whose address the server cannot see
        // is not a caller who should be able to guess without limit.
        let from = caller
            .peer
            .map(|peer| peer.ip())
            .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED.into());
        let grant = self
            .0
            .auth()
            .login(from, password)
            .map_err(|error| Refusal::NeedsPassword(error.to_string()))?;
        // **A cookie, because on this host the token is the browser's.** One per browser, and a
        // browser following a link cannot be told to send an `Authorization` header — which is the
        // whole reason the cookie exists. See `guard::LoggedIn`.
        Ok(LoggedIn::Cookie(Grant {
            token: grant.token,
            expires_in_secs: grant.expires_in_secs,
        }))
    }
}

/// The owner's page, ready to be nested under `/admin`.
///
/// Returns only a router, unlike [`crate::remote::build`], because there is nothing to pump: this
/// page reloads rather than holding an event stream open, which is the right trade for a surface
/// somebody uses once rather than all evening.
///
/// **One `ThisMachine` behind every trait object**, because they are groupings of one machine rather
/// than several things: the seam is split by *tab* so the strip and the traits can be read against
/// each other, not because a host might answer them from different places. A host that genuinely
/// could — one talking to two machines — is not a thing this product has.
pub fn build(state: ApiState) -> axum::Router {
    let guard = Arc::new(AdminAclGuard::new(state.clone()));
    let machine = Arc::new(km_admin_pages::in_process::ThisMachine::new(state.clone()));
    km_admin_pages::router(
        km_admin_pages::Admin::over(
            km_admin_pages::machine::Capabilities::machine(),
            km_admin_pages::ICON_MACHINE_PNG,
            guard,
            machine.clone(),
        )
        .with_problems(machine),
    )
}
