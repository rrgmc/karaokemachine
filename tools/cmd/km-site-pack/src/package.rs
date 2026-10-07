//! Turns the folder into packages, through `km-pack`'s own library.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use km_pack::volumes::split_into_volumes;
use km_pack::{BuildEvent, BuildOptions, DescribeOptions, Rejection, Skipped, Spec};

use crate::Event;

/// What the packages are called and which songs go in.
#[derive(Debug, Clone)]
pub struct PackageOptions {
    /// The package's name. The folder's own name when absent.
    pub name: Option<String>,
    /// The package's version.
    pub version: String,
    /// Who publishes the package, when somebody does.
    pub publisher: Option<String>,
    /// The language of a song whose language the file does not give, as an ISO 639 code.
    pub default_language: String,
    /// The lowest suitability a song may have and still go in.
    pub min_suitability: Option<u8>,
    /// Whether a file with no words goes in.
    pub keep_wordless: bool,
    /// Where the packages and their descriptions are written. The folder's parent when absent, so
    /// that no package lands among the songs it was built from.
    pub out_dir: Option<PathBuf>,
}

impl Default for PackageOptions {
    fn default() -> Self {
        Self {
            name: None,
            version: "1.0.0".to_owned(),
            publisher: None,
            default_language: "und".to_owned(),
            min_suitability: None,
            keep_wordless: false,
            out_dir: None,
        }
    }
}

/// One package that was written.
#[derive(Debug, Clone)]
pub struct Built {
    /// The package.
    pub path: PathBuf,
    /// The description it was built from, which `km-pack build` builds again.
    pub description: PathBuf,
    /// The plain-text listing beside it.
    pub listing: Option<PathBuf>,
    /// How many songs it holds.
    pub songs: usize,
}

/// What packaging a folder gave.
#[derive(Debug, Default)]
pub struct Packaged {
    /// The packages written, one per volume.
    pub packages: Vec<Built>,
    /// The files left out when the folder was read, each with the reason.
    pub rejected: Vec<(PathBuf, Rejection)>,
    /// The songs a build could not put in, each with the reason.
    pub skipped: Vec<Skipped>,
    /// Whether the caller stopped the work.
    pub canceled: bool,
}

/// A description and the two files it leads to.
struct Planned {
    spec: Spec,
    description: PathBuf,
    out: PathBuf,
}

/// Divides one description into volumes and says where each is written.
fn plan(spec: &Spec, out_dir: &Path) -> Vec<Planned> {
    split_into_volumes(spec, usize::from(km_songcode::MAX_SLOT))
        .into_iter()
        .map(|mut spec| {
            let package = &spec.package;
            let stem = km_kmpkg::name_slug(&package.name).unwrap_or_else(|| package.id.clone());
            let version = if km_kmpkg::is_safe_name(&package.version) {
                package.version.clone()
            } else {
                km_kmpkg::name_slug(&package.version).unwrap_or_else(|| "1.0.0".to_owned())
            };
            let out = out_dir.join(format!("{stem}-{version}.kmpkg"));
            spec.package.out = Some(slashed(&out));
            Planned {
                description: out_dir.join(format!("{stem}.kmspec.yaml")),
                out,
                spec,
            }
        })
        .collect()
}

/// A path in the spelling a description holds on every system.
fn slashed(path: &Path) -> String {
    km_pack::spec::slashed(&path.display().to_string())
}

