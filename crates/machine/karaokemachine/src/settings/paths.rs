//! Where everything lives on this machine, and how that is worked out per platform.
//!
//! `Paths` and the constants naming the folders under it, `WallpaperSource`, and the discovery
//! helpers behind them — the checkout overlay, Android's internal and external storage, and the two
//! tests (`holds_wallpapers`, `is_package`) that decide what a directory is by looking in it.
//!
//! **None of this is a setting**, which is why it is no longer in the file called `settings.rs`.
//! The two halves change for opposite reasons: this one changes when a *platform's* filesystem
//! layout does — a new Android storage rule, a bundle relocated on macOS — and the model beside it
//! changes when a feature gains a knob. They had been sharing a 4,097-line file, of which this was
//! 841 lines that never mention a preference.
//!
//! Re-exported from `crate::settings`, so `settings::Paths` and `settings::APP_NAME` are still what
//! every caller writes.

use super::*;

/// The application's identifier in platform config and data directories.
///
/// Also the stem of the log files `--log-file` writes, so that a folder holding several programs'
/// logs says which is which.
pub const APP_NAME: &str = "karaokemachine";

/// Where wallpapers live — below the asset directory for the shipped set, and below the **data**
/// directory for the owner's own.
///
/// One word for both, because they are the same kind of folder seen from two sides: the one that
/// ships and the one that replaces it. See [`Paths::wallpaper_dir`] for the order they are consulted
/// in, and the `Where the owner's own wallpapers live` decision in `docs/decisions/` for why the second
/// one has to exist at all — `assets/` is root-owned under `/opt`, sealed inside a signed bundle on
/// macOS and re-unpacked from the APK on Android, so on three of four platforms the owner cannot
/// remove a shipped wallpaper even if they want to.
pub const WALLPAPER_SUBDIR: &str = "wallpapers";

/// Where song packages live below the **data** directory.
///
/// Deliberately not below the asset directory, where the wallpapers and the SoundFont are. That tree
/// ships with the build and is read-only where it matters — root-owned under `/opt` on Debian, inside
/// a signed bundle on macOS, unpacked from the APK on Android — whereas packages are the owner's own
/// content, they accumulate, and `km-catalog` reads each one **in place** for as long as it stays
/// installed. They belong beside `library.sqlite`, which is the other thing here that grows with use.
///
/// Resolved by rule and never by a setting: a second way to say where packages are is a second thing
/// that can disagree with the paths the catalog has already stored. The ways to put one somewhere
/// else are better ones — name it in `packages`, install it through the API, or move the whole
/// install with `--data-dir`.
///
pub const PACKAGE_SUBDIR: &str = "packages";

/// Where the owner's own SoundFont banks live below the **data** directory.
///
/// The same argument [`PACKAGE_SUBDIR`] makes, and for a case that is if anything sharper. A bank is
/// the owner's own content, it accumulates, and it is large — 6 MiB to a gigabyte. `assets/` is the
/// wrong home for all three reasons: it ships with the build, it is read-only where it matters (
/// root-owned under `/opt`, sealed in a signed bundle on macOS, re-unpacked from the APK on
/// Android), and everything in it is copied into **every** carrier — so a 206 MiB override left
/// there would be added to the `.deb`, the Windows folder, the bundle and the APK alike.
///
/// That last point is not hypothetical: `tools/dist/check-assets.sh` refuses a staging run where
/// `assets/soundfont/` holds two banks, precisely because the `.deb` globs `assets/soundfont/*.sf2`.
/// This folder is where a second bank is allowed to exist.
pub const SOUNDFONT_SUBDIR: &str = "soundfonts";

/// Where a song uploaded for an audition is staged, below the **data** directory.
///
/// The third folder to make [`PACKAGE_SUBDIR`]'s argument, and the easiest of the three: this holds
/// nothing an owner chose, nothing they should have to find, and nothing worth keeping past the next
/// audition. **Resolved by rule and never by a setting** for the reason that constant gives — a
/// second way to say where it is is a second thing that can disagree with the first — and the
/// setting that does exist, `debug.accept_uploads`, answers *whether* rather than *where*.
///
/// Scratch, not storage. One folder per audition below this one, swept on the way in.
pub const AUDITION_SUBDIR: &str = "auditions";

/// Where a running stream's playlist and segments go, below the data directory.
///
/// **Resolved by rule and never by a setting**, exactly as [`AUDITION_SUBDIR`] is and for the same
/// reason: nothing here is an owner's to keep, find or name. The muxer deletes segments as they age
/// out of the playlist, so the folder is bounded by the window rather than by anything sweeping it.
///
/// It is deliberately inside the data directory rather than a temporary one, so that two machines
/// given two `--data-dir`s stream into two places without being told to.
pub const STREAM_SUBDIR: &str = "stream";

/// Where the bundled font lives below the asset directory.
pub const FONT_SUBPATH: &str = "fonts/karaoke.ttf";

/// Bundled SoundFont names, in order of preference, below the asset directory.
///
/// `GeneralUser-GS.sf2` is what `tools/setup/fetch-assets.sh` installs; the other two are the names an
/// operator dropping in their own bank is most likely to use.
pub const SOUNDFONT_SUBPATHS: &[&str] = &[
    "soundfont/gm.sf2",
    "soundfont/GeneralUser-GS.sf2",
    "soundfont/FluidR3_GM.sf2",
];

