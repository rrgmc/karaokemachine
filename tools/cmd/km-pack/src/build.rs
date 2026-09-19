//! Turning a [`Spec`] into a `.kmpkg`. The one place a package is built.
//!
//! One loop and one input. A description is what the command line reads from a file and what the
//! curation tool builds in memory from its database, so "the two tools cannot produce different
//! packages from the same songs" is a property of the code rather than a thing to keep remembering.
//!
//! Two loops kept in step by both calling the same per-song primitives is most of the way there and
//! leaves the parts that actually decide what a package *contains* duplicated: numbering, duplicate
//! detection, the order things are added in, and the language gate.
//!
//! # Nothing here prints
//!
//! The rule the rest of this library already follows, and it is what makes the module usable from a
//! web server. Progress is a [`BuildEvent`] handed to a callback; the outcome is a [`BuildOutcome`]
//! the caller renders. `km-pack` turns those into lines on stderr, `km-package-builder` turns them
//! into counters an HTTP poll reads.
//!
//! **A callback rather than a shared `Arc<Progress>`**, which is what `km-package-builder`'s scan
//! uses and is right *there* because it has one consumer. This has three with different needs: the
//! command line wants text as it happens, the web tool wants atomics, and the tests want neither.
//! An `Arc<Progress>` in the signature would make the command line poll its own atomics in order to
//! print. Each caller owns the state it wants instead — which is exactly how
//! [`crate::add_video_song`]'s existing `on_progress` already works.
//!
//! # Provenance
//!
//! The description's values are laid over a **fresh** parse through [`crate::apply_edits`], so a
//! field is marked `edited` only where the description and the file disagree. See the module
//! documentation of [`crate::spec`] for why that is derived rather than stored.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use km_kmpkg::{Language, PackageBuilder, PackageMeta, content_hash};
use km_song::{ParseOptions, Song};
use km_suitability::Analysis;

use crate::spec::{Spec, SpecSong};
use crate::{ChosenFields, Edits, apply_edits, entry_from_analysis, extension};

/// Where a build reads from and writes to.
#[derive(Debug, Clone, Copy)]
pub struct BuildOptions<'a> {
    /// The folder `file:` values resolve against — [`Spec::base`], or a curation root.
    pub base: &'a Path,
    /// Where to write, overriding whatever the description says.
    pub out: Option<&'a Path>,
    /// Do everything except write the package.
    pub dry_run: bool,
    /// Measure how loud each video and MP3+G song is, so the machine can level it.
    ///
    /// **On for a real build**, and off is `km-pack build --no-loudness`. It costs a full audio
    /// decode per media song, which is worth paying once for a package somebody will keep and is
    /// not worth paying on every iteration of a description somebody is still editing. A song built
    /// without a measurement plays unlevelled, exactly as one built before the field existed does.
    ///
    /// A MIDI song is unaffected either way: it is the reference the other two are levelled to and
    /// carries no measurement at all.
    pub measure_loudness: bool,
    /// Write a plain-text listing of the package beside it, at `<out>.txt`.
    ///
    /// **Off here and on in the curation tool**, and the split is who is holding the package. A
    /// package built from a description is built repeatedly while the description is edited, and a
    /// second file rewritten each time is noise in the folder; one built to be handed to somebody is
    /// handed over with the thing that says what is in it. `km-pack build --listing` asks for it.
    ///
    /// It says only what the manifest says — see [`crate::listing`], which is where the rule about
    /// naming nothing local is kept.
    pub write_listing: bool,
}

/// What a build is doing, as it does it.
///
/// Borrowed rather than owned throughout: a build of four thousand songs raises four thousand of
/// these and neither caller keeps one.
#[derive(Debug)]
pub enum BuildEvent<'a> {
    /// How many songs there are to do, before any of them is read.
    Starting {
        /// Songs in the description.
        songs: usize,
    },
    /// A song is about to be read.
    Song {
        /// Its position in the description, from zero.
        index: usize,
        /// The number it will carry.
        number: u32,
        /// Where its bytes are.
        source: &'a Path,
    },
    /// A video is being re-encoded, which is the only part of a build that takes minutes.
    #[cfg(feature = "video")]
    Encoding {
        /// The song being encoded.
        number: u32,
        /// How far through.
        progress: crate::profile::Progress,
    },
    /// A song went in, with any finding about how.
    Added {
        /// The song's number.
        number: u32,
        /// What happened, when it was not simply "it went in".
        note: Option<String>,
    },
    /// A song did not go in.
    Skipped {
        /// Where its bytes were.
        source: &'a Path,
        /// Why not.
        why: &'a str,
    },
    /// Every song is in and the archive is being written.
    ///
    /// Raised because [`PackageBuilder::write`] has no progress of its own, so a four-thousand-song
    /// zip is a silent minute at the end of a build that has otherwise been reporting all along.
    /// Without this, both front ends appear to hang at 100%.
    Writing {
        /// Where the package is going.
        out: &'a Path,
    },
}

/// A song that was left out, and why.
///
/// One shape for both front ends. `km-pack` used `(PathBuf, Rejection)` and `km-package-builder`
/// used `(u32, String)`, which meant the same list could not be rendered by one function.
#[derive(Debug, Clone)]
pub struct Skipped {
    /// The number it would have had, when one had been decided.
    pub number: Option<u32>,
    /// Where its bytes were.
    pub source: PathBuf,
    /// Why it was left out, in a sentence somebody can act on.
    pub why: String,
}

