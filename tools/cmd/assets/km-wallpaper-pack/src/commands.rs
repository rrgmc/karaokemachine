//! The three phases, plus `verify`.
//!
//! Each is a function of what came before it and nothing else: `fetch` writes the cache, `analyze`
//! reads the cache and writes `analysis.json`, `build` reads both and writes the pack. That is what
//! makes the expensive part happen once and the tuning happen often.
//!
//! "The expensive part" is two parts and both are cached: `fetch`'s bytes and `analyze`'s
//! measurements — see [`crate::cache::Measurement`]. Caching only the bytes means tuning a threshold
//! re-decodes the whole corpus. `--remeasure` is how to ask for that deliberately, when the *code*
//! changes rather than the config.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cache::{Cache, Measurement};
use crate::config::{Config, ProviderKind};
use crate::dedupe::{self, Candidate};
use crate::error::{Error, Result};
use crate::manifest::{Entry, Manifest};
use crate::metrics::{self, ImageMetrics};
use crate::process;
use crate::progress;
use crate::providers::openverse::Openverse;
use crate::providers::pexels::Pexels;
use crate::providers::pixabay::Pixabay;
use crate::providers::{Keys, Provider, RemoteImage};
use crate::select::{self, Rejection, Scored};

/// How many downloads run at once per provider.
///
/// Eight is polite rather than fast: the throttle already decides the rate, and this only stops a
/// slow CDN from serializing the whole run behind one 6 MB file.
const CONCURRENT_DOWNLOADS: usize = 8;

/// How often the measuring loop says how far it has got.
///
/// Two hundred rather than a progress bar: this is a phase that should now take seconds on a warm
/// cache and minutes on a cold one, and a dependency that draws over the terminal would be a poor
/// trade for that. Log lines also survive being piped to a file, which a bar does not.
const PROGRESS_EVERY: usize = 200;

/// What `analyze` writes and `build` reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    /// The config that produced it, so `build` can refuse a mismatched pair.
    pub config_hash: String,
    /// The chosen images, in pack order.
    pub chosen: Vec<Chosen>,
    /// Why everything else was left out.
    pub rejected: Vec<Rejected>,
}

/// One selected image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chosen {
    /// `provider:id`.
    pub key: String,
    /// The provider's record, carried through so `build` needs no second index read.
    pub image: RemoteImage,
    /// The darkening it needs.
    pub required_alpha: f32,
    /// The contrast it will be read at.
    pub measured_contrast: f32,
    /// Perceptual hash.
    pub phash: u64,
    /// Its score.
    pub score: f32,
    /// Copies it stood in for.
    pub deduped_into: Vec<String>,
}

/// One rejection, as written to `analysis.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rejected {
    /// `provider:id`.
    pub key: String,
    /// The reason, in the form the histogram counts.
    pub reason: String,
    /// What was measured, when anything was.
    ///
    /// A reason on its own says which threshold an image failed and not by how much, and a threshold
    /// cannot be tuned from that. `None` only for [`Rejection::Unmeasurable`], where there genuinely
    /// is no measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<MeasuredSummary>,
}

/// The measurements behind one rejection.
///
/// Deliberately not the whole of [`ImageMetrics`]: these are the five numbers that correspond to a
/// threshold somebody might move.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasuredSummary {
    /// Against `min_entropy`.
    pub entropy: f32,
    /// Against `max_band_busyness`.
    pub band_busyness: f32,
    /// Relative luminance of the brightest band cell — what the contrast solver was given.
    pub band_max_cell_luma: f32,
    /// The darkening that would reach the target with no cap, against `assumed_dim`.
    ///
    /// This is the number that turns `contrast_unreachable` from a verdict into a margin: an image
    /// wanting 0.47 of a 0.45 scrim is a threshold away from shipping and one wanting 0.90 is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_alpha_uncapped: Option<f32>,
    /// The contrast the image would actually be read at, at `assumed_dim`.
    pub contrast_at_assumed_dim: f32,
}

impl MeasuredSummary {
    /// Summarizes one measurement.
    fn of(metrics: &ImageMetrics) -> Self {
        Self {
            entropy: metrics.entropy,
            band_busyness: metrics.band_busyness,
            band_max_cell_luma: metrics.band_max_cell_luma,
            required_alpha_uncapped: metrics.required_alpha_uncapped,
            contrast_at_assumed_dim: metrics.contrast_at_assumed_dim,
        }
    }
}

