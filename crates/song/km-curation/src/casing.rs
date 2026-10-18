//! Putting capitals back into a name that has none of its own.
//!
//! A real corpus arrives shouting. A file's title meta event is `CORCOVADO`, its artist is
//! `TOM JOBIM`, and the stem *Title from file name* writes is `CORCOVAD` — all of which are the same
//! defect: the case was thrown away before this tool ever saw the file, and a list of ten thousand
//! capitalized rows is harder to read than the same rows in mixed case.
//!
//! **Case a person typed is evidence of a decision**, so [`recase`] leaves it alone. `d'Angelo`,
//! `McCartney` and `Tom Jobim` are right as they stand and no rule here can tell them from a mistake;
//! a typed name holding both a capital and a small letter is therefore returned untouched. What is
//! rewritten is a name that is entirely one case, which is the shape the defect has in typed text.
//!
//! **Case the file carried is not a decision, so [`recase_from_file`] rewrites it whatever shape it
//! has.** `Garota De Ipanema` and `Tom jobim` are what a sequencer wrote, and they are the same defect
//! as `CORCOVADO`: nobody chose that case for this corpus. The cost is a file name that happened to be
//! right in an unusual case, so `McCartney` from a title meta event comes back `Mccartney`.
//!
//! **An acronym in a shouting name cannot be told from the shouting, and comes back recased.**
//! `AC/DC` in a corpus that also holds `CORCOVADO` is the same nine bits to any rule this size, so it
//! comes out `Ac/Dc`. That is the limit of the mixed-case guard rather than a gap in it: the guard
//! protects a name somebody has already cased, and an all-capital acronym is a name nobody has.
//!
//! **The answer is a first pass and says so on the page.** Neither an acronym nor a roman numeral
//! survives a rule this size, and the song's own edit box is one click away for the rows it gets
//! wrong. The alternative — no button — leaves the whole corpus shouting.

/// Words that stay in small letters when they fall inside a name.
///
/// **One list, English and Portuguese together, and not one list per language.** Almost no song in a
/// real corpus carries a language: the column is set by curation, and this button is reached long
/// before that pass is done — so a list chosen by the language column would be inert on the rows that
/// need it most.
///
/// The cost is the handful of words the two languages share and disagree about. `do` and `no` are
/// Portuguese articles and English verbs, so *I Do Love You* comes back as *I do Love You*. The
/// corpus is majority Brazilian, which is the side to be wrong on: `de`, `do` and `da` fall inside
/// almost every Portuguese title there is, and mid-title English `do` is rare. A row it gets wrong is
/// one edit, and the button promises a first pass.
///
/// Folded before it is looked up, so `À` and `a` are one entry rather than two.
const SMALL_WORDS: &[&str] = &[
    // English articles, conjunctions and the short prepositions.
    "a", "an", "the", "and", "but", "or", "nor", "as", "at", "by", "for", "from", "in", "into",
    "of", "off", "on", "onto", "over", "per", "to", "up", "via", "with",
    // Portuguese articles and their contractions, conjunctions, short prepositions.
    "ao", "aos", "as", "com", "da", "das", "de", "do", "dos", "e", "em", "na", "nas", "no", "nos",
    "num", "numa", "o", "os", "para", "pela", "pelo", "por", "pra", "que", "se", "um", "uma",
];

/// A name a person typed with capitals put back, or `None` for a name to leave exactly as it is.
///
/// `None` covers three cases and they are one rule: there is nothing here to decide. A name that
/// already mixes capitals and small letters has been decided by somebody, a name with no letters in
/// it has nothing to capitalize, and a name the rule would return unchanged is not a write.
pub fn recase(value: &str) -> Option<String> {
    if has_case_of_its_own(value) {
        return None;
    }
    recase_from_file(value)
}

/// A name the file gave with capitals put back, or `None` when the rule would not change it.
///
/// No mixed-case guard: the case in a title meta event is a sequencer's, not a curator's. See the
/// module header.
pub fn recase_from_file(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    let recased = rewrite(value);
    (recased != value).then_some(recased)
}

/// Whether somebody has already cased this name.
///
/// Both halves have to be present. `CORCOVADO` is upper and nothing else, `garota de ipanema` is
/// lower and nothing else, and each is the defect; `Tom Jobim` is both and is an answer.
///
/// **A script with no case at all reaches the rewrite and comes out of it unchanged**, which is what
/// carries the Japanese half of a corpus through: kana and Han are alphabetic and case-mapped to
/// themselves, so the walk below copies them and `recase` sees no difference to write.
fn has_case_of_its_own(value: &str) -> bool {
    value.chars().any(char::is_uppercase) && value.chars().any(char::is_lowercase)
}

