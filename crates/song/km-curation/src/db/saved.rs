//! Filters somebody named.
//!
//! `favorites` with the tree and the songs taken out: a name, and the thing it names. What it names
//! is a query string rather than a set of rows, which is the whole difference between filing songs
//! and describing them.
//!
//! **Here rather than beside the recent-folder list, where the cursor is.** The two look alike and
//! are not the same fact. Where somebody happens to be is a fact about a run, kept per user and
//! thrown away when the folder is evicted from a list of twelve; `Portuguese, unclassified` is a
//! judgment about how this corpus divides, and it belongs to the corpus the way a favorite and a tag
//! do. See `A filter can be given a name, and then it is not the cursor` in `docs/decisions/`.

use super::*;

impl Db {
    /// Every saved filter, in the order the strip draws them.
    pub fn saved_filters(&self) -> Result<Vec<SavedFilter>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT id, name, query FROM saved_filters ORDER BY sort_key, name")?;
        let rows = statement
            .query_map([], |row| {
                Ok(SavedFilter {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    query: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The saved filter under this name, for the question the confirmation asks.
    ///
    /// Its own query rather than a failed insert, so *that name is taken* is an ordinary answer
    /// instead of a constraint violation somebody has to recognize — and the answer carries the
    /// query being replaced, which is what the confirmation has to show.
    pub fn saved_filter_named(&self, name: &str) -> Result<Option<SavedFilter>, DbError> {
        let found = self
            .conn
            .query_row(
                "SELECT id, name, query FROM saved_filters WHERE name = ?1",
                params![name.trim()],
                |row| {
                    Ok(SavedFilter {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        query: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(found)
    }

    /// Writes a filter down under a name, replacing whatever that name held.
    ///
    /// **One statement, so there is no read-then-write for two tabs to interleave** — the rule
    /// `toggle_favorite` states one file over. Whether replacing was *wanted* is asked before this
    /// is called and is not re-asked here: this is the write, and a caller that has already
    /// confirmed must not be refused by the same question a second time.
    ///
    /// An empty query is legal and means the whole corpus. An empty name is not, and is the one
    /// thing this refuses, because a chip with nothing written on it cannot be told from the next.
    pub fn save_filter(&self, name: &str, query: &str, now: &str) -> Result<i64, DbError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DbError::Rejected("a saved filter needs a name".to_owned()));
        }
        let id = self.conn.query_row(
            "INSERT INTO saved_filters(name, query, sort_key, saved_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET query    = excluded.query,
                                             sort_key = excluded.sort_key,
                                             saved_at = excluded.saved_at
             RETURNING id",
            params![name, query, km_song::text::fold(name), now],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// One saved filter by its row id, for the chip that draws it on its own.
    pub fn saved_filter(&self, id: i64) -> Result<Option<SavedFilter>, DbError> {
        let found = self
            .conn
            .query_row(
                "SELECT id, name, query FROM saved_filters WHERE id = ?1",
                params![id],
                |row| {
                    Ok(SavedFilter {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        query: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(found)
    }

    /// Writes a new query into a filter that already has a name, keeping the name.
    ///
    /// `false` when the row has gone, which two tabs on one strip is the ordinary way to reach — the
    /// same answer [`Self::delete_saved_filter`] gives to the same situation.
    ///
    /// What was replaced is not answered here: the caller has to have read the row already, because
    /// what the page is rewritten to depends on what it held.
    pub fn update_saved_filter(&self, id: i64, query: &str, now: &str) -> Result<bool, DbError> {
        let written = self.conn.execute(
            "UPDATE saved_filters SET query = ?2, saved_at = ?3 WHERE id = ?1",
            params![id, query, now],
        )?;
        Ok(written > 0)
    }

    /// Gives a saved filter a different name, keeping the query it holds.
    ///
    /// **A name that is taken is refused rather than replaced**, which is the opposite of
    /// [`Self::save_filter`] and is the same act pointed the other way: saving writes a query
    /// somebody is looking at into a name, and renaming would write a name over a *query* that is
    /// not on screen. There is nothing to show in a confirmation and a row would be lost.
    pub fn rename_saved_filter(&self, id: i64, name: &str) -> Result<(), DbError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DbError::Rejected("a saved filter needs a name".to_owned()));
        }
        if let Some(other) = self.saved_filter_named(name)?
            && other.id != id
        {
            return Err(DbError::Rejected(format!("{name} is already saved")));
        }
        self.conn.execute(
            "UPDATE saved_filters SET name = ?2, sort_key = ?3 WHERE id = ?1",
            params![id, name, km_song::text::fold(name)],
        )?;
        Ok(())
    }

    /// Forgets one.
    ///
    /// `false` when it was already gone, which is not an error: two tabs showing the same strip is
    /// the ordinary way that happens, and the answer to *it is not there* is the page that no longer
    /// draws it.
    pub fn delete_saved_filter(&self, id: i64) -> Result<bool, DbError> {
        let gone = self
            .conn
            .execute("DELETE FROM saved_filters WHERE id = ?1", params![id])?;
        Ok(gone > 0)
    }
}
