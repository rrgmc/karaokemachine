//! What the phone remembers.
//!
//! Four cookies, all of them small, all `HttpOnly` and `SameSite=Lax`, and none of them anything the
//! machine needs to know. They exist because a remote is picked up, put down, and picked up again:
//! coming back within the evening to a list that has forgotten the language you chose and the artist
//! you were inside is the difference between a remote and a search engine.
//!
//! The division is deliberate and follows the Go remote's:
//!
//! * **[`BROWSE`] is where you were** — mode, query, filters, which artist or folder you had opened.
//!   Restored when the Songs tab is tapped with no query at all, which is exactly what tapping a tab
//!   means, and *not* when a URL says something, because then the URL is the answer. Two hours,
//!   refreshed by every Songs-tab request: a remote picked up after a break is in the middle of
//!   something, and a remote picked up the next evening is starting.
//! * **[`SINGER`] and [`EXTRA`] are what you prefer** — your name, and whether the extra row
//!   actions are revealed. These outlive a session by a year and are never cleared by navigation.
//!   [`HIDDEN`], the packages left out of this phone's song list, is a preference of the same kind.
//!
//! * **[`AT`] is where you were *just now*** — the row at the top of the Songs list. An hour, the
//!   shortest of the three: coming back from the Queue tab should land on the row you were reading,
//!   and a break long enough to forget [`BROWSE`] takes the row with it.
//!
//! **[`AT`] is the one cookie the server reads and never writes**, and it is the only one a script
//! can see. `live.js` sets it, because the position is a fact only the browser has; nothing in Rust
//! sets it, which is what leaves [`set`]'s `HttpOnly` unconditional and means no Rust path can emit
//! a script-readable cookie by accident. [`TOKEN`] is why that default is worth keeping absolute.
//! **Two functions build a cookie here and both force it**: [`set`] for the five below, and
//! `km_locale::set_cookie` for [`LOCALE`], which is shared with `km-admin`.
//!
//! Encoding is percent-escaping done here rather than by a dependency. A cookie value may not carry
//! a space, a semicolon or a comma, song titles carry all three, and the alternative — pulling in a
//! form-encoding crate for two functions — is a dependency for thirty lines.

use axum::http::HeaderMap;
use axum::http::header::{self, COOKIE, SET_COOKIE};
use km_locale::Locale;
use km_songcode::SongCode;

use crate::model::Mode;

/// Where you were: mode, query, filters, the artist or folder you had opened.
pub const BROWSE: &str = "km_browse";
/// What to put in the `singer` field of everything queued from this phone.
pub const SINGER: &str = "km_singer";
/// Whether the extra per-row actions are revealed.
pub const EXTRA: &str = "km_extra";
/// The packages this phone leaves out of its song list, as package ids joined by `.`.
///
/// A preference, so it lasts a year like [`SINGER`]. `.` because a package id is sixteen hexadecimal
/// characters and a dot needs no escaping, which keeps the most ids inside a cookie's size.
pub const HIDDEN: &str = "km_hidden";
/// The most package ids [`HIDDEN`] holds: 128 ids of seventeen characters is about 2.2 KB, well
/// inside the 4 KB a browser keeps for one cookie.
pub const MAX_HIDDEN: usize = 128;
/// What language this viewer reads the pages in.
///
/// **Per viewer, not per machine**, which is the whole difference between this and
/// `machine.locale`. The television is in a room and the room has one language; a phone belongs to
/// one person, and two people at the same party can read the same queue in two languages.
///
/// Absent until somebody chooses, and absent is not English: with no cookie the page follows
/// `Accept-Language`, so a guest who has never used this remote gets their own language on the
/// first load. See [`Prefs::locale`].
pub const LOCALE: &str = km_locale::COOKIE;
/// The admin token, once somebody has typed the machine's password.
///
/// A cookie rather than anything cleverer, because these are ordinary page loads and form posts: a
/// browser cannot be told to put an `Authorization` header on a link, which is what the JSON API
/// expects and what the dev remote can do because it is driven by script. `HttpOnly` means no script
/// on the page can read it, which is worth having even here — the token is the machine's password by
/// another name.
pub const TOKEN: &str = "km_token";

/// The row that was at the top of the Songs list when you last left it.
///
/// `row=<code>&at=<index>&list=<tag>`. Written by `live.js` on `pagehide` and read here; see the
/// module header for why this one is not `HttpOnly` and why nothing in Rust sets it.
///
/// The `list` tag is what stops an anchor being applied to a list it was not taken from. [`BROWSE`]
/// cannot serve as one: it is rewritten by the htmx search swap too, so by the time an old anchor
/// arrives the cookie beside it already describes the *new* list and the two would always agree.
pub const AT: &str = "km_at";

