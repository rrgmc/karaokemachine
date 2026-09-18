//! What a song is, as the browser asks about it: browsing, searching the words, and editing.
//!
//! The three sections the file is in are the three shapes a question about songs takes. **Browsing**
//! is a `Filter` turned into a `WHERE` and answered from an index — every sort here is an index seek
//! rather than a sort of the corpus, which `tests.rs` asserts for each one because at corpus size
//! the difference is 9.71 ms against 4.21 s. **Searching the words** is FTS5, and is separate because
//! it is the one place a person's text could reach SQL as syntax rather than data. **Editing** is
//! every write to a song row, including the bulk ones that take a filter rather than a list of ids.
//!
//! Tags and language are here rather than in files of their own: a tag is a row in a side table but
//! it is *asked about* as a property of a song, and every one of these methods is either a narrowing
//! of the browse query or a write that the browse query then has to see.

use super::*;

impl Db {
    /// Runs a browse query, returning at most `filter.limit` rows.
    ///
    /// **A test's convenience.** Every page in the running tool wants the pair
    /// [`Self::songs_page`] answers — the rows and whether there is another page — so this exists
    /// only to keep forty assertions from unwrapping a tuple, and is `#[cfg(test)]` so it cannot
    /// quietly become a second way to browse.
    #[cfg(test)]
    pub fn songs(&self, filter: &Filter) -> Result<Vec<SongRow>, DbError> {
        Ok(self.songs_page(filter)?.0)
    }

