//! Tests for a zip holding one song.
//!
//! The audio is a real MP3, because the claim under test is that a zipped song builds the package
//! its loose files build, and that is a claim about decoding and hashing as much as about names.

use std::io::Write;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use super::*;
use crate::spec::SpecPackage;
use km_testkit::Scratch;

pub(crate) const TONE: &[u8] =
    include_bytes!("../../../../crates/playback/km-cdg/fixtures/tone.mp3");

const ULTRASTAR: &[u8] =
    b"#TITLE:Song\n#MP3:song.mp3\n#BPM:300\n: 0 4 0 Hel\n: 4 2 0 lo\n- 8\n: 10 6 0 world\nE\n";

const LRC: &[u8] = b"[ti:Song]\n[ar:Someone]\n[00:01.00]First line\n[00:02.00]Second line\n";

/// A CD+G stream that draws one tile, which is what makes it a song's words.
pub(crate) fn graphics() -> Vec<u8> {
    let mut packet = [0u8; 24];
    packet[0] = 9;
    packet[1] = 6;
    packet[4..8].copy_from_slice(&[0, 1, 2, 3]);
    packet[8..20].fill(0x3F);
    let mut stream = packet.to_vec();
    stream.resize(24 * 300, 0);
    stream
}

/// The entries of a zip to write: each one's name and its bytes.
type Entries<'a> = &'a [(&'a str, &'a [u8])];

/// Writes a zip of `entries` into `dir`.
fn zip_of(dir: &Scratch, name: &str, entries: Entries<'_>) -> PathBuf {
    let path = dir.join(name);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).expect("create a zip"));
    let options = zip::write::SimpleFileOptions::default();
    for (entry, bytes) in entries {
        zip.start_file(*entry, options).expect("start an entry");
        zip.write_all(bytes).expect("write an entry");
    }
    zip.finish().expect("finish the zip");
    path
}

fn found_from(verdict: &ZipVerdict) -> Option<&str> {
    match verdict {
        ZipVerdict::Song(song) => Some(song.found_from().name.as_str()),
        _ => None,
    }
}

fn kind(verdict: &ZipVerdict) -> Option<ZipKind> {
    match verdict {
        ZipVerdict::Song(song) => Some(song.kind()),
        _ => None,
    }
}

/// Builds a package of the one song `file` names, and returns what the build said.
fn built(base: &Path, file: &str, out: &Path) -> BuildOutcome {
    built_from(
        base,
        SpecSong {
            file: file.to_owned(),
            ..SpecSong::default()
        },
        out,
    )
}

/// Builds a package of one song as a description names it.
fn built_from(base: &Path, song: SpecSong, out: &Path) -> BuildOutcome {
    let spec = Spec {
        package: SpecPackage {
            id: km_kmpkg::EXAMPLE_ID.to_owned(),
            name: "Volume One".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            created: None,
            volume: None,
            default_language: Some("en".to_owned()),
            encoding: None,
            start_number: 1,
            transcode: true,
            out: None,
            uncurated: false,
            linked: false,
        },
        root: None,
        songs: vec![song],
    };
    build::build(
        &spec,
        &BuildOptions {
            base,
            out: Some(out),
            dry_run: false,
            measure_loudness: true,
            write_listing: false,
            flags: km_kmpkg::PackageFlags::NONE,
            skip_duplicates: false,
        },
        |_| ControlFlow::Continue(()),
    )
    .expect("build")
}

fn songs_of(outcome: &BuildOutcome) -> Vec<km_kmpkg::SongEntry> {
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    outcome.manifest.as_ref().expect("a manifest").songs.clone()
}

#[test]
fn a_zipped_pair_builds_the_package_its_loose_files_build() {
    let dir = Scratch::new("zip-cdg-parity");
    let cdg = graphics();
    dir.write("loose/Someone - Song.mp3", TONE);
    dir.write("loose/Someone - Song.cdg", &cdg);
    std::fs::create_dir_all(dir.join("zipped")).expect("a folder");
    zip_of(
        &dir,
        "zipped/anything.zip",
        &[("Someone - Song.mp3", TONE), ("Someone - Song.cdg", &cdg)],
    );

    let loose = built(
        &dir.join("loose"),
        "Someone - Song.mp3",
        &dir.join("loose.kmpkg"),
    );
    let zipped = built(
        &dir.join("zipped"),
        "anything.zip",
        &dir.join("zipped.kmpkg"),
    );

    assert_eq!(zipped.cdg_written, 1);
    assert_eq!(songs_of(&zipped), songs_of(&loose));
    assert_eq!(
        songs_of(&zipped)[0].content_hash.as_deref(),
        Some(km_kmpkg::pair_content_hash(TONE, &cdg).as_str())
    );

    let package = km_kmpkg::Package::open(dir.join("zipped.kmpkg")).expect("open");
    assert!(package.missing_entries().expect("check").is_empty());
    assert_eq!(package.graphics_bytes(1).expect("graphics"), cdg);
    let mut audio = Vec::new();
    std::io::Read::read_to_end(&mut package.media_reader(1).expect("audio"), &mut audio)
        .expect("read");
    assert_eq!(audio, TONE);
}

