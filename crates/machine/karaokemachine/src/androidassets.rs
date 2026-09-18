//! Unpacking the APK's bundled assets into app-private storage.
//!
//! Android assets are not files. They live inside the APK, and only the asset API can read them — but
//! everything that consumes them here wants a **path**: `rustysynth` opens the SoundFont, `SDL_ttf`
//! opens the font, and the `image` crate opens wallpapers. Rewriting all three to take a reader is a
//! large change for no benefit, so the assets are unpacked once into the directory
//! [`crate::settings::Paths::asset_dir`] already points at, and everything downstream carries on
//! working with ordinary files.
//!
//! **SDL does the reading, so this needs no JNI and no `unsafe`.** `SDL_IOFromFile` on Android tries
//! internal storage for a relative path and then falls back to the APK's asset system
//! (`SDL/src/io/SDL_iostream.c`, which calls `Android_JNI_FileOpen` → `AAssetManager_open`). The
//! `sdl3` crate wraps that as [`IOStream`], which implements [`std::io::Read`] — so unpacking is
//! `io::copy` and nothing more.
//!
//! **The manifest is not bureaucracy.** SDL can open an asset by name but has no way to *list* an
//! asset directory, so something has to say what is in there. `tools/port/machine/android/assets.sh` writes
//! `MANIFEST` as `size<TAB>path` lines; sizes are included so that editing a file without adding or
//! removing one still triggers a re-unpack.

use std::path::Path;

/// The unpacking itself, which needs SDL and an APK to read from.
///
/// Split out so that [`Entry`] below — the only part with logic that can be wrong, and the part that
/// refuses a path escaping the asset directory — compiles and is tested on every platform. Gated as a
/// whole it would carry tests that never ran anywhere, and gated per-item the host build trips
/// `dead_code` on every function.
#[cfg(target_os = "android")]
pub use apk::unpack;

#[cfg(target_os = "android")]
mod apk {
    use std::fs;
    use std::io;

    use sdl3::iostream::IOStream;

    use super::Entry;
    use crate::settings::Paths;

    /// The bundled list of what to unpack, relative to the APK's asset root.
    const MANIFEST: &str = "MANIFEST";

    /// Unpacks the bundled assets if they are missing or out of date.
    ///
    /// Reports rather than fails. Every asset has a documented fallback — test tone, system font,
    /// generated gradient — so a machine that cannot unpack is still a machine, and refusing to start
    /// would trade a quiet degradation for no karaoke at all.
    /// Note that `paths.asset(...)` is used here as a **write** target, which is safe only because
    /// [`crate::settings::Paths::overlay_asset_dir`] is always `None` on Android — structurally,
    /// since Android goes through `rooted_at`, and again in `checkout_overlay` itself. Anything that
    /// ever makes the overlay reachable here would turn these into writes into a directory the
    /// machine only means to read.
    pub fn unpack(paths: &Paths) {
        let Some(wanted) = read_asset_to_string(MANIFEST) else {
            // An APK built without `tools/port/machine/android/assets.sh`. Normal during native-only iteration, so
            // this is not a warning.
            tracing::debug!("no asset manifest in the APK; nothing bundled to unpack");
            return;
        };

        let installed = fs::read_to_string(paths.asset(MANIFEST)).ok();
        if installed.as_deref() == Some(wanted.as_str()) {
            tracing::debug!("bundled assets are already unpacked and current");
            return;
        }

        let entries: Vec<Entry> = wanted.lines().filter_map(Entry::parse).collect();
        if entries.is_empty() {
            tracing::warn!("the asset manifest is present but lists nothing usable");
            return;
        }

        tracing::info!(
            files = entries.len(),
            dir = %paths.asset_dir.display(),
            "unpacking bundled assets"
        );

        let mut bytes: u64 = 0;
        let mut failed = 0usize;
        for entry in &entries {
            match unpack_one(entry, paths) {
                Ok(written) => bytes += written,
                Err(error) => {
                    failed += 1;
                    tracing::error!(path = entry.path, %error, "could not unpack a bundled asset");
                }
            }
        }

        if failed > 0 {
            // The manifest is deliberately not written, so the next launch tries again rather than
            // treating a half-unpacked directory as finished.
            tracing::error!(
                failed,
                of = entries.len(),
                "some assets did not unpack; will retry on the next start"
            );
            return;
        }

        // **Adding and overwriting is not enough: app-private storage survives an upgrade, so an
        // asset this build stopped shipping stays on the device for ever unless something removes
        // it.** That is not tidiness. The display *lists* the wallpapers directory rather than
        // reading a manifest, so when a build replaced four gradient PNGs with a zip of seven
        // photographs, every device that had ever run the older build went on drawing eleven
        // wallpapers — the four stale ones included — and the new pack looked broken rather than
        // new. Found on a Google TV Streamer three releases later.
        //
        // **Only paths the previous manifest listed are removed**, which is what makes this safe: a
        // file somebody put in the asset directory by hand was never in a manifest and is left
        // alone. The old manifest goes through `Entry::parse` for the same reason the new one does —
        // it is a file on disk by then, and the escape check is the whole point of parsing it.
        //
        // A removal that fails is logged and not fatal. Withholding the manifest to force a retry
        // is the wrong trade here: the unpack itself succeeded, so the cost would be re-copying
        // thirty megabytes on every launch for ever to chase one file that will not delete.
        if let Some(installed) = installed.as_deref() {
            for path in super::departed(installed, &wanted) {
                match fs::remove_file(paths.asset(path)) {
                    Ok(()) => tracing::info!(path, "removed an asset this build no longer carries"),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => {
                        tracing::warn!(path, %error, "could not remove a departed asset");
                    }
                }
            }
        }

        // Written last, for the same reason: it is the marker that says the directory is complete.
        if let Err(error) = fs::write(paths.asset(MANIFEST), &wanted) {
            tracing::error!(%error, "unpacked the assets but could not record the manifest");
            return;
        }
        tracing::info!(files = entries.len(), bytes, "bundled assets are ready");
    }

