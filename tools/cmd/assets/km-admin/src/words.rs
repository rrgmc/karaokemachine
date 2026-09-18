//! What this program's own pages say.
//!
//! # Why there are two catalogs on one page
//!
//! Its pages are half somebody else's. *This machine*, Songs, Pictures and Sound are
//! `km-admin-pages`' markup and draw from that crate's catalog; the front door, the picture
//! searching and the bank fetching are this program's and draw from this one. Both halves obey
//! `Catalogs live beside the words they translate`, and keys for these pages kept in that crate
//! would sit beside markup that never spends them — which its own `no_message_is_left_unused`
//! refuses.
//!
//! One locale, read from the request by [`locale`], feeds both renders, so the two halves cannot
//! disagree about what the reader speaks.
//!
//! # What is not keyed
//!
//! **Data.** A bank's *what it is like* note and its license come out of `km-banks`, a provider's
//! terms and a photograph's credit from the provider, a rejection code from `km-wallpaper-pack`.
//! Those are values, like a song's title on the singer's remote.
//!
//! **`--help`.** `What a user reads is written in plain application language` governs a command
//! line, and clap builds one out of doc comments on a `Cli` struct. A page's catalog does not reach
//! it.
//!
//! **A job's outcome sentence.** An outcome is composed while a job runs, detached from any request,
//! so no locale is in reach. A *phase* escapes that by travelling as a key and being worded at
//! render time; an outcome would need the same shape to follow. See [`crate::job::phase`].

use std::sync::OnceLock;

use km_locale::{Catalog, Locale};

/// The keys composed in Rust rather than spent by a `|t` in markup.
///
/// Each interpolates something. `km_locale::filters` keeps markup to a key and nothing else: a name,
/// a count or a total goes in through [`Catalog::msg_with`] where a test can reach it, because *"a
/// plural is arithmetic"*. `km-admin-pages` composes `bank-remove` and `package-remove` the same way.
///
/// A list rather than a scan, because these calls sit in two files and a scanner would need an
/// include per file. `every_key_the_rust_asks_for_exists` and `no_message_is_left_unused` check the
/// list against the catalog in both directions.
pub const COMPOSED: &[&str] = &[
    "bank-remove-confirm",
    "pack-remove-confirm",
    "review-verdict",
    "picture-alt",
    // What the door says it already holds. Composed because it names the machine, and paired with
    // the sentence that names none for a machine whose owner never named it.
    "door-saved-for",
    "door-saved-here",
    // The front door's three answers. These interpolate nothing and are here for the other half of
    // this list's job: a sentence spent from Rust rather than from markup is a sentence the template
    // scanner cannot see, and one it cannot see is one `no_message_is_left_unused` would delete.
    "door-needs-an-address",
    "door-no-machine",
    "door-password-refused",
    "door-needs-a-password",
    "door-unreachable",
    // ...and its fourth, which is the one confirmation among them: what the language picker says
    // once it has taken. Worded in the language just chosen rather than in the one the request
    // arrived in, which is why it is spent from Rust at all.
    "door-locale-changed",
    // ...and what a send refused for want of one says when it puts somebody there.
    "send-needs-password",
    "send-needs-machine",
    // Every other control that refuses. Each lands as a banner on the page it was pressed on, so
    // each is worded where a locale is in reach rather than where the fault is found.
    "bank-unknown",
    "bank-not-here",
    "bank-remove-failed",
    "pack-unknown",
    "pack-remove-failed",
    "keys-not-remembered",
    "keys-not-forgotten",
    "job-busy-search",
    "job-busy-fetch",
    "search-needs-terms",
    "search-needs-key",
    "pack-name-needed",
];

/// This program's own words, one catalog per locale.
const CATALOGS: &[(Locale, &str)] = &[
    (Locale::English, include_str!("../i18n/en.ftl")),
    (
        Locale::BrazilianPortuguese,
        include_str!("../i18n/pt-BR.ftl"),
    ),
];

