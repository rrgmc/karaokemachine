//! Whether one tune may be passed on.
//!
//! The Open Hymnal states a copyright status per tune, in a `C: copyright: …` line of the tune's
//! own source, and it splits the question the way it actually divides: **music, setting, words and
//! translation each hold a separate status**. Most of its tunes are public domain in all four. Some
//! are not, and the ones that are not look exactly like the ones that are until you read the line.
//!
//! This module is shaped after `tools/cmd/assets/km-wallpaper-pack/src/license.rs`, and keeps the two rules
//! that one established, because they are the same rules and this is the same question:
//!
//! 1. **The allow-list is a constant in the code and never a config key**, so that widening it is a
//!    change somebody reviews rather than a setting somebody types.
//! 2. **It fails closed.** A status this module does not positively recognize is [`Restricted`],
//!    not "probably fine" — an item whose license nobody wrote down is exactly the thing that must
//!    not ship.
//!
//! The case that shows why it earns its keep is real and is in the pinned edition: `X:72`,
//! *Twas In The Moon of Wintertime*, whose words and music are public domain and whose **setting**
//! is CPDL's. CPDL's default terms are CC BY-SA, which this project already declines (see the
//! `Where a wallpaper pack's photographs may come from` decision in `docs/decisions/`). It is not in
//! the pack, and `the_cpdl_setting_is_refused` is the guard that it cannot drift in.
//!
//! [`Restricted`]: Redistribution::Restricted

/// What may be done with a tune.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redistribution {
    /// Public domain in every layer the source names. This pack may carry it.
    PublicDomain,
    /// Anything else, with the reason in the source's own words.
    Restricted(String),
}

impl Redistribution {
    /// Whether the pack may carry this tune.
    #[must_use]
    pub fn is_public_domain(&self) -> bool {
        matches!(self, Self::PublicDomain)
    }
}

/// The only statements accepted as "public domain in every layer".
///
/// Matched against the whole `copyright:` value with the punctuation and the Open Hymnal's own
/// trailing provenance sentence removed. Deliberately a short list of *whole* statements rather
/// than a search for the words "public domain" anywhere: `music and setting public domain. Words:
/// Copyright 2010, …` contains that phrase and is not what this pack may take.
const PUBLIC_DOMAIN: [&str; 2] = ["public domain", "music & lyrics public domain"];

/// Phrases that make a status restricted however the rest of the line reads.
///
/// A belt to the allow-list's braces. Any of these appearing anywhere in the copyright value is a
/// refusal even if the value would otherwise have matched, because all three name a *live* right
/// held by somebody: an active copyright, a share-alike body whose terms are CC BY-SA, and a
/// permission granted for one purpose rather than to everybody.
const RESTRICTED_MARKERS: [&str; 3] = ["copyright 1", "copyright 2", "cpdl"];

/// Reads the copyright status out of a tune's `C:` lines.
///
/// The Open Hymnal writes several `C:` lines per tune — attribution prose for the words and the
/// music, then one that begins `copyright:`. Only the last is a statement of status; the others are
/// credits and belong in `CREDITS.md`. A tune with no `copyright:` line at all is
/// [`Redistribution::Restricted`], not an oversight to wave through.
#[must_use]
pub fn assess(copyright_lines: &[String]) -> Redistribution {
    let Some(value) = status_line(copyright_lines) else {
        return Redistribution::Restricted("the source states no copyright status".to_owned());
    };

    let folded = value.to_lowercase();
    if let Some(marker) = RESTRICTED_MARKERS.iter().find(|m| folded.contains(**m)) {
        return Redistribution::Restricted(format!("names a live right ({marker}): {value}"));
    }

    let trimmed = trim_statement(&folded);
    if PUBLIC_DOMAIN.contains(&trimmed.as_str()) {
        Redistribution::PublicDomain
    } else {
        Redistribution::Restricted(format!("unrecognized status: {value}"))
    }
}

/// The value of the tune's `copyright:` line, if it has one.
fn status_line(copyright_lines: &[String]) -> Option<String> {
    copyright_lines.iter().find_map(|line| {
        let folded = line.to_lowercase();
        let at = folded.find("copyright:")?;
        Some(line[at + "copyright:".len()..].trim().to_owned())
    })
}

/// Strips the Open Hymnal's own provenance sentence and trailing punctuation.
///
/// Every tune's status ends `. This score is a part of the Open Hymnal Project, <year> Revision.`,
/// which says nothing about rights. Cutting at the first sentence keeps the comparison a comparison
/// of statements rather than of editions.
fn trim_statement(folded: &str) -> String {
    let head = folded.split('.').next().unwrap_or(folded);
    head.trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_ordinary_case_is_public_domain() {
        let c = lines(&[
            "Words: Josef Mohr, 1818. stanzas 1,3 Translated by John Freeman Young, 1863.",
            "Music: 'Stille Nacht' Franz Xaver Gruber, 1818.  Setting: \"Concordia\", 1908.",
            "copyright: public domain.  This score is a part of the Open Hymnal Project, 2005 Revision.",
        ]);
        assert_eq!(assess(&c), Redistribution::PublicDomain);
    }

    /// `X:72` in the pinned edition. Words and music are free and the *setting* is not, which is
    /// the whole reason this gate reads four layers rather than one.
    #[test]
    fn the_cpdl_setting_is_refused() {
        let c = lines(&[
            "copyright: Music & Lyrics public domain. Setting: CPDL (see http://www2.cpdl.org/).",
            "This score is a part of the Open Hymnal Project, 2013 Revision.",
        ]);
        assert!(!assess(&c).is_public_domain());
    }

    /// The shape that would slip past a search for the words "public domain".
    #[test]
    fn a_live_words_copyright_is_refused_despite_the_phrase() {
        let c = lines(&[
            "copyright: music and setting public domain.  Words: Copyright 2010, A. Person. \
             These lyrics may be freely reproduced for Christian worship.",
        ]);
        assert!(!assess(&c).is_public_domain());
    }

    #[test]
    fn silence_is_not_permission() {
        let c = lines(&["Words: somebody, 1870.", "Music: somebody else, 1871."]);
        assert!(!assess(&c).is_public_domain());
        assert!(assess(&[]).is_public_domain().eq(&false));
    }

    #[test]
    fn the_provenance_sentence_does_not_change_the_answer() {
        let with = lines(&["copyright: public domain.  This score is a part of the Open Hymnal."]);
        let without = lines(&["copyright: public domain"]);
        assert_eq!(assess(&with), assess(&without));
        assert_eq!(assess(&without), Redistribution::PublicDomain);
    }
}
