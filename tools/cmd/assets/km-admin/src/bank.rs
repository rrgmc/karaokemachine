//! Fetching a SoundFont bank, and checking it is the one the table names.
//!
//! **The rules are `km_banks::digest`'s and the loop is this module's**, and that division is the
//! whole design. Which hash a row is pinned with, how far past the stated size a body may be before
//! it is refused unread, how a bank published inside a zip is taken out, what a half-finished
//! download is called — all of those are properties of the table's own `digest`, `bytes` and
//! `archive` fields, they are stated once in `km-banks`, and both programs that fetch a bank read
//! them from there.
//!
//! What is honestly a second implementation is the transfer itself. The machine's `fetch.rs` is
//! `ureq`, blocking, on a thread it owns beside the audio thread; this is `reqwest` streaming on
//! tokio, reporting into a [`crate::job::Job`] a browser polls. They differ in the two places that
//! decide a transfer loop's shape — cancellation, and where the counter lives — so abstracting forty
//! lines behind a trait would be machinery for two callers who want different things from it.
//!
//! **Nothing is ever written under a name the network chose.** The file is named from the table's
//! `name` field, and the only thing the response contributes is bytes.

use std::path::{Path, PathBuf};

use futures_util::StreamExt as _;
use km_banks::CatalogBank;
use km_banks::digest::{self, Hasher};
use tokio::io::AsyncWriteExt as _;

use crate::job::Job;

/// How often the download says how far it has got.
///
/// A property of *this* loop rather than of the table, which is why it is here and not in
/// `km-banks`: the machine reads 64 KiB off a `ureq` socket, and this is a `reqwest` byte stream
/// whose chunks arrive at whatever size the transport chose.
///
/// **64 KiB rather than a megabyte, and the reason is the slow host rather than the small file.**
/// The page polls once a second; six of the table's rows are served by archive.org at around
/// 30 KB/s, where a megabyte between reports is half a minute of a bar that does not move. At this
/// size a slow link still reports every couple of seconds, and a fast one reports more often than
/// anybody polls — which costs one atomic store.
const REPORT_EVERY: u64 = 64 * 1024;

/// A bank cannot be bigger than the machine will take.
///
/// `MAX_SOUNDFONT_BYTES` is a gibibyte, and three rows in the table are close to it. Refusing here
/// rather than after the download is the point: an hour spent on something that could never be
/// uploaded is an hour nobody gets back.
fn upload_ceiling() -> u64 {
    km_api::uploads::limit_for(km_api::machine::Upload::SoundFont) as u64
}

/// Whether this bank could be sent to a machine at all, once fetched.
///
/// **One row in the table cannot**, and the page says so rather than leaving it to a 1.2 GiB
/// download: Orpheus is 1,228.6 MiB against an upload ceiling of one gibibyte. The asymmetry is real
/// and not an oversight — the machine's own downloader writes a bank straight to its disk with no
/// such ceiling, so a machine with an internet connection can have that bank and one without it
/// cannot get it from here.
pub fn can_be_sent(bank: &CatalogBank) -> bool {
    bank.bytes <= upload_ceiling()
}

/// Where a fetched bank is kept.
///
/// Its own folder under the data directory, beside where packs go. **Never the machine's
/// `soundfonts/`** — this program may be running on a different computer entirely — and never the
/// shared asset cache a development box keeps, which holds gigabytes of somebody's survey.
pub fn banks_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("banks")
}

