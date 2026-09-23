//! Turning the pinned Open Hymnal into the Christmas carol pack.
//!
//! The pack is a **separate download** and is never bundled with the machine: nothing here writes
//! into `assets/`, and no carrier changes because of it. See the `A downloadable song pack`
//! decision in `docs/decisions/`.
//!
//! # What this does, and what it deliberately does not
//!
//! It produces `.kar` files and the `.kmspec.yaml` that describes them. It **does not build the
//! package** — `km-pack build` does, from that description, exactly as it would for a package
//! somebody wrote by hand. That is the same rule `km-package-builder` follows and for the same
//! reason: two tools writing subtly different manifests from the same songs is a defect nobody
//! notices until a package behaves oddly on the machine.
//!
//! # The pipeline
//!
//! 1. [`abc::split`] the hymnal into tunes and take the sixteen [`selection::CAROLS`] names.
//! 2. [`license::assess`] each one's own copyright line, and **fail closed**.
//! 3. [`abc::Tune::expand`] it, so each verse gets its own pass of the music.
//! 4. Shell out to `abc2midi`, which writes Soft Karaoke natively.
//! 5. [`midi::rewrite`] the result: name the melody track, write the `@T`/`@L` headers, drop
//!    abc2midi's own labels out of the lyric stream.
//! 6. **Check the file with the machine's own parser and scorer** before writing it.
//!
//! Step 6 is why this is a Rust program and not a shell script. The tool that makes the file is the
//! one that refuses it: a carol that will not parse, or whose lyrics are not timed per syllable, or
//! whose syllables do not land on the notes, stops the build by name rather than reaching a package
//! and being discovered by somebody trying to sing it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use km_pack::spec::{Spec, SpecPackage, SpecSong};
use km_song::{COMFORTABLE_LINE_CHARS, LyricLine, ParseOptions, Song};
use km_suitability::Analysis;

pub mod abc;
pub mod license;
pub mod midi;
pub mod selection;

/// What a build needs to know.
pub struct Options {
    /// The pinned hymnal, already fetched and verified.
    pub source: PathBuf,
    /// Where the songs, the description and the credits are written.
    pub out: PathBuf,
    /// The `abc2midi` to run.
    pub abc2midi: PathBuf,
    /// The pack's version, which is its own and not the workspace's.
    pub version: String,
    /// Keep the generated ABC beside each song, for looking at when something sounds wrong.
    pub keep_abc: bool,
}

/// One carol, once it has been made and checked.
pub struct Built {
    /// Its song number within the package.
    pub number: u32,
    /// The title as the pack files it.
    pub title: String,
    /// The artist as the pack files it.
    pub artist: String,
    /// The tune's own `C:` lines, for `CREDITS.md`.
    pub credits: Vec<String>,
    /// How many verses it carries.
    pub verses: usize,
    /// What the machine's own analysis made of it.
    pub suitability: u8,
    /// The suitability's components, so a report can say which point went missing.
    pub breakdown: km_suitability::Breakdown,
    /// How long it plays for.
    pub duration_ms: u64,
    /// How many syllables there are to sing.
    pub syllables: usize,
}

/// The lowest total this pack will carry.
///
/// Six, not nine, and the difference is one carol: *Away In A Manger* is set for two voices with
/// chords in the treble staff, so nothing in it is monophonic and no melody channel can be found
/// however the tracks are named — it loses the melody's two points and the arrangement's, and is
/// perfectly singable regardless. The components that decide whether a song can be *sung* are
/// checked exactly rather than by total; see [`check`].
const MINIMUM_SUITABILITY: u8 = 6;

/// Where the generated songs go, under `--out`.
const SONGS_SUBDIR: &str = "songs";

