//! The `.kmpkg` song package.
//!
//! A package holds a [`manifest::Manifest`] and the songs it describes, in a container of this
//! project's own — see [`container`]. It is the only way songs get into the machine, apart from the
//! debug path that loads a single file.
//!
//! Packages are read **in place**, never extracted. A karaoke MIDI file is a few kilobytes, so
//! opening the package to pull one out costs nothing worth optimizing, and it avoids keeping a second
//! copy of every song on disk and a cache to keep in step.
//!
//! Two rules the reader does not bend: a path inside the package that escapes it is refused rather
//! than followed, and a manifest from a newer format version is refused rather than half-understood.

mod container;
pub mod language;
pub mod manifest;
pub mod tag;

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::container::{CappedRead, Container, ContainerError, Method, Writer};

pub use crate::language::{Language, TABLE_REVISION as LANGUAGE_TABLE_REVISION};
pub use crate::manifest::EditedField;
pub use crate::manifest::{
    BreakdownRecord, EXAMPLE_ID, FORMAT_VERSION, FORMAT_VERSION_LRC, FORMAT_VERSION_MEDIA,
    FORMAT_VERSION_MIDI_ONLY, FORMAT_VERSION_ULTRASTAR, FORMAT_VERSIONS_READ, GENERATED_ID_CHARS,
    LoudnessRecord, MANIFEST_PATH, Manifest, ManifestProblem, MelodyRecord, PackageMeta, SongEntry,
    SongKind, SuitabilityRecord, VolumeOf, WarningRecord, is_safe_name, is_safe_path, name_slug,
};
pub use crate::tag::Tag;

/// The most of `manifest.json` this build will hold in memory.
///
/// **A package's own limits put a ceiling on an honest manifest**: at most
/// [`km_songcode::MAX_SLOT`] songs, each a title, an artist, a path and a handful of small records.
/// A few hundred kilobytes covers a full one, and this leaves three orders of magnitude over that.
///
/// The number exists because the manifest is *decompressed* before anything has decided the package
/// is real, and deflate reaches roughly a thousand to one — so a `.kmpkg` well inside the upload
/// limit can carry a manifest that exhausts memory. **Every start reads every package in the scanned
/// folders**, which is what makes this worth a constant: one such file is otherwise a machine that
/// stops booting rather than a song that will not play.
pub const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

/// The most of one MIDI entry this build will hold in memory.
///
/// A karaoke MIDI is kilobytes. This is far above any real one and far below a size that would
/// trouble the television.
pub const MAX_SONG_BYTES: u64 = 32 * 1024 * 1024;

/// The most of one `.cdg` entry this build will hold in memory.
///
/// CD+G runs at a fixed 300 packets of 24 bytes a second, so a *ten-hour* graphics stream would
/// still fit here. Separate from [`MAX_SONG_BYTES`] because the two differ by three orders of
/// magnitude and one number covering both would have to be the larger.
pub const MAX_GRAPHICS_BYTES: u64 = 256 * 1024 * 1024;

/// The most of one lyric timeline entry this build will hold in memory.
///
/// A timeline is a few hundred syllables of JSON, tens of kilobytes. The ceiling is
/// [`MAX_SONG_BYTES`]'s, for the reason that one gives.
pub const MAX_LYRICS_BYTES: u64 = MAX_SONG_BYTES;

/// Reads `manifest.json` as text, refusing one that expands past [`MAX_MANIFEST_BYTES`].
///
/// Shared by [`Package::open`] and the diagnostic read, because the ceiling has to be the same one:
/// a manifest that stops the machine booting is exactly the manifest somebody would then reach for
/// `km-pack check` to look at.
fn read_manifest_bytes<R: Read + Seek>(
    container: &mut Container<R>,
    display: &str,
) -> Result<String, PackageError> {
    let entry = container
        .entry(MANIFEST_PATH)
        .ok_or_else(|| PackageError::NoManifest(display.to_owned()))?
        .clone();
    let bytes = container
        .read_entry(&entry, MAX_MANIFEST_BYTES)
        .map_err(|error| match error {
            CappedRead::TooLarge => PackageError::EntryTooLarge {
                path: display.to_owned(),
                entry: MANIFEST_PATH.to_owned(),
                limit: MAX_MANIFEST_BYTES,
            },
            CappedRead::Io(source) => PackageError::Io {
                path: display.to_owned(),
                source,
            },
        })?;
    String::from_utf8(bytes).map_err(|error| PackageError::Io {
        path: display.to_owned(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, error),
    })
}

/// Opens the container at `path`, turning its refusals into a package's.
fn open_container(path: &Path, display: &str) -> Result<Container<std::fs::File>, PackageError> {
    let file = std::fs::File::open(path).map_err(|source| PackageError::Io {
        path: display.to_owned(),
        source,
    })?;
    Container::open(file).map_err(|error| container_error(error, display))
}

/// What a container's refusal means about the package it was read from.
fn container_error(error: ContainerError, display: &str) -> PackageError {
    match error {
        ContainerError::Io(source) => PackageError::Io {
            path: display.to_owned(),
            source,
        },
        ContainerError::NotAPackage => PackageError::NotAPackage(display.to_owned()),
        ContainerError::UnsupportedContainer(version) => PackageError::NewerPackage {
            path: display.to_owned(),
            version,
        },
        ContainerError::Malformed(message) => PackageError::Archive {
            path: display.to_owned(),
            message,
        },
    }
}

/// Why a package could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    /// The file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file involved.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The package's structure does not hold together.
    #[error("{path} is not a readable package: {message}")]
    Archive {
        /// The file involved.
        path: String,
        /// What is wrong with it.
        message: String,
    },
    /// It does not begin the way a package does.
    #[error("{0} is not a .kmpkg")]
    NotAPackage(String),
    /// It is a package a newer build wrote.
    #[error("{path} was written by a newer version of this program (container {version})")]
    NewerPackage {
        /// The file involved.
        path: String,
        /// The container it names.
        version: u16,
    },
    /// The package has no manifest.
    #[error("{0} has no {MANIFEST_PATH}; is it a .kmpkg?")]
    NoManifest(String),
    /// The manifest is not valid JSON, or does not match the schema.
    #[error("{path}: could not read {MANIFEST_PATH}: {source}")]
    Manifest {
        /// The file involved.
        path: String,
        /// The parse error.
        source: serde_json::Error,
    },
    /// The manifest is structurally wrong.
    #[error("{path} is not usable: {}", format_problems(problems))]
    Invalid {
        /// The file involved.
        path: String,
        /// Everything wrong with it.
        problems: Vec<ManifestProblem>,
    },
    /// An entry expanded past what this build will hold in memory for it.
    ///
    /// **A compressed entry can be very much larger than the file holding it**, so the archive's own
    /// declared size is not what this is measured against — the bytes are counted as they arrive,
    /// and the read stops. Reported rather than truncated: half a song is not a song, and a package
    /// that carries one is a package to say something about.
    #[error("{path}: {entry} is larger than this build will read for it ({limit} bytes)")]
    EntryTooLarge {
        /// The file involved.
        path: String,
        /// The entry that ran over.
        entry: String,
        /// The ceiling it ran over.
        limit: u64,
    },
    /// No song with that number.
    #[error("no song numbered {0} in this package")]
    NoSuchSong(u32),
    /// The manifest names a file the archive does not contain.
    #[error("song {number} names {file}, which is not in the archive")]
    MissingEntry {
        /// The song number.
        number: u32,
        /// The path it named.
        file: String,
    },
    /// A media entry was asked for as MIDI, or the other way about.
    ///
    /// A caller reaching this took the wrong branch: MIDI comes back whole from
    /// [`Package::read_song`], and media is seeked into through [`Package::media_reader`].
    #[error("song {number} is not that kind of song: {file}")]
    NotMedia {
        /// The song number.
        number: u32,
        /// The entry it named.
        file: String,
    },
    /// An entry a decoder seeks into is compressed, so there is no byte range to seek into.
    ///
    /// Video and MP3+G audio are written uncompressed, because that is what lets a decoder read the
    /// package as a file from an offset instead of extracting it. A deflated one would have to be
    /// inflated from its start to reach its middle, which is the whole thing this avoids.
    ///
    /// **Not every media entry** — the `.cdg` is read whole and is deflated on purpose. See
    /// [`is_seekable_entry`], which is the one place that line is drawn.
    #[error(
        "song {number}'s media {file} is compressed; an entry the decoder seeks into must be \
         stored uncompressed so it can be played by seeking into it"
    )]
    NotStored {
        /// The song number.
        number: u32,
        /// The entry it named.
        file: String,
    },
    /// A file changed size while it was being packaged.
    ///
    /// Caught by comparing what was copied against what the source measured, because the mismatch
    /// is otherwise silent: the entry and the manifest would disagree about a song nobody has played
    /// yet, and had the file grown past four gibibytes the `large_file` decision made from the old
    /// length would already have been the wrong one.
    #[error(
        "{path} changed while it was being packaged: expected {expected} bytes, copied {copied}"
    )]
    SourceChanged {
        /// The source file involved.
        path: String,
        /// What it measured before the copy.
        expected: u64,
        /// What the copy actually moved.
        copied: u64,
    },
    /// A song number was used twice while building.
    #[error("song number {0} was added twice")]
    DuplicateNumber(u32),
    /// An UltraStar song's lyric timeline entry is not a timeline.
    #[error("{path}: song {number}'s lyric timeline could not be read: {source}")]
    Lyrics {
        /// The file involved.
        path: String,
        /// The song number.
        number: u32,
        /// Why the entry is not a timeline.
        source: serde_json::Error,
    },
}

