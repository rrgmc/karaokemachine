//! Installing a package that was dragged onto the window.
//!
//! **Why this exists.** The other ways a `.kmpkg` reaches the machine both assume somebody who
//! already knows where things are: a `POST /api/v1/packages` naming a path, or a file copied into
//! the packages folder that `--show-paths` has to tell you about first. The machine has a window on
//! a desktop, and a file manager is the tool the owner already has open when they have just built a
//! package; dropping it on the window is the shortest route from *I have a package* to *the songs
//! are there*, and it needs no restart.
//!
//! Three things about the shape of this module, each of which the alternative gets wrong.
//!
//! **The work is on a thread of its own.** `Catalog::install` holds the library's mutex for the
//! whole transaction, and indexing a four-thousand-song package into SQLite and rebuilding the
//! search index takes seconds — the API route runs it under `spawn_blocking` for exactly this
//! reason. Run inline in the event loop it would freeze the picture, which on a machine that may be
//! playing a song at the time is not a cosmetic problem.
//!
//! **The file is copied into the packages folder first, and that rule is now shared.** [`place`] is
//! called by the API route too, so there is one home for the naming and copying decisions rather
//! than two that can drift apart. Installing where the file lies is the alternative, and it is not
//! on offer: the folders are the truth, so a package left in a downloads folder is not installed at
//! all. Copying in is simply what *taking* a file means. Where the copy goes is [`crate::settings::Paths::packages_write_dir`], which on Android is
//! the external folder rather than the private one, because a package can be tens of gigabytes.
//!
//! **A package is stored under a name the manifest decides, not under the one it arrived as.**
//! `<name>-<id>.kmpkg`, so a folder can be read at a glance and a rebuilt package lands on top of
//! the one it replaces however the sender happened to name the file. The id in the name is what
//! makes that true: it is what an install is keyed on, so two files carrying it are one package and
//! two files without it never are.
//!
//! **Nothing is ever overwritten with something else.** A file already at the derived name is
//! opened, and it is taken only when it holds the same package. Anything else — a file that will
//! not open, most of all — gets stepped over to `-2`, because destroying the owner's only copy of a
//! package is precisely the mistake the uninstall route was designed not to make.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;

use km_api::machine::Catalog as _;

use crate::machine::{Machine, reason_without_path};
use crate::settings::Paths;

/// The most files one drop may hand over before the rest are refused.
///
/// SDL delivers one event per file, so a folder's worth of packages dragged in one gesture arrives
/// as a burst. The queue is unbounded and the worker is sequential, so the only real cost of a large
/// burst is a long silence; the cap exists so that a mis-drag of a whole music library says so
/// rather than appearing to hang for an hour.
const MAX_QUEUED: usize = 32;

/// How far the `-2`, `-3` suffixes go before a drop is refused for want of a name.
///
/// **Nearly unreachable, and kept for the case that is left.** A derived name carries the package's
/// id, so two *different* packages cannot ask for one file name any more. What can still sit at the
/// name is a file that will not open — which the startup scan already complains about, and which
/// taking the name of would both hide the complaint and destroy the file. Reaching thirty-two of
/// those is a folder somebody should look at rather than a case to handle silently.
const MAX_NAME_ATTEMPTS: u32 = 32;

/// What became of a dropped file.
///
/// Rendered by the display as a [`km_display::Flash`]; the strings are written here rather than
/// there because they name packages and songs, which `km-display` knows nothing about — the same
/// seam `SongInfo::language` and the standing package notice both use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropStatus {
    /// Something is being installed, and nothing has gone wrong yet.
    Working(String),
    /// It went in.
    Done(String),
    /// It did not.
    Failed(String),
}

/// Something for the worker to do.
///
/// Both jobs hold the library's mutex for seconds, so both belong off the display thread — and
/// putting them on **one** queue rather than two is what stops a rescan and a drop running at the
/// same time and each waiting on the other's transaction.
enum Job {
    /// A file somebody dropped on the window.
    Install(PathBuf),
    /// Read the packages folders again. `Ctrl+F10`.
    Rescan,
}

/// What a rescan has to say for itself in one line across a television.
///
/// Reads as a sentence rather than a report, because it is shown for a few seconds from across a
/// room. Deferred removals are named as *waiting* rather than omitted: a rescan that found a package
/// gone and could not act must not look like one that found nothing.
fn rescan_summary(report: &km_api::machine::RescanReport) -> String {
    let mut parts = vec![match report.installed {
        1 => "1 package".to_owned(),
        n => format!("{n} packages"),
    }];
    if !report.removed.is_empty() {
        parts.push(format!("{} removed", report.removed.len()));
    }
    if !report.deferred.is_empty() {
        parts.push(format!(
            "{} waiting for the queue to empty",
            report.deferred.len()
        ));
    }
    if report.problems > 0 {
        parts.push(match report.problems {
            1 => "1 problem".to_owned(),
            n => format!("{n} problems"),
        });
    }
    parts.join(" \u{b7} ")
}

/// Installs packages dropped on the window, off the display thread.
///
/// Owns one worker for the life of the window. Dropping this closes the channel, which is what ends
/// the worker: it finishes the package in hand and then returns, so a machine closed mid-install
/// does not leave a half-copied file behind.
pub struct DropInstaller {
    to_worker: Sender<Job>,
    from_worker: Receiver<DropStatus>,
    /// How many paths have been handed over and not yet reported on.
    ///
    /// Kept here rather than asked of the channel because a `Sender` cannot say how much is
    /// outstanding, and the cap is about what the owner just did rather than about memory.
    outstanding: usize,
}