/// What a build did.
#[derive(Debug, Clone, Default)]
pub struct BuildOutcome {
    /// Where the package went, or would have gone on a dry run.
    pub out_path: PathBuf,
    /// Songs written into it.
    pub written: usize,
    /// Songs left out, with the reason.
    pub skipped: Vec<Skipped>,
    /// Problems the manifest itself reports. A non-empty list means nothing was written.
    pub problems: Vec<String>,
    /// Songs with no language, which stop the build. Number and title, so a page can link to each.
    ///
    /// A field rather than an error, because the answer to "why did it not build?" is a list of
    /// songs to go and classify.
    pub unlanguaged: Vec<(u32, String)>,
    /// Videos stored as they arrived, being already in the packaging profile.
    pub videos_copied: usize,
    /// Videos re-encoded on the way in.
    ///
    /// Counted apart from [`Self::videos_copied`] because they are what a build spends its time on.
    pub videos_transcoded: usize,
    /// MP3+G pairs written, both files each.
    pub cdg_written: usize,
    /// UltraStar songs written, as their audio and a lyric timeline each.
    pub ultrastar_written: usize,
    /// Whether the callback asked the build to stop before it ran out of songs.
    pub canceled: bool,
    /// Where the plain-text listing went, when one was asked for and written.
    ///
    /// A field rather than something a caller derives from [`Self::out_path`], so a page or a
    /// console can name the second file it just produced without knowing how it is named.
    pub listing_path: Option<PathBuf>,
    /// The manifest as built, for a caller that wants to summarize what went in.
    ///
    /// Carried rather than recomputed because the alternative is a dozen aggregate fields here that
    /// only one caller reads — suitabilities, melody coverage, the counts per kind. `None` when the
    /// build stopped before there was a manifest worth having.
    pub manifest: Option<km_kmpkg::Manifest>,
}

impl BuildOutcome {
    /// Whether anything at all went into the package.
    pub fn is_empty(&self) -> bool {
        self.written == 0
    }

    /// Whether the package was actually written.
    pub fn wrote(&self) -> bool {
        self.written > 0
            && self.problems.is_empty()
            && self.unlanguaged.is_empty()
            && !self.canceled
    }
}

/// A folder a build re-encodes into, removed however the build ends.
///
/// It exists because a build now streams media into the archive rather than writing it beside one,
/// so a re-encode needs somewhere to land in between. Removing it on `Drop` rather than at the end
/// of [`build`] is what covers that function's several early returns — a canceled run, an
/// unbuildable song, the language gate — without each of them having to remember.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // Best effort and silent. It is created lazily, so a build with no re-encode in it leaves
        // nothing here to remove and nothing to report about that.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Builds a package from a description.
