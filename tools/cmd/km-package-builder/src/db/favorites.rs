//! The favorites: somebody's own filing of the corpus, as a flat set of named lists.
//!
//! One table of lists and a join table beside it, so a song can be in several at once. A list never
//! holds another — see `A favorite does not nest` in `docs/decisions/curation.md`.
//!
//! **The star in the browse list is `toggle_favorite`, and it is one round trip on purpose.** A
//! second click has to take the song back out, and a read-then-write would let two clicks on two
//! pages both decide they were the one putting it in.

use super::*;

impl Db {
    /// Every favorite, by name.
    ///
    /// **The count is of entries the list can show.** A star stays on a song somebody throws away,
    /// so a count off `song_favorites` alone promises songs that opening the list does not hold.
    pub fn favorites(&self) -> Result<Vec<FavoriteNode>, DbError> {
        let browsable = browsable("s.");
        let mut statement = self.conn.prepare(&format!(
            "SELECT f.id, f.name,
                    (SELECT COUNT(*) FROM song_favorites sf
                      JOIN songs s ON s.id = sf.song_id
                      WHERE sf.favorite_id = f.id AND {browsable}),
                    f.temporary
             FROM favorites f ORDER BY f.name COLLATE NOCASE",
        ))?;
        let rows: Vec<(i64, String, i64, bool)> = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let redundant = self.redundant_favorites()?;
        Ok(rows
            .into_iter()
            .map(|(id, name, count, temporary)| FavoriteNode {
                id,
                name,
                song_count: count as u32,
                second_copies: redundant.get(&id).copied().unwrap_or(0),
                temporary,
            })
            .collect())
    }

