//! What `--set-soundfont` remembers so that `--clear-soundfont` can put it back.
//!
//! Playing a different General MIDI bank is two settings and not one. `audio.soundfont` says which
//! file, and `audio.music_volume` says how loud — and the second is not decoration: of the eleven
//! banks the survey could open, six exceed full scale at the default `1.0`, so
//! MuseScore_General is paired with `0.7` and FluidR3_GM with `0.6`. A command that set the path and
//! left the level alone would hand somebody a bank that clips.
//!
//! Setting both means the old level has to go somewhere, or clearing the override would have to
//! invent one — and `1.0` is a *default*, not necessarily what was there. Hence this file.
//!
//! # Why a file beside `settings.json` and not a key inside it
//!
//! The same argument [`crate::settings::Paths::overlay_asset_dir`] makes for resolving the overlay
//! by rule: **a second way to say something is a second thing that can disagree with the first.** A
//! `music_volume_before_override` key in `settings.json` would be a key an owner could set, edit or
//! copy between machines, and it means nothing on its own — it is a note this command left for
//! itself. So it lives beside the settings file rather than in it, is written only by
//! `--set-soundfont`, and is removed by `--clear-soundfont`.
//!
//! Losing it is not a failure. A clear with no stash clears the path, leaves the level alone and
//! says so, which is the honest thing to do when there is no stashed level to put back.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::Paths;

/// What the file is called, beside `settings.json` in the config directory.
const FILE: &str = "soundfont-override.json";

/// The note `--set-soundfont` leaves for `--clear-soundfont`.
///
/// **Two numbers and deliberately not the bank's name.** A `bank: "musescore"` field would read
/// nicely and be a second place the answer lives: `audio.soundfont` in `settings.json` already says
/// which file is playing, `--show-paths` already prints it, and a name recorded here could disagree
/// with it after any hand edit. Nothing needs it, so nothing stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override {
    /// `audio.music_volume` as it was found, before any of this.
    pub previous_music_volume: f32,
    /// `audio.music_volume` as it was left. **The reason clearing is safe**: if the current level is
    /// no longer this, somebody has tuned it by hand since, and restoring would throw that away.
    pub applied_music_volume: f32,
}

/// Where the note lives.
pub fn file(paths: &Paths) -> PathBuf {
    paths.config_dir.join(FILE)
}