/// Builds the pack. Returns what was made, in song-number order.
pub fn build(options: &Options) -> Result<Vec<Built>> {
    let source = read_latin1(&options.source)
        .with_context(|| format!("reading {}", options.source.display()))?;
    let tunes = abc::split(&source);
    if tunes.is_empty() {
        bail!("{} holds no tunes", options.source.display());
    }

    let songs_dir = options.out.join(SONGS_SUBDIR);
    fs::create_dir_all(&songs_dir).with_context(|| format!("creating {}", songs_dir.display()))?;

    let mut built = Vec::with_capacity(selection::CAROLS.len());
    for (index, carol) in selection::CAROLS.iter().enumerate() {
        let number = u32::try_from(index + 1)?;
        built.push(one(options, &tunes, carol, number, &songs_dir)?);
    }

    write_spec(options, &built)?;
    write_credits(options, &built)?;
    Ok(built)
}

/// Makes one carol, or explains which rule stopped it.
fn one(
    options: &Options,
    tunes: &[abc::Tune],
    carol: &selection::Carol,
    number: u32,
    songs_dir: &Path,
) -> Result<Built> {
    let tune = tunes
        .iter()
        .find(|t| t.number == carol.number)
        .with_context(|| format!("X:{} is not in this edition", carol.number))?;

    // The number selects and the title checks. A re-pin that renumbered would stop here rather
    // than quietly putting a different carol in the pack.
    if tune.title != carol.title {
        bail!(
            "X:{} is \"{}\" in this edition, not \"{}\" -- the pinned edition has changed",
            carol.number,
            tune.title,
            carol.title
        );
    }

    match license::assess(&tune.credits) {
        license::Redistribution::PublicDomain => {}
        license::Redistribution::Restricted(why) => {
            bail!(
                "\"{}\" (X:{}) may not be passed on: {why}",
                carol.title,
                carol.number
            );
        }
    }

    let expanded = tune.expand(selection::MELODY_VOICE)?;

    let stem = file_stem(carol.title);
    let abc_path = songs_dir.join(format!("{stem}.abc"));
    let mid_path = songs_dir.join(format!("{stem}.abc.mid"));
    fs::write(&abc_path, &expanded.abc)
        .with_context(|| format!("writing {}", abc_path.display()))?;

    run_abc2midi(&options.abc2midi, &abc_path, &mid_path)?;
    let raw = fs::read(&mid_path).with_context(|| format!("reading {}", mid_path.display()))?;
    let headers = midi::Headers {
        title: carol.title.to_owned(),
        artist: carol.artist.to_owned(),
        language: selection::LANGUAGE_TAG.to_owned(),
    };
    let bytes =
        midi::rewrite(&raw, &headers).with_context(|| format!("rewriting {}", carol.title))?;

    let kar_path = songs_dir.join(format!("{stem}.kar"));
    fs::write(&kar_path, &bytes).with_context(|| format!("writing {}", kar_path.display()))?;

    let _ = fs::remove_file(&mid_path);
    if !options.keep_abc {
        let _ = fs::remove_file(&abc_path);
    }

    let checked = check(&bytes, carol)?;
    Ok(Built {
        number,
        title: carol.title.to_owned(),
        artist: carol.artist.to_owned(),
        credits: tune.credits.clone(),
        verses: expanded.verses,
        suitability: checked.suitability,
        breakdown: checked.breakdown,
        duration_ms: checked.duration_ms,
        syllables: checked.syllables,
    })
}

/// What the machine's own code makes of a generated file.
struct Checked {
    suitability: u8,
    breakdown: km_suitability::Breakdown,
    duration_ms: u64,
    syllables: usize,
}

