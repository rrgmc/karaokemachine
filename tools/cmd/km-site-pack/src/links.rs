//! The links a page carries.
//!
//! This reads attribute values and nothing else. A site that offers song files is often hand-written
//! markup a strict parser refuses, and the two attributes that carry an address are spelled the same
//! in all of it.

use std::collections::BTreeSet;

use url::Url;

/// The attributes whose value is an address: a link, and a frame or an embedded player.
const ATTRIBUTES: [&str; 2] = ["href", "src"];

/// Every `http` and `https` address a page links, resolved against the page's own.
///
/// Each address appears once, in the order the page gives them, with its fragment removed.
#[must_use]
pub fn extract_links(html: &str, base: &Url) -> Vec<Url> {
    let bytes = html.as_bytes();
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let Some((value, next)) = attribute_at(html, at) else {
            at += 1;
            continue;
        };
        at = next;
        if let Some(url) = resolve(value, base)
            && seen.insert(url.as_str().to_owned())
        {
            links.push(url);
        }
    }
    links
}

/// The value of an address attribute starting at `at`, and where to read on from.
fn attribute_at(html: &str, at: usize) -> Option<(&str, usize)> {
    let bytes = html.as_bytes();
    // An attribute follows white space or the quote that closed the one before it. That keeps
    // `data-href` and a word inside running text out.
    if at > 0 && !(bytes[at - 1].is_ascii_whitespace() || matches!(bytes[at - 1], b'"' | b'\'')) {
        return None;
    }
    let name = ATTRIBUTES.iter().find(|name| {
        bytes
            .get(at..at + name.len())
            .is_some_and(|found| found.eq_ignore_ascii_case(name.as_bytes()))
    })?;
    let mut cursor = skip_space(bytes, at + name.len());
    if bytes.get(cursor) != Some(&b'=') {
        return None;
    }
    cursor = skip_space(bytes, cursor + 1);
    let quote = *bytes.get(cursor)?;
    let (start, end) = if quote == b'"' || quote == b'\'' {
        let start = cursor + 1;
        let length = bytes[start..].iter().position(|&b| b == quote)?;
        (start, start + length)
    } else {
        let length = bytes[cursor..]
            .iter()
            .position(|&b| b.is_ascii_whitespace() || b == b'>')
            .unwrap_or(bytes.len() - cursor);
        (cursor, cursor + length)
    };
    // Both ends sit beside an ASCII byte, so the slice falls on character boundaries.
    Some((html.get(start..end)?, end))
}

fn skip_space(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    at
}

/// One attribute value as an address, or nothing when it is not one this tool follows.
fn resolve(value: &str, base: &Url) -> Option<Url> {
    let value = unescape(value.trim());
    if value.is_empty() || value.starts_with('#') {
        return None;
    }
    let mut url = base.join(&value).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

/// Undoes the character references an address is commonly written with.
fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    value
        .replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#32;", " ")
}

/// The addresses a sitemap lists, which are the text of its `<loc>` elements.
#[must_use]
pub fn sitemap_locations(xml: &str) -> Vec<Url> {
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find("<loc>") {
        rest = &rest[open + "<loc>".len()..];
        let Some(close) = rest.find("</loc>") else {
            break;
        };
        if let Ok(url) = Url::parse(&unescape(rest[..close].trim()))
            && matches!(url.scheme(), "http" | "https")
        {
            found.push(url);
        }
        rest = &rest[close..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(html: &str) -> Vec<String> {
        let base = Url::parse("http://127.0.0.1:8000/songs/index.html").expect("a base");
        extract_links(html, &base)
            .into_iter()
            .map(String::from)
            .collect()
    }

    #[test]
    fn quoted_unquoted_and_spaced_values_are_all_read() {
        let found = links(
            r#"<a href="one.kar">1</a> <A HREF='two.kar'>2</A>
               <a href=three.kar>3</a> <a class="x" href = "four.kar">4</a>"#,
        );
        assert_eq!(
            found,
            [
                "http://127.0.0.1:8000/songs/one.kar",
                "http://127.0.0.1:8000/songs/two.kar",
                "http://127.0.0.1:8000/songs/three.kar",
                "http://127.0.0.1:8000/songs/four.kar",
            ]
        );
    }

    #[test]
    fn a_relative_address_is_resolved_against_the_page() {
        let found = links(r#"<a href="../up.kar"> <a href="/root.kar"> <frame src="menu.html">"#);
        assert_eq!(
            found,
            [
                "http://127.0.0.1:8000/up.kar",
                "http://127.0.0.1:8000/root.kar",
                "http://127.0.0.1:8000/songs/menu.html",
            ]
        );
    }

    #[test]
    fn a_name_outside_ascii_is_spelled_as_a_server_accepts_it() {
        let found = links(r#"<a href="josé one.kar">x</a>"#);
        assert_eq!(found, ["http://127.0.0.1:8000/songs/jos%C3%A9%20one.kar"]);
    }

    #[test]
    fn a_character_reference_and_a_fragment_are_undone() {
        let found = links(r##"<a href="get.php?a=1&amp;b=2#top">x</a> <a href="#top">y</a>"##);
        assert_eq!(found, ["http://127.0.0.1:8000/songs/get.php?a=1&b=2"]);
    }

    #[test]
    fn what_is_not_an_address_to_follow_is_left_out() {
        let found = links(
            r#"<a href="mailto:a@example.com"> <a href="javascript:void(0)">
               <a data-href="no.kar"> the word href=loose.kar in text is read, as markup is
               <a href="one.kar"> <a href="one.kar">"#,
        );
        assert_eq!(
            found,
            [
                "http://127.0.0.1:8000/songs/loose.kar",
                "http://127.0.0.1:8000/songs/one.kar",
            ]
        );
    }

    #[test]
    fn a_sitemap_gives_its_locations() {
        let found = sitemap_locations(
            "<urlset><url><loc> http://127.0.0.1/a.html </loc></url>\
             <url><loc>http://127.0.0.1/b.html?x=1&amp;y=2</loc></url><url><loc>nope</loc></url>",
        );
        let found: Vec<String> = found.into_iter().map(String::from).collect();
        assert_eq!(
            found,
            ["http://127.0.0.1/a.html", "http://127.0.0.1/b.html?x=1&y=2"]
        );
    }
}
