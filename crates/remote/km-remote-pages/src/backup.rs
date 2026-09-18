//! The whole favorites collection — every folder and the songs filed in it — as one file.
//!
//! # Why this is not [`crate::share`]
//!
//! That module exists to fit one folder through a QR code, and every decision in it follows from
//! that: decimal digits, a width field so codes pack tightly, a mod-97 check because a camera can
//! read three quarters of a code. None of it applies to a file, and a file has the one thing a code
//! cannot have — room. So this is a separate format for a separate job, and **the two meet only at
//! [`crate::machine::Favorites::add_songs`]**.
//!
//! Sharing moves one folder between two phones standing next to each other. A backup moves
//! everything, to somewhere it can sit for a year. Neither subsumes the other: a QR code cannot
//! carry a database of any size — one folder outgrows it somewhere past a thousand songs — and a
//! file is no use for handing a folder to the person next to you.
//!
//! # Why a file at all, when the platform has backups
//!
//! `Where a phone keeps its favorites` turns Android's Auto Backup off wholesale on two grounds: a
//! 25 MB per-app quota a real catalog mirror is well past, and that it copies files as they lie
//! while both databases are open WAL, so an uncheckpointed one restores inconsistently. **This
//! answers both by how it is taken rather than merely by existing.** It reads rows out through SQL
//! on the connection that wrote them, so it sees committed data by construction and there is no
//! `-wal` sibling to leave behind; and it carries the collection and not the mirror, so an
//! eleven-hundred-song file is under a hundred kilobytes and the quota is beside the point.
//!
//! # Strict about identity, forgiving about shape
//!
//! A folder with no name, a song whose code is missing or mistyped, the same folder written twice:
//! all of these are recoverable, and recovering them is better than refusing a file somebody may
//! have edited by hand. What is not recoverable is a file that is not ours, one that is damaged, or
//! one holding nothing — those refuse, for the reason [`crate::share::decode`] refuses a truncated
//! code. Half a restore is worse than none.
//!
//! **[`KIND`] is what carries identity, and it is here because JSON has no root element name.** The
//! sibling project writes XML and gets this free: a `<favorites>` tag is what turns away a
//! photograph or a spreadsheet handed to a file picker. Without a discriminator there would be
//! nothing to tell a favorites document from any other JSON carrying a `folders` array.
//!
//! The [`FORMAT`] number runs the other way — **compared and reported, never enforced** — per
//! `Backing up what a person typed, and nothing else`: nobody types into a backup, so there is no
//! typo to catch, and the expensive failure is a file refused at the moment somebody is trying to
//! get their collection back.

use std::time::{SystemTime, UNIX_EPOCH};

use km_songcode::SongCode;
use serde::{Deserialize, Serialize};

use crate::machine::SongRef;

/// The document shape this build writes.
///
/// A file declaring more than this is **read anyway** and the difference reported — see the module
/// header, and [`is_from_the_future`].
///
/// `2` since [`SongDoc::package_id`] and [`SongDoc::content_hash`] arrived. **A format 1 file still
/// restores exactly as it always did**: the two fields default to absent, and a song carrying
/// neither falls to the code rung, which is the only rung format 1 ever had. The number moved
/// because a reader deserves to be told which of the two it is holding, not because anything
/// refuses the older one.
pub const FORMAT: u32 = 2;

/// What a favorites file says it is.
///
/// The one field a restore checks, and the reason is in the module header: JSON has no root element
/// name, so identity has to be a field or it is nothing.
pub const KIND: &str = "km-remote-favorites";

/// A whole collection, as one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// The sentence this file opens with, since JSON has no comments.
    ///
    /// Written out and **ignored on the way in**. It does the job `km_pack::Spec::HEADER` does in a
    /// description: somebody opening this a year from now should not have to guess what it is or
    /// what restoring it will do.
    #[serde(default = "note")]
    pub note: String,
    /// What kind of document this is. See [`KIND`].
    #[serde(default)]
    pub kind: String,
    /// The document shape. See [`FORMAT`].
    #[serde(default = "first_format")]
    pub format: u32,
    /// When it was written, RFC 3339 in UTC.
    #[serde(default)]
    pub written_at: String,
    /// The folders, each with what is filed in it.
    ///
    /// **A folder with nothing in it is left out**, and out of the count the page shows before
    /// taking a backup: there is nothing in an empty folder to carry, and a page promising four
    /// folders that writes three is worse than one that says three.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<FolderDoc>,
}