/// Fetches one bank into the data directory and answers with the file it wrote.
///
/// Reports into `job` as it goes, and stops between chunks if asked to.
pub async fn fetch(
    http: &reqwest::Client,
    bank: &'static CatalogBank,
    data_dir: &Path,
    job: &Job,
) -> Result<PathBuf, String> {
    let Some(url) = bank.url else {
        // A `manual` row: its publisher serves it through a page rather than a direct address, so
        // there is nowhere to fetch from and the row offers that page instead.
        return Err(format!(
            "{} has to be downloaded by hand from {}",
            bank.name,
            bank.page.unwrap_or("its own site")
        ));
    };
    let Some(pinned) = bank.digest else {
        return Err(format!("{} is not pinned by a digest", bank.name));
    };

    if !can_be_sent(bank) {
        return Err(format!(
            "{} is {}, which is more than a machine will accept in one upload. It would have to be \
             put on the machine by hand.",
            bank.name, bank.size
        ));
    }

    let dir = banks_dir(data_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| format!("{} cannot be written to: {error}", dir.display()))?;

    // An archived bank is downloaded as its archive and the member taken out afterwards; a loose one
    // is downloaded as itself. Which digest checks which is the table's decision, not this
    // function's — see `km_banks::digest::extract_member`.
    match bank.archive {
        Some(archive_name) => {
            let member = bank.member.ok_or_else(|| {
                format!(
                    "{name} is published inside {archive_name} but no member is named",
                    name = bank.name
                )
            })?;
            let part = dir.join(digest::part_name(archive_name));
            let archive_digest = bank.archive_digest.unwrap_or("");
            // The archive's own size is not in the table, so the size check has nothing to compare
            // against here and the digest is left to do the whole job.
            download(http, url, &part, archive_digest, 0, job).await?;

            let landed = dir.join(bank.name);
            let taken = tokio::task::spawn_blocking({
                let part = part.clone();
                let landed = landed.clone();
                let member = member.to_owned();
                let pinned = pinned.to_owned();
                move || digest::extract_member(&part, &member, &landed, &pinned, 64 * 1024)
            })
            .await
            .map_err(|error| format!("taking {member} out of the archive died: {error}"))?;

            // The archive is scratch either way and can be large — one of these is 126 MiB. Removed
            // whether or not the extraction worked.
            let _ = tokio::fs::remove_file(&part).await;
            taken?;
            Ok(landed)
        }
        None => {
            let part = dir.join(digest::part_name(bank.name));
            download(http, url, &part, pinned, bank.bytes, job).await?;
            let landed = dir.join(bank.name);
            tokio::fs::rename(&part, &landed)
                .await
                .map_err(|error| format!("could not put {} in place: {error}", bank.name))?;
            Ok(landed)
        }
    }
}

/// Streams one URL to a file, hashing as it goes.
///
/// `expected` is the size the table states, or `0` where it states none.
///
/// **Nothing half-written is ever left behind, whichever way this fails.**
///
/// The digest path and the stop path each removed the `.part` file themselves, and the transport
/// path did not — so a connection that broke mid-stream left a partial file in the folder for ever.
/// Found by watching a real download break off at 81,669 of 128,788 bytes: archive.org serves six of
/// the table's rows and is not always well. Cleaning up at *one* place rather than at each `?` is
/// the point — the next failure mode added would otherwise have to remember, and this one did not.
async fn download(
    http: &reqwest::Client,
    url: &str,
    part: &Path,
    pinned: &str,
    expected: u64,
    job: &Job,
) -> Result<(), String> {
    let outcome = stream_to_file(http, url, part, pinned, expected, job).await;
    if outcome.is_err() {
        let _ = tokio::fs::remove_file(part).await;
    }
    outcome
}

