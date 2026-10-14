//! What one folder holds, and what a person changed about it before building.
//!
//! **Nothing here is stored.** A run reads a folder, lists its songs, takes a few renames and
//! exclusions, and writes packages. The next run reads the folder again. A tool that kept its
//! edits would be a curation database, and `km-package-builder` is that tool. See `A package can be
//! built straight from a folder` in `docs/decisions/curation.md`.
//!
//! **Numbers are dense and follow the list.** Every kept song takes the next number, and a package
//! is at most [`km_songcode::MAX_SLOT`] of them. Excluding a song closes its gap, because a package
//! nobody curated has no printed book whose numbers a gap would protect.

use std::path::{Path, PathBuf};

use km_pack::volumes::split_into_volumes;
use km_pack::{CdgOrphan, Description, Rejection, Spec, SpecPackage, SpecSong};

/// Which kind of song a row is, read from its file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A MIDI or karaoke MIDI file.
    Midi,
    /// A video file.
    Video,
    /// An MP3+G pair, listed under its MP3.
    Cdg,
    /// An UltraStar `.txt` and the audio it names.
    UltraStar,
    /// An LRC file and the MP3 of its stem.
    Lrc,
}

impl Kind {
    /// Every key [`Self::key`] returns, for the catalog parity tests.
    pub const KEYS: &[&str] = &[
        "kind-midi",
        "kind-video",
        "kind-cdg",
        "kind-ultrastar",
        "kind-lrc",
    ];

    /// The kind a description's `file:` value names.
    ///
    /// `describe` writes an MP3+G pair under its audio, and an UltraStar or LRC song under its text
    /// file, so the extension is the whole answer.
    #[must_use]
    pub fn of(file: &str) -> Self {
        let extension = Path::new(file)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if extension == "txt" {
            Self::UltraStar
        } else if extension == "lrc" {
            Self::Lrc
        } else if km_pack::VIDEO_EXTENSIONS.contains(&extension.as_str()) {
            Self::Video
        } else if matches!(extension.as_str(), "mid" | "midi" | "kar" | "rmi") {
            Self::Midi
        } else {
            Self::Cdg
        }
    }

    /// The kind of a song in `folder`, which for a zip is the kind of the song inside it.
    ///
    /// A zip's extension says nothing about the song it holds, so the zip is read.
    #[must_use]
    pub fn of_in(folder: &Path, file: &str) -> Self {
        if km_pack::is_zip_candidate(Path::new(file))
            && let km_pack::ZipVerdict::Song(song) = km_pack::read_zip(&folder.join(file))
        {
            return match song.kind() {
                km_pack::ZipKind::Cdg => Self::Cdg,
                km_pack::ZipKind::UltraStar => Self::UltraStar,
                km_pack::ZipKind::Lrc => Self::Lrc,
            };
        }
        Self::of(file)
    }

    /// The catalog key a page draws the kind with.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Midi => "kind-midi",
            Self::Video => "kind-video",
            Self::Cdg => "kind-cdg",
            Self::UltraStar => "kind-ultrastar",
            Self::Lrc => "kind-lrc",
        }
    }
}

/// One song the folder holds.
#[derive(Debug, Clone)]
pub struct Row {
    /// The song as the description has it, with any rename laid over it.
    pub song: SpecSong,
    /// What kind of song it is.
    pub kind: Kind,
    /// Its suitability out of 10. Only a MIDI song has one.
    pub suitability: Option<u8>,
    /// Whether it goes into the package.
    pub kept: bool,
}

impl Row {
    /// The file's name without its folder or extension, which a build calls a song with no title.
    #[must_use]
    pub fn stem(&self) -> String {
        Path::new(&self.song.file)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Whether it has a suitability, and that suitability is below [`GOOD_SUITABILITY`].
    #[must_use]
    pub fn is_low(&self) -> bool {
        self.suitability
            .is_some_and(|value| value < GOOD_SUITABILITY)
    }
}

/// The lowest suitability that **Leave out below 8** keeps: the start of the `high` band.
pub const GOOD_SUITABILITY: u8 = 8;

/// A file the folder holds that is not a song, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left {
    /// Its path under the folder, with forward slashes.
    pub file: String,
    /// Why it is not a song.
    pub why: Why,
}

