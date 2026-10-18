//! What this tool's pages say.
//!
//! The catalogs are `i18n/` in this crate, `include_str!`-ed here. The machinery is `km-locale`'s
//! and the words are this tool's. A missing key draws `⟦the-key⟧` rather than nothing.
//!
//! **The tests that stop one reaching a page are in `km-package-builder`'s `words.rs`**, beside the
//! templates they read. See `Catalogs live beside the words they translate` in
//! `docs/decisions/repository.md`.
//!
//! # Which language a page is in
//!
//! **A stored setting, read from [`crate::settings::Settings`].** One curator at one desktop is the
//! machine's shape rather than the remote's: a television belongs to a room and a phone to a person,
//! and this tool belongs to whoever is curating. `Accept-Language` is what an unset setting means,
//! so a first run opens in the browser's language without anybody finding the picker; the picker on
//! the Settings page is what makes the choice changeable, and nothing is written to `settings.json`
//! until it is used. See `What language the package builder speaks` in `docs/decisions/curation.md`.
//!
//! # What is not keyed
//!
//! **Data.** A song's title and artist, a folder's path, a tag somebody typed, a package's id, and
//! the English names `km_kmpkg::Language` carries for its pickers — that crate's own header says it
//! holds an English name for the pickers that have to show it. Those are values, like a song title
//! on the singer's remote.
//!
//! **The product's name and its version.** `A product name and a number are the same in every
//! language, and a translator handed them can only make them wrong.`
//!
//! **`--help`.** `What a user reads is written in plain application language` governs a command
//! line, and clap builds one out of doc comments on a `Cli` struct. A page's catalog does not reach
//! it.
//!
//! **The tray and the menu bar.** `km-tray` words *Show window*, *Open in browser* and *Quit* for
//! three programs, and none of the three passes it a locale.

use km_locale::{Catalog, Catalogs, Locale};

/// This tool's words, one catalog per locale.
static CATALOGS: Catalogs = Catalogs::new(
    "km-package-builder",
    &[
        (Locale::English, include_str!("../i18n/en.ftl")),
        (
            Locale::BrazilianPortuguese,
            include_str!("../i18n/pt-BR.ftl"),
        ),
    ],
);

/// These pages' messages for one locale, parsed once.
#[must_use]
pub fn messages(locale: Locale) -> &'static Catalog {
    CATALOGS.get(locale)
}
