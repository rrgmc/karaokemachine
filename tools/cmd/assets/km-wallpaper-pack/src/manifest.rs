//! What a built pack says about itself.
//!
//! Two files travel with the images, and they answer different questions:
//!
//! * `manifest.json` — provenance and measurements. Which license, which id, which search term, what
//!   alpha the image needed and what contrast it will actually be read at. This is what makes a pack
//!   **auditable**: every picture traces back to a license and to the number that let it in. The
//!   license is recorded **per image**, because the alternative — deriving it from the provider when
//!   the file is read — cannot describe a source that aggregates many, and is why nobody looked one
//!   up for a month.
//! * `ATTRIBUTION.md` — the human list, grouped by license, one line per photographer, opening with
//!   the statement that every image was modified. Generated for every pack whether or not a license
//!   demands it, because Pexels' guidelines ask for visible credit, because CC BY requires both the
//!   credit and the modification notice, and because a credits screen is a thing this app should
//!   eventually have.
//!
//! Neither is read at run time. The app finds wallpapers by scanning its folder — and reads a zip in
//! that folder as a folder of wallpapers, which is why the pack ships as one file to drop in.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, ProviderKind, Size};
use crate::error::{Error, Result};
use crate::license::License;

/// The manifest's name inside a pack.
pub const MANIFEST_FILE: &str = "manifest.json";

/// The credits file's name inside a pack.
pub const ATTRIBUTION_FILE: &str = "ATTRIBUTION.md";

/// What every pack zip is called before its config hash.
pub const ZIP_PREFIX: &str = "wallpapers-";

/// What a built pack's zip is called.
///
/// **The config hash and nothing else, because the hash is what a pack *is*.** Two packs built from
/// two configs cannot collide, which is what the name has to guarantee — the zip is copied into
/// `dist/` where packs from different runs sit beside each other and no directory is a record of one
/// build.
///
/// **The picture count stays out of it.** A count is a fact about one build rather than about the
/// pack, and the same search against a grown cache finds a different number — so a count in the name
/// gives that rebuild a file of its own, which sits in the wallpapers folder beside the first and
/// shows every photograph in both twice, `Playlist` keying a picture on its archive's name. A
/// rebuild of one search has to land on the file it replaces, which is what `km-admin`'s `keep`
/// assumes. The count is in `manifest.json`, which is where the list rows read it.
///
/// One definition, called by `commands::build` when it writes the zip and by `main` when it copies
/// it: two places deriving the same name independently is exactly how a copy step ends up looking
/// for a file that was written under another name.
pub fn zip_name(config_hash: &str) -> String {
    format!("{ZIP_PREFIX}{}.zip", &config_hash[..8])
}

/// The build timestamp, RFC 3339 to the second — what [`Manifest::generated_at`] is set from.
///
/// Hand-formatted from the epoch rather than pulling in a date library for one string: the manifest
/// wants a stamp a person can read, and this crate has no other use for calendars.
///
/// **Here rather than in `main.rs`, where it was**, because `build` takes this string as an argument
/// and every caller therefore needs it: `km-admin` drives the same phase from a page, and a second
/// copy of a calendar algorithm to fill one field would be an absurd thing to keep in step.
pub fn generated_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let days = seconds / 86_400;
    let time = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

/// Howard's `civil_from_days`, the standard branch-free calendar conversion.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The manifest of a built pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// When it was built, RFC 3339. The one field a determinism check has to ignore.
    pub generated_at: String,
    /// The version of the tool that built it.
    pub tool_version: String,
    /// `sha256:…` over the settings, so a pack can be traced to the config that produced it.
    pub config_hash: String,
    /// The gate every image in here passed.
    pub legibility: LegibilityRecord,
    /// Whether this pack may be handed on: true only when **every** image's license grants it.
    ///
    /// **Computed from the entries, never carried on a `RemoteImage`.** `index.jsonl` is a text file
    /// anybody can edit, and a boolean stored there would let a hand-edit promote a Pixabay image
    /// into a redistributable pack. Deriving it here means the only way to change the answer is to
    /// change what the images are.
    ///
    /// A pack whose entries predate `license_code` reads as `false`, which is the honest answer
    /// rather than a wrong one: nothing in that file says the licenses may be passed on.
    #[serde(default)]
    pub redistributable: bool,
    /// The images, in pack order.
    pub images: Vec<Entry>,
}

