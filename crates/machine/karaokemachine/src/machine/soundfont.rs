//! What the machine does about SoundFonts: choosing one, fetching one, switching between them, and
//! carrying out a setup program's first-run request.
//!
//! **A second `impl Machine` in a file of its own, rather than a `SoundFonts` type**, and the
//! distinction is worth stating because the type was tried first. These methods reach `paths`,
//! `lock_settings`, `save_settings`, `engine` and `lock_state` as well as the three fields that are
//! theirs alone (`soundfont_slot`, `first_run`, `downloader`) — so a standalone type would have
//! taken five collaborators threaded through two dozen methods, which is a worse interface than the
//! one it replaced. The cluster is cohesive as *code* and not as *data*, and a file is what
//! expresses that.
//!
//! What it buys is the thing that was actually wrong: `machine.rs` was 6,048 lines with 199 methods
//! on one type, and this was the largest seam in it that could be moved without inventing an
//! interface.
//!
//! A child module sees its parent's private items, so nothing here needed widening on the way in.
//! The methods `machine.rs` calls back into are `pub(super)`, and that list is the whole of the
//! coupling in the other direction.
//!
//! Not to be confused with [`crate::soundfont`], which is the stateless half: what a bank file *is*,
//! where they are found, and how one is validated. This module is what a running machine does with
//! them.

use super::*;

impl Machine {
    /// Switches the SoundFont to a switcher slot, keeping the song where it was.
    ///
    /// **`Ctrl+1`…`Ctrl+9`, and nothing else reaches this.** Off unless `debug.soundfonts` names at
    /// least one bank.
    ///
    /// **The slot is written down; `audio.soundfont` still is not.** Which of those two is
    /// remembered is the whole of the rule: an evening of comparing banks resumes where it stopped,
    /// and emptying `debug.soundfonts` still returns the machine to the bank it resolves for itself
    /// — which is what makes leaving the slots configured safe. Slot 1 clears the entry rather than
    /// writing `1`, so a machine that ends on its own bank leaves nothing behind.
    ///
    /// Deliberately *not* gated on [`Machine::output_change_allowed`], which refuses unless the
    /// machine is idle with an empty queue. That gate exists because the player lives inside the
    /// stream a device change drops, and it is the exact opposite of what this is for: the only
    /// interesting moment to change bank is mid-song. So this one restores the song itself.
    ///
    /// Three shapes, decided by what is loaded:
    ///
    /// * **nothing** — assign the bank; the next song opens on it. No gap, because no stream.
    /// * **a MIDI song** — assign, drop the stream, re-send the retained `Arc<Song>`, seek back to
    ///   where it was, and play on if it was playing. There is an audible hole while the device
    ///   closes and reopens; that is the price of a synthesizer that takes its bank in `new`.
    /// * **a video or MP3+G song** — assign only, and say the bank is pending. Those songs play
    ///   through no synthesizer at all, so there is nothing to hear differently — and their audio
    ///   is a `TrackPlayer` that was *moved* into the audio thread and cannot be built again, so
    ///   dropping the stream would end the song for good.
    pub fn switch_debug_soundfont(&self, slot: u8) -> Result<(), ControlError> {
        if !self.lock_settings().soundfont_switcher_on() {
            // Not an error the singer should see. The keys are live in every build, so this is the
            // ordinary case of somebody pressing one on a machine that was never configured for it.
            tracing::debug!(
                slot,
                "the SoundFont switcher is off; debug.soundfonts is empty"
            );
            return Ok(());
        }

        let (path, name, music_volume) = match slot {
            1 => {
                // Resolved rather than remembered, so slot 1 keeps meaning "what this machine plays
                // by default" even if `audio.soundfont` is set.
                let configured = self.lock_settings().audio.soundfont.clone();
                let selected = crate::soundfont::resolve(&self.paths, configured.as_deref());
                let path = crate::engine::resolve_soundfont(selected.path.as_deref(), &self.paths)
                    .map_err(ControlError::Failed)?;
                (path, BUNDLED_BANK_NAME.to_owned(), None)
            }
            _ => {
                let settings = self.lock_settings();
                let Some(bank) = settings.soundfont_slot(slot) else {
                    return Err(ControlError::Rejected(format!(
                        "nothing is configured in SoundFont slot {slot}"
                    )));
                };
                (bank.path.clone(), bank.name.clone(), bank.music_volume)
            }
        };

        let close = self.switch_soundfont(&path, music_volume)?;

        if let Ok(mut current) = self.soundfont_slot.lock() {
            *current = SoundFontSlot {
                slot,
                name: name.clone(),
                pending: !close,
            };
        }

        // Written after the switch rather than before it, so a slot that would not open leaves no
        // record for the next start to fail on again. `None` for slot 1: a machine back on its own
        // bank has nothing to remember.
        self.lock_settings().debug.soundfont_slot = (slot > 1).then_some(slot);
        self.save_settings();

        tracing::info!(
            slot,
            bank = %name,
            path = %path.display(),
            pending = !close,
            "switched the SoundFont"
        );
        Ok(())
    }

