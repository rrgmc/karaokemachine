//! Packages — the thing this tool exists to produce — and the merges that change what goes in one.
//!
//! A package is a row, one volume or more, and an ordered membership, and the ordering is the part
//! with rules: a song number runs 1 to 999 within a volume and the machine supplies the thousands, so
//! `add_to_package` places what fits in the last volume and counts the rest apart, and
//! `renumber_package` refuses a reflow that would run past it rather than truncating one. Both are
//! whole-transaction: half a renumber is worse than none.
//!
//! **Sourcing is here because it is the second thing that decides what a package contains.** A
//! package sourced from favorites holds the union of those lists and nothing else, and
//! [`Db::sync_package`] is what makes the two agree: it removes first, so the numbers its removals
//! free can be handed straight to the songs arriving, and nothing already in the package moves.
//! [`place_songs`] is the one copy of the insert both it and `add_to_package` go through, because two
//! placements would be two ideas of what a clash is.
//!
//! **Merges are here because they are the third thing that decides what a package contains.** When
//! one song is merged into another every query hides the merged one, so a package built afterwards
//! gets the survivor — and the guard against a merge that would chain or point at itself is in the
//! `WHERE` of [`MERGE_SQL`], not in a read beforehand, because a chain that formed between a check
//! and a write is the kind of fault nobody could reproduce.

use super::*;