impl DropInstaller {
    /// Starts the worker.
    pub fn spawn(machine: Arc<Machine>) -> Self {
        let (to_worker, work) = channel::<Job>();
        let (report, from_worker) = channel::<DropStatus>();

        thread::Builder::new()
            .name("km-package-install".to_owned())
            .spawn(move || {
                // Ends when the sender is dropped, which is when the window closes.
                for job in work {
                    // **Said before the work, not after.** Both jobs take seconds, and the whole
                    // point of this message is that something is on screen while they do.
                    let working = match &job {
                        Job::Install(path) => format!("installing {}…", file_name(path)),
                        Job::Rescan => "reading the packages folders…".to_owned(),
                    };
                    if report.send(DropStatus::Working(working)).is_err() {
                        break;
                    }

                    let status = match job {
                        Job::Install(path) => {
                            let name = file_name(&path);
                            match install(&machine, &path) {
                                Ok(message) => DropStatus::Done(message),
                                Err(reason) => {
                                    tracing::warn!(path = %path.display(), reason, "a dropped file was not installed");
                                    DropStatus::Failed(format!(
                                        "\"{name}\" was not installed: {reason}"
                                    ))
                                }
                            }
                        }
                        Job::Rescan => DropStatus::Done(rescan_summary(&machine.rescan_now())),
                    };
                    if report.send(status).is_err() {
                        break;
                    }
                }
            })
            // A machine that cannot start a thread has worse problems than drag-and-drop, but it
            // must not fail to open a window over one: the caller treats this as absent.
            .ok();

        Self {
            to_worker,
            from_worker,
            outstanding: 0,
        }
    }

    /// Hands a dropped path to the worker.
    ///
    /// Returns the message to show if the drop was refused outright — a file that is not a package,
    /// or too many at once. Both are answered here rather than on the worker so that the refusal is
    /// immediate: a `.mp3` dropped on the window should say so at once, not after whatever is ahead
    /// of it in the queue.
    pub fn submit(&mut self, path: PathBuf) -> Option<DropStatus> {
        if !is_package(&path) {
            let name = file_name(&path);
            return Some(DropStatus::Failed(format!(
                "\"{name}\" is not a song package: drop a .kmpkg file"
            )));
        }
        if self.outstanding >= MAX_QUEUED {
            return Some(DropStatus::Failed(format!(
                "too many at once: {MAX_QUEUED} packages are already going in"
            )));
        }
        self.hand_over(Job::Install(path))
    }

    /// Asks the worker to read the packages folders again. `Ctrl+F10`.
    ///
    /// Capped alongside drops on the same counter, because it is the same worker and the same
    /// library mutex: thirty rescans queued behind each other would be one long silence, which is
    /// exactly what the cap exists to say something about.
    pub fn rescan(&mut self) -> Option<DropStatus> {
        if self.outstanding >= MAX_QUEUED {
            return Some(DropStatus::Failed(
                "not just now: packages are still going in".to_owned(),
            ));
        }
        self.hand_over(Job::Rescan)
    }

    /// The shared tail of [`Self::submit`] and [`Self::rescan`].
    fn hand_over(&mut self, job: Job) -> Option<DropStatus> {
        match self.to_worker.send(job) {
            Ok(()) => {
                self.outstanding += 1;
                None
            }
            Err(_) => Some(DropStatus::Failed(
                "packages cannot be installed in this run".to_owned(),
            )),
        }
    }