#[test]
fn a_zipped_ultrastar_song_builds_the_package_its_loose_files_build() {
    let dir = Scratch::new("zip-ultrastar-parity");
    dir.write("loose/Someone - Song.txt", ULTRASTAR);
    dir.write("loose/SONG.MP3", TONE);
    std::fs::create_dir_all(dir.join("zipped")).expect("a folder");
    zip_of(
        &dir,
        "zipped/song.zip",
        &[
            ("album/Someone - Song.txt", ULTRASTAR),
            ("album/SONG.MP3", TONE),
        ],
    );

    let loose = built(
        &dir.join("loose"),
        "Someone - Song.txt",
        &dir.join("loose.kmpkg"),
    );
    let zipped = built(&dir.join("zipped"), "song.zip", &dir.join("zipped.kmpkg"));

    assert_eq!(zipped.ultrastar_written, 1);
    assert_eq!(songs_of(&zipped), songs_of(&loose));
    assert_eq!(
        songs_of(&zipped)[0].content_hash.as_deref(),
        Some(km_kmpkg::pair_content_hash(TONE, ULTRASTAR).as_str())
    );
}

#[test]
fn a_zipped_lrc_song_builds_the_package_its_loose_files_build() {
    let dir = Scratch::new("zip-lrc-parity");
    dir.write("loose/Someone - Song.lrc", LRC);
    dir.write("loose/Someone - Song.mp3", TONE);
    std::fs::create_dir_all(dir.join("zipped")).expect("a folder");
    zip_of(
        &dir,
        "zipped/song.zip",
        &[("Someone - Song.lrc", LRC), ("Someone - Song.mp3", TONE)],
    );

    let loose = built(
        &dir.join("loose"),
        "Someone - Song.lrc",
        &dir.join("loose.kmpkg"),
    );
    let zipped = built(&dir.join("zipped"), "song.zip", &dir.join("zipped.kmpkg"));

    assert_eq!(zipped.lrc_written, 1);
    assert_eq!(songs_of(&zipped), songs_of(&loose));
}

#[test]
fn what_an_operating_system_adds_to_a_zip_is_not_part_of_the_song() {
    assert!(is_os_junk("__MACOSX/._song.mp3"));
    assert!(is_os_junk("album/__MACOSX/song.mp3"));
    assert!(is_os_junk("album/._song.cdg"));
    assert!(is_os_junk(".DS_Store"));
    assert!(is_os_junk("album/Thumbs.db"));
    assert!(is_os_junk("album/DESKTOP.INI"));
    assert!(!is_os_junk("album/song.mp3"));
    assert!(!is_os_junk("MACOSX/song.mp3"));

    // The `._` file beside an MP3 has the MP3's extension, and would read as a second song.
    let dir = Scratch::new("zip-junk");
    let zip = zip_of(
        &dir,
        "song.zip",
        &[
            ("song.mp3", TONE),
            ("song.cdg", &graphics()),
            ("__MACOSX/._song.mp3", b"resource fork"),
            ("__MACOSX/._song.cdg", b"resource fork"),
            ("._song.mp3", b"resource fork"),
            (".DS_Store", b"folder view"),
            ("Thumbs.db", b"thumbnails"),
            ("desktop.ini", b"[.ShellClassInfo]"),
        ],
    );
    let verdict = read_zip(&zip);
    assert_eq!(kind(&verdict), Some(ZipKind::Cdg), "{verdict:?}");
    assert_eq!(found_from(&verdict), Some("song.mp3"));
}

