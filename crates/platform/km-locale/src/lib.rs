//! Which language a surface speaks, and the messages it says in it.
//!
//! **The machinery is here and the words are not.** A [`Catalog`] is built by the crate whose words
//! they are, out of `.ftl` files that crate holds and `include_str!`s for itself — so a message and
//! the code that says it are never in two crates, and this one never grows a dependency on anything
//! it is translating. What is shared is the part five surfaces would otherwise answer five ways:
//! what a locale is, how a browser's `Accept-Language` becomes one of ours, and what happens when a
//! key is missing.
//!
//! **A missing key renders the key**, in markers, rather than an empty string — see [`Catalog::msg`].
//! A blank line on a television is a fault nobody can report; `⟦queue-full⟧` is a fault anybody can
//! read out over the phone. Tests are what stop it reaching a screen at all: see
//! [`Catalog::missing_from`].
//!
//! Fluent, rather than a table of format strings, because two of the things Portuguese needs are
//! exactly the two a `&'static str` per key cannot express — choosing a plural by the number beside
//! it, and agreeing an article with the noun it introduces. Both are data in a `.ftl` file here and
//! a branch at the call site otherwise.

#[cfg(feature = "askama")]
pub mod filters;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

// **The concurrent bundle, not the default one.** They differ only in their memoizer, and the
// default's is a `RefCell` — so `FluentBundle<FluentResource>` is not `Sync`, and a `Catalog` built
// once and shared by every request on a multi-threaded runtime would not compile. That is exactly
// how every caller here uses one. `every_catalog_crosses_a_thread` is the test that says so.
use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use fluent_syntax::ast::Entry;
use unic_langid::LanguageIdentifier;

/// A language a surface can speak.
///
/// Deliberately a small closed set rather than "any tag Fluent will parse". Every value here has a
/// catalog behind it in every crate that has catalogs at all, which is what makes
/// [`Catalog::missing_from`] able to say a build is complete — and what stops a settings file
/// naming `de` and getting a screen half in German.
///
/// **`Locale`, never `Language`.** `language` already means the language a *song* is sung in, all
/// the way through this product — `km_kmpkg::Language`, the API's `?language=`, the remote's picker,
/// the song book's sections. See `One spelling per concept, across every surface`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Locale {
    /// US English, and the language every message is written in first.
    English,
    /// Brazilian Portuguese.
    BrazilianPortuguese,
}

impl Locale {
    /// Every locale, in the order a picker shows them.
    ///
    /// English first because it is the source language and the fallback, not because of where it
    /// sorts.
    pub const ALL: &'static [Locale] = &[Locale::English, Locale::BrazilianPortuguese];

    /// The BCP 47 tag — `en`, `pt-BR`.
    ///
    /// This is what `settings.json` stores, what a cookie carries and what `<html lang>` says.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Locale::English => "en",
            Locale::BrazilianPortuguese => "pt-BR",
        }
    }

    /// What this language calls itself.
    ///
    /// **Its own name, not its English one.** A picker offering `Portuguese (Brazil)` is naming the
    /// language to somebody who by definition cannot read the label; `Português (Brasil)` is legible
    /// to exactly the person who would choose it. This is the one string in the product that is
    /// deliberately not in the reader's language, and it is why it lives in code rather than in a
    /// catalog — putting it in one would translate it, which is the mistake.
    #[must_use]
    pub fn endonym(self) -> &'static str {
        match self {
            Locale::English => "English",
            Locale::BrazilianPortuguese => "Português (Brasil)",
        }
    }

    /// Reads a tag back, or `None` for anything that is not one of ours.
    ///
    /// Case-insensitive, because a cookie and a settings file are both hand-editable and `pt-br` is
    /// the same request as `pt-BR`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Locale> {
        Locale::ALL
            .iter()
            .copied()
            .find(|locale| locale.tag().eq_ignore_ascii_case(value))
    }

    /// The nearest locale to a tag, falling back to the language alone.
    ///
    /// `pt-PT` and `pt` both reach [`Locale::BrazilianPortuguese`], because a Portuguese speaker is
    /// far better served by Brazilian Portuguese than by English. `en-GB` reaches
    /// [`Locale::English`] for the same reason. What this does *not* do is reach across languages:
    /// `de` gets `None` and the caller falls back.
    #[must_use]
    pub fn best_match(tag: &str) -> Option<Locale> {
        if let Some(exact) = Locale::parse(tag) {
            return Some(exact);
        }
        let wanted = tag.split(['-', '_']).next()?;
        Locale::ALL
            .iter()
            .copied()
            .find(|locale| locale.tag().split('-').next() == Some(wanted))
    }

    /// The language identifier Fluent wants.
    #[must_use]
    pub fn langid(self) -> LanguageIdentifier {
        self.tag()
            .parse()
            .expect("every tag here is a valid BCP 47")
    }
}

