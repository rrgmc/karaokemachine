//! Which machine this program is pointed at, between runs.
//!
//! **A third file beside `settings.json` and `provider-keys.json`, and it is a third for their
//! reason.** [`crate::pictures::Settings`] is what the Pictures section is set to look for and
//! [`crate::keys`] is a credential; a machine address is neither, and folding it into either would
//! make "remember which machine" the same decision as "remember my search terms" or "remember my
//! key". Those were deliberately separated — see the module header on [`crate::keys`] — and this
//! stays out of both.
//!
//! **Not in [`crate::machine`], which is the other obvious home.** That module's header carries the
//! standing rule that it writes nothing down: *a bearer token in a file is a credential this program
//! was not asked to keep*. A file write beside that paragraph reads as a contradiction, whatever the
//! comment next to it says.
//!
//! **The storage is `km_api::discover::known` rather than a copy of it.** A line of text with its
//! own write-through-a-rename would be the same idiom kept in three programs at once. What is stored
//! is a record: the machine's instance id, its address, its name and when it last answered. The id
//! is the part that matters, because a home network moves addresses and an identity is the only
//! thing that survives that.
//!
//! ## What is remembered is what somebody chose, not what answered
//!
//! `km-remote` writes its address down only once the machine has replied, which is right for a
//! remote: an address that never answers is of no use to it. **It is the wrong rule here.** This
//! program is built around the machine being off — see the crate header, *"a machine that is
//! switched off, or whose password nobody remembers, is then a delay rather than a dead end"* — so
//! somebody may well point it at a television box that is unplugged, download a bank, and send it
//! tomorrow. Forgetting the address overnight because nothing answered would defeat the one
//! behavior this program is most careful about.

use std::path::{Path, PathBuf};

use km_api::discover::known::{self, Known, Why};

/// Where the chosen machine is written down.
const MACHINE_FILE: &str = "machine.json";

/// Where the chosen machine is written down.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(MACHINE_FILE)
}

/// The machine chosen last time, if there was one.
///
/// A file that is missing, unreadable, blank or written by a later build all mean the same thing to
/// a caller, which is that there is no machine yet.
pub fn load(data_dir: &Path) -> Option<Known> {
    known::read(&path(data_dir))
}

/// Writes the chosen machine down, so the next run opens already pointed at it.
///
/// **Through a temporary file and a rename**, so an interrupted write leaves the old record rather
/// than half of a new one. That is `known::write`'s doing now rather than a copy of it here.
///
/// **The identity is not set from here, and that is the module header's rule in code.** What
/// somebody chose is an *address*; what is at that address is something only a `/discover` can say,
/// and this program is built around that answer never arriving. So a chosen machine starts with no
/// id and gains one the first time it answers — see [`answered`].
pub fn save(data_dir: &Path, url: &str) {
    let _ = std::fs::create_dir_all(data_dir);
    let keep = load(data_dir).filter(|known| known.url == url);
    known::write(
        &path(data_dir),
        &keep.unwrap_or_else(|| Known::at(url, Why::Chosen)),
    );
}

/// Records what the machine at the chosen address said about itself.
///
/// **This does not choose anything**, which is why it can be called from a status check: the address
/// is whatever somebody already picked, and all this adds is *which machine is there*. That is what
/// makes a later move recognizable as the same machine rather than as a new choice nobody made.
pub fn answered(data_dir: &Path, url: &str, id: &str, name: Option<String>) {
    let Some(known) = load(data_dir).filter(|known| known.url == url) else {
        return;
    };
    if known.id.as_deref() == Some(id) && known.name == name {
        return;
    }
    known::write(
        &path(data_dir),
        &known.answered(id, name, std::time::SystemTime::now()),
    );
}

/// Follows the chosen machine to a new address.
///
/// Called only where the id in the record is seen elsewhere, which is a fact about *where a machine
/// somebody already chose has gone* rather than a choice between machines.
pub fn moved(data_dir: &Path, url: &str) {
    let Some(known) = load(data_dir) else {
        return;
    };
    known::write(
        &path(data_dir),
        &Known {
            url: url.to_owned(),
            ..known
        },
    );
}

/// Forgets the chosen machine.
///
/// What clearing the address field does. A file that was not there is not a failure — the caller
/// asked for it to be gone, and it is.
pub fn forget(data_dir: &Path) {
    known::forget(&path(data_dir));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_machine_comes_back_next_run() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        // Nothing chosen yet, which is what a fresh install is.
        assert_eq!(load(dir.path()), None);

        save(dir.path(), "http://192.168.1.42:8177");
        assert_eq!(
            load(dir.path()).map(|known| known.url),
            Some("http://192.168.1.42:8177".to_owned())
        );
        assert_eq!(
            load(dir.path()).and_then(|known| known.id),
            None,
            "choosing an address says nothing about which machine is at it"
        );

        // ...and clearing the address field takes it away for good.
        forget(dir.path());
        assert_eq!(load(dir.path()), None);
        assert!(!path(dir.path()).exists());
    }

    #[test]
    fn forgetting_nothing_is_not_a_failure() {
        // Clearing an address field that was already empty, which is a thing somebody can do.
        let dir = tempfile::tempdir().expect("a temporary folder");
        forget(dir.path());
        forget(dir.path());
        assert_eq!(load(dir.path()), None);
    }

    #[test]
    fn a_blank_file_is_the_same_as_no_machine() {
        // Which is what a hand-edited file, or a truncated one, can leave behind. Every caller needs
        // one answer for it and the answer is "no machine yet".
        let dir = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(path(dir.path()), "   \n").expect("write");
        assert_eq!(load(dir.path()), None);
    }

    /// A machine that answers gains an identity without anybody having chosen again, and a machine
    /// that then moves is followed to the new address with that identity intact.
    #[test]
    fn answering_records_the_identity_and_a_move_keeps_it() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        save(dir.path(), "http://192.168.1.42:8177");

        answered(
            dir.path(),
            "http://192.168.1.42:8177",
            "abc123",
            Some("Living Room".to_owned()),
        );
        let known = load(dir.path()).expect("still chosen");
        assert_eq!(known.id.as_deref(), Some("abc123"));
        assert_eq!(known.name.as_deref(), Some("Living Room"));

        moved(dir.path(), "http://192.168.1.77:8177");
        let known = load(dir.path()).expect("still chosen");
        assert_eq!(known.url, "http://192.168.1.77:8177");
        assert_eq!(known.id.as_deref(), Some("abc123"), "the same machine");
    }

    /// An answer at an address nobody chose is not recorded — that would be this program pointing
    /// itself somewhere, which is the one thing its rule forbids.
    #[test]
    fn an_answer_from_somewhere_else_is_not_recorded() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        save(dir.path(), "http://192.168.1.42:8177");
        answered(dir.path(), "http://10.0.0.1:8177", "stranger", None);
        assert_eq!(load(dir.path()).and_then(|known| known.id), None);
    }

    #[test]
    fn nothing_is_left_behind_by_a_write() {
        // The rename's temporary file is this program's own litter if it survives, and it would sit
        // in the same folder somebody is told holds their packs.
        let dir = tempfile::tempdir().expect("a temporary folder");
        save(dir.path(), "http://192.168.1.42:8177");
        let left: Vec<String> = std::fs::read_dir(dir.path())
            .expect("the folder")
            .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
            .collect();
        assert_eq!(left, [MACHINE_FILE]);
    }
}