/// The note as it stands, or `None`.
///
/// Missing, unreadable and unparseable all mean the same thing — nothing was recorded — for the
/// reason [`crate::cli`]'s `peek_settings` gives about the settings file: the cost of being wrong is
/// one line of a report, and refusing to clear an override because a scratch file would not parse
/// would be a worse answer than clearing it.
pub fn read(paths: &Paths) -> Option<Override> {
    let text = std::fs::read_to_string(file(paths)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes the note, creating the config directory if this is a fresh install.
pub fn write(paths: &Paths, value: &Override) -> std::io::Result<()> {
    std::fs::create_dir_all(&paths.config_dir)?;
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(file(paths), text)
}

/// Removes it. A note that was not there is not an error — clearing twice is a thing people do.
pub fn remove(paths: &Paths) -> std::io::Result<()> {
    match std::fs::remove_file(file(paths)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// What a `--set-soundfont` should record, given what is already recorded.
///
/// **The stash is written once and then left alone**, which is what makes a chain of switches
/// behave: musescore at 0.7, then fluidr3 at 0.6, then clear, puts back whatever the level was
/// before musescore — not 0.7. Only `applied_music_volume` moves, because that is the value clearing
/// compares against.
pub fn stash(existing: Option<Override>, current: f32, applied: f32) -> Override {
    Override {
        previous_music_volume: match existing {
            Some(previous) => previous.previous_music_volume,
            None => current,
        },
        applied_music_volume: applied,
    }
}

/// The level to go back to, or `None` for "leave it where it is".
///
/// `None` means either that nothing was recorded, or that the level has been changed by hand since
/// it was applied and is now somebody's own choice rather than this command's.
///
/// **Both halves of the pair ask this**, which is less obvious for `--set-soundfont` than for
/// `--clear-soundfont` and matters just as much. Switching from a bank that needed `0.8` to one that
/// needs nothing must not leave `0.8` behind: that number was a property of the first bank. So a set
/// with no explicit level starts from this answer rather than from whatever is currently in the
/// settings file.
pub fn restore_to(note: Option<&Override>, current: f32) -> Option<f32> {
    let note = note?;
    // Compared exactly, and that is right rather than lax: both sides are values this command wrote
    // or read back from the same JSON, so there is no arithmetic between them to accumulate error.
    // An `f32` that has been through `serde_json` and back is bit-identical.
    (note.applied_music_volume == current).then_some(note.previous_music_volume)
}

/// Reads the bank, so that a file that will not play is refused before it is written down — and
/// reports what loading it cost.
///
/// **This is the part that earns the flag its place**, rather than it being a settings editor with a
/// long name. Fifteen banks were opened through this machine's own
/// synthesizer and **four of them did not load at all** — a whole bank refused over a single bad
/// sample header. Without this check the symptom of pointing `audio.soundfont` at one of them is a
/// machine that comes up on a sine test tone with the reason in a log nobody is reading.
///
/// The returned [`km_audio::BankDefects`] is the other half of the same argument. Since the fork,
/// that single bad sample header is dropped instead of refusing the bank — so the failure this check
/// was written for has partly turned into a bank that loads *incomplete*, and the person choosing it
/// by hand is exactly who should hear about that. The bank is parsed either way; nothing extra is
/// read to find out.
pub fn check_plays(path: &Path) -> Result<km_audio::BankDefects, String> {
    km_audio::Bank::load(path)
        .map(|bank| bank.defects().clone())
        .map_err(|error| error.to_string())
}

/// The id the bundled bank answers to, in the picker and on the API.
///
/// A name rather than a path because it is not one file: [`crate::engine::resolve_soundfont`] picks
/// between three candidate subpaths across two directories, and which one wins is not something a
/// remote should be restating. Selecting it means *clear the setting*, not *set the setting to this
/// path* — the difference matters on a machine whose asset directory later changes.
pub const BUNDLED_ID: &str = "bundled";

/// One bank the machine can be switched to now, without fetching anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Stable across restarts and safe in a URL: the file stem, or [`BUNDLED_ID`].
    pub id: String,
    /// What to show. The file stem for a dropped-in bank, since nothing else in the file is
    /// trustworthy — a `.sf2`'s internal name is frequently the name of the bank it was copied from.
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub bundled: bool,
}

/// Every bank on this machine: the bundled one, then whatever is in the owner's folder.
///
/// **Sorted by name, with the bundled bank first**, so the list does not reorder itself when a
/// download lands and the row somebody was about to tap moves out from under their thumb.
///
/// Unreadable entries are skipped rather than reported. A folder the owner drops files into will
/// collect things that are not banks, and a picker is not the place to explain a stray `.txt` — the
/// bank that will not *play* is a different case and is caught by [`check_plays`] at the moment
/// somebody chooses it, which is when they are in a position to do something about it.
/// Which bank a setting names, and why it does not name one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chosen {
    /// The bank's file, when the id resolved to one. `None` means fall through to the bundled bank.
    pub path: Option<PathBuf>,
    /// Why the setting named nothing, when it named something that is not there.
    ///
    /// `None` both when nothing was configured — an ordinary machine on the bundled bank — and when
    /// the id resolved. It is set only for a **stale** id, which is the one case worth telling
    /// somebody about, and it is the sentence they are told.
    pub missing: Option<String>,
}

/// Turns `audio.soundfont`'s id into a file, against the folder as it is now.
///
/// **The reversal that ids make possible.** While the setting held a *path*, an explicit name that
/// was not there was worth refusing over — the engine did, and the machine came up on a sine test
/// tone rather than "quietly falling back to a bundled bank the operator did not ask for". That
/// reasoning was right about a path and is wrong about an id: an id that matches nothing is a
/// **stale name**, not an instruction, and the folder is what says which banks exist. The bundled
/// bank is a working machine; a test tone is not. So this falls back, and carries the reason so that
/// something can say what it did.
pub fn resolve(paths: &Paths, configured: Option<&str>) -> Chosen {
    let Some(id) = configured else {
        return Chosen::default();
    };
    match installed(paths).into_iter().find(|bank| bank.id == id) {
        Some(bank) if bank.bundled => Chosen::default(),
        Some(bank) => Chosen {
            path: Some(bank.path),
            missing: None,
        },
        None => Chosen {
            path: None,
            missing: Some(format!(
                "the chosen SoundFont \"{id}\" is not in the SoundFont folder any more, so the \
                 bundled bank is playing"
            )),
        },
    }
}

pub fn installed(paths: &Paths) -> Vec<Installed> {
    let mut banks = Vec::new();

    if let Ok(path) = crate::engine::resolve_soundfont(None, paths) {
        banks.push(Installed {
            id: BUNDLED_ID.to_owned(),
            name: "Bundled".to_owned(),
            bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            path,
            bundled: true,
        });
    }

    // **Both folders, private first.** On Android that second one is the only place a file manager,
    // a USB copy or `adb push` can reach — see [`Paths::soundfonts_dirs`]. Everywhere else it is one
    // folder and this loop runs once.
    //
    // A name present in both is kept from the first, which is the rule `packages_to_install` uses
    // and is the same argument: the machine's own download wins over a copy on shared storage,
    // rather than which one wins depending on what happened to be there.
    let mut own: Vec<Installed> = Vec::new();
    for dir in paths.soundfonts_dirs() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_bank_file(&path) {
                continue;
            }
            let Some(id) = bank_id(&path) else { continue };
            if own.iter().any(|bank| bank.id == id) {
                continue;
            }
            own.push(Installed {
                id,
                name: bank_name(&path),
                bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
                path,
                bundled: false,
            });
        }
    }

    own.sort_by_key(|bank| bank.name.to_lowercase());
    banks.extend(own);
    banks
}

