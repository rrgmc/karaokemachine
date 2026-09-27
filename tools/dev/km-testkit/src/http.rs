//! An axum router driven as a service, with no socket bound.
//!
//! **A test reaches the real router and not a copy of its routes.** `tower`'s `oneshot` hands one
//! request to the same `Router` a server would serve. What comes back is everything a browser would
//! see, so a test can ask for the part it is about.
//!
//! Each crate keeps the request shapes that are its own: a bearer token, a peer address, a
//! `sec-fetch-site` header. What is here is what every one of them repeated.

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::ServiceExt as _;

/// The media type of a form post, as a browser and htmx send it.
pub const FORM: &str = "application/x-www-form-urlencoded";

/// The boundary [`multipart`] writes between parts.
pub const BOUNDARY: &str = "----km-testkit-boundary";

/// What a router answered, whole.
#[derive(Debug)]
pub struct Answer {
    /// The status line's code.
    pub status: StatusCode,
    /// Every header, `Location` and `Set-Cookie` among them.
    pub headers: HeaderMap,
    /// The body, unread by anything.
    pub body: Bytes,
}

impl Answer {
    /// The body as text.
    ///
    /// Lossy, so a test of a page never fails on a stray byte. A PDF or a PNG wants [`Self::body`].
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// One header's value, when it is there and is text.
    #[must_use]
    pub fn header(&self, name: impl header::AsHeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// Where the answer sent the browser next, or nothing.
    ///
    /// A form post answers `303` and carries its notice in this URL's query string, so this is the
    /// part of a redirect worth asserting.
    #[must_use]
    pub fn location(&self) -> &str {
        self.header(header::LOCATION).unwrap_or_default()
    }
}

/// Sends one request through a router and reads everything that comes back.
///
/// # Panics
///
/// When the router or its body fails, which no test here expects of a router.
pub async fn send(app: Router, request: Request<Body>) -> Answer {
    let response = app.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the body");
    Answer {
        status,
        headers,
        body,
    }
}

/// A `GET` of one path.
///
/// # Panics
///
/// When `uri` is not a URI, which is a fault in the test.
#[must_use]
pub fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).expect("a request")
}

/// A form `POST` of an encoded body, carrying the media type a browser sends.
///
/// # Panics
///
/// When `uri` is not a URI, which is a fault in the test.
#[must_use]
pub fn form(uri: &str, body: impl Into<Body>) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, FORM)
        .body(body.into())
        .expect("a request")
}

/// A `multipart/form-data` body, and the media type that names its boundary.
///
/// Each part is a field name, a file name when the part is a file, and its bytes. The body is built
/// by hand, so a test of an upload route is a test of what that route really parses.
#[must_use]
pub fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> (String, Vec<u8>) {
    let mut body: Vec<u8> = Vec::new();
    for (name, file_name, bytes) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let disposition = match file_name {
            Some(file_name) => {
                format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"")
            }
            None => format!("Content-Disposition: form-data; name=\"{name}\""),
        };
        body.extend_from_slice(disposition.as_bytes());
        body.extend_from_slice(b"\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}
