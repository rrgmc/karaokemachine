//! Folding titles and artist names into something comparable.
//!
//! Several separate parts of this workspace need to decide whether two pieces of text name the same
//! thing, and none of them can ask SQLite: `km-package-builder` groups the files that look like one recording,
//! `km-remote-pages` filters a favorites folder in memory, the song book files a song under a letter, and
//! **both catalogs** — the machine's `library.sqlite` and the offline remote's mirror — store
//! folded sort keys, because SQLite's default collation puts every accented character after `Z` and
//! its `NOCASE` collation is ASCII-only, so neither of them can put `Águas de Março` under `A`.
//!
//! That last one is why this function is the alphabet of the whole product rather than a helper:
//! the machine's screens, the online remote, the offline remote and the printed book all order by
//! what it returns, so a change here moves every list at once.
//!
//! The folding here is deliberately the same shape as `unicode61 remove_diacritics 2`, the tokenizer
//! `km-catalog`'s FTS5 index uses. If these two disagreed, a song found by the search box could
//! silently fail to match the same song filtered in memory, which is the kind of fault that looks
//! like bad data rather than like a bug.
//!
//! **The table is what SQLite does, transcribed — not a Unicode normalization crate, and no longer
//! guesswork.** `unicode61 remove_diacritics 2` is the authority here because the FTS5 index is the
//! one folding this crate cannot change, so [`fold_char`] is generated from it: every character in
//! Latin-1 Supplement, Latin Extended-A and Latin Extended-B was put through the tokenizer and the
//! answers grouped. `fold_and_the_search_index_agree` in `km-catalog` walks the same range and fails
//! if the two ever part company, which is what makes this a checked claim rather than a comment.
//!
//! **A stroke is not an accent, and neither is a ligature.** `remove_diacritics` takes combining
//! marks off; it leaves `ł ø æ đ ß ı` exactly as they are, because they are letters in their own
//! right rather than a decorated `l o a d s i`. So this leaves them alone too. A Polish title
//! therefore files under `Ł` and not under `L` — which is where a Pole would look for it, and, more
//! to the point, is where the search box also puts it.

/// How many times [`fold`]'s output has changed.
///
/// **Stored by every database that keeps folded columns**, so that a change here re-folds each of
/// them exactly once: `km-catalog` puts it in its `meta` table and `km-package-builder` beside its
/// browse keys. A consumer cannot see a source edit, only this number — so bump it whenever
/// [`fold`], [`fold_char`] or [`initial`] would answer differently for any input, and do not bump it
/// for anything else, because each bump costs every catalog in the field one pass over its songs.
///
/// `1` was Latin-1 accents only, which left Czech, Polish, Hungarian and the Baltic languages
/// sorting after `Z`. `2` is the table derived from `unicode61 remove_diacritics 2`.
///
/// The same shape as `km_kmpkg::LANGUAGE_TABLE_REVISION`, and for the same reason.
pub const FOLD_REVISION: u32 = 2;

/// Folds text to a comparison key: lower case, no accents, punctuation as a single space.
///
/// Leading and trailing space is trimmed and runs are collapsed, so `"Águas de  Março!"` and
/// `"aguas de marco"` both come out as `aguas de marco`.
pub fn fold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_space = true;
    for ch in value.chars().flat_map(fold_char) {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_was_space = false;
        } else if !last_was_space {
            out.push(' ');
            last_was_space = true;
        }
    }
    out.trim_end().to_owned()
}

/// The bucket a title belongs in for an A–Z browse strip.
///
/// `'#'` for anything that starts with a digit, and `None` for text with no letter or digit at all —
/// which happens, because a real corpus contains files named `---.kar`.
pub fn initial(value: &str) -> Option<char> {
    let folded = fold(value);
    let first = folded.chars().next()?;
    if first.is_ascii_digit() {
        Some('#')
    } else {
        first.to_uppercase().next()
    }
}

