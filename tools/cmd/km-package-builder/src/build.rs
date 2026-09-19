//! Turning a curated selection into a `.kmpkg`, and reading one back in.
//!
//! **This module describes; it does not package.** Everything that decides what goes into a package
//! — the order, the numbering, the duplicate net, the language gate, what a manifest entry holds —
//! lives in `km_pack::build`, and what happens here is [`spec_for`]: reading this folder's database
//! into the same `km_pack::Spec` that `km-pack build` reads out of a YAML file.
//!
//! That is the whole point of the arrangement. The two tools used to keep in step by both calling
//! the same per-song primitives, which left the parts that actually decide a package's *contents*
//! duplicated in two loops somebody had to remember to change together. Now there is one loop, and
//! "a package built here and one built by `km-pack` from the same songs are the same package" is a
//! property of the code.
//!
//! It also means [`spec_for`] is the whole of *Write the description* — the same value, serialized
//! instead of consumed.
//!
//! # Why the description carries the *effective* title, and why that is safe
//!
//! Every song gets the title and artist that would be shown for it — what a person typed, else what
//! the file said, else the file's own name. Not the typed value alone, even though a MIDI title goes
//! through `apply_edits` and could therefore be recorded as a correction: **a description of bare
//! paths is not worth opening**, and being opened and corrected is the only reason it exists.
//!
//! It is safe because the build re-derives exactly the same fallback chain from the same bytes —
//! `parsed.meta.title` else the stem — so a description saying back what detection found compares
//! equal and marks nothing. That is the same property `km-pack spec` relies on, and
//! `the_written_description_records_no_correction_nobody_made` is the test that would catch the two
//! chains drifting apart.
//!
//! The one way it can be wrong is staleness: a source file edited since the scan leaves `det_title`
//! saying something the file no longer says, and the build then records a correction nobody made.
//! That errs towards keeping what a person last reviewed, which is the right direction, and a rescan
//! fixes it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use km_kmpkg::Package;
use km_pack::{Spec, SpecPackage, SpecSong};

use crate::db::{Db, DbError, Shared};
use crate::scan::timestamp;

/// What a build did, plus the two things this tool knows and `km_pack` cannot.
#[derive(Debug, Clone, Default)]
pub struct BuildReport {
    /// Where the package was written.
    pub out_path: String,
    /// The version it went out under.
    ///
    /// Said back on the page because a build may have raised it, and a number that moved without
    /// anybody being told is one somebody will later think they typed.
    pub version: String,
    /// Songs written into it.
    pub written: usize,
    /// Songs left out, with the reason. Number first, because the page shows numbers.
    pub skipped: Vec<(u32, String)>,
    /// Problems the manifest itself reports.
    pub problems: Vec<String>,
    /// Songs with no language, which stop the build. Number and title, so the page can link to each.
    pub unlanguaged: Vec<(u32, String)>,
    /// Videos stored as they arrived, because they were already in the packaging profile.
    pub videos_copied: usize,
    /// Videos re-encoded on the way in.
    ///
    /// Counted separately from [`Self::videos_copied`] because they are what a build spends its time
    /// on: a page that said only "4 songs written" after twenty minutes would look as though it had
    /// hung and then lied about it.
    pub videos_transcoded: usize,
    /// MP3+G pairs written, both files each.
    pub cdg_written: usize,
    /// UltraStar songs written.
    pub ultrastar_written: usize,
    /// Where the song list went, when one was asked for. Empty when it was not.
    ///
    /// Said back for the reason the version is: a build that wrote two files and named one leaves
    /// somebody looking for the other, or not knowing it is there to be handed over with the
    /// package.
    pub listing_path: String,
}

impl BuildReport {
    /// Whether anything at all went into the package.
    pub fn is_empty(&self) -> bool {
        self.written == 0
    }
}

/// A description read out of the curation database, and what could not be described.
#[derive(Debug)]
pub struct Curated {
    /// The description, ready to build or to write out.
    pub spec: Spec,
    /// Members whose source file is gone, so there was nothing to describe.
    pub missing: Vec<(u32, String)>,
}

/// Reads a package's members into a description.
///
/// This is the only database work a build does, and it is all up front. Everything after it — the
/// parsing, the analysis, an ffmpeg re-encode, writing the archive — needs no database at all, which
/// is what lets a build run on a thread of its own without holding the connection for the minutes it
/// takes.
pub fn spec_for(db: &Db, package_id: &str, volume: u32) -> Result<Curated, DbError> {
    let package = db.package_volume(package_id, volume)?;
    let members = db.package_members(package_id, volume)?;

    let mut songs = Vec::with_capacity(members.len());
    let mut missing = Vec::new();

    for member in &members {
        let Some(relative) = &member.path else {
            missing.push((member.number, "its source file is gone".to_owned()));
            continue;
        };
        let detail = db.song(&member.song_id)?;

        songs.push(SpecSong {
            file: km_pack::spec::slashed(relative),
            number: Some(member.number),
            // The effective values, so the file reads as a person left it. See the module documentation for
            // why writing what detection found records no correction.
            title: Some(detail.effective_title()),
            artist: detail.effective_artist(),
            // Typed only, never `det_language_tag`: the build detects that for itself from the same
            // bytes, and passing it here would record it as a correction of itself.
            language: detail.language.clone(),
            // Read from `song_tags`, which is where this tool keeps them — so a tag reaches a
            // package only because a song here carries it. That is what keeps a *suggested* tag out
            // of every package: a suggestion is a word offered in a picker, and until somebody puts
            // it on a song there is no row to read. See `settings::Settings::default_tags`.
            tags: db.tags_of(&member.song_id)?,
            encoding: detail.lyric_encoding.clone(),
            transpose: detail
                .default_transpose
                .map(|value| value.clamp(-12, 12) as i8),
            // The same silence as the corrections below: absent where nobody has said, so the
            // build measures the file for itself. A value is somebody overruling that measurement,
            // and `Some(false)` overrules it as much as `Some(true)` does.
            lyrics_hidden: detail.lyrics_hidden,
            // Silent where nobody has decided, so the build detects them and a song goes on
            // benefiting from a detector that has learned something since it was scanned.
            fixes: crate::fixes::stored(detail.fixes.as_deref()),
            // The same silence, one value over: absent where nobody has named a channel, so the
            // build takes what its own analysis finds. `Some(None)` is somebody saying this song has
            // no melody, which the machine reads as a guide-melody toggle it must not offer.
            melody: crate::fixes::MelodyChoice::parse(detail.melody_chosen.as_deref())
                .map(crate::fixes::MelodyChoice::channel),
            // Found beside the audio by the rule play time uses, so the packager and the machine
            // cannot disagree about which graphics belong to a song.
            graphics: None,
        });
    }

    Ok(Curated {
        spec: Spec {
            package: SpecPackage {
                // The volume's own id and name, which are the package's own while it has one volume.
                id: package.volume_id.clone(),
                name: package.volume_name(),
                version: package.version.clone(),
                publisher: package.publisher.clone(),
                // Left for the build to stamp, so a written description does not claim a moment that
                // has not happened yet.
                created: None,
                // Always written, including for a package of one volume, so a file imported back
                // knows which package it is part of whatever has happened to that package since.
                volume: Some(km_kmpkg::VolumeOf {
                    of: package.id.clone(),
                    name: package.name.clone(),
                    number: package.volume,
                }),
                // Straight through, and the build fills gaps with it without marking anything
                // hand-edited. `None` keeps the old behavior: a song with no language stops the
                // build and is listed for somebody to go and classify.
                default_language: package.default_language.clone(),
                encoding: None,
                start_number: package.start_number.max(1),
                transcode: true,
                // Forward slashes like `root` below. A backslash is an ordinary character in a YAML
                // plain scalar so either spells correctly, but a file somebody reads should not
                // change convention halfway down — and forward slashes are the spelling that also
                // works when the description is carried to the appliance.
                out: package.out_path.as_deref().map(km_pack::spec::slashed),
            },
            // The corpus root, named absolutely: a description written out of this database may be
            // saved anywhere, and the songs it names do not move with it.
            root: Some(km_pack::spec::slashed(
                &crate::model::tidy(db.root()).display().to_string(),
            )),
            songs,
        },
        missing,
    })
}