/// Where settings, the catalog and the log live.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Directory holding `settings.json`.
    pub config_dir: PathBuf,
    /// Directory holding `library.sqlite`.
    pub data_dir: PathBuf,
    /// Directory holding the bundled SoundFont, font and wallpapers.
    ///
    /// Read-only as far as the machine is concerned, and separate from the two above because it
    /// ships with the build rather than accumulating with use. See [`Paths::discover_asset_dirs`]
    /// for how it is found and why it is not simply `assets`.
    pub asset_dir: PathBuf,
    /// A directory whose files win over [`Paths::asset_dir`]'s, per path. `None` for every
    /// installed build.
    ///
    /// `local/assets/` in a checkout, mirroring `assets/`'s shape. It exists because `assets/` is
    /// the *shipped* tree — every carrier copies it wholesale — so the two things a developer wants
    /// locally, an override SoundFont and a built wallpaper pack, had nowhere to go that a release
    /// would not pick up. `/local/` is gitignored wholesale, so nothing here can be committed by
    /// accident either.
    ///
    /// **Resolved by rule and never by a setting**, exactly as [`PACKAGE_SUBDIR`] is and for the
    /// same reason: a second way to *say* where assets are is a second thing that can disagree with
    /// the first. The three ways to point the machine at a file somewhere else — `audio.soundfont`,
    /// `wallpaper.dir` and `display.font` — are untouched, and each still wins outright over both
    /// directories here.
    ///
    /// See [`checkout_overlay`] for exactly when this is `Some`, and why an installed build can
    /// never reach it.
    pub overlay_asset_dir: Option<PathBuf>,
    /// A second, **public** directory packages are also scanned from. `None` everywhere but Android.
    ///
    /// Android's app-private storage is reachable only through `adb` and `run-as`, which is
    /// a test convenience rather than a product answer — and it is
    /// one an owner with a 16 GB MP3+G library cannot use at all. This is the app's *external* files
    /// directory, which needs no manifest permission on any API level, can be written by a file
    /// manager or a plain `adb push`, and is removed with the app.
    ///
    /// It is a **second rule, not a setting**, which is what keeps the `Where packages live`
    /// decision in `docs/decisions/` intact: nothing in `settings.json` names it, it cannot be pointed
    /// anywhere else, and the catalog goes on storing absolute paths either way — so there is
    /// still no second way to *say* where packages are that could disagree with what was stored.
    pub extra_data_dir: Option<PathBuf>,
    /// Whether [`Self::extra_data_dir`] can be **written**, not merely read.
    ///
    /// Separate from the path being `Some`, and the split is load-bearing. Scanning that folder
    /// needs only to read it, so a volume that is mounted read-only should still contribute the
    /// packages it holds; but [`Self::write_dir`] prefers it for *new* files, and preferring a
    /// folder that cannot be written to would turn every drop into a failure.
    ///
    /// Android reports both facts in one bitmask, and until this existed the machine asked for the
    /// mask, logged it, and then ignored it — harmless while the folder was only ever scanned.
    /// `false` everywhere the folder is `None`.
    pub extra_data_writable: bool,
    /// Further folders packages are scanned from, named by the owner in `settings.package_dirs`.
    ///
    /// **This one *is* a setting, unlike [`Self::extra_data_dir`] above**, and the `Where packages
    /// live` decision is intact for the reason that row already gives: what it refuses is a second
    /// way to say where the *machine's own* packages folder is, because that could disagree with the
    /// absolute paths the catalog has already stored. This adds places to *look*, stores absolute
    /// paths exactly as before, and takes nothing away from the folder in the data directory.
    ///
    /// Filled in by [`crate::machine::Machine::new`] rather than by a constructor here, because a
    /// `Paths` has to exist before there is a settings file to read — it is what names the file.
    pub extra_package_dirs: Vec<PathBuf>,
}

impl Paths {
    /// The platform's directories for this application.
    ///
    /// Falls back to `./karaokemachine-data` when the platform will not name a home directory —
    /// which happens on a stripped-down container, and is not a reason to refuse to run.
    pub fn discover() -> Self {
        #[cfg(target_os = "android")]
        if let Some(dir) = android_internal_storage() {
            // One directory for all three: Android gives an app one private area, and there is no
            // config/data distinction to honor.
            let mut paths = Self::rooted_at(dir);
            // ...and one more that is *public*, so songs can be put there without `adb`. Absent on
            // a device whose external storage is unavailable, which is a degradation and not a
            // fault: the machine then behaves exactly as it did before this existed.
            let (external, writable) = android_external_storage();
            paths.extra_data_dir = external;
            paths.extra_data_writable = writable;
            return paths;
        }

        // Where Android asks SDL, iOS is *told*: the container's directories arrive from Swift
        // before SDL starts. SDL has no `SDL_GetIOSInternalStoragePath` to ask, and `directories`
        // would answer with a macOS path outside the container -- one that resolves and cannot be
        // written to, which is the failure that looks like a bug in the catalog.
        #[cfg(target_os = "ios")]
        if let Some(dirs) = crate::ioscfg::dirs() {
            return Self::in_container(&dirs.support, &dirs.documents);
        }

        let (asset_dir, overlay_asset_dir) = Self::discover_asset_dirs();
        match directories::ProjectDirs::from("", "", APP_NAME) {
            Some(dirs) => Self {
                config_dir: dirs.config_dir().to_path_buf(),
                data_dir: dirs.data_dir().to_path_buf(),
                asset_dir,
                overlay_asset_dir,
                extra_data_dir: None,
                extra_data_writable: false,
                extra_package_dirs: Vec::new(),
            },
            None => {
                let fallback = PathBuf::from("karaokemachine-data");
                tracing::warn!(
                    dir = %fallback.display(),
                    "no platform config directory; keeping settings and the catalog here"
                );
                Self {
                    config_dir: fallback.clone(),
                    data_dir: fallback,
                    asset_dir,
                    overlay_asset_dir,
                    extra_data_dir: None,
                    extra_data_writable: false,
                    extra_package_dirs: Vec::new(),
                }
            }
        }
    }