/// Reads the song files under `dir` and writes them as one package, or as volumes past the number
/// one package holds.
///
/// A file with no words stays where it is and goes into no package, unless `keep_wordless` says
/// otherwise. Every package is marked uncurated.
///
/// # Errors
///
/// When the folder is not there, the language is not a known code, or a package cannot be written.
/// A folder that yields no song is not an error: [`Packaged::packages`] is empty.
pub fn package(
    dir: &Path,
    options: &PackageOptions,
    mut on: impl FnMut(Event<'_>) -> ControlFlow<()>,
) -> Result<Packaged> {
    if !dir.is_dir() {
        bail!("{} is not a folder", dir.display());
    }
    let dir =
        std::path::absolute(dir).with_context(|| format!("could not resolve {}", dir.display()))?;
    let language = km_kmpkg::Language::parse(options.default_language.trim())
        .with_context(|| {
            format!(
                "{:?} is not an ISO 639-1 language code (try `und`)",
                options.default_language
            )
        })?
        .code()
        .to_owned();
    let name = options
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            dir.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "package".to_owned());
    let out_dir = match &options.out_dir {
        Some(out_dir) => std::path::absolute(out_dir)
            .with_context(|| format!("could not resolve {}", out_dir.display()))?,
        None => dir.parent().unwrap_or(&dir).to_path_buf(),
    };

    let mut report = Packaged::default();
    let mut stopped = false;
    let described = km_pack::describe(
        &dir,
        &DescribeOptions {
            id: km_kmpkg::PackageMeta::new_id(),
            name,
            version: options.version.clone(),
            publisher: options.publisher.clone(),
            default_language: Some(language),
            min_suitability: options.min_suitability,
            require_lyrics: !options.keep_wordless,
            // The division into volumes happens below, so the walk is not held to one package.
            unbounded: true,
            ..DescribeOptions::default()
        },
        |done, total| {
            if !stopped {
                stopped = on(Event::Read { done, total }).is_break();
            }
        },
    )?;
    report.rejected = described.rejected;
    if stopped {
        report.canceled = true;
        return Ok(report);
    }
    if described.spec.songs.is_empty() {
        return Ok(report);
    }

    let mut spec = described.spec;
    spec.root = Some(slashed(&dir));
    let planned = plan(&spec, &out_dir);
    let volumes = planned.len();

    for (volume, planned) in planned.iter().enumerate() {
        planned.spec.write(&planned.description)?;
        let total = planned.spec.songs.len();
        let outcome = km_pack::build::build(
            &planned.spec,
            &BuildOptions {
                base: &dir,
                out: Some(&planned.out),
                dry_run: false,
                measure_loudness: true,
                write_listing: true,
                flags: km_kmpkg::PackageFlags::NONE,
            },
            |event| match event {
                BuildEvent::Song { index, .. } => on(Event::Build {
                    volume,
                    volumes,
                    done: index,
                    total,
                }),
                BuildEvent::Writing { out } => on(Event::Write { out }),
                _ => ControlFlow::Continue(()),
            },
        )?;

        if outcome.canceled {
            report.canceled = true;
            return Ok(report);
        }
        if !outcome.problems.is_empty() {
            bail!(
                "{} was not written: {}",
                planned.out.display(),
                outcome.problems.join("; ")
            );
        }
        if !outcome.unlanguaged.is_empty() {
            bail!(
                "{} was not written: {} song(s) name no language",
                planned.out.display(),
                outcome.unlanguaged.len()
            );
        }
        let wrote = outcome.wrote();
        report.skipped.extend(outcome.skipped);
        if wrote {
            report.packages.push(Built {
                path: outcome.out_path,
                description: planned.description.clone(),
                listing: outcome.listing_path,
                songs: outcome.written,
            });
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use km_pack::{SpecPackage, SpecSong};

    use super::*;

    fn spec_of(songs: u32) -> Spec {
        Spec {
            package: SpecPackage {
                id: km_kmpkg::PackageMeta::new_id(),
                name: "Site Songs".to_owned(),
                version: "1.0.0".to_owned(),
                publisher: None,
                created: None,
                volume: None,
                default_language: Some("und".to_owned()),
                encoding: None,
                start_number: 1,
                transcode: true,
                out: None,
                uncurated: true,
            },
            root: None,
            songs: (1..=songs)
                .map(|number| SpecSong {
                    file: format!("{number}.kar"),
                    number: Some(number),
                    ..SpecSong::default()
                })
                .collect(),
        }
    }

    #[test]
    fn a_folder_that_fits_is_one_package() {
        let planned = plan(&spec_of(999), Path::new("out"));
        assert_eq!(planned.len(), 1);
        assert_eq!(
            planned[0].out,
            Path::new("out").join("site-songs-1.0.0.kmpkg")
        );
        assert_eq!(
            planned[0].description,
            Path::new("out").join("site-songs.kmspec.yaml")
        );
        assert!(planned[0].spec.package.uncurated);
    }

    #[test]
    fn a_folder_past_one_package_is_divided_into_volumes() {
        let planned = plan(&spec_of(1000), Path::new("out"));
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].spec.songs.len(), 999);
        assert_eq!(planned[1].spec.songs.len(), 1);
        assert_eq!(
            planned[1].out,
            Path::new("out").join("site-songs-vol2-1.0.0.kmpkg")
        );
        assert_ne!(planned[0].description, planned[1].description);
        for volume in &planned {
            volume.spec.validate().expect("a volume is a valid package");
        }
    }
}