/// Builds a package from what a person has selected.
///
/// The source files are re-read and re-analyzed rather than trusted from the database, because the
/// package must describe the bytes it actually contains. The hand-set fields are then laid over the
/// top by `km_pack::build`, which is the one thing analysis must not be allowed to overwrite.
///
/// # Three phases, and the middle one holds no lock
///
/// This takes the `Arc<Shared>` rather than a `&Db` on purpose, and that is the whole reason a
/// build can report progress at all. It **locks briefly** to read the description, **does not lock**
/// for the parsing, the analysis, an ffmpeg re-encode and the archive write — which is the part that
/// takes minutes — and **locks briefly again** to record what it did. Held across the middle, the
/// mutex would queue every other page and every progress poll behind the very build they are asking
/// about.
///
/// `progress` is written as it goes and asked, between songs, whether to stop.
pub fn build(
    db: &Arc<Shared>,
    package_id: &str,
    volume: u32,
    out: &Path,
    write_listing: bool,
    progress: &BuildProgress,
) -> Result<BuildReport, DbError> {
    progress.say(Phase::Reading);
    let (curated, raised) = {
        let guard = db.lock();
        let mut curated = spec_for(&guard, package_id, volume)?;
        // Raised here, into the description this build is about to consume, rather than written to
        // the database first: the archive and the row must not be able to disagree about what was
        // written, and the row is only entitled to move once a file exists. It is stored below, in
        // the lock the build takes to record what it did.
        let raised = raise_for(&guard, package_id, volume)?;
        if let Some(version) = &raised {
            curated.spec.package.version = version.clone();
        }
        (curated, raised)
    };
    let root = PathBuf::from(curated.spec.root.clone().unwrap_or_default());
    let version = curated.spec.package.version.clone();
    progress.start(curated.spec.songs.len() as u64);

    let outcome = km_pack::build::build(
        &curated.spec,
        &km_pack::BuildOptions {
            base: &root,
            out: Some(out),
            dry_run: false,
            // No switch on the page, deliberately. `--no-loudness` exists for somebody iterating on
            // a description at a command line; a build started from here is a package somebody
            // means to keep and install, and a control whose only effect is to make that package
            // worse is not one worth drawing.
            measure_loudness: true,
            write_listing,
        },
        |event| progress.observe(&event),
    )
    .map_err(|error| DbError::Rejected(format!("{error:#}")))?;

    let mut report = report_from(&outcome, &version);
    // Prepended, so the songs that were never even describable come first: they are the ones a
    // person has to act on, and the rest are files that were read and refused.
    for missing in curated.missing.into_iter().rev() {
        report.skipped.insert(0, missing);
    }

    if outcome.wrote() {
        let guard = db.lock();
        guard.record_build(package_id, volume, &report.out_path, &timestamp())?;
        // Only now, so a build that was refused, failed or stopped burns no number and the field
        // on the page goes on naming the .kmpkg that is actually on disk.
        if let Some(version) = &raised {
            guard.set_package_version(package_id, volume, version)?;
        }
    }
    Ok(report)
}

/// Builds every volume of a package that holds a song, one after another, each under its default name.
///
/// **The default name, because there is one box per file and this run writes several.** Each volume is
/// written as [`default_out_path`] names it — the volume's name and the version the build is about to
/// write — into `folder`, or into the data folder when none is given, so the files of one run sort
/// together and none overwrites another.
///
/// **One volume's refusal does not stop the next**: a volume missing a language is that volume's to
/// fix, and every other volume's file is still worth having. An error that is not a refusal — the
/// database, a file that could not be written — ends the run, as does a stop.
pub fn build_all(
    db: &Arc<Shared>,
    package_id: &str,
    folder: Option<&Path>,
    write_listing: bool,
    progress: &BuildProgress,
) -> Result<(), DbError> {
    let (volumes, root, raise) = {
        let guard = db.lock();
        let volumes: Vec<u32> = guard
            .package_volumes(package_id)?
            .into_iter()
            .filter(|volume| volume.song_count > 0)
            .map(|volume| volume.volume)
            .collect();
        (
            volumes,
            guard.root().to_path_buf(),
            guard.raise_version(package_id)?,
        )
    };
    for volume in volumes {
        if progress.stopping() {
            break;
        }
        progress
            .building_volume
            .store(u64::from(volume), Ordering::Relaxed);
        for counter in [
            &progress.total,
            &progress.done,
            &progress.written,
            &progress.skipped,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
        let out = {
            let guard = db.lock();
            let row = guard.package_volume(package_id, volume)?;
            let name = file_name(&default_out_path(&root, &row, raise));
            folder
                .map_or_else(|| crate::db::data_dir(&root), Path::to_path_buf)
                .join(name)
        };
        let report = build(db, package_id, volume, &out, write_listing, progress)?;
        if let Ok(mut built) = progress.volumes_built.lock() {
            built.push((volume, report));
        }
    }
    Ok(())
}

/// The version a build of this package should write, when it is not the one already stored.
///
/// `None` — meaning write what the row says — in four cases, and each is a decision rather than a
/// fallback: the box is unticked; the package has never been built, so it ships at the version
/// somebody typed and only later builds raise; the stored version is not `X.Y.Z`, which is what a
/// package imported from a `.kmpkg` built elsewhere may carry; or the patch number cannot go any
/// higher.
fn raise_for(db: &Db, package_id: &str, volume: u32) -> Result<Option<String>, DbError> {
    if !db.raise_version(package_id)? {
        return Ok(None);
    }
    let package = db.package_volume(package_id, volume)?;
    if package.built_at.is_none() {
        return Ok(None);
    }
    Ok(crate::version::parse(&package.version)
        .and_then(|version| version.raised())
        .map(|version| version.to_string()))
}

/// How a build is getting on, for the page watching it.
///
/// Modeled on `scan::Progress` rather than on anything new — atomics for the counters, a `Mutex`
/// for the words, and the `cancel`/`finished` pair — because this tool already has one long job that
/// reports itself and a second shape would be a second thing to learn. What it does not copy is the
/// scan's read/write split: a build has one worker and no backlog to explain.
#[derive(Debug, Default)]
pub struct BuildProgress {
    /// Which package this belongs to, so a page can tell somebody else's build from its own.
    pub package_id: String,
    /// Which of that package's volumes, for the same reason. 0 is a run over every volume.
    pub volume: u32,
    /// The volume a run over every volume is building now, or 0 before the first.
    pub building_volume: AtomicU64,
    /// What each volume a run over every volume has finished did, in order.
    pub volumes_built: Mutex<Vec<(u32, BuildReport)>>,
    /// Songs in the description.
    pub total: AtomicU64,
    /// Songs dealt with, one way or the other.
    pub done: AtomicU64,
    /// Songs that went in.
    pub written: AtomicU64,
    /// Songs left out.
    pub skipped: AtomicU64,
    /// How far through the video being re-encoded right now, or [`NOT_ENCODING`].
    ///
    /// The one part of a build that is minutes rather than milliseconds, so it gets a number of its
    /// own: without it the bar sits still on one song and the whole thing looks stuck.
    pub encoding: AtomicU64,
    /// What it is doing, in words, for the line under the bar.
    pub phase: Mutex<Phase>,
    /// Set to ask the build to stop. Never cleared: a `BuildProgress` belongs to one run.
    ///
    /// Checked between songs rather than inside ffmpeg, so a stop takes at most one song — which is
    /// what keeps closing the folder mid-build from freezing the tool for a whole re-encode.
    pub cancel: AtomicBool,
    /// Set when the run has ended, however it ended.
    pub finished: AtomicBool,
    /// What went wrong, if anything did.
    pub error: Mutex<Option<String>>,
    /// What the finished build has to say, so the last poll renders the report the page always did.
    pub report: Mutex<Option<BuildReport>>,
}

/// What a build is doing, as a value rather than as a sentence.
///
/// **An enum and not a key**, which the scan's phases are, because four of these carry something: a
/// count, a song's position, a file name. A build runs on a worker thread with no request and so no
/// language in reach, so the fact travels and [`BuildProgressView::say_phase`] writes the words in
/// whatever language the page polling for them is being drawn in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Phase {
    /// Before anything has been read.
    #[default]
    Starting,
    /// Reading what the package says it holds.
    Reading,
    /// About to write this many songs.
    Packaging {
        /// How many there are.
        songs: u64,
    },
    /// Working through them, one at a time.
    Song {
        /// Which one, counting from one.
        index: u64,
        /// Out of how many.
        total: u64,
        /// The file being read.
        name: String,
    },
    /// Re-encoding a video, which is the one step that is minutes rather than milliseconds.
    ///
    /// Only a build with the `video` feature has anything to re-encode, and only that build names
    /// this variant — hence the gate, which is the one `km_pack::BuildEvent::Encoding` already
    /// carries.
    #[cfg(feature = "video")]
    Encoding {
        /// The song number being re-encoded.
        number: u32,
    },
    /// Writing the archive, which has no progress of its own.
    Writing {
        /// The file being written.
        name: String,
    },
    /// Ended with a package.
    Done,
    /// Ended because somebody asked it to.
    Stopped,
}

