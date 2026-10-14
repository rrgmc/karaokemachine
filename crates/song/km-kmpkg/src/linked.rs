//! Reading a song file a linked package names, from inside one folder and nowhere else.
//!
//! **A linked package holds paths, and a path in a package is a claim somebody else wrote.** The
//! machine joins each one onto the folder its owner configured. A path that climbs, an absolute
//! path, or a link placed inside that folder could each make the machine read a file its owner
//! never offered.
//!
//! **Two checks, because each sees what the other cannot.** [`check`] reads the words of the path
//! and touches no disk, so a package is refused when it opens. [`open`] asks the file system where
//! the path really leads, which is the only way to see a symbolic link or a junction.
//!
//! **Nothing here answers a path.** [`open`] gives a file already open for reading, so no caller
//! holds a name it could write to or delete.
//!
//! See `A linked file is read only from inside one folder` in `docs/decisions/packaging.md`.

use std::fs::File;
use std::path::Path;

/// The longest relative path a linked package may record, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;

/// The longest single folder or file name in a linked path, in bytes.
///
/// It is the limit of every file system the machine runs on.
pub const MAX_COMPONENT_BYTES: usize = 255;

/// The names Windows gives to devices, which no file may take in any folder.
///
/// Windows matches them without regard to case and ignores an extension, so `nul.mid` is the null
/// device. They are refused on every platform, and a package therefore reads the same everywhere.
const DEVICE_NAMES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// The device families that take one digit, `COM1` and `LPT1` among them.
const NUMBERED_DEVICES: [&str; 2] = ["COM", "LPT"];

/// Why a linked path was refused.
#[derive(Debug, thiserror::Error)]
pub enum LinkedPathError {
    /// The path is empty.
    #[error("a linked file has no path")]
    Empty,

    /// The path, or one name in it, is longer than a file system holds.
    #[error("the linked path '{0}' is too long")]
    TooLong(String),

    /// The path holds a control character.
    #[error("the linked path {0:?} holds a control character")]
    ControlCharacter(String),

    /// The path holds a backslash, which is a separator on one platform and a letter on another.
    #[error("the linked path '{0}' holds a backslash")]
    Backslash(String),

    /// The path holds a colon, which names a drive or an alternate data stream.
    #[error("the linked path '{0}' holds a colon")]
    Colon(String),

    /// The path starts at the root of a file system.
    #[error("the linked path '{0}' is not relative")]
    Absolute(String),

    /// One name in the path is empty, `.` or `..`.
    #[error("the linked path '{0}' does not name a file below its folder")]
    BadComponent(String),

    /// One name in the path ends in a dot or a space, which Windows removes.
    #[error("the linked path '{0}' has a name ending in a dot or a space")]
    TrailingDotOrSpace(String),

    /// One name in the path is a Windows device.
    #[error("the linked path '{0}' names a device")]
    DeviceName(String),

    /// The folder linked files are read from cannot be used.
    #[error("the folder for linked files cannot be read: {0}")]
    RootUnusable(#[source] std::io::Error),

    /// No file is at the path.
    #[error("the linked file '{0}' is not there")]
    NotFound(String),

    /// The path leads outside the folder linked files are read from.
    #[error("the linked path '{0}' leads outside the folder for linked files")]
    Outside(String),

    /// The path leads to something that is not a file.
    #[error("the linked path '{0}' is not a file")]
    NotAFile(String),

    /// The file is there and could not be read.
    #[error("the linked file '{path}' cannot be read: {source}")]
    Io {
        /// The path as the package records it.
        path: String,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
}

/// Checks the words of a linked path, without touching a disk.
///
/// A path that passes is relative, uses `/` alone, and names a file a folder on any platform can
/// hold. **It refuses and never repairs**, for the reason
/// [`is_safe_path`](crate::manifest::is_safe_path) gives.
///
/// # Errors
///
/// The first rule the path breaks, in the order the variants of [`LinkedPathError`] are declared.
pub fn check(relative: &str) -> Result<(), LinkedPathError> {
    let named = || relative.to_owned();
    if relative.is_empty() {
        return Err(LinkedPathError::Empty);
    }
    if relative.len() > MAX_PATH_BYTES {
        return Err(LinkedPathError::TooLong(named()));
    }
    if relative.chars().any(char::is_control) {
        return Err(LinkedPathError::ControlCharacter(named()));
    }
    if relative.contains('\\') {
        return Err(LinkedPathError::Backslash(named()));
    }
    if relative.contains(':') {
        return Err(LinkedPathError::Colon(named()));
    }
    if relative.starts_with('/') {
        return Err(LinkedPathError::Absolute(named()));
    }
    for component in relative.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(LinkedPathError::BadComponent(named()));
        }
        if component.len() > MAX_COMPONENT_BYTES {
            return Err(LinkedPathError::TooLong(named()));
        }
        if component.ends_with(['.', ' ']) {
            return Err(LinkedPathError::TrailingDotOrSpace(named()));
        }
        if is_device_name(component) {
            return Err(LinkedPathError::DeviceName(named()));
        }
    }
    Ok(())
}

