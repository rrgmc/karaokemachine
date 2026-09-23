//! The package description: what goes into a package, written down where a person can read it.
//!
//! A `.kmpkg` is the output and the curation database is the working copy; until now there was
//! nothing in between. `km-pack build` took a *folder* and packaged whatever was in it, and the one
//! escape hatch — a `--index` CSV — could correct a title but could not say which files belonged in
//! the package at all. So nothing anywhere described a package in a form somebody could open, read,
//! correct and keep beside the songs.
//!
//! This is that form. `km-pack spec` writes one from a folder, `km-package-builder` writes one from
//! its database, and `km-pack build` builds from one and from nothing else. The same value, built in
//! memory, is what the curation tool's own build consumes — so the two tools cannot produce
//! different packages from the same songs, which a convention alone could not guarantee.
//!
//! # Provenance is derived, never stored
//!
//! There is deliberately **no `edited:` key**, and there never will be. A description carries values
//! only; the build constructs the entry a fresh parse of the file implies, and then applies these
//! values through [`crate::apply_edits`], which marks a field hand-edited **only where the two
//! differ**. Three things follow, and they are the reason this is the right shape:
//!
//! * A description written by `km-pack spec` says back what detection found, so building it marks
//!   nothing edited. Generating a description is provenance-neutral and safe to repeat.
//! * A person who corrects a title in the file gets the `edited` flag for free, with no second key to
//!   remember and keep true.
//! * `edited` becomes a *fact about a disagreement* rather than a claim somebody has to maintain. A
//!   stored list can lie; a comparison cannot.
//!
//! One consequence to know before it is mistaken for a bug: if the source file changes after a
//! description is written, the build compares a stale value against fresh detection and marks it
//! edited. That errs towards keeping what a person reviewed, which is the right way for it to err.
//!
//! # What is not in here
//!
//! No `kind:` key. Everything else in this workspace decides what a song is from its extension or
//! from probing the bytes, and a second source of truth that can disagree with the file is a bug
//! waiting to be filed. No analysis either — the suitability, the melody and the duration are
//! facts about the bytes and are re-derived at build time, so a description cannot go stale in a
//! way that changes what the machine is told about a song.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use km_kmpkg::Language;
use serde::{Deserialize, Serialize};

/// The default package version, when a description does not say.
fn default_version() -> String {
    "1.0.0".to_owned()
}

/// The default first song number.
fn first_number() -> u32 {
    1
}

/// Videos outside the packaging profile are re-encoded unless a description says otherwise.
fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands over a reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

/// A package, and the songs that go into it.
///
/// `deny_unknown_fields` throughout, and it is a deliberate trade rather than strictness for its own
/// sake. The failure it prevents is the expensive one: `langauge:` misspelled in a four-thousand-song
/// description packages four thousand songs with no language and says nothing. What it costs is that
/// a description written by a newer `km-pack` is refused by an older one — with a message naming the
/// key, which is a far better outcome than the older one quietly ignoring half of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// The package's own details.
    pub package: SpecPackage,
    /// The folder `file:` values are resolved against, itself relative to the description's folder.
    ///
    /// Absent means the description's own folder, which is what makes a description movable with the
    /// songs it describes. It earns its place for the case the generator hits constantly: a
    /// description written somewhere other than the corpus would otherwise need a `../../..` prefix
    /// on every one of four thousand rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The songs, in the order they will be added.
    #[serde(default)]
    pub songs: Vec<SpecSong>,
}

/// What the manifest's `package` block will say, plus the knobs that apply to the whole build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecPackage {
    /// Reinstalling the same identifier replaces it, so keep it stable.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Package version. A string, so `1.0` in a hand-written file is refused with a type error
    /// rather than silently becoming the float one.
    #[serde(default = "default_version")]
    pub version: String,
    /// Who made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// When it was made. Absent means the build stamps the moment it ran.
    ///
    /// Present makes a build reproducible byte for byte, which is worth having and is why this is
    /// writable at all — nothing sets it today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// Which curated set this package is a volume of. Written into the manifest as it stands; see
    /// `km_kmpkg::PackageMeta::volume`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<km_kmpkg::VolumeOf>,
    /// Files any song that names no language under this code.
    ///
    /// It fills gaps only and does **not** mark the field hand-edited, because nobody edited it. With
    /// no default set, a song that names no language refuses the build, exactly as before — so this
    /// is a statement somebody makes about a package rather than a rule being switched off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_language: Option<String>,
    /// Decode every song's lyrics with this, unless the song pins its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The first number handed to a song that names none.
    #[serde(default = "first_number")]
    pub start_number: u32,
    /// Re-encode videos that fall outside the packaging profile.
    ///
    /// On by default because the point of a profile is that the machine only ever meets one thing. A
    /// video the machine could not play at all is refused rather than stored either way.
    #[serde(default = "yes")]
    pub transcode: bool,
    /// Where the `.kmpkg` goes, relative to the description. `--out` overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// The package was described straight from a folder, and nobody has reviewed it.
    ///
    /// **`km-pack spec` writes `true`**, and the build sets the header flag
    /// [`km_kmpkg::PackageFlags::UNCURATED`] from it. A person who reviews the description deletes
    /// the line. See `An uncurated package says so everywhere but the television` in
    /// `docs/decisions/packaging.md`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub uncurated: bool,
}