/// Two hours. Where you were is worth remembering across a break in an evening, and a fresh evening
/// that opens inside somebody's half-finished search reads as a fault rather than as a memory.
///
/// Every Songs-tab request rewrites the cookie, so the two hours run from the last time somebody was
/// browsing rather than from the first — which is what makes this "you have just put the phone
/// down" rather than "you looked at this recently".
const BROWSE_MAX_AGE: u32 = 60 * 60 * 2;
/// A year. What you prefer does not go stale.
const PREF_MAX_AGE: u32 = 60 * 60 * 24 * 365;

/// The longest name that will be stored, in characters.
///
/// Not a validation rule so much as a refusal to put an essay in a header. It is truncated rather
/// than rejected: somebody who pasted something silly gets a short name, not an error page.
const MAX_SINGER: usize = 40;

/// The row somebody was looking at, and the list it was in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// The song at the top of the screen.
    pub row: SongCode,
    /// How far down the list it was, so the server knows how many rows to render.
    pub at: usize,
    /// [`list_tag`] of the list it was taken from.
    pub list: String,
}

/// Everything read off the request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefs {
    /// The name to file queued songs under.
    pub singer: Option<String>,
    /// Whether the extra row actions are showing.
    pub extra: bool,
    /// The remembered browse state, as a query string.
    pub browse: Option<String>,
    /// The row the Songs list was left on.
    pub at: Option<Anchor>,
    /// What language to draw the page in.
    ///
    /// Already resolved: the cookie if there is one, the browser's `Accept-Language` if not, and
    /// English if neither names a language this build has. A page never has to ask twice.
    pub locale: Locale,
    /// The package ids this phone leaves out of its song list. See [`HIDDEN`].
    pub hidden_packages: Vec<String>,
}

impl Prefs {
    /// Reads them all off the request headers.
    pub fn read(headers: &HeaderMap) -> Self {
        Self {
            singer: cookie(headers, SINGER).filter(|name| !name.is_empty()),
            extra: cookie(headers, EXTRA).as_deref() == Some("1"),
            browse: cookie(headers, BROWSE).filter(|state| !state.is_empty()),
            at: cookie(headers, AT).as_deref().and_then(anchor),
            locale: locale(headers),
            hidden_packages: cookie(headers, HIDDEN)
                .map(|value| hidden_packages(value.split('.')))
                .unwrap_or_default(),
        }
    }
}

/// The package ids worth keeping from a list somebody sent: generated ids only, each once, and no
/// more than [`MAX_HIDDEN`].
///
/// **Anything else is dropped rather than refused**, for the reason [`anchor`] gives: anybody can
/// edit a cookie, and the answer to a bad one is the page without it. A machine refuses a package
/// whose id is not of the generated shape, so no other value can name a package.
pub fn hidden_packages<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for id in ids.into_iter().map(str::trim) {
        if kept.len() == MAX_HIDDEN {
            break;
        }
        if km_kmpkg::PackageMeta::is_generated_id(id) && !kept.iter().any(|known| known == id) {
            kept.push(id.to_owned());
        }
    }
    kept
}

/// The `Set-Cookie` value for [`HIDDEN`]. An empty list clears the cookie.
pub fn set_hidden(ids: &[String]) -> String {
    if ids.is_empty() {
        set(HIDDEN, "", 0)
    } else {
        set_pref(HIDDEN, &ids.join("."))
    }
}

/// Parses the [`AT`] cookie, or `None` for anything that does not read as one.
///
/// **Anything malformed is ignored rather than refused**, which is `decode`'s rule one level up and
/// for the same reason: anybody can edit a cookie, and the answer to a bad one is to render the page
/// without it, not to refuse the page. `at` is clamped rather than rejected, because a number past
/// the cap is a phone that scrolled a long way and not an attack — it lands at the ceiling and the
/// row is simply not in the window.
pub fn anchor(value: &str) -> Option<Anchor> {
    let mut row = None;
    let mut at = None;
    let mut list = None;
    for pair in value.split('&') {
        let Some((key, raw)) = pair.split_once('=') else {
            continue;
        };
        let raw = decode(raw);
        match key {
            // `SongCode`'s `FromStr` is digits-only, so this validates itself and needs no escaping.
            "row" => row = raw.parse::<SongCode>().ok(),
            "at" => at = raw.parse::<usize>().ok(),
            "list" => list = Some(raw),
            _ => {}
        }
    }
    Some(Anchor {
        row: row?,
        at: at?.min(crate::handlers::MAX_RESTORE.saturating_sub(1)),
        list: list?,
    })
}