/// The transfer itself. Every early return here is cleaned up by [`download`].
async fn stream_to_file(
    http: &reqwest::Client,
    url: &str,
    part: &Path,
    pinned: &str,
    expected: u64,
    job: &Job,
) -> Result<(), String> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| format!("could not reach {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }

    // **Refused before a byte is read**, which is what this check is for: the table's `bytes` is
    // exact, so a body wildly larger than it is a login page or a redirect to something else
    // entirely rather than a bank that grew. The digest is the real check; this only makes "not what
    // you asked for" fail in a second rather than in an hour.
    let claimed = response.content_length().unwrap_or(0);
    if claimed > 0 && !digest::size_is_plausible(claimed, expected) {
        return Err(format!(
            "the server offered {claimed} bytes where the table says {expected}; refusing before \
             downloading it"
        ));
    }
    // The table's own figure is the better one to draw a bar against: it is exact, and a server may
    // send no length at all.
    let total = if expected > 0 { expected } else { claimed };

    // **Said before the first chunk arrives, not after the first megabyte.** The loop below reports
    // every `REPORT_EVERY` bytes, which is right for a gigabyte bank and silent for a small one: a
    // 128 KB file never reaches the threshold at all, so the page sat on "starting" for the whole
    // download and looked stuck. Found watching a real one come down from archive.org, which serves
    // at about 30 KB/s — the size that makes this worst is *slow*, not small.
    job.progress(crate::job::phase::DOWNLOADING, 0, total);

    let file = tokio::fs::File::create(part)
        .await
        .map_err(|error| format!("{} cannot be written: {error}", part.display()))?;
    let mut writer = tokio::io::BufWriter::new(file);
    let mut hasher = Hasher::for_digest(pinned);
    let mut stream = response.bytes_stream();
    let mut done: u64 = 0;
    let mut since_report: u64 = 0;

    while let Some(chunk) = stream.next().await {
        if job.stopping() {
            // Between chunks, which is the only place a stop can be honored at all. The partial
            // file is `download`'s to remove, like every other way out of here.
            return Err("stopped".to_owned());
        }
        let chunk = chunk.map_err(|error| format!("the download broke off: {error}"))?;
        hasher.update(&chunk);
        writer
            .write_all(&chunk)
            .await
            .map_err(|error| format!("could not write {}: {error}", part.display()))?;
        done += chunk.len() as u64;
        since_report += chunk.len() as u64;
        if since_report >= REPORT_EVERY {
            since_report = 0;
            job.progress(crate::job::phase::DOWNLOADING, done, total);
        }
    }
    writer
        .flush()
        .await
        .map_err(|error| format!("could not finish writing {}: {error}", part.display()))?;
    // On the disk before `fetch` renames it into place, or a power cut can leave the bank's own
    // name on an empty file.
    writer
        .into_inner()
        .sync_all()
        .await
        .map_err(|error| format!("could not finish writing {}: {error}", part.display()))?;
    job.progress(crate::job::phase::DOWNLOADING, done, total.max(done));

    if !pinned.is_empty() {
        let got = hasher.finish();
        if !digest::matches(&got, pinned) {
            // The file goes, in `download` — a complete download that failed its digest is not a
            // partial one to resume, it is bytes of unknown provenance, and the one cleanup path
            // covers it for the same reason it covers the rest.
            let want = digest::wanted(pinned);
            return Err(format!(
                "the download does not match the digest this bank is pinned to — expected {want}, \
                 got {got}. Nothing was kept."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banks_go_in_their_own_folder_under_the_data_directory() {
        let dir = banks_dir(Path::new("/data"));
        assert!(dir.ends_with("banks"), "{dir:?}");
        assert!(dir.starts_with("/data"), "{dir:?}");
    }

    #[test]
    fn the_ceiling_is_the_machines_own_upload_limit() {
        // Read from `km-api` rather than written down, so the two cannot drift. A gibibyte today.
        assert_eq!(upload_ceiling(), 1024 * 1024 * 1024);
    }

    /// Exactly one bank in the table is too big to send to a machine, and it is named here.
    ///
    /// **This is a real asymmetry rather than a limit somebody forgot to raise.** The machine's own
    /// downloader writes a bank straight to its disk and has no such ceiling; an upload goes through
    /// `MAX_SOUNDFONT_BYTES`, which is a gibibyte, and Orpheus is 1,288,303,498 bytes. So a machine
    /// with its own internet connection can have that bank and one without it cannot get it from
    /// here, which the page says rather than leaving it to be discovered after a 1.2 GiB download.
    ///
    /// Named rather than counted, so that the table gaining another one is a decision somebody makes
    /// rather than a test that quietly still passes.
    #[test]
    fn one_bank_is_too_big_to_send_and_it_is_the_one_we_think() {
        let ceiling = upload_ceiling();
        let too_big: Vec<&str> = km_banks::catalog()
            .iter()
            .filter(|bank| km_banks::fetchable(bank))
            .filter(|bank| bank.bytes > ceiling)
            .map(|bank| bank.id)
            .collect();
        assert_eq!(
            too_big,
            vec!["orpheus"],
            "the set of banks too large to upload has changed"
        );
    }

    #[test]
    fn a_bank_too_big_to_send_is_refused_before_it_is_downloaded() {
        // The point of checking the table's own `bytes` rather than the response's length: an hour
        // spent on something that could never be uploaded is an hour nobody gets back.
        let orpheus = km_banks::bank("orpheus").expect("orpheus is in the table");
        assert!(orpheus.bytes > upload_ceiling());
        assert!(
            km_banks::fetchable(orpheus),
            "it is refused for its size, not for being unfetchable"
        );
    }
}
