//! Fetching a SoundFont bank the owner asked for, on a thread of its own.
//!
//! **This is the first thing in the machine that opens an outbound socket**, and the
//! [`Nothing downloads`] decision was rewritten to allow it rather than stretched to cover it. Every
//! constraint that row still carries is enforced here rather than promised:
//!
//! * nothing is fetched at startup, on a timer, or for a song — only when somebody names one bank;
//! * a machine with no network starts, plays and behaves exactly as before, because nothing on any
//!   other path calls into this module;
//! * a failure is a sentence on a screen, never a machine that will not run.
//!
//! The shape is [`crate::dropped::DropInstaller`]'s: a named thread, a status behind a mutex, and a
//! caller that reads it whenever it likes and never blocks on it. One download at a time, because a
//! second one is not a feature anybody asked for and queueing is the part that would need thinking
//! about.
//!
//! [`Nothing downloads`]: https://example.invalid
//! (see `docs/decisions/repository.md#nothing-downloads`)

use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

use km_banks::CatalogBank;
use km_banks::digest::{self, Hasher};

use crate::settings::Paths;

/// How much to read from the socket at a time.
///
/// Big enough that a gigabyte bank is not a million lock acquisitions, small enough that the
/// progress a person is watching moves. At 64 KiB the largest bank in the table reports about
/// sixteen thousand times.
///
/// **This one stayed behind when the rest of the download rules moved to `km-banks`**, and the split
/// is the point: what a row is pinned with is a property of the table, and how big a bite this loop
/// takes out of a socket is a property of this loop. `km-admin` streams the same banks over
/// `reqwest` and has no reason to read in the same units.
const CHUNK: usize = 64 * 1024;

/// What the downloader is doing, as a person would be told.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Fetching {
    /// Nothing has been asked for since the machine started.
    #[default]
    Idle,
    /// A download is running.
    Working {
        /// The bank's id in the table.
        id: String,
        /// What to call it on a screen.
        name: String,
        /// Bytes read so far.
        done: u64,
        /// What the table says the finished file is, so a bar has an end even before the server
        /// says anything about length.
        total: u64,
    },
    /// The last download finished and the bank is on disk.
    Done { id: String, name: String },
    /// The last download did not finish, and why — already worded for a person.
    Failed {
        id: String,
        name: String,
        why: String,
    },
}

impl Fetching {
    /// Whether a download is running now.
    pub fn busy(&self) -> bool {
        matches!(self, Fetching::Working { .. })
    }
}

/// Runs one download at a time and says how it is going.
#[derive(Debug)]
pub struct Downloader {
    state: Arc<Mutex<Fetching>>,
    paths: Paths,
}

impl Downloader {
    pub fn new(paths: Paths) -> Self {
        Self {
            state: Arc::new(Mutex::new(Fetching::Idle)),
            paths,
        }
    }

    /// What the last or current download is doing.
    pub fn status(&self) -> Fetching {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Starts fetching a bank, unless one is already being fetched.
    ///
    /// Returns immediately: the work is on a thread, and the caller reads [`status`](Self::status)
    /// whenever it likes. `Err` is only ever "there is already one running" or "this row cannot be
    /// fetched at all" — a network failure happens later and is reported through the status, because
    /// by then the request that asked for it has long been answered.
    pub fn start(&self, bank: &'static CatalogBank) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.busy() {
            return Err("a SoundFont is already being fetched".to_owned());
        }
        let Some(url) = bank.url else {
            // A `manual` row. Its publisher serves it through a page rather than a direct address,
            // so there is nowhere to fetch it from and the page is what a remote offers instead.
            return Err(format!(
                "{} has to be downloaded by hand from {}",
                bank.name,
                bank.page.unwrap_or("its own site")
            ));
        };
        let Some(digest) = bank.digest else {
            return Err(format!("{} is not pinned by a digest", bank.name));
        };

        *state = Fetching::Working {
            id: bank.id.to_owned(),
            name: bank.name.to_owned(),
            done: 0,
            total: bank.bytes,
        };
        drop(state);

        let shared = Arc::clone(&self.state);
        let target = self.paths.soundfonts_dir();
        let name = bank.name;
        let id = bank.id;
        // The digest checked against what is written: the member's own for an archived bank, the
        // file's for a loose one.
        let archive = bank
            .archive
            .map(|archive| (archive, bank.archive_digest, bank.member));

        std::thread::Builder::new()
            .name("km-soundfont-fetch".to_owned())
            .spawn(move || {
                let outcome = run(&shared, &target, url, digest, name, archive);
                let mut state = shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *state = match outcome {
                    Ok(()) => {
                        tracing::info!(bank = %name, "fetched a SoundFont");
                        Fetching::Done {
                            id: id.to_owned(),
                            name: name.to_owned(),
                        }
                    }
                    Err(why) => {
                        // Warn rather than error: a download that did not finish is a thing that
                        // happens to a machine under a television, and it has changed nothing.
                        tracing::warn!(bank = %name, %why, "could not fetch a SoundFont");
                        Fetching::Failed {
                            id: id.to_owned(),
                            name: name.to_owned(),
                            why,
                        }
                    }
                };
            })
            .map_err(|error| format!("could not start the download: {error}"))?;
        Ok(())
    }
}