/// One song: where to read it from, and what to call it.
///
/// Every field but [`Self::file`] is optional, and an absent one means *whatever the file says*. So
/// the smallest useful description is a package block and a list of paths, written somewhere it can
/// be read and corrected.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecSong {
    /// The source file, relative to [`Spec::root`]. An absolute path is honored as it stands.
    ///
    /// A `String` rather than a `PathBuf` on purpose: a `PathBuf` holding a non-UTF-8 name fails to
    /// serialize, and it fails opaquely. As a string, the generator refuses such a path **by name**
    /// on the way in — the constraint [`crate::index_key`] has always lived under for the CSV.
    pub file: String,
    /// The queueing number. Absent means the next free one from `start_number`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    /// The title. Absent means whatever the file says, falling back to its own name.
    ///
    /// **What this means depends on the kind of song, and the difference is not an oversight.** For a
    /// MIDI file it is routed through [`crate::apply_edits`] against a fresh parse, so setting it to
    /// what the file already says records no correction. For a video or an MP3+G pair it simply
    /// replaces the container tag, with no provenance recorded — a tag is not a parse, so there is
    /// nothing to have overruled. A generator must therefore write a MIDI song's *typed* title here
    /// and leave it absent when nobody typed one; writing the detected one would mark every song in
    /// the corpus hand-edited on the next build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The performer. The same rule as [`Self::title`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    /// An ISO 639-1 code, or `und`. Validated when the description is read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// What this song is filed under — `rock`, `anime`. Folded to slugs when the description is read.
    ///
    /// **Optional in a way a language is not.** A build refuses a song with no language, so
    /// [`SpecPackage::default_language`] exists to fill the gap; nothing refuses a song for having no
    /// tag, which is why there is no `default_tags` beside it. A package-wide default here would be a
    /// value nobody checked, written onto every song for no reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Decode this song's lyrics with this, whatever detection would have chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// Default transposition in semitones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transpose: Option<i8>,
    /// Whether the machine plays this song and draws none of its words.
    ///
    /// **Three states, and YAML spells all three**: the key absent takes whatever detection finds,
    /// `lyrics_hidden: true` silences the words on a song detection was content with, and
    /// `lyrics_hidden: false` draws them on a song detection would have silenced. The last is why
    /// the key being present matters at all — see `Edits::lyrics_hidden`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics_hidden: Option<bool>,
    /// The corrections in force on this song's own MIDI events.
    ///
    /// **Stating any list at all is an edit**, including an empty one, which is how a description
    /// says *leave this song alone* against a detector that would otherwise propose something. A
    /// song the description is silent about takes whatever detection finds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixes: Option<Vec<km_fixes::Fix>>,
    /// The melody channel, 0-based, where somebody disagreed with detection.
    ///
    /// **Three states, and YAML spells all three**: the key absent takes whatever detection finds,
    /// `melody: ~` says this song has no melody channel, and a number names one. The middle state is
    /// why this is an `Option<Option<u8>>` where `transpose` beside it is not — *nobody has said* and
    /// *there is none* send a singer to two different places, because the machine offers its
    /// guide-melody toggle only on a song that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub melody: Option<Option<u8>>,
    /// The `.cdg` beside an MP3, when it is not the sibling with the same stem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics: Option<String>,
}

impl Spec {
    /// Reads and validates a description.
    pub fn read(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let spec: Self =
            serde_norway::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        spec.validate()
            .with_context(|| format!("in {}", path.display()))?;
        Ok(spec)
    }

