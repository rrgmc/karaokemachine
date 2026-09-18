//! The machine passwords this computer has been told to remember.
//!
//! **The store itself is `km-passwords`**, shared with `km-package-builder`: two tools that send
//! things to a machine meet the same closed door and hold the same kind of credential. What stays
//! here is the part that is this program's rather than the format's, which is **where the file
//! goes**.
//!
//! # In this program's own data folder, beside the provider keys
//!
//! [`crate::keys`] is the policy this follows, and its words are the reason: *"in memory by default,
//! and in this program's own data folder only if asked"*. A password belongs where a provider key
//! already lives, under the same `--data-dir` and with the same *forgetting deletes the file*
//! promise. `The password is remembered per machine, or not at all` in
//! `docs/decisions/curation.md` is the decision, and `Where a key somebody typed into a page lives`
//! in `docs/decisions/repository.md` is the policy behind both.
//!
//! **`km-package-builder` keeps its own in a per-user config directory under an environment
//! variable, and that is not a divergence.** That program has no `--data-dir`: an environment
//! variable is the only way to point a run somewhere else. This one is given a directory on the
//! command line already, so naming the same thing twice would be a second way to say it.
//!
//! **The data folder is what makes a test and a smoke run safe**, and it is the reason this is not a
//! process-global path behind a `cfg!(test)`. `cfg(test)` is per crate and an integration test links
//! the library compiled *without* it, so a guard written that way is off in exactly the tests that
//! drive the whole program -- which is how a suite came to create a config directory in the owner's
//! profile. A path that arrives as an argument cannot do that: a test passes a scratch directory and
//! `--data-dir` covers every run somebody starts by hand.
//!
//! # A token is still never written down
//!
//! [`crate::machine`]'s header says a bearer token is *"a credential this program was not asked to
//! keep"*, and that stands: what is remembered is the **password**, and it is exchanged for a fresh
//! token whenever one is wanted. A token expires and the machine forgets its own on restart, where a
//! standing instruction to log in does neither.

use std::path::Path;

pub use km_passwords::{Remembered, owner_only};

/// Reads what this computer remembers for the machine this run is pointed at.
///
/// Beside `provider-keys.json` and `settings.json` in the same folder, and its own file for
/// [`crate::keys`]' reason: deleting it is a whole answer to *forget my password*, and two files
/// cannot become one decision by accident.
#[must_use]
pub fn load(data_dir: &Path) -> Remembered {
    Remembered::at(Some(data_dir.join(km_passwords::FILE_NAME)))
}
