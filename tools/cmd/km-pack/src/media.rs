//! Where one file of a song is: on disk, or inside a zip.
//!
//! **One type for both, so a song is packaged by one function whichever it is.** The package built
//! from a zipped song is the package its loose files build, and a second copy of the packaging code
//! is how the two would come to differ.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

/// The most entries a zip may list and still be read as a song.
///
/// A zip past this is refused and never read in part, because the entries left unread could hold a
/// second song. A zipped song is a handful of files.
pub const MAX_ZIP_ENTRIES: usize = 2_000;

/// The most of a zipped MP3 this build holds in memory.
///
/// An hour at 128 kbit/s. The audio is held while one song is probed, measured and hashed.
pub const MAX_ZIPPED_AUDIO: u64 = 64 * 1024 * 1024;

/// The most of a zipped `.cdg` this build holds in memory.
///
/// CD+G runs at 7,200 bytes a second, so this is over two hours of it.
pub const MAX_ZIPPED_GRAPHICS: u64 = 64 * 1024 * 1024;

/// The most of a zipped `.txt` or `.lrc` this build holds in memory.
pub const MAX_ZIPPED_TEXT: u64 = 4 * 1024 * 1024;

/// One entry of a zip, named by where it is and not by its bytes.
///
/// The archive is opened again for each read, so nothing holds a handle that a rewritten zip would
/// leave stale. Each read checks the entry still has the name and size the listing gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipMember {
    /// The zip.
    pub archive: PathBuf,
    /// The entry's place in the zip's directory.
    pub index: usize,
    /// The entry's name, with forward slashes.
    pub name: String,
    /// How long the entry says its bytes are.
    pub size: u64,
}

impl ZipMember {
    /// Opens the entry and hands `read` a reader over its bytes.
    fn open<T>(&self, read: impl FnOnce(&mut dyn Read) -> T) -> std::io::Result<T> {
        let file = std::fs::File::open(&self.archive)?;
        let mut zip =
            zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(std::io::Error::other)?;
        let mut entry = zip.by_index(self.index).map_err(std::io::Error::other)?;
        if entry_name(entry.name()) != self.name || entry.size() != self.size {
            return Err(std::io::Error::other(
                "the zip changed while it was being read",
            ));
        }
        Ok(read(&mut entry))
    }

    /// Reads the whole entry, refusing one past `limit` bytes.
    ///
    /// The size a zip states is the zip's own claim, so the read is what is bounded.
    ///
    /// # Errors
    ///
    /// If the zip cannot be read, or the entry is larger than `limit` or than it said.
    pub fn read(&self, limit: u64) -> std::io::Result<Vec<u8>> {
        let too_large =
            || std::io::Error::other(format!("it is larger than {} MiB", limit / (1024 * 1024)));
        if self.size > limit {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        self.open(|entry| entry.take(limit.saturating_add(1)).read_to_end(&mut bytes))??;
        if bytes.len() as u64 > limit {
            return Err(too_large());
        }
        Ok(bytes)
    }

    /// The entry's file name, without the folders above it.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

impl std::fmt::Display for ZipMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} in {}", self.name, self.archive.display())
    }
}

impl km_kmpkg::EntrySource for ZipMember {
    fn name(&self) -> String {
        self.to_string()
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn with_reader(&self, read: &mut dyn FnMut(&mut dyn Read)) -> std::io::Result<()> {
        self.open(|entry| read(entry))
    }
}

/// A zip entry's name as this crate compares it: forward slashes only.
///
/// A zip written on Windows by an old tool separates folders with a backslash.
pub(crate) fn entry_name(raw: &str) -> String {
    raw.replace('\\', "/")
}

/// One file of a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Media {
    /// A file on disk.
    Disk(PathBuf),
    /// An entry of a zip.
    Zipped(ZipMember),
}

impl Media {
    /// The file's own name as a path, for its stem and its extension.
    fn named(&self) -> &Path {
        match self {
            Self::Disk(path) => path,
            Self::Zipped(member) => Path::new(member.file_name()),
        }
    }

    /// The last-resort title: the file's name without its extension.
    #[must_use]
    pub fn stem(&self) -> String {
        crate::file_stem(self.named())
    }

    /// The file's extension in lower case, without the dot. `mp3` when it has none.
    #[must_use]
    pub fn extension(&self) -> String {
        crate::extension_of(self.named())
    }

    /// Where the package writer takes the bytes from.
    #[must_use]
    pub fn source(&self) -> km_kmpkg::Source {
        match self {
            Self::Disk(path) => km_kmpkg::Source::Path(path.clone()),
            Self::Zipped(member) => km_kmpkg::Source::Entry(Arc::new(member.clone())),
        }
    }

