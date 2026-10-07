//! Brings the listed files into a folder.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use url::Url;

use crate::Event;
use crate::client::Client;
use crate::crawl::FileLink;
use crate::names::{Extensions, Taken, last_segment, safe_file_name};

/// The most one downloaded file may weigh. A karaoke MIDI is tens of kilobytes; an archive of a
/// few hundred of them is the largest thing a site offers.
pub const MAX_FILE: u64 = 256 * 1024 * 1024;

/// What a download brought in.
#[derive(Debug, Default)]
pub struct Downloaded {
    /// The files written by this run.
    pub written: Vec<PathBuf>,
    /// How many files were in the folder already and were left as they were.
    pub kept: usize,
    /// The addresses that gave no usable file, each with the reason.
    pub refused: Vec<(Url, String)>,
    /// Whether the caller stopped the download.
    pub canceled: bool,
}

/// Whether a file's first bytes are those of the kind its name claims.
///
/// A server that has lost a file often answers with a page and status 200. The page would
/// otherwise sit in the folder under a song's name.
#[must_use]
pub fn looks_like(name: &str, head: &[u8]) -> bool {
    if Extensions::is_archive(name) {
        head.starts_with(b"PK")
    } else {
        head.starts_with(b"MThd") || head.starts_with(b"RIFF")
    }
}

/// The name each listed file is saved under, in the order given.
///
/// A file whose address yields no usable name gets none. Two addresses that end in one name get
/// two names, and the same list always gives the same answer, so a second run finds its files.
#[must_use]
pub fn plan_names(files: &[FileLink]) -> Vec<Option<String>> {
    let mut taken = Taken::default();
    files
        .iter()
        .map(|file| {
            let raw = last_segment(&file.url)?;
            Some(taken.claim(&safe_file_name(&raw)?))
        })
        .collect()
}

/// Downloads each listed file into `dir`, one at a time.
///
/// A file already in the folder is left alone, so a run that was stopped carries on where it was.
///
/// # Errors
///
/// When the folder cannot be created or written. A file that cannot be fetched is reported in
/// [`Downloaded::refused`] and does not stop the run.
pub fn download(
    client: &Client,
    files: &[FileLink],
    dir: &Path,
    mut on: impl FnMut(Event<'_>) -> ControlFlow<()>,
) -> Result<Downloaded> {
    std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    let mut report = Downloaded::default();
    let names = plan_names(files);

    for (index, (file, name)) in files.iter().zip(names).enumerate() {
        if on(Event::Download {
            url: &file.url,
            index,
            total: files.len(),
        })
        .is_break()
        {
            report.canceled = true;
            break;
        }
        let Some(name) = name else {
            report
                .refused
                .push((file.url.clone(), "its address names no file".to_owned()));
            continue;
        };
        let target = dir.join(&name);
        if target.is_file() {
            report.kept += 1;
            continue;
        }
        let answer = match client.get(&file.url, MAX_FILE) {
            Ok(answer) => answer,
            Err(error) => {
                report
                    .refused
                    .push((file.url.clone(), format!("{error:#}")));
                continue;
            }
        };
        let fault = if answer.challenged {
            Some("the site answers only a browser".to_owned())
        } else if !answer.is_ok() {
            Some(format!("status {}", answer.status))
        } else if answer.too_large {
            Some(format!("larger than {} MiB", MAX_FILE / (1024 * 1024)))
        } else if !looks_like(&name, &answer.body) {
            Some("the server sent something that is not this kind of file".to_owned())
        } else {
            None
        };
        if let Some(fault) = fault {
            report.refused.push((file.url.clone(), fault));
            continue;
        }
        write_whole(&target, &answer.body)?;
        report.written.push(target);
    }
    Ok(report)
}

/// Writes a file under a second name and renames it, so a stopped run leaves no half file under a
/// name the next run would trust.
pub(crate) fn write_whole(target: &Path, bytes: &[u8]) -> Result<()> {
    let mut part = target.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    std::fs::write(&part, bytes).with_context(|| format!("could not write {}", part.display()))?;
    std::fs::rename(&part, target)
        .with_context(|| format!("could not write {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(address: &str) -> FileLink {
        FileLink {
            url: Url::parse(address).expect("a url"),
            page: Url::parse("http://127.0.0.1/").expect("a url"),
        }
    }

    #[test]
    fn two_addresses_ending_in_one_name_get_two_names() {
        let names = plan_names(&[
            link("http://127.0.0.1/a/song.kar"),
            link("http://127.0.0.1/b/song.kar"),
            link("http://127.0.0.1/c/"),
            link("http://127.0.0.1/d/jos%C3%A9.mid"),
        ]);
        assert_eq!(
            names,
            [
                Some("song.kar".to_owned()),
                Some("song (2).kar".to_owned()),
                None,
                Some("josé.mid".to_owned()),
            ]
        );
    }

    #[test]
    fn an_encoded_separator_cannot_leave_the_folder() {
        let names = plan_names(&[link("http://127.0.0.1/a/..%2F..%2Fevil.kar")]);
        assert_eq!(names, [Some("evil.kar".to_owned())]);
    }

    #[test]
    fn a_page_under_a_song_name_is_not_a_song() {
        assert!(looks_like("a.kar", b"MThd\0\0\0\x06"));
        assert!(looks_like("a.mid", b"RIFF...."));
        assert!(!looks_like("a.kar", b"<!DOCTYPE html>"));
        assert!(looks_like("a.zip", b"PK\x03\x04"));
        assert!(!looks_like("a.zip", b"MThd"));
    }
}