fn format_problems(problems: &[ManifestProblem]) -> String {
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// An open package.
///
/// Holds the manifest in memory and the path to the archive; song bytes are read on demand.
#[derive(Debug, Clone)]
pub struct Package {
    path: PathBuf,
    manifest: Manifest,
}

impl Package {
    /// Opens a package and reads its manifest.
    ///
    /// The manifest is validated here, so a package that would misbehave later is rejected at the
    /// point somebody can still do something about it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PackageError> {
        let path = path.as_ref();
        let display = path.display().to_string();

        let mut container = open_container(path, &display)?;
        let json = read_manifest_bytes(&mut container, &display)?;

        let manifest: Manifest =
            serde_json::from_str(&json).map_err(|source| PackageError::Manifest {
                path: display.clone(),
                source,
            })?;

        let problems = manifest.problems();
        if !problems.is_empty() {
            return Err(PackageError::Invalid {
                path: display,
                problems,
            });
        }

        Ok(Self {
            path: path.to_path_buf(),
            manifest,
        })
    }

    /// The archive's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// How many songs it holds.
    pub fn len(&self) -> usize {
        self.manifest.songs.len()
    }

    /// Whether it holds no songs. Cannot happen for an opened package, which is validated.
    pub fn is_empty(&self) -> bool {
        self.manifest.songs.is_empty()
    }

    /// Reads one song's MIDI bytes.
    ///
    /// Only ever a MIDI song: this returns the whole entry in memory, which is right for a few
    /// kilobytes of MIDI and wrong for a video. Media lives in the same archive now, so the
    /// distinction is no longer about *where* the bytes are but about *how many* — see
    /// [`Package::media_reader`], which seeks into an entry rather than reading it.
    pub fn read_song(&self, number: u32) -> Result<Vec<u8>, PackageError> {
        let entry = self
            .manifest
            .song(number)
            .ok_or(PackageError::NoSuchSong(number))?;

        if !entry.kind.is_midi() {
            return Err(PackageError::NotMedia {
                number,
                file: entry.file.clone(),
            });
        }

        self.read_entry_whole(number, &entry.file.clone(), MAX_SONG_BYTES)
    }

    /// Checks that every entry the manifest names is actually in the archive.
    ///
    /// One rule for every kind of song, which is the point of media living in the package: there is
    /// no second place to look and nothing that can be missing from one of them and present in the
    /// other.
    ///
    /// Not done on open, because it means walking the whole directory; a packaging tool wants it, a
    /// machine loading one song does not.
    pub fn missing_entries(&self) -> Result<Vec<u32>, PackageError> {
        let display = self.path.display().to_string();
        let container = open_container(&self.path, &display)?;

        Ok(self
            .manifest
            .songs
            .iter()
            .filter(|song| {
                !is_safe_path(&song.file)
                    || !container.contains(&song.file)
                    // **Both halves, because half a pair is not a degraded song.** An MP3 with no
                    // `.cdg` or no timeline has no words in it at all, which is the same reason a
                    // bare audio file is not a song source.
                    || companion_entry_for(song.kind, &song.file)
                        .is_some_and(|companion| !container.contains(&companion))
                    // Written by a newer build. Its files cannot be looked for, because what they
                    // are is exactly what is not understood.
                    || matches!(song.kind, SongKind::Unknown)
            })
            .map(|song| song.number)
            .collect())
    }

    /// A seekable window over one song's media entry: a video, or an MP3+G song's audio.
    ///
    /// **Nothing is extracted and nothing is held in memory.** The archive is opened to learn where
    /// the entry's bytes begin and how many there are, and is then dropped; what survives is the
    /// file handle, an offset and a length. A decoder reads the package *as a file*, from an offset.
    ///
    /// This is why every media entry is stored uncompressed. A deflated entry has no byte range to
    /// seek into — you would have to inflate from the start to reach the middle — so one is refused
    /// here rather than silently read wrong.
    pub fn media_reader(&self, number: u32) -> Result<EntryWindow, PackageError> {
        let entry = self
            .manifest
            .song(number)
            .ok_or(PackageError::NoSuchSong(number))?;
        if entry.kind.is_midi() {
            return Err(PackageError::NotMedia {
                number,
                file: entry.file.clone(),
            });
        }
        self.window(number, &entry.file.clone())
    }

    /// An MP3+G song's graphics, read whole.
    ///
    /// **Whole, deliberately**, and it is the same argument [`Package::read_song`] makes about MIDI
    /// rather than a different one: a six-minute `.cdg` is 2.6 MB, and `km-cdg` replays it from
    /// packet zero on every seek and so holds all of it anyway. A window would buy nothing and be a
    /// second shape to keep in step.
    ///
    /// **This is why the `.cdg` is the one media entry that is deflated** — see [`media_options`].
    /// Reading it through the archive rather than through [`Package::window`] is what allows that,
    /// and it costs nothing here: an entry that is read from its start has no use for a byte range.
    /// It also takes either method without asking, so an entry carried across a rebuild by
    /// `raw_copy_file`, which preserves whatever it found, still reads.
    pub fn graphics_bytes(&self, number: u32) -> Result<Vec<u8>, PackageError> {
        let entry = self
            .manifest
            .song(number)
            .ok_or(PackageError::NoSuchSong(number))?;
        if !entry.kind.is_cdg() {
            return Err(PackageError::NotMedia {
                number,
                file: entry.file.clone(),
            });
        }
        self.read_entry_whole(number, &graphics_entry_for(&entry.file), MAX_GRAPHICS_BYTES)
    }

    /// An UltraStar or LRC song's lyric timeline, in milliseconds from the start of its audio.
    ///
    /// Read whole, as [`Package::graphics_bytes`] reads a `.cdg`, and parsed here so that no caller
    /// holds the stored form. The ticks are milliseconds: pair it with
    /// `km_song::recording::TICKS_PER_SECOND`.
    pub fn lyric_timeline(&self, number: u32) -> Result<km_song::LyricTimeline, PackageError> {
        let entry = self
            .manifest
            .song(number)
            .ok_or(PackageError::NoSuchSong(number))?;
        if !entry.kind.carries_timeline() {
            return Err(PackageError::NotMedia {
                number,
                file: entry.file.clone(),
            });
        }
        let bytes =
            self.read_entry_whole(number, &lyrics_entry_for(&entry.file), MAX_LYRICS_BYTES)?;
        serde_json::from_slice(&bytes).map_err(|source| PackageError::Lyrics {
            path: self.path.display().to_string(),
            number,
            source,
        })
    }

    /// One entry read from its start, whatever it is compressed with.
    ///
    /// The whole-read counterpart to [`Package::window`], and shared by [`Package::read_song`] and
    /// [`Package::graphics_bytes`] so that the safe-path re-check and the sizing hint exist once.
    fn read_entry_whole(
        &self,
        number: u32,
        name: &str,
        limit: u64,
    ) -> Result<Vec<u8>, PackageError> {
        let display = self.path.display().to_string();

        // Re-checked here rather than trusted from validation, because this is the point where a
        // bad path would actually be followed.
        if !is_safe_path(name) {
            return Err(PackageError::Invalid {
                path: display,
                problems: vec![ManifestProblem::UnsafePath {
                    number,
                    path: name.to_owned(),
                }],
            });
        }

        let mut container = open_container(&self.path, &display)?;
        let entry = container
            .entry(name)
            .ok_or_else(|| PackageError::MissingEntry {
                number,
                file: name.to_owned(),
            })?
            .clone();

        container
            .read_entry(&entry, limit)
            .map_err(|error| match error {
                CappedRead::TooLarge => PackageError::EntryTooLarge {
                    path: display,
                    entry: name.to_owned(),
                    limit,
                },
                CappedRead::Io(source) => PackageError::Io {
                    path: display,
                    source,
                },
            })
    }

    /// Every media entry the archive holds, with its size and how it is stored.
    ///
    /// For `km-pack check`, which is where the "an entry a decoder seeks into is stored" rule stops
    /// being an intention and becomes something a person can verify — pair `stored` with
    /// [`is_seekable_entry`] to ask whether an entry is stored *that must be*. Reads the directory
    /// only — the lengths and methods are already there — so it costs one walk and no reads.
    pub fn media_entries(&self) -> Result<Vec<MediaEntry>, PackageError> {
        let display = self.path.display().to_string();
        let container = open_container(&self.path, &display)?;

        let mut names: Vec<String> = Vec::new();
        for song in &self.manifest.songs {
            if song.kind.is_midi() || !is_safe_path(&song.file) {
                continue;
            }
            names.push(song.file.clone());
            names.extend(companion_entry_for(song.kind, &song.file));
        }

        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            let Some(inner) = container.entry(&name) else {
                continue;
            };
            entries.push(MediaEntry {
                stored: inner.method == Method::Stored,
                size: inner.real_len,
                name,
            });
        }
        Ok(entries)
    }

    /// The shared body of [`Package::media_reader`] and anything else that seeks into an entry.
    fn window(&self, number: u32, name: &str) -> Result<EntryWindow, PackageError> {
        let display = self.path.display().to_string();

        // Re-checked here rather than trusted from validation, for the reason `read_song` gives:
        // this is the point where a bad name would actually be followed.
        if !is_safe_path(name) {
            return Err(PackageError::Invalid {
                path: display,
                problems: vec![ManifestProblem::UnsafePath {
                    number,
                    path: name.to_owned(),
                }],
            });
        }

        let container = open_container(&self.path, &display)?;

        // The directory is the only table, so the byte range is simply read from it — there is no
        // second copy of a length for this to have to agree with, and a package whose entries do not
        // fit inside it was refused when it was opened.
        let (start, len) = {
            let inner = container
                .entry(name)
                .ok_or_else(|| PackageError::MissingEntry {
                    number,
                    file: name.to_owned(),
                })?;
            if inner.method != Method::Stored {
                return Err(PackageError::NotStored {
                    number,
                    file: name.to_owned(),
                });
            }
            (inner.offset, inner.stored_len)
        };

        // The same handle the directory was read through, so there is one open and no gap in which
        // the file could be replaced between learning the offset and reading from it.
        let mut file = container.into_inner();

        file.seek(std::io::SeekFrom::Start(start))
            .map_err(|source| PackageError::Io {
                path: display,
                source,
            })?;
        Ok(EntryWindow {
            file,
            start,
            len,
            pos: 0,
            name: name.to_owned(),
        })
    }
}

/// An owned, seekable window over one stored entry of a package.
///
/// **The point of it is the lifetime.** `zip`'s own `ZipFileSeek` borrows the `ZipArchive` it came
/// from, so it can be neither `'static` nor moved to a decoder thread. This learns the entry's byte
/// range, drops the archive, keeps the file handle, and is thereafter just a file with a start and a
/// length — `Read + Seek + Send + Sync + 'static`, which is what ffmpeg's `StreamIo::from_read_seek`
/// (`Send`) and symphonia's `MediaSource` (`Send + Sync`) each require. Do not "simplify" the `Sync`
/// away: only one of the two callers needs it, and it is the one that would fail to compile later.
///
/// **Every offset in it is `u64`.** `armeabi-v7a` is a real target — the television runs a 32-bit
/// OS — and a 500 MB entry six gigabytes into a package is exactly the arithmetic that must not pass
/// through `usize` on the way.
///
/// No `BufReader` around it, deliberately: ffmpeg's `StreamIo` buffers 32 KiB of its own and
/// symphonia's `MediaSourceStream` buffers too, so a third layer would be a copy for nothing.
#[derive(Debug)]
pub struct EntryWindow {
    /// The package, seeked to somewhere inside the entry. Its cursor is authoritative.
    file: std::fs::File,
    /// Absolute offset of the entry's first byte within the package.
    start: u64,
    /// The entry's length. For a stored entry this is both the stored and the real size.
    len: u64,
    /// Read position relative to `start`, mirroring the file's cursor so `read` can clamp without
    /// asking the operating system where it is.
    pos: u64,
    /// The entry's name inside the archive.
    name: String,
}

impl EntryWindow {
    /// The entry's name inside the archive, such as `media/0007.mp4`.
    ///
    /// Both decoders want it: it is the subject of their error messages, and — because custom I/O
    /// hands ffmpeg no filename to look at — it is also the format hint it probes from.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How many bytes the entry holds.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the entry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Moves to a position measured from the entry's own start.
    ///
    /// Positions past the end are permitted, exactly as a `File` permits them, and the next read
    /// returns nothing. Refusing them would surprise ffmpeg, whose size probe seeks to the end and
    /// back on every open.
    fn seek_within(&mut self, pos: u64) -> std::io::Result<u64> {
        self.file
            .seek(std::io::SeekFrom::Start(self.start.saturating_add(pos)))?;
        self.pos = pos;
        Ok(pos)
    }
}

impl Read for EntryWindow {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.len.saturating_sub(self.pos);
        if remaining == 0 {
            return Ok(0);
        }
        // The one narrowing in the type, and it narrows **downwards**: `remaining` may be five
        // gigabytes on a 32-bit build and `buf.len()` cannot be, so the `min` happens before the
        // `try_from` and never after it. Without this clamp a reader runs straight out of its entry
        // and into the next file's local header, which presents as a video that plays and then
        // shows garbage.
        let want = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let read = self.file.read(&mut buf[..want])?;
        self.pos = self.pos.saturating_add(read as u64);
        Ok(read)
    }
}

impl Seek for EntryWindow {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        let target: i64 = match from {
            std::io::SeekFrom::Start(pos) => return self.seek_within(pos),
            std::io::SeekFrom::End(offset) => i64::try_from(self.len)
                .unwrap_or(i64::MAX)
                .saturating_add(offset),
            std::io::SeekFrom::Current(offset) => i64::try_from(self.pos)
                .unwrap_or(i64::MAX)
                .saturating_add(offset),
        };
        let target = u64::try_from(target).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cannot seek before the start of a package entry",
            )
        })?;
        self.seek_within(target)
    }

    fn stream_position(&mut self) -> std::io::Result<u64> {
        Ok(self.pos)
    }
}

/// One media entry in a package, as the central directory describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    /// Its name inside the archive.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Whether it is stored uncompressed, which is the only way it can be seeked into. Required of
    /// the entries [`is_seekable_entry`] names; the `.cdg` is deflated and this is `false` for it.
    pub stored: bool,
}

