//! Finding the same photograph twice.
//!
//! Stock APIs return the same sunset under eight different search terms, so this stage routinely
//! drops a fifth of the candidates. Two passes, because there are two kinds of "same":
//!
//! * **The same bytes**, from the same URL fetched under two terms — settled by SHA-256.
//! * **The same picture, re-encoded or re-cropped** — settled by a perceptual hash and a Hamming
//!   distance. This is the one that matters: a pack of a hundred wallpapers where six are the same
//!   lake looks careless in a way no single image does.
//!
//! The clustering is the naive O(n²) comparison. At five thousand candidates that is 12.5 million
//! 64-bit XORs and a popcount each, which is milliseconds — a BK-tree would be faster asymptotically
//! and slower to read, and nothing here is asymptotic.

use std::collections::BTreeMap;

/// One candidate, as deduplication sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Provider-scoped identity, `provider:id`.
    pub key: String,
    /// Digest of the original bytes.
    pub sha256: String,
    /// Perceptual hash.
    pub phash: u64,
}

/// A group of candidates that are the same photograph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    /// The one to keep — decided by the caller, which is what knows the scores.
    pub keep: String,
    /// The others, recorded so the manifest can say what was dropped and why.
    pub dropped: Vec<String>,
}

/// Groups candidates, keeping whichever member `rank` scores highest.
///
/// `rank` rather than a score field: the ranking is the selection stage's business, and threading its
/// weights through here would put two ideas in one module. Ties fall to the lexicographically first
/// key, so a run is reproducible.
///
/// **What it guarantees**: no two kept keys are within `max_distance` of each other. That is the
/// whole point of the stage, and it holds only because clusters are seeded in score order — see the
/// comment on the sort below.
pub fn cluster(
    candidates: &[Candidate],
    max_distance: u32,
    rank: impl Fn(&str) -> f32,
) -> Vec<Cluster> {
    // Exact duplicates first: they are free to find, and collapsing them shrinks the quadratic pass.
    let mut by_digest: BTreeMap<&str, Vec<&Candidate>> = BTreeMap::new();
    for candidate in candidates {
        by_digest
            .entry(&candidate.sha256)
            .or_default()
            .push(candidate);
    }

    // One representative per digest, then perceptual clustering over those.
    let mut representatives: Vec<&Candidate> = Vec::with_capacity(by_digest.len());
    let mut exact_extras: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (_, mut group) in by_digest {
        group.sort_by(|a, b| best_first(rank(&a.key), rank(&b.key), &a.key, &b.key));
        let (first, rest) = group.split_first().expect("a group holds at least one");
        representatives.push(first);
        exact_extras.insert(
            first.key.as_str(),
            rest.iter().map(|other| other.key.clone()).collect(),
        );
    }
    // Best first, so each seed is the image its cluster will keep.
    //
    // Seeding in key order and *then* promoting the best member breaks the one invariant this stage
    // exists to provide: a seed only ever absorbs images within `max_distance` of itself, so two
    // seeds are always further apart than that — but two promoted members need not be. A real run
    // shipped six wallpapers of which five pairs were inside the threshold, one pair four bits
    // apart: the stage dropped 1,602 images as duplicates and then shipped duplicates. Seeding in
    // score order keeps the same image and makes the guarantee hold.
    representatives.sort_by(|a, b| best_first(rank(&a.key), rank(&b.key), &a.key, &b.key));

    let mut assigned = vec![false; representatives.len()];
    let mut clusters = Vec::new();
    for index in 0..representatives.len() {
        if assigned[index] {
            continue;
        }
        assigned[index] = true;
        let keep = representatives[index];
        let mut members = vec![keep];
        for other in (index + 1)..representatives.len() {
            if assigned[other] {
                continue;
            }
            if distance(keep.phash, representatives[other].phash) <= max_distance {
                assigned[other] = true;
                members.push(representatives[other]);
            }
        }

        let (_, rest) = members.split_first().expect("a cluster holds at least one");
        let mut dropped: Vec<String> = rest.iter().map(|member| member.key.clone()).collect();
        // Every byte-identical copy of every member is dropped too, whichever member it hung off.
        for member in &members {
            if let Some(extras) = exact_extras.get(member.key.as_str()) {
                dropped.extend(extras.iter().cloned());
            }
        }
        dropped.sort();
        dropped.dedup();
        clusters.push(Cluster {
            keep: keep.key.clone(),
            dropped,
        });
    }
    clusters
}

