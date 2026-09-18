//! Which carols the pack carries.
//!
//! **Selected by `X:` number, with the title asserted.** The reference number is the stable
//! identifier within a pinned edition, and the edition is pinned by checksum in
//! `tools/dist/carols.sh` — but a re-pin to a later edition could renumber, and a pack that
//! silently swapped one carol for another would be worse than a pack that failed to build. So the
//! number selects and the title checks.
//!
//! Every one of these was read individually against its own `C: copyright:` line and is public
//! domain in all four layers the hymnal distinguishes — music, setting, words and translation.
//! `license::assess` re-checks that at every build rather than trusting this comment.
//!
//! **`X:72`, *Twas In The Moon of Wintertime*, is deliberately absent.** Its words and music are
//! public domain and its *setting* is CPDL's, whose default terms are CC BY-SA. It is the reason
//! the gate reads four layers and not one.
//!
//! The artist is the composer of the tune where the hymnal names one and `Traditional …` where it
//! does not, because a karaoke catalog sorts by artist and sixteen songs all filed under one word
//! is an index that indexes nothing. The full attribution — words, music, setting, translation and
//! their dates — is not lost: it goes to `CREDITS.md` verbatim, from the tune's own `C:` lines.

/// One carol, as the pack asks for it.
pub struct Carol {
    /// The hymnal's `X:` reference number.
    pub number: u32,
    /// The title the tune must carry, or the build stops.
    pub title: &'static str,
    /// What the machine files it under.
    pub artist: &'static str,
}

/// The sixteen, in the order they take their song numbers.
pub const CAROLS: [Carol; 16] = [
    Carol {
        number: 39,
        title: "O Come O Come Emmanuel",
        artist: "Traditional French",
    },
    Carol {
        number: 49,
        title: "Angels From the Realms of Glory",
        artist: "Henry Smart",
    },
    Carol {
        number: 50,
        title: "Angels We Have Heard On High",
        artist: "Traditional French",
    },
    Carol {
        number: 51,
        title: "Away In A Manger",
        artist: "James R. Murray",
    },
    Carol {
        number: 55,
        title: "Gentle Mary Laid Her Child",
        artist: "Traditional",
    },
    Carol {
        number: 57,
        title: "Hark! The Herald Angels Sing",
        artist: "Felix Mendelssohn",
    },
    Carol {
        number: 58,
        title: "I Heard The Bells On Christmas Day",
        artist: "John B. Calkin",
    },
    Carol {
        number: 59,
        title: "In The Bleak MidWinter",
        artist: "Gustav Holst",
    },
    Carol {
        number: 60,
        title: "It Came Upon A Midnight Clear",
        artist: "Richard S. Willis",
    },
    Carol {
        number: 61,
        title: "Joy to the World",
        artist: "George F. Handel",
    },
    Carol {
        number: 65,
        title: "O Come, All Ye Faithful",
        artist: "John F. Wade",
    },
    Carol {
        number: 66,
        title: "O Little Town of Bethlehem",
        artist: "Lewis H. Redner",
    },
    Carol {
        number: 67,
        title: "See Amid the Winter's Snow",
        artist: "John Goss",
    },
    Carol {
        number: 68,
        title: "Silent Night",
        artist: "Franz Xaver Gruber",
    },
    Carol {
        number: 70,
        title: "The First Noel",
        artist: "Traditional English",
    },
    Carol {
        number: 73,
        title: "What Child Is This?",
        artist: "Traditional English",
    },
];

/// The package's own identity.
///
/// `PackageMeta::suggested_bank` hashes this id into a bank, so the carols land in the same thousand
/// on every machine and next year, without squatting on a low number somebody may want for their own
/// volumes.
///
/// **Written out rather than generated, and it is still a generated id.** Every other package gets
/// one from `PackageMeta::new_id` at the moment somebody starts it; this one is published, so the
/// value has to be the same in every copy ever built — a fresh one per build would put the carols in
/// a different thousand each time and reinstall as a second package rather than replacing the first.
/// The shape is what `PackageMeta::is_generated_id` requires, so the id says nothing about where the
/// package was made, which is the rule it is written out to keep rather than to escape.
pub const PACKAGE_ID: &str = "c5a201f8e6b4d379";

/// What the machine and the printed book call it.
pub const PACKAGE_NAME: &str = "Christmas Carols";

/// Who made the package, as distinct from who wrote the songs.
pub const PACKAGE_PUBLISHER: &str = "Open Hymnal Project (public domain)";

/// All sixteen are English. Written per song rather than as a package default, so each `.kar` says
/// so itself and a song lifted out of the package still classifies.
pub const LANGUAGE: &str = "en";

/// The same, in the four-letter form Soft Karaoke's `@L` line uses in real files.
pub const LANGUAGE_TAG: &str = "ENGL";

/// The voice that carries the tune in every hymnal setting: the first, the soprano.
pub const MELODY_VOICE: &str = "S1V1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selection_is_the_sixteen_and_they_are_distinct() {
        assert_eq!(CAROLS.len(), 16);
        let mut numbers: Vec<u32> = CAROLS.iter().map(|c| c.number).collect();
        numbers.sort_unstable();
        numbers.dedup();
        assert_eq!(numbers.len(), 16);
    }

    /// The carol whose setting is CPDL's. If a later edition renumbers and this ever becomes one of
    /// ours by accident, `license::assess` still refuses it -- but it should never be asked for.
    #[test]
    fn the_cpdl_carol_is_not_asked_for() {
        assert!(!CAROLS.iter().any(|c| c.number == 72));
    }

    #[test]
    fn every_carol_is_filed_under_something() {
        assert!(
            CAROLS
                .iter()
                .all(|c| !c.title.is_empty() && !c.artist.is_empty())
        );
    }
}
