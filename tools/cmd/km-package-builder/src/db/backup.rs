//! Reading the hand-made half of a database out, and putting it back.
//!
//! **What a backup carries is what a rescan cannot reproduce**: the titles and artists somebody
//! typed, the suitability they set, the tags, the favorites and what is filed in them. Everything
//! else — the paths, the sizes, the detected metadata — comes back from the corpus for free, so none
//! of it is here.
//!
//! That is why favorites cross this boundary as *names* rather than as ids: a restore lands in a
//! database whose `favorites.id` sequence is its own, and the name is the only thing both sides
//! agree about.
//!
//! [`Db::apply_restore`] is one transaction over the whole plan. `crate::backup` decides what the
//! plan is; this only applies it.

use super::favorites::FavoriteRow;
use super::packages::MERGE_SQL;
use super::*;

impl Db {
    /// The `songs` predicate that means "somebody has said something about this".
    ///
    /// Built from [`crate::backup::HAND_SET_COLUMNS`] rather than typed out, so that a column added
    /// there is a column this finds — the backup's field list is written down once and this is one
    /// of the two places that must not hold a second copy of it.
    ///
    /// The last disjunct is the one that is easy to leave out and the reason it is spelled: a song
    /// somebody has only *favorited* carries nothing in its own row, and is still a song they made
    /// a decision about.
    fn hand_set_predicate() -> String {
        let columns: Vec<String> = crate::backup::HAND_SET_COLUMNS
            .iter()
            .map(|column| format!("{column} IS NOT NULL"))
            .collect();
        format!(
            "{} OR id IN (SELECT song_id FROM song_favorites)",
            columns.join(" OR ")
        )
    }