    /// Every bank this machine could switch to now, and which one is playing.
    ///
    /// Read-only and unfailing, like [`Machine::soundfont`] beside it: a folder that cannot be read
    /// yields the bundled bank alone, which is the truth about what is playing rather than an error
    /// about a directory.
    pub fn soundfont_banks(&self) -> (Vec<crate::soundfont::Installed>, String) {
        let configured = self.lock_settings().audio.soundfont.clone();
        let banks = crate::soundfont::installed(&self.paths);
        // Which row is selected is decided by the *setting*, not by what the engine happens to hold:
        // a debug slot may have swapped the bank for this run, and the picker must go on showing
        // what the machine will come back on. `Ctrl+1`…`Ctrl+9` are explicitly run-only.
        //
        // **And it is now a name against a name.** While the setting held a path this had to
        // compare two spellings of one file, and getting that wrong reported the *bundled* bank as
        // selected while the machine played another. An id either matches a row or does not, and
        // when it does not the machine really is on the bundled bank — so the picker agrees with the
        // sound rather than contradicting it.
        let selected = configured
            .filter(|id| banks.iter().any(|bank| bank.id == *id))
            .unwrap_or_else(|| crate::soundfont::BUNDLED_ID.to_owned());
        (banks, selected)
    }

    /// Banks the table knows that are not on this machine, and what the downloader is doing.
    ///
    /// **Matched by filename**, which is the join between a row in the table and a file in the
    /// folder — the same rule [`measured_level`] uses, and for the same reason.
    ///
    /// `all` is the only thing that changes the width: it keeps the unranked rows, which are the
    /// rest of the survey. Everything else about the list is the same at either width, so a caller
    /// cannot get a row here that it could not fetch.
    pub(super) fn soundfont_offers(
        &self,
        all: bool,
    ) -> (
        Vec<km_api::machine::SoundFontOffer>,
        Option<km_api::machine::SoundFontFetch>,
    ) {
        let present: Vec<String> = {
            crate::soundfont::installed(&self.paths)
                .into_iter()
                .filter_map(|bank| {
                    bank.path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned)
                })
                .collect()
        };

        let offers = crate::banks::catalog()
            .iter()
            // **Only the ranked rows, unless the caller asked for the rest.** The table is the whole
            // survey — every bank measured that has a source worth pinning to,
            // which is dozens — and a phone is not where somebody browses a catalog. `rank` marks
            // the shortlist; the rest are reachable from a shell, through `task soundfont
            // BANK=<id>` and the switcher, and from the `/dev/` page, which asks for `all`.
            .filter(|row| all || row.rank.is_some())
            // The bundled bank is never an offer: it is already here by definition, and a row
            // inviting somebody to download the bank they are listening to would be nonsense.
            .filter(|row| !row.bundled)
            .filter(|row| !present.iter().any(|name| name == row.name))
            .map(|row| km_api::machine::SoundFontOffer {
                id: row.id.to_owned(),
                name: row.name.to_owned(),
                size: row.size.to_owned(),
                bytes: row.bytes,
                license: row.license.to_owned(),
                note: row.note.to_owned(),
                fetchable: row.url.is_some() && row.digest.is_some(),
                page: row.page.map(str::to_owned),
                recommended: row.recommended,
                offered: row.rank.is_some(),
            })
            .collect();

