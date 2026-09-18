//! One open folder, and everything that belongs to it.
//!
//! This is the half of the old `State` that is *about a corpus*: the database, the folder it sits
//! in, and whatever scan is running over it. It was split out when the tool learned to open a folder
//! chosen from a page rather than one named on the command line, because at that point "the
//! database" stopped being a thing the process had for its whole life and became a thing it has
//! zero or one of at a time.
//!
//! **A folder holds two connections on its database, and which one a caller gets says what it may
//! do.** The writing one is held by a scan for the length of a batch; the reading one is what a page
//! is drawn through, so a render waits for nothing. Write-ahead logging is what makes the pair safe,
//! and a database outside it has no second connection at all — see [`Workspace::reader`]. The
//! reading one is also let go before the closing checkpoint, because a checkpoint cannot truncate
//! past a reader.
//!
//! **The job handles live here rather than on [`State`](crate::server::State), and that is
//! load-bearing.** A scan is a detached `std::thread` holding a clone of `Arc<Shared>`, running
//! for minutes. Left on the shared state it would go on writing into the *previous* corpus after a
//! swap, while the pages showed the new one — a data-loss bug with no symptom until somebody noticed
//! rows appearing in a folder nobody had open. Owned by the workspace, a scan cannot outlive the
//! folder it is scanning: [`Workspace::drop`] stops and joins it first.
//!
//! **There are two such jobs now**, and the second is worse if it is got wrong. A build holds no
//! lock for most of its life — that is the point of it, and what lets the rest of the tool answer
//! while a package is being written — so `Drop`'s `db.lock()` would succeed *while one was still
//! running*, close the connection under it, and leave the build's final `record_build` writing into
//! a database that has been checkpointed and closed. Stopping both before the checkpoint is what
//! makes that impossible, and it costs at most one song: the build asks whether to stop between
//! them rather than inside an ffmpeg re-encode.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::build::{BuildProgress, BuildProgressView};
use crate::db::{Db, Shared};
use crate::scan::{Progress, ProgressView, ScanOptions};