impl Phase {
    /// What this says, in one language.
    pub fn say(&self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Starting => words.msg("build-phase-starting").into_owned(),
            Self::Reading => words.msg("build-phase-reading").into_owned(),
            Self::Packaging { songs } => words
                .msg_with(
                    "build-phase-packaging",
                    &[("count", i64::try_from(*songs).unwrap_or(i64::MAX).into())],
                )
                .into_owned(),
            Self::Song { index, total, name } => words
                .msg_with(
                    "build-phase-song",
                    &[
                        ("index", i64::try_from(*index).unwrap_or(i64::MAX).into()),
                        ("total", i64::try_from(*total).unwrap_or(i64::MAX).into()),
                        ("name", name.as_str().into()),
                    ],
                )
                .into_owned(),
            #[cfg(feature = "video")]
            Self::Encoding { number } => words
                .msg_with(
                    "build-phase-encoding",
                    &[("number", i64::from(*number).into())],
                )
                .into_owned(),
            Self::Writing { name } => words
                .msg_with("build-phase-writing", &[("name", name.as_str().into())])
                .into_owned(),
            Self::Done => words.msg("build-phase-done").into_owned(),
            Self::Stopped => words.msg("build-phase-stopped").into_owned(),
        }
    }
}

/// [`BuildProgress::encoding`] when nothing is being re-encoded.
pub const NOT_ENCODING: u64 = u64::MAX;

impl BuildProgress {
    /// A fresh run, for one package.
    pub fn new(package_id: &str, volume: u32) -> Self {
        Self {
            package_id: package_id.to_owned(),
            volume,
            encoding: AtomicU64::new(NOT_ENCODING),
            phase: Mutex::new(Phase::Starting),
            ..Self::default()
        }
    }

    /// Sets what the line under the bar is about.
    pub fn say(&self, phase: Phase) {
        if let Ok(mut slot) = self.phase.lock() {
            *slot = phase;
        }
    }

    /// Records how many songs there are to do.
    pub fn start(&self, songs: u64) {
        self.total.store(songs, Ordering::Relaxed);
    }

    /// Asks the run to stop at the next song boundary.
    pub fn ask_to_stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Whether a stop has been asked for.
    pub fn stopping(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Records how a run over every volume ended. What each volume did is already in
    /// [`Self::volumes_built`].
    pub fn finish_all(&self, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => self.say(Phase::Done),
            Err(error) => {
                if let Ok(mut slot) = self.error.lock() {
                    *slot = Some(error);
                }
                self.say(Phase::Stopped);
            }
        }
        self.encoding.store(NOT_ENCODING, Ordering::Relaxed);
        self.finished.store(true, Ordering::Release);
    }

    /// Records the outcome and marks the run over.
    pub fn finish(&self, outcome: Result<BuildReport, String>) {
        match outcome {
            Ok(report) => {
                if let Ok(mut slot) = self.report.lock() {
                    *slot = Some(report);
                }
                self.say(Phase::Done);
            }
            Err(error) => {
                if let Ok(mut slot) = self.error.lock() {
                    *slot = Some(error);
                }
                self.say(Phase::Stopped);
            }
        }
        self.encoding.store(NOT_ENCODING, Ordering::Relaxed);
        self.finished.store(true, Ordering::Release);
    }

    /// Takes one event from the build, and answers whether to carry on.
    fn observe(&self, event: &km_pack::BuildEvent<'_>) -> std::ops::ControlFlow<()> {
        match event {
            km_pack::BuildEvent::Starting { songs } => {
                self.start(*songs as u64);
                self.say(Phase::Packaging {
                    songs: *songs as u64,
                });
            }
            km_pack::BuildEvent::Song { index, source, .. } => {
                self.encoding.store(NOT_ENCODING, Ordering::Relaxed);
                let name = source
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.say(Phase::Song {
                    index: *index as u64 + 1,
                    total: self.total.load(Ordering::Relaxed),
                    name,
                });
            }
            #[cfg(feature = "video")]
            km_pack::BuildEvent::Encoding { number, progress } => {
                self.encoding
                    .store(u64::from(progress.percent()), Ordering::Relaxed);
                self.say(Phase::Encoding { number: *number });
            }
            km_pack::BuildEvent::Added { .. } => {
                self.written.fetch_add(1, Ordering::Relaxed);
                self.done.fetch_add(1, Ordering::Relaxed);
            }
            km_pack::BuildEvent::Skipped { .. } => {
                self.skipped.fetch_add(1, Ordering::Relaxed);
                self.done.fetch_add(1, Ordering::Relaxed);
            }
            km_pack::BuildEvent::Writing { out } => {
                self.encoding.store(NOT_ENCODING, Ordering::Relaxed);
                let name = out
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Said out loud because the archive write has no progress of its own, and on a large
                // package it is a silent minute that otherwise looks like a hang at 100%.
                self.say(Phase::Writing { name });
            }
        }
        if self.stopping() {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    }

    /// A snapshot, for rendering.
    pub fn snapshot(&self) -> BuildProgressView {
        let total = self.total.load(Ordering::Relaxed);
        let done = self.done.load(Ordering::Relaxed);
        let encoding = self.encoding.load(Ordering::Relaxed);
        BuildProgressView {
            package_id: self.package_id.clone(),
            volume: self.volume,
            building_volume: u32::try_from(self.building_volume.load(Ordering::Relaxed))
                .unwrap_or(u32::MAX),
            volumes_built: self
                .volumes_built
                .lock()
                .map(|slot| slot.clone())
                .unwrap_or_else(|error| error.into_inner().clone()),
            total,
            done,
            written: self.written.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            encoding: (encoding != NOT_ENCODING).then_some(encoding.min(100)),
            percent: done
                .saturating_mul(100)
                .checked_div(total)
                .unwrap_or(0)
                .min(100),
            phase: self
                .phase
                .lock()
                .map(|slot| slot.clone())
                .unwrap_or_else(|error| error.into_inner().clone()),
            // Left for whoever draws it; see `BuildProgressView::say_phase`.
            phase_said: String::new(),
            finished: self.finished.load(Ordering::Acquire),
            error: self
                .error
                .lock()
                .map(|slot| slot.clone())
                .unwrap_or_else(|error| error.into_inner().clone()),
            report: self
                .report
                .lock()
                .map(|slot| slot.clone())
                .unwrap_or_else(|error| error.into_inner().clone()),
        }
    }
}

/// A snapshot of a build in flight, or of one that has ended.
#[derive(Debug, Clone, Default)]
pub struct BuildProgressView {
    /// Which package it belongs to.
    pub package_id: String,
    /// Which of its volumes, or 0 for a run over every volume.
    pub volume: u32,
    /// The volume a run over every volume is building now.
    pub building_volume: u32,
    /// What each finished volume of such a run did.
    pub volumes_built: Vec<(u32, BuildReport)>,
    /// Songs to do.
    pub total: u64,
    /// Songs done.
    pub done: u64,
    /// Songs that went in.
    pub written: u64,
    /// Songs left out.
    pub skipped: u64,
    /// How far through a re-encode, when one is running.
    pub encoding: Option<u64>,
    /// How far through, as a percentage of songs.
    pub percent: u64,
    /// What it is doing, as a fact rather than as a sentence.
    pub phase: Phase,
    /// What that says, once somebody drawing a page has said in which language.
    ///
    /// Empty until [`Self::say_phase`] is called, which the handler does: a snapshot is taken on a
    /// worker thread, where no request and so no language is in reach.
    pub phase_said: String,
    /// Whether it has ended.
    pub finished: bool,
    /// What went wrong, if anything.
    pub error: Option<String>,
    /// What it did, once it has ended.
    pub report: Option<BuildReport>,
}

impl BuildProgressView {
    /// Words the phase, in the language the page asking is being drawn in.
    pub fn say_phase(&mut self, locale: km_locale::Locale) {
        self.phase_said = self.phase.say(locale);
    }
}

/// Turns what `km_pack` reports into what this tool's page renders.
///
/// The two differ in one way that matters: a page shows song *numbers*, and `km_pack` reports paths,
/// because from a folder of loose files a path is the only identity a refused song has.
fn report_from(outcome: &km_pack::BuildOutcome, version: &str) -> BuildReport {
    BuildReport {
        out_path: outcome.out_path.display().to_string(),
        version: version.to_owned(),
        written: outcome.written,
        skipped: outcome
            .skipped
            .iter()
            .map(|skipped| {
                let name = Path::new(&skipped.source)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| skipped.source.display().to_string());
                (
                    skipped.number.unwrap_or(0),
                    format!("{name} {}", skipped.why),
                )
            })
            .collect(),
        problems: outcome.problems.clone(),
        unlanguaged: outcome.unlanguaged.clone(),
        videos_copied: outcome.videos_copied,
        videos_transcoded: outcome.videos_transcoded,
        cdg_written: outcome.cdg_written,
        ultrastar_written: outcome.ultrastar_written,
        listing_path: outcome
            .listing_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    }
}