/// Hamming distance between two perceptual hashes.
pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Higher score first, then lexicographic key, so ordering never depends on iteration order.
fn best_first(a_score: f32, b_score: f32, a_key: &str, b_key: &str) -> std::cmp::Ordering {
    b_score.total_cmp(&a_score).then_with(|| a_key.cmp(b_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(key: &str, sha: &str, phash: u64) -> Candidate {
        Candidate {
            key: key.to_owned(),
            sha256: sha.to_owned(),
            phash,
        }
    }

    #[test]
    fn byte_identical_copies_collapse_to_one() {
        let candidates = [
            candidate("pixabay:1", "aaaa", 0x0f0f_0f0f_0f0f_0f0f),
            candidate("pixabay:2", "aaaa", 0x0f0f_0f0f_0f0f_0f0f),
        ];
        let clusters = cluster(&candidates, 0, |_| 1.0);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].keep, "pixabay:1");
        assert_eq!(clusters[0].dropped, vec!["pixabay:2"]);
    }

    #[test]
    fn the_same_photograph_re_encoded_lands_in_one_cluster() {
        // A JPEG at q60 and the same picture at q95: a handful of hash bits differ, no more.
        let original =
            0b1010_1010_1010_1010_1010_1010_1010_1010_1010_1010_1010_1010_1010_1010_1010_1010;
        let recompressed = original ^ 0b111; // three bits
        let candidates = [
            candidate("pixabay:1", "aaaa", original),
            candidate("pexels:9", "bbbb", recompressed),
        ];
        let clusters = cluster(&candidates, 10, |_| 1.0);
        assert_eq!(clusters.len(), 1, "one photograph, two sources");
        assert_eq!(clusters[0].dropped.len(), 1);
    }

    #[test]
    fn two_different_photographs_stay_apart() {
        let candidates = [
            candidate("pixabay:1", "aaaa", 0x0000_0000_0000_0000),
            candidate("pixabay:2", "bbbb", 0xFFFF_FFFF_FFFF_FFFF),
        ];
        let clusters = cluster(&candidates, 10, |_| 1.0);
        assert_eq!(clusters.len(), 2, "64 bits apart is not the same lake");
    }

    #[test]
    fn the_highest_scoring_member_is_the_one_kept() {
        let candidates = [
            candidate("pixabay:small", "aaaa", 0),
            candidate("pixabay:large", "bbbb", 1),
        ];
        let clusters = cluster(&candidates, 10, |key| {
            if key.ends_with("large") { 9.0 } else { 1.0 }
        });
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].keep, "pixabay:large");
        assert_eq!(clusters[0].dropped, vec!["pixabay:small"]);
    }

    #[test]
    fn an_exact_copy_of_a_near_duplicate_is_still_dropped() {
        // Three sightings: one photograph, one re-encode of it, and a byte-identical copy of the
        // re-encode. All three must end in one cluster, or the pack ships the same lake twice.
        let candidates = [
            candidate("pixabay:1", "aaaa", 0),
            candidate("pexels:2", "bbbb", 3),
            candidate("pexels:3", "bbbb", 3),
        ];
        let clusters = cluster(&candidates, 10, |_| 1.0);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].dropped.len(), 2, "{:?}", clusters[0]);
    }

    #[test]
    fn ties_break_the_same_way_every_run() {
        let candidates = [
            candidate("pixabay:b", "bbbb", 0),
            candidate("pixabay:a", "aaaa", 1),
        ];
        let first = cluster(&candidates, 10, |_| 1.0);
        let reversed: Vec<_> = candidates.iter().rev().cloned().collect();
        let second = cluster(&reversed, 10, |_| 1.0);
        assert_eq!(first, second, "input order must not decide the answer");
        assert_eq!(first[0].keep, "pixabay:a");
    }

    /// The invariant the whole stage exists to provide.
    ///
    /// A chain of hashes each a few bits from the last, with scores that rise along the chain — so
    /// the best member of a cluster is never its lexicographically first. Seeding by key and then
    /// promoting the best member satisfies every other test in this module and still ships two
    /// images it would itself call duplicates.
    #[test]
    fn no_two_kept_images_are_near_each_other() {
        // Four photographs six bits apart along a line, so a and b cluster and c and d cluster, but
        // b and c — the two the scores promote — are themselves only six bits apart.
        let candidates = [
            candidate("pixabay:a", "sha_a", 0x0_0000),
            candidate("pixabay:b", "sha_b", 0x0_003F),
            candidate("pixabay:c", "sha_c", 0x0_0FFF),
            candidate("pixabay:d", "sha_d", 0x3_FFFF),
        ];
        assert_eq!(distance(candidates[1].phash, candidates[2].phash), 6);

        // b and c score highest, so each is the member its cluster would promote.
        let clusters = cluster(&candidates, 10, |key| {
            if key.ends_with('b') || key.ends_with('c') {
                9.0
            } else {
                1.0
            }
        });

        let kept: Vec<&Candidate> = clusters
            .iter()
            .map(|cluster| {
                candidates
                    .iter()
                    .find(|candidate| candidate.key == cluster.keep)
                    .expect("a kept key is one of the candidates")
            })
            .collect();

        for (index, one) in kept.iter().enumerate() {
            for other in &kept[index + 1..] {
                let apart = distance(one.phash, other.phash);
                assert!(
                    apart > 10,
                    "{} and {} are both kept but only {apart} bits apart: the pack ships the same \
                     photograph twice",
                    one.key,
                    other.key
                );
            }
        }
    }

    #[test]
    fn distance_is_the_number_of_differing_bits() {
        assert_eq!(distance(0, 0), 0);
        assert_eq!(distance(0b1011, 0b0010), 2);
        assert_eq!(distance(u64::MAX, 0), 64);
    }
}