/// One folder and what is in it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FolderDoc {
    /// What it is called.
    ///
    /// Matched case-insensitively on the way in, to agree with `folder.name`'s
    /// `COLLATE NOCASE UNIQUE`: `Party` and `party` are one folder there, and a file naming both
    /// would otherwise restore into a folder that then refused to exist.
    pub name: String,
    /// The songs filed in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub songs: Vec<SongDoc>,
}

/// One membership.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongDoc {
    /// The song's code, as text: the rejoin key of last resort, and the only one a format 1 file
    /// has.
    ///
    /// **Not the only field a restore reads**, and the two below say why: a code is only as stable as
    /// the bank inside it, and a bank belongs to the machine rather than to the package. This is
    /// what the document is keyed and de-duplicated by, and what a song with no recorded identity
    /// restores under.
    ///
    /// **A `String` and not a [`SongCode`], held wide on purpose** — the same rule the package
    /// builder's backup states by holding an `i64` where its database has a `u8`. `SongCode`'s
    /// `FromStr` refuses `"0"`, `"abc"` and anything above `km_songcode::MAX_NUMBER`, and serde has
    /// no way to refuse one row: a single mistyped code would fail the whole parse and take every
    /// other song in the document with it. Held as text, one impossible value is one line in a
    /// report.
    pub code: String,
    /// What the song was called when the file was written.
    ///
    /// **Written from the catalog at export and ignored at import.** A backup that says nothing
    /// about what is in it cannot be checked, cannot be diffed, and cannot be salvaged by hand when
    /// something else has already gone wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Who performed it. The same treatment as the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    /// The package the song came from.
    ///
    /// **Read on the way back, unlike the two above, and that is the whole difference between them.**
    /// A title is written so a person can read the file; this is written so the file can be
    /// restored onto a machine that numbers its catalog differently. A code carries a bank, a bank
    /// is assigned by the machine rather than the package, and two machines can bank the same
    /// package differently — so a code alone can restore onto the *wrong song*, which is the one
    /// outcome worse than restoring nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    /// What the package said the song's content hashes to.
    ///
    /// With [`Self::package_id`] this is a candidate key; on its own it still names a recording.
    /// See `Songs::resolve` for the order the two are tried in and why a missing one falls through
    /// to the code rather than failing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

/// Why a file could not be read.
///
/// A variant rather than a sentence, for [`crate::share::ShareError`]'s reason: the page that prints
/// this is drawn in the viewer's language and a parser has no viewer to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BackupError {
    /// Well-formed JSON, and not a favorites file. See [`KIND`].
    #[error("this does not look like a favorites file")]
    NotOurs,
    /// Not JSON at all, or truncated partway.
    #[error("that file could not be read")]
    Damaged,
    /// It parsed and holds nothing to restore.
    #[error("there are no songs in this file")]
    Empty,
    /// Larger than a favorites file is ever going to be.
    #[error("that file is too large to be a favorites backup")]
    TooLarge,
    /// The picker came back with nothing.
    #[error("no file was chosen")]
    NoFile,
}

impl BackupError {
    /// The message id a page words this with.
    #[must_use]
    pub fn message_key(self) -> &'static str {
        match self {
            Self::NotOurs => "backup-error-not-ours",
            Self::Damaged => "backup-error-damaged",
            Self::Empty => "backup-error-empty",
            Self::TooLarge => "backup-error-too-large",
            Self::NoFile => "backup-error-no-file",
        }
    }

    /// Every variant, for the test that checks each reaches a message that exists.
    pub const ALL: &'static [Self] = &[
        Self::NotOurs,
        Self::Damaged,
        Self::Empty,
        Self::TooLarge,
        Self::NoFile,
    ];
}

