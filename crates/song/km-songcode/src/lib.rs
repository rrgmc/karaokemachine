//! How a singer names a song: one number, carrying the package it came from inside it.
//!
//! A song number is **a bank and a slot**, `bank * 1000 + slot`. The slot is the song's number
//! inside its package, 1 to 999; the bank is 1 to 9999 and says which package's block of a thousand
//! the song sits in. `1001` is the first song of bank 1, and `3500` is bank 3's five hundredth song.
//!
//! **Bank 0 holds no package**, so nothing a singer dials in three digits is a song. That block is
//! the machine's own — see `Bank 0 is the machine's own` — and [`SongCode::in_bank`] refuses it.
//!
//! **The bank belongs to the package and the machine assigns it**, so two packages that both number
//! their songs from 1 land in different thousands and cannot collide. That is what this replaces: a
//! 1–3 letter package *prefix*, which said the same thing with letters that a D-pad under a
//! television does not have. Splitting the number instead costs nothing at the keypad — every code
//! is still digits — and it is what a commercial machine's printed book already does with the volume
//! a song sits on.
//!
//! **A crate of its own, with two dependencies and nothing else.** The type has to be visible to the
//! catalog, the queue, the API, the display and both remotes, and there is no existing crate all
//! of those can see: `km-audio` depends only on `km-song`, and `km-display` deliberately avoids
//! `km-kmpkg` (see `SongInfo::language`). Putting it in `km-song` would have made the MIDI parser
//! the home of a catalog concept and dragged `midly` into `km-catalog`.
//!
//! **One representation on the wire: a string.** A code is `"500"`, never an integer, and nothing
//! accepts more than one spelling. The reason is not ambiguity — a bare integer names exactly one
//! song — but that there is one spelling at all: every route, element id, template and script spells
//! it this way, and a second accepted shape is the thing this rule exists to prevent.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Highest number a song may carry.
///
/// **The keypad is where this comes from, and that is still the whole argument.** A number exists to
/// be dialled, and the machine's number entry takes [`MAX_DIGITS`] digits; a song numbered above
/// this would sit in the catalog, answer to search, and be unaskable at the one surface the number
/// is for. So the limit is not a storage bound — a `u32` holds four hundred times as much, and
/// SQLite would take eight times that again — it is the size of what a singer can type.
///
/// **It was six digits and is now seven, and that is a cost rather than a free change.** The seventh
/// digit bought [`MAX_BANK`] a factor of ten, which is what keeps two independently built packages
/// out of the same thousand — see `A package's bank comes from its id`. What it spends is the
/// television: a song has been dialled on a Google TV Streamer with the remote alone at several
/// D-pad presses a digit, and the idle number pad was switched back on for every Android precisely
/// so a television is self-sufficient without a phone. Seven digits makes that a third more work.
/// The judgment is that the remote app is how a number is normally entered and the keypad is the
/// fallback that must keep working, not the surface being optimized for.
///
/// **It is the bound on a whole code, and [`MAX_SLOT`] is the bound on a song's number inside its
/// package.** Those are two different limits at two different places, and conflating them is how a
/// package would come to hold a song that dials into the next package's bank.
pub const MAX_NUMBER: u32 = 9_999_999;

/// Digits in [`MAX_NUMBER`], and so what a number-entry surface accepts.
pub const MAX_DIGITS: usize = 7;

/// How many numbers a bank holds, and so the multiplier between a bank and its first song.
pub const BANK_SPAN: u32 = 1_000;

/// Highest slot a song may take inside its package — and so **how many songs a package may hold**.
///
/// This is a statement about curation before it is one about arithmetic. Packages here are put
/// together by hand, and a cap is what discourages pointing the builder at a corpus and importing
/// all of it; that it also makes a bank exactly a thousand wide is what makes it cheap.
///
/// A slot above this does not merely fail to dial: it lands inside the **next** package's bank, so
/// such a package names songs that are not its own wherever it is read. That is why it is enforced
/// where a slot is *assigned* — `Manifest::problems`, both packaging tools — rather than only where
/// a code is typed.
pub const MAX_SLOT: u16 = 999;

