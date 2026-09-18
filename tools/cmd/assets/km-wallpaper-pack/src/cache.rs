//! The on-disk cache: search responses, original bytes, the index of what is known, and what each
//! image measured.
//!
//! The cache is what makes the tool usable at all. Stock APIs have quotas measured in hundreds of
//! requests per hour, and tuning a legibility threshold takes dozens of runs — so **every byte that
//! ever comes off the network is kept**, and `analyze` and `build` never ask for one. A second
//! `fetch` over an unchanged config does no network I/O whatsoever.
//!
//! **The network is not the only expensive thing, and for a long time it was the only one cached.**
//! Measuring a photograph costs a JPEG decode, a Lanczos3 resize to output size, a perceptual hash, a
//! Gaussian blur and a per-pixel vignette; over 4,682 images that was 78 minutes, repeated in full
//! every time anybody moved a threshold by 0.01. [`Measurement`] records are kept in `metrics.jsonl`
//! for the same reason the bytes are kept in `originals/`, and keyed by
//! [`crate::config::Config::measurement_hash`] so that only a setting which genuinely changes a
//! measurement invalidates one.
//!
//! Two failure modes are designed out rather than handled:
//!
//! * **A truncated image never enters the cache.** Downloads are streamed to `<name>.part` and
//!   renamed only when complete, so killing the process leaves either nothing or a whole file. A
//!   half-written JPEG would decode — badly, and differently on different decoders — which is the
//!   worst shape a cache entry can have.
//! * **The index is append-only.** It is a record of what was seen, not a database to be edited, so
//!   two runs cannot race into a lost update. Duplicates in it are expected and are collapsed on
//!   load, keyed by provider and id.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::ProviderKind;
use crate::error::{Error, Result};
use crate::providers::RemoteImage;