/// A folder that is open, with its database and its two long-running jobs.
pub struct Workspace {
    /// The database, for everything that writes to it.
    pub db: Arc<Shared>,
    /// A second connection on the same file, which only reads.
    ///
    /// **This is what keeps a page answering while a scan runs.** The writer above is held for a
    /// whole scan batch, so a page that needed it waited for a gap between batches, and on a
    /// corpus-sized scan that wait had no bound. WAL makes the pair safe: this connection sees the
    /// last committed state while a batch is in flight, and neither waits for the other.
    ///
    /// **`None` means read through [`Self::db`]**, and it is what a database outside write-ahead
    /// logging gets. Those are the modes where a writer excludes readers outright, so a second
    /// connection there answers "database is locked" where one shared connection merely answers
    /// late — and an in-memory database, which every test takes, is among them: `:memory:` opened
    /// twice is two empty databases rather than two views of one. Falling back means such a folder
    /// behaves as it did before this existed.
    reader: Option<Arc<Shared>>,
    /// The folder being curated.
    root: PathBuf,
    /// The running scan, if there is one.
    scan: Mutex<Option<Arc<Progress>>>,
    /// The thread that scan is on, kept so closing can wait for it rather than kill it.
    scan_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The running build, if there is one, and the last one's result once it has ended.
    ///
    /// One slot, so two builds cannot run at once. That is deliberate serialization rather than a
    /// limitation: two threads writing packages would both call `record_build`, and the page has one
    /// place to show a bar.
    build: Mutex<Option<Arc<BuildProgress>>>,
    /// The thread that build is on, for the same reason as [`Self::scan_thread`].
    build_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Workspace {
    /// Wraps an open database, and opens a reading connection beside it.
    ///
    /// **The reading connection is opened here rather than by the caller**, because here is the one
    /// point where the writing connection is known to have finished with the schema: `Db::prepare`
    /// has run the version ladder, the backfills and the index build by the time a `Db` exists to be
    /// handed over. A reader opened any earlier could see a table that is about to be rebuilt.
    ///
    /// **It cannot fail the open.** A folder whose second connection will not open is still a folder
    /// somebody asked for, so the reader falls back to the writer and the tool goes on working the
    /// way it did before it had one. The log says which happened, because the two are
    /// indistinguishable from any page.
    pub fn new(db: Db) -> Self {
        let root = db.root().to_path_buf();
        let reader = if db.in_wal() {
            match Db::open_reading(&root) {
                Ok(reader) => Some(Arc::new(Shared::new(reader))),
                Err(error) => {
                    tracing::warn!(
                        "reading {} through the writing connection: {error}",
                        root.display()
                    );
                    None
                }
            }
        } else {
            None
        };
        Self {
            db: Arc::new(Shared::new(db)),
            reader,
            root,
            scan: Mutex::new(None),
            scan_thread: Mutex::new(None),
            build: Mutex::new(None),
            build_thread: Mutex::new(None),
        }
    }

    /// The connection a page reads through, which is the writing one where there is no second.
    pub fn reader(&self) -> &Arc<Shared> {
        self.reader.as_ref().unwrap_or(&self.db)
    }

    /// The folder being curated.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether a scan is running right now.
    pub fn scan_running(&self) -> bool {
        self.scan
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|p| !p.snapshot().finished))
            .unwrap_or(false)
    }

    /// The current or last scan's progress.
    pub fn scan_progress(&self) -> ProgressView {
        self.scan
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|p| p.snapshot()))
            .unwrap_or_default()
    }

    /// Starts a scan on a thread of its own, and returns its progress handle.
    ///
    /// The one place a scan is spawned, so the command line and the Scan page's button cannot end up
    /// with different ideas about how one is started or stopped. A plain thread rather than a tokio
    /// task: the scan is minutes of blocking CPU and file I/O, and it must not sit on a runtime
    /// worker while the browser is asking it for progress.
    pub fn start_scan(&self, options: ScanOptions) -> Arc<Progress> {
        let progress = Arc::new(Progress::default());
        if let Ok(mut slot) = self.scan.lock() {
            *slot = Some(Arc::clone(&progress));
        }

        let db = Arc::clone(&self.db);
        let handle = Arc::clone(&progress);
        let thread = std::thread::spawn(move || {
            if let Err(error) = crate::scan::run(&db, options, &handle) {
                tracing::error!("the scan failed: {error}");
            }
        });
        if let Ok(mut slot) = self.scan_thread.lock() {
            // Any handle already here belongs to a run that has ended — `start_scan` is only reached
            // past a `scan_running` guard — so dropping it detaches a thread that is already gone.
            *slot = Some(thread);
        }
        progress
    }

    /// Asks a running scan to stop, and returns without waiting for it.
    ///
    /// **The Stop button's half of [`Self::stop_scan`]**, and the half without the join: the run
    /// writes what it has already read before it ends, which is up to a few batches, and a request
    /// that waited for that would hold the page for as long. The panel's poll reports the end.
    /// Returns whether there was a run to ask.
    pub fn ask_scan_to_stop(&self) -> bool {
        let running = self.scan_running();
        if running
            && let Ok(slot) = self.scan.lock()
            && let Some(progress) = slot.as_ref()
        {
            progress.ask_to_stop();
        }
        running
    }

    /// Asks a running scan to stop, and waits for it to put down what it is holding.
    ///
    /// Returns whether there was one to stop. Without this, leaving the tool killed the scan thread
    /// wherever it stood: the process exits when `main` returns, and the scan was never joined. The
    /// batch in hand and everything queued behind it were simply lost.
    pub fn stop_scan(&self) -> bool {
        let running = self.scan_running();
        if let Ok(slot) = self.scan.lock()
            && let Some(progress) = slot.as_ref()
        {
            progress.ask_to_stop();
        }
        let thread = self
            .scan_thread
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(thread) = thread {
            // A panicking scan is already reported by the panic hook the run installs; there is
            // nothing useful to do with it here beyond not propagating it out of a shutdown path.
            let _ = thread.join();
        }
        running
    }

    /// Whether the join handle has been consumed, for the test that proves stopping joins.
    #[cfg(test)]
    pub fn scan_thread_is_clear(&self) -> bool {
        self.scan_thread
            .lock()
            .map(|slot| slot.is_none())
            .unwrap_or(true)
    }

    // -- the build ------------------------------------------------------------------------------

    /// Whether a build is running right now.
    pub fn build_running(&self) -> bool {
        self.build
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|p| !p.snapshot().finished))
            .unwrap_or(false)
    }

    /// The current or last build's progress, if there has been one.
    pub fn build_progress(&self) -> Option<BuildProgressView> {
        self.build
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|p| p.snapshot()))
    }

    /// Starts a build on a thread of its own.
    ///
    /// A plain `std::thread` and not `spawn_blocking`, for the reason [`Self::start_scan`] gives: a
    /// job that runs for minutes does not belong on a runtime's blocking pool, where it competes with
    /// every request that needs the database.
    ///
    /// The thread takes the `Arc<Shared>` and `crate::build::build` decides when to lock it —
    /// briefly at each end, and not at all across the middle. That is what keeps the rest of the tool
    /// answering while a package builds.
    pub fn start_build(
        &self,
        package_id: String,
        volume: u32,
        out: PathBuf,
        write_listing: bool,
    ) -> Arc<BuildProgress> {
        let progress = Arc::new(BuildProgress::new(&package_id, volume));
        if let Ok(mut slot) = self.build.lock() {
            *slot = Some(Arc::clone(&progress));
        }

        let db = Arc::clone(&self.db);
        let handle = Arc::clone(&progress);
        let thread = std::thread::spawn(move || {
            let outcome =
                crate::build::build(&db, &package_id, volume, &out, write_listing, &handle)
                    .map_err(|error| error.to_string());
            handle.finish(outcome);
        });
        if let Ok(mut slot) = self.build_thread.lock() {
            // Any handle already here belongs to a run that has ended — `start_build` is only
            // reached past a `build_running` guard — so dropping it detaches a finished thread.
            *slot = Some(thread);
        }
        progress
    }

    /// Starts a build of every volume of a package on a thread of its own.
    ///
    /// [`Self::start_build`]'s arrangement, over `crate::build::build_all`: one progress handle for
    /// the whole run, marked volume 0, so the one-build-at-a-time guard covers it and a stop ends it.
    pub fn start_build_all(
        &self,
        package_id: String,
        folder: Option<PathBuf>,
        write_listing: bool,
    ) -> Arc<BuildProgress> {
        let progress = Arc::new(BuildProgress::new(&package_id, 0));
        if let Ok(mut slot) = self.build.lock() {
            *slot = Some(Arc::clone(&progress));
        }

        let db = Arc::clone(&self.db);
        let handle = Arc::clone(&progress);
        let thread = std::thread::spawn(move || {
            let outcome = crate::build::build_all(
                &db,
                &package_id,
                folder.as_deref(),
                write_listing,
                &handle,
            )
            .map_err(|error| error.to_string());
            handle.finish_all(outcome);
        });
        if let Ok(mut slot) = self.build_thread.lock() {
            *slot = Some(thread);
        }
        progress
    }

    /// Asks a running build to stop, and waits for it to put down what it is holding.
    ///
    /// Returns whether there was one to stop. The wait is bounded by one song rather than by the
    /// whole run: `km_pack::build` asks the callback between songs, so the worst case is a single
    /// ffmpeg re-encode rather than a package of them.
    ///
    /// Nothing half-written survives: the archive goes through a temporary file and a rename, so a
    /// build stopped before that leaves the previous package exactly as it was.
    pub fn stop_build(&self) -> bool {
        let running = self.build_running();
        if let Ok(slot) = self.build.lock()
            && let Some(progress) = slot.as_ref()
        {
            progress.ask_to_stop();
        }
        let thread = self
            .build_thread
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(thread) = thread {
            let _ = thread.join();
        }
        running
    }
}

