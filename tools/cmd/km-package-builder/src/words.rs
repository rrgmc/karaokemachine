//! What this tool's pages say.
//!
//! The catalogs are `i18n/` beside the templates, `include_str!`-ed here, which is
//! `Catalogs live beside the words they translate` — the machinery is `km-locale`'s and the words
//! are this crate's. A missing key draws `⟦the-key⟧` rather than nothing, and the tests below are
//! what stop one reaching a page.
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

use std::sync::OnceLock;

use km_locale::{Catalog, Locale};

/// The keys the Rust reaches through something a scanner cannot see.
///
/// **Nearly every lookup here is a literal and is scanned** — `rust_keys` below reads `msg("…")` and
/// `msg_with("…"` out of the sources that make them, which is the arrangement `km-admin-pages` uses
/// and the one that does not fall behind. What is left for this list is the other shape: a key
/// returned by a `match` arm, where the literal is a `&'static str` rather than an argument to a
/// lookup.
///
/// `every_key_the_rust_asks_for_exists` and `no_message_is_left_unused` check it against the catalog
/// in both directions, which is the only thing that reads it.
#[cfg(test)]
const COMPOSED: &[&str] = &[
    // The header's counts strip. Four plurals over four numbers, spent through one closure in
    // `Chrome::new` — so the `msg_with` call carries a variable where the scanner wants a literal,
    // and four near-identical calls written out to satisfy it would be the worse code.
    "header-songs",
    "header-files",
    "header-failed",
    "header-favorites",
    // The five confirmations, whose subject and button are both plurals over one count and are
    // worded together by `handlers::confirm_words`. Which button a press gets is a branch, so the
    // key arrives as an argument and the scanner sees a variable.
    "confirm-songs",
    "confirm-files",
    "confirm-set",
    "confirm-tag",
    "confirm-untag",
    "confirm-file",
    "confirm-unfile",
    "confirm-reread",
    // ...and the pagers' labels, all worded by `views::say_page`, where the browse list's two depend
    // on whether a scan is writing rows underneath. See `SongRows::say_range`.
    "songs-range",
    "songs-range-scanning",
    "hits-range",
    "folders-range",
    // What a row's favorites button offers, which is four sentences and one of them per press. The
    // branch is in `SongRow::say`, so the key reaches `msg_with` as a variable.
    "row-favorites-close",
    "row-favorites-filed",
    "row-favorites-working",
    "row-favorites-none",
    // ...and the two the chooser's own buttons offer, chosen per favorite in `PickerFavorite::list`.
    "row-take-out-of",
    "row-put-in",
    // Four sentences a bulk action answers with, where which one is a branch on the direction it
    // went — so the key reaches `msg_with` as a variable.
    "said-tag-set",
    "said-tag-removed",
    "said-filed",
    "said-took-out",
    "said-now-a-working-list",
    "said-now-a-filing",
    // The three lyric-granularity chips, whose key is chosen by a `match` on what the bar sent.
    "chip-lyrics-per-syllable",
    "chip-lyrics-per-line",
    "chip-no-lyrics",
    // What a build spent its time on, said through one closure in `build_detail`.
    "said-build-re-encoded",
    "said-build-copied",
    "said-build-cdg-pairs",
    "said-build-ultrastar",
    // Which of the two the Debugging button answers with is a branch on what it just set.
    "said-debugging-on",
    "said-debugging-off",
    // The three counts a sync confirmation shows, worded through one closure in `sync_package` for
    // the reason the header's four are: three near-identical calls to satisfy the scanner would be
    // the worse code.
    "sync-adding",
    "sync-removing",
    "sync-keeping",
    "sync-starts-volumes",
];