/// The legibility settings a pack was built under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegibilityRecord {
    /// The contrast ratio every image reaches.
    pub target_contrast: f32,
    /// The band it was measured in, as fractions of image height.
    pub band: [f32; 2],
    /// The darkening the app was assumed to apply.
    pub assumed_dim: f32,
    /// The text color contrast was measured against.
    pub text_color: String,
}

/// One image in a pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Path inside the pack, e.g. `1920x1080/scenery-001-….jpg`.
    pub file: String,
    /// Which provider it came from, when a provider is what it came from.
    ///
    /// `None` for an image curated by hand rather than fetched — see `commands::local`. It is
    /// metadata now rather than the thing a license is derived from: the license is per image and
    /// lives in [`Entry::license`], because a source that aggregates many licenses cannot be asked
    /// for one. `#[serde(default)]` so every manifest written before this still reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    /// The provider's id, or a stable identifier for a curated image.
    pub source_id: String,
    /// The page a person can visit.
    pub source_url: String,
    /// Who took it.
    pub author: String,
    /// Their page, when there is one.
    pub author_url: Option<String>,
    /// The license it ships under, named as a person would say it: `CC0 1.0`, `CC BY 4.0`,
    /// `Pixabay Content License`.
    pub license: String,
    /// Where that license's text is, when it has a URL.
    ///
    /// Required in substance by CC BY, which wants the license identified and linked, and simply
    /// useful for everything else. `None` for a provider's own terms, which `ATTRIBUTION.md` then
    /// names without linking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    /// The machine-readable license code, which is what [`Manifest::redistributable`] is decided
    /// from — `cc0`, `by`, `pixabay`, …
    ///
    /// A name alone cannot be reasoned about: `CC BY 4.0` and `CC BY-NC-ND 4.0` differ by two
    /// letters and by everything that matters. `#[serde(default)]` because manifests written before
    /// this exist and must keep reading — the shipped `assets/wallpapers/default-wallpapers.zip`
    /// carries one, and it cannot be rebuilt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_code: Option<String>,
    /// The search term that surfaced it. Empty for a curated image, which no search found.
    #[serde(default)]
    pub query: String,
    /// The darkening this image needed to reach the target.
    ///
    /// Kept even though nothing bakes it in: it is the number a per-image scrim or a future "dim
    /// background" setting would want, and re-deriving it means re-downloading the original.
    pub required_alpha: f32,
    /// The contrast it is actually read at, under the app's own scrim.
    pub measured_contrast: f32,
    /// Perceptual hash, hex.
    pub phash: String,
    /// Ids of the copies this one stood in for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deduped_into: Vec<String>,
}

impl Manifest {
    /// A manifest for a set of entries.
    pub fn new(config: &Config, generated_at: String, images: Vec<Entry>) -> Self {
        Self {
            generated_at,
            tool_version: crate::VERSION.to_owned(),
            config_hash: format!("sha256:{}", config.hash()),
            legibility: LegibilityRecord {
                target_contrast: config.legibility.target_contrast,
                band: [config.legibility.band_top, config.legibility.band_bottom],
                assumed_dim: config.legibility.assumed_dim,
                text_color: config.legibility.text_color.to_hex(),
            },
            redistributable: Self::all_redistributable(&images),
            images,
        }
    }