impl Default for Locale {
    /// English, which is the language every message is written in before it is translated.
    fn default() -> Self {
        Locale::English
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// Picks a locale from an `Accept-Language` header.
///
/// Quality values are honored, and an unparseable one is treated as `q=1` rather than as a reason to
/// discard the entry — a header this code cannot read is still a person asking for something.
/// `*` is ignored: it means "anything", and the caller's default already is the anything.
///
/// Returns `None` when nothing in the header is a language we have, which is the caller's cue to use
/// its own default rather than this function's idea of one.
///
/// ```
/// # use km_locale::{Locale, negotiate};
/// assert_eq!(negotiate("pt-BR,pt;q=0.9,en;q=0.8"), Some(Locale::BrazilianPortuguese));
/// assert_eq!(negotiate("de,en;q=0.5"), Some(Locale::English));
/// assert_eq!(negotiate("de"), None);
/// ```
#[must_use]
pub fn negotiate(header: &str) -> Option<Locale> {
    let mut best: Option<(f32, Locale)> = None;
    for entry in header.split(',') {
        let mut parts = entry.split(';');
        let tag = parts.next().unwrap_or_default().trim();
        if tag.is_empty() || tag == "*" {
            continue;
        }
        let quality = parts
            .find_map(|part| {
                part.trim()
                    .strip_prefix("q=")
                    .map(str::trim)
                    .map(str::to_owned)
            })
            .and_then(|q| q.parse::<f32>().ok())
            .unwrap_or(1.0);
        let Some(locale) = Locale::best_match(tag) else {
            continue;
        };
        // Strictly greater, so the first entry at a given quality wins — a header is written in
        // preference order and two entries at `q=1` are not a tie the parser gets to break.
        if best.is_none_or(|(best_q, _)| quality > best_q) {
            best = Some((quality, locale));
        }
    }
    best.map(|(_, locale)| locale)
}

/// The cookie a web surface remembers a chosen language in.
///
/// **Here rather than in either pages crate**, because both are mounted on one origin — the singer's
/// remote at `/` and the owner's pages at `/admin/` — and a cookie written under two spellings would
/// be a viewer who chose Portuguese on one page and got English on the other.
pub const COOKIE: &str = "km_locale";

/// How long a chosen language is remembered on a device.
///
/// A year. What somebody prefers to read in does not go stale, and a shorter life would mean a
/// phone that was put down for a season coming back in whatever language its browser was installed
/// with.
pub const COOKIE_MAX_AGE: u32 = 60 * 60 * 24 * 365;

/// The `Set-Cookie` value that remembers a chosen language on this device.
///
/// **Here for [`COOKIE`]'s reason, one step further.** Three surfaces write this cookie and the
/// name alone is not the whole spelling: a `Path` that was not `/` would leave `/admin/` reading a
/// cookie the remote could not see, and a `Max-Age` that differed would make one surface forget
/// what the other remembered.
///
/// `HttpOnly`, because nothing on any page has a reason to read this in script. `SameSite=Lax`
/// rather than `Strict`, so that following a link into the remote arrives in the right language.
/// **No `Secure`**: these pages are served over plain HTTP on a home LAN, and marking it would mean
/// a browser silently discarding every one.
///
/// No escaping, and none is owed: a tag is [`Locale::tag`]'s own `&'static str` — letters and one
/// hyphen — rather than anything a reader typed.
///
/// ```
/// # use km_locale::{Locale, set_cookie};
/// assert_eq!(
///     set_cookie(Locale::BrazilianPortuguese),
///     "km_locale=pt-BR; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax"
/// );
/// ```
#[must_use]
pub fn set_cookie(locale: Locale) -> String {
    format!(
        "{COOKIE}={}; Path=/; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Lax",
        locale.tag()
    )
}

/// What language to draw a page in, given what a request said.
///
/// **The cookie wins over the header.** The cookie is a choice somebody made on this device; the
/// header is what their browser happened to be installed with. Neither is an error when it names a
/// language this build does not have — a cookie is something anybody can edit — so both fall through
/// to English, which is what every message is written in first.
///
/// ```
/// # use km_locale::{Locale, choose};
/// assert_eq!(choose(Some("pt-BR"), Some("en")), Locale::BrazilianPortuguese);
/// assert_eq!(choose(None, Some("pt-PT,pt;q=0.9")), Locale::BrazilianPortuguese);
/// assert_eq!(choose(Some("nonsense"), Some("pt-BR")), Locale::BrazilianPortuguese);
/// assert_eq!(choose(None, None), Locale::English);
/// ```
#[must_use]
pub fn choose(cookie: Option<&str>, accept_language: Option<&str>) -> Locale {
    if let Some(chosen) = cookie.and_then(Locale::parse) {
        return chosen;
    }
    accept_language.and_then(negotiate).unwrap_or_default()
}

/// One locale's messages, ready to look up.
///
/// Built by the crate that owns the words. The usual shape is a `&'static` per locale, resolved once
/// and handed to whatever renders:
///
/// ```
/// # use km_locale::{Catalog, Locale};
/// let catalog = Catalog::new(Locale::English, "greeting = Hello\n").expect("the catalog parses");
/// assert_eq!(catalog.msg("greeting"), "Hello");
/// assert_eq!(catalog.msg("nope"), "⟦nope⟧");
/// ```
pub struct Catalog {
    locale: Locale,
    bundle: FluentBundle<FluentResource>,
    /// Every key, collected while the resource was still readable.
    ///
    /// `FluentBundle` can answer `has_message` for a key you already have but cannot list what it
    /// holds, and the resource is moved into it by `add_resource`. So the keys are taken on the way
    /// past — which is the only chance — and kept for [`Catalog::missing_from`], the test that stops
    /// an untranslated key reaching a screen.
    keys: BTreeSet<String>,
}

impl Catalog {
    /// Parses a `.ftl` source into a catalog.
    ///
    /// **Isolating marks are off.** Fluent wraps interpolated values in U+2068/U+2069 by default, to
    /// keep a right-to-left name from rearranging the sentence around it. Nothing here renders
    /// right-to-left — `Non-Latin text` defers every script that would — and the marks are not
    /// characters SDL3_ttf, a `<title>`, or the song book's cp1252 encoder have any answer for. They
    /// would arrive as blank boxes in exactly the places a song title goes.
    ///
    /// # Errors
    ///
    /// Returns every parse error Fluent reported, as sentences. A catalog is compiled into the
    /// binary, so this failing is a build fault rather than anything a user can cause — the tests in
    /// each owning crate are what turn it into one.
    pub fn new(locale: Locale, source: &str) -> Result<Catalog, Vec<String>> {
        let resource = FluentResource::try_new(source.to_owned())
            .map_err(|(_, errors)| errors.iter().map(ToString::to_string).collect::<Vec<_>>())?;
        let keys = resource
            .entries()
            // Messages only. A `Term` (`-brand = …`) is a building block referenced from inside
            // another message and is never looked up by name, so counting one as a key would make
            // every parity test demand a translation for something nothing can ask for.
            .filter_map(|entry| match entry {
                Entry::Message(message) => Some(message.id.name.to_owned()),
                _ => None,
            })
            .collect();
        let mut bundle = FluentBundle::new_concurrent(vec![locale.langid()]);
        bundle.set_use_isolating(false);
        bundle
            .add_resource(resource)
            .map_err(|errors| errors.iter().map(ToString::to_string).collect::<Vec<_>>())?;
        Ok(Catalog {
            locale,
            bundle,
            keys,
        })
    }

    /// Which locale this speaks.
    #[must_use]
    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// A message with no arguments.
    ///
    /// A key that is not here renders as `⟦the-key⟧` rather than as nothing. See the module header:
    /// the point is that the failure is legible from across a room and quotable over a telephone,
    /// where a blank line is neither.
    #[must_use]
    pub fn msg(&self, key: &str) -> Cow<'_, str> {
        self.format(key, None)
    }

    /// A message with arguments, for a plural or an interpolation.
    ///
    /// ```
    /// # use km_locale::{Catalog, Locale};
    /// let source = "songs = { $count ->\n    [one] { $count } song\n   *[other] { $count } songs\n }\n";
    /// let catalog = Catalog::new(Locale::English, source).expect("the catalog parses");
    /// assert_eq!(catalog.msg_with("songs", &[("count", 1.into())]), "1 song");
    /// assert_eq!(catalog.msg_with("songs", &[("count", 155.into())]), "155 songs");
    /// ```
    #[must_use]
    pub fn msg_with<'a>(&self, key: &str, args: &[(&'a str, FluentValue<'a>)]) -> Cow<'_, str> {
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, value.clone());
        }
        self.format(key, Some(&fluent_args))
    }

    /// Every key this catalog defines.
    ///
    /// The raw material for the parity tests each owning crate writes.
    #[must_use]
    pub fn keys(&self) -> &BTreeSet<String> {
        &self.keys
    }

    /// The keys `other` has that this one does not.
    ///
    /// **This is the test every crate with catalogs owes.** Fluent resolves a missing key at run
    /// time and there is no compiler to catch one, so the guarantee askama was chosen for —
    /// "a field renamed and not updated in the markup is a build failure rather than a blank card
    /// discovered by somebody holding a microphone" — is bought back here, one step later, by
    /// asserting this is empty against the English catalog.
    #[must_use]
    pub fn missing_from(&self, other: &Catalog) -> BTreeSet<String> {
        other.keys.difference(&self.keys).cloned().collect()
    }

    /// The lookup both `msg` methods funnel through.
    fn format(&self, key: &str, args: Option<&FluentArgs<'_>>) -> Cow<'_, str> {
        let Some(message) = self.bundle.get_message(key) else {
            return Cow::Owned(format!("⟦{key}⟧"));
        };
        let Some(pattern) = message.value() else {
            return Cow::Owned(format!("⟦{key}⟧"));
        };
        let mut errors = Vec::new();
        let rendered = self.bundle.format_pattern(pattern, args, &mut errors);
        // A resolution error still yields a best-effort string — a missing argument comes back as
        // `{$count}` rather than as nothing — so the rendering is kept and the fault is left to the
        // tests, which see it as an unresolvable reference rather than as a wrong sentence at 3am.
        rendered
    }
}

