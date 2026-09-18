//! What this tool will accept as a package's version, and how a build raises one.
//!
//! **Three numbers separated by dots, and nothing else.** `km_kmpkg::PackageMeta::version` is
//! free-form and `km-pack` builds from a hand-written description that says anything at all; this
//! is the *curation tool's* rule about what it will store, and it is narrow so that
//! [`Version::raised`] always has an answer. A version somebody can type is a version a build can
//! move.
//!
//! A package opened from a `.kmpkg` built elsewhere keeps whatever string it carries — see
//! `build::import`. Nothing here is applied to it, because refusing an import over a label would
//! throw away the songs to save the string.

use std::fmt;

/// A package version this tool can read and raise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// The number a person moves when a package becomes a different thing.
    pub major: u32,
    /// The number a person moves when a package becomes a new edition of the same thing.
    pub minor: u32,
    /// The number a build moves. See [`Version::raised`].
    pub patch: u32,
}

/// Reads `X.Y.Z`, or nothing.
///
/// **Round-tripped rather than validated field by field.** Parsing three `u32`s accepts `01.0.0`,
/// `+1.0.0` and `1.0.0␠`, all of which store a string that is not the one a later read produces.
/// Comparing the rendered value against the input refuses every one of those in a single rule, and
/// the rule stays right when a field is added.
pub fn parse(text: &str) -> Option<Version> {
    let mut parts = text.split('.');
    let version = Version {
        major: parts.next()?.parse().ok()?,
        minor: parts.next()?.parse().ok()?,
        patch: parts.next()?.parse().ok()?,
    };
    if parts.next().is_some() || version.to_string() != text {
        return None;
    }
    Some(version)
}

impl Version {
    /// The version the next build writes, or `None` when there is no next one.
    ///
    /// **The patch, and deliberately not the minor.** Whichever number a build moves on its own
    /// stops saying anything a person chose, so the smallest one takes it: `Z` counts builds since
    /// an edition and `Y` stays the curator's to raise when a rebuild really is a new edition. A
    /// tool that moved the minor would reach `1.40.0` in a fortnight and leave nothing below the
    /// major to declare an edition with.
    ///
    /// `None` on overflow rather than a wrap or a saturating stop: a patch number at `u32::MAX` is
    /// not a package anybody is building, and the caller already has a branch for "this one cannot
    /// be raised".
    pub fn raised(&self) -> Option<Self> {
        Some(Self {
            patch: self.patch.checked_add(1)?,
            ..*self
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The version the next build will write into the package.
///
/// **One answer, asked by two callers.** The label beside the tick box names this number and the
/// default file name is built from it, and a build that raised the version while the box on the page
/// still said the old one would put a name on the form that no file on disk ever had. Two copies of
/// the question is how they come to disagree.
///
/// The three cases are the build's own. A package that has never been built ships at the version
/// somebody typed, so the first build is exempt. A box that is not ticked spends nothing. And a
/// version that is not three numbers cannot be raised, which the label says out loud.
pub fn next(version: &str, built_before: bool, raise: bool) -> String {
    if !built_before || !raise {
        return version.to_owned();
    }
    parse(version)
        .and_then(|version| version.raised())
        .map_or_else(|| version.to_owned(), |raised| raised.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_three_numbers_and_nothing_else() {
        assert_eq!(
            parse("1.0.0"),
            Some(Version {
                major: 1,
                minor: 0,
                patch: 0
            })
        );
        assert_eq!(
            parse("10.20.30"),
            Some(Version {
                major: 10,
                minor: 20,
                patch: 30
            })
        );

        // Two parts is the one a hand-written description is most likely to carry, and the one
        // `km-pack`'s own sample warns about quoting.
        assert_eq!(parse("1.0"), None);
        assert_eq!(parse("1.0.0.0"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("v1.0.0"), None);
        assert_eq!(parse("1.0.0-beta"), None);
        assert_eq!(parse("2024-spring"), None);
        // Each of these parses as three numbers and stores a string a later read does not produce.
        // They are what the round-trip is for.
        assert_eq!(parse("01.0.0"), None);
        assert_eq!(parse("1.0.0 "), None);
        assert_eq!(parse(" 1.0.0"), None);
        assert_eq!(parse("+1.0.0"), None);
    }

    #[test]
    fn raising_moves_only_the_patch() {
        let raise = |text: &str| parse(text).and_then(|v| v.raised()).map(|v| v.to_string());

        assert_eq!(raise("1.0.0").as_deref(), Some("1.0.1"));
        assert_eq!(raise("1.2.3").as_deref(), Some("1.2.4"));
        // No carry into the minor, which is the whole point: a tenth build of edition 1.9 is
        // 1.9.10, and the edition somebody declared is still 1.9.
        assert_eq!(raise("1.9.9").as_deref(), Some("1.9.10"));
        assert_eq!(raise("2.0.7").as_deref(), Some("2.0.8"));

        assert_eq!(
            Version {
                major: 1,
                minor: 0,
                patch: u32::MAX
            }
            .raised(),
            None
        );
    }

    /// The number the label names and the number the file name carries are the same number.
    #[test]
    fn the_next_version_is_the_one_the_build_will_write() {
        // A package that has never been built ships at the version somebody typed.
        assert_eq!(next("1.0.0", false, true), "1.0.0");
        // An unticked box spends nothing.
        assert_eq!(next("1.0.0", true, false), "1.0.0");
        // A rebuild with the box ticked moves the patch, and only the patch.
        assert_eq!(next("1.0.0", true, true), "1.0.1");
        assert_eq!(next("1.9.9", true, true), "1.9.10");
        // A version this tool cannot read is one it cannot raise, and it says so rather than
        // inventing a number.
        assert_eq!(next("2024-spring", true, true), "2024-spring");
    }

    /// Every version this tool writes is one it can read again, which is what lets a build raise
    /// the same package a second time.
    #[test]
    fn a_raised_version_can_be_raised_again() {
        let mut version = parse("1.0.0").expect("a version");
        for expected in ["1.0.1", "1.0.2", "1.0.3"] {
            version = version.raised().expect("raisable");
            assert_eq!(version.to_string(), expected);
            assert_eq!(parse(&version.to_string()), Some(version));
        }
    }
}
