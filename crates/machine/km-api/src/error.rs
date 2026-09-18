//! Turning failures into responses.
//!
//! Every error a handler can produce funnels through [`ApiError`], which owns two decisions: the
//! status code, and the stable machine-readable `error` string in the body. Those strings are API —
//! a remote matches on them to decide whether to show "that number isn't in the catalog" or "the
//! queue is full" — so they live here rather than being spelled out at each call site where a typo
//! would go unnoticed.
//!
//! The `message` beside them is for a person and is explicitly not stable.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::dto::ErrorDto;
use crate::machine::{CatalogError, ControlError};

/// Something that stops a request from succeeding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// No such song, queue entry, package or microphone.
    #[error("{0}")]
    NotFound(String),
    /// The path is not an endpoint at all.
    ///
    /// Separate from [`ApiError::NotFound`] because the two are different problems: a client that
    /// asked for a song nobody has should show "not in the catalog", and a client that called a
    /// path this build does not have has a bug.
    #[error("{0}")]
    UnknownEndpoint(String),
    /// The request was malformed or its values were out of range.
    #[error("{0}")]
    BadRequest(String),
    /// A token is required and was not supplied, or has expired.
    #[error("{0}")]
    Unauthorized(String),
    /// The caller is not allowed here at all — a privileged route reached from off-machine with no
    /// password configured.
    #[error("{0}")]
    Forbidden(String),
    /// The body is bigger than the route will take.
    ///
    /// **Separate from [`ApiError::BadRequest`] because the request was not malformed.** An upload
    /// that trips a limit is well formed and simply too big, and the difference matters to the
    /// client: a 400 says *fix what you sent*, where a 413 says *send a smaller one*. It is also
    /// what a per-route `DefaultBodyLimit` trip already is as far as axum is concerned, so answering
    /// 400 made the machine disagree with its own middleware.
    #[error("{0}")]
    PayloadTooLarge(String),
    /// The machine cannot do this in its current state.
    #[error("{message}")]
    Conflict {
        /// The stable code, since several distinguishable things map to 409.
        code: &'static str,
        /// The explanation.
        message: String,
    },
    /// Too many attempts.
    #[error("{message}")]
    TooManyRequests {
        /// When to try again.
        retry_after_secs: u64,
        /// The explanation.
        message: String,
    },
    /// Something broke that is not the caller's fault.
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    /// The code for a path that is not an endpoint. Stable; clients match on it.
    pub const UNKNOWN_ENDPOINT: &'static str = "unknown_endpoint";

    /// The generic 409, for a refusal with no finer name of its own.
    pub const UNAVAILABLE: &'static str = "unavailable";

    /// A missing thing, named.
    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self::NotFound(format!("no such {what}"))
    }

    /// A 409 carrying whatever stable name the refusal brought with it.
    ///
    /// A refusal with no code of its own becomes [`Self::UNAVAILABLE`], which is what every one of
    /// them was before any of them had a name. A client that does not know the finer code falls
    /// back to exactly the same sentence it would have shown then.
    pub fn unavailable(refusal: crate::machine::Refusal) -> Self {
        Self::Conflict {
            code: refusal.code.unwrap_or(Self::UNAVAILABLE),
            message: refusal.message,
        }
    }

    /// The queue is at its limit.
    pub fn queue_full() -> Self {
        Self::Conflict {
            code: "queue_full",
            message: format!("the queue is full ({} songs)", km_queue::queue::MAX_QUEUED),
        }
    }

    /// The status code this maps to.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) | Self::UnknownEndpoint(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Conflict { .. } => StatusCode::CONFLICT,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The stable code a client may match on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::UnknownEndpoint(_) => Self::UNKNOWN_ENDPOINT,
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::PayloadTooLarge(_) => "too_large",
            Self::Conflict { code, .. } => code,
            Self::TooManyRequests { .. } => "too_many_requests",
            Self::Internal(_) => "internal",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code();
        if status.is_server_error() {
            // A 500 is our bug, and the only place it will ever be noticed is the log — the caller
            // gets a sentence. Recording it at error level is what makes it findable afterwards.
            tracing::error!(error = %self, "request failed");
        } else {
            tracing::debug!(error = %self, code, "request rejected");
        }

        let body = Json(ErrorDto::new(code, self.to_string()));
        match self {
            // `Retry-After` is the standard way to say it, and a client that honors it stops
            // hammering without needing to understand our body format.
            Self::TooManyRequests {
                retry_after_secs, ..
            } => (
                status,
                [("retry-after", retry_after_secs.to_string())],
                body,
            )
                .into_response(),
            // Naming the scheme is what makes a 401 actionable rather than mysterious.
            Self::Unauthorized(_) => (
                status,
                [(
                    "www-authenticate",
                    "Bearer realm=\"karaokemachine\"".to_owned(),
                )],
                body,
            )
                .into_response(),
            _ => (status, body).into_response(),
        }
    }
}

