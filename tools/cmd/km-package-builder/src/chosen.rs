//! Which machine this tool would install into, and where that fact is kept.
//!
//! **In the workspace database, and that is the decision rather than a convenience.** A `.kmbuild`
//! is a document — see `A corpus is a document` in `docs/decisions/curation.md` — and a second
//! computer opening the same corpus should talk to the same machine. An identity kept anywhere else
//! would be free to disagree with the address it describes.
//!
//! **One record, as a [`Known`] in JSON in one setting.** An address, an id and a name have to be
//! written together and read together, which is a record; it is the same type `km-remote` and
//! `km-admin` keep, so the three programs share a shape as well as a policy.
//! What it buys beyond tidiness is `last_connected`: the Settings page can say that nothing has
//! answered at this address in six hours, which is exactly the state a corpus carried to another
//! house is in.
//!
//! ## What is remembered is what somebody chose, not what answered
//!
//! `km-remote` writes its address down only once the machine has replied, which is right for a
//! remote: an address that never answers is of no use to it. **It is the wrong rule here**, exactly
//! as it is in `km-admin` — somebody points this at a television box that is unplugged and curates a
//! corpus for a week before sending anything to it. So a chosen address is written immediately and
//! gains an id the first time something answers at it.
//!
//! ## And a workspace that has never been told takes this computer's last choice
//!
//! A brand-new `.kmbuild` used to start at `http://127.0.0.1:8177` however many times its owner had
//! pointed this tool somewhere else, so the first install went to a loopback address with nothing on
//! it. [`seed`] fills a workspace that has never been told from a per-user record — and that record
//! is written **only from an address somebody set by hand in this tool**, never from a browse. That
//! distinction is the whole of why it is not the adopting `Discovering a machine in the package
//! builder` forbids: seeding a new workspace from a choice the same person made on the same computer
//! is not pointing this tool at a machine nobody named.

use std::path::PathBuf;

use km_api::discover::known::{self, Known, Why};

use crate::db::{Db, DbError};

/// The setting holding the machine this workspace was told about, as a [`Known`] in JSON.
pub const SETTING: &str = "app_machine";

/// The environment variable that moves the per-user record, for a test or a second install.
///
/// The same shape [`crate::recent`] uses and for the same reason — see its own note.
pub const ENV_VAR: &str = "KM_PACKAGE_BUILDER_MACHINE";

/// The machine this workspace was told about, if it has been told.
///
/// A record this build cannot parse is no record, which is the same answer every other reader of a
/// [`Known`] gives.
pub fn load(db: &Db) -> Result<Option<Known>, DbError> {
    Ok(db
        .setting(SETTING)?
        .and_then(|text| serde_json::from_str(&text).ok()))
}

/// Points this workspace at an address somebody typed or pressed.
///
/// **The identity is dropped, which is the guard the follow needs.** An address somebody named has
/// not said what is at it, and inheriting the previous machine's id would let the follow drag that
/// address straight off to wherever that machine went — the opposite of what typing one means. It is
/// filled in again by [`answered`], from a `/discover` that replied *at this address*.
///
/// It also becomes this computer's remembered choice, for [`seed`]. `--machine` never reaches here;
/// see `State::pin_machine`.
pub fn save(db: &Db, url: &str) -> Result<(), DbError> {
    write(db, &Known::at(url, Why::Chosen))?;
    remember(url);
    Ok(())
}

/// Records what the machine at the address in force said about itself.
///
/// **Only from a `/discover` that answered at that address.** Anything else would be this tool
/// deciding which machine it is pointed at, which is the one thing its rule forbids.
pub fn answered(db: &Db, url: &str, id: &str, name: Option<String>) -> Result<(), DbError> {
    let Some(known) = load(db)?.filter(|known| known.url == url) else {
        return Ok(());
    };
    if known.id.as_deref() == Some(id) && known.name == name {
        return Ok(());
    }
    write(db, &known.answered(id, name, std::time::SystemTime::now()))
}

/// Follows the machine this workspace knows to a new address.
///
/// Called only where `known::choose` said so, which is a fact about where a machine somebody already
/// chose has gone rather than a choice between machines.
pub fn moved(db: &Db, url: &str) -> Result<(), DbError> {
    let Some(known) = load(db)? else {
        return Ok(());
    };
    write(
        db,
        &Known {
            url: url.to_owned(),
            ..known
        },
    )
}