    /// Copies one asset out of the APK, creating its directory.
    fn unpack_one(entry: &Entry<'_>, paths: &Paths) -> io::Result<u64> {
        let target = paths.asset(entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut source = IOStream::from_file(entry.path, "rb")
            .map_err(|error| io::Error::other(format!("APK asset: {error}")))?;
        let mut sink = fs::File::create(&target)?;
        let written = io::copy(&mut source, &mut sink)?;

        // Checked because a short read here is silent otherwise, and the failure it produces later is a
        // truncated SoundFont or a font that will not parse — neither of which points back to unpacking.
        if written != entry.size {
            return Err(io::Error::other(format!(
                "expected {} bytes, unpacked {written}",
                entry.size
            )));
        }
        Ok(written)
    }

    /// Reads a whole asset out of the APK as text, or `None` if it is not there.
    fn read_asset_to_string(name: &str) -> Option<String> {
        let mut stream = IOStream::from_file(name, "rb").ok()?;
        let mut text = String::new();
        io::Read::read_to_string(&mut stream, &mut text).ok()?;
        Some(text)
    }
}

/// The paths the last unpack wrote that this one no longer carries.
///
/// Out here beside [`Entry`] rather than inside `apk` for the reason the module header gives: it is
/// logic that can be wrong and has no SDL in it, so it compiles and is tested everywhere.
///
/// Both sides go through [`Entry::parse`], so a malformed or escaping line in either manifest is
/// ignored rather than acted on — and a line that cannot be parsed on the *installed* side simply
/// leaves its file where it is, which is the safe direction to fail in.
fn departed<'a>(installed: &'a str, wanted: &str) -> Vec<&'a str> {
    let kept: std::collections::HashSet<&str> = wanted
        .lines()
        .filter_map(Entry::parse)
        .map(|entry| entry.path)
        .collect();
    installed
        .lines()
        .filter_map(Entry::parse)
        .map(|entry| entry.path)
        .filter(|path| !kept.contains(path))
        .collect()
}

/// One line of the manifest.
struct Entry<'a> {
    size: u64,
    path: &'a str,
}

