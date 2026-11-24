//! Writing the assets the `.so` carries into the data folder, for the probe app.
//!
//! **A development arrangement.** M2 to M4 load the machine into the probe app over its port, and
//! the `.so` is the only thing that arrives, so the `tizen-embed-assets` feature compiles the font,
//! the wallpapers and the development console into it (see `build.rs`). Everything that reads them
//! wants a path, as on Android, so they are written out once into [`Paths::asset_dir`] and the
//! machine carries on with ordinary files. The app's `.tpk` carries them in `res/assets` instead,
//! read in place, and its library is built without them.
//!
//! The manifest and the removal of departed files are `androidassets`', and for the same reasons:
//! `size<TAB>path` lines, written last as the marker that the folder is complete, and only files the
//! previous manifest named are removed.
//!
//! Plain file writing with no FFI, so it is compiled and tested on every host (see `lib.rs`).

use std::fs;
use std::io;

use crate::settings::Paths;

/// The marker file, as on Android.
const MANIFEST: &str = "MANIFEST";

/// The manifest for a set of files: `size<TAB>path`, sorted by path.
pub(crate) fn manifest(files: &[(&str, &[u8])]) -> String {
    let mut lines: Vec<String> = files
        .iter()
        .map(|(path, bytes)| format!("{}\t{path}\n", bytes.len()))
        .collect();
    lines.sort_by(|a, b| a.split('\t').nth(1).cmp(&b.split('\t').nth(1)));
    lines.concat()
}

/// Writes `files` under the asset directory unless the installed manifest already matches.
///
/// Reports rather than fails, as on Android: each asset has a fallback, and a machine without its
/// wallpapers is still a machine.
pub(crate) fn unpack(paths: &Paths, files: &[(&str, &[u8])]) {
    if files.is_empty() {
        tracing::info!("this build carries no assets; the machine uses what is already on disk");
        return;
    }
    let wanted = manifest(files);
    let installed = fs::read_to_string(paths.asset(MANIFEST)).ok();
    if installed.as_deref() == Some(wanted.as_str()) {
        tracing::info!(
            files = files.len(),
            "the carried assets are already written"
        );
        return;
    }

    let mut failed = 0usize;
    let mut bytes = 0usize;
    for (path, contents) in files {
        // The same escape check the Android manifest gets. The table is generated at build time,
        // so this only refuses what `build.rs` should never have produced.
        if crate::androidassets::is_safe_asset_path(path) {
            match write_one(paths, path, contents) {
                Ok(()) => bytes += contents.len(),
                Err(error) => {
                    failed += 1;
                    tracing::error!(path, %error, "could not write a carried asset");
                }
            }
        } else {
            failed += 1;
            tracing::error!(path, "refused a carried asset outside the asset directory");
        }
    }
    if failed > 0 {
        // No manifest, so the next start tries again.
        tracing::error!(failed, of = files.len(), "some assets were not written");
        return;
    }
    if let Some(installed) = installed.as_deref() {
        for path in crate::androidassets::departed(installed, &wanted) {
            match fs::remove_file(paths.asset(path)) {
                Ok(()) => tracing::info!(path, "removed an asset this build no longer carries"),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(path, %error, "could not remove a departed asset"),
            }
        }
    }
    if let Err(error) = fs::write(paths.asset(MANIFEST), &wanted) {
        tracing::error!(%error, "wrote the assets but could not record the manifest");
        return;
    }
    tracing::info!(
        files = files.len(),
        bytes,
        dir = %paths.asset_dir.display(),
        "wrote the carried assets"
    );
}

fn write_one(paths: &Paths, path: &str, contents: &[u8]) -> io::Result<()> {
    let target = paths.asset(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target, contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> (km_testkit::Scratch, Paths) {
        let dir = km_testkit::Scratch::new(name);
        let paths = Paths::rooted_at(dir.path());
        (dir, paths)
    }

    #[test]
    fn the_manifest_is_sorted_by_path() {
        let files: &[(&str, &[u8])] = &[("b/x.txt", b"12"), ("a.txt", b"1")];
        assert_eq!(manifest(files), "1\ta.txt\n2\tb/x.txt\n");
    }

    #[test]
    fn files_are_written_and_a_second_start_writes_nothing() {
        let (_dir, paths) = scratch("tizen-assets");
        let files: &[(&str, &[u8])] = &[
            ("fonts/karaoke.ttf", b"font"),
            ("remote-dev/index.html", b"<p>"),
        ];
        unpack(&paths, files);
        assert_eq!(
            fs::read(paths.asset("fonts/karaoke.ttf")).expect("font"),
            b"font"
        );
        assert_eq!(
            fs::read_to_string(paths.asset(MANIFEST)).expect("manifest"),
            manifest(files)
        );

        // A file changed behind the manifest's back is left alone when the manifest still matches.
        fs::write(paths.asset("fonts/karaoke.ttf"), b"edited").expect("edit");
        unpack(&paths, files);
        assert_eq!(
            fs::read(paths.asset("fonts/karaoke.ttf")).expect("font"),
            b"edited"
        );
    }

    #[test]
    fn a_file_the_new_build_does_not_carry_is_removed() {
        let (_dir, paths) = scratch("tizen-assets");
        unpack(&paths, &[("old.txt", b"o"), ("kept.txt", b"k")]);
        fs::write(paths.asset("mine.txt"), b"m").expect("a file somebody put there");
        unpack(&paths, &[("kept.txt", b"kk")]);
        assert!(!paths.asset("old.txt").exists());
        assert!(
            paths.asset("mine.txt").exists(),
            "never in a manifest, so never removed"
        );
        assert_eq!(fs::read(paths.asset("kept.txt")).expect("kept"), b"kk");
    }

    #[test]
    fn an_escaping_path_is_refused_and_no_manifest_is_written() {
        let (_dir, paths) = scratch("tizen-assets");
        unpack(&paths, &[("../settings.json", b"x")]);
        assert!(!paths.asset(MANIFEST).exists());
        assert!(!paths.data_dir.join("settings.json").exists());
    }
}
