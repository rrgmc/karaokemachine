//! The container a `.kmpkg` is: magic bytes, entries, a directory, a footer.
//!
//! **A package is not an archive a file manager can open.** It is handed to a stranger, and a file
//! that opens as a folder invites renaming a song, swapping an audio file or deleting the manifest.
//! Nothing here is encryption and nothing here hides the directory: what it does is refuse to be a
//! format every desktop already knows how to take apart. See `A package is not an archive a file
//! manager can open` in `docs/decisions/packaging.md`.
//!
//! **The shape is what seeking into media in place requires.** An entry is a byte range in the file,
//! recorded in one directory, so a decoder plays a video by learning `(offset, len)` and then reading
//! the package *as a file* from that offset. Nothing is extracted and nothing is held.
//!
//! ```text
//! 0                header: magic, container version
//! 16               the entries, back to back
//!                  the directory: one record per entry
//!                  footer: where the directory is, and the closing magic
//! ```
//!
//! Three properties the layout is chosen for:
//!
//! - **The directory is the only table.** An entry carries no header of its own, so there is no
//!   second copy of a length for the two to disagree about.
//! - **The footer is a fixed size at a fixed end.** Opening is two seeks and no search, and a file
//!   that lost its tail fails on the closing magic rather than on a signature found inside stored
//!   media.
//! - **The manifest is written last**, immediately before the directory, so correcting a title can be
//!   a truncation and a re-append rather than a copy of every video in the package.
//!
//! Every offset and length is `u64` and every integer is little-endian. The television runs a 32-bit
//! OS, so a 500 MB entry six gigabytes into a package is arithmetic that must not pass through
//! `usize` on the way; and an endianness a reader has to guess is one that will be guessed wrong.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};

use crc32fast::Hasher;
use flate2::Compression;
use flate2::write::{DeflateDecoder, DeflateEncoder};

/// The first eight bytes of every package.
///
/// `KMPKG` so that a hex dump says what the file is, then `0x1a` so that a terminal asked to print
/// one stops at the end of the name instead of filling the screen with a video.
pub(crate) const MAGIC: [u8; 8] = [b'K', b'M', b'P', b'K', b'G', 0x1a, 0x00, 0x00];

/// The container this build writes and the only one it reads.
///
/// Separate from the manifest's own `format`, which says what *kind of songs* a package holds. This
/// says how to find them.
pub(crate) const CONTAINER_VERSION: u16 = 1;

/// Magic, version, and room to grow without moving the first entry.
pub(crate) const HEADER_LEN: u64 = 16;

/// The last eight bytes of every package. Distinct from [`MAGIC`] so that a file cut in half cannot
/// end in something that reads like a whole one.
pub(crate) const FOOTER_MAGIC: [u8; 8] = *b"KMPKGEND";

/// Directory offset, directory length, closing magic.
pub(crate) const FOOTER_LEN: u64 = 24;

/// The most of a directory this build will hold in memory.
///
/// A package holds at most 999 songs and so at most a couple of thousand entries, each record a name
/// and five numbers. This leaves two orders of magnitude over a full one, and it exists for the
/// reason every ceiling here exists: the directory is read before anything has decided the package is
/// real, and a length is a claim made by whoever wrote the file.
pub(crate) const MAX_DIRECTORY_BYTES: u64 = 4 * 1024 * 1024;

/// How much of an entry is read at a time, when it is copied rather than held.
///
/// A video is hundreds of megabytes, so the peak is this buffer rather than the file.
const COPY_BUFFER: usize = 1 << 20;

/// What every deflated entry is compressed at.
///
/// **Named rather than taken from `Compression::default()`**, because a package's bytes are a
/// reproducibility promise: two builds of the same songs are byte-identical, and a default that moved
/// underneath this would break that quietly and everywhere at once.
const DEFLATE_LEVEL: Compression = Compression::new(6);

/// How an entry's bytes are held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// Written as they are, so the entry is a byte range a decoder can seek into.
    Stored,
    /// Deflated, so the entry has to be read from its start.
    Deflate,
}

impl Method {
    /// What the directory records.
    const fn code(self) -> u8 {
        match self {
            Self::Stored => 0,
            Self::Deflate => 1,
        }
    }

    /// The method a directory record names, or `None` for one from a build that knew another.
    const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Stored),
            1 => Some(Self::Deflate),
            _ => None,
        }
    }
}