/// What language to draw a page in, given what this request said.
///
/// Delegated rather than spelled again. The rule — the cookie the singer's remote writes, then
/// `Accept-Language` — lives in `km-locale` and is read for these pages by
/// `km_admin_pages::words::locale`. Two spellings of it is how the two halves of a page would come
/// to disagree.
#[must_use]
pub fn locale(headers: &axum::http::HeaderMap) -> Locale {
    km_admin_pages::words::locale(headers)
}

/// These pages' messages for one locale, parsed once.
#[must_use]
pub fn messages(locale: Locale) -> &'static Catalog {
    static PARSED: OnceLock<Vec<(Locale, Catalog)>> = OnceLock::new();
    let parsed = PARSED.get_or_init(|| {
        CATALOGS
            .iter()
            .map(|(locale, source)| {
                let catalog = Catalog::new(*locale, source).unwrap_or_else(|errors| {
                    panic!("{locale} km-admin catalog: {}", errors.join("; "))
                });
                (*locale, catalog)
            })
            .collect()
    });
    parsed
        .iter()
        .find(|(candidate, _)| *candidate == locale)
        .map(|(_, catalog)| catalog)
        .expect("every locale has a catalog")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_parses() {
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            assert!(
                !catalog.keys().is_empty(),
                "{locale} parsed to nothing at all"
            );
        }
    }

    /// Every message is translated, so no page falls back to a bracketed key.
    ///
    /// English is the reference, because it is where a key is added: a translator's file falling
    /// behind is the ordinary case and is what this catches.
    #[test]
    fn every_message_is_translated() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let missing = messages(*locale).missing_from(english);
            assert!(
                missing.is_empty(),
                "{locale} is missing {missing:?}, so those pages would draw a key"
            );
        }
    }

    /// ...and no locale carries a key English does not, which nothing asks for.
    #[test]
    fn no_locale_invents_a_message_english_does_not_have() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let extra = english.missing_from(messages(*locale));
            assert!(
                extra.is_empty(),
                "{locale} has {extra:?}, which English does not and nothing asks for"
            );
        }
    }

    /// Every key the markup asks for exists.
    ///
    /// The half a constant cannot reach: a key in a template is a string literal askama passes
    /// through untouched, so a rename that misses one compiles and renders `⟦banks-heading⟧`.
    #[test]
    fn every_key_in_the_markup_is_in_the_catalog() {
        let english = messages(Locale::English);
        let mut seen = 0usize;
        for key in markup_keys() {
            seen += 1;
            assert!(
                english.keys().contains(&key),
                "a template asks for `{key}`, which no catalog has"
            );
        }
        assert!(
            seen > 40,
            "the scanner found only {seen} keys; it is broken"
        );
    }

    /// Nothing sits in the catalog that no page asks for.
    #[test]
    fn no_message_is_left_unused() {
        let mut used = markup_keys();
        used.extend(rust_keys());
        for key in messages(Locale::English).keys() {
            assert!(
                used.contains(key),
                "`{key}` is in the catalog and nothing asks for it"
            );
        }
    }

    /// Every `"key"|t` in this program's templates.
    ///
    /// The scanner `km-admin-pages` uses, and crude on purpose: a quoted run of `[a-z0-9-]` followed
    /// immediately by `|t }}` is a key and anything else is not. It therefore finds a key inside an
    /// attribute as well as one in text, and cannot mistake a class name for a message.
    fn markup_keys() -> Vec<String> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
        let mut keys = Vec::new();
        for entry in std::fs::read_dir(dir).expect("the templates directory") {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_none_or(|ext| ext != "html") {
                continue;
            }
            let markup = std::fs::read_to_string(&path).expect("read the template");
            for (index, _) in markup.match_indices('"') {
                let rest = &markup[index + 1..];
                let Some(end) = rest.find('"') else { continue };
                let candidate = &rest[..end];
                if candidate.is_empty()
                    || !candidate
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    continue;
                }
                let after = rest[end + 1..].trim_start();
                if after.starts_with("|t }}") || after.starts_with("| t }}") {
                    keys.push(candidate.to_owned());
                }
            }
        }
        keys
    }

    /// Every key this program's Rust looks up.
    ///
    /// Two lists rather than a scan. The job phases are `const`s in [`crate::job::phase`] because a
    /// phase is written down where no request is in reach; the rest are composed with `msg_with`
    /// because they interpolate a name or a count. `km-admin-pages` scans its own `msg(` calls,
    /// which it can because they are all in one file.
    fn rust_keys() -> Vec<String> {
        crate::job::phase::ALL
            .iter()
            .chain(crate::words::COMPOSED)
            .map(|key| (*key).to_owned())
            .collect()
    }

    /// Every composed message really interpolates what its caller passes.
    ///
    /// **A key existing is not the same as its arguments matching.** `msg_with` takes names, the
    /// `.ftl` declares them as `{ $name }`, and nothing else here compares the two: a caller passing
    /// `bank` to a message written `{ $name }` renders the placeholder and drops the value. That
    /// reaches a page only where a bank is already downloaded or a pack already built, which no test
    /// arranges and neither locale's page shows on a fresh machine.
    ///
    /// Asked of **both** locales, because a translator retyping a placeholder is the likelier half.
    #[test]
    fn every_composed_message_puts_its_argument_in() {
        let cases: &[(&str, &[(&str, &str)])] = &[
            ("bank-remove-confirm", &[("bank", "GeneralUser-GS.sf2")]),
            ("pack-remove-confirm", &[("pack", "wallpapers-beaches.zip")]),
            ("picture-alt", &[("author", "A Photographer")]),
            (
                "bank-remove-failed",
                &[("bank", "GeneralUser-GS.sf2"), ("why", "access is denied")],
            ),
            (
                "pack-remove-failed",
                &[("pack", "wallpapers-beaches"), ("why", "access is denied")],
            ),
            ("keys-not-remembered", &[("why", "access is denied")]),
            ("keys-not-forgotten", &[("why", "access is denied")]),
            ("search-needs-key", &[("provider", "Pixabay")]),
            ("door-saved-for", &[("machine", "Living Room")]),
        ];
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for (key, args) in cases {
                let bound: Vec<_> = args
                    .iter()
                    .map(|(name, value)| (*name, (*value).into()))
                    .collect();
                let said = catalog.msg_with(key, &bound);
                for (name, value) in *args {
                    assert!(
                        said.contains(value),
                        "{locale}'s `{key}` dropped `{name}`: {said}"
                    );
                    assert!(
                        !said.contains(&format!("${name}")),
                        "{locale}'s `{key}` left the placeholder in: {said}"
                    );
                }
            }
        }
    }

    /// The verdict counts both numbers and agrees with itself about the plural.
    #[test]
    fn the_review_verdict_counts_and_pluralizes() {
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            let one = catalog.msg_with(
                "review-verdict",
                &[("chosen", 1.into()), ("looked_at", 1.into())],
            );
            let many = catalog.msg_with(
                "review-verdict",
                &[("chosen", 96.into()), ("looked_at", 240.into())],
            );
            assert!(one.contains('1'), "{locale} lost its counts: {one}");
            assert!(
                many.contains("96") && many.contains("240"),
                "{locale} lost a count: {many}"
            );
            assert_ne!(
                one, many,
                "{locale} words one picture and many the same way"
            );
        }
    }

    #[test]
    fn every_key_the_rust_asks_for_exists() {
        let english = messages(Locale::English);
        let keys = rust_keys();
        assert!(keys.len() > 8, "the list has only {} keys", keys.len());
        for key in keys {
            assert!(
                english.keys().contains(&key),
                "the Rust asks for `{key}`, which no catalog has"
            );
        }
    }
}
