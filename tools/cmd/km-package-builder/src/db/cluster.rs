//! Grouping suggested pairs into clusters, and choosing which file of one a curator sees.
//!
//! **A pair is not the unit of work, and treating it as one is what made the suggestion list
//! unusable.** On the corpus this was measured against, 28,575 pairs are 13,421 clusters — the same
//! finding restated twice over, because one song paired with five others is five rows saying one
//! thing. Nothing here asks about a pair.
//!
//! **Nor does anything here ask.** Of those clusters, 89% have their top suitability tied: the files
//! are equally good and any of them will do. Where they are not tied the ordering below already
//! knows which is better. So a cluster gets a representative and the rest are set aside, and the
//! only judgment left for a person is the one the machine cannot make — *these are not the same song
//! at all*, which is a dismissal on the pair that split the cluster.
//!
//! **What is set aside is set aside in [`songs.duplicate_of`], never in `merged_into`.** Both hide a
//! song from browsing and only one of them is somebody's word. See the column's own note in
//! `schema.sql`.

use std::collections::{HashMap, HashSet};

use super::*;

/// How a cluster's representative is chosen, as SQL ordering.
///
/// **Suitability first, then copies, then id.** Suitability is what the tool already believes about
/// a file and it separates a broken one from a working one; where it ties — which is most of the
/// time — the file the world has more copies of is the one that was traded, and taking it means a
/// curator sees the version they are most likely to meet elsewhere. The id last is not a preference
/// but a promise: without it two equally good, equally copied files would swap places between runs,
/// and a representative that moves is a browse list that reshuffles for no reason anybody can see.
///
/// A NULL suitability sorts last rather than as zero, the rule the rest of this database follows:
/// a file nobody could score and a file scored unusable are different facts.
const BEST_FIRST: &str = "suitability DESC NULLS LAST, file_count DESC, id ASC";

impl Db {
    /// Groups every undismissed pair into clusters and writes `duplicate_of` and `version_count`.
    ///
    /// **Union-find over the pairs, in memory.** The alternative is a recursive CTE per song, and
    /// the whole point of this pass is that it runs once for the corpus rather than once per row.
    /// Hundreds of thousands of ids and 28,575 edges is a few megabytes and a few milliseconds.
    ///
    /// **Only `verdict IS NULL` pairs join a cluster.** That is what makes dismissing a pair mean
    /// something: it splits the cluster the next time this runs, and the songs it separated come
    /// back into the browse list. A pair verdicted `same` is already a merge and its song is hidden
    /// by `merged_into` instead, which is why merged songs are left out entirely below.
    ///
    /// Every cluster is rewritten from nothing on each pass, so a song that has left one — because
    /// its pair was dismissed, or because the fingerprint no longer matches — is released rather
    /// than left hidden by a decision nobody can find.
    pub fn cluster(&mut self) -> Result<ClusterCounts, DbError> {
        let mut edges = self.undismissed_pairs()?;
        let apart = self.dismissed_pairs()?;
        // Sorted, so a group does not depend on the order SQLite happened to return its rows in.
        // It matters only once an edge can be refused: which edge is dropped decides which side of
        // a dismissal the rest of a chain falls on, and that must be the same on every pass.
        edges.sort_unstable();

        let mut sets = DisjointSet::default();
        // Before any joining, so no edge is accepted that a later dismissal would have forbidden.
        for (a, b) in &apart {
            sets.keep_apart(a, b);
        }
        for (a, b) in &edges {
            sets.union(a, b);
        }

        // Cluster id to its members. A song with no edge is in no cluster and is never touched.
        let mut clusters: HashMap<usize, Vec<&str>> = HashMap::new();
        for song in sets.members() {
            clusters.entry(sets.find_id(song)).or_default().push(song);
        }

        let transaction = self.conn.transaction()?;
        // Cleared whole rather than diffed. A pass that only wrote what it found would leave a song
        // hidden by a cluster that no longer exists, and nothing on any page would explain why.
        transaction
            .execute_batch("UPDATE songs SET duplicate_of = NULL, version_count = 1 WHERE duplicate_of IS NOT NULL OR version_count <> 1")?;

        let mut cluster_count = 0usize;
        let mut set_aside = 0usize;
        {
            let mut best = transaction.prepare(&format!(
                "SELECT id FROM songs WHERE id IN (SELECT value FROM json_each(?1))
                 ORDER BY {BEST_FIRST} LIMIT 1"
            ))?;
            let mut hide = transaction.prepare(
                "UPDATE songs SET duplicate_of = ?2
                 WHERE id IN (SELECT value FROM json_each(?1)) AND id <> ?2",
            )?;
            let mut count =
                transaction.prepare("UPDATE songs SET version_count = ?2 WHERE id = ?1")?;

            for members in clusters.values() {
                // A cluster whose songs have all been merged away, or deleted by a scan, is not a
                // cluster. `undismissed_pairs` already excludes merged songs, so this catches the
                // narrower case of a pair whose songs went with `forget_missing`.
                if members.len() < 2 {
                    continue;
                }
                let ids = serde_json::to_string(members).unwrap_or_else(|_| "[]".to_owned());
                let Some(representative) = best
                    .query_row(params![&ids], |row| row.get::<_, String>(0))
                    .optional()?
                else {
                    continue;
                };
                set_aside += hide.execute(params![&ids, &representative])?;
                count.execute(params![&representative, members.len() as i64])?;
                cluster_count += 1;
            }
        }
        transaction.commit()?;

        Ok(ClusterCounts {
            clusters: cluster_count as u32,
            set_aside: set_aside as u32,
        })
    }

