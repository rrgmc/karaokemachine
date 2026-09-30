//! Reading a cookie that one of the machine's pages wrote.
//!
//! **One reader for both page crates**, because they share one origin and read the same cookies.
//! The singer's remote and the owner's pages both hold the admin token and the chosen language. Two
//! readers could disagree about what a request carries, and a page would then act on a cookie the
//! other page could not see.

use axum::http::HeaderMap;
use axum::http::header::COOKIE;

/// One cookie's decoded value, or `None` when the request does not carry it.
///
/// **Every `Cookie` header, not the first.** HTTP/2 lets a client send each cookie in a header of
/// its own, so reading only the first would lose the rest.
///
/// **Percent-decoded, with `+` read as a space**, because that is how a page writes a value somebody
/// typed. A token or a language tag holds neither, so decoding leaves it as it is.
///
/// ```
/// # use axum::http::{HeaderMap, header::COOKIE};
/// let mut headers = HeaderMap::new();
/// headers.append(COOKIE, "km_locale=pt-BR".parse().unwrap());
/// headers.append(COOKIE, "km_singer=Ana+Lu%C3%ADsa".parse().unwrap());
/// assert_eq!(km_api::cookie::read(&headers, "km_singer").as_deref(), Some("Ana Luísa"));
/// assert_eq!(km_api::cookie::read(&headers, "km_token"), None);
/// ```
#[must_use]
pub fn read(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| decode(value.trim()))
}

fn decode(value: &str) -> String {
    percent_encoding::percent_decode_str(&value.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}
