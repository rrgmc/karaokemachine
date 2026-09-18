//! What the owner's pages say.
//!
//! **This crate's own catalog, beside this crate's own markup** — the same judgement the asset hash
//! already makes here: what these words belong to is *these* pages, and a rename that orphans a key
//! should be caught in one crate rather than found on somebody else's screen.
//!
//! There is no error vocabulary here, unlike `km-remote-pages`. These pages are served by the
//! machine itself, in its own process, so nothing arrives from across a wire needing a code to be
//! rendered from.

use std::sync::OnceLock;

use axum::http::HeaderMap;
use axum::http::header;
use km_locale::{Catalog, Locale};

/// What language to draw a page in, given what this request said.
///
/// **The same cookie the singer's remote writes**, because both are mounted on one origin and a
/// viewer who chose Portuguese at `/` should not meet English at `/admin/`. The rule itself lives in
/// `km-locale` so there is one spelling of it; this is the two lines of header reading that crate
/// deliberately does not do, having no `http` dependency.
#[must_use]
pub fn locale(headers: &HeaderMap) -> Locale {
    km_locale::choose(
        crate::handlers::cookie(headers, km_locale::COOKIE).as_deref(),
        headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok()),
    )
}

/// The owner's words, one catalog per locale.
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
                    panic!("{locale} admin catalog: {}", errors.join("; "))
                });
                (*locale, catalog)
            })
            .collect()
    });
    parsed
        .iter()
        .find(|(candidate, _)| *candidate == locale)
        .map(|(_, catalog)| catalog)
        .expect("every locale has an admin catalog")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_parses() {
        for locale in Locale::ALL {
            assert!(!messages(*locale).keys().is_empty(), "{locale}");
        }
    }

    #[test]
    fn every_message_is_translated() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let missing = messages(*locale).missing_from(english);
            assert!(
                missing.is_empty(),
                "{locale} has not caught up: {missing:?}"
            );
        }
    }

    #[test]
    fn no_locale_invents_a_message_english_does_not_have() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let extra = english.missing_from(messages(*locale));
            assert!(
                extra.is_empty(),
                "{locale} has keys nothing asks for: {extra:?}"
            );
        }
    }

    /// Every `|t` key in the markup is in the catalog.
    ///
    /// The half a constant cannot reach: a key in a template is a string literal askama passes
    /// through untouched, so a rename that misses one compiles and renders `⟦tab-songs⟧`.
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
            seen > 20,
            "the scanner found only {seen} keys; it is broken"
        );
    }

    #[test]
    fn no_message_is_left_unused() {
        // A key nothing looks up is a leftover from a rename, sitting there looking like work for
        // whoever translates next.
        let mut used = markup_keys();
        used.extend(rust_keys());
        // **The one set the scanner cannot see.** `AdminError::key` maps variants to keys with a
        // `match`, not with a `msg("…")` call, so `rust_keys` walks straight past all of them and
        // this test would report every error message as unused. `machine::ERROR_KEYS` is the list,
        // and `every_error_key_is_listed` over there is what stops the list falling behind the
        // `match` -- which is the failure this exemption would otherwise hide.
        used.extend(
            crate::machine::ERROR_KEYS
                .iter()
                .map(|key| (*key).to_owned()),
        );
        for key in messages(Locale::English).keys() {
            assert!(
                used.contains(key),
                "`{key}` is in the catalog and nothing asks for it"
            );
        }
    }

    /// Every key this crate's Rust looks up.
    ///
    /// **Scanned rather than listed**, unlike `km-remote-pages`' constants, because here they are
    /// all `msg("…")` and `msg_with("…", …)` literals in one file — so a scan sees exactly what a
    /// list would, and cannot fall behind it. `every_key_in_the_rust_exists` is the other half.
    fn rust_keys() -> Vec<String> {
        let source = include_str!("handlers.rs");
        let mut keys = Vec::new();
        // **The whitespace between `(` and the key is skipped**, because `cargo fmt` puts a newline
        // and twenty-eight spaces there the moment a call grows past the line width — which it did,
        // and this test then reported a key nothing asks for while the handler was asking for it.
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
        keys
    }

    #[test]
    fn every_key_in_the_rust_exists() {
        let english = messages(Locale::English);
        let keys = rust_keys();
        assert!(keys.len() > 8, "the scanner found only {} keys", keys.len());
        for key in keys {
            assert!(
                english.keys().contains(&key),
                "the handlers ask for `{key}`, which no catalog has"
            );
        }
    }

    /// Every `"key"|t` across this crate's templates.
    fn markup_keys() -> Vec<String> {
        let mut keys = Vec::new();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates"))
            .expect("the templates directory")
        {
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

    /// **No template shows a reader a word no catalog can reach.**
    ///
    /// The parity tests above compare *keys* — every key the markup asks for exists, every message
    /// in the catalog is asked for. None of them can see a sentence that was never a key at all,
    /// which is the shape this catches and the one that reached a screen: `A SoundFont decides what
    /// the instruments sound like` sat under a Portuguese heading, and `{{ interval }} seconds`
    /// wrote the unit beside a translated label. Both were literal text in a template, so both were
    /// English in every locale and no key-to-key check could tell.
    ///
    /// What is left after the comments, the expressions, the statements, the tags and the entities
    /// are taken out is what a person reads. It has to be nothing.
    ///
    /// **Attributes go with the tags, and that is a hole.** `aria-label` and `placeholder` are read
    /// out to somebody; stripping `<[^>]*>` takes them along with the markup. Every one of them in
    /// this crate is `{{ … }}` today, so the check would pass regardless — naming the gap is worth
    /// more here than a parser that finds it, because the day one is typed as a literal is the day
    /// somebody has to remember this paragraph rather than run a test.
    #[test]
    fn no_template_carries_its_own_prose() {
        let mut found: Vec<String> = Vec::new();
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
                if line.chars().any(char::is_alphabetic) {
                    found.push(format!("{name}: {}", line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "these templates print words of their own, which no catalog can translate:\n  {}",
            found.join("\n  ")
        );
    }

    /// What a reader sees, with everything a machine reads taken out.
    ///
    /// **Comments come out first**, because an askama comment is where every other shape is
    /// legitimately quoted — this crate's templates carry paragraphs of reasoning holding `{{`,
    /// `{%` and whole tags, and a scanner that took the expressions out first would leave their
    /// prose behind and report every one of them.
    fn visible_text(markup: &str) -> String {
        let mut text = markup.to_owned();
        for (open, close) in [("{#", "#}"), ("<!--", "-->"), ("{{", "}}"), ("{%", "%}")] {
            text = cut_between(&text, open, close);
        }
        // Tags, attributes and all. And an entity is punctuation somebody typed as a word:
        // `&hellip;` is an ellipsis, not a sentence.
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
}