fn write(db: &Db, known: &Known) -> Result<(), DbError> {
    let text = serde_json::to_string(known)
        .map_err(|error| DbError::Rejected(format!("could not write the machine: {error}")))?;
    db.set_setting(SETTING, &text)
}

/// Where this computer's last hand-set machine is written down, or `None` if there is nowhere.
///
/// `None` under `cfg(test)` for [`crate::recent::path`]'s reason: a test run must not write into the
/// user's own config folder, and a seed that reached across from another test would make the tests
/// order-dependent.
pub fn seed_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    if let Some(named) = std::env::var_os(ENV_VAR) {
        let named = PathBuf::from(named);
        return (!named.as_os_str().is_empty()).then_some(named);
    }
    let dirs = directories::ProjectDirs::from("", "", "km-package-builder")?;
    Some(dirs.config_dir().join("machine.json"))
}

/// The machine this computer was last pointed at by hand, for a workspace that has never been told.
pub fn seed() -> Option<Known> {
    known::read(&seed_path()?)
}

/// Writes this computer's choice down. **Only ever called from [`save`]**, which is only ever
/// reached from the Settings box or a discovery row somebody pressed. `--machine` is neither: it holds
/// for the run and is written nowhere.
fn remember(url: &str) {
    let Some(path) = seed_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    known::write(&path, &Known::at(url, Why::Chosen));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scratch;

    fn db(name: &str) -> (Scratch, Db) {
        let scratch = Scratch::new(name);
        let db = Db::create(&scratch.0).expect("create");
        (scratch, db)
    }

    /// A workspace that was never told holds nothing, and one that was told remembers the address
    /// without pretending to know what is at it.
    #[test]
    fn choosing_an_address_says_nothing_about_which_machine_is_there() {
        let (_scratch, db) = db("chosen-basic");
        assert_eq!(load(&db).expect("load"), None);

        save(&db, "http://192.168.1.42:8177").expect("save");
        let known = load(&db).expect("load").expect("chosen");
        assert_eq!(known.url, "http://192.168.1.42:8177");
        assert_eq!(known.id, None);
    }

    /// A machine that answers gains an identity, and one that moves keeps it.
    #[test]
    fn answering_records_the_identity_and_a_move_keeps_it() {
        let (_scratch, db) = db("chosen-answered");
        save(&db, "http://192.168.1.42:8177").expect("save");

        answered(
            &db,
            "http://192.168.1.42:8177",
            "abc123",
            Some("Living Room".to_owned()),
        )
        .expect("answered");
        let known = load(&db).expect("load").expect("chosen");
        assert_eq!(known.id.as_deref(), Some("abc123"));
        assert_eq!(known.name.as_deref(), Some("Living Room"));

        moved(&db, "http://192.168.1.77:8177").expect("moved");
        let known = load(&db).expect("load").expect("chosen");
        assert_eq!(known.url, "http://192.168.1.77:8177");
        assert_eq!(known.id.as_deref(), Some("abc123"), "the same machine");
    }

    /// An answer from an address nobody chose is not recorded.
    #[test]
    fn an_answer_from_somewhere_else_is_not_recorded() {
        let (_scratch, db) = db("chosen-stranger");
        save(&db, "http://192.168.1.42:8177").expect("save");
        answered(&db, "http://10.0.0.1:8177", "stranger", None).expect("answered");
        assert_eq!(load(&db).expect("load").and_then(|known| known.id), None);
    }

    /// Setting an address by hand clears the identity, which is what stops the follow dragging a
    /// typed address off to wherever the previous machine went.
    #[test]
    fn setting_an_address_by_hand_clears_the_identity() {
        let (_scratch, db) = db("chosen-retyped");
        save(&db, "http://192.168.1.42:8177").expect("save");
        answered(&db, "http://192.168.1.42:8177", "abc123", None).expect("answered");

        save(&db, "http://10.0.0.5:8177").expect("save");
        let known = load(&db).expect("load").expect("chosen");
        assert_eq!(known.url, "http://10.0.0.5:8177");
        assert_eq!(known.id, None, "a typed address has not said what is at it");
    }
}