    /// Makes the bytes ready to be probed, measured and hashed.
    ///
    /// A file on disk stays on disk and is streamed by each of those. A zip entry is read once, up
    /// to `limit` bytes, because each of them would otherwise inflate it again.
    ///
    /// # Errors
    ///
    /// If a zip entry cannot be read or is larger than `limit`.
    pub fn load(&self, limit: u64) -> Result<Loaded> {
        match self {
            Self::Disk(path) => Ok(Loaded::Disk(path.clone())),
            Self::Zipped(member) => {
                let bytes = member
                    .read(limit)
                    .with_context(|| format!("reading {member}"))?;
                Ok(Loaded::Bytes {
                    bytes: bytes.into(),
                    name: member.to_string(),
                })
            }
        }
    }
}

impl std::fmt::Display for Media {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disk(path) => write!(f, "{}", path.display()),
            Self::Zipped(member) => write!(f, "{member}"),
        }
    }
}

impl From<PathBuf> for Media {
    fn from(path: PathBuf) -> Self {
        Self::Disk(path)
    }
}

impl From<&Path> for Media {
    fn from(path: &Path) -> Self {
        Self::Disk(path.to_path_buf())
    }
}

/// One file of a song, ready to be read more than once.
#[derive(Debug, Clone)]
pub enum Loaded {
    /// A file on disk, streamed at each read.
    Disk(PathBuf),
    /// A zip entry's bytes.
    Bytes {
        /// The bytes.
        bytes: Arc<[u8]>,
        /// What to call them in an error.
        name: String,
    },
}

impl Loaded {
    /// What the audio is. See [`km_cdg::probe_audio`].
    pub fn probe_audio(&self) -> Result<km_cdg::AudioInfo, km_cdg::CdgError> {
        match self {
            Self::Disk(path) => km_cdg::probe_audio(path),
            Self::Bytes { bytes, name } => {
                km_cdg::probe_audio_from(Cursor::new(Arc::clone(bytes)), name)
            }
        }
    }

    /// How loud the audio is. See [`km_cdg::measure_loudness`].
    pub fn measure_loudness(&self) -> Result<Option<km_loudness::Loudness>, km_cdg::CdgError> {
        match self {
            Self::Disk(path) => km_cdg::measure_loudness(path),
            Self::Bytes { bytes, name } => {
                km_cdg::measure_loudness_from(Cursor::new(Arc::clone(bytes)), name)
            }
        }
    }

    /// What both halves of an MP3+G song are, with `self` the audio. See [`km_cdg::probe`].
    pub fn probe_with(&self, graphics: &Self) -> Result<km_cdg::CdgInfo, km_cdg::CdgError> {
        match (self, graphics) {
            (Self::Disk(audio), Self::Disk(graphics)) => km_cdg::probe(audio, graphics),
            (
                Self::Bytes { bytes, name },
                Self::Bytes {
                    bytes: graphics,
                    name: graphics_name,
                },
            ) => km_cdg::probe_from(
                Cursor::new(Arc::clone(bytes)),
                name,
                graphics,
                graphics_name,
            ),
            // A song is two files on disk or two entries of one zip, and never one of each.
            _ => Err(km_cdg::CdgError::Io {
                path: self.name(),
                source: std::io::Error::other("the two halves are not in the same place"),
            }),
        }
    }

    /// The hash of `self` and then `second`, as one song's identity.
    ///
    /// The same digest wherever the bytes are, so a zipped song and its loose files are one song.
    pub fn hash_with(&self, second: &Self) -> std::io::Result<String> {
        match (self, second) {
            (Self::Disk(first), Self::Disk(second)) => {
                km_kmpkg::pair_content_hash_of(first, second)
            }
            (Self::Bytes { bytes: first, .. }, Self::Bytes { bytes: second, .. }) => {
                Ok(km_kmpkg::pair_content_hash(first, second))
            }
            _ => Err(std::io::Error::other(
                "the two halves are not in the same place",
            )),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Disk(path) => path.display().to_string(),
            Self::Bytes { name, .. } => name.clone(),
        }
    }
}

/// Whether a zip entry is something an operating system added and nobody chose.
///
/// A zip made on macOS carries an `__MACOSX` folder of `._` files, one beside each real file and
/// under its name. One of those beside an MP3 has the MP3's extension and none of its audio.
#[must_use]
pub fn is_os_junk(name: &str) -> bool {
    let mut parts = name.split('/').rev();
    let file = parts.next().unwrap_or(name);
    file.starts_with("._")
        || [".ds_store", "thumbs.db", "desktop.ini"]
            .iter()
            .any(|junk| file.eq_ignore_ascii_case(junk))
        || name.split('/').any(|part| part == "__MACOSX")
}