/// Refuses a file the machine could not sing from.
///
/// **The two components checked exactly are the two that decide whether a song works**: `lyrics`
/// must be 3, meaning there are enough of them and they are timed per syllable rather than per
/// line, and `sync` must be 3, meaning those timings land on the notes. Melody and arrangement are
/// worth points and are not worth failing a build over — a carol with no drum channel is a carol,
/// and one whose setting is too chordal for a melody channel to be found is still singable.
///
/// **How long it is sung for is checked ahead of both**, because a carol too short to be worth
/// choosing scores zero on the pair and neither message would say why.
fn check(bytes: &[u8], carol: &selection::Carol) -> Result<Checked> {
    let song = Song::parse(bytes, &ParseOptions::default())
        .with_context(|| format!("\"{}\" did not parse as a karaoke file", carol.title))?;
    let analysis = Analysis::of(&song);
    let breakdown = analysis.suitability.breakdown;

    // Before the two below, because a carol sung for under three quarters of a minute scores zero on
    // both of them and neither message names the cause: a one-verse conversion is timed per syllable
    // and lands on every note, and is simply over. Verse expansion is what answers it.
    let sung_ms = km_suitability::sung_span_ms(&song);
    let least = km_suitability::Thresholds::default().min_sung_ms;
    if sung_ms < least {
        bail!(
            "\"{}\" is sung for {}.{:01}s, under the {}s a song needs to be worth choosing: it \
             wants more verses",
            carol.title,
            sung_ms / 1_000,
            (sung_ms % 1_000) / 100,
            least / 1_000,
        );
    }
    if breakdown.lyrics < 3 {
        bail!(
            "\"{}\" scored {}/3 on lyrics: they are not timed per syllable, or there are too few",
            carol.title,
            breakdown.lyrics
        );
    }
    if breakdown.sync < 3 {
        bail!(
            "\"{}\" scored {}/3 on sync: the syllables do not land on the notes",
            carol.title,
            breakdown.sync
        );
    }
    // A hymnal's `w:` line is a whole musical system, so the naive conversion produced lines more
    // than twice as long as anything real karaoke files contain. `midi::rewrap_lines` subdivides
    // them; this is what says so afterwards, on the parsed file rather than on our own bookkeeping.
    // A single word longer than the budget is exempt, because no break rule can help it.
    if let Some(line) = song.lyrics.lines.iter().map(LyricLine::text).find(|text| {
        text.chars().count() > COMFORTABLE_LINE_CHARS && text.split_whitespace().count() > 1
    }) {
        bail!(
            "\"{}\" has a {}-character line, past the {COMFORTABLE_LINE_CHARS} that read comfortably: {line:?}",
            carol.title,
            line.chars().count(),
        );
    }
    let suitability = analysis.suitability_value();
    if suitability < MINIMUM_SUITABILITY {
        bail!(
            "\"{}\" scored {suitability}/10, below the pack's floor of {MINIMUM_SUITABILITY}",
            carol.title
        );
    }

    Ok(Checked {
        suitability,
        breakdown,
        duration_ms: u64::from(song.duration_ms()),
        syllables: song.lyrics.lines.iter().map(|l| l.syllables.len()).sum(),
    })
}

/// Runs abc2midi over one tune.
///
/// `-STFW` puts the words in a track of their own, which is what makes the lyric track findable by
/// name in [`midi::rewrite`]. `-NCOM` drops the bar-number comments, which would otherwise arrive
/// as text events among the syllables.
///
/// **Warnings are not failures and errors are.** The hymnal's own notation raises a great many
/// benign warnings — pickup bars at the verse seams, and score decorations like `!sintro!` that
/// mean nothing to a MIDI file — while `Error` means a note was dropped.
fn run_abc2midi(abc2midi: &Path, input: &Path, output: &Path) -> Result<()> {
    let result = Command::new(abc2midi)
        .arg(input)
        .args(["-NCOM", "-STFW", "-o"])
        .arg(output)
        .output()
        .with_context(|| format!("running {}", abc2midi.display()))?;

    let out = String::from_utf8_lossy(&result.stdout);
    let err = String::from_utf8_lossy(&result.stderr);
    let errors: Vec<&str> = out
        .lines()
        .chain(err.lines())
        .filter(|line| line.starts_with("Error"))
        .collect();
    if !errors.is_empty() {
        bail!(
            "abc2midi dropped music from {}:\n  {}",
            input.display(),
            errors.join("\n  ")
        );
    }
    if !output.exists() {
        bail!("abc2midi wrote no file for {}", input.display());
    }
    Ok(())
}