/// Capitalizes each word, small words excepted, keeping every separator exactly where it was.
///
/// **A word is a run of alphanumerics and nothing else**, so an apostrophe and a hyphen both end one:
/// `d'angelo` is `d` then `angelo` and comes back `D'Angelo`, which is the spelling wanted. The
/// separators are copied through untouched, so spacing, punctuation and brackets survive a pass.
///
/// **The word that opens a phrase and the word that closes one are always capitalized**, whatever the
/// list says. *The Dark Side of the Moon* keeps its leading `The`, a title ending in a preposition —
/// *What Are You Waiting For* — does not trail off in small letters, and a phrase inside brackets or
/// after a spaced dash begins the same way the whole name does: *Chega de Saudade (Ao Vivo)*, not
/// *(ao Vivo)*. See [`opens_phrase`].
fn rewrite(value: &str) -> String {
    let words = word_bounds(value);
    let last = words.len().saturating_sub(1);
    let mut out = String::with_capacity(value.len());
    let mut cut = 0;
    for (index, &(start, end)) in words.iter().enumerate() {
        let before = &value[cut..start];
        out.push_str(before);
        let word = &value[start..end];
        let after = match words.get(index + 1) {
            Some(&(next, _)) => &value[end..next],
            None => &value[end..],
        };
        let edge = index == 0 || index == last || opens_phrase(before) || closes_phrase(after);
        let small = !edge && SMALL_WORDS.contains(&km_song::text::fold(word).as_str());
        out.push_str(&match small {
            true => word.to_lowercase(),
            false => capitalize(word),
        });
        cut = end;
    }
    out.push_str(&value[cut..]);
    out
}

/// Whether the text in front of a word starts something the word is the beginning of.
///
/// A bracket, a quotation mark, a colon or a semicolon, and a dash with space around it — which is
/// how a corpus writes `ARTIST - TITLE`, and the `TITLE` half wants its capital whatever word it
/// starts with. **A dash without space is inside a word**, so `re-recording` stays one phrase and does
/// not gain a capital in the middle.
///
/// An apostrophe is not on the list and must not be: it ends a word without starting a phrase, which
/// is what leaves `d'Angelo` spelled the way it is rather than becoming `D'angelo`.
fn opens_phrase(before: &str) -> bool {
    before.chars().any(|ch| "([{<\"«:;".contains(ch))
        || (before.chars().any(|ch| "-–—".contains(ch)) && before.chars().any(char::is_whitespace))
}

/// Whether the text behind a word closes something the word is the end of.
fn closes_phrase(after: &str) -> bool {
    after.chars().any(|ch| ")]}>\"»".contains(ch))
}