    /// Whether every image in a set may be handed on.
    ///
    /// **An empty pack is not redistributable**, deliberately, rather than by the vacuous truth
    /// `all()` gives: "nothing here may be shared" is a safer thing for an empty manifest to say
    /// than
    /// "everything here may be", and a pack with no images is a build that went wrong anyway.
    fn all_redistributable(images: &[Entry]) -> bool {
        !images.is_empty()
            && images.iter().all(|entry| {
                entry.license_code.as_deref().is_some_and(|code| {
                    License {
                        code: code.to_owned(),
                        name: entry.license.clone(),
                        url: entry.license_url.clone(),
                    }
                    .redistribution()
                    .is_granted()
                })
            })
    }

    /// Reads a manifest from a built pack.
    pub fn read(pack: &Path) -> Result<Self> {
        let path = pack.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_str(&text)
            .map_err(|error| Error::PackMismatch(format!("{}: {error}", path.display())))
    }

    /// The pretty-printed JSON, newline-terminated.
    pub fn to_json(&self) -> Result<String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|error| Error::Io {
            path: MANIFEST_FILE.to_owned(),
            source: std::io::Error::other(error),
        })?;
        text.push('\n');
        Ok(text)
    }

    /// The attribution file, grouped by **license**.
    ///
    /// It used to group by provider and put `provider.license()` in the heading, which worked only
    /// while a provider had exactly one license. It cannot survive a source that aggregates many —
    /// and, more to the point, the heading was asserting something no longer derived from the
    /// provider at all. Grouping by the thing the reader actually needs is also the better file: what
    /// somebody reusing an image wants to know first is what they are allowed to do.
    ///
    /// Sorted by license, then author, then id, so the file is stable between runs — a credits list
    /// that reshuffles itself is a diff nobody can read.
    ///
    /// **The modification notice is stated once, at the top, and it is an obligation rather than a
    /// courtesy**: CC BY requires that changes be indicated. Every image in every pack is cropped,
    /// resized, blurred, vignetted and re-encoded without exception — `process::render` has no path
    /// that skips it — so the tool is in the rare position of being able to say "every" honestly, and
    /// a fact repeated on all 120 lines would be noise rather than emphasis.
    pub fn attribution(&self) -> String {
        let mut by_license: BTreeMap<(&str, Option<&str>), Vec<&Entry>> = BTreeMap::new();
        for entry in &self.images {
            by_license
                .entry((entry.license.as_str(), entry.license_url.as_deref()))
                .or_default()
                .push(entry);
        }

        let mut out = String::from(
            "# Image credits\n\n\
             Every wallpaper in this pack is listed here with its photographer, its source page and \
             its license.\n\n\
             **Every image has been modified from the original.** Each was cropped to the display's \
             aspect ratio, resized, blurred, vignetted and re-encoded, and its embedded metadata was \
             removed. No image in this pack is a copy of the photograph as it was published.\n",
        );
        for ((license, url), mut entries) in by_license {
            entries.sort_by(|a, b| {
                a.author
                    .to_lowercase()
                    .cmp(&b.author.to_lowercase())
                    .then_with(|| a.source_id.cmp(&b.source_id))
            });
            match url {
                Some(url) => out.push_str(&format!(
                    "\n## {license}\n\n{} image(s), under [{license}]({url}).\n\n",
                    entries.len()
                )),
                None => out.push_str(&format!(
                    "\n## {license}\n\n{} image(s), under the {license}.\n\n",
                    entries.len()
                )),
            }
            for entry in entries {
                let author = match &entry.author_url {
                    Some(url) => format!("[{}]({})", entry.author, url),
                    None => entry.author.clone(),
                };
                out.push_str(&format!(
                    "- `{}` — {} — [source]({})\n",
                    entry.file, author, entry.source_url
                ));
            }
        }
        out
    }
}