/// The download itself, on the worker thread.
fn run(
    shared: &Arc<Mutex<Fetching>>,
    target: &Path,
    url: &str,
    digest: &str,
    name: &str,
    archive: Option<(&str, Option<&str>, Option<&str>)>,
) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|error| format!("{} cannot be written to: {error}", target.display()))?;

    // **Anything left by a download that did not finish goes first**, and this is the only thing
    // that can collect the one case no error path reaches: the process being *killed* mid-download.
    // That is not a remote possibility on the machine this matters most on — a television app that
    // gets backgrounded is killed, and app-private storage is somewhere nobody can go and delete a
    // 126 MiB leftover by hand. Swept here rather than at startup because this is the moment the
    // folder is about to be written to and the moment somebody is watching a result.
    sweep_parts(target);

    match archive {
        None => {
            let part = target.join(format!(".{name}.part"));
            // **Cleaned up on the way out, not only on success.** `download` removes the file when
            // the digest is wrong, and `finish` removes it when the bank will not play — but a
            // network drop and a full disk both left it behind, which is the worst moment to keep a
            // partial copy of a 300 MiB file.
            let result = download(shared, url, &part, digest)
                .and_then(|()| finish(&part, &target.join(name), name));
            if result.is_err() {
                let _ = std::fs::remove_file(&part);
            }
            result
        }
        Some((archive_name, archive_digest, member)) => {
            let member = member.ok_or_else(|| {
                format!("{name} is published inside {archive_name} but no member is named")
            })?;
            let part = target.join(format!(".{archive_name}.part"));
            let extracted = target.join(format!(".{name}.part"));
            // The archive's digest where the table gives one; otherwise the member is the only thing
            // that gets checked, which is the check that matters.
            let result = download(shared, url, &part, archive_digest.unwrap_or(""))
                .and_then(|()| digest::extract_member(&part, member, &extracted, digest, CHUNK))
                .and_then(|()| finish(&extracted, &target.join(name), name));
            // The archive is scratch either way and can be large -- FluidR3's is 126 MiB. Removed
            // whether or not the download reached the end, which the earlier arrangement did not do:
            // `download(...)?` returned before this line and left the archive on disk.
            let _ = std::fs::remove_file(&part);
            if result.is_err() {
                let _ = std::fs::remove_file(&extracted);
            }
            result
        }
    }
}

/// Removes `.part` files a previous download left in the bank folder.
///
/// **Named conservatively on purpose.** The folder is one an owner is invited to drop banks into, so
/// this takes only the exact shape this module writes — a leading dot and a `.part` extension — and
/// leaves everything else alone. A bank is a `.sf2` and cannot match.
fn sweep_parts(target: &Path) {
    let Ok(entries) = std::fs::read_dir(target) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') && name.ends_with(".part") && path.is_file() {
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::info!(file = %path.display(), "removed an unfinished download"),
                Err(error) => {
                    tracing::warn!(file = %path.display(), %error, "could not remove an unfinished download");
                }
            }
        }
    }
}