    /// How many entries in each favorite are a second version of a song already in that same list.
    ///
    /// **Memberships minus distinct recordings**, which is the whole calculation: two entries whose
    /// songs share a cluster are one recording filed twice, however they came to be there. A list
    /// holding three versions of one song contributes two.
    ///
    /// On the corpus this was written against, 2,169 of 7,089 memberships — 30.6% — answer to this.
    /// Nobody chose that: the browse list showed six rows of one song and more than one got starred,
    /// which is the thing [`VersionsFilter::Collapsed`] now prevents. This reports what was filed
    /// before it did.
    pub fn redundant_favorites(&self) -> Result<HashMap<i64, u32>, DbError> {
        let browsable = browsable("s.");
        let mut statement = self.conn.prepare(&format!(
            "SELECT sf.favorite_id,
                    COUNT(*) - COUNT(DISTINCT coalesce(s.duplicate_of, s.id))
             FROM song_favorites sf JOIN songs s ON s.id = sf.song_id
             WHERE {browsable}
             GROUP BY sf.favorite_id",
        ))?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? as u32))
        })?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(Into::into)
    }

    /// Drops every entry in one favorite that is a second version of a song the list keeps.
    ///
    /// **One version of each recording survives, and it is the best one the list actually holds**
    /// — not the cluster's representative, which may not be in this list at all. Somebody who
    /// starred two poor copies and never the good one keeps a poor copy rather than losing the
    /// song, which is the difference between tidying a list and editing it.
    ///
    /// **The survivor has to be one the list can show.** A song somebody threw away keeps its star,
    /// and without the term it wins on suitability and takes every live copy of that recording out
    /// of the list — which leaves the recording represented by an entry no page draws. Where every
    /// copy is deleted the subquery answers nothing, `id <> NULL` is never true, and the list is
    /// left alone.
    ///
    /// Answers how many entries went, so the sentence afterwards can say it.
    pub fn tidy_favorite(&self, favorite: i64) -> Result<usize, DbError> {
        // `song_favorites` has no id of its own, so the survivor is named by song rather than by
        // row: within one list a song appears at most once, which the primary key guarantees.
        let removed = self.conn.execute(
            "DELETE FROM song_favorites
             WHERE favorite_id = ?1 AND song_id IN (
               SELECT s.id FROM song_favorites sf JOIN songs s ON s.id = sf.song_id
               WHERE sf.favorite_id = ?1
                 AND s.id <> (
                   SELECT best.id FROM song_favorites sfb JOIN songs best ON best.id = sfb.song_id
                   WHERE sfb.favorite_id = ?1
                     AND coalesce(best.duplicate_of, best.id) = coalesce(s.duplicate_of, s.id)
                     AND best.deleted_at IS NULL
                   ORDER BY best.suitability DESC NULLS LAST, best.file_count DESC, best.id ASC
                   LIMIT 1))",
            params![favorite],
        )?;
        Ok(removed)
    }

    /// The favorites one song is in.
    pub fn favorites_for(&self, song_id: &str) -> Result<Vec<(i64, String)>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT f.id, f.name FROM favorites f
             JOIN song_favorites sf ON sf.favorite_id = f.id
             WHERE sf.song_id = ?1 ORDER BY f.name",
        )?;
        let rows = statement.query_map([song_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Creates a favorite, returning its id.
    pub fn create_favorite(&self, name: &str) -> Result<i64, DbError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DbError::Rejected("a favorite needs a name".to_owned()));
        }
        self.conn
            .execute("INSERT INTO favorites(name) VALUES (?1)", params![name])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Renames a favorite.
    pub fn rename_favorite(&self, id: i64, name: &str) -> Result<(), DbError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DbError::Rejected("a favorite needs a name".to_owned()));
        }
        self.conn.execute(
            "UPDATE favorites SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    /// Says whether a favorite is a working list — scaffolding for a later pass — or a filing.
    ///
    /// The flag alone: no song is touched, nothing moves, and turning it back makes the list what it
    /// was. What changes is one thing, the browse row's gold star, which stops claiming that a song
    /// in nothing but working lists has been filed.
    pub fn set_favorite_temporary(&self, id: i64, temporary: bool) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE favorites SET temporary = ?2 WHERE id = ?1",
            params![id, temporary],
        )?;
        Ok(())
    }

    /// Deletes a favorite and, by cascade, its memberships. The songs themselves are untouched.
    pub fn delete_favorite(&self, id: i64) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM favorites WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Files a whole list of songs into one favorite, in a single transaction.
    ///
    /// **One commit rather than one per song, which is what filing a ticked page used to cost.**
    /// Every caller here loops, and [`Self::set_favorite`] issues a bare `execute` — so in WAL mode
    /// that was one commit and one `fsync` per song, and a failure half way through left a partial
    /// result nobody was told about. Five hundred ticked rows was five hundred fsyncs.
    ///
    /// It is also the shape every other bulk write in this file already has:
    /// [`Self::add_to_package`], [`Self::add_tag_of`] and [`Self::set_names_from_stem`] all wrap
    /// their loops. This one simply had not been noticed.
    /// Answers with how many memberships were written or removed, which is not how many songs were
    /// named: filing a song already in the favorite writes nothing, and that is success.
    pub fn set_favorites(
        &mut self,
        song_ids: &[String],
        favorite: i64,
        member: bool,
    ) -> Result<u32, DbError> {
        let transaction = self.conn.transaction()?;
        let mut changed = 0u32;
        {
            // Prepared once for the whole batch rather than per row, which is the other half of what
            // a loop of `execute` was paying.
            let mut statement = transaction.prepare(if member {
                "INSERT OR IGNORE INTO song_favorites(song_id, favorite_id) VALUES (?1, ?2)"
            } else {
                "DELETE FROM song_favorites WHERE song_id = ?1 AND favorite_id = ?2"
            })?;
            for song_id in song_ids {
                changed += statement.execute(params![song_id, favorite])? as u32;
            }
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// Files every song a filter matches into one favorite, or takes every one of them out of it.
    ///
    /// **The `WHERE` is [`Filter::to_sql`] verbatim**, the discipline [`Db::add_tag_for`] rests on:
    /// what the list shows and what this writes are one clause, so they cannot diverge, and
    /// `merged_into IS NULL` is inherited rather than remembered.
    ///
    /// One statement rather than a loop over ids, so the size of the set costs nothing in round
    /// trips — a filter here can name hundreds of thousands of songs.
    pub fn set_favorites_for(
        &self,
        filter: &Filter,
        favorite: i64,
        member: bool,
    ) -> Result<u32, DbError> {
        let (where_clause, mut values) = filter.to_sql();
        values.push(Binding::Integer(favorite));
        let slot = values.len();
        let sql = if member {
            format!(
                "INSERT OR IGNORE INTO song_favorites (song_id, favorite_id)
                 SELECT s.id, ?{slot} FROM songs s WHERE {where_clause}"
            )
        } else {
            format!(
                "DELETE FROM song_favorites WHERE favorite_id = ?{slot}
                   AND song_id IN (SELECT s.id FROM songs s WHERE {where_clause})"
            )
        };
        let changed = self.conn.execute(&sql, params_from_iter(values.iter()))?;
        Ok(changed as u32)
    }

    /// Puts a song in a favorite, or takes it out.
    ///
    /// One song. [`Self::set_favorites`] is what a list wants — a loop of this is a commit per song.
    pub fn set_favorite(&self, song_id: &str, favorite: i64, member: bool) -> Result<(), DbError> {
        if member {
            self.conn.execute(
                "INSERT OR IGNORE INTO song_favorites(song_id, favorite_id) VALUES (?1, ?2)",
                params![song_id, favorite],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM song_favorites WHERE song_id = ?1 AND favorite_id = ?2",
                params![song_id, favorite],
            )?;
        }
        Ok(())
    }

    /// Puts a song in a favorite and reports whether it is in it afterwards, for the star in the
    /// song list: one click files it, a second click on the same favorite takes it back out.
    pub fn toggle_favorite(&self, song_id: &str, favorite: i64) -> Result<bool, DbError> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM song_favorites WHERE song_id = ?1 AND favorite_id = ?2",
            params![song_id, favorite],
            |row| row.get(0),
        )?;
        // Checked here rather than trusted from the browser: the row on screen may be minutes old,
        // and a stale idea of "it is already in this one" would silently do the opposite thing.
        let member = exists == 0;
        if member {
            let known: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM songs WHERE id = ?1",
                [song_id],
                |row| row.get(0),
            )?;
            if known == 0 {
                return Err(DbError::NotFound(format!("song {song_id}")));
            }
        }
        self.set_favorite(song_id, favorite, member)?;
        Ok(member)
    }
}
/// One row of `favorites`, as a backup reads it.
pub(super) struct FavoriteRow {
    /// What the list is called, which is also what a backup matches it on.
    pub(super) name: String,
    /// Whether it is a working list rather than a filing.
    pub(super) temporary: bool,
}
