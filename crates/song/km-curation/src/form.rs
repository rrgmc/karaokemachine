//! Reading `application/x-www-form-urlencoded` bodies, repeated keys included.
//!
//! Axum's `Form` extractor goes through `serde_urlencoded`, which **cannot represent a repeated
//! key**. That is not an exotic case here: it is what a form of ticked checkboxes sends. Selecting
//! four songs posts `song_id=a&song_id=b&song_id=c&song_id=d`, and asking serde for a `Vec<String>`
//! answers
//!
//! ```text
//! Failed to deserialize form body: song_id: invalid type: string "…", expected a sequence
//! ```
//!
//! while asking for a `HashMap` is worse — it silently keeps one and drops the rest, so ticking three
//! categories would file the song under one and nobody would be told.
//!
//! So the body is read as text and turned into a list of pairs here, which is the only shape that
//! matches what a browser actually sends.
//!
//! **The splitting and the unescaping are `form_urlencoded`'s**, not this module's. They were
//! hand-rolled — a `%XX` decoder, a hex-nibble pair and a `+`-aware walk, about sixty lines — beside
//! a second, independently-written copy of the same rules in `km-remote-pages`. What is genuinely
//! this crate's is `Fields` itself, and the argument in the header above is about *repeated keys*
//! rather than about parsing: it is `serde_urlencoded`'s `Deserialize` that cannot represent one,
//! where `form_urlencoded::parse` is an iterator of pairs and handles them natively. The crate was
//! already in the tree, pulled in by `serde_urlencoded`, so this costs no compilation.
//!
//! **Two readers share one body, deliberately.** A
//! browse action's POST carries the ticked rows *and* the filter bar: this reads the first, because
//! `song_id` arrives a hundred times, and `serde_urlencoded` reads the second into `FilterQuery`,
//! because a flat struct of twenty fields should not be a twenty-first list of field names in this
//! crate. They parse the same string in the same request and agree about `+` and `%XX`, which is the
//! only thing they have to agree about.

/// The fields of one submitted form, in the order they arrived.
#[derive(Debug, Clone, Default)]
pub struct Fields(Vec<(String, String)>);

impl Fields {
    /// Parses a form body.
    ///
    /// A key with no `=` is kept with an empty value, which is what a browser sends for an empty
    /// `<input>` in some cases and what `<button name="x">` sends with no value attribute.
    ///
    /// **`form_urlencoded::parse` does the splitting and the unescaping**, rather than a walk over
    /// the string with a hand-rolled `%XX` decoder beside it. What this crate keeps is `Fields`:
    /// the module header's argument is about *repeated keys*, and it is `serde_urlencoded`'s
    /// `Deserialize` that cannot represent one — `form_urlencoded::parse` is an iterator of pairs
    /// and handles them natively, which is exactly what a form of ticked checkboxes needs. The
    /// crate is already in the tree, pulled in by `serde_urlencoded` itself, so leaning on it adds no
    /// dependency and leaves one fewer codec to keep right.
    pub fn parse(body: &str) -> Self {
        Self(
            form_urlencoded::parse(body.as_bytes())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect(),
        )
    }

    /// The first value for a key, exactly as it was typed.
    ///
    /// Empty when the key is absent, which is the same as an empty box — for a form where every box
    /// is always submitted, those two really are the same thing.
    pub fn text(&self, key: &str) -> &str {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
    }

    /// The first value for a key, trimmed, or `None` when it is absent or blank.
    pub fn one(&self, key: &str) -> Option<&str> {
        let value = self.text(key).trim();
        (!value.is_empty()).then_some(value)
    }

    /// Whether the key was submitted at all.
    ///
    /// This is how a checkbox is read: an unticked box sends nothing, so presence *is* the value.
    pub fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(name, _)| name == key)
    }

    /// Every value for a key, blanks dropped.
    pub fn all(&self, key: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
            .collect()
    }

    /// The first value for a key, parsed, or `None` when it is absent or will not parse.
    pub fn parsed<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.one(key).and_then(|value| value.parse().ok())
    }
}