    /// Writes a description, with the header that says what it is and how to build it.
    ///
    /// The header is not decoration. This file's whole purpose is to be opened by a person, and the
    /// two YAML traps below have bitten every project that has ever shipped a hand-edited YAML
    /// document — saying so in the file is the only place the warning is read at the moment it
    /// matters.
    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let body = serde_norway::to_string(self).context("writing the description")?;
        let text = format!("{}{}", Self::HEADER, quote_norwegian(&body));
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// The comment block every written description opens with.
    const HEADER: &'static str = "\
# A karaoke package description. Build it with:
#
#     km-pack build this-file.kmspec.yaml
#
# Every song field but `file` may be left out, and leaving one out means \"whatever the file says\".
# Setting one to something the file does not say records it as a correction, so a rebuild keeps it.
#
# `uncurated: true` marks the package as described from a folder that nobody reviewed. The machine's
# lists of packages say so. Delete the line once somebody has reviewed the songs.
#
# Two values YAML can read as something other than what you meant, if you edit them by hand:
#
#   version: \"1.0\"     quote a two-part version -- bare 1.0 is a number, and this wants a string
#   language: \"no\"     quote it -- `no` is Norwegian, and bare no is false to a YAML 1.1 reader
#
";

    /// The folder [`SpecSong::file`] values resolve against.
    ///
    /// `root` is itself relative to the description's own folder, so a description that sits beside
    /// its songs moves with them and one written elsewhere names the corpus absolutely.
    pub fn base(&self, spec_path: &Path) -> PathBuf {
        let here = spec_path.parent().unwrap_or(Path::new("."));
        match self.root.as_deref().filter(|root| !root.is_empty()) {
            Some(root) => here.join(root),
            None => here.to_path_buf(),
        }
    }

    /// Where a song's bytes are, given the folder from [`Self::base`].
    pub fn source(base: &Path, song: &SpecSong) -> PathBuf {
        let file = Path::new(&song.file);
        if file.is_absolute() {
            file.to_path_buf()
        } else {
            base.join(file)
        }
    }

    /// Everything that can be checked without opening a single song.
    ///
    /// Refused here rather than discovered during the build, for the reason `km-pack build` already
    /// applied to `--default-language`: a typo should cost nothing, not surface after several
    /// thousand files have been read.
    pub fn validate(&self) -> Result<()> {
        if self.package.id.trim().is_empty() {
            bail!("the package needs an `id`");
        }
        if self.package.name.trim().is_empty() {
            bail!("the package needs a `name`");
        }
        if let Some(raw) = &self.package.default_language {
            check_language(raw).context("package.default_language")?;
        }
        if self.package.start_number == 0 {
            bail!("package.start_number: 0 is not a song number");
        }
        if self.package.start_number > u32::from(km_songcode::MAX_SLOT) {
            bail!(
                "package.start_number: {} is above the highest song number, {}",
                self.package.start_number,
                km_songcode::MAX_SLOT
            );
        }

        // Numbers are checked here and not only by the manifest, because a duplicate number is the
        // one defect in a description that a person cannot see by reading it -- the two rows are a
        // thousand lines apart. `PackageBuilder` would refuse it too, after the whole build.
        let mut seen = BTreeSet::new();
        for (index, song) in self.songs.iter().enumerate() {
            let at = || format!("song {} ({})", index + 1, song.file);
            if song.file.trim().is_empty() {
                bail!("song {} has no `file`", index + 1);
            }
            if let Some(raw) = &song.language {
                check_language(raw).with_context(at)?;
            }
            check_tags(&song.tags).with_context(at)?;
            if let Some(number) = song.number {
                if number == 0 {
                    bail!("{}: 0 is not a song number", at());
                }
                if number > u32::from(km_songcode::MAX_SLOT) {
                    bail!(
                        "{}: {number} is above the highest song number, {}; the machine's keypad \
                         takes {} digits",
                        at(),
                        km_songcode::MAX_SLOT,
                        km_songcode::MAX_DIGITS
                    );
                }
                if !seen.insert(number) {
                    bail!("{}: number {number} is claimed by another song", at());
                }
            }
        }
        Ok(())
    }
}

/// A path written the way a description spells one: with forward slashes.
///
/// A description travels — written on Windows, built on the appliance — and `Path::join` understands
/// `/` on every platform, so forward slashes are the portable spelling and the one this format uses.
///
/// **It replaces [`std::path::MAIN_SEPARATOR`] and not a literal backslash**, which is the whole
/// reason this is a function rather than a `.replace()` at each of five call sites. On Unix a
/// backslash is an ordinary character in a file name, so replacing one there would quietly corrupt
/// the path of any song whose name contains it — a fault that cannot happen on the machine this was
/// written on and would surface only on the box under the television. On Unix the separator already
/// *is* `/`, so this is a no-op there, which is exactly right.
pub fn slashed(path: &str) -> String {
    path.replace(std::path::MAIN_SEPARATOR, "/")
}