    /// Where the bundled assets are, on a desktop install.
    ///
    /// `assets/…` on its own is a working-directory-relative path, which is wrong in two ways that
    /// only show up outside a developer's shell: an installed build launched from a menu or a
    /// service manager has whatever working directory its launcher felt like, and on Android it is
    /// `/`. Either way the machine silently loses its SoundFont, its font and its wallpapers.
    ///
    /// So: the directory beside the executable if it has an `assets` folder, then
    /// `../Resources/assets` if the executable is inside a macOS bundle, otherwise the working
    /// directory. The first covers an install laid out as `bin/karaokemachine` + `bin/assets/`; the
    /// last keeps `cargo run` from the repository root working, because `target/debug/` has no
    /// `assets` sibling and the fallback lands on the repository root where `assets/` really is.
    ///
    /// Absolute either way. Partly so `--show-paths` names a directory somebody can go and look at,
    /// and partly so the answer cannot change underneath the machine if anything ever calls
    /// `set_current_dir`.
    ///
    /// Returns the overlay beside it — see [`Paths::overlay_asset_dir`]. **Only the working-directory
    /// branch can carry one.** The two above it are what an installed build takes: a folder beside
    /// the executable, and a signed bundle's `Contents/Resources`. Neither has any business
    /// consulting whatever directory somebody happened to launch from.
    pub(super) fn discover_asset_dirs() -> (PathBuf, Option<PathBuf>) {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let cwd = std::env::current_dir().ok();
        Self::asset_dirs_from(exe_dir.as_deref(), cwd.as_deref())
    }

    /// The two directories above, decided from arguments rather than from the environment.
    ///
    /// Kept apart from reading `current_exe` and `current_dir` so that the branches can be tested at
    /// all — the same split, and for the same reason, as [`Settings::packages_to_install`] being
    /// kept apart from the installing.
    pub(super) fn asset_dirs_from(
        exe_dir: Option<&Path>,
        cwd: Option<&Path>,
    ) -> (PathBuf, Option<PathBuf>) {
        if let Some(dir) = exe_dir.filter(|dir| dir.join("assets").is_dir()) {
            return (dir.join("assets"), None);
        }

        // Inside a macOS application bundle. `Contents/MacOS` is for executables and nothing else --
        // Apple's layout puts everything the app merely *reads* in `Contents/Resources` -- so a
        // bundle cannot use the rule above without being laid out wrongly, and a wrongly laid out
        // bundle is the kind of thing that works until the day something signs it.
        //
        // Checked after the sibling directory rather than before, so a developer running the binary
        // straight out of a bundle they have added an `assets` folder to still gets that one.
        if let Some(dir) = exe_dir.filter(|dir| dir.ends_with("Contents/MacOS")) {
            let resources = dir.join("../Resources/assets");
            if resources.is_dir() {
                // Canonicalised so `--show-paths` prints somewhere a person can go and look, rather
                // than a path with `..` in the middle of it.
                return (resources.canonicalize().unwrap_or(resources), None);
            }
            // A bundle with no `Resources/assets` falls through to the working directory, which is
            // the "developer running the binary straight out of a bundle" case the ordering above
            // was already written for -- so it *can* carry an overlay, deliberately.
        }
        match cwd {
            Some(cwd) => (cwd.join("assets"), checkout_overlay(cwd)),
            // No working directory to resolve against is a strange state, but a relative path still
            // behaves exactly as it did before this function existed.
            None => (PathBuf::from("assets"), None),
        }
    }

