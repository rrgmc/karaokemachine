//! Walking a folder and writing down what is in it: the other half of [`crate::spec`].
//!
//! Walking a folder and packaging the result in one step leaves what it decided — which files are
//! songs, what they are called, what number each gets, which copies are duplicates — written
//! nowhere a person can read or correct. A description is the reviewable middle step.
//!
//! # What goes in the description, and what does not
//!
//! For a MIDI file, everything detection found: the title, the artist and the language. That is the
//! point of the file — a list of paths would be a manifest of work rather than something anybody
//! reads — and it costs nothing at build time, because the build re-derives the same values from the
//! same bytes and [`crate::apply_edits`] therefore marks none of them as corrections. See
//! [`crate::spec`] for why provenance works that way.
//!
//! For a video or an MP3+G pair, only the path and the number. Both would have to be *probed* to say
//! more — a container tag, an ID3 frame — and probing a video needs the `video` feature, which this
//! must work without. A build with no such feature still has to list every `.mp4` it finds, or a
//! description generated on one silently loses them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use km_kmpkg::Language;
use km_song::{ParseOptions, Song};
use km_suitability::Analysis;

use crate::spec::{Spec, SpecPackage, SpecSong};
use crate::{CdgOrphan, Override, Rejection};

/// What to call the package, and which files are worth describing.
///
/// The three filters — [`Self::min_suitability`], [`Self::require_lyrics`] and [`Self::limit`] — used to
/// be flags on `km-pack build`. They belong here: they decide *what is selected*, and selection is
/// now what a description records. A file they exclude simply has no row.
#[derive(Debug, Clone)]
pub struct DescribeOptions {
    /// Package identifier. Reinstalling the same one replaces it, so keep it stable.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Who made it.
    pub publisher: Option<String>,
    /// The first song number to hand out.
    pub start_number: u32,
    /// Written into the description as `package.default_language`.
    pub default_language: Option<String>,
    /// Force every song's lyric encoding rather than detecting it.
    pub encoding: Option<String>,
    /// Whether the build should re-encode videos outside the packaging profile.
    pub transcode: bool,
    /// Where the built package should go, relative to the description.
    pub out: Option<String>,
    /// Leave out songs scoring below this out of 10.
    pub min_suitability: Option<u8>,
    /// Leave out files with no lyrics at all, which cannot be sung from.
    pub require_lyrics: bool,
    /// Stop after this many accepted songs.
    pub limit: Option<usize>,
    /// Per-file overrides read from a CSV, keyed by [`crate::index_key`].
    pub index: BTreeMap<String, Override>,
}

impl Default for DescribeOptions {
    fn default() -> Self {
        Self {
            id: "package".to_owned(),
            name: "package".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: None,
            encoding: None,
            transcode: true,
            out: None,
            min_suitability: None,
            require_lyrics: false,
            limit: None,
            index: BTreeMap::new(),
        }
    }
}

/// A description, and everything the walk decided not to put in it.
#[derive(Debug)]
pub struct Description {
    /// What was found, ready to write or to build.
    pub spec: Spec,
    /// Files that were looked at and left out, with the reason.
    pub rejected: Vec<(PathBuf, Rejection)>,
    /// Half an MP3+G pair, which is not a song and is never silently dropped.
    ///
    /// Said out loud rather than skipped: a folder that quietly loses files is a folder nobody can
    /// reconcile against what they put in it.
    pub orphans: Vec<(PathBuf, CdgOrphan)>,
}

