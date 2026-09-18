//! The machine passwords this computer has been told to remember.
//!
//! **The store itself is `km-passwords`**, shared with `km-admin`: both tools send things to a
//! machine, both meet the same closed door, and both hold a password. What stays here is the part
//! that is this program's rather than the format's, which is **where the file goes**.
//!
//! `Where a key somebody typed into a page lives` in `docs/decisions/repository.md` is the policy,
//! and `The password is remembered per machine, or not at all` in `docs/decisions/curation.md` is
//! the decision. The reasoning for the file's shape, the id it is keyed by and why forgetting
//! deletes is in that crate's own header.
//!
//! **Not in the `.kmbuild`**, which is the part worth being exact about here, because this is the
//! program with a curation database. A corpus is a *document* — see `A corpus is a document` in
//! `docs/decisions/curation.md`, and [`crate::chosen`] for why the machine's *address* is in there
//! deliberately. A document travels with the corpus: it goes on an external drive, it gets opened on
//! a second computer, it is the thing somebody would hand to a friend along with the songs. An
//! address in it is a convenience; a password in it is a credential in a file nobody thinks of as
//! holding one.

use std::path::PathBuf;

pub use km_passwords::{Remembered, owner_only};

/// The environment variable that says where this run's remembered passwords go.
///
/// [`crate::recent::ENV_VAR`]'s shape and its reasoning, and here it carries more weight: a smoke
/// test, a screenshot run or a scripted build that wrote into this file would be putting a
/// credential in the owner's own store. Set and empty is a run that remembers nothing.
pub const ENV_VAR: &str = "KM_PACKAGE_BUILDER_PASSWORDS";

/// The name this program registers with the platform, for its config directory.
const APP: &str = "km-package-builder";

/// Where the file is kept, or `None` if there is nowhere to keep it.
///
/// **`None` under `cfg(test)` first**, for [`crate::recent::path`]'s reason and one more: a suite
/// that wrote here would be writing a test password into the config directory of whoever is
/// building. **This guard has to be in this crate** rather than in `km-passwords`, because
/// `cfg(test)` is per crate: one written over there would be off during a build of this program's
/// own tests.
///
/// **What it does not cover is an integration test**, which links this library compiled *without*
/// `cfg(test)`. Nothing under `tests/` reaches a write today — the only writer is the sign-in
/// handler, and `tests/upload.rs` drives `Client::log_in` directly — so the guard holds for every
/// caller there is. It would stop holding the day an integration test posted that form, and the
/// failure is silent: a password in the config directory of whoever ran the suite. `km-admin` met
/// exactly that and answers it by taking the folder as an argument, which this program has no
/// `--data-dir` to do.
fn path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    km_passwords::file_for(std::env::var_os(ENV_VAR).as_deref(), APP)
}

/// Reads what this computer remembers, or nothing.
#[must_use]
pub fn load() -> Remembered {
    Remembered::at(path())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test run has nowhere to write, whatever the environment says.
    ///
    /// The property the `cfg!(test)` guard above exists for, asserted in the crate the guard is in.
    #[test]
    fn a_test_run_remembers_nothing_on_disk() {
        assert_eq!(path(), None);
        assert_eq!(load().get("abc123"), None);
    }
}