    /// Uses one directory for everything. For tests, and for a portable install.
    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.clone(),
            asset_dir: root.join("assets"),
            // ...and no overlay, on exactly the argument the comment below makes. This constructor
            // is what the tests use, and they run with the workspace root as their working
            // directory -- which on a developer's machine has both `assets/` and `local/assets/`.
            // A `rooted_at` that reached out to the cwd would therefore hand
            // `the_bundled_candidates_are_resolved_against_the_asset_directory` a different path
            // than the one it just wrote, failing on a developer's box and passing on CI, and in
            // the passing direction it would mean the test suite opens a 206 MiB bank.
            //
            // Nothing in production wants this constructor's assets any more: `--data-dir` takes
            // [`Paths::data_rooted_at`], which keeps normal asset discovery.
            overlay_asset_dir: None,
            data_dir: root,
            // One directory means one, which is the whole point of this constructor: a test that
            // asked for a scratch tree must not also start scanning a real device's storage.
            extra_data_dir: None,
            extra_data_writable: false,
            extra_package_dirs: Vec::new(),
        }
    }

    /// Settings, catalog and packages under one directory — and the assets wherever they really
    /// are. What `--data-dir` takes.
    ///
    /// **This is the whole difference from [`Paths::rooted_at`].** That constructor moves the asset
    /// directory along with everything else, which is right for a test and for a portable install
    /// and wrong for `--data-dir`: a flag documented as "keep a run out of the real install" would
    /// also take away the SoundFont, the font and the wallpapers, and a scratch run would come up
    /// on a sine test tone over a plain gradient. `crates/machine/karaokemachine/debian/postinst`
    /// avoids `--data-dir` and describes that trap; this constructor is what closes it.
    ///
    /// Assets — and the checkout overlay with them — come from [`Paths::discover_asset_dirs`], the
    /// same answer a run with no `--data-dir` gets. There is no separate opt-in for the overlay any
    /// more: a developer running `--data-dir ./local/km-<name>` from a checkout is in a checkout,
    /// which is precisely what that function already decides.
    ///
    /// `extra_data_dir` stays `None` on [`Paths::rooted_at`]'s reasoning rather than
    /// [`Paths::discover`]'s: somebody who named a packages directory did not also ask for Android's
    /// public one.
    pub fn data_rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let (asset_dir, overlay_asset_dir) = Self::discover_asset_dirs();
        Self {
            config_dir: root.clone(),
            data_dir: root,
            asset_dir,
            overlay_asset_dir,
            extra_data_dir: None,
            extra_data_writable: false,
            extra_package_dirs: Vec::new(),
        }
    }

    /// Settings, catalog and wallpapers in the application's private directory; packages in the one
    /// a person can see. What an iOS container gives us.
    ///
    /// **The difference from [`Paths::data_rooted_at`] is the second directory**, and it is the
    /// whole reason this is a constructor rather than a call to that one. `support` is
    /// `Library/Application Support/…`, which is backed up and invisible; `documents` is `Documents/`,
    /// which `UIFileSharingEnabled` puts in the Files app, so `Documents/packages` is where a
    /// `.kmpkg` copied over USB lands. `packages_dirs()` already reads `extra_data_dir` as a second
    /// place to look, so naming it here is all an iOS build needs.
    ///
    /// **`documents` itself, not `documents/packages`.** The subdirectory is [`packages_dirs`]'s to
    /// append, exactly as it is on Android, where `extra_data_dir` is the app's public files
    /// directory and not a packages folder inside it. Passing the deeper path here reaches the
    /// device as `Documents/packages/packages`: a folder nothing writes to, beside the one the
    /// Files app shows, with no error to say which is which.
    ///
    /// [`packages_dirs`]: Paths::packages_dirs
    ///
    /// Writable, unlike Android's, where external storage can be absent or read-only: a container's
    /// own `Documents` is neither.
    ///
    /// Assets come from [`Paths::discover_asset_dirs`], which takes `<exe dir>/assets` when it is
    /// there. An `.app` keeps its executable at the bundle root, so a folder reference named
    /// `assets` is found with no special case here.
    #[must_use]
    pub fn in_container(support: &Path, documents: &Path) -> Self {
        let (asset_dir, overlay_asset_dir) = Self::discover_asset_dirs();
        Self {
            config_dir: support.to_path_buf(),
            data_dir: support.to_path_buf(),
            asset_dir,
            overlay_asset_dir,
            extra_data_dir: Some(documents.to_path_buf()),
            extra_data_writable: true,
            extra_package_dirs: Vec::new(),
        }
    }

    /// The settings file.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    /// The catalog database.
    pub fn library_file(&self) -> PathBuf {
        self.data_dir.join("library.sqlite")
    }

    /// The folder `--log-file` writes this run's log into.
    ///
    /// **Deliberately not made by [`Paths::create`]**, unlike the packages and wallpapers folders
    /// beside it. Those are invitations — a place the owner is meant to put something, and one that
    /// is no use if it only appears once they have worked out where it would be. This is an output:
    /// an empty `logs/` says nothing to anybody, and the run that wants one makes it.
    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join(km_logfile::SUBDIR)
    }

    /// The folder song packages are dropped into. See [`PACKAGE_SUBDIR`].
    ///
    /// **Still "the" packages folder, singular**, and deliberately not replaced by
    /// [`Paths::packages_dirs`]: it is the one [`Paths::create`] makes, the one `--show-paths` calls
    /// the answer to *where do I put my songs?*, and on every platform but Android it is the only
    /// one there is.
    pub fn packages_dir(&self) -> PathBuf {
        self.data_dir.join(PACKAGE_SUBDIR)
    }

    /// The folder the owner drops their own wallpapers into. See [`WALLPAPER_SUBDIR`].
    ///
    /// Beside [`Paths::packages_dir`] and made by [`Paths::create`] for the same reason: it is a
    /// place somebody is *meant* to find, and a folder that appears only once they have worked out
    /// where it would be is no use to them.
    ///
    /// This is the folder's *location*, not the answer to which wallpapers are shown — that is
    /// [`Paths::wallpaper_dir`], because an empty one falls through to the shipped set.
    pub fn wallpapers_dir(&self) -> PathBuf {
        self.data_dir.join(WALLPAPER_SUBDIR)
    }

    /// The folder the owner's own SoundFont banks live in. See [`SOUNDFONT_SUBDIR`].
    ///
    /// Beside [`Paths::packages_dir`] and [`Paths::wallpapers_dir`], made by [`Paths::create`] on
    /// the same argument: it is a place somebody is *meant* to find, and a folder that appears only
    /// once they have worked out where it would be is no use to them.
    ///
    /// **Unlike the wallpapers folder, empty here means exactly what it says.** An empty wallpapers
    /// folder cannot mean "show these", so [`Paths::wallpaper_dir`] has to test contents rather than
    /// existence; a bank has no such fallback to be confused with. Empty means the machine plays the
    /// bundled bank, which is what it does with no folder at all.
    pub fn soundfonts_dir(&self) -> PathBuf {
        self.data_dir.join(SOUNDFONT_SUBDIR)
    }

    /// Where a running stream writes, and where the API serves it from.
    ///
    /// **Not created here**, unlike the three folders [`Paths::create`] makes: a directory that
    /// exists is what the API serves, so making one on every start would put an empty playlist
    /// folder on every machine that never streams. The encoder creates it when it opens.
    #[must_use]
    pub fn stream_dir(&self) -> PathBuf {
        self.data_dir.join(STREAM_SUBDIR)
    }

    /// Every folder banks are scanned from, in the order they are scanned.
    ///
    /// One entry everywhere except Android, where the public directory described on
    /// [`Paths::extra_data_dir`] is a second — the same arrangement [`Paths::packages_dirs`] makes,
    /// for the same reason and with the same collision rule: **the private folder is scanned first**,
    /// so a bank dropped onto shared storage cannot displace one the machine downloaded itself.
    ///
    /// **Without this the two halves of the SoundFont feature disagree on Android.** A bank fetched
    /// from the Setup tab lands in [`Paths::soundfonts_dir`], which is under the app-*private*
    /// directory; a bank copied onto the device by a file manager, a USB stick or `adb push` cannot
    /// go there at all without `run-as`. So the one folder somebody can actually put fifty banks in
    /// for a listening test was the one folder nothing looked at.
    pub fn soundfonts_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.soundfonts_dir()];
        if let Some(extra) = &self.extra_data_dir {
            dirs.push(extra.join(SOUNDFONT_SUBDIR));
        }
        dirs
    }

    /// The folder wallpapers are actually read from, and which rule produced it.
    ///
    /// Three candidates, first match wins — the owner's own folder, the checkout overlay, then the
    /// shipped tree. `wallpaper.dir` in settings beats all three and is applied by
    /// [`WallpaperSettings::to_config`] before this is ever called.
    ///
    /// **A candidate has to *hold wallpapers*, not merely exist**, and that is load-bearing rather
    /// than fastidious. [`Paths::create`] makes the owner's folder empty on first start so that it
    /// can be found, and [`km_display::Playlist`] holds exactly one directory with no merging — so
    /// an existence test would replace the shipped set with nothing the moment the folder was
    /// created. A black screen, with nothing in any log to explain it.
    ///
    /// The same trap was latent in the checkout overlay, where [`Paths::asset`]'s `exists()` rule
    /// means an empty `local/assets/wallpapers/` blanks the wallpapers today. It is fixed here for
    /// both, which is why this does not simply call [`Paths::asset`]: that function answers "which
    /// file", and its `exists()` is right for one; this answers "which folder has pictures in it",
    /// and only contents can say.
    ///
    /// **The test is a real scan**, so it cannot disagree with what the machine will go on to show —
    /// a folder of extensions nothing decodes, or a zip holding no images, is empty to both. It costs
    /// one directory read plus each archive's central directory, and `display.rs` then scans the
    /// winner again.
    ///
    /// **That is once per wallpaper cycle rather than once at startup.** The choice has to be
    /// re-made, because it is made *by contents* and the contents change: on every machine before
    /// its first picture the owner's folder is empty on purpose, so a choice frozen at startup
    /// means dropping an image in scans a folder that has already lost the argument. It must be able to move back, too — `Where the owner's own
    /// wallpapers live` promises that emptying the folder returns the shipped set. Two scans of a
    /// handful of entries every thirty seconds on an idle machine is not worth a cleverer
    /// arrangement; having `holds_wallpapers` hand back the `Playlist` it already built would be
    /// the one to reach for if it ever were.
    pub fn wallpaper_dir(&self) -> (PathBuf, WallpaperSource) {
        let owner = self.wallpapers_dir();
        if holds_wallpapers(&owner) {
            return (owner, WallpaperSource::Owner);
        }
        if let Some(overlay) = &self.overlay_asset_dir {
            let candidate = overlay.join(WALLPAPER_SUBDIR);
            if holds_wallpapers(&candidate) {
                return (candidate, WallpaperSource::Overlay);
            }
        }
        (
            self.asset_dir.join(WALLPAPER_SUBDIR),
            WallpaperSource::Bundled,
        )
    }

    /// Every folder packages are scanned from, in the order they are scanned.
    ///
    /// One entry everywhere except Android, where the public directory described on
    /// [`Paths::extra_data_dir`] is a second.
    ///
    /// **The private folder is scanned first, and the order is load-bearing.**
    /// [`crate::machine::startup_plan`] resolves a collision in favor of whatever was offered
    /// first, so scanning the public folder first would let a package dropped onto shared storage
    /// displace one the machine already has — silently, and differently depending on what happened
    /// to be there. The owner's own install wins.
    pub fn packages_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.packages_dir()];
        if let Some(extra) = &self.extra_data_dir {
            dirs.push(extra.join(PACKAGE_SUBDIR));
        }
        // The owner's own, last: the machine's folder stays the first scanned, so which of two
        // packages wanting one bank keeps it does not change for anybody who sets none of these.
        dirs.extend(self.extra_package_dirs.iter().cloned());
        dirs
    }

    /// The folder a content file the machine has been *handed* should be written into.
    ///
    /// **Not the same question as [`Paths::packages_dirs`] or [`Paths::soundfonts_dirs`]**, and the
    /// two now answer differently on Android. Those say *where do I look*, in a precedence order
    /// that is load-bearing; this says *where do I put it*, and the answer is by **room**.
    ///
    /// Android's `data_dir` is app-private **internal** storage, which is the smaller volume on
    /// most devices — and a package reaches tens of gigabytes. So the external files directory is
    /// preferred whenever it is there and the platform says it can be written to. Everywhere else
    /// there is no second candidate and this is the private folder, exactly as before.
    ///
    /// **The scan order deliberately does not follow.** The private folder stays *first* in
    /// `packages_dirs`, for the reason that method gives: nothing dropped onto shared storage may
    /// displace what the machine already installed. So a copied-in package lands in the folder that
    /// is scanned *second*, which is correct rather than merely tolerable — deduplication is by
    /// package id, so a new package has nothing to collide with, and if the owner later puts the
    /// same package in the private folder by hand the private copy wins, exactly as that rule
    /// intends.
    ///
    /// Never [`Paths::asset_dir`], and that is an invariant rather than a preference: the asset
    /// tree ships with the build and is read-only where it counts — root-owned under `/opt` on
    /// Debian, inside a signed bundle on macOS, unpacked from the APK on Android.
    fn write_dir(&self, subdir: &str) -> PathBuf {
        if let Some(extra) = &self.extra_data_dir
            && self.extra_data_writable
        {
            return extra.join(subdir);
        }
        self.data_dir.join(subdir)
    }

    /// Where a package handed to the machine is copied. See [`Paths::write_dir`].
    pub fn packages_write_dir(&self) -> PathBuf {
        self.write_dir(PACKAGE_SUBDIR)
    }

    /// Where a SoundFont bank handed to the machine is installed. See [`Paths::write_dir`].
    pub fn soundfonts_write_dir(&self) -> PathBuf {
        self.write_dir(SOUNDFONT_SUBDIR)
    }

    /// Whether a file is inside a folder this machine owns and may therefore delete from.
    ///
    /// **The guard on every destructive path**, and structural rather than a check somebody has to
    /// remember at each call site. What it excludes is [`Paths::asset_dir`] — the tree that ships
    /// with the build, which on Android is unpacked from the APK — and anywhere else that is
    /// neither scanned for content nor written to by the machine.
    ///
    /// A `debug.` entry pointing *inside* one of these folders still passes here, because this
    /// answers "is this the machine's territory" rather than "is this the machine's file". Whoever
    /// deletes has to ask both.
    pub fn is_mine_to_delete(&self, path: &Path) -> bool {
        let target = tidy(path);
        self.packages_dirs()
            .into_iter()
            .chain(self.soundfonts_dirs())
            .chain([self.wallpapers_dir()])
            .any(|dir| target.starts_with(tidy(&dir)))
    }

    /// A bundled asset, by its path below the asset directory.
    pub fn asset(&self, relative: impl AsRef<Path>) -> PathBuf {
        let relative = relative.as_ref();
        // Per path, not per tree: an overlay holding only a SoundFont leaves the bundled font and
        // the bundled wallpapers exactly where they were.
        //
        // `exists` rather than `is_file`, because `wallpapers` is asked for as a directory and
        // `soundfont/gm.sf2` as a file, and one rule that covers both is easier to hold in the head
        // than two that nearly agree. The consequence for the directory case is worth knowing: a
        // `local/assets/wallpapers/` **replaces** the bundled folder rather than adding to it, so
        // the four committed gradients are not shown while one is there.
        //
        // This touches the filesystem, which `asset()` did not used to. Every caller resolves once
        // at startup -- three SoundFont candidates, the font, the wallpaper folder, the dev remote,
        // `--show-paths` -- so it is single-digit `stat` calls per run.
        if let Some(overlay) = &self.overlay_asset_dir {
            let candidate = overlay.join(relative);
            if candidate.exists() {
                return candidate;
            }
        }
        self.asset_dir.join(relative)
    }

    /// Creates both writable directories, and the packages, wallpapers and SoundFont folders inside
    /// the second.
    ///
    /// Not the asset directory: it ships with the build, and creating an empty one would turn "no
    /// assets installed" into a directory that looks deliberate.
    ///
    /// The packages folder is the exact opposite case, which is why it is made here and the asset
    /// directory is not. It is the place an owner is *meant* to find and drop a `.kmpkg` into, and a
    /// folder that only appears once somebody has already worked out where it would be is no use to
    /// them. Empty means "no songs yet", which is true and is what the machine says on screen.
    ///
    /// The wallpapers folder is there on the same argument, and it is the one that shows what the
    /// argument costs: an empty directory *is* deliberate here — it is an invitation — so emptiness
    /// cannot be allowed to mean "show these". [`Paths::wallpaper_dir`] therefore tests contents
    /// rather than existence, and this line is why it has to.
    pub fn create(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.packages_dir())?;
        std::fs::create_dir_all(self.wallpapers_dir())?;
        std::fs::create_dir_all(self.soundfonts_dir())?;

        // The public one is made too, and by exactly the argument the doc comment above gives for
        // the private one: it is the place an owner is *meant* to find, and a folder that appears
        // only once somebody has worked out where it would be is no use to them. On Android it is
        // the one they can actually reach with a file manager.
        //
        // **Best-effort, unlike the three above.** External storage can be unmounted or read-only,
        // and that is not a reason to refuse to start a machine whose own private directory is
        // perfectly fine.
        if let Some(extra) = &self.extra_data_dir {
            for (subdir, what) in [
                (PACKAGE_SUBDIR, "packages"),
                (SOUNDFONT_SUBDIR, "SoundFont"),
            ] {
                let dir = extra.join(subdir);
                if let Err(error) = std::fs::create_dir_all(&dir) {
                    tracing::warn!(
                        dir = %dir.display(),
                        %error,
                        "could not make the shared {what} folder; only the private one will be scanned"
                    );
                }
            }
        }
        Ok(())
    }
}

