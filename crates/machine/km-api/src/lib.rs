//! HTTP and WebSocket control API for the karaoke machine.
//!
//! `axum` over `tokio`, JSON in and out, plus one WebSocket carrying everything that changes. The
//! crate is built so the whole surface is testable with no audio device, no SoundFont and no
//! catalog on disk: it never touches the engine or the library directly, only the two traits in
//! [`machine`], and the `testing` feature supplies an in-memory machine that implements them.
//!
//! Where to look:
//!
//! * [`routes`] — every path and the route id that guards it. Start here.
//! * [`machine`] — the seam: what the API can ask the machine to do.
//! * [`power`] — the second seam, for the one thing that is a fact about the *host* rather than
//!   about the machine: whether this box can turn itself off.
//! * [`dto`] — the wire format, defined in one place so the contract is readable.
//! * [`routes`] — the router, and the URL prefix that is the permission.
//! * [`auth`] — the single shared password, hashed, exchanged for a short-lived token.
//! * [`events`] — the WebSocket event stream, and why per-syllable position is never streamed.
//! * [`connect`] — working out the address a phone should actually type, and saying so honestly
//!   when there is not one.
//! * [`listener`] — holding the port, for the platforms that take it away while nobody is looking.
//! * [`discover`] — the mDNS advertisement and the always-public discovery payload.
//! * [`server`] — shared state, authorization, binding and serving.
//!
//! See `docs/ARCHITECTURE.md` for the design and `CLAUDE.md` for the rule that changes of direction
//! are recorded there.

/// This crate's tracing target, for anything outside it that names one in a filter string.
///
/// The `dev_server` example is the caller: an example is its own crate, so `CARGO_CRATE_NAME` there
/// answers `dev_server` and cannot be used to name this one. A filter pointing at a target that
/// does not exist is an error nowhere — it just prints nothing — so the name is exported rather
/// than spelled twice.
pub const LOG_TARGET: &str = env!("CARGO_CRATE_NAME");

pub mod auth;
pub mod book;
pub mod config;
pub mod connect;
pub mod discover;
pub mod dto;
pub mod error;
pub mod events;
pub mod handlers;
pub mod listener;
pub mod machine;
pub mod ops;
pub mod power;
pub mod routes;
pub mod server;
pub mod watch;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use crate::auth::{AdminAuth, Grant, LoginError, MIN_PASSWORD_CHARS};
pub use crate::config::ApiConfig;
pub use crate::connect::{ConnectInfo, ConnectProblem, DEFAULT_PORT};
pub use crate::discover::{Advert, Discovery, SERVICE_TYPE};
pub use crate::error::{ApiError, ApiResult};
pub use crate::events::{EndReason, Event, Events, STATE_INTERVAL};
pub use crate::listener::Relisten;
pub use crate::machine::{Audition, Catalog, CatalogError, ControlError, Controller};
pub use crate::power::{Power, PowerError};
pub use crate::routes::Extras;

/// Taking a file an owner sent, for whoever is serving the page they sent it from.
///
/// **A front door onto [`handlers`]' own streaming rather than a second implementation**, and the
/// reason it is public at all: `km-admin-pages` serves forms that post the same three kinds of file
/// the JSON routes take, and it must not grow its own multipart handling. That is the
/// `One implementation of each operation` decision applied to the half of an upload nobody thinks
/// of as an operation — the size cap, the extension list, the name sanitising and the streaming.
pub mod uploads {
    use crate::error::ApiResult;
    use crate::machine::Upload;
    use crate::server::ApiState;

    /// Streams one uploaded file into the machine and says what happened, in a sentence.
    ///
    /// Takes a `Multipart` that has already been extracted, because a page's handler wants axum's
    /// own rejection to reach its own error page rather than this crate's JSON shape.
    pub async fn receive(
        state: &ApiState,
        form: axum::extract::Multipart,
        kind: Upload,
    ) -> ApiResult<String> {
        crate::handlers::receive_upload(state, Ok(form), kind).await
    }

    /// How large this kind may be, for a page that wants to say so before somebody picks a file.
    #[must_use]
    pub fn limit_for(kind: Upload) -> usize {
        crate::handlers::limits_for(kind).0
    }

