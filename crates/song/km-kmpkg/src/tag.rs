//! A word somebody filed a song under — `rock`, `anime`, `brasil`.
//!
//! The second dimension a catalog can be narrowed on, and deliberately unlike [`crate::Language`] in
//! every way that matters:
//!
//! * **The vocabulary is open.** A language is a row in a 184-entry table compiled into the binary,
//!   and [`crate::Language::parse`] refuses anything not in it. A tag exists because somebody typed
//!   it, so there is nothing to refuse *against* — which is why [`Tag::parse`] **normalizes** rather
//!   than validating.
//! * **A song has many.** So the filter is AND, and nothing here is an `Option`.
//! * **Nothing detects one.** A language has a detected half and a hand-set half; a tag has only the
//!   hand-set half. It is pure curation, which is why it has to survive a backup and a rebuild.
//!
//! # Why folding rather than refusing
//!
//! A slug typed by hand is a slug spelled six ways. `Forró`, `forro`, `FORRÓ` and `Forro ` are one
//! word, and a control that answers three of them with *that is not a valid tag* teaches people to
//! type the fourth rather than teaching them anything about their catalog.
//!
//! So a tag is [`km_song::text::fold`]'s output with its words joined by `-`. That function is the
//! alphabet of the whole product already — it fills the catalog's sort keys and matches FTS5's
//! `unicode61 remove_diacritics 2` — so `forro` as a tag sorts and compares like `forro` in a title,
//! and there is no second normalization to keep in step with the first.
//!
//! # …and then refusing what is not a slug
//!
//! **A tag is `[a-z0-9-]` and nothing else.** Folding is what gets most typed words there — `Forró`,
//! `FORRÓ` and `forro ` all arrive as `forro`, because [`km_song::text::fold`] maps the Latin-1
//! accents to ASCII — but `fold` passes through every alphanumeric it has no mapping for, so without
//! this check a Cyrillic or Japanese word would become a tag.
//!
//! Two reasons it does not.
//!
//! * **A slug is what was asked for**, and a slug is an ASCII thing. It is typed into a URL, read
//!   back off one, and shown in a chip a few characters wide; a vocabulary somebody cannot type on
//!   the keyboard in front of them is a vocabulary they cannot filter by.
//! * **A tag is not a title.** The obvious objection is that this makes the one place in the product
//!   where a title may hold a character a tag may not — and that is true and is the point: a title is
//!   *the song's own name* and has to be stored as it is, whereas a tag is a word chosen from a
//!   vocabulary to file things under. The two are different kinds of string and the constraint
//!   follows the difference.
//!
//! **Refused rather than stripped**, which matters for the words this actually catches. Stripping
//! would turn `rock日本` into `rock` — a tag that looks right, filters wrongly, and says nothing —
//! where `None` lets every caller that should complain complain by name. `km-pack` and the curation
//! tool both do; a `?tags=` on a browsing surface drops it, exactly as it drops a typo.
//!
//! **The Latin letters `fold` has no entry for go with them**: `ß`, `ø`, `æ`, `ł` and the rest are
//! not in its table, so `Straße` is refused rather than stored as `straße`. Extending that table is
//! not the fix — it is the alphabet of every title, sort key and search box in the product, and
//! widening it for tags would change all four. Somebody types `strasse`, which is the spelling they
//! would have to search for anyway.

use km_song::text::fold;