/// Why a file is not a song. Each is a catalog key; see [`Why::key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// It could not be read.
    Unreadable,
    /// It is not a MIDI file this build can read.
    NotMidi,
    /// It holds the same bytes as the named file, which is listed.
    Copy(String),
    /// An UltraStar file this project does not package, with `km-pack`'s own reason.
    UltraStar(String),
    /// An LRC file this project does not package, with `km-pack`'s own reason.
    Lrc(String),
    /// An MP3 with no `.cdg` beside it.
    NoGraphics,
    /// A `.cdg` with no audio beside it.
    NoAudio,
    /// Anything else `km-pack` reported, in its own words.
    Other(String),
}

impl Why {
    /// Every key [`Self::key`] returns, for the catalog parity tests.
    pub const KEYS: &[&str] = &[
        "left-unreadable",
        "left-not-midi",
        "left-copy",
        "left-ultrastar",
        "left-lrc",
        "left-no-graphics",
        "left-no-audio",
        "left-other",
    ];

    /// The catalog key this reason is worded by. `left-copy`, `left-ultrastar`, `left-lrc` and
    /// `left-other` take a `$detail`.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Self::Unreadable => "left-unreadable",
            Self::NotMidi => "left-not-midi",
            Self::Copy(_) => "left-copy",
            Self::UltraStar(_) => "left-ultrastar",
            Self::Lrc(_) => "left-lrc",
            Self::NoGraphics => "left-no-graphics",
            Self::NoAudio => "left-no-audio",
            Self::Other(_) => "left-other",
        }
    }

    /// The value a key with a `$detail` fills in.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Copy(detail)
            | Self::UltraStar(detail)
            | Self::Lrc(detail)
            | Self::Other(detail) => Some(detail),
            _ => None,
        }
    }
}

/// Where a kept song lands: which volume, and which number inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// The volume, from 1.
    pub volume: usize,
    /// The song's number inside that volume, from 1.
    pub number: usize,
}

/// What the package form holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageForm {
    /// The package's name. A set of volumes adds ` vol<n>` to it.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Who made it, when somebody says.
    pub publisher: Option<String>,
    /// The language every song with none of its own is filed under.
    pub language: String,
    /// Where the packages are written.
    pub out_dir: PathBuf,
    /// Whether the packages are linked: they hold no song file, and name each one by its path
    /// below [`linked_base`](Self::linked_base).
    pub linked: bool,
    /// The folder a linked package's paths start at: the folder that was read, or one above it.
    pub linked_base: PathBuf,
}

impl PackageForm {
    /// What the form offers for a folder nobody has described yet.
    ///
    /// The folder's own name, version `1.0.0`, `und` for the language, and the folder's parent to
    /// write into. The parent rather than the folder, so that a package is not written into the
    /// folder it was read from.
    #[must_use]
    pub fn for_folder(folder: &Path) -> Self {
        Self {
            name: folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Karaoke".to_owned()),
            version: "1.0.0".to_owned(),
            publisher: None,
            language: UNDETERMINED.to_owned(),
            out_dir: folder.parent().unwrap_or(folder).to_path_buf(),
            // Off, because a package that holds its songs plays on any machine.
            linked: false,
            linked_base: folder.to_path_buf(),
        }
    }
}

/// The folders between `base` and the folder that was read, joined with `/`.
///
/// Every linked path starts with it, so a package built from one collection can share the
/// machine's folder for linked packages with packages built from the folder above it. It is empty
/// when `base` is the folder itself. Both folders are resolved on the disk, so a difference of
/// case or a link does not make a folder look outside its parent.
///
/// # Errors
///
/// The catalog key that says why: `base` or the folder is not there, the folder is not inside
/// `base`, or a folder name between them is one a linked path cannot carry.
pub fn linked_prefix(folder: &Path, base: &Path) -> Result<String, &'static str> {
    let (Ok(folder), Ok(base)) = (std::fs::canonicalize(folder), std::fs::canonicalize(base))
    else {
        return Err("said-linked-base-missing");
    };
    let Ok(below) = folder.strip_prefix(&base) else {
        return Err("said-linked-base-outside");
    };
    let prefix = below
        .components()
        .map(|name| name.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    if !prefix.is_empty() && km_kmpkg::linked::check(&prefix).is_err() {
        return Err("said-linked-base-name");
    }
    Ok(prefix)
}

