//! The bank a setup program was told to fetch, honored the first time the machine starts.
//!
//! A tick box in the Windows setup program and the macOS `.pkg` offers to download the recommended
//! bank — 261.9 MiB, which is eight times the bundled one and far too much to put inside a carrier.
//! Neither installer downloads it: they write this file, and the machine does the fetching on its
//! first start with [`crate::fetch`], which already verifies the digest and refuses a bank that will
//! not play.
//!
//! # Why a file beside `settings.json` and not a key inside it
//!
//! The same argument [`crate::soundfont`] makes for its own note, plus one this file has and that
//! one does not: **the installer writes this, and it has never run the machine.** A key inside
//! `settings.json` would mean an installer that either overwrites a settings file it did not create
//! — losing whatever an upgrade was standing on — or parses and merges JSON it does not own. A file
//! it can simply place asks nothing of it. It is also a *request* rather than a setting: it is
//! consumed and deleted, and nothing an owner sets is ever deleted by the machine.
//!
//! # What holds it inside `Nothing downloads`
//!
//! That decision's surviving half is that nothing is fetched unasked. Ticking the box is the asking;
//! this file is only how the answer reaches a program that was not running at the time. So:
//!
//! * **no file, no code path.** A machine whose owner left the box unticked never reaches
//!   [`crate::fetch`], which is the property that decision cares about most;
//! * **one bank, named**, out of the compiled table, from its pinned URL against its pinned digests;
//! * **[`MAX_ATTEMPTS`] starts and then it gives up**, so a machine with no network is not a machine
//!   that tries again for ever. That is the difference between honoring a request and a background
//!   refresh, which the same decision rules out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::banks::CatalogBank;
use crate::settings::Paths;

/// What the file is called, beside `settings.json` in the config directory.
const FILE: &str = "first-run-soundfont.json";

/// The bank name that means "whichever row the table recommends".
///
/// The installers write the concrete id, so that the bank somebody was offered is the bank that
/// arrives even if the recommendation moves between the release and the first start. This spelling
/// is for `--first-run-soundfont`, where naming the recommendation rather than a row is the useful
/// thing to be able to type.
pub const RECOMMENDED: &str = "recommended";

/// How many starts may try before the request is given up on.
///
/// **Three rather than one**, because the first start of an appliance is exactly when the network is
/// least likely to be up — a box carried to a television and switched on has often not been given a
/// cable or a password yet. **Three rather than for ever**, because a request that never expires is
/// the "background refresh" `Nothing downloads` refuses, dressed as a retry.
const MAX_ATTEMPTS: u32 = 3;

/// What a setup program asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// A bank id from the table, or [`RECOMMENDED`].
    pub bank: String,
    /// How many starts have tried so far.
    ///
    /// Defaulted, because the installers write only `bank` — they have no business knowing this
    /// exists, and a two-key file they would have to keep in step is a second thing to get wrong.
    #[serde(default)]
    pub attempts: u32,
}

impl Request {
    /// A fresh request naming one bank.
    pub fn new(bank: impl Into<String>) -> Self {
        Self {
            bank: bank.into(),
            attempts: 0,
        }
    }

    /// The row this names, or `None` for a bank the table does not know.
    ///
    /// A table that no longer holds the row an old installer wrote is not an error worth keeping the
    /// request for: nothing later will make it resolve.
    pub fn resolve(&self) -> Option<&'static CatalogBank> {
        let catalog = crate::banks::catalog();
        if self.bank == RECOMMENDED {
            return catalog.iter().find(|row| row.recommended);
        }
        catalog.iter().find(|row| row.id == self.bank)
    }

    /// This request with one more start counted against it, or `None` once it has had its three.
    pub fn attempted(&self) -> Option<Self> {
        let attempts = self.attempts.saturating_add(1);
        (attempts <= MAX_ATTEMPTS).then(|| Self {
            bank: self.bank.clone(),
            attempts,
        })
    }
}

/// Where the request lives.
pub fn file(paths: &Paths) -> PathBuf {
    paths.config_dir.join(FILE)
}