/// Puts quotes round the one language code YAML has an opinion about.
///
/// `no` is Norwegian. It is also `false` to any reader following YAML 1.1 — `yq`, PyYAML, Ruby's
/// psych — and the writer here emits it bare, because the 1.2 core schema it implements says a plain
/// `no` is a string. Our own [`Spec::read`] is therefore safe either way, and that is not enough:
/// this file exists to be opened by people and is a perfectly ordinary thing to run another tool
/// over, and a language that silently becomes `false` on the way through one is the kind of fault
/// nobody traces back to a quoting rule.
///
/// One code and not a general escaper, because one code is the whole problem. Of YAML 1.1's boolean
/// spellings — `y`, `n`, `yes`, `no`, `on`, `off`, `true`, `false` and their cases — `no` is the
/// only one that is also an ISO 639-1 language.
fn quote_norwegian(yaml: &str) -> String {
    yaml.lines()
        .map(|line| match line.strip_suffix(": no") {
            Some(head)
                if head.trim_start().starts_with("language")
                    || head.trim_start().starts_with("default_language") =>
            {
                format!("{head}: \"no\"")
            }
            _ => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Refuses anything that is not a language code, naming what was written.
///
/// The same rule [`crate::read_index`] applies to the CSV, and for the same reason: a value that is
/// not a code reaches the manifest and makes the column unfilterable for one song in a thousand,
/// which is the hardest kind of fault to notice.
fn check_language(raw: &str) -> Result<()> {
    if Language::parse(raw).is_none() {
        bail!("{raw:?} is not an ISO 639-1 language code (try `und`)");
    }
    Ok(())
}

/// Refuses a tag list by name, rather than dropping what will not fold.
///
/// **Stricter than the wire is, and deliberately.** `km_kmpkg::tag::parse_list` drops a word that
/// folds to nothing, because a `?tags=` on a browsing surface naming one real tag and one typo
/// should be a narrower list rather than a 400. A description is the opposite case: somebody wrote
/// this file meaning every line of it, so a tag that would silently vanish from the package is worth
/// stopping the build over — and saying which one.
fn check_tags(tags: &[String]) -> Result<()> {
    if tags.len() > km_kmpkg::tag::MAX_PER_SONG {
        bail!(
            "{} tags is more than the {} a song may carry",
            tags.len(),
            km_kmpkg::tag::MAX_PER_SONG
        );
    }
    for raw in tags {
        if km_kmpkg::Tag::parse(raw).is_none() {
            bail!(
                "{raw:?} is not a tag: a tag is a slug — ASCII letters, digits and `-`, no longer \
                 than {} characters. Accents fold away ({:?} is the tag `forro`); anything else \
                 does not.",
                km_kmpkg::tag::MAX_LENGTH,
                "Forró"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A description with everything filled in.
    fn full() -> Spec {
        Spec {
            package: SpecPackage {
                id: km_kmpkg::EXAMPLE_ID.to_owned(),
                name: "Volume One".to_owned(),
                version: "1.0.0".to_owned(),
                publisher: Some("The Hall".to_owned()),
                created: None,
                volume: None,
                default_language: Some("en".to_owned()),
                encoding: None,
                start_number: 1,
                transcode: true,
                out: Some("vol1.kmpkg".to_owned()),
                uncurated: false,
            },
            root: Some("songs".to_owned()),
            songs: vec![
                SpecSong {
                    file: "a.kar".to_owned(),
                    number: Some(1),
                    title: Some("Tempo Perdido".to_owned()),
                    artist: Some("Legião Urbana".to_owned()),
                    language: Some("pt".to_owned()),
                    ..SpecSong::default()
                },
                SpecSong {
                    file: "b.mp4".to_owned(),
                    number: Some(2),
                    transpose: Some(-2),
                    ..SpecSong::default()
                },
            ],
        }
    }

    #[test]
    fn a_description_survives_a_round_trip() {
        let text = serde_norway::to_string(&full()).expect("write");
        let back: Spec = serde_norway::from_str(&text).expect("read");
        assert_eq!(back, full());
    }

    /// The minimum: a package block and a list of paths.
    ///
    /// This is the shape a person writes by hand, and every default it leans on has to be the one
    /// `km-pack build <folder>` used to apply.
    #[test]
    fn the_smallest_useful_description_is_a_block_and_some_paths() {
        let spec: Spec = serde_norway::from_str(
            "package:\n  id: vol1\n  name: Volume One\nsongs:\n  - file: a.kar\n",
        )
        .expect("read");
        assert_eq!(spec.package.version, "1.0.0");
        assert_eq!(spec.package.start_number, 1);
        assert!(spec.package.transcode, "re-encoding is the default");
        assert_eq!(spec.songs[0].number, None, "the build assigns one");
        spec.validate().expect("valid");
    }

    /// A misspelled key is refused rather than ignored.
    ///
    /// The whole argument for `deny_unknown_fields`: `langauge:` silently dropped would package the
    /// song with no language, which is the defect this field exists to prevent.
    #[test]
    fn a_misspelled_key_is_an_error_and_names_itself() {
        let error = serde_norway::from_str::<Spec>(
            "package:\n  id: v\n  name: V\nsongs:\n  - file: a.kar\n    langauge: pt\n",
        )
        .expect_err("refused");
        assert!(
            error.to_string().contains("langauge"),
            "the message has to name the key: {error}"
        );
    }

    /// `bank:` is not a key here, and a description carrying one is refused by name.
    ///
    /// A package's bank comes from its id and from nothing else, so there is nowhere in a
    /// description to ask for a different one. Refused rather than ignored is what makes that
    /// legible: a curator who types a bank is told the key does not exist, instead of building a
    /// package that lands somewhere other than the number they wrote down.
    #[test]
    fn a_description_naming_a_bank_is_refused_by_name() {
        let error = serde_norway::from_str::<Spec>(
            "package:\n  id: vol1\n  name: Volume One\n  bank: 3\nsongs: []\n",
        )
        .expect_err("refused");
        assert!(error.to_string().contains("bank"), "got {error}");
    }

    /// `no` is Norwegian, and YAML would rather it were `false`.
    ///
    /// The trap this file's own header warns about. Quoted it is a string here; the test exists so
    /// that a later change of YAML library cannot quietly turn a language into a boolean.
    #[test]
    fn a_quoted_no_is_norwegian_and_not_a_boolean() {
        let spec: Spec = serde_norway::from_str(
            "package:\n  id: v\n  name: V\nsongs:\n  - file: a.kar\n    language: \"no\"\n",
        )
        .expect("read");
        assert_eq!(spec.songs[0].language.as_deref(), Some("no"));
        spec.validate().expect("Norwegian is a language");

        // And the write side, which is the half a generated file depends on. The serializer emits it
        // bare -- correct under YAML 1.2, and `false` to every 1.1 reader somebody might run over
        // the file -- so `quote_norwegian` puts it back.
        let mut written = spec.clone();
        written.package.default_language = Some("no".to_owned());
        let text = quote_norwegian(&serde_norway::to_string(&written).expect("write"));
        assert!(text.contains("language: \"no\""), "song: {text}");
        assert!(text.contains("default_language: \"no\""), "package: {text}");
        let back: Spec = serde_norway::from_str(&text).expect("read back");
        assert_eq!(back.songs[0].language.as_deref(), Some("no"));
        assert_eq!(back.package.default_language.as_deref(), Some("no"));
    }

    /// The quoting touches the two language keys and nothing else that happens to end in `no`.
    #[test]
    fn quoting_norwegian_leaves_every_other_value_alone() {
        assert_eq!(quote_norwegian("  title: no\n"), "  title: no\n");
        assert_eq!(quote_norwegian("  file: piano\n"), "  file: piano\n");
        assert_eq!(quote_norwegian("  language: nor\n"), "  language: nor\n");
        assert_eq!(quote_norwegian("  language: no\n"), "  language: \"no\"\n");
    }

    #[test]
    fn a_language_that_is_not_a_code_is_refused_by_name() {
        let mut spec = full();
        spec.songs[0].language = Some("Portuguese".to_owned());
        let error = spec.validate().expect_err("refused");
        let text = format!("{error:#}");
        assert!(text.contains("Portuguese"), "{text}");
        assert!(text.contains("a.kar"), "it has to say which song: {text}");
    }

    /// Two songs claiming one number, which is the defect a person cannot see by reading.
    #[test]
    fn a_number_claimed_twice_is_refused() {
        let mut spec = full();
        spec.songs[1].number = Some(1);
        let error = spec.validate().expect_err("refused");
        assert!(format!("{error:#}").contains("claimed by another song"));
    }

    /// A number nobody can dial, which is the defect a person cannot see by reading either.
    #[test]
    fn a_number_above_the_highest_is_refused() {
        let mut spec = full();
        spec.songs[1].number = Some(u32::from(km_songcode::MAX_SLOT) + 1);
        let error = spec.validate().expect_err("refused");
        assert!(
            format!("{error:#}").contains("above the highest song number"),
            "{error:#}"
        );
    }

    #[test]
    fn the_highest_number_itself_is_accepted() {
        let mut spec = full();
        spec.songs[1].number = Some(u32::from(km_songcode::MAX_SLOT));
        spec.validate().expect("the last dialable number is a song");
    }

    #[test]
    fn a_start_number_out_of_range_is_refused_rather_than_quietly_lifted() {
        // Unchecked at both ends, `start_number` 0 is silently raised to 1 by a `.max(1)` in two
        // places, so a description saying 0 builds a package saying 1.
        let mut spec = full();
        spec.package.start_number = 0;
        assert!(spec.validate().is_err());
        spec.package.start_number = u32::from(km_songcode::MAX_SLOT) + 1;
        assert!(spec.validate().is_err());
    }

    /// A description spells its paths with forward slashes, on whichever platform wrote it.
    ///
    /// The Unix half is the one worth having: a backslash is an ordinary character in a file name
    /// there, so a blanket replacement would corrupt the path of any song whose name contains one —
    /// on a machine nobody would be looking at when it happened.
    #[test]
    fn a_path_is_written_with_forward_slashes_and_a_unix_name_survives() {
        assert_eq!(slashed("a/b.kar"), "a/b.kar");
        if cfg!(windows) {
            assert_eq!(slashed(r"sub\a.kar"), "sub/a.kar");
        } else {
            assert_eq!(
                slashed(r"odd\name.kar"),
                r"odd\name.kar",
                "a backslash is part of the name here, not a separator"
            );
        }
    }

    #[test]
    fn a_package_with_no_id_or_no_name_is_refused() {
        let mut spec = full();
        spec.package.id = "  ".to_owned();
        assert!(spec.validate().is_err());

        let mut spec = full();
        spec.package.name = String::new();
        assert!(spec.validate().is_err());
    }

    /// Paths resolve against the description, so building one means the same from any directory.
    #[test]
    fn paths_resolve_against_the_description_and_its_root() {
        let spec = full();
        let base = spec.base(Path::new("/specs/vol1.kmspec.yaml"));
        assert_eq!(base, Path::new("/specs").join("songs"));
        assert_eq!(
            Spec::source(&base, &spec.songs[0]),
            Path::new("/specs").join("songs").join("a.kar")
        );

        // No `root:` at all means the description's own folder.
        let mut bare = full();
        bare.root = None;
        assert_eq!(
            bare.base(Path::new("/specs/vol1.kmspec.yaml")),
            Path::new("/specs")
        );
    }

    /// An absolute `file:` is taken as it stands, which is what a member outside the corpus needs.
    #[test]
    fn an_absolute_source_ignores_the_root() {
        let absolute = if cfg!(windows) {
            "C:/elsewhere/a.kar"
        } else {
            "/elsewhere/a.kar"
        };
        let song = SpecSong {
            file: absolute.to_owned(),
            ..SpecSong::default()
        };
        assert_eq!(
            Spec::source(Path::new("/corpus"), &song),
            Path::new(absolute)
        );
    }

    /// Written to disk and read back, header and all.
    #[test]
    fn a_written_description_reads_back_and_says_how_to_build_it() {
        let dir = std::env::temp_dir().join(format!("km-pack-spec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("vol1.kmspec.yaml");

        full().write(&path).expect("write");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.starts_with('#'), "the header comes first: {text}");
        assert!(text.contains("km-pack build"), "it says how to build it");
        assert!(text.contains("Norwegian"), "and warns about `no`");
        assert_eq!(Spec::read(&path).expect("read back"), full());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writing refuses to record something that could not be built.
    #[test]
    fn an_invalid_description_is_never_written() {
        let dir = std::env::temp_dir().join(format!("km-pack-spec-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("bad.kmspec.yaml");

        let mut spec = full();
        spec.package.default_language = Some("English".to_owned());
        assert!(spec.write(&path).is_err());
        assert!(!path.exists(), "nothing was left behind");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