/// Highest bank the machine may assign.
///
/// Ten thousand banks of 999 songs is 9,999,000 songs, which is far more than anybody will install
/// and exactly what seven digits reach: [`MAX_BANK`] and [`MAX_SLOT`] together are [`MAX_NUMBER`],
/// and a test says so rather than leaving the three constants to agree by hand.
///
/// **The size is chosen against collisions, not against how many packages fit.** A package that
/// names no bank is banked at `1 + SHA-256(id) % MAX_BANK`, so what this number governs is how often
/// two independently built packages want the same thousand — a birthday problem over `MAX_BANK`,
/// counted over the packages installed *together* rather than the packages in the world. A thousand
/// banks put that at 17% for twenty packages and 35% for thirty; ten thousand put it at 1.9% and
/// 4.3%. Nothing fails when it happens — `choose_bank` takes the next free bank — but the loser's
/// already-printed book is then wrong, which is the one thing the derivation exists to prevent.
///
/// **9999 and not more, because 99999 would not fit a `u16`.** The next step up costs an eighth
/// digit and widens every bank in the workspace to `u32` for a gain that only tells past about fifty
/// packages on one machine. See the note on [`MAX_NUMBER`] for what the seventh digit already spends.
pub const MAX_BANK: u16 = 9_999;

/// Why a song code could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodeError {
    /// It was empty, or something that is not a digit was in it.
    #[error("{0:?} is not a song number")]
    NotANumber(String),
    /// Zero is not a song number, and never has been — see `Manifest::problems`.
    #[error("0 is not a song number")]
    ZeroNumber,
    /// The number is above [`MAX_NUMBER`].
    ///
    /// Carries the text as typed rather than a `u32`, because the whole point is that the value may
    /// not fit one: `99999999999` and `10000000` are the same mistake and are refused the same way.
    #[error("{0:?} is above the highest song number, {MAX_NUMBER}")]
    NumberTooLarge(String),
}

/// How a singer names a song: a bank and a slot, run together as one number.
///
/// Ordered by the number itself, which is the same thing as ordering by bank and then by slot — so
/// each package's block is contiguous and the catalog's paging is one seek. That equivalence used
/// to need proving, when the first half was three letters; now it is what `u32` already does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SongCode(u32);

impl SongCode {
    /// A code from a number that is already known to be one.
    ///
    /// `const`, so a fixture can write `SongCode::new(1001)`. It does **not** check the range:
    /// everything reading a number from outside goes through [`FromStr`] or [`SongCode::in_bank`],
    /// and making this fallible would put a `Result` in a hundred literals for no reader's benefit.
    pub const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The whole number, as it is dialled and as it is stored.
    pub const fn number(&self) -> u32 {
        self.0
    }

    /// Which package's block of a thousand this song is in.
    pub const fn bank(&self) -> u16 {
        (self.0 / BANK_SPAN) as u16
    }

    /// The song's own number inside its package.
    ///
    /// Zero for a number like `3000`, which is a code nobody's package holds. That is deliberately
    /// not a parse error — see [`FromStr`].
    pub const fn slot(&self) -> u16 {
        (self.0 % BANK_SPAN) as u16
    }

    /// The code a package's song takes once the machine has banked it.
    ///
    /// The one place the two halves are put together, so the multiplication is written once. `None`
    /// for a bank or a slot outside its range, which is what stops a bad slot silently addressing
    /// the neighboring package.
    ///
    /// **Bank 0 is refused here exactly as slot 0 is**, because it is the machine's own — see
    /// `Bank 0 is the machine's own`. Refusing it in the constructor is what makes the reservation
    /// a property of the type rather than a check each surface remembers to make: no package's song
    /// can be given a three-digit number by any road.
    pub const fn in_bank(bank: u16, slot: u16) -> Option<Self> {
        if bank == 0 || bank > MAX_BANK || slot == 0 || slot > MAX_SLOT {
            return None;
        }
        Some(Self(bank as u32 * BANK_SPAN + slot as u32))
    }
}