/// Stops both workers and checkpoints the database, in that order.
///
/// **Both, and before the checkpoint.** A build holds no lock for most of its life, which is what
/// makes it useful and also what makes it dangerous here: `db.lock()` below would succeed *while a
/// build was still running*, close the connection under it, and leave the build's final
/// `record_build` writing to a database that has been checkpointed and closed. The package on disk
/// and what the database says was built would then disagree, with nothing to notice it. Stopping
/// costs at most one song, because the build asks whether to stop between them.
///
/// **In `Drop` rather than in whatever code decided to close the folder**, because a swap only
/// replaces the slot: requests already in flight still hold an `Arc<Workspace>` and are still using
/// this database. Checkpointing from the swapping thread would run `wal_checkpoint(TRUNCATE)` on a
/// connection another thread is reading through. Here it is guaranteed to be last, when the final
/// reference goes.
///
/// Both steps are best-effort and neither can be reported to anybody — by this point there may be no
/// console and no page waiting on an answer — so they go to the log.
impl Drop for Workspace {
    fn drop(&mut self) {
        if self.stop_scan() {
            tracing::info!(
                "stopped the scan on {}; what it had read is written",
                self.root.display()
            );
        }
        if self.stop_build() {
            tracing::info!(
                "stopped the build on {}; the package it was replacing is untouched",
                self.root.display()
            );
        }
        // **Before the checkpoint below, because a checkpoint cannot truncate past a reader.**
        // `wal_checkpoint(TRUNCATE)` needs to be the only connection on the file; with this one still
        // open it folds the pages back but leaves the journal at its full size, which is the one
        // outcome `Db::prepare` goes to trouble to avoid. Dropping it here closes it unless a request
        // is still reading through it, which is the same best-effort the writer's own close makes.
        drop(self.reader.take());
        // `Shared::lock` recovers a poisoned mutex rather than refusing it, which is the right
        // answer here for the reason it gives: a panic elsewhere that happened to be holding this
        // does not make the file untrustworthy, and the journal is better folded back than left.
        let pages = self.db.lock().close();
        if pages > 0 {
            tracing::debug!(
                "wrote {pages} page(s) of journal back into {}",
                self.root.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Scratch;

    /// Creating a database costs about the same on a spawned thread as on the main one.
    ///
    /// Written while chasing what looked like a hang in the Open page's job: the `Db::create` that
    /// returns instantly from `main` appeared to take three quarters of a minute from the thread the
    /// job runs on. It turned out to be the filesystem rather than the threading — a brand-new
    /// database file is scanned by the system before it can be written to, and a second run of the
    /// same thing took a fifth of the time — but the two are pinned together here so that a real
    /// divergence is a test failure rather than another afternoon.
    #[test]
    fn creating_a_database_off_the_main_thread_is_not_slower() {
        let scratch = Scratch::new("thread-timing");
        let folder = scratch.0.clone();
        std::fs::create_dir_all(folder.join("main")).expect("temp folder");
        std::fs::create_dir_all(folder.join("spawned")).expect("temp folder");

        let here = folder.join("main");
        let started = std::time::Instant::now();
        drop(crate::db::Db::create(&here).expect("create on this thread"));
        let on_main = started.elapsed();

        let there = folder.join("spawned");
        let elsewhere = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            drop(crate::db::Db::create(&there).expect("create on a spawned thread"));
            started.elapsed()
        })
        .join()
        .expect("the thread finished");

        // Deliberately generous. This is a wall-clock assertion on a machine that is also compiling
        // things, and the point is to catch a difference of *seconds* — which is what was seen —
        // rather than to police jitter.
        assert!(
            elsewhere < on_main + std::time::Duration::from_secs(5),
            "creating took {on_main:?} on the main thread and {elsewhere:?} on a spawned one"
        );
    }

    /// Closing a workspace stops its scan before it checkpoints.
    ///
    /// The ordering is the whole reason `Drop` exists on this type: a scan writes through the same
    /// connection the checkpoint runs on, so checkpointing first would race a thread that is still
    /// committing batches. Dropping is also the *only* way to close a folder, which is what makes it
    /// impossible to swap corpora and leave a scan writing into the one that is no longer open.
    #[test]
    fn dropping_a_workspace_stops_its_scan() {
        let scratch = Scratch::new("drop-stops-scan");
        let folder = scratch.0.clone();

        let workspace = Workspace::new(crate::db::Db::create(&folder).expect("create"));
        let progress = workspace.start_scan(ScanOptions::default());
        drop(workspace);

        // The handle is joined by `stop_scan`, so by the time the drop has returned the run is over.
        // No sleeping and no polling, which is what keeps this from being a flake.
        assert!(
            progress.snapshot().finished,
            "the scan should have been stopped and joined by the drop"
        );
    }

    /// And its build, for a reason that is worse than the scan's if it is got wrong.
    ///
    /// A build deliberately holds no lock for most of its life, so `Drop`'s `db.lock()` would
    /// succeed *while one was still running*, close the connection under it, and leave the build's
    /// final `record_build` writing into a database that has been checkpointed and closed. The
    /// package on disk and what the database says was built would then disagree, with nothing
    /// anywhere to notice it.
    #[test]
    fn dropping_a_workspace_stops_its_build() {
        let scratch = Scratch::new("drop-stops-build");
        let folder = scratch.0.clone();

        let workspace = Workspace::new(crate::db::Db::create(&folder).expect("create"));
        // No such package, so the build fails immediately -- which is fine and is not what is being
        // tested. What is being tested is that the drop joined the thread rather than walking away
        // from it, and a run that ends by failing exercises that exactly as well as one that ends by
        // succeeding.
        let progress =
            workspace.start_build("nothing".to_owned(), 1, folder.join("nothing.kmpkg"), false);
        drop(workspace);

        assert!(
            progress.snapshot().finished,
            "the build should have been stopped and joined by the drop"
        );
    }

    /// A folder on disk is read through a connection of its own.
    #[test]
    fn a_folder_on_disk_gets_a_reading_connection() {
        let scratch = Scratch::new("reader-on-disk");
        let workspace = Workspace::new(crate::db::Db::create(&scratch.0).expect("create"));

        assert!(
            !Arc::ptr_eq(workspace.reader(), &workspace.db),
            "a page must not be drawn through the connection a scan holds"
        );
    }

    /// One that cannot have a second connection is read through the writing one.
    ///
    /// **The fallback, and it is reached by every other test in the crate**: an in-memory database
    /// has no journal to put into WAL, and WAL is what makes two connections safe. So the answer has
    /// to be the connection that exists rather than a refusal to open the folder.
    #[test]
    fn a_folder_that_cannot_have_a_second_connection_is_read_through_the_first() {
        let workspace =
            Workspace::new(crate::db::Db::open_in_memory(Path::new("/corpus")).expect("in memory"));

        assert!(
            Arc::ptr_eq(workspace.reader(), &workspace.db),
            "with no second connection there is one connection, not an error"
        );
    }
}