/// A short stamp for a browse state, so an anchor can say which list it came from.
///
/// The same FNV-1a and the same eight hex digits `ASSET_VERSION` uses, and the same caveat: it is
/// not defending against anything, it only has to differ when the list differs. `None` — the default
/// state, everything at its defaults — hashes the empty string, so the top of the plain song list
/// has a tag like every other view rather than a special case.
pub fn list_tag(state: Option<&str>) -> String {
    let hash = crate::fnv1a(state.unwrap_or("").as_bytes(), crate::FNV_OFFSET);
    String::from_utf8_lossy(&crate::hex8(hash)).into_owned()
}

/// One cookie's decoded value.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(COOKIE) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        for pair in header.split(';') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key.trim() == name {
                return Some(decode(value.trim()));
            }
        }
    }
    None
}

/// The `Set-Cookie` value for a preference that should outlive the session.
pub fn set_pref(name: &str, value: &str) -> String {
    set(name, value, PREF_MAX_AGE)
}

/// The `Set-Cookie` value for an admin token.
///
/// Its lifetime comes from the machine, not from here, and that matters: the machine holds its
/// tokens in memory and forgets them all on a restart, so a cookie that outlived the token would
/// leave a phone quietly holding something that no longer works — and every action failing for a
/// reason the page could not name. Matching the two means the cookie disappears at about the moment
/// the token does, and the login page comes back.
pub fn set_token(token: &str, expires_in_secs: u64) -> String {
    set(TOKEN, token, expires_in_secs.min(u32::MAX as u64) as u32)
}

/// What language to draw a page in, given what this request said.
///
/// **The cookie wins over the header**, because the cookie is a choice somebody made on this device
/// and the header is what their browser was installed with. Neither is an error when it names a
/// language this build does not have — a nonsense cookie is something anybody can type — so both
/// fall through to English, which is what every message is written in first.
///
/// **Negotiation happens here rather than in middleware or an extractor**, because [`Prefs::read`]
/// is already the first line of every handler and this is a preference like the other four. The
/// alternative would have to be installed by each of the five hosts that mount these pages, which is
/// exactly the drift that arrangement exists to avoid.
#[must_use]
pub fn locale(headers: &HeaderMap) -> Locale {
    km_locale::choose(
        cookie(headers, LOCALE).as_deref(),
        headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok()),
    )
}

/// The `Set-Cookie` value for a chosen language.
///
/// **Built by `km-locale` rather than by [`set_pref`] beside it**, alone among the five here, and
/// the reason is that this is the one cookie a second program also writes: `km-admin`'s front door
/// offers the same choice on its own origin. The name lives there already, for the surfaces that
/// share an origin; the `Path` and the life are the rest of the same spelling. A year, like the
/// preferences around it.
#[must_use]
pub fn set_locale(locale: Locale) -> String {
    km_locale::set_cookie(locale)
}

/// The `Set-Cookie` value for the remembered browse state.
pub fn set_browse(state: &str) -> String {
    set(BROWSE, state, BROWSE_MAX_AGE)
}

/// The `Set-Cookie` value that removes a cookie.
///
/// A browse state back at its defaults is *cleared* rather than stored, so that a phone which has
/// been reset does not keep sending a cookie saying "everything is normal".
pub fn clear(name: &str) -> String {
    set(name, "", 0)
}

fn set(name: &str, value: &str, max_age: u32) -> String {
    // No `Secure`: the remote is served over plain HTTP on a home LAN, and marking these `Secure`
    // would mean a browser silently discarding every one of them. `SameSite=Lax` is what actually
    // matters here, and it is set.
    format!(
        "{name}={}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax",
        encode(value)
    )
}

/// Adds a `Set-Cookie` to a response's headers, keeping any already there.
pub fn attach(headers: &mut HeaderMap, cookie: String) {
    if let Ok(value) = cookie.parse() {
        headers.append(SET_COOKIE, value);
    }
}

/// Trims a typed name to something worth storing.
pub fn tidy_singer(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_SINGER).collect())
}