/// Where each word starts and ends, as byte offsets into `value`.
fn word_bounds(value: &str) -> Vec<(usize, usize)> {
    let mut bounds = Vec::new();
    let mut open: Option<usize> = None;
    for (at, ch) in value.char_indices() {
        match (ch.is_alphanumeric(), open) {
            (true, None) => open = Some(at),
            (false, Some(start)) => {
                bounds.push((start, at));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(start) = open {
        bounds.push((start, value.len()));
    }
    bounds
}

/// One word with a capital on the front and small letters behind it.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first
        .to_uppercase()
        .chain(chars.flat_map(char::to_lowercase))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shouting_name_comes_back_in_mixed_case() {
        assert_eq!(recase("CORCOVADO").as_deref(), Some("Corcovado"));
        assert_eq!(recase("TOM JOBIM").as_deref(), Some("Tom Jobim"));
    }

    #[test]
    fn a_whispering_name_comes_back_the_same_way() {
        assert_eq!(
            recase("garota de ipanema").as_deref(),
            Some("Garota de Ipanema")
        );
    }

    /// The list is what the button is for, and it is read in both languages at once.
    #[test]
    fn the_small_words_stay_small_in_the_middle() {
        assert_eq!(
            recase("THE DARK SIDE OF THE MOON").as_deref(),
            Some("The Dark Side of the Moon")
        );
        assert_eq!(
            recase("COMO UMA ONDA NO MAR").as_deref(),
            Some("Como uma Onda no Mar")
        );
        assert_eq!(
            recase("AQUARELA DO BRASIL").as_deref(),
            Some("Aquarela do Brasil")
        );
    }

    /// Whatever the list says. A title is not left trailing off in small letters.
    #[test]
    fn the_first_and_last_words_are_capitalized_anyway() {
        assert_eq!(
            recase("THE WAY WE WERE").as_deref(),
            Some("The Way We Were")
        );
        assert_eq!(
            recase("WHAT ARE YOU WAITING FOR").as_deref(),
            Some("What Are You Waiting For")
        );
        assert_eq!(
            recase("A DAY IN THE LIFE").as_deref(),
            Some("A Day in the Life")
        );
    }

    /// The rule this one exists to protect: a name somebody has cased is an answer, not a defect.
    #[test]
    fn a_name_that_already_has_capitals_is_left_alone() {
        for name in ["d'Angelo", "McCartney", "Tom Jobim", "Bee Gees", "iPhone"] {
            assert_eq!(recase(name), None, "{name} should be left as it is");
        }
    }

    /// A name the file gave has no decision behind its case, so a mixed one is rewritten too.
    #[test]
    fn a_mixed_name_from_the_file_is_recased() {
        assert_eq!(recase_from_file("Tom jobim").as_deref(), Some("Tom Jobim"));
        assert_eq!(
            recase_from_file("Garota De Ipanema").as_deref(),
            Some("Garota de Ipanema")
        );
        assert_eq!(recase_from_file("Corcovado"), None);
        assert_eq!(recase_from_file("上を向いて歩こう"), None);
        assert_eq!(recase_from_file("  "), None);
    }

    /// The limit of that guard, written down so it is a known answer rather than a surprise.
    ///
    /// An acronym shouting among shouting names is the same nine bits as a word, so it is recased
    /// with them. A person who wants `AC/DC` back types it once, in the edit box the row already has.
    #[test]
    fn an_acronym_with_no_case_of_its_own_is_recased_with_everything_else() {
        assert_eq!(recase("AC/DC").as_deref(), Some("Ac/Dc"));
        assert_eq!(recase("k.d. lang").as_deref(), Some("K.D. Lang"));
    }

    /// An apostrophe and a hyphen end a word, which is what gets this spelling right.
    #[test]
    fn punctuation_is_copied_through_and_still_ends_a_word() {
        assert_eq!(recase("D'ANGELO").as_deref(), Some("D'Angelo"));
        assert_eq!(
            recase("RE-RECORDING OF A SONG").as_deref(),
            Some("Re-Recording of a Song")
        );
    }

    /// A phrase inside brackets or after a spaced dash begins the way the whole name does.
    #[test]
    fn a_phrase_of_its_own_gets_its_own_capital() {
        assert_eq!(
            recase("CHEGA  DE   SAUDADE (AO VIVO)").as_deref(),
            Some("Chega  de   Saudade (Ao Vivo)")
        );
        assert_eq!(
            recase("TOM JOBIM - A GAROTA DE IPANEMA").as_deref(),
            Some("Tom Jobim - A Garota de Ipanema")
        );
        assert_eq!(
            recase("SOMETHING: THE SEQUEL").as_deref(),
            Some("Something: The Sequel")
        );
    }

    /// Accents are part of the word and survive both the fold that looks a word up and the write.
    #[test]
    fn an_accented_name_keeps_its_accents() {
        assert_eq!(recase("ÁGUAS DE MARÇO").as_deref(), Some("Águas de Março"));
        assert_eq!(
            recase("CORAÇÃO VAGABUNDO").as_deref(),
            Some("Coração Vagabundo")
        );
    }

    /// A digit is a word and capitalizing it changes nothing, so the words around it still decide.
    #[test]
    fn a_number_in_a_name_is_carried_through() {
        assert_eq!(recase("99 LUFTBALLONS").as_deref(), Some("99 Luftballons"));
    }

    /// There is nothing to decide, so there is nothing to write.
    #[test]
    fn a_name_with_no_letters_and_a_name_already_right_are_both_left() {
        assert_eq!(recase(""), None);
        assert_eq!(recase("   "), None);
        assert_eq!(recase("---"), None);
        assert_eq!(recase("Corcovado"), None);
    }

    /// A script with no case passes through the rewrite and comes out identical, so nothing is
    /// written — which is what keeps this button off the Japanese half of a corpus.
    #[test]
    fn a_script_with_no_case_is_not_a_write() {
        assert_eq!(recase("上を向いて歩こう"), None);
        assert_eq!(recase("서울"), None);
    }
}