/// The request as it stands, or `None` for one that is absent or unreadable.
///
/// **Reads and nothing else.** `--show-paths` calls this, and that command's whole discipline is
/// that saying where things are must not change anything — see `peek_settings` in [`crate::cli`].
/// Tidying an unreadable request away is [`discard_unreadable`], called from the one place that is
/// entitled to write.
pub fn read(paths: &Paths) -> Option<Request> {
    let text = std::fs::read_to_string(file(paths)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Removes a request that cannot be read, and says whether it removed one.
///
/// **A file that will not parse is removed rather than ignored**, which is the opposite of what
/// [`crate::soundfont::read`] does with its note and is right for the opposite reason: that one is a
/// note the machine left itself, where losing it costs one line of a report. This is an instruction
/// nobody can carry out, and leaving it would mean reading and failing to understand the same bytes
/// at every start for the life of the install.
pub fn discard_unreadable(paths: &Paths) -> bool {
    let path = file(paths);
    if !path.is_file() || read(paths).is_some() {
        return false;
    }
    tracing::warn!(
        path = %path.display(),
        "the first-start SoundFont request could not be read; removing it"
    );
    let _ = remove(paths);
    true
}

/// Writes the request, creating the config directory if this is a fresh install.
pub fn write(paths: &Paths, request: &Request) -> std::io::Result<()> {
    std::fs::create_dir_all(&paths.config_dir)?;
    let text = serde_json::to_string_pretty(request).map_err(std::io::Error::other)?;
    std::fs::write(file(paths), text)
}

/// Removes it. A request that was not there is not an error — every path here ends by clearing one.
pub fn remove(paths: &Paths) -> std::io::Result<()> {
    match std::fs::remove_file(file(paths)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// A sentence for the television about a first-start download.
///
/// The same three shapes [`crate::dropped::DropStatus`] has, and rendered by the display through the
/// same `Flash`. Not that type: a bank arriving because a box was ticked at install time and a
/// package arriving because somebody dragged it onto the window are different events, and the one
/// thing they share is how they are drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// A download is running. Stays on screen until its own result replaces it.
    Working(String),
    /// The bank is installed and chosen.
    Done(String),
    /// It did not work, in the downloader's own words.
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that removes itself. The same shape `settings::tests::Scratch` uses, and
    /// separate from it because a test module cannot reach another one's private helper.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-firstrun-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).expect("make the scratch directory");
            Self(dir)
        }

        fn paths(&self) -> Paths {
            Paths::rooted_at(&self.0)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn recommended_resolves_to_the_one_flagged_row() {
        let row = Request::new(RECOMMENDED)
            .resolve()
            .expect("the table recommends a bank");
        assert!(row.recommended, "the resolved row is the recommended one");
        assert!(
            row.url.is_some() && row.digest.is_some(),
            "and it is one this machine can actually fetch"
        );
    }

    #[test]
    fn a_concrete_id_resolves_to_its_own_row() {
        let row = Request::new("generaluser").resolve().expect("a known bank");
        assert_eq!(row.id, "generaluser");
    }

    #[test]
    fn a_bank_the_table_does_not_know_resolves_to_nothing() {
        assert!(Request::new("no-such-bank").resolve().is_none());
    }

    /// Three starts try, and the fourth is not offered one.
    #[test]
    fn attempts_run_out() {
        let mut request = Request::new(RECOMMENDED);
        for expected in 1..=MAX_ATTEMPTS {
            request = request.attempted().expect("a start is still allowed");
            assert_eq!(request.attempts, expected);
        }
        assert!(
            request.attempted().is_none(),
            "the request is spent after {MAX_ATTEMPTS} starts"
        );
    }

    /// The installers write `{"bank": "..."}` and nothing else, so that has to be a whole request.
    #[test]
    fn the_file_an_installer_writes_parses() {
        let request: Request = serde_json::from_str(r#"{"bank": "colombogmgs2"}"#)
            .expect("an installer's file parses");
        assert_eq!(request.bank, "colombogmgs2");
        assert_eq!(request.attempts, 0);
    }

    #[test]
    fn a_request_round_trips_through_the_file() {
        let scratch = Scratch::new("round-trip");
        let paths = scratch.paths();
        assert!(read(&paths).is_none(), "nothing is requested yet");

        let request = Request::new("colombogmgs2");
        write(&paths, &request).expect("the request is written");
        assert_eq!(read(&paths), Some(request));

        remove(&paths).expect("the request is removed");
        assert!(read(&paths).is_none());
        remove(&paths).expect("removing a request that is not there is not an error");
    }

    /// Bytes nobody can act on are deleted, not read again at every start for ever.
    #[test]
    fn a_file_that_will_not_parse_is_removed() {
        let scratch = Scratch::new("bad-json");
        let paths = scratch.paths();
        std::fs::create_dir_all(&paths.config_dir).expect("the config directory");
        std::fs::write(file(&paths), "not json").expect("the bad file is written");

        assert!(read(&paths).is_none(), "it cannot be read");
        assert!(discard_unreadable(&paths), "so it is thrown away");
        assert!(
            !file(&paths).exists(),
            "and it is gone rather than waiting to fail again"
        );
    }

    /// …and a request that reads perfectly well is left exactly where it is.
    #[test]
    fn a_readable_request_is_not_discarded() {
        let scratch = Scratch::new("good-json");
        let paths = scratch.paths();
        write(&paths, &Request::new(RECOMMENDED)).expect("the request is written");

        assert!(!discard_unreadable(&paths));
        assert!(file(&paths).exists());
    }

    /// Nothing to discard where nothing was requested — the case every ordinary start takes.
    #[test]
    fn no_request_is_not_an_unreadable_one() {
        let scratch = Scratch::new("absent");
        assert!(!discard_unreadable(&scratch.paths()));
    }
}