/// The query string that would restore this browse state.
///
/// Returns `None` for a state that is entirely default, which is the signal to clear the cookie
/// rather than store it.
pub fn browse_state(
    mode: Mode,
    query: &str,
    language: Option<&str>,
    tags: &[String],
    initial: Option<char>,
    artist: Option<&str>,
    folder: Option<i64>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if mode != Mode::Songs {
        parts.push(format!("mode={}", mode.as_str()));
    }
    if !query.trim().is_empty() {
        parts.push(format!("q={}", encode(query)));
    }
    if let Some(language) = language.filter(|value| !value.is_empty()) {
        parts.push(format!("language={}", encode(language)));
    }
    // Comma-joined, and it survives `encode`/`decode` as one value — which is the reason the wire
    // spelling is a scalar rather than a repeated key. A list would need a second escaping inside
    // a cookie that is already `&`-separated and percent-encoded.
    if !tags.is_empty() {
        parts.push(format!("tags={}", encode(&tags.join(","))));
    }
    if let Some(initial) = initial {
        parts.push(format!("initial={}", encode(&initial.to_string())));
    }
    if let Some(artist) = artist.filter(|value| !value.is_empty()) {
        parts.push(format!("artist={}", encode(artist)));
    }
    if let Some(folder) = folder {
        parts.push(format!("folder={folder}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("&"))
    }
}

/// Percent-encodes everything a cookie value or a query parameter may not carry.
///
/// Conservative on purpose — anything outside the unreserved set goes through — because the values
/// here are song titles and people's names out of a corpus that contains every punctuation mark
/// there is.
///
/// **Not the same codec as `km-package-builder`'s `form::encode`, and not to be merged with it.**
/// They were read as duplicates in a review and are not: that one is *form* encoding and writes a
/// space as `+`, because it is the inverse of a form body's parser. This one writes `%20`, because
/// what it mostly encodes is a **cookie value**, where `+` is a plus sign and nothing else. Sharing
/// one function would mean choosing one of those two meanings for a space and being wrong about the
/// other half of the callers.
///
/// The same reason rules out `form_urlencoded::byte_serialize` here, which is what the builder's now
/// uses: it is the form convention, `+` and all.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Undoes [`encode`], and leaves anything malformed alone rather than failing.
///
/// A cookie is attacker-controlled in the sense that anybody can edit one; the right response to a
/// stray `%` is to render it, not to refuse the page.
pub fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        if bytes[index] == b'+' {
            out.push(b' ');
            index += 1;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, cookie.parse().expect("a cookie header"));
        headers
    }

    #[test]
    fn a_round_trip_survives_everything_a_title_can_contain() {
        for value in [
            "Águas de Março",
            "AC/DC",
            "rock 'n' roll",
            "a; b, c=d",
            "100%",
        ] {
            assert_eq!(decode(&encode(value)), value, "{value}");
        }
    }

    #[test]
    fn hidden_packages_round_trip_and_drop_what_cannot_name_a_package() {
        let ids = vec!["1f4a9c8e2b7d0356".to_owned(), "a1b2c3d4e5f60789".to_owned()];
        let set = set_hidden(&ids);
        let value = set
            .strip_prefix("km_hidden=")
            .and_then(|rest| rest.split(';').next())
            .expect("a value");
        let headers = headers_with(&format!("km_hidden={value}"));
        assert_eq!(Prefs::read(&headers).hidden_packages, ids);

        let messy = hidden_packages([
            "1f4a9c8e2b7d0356",
            "vol1",
            "1F4A9C8E2B7D0356",
            "1f4a9c8e2b7d0356",
            "",
        ]);
        assert_eq!(messy, ["1f4a9c8e2b7d0356"]);

        let many: Vec<String> = (0..MAX_HIDDEN + 10).map(|n| format!("{n:016x}")).collect();
        assert_eq!(
            hidden_packages(many.iter().map(String::as_str)).len(),
            MAX_HIDDEN
        );
        assert!(
            set_hidden(&[]).contains("Max-Age=0"),
            "nothing hidden clears it"
        );
    }

    #[test]
    fn one_cookie_is_found_among_several() {
        let headers = headers_with("other=1; km_singer=Ana; km_extra=1");
        assert_eq!(cookie(&headers, SINGER).as_deref(), Some("Ana"));
        assert_eq!(cookie(&headers, EXTRA).as_deref(), Some("1"));
        assert_eq!(cookie(&headers, BROWSE), None);
    }

    #[test]
    fn preferences_read_off_a_request() {
        let headers = headers_with("km_singer=Ana; km_extra=1; km_sort=0");
        let prefs = Prefs::read(&headers);
        assert_eq!(prefs.singer.as_deref(), Some("Ana"));
        assert!(prefs.extra);
    }

    /// Anyone can edit a cookie; a stray `%` should render, not produce an error page.
    #[test]
    fn a_malformed_escape_is_left_alone_rather_than_rejected() {
        assert_eq!(decode("100% sure"), "100% sure");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("%"), "%");
    }

    #[test]
    fn a_default_browse_state_is_not_worth_storing() {
        assert_eq!(
            browse_state(Mode::Songs, "  ", None, &[], None, None, None),
            None
        );
    }

    #[test]
    fn a_browse_state_restores_where_you_were() {
        let state = browse_state(
            Mode::Artists,
            "jobim",
            Some("pt"),
            &[],
            Some('J'),
            Some("Tom Jobim"),
            None,
        )
        .expect("a state");
        assert_eq!(
            state,
            "mode=artists&q=jobim&language=pt&initial=J&artist=Tom%20Jobim"
        );
    }

    /// The tags ride the cookie as one comma-joined value, which is what lets a scalar carry a set.
    #[test]
    fn a_browse_state_remembers_the_tags() {
        let tags = ["brasil".to_owned(), "rock".to_owned()];
        let state = browse_state(Mode::Songs, "", None, &tags, None, None, None).expect("a state");
        assert_eq!(state, "tags=brasil%2Crock");
        // And it comes back out as the same one value: the comma is encoded, so the `&`-separated
        // cookie cannot mistake it for a second parameter.
        assert_eq!(decode("brasil%2Crock"), "brasil,rock");
    }

    #[test]
    fn a_long_name_is_shortened_rather_than_refused() {
        let long = "a".repeat(200);
        assert_eq!(tidy_singer(&long).expect("a name").chars().count(), 40);
        assert_eq!(tidy_singer("   "), None);
        assert_eq!(tidy_singer("  Ana  ").as_deref(), Some("Ana"));
    }

    #[test]
    fn clearing_a_cookie_expires_it() {
        assert!(clear(BROWSE).contains("Max-Age=0"));
    }

    /// Where you were lasts a break in an evening, and what you prefer lasts a year.
    ///
    /// The number is the whole of the feature: a phone opened the next day has no cookie to send,
    /// so the Songs tab draws the top of the list with nothing filtered and nothing typed.
    #[test]
    fn a_browse_state_is_kept_for_two_hours_and_a_preference_for_a_year() {
        assert!(set_browse("q=jobim").contains("Max-Age=7200"));
        assert!(set_pref(SINGER, "Ana").contains("Max-Age=31536000"));
    }

    /// `Secure` would make a browser discard every one of these over plain HTTP on a home LAN.
    #[test]
    fn cookies_are_not_marked_secure() {
        let value = set_pref(SINGER, "Ana");
        assert!(!value.contains("Secure"));
        assert!(value.contains("SameSite=Lax"));
        assert!(value.contains("HttpOnly"));
    }

    #[test]
    fn an_anchor_reads_back_the_way_the_script_wrote_it() {
        let read = anchor("row=1137&at=137&list=deadbeef").expect("an anchor");
        assert_eq!(read.row, km_songcode::SongCode::new(1137));
        assert_eq!(read.at, 137);
        assert_eq!(read.list, "deadbeef");
    }

    /// A phone that scrolled a very long way is not an attack; it lands at the ceiling, where the
    /// row it named is simply not in the window.
    #[test]
    fn an_anchor_past_the_cap_is_clamped_rather_than_refused() {
        let read = anchor("row=1001&at=99999&list=x").expect("an anchor");
        assert_eq!(read.at, crate::handlers::MAX_RESTORE - 1);
    }

    /// Anybody can edit a cookie. Every one of these is ignored, not an error page.
    #[test]
    fn an_anchor_that_does_not_read_as_one_is_ignored() {
        assert_eq!(anchor("row=hello&at=1&list=x"), None);
        assert_eq!(anchor("row=1001&list=x"), None);
        assert_eq!(anchor("row=1001&at=1"), None);
        assert_eq!(anchor(""), None);
    }

    /// The tag only has to differ when the list differs — the same claim `ASSET_VERSION` makes.
    #[test]
    fn two_lists_are_stamped_differently_and_one_list_is_stamped_the_same_way_twice() {
        let default = list_tag(None);
        assert_eq!(default.len(), 8);
        assert_eq!(default, list_tag(None));
        assert_eq!(list_tag(Some("q=jobim")), list_tag(Some("q=jobim")));
        assert_ne!(default, list_tag(Some("q=jobim")));
        assert_ne!(list_tag(Some("q=jobim")), list_tag(Some("q=jobin")));
    }
}
