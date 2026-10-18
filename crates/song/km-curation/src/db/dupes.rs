//! Suggested near-duplicates, and the verdicts a person records on them.
//!
//! Exact duplicates never reach here: a song id *is* the hash of its bytes, so byte-identical files
//! are one row already and [`Db::exact_duplicates`] only lists which. What the rest of this module
//! serves is the harder half -- the same recording re-saved, re-encoded or trimmed, which is
//! different bytes and therefore a different song.
//!
//! **Nothing here merges on its own.** [`Db::store_candidates`] proposes and [`Db::resolve_duplicate`]
//! records what somebody decided, because a wrong merge hides a song and leaves nothing to notice
//! later. A pair called different is kept as a verdict rather than deleted, or reviewing the list
//! would never end.
//!
//! Merging itself is one level deep and the rule lives in `packages.rs`: `b` points at `a`, and
//! anything already pointing at `b` is moved to point at `a` too, so a chain cannot form.

use super::*;

impl Db {
    /// Records that two files are not the same recording after all, and lets the second browse.
    ///
    /// **The only judgment left for a person, and it is made where both files can be heard** — on a
    /// song's own page, beside a play button for each version. Everything else about a group the
    /// tool decides for itself.
    ///
    /// The verdict is what makes it stick: a dismissed pair never joins a group again, so the next
    /// pass leaves these two apart. Clearing `duplicate_of` here as well is what makes the answer
    /// visible now rather than after a whole-corpus pass — and it is only approximately right, since
    /// a third file may still hold the two together until that pass runs.
    pub fn dismiss_pair(&self, a_id: &str, b_id: &str) -> Result<(), DbError> {
        // Stored with the smaller id first, which is how `suggest` writes them.
        let (first, second) = if a_id <= b_id {
            (a_id, b_id)
        } else {
            (b_id, a_id)
        };
        // **Inserted when no pair was suggested, and that is the load-bearing half.** The lyric pass
        // joins a set as a star, so a set of four holds three pairs and neither of the two leaves is
        // paired with the other -- while the song page offers this against every other version in
        // the set. An `UPDATE` alone matched nothing there and answered `Ok`, so the sentence said
        // the two would not be grouped again and nothing had been written down.
        //
        // The similarity is 0 and the reason says what it is: nothing proposed this pair, a person
        // did, and the row exists to record the refusal rather than the proposal.
        self.conn.execute(
            "INSERT INTO duplicate_candidates(a_id, b_id, similarity, reason, verdict)
             VALUES (?1, ?2, 0.0, 'told apart', 'different')
             ON CONFLICT(a_id, b_id) DO UPDATE SET verdict = 'different'",
            params![first, second],
        )?;
        // Cleared so the answer is visible now rather than after a whole-corpus pass. `Db::cluster`
        // is what makes it hold.
        self.conn.execute(
            "UPDATE songs SET duplicate_of = NULL WHERE id IN (?1, ?2)",
            params![a_id, b_id],
        )?;
        Ok(())
    }

    /// Replaces the suggestion list with a freshly computed one, keeping every existing verdict.
    ///
    /// A pair somebody has already called different must never come back, or reviewing the list is
    /// endless.
    ///
    /// **A proposal is the tool's and a verdict is a person's, and only the first is replaced.** An
    /// unverdicted row says this pass's predecessor thought two files alike; the pass running now is
    /// the authority on that, and a row it does not propose again is evidence nothing stands behind
    /// -- which [`Db::cluster`] would otherwise go on grouping by, since it reads every undismissed
    /// pair in the table rather than the ones just written.
    ///
    /// **Clustering is not done here**, though it is the same pass from a caller's side.
    /// [`Db::cluster`] takes its own transaction, and nesting it inside this one would mean a
    /// suggestion list that committed only if the grouping also succeeded -- where the two are
    /// independently useful and the grouping can be run again over a list already stored.
    ///
    /// Answers with the number of pairs stored: what this pass proposed, less whatever part of it
    /// somebody has already passed a verdict on.
    pub fn store_candidates(
        &mut self,
        pairs: &[(String, String, f32, String)],
    ) -> Result<usize, DbError> {
        let transaction = self.conn.transaction()?;
        transaction.execute("DELETE FROM duplicate_candidates WHERE verdict IS NULL", [])?;
        let mut added = 0usize;
        {
            // A row left to conflict with is a verdicted one, and there is nothing to do to it: the
            // similarity and reason that go with a proposal say nothing about a decision somebody
            // has already made, and the verdict is the whole of what the row is kept for.
            let mut insert = transaction.prepare(
                "INSERT INTO duplicate_candidates(a_id, b_id, similarity, reason)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(a_id, b_id) DO NOTHING",
            )?;
            for (a, b, similarity, reason) in pairs {
                added += insert.execute(params![a, b, similarity, reason])?;
            }
        }
        transaction.commit()?;
        Ok(added)
    }

    /// Every song's fingerprint and effective names, for the near-duplicate pass.
    ///
    /// The file-name fallback is deliberately *not* applied here, unlike everywhere a title is shown.
    /// A shape match needs a matching name to become a suggestion, and file names are the one kind of
    /// name a corpus repeats without meaning anything by it — every `TRACK01.mid` under every folder
    /// would confirm every other, and the review queue this is careful to keep small would fill with
    /// pairs that are not the same song at all.
    ///
    /// **[`browsable`] and not `merged_into IS NULL` alone.** A deleted song is one somebody threw
    /// away, so pairing it wastes a place in the review queue — and worse, `Db::cluster` picks a
    /// representative by suitability, so a deleted song can win one and hide every live copy behind
    /// a row no list will draw.
    pub fn fingerprints(&self) -> Result<Vec<Fingerprint>, DbError> {
        let browsable = browsable("");
        let mut statement = self.conn.prepare(&format!(
            "SELECT id, fingerprint, coalesce(title, det_title, ''),
                    coalesce(artist, det_artist, ''), duration_ms, lyrics
             FROM songs WHERE {browsable}",
        ))?;
        let rows = statement.query_map([], |row| {
            Ok(Fingerprint {
                id: row.get(0)?,
                fingerprint: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                title: row.get(2)?,
                artist: row.get(3)?,
                duration_ms: row.get::<_, i64>(4)? as u32,
                // Keyed here rather than carried, so the words themselves are read and dropped one
                // row at a time. A song with none, or with too few to identify it by, keys to NULL
                // and never pairs on them.
                lyric_key: row
                    .get::<_, Option<String>>(5)?
                    .as_deref()
                    .and_then(crate::dupes::lyric_key),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