    /// What the file part must be called, on all three upload routes.
    ///
    /// **Re-exported here so a client does not have to reach into `handlers` for it.** This module
    /// is the front door for everything about an upload, and a program on the other end of the wire
    /// needs the field name as much as the page on this side needs [`receive`] — `km-admin` builds
    /// the multipart body that `receive_upload` reads, and a disagreement about this one string is a
    /// 400 that says only "no file part".
    pub use crate::handlers::FILE_FIELD;

    /// The extensions one kind of upload accepts.
    ///
    /// For the same reason as [`FILE_FIELD`]: a page that lets somebody choose a file should say
    /// what it can take before they choose, rather than after the machine has refused it.
    #[must_use]
    pub fn extensions_for(kind: Upload) -> &'static [&'static str] {
        crate::handlers::limits_for(kind).1
    }

    /// What an `<input type="file">` should put in its `accept` attribute for this kind.
    ///
    /// **Built from [`extensions_for`] rather than typed into a template**, which is the whole
    /// reason that function is public: a hand-written list is one that drifts from what the machine
    /// actually takes, and the drift is invisible until somebody's file is refused *after* they
    /// picked it. `km-admin` derived its three and the owner's page typed its three out; they
    /// happened to agree, which is the state a copy is in right before it stops agreeing.
    ///
    /// # No wildcard for any of the three, and that is not an oversight
    ///
    /// None of them has a media type a chooser knows. `.kmpkg` is this project's own, `.sf2` is
    /// `audio/x-soundfont` at best, and a wallpaper pack is a `.zip` — so a wildcard would either
    /// match nothing or, widened to `application/*`, offer every file on the device.
    ///
    /// **A wallpaper takes no `image/*` either, because a picture is not what the route wants.** The
    /// wallpaper folder holds packs, so offering a phone its camera roll would put somebody one tap
    /// from a file the machine refuses. See `A wallpaper is a pack` in
    /// `docs/decisions/interface.md`, which is where what that costs is argued.
    #[must_use]
    pub fn accept_for(kind: Upload) -> String {
        let extensions = extensions_for(kind)
            .iter()
            .map(|extension| format!(".{extension}"));
        extensions.collect::<Vec<_>>().join(",")
    }

    /// How large this kind may be, in words a person can read: `64 MB`, `2 GB`.
    ///
    /// **The same words the machine's own refusal uses**, which is the whole reason it is here
    /// rather than in each caller. A page that says "up to 64 MB" above the chooser and a machine
    /// that says "the limit is 67108864" when it refuses have told somebody two different things
    /// about one number.
    #[must_use]
    pub fn limit_in_words(kind: Upload) -> String {
        crate::handlers::bytes_in_words(limit_for(kind) as u64)
    }

    /// Where this kind of upload goes, relative to [`crate::routes::API_PREFIX`].
    ///
    /// **The largest of the strings this module exists to stop a client guessing at**, and the one
    /// it did not carry until a client guessed wrong. `km-admin` read [`FILE_FIELD`], [`limit_for`]
    /// and [`extensions_for`] from here and hand-wrote the fifth thing a request needs — so every
    /// file it ever sent, of all three kinds, went to a path with no `/admin` in it and reached
    /// nothing. A package answered the fallback's 404; a picture and a bank have public `GET` twins
    /// at those paths and answered a bare 405.
    ///
    /// **Relative to the prefix, and deliberately not a second absolute spelling**, which is the
    /// convention [`crate::routes::SURFACE`] already uses and lets a test compare the two directly.
    /// Two functions differing by `/api/v1` would be the drift this one exists to remove.
    ///
    /// The `match` is exhaustive, so a fourth kind of upload is a compile error here rather than a
    /// fourth guess at a call site. See `The URL prefix is the permission` in
    /// `docs/decisions/api-and-network.md` for why all three sit under `/admin/`.
    #[must_use]
    pub fn path_for(kind: Upload) -> &'static str {
        match kind {
            Upload::Package => "/admin/packages/upload",
            Upload::Wallpaper => "/admin/wallpapers",
            Upload::SoundFont => "/admin/audio/soundfonts",
        }
    }
}
pub use crate::server::{ApiState, Listening, bind, bind_with};