/// Whether two paths name the same file.
///
/// **Not `==`, because the two sides are spelled differently and both spellings are correct.** It
/// was written when `audio.soundfont` held an absolutised path while a scan of a relative
/// `--data-dir` yielded relative ones; comparing them as written cost two bugs at once, and then a
/// third — the picker reporting the *bundled* bank as selected while the machine played another.
/// **Choosing by id has made that whole class impossible rather than handled**, so this no longer
/// has anything to do with which bank is selected.
///
/// It survives for the two places that still compare *files*: refusing to overwrite a different bank
/// of the same name when one is installed, and deciding whether a bank about to be deleted is one
/// `debug.soundfonts` names. The second is the one that needs it, because `--set-debug-soundfonts`
/// stores paths **as typed** and never absolutises them, so a hand-written relative entry would
/// defeat `==` and leave a slot's file deletable after all.
///
/// Absolute rather than canonical: `canonicalize` touches the filesystem, resolves symlinks — which
/// an owner may reasonably have used to point at a bank on another volume — and on Windows returns
/// a `\\?\` prefix that would then have to be stripped everywhere else. Both callers are asking
/// whether two names refer to one file, not whether two files have identical contents.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(a), Ok(b)) => a == b,
        // Absolutising fails only for an empty path or a path the current directory cannot be read
        // for. Falling back to the literal comparison keeps the old answer rather than inventing one.
        _ => a == b,
    }
}

/// `.sf2` only, case-insensitively.
///
/// **`.sf3` is deliberately not here.** `rustysynth` refuses it outright — the research note's own
/// §3 records `SoundFont3 is not yet supported` — so listing one would offer a row that can only
/// ever fail, and the MuseScore mirror serves an `.sf3` beside every `.sf2` for anybody to
/// accidentally copy.
fn is_bank_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sf2"))
}