    /// Every song carrying something a person typed, and no others.
    ///
    /// **The `WHERE` is the point.** On the corpus this tool was measured against, a backup of
    /// every row would be a hundred megabytes of nulls, and the few thousand rows
    /// somebody has actually touched are the whole content of the file.
    ///
    /// Not [`Self::song`] in a loop, which runs three more queries per song for the files, the
    /// favorites and the packages. This is two flat queries against thousands of round trips.
    pub fn hand_set_songs(&self) -> Result<Vec<HandSetSong>, DbError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT id, title, artist, language, lyric_encoding, default_transpose, fixes,
                    user_score, notes, merged_into, melody_chosen, {}
             FROM songs WHERE {} ORDER BY id",
            eff_title(""),
            Self::hand_set_predicate()
        ))?;
        let rows = statement.query_map([], |row| {
            Ok(HandSetSong {
                id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                language: row.get(3)?,
                lyric_encoding: row.get(4)?,
                default_transpose: row.get(5)?,
                fixes: row.get(6)?,
                user_score: row.get(7)?,
                notes: row.get(8)?,
                merged_into: row.get(9)?,
                // Appended rather than slotted in beside `fixes`, because every `row.get(N)` here
                // is positional and an insertion renumbers the lot.
                melody_chosen: row.get(10)?,
                seen_as: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// How many songs [`Self::hand_set_songs`] would return.
    ///
    /// For the Settings panel, which must not offer to back up a whole corpus when the answer is
    /// eleven. The identical predicate, so the number on the page and the file cannot disagree.
    pub fn hand_set_count(&self) -> Result<u32, DbError> {
        let count: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM songs WHERE {}",
                Self::hand_set_predicate()
            ),
            [],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }

    /// Which of these song ids this corpus actually holds.
    ///
    /// One prepared statement executed per id rather than an `IN` list, which would have to be
    /// chunked: SQLite caps the parameters in one statement, a restored backup can carry thousands
    /// of songs, and a chunk size is a number that is right until the day it is not. Each execution
    /// is a seek on the primary key.
    pub fn existing_song_ids(&self, ids: &[String]) -> Result<BTreeSet<String>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT 1 FROM songs WHERE id = ?1 LIMIT 1")?;
        let mut present = BTreeSet::new();
        for id in ids {
            if statement.exists([id])? {
                present.insert(id.clone());
            }
        }
        Ok(present)
    }

    /// Every favorite by name, with whether it is a working list.
    ///
    /// Not [`Self::favorites`], which carries the two counts a page draws and no backup needs.
    pub fn favorite_names(&self) -> Result<Vec<(String, bool)>, DbError> {
        let mut out: Vec<(String, bool)> = self
            .favorites_by_id()?
            .into_values()
            .map(|row| (row.name, row.temporary))
            .collect();
        out.sort();
        Ok(out)
    }

    /// Which favorites each song is filed under, by name.
    ///
    /// The names are read once and joined in Rust rather than per membership in SQL: the lists are
    /// tens of rows and the memberships are thousands, so the join is the expensive half of a cheap
    /// question.
    pub fn favorite_memberships(&self) -> Result<BTreeMap<String, Vec<String>>, DbError> {
        let names = self.favorites_by_id()?;
        let mut statement = self
            .conn
            .prepare("SELECT song_id, favorite_id FROM song_favorites ORDER BY song_id")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in rows {
            let (song_id, favorite_id) = row?;
            if let Some(favorite) = names.get(&favorite_id) {
                out.entry(song_id).or_default().push(favorite.name.clone());
            }
        }
        Ok(out)
    }

    /// The whole favorites table, by id.
    ///
    /// The shared read behind the two above, so the names they each use are one description of them.
    fn favorites_by_id(&self) -> Result<BTreeMap<i64, FavoriteRow>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT id, name, temporary FROM favorites")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                FavoriteRow {
                    name: row.get(1)?,
                    temporary: row.get::<_, i64>(2)? != 0,
                },
            ))
        })?;
        Ok(rows.collect::<Result<BTreeMap<i64, FavoriteRow>, _>>()?)
    }

    /// Writes a whole restore in one transaction.
    ///
    /// **The per-field setters above cannot be used here, and the compiler says so.** A
    /// `rusqlite::Transaction` holds `&mut self.conn` for as long as it lives, so `self.edit_song(..)`
    /// will not borrow; and `self.add_to_package(..)`, which opens a transaction of its own, would
    /// issue a second `BEGIN` that SQLite refuses outright. Rather than make thirty setters
    /// transaction-aware for one caller, the restore is the one write path that prepares its own
    /// statements on the transaction — the arrangement [`Self::write_scanned`] and
    /// [`Self::forget_missing`] already have, for the same reason.
    ///
    /// **The policy travels *into* the SQL as a bound flag** rather than being resolved above it,
    /// which is what makes `coalesce(new, old)` and `coalesce(old, new)` one statement instead of two
    /// write paths free to drift. Neither direction can write NULL: an absent value in a backup means
    /// "nobody said", so `coalesce` with the column itself is the whole of the rule.
    ///
    /// The FTS triggers fire on every `UPDATE songs` here and that is correct — a restored title has
    /// to be findable. Dropping them around a write pays off when hundreds of thousands of rows are
    /// rewritten, and this writes the few thousand somebody typed.
    pub fn apply_restore(&mut self, plan: &crate::backup::Plan) -> Result<RestoreOutcome, DbError> {
        let mut outcome = RestoreOutcome::default();
        let transaction = self.conn.transaction()?;
        {
            // The favorites that already exist, so a restore reuses one rather than colliding with
            // `favorites_name`.
            let mut known: BTreeMap<String, i64> = BTreeMap::new();
            {
                let mut statement = transaction.prepare("SELECT id, name FROM favorites")?;
                let rows = statement.query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, i64>(0)?))
                })?;
                for row in rows {
                    let (name, id) = row?;
                    known.insert(name, id);
                }
            }

            {
                let mut insert = transaction
                    .prepare("INSERT INTO favorites(name, temporary) VALUES (?1, ?2)")?;
                // **Only where the file wins.** A restore that is not overwriting leaves a list
                // somebody has here exactly as they have it, and only says what kind of list it is
                // when it makes one: `temporary` carries no value meaning *nobody has said* — false
                // is a decision as much as true is.
                let mut set_temporary =
                    transaction.prepare("UPDATE favorites SET temporary = ?2 WHERE id = ?1")?;
                for favorite in &plan.favorites {
                    match known.get(&favorite.name) {
                        Some(id) => {
                            if plan.overwrite {
                                set_temporary.execute(params![id, favorite.temporary])?;
                            }
                        }
                        None => {
                            insert.execute(params![favorite.name, favorite.temporary])?;
                            known.insert(favorite.name.clone(), transaction.last_insert_rowid());
                            outcome.favorites_created += 1;
                        }
                    }
                }
            }

            {
                let mut update = transaction.prepare(
                    "UPDATE songs SET
                         title = CASE WHEN ?10 THEN coalesce(?2, title) ELSE coalesce(title, ?2) END,
                         artist = CASE WHEN ?10 THEN coalesce(?3, artist)
                                       ELSE coalesce(artist, ?3) END,
                         language = CASE WHEN ?10 THEN coalesce(?4, language)
                                         ELSE coalesce(language, ?4) END,
                         lyric_encoding = CASE WHEN ?10 THEN coalesce(?5, lyric_encoding)
                                               ELSE coalesce(lyric_encoding, ?5) END,
                         default_transpose = CASE WHEN ?10 THEN coalesce(?6, default_transpose)
                                                  ELSE coalesce(default_transpose, ?6) END,
                         fixes = CASE WHEN ?10 THEN coalesce(?7, fixes)
                                      ELSE coalesce(fixes, ?7) END,
                         user_score = CASE WHEN ?10 THEN coalesce(?8, user_score)
                                           ELSE coalesce(user_score, ?8) END,
                         notes = CASE WHEN ?10 THEN coalesce(?9, notes) ELSE coalesce(notes, ?9) END,
                         melody_chosen = CASE WHEN ?10 THEN coalesce(?11, melody_chosen)
                                              ELSE coalesce(melody_chosen, ?11) END
                     WHERE id = ?1",
                )?;
                for song in &plan.songs {
                    outcome.songs_applied += update.execute(params![
                        song.id,
                        song.title,
                        song.artist,
                        song.language,
                        song.lyric_encoding,
                        song.default_transpose,
                        song.fixes,
                        song.user_score,
                        song.notes,
                        plan.overwrite,
                        // Past `?10`, which is the overwrite flag every arm reads: appended so the
                        // numbering of the nine above is untouched.
                        song.melody_chosen,
                    ])?;
                }
            }

            // A second pass, so the order songs appear in the file cannot decide whether a chain
            // forms. The `EXISTS` is [`Self::set_merged_into`]'s guard, which the borrow checker
            // will not let us call from in here.
            {
                let mut merge = transaction.prepare(MERGE_SQL)?;
                for (id, target) in &plan.merges {
                    if merge.execute(named_params! {
                        ":song": id,
                        ":target": target,
                        ":overwrite": plan.overwrite,
                    })? > 0
                    {
                        outcome.merges_applied += 1;
                    } else {
                        outcome.merges_refused.push(id.clone());
                    }
                }
            }

            {
                let mut file = transaction.prepare(
                    "INSERT OR IGNORE INTO song_favorites(song_id, favorite_id) VALUES (?1, ?2)",
                )?;
                for (song_id, name) in &plan.memberships {
                    if let Some(favorite) = known.get(name) {
                        file.execute(params![song_id, favorite])?;
                        outcome.memberships_applied += 1;
                    }
                }
            }

            // After the title and artist pass above. The merge pass between them writes only
            // `merged_into`, which changes no name — so this is placed for readability rather than
            // for correctness, and `refold` would find the same rows wherever in here it ran.
            Self::refold(&transaction)?;
        }
        transaction.commit()?;
        Ok(outcome)
    }
}
