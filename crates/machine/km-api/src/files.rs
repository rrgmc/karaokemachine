//! Replacing a file whole, so a reader sees the old contents or the new ones and never half of each.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes `contents` to `path` through a temporary file beside it, then renames it into place.
///
/// **The data is synced before the rename.** Without that, a power cut soon after can leave the new
/// name pointing at an empty file on some filesystems, which is worse than the old contents. The
/// parent directory is made if it is missing. A failure removes the temporary file, so nothing is
/// left behind that looks like the real one.
///
/// # Errors
///
/// Whatever creating the directory, writing, syncing or renaming reports.
pub fn replace(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temporary_for(path);
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(contents)?;
        file.sync_all()
    });
    written
        .and_then(|()| std::fs::rename(&temporary, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
}

/// The name a replacement is written under: the whole file name plus `.tmp`, so `a.json` and `a.txt`
/// never share one.
fn temporary_for(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use km_testkit::Scratch;

    use super::*;

    #[test]
    fn a_file_is_replaced_whole_and_nothing_is_left_beside_it() {
        let scratch = Scratch::new("files-replace");
        let path = scratch.path().join("nested").join("record.json");
        replace(&path, b"first").expect("the first write");
        replace(&path, b"second").expect("the second write");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        let names: Vec<_> = std::fs::read_dir(path.parent().expect("a parent"))
            .expect("list")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert_eq!(names, ["record.json"]);
    }

    #[test]
    fn a_failed_rename_leaves_no_temporary_file() {
        let scratch = Scratch::new("files-refused");
        // A directory where the file should go makes the rename fail on every platform.
        let path = scratch.path().join("taken");
        std::fs::create_dir(&path).expect("a directory in the way");
        std::fs::write(path.join("inside"), b"x").expect("keep it non-empty");
        assert!(replace(&path, b"data").is_err());
        assert!(!temporary_for(&path).exists());
    }
}