/// What to call a bank on screen: its file stem, unchanged.
///
/// The file's own `INAM` chunk is deliberately not used. Banks are copied and re-cut constantly and
/// the internal name is routinely the name of whatever they were cut from — the research note found
/// several under names belonging to other banks entirely. What the owner called the file is the one
/// name they will recognize, because they are the one who put it there.
fn bank_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// A URL-safe id from the file stem: lowercase, and every run of anything else becomes one `-`.
///
/// **Slugged rather than passed through**, because these ids travel in URL path segments and real
/// bank filenames are full of spaces and dots — `Roland SC-55 v3.7.sf2`, `SC-55 SoundFont v1.2b.sf2`,
/// `Arachno SoundFont - Version 1.0.sf2`. Percent-encoding would work and would put `%20` in front
/// of anybody reading a log or a URL for the rest of the feature's life.
///
/// It does not need to round-trip to a filename: selection looks the id up in [`installed`] rather
/// than rebuilding a path from it, which is also what stops a crafted id reaching the filesystem.
pub(crate) fn bank_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let mut id = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-').to_owned();
    if id.is_empty() || id == BUNDLED_ID {
        return None;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real filenames from the bank table, which is where the awkward characters come from.
    #[test]
    fn a_bank_id_is_url_safe_and_its_name_is_the_file_as_named() {
        let cases = [
            (
                "Roland SC-55 v3.7.sf2",
                "roland-sc-55-v3-7",
                "Roland SC-55 v3.7",
            ),
            (
                "SC-55 SoundFont v1.2b.sf2",
                "sc-55-soundfont-v1-2b",
                "SC-55 SoundFont v1.2b",
            ),
            (
                "Arachno SoundFont - Version 1.0.sf2",
                "arachno-soundfont-version-1-0",
                "Arachno SoundFont - Version 1.0",
            ),
            ("FluidR3_GM.sf2", "fluidr3-gm", "FluidR3_GM"),
            (
                "MuseScore_General.sf2",
                "musescore-general",
                "MuseScore_General",
            ),
        ];
        for (file, id, name) in cases {
            let path = PathBuf::from(file);
            assert_eq!(bank_id(&path).as_deref(), Some(id), "id of {file}");
            assert_eq!(bank_name(&path), name, "name of {file}");
        }
    }

    /// Nothing may mint the bundled bank's id, or an empty one.
    #[test]
    fn an_id_that_would_collide_with_the_bundled_bank_or_be_empty_is_refused() {
        assert_eq!(bank_id(&PathBuf::from("bundled.sf2")), None);
        // Case and punctuation both slug down to the same reserved word.
        assert_eq!(bank_id(&PathBuf::from("Bundled.sf2")), None);
        assert_eq!(bank_id(&PathBuf::from("-_-.sf2")), None);
    }

    /// `.sf3` loads in nothing this machine ships, so it is never offered.
    #[test]
    fn only_sf2_files_are_offered() {
        // `is_bank_file` also tests `is_file`, so this checks the extension rule on its own terms.
        assert!(!is_bank_file(&PathBuf::from("MuseScore_General.sf3")));
        assert!(!is_bank_file(&PathBuf::from("notes.txt")));
    }

    /// Android's public folder is scanned too, and the private one wins a name it also has.
    ///
    /// The case this exists for is not a collision, though: it is a device with fifty banks on it
    /// for a listening test, none of which can be put in the private directory without `run-as`.
    #[test]
    fn banks_are_found_in_the_shared_folder_as_well_as_the_private_one() {
        // The same scratch shape settings.rs's tests use, rather than a dependency for one test:
        // named after the process and thread so a parallel run cannot collide with itself.
        let dir = std::env::temp_dir().join(format!(
            "km-soundfont-shared-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut paths = Paths::rooted_at(dir.join("private"));
        paths.extra_data_dir = Some(dir.join("shared"));
        paths.create().expect("create");

        // One in each, plus a name that is in both.
        std::fs::write(paths.soundfonts_dir().join("Private.sf2"), b"x").expect("write");
        let shared = paths.soundfonts_dirs()[1].clone();
        std::fs::write(shared.join("Shared.sf2"), b"x").expect("write");
        std::fs::write(paths.soundfonts_dir().join("Both.sf2"), b"private").expect("write");
        std::fs::write(shared.join("Both.sf2"), b"shared").expect("write");

        let found = installed(&paths);
        let ids: Vec<&str> = found.iter().map(|bank| bank.id.as_str()).collect();
        assert!(ids.contains(&"private"), "{ids:?}");
        assert!(ids.contains(&"shared"), "{ids:?}");

        // `packages_to_install`'s rule: whatever the machine has itself keeps the name.
        let both = found.iter().find(|bank| bank.id == "both").expect("both");
        assert_eq!(both.path, paths.soundfonts_dir().join("Both.sf2"));
        assert_eq!(ids.iter().filter(|id| **id == "both").count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stale id falls back to the bundled bank, with a reason, rather than to a test tone.
    ///
    /// **The reversal this whole change turns on.** While the setting held a path, a name that was
    /// not there was worth refusing over — and the machine came up on a sine wave with the reason in
    /// a log line, while the picker reported *"Bundled"* as selected because the row for a missing
    /// file was dropped and the lookup fell through. So the one screen that could have said what was
    /// wrong said the opposite. An id that matches nothing is a stale name, not an instruction.
    #[test]
    fn a_stale_bank_id_falls_back_to_bundled_and_says_why() {
        let dir = std::env::temp_dir().join(format!(
            "km-soundfont-stale-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted_at(dir.clone());
        paths.create().expect("create");
        let bank = paths.soundfonts_dir().join("Chosen.sf2");
        std::fs::write(&bank, b"x").expect("write");

        // The bank that is there resolves to its file and says nothing.
        let chosen = resolve(&paths, Some("chosen"));
        assert_eq!(chosen.path.as_deref(), Some(bank.as_path()));
        assert!(chosen.missing.is_none());

        // The one that is not resolves to nothing -- so the caller falls through to the bundled
        // candidates -- and carries a sentence naming it.
        let stale = resolve(&paths, Some("gone"));
        assert!(stale.path.is_none(), "a stale id must not name a file");
        let reason = stale.missing.expect("a stale id is worth saying out loud");
        assert!(reason.contains("gone"), "{reason}");
        assert!(reason.contains("bundled"), "{reason}");

        // And nothing configured is not a fault: an ordinary machine on the bundled bank.
        let none = resolve(&paths, None);
        assert!(none.path.is_none());
        assert!(none.missing.is_none(), "no setting is not a stale setting");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bank outside every scanned folder is not a row at all.
    ///
    /// Adding one on the strength of `audio.soundfont` naming it is the only route by which
    /// `installed` could yield a file the machine does not own — and therefore the only route by
    /// which `delete_soundfont` could reach one. Not adding it closes that **by construction**
    /// rather than by a check somebody has to remember.
    #[test]
    fn a_bank_outside_the_folders_is_not_offered() {
        let dir = std::env::temp_dir().join(format!(
            "km-soundfont-outside-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted_at(dir.clone());
        paths.create().expect("create");
        let elsewhere = dir.join("Elsewhere.sf2");
        std::fs::write(&elsewhere, b"x").expect("write");

        let ids: Vec<String> = installed(&paths).into_iter().map(|bank| bank.id).collect();
        assert!(
            !ids.iter().any(|id| id == "elsewhere"),
            "the folders are what say which banks exist: {ids:?}"
        );
        // ...and it cannot be chosen either, which is what makes `--set-soundfont` install it.
        assert!(resolve(&paths, Some("elsewhere")).path.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// And with no shared folder there is one directory, which is every platform but Android.
    #[test]
    fn there_is_one_folder_where_there_is_no_shared_one() {
        let paths = Paths::rooted_at(std::env::temp_dir().join("km-no-shared"));
        assert_eq!(paths.soundfonts_dirs(), vec![paths.soundfonts_dir()]);
    }

    fn note(previous: f32, applied: f32) -> Override {
        Override {
            previous_music_volume: previous,
            applied_music_volume: applied,
        }
    }

    #[test]
    fn a_first_stash_records_the_level_it_found() {
        let stashed = stash(None, 0.9, 0.8);
        assert_eq!(stashed.previous_music_volume, 0.9);
        assert_eq!(stashed.applied_music_volume, 0.8);
    }

    /// The chain that the "written once" rule exists for: switching bank to bank must not make the
    /// previous bank's level the thing that gets restored.
    #[test]
    fn a_second_stash_keeps_the_original_level_and_moves_the_applied_one() {
        let first = stash(None, 0.9, 0.8);
        let second = stash(Some(first), 0.8, 0.7);
        assert_eq!(
            second.previous_music_volume, 0.9,
            "still what it was before any of this"
        );
        assert_eq!(second.applied_music_volume, 0.7);
    }

    #[test]
    fn clearing_restores_the_level_that_was_found() {
        assert_eq!(restore_to(Some(&note(0.9, 0.8)), 0.8), Some(0.9));
    }

    /// A level tuned by hand since the override was set is somebody's own choice, and clearing must
    /// not throw it away.
    #[test]
    fn clearing_keeps_a_level_that_has_been_changed_since() {
        assert_eq!(restore_to(Some(&note(0.9, 0.8)), 0.5), None);
    }

    #[test]
    fn clearing_with_nothing_recorded_leaves_the_level_alone() {
        assert_eq!(restore_to(None, 0.8), None);
    }

    /// The whole switching sequence, as `cli::set_soundfont` and `cli::clear_soundfont` compose
    /// these two functions. It is written out because the interesting behavior is in the
    /// composition rather than in either half, and because getting it wrong is silent: a bank that
    /// plays 20% quiet than it should sounds like a quiet bank.
    #[test]
    fn switching_between_banks_never_leaves_a_previous_bank_s_reduction_behind() {
        let mut level = 1.0_f32;
        let mut note = None;

        // A bank that clips at 1.0, so it is asked for at 0.8.
        let applied = 0.8;
        note = Some(stash(note, level, applied));
        level = applied;

        // ...then one that needs no reduction, and asks for none. The level must go back to 1.0 --
        // 0.8 was the previous bank's requirement, not the owner's preference.
        let baseline = restore_to(note.as_ref(), level).unwrap_or(level);
        assert_eq!(
            baseline, 1.0,
            "the second bank starts from the owner's own level"
        );
        note = Some(stash(note, level, baseline));
        level = baseline;

        // ...and clearing from there is a no-op on the level rather than a second restore.
        assert_eq!(restore_to(note.as_ref(), level), Some(1.0));
        assert_eq!(level, 1.0);
    }

    /// The guard that keeps the rule above from eating a hand edit: 0.5 was typed by somebody while
    /// an override was in force, so switching banks must start from 0.5 and not from 1.0.
    #[test]
    fn a_hand_edited_level_survives_a_switch_as_well_as_a_clear() {
        let note = stash(None, 1.0, 0.8);
        let hand_edited = 0.5;
        assert_eq!(restore_to(Some(&note), hand_edited), None);
        let baseline = restore_to(Some(&note), hand_edited).unwrap_or(hand_edited);
        assert_eq!(baseline, 0.5);
    }

    #[test]
    fn a_note_round_trips_through_json() {
        let written = note(1.0, 0.8);
        let text = serde_json::to_string(&written).expect("serialize");
        let read: Override = serde_json::from_str(&text).expect("parse");
        assert_eq!(read, written);
    }

    /// Anything that is not a bank is refused, which is the whole reason this check exists.
    #[test]
    fn a_file_that_is_not_a_bank_is_refused() {
        let path = std::env::temp_dir().join("km-not-a-bank.sf2");
        std::fs::write(&path, b"this is not a RIFF sfbk file").expect("write");
        let error = check_plays(&path).expect_err("a text file is not a soundfont");
        assert!(!error.is_empty(), "the refusal says something");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_that_is_not_there_is_refused() {
        assert!(check_plays(Path::new("no/such/bank.sf2")).is_err());
    }
}