/// A cache rooted at one directory.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Opens (and creates) a cache at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for sub in ["responses", "originals"] {
            let dir = root.join(sub);
            std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
                path: dir.display().to_string(),
                source,
            })?;
        }
        Ok(Self { root })
    }

    /// The directory this cache lives in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where a search response is kept.
    ///
    /// Keyed by a hash of the whole request rather than by term and page, because a provider's
    /// parameters are part of what produced the answer: change `min_width` in the config and the
    /// cached response for "mountain lake dusk" page 1 stops answering the question being asked.
    pub fn response_path(&self, provider: ProviderKind, request: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(request.as_bytes());
        let digest = hex::encode(hasher.finalize());
        self.root
            .join("responses")
            .join(provider.as_str())
            .join(format!("{}.json", &digest[..32]))
    }

    /// A cached search response, if there is one.
    pub fn response(&self, provider: ProviderKind, request: &str) -> Option<String> {
        std::fs::read_to_string(self.response_path(provider, request)).ok()
    }

    /// Stores a search response.
    pub fn put_response(&self, provider: ProviderKind, request: &str, body: &str) -> Result<()> {
        let path = self.response_path(provider, request);
        write_atomically(&path, body.as_bytes())
    }

    /// Where an original image is kept.
    pub fn original_path(&self, provider: ProviderKind, id: &str, extension: &str) -> PathBuf {
        self.root
            .join("originals")
            .join(provider.as_str())
            .join(format!("{}.{extension}", sanitise(id)))
    }

    /// Any cached original for this image, whatever extension it was saved under.
    pub fn original(&self, provider: ProviderKind, id: &str) -> Option<PathBuf> {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            let path = self.original_path(provider, id, extension);
            if path.is_file() {
                return Some(path);
            }
        }
        None
    }

    /// Stores original bytes, atomically.
    pub fn put_original(
        &self,
        provider: ProviderKind,
        id: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        let path = self.original_path(provider, id, extension);
        write_atomically(&path, bytes)?;
        Ok(path)
    }

    /// Where a thumbnail of an original is kept.
    ///
    /// **A third kind of derived file beside `originals/` and `gray/`, and it exists for a reader
    /// rather than for the pipeline.** Nothing in `fetch`, `analyze` or `build` looks at one: they
    /// are for `km-admin`, whose review grid shows a hundred and twenty candidates at once. Serving
    /// the originals there would push several hundred megabytes through a webview to draw tiles a few
    /// hundred pixels wide.
    ///
    /// Under `--cache-dir` with everything else derived, so one setting still governs where a run's
    /// working files go and one `rm -rf` still removes them. Always JPEG regardless of what the
    /// original was: these are lossy previews of photographs, and a caller that wants the real thing
    /// asks for [`Cache::original`].
    pub fn thumb_path(&self, provider: ProviderKind, id: &str) -> PathBuf {
        self.root
            .join("thumbs")
            .join(provider.as_str())
            .join(format!("{}.jpg", sanitise(id)))
    }

    /// A cached thumbnail, if one has been made.
    pub fn thumb(&self, provider: ProviderKind, id: &str) -> Option<PathBuf> {
        let path = self.thumb_path(provider, id);
        path.is_file().then_some(path)
    }

    /// Stores a thumbnail, atomically.
    ///
    /// **Not keyed on the measurement hash, and that is deliberate.** A thumbnail is a picture of the
    /// original, so nothing in `[legibility]` or `[filters]` can change what it should look like —
    /// which is why adding this does not disturb [`crate::config::Config::measurement_hash`] or cost
    /// a re-measure. The original is immutable once cached, so a thumbnail of it never goes stale.
    pub fn put_thumb(&self, provider: ProviderKind, id: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.thumb_path(provider, id);
        write_atomically(&path, bytes)?;
        Ok(path)
    }

    /// Every memoised measurement taken under these settings, keyed by `provider:id`.
    ///
    /// `params` is [`crate::config::Config::measurement_hash`]. Records written under any other
    /// settings are skipped rather than returned, so one file can hold the answers for several
    /// configs and a run only ever sees its own.
    ///
    /// Keyed on identity alone, without re-reading the bytes to check the digest, because an original
    /// is immutable once it is in the cache: [`Cache::put_original`] renames atomically from `.part`,
    /// and `fetch` never re-downloads a file that is already present. The SHA-256 is *stored* in the
    /// record — dedupe and `analysis.json` both need it — but it is a value, not the key.
    pub fn measurements(&self, params: &str) -> Result<BTreeMap<String, Measurement>> {
        let mut by_key = BTreeMap::new();
        for record in self.read_jsonl::<Measurement>(&self.metrics_path())? {
            if record.params != params {
                continue;
            }
            by_key.insert(format!("{}:{}", record.provider, record.id), record);
        }
        Ok(by_key)
    }

    /// Appends measurements.
    pub fn extend_measurements(&self, records: &[Measurement]) -> Result<()> {
        self.append_jsonl(&self.metrics_path(), records)
    }

    /// The index of every image ever seen.
    ///
    /// Later records win, so a re-fetch that corrects a field takes effect without rewriting history.
    pub fn index(&self) -> Result<Vec<RemoteImage>> {
        let mut by_id: BTreeMap<(ProviderKind, String), RemoteImage> = BTreeMap::new();
        for image in self.read_jsonl::<RemoteImage>(&self.index_path())? {
            by_id.insert((image.provider, image.id.clone()), image);
        }
        Ok(by_id.into_values().collect())
    }

    /// Appends records to the index.
    pub fn extend_index(&self, images: &[RemoteImage]) -> Result<()> {
        self.append_jsonl(&self.index_path(), images)
    }

    /// One JSON object per line, so appending is a write rather than a read-modify-write.
    fn index_path(&self) -> PathBuf {
        self.root.join("index.jsonl")
    }

    /// The measurements, in the same shape and for the same reason as the index.
    fn metrics_path(&self) -> PathBuf {
        self.root.join("metrics.jsonl")
    }

    /// Reads a JSONL file, skipping what will not parse. A missing file is an empty one.
    fn read_jsonl<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<Vec<T>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::Io {
                    path: path.display().to_string(),
                    source,
                });
            }
        };

        let mut records = Vec::new();
        for (line, text) in text.lines().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<T>(text) {
                Ok(record) => records.push(record),
                // One unreadable line is not worth losing the rest of a file that took an hour of
                // quota — or an hour of CPU — to build. Said out loud, because silently dropping
                // records would show up much later as a pack that is mysteriously smaller than the
                // cache, or as a measurement phase that is mysteriously slow again.
                Err(error) => tracing::warn!(
                    path = %path.display(),
                    line = line + 1,
                    %error,
                    "skipping an unreadable cache entry"
                ),
            }
        }
        Ok(records)
    }

    /// Appends records to a JSONL file, creating it if it is not there.
    fn append_jsonl<T: serde::Serialize>(&self, path: &Path, records: &[T]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|source| Error::Io {
                path: path.display().to_string(),
                source,
            })?;
        for record in records {
            let line = serde_json::to_string(record).map_err(|error| Error::Io {
                path: path.display().to_string(),
                source: std::io::Error::other(error),
            })?;
            writeln!(file, "{line}").map_err(|source| Error::Io {
                path: path.display().to_string(),
                source,
            })?;
        }
        Ok(())
    }
}

