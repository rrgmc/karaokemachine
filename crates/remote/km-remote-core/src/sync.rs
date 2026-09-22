//! Copying a machine's catalog onto this phone.
//!
//! One question first, and it is the one that makes this cheap: **has anything changed?**
//! `GET /discover` is always public and transfers nothing, and it carries the machine's instance id
//! and its `catalog_version`. If the mirror already holds that pair, the answer is no and nothing
//! is downloaded — which on a six-figure catalog is the difference between a refresh that costs
//! nothing and one that costs a minute of somebody's evening.
//!
//! When something has changed, the whole catalog is re-read. **Not an incremental merge**: the
//! export says what a machine *has*, not what it has gained, so a song removed by uninstalling a
//! package would otherwise stay in the mirror forever — queueable from this phone and from nowhere
//! else.

use std::sync::{Arc, Mutex};

use km_api::dto::SongDto;
use km_remote_pages::machine::RemoteError;
use km_songcode::SongCode;

use crate::client::Api;
use crate::mirror::Mirror;

/// How many songs to ask for at once.
///
/// The route's own cap is 5,000; this is below it deliberately. A page is held whole in memory on
/// both sides, and on a phone the difference between twenty requests and four is not worth the
/// difference between a two-megabyte allocation and a ten-megabyte one.
const PAGE: usize = 1_000;

/// A guard against paging forever.
///
/// Only reachable if a machine returned a page whose last number was not greater than the one asked
/// after, which would be a bug on the other side — but a phone-facing loop with no bound is a phone
/// that gets hot in somebody's pocket.
const MAX_PAGES: usize = 10_000;

/// What a refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The mirror already held this machine's catalog at this version.
    AlreadyCurrent {
        /// How many songs it holds.
        songs: usize,
    },
    /// The catalog was re-read.
    Imported {
        /// How many songs arrived.
        songs: usize,
        /// The version they are of.
        version: u64,
    },
}

/// What a refresh did, and what it learned on the way.
///
/// The two are separated because they answer different questions. [`Outcome`] is the *work* — it is
/// what the sentence shown to a person is built from — and the name is a fact about the machine that
/// happens to be free here.
///
/// **Free, and that is the argument for taking it from this call rather than from the locator.**
/// `refresh` opens with `GET /discover`, which is public, transfers nothing and already carries the
/// machine's name; a browse-time name would be a snapshot of what the machine was called when it was
/// found, and would go stale the moment somebody renamed it. This one is at most one refresh old,
/// and every path that changes which machine is in hand — connecting, rescanning, the periodic
/// refresh — goes through here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refreshed {
    /// What the refresh did.
    pub outcome: Outcome,
    /// What the machine calls itself, or `None` where it advertises nothing worth showing.
    ///
    /// Already through [`km_api::discover::display_name`], so a caller never has to decide what an
    /// empty name means — a machine whose settings file was edited by hand can put one on the wire
    /// however firmly the writing half refuses it.
    pub name: Option<String>,
    /// The machine's instance id, as it reports it.
    ///
    /// **Taken from the same call and for the same reason the name is**, one paragraph up: it is
    /// already parsed here to ask the mirror whether its copy is of this machine, and was thrown
    /// away one line later. It is what `km_api::discover::known` anchors a record on, and what tells
    /// the difference between *nothing has answered at this address* and *something answered and it
    /// is not ours* — which is the case only an identity can see.
    ///
    /// Empty on a machine too old to have one, which reads as `None` rather than `Some("")`.
    pub id: Option<String>,
}

/// Brings the mirror up to date with a machine, downloading only if it has to.
pub async fn refresh(
    api: &Api,
    mirror: &Arc<Mutex<Mirror>>,
    force: bool,
) -> Result<Refreshed, RemoteError> {
    let discovery = api.discover().await?;
    let version = discovery.catalog_version;
    // Read here rather than at the end: the early return below is the common case by a wide margin,
    // and a name that only came back from a refresh that downloaded something would be absent on
    // almost every call.
    let name = km_api::discover::display_name(&discovery.name).map(str::to_owned);
    let id = Some(discovery.id.clone()).filter(|id| !id.is_empty());

    if !force && let Some(version) = version {
        let current = {
            let guard = mirror
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.is_current(&discovery.id, version)
        };
        if current {
            let songs = held(mirror)?;
            return Ok(Refreshed {
                outcome: Outcome::AlreadyCurrent { songs },
                name: name.clone(),
                id: id.clone(),
            });
        }
    }

    let mut songs: Vec<SongDto> = Vec::new();
    let mut after: Option<SongCode> = None;
    let mut reported: Option<u64> = version;

    for _ in 0..MAX_PAGES {
        let (page, page_version) = api.export(after, PAGE).await?;
        if page_version.is_some() {
            reported = page_version;
        }
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|song| song.number);
        // Defensive, and the only thing standing between a machine misbehaving and a loop that never
        // ends: a page must move the cursor forward.
        if last <= after {
            return Err(RemoteError::Failed(
                "the machine sent a catalog page that did not move forward".to_owned(),
            ));
        }
        after = last;
        let short = page.len() < PAGE;
        songs.extend(page);
        if short {
            break;
        }
    }

    // The version the *export* reported is what gets stored, not the one `/discover` said a moment
    // earlier. If a package was installed halfway through, the two differ, and storing the older one
    // would leave the mirror claiming to be current when it holds a stitched-together catalog.
    let version = reported.unwrap_or_default();
    let count = songs.len();
    // Names and flags only, and a failure here does not cost the songs: a package with no name row is listed
    // on Setup under its id, which is worse than its name and far better than an empty mirror.
    let packages: Vec<(String, String, u32)> = match api.packages().await {
        Ok(list) => list
            .packages
            .into_iter()
            .map(|package| (package.id, package.name, package.flags))
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "package names did not arrive; Setup will list packages by id");
            Vec::new()
        }
    };
    {
        let mut guard = mirror
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .replace(&discovery.id, version, &songs, &packages)
            .map_err(|error| RemoteError::Failed(error.to_string()))?;
    }
    Ok(Refreshed {
        outcome: Outcome::Imported {
            songs: count,
            version,
        },
        name,
        id,
    })
}

fn held(mirror: &Arc<Mutex<Mirror>>) -> Result<usize, RemoteError> {
    mirror
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .count()
        .map_err(|error| RemoteError::Failed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(number: u32) -> SongDto {
        SongDto {
            number: SongCode::new(number),
            title: format!("Song {number}"),
            artist: None,
            language: None,
            kind: km_kmpkg::SongKind::Midi,
            duration_ms: 1000,
            suitability: None,
            melody_available: false,
            default_transpose: 0,
            package_id: "vol1".to_owned(),
            content_hash: Some(format!("{number:032x}")),
            lyric_preview: Vec::new(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn a_mirror_at_the_same_version_of_the_same_machine_needs_no_download() {
        let mut mirror = Mirror::open_in_memory().expect("open");
        mirror
            .replace("machine-1", 4, &[song(1), song(2)], &[])
            .expect("import");
        assert!(mirror.is_current("machine-1", 4));
        assert!(!mirror.is_current("machine-1", 5));
        assert!(!mirror.is_current("machine-2", 4));
    }
}