/// Zips a built pack: the images the manifest names, the manifest and the attribution.
///
/// The zip is the deliverable, because the app reads a zip in its wallpaper folder as a folder of
/// wallpapers — one file to drop in, one file to remove, and nothing half-copied in between.
///
/// **The manifest decides what goes in, not the directory.** Walking `out` ships whatever happens
/// to be lying in it: `analysis.json`, a `.part` file from a killed run, the previous zip, and every
/// image of every earlier build — output names carry the selection index and the perceptual hash, so
/// a rebuild adds files beside its predecessor rather than replacing it. Nothing downstream would
/// catch that either, since `verify` reads the manifest and never the directory. Naming the files
/// also means the archive cannot contain itself, so no special case is needed for the zip.
///
/// The names are sorted and deduplicated, so two runs over the same pack produce the same bytes.
pub fn zip_pack(pack: &Path, out: &Path, manifest: &Manifest) -> Result<u64> {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    names.insert(MANIFEST_FILE);
    names.insert(ATTRIBUTION_FILE);
    for entry in &manifest.images {
        names.insert(entry.file.as_str());
    }

    let file = std::fs::File::create(out).map_err(|source| Error::Io {
        path: out.display().to_string(),
        source,
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut written = 0_u64;
    for name in names {
        let path = inside(pack, name);
        let bytes = std::fs::read(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::PackMismatch(format!("{name} is in the manifest and not in the pack"))
            } else {
                Error::Io {
                    path: path.display().to_string(),
                    source,
                }
            }
        })?;
        zip.start_file(name, options)
            .map_err(|error| Error::PackMismatch(error.to_string()))?;
        std::io::Write::write_all(&mut zip, &bytes).map_err(|source| Error::Io {
            path: out.display().to_string(),
            source,
        })?;
        written += bytes.len() as u64;
    }
    zip.finish()
        .map_err(|error| Error::PackMismatch(error.to_string()))?;
    Ok(written)
}