/// The graphics entry that pairs with an MP3+G song's audio entry.
///
/// **Resolved by rule and never from a manifest field**, which is unchanged from when the pair lived
/// in a folder and unchanged for the same reason: a field that can only ever hold one value is a
/// field a hand-edited manifest can set wrong and every writer has to agree about.
///
/// A **string** operation rather than a `Path` one. An archive entry is a plain name with forward
/// slashes in it, and going through `Path::with_extension` invites the host's opinions about what a
/// file name is — which on Windows differ from the archive's.
#[must_use]
pub fn graphics_entry_for(audio: &str) -> String {
    match audio.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.{GRAPHICS_EXTENSION}"),
        None => format!("{audio}.{GRAPHICS_EXTENSION}"),
    }
}

/// The extension a packaged UltraStar or LRC song's lyric timeline is stored under.
pub const LYRICS_EXTENSION: &str = "json";

/// The lyric timeline entry that pairs with an UltraStar or LRC song's audio entry.
///
/// Resolved by rule, as [`graphics_entry_for`] is and for its reason.
#[must_use]
pub fn lyrics_entry_for(audio: &str) -> String {
    match audio.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.{LYRICS_EXTENSION}"),
        None => format!("{audio}.{LYRICS_EXTENSION}"),
    }
}

/// The entry a song of this kind keeps beside its audio, if it keeps one.
///
/// **The one place the second entry is named**, so that checking, listing and copying a package
/// cannot disagree about which songs have one.
#[must_use]
pub fn companion_entry_for(kind: SongKind, file: &str) -> Option<String> {
    match kind {
        SongKind::Cdg => Some(graphics_entry_for(file)),
        SongKind::UltraStar | SongKind::Lrc => Some(lyrics_entry_for(file)),
        SongKind::Midi | SongKind::Video | SongKind::Unknown => None,
    }
}

/// Extensions treated as video songs.
///
/// Deliberately ordinary container extensions rather than an invented one. A video song's identity
/// is the hash of its bytes, and ffmpeg recognizes containers by reading them, so a private extension
/// would buy nothing and cost the things that make a folder browsable by hand — the operating
/// system's thumbnails, previews and double-click-to-play. Whether a file has been transcoded to the
/// packaging profile is a property of its bytes, checked by probing it, not something a name can
/// claim.
///
/// **Here rather than in `km-pack` because three crates need it and only two of them are tools.**
/// Packaging asks in order to walk a folder; `km-app` asks in order to tell a loose `.mp4` from a
/// `.kar` before deciding how to open it — and it must be able to ask in a build with no `video`
/// feature, so that such a build says "this build cannot play video" rather than "not a MIDI file".
pub const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "mkv", "webm", "mov"];

/// Whether a path looks like a video song.
#[must_use]
pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| VIDEO_EXTENSIONS.contains(&ext.as_str()))
}

/// The extension a CD+G graphics file has, and the one a packaged pair is renamed to.
pub const GRAPHICS_EXTENSION: &str = "cdg";

/// Extensions treated as the audio half of an MP3+G song.
///
/// One entry, and a list anyway: `km-cdg` is built on `symphonia`, which decodes rather more than
/// MP3, so widening this is a one-line change the day a corpus turns up in another format. What the
/// measured corpus holds is 2,847 MP3s and nothing else, and inventing support for a case that does
/// not occur is how an untested path ships.
pub const AUDIO_EXTENSIONS: [&str; 1] = ["mp3"];

/// Whether a path looks like the graphics half of an MP3+G song.
///
/// **Here rather than in `km-pack` for the same reason [`is_video_file`] is**: `km-app` must be able
/// to tell a loose `.cdg` from a `.kar` in order to say something useful about it.
#[must_use]
pub fn is_graphics_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(GRAPHICS_EXTENSION))
}

/// The extension an LRC lyrics file has.
pub const LRC_EXTENSION: &str = "lrc";

/// Whether a path looks like an LRC lyrics file.
#[must_use]
pub fn is_lrc_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(LRC_EXTENSION))
}

/// Whether a path looks like the audio half of an MP3+G song.
#[must_use]
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
}

/// Finds the file that pairs with one half of an MP3+G song, if it is on disk.
///
/// Give it either half and it looks for the other: an `.mp3` for a `.cdg`, or the reverse.
///
/// **Deliberately tolerant, because a real corpus is not tidy.** Measured over 2,851 tracks:
/// extensions come in both cases (`.MP3` and `.mp3` in one folder), and one pair differs only by a
/// **trailing space** in the stem. So the obvious spellings are tried first, and only if none of
/// them exists is the directory listed and matched on a trimmed, lowercased stem.
///
/// Note the Windows hazard in that slow path: Win32 strips trailing spaces from path components, so
/// the path returned is the one `read_dir` gave, and a caller must report a failure to open it
/// rather than quietly dropping the song.
#[must_use]
pub fn pair_for(path: &Path) -> Option<PathBuf> {
    let wanted: &[&str] = if is_graphics_file(path) {
        &AUDIO_EXTENSIONS
    } else if is_audio_file(path) {
        std::slice::from_ref(&GRAPHICS_EXTENSION)
    } else {
        return None;
    };
    sibling_with_extension(path, wanted)
}

/// Finds the file beside `path` with the same stem and one of the `wanted` extensions.
///
/// The search [`pair_for`] makes, for any pair of extensions: the obvious spellings first, then the
/// directory matched on a trimmed, lowercased stem. An LRC file finds its audio the same way.
#[must_use]
pub fn sibling_with_extension(path: &Path, wanted: &[&str]) -> Option<PathBuf> {
    for extension in wanted {
        for spelling in [
            extension.to_ascii_lowercase(),
            extension.to_ascii_uppercase(),
        ] {
            let candidate = path.with_extension(spelling);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let stem = path.file_stem()?.to_str()?.trim().to_lowercase();
    std::fs::read_dir(path.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|candidate| {
            candidate
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| wanted.iter().any(|want| ext.eq_ignore_ascii_case(want)))
                && candidate
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.trim().to_lowercase() == stem)
        })
}

/// Hashes both halves of an MP3+G song together, as one recording's identity.
///
/// **Not the audio alone**, and the difference matters. The duplicate rule is "the same recording
/// filed under two numbers", and the same backing track with two different `.cdg` files is two
/// different karaoke songs — different words, different timing, different disc. Hashing the audio
/// alone would collapse them, and `ManifestProblem::DuplicateContent` would then refuse a package
/// that is perfectly legitimate.
#[must_use]
pub fn pair_content_hash(audio: &[u8], graphics: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(audio);
    digest.update(graphics);
    let digest = digest.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Hashes MIDI bytes, for spotting the same recording under two numbers.
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    // Truncated to 16 bytes: this identifies duplicates in a catalog, it is not a security
    // boundary, and a shorter string keeps the manifest readable.
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Feeds a whole file into a digest a megabyte at a time.
///
/// A hand-rolled loop rather than `io::copy`, because `Sha256` is only an `io::Write` when `sha2`'s
/// `std` feature is on and turning that on to save four lines is a dependency change for nothing.
fn hash_file_into(digest: &mut Sha256, path: &Path) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        digest.update(&buffer[..read]);
    }
}

