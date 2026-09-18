//! Reading a posted form.
//!
//! Deliberately not `axum::Form`. That extractor **rejects** a request whose `Content-Type` is not
//! exactly `application/x-www-form-urlencoded`, with a 415 — and a 415 is a response htmx does not
//! swap, so the button that produced it goes visually dead and the reason appears nowhere. That is
//! precisely the failure mode `handlers.rs` opens by saying it avoids, and it would have been
//! reintroduced by the extractor rather than by a handler.
//!
//! htmx does send that header, so the browser path was never in doubt. What this removes is the
//! class of failure where something else — a wrapper, a proxy, a future native client, a
//! hand-written request — does not, and the page answers by doing nothing at all. Reading the body
//! as a string and pulling the fields out of it cannot fail that way: a body that says nothing is a
//! form with no fields set, which every one of these handlers already has an answer for.
//!
//! `tools/cmd/km-package-builder` takes its bodies as strings too, for a different reason — repeated
//! keys, which `serde_urlencoded` cannot represent. Two tools, two reasons, one conclusion.

use crate::prefs::decode;

/// The first value of a field, or `None` when it is absent or empty.
///
/// Empty and absent are folded together on purpose. A text input somebody cleared posts `name=`,
/// and every caller here means the same thing by that as by not sending it at all.
pub fn field(body: &str, key: &str) -> Option<String> {
    for pair in body.split('&') {
        let (name, value) = match pair.split_once('=') {
            Some(split) => split,
            // `?multi` with no `=` is a field that is present and empty, which is still not a value.
            None => (pair, ""),
        };
        if decode(name) == key {
            let value = decode(value);
            return if value.is_empty() { None } else { Some(value) };
        }
    }
    None
}

/// Every non-empty value of a field, in the order posted.
///
/// For a group of checkboxes sharing one name, which post one pair per box that is ticked.
pub fn values(body: &str, key: &str) -> Vec<String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(name, _)| decode(name) == key)
        .map(|(_, value)| decode(value))
        .filter(|value| !value.is_empty())
        .collect()
}

/// Whether a field is present and set to something truthy.
///
/// `1` and `true` and nothing else. A checkbox that is off is not posted at all, and a hidden field
/// carrying an empty string is off — which is how the picker's `multi` flag travels.
pub fn flag(body: &str, key: &str) -> bool {
    matches!(field(body, key).as_deref(), Some("1") | Some("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_is_found_and_decoded() {
        let body = "singer=Ana+Carolina&multi=1";
        assert_eq!(field(body, "singer").as_deref(), Some("Ana Carolina"));
        assert!(flag(body, "multi"));
    }

    #[test]
    fn percent_escapes_survive_the_round_trip() {
        let body = format!("name={}", crate::prefs::encode("Águas & Março"));
        assert_eq!(field(&body, "name").as_deref(), Some("Águas & Março"));
    }

    /// A cleared text box posts `name=`, and every caller means the same thing by that as by not
    /// sending the field at all.
    #[test]
    fn an_empty_value_is_the_same_as_an_absent_one() {
        assert_eq!(field("name=&multi=1", "name"), None);
        assert_eq!(field("multi=1", "name"), None);
    }

    #[test]
    fn a_repeated_field_gives_every_value() {
        let body = "show=a&listed=a&show=b&show=";
        assert_eq!(values(body, "show"), ["a", "b"]);
        assert!(values(body, "absent").is_empty());
    }

    #[test]
    fn a_flag_is_off_unless_it_says_otherwise() {
        assert!(!flag("multi=", "multi"));
        assert!(!flag("", "multi"));
        assert!(!flag("multi=0", "multi"));
        assert!(flag("multi=true", "multi"));
    }

    /// The whole point: a body that says nothing is a form with nothing set, not an error.
    #[test]
    fn an_empty_body_is_a_form_with_no_fields() {
        assert_eq!(field("", "anything"), None);
        assert!(!flag("", "anything"));
    }

    #[test]
    fn the_first_value_wins_when_a_key_repeats() {
        assert_eq!(field("a=one&a=two", "a").as_deref(), Some("one"));
    }
}