impl fmt::Debug for Catalog {
    /// `FluentBundle` is not `Debug`, and a catalog's interesting fact is which locale it is anyway.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Catalog")
            .field("locale", &self.locale)
            .field("keys", &self.keys().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_reads_back_as_the_locale_that_wrote_it() {
        for locale in Locale::ALL {
            assert_eq!(Locale::parse(locale.tag()), Some(*locale), "{locale}");
        }
    }

    #[test]
    fn a_hand_edited_tag_is_read_case_insensitively() {
        // A cookie and a settings file are both things somebody types.
        assert_eq!(Locale::parse("PT-br"), Some(Locale::BrazilianPortuguese));
    }

    #[test]
    fn a_region_we_do_not_have_falls_back_to_the_language() {
        // European Portuguese is served far better by Brazilian Portuguese than by English.
        assert_eq!(
            Locale::best_match("pt-PT"),
            Some(Locale::BrazilianPortuguese)
        );
        assert_eq!(Locale::best_match("en-GB"), Some(Locale::English));
    }

    #[test]
    fn a_language_we_do_not_have_matches_nothing() {
        // Rather than reaching for whichever locale happens to sort first.
        assert_eq!(Locale::best_match("de"), None);
        assert_eq!(Locale::best_match("ja-JP"), None);
    }

    #[test]
    fn negotiation_honors_quality_rather_than_order() {
        assert_eq!(
            negotiate("en;q=0.5,pt-BR;q=0.9"),
            Some(Locale::BrazilianPortuguese)
        );
    }

    #[test]
    fn the_first_entry_wins_a_tie_because_a_header_is_written_in_preference_order() {
        assert_eq!(negotiate("pt-BR,en"), Some(Locale::BrazilianPortuguese));
        assert_eq!(negotiate("en,pt-BR"), Some(Locale::English));
    }

    #[test]
    fn a_header_naming_nothing_we_have_declines_rather_than_guessing() {
        // `None` is the caller's cue to use its own default, which is not the same as this function
        // having an opinion about which locale a German speaker wants.
        assert_eq!(negotiate("de,fr;q=0.8"), None);
        assert_eq!(negotiate(""), None);
    }

    #[test]
    fn a_wildcard_is_not_a_choice() {
        // `*` means "anything", and the caller's default already is the anything.
        assert_eq!(negotiate("*"), None);
        assert_eq!(negotiate("*,pt;q=0.1"), Some(Locale::BrazilianPortuguese));
    }

    #[test]
    fn an_unreadable_quality_is_still_somebody_asking() {
        assert_eq!(
            negotiate("pt-BR;q=banana"),
            Some(Locale::BrazilianPortuguese)
        );
    }

    #[test]
    fn a_missing_key_renders_the_key_rather_than_nothing() {
        let catalog = Catalog::new(Locale::English, "greeting = Hello\n").expect("it parses");
        // Legible from across a room and quotable over a telephone; a blank line is neither.
        assert_eq!(catalog.msg("absent"), "⟦absent⟧");
    }

    #[test]
    fn a_plural_is_chosen_by_the_number_beside_it() {
        let source =
            "songs = { $count ->\n    [one] { $count } song\n   *[other] { $count } songs\n }\n";
        let catalog = Catalog::new(Locale::English, source).expect("it parses");
        assert_eq!(catalog.msg_with("songs", &[("count", 1.into())]), "1 song");
        assert_eq!(catalog.msg_with("songs", &[("count", 0.into())]), "0 songs");
        assert_eq!(
            catalog.msg_with("songs", &[("count", 155.into())]),
            "155 songs"
        );
    }

    #[test]
    fn portuguese_chooses_its_own_plural_rule_rather_than_englishs() {
        let source = "songs = { $count ->\n    [one] { $count } música\n   *[other] { $count } músicas\n }\n";
        let catalog = Catalog::new(Locale::BrazilianPortuguese, source).expect("it parses");
        assert_eq!(
            catalog.msg_with("songs", &[("count", 1.into())]),
            "1 música"
        );
        assert_eq!(
            catalog.msg_with("songs", &[("count", 155.into())]),
            "155 músicas"
        );
    }

    #[test]
    fn an_interpolated_value_carries_no_isolating_marks() {
        // U+2068/U+2069 have no glyph in the bundled font and no answer in the book's cp1252
        // encoder, so a song title wrapped in them would draw two blank boxes on a television.
        let catalog =
            Catalog::new(Locale::English, "next = next: { $title }\n").expect("it parses");
        let rendered = catalog.msg_with("next", &[("title", "Tempo Perdido".into())]);
        assert_eq!(rendered, "next: Tempo Perdido");
        assert!(
            !rendered.contains('\u{2068}') && !rendered.contains('\u{2069}'),
            "{rendered:?} carries an isolating mark"
        );
    }

    #[test]
    fn missing_from_names_what_a_translation_has_not_caught_up_with() {
        let english =
            Catalog::new(Locale::English, "one = One\ntwo = Two\n").expect("english parses");
        let portuguese =
            Catalog::new(Locale::BrazilianPortuguese, "one = Um\n").expect("portuguese parses");
        assert_eq!(
            portuguese.missing_from(&english),
            BTreeSet::from(["two".to_owned()])
        );
        assert!(english.missing_from(&portuguese).is_empty());
    }

    #[test]
    fn every_locale_names_itself_in_its_own_language() {
        // The one string deliberately not in the reader's language: a picker offering `Portuguese`
        // is naming the language to somebody who by definition cannot read the label.
        assert_eq!(Locale::BrazilianPortuguese.endonym(), "Português (Brasil)");
        for locale in Locale::ALL {
            assert!(!locale.endonym().is_empty(), "{locale}");
        }
    }

    #[test]
    fn english_is_the_default_because_it_is_what_every_message_is_written_in() {
        assert_eq!(Locale::default(), Locale::English);
    }

    #[test]
    fn every_catalog_crosses_a_thread() {
        // A catalog is built once and shared by every request on a multi-threaded runtime, so this
        // is the shape every caller needs. It is a compile-time assertion written as a test because
        // the failure it guards against — reaching for the default `FluentBundle`, whose memoizer is
        // a `RefCell` — is a one-word edit that looks harmless.
        fn shared<T: Send + Sync>() {}
        shared::<Catalog>();
        shared::<Locale>();
    }
}
