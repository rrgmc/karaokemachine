//! What a corpus scan writes, and what it forgets.
//!
//! The hot path of the whole tool: a whole corpus walked, parsed and written in batches, and the
//! reason [`Known`] exists is so a re-scan touches only what changed — the size and time of every
//! path, and the revision of the analysis that decided its song, read once at the start.
//!
//! **[`Db::write_scanned`] binds by name, not by ordinal.** Its upsert is 44 columns, and when they
//! were numbered `?1`..`?40` two of them shared a value, so every ordinal after it was offset by one
//! from its column position. An off-by-one there wrote a detected encoding into a melody confidence —
//! both nullable, no error — across a whole corpus. `tests.rs` holds the mapping as a test as well.
//!
//! Forgetting is the other half and is deliberately two steps: a file that has gone takes its row
//! with it, and a song left with no files at all is swept separately, because a song whose only copy
//! moved between two scans must not lose the title somebody typed.

use super::*;

/// What the database already knows about one file, and everything a re-scan needs to skip it.
///
/// **Three facts and not two**, because there are two ways for a row to be out of date and only one
/// of them is about the file. The bytes may have changed, which size and time answer; or this build
/// may decide something different about the same bytes, which [`Self::analysis_revision`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    /// Size in bytes as it was last read.
    pub size: u64,
    /// Modification time as it was last read.
    pub mtime: i64,
    /// Which revision of the analysis decided this file's song, or read the file where it has no
    /// song; `None` where nothing recorded it.
    pub analysis_revision: Option<u32>,
}