impl fmt::Display for SongCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl FromStr for SongCode {
    type Err = CodeError;

    /// Reads `500`, and `007` too.
    ///
    /// Leading zeros are accepted and dropped — a keypad has always allowed `007`.
    ///
    /// **A number whose slot is zero is a code**, and `3000` parses as readily as `3001`. Nobody's
    /// package holds it, so it misses and the machine says `no song 3000` — which is what every
    /// other unheld number does. Refusing it here would make the keypad reject a number rather than
    /// answer it, and `ZeroNumber` exists only because 0 is what "nothing typed" looks like.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let trimmed = text.trim();
        if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(CodeError::NotANumber(trimmed.to_owned()));
        }
        // Overflowing a `u32` and merely being above `MAX_NUMBER` are the same mistake — a number
        // nobody can dial — so they are refused as one error rather than two. Parsing into `u64`
        // first is what lets the message quote the value; `trimmed` is already known to be all ASCII
        // digits, so the only failure left is length.
        let number = trimmed
            .parse::<u64>()
            .ok()
            .filter(|number| *number <= u64::from(MAX_NUMBER))
            .ok_or_else(|| CodeError::NumberTooLarge(trimmed.to_owned()))?;
        if number == 0 {
            return Err(CodeError::ZeroNumber);
        }
        // Safe by the bound just checked.
        Ok(Self(number as u32))
    }
}