/// Which of [`Paths::wallpaper_dir`]'s three rules produced the folder in use.
///
/// Exists so `--show-paths` can say *why* it is showing what it is showing, which is the fastest
/// answer to "I dropped my pictures in and nothing changed" — the usual cause being that they went
/// somewhere the machine was never going to look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperSource {
    /// `wallpaper.dir` named it, which beats all three rules below.
    ///
    /// A variant of its own rather than reporting `None` alongside a folder, because `null` meaning
    /// "a setting named it" would be `null` meaning two things — and `--show-paths` already renders
    /// this case as its own line.
    Setting,
    /// The owner's own folder in the data directory. See [`Paths::wallpapers_dir`].
    Owner,
    /// `local/assets/wallpapers/` in a checkout. See [`Paths::overlay_asset_dir`].
    Overlay,
    /// The set that shipped with the build.
    Bundled,
}

impl WallpaperSource {
    /// A few words for `--show-paths`, in that command's voice.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Setting => "named by wallpaper.dir, which beats every rule",
            Self::Owner => "your own — the folder below is what is shown",
            Self::Overlay => "the checkout's local overlay",
            Self::Bundled => "the set that shipped with this build",
        }
    }
}

/// Whether a directory holds anything the display would actually put on screen.
///
/// **A real scan rather than an extension test**, so this cannot disagree with what
/// [`km_display::Playlist`] goes on to find: a folder of files nothing decodes, and a zip holding no
/// images, are both empty here exactly as they would be there. That is the whole reason it is written
/// this way — the alternative is a second copy of two extension lists, drifting.
///
/// A missing directory is empty rather than an error, which is what lets the caller be a plain chain
/// of first-match-wins.
///
/// **`&[]` for the extras, and that is load-bearing.** `debug.wallpapers`
/// names files that are layered *on top of* whichever folder wins, so counting them here would let
/// one debug entry make an empty owner folder "hold wallpapers" — which would suppress the bundled
/// set and turn an additive setting into a replacing one. That is precisely the black-screen trap
/// [`Paths::wallpaper_dir`] describes: an empty owner folder is created on first start so it can be
/// found, and anything that makes emptiness look like contents blanks the screen with nothing in any
/// log.
fn holds_wallpapers(dir: &Path) -> bool {
    !km_display::Playlist::scan(dir, &[]).is_empty()
}