/// One entry, as the directory records it.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    /// Its name, with forward slashes.
    pub(crate) name: String,
    /// How its bytes are held.
    pub(crate) method: Method,
    /// Where its bytes begin, from the start of the package.
    pub(crate) offset: u64,
    /// How many bytes it occupies. For a stored entry this is also [`Entry::real_len`].
    pub(crate) stored_len: u64,
    /// How many bytes it becomes.
    pub(crate) real_len: u64,
    /// CRC-32 of those bytes, so a damaged package can be diagnosed rather than merely played wrong.
    pub(crate) crc: u32,
}

/// Why a container could not be read or written.
#[derive(Debug)]
pub(crate) enum ContainerError {
    /// The read or write itself failed.
    Io(std::io::Error),
    /// It does not begin the way a package does.
    NotAPackage,
    /// It names a container this build does not read.
    UnsupportedContainer(u16),
    /// It is a package whose structure does not hold together, with what is wrong with it.
    Malformed(String),
}

impl From<std::io::Error> for ContainerError {
    fn from(source: std::io::Error) -> Self {
        Self::Io(source)
    }
}

/// Why a capped read stopped.
pub(crate) enum CappedRead {
    /// The entry went on past the ceiling.
    TooLarge,
    /// The read itself failed.
    Io(std::io::Error),
}

/// Reads at most `limit` bytes, refusing more rather than holding them.
///
/// **The cap is on the bytes that arrive, not on the length the directory claims.** A directory is
/// written by whoever made the file, so sizing a buffer from it hands an attacker the allocation —
/// and `Vec::with_capacity` on a claimed `2^63` reaches `handle_alloc_error`, which aborts the
/// process and cannot be caught. The claim is still used, clamped, because it is right almost always
/// and one allocation beats growing a buffer twenty times.
///
/// Reading `limit + 1` is what tells a file that exactly fills the ceiling from one that runs past
/// it, without holding the overrun.
pub(crate) fn read_capped(
    reader: &mut impl Read,
    declared: u64,
    limit: u64,
) -> Result<Vec<u8>, CappedRead> {
    let hint = usize::try_from(declared.min(limit)).unwrap_or(usize::MAX);
    let mut bytes = Vec::with_capacity(hint);
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(CappedRead::Io)?;
    if bytes.len() as u64 > limit {
        return Err(CappedRead::TooLarge);
    }
    Ok(bytes)
}

/// An open container: the reader it came from, and every entry's byte range.
#[derive(Debug)]
pub(crate) struct Container<R> {
    reader: R,
    entries: BTreeMap<String, Entry>,
    /// The order they were written in, which is the order a listing reports them.
    order: Vec<String>,
}

impl<R: Read + Seek> Container<R> {
    /// Reads a container's header, footer and directory.
    ///
    /// **Every entry is checked against where the directory begins**, so a half-copied package fails
    /// here rather than six seconds into a song. A directory read into memory makes a truncated file
    /// open perfectly otherwise, and the fault then surfaces as garbage on the television.
    pub(crate) fn open(mut reader: R) -> Result<Self, ContainerError> {
        let total = reader.seek(SeekFrom::End(0))?;
        if total < HEADER_LEN + FOOTER_LEN {
            return Err(ContainerError::NotAPackage);
        }

        reader.rewind()?;
        let mut header = [0_u8; HEADER_LEN as usize];
        reader.read_exact(&mut header)?;
        if header[..8] != MAGIC {
            return Err(ContainerError::NotAPackage);
        }
        let version = u16::from_le_bytes([header[8], header[9]]);
        if version != CONTAINER_VERSION {
            return Err(ContainerError::UnsupportedContainer(version));
        }

        reader.seek(SeekFrom::Start(total - FOOTER_LEN))?;
        let mut footer = [0_u8; FOOTER_LEN as usize];
        reader.read_exact(&mut footer)?;
        if footer[16..24] != FOOTER_MAGIC {
            return Err(ContainerError::Malformed(
                "it does not end the way a package ends; it is truncated".to_owned(),
            ));
        }
        let directory_at = u64::from_le_bytes(footer[0..8].try_into().expect("eight bytes"));
        let directory_len = u64::from_le_bytes(footer[8..16].try_into().expect("eight bytes"));

        if directory_len > MAX_DIRECTORY_BYTES {
            return Err(ContainerError::Malformed(format!(
                "its directory claims {directory_len} bytes, past the {MAX_DIRECTORY_BYTES} this \
                 build will read"
            )));
        }
        let directory_end = directory_at.checked_add(directory_len).ok_or_else(|| {
            ContainerError::Malformed("its directory overflows the file".to_owned())
        })?;
        if directory_at < HEADER_LEN || directory_end != total - FOOTER_LEN {
            return Err(ContainerError::Malformed(
                "its directory is not where its footer says it is".to_owned(),
            ));
        }

        reader.seek(SeekFrom::Start(directory_at))?;
        let mut directory = vec![0_u8; directory_len as usize];
        reader.read_exact(&mut directory)?;

        let (entries, order) = parse_directory(&directory, directory_at)?;
        Ok(Self {
            reader,
            entries,
            order,
        })
    }