/// Percent-encodes a value for a query string.
///
/// `form_urlencoded`'s own serializer, which is the exact inverse of the parser above — so a value
/// that goes out through this and comes back through that is the value that was typed, and there is
/// no second opinion about `+`, `~` or a non-ASCII byte for the two to disagree over. It writes into
/// a `String` and hands it back, so the odd-looking empty key is just how that builder is addressed
/// when only the value is wanted.
pub fn encode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case the axum `Form` extractor could not express, and the reason this module exists.
    #[test]
    fn a_repeated_key_keeps_every_value() {
        let fields = Fields::parse("package_id=vol1&song_id=aaa&song_id=bbb&song_id=ccc");
        assert_eq!(fields.one("package_id"), Some("vol1"));
        assert_eq!(fields.all("song_id"), vec!["aaa", "bbb", "ccc"]);
    }

    #[test]
    fn an_unticked_checkbox_sends_nothing_and_that_is_the_answer() {
        let fields = Fields::parse("favorite=1");
        assert!(fields.has("favorite"));
        assert!(!fields.has("unpackaged"));
    }

    #[test]
    fn plus_is_a_space_and_percent_escapes_decode() {
        let fields = Fields::parse("title=Banho+de+Espuma&artist=Rita+Lee");
        assert_eq!(fields.one("title"), Some("Banho de Espuma"));
        assert_eq!(fields.one("artist"), Some("Rita Lee"));
        // Through `Fields` rather than a bare decoder, which is what the crate actually exercises.
        let fields = Fields::parse("title=%C3%81guas+de+Mar%C3%A7o&artist=a%26b%3Dc");
        assert_eq!(fields.one("title"), Some("Águas de Março"));
        assert_eq!(fields.one("artist"), Some("a&b=c"));
    }

    #[test]
    fn an_ampersand_in_a_value_does_not_split_the_field() {
        let fields = Fields::parse("title=Rock+%26+Roll&artist=x");
        assert_eq!(fields.one("title"), Some("Rock & Roll"));
        assert_eq!(fields.one("artist"), Some("x"));
    }

    #[test]
    fn an_empty_box_is_absent_rather_than_a_value() {
        let fields = Fields::parse("title=Something&artist=&language=++");
        assert_eq!(fields.one("artist"), None);
        assert_eq!(fields.one("language"), None);
        // ...but it was submitted, which is what tells "clear this" apart from "do not touch it".
        assert!(fields.has("artist"));
        assert_eq!(fields.text("artist"), "");
    }

    /// A `%` that is not an escape stays a `%`, which is the leniency the hand-rolled decoder had.
    ///
    /// Kept when that decoder was replaced by `form_urlencoded::parse`, because it is the one
    /// behaviour the swap could have changed without any other test noticing -- somebody typing
    /// `100%` into the search box is not writing an escape sequence.
    #[test]
    fn a_stray_percent_is_a_literal_percent() {
        let fields = Fields::parse("a=100%&b=50%off&c=%zz");
        assert_eq!(fields.one("a"), Some("100%"));
        assert_eq!(fields.one("b"), Some("50%off"));
        assert_eq!(fields.one("c"), Some("%zz"));
    }

    #[test]
    fn a_key_with_no_equals_sign_is_still_a_key() {
        let fields = Fields::parse("force");
        assert!(fields.has("force"));
        assert_eq!(fields.text("force"), "");
    }

    #[test]
    fn an_empty_body_yields_nothing() {
        let fields = Fields::parse("");
        assert!(!fields.has("anything"));
        assert!(fields.all("song_id").is_empty());
    }

    #[test]
    fn numbers_parse_or_do_not() {
        let fields = Fields::parse("number=9001&bad=nine");
        assert_eq!(fields.parsed::<u32>("number"), Some(9001));
        assert_eq!(fields.parsed::<u32>("bad"), None);
        assert_eq!(fields.parsed::<u32>("missing"), None);
    }

    #[test]
    fn encoding_and_decoding_round_trip() {
        for value in ["Tom Jobim", "Águas de Março", "a&b=c", "100%", "x+y"] {
            // Out through `encode` and back through the parser, which is the pair that has to agree:
            // `encode` writes query strings this tool then reads back as forms.
            let fields = Fields::parse(&format!("v={}", encode(value)));
            assert_eq!(fields.text("v"), value, "round trip of {value:?}");
        }
    }
}