        let fetching = match self.downloader.status() {
            crate::fetch::Fetching::Idle => None,
            crate::fetch::Fetching::Working {
                id,
                name,
                done,
                total,
            } => Some(km_api::machine::SoundFontFetch {
                id,
                name,
                state: km_api::machine::SoundFontFetchState::Working,
                done,
                total,
                problem: None,
            }),
            crate::fetch::Fetching::Done { id, name } => Some(km_api::machine::SoundFontFetch {
                id,
                name,
                state: km_api::machine::SoundFontFetchState::Done,
                done: 0,
                total: 0,
                problem: None,
            }),
            crate::fetch::Fetching::Failed { id, name, why } => {
                Some(km_api::machine::SoundFontFetch {
                    id,
                    name,
                    state: km_api::machine::SoundFontFetchState::Failed,
                    done: 0,
                    total: 0,
                    problem: Some(why),
                })
            }
        };
        (offers, fetching)
    }

    /// Starts fetching a bank the table knows about.
    pub fn fetch_soundfont(&self, id: &str) -> Result<(), ControlError> {
        let Some(row) = crate::banks::catalog().iter().find(|row| row.id == id) else {
            return Err(ControlError::Rejected(format!("no SoundFont called {id}")));
        };
        self.downloader.start(row).map_err(ControlError::Rejected)
    }

    /// Carries out the tick box a setup program was given, at the first start after an install.
    ///
    /// **This is the one place a download begins without a request arriving over the API**, and the
    /// request is no less explicit for having been made in a different program: somebody ticked a
    /// box naming one bank, and this is how the answer reaches a machine that was not running at the
    /// time. See [`crate::firstrun`] for what holds it inside `Nothing downloads`.
    ///
    /// Four ways out before anything is fetched, in this order and for these reasons:
    ///
    /// * **no request** — the common case, and one `read` of a file that is not there;
    /// * **a bank the table does not know**, which nothing later will make resolve;
    /// * **an owner who has already chosen a bank**, which is a more recent statement than the tick
    ///   box: a repair or upgrade install must not talk over it;
    /// * **the bank is already here**, so choosing it is the whole of what is left to do.
    ///
    /// Failure is never fatal and never propagated. A machine that could not fetch a bank is a
    /// machine playing the bundled one, which is exactly what it would have been doing anyway.
    pub fn start_first_run_soundfont(&self) {
        let Some(request) = crate::firstrun::read(&self.paths) else {
            // Nothing to do in the ordinary case, and in the other one this is the only path
            // entitled to throw away bytes nobody can act on — `--show-paths` reads and never
            // writes.
            crate::firstrun::discard_unreadable(&self.paths);
            return;
        };
        let Some(row) = request.resolve() else {
            tracing::warn!(
                bank = %request.bank,
                "a setup program asked for a SoundFont this build's table does not know"
            );
            self.forget_first_run_request();
            return;
        };

        if self.lock_settings().audio.soundfont.is_some() {
            tracing::info!(
                bank = %row.id,
                "a SoundFont is already chosen, so the first-start request is dropped"
            );
            self.forget_first_run_request();
            return;
        }

        if let Some(bank) = self.installed_bank_named(row.name) {
            match self.select_soundfont(&bank.id) {
                Ok(()) => self.notify_first_run(crate::firstrun::Notice::Done(format!(
                    "{} is now the instrument bank",
                    bank.name
                ))),
                Err(error) => {
                    tracing::warn!(bank = %bank.id, %error, "could not choose the requested bank");
                    self.notify_first_run(crate::firstrun::Notice::Failed(error.to_string()));
                }
            }
            self.forget_first_run_request();
            return;
        }

        let Some(attempted) = request.attempted() else {
            tracing::warn!(
                bank = %row.id,
                "the first-start SoundFont request has used its attempts; giving up"
            );
            self.forget_first_run_request();
            self.notify_first_run(crate::firstrun::Notice::Failed(format!(
                "{} could not be downloaded. It can still be chosen from the SoundFont page.",
                row.name
            )));
            return;
        };
        // Counted *before* the download starts, so a start that dies part-way through one still
        // spends an attempt. The alternative counts only tidy failures, which is the wrong way round
        // — a machine that crashes while fetching is the one most in need of a limit.
        if let Err(error) = crate::firstrun::write(&self.paths, &attempted) {
            tracing::warn!(%error, "could not record the first-start SoundFont attempt");
        }

        match self.fetch_soundfont(row.id) {
            Ok(()) => {
                if let Ok(mut first_run) = self.first_run.lock() {
                    first_run.fetching = Some(row.id.to_owned());
                    first_run.announced = None;
                }
                tracing::info!(
                    bank = %row.id,
                    size = %row.size,
                    attempt = attempted.attempts,
                    "fetching the SoundFont a setup program was asked for"
                );
                self.notify_first_run(crate::firstrun::Notice::Working(format!(
                    "downloading {} — {}",
                    row.name, row.size
                )));
            }
            Err(error) => {
                tracing::warn!(bank = %row.id, %error, "could not start the first-start download");
                self.notify_first_run(crate::firstrun::Notice::Failed(error.to_string()));
            }
        }
    }

    /// Follows a first-start download, and chooses the bank the moment it lands.
    ///
    /// Called from the poll loop beside [`Machine::settle_pending_soundfont`], and free on every
    /// ordinary run for the same reason: one uncontended lock that returns at once unless this start
    /// began a download of its own.
    ///
    /// **The id is checked against the status**, because the downloader is shared. Somebody opening
    /// the SoundFont page and fetching a second bank while this one runs must not have their
    /// download chosen, or its failure reported, as though it were this one.
    pub(super) fn settle_first_run_soundfont(&self) {
        let Ok(first_run) = self.first_run.lock() else {
            return;
        };
        let Some(wanted) = first_run.fetching.clone() else {
            return;
        };
        let announced = first_run.announced;
        // Dropped before anything below, because choosing a bank locks the settings and rebuilds the
        // audio stream, and holding this while that happens would make the display wait on a
        // synthesizer.
        drop(first_run);

        match self.downloader.status() {
            crate::fetch::Fetching::Working {
                id,
                name,
                done,
                total,
            } if id == wanted => {
                let percent = percentage(done, total);
                // Redrawn only when the number changes, and only every fifth percent: at 64 KiB a
                // chunk the largest bank in the table reports about sixteen thousand times.
                if percent.is_some_and(|percent| {
                    announced.is_none_or(|announced| percent >= announced.saturating_add(5))
                }) {
                    if let Ok(mut first_run) = self.first_run.lock() {
                        first_run.announced = percent;
                    }
                    self.notify_first_run(crate::firstrun::Notice::Working(format!(
                        "downloading {name} — {}%",
                        percent.unwrap_or(0)
                    )));
                }
            }
            crate::fetch::Fetching::Done { id, name } if id == wanted => {
                self.finish_first_run_soundfont(&name);
            }
            crate::fetch::Fetching::Failed { id, why, .. } if id == wanted => {
                // **The request file is left where it is**, so the next start tries again — up to
                // the three [`crate::firstrun`] allows. A first boot before the network is up is the
                // case this exists for.
                tracing::warn!(bank = %wanted, why = %why, "the first-start SoundFont download failed");
                self.clear_first_run_fetch();
                self.notify_first_run(crate::firstrun::Notice::Failed(why));
            }
            _ => {}
        }
    }

    /// Chooses the bank a first-start download just installed, and retires the request.
    fn finish_first_run_soundfont(&self, name: &str) {
        self.clear_first_run_fetch();
        // The request goes whatever happens next. The download succeeded, so the bank is on the
        // machine and the tick box has been honored; a bank that then refuses to be chosen is a
        // thing to say once, not to retry at every start.
        self.forget_first_run_request();

        let Some(bank) = self.installed_bank_named(name) else {
            tracing::warn!(%name, "the fetched bank is not in the folder it was written to");
            return;
        };
        match self.select_soundfont(&bank.id) {
            Ok(()) => {
                tracing::info!(bank = %bank.id, "chose the SoundFont a setup program asked for");
                self.notify_first_run(crate::firstrun::Notice::Done(format!(
                    "{} is now the instrument bank",
                    bank.name
                )));
            }
            Err(error) => {
                tracing::warn!(bank = %bank.id, %error, "could not choose the fetched bank");
                self.notify_first_run(crate::firstrun::Notice::Failed(error.to_string()));
            }
        }
    }

    /// The next sentence about a first-start download, if there is one.
    ///
    /// Drained by the display once a frame, the way [`crate::dropped::DropInstaller::poll`] is, and
    /// for the same reason: only the newest of these is worth screen space.
    pub fn take_first_run_notice(&self) -> Option<crate::firstrun::Notice> {
        self.first_run.lock().ok()?.notice.take()
    }

    /// Leaves a sentence for the display, replacing any it has not drawn yet.
    fn notify_first_run(&self, notice: crate::firstrun::Notice) {
        if let Ok(mut first_run) = self.first_run.lock() {
            first_run.notice = Some(notice);
        }
    }

    /// Forgets that this start began a download, without touching the request file.
    fn clear_first_run_fetch(&self) {
        if let Ok(mut first_run) = self.first_run.lock() {
            first_run.fetching = None;
            first_run.announced = None;
        }
    }

    /// Removes the request file. A request that cannot be removed is logged and not retried.
    fn forget_first_run_request(&self) {
        if let Err(error) = crate::firstrun::remove(&self.paths) {
            tracing::warn!(%error, "could not remove the first-start SoundFont request");
        }
    }

    /// The installed bank whose file has this name, which is the join between a row in the table and
    /// a file in the folder — the same rule [`measured_level`] and [`Machine::soundfont_offers`]
    /// use.
    fn installed_bank_named(&self, name: &str) -> Option<crate::soundfont::Installed> {
        let (banks, _) = self.soundfont_banks();
        banks.into_iter().find(|bank| {
            bank.path
                .file_name()
                .and_then(|file| file.to_str())
                .is_some_and(|file| file.eq_ignore_ascii_case(name))
        })
    }

    /// Chooses a bank and keeps the choice, putting it in force without stopping the song.
    ///
    /// **The persistent half of what `Ctrl+1`…`Ctrl+9` does for one run**, and it is the same
    /// protocol `--set-soundfont` follows at a shell, deliberately: check the bank plays, work out
    /// the level, write `settings.json`, stash what the level was so choosing another bank later
    /// does not inherit this one's reduction. A second protocol would be a second thing to disagree
    /// with the first about what the owner's own level is.
    pub fn select_soundfont(&self, id: &str) -> Result<(), ControlError> {
        let (banks, _) = self.soundfont_banks();
        let Some(bank) = banks.iter().find(|bank| bank.id == id) else {
            return Err(ControlError::Rejected(format!("no SoundFont called {id}")));
        };

        // Before anything is written, and with the synthesizer's own words: four of the fifteen
        // banks the research note tried are refused where other players accept them, so "this file
        // is fine and this synthesizer is strict" is the message somebody needs at the moment they
        // choose it.
        //
        // **Opening it *is* the check, and the bank is kept rather than dropped.** A separate
        // `check_plays` that loads the bank, reads its defects and throws it away leaves
        // `switch_soundfont` loading the same file again a few lines later — on a Google TV
        // Streamer, two full load-and-free cycles for one choice, each peaking at 674 MB resident,
        // and 4.4 s of stall for the largest bank offered. Nothing about the check is weaker for
        // it: a bank that will not open fails here, before a single settings key is written.
        let loaded = load_bank(&bank.path)?;

        // **The level the research note measured for this bank, if it is one the note measured.**
        // Matched on the filename, which is what the table names and what the owner dropped in the
        // folder — so a `Roland SC-55 v3.7.sf2` put there by hand gets its 0.6 and does not clip two
        // songs in seven, exactly as `task soundfont BANK=sc55-v37` would have set it. A bank the
        // table does not know keeps whatever the owner's level is; nothing here guesses one.
        let measured = measured_level(&bank.path);

        // **Read before the lock, not inside it.** `lock_settings` is what `Machine::locale` takes
        // once a frame — its own doc says "the display asks once a frame" — and what a wallpaper
        // cycle takes on every advance. Opening and parsing a file underneath it puts disk I/O on
        // the lock the picture waits on, which is the discipline this module's header states about
        // `load_bank` two lines above and then broke here. The stall is sub-millisecond, which is
        // why it survived; hoisting it costs nothing at all.
        let existing = crate::soundfont::read(&self.paths);

        {
            let mut settings = self.lock_settings();
            let was = settings.audio.music_volume;
            // What the level would be with no override at all. Switching away from a bank that
            // wanted 0.6 must not leave 0.6 behind: that number was a property of the bank rather
            // than a choice anybody made.
            let baseline = crate::soundfont::restore_to(existing.as_ref(), was).unwrap_or(was);
            let applied = if bank.bundled {
                baseline
            } else {
                measured.unwrap_or(baseline)
            };
            // The bank's **id**, not its path. Selecting the bundled bank means clearing the
            // setting rather than writing a path to it, which is what keeps it right on a machine
            // whose asset directory later changes.
            settings.audio.soundfont = if bank.bundled {
                None
            } else {
                Some(bank.id.clone())
            };
            settings.audio.music_volume = applied;
            drop(settings);

            if bank.bundled {
                if let Err(error) = crate::soundfont::remove(&self.paths) {
                    tracing::warn!(%error, "could not remove the SoundFont level note");
                }
            } else {
                let note = crate::soundfont::stash(existing, was, applied);
                if let Err(error) = crate::soundfont::write(&self.paths, &note) {
                    tracing::warn!(%error, "could not write the SoundFont level note");
                }
            }
        }
        self.save_settings();

        // `None` rather than a level of its own: the settings value was just set to the right one,
        // and the swap falls back to it. Passing it twice would be two sources for one number.
        let close = self.switch_loaded_soundfont(loaded, &bank.path, None)?;
        tracing::info!(
            bank = %bank.name,
            id = %bank.id,
            path = %bank.path.display(),
            pending = !close,
            "chose a SoundFont"
        );
        Ok(())
    }

    /// Why a bank's file is not the machine's to delete, in a sentence, or `None` if it is.
    ///
    /// The package half of this is [`not_mine_to_delete`], and this is its twin for the same reason:
    /// [`Machine::delete_soundfont`] refuses with it and [`Machine::soundfonts`] carries it out to a
    /// page, so the Remove control a page draws and the answer the route gives cannot describe two
    /// different rules. Until this existed the Sound tab omitted Remove for the bundled bank only,
    /// so a `debug.soundfonts` row offered a button that was always refused.
    ///
    /// **Only the permanent refusals.** A download in flight is the third thing `delete_soundfont`
    /// says no to and it stays where it is, because it becomes allowed by waiting — and a control
    /// that vanishes and comes back is a page that looks broken.
    ///
    /// A method rather than a free function, unlike its package twin: what it reads is a settings
    /// section and a field on the row, so there is no filesystem layout to root at a scratch
    /// directory and nothing a free function would make reachable.
    pub(super) fn bank_not_mine_to_delete(
        &self,
        bank: &crate::soundfont::Installed,
    ) -> Option<String> {
        // The bundled bank is an unpacked asset: deleting it would succeed, and it would be back on
        // the next launch. Better to say so than to do nothing convincingly.
        if bank.bundled {
            return Some(
                "the bundled SoundFont ships with the machine and cannot be removed".to_owned(),
            );
        }

        // **A bank a `debug.soundfonts` slot names is not the machine's to remove**, whichever
        // folder it happens to sit in. Nothing else in the `debug.` section can be changed by an API
        // route, and the delete route is the one place that could reach *through* it to a file: the
        // slots are something an owner configured by hand, and a route that deleted what they name
        // would make the switcher's own keys the thing that broke.
        //
        // Compared with `same_file` rather than `==`, and that is load-bearing here in a way it is
        // not elsewhere: `--set-debug-soundfonts` stores each path **as typed** and never
        // absolutises it, so a hand-written relative entry would defeat a literal comparison and
        // leave the file deletable after all.
        //
        // Deleting the `soundfont.rs` exception that used to add a configured path to this list has
        // already closed the sharper half of this by construction — `installed` can only yield files
        // inside folders the machine owns now, where before it could yield the shared asset cache a
        // development build fetches into, and every worktree on the box shares one copy of that.
        let slot = self
            .lock_settings()
            .debug
            .soundfonts
            .iter()
            .position(|slot| crate::soundfont::same_file(&slot.path, &bank.path))?;
        Some(format!(
            "\"{}\" is SoundFont slot {} in debug.soundfonts, so its file is not the machine's to \
             remove: take it out with --set-debug-soundfonts or --clear-debug-soundfonts",
            bank.name,
            slot + 2
        ))
    }

    /// Removes an installed bank from disk.
    ///
    /// **This exists because of the television.** Everywhere else a bank is a file in a folder
    /// `--show-paths` names, and removing one is somebody's file manager. On Android the folder the
    /// downloader writes to is app-*private* storage, which no file manager, USB copy or `adb push`
    /// can reach without `run-as` — so on a box under a screen, with no shell anywhere near it, a
    /// 262 MiB bank fetched by a mistaken tap was permanent until the app was uninstalled.
    ///
    /// Three refusals, each for its own reason:
    ///
    /// * **An id that is not in the list**, resolved through [`crate::soundfont::installed`] rather
    ///   than by rebuilding a path out of the id. That is the rule [`Machine::select_soundfont`]
    ///   already keeps and it is what stops a crafted id reaching the filesystem — ids are slugged
    ///   and do not round-trip.
    /// * **The bundled bank**, which is an unpacked asset: deleting it would succeed, and it would
    ///   be back on the next launch. Better to say so than to do nothing convincingly.
    /// * **While a download is running**, so a delete cannot race the rename that finishes one.
    ///
    /// **Deleting the bank the setting names is allowed**, and falls back to the bundled one first.
    /// Refusing until something else is chosen is the tidier-looking rule and is wrong for the case
    /// above: the selected bank may be the very mistake somebody is trying to undo, and a refusal is
    /// a dead end for somebody holding a D-pad. Going through `select_soundfont` rather than writing
    /// the keys here means the level protocol — the stash, `restore_to`, the live swap — is the
    /// existing one and not a second copy that can disagree with it.
    ///
    /// **The file goes last.** If the fallback fails there is nothing to undo, where deleting first
    /// would leave the machine pointing `audio.soundfont` at a file that is gone.
    pub fn delete_soundfont(&self, id: &str) -> Result<(), ControlError> {
        if self.downloader.status().busy() {
            return Err(ControlError::Unavailable(
                "a SoundFont is being downloaded; try again when it has finished".into(),
            ));
        }

        // `selected` comes back from here rather than being worked out again, so "is this the bank
        // in force?" has one answer in this file. Note what that answer is about: the *setting*, not
        // what the engine is holding. A `Ctrl+1`…`Ctrl+9` slot swaps the bank for this run without
        // writing anything, and deleting a file has no business undoing a debug switch.
        let (banks, selected) = self.soundfont_banks();
        let Some(bank) = banks.iter().find(|bank| bank.id == id) else {
            return Err(ControlError::Rejected(format!("no SoundFont called {id}")));
        };
        // The two **permanent** refusals, asked through the one function the bank list also asks,
        // so a page that leaves Remove off a row leaves it off for the sentence this would have
        // given. The busy check above is deliberately not in there: it becomes allowed by waiting.
        if let Some(reason) = self.bank_not_mine_to_delete(bank) {
            return Err(ControlError::Rejected(reason));
        }
        let path = bank.path.clone();
        let name = bank.name.clone();

        let was_selected = selected == id;
        if was_selected {
            self.select_soundfont(crate::soundfont::BUNDLED_ID)?;
        }

        std::fs::remove_file(&path).map_err(|error| {
            ControlError::Rejected(format!("{} could not be removed: {error}", path.display()))
        })?;
        tracing::info!(
            bank = %name,
            id = %id,
            path = %path.display(),
            was_selected,
            "removed a SoundFont"
        );
        Ok(())
    }

    /// Puts a bank in force now, keeping the song and its position. The swap itself, with no opinion
    /// about where the bank came from.
    ///
    /// Returns whether the stream was rebuilt. `false` means the bank is assigned but a video or
    /// MP3+G song is still playing through the old stream, so it will not be *heard* until the next
    /// MIDI song — a distinction the caller has to be able to report, because "chosen" and
    /// "sounding" are different claims.
    ///
    /// **Extracted so there is one swap and not two.** The debug slots and the remote's picker are
    /// two ways of asking for the same thing, and the ordering below is the whole of why this is
    /// delicate: open before changing anything, snapshot before the stream goes, reload after, level
    /// last. A second copy of that would be a second thing to get subtly wrong.
    fn switch_soundfont(
        &self,
        path: &Path,
        music_volume: Option<f32>,
    ) -> Result<bool, ControlError> {
        // Opened before anything is changed, and on this thread: parsing a bank is tens of
        // megabytes of work that the audio thread's 250 ms housekeeping cadence has no room for,
        // and a bank that will not open must leave the machine exactly as it was. Same ordering
        // `--set-soundfont` follows, and for the same reason.
        let bank = load_bank(path)?;
        self.switch_loaded_soundfont(bank, path, music_volume)
    }

    /// The same swap, for a caller that has already parsed the bank.
    ///
    /// **Split out because parsing one is the expensive half and one caller was doing it twice.**
    /// [`Machine::select_soundfont`] opens the bank to check it plays and then opened it again to
    /// put it in force; measured on a Google TV Streamer, switching between the two largest banks
    /// the machine offers climbed to 674 MB resident, fell back, and climbed to 674 MB *again* —
    /// two full load-and-free cycles for one choice, and 4.4 s of it. Nothing about the check was
    /// wrong; it simply threw away the thing it had just built.
    fn switch_loaded_soundfont(
        &self,
        bank: km_audio::Bank,
        path: &Path,
        music_volume: Option<f32>,
    ) -> Result<bool, ControlError> {
        // Snapshotted together, before the stream goes, because every one of them is about to stop
        // being readable or stop being true.
        let position_ms = self.engine.position_ms();
        let was_playing = self.engine.transport() == Transport::Playing;
        // One lock for both answers, so a song that starts between two reads cannot leave the
        // machine rebuilding a stream it has decided not to reload.
        let (midi, has_song) = {
            let state = self.lock_state();
            let midi = state.loaded.as_ref().and_then(|loaded| {
                loaded
                    .song()
                    .map(|song| (Arc::clone(song), loaded.melody_channel, loaded.fixes))
            });
            (midi, state.loaded.is_some())
        };
        let close = rebuild_stream_for(has_song, midi.is_some());

        if !self.engine.set_soundfont(bank, path.to_path_buf(), close) {
            return Err(ControlError::Unavailable(Refusal::coded(
                NO_SOUND,
                self.engine.sound().describe(),
            )));
        }

        if let Some((song, melody_channel, fixes)) = midi {
            // `Sticky` replays transpose, tempo, the melody mute and the volume when the stream
            // reopens, so only the song and its position are this function's business. `SeekMs`
            // does more than move the clock: it replays every channel's program and controller
            // state onto what is a brand-new synthesizer, which is why the song comes back sounding
            // like itself rather than like sixteen default pianos.
            self.engine.send(Command::Load(km_audio::audio::Load::Midi {
                song,
                melody_channel,
                // Carried from the loaded song rather than found again: the fixes in force include
                // whatever somebody turned on by hand, which re-detecting would not know about.
                fixes,
            }));
            if position_ms > 0 {
                self.engine.send(Command::SeekMs(position_ms));
            }
            if was_playing {
                self.engine.send(Command::Play);
            }
        }

        // The volume travels with the bank, when the bank was measured: banks differ enough in
        // level to clip, and an A/B where one is simply louder answers the wrong question. Sent
        // after the reload so it wins over `Sticky`'s replay of the settings value.
        //
        // **Sent on every swap, including the swaps that have no level of their own**, so a bank
        // that wants no reduction actively puts the machine's level back instead of inheriting the
        // last bank's. See `volume_for_bank`.
        let machine_volume = self.lock_settings().audio.music_volume;
        self.engine.send(Command::SetMusicVolume(volume_for_bank(
            music_volume,
            machine_volume,
        )));

        tracing::debug!(
            path = %path.display(),
            position_ms,
            rebuilt = close,
            "put a SoundFont in force"
        );
        Ok(close)
    }

    /// What the on-screen label should say about the bank, or `None` when the switcher is off.
    ///
    /// Rebuilt each frame rather than cached, because it is three words and the alternative is a
    /// second place for the current slot to live. The label exists at all so that a recording of a
    /// session says which bank was playing — which means it has to be right in a still frame, with
    /// nobody around to ask.
    pub fn debug_soundfont_label(&self) -> Option<String> {
        let total = {
            let settings = self.lock_settings();
            if !settings.soundfont_switcher_on() {
                return None;
            }
            // Slot 1 is a real slot somebody can press, so it counts.
            settings.debug.soundfonts.len() + 1
        };
        let current = self
            .soundfont_slot
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        Some(soundfont_label(
            current.slot,
            total,
            &current.name,
            current.pending,
        ))
    }

    /// Clears a pending bank once a stream has been opened around it.
    ///
    /// Called from the poll loop rather than the engine, which cannot see the difference: the
    /// engine opens streams for a great many reasons and only this one retires a pending swap.
    pub(super) fn settle_pending_soundfont(&self) {
        let Ok(mut current) = self.soundfont_slot.lock() else {
            return;
        };
        if !current.pending {
            return;
        }
        // A MIDI song is playing through the synthesizer, so whatever bank that synthesizer was
        // built around is the one being heard — and since a pending swap already assigned it, the
        // stream now open must be using it.
        let midi_playing = self
            .lock_state()
            .loaded
            .as_ref()
            .is_some_and(|loaded| loaded.kind.is_midi());
        if midi_playing {
            current.pending = false;
        }
    }

    /// Which bank is playing, and why — the HTTP form of `--show-paths`' `soundfont` line.
    ///
    /// **Read from the engine and never re-resolved**, which is the whole difference between this
    /// and the CLI report: `--show-paths` runs before there is a machine, so it has to ask
    /// [`crate::engine::resolve_soundfont`] what *would* be chosen. Here a machine is running and
    /// has already tried, and the two answers disagree in exactly the case worth reporting — a bank
    /// that resolves and then will not parse, which [`crate::engine::choose_instrument`] turns into
    /// a test tone and a re-resolution would call present.
    pub(super) fn describe_soundfont(&self) -> SoundFontStatus {
        // Three answers rather than two, and the third is the one the id change made reachable: a
        // setting naming a bank the folder no longer holds. The machine is on the bundled bank and
        // working, so this is neither `Setting` (it is not what is playing) nor plainly `Bundled`
        // (somebody did choose, and their choice is stale). Resolved against the folder as it is
        // now, which is the same question `soundfont_banks` asks to decide which row is ticked — so
        // the two cannot disagree.
        let configured = self.lock_settings().audio.soundfont.clone();
        let selected = crate::soundfont::resolve(&self.paths, configured.as_deref());
        let chosen_by = match (&configured, &selected.missing) {
            (Some(_), None) => SoundFontChoice::Setting,
            (Some(_), Some(_)) => SoundFontChoice::Fallback,
            (None, _) => SoundFontChoice::Bundled,
        };
        match &self.engine.sound() {
            // `defects` is deliberately not carried onto the wire. `problem` means "why there is no
            // bank", and a bank that dropped a record is playing rather than absent, so putting it
            // there would make every client that shows `problem` report a working machine as broken.
            // It has a home already -- the journal line and `--set-soundfont` -- and giving the API
            // its own field is a change to the published surface rather than a detail of this one.
            crate::engine::Sound::SoundFont { path, .. } => SoundFontStatus {
                path: Some(path.display().to_string()),
                chosen_by: Some(chosen_by),
                playing: SoundKind::SoundFont,
                problem: None,
                // Beside `problem` rather than in it: this machine is working, and a client that
                // showed a stale setting where it shows "there is no bank" would call it broken.
                fallback: selected.missing,
            },
            crate::engine::Sound::TestTone { reason } => SoundFontStatus {
                path: None,
                chosen_by: None,
                playing: SoundKind::TestTone,
                problem: Some(reason.clone()),
                fallback: None,
            },
            crate::engine::Sound::Silent { reason } => SoundFontStatus {
                path: None,
                chosen_by: None,
                playing: SoundKind::Silent,
                problem: Some(reason.clone()),
                fallback: None,
            },
        }
    }
}