/// `fetch`: search, then download whatever is not cached yet.
///
/// **`keys` is the caller's, not the environment's**, which is what lets a program with a form drive
/// this at all — see [`crate::providers::Keys`]. `main.rs` passes `Keys::from_env()`.
pub async fn fetch(
    config: &Config,
    cache: &Cache,
    keys: &Keys,
    refresh: bool,
    dry_run: bool,
    progress: progress::Sink<'_>,
) -> Result<usize> {
    let client = crate::providers::client()?;

    // Counted in query groups rather than in search requests: a group is `terms × pages` and the
    // throttle decides how long each takes, so the only number known before the phase starts is how
    // many groups the config asked for.
    let groups = config.queries.len();
    let mut found = Vec::new();
    for (done, group) in config.queries.iter().enumerate() {
        progress::tick(progress, progress::Phase::Searching, done, groups);
        match group.provider {
            ProviderKind::Pixabay => {
                let provider = Pixabay::new(client.clone(), keys, config.filters.min_source_width)?;
                found.extend(
                    search_group(&provider, group, cache, refresh, |term, page| {
                        provider.request(term, page)
                    })
                    .await?,
                );
            }
            ProviderKind::Pexels => {
                let provider = Pexels::new(client.clone(), keys)?;
                found.extend(
                    search_group(&provider, group, cache, refresh, |term, page| {
                        provider.request(term, page)
                    })
                    .await?,
                );
            }
            ProviderKind::Openverse => {
                // No `?`: this is the one provider that runs without a credential. It takes
                // `min_source_width` for the same reason Pixabay does, except that Openverse has no
                // width parameter, so the gate is applied to the results instead.
                let provider =
                    Openverse::new(client.clone(), keys, config.filters.min_source_width);
                found.extend(
                    search_group(&provider, group, cache, refresh, |term, page| {
                        provider.request(term, page)
                    })
                    .await?,
                );
            }
        }
    }

    progress::tick(progress, progress::Phase::Searching, groups, groups);
    tracing::info!(candidates = found.len(), "search complete");
    if dry_run {
        return Ok(found.len());
    }
    cache.extend_index(&found)?;

    // Only what is missing. A second run over an unchanged config downloads nothing at all, which is
    // the property that makes tuning affordable.
    let missing: Vec<RemoteImage> = found
        .into_iter()
        .filter(|image| cache.original(image.provider, &image.id).is_none())
        .collect();
    tracing::info!(to_download = missing.len(), "downloading originals");

    // **Here rather than at the top of the function, because this is the first moment the total is
    // known.** Every search has answered by now; before that a bar can only be indeterminate, which
    // is what a `total` of zero says.
    let mut attempted = 0_usize;
    let mut downloaded = 0_usize;
    progress::tick(progress, progress::Phase::Downloading, 0, missing.len());
    for chunk in missing.chunks(CONCURRENT_DOWNLOADS) {
        let mut tasks = Vec::new();
        for image in chunk {
            let client = client.clone();
            let cache = cache.clone();
            let image = image.clone();
            tasks.push(tokio::spawn(async move {
                download(&client, &cache, &image).await
            }));
        }
        for task in tasks {
            match task.await {
                Ok(Ok(())) => downloaded += 1,
                // One unavailable photograph is not worth ending a run that has spent an hour of
                // quota. Named, so a pattern of failures is visible.
                Ok(Err(error)) => tracing::warn!(%error, "skipping an image"),
                Err(error) => tracing::warn!(%error, "a download task died"),
            }
            // Counts what was *attempted*, not what succeeded: a skipped photograph is finished
            // work, and a bar that stalled every time a CDN 404'd would be lying about being stuck.
            attempted += 1;
            progress::tick(
                progress,
                progress::Phase::Downloading,
                attempted,
                missing.len(),
            );
        }
    }
    Ok(downloaded)
}

/// Searches every term and page of one config group, using the cache where it can.
async fn search_group<P: Provider>(
    provider: &P,
    group: &crate::config::QueryGroup,
    cache: &Cache,
    refresh: bool,
    request_for: impl Fn(&str, u32) -> String,
) -> Result<Vec<RemoteImage>> {
    let mut out = Vec::new();
    for term in &group.terms {
        for page in 1..=group.pages {
            let request = request_for(term, page);
            if !refresh && let Some(body) = cache.response(provider.kind(), &request) {
                let images = parse_for(provider.kind(), &body, term)?;
                tracing::debug!(term, page, hits = images.len(), "cached");
                out.extend(images);
                continue;
            }

            match provider.search(term, page).await {
                Ok(images) => {
                    tracing::info!(
                        provider = %provider.kind(),
                        term,
                        page,
                        hits = images.len(),
                        "searched"
                    );
                    // Cached as the normalized records rather than the provider's own body: it is
                    // what the next run needs, and it keeps a provider's response shape out of the
                    // cache format.
                    let body = serde_json::to_string(&images).map_err(|error| Error::Io {
                        path: "response cache".to_owned(),
                        source: std::io::Error::other(error),
                    })?;
                    cache.put_response(provider.kind(), &request, &body)?;
                    out.extend(images);
                }
                // A term the provider dislikes costs that term, not the run.
                Err(error) => tracing::warn!(term, page, %error, "search failed"),
            }
        }
    }
    Ok(out)
}

/// Cached responses are stored as normalized records, so this is the one shape to read back.
fn parse_for(provider: ProviderKind, body: &str, term: &str) -> Result<Vec<RemoteImage>> {
    let mut images: Vec<RemoteImage> = serde_json::from_str(body).map_err(|error| {
        Error::provider(
            provider.as_str(),
            format!("unreadable cached response: {error}"),
        )
    })?;
    // The term is part of the cache key, but a hand-edited cache should not be able to lie about
    // which query found a picture — selection's diversity cap depends on it.
    for image in &mut images {
        image.query = term.to_owned();
    }
    Ok(images)
}

