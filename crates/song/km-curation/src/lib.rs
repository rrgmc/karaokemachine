//! The package builder's curation database, its corpus scan and its package writer.
//!
//! **This is `km-package-builder` without its pages.** The tool's server, its handlers, its
//! templates and its two binaries live in `tools/cmd/km-package-builder`, and everything they show
//! comes from here. Nothing in this crate renders markup or answers a request.
//!
//! **It is a crate because the compiler type-checks a crate as one unit.** An edit to a handler or a
//! template then type-checks the pages and not this. See `The curation tool is two crates` in
//! `docs/decisions/repository.md`.
//!
//! The words the pages say are here too, in `i18n/`, because the database, the scan and the package
//! writer word messages of their own. The parity tests that hold the catalog to the pages live in
//! the tool, which is the one crate that can see both.

pub mod backup;
pub mod build;
pub mod casing;
pub mod chosen;
pub mod db;
pub mod dupes;
pub mod fixes;
pub mod form;
pub mod hint;
pub mod lyric_likeness;
pub mod model;
pub mod names;
pub mod passwords;
pub mod recent;
pub mod scan;
pub mod settings;
pub mod similar;
pub mod step;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod version;
pub mod words;
pub mod workspace;