    /// What the last clustering pass left behind, read back rather than remembered.
    ///
    /// The Duplicates page reports this, and reading it is cheaper than storing it: both are index
    /// scans over a column most rows are NULL in.
    pub fn cluster_counts(&self) -> Result<ClusterCounts, DbError> {
        let clusters: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM songs WHERE version_count > 1",
            [],
            |row| row.get(0),
        )?;
        let set_aside: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM songs WHERE duplicate_of IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(ClusterCounts {
            clusters: clusters as u32,
            set_aside: set_aside as u32,
        })
    }

    /// The other files this one is a version of, best first, itself excluded.
    ///
    /// Answers for a representative and for a song set aside alike: both name the same cluster, one
    /// through its own id and one through `duplicate_of`.
    pub fn versions_of(&self, id: &str) -> Result<Vec<SongRow>, DbError> {
        let representative: Option<String> = self
            .conn
            .query_row(
                "SELECT coalesce(duplicate_of, id) FROM songs WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(representative) = representative else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT {} FROM songs s
             WHERE (s.id = ?1 OR s.duplicate_of = ?1) AND s.id <> ?2
             ORDER BY s.{}",
            browse_columns(),
            BEST_FIRST.replace(", ", ", s."),
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![representative, id], song_row)?;
        let mut rows = rows.collect::<Result<Vec<_>, _>>()?;
        self.fill_tags(&mut rows)?;
        Ok(rows)
    }

    /// Releases one song from its cluster, so it browses on its own again.
    ///
    /// **The song's own row and nothing else.** Clearing the whole cluster would be a second meaning
    /// for one button — *this file is not that recording* and *stop grouping any of these* are
    /// different requests, and only the first is what somebody looking at one song is saying.
    ///
    /// The next clustering pass would put it back, which is correct and is why the button that calls
    /// this sits beside the one that dismisses the pair: releasing is for looking, dismissing is for
    /// good.
    pub fn release_from_cluster(&self, id: &str) -> Result<(), DbError> {
        self.conn
            .execute("UPDATE songs SET duplicate_of = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Every pair somebody has told apart, as ids.
    ///
    /// Read whole rather than per group: a person dismisses a handful over a corpus of hundreds of
    /// thousands, so the list is short and asking for it once is cheaper than asking per candidate
    /// edge. Merged songs are not excluded here -- a dismissal that names one costs a lookup that
    /// finds nothing, where filtering them would cost a join.
    fn dismissed_pairs(&self) -> Result<Vec<(String, String)>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT a_id, b_id FROM duplicate_candidates WHERE verdict = 'different'")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Releases every song set aside behind a representative no list draws. Returns how many.
    ///
    /// **A cluster is written once and read until the next pass, so deleting its representative
    /// takes the whole cluster off the page.** The representative fails `deleted_at IS NULL` and
    /// every member it hides fails `duplicate_of IS NULL`, and the songs that are left go nowhere a
    /// curator can reach. Releasing them shows each copy on its own until the next pass collapses
    /// them behind a live one, which is the module's rule: a song that has left a cluster is
    /// released rather than left hidden by a decision nobody can find.
    ///
    /// Called by the writers that delete, because they are what can empty a cluster's head.
    pub(super) fn release_behind_hidden(&self) -> Result<u32, DbError> {
        let browsable = browsable("r.");
        let released = self.conn.execute(
            &format!(
                "UPDATE songs SET duplicate_of = NULL, version_count = 1
                  WHERE duplicate_of IS NOT NULL
                    AND NOT EXISTS (SELECT 1 FROM songs r
                                     WHERE r.id = songs.duplicate_of AND {browsable})"
            ),
            [],
        )?;
        Ok(released as u32)
    }

    /// Every undismissed pair, as ids, with songs no list draws left out.
    ///
    /// **[`browsable`] on both sides**, which is what keeps a deleted song out of a cluster. It is
    /// asked here as well as in `Db::fingerprints` because a candidate row outlives the pass that
    /// wrote it: a song deleted after the pairs were found is still named by them.
    fn undismissed_pairs(&self) -> Result<Vec<(String, String)>, DbError> {
        let (a, b) = (browsable("a."), browsable("b."));
        let mut statement = self.conn.prepare(&format!(
            "SELECT d.a_id, d.b_id FROM duplicate_candidates d
             JOIN songs a ON a.id = d.a_id
             JOIN songs b ON b.id = d.b_id
             WHERE d.verdict IS NULL AND {a} AND {b}",
        ))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// What a clustering pass found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClusterCounts {
    /// Groups of two or more files that look like one recording.
    pub clusters: u32,
    /// Songs the browse list hides because a better copy of them is in a cluster.
    pub set_aside: u32,
}

/// Union-find over song ids, keyed by index so the ids are stored once.
///
/// **It refuses an edge rather than accepting every one**, which is the whole of what makes a
/// dismissal hold. Joining is transitive and a dismissal is not: telling the tool that `a` and `b`
/// are different recordings says nothing about `c`, so an unconstrained pass rebuilds the group
/// through `c` and the person's answer lasts until the next button press.
#[derive(Default)]
struct DisjointSet<'a> {
    index: HashMap<&'a str, usize>,
    ids: Vec<&'a str>,
    parent: Vec<usize>,
    /// Members of each component, kept only on its root. Groups run to about twenty songs, so
    /// carrying the list costs less than walking every id to rebuild it per edge.
    members: Vec<Vec<usize>>,
    /// Who may not share a component with whom, by index, both ways round.
    apart: HashMap<usize, HashSet<usize>>,
}

impl<'a> DisjointSet<'a> {
    fn intern(&mut self, id: &'a str) -> usize {
        if let Some(&at) = self.index.get(id) {
            return at;
        }
        let at = self.ids.len();
        self.index.insert(id, at);
        self.ids.push(id);
        self.parent.push(at);
        self.members.push(vec![at]);
        at
    }

    /// Records that two songs must never end up in one group.
    fn keep_apart(&mut self, a: &'a str, b: &'a str) {
        let (a, b) = (self.intern(a), self.intern(b));
        self.apart.entry(a).or_default().insert(b);
        self.apart.entry(b).or_default().insert(a);
    }

    /// Path halving, which keeps the tree flat without the second pass full compression needs.
    fn root(&mut self, mut at: usize) -> usize {
        while self.parent[at] != at {
            self.parent[at] = self.parent[self.parent[at]];
            at = self.parent[at];
        }
        at
    }

    /// Joins two songs unless a dismissal forbids it. Answers whether it joined them.
    ///
    /// **Checked against the smaller side.** Every dismissal is stored both ways round, so asking
    /// whether any member of one component is kept apart from any member of the other needs only
    /// one of the two walked — and with nothing dismissed it is one hash miss per member.
    fn union(&mut self, a: &'a str, b: &'a str) -> bool {
        let (a, b) = (self.intern(a), self.intern(b));
        let (ra, rb) = (self.root(a), self.root(b));
        if ra == rb {
            return false;
        }
        if !self.apart.is_empty() {
            let (small, large) = if self.members[ra].len() <= self.members[rb].len() {
                (ra, rb)
            } else {
                (rb, ra)
            };
            for member in &self.members[small] {
                if let Some(forbidden) = self.apart.get(member)
                    && self.members[large]
                        .iter()
                        .any(|other| forbidden.contains(other))
                {
                    return false;
                }
            }
        }
        // The larger list absorbs the smaller, so the copying stays near linear over a whole pass.
        let (keep, gone) = if self.members[ra].len() >= self.members[rb].len() {
            (ra, rb)
        } else {
            (rb, ra)
        };
        let moved = std::mem::take(&mut self.members[gone]);
        self.members[keep].extend(moved);
        self.parent[gone] = keep;
        true
    }

    fn members(&self) -> Vec<&'a str> {
        self.ids.clone()
    }

    fn find_id(&mut self, id: &'a str) -> usize {
        let at = self.intern(id);
        self.root(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_of_pairs_is_one_cluster() {
        let mut sets = DisjointSet::default();
        sets.union("a", "b");
        sets.union("b", "c");
        sets.union("c", "d");
        let root = sets.find_id("a");
        for id in ["b", "c", "d"] {
            assert_eq!(sets.find_id(id), root, "{id} should share a's cluster");
        }
    }

    #[test]
    fn two_untouched_pairs_stay_two_clusters() {
        let mut sets = DisjointSet::default();
        sets.union("a", "b");
        sets.union("c", "d");
        assert_ne!(sets.find_id("a"), sets.find_id("c"));
    }

    #[test]
    fn a_song_in_no_pair_is_in_no_cluster() {
        let mut sets = DisjointSet::default();
        sets.union("a", "b");
        assert_eq!(sets.members(), vec!["a", "b"]);
    }
}