impl Serialize for SongCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SongCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(text: &str) -> SongCode {
        text.parse().expect("a code")
    }

    #[test]
    fn a_number_round_trips() {
        let parsed = code("10234");
        assert_eq!(parsed.number(), 10_234);
        assert_eq!(parsed.to_string(), "10234");
    }

    #[test]
    fn leading_zeros_are_dropped_as_a_keypad_has_always_allowed() {
        assert_eq!(code("007"), SongCode::new(7));
    }

    #[test]
    fn what_is_not_a_code() {
        assert!(matches!(
            "".parse::<SongCode>(),
            Err(CodeError::NotANumber(_))
        ));
        // Letters are not a code any more, and that is the change: `BR500` is a typo now.
        assert!(matches!(
            "BR500".parse::<SongCode>(),
            Err(CodeError::NotANumber(_))
        ));
        // A letter among the digits is a typo too, and taking the `5` out of it would queue a song
        // nobody asked for.
        assert!(matches!(
            "5A0".parse::<SongCode>(),
            Err(CodeError::NotANumber(_))
        ));
        assert_eq!("0".parse::<SongCode>(), Err(CodeError::ZeroNumber));
        // A number too large for the catalog is a typo, not a song — and one digit past the limit
        // is refused exactly as eleven digits are, rather than as a different kind of mistake.
        assert!(matches!(
            "99999999999".parse::<SongCode>(),
            Err(CodeError::NumberTooLarge(_))
        ));
        assert!(matches!(
            "10000000".parse::<SongCode>(),
            Err(CodeError::NumberTooLarge(_))
        ));
    }

    #[test]
    fn a_number_with_no_slot_is_a_code_that_simply_misses() {
        // 3000 is bank 3's slot 0, which no package holds. It parses, so the machine answers
        // `no song 3000` rather than refusing the keys as they are pressed.
        let parsed = code("3000");
        assert_eq!(parsed.bank(), 3);
        assert_eq!(parsed.slot(), 0);
        // ...and it is not a code any package could have produced.
        assert_eq!(SongCode::in_bank(3, 0), None);
    }

    #[test]
    fn the_highest_number_a_keypad_can_dial_is_a_code() {
        // Seven digits is what the machine's number entry takes, so the last number it can produce
        // has to be a number the parser accepts. The pair of assertions is the limit itself.
        assert_eq!(MAX_NUMBER.to_string().len(), MAX_DIGITS);
        assert_eq!("9999999".parse::<SongCode>(), Ok(SongCode::new(MAX_NUMBER)));
        // Leading zeros do not count against the limit: they are dropped, as a keypad has always
        // allowed, so this is seven significant digits and not eight characters' worth.
        assert_eq!(
            "09999999".parse::<SongCode>(),
            Ok(SongCode::new(MAX_NUMBER))
        );
    }

    #[test]
    fn the_last_bank_and_the_last_slot_are_the_highest_number() {
        // The three constants are one statement written three ways, and this is what keeps them
        // from drifting: widening a bank without widening the ceiling would make the top bank
        // undialable, and narrowing the ceiling would strand it in the catalog.
        assert_eq!(
            u32::from(MAX_BANK) * BANK_SPAN + u32::from(MAX_SLOT),
            MAX_NUMBER
        );
        assert_eq!(
            SongCode::in_bank(MAX_BANK, MAX_SLOT),
            Some(SongCode::new(MAX_NUMBER))
        );
    }

    #[test]
    fn a_code_splits_into_the_bank_and_the_slot_it_was_made_of() {
        for (bank, slot) in [(1, 1), (1, 999), (7, 500), (999, 999), (9999, 999)] {
            let code = SongCode::in_bank(bank, slot).expect("a code");
            assert_eq!(code.bank(), bank, "bank of {code}");
            assert_eq!(code.slot(), slot, "slot of {code}");
        }
        assert_eq!(
            SongCode::in_bank(1, 1).expect("a code").to_string(),
            "1001",
            "a bank starts a thousand up and its first song is slot 1, not slot 0"
        );
    }

    #[test]
    fn a_slot_outside_its_package_is_refused_rather_than_reaching_the_next_bank() {
        // The whole reason the cap is enforced rather than assumed: slot 1000 of bank 1 would be
        // 2000, which is bank 2's, so a package carrying one would name a song that is not its own.
        assert_eq!(SongCode::in_bank(1, MAX_SLOT + 1), None);
        assert_eq!(SongCode::in_bank(1, 0), None, "slot 0 is not a song");
        assert_eq!(SongCode::in_bank(MAX_BANK + 1, 1), None);
    }

    #[test]
    fn no_song_can_be_put_in_bank_zero() {
        // The reservation, at the one place a code is built from its halves: bank 0 is the
        // machine's, so a three-digit number is not a song however it is asked for.
        assert_eq!(SongCode::in_bank(0, 1), None);
        assert_eq!(SongCode::in_bank(0, 500), None);
        assert_eq!(SongCode::in_bank(0, MAX_SLOT), None);
        // It still *parses*, and that half must not follow: the keypad answers `no song 500`
        // rather than refusing the keys, and what a number below 1000 comes to mean is the
        // machine's own business.
        assert_eq!("500".parse::<SongCode>(), Ok(SongCode::new(500)));
    }

    #[test]
    fn codes_sort_by_number_and_so_by_bank_then_slot() {
        let mut codes = [code("2"), code("1010"), code("10"), code("1002"), code("1")];
        codes.sort();
        let sorted: Vec<String> = codes.iter().map(SongCode::to_string).collect();
        // Each bank's block is contiguous and inside one the order is numeric — 1010 after 1002,
        // and 1002 after 10, which a string ordering would get wrong.
        assert_eq!(sorted, ["1", "2", "10", "1002", "1010"]);
    }

    #[test]
    fn a_code_is_a_string_on_the_wire_and_nothing_else() {
        let json = serde_json::to_string(&code("1500")).expect("json");
        assert_eq!(json, "\"1500\"");
        assert_eq!(
            serde_json::from_str::<SongCode>("\"1500\"").expect("code"),
            code("1500")
        );
        // And an integer is refused rather than quietly accepted: one spelling, always.
        assert!(serde_json::from_str::<SongCode>("1500").is_err());
    }
}
