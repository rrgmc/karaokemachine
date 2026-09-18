//! Taking somebody's contact details out of the words.
//!
//! A great many files in the wild carry the sequencer's business card inside the lyric track — an
//! email address, a web address, a telephone number — and it is sung. Not sung *about*: the machine
//! draws it across the television, wipes it in time with the music, publishes it on the
//! `lyric_line` event, stores it in the package's preview, prints it in the song book's
//! `INÍCIO DA LETRA` column and indexes it for search. A stranger's address ends up in every one of
//! those, on a machine that stranger has never heard of.
//!
//! **This is the third contact-detail rule in the workspace, and the three are deliberately not
//! one.** The other two are booleans about a whole line and are load-bearing where they are:
//!
//! - [`crate::looks_like_a_banner`] decides whether a *leading* line is worth showing as a
//!   package's preview. Eleven rules, tuned for a position where a false positive costs one line.
//! - `km_suitability`'s `is_credit_line` decides whether a line counts towards how much lyric a
//!   file has, which is an input to the stored 0–10 suitability. **A rule added there moves
//!   suitabilities in packages that already exist** — see `docs/architecture/song.md`.
//!
//! This one answers a different question again: *which characters* are contact details, so the rest
//! of the line can survive. It returns spans rather than a verdict, and it is the only one of the
//! three that rewrites anything.
//!
//! **Masking, not dropping.** A line is not thrown away, because the shape that matters is not only
//! the pure business card — it is the address sitting inside something otherwise singable, where
//! dropping the line loses words the singer needed. A false positive costs a word.

use std::ops::Range;

/// What replaces a redacted span.
///
/// One em dash for the whole span, not one per syllable it crossed: the reader is being told
/// something was removed, once, and a row of dashes reads as damage rather than as a redaction.
pub const MASK: &str = "—";

/// The byte ranges of `line` that hold contact details, in order and never overlapping.
///
/// Empty when the line is clean, which is the overwhelmingly common case and is why this is worth
/// checking before allocating anything.
///
/// **The rules are the three `is_credit_line` already names**, deliberately, so that a line this
/// redacts is a line that already counted as a credit rather than as lyric. Two of them are
/// straightforward as spans; the third is not, and the difference is the whole of the care here.
///
/// - **An email address**: the whitespace-delimited token containing an `@` whose next token is a
///   dotted host. The token, not the rest of the line — `someone@example.com (0**19) 5550123` is
///   the *shape* of one line in the corpus (redacted here, being a stranger's), and the address is
///   the first token of it rather than all three.
/// - **A web address**: a token holding `http://`, `https://` or `www.`.
/// - **A telephone number**: a token of seven or more digits and their separators — **but only on a
///   line that already carries an address, or that holds no word outside its contact details.**
///   That restriction is the one judgment in this module, and it exists because a bare number span
///   is ambiguous in a way the other two are not: `867-5309` is a lyric, and a span rule with no
///   context deletes the hook of a song somebody came to sing. Note that `is_credit_line`'s digit
///   *ratio* does not save you here — `867-5309 I got it` passes it exactly, seven digits against
///   fourteen solid characters — which is why this asks a different question rather than borrowing
///   that one. Setting a line aside costs a syllable count; masking it costs the song.
pub fn contact_spans(line: &str) -> Vec<Range<usize>> {
    let mut spans: Vec<Range<usize>> = Vec::new();
    let mut addressed = false;

    for (start, token) in tokens(line) {
        let lower = token.to_lowercase();
        if is_web(&lower) || is_email(&lower) {
            spans.push(start..start + token.len());
            addressed = true;
        }
    }

    // A line with no address on it, which is nonetheless nothing but contact details.
    //
    // **Not `is_credit_line`'s digit ratio, and the difference is a song.** That rule asks whether
    // at least half the solid characters are digits, which `867-5309 I got it` passes exactly —
    // seven digits against fourteen. It is the right rule where it lives, because setting that line
    // aside costs a syllable count; it is the wrong one here, because masking the span takes the
    // hook out of a song somebody came to sing. So this asks the stricter question: is there a
    // *word* on this line that is not part of the contact details? If there is, the line is
    // somebody's lyric with a number in it and nothing here is touched.
    let contact_shaped = !line.split_whitespace().any(|token| {
        let lower = token.to_lowercase();
        !is_telephone(token)
            && !is_web(&lower)
            && !is_email(&lower)
            && token.chars().any(char::is_alphabetic)
    });

    if addressed || contact_shaped {
        for (start, token) in tokens(line) {
            let range = start..start + token.len();
            if spans.iter().any(|span| overlaps(span, &range)) {
                continue;
            }
            if is_telephone(token) {
                spans.push(range);
            }
        }
    }

    spans.sort_by_key(|span| span.start);
    spans
}

/// `line` with every [`contact_spans`] range replaced by [`MASK`], or `None` when it is clean.
///
/// `None` rather than an unchanged copy so a caller can tell "nothing to do" from "redacted to the
/// same thing", and so the common case allocates nothing.
pub fn redact(line: &str) -> Option<String> {
    let spans = contact_spans(line);
    if spans.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(line.len());
    let mut cursor = 0usize;
    for span in spans {
        out.push_str(&line[cursor..span.start]);
        out.push_str(MASK);
        cursor = span.end;
    }
    out.push_str(&line[cursor..]);
    Some(out)
}

