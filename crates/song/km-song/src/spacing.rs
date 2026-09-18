//! The underscore a karaoke writer uses to say something about a space.
//!
//! A great many files carry underscores inside their lyric events, and every one of them is markup
//! about spacing rather than a character anybody sings. Unread, it is drawn across the television,
//! wiped in time with the music, published on the `lyric_line` event, stored in the package's
//! preview, printed in the song book and indexed for search — so `Se apronta` reaches a singer as
//! `Se_apronta`.
//!
//! **It carries two opposite meanings, and which one is meant is decided by position.** Both are in
//! the corpus, and a rule keyed on the character alone corrupts one of them:
//!
//! - **An elided space**, where the word carries on either side of the mark. Portuguese sings two
//!   words across one note and the writer joins them so the syllable stays one timing point:
//!   `Se_a` + `pron` + `ta ` is `Se apronta`. This is nearly all of it.
//! - **A cancelled space**, where nothing but whitespace follows the mark. A writer who ends every
//!   syllable with a space needs a way to say *not this one*: `MEL_ ` + `O_ ` + `DY ` is `MELODY`,
//!   and reading the mark as a space instead would give `MEL O DY`.
//!
//! A third shape is a mark that landed on neither, almost always on the syllable after the one it
//! belonged to — `do ` + `_a ` + `ve` + `jo ` for `do a vejo`, where the space it asks for is
//! already there. The mark goes and the spacing around it stays as written, which is the reading
//! that costs nothing whichever was meant.
//!
//! **A run of marks is one mark**, and everything here walks characters rather than bytes, so a
//! syllable is never cut through the middle of one.

/// What one space mark says about a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceMark {
    /// A space the writer elided: alphanumeric text on both sides of the mark.
    Elided,
    /// A space the writer cancelled: nothing but whitespace after the mark.
    Cancelled,
    /// A mark that asks for neither, and whose spacing is already as it should be.
    Stray,
}

impl SpaceMark {
    /// The name this shape is counted under.
    pub fn name(self) -> &'static str {
        match self {
            Self::Elided => "elided space",
            Self::Cancelled => "cancelled space",
            Self::Stray => "stray mark",
        }
    }
}

/// Every space mark in one syllable's text, in the order they were written.
///
/// Empty for the overwhelming majority of syllables, which is what makes it cheap enough to ask of
/// every one. `km-lyrics scan` counts these across a corpus; once [`resolve`] runs where a syllable
/// is written, the answer over a whole corpus is nothing, and that is the check that the rule
/// covers every shape rather than the two that were looked for.
pub fn marks(text: &str) -> Vec<SpaceMark> {
    if !text.contains(MARK) {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    walk(&chars, |mark, _, _| out.push(mark));
    out
}

/// Resolves the space marks in one syllable's text into the spacing they ask for.
///
/// Applied where a syllable is first written rather than where a line is read, for the reason
/// [`crate::timeline::build_timeline`]'s redaction gives: the display draws the current line from
/// the syllables directly so it can wipe one at a time, and the API puts the same field on the
/// wire, so a rule applied at the joining would leave the mark on the television and hide it only
/// from whatever nobody was looking at. **Timing is untouched** — a syllable whose text loses a
/// character keeps its ticks.
///
/// The text comes back unchanged when it holds no mark, which is nearly always.
pub fn resolve(text: &str) -> String {
    if !text.contains(MARK) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0usize;
    walk(&chars, |mark, start, end| {
        out.extend(chars.get(copied..start).unwrap_or_default());
        if mark == SpaceMark::Elided {
            out.push(' ');
        }
        // The whitespace after a cancelled space is the space it cancels, so it goes with the mark.
        // Nothing but whitespace can follow one, which is what made it that shape.
        copied = if mark == SpaceMark::Cancelled {
            chars.len()
        } else {
            end
        };
    });
    out.extend(chars.get(copied..).unwrap_or_default());
    out
}

/// The character the convention is written with.
const MARK: char = '_';

/// Calls `found` for each run of marks, with the run's character range.
fn walk(chars: &[char], mut found: impl FnMut(SpaceMark, usize, usize)) {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != MARK {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i] == MARK {
            i += 1;
        }
        let before = start.checked_sub(1).map(|p| chars[p]);
        let after = chars.get(i).copied();
        let mark = match (before, after) {
            (Some(b), Some(a)) if b.is_alphanumeric() && a.is_alphanumeric() => SpaceMark::Elided,
            _ if chars[i..].iter().all(|c| c.is_whitespace()) => SpaceMark::Cancelled,
            _ => SpaceMark::Stray,
        };
        found(mark, start, i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_underscore_between_letters_is_the_space_it_stands_for() {
        assert_eq!(resolve("Se_a"), "Se a");
        assert_eq!(resolve("de_a"), "de a");
        assert_eq!(marks("Se_a"), vec![SpaceMark::Elided]);
    }

    #[test]
    fn a_trailing_underscore_takes_the_space_after_it() {
        // The three syllables of MELODY, from a file that ends every syllable with a space.
        let joined: String = ["MEL_ ", "O_ ", "DY "].iter().map(|s| resolve(s)).collect();
        assert_eq!(joined, "MELODY ");
        assert_eq!(marks("MEL_ "), vec![SpaceMark::Cancelled]);
    }

    #[test]
    fn an_underscore_at_the_very_end_is_a_cancelled_space_too() {
        assert_eq!(resolve("TO_"), "TO");
    }

    #[test]
    fn a_leading_underscore_goes_and_leaves_the_spacing_alone() {
        let joined: String = ["do ", "_a ", "ve", "jo "]
            .iter()
            .map(|s| resolve(s))
            .collect();
        assert_eq!(joined, "do a vejo ");
        assert_eq!(marks("_a "), vec![SpaceMark::Stray]);
    }

    #[test]
    fn a_mark_before_a_space_that_is_not_the_end_only_goes_itself() {
        assert_eq!(resolve("a_ b"), "a b");
    }

    #[test]
    fn a_run_of_marks_is_one_mark() {
        assert_eq!(resolve("a__b"), "a b");
        assert_eq!(marks("a__b"), vec![SpaceMark::Elided]);
    }

    #[test]
    fn a_syllable_that_is_only_marks_comes_back_empty() {
        assert_eq!(resolve("_"), "");
        assert_eq!(resolve("___"), "");
    }

    #[test]
    fn resolving_does_not_cut_a_multibyte_character() {
        assert_eq!(resolve("não_é"), "não é");
        assert_eq!(resolve("coração_ "), "coração");
    }

    #[test]
    fn text_with_no_mark_comes_back_as_it_was() {
        assert_eq!(resolve("pra recomecar "), "pra recomecar ");
        assert!(marks("pra recomecar ").is_empty());
    }

    #[test]
    fn several_marks_in_one_syllable_are_each_read_on_their_own() {
        assert_eq!(
            marks("a_b_ "),
            vec![SpaceMark::Elided, SpaceMark::Cancelled]
        );
        assert_eq!(resolve("a_b_ "), "a b");
    }
}