#[test]
fn a_zip_without_both_files_of_a_song_is_not_a_song() {
    let dir = Scratch::new("zip-not-a-song");
    let cdg = graphics();
    let cases: [(&str, Entries<'_>); 8] = [
        ("audio-only.zip", &[("song.mp3", TONE)]),
        ("graphics-only.zip", &[("song.cdg", &cdg)]),
        ("words-only.zip", &[("song.txt", ULTRASTAR)]),
        ("lrc-only.zip", &[("song.lrc", LRC)]),
        ("other-stem.zip", &[("one.mp3", TONE), ("two.cdg", &cdg)]),
        (
            "two-folders.zip",
            &[("a/song.mp3", TONE), ("b/song.cdg", &cdg)],
        ),
        (
            "midi-and-video.zip",
            &[("song.mid", b"MThd"), ("song.mp4", b"video")],
        ),
        (
            "readme.zip",
            &[
                ("song.mp3", TONE),
                ("readme.txt", b"thank you for downloading"),
            ],
        ),
    ];
    for (name, entries) in cases {
        let verdict = read_zip(&zip_of(&dir, name, entries));
        assert!(
            matches!(verdict, ZipVerdict::NotASong),
            "{name}: {verdict:?}"
        );
    }
}

#[test]
fn a_zip_inside_a_zip_is_not_opened() {
    let dir = Scratch::new("zip-nested");
    let inner = zip_of(
        &dir,
        "inner.zip",
        &[("song.mp3", TONE), ("song.cdg", &graphics())],
    );
    let inner = std::fs::read(inner).expect("the inner zip");
    let outer = zip_of(&dir, "outer.zip", &[("inner.zip", &inner)]);
    assert!(matches!(read_zip(&outer), ZipVerdict::NotASong));
}

#[test]
fn a_zip_holding_two_songs_is_refused() {
    let dir = Scratch::new("zip-several");
    let cdg = graphics();
    let zip = zip_of(
        &dir,
        "two.zip",
        &[
            ("a.mp3", TONE),
            ("a.cdg", &cdg),
            ("more/b.lrc", LRC),
            ("more/b.mp3", TONE),
        ],
    );
    let verdict = read_zip(&zip);
    let ZipVerdict::SeveralSongs(names) = &verdict else {
        panic!("{verdict:?}");
    };
    assert_eq!(names, &["a.mp3", "more/b.lrc"]);
    assert!(verdict.refusal().expect("a reason").contains("2 songs"));

    let outcome = built(&dir, "two.zip", &dir.join("two.kmpkg"));
    assert_eq!(outcome.skipped.len(), 1);
    assert!(
        outcome.skipped[0].why.contains("2 songs"),
        "{:?}",
        outcome.skipped
    );
}

#[test]
fn a_pair_in_a_zip_is_found_as_a_pair_on_disk_is() {
    let dir = Scratch::new("zip-tolerant");
    let cdg = graphics();
    for (name, entries) in [
        (
            "case.zip",
            [("loud.MP3", TONE), ("loud.cdg", cdg.as_slice())],
        ),
        (
            "space.zip",
            [("hurricane.mp3", TONE), ("hurricane .CDG", &cdg)],
        ),
        (
            "backslash.zip",
            [("album\\song.mp3", TONE), ("album\\song.cdg", &cdg)],
        ),
    ] {
        let verdict = read_zip(&zip_of(&dir, name, &entries));
        assert_eq!(kind(&verdict), Some(ZipKind::Cdg), "{name}: {verdict:?}");
    }
}

#[test]
fn an_mp3_in_a_zip_is_one_song_and_the_pair_comes_first() {
    let dir = Scratch::new("zip-precedence");
    let cdg = graphics();

    let pair_and_words = zip_of(
        &dir,
        "pair-and-words.zip",
        &[
            ("song.mp3", TONE),
            ("song.cdg", &cdg),
            ("song.txt", ULTRASTAR),
            ("song.lrc", LRC),
        ],
    );
    assert_eq!(kind(&read_zip(&pair_and_words)), Some(ZipKind::Cdg));

    let both_words = zip_of(
        &dir,
        "both-words.zip",
        &[
            ("song.mp3", TONE),
            ("song.txt", ULTRASTAR),
            ("song.lrc", LRC),
        ],
    );
    assert_eq!(kind(&read_zip(&both_words)), Some(ZipKind::UltraStar));
}

#[test]
fn a_song_that_cannot_be_packaged_matters_only_when_it_is_the_one_song() {
    let dir = Scratch::new("zip-refused");
    let duet = b"#TITLE:Song\n#MP3:song.mp3\n#BPM:300\nP1\n: 0 4 0 Hel\nP2\n: 4 2 0 lo\nE\n";
    assert!(
        km_song::ultrastar::parse(duet).is_err(),
        "the fixture has to be a file this project refuses"
    );

    let alone = zip_of(&dir, "alone.zip", &[("duet.txt", duet), ("song.mp3", TONE)]);
    let verdict = read_zip(&alone);
    let ZipVerdict::Refused(reason) = &verdict else {
        panic!("{verdict:?}");
    };
    assert!(reason.starts_with("duet.txt: "), "{reason}");

    let beside = zip_of(
        &dir,
        "beside.zip",
        &[
            ("duet.txt", duet),
            ("song.txt", ULTRASTAR),
            ("song.mp3", TONE),
        ],
    );
    assert_eq!(found_from(&read_zip(&beside)), Some("song.txt"));
}

#[test]
fn a_zip_that_cannot_be_read_as_one_song_is_refused() {
    let dir = Scratch::new("zip-unusable");

    let not_a_zip = dir.write("broken.zip", b"this is not a zip");
    assert!(matches!(read_zip(&not_a_zip), ZipVerdict::Refused(_)));

    let many: Vec<String> = (0..=media::MAX_ZIP_ENTRIES)
        .map(|index| format!("{index}.bin"))
        .collect();
    let entries: Vec<(&str, &[u8])> = many.iter().map(|name| (name.as_str(), &b""[..])).collect();
    let verdict = read_zip(&zip_of(&dir, "collection.zip", &entries));
    let ZipVerdict::Refused(reason) = &verdict else {
        panic!("{verdict:?}");
    };
    assert!(reason.contains("collection"), "{reason}");
}

#[test]
fn a_song_behind_a_password_is_refused() {
    let dir = Scratch::new("zip-password");
    let path = dir.join("locked.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).expect("create a zip"));
    let locked = zip::write::SimpleFileOptions::default()
        .with_aes_encryption(zip::AesMode::Aes128, "password");
    for (entry, bytes) in [("song.mp3", TONE), ("song.cdg", graphics().as_slice())] {
        zip.start_file(entry, locked).expect("start an entry");
        zip.write_all(bytes).expect("write an entry");
    }
    zip.finish().expect("finish the zip");

    let verdict = read_zip(&path);
    let ZipVerdict::Refused(reason) = &verdict else {
        panic!("{verdict:?}");
    };
    assert!(reason.contains("password"), "{reason}");
}

#[test]
fn an_entry_larger_than_its_limit_is_not_held() {
    let dir = Scratch::new("zip-limit");
    let zip = zip_of(&dir, "song.zip", &[("song.mp3", TONE)]);
    let member = ZipMember {
        archive: zip,
        index: 0,
        name: "song.mp3".to_owned(),
        size: TONE.len() as u64,
    };
    assert_eq!(member.read(TONE.len() as u64).expect("within"), TONE);
    assert!(member.read(TONE.len() as u64 - 1).is_err());

    // An entry that is not the one the listing named is a zip that changed underneath the read.
    let renamed = ZipMember {
        name: "other.mp3".to_owned(),
        ..member
    };
    assert!(renamed.read(u64::MAX - 1).is_err());
}

#[test]
fn a_folder_is_described_with_its_zipped_songs_numbered_last() {
    let dir = Scratch::new("zip-describe");
    let cdg = graphics();
    dir.write("b.lrc", LRC);
    dir.write("b.mp3", TONE);
    zip_of(&dir, "a.zip", &[("a.mp3", TONE), ("a.cdg", &cdg)]);
    zip_of(&dir, "pictures.zip", &[("one.png", b"png")]);
    zip_of(
        &dir,
        "two.zip",
        &[
            ("a.mp3", TONE),
            ("a.cdg", &cdg),
            ("b.mp3", TONE),
            ("b.cdg", &cdg),
        ],
    );

    let described = describe(&dir, &DescribeOptions::default(), |_, _| {}).expect("describe");
    let files: Vec<&str> = described
        .spec
        .songs
        .iter()
        .map(|song| song.file.as_str())
        .collect();
    assert_eq!(files, ["b.lrc", "a.zip"]);

    let rejected: Vec<_> = described
        .rejected
        .iter()
        .filter(|(_, why)| matches!(why, Rejection::Zip(_)))
        .map(|(path, _)| file_stem(path))
        .collect();
    assert_eq!(rejected, ["two"], "{:?}", described.rejected);

    let mut walked = Vec::new();
    collect_songs(&dir, &mut walked);
    assert!(walked.iter().any(|path| is_zip_candidate(path)));
}

#[test]
fn a_zipped_song_takes_no_named_graphics_file() {
    let dir = Scratch::new("zip-graphics-name");
    zip_of(
        &dir,
        "song.zip",
        &[("song.mp3", TONE), ("song.cdg", &graphics())],
    );
    let outcome = built_from(
        &dir,
        SpecSong {
            file: "song.zip".to_owned(),
            graphics: Some("other.cdg".to_owned()),
            ..SpecSong::default()
        },
        &dir.join("song.kmpkg"),
    );
    assert_eq!(outcome.skipped.len(), 1);
    assert!(
        outcome.skipped[0].why.contains("named graphics"),
        "{:?}",
        outcome.skipped
    );
}
