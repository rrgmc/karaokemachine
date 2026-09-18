//! The askama seam: `{{ "queue-full"|t }}`.
//!
//! **No template struct gains a field.** askama 0.16 carries a `&dyn Values` store through
//! `render_with_values`, and it propagates into a nested `{{ child|safe }}` render on its own — so
//! the locale is put in once, by whichever of the render helpers a handler called, and every
//! template and fragment underneath can reach it. The alternative was a `t:` field on each of the
//! template structs across the two pages crates, threaded through every construction site, which is
//! a lot of edits to say one thing.
//!
//! **Markup carries a key and nothing else.** A message that interpolates anything — a song title, a
//! folder, a count — is composed in Rust through [`Catalog::msg_with`] and arrives as a field. That
//! is not a limitation worked around but the rule `km-remote-pages`'s `model` module already states
//! about formatting: "the alternative is arithmetic in markup, which is where it stops being
//! testable." A plural is arithmetic.
//!
//! ```text
//! <a href="/now">{{ "tab-now"|t }}</a>
//! <button title="{{ "clear-search"|t }}" aria-label="{{ "clear-search"|t }}">✕</button>
//! <span>{{ song_count }}</span>   {# composed in Rust, where a test can reach it #}
//! ```
//!
//! Escaping is left on, which is correct: a translated string is data like any other, and a catalog
//! is a file somebody edits.

// `filter_fn` expands into a builder struct and two methods and documents none of them, so the
// workspace's `missing_docs` fires on generated code nobody can write a doc comment for. It has to
// sit on the module because the macro drops attributes written above it. Confined to this file,
// which is three public items long and documents all three, so what it hides is only ever generated.
#![allow(
    missing_docs,
    reason = "askama::filter_fn expands into an undocumented builder struct"
)]

use std::any::Any;

use askama::Values;

use crate::Catalog;

/// The key the locale is filed under in askama's values store.
///
/// Namespaced because the store is shared with whatever else a host puts in it, and a bare `locale`
/// is the sort of word two crates pick independently.
pub const VALUES_KEY: &str = "km_locale.catalog";

/// Puts a catalog where the [`t`] filter will find it.
///
/// A key-and-value pair is askama's smallest `Values` implementation, so this allocates nothing and
/// the catalog travels as a reference. The value is `&dyn Any` because that is what askama's `Value`
/// is implemented for, and the filter downcasts it back:
///
/// ```ignore
/// template.render_with_values(&km_locale::filters::values(catalog))
/// ```
#[must_use]
pub fn values(catalog: &'static Catalog) -> (&'static str, &'static dyn Any) {
    (VALUES_KEY, catalog)
}

/// One message, in the locale this render was given.
///
/// A key the catalog does not have renders as `⟦the-key⟧` rather than as nothing — see
/// [`Catalog::msg`]. **A locale that was never put in the store is the same kind of fault and gets
/// the same kind of answer**: the key, rather than a 500. A handler that forgot to pass one has a
/// bug, and the bug should be legible on the page rather than turning every route it touches into a
/// blank error — which is the same judgement `views::page` already makes about a template that will
/// not render.
///
/// # Errors
///
/// Never. The `Result` is askama's filter signature, not a failure this can have.
#[askama::filter_fn]
pub fn t(key: &str, values: &dyn Values) -> askama::Result<String> {
    let Ok(catalog) = askama::get_value::<Catalog>(values, VALUES_KEY) else {
        return Ok(format!("⟦{key}⟧"));
    };
    Ok(catalog.msg(key).into_owned())
}

#[cfg(test)]
mod tests {
    use askama::Template;

    use super::*;
    use crate::Locale;
    // Askama resolves `|t` against a `filters` module in the scope the template is derived in. These
    // tests are *inside* that module, so they have to name it the way a caller does — which is also
    // the line every consuming crate writes once in its own `views.rs`.
    use crate::filters;

    /// Two messages, one of them a plural, and a nested fragment to prove the store propagates.
    const SOURCE: &str = "\
tab-now = Now
songs = { $count ->
    [one] { $count } song
   *[other] { $count } songs
 }
";

    fn catalog() -> &'static Catalog {
        // Leaked rather than a `OnceLock`, because a test wants a fresh one and `&'static` is the
        // shape the filter takes.
        Box::leak(Box::new(
            Catalog::new(Locale::English, SOURCE).expect("the fixture parses"),
        ))
    }

    #[derive(Template)]
    #[template(source = "{{ \"tab-now\"|t }}", ext = "txt")]
    struct Simple;

    #[derive(Template)]
    #[template(source = "{{ \"absent-key\"|t }}", ext = "txt")]
    struct Absent;

    #[derive(Template)]
    #[template(source = "[{{ inner|safe }}]", ext = "txt")]
    struct Outer {
        inner: Simple,
    }

    #[test]
    fn a_key_renders_the_message() {
        let rendered = Simple
            .render_with_values(&values(catalog()))
            .expect("it renders");
        assert_eq!(rendered, "Now");
    }

    #[test]
    fn a_plural_is_composed_in_rust_and_arrives_as_a_field() {
        // Deliberately not a filter argument. Markup carries a key; anything that computes is
        // composed where a test can reach it, which is the rule the pages' `model` module states.
        let catalog = catalog();
        for (n, expected) in [(1, "1 song"), (0, "0 songs"), (155, "155 songs")] {
            assert_eq!(catalog.msg_with("songs", &[("count", n.into())]), expected);
        }
    }

    #[test]
    fn the_store_reaches_a_nested_fragment() {
        // The whole reason no template struct needs a field: a fragment rendered inside another
        // through `{{ child|safe }}` is given the same values, so a locale put in once reaches all
        // the way down.
        let rendered = Outer { inner: Simple }
            .render_with_values(&values(catalog()))
            .expect("it renders");
        assert_eq!(rendered, "[Now]");
    }

    #[test]
    fn a_missing_key_renders_the_key_rather_than_failing_the_page() {
        let rendered = Absent
            .render_with_values(&values(catalog()))
            .expect("it renders");
        assert_eq!(rendered, "⟦absent-key⟧");
    }

    #[test]
    fn a_render_with_no_locale_says_so_rather_than_erroring() {
        // A handler that forgot to pass one has a bug, and it should be legible on the page instead
        // of turning the route into a blank 500.
        let rendered = Simple.render().expect("it still renders");
        assert_eq!(rendered, "⟦tab-now⟧");
    }
}