impl From<CatalogError> for ApiError {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::NotFound(what) => Self::not_found(what),
            CatalogError::Rejected(message) => Self::BadRequest(message),
            // The same 409 `ControlError::Unavailable` produces, for the same reason.
            CatalogError::Unavailable(refusal) => Self::unavailable(refusal),
            CatalogError::Failed(message) => Self::Internal(message),
        }
    }
}

impl From<ControlError> for ApiError {
    fn from(error: ControlError) -> Self {
        match error {
            ControlError::NotFound(what) => Self::not_found(what),
            ControlError::QueueFull => Self::queue_full(),
            // A 409 rather than a 400: the request is perfectly well formed, the machine just is not
            // in a state where it means anything. A remote should retry it later, not fix it.
            ControlError::Unavailable(refusal) => Self::unavailable(refusal),
            ControlError::Rejected(message) => Self::BadRequest(message),
            ControlError::Failed(message) => Self::Internal(message),
        }
    }
}

impl From<crate::power::PowerError> for ApiError {
    fn from(error: crate::power::PowerError) -> Self {
        use crate::power::PowerError;
        match error {
            // A 409 for the same reason `ControlError::Unavailable` is one: the request was
            // perfectly well formed and the machine is simply not in a state where it means
            // anything. A stable code of its own because "the operating system refused" is
            // distinguishable from every other 409 here and a client may want to say so differently.
            PowerError::Refused(message) => Self::Conflict {
                code: "power_refused",
                message,
            },
            PowerError::Failed(message) => Self::Internal(message),
        }
    }
}

impl From<crate::auth::LoginError> for ApiError {
    fn from(error: crate::auth::LoginError) -> Self {
        use crate::auth::LoginError;
        match error {
            // 404 rather than 401: with no password configured there is no admin mode to log in to,
            // and answering 401 would tell a prober that a password exists and they guessed wrong.
            LoginError::NotConfigured => Self::NotFound("admin mode is not configured".to_owned()),
            LoginError::Rejected => Self::Unauthorized("incorrect password".to_owned()),
            LoginError::RateLimited { retry_after_secs } => Self::TooManyRequests {
                retry_after_secs,
                message: format!("too many attempts; try again in {retry_after_secs}s"),
            },
            LoginError::BadStoredHash => {
                Self::Internal("the stored password hash is unreadable".to_owned())
            }
        }
    }
}