    /// The same query, and whether a row exists beyond the page it returned.
    ///
    /// **One row more than the page is asked for, and the extra one is thrown away.** That single
    /// spare row is what tells the paging buttons whether there is a next page — a question the
    /// browse page used to answer with `SELECT COUNT(*)` over the whole filtered corpus, on every
    /// page turn, to compare against the offset. Counting the whole corpus to decide whether to draw one
    /// button was the single most expensive thing the page did, and the answer was always already
    /// sitting one row past the `LIMIT`.
    ///
    /// It costs nothing measurable: the extra row comes from the same index scan already positioned
    /// there, so it is one more step of a cursor rather than a second pass.
    ///
    /// The count has *not* gone away entirely — the "page 1 of N" label still wants a real
    /// total, and the five-pages-on button still has to clamp to the last page. What has changed is
    /// that those need it **once per filter** rather than once per page turn; see `page_links`.
    pub fn songs_page(&self, filter: &Filter) -> Result<(Vec<SongRow>, bool), DbError> {
        let (where_sql, bindings) = filter.to_sql();
        let sql = format!(
            "SELECT {} FROM songs s
             WHERE {where_sql}
             ORDER BY {}
             LIMIT ?{} OFFSET ?{}",
            browse_columns(),
            filter.order_by(),
            bindings.len() + 1,
            bindings.len() + 2,
        );

        let mut values = bindings;
        values.push(Binding::Integer(i64::from(filter.limit.saturating_add(1))));
        values.push(Binding::Integer(i64::from(filter.offset)));

        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values.iter()), song_row)?;
        let mut rows = rows.collect::<Result<Vec<_>, _>>()?;
        let has_more = rows.len() > filter.limit as usize;
        rows.truncate(filter.limit as usize);
        self.fill_tags(&mut rows)?;
        Ok((rows, has_more))
    }

    /// One row of the browse table, by content hash.
    ///
    /// The same columns the list itself selects, so a row re-rendered after an edit is the row the
    /// list would have drawn. A song that has been merged away still answers here: the row is being
    /// fetched because something on the page refers to it, and a hole where a row was is worse than a
    /// row that says what happened.
    pub fn song_row(&self, id: &str) -> Result<SongRow, DbError> {
        let sql = format!("SELECT {} FROM songs s WHERE s.id = ?1", browse_columns());
        let mut statement = self.conn.prepare(&sql)?;
        let mut row = statement
            .query_row([id], song_row)
            .optional()?
            .ok_or_else(|| DbError::NotFound(format!("song {id}")))?;
        row.tags = self.tags_of(id)?;
        Ok(row)
    }

    /// Fills in the tags of a page of rows, in one query rather than one per row.
    ///
    /// **Not a join in `browse_columns`**, and the reason is what a join would do to the paging:
    /// `songs_page` selects `limit + 1` rows to learn whether there is another page, and a row that
    /// multiplied by its tags would make that count meaningless. A second query over the ids the
    /// page actually holds is the same one round trip and cannot lie about the page boundary.
    pub(super) fn fill_tags(&self, rows: &mut [SongRow]) -> Result<(), DbError> {
        if rows.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let mut found = self.tags_of_many(&ids)?;
        for row in rows.iter_mut() {
            row.tags = found.remove(&row.id).unwrap_or_default();
        }
        Ok(())
    }

    // -- searching the words ----------------------------------------------------------------

    /// Songs whose lyrics match, best first, with the matching passage from each.
    ///
    /// The rows are the browse table's rows — the same [`browse_columns`] every other list selects —
    /// so a hit carries its play button, its star and its score selects and can be curated where it
    /// was found. Only the passage is extra.
    ///
    /// Ordered by `bm25`, which is what separates this from the browse page: there the sort is a
    /// property of the song, here it is how well the words matched, and a search for a half-heard
    /// line wants the song that sings it most, not the one with the highest suitability.
    pub fn lyric_search(&self, search: &LyricSearch) -> Result<Vec<LyricHit>, DbError> {
        let Some(query) = search.match_query() else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT {},
                    -- The markers are two control characters rather than `<b>`/`</b>`, because this
                    -- is arbitrary text out of an arbitrary file and it is about to be put on a
                    -- page. `views::highlight` cuts it into (text, matched) pairs which the template
                    -- escapes one by one, so no lyric can ever become markup. A file that genuinely
                    -- contains a STX draws one wrong highlight; the alternative risked worse.
                    snippet(lyrics_fts, 0, char(2), char(3), char(8230), {SNIPPET_TOKENS})
             FROM lyrics_fts
             JOIN songs s ON s.rowid = lyrics_fts.rowid
             WHERE lyrics_fts MATCH ?1 AND s.merged_into IS NULL
             ORDER BY bm25(lyrics_fts)
             LIMIT ?2 OFFSET ?3",
            browse_columns(),
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(
            params![query, search.limit, search.offset],
            |row| -> rusqlite::Result<LyricHit> {
                let song = song_row(row)?;
                // One past the browse columns, whose count is a detail of `browse_columns`.
                let passage: String = row.get(row.as_ref().column_count() - 1)?;
                Ok(LyricHit { song, passage })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// How many songs the lyric search matches, ignoring its paging.
    pub fn lyric_search_count(&self, search: &LyricSearch) -> Result<u32, DbError> {
        let Some(query) = search.match_query() else {
            return Ok(0);
        };
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM lyrics_fts
             JOIN songs s ON s.rowid = lyrics_fts.rowid
             WHERE lyrics_fts MATCH ?1 AND s.merged_into IS NULL",
            [query],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }

    /// Songs whose name is like `title` and `artist`, likeliest first, headed by the song `from`.
    ///
    /// **The song the search started from comes first, whatever its likeness**, so every other row is
    /// read against it. It is fetched by id rather than left to the index, because a name edited in
    /// the boxes can push it out of the candidates or below the threshold.
    ///
    /// **Gathered by the index and ordered in Rust.** [`crate::similar::match_query`] ORs a prefix of
    /// every word, which is wide enough to reach a misspelling and is answered by `songs_fts` without
    /// reading the table; `bm25` puts the files sharing the most and rarest words first, and the first
    /// [`crate::similar::CANDIDATES`] of those are scored by [`crate::similar::likeness`]. What clears
    /// [`crate::similar::THRESHOLD`] is sorted and cut to [`crate::similar::SHOWN`].
    ///
    /// **`filter` narrows the candidates before they are scored**, so a match the filter keeps is
    /// reached even when unfiltered files crowd it past [`crate::similar::CANDIDATES`]. The song
    /// searched from ignores it: every other row is read against that one.
    ///
    /// **A file another has been merged into is left out**, as the lyric search leaves it out: it is
    /// reached through that song. A file the duplicate pass hid as a version is left out only when
    /// `filter` collapses versions, which the page does unless every version is asked for.
    pub fn similar_names(
        &self,
        title: &str,
        artist: &str,
        from: &str,
        filter: &Filter,
    ) -> Result<Vec<SongRow>, DbError> {
        let Some(query) = crate::similar::match_query(title, artist) else {
            return Ok(Vec::new());
        };
        let (where_sql, bindings) = filter.to_sql();
        let sql = format!(
            "SELECT {}
             FROM songs_fts
             JOIN songs s ON s.rowid = songs_fts.rowid
             WHERE songs_fts MATCH ?{} AND s.id <> ?{} AND {where_sql}
             ORDER BY bm25(songs_fts)
             LIMIT ?{}",
            browse_columns(),
            bindings.len() + 1,
            bindings.len() + 2,
            bindings.len() + 3,
        );
        let mut values = bindings;
        values.push(Binding::Text(query));
        values.push(Binding::Text(from.to_owned()));
        values.push(Binding::Integer(i64::from(crate::similar::CANDIDATES)));
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values.iter()), song_row)?;
        let mut hits = Vec::new();
        for row in rows {
            let mut song = row?;
            let likeness = crate::similar::likeness(
                title,
                artist,
                &song.title,
                song.artist.as_deref().unwrap_or(""),
            );
            if likeness >= crate::similar::THRESHOLD {
                song.likeness = Some(likeness);
                hits.push((likeness, song));
            }
        }
        // Stable, so equally alike names keep the index's order, which puts the rarer shared words
        // first.
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut hits: Vec<SongRow> = hits.into_iter().map(|(_, song)| song).collect();

        let origin = self
            .conn
            .query_row(
                &format!(
                    "SELECT {} FROM songs s WHERE s.id = ?1 AND s.merged_into IS NULL",
                    browse_columns()
                ),
                [from],
                song_row,
            )
            .optional()?;
        if let Some(mut song) = origin {
            song.likeness = Some(crate::similar::likeness(
                title,
                artist,
                &song.title,
                song.artist.as_deref().unwrap_or(""),
            ));
            song.searched_from = true;
            hits.insert(0, song);
        }
        hits.truncate(crate::similar::SHOWN);
        Ok(hits)
    }

    /// Whether any song's words have been indexed at all.
    ///
    /// `EXISTS` rather than a count on purpose: the question the page asks is "has a scan ever
    /// written lyrics here?", and on a database indexed by an earlier version the answer is no for
    /// every row — counting them would walk the whole index to say so. It is what tells an empty
    /// result page whether to say *nothing matches* or *nothing is indexed yet, run a full scan*,
    /// which are different problems with different fixes.
    pub fn lyrics_indexed(&self) -> Result<bool, DbError> {
        let any: i64 =
            self.conn
                .query_row("SELECT EXISTS(SELECT 1 FROM lyrics_fts)", [], |row| {
                    row.get(0)
                })?;
        Ok(any != 0)
    }

    /// How many songs a browse query matches, ignoring its paging.
    pub fn song_count(&self, filter: &Filter) -> Result<u32, DbError> {
        let (where_sql, bindings) = filter.to_sql();
        let sql = format!("SELECT COUNT(*) FROM songs s WHERE {where_sql}");
        let mut statement = self.conn.prepare(&sql)?;
        let count: i64 =
            statement.query_row(params_from_iter(bindings.iter()), |row| row.get(0))?;
        Ok(count as u32)
    }

    /// One song, by content hash.
    pub fn song(&self, id: &str) -> Result<SongDetail, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT id, det_title, det_artist, det_language, title, artist, language,
                    lyric_encoding, default_transpose, flavor, granularity, duration_ms,
                    note_count, channel_count, line_count, syllable_count, det_encoding,
                    det_encoding_source, melody_channel, melody_confidence, melody_abstained,
                    suitability, suitability_lyrics, suitability_sync, suitability_channels,
                    suitability_arrangement, warnings,
                    user_score, notes, merged_into, stem,
                    kind, width, height, frame_rate_milli, video_codec, audio_codec,
                    det_language_tag,
                    cdg_graphics_path, cdg_sample_rate, cdg_channels, cdg_packets,
                    cdg_graphics_ms, cdg_short_by_ms, cdg_tiles, cdg_unknown,
                    duplicate_of, fixes, melody_chosen, first_seen
             FROM songs WHERE id = ?1",
        )?;
        let detail = statement
            .query_row([id], |row| {
                let kind = SongKind::from_str(&row.get::<_, String>(31)?);
                // Built from `kind` rather than from whether the columns happen to be NULL. A MIDI
                // row written before the rebuild has every one of them filled, and a video row has
                // none — but reading the column that says so is what keeps a half-written row from
                // being silently reinterpreted as the other kind of song.
                let midi = kind
                    .is_midi()
                    .then(|| {
                        Ok::<_, rusqlite::Error>(MidiDetail {
                            flavor: row.get(9)?,
                            granularity: row.get(10)?,
                            note_count: row.get::<_, i64>(12)? as u32,
                            channel_count: row.get::<_, i64>(13)? as u32,
                            line_count: row.get::<_, i64>(14)? as u32,
                            syllable_count: row.get::<_, i64>(15)? as u32,
                            det_encoding: row.get(16)?,
                            det_encoding_source: row.get(17)?,
                            melody_channel: row.get::<_, Option<i64>>(18)?.map(|v| v as u8),
                            melody_confidence: row.get(19)?,
                            melody_abstained: row.get(20)?,
                            suitability: row.get::<_, i64>(21)? as u8,
                            suitability_lyrics: row.get::<_, i64>(22)? as u8,
                            suitability_sync: row.get::<_, i64>(23)? as u8,
                            suitability_channels: row.get::<_, i64>(24)? as u8,
                            suitability_arrangement: row.get::<_, i64>(25)? as u8,
                            warnings: row.get(26)?,
                        })
                    })
                    .transpose()?;
                let video = kind
                    .is_video()
                    .then(|| {
                        Ok::<_, rusqlite::Error>(VideoFacts {
                            width: row.get::<_, Option<i64>>(32)?.unwrap_or(0) as u32,
                            height: row.get::<_, Option<i64>>(33)?.unwrap_or(0) as u32,
                            frame_rate_milli: row.get::<_, Option<i64>>(34)?.unwrap_or(0) as u32,
                            video_codec: row.get::<_, Option<String>>(35)?.unwrap_or_default(),
                            audio_codec: row.get::<_, Option<String>>(36)?.unwrap_or_default(),
                        })
                    })
                    .transpose()?;
                let cdg = kind
                    .is_cdg()
                    .then(|| {
                        Ok::<_, rusqlite::Error>(CdgFacts {
                            graphics_path: row.get::<_, Option<String>>(38)?.unwrap_or_default(),
                            sample_rate: row.get::<_, Option<i64>>(39)?.unwrap_or(0) as u32,
                            channels: row.get::<_, Option<i64>>(40)?.unwrap_or(0) as u16,
                            packets: row.get::<_, Option<i64>>(41)?.unwrap_or(0) as u32,
                            graphics_ms: row.get::<_, Option<i64>>(42)?.unwrap_or(0) as u32,
                            graphics_short_by_ms: row.get::<_, Option<i64>>(43)?.unwrap_or(0)
                                as u32,
                            tiles_written: row.get::<_, Option<i64>>(44)?.unwrap_or(0) as u32,
                            unknown_instructions: row.get::<_, Option<i64>>(45)?.unwrap_or(0)
                                as u32,
                        })
                    })
                    .transpose()?;
                Ok(SongDetail {
                    id: row.get(0)?,
                    det_title: row.get(1)?,
                    det_artist: row.get(2)?,
                    det_language: row.get(3)?,
                    title: row.get(4)?,
                    artist: row.get(5)?,
                    language: row.get(6)?,
                    lyric_encoding: row.get(7)?,
                    default_transpose: row.get(8)?,
                    kind,
                    duration_ms: row.get::<_, i64>(11)? as u32,
                    midi,
                    video,
                    cdg,
                    user_score: row.get::<_, Option<i64>>(27)?.map(|v| v as u8),
                    notes: row.get(28)?,
                    merged_into: row.get(29)?,
                    stem: row.get(30)?,
                    // Appended to the SELECT rather than slotted in beside `det_language`, because
                    // every `row.get(N)` above is positional and an insertion renumbers the lot.
                    det_language_tag: row.get(37)?,
                    // Appended, so it is one past `cdg_unknown` at 46 -- the rule the note on
                    // `det_language_tag` above states, since every `row.get(N)` here is positional.
                    duplicate_of: row.get(46)?,
                    fixes: row.get(47)?,
                    // Appended for the same positional reason, one past `fixes` at 47.
                    melody_chosen: row.get(48)?,
                    first_seen: row.get(49)?,
                    files: Vec::new(),
                    favorites: Vec::new(),
                    packages: Vec::new(),
                })
            })
            .optional()?;

        let mut detail = detail.ok_or_else(|| DbError::NotFound(format!("song {id}")))?;
        detail.files = self.files_for(id)?;
        detail.favorites = self.favorites_for(id)?;
        detail.packages = self.packages_for(id)?;
        Ok(detail)
    }

    /// The files on disk that are copies of a song, best first.
    pub fn files_for(&self, id: &str) -> Result<Vec<SongFile>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT path, size FROM files WHERE song_id = ?1 ORDER BY path")?;
        let rows = statement.query_map([id], |row| {
            Ok(SongFile {
                path: row.get(0)?,
                size: row.get::<_, i64>(1)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The path a song should be played or packaged from: its first surviving file.
    pub fn best_file(&self, id: &str) -> Result<(i64, PathBuf), DbError> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, path FROM files WHERE song_id = ?1 AND scan_status = 'ok'
                 ORDER BY path LIMIT 1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (file_id, relative) =
            row.ok_or_else(|| DbError::NotFound(format!("a readable file for song {id}")))?;
        Ok((file_id, contained(&self.root, &relative)?))
    }

    // -- editing ----------------------------------------------------------------------------

    /// Applies a person's corrections. `None` in a field means "leave alone"; `Some(None)` clears it.
    pub fn edit_song(&self, id: &str, edit: &SongEdit) -> Result<(), DbError> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Binding> = Vec::new();

        if let Some(title) = &edit.title {
            sets.push("title = ?");
            values.push(match title {
                Some(value) => Binding::Text(value.clone()),
                None => Binding::Null,
            });
        }
        if let Some(artist) = &edit.artist {
            sets.push("artist = ?");
            values.push(match artist {
                Some(value) => Binding::Text(value.clone()),
                None => Binding::Null,
            });
        }
        if let Some(language) = &edit.language {
            sets.push("language = ?");
            values.push(match language {
                // Refused rather than stored, following `set_score` refusing a rating over 10: a
                // column that a filter and a sort read cannot hold a value nothing can match.
                // Canonicalised on the way in, so `PT` typed into a URL is stored as `pt` and the
                // comparisons stay exact.
                Some(value) => Binding::Text(
                    Language::parse(value)
                        .ok_or_else(|| {
                            DbError::Rejected(format!(
                                "{value:?} is not a language code -- the Language box lists them"
                            ))
                        })?
                        .code()
                        .to_owned(),
                ),
                None => Binding::Null,
            });
        }
        if let Some(encoding) = &edit.lyric_encoding {
            sets.push("lyric_encoding = ?");
            values.push(match encoding {
                Some(value) => Binding::Text(value.clone()),
                None => Binding::Null,
            });
        }
        if let Some(transpose) = &edit.default_transpose {
            sets.push("default_transpose = ?");
            values.push(match transpose {
                Some(value) => Binding::Integer(i64::from(*value)),
                None => Binding::Null,
            });
        }
        if let Some(notes) = &edit.notes {
            sets.push("notes = ?");
            values.push(match notes {
                Some(value) => Binding::Text(value.clone()),
                None => Binding::Null,
            });
        }
        if let Some(fixes) = &edit.fixes {
            sets.push("fixes = ?");
            values.push(match fixes {
                Some(value) => Binding::Text(value.clone()),
                None => Binding::Null,
            });
        }
        // `melody_channel` is untouched here on purpose: that column is what detection found, and a
        // person's answer lives beside it rather than over it, so a rescan keeps recording what the
        // file implies and the answer outlives it.
        if let Some(chosen) = &edit.melody_chosen {
            sets.push("melody_chosen = ?");
            values.push(match chosen {
                Some(value) => Binding::Text(value.clone()),
                None => Binding::Null,
            });
        }
        if sets.is_empty() {
            return Ok(());
        }

        let sql = format!("UPDATE songs SET {} WHERE id = ?", sets.join(", "));
        values.push(Binding::Text(id.to_owned()));
        // A transaction for one row, so the edit and the sort key it invalidates land together. The
        // key is not computed from the bound value: clearing a title falls the effective one back
        // to `det_title` or the file name, and this function does not know either — `refold` reads
        // what the write actually produced.
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(&sql, params_from_iter(values.iter()))?;
        if changed == 0 {
            return Err(DbError::NotFound(format!("song {id}")));
        }
        Self::refold(&transaction)?;
        transaction.commit()?;
        Ok(())
    }

    /// Sets or clears the person's own rating.
    pub fn set_user_score(&self, id: &str, score: Option<u8>) -> Result<(), DbError> {
        self.set_score("user_score", id, score)
    }

    /// Every language this corpus actually holds, in code order.
    ///
    /// The language pickers are built from this rather than from the standard. `km_kmpkg` carries
    /// all 186 ISO 639-1 codes, which is right for a *table* and useless as a *dropdown*: a corpus
    /// sorted into `Brasil/`, `Ingles/` and `japanese/` has three, and finding them among 186 is the
    /// interaction this replaces. Where the picker also has to be able to *set* a language the corpus
    /// has never held, the full list is offered beside this one — see `Choice::languages_grouped`.
    ///
    /// **A loose index scan, not a `SELECT DISTINCT`.** The distinct answer is a handful of rows and
    /// the table is hundreds of thousands, so a plain `DISTINCT` walks every entry of
    /// `songs_browse_language_artist` to produce four strings — on every page render. The recursive form
    /// below asks for the smallest tag, then the smallest greater than that, and so on: one index
    /// seek per language that exists, and nothing per song. `eff_language` is inlined for the reason
    /// `create_browse_indexes` gives — SQLite matches an expression index tree for tree, so a second
    /// spelling of the expression would silently stop the index being used.
    ///
    /// `merged_into IS NULL` because a merged song is not in the list either, and matching the
    /// index's own partial predicate is what lets it be used at all.
    pub fn languages_present(&self) -> Result<Vec<Language>, DbError> {
        let language = eff_language("s.");
        let sql = format!(
            "WITH RECURSIVE present(tag) AS (
                 SELECT min({language}) FROM songs s
                  WHERE s.merged_into IS NULL AND {language} IS NOT NULL
                 UNION ALL
                 SELECT (SELECT min({language}) FROM songs s
                          WHERE s.merged_into IS NULL AND {language} > present.tag)
                   FROM present WHERE present.tag IS NOT NULL)
             SELECT tag FROM present WHERE tag IS NOT NULL"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let tags = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<String>, _>>()?;
        // A tag the table does not know is dropped rather than shown. `edit_song` rejects one on the
        // way in and `Language::detect` only ever writes one it produced, so this should be empty —
        // but a database hand-edited with `sqlite3` is a database, and a picker offering an option
        // that cannot be selected back is worse than a shorter picker.
        Ok(tags.iter().filter_map(|tag| Language::parse(tag)).collect())
    }

    /// Every tag any song here carries, in picker order.
    ///
    /// **A plain read of the `tags` table**, where [`Self::languages_present`] beside it has to skip-
    /// scan `songs` with a recursive CTE — which is the whole reason that table exists. A language is
    /// a column on a song, so the distinct set has to be computed; a tag has a vocabulary of its own,
    /// kept in step by [`Self::add_tag_of`] and its neighbours.
    ///
    /// Ordered by the fold rather than by count, unlike the machine's own picker: this is a datalist
    /// somebody is typing into, so alphabetical is what makes an entry findable. Popularity is the
    /// right order for a filter offering a short list; it is the wrong one for a lookup.
    pub fn tags_present(&self) -> Result<Vec<String>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT name FROM tags ORDER BY sort_key, name")?;
        let tags = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(tags)
    }

    /// The tags one song carries, sorted.
    pub fn tags_of(&self, id: &str) -> Result<Vec<String>, DbError> {
        let mut statement = self
            .conn
            .prepare("SELECT tag FROM song_tags WHERE song_id = ?1 ORDER BY tag")?;
        let tags = statement
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(tags)
    }

    /// The tags of every song named, keyed by id.
    ///
    /// One query for a whole page rather than one per row — the same reason `starred_set` exists in
    /// the remote, and the difference between one round trip and fifty when a row draws its tags.
    pub fn tags_of_many(&self, ids: &[String]) -> Result<HashMap<String, Vec<String>>, DbError> {
        let mut found: HashMap<String, Vec<String>> = HashMap::new();
        for batch in ids.chunks(ID_CHUNK) {
            let (holes, values) = id_holes(batch, 1);
            let sql = format!(
                "SELECT song_id, tag FROM song_tags WHERE song_id IN ({holes}) ORDER BY tag"
            );
            let mut statement = self.conn.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, tag) = row?;
                found.entry(id).or_default().push(tag);
            }
        }
        Ok(found)
    }

    /// The songs named, best first, for the quality hint the Songs page draws on ticked rows.
    ///
    /// **MIDI files only, and the rest are dropped rather than ordered badly.** A video song and an
    /// MP3+G song are a flat 10 by what they are rather than by measurement — see
    /// `Suitability, for a song that was made to be sung to` in `docs/decisions/songs.md` — and
    /// every component behind that 10 is a fill rather than a reading, so an order over them would
    /// be the id tie-break wearing a number. The handler says how many it left out.
    ///
    /// **The order is decided in Rust, by [`crate::hint::order`], and not by an `ORDER BY`.** No
    /// index covers seven keys over an arbitrary list of ids, the list is at most a page of ticks,
    /// and a comparator can be read and tested against the reasoning that produced it where a
    /// formatted `ORDER BY` cannot. What SQL does here is fetch the columns.
    ///
    /// **Every column fetched is one a scan wrote, and `user_score` is not among them**, for the
    /// reason [`crate::hint::order`] gives: a rating says how much this song is wanted in a package
    /// rather than which copy of it is the better file.
    ///
    /// Chunked at [`ID_CHUNK`] for [`Self::add_tag_of`]'s reason: SQLite binds at most 999
    /// parameters. Ordering happens once over everything the chunks found, so a set larger than one
    /// chunk is ordered as one set.
    pub fn quality_hint(&self, ids: &[String]) -> Result<Vec<String>, DbError> {
        let mut keys = Vec::with_capacity(ids.len());
        for batch in ids.chunks(ID_CHUNK) {
            let (holes, values) = id_holes(batch, 1);
            let sql = format!(
                "SELECT s.id, s.suitability, s.suitability_lyrics,
                        s.suitability_sync, s.suitability_arrangement, s.channel_count,
                        s.det_encoding_source, s.file_count
                   FROM songs s
                  WHERE s.id IN ({holes}) AND s.kind = 'midi'"
            );
            let mut statement = self.conn.prepare(&sql)?;
            let found = statement.query_map(params_from_iter(values.iter()), |row| {
                Ok(crate::hint::Key {
                    id: row.get(0)?,
                    suitability: row.get::<_, Option<i64>>(1)?.map(|v| v as u8),
                    lyrics: row.get::<_, Option<i64>>(2)?.map(|v| v as u8),
                    sync: row.get::<_, Option<i64>>(3)?.map(|v| v as u8),
                    arrangement: row.get::<_, Option<i64>>(4)?.map(|v| v as u8),
                    channel_count: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                    encoding_source: row.get(6)?,
                    file_count: row.get::<_, i64>(7)? as u32,
                })
            })?;
            for key in found {
                keys.push(key?);
            }
        }
        Ok(crate::hint::order(keys))
    }

    /// Adds a tag to the songs named. Returns how many rows were written.
    ///
    /// **`add` and `remove`, and there is no `replace`** — see `Assigning tags in bulk` in
    /// `docs/decisions/curation.md`. A song carries several tags, so a *set* would silently destroy
    /// tagging work done elsewhere, and this is an action a filter can point at a quarter of a
    /// million rows.
    ///
    /// Chunked at [`ID_CHUNK`] for [`Self::set_language_of`]'s reason: SQLite binds at most 999
    /// parameters and the header tick-box ticks a whole page.
    pub fn add_tag_of(&self, ids: &[String], tag: &Tag) -> Result<u32, DbError> {
        self.remember_tag(tag)?;
        let mut changed = 0u32;
        for batch in ids.chunks(ID_CHUNK) {
            let (holes, mut values) = id_holes(batch, 2);
            let sql = format!(
                "INSERT OR IGNORE INTO song_tags (song_id, tag)
                 SELECT id, ?1 FROM songs WHERE id IN ({holes}) AND merged_into IS NULL"
            );
            values.insert(0, Binding::Text(tag.as_str().to_owned()));
            changed += self.conn.execute(&sql, params_from_iter(values.iter()))? as u32;
        }
        Ok(changed)
    }

    /// Takes a tag off the songs named. Returns how many rows went.
    pub fn remove_tag_of(&self, ids: &[String], tag: &Tag) -> Result<u32, DbError> {
        let mut changed = 0u32;
        for batch in ids.chunks(ID_CHUNK) {
            let (holes, mut values) = id_holes(batch, 2);
            let sql = format!("DELETE FROM song_tags WHERE tag = ?1 AND song_id IN ({holes})");
            values.insert(0, Binding::Text(tag.as_str().to_owned()));
            changed += self.conn.execute(&sql, params_from_iter(values.iter()))? as u32;
        }
        self.forget_unused_tag(tag)?;
        Ok(changed)
    }

    /// Adds a tag to every song a filter matches. Returns how many rows were written.
    ///
    /// **The `WHERE` is [`Filter::to_sql`] verbatim**, the same discipline
    /// [`Self::set_language_for`] rests on: what the list shows and what this writes are one clause,
    /// so they cannot diverge, and `merged_into IS NULL` is inherited rather than remembered.
    pub fn add_tag_for(&self, filter: &Filter, tag: &Tag) -> Result<u32, DbError> {
        self.remember_tag(tag)?;
        let (where_clause, mut values) = filter.to_sql();
        values.push(Binding::Text(tag.as_str().to_owned()));
        let sql = format!(
            "INSERT OR IGNORE INTO song_tags (song_id, tag)
             SELECT s.id, ?{} FROM songs s WHERE {where_clause}",
            values.len()
        );
        let changed = self.conn.execute(&sql, params_from_iter(values.iter()))?;
        Ok(changed as u32)
    }

    /// Takes a tag off every song a filter matches. Returns how many rows went.
    pub fn remove_tag_for(&self, filter: &Filter, tag: &Tag) -> Result<u32, DbError> {
        let (where_clause, mut values) = filter.to_sql();
        values.push(Binding::Text(tag.as_str().to_owned()));
        let sql = format!(
            "DELETE FROM song_tags WHERE tag = ?{}
               AND song_id IN (SELECT s.id FROM songs s WHERE {where_clause})",
            values.len()
        );
        let changed = self.conn.execute(&sql, params_from_iter(values.iter()))?;
        self.forget_unused_tag(tag)?;
        Ok(changed as u32)
    }

    /// Puts a tag in the vocabulary, if it is not already there.
    fn remember_tag(&self, tag: &Tag) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO tags (name, sort_key) VALUES (?1, ?2)",
            params![tag.as_str(), km_song::text::fold(tag.as_str())],
        )?;
        Ok(())
    }

    /// Drops a tag from the vocabulary once its last song has lost it.
    ///
    /// So the picker is a list of words in use rather than a museum of every word ever typed —
    /// which, on a corpus this size, is the difference between a useful datalist and one nobody
    /// reads. A default tag set in the settings is offered anyway; see `settings::Settings`.
    fn forget_unused_tag(&self, tag: &Tag) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM tags WHERE name = ?1
               AND NOT EXISTS (SELECT 1 FROM song_tags WHERE tag = ?1)",
            params![tag.as_str()],
        )?;
        Ok(())
    }

    /// How many songs a filter matches, ignoring its paging.
    ///
    /// What the bulk set shows before it is confirmed. The same `Filter` the list was built from, so
    /// the number cannot disagree with what is on screen.
    pub fn count_matching(&self, filter: &Filter) -> Result<u32, DbError> {
        let (where_clause, values) = filter.to_sql();
        let sql = format!("SELECT COUNT(*) FROM songs s WHERE {where_clause}");
        let count: i64 = self
            .conn
            .query_row(&sql, params_from_iter(values.iter()), |row| row.get(0))?;
        Ok(count as u32)
    }

    /// Sets, or clears, the language of every song a filter matches. Returns how many changed.
    ///
    /// **The `WHERE` is [`Filter::to_sql`] verbatim**, which is the whole design: what the list shows
    /// and what this writes are built from the same clause, so they cannot diverge — and
    /// `merged_into IS NULL` is inherited rather than remembered. The filter's `limit` and `offset`
    /// are deliberately not applied: the action is over everything matching, not over the page.
    ///
    /// This is the tool's first bulk action, and it exists because of a measurement. On the local
    /// corpus 92% of songs have no language at all — most files declare none and are
    /// written in an encoding that implies none — so classifying them one page at a time is not a
    /// thing anybody would finish.
    pub fn set_language_for(
        &self,
        filter: &Filter,
        language: Option<Language>,
    ) -> Result<u32, DbError> {
        let (where_clause, mut values) = filter.to_sql();
        values.push(match language {
            Some(language) => Binding::Text(language.code().to_owned()),
            None => Binding::Null,
        });
        let sql = format!(
            "UPDATE songs AS s SET language = ?{} WHERE {where_clause}",
            values.len()
        );
        let changed = self.conn.execute(&sql, params_from_iter(values.iter()))?;
        Ok(changed as u32)
    }

    /// Sets, or clears, the language of the songs named. Returns how many changed.
    ///
    /// **The ticked half of the bulk language set**, beside [`Self::set_language_for`]'s filter-wide
    /// one. Two statements rather than one because the two are genuinely different questions — *these
    /// songs* and *this description of songs* — and folding ids into [`Filter`] would put a list of
    /// content hashes inside the type whose whole job is to be the browse query.
    ///
    /// **Chunked**, because SQLite binds at most 999 parameters in one statement and nothing stops a
    /// caller handing this more: the header tick-box ticks a page, and a page is a constant that has
    /// moved before.
    /// `only_unset` narrows to the songs nobody has said a language for and nothing implied one for,
    /// which is [`LanguageFilter::Unset`]'s own clause — asked for rather than restated, so the
    /// ticked half and the filter-wide half cannot come to disagree about what *unset* means.
    pub fn set_language_of(
        &self,
        ids: &[String],
        language: Option<Language>,
        only_unset: bool,
    ) -> Result<u32, DbError> {
        let tag = match language {
            Some(language) => Binding::Text(language.code().to_owned()),
            None => Binding::Null,
        };
        let mut changed = 0u32;
        for batch in ids.chunks(ID_CHUNK) {
            let (holes, mut values) = id_holes(batch, 2);
            let sql = format!(
                "UPDATE songs SET language = ?1 WHERE id IN ({holes}){}",
                unset_narrowing(only_unset)
            );
            values.insert(0, tag.clone());
            changed += self.conn.execute(&sql, params_from_iter(values.iter()))? as u32;
        }
        Ok(changed)
    }

    /// How many of the songs named would be written, given the same narrowing.
    ///
    /// Its own query rather than `ids.len()`, so the confirmation's number is what the write will do
    /// rather than what was ticked: with *only songs with no language yet*, most of a ticked page is
    /// usually already classified.
    pub fn count_of(&self, ids: &[String], only_unset: bool) -> Result<u32, DbError> {
        let mut count = 0u32;
        for batch in ids.chunks(ID_CHUNK) {
            let (holes, values) = id_holes(batch, 1);
            let sql = format!(
                "SELECT COUNT(*) FROM songs WHERE id IN ({holes}){}",
                unset_narrowing(only_unset)
            );
            let found: i64 = self
                .conn
                .query_row(&sql, params_from_iter(values.iter()), |row| row.get(0))?;
            count += found as u32;
        }
        Ok(count)
    }

    /// Every song the filter matches, in the order the list was showing them.
    ///
    /// Reuses `Filter::to_sql` verbatim, so what this returns and what the browse list draws cannot
    /// come apart — the same guarantee [`Self::set_language_for`] rests on, and the reason both go
    /// through that one function rather than composing a `WHERE` of their own.
    ///
    /// **`limit` and `offset` are deliberately ignored**, exactly as they are there: this answers
    /// *everything matching*, not the page somebody is looking at.
    ///
    /// **The order is load-bearing rather than incidental.** The one caller feeds this straight into
    /// [`Self::add_to_package`], which numbers songs in the order it is given them — so a package
    /// built from a list sorted by title comes out numbered by title, which is what somebody who
    /// sorted the list before pressing the button meant.
    ///
    /// It collects the ids rather than streaming them, which is what lets the numbering stay in
    /// `add_to_package` instead of being written a second time as an `INSERT … SELECT`; the count is
    /// shown and confirmed before this is ever called.
    ///
    /// **`limit` is not an optimization, it is the arithmetic.** A package numbers its songs 1 to
    /// `km_songcode::MAX_SLOT`, and `add_to_package` places only what fits under that ceiling — so on
    /// the owner's own corpus, "make a package from this filter" with a broad filter read **every
    /// matching id** out of SQLite, sorted them, and allocated a `String` for each so that a few
    /// hundred could be used. The `ORDER BY` already decides which of them, so asking for as many as
    /// the package has free is the same answer.
    ///
    /// `None` is every match, which is what the tests and any future whole-corpus caller want.
    pub fn matching_ids(
        &self,
        filter: &Filter,
        limit: Option<u32>,
    ) -> Result<Vec<String>, DbError> {
        let (where_clause, values) = filter.to_sql();
        // **The id and nothing else.** Selecting `eff_title`, `eff_artist` and `file_count` here
        // would be for no reason but to give `order_by`'s aliases somewhere to be defined — and
        // `SELECT s.id … ORDER BY eff_title` is a *runtime* error naming a column that does not
        // exist. The sort reads stored columns, so nothing needs those aliases, and two `coalesce`
        // trees per row are not evaluated across a match set that can be the whole corpus.
        // Interpolated rather than bound, like every other `LIMIT` in this file: it is a `u32` this
        // code chose, never anything a person typed.
        let sql = format!(
            "SELECT s.id FROM songs s WHERE {where_clause} ORDER BY {}{}",
            filter.order_by(),
            limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default()
        );
        let mut statement = self.conn.prepare(&sql)?;
        let ids = statement
            .query_map(params_from_iter(values.iter()), |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// One file path per song a filter matches, for a run that re-reads them.
    ///
    /// **One and not every copy.** The copies of a song are byte-identical by construction — the id
    /// *is* the hash — so reading a second would produce the same analysis for the same song row at
    /// the cost of parsing the file again. Which one is [`Self::best_file`]'s rule and the browse
    /// row's, so what gets re-read is the file the row is showing.
    ///
    /// Relative paths, which is what `files` holds and what a scan matches against.
    pub fn paths_matching(&self, filter: &Filter) -> Result<Vec<String>, DbError> {
        let (where_clause, values) = filter.to_sql();
        let sql = format!(
            "SELECT (SELECT f.path FROM files f
                      WHERE f.song_id = s.id AND f.scan_status = 'ok'
                      ORDER BY f.path LIMIT 1)
             FROM songs s WHERE {where_clause}"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let paths = statement
            .query_map(params_from_iter(values.iter()), |row| {
                row.get::<_, Option<String>>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        // A song whose every copy has gone from disk answers NULL, and there is nothing to re-read.
        Ok(paths.into_iter().flatten().collect())
    }

    /// The same, for songs named one by one.
    ///
    /// Chunked at [`ID_CHUNK`] for [`Self::add_tag_of`]'s reason: SQLite binds at most 999
    /// parameters and the header tick-box ticks a whole page.
    pub fn paths_of(&self, ids: &[String]) -> Result<Vec<String>, DbError> {
        let mut paths = Vec::with_capacity(ids.len());
        for batch in ids.chunks(ID_CHUNK) {
            let (holes, values) = id_holes(batch, 1);
            let sql = format!(
                "SELECT MIN(f.path) FROM files f
                  WHERE f.song_id IN ({holes}) AND f.scan_status = 'ok'
                  GROUP BY f.song_id"
            );
            let mut statement = self.conn.prepare(&sql)?;
            let found = statement
                .query_map(params_from_iter(values.iter()), |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            paths.extend(found);
        }
        Ok(paths)
    }

    /// Writes each song's own file name into its title and empties its artist, for the rows
    /// somebody ticked.
    ///
    /// **The artist goes with the title, because one pass of one sequencer wrote both.** A file
    /// whose title meta event says `UNTITLED` carries an artist from the same hand — the tool's
    /// name, a studio, a person who is not the performer — and a curator who has just replaced the
    /// title would otherwise walk the same rows a second time for it.
    ///
    /// **It writes `title`, rather than clearing it**, and the difference is the whole point. Clearing
    /// falls back to `det_title`, which on this corpus is the problem being fixed: a great many files
    /// carry a title meta event saying `UNTITLED`, `Karaoke` or the name of whoever sequenced it, and
    /// a row showing that is a row nobody can find again. The file's own name is the better answer,
    /// and choosing it is a person's decision — so it lands in the column that holds a person's
    /// decisions, survives the next scan, and lights the `ed` tag, all of which are true.
    ///
    /// **The artist is emptied and not cleared**, which is the same distinction one column over and
    /// resolves the opposite way. [`sql::eff_artist`](super::sql::eff_artist) has no `nullif`, so a
    /// NULL `artist` falls back to `det_artist` — the very value being escaped — while `''` reads as
    /// *explicitly nobody* and stands in front of it. The browse list sorts the two apart, empty
    /// first and absent last, so a batch done this way arrives together where somebody can look at
    /// it.
    ///
    /// `det_title` and `det_artist` are deliberately untouched: they have to stay answerable to
    /// *what did the file say?*, which the song page's edit form shows and which the whole `det_*`
    /// half of this table exists for.
    ///
    /// A song with no `stem` — nothing but a database written before that column existed and not yet
    /// reopened — is skipped rather than blanked, so the count returned is what actually changed,
    /// and so that a row keeps its artist where it did not get a title.
    pub fn set_names_from_stem(&mut self, song_ids: &[String]) -> Result<usize, DbError> {
        let transaction = self.conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut update = transaction.prepare(
                "UPDATE songs SET title = stem, artist = '' \
                 WHERE id = ?1 AND nullif(stem, '') IS NOT NULL",
            )?;
            for song_id in song_ids {
                changed += update.execute([song_id])?;
            }
        }
        Self::refold(&transaction)?;
        transaction.commit()?;
        Ok(changed)
    }

    /// Puts capitals back into the title and artist of the rows somebody ticked.
    ///
    /// The rule is [`crate::casing::recase`]'s and the whole of it is there. What is here is which
    /// text it is asked about and where the answer lands.
    ///
    /// **It asks about the name on the screen and writes into the column a person owns**, which is
    /// the same split `set_names_from_stem` respects one method up: `det_title` and `det_artist` stay
    /// answerable to *what did the file say?*, and a curator's capitals go in `title` and `artist`
    /// where they survive the next scan and light the `ed` tag.
    ///
    /// **Each field is judged by where its name came from.** A typed name gets
    /// [`crate::casing::recase`], which leaves mixed case alone; a name the file gave gets
    /// [`crate::casing::recase_from_file`], which does not, because a sequencer's case is no decision.
    ///
    /// **A row with no name of either kind is skipped rather than given one.** `eff_title` falls back
    /// to the stem for display, and following that fallback here would quietly make this the
    /// file-name button as well — two actions in one press, only one of them asked for. A row showing
    /// its stem has no title to recase, and the button beside this one is the one that gives it a
    /// title.
    ///
    /// An artist recorded as the empty string is *explicitly nobody* and has no capitals to fix, so
    /// the `nullif` here reads it the same way as an absent one and leaves it exactly as it is.
    ///
    /// The count returned is songs, not fields: a row whose title moved and whose artist did not is
    /// one song fixed, and the difference between it and the count ticked is what the page reports
    /// as names that already had capitals of their own.
    pub fn fix_name_case(&mut self, song_ids: &[String]) -> Result<usize, DbError> {
        let transaction = self.conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut read = transaction.prepare(
                "SELECT nullif(title, ''), nullif(det_title, ''), nullif(artist, ''), \
                        nullif(det_artist, '') \
                 FROM songs WHERE id = ?1",
            )?;
            let mut write =
                transaction.prepare("UPDATE songs SET title = ?2, artist = ?3 WHERE id = ?1")?;
            for song_id in song_ids {
                let found = read
                    .query_row([song_id], |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .optional()?;
                let Some((title, det_title, artist, det_artist)) = found else {
                    continue;
                };
                // The name in front of somebody, which is what they ticked the row about.
                let new_title = match &title {
                    Some(typed) => crate::casing::recase(typed),
                    None => det_title
                        .as_deref()
                        .and_then(crate::casing::recase_from_file),
                };
                let new_artist = match &artist {
                    Some(typed) => crate::casing::recase(typed),
                    None => det_artist
                        .as_deref()
                        .and_then(crate::casing::recase_from_file),
                };
                if new_title.is_none() && new_artist.is_none() {
                    continue;
                }
                // Each column keeps what it held wherever the rule had nothing to say, so fixing a
                // title cannot blank an artist and fixing an artist cannot take a title back to what
                // the file said.
                write.execute(params![song_id, new_title.or(title), new_artist.or(artist)])?;
                changed += 1;
            }
        }
        Self::refold(&transaction)?;
        transaction.commit()?;
        Ok(changed)
    }

    /// Takes the artist out of the title of each ticked song whose artist is blank.
    ///
    /// The rule is [`crate::names::artist_and_title`]'s and the whole of it is there. What is here is
    /// which text it is asked about, which rows it is asked about at all, and where the answer lands.
    ///
    /// **Only a row with nothing in its artist cell.** The fourth column read below is
    /// [`sql::eff_artist`](super::sql::eff_artist) with a `nullif` around it, so an artist recorded as
    /// the empty string reads the same as an absent one and both count as blank. That is the one place
    /// this method departs from the column's usual reading, and it is the point of the button: `''` is
    /// what *Title from file name* leaves behind, and those rows are exactly the ones whose artist is
    /// still sitting in the title. A row that names an artist has had this judgment made about it.
    ///
    /// **It asks about the name on the screen**, which is [`sql::eff_title`](super::sql::eff_title)'s
    /// three steps — what a person typed, else what the file said, else the file's own name. A row
    /// whose only name is its stem splits too. That is the opposite answer to `fix_name_case` one
    /// method up, and what makes the two differ is what the write puts in: recasing a stem row would
    /// be the file-name button's work done again, where splitting one fills the artist column that
    /// button never fills.
    ///
    /// **The write lands in `title` and `artist`**, the half of the table a person owns, so it
    /// survives the next scan and lights the `ed` tag. `det_title` and `det_artist` stay answerable to
    /// *what did the file say?*, which is what the whole `det_` half exists for.
    ///
    /// The count returned is songs written. The difference between it and the number ticked is rows
    /// that named an artist already or held no seam, which is what the page reports.
    pub fn split_artist_from_title(&mut self, song_ids: &[String]) -> Result<usize, DbError> {
        let transaction = self.conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut read = transaction.prepare(&format!(
                "SELECT {}, nullif({}, '') FROM songs WHERE id = ?1",
                eff_title(""),
                eff_artist("")
            ))?;
            let mut write =
                transaction.prepare("UPDATE songs SET title = ?2, artist = ?3 WHERE id = ?1")?;
            for song_id in song_ids {
                let found = read
                    .query_row([song_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                    })
                    .optional()?;
                let Some((title, artist)) = found else {
                    continue;
                };
                if artist.is_some() {
                    continue;
                }
                let Some((artist, title)) = crate::names::artist_and_title(&title) else {
                    continue;
                };
                write.execute(params![song_id, title, artist])?;
                changed += 1;
            }
        }
        Self::refold(&transaction)?;
        transaction.commit()?;
        Ok(changed)
    }

    /// The body behind `set_user_score`.
    ///
    /// `column` is a literal from the caller above and never anything a person typed, which is
    /// why it can be formatted into the statement. `None` writes NULL, which is UNSET — and clearing
    /// a score has to be possible, or a mis-click is permanent.
    fn set_score(&self, column: &str, id: &str, score: Option<u8>) -> Result<(), DbError> {
        if let Some(score) = score
            && score > 10
        {
            return Err(DbError::Rejected("a rating runs from 0 to 10".to_owned()));
        }
        let changed = self.conn.execute(
            &format!("UPDATE songs SET {column} = ?2 WHERE id = ?1"),
            params![id, score.map(i64::from)],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound(format!("song {id}")));
        }
        Ok(())
    }
}