/// Streams one original into the cache, atomically.
async fn download(client: &reqwest::Client, cache: &Cache, image: &RemoteImage) -> Result<()> {
    let response = client
        .get(&image.download_url)
        .send()
        .await
        .map_err(|error| Error::provider(image.provider.as_str(), error.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::provider(
            image.provider.as_str(),
            format!("{} for {}", response.status(), image.download_url),
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| Error::provider(image.provider.as_str(), error.to_string()))?;

    // Decoded before it is kept, so a cache never holds a file that only *looks* like a JPEG. A
    // truncated download from a flaky CDN is the common case this catches.
    let decoded = image::load_from_memory(&bytes).map_err(|error| Error::Image {
        path: PathBuf::from(&image.download_url),
        message: error.to_string(),
    })?;
    let (width, _) = image::GenericImageView::dimensions(&decoded);
    if width == 0 {
        return Err(Error::Image {
            path: PathBuf::from(&image.download_url),
            message: "zero-width image".to_owned(),
        });
    }
    cache.put_original(image.provider, &image.id, image.extension(), &bytes)?;
    Ok(())
}

/// `analyze`: measure, deduplicate, select. No network.
///
/// `remeasure` ignores the measurement cache and decodes everything again. The cache key covers the
/// settings a measurement depends on but cannot see a change to *this crate's own code*, so anybody
/// editing `metrics.rs` or `process.rs` needs a way to say so.
pub fn analyze(
    config: &Config,
    cache: &Cache,
    out: &Path,
    remeasure: bool,
    dry_run: bool,
    progress: progress::Sink<'_>,
) -> Result<Analysis> {
    let index = cache.index()?;

    // Only the measurements taken under settings that still apply; everything else is a miss.
    let params = config.measurement_hash();
    let memo = if remeasure {
        BTreeMap::new()
    } else {
        cache.measurements(&params)?
    };
    let hits = index
        .iter()
        .filter(|image| memo.contains_key(&format!("{}:{}", image.provider, image.id)))
        .count();
    let todo = index.len() - hits;
    tracing::info!(
        known = index.len(),
        cached = hits,
        measuring = todo,
        "measuring the cache"
    );

    // Embarrassingly parallel, and the slow part of the whole tool: a few thousand JPEG decodes.
    //
    // The `Err` arm carries why, rather than an empty option: an image that cannot be measured still
    // has to be accounted for, or `chosen + rejected` quietly stops equalling `known`.
    //
    // The counter exists because this phase used to print its first line before doing any work and
    // its next one after finishing all of it, which over 4,682 images meant well over an hour of
    // total silence. "Working" and "hung" have to be distinguishable from the terminal. It counts
    // only cache misses, so the number on screen is work actually being done.
    type Measured = std::result::Result<Measurement, &'static str>;
    let done = AtomicUsize::new(0);
    let started = std::time::Instant::now();
    let measured: Vec<(RemoteImage, Measured)> = index
        .par_iter()
        .map(|image| {
            let key = format!("{}:{}", image.provider, image.id);
            if let Some(hit) = memo.get(&key) {
                return (image.clone(), Ok(hit.clone()));
            }
            let Some(path) = cache.original(image.provider, &image.id) else {
                return (image.clone(), Err("no_cached_original"));
            };
            let measurement = match measure_original(&path, image, &params, config) {
                Ok(measurement) => Ok(measurement),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "could not measure");
                    Err("decode_failed")
                }
            };
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            // Every image, because a bar is drawn by a poller reading the latest value rather than
            // by every value being rendered — so throttling here would only make it jerkier. The log
            // line below keeps its two-hundred, which is a terminal's business.
            progress::tick(progress, progress::Phase::Measuring, n, todo);
            if n.is_multiple_of(PROGRESS_EVERY) || n == todo {
                tracing::info!(
                    measured = n,
                    of = todo,
                    elapsed_s = started.elapsed().as_secs(),
                    "measuring"
                );
            }
            (image.clone(), measurement)
        })
        .collect();

    // Written once, after the loop, rather than from inside it: the parallel section stays lock-free
    // and one append cannot interleave two runs' lines. Nothing under `--dry-run`, which promises to
    // write nothing anywhere.
    if !dry_run {
        let fresh: Vec<Measurement> = measured
            .iter()
            .filter(|(image, _)| !memo.contains_key(&format!("{}:{}", image.provider, image.id)))
            .filter_map(|(_, measurement)| measurement.as_ref().ok().cloned())
            .collect();
        cache.extend_measurements(&fresh)?;
    }

    let mut scored: Vec<Scored> = Vec::new();
    let mut rejected: Vec<(String, Rejection)> = Vec::new();
    let mut digests: BTreeMap<String, (String, u64)> = BTreeMap::new();
    let mut records: BTreeMap<String, RemoteImage> = BTreeMap::new();
    let mut summaries: BTreeMap<String, MeasuredSummary> = BTreeMap::new();

    for (image, measurement) in measured {
        let key = format!("{}:{}", image.provider, image.id);
        records.insert(key.clone(), image.clone());
        let measurement = match measurement {
            Ok(measurement) => measurement,
            Err(cause) => {
                rejected.push((key, Rejection::Unmeasurable(cause)));
                continue;
            }
        };
        let Measurement {
            sha256,
            phash,
            decoded_width,
            metrics,
            ..
        } = measurement;
        digests.insert(key.clone(), (sha256, phash));
        // Taken before `score` consumes the metrics, and kept for every image rather than only the
        // rejected ones: which images end up rejected is not known until deduplication and selection
        // have both run.
        summaries.insert(key.clone(), MeasuredSummary::of(&metrics));

        // **Before `select::score`, and deliberately not inside it.** `score` is about the picture —
        // its size, its entropy, the contrast where the words go — and takes `Dimensions` and
        // `ImageMetrics` because that is all it needs. This is about the paperwork, and threading a
        // license through the scorer to reject on it would blur a seam that is currently clean.
        //
        // Recorded as a rejection rather than skipped, so it lands in the histogram like every other
        // reason and `chosen + rejected == known` still holds.
        if config.filters.redistributable_only {
            let license = image.license();
            if !license.redistribution().is_granted() {
                rejected.push((key, select::Rejection::NotRedistributable(license.code)));
                continue;
            }
        }

        match select::score(
            key,
            image.query.clone(),
            select::Dimensions {
                source_width: image.width,
                decoded_width,
                megapixels: image.megapixels(),
            },
            metrics,
            &config.filters,
            &config.legibility,
        ) {
            Ok(image) => scored.push(image),
            Err(rejection) => rejected.push(rejection),
        }
    }

    // Deduplicate what survived the hard filters, keeping the best-scoring member of each cluster.
    let by_key: BTreeMap<&str, &Scored> = scored.iter().map(|s| (s.key.as_str(), s)).collect();
    let candidates: Vec<Candidate> = scored
        .iter()
        .filter_map(|image| {
            digests.get(&image.key).map(|(digest, phash)| Candidate {
                key: image.key.clone(),
                sha256: digest.clone(),
                phash: *phash,
            })
        })
        .collect();
    let clusters = dedupe::cluster(&candidates, config.filters.phash_distance, |key| {
        by_key.get(key).map_or(0.0, |image| image.score)
    });

    let mut kept: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for cluster in clusters {
        for dropped in &cluster.dropped {
            rejected.push((
                dropped.clone(),
                Rejection::DuplicateOf(cluster.keep.clone()),
            ));
        }
        kept.insert(cluster.keep, cluster.dropped);
    }
    let survivors: Vec<Scored> = scored
        .into_iter()
        .filter(|image| kept.contains_key(&image.key))
        .collect();

    let selection = select::select(survivors, &config.filters, config.output.target_count);
    rejected.extend(selection.rejected);

    let chosen = selection
        .chosen
        .iter()
        .filter_map(|image| {
            let record = records.get(&image.key)?;
            Some(Chosen {
                key: image.key.clone(),
                image: record.clone(),
                required_alpha: image.metrics.required_alpha.unwrap_or(0.0),
                measured_contrast: image.metrics.measured_contrast.unwrap_or(0.0),
                phash: digests.get(&image.key).map_or(0, |(_, phash)| *phash),
                score: image.score,
                deduped_into: kept
                    .get(&image.key)
                    .map(|dropped| {
                        dropped
                            .iter()
                            .map(|key| key.rsplit(':').next().unwrap_or(key).to_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect();

    let analysis = Analysis {
        config_hash: config.hash(),
        chosen,
        rejected: rejected
            .into_iter()
            .map(|(key, reason)| Rejected {
                reason: reason.label(),
                metrics: summaries.get(&key).cloned(),
                key,
            })
            .collect(),
    };

    let mut histogram: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in &analysis.rejected {
        let kind = Rejection::kind_of_label(&entry.reason);
        *histogram.entry(kind).or_insert(0) += 1;
    }
    tracing::info!(
        chosen = analysis.chosen.len(),
        rejected = analysis.rejected.len(),
        known = index.len(),
        "selection complete"
    );
    for (reason, count) in &histogram {
        tracing::info!(reason, count, "rejected");
    }

    // Every image is either in the pack or has a reason. If that ever stops being true, say so here
    // rather than leaving the difference to be noticed by subtracting two log lines a month later.
    let accounted = analysis.chosen.len() + analysis.rejected.len();
    if accounted != index.len() {
        tracing::warn!(
            known = index.len(),
            accounted,
            "images in neither the pack nor the rejections: this is a bug in analyze"
        );
    }
    report_contrast_margins(&analysis, config.legibility.assumed_dim);

    if !dry_run {
        std::fs::create_dir_all(out).map_err(|source| Error::Io {
            path: out.display().to_string(),
            source,
        })?;
        let path = out.join("analysis.json");
        let mut text = serde_json::to_string_pretty(&analysis).map_err(|error| Error::Io {
            path: path.display().to_string(),
            source: std::io::Error::other(error),
        })?;
        text.push('\n');
        crate::cache::write_atomically(&path, text.as_bytes())?;
    }
    Ok(analysis)
}

/// Says what a larger scrim would buy, for the images the current one turned away.
///
/// `contrast_unreachable` is usually the biggest bucket in the histogram and, on its own, the least
/// actionable: it does not distinguish a photograph that missed by a hundredth of an alpha from a
/// white sky no scrim could rescue. These lines answer the question the count provokes — "would
/// loosening it help, and by how much?" — from measurements already taken, without a second
/// forty-minute run.
fn report_contrast_margins(analysis: &Analysis, assumed_dim: f32) {
    let wanted: Vec<f32> = analysis
        .rejected
        .iter()
        .filter(|entry| entry.reason == "contrast_unreachable")
        .filter_map(|entry| entry.metrics.as_ref()?.required_alpha_uncapped)
        .collect();
    if wanted.is_empty() {
        return;
    }

    // Steps past the current scrim rather than absolute figures, so the report follows `assumed_dim`
    // when it moves.
    for step in [0.05_f32, 0.10, 0.15, 0.20] {
        let scrim = assumed_dim + step;
        if scrim > 1.0 {
            break;
        }
        let gained = wanted.iter().filter(|alpha| **alpha <= scrim).count();
        tracing::info!(
            scrim = format!("{scrim:.2}"),
            gained,
            "images a deeper scrim would admit"
        );
    }
    let hopeless = wanted.len();
    tracing::info!(
        turned_away = hopeless,
        at_scrim = format!("{assumed_dim:.2}"),
        "of these, none can be read at the scrim the app applies"
    );
}

/// Measures one cached original, hashes it, and digests its bytes on the way past.
fn measure_original(
    path: &Path,
    image: &RemoteImage,
    params: &str,
    config: &Config,
) -> Result<Measurement> {
    use sha2::{Digest, Sha256};

    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    let digest = hex::encode(Sha256::digest(&bytes));
    let decoded = image::load_from_memory(&bytes).map_err(|error| Error::Image {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    // The width of the file, which is not the width the provider advertised for the photograph. See
    // `Filters::min_decoded_width`.
    let (decoded_width, _) = image::GenericImageView::dimensions(&decoded);

    // Measured on the picture as it will be *shown*: the same crop, resize and blur `build` applies,
    // at the first output size. Measuring the original instead would judge detail that the downscale
    // is about to remove and a blur is about to remove again.
    //
    // The hash is taken one step earlier, on the cropped-and-resized picture before the blur and the
    // vignette. Those are legibility treatments applied identically to every candidate, and hashing
    // after them makes every candidate resemble every other. Splitting `render` this way costs
    // nothing: `treat` picks up exactly where `canonical` left off.
    let size = config.output.sizes[0];
    let canonical = process::canonical(&decoded, size);
    let phash = metrics::perceptual_hash(&canonical);
    let rendered = process::treat(canonical, &config.legibility);
    Ok(Measurement {
        provider: image.provider,
        id: image.id.clone(),
        params: params.to_owned(),
        sha256: digest,
        phash,
        decoded_width,
        metrics: metrics::measure(&rendered, &config.legibility),
    })
}

/// `build`: process the selection and write the pack.
pub fn build(
    config: &Config,
    cache: &Cache,
    analysis: &Analysis,
    out: &Path,
    force: bool,
    dry_run: bool,
    generated_at: String,
) -> Result<Manifest> {
    if analysis.config_hash != config.hash() {
        return Err(Error::PackMismatch(
            "analysis.json was produced by a different config; re-run analyze".to_owned(),
        ));
    }
    // A used output directory is refused, and `--force` *replaces* the pack that is in it rather than
    // writing a new one on top. Output names carry the selection index and the perceptual hash, so a
    // rebuild never overwrites its predecessor: without the sweep the directory accumulates images no
    // manifest names, and they would ship — the zip is the deliverable and `verify` reads the
    // manifest, not the directory. `manifest::pack_artifacts` is the one definition of what belongs
    // to the pack, so the guard and the sweep cannot disagree; `analysis.json` is not in it, being
    // this command's own input.
    if !dry_run {
        let existing = crate::manifest::pack_artifacts(out)?;
        if !existing.is_empty() {
            if !force {
                return Err(Error::PackMismatch(format!(
                    "{} already holds a pack; pass --force to replace it",
                    out.display()
                )));
            }
            let removed = crate::manifest::clear_pack(out)?;
            tracing::info!(
                removed = removed.len(),
                out = %out.display(),
                "replaced the pack that was there"
            );
        }
    }

    let mut entries = Vec::new();
    // Per output size: how many chosen images had to be upscaled to reach it, and the narrowest one.
    let mut upscaled: BTreeMap<crate::config::Size, (usize, u32)> = BTreeMap::new();
    for (index, chosen) in analysis.chosen.iter().enumerate() {
        let Some(path) = cache.original(chosen.image.provider, &chosen.image.id) else {
            tracing::warn!(key = chosen.key, "no cached original; skipping");
            continue;
        };
        let bytes = std::fs::read(&path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        let decoded = image::load_from_memory(&bytes).map_err(|error| Error::Image {
            path: path.clone(),
            message: error.to_string(),
        })?;
        let (decoded_width, _) = image::GenericImageView::dimensions(&decoded);

        for size in &config.output.sizes {
            // Counted here, reported once at the end. An output size larger than the file it comes
            // from is a Lanczos3 upscale that invents every extra pixel, and it went unnoticed for a
            // month: the pack shipped 4K images built from 1280-pixel downloads, because the only
            // size gate in the tool read the provider's metadata rather than the file.
            //
            // A line per image would be the obvious way to say so and the wrong one. Providers cap
            // what they serve, so this is all-or-nothing in practice — 120 identical warnings that
            // scroll past and teach nobody anything. One line carrying the count and the narrowest
            // source is a fact somebody will read.
            if decoded_width < size.width {
                let worst = upscaled.entry(*size).or_insert((0usize, u32::MAX));
                worst.0 += 1;
                worst.1 = worst.1.min(decoded_width);
            }
            let rendered = process::render(&decoded, *size, &config.legibility);
            let measured = metrics::measure(&rendered, &config.legibility);
            // The gate again, on the bytes that are about to be written. `analyze` measured the first
            // size; a 4K crop of the same photograph is a different picture and has to earn its place
            // on its own.
            let Some(contrast) = measured.measured_contrast else {
                tracing::warn!(
                    key = chosen.key,
                    %size,
                    "dropped at build: contrast unreachable at this size"
                );
                continue;
            };

            let name = process::output_name(index + 1, chosen.phash, *size, config.output.format);
            let file = format!("{size}/{name}");
            if !dry_run {
                let encoded =
                    process::encode(&rendered, config.output.format, config.output.jpeg_quality)?;
                crate::cache::write_atomically(&out.join(&file), &encoded)?;
            }

            let license = chosen.image.license();
            entries.push(Entry {
                file,
                provider: Some(chosen.image.provider),
                source_id: chosen.image.id.clone(),
                source_url: chosen.image.page_url.clone(),
                author: chosen.image.author.clone(),
                author_url: chosen.image.author_url.clone(),
                // **The image's own license, not its provider's.** A `chosen.image.provider.license()`
                // here is the shape that makes the whole question invisible: it can describe a
                // site with one license and nothing else, so nobody ever has to ask what a pack
                // may be used for. `RemoteImage::license()` answers with the provider's terms
                // where that is the truth -- both stock providers -- and with the image's own
                // where a source aggregates many.
                license: license.name.clone(),
                license_url: license.url.clone(),
                license_code: Some(license.code.clone()),
                query: chosen.image.query.clone(),
                required_alpha: round4(chosen.required_alpha),
                measured_contrast: round4(contrast),
                phash: format!("{:016x}", chosen.phash),
                deduped_into: chosen.deduped_into.clone(),
            });
        }
    }

    for (size, (count, narrowest)) in &upscaled {
        tracing::warn!(
            %size,
            images = count,
            narrowest_source = narrowest,
            "upscaled: the sources are narrower than this output size, so the extra pixels are \
             invented. Ask for a size the providers actually serve."
        );
    }

    let manifest = Manifest::new(config, generated_at, entries);
    warn_if_not_redistributable(&manifest);
    if !dry_run {
        crate::cache::write_atomically(
            &out.join(crate::manifest::MANIFEST_FILE),
            manifest.to_json()?.as_bytes(),
        )?;
        crate::cache::write_atomically(
            &out.join(crate::manifest::ATTRIBUTION_FILE),
            manifest.attribution().as_bytes(),
        )?;
        if config.output.zip {
            let name = crate::manifest::zip_name(&config.hash());
            let zip = out.join(&name);
            let bytes = crate::manifest::zip_pack(out, &zip, &manifest)?;
            tracing::info!(zip = %zip.display(), bytes, "packed");
        }
    }
    Ok(manifest)
}

/// Copy a built pack's zip out of `out` and into `zip_dest`, returning where it went.
///
/// A copy rather than a move: `out` stays the complete record of a build — loose images, manifest,
/// credits and zip — which is what [`verify`] reads and what lets a `--force` rebuild's sweep tell
/// the pack apart from everything else in the directory.
///
/// `Ok(None)` means there was nothing to do because the two directories are the same, which is how
/// `--zip-dest ./out` asks for no copy at all. That case is checked rather than allowed to happen:
/// on Windows copying a file onto itself truncates it rather than failing, so the pack's own zip
/// would be destroyed by the step meant to distribute it. The comparison canonicalises the
/// *directories*, because the destination file does not exist yet and so cannot be canonicalised.
pub fn copy_zip(out: &Path, zip_dest: &Path, name: &str) -> Result<Option<PathBuf>> {
    let same = match (out.canonicalize(), zip_dest.canonicalize()) {
        (Ok(out), Ok(dest)) => out == dest,
        // An absent destination is not the same directory as anything, and is about to be created.
        _ => false,
    };
    if same {
        return Ok(None);
    }

    std::fs::create_dir_all(zip_dest).map_err(|source| Error::Io {
        path: zip_dest.display().to_string(),
        source,
    })?;
    let to = zip_dest.join(name);
    std::fs::copy(out.join(name), &to).map_err(|source| Error::Io {
        path: to.display().to_string(),
        source,
    })?;
    Ok(Some(to))
}

/// Builds a pack from a folder of images somebody chose, and a sidecar saying where each came from.
///
/// The curated counterpart of [`build`]. See [`crate::curated`] for why the shipped set is made this
/// way and the packs are not — and note the one thing the two must not differ in: **the gate is the
/// same gate**, run against the same [`crate::config::Legibility`], because a shipped wallpaper that
/// cannot be read fails on every machine rather than on one packager's.
///
/// Settings come from [`Config::default`] and there is no config file, which is not a shortcut:
/// [`crate::config::Legibility`]'s defaults are read off the app's own `Theme` and
/// `WallpaperConfig`, so they are already exactly what a set destined for `assets/` has to be
/// measured against. `config.example.toml` restates them and the two agree.
///
/// **A rejected image stops the run rather than being dropped.** [`build`] skips one and carries on,
/// which is right when it is choosing a hundred out of thousands and nobody named any of them. Here
/// somebody chose each one by hand: silently shipping five of the six they picked would be a worse
/// answer than saying which one failed and why.
pub fn local(
    dir: &Path,
    credits_path: Option<&Path>,
    out: &Path,
    force: bool,
    dry_run: bool,
    generated_at: String,
) -> Result<Manifest> {
    let config = Config::default();

    // Sorted, so the pack's order and its file numbering are a property of the folder rather than of
    // the order the filesystem happened to hand them back in. A shipped set that renumbers itself
    // between two runs on two machines is a diff nobody can read.
    let mut sources: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.display().to_string(),
        source,
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_source_image(&path) {
            sources.push(path);
        }
    }
    sources.sort();
    if sources.is_empty() {
        return Err(Error::config(format!(
            "{} holds no images this tool can read",
            dir.display()
        )));
    }

    let names: std::collections::BTreeSet<String> = sources
        .iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect();
    let credits_path = credits_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dir.join(crate::curated::CREDITS_FILE));
    let credits = crate::curated::Credits::read(&credits_path, &names)?;

    if !dry_run {
        let existing = crate::manifest::pack_artifacts(out)?;
        if !existing.is_empty() {
            if !force {
                return Err(Error::PackMismatch(format!(
                    "{} already holds a pack; pass --force to replace it",
                    out.display()
                )));
            }
            crate::manifest::clear_pack(out)?;
        }
    }

    let mut pack_entries = Vec::new();
    let mut rejected = 0usize;
    for (index, path) in sources.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a name, since that is how the set was built");
        let credit = credits
            .for_file(name)
            .expect("described, since Credits::read proved every file is");

        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        // A file that will not decode is a rejection like any other while surveying, and fatal in a
        // real build. The case is not hypothetical and not caught by the extension check: Wikimedia
        // serves TIFFs under a `.jpg` name, and one of them stopped a survey of a hundred candidates
        // after five.
        let decoded = match image::load_from_memory(&bytes) {
            Ok(decoded) => decoded,
            Err(error) if dry_run => {
                tracing::warn!(file = name, "rejected: will not decode: {error}");
                rejected += 1;
                continue;
            }
            Err(error) => {
                return Err(Error::Image {
                    path: path.clone(),
                    message: error.to_string(),
                });
            }
        };
        let (decoded_width, _) = image::GenericImageView::dimensions(&decoded);
        let phash = metrics::perceptual_hash(&decoded);

        for size in &config.output.sizes {
            // The same warning `build` gives, and it matters more here: a shipped wallpaper built by
            // inventing two thirds of its pixels is soft on the one screen everybody sees.
            if decoded_width < size.width {
                tracing::warn!(
                    file = name,
                    %size,
                    decoded_width,
                    "upscaled: this source is narrower than the output size, so the extra pixels \
                     are invented. Find a larger original."
                );
            }
            let rendered = process::render(&decoded, *size, &config.legibility);
            let measured = metrics::measure(&rendered, &config.legibility);
            // **`--dry-run` surveys; a real run refuses.** The two are different jobs and the flag
            // is where the difference belongs. Choosing which eight of thirty candidates to ship
            // needs every verdict at once — stopping at the first failure would mean thirty runs —
            // whereas a build of the set somebody settled on must not quietly drop one of them.
            let verdict = match measured.measured_contrast {
                None => Err(format!(
                    "no darkening within {:.2} reaches {:.2}:1",
                    config.legibility.assumed_dim, config.legibility.target_contrast
                )),
                Some(contrast) if contrast < config.legibility.target_contrast => Err(format!(
                    "{contrast:.2}:1, below the {:.2}:1 a shipped wallpaper has to reach",
                    config.legibility.target_contrast
                )),
                Some(contrast) => Ok(contrast),
            };
            let contrast = match verdict {
                Ok(contrast) => {
                    if dry_run {
                        tracing::info!(file = name, %size, contrast, "passes");
                    }
                    contrast
                }
                Err(why) if dry_run => {
                    tracing::warn!(file = name, %size, "rejected: {why}");
                    rejected += 1;
                    continue;
                }
                Err(why) => {
                    return Err(Error::config(format!(
                        "{name} at {size}: {why}. Pick a calmer photograph."
                    )));
                }
            };

            let file = format!(
                "{size}/{}",
                process::output_name(index + 1, phash, *size, config.output.format)
            );
            if !dry_run {
                let encoded =
                    process::encode(&rendered, config.output.format, config.output.jpeg_quality)?;
                crate::cache::write_atomically(&out.join(&file), &encoded)?;
            }

            pack_entries.push(Entry {
                file,
                // No provider: nobody's API was asked. The license is the credit's own, which is the
                // whole reason `Entry` stopped deriving one from a provider.
                provider: None,
                source_id: name.to_owned(),
                source_url: credit.source_url.clone(),
                author: credit.author.clone(),
                author_url: credit.author_url.clone(),
                license: credit.license.clone(),
                license_url: credit.license_url.clone(),
                // The sidecar is typed by a person and carries a license *name*, so the code is
                // either stated outright or looked up by exact match. `code_for_name` falls to
                // `unstated`, which is not redistributable — the right way to be wrong, since the
                // alternative reading of a name it does not know is to guess in the permissive
                // direction.
                license_code: Some(credit.code()),
                query: String::new(),
                required_alpha: round4(measured.required_alpha.unwrap_or(0.0)),
                measured_contrast: round4(contrast),
                phash: format!("{phash:016x}"),
                deduped_into: Vec::new(),
            });
        }
    }

    if dry_run && rejected > 0 {
        tracing::warn!(
            rejected,
            kept = pack_entries.len(),
            "a real run would stop at the first of these"
        );
    }

    let manifest = Manifest::new(&config, generated_at, pack_entries);
    warn_if_not_redistributable(&manifest);
    if !dry_run {
        crate::cache::write_atomically(
            &out.join(crate::manifest::MANIFEST_FILE),
            manifest.to_json()?.as_bytes(),
        )?;
        crate::cache::write_atomically(
            &out.join(crate::manifest::ATTRIBUTION_FILE),
            manifest.attribution().as_bytes(),
        )?;
        if config.output.zip {
            let name = crate::manifest::zip_name(&config.hash());
            let zip = out.join(&name);
            let bytes = crate::manifest::zip_pack(out, &zip, &manifest)?;
            tracing::info!(zip = %zip.display(), bytes, "packed");
        }
    }
    Ok(manifest)
}

/// Says once, at the end of a build, when the pack that was just written may not be handed on.
///
/// **One line, not one per image**, and the precedent is the upscale warning a few lines above: a
/// provider's terms apply to everything it served, so this is all-or-nothing in practice, and 120
/// identical warnings scrolling past teach nobody anything.
///
/// It names which source the images came from and what that source's terms cover, which is why
/// [`crate::license::Redistribution`] carries a sentence: "this pack is not redistributable" on its
/// own does not tell somebody where the answer came from.
fn warn_if_not_redistributable(manifest: &Manifest) {
    if manifest.redistributable {
        return;
    }
    let mut by_source: BTreeMap<&str, usize> = BTreeMap::new();
    let mut clause = None;
    for entry in &manifest.images {
        let license = crate::license::License {
            code: entry.license_code.clone().unwrap_or_default(),
            name: entry.license.clone(),
            url: entry.license_url.clone(),
        };
        if let crate::license::Redistribution::Personal { source, terms } = license.redistribution()
        {
            *by_source.entry(source).or_default() += 1;
            clause.get_or_insert(terms);
        }
    }
    let sources: Vec<String> = by_source
        .iter()
        .map(|(source, count)| format!("{source} ({count})"))
        .collect();
    tracing::warn!(
        images = manifest.images.len(),
        sources = sources.join(", "),
        "this pack is for the machine that built it and may not be passed on: {}",
        clause.unwrap_or("its images' licenses do not grant redistribution.")
    );
}

/// Whether a file is one this tool would try to decode as a source image.
///
/// By extension, and deliberately narrower than what `image` can open: these are the formats a
/// photograph actually arrives in. It exists mainly to let the sidecar's "every image is described"
/// check ignore the sidecar itself, a README, or a `.gitkeep` sitting in the same folder.
fn is_source_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let extension = extension.to_ascii_lowercase();
            matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "webp")
        })
        .unwrap_or(false)
}

/// `verify`: re-check a built pack against its own gate.
///
/// Re-decodes every file and re-measures it at the *manifest's* settings rather than the config's, so
/// a pack built last month is checked against the rules it was built under. The point is to catch a
/// hand-added image or an edited threshold, and both of those show up as a file that no longer clears
/// the number written beside it.
pub fn verify(pack: &Path) -> Result<usize> {
    let manifest = Manifest::read(pack)?;
    let legibility = crate::config::Legibility {
        band_top: manifest.legibility.band[0],
        band_bottom: manifest.legibility.band[1],
        text_color: crate::config::Rgb::from_hex(&manifest.legibility.text_color)?,
        assumed_dim: manifest.legibility.assumed_dim,
        target_contrast: manifest.legibility.target_contrast,
        // The files are already blurred and vignetted; doing either again would measure a picture
        // that is not in the pack.
        blur_sigma: 0.0,
        vignette: false,
    };

    let outcomes: Vec<std::result::Result<(), String>> = manifest
        .images
        .par_iter()
        .map(|entry| {
            let path = pack.join(&entry.file);
            let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", entry.file))?;
            let decoded = image::load_from_memory(&bytes)
                .map_err(|error| format!("{}: {error}", entry.file))?;
            let measured = metrics::measure(&decoded, &legibility);
            match measured.measured_contrast {
                Some(contrast) if contrast >= manifest.legibility.target_contrast => Ok(()),
                Some(contrast) => Err(format!(
                    "{}: {contrast:.2}:1, below the {:.2}:1 this pack promises",
                    entry.file, manifest.legibility.target_contrast
                )),
                None => Err(format!(
                    "{}: no darkening within {:.2} reaches {:.2}:1",
                    entry.file,
                    manifest.legibility.assumed_dim,
                    manifest.legibility.target_contrast
                )),
            }
        })
        .collect();

    let mut failures = 0;
    let mut missing = Vec::new();
    for outcome in outcomes {
        if let Err(problem) = outcome {
            if problem.contains("No such file")
                || problem.contains("cannot find")
                || problem.contains("os error 2")
            {
                missing.push(problem);
            } else {
                tracing::error!("{problem}");
                failures += 1;
            }
        }
    }
    if !missing.is_empty() {
        return Err(Error::PackMismatch(missing.join("; ")));
    }
    if failures > 0 {
        return Err(Error::ContrastFailed { count: failures });
    }
    Ok(manifest.images.len())
}

/// Four decimal places, so a manifest is stable and readable rather than carrying float noise.
fn round4(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_keeps_a_manifest_readable_and_comparable() {
        assert_eq!(round4(0.123_456_7), 0.1235);
        assert_eq!(round4(7.0), 7.0);
    }

    #[test]
    fn the_zip_is_copied_into_a_destination_that_need_not_exist_yet() {
        // The normal case, and more so since the default became `local/assets/wallpapers`: that
        // folder is gitignored, so a fresh checkout of a machine that has never built a pack has no
        // reason to have one at all, and the copy has to make it rather than fail.
        let root = tempfile::tempdir().expect("tempdir");
        let out = root.path().join("out");
        std::fs::create_dir_all(&out).expect("out");
        std::fs::write(out.join("wallpapers-abcdef12.zip"), b"pack").expect("zip");

        let dest = root.path().join("local/assets/wallpapers");
        let to = copy_zip(&out, &dest, "wallpapers-abcdef12.zip")
            .expect("the copy must succeed")
            .expect("a different directory means a copy happened");

        assert_eq!(to, dest.join("wallpapers-abcdef12.zip"));
        assert_eq!(std::fs::read(&to).expect("read back"), b"pack");
        // A copy, not a move: `verify --pack ./out` still has something to read.
        assert!(out.join("wallpapers-abcdef12.zip").exists());
    }

    #[test]
    fn copying_onto_itself_is_refused_rather_than_destroying_the_pack() {
        // `--zip-dest ./out` is how somebody asks for no copy at all. On Windows `fs::copy` onto the
        // same path truncates the file rather than erroring, so without the guard the one command
        // meant to distribute the pack would empty it instead.
        let root = tempfile::tempdir().expect("tempdir");
        let out = root.path().join("out");
        std::fs::create_dir_all(&out).expect("out");
        std::fs::write(out.join("wallpapers-abcdef12.zip"), b"pack").expect("zip");

        let copied = copy_zip(&out, &out, "wallpapers-abcdef12.zip").expect("must not fail");

        assert!(copied.is_none(), "the same directory means nothing to do");
        assert_eq!(
            std::fs::read(out.join("wallpapers-abcdef12.zip")).expect("read back"),
            b"pack",
            "the pack must survive untouched"
        );
    }

    #[test]
    fn a_missing_zip_names_the_file_it_could_not_write() {
        let root = tempfile::tempdir().expect("tempdir");
        let out = root.path().join("out");
        std::fs::create_dir_all(&out).expect("out");

        let error = copy_zip(&out, &root.path().join("dest"), "wallpapers-abcdef12.zip")
            .expect_err("there is no zip to copy");

        assert!(
            error.to_string().contains("wallpapers-abcdef12.zip"),
            "{error}"
        );
    }

    #[test]
    fn a_cached_response_cannot_lie_about_which_term_found_a_picture() {
        let body = serde_json::to_string(&vec![RemoteImage {
            provider: ProviderKind::Pixabay,
            id: "1".to_owned(),
            download_url: "https://x.test/a.jpg".to_owned(),
            page_url: String::new(),
            author: "A".to_owned(),
            author_url: None,
            width: 4000,
            height: 3000,
            tags: Vec::new(),
            query: "whatever was in the file".to_owned(),
            license: None,
            title: None,
            attribution: None,
        }])
        .expect("json");

        let images = parse_for(ProviderKind::Pixabay, &body, "mountain lake dusk").expect("parse");
        assert_eq!(
            images[0].query, "mountain lake dusk",
            "the diversity cap depends on this, so the caller's term wins"
        );
    }

    #[test]
    fn build_refuses_an_analysis_from_a_different_config() {
        let config = Config::parse("[[queries]]\nprovider=\"pixabay\"\nterms=[\"a\"]\npages=1\n")
            .expect("config");
        let analysis = Analysis {
            config_hash: "not the same".to_owned(),
            chosen: Vec::new(),
            rejected: Vec::new(),
        };
        let dir = tempfile::tempdir().expect("temp");
        let cache = Cache::open(dir.path().join("cache")).expect("cache");
        let error = build(
            &config,
            &cache,
            &analysis,
            &dir.path().join("out"),
            false,
            true,
            "now".to_owned(),
        )
        .expect_err("must refuse");
        assert_eq!(error.exit_code(), 3, "{error}");
        assert!(error.to_string().contains("re-run analyze"), "{error}");
    }
}