/// The files whose `msg` calls the key scanner reads.
///
/// A list rather than a directory walk, because `include_str!` takes a literal — and naming them is
/// the point at which somebody adding a catalog lookup to a new file is told to add the file.
#[cfg(test)]
const SOURCES: &[&str] = &[
    include_str!("handlers.rs"),
    include_str!("views.rs"),
    include_str!("server.rs"),
    include_str!("app.rs"),
    include_str!("db.rs"),
    include_str!("db/filter.rs"),
    include_str!("model.rs"),
    include_str!("lib.rs"),
    include_str!("browse.rs"),
    include_str!("scan.rs"),
    include_str!("build.rs"),
];

/// This tool's words, one catalog per locale.
const CATALOGS: &[(Locale, &str)] = &[
    (Locale::English, include_str!("../i18n/en.ftl")),
    (
        Locale::BrazilianPortuguese,
        include_str!("../i18n/pt-BR.ftl"),
    ),
];

/// These pages' messages for one locale, parsed once.
#[must_use]
pub fn messages(locale: Locale) -> &'static Catalog {
    static PARSED: OnceLock<Vec<(Locale, Catalog)>> = OnceLock::new();
    let parsed = PARSED.get_or_init(|| {
        CATALOGS
            .iter()
            .map(|(locale, source)| {
                let catalog = Catalog::new(*locale, source).unwrap_or_else(|errors| {
                    panic!("{locale} km-package-builder catalog: {}", errors.join("; "))
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
    /// through untouched, so a rename that misses one compiles and renders `⟦nav-songs⟧`.
    #[test]
    fn every_key_in_the_markup_is_in_the_catalog() {
        let english = messages(Locale::English);
        let keys = markup_keys();
        assert!(!keys.is_empty(), "the scanner found nothing; it is broken");
        for key in keys {
            assert!(
                english.keys().contains(&key),
                "a template asks for `{key}`, which no catalog has"
            );
        }
    }

    /// A key the markup asks for takes no variables.
    ///
    /// `|t` passes none, so a message with a `{ $name }` in it draws that placeholder as `{$name}`
    /// on the page. Such a message belongs to the Rust, which fills it through `msg_with`.
    #[test]
    fn no_key_in_the_markup_wants_a_variable() {
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in markup_keys() {
                let text = catalog.msg(&key);
                assert!(
                    !text.contains("{$"),
                    "a template asks for `{key}` through `|t`, which passes no variables, \
                     so {locale} draws `{text}`"
                );
            }
        }
    }

    /// Every key this tool's Rust looks up.
    #[test]
    fn every_key_the_rust_asks_for_exists() {
        let english = messages(Locale::English);
        let keys = rust_keys();
        assert!(
            keys.len() > 8,
            "the scanner found only {} keys; it is broken",
            keys.len()
        );
        for key in keys {
            assert!(
                english.keys().contains(&key),
                "the Rust asks for `{key}`, which no catalog has"
            );
        }
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

    /// Every key the Rust looks up, scanned plus the handful that cannot be.
    ///
    /// **The whitespace between `(` and the key is skipped**, which `km-admin-pages`' copy of this
    /// learned the hard way: `cargo fmt` puts a newline and a column of spaces there the moment a
    /// call grows past the line width, and a scanner that did not trim then reported a key as unused
    /// while a handler was asking for it.
    fn rust_keys() -> Vec<String> {
        let mut keys: Vec<String> = COMPOSED
            .iter()
            // The scan statuses and the scan's phases, which reach a page as a code rather than as
            // a sentence. Each keeps its own list beside the code that returns it, so a new one is a
            // failing test here rather than a bracketed key on the Scan page.
            .chain(crate::model::ScanStatus::KEYS)
            .chain(crate::scan::phase::ALL)
            .chain(crate::db::OPENING_LADDER)
            .chain(crate::model::SongKind::KEYS)
            .chain(crate::handlers::ABSTENTION_KEYS)
            .map(|key| (*key).to_owned())
            .collect();
        for source in SOURCES {
            for call in ["msg(", "msg_with("] {
                for (index, _) in source.match_indices(call) {
                    let rest = source[index + call.len()..].trim_start();
                    let Some(quoted) = rest.strip_prefix('"') else {
                        continue;
                    };
                    if let Some(end) = quoted.find('"') {
                        keys.push(quoted[..end].to_owned());
                    }
                }
            }
        }
        keys
    }

    /// No template prints a word of its own.
    ///
    /// **The completeness check**, and the only one there is: the other tests say the catalogs agree
    /// with each other and with what is asked for, and none of them can see a sentence that was
    /// never keyed at all. This reads every template with the markup taken out and fails on whatever
    /// alphabetic text is left.
    ///
    /// **Attributes are its blind spot**, and the templates here carry eighty-odd `title`,
    /// `placeholder`, `aria-label` and `hx-confirm` values a reader sees. Those are covered by
    /// `every_key_in_the_markup_is_in_the_catalog` once they are keys and by nothing at all before,
    /// so a new one is a thing to look at rather than a thing to be told about.
    #[test]
    fn no_template_carries_its_own_prose() {
        let mut found: Vec<String> = Vec::new();
        let mut matched = vec![false; ALLOWED.len()];
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates"))
            .expect("the templates directory")
        {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_none_or(|ext| ext != "html") {
                continue;
            }
            let markup = std::fs::read_to_string(&path).expect("read the template");
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            for line in visible_text(&markup).lines() {
                let line = line.trim();
                if !line.chars().any(char::is_alphabetic) {
                    continue;
                }
                if let Some(at) = ALLOWED.iter().position(|allowed| *allowed == line) {
                    matched[at] = true;
                    continue;
                }
                found.push(format!("{name}: {line}"));
            }
        }
        assert!(
            found.is_empty(),
            "these templates print words of their own, which no catalog can translate:\n  {}",
            found.join("\n  ")
        );
        // ...and the list of exceptions is not a place to leave things. One that stops matching is
        // one whose markup has moved on, and a list nobody prunes is a hole nobody can see.
        for (allowed, hit) in ALLOWED.iter().zip(matched) {
            assert!(hit, "`{allowed}` is allowed in markup and appears in none");
        }
    }

    /// What a reader sees, with everything a machine reads taken out.
    ///
    /// **Comments come out first**, because an askama comment is where every other shape is
    /// legitimately quoted — the templates here carry paragraphs of reasoning holding `{{`, `{%` and
    /// whole tags, and a scanner that took the expressions out first would leave their prose behind
    /// and report every one of them.
    ///
    /// **A `.mono` span comes out too, and so does an inline `<script>`.** That class means *a word
    /// a machine reads* in this stylesheet — a settings key, a path, a command line — and a script
    /// is code; neither is prose a translator should be handed.
    fn visible_text(markup: &str) -> String {
        let mut text = markup.to_owned();
        for (open, close) in [
            ("{#", "#}"),
            ("<!--", "-->"),
            // An inline script is code, not prose. `open_progress.html` carries two, which is that
            // file's own idiom for the two endings that redirect rather than swap.
            ("<script>", "</script>"),
            ("<span class=\"mono\">", "</span>"),
            ("{{", "}}"),
            ("{%", "%}"),
        ] {
            text = cut_between(&text, open, close);
        }
        // Tags, attributes and all. And an entity is punctuation somebody typed as a word:
        // `&middot;` is a separator, not a sentence.
        text = cut_between(&text, "<", ">");
        text = cut_between(&text, "&", ";");
        text
    }

    /// Everything outside `open`…`close`, with what was between them dropped.
    ///
    /// A missing `close` drops the rest of the file: an unterminated comment or tag is markup this
    /// scanner cannot read, and reading it as prose would report the whole template.
    fn cut_between(text: &str, open: &str, close: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(open) {
            out.push_str(&rest[..start]);
            // The newlines inside are kept, so a report names the line a word is on rather than
            // gluing every stripped span's neighbours into one.
            let inside = &rest[start + open.len()..];
            let Some(end) = inside.find(close) else {
                out.extend(rest[start..].chars().filter(|c| *c == '\n'));
                return out;
            };
            out.extend(inside[..end].chars().filter(|c| *c == '\n'));
            rest = &inside[end + close.len()..];
        }
        out.push_str(rest);
        out
    }

    /// The exact lines a template may print without a key.
    ///
    /// Each is a name rather than a sentence — a file format or a product — and a translator handed
    /// one could only make it wrong. Every entry is checked to still appear, so this cannot become
    /// somewhere to put a sentence nobody wanted to key.
    const ALLOWED: &[&str] = &[
        // A character set and two units, each the same in every language.
        "UTF-8", "fps", "Hz",
    ];

    /// The two sentences `static/ui.js` fills in keep the markers it fills.
    ///
    /// **The one test with nothing behind it.** No server test renders that file and no page test
    /// fetches it, so a translator who writes `{o que}` for `{what}` produces `undefined answered
    /// 500` on a Portuguese desktop and every other check passes. Asked of both locales for that
    /// reason.
    #[test]
    fn the_script_patterns_keep_the_markers_the_browser_fills() {
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            let answered = catalog.msg_with(
                "js-answered",
                &[("what", "{what}".into()), ("status", "{status}".into())],
            );
            assert!(
                answered.contains("{what}") && answered.contains("{status}"),
                "{locale}'s `js-answered` lost a marker: {answered}"
            );
            for key in ["js-timed-out", "js-swap-failed"] {
                let said = catalog.msg_with(key, &[("what", "{what}".into())]);
                assert!(
                    said.contains("{what}"),
                    "{locale}'s `{key}` lost its marker: {said}"
                );
            }
        }
    }

    /// Zero takes the plural, in every language that has one.
    ///
    /// **CLDR files 0 under `one` for Portuguese**, which is right for `0,5 dia` and wrong for a
    /// plain count: `0 favorita` is not what anybody writes. Fluent matches an exact number before a
    /// plural category, so each of those messages carries a `[0]` variant — and this is what says a
    /// new one has to as well.
    #[test]
    fn zero_reads_as_a_plural() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in english.keys() {
                let none = catalog.msg_with(key, &[("count", 0.into())]);
                let one = catalog.msg_with(key, &[("count", 1.into())]);
                let many = catalog.msg_with(key, &[("count", 7.into())]);
                // Only a message that actually branches on `count` is being asked about; the rest
                // render the same three ways and say nothing either way.
                if one == many {
                    continue;
                }
                assert_eq!(
                    none.replace('0', "7"),
                    many,
                    "{locale}'s `{key}` words zero like one rather than like many"
                );
            }
        }
    }

    /// A count reads as a count, and each language chooses its own plural.
    #[test]
    fn the_header_counts_agree_with_the_number_beside_them() {
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in [
                "header-songs",
                "header-files",
                "header-failed",
                "header-favorites",
            ] {
                let one = catalog.msg_with(key, &[("count", 1.into())]);
                let many = catalog.msg_with(key, &[("count", 12489.into())]);
                assert!(
                    one.contains('1'),
                    "{locale}'s `{key}` lost its count: {one}"
                );
                assert!(
                    many.contains("12489"),
                    "{locale}'s `{key}` lost its count: {many}"
                );
            }
            // ...and one of them is a word that actually changes, so a catalog wired to the wrong
            // selector is caught rather than assumed.
            let one = catalog.msg_with("header-songs", &[("count", 1.into())]);
            let many = catalog.msg_with("header-songs", &[("count", 2.into())]);
            assert_ne!(one, many, "{locale} words one song and two the same way");
        }
    }

    /// Every `"key"|t` in this tool's templates.
    ///
    /// Crude on purpose, and the same scanner the other two catalogs use: a quoted run of
    /// `[a-z0-9-]` followed immediately by `|t }}` is a key and anything else is not. It therefore
    /// finds a key inside an attribute as well as one in text, and cannot mistake a class name for a
    /// message.
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
}