/// One memoised measurement: everything `crate::commands::measure_original` produced for one image.
///
/// This is the expensive thing, and the reason it is kept rather than the grayscale downscale an
/// earlier design reached for. The downscale is one step of six — the decode, the Lanczos3 resize to
/// output size, the perceptual hash, the blur and the vignette all come first, and all of them would
/// still have to run. Memoising the answer skips the lot.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Measurement {
    /// Which provider the image came from.
    pub provider: ProviderKind,
    /// The provider's id for it.
    pub id: String,
    /// [`crate::config::Config::measurement_hash`] of the settings this was measured under.
    pub params: String,
    /// SHA-256 of the original's bytes, which the pack manifest and `verify` both quote.
    pub sha256: String,
    /// Perceptual hash of the cropped-and-resized picture, before the legibility treatments.
    pub phash: u64,
    /// Width of the decoded original, for [`crate::config::Filters::min_decoded_width`].
    pub decoded_width: u32,
    /// What was measured.
    pub metrics: crate::metrics::ImageMetrics,
}

/// Writes a file by writing `<path>.part` and renaming it.
///
/// The rename is what makes it atomic on every platform this runs on. Without it, a kill mid-write
/// leaves a file that exists, is the wrong length, and will be trusted by the next run.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let part = path.with_extension(format!(
        "{}.part",
        path.extension().and_then(|e| e.to_str()).unwrap_or("tmp")
    ));
    std::fs::write(&part, bytes).map_err(|source| Error::Io {
        path: part.display().to_string(),
        source,
    })?;
    std::fs::rename(&part, path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })
}

