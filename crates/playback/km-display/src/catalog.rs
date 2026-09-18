//! How much this machine has to sing, in one line.
//!
//! The idle screen already answers *where the remote is* and *type a number here*. It did not answer
//! the question somebody standing in front of it asks first — is there anything in this thing? An
//! empty catalog and a full one looked identical until a number was refused, and a refusal is a
//! poor way to find out that no package was ever installed.
//!
//! Pure formatting, in the shape [`crate::connect::ConnectInfo::panel`] uses: the counts come from
//! the caller and this decides the words, so the sentence is testable without a screen.

/// Song and package counts, as the idle screen states them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatalogSummary {
    /// Songs across every installed package.
    pub songs: usize,
    /// Packages installed.
    pub packages: usize,
}

impl CatalogSummary {
    /// A summary of a catalog with these counts.
    #[must_use]
    pub fn new(songs: usize, packages: usize) -> Self {
        Self { songs, packages }
    }

    /// The line to draw.
    ///
    /// **A machine with nothing in it says so in words**, rather than showing `0 songs · 0 packages`.
    /// Zero is the state that needs explaining, and a row of zeroes reads as a fault in the counter
    /// rather than as an empty catalog.
    #[must_use]
    pub fn line(&self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        if self.songs == 0 {
            return words.msg(crate::words::CATALOG_EMPTY).into_owned();
        }
        // **Each count is passed twice, and the pair is not redundant.** `$songs` is the number, and
        // it is what Fluent's plural selector reads; `$songs_display` is the same number already
        // grouped into threes, which is what gets printed. Passing only the grouped form would make
        // `1,000` a string and lose the selector; passing only the raw one would print `12489`.
        words
            .msg_with(
                crate::words::CATALOG_SUMMARY,
                &[
                    ("songs", (self.songs as i64).into()),
                    ("songs_display", group(self.songs, locale).into()),
                    ("packages", (self.packages as i64).into()),
                    ("packages_display", group(self.packages, locale).into()),
                ],
            )
            .into_owned()
    }
}

/// Digits in threes, so a five-figure catalog can be read at television distance.
///
/// **The separator follows the language, and only now that the words do.** While every word on this
/// screen was English, a comma was right for all of them: `12.489` beside the English `songs` was
/// the worse of the two inconsistencies. Both halves move together — a machine set to Brazilian
/// Portuguese says `12.489 músicas`, and one set to English says `12,489 songs`.
///
/// A table of two rather than a locale-formatting dependency, because this is the only number in
/// the product that is grouped at all: a song number is dialled, a duration is a clock, and a
/// suitability is one digit.
fn group(value: usize, locale: km_locale::Locale) -> String {
    let separator = match locale {
        km_locale::Locale::English => ',',
        km_locale::Locale::BrazilianPortuguese => '.',
    };
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use km_locale::Locale;

    use super::*;

    const EN: Locale = Locale::English;
    const PT: Locale = Locale::BrazilianPortuguese;

    #[test]
    fn a_full_catalog_says_how_much_of_it_there_is() {
        assert_eq!(
            CatalogSummary::new(1234, 5).line(EN),
            "1,234 songs · 5 packages"
        );
    }

    #[test]
    fn an_empty_catalog_is_explained_rather_than_shown_as_zeroes() {
        assert_eq!(CatalogSummary::new(0, 0).line(EN), "No songs installed");
    }

    #[test]
    fn a_package_that_installed_nothing_still_reads_as_empty() {
        // Zero songs is the fact worth stating; how many packages produced none of them is not
        // something to put on a television.
        assert_eq!(CatalogSummary::new(0, 3).line(EN), "No songs installed");
    }

    #[test]
    fn one_of_a_thing_is_singular() {
        assert_eq!(CatalogSummary::new(1, 1).line(EN), "1 song · 1 package");
    }

    #[test]
    fn a_portuguese_machine_counts_in_portuguese() {
        assert_eq!(
            CatalogSummary::new(1234, 5).line(PT),
            "1.234 músicas · 5 pacotes"
        );
        assert_eq!(CatalogSummary::new(1, 1).line(PT), "1 música · 1 pacote");
        assert_eq!(
            CatalogSummary::new(0, 0).line(PT),
            "Nenhuma música instalada"
        );
    }

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group(0, EN), "0");
        assert_eq!(group(7, EN), "7");
        assert_eq!(group(999, EN), "999");
        assert_eq!(group(1000, EN), "1,000");
        assert_eq!(group(12_489, EN), "12,489");
        // A catalog nobody would build, but the formatting has to survive one.
        assert_eq!(group(120_500, EN), "120,500");
        assert_eq!(group(1_234_567, EN), "1,234,567");
    }

    #[test]
    fn the_separator_follows_the_language_the_words_are_in() {
        // The two move together, which is the whole argument: `12.489` beside an English `songs`
        // was the worse inconsistency, and so is `12,489` beside `músicas`.
        assert_eq!(group(12_489, PT), "12.489");
        assert_eq!(group(1_234_567, PT), "1.234.567");
        assert_eq!(group(999, PT), "999");
    }

    #[test]
    fn the_default_is_an_empty_catalog() {
        assert_eq!(CatalogSummary::default().line(EN), "No songs installed");
    }
}
