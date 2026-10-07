//! What a file a stranger named is called on this disk.
//!
//! A server chooses the last segment of a URL and an archive chooses its entry names. Both arrive
//! here before anything is written, and what leaves is one plain file name with no folder in it.

use std::collections::BTreeSet;
use std::path::Path;

use percent_encoding::percent_decode_str;
use url::Url;

/// The longest name written, in bytes. Windows refuses a longer path component than 255 UTF-16
/// units, and a clash suffix and `.part` still have to fit.
const MAX_NAME: usize = 180;

/// The archive extension, which is opened rather than packaged.
pub const ARCHIVE: &str = "zip";

/// The song-file extensions a run takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extensions(Vec<String>);

impl Default for Extensions {
    /// `.kar`, `.mid` and `.midi`. A host that accepts only `.mid` serves its karaoke files under
    /// that name, so the words decide what a song is and the extension does not.
    fn default() -> Self {
        Self(vec!["kar".to_owned(), "mid".to_owned(), "midi".to_owned()])
    }
}

impl Extensions {
    /// Reads a list such as `kar,mid`. A leading dot and letter case are ignored.
    #[must_use]
    pub fn parse(list: &str) -> Self {
        let mut found: Vec<String> = list
            .split(',')
            .map(|item| item.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|item| !item.is_empty() && item != ARCHIVE)
            .collect();
        found.dedup();
        if found.is_empty() {
            Self::default()
        } else {
            Self(found)
        }
    }

    /// Whether a file of this name is a song file this run takes.
    #[must_use]
    pub fn is_song(&self, name: &str) -> bool {
        extension(name).is_some_and(|found| self.0.contains(&found))
    }

    /// Whether a file of this name is an archive to open.
    #[must_use]
    pub fn is_archive(name: &str) -> bool {
        extension(name).as_deref() == Some(ARCHIVE)
    }

    /// Whether a link to this name is worth downloading at all.
    #[must_use]
    pub fn is_wanted(&self, name: &str) -> bool {
        self.is_song(name) || Self::is_archive(name)
    }
}

/// The lowercase extension of a name, without its dot.
#[must_use]
pub fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|found| found.to_str())
        .map(str::to_ascii_lowercase)
}

/// The last path segment of a URL as a person would read it.
#[must_use]
pub fn last_segment(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.next_back()?;
    if segment.is_empty() {
        return None;
    }
    Some(percent_decode_str(segment).decode_utf8_lossy().into_owned())
}

/// Reduces a name from outside to one plain file name, or refuses it.
///
/// A folder separator, a drive colon and every character a file system reserves become `_`. So no
/// name that leaves here can reach a folder other than the one it is joined to.
#[must_use]
pub fn safe_file_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let mut name: String = last
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    while name.ends_with(['.', ' ']) {
        name.pop();
    }
    let name = name.trim_start().to_owned();
    if name.is_empty() || name.chars().all(|c| c == '.' || c == '_') {
        return None;
    }
    let name = shorten(&name);
    if is_reserved(&name) {
        return Some(format!("_{name}"));
    }
    Some(name)
}

/// Cuts a name to [`MAX_NAME`] bytes and keeps its extension.
fn shorten(name: &str) -> String {
    if name.len() <= MAX_NAME {
        return name.to_owned();
    }
    let (stem, tail) = split(name);
    let mut keep = MAX_NAME.saturating_sub(tail.len());
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{tail}", &stem[..keep])
}

/// A device name Windows opens instead of a file, whatever extension follows it.
fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| c.is_ascii_digit()))
}

/// A name as its stem and its extension, the dot staying with the extension.
fn split(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 => name.split_at(at),
        _ => (name, ""),
    }
}

/// Hands out names so that no two files of one run share one.
///
/// Letter case is folded for the comparison, because Windows and macOS hold `A.kar` and `a.kar` to
/// be the same file.
#[derive(Debug, Default)]
pub struct Taken(BTreeSet<String>);

impl Taken {
    /// Returns `name`, or `name (2)` and upward when an earlier file of this run took it.
    pub fn claim(&mut self, name: &str) -> String {
        if self.0.insert(name.to_lowercase()) {
            return name.to_owned();
        }
        let (stem, tail) = split(name);
        let mut count = 2_u32;
        loop {
            let candidate = format!("{stem} ({count}){tail}");
            if self.0.insert(candidate.to_lowercase()) {
                return candidate;
            }
            count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_keeps_its_letters_and_loses_its_folders() {
        assert_eq!(
            safe_file_name("josé - canción.kar").as_deref(),
            Some("josé - canción.kar")
        );
        assert_eq!(
            safe_file_name("../../etc/passwd.kar").as_deref(),
            Some("passwd.kar")
        );
        assert_eq!(
            safe_file_name(r"..\..\windows\a.mid").as_deref(),
            Some("a.mid")
        );
        assert_eq!(safe_file_name("C:evil.kar").as_deref(), Some("C_evil.kar"));
        assert_eq!(safe_file_name("what?.kar").as_deref(), Some("what_.kar"));
    }

    #[test]
    fn a_name_that_is_no_name_is_refused() {
        assert_eq!(safe_file_name(""), None);
        assert_eq!(safe_file_name(".."), None);
        assert_eq!(safe_file_name("a/.."), None);
        assert_eq!(safe_file_name("  .  "), None);
    }

    #[test]
    fn a_device_name_is_moved_aside() {
        assert_eq!(safe_file_name("con.kar").as_deref(), Some("_con.kar"));
        assert_eq!(safe_file_name("LPT1.mid").as_deref(), Some("_LPT1.mid"));
        assert_eq!(
            safe_file_name("console.kar").as_deref(),
            Some("console.kar")
        );
    }

    #[test]
    fn a_long_name_is_cut_and_keeps_its_extension() {
        let long = format!("{}.kar", "é".repeat(300));
        let cut = safe_file_name(&long).expect("a name");
        assert!(cut.len() <= MAX_NAME);
        assert!(cut.ends_with(".kar"));
    }

    #[test]
    fn a_second_file_of_the_same_name_is_numbered() {
        let mut taken = Taken::default();
        assert_eq!(taken.claim("a.kar"), "a.kar");
        assert_eq!(taken.claim("A.kar"), "A (2).kar");
        assert_eq!(taken.claim("a.kar"), "a (3).kar");
    }

    #[test]
    fn the_extension_list_is_read_loosely_and_never_holds_the_archive() {
        let only = Extensions::parse(" .KAR , zip ");
        assert!(only.is_song("x.kar"));
        assert!(!only.is_song("x.mid"));
        assert!(!only.is_song("x.zip"));
        assert!(only.is_wanted("x.ZIP"));
        assert!(Extensions::default().is_song("x.MIDI"));
        assert_eq!(Extensions::parse(" , "), Extensions::default());
    }

    #[test]
    fn the_last_segment_is_read_as_text() {
        let url = Url::parse("http://127.0.0.1/a/jos%C3%A9%20one.kar?x=1").expect("a url");
        assert_eq!(last_segment(&url).as_deref(), Some("josé one.kar"));
        let folder = Url::parse("http://127.0.0.1/a/").expect("a url");
        assert_eq!(last_segment(&folder), None);
    }
}