/// Shorthand for handler results.
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn body_of(error: ApiError) -> (StatusCode, ErrorDto, axum::http::HeaderMap) {
        let response = error.into_response();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read body");
        let dto = serde_json::from_slice(&bytes).expect("an error body");
        (status, dto, headers)
    }

    #[tokio::test]
    async fn a_missing_song_is_a_404_with_a_stable_code() {
        let (status, body, _) = body_of(ApiError::not_found("song 9999")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error, "not_found");
        assert_eq!(body.message, "no such song 9999");
    }

    #[tokio::test]
    async fn a_full_queue_is_a_409_distinguishable_from_other_conflicts() {
        let (status, body, _) = body_of(ApiError::queue_full()).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "queue_full");
        assert!(
            body.message
                .contains(&km_queue::queue::MAX_QUEUED.to_string())
        );
    }

    #[tokio::test]
    async fn a_refused_power_request_carries_the_operating_systems_own_words() {
        // The sentence *is* the diagnosis — `Interactive authentication required.` is searchable
        // where "power off failed" is not — so the mapping must not replace it with one of ours.
        let (status, body, _) = body_of(
            crate::power::PowerError::Refused("Interactive authentication required.".to_owned())
                .into(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "power_refused");
        assert_eq!(body.message, "Interactive authentication required.");
    }

    #[tokio::test]
    async fn a_power_request_that_could_not_be_made_is_ours_to_answer_for() {
        // Not a 409: nothing about the machine's *state* refused this. The tool was missing or would
        // not run, which is this build's problem and not something the caller can retry into.
        let (status, body, _) =
            body_of(crate::power::PowerError::Failed("systemctl is not on PATH".to_owned()).into())
                .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.message, "systemctl is not on PATH");
    }

    #[tokio::test]
    async fn a_refusal_with_no_name_is_a_409_a_client_can_tell_apart_from_a_full_queue() {
        let (status, body, _) =
            body_of(ControlError::Unavailable("nothing is playing".into()).into()).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, ApiError::UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_refusal_that_has_a_name_carries_it_instead_of_the_generic_one() {
        // The whole point of the code: `nothing is playing` and `a video song has no key to change`
        // are both 409s and are not the same sentence, and only one of them is about the song.
        let (status, body, _) = body_of(
            ControlError::Unavailable(crate::machine::Refusal::coded(
                "no_key_video",
                "a video song has no key to change",
            ))
            .into(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "no_key_video");
        // The message rides along for the log and for a client with no catalog; it is not the only
        // channel any more, which is what lets a page render this in Portuguese.
        assert_eq!(body.message, "a video song has no key to change");
    }

    #[tokio::test]
    async fn a_rate_limited_login_says_when_to_come_back() {
        let (status, body, headers) = body_of(ApiError::TooManyRequests {
            retry_after_secs: 42,
            message: "too many attempts".to_owned(),
        })
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.error, "too_many_requests");
        assert_eq!(headers["retry-after"], "42");
    }

    #[tokio::test]
    async fn a_401_names_the_scheme_so_a_client_knows_what_to_send() {
        let (status, _, headers) = body_of(ApiError::Unauthorized("no token".to_owned())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            headers["www-authenticate"]
                .to_str()
                .expect("ascii")
                .starts_with("Bearer")
        );
    }

    #[tokio::test]
    async fn logging_in_with_no_password_configured_does_not_confirm_a_password_exists() {
        let (status, body, _) = body_of(crate::auth::LoginError::NotConfigured.into()).await;
        // 404, not 401: a prober learns "there is no admin mode here", which is true, rather than
        // "there is one and you guessed wrong".
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error, "not_found");
    }

    #[test]
    fn a_catalog_failure_is_our_fault_not_the_callers() {
        let error: ApiError = CatalogError::Failed("disk is on fire".to_owned()).into();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        // ...whereas a rejection is the caller's.
        let error: ApiError = CatalogError::Rejected("path escapes the root".to_owned()).into();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn every_variant_has_a_code_and_a_status_that_agree_about_whose_fault_it_is() {
        let cases = [
            ApiError::NotFound("x".to_owned()),
            ApiError::BadRequest("x".to_owned()),
            ApiError::Unauthorized("x".to_owned()),
            ApiError::Forbidden("x".to_owned()),
            ApiError::queue_full(),
            ApiError::TooManyRequests {
                retry_after_secs: 1,
                message: "x".to_owned(),
            },
            ApiError::Internal("x".to_owned()),
        ];
        for error in cases {
            assert!(!error.code().is_empty());
            assert!(error.status().is_client_error() || error.status().is_server_error());
        }
    }
}