/// Streams a URL to a file, checking the digest as it goes and reporting progress.
fn download(
    shared: &Arc<Mutex<Fetching>>,
    url: &str,
    part: &Path,
    digest: &str,
) -> Result<(), String> {
    let response = ureq::get(url)
        .call()
        .map_err(|error| format!("could not reach {url}: {error}"))?;

    // What the server says it will send, used only to refuse something obviously wrong before
    // spending an hour on it. The table's byte count is the number a person sees.
    let claimed: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    let expected = match shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        Fetching::Working { total, .. } => total,
        _ => 0,
    };
    if let Some(claimed) = claimed
        && !digest::size_is_plausible(claimed, expected)
    {
        return Err(format!(
            "the server offered {claimed} bytes where the table says {expected}; refusing before \
             downloading it"
        ));
    }

    let file = std::fs::File::create(part)
        .map_err(|error| format!("{} cannot be written: {error}", part.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut reader = response.into_body().into_reader();
    let mut hasher = Hasher::for_digest(digest);
    let mut buffer = vec![0u8; CHUNK];
    let mut done: u64 = 0;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("the download stopped: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        std::io::Write::write_all(&mut writer, &buffer[..read])
            .map_err(|error| format!("could not write {}: {error}", part.display()))?;
        done += read as u64;
        if let Ok(mut state) = shared.lock()
            && let Fetching::Working { done: at, .. } = &mut *state
        {
            *at = done;
        }
    }
    std::io::Write::flush(&mut writer)
        .map_err(|error| format!("could not finish writing {}: {error}", part.display()))?;
    drop(writer);

    if !digest.is_empty() {
        let got = hasher.finish();
        if !digest::matches(&got, digest) {
            let _ = std::fs::remove_file(part);
            let want = digest::wanted(digest);
            return Err(format!(
                "the download does not match the digest this bank is pinned to — expected {want}, \
                 got {got}. Nothing was installed."
            ));
        }
    }
    Ok(())
}

/// Checks the bank plays, then moves it into place under its real name.
///
/// **The play check is before the rename and not after**, which is the whole reason a `.part` file
/// exists: a bank that this synthesizer refuses must never appear in the folder, because the folder
/// is a picker and every name in it is a promise that tapping it will work.
fn finish(part: &Path, final_path: &Path, name: &str) -> Result<(), String> {
    if let Err(error) = crate::soundfont::check_plays(part) {
        let _ = std::fs::remove_file(part);
        return Err(format!(
            "{name} downloaded correctly but will not play: {error}. This machine's synthesizer is \
             stricter than most. Nothing was installed."
        ));
    }
    std::fs::rename(part, final_path).map_err(|error| {
        let _ = std::fs::remove_file(part);
        format!("could not put {name} in place: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two tests that were here about digests and `.part` names have gone to
    // `km_banks::digest`, with the code they were testing. What is left below is about *this*
    // module: the thread, the refusals, and the sweep.

    #[test]
    fn nothing_is_fetched_for_a_manual_row() {
        let paths = Paths::rooted_at(std::env::temp_dir().join("km-fetch-test"));
        let downloader = Downloader::new(paths);
        let arachno = crate::banks::catalog()
            .iter()
            .find(|bank| bank.id == "arachno")
            .expect("arachno is in the table");

        let refusal = downloader
            .start(arachno)
            .expect_err("arachno cannot be fetched");
        assert!(
            refusal.contains("by hand"),
            "it says what to do instead: {refusal}"
        );
        assert!(refusal.contains("arachnosoft.com"), "and where: {refusal}");
        // And nothing started, so the machine is exactly as it was.
        assert_eq!(downloader.status(), Fetching::Idle);
    }

    #[test]
    fn an_unfinished_download_is_swept_and_the_banks_beside_it_are_not() {
        // The case no error path can reach: the process was killed mid-download, so nothing ran to
        // clean up after it. On Android that leftover sits in app-private storage, where there is no
        // file manager to remove it by hand and it is invisible to the picker, which lists `.sf2`.
        let dir = std::env::temp_dir().join("km-fetch-sweep-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");

        let leftover = dir.join(".FluidR3_GM.sf2.part");
        let archive = dir.join(".fluid-soundfont.zip.part");
        let bank = dir.join("GeneralUser-GS.sf2");
        let notes = dir.join("notes.txt");
        for file in [&leftover, &archive, &bank, &notes] {
            std::fs::write(file, b"x").expect("a file");
        }

        sweep_parts(&dir);

        assert!(!leftover.exists(), "the part file went");
        assert!(!archive.exists(), "the archive's part file went too");
        // The folder is one an owner drops banks into, so the sweep has to be narrow: a real bank
        // and a stray file of their own are both left exactly where they were.
        assert!(bank.exists(), "a real bank is untouched");
        assert!(notes.exists(), "and so is anything else in the folder");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