/// One folder as a restore will file it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restorable {
    /// The name, trimmed.
    pub name: String,
    /// The songs, de-duplicated within this folder and **not** across folders — a song filed in two
    /// folders belongs in both.
    ///
    /// Carrying the identity the file held, not only the code: a document written on another
    /// machine names numbers that mean other songs here, and the package and hash are what turn
    /// them back into the right ones. De-duplicated **on the code**, which is what the file is
    /// keyed by — two rows for one code are one membership however they are labelled.
    pub songs: Vec<SongRef>,
    /// Codes in the file this build could not read at all.
    ///
    /// Counted rather than silently dropped, because a file somebody edited by hand is exactly the
    /// case this format is forgiving for, and a count is how they find out they fumbled one.
    pub unreadable: usize,
}

impl Document {
    /// The document a backup taken now would be.
    pub fn of(folders: Vec<FolderDoc>) -> Self {
        Self {
            note: note(),
            kind: KIND.to_owned(),
            format: FORMAT,
            written_at: written_at(SystemTime::now()),
            folders,
        }
    }

    /// How many memberships it holds, which is what a restore will read.
    #[must_use]
    pub fn songs(&self) -> usize {
        self.folders.iter().map(|folder| folder.songs.len()).sum()
    }

    /// The document as a file's bytes, pretty-printed because a person opens this.
    pub fn to_json(&self) -> Result<String, BackupError> {
        // Indented costs a few bytes a song against a file that can be read, edited and diffed. A
        // trailing newline, because a text file wants one and `serde_json` writes none.
        let mut text = serde_json::to_string_pretty(self).map_err(|_| BackupError::Damaged)?;
        text.push('\n');
        Ok(text)
    }

    /// `km-favorites-2026-09-07.json`.
    ///
    /// Dated, because a backup is a thing you take more than once and two files of one name in a
    /// downloads folder is a thing you take once. **ASCII throughout**, so the
    /// `Content-Disposition` needs no RFC 5987 encoding — worth knowing, because the folder names
    /// *inside* the file are not ASCII and the next reader will wonder.
    ///
    /// The date is sliced off [`Document::written_at`] rather than computed again, so the calendar
    /// arithmetic below appears once in this file.
    #[must_use]
    pub fn filename(&self) -> String {
        let day = self.written_at.get(..10).unwrap_or("undated");
        format!("km-favorites-{day}.json")
    }
}

/// Reads a document, cleaning up what it can and refusing what it cannot.
///
/// Folders arrive in the order the file lists them, with duplicates merged into the first mention.
pub fn read(text: &str) -> Result<(Document, Vec<Restorable>), BackupError> {
    let document: Document = serde_json::from_str(text).map_err(|_| BackupError::Damaged)?;
    // Identity before anything else: a well-formed JSON document that is not ours is turned away
    // here, which is the job an XML root element does for the sibling project.
    if document.kind != KIND {
        return Err(BackupError::NotOurs);
    }

    let mut out: Vec<Restorable> = Vec::new();
    for folder in &document.folders {
        let name = folder.name.trim();
        if name.is_empty() {
            continue; // nothing to file it under, and `ensure_folder` would refuse the name anyway
        }
        let at = match out
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(name))
        {
            Some(at) => at,
            None => {
                out.push(Restorable {
                    name: name.to_owned(),
                    songs: Vec::new(),
                    unreadable: 0,
                });
                out.len() - 1
            }
        };
        for song in &folder.songs {
            // A code this build cannot parse is one line in a report, not a failed file — which is
            // the whole reason `SongDoc::code` is a `String`.
            let Ok(code) = song.code.trim().parse::<SongCode>() else {
                out[at].unreadable += 1;
                continue;
            };
            // De-duplicated on the code, which is what the document is keyed by. Two rows naming one
            // code are one membership even if they disagree about the rest, and the first wins —
            // the same rule `add_songs` applies to the same file read twice.
            if !out[at].songs.iter().any(|held| held.code == code) {
                out[at].songs.push(SongRef {
                    code,
                    // Blank is not empty: a hand-edited file, or one written before these existed,
                    // says nothing here and restores by code exactly as it always did.
                    package_id: trimmed(song.package_id.as_deref()),
                    content_hash: trimmed(song.content_hash.as_deref()),
                });
            }
        }
    }

    if out.iter().all(|folder| folder.songs.is_empty()) {
        return Err(BackupError::Empty);
    }
    Ok((document, out))
}