///
/// The callback sees every [`BuildEvent`] and answers [`ControlFlow::Break`] to stop. Stopping is
/// checked **between songs** rather than inside ffmpeg, so a canceled build waits for the song in
/// hand and not for the whole run — which is what keeps closing the curation tool during a build
/// from freezing it for the length of a re-encode.
///
/// Errors are reserved for the things that stop a build being possible at all: no output path, a
/// description naming a language that is not a code, a write that failed. A song that cannot be read
/// is a [`Skipped`], never an error — one bad file must not lose the other three thousand nine
/// hundred and ninety-nine.
pub fn build(
    spec: &Spec,
    options: &BuildOptions<'_>,
    mut on: impl FnMut(BuildEvent<'_>) -> ControlFlow<()>,
) -> Result<BuildOutcome> {
    spec.validate()?;

    let out = match options.out {
        Some(path) => path.to_path_buf(),
        None => match spec.package.out.as_deref().filter(|out| !out.is_empty()) {
            Some(out) => options.base.join(out),
            None => bail!(
                "nowhere to write the package: give --out, or an `out:` in the description's \
                 `package` block"
            ),
        },
    };

    // Parsed before a single file is opened, so a typo costs nothing rather than surfacing after
    // several thousand files have been read. `validate` already refused anything unparseable.
    let default_language = spec
        .package
        .default_language
        .as_deref()
        .and_then(Language::parse);

    let mut builder = PackageBuilder::new(PackageMeta {
        id: spec.package.id.clone(),
        name: spec.package.name.clone(),
        version: spec.package.version.clone(),
        publisher: spec.package.publisher.clone(),
        created: Some(
            spec.package
                .created
                .clone()
                .unwrap_or_else(crate::timestamp),
        ),
        volume: spec.package.volume.clone(),
    });

    let mut outcome = BuildOutcome {
        out_path: out.clone(),
        ..BuildOutcome::default()
    };

    // A `BTreeSet` and not a `Vec`. A fully numbered four-thousand-song description makes a
    // linear `contains` quadratic, and this function is on a web page's critical path as well as
    // a command line's.
    let mut taken: BTreeSet<u32> = spec.songs.iter().filter_map(|song| song.number).collect();
    let mut next_number = spec.package.start_number.max(1);

    // The duplicate net. Selecting what goes in a package is the description's job, so ordinarily
    // there is nothing here to catch -- but "the same recording under two numbers is a catalog
    // defect" is a guarantee this codebase makes loudly, and a hand-written description can
    // reintroduce one. The hash is computed for every MIDI song anyway.
    let mut hashes: BTreeMap<String, u32> = BTreeMap::new();

    // Where a re-encode lands before it is streamed into the archive. Beside the output rather than
    // in the system temp folder, because a re-encoded 4K video is gigabytes and `/tmp` is a tmpfs on
    // the machines this runs on. A directory rather than individual temp files, so one
    // `remove_dir_all` cleans up after a run that ended early — and it will, whichever of this
    // function's several early returns it takes, because the guard does it on drop.
    //
    // Deliberately **not** owned by `PackageBuilder`: the builder is also handed the owner's own
    // videos on the copy path, and a builder that deleted what it was given would be a foot-gun.
    let scratch = ScratchDir(out.with_extension("kmpkg.build"));
    // Looked up once for the whole build, and only if something might need it.
    // `km-package-builder` used to probe ffmpeg once per video.
    let encoders = video_encoders(spec, &mut on);

    if on(BuildEvent::Starting {
        songs: spec.songs.len(),
    })
    .is_break()
    {
        outcome.canceled = true;
        return Ok(outcome);
    }

    for (index, song) in spec.songs.iter().enumerate() {
        let source = Spec::source(options.base, song);

        let number = match song.number {
            Some(number) => number,
            None => {
                while taken.contains(&next_number)
                    && next_number <= u32::from(km_songcode::MAX_SLOT)
                {
                    next_number += 1;
                }
                // Running out is a skipped song and not a failed build: the description named more
                // songs than there are numbers, and the useful answer is the package plus a list of
                // what would not fit. It is also the guard the bare `next_number += 1` never had --
                // it would have panicked in debug at `u32::MAX`.
                if next_number > u32::from(km_songcode::MAX_SLOT) {
                    let why = format!(
                        "no song number left; they run 1 to {}",
                        km_songcode::MAX_SLOT
                    );
                    if on(BuildEvent::Skipped {
                        source: &source,
                        why: &why,
                    })
                    .is_break()
                    {
                        outcome.canceled = true;
                        return Ok(outcome);
                    }
                    outcome.skipped.push(Skipped {
                        number: None,
                        source,
                        why,
                    });
                    continue;
                }
                taken.insert(next_number);
                next_number
            }
        };

        if on(BuildEvent::Song {
            index,
            number,
            source: &source,
        })
        .is_break()
        {
            outcome.canceled = true;
            return Ok(outcome);
        }

        // Kind comes from the file, never from the description. Every other part of this workspace
        // decides it the same way, and a second source of truth that can disagree with the bytes is
        // a defect waiting to happen.
        let added = if crate::is_midi(&source) {
            add_midi(
                &mut builder,
                spec,
                song,
                &source,
                number,
                &mut hashes,
                &mut outcome,
            )
        } else if crate::is_video(&source) {
            add_video(
                &mut builder,
                song,
                &source,
                scratch.path(),
                number,
                spec.package.transcode,
                encoders.as_ref(),
                options.dry_run,
                options.measure_loudness,
                &mut outcome,
                &mut on,
            )
        } else if crate::is_audio(&source) {
            add_cdg(
                &mut builder,
                song,
                &source,
                number,
                options.dry_run,
                options.measure_loudness,
                &mut outcome,
            )
        } else if crate::is_ultrastar_candidate(&source) {
            add_ultrastar(
                &mut builder,
                song,
                &source,
                number,
                options.dry_run,
                options.measure_loudness,
                &mut outcome,
            )
        } else {
            Err(format!(
                "{} is not a MIDI file, a video, an MP3+G pair or an UltraStar file",
                source.display()
            ))
        };

        match added {
            Ok(note) => {
                taken.insert(number);
                if on(BuildEvent::Added { number, note }).is_break() {
                    outcome.canceled = true;
                    return Ok(outcome);
                }
            }
            Err(why) => {
                if on(BuildEvent::Skipped {
                    source: &source,
                    why: &why,
                })
                .is_break()
                {
                    outcome.canceled = true;
                    return Ok(outcome);
                }
                outcome.skipped.push(Skipped {
                    number: Some(number),
                    source,
                    why,
                });
            }
        }
    }

    outcome.written = builder.len();
    outcome.problems = builder.problems().iter().map(ToString::to_string).collect();
    if outcome.written == 0 || !outcome.problems.is_empty() {
        outcome.manifest = Some(builder.manifest().clone());
        return Ok(outcome);
    }

    // After everything is added and after `apply_edits`, so a filled language is never compared
    // against detection and so can never be recorded as a correction. Videos and MP3+G songs are the
    // population this most often catches: no container states what language the singing is in, and
    // CD+G carries no text at all.
    settle_languages(&mut builder, default_language, &mut outcome);
    outcome.manifest = Some(builder.manifest().clone());
    if !outcome.unlanguaged.is_empty() || options.dry_run {
        return Ok(outcome);
    }

    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let _ = on(BuildEvent::Writing { out: &out });
    // Through a temporary name and a rename, the same trick `write_package` uses. A half-written
    // 300 KB manifest is survivable; a half-written twenty-gigabyte archive sitting in a folder
    // the machine scans at every start is not, and with media inside the package that is what an
    // interrupted build would leave.
    let partial = out.with_extension("kmpkg.part");
    builder
        .write(&partial)
        .with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, &out)
        .with_context(|| format!("renaming {} into place", partial.display()))?;

    // After the rename, so the listing and the package it describes appear together: a reader who
    // finds the text file finds the archive beside it. There is no temporary name here because
    // there is nothing to protect — a half-written listing is a text file somebody rebuilds, and it
    // sits outside the folder the machine scans.
    if options.write_listing {
        let listing_path = listing_path(&out);
        // `outcome.manifest` was set above from the builder, so the listing describes exactly what
        // went in rather than what the description asked for.
        if let Some(manifest) = &outcome.manifest {
            std::fs::write(&listing_path, crate::listing::listing(manifest))
                .with_context(|| format!("writing {}", listing_path.display()))?;
            outcome.listing_path = Some(listing_path);
        }
    }
    Ok(outcome)
}