/// What a built pack consists of, as paths under `pack`, sorted.
///
/// One definition, read by both halves of `build`: the guard that refuses a used output directory and
/// the sweep `--force` runs before writing. Two definitions would drift, and the drift would surface
/// as a stale image in a deliverable rather than as an error.
///
/// A directory *named like a size* counts, rather than the sizes the config currently asks for, so a
/// pack built under a size list that has since changed is still recognized as the pack it is.
/// Everything else in the directory belongs to somebody — `analysis.json` above all, which is
/// `build`'s own input — and is never named here.
pub fn pack_artifacts(pack: &Path) -> Result<Vec<PathBuf>> {
    if !pack.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(pack).map_err(|source| Error::Io {
        path: pack.display().to_string(),
        source,
    })?;

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let belongs = if path.is_dir() {
            name.parse::<Size>().is_ok()
        } else {
            name == MANIFEST_FILE
                || name == ATTRIBUTION_FILE
                || (name.starts_with(ZIP_PREFIX) && name.ends_with(".zip"))
        };
        if belongs {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// Removes the pack in `pack` and returns what it removed.
///
/// The caller logs that list: a `--force` that silently deletes a hundred megabytes is worse than one
/// that says what it threw away.
pub fn clear_pack(pack: &Path) -> Result<Vec<PathBuf>> {
    let artifacts = pack_artifacts(pack)?;
    for path in &artifacts {
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        removed.map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(artifacts)
}

/// A `/`-separated name inside a pack, joined on one component at a time.
///
/// A manifest always writes forward slashes, whichever platform reads it. Handing the whole string to
/// `Path::join` happens to work on both — Windows accepts `/` as a separator too — but only by that
/// accident; splitting says what is meant, and gives real path components to compare and to create.
fn inside(pack: &Path, name: &str) -> PathBuf {
    name.split('/')
        .fold(pack.to_path_buf(), |path, part| path.join(part))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pack's name is its config and nothing about one build of it.
    ///
    /// **The property, not the string.** The same search against a grown cache finds a different
    /// number of pictures, and a name that moved with it would give that rebuild a file of its own —
    /// sitting in the wallpapers folder beside the first and showing every photograph in both twice,
    /// `Playlist` keying a picture on its archive's name. A rebuild lands on the file it replaces.
    #[test]
    fn a_pack_is_named_for_its_config_and_not_for_one_build_of_it() {
        let hash = "e36b9929c0ffee00";
        assert_eq!(zip_name(hash), "wallpapers-e36b9929.zip");
        // Two searches are two packs and must not collide.
        assert_ne!(zip_name(hash), zip_name("aaaaaaaabbbbbbbb"));
        // Eight characters of it, so the name stays short enough to read in a folder.
        assert!(zip_name(hash).starts_with(ZIP_PREFIX));
    }

    #[test]
    fn the_epoch_and_a_known_date_both_come_out_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-08-24 is 20689 days after the epoch.
        assert_eq!(civil_from_days(20_689), (2026, 8, 24));
    }

    #[test]
    fn the_timestamp_is_rfc_3339_shaped() {
        let stamp = generated_now();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert_eq!(stamp.as_bytes()[10], b'T', "{stamp}");
        assert!(stamp.starts_with("20"), "{stamp}");
    }

    fn entry(file: &str, provider: ProviderKind, author: &str, id: &str) -> Entry {
        Entry {
            file: file.to_owned(),
            provider: Some(provider),
            source_id: id.to_owned(),
            source_url: format!("https://example.test/{id}"),
            author: author.to_owned(),
            author_url: Some(format!("https://example.test/@{author}")),
            license: provider
                .whole_site_license()
                .expect("a stock provider has one license")
                .name,
            license_url: None,
            license_code: Some(provider.as_str().to_owned()),
            query: "mountain lake dusk".to_owned(),
            required_alpha: 0.12,
            measured_contrast: 8.4,
            phash: "a3f9c2d1".to_owned(),
            deduped_into: Vec::new(),
        }
    }

    fn manifest() -> Manifest {
        let config =
            Config::parse("[[queries]]\nprovider = \"pixabay\"\nterms = [\"lake\"]\npages = 1\n")
                .expect("config");
        Manifest::new(
            &config,
            "2026-08-24T00:00:00Z".to_owned(),
            vec![
                entry("1920x1080/a.jpg", ProviderKind::Pexels, "Zoe", "9"),
                entry("1920x1080/b.jpg", ProviderKind::Pixabay, "adam", "1"),
                entry("1920x1080/c.jpg", ProviderKind::Pixabay, "Bella", "2"),
            ],
        )
    }

    #[test]
    fn the_manifest_records_the_gate_the_pack_was_built_under() {
        let manifest = manifest();
        assert_eq!(manifest.legibility.target_contrast, 7.0);
        assert_eq!(manifest.legibility.assumed_dim, 0.45, "the app's own scrim");
        assert_eq!(manifest.legibility.band, [0.40, 0.68]);
        assert_eq!(manifest.legibility.text_color, "#ECEFF4");
        assert!(manifest.config_hash.starts_with("sha256:"));
        assert_eq!(manifest.tool_version, crate::VERSION);
    }

    #[test]
    fn a_manifest_round_trips_through_its_own_json() {
        let manifest = manifest();
        let json = manifest.to_json().expect("json");
        let read: Manifest = serde_json::from_str(&json).expect("parse");
        assert_eq!(read.images, manifest.images);
        assert_eq!(read.legibility, manifest.legibility);
        assert!(json.ends_with('\n'), "a text file ends with a newline");
    }

    /// A pack of stock-library images says so, whatever it cost to build.
    #[test]
    fn a_pack_from_a_provider_whose_terms_forbid_it_is_not_redistributable() {
        assert!(!manifest().redistributable);
    }

    #[test]
    fn a_pack_whose_every_image_grants_redistribution_says_so() {
        let mut entry = entry("1920x1080/a.jpg", ProviderKind::Pixabay, "Ada", "9");
        entry.provider = None;
        entry.license = "CC0 1.0".to_owned();
        entry.license_code = Some("cc0".to_owned());
        let config = Config::parse("[[queries]]\nprovider=\"pixabay\"\nterms=[\"x\"]\npages=1\n")
            .expect("config");
        assert!(Manifest::new(&config, "now".to_owned(), vec![entry]).redistributable);
    }

    /// One image is enough to make a pack unshippable, which is the point of `all`.
    #[test]
    fn one_image_that_may_not_be_passed_on_settles_the_whole_pack() {
        let mut good = entry("1920x1080/a.jpg", ProviderKind::Pixabay, "Ada", "9");
        good.license_code = Some("cc0".to_owned());
        let bad = entry("1920x1080/b.jpg", ProviderKind::Pixabay, "Bob", "10");
        let config = Config::parse("[[queries]]\nprovider=\"pixabay\"\nterms=[\"x\"]\npages=1\n")
            .expect("config");
        assert!(!Manifest::new(&config, "now".to_owned(), vec![good, bad]).redistributable);
    }

    /// **The compatibility test that guards a file which cannot be rebuilt.**
    ///
    /// `assets/wallpapers/default-wallpapers.zip` ships seven CC0 photographs and carries a manifest
    /// written before `license_code` and `redistributable` existed. `verify` deserializes the whole
    /// struct, so a required field on either would have stopped the shipped wallpapers verifying —
    /// and the originals they were built from are gone, so it could not simply be rebuilt.
    #[test]
    fn a_manifest_written_before_these_fields_existed_still_reads() {
        // `r##`, not `r#`: the text color below is `"#ECEFF4"`, and `"#` would close an `r#` string
        // in the middle of the fixture.
        let json = r##"{
            "generated_at": "2026-08-31T00:00:00Z",
            "tool_version": "1.3.0",
            "config_hash": "sha256:abc",
            "legibility": {
                "target_contrast": 7.0, "band": [0.4, 0.68],
                "assumed_dim": 0.45, "text_color": "#ECEFF4"
            },
            "images": [{
                "file": "1920x1080/scenery-001.jpg",
                "source_id": "01-milky-way.jpg",
                "source_url": "https://commons.wikimedia.org/w/index.php?curid=61854117",
                "author": "Andrew Coelho", "author_url": null,
                "license": "CC0 1.0",
                "license_url": "https://creativecommons.org/publicdomain/zero/1.0/",
                "query": "", "required_alpha": 0.0, "measured_contrast": 12.1,
                "phash": "17193f9f"
            }]
        }"##;

        let manifest: Manifest = serde_json::from_str(json).expect("an old manifest still reads");
        assert_eq!(manifest.images.len(), 1);
        assert_eq!(manifest.images[0].license_code, None);
        assert!(
            !manifest.redistributable,
            "a file that never claimed it may be passed on must not start claiming it"
        );
    }

    /// It used to group by provider and name the license in the heading, which held only while a
    /// provider had exactly one. What a reader needs first is what they may *do* with an image, so
    /// the license is the grouping now and the provider is metadata.
    #[test]
    fn attribution_groups_by_license() {
        let text = manifest().attribution();
        assert!(text.contains("## Pixabay Content License"), "{text}");
        assert!(text.contains("## Pexels License"), "{text}");
        // Every photographer, linked, with a link to the source page beside it.
        assert!(text.contains("[Zoe](https://example.test/@Zoe)"), "{text}");
        assert!(text.contains("[source](https://example.test/1)"), "{text}");
    }

    /// CC BY wants the license identified *and* linked, so a license carrying a URL is rendered as
    /// one. A provider's own terms have no per-image URL and are named without a link rather than
    /// linked to something invented.
    #[test]
    fn a_license_with_a_url_is_linked_and_one_without_is_only_named() {
        let mut cc = entry("1920x1080/a.jpg", ProviderKind::Pixabay, "Ada", "9");
        cc.provider = None;
        cc.license = "CC0 1.0".to_owned();
        cc.license_url = Some("https://creativecommons.org/publicdomain/zero/1.0/".to_owned());
        let config = Config::parse("[[queries]]\nprovider=\"pixabay\"\nterms=[\"x\"]\npages=1\n")
            .expect("config");
        let text = Manifest::new(&config, "now".to_owned(), vec![cc]).attribution();
        assert!(
            text.contains("[CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/)"),
            "{text}"
        );

        let text = manifest().attribution();
        assert!(
            text.contains("under the Pixabay Content License."),
            "{text}"
        );
    }

    /// CC BY requires that modification be indicated, and `process::render` modifies every image
    /// without exception — so the notice is stated once rather than repeated per line. If a future
    /// change ever lets an image through unprocessed, this is the assertion that has to be revisited
    /// rather than quietly left saying something untrue.
    #[test]
    fn attribution_says_every_image_was_modified() {
        let text = manifest().attribution();
        assert!(
            text.contains("**Every image has been modified from the original.**"),
            "{text}"
        );
        assert!(text.contains("cropped"), "{text}");
    }

    #[test]
    fn attribution_is_sorted_case_insensitively_so_it_does_not_reshuffle() {
        let text = manifest().attribution();
        let adam = text.find("adam").expect("adam");
        let bella = text.find("Bella").expect("Bella");
        assert!(
            adam < bella,
            "lower-case names sort with the rest, not before it"
        );
    }

    #[test]
    fn an_author_with_no_page_is_still_credited() {
        let mut entry = entry("1920x1080/a.jpg", ProviderKind::Pixabay, "Anonymous", "3");
        entry.author_url = None;
        let config = Config::parse("[[queries]]\nprovider=\"pixabay\"\nterms=[\"x\"]\npages=1\n")
            .expect("config");
        let text = Manifest::new(&config, "now".to_owned(), vec![entry]).attribution();
        assert!(text.contains("— Anonymous —"), "{text}");
    }

    /// A pack on disk holding `files`, plus the manifest and attribution every pack has.
    fn built_pack(pack: &Path, files: &[&str]) {
        std::fs::create_dir_all(pack).expect("mkdir");
        for name in files {
            let path = inside(pack, name);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            std::fs::write(&path, name.as_bytes()).expect("write");
        }
        std::fs::write(pack.join(MANIFEST_FILE), b"{}").expect("write");
        std::fs::write(pack.join(ATTRIBUTION_FILE), b"# credits").expect("write");
    }

    /// A manifest naming `files`, for the zip to work from.
    fn manifest_naming(files: &[&str]) -> Manifest {
        let mut manifest = manifest();
        manifest.images = files
            .iter()
            .map(|file| entry(file, ProviderKind::Pixabay, "Ansel", "1"))
            .collect();
        manifest
    }

    #[test]
    fn a_zip_holds_what_the_manifest_names_and_nothing_else_in_the_directory() {
        let dir = tempfile::tempdir().expect("temp");
        let pack = dir.path().join("out");
        built_pack(&pack, &["1920x1080/a.jpg"]);
        // The three ways a directory collects files no manifest names: an earlier build's image
        // (names carry the index and the hash, so nothing is overwritten), an earlier zip, and the
        // analysis this build was made from.
        std::fs::write(pack.join("1920x1080/scenery-099-deadbeef.jpg"), b"stale").expect("write");
        std::fs::write(pack.join("wallpapers-abcdef12.zip"), b"an older pack").expect("write");
        std::fs::write(pack.join("analysis.json"), b"[]").expect("write");

        let zip_path = pack.join("wallpapers-00000000.zip");
        let written =
            zip_pack(&pack, &zip_path, &manifest_naming(&["1920x1080/a.jpg"])).expect("zip");
        assert!(written > 0);

        let file = std::fs::File::open(&zip_path).expect("open");
        let archive = zip::ZipArchive::new(file).expect("read");
        let mut names: Vec<_> = archive.file_names().collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["1920x1080/a.jpg", "ATTRIBUTION.md", "manifest.json"],
            "forward slashes, so the app reads the same names on every platform"
        );
    }

    #[test]
    fn a_manifest_naming_a_file_the_pack_does_not_hold_is_a_mismatch() {
        let dir = tempfile::tempdir().expect("temp");
        let pack = dir.path().join("out");
        built_pack(&pack, &["1920x1080/a.jpg"]);

        let error = zip_pack(
            &pack,
            &dir.path().join("pack.zip"),
            &manifest_naming(&["1920x1080/a.jpg", "1920x1080/gone.jpg"]),
        )
        .expect_err("a promise about a file that is not there");
        assert_eq!(error.exit_code(), 3, "{error}");
        assert!(error.to_string().contains("gone.jpg"), "{error}");
    }

    #[test]
    fn zipping_twice_produces_the_same_archive() {
        let dir = tempfile::tempdir().expect("temp");
        let pack = dir.path().join("out");
        built_pack(&pack, &["a/one.jpg", "a/two.jpg"]);
        let manifest = manifest_naming(&["a/two.jpg", "a/one.jpg"]);
        let first = dir.path().join("one.zip");
        let second = dir.path().join("two.zip");
        zip_pack(&pack, &first, &manifest).expect("zip");
        zip_pack(&pack, &second, &manifest).expect("zip");
        assert_eq!(
            std::fs::read(&first).expect("read"),
            std::fs::read(&second).expect("read"),
            "the names are sorted, so the archive is byte-stable"
        );
    }

    #[test]
    fn what_belongs_to_a_pack_is_named_once_and_nothing_else_is() {
        let dir = tempfile::tempdir().expect("temp");
        let pack = dir.path().join("out");
        built_pack(&pack, &["1920x1080/a.jpg", "3840x2160/b.jpg"]);
        std::fs::write(pack.join("wallpapers-abcdef12.zip"), b"zip").expect("write");
        // Neither of these is the pack's: one is what `build` reads to make it, the other is a
        // person's.
        std::fs::write(pack.join("analysis.json"), b"[]").expect("write");
        std::fs::write(pack.join("notes.txt"), b"my notes").expect("write");
        std::fs::create_dir_all(pack.join("keep")).expect("mkdir");

        let named: Vec<String> = pack_artifacts(&pack)
            .expect("artifacts")
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            named,
            vec![
                "1920x1080",
                "3840x2160",
                "ATTRIBUTION.md",
                "manifest.json",
                "wallpapers-abcdef12.zip",
            ],
            "a directory named like a size belongs to the pack; analysis.json never does"
        );
    }

    #[test]
    fn clearing_a_pack_leaves_everything_that_is_not_the_pack() {
        let dir = tempfile::tempdir().expect("temp");
        let pack = dir.path().join("out");
        built_pack(&pack, &["1920x1080/a.jpg"]);
        std::fs::write(pack.join("wallpapers-abcdef12.zip"), b"zip").expect("write");
        std::fs::write(pack.join("analysis.json"), b"[]").expect("write");

        let removed = clear_pack(&pack).expect("clear");
        assert_eq!(removed.len(), 4, "{removed:?}");
        assert!(
            !pack.join("1920x1080").exists(),
            "images and their directory"
        );
        assert!(!pack.join(MANIFEST_FILE).exists());
        assert!(!pack.join(ATTRIBUTION_FILE).exists());
        assert!(!pack.join("wallpapers-abcdef12.zip").exists());
        assert!(
            pack.join("analysis.json").is_file(),
            "the analysis is this build's input, not the last build's output"
        );

        assert!(
            clear_pack(&pack).expect("clear again").is_empty(),
            "clearing an already-clear directory is not an error"
        );
        assert!(
            clear_pack(&dir.path().join("never-built"))
                .expect("clear a missing directory")
                .is_empty()
        );
    }
}