/// Whether this file was written by a build that knows more than this one.
///
/// **Not a refusal** — it is a line the report carries, so that a restore which quietly understood
/// less than the file said says so.
#[must_use]
pub fn is_from_the_future(document: &Document) -> bool {
    document.format > FORMAT
}

/// The sentence every backup opens with.
fn note() -> String {
    "Favorites from the KaraokeMachine remote. Each song carries the pack it came from and a hash \
     of its contents, so this restores onto another phone or another machine even if that one \
     numbers its song list differently; the number a singer dials is used only when those are \
     missing. The title is here so the file can be read, and is ignored when it is restored. \
     Restoring only ever adds: folders that are missing are created, songs already filed stay as \
     they are, and nothing is ever removed."
        .to_owned()
}

/// What a file with no `format` key is assumed to be.
fn first_format() -> u32 {
    1
}

/// A field trimmed, with blank read as absent.
///
/// The forgiving half of `Strict about identity, forgiving about shape`, applied to the two
/// identifying fields: `""` in a hand-edited file means the same as leaving the key out, and reading
/// it as a hash nothing matches would strand the row on a rung it should have fallen straight past.
fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// A moment as RFC 3339 in UTC.
///
/// **Hand-rolled, and that is the consistent move rather than a shortcut.** This workspace has no
/// date library and five crates already do this — `karaokemachine`'s `machine.rs`, `km-logfile`,
/// `km-wallpaper-pack`, `km-pack` and `km-package-builder`'s `scan.rs` — each privately. Reaching
/// for a sixth dependency for one timestamp would be a poor trade, and hoisting one of the five into
/// a shared crate is a change worth making on its own rather than smuggling in behind a feature.
/// It appears once here: [`Document::filename`] slices this rather than computing a date again.
fn written_at(at: SystemTime) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rest / 3_600, (rest / 60) % 60, rest % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The calendar date `days` after 1970-01-01, in the proleptic Gregorian calendar.
///
/// Howard Hinnant's `civil_from_days`, which is what every date library does inside. It shifts the
/// year to start in March so the leap day is the last day of it, which removes every special case:
/// the four-, hundred- and four-hundred-year rules all fall out of the era arithmetic.
///
/// Unsigned throughout, because [`written_at`] never passes a date before the epoch — that would
/// mean a clock set to the 1960s, and it answers `1970-01-01` rather than carrying signed
/// arithmetic through here for a case that cannot arise.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // 719_468 is 1970-01-01 counted from 0000-03-01, the start of the first era.
    let z = days + 719_468;
    let era = z / 146_097; // 146_097 days is 400 years exactly.
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The codes a folder restored to, which is what most of these tests are about.
    ///
    /// A `Restorable` carries a whole `SongRef` per row now; the tests that predate that are still
    /// asking the same question and should still read as if they were.
    fn codes(folder: &Restorable) -> Vec<SongCode> {
        folder.songs.iter().map(|song| song.code).collect()
    }

    fn song(code: u32, title: &str) -> SongDoc {
        SongDoc {
            code: code.to_string(),
            title: Some(title.to_owned()),
            artist: Some("Dire Straits".to_owned()),
            package_id: None,
            content_hash: None,
        }
    }

    fn document() -> Document {
        Document::of(vec![
            FolderDoc {
                name: "Rock".to_owned(),
                songs: vec![
                    song(1001, "Sultans of Swing"),
                    song(2005, "Money for Nothing"),
                ],
            },
            FolderDoc {
                name: "Festa".to_owned(),
                songs: vec![song(1001, "Sultans of Swing")],
            },
        ])
    }

    #[test]
    fn a_collection_survives_the_round_trip() {
        let written = document();
        let (read_back, restorable) = read(&written.to_json().expect("json")).expect("read");
        assert_eq!(read_back.folders, written.folders);
        assert_eq!(restorable.len(), 2);
        assert_eq!(
            codes(&restorable[0]),
            vec![SongCode::new(1001), SongCode::new(2005)]
        );
    }

    /// JSON has no comments, so the sentence has to be a field — and somebody opening this a year
    /// from now is the whole reason it is there.
    #[test]
    fn the_file_opens_with_a_sentence_saying_what_it_is() {
        let json = document().to_json().expect("json");
        assert!(json.contains("Restoring only ever adds"), "{json}");
        assert!(json.contains(KIND), "{json}");
    }

    /// The titles are what make a backup worth opening, and they are not what a restore reads.
    #[test]
    fn a_title_rides_along_and_is_ignored_on_the_way_back() {
        let json = document().to_json().expect("json");
        assert!(json.contains("Sultans of Swing"));

        // The same file with every title changed restores identically.
        let meddled = json.replace("Sultans of Swing", "Something Else Entirely");
        let (_, from_meddled) = read(&meddled).expect("read");
        let (_, from_original) = read(&json).expect("read");
        assert_eq!(from_meddled, from_original);
    }

    /// Export preserves and import filters, and the asymmetry is deliberate: dropping a code here
    /// would lose it for good on the next catalog refresh.
    #[test]
    fn a_song_the_catalog_cannot_name_is_still_written() {
        let document = Document::of(vec![FolderDoc {
            name: "Rock".to_owned(),
            songs: vec![SongDoc {
                code: "1001".to_owned(),
                title: None,
                artist: None,
                package_id: None,
                content_hash: None,
            }],
        }]);
        let json = document.to_json().expect("json");
        assert!(json.contains("\"code\": \"1001\""), "{json}");
        let (_, restorable) = read(&json).expect("read");
        assert_eq!(codes(&restorable[0]), vec![SongCode::new(1001)]);
    }

    /// A recovery that did not happen is the expensive failure, so a newer file is read and the
    /// difference reported.
    #[test]
    fn a_document_from_a_newer_format_is_read_and_reported() {
        // Spelled from `FORMAT` rather than from the number it happens to be: this test asserts that
        // a *newer* file is read, and hardcoding the current number turns the replacement into a
        // silent no-op the day the format moves — which is exactly what happened when it did.
        let json = document()
            .to_json()
            .expect("json")
            .replace(&format!("\"format\": {FORMAT}"), "\"format\": 99");
        let (read_back, restorable) = read(&json).expect("a newer file still restores");
        assert!(is_from_the_future(&read_back));
        assert_eq!(restorable.len(), 2);
    }

    #[test]
    fn an_unknown_field_does_not_refuse_a_recovery() {
        let json = document()
            .to_json()
            .expect("json")
            .replace("\"folders\":", "\"invented_later\": 7,\n  \"folders\":");
        assert!(read(&json).is_ok(), "{json}");
    }

    /// The job an XML root element does for the sibling project, done by a field here.
    #[test]
    fn a_file_that_is_not_ours_is_refused() {
        let json = document()
            .to_json()
            .expect("json")
            .replace(KIND, "somebody-elses-export");
        assert_eq!(read(&json), Err(BackupError::NotOurs));

        // Well-formed JSON that happens to carry a `folders` array is exactly what the
        // discriminator exists to turn away.
        assert_eq!(
            read(r#"{"folders":[{"name":"Rock","songs":[{"code":"1001"}]}]}"#),
            Err(BackupError::NotOurs)
        );
    }

    #[test]
    fn a_damaged_file_is_refused_rather_than_half_read() {
        let json = document().to_json().expect("json");
        assert_eq!(read(&json[..json.len() / 2]), Err(BackupError::Damaged));
        assert_eq!(read("not json at all"), Err(BackupError::Damaged));
        assert_eq!(read(""), Err(BackupError::Damaged));
        // A JPEG handed to the picker.
        assert_eq!(read("\u{FFFD}\u{FFFD}\u{FFFD}"), Err(BackupError::Damaged));
    }

    #[test]
    fn a_file_with_nothing_in_it_is_refused() {
        let empty = Document::of(Vec::new());
        assert_eq!(
            read(&empty.to_json().expect("json")),
            Err(BackupError::Empty)
        );

        let no_songs = Document::of(vec![FolderDoc {
            name: "Rock".to_owned(),
            songs: Vec::new(),
        }]);
        assert_eq!(
            read(&no_songs.to_json().expect("json")),
            Err(BackupError::Empty)
        );
    }

    /// `folder.name` is `COLLATE NOCASE UNIQUE`, so a file naming both has to land in one folder.
    #[test]
    fn a_folder_name_differing_only_in_case_is_one_folder() {
        let document = Document::of(vec![
            FolderDoc {
                name: "Rock".to_owned(),
                songs: vec![song(1001, "One")],
            },
            FolderDoc {
                name: "rock".to_owned(),
                songs: vec![song(2005, "Two")],
            },
        ]);
        let (_, restorable) = read(&document.to_json().expect("json")).expect("read");
        assert_eq!(restorable.len(), 1, "one folder, mentioned twice");
        assert_eq!(restorable[0].name, "Rock", "the first mention names it");
        assert_eq!(
            codes(&restorable[0]),
            vec![SongCode::new(1001), SongCode::new(2005)]
        );
    }

    #[test]
    fn a_blank_folder_name_is_dropped_rather_than_failing_the_file() {
        let document = Document::of(vec![
            FolderDoc {
                name: "   ".to_owned(),
                songs: vec![song(1001, "One")],
            },
            FolderDoc {
                name: "Rock".to_owned(),
                songs: vec![song(2005, "Two")],
            },
        ]);
        let (_, restorable) = read(&document.to_json().expect("json")).expect("read");
        assert_eq!(restorable.len(), 1);
        assert_eq!(restorable[0].name, "Rock");
    }

    /// A file from before the identity fields restores exactly as it always did — the two default
    /// to absent, and the row falls to the code rung, which is the only rung it ever had.
    #[test]
    fn a_format_one_document_still_restores_by_its_codes() {
        let json = r#"{
            "note": "whatever this said",
            "kind": "km-remote-favorites",
            "format": 1,
            "written_at": "2026-01-01T00:00:00Z",
            "folders": [{"name": "Rock", "songs": [{"code": "1001", "title": "One"}]}]
        }"#;
        let (document, restorable) = read(json).expect("an older file still restores");
        assert_eq!(document.format, 1);
        assert_eq!(codes(&restorable[0]), vec![SongCode::new(1001)]);
        assert!(
            !restorable[0].songs[0].is_identified(),
            "nothing was carried, so nothing is claimed"
        );
    }

    /// The identity survives the round trip, which is what lets a restore onto a machine that
    /// numbers its catalog differently land on the right songs.
    #[test]
    fn the_package_and_hash_survive_the_round_trip() {
        let document = Document::of(vec![FolderDoc {
            name: "Rock".to_owned(),
            songs: vec![SongDoc {
                code: "1001".to_owned(),
                title: Some("One".to_owned()),
                artist: None,
                package_id: Some("vol1".to_owned()),
                content_hash: Some("aaa".to_owned()),
            }],
        }]);
        let (_, restorable) = read(&document.to_json().expect("json")).expect("read");
        let song = &restorable[0].songs[0];
        assert_eq!(song.package_id.as_deref(), Some("vol1"));
        assert_eq!(song.content_hash.as_deref(), Some("aaa"));
    }

    /// `Strict about identity, forgiving about shape`, applied to the new fields: a hand-edited file
    /// that left one blank means the same as one that left it out, and reading `""` as a hash
    /// nothing matches would strand the row on a rung it should have fallen straight past.
    #[test]
    fn a_blank_identity_reads_as_absent_rather_than_as_a_hash_of_nothing() {
        let json = r#"{
            "kind": "km-remote-favorites",
            "format": 2,
            "folders": [{"name": "Rock", "songs": [
                {"code": "1001", "package_id": "  ", "content_hash": ""}
            ]}]
        }"#;
        let (_, restorable) = read(json).expect("read");
        assert!(!restorable[0].songs[0].is_identified());
        assert_eq!(restorable[0].songs[0].package_id, None);
    }

    /// Two rows naming one code are one membership however they are labelled — the same rule
    /// `add_songs` applies to the same file read twice.
    #[test]
    fn two_rows_for_one_code_are_one_membership() {
        let json = r#"{
            "kind": "km-remote-favorites",
            "format": 2,
            "folders": [{"name": "Rock", "songs": [
                {"code": "1001", "content_hash": "aaa"},
                {"code": "1001", "content_hash": "bbb"}
            ]}]
        }"#;
        let (_, restorable) = read(json).expect("read");
        assert_eq!(codes(&restorable[0]), vec![SongCode::new(1001)]);
        assert_eq!(
            restorable[0].songs[0].content_hash.as_deref(),
            Some("aaa"),
            "the first wins"
        );
    }

    /// The reason `SongDoc::code` is a `String`: one fumbled row must not cost the other thousand.
    #[test]
    fn a_mistyped_code_costs_one_row_and_not_the_file() {
        let document = Document::of(vec![FolderDoc {
            name: "Rock".to_owned(),
            songs: vec![
                SongDoc {
                    code: "10O1".to_owned(), // a letter O, typed by hand
                    title: None,
                    artist: None,
                    package_id: None,
                    content_hash: None,
                },
                song(2005, "Two"),
            ],
        }]);
        let (_, restorable) = read(&document.to_json().expect("json")).expect("read");
        assert_eq!(codes(&restorable[0]), vec![SongCode::new(2005)]);
        assert_eq!(restorable[0].unreadable, 1, "counted, not silently dropped");
    }

    /// De-duplicated within a folder and not across them.
    #[test]
    fn a_song_filed_in_two_folders_stays_in_both() {
        let (_, restorable) = read(&document().to_json().expect("json")).expect("read");
        assert!(codes(&restorable[0]).contains(&SongCode::new(1001)));
        assert!(codes(&restorable[1]).contains(&SongCode::new(1001)));
    }

    #[test]
    fn a_song_written_twice_in_one_folder_is_filed_once() {
        let document = Document::of(vec![FolderDoc {
            name: "Rock".to_owned(),
            songs: vec![song(1001, "One"), song(1001, "One again")],
        }]);
        let (_, restorable) = read(&document.to_json().expect("json")).expect("read");
        assert_eq!(codes(&restorable[0]), vec![SongCode::new(1001)]);
    }

    #[test]
    fn the_filename_carries_the_day_it_was_written() {
        let mut document = document();
        document.written_at = "2026-09-07T18:02:11Z".to_owned();
        assert_eq!(document.filename(), "km-favorites-2026-09-07.json");
        assert!(
            document.filename().is_ascii(),
            "no RFC 5987 encoding needed"
        );
    }

    /// The calendar arithmetic, against dates a leap-year rule gets wrong if it is written by hand.
    #[test]
    fn the_timestamp_is_the_right_day_across_the_leap_year_rules() {
        let at = |seconds: u64| written_at(UNIX_EPOCH + std::time::Duration::from_secs(seconds));
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(at(86_400), "1970-01-02T00:00:00Z");
        // 2000 was a leap year and 1900 was not; 2024-02-29 is the one to get wrong.
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(at(1_788_000_000), "2026-08-29T10:40:00Z");
    }

    #[test]
    fn every_error_this_reader_raises_reaches_a_message_that_exists() {
        for error in BackupError::ALL {
            let key = error.message_key();
            for locale in km_locale::Locale::ALL {
                assert!(
                    crate::words::messages(*locale).keys().contains(key),
                    "no `{key}` in the {locale} catalog"
                );
            }
        }
    }
}
