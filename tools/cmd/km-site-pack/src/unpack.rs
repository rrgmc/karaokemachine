//! Opens the archives a site offers its songs in.

use std::io::Read as _;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::Event;
use crate::download::{looks_like, write_whole};
use crate::names::{Extensions, Taken, safe_file_name};

/// The most entries read from one archive.
const MAX_ENTRIES: usize = 20_000;

/// The most one song file out of an archive may weigh.
const MAX_ENTRY: u64 = 32 * 1024 * 1024;

/// The most one archive may unpack to, which is what bounds an archive built to fill a disk.
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;

/// What unpacking brought out.
#[derive(Debug, Default)]
pub struct Unpacked {
    /// How many archives were opened by this run.
    pub archives: usize,
    /// How many archives a run before this one had opened, which were left as they were.
    pub kept: usize,
    /// The song files written.
    pub written: Vec<PathBuf>,
    /// What was left inside an archive, or an archive left shut, each with the reason.
    pub skipped: Vec<(String, String)>,
    /// Whether the caller stopped the work.
    pub canceled: bool,
}

/// Opens every `.zip` directly inside `dir` and writes its song files to a folder beside it.
///
/// `songs.zip` unpacks to `songs/`. Each entry is written under its own base name, whatever folder
/// the archive claims for it. An archive whose folder is there already is left shut.
///
/// # Errors
///
/// When the folder cannot be read or written. An archive that cannot be opened is reported in
/// [`Unpacked::skipped`] and does not stop the run.
pub fn unpack(
    dir: &Path,
    extensions: &Extensions,
    mut on: impl FnMut(Event<'_>) -> ControlFlow<()>,
) -> Result<Unpacked> {
    let mut archives: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("could not read {}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(Extensions::is_archive)
        })
        .collect();
    archives.sort();

    let mut report = Unpacked::default();
    for archive in archives {
        let Some(stem) = archive.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let target = dir.join(stem);
        if target.is_dir() {
            report.kept += 1;
            continue;
        }
        if on(Event::Unpack { archive: &archive }).is_break() {
            report.canceled = true;
            break;
        }
        let label = archive
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match unpack_one(&archive, &target, extensions, &label, &mut report) {
            Ok(()) => report.archives += 1,
            Err(error) => report.skipped.push((label, format!("{error:#}"))),
        }
    }
    Ok(report)
}

/// Writes the song files of one archive into `target`.
fn unpack_one(
    archive: &Path,
    target: &Path,
    extensions: &Extensions,
    label: &str,
    report: &mut Unpacked,
) -> Result<()> {
    let file = std::fs::File::open(archive)
        .with_context(|| format!("could not open {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .context("it is not a readable zip archive")?;

    let mut taken = Taken::default();
    let mut total: u64 = 0;
    let count = zip.len();
    if count > MAX_ENTRIES {
        report.skipped.push((
            label.to_owned(),
            format!("only the first {MAX_ENTRIES} of its {count} entries were read"),
        ));
    }

    for index in 0..count.min(MAX_ENTRIES) {
        let mut entry = match zip.by_index(index) {
            Ok(entry) => entry,
            // An entry behind a password arrives here, and so does one this build cannot inflate.
            Err(error) => {
                report
                    .skipped
                    .push((format!("{label}: entry {}", index + 1), error.to_string()));
                continue;
            }
        };
        if entry.is_dir() {
            continue;
        }
        let inside = entry.name().to_owned();
        let Some(name) = safe_file_name(&inside) else {
            continue;
        };
        if Extensions::is_archive(&name) {
            report.skipped.push((
                format!("{label}: {inside}"),
                "an archive inside an archive is not opened".to_owned(),
            ));
            continue;
        }
        if !extensions.is_song(&name) {
            continue;
        }

        // The size an archive states is the archive's own claim, so the read is what is bounded.
        let mut bytes = Vec::new();
        if let Err(error) = entry.by_ref().take(MAX_ENTRY + 1).read_to_end(&mut bytes) {
            report
                .skipped
                .push((format!("{label}: {inside}"), error.to_string()));
            continue;
        }
        let fault = if bytes.len() as u64 > MAX_ENTRY {
            Some(format!("larger than {} MiB", MAX_ENTRY / (1024 * 1024)))
        } else if !looks_like(&name, &bytes) {
            Some("it is not a MIDI file".to_owned())
        } else {
            None
        };
        if let Some(fault) = fault {
            report.skipped.push((format!("{label}: {inside}"), fault));
            continue;
        }
        total += bytes.len() as u64;
        if total > MAX_TOTAL {
            report.skipped.push((
                label.to_owned(),
                format!(
                    "it unpacks to more than {} MiB, and the rest was left inside",
                    MAX_TOTAL / (1024 * 1024)
                ),
            ));
            break;
        }

        std::fs::create_dir_all(target)
            .with_context(|| format!("could not create {}", target.display()))?;
        let path = target.join(taken.claim(&name));
        write_whole(&path, &bytes)?;
        report.written.push(path);
    }
    Ok(())
}