impl Db {
    /// What is known about every file, so a re-scan can skip what nothing has changed about.
    pub fn known_files(&self) -> Result<BTreeMap<String, Known>, DbError> {
        // **The join is what lets a scan notice that the analysis moved rather than the file.** Size
        // and time answer "are these the same bytes"; the revision answers "would this build write
        // the same row about them", and a scan has to ask both before it may skip. `files_song` and
        // the `songs` primary key make it a keyed lookup per row rather than a second scan.
        //
        // LEFT, because a file that could not be read has no song to join to and must not vanish
        // from the map: it is still a path this corpus knows about.
        //
        // **A file with no song answers from its own row.** A failure, a readme `.txt` and the
        // `.cdg` half of a pair have no song revision to ask, so taking the song's alone made every
        // one of them fail the skip test on every scan, and a changed-file scan re-read them all.
        let mut statement = self.conn.prepare(
            "SELECT f.path, f.size, f.mtime,
                    CASE WHEN f.song_id IS NULL THEN f.analysis_revision
                         ELSE s.analysis_revision END
               FROM files f LEFT JOIN songs s ON s.id = f.song_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Known {
                    size: row.get::<_, i64>(1)? as u64,
                    mtime: row.get::<_, i64>(2)?,
                    analysis_revision: row.get::<_, Option<u32>>(3)?,
                },
            ))
        })?;
        Ok(rows.collect::<Result<BTreeMap<_, _>, _>>()?)
    }

    /// Writes a batch of scan results in one transaction.
    pub fn write_scanned(&mut self, batch: &[ScannedFile], now: &str) -> Result<(), DbError> {
        let transaction = self.conn.transaction()?;
        {
            let mut upsert_song = transaction.prepare(
                "INSERT INTO songs(
                     id, det_title, det_artist, det_language, flavor, granularity, duration_ms,
                     note_count, channel_count, line_count, syllable_count, det_encoding,
                     det_encoding_source, melody_channel, melody_confidence, melody_abstained,
                     suitability, suitability_lyrics, suitability_sync, suitability_channels,
                     suitability_arrangement, warnings, analysis_revision,
                     fingerprint, first_seen, last_scanned, stem, lyrics,
                     kind, width, height, frame_rate_milli, video_codec, audio_codec,
                     det_language_tag, det_language_guess, det_language_guess_confidence,
                     cdg_graphics_path, cdg_sample_rate, cdg_channels, cdg_packets,
                     cdg_graphics_ms, cdg_short_by_ms, cdg_tiles, cdg_unknown)
                 -- **Named, and that is not a style choice.** Forty-one columns were bound by
                 -- ordinal, and `first_seen` and `last_scanned` share one value -- so the run read
                 -- `?21, ?22, ?23, ?23, ?24`, and from that point every ordinal sat one place to the
                 -- left of its column. Adding a column meant editing four things in lockstep: this
                 -- list, the run (renumbering everything after the insertion point), the `DO UPDATE
                 -- SET` below, and the `params!` array. An off-by-one wrote `det_encoding` into
                 -- `melody_confidence` -- both nullable, no error, no complaint from SQLite -- and
                 -- did it across a whole corpus. A name cannot be off by one, `:now` says outright
                 -- what `?23, ?23` only implied, and a typo is a runtime error that names the
                 -- parameter it could not find.
                 VALUES (:id, :det_title, :det_artist, :det_language, :flavor, :granularity,
                         :duration_ms, :note_count, :channel_count, :line_count, :syllable_count,
                         :det_encoding, :det_encoding_source, :melody_channel, :melody_confidence,
                         :melody_abstained, :suitability, :suitability_lyrics, :suitability_sync,
                         :suitability_channels, :suitability_arrangement, :warnings, :analysis_revision,
                         :fingerprint, :now, :now, :stem, :lyrics,
                         :kind, :width, :height, :frame_rate_milli, :video_codec, :audio_codec,
                         :det_language_tag, :det_language_guess, :det_language_guess_confidence,
                         :cdg_graphics_path, :cdg_sample_rate, :cdg_channels, :cdg_packets,
                         :cdg_graphics_ms, :cdg_short_by_ms, :cdg_tiles, :cdg_unknown)
                 ON CONFLICT(id) DO UPDATE SET
                     det_title = excluded.det_title,
                     det_artist = excluded.det_artist,
                     det_language = excluded.det_language,
                     det_language_tag = excluded.det_language_tag,
                     det_language_guess = excluded.det_language_guess,
                     det_language_guess_confidence = excluded.det_language_guess_confidence,
                     flavor = excluded.flavor,
                     granularity = excluded.granularity,
                     duration_ms = excluded.duration_ms,
                     note_count = excluded.note_count,
                     channel_count = excluded.channel_count,
                     line_count = excluded.line_count,
                     syllable_count = excluded.syllable_count,
                     det_encoding = excluded.det_encoding,
                     det_encoding_source = excluded.det_encoding_source,
                     melody_channel = excluded.melody_channel,
                     melody_confidence = excluded.melody_confidence,
                     melody_abstained = excluded.melody_abstained,
                     suitability = excluded.suitability,
                     suitability_lyrics = excluded.suitability_lyrics,
                     suitability_sync = excluded.suitability_sync,
                     suitability_channels = excluded.suitability_channels,
                     suitability_arrangement = excluded.suitability_arrangement,
                     warnings = excluded.warnings,
                     analysis_revision = excluded.analysis_revision,
                     last_scanned = excluded.last_scanned,
                     fingerprint = excluded.fingerprint,
                     lyrics = excluded.lyrics,
                     kind = excluded.kind,
                     width = excluded.width,
                     height = excluded.height,
                     frame_rate_milli = excluded.frame_rate_milli,
                     video_codec = excluded.video_codec,
                     audio_codec = excluded.audio_codec,
                     cdg_graphics_path = excluded.cdg_graphics_path,
                     cdg_sample_rate = excluded.cdg_sample_rate,
                     cdg_channels = excluded.cdg_channels,
                     cdg_packets = excluded.cdg_packets,
                     cdg_graphics_ms = excluded.cdg_graphics_ms,
                     cdg_short_by_ms = excluded.cdg_short_by_ms,
                     cdg_tiles = excluded.cdg_tiles,
                     cdg_unknown = excluded.cdg_unknown,
                     -- A song can have several byte-identical copies under different names and there
                     -- is no right answer among them, so the smallest wins: an arbitrary choice, but
                     -- the same arbitrary choice every scan, which is what stops the title of a
                     -- nameless song changing depending on which thread got there first.
                     stem = CASE
                         WHEN songs.stem IS NULL OR excluded.stem < songs.stem
                         THEN excluded.stem ELSE songs.stem END",
            )?;
            let mut upsert_file = transaction.prepare(
                "INSERT INTO files(path, size, mtime, content_hash, song_id, scan_status,
                                   scan_error, scanned_at, analysis_revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(path) DO UPDATE SET
                     size = excluded.size, mtime = excluded.mtime,
                     content_hash = excluded.content_hash, song_id = excluded.song_id,
                     scan_status = excluded.scan_status, scan_error = excluded.scan_error,
                     scanned_at = excluded.scanned_at,
                     analysis_revision = excluded.analysis_revision",
            )?;

            for file in batch {
                if let Some(song) = &file.song {
                    // A video fills none of the MIDI columns and a MIDI file fills none of the video
                    // ones, so each block contributes NULLs when it is absent. **The suitability is
                    // outside that shape and is bound unconditionally**, because every kind of song
                    // has one: a browse list, the band filter and the sort all read that column, and
                    // a number invented on the way out would agree with the page and not with the
                    // `WHERE` clause beside it.
                    let midi = song.midi.as_ref();
                    let video = song.video.as_ref();
                    let cdg = song.cdg.as_ref();
                    let ultrastar = song.ultrastar.as_ref();
                    // Read once and bound twice, so the code and the confidence beside it cannot
                    // come from two different readings of the same song.
                    let guessed = km_langguess::guess(
                        song.lyrics.as_deref(),
                        song.det_title.as_deref().or(Some(song.stem.as_str())),
                    );
                    upsert_song.execute(named_params! {
                        ":id": song.id,
                        ":det_title": song.det_title,
                        ":det_artist": song.det_artist,
                        ":det_language": song.det_language,
                        ":flavor": midi.map(|m| m.flavor.clone()),
                        ":granularity": midi.map(|m| m.granularity.clone()),
                        ":duration_ms": song.duration_ms,
                        ":note_count": midi.map(|m| m.note_count),
                        ":channel_count": midi.map(|m| m.channel_count),
                        ":line_count": midi
                            .map(|m| m.line_count)
                            .or(ultrastar.map(|u| u.line_count)),
                        ":syllable_count": midi
                            .map(|m| m.syllable_count)
                            .or(ultrastar.map(|u| u.syllable_count)),
                        ":det_encoding": midi
                            .map(|m| m.det_encoding.clone())
                            .or(ultrastar.map(|u| u.det_encoding.clone())),
                        ":det_encoding_source": midi
                            .map(|m| m.det_encoding_source.clone())
                            .or(ultrastar.map(|u| u.det_encoding_source.clone())),
                        ":melody_channel": midi.and_then(|m| m.melody_channel).map(i64::from),
                        ":melody_confidence": midi.and_then(|m| m.melody_confidence),
                        ":melody_abstained": midi.and_then(|m| m.melody_abstained.clone()),
                        ":suitability": song.suitability.value,
                        ":suitability_lyrics": song.suitability.breakdown.0,
                        ":suitability_sync": song.suitability.breakdown.1,
                        ":suitability_channels": song.suitability.breakdown.2,
                        ":suitability_arrangement": song.suitability.breakdown.3,
                        // Never NULL: the column is NOT NULL with a `[]` default, and a song with
                        // nothing wrong with it legitimately has no warnings rather than unknown
                        // ones.
                        ":warnings": song.suitability.warnings,
                        // **The one value here that does not come from the file.** It says which
                        // build decided the rest, so it is the same for every row in a run and is
                        // bound from the constant rather than carried on `ScannedSong` — which is
                        // built at three separate sites, each of which would be somewhere to forget
                        // it.
                        ":analysis_revision": km_suitability::ANALYSIS_REVISION,
                        // Bound once and named twice in the statement, where the ordinal version
                        // had to spell `?23` twice and offset everything after it.
                        ":now": now,
                        ":stem": song.stem,
                        ":fingerprint": song.fingerprint,
                        ":lyrics": song.lyrics,
                        ":kind": song.kind().as_str(),
                        ":width": video.map(|v| v.width),
                        ":height": video.map(|v| v.height),
                        ":frame_rate_milli": video.map(|v| v.frame_rate_milli),
                        ":video_codec": video.map(|v| v.video_codec.clone()),
                        ":audio_codec": video.map(|v| v.audio_codec.clone()),
                        // Derived here rather than carried on `ScannedSong`, which holds only what
                        // the file said: two fields that have to agree is a shape that lets them
                        // disagree, and the two inputs are already in hand three lines apart. A
                        // video reaches this with both `None` and gets `None`, which is right --
                        // no container states what language the singing is in.
                        ":det_language_tag": Language::detect(
                            song.det_language.as_deref(),
                            midi.map(|m| m.det_encoding.as_str()),
                        )
                        .map(Language::code),
                        // The weakest witness, and the only one that reads the song rather than
                        // what the song says about itself. A video and an MP3+G reach this with no
                        // lyrics and only a file name to go on, which is usually too little to
                        // place and is then left NULL.
                        ":det_language_guess": guessed.map(|g| g.language().code()),
                        ":det_language_guess_confidence": guessed.map(km_langguess::Guess::confidence),
                        ":cdg_graphics_path": cdg.map(|c| c.graphics_path.clone()),
                        ":cdg_sample_rate": cdg.map(|c| c.sample_rate),
                        ":cdg_channels": cdg.map(|c| c.channels),
                        ":cdg_packets": cdg.map(|c| c.packets),
                        ":cdg_graphics_ms": cdg.map(|c| c.graphics_ms),
                        ":cdg_short_by_ms": cdg.map(|c| c.graphics_short_by_ms),
                        ":cdg_tiles": cdg.map(|c| c.tiles_written),
                        ":cdg_unknown": cdg.map(|c| c.unknown_instructions),
                    })?;
                }
                upsert_file.execute(params![
                    file.path,
                    file.size as i64,
                    file.mtime,
                    file.content_hash,
                    file.song.as_ref().map(|song| song.id.clone()),
                    file.status.as_str(),
                    file.error,
                    now,
                    km_suitability::ANALYSIS_REVISION,
                ])?;
            }
        }
        // The one write path where the fold genuinely could not be done in the statement above: the
        // effective title falls back through `songs.title`, a hand-set column the scan never writes
        // and does not know, so no bindable expression can produce the post-write value. Reading it
        // back is the only answer, and at 500 rows a batch it is 1,000 primary-key seeks.
        Self::refold(&transaction)?;
        transaction.commit()?;
        Ok(())
    }

    /// Removes files that are no longer on disk, and any song left with no copies at all.
    ///
    /// A song a package still names is kept even with no files, and shows as *source missing*.
    /// Deleting a curated selection because a drive was unmounted is the one failure this must not
    /// have.
    ///
    /// **It is handed what is gone, not what is present, and that is the whole performance story.**
    /// Taking `seen` — every path found on disk — means inserting all of it into a temp table one
    /// row at a time and then deleting with two `NOT IN` anti-joins. On the real corpus that is
    /// One single-row insert per file followed by a full scan of `files` *and* a full scan of `songs`,
    /// paid in full on every completed scan **whether or not a single file had gone**, which is the
    /// overwhelmingly common case: minutes of the tool being frozen to delete nothing.
    ///
    /// The set difference costs nothing where it is computed instead: `Db::known_files` already loads
    /// every path in this table into memory at the start of a scan, and the walk already has every
    /// path on disk, so `known - seen` is a hash lookup per file in the caller and this is left with
    /// only the rows that actually go. Empty is the fast path and does no SQL at all.
    ///
    /// Called in chunks so the caller can release the database mutex between them — see the note on
    /// the scan's writer batches, which exists for the same reason.
    pub fn forget_missing(&mut self, gone: &[String]) -> Result<(usize, usize), DbError> {
        if gone.is_empty() {
            return Ok((0, 0));
        }
        let transaction = self.conn.transaction()?;
        let mut files = 0usize;
        // Collected before the delete, because afterwards there is nothing left to ask. A song can
        // only lose its last file when one of its files is deleted, so this is every song the sweep
        // below could possibly find orphaned — which is what lets that sweep be a handful of seeks
        // instead of a scan of the whole `songs` table.
        let mut touched: BTreeSet<String> = BTreeSet::new();
        {
            let mut owner = transaction
                .prepare("SELECT song_id FROM files WHERE path = ?1 AND song_id IS NOT NULL")?;
            // An exact-path delete seeks `sqlite_autoindex_files_1`, where `path NOT IN (…)` could
            // not use it at all.
            let mut delete = transaction.prepare("DELETE FROM files WHERE path = ?1")?;
            for path in gone {
                if let Some(id) = owner
                    .query_row([path], |row| row.get::<_, String>(0))
                    .optional()?
                {
                    touched.insert(id);
                }
                files += delete.execute([path])?;
            }
        }
        let mut songs = 0usize;
        {
            // Term for term the predicate the whole-table statement used, restricted to the songs
            // that could have changed. Both `NOT EXISTS` clauses are index seeks — `files_song` and
            // `package_songs_song`.
            let mut delete = transaction.prepare(
                "DELETE FROM songs
                 WHERE id = ?1
                   AND NOT EXISTS (SELECT 1 FROM files WHERE song_id = ?1)
                   AND NOT EXISTS (SELECT 1 FROM package_songs WHERE song_id = ?1)",
            )?;
            for id in &touched {
                songs += delete.execute([id])?;
            }
        }
        transaction.commit()?;
        Ok((files, songs))
    }

    /// Removes songs that were left with no files by something other than this scan.
    ///
    /// [`Db::forget_missing`] only looks at songs whose files it just deleted, which is exact for
    /// every orphan *it* creates and blind to one that was already there — a row left by an older
    /// version, or by a write path that has since been fixed. The whole-table sweep it replaced
    /// caught those as a side effect, so this keeps that property rather than dropping it quietly.
    ///
    /// It stays cheap because it asks the denormalized count rather than `files`: `file_count = 0`
    /// is a seek on `songs_file_count`, and the column is trigger-maintained, so it cannot disagree
    /// with the table it summarizes. Run once at the end of a scan, not per chunk.
    pub fn forget_orphaned_songs(&self) -> Result<usize, DbError> {
        Ok(self.conn.execute(
            "DELETE FROM songs
             WHERE file_count = 0
               AND id NOT IN (SELECT song_id FROM package_songs)",
            [],
        )?)
    }

    /// How the last scan went, for the status page — the failures nobody has accepted yet.
    pub fn failure_tally(&self) -> Result<Vec<FailureTally>, DbError> {
        self.tally(false)
    }

    /// The same, over the failures somebody has accepted.
    ///
    /// Listed rather than merely counted, because the line offering them back has to say *what*
    /// was accepted: "3 reasons removed" with no reasons is a number nobody can act on.
    pub fn dismissed_tally(&self) -> Result<Vec<FailureTally>, DbError> {
        self.tally(true)
    }

    /// How many songs this build would answer differently from whatever last answered.
    ///
    /// **Without a count, a stale corpus and a current one look exactly alike.** Nothing about the
    /// files has moved, so the Scan page says the folder was scanned and the browse list shows a
    /// number beside every song; that the numbers were worked out by a build with a different idea
    /// is knowable only from this column. A count is what turns "a scan would re-read things" from
    /// a fact about the code into one somebody can see.
    ///
    /// Merged songs are left out, the way every count here leaves them out: a merged row is the same
    /// recording as the one it points at and is shown nowhere.
    pub fn stale_analysis_count(&self) -> Result<i64, DbError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM songs
              WHERE merged_into IS NULL
                AND (analysis_revision IS NULL OR analysis_revision <> ?1)",
            [km_suitability::ANALYSIS_REVISION],
            |row| row.get(0),
        )?)
    }

    /// Moves each song forward through the revisions that cannot change it, without reading its file.
    ///
    /// **A revision that changed one kind of file should cost a scan of that kind of file.** Every
    /// row written by an older build fails the skip test in a scan, so a bump alone makes the next
    /// scan read the whole corpus. [`km_suitability::REVISIONS`] says which rows each revision can
    /// reach; a row outside that reach would be written again unchanged, so its stored revision is
    /// raised here instead.
    ///
    /// In order, one revision at a time, so a row climbs until the first revision that reaches it
    /// and stops there. A row with no revision is left alone: nothing says which build wrote it.
    /// Idempotent, and a statement that matches nothing on every open after the first.
    pub fn promote_unreached_revisions(&self) -> Result<(), DbError> {
        for revision in km_suitability::REVISIONS {
            // The column is spliced in from a closed match, never from input, so the text is fixed.
            let bound = match revision.reach {
                km_suitability::Reach::Everything => continue,
                // Every song is re-read, and the file below it that holds no song is not.
                km_suitability::Reach::EverySong => None,
                km_suitability::Reach::LyricLinesAtLeast(least) => Some(("line_count", least)),
                km_suitability::Reach::SyllablesAtLeast(least) => Some(("syllable_count", least)),
            };
            if let Some((column, least)) = bound {
                self.conn.execute(
                    &format!(
                        "UPDATE songs SET analysis_revision = ?1
                          WHERE analysis_revision = ?2
                            AND ({column} IS NULL OR {column} < ?3)"
                    ),
                    rusqlite::params![revision.number, revision.number - 1, least],
                )?;
            }
            // A file with no song has no count to reach, which is the NULL count above: a revision
            // with a limited reach cannot turn a failure into a song, or it would reach everything.
            self.conn.execute(
                "UPDATE files SET analysis_revision = ?1
                  WHERE analysis_revision = ?2 AND song_id IS NULL",
                rusqlite::params![revision.number, revision.number - 1],
            )?;
        }
        Ok(())
    }

    /// One side or the other of the dismissal line.
    ///
    /// **A dismissal matches on the status as well as the file**, so a file that starts failing a
    /// different way counts again rather than staying quiet under a verdict passed on something
    /// else. The example is drawn from the same side as the count, or a removed row would offer a
    /// path that is not among the files it is counting.
    fn tally(&self, dismissed: bool) -> Result<Vec<FailureTally>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT f.scan_status, COUNT(*), MIN(f.path)
             FROM files f
             WHERE f.scan_status <> 'ok'
               AND ?1 = EXISTS (
                     SELECT 1 FROM dismissed_failures d
                     WHERE d.file_id = f.id AND d.scan_status = f.scan_status)
             GROUP BY f.scan_status ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement.query_map([dismissed], |row| {
            let status: String = row.get(0)?;
            Ok(FailureTally {
                reason: ScanStatus::from_str(&status).key().to_owned(),
                status,
                count: row.get::<_, i64>(1)? as u32,
                example: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Accepts every file failing this way now, and answers how many.
    ///
    /// **Now, and not the reason itself.** A ninth orphan `.cdg` appearing after somebody has been
    /// through eight is the news this list exists to carry, so it is the files that are accepted
    /// and a later one counts on its own.
    pub fn dismiss_failures(&self, status: &str, now: &str) -> Result<usize, DbError> {
        Ok(self.conn.execute(
            "INSERT INTO dismissed_failures(file_id, scan_status, dismissed_at)
             SELECT id, scan_status, ?2 FROM files WHERE scan_status = ?1
             ON CONFLICT(file_id) DO UPDATE SET scan_status = excluded.scan_status,
                                                dismissed_at = excluded.dismissed_at",
            params![status, now],
        )?)
    }

    /// Puts every file accepted under this reason back on the list, and answers how many.
    pub fn restore_failures(&self, status: &str) -> Result<usize, DbError> {
        Ok(self.conn.execute(
            "DELETE FROM dismissed_failures WHERE scan_status = ?1",
            params![status],
        )?)
    }
}