/// What happened when an existing package was read back in.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    /// The package's id.
    pub package_id: String,
    /// Entries matched to a song already in the corpus.
    pub matched: usize,
    /// Entries whose bytes are nowhere under the root, with their titles.
    pub unmatched: Vec<(u32, String)>,
    /// Entries whose hand-set language this build cannot read, with what the package said.
    ///
    /// A package written by a later build may carry a code this one does not have. It is reported
    /// rather than imported, and it does not stop the import.
    pub unreadable_language: Vec<(u32, String)>,
}

/// Re-opens a `.kmpkg` as a curation package.
///
/// Entries are matched to songs by **content hash**, the way `km-pack spec --from` matches them:
/// paths and numbers both change across rebuilds, and the bytes of a recording do not. An entry whose
/// hash is nowhere in the corpus is listed rather than dropped — that is a file the packager has and
/// this folder does not, which is worth knowing rather than silently losing.
pub fn import(db: &mut Db, path: &Path) -> Result<ImportReport, DbError> {
    let package = Package::open(path)
        .map_err(|error| DbError::Rejected(format!("opening {}: {error}", path.display())))?;
    let manifest = package.manifest().clone();

    let now = timestamp();
    // A file that names its package joins it as the volume it says it is. One that names none is a
    // package of one volume under its own id, which is what every file built before volumes was.
    let (package_id, name, volume) = match &manifest.package.volume {
        Some(of) => (of.of.clone(), of.name.clone(), of.number.max(1)),
        None => (
            manifest.package.id.clone(),
            manifest.package.name.clone(),
            1,
        ),
    };
    let row = crate::model::PackageRow {
        name,
        version: manifest.package.version.clone(),
        publisher: manifest.package.publisher.clone(),
        start_number: manifest.songs.iter().map(|s| s.number).min().unwrap_or(1),
        // **`None`, not `en`**, and re-importing must not change it: every song in a package that
        // built successfully already has a language, so a default would have nothing to do — and
        // silently attaching one to a package somebody deliberately left strict would undo that
        // choice on the first re-open. `update_package` writes this, so the value has to be the
        // honest one rather than a convenient one.
        default_language: db
            .package(&package_id)
            .ok()
            .and_then(|existing| existing.default_language),
        out_path: Some(path.display().to_string()),
        song_count: manifest.songs.len() as u32,
        volume,
        volume_id: manifest.package.id.clone(),
        ..crate::model::PackageRow::new(&package_id, "")
    };
    // Re-importing the same package updates it rather than failing: opening one twice is a normal
    // thing to do, and refusing would mean deleting it by hand first.
    if db.package(&package_id).is_err() {
        // Volume 1 takes the package's id, so a later volume arriving first still leaves the
        // package the shape every other one has.
        db.create_package(&row, &now)?;
    }
    db.ensure_volume(&package_id, volume, &manifest.package.id, &now)?;
    db.update_package(&row)?;

    let mut report = ImportReport {
        package_id: row.id.clone(),
        ..ImportReport::default()
    };

    for entry in &manifest.songs {
        // The manifest's own hash is what an entry is matched by; every package this build writes
        // records one for every song. An entry without one is reported rather than re-hashed: for a
        // media song re-hashing is **possible and wrong**, because its recorded hash is its
        // *source*'s — see `add_video_song` — and a re-encoded video's stored bytes are not the
        // source's bytes, so recomputing would quietly record something that matches nothing.
        let Some(hash) = entry.content_hash.clone() else {
            report.unmatched.push((entry.number, entry.title.clone()));
            continue;
        };

        if db.song(&hash).is_err() {
            report.unmatched.push((entry.number, entry.title.clone()));
            continue;
        }

        db.add_to_volume(&row.id, row.volume, std::slice::from_ref(&hash), &now)?;
        db.set_package_number(&row.id, &hash, entry.number)?;

        // Corrections recorded in the package become corrections here. That is the whole point of
        // re-opening one: the hand-edited titles come back rather than being retyped.
        let edit = crate::db::SongEdit {
            title: entry
                .is_edited(km_kmpkg::EditedField::Title)
                .then(|| Some(entry.title.clone())),
            artist: entry
                .is_edited(km_kmpkg::EditedField::Artist)
                .then(|| entry.artist.clone()),
            // Mapped through `Language::parse` rather than handed over as it stands. `edit_song`
            // refuses a value that is not a code, and the values in a package are not this build's
            // to trust: an older one carries a raw `ENGL`, and a later one may carry a code this
            // build has never heard of. Either would abort the whole import at the first such song,
            // which would make re-opening a package a thing that fails for reasons nobody could act
            // on. Dropped and counted instead -- the import's job is to recover what it can.
            language: entry.is_edited(km_kmpkg::EditedField::Language).then(|| {
                let parsed = entry
                    .language
                    .as_deref()
                    .and_then(km_kmpkg::Language::parse);
                if parsed.is_none()
                    && let Some(raw) = entry.language.as_deref()
                {
                    report
                        .unreadable_language
                        .push((entry.number, raw.to_owned()));
                }
                parsed.map(|language| language.code().to_owned())
            }),
            lyric_encoding: entry
                .is_edited(km_kmpkg::EditedField::LyricEncoding)
                .then(|| entry.lyric_encoding.clone()),
            default_transpose: entry
                .is_edited(km_kmpkg::EditedField::DefaultTranspose)
                .then_some(Some(entry.default_transpose)),
            // The marker and not the field, for the reason the corrections below give: the flag a
            // package carries is usually its build's own measurement speaking through a file, and
            // importing that would turn a measurement into a decision nobody made.
            lyrics_hidden: entry
                .is_edited(km_kmpkg::EditedField::LyricsHidden)
                .then_some(Some(entry.lyrics_hidden)),
            notes: None,
            // Only a hand-edited list comes back. A package's detected corrections are this build's
            // own detector speaking through a file, and importing them would turn a proposal into a
            // decision nobody made.
            fixes: entry
                .is_edited(km_kmpkg::EditedField::Fixes)
                .then_some(Some(crate::fixes::encode(&entry.fixes))),
            // The marker and not the field, for the reason the line above gives: a melody record a
            // detector wrote is a proposal, and importing one would turn it into a decision nobody
            // made. Where somebody did decide, an absent record means they said there is none.
            melody_chosen: entry.is_edited(km_kmpkg::EditedField::Melody).then(|| {
                Some(
                    entry
                        .melody
                        .as_ref()
                        .map_or(crate::fixes::MelodyChoice::None, |record| {
                            crate::fixes::MelodyChoice::Channel(record.channel)
                        })
                        .as_str(),
                )
            }),
        };
        db.edit_song(&hash, &edit)?;
        report.matched += 1;
    }

    Ok(report)
}

/// Where a package should be written by default: the corpus's own data folder, under a name that
/// says which package and which build.
///
/// **`<name>-<version>.kmpkg`, so two builds are told apart on sight.** A version's only job is to
/// tell two files of one package apart — nothing in the machine compares two of them — and a name
/// that leaves it out gives the version nowhere to do that job: the second build overwrites the
/// first and the number in the manifest is the only record that they differed.
///
/// **The version is the one the build will write**, from [`crate::version::next`], which is also
/// what the tick box's label names. The raise happens before the write, so a default built from the
/// *stored* version would put a name on the form that no file on disk ever had.
///
/// **A name rather than the id**, which is sixteen hexadecimal characters and tells a person nothing
/// about a folder of them. Where the id is needed is on the machine, and the machine derives its own
/// name — see `km_kmpkg::PackageMeta::file_stem` — so nothing downstream depends on what this
/// returns. It is a default in a box somebody may type over.
///
/// It used to land loose in the corpus root. See [`crate::db::DATA_SUBDIR`] for why it no longer
/// does, and note that nothing here needs to create the folder — [`build`] writes through
/// `km_pack`, which `create_dir_all`s the parent of whatever it is given.
pub fn default_out_path(root: &Path, package: &crate::model::PackageRow, raise: bool) -> PathBuf {
    let version = crate::version::next(&package.version, package.built_at.is_some(), raise);
    // Both halves are folded before they are joined. A typed version is already three numbers and
    // an imported package's id and version have both been through `Manifest::problems`, so neither
    // fold has anything to do — and this is the line that builds a path out of two strings a
    // document supplied, which is the line worth being unable to get wrong.
    crate::db::data_dir(root).join(format!(
        "{}-{}.kmpkg",
        package_stem(package),
        nameable(&version)
    ))
}