/// A provider id, made safe to use as a file name.
///
/// Pixabay ids are numeric and Pexels' are too, but a provider added later might not be, and a `/`
/// in a file name is a directory nobody asked for.
fn sanitise(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::RemoteImage;

    fn image(id: &str, query: &str) -> RemoteImage {
        RemoteImage {
            provider: ProviderKind::Pixabay,
            id: id.to_owned(),
            download_url: format!("https://example.test/{id}.jpg"),
            page_url: format!("https://pixabay.com/photos/{id}/"),
            author: "Somebody".to_owned(),
            author_url: None,
            width: 3000,
            height: 2000,
            tags: vec!["mountain".to_owned()],
            query: query.to_owned(),
            license: None,
            title: None,
            attribution: None,
        }
    }

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = Cache::open(dir.path()).expect("open");
        (dir, cache)
    }

    #[test]
    fn a_response_is_keyed_by_the_whole_request_not_just_the_term() {
        let (_dir, cache) = cache();
        let one = cache.response_path(ProviderKind::Pixabay, "term=lake&page=1&min_width=2400");
        let other = cache.response_path(ProviderKind::Pixabay, "term=lake&page=1&min_width=3000");
        assert_ne!(
            one, other,
            "a different min_width is a different question, so it cannot share an answer"
        );
    }

    #[test]
    fn a_cached_response_is_returned_and_a_missing_one_is_not() {
        let (_dir, cache) = cache();
        assert!(cache.response(ProviderKind::Pixabay, "nothing").is_none());
        cache
            .put_response(ProviderKind::Pixabay, "term=lake", "{\"hits\":[]}")
            .expect("store");
        assert_eq!(
            cache
                .response(ProviderKind::Pixabay, "term=lake")
                .as_deref(),
            Some("{\"hits\":[]}")
        );
    }

    #[test]
    fn a_thumbnail_is_stored_and_found_and_is_not_an_original() {
        let (_dir, cache) = cache();
        assert!(cache.thumb(ProviderKind::Openverse, "abc").is_none());

        let path = cache
            .put_thumb(ProviderKind::Openverse, "abc", b"not really a jpeg")
            .expect("store");
        assert_eq!(
            cache.thumb(ProviderKind::Openverse, "abc").as_deref(),
            Some(path.as_path())
        );

        // A thumbnail must never be mistaken for the real file: `original` looks under
        // `originals/` and must not find one of these, whatever extension they share.
        assert!(
            cache.original(ProviderKind::Openverse, "abc").is_none(),
            "a thumbnail is not an original"
        );
        assert_ne!(
            cache.thumb_path(ProviderKind::Openverse, "abc"),
            cache.original_path(ProviderKind::Openverse, "abc", "jpg")
        );
    }

    #[test]
    fn a_thumbnail_is_always_a_jpeg_whatever_the_original_was() {
        // These are lossy previews of photographs; a caller wanting the real bytes asks for
        // `original`. One extension means the path is a pure function of the id.
        let (_dir, cache) = cache();
        let path = cache.thumb_path(ProviderKind::Pixabay, "9876");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("jpg"));
    }

    #[test]
    fn an_original_is_found_under_whichever_extension_it_arrived_with() {
        let (_dir, cache) = cache();
        assert!(cache.original(ProviderKind::Pixabay, "12345").is_none());
        cache
            .put_original(ProviderKind::Pixabay, "12345", "png", b"not really a png")
            .expect("store");
        let found = cache
            .original(ProviderKind::Pixabay, "12345")
            .expect("found");
        assert_eq!(found.extension().and_then(|e| e.to_str()), Some("png"));
    }

    #[test]
    fn nothing_partial_is_left_behind_by_an_atomic_write() {
        let (dir, cache) = cache();
        cache
            .put_original(ProviderKind::Pixabay, "999", "jpg", b"whole")
            .expect("store");
        let leftovers: Vec<_> = walk(dir.path())
            .into_iter()
            .filter(|path| path.to_string_lossy().contains(".part"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn the_index_collapses_repeats_and_keeps_the_last_word() {
        let (_dir, cache) = cache();
        cache
            .extend_index(&[image("1", "mountain lake dusk"), image("2", "misty forest")])
            .expect("append");
        // The same photograph surfaced again by another term, which is what stock APIs do all day.
        cache
            .extend_index(&[image("1", "alpine valley clouds")])
            .expect("append");

        let index = cache.index().expect("read");
        assert_eq!(index.len(), 2, "two photographs, three sightings");
        let first = index.iter().find(|image| image.id == "1").expect("id 1");
        assert_eq!(
            first.query, "alpine valley clouds",
            "the later record wins, so a corrected field takes effect"
        );
    }

    #[test]
    fn one_corrupt_index_line_does_not_lose_the_others() {
        let (dir, cache) = cache();
        cache.extend_index(&[image("1", "lake")]).expect("append");
        let path = dir.path().join("index.jsonl");
        let mut text = std::fs::read_to_string(&path).expect("read");
        text.push_str("{ this is not json\n");
        std::fs::write(&path, text).expect("write");
        cache.extend_index(&[image("2", "forest")]).expect("append");

        let index = cache.index().expect("read");
        assert_eq!(index.len(), 2, "the readable records survive");
    }

    #[test]
    fn a_missing_index_is_an_empty_one_rather_than_an_error() {
        let (_dir, cache) = cache();
        assert!(cache.index().expect("read").is_empty());
    }

    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(root) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }
}