    /// One entry by name.
    pub(crate) fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    /// Whether it holds an entry of that name.
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Every entry, in the order they were written.
    pub(crate) fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.order.iter().map(|name| {
            self.entries
                .get(name)
                .expect("named by the order it was read in")
        })
    }

    /// One entry read from its start, whatever it is held as, refusing one that expands past `limit`.
    pub(crate) fn read_entry(&mut self, entry: &Entry, limit: u64) -> Result<Vec<u8>, CappedRead> {
        self.reader
            .seek(SeekFrom::Start(entry.offset))
            .map_err(CappedRead::Io)?;
        let mut ranged = (&mut self.reader).take(entry.stored_len);
        match entry.method {
            Method::Stored => read_capped(&mut ranged, entry.real_len, limit),
            Method::Deflate => {
                // Inflated through the same ceiling, counting the bytes that *arrive*: deflate
                // reaches roughly a thousand to one, so a package well inside the upload limit can
                // carry an entry that exhausts memory.
                let mut sink = CappedSink {
                    bytes: Vec::with_capacity(
                        usize::try_from(entry.real_len.min(limit)).unwrap_or(usize::MAX),
                    ),
                    limit,
                };
                let mut decoder = DeflateDecoder::new(&mut sink);
                match std::io::copy(&mut ranged, &mut decoder) {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WriteZero => {
                        return Err(CappedRead::TooLarge);
                    }
                    Err(error) => return Err(CappedRead::Io(error)),
                }
                match decoder.finish() {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WriteZero => {
                        return Err(CappedRead::TooLarge);
                    }
                    Err(error) => return Err(CappedRead::Io(error)),
                }
                Ok(sink.bytes)
            }
        }
    }

    /// Whether an entry's bytes still hash to what the directory says they do.
    ///
    /// **Streamed and discarded**, so checking a 20 GB package costs a read rather than a
    /// re-encode and holds nothing: the question is whether the bytes are the ones that were
    /// written, and answering it does not require keeping them.
    pub(crate) fn verify(&mut self, entry: &Entry) -> Result<bool, ContainerError> {
        self.reader.seek(SeekFrom::Start(entry.offset))?;
        let mut ranged = (&mut self.reader).take(entry.stored_len);
        let mut crc = Hasher::new();
        let mut sink = Hashing {
            crc: &mut crc,
            count: 0,
        };
        match entry.method {
            Method::Stored => {
                std::io::copy(&mut ranged, &mut sink)?;
            }
            Method::Deflate => {
                let mut decoder = DeflateDecoder::new(&mut sink);
                std::io::copy(&mut ranged, &mut decoder)?;
                decoder.finish()?;
            }
        }
        let counted = sink.count;
        Ok(crc.finalize() == entry.crc && counted == entry.real_len)
    }

    /// The reader the container was read through, for a caller that has already learned which entry
    /// it wants and is copying the bytes out.
    pub(crate) fn reader_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    /// The reader the container was read through, so a caller that learned a byte range can go on
    /// using the handle it already has.
    ///
    /// **There is then no gap** in which the file could be replaced between learning where an entry
    /// is and reading from it.
    pub(crate) fn into_inner(self) -> R {
        self.reader
    }
}

/// A sink that refuses to grow past a ceiling, so an inflating read stops rather than allocating.
struct CappedSink {
    bytes: Vec<u8>,
    limit: u64,
}