impl<'a> Entry<'a> {
    /// Parses `size<TAB>path`, ignoring anything malformed.
    fn parse(line: &'a str) -> Option<Self> {
        let (size, path) = line.split_once('\t')?;
        let size = size.trim().parse().ok()?;
        let path = path.trim();
        // A path escaping the asset directory would let a rebuilt APK write anywhere the app can.
        // Nothing generates one, which is exactly why it is worth refusing rather than assuming.
        //
        // The leading-separator check is on the string rather than `Path::is_absolute`, because the
        // manifest is written on a build host and read on a device, and those can disagree about what
        // "absolute" means: on Windows `Path::new("/data/data/…").is_absolute()` is **false**, since
        // an absolute path there needs a drive prefix. `is_absolute` is still worth keeping for the
        // `C:\…` case it does catch.
        if path.is_empty()
            || path.starts_with('/')
            || path.starts_with('\\')
            || path.contains("..")
            || Path::new(path).is_absolute()
        {
            return None;
        }
        Some(Self { size, path })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `Entry::parse` is the only part with logic and no SDL in it, so it is the part worth testing.
    // The unpacking itself needs an APK.

    #[test]
    fn a_manifest_line_is_a_size_and_a_path() {
        let entry = Entry::parse("32319396\tsoundfont/GeneralUser-GS.sf2").expect("parses");
        assert_eq!(entry.size, 32_319_396);
        assert_eq!(entry.path, "soundfont/GeneralUser-GS.sf2");
    }

    #[test]
    fn malformed_lines_are_skipped_rather_than_guessed_at() {
        assert!(Entry::parse("").is_none());
        assert!(Entry::parse("no tab here").is_none());
        assert!(Entry::parse("notanumber\tfonts/x.ttf").is_none());
        assert!(Entry::parse("123\t").is_none(), "empty path");
    }

    #[test]
    fn a_path_escaping_the_asset_directory_is_refused() {
        // Nothing we ship generates these. The check exists because the manifest comes out of an APK,
        // and an APK is a thing somebody else can rebuild.
        assert!(Entry::parse("10\t../settings.json").is_none());
        assert!(Entry::parse("10\tsoundfont/../../settings.json").is_none());
        assert!(
            Entry::parse("10\t/data/data/other/files/x").is_none(),
            "absolute paths must be refused too"
        );
    }

    // The real case, and the one that took three releases to notice: the wallpapers stopped being
    // four loose PNGs and became one zip, and nothing removed the four.
    #[test]
    fn an_asset_that_left_the_manifest_is_reported_as_departed() {
        let installed = "1\twallpapers/01-dusk.png\n\
                         2\twallpapers/02-ember.png\n\
                         3\tsoundfont/GeneralUser-GS.sf2\n";
        let wanted = "9\twallpapers/default-wallpapers.zip\n\
                      3\tsoundfont/GeneralUser-GS.sf2\n";
        assert_eq!(
            departed(installed, wanted),
            vec!["wallpapers/01-dusk.png", "wallpapers/02-ember.png"]
        );
    }

    #[test]
    fn a_file_that_only_changed_size_has_not_departed() {
        // The size is what triggers the re-unpack; it must not also mark the file for deletion, or
        // an edited asset would be removed and then written back on every launch.
        let installed = "1\tsoundfont/GeneralUser-GS.sf2\n";
        let wanted = "2\tsoundfont/GeneralUser-GS.sf2\n";
        assert!(departed(installed, wanted).is_empty());
    }

    #[test]
    fn nothing_departs_when_the_manifests_agree() {
        let manifest = "1\ta.txt\n2\tb/c.txt\n";
        assert!(departed(manifest, manifest).is_empty());
    }

    #[test]
    fn an_unparseable_installed_line_leaves_its_file_alone() {
        // Failing towards "keep the file" rather than "delete something we could not read".
        let installed = "10\t../settings.json\nnonsense\n10\tgone.txt\n";
        assert_eq!(departed(installed, ""), vec!["gone.txt"]);
    }

    #[test]
    fn surrounding_whitespace_does_not_defeat_the_parse() {
        let entry = Entry::parse(" 42 \t fonts/karaoke.ttf ").expect("parses");
        assert_eq!(entry.size, 42);
        assert_eq!(entry.path, "fonts/karaoke.ttf");
    }
}