    /// The next thing the worker has to say, if it has anything.
    ///
    /// Non-blocking, and called once a frame: the display thread may never wait on this.
    pub fn poll(&mut self) -> Option<DropStatus> {
        match self.from_worker.try_recv() {
            Ok(status) => {
                // Only a verdict retires a path; a `Working` is the same one still in progress.
                if !matches!(status, DropStatus::Working(_)) {
                    self.outstanding = self.outstanding.saturating_sub(1);
                }
                Some(status)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

/// Puts one dropped package where it belongs and installs it.
///
/// The error is a sentence for the screen, already stripped of the path — the same treatment
/// `Machine::record_package_problem` gives a startup failure, and for the same reason: a Windows
/// path is one unbreakable word and would take the whole line the reason needs.
fn install(machine: &Arc<Machine>, path: &Path) -> Result<String, String> {
    let debug_packages = machine.debug_packages();
    let (destination, _id) = adopt(machine.paths(), &debug_packages, path)?;
    let report = machine
        .install(&destination)
        .map_err(|error| error.to_string())?;

    // The wording is [`km_api::dto::InstallReportDto::sentence`]'s and is not spelled out here.
    // Spelling it out here and again in `Machine::accept_upload` is two copies of one sentence in
    // one crate, which is how a drop and an upload come to report the same install in two different
    // ways. What it quotes is the package's name, which the report carries out of the manifest the
    // install just read — `adopt` has the same manifest open a moment earlier and passing its copy
    // along would be a second route to one string.
    Ok(km_api::dto::InstallReportDto::from(&report).sentence())
}

/// Takes a package into the machine's own folders, and says where it landed.
///
/// The half of [`install`] that does not need a running machine, split out because a
/// double-clicked `.kmpkg` reaches [`crate::cli`] before there is one — and putting a second copy
/// policy there is exactly how two copies of a package under two names get into a catalog. There
/// is one, and this is it.
///
/// Nothing is installed here: on the cold path the ordinary startup scan finds the file where this
/// left it, which is the behavior `settings::packages_to_install` has always had.
///
/// Returns where it landed **and what it calls itself**, because both callers need the id and
/// opening the package a second time to get it would be reading a manifest off a spinning disk
/// twice for a string already in hand.
pub(crate) fn adopt(
    paths: &Paths,
    debug_packages: &[PathBuf],
    path: &Path,
) -> Result<(PathBuf, String), String> {
    // Opened before anything is copied. A file that will not open is refused here for the price of
    // reading its manifest, rather than after a gigabyte has been copied into the packages folder
    // and left there for the next start to trip over.
    //
    // The same stripping the startup scan applies, from the same function: `km_kmpkg::PackageError`
    // names the file in every variant it has, the sentence this ends up in has already named it, and
    // one line cannot hold a Windows path twice.
    //
    // The stem comes out of the same read as the id, because the name it is built from is beside the
    // id in the manifest and opening the archive again for it would be a second seek for a string
    // already in hand.
    let (stem, id) = {
        let package = km_kmpkg::Package::open(path).map_err(|error| {
            reason_without_path(&path.display().to_string(), &error.to_string())
        })?;
        let meta = &package.manifest().package;
        (meta.file_stem(), meta.id.clone())
    };
    // The **write** folder, which on Android is the external one rather than the private one: a
    // package reaches tens of gigabytes and internal storage is the smaller volume. Deliberately not
    // the first folder scanned; see `Paths::write_dir`.
    let destination = place(
        &paths.packages_write_dir(),
        &paths.packages_dirs(),
        debug_packages,
        path,
        &stem,
        &id,
    )?;
    Ok((destination, id))
}

/// Where a dropped package should live, copying it there if it is not already somewhere scanned.
///
/// Takes the folder lists rather than the [`Machine`] they come from — `dir` is the one folder a
/// copy goes into, `scanned` is every folder a restart would find it in, and `debug_packages` is
/// what the owner named by hand — so the naming, copying and sweeping rules can be tested against a
/// scratch directory instead of a whole machine.
///
/// `stem` is the package's own, from [`km_kmpkg::PackageMeta::file_stem`], and not the source
/// file's: what a file is called on the way in says nothing about which package is inside it.
pub(crate) fn place(
    dir: &Path,
    scanned: &[PathBuf],
    debug_packages: &[PathBuf],
    path: &Path,
    stem: &str,
    id: &str,
) -> Result<PathBuf, String> {
    // Already in a folder the machine scans — the owner's own packages folder, Android's shared one,
    // or a folder they named in `settings.package_dirs`. Copying it would produce a second file of
    // the same package for no reason, and a rescan would then find both. Left under whatever name it
    // has, too: the owner put it there, and renaming somebody's file where it lies is not what
    // taking a package in means.
    if scanned.iter().any(|dir| is_directly_inside(path, dir)) {
        return Ok(path.to_owned());
    }

    // The name is settled before the folder is made, so a placement that is going to be refused
    // leaves nothing behind — not even an empty folder somebody would later wonder about.
    let destination = free_name(dir, stem, id)?;
    fs::create_dir_all(dir)
        .map_err(|error| format!("the packages folder is not there: {error}"))?;

    // Copied under a name the scanner ignores and then renamed, so that a copy interrupted halfway
    // — a full disk, an unplugged drive, the machine being closed — cannot leave something that
    // looks like a package in the folder the machine reads at every start.
    let partial = destination.with_extension("part");
    fs::copy(path, &partial).map_err(|error| {
        let _ = fs::remove_file(&partial);
        format!("it could not be copied into the packages folder: {error}")
    })?;
    fs::rename(&partial, &destination).map_err(|error| {
        let _ = fs::remove_file(&partial);
        format!("it could not be copied into the packages folder: {error}")
    })?;

    // After the new file is safely in place, never before: a sweep that ran first would take the
    // only copy away and then have the copy fail.
    sweep_superseded(dir, debug_packages, &destination, id);
    Ok(destination)
}

/// Removes any other file in the write folder holding the package that has just landed.
///
/// **The other half of a name the manifest decides.** A derived name means the ordinary case never
/// reaches here — a rebuild lands on its predecessor and there is nothing left over. What is left
/// over is a file from before this rule existed, or one delivered under a name of somebody else's
/// choosing, and leaving it would put two files of one package in the folder. That is not merely
/// untidy: the scan sorts by file name and keeps the first id it meets, so the *older* file would go
/// on being the installed one, silently and for ever.
///
/// **What it may take is narrow, and each bound answers a rule already in force.** Only `dir`, which
/// is the folder the machine writes to and therefore one it owns — never another entry in
/// `packages_dirs`, because a `package_dirs` folder is somewhere the owner keeps their own files.
/// Never the file just written. Never a file named in `debug.packages`, which is the one place an
/// owner names a single file and is not the machine's to remove. And never a file that will not
/// open, which is a standing fault the Problems tab reports and which taking away would hide.
///
/// At `warn` and one line each, matching the account the uninstall route keeps: this is the machine
/// destroying a file, and the log is the only record of it.
fn sweep_superseded(dir: &Path, debug_packages: &[PathBuf], keep: &Path, id: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_package(path))
    {
        if crate::settings::tidy(&path) == crate::settings::tidy(keep) {
            continue;
        }
        if debug_packages
            .iter()
            .any(|named| crate::settings::tidy(named) == crate::settings::tidy(&path))
        {
            continue;
        }
        if !km_kmpkg::Package::open(&path).is_ok_and(|held| held.manifest().package.id == id) {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => tracing::warn!(
                package = %id,
                superseded = %file_name(&path),
                kept = %file_name(keep),
                "deleting an older file of a package that has just been installed again"
            ),
            // Not an error the install fails on: the package went in, and what is left is a second
            // file of it that the next start will skip. Saying so is the whole of what can be done.
            Err(error) => tracing::warn!(
                package = %id,
                superseded = %file_name(&path),
                %error,
                "could not delete an older file of a package that has just been installed again"
            ),
        }
    }
}

/// A name in the packages folder that does not take somebody else's file.
///
/// The package's derived name first. If something is already there under it, that file is opened:
/// the same id is the same package and this is an upgrade, so its file is replaced — which is what
/// makes rebuilding a volume and installing it again do the obvious thing, whatever the rebuilt file
/// was called.
///
/// Anything else at that name gets stepped over to `-2`, `-3` and so on. Two *different* packages
/// can no longer arrive here, the id being part of the name, so what this is left guarding is a file
/// that will not open — see [`MAX_NAME_ATTEMPTS`].
fn free_name(dir: &Path, stem: &str, id: &str) -> Result<PathBuf, String> {
    // The stem is built from the manifest, so it is the package's word for where its own file
    // should go. `Manifest::problems` refuses an id that is not a name and `file_stem` folds one
    // anyway, which makes this unreachable through either route in — and it is here because this
    // is the line that joins, and a guard at the join survives a caller that stops doing both.
    if !km_kmpkg::is_safe_name(stem) {
        return Err(format!(
            "{stem:?} is not a name a package's file can be called, so it will not be written"
        ));
    }
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let candidate = if attempt == 1 {
            dir.join(format!("{stem}.kmpkg"))
        } else {
            dir.join(format!("{stem}-{attempt}.kmpkg"))
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
        // An unreadable file already sitting there is not something to overwrite either: it is a
        // fault the startup scan already complains about, and taking its name would hide it.
        if km_kmpkg::Package::open(&candidate).is_ok_and(|held| held.manifest().package.id == id) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "the packages folder already holds {MAX_NAME_ATTEMPTS} files called \"{stem}\" that are not \
         this package"
    ))
}

/// Whether a file sits directly in this folder, rather than merely below it.
///
/// Directly, because that is all [`crate::settings::packages_to_install`] scans: a package two
/// folders down inside the packages folder is not installed by a restart, so leaving it where it is
/// would make a drop work once and never again.
///
/// **Both sides go through [`crate::settings::tidy`]**, as every other comparison of these paths
/// does, and it stopped being cosmetic when [`place`] gained a second caller in the API route. A
/// raw comparison is wrong wherever the two spellings differ but name one directory — a symlinked
/// home on the appliance, macOS resolving `/var` to `/private/var`, a Windows 8.3 short name — and
/// the failure it produces is expensive rather than obvious: `place` decides the file is *not* in
/// the folder, `free_name` hands back the same-id file already sitting there, and the copy then
/// writes the archive **onto itself** through a `.part`. That "works" while moving twenty gigabytes
/// for nothing, and on a nearly full disk it fails instead.
fn is_directly_inside(path: &Path, dir: &Path) -> bool {
    crate::settings::tidy(path)
        .parent()
        .is_some_and(|parent| parent == crate::settings::tidy(dir))
}

/// Whether a path names a package, by extension and ignoring case.
///
/// The same test [`crate::settings`] applies to the packages folder, and case-insensitive for the
/// same reason: this is a file somebody dragged out of a file manager, and Windows hands back
/// whatever case the copy happened to have.
fn is_package(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("kmpkg"))
}

/// The file's name, for a message that has no room for its path.
fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("that file")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that removes itself, as in `crate::settings`'s tests.
    ///
    /// Named for the process and the thread, because the whole suite runs in parallel against one
    /// system temporary directory.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-dropped-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("make the scratch directory");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_a_kmpkg_is_a_package() {
        assert!(is_package(Path::new("vol1.kmpkg")));
        // Windows hands back whatever case the copy had.
        assert!(is_package(Path::new("VOL1.KMPKG")));
        assert!(!is_package(Path::new("song.mid")));
        // The name a half-finished copy carries. It must not read as a package, or the very
        // precaution that keeps a broken copy out of the folder would put one in.
        assert!(!is_package(Path::new("vol1.part")));
        assert!(!is_package(Path::new("kmpkg")));
    }

    #[test]
    fn a_file_below_the_folder_is_not_in_it() {
        let dir = Path::new("/data/packages");
        assert!(is_directly_inside(Path::new("/data/packages/a.kmpkg"), dir));
        // The startup scan does not recurse, so neither may this: leaving a package one folder down
        // where it lies would install it once and never again.
        assert!(!is_directly_inside(
            Path::new("/data/packages/old/a.kmpkg"),
            dir
        ));
        assert!(!is_directly_inside(Path::new("/elsewhere/a.kmpkg"), dir));
    }

    /// One folder spelled two ways is one folder.
    ///
    /// Cheap to get wrong and expensive when it is: a raw comparison would decide the file is not in
    /// the folder, `free_name` would hand back the same-id file already there, and the copy would
    /// write the archive **onto itself** through a `.part` — moving twenty gigabytes for nothing, or
    /// failing outright on a nearly full disk. Real paths rather than invented ones, because `tidy`
    /// canonicalises and canonicalising only resolves something that exists.
    #[test]
    fn one_folder_spelled_two_ways_is_still_one_folder() {
        let scratch = Scratch::new("spellings");
        let dir = scratch.path().join("packages");
        std::fs::create_dir_all(&dir).expect("the folder");
        let package = dir.join("vol1.kmpkg");
        std::fs::write(&package, b"not really a package").expect("the file");

        // The same folder reached through a `.` component — the shape a caller assembling a path
        // from parts produces, and the cheapest stand-in for the platform spellings that cannot be
        // manufactured in a test: a symlinked home, macOS resolving `/var`, a Windows short name.
        let roundabout = dir.join(".");
        assert!(
            is_directly_inside(&package, &roundabout),
            "a second spelling of the folder must not read as a different folder"
        );
    }

    #[test]
    fn a_free_name_is_the_packages_own_when_nothing_is_there() {
        let scratch = Scratch::new("free");
        let chosen =
            free_name(scratch.path(), "brasil1-vol1", km_kmpkg::EXAMPLE_ID).expect("a name");
        assert_eq!(chosen, scratch.path().join("brasil1-vol1.kmpkg"));
    }

    #[test]
    fn a_file_at_that_name_that_is_not_the_same_package_is_stepped_over() {
        // The file here is not a package at all, so its id can never match the incoming one's —
        // which is the only case left now that the name carries the id, and it is somebody's file
        // that this must not take the name of. The startup scan already complains about an
        // unreadable package; overwriting it would silence the complaint and destroy the file in one
        // go.
        let scratch = Scratch::new("collide");
        fs::write(scratch.path().join("brasil1-vol1.kmpkg"), b"not a package").expect("write");
        let chosen =
            free_name(scratch.path(), "brasil1-vol1", km_kmpkg::EXAMPLE_ID).expect("a name");
        assert_eq!(chosen, scratch.path().join("brasil1-vol1-2.kmpkg"));
    }

    #[test]
    fn names_run_out_rather_than_looping_for_ever() {
        let scratch = Scratch::new("full");
        fs::write(scratch.path().join("brasil1-vol1.kmpkg"), b"x").expect("write");
        for attempt in 2..=MAX_NAME_ATTEMPTS {
            fs::write(
                scratch.path().join(format!("brasil1-vol1-{attempt}.kmpkg")),
                b"x",
            )
            .expect("write");
        }
        assert!(
            free_name(scratch.path(), "brasil1-vol1", km_kmpkg::EXAMPLE_ID).is_err(),
            "a folder holding every name should be reported rather than searched for ever"
        );
    }

    /// Writes a real one-song package, so the id comparison is made against a manifest rather than
    /// against a stub that could agree with the code while the code was wrong.
    fn write_package(path: &Path, id: &str) {
        write_named_package(path, id, "Vol 1", "1.0.0");
    }

    /// The file name the machine gives a package of this name and id.
    ///
    /// Asked of `km_kmpkg` rather than spelled out, so a test cannot go on passing against a rule
    /// the placement code no longer follows.
    fn stem_of(id: &str, name: &str) -> String {
        km_kmpkg::PackageMeta {
            id: id.to_owned(),
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            created: None,
            volume: None,
        }
        .file_stem()
    }

    fn write_named_package(path: &Path, id: &str, name: &str, version: &str) {
        let mut builder = km_kmpkg::PackageBuilder::new(km_kmpkg::PackageMeta {
            id: id.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            publisher: None,
            created: None,
            volume: None,
        });
        builder
            .add(
                km_kmpkg::SongEntry {
                    number: 1,
                    kind: km_kmpkg::SongKind::Midi,
                    title: "A Song".to_owned(),
                    artist: None,
                    language: Some("und".to_owned()),
                    file: String::new(),
                    duration_ms: 1_000,
                    lyric_encoding: None,
                    default_transpose: 0,
                    lyrics_hidden: false,
                    fixes: Vec::new(),
                    melody: None,
                    melody_abstained: None,
                    suitability: None,
                    lyric_preview: Vec::new(),
                    tags: Vec::new(),
                    loudness: None,
                    content_hash: None,
                    edited: Vec::new(),
                },
                km_song::testing::soft_karaoke(),
            )
            .expect("add the song");
        builder.write(path).expect("write the package");
    }

    /// What is in a folder, by file name, sorted so an assertion reads the same on every platform.
    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_package_already_in_a_scanned_folder_is_installed_where_it_lies() {
        // Copying it would leave two files of one package in the folder, and the next start would
        // find both. It keeps the name it has, too: the owner put it there.
        let scratch = Scratch::new("in-place");
        let dir = scratch.path().join("packages");
        fs::create_dir_all(&dir).expect("the packages folder");
        let source = dir.join("whatever-they-called-it.kmpkg");
        write_package(&source, km_kmpkg::EXAMPLE_ID);

        let chosen = place(
            &dir,
            std::slice::from_ref(&dir),
            &[],
            &source,
            &stem_of(km_kmpkg::EXAMPLE_ID, "Vol 1"),
            km_kmpkg::EXAMPLE_ID,
        )
        .expect("a destination");
        assert_eq!(chosen, source);
        assert_eq!(listing(&dir).len(), 1, "nothing was copied");
    }

    /// A package names the file it is installed as, so the name is somebody else's to choose.
    ///
    /// **Asserted by where the file went, not by whether an error came back.** A refusal that still
    /// wrote the file would pass a test that only read the `Result`, and writing the file is the
    /// whole of the harm — the machine takes a package from the operating system before anybody has
    /// typed a password.
    #[test]
    fn a_stem_that_would_leave_the_packages_folder_writes_nothing() {
        let scratch = Scratch::new("escaping-stem");
        let dir = scratch.path().join("data").join("packages");
        let elsewhere = scratch.path().join("downloads");
        fs::create_dir_all(&elsewhere).expect("the downloads folder");
        let source = elsewhere.join("innocent.kmpkg");
        write_package(&source, km_kmpkg::EXAMPLE_ID);

        // What the manifest would have to say for the file to land outside. Passed to `place`
        // directly, because the two gates before it — `problems` and `file_stem` — each already
        // stop this, and what is under test is the line that joins.
        for stem in [
            "../../../../evil",
            "..\\..\\evil",
            "/etc/cron.d/evil",
            "D:\\tunes\\evil",
            "vol1/../../evil",
            "..",
            "",
        ] {
            let outcome = place(
                &dir,
                std::slice::from_ref(&dir),
                &[],
                &source,
                stem,
                km_kmpkg::EXAMPLE_ID,
            );
            assert!(
                outcome.is_err(),
                "{stem:?} must not be joined onto the packages folder"
            );
        }

        // The scratch folder holds the download and nothing else: no `evil.kmpkg` beside it, no
        // `.part` left behind, and no packages folder brought into being by a refused write.
        let mut left: Vec<String> = fs::read_dir(scratch.path())
            .expect("the scratch folder")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into()
            })
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec!["downloads".to_owned()],
            "a refused placement wrote something"
        );
        assert_eq!(
            listing(&elsewhere).len(),
            1,
            "the file that was handed in is all that is there"
        );
    }

    /// The name on the way in decides nothing; the manifest does.
    #[test]
    fn a_package_is_stored_under_the_name_its_manifest_implies() {
        let scratch = Scratch::new("copy-in");
        let dir = scratch.path().join("packages");
        let elsewhere = scratch.path().join("downloads");
        fs::create_dir_all(&elsewhere).expect("the downloads folder");
        let source = elsewhere.join("brasil1-1.0.0.kmpkg");
        write_named_package(&source, "1f4a9c8e2b7d0356", "Brasil Volume 1", "1.0.0");

        let chosen = place(
            &dir,
            std::slice::from_ref(&dir),
            &[],
            &source,
            &stem_of("1f4a9c8e2b7d0356", "Brasil Volume 1"),
            "1f4a9c8e2b7d0356",
        )
        .expect("a destination");
        assert_eq!(
            chosen,
            dir.join("brasil-volume-1-1f4a9c8e2b7d0356.kmpkg"),
            "the built file's name must not reach the packages folder"
        );
        assert!(source.exists(), "the owner's own copy is left alone");
        // The `.part` name exists so an interrupted copy is never mistaken for a package. A
        // completed one must not leave it behind.
        assert_eq!(
            listing(&dir),
            vec!["brasil-volume-1-1f4a9c8e2b7d0356.kmpkg".to_owned()]
        );
    }

    /// **The case this naming rule exists for.**
    ///
    /// A rebuild carries a new version and therefore a new file name, and it still has to land on
    /// the file it replaces. Under the old rule — the source file's own stem — the two names
    /// differed, so the folder kept both and the scan went on serving whichever sorted first, which
    /// is the older one.
    #[test]
    fn a_rebuild_under_a_new_file_name_still_replaces_its_predecessor() {
        let scratch = Scratch::new("upgrade");
        let dir = scratch.path().join("packages");
        fs::create_dir_all(&dir).expect("the packages folder");
        let stem = stem_of("1f4a9c8e2b7d0356", "Brasil Volume 1");
        write_named_package(
            &dir.join(format!("{stem}.kmpkg")),
            "1f4a9c8e2b7d0356",
            "Brasil Volume 1",
            "1.0.0",
        );

        let elsewhere = scratch.path().join("downloads");
        fs::create_dir_all(&elsewhere).expect("the downloads folder");
        let source = elsewhere.join("brasil1-1.0.1.kmpkg");
        write_named_package(&source, "1f4a9c8e2b7d0356", "Brasil Volume 1", "1.0.1");

        let chosen = place(
            &dir,
            std::slice::from_ref(&dir),
            &[],
            &source,
            &stem,
            "1f4a9c8e2b7d0356",
        )
        .expect("a destination");
        assert_eq!(chosen, dir.join(format!("{stem}.kmpkg")));
        assert_eq!(listing(&dir), vec![format!("{stem}.kmpkg")]);
        assert_eq!(
            km_kmpkg::Package::open(&chosen)
                .expect("a package")
                .manifest()
                .package
                .version,
            "1.0.1",
            "the file in the folder must be the build that just arrived"
        );
    }

    #[test]
    fn two_different_packages_of_one_name_get_two_files_and_no_ladder() {
        // Two volumes both called `Vol 1` — different packages that happen to share a name. The id
        // in the file name is what keeps them apart, so neither has to step over the other.
        let scratch = Scratch::new("collide-real");
        let dir = scratch.path().join("packages");
        fs::create_dir_all(&dir).expect("the packages folder");
        let held = stem_of("a1b2c3d4e5f60789", "Vol 1");
        write_named_package(
            &dir.join(format!("{held}.kmpkg")),
            "a1b2c3d4e5f60789",
            "Vol 1",
            "1.0.0",
        );

        let elsewhere = scratch.path().join("downloads");
        fs::create_dir_all(&elsewhere).expect("the downloads folder");
        let source = elsewhere.join("vol1.kmpkg");
        write_named_package(&source, "0987f6e5d4c3b2a1", "Vol 1", "1.0.0");

        let incoming = stem_of("0987f6e5d4c3b2a1", "Vol 1");
        let chosen = place(
            &dir,
            std::slice::from_ref(&dir),
            &[],
            &source,
            &incoming,
            "0987f6e5d4c3b2a1",
        )
        .expect("a destination");
        assert_eq!(chosen, dir.join(format!("{incoming}.kmpkg")));
        assert!(
            !incoming.contains("-2"),
            "the ladder has nothing to do here: {incoming}"
        );
        assert_eq!(
            km_kmpkg::Package::open(dir.join(format!("{held}.kmpkg")))
                .expect("still a package")
                .manifest()
                .package
                .id,
            "a1b2c3d4e5f60789",
            "the package that was there is untouched"
        );
    }

    /// The sweep is what keeps a folder honest across the change of rule.
    ///
    /// A folder holding a package under a name from before this rule existed would otherwise keep
    /// both files, and the scan takes the first by name — which is as likely as not the older one.
    #[test]
    fn an_older_file_of_the_same_package_is_swept_out_of_the_write_folder() {
        let scratch = Scratch::new("sweep");
        let dir = scratch.path().join("packages");
        fs::create_dir_all(&dir).expect("the packages folder");
        // The name the builder used to write, which sorts before any slugged name beginning with a
        // letter — so leaving it is not a tidiness problem but a wrong-version-served one.
        write_named_package(
            &dir.join("1f4a9c8e2b7d0356.kmpkg"),
            "1f4a9c8e2b7d0356",
            "Brasil Volume 1",
            "1.0.0",
        );

        let elsewhere = scratch.path().join("downloads");
        fs::create_dir_all(&elsewhere).expect("the downloads folder");
        let source = elsewhere.join("brasil1-1.0.1.kmpkg");
        write_named_package(&source, "1f4a9c8e2b7d0356", "Brasil Volume 1", "1.0.1");

        let stem = stem_of("1f4a9c8e2b7d0356", "Brasil Volume 1");
        let chosen = place(
            &dir,
            std::slice::from_ref(&dir),
            &[],
            &source,
            &stem,
            "1f4a9c8e2b7d0356",
        )
        .expect("a destination");
        assert_eq!(chosen, dir.join(format!("{stem}.kmpkg")));
        assert_eq!(
            listing(&dir),
            vec![format!("{stem}.kmpkg")],
            "the file from before the rule must not be left behind"
        );
    }

    /// What the sweep may not take: another package, another folder, and a file the owner named.
    #[test]
    fn the_sweep_leaves_alone_what_is_not_the_machines_to_remove() {
        let scratch = Scratch::new("sweep-guards");
        let dir = scratch.path().join("packages");
        let theirs = scratch.path().join("their-packages");
        fs::create_dir_all(&dir).expect("the packages folder");
        fs::create_dir_all(&theirs).expect("their folder");

        // Same package, in a folder the machine only scans. Somewhere the owner keeps their own
        // files is not somewhere the machine deletes from.
        write_named_package(
            &theirs.join("old-copy.kmpkg"),
            "1f4a9c8e2b7d0356",
            "Brasil Volume 1",
            "1.0.0",
        );
        // Same package, in the write folder, but named in `debug.packages`. The one place an owner
        // names a single file, and not the machine's to remove.
        let named = dir.join("named-by-hand.kmpkg");
        write_named_package(&named, "1f4a9c8e2b7d0356", "Brasil Volume 1", "1.0.0");
        // A different package, which the sweep has no business touching at all.
        write_named_package(
            &dir.join("someone-else.kmpkg"),
            "0987f6e5d4c3b2a1",
            "Italia",
            "1.0.0",
        );

        let elsewhere = scratch.path().join("downloads");
        fs::create_dir_all(&elsewhere).expect("the downloads folder");
        let source = elsewhere.join("brasil1-1.0.1.kmpkg");
        write_named_package(&source, "1f4a9c8e2b7d0356", "Brasil Volume 1", "1.0.1");

        let stem = stem_of("1f4a9c8e2b7d0356", "Brasil Volume 1");
        place(
            &dir,
            &[dir.clone(), theirs.clone()],
            std::slice::from_ref(&named),
            &source,
            &stem,
            "1f4a9c8e2b7d0356",
        )
        .expect("a destination");

        assert_eq!(
            listing(&dir),
            vec![
                format!("{stem}.kmpkg"),
                "named-by-hand.kmpkg".to_owned(),
                "someone-else.kmpkg".to_owned(),
            ]
        );
        assert_eq!(listing(&theirs), vec!["old-copy.kmpkg".to_owned()]);
    }

    /// A file that will not open is a standing fault, and taking it away would hide it.
    #[test]
    fn the_sweep_leaves_a_file_it_cannot_read() {
        let scratch = Scratch::new("sweep-unreadable");
        let dir = scratch.path().join("packages");
        fs::create_dir_all(&dir).expect("the packages folder");
        fs::write(dir.join("broken.kmpkg"), b"not a package").expect("write");

        let elsewhere = scratch.path().join("downloads");
        fs::create_dir_all(&elsewhere).expect("the downloads folder");
        let source = elsewhere.join("vol1.kmpkg");
        write_package(&source, km_kmpkg::EXAMPLE_ID);

        let stem = stem_of(km_kmpkg::EXAMPLE_ID, "Vol 1");
        place(
            &dir,
            std::slice::from_ref(&dir),
            &[],
            &source,
            &stem,
            km_kmpkg::EXAMPLE_ID,
        )
        .expect("a destination");
        assert_eq!(
            listing(&dir),
            vec!["broken.kmpkg".to_owned(), format!("{stem}.kmpkg")]
        );
    }

    #[test]
    fn a_dropped_file_that_is_not_a_package_is_refused_before_it_reaches_the_worker() {
        // Answered by `submit` rather than by the worker, so that dropping an `.mp3` says so at
        // once instead of after whatever is ahead of it in the queue.
        let (to_worker, _work) = channel::<Job>();
        let (_report, from_worker) = channel::<DropStatus>();
        let mut installer = DropInstaller {
            to_worker,
            from_worker,
            outstanding: 0,
        };

        let refused = installer.submit(PathBuf::from("/downloads/song.mp3"));
        assert!(matches!(refused, Some(DropStatus::Failed(_))));
        assert_eq!(installer.outstanding, 0, "nothing was queued");

        assert!(
            installer
                .submit(PathBuf::from("/downloads/vol1.kmpkg"))
                .is_none()
        );
        assert_eq!(installer.outstanding, 1);
    }

    #[test]
    fn a_burst_is_capped_rather_than_appearing_to_hang() {
        let (to_worker, _work) = channel::<Job>();
        let (_report, from_worker) = channel::<DropStatus>();
        let mut installer = DropInstaller {
            to_worker,
            from_worker,
            outstanding: MAX_QUEUED,
        };
        assert!(matches!(
            installer.submit(PathBuf::from("/downloads/vol1.kmpkg")),
            Some(DropStatus::Failed(_))
        ));
    }

    /// A rescan queues on the same worker as a drop, and is capped with it.
    ///
    /// One queue rather than two, so a rescan and an install cannot run at once and each wait on
    /// the other's transaction — and one counter, because thirty rescans behind each other is the
    /// same long silence the cap exists to say something about.
    #[test]
    fn a_rescan_shares_the_queue_and_the_cap_with_a_drop() {
        let (to_worker, _work) = channel::<Job>();
        let (_report, from_worker) = channel::<DropStatus>();
        let mut installer = DropInstaller {
            to_worker,
            from_worker,
            outstanding: 0,
        };

        assert!(installer.rescan().is_none(), "an idle worker takes it");
        assert_eq!(installer.outstanding, 1);

        installer.outstanding = MAX_QUEUED;
        assert!(
            matches!(installer.rescan(), Some(DropStatus::Failed(_))),
            "a full queue refuses it rather than making it wait unseen"
        );
    }

    /// A rescan's one line says what it did, and names work it could not do yet.
    ///
    /// The deferred count is the half worth pinning: a rescan that found a package gone and could
    /// not act on it must not read the same as one that found nothing to do.
    #[test]
    fn a_rescan_says_what_it_did_and_what_is_waiting() {
        let quiet = rescan_summary(&km_api::machine::RescanReport {
            installed: 3,
            ..Default::default()
        });
        assert_eq!(quiet, "3 packages");

        let busy = rescan_summary(&km_api::machine::RescanReport {
            installed: 1,
            removed: vec!["vol2".to_owned()],
            deferred: vec!["vol3".to_owned()],
            problems: 2,
        });
        assert!(busy.starts_with("1 package \u{b7} 1 removed"), "{busy}");
        assert!(busy.contains("waiting for the queue to empty"), "{busy}");
        assert!(busy.ends_with("2 problems"), "{busy}");
    }

    #[test]
    fn only_a_verdict_retires_a_queued_drop() {
        // A `Working` is the same package still in progress. Counting it as finished would let the
        // cap drift upwards by one for every file dropped.
        let (to_worker, _work) = channel::<Job>();
        let (report, from_worker) = channel::<DropStatus>();
        let mut installer = DropInstaller {
            to_worker,
            from_worker,
            outstanding: 2,
        };

        report
            .send(DropStatus::Working("installing vol1.kmpkg…".to_owned()))
            .expect("send");
        report
            .send(DropStatus::Done("installed \"vol1\"".to_owned()))
            .expect("send");

        assert!(matches!(installer.poll(), Some(DropStatus::Working(_))));
        assert_eq!(installer.outstanding, 2);
        assert!(matches!(installer.poll(), Some(DropStatus::Done(_))));
        assert_eq!(installer.outstanding, 1);
        assert!(installer.poll().is_none());
    }
}