/// Hashes a file's bytes without holding them.
///
/// The slice form above is right for MIDI, which the packager has read anyway. A video is hundreds
/// of megabytes and must not land in a `Vec` on a device with three gigabytes of address space, so
/// this streams. Same truncation and same meaning — it identifies duplicates in a catalog.
pub fn content_hash_of(path: &Path) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    hash_file_into(&mut digest, path)?;
    let digest = digest.finalize();
    Ok(digest[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// Hashes both halves of an MP3+G song together, without holding either.
///
/// The streaming twin of [`pair_content_hash`], and both halves for the reason that one gives: two
/// karaoke songs can share an MP3 and differ entirely in their words.
pub fn pair_content_hash_of(audio: &Path, graphics: &Path) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    hash_file_into(&mut digest, audio)?;
    hash_file_into(&mut digest, graphics)?;
    let digest = digest.finalize();
    Ok(digest[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// Where one entry's bytes come from when the package is written.
///
/// Nothing but MIDI is held in memory. A media entry records a **path** and is copied in at
/// [`PackageBuilder::write`] time, so a builder holding four thousand videos costs four thousand
/// paths rather than twenty gigabytes.
#[derive(Debug)]
enum Content {
    /// Held in memory. MIDI only, and only because a MIDI file is a few kilobytes.
    Bytes(Vec<u8>),
    /// A file on disk, streamed in when the package is written.
    File(PathBuf),
    /// An entry of another package, copied across byte for byte without re-encoding.
    Copied {
        /// The package it comes from.
        from: PathBuf,
        /// Its name in that package. The same name it will have here.
        entry: String,
    },
}

/// One entry waiting to be written.
#[derive(Debug)]
struct Pending {
    name: String,
    content: Content,
}

/// The names of entries whose bytes are not the bytes they were written as.
///
/// **Empty is the answer for a package that arrived intact**, and a name in the list is a package to
/// rebuild or fetch again rather than one to play. The directory records a CRC per entry for this,
/// because a decoder streaming a video has no way to notice that it has been handed the wrong bytes.
///
/// Read through the container rather than through [`Package`], because the package worth asking this
/// about is often one whose manifest is already refused, and `Package::open` would never get here.
///
/// **It reads the whole package**, which for a video library is most of a minute, so it is what
/// `km-pack check --verify` does rather than what every check does.
///
/// # Errors
///
/// If the file cannot be opened or is not a package.
pub fn damaged_entries(path: &Path) -> Result<Vec<String>, PackageError> {
    let display = path.display().to_string();
    let mut container = open_container(path, &display)?;

    let entries: Vec<container::Entry> = container.entries().cloned().collect();
    let mut damaged = Vec::new();
    for entry in entries {
        let intact = container
            .verify(&entry)
            .map_err(|error| container_error(error, &display))?;
        if !intact {
            damaged.push(entry.name);
        }
    }
    Ok(damaged)
}

/// How a media entry is held.
///
/// A function rather than a line at the call site so that a test can drive the real decision instead
/// of a copy of it.
///
/// **The rule is what the decoder does with it, not that it is media.** An entry a decoder *seeks
/// into* is stored, because a compressed entry has no byte range to seek into and seeking into media
/// in place is the whole design. That is the video and the MP3, and it costs nothing: H.264 and MP3
/// are entropy-coded already, so deflating either would spend time to save nothing.
///
/// **The `.cdg` is deflated**, because nothing ever seeks into one. [`Package::graphics_bytes`]
/// reads it whole, since `km-cdg` replays from packet zero on every seek and holds all of it anyway.
/// Measured over 60 corpus files, CD+G deflates to **14.7%** — mean 1.85 MB down to 272 KB, which
/// takes **26%** off a real MP3+G package once its MP3s are counted too.
///
/// It is also not a latency trade, which is what the earlier blanket rule assumed. Inflating a
/// 1.67 MB `.cdg` measured *below* the cost of spawning the process that did it — single-digit
/// milliseconds — while removing ~1.5 MB from the read, so on anything slower than about 150 MB/s
/// the deflated entry reaches the screen sooner. See `docs/decisions/packaging.md`.
///
/// **There is no size above which a media entry needs anything else.** Every offset and length the
/// container records is `u64`, and one video crossing four gibibytes is ordinary.
fn media_method(name: &str) -> Method {
    if is_seekable_entry(name) {
        Method::Stored
    } else {
        Method::Deflate
    }
}

/// Whether a decoder will seek into this archive entry rather than read it whole.
///
/// The one place the "stored" rule is decided, so that the writer, [`Package::media_entries`] and
/// `km-pack check` cannot drift apart about which entries it covers. Takes an **archive entry name**
/// — a plain string with forward slashes — rather than a `Path`, for [`graphics_entry_for`]'s
/// reason: the host's opinions about what a file name is differ from the archive's.
#[must_use]
pub fn is_seekable_entry(name: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    VIDEO_EXTENSIONS.contains(&extension.as_str()) || AUDIO_EXTENSIONS.contains(&extension.as_str())
}

/// Builds a new package.
pub struct PackageBuilder {
    manifest: Manifest,
    files: Vec<Pending>,
}

impl PackageBuilder {
    /// Starts an empty package.
    pub fn new(package: PackageMeta) -> Self {
        Self {
            manifest: Manifest::new(package),
            files: Vec::new(),
        }
    }

    /// Adds a song, choosing its path inside the archive.
    ///
    /// The content hash is computed here, so duplicate detection needs no second pass over the files.
    pub fn add(&mut self, mut entry: SongEntry, midi: Vec<u8>) -> Result<&SongEntry, PackageError> {
        if self.manifest.song(entry.number).is_some() {
            return Err(PackageError::DuplicateNumber(entry.number));
        }
        entry.kind = SongKind::Midi;
        if entry.file.trim().is_empty() {
            entry.file = format!("midi/{}.mid", entry.number);
        }
        entry.content_hash = Some(content_hash(&midi));

        self.files.push(Pending {
            name: entry.file.clone(),
            content: Content::Bytes(midi),
        });
        self.manifest.songs.push(entry);
        Ok(self.manifest.songs.last().expect("just pushed"))
    }

    /// Adds a video song, whose file is copied into the archive when the package is written.
    ///
    /// `name` is the entry it becomes; `source` is where its bytes are now. **Nothing is read
    /// here** — a video is hundreds of megabytes, and a builder that buffered one could not build
    /// the packages this exists for.
    ///
    /// The content hash is still the caller's to supply, and for a sharper version of the reason it
    /// always was: it is the hash of the **source**, which for a re-encoded video is not what the
    /// archive will hold, so it cannot be recovered from the package afterwards. Use
    /// [`content_hash_of`], which streams. A consequence worth knowing: the source is read twice,
    /// once to hash and once to copy, where the old folder path read it once into memory. Two
    /// sequential reads are the cheaper half of that trade.
    pub fn add_video_source(
        &mut self,
        mut entry: SongEntry,
        name: &str,
        source: &Path,
        content_hash: Option<String>,
    ) -> Result<&SongEntry, PackageError> {
        if self.manifest.song(entry.number).is_some() {
            return Err(PackageError::DuplicateNumber(entry.number));
        }
        entry.kind = SongKind::Video;
        entry.file = name.to_owned();
        entry.content_hash = content_hash;

        self.files.push(Pending {
            name: name.to_owned(),
            content: Content::File(source.to_path_buf()),
        });
        self.manifest.songs.push(entry);
        Ok(self.manifest.songs.last().expect("just pushed"))
    }

    /// Adds an MP3+G song: **two** entries, the audio under `name` and the graphics under the name
    /// [`graphics_entry_for`] derives from it.
    ///
    /// The graphics entry is not recorded in the manifest, for the reason that rule gives: a field
    /// that can only ever hold one value is one a hand-edited manifest can set wrong.
    pub fn add_cdg_source(
        &mut self,
        mut entry: SongEntry,
        name: &str,
        audio: &Path,
        graphics: &Path,
        content_hash: Option<String>,
    ) -> Result<&SongEntry, PackageError> {
        if self.manifest.song(entry.number).is_some() {
            return Err(PackageError::DuplicateNumber(entry.number));
        }
        entry.kind = SongKind::Cdg;
        entry.file = name.to_owned();
        entry.content_hash = content_hash;

        self.files.push(Pending {
            name: name.to_owned(),
            content: Content::File(audio.to_path_buf()),
        });
        self.files.push(Pending {
            name: graphics_entry_for(name),
            content: Content::File(graphics.to_path_buf()),
        });
        self.manifest.songs.push(entry);
        Ok(self.manifest.songs.last().expect("just pushed"))
    }

    /// Adds an UltraStar or LRC song: its audio under `name`, and its lyric timeline under the
    /// name [`lyrics_entry_for`] derives from it.
    ///
    /// The timeline is what the machine reads, so the lyrics file itself never enters the package:
    /// its dialects are handled once, here at the packager. See `The machine never reads an
    /// UltraStar file` in `docs/decisions/song-sources.md`.
    ///
    /// # Panics
    ///
    /// When `kind` is not one that [`SongKind::carries_timeline`].
    pub fn add_timeline_source(
        &mut self,
        kind: SongKind,
        mut entry: SongEntry,
        name: &str,
        audio: &Path,
        timeline: &km_song::LyricTimeline,
        content_hash: Option<String>,
    ) -> Result<&SongEntry, PackageError> {
        assert!(kind.carries_timeline(), "{kind:?} carries no timeline");
        if self.manifest.song(entry.number).is_some() {
            return Err(PackageError::DuplicateNumber(entry.number));
        }
        entry.kind = kind;
        entry.file = name.to_owned();
        entry.content_hash = content_hash;

        let json = serde_json::to_vec(timeline).expect("a lyric timeline serializes");
        self.files.push(Pending {
            name: name.to_owned(),
            content: Content::File(audio.to_path_buf()),
        });
        self.files.push(Pending {
            name: lyrics_entry_for(name),
            content: Content::Bytes(json),
        });
        self.manifest.songs.push(entry);
        Ok(self.manifest.songs.last().expect("just pushed"))
    }

    /// Re-adds a media song by copying its bytes straight across from the package it came from.
    ///
    /// For rebuilding a package from an existing one — correcting a title, re-running analysis.
    /// The entries go through `ZipWriter::raw_copy_file`, so nothing is decoded, nothing is
    /// re-encoded, nothing is held in memory and the CRC is carried over rather than recomputed.
    ///
    /// **This is what `add_out_of_archive` used to achieve by not copying anything at all**, and it
    /// is the cost of a package being one file: an edit now moves the whole archive. It is a byte
    /// copy on a curation workstation rather than work on the machine, which is why it was accepted.
    ///
    /// The hash is **kept**, unlike the MIDI path where it is recomputed from the bytes: the bytes
    /// are not being read, and a media song's recorded hash is its source's rather than the
    /// archive's, so recomputing it here would quietly record something else.
    pub fn add_media_copied(
        &mut self,
        entry: SongEntry,
        from: &Package,
    ) -> Result<&SongEntry, PackageError> {
        if self.manifest.song(entry.number).is_some() {
            return Err(PackageError::DuplicateNumber(entry.number));
        }
        debug_assert!(
            !entry.kind.is_midi(),
            "add_media_copied is for a song whose bytes are media"
        );
        self.files.push(Pending {
            name: entry.file.clone(),
            content: Content::Copied {
                from: from.path().to_path_buf(),
                entry: entry.file.clone(),
            },
        });
        if let Some(companion) = companion_entry_for(entry.kind, &entry.file) {
            self.files.push(Pending {
                name: companion.clone(),
                content: Content::Copied {
                    from: from.path().to_path_buf(),
                    entry: companion,
                },
            });
        }
        self.manifest.songs.push(entry);
        Ok(self.manifest.songs.last().expect("just pushed"))
    }

    /// Songs added so far.
    pub fn len(&self) -> usize {
        self.manifest.songs.len()
    }

    /// Whether nothing has been added.
    pub fn is_empty(&self) -> bool {
        self.manifest.songs.is_empty()
    }

    /// The manifest as it stands, for inspection before writing.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The songs added so far, for a last pass over the whole set before writing.
    ///
    /// Exists for the questions that can only be answered once everything is in — filling in a
    /// `--default-language` for the songs that ended up without one is the case it was added for.
    /// Adding a song is still [`PackageBuilder::add`]'s business, because that is what computes the
    /// content hash and stores the bytes; this only lets a caller revise what is already there.
    pub fn songs_mut(&mut self) -> &mut [SongEntry] {
        &mut self.manifest.songs
    }

    /// Everything wrong with what has been built so far.
    pub fn problems(&self) -> Vec<ManifestProblem> {
        self.manifest.problems()
    }

    /// Writes the package.
    ///
    /// Refuses to write an invalid package: shipping one that cannot be installed wastes the time of
    /// whoever tries.
    pub fn write(mut self, path: impl AsRef<Path>) -> Result<Manifest, PackageError> {
        let path = path.as_ref();
        let display = path.display().to_string();

        // Decided here rather than at construction, because it depends on what was added.
        self.manifest.format = self.manifest.required_format();

        let problems = self.manifest.problems();
        if !problems.is_empty() {
            return Err(PackageError::Invalid {
                path: display,
                problems,
            });
        }

        let file = std::fs::File::create(path).map_err(|source| PackageError::Io {
            path: display.clone(),
            source,
        })?;
        let file = std::io::BufWriter::with_capacity(1 << 20, file);

        let json = serde_json::to_string_pretty(&self.manifest).map_err(|source| {
            PackageError::Manifest {
                path: display.clone(),
                source,
            }
        })?;

        let fault = |error: ContainerError| container_error(error, &display);
        let mut writer = Writer::new(file).map_err(fault)?;

        // Opened once each and reused, though in practice a rebuild reads exactly one.
        let mut sources: std::collections::HashMap<PathBuf, Container<std::fs::File>> =
            std::collections::HashMap::new();

        for pending in &self.files {
            match &pending.content {
                Content::Bytes(bytes) => {
                    // MIDI deflates: measured over the corpus it comes down by about 70% — 71.8%
                    // across 400 `.kar`, 68.4% across 300 `.mid`. MIDI is **not** "mostly already
                    // dense", which is the plausible wrong argument for the other setting. The
                    // saving is small in absolute terms — a few tens of kilobytes a song — because a
                    // MIDI file is small; that is a reason not to care either way, not a reason to
                    // store it.
                    writer
                        .write_entry(&pending.name, Method::Deflate, bytes)
                        .map_err(fault)?;
                }
                Content::File(source) => {
                    let handle = std::fs::File::open(source).map_err(|error| PackageError::Io {
                        path: source.display().to_string(),
                        source: error,
                    })?;
                    let len = handle
                        .metadata()
                        .map_err(|error| PackageError::Io {
                            path: source.display().to_string(),
                            source: error,
                        })?
                        .len();
                    // Streamed, so the peak is the buffer rather than the file. This is the whole
                    // reason a media entry records a path instead of bytes.
                    let copied = writer
                        .stream_entry(
                            &pending.name,
                            media_method(&pending.name),
                            &mut std::io::BufReader::with_capacity(1 << 20, handle),
                        )
                        .map_err(fault)?;
                    if copied != len {
                        // A source that changed size underneath the build would otherwise produce a
                        // package whose entry and manifest quietly disagree about a song nobody has
                        // played yet.
                        return Err(PackageError::SourceChanged {
                            path: source.display().to_string(),
                            expected: len,
                            copied,
                        });
                    }
                }
                Content::Copied { from, entry } => {
                    let source = match sources.entry(from.clone()) {
                        std::collections::hash_map::Entry::Occupied(held) => held.into_mut(),
                        std::collections::hash_map::Entry::Vacant(slot) => {
                            let held = open_container(from, &from.display().to_string())?;
                            slot.insert(held)
                        }
                    };
                    // The source package is one this build did not write — a rebuild starts from
                    // whatever was handed in. The package written here goes out under the builder's
                    // name, so anything it launders is something a later reader would be right to
                    // trust.
                    if !is_safe_path(entry) {
                        return Err(PackageError::Invalid {
                            path: from.display().to_string(),
                            problems: vec![ManifestProblem::UnsafePath {
                                number: 0,
                                path: entry.clone(),
                            }],
                        });
                    }
                    let held = source
                        .entry(entry)
                        .ok_or_else(|| PackageError::MissingEntry {
                            number: 0,
                            file: entry.clone(),
                        })?
                        .clone();
                    // A byte copy: no decode, no re-encode, flat memory. This is what keeps
                    // correcting one title in a 20 GB package a copy of 20 GB rather than a
                    // re-encode of it.
                    writer
                        .raw_copy(entry, source.reader_mut(), &held)
                        .map_err(fault)?;
                }
            }
        }

        // Last, so that correcting a title can be a truncation and a re-append rather than a copy
        // of every video in the package. What says the file is a package is its first eight bytes.
        writer
            .write_entry(MANIFEST_PATH, Method::Deflate, json.as_bytes())
            .map_err(fault)?;

        let file = writer.finish().map_err(fault)?;
        file.into_inner()
            .map_err(|error| PackageError::Io {
                path: display.clone(),
                source: error.into_error(),
            })?
            .sync_all()
            .map_err(|source| PackageError::Io {
                path: display,
                source,
            })?;
        Ok(self.manifest)
    }
}

/// Reads a package's manifest without validating it, for diagnosing a broken package.
///
/// [`Package::open`] refuses an invalid manifest, which is right for anything that will play songs
/// but useless when the question is *why* a package will not open.
pub fn read_manifest_unchecked(path: impl AsRef<Path>) -> Result<Manifest, PackageError> {
    let path = path.as_ref();
    let display = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|source| PackageError::Io {
        path: display.clone(),
        source,
    })?;
    read_manifest_from(file, &display)
}

fn read_manifest_from<R: Read + Seek>(reader: R, display: &str) -> Result<Manifest, PackageError> {
    let mut container = Container::open(reader).map_err(|error| container_error(error, display))?;
    let json = read_manifest_bytes(&mut container, display)?;
    serde_json::from_str(&json).map_err(|source| PackageError::Manifest {
        path: display.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory of this test's own, removed however the test ends.
    ///
    /// **The process id is what makes it a test's own**, and it is load-bearing rather than tidy. A
    /// name of `km-package-tests-{name}` alone is unique within one run and shared by every run on
    /// the machine — so two `cargo test` invocations of this crate at once write the same paths,
    /// and one deletes the other's package out from under it mid-test. That is not hypothetical
    /// here: worktrees share one `%TEMP%`, and several sessions build in this repository at a time.
    /// It presents as two tests failing together, passing on a re-run, and passing under
    /// `--test-threads=1` — which reads like a race in the code under test rather than in the
    /// harness around it.
    ///
    /// The thread id is belt and braces for a `name` used from two threads at once; each test
    /// passes its own today.
    ///
    /// Cleaning up on [`Drop`] rather than at the end of each test is what covers a panic, which is
    /// exactly when a test used to leave its directory behind. The same shape as `Scratch` in
    /// `km-package-builder`'s build tests.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-package-tests-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            // Only reachable if a previous run was killed before its `Drop` ran *and* the operating
            // system handed out the same process id again. Cheap, and the alternative is a test
            // reading a stale package it did not write.
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
    }

    impl std::ops::Deref for Scratch {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(name: &str) -> Scratch {
        Scratch::new(name)
    }

    fn meta() -> PackageMeta {
        PackageMeta {
            id: EXAMPLE_ID.to_owned(),
            name: "Volume 1".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: Some("Nobody".to_owned()),
            created: None,
            volume: None,
        }
    }

    fn entry(number: u32) -> SongEntry {
        SongEntry {
            number,
            kind: SongKind::Midi,
            title: format!("Song {number}"),
            artist: Some("Someone".to_owned()),
            language: Some("eng".to_owned()),
            file: String::new(),
            duration_ms: 210_000,
            lyric_encoding: None,
            default_transpose: 0,
            lyrics_hidden: false,
            fixes: Vec::new(),
            melody: None,
            melody_abstained: None,
            suitability: None,
            lyric_preview: Vec::new(),
            tags: Vec::new(),
            loudness: None,
            content_hash: None,
            edited: Vec::new(),
        }
    }

    /// A package built by hand, so a window can be tested without the writer that will produce one.
    ///
    /// `media` entries go in exactly as asked, method included, which is what lets the refusal cases
    /// below exist at all — the real writer would never produce them.
    /// One block of bytes over and over, as a reader, so a bomb can be written without being held.
    struct Repeating<'a> {
        block: &'a [u8],
        left: u64,
        at: usize,
    }

    impl Read for Repeating<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Ok(0);
            }
            let take = buf.len().min(self.block.len() - self.at);
            buf[..take].copy_from_slice(&self.block[self.at..self.at + take]);
            self.at += take;
            if self.at == self.block.len() {
                self.at = 0;
                self.left -= 1;
            }
            Ok(take)
        }
    }

    /// A package holding a manifest and nothing else, written exactly as given.
    ///
    /// For the cases where what is being tested is the manifest itself — a format the reader
    /// refuses, a kind it does not know — and the songs would only be in the way.
    fn manifest_only(path: &Path, manifest: &Manifest) {
        let file = std::fs::File::create(path).expect("create");
        let mut writer = Writer::new(file).expect("header");
        writer
            .write_entry(
                MANIFEST_PATH,
                Method::Deflate,
                serde_json::to_string_pretty(manifest)
                    .expect("json")
                    .as_bytes(),
            )
            .expect("manifest");
        writer.finish().expect("finish");
    }

    fn hand_built(path: &Path, songs: Vec<SongEntry>, media: &[(&str, &[u8], Method)]) {
        let mut manifest = Manifest::new(meta());
        manifest.songs = songs;
        manifest.format = manifest.required_format();

        let file = std::fs::File::create(path).expect("create");
        let mut writer = Writer::new(file).expect("header");
        for (name, bytes, method) in media {
            writer.write_entry(name, *method, bytes).expect("entry");
        }
        writer
            .write_entry(
                MANIFEST_PATH,
                Method::Deflate,
                serde_json::to_string_pretty(&manifest)
                    .expect("json")
                    .as_bytes(),
            )
            .expect("manifest");
        writer.finish().expect("finish");
    }

    fn video_entry(number: u32, file: &str) -> SongEntry {
        let mut song = entry(number);
        song.kind = SongKind::Video;
        song.file = file.to_owned();
        song
    }

    /// Every byte identifies its own offset, so a window that is off by even one is caught.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_stored_media_entry_can_be_seeked_into() {
        let dir = temp_dir("window-seek");
        let path = dir.join("vol1.kmpkg");
        let bytes = pattern(64 * 1024);
        hand_built(
            &path,
            vec![video_entry(2, "media/0002.mp4")],
            &[("media/0002.mp4", bytes.as_slice(), Method::Stored)],
        );

        let package = Package::open(&path).expect("open");
        let mut window = package.media_reader(2).expect("window");
        assert_eq!(window.name(), "media/0002.mp4");
        assert_eq!(window.len(), bytes.len() as u64);
        assert!(!window.is_empty());

        let mut buf = [0u8; 16];
        window.seek(std::io::SeekFrom::Start(40_000)).expect("seek");
        assert_eq!(window.stream_position().expect("pos"), 40_000);
        window.read_exact(&mut buf).expect("read");
        assert_eq!(buf[..], bytes[40_000..40_016]);

        window.seek(std::io::SeekFrom::Current(-8)).expect("seek");
        let mut eight = [0u8; 8];
        window.read_exact(&mut eight).expect("read");
        assert_eq!(eight[..], bytes[40_008..40_016]);

        window.seek(std::io::SeekFrom::End(-4)).expect("seek");
        let mut four = [0u8; 4];
        window.read_exact(&mut four).expect("read");
        assert_eq!(four[..], bytes[bytes.len() - 4..]);

        // **The assertion that catches a missing length clamp.** Without one, a reader positioned at
        // the end of its entry runs straight into the next file's local header and a video plays on
        // into whatever follows it.
        assert_eq!(window.stream_position().expect("pos"), bytes.len() as u64);
        assert_eq!(window.read(&mut buf).expect("read at the end"), 0);
        window
            .seek(std::io::SeekFrom::Start(bytes.len() as u64 + 5_000))
            .expect("seeking past the end is allowed, as it is on a File");
        assert_eq!(window.read(&mut buf).expect("read past the end"), 0);

        // And the whole entry comes back byte for byte.
        window.seek(std::io::SeekFrom::Start(0)).expect("rewind");
        let mut all = Vec::new();
        window.read_to_end(&mut all).expect("read all");
        assert_eq!(all, bytes);
    }

    #[test]
    fn seeking_before_the_start_of_an_entry_is_refused() {
        let dir = temp_dir("window-underflow");
        let path = dir.join("vol1.kmpkg");
        hand_built(
            &path,
            vec![video_entry(2, "media/0002.mp4")],
            &[("media/0002.mp4", b"abcdef", Method::Stored)],
        );

        let package = Package::open(&path).expect("open");
        let mut window = package.media_reader(2).expect("window");
        // Not clamped to zero: a decoder that computed a negative offset has a bug, and reading the
        // package's own local headers back to it would hide that rather than report it.
        assert!(window.seek(std::io::SeekFrom::Current(-1)).is_err());
        assert!(window.seek(std::io::SeekFrom::End(-99)).is_err());
    }

    #[test]
    fn a_compressed_media_entry_is_refused() {
        let dir = temp_dir("window-deflated");
        let path = dir.join("vol1.kmpkg");
        hand_built(
            &path,
            vec![video_entry(2, "media/0002.mp4")],
            &[("media/0002.mp4", pattern(4096).as_slice(), Method::Deflate)],
        );

        let package = Package::open(&path).expect("open");
        let error = package.media_reader(2).expect_err("refused");
        assert!(matches!(error, PackageError::NotStored { number: 2, .. }));
        assert!(error.to_string().contains("stored uncompressed"));
    }

    #[test]
    fn a_media_entry_the_manifest_names_but_the_archive_lacks() {
        let dir = temp_dir("window-missing");
        let path = dir.join("vol1.kmpkg");
        hand_built(&path, vec![video_entry(2, "media/0002.mp4")], &[]);

        let package = Package::open(&path).expect("open");
        assert!(matches!(
            package.media_reader(2).expect_err("refused"),
            PackageError::MissingEntry { number: 2, .. }
        ));
        assert_eq!(package.missing_entries().expect("check"), vec![2]);
    }

    #[test]
    fn a_truncated_package_is_refused_rather_than_read_past() {
        let dir = temp_dir("window-truncated");
        let path = dir.join("vol1.kmpkg");
        hand_built(
            &path,
            vec![video_entry(2, "media/0002.mp4")],
            &[("media/0002.mp4", pattern(8192).as_slice(), Method::Stored)],
        );

        // A half-finished copy. The central directory is still readable because it was read into
        // memory on open, so without the bounds check this would hand back a window running past
        // the end of the file and fail six seconds into a song instead of now.
        let full = std::fs::metadata(&path).expect("metadata").len();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open for truncation");
        let package = Package::open(&path).expect("open");
        file.set_len(full - 4096).expect("truncate");

        let error = package.media_reader(2).expect_err("refused");
        assert!(error.to_string().contains("truncated"), "{error}");
    }

    #[test]
    fn asking_a_midi_song_for_a_media_window_is_refused() {
        let dir = temp_dir("window-not-media");
        let path = dir.join("vol1.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi bytes".to_vec()).expect("add");
        builder.write(&path).expect("write");

        let package = Package::open(&path).expect("open");
        assert!(matches!(
            package.media_reader(1).expect_err("refused"),
            PackageError::NotMedia { number: 1, .. }
        ));
        assert!(matches!(
            package.media_reader(99).expect_err("refused"),
            PackageError::NoSuchSong(99)
        ));
    }

    #[test]
    fn a_graphics_entry_is_named_from_its_audio() {
        assert_eq!(graphics_entry_for("media/0012.mp3"), "media/0012.cdg");
        assert_eq!(graphics_entry_for("0012.mp3"), "0012.cdg");
        // A name with no extension at all still gets one rather than losing its identity.
        assert_eq!(graphics_entry_for("media/track"), "media/track.cdg");
        // Only the last dot, so a stem with dots in it survives.
        assert_eq!(graphics_entry_for("media/a.b.mp3"), "media/a.b.cdg");
    }

    #[test]
    fn the_seek_rule_covers_video_and_audio_but_not_graphics() {
        // The one place the writer, the listing and `km-pack check` agree about which entries the
        // "stored" rule binds. Case-insensitive because an archive name is whatever a packager's
        // source file was called.
        for name in [
            "media/0002.mp4",
            "media/0002.MKV",
            "media/0002.webm",
            "media/0002.mov",
            "media/0012.mp3",
        ] {
            assert!(is_seekable_entry(name), "{name} is seeked into");
        }
        for name in [
            "media/0012.cdg",
            "media/0012.CDG",
            "midi/0001.kar",
            MANIFEST_PATH,
            "media/noextension",
        ] {
            assert!(!is_seekable_entry(name), "{name} is read whole");
        }
    }

    #[test]
    fn the_media_listing_says_what_is_stored_and_how_big_it_is() {
        let dir = temp_dir("window-listing");
        let path = dir.join("vol1.kmpkg");
        hand_built(
            &path,
            vec![video_entry(2, "media/0002.mp4")],
            &[("media/0002.mp4", pattern(1234).as_slice(), Method::Stored)],
        );

        let package = Package::open(&path).expect("open");
        let entries = package.media_entries().expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "media/0002.mp4");
        assert_eq!(entries[0].size, 1234);
        assert!(entries[0].stored);
        // `size` is the uncompressed length either way, which is what makes the listing's total
        // comparable across a package whose `.cdg`s are deflated and whose video is not.
        assert!(is_seekable_entry(&entries[0].name));
    }

    #[test]
    fn the_listing_separates_what_must_be_stored_from_what_merely_is_not() {
        // What `km-pack check` reports on: a deflated `.cdg` is expected and a deflated `.mp3` is a
        // fault, and the listing has to let the two be told apart.
        let dir = temp_dir("window-listing-split");
        let path = dir.join("vol1.kmpkg");
        let mut song = entry(12);
        song.kind = SongKind::Cdg;
        song.file = "media/0012.mp3".to_owned();
        hand_built(
            &path,
            vec![song],
            &[
                ("media/0012.mp3", pattern(9_000).as_slice(), Method::Stored),
                (
                    "media/0012.cdg",
                    pattern(40_000).as_slice(),
                    Method::Deflate,
                ),
            ],
        );

        let package = Package::open(&path).expect("open");
        let entries = package.media_entries().expect("entries");
        let faults: Vec<&str> = entries
            .iter()
            .filter(|entry| is_seekable_entry(&entry.name) && !entry.stored)
            .map(|entry| entry.name.as_str())
            .collect();
        assert!(faults.is_empty(), "nothing here is wrongly compressed");

        let deflated: Vec<&str> = entries
            .iter()
            .filter(|entry| !entry.stored)
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(deflated, vec!["media/0012.cdg"]);
    }

    #[test]
    fn a_video_song_round_trips_through_the_archive() {
        let dir = temp_dir("write-video");
        let source = dir.join("clip.mp4");
        let bytes = pattern(40_000);
        std::fs::write(&source, &bytes).expect("source");

        let path = dir.join("vol1.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi bytes".to_vec()).expect("midi");
        builder
            .add_video_source(
                entry(2),
                "media/0002.mp4",
                &source,
                Some(content_hash_of(&source).expect("hash")),
            )
            .expect("video");
        builder.write(&path).expect("write");

        let package = Package::open(&path).expect("open");
        assert!(package.missing_entries().expect("check").is_empty());
        assert_eq!(package.read_song(1).expect("midi"), b"midi bytes");

        let mut window = package.media_reader(2).expect("window");
        let mut read_back = Vec::new();
        window.read_to_end(&mut read_back).expect("read");
        assert_eq!(read_back, bytes);

        // Deleting the source proves the bytes are in the package rather than referenced from it.
        std::fs::remove_file(&source).expect("remove");
        assert!(package.media_reader(2).is_ok());
    }

    #[test]
    fn an_mp3_plus_g_pair_round_trips_as_two_entries() {
        let dir = temp_dir("write-cdg");
        let audio = dir.join("song.mp3");
        let graphics = dir.join("song.cdg");
        let audio_bytes = pattern(9_000);
        let graphics_bytes = pattern(2_600);
        std::fs::write(&audio, &audio_bytes).expect("audio");
        std::fs::write(&graphics, &graphics_bytes).expect("graphics");

        let path = dir.join("vol1.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder
            .add_cdg_source(
                entry(12),
                "media/0012.mp3",
                &audio,
                &graphics,
                Some(pair_content_hash_of(&audio, &graphics).expect("hash")),
            )
            .expect("cdg");
        builder.write(&path).expect("write");

        let package = Package::open(&path).expect("open");
        assert!(package.missing_entries().expect("check").is_empty());

        let mut window = package.media_reader(12).expect("audio window");
        assert_eq!(window.name(), "media/0012.mp3");
        let mut read_back = Vec::new();
        window.read_to_end(&mut read_back).expect("read");
        assert_eq!(read_back, audio_bytes);
        assert_eq!(
            package.graphics_bytes(12).expect("graphics"),
            graphics_bytes
        );
    }

    #[test]
    fn half_a_pair_is_still_not_a_song() {
        // The rule survives the move from a folder to the archive, and for the reason it always had:
        // an MP3 with no `.cdg` has no words in it at all.
        let dir = temp_dir("write-half-pair");
        let path = dir.join("vol1.kmpkg");
        let mut song = entry(12);
        song.kind = SongKind::Cdg;
        song.file = "media/0012.mp3".to_owned();
        hand_built(
            &path,
            vec![song],
            &[("media/0012.mp3", pattern(64).as_slice(), Method::Stored)],
        );

        let package = Package::open(&path).expect("open");
        assert_eq!(package.missing_entries().expect("check"), vec![12]);
    }

    #[test]
    fn the_manifest_is_last_and_only_seekable_media_is_stored() {
        let dir = temp_dir("write-order");
        let source = dir.join("clip.mp4");
        std::fs::write(&source, pattern(5_000)).expect("source");
        let audio = dir.join("song.mp3");
        let graphics = dir.join("song.cdg");
        std::fs::write(&audio, pattern(9_000)).expect("audio");
        std::fs::write(&graphics, pattern(40_000)).expect("graphics");

        let path = dir.join("vol1.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi bytes".to_vec()).expect("midi");
        builder
            .add_video_source(entry(2), "media/0002.mp4", &source, None)
            .expect("video");
        builder
            .add_cdg_source(entry(12), "media/0012.mp3", &audio, &graphics, None)
            .expect("cdg");
        builder.write(&path).expect("write");

        let file = std::fs::File::open(&path).expect("open");
        let container = Container::open(file).expect("container");
        let names: Vec<&str> = container.entries().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names.last().copied(),
            Some(MANIFEST_PATH),
            "the manifest goes last, so correcting a title is a truncation rather than a copy"
        );

        // The two a decoder seeks into. Stored, and their two lengths agree — a directory claiming
        // otherwise would make the window read on into whatever follows it.
        for name in ["media/0002.mp4", "media/0012.mp3"] {
            let media = container.entry(name).expect("media");
            assert_eq!(
                media.method,
                Method::Stored,
                "{name} is seeked into, so it must be stored"
            );
            assert_eq!(media.stored_len, media.real_len);
        }

        // The one that is read whole. Deflated, and the saving is real rather than nominal.
        let cdg = container.entry("media/0012.cdg").expect("graphics");
        assert_eq!(
            cdg.method,
            Method::Deflate,
            "nothing seeks into a .cdg, so it is deflated"
        );
        assert!(
            cdg.stored_len < cdg.real_len,
            "a deflated .cdg that did not shrink would mean the method never took effect: \
             {} not smaller than {}",
            cdg.stored_len,
            cdg.real_len
        );
    }

    #[test]
    fn a_rebuild_copies_media_across_without_re_encoding_it() {
        let dir = temp_dir("write-copied");
        let source = dir.join("clip.mp4");
        std::fs::write(&source, pattern(30_000)).expect("source");

        let first = dir.join("vol1.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder
            .add_video_source(entry(2), "media/0002.mp4", &source, Some("abc".to_owned()))
            .expect("video");
        builder.write(&first).expect("write");

        // What `apply`, `edit` and `reanalyze` do: read the old package, change the manifest, write
        // a new one. The media goes across byte for byte.
        let old = Package::open(&first).expect("open");
        let mut corrected = old.manifest().song(2).expect("song").clone();
        corrected.title = "A better title".to_owned();
        let second = dir.join("vol2.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add_media_copied(corrected, &old).expect("copy");
        builder.write(&second).expect("write");

        let crc_of = |path: &Path| {
            let file = std::fs::File::open(path).expect("open");
            let container = Container::open(file).expect("container");
            let entry = container.entry("media/0002.mp4").expect("media");
            (entry.crc, entry.real_len, entry.stored_len, entry.method)
        };
        // CRC equality is the cheapest strong proof that the copy moved the bytes rather than
        // re-writing them, and it would catch a re-compression that happened to round-trip.
        assert_eq!(crc_of(&first), crc_of(&second));

        let rebuilt = Package::open(&second).expect("open");
        assert_eq!(
            rebuilt.manifest().song(2).expect("song").title,
            "A better title"
        );
        assert_eq!(
            rebuilt
                .manifest()
                .song(2)
                .expect("song")
                .content_hash
                .as_deref(),
            Some("abc"),
            "a media song's hash is its source's, and the source is not what the archive holds"
        );
        assert!(rebuilt.missing_entries().expect("check").is_empty());
    }

    #[test]
    fn two_builds_of_the_same_songs_are_byte_identical() {
        // The property the fixed stamp exists for, and the one that would rot silently: a package
        // that quietly differs from a rebuild of its own description cannot be checked against it.
        let dir = temp_dir("reproducible");
        let clip = dir.join("clip.mp4");
        std::fs::write(&clip, pattern(30_000)).expect("source");

        let build = |path: &Path| {
            let mut builder = PackageBuilder::new(meta());
            builder.add(entry(1), b"a song".to_vec()).expect("song");
            builder
                .add_video_source(
                    video_entry(2, "media/0002.mp4"),
                    "media/0002.mp4",
                    &clip,
                    None,
                )
                .expect("video");
            builder.write(path).expect("write");
        };

        let first = dir.join("first.kmpkg");
        let second = dir.join("second.kmpkg");
        build(&first);
        build(&second);

        assert_eq!(
            std::fs::read(&first).expect("read"),
            std::fs::read(&second).expect("read"),
            "the same songs and the same description must produce the same bytes"
        );
    }

    #[test]
    fn a_rebuild_of_an_unchanged_package_is_byte_identical_to_it() {
        // **A package says nothing about the machine that built it, structurally.** The container
        // has no field for a moment, a host or a mode, so a rebuild has nothing to carry forward and
        // nothing to stamp afresh — and the strongest way to say so is that rebuilding a package
        // without changing it reproduces it exactly, whenever and wherever that happens.
        let dir = temp_dir("rebuild-identical");
        let old_path = dir.join("old.kmpkg");
        let mut song = entry(12);
        song.kind = SongKind::Cdg;
        song.file = "media/0012.mp3".to_owned();
        hand_built(
            &old_path,
            vec![song.clone()],
            &[
                ("media/0012.mp3", &pattern(20_000), Method::Stored),
                ("media/0012.cdg", &pattern(40_000), Method::Deflate),
            ],
        );

        let old = Package::open(&old_path).expect("open");
        let rebuilt_path = dir.join("new.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add_media_copied(song, &old).expect("copy");
        builder.write(&rebuilt_path).expect("write");

        assert_eq!(
            std::fs::read(&old_path).expect("read"),
            std::fs::read(&rebuilt_path).expect("read"),
            "a rebuild that changed nothing must change nothing"
        );

        // The rebuild also has to be a byte copy: the whole reason it exists is that correcting one
        // title in a 20 GB package must not re-compress it.
        let entry_of = |path: &Path, name: &str| {
            let file = std::fs::File::open(path).expect("open");
            let container = Container::open(file).expect("container");
            let entry = container.entry(name).expect("entry");
            (entry.crc, entry.real_len, entry.stored_len, entry.method)
        };
        for name in ["media/0012.mp3", "media/0012.cdg"] {
            assert_eq!(entry_of(&old_path, name), entry_of(&rebuilt_path, name));
        }
    }

    #[test]
    fn a_package_whose_graphics_were_stored_still_reads_and_rebuilds_unchanged() {
        // Every MP3+G package built before the `.cdg` was deflated has a stored one, and those keep
        // playing: `graphics_bytes` reads the entry through the archive and so takes either method.
        // A rebuild must not quietly re-compress them either — `raw_copy_file` carries across
        // whatever it found, which is what lets old and new packages coexist with no converter.
        let dir = temp_dir("write-cdg-stored");
        let path = dir.join("vol1.kmpkg");
        let graphics = pattern(40_000);
        let mut song = entry(12);
        song.kind = SongKind::Cdg;
        song.file = "media/0012.mp3".to_owned();
        hand_built(
            &path,
            vec![song],
            &[
                ("media/0012.mp3", pattern(9_000).as_slice(), Method::Stored),
                ("media/0012.cdg", graphics.as_slice(), Method::Stored),
            ],
        );

        let old = Package::open(&path).expect("open");
        assert_eq!(
            old.graphics_bytes(12).expect("graphics"),
            graphics,
            "a stored .cdg still reads"
        );

        let mut corrected = old.manifest().song(12).expect("song").clone();
        corrected.title = "A better title".to_owned();
        let second = dir.join("vol2.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add_media_copied(corrected, &old).expect("copy");
        builder.write(&second).expect("write");

        let file = std::fs::File::open(&second).expect("open");
        let container = Container::open(file).expect("container");
        assert_eq!(
            container.entry("media/0012.cdg").expect("graphics").method,
            Method::Stored,
            "a rebuild carries the old method across rather than imposing the rule afresh"
        );
        drop(container);

        let rebuilt = Package::open(&second).expect("open");
        assert_eq!(rebuilt.graphics_bytes(12).expect("graphics"), graphics);
    }

    /// A manifest at a format this build does not write is refused, older or newer.
    ///
    /// **Every number outside [`FORMAT_VERSIONS_READ`]**, including the gaps between the ones it
    /// writes: a check written as "above the newest" would let a 2 or a 3 through to fail song by
    /// song, and one written as "below 4" would swallow format 1, which every MIDI-only package is.
    #[test]
    fn a_package_at_a_format_this_build_does_not_write_is_refused() {
        for version in [0, 2, 3, FORMAT_VERSION + 1] {
            let dir = temp_dir(&format!("unread-format-{version}"));
            let path = dir.join("vol1.kmpkg");

            let mut manifest = Manifest::new(meta());
            manifest.songs = vec![video_entry(2, "0002.mp4")];
            manifest.format = version;
            manifest_only(&path, &manifest);

            let error = Package::open(&path).expect_err("refused");
            let said = error.to_string();
            assert!(
                said.contains(&format!(
                    "manifest format {version} is not one this build reads"
                )),
                "{said}"
            );
        }
    }

    /// And a MIDI-only package is untouched by the bump, which is the whole point of by-content.
    #[test]
    fn a_midi_only_package_still_opens_and_is_still_format_one() {
        let dir = temp_dir("midi-only-format");
        let path = dir.join("vol1.kmpkg");

        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi bytes".to_vec()).expect("add");
        let manifest = builder.write(&path).expect("write");

        assert_eq!(manifest.format, FORMAT_VERSION_MIDI_ONLY);
        let package = Package::open(&path).expect("a MIDI package must go on opening");
        assert_eq!(package.len(), 1);
    }

    #[test]
    fn a_package_round_trips() {
        let dir = temp_dir("round-trip");
        let path = dir.join("vol1.kmpkg");

        let mut builder = PackageBuilder::new(meta());
        builder
            .add(entry(101), b"first song bytes".to_vec())
            .expect("add");
        builder
            .add(entry(102), b"second song bytes".to_vec())
            .expect("add");
        let written = builder.write(&path).expect("write");
        assert_eq!(written.songs.len(), 2);

        let package = Package::open(&path).expect("open");
        assert_eq!(package.len(), 2);
        assert_eq!(package.manifest().package.name, "Volume 1");
        assert_eq!(package.read_song(101).expect("read"), b"first song bytes");
        assert_eq!(package.read_song(102).expect("read"), b"second song bytes");
    }

    #[test]
    fn a_file_path_is_generated_when_none_is_given() {
        let mut builder = PackageBuilder::new(meta());
        let added = builder.add(entry(4_242), b"x".to_vec()).expect("add");
        assert_eq!(added.file, "midi/4242.mid");
    }

    #[test]
    fn a_content_hash_is_recorded_for_every_song() {
        let mut builder = PackageBuilder::new(meta());
        let added = builder.add(entry(1), b"some midi".to_vec()).expect("add");
        assert!(added.content_hash.is_some());
    }

    #[test]
    fn identical_songs_hash_identically_and_different_ones_do_not() {
        assert_eq!(content_hash(b"same"), content_hash(b"same"));
        assert_ne!(content_hash(b"same"), content_hash(b"different"));
        // 16 bytes as hex.
        assert_eq!(content_hash(b"x").len(), 32);
    }

    #[test]
    fn adding_the_same_number_twice_is_refused() {
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(5), b"a".to_vec()).expect("add");
        let result = builder.add(entry(5), b"b".to_vec());
        assert!(matches!(result, Err(PackageError::DuplicateNumber(5))));
    }

    #[test]
    fn writing_a_package_with_duplicate_content_is_refused() {
        // The same recording under two numbers. Shipping it would put a duplicate in the
        // catalog, which is exactly the defect this corpus produces.
        let dir = temp_dir("dup-content");
        let mut builder = PackageBuilder::new(meta());
        builder
            .add(entry(1), b"identical bytes".to_vec())
            .expect("add");
        builder
            .add(entry(2), b"identical bytes".to_vec())
            .expect("add");

        let result = builder.write(dir.join("dup.kmpkg"));
        match result {
            Err(PackageError::Invalid { problems, .. }) => assert!(
                problems
                    .iter()
                    .any(|p| matches!(p, ManifestProblem::DuplicateContent { .. })),
                "got {problems:?}"
            ),
            other => panic!("expected a duplicate-content rejection, got {other:?}"),
        }
    }

    #[test]
    fn writing_an_empty_package_is_refused() {
        let dir = temp_dir("empty");
        let builder = PackageBuilder::new(meta());
        assert!(builder.write(dir.join("empty.kmpkg")).is_err());
    }

    #[test]
    fn opening_a_missing_file_reports_it_clearly() {
        let result = Package::open("definitely/not/here.kmpkg");
        assert!(matches!(result, Err(PackageError::Io { .. })));
    }

    #[test]
    fn a_file_that_was_never_a_package_is_rejected() {
        let dir = temp_dir("not-a-package");
        let path = dir.join("nope.kmpkg");
        std::fs::write(&path, b"this is not a package, it is a note").expect("write");
        assert!(matches!(
            Package::open(&path),
            Err(PackageError::NotAPackage(_))
        ));
    }

    #[test]
    fn a_package_without_a_manifest_is_rejected() {
        let dir = temp_dir("no-manifest");
        let path = dir.join("bare.kmpkg");
        let file = std::fs::File::create(&path).expect("create");
        let mut writer = Writer::new(file).expect("header");
        writer
            .write_entry("song.kar", Method::Deflate, b"midi")
            .expect("entry");
        writer.finish().expect("finish");

        assert!(matches!(
            Package::open(&path),
            Err(PackageError::NoManifest(_))
        ));
    }

    /// A manifest is decompressed before anything has decided the package is real.
    ///
    /// **Every start opens every package in the scanned folders**, so this is the difference between
    /// a song that will not play and a machine that will not boot. Built as a real bomb rather than
    /// asserted against the constant: deflate does the compressing, so the file on disk is a few
    /// kilobytes and what it costs to open is what is under test.
    #[test]
    fn a_manifest_that_expands_past_the_ceiling_is_refused_rather_than_held() {
        let dir = temp_dir("manifest-bomb");
        let path = dir.join("bomb.kmpkg");
        let file = std::fs::File::create(&path).expect("create");
        let mut writer = Writer::new(file).expect("header");
        // One megabyte at a time, so the test holds a megabyte rather than the whole expansion.
        let block = vec![b' '; 1024 * 1024];
        let blocks = MAX_MANIFEST_BYTES / (1024 * 1024) + 2;
        let mut bomb = Repeating {
            block: &block,
            left: blocks,
            at: 0,
        };
        writer
            .stream_entry(MANIFEST_PATH, Method::Deflate, &mut bomb)
            .expect("start");
        writer.finish().expect("finish");

        let on_disk = std::fs::metadata(&path).expect("metadata").len();
        assert!(
            on_disk < 1024 * 1024,
            "the bomb should be small on disk, or it is not testing what it says: {on_disk} bytes"
        );

        match Package::open(&path) {
            Err(PackageError::EntryTooLarge { entry, limit, .. }) => {
                assert_eq!(entry, MANIFEST_PATH);
                assert_eq!(limit, MAX_MANIFEST_BYTES);
            }
            other => panic!("a manifest bomb must be refused by size, got {other:?}"),
        }
    }

    /// The other half: an ordinary manifest is nowhere near the ceiling and still opens.
    #[test]
    fn an_ordinary_package_is_far_inside_the_manifest_ceiling() {
        let dir = temp_dir("manifest-ordinary");
        let path = dir.join("vol.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi".to_vec()).expect("add");
        builder.write(&path).expect("write");

        assert!(Package::open(&path).is_ok());
    }

    #[test]
    fn reading_an_absent_song_number_is_an_error() {
        let dir = temp_dir("absent-song");
        let path = dir.join("vol.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"x".to_vec()).expect("add");
        builder.write(&path).expect("write");

        let package = Package::open(&path).expect("open");
        assert!(matches!(
            package.read_song(999),
            Err(PackageError::NoSuchSong(999))
        ));
    }

    #[test]
    fn a_manifest_naming_a_missing_file_is_detected() {
        let dir = temp_dir("missing-entry");
        let path = dir.join("broken.kmpkg");

        // Built by hand so the manifest names a file the archive does not contain.
        let mut manifest = Manifest::new(meta());
        let mut song = entry(7);
        song.file = "midi/7.kar".to_owned();
        manifest.songs.push(song);

        manifest_only(&path, &manifest);

        let package = Package::open(&path).expect("the manifest itself is valid");
        assert_eq!(package.missing_entries().expect("scan"), vec![7]);
        assert!(matches!(
            package.read_song(7),
            Err(PackageError::MissingEntry { number: 7, .. })
        ));
    }

    #[test]
    fn a_traversal_path_is_refused_at_read_time_not_followed() {
        let dir = temp_dir("traversal");
        let path = dir.join("evil.kmpkg");

        let mut manifest = Manifest::new(meta());
        let mut song = entry(1);
        song.file = "../escaped.kar".to_owned();
        manifest.songs.push(song);

        manifest_only(&path, &manifest);

        // Refused on open, because validation catches it before anything is read.
        assert!(matches!(
            Package::open(&path),
            Err(PackageError::Invalid { .. })
        ));
    }

    #[test]
    fn a_broken_manifest_can_still_be_read_for_diagnosis() {
        let dir = temp_dir("diagnose");
        let path = dir.join("broken.kmpkg");

        // Two songs with the same number: invalid, so Package::open refuses it.
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(entry(3));
        manifest.songs.push(entry(3));

        manifest_only(&path, &manifest);

        assert!(
            Package::open(&path).is_err(),
            "an invalid package must not open"
        );
        // But a tool asking *why* can still see inside.
        let raw = read_manifest_unchecked(&path).expect("unchecked read");
        assert_eq!(raw.songs.len(), 2);
        assert!(
            raw.problems()
                .contains(&ManifestProblem::DuplicateNumber(3))
        );
    }

    #[test]
    fn error_messages_name_the_file_and_the_problem() {
        let dir = temp_dir("messages");
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"a".to_vec()).expect("add");
        builder.add(entry(2), b"a".to_vec()).expect("add");
        let error = builder
            .write(dir.join("dup.kmpkg"))
            .expect_err("should refuse");
        let message = error.to_string();
        assert!(message.contains("dup.kmpkg"), "got {message}");
        assert!(
            message.contains("byte-identical"),
            "the message should say what is wrong: {message}"
        );
    }

    #[test]
    fn an_all_midi_package_is_still_written_as_format_one() {
        let dir = temp_dir("midi-format");
        let path = dir.join("vol1.kmpkg");

        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi".to_vec()).expect("add");
        let manifest = builder.write(&path).expect("write");

        assert_eq!(
            manifest.format, FORMAT_VERSION_MIDI_ONLY,
            "adopting video must not make every package unreadable to an older build"
        );
        let json = read_manifest_json(&path);
        assert!(
            !json.contains("\"kind\""),
            "a MIDI-only manifest should not mention kind at all: {json}"
        );
    }

    #[test]
    fn a_package_holding_a_video_declares_the_media_format() {
        let dir = temp_dir("video-format");
        let clip = dir.join("clip.mp4");
        std::fs::write(&clip, pattern(64)).expect("clip");
        let path = dir.join("vol1.kmpkg");

        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi".to_vec()).expect("add midi");
        builder
            .add_video_source(entry(2), "media/0002.mp4", &clip, Some("abc123".to_owned()))
            .expect("add video");
        let manifest = builder.write(&path).expect("write");

        // 4, and by content: a MIDI-only package is 1. Video and MP3+G share this one constant
        // because both live inside the archive and differ in nothing a format version protects.
        assert_eq!(manifest.format, FORMAT_VERSION_MEDIA);
        assert!(manifest.has_video());
        assert!(!manifest.has_cdg());
        let json = read_manifest_json(&path);
        assert!(json.contains("\"kind\": \"video\""), "{json}");
    }

    #[test]
    fn a_package_holding_both_kinds_of_media_declares_the_same_format() {
        let dir = temp_dir("cdg-format");
        let clip = dir.join("clip.mp4");
        let audio = dir.join("song.mp3");
        let graphics = dir.join("song.cdg");
        for (file, len) in [(&clip, 64), (&audio, 32), (&graphics, 16)] {
            std::fs::write(file, pattern(len)).expect("source");
        }
        let path = dir.join("vol1.kmpkg");

        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi".to_vec()).expect("add midi");
        builder
            .add_video_source(entry(2), "media/0002.mp4", &clip, Some("abc123".to_owned()))
            .expect("add video");
        builder
            .add_cdg_source(
                entry(3),
                "media/0003.mp3",
                &audio,
                &graphics,
                Some("def456".to_owned()),
            )
            .expect("add cdg");
        let manifest = builder.write(&path).expect("write");

        // Both kinds, one version. There is nothing left for a second number to protect, since
        // neither can be read by a build that predates the move.
        assert_eq!(manifest.format, FORMAT_VERSION_MEDIA);
        assert!(manifest.has_cdg());
        assert!(manifest.has_video());
        let json = read_manifest_json(&path);
        assert!(json.contains("\"kind\": \"cdg\""), "{json}");
    }

    #[test]
    fn an_mp3_plus_g_entry_naming_something_that_is_not_audio_is_a_problem() {
        // Its graphics are found from the audio's name, so a `file` that is not audio makes them
        // unfindable — and the failure would otherwise surface at singing time.
        let mut manifest = Manifest::new(meta());
        let mut song = entry(4);
        song.kind = SongKind::Cdg;
        song.file = "0004.wav".to_owned();
        manifest.songs.push(song);

        assert!(
            manifest
                .problems()
                .iter()
                .any(|problem| matches!(problem, ManifestProblem::FileNotAudio { number: 4, .. })),
            "{:?}",
            manifest.problems()
        );
    }

    fn a_timeline() -> km_song::LyricTimeline {
        km_song::ultrastar::parse(
            b"#TITLE:Song\n#MP3:song.mp3\n#BPM:300\n: 0 4 0 Hel\n: 4 2 0 lo\n- 8\n: 10 6 0 world\nE\n",
        )
        .expect("parses")
        .timeline
    }

    #[test]
    fn an_ultrastar_song_round_trips_as_its_audio_and_its_timeline() {
        let dir = temp_dir("write-ultrastar");
        let audio = dir.join("song.mp3");
        let audio_bytes = pattern(9_000);
        std::fs::write(&audio, &audio_bytes).expect("audio");
        let timeline = a_timeline();

        let path = dir.join("vol1.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder.add(entry(1), b"midi".to_vec()).expect("midi");
        builder
            .add_timeline_source(
                SongKind::UltraStar,
                entry(12),
                "media/0012.mp3",
                &audio,
                &timeline,
                Some("abc123".to_owned()),
            )
            .expect("ultrastar");
        let manifest = builder.write(&path).expect("write");
        assert_eq!(manifest.format, FORMAT_VERSION_ULTRASTAR);
        assert!(read_manifest_json(&path).contains("\"kind\": \"ultrastar\""));

        let package = Package::open(&path).expect("open");
        assert!(package.missing_entries().expect("check").is_empty());
        let mut window = package.media_reader(12).expect("audio window");
        let mut read_back = Vec::new();
        window.read_to_end(&mut read_back).expect("read");
        assert_eq!(read_back, audio_bytes);
        assert_eq!(package.lyric_timeline(12).expect("timeline"), timeline);
        assert!(matches!(
            package.lyric_timeline(1),
            Err(PackageError::NotMedia { number: 1, .. })
        ));

        let entries = package.media_entries().expect("entries");
        let names: Vec<(&str, bool)> = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.stored))
            .collect();
        assert_eq!(
            names,
            [("media/0012.mp3", true), ("media/0012.json", false)],
            "the audio is seeked into and the timeline is read whole"
        );

        // A rebuild carries both entries across.
        let rebuilt = dir.join("vol1-rebuilt.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder
            .add_media_copied(package.manifest().song(12).expect("song").clone(), &package)
            .expect("copied");
        builder.write(&rebuilt).expect("write rebuilt");
        let rebuilt = Package::open(&rebuilt).expect("open rebuilt");
        assert_eq!(rebuilt.lyric_timeline(12).expect("timeline"), timeline);
    }

    #[test]
    fn an_lrc_song_is_stored_as_an_ultrastar_song_is_under_its_own_kind_and_format() {
        let dir = temp_dir("write-lrc");
        let audio = dir.join("song.mp3");
        std::fs::write(&audio, pattern(4_000)).expect("audio");
        let timeline = km_song::lrc::parse(b"[00:01.00]First line\n[00:04.00]Second line\n")
            .expect("parses")
            .timeline;

        let path = dir.join("vol1.kmpkg");
        let mut builder = PackageBuilder::new(meta());
        builder
            .add_timeline_source(
                SongKind::Lrc,
                entry(3),
                "media/0003.mp3",
                &audio,
                &timeline,
                None,
            )
            .expect("lrc");
        builder
            .add_timeline_source(
                SongKind::UltraStar,
                entry(4),
                "media/0004.mp3",
                &audio,
                &a_timeline(),
                None,
            )
            .expect("ultrastar");
        let manifest = builder.write(&path).expect("write");
        assert_eq!(manifest.format, FORMAT_VERSION_LRC);
        assert_eq!(manifest.format, FORMAT_VERSION);
        assert!(read_manifest_json(&path).contains("\"kind\": \"lrc\""));

        let package = Package::open(&path).expect("open");
        assert!(package.missing_entries().expect("check").is_empty());
        assert_eq!(package.lyric_timeline(3).expect("timeline"), timeline);
        assert_eq!(SongKind::from_wire("lrc"), SongKind::Lrc);
        assert_eq!(
            companion_entry_for(SongKind::Lrc, "media/0003.mp3").as_deref(),
            Some("media/0003.json")
        );
    }

    #[test]
    fn an_lrc_file_finds_the_audio_with_its_stem() {
        let dir = temp_dir("lrc-sibling");
        let lyrics = dir.join("Someone - Song.lrc");
        std::fs::write(&lyrics, "[00:01.00]words").expect("lyrics");
        assert_eq!(sibling_with_extension(&lyrics, &AUDIO_EXTENSIONS), None);
        std::fs::write(dir.join("Someone - Song.MP3"), pattern(16)).expect("audio");
        let found = sibling_with_extension(&lyrics, &AUDIO_EXTENSIONS).expect("found");
        // Compared without case: a file system that ignores it opens the upper-case file under the
        // lower-case spelling tried first, and hands that spelling back.
        assert!(found.is_file(), "{found:?}");
        assert!(
            found
                .to_string_lossy()
                .to_lowercase()
                .ends_with("someone - song.mp3"),
            "{found:?}"
        );
        assert!(is_lrc_file(&lyrics));
    }

    #[test]
    fn an_ultrastar_song_without_its_timeline_is_not_a_song() {
        let dir = temp_dir("write-ultrastar-half");
        let path = dir.join("vol1.kmpkg");
        let mut song = entry(12);
        song.kind = SongKind::UltraStar;
        song.file = "media/0012.mp3".to_owned();
        hand_built(
            &path,
            vec![song],
            &[("media/0012.mp3", pattern(64).as_slice(), Method::Stored)],
        );

        let package = Package::open(&path).expect("open");
        assert_eq!(package.missing_entries().expect("check"), vec![12]);
    }

    #[test]
    fn a_kind_from_a_newer_build_reads_as_unknown_rather_than_failing_the_manifest() {
        // The catch-all that was added a kind too late to help: without it an unrecognized `kind`
        // fails the whole manifest with a JSON error, before the format check can name the version.
        // Built from the real types and then edited, rather than hand-written: a literal manifest
        // here would be a second copy of the schema to keep in step.
        let mut manifest = Manifest::new(meta());
        let mut song = entry(1);
        song.kind = SongKind::Video;
        manifest.songs.push(song);

        let json = serde_json::to_string(&manifest)
            .expect("serializes")
            .replace("\"video\"", "\"hologram\"");

        let read: Manifest = serde_json::from_str(&json).expect("an unknown kind still parses");
        assert_eq!(read.songs[0].kind, SongKind::Unknown);
        // Not MIDI, so nothing tries to read it as MIDI — and `missing_entries` counts it missing
        // whatever the archive holds, because what its bytes *are* is exactly what is not understood.
        assert!(!read.songs[0].kind.is_midi());
    }

    #[test]
    fn a_manifest_without_kind_reads_as_midi() {
        // Exactly what a package built before video existed looks like.
        let json = r#"{
            "format": 1,
            "package": { "id": "1f4a9c8e2b7d0356", "name": "Old", "version": "1.0.0" },
            "songs": [
                { "number": 1, "title": "Anything", "file": "midi/1.mid", "duration_ms": 1000 }
            ]
        }"#;
        let manifest: Manifest = serde_json::from_str(json).expect("older manifests still parse");
        assert_eq!(manifest.songs[0].kind, SongKind::Midi);
        assert!(!manifest.has_video());
        assert!(manifest.problems().is_empty());
    }

    #[test]
    fn a_package_with_a_language_this_build_does_not_know_still_opens() {
        // `Package::open` runs the same `manifest.problems()` that `PackageBuilder::write` does, so a
        // language rule added to `ManifestProblem` would make a package from a newer build, carrying
        // a code this one has never heard of, unopenable. Language is therefore checked by the
        // *packagers* -- `km-pack build` and `km-package-builder`'s build -- and never here.
        let dir = temp_dir("unknown-language");
        let path = dir.join("vol1.kmpkg");

        let mut builder = PackageBuilder::new(meta());
        let mut song = entry(101);
        song.language = Some("zz".to_owned());
        builder.add(song, b"song bytes".to_vec()).expect("add");
        builder.write(&path).expect("write");

        let package = Package::open(&path).expect("a code this build does not know still opens");
        assert_eq!(
            package.manifest().songs[0].language.as_deref(),
            Some("zz"),
            "the raw value is handed back untouched -- the wire is open though the type is closed"
        );
    }

    /// Pulls the manifest back out of a written package, to assert on what was actually stored.
    fn read_manifest_json(path: &Path) -> String {
        let file = std::fs::File::open(path).expect("open package");
        let mut container = Container::open(file).expect("container");
        let entry = container.entry(MANIFEST_PATH).expect("manifest").clone();
        let bytes = container
            .read_entry(&entry, MAX_MANIFEST_BYTES)
            .ok()
            .expect("read");
        String::from_utf8(bytes).expect("text")
    }
}