impl Write for CappedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len() as u64 + buf.len() as u64 > self.limit {
            // `WriteZero` rather than a bespoke kind, because it is what the copy above reads back
            // to tell "it ran over" from "the disk failed".
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Reads the directory's records, checking each against where the entries have to end.
fn parse_directory(
    directory: &[u8],
    directory_at: u64,
) -> Result<(BTreeMap<String, Entry>, Vec<String>), ContainerError> {
    let malformed = |what: &str| ContainerError::Malformed(what.to_owned());

    if directory.len() < 4 {
        return Err(malformed("its directory is too short to hold a count"));
    }
    let count = u32::from_le_bytes(directory[0..4].try_into().expect("four bytes"));
    let mut at = 4usize;

    let mut entries = BTreeMap::new();
    let mut order = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let take = |at: usize, len: usize| -> Result<&[u8], ContainerError> {
            directory
                .get(at..at + len)
                .ok_or_else(|| malformed("a directory record runs past the directory"))
        };

        let name_len = u16::from_le_bytes(take(at, 2)?.try_into().expect("two bytes")) as usize;
        at += 2;
        let name = std::str::from_utf8(take(at, name_len)?)
            .map_err(|_| malformed("a directory record names an entry that is not text"))?
            .to_owned();
        at += name_len;

        let method = Method::from_code(take(at, 1)?[0]).ok_or_else(|| {
            ContainerError::Malformed(format!("{name} is held in a way this build does not read"))
        })?;
        at += 1;
        let offset = u64::from_le_bytes(take(at, 8)?.try_into().expect("eight bytes"));
        at += 8;
        let stored_len = u64::from_le_bytes(take(at, 8)?.try_into().expect("eight bytes"));
        at += 8;
        let real_len = u64::from_le_bytes(take(at, 8)?.try_into().expect("eight bytes"));
        at += 8;
        let crc = u32::from_le_bytes(take(at, 4)?.try_into().expect("four bytes"));
        at += 4;

        // In `u64` throughout, and checked against where the directory begins rather than against
        // the file's length: an entry reaching into the directory is as wrong as one reaching past
        // the end, and only this comparison catches it.
        let end = offset.checked_add(stored_len).ok_or_else(|| {
            ContainerError::Malformed(format!("{name} claims a length that overflows the package"))
        })?;
        if offset < HEADER_LEN || end > directory_at {
            return Err(ContainerError::Malformed(format!(
                "{name} ends at {end} and the package's entries end at {directory_at}; it is \
                 truncated"
            )));
        }
        // A stored entry whose two lengths disagree would make a window read on into whatever
        // follows it.
        if method == Method::Stored && stored_len != real_len {
            return Err(ContainerError::Malformed(format!(
                "{name} is stored and claims two different lengths"
            )));
        }

        if entries
            .insert(
                name.clone(),
                Entry {
                    name: name.clone(),
                    method,
                    offset,
                    stored_len,
                    real_len,
                    crc,
                },
            )
            .is_some()
        {
            return Err(ContainerError::Malformed(format!(
                "{name} is in the directory twice"
            )));
        }
        order.push(name);
    }
    if at != directory.len() {
        return Err(malformed("its directory is longer than its records"));
    }
    Ok((entries, order))
}

/// Writes a container, entry by entry, in the order they are given.
pub(crate) struct Writer<W: Write> {
    writer: W,
    /// Where the next entry begins, which is also how much has been written.
    at: u64,
    entries: Vec<Entry>,
}

impl<W: Write> Writer<W> {
    /// Starts a package, writing its header.
    pub(crate) fn new(mut writer: W) -> Result<Self, ContainerError> {
        let mut header = [0_u8; HEADER_LEN as usize];
        header[..8].copy_from_slice(&MAGIC);
        header[8..10].copy_from_slice(&CONTAINER_VERSION.to_le_bytes());
        writer.write_all(&header)?;
        Ok(Self {
            writer,
            at: HEADER_LEN,
            entries: Vec::new(),
        })
    }

    /// Writes one entry from bytes already in memory.
    pub(crate) fn write_entry(
        &mut self,
        name: &str,
        method: Method,
        bytes: &[u8],
    ) -> Result<(), ContainerError> {
        self.stream_entry(name, method, &mut &bytes[..])?;
        Ok(())
    }