/// Whether Windows reads one folder or file name as a device.
fn is_device_name(component: &str) -> bool {
    // Windows looks at what stands before the first dot, and ignores spaces after it.
    let stem = component.split('.').next().unwrap_or(component).trim_end();
    if DEVICE_NAMES
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
    {
        return true;
    }
    NUMBERED_DEVICES.iter().any(|family| {
        let mut rest = stem.chars().skip(family.len());
        stem.get(..family.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(family))
            // The superscript digits count too: Windows takes `COM¹` for `COM1`.
            && rest
                .next()
                .is_some_and(|digit| digit.is_ascii_digit() || matches!(digit, '¹' | '²' | '³'))
            && rest.next().is_none()
    })
}

/// A linked file, open for reading.
///
/// It holds no path. The file was confirmed to sit inside its folder before it was opened, and a
/// handle cannot be turned back into a name to write to.
#[derive(Debug)]
pub struct LinkedFile {
    file: File,
    len: u64,
}

impl LinkedFile {
    /// How long the file is, in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the file holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The file itself, positioned at its start.
    #[must_use]
    pub fn into_file(self) -> File {
        self.file
    }
}

/// Opens the file a linked path names, when it really sits inside `root`.
///
/// The words are checked with [`check`] first. Then the file system resolves both `root` and the
/// joined path, every symbolic link and junction followed. The resolved file must lie below the
/// resolved folder. A link that stays inside the folder is followed, and one that leaves is refused.
///
/// The resolved path is the one opened, so a link changed after the comparison cannot redirect the
/// read. What stays possible is a local process replacing a folder between the two steps. A package
/// cannot do that, and a package is what this guards against.
///
/// # Errors
///
/// Whatever [`check`] refuses, [`LinkedPathError::RootUnusable`] when `root` is not a folder that
/// can be read, and one of `NotFound`, `Outside`, `NotAFile` or `Io` for the file.
pub fn open(root: &Path, relative: &str) -> Result<LinkedFile, LinkedPathError> {
    check(relative)?;
    let named = || relative.to_owned();
    let failed = |source: std::io::Error| {
        if source.kind() == std::io::ErrorKind::NotFound {
            LinkedPathError::NotFound(named())
        } else {
            LinkedPathError::Io {
                path: named(),
                source,
            }
        }
    };

    let root = root.canonicalize().map_err(LinkedPathError::RootUnusable)?;
    if !root.is_dir() {
        return Err(LinkedPathError::RootUnusable(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "not a folder",
        )));
    }

    // Both sides are resolved forms, so one comparison of whole components is exact. On Windows
    // each carries the `\\?\` prefix and the spelling the disk holds.
    let target = root.join(relative).canonicalize().map_err(failed)?;
    if !target.starts_with(&root) {
        return Err(LinkedPathError::Outside(named()));
    }

    let file = File::open(&target).map_err(failed)?;
    // Asked of the handle, so the answer is about the file that will be read.
    let metadata = file.metadata().map_err(failed)?;
    if !metadata.is_file() {
        return Err(LinkedPathError::NotAFile(named()));
    }
    Ok(LinkedFile {
        file,
        len: metadata.len(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use km_testkit::Scratch;

    use super::*;

    fn contents(root: &Path, relative: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        open(root, relative)
            .expect("the linked file opens")
            .into_file()
            .read_to_end(&mut bytes)
            .expect("the linked file reads");
        bytes
    }

    #[test]
    fn a_plain_relative_path_is_accepted() {
        for path in [
            "song.mid",
            "rock/song.mid",
            "A Band/An Album (1999)/01 - Song.mp3",
            "Músicas/canção.kar",
            "console.mid",
            "com.mid",
            "com10.mid",
            ".hidden/song.mid",
        ] {
            assert!(check(path).is_ok(), "{path} should pass");
        }
    }

    #[test]
    fn an_absolute_path_is_refused() {
        assert!(matches!(
            check("/tunes/karaoke/song.mid"),
            Err(LinkedPathError::Absolute(_))
        ));
    }

    #[test]
    fn a_drive_letter_is_refused() {
        for path in ["D:/tunes/song.mid", "D:song.mid", "song.mid:stream"] {
            assert!(
                matches!(check(path), Err(LinkedPathError::Colon(_))),
                "{path}"
            );
        }
    }

    #[test]
    fn a_backslash_is_refused() {
        for path in [
            "rock\\song.mid",
            "\\\\server\\share\\song.mid",
            "..\\song.mid",
        ] {
            assert!(
                matches!(check(path), Err(LinkedPathError::Backslash(_))),
                "{path}"
            );
        }
    }

    #[test]
    fn a_parent_component_is_refused_wherever_it_falls() {
        for path in ["../song.mid", "rock/../song.mid", "rock/..", ".."] {
            assert!(
                matches!(check(path), Err(LinkedPathError::BadComponent(_))),
                "{path}"
            );
        }
    }

    #[test]
    fn an_empty_or_dot_component_is_refused() {
        for path in ["rock//song.mid", "./song.mid", "rock/./song.mid", "rock/"] {
            assert!(
                matches!(check(path), Err(LinkedPathError::BadComponent(_))),
                "{path}"
            );
        }
        assert!(matches!(check(""), Err(LinkedPathError::Empty)));
    }

    #[test]
    fn a_device_name_is_refused_in_any_case_and_with_an_extension() {
        for path in [
            "NUL",
            "nul.mid",
            "rock/Con.tar.gz",
            "aux/song.mid",
            "COM1",
            "lpt9.mid",
            "com0.mid",
            "COM¹.mid",
            "conin$.mid",
            "nul .mid",
        ] {
            assert!(
                matches!(check(path), Err(LinkedPathError::DeviceName(_))),
                "{path}"
            );
        }
    }

    #[test]
    fn a_component_ending_in_a_dot_or_a_space_is_refused() {
        for path in [
            "song.mid.",
            "song.mid ",
            "rock./song.mid",
            "rock /song.mid",
            "...",
        ] {
            assert!(
                matches!(check(path), Err(LinkedPathError::TrailingDotOrSpace(_))),
                "{path}"
            );
        }
    }

    #[test]
    fn a_control_character_is_refused() {
        for path in ["song\0.mid", "rock\n/song.mid", "song\u{7f}.mid"] {
            assert!(
                matches!(check(path), Err(LinkedPathError::ControlCharacter(_))),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_path_longer_than_a_file_system_holds_is_refused() {
        let long_name = format!("{}.mid", "a".repeat(MAX_COMPONENT_BYTES));
        assert!(matches!(
            check(&long_name),
            Err(LinkedPathError::TooLong(_))
        ));
        let long_path = vec!["folder"; MAX_PATH_BYTES / 6].join("/");
        assert!(matches!(
            check(&long_path),
            Err(LinkedPathError::TooLong(_))
        ));
    }

    #[test]
    fn a_file_below_the_folder_is_opened_and_read() {
        let scratch = Scratch::new("linked-reads");
        scratch.write("library/rock/song.mid", b"notes");
        let root = scratch.join("library");

        let file = open(&root, "rock/song.mid").expect("opens");
        assert_eq!(file.len(), 5);
        assert!(!file.is_empty());
        assert_eq!(contents(&root, "rock/song.mid"), b"notes");
    }

    #[test]
    fn a_bad_path_is_refused_before_the_disk_is_asked() {
        let scratch = Scratch::new("linked-words-first");
        scratch.write("secret.mid", b"private");
        std::fs::create_dir_all(scratch.join("library")).expect("make the folder");

        assert!(matches!(
            open(&scratch.join("library"), "../secret.mid"),
            Err(LinkedPathError::BadComponent(_))
        ));
    }

    #[test]
    fn a_missing_file_is_its_own_error() {
        let scratch = Scratch::new("linked-missing-file");
        assert!(matches!(
            open(scratch.path(), "rock/song.mid"),
            Err(LinkedPathError::NotFound(_))
        ));
    }

    #[test]
    fn a_missing_root_is_its_own_error() {
        let scratch = Scratch::new("linked-missing-root");
        assert!(matches!(
            open(&scratch.join("nowhere"), "song.mid"),
            Err(LinkedPathError::RootUnusable(_))
        ));
    }

    #[test]
    fn a_root_that_is_a_file_is_refused() {
        let scratch = Scratch::new("linked-root-is-a-file");
        let root = scratch.write("library", b"not a folder");
        assert!(matches!(
            open(&root, "song.mid"),
            Err(LinkedPathError::RootUnusable(_))
        ));
    }

    #[test]
    fn a_directory_is_not_a_song_file() {
        let scratch = Scratch::new("linked-directory");
        scratch.write("rock/song.mid", b"notes");
        assert!(matches!(
            open(scratch.path(), "rock"),
            Err(LinkedPathError::NotAFile(_) | LinkedPathError::Io { .. })
        ));
    }

    /// Makes `link` lead to the folder `target`, the way an unprivileged account can on each
    /// platform: a symbolic link on unix and a junction on Windows.
    fn link_folder(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).expect("make the symbolic link");
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                // Rebuilt from its components, because `mklink` reads a `/` as the start of an option.
                .arg(link.components().collect::<std::path::PathBuf>())
                .arg(target.components().collect::<std::path::PathBuf>())
                .output()
                .expect("run mklink");
            assert!(made.status.success(), "mklink /J failed: {made:?}");
        }
    }

    #[test]
    fn a_link_leaving_the_folder_is_refused() {
        let scratch = Scratch::new("linked-link-out");
        scratch.write("private/secret.mid", b"private");
        std::fs::create_dir_all(scratch.join("library")).expect("make the folder");
        link_folder(&scratch.join("private"), &scratch.join("library/way-out"));

        assert!(matches!(
            open(&scratch.join("library"), "way-out/secret.mid"),
            Err(LinkedPathError::Outside(_))
        ));
    }

    #[test]
    fn a_link_staying_inside_the_folder_is_followed() {
        let scratch = Scratch::new("linked-link-in");
        scratch.write("library/rock/song.mid", b"notes");
        link_folder(
            &scratch.join("library/rock"),
            &scratch.join("library/favorites"),
        );

        assert_eq!(
            contents(&scratch.join("library"), "favorites/song.mid"),
            b"notes"
        );
    }

    #[test]
    fn a_root_reached_through_a_link_still_holds_its_files() {
        let scratch = Scratch::new("linked-root-link");
        scratch.write("real/song.mid", b"notes");
        link_folder(&scratch.join("real"), &scratch.join("library"));

        assert_eq!(contents(&scratch.join("library"), "song.mid"), b"notes");
    }

    #[test]
    fn a_sibling_folder_sharing_a_prefix_is_outside() {
        let scratch = Scratch::new("linked-prefix");
        scratch.write("karaoke2/song.mid", b"other");
        std::fs::create_dir_all(scratch.join("karaoke")).expect("make the folder");
        link_folder(
            &scratch.join("karaoke2"),
            &scratch.join("karaoke/next-door"),
        );

        assert!(matches!(
            open(&scratch.join("karaoke"), "next-door/song.mid"),
            Err(LinkedPathError::Outside(_))
        ));
    }
}