/// An absolute path fit to write into a file a person will read.
///
/// `canonicalize` on Windows returns the extended-length form, `\\?\C:\…`. It is a valid path and
/// every API accepts it, but `settings.json` is hand-edited, and a path nobody recognizes invites
/// somebody to "fix" it into something that no longer matches. Stripped back to the plain form.
pub(crate) fn tidy(path: &Path) -> PathBuf {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = canonical.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => canonical,
    }
}

/// The packages sitting in `dir`, in the order they should be installed.
///
/// The decision half of the packages folder, kept apart from the installing so that it can be tested
/// against a real directory without an audio device, a catalog or a machine — the same split
/// `androidassets` makes between [`crate::androidassets`]'s path checking and the unpacking that
/// needs an APK.
///
/// Creates the folder when it is missing, and says so rather than failing when it cannot: a machine
/// with nowhere to drop packages is still a machine, and every other route to installing one still
/// works.
///
/// **Sorted by file name, and that sort carries the whole stability guarantee on its own.** Two
/// packages claiming one song number is not a collision (see `Number collisions are impossible, not
/// refused`), so install order does not decide what the catalog holds. What it does decide is which
/// of two packages wanting one bank keeps it and which takes the next, so an unsorted scan gives the
/// same folder different numbers on two starts — a printed book going stale for no reason anybody
/// could see. Nothing else holds this up. Paths go through [`tidy`], because the spelling the
/// catalog stored need not match the one a directory walk produces.
///
/// **This answers *what is in the folder*, and nothing else.** Deciding what to skip is the caller's
/// job now, because the only honest answer is by package **id** rather than by path — the same file
/// can be in two folders under two names — and that needs a manifest opened, which is exactly the
/// work this function exists to stay out of.
pub(crate) fn packages_to_install(dir: &Path) -> Vec<PathBuf> {
    if let Err(error) = std::fs::create_dir_all(dir) {
        tracing::warn!(
            dir = %dir.display(),
            %error,
            "could not make the packages folder; nothing will be picked up from it"
        );
        return Vec::new();
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(dir = %dir.display(), %error, "could not read the packages folder");
            return Vec::new();
        }
    };

    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_package(path))
        .map(|path| tidy(&path))
        .collect();

    // By file name rather than by full path: they all share a parent, so the parent contributes
    // nothing but would drag platform path separators into the ordering.
    found.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    found.dedup();
    found
}