    /// Writes one entry from a reader, returning how many bytes it took from it.
    ///
    /// **Streamed, so the peak is the buffer rather than the file.** The count comes back so that a
    /// source which changed size underneath the build is caught rather than producing a package whose
    /// entry and manifest quietly disagree.
    pub(crate) fn stream_entry(
        &mut self,
        name: &str,
        method: Method,
        source: &mut impl Read,
    ) -> Result<u64, ContainerError> {
        let offset = self.at;
        let mut crc = Hasher::new();
        let mut counted = Counted {
            inner: &mut self.writer,
            count: 0,
        };
        let real_len = match method {
            Method::Stored => copy_hashing(source, &mut counted, &mut crc)?,
            Method::Deflate => {
                let mut encoder = DeflateEncoder::new(&mut counted, DEFLATE_LEVEL);
                let real = copy_hashing(source, &mut encoder, &mut crc)?;
                encoder.finish()?;
                real
            }
        };
        let stored_len = counted.count;
        self.at += stored_len;
        self.entries.push(Entry {
            name: name.to_owned(),
            method,
            offset,
            stored_len,
            real_len,
            crc: crc.finalize(),
        });
        Ok(real_len)
    }

    /// Takes one entry across from another package without decoding it.
    ///
    /// **A byte copy: no decode, no re-encode, flat memory.** This is what keeps correcting one title
    /// in a 20 GB package a copy rather than a re-encode, and it carries the CRC across so the new
    /// package says the same thing about those bytes as the old one did.
    pub(crate) fn raw_copy(
        &mut self,
        name: &str,
        source: &mut (impl Read + Seek),
        entry: &Entry,
    ) -> Result<(), ContainerError> {
        source.seek(SeekFrom::Start(entry.offset))?;
        let offset = self.at;
        let mut ranged = source.take(entry.stored_len);
        let copied = std::io::copy(&mut ranged, &mut self.writer)?;
        if copied != entry.stored_len {
            return Err(ContainerError::Malformed(format!(
                "{name} is {} bytes in the package it came from and {copied} were read",
                entry.stored_len
            )));
        }
        self.at += copied;
        self.entries.push(Entry {
            name: name.to_owned(),
            method: entry.method,
            offset,
            stored_len: entry.stored_len,
            real_len: entry.real_len,
            crc: entry.crc,
        });
        Ok(())
    }

    /// Writes the directory and the footer, and gives back what was being written to.
    pub(crate) fn finish(mut self) -> Result<W, ContainerError> {
        let directory_at = self.at;
        let mut directory = Vec::with_capacity(64 * self.entries.len() + 4);
        let count = u32::try_from(self.entries.len()).map_err(|_| {
            ContainerError::Malformed("it holds more entries than a package can name".to_owned())
        })?;
        directory.extend_from_slice(&count.to_le_bytes());
        for entry in &self.entries {
            let name_len = u16::try_from(entry.name.len()).map_err(|_| {
                ContainerError::Malformed(format!("{} is too long to name", entry.name))
            })?;
            directory.extend_from_slice(&name_len.to_le_bytes());
            directory.extend_from_slice(entry.name.as_bytes());
            directory.push(entry.method.code());
            directory.extend_from_slice(&entry.offset.to_le_bytes());
            directory.extend_from_slice(&entry.stored_len.to_le_bytes());
            directory.extend_from_slice(&entry.real_len.to_le_bytes());
            directory.extend_from_slice(&entry.crc.to_le_bytes());
        }
        self.writer.write_all(&directory)?;

        let mut footer = [0_u8; FOOTER_LEN as usize];
        footer[0..8].copy_from_slice(&directory_at.to_le_bytes());
        footer[8..16].copy_from_slice(&(directory.len() as u64).to_le_bytes());
        footer[16..24].copy_from_slice(&FOOTER_MAGIC);
        self.writer.write_all(&footer)?;
        self.writer.flush()?;
        Ok(self.writer)
    }
}

/// A sink that hashes what it is given and keeps none of it.
struct Hashing<'a> {
    crc: &'a mut Hasher,
    count: u64,
}

impl Write for Hashing<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.crc.update(buf);
        self.count += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A writer that counts what passes through it, so an entry's stored length needs no seek to learn.
struct Counted<'a, W: Write> {
    inner: &'a mut W,
    count: u64,
}