/// Writes the description `km-pack build` takes.
fn write_spec(options: &Options, built: &[Built]) -> Result<()> {
    let spec = Spec {
        package: SpecPackage {
            id: selection::PACKAGE_ID.to_owned(),
            name: selection::PACKAGE_NAME.to_owned(),
            version: options.version.clone(),
            publisher: Some(selection::PACKAGE_PUBLISHER.to_owned()),
            // Left to `km-pack build` to stamp, so this description is the same bytes on every run.
            created: None,
            volume: None,
            // Every song names its own language, so there is nothing for a default to fill.
            default_language: None,
            encoding: None,
            start_number: 1,
            // There is no video here and never will be, so nothing to re-encode.
            transcode: false,
            out: Some(format!("{}.kmpkg", selection::PACKAGE_ID)),
            uncurated: false,
        },
        // No `root:`, deliberately. `out:` resolves against the description's base, and the base
        // is `root:` where there is one -- so a root of `songs` would put the built package inside
        // the songs folder. Each song names its folder instead, which `km-pack` slash-separates.
        root: None,
        songs: built
            .iter()
            .map(|b| SpecSong {
                file: format!("{}/{}.kar", SONGS_SUBDIR, file_stem(&b.title)),
                number: Some(b.number),
                title: Some(b.title.clone()),
                artist: Some(b.artist.clone()),
                language: Some(selection::LANGUAGE.to_owned()),
                ..SpecSong::default()
            })
            .collect(),
    };
    spec.validate()?;
    let path = options
        .out
        .join(format!("{}.kmspec.yaml", selection::PACKAGE_ID));
    spec.write(&path)
        .with_context(|| format!("writing {}", path.display()))
}

/// Writes the attribution that travels with the pack.
///
/// Every line here is the hymnal's own, verbatim, because a license claim paraphrased is a license
/// claim weakened. The same reasoning as `assets/wallpapers/CREDITS.md`.
fn write_credits(options: &Options, built: &[Built]) -> Result<()> {
    let mut out = String::new();
    out.push_str("# Christmas Carols — credits and copyright\n\n");
    out.push_str(
        "Every carol here is in the public domain in all four of the parts a hymn divides into —\n\
         music, setting, words and translation — and each entry below quotes the statement of that\n\
         from the source, verbatim.\n\n\
         The sequences were generated from the Open Hymnal Project's ABC Plus sources\n\
         (<http://openhymnal.org/>), whose own compilation work that project places in the public\n\
         domain. The Open Hymnal states that it applies the copyright law of the United States of\n\
         America; elsewhere, verifying a hymn is freely distributable is the reader's own business.\n\n\
         Nothing here is a recording. Each file is a MIDI sequence produced from a printed score.\n\n",
    );
    for b in built {
        out.push_str(&format!("## {}. {} — {}\n\n", b.number, b.title, b.artist));
        for line in &b.credits {
            out.push_str(&format!("- {line}\n"));
        }
        out.push('\n');
    }
    let path = options.out.join("CREDITS.md");
    fs::write(&path, out).with_context(|| format!("writing {}", path.display()))
}

/// A filename for a title: letters, digits and dashes.
fn file_stem(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Reads a file as ISO-8859-1.
///
/// The hymnal is Latin-1 — `Concordia Kinderchöre`, `Jean de Brébeuf` — and is not valid UTF-8, so
/// it cannot simply be `read_to_string`d. Every byte maps to the code point of the same value,
/// which is what ISO-8859-1 is.
fn read_latin1(path: &Path) -> Result<String> {
    Ok(fs::read(path)?.into_iter().map(char::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_becomes_a_filename() {
        assert_eq!(file_stem("Silent Night"), "silent-night");
        assert_eq!(file_stem("What Child Is This?"), "what-child-is-this");
        assert_eq!(
            file_stem("Hark! The Herald Angels Sing"),
            "hark-the-herald-angels-sing"
        );
        assert_eq!(
            file_stem("O Come, All Ye Faithful"),
            "o-come-all-ye-faithful"
        );
    }

    #[test]
    fn latin1_bytes_become_the_characters_they_stand_for() {
        let dir = std::env::temp_dir().join("km-carols-latin1-test");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        fs::write(
            &path,
            [
                b'K', b'i', b'n', b'd', b'e', b'r', b'c', b'h', 0xF6, b'r', b'e',
            ],
        )
        .unwrap();
        assert_eq!(read_latin1(&path).unwrap(), "Kinderchöre");
        let _ = fs::remove_file(&path);
    }
}