/// Whether a path names a package, by extension and ignoring case.
///
/// Case-insensitive because the folder is somewhere a person copies files into, and Windows will
/// hand back whatever case the copy had.
fn is_package(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("kmpkg"))
}

/// `<dir>/local/assets`, when `dir` is a checkout of this repository.
///
/// A checkout is recognized by **two** directories, both of which must be there. `assets/` is
/// committed and is what the overlay overlays; `local/assets/` is covered by `.gitignore`'s
/// wholesale `/local/` and therefore exists only where somebody deliberately made it.
///
/// Requiring `assets/` too is what keeps this an *overlay* rather than a second asset source: it may
/// only ever supplement a base that is really there, which is what keeps `--show-paths`' "does not
/// exist — run `tools/setup/fetch-assets.sh`" note truthful, and stops a broken checkout from being
/// silently half-repaired by a stray directory somewhere else.
///
/// **No `.git` check, deliberately.** In a `git worktree` `.git` is a *file*, not a directory, so an
/// `is_dir` test would disable the overlay in exactly the place `tools/dev/worktree.sh` goes to the
/// trouble of seeding it.
///
/// Never on Android. That holds structurally — Android's [`Paths::discover`] goes through
/// [`Paths::rooted_at`], which never enables an overlay — and again here, so that the property
/// survives somebody rearranging `discover` later. Android's working directory is `/`.
pub(super) fn checkout_overlay(dir: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "android") {
        return None;
    }
    let overlay = dir.join("local").join("assets");
    (overlay.is_dir() && dir.join("assets").is_dir()).then_some(overlay)
}