/// Folds one character to its unaccented, lower-case form.
///
/// **Both cases are listed rather than lower-casing first**, because `İ`'s lower case is two
/// characters — an `i` and a combining dot — and the dot is not alphanumeric, so [`fold`] would cut
/// `İstanbul` into two words where the search index has one.
///
/// Generated from `unicode61 remove_diacritics 2` over U+00C0–U+024F; see the module documentation.
fn fold_char(ch: char) -> impl Iterator<Item = char> {
    let folded = match ch {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'Ā' | 'ā' | 'Ă'
        | 'ă' | 'Ą' | 'ą' | 'Ǎ' | 'ǎ' | 'Ǟ' | 'ǟ' | 'Ǻ' | 'ǻ' | 'Ȁ' | 'ȁ' | 'Ȃ' | 'ȃ' | 'Ȧ'
        | 'ȧ' => 'a',
        'Ç' | 'ç' | 'Ć' | 'ć' | 'Ĉ' | 'ĉ' | 'Ċ' | 'ċ' | 'Č' | 'č' => 'c',
        'Ď' | 'ď' => 'd',
        'È' | 'É' | 'Ê' | 'Ë' | 'è' | 'é' | 'ê' | 'ë' | 'Ē' | 'ē' | 'Ĕ' | 'ĕ' | 'Ė' | 'ė' | 'Ę'
        | 'ę' | 'Ě' | 'ě' | 'Ȅ' | 'ȅ' | 'Ȇ' | 'ȇ' | 'Ȩ' | 'ȩ' => 'e',
        'Ĝ' | 'ĝ' | 'Ğ' | 'ğ' | 'Ġ' | 'ġ' | 'Ģ' | 'ģ' | 'Ǧ' | 'ǧ' | 'Ǵ' | 'ǵ' => 'g',
        'Ĥ' | 'ĥ' | 'Ȟ' | 'ȟ' => 'h',
        'Ì' | 'Í' | 'Î' | 'Ï' | 'ì' | 'í' | 'î' | 'ï' | 'Ĩ' | 'ĩ' | 'Ī' | 'ī' | 'Ĭ' | 'ĭ' | 'Į'
        | 'į' | 'İ' | 'Ǐ' | 'ǐ' | 'Ȉ' | 'ȉ' | 'Ȋ' | 'ȋ' => 'i',
        'Ĵ' | 'ĵ' | 'ǰ' => 'j',
        'Ķ' | 'ķ' | 'Ǩ' | 'ǩ' => 'k',
        'Ĺ' | 'ĺ' | 'Ļ' | 'ļ' | 'Ľ' | 'ľ' => 'l',
        'Ñ' | 'ñ' | 'Ń' | 'ń' | 'Ņ' | 'ņ' | 'Ň' | 'ň' | 'Ǹ' | 'ǹ' => 'n',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'Ō' | 'ō' | 'Ŏ' | 'ŏ' | 'Ő'
        | 'ő' | 'Ơ' | 'ơ' | 'Ǒ' | 'ǒ' | 'Ǫ' | 'ǫ' | 'Ǭ' | 'ǭ' | 'Ȍ' | 'ȍ' | 'Ȏ' | 'ȏ' | 'Ȫ'
        | 'ȫ' | 'Ȭ' | 'ȭ' | 'Ȯ' | 'ȯ' | 'Ȱ' | 'ȱ' => 'o',
        'Ŕ' | 'ŕ' | 'Ŗ' | 'ŗ' | 'Ř' | 'ř' | 'Ȑ' | 'ȑ' | 'Ȓ' | 'ȓ' => 'r',
        'Ś' | 'ś' | 'Ŝ' | 'ŝ' | 'Ş' | 'ş' | 'Š' | 'š' | 'ſ' | 'Ș' | 'ș' => 's',
        'Ţ' | 'ţ' | 'Ť' | 'ť' | 'Ț' | 'ț' => 't',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'ù' | 'ú' | 'û' | 'ü' | 'Ũ' | 'ũ' | 'Ū' | 'ū' | 'Ŭ' | 'ŭ' | 'Ů'
        | 'ů' | 'Ű' | 'ű' | 'Ų' | 'ų' | 'Ư' | 'ư' | 'Ǔ' | 'ǔ' | 'Ǖ' | 'ǖ' | 'Ǘ' | 'ǘ' | 'Ǚ'
        | 'ǚ' | 'Ǜ' | 'ǜ' | 'Ȕ' | 'ȕ' | 'Ȗ' | 'ȗ' => 'u',
        'Ŵ' | 'ŵ' => 'w',
        'Ý' | 'ý' | 'ÿ' | 'Ŷ' | 'ŷ' | 'Ÿ' | 'Ȳ' | 'ȳ' => 'y',
        'Ź' | 'ź' | 'Ż' | 'ż' | 'Ž' | 'ž' => 'z',
        other => other,
    };
    folded.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_and_punctuation_fold_away() {
        assert_eq!(fold("Águas de  Março!"), "aguas de marco");
        assert_eq!(fold("AC/DC"), "ac dc");
        assert_eq!(fold("rock 'n' roll"), "rock n roll");
    }

    #[test]
    fn folding_is_what_makes_two_spellings_of_one_song_the_same() {
        assert_eq!(fold("Coração"), fold("CORACAO"));
    }

    /// A corpus really does contain files named like this, and a browse strip that panicked on one
    /// would be a strip that worked until the day somebody scrolled far enough.
    #[test]
    fn text_with_nothing_to_sort_by_has_no_initial() {
        assert_eq!(initial("---"), None);
        assert_eq!(initial(""), None);
    }

    #[test]
    fn digits_share_one_bucket() {
        assert_eq!(initial("99 Luftballons"), Some('#'));
        assert_eq!(initial("Águas"), Some('A'));
    }
}