impl Db {
    /// Every package curated in this folder, each seen through its first volume.
    pub fn packages(&self) -> Result<Vec<PackageRow>, DbError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {PACKAGE_COLUMNS} WHERE v.volume = 1 ORDER BY p.created_at DESC, p.id"
        ))?;
        let rows = statement.query_map([], package_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// One package, seen through its first volume.
    pub fn package(&self, id: &str) -> Result<PackageRow, DbError> {
        self.package_volume(id, 1)
    }

    /// One package, seen through the volume asked for.
    pub fn package_volume(&self, id: &str, volume: u32) -> Result<PackageRow, DbError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {PACKAGE_COLUMNS} WHERE p.id = ?1 AND v.volume = ?2"
        ))?;
        statement
            .query_row(params![id, volume], package_row)
            .optional()?
            .ok_or_else(|| DbError::NotFound(format!("package {id} volume {volume}")))
    }

    /// Every volume of one package, in order.
    pub fn package_volumes(&self, id: &str) -> Result<Vec<PackageRow>, DbError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {PACKAGE_COLUMNS} WHERE p.id = ?1 ORDER BY v.volume"
        ))?;
        let rows = statement.query_map([id], package_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every volume of every package, sourced ones included, in the order [`Self::packages`] lists
    /// the packages.
    ///
    /// **For the replace control on a song's page**, which names a volume because a number is only
    /// unique inside one. Sourced packages are here and not in [`Self::packages_taking_songs`],
    /// because a replacement changes the lists such a package follows along with it.
    pub fn package_volumes_all(&self) -> Result<Vec<PackageRow>, DbError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {PACKAGE_COLUMNS} ORDER BY p.created_at DESC, p.id, v.volume"
        ))?;
        let rows = statement.query_map([], package_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The packages a song already belongs to: the package, the volume, and the number there.
    pub fn packages_for(&self, song_id: &str) -> Result<Vec<(String, u32, u32)>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT package_id, volume, number FROM package_songs
              WHERE song_id = ?1 ORDER BY package_id",
        )?;
        let rows = statement.query_map([song_id], |row| {
            Ok((
                row.get(0)?,
                row.get::<_, i64>(1)? as u32,
                row.get::<_, i64>(2)? as u32,
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Creates a package and its first volume, which takes the package's id.
    ///
    /// **The name falling back to the id is a last resort now, not a convenience.** It was a decent
    /// default when an id was `classic-rock-01`, typed by the same person who would have typed the
    /// name; it produces sixteen hex characters since ids became generated. The *form* therefore
    /// refuses a blank name in `handlers::create_package`, and this stays only so that `build::import`
    /// can open a `.kmpkg` whose manifest names nothing rather than refusing it outright.
    pub fn create_package(&self, package: &PackageRow, now: &str) -> Result<(), DbError> {
        let id = package.id.trim();
        if id.is_empty() {
            return Err(DbError::Rejected("a package needs an id".to_owned()));
        }
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO packages(id, name, publisher, default_language, volume_format, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                if package.name.trim().is_empty() {
                    id
                } else {
                    package.name.trim()
                },
                package.publisher,
                package.default_language,
                package.volume_format,
                now
            ],
        )?;
        transaction.execute(
            "INSERT INTO package_volumes(package_id, volume, id, package_version, start_number,
                                         created_at)
             VALUES (?1, 1, ?1, ?2, ?3, ?4)",
            params![id, package.version, package.start_number, now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Deletes a package. The songs themselves are untouched.
    pub fn delete_package(&self, id: &str) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM packages WHERE id = ?1", [id])?;
        Ok(())
    }

    /// What one volume of a package holds, in number order.
    pub fn package_members(&self, id: &str, volume: u32) -> Result<Vec<PackageMember>, DbError> {
        let sql = format!(
            "SELECT ps.number, ps.song_id,
                    {} AS eff_title,
                    {} AS eff_artist,
                    s.suitability, s.user_score, s.melody_channel, s.duration_ms,
                    (SELECT f.path FROM files f
                      WHERE f.id = ps.file_id OR (ps.file_id IS NULL AND f.song_id = ps.song_id)
                      ORDER BY f.path LIMIT 1),
                    -- Appended, so the reader below gains an index and none of the existing ones
                    -- move. The same expression the browse column reads, so a song listed under one
                    -- language there and another here would be impossible rather than unlikely.
                    {} AS eff_language
             FROM package_songs ps
             JOIN songs s ON s.id = ps.song_id
             WHERE ps.package_id = ?1 AND ps.volume = ?2
             ORDER BY ps.number",
            eff_title("s."),
            eff_artist("s."),
            eff_language("s."),
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![id, volume], |row| {
            Ok(PackageMember {
                number: row.get::<_, i64>(0)? as u32,
                song_id: row.get(1)?,
                title: row.get(2)?,
                artist: row.get(3)?,
                suitability: row.get::<_, Option<i64>>(4)?.map(|v| v as u8),
                user_score: row.get::<_, Option<i64>>(5)?.map(|v| v as u8),
                melody_channel: row.get::<_, Option<i64>>(6)?.map(|v| v as u8),
                duration_ms: row.get::<_, i64>(7)? as u32,
                // Dropped rather than refused, and this is the one reader where that is right: a
                // member with no path is already the "its source file is gone" case a build
                // reports per song, so a path that will not be followed lands in a report somebody
                // reads instead of failing the whole package. See [`crate::db::contained`].
                path: row
                    .get::<_, Option<String>>(8)?
                    .filter(|path| km_kmpkg::is_safe_path(path)),
                language: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// How many numbers a package has left for a hand add, which is how many songs it can still
    /// take.
    ///
    /// **Its last volume's, and only that one's.** A hand add appends where the package ends; only a
    /// sync starts a volume, because a volume is filled from lists somebody curated rather than from
    /// whatever a filter matched. See `A package holds volumes` in `docs/decisions/curation.md`.
    ///
    /// **For saying so before a write, never for deciding one.** A filter-wide add reads this to ask
    /// how many of its matches to fetch out of the corpus and to say what will not fit, and
    /// [`Self::add_to_package`] recomputes the same number inside its own transaction because that
    /// is the one that must be right. The two agree by sharing [`next_number`].
    pub fn package_room(&self, package_id: &str) -> Result<u32, DbError> {
        let volume = last_volume(&self.conn, package_id)?;
        let next = next_number(&self.conn, package_id, volume)?;
        let left = i64::from(km_songcode::MAX_SLOT) - next + 1;
        Ok(u32::try_from(left.max(0)).unwrap_or(0))
    }

    /// Adds songs to a package's last volume, assigning each the next free number.
    ///
    /// A song already in the package, in any volume, is skipped rather than an error, because adding
    /// a selection that overlaps what is already there is a normal thing to do, and one the numbering
    /// has no room for is left out. [`Added`] counts the three apart, so the caller names which
    /// happened rather than subtracting one number from another and guessing.
    pub fn add_to_package(
        &mut self,
        package_id: &str,
        song_ids: &[String],
        now: &str,
    ) -> Result<Added, DbError> {
        let volume = last_volume(&self.conn, package_id)?;
        self.add_to_volume(package_id, volume, song_ids, now)
    }

    /// Adds songs to one named volume of a package, assigning each that volume's next free number.
    ///
    /// [`Self::add_to_package`] with the volume chosen by the caller, which is what an import needs: a
    /// file says which volume it is, and its songs belong there whatever the package's last volume is.
    pub fn add_to_volume(
        &mut self,
        package_id: &str,
        volume: u32,
        song_ids: &[String],
        now: &str,
    ) -> Result<Added, DbError> {
        let transaction = self.conn.transaction()?;
        // Inside the transaction, which is the half [`Self::package_room`] cannot have: what a
        // confirmation showed is a number from a moment ago, and what the insert numbers from has to
        // be the number as it is when the rows go in.
        let next = next_number(&transaction, package_id, volume)?;
        // The append this route has always done, and the one [`Self::package_room`] promises.
        let mut numbers =
            (next..=i64::from(km_songcode::MAX_SLOT)).map(|number| (i64::from(volume), number));
        let added = place_songs(&transaction, package_id, song_ids, &mut numbers, now)?;
        let added = fill_full(&transaction, package_id, volume, added)?;
        transaction.commit()?;
        Ok(added)
    }

    /// Makes sure a package has a volume of this number under this id, adding it when it is missing.
    ///
    /// **An id that disagrees is refused.** A volume's id is what a machine keys an install on, so a
    /// file claiming to be volume 2 of a package whose volume 2 here is another file is two packages
    /// that must not be merged by accident.
    pub fn ensure_volume(
        &self,
        package_id: &str,
        volume: u32,
        volume_id: &str,
        now: &str,
    ) -> Result<(), DbError> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM package_volumes WHERE package_id = ?1 AND volume = ?2",
                params![package_id, volume],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(id) if id == volume_id => Ok(()),
            Some(id) => Err(DbError::Rejected(format!(
                "volume {volume} of package {package_id} is {id} here, not {volume_id}"
            ))),
            None => {
                self.conn.execute(
                    "INSERT INTO package_volumes(package_id, volume, id, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![package_id, volume, volume_id, now],
                )?;
                Ok(())
            }
        }
    }

    /// Removes one song from a package, from whichever volume holds it.
    pub fn remove_from_package(&self, package_id: &str, song_id: &str) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM package_songs WHERE package_id = ?1 AND song_id = ?2",
            params![package_id, song_id],
        )?;
        Ok(())
    }

    /// Gives one member a specific number, inside the volume that holds it.
    pub fn set_package_number(
        &self,
        package_id: &str,
        song_id: &str,
        number: u32,
    ) -> Result<(), DbError> {
        if number == 0 {
            return Err(DbError::Rejected("song numbers start at 1".to_owned()));
        }
        if number > u32::from(km_songcode::MAX_SLOT) {
            return Err(DbError::Rejected(format!(
                "a package numbers its songs 1 to {}; the machine adds the bank, so a song above \
                 that would be dialled as another package's",
                km_songcode::MAX_SLOT
            )));
        }
        let taken: Option<String> = self
            .conn
            .query_row(
                "SELECT other.song_id FROM package_songs mine
                   JOIN package_songs other
                     ON other.package_id = mine.package_id AND other.volume = mine.volume
                  WHERE mine.package_id = ?1 AND mine.song_id = ?2 AND other.number = ?3",
                params![package_id, song_id, number],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(other) = taken
            && other != song_id
        {
            return Err(DbError::Rejected(format!(
                "number {number} is already used by another song in this package"
            )));
        }
        self.conn.execute(
            "UPDATE package_songs SET number = ?3 WHERE package_id = ?1 AND song_id = ?2",
            params![package_id, song_id, number],
        )?;
        Ok(())
    }

    /// What putting a song in the place of the one at `number` would do, without doing it.
    ///
    /// The inner `Err` is a refusal somebody is told about; the outer one is a failure to read.
    pub fn replacement_at(
        &self,
        package_id: &str,
        volume: u32,
        number: u32,
        new_song_id: &str,
    ) -> Result<Result<Replacement, ReplaceRefusal>, DbError> {
        plan_replacement(&self.conn, package_id, volume, number, new_song_id)
    }

    /// Puts a song in the place of the one at `number`, keeping the volume and the number.
    ///
    /// **The number is what stays**, because it is what a songbook prints and what a singer dials.
    /// Remove and Add would give the new song the number after the highest.
    ///
    /// **In a package that follows favorites, the lists change too.** A sync cannot tell that the new
    /// song stands for the old one: it takes the old one out and gives the new one the lowest free
    /// number, which is the old number only by chance. So in each list this package follows, the new
    /// song takes the old one's place, and the next sync finds nothing to move. The match is
    /// [`WANTED_SQL`]'s, so a list holding a song merged into the old one is changed as well. A list
    /// this package does not follow is left alone.
    pub fn replace_in_package(
        &mut self,
        package_id: &str,
        volume: u32,
        number: u32,
        new_song_id: &str,
        now: &str,
    ) -> Result<Result<Replacement, ReplaceRefusal>, DbError> {
        let transaction = self.conn.transaction()?;
        let replacement =
            match plan_replacement(&transaction, package_id, volume, number, new_song_id)? {
                Ok(replacement) => replacement,
                Err(refusal) => return Ok(Err(refusal)),
            };
        let file_id: Option<i64> = transaction
            .query_row(BEST_FILE_SQL, [new_song_id], |row| row.get(0))
            .optional()?;
        transaction.execute(
            "UPDATE package_songs SET song_id = ?4, file_id = ?5, added_at = ?6
              WHERE package_id = ?1 AND volume = ?2 AND number = ?3",
            params![package_id, volume, number, new_song_id, file_id, now],
        )?;
        if !replacement.favorites.is_empty() {
            let old_id = &replacement.old.0;
            transaction.execute(
                "INSERT OR IGNORE INTO song_favorites(song_id, favorite_id)
                 SELECT DISTINCT :new, sf.favorite_id
                   FROM song_favorites sf
                   JOIN package_favorites pf ON pf.favorite_id = sf.favorite_id
                   JOIN songs src ON src.id = sf.song_id
                  WHERE pf.package_id = :package AND coalesce(src.merged_into, src.id) = :old",
                named_params! { ":new": new_song_id, ":package": package_id, ":old": old_id },
            )?;
            transaction.execute(
                "DELETE FROM song_favorites
                  WHERE favorite_id IN
                        (SELECT favorite_id FROM package_favorites WHERE package_id = :package)
                    AND song_id IN
                        (SELECT id FROM songs WHERE coalesce(merged_into, id) = :old)",
                named_params! { ":package": package_id, ":old": old_id },
            )?;
        }
        transaction.commit()?;
        Ok(Ok(replacement))
    }

    /// Re-flows every member of one volume from that volume's start number, keeping their order.
    ///
    /// Done in two passes through a negative range, because the numbers are a primary key and a
    /// straight update would collide with a row it has not moved yet.
    pub fn renumber_package(&mut self, package_id: &str, volume: u32) -> Result<usize, DbError> {
        let start = start_number(&self.conn, package_id, volume)?;

        let transaction = self.conn.transaction()?;
        let ids: Vec<String> = {
            let mut statement = transaction.prepare(
                "SELECT song_id FROM package_songs
                  WHERE package_id = ?1 AND volume = ?2 ORDER BY number",
            )?;
            let rows = statement.query_map(params![package_id, volume], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        // Refused whole rather than half done: a re-flow is one gesture, and renumbering the first
        // half of a package while leaving the rest parked is worse than not starting.
        let last = start + ids.len() as i64 - 1;
        if last > i64::from(km_songcode::MAX_SLOT) {
            return Err(DbError::Rejected(format!(
                "renumbering {} song(s) from {start} would reach {last}, and song numbers stop at {}",
                ids.len(),
                km_songcode::MAX_SLOT
            )));
        }
        {
            let mut park = transaction.prepare(
                "UPDATE package_songs SET number = ?3 WHERE package_id = ?1 AND song_id = ?2",
            )?;
            for (index, song_id) in ids.iter().enumerate() {
                park.execute(params![package_id, song_id, -(index as i64) - 1])?;
            }
            for (index, song_id) in ids.iter().enumerate() {
                park.execute(params![package_id, song_id, start + index as i64])?;
            }
        }
        transaction.commit()?;
        Ok(ids.len())
    }

    /// Records where a volume was written and when.
    pub fn record_build(
        &self,
        package_id: &str,
        volume: u32,
        out: &str,
        now: &str,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE package_volumes SET out_path = ?3, built_at = ?4
              WHERE package_id = ?1 AND volume = ?2",
            params![package_id, volume, out, now],
        )?;
        Ok(())
    }

    /// Whether a build of this package raises its patch number first.
    ///
    /// **Read through its own method rather than off [`PackageRow`], and so is every write below.**
    /// [`Self::update_package`] sets every editable column from a row `handlers::package_row`
    /// builds out of a form, and neither form that reaches it carries the tick box — so a field on
    /// the row would be cleared every time somebody saved a package's name.
    pub fn raise_version(&self, package_id: &str) -> Result<bool, DbError> {
        Ok(self.conn.query_row(
            "SELECT raise_version FROM packages WHERE id = ?1",
            params![package_id],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    /// Says whether a build of this package raises its patch number first.
    pub fn set_raise_version(&self, package_id: &str, raise: bool) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE packages SET raise_version = ?2 WHERE id = ?1",
            params![package_id, raise],
        )?;
        Ok(())
    }

    /// Records the version a build of one volume has just written.
    ///
    /// Narrow on purpose: this runs in a build's last lock, which holds no `PackageRow` and must
    /// not overwrite a name or a language somebody edited during the minutes the build was
    /// unlocked.
    pub fn set_package_version(
        &self,
        package_id: &str,
        volume: u32,
        version: &str,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE package_volumes SET package_version = ?3 WHERE package_id = ?1 AND volume = ?2",
            params![package_id, volume, version],
        )?;
        Ok(())
    }

    /// Updates what is said about the whole package: its name, publisher, default language and how
    /// its volumes are numbered in their names.
    ///
    /// **Narrow, beside [`Self::update_package`]**, because the Details form carries these three and
    /// nothing about a volume. Writing a whole row from it would set a volume's version back to
    /// whatever a form that never showed one defaulted to.
    pub fn update_package_details(
        &self,
        id: &str,
        name: &str,
        publisher: Option<&str>,
        default_language: Option<&str>,
        volume_format: &str,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE packages SET name = ?2, publisher = ?3, default_language = ?4, volume_format = ?5
              WHERE id = ?1",
            params![id, name, publisher, default_language, volume_format],
        )?;
        Ok(())
    }

    /// Updates one volume's version, its first number, or both. `None` leaves that one alone.
    pub fn update_volume(
        &self,
        id: &str,
        volume: u32,
        version: Option<&str>,
        start_number: Option<u32>,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE package_volumes
                SET package_version = coalesce(?3, package_version),
                    start_number = coalesce(?4, start_number)
              WHERE package_id = ?1 AND volume = ?2",
            params![id, volume, version, start_number],
        )?;
        Ok(())
    }

    /// Updates a package's own details, and the version and first number of the volume the row
    /// shows.
    pub fn update_package(&self, package: &PackageRow) -> Result<(), DbError> {
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "UPDATE packages SET name = ?2, publisher = ?3, default_language = ?4 WHERE id = ?1",
            params![
                package.id,
                package.name,
                package.publisher,
                package.default_language
            ],
        )?;
        transaction.execute(
            "UPDATE package_volumes SET package_version = ?3, start_number = ?4
              WHERE package_id = ?1 AND volume = ?2",
            params![
                package.id,
                package.volume,
                package.version,
                package.start_number
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    // -- sourcing from favorites ----------------------------------------------------------------

    /// The favorites a package is sourced from — id, name, and whether the list is a working one.
    ///
    /// A tuple rather than a [`FavoriteNode`], which `Db::favorites_for` is already the precedent
    /// for: a node carries a song count and a second-copies count that belong to the Favorites page,
    /// and half-filling them here would put two numbers on the screen that mean nothing.
    pub fn package_sources(&self, package_id: &str) -> Result<Vec<(i64, String, bool)>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT f.id, f.name, f.temporary
             FROM package_favorites pf JOIN favorites f ON f.id = pf.favorite_id
             WHERE pf.package_id = ?1 ORDER BY f.name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([package_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every package-and-favorite pair there is, with both names.
    ///
    /// **One query behind two pages.** The Packages page groups these by package to say which lists
    /// decide a volume; the Favorites page groups them by favorite to say which volumes a Delete
    /// would change. Two queries would be two ideas of what a source is, and a page apiece paying
    /// one round trip per row.
    pub fn package_sources_all(&self) -> Result<Vec<SourceLink>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT pf.package_id, p.name, pf.favorite_id, f.name
             FROM package_favorites pf
             JOIN packages p ON p.id = pf.package_id
             JOIN favorites f ON f.id = pf.favorite_id
             ORDER BY p.name COLLATE NOCASE, f.name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SourceLink {
                package_id: row.get(0)?,
                package_name: row.get(1)?,
                favorite_id: row.get(2)?,
                favorite_name: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Puts one favorite among a package's sources, or takes it out, and answers whether the package
    /// draws on that list afterwards.
    ///
    /// **One list per call**, which is the shape [`Self::set_favorite`] already has and the one the
    /// page's two gestures are: a row's Remove, and a name picked out and added. A call that replaced
    /// the whole set would make each of those post every other list back, so a page open while
    /// another added one would take it away again.
    ///
    /// **A working list is never taken as a source, and the guard is in the statement.** A list
    /// meaning *decide about these later* is the opposite of a volume somebody is ready to build, and
    /// a check read beforehand would be a check the flag could change under. See
    /// [`A working list is not a source`](../../../../../docs/decisions/curation.md).
    ///
    /// **The answer is read back rather than taken from the row count**, so *already a source* and
    /// *refused because it is a working list* cannot arrive as the same number. The caller says a
    /// different sentence for each.
    ///
    /// **It syncs nothing.** Pointing a package at another list is a decision, and what that would do
    /// to its songs is what [`Self::package_sync_plan`] answers in counts before anything is written.
    pub fn set_package_source(
        &self,
        package_id: &str,
        favorite: i64,
        member: bool,
    ) -> Result<bool, DbError> {
        if member {
            self.conn.execute(
                "INSERT OR IGNORE INTO package_favorites(package_id, favorite_id)
                 SELECT ?1, f.id FROM favorites f WHERE f.id = ?2 AND f.temporary = 0",
                params![package_id, favorite],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM package_favorites WHERE package_id = ?1 AND favorite_id = ?2",
                params![package_id, favorite],
            )?;
        }
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM package_favorites WHERE package_id = ?1 AND favorite_id = ?2",
                params![package_id, favorite],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Every package a song may still be added to one at a time: the ones no favorite sources.
    ///
    /// **[`Self::packages`] with one clause, rather than a field on [`PackageRow`].** The Packages
    /// page lists every package and these two selects list the ones that are an answer to *add these
    /// songs to which?* — and a row that carried the fact would have to be invented by
    /// `build::import`, which reads a `PackageRow` out of a manifest.
    pub fn packages_taking_songs(&self) -> Result<Vec<PackageRow>, DbError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {PACKAGE_COLUMNS}
              WHERE v.volume = 1
                AND NOT EXISTS (SELECT 1 FROM package_favorites pf WHERE pf.package_id = p.id)
              ORDER BY p.created_at DESC, p.id"
        ))?;
        let rows = statement.query_map([], package_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Whether any favorite sources this package, which is the whole of what *sourced* means.
    ///
    /// Its own query rather than `package_sources(id)?.is_empty()`: the add route asks this about a
    /// package it is not otherwise reading, and the answer is a row's existence rather than a list
    /// nobody wants.
    pub fn is_sourced(&self, package_id: &str) -> Result<bool, DbError> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM package_favorites WHERE package_id = ?1 LIMIT 1",
                [package_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// What a sync would do, without doing any of it.
    ///
    /// Counted through [`WANTED_SQL`], which is the statement the write runs — so the numbers
    /// somebody reads and the set that is written are one clause. The three rules in
    /// `Acting on a whole filter` are what this exists to keep.
    pub fn package_sync_plan(&self, package_id: &str) -> Result<SyncPlan, DbError> {
        // Asked first, so a package that is not there is `NotFound` rather than a plan of zeros.
        start_number(&self.conn, package_id, 1)?;
        let sources = self.package_sources(package_id)?;

        let would_remove: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM package_songs
                  WHERE package_id = :package AND song_id NOT IN ({WANTED_SQL})"
            ),
            named_params! { ":package": package_id },
            |row| row.get(0),
        )?;
        let kept: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM package_songs
                  WHERE package_id = :package AND song_id IN ({WANTED_SQL})"
            ),
            named_params! { ":package": package_id },
            |row| row.get(0),
        )?;
        let would_add: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM ({WANTED_SQL}) w
                  WHERE w.song_id NOT IN
                        (SELECT song_id FROM package_songs WHERE package_id = :package)"
            ),
            named_params! { ":package": package_id },
            |row| row.get(0),
        )?;
        // Each volume's free numbers, summed. The numbers below a volume's first number are not
        // free: a member sitting under a start somebody raised after the fact keeps its number, and
        // nothing is ever handed one there. The same reading `next_number`'s `max` gives.
        let room: i64 = self.conn.query_row(
            &format!(
                "SELECT coalesce(SUM(max(0, :max - v.start_number + 1 -
                    (SELECT COUNT(*) FROM package_songs ps
                      WHERE ps.package_id = v.package_id AND ps.volume = v.volume
                        AND ps.number >= v.start_number AND ps.song_id IN ({WANTED_SQL})))), 0)
                   FROM package_volumes v WHERE v.package_id = :package"
            ),
            named_params! { ":package": package_id, ":max": i64::from(km_songcode::MAX_SLOT) },
            |row| row.get(0),
        )?;

        Ok(SyncPlan {
            sources,
            would_add: u32::try_from(would_add).unwrap_or(u32::MAX),
            would_remove: u32::try_from(would_remove).unwrap_or(u32::MAX),
            kept: u32::try_from(kept).unwrap_or(u32::MAX),
            new_volumes: volumes_needed(would_add - room),
        })
    }

    /// Makes a package hold exactly what the favorites it is sourced from hold.
    ///
    /// **One transaction, and the removals come first inside it.** That order is what lets a new song
    /// take a number this sync has just freed: `number` is half the primary key, so an insert into a
    /// slot a surviving row still holds is a constraint failure. [`Self::renumber_package`]'s walk
    /// through negative numbers is not needed, because nothing already in the package ever moves —
    /// a member keeps the volume and the number it had, which is the promise the whole arrangement
    /// rests on.
    ///
    /// **New songs fill the freed gaps before they append**, lowest volume first and lowest number
    /// first inside it. Numbering from the highest — which is what [`Self::add_to_package`] rightly
    /// does for a hand add whose room has been said out loud — would spend a volume's 999 slots on
    /// the songs a list has held and lost, so a list edited a few hundred times would run out with
    /// forty songs in it.
    ///
    /// **What every volume together cannot hold starts new volumes**, as many as it takes, each
    /// numbered from 1 under an id of its own.
    ///
    /// **A package with no sources is refused rather than emptied.** Pressing Sync on one nobody has
    /// given a list to means nothing, and the reading that empties it is the one that costs a
    /// package.
    pub fn sync_package(&mut self, package_id: &str, now: &str) -> Result<Synced, DbError> {
        start_number(&self.conn, package_id, 1)?;
        if !self.is_sourced(package_id)? {
            return Err(DbError::Rejected(format!(
                "package {package_id} is sourced from no favorite, and a sync of one would empty it"
            )));
        }

        let transaction = self.conn.transaction()?;
        // Read inside the transaction, for [`Self::add_to_package`]'s reason: what a confirmation
        // showed is a set from a moment ago, and what the write acts on has to be the set as it is.
        let wanted: Vec<String> = {
            let mut statement = transaction.prepare(&format!(
                "SELECT s.id FROM ({WANTED_SQL}) w JOIN songs s ON s.id = w.song_id
                  ORDER BY {WITHIN_TITLE}"
            ))?;
            let rows =
                statement.query_map(named_params! { ":package": package_id }, |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let removed = transaction.execute(
            &format!(
                "DELETE FROM package_songs
                  WHERE package_id = :package AND song_id NOT IN ({WANTED_SQL})"
            ),
            named_params! { ":package": package_id },
        )? as u32;

        // What survived, and where it sits. Both are read after the delete, so the numbers this walks
        // are the ones actually still taken.
        let (taken, held) = {
            let mut statement = transaction.prepare(
                "SELECT volume, number, song_id FROM package_songs WHERE package_id = ?1",
            )?;
            let rows = statement.query_map([package_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut taken: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
            let mut held = BTreeSet::new();
            for row in rows {
                let (volume, number, song_id) = row?;
                taken.entry(volume).or_default().insert(number);
                held.insert(song_id);
            }
            (taken, held)
        };
        let kept = u32::try_from(held.len()).unwrap_or(u32::MAX);

        let newcomers: Vec<String> = wanted
            .into_iter()
            .filter(|song_id| !held.contains(song_id))
            .collect();

        let mut volumes = volume_starts(&transaction, package_id)?;
        let room: usize = volumes
            .iter()
            .map(|(volume, start)| {
                free_numbers(taken.get(volume).unwrap_or(&BTreeSet::new()), *start).count()
            })
            .sum();
        let new_volumes = volumes_needed(newcomers.len() as i64 - room as i64);
        for _ in 0..new_volumes {
            let next = volumes.last().map_or(1, |(volume, _)| volume + 1);
            add_volume(&transaction, package_id, next, now)?;
            volumes.push((next, 1));
        }

        let empty = BTreeSet::new();
        let mut numbers = volumes.iter().flat_map(|(volume, start)| {
            free_numbers(taken.get(volume).unwrap_or(&empty), *start)
                .map(move |number| (*volume, number))
        });
        let placed = place_songs(&transaction, package_id, &newcomers, &mut numbers, now)?;
        transaction.commit()?;

        Ok(Synced {
            placed,
            removed,
            kept,
            new_volumes,
        })
    }

    // -- merges ---------------------------------------------------------------------------------

    /// States or clears a merge, for a test that needs one to exist.
    ///
    /// **[`SongEdit`] has no field for this and should not gain one**: an edit is a correction to
    /// what a song *is*, and a merge is a statement that two songs are one recording. In the running
    /// tool a merge is only ever made by [`Self::resolve_duplicate`] from the duplicates page and
    /// undone by [`Self::unmerge`], which is why this is `#[cfg(test)]` rather than a third way in.
    ///
    /// It shares [`MERGE_SQL`] with the restore, so what a test proves about the one-level rule here
    /// is proved about the statement the restore actually runs. Answers `false` when nothing was
    /// written, which is what a refused merge looks like.
    #[cfg(test)]
    pub fn set_merged_into(&self, id: &str, target: Option<&str>) -> Result<bool, DbError> {
        let changed = match target {
            Some(target) if target == id => {
                return Err(DbError::Rejected(
                    "a song cannot be merged into itself".to_owned(),
                ));
            }
            // `true`: an explicit call says to record this merge, so an existing one is replaced.
            Some(target) => self.conn.execute(
                MERGE_SQL,
                named_params! { ":song": id, ":target": target, ":overwrite": true },
            )?,
            None => self
                .conn
                .execute("UPDATE songs SET merged_into = NULL WHERE id = ?1", [id])?,
        };
        Ok(changed > 0)
    }

    /// Undoes a merge, and reopens the suggestion that made it.
    ///
    /// **Both statements or neither is the point.** A pair is suggested again only while its verdict
    /// is NULL, so clearing the merge without clearing the verdict would take the song back out of
    /// hiding and leave the pair judged for good — unfindable from the page that judged it.
    pub fn unmerge(&self, id: &str) -> Result<(), DbError> {
        self.conn
            .execute("UPDATE songs SET merged_into = NULL WHERE id = ?1", [id])?;
        self.conn.execute(
            "UPDATE duplicate_candidates SET verdict = NULL WHERE b_id = ?1 AND verdict = 'same'",
            [id],
        )?;
        Ok(())
    }
}

/// The songs the favorites a package is sourced from hold, each once, as `song_id`.
///
/// **A constant because the count and the write both run it**, which is [`MERGE_SQL`]'s discipline
/// one table over: a confirmation saying twelve songs go in, over a write that reads a different set,
/// is a number nobody can check. Four statements interpolate it and every one binds `:package`.
///
/// **`DISTINCT` is the whole of "a song in several lists arrives once."** A curator filing one song
/// under *Brasil Axé* and *Brasil Samba* has said two things about it and asked for one entry.
///
/// **`coalesce` resolves a merge, so the union is of survivors.** This module's header already states
/// the rule — every query hides a merged song, so a package built afterwards gets the one it turned
/// out to be — and a star put on a file before the merge goes on naming the row that lost. One level
/// only, which [`MERGE_SQL`] guarantees. The visible consequence is worth knowing: a package holding
/// the merged id loses that entry and gains the survivor's, so a sync after a merge reports one out
/// and one in for what a person would call one song.
///
/// **`duplicate_of` is deliberately not collapsed.** That column is a machine's guess where
/// `merged_into` is somebody's word, and two versions of one recording both starred are two members
/// — counted as [`Added::clashed`] and kept, exactly as a hand add keeps them.
pub(super) const WANTED_SQL: &str = "SELECT DISTINCT coalesce(src.merged_into, src.id) AS song_id
     FROM package_favorites pf
     JOIN song_favorites sf ON sf.favorite_id = pf.favorite_id
     JOIN songs src ON src.id = sf.song_id
    WHERE pf.package_id = :package";

/// The numbers a volume has free, lowest first, from its first number to the last slot.
///
/// Only the range a volume numbers in: a member sitting *below* a first number somebody raised after
/// the fact keeps its number and nothing is ever handed one down there, which is the reading
/// [`next_number`]'s `max` already gives.
///
/// The naive filter is exact and costs nothing — there are 999 candidates at most, and the set it
/// tests against is the volume itself.
fn free_numbers(taken: &BTreeSet<i64>, start: i64) -> impl Iterator<Item = i64> + '_ {
    (start..=i64::from(km_songcode::MAX_SLOT)).filter(move |number| !taken.contains(number))
}

/// How many volumes it takes to hold `overflow` songs no existing volume has a number for.
fn volumes_needed(overflow: i64) -> u32 {
    let per_volume = i64::from(km_songcode::MAX_SLOT);
    u32::try_from((overflow.max(0) + per_volume - 1) / per_volume).unwrap_or(u32::MAX)
}

/// A volume's first number, and the error that says the package or the volume is not there.
///
/// Its own function because several callers want it and some of them hold a transaction —
/// [`next_number`]'s reason for being one, and the same `&Connection` that a `Transaction` derefs to.
fn start_number(
    conn: &rusqlite::Connection,
    package_id: &str,
    volume: u32,
) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT start_number FROM package_volumes WHERE package_id = ?1 AND volume = ?2",
        params![package_id, volume],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DbError::NotFound(format!("package {package_id}")))
}

/// The highest volume a package has, which is where a hand add appends.
fn last_volume(conn: &rusqlite::Connection, package_id: &str) -> Result<u32, DbError> {
    let last: Option<i64> = conn.query_row(
        "SELECT MAX(volume) FROM package_volumes WHERE package_id = ?1",
        [package_id],
        |row| row.get(0),
    )?;
    last.map(|volume| volume as u32)
        .ok_or_else(|| DbError::NotFound(format!("package {package_id}")))
}

/// Every volume of a package with its first number, in order.
fn volume_starts(
    conn: &rusqlite::Connection,
    package_id: &str,
) -> Result<Vec<(i64, i64)>, DbError> {
    let mut statement = conn.prepare(
        "SELECT volume, start_number FROM package_volumes WHERE package_id = ?1 ORDER BY volume",
    )?;
    let rows = statement.query_map([package_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Starts a volume under an id of its own, numbered from 1 at the first version a package takes.
fn add_volume(
    conn: &rusqlite::Connection,
    package_id: &str,
    volume: i64,
    now: &str,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO package_volumes(package_id, volume, id, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![package_id, volume, km_kmpkg::PackageMeta::new_id(), now],
    )?;
    Ok(())
}

/// Puts songs into a package at the volume and number `numbers` hands out, counting the outcomes
/// apart.
///
/// **A free function over the connection, for [`next_number`]'s reason**: the two callers number
/// differently and must not *place* differently. [`Db::add_to_package`] hands it the append it has
/// always done and [`Db::package_room`] promises; [`Db::sync_package`] hands it [`free_numbers`]
/// across every volume, which fills the gaps it has just made. Everything else — the already-there
/// skip, the clash count, the best-file lookup, the insert — is one copy.
///
/// **Both checks read the whole package, not one volume.** A song is in a package once whichever
/// volume numbered it, and a second file of a recording is a clash wherever the first one sits.
///
/// **Running out of numbers is the iterator running out**, which is the ceiling said once rather than
/// once per caller. The loop runs on instead of stopping: an exhausted iterator answers every later
/// song the same way, and the ids behind it may include songs the package already holds, which a
/// caller has to be able to tell apart from these to name the right remedy.
fn place_songs(
    conn: &rusqlite::Connection,
    package_id: &str,
    song_ids: &[String],
    numbers: &mut dyn Iterator<Item = (i64, i64)>,
    now: &str,
) -> Result<Added, DbError> {
    let mut added = 0usize;
    let mut already = 0usize;
    let mut no_room = 0usize;
    let mut clashed = 0usize;

    let mut exists =
        conn.prepare("SELECT 1 FROM package_songs WHERE package_id = ?1 AND song_id = ?2")?;
    let mut best_file = conn.prepare(BEST_FILE_SQL)?;
    let mut insert = conn.prepare(
        "INSERT INTO package_songs(package_id, volume, number, song_id, file_id, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    // Whether this package already holds a *different file of the same recording*, which is what
    // `exists` above cannot see: that one asks about this very song. Read fresh on every iteration
    // rather than once, so two versions arriving in one batch catch each other -- the second sees the
    // first already inserted.
    let mut clashes = conn.prepare(
        "SELECT 1 FROM package_songs ps
         JOIN songs other ON other.id = ps.song_id
         JOIN songs mine ON mine.id = ?2
         WHERE ps.package_id = ?1
           AND coalesce(other.duplicate_of, other.id)
             = coalesce(mine.duplicate_of, mine.id)
         LIMIT 1",
    )?;

    for song_id in song_ids {
        if exists
            .query_row(params![package_id, song_id], |_| Ok(()))
            .optional()?
            .is_some()
        {
            already += 1;
            continue;
        }
        // Asked after the skip above, so a song the package already holds spends no number.
        let Some((volume, number)) = numbers.next() else {
            no_room += 1;
            continue;
        };
        // Counted, never refused. `km_kmpkg`'s `DuplicateContent` refuses two entries with the same
        // bytes because that is a catalog defect; two versions of one recording is a judgment, and a
        // curator may mean it -- an acoustic take and a full arrangement are the same recording to a
        // fingerprint and two songs to a singer.
        if clashes
            .query_row(params![package_id, song_id], |_| Ok(()))
            .optional()?
            .is_some()
        {
            clashed += 1;
        }
        let file_id: Option<i64> = best_file
            .query_row([song_id], |row| row.get(0))
            .optional()?;
        insert.execute(params![package_id, volume, number, song_id, file_id, now])?;
        added += 1;
    }

    Ok(Added {
        added: added as u32,
        already: already as u32,
        no_room: no_room as u32,
        clashed: clashed as u32,
        full: false,
    })
}

/// The file a package reads for a song: the first readable copy by path.
///
/// A constant because a placement and a replacement both choose one, and two choices would build two
/// different packages out of the same members.
const BEST_FILE_SQL: &str =
    "SELECT id FROM files WHERE song_id = ?1 AND scan_status = 'ok' ORDER BY path LIMIT 1";

/// Reads what [`Db::replace_in_package`] would do, over a connection or the transaction that writes.
fn plan_replacement(
    conn: &rusqlite::Connection,
    package_id: &str,
    volume: u32,
    number: u32,
    new_song_id: &str,
) -> Result<Result<Replacement, ReplaceRefusal>, DbError> {
    let package = conn
        .query_row(
            &format!("SELECT {PACKAGE_COLUMNS} WHERE p.id = ?1 AND v.volume = ?2"),
            params![package_id, volume],
            package_row,
        )
        .optional()?
        .ok_or_else(|| DbError::NotFound(format!("package {package_id} volume {volume}")))?;

    let named = format!(
        "SELECT s.id, {} AS eff_title, {} AS eff_artist, s.merged_into FROM songs s WHERE s.id = ?1",
        eff_title("s."),
        eff_artist("s."),
    );
    let song = |id: &str| {
        conn.query_row(&named, [id], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get(1)?, row.get(2)?),
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .optional()
    };
    let Some((new, merged_into)) = song(new_song_id)? else {
        return Err(DbError::NotFound(format!("song {new_song_id}")));
    };
    if merged_into.is_some() {
        return Ok(Err(ReplaceRefusal::Merged));
    }

    let old_id: Option<String> = conn
        .query_row(
            "SELECT song_id FROM package_songs WHERE package_id = ?1 AND volume = ?2 AND number = ?3",
            params![package_id, volume, number],
            |row| row.get(0),
        )
        .optional()?;
    let Some(old_id) = old_id else {
        return Ok(Err(ReplaceRefusal::Empty(package.volume_name())));
    };
    if old_id == new_song_id {
        return Ok(Err(ReplaceRefusal::SameSong));
    }
    let held: Option<(u32, u32)> = conn
        .query_row(
            "SELECT volume, number FROM package_songs WHERE package_id = ?1 AND song_id = ?2",
            params![package_id, new_song_id],
            |row| Ok((row.get::<_, i64>(0)? as u32, row.get::<_, i64>(1)? as u32)),
        )
        .optional()?;
    if let Some((held_volume, held_number)) = held {
        let name = PackageRow {
            volume: held_volume,
            ..package.clone()
        }
        .volume_name();
        return Ok(Err(ReplaceRefusal::AlreadyIn(name, held_number)));
    }
    let Some((old, _)) = song(&old_id)? else {
        return Err(DbError::NotFound(format!("song {old_id}")));
    };

    let favorites = {
        let mut statement = conn.prepare(
            "SELECT DISTINCT f.name FROM package_favorites pf
               JOIN favorites f ON f.id = pf.favorite_id
               JOIN song_favorites sf ON sf.favorite_id = pf.favorite_id
               JOIN songs src ON src.id = sf.song_id
              WHERE pf.package_id = ?1 AND coalesce(src.merged_into, src.id) = ?2
              ORDER BY f.name COLLATE NOCASE",
        )?;
        let rows = statement.query_map(params![package_id, old_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<String>, _>>()?
    };

    Ok(Ok(Replacement {
        volume_name: package.volume_name(),
        number,
        old,
        new,
        favorites,
    }))
}

/// Answers whether a volume that could not take everything is full, rather than merely high.
///
/// Read only when something was left out, because it is asked in order to choose between two
/// sentences and there is no sentence to choose when everything went in: numbers that ran to the end
/// from a high first number are re-flowed, and a volume holding every slot is full.
fn fill_full(
    conn: &rusqlite::Connection,
    package_id: &str,
    volume: u32,
    added: Added,
) -> Result<Added, DbError> {
    if added.no_room == 0 {
        return Ok(added);
    }
    let held: i64 = conn.query_row(
        "SELECT COUNT(*) FROM package_songs WHERE package_id = ?1 AND volume = ?2",
        params![package_id, volume],
        |row| row.get(0),
    )?;
    Ok(Added {
        full: held >= i64::from(km_songcode::MAX_SLOT),
        ..added
    })
}

/// The number the next song added to a volume would take.
///
/// A free function over a `&Connection` rather than a method, because the two callers reach the
/// database differently: [`Db::package_room`] holds `&self` and [`Db::add_to_package`] holds a
/// transaction, which derefs to one. **Two copies of this arithmetic is one copy that stops
/// agreeing** — a confirmation saying a package has room for four hundred, over a write that numbers
/// from somewhere else, is a number nobody can check.
///
/// A volume with no members starts at its own `start_number`, and `max` is what keeps a start that
/// was raised after the fact from handing out a number behind the ones already given.
fn next_number(conn: &rusqlite::Connection, package_id: &str, volume: u32) -> Result<i64, DbError> {
    let start = start_number(conn, package_id, volume)?;
    let next: i64 = conn.query_row(
        "SELECT coalesce(MAX(number) + 1, ?3) FROM package_songs
          WHERE package_id = ?1 AND volume = ?2",
        params![package_id, volume, start],
        |row| row.get(0),
    )?;
    Ok(next.max(start))
}

/// The `UPDATE` that records a merge, and refuses one that would chain or point at itself.
///
/// A constant because it is needed in two places that cannot call one another: `Db::set_merged_into`
/// and the merge pass inside `Db::apply_restore`, which holds the transaction and therefore cannot
/// borrow `&self` to reach a method. **Two copies of a guard is one copy that stops being a guard.**
///
/// The rule is `Db::resolve_duplicate`'s: merging is one level deep, so "the song this really is" is
/// always one hop away, which every query that hides a merged song assumes. The check is in the
/// `WHERE` rather than in a read beforehand, because a chain that formed between a check and a write
/// would be the kind of fault nobody could ever reproduce.
///
/// Named rather than ordinal, so no doc line has to say "`?1` the song, `?2` its target, `?3`
/// whether an existing merge may be replaced": the statement says it. Such a line exists only
/// where the SQL cannot, which is the same reason `write_scanned` is named too.
pub(super) const MERGE_SQL: &str = "UPDATE songs SET merged_into = :target
     WHERE id = :song
       AND :target <> :song
       AND (:overwrite OR merged_into IS NULL)
       AND EXISTS (SELECT 1 FROM songs t WHERE t.id = :target AND t.merged_into IS NULL)";