/// `file` below `prefix`, as a linked package's description names it.
fn below(prefix: &str, file: &str) -> String {
    if prefix.is_empty() {
        file.to_owned()
    } else {
        format!("{prefix}/{file}")
    }
}

/// The standard's own "undetermined", and what the form offers first.
///
/// A build refuses a song with no language, and nobody reviewed these songs. `und` is the honest
/// answer. See `tools/cmd/km-pack/CLAUDE.md`.
pub const UNDETERMINED: &str = "und";

/// One volume ready to build: its description and where it goes.
#[derive(Debug, Clone)]
pub struct Planned {
    /// What the volume holds.
    pub spec: Spec,
    /// The file it is written to.
    pub out: PathBuf,
}

/// A folder's songs, as read and as changed.
#[derive(Debug, Clone)]
pub struct Session {
    /// The folder the songs were read from, which every `file:` value is relative to.
    pub folder: PathBuf,
    /// Every song, in the order the folder was read.
    pub rows: Vec<Row>,
    /// Every file that is not a song.
    pub left: Vec<Left>,
}

impl Session {
    /// A session over what `describe` found in `folder`.
    #[must_use]
    pub fn from_description(folder: PathBuf, description: Description) -> Self {
        let Description {
            spec,
            rejected,
            orphans,
            suitability,
        } = description;

        let file_of = |number: u32| {
            spec.songs
                .iter()
                .find(|song| song.number == Some(number))
                .map(|song| song.file.clone())
                .unwrap_or_else(|| number.to_string())
        };
        let relative = |path: &Path| {
            path.strip_prefix(&folder)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let mut left: Vec<Left> = rejected
            .iter()
            .map(|(path, rejection)| Left {
                file: relative(path),
                why: match rejection {
                    Rejection::Unreadable => Why::Unreadable,
                    Rejection::NotMidi => Why::NotMidi,
                    Rejection::DuplicateOf(number) => Why::Copy(file_of(*number)),
                    Rejection::UltraStar(reason) => Why::UltraStar(reason.clone()),
                    Rejection::Lrc(reason) => Why::Lrc(reason.clone()),
                    other => Why::Other(other.to_string()),
                },
            })
            .collect();
        left.extend(orphans.iter().map(|(path, orphan)| Left {
            file: relative(path),
            why: match orphan {
                CdgOrphan::NoGraphics => Why::NoGraphics,
                CdgOrphan::NoAudio => Why::NoAudio,
            },
        }));
        left.sort_by(|a, b| a.file.cmp(&b.file));

        let rows = spec
            .songs
            .into_iter()
            .map(|song| Row {
                kind: Kind::of_in(&folder, &song.file),
                suitability: suitability.get(&song.file).copied(),
                kept: true,
                song,
            })
            .collect();
        Self { folder, rows, left }
    }

    /// How many songs go into the package.
    #[must_use]
    pub fn kept(&self) -> usize {
        self.rows.iter().filter(|row| row.kept).count()
    }

    /// How many packages the kept songs make.
    #[must_use]
    pub fn volumes(&self) -> usize {
        self.kept().div_ceil(per_volume())
    }

    /// Where each row lands, in row order: `None` for a row that is left out.
    #[must_use]
    pub fn slots(&self) -> Vec<Option<Slot>> {
        let per_volume = per_volume();
        let mut next = 0;
        self.rows
            .iter()
            .map(|row| {
                row.kept.then(|| {
                    let at = next;
                    next += 1;
                    Slot {
                        volume: at / per_volume + 1,
                        number: at % per_volume + 1,
                    }
                })
            })
            .collect()
    }

    /// Renames a song. A blank value takes back whatever the file says.
    pub fn rename(&mut self, index: usize, title: &str, artist: &str) -> bool {
        let Some(row) = self.rows.get_mut(index) else {
            return false;
        };
        row.song.title = nonblank(title);
        row.song.artist = nonblank(artist);
        true
    }

    /// Puts every song from `from` to `to` in the package, or leaves them all out.
    ///
    /// Both ends count, and either may come first, because a shift-click reaches back up the list
    /// as often as down it. One song is a run whose two ends are the same.
    pub fn keep(&mut self, from: usize, to: usize, kept: bool) -> bool {
        let (first, last) = (from.min(to), from.max(to));
        let Some(rows) = self.rows.get_mut(first..=last) else {
            return false;
        };
        for row in rows {
            row.kept = kept;
        }
        true
    }

    /// Whether a kept song has a suitability below [`GOOD_SUITABILITY`].
    #[must_use]
    pub fn keeps_low(&self) -> bool {
        self.rows.iter().any(|row| row.kept && row.is_low())
    }

    /// Leaves out every song whose suitability is below [`GOOD_SUITABILITY`].
    ///
    /// A song with no suitability stays as it is, because nothing measured it.
    pub fn leave_out_low(&mut self) {
        for row in self.rows.iter_mut().filter(|row| row.is_low()) {
            row.kept = false;
        }
    }

    /// The first kept song's path below the machine's folder for linked packages.
    #[must_use]
    pub fn linked_example(&self, prefix: &str) -> Option<String> {
        self.rows
            .iter()
            .find(|row| row.kept)
            .map(|row| below(prefix, &row.song.file))
    }

    /// Every volume the kept songs make, each with its description and its file.
    ///
    /// `id` is the package's id, which the first volume keeps. The description says `uncurated:
    /// true`, which is what makes every file this tool writes carry the flag. `prefix` is what
    /// [`linked_prefix`] gave, and a linked package's paths start with it.
    #[must_use]
    pub fn plan(&self, form: &PackageForm, id: String, prefix: &str) -> Vec<Planned> {
        let songs: Vec<SpecSong> = self
            .rows
            .iter()
            .filter(|row| row.kept)
            .zip(1..)
            .map(|(row, number)| SpecSong {
                number: Some(number),
                file: if form.linked {
                    below(prefix, &row.song.file)
                } else {
                    row.song.file.clone()
                },
                ..row.song.clone()
            })
            .collect();
        let spec = Spec {
            package: SpecPackage {
                id,
                name: form.name.trim().to_owned(),
                version: form.version.trim().to_owned(),
                publisher: form.publisher.clone(),
                created: None,
                volume: None,
                default_language: nonblank(&form.language),
                encoding: None,
                start_number: 1,
                transcode: true,
                out: None,
                uncurated: true,
                // The paths are below `form.linked_base`, which the build resolves them against.
                linked: form.linked,
            },
            root: None,
            songs,
        };
        split_into_volumes(&spec, per_volume())
            .into_iter()
            .map(|spec| Planned {
                out: form.out_dir.join(file_name(&spec.package)),
                spec,
            })
            .collect()
    }
}

/// How many songs one package holds.
fn per_volume() -> usize {
    usize::from(km_songcode::MAX_SLOT)
}

/// `<name>-<version>.kmpkg`, the name `km-package-builder` gives a package.
///
/// The id stands in for a name that folds to nothing. See `What the tool calls the package it
/// writes` in `docs/decisions/curation.md`.
#[must_use]
pub fn file_name(package: &SpecPackage) -> String {
    let stem = km_kmpkg::name_slug(&package.name).unwrap_or_else(|| package.id.clone());
    let version = if km_kmpkg::is_safe_name(&package.version) {
        package.version.clone()
    } else {
        km_kmpkg::name_slug(&package.version).unwrap_or_else(|| "1.0.0".to_owned())
    };
    format!("{stem}-{version}.kmpkg")
}

fn nonblank(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zip_is_drawn_as_the_kind_of_song_inside_it() {
        use std::io::Write;

        let scratch = km_testkit::Scratch::new("simple-zip-kind");
        let file = std::fs::File::create(scratch.join("song.zip")).expect("create a zip");
        let mut zip = zip::ZipWriter::new(file);
        for (entry, bytes) in [
            (
                "song.txt",
                &b"#TITLE:Song\n#MP3:song.mp3\n#BPM:300\n: 0 4 0 la\nE\n"[..],
            ),
            ("song.mp3", b"audio"),
        ] {
            zip.start_file(entry, zip::write::SimpleFileOptions::default())
                .expect("start an entry");
            zip.write_all(bytes).expect("write an entry");
        }
        zip.finish().expect("finish the zip");

        assert_eq!(Kind::of_in(&scratch, "song.zip"), Kind::UltraStar);
        // A name alone says nothing, and a zip that is not there is drawn as its extension is.
        assert_eq!(Kind::of_in(&scratch, "absent.zip"), Kind::of("absent.zip"));
        assert_eq!(Kind::of_in(&scratch, "song.lrc"), Kind::Lrc);
    }

    fn session(count: u32) -> Session {
        Session {
            folder: PathBuf::from("/tunes/karaoke"),
            rows: (1..=count)
                .map(|number| Row {
                    song: SpecSong {
                        file: format!("{number}.kar"),
                        number: Some(number),
                        title: Some(format!("Song {number}")),
                        ..SpecSong::default()
                    },
                    kind: Kind::Midi,
                    suitability: Some(7),
                    kept: true,
                })
                .collect(),
            left: Vec::new(),
        }
    }

    fn form() -> PackageForm {
        PackageForm::for_folder(Path::new("/tunes/karaoke"))
    }

    #[test]
    fn a_kind_is_read_from_the_extension() {
        assert_eq!(Kind::of("a/b.KAR"), Kind::Midi);
        assert_eq!(Kind::of("clip.mp4"), Kind::Video);
        assert_eq!(Kind::of("pair.mp3"), Kind::Cdg);
        assert_eq!(Kind::of("song.txt"), Kind::UltraStar);
        assert_eq!(Kind::of("song.LRC"), Kind::Lrc);
    }

    #[test]
    fn the_form_offers_the_folders_name_and_writes_beside_it() {
        let form = form();
        assert_eq!(form.name, "karaoke");
        assert_eq!(form.version, "1.0.0");
        assert_eq!(form.language, "und");
        assert_eq!(form.out_dir, PathBuf::from("/tunes"));
    }

    #[test]
    fn a_run_is_kept_or_left_out_whichever_end_comes_first() {
        let mut session = session(5);
        assert!(session.keep(3, 1, false));
        let kept: Vec<bool> = session.slots().iter().map(Option::is_some).collect();
        assert_eq!(kept, [true, false, false, false, true]);
        assert!(session.keep(1, 2, true));
        let kept: Vec<bool> = session.slots().iter().map(Option::is_some).collect();
        assert_eq!(kept, [true, true, true, false, true]);
        assert!(
            !session.keep(4, 5, false),
            "a run past the end changes nothing"
        );
        assert!(session.slots()[4].is_some());
    }

    #[test]
    fn a_song_below_8_is_left_out_and_one_with_no_suitability_stays() {
        let mut session = session(4);
        session.rows[1].suitability = Some(8);
        session.rows[2].suitability = None;
        session.rows[2].kind = Kind::Video;
        assert!(session.keeps_low());
        session.leave_out_low();
        let kept: Vec<bool> = session.rows.iter().map(|row| row.kept).collect();
        assert_eq!(kept, [false, true, true, false]);
        assert!(!session.keeps_low());
    }

    #[test]
    fn leaving_a_song_out_closes_its_gap() {
        let mut session = session(3);
        session.keep(1, 1, false);
        assert_eq!(
            session.slots(),
            [
                Some(Slot {
                    volume: 1,
                    number: 1
                }),
                None,
                Some(Slot {
                    volume: 1,
                    number: 2
                }),
            ]
        );
        let planned = session.plan(&form(), "0123456789abcdef".to_owned(), "");
        assert_eq!(planned.len(), 1);
        let files: Vec<&str> = planned[0]
            .spec
            .songs
            .iter()
            .map(|song| song.file.as_str())
            .collect();
        assert_eq!(files, ["1.kar", "3.kar"]);
        assert_eq!(planned[0].spec.songs[1].number, Some(2));
    }

    #[test]
    fn a_rename_is_laid_over_the_song_and_a_blank_takes_it_back() {
        let mut session = session(1);
        session.rename(0, "  Better  ", "Somebody");
        assert_eq!(session.rows[0].song.title.as_deref(), Some("Better"));
        assert_eq!(session.rows[0].song.artist.as_deref(), Some("Somebody"));
        session.rename(0, " ", "");
        assert_eq!(session.rows[0].song.title, None);
        assert_eq!(session.rows[0].song.artist, None);
        assert!(!session.rename(9, "x", "y"), "no such row");
    }

    #[test]
    fn every_plan_is_uncurated_and_named_by_the_package() {
        let planned = session(2).plan(&form(), "0123456789abcdef".to_owned(), "");
        assert!(planned[0].spec.package.uncurated);
        assert_eq!(
            planned[0].spec.package.default_language.as_deref(),
            Some("und")
        );
        assert_eq!(planned[0].out, PathBuf::from("/tunes/karaoke-1.0.0.kmpkg"));
    }

    #[test]
    fn a_plan_is_linked_only_when_the_form_asks() {
        let planned = session(2).plan(&form(), "0123456789abcdef".to_owned(), "");
        assert!(!planned[0].spec.package.linked, "off until somebody asks");

        let linked = PackageForm {
            linked: true,
            ..form()
        };
        let planned = session(1000).plan(&linked, "0123456789abcdef".to_owned(), "rock");
        assert_eq!(planned.len(), 2);
        assert!(
            planned.iter().all(|volume| volume.spec.package.linked),
            "every volume of a linked package is linked"
        );
        assert!(planned.iter().all(|volume| volume.spec.package.uncurated));
        assert_eq!(planned[0].spec.songs[0].file, "rock/1.kar");
    }

    #[test]
    fn a_package_that_holds_its_songs_keeps_their_paths() {
        let planned = session(1).plan(&form(), "0123456789abcdef".to_owned(), "rock");
        assert_eq!(planned[0].spec.songs[0].file, "1.kar");
    }

    #[test]
    fn the_paths_start_at_the_base_folder() {
        let scratch = km_testkit::Scratch::new("simple-linked-prefix");
        let folder = scratch.join("cdg").join("Some Collection");
        std::fs::create_dir_all(&folder).expect("a folder");

        assert_eq!(linked_prefix(&folder, &folder), Ok(String::new()));
        assert_eq!(
            linked_prefix(&folder, scratch.path()),
            Ok("cdg/Some Collection".to_owned())
        );
        assert_eq!(
            linked_prefix(scratch.path(), &folder),
            Err("said-linked-base-outside")
        );
        assert_eq!(
            linked_prefix(&folder, &scratch.join("absent")),
            Err("said-linked-base-missing")
        );
    }

    #[test]
    fn the_example_is_the_first_kept_song_below_the_prefix() {
        let mut session = session(2);
        assert_eq!(session.linked_example(""), Some("1.kar".to_owned()));
        session.rows[0].kept = false;
        assert_eq!(session.linked_example("cdg"), Some("cdg/2.kar".to_owned()));
    }

    #[test]
    fn a_folder_past_one_package_becomes_volumes() {
        let session = session(1000);
        assert_eq!(session.volumes(), 2);
        assert_eq!(
            session.slots()[999],
            Some(Slot {
                volume: 2,
                number: 1
            })
        );
        let planned = session.plan(&form(), "0123456789abcdef".to_owned(), "");
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[1].spec.songs.len(), 1);
        assert_eq!(planned[1].spec.package.name, "karaoke vol2");
        assert_eq!(
            planned[1].out,
            PathBuf::from("/tunes/karaoke-vol2-1.0.0.kmpkg")
        );
        assert!(planned.iter().all(|volume| volume.spec.package.uncurated));
    }

    #[test]
    fn a_name_that_folds_to_nothing_is_filed_under_the_id() {
        let mut form = form();
        form.name = "日本".to_owned();
        let planned = session(1).plan(&form, "0123456789abcdef".to_owned(), "");
        assert_eq!(
            planned[0].out,
            PathBuf::from("/tunes/0123456789abcdef-1.0.0.kmpkg")
        );
    }
}