/// The last part of a path, which is what the Build tab's boxes show beside the one folder field.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Where a package's description should be written by default: beside the package.
///
/// **Named the way the package it describes is named.** The two land in one folder and say the same
/// thing about the same package, so a reader who can pick a `.kmpkg` out of that folder by eye can
/// pick its description out too. No version in it: a description is of the package, where a `.kmpkg`
/// is of one build of it and a rebuild writes another file beside the first.
pub fn default_spec_path(root: &Path, package: &crate::model::PackageRow) -> PathBuf {
    crate::db::data_dir(root).join(format!("{}.kmspec.yaml", package_stem(package)))
}

/// What a package's own files are called, before whatever distinguishes one from another.
///
/// The id when a name folds away to nothing, for the reason `file_stem` gives: a package whose name
/// is entirely non-ASCII still has to be addressable.
fn package_stem(package: &crate::model::PackageRow) -> String {
    nameable(
        &km_kmpkg::name_slug(&package.volume_name())
            .unwrap_or_else(|| nameable(&package.volume_id)),
    )
}

/// A value from a package row, as something that can be one component of a file name.
///
/// **Every caller's value has already been checked somewhere else** — `Manifest::problems` refuses
/// an id or a version that is not a name, and the two forms that take a typed version refuse
/// anything but three numbers. This is what makes those checks unnecessary rather than load-bearing:
/// the paths these defaults name are built here, and a fold here cannot be skipped by a route added
/// later.
///
/// Folded rather than refused, because these are *defaults offered in a box somebody may type over*
/// and a build page that failed to render would be a worse answer than one offering an odd name.
fn nameable(value: &str) -> String {
    if km_kmpkg::is_safe_name(value) {
        return value.to_owned();
    }
    km_kmpkg::name_slug(value).unwrap_or_else(|| "package".to_owned())
}

