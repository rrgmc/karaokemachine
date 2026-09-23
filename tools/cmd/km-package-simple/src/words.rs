//! What this tool's pages say.
//!
//! The catalogs are `i18n/` beside the templates, `include_str!`-ed here. The machinery is
//! `km-locale`'s and the words are this crate's. A missing key draws `⟦the-key⟧` rather than
//! nothing, and the tests below are what stop one reaching a page.
//!
//! **Data is not keyed**: a folder, a file name, a song's title, and the English language names
//! `km_kmpkg::Language` carries for its pickers. Nor is `--help`, which clap builds from doc
//! comments, and nor are the tray's words, which `km-tray` owns.

use std::sync::OnceLock;

use km_locale::{Catalog, Locale};

/// The files whose `msg` calls the key scanner reads.
#[cfg(test)]
const SOURCES: &[&str] = &[
    include_str!("server.rs"),
    include_str!("views.rs"),
    include_str!("app.rs"),
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
                    panic!("{locale} km-package-simple catalog: {}", errors.join("; "))
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
    fn every_message_is_translated_and_none_is_invented() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            let missing = catalog.missing_from(english);
            assert!(missing.is_empty(), "{locale} is missing {missing:?}");
            let extra = english.missing_from(catalog);
            assert!(
                extra.is_empty(),
                "{locale} has {extra:?}, which English does not"
            );
        }
    }

    #[test]
    fn every_key_in_the_markup_is_in_the_catalog_and_takes_no_variable() {
        let keys = markup_keys();
        assert!(keys.len() > 20, "the scanner found only {}", keys.len());
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in &keys {
                assert!(catalog.keys().contains(key), "a template asks for `{key}`");
                let text = catalog.msg(key);
                assert!(
                    !text.contains("{$"),
                    "`{key}` wants a variable, which `|t` cannot pass: {text}"
                );
            }
        }
    }

    #[test]
    fn every_key_the_rust_asks_for_exists() {
        let english = messages(Locale::English);
        let keys = rust_keys();
        assert!(keys.len() > 10, "the scanner found only {}", keys.len());
        for key in keys {
            assert!(english.keys().contains(&key), "the Rust asks for `{key}`");
        }
    }

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

    /// Zero takes the plural, in every language that has one. See `km-package-builder`'s twin.
    #[test]
    fn zero_reads_as_a_plural() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let catalog = messages(*locale);
            for key in english.keys() {
                let args = |count: i64| {
                    [
                        ("count", count.into()),
                        ("kept", count.into()),
                        ("volumes", count.into()),
                    ]
                };
                let none = catalog.msg_with(key, &args(0));
                let one = catalog.msg_with(key, &args(1));
                let many = catalog.msg_with(key, &args(7));
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

    /// No template prints a word of its own: what is left with the markup taken out is not prose.
    ///
    /// The completeness check `km-package-builder` carries, by the same scanner. Attributes are
    /// its blind spot there too.
    #[test]
    fn no_template_carries_its_own_prose() {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates"))
            .expect("the templates directory")
        {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_none_or(|ext| ext != "html") {
                continue;
            }
            let markup = std::fs::read_to_string(&path).expect("read the template");
            let mut text = markup.clone();
            for (open, close) in [
                ("{#", "#}"),
                ("<!--", "-->"),
                ("<script>", "</script>"),
                ("{{", "}}"),
                ("{%", "%}"),
                ("<", ">"),
                ("&", ";"),
            ] {
                text = cut_between(&text, open, close);
            }
            for line in text.lines() {
                let line = line.trim();
                if line.chars().any(char::is_alphabetic) {
                    found.push(format!("{}: {line}", path.display()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "templates print words of their own:\n  {}",
            found.join("\n  ")
        );
    }

    /// Everything outside `open`…`close`, with what was between them dropped.
    fn cut_between(text: &str, open: &str, close: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(open) {
            out.push_str(&rest[..start]);
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

    /// Every key the Rust looks up: the literal `msg("…")` calls, and the keys a `match` returns.
    fn rust_keys() -> Vec<String> {
        let mut keys: Vec<String> = crate::session::Kind::KEYS
            .iter()
            .chain(crate::session::Why::KEYS)
            .chain(crate::server::SAID_KEYS)
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

    /// Every `"key"|t` in the templates, by the scanner `km-package-builder` uses.
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
                if after.starts_with("|t }}") {
                    keys.push(candidate.to_owned());
                }
            }
        }
        keys
    }
}
