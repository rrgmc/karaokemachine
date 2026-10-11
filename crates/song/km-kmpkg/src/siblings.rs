//! Finding one file's partner among the names of one folder.
//!
//! **Pure functions over names, because a folder is not the only place a pair sits.** A directory
//! listing and an archive's entry list both reduce to the file names of one folder, and a pairing
//! rule written once per source is two rules that drift apart.

use std::path::Path;

/// Finds the name among `names` with the same stem as `name` and one of the `wanted` extensions.
///
/// `names` are the bare file names of one folder. The obvious spellings are tried first, each
/// extension in lower case and then in upper case. Only if none is there is a name matched on a
/// trimmed, lowercased stem and an extension in any case.
///
/// **Tolerant because a real corpus is not tidy**: extensions come in both cases in one folder, and
/// a pair can differ by a trailing space in the stem.
#[must_use]
pub fn sibling_among<'a>(name: &str, wanted: &[&str], names: &[&'a str]) -> Option<&'a str> {
    let path = Path::new(name);
    for extension in wanted {
        for spelling in [
            extension.to_ascii_lowercase(),
            extension.to_ascii_uppercase(),
        ] {
            let candidate = path.with_extension(spelling);
            if let Some(found) = names
                .iter()
                .find(|other| Path::new(other) == candidate.as_path())
            {
                return Some(found);
            }
        }
    }

    let stem = path.file_stem()?.to_str()?.trim().to_lowercase();
    names.iter().copied().find(|other| {
        let other = Path::new(other);
        other
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| wanted.iter().any(|want| ext.eq_ignore_ascii_case(want)))
            && other
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.trim().to_lowercase() == stem)
    })
}

/// Finds the name among `names` that a lyrics file's header gives for its audio.
///
/// The exact name first, then the same name without regard to case, because a header written on one
/// file system names a file renamed on another.
#[must_use]
pub fn named_among<'a>(wanted: &str, names: &[&'a str]) -> Option<&'a str> {
    if let Some(found) = names.iter().find(|name| **name == wanted) {
        return Some(found);
    }
    let lowered = wanted.to_lowercase();
    names
        .iter()
        .copied()
        .find(|name| name.to_lowercase() == lowered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AUDIO_EXTENSIONS, GRAPHICS_EXTENSION, sibling_with_extension};
    use km_testkit::Scratch;

    #[test]
    fn a_sibling_is_found_by_the_obvious_spelling_before_the_tolerant_one() {
        let names = ["song.cdg", "song.mp3", "song .mp3"];
        assert_eq!(
            sibling_among("song.cdg", &AUDIO_EXTENSIONS, &names),
            Some("song.mp3")
        );
    }

    #[test]
    fn a_sibling_is_found_in_either_case_and_past_a_trailing_space() {
        assert_eq!(
            sibling_among("loud.cdg", &AUDIO_EXTENSIONS, &["loud.cdg", "loud.MP3"]),
            Some("loud.MP3")
        );
        assert_eq!(
            sibling_among(
                "Loud.mp3",
                &[GRAPHICS_EXTENSION],
                &["Loud.mp3", "loud .Cdg"]
            ),
            Some("loud .Cdg")
        );
        assert_eq!(
            sibling_among("loud.cdg", &AUDIO_EXTENSIONS, &["loud.cdg", "quiet.mp3"]),
            None
        );
    }

    #[test]
    fn a_named_file_is_found_exactly_and_then_without_case() {
        let names = ["Someone - Song.MP3", "someone - song.mp3"];
        assert_eq!(
            named_among("someone - song.mp3", &names),
            Some("someone - song.mp3")
        );
        assert_eq!(
            named_among("SOMEONE - SONG.MP3", &names),
            Some("Someone - Song.MP3")
        );
        assert_eq!(named_among("other.mp3", &names), None);
    }

    /// The folder search and the name search are one rule, so each case is asked of both.
    #[test]
    fn a_folder_and_a_list_of_names_give_the_same_sibling() {
        let cases: [(&str, &[&str], Option<&str>); 4] = [
            ("a.cdg", &["a.cdg", "a.mp3"], Some("a.mp3")),
            ("b.cdg", &["b.cdg", "b.MP3"], Some("b.MP3")),
            ("c.cdg", &["c.cdg", "c .mp3"], Some("c .mp3")),
            ("d.cdg", &["d.cdg", "e.mp3"], None),
        ];
        for (index, (name, names, expected)) in cases.into_iter().enumerate() {
            assert_eq!(sibling_among(name, &AUDIO_EXTENSIONS, names), expected);

            let dir = Scratch::new(&format!("sibling-parity-{index}"));
            for other in names {
                dir.write(other, b"x");
            }
            let found = sibling_with_extension(&dir.join(name), &AUDIO_EXTENSIONS);
            let found = found
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|found| found.to_str())
                .map(str::to_lowercase);
            // Compared without case: a file system that ignores it answers to the lower-case
            // spelling tried first, and hands that spelling back.
            assert_eq!(found, expected.map(str::to_lowercase), "{name}");
        }
    }
}