/// Writes a package's description to disk.
///
/// The same value the build consumes, serialized instead. That is the whole implementation and the
/// whole point: there is no second code path that could describe a package differently from the way
/// it is built, so what this writes is exactly what a `km-pack build` of it would produce.
pub fn write_spec(db: &Db, package_id: &str, volume: u32, out: &Path) -> Result<usize, DbError> {
    let curated = spec_for(db, package_id, volume)?;
    let songs = curated.spec.songs.len();
    curated
        .spec
        .write(out)
        .map_err(|error| DbError::Rejected(format!("writing {}: {error:#}", out.display())))?;
    Ok(songs)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::Db;
    use crate::scan::{ScanOptions, run};
    use std::sync::Arc;

    use crate::testing::Scratch;

    /// Scans a one-song corpus and returns its database and the song's id.
    fn corpus(scratch: &Scratch) -> (Arc<Shared>, String) {
        std::fs::write(scratch.0.join("song.kar"), km_song::testing::soft_karaoke())
            .expect("write a fixture");
        let db = Arc::new(Shared::new(Db::open_in_memory(&scratch.0).expect("open")));
        run(
            &db,
            ScanOptions::default(),
            &Arc::new(crate::scan::Progress::default()),
        )
        .expect("scan");

        let id = {
            let guard = db.lock();
            guard
                .songs(&crate::db::Filter::default())
                .expect("browse")
                .first()
                .expect("the fixture was scanned")
                .id
                .clone()
        };
        (db, id)
    }

    /// Builds a package and waits for it, with a throwaway progress nobody watches.
    ///
    /// The tests here are about what a build *produces*; that it reports itself along the way is
    /// `Workspace`'s business and is tested there.
    fn build_now(db: &Arc<Shared>, package_id: &str, out: &Path) -> BuildReport {
        build(
            db,
            package_id,
            1,
            out,
            false,
            &BuildProgress::new(package_id, 1),
        )
        .expect("build")
    }

    /// Makes a package holding the given songs.
    fn package_with(guard: &mut Db, ids: &[String]) {
        let row = crate::model::PackageRow {
            id: km_kmpkg::EXAMPLE_ID.to_owned(),
            name: "Volume One".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            // No default, so these tests exercise the gate rather than the prefill. The prefill has
            // a test of its own.
            default_language: None,
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        };
        guard.create_package(&row, &timestamp()).expect("create");
        guard
            .add_to_package(km_kmpkg::EXAMPLE_ID, ids, &timestamp())
            .expect("add");
    }

    /// What a curated package carries about a song — and what it deliberately does not.
    ///
    /// The detected suitability goes in through `km_pack`'s shared build, which this path shares
    /// with `km-pack build`, so it is asserted here to pin that the *curation* route really does go
    /// through it and has not quietly grown a hand-rolled entry. The person's own rating is the
    /// case this test guards from the other side: it is what one listener thought of a file, so it
    /// stays in this corpus's database and must never reach a package. Nothing detects it, so a
    /// leak would be silent in both directions.
    #[test]
    fn a_built_package_carries_what_was_detected_and_never_what_was_judged() {
        let scratch = Scratch::new("suitability");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
            guard.set_user_score(&id, Some(8)).expect("judge");
        }
        // Outside the guard, and it has to be: `build` takes the lock itself, briefly at each end,
        // so calling it while holding one deadlocks. That is the arrangement that lets the rest of
        // the tool answer while a package builds.
        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert_eq!(report.written, 1, "{:?}", report.skipped);
        assert!(report.problems.is_empty(), "{:?}", report.problems);

        // Not a hard-coded number: what the package claims has to agree with what the scan recorded
        // for the same bytes, whatever the scoring rules happen to say this month.
        let detected = db
            .lock()
            .song(&id)
            .expect("song")
            .midi
            .expect("a MIDI song was scanned")
            .suitability;

        let package = km_kmpkg::Package::open(&out).expect("open the package");
        let entry = &package.manifest().songs[0];

        let suitability = entry
            .suitability
            .as_ref()
            .expect("the detected suitability travels with the song");
        assert_eq!(suitability.value, detected);
        let breakdown = suitability.breakdown;
        assert_eq!(
            u32::from(breakdown.lyrics)
                + u32::from(breakdown.sync)
                + u32::from(breakdown.channels)
                + u32::from(breakdown.arrangement),
            u32::from(suitability.value),
            "the breakdown has to add up to the suitability, or neither is explainable"
        );
        // The judgment stayed at home. Read off the serialized manifest rather than off a struct
        // field, because there is no field and the assertion has to be about what a recipient of
        // this file can actually see.
        let json = serde_json::to_string(package.manifest()).expect("serializes");
        assert!(!json.contains("user_score"), "got {json}");

        // ...and it is still here, where it was made.
        assert_eq!(db.lock().song(&id).expect("song").user_score, Some(8));

        // Importing the package back into a fresh corpus therefore restores everything a package
        // carries and invents no judgment, which is the half that used to travel.
        let second = Scratch::new("import");
        let (fresh, _) = corpus(&second);
        let mut guard = fresh.lock();
        let report = import(&mut guard, &out).expect("import");
        assert_eq!(report.matched, 1, "unmatched: {:?}", report.unmatched);
        assert_eq!(guard.song(&id).expect("song").user_score, None);
    }

    /// A later volume is written under its own id and name, says which package it belongs to, and
    /// imports back into that package as the same volume.
    #[test]
    fn a_second_volume_builds_as_its_own_file_and_imports_back_into_its_package() {
        let scratch = Scratch::new("second-volume");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol2.kmpkg");
        let second = km_kmpkg::PackageMeta::new_id();
        {
            let mut guard = db.lock();
            package_with(&mut guard, &[]);
            guard
                .ensure_volume(km_kmpkg::EXAMPLE_ID, 2, &second, &timestamp())
                .expect("a second volume");
            guard
                .add_to_volume(
                    km_kmpkg::EXAMPLE_ID,
                    2,
                    std::slice::from_ref(&id),
                    &timestamp(),
                )
                .expect("add");
        }
        let report = build(
            &db,
            km_kmpkg::EXAMPLE_ID,
            2,
            &out,
            false,
            &BuildProgress::new(km_kmpkg::EXAMPLE_ID, 2),
        )
        .expect("build");
        assert_eq!(report.written, 1, "{:?}", report.skipped);

        let manifest = km_kmpkg::Package::open(&out)
            .expect("open")
            .manifest()
            .clone();
        assert_eq!(manifest.package.id, second);
        assert_eq!(manifest.package.name, "Volume One vol2");
        assert_eq!(
            manifest.package.volume,
            Some(km_kmpkg::VolumeOf {
                of: km_kmpkg::EXAMPLE_ID.to_owned(),
                name: "Volume One".to_owned(),
                number: 2,
            })
        );

        let fresh_scratch = Scratch::new("second-volume-import");
        let (fresh, _) = corpus(&fresh_scratch);
        let mut guard = fresh.lock();
        let imported = import(&mut guard, &out).expect("import");
        assert_eq!(imported.matched, 1, "unmatched: {:?}", imported.unmatched);
        let volume = guard
            .package_volume(km_kmpkg::EXAMPLE_ID, 2)
            .expect("the volume it said it was");
        assert_eq!(volume.volume_id, second);
        assert_eq!(volume.name, "Volume One");
        assert_eq!(volume.song_count, 1);
    }

    /// Building every volume writes each one that holds a song under its default name, and skips an
    /// empty one.
    #[test]
    fn building_every_volume_writes_each_under_its_default_name() {
        let scratch = Scratch::new("every-volume");
        let (db, id) = corpus(&scratch);
        {
            let mut guard = db.lock();
            package_with(&mut guard, &[]);
            guard
                .ensure_volume(
                    km_kmpkg::EXAMPLE_ID,
                    2,
                    &km_kmpkg::PackageMeta::new_id(),
                    &timestamp(),
                )
                .expect("a second volume");
            guard
                .add_to_volume(
                    km_kmpkg::EXAMPLE_ID,
                    2,
                    std::slice::from_ref(&id),
                    &timestamp(),
                )
                .expect("add");
        }
        let progress = BuildProgress::new(km_kmpkg::EXAMPLE_ID, 0);
        build_all(
            &db,
            km_kmpkg::EXAMPLE_ID,
            Some(&scratch.0),
            false,
            &progress,
        )
        .expect("build every volume");

        let built = progress.snapshot().volumes_built;
        assert_eq!(
            built.iter().map(|(volume, _)| *volume).collect::<Vec<_>>(),
            [2],
            "the empty first volume is skipped"
        );
        assert_eq!(built[0].1.written, 1, "{:?}", built[0].1.skipped);
        assert!(
            scratch.0.join("volume-one-vol2-1.0.0.kmpkg").is_file(),
            "written under the second volume's default name"
        );
    }

    /// The gate, both ways round.
    ///
    /// The negative half is the one that matters: the fixture's own `@LENGL` header is enough, so a
    /// song nobody has classified still builds. Without that, the gate would fire on a corpus that
    /// had done nothing wrong, and the first thing anybody would do is turn it off.
    #[test]
    fn a_package_needs_a_language_but_a_detected_one_counts() {
        let scratch = Scratch::new("language-gate");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
        }

        // Nobody has typed a language, and the build goes through: the file said `@LENGL` and that
        // was read.
        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert!(
            report.unlanguaged.is_empty(),
            "a detected language satisfies the gate: {:?}",
            report.unlanguaged
        );
        assert_eq!(report.written, 1);
        let package = km_kmpkg::Package::open(&out).expect("open");
        assert_eq!(package.manifest().songs[0].language.as_deref(), Some("en"));

        // And now a file with no header at all and nothing in its encoding to go on -- the shape
        // most of a real corpus has, where the gate actually bites. The build re-reads and
        // re-analyzes the file, so it has to be the *file* that says nothing, not the database.
        std::fs::write(
            scratch.0.join("plain.mid"),
            km_song::testing::lyric_events(),
        )
        .expect("write a fixture with no @L header");
        run(
            &db,
            ScanOptions::default(),
            &Arc::new(crate::scan::Progress::default()),
        )
        .expect("rescan");

        let plain = {
            let mut guard = db.lock();
            let plain = guard
                .songs(&crate::db::Filter::default())
                .expect("browse")
                .into_iter()
                .find(|row| row.id != id)
                .expect("the second fixture was scanned")
                .id;
            assert_eq!(
                guard.song(&plain).expect("song").det_language_tag,
                None,
                "nothing about this file says a language"
            );
            guard
                .add_to_package(
                    km_kmpkg::EXAMPLE_ID,
                    std::slice::from_ref(&plain),
                    &timestamp(),
                )
                .expect("add");
            plain
        };

        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert_eq!(
            report.unlanguaged.len(),
            1,
            "the song nothing could classify stops the build; got {:?}",
            report.unlanguaged
        );

        // And a person saying so is what unblocks it.
        db.lock()
            .edit_song(
                &plain,
                &crate::db::SongEdit {
                    language: Some(Some("pt".to_owned())),
                    ..crate::db::SongEdit::default()
                },
            )
            .expect("classify it");
        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert!(report.unlanguaged.is_empty(), "{:?}", report.unlanguaged);
        let package = km_kmpkg::Package::open(&out).expect("open");
        let entry = package
            .manifest()
            .songs
            .iter()
            .find(|song| song.language.as_deref() == Some("pt"))
            .expect("the classified song is in the package");
        assert!(
            entry.is_edited(km_kmpkg::EditedField::Language),
            "a typed language is recorded as a correction, so a rebuild keeps it"
        );
    }

    /// A package's own default fills what nobody classified, in the package and nowhere else.
    ///
    /// The second half is the one that would be silent if it broke: the whole promise of this
    /// setting is that it does not quietly answer, on somebody's behalf, a question the curation
    /// tool exists to ask properly.
    #[test]
    fn a_packages_default_language_fills_the_package_and_never_the_corpus() {
        let scratch = Scratch::new("default-language");
        let (db, _) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        // A file with no `@L` header and nothing in its encoding to go on: the shape most of a real
        // corpus has, and the population the gate actually catches.
        std::fs::write(
            scratch.0.join("plain.mid"),
            km_song::testing::lyric_events(),
        )
        .expect("write a fixture with no @L header");
        run(
            &db,
            ScanOptions::default(),
            &Arc::new(crate::scan::Progress::default()),
        )
        .expect("rescan");

        let plain = {
            let mut guard = db.lock();
            let plain = guard
                .songs(&crate::db::Filter::default())
                .expect("browse")
                .into_iter()
                .map(|row| row.id)
                .find(|id| {
                    guard
                        .song(id)
                        .is_ok_and(|detail| detail.det_language_tag.is_none())
                })
                .expect("the fixture with no @L header was scanned");
            package_with(&mut guard, std::slice::from_ref(&plain));
            plain
        };

        // With no default, the gate fires -- the behavior this tool had before the column existed.
        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert_eq!(report.unlanguaged.len(), 1, "{:?}", report.unlanguaged);

        // With one, the package builds and the song goes in under it.
        {
            let guard = db.lock();
            let mut row = guard.package(km_kmpkg::EXAMPLE_ID).expect("package");
            row.default_language = Some("en".to_owned());
            guard.update_package(&row).expect("set the default");
        }

        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert!(report.unlanguaged.is_empty(), "{:?}", report.unlanguaged);
        let package = km_kmpkg::Package::open(&out).expect("open");
        let entry = &package.manifest().songs[0];
        assert_eq!(entry.language.as_deref(), Some("en"));
        assert!(
            !entry.is_edited(km_kmpkg::EditedField::Language),
            "a blanket default is not a correction somebody made"
        );

        // And the corpus is untouched: nobody has said what this song is in, and the tool must not
        // start claiming otherwise because a package needed an answer.
        assert_eq!(
            db.lock().song(&plain).expect("song").language,
            None,
            "the build wrote a language into the package, not into the database"
        );
    }

    /// The property the module documentation rests on, asserted rather than only described.
    ///
    /// The description carries the *effective* title so the file is worth opening, and that is only
    /// safe because the build re-derives the same fallback chain and therefore compares equal. If
    /// the two chains ever drift, every song in a corpus is silently marked hand-edited — which
    /// fails nothing and costs the flag its meaning. This is what would catch it.
    #[test]
    fn the_written_description_records_no_correction_nobody_made() {
        let scratch = Scratch::new("provenance");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));

            // Nobody has typed anything, so the description says what the file says...
            let curated = spec_for(&guard, km_kmpkg::EXAMPLE_ID, 1).expect("describe");
            assert!(
                curated.spec.songs[0]
                    .title
                    .as_deref()
                    .is_some_and(|title| !title.is_empty()),
                "a description of bare paths is not worth opening"
            );
        }

        // ...and building it therefore records no correction, because the build works out the same
        // title for itself from the same bytes.
        build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        let package = km_kmpkg::Package::open(&out).expect("open");
        assert!(
            package.manifest().songs[0].edited.is_empty(),
            "nothing was corrected: {:?}",
            package.manifest().songs[0].edited
        );

        // Typing one puts it in the description, and building marks exactly that field.
        {
            let guard = db.lock();
            guard
                .edit_song(
                    &id,
                    &crate::db::SongEdit {
                        title: Some(Some("A Typed Title".to_owned())),
                        ..crate::db::SongEdit::default()
                    },
                )
                .expect("type a title");
            let curated = spec_for(&guard, km_kmpkg::EXAMPLE_ID, 1).expect("describe");
            assert_eq!(
                curated.spec.songs[0].title.as_deref(),
                Some("A Typed Title")
            );
        }

        build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        let package = km_kmpkg::Package::open(&out).expect("open");
        let entry = &package.manifest().songs[0];
        assert_eq!(entry.title, "A Typed Title");
        assert!(entry.is_edited(km_kmpkg::EditedField::Title));
        assert!(
            !entry.is_edited(km_kmpkg::EditedField::Artist),
            "and only that field: {:?}",
            entry.edited
        );
    }

    /// A tag reaches a package only because a song carries it — a suggestion never does.
    ///
    /// **The property that makes the suggested vocabulary a hint rather than a commitment.** The
    /// settings file offers `pop`, `rock` and `classic-rock` in every picker; this asserts that the
    /// mechanism by which they could leak into a package does not exist, because `spec_for` reads
    /// `song_tags` and a word nobody has put on a song has no row there. It also covers the remotes
    /// by construction: a catalog is built from packages, so a tag in no package is in no mirror.
    #[test]
    fn a_tag_reaches_a_package_only_because_a_song_carries_it() {
        let scratch = Scratch::new("tags-into-package");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));

            // The suggestions are offerable and nothing here carries one.
            let settings = crate::settings::Settings::default();
            let present = guard.tags_present().expect("vocabulary");
            assert!(present.is_empty());
            assert_eq!(settings.hints(&present), ["classic-rock", "pop", "rock"]);

            let curated = spec_for(&guard, km_kmpkg::EXAMPLE_ID, 1).expect("describe");
            assert!(
                curated.spec.songs[0].tags.is_empty(),
                "a description carries what songs are filed under, not what is suggested"
            );
        }

        build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert!(
            km_kmpkg::Package::open(&out)
                .expect("open")
                .manifest()
                .songs[0]
                .tags
                .is_empty()
        );

        // Put one on the song, and it travels — including the fact that a person set it, which is
        // what stops the next rebuild from source dropping it.
        {
            let guard = db.lock();
            let tag = km_kmpkg::Tag::parse("rock").expect("a tag");
            guard
                .add_tag_of(std::slice::from_ref(&id), &tag)
                .expect("tag it");
            assert_eq!(
                spec_for(&guard, km_kmpkg::EXAMPLE_ID, 1)
                    .expect("describe")
                    .spec
                    .songs[0]
                    .tags,
                ["rock"]
            );
        }

        build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        let package = km_kmpkg::Package::open(&out).expect("open");
        let entry = &package.manifest().songs[0];
        assert_eq!(entry.tags, ["rock"]);
        assert!(
            entry.is_edited(km_kmpkg::EditedField::Tags),
            "nothing detects a tag, so any tag is hand curation: {:?}",
            entry.edited
        );
    }

    /// A member whose file has gone is reported by number rather than losing the whole build.
    #[test]
    fn a_member_whose_file_is_gone_is_reported_and_the_rest_still_builds() {
        let scratch = Scratch::new("missing");
        let (db, id) = corpus(&scratch);

        let mut guard = db.lock();
        package_with(&mut guard, std::slice::from_ref(&id));

        // The row survives the file, which is the state a scan leaves behind after a delete.
        std::fs::remove_file(scratch.0.join("song.kar")).expect("delete the source");
        drop(guard);
        run(
            &db,
            ScanOptions::default(),
            &Arc::new(crate::scan::Progress::default()),
        )
        .expect("rescan");

        let guard = db.lock();
        let curated = spec_for(&guard, km_kmpkg::EXAMPLE_ID, 1).expect("describe");
        assert!(curated.spec.songs.is_empty());
        assert_eq!(curated.missing.len(), 1);
        assert_eq!(curated.missing[0].0, 1, "reported by its song number");
    }

    /// Writing the description and building it gives the package this page would have built.
    ///
    /// This is the property the whole arrangement exists for, and the one that would rot silently:
    /// nothing fails if the two drift, the packages just quietly stop matching. Asserted over the
    /// manifest rather than the bytes, because `created` is a timestamp and the zip is not
    /// reproducible.
    #[test]
    fn the_written_description_builds_the_same_package_this_page_would() {
        let scratch = Scratch::new("round-trip");
        let (db, id) = corpus(&scratch);

        let spec_path = scratch.0.join("vol1.kmspec.yaml");
        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
            guard.set_user_score(&id, Some(7)).expect("judge");
            guard
                .edit_song(
                    &id,
                    &crate::db::SongEdit {
                        title: Some(Some("A Typed Title".to_owned())),
                        ..crate::db::SongEdit::default()
                    },
                )
                .expect("type a title");
            write_spec(&guard, km_kmpkg::EXAMPLE_ID, 1, &spec_path).expect("describe");
        }

        // What this page builds.
        let here = scratch.0.join("here.kmpkg");
        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &here);
        assert_eq!(report.written, 1, "{:?}", report.skipped);

        // And what the description it wrote builds, through km-pack's own reader.
        let spec = km_pack::Spec::read(&spec_path).expect("read it back");
        let there = scratch.0.join("there.kmpkg");
        km_pack::build::build(
            &spec,
            &km_pack::BuildOptions {
                base: &spec.base(&spec_path),
                out: Some(&there),
                dry_run: false,
                measure_loudness: true,
                write_listing: false,
            },
            |_| std::ops::ControlFlow::Continue(()),
        )
        .expect("build the description");

        let mine = km_kmpkg::Package::open(&here).expect("open");
        let theirs = km_kmpkg::Package::open(&there).expect("open");
        assert_eq!(
            mine.manifest().songs,
            theirs.manifest().songs,
            "the two routes have to agree about every song, or the description is a second truth"
        );
        assert_eq!(mine.manifest().songs[0].title, "A Typed Title");
        assert!(mine.manifest().songs[0].is_edited(km_kmpkg::EditedField::Title));
        // The song was rated a 7 above, and neither route says so: a description carries no
        // hand-set rating, so there is nothing for the two to agree or disagree about.
        let text = std::fs::read_to_string(&spec_path).expect("read the description");
        assert!(!text.contains("suitability"), "got {text}");
    }

    /// A row standing in for one the database would hand back.
    fn built_row(name: &str, version: &str, built: bool) -> crate::model::PackageRow {
        crate::model::PackageRow {
            id: "1f4a9c8e2b7d0356".to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            publisher: None,
            start_number: 1,
            default_language: Some("en".to_owned()),
            out_path: None,
            built_at: built.then(|| "2026-09-10T00:00:00Z".to_owned()),
            song_count: 3,
            ..crate::model::PackageRow::new("1f4a9c8e2b7d0356", "")
        }
    }

    /// A package and its description land in the corpus's data folder, and not loose in the root.
    ///
    /// Both in the same test because the pair have to agree: the description is written "beside the
    /// package", and a spec that went somewhere else would be a second answer to where output goes.
    #[test]
    fn the_default_output_sits_in_the_corpus_data_folder() {
        let root = Path::new("/corpus");
        let row = built_row("Brasil Volume 1", "1.0.0", false);
        let package = default_out_path(root, &row, true);
        let spec = default_spec_path(root, &row);

        assert_eq!(
            package,
            crate::db::data_dir(root).join("brasil-volume-1-1.0.0.kmpkg")
        );
        // Named for the package and not for its id, so the two files in that folder are recognizable
        // as the same package's. The version belongs to the build and not to the description.
        assert_eq!(
            spec,
            crate::db::data_dir(root).join("brasil-volume-1.kmspec.yaml")
        );
        assert_eq!(
            package.parent(),
            spec.parent(),
            "a description is written beside the package it describes"
        );
        // And not in the root, which is where hundreds of thousands of songs are.
        assert_ne!(package.parent(), Some(root));
    }

    /// A package of one volume set to number it writes `vol1` into both file names from the first
    /// build, which are the names its first volume keeps when a second starts.
    #[test]
    fn a_numbered_single_volume_names_its_files_as_volume_one() {
        let root = Path::new("/corpus");
        let row = crate::model::PackageRow {
            number_one_volume: true,
            ..built_row("Brasil", "1.0.0", false)
        };
        assert_eq!(row.volume_name(), "Brasil vol1");
        assert_eq!(
            default_out_path(root, &row, true),
            crate::db::data_dir(root).join("brasil-vol1-1.0.0.kmpkg")
        );
        assert_eq!(
            default_spec_path(root, &row),
            crate::db::data_dir(root).join("brasil-vol1.kmspec.yaml")
        );
    }

    /// The name on the box is the file the build will actually write.
    ///
    /// The raise happens before the write, so a default built from the *stored* version would put a
    /// name on the form that no file on disk ever had — which is the thing the label beside it
    /// already goes out of its way to avoid.
    #[test]
    fn the_default_name_carries_the_version_the_build_will_write() {
        let root = Path::new("/corpus");
        let named = |row: &crate::model::PackageRow, raise: bool| {
            default_out_path(root, row, raise)
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned()
        };

        // A package that has never been built ships at the version somebody typed, box or no box.
        let first = built_row("Brasil Volume 1", "1.0.0", false);
        assert_eq!(named(&first, true), "brasil-volume-1-1.0.0.kmpkg");

        // A rebuild with the box ticked names the version it is about to raise to; unticked, the one
        // it will keep.
        let again = built_row("Brasil Volume 1", "1.0.0", true);
        assert_eq!(named(&again, true), "brasil-volume-1-1.0.1.kmpkg");
        assert_eq!(named(&again, false), "brasil-volume-1-1.0.0.kmpkg");

        // A version this tool cannot raise still names a file.
        let odd = built_row("Brasil Volume 1", "2024-spring", true);
        assert_eq!(named(&odd, true), "brasil-volume-1-2024-spring.kmpkg");

        // A name that folds away to nothing falls back to the id, which is always a legal name.
        let nameless = built_row("日本の歌", "1.0.0", false);
        assert_eq!(named(&nameless, true), "1f4a9c8e2b7d0356-1.0.0.kmpkg");
    }

    /// What the version of a package on disk is, so a test asserts the file rather than the row.
    fn version_of(out: &Path) -> String {
        km_kmpkg::Package::open(out)
            .expect("open")
            .manifest()
            .package
            .version
            .clone()
    }

    /// What the row says the version is.
    fn stored_version(db: &Arc<Shared>, package_id: &str) -> String {
        db.lock().package(package_id).expect("package").version
    }

    /// The whole of the feature, in the order somebody meets it.
    ///
    /// **Both the file and the row, every time.** The two agreeing is the property — a version
    /// raised into the description but not recorded, or recorded but not built, would leave the
    /// field on the page naming a package that does not exist. Nothing else would notice.
    #[test]
    fn the_first_build_keeps_the_version_and_every_build_after_it_raises_the_patch() {
        let scratch = Scratch::new("raise-version");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
            assert!(
                guard.raise_version(km_kmpkg::EXAMPLE_ID).expect("the flag"),
                "a new package arrives with the box ticked"
            );
        }

        // The first build ships at the version somebody typed. A package whose only build had
        // already moved the number would never exist at the version its creator chose.
        assert_eq!(build_now(&db, km_kmpkg::EXAMPLE_ID, &out).version, "1.0.0");
        assert_eq!(version_of(&out), "1.0.0");
        assert_eq!(stored_version(&db, km_kmpkg::EXAMPLE_ID), "1.0.0");

        // Every build after it raises first, so the row keeps naming the file on disk.
        for expected in ["1.0.1", "1.0.2", "1.0.3"] {
            assert_eq!(build_now(&db, km_kmpkg::EXAMPLE_ID, &out).version, expected);
            assert_eq!(version_of(&out), expected);
            assert_eq!(stored_version(&db, km_kmpkg::EXAMPLE_ID), expected);
        }

        // The minor is untouched throughout, which is what leaves it meaning something a person
        // chose rather than a count of rebuilds.
        assert_eq!(
            crate::version::parse(&stored_version(&db, km_kmpkg::EXAMPLE_ID))
                .expect("a version this tool wrote is one it can read")
                .minor,
            0
        );
    }

    /// A build that wrote nothing raises nothing.
    ///
    /// The number moves in the same lock that records the build, so a refusal cannot spend one.
    /// Without this a corpus would drift its versions forward every time somebody hit the language
    /// gate, and the field would stop naming any file that exists.
    #[test]
    fn a_build_that_wrote_nothing_raises_nothing() {
        let scratch = Scratch::new("raise-refused");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
        }
        build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert_eq!(stored_version(&db, km_kmpkg::EXAMPLE_ID), "1.0.0");

        // A song nothing can classify stops the build, the way the language gate's own test sets
        // up. The package on disk stays at 1.0.0, so the row has to as well.
        std::fs::write(
            scratch.0.join("plain.mid"),
            km_song::testing::lyric_events(),
        )
        .expect("write a fixture with no @L header");
        run(
            &db,
            ScanOptions::default(),
            &Arc::new(crate::scan::Progress::default()),
        )
        .expect("rescan");
        {
            let mut guard = db.lock();
            let plain = guard
                .songs(&crate::db::Filter::default())
                .expect("browse")
                .into_iter()
                .map(|row| row.id)
                .find(|other| {
                    guard
                        .song(other)
                        .is_ok_and(|detail| detail.det_language_tag.is_none())
                })
                .expect("the fixture with no @L header was scanned");
            guard
                .add_to_package(
                    km_kmpkg::EXAMPLE_ID,
                    std::slice::from_ref(&plain),
                    &timestamp(),
                )
                .expect("add");
        }

        let report = build_now(&db, km_kmpkg::EXAMPLE_ID, &out);
        assert_eq!(report.unlanguaged.len(), 1, "{:?}", report.unlanguaged);
        assert_eq!(
            stored_version(&db, km_kmpkg::EXAMPLE_ID),
            "1.0.0",
            "a refused build spends no number"
        );
        assert_eq!(version_of(&out), "1.0.0");
    }

    /// An unticked box is the whole of turning it off.
    #[test]
    fn an_unticked_box_leaves_the_version_alone() {
        let scratch = Scratch::new("raise-off");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
            guard
                .set_raise_version(km_kmpkg::EXAMPLE_ID, false)
                .expect("untick");
        }

        for _ in 0..3 {
            assert_eq!(build_now(&db, km_kmpkg::EXAMPLE_ID, &out).version, "1.0.0");
        }
        assert_eq!(stored_version(&db, km_kmpkg::EXAMPLE_ID), "1.0.0");
        assert_eq!(version_of(&out), "1.0.0");
    }

    /// A version this tool would refuse to be told still builds, and is left exactly as it is.
    ///
    /// **The import path, reached the way `import` reaches it.** A `.kmpkg` built elsewhere may
    /// carry any string at all, and the answer is to build it and say the version cannot be raised
    /// — not to refuse the package or to overwrite what somebody called it.
    #[test]
    fn a_version_this_tool_would_not_accept_builds_and_is_left_alone() {
        let scratch = Scratch::new("raise-odd");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
            let mut row = guard.package(km_kmpkg::EXAMPLE_ID).expect("package");
            row.version = "2024-spring".to_owned();
            guard.update_package(&row).expect("as an import would");
        }

        assert!(crate::version::parse("2024-spring").is_none());
        for _ in 0..2 {
            assert_eq!(
                build_now(&db, km_kmpkg::EXAMPLE_ID, &out).version,
                "2024-spring"
            );
        }
        assert_eq!(stored_version(&db, km_kmpkg::EXAMPLE_ID), "2024-spring");
        assert_eq!(version_of(&out), "2024-spring");
    }

    /// Writing a description raises nothing: it says what the package is, not what a build made it.
    #[test]
    fn writing_a_description_raises_no_version() {
        let scratch = Scratch::new("raise-spec");
        let (db, id) = corpus(&scratch);
        let out = scratch.0.join("vol1.kmpkg");
        let spec_path = scratch.0.join("vol1.kmspec.yaml");

        {
            let mut guard = db.lock();
            package_with(&mut guard, std::slice::from_ref(&id));
        }
        // Built once, so the package is past the first-build exemption and a raise is live.
        build_now(&db, km_kmpkg::EXAMPLE_ID, &out);

        {
            let guard = db.lock();
            write_spec(&guard, km_kmpkg::EXAMPLE_ID, 1, &spec_path).expect("describe");
            write_spec(&guard, km_kmpkg::EXAMPLE_ID, 1, &spec_path).expect("describe again");
        }
        assert_eq!(stored_version(&db, km_kmpkg::EXAMPLE_ID), "1.0.0");
        assert_eq!(
            km_pack::Spec::read(&spec_path)
                .expect("read it back")
                .package
                .version,
            "1.0.0",
            "a description says what the package is, and writing one is not a build"
        );

        // ...and the next real build still raises, so writing a description did not consume it.
        assert_eq!(build_now(&db, km_kmpkg::EXAMPLE_ID, &out).version, "1.0.1");
    }

    #[test]
    fn an_empty_report_knows_it_is_empty() {
        assert!(BuildReport::default().is_empty());
        assert!(
            !BuildReport {
                written: 1,
                ..BuildReport::default()
            }
            .is_empty()
        );
    }
}