/// Where a package's listing goes: the package's own name with `.txt` on the end.
///
/// `vol1-1.2.0.kmpkg.txt` rather than `vol1-1.2.0.txt`, so the two sort together in a folder and the
/// name says which package the text belongs to. Appended rather than substituted for that reason:
/// a package and its listing differ by an extension and by nothing else.
#[must_use]
pub fn listing_path(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_owned();
    name.push(".txt");
    PathBuf::from(name)
}

/// The encoders, looked up once, and only when a description could need them.
///
/// A description of files already in profile must not require an ffmpeg capable of *encoding*,
/// because nothing is going to encode. A failure here is reported and then ignored: the build goes
/// on and stores what it can.
#[cfg(feature = "video")]
fn video_encoders(
    spec: &Spec,
    on: &mut impl FnMut(BuildEvent<'_>) -> ControlFlow<()>,
) -> Option<crate::profile::Encoders> {
    if !spec.package.transcode
        || !spec
            .songs
            .iter()
            .any(|song| crate::is_video(Path::new(&song.file)))
    {
        return None;
    }
    match crate::profile::encoders() {
        Ok(found) => Some(found),
        Err(error) => {
            let why = format!("no re-encoding available: {error}");
            let _ = on(BuildEvent::Skipped {
                source: Path::new(""),
                why: &why,
            });
            None
        }
    }
}

/// There is nothing to look up in a build with no `video` feature.
#[cfg(not(feature = "video"))]
fn video_encoders(
    _spec: &Spec,
    _on: &mut impl FnMut(BuildEvent<'_>) -> ControlFlow<()>,
) -> Option<()> {
    None
}

/// Adds one MIDI song, returning any finding about it.
///
/// The description's values are laid over a fresh parse rather than passed into
/// [`entry_from_analysis`], and that is the whole provenance mechanism: `apply_edits` marks a field
/// only where the two disagree, so a description saying what the file already says records no
/// correction and one that overrules it records exactly which fields.
#[allow(clippy::too_many_arguments)]
fn add_midi(
    builder: &mut PackageBuilder,
    spec: &Spec,
    song: &SpecSong,
    source: &Path,
    number: u32,
    hashes: &mut BTreeMap<String, u32>,
    outcome: &mut BuildOutcome,
) -> Result<Option<String>, String> {
    let _ = outcome;
    let bytes = std::fs::read(source).map_err(|error| format!("could not be read: {error}"))?;

    let hash = content_hash(&bytes);
    if let Some(existing) = hashes.get(&hash) {
        return Err(format!("the same recording as song {existing}"));
    }

    let encoding = song
        .encoding
        .clone()
        .or_else(|| spec.package.encoding.clone());
    let options = ParseOptions {
        declared_encoding: encoding.clone(),
        inference: None,
    };
    let parsed = Song::parse(&bytes, &options).map_err(|_| "is not a MIDI file".to_owned())?;
    let analysis = Analysis::of(&parsed);

    // `language: None` on purpose, and the same for the title and artist below: what the *file*
    // says is what goes in first, so that `apply_edits` has something honest to compare against.
    let mut entry = entry_from_analysis(
        &parsed,
        &analysis,
        ChosenFields {
            number,
            title: parsed
                .meta
                .title
                .clone()
                .unwrap_or_else(|| crate::file_stem(source)),
            artist: parsed.meta.artist.clone(),
            language: None,
            file: format!("midi/{number}{}", extension(source)),
            lyric_encoding: encoding,
        },
    );

    let detected = entry.clone();
    let edits = Edits {
        title: song.title.clone(),
        artist: song.artist.clone().map(Some),
        language: song.language.clone().map(Some),
        // Always `Some`, where its neighbours are `Some` only when the description said something:
        // an empty list is a real value here rather than a silence, so a rebuild whose description
        // dropped a tag has to take it off rather than leaving it.
        tags: Some(song.tags.clone()),
        encoding: song.encoding.clone().map(Some),
        transpose: song.transpose,
        lyrics_hidden: song.lyrics_hidden,
        fixes: song.fixes.clone(),
        melody: song.melody,
    };
    apply_edits(&mut entry, &edits, &detected);

    hashes.insert(hash, number);
    builder
        .add(entry, bytes)
        .map_err(|error| error.to_string())?;
    Ok(None)
}

/// Adds one video song.
///
/// A typed title replaces the container's tag directly rather than going through `apply_edits`, and
/// that is not an oversight: `edited` records where somebody overruled a *parse*, and a video's
/// title is read from a tag. There is no detection here to have corrected.
#[cfg(feature = "video")]
#[allow(clippy::too_many_arguments)]
fn add_video(
    builder: &mut PackageBuilder,
    song: &SpecSong,
    source: &Path,
    media_dir: &Path,
    number: u32,
    transcode: bool,
    encoders: Option<&crate::profile::Encoders>,
    dry_run: bool,
    measure_loudness: bool,
    outcome: &mut BuildOutcome,
    on: &mut impl FnMut(BuildEvent<'_>) -> ControlFlow<()>,
) -> Result<Option<String>, String> {
    let request = crate::VideoRequest {
        number,
        title: song.title.clone(),
        artist: song.artist.clone(),
        // The description is the only thing that can know this, and until it was passed here it was
        // written down and thrown away: every video song took the package's `default_language`
        // however specific the description had been.
        language: song.language.clone(),
        // From the description, which is the only thing that knows: no container tag says what a
        // song is filed under.
        tags: song.tags.clone(),
        transcode: transcode && encoders.is_some(),
        measure_loudness,
        dry_run,
    };
    let result =
        crate::add_video_song(builder, source, media_dir, &request, encoders, |progress| {
            let _ = on(BuildEvent::Encoding { number, progress });
        });
    let video = result.map_err(|error| format!("{error:#}"))?;

    if video.transcoded {
        outcome.videos_transcoded += 1;
    } else {
        outcome.videos_copied += 1;
    }

    let how = if video.transcoded {
        " re-encoded"
    } else if video.mismatches.is_empty() {
        ""
    } else {
        " kept as it is"
    };
    let mut note = format!(
        "{}x{}, {} ms{how}",
        video.info.width, video.info.height, video.info.duration_ms
    );
    for mismatch in &video.mismatches {
        note.push_str(&format!("\n      {mismatch}"));
    }
    // Beside the profile mismatches, on the same reasoning: a finding about a file that went in
    // anyway. The MP3+G path says it through `CdgOutcome::findings`, which already means
    // exactly this and which a video has no equivalent of.
    if let Some(reason) = &video.loudness_note {
        note.push_str(&format!("\n      {reason}"));
    }
    Ok(Some(note))
}

/// Refuses a video, in a build with no `video` feature.
///
/// The rest of the package is still written, and the reason names the feature so it is actionable.
/// Stricter than not looking at videos at all in such a build, which leaves a folder half of which
/// is video producing a package half the size with nothing said about the difference.
#[cfg(not(feature = "video"))]
#[allow(clippy::too_many_arguments)]
fn add_video(
    _builder: &mut PackageBuilder,
    _song: &SpecSong,
    _source: &Path,
    _media_dir: &Path,
    _number: u32,
    _transcode: bool,
    _encoders: Option<&()>,
    _dry_run: bool,
    _measure_loudness: bool,
    _outcome: &mut BuildOutcome,
    _on: &mut impl FnMut(BuildEvent<'_>) -> ControlFlow<()>,
) -> Result<Option<String>, String> {
    Err("this build has no `video` feature, so it cannot package a video".to_owned())
}

/// Adds one MP3+G pair.
///
/// **No `#[cfg]` twin, unlike [`add_video`], and the absence is the point**: `km-cdg` is pure Rust,
/// so there is no build of this that can list a pair and be unable to package it.
fn add_cdg(
    builder: &mut PackageBuilder,
    song: &SpecSong,
    audio: &Path,
    number: u32,
    dry_run: bool,
    measure_loudness: bool,
    outcome: &mut BuildOutcome,
) -> Result<Option<String>, String> {
    // Named in the description when it is not the sibling with the same stem; found by the rule play
    // time uses otherwise, so the packager and the machine cannot disagree about which graphics
    // belong to a song.
    let graphics = match song.graphics.as_deref() {
        Some(named) => {
            let path = audio.parent().unwrap_or(Path::new(".")).join(named);
            if !path.is_file() {
                return Err(format!("the named graphics file {named} is not there"));
            }
            path
        }
        None => crate::pair_for(audio).ok_or_else(|| "there is no .cdg beside it".to_owned())?,
    };

    let request = crate::CdgRequest {
        number,
        title: song.title.clone(),
        artist: song.artist.clone(),
        // As for a video, and for a stronger reason: an MP3+G pair has no text in it at all, so the
        // description is not merely the best source of the language, it is the only one.
        language: song.language.clone(),
        // As for a video, and for the stronger reason: an MP3+G pair has no text in it at all.
        tags: song.tags.clone(),
        measure_loudness,
        dry_run,
    };
    let cdg = crate::add_cdg_song(
        builder,
        &crate::CdgPair {
            audio: audio.to_path_buf(),
            graphics,
        },
        &request,
    )
    .map_err(|error| format!("{error:#}"))?;

    outcome.cdg_written += 1;
    let note = cdg
        .findings
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n      ");
    Ok((!note.is_empty()).then_some(note))
}

/// Adds one UltraStar song, from its `.txt`.
fn add_ultrastar(
    builder: &mut PackageBuilder,
    song: &SpecSong,
    text: &Path,
    number: u32,
    dry_run: bool,
    measure_loudness: bool,
    outcome: &mut BuildOutcome,
) -> Result<Option<String>, String> {
    let source = crate::read_ultrastar(text).map_err(|refusal| refusal.to_string())?;
    let request = crate::ultrastar::UltraStarRequest {
        number,
        title: song.title.clone(),
        artist: song.artist.clone(),
        language: song.language.clone(),
        tags: song.tags.clone(),
        lyrics_hidden: song.lyrics_hidden,
        measure_loudness,
        dry_run,
    };
    let added = crate::ultrastar::add_ultrastar_song(builder, &source, &request)
        .map_err(|error| format!("{error:#}"))?;

    outcome.ultrastar_written += 1;
    let note = added.findings.join("\n      ");
    Ok((!note.is_empty()).then_some(note))
}

/// Fills the default language where nothing else supplied one, and lists what is left.
///
/// # Why the check is here and not in the manifest
///
/// It would be natural to make this a `ManifestProblem`, so every route into a package got the same
/// guarantee for free. It cannot be: `Package::open` runs `Manifest::problems()` as well as
/// `PackageBuilder::write` does, so a language rule there would refuse to *open* every package built
/// before language was a code — all of which carry a raw `ENGL` — and empty the catalog of every
/// machine in service. Requiring a language is a curation rule, so it lives in the thing that
/// curates, which is now this one function rather than two.
fn settle_languages(
    builder: &mut PackageBuilder,
    default: Option<Language>,
    outcome: &mut BuildOutcome,
) {
    if let Some(default) = default {
        for song in builder.songs_mut() {
            if song.language.is_none() {
                song.language = Some(default.code().to_owned());
                // Deliberately not `mark_edited`: `edited` means detection got this song wrong and
                // the correction must survive a rebuild. A blanket default is not a correction, and
                // marking four thousand songs hand-edited would make the flag stop meaning anything.
            }
        }
        return;
    }

    outcome.unlanguaged = builder
        .manifest()
        .songs
        .iter()
        .filter(|song| song.language.is_none())
        .map(|song| (song.number, song.title.clone()))
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::spec::{SpecPackage, SpecSong};

    /// A scratch folder that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-pack-build-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }

        fn write(&self, name: &str, bytes: Vec<u8>) {
            std::fs::write(self.0.join(name), bytes).expect("fixture");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn spec_for(songs: Vec<SpecSong>) -> Spec {
        Spec {
            package: SpecPackage {
                id: km_kmpkg::EXAMPLE_ID.to_owned(),
                name: "Volume One".to_owned(),
                version: "1.0.0".to_owned(),
                publisher: None,
                created: None,
                volume: None,
                default_language: None,
                encoding: None,
                start_number: 1,
                transcode: true,
                out: None,
            },
            root: None,
            songs,
        }
    }

    /// Runs a build, ignoring every event.
    fn run(spec: &Spec, base: &Path, out: &Path) -> BuildOutcome {
        run_with(spec, base, out, false)
    }

    fn run_with(spec: &Spec, base: &Path, out: &Path, write_listing: bool) -> BuildOutcome {
        build(
            spec,
            &BuildOptions {
                base,
                out: Some(out),
                dry_run: false,
                measure_loudness: true,
                write_listing,
            },
            |_| ControlFlow::Continue(()),
        )
        .expect("build")
    }

    /// The whole provenance argument, in one test.
    ///
    /// A description that says back what the file says records nothing; one that overrules it
    /// records exactly the field it overruled. Get this wrong in the generous direction and every
    /// song in a corpus is marked hand-edited, which costs the flag its meaning and fails nothing.
    #[test]
    fn a_description_marks_only_what_it_disagrees_with() {
        let scratch = Scratch::new("provenance");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        let out = scratch.0.join("vol1.kmpkg");

        // What the fixture itself says, so the description can agree with it exactly.
        let bytes = km_song::testing::soft_karaoke();
        let parsed = Song::parse(&bytes, &ParseOptions::default()).expect("parse");
        let said = parsed.meta.title.clone().expect("the fixture has a title");

        let agreeing = spec_for(vec![SpecSong {
            file: "a.kar".to_owned(),
            number: Some(1),
            title: Some(said),
            ..SpecSong::default()
        }]);
        run(&agreeing, &scratch.0, &out);
        let package = km_kmpkg::Package::open(&out).expect("open");
        assert!(
            package.manifest().songs[0].edited.is_empty(),
            "agreeing with the file is not a correction: {:?}",
            package.manifest().songs[0].edited
        );

        let correcting = spec_for(vec![SpecSong {
            file: "a.kar".to_owned(),
            number: Some(1),
            title: Some("Something Else".to_owned()),
            ..SpecSong::default()
        }]);
        run(&correcting, &scratch.0, &out);
        let package = km_kmpkg::Package::open(&out).expect("open");
        let entry = &package.manifest().songs[0];
        assert_eq!(entry.title, "Something Else");
        assert!(
            entry.is_edited(km_kmpkg::EditedField::Title),
            "overruling the file is a correction, so a rebuild keeps it"
        );
        assert!(
            !entry.is_edited(km_kmpkg::EditedField::Artist),
            "and only the field that was overruled: {:?}",
            entry.edited
        );
    }

    /// The gate, both ways round.
    ///
    /// The negative half matters most: the fixture's own `@LENGL` is enough, so a song nobody has
    /// classified still builds. Without that the gate would fire on a corpus that had done nothing
    /// wrong, and the first thing anybody would do is turn it off.
    #[test]
    fn a_song_with_no_language_stops_the_build_unless_the_package_names_a_default() {
        let scratch = Scratch::new("language");
        scratch.write("plain.mid", km_song::testing::lyric_events());
        let out = scratch.0.join("vol1.kmpkg");

        let bare = spec_for(vec![SpecSong {
            file: "plain.mid".to_owned(),
            number: Some(1),
            ..SpecSong::default()
        }]);
        let outcome = run(&bare, &scratch.0, &out);
        assert_eq!(outcome.unlanguaged.len(), 1, "{:?}", outcome.unlanguaged);
        assert!(!outcome.wrote());
        assert!(!out.exists(), "and nothing was written");

        let mut defaulted = bare.clone();
        defaulted.package.default_language = Some("en".to_owned());
        let outcome = run(&defaulted, &scratch.0, &out);
        assert!(outcome.unlanguaged.is_empty());
        assert!(outcome.wrote());

        let package = km_kmpkg::Package::open(&out).expect("open");
        let entry = &package.manifest().songs[0];
        assert_eq!(entry.language.as_deref(), Some("en"));
        assert!(
            !entry.is_edited(km_kmpkg::EditedField::Language),
            "a blanket default is not a correction somebody made"
        );

        // And a description that says so *is* a correction, because the file said nothing.
        let mut said = bare.clone();
        said.songs[0].language = Some("pt".to_owned());
        run(&said, &scratch.0, &out);
        let package = km_kmpkg::Package::open(&out).expect("open");
        let entry = &package.manifest().songs[0];
        assert_eq!(entry.language.as_deref(), Some("pt"));
        assert!(entry.is_edited(km_kmpkg::EditedField::Language));
    }

    /// A description carrying the same recording twice, which `PackageBuilder` would not catch:
    /// the two entries have different numbers and are perfectly valid.
    #[test]
    fn the_same_recording_twice_is_caught_and_named() {
        let scratch = Scratch::new("dupes");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        scratch.write("copy.kar", km_song::testing::soft_karaoke());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![
            SpecSong {
                file: "a.kar".to_owned(),
                number: Some(1),
                ..SpecSong::default()
            },
            SpecSong {
                file: "copy.kar".to_owned(),
                number: Some(2),
                ..SpecSong::default()
            },
        ]);
        let outcome = run(&spec, &scratch.0, &out);
        assert_eq!(outcome.written, 1);
        assert_eq!(outcome.skipped.len(), 1);
        assert!(
            outcome.skipped[0].why.contains("song 1"),
            "it has to say which one it is a copy of: {}",
            outcome.skipped[0].why
        );
    }

    /// Numbers are handed out from `start_number`, skipping anything a later song claims.
    #[test]
    fn numbers_fill_the_gaps_around_what_the_description_claims() {
        let scratch = Scratch::new("numbers");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        scratch.write("b.mid", km_song::testing::lyric_events());
        let out = scratch.0.join("vol1.kmpkg");

        let mut spec = spec_for(vec![
            SpecSong {
                file: "a.kar".to_owned(),
                ..SpecSong::default()
            },
            SpecSong {
                file: "b.mid".to_owned(),
                number: Some(100),
                ..SpecSong::default()
            },
        ]);
        spec.package.start_number = 100;
        spec.package.default_language = Some("en".to_owned());

        let outcome = run(&spec, &scratch.0, &out);
        assert_eq!(outcome.written, 2, "{:?}", outcome.skipped);
        let package = km_kmpkg::Package::open(&out).expect("open");
        let mut numbers: Vec<u32> = package.manifest().songs.iter().map(|s| s.number).collect();
        numbers.sort_unstable();
        assert_eq!(numbers, vec![100, 101], "the claimed number was not reused");
    }

    /// A file that is none of the four kinds is named rather than passed over.
    #[test]
    fn a_file_that_is_not_a_song_is_reported() {
        let scratch = Scratch::new("stranger");
        scratch.write("notes.docx", b"not a song".to_vec());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![SpecSong {
            file: "notes.docx".to_owned(),
            ..SpecSong::default()
        }]);
        let outcome = run(&spec, &scratch.0, &out);
        assert_eq!(outcome.written, 0);
        assert_eq!(outcome.skipped.len(), 1);
        assert!(outcome.skipped[0].why.contains("not a MIDI file"));
    }

    /// A build leaves nothing beside the package it wrote.
    ///
    /// Three absences, one for each thing this milestone changed. No `vol1.media` — media is inside
    /// the archive now and the folder should never be created. No `vol1.kmpkg.build` — the scratch
    /// folder a re-encode lands in is removed by its guard however the build ends. And no
    /// `vol1.kmpkg.part` — the archive is written under a temporary name and renamed, so a build
    /// interrupted between those two leaves nothing the machine's folder scan would pick up.
    ///
    /// (What goes *into* the archive is `km-kmpkg`'s round-trip tests; this is about what does
    /// not end up outside it.)
    #[test]
    fn a_build_leaves_nothing_beside_the_package() {
        let scratch = Scratch::new("nothing-beside");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![SpecSong {
            file: "a.kar".to_owned(),
            ..SpecSong::default()
        }]);
        let outcome = run(&spec, &scratch.0, &out);
        assert_eq!(outcome.written, 1, "{:?}", outcome.skipped);
        assert!(out.is_file());

        assert!(!scratch.0.join("vol1.media").exists(), "no media folder");
        assert!(!scratch.0.join("vol1.kmpkg.build").exists(), "no scratch");
        assert!(!scratch.0.join("vol1.kmpkg.part").exists(), "no partial");
        // The listing is asked for and is not written otherwise, which is what keeps a folder of
        // repeated builds to one file per package.
        assert!(!scratch.0.join("vol1.kmpkg.txt").exists(), "no listing");
    }

    /// Asked for, the listing is the one thing that appears beside the package.
    ///
    /// The other half of the test above: the flag adds a file and adds nothing else, so the three
    /// absences that test is about hold whichever way it is set.
    #[test]
    fn a_listing_is_written_beside_the_package_when_it_is_asked_for() {
        let scratch = Scratch::new("listing-beside");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![SpecSong {
            file: "a.kar".to_owned(),
            ..SpecSong::default()
        }]);
        let outcome = run_with(&spec, &scratch.0, &out, true);
        assert_eq!(outcome.written, 1, "{:?}", outcome.skipped);

        let listing = scratch.0.join("vol1.kmpkg.txt");
        assert!(listing.is_file(), "the listing is beside the package");
        assert_eq!(
            outcome.listing_path.as_deref(),
            Some(listing.as_path()),
            "the outcome names the second file it wrote"
        );
        // `.kmpkg.txt` and not `.txt`: the two sort together and the name says which package the
        // text belongs to.
        assert!(!scratch.0.join("vol1.txt").exists());

        let text = std::fs::read_to_string(&listing).expect("read");
        assert!(text.contains(&spec.package.name), "{text}");
        // What went in, read off the manifest rather than off the description.
        assert!(text.contains("Songs:     1"), "{text}");

        assert!(!scratch.0.join("vol1.media").exists(), "no media folder");
        assert!(!scratch.0.join("vol1.kmpkg.build").exists(), "no scratch");
        assert!(!scratch.0.join("vol1.kmpkg.part").exists(), "no partial");
    }

    /// A dry run writes no listing, because it writes no package for one to describe.
    #[test]
    fn a_dry_run_writes_no_listing_either() {
        let scratch = Scratch::new("listing-dry");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![SpecSong {
            file: "a.kar".to_owned(),
            ..SpecSong::default()
        }]);
        let outcome = build(
            &spec,
            &BuildOptions {
                base: &scratch.0,
                out: Some(&out),
                dry_run: true,
                measure_loudness: true,
                write_listing: true,
            },
            |_| ControlFlow::Continue(()),
        )
        .expect("build");

        assert!(!out.exists(), "a dry run writes no package");
        assert!(!scratch.0.join("vol1.kmpkg.txt").exists(), "nor a listing");
        assert!(outcome.listing_path.is_none());
    }

    /// A dry run says everything a real one would and writes nothing.
    #[test]
    fn a_dry_run_writes_nothing() {
        let scratch = Scratch::new("dry");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![SpecSong {
            file: "a.kar".to_owned(),
            ..SpecSong::default()
        }]);
        let outcome = build(
            &spec,
            &BuildOptions {
                base: &scratch.0,
                out: Some(&out),
                dry_run: true,
                measure_loudness: true,
                write_listing: false,
            },
            |_| ControlFlow::Continue(()),
        )
        .expect("build");
        assert_eq!(outcome.written, 1);
        assert!(!out.exists(), "a dry run leaves no package behind");
    }

    /// Breaking out of the callback stops between songs, so a canceled build is prompt.
    #[test]
    fn the_callback_can_stop_a_build() {
        let scratch = Scratch::new("cancel");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        scratch.write("b.mid", km_song::testing::lyric_events());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![
            SpecSong {
                file: "a.kar".to_owned(),
                ..SpecSong::default()
            },
            SpecSong {
                file: "b.mid".to_owned(),
                ..SpecSong::default()
            },
        ]);

        let mut seen = 0;
        let outcome = build(
            &spec,
            &BuildOptions {
                base: &scratch.0,
                out: Some(&out),
                dry_run: false,
                measure_loudness: true,
                write_listing: false,
            },
            |event| {
                if let BuildEvent::Song { .. } = event {
                    seen += 1;
                    if seen == 2 {
                        return ControlFlow::Break(());
                    }
                }
                ControlFlow::Continue(())
            },
        )
        .expect("build");

        assert!(outcome.canceled);
        assert!(!out.exists(), "a stopped build writes nothing");
    }

    /// The archive write is announced, because it is the one silent minute in a large build.
    #[test]
    fn the_write_is_announced_before_it_starts() {
        let scratch = Scratch::new("writing");
        scratch.write("a.kar", km_song::testing::soft_karaoke());
        let out = scratch.0.join("vol1.kmpkg");

        let spec = spec_for(vec![SpecSong {
            file: "a.kar".to_owned(),
            ..SpecSong::default()
        }]);
        let mut announced = false;
        build(
            &spec,
            &BuildOptions {
                base: &scratch.0,
                out: Some(&out),
                dry_run: false,
                measure_loudness: true,
                write_listing: false,
            },
            |event| {
                if let BuildEvent::Writing { .. } = event {
                    announced = true;
                }
                ControlFlow::Continue(())
            },
        )
        .expect("build");
        assert!(announced);
    }

    /// A build writes no bank, and the package still has one.
    ///
    /// Both halves matter. Nothing in the manifest names a thousand, so a `.kmpkg` says nothing
    /// about where it lands and cannot disagree with a machine that put it somewhere else; and
    /// `wanted_bank` still answers, from the id, which is what a book printed before any machine has
    /// seen the files is numbered from.
    #[test]
    fn a_build_writes_no_bank_and_the_package_is_still_banked_by_its_id() {
        let manifest = manifest_for(&spec_for(Vec::new()));
        assert_eq!(
            manifest.package.wanted_bank(),
            km_kmpkg::PackageMeta::suggested_bank(&manifest.package.id)
        );
        let json = serde_json::to_string(&manifest).expect("json");
        assert!(!json.contains("bank"), "no bank is written: {json}");
    }

    /// Builds far enough to see the manifest, without writing anything.
    fn manifest_for(spec: &Spec) -> km_kmpkg::Manifest {
        let report = build(
            spec,
            &BuildOptions {
                base: Path::new("."),
                out: Some(Path::new("unused.kmpkg")),
                dry_run: true,
                measure_loudness: true,
                write_listing: false,
            },
            |_| ControlFlow::Continue(()),
        )
        .expect("build");
        report
            .manifest
            .expect("a dry run still builds the manifest")
    }

    #[test]
    fn a_description_with_no_out_and_no_flag_is_an_error() {
        let spec = spec_for(Vec::new());
        let error = build(
            &spec,
            &BuildOptions {
                base: Path::new("."),
                out: None,
                dry_run: true,
                measure_loudness: true,
                write_listing: false,
            },
            |_| ControlFlow::Continue(()),
        )
        .expect_err("refused");
        assert!(format!("{error:#}").contains("nowhere to write"));
    }
}