/// The app's private directory on Android, as SDL reports it.
///
/// Necessary because `directories` has no Android module — it ships `lin`, `mac`, `win` and `wasm`,
/// so on Android it takes the Linux XDG path and depends on `$HOME`. Unset, it returns `None`, the
/// fallback below lands on a relative directory, and the working directory on Android is `/`, which
/// is not writable. The machine would fail to create its own settings file.
///
/// SDL is asked rather than JNI: `SDL_GetAndroidInternalStoragePath` is exactly this value, and by
/// the time `SDL_main` runs the activity has already set up everything it needs. Going through JNI
/// directly would mean the same answer and a great deal more code.
#[cfg(target_os = "android")]
#[expect(
    unsafe_code,
    reason = "one FFI call to SDL for a borrowed C string; sdl3-rs does not wrap this one"
)]
fn android_internal_storage() -> Option<PathBuf> {
    // SAFETY: SDL owns the returned string and keeps it alive for the process; it must not be freed.
    // A null return means SDL could not determine the path, which is checked here.
    let raw = unsafe { sdl3::sys::system::SDL_GetAndroidInternalStoragePath() };
    if raw.is_null() {
        tracing::error!("SDL could not report the app's private directory");
        return None;
    }
    // SAFETY: non-null and NUL-terminated by SDL's contract.
    let path = unsafe { std::ffi::CStr::from_ptr(raw) };
    match path.to_str() {
        Ok(text) => Some(PathBuf::from(text)),
        Err(error) => {
            tracing::error!(%error, "the app's private directory is not valid UTF-8");
            None
        }
    }
}

/// The app's **public** per-app directory on Android, as SDL reports it.
///
/// `SDL_GetAndroidExternalStoragePath` wraps `Context.getExternalFilesDir(null)`, which is
/// `/storage/emulated/0/Android/data/<package>/files` on a normal device. Three properties make it
/// the right answer here, and each rules out an alternative:
///
/// * **It needs no permission, on any API level.** `MANEGE_EXTERNAL_STORAGE` would — an "All files
///   access" prompt, which on a television is a hostile thing to ask for and on some builds cannot
///   be granted at all.
/// * **It is an ordinary path, so it needs no picker.** The Storage Access Framework hands back a
///   content URI, which every path in this application would have to learn about; and a picker is
///   the wrong shape for an appliance that starts itself under a television.
/// * **Anything can write to it** — a file manager, a USB copy, or `adb push` with no `run-as` —
///   which is the entire point, since the private directory can be reached by none of those.
///
/// `None` is an ordinary answer, not an error: external storage can be unmounted, and a device may
/// refuse the directory. The machine then scans one folder instead of two.
///
/// **The state's write bit is acted on, and returned beside the path**, because the two answer
/// different questions. `getExternalFilesDir` succeeding is what decides whether the folder is
/// *scanned*; the write bit is what decides whether it is where a handed-in file gets *put* — see
/// [`Paths::write_dir`]. Read-only shared storage should still contribute the packages it holds,
/// so the path is returned either way and only the flag goes false.
#[cfg(target_os = "android")]
#[expect(
    unsafe_code,
    reason = "two FFI calls to SDL for a borrowed C string and an enum; sdl3-rs wraps neither"
)]
fn android_external_storage() -> (Option<PathBuf>, bool) {
    // SAFETY: no arguments, no pointers; returns a bitmask SDL owns entirely.
    let state = unsafe { sdl3::sys::system::SDL_GetAndroidExternalStorageState() };
    let writable = state & sdl3::sys::system::SDL_ANDROID_EXTERNAL_STORAGE_WRITE != 0;
    tracing::info!(state, writable, "android external storage state");

    // SAFETY: SDL owns the returned string and keeps it alive for the process; it must not be
    // freed. A null return means SDL could not determine the path, which is checked here.
    let raw = unsafe { sdl3::sys::system::SDL_GetAndroidExternalStoragePath() };
    if raw.is_null() {
        tracing::info!(
            "no external files directory; packages can only be put in the private folder"
        );
        return (None, false);
    }
    // SAFETY: non-null and NUL-terminated by SDL's contract.
    let path = unsafe { std::ffi::CStr::from_ptr(raw) };
    match path.to_str() {
        Ok(text) => (Some(PathBuf::from(text)), writable),
        Err(error) => {
            tracing::error!(%error, "the app's external directory is not valid UTF-8");
            (None, false)
        }
    }
}