/// The whitespace-delimited tokens of `line`, each with its byte offset.
fn tokens(line: &str) -> impl Iterator<Item = (usize, &str)> {
    line.split_whitespace()
        .map(move |token| (offset_of(line, token), token))
}

/// Where `token` — which must be a slice of `line` — begins in it.
///
/// Pointer arithmetic on the slices rather than a search, because a token can repeat within a line
/// and `find` would return the first one every time.
fn offset_of(line: &str, token: &str) -> usize {
    token.as_ptr() as usize - line.as_ptr() as usize
}

/// Whether two ranges share any byte.
fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// An `@` whose next whitespace-delimited token is a dotted host.
///
/// Within a single token here, so the host is simply what follows the `@`. Takes the **last** `@`,
/// because `mailto:someone@example.com` and a stray leading `@` both appear, and the host is always
/// after the final one.
fn is_email(token: &str) -> bool {
    let Some((before, host)) = token.rsplit_once('@') else {
        return false;
    };
    // Something has to be in front of it, or `@example.com` is a social handle rather than an
    // address — and `meet me @ the corner` must survive, which is the test the boolean rule in
    // `karaoke.rs` already carries.
    if before.chars().filter(|c| c.is_alphanumeric()).count() == 0 {
        return false;
    }
    let host = host.trim_end_matches(|c: char| !c.is_alphanumeric());
    host.contains('.') && host.len() > 3
}

/// `http://`, `https://` or a bare `www.` host.
fn is_web(token: &str) -> bool {
    token.contains("http://") || token.contains("https://") || token.contains("www.")
}

/// A token that is a telephone number rather than a word or a year.
///
/// Seven digits is the floor `is_credit_line` uses and is kept, so `1999` and a `1, 2, 3, 4`
/// count-in are untouched. Separators are allowed between them because a written number carries
/// them; letters are not, because a token with letters in it is a word.
fn is_telephone(token: &str) -> bool {
    let digits = token.chars().filter(char::is_ascii_digit).count();
    if digits < 7 {
        return false;
    }
    token
        .chars()
        .all(|c| c.is_ascii_digit() || "()-. /+*".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every address below is either a reserved documentation domain or invented. The rule this
    // demonstrates is about *shape*, so a documentation address exercises it exactly as a real one
    // would — and a real one would be a stranger's, which no committed file here may carry.

    #[test]
    fn an_address_is_masked_and_the_rest_of_the_line_survives() {
        assert_eq!(
            redact("Sung by someone@example.com tonight").as_deref(),
            Some("Sung by — tonight")
        );
        assert_eq!(
            redact("Visit https://example.test/kar now").as_deref(),
            Some("Visit — now")
        );
        assert_eq!(
            redact("karaoke by www.example.com").as_deref(),
            Some("karaoke by —")
        );
    }

    #[test]
    fn a_clean_line_is_left_entirely_alone() {
        assert_eq!(redact("Descobridor dos sete mares"), None);
        assert_eq!(redact(""), None);
        // The `@` that is not an address, which is why the host is checked rather than the sign.
        assert_eq!(redact("meet me @ the corner"), None);
    }

    #[test]
    fn the_business_card_loses_the_address_and_the_number() {
        // The corpus shape, with the address redacted and the masked area code kept because it is
        // what makes the case recognizable.
        assert_eq!(
            redact("someone@example.com (0**19) 5550123").as_deref(),
            Some("— (0**19) —")
        );
    }

    /// **The reason the telephone rule asks about the line and not only the token.**
    #[test]
    fn a_number_in_a_real_lyric_is_not_a_telephone_number() {
        // Famously. A span rule with no context takes the hook out of the song.
        assert_eq!(redact("867-5309 I got it"), None);
        assert_eq!(redact("1999 was the year"), None);
        assert_eq!(redact("one two three four"), None);
    }

    #[test]
    fn a_number_beside_an_address_is_a_telephone_number() {
        // The same digits as the case above, on a line that has already declared itself.
        assert_eq!(
            redact("call 867-5309 or www.example.com").as_deref(),
            Some("call — or —")
        );
    }

    #[test]
    fn a_line_that_is_mostly_digits_is_contact_shaped_on_its_own() {
        // No address on it at all, and `is_credit_line` would already have set it aside.
        assert_eq!(redact("0**17 3463-1150").as_deref(), Some("0**17 —"));
    }

    #[test]
    fn spans_are_ordered_and_do_not_overlap() {
        let line = "a@example.com and b@example.test";
        let spans = contact_spans(line);
        assert_eq!(spans.len(), 2);
        assert!(spans[0].end <= spans[1].start);
        assert_eq!(&line[spans[0].clone()], "a@example.com");
        assert_eq!(&line[spans[1].clone()], "b@example.test");
    }

    #[test]
    fn a_repeated_token_is_found_at_both_of_its_offsets() {
        // The reason `offset_of` does pointer arithmetic rather than `str::find`, which would
        // return the first occurrence twice and mask the wrong half of the line.
        let line = "www.example.com www.example.com";
        let spans = contact_spans(line);
        assert_eq!(spans, vec![0..15, 16..31]);
        assert_eq!(redact(line).as_deref(), Some("— —"));
    }

    #[test]
    fn an_accented_line_is_sliced_on_character_boundaries() {
        // Spans land on whitespace boundaries, so they are always char boundaries — but the slicing
        // is the part that panics if that is ever wrong, so it is worth a line with multi-byte
        // characters either side of the address.
        assert_eq!(
            redact("coração someone@example.com coração").as_deref(),
            Some("coração — coração")
        );
    }
}