/// Whether a folded, joined string is a slug: ASCII lower case, digits and `-`.
///
/// **Only the character set is checked, not the shape**, and that is not an omission: `fold` turns
/// every non-alphanumeric into a space, so the only hyphens in the string are the ones
/// [`Tag::parse`] put between non-empty words. A leading, trailing or doubled `-` is therefore not
/// reachable, and a check for it would be a claim about this function's caller rather than about the
/// string.
fn is_slug(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

/// The longest a tag may be, in characters, after folding.
///
/// A person picking from a list is the reader here, not a database: forty characters is already
/// longer than any tag anybody wants to see in a chip, and a bound means a hand-edited manifest
/// cannot make one unbounded.
pub const MAX_LENGTH: usize = 40;

/// The most tags one song may carry.
///
/// Same argument as [`MAX_LENGTH`] one level up: nothing about the storage needs a limit, and a row
/// that can grow without one is a row a hand-edited manifest can make arbitrarily large.
pub const MAX_PER_SONG: usize = 32;

/// A song tag, normalized.
///
/// Constructed only through [`Tag::parse`], so a value of this type is always a legal tag: non-empty,
/// no longer than [`MAX_LENGTH`], and matching `[a-z0-9]([a-z0-9-]*[a-z0-9])?` — ASCII lower case,
/// digits, and single hyphens between words.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(String);

impl Tag {
    /// Folds a typed word into a tag, or `None` if what comes out is not one.
    ///
    /// Three ways to get `None`, and they are not the same kind of answer:
    ///
    /// * **Nothing survived the fold** — the string was empty or was all punctuation. There was no
    ///   tag in it to begin with.
    /// * **It is longer than [`MAX_LENGTH`]**.
    /// * **It is not ASCII.** `fold` maps the Latin-1 accents and passes everything else through, so
    ///   this is what stops a Cyrillic or Japanese word — or a `ß` — becoming a tag. See the module
    ///   doc for why a tag is held to this where a title is not.
    ///
    /// What it never means is *you spelled that wrong*: `Forró`, `FORRÓ` and `forro ` are all the
    /// tag `forro`, and none of them is a mistake.
    #[must_use]
    pub fn parse(value: &str) -> Option<Tag> {
        let folded = fold(value);
        if folded.is_empty() {
            return None;
        }
        let slug: String = folded
            .split(' ')
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if slug.is_empty() || slug.len() > MAX_LENGTH || !is_slug(&slug) {
            return None;
        }
        Some(Tag(slug))
    }

    /// The slug, as it is stored everywhere.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The slug, consuming the tag.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reads a comma-joined list of tags — the one wire spelling, `rock,brasil`.
///
/// **A comma is the separator everywhere a set of tags travels**: the `?tags=` parameter on every
/// browsing surface, the packed `songs.tags` column, and the `--index` CSV's own cell. That is
/// unambiguous by construction rather than by convention — [`Tag::parse`] folds a comma to a word
/// break, so no tag can contain one.
///
/// Unreadable words are dropped rather than failing the whole list, which is the same judgement
/// `?language=` makes about a code nobody has: a filter naming one real tag and one typo is a
/// narrower list, not a 400.
#[must_use]
pub fn parse_list(value: &str) -> Vec<Tag> {
    let mut tags: Vec<Tag> = value.split(',').filter_map(Tag::parse).collect();
    tags.sort();
    tags.dedup();
    tags.truncate(MAX_PER_SONG);
    tags
}

/// Writes a set of tags to the wire spelling: sorted, de-duplicated, comma-joined.
///
/// Sorted so that two songs with the same tags in a different order hold the same string, which is
/// what lets the packed column be compared and hashed rather than parsed.
#[must_use]
pub fn join(tags: &[Tag]) -> String {
    let mut sorted: Vec<&str> = tags.iter().map(Tag::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_and_case_fold_to_one_tag() {
        let expected = Tag::parse("forro");
        assert!(expected.is_some());
        assert_eq!(Tag::parse("Forró"), expected);
        assert_eq!(Tag::parse("FORRÓ"), expected);
        assert_eq!(Tag::parse("  forro  "), expected);
    }

    #[test]
    fn words_are_joined_with_a_dash() {
        assert_eq!(Tag::parse("Rock & Roll").unwrap().as_str(), "rock-roll");
        assert_eq!(Tag::parse("anos 80").unwrap().as_str(), "anos-80");
        assert_eq!(Tag::parse("já--foi").unwrap().as_str(), "ja-foi");
    }

    #[test]
    fn nothing_survives_an_empty_or_punctuation_only_word() {
        assert_eq!(Tag::parse(""), None);
        assert_eq!(Tag::parse("   "), None);
        assert_eq!(Tag::parse("!!!"), None);
        assert_eq!(Tag::parse("---"), None);
    }

    #[test]
    fn a_tag_longer_than_the_bound_is_refused() {
        let long = "a".repeat(MAX_LENGTH);
        assert_eq!(Tag::parse(&long).unwrap().as_str(), long);
        assert_eq!(Tag::parse(&"a".repeat(MAX_LENGTH + 1)), None);
    }

    #[test]
    fn a_tag_can_never_contain_the_separator() {
        // Which is what makes the comma-joined column and the `?tags=` parameter unambiguous.
        assert_eq!(Tag::parse("rock,brasil").unwrap().as_str(), "rock-brasil");
    }

    #[test]
    fn a_list_is_sorted_and_deduplicated() {
        let tags = parse_list("Rock, brasil ,ROCK,,rock");
        let joined = join(&tags);
        assert_eq!(joined, "brasil,rock");
    }

    #[test]
    fn a_list_stops_at_the_bound() {
        let many: String = (0..MAX_PER_SONG + 10)
            .map(|n| format!("tag{n}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(parse_list(&many).len(), MAX_PER_SONG);
    }

    /// A tag is a slug, and a slug is ASCII.
    ///
    /// `fold` maps the Latin-1 accents and passes everything else through, so this is the check that
    /// stops what it did not map. The first two lines are the pair that make the point: accented
    /// Latin *is* a tag, because it folds to ASCII, and everything else is not.
    #[test]
    fn a_tag_is_ascii_or_it_is_not_a_tag() {
        assert_eq!(Tag::parse("Forró").unwrap().as_str(), "forro");
        assert_eq!(Tag::parse("Coração").unwrap().as_str(), "coracao");

        for refused in ["日本", "рок", "ελληνικά", "Straße"] {
            assert_eq!(Tag::parse(refused), None, "{refused:?} is not a slug");
        }
    }

    /// A word with one non-ASCII letter in it is **refused, not stripped**.
    ///
    /// The distinction that makes the check worth having rather than a filter: stripping would turn
    /// this into `rock`, which looks right, filters wrongly, and says nothing to anybody. `None`
    /// lets `km-pack` and the curation tool refuse it by name.
    #[test]
    fn a_partly_ascii_word_is_refused_rather_than_trimmed_down() {
        assert_eq!(Tag::parse("rock日本"), None);
        assert_eq!(Tag::parse("日本 rock"), None);
    }
}