impl<W: Write> Write for Counted<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.count += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Copies a reader into a writer, hashing what passes.
///
/// One pass rather than `std::io::copy` and a second read: the CRC is over the entry's real bytes,
/// which are exactly what is being read here.
fn copy_hashing(
    source: &mut impl Read,
    sink: &mut impl Write,
    crc: &mut Hasher,
) -> Result<u64, ContainerError> {
    let mut buffer = vec![0_u8; COPY_BUFFER];
    let mut total = 0_u64;
    loop {
        let read = match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ContainerError::Io(error)),
        };
        crc.update(&buffer[..read]);
        sink.write_all(&buffer[..read])?;
        total += read as u64;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Writes a container into memory, so a test can assert on the bytes themselves.
    fn written(entries: &[(&str, Method, &[u8])]) -> Vec<u8> {
        let mut writer = Writer::new(Cursor::new(Vec::new())).expect("header");
        for (name, method, bytes) in entries {
            writer.write_entry(name, *method, bytes).expect("entry");
        }
        writer.finish().expect("finish").into_inner()
    }

    #[test]
    fn an_entry_written_either_way_reads_back_exactly() {
        // The test whose absence is why this format was designed twice before and read back neither
        // time: every method round-trips, and the CRC says so independently of the bytes.
        let stored = b"stored bytes, and a decoder seeks into these".to_vec();
        let deflated = "deflate me ".repeat(500).into_bytes();
        let bytes = written(&[
            ("media/1.mp3", Method::Stored, &stored),
            ("media/1.cdg", Method::Deflate, &deflated),
        ]);

        let mut container = Container::open(Cursor::new(bytes)).expect("open");
        for (name, expected) in [("media/1.mp3", &stored), ("media/1.cdg", &deflated)] {
            let entry = container.entry(name).expect("entry").clone();
            assert_eq!(entry.real_len, expected.len() as u64, "{name} real length");
            let read = container
                .read_entry(&entry, 1 << 20)
                .unwrap_or_else(|_| panic!("{name} reads"));
            assert_eq!(&read, expected, "{name} reads back what was written");
            assert_eq!(entry.crc, crc32fast::hash(expected), "{name} crc");
        }
    }

    #[test]
    fn a_deflated_entry_is_smaller_and_a_stored_one_is_a_byte_range() {
        let payload = "the same line over and over ".repeat(400).into_bytes();
        let bytes = written(&[
            ("media/1.mp4", Method::Stored, &payload),
            ("midi/1.mid", Method::Deflate, &payload),
        ]);
        let container = Container::open(Cursor::new(bytes.clone())).expect("open");

        let stored = container.entry("media/1.mp4").expect("stored");
        assert_eq!(
            stored.offset, HEADER_LEN,
            "the first entry follows the header"
        );
        assert_eq!(
            stored.stored_len, stored.real_len,
            "a stored entry has one length"
        );
        // The whole point of storing it: the bytes are simply there, at the offset the directory
        // names, with nothing to inflate first.
        let at = usize::try_from(stored.offset).expect("offset");
        let end = at + usize::try_from(stored.stored_len).expect("length");
        assert_eq!(
            &bytes[at..end],
            &payload[..],
            "a stored entry is a byte range"
        );

        let deflated = container.entry("midi/1.mid").expect("deflated");
        assert!(
            deflated.stored_len < deflated.real_len,
            "deflating this payload must save something: {} against {}",
            deflated.stored_len,
            deflated.real_len
        );
    }

    #[test]
    fn a_package_says_what_it_is_in_its_first_bytes() {
        let bytes = written(&[("manifest.json", Method::Deflate, b"{}")]);
        assert_eq!(&bytes[..8], &MAGIC, "a hex dump says what the file is");
        assert_eq!(
            &bytes[bytes.len() - 8..],
            &FOOTER_MAGIC,
            "and the tail says it is all there"
        );
    }

    #[test]
    fn something_that_was_never_a_package_is_refused_as_one() {
        let bytes = b"just a text file, sitting in the packages folder".to_vec();
        assert!(matches!(
            Container::open(Cursor::new(bytes)),
            Err(ContainerError::NotAPackage)
        ));
    }

    #[test]
    fn a_container_from_a_newer_build_names_itself_rather_than_being_read_wrong() {
        let mut bytes = written(&[("manifest.json", Method::Deflate, b"{}")]);
        bytes[8..10].copy_from_slice(&(CONTAINER_VERSION + 1).to_le_bytes());
        assert!(matches!(
            Container::open(Cursor::new(bytes)),
            Err(ContainerError::UnsupportedContainer(v)) if v == CONTAINER_VERSION + 1
        ));
    }

    #[test]
    fn a_truncated_package_is_refused_wherever_it_was_cut() {
        let payload = vec![7_u8; 4096];
        let whole = written(&[
            ("media/1.mp3", Method::Stored, &payload),
            ("manifest.json", Method::Deflate, b"{}"),
        ]);

        // Into the footer, into the directory, and into the entry data: a package cut anywhere at
        // all must fail at open, because a directory read into memory makes a truncated file open
        // perfectly and break only when somebody sings.
        for cut in [4, 30, 2048, whole.len() - HEADER_LEN as usize] {
            let short = whole[..whole.len() - cut].to_vec();
            assert!(
                Container::open(Cursor::new(short)).is_err(),
                "cutting {cut} bytes off the end must be refused"
            );
        }
    }

    #[test]
    fn an_entry_reaching_into_the_directory_is_refused() {
        let whole = written(&[("media/1.mp3", Method::Stored, &vec![3_u8; 1024])]);
        let mut bent = whole.clone();
        // The stored length in the one directory record, grown past where the entries end. Counting
        // back from where the directory ends, a record finishes with the stored length, the real
        // length and the crc.
        let directory_ends = whole.len() - FOOTER_LEN as usize;
        let at = directory_ends - (8 + 8 + 4);
        bent[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(
            matches!(
                Container::open(Cursor::new(bent)),
                Err(ContainerError::Malformed(_))
            ),
            "an entry that runs past the entries is refused rather than followed"
        );
    }

    #[test]
    fn an_entry_that_inflates_past_its_ceiling_is_refused_rather_than_held() {
        // Deflate reaches roughly a thousand to one, so the ceiling has to be on what arrives.
        let bomb = vec![0_u8; 1 << 20];
        let bytes = written(&[("midi/1.mid", Method::Deflate, &bomb)]);
        let mut container = Container::open(Cursor::new(bytes)).expect("open");
        let entry = container.entry("midi/1.mid").expect("entry").clone();
        assert!(
            matches!(
                container.read_entry(&entry, 4096),
                Err(CappedRead::TooLarge)
            ),
            "an entry past the ceiling stops rather than allocating"
        );
    }

    #[test]
    fn two_writes_of_the_same_entries_are_byte_identical() {
        // The reproducibility promise, at the level the container is responsible for: nothing here
        // records a moment, a host or a mode, so there is nothing to differ.
        let payload = "reproducible ".repeat(300).into_bytes();
        let once = written(&[("media/1.mp3", Method::Stored, &payload)]);
        let twice = written(&[("media/1.mp3", Method::Stored, &payload)]);
        assert_eq!(once, twice);
    }

    #[test]
    fn a_raw_copy_moves_the_bytes_and_the_crc_and_nothing_else() {
        let payload = "carried across ".repeat(200).into_bytes();
        let source_bytes = written(&[("media/1.cdg", Method::Deflate, &payload)]);
        let source = Container::open(Cursor::new(source_bytes)).expect("open");
        let entry = source.entry("media/1.cdg").expect("entry").clone();
        let mut reader = source.into_inner();

        let mut writer = Writer::new(Cursor::new(Vec::new())).expect("header");
        writer
            .raw_copy("media/1.cdg", &mut reader, &entry)
            .expect("copy");
        let rebuilt = writer.finish().expect("finish").into_inner();

        let mut container = Container::open(Cursor::new(rebuilt)).expect("open");
        let copied = container.entry("media/1.cdg").expect("entry").clone();
        assert_eq!(copied.method, entry.method, "the method is carried");
        assert_eq!(copied.crc, entry.crc, "the crc is carried, not recomputed");
        assert_eq!(copied.real_len, entry.real_len);
        assert_eq!(
            copied.stored_len, entry.stored_len,
            "nothing was re-encoded"
        );
        assert_eq!(
            container.read_entry(&copied, 1 << 20).ok().expect("reads"),
            payload,
            "and it still reads back"
        );
    }

    #[test]
    fn the_directory_reports_entries_in_the_order_they_were_written() {
        let bytes = written(&[
            ("media/2.mp4", Method::Stored, b"video"),
            ("midi/1.mid", Method::Deflate, b"midi"),
            ("manifest.json", Method::Deflate, b"{}"),
        ]);
        let container = Container::open(Cursor::new(bytes)).expect("open");
        let names: Vec<&str> = container.entries().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["media/2.mp4", "midi/1.mid", "manifest.json"]);
    }
}