/// Walks a folder and describes what is in it.
///
/// `on_progress` is called with `(files done, files in total)` as the MIDI walk proceeds — the one
/// slow phase, since every file is read, parsed and analyzed. It is not called for videos or MP3+G
/// pairs, which are only listed.
pub fn describe(
    dir: &Path,
    options: &DescribeOptions,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<Description> {
    let mut songs = Vec::new();
    let mut rejected = Vec::new();

    // Every number the index claimed, so the walk never hands one out twice.
    let mut taken: BTreeSet<u32> = options
        .index
        .values()
        .filter_map(|over| over.number)
        .collect();
    let mut next_number = options.start_number.max(1);
    let mut hashes: BTreeMap<String, u32> = BTreeMap::new();

    let mut files = Vec::new();
    crate::collect_midi(dir, &mut files);
    // Sorted so the same folder always produces the same description.
    files.sort();
    let total = files.len();

    for (done, path) in files.iter().enumerate() {
        on_progress(done, total);
        if options.limit.is_some_and(|limit| songs.len() >= limit) {
            break;
        }
        let key = crate::index_key(dir, path);
        let over = options.index.get(&key).cloned().unwrap_or_default();

        let Ok(bytes) = std::fs::read(path) else {
            rejected.push((path.clone(), Rejection::Unreadable));
            continue;
        };

        // Before parsing: a real corpus is full of copies, and hashing is far cheaper than parsing.
        let hash = km_kmpkg::content_hash(&bytes);
        if let Some(existing) = hashes.get(&hash) {
            rejected.push((path.clone(), Rejection::DuplicateOf(*existing)));
            continue;
        }

        let encoding = over.encoding.clone().or_else(|| options.encoding.clone());
        let parse = ParseOptions {
            declared_encoding: encoding.clone(),
            inference: None,
        };
        let Ok(parsed) = Song::parse(&bytes, &parse) else {
            rejected.push((path.clone(), Rejection::NotMidi));
            continue;
        };

        let analysis = Analysis::of(&parsed);
        if options.require_lyrics && parsed.lyrics.is_empty() {
            rejected.push((path.clone(), Rejection::NoLyrics));
            continue;
        }
        if let Some(minimum) = options.min_suitability
            && analysis.suitability_value() < minimum
        {
            rejected.push((
                path.clone(),
                Rejection::LowSuitability(analysis.suitability_value()),
            ));
            continue;
        }

        let Some(number) = claim(over.number, &mut taken, &mut next_number) else {
            rejected.push((path.clone(), Rejection::NoNumber));
            continue;
        };

        // The detected values, so the description reads as a finished answer. They are what the
        // build would work out for itself, so writing them back records no correction.
        let language = over
            .language
            .map(|language| language.code().to_owned())
            .or_else(|| {
                Language::detect(parsed.meta.language.as_deref(), Some(parsed.decoder.name()))
                    .map(|language| language.code().to_owned())
            });

        hashes.insert(hash, number);
        songs.push(SpecSong {
            file: relative(dir, path),
            number: Some(number),
            title: Some(
                over.title
                    .or_else(|| parsed.meta.title.clone())
                    .unwrap_or_else(|| crate::file_stem(path)),
            ),
            artist: over.artist.or_else(|| parsed.meta.artist.clone()),
            language,
            // From the `--index` CSV where one names them, and nothing else: no file says what a
            // song is filed under, so a walk of a folder has nothing to detect.
            tags: over.tags,
            encoding,
            transpose: None,
            // Silent for the reason `fixes` below is: a build works this one out from the file, and
            // writing the answer down would turn a proposal into a decision nobody made.
            lyrics_hidden: None,
            // Silent rather than empty, so a build detects them. Writing a list here would freeze
            // whatever this walk found into a description that then stops learning.
            fixes: None,
            // Silent for the same reason: a walk detects the melody channel and writing that
            // answer down would turn a proposal into a decision nobody made.
            melody: None,
            graphics: None,
        });
    }
    on_progress(total, total);

    // Videos and MP3+G pairs after the MIDI files, which is the order `km-pack build` walked them in
    // and therefore the numbering an existing package already has.
    // The UltraStar files are read before the videos and the pairs are listed, because what they name
    // is part of their song: the MP3 is not a pair missing its graphics, and the video is not a song.
    let (mut sung, mut refused) = (Vec::new(), Vec::new());
    crate::ultrastar::collect_ultrastar(dir, &mut sung, &mut refused);
    sung.sort_by(|a, b| a.text.cmp(&b.text));
    let claimed: BTreeSet<PathBuf> = sung
        .iter()
        .flat_map(crate::UltraStarSource::claimed_media)
        .collect();

    let mut videos = Vec::new();
    crate::collect_videos(dir, &mut videos);
    videos.retain(|path| !claimed.contains(path));
    videos.sort();
    for path in &videos {
        if options.limit.is_some_and(|limit| songs.len() >= limit) {
            break;
        }
        let over = options
            .index
            .get(&crate::index_key(dir, path))
            .cloned()
            .unwrap_or_default();
        let Some(number) = claim(over.number, &mut taken, &mut next_number) else {
            rejected.push((path.clone(), Rejection::NoNumber));
            continue;
        };
        songs.push(SpecSong {
            file: relative(dir, path),
            number: Some(number),
            title: over.title,
            artist: over.artist,
            language: over.language.map(|language| language.code().to_owned()),
            ..SpecSong::default()
        });
    }

    let mut pairs = Vec::new();
    let mut orphans = Vec::new();
    crate::collect_cdg(dir, &mut pairs, &mut orphans);
    orphans.retain(|(path, _)| !claimed.contains(path));
    pairs.sort_by(|a, b| a.audio.cmp(&b.audio));
    orphans.sort_by(|a, b| a.0.cmp(&b.0));
    for pair in &pairs {
        if options.limit.is_some_and(|limit| songs.len() >= limit) {
            break;
        }
        let over = options
            .index
            .get(&crate::index_key(dir, &pair.audio))
            .cloned()
            .unwrap_or_default();
        let Some(number) = claim(over.number, &mut taken, &mut next_number) else {
            rejected.push((pair.audio.clone(), Rejection::NoNumber));
            continue;
        };
        songs.push(SpecSong {
            file: relative(dir, &pair.audio),
            number: Some(number),
            title: over.title,
            artist: over.artist,
            language: over.language.map(|language| language.code().to_owned()),
            // Found by rule at build time, exactly as it is at play time. Named here only when the
            // pairing is not the sibling with the same stem, which `collect_cdg` never produces.
            ..SpecSong::default()
        });
    }

    // Numbered after the pairs. A text file that is not an UltraStar file is neither listed nor
    // rejected, because a song folder holds readmes too.
    for (path, refusal) in refused {
        rejected.push((path, Rejection::UltraStar(refusal.to_string())));
    }
    for source in &sung {
        if options.limit.is_some_and(|limit| songs.len() >= limit) {
            break;
        }
        let over = options
            .index
            .get(&crate::index_key(dir, &source.text))
            .cloned()
            .unwrap_or_default();
        let Some(number) = claim(over.number, &mut taken, &mut next_number) else {
            rejected.push((source.text.clone(), Rejection::NoNumber));
            continue;
        };
        songs.push(SpecSong {
            file: relative(dir, &source.text),
            number: Some(number),
            title: over.title,
            artist: over.artist,
            language: over.language.map(|language| language.code().to_owned()),
            ..SpecSong::default()
        });
    }

    // **A hard error rather than a short description.** Running out of numbers is reported per song
    // as `Rejection::NoNumber` above, which is right for a folder that overflows by a handful — but
    // pointing this at a corpus produces thousands of them and a package that is silently the first
    // 999 files in sorted order. A curated volume is a thing somebody chose; refusing to describe
    // the folder at all is what says so.
    let overflowed = rejected
        .iter()
        .filter(|(_, why)| matches!(why, Rejection::NoNumber))
        .count();
    // **The folder being too big, not the numbering running out.** A high `start_number` with a
    // handful of files legitimately exhausts the range and is reported per song, as it always was;
    // what is refused here is a folder that could never be one package.
    if songs.len() + overflowed > usize::from(km_songcode::MAX_SLOT) {
        anyhow::bail!(
            "a package holds at most {} songs; this folder yields {} — split it, or select with \
             --min-suitability, --require-lyrics or --limit",
            km_songcode::MAX_SLOT,
            songs.len() + overflowed
        );
    }

    Ok(Description {
        spec: Spec {
            package: SpecPackage {
                id: options.id.clone(),
                name: options.name.clone(),
                version: options.version.clone(),
                publisher: options.publisher.clone(),
                created: None,
                volume: None,
                default_language: options.default_language.clone(),
                encoding: options.encoding.clone(),
                start_number: options.start_number.max(1),
                transcode: options.transcode,
                out: options.out.clone(),
            },
            root: None,
            songs,
        },
        rejected,
        orphans,
    })
}

/// The number a song gets: the one the index claimed, or the next free one.
///
/// `None` means there is no number to be had — the index claimed zero or something above
/// [`km_songcode::MAX_SLOT`], or the folder has run past the last number a keypad can dial. The
/// caller turns that into a [`Rejection::NoNumber`], so a folder too large to number tells you
/// which songs it could not place rather than producing a package nobody can ask for.
fn claim(claimed: Option<u32>, taken: &mut BTreeSet<u32>, next: &mut u32) -> Option<u32> {
    if let Some(number) = claimed {
        return (number != 0 && number <= u32::from(km_songcode::MAX_SLOT)).then(|| {
            taken.insert(number);
            number
        });
    }
    while taken.contains(next) && *next <= u32::from(km_songcode::MAX_SLOT) {
        *next += 1;
    }
    // Also the overflow guard the unbounded walk never had: `*next += 1` at `u32::MAX` panics in
    // debug, and the limit is reached six thousand times sooner than that anyway.
    if *next > u32::from(km_songcode::MAX_SLOT) {
        return None;
    }
    let number = *next;
    taken.insert(number);
    *next += 1;
    Some(number)
}

/// A path relative to the folder being described, with forward slashes.
///
/// Forward slashes, through [`crate::spec::slashed`] — which replaces the platform's separator and
/// not a literal backslash, because on Unix a backslash is part of a file's name.
fn relative(dir: &Path, path: &Path) -> String {
    crate::spec::slashed(&path.strip_prefix(dir).unwrap_or(path).to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-pack-describe-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }

        fn write(&self, name: &str, bytes: Vec<u8>) {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(path, bytes).expect("fixture");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn describe_it(dir: &Path, options: &DescribeOptions) -> Description {
        describe(dir, options, |_, _| {}).expect("describe")
    }

    /// The detected title, artist and language land in the description, and the paths are portable.
    #[test]
    fn a_description_says_what_the_files_say() {
        let scratch = Scratch::new("detected");
        scratch.write("sub/a.kar", km_song::testing::soft_karaoke());

        let found = describe_it(&scratch.0, &DescribeOptions::default());
        assert_eq!(found.spec.songs.len(), 1);
        let song = &found.spec.songs[0];
        assert_eq!(song.file, "sub/a.kar", "forward slashes, on every platform");
        assert_eq!(song.number, Some(1));
        assert!(
            song.title.is_some(),
            "the title is worth reading in the file"
        );
        assert_eq!(
            song.language.as_deref(),
            Some("en"),
            "the fixture's own @LENGL header"
        );
        found.spec.validate().expect("valid");
    }

    /// Two copies of one recording become one row, and the copy is reported.
    #[test]
    fn a_duplicate_is_left_out_and_named() {
        let scratch = Scratch::new("dupes");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        scratch.write("b.kar", km_song::testing::soft_karaoke());

        let found = describe_it(&scratch.0, &DescribeOptions::default());
        assert_eq!(found.spec.songs.len(), 1);
        assert_eq!(found.rejected.len(), 1);
        assert!(matches!(found.rejected[0].1, Rejection::DuplicateOf(1)));
    }

    /// A file that is not MIDI at all is reported rather than passed over.
    #[test]
    fn a_file_that_will_not_parse_is_reported() {
        let scratch = Scratch::new("garbage");
        scratch.write("broken.mid", b"not a midi file at all".to_vec());

        let found = describe_it(&scratch.0, &DescribeOptions::default());
        assert!(found.spec.songs.is_empty());
        assert!(matches!(found.rejected[0].1, Rejection::NotMidi));
    }

    /// `--require-lyrics` and `--min-suitability` decide what is selected, which is what a description
    /// is.
    #[test]
    fn the_filters_decide_what_gets_a_row() {
        let scratch = Scratch::new("filters");
        scratch.write("silent.mid", km_song::testing::instrumental());

        let found = describe_it(
            &scratch.0,
            &DescribeOptions {
                require_lyrics: true,
                ..DescribeOptions::default()
            },
        );
        assert!(found.spec.songs.is_empty());
        assert!(matches!(found.rejected[0].1, Rejection::NoLyrics));

        let found = describe_it(
            &scratch.0,
            &DescribeOptions {
                min_suitability: Some(11),
                ..DescribeOptions::default()
            },
        );
        assert!(matches!(found.rejected[0].1, Rejection::LowSuitability(_)));
    }

    /// Numbering starts where it is told and never reuses what the index claimed.
    #[test]
    fn numbers_start_where_they_are_told_and_avoid_the_index() {
        let scratch = Scratch::new("numbering");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        scratch.write("b.mid", km_song::testing::lyric_events());

        let mut index = BTreeMap::new();
        index.insert(
            "b.mid".to_owned(),
            Override {
                number: Some(900),
                ..Override::default()
            },
        );
        let found = describe_it(
            &scratch.0,
            &DescribeOptions {
                start_number: 900,
                index,
                ..DescribeOptions::default()
            },
        );

        let mut numbers: Vec<u32> = found.spec.songs.iter().filter_map(|s| s.number).collect();
        numbers.sort_unstable();
        assert_eq!(numbers, vec![900, 901], "the claimed number was kept");
        assert_eq!(
            found
                .spec
                .songs
                .iter()
                .find(|s| s.file == "b.mid")
                .and_then(|s| s.number),
            Some(900),
            "and kept by the song that claimed it"
        );
    }

    /// A folder that runs past the last dialable number says which songs it could not place.
    #[test]
    fn numbering_stops_at_the_highest_number_rather_than_running_past_it() {
        let scratch = Scratch::new("numbering-ceiling");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        scratch.write("b.mid", km_song::testing::lyric_events());

        let found = describe_it(
            &scratch.0,
            &DescribeOptions {
                start_number: u32::from(km_songcode::MAX_SLOT),
                ..DescribeOptions::default()
            },
        );

        let numbers: Vec<u32> = found.spec.songs.iter().filter_map(|s| s.number).collect();
        assert_eq!(
            numbers,
            vec![u32::from(km_songcode::MAX_SLOT)],
            "one number was left"
        );
        assert_eq!(found.rejected.len(), 1, "{:?}", found.rejected);
        assert!(matches!(found.rejected[0].1, Rejection::NoNumber));
    }

    /// A number the index claims that no keypad could dial is refused, as a claimed 0 already was.
    #[test]
    fn an_index_cannot_claim_a_number_above_the_highest() {
        let scratch = Scratch::new("numbering-claimed");
        scratch.write("a.kar", km_song::testing::soft_karaoke());

        let mut index = BTreeMap::new();
        index.insert(
            "a.kar".to_owned(),
            Override {
                number: Some(u32::from(km_songcode::MAX_SLOT) + 1),
                ..Override::default()
            },
        );
        let found = describe_it(
            &scratch.0,
            &DescribeOptions {
                index,
                ..DescribeOptions::default()
            },
        );

        assert!(found.spec.songs.is_empty());
        assert!(matches!(found.rejected[0].1, Rejection::NoNumber));
    }

    /// A description carries the package block through, so writing it needs nothing else.
    #[test]
    fn the_package_block_comes_from_the_options() {
        let scratch = Scratch::new("package");
        let found = describe_it(
            &scratch.0,
            &DescribeOptions {
                id: km_kmpkg::EXAMPLE_ID.to_owned(),
                name: "Volume One".to_owned(),
                default_language: Some("pt".to_owned()),
                out: Some("vol1.kmpkg".to_owned()),
                transcode: false,
                ..DescribeOptions::default()
            },
        );
        assert_eq!(found.spec.package.id, km_kmpkg::EXAMPLE_ID);
        assert_eq!(found.spec.package.default_language.as_deref(), Some("pt"));
        assert_eq!(found.spec.package.out.as_deref(), Some("vol1.kmpkg"));
        assert!(!found.spec.package.transcode);
    }
}
