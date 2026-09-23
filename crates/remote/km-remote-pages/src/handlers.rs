//! Every handler.
//!
//! Two shapes, and the distinction decides how a page behaves.
//!
//! * A **`GET`** returns either a whole page or the one fragment htmx asked for, chosen by the
//!   `fragment` parameter — but **only for a real htmx request**. A plain browser asking for the same
//!   URL gets the page. That is not politeness: `hx-push-url` puts `fragment=list` in the address
//!   bar, so without the downgrade a reload would render a bare list with no layout, no tab bar and
//!   no way back.
//! * A **`POST`** returns whatever it changed, plus an out-of-band toast saying what happened. It
//!   never redirects: a redirect after an htmx post reloads the whole page and loses the search
//!   somebody spent a minute narrowing.
//!
//! **Nothing here answers with a non-2xx for an ordinary refusal.** htmx does not swap on an error
//! response, so a queue-is-full reported as a 409 would leave the button that caused it visually
//! dead and the reason nowhere on screen. The status codes the API uses are turned into toasts here,
//! which is the layer that has somewhere to put them.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use askama::Template;
use axum::extract::{Path, Query, State};

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use km_api::dto::{OriginDto, SettingsPatchDto};
use km_locale::{Catalog, Locale};
use km_song::text::fold;
use km_songcode::SongCode;
use qrcode::EcLevel;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::form;

use crate::machine::{
    ArtistFilter, BrowseQuery, Connection, FolderRow, Miss, Order, Reconciliation, RemoteError,
    Resolution, Scanned, SongPage, SongRef, Transport,
};
use crate::model::{Mode, PlayerView, QueueRow, SongRow};
use crate::prefs::{self, Prefs};
use crate::views::{
    self, ArtistLine, BackupPage, Badge, Banner, BrowseBlock, BrowsePage, Chrome, Conn, FolderLine,
    ListBlock, MachineBlock, NowBar, NowPage, PackageSetting, PackagesPage, PlayerBlock,
    PositionBlock, QueueBlock, QueueCount, QueuePage, RestoreDonePage, RestorePage, RowsBlock,
    SetupPage, ShareConfirmPage, ShareDonePage, SharePage, ShareReceivePage, ShareSendPage, Sheet,
    SongActions, SongLine, Star, StepHead, Toast,
};
use crate::{Remote, backup, share, sse};

/// How many rows a page of the catalog holds.
///
/// Fifty, as the Go remote settled on. It is about a screen and a half on a phone, which is enough
/// to be worth scrolling and short enough that `Load more` arrives before the flick runs out.
const PAGE: usize = 50;

/// The most rows a restore will render in one go.
///
/// Coming back to the Songs tab renders every page up to the one holding the row you were on, so
/// that the row exists to be scrolled to — nothing server-side records that `Load more` appended
/// anything, so the alternative is the browser pressing that button three times while we try to set
/// a scroll under it.
///
/// Ten pages. A song `<li>` is roughly a kilobyte of markup once the inline YouTube path and the
/// action buttons are counted, so five hundred of them is about half a megabyte over a home LAN and
/// a few hundred milliseconds of layout on a phone — once, on a tab press that was reloading the
/// document anyway. On the server it is one query with `LIMIT 500` and one favorites lookup over
/// five hundred codes, which sits inside the budget [`MAX_PERSONAL`] already accepts. Past the cap
/// the row is not in the window, nothing scrolls, and you are at the top — which is what this tab
/// did before any of this existed.
pub(crate) const MAX_RESTORE: usize = 10 * PAGE;

/// The largest personal list assembled in memory.
///
/// A favorites folder is filtered and paged in Rust rather than in SQL — see [`BrowseQuery`]'s
/// note. This is the guard on that decision being reasonable: past it, the list is truncated and
/// said to be truncated, rather than the remote quietly reading ten thousand rows into a
/// phone-facing request.
const MAX_PERSONAL: usize = 2_000;

/// What a browse request can say.
///
/// Every field defaulted, because the tab bar links to a bare `/` and a missing parameter has to
/// mean "not set" rather than "bad request".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BrowseParams {
    /// Which list.
    #[serde(default)]
    pub mode: Option<String>,
    /// What is typed in the box.
    #[serde(default)]
    pub q: Option<String>,
    /// An ISO 639-1 code.
    #[serde(default)]
    pub language: Option<String>,
    /// The tags chosen, comma-joined — `rock,brasil`.
    ///
    /// **One scalar, never a repeated key**, and that is forced rather than preferred: `Query` here
    /// is `serde_urlencoded`, which answers `?tags=rock&tags=brasil` with a 400 — and htmx does not
    /// swap on an error, so the page would simply stop responding with nothing said. A comma cannot
    /// occur inside a slug, so the join is unambiguous, and one scalar survives the `km_browse`
    /// cookie's `encode`/`decode` untouched where a list would need a second escaping.
    #[serde(default)]
    pub tags: Option<String>,
    /// A tag being *added* to that set, from the picker.
    ///
    /// A second parameter rather than the picker being named `tags`, and it is the same discipline
    /// that makes the builder's bulk control `set_language` rather than `language`: the current set
    /// rides the form as a hidden `tags` field, so a picker sharing the name would put two `tags`
    /// keys in one request and 400. The handler merges this in and renders with it cleared.
    #[serde(default)]
    pub add_tag: Option<String>,
    /// An initial, or `#` for a digit.
    #[serde(default)]
    pub initial: Option<String>,
    /// The artist being browsed inside.
    #[serde(default)]
    pub artist: Option<String>,
    /// The folder being browsed inside.
    #[serde(default)]
    pub folder: Option<i64>,
    /// How far into the list.
    #[serde(default)]
    pub offset: Option<usize>,
    /// `browse`, `list`, `rows` or `extra`. Honored only for an htmx request.
    #[serde(default)]
    pub fragment: Option<String>,
    /// Present when the ⋯ box is now ticked, absent when it is not.
    ///
    /// **The presence is the value, and that is not a shortcut.** The control is a checkbox, and
    /// htmx sends a ticked one's `actions=1` and omits an unticked one — so a request that carries
    /// nothing is a phone saying *off*, told apart from every other request by `fragment=extra`.
    /// Reading the state the box now holds is what makes a second press correct: the old spelling
    /// baked `actions=0`/`actions=1` into a button's own URL, which only stayed right for as long as
    /// something re-rendered that button.
    ///
    /// A parameter on the browse route rather than a route of its own, because a route would be one
    /// more path to place on the machine's side of the admin prefix and one more thing to say about
    /// it — to say what nothing else here needs saying, that a preference on this phone touches no
    /// machine.
    #[serde(default)]
    pub actions: Option<String>,
}

impl BrowseParams {
    /// Whether this request said anything at all.
    ///
    /// A bare `/` is what tapping the Songs tab produces, and it is the signal to restore where
    /// somebody was. A URL that says something is itself the answer, and nothing is restored over it.
    fn is_bare(&self) -> bool {
        self.mode.is_none()
            && self.q.is_none()
            && self.language.is_none()
            && self.tags.is_none()
            && self.add_tag.is_none()
            && self.initial.is_none()
            && self.artist.is_none()
            && self.folder.is_none()
            && self.offset.is_none()
    }

    /// Parses a remembered browse state back into parameters.
    fn from_state(state: &str) -> Self {
        let mut params = Self::default();
        for pair in state.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let value = prefs::decode(value);
            match key {
                "mode" => params.mode = Some(value),
                "q" => params.q = Some(value),
                "language" => params.language = Some(value),
                "tags" => params.tags = Some(value),
                "initial" => params.initial = Some(value),
                "artist" => params.artist = Some(value),
                "folder" => params.folder = value.parse().ok(),
                _ => {}
            }
        }
        params
    }

    /// Which list this asks for, defaulting to songs.
    fn mode(&self, capabilities: crate::Capabilities) -> Mode {
        let mode = match self.mode.as_deref() {
            Some("artists") => Mode::Artists,
            Some("favorites") => Mode::Favorites,
            _ => Mode::Songs,
        };
        // A mode this build does not have is not an error — an old bookmark, or a link shared from a
        // phone running the other one. It lands on the songs list rather than on a page saying no.
        match mode {
            Mode::Favorites if !capabilities.favorites => Mode::Songs,
            other => other,
        }
    }

    /// The text in the box, with whitespace-only treated as empty.
    fn text(&self) -> String {
        self.q.clone().unwrap_or_default()
    }

    /// The chosen initial, if the mode allows one.
    fn initial(&self, capabilities: crate::Capabilities) -> Option<char> {
        if !capabilities.initial_filter {
            return None;
        }
        self.initial
            .as_deref()
            .and_then(|value| value.chars().next())
            .map(|initial| initial.to_ascii_uppercase())
    }

    /// The chosen language, blank treated as unset.
    fn language(&self) -> Option<String> {
        self.language
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    /// The tags chosen, folded, sorted and de-duplicated — with `add_tag` merged in.
    ///
    /// Merging here rather than in the handler is what makes the picker a *filter* and not a second
    /// state: every caller that asks what is being narrowed by gets the same answer, including the
    /// query builders that put it back into a link. `Tag::parse` runs on both, so a bookmark from a
    /// build that spelled a tag differently still lands on the slug the catalog holds.
    fn tags(&self) -> Vec<String> {
        let mut chosen = km_kmpkg::tag::parse_list(self.tags.as_deref().unwrap_or_default());
        if let Some(added) = self.add_tag.as_deref().and_then(km_kmpkg::Tag::parse)
            && !chosen.contains(&added)
        {
            chosen.push(added);
            chosen.sort();
        }
        chosen.into_iter().map(km_kmpkg::Tag::into_string).collect()
    }

    /// The artist being browsed inside.
    fn artist(&self) -> Option<String> {
        self.artist
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }
}

/// Whether this came from htmx rather than from the address bar.
fn is_htmx(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

/// The fragment this request wants, once the htmx check has had its say.
fn wanted_fragment(params: &BrowseParams, headers: &HeaderMap) -> Option<&'static str> {
    if !is_htmx(headers) {
        return None;
    }
    match params.fragment.as_deref() {
        Some("browse") => Some("browse"),
        Some("list") => Some("list"),
        Some("rows") => Some("rows"),
        // Not a piece of the page at all — the ⋯ preference, which is answered with a cookie and no
        // body. It is spelled as a fragment because it rides the same route and must be told apart
        // from an ordinary browse request that happens to carry no `actions`.
        Some("extra") => Some("extra"),
        _ => None,
    }
}

/// Something went wrong that is not a refusal — a database that will not open.
///
/// Distinct from a toast on purpose: a toast says "that did not happen", and this says "this page
/// cannot be drawn". They are different sentences and they belong in different places.
/// **The sentence is this crate's and the detail is the log's.** Rendering `error.to_string()`
/// would put a SQLite message naming a file and an offset in front of the person at the party —
/// genuinely useful, and useful to exactly one reader, who is not them. The `warn!` above it already carries that verbatim; what the
/// page says is a sentence in the language it is being read in.
fn failure(error: &RemoteError, locale: Locale) -> Response {
    tracing::warn!(%error, "the remote could not answer a request");
    views::page(
        &views::Failure {
            message: views::message_for(error, locale).text,
        },
        locale,
    )
}

/// Everything a full page needs around its content.
async fn chrome(state: &Remote, tab: &'static str, prefs: &Prefs) -> Chrome {
    let connection = state.machine.connection();
    let queue_len = state
        .machine
        .queue()
        .await
        .map(|queue| queue.len)
        .unwrap_or(0);
    Chrome {
        tab,
        capabilities: state.capabilities,
        queue_count: QueueCount { len: queue_len },
        conn: Conn {
            online: connection.online,
        },
        banner: Banner { connection },
        singer: prefs.singer.clone(),
        assets: crate::ASSET_VERSION,
        lang: prefs.locale.tag(),
        locales: views::LocaleChoice::all(prefs.locale),
        // This crate's version, which is the whole repository's — so it is the *host's* build that
        // is reported, the machine's or the offline app's, whichever mounted these pages. See
        // `Chrome::version`.
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// `GET /` — the Songs tab, in whichever of its four modes.
pub async fn browse(
    State(state): State<Remote>,
    Query(params): Query<BrowseParams>,
    headers: HeaderMap,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;

    // The ⋯ box was ticked or unticked. **Answered before anything is read, and with nothing at
    // all.** It changes how a row draws itself, not which rows there are, so the browser has
    // already revealed them by the time this arrives and there is no list here worth building — the
    // whole of the answer is the `Set-Cookie` that makes the next page load agree. It used to
    // re-render `#browse`, which threw away every page `Load more` had appended; see
    // `_song_actions.html`.
    if wanted_fragment(&params, &headers) == Some("extra") {
        return extra_preference(params.actions.is_some());
    }

    // Tapping the tab bar is a bare `/`, and it should come back to where you were rather than to
    // the top of a corpus. A URL that says anything is the answer and overrides this.
    //
    // `fragment` survives the restore, because which piece of the page to draw is not part of
    // *where you were*: a restore that dropped it would render a whole page where a fragment was
    // asked for.
    let bare = params.is_bare();
    let params = match (&bare, &prefs.browse) {
        (true, Some(state)) => BrowseParams {
            fragment: params.fragment.clone(),
            ..BrowseParams::from_state(state)
        },
        _ => params,
    };

    // What the cookie will say by the end of this, computed here because three things want it: the
    // list tag below, the anchor check, and the `Set-Cookie` at the bottom.
    let remembered = prefs::browse_state(
        params.mode(state.capabilities),
        &params.text(),
        params.language().as_deref(),
        &params.tags(),
        params.initial(state.capabilities),
        params.artist().as_deref(),
        params.folder,
    );
    let tag = prefs::list_tag(remembered.as_deref());

    // **Four conditions, and each one rules out a case that would otherwise misfire.** A URL that
    // says something is the answer, exactly as it is for `browse` above. An htmx fragment must never
    // carry a restore, which is what leaves `_browse.html`'s deliberate `show:window:top` after a
    // search alone. An anchor whose tag does not match belongs to a list somebody has since filtered
    // away. And a missing or malformed cookie is a first visit, which is the top of the list.
    let anchor = prefs
        .at
        .as_ref()
        .filter(|_| bare && wanted_fragment(&params, &headers).is_none())
        .filter(|anchor| anchor.list == tag);

    // Round the anchor up to a whole page, so the row it names is present with at most a page of
    // rows below it and `Load more` carries on from a page boundary. Note what this deliberately
    // does not do: it does not replay however many times somebody pressed that button. Ten pages
    // loaded and then a flick back to row 10 comes back as fifty rows, which is what they could see.
    let span = match anchor {
        Some(anchor) => (((anchor.at / PAGE) + 1) * PAGE).min(MAX_RESTORE),
        None => PAGE,
    };

    let block = match browse_block(&state, &params, &prefs, span, anchor.map(|a| a.row), &tag).await
    {
        Ok(block) => block,
        Err(error) => return failure(&error, locale),
    };

    let mut response = match wanted_fragment(&params, &headers) {
        Some("browse") => views::page(&block, locale),
        Some("list") => views::page(&block.list, locale),
        Some("rows") => views::page(&block.list.rows, locale),
        _ => {
            let chrome = chrome(&state, "browse", &prefs).await;
            views::page(
                &BrowsePage {
                    chrome,
                    browse: block,
                },
                locale,
            )
        }
    };

    // Remember where this left somebody. Cleared rather than stored when everything is back at its
    // defaults, so a phone that has been reset stops sending a cookie saying "normal".
    let cookie = match remembered {
        Some(state) => prefs::set_browse(&state),
        None => prefs::clear(prefs::BROWSE),
    };
    prefs::attach(response.headers_mut(), cookie);
    response
}

/// The ⋯ box, remembered for a year and answered with nothing to draw.
///
/// `204` rather than an empty `200`: there is genuinely no body, htmx swaps nothing either way, and
/// a status that says so is one fewer thing for a reader to wonder about.
///
/// `"0"` rather than clearing the cookie, because [`Prefs::read`] tests for `"1"` — so off is stored
/// as plainly as on, and a phone that has turned the extras off keeps them off. Only a press ever
/// reaches here, which is what keeps a `Set-Cookie` off the hundred other requests an evening makes.
fn extra_preference(on: bool) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    prefs::attach(
        response.headers_mut(),
        prefs::set_pref(prefs::EXTRA, if on { "1" } else { "0" }),
    );
    response
}

/// Builds the whole browse block for a request.
///
/// `span` is how many rows this page holds — [`PAGE`] for every ordinary request, and up to
/// [`MAX_RESTORE`] for the one tab press that is coming back to a row somebody was reading.
/// `anchor` is that row, rendered onto the list so the browser can find it, and `tag` stamps which
/// list these rows are, so an anchor captured in another one is never applied to them.
async fn browse_block(
    state: &Remote,
    params: &BrowseParams,
    prefs: &Prefs,
    span: usize,
    anchor: Option<SongCode>,
    tag: &str,
) -> Result<BrowseBlock, RemoteError> {
    let capabilities = state.capabilities;
    // Every sentence this block composes — the empty-list line and the count under it — is looked up
    // here rather than written out, for the reason `views::render` puts a catalog in the values
    // store: a page in Portuguese with an English sentence in the middle of it is the same fault
    // whether the English came from markup or from a `format!`.
    let words = crate::words::messages(prefs.locale);
    let mode = params.mode(capabilities);
    let text = params.text();
    let language = params.language();
    let initial = params.initial(capabilities);
    let artist = params.artist();
    let offset = params.offset.unwrap_or(0);

    let folder = match (params.folder, &state.favorites) {
        (Some(id), Some(favorites)) => favorites.folder(id).await?,
        _ => None,
    };

    // Which list, and what to say if it is empty. The four arms are genuinely four different
    // questions, which is why this is a match and not a parameterised query.
    // The fourth is what a folder could not place, which only the songs arm can produce and only
    // for a folder — see [`Listing`].
    let (rows, total, empty, unplaced) = match mode {
        Mode::Artists if artist.is_none() => {
            let artists = state
                .songs
                .artists(
                    Some(&text)
                        .filter(|t| !t.trim().is_empty())
                        .map(String::as_str),
                    &prefs.hidden_packages,
                )
                .await?;
            // **The letter is applied here rather than in SQL, and that is a measurement rather
            // than a shortcut.** Both catalogs already hand back every artist and this arm already
            // pages in Rust — `sort_artist` is a folded *name*, not a folded initial, so narrowing
            // in SQL would mean either a `LIKE 'a%'` on an unindexed column or a second indexed
            // column and a migration for each of the two databases. What it costs as written is one
            // `initial` call per artist over a list that has already been built.
            //
            // It reads `km_song::text::initial`, which is the same function the mirror stores for
            // song titles, so an artist files under the letter their name sorts under: `Águas` under
            // `A`, a numbered name under `#`.
            //
            // Inert online without a second check: `BrowseParams::initial` answers `None` when the
            // capability is off, and `Capabilities::initial_filter` is off there.
            let artists: Vec<_> = artists
                .into_iter()
                .filter(|row| {
                    initial.is_none_or(|want| km_song::text::initial(&row.name) == Some(want))
                })
                .collect();
            let total = artists.len();
            let page: Vec<ArtistLine> = artists
                .into_iter()
                .skip(offset)
                .take(span)
                .map(|row| ArtistLine {
                    count: song_count(words, row.songs),
                    row,
                })
                .collect();
            let more = offset + page.len() < total;
            (
                RowsBlock {
                    kind: "artists",
                    songs: Vec::new(),
                    artists: page,
                    folders: Vec::new(),
                    next: more.then(|| next_query(params, offset + span)),
                    favorites: capabilities.favorites,
                    in_folder: None,
                    context: String::new(),
                    list_tag: tag.to_owned(),
                    anchor: None,
                },
                Some(total),
                empty_text(words, "artists", &text, initial, None),
                Vec::new(),
            )
        }
        Mode::Favorites if folder.is_none() => {
            let folders = match &state.favorites {
                Some(favorites) => favorites.folders().await?,
                None => Vec::new(),
            };
            let needle = fold(&text);
            let matching: Vec<FolderRow> = folders
                .into_iter()
                .filter(|f| needle.is_empty() || fold(&f.name).contains(&needle))
                .collect();
            let total = matching.len();
            let page: Vec<FolderLine> = matching
                .into_iter()
                .skip(offset)
                .take(span)
                .map(|row| FolderLine {
                    count: song_count(words, row.songs),
                    row,
                })
                .collect();
            let more = offset + page.len() < total;
            (
                RowsBlock {
                    kind: "folders",
                    songs: Vec::new(),
                    artists: Vec::new(),
                    folders: page,
                    next: more.then(|| next_query(params, offset + span)),
                    favorites: capabilities.favorites,
                    in_folder: None,
                    context: String::new(),
                    list_tag: tag.to_owned(),
                    anchor: None,
                },
                Some(total),
                empty_text(words, "folders", &text, None, None),
                Vec::new(),
            )
        }
        _ => {
            let Listing { page, unplaced } =
                songs_for(state, params, prefs, mode, &folder, offset, span).await?;
            let context = context_query(params, mode);
            let starred = starred_set(state, &page.songs).await;
            let lines = page
                .songs
                .iter()
                .map(|song| {
                    let number = song.number;
                    SongLine {
                        row: SongRow {
                            song: song.clone(),
                            starred: starred.contains(&number),
                        },
                        star: capabilities.favorites.then(|| Star {
                            number,
                            on: starred.contains(&number),
                            oob: false,
                        }),
                        actions: SongActions {
                            number,
                            in_folder: folder.as_ref().map(|f| f.id),
                            context: context.clone(),
                            confirm: words
                                .msg_with(
                                    "song-unfavorite-confirm",
                                    &[("title", song.title.as_str().into())],
                                )
                                .into_owned(),
                        },
                    }
                })
                .collect();
            let empty = empty_text(
                words,
                "songs",
                &text,
                initial,
                folder.as_ref().map(|f| f.name.as_str()),
            );
            // Somebody who hid a package and forgot sees a search come back empty, so the sentence
            // says where the rest went. Not in a folder, which a hide does not narrow.
            let empty = if folder.is_none() && !prefs.hidden_packages.is_empty() {
                format!("{empty} {}", words.msg("empty-hidden-packages"))
            } else {
                empty
            };
            (
                RowsBlock {
                    kind: "songs",
                    songs: lines,
                    artists: Vec::new(),
                    folders: Vec::new(),
                    next: page.more.then(|| next_query(params, offset + span)),
                    favorites: capabilities.favorites,
                    in_folder: folder.as_ref().map(|f| f.id),
                    context,
                    list_tag: tag.to_owned(),
                    anchor,
                },
                page.total,
                empty,
                // The count first and the remedies under it, which is the order the restore report
                // reads in: how many, then what to do about the ones something can be done about.
                not_here_lines(words, &unplaced),
            )
        }
    };

    // The language picker is only worth drawing where there is a choice to make. A catalog whose
    // songs are all in one language would otherwise carry a select with one option in it.
    //
    // **Unconditional, and it always was.** This read `if capabilities.initial_filter || true`, which
    // is a condition with no false arm — so the `else` was unreachable and the capability named in
    // it decided nothing. The picker is wanted in both modes, which is what the `|| true` was
    // reaching for and what this now says outright; restoring a `initial_filter` gate would take the
    // language picker away from the machine's own remote, which is a different feature and not one
    // anybody asked to remove.
    let languages = {
        let all = state
            .songs
            .languages(&prefs.hidden_packages)
            .await
            .unwrap_or_default();
        if all.len() > 1 { all } else { Vec::new() }
    };

    // The tag picker offers what is *not* already chosen — an option that changes nothing is not a
    // choice. **Not the `len() > 1` rule the languages take**, and the asymmetry is real: a language
    // is one of a closed set and every song has at most one, so a catalog with one language has no
    // choice to make; a song has many tags, so a catalog with exactly one tag still has two states
    // worth being in. What makes the picker vanish here is having nothing left to add.
    let chosen = params.tags();
    let tag_choices: Vec<_> = state
        .songs
        .tags(&prefs.hidden_packages)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|row| !chosen.contains(&row.tag))
        .collect();

    Ok(BrowseBlock {
        capabilities,
        mode,
        query: text,
        language,
        languages,
        tags: chosen,
        tag_choices,
        initial,
        artist,
        folder,
        extra: prefs.extra,
        package_problems: state.machine.package_problems().await,
        list: ListBlock {
            empty,
            total,
            summary: list_summary(words, rows.len(), total),
            rows,
            not_here: unplaced,
        },
    })
}

/// The songs for a browse request, from whichever source the mode implies.
///
/// Two sources, and only the first goes near the catalog's search. A favorites folder is a
/// bounded personal list, so it is fetched by number and filtered here — see the note on
/// [`BrowseQuery`].
/// A page of rows, and what a folder could not place to produce it.
///
/// **A second field rather than one on [`SongPage`]**, which is what `Songs::search` answers with:
/// every catalog would then carry a vector it has nothing to put in, to serve the one mode that
/// resolves favorites rather than searching.
struct Listing {
    page: SongPage,
    /// The favorites this catalog cannot show, and why — empty in every mode but a folder.
    unplaced: Vec<(SongRef, Miss)>,
}

impl From<SongPage> for Listing {
    fn from(page: SongPage) -> Self {
        Self {
            page,
            unplaced: Vec::new(),
        }
    }
}

/// **A hidden package narrows only the search.** A favorites folder is songs somebody chose one by
/// one, and a typed song number names exactly one song, so both still find a song whose package this
/// phone hides.
async fn songs_for(
    state: &Remote,
    params: &BrowseParams,
    prefs: &Prefs,
    mode: Mode,
    folder: &Option<FolderRow>,
    offset: usize,
    span: usize,
) -> Result<Listing, RemoteError> {
    let text = params.text();
    let language = params.language();
    let tags = params.tags();
    let initial = params.initial(state.capabilities);
    // Whether the whole folder is what is on screen, which is what lets it be spoken about.
    let unfiltered =
        text.trim().is_empty() && language.is_none() && tags.is_empty() && initial.is_none();

    // A folder: fetch by what each favorite is, filter and page here.
    let refs: Option<Vec<SongRef>> = match (mode, folder) {
        (Mode::Favorites, Some(folder)) => match &state.favorites {
            Some(favorites) => Some(favorites.song_refs(folder.id).await?),
            None => Some(Vec::new()),
        },
        _ => None,
    };

    if let Some(mut refs) = refs {
        let truncated = refs.len() > MAX_PERSONAL;
        if truncated {
            tracing::warn!(
                held = refs.len(),
                shown = MAX_PERSONAL,
                "this list is longer than the remote assembles in memory; showing the first part"
            );
            refs.truncate(MAX_PERSONAL);
        }
        let resolved = state.songs.resolve(&refs).await?;
        // **The repair, and it lives here because this is where somebody looks.** Drawing a folder
        // is the one thing that happens often and against a fresh mirror, so it is where a favorite
        // filed before these columns existed picks up its package and hash, and where one whose
        // package has been re-banked is moved to the number it is under now. Nothing is removed —
        // see `Favorites::reconcile`.
        if let Some(favorites) = &state.favorites {
            let repairs: Vec<_> = resolved.iter().filter_map(Reconciliation::of).collect();
            if !repairs.is_empty() {
                let moved = repairs.iter().filter(|row| row.was != row.now).count();
                if moved > 0 {
                    tracing::info!(
                        moved,
                        "favorites whose songs are under new numbers; refiling them"
                    );
                }
                favorites.reconcile(&repairs).await?;
            }
        }
        // **Collected before the filters below, because they are a fact about the folder rather
        // than about a search.** A favorite that did not resolve was never a row this list could
        // narrow, so counting it against the whole folder is the only reading that holds.
        let unplaced: Vec<(SongRef, Miss)> = if unfiltered {
            resolved
                .iter()
                .filter_map(|row| {
                    row.outcome
                        .as_ref()
                        .err()
                        .map(|miss| (row.asked.clone(), *miss))
                })
                .collect()
        } else {
            // Narrowed, so the folder as a whole is not what is on screen and a sentence about it
            // would read as a statement about the search.
            Vec::new()
        };
        let songs: Vec<_> = resolved
            .iter()
            .filter_map(Resolution::song)
            .cloned()
            .collect();
        let needle = fold(&text);
        let matching: Vec<_> = songs
            .into_iter()
            .filter(|song| {
                if !needle.is_empty() {
                    let haystack = format!(
                        "{} {}",
                        fold(&song.title),
                        song.artist.as_deref().map(fold).unwrap_or_default()
                    );
                    if !haystack.contains(&needle) {
                        return false;
                    }
                }
                if let Some(language) = &language
                    && song.language.as_deref() != Some(language.as_str())
                {
                    return false;
                }
                if let Some(initial) = initial
                    && km_song::text::initial(&song.title) != Some(initial)
                {
                    return false;
                }
                // Any tag, not every — the same OR the catalog makes, in memory. Both sides are
                // slugs by now: `BrowseParams::tags` folded the query and packaging folded the song.
                //
                // **The emptiness is checked first**, because no tags at all is no tag filter, where
                // an `any` over nothing is nothing.
                if !tags.is_empty()
                    && !tags
                        .iter()
                        .any(|wanted| song.tags.iter().any(|held| held == wanted))
                {
                    return false;
                }
                true
            })
            .collect();
        let total = matching.len();
        let page: Vec<_> = matching.into_iter().skip(offset).take(span).collect();
        let more = offset + page.len() < total;
        return Ok(Listing {
            page: SongPage {
                songs: page,
                total: Some(total),
                more,
            },
            unplaced,
        });
    }

    // **A search that is exactly a song code finds that song.** The box has said "Song or number"
    // since M14 and never delivered it: the text went straight to an FTS match over title and
    // artist, so typing `10234` found songs with `10234` in their *name* and nothing else. Now that
    // a code is a string with its own grammar it can simply be tried — and only an exact, whole-text
    // match counts, so `500` still searches for the word as well when no song has that code.
    if artist_free(params)
        && let Ok(code) = text.trim().parse::<SongCode>()
        && let Ok(Some(song)) = state.songs.song(code).await
    {
        return Ok(SongPage {
            songs: vec![song],
            total: Some(1),
            more: false,
        }
        .into());
    }

    let artist = params.artist().map(ArtistFilter::Exactly);
    state
        .songs
        .search(&BrowseQuery {
            text: Some(text).filter(|t| !t.trim().is_empty()),
            artist,
            language,
            tags,
            initial,
            hidden_packages: prefs.hidden_packages.clone(),
            order: if params.q.as_deref().is_some_and(|q| !q.trim().is_empty()) {
                Order::Best
            } else {
                Order::Title
            },
            limit: span,
            offset,
        })
        .await
        .map(Listing::from)
}

/// Whether this request narrows to nothing but the text.
///
/// A code lookup answers with one song, so it must not quietly ignore a filter somebody set: inside
/// an artist, or with a language chosen, the text stays a search.
fn artist_free(params: &BrowseParams) -> bool {
    params.artist().is_none()
        && params.language().is_none()
        && params.tags().is_empty()
        && params.initial.is_none()
}

/// Which of these songs this phone has filed somewhere.
///
/// One question for a whole page rather than one per row — the difference between one round trip and
/// fifty, and the shape `favdb.FavoritedIDs` landed on for the same reason.
async fn starred_set(state: &Remote, songs: &[km_api::dto::SongDto]) -> HashSet<SongCode> {
    let Some(favorites) = &state.favorites else {
        return HashSet::new();
    };
    let numbers: Vec<SongCode> = songs.iter().map(|song| song.number).collect();
    favorites.favorited(&numbers).await.unwrap_or_default()
}

/// The query string for the next page.
fn next_query(params: &BrowseParams, offset: usize) -> String {
    let mut parts = Vec::new();
    if let Some(mode) = &params.mode {
        parts.push(format!("mode={}", prefs::encode(mode)));
    }
    if let Some(q) = &params.q {
        parts.push(format!("q={}", prefs::encode(q)));
    }
    if let Some(language) = &params.language {
        parts.push(format!("language={}", prefs::encode(language)));
    }
    // The merged set, so a link built from this view carries a tag just added from the picker.
    // Rebuilt rather than copied from `params.tags`: `add_tag` is not in it yet.
    let chosen = params.tags();
    if !chosen.is_empty() {
        parts.push(format!("tags={}", prefs::encode(&chosen.join(","))));
    }
    if let Some(initial) = &params.initial {
        parts.push(format!("initial={}", prefs::encode(initial)));
    }
    if let Some(artist) = &params.artist {
        parts.push(format!("artist={}", prefs::encode(artist)));
    }
    if let Some(folder) = params.folder {
        parts.push(format!("folder={folder}"));
    }
    parts.push(format!("offset={offset}"));
    parts.join("&")
}

/// The query string that names this view, for an action that has to come back to it.
fn context_query(params: &BrowseParams, mode: Mode) -> String {
    let mut parts = vec![format!("mode={}", mode.as_str())];
    if let Some(q) = &params.q {
        parts.push(format!("q={}", prefs::encode(q)));
    }
    if let Some(language) = &params.language {
        parts.push(format!("language={}", prefs::encode(language)));
    }
    // The merged set, so a link built from this view carries a tag just added from the picker.
    // Rebuilt rather than copied from `params.tags`: `add_tag` is not in it yet.
    let chosen = params.tags();
    if !chosen.is_empty() {
        parts.push(format!("tags={}", prefs::encode(&chosen.join(","))));
    }
    if let Some(initial) = &params.initial {
        parts.push(format!("initial={}", prefs::encode(initial)));
    }
    if let Some(artist) = &params.artist {
        parts.push(format!("artist={}", prefs::encode(artist)));
    }
    if let Some(folder) = params.folder {
        parts.push(format!("folder={folder}"));
    }
    parts.join("&")
}

/// What to say when a list has nothing in it.
///
/// What a move or a refresh did, as a sentence.
///
/// **The words are here and the facts came from `km-remote-core`.** Those three methods used to
/// answer a finished `Now using {url}. Copied {n} songs from the machine.`, composed in a crate with
/// no catalog — so the one sentence a person sees after pressing *Use this one* was English
/// whatever language the rest of the page was in. See [`crate::machine::Copied`].
fn copied_sentence(locale: Locale, copied: &crate::machine::Copied) -> String {
    use crate::machine::CopyOutcome;

    let words = crate::words::messages(locale);
    let outcome = match copied.outcome {
        CopyOutcome::AlreadyCurrent(songs) => {
            words.msg_with("machine-copy-current", &[("count", (songs as i64).into())])
        }
        CopyOutcome::Imported(songs) => {
            words.msg_with("machine-copy-imported", &[("count", (songs as i64).into())])
        }
        CopyOutcome::NotAnswering => words.msg("machine-copy-not-answering"),
    };
    match &copied.moved_to {
        Some(url) => format!(
            "{} {outcome}",
            words.msg_with("machine-now-using", &[("url", url.as_str().into())])
        ),
        None => outcome.into_owned(),
    }
}

/// One message, in the language of the request being answered.
///
/// The short form of `words::messages(locale).msg(key)`, which this file does thirty times — once
/// for every sentence a `Toast::` call would otherwise carry as a string literal.
fn say(locale: Locale, key: &str) -> String {
    crate::words::messages(locale).msg(key).into_owned()
}

/// `12 songs`, under an artist or a folder.
///
/// The same message the list count uses, because it is the same sentence — a number of songs — and
/// two ids would be two chances to translate it differently.
fn song_count(words: &Catalog, songs: usize) -> String {
    words
        .msg_with("count-songs", &[("count", (songs as i64).into())])
        .into_owned()
}

/// `112 shown`, or `50 of 112` where the total was cheap to know.
///
/// Beside [`empty_text`] because it is the same kind of thing: two numbers and a word, which is a
/// sentence and not markup. The `shown` arm is a plural in Portuguese and reads as one word there
/// rather than two, which is exactly what a catalog is for.
fn list_summary(words: &Catalog, showing: usize, total: Option<usize>) -> String {
    match total {
        Some(total) if total != showing => words
            .msg_with(
                "list-count-of",
                &[
                    ("showing", (showing as i64).into()),
                    ("total", (total as i64).into()),
                ],
            )
            .into_owned(),
        _ => words
            .msg_with("list-count-shown", &[("count", (showing as i64).into())])
            .into_owned(),
    }
}

/// Composed here rather than branched in markup: it is three kinds of list times four reasons, which
/// is a table. "Nothing matches" on its own is the unhelpful version — what somebody needs to know is
/// *which* of the things they have set is the one hiding everything.
///
/// **Every arm goes through the catalog**, and the two that interpolate go through it with arguments
/// rather than around it with `format!` — which is the rule the filters module states: markup carries
/// a key, and anything that puts a value into a sentence is composed where a test can reach it.
fn empty_text(
    words: &Catalog,
    kind: &str,
    query: &str,
    initial: Option<char>,
    folder: Option<&str>,
) -> String {
    let query = query.trim();
    let initial = initial.map(|initial| initial.to_string());
    let sentence = match (kind, query.is_empty(), initial.as_deref(), folder) {
        ("folders", true, _, _) => words.msg("empty-folders"),
        ("folders", false, _, _) => {
            words.msg_with("empty-folders-search", &[("query", query.into())])
        }
        // **The letter arms come first, because the wildcard ones below them would swallow these.**
        // Before the A–Z picker reached the artists list there was nothing to swallow; now
        // `("artists", true, Some('Q'), _)` matching `("artists", true, _, _)` would answer "nobody
        // has been tagged with an artist" for a corpus with several thousand of them.
        ("artists", true, Some(initial), _) => {
            words.msg_with("empty-artists-initial", &[("initial", initial.into())])
        }
        ("artists", false, Some(initial), _) => words.msg_with(
            "empty-artists-initial-search",
            &[("initial", initial.into()), ("query", query.into())],
        ),
        ("artists", true, None, _) => words.msg("empty-artists"),
        ("artists", false, None, _) => {
            words.msg_with("empty-artists-search", &[("query", query.into())])
        }
        (_, true, Some(initial), Some(folder)) => words.msg_with(
            "empty-folder-initial",
            &[("folder", folder.into()), ("initial", initial.into())],
        ),
        (_, true, Some(initial), None) => {
            words.msg_with("empty-initial", &[("initial", initial.into())])
        }
        (_, true, None, Some(folder)) => {
            words.msg_with("empty-folder", &[("folder", folder.into())])
        }
        (_, true, None, None) => words.msg("empty-songs"),
        (_, false, Some(initial), _) => words.msg_with(
            "empty-initial-search",
            &[("initial", initial.into()), ("query", query.into())],
        ),
        (_, false, None, Some(folder)) => words.msg_with(
            "empty-folder-search",
            &[("folder", folder.into()), ("query", query.into())],
        ),
        (_, false, None, None) => words.msg_with("empty-search", &[("query", query.into())]),
    };
    sentence.into_owned()
}

/// `GET /now`
pub async fn now(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let chrome = chrome(&state, "now", &prefs).await;
    let player = player_block(&state).await;
    views::page(&NowPage { chrome, player }, locale)
}

/// `GET /setup`
///
/// The machine card and the two preferences. It asks the state for nothing the Now tab did not
/// already ask it for; what makes it a page of its own is that none of it is about the song.
pub async fn setup(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let chrome = chrome(&state, "setup", &prefs).await;
    let machine = machine_block(&state, locale).await;
    let packages_tab = package_settings(&state, &prefs).await.len() > 1;
    views::page(
        &SetupPage {
            chrome,
            machine,
            packages_tab,
        },
        locale,
    )
}

/// `GET /setup/packages` — the Setup tab's second page: which packages this phone searches.
pub async fn setup_packages(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let chrome = chrome(&state, "setup", &prefs).await;
    let packages = package_settings(&state, &prefs).await;
    views::page(&PackagesPage { chrome, packages }, locale)
}

/// Every package the catalog holds, and whether this phone shows it.
///
/// A catalog that will not answer gives no packages rather than failing the page, which on `/setup`
/// also holds the singer's name and the machine card.
async fn package_settings(state: &Remote, prefs: &Prefs) -> Vec<PackageSetting> {
    let words = crate::words::messages(prefs.locale);
    state
        .songs
        .packages()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| PackageSetting {
            shown: !prefs.hidden_packages.contains(&row.id),
            uncurated: row.flags.is_uncurated(),
            songs: words
                .msg_with("count-songs", &[("count", row.songs.into())])
                .into_owned(),
            id: row.id,
            name: row.name,
        })
        .collect()
}

/// `POST /packages/hidden` — which packages this phone leaves out of its song list.
///
/// The form posts every package it listed as `listed` and every ticked box as `show`. A listed
/// package that is not shown is hidden. **An id the form did not list keeps its place in the
/// cookie**, so a package that is away from this catalog, or from this machine, stays hidden when it
/// comes back.
///
/// Answers with a toast and the cookie, as [`set_singer`] does: the list it changes is on another
/// tab, which reads the cookie the next time it draws.
pub async fn set_hidden_packages(headers: HeaderMap, body: String) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let listed = form::values(&body, "listed");
    let shown = form::values(&body, "show");
    let kept = prefs
        .hidden_packages
        .iter()
        .filter(|id| !listed.contains(id))
        .map(String::as_str);
    let chosen = listed
        .iter()
        .filter(|id| !shown.contains(id))
        .map(String::as_str);
    let hidden = prefs::hidden_packages(kept.chain(chosen));

    let toast = if hidden.is_empty() {
        Toast::info(say(locale, "packages-all-shown"))
    } else {
        Toast::good(say(locale, "packages-hidden-saved"))
    };
    let mut response = views::toast_only(toast, locale);
    prefs::attach(response.headers_mut(), prefs::set_hidden(&hidden));
    response
}

/// The machine card as it stands, or `None` where there is no machine to choose.
///
/// The `Option` is the mode, and it is asked of [`Remote::connect`] rather than of
/// [`crate::Capabilities`]: the capability says whether the card belongs on the page and this says
/// whether anything can answer for it, and a build that turned the first on without the second would
/// otherwise render a card full of nothing.
async fn machine_block(state: &Remote, locale: Locale) -> Option<MachineBlock> {
    Some(MachineBlock::of(machine_status(state).await?, locale))
}

/// Which machine this device is talking to, before any of it is worded.
///
/// Split from [`machine_block`] for the fan-out, which needs one status turned into one card per
/// language rather than one card — see [`everywhere`].
async fn machine_status(state: &Remote) -> Option<crate::machine::MachineStatus> {
    Some(state.connect.as_ref()?.status().await)
}

/// What the machine says it is doing, or the placeholder for one that will not answer.
///
/// A machine that cannot be reached is not a failure here — it is an idle card and a banner. The
/// offline app spends most of its life in exactly this state.
async fn player_view(state: &Remote) -> PlayerView {
    let connection = state.machine.connection();
    match state.machine.state().await {
        Ok(snapshot) => PlayerView {
            state: snapshot,
            online: connection.online,
        },
        Err(_) => PlayerView::unreachable(),
    }
}

/// The player card as it stands.
async fn player_block(state: &Remote) -> PlayerBlock {
    PlayerBlock::of(player_view(state).await)
}

/// The Queue page's now bar as it stands.
async fn nowbar_block(state: &Remote) -> NowBar {
    NowBar::of(player_view(state).await)
}

/// `GET /queue`
pub async fn queue_page(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let chrome = chrome(&state, "queue", &prefs).await;
    let nowbar = nowbar_block(&state).await;
    let queue = queue_block(&state).await;
    views::page(
        &QueuePage {
            chrome,
            nowbar,
            queue,
        },
        locale,
    )
}

/// The queue list as it stands.
async fn queue_block(state: &Remote) -> QueueBlock {
    let online = state.machine.connection().online;
    let entries = state
        .machine
        .queue()
        .await
        .map(|queue| queue.entries)
        .unwrap_or_default();
    let last = entries.len().saturating_sub(1);
    QueueBlock {
        rows: entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| QueueRow {
                entry,
                first: index == 0,
                last: index == last,
            })
            .collect(),
        online,
    }
}

/// `GET /events`
///
/// **Opening a stream is also a request to try the machine again**, and that is the whole of the
/// wake seam — there is no route for it and there should not be. A page opens a stream when it
/// loads, when a tab press replaces the document, and when it comes back on screen after a
/// background; every one of those is somebody looking at the remote again, which is the moment
/// worth spending an attempt on. A separate `POST` would have had to be called from all three, would
/// cost a second request against the six-connection budget the whole of `live.js` exists to protect,
/// and could drift out of step with the reopen. Here the reopen *is* the request.
///
/// The `!online` gate lives here rather than in [`Machine::wake`](crate::machine::Machine::wake), so
/// that the trait method stays safe to call at any time and both branches of this decision are
/// observable from a test.
pub async fn events(State(state): State<Remote>, headers: HeaderMap) -> Response {
    if !state.machine.connection().online {
        state.machine.wake();
    }
    // **The stream is opened in the viewer's language**, which is the half of the fan-out the pump
    // cannot supply: it renders every fragment in every language and this is where one page's copies
    // are picked out. The same cookie every other route reads, so a device that chose Portuguese on
    // the Queue tab keeps it in what arrives over the stream a second later.
    let response = state.hub.stream(prefs::locale(&headers)).into_response();
    // **The count, not just the fact.** A browser allows about six connections to one host, and a
    // stream holds one for as long as its page does — so streams piling up is the failure that
    // presents as "the remote hangs for half a minute" with nothing wrong on either side of it.
    // Counted after subscribing, so this is the number now listening rather than the number before.
    // Two open pages is ordinary; a number that climbs as somebody presses tabs is the fault.
    tracing::debug!(listeners = state.hub.listeners(), "an event stream opened");
    response
}

/// `POST /singer`
pub async fn set_singer(headers: HeaderMap, body: String) -> Response {
    let locale = prefs::locale(&headers);
    let name = form::field(&body, "singer")
        .as_deref()
        .and_then(prefs::tidy_singer);
    let toast = match &name {
        Some(name) => Toast::good(
            crate::words::messages(locale)
                .msg_with("singer-set", &[("name", name.as_str().into())]),
        ),
        None => Toast::info(say(locale, "singer-cleared")),
    };
    // **A toast and nothing else.** Answering with the queue would look like keeping the rows in
    // step and would not be: a row prints the singer the *machine* recorded on that entry, not the
    // name in this cookie, so the swap would change nothing anybody could see. The name
    // applies to what you queue next, and the toast is what says so.
    let mut response = views::toast_only(toast, locale);
    let cookie = match &name {
        Some(name) => prefs::set_pref(prefs::SINGER, name),
        None => prefs::clear(prefs::SINGER),
    };
    prefs::attach(response.headers_mut(), cookie);
    response
}

/// `POST /locale` — what language this device reads the remote in.
///
/// **Answers with a full refresh rather than a fragment**, which is the one place this page departs
/// from htmx-swaps-a-piece. Every word on the document changes, including the tab bar and
/// `<html lang>`, so there is no target that would be right; `HX-Refresh` is htmx's own way of
/// saying "load the page again", and the cookie set here is what the reload reads.
///
/// A tag this build has no catalog for is ignored rather than refused: it can only come from a
/// hand-made request, and the page it would refuse is the page somebody is reading.
pub async fn set_locale(body: String) -> Response {
    let Some(chosen) = form::field(&body, "locale")
        .as_deref()
        .and_then(Locale::parse)
    else {
        // Nothing changed and nothing to say. A toast here would be a sentence about a request no
        // browser makes — the control is a `<select>` of exactly the tags this build has.
        return StatusCode::NO_CONTENT.into_response();
    };
    let mut response = (StatusCode::OK, [("hx-refresh", "true")]).into_response();
    prefs::attach(response.headers_mut(), prefs::set_locale(chosen));
    response
}

// -- which machine ------------------------------------------------------------------------------
//
// Three actions, one answer shape: the card as it now stands, plus a toast saying what happened.
// **None of them returns a non-2xx for an ordinary refusal** — an address nothing answers at, a
// browse that finds nothing — because htmx does not swap an error response, so the button would go
// visually dead and the reason would appear nowhere. That is the same rule `views::message_for`
// records, applied to the one part of this page that changes where the remote is pointing.

/// The card, the offer block, and something to say. The shape all four actions answer with.
///
/// `found` is the address a rescan turned up and left alone; every other answer passes `None`, which
/// is what empties the block. Passing it on *every* answer rather than only on a rescan is the point:
/// an offer that outlived the press that acted on it would sit there recommending a machine this
/// device has since moved to.
async fn machine_card(
    state: &Remote,
    toast: Toast,
    found: Vec<crate::machine::Offer>,
    locale: Locale,
) -> Response {
    match machine_block(state, locale).await {
        Some(card) => {
            views::with_toast_and_oob(&card, &views::MachineFound { offers: found }, toast, locale)
        }
        // Unreachable through the router — the routes are only useful in a build that has a
        // `connect` — but saying so beats rendering an empty card.
        None => views::toast_only(Toast::warn(say(locale, NO_CHOICE)), locale),
    }
}

/// The message id every one of the four uses when this build has no `connect` to ask.
pub(crate) const NO_CHOICE: &str = "machine-not-chooseable";

/// `POST /machine/connect` — use the address somebody typed, and keep it.
pub async fn connect_machine(
    State(state): State<Remote>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let locale = prefs::locale(&headers);
    let Some(connect) = state.connect.clone() else {
        return machine_card(
            &state,
            Toast::warn(say(locale, NO_CHOICE)),
            Vec::new(),
            locale,
        )
        .await;
    };
    // `form::field` folds empty into absent, so a box somebody cleared and a box they never filled
    // are one case here — which is the same thing to say about both.
    let Some(address) = form::field(&body, "address") else {
        return machine_card(
            &state,
            Toast::warn(say(locale, "machine-type-address")),
            Vec::new(),
            locale,
        )
        .await;
    };

    let toast = match connect.connect_to(&address).await {
        Ok(copied) => Toast::good(copied_sentence(locale, &copied)),
        Err(error) => views::message_for(&error, locale),
    };
    machine_card(&state, toast, Vec::new(), locale).await
}

/// `POST /machine/rescan` — look on the network now, and stop keeping whatever was named.
///
/// **Four answers, and only one of them moves anything.** A remote with a machine answering keeps it
/// and is shown what else is out there; the offer goes into `#machine-found` and waits for a press.
/// See [`Scanned`](crate::machine::Scanned) for why looking and switching are two acts.
pub async fn rescan_machine(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let locale = prefs::locale(&headers);
    let Some(connect) = state.connect.clone() else {
        return machine_card(
            &state,
            Toast::warn(say(locale, NO_CHOICE)),
            Vec::new(),
            locale,
        )
        .await;
    };
    let (toast, found) = match connect.rescan().await {
        Ok(scan) => {
            let others = scan.others;
            let words = crate::words::messages(locale);
            // What else answered, said once and appended to whichever sentence the outcome produced
            // — because that is the fact this button used to drop. The offers themselves are below
            // the card; this only says how many there are to look at.
            //
            // A plural over a count, so it is a Fluent selector rather than a `match` on three arms
            // in Rust: `one` and `other` are not the same set of cases in every language, and
            // choosing between them is the catalog's job.
            let and_more = if others.is_empty() {
                String::new()
            } else {
                format!(
                    " {}",
                    words.msg_with(
                        "machine-more-answered",
                        &[("count", (others.len() as i64).into())],
                    )
                )
            };
            let toast = match scan.outcome {
                // Named where the machine said so, because "Found Living Room." is the sentence
                // somebody pressing this wants and an address is the one they can already see on the
                // card below. The rescan refreshes before it returns, so the name is in hand by the
                // time this reads it; a machine that advertises none falls back to what this always
                // said.
                Scanned::Using(url) => {
                    let said = match connect.status().await.connection.name {
                        Some(name) => words.msg_with(
                            "machine-found-named",
                            &[("name", name.as_str().into()), ("url", url.as_str().into())],
                        ),
                        None => words.msg_with("machine-found", &[("url", url.as_str().into())]),
                    };
                    Toast::good(format!("{said}{and_more}"))
                }
                // Not a failure, and worded so it does not read as one: the pin has still been
                // cleared, which is half of what somebody pressing this asked for. The same is true
                // of the two below.
                Scanned::Nothing => Toast::info(say(locale, "machine-scan-nothing")),
                // **The sentence the reported fault was made of.** It used to end here, with the
                // other machines discarded — so a house with two machines was told it had one.
                Scanned::Already(_) => {
                    Toast::info(format!("{}{and_more}", words.msg("machine-scan-already")))
                }
                // **The answer for a remote whose own machine is not on the network**, and every
                // stranger that is arrives in `and_more` and in the offers below. Worded as
                // `Nothing`'s case rather than given a sentence of its own, since the connection is
                // untouched either way and the list is what the press earned.
                Scanned::Kept => {
                    Toast::info(format!("{}{and_more}", words.msg("machine-scan-kept")))
                }
            };
            (toast, others)
        }
        Err(error) => (views::message_for(&error, locale), Vec::new()),
    };
    machine_card(&state, toast, found, locale).await
}

/// `POST /machine/use` — move to an address a rescan offered, without pinning it.
pub async fn use_machine(
    State(state): State<Remote>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let locale = prefs::locale(&headers);
    let Some(connect) = state.connect.clone() else {
        return machine_card(
            &state,
            Toast::warn(say(locale, NO_CHOICE)),
            Vec::new(),
            locale,
        )
        .await;
    };
    // Absent means the offer was pressed after the page had moved on, which is not worth a sentence
    // of its own: the block is emptied and the card says where this device actually is.
    let Some(address) = form::field(&body, "address") else {
        return machine_card(
            &state,
            Toast::info(say(locale, "machine-offer-gone")),
            Vec::new(),
            locale,
        )
        .await;
    };

    let toast = match connect.use_found(&address).await {
        Ok(copied) => Toast::good(copied_sentence(locale, &copied)),
        Err(error) => views::message_for(&error, locale),
    };
    machine_card(&state, toast, Vec::new(), locale).await
}

/// `POST /machine/refresh` — re-read the song list from whichever machine is in hand.
pub async fn refresh_machine(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let locale = prefs::locale(&headers);
    let Some(connect) = state.connect.clone() else {
        return machine_card(
            &state,
            Toast::warn(say(locale, NO_CHOICE)),
            Vec::new(),
            locale,
        )
        .await;
    };
    let toast = match connect.refresh().await {
        Ok(copied) => Toast::good(copied_sentence(locale, &copied)),
        Err(error) => views::message_for(&error, locale),
    };
    machine_card(&state, toast, Vec::new(), locale).await
}

/// Whether a just-queued song is the one the machine has loaded.
///
/// **This exists because a failed `move_entry` is not necessarily a failure**, and the case it
/// covers is not an edge one — it is what happens every time somebody uses a machine that is not
/// already playing. Adding to an empty queue wakes the machine, and `advance()` takes the song
/// straight to the deck; the entry id the API just handed back therefore names something that no
/// longer exists, and moving it answers "not found". Reporting *queued, but it could not be moved
/// up* for that says the opposite of what happened: the song is not queued, it is playing.
///
/// Asked *after* the move rather than before it, and this is the whole reason the order is that way
/// round. Reading the state first would answer a question about the moment before the song was
/// added, and the interesting race — the song in the deck ending while the request is in flight —
/// would still be open, only now with a stale answer in hand instead of a fresh one.
///
/// It matches on the song number, so a song queued a second time while an identical one is already
/// loaded reads as having reached the deck. That is a real ambiguity and it resolves the kind way:
/// what the singer asked to hear is what is playing.
async fn is_on_the_deck(state: &Remote, number: SongCode) -> bool {
    let Ok(machine) = state.machine.state().await else {
        return false;
    };
    let Some(playing) = machine.now_playing else {
        return false;
    };
    // A demo song counts. It genuinely is on the deck, and somebody who just queued the song they
    // can already hear should be told so rather than being left to wonder — the only difference is
    // that this one arrived without being asked for.
    match playing.origin {
        OriginDto::Catalog { number: loaded, .. } | OriginDto::Demo { number: loaded } => {
            loaded == number
        }
        OriginDto::File { .. } => false,
    }
}

/// Whether a just-queued song is now the next thing that will be heard.
///
/// Either it was moved to the front of the queue, or the machine had nothing to play and took it
/// there itself — see [`is_on_the_deck`], which is the half that is easy to mistake for a failure.
async fn reached_the_front(state: &Remote, number: SongCode, entry_id: u64) -> bool {
    state.machine.move_entry(entry_id, 0).await.is_ok() || is_on_the_deck(state, number).await
}

/// `POST /song/{number}/{action}` — `queue`, `next` or `now`.
pub async fn song_action(
    State(state): State<Remote>,
    Path((number, action)): Path<(SongCode, String)>,
    headers: HeaderMap,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let singer = prefs.singer.as_deref();

    let outcome = match action.as_str() {
        "queue" => state.machine.enqueue(number, singer).await.map(|added| {
            (
                Badge {
                    level: "badge-good",
                    text: "badge-queued",
                },
                Toast::good(
                    crate::words::messages(locale)
                        .msg_with("queued-song", &[("title", added.title.as_str().into())]),
                ),
            )
        }),
        // Two calls, because the API has no "queue at the front" — and faking one inside the machine
        // trait would mean both implementations doing this same pair anyway. If neither lands the
        // song at the front the badge says so, because a song is still queued either way.
        "next" => match state.machine.enqueue(number, singer).await {
            Ok(added) => {
                if reached_the_front(&state, number, added.entry_id).await {
                    Ok((
                        Badge {
                            level: "badge-good",
                            text: "badge-next-up",
                        },
                        Toast::good(crate::words::messages(locale).msg_with(
                            "playing-next-song",
                            &[("title", added.title.as_str().into())],
                        )),
                    ))
                } else {
                    Ok((
                        Badge {
                            level: "badge-good",
                            text: "badge-queued",
                        },
                        Toast::warn(crate::words::messages(locale).msg_with(
                            "queued-not-moved",
                            &[("title", added.title.as_str().into())],
                        )),
                    ))
                }
            }
            Err(error) => Err(error),
        },
        // Three calls, for the same reason `next` is two: the API has no "play this one instead".
        // Queue it, move it to the front, and then end whatever is playing so that the front is
        // reached — which is `advance()` on the machine either way, whether it arrives as a skip or
        // as a wake.
        //
        // **Skip first and fall back to play**, rather than reading the state to find out which is
        // wanted. `Skip` answers `Unavailable` when nothing is loaded and `Play` answers
        // `Unavailable` when nothing is loaded *and* nothing is queued — which cannot be the case
        // here, since a song was just put at the front. Asking first would be a fourth round trip
        // and would still be racing the song that ends between the question and the answer.
        "now" => match state.machine.enqueue(number, singer).await {
            Ok(added) => {
                let moved = state.machine.move_entry(added.entry_id, 0).await.is_ok();
                let on_deck = !moved && is_on_the_deck(&state, number).await;

                if !moved && !on_deck {
                    Ok((
                        Badge {
                            level: "badge-good",
                            text: "badge-queued",
                        },
                        Toast::warn(crate::words::messages(locale).msg_with(
                            "queued-not-moved",
                            &[("title", added.title.as_str().into())],
                        )),
                    ))
                } else {
                    let started = if on_deck {
                        // It is already the loaded song, so **skipping here would skip the very
                        // song that was asked for** — which is why the two paths cannot be merged.
                        // Play is insurance for a machine that loaded it without starting it, and
                        // its answer is ignored: a machine that is already playing may refuse it,
                        // and being on the deck is the outcome ▶ promised either way.
                        let _ = state.machine.transport(Transport::Play).await;
                        true
                    } else {
                        match state.machine.transport(Transport::Skip).await {
                            Ok(_) => true,
                            // Nothing was playing, so there is nothing to skip and the front of the
                            // queue is simply where to start.
                            Err(RemoteError::Unavailable { .. }) => {
                                state.machine.transport(Transport::Play).await.is_ok()
                            }
                            Err(_) => false,
                        }
                    };
                    if started {
                        Ok((
                            Badge {
                                level: "badge-good",
                                text: "badge-playing",
                            },
                            Toast::good(crate::words::messages(locale).msg_with(
                                "playing-now-song",
                                &[("title", added.title.as_str().into())],
                            )),
                        ))
                    } else {
                        // The song is at the front and will play when this one ends, which is what
                        // `next up` means — saying `playing` here would be a lie about a television
                        // nobody in this room can see from their phone.
                        Ok((
                            Badge {
                                level: "badge-good",
                                text: "badge-next-up",
                            },
                            Toast::warn(crate::words::messages(locale).msg_with(
                                "next-not-started",
                                &[("title", added.title.as_str().into())],
                            )),
                        ))
                    }
                }
            }
            Err(error) => Err(error),
        },
        _ => {
            return (StatusCode::NOT_FOUND, "no such action").into_response();
        }
    };

    match outcome {
        Ok((badge, toast)) => views::with_toast(&badge, toast, locale),
        // A command the machine never acknowledged is not a failure — the song is almost certainly
        // queued and what is missing is the confirmation. Saying "sent" is the honest answer, and
        // coloring it red would be a lie about a song that is in fact in the queue.
        Err(RemoteError::NotAcknowledged) => views::with_toast(
            &Badge {
                level: "",
                text: "badge-sent",
            },
            views::message_for(&RemoteError::NotAcknowledged, locale),
            locale,
        ),
        // **The toast and nothing else.** A refusal that was about this moment — the queue was full,
        // the machine was away — can simply be tried again once it is not, and the buttons to try it
        // with never went anywhere: an answer is added to the row's slot rather than put in place of
        // it, so there is nothing here to put back.
        //
        // Re-rendering `SongActions` here would be wrong twice over. `hx-swap` is `innerHTML` on
        // `#song-{number}` and that fragment *is* the `<div class="song-actions"
        // id="song-{number}">`, so a refusal would nest a second copy of the slot inside the first
        // and give the document two elements with one id; and it would pass `in_folder: None`, so
        // a song refused while a favorites folder was open would lose its ✕ until the list was
        // drawn again.
        Err(error) => views::toast_only(views::message_for(&error, locale), locale),
    }
}

/// `POST /queue/{entry}/{action}` — `up`, `down` or `remove`.
pub async fn queue_action(
    State(state): State<Remote>,
    Path((entry, action)): Path<(u64, String)>,
    headers: HeaderMap,
) -> Response {
    let locale = prefs::locale(&headers);
    // The index a move needs is the entry's own, plus or minus one — which means reading the queue
    // first. Doing it here rather than in the trait keeps both implementations free of a concept the
    // API does not have.
    let position = state.machine.queue().await.ok().and_then(|queue| {
        queue
            .entries
            .iter()
            .find(|e| e.id == entry)
            .map(|e| e.position)
    });

    let result = match (action.as_str(), position) {
        ("remove", _) => state.machine.dequeue(entry).await.map(|_| ()),
        ("up", Some(position)) => state
            .machine
            .move_entry(entry, position.saturating_sub(1))
            .await
            .map(|_| ()),
        ("down", Some(position)) => state
            .machine
            .move_entry(entry, position + 1)
            .await
            .map(|_| ()),
        (_, None) => Err(RemoteError::NotFound),
        _ => return (StatusCode::NOT_FOUND, "no such action").into_response(),
    };

    // The list comes back either way, and it comes back **from the machine** rather than from what
    // this handler believes it did. A move that was refused then shows the order that actually
    // holds, instead of the one the tap was aiming at.
    let block = queue_block(&state).await;
    match result {
        Ok(()) => views::page(&block, locale),
        // The machine wants a password and this phone has not got one. Somewhere to type it beats a
        // toast that says so and leaves nothing to do about it.
        Err(error) => views::with_toast(&block, views::message_for(&error, locale), locale),
    }
}

/// Which fragment a control press is to be answered with.
///
/// Absent means the player card, which is what the Now page's own buttons want and what this route
/// answered before there was anywhere else to press one. `fragment=nowbar` is the Queue page's now
/// bar. The same `?fragment=` idiom the browse routes use, and for the same reason: which fragment
/// a press wants is a property of the press, not of the action.
///
/// It never reaches the machine's own permission test, which is handed a *path* and answers on the
/// `/api/v1/admin/` prefix alone, so a query parameter cannot be a way round it. A test pins it.
#[derive(Debug, Default, Deserialize)]
pub struct ControlParams {
    /// `nowbar`, or nothing.
    #[serde(default)]
    fragment: Option<String>,
}

impl ControlParams {
    /// Whether the answer belongs on the Queue page.
    fn wants_nowbar(&self) -> bool {
        self.fragment.as_deref() == Some("nowbar")
    }
}

/// `POST /control/{action}` — the transport and settings buttons, from either page.
pub async fn control(
    State(state): State<Remote>,
    Path(action): Path<String>,
    Query(params): Query<ControlParams>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let locale = prefs::locale(&headers);
    // The steppers are relative — "one semitone up from whatever it is" — so the current state has
    // to be read before it can be changed. A machine that will not answer that cannot be told to do
    // anything either, so this stops here with the reason rather than sending a command computed
    // from a value nobody has.
    let current = match state.machine.state().await {
        Ok(current) => current,
        Err(error) => {
            let toast = views::message_for(&error, locale);
            return if params.wants_nowbar() {
                views::with_toast(&NowBar::unreachable(), toast, locale)
            } else {
                views::with_toast(&PlayerBlock::unreachable(), toast, locale)
            };
        }
    };

    let result = match action.as_str() {
        "play" => state.machine.transport(Transport::Play).await.map(|_| ()),
        "pause" => state.machine.transport(Transport::Pause).await.map(|_| ()),
        "skip" => state.machine.transport(Transport::Skip).await.map(|_| ()),
        "restart" => state
            .machine
            .transport(Transport::Restart)
            .await
            .map(|_| ()),
        "stop" => state.machine.transport(Transport::Stop).await.map(|_| ()),
        "demo" => state.machine.start_demo().await,
        "transpose-up" => patch_transpose(&state, current.settings.transpose + 1).await,
        "transpose-down" => patch_transpose(&state, current.settings.transpose - 1).await,
        "transpose-reset" => patch_transpose(&state, 0).await,
        "tempo-up" => patch_tempo(&state, current.settings.tempo_ratio + 0.05).await,
        "tempo-down" => patch_tempo(&state, current.settings.tempo_ratio - 0.05).await,
        "tempo-reset" => patch_tempo(&state, 1.0).await,
        "melody" => state
            .machine
            .update_settings(&SettingsPatchDto {
                melody_enabled: Some(!current.settings.melody_enabled),
                ..Default::default()
            })
            .await
            .map(|_| ()),
        "volume" => {
            let percent: f32 = form::field(&body, "value")
                .as_deref()
                .and_then(|value| value.parse().ok())
                .unwrap_or(current.settings.music_volume * 100.0);
            state
                .machine
                .update_settings(&SettingsPatchDto {
                    music_volume: Some((percent / 100.0).clamp(0.0, 1.0)),
                    ..Default::default()
                })
                .await
                .map(|_| ())
        }
        _ => return (StatusCode::NOT_FOUND, "no such control").into_response(),
    };

    // Re-read rather than assume. A stepper that walked past what the machine accepts, or a control
    // the loaded song refuses, must leave the card showing the value that is really set.
    let view = player_view(&state).await;
    // **`demo` is the one press whose success the card cannot show**, and the toast is how it says
    // so. The machine sets a flag and its own poll thread starts the song a moment later, so the
    // state read above was taken while the deck was still empty — the card that comes back says
    // `Nothing playing` about a machine that is about to. `SongStarted` replaces it over the event
    // stream when the song really begins; this is what fills the gap, and what covers the case where
    // that republish arrives before the swap does.
    //
    // **`skip` is the same press wearing a different label whenever the deck was empty.** A skip
    // that succeeds with nothing on it succeeded by starting a demo, which is the only thing it
    // could have done, so it has the gap above and is filled the same way. The guard is the state
    // read before the command, which is already in hand — an ordinary skip cannot reach it.
    let toast = match (&result, action.as_str()) {
        (Ok(()), "demo") => Some(Toast::good(say(locale, "demo-starting"))),
        (Ok(()), "skip") if current.now_playing.is_none() => {
            Some(Toast::good(say(locale, "demo-starting")))
        }
        (Err(error), _) => Some(views::message_for(error, locale)),
        _ => None,
    };

    // The two cards are the same state rendered twice; which one the caller gets is decided by the
    // press rather than by the outcome, so the branch is here and not inside each arm.
    if params.wants_nowbar() {
        let block = NowBar::of(view);
        match toast {
            Some(toast) => views::with_toast(&block, toast, locale),
            None => views::page(&block, locale),
        }
    } else {
        let block = PlayerBlock::of(view);
        match toast {
            Some(toast) => views::with_toast(&block, toast, locale),
            None => views::page(&block, locale),
        }
    }
}

/// Shifts the key, clamped to the range the machine accepts.
async fn patch_transpose(state: &Remote, semitones: i8) -> Result<(), RemoteError> {
    state
        .machine
        .update_settings(&SettingsPatchDto {
            transpose: Some(semitones.clamp(-6, 6)),
            ..Default::default()
        })
        .await
        .map(|_| ())
}

/// Changes the tempo, clamped so a stepper cannot walk it into silence.
async fn patch_tempo(state: &Remote, ratio: f32) -> Result<(), RemoteError> {
    state
        .machine
        .update_settings(&SettingsPatchDto {
            tempo_ratio: Some(ratio.clamp(0.5, 1.5)),
            ..Default::default()
        })
        .await
        .map(|_| ())
}

/// `GET /favorites/sheet/{number}`
pub async fn sheet(
    State(state): State<Remote>,
    Path(number): Path<SongCode>,
    Query(params): Query<SheetParams>,
    headers: HeaderMap,
) -> Response {
    let locale = prefs::locale(&headers);
    match sheet_for(&state, number, params.multi.is_some(), None).await {
        Ok(sheet) => views::page(&sheet, locale),
        Err(error) => failure(&error, locale),
    }
}

/// What the sheet's own links say.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SheetParams {
    /// Present when the sheet should stay open after a tap.
    #[serde(default)]
    pub multi: Option<String>,
}

/// Builds the picker from the database, every time.
///
/// Never patched in place: two phones can be filing the same song at once, and a sheet that had
/// patched itself would show one of them a state that never existed.
async fn sheet_for(
    state: &Remote,
    number: SongCode,
    multi: bool,
    error: Option<String>,
) -> Result<Sheet, RemoteError> {
    let Some(favorites) = &state.favorites else {
        return Err(RemoteError::Unavailable {
            code: crate::machine::NO_FAVORITES.to_owned(),
            message: "this remote has no favorites".to_owned(),
        });
    };
    let song = state.songs.song(number).await?;
    let folders = favorites.folders().await?;
    let inside: HashSet<i64> = favorites
        .folders_for_song(number)
        .await?
        .into_iter()
        .collect();
    Ok(Sheet {
        number,
        title: song
            .as_ref()
            .map(|song| song.title.clone())
            .unwrap_or_else(|| format!("#{number}")),
        artist: song.and_then(|song| song.artist),
        folders: folders
            .into_iter()
            .map(|folder| {
                let is_in = inside.contains(&folder.id);
                (folder, is_in)
            })
            .collect(),
        multi,
        error,
    })
}

/// `GET /favorites/sheet/close`
pub async fn sheet_close() -> Response {
    Html("").into_response()
}

/// `POST /favorites/folders` — make a folder, and file a song into it in the same tap.
pub async fn create_folder(
    State(state): State<Remote>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let locale = prefs::locale(&headers);
    let Some(favorites) = &state.favorites else {
        return failure(
            &RemoteError::Unavailable {
                code: crate::machine::NO_FAVORITES.to_owned(),
                message: "this remote has no favorites".to_owned(),
            },
            locale,
        );
    };
    let name = form::field(&body, "name").unwrap_or_default();
    let song = form::field(&body, "song").and_then(|value| value.parse::<SongCode>().ok());
    let multi = form::flag(&body, "multi");

    if name.trim().is_empty() {
        return match song {
            Some(number) => {
                match sheet_for(
                    &state,
                    number,
                    multi,
                    Some(say(locale, "folder-needs-name")),
                )
                .await
                {
                    Ok(sheet) => views::page(&sheet, locale),
                    Err(error) => failure(&error, locale),
                }
            }
            None => views::toast_only(Toast::warn(say(locale, "folder-needs-name")), locale),
        };
    }

    match favorites.ensure_folder(name.trim()).await {
        Ok((folder, _created)) => {
            if let Some(number) = song {
                // Create-and-file in one tap: the first folder has to be makeable at the moment
                // somebody wants one, or it never gets made.
                let _ = favorites.toggle(folder.id, number).await;
                after_filing(&state, number, &folder.name, true, multi, locale).await
            } else {
                views::toast_only(
                    Toast::good(
                        crate::words::messages(locale)
                            .msg_with("folder-made", &[("folder", folder.name.as_str().into())]),
                    ),
                    locale,
                )
            }
        }
        Err(error) => match song {
            Some(number) => match sheet_for(&state, number, multi, Some(error.to_string())).await {
                Ok(sheet) => views::page(&sheet, locale),
                Err(error) => failure(&error, locale),
            },
            None => views::toast_only(views::message_for(&error, locale), locale),
        },
    }
}

/// `POST /favorites/folders/{id}/rename`
pub async fn rename_folder(
    State(state): State<Remote>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let locale = prefs::locale(&headers);
    let Some(favorites) = &state.favorites else {
        return failure(
            &RemoteError::Unavailable {
                code: crate::machine::NO_FAVORITES.to_owned(),
                message: "this remote has no favorites".to_owned(),
            },
            locale,
        );
    };
    let name = form::field(&body, "name").unwrap_or_default();
    match favorites.rename_folder(id, name.trim()).await {
        Ok(()) => views::toast_only(
            Toast::good(
                crate::words::messages(locale)
                    .msg_with("folder-renamed", &[("folder", name.trim().into())]),
            ),
            locale,
        ),
        Err(error) => views::toast_only(views::message_for(&error, locale), locale),
    }
}

/// `POST /favorites/folders/{id}/delete`
pub async fn delete_folder(
    State(state): State<Remote>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let locale = prefs::locale(&headers);
    let Some(favorites) = &state.favorites else {
        return failure(
            &RemoteError::Unavailable {
                code: crate::machine::NO_FAVORITES.to_owned(),
                message: "this remote has no favorites".to_owned(),
            },
            locale,
        );
    };
    match favorites.delete_folder(id).await {
        Ok(()) => views::toast_only(Toast::good(say(locale, "folder-deleted")), locale),
        Err(error) => views::toast_only(views::message_for(&error, locale), locale),
    }
}

/// `POST /favorites/folders/{id}/toggle/{number}` — file a song, or take it back out.
pub async fn toggle_favorite(
    State(state): State<Remote>,
    Path((id, number)): Path<(i64, SongCode)>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let locale = prefs::locale(&headers);
    let Some(favorites) = &state.favorites else {
        return failure(
            &RemoteError::Unavailable {
                code: crate::machine::NO_FAVORITES.to_owned(),
                message: "this remote has no favorites".to_owned(),
            },
            locale,
        );
    };
    let multi = form::flag(&body, "multi");
    let name = favorites
        .folder(id)
        .await
        .ok()
        .flatten()
        .map(|folder| folder.name)
        .unwrap_or_else(|| "that folder".to_owned());

    match favorites.toggle(id, number).await {
        Ok(added) => after_filing(&state, number, &name, added, multi, locale).await,
        Err(error) => match sheet_for(&state, number, multi, Some(error.to_string())).await {
            Ok(sheet) => views::page(&sheet, locale),
            Err(error) => failure(&error, locale),
        },
    }
}

/// What the page gets back after a song is filed or unfiled.
///
/// In the one-tap way this **closes the sheet and updates the star behind it out of band** — which
/// is what keeps the list from being re-rendered and the reader's place from being lost. In the
/// multi way the sheet is drawn again from the database, so the ticks are the truth rather than a
/// patch.
async fn after_filing(
    state: &Remote,
    number: SongCode,
    folder: &str,
    added: bool,
    multi: bool,
    locale: Locale,
) -> Response {
    let toast = if added {
        Toast::good(
            crate::words::messages(locale).msg_with("favorite-added", &[("folder", folder.into())]),
        )
    } else {
        Toast::info(
            crate::words::messages(locale)
                .msg_with("favorite-removed", &[("folder", folder.into())]),
        )
    };

    if multi {
        return match sheet_for(state, number, true, None).await {
            Ok(sheet) => views::with_toast(&sheet, toast, locale),
            Err(error) => failure(&error, locale),
        };
    }

    let still_in = match &state.favorites {
        Some(favorites) => !favorites
            .folders_for_song(number)
            .await
            .unwrap_or_default()
            .is_empty(),
        None => false,
    };
    // The sheet closes -- the response replaces `#sheet` with nothing -- and the star behind it is
    // updated out of band. That is what keeps the list itself from being re-rendered, and with it
    // the reader's place in a thousand rows.
    views::with_toast(
        &Star {
            number,
            on: still_in,
            oob: true,
        },
        toast,
        locale,
    )
}

/// `POST /favorites/folders/{id}/remove/{number}` — from inside a folder's own list.
pub async fn remove_favorite(
    State(state): State<Remote>,
    Path((id, number)): Path<(i64, SongCode)>,
    Query(params): Query<BrowseParams>,
    headers: HeaderMap,
) -> Response {
    let locale = prefs::locale(&headers);
    let Some(favorites) = &state.favorites else {
        return failure(
            &RemoteError::Unavailable {
                code: crate::machine::NO_FAVORITES.to_owned(),
                message: "this remote has no favorites".to_owned(),
            },
            locale,
        );
    };
    if let Err(error) = favorites.remove(id, number).await {
        return views::toast_only(views::message_for(&error, locale), locale);
    }
    // The whole browse block comes back, not the row: taking a song out changes the count beside the
    // filters as well as the list, and they are one fragment.
    let prefs = Prefs::read(&headers);
    // One page and no anchor: this is a swap, not a tab press. The tag is still stamped on, so that
    // rows arriving from here are labeled with the list they belong to like every other row — an
    // anchor captured before the removal is about this same list and stays usable.
    let tag = prefs::list_tag(
        prefs::browse_state(
            params.mode(state.capabilities),
            &params.text(),
            params.language().as_deref(),
            &params.tags(),
            params.initial(state.capabilities),
            params.artist().as_deref(),
            params.folder,
        )
        .as_deref(),
    );
    match browse_block(&state, &params, &prefs, PAGE, None, &tag).await {
        Ok(block) => views::with_toast(
            &block,
            Toast::info(say(locale, "favorite-removed-short")),
            locale,
        ),
        Err(error) => failure(&error, locale),
    }
}

// -- sharing a folder, and carrying the collection to a file -------------------------------------

/// The most bytes a restore will read.
///
/// **Two caps, and the ordering looks redundant until it does not.** axum's `DefaultBodyLimit` is
/// 2 MB for a `String` body, it is applied *before* a handler runs, and it answers `413` — a status
/// htmx will not swap, which is precisely the dead-button failure `form.rs` was written against. So
/// the route carries `DefaultBodyLimit::max(RESTORE_LIMIT)` to let anything a person could plausibly
/// pick *reach* this code, and this constant is the refusal that gets a sentence.
///
/// Generous on purpose: an eleven-hundred-song export — past the point one folder fits in any QR
/// code — is under a hundred kilobytes, so this is two orders of magnitude of room and still small
/// enough that a file picked by mistake fails fast.
pub const MAX_DOCUMENT: usize = 2 << 20;

/// What the route's own body limit allows through, so the refusal above is the one that speaks.
pub const RESTORE_LIMIT: usize = 8 << 20;

/// The collection, or the online mode's refusal.
fn collection(state: &Remote) -> Result<&Arc<dyn crate::machine::Favorites>, RemoteError> {
    state
        .favorites
        .as_ref()
        .ok_or_else(|| RemoteError::Unavailable {
            code: crate::machine::NO_FAVORITES.to_owned(),
            message: "this remote has no favorites".to_owned(),
        })
}

/// The folder every share screen hangs off.
///
/// A stale id goes back to the folder list rather than erroring: the folder was deleted in another
/// tab, which is not a fault worth a page of its own.
///
/// **The locale is a parameter, not a default.** A database fault here draws a page, and a page this
/// crate draws is in the viewer's language — an English sentence inside a Portuguese page is the one
/// failure `machine::codes` exists to prevent, and hardcoding a locale for the unhappy path is how
/// it comes back.
///
/// `Box`ed, because a bare `Response` in an `Err` is 128 bytes on every call of every screen and
/// clippy's `result_large_err` is right about it.
async fn share_folder(state: &Remote, id: i64, locale: Locale) -> Result<FolderRow, Box<Response>> {
    let favorites = match collection(state) {
        Ok(favorites) => favorites,
        Err(error) => return Err(Box::new(failure(&error, locale))),
    };
    match favorites.folder(id).await {
        Ok(Some(folder)) => Ok(folder),
        Ok(None) => Err(Box::new(Redirect::to("/?mode=favorites").into_response())),
        Err(error) => Err(Box::new(failure(&error, locale))),
    }
}

/// What this device's catalog made of a set of incoming favorites.
pub struct Placed {
    /// The numbers to file, **as this catalog numbers them** rather than as they arrived.
    ///
    /// The difference is the point of carrying an identity at all: a folder shared from a phone
    /// whose machine banked a package differently arrives naming numbers that mean other songs
    /// here, and resolving by content is what turns them into the right ones.
    pub keep: Vec<SongCode>,
    /// The ones nothing here could place, with why.
    ///
    /// **Listed rather than only dropped.** Silently discarding them leaves somebody counting a
    /// folder to work out that anything happened at all. The
    /// remedy differs by [`Miss`], which is why the reason travels with the row — see
    /// `curation.md`'s `Backing up what a person typed, and nothing else`, which reached the same
    /// conclusion for the package builder: an unmatched song is listed, never invented.
    pub missing: Vec<(SongRef, Miss)>,
}

/// The songs from `wanted` that this device's catalog can actually show.
///
/// **Dropped before the write rather than stored and hidden.** A folder's count comes from the
/// favorites database while its listing is filtered through the catalog, so keeping a code nothing
/// can draw would make a folder claim more songs than it lists. What is new is that they are
/// *reported* on the way past, so a merge can say what it could not take.
///
/// **One call with the whole slice, and no batching.** Neither implementation of
/// [`Songs::resolve`](crate::machine::Songs::resolve) binds a parameter per row — the mirror runs
/// prepared statements per song and the machine's adapter loops its catalog inside a single visit —
/// so there is no variable limit to divide up, and a batch loop would only add hops onto the
/// blocking pool.
///
/// **A catalog holding nothing keeps everything, and that case is not academic**: it is a
/// `km-remote` that has never reached a machine, which is this app's normal starting state.
/// Filtering against an empty mirror would discard somebody's whole collection at the exact moment
/// they were restoring it, and report the loss as a count. The temporary drift is a state this app
/// already accepts — a favorite is allowed to outlive the package its song came from — and it
/// resolves itself the first time the catalog is copied.
async fn known_songs(state: &Remote, wanted: &[SongRef]) -> Result<Placed, RemoteError> {
    if wanted.is_empty() {
        return Ok(Placed {
            keep: Vec::new(),
            missing: Vec::new(),
        });
    }
    // **The error propagates rather than reading as an empty catalog.** `unwrap_or(0)` here would
    // fold a transient fault into the "nothing copied yet" branch below, which skips the filter
    // entirely — so a catalog that failed to answer would silently file codes it could have
    // rejected, and say nothing. The next line has always propagated its own failure; these two
    // ask the same question of the same database and now fail the same way.
    if state.songs.count().await? == 0 {
        return Ok(Placed {
            keep: wanted.iter().map(|asked| asked.code).collect(),
            missing: Vec::new(),
        });
    }
    let mut keep = Vec::with_capacity(wanted.len());
    let mut missing = Vec::new();
    for resolution in state.songs.resolve(wanted).await? {
        match resolution.outcome {
            Ok(song) => keep.push(song.number),
            Err(miss) => missing.push((resolution.asked, miss)),
        }
    }
    Ok(Placed { keep, missing })
}

/// One folder encoded, and whether the result is beyond what a QR can hold.
async fn folder_code(
    state: &Remote,
    folder: &FolderRow,
) -> Result<(Result<String, share::ShareError>, bool), RemeteCodeError> {
    let favorites = collection(state).map_err(RemeteCodeError)?;
    let songs = favorites
        .song_ids(folder.id)
        .await
        .map_err(RemeteCodeError)?;
    let encoded = share::encode(&share::Folder {
        name: folder.name.clone(),
        songs,
    });
    let too_dense = match &encoded {
        // Asked here as well as in the image handler, because a page that links an image the
        // handler will refuse shows a broken icon and no reason.
        Ok(code) => {
            qrcode::QrCode::with_error_correction_level(code.as_bytes(), EcLevel::M).is_err()
        }
        Err(_) => false,
    };
    Ok((encoded, too_dense))
}

/// A wrapper so `folder_code` can fail with a page-able error without borrowing one.
struct RemeteCodeError(RemoteError);

/// The header a share or backup screen wears.
fn step_head(title: String, back: Option<String>) -> StepHead {
    StepHead { back, title }
}

/// `GET /favorites/share/{folder}`
pub async fn share_choose(
    State(state): State<Remote>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let folder = match share_folder(&state, id, locale).await {
        Ok(folder) => folder,
        Err(response) => return *response,
    };
    let words = crate::words::messages(locale);
    let title = folder_sentence(words, "share-title", &folder.name);
    let chrome = chrome(&state, "browse", &prefs).await;
    views::page(
        &SharePage {
            chrome,
            head: step_head(
                title,
                Some(format!("/?mode=favorites&folder={}", folder.id)),
            ),
            receive_line: folder_sentence(words, "share-receive-sub", &folder.name),
            one_way_line: folder_sentence(words, "share-one-way", &folder.name),
            folder,
        },
        locale,
    )
}

/// `GET /favorites/share/{folder}/send`
pub async fn share_send(
    State(state): State<Remote>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let folder = match share_folder(&state, id, locale).await {
        Ok(folder) => folder,
        Err(response) => return *response,
    };
    let (encoded, too_dense) = match folder_code(&state, &folder).await {
        Ok(pair) => pair,
        Err(RemeteCodeError(error)) => return failure(&error, locale),
    };
    let words = crate::words::messages(locale);
    let chrome = chrome(&state, "browse", &prefs).await;
    let (code, error) = match encoded {
        Ok(code) => (code, None),
        Err(refusal) => (String::new(), Some(say(locale, refusal.message_key()))),
    };
    views::page(
        &ShareSendPage {
            chrome,
            head: step_head(folder.name.clone(), Some(share_root(folder.id))),
            count: song_count(words, folder.songs),
            image_alt: folder_sentence(words, "share-code-alt", &folder.name),
            code,
            too_dense,
            error,
            folder,
        },
        locale,
    )
}

/// `GET /favorites/share/{folder}/code.svg`
///
/// **An SVG rather than a raster**, which deletes a paragraph rather than writing one: a PNG has to
/// be generated at a chosen pixel scale, and the right scale is a function of how large the page
/// draws it and what the device's pixel ratio is — generate at the natural size and it blurs on the
/// way up, generate far larger and a downscale eats whole modules. A vector is right at every size,
/// and the library's own `shape-rendering="crispEdges"` keeps the modules square.
///
/// **Level M and not L**, spelled out although it is `QrCode::new`'s default so that it cannot move
/// under a dependency bump: these are read off a glossy screen at an angle, and fifteen percent
/// recovery buys a much better chance of a scan succeeding first time for a few modules.
/// **Every unhappy path here is a bare status, and none of them is a page.** This is what an
/// `<img src>` fetches, so there is no document to word a sentence into and no viewer whose language
/// to word it in — a redirect or a failure page would arrive as the image, which is a broken icon
/// with extra steps. It therefore does not go through [`share_folder`], whose whole job is choosing
/// between a redirect and a page. The reason lands in the log, and the page that linked this asked
/// the same questions before drawing the link.
pub async fn share_image(State(state): State<Remote>, Path(id): Path<i64>) -> Response {
    let Ok(favorites) = collection(&state) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let folder = match favorites.folder(id).await {
        Ok(Some(folder)) => folder,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(%error, folder = id, "could not read a folder to draw its code");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let (encoded, too_dense) = match folder_code(&state, &folder).await {
        Ok(pair) => pair,
        Err(RemeteCodeError(error)) => {
            tracing::warn!(%error, folder = folder.id, "could not read a folder to draw its code");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    // **No body to read on a refusal.** This is an `<img src>`, so there is no page here to
    // word anything into, and the page that linked it asked the same question before drawing the
    // link. The log is where the answer belongs.
    let Ok(code) = encoded else {
        tracing::warn!(folder = folder.id, "that folder has no code to draw");
        return StatusCode::BAD_REQUEST.into_response();
    };
    if too_dense {
        tracing::warn!(folder = folder.id, "that folder is too dense for one code");
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(drawn) = qrcode::QrCode::with_error_correction_level(code.as_bytes(), EcLevel::M) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // One module to one unit, so the `viewBox` is the module count and the browser rasterizes at
    // whatever the screen actually is. The library draws the four-module quiet zone the standard
    // requires, and a reader needs it.
    let svg = drawn
        .render::<qrcode::render::svg::Color<'_>>()
        .module_dimensions(1, 1)
        .build();
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            // The folder changes as songs are filed, and the URL does not.
            (header::CACHE_CONTROL, "no-store"),
        ],
        svg,
    )
        .into_response()
}

/// `GET /favorites/share/{folder}/receive`
pub async fn share_receive(
    State(state): State<Remote>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let folder = match share_folder(&state, id, locale).await {
        Ok(folder) => folder,
        Err(response) => return *response,
    };
    let chrome = chrome(&state, "browse", &prefs).await;
    views::page(&receiving(chrome, locale, folder, None), locale)
}

/// The receive screen, which is also where a code that would not decode comes back to.
fn receiving(
    chrome: Chrome,
    locale: Locale,
    folder: FolderRow,
    error: Option<String>,
) -> ShareReceivePage {
    let words = crate::words::messages(locale);
    ShareReceivePage {
        chrome,
        head: step_head(
            folder_sentence(words, "share-title", &folder.name),
            Some(share_root(folder.id)),
        ),
        target_line: folder_sentence(words, "share-point-at", &folder.name),
        error,
        folder,
    }
}

/// `POST /favorites/share/{folder}/receive` — what was read, before anything is written.
pub async fn share_scanned(
    State(state): State<Remote>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let folder = match share_folder(&state, id, locale).await {
        Ok(folder) => folder,
        Err(response) => return *response,
    };
    let chrome = chrome(&state, "browse", &prefs).await;
    let raw = form::field(&body, "code").unwrap_or_default();
    let scanned = match share::decode(&raw) {
        Ok(scanned) => scanned,
        Err(refusal) => {
            return views::page(
                &receiving(
                    chrome,
                    locale,
                    folder,
                    Some(say(locale, refusal.message_key())),
                ),
                locale,
            );
        }
    };

    let words = crate::words::messages(locale);
    let name = (!scanned.name.is_empty()).then(|| scanned.name.clone());
    // Not an error, and said out loud even so: names not matching is also what a mis-scan looks
    // like.
    let mismatch = name
        .as_ref()
        .filter(|from| !from.eq_ignore_ascii_case(&folder.name))
        .map(|from| {
            words
                .msg_with(
                    "share-confirm-mismatch",
                    &[
                        ("from", from.as_str().into()),
                        ("into", folder.name.as_str().into()),
                    ],
                )
                .into_owned()
        });
    views::page(
        &ShareConfirmPage {
            chrome,
            head: step_head(
                folder_sentence(words, "share-title", &folder.name),
                Some(format!("{}/receive", share_root(folder.id))),
            ),
            scanned_count: song_count(words, scanned.songs.len()),
            scanned_name: name,
            raw,
            mismatch,
            add_label: folder_sentence(words, "share-confirm-add", &folder.name),
            folder,
        },
        locale,
    )
}

/// `POST /favorites/share/{folder}/merge`
///
/// **No redirect afterwards**, and no confirmation before: the write is add-only and idempotent, so
/// a reload that repeats it changes nothing, and showing the outcome directly is worth more than
/// guarding against a resubmission that cannot do harm.
pub async fn share_merge(
    State(state): State<Remote>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let folder = match share_folder(&state, id, locale).await {
        Ok(folder) => folder,
        Err(response) => return *response,
    };
    let chrome = chrome(&state, "browse", &prefs).await;
    let raw = form::field(&body, "code").unwrap_or_default();
    let scanned = match share::decode(&raw) {
        Ok(scanned) => scanned,
        Err(refusal) => {
            return views::page(
                &receiving(
                    chrome,
                    locale,
                    folder,
                    Some(say(locale, refusal.message_key())),
                ),
                locale,
            );
        }
    };

    // **A code carries numbers and nothing else**, which is the format's own constraint rather than
    // an oversight: it has to fit through a QR in numeric mode and stay byte-for-byte what the
    // sibling project writes. So a scanned folder resolves on the number rung alone — correct for
    // what sharing is for, two phones in one room pointed at one machine.
    let scanned_refs: Vec<SongRef> = scanned.songs.iter().copied().map(SongRef::code).collect();
    let placed = match known_songs(&state, &scanned_refs).await {
        Ok(placed) => placed,
        Err(error) => return failure(&error, locale),
    };
    let keep = placed.keep;
    let favorites = match collection(&state) {
        Ok(favorites) => favorites,
        Err(error) => return failure(&error, locale),
    };
    let added = match favorites.add_songs(folder.id, &keep).await {
        Ok(added) => added,
        Err(error) => return failure(&error, locale),
    };

    let unknown = scanned.songs.len() - keep.len();
    tracing::info!(
        folder = %folder.name,
        scanned = scanned.songs.len(),
        added,
        unknown,
        "merged a shared folder"
    );
    let words = crate::words::messages(locale);
    views::page(
        &ShareDonePage {
            chrome,
            head: step_head(folder_sentence(words, "share-title", &folder.name), None),
            outcome: outcome_line(words, added, keep.len() - added),
            left_out: left_out_line(words, unknown),
            open_label: folder_sentence(words, "share-open-folder", &folder.name),
            folder,
        },
        locale,
    )
}

/// `GET /favorites/backup`
pub async fn backup_choose(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    let favorites = match collection(&state) {
        Ok(favorites) => favorites,
        Err(error) => return failure(&error, locale),
    };
    let folders = match favorites.folders().await {
        Ok(folders) => folders,
        Err(error) => return failure(&error, locale),
    };
    // Counted over what the *file* will hold, so an empty folder is in neither number: a page
    // promising four folders that writes three is worse than one that says three.
    let carried: Vec<&FolderRow> = folders.iter().filter(|f| f.songs > 0).collect();
    let songs: usize = carried.iter().map(|f| f.songs).sum();
    let words = crate::words::messages(locale);
    let chrome = chrome(&state, "setup", &prefs).await;
    views::page(
        &BackupPage {
            chrome,
            // Back to the Setup tab, which is the path this was reached by. The favorites
            // themselves are a destination rather than a way back, and `backup_done.html` is where
            // that link belongs.
            head: step_head(say(locale, "backup-title"), Some("/setup".to_owned())),
            any: songs > 0,
            holds: words
                .msg_with(
                    "backup-save-sub",
                    &[
                        ("songs", song_count(words, songs).into()),
                        ("folders", folder_count(words, carried.len()).into()),
                    ],
                )
                .into_owned(),
        },
        locale,
    )
}

/// `GET /favorites/backup.json`
///
/// **An attachment, and that header is load-bearing rather than decorative**: it is what both phone
/// shells key off to turn this into a file the device can put somewhere, and in an ordinary browser
/// it is what makes the page save rather than render.
pub async fn backup_export(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let locale = prefs::locale(&headers);
    let favorites = match collection(&state) {
        Ok(favorites) => favorites,
        Err(error) => return failure(&error, locale),
    };
    let folders = match favorites.folders().await {
        Ok(folders) => folders,
        Err(error) => return failure(&error, locale),
    };

    let mut carried = Vec::new();
    for folder in &folders {
        let songs = match favorites.song_refs(folder.id).await {
            Ok(songs) => songs,
            Err(error) => return failure(&error, locale),
        };
        if songs.is_empty() {
            continue; // an empty folder carries nothing and restores to nothing
        }
        let named = match state.songs.resolve(&songs).await {
            Ok(named) => named,
            Err(error) => return failure(&error, locale),
        };
        carried.push(backup::FolderDoc {
            name: folder.name.clone(),
            songs: named
                .iter()
                .map(|resolution| {
                    // **A song the catalog cannot name is still written, code alone.** Export
                    // preserves and import filters, and the asymmetry is deliberate: dropping it
                    // here would lose it for good, whereas storing one at restore is the count
                    // drift `known_songs` describes.
                    //
                    // **The identity is written even then, from what the favorite itself holds.**
                    // That is the case this file most needs to carry: a favorite of a package that
                    // is not installed today has no title to write, and its hash is the only thing
                    // that will ever find it again. The catalog's answer is preferred where there
                    // is one, because it is the fresher of the two.
                    let asked = &resolution.asked;
                    let found = resolution.song();
                    backup::SongDoc {
                        code: asked.code.to_string(),
                        title: found.map(|song| song.title.clone()),
                        artist: found.and_then(|song| song.artist.clone()),
                        package_id: found
                            .map(|song| song.package_id.clone())
                            .or_else(|| asked.package_id.clone()),
                        content_hash: found
                            .and_then(|song| song.content_hash.clone())
                            .or_else(|| asked.content_hash.clone()),
                    }
                })
                .collect(),
        });
    }

    let document = backup::Document::of(carried);
    let body = match document.to_json() {
        Ok(body) => body,
        Err(_) => {
            return failure(
                &RemoteError::Failed("the file could not be written".to_owned()),
                locale,
            );
        }
    };
    tracing::info!(
        folders = document.folders.len(),
        songs = document.songs(),
        "wrote a favorites backup"
    );
    let disposition = format!("attachment; filename=\"{}\"", document.filename());
    (
        [
            (
                header::CONTENT_TYPE,
                "application/json; charset=utf-8".to_owned(),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            // The favorites change as songs are filed, and the URL does not.
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        body,
    )
        .into_response()
}

/// `GET /favorites/backup/restore`
pub async fn backup_restore_page(State(state): State<Remote>, headers: HeaderMap) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    // **Guarded although this handler writes nothing.** Every other screen in both flows refuses
    // here, and one that drew a form instead would offer the online remote a file picker whose
    // Restore button could only ever refuse — which is the grayed-out control `Capabilities` exists
    // to prevent, arrived at by a different road.
    if let Err(error) = collection(&state) {
        return failure(&error, locale);
    }
    let chrome = chrome(&state, "setup", &prefs).await;
    views::page(&restoring(chrome, locale, None), locale)
}

/// The restore screen, which is also where a file this build cannot read comes back to.
fn restoring(chrome: Chrome, locale: Locale, error: Option<String>) -> RestorePage {
    RestorePage {
        chrome,
        head: step_head(
            say(locale, "backup-restore-title"),
            Some("/favorites/backup".to_owned()),
        ),
        error,
    }
}

/// `POST /favorites/backup/restore`
///
/// No confirmation and no redirect, for [`share_merge`]'s reason: the write is add-only and
/// idempotent, so a reload that repeats it changes nothing.
pub async fn backup_restore(
    State(state): State<Remote>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let prefs = Prefs::read(&headers);
    let locale = prefs.locale;
    // Before the document is even looked at: a remote with no collection has nothing to restore
    // *into*, and saying so beats reporting what was wrong with a file that could never have landed.
    if let Err(error) = collection(&state) {
        return failure(&error, locale);
    }
    let chrome = chrome(&state, "setup", &prefs).await;
    let refuse = |chrome, error: backup::BackupError| {
        views::page(
            &restoring(chrome, locale, Some(say(locale, error.message_key()))),
            locale,
        )
    };

    let text = form::field(&body, "document").unwrap_or_default();
    if text.trim().is_empty() {
        return refuse(chrome, backup::BackupError::NoFile);
    }
    if text.len() > MAX_DOCUMENT {
        return refuse(chrome, backup::BackupError::TooLarge);
    }
    let (document, restorable) = match backup::read(&text) {
        Ok(read) => read,
        Err(error) => return refuse(chrome, error),
    };

    let favorites = match collection(&state) {
        Ok(favorites) => favorites,
        Err(error) => return failure(&error, locale),
    };
    let (mut added, mut already, mut unknown, mut created) = (0usize, 0usize, 0usize, 0usize);
    let mut unplaced: Vec<(SongRef, Miss)> = Vec::new();
    let read: usize = restorable.iter().map(|folder| folder.songs.len()).sum();
    let unreadable: usize = restorable.iter().map(|folder| folder.unreadable).sum();
    // Counted over the folders that actually carried something, matching the number the *export*
    // page shows for the same reason: a report claiming four folders where three were written is the
    // drift both sides of this feature are written to avoid. A folder every one of whose rows was
    // unreadable is in `unreadable` and in neither of these.
    let folders = restorable
        .iter()
        .filter(|folder| !folder.songs.is_empty())
        .count();

    for folder in &restorable {
        // **A folder with nothing readable in it is not created**, which is the export's own rule
        // read backwards: that side leaves an empty folder out of the file and out of the count, so
        // this side must not conjure one. Reachable only from a hand-edited file — every code in one
        // folder mistyped — and without this it would make an empty folder and report it under
        // *folders created*, which is a change somebody did not ask for and cannot see the reason
        // for. Its unreadable rows are still counted: `unreadable` is summed over every folder
        // above, before this loop narrows to the ones worth writing.
        if folder.songs.is_empty() {
            continue;
        }
        let (target, made) = match favorites.ensure_folder(&folder.name).await {
            Ok(pair) => pair,
            Err(error) => return failure(&error, locale),
        };
        if made {
            created += 1;
        }
        let placed = match known_songs(&state, &folder.songs).await {
            Ok(placed) => placed,
            Err(error) => return failure(&error, locale),
        };
        let keep = placed.keep;
        unknown += folder.songs.len() - keep.len();
        // **Listed, not merely counted.** A count says a number went missing; this says which song
        // and what to do about it — a package that is not installed is something the owner can go
        // and install, and a recording that is not in a package they do have is not. The rows are
        // gathered across every folder and reported once, beside the counts.
        unplaced.extend(placed.missing);
        match favorites.add_songs(target.id, &keep).await {
            Ok(new) => {
                added += new;
                already += keep.len() - new;
            }
            Err(error) => return failure(&error, locale),
        }
    }

    tracing::info!(
        folders = restorable.len(),
        created,
        read,
        added,
        unknown,
        unreadable,
        "restored a favorites backup"
    );
    // The page names a sample; this names all of them, because a collection somebody is trying to
    // recover is exactly the case where the rest matters and a phone screen has no room for it.
    if !unplaced.is_empty() {
        let codes = unplaced
            .iter()
            .map(|(asked, _)| asked.code.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            count = unplaced.len(),
            %codes,
            "favorites this device's song list could not place; left out"
        );
    }
    let words = crate::words::messages(locale);
    let mut detail = vec![
        words
            .msg_with(
                "backup-read-from",
                &[
                    ("read", (read as i64).into()),
                    ("folders", folder_count(words, folders).into()),
                ],
            )
            .into_owned(),
    ];
    if already > 0 {
        detail.push(
            words
                .msg_with(
                    "favorites-already-here",
                    &[("count", (already as i64).into())],
                )
                .into_owned(),
        );
    }
    if created > 0 {
        detail.push(
            words
                .msg_with(
                    "backup-folders-created",
                    &[("count", (created as i64).into())],
                )
                .into_owned(),
        );
    }
    views::page(
        &RestoreDonePage {
            chrome,
            head: step_head(say(locale, "backup-restore-title"), None),
            added: words
                .msg_with("favorites-added", &[("added", (added as i64).into())])
                .into_owned(),
            detail: detail.join(", "),
            left_out: left_out_line(words, unknown),
            unplaced: unplaced_lines(words, &unplaced),
            unreadable: (unreadable > 0).then(|| {
                words
                    .msg_with(
                        "backup-unreadable",
                        &[("count", (unreadable as i64).into())],
                    )
                    .into_owned()
            }),
            // Reported, never enforced — a file refused by an older build is a recovery that did
            // not happen.
            format_note: backup::is_from_the_future(&document)
                .then(|| say(locale, "backup-format-newer")),
        },
        locale,
    )
}

/// The share root for a folder, matching [`views`]'s own.
fn share_root(folder: i64) -> String {
    format!("/favorites/share/{folder}")
}

/// A sentence with a folder's name in it.
fn folder_sentence(words: &Catalog, key: &str, folder: &str) -> String {
    words
        .msg_with(key, &[("folder", folder.into())])
        .into_owned()
}

/// `4 folders`, beside [`song_count`].
fn folder_count(words: &Catalog, folders: usize) -> String {
    words
        .msg_with("count-folders", &[("count", (folders as i64).into())])
        .into_owned()
}

/// `18 songs added, 4 already here`.
fn outcome_line(words: &Catalog, added: usize, already: usize) -> String {
    let mut parts = vec![
        words
            .msg_with("favorites-added", &[("added", (added as i64).into())])
            .into_owned(),
    ];
    if already > 0 {
        parts.push(
            words
                .msg_with(
                    "favorites-already-here",
                    &[("count", (already as i64).into())],
                )
                .into_owned(),
        );
    }
    parts.join(", ")
}

/// What this device's catalog cannot show, said once for both features.
fn left_out_line(words: &Catalog, unknown: usize) -> Option<String> {
    (unknown > 0).then(|| {
        words
            .msg_with("favorites-left-out", &[("count", (unknown as i64).into())])
            .into_owned()
    })
}

/// How many codes one of the sentences below names before it stops.
///
/// A restore onto a phone with no packages installed misses everything, and a sentence carrying
/// eleven hundred numbers is not a sentence. The count is always exact; the list is a sample, and
/// the log line beside it has the rest.
const NAMED_IN_A_LINE: usize = 12;

/// What a folder holds and cannot show, said where somebody is looking at the folder.
///
/// **A folder's count comes from the collection and its rows come from the catalog**, so a phone
/// pointed at a machine without those packages draws a folder that claims more songs than it lists.
/// The count is the honest one — a favorite outliving the package its song came from is a state
/// this app keeps on purpose — and what was missing is the sentence saying so.
///
/// **Its own count line rather than `favorites-left-out`.** That one ends *"so they were left out"*,
/// which is true of a restore deciding what to write and false of a folder being read: nothing was
/// left out of anything here, and the songs come back by pointing the remote at a machine that has
/// them. The two remedy lines underneath are the restore report's own, unchanged.
fn not_here_lines(words: &Catalog, unplaced: &[(SongRef, Miss)]) -> Vec<String> {
    if unplaced.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        words
            .msg_with(
                "favorites-not-here",
                &[("count", (unplaced.len() as i64).into())],
            )
            .into_owned(),
    ];
    lines.extend(unplaced_lines(words, unplaced));
    lines
}

/// The unplaceable favorites, grouped by what would fix them.
///
/// **One line per remedy rather than one per song**, and the grouping *is* the information: a
/// package that is not installed is something the owner can go and install, and a recording that is
/// in none of the packages they do have is not. [`Miss::NumberAbsent`] gets no line here — it is a
/// favorite carrying nothing but a number, which is what `favorites-left-out` has always counted.
fn unplaced_lines(words: &Catalog, unplaced: &[(SongRef, Miss)]) -> Vec<String> {
    let mut lines = Vec::new();
    for (reason, key) in [
        (Miss::PackageAbsent, "favorites-missing-package"),
        (Miss::RecordingAbsent, "favorites-missing-recording"),
    ] {
        let codes: Vec<String> = unplaced
            .iter()
            .filter(|(_, miss)| *miss == reason)
            .map(|(asked, _)| asked.code.to_string())
            .collect();
        if codes.is_empty() {
            continue;
        }
        // The count is over every one of them; the list is trimmed. A sentence that said "12 songs"
        // and then named twelve out of nine hundred would be the lie this split avoids.
        let named = codes
            .iter()
            .take(NAMED_IN_A_LINE)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let named = if codes.len() > NAMED_IN_A_LINE {
            format!("{named}…")
        } else {
            named
        };
        lines.push(
            words
                .msg_with(
                    key,
                    &[
                        ("count", (codes.len() as i64).into()),
                        ("codes", named.as_str().into()),
                    ],
                )
                .into_owned(),
        );
    }
    lines
}

/// Keeps the fan-out fed.
///
/// One task per process, subscribing to the machine's events and turning each into the fragments the
/// open pages are waiting for. This is where the 250 ms state event is **split**: the player card is
/// republished only when the song or the settings actually change, and the position — which moves
/// constantly and is what the card would otherwise be rebuilt for — goes out at most once a second
/// as a fragment of its own. That split is the reason no control on the Now page needs protecting
/// from being replaced under a finger.
pub fn spawn_pump(state: Remote) {
    tokio::spawn(async move { pump(state).await });
}

/// How often the position fragment goes out while a song plays.
const POSITION_INTERVAL: Duration = Duration::from_millis(1000);

/// How often the connection is re-examined when nothing else is happening.
const CONNECTION_INTERVAL: Duration = Duration::from_millis(1000);

async fn pump(state: Remote) {
    let mut events = state.machine.subscribe();
    let mut ticker = tokio::time::interval(CONNECTION_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut last_position = tokio::time::Instant::now() - POSITION_INTERVAL;
    let mut last_card: Option<Vec<(Locale, String)>> = None;
    let mut last_connection: Option<Connection> = None;

    publish_everything(&state).await;

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(km_api::events::Event::State { state: snapshot }) => {
                    let view = PlayerView { state: snapshot, online: state.machine.connection().online };
                    // The card, only when something on it actually changed. Comparing the rendered
                    // markup is both the simplest and the most honest test of that: it is exactly
                    // the question "would a page look different?" — asked of every language at
                    // once, since they are rendered from one state and change together.
                    // The bar goes out with it, and is gated by the same comparison: it is a strict
                    // subset of the card, so a card that did not change did not change the bar.
                    match render_card(&view) {
                        Ok(card) if last_card.as_ref() != Some(&card) => {
                            publish_everywhere(&state, sse::PLAYER, &card);
                            publish_rendered(
                                &state,
                                sse::NOWBAR,
                                everywhere(|_| NowBar::of(view.clone())),
                            );
                            last_card = Some(card);
                        }
                        // Rendered, and identical to what every page already has.
                        Ok(_) => {}
                        // Said out loud rather than swallowed by an `if let`. A card that will
                        // not render leaves `last_card` untouched, so the next event tries it
                        // again — for ever, and with every open page frozen on stale state. The
                        // freeze is not fixed by logging, but "the remote just stops updating
                        // sometimes" becomes a line naming the reason.
                        Err(error) => {
                            tracing::error!(%error, "the player card could not be rendered");
                        }
                    }
                    if last_position.elapsed() >= POSITION_INTERVAL {
                        publish_rendered(
                            &state,
                            sse::POSITION,
                            everywhere(|_| PositionBlock::of(&view)),
                        );
                        // Moved whether or not that rendered. Inside the success branch it meant a
                        // position block that would not render was retried on every single event
                        // rather than every `POSITION_INTERVAL`.
                        last_position = tokio::time::Instant::now();
                    }
                }
                Ok(km_api::events::Event::QueueChanged { queue }) => {
                    publish_queue(&state, queue.len).await;
                }
                Ok(km_api::events::Event::SongStarted { .. })
                | Ok(km_api::events::Event::SongEnded { .. })
                | Ok(km_api::events::Event::SettingsChanged { .. }) => {
                    last_card = None;
                    publish_player(&state).await;
                }
                // The stream admitted it dropped events. Re-fetch rather than carry on: a queue that
                // is believed current and is not shows the wrong person as next.
                Ok(km_api::events::Event::Desync { skipped }) => {
                    tracing::debug!(skipped, "resynchronizing after a gap in the event stream");
                    last_card = None;
                    publish_everything(&state).await;
                }
                Ok(_) => {}
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "the pump fell behind the machine's events");
                    last_card = None;
                    publish_everything(&state).await;
                }
                Err(RecvError::Closed) => {
                    tracing::debug!("the machine's event stream closed; the pump is stopping");
                    return;
                }
            },
            _ = ticker.tick() => {
                let connection = state.machine.connection();
                if last_connection.as_ref() != Some(&connection) {
                    publish_connection(&state, &connection).await;
                    // Coming back after an absence means everything on every open page is stale.
                    if connection.online {
                        last_card = None;
                        publish_everything(&state).await;
                    }
                    last_connection = Some(connection);
                }
            }
        }
    }
}

/// One fragment, rendered once for every language this build speaks.
///
/// **The pump has no viewer to ask.** It is one task per process feeding every open page, while the
/// language is a choice each device made for itself — so it renders the lot and [`sse::Hub`] hands
/// each page its own copies. `Locale::ALL` is two, and the usual state of a machine under a
/// television is nobody watching at all.
///
/// **A builder rather than a template**, because a fragment may have to be *built* per language and not
/// only rendered per language: a sentence with a count in it is composed in Rust through
/// `Catalog::msg_with` and arrives as a field, which is one field per locale. Most fragments ignore
/// the argument and are `|_|`.
///
/// Going through [`views::render`] rather than `Template::render` is the whole of the fix this
/// exists for: a bare render puts no catalog in askama's values store, and every `{{ "key"|t }}`
/// under it comes out as `⟦key⟧` — correct on the page as it loaded and overwritten a second later
/// by the fan-out's replay.
fn everywhere<T: Template>(
    build: impl Fn(Locale) -> T,
) -> Result<Vec<(Locale, String)>, askama::Error> {
    Locale::ALL
        .iter()
        .map(|&locale| views::render(&build(locale), locale).map(|html| (locale, html)))
        .collect()
}

/// Publishes one fragment's languages, all of them.
fn publish_everywhere(state: &Remote, event: &'static str, copies: &[(Locale, String)]) {
    for (locale, html) in copies {
        state.hub.publish(*locale, event, html.clone());
    }
}

/// Publishes a fragment that has just been rendered, or says why it could not be.
///
/// **An `if let Ok(copies) = …` with no `else` makes a render failure the quietest thing this file
/// can do**, which is what this exists to prevent. A missing key in one locale's catalog, or a
/// `Catalog::msg_with` argument that does not match its message, and the whole publish vanishes —
/// for every language, since they are rendered together. Worse, the pump's `last_card` is assigned
/// only *inside* the success branch, so a card that will not render is re-attempted on every event,
/// fails every time, and leaves every open page frozen on stale state.
///
/// `Event::Desync` and a lagged receiver both get a `tracing::debug!` a few lines below. A fragment
/// that will not render is strictly worse than either, which is the whole argument for one function
/// here rather than the pattern repeated nine times.
fn publish_rendered(
    state: &Remote,
    event: &'static str,
    copies: Result<Vec<(Locale, String)>, askama::Error>,
) {
    match copies {
        Ok(copies) => publish_everywhere(state, event, &copies),
        Err(error) => {
            tracing::error!(%error, event, "a fragment could not be rendered, so it was not sent");
        }
    }
}

fn render_card(view: &PlayerView) -> Result<Vec<(Locale, String)>, askama::Error> {
    everywhere(|_| PlayerBlock::of(view.clone()))
}

/// The card and the bar, which are one state rendered for two pages.
async fn publish_player(state: &Remote) {
    let view = player_view(state).await;
    publish_rendered(
        state,
        sse::PLAYER,
        everywhere(|_| PlayerBlock::of(view.clone())),
    );
    publish_rendered(state, sse::NOWBAR, everywhere(|_| NowBar::of(view.clone())));
}

async fn publish_queue(state: &Remote, len: usize) {
    let queue = queue_block(state).await;
    publish_rendered(
        state,
        sse::QUEUE,
        everywhere(|_| QueueBlock {
            rows: queue.rows.clone(),
            online: queue.online,
        }),
    );
    publish_rendered(state, sse::QUEUE_COUNT, everywhere(|_| QueueCount { len }));
}

/// The dot, the banner, and — where this build has one — the machine card.
///
/// Async because of the third: the card is asked of [`crate::machine::Connect`], which is a trait
/// with `async` methods. Both callers were already `async`.
async fn publish_connection(state: &Remote, connection: &Connection) {
    publish_rendered(
        state,
        sse::CONN,
        everywhere(|_| Conn {
            online: connection.online,
        }),
    );
    publish_rendered(
        state,
        sse::BANNER,
        everywhere(|_| Banner {
            connection: connection.clone(),
        }),
    );
    // Nothing at all in the online mode, rather than an empty fragment: a page that never asked for
    // this event has nowhere to put it.
    if let Some(status) = machine_status(state).await {
        publish_rendered(
            state,
            sse::MACHINE,
            everywhere(|locale| MachineBlock::of(status.clone(), locale)),
        );
    }
}

async fn publish_everything(state: &Remote) {
    publish_player(state).await;
    let len = state.machine.queue().await.map(|q| q.len).unwrap_or(0);
    publish_queue(state, len).await;
    publish_connection(state, &state.machine.connection()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_request_is_the_signal_to_restore_where_somebody_was() {
        assert!(BrowseParams::default().is_bare());
        assert!(
            !BrowseParams {
                q: Some("tempo".to_owned()),
                ..Default::default()
            }
            .is_bare()
        );
    }

    #[test]
    fn a_remembered_state_parses_back_into_parameters() {
        let params = BrowseParams::from_state("mode=artists&q=tom%20jobim&language=pt&folder=4");
        assert_eq!(params.mode.as_deref(), Some("artists"));
        assert_eq!(params.q.as_deref(), Some("tom jobim"));
        assert_eq!(params.language.as_deref(), Some("pt"));
        assert_eq!(params.folder, Some(4));
    }

    /// A link shared from the offline app, opened on the machine's own remote. It lands on the songs
    /// list rather than on a page saying no.
    #[test]
    fn a_mode_this_build_does_not_have_falls_back_rather_than_failing() {
        let params = BrowseParams {
            mode: Some("favorites".to_owned()),
            ..Default::default()
        };
        assert_eq!(params.mode(crate::Capabilities::online()), Mode::Songs);
        assert_eq!(params.mode(crate::Capabilities::offline()), Mode::Favorites);
    }

    /// "Nothing matches" is the unhelpful version. What somebody needs is which of the things they
    /// have set is hiding everything.
    #[test]
    fn an_empty_list_says_which_filter_emptied_it() {
        let english = crate::words::messages(Locale::English);
        assert!(empty_text(english, "songs", "tempo", Some('J'), None).contains("starting with J"));
        assert!(empty_text(english, "songs", "", None, Some("Party")).contains("Party is empty"));
        assert!(empty_text(english, "folders", "", None, None).contains("No favorites yet"));
        assert!(empty_text(english, "artists", "jobim", None, None).contains("jobim"));
    }

    /// And it says it in the viewer's language.
    ///
    /// The table was eleven `format!`s, so a Portuguese page said "No song starts with J" in the
    /// middle of an otherwise translated list — the quiet half of the same fault the fan-out had
    /// loudly. Every arm is checked because a table is exactly where one arm gets missed.
    #[test]
    fn an_empty_list_says_it_in_the_viewers_language() {
        let portuguese = crate::words::messages(Locale::BrazilianPortuguese);
        let cases = [
            ("songs", "", None, None),
            ("songs", "tempo", None, None),
            ("songs", "", Some('J'), None),
            ("songs", "tempo", Some('J'), None),
            ("songs", "", None, Some("Festa")),
            ("songs", "tempo", None, Some("Festa")),
            ("songs", "", Some('J'), Some("Festa")),
            ("artists", "", None, None),
            ("artists", "jobim", None, None),
            ("folders", "", None, None),
            ("folders", "festa", None, None),
        ];
        for (kind, query, initial, folder) in cases {
            let sentence = empty_text(portuguese, kind, query, initial, folder);
            assert!(
                !sentence.contains('⟦'),
                "`{kind}`/`{query}` has no Portuguese: {sentence}"
            );
        }
    }

    #[test]
    fn the_next_page_link_keeps_every_filter() {
        let params = BrowseParams {
            mode: Some("songs".to_owned()),
            q: Some("tempo".to_owned()),
            language: Some("pt".to_owned()),
            ..Default::default()
        };
        let query = next_query(&params, 50);
        assert!(query.contains("q=tempo"));
        assert!(query.contains("language=pt"));
        assert!(query.ends_with("offset=50"));
    }

    #[test]
    fn only_an_htmx_request_gets_a_fragment() {
        let params = BrowseParams {
            fragment: Some("list".to_owned()),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        assert_eq!(wanted_fragment(&params, &headers), None);
        headers.insert("HX-Request", "true".parse().expect("a header"));
        assert_eq!(wanted_fragment(&params, &headers), Some("list"));
    }

    /// **Every fragment the fan-out pushes, in every language, with its words in it.**
    ///
    /// The fault this pins is the one a person reported from the sofa: the Now tab drew
    /// `⟦control-key⟧`, `⟦nobody-singing⟧` and `⟦tab-book⟧` a second after loading correctly. The
    /// pump rendered through `Template::render` rather than [`views::render`], so askama's values
    /// store held no catalog and [`km_locale::filters::t`] answered every key with its own name.
    ///
    /// It is asserted here rather than over the router because these eight are the whole of what the
    /// pump publishes and none of them needs a machine to build — and because the marker is one
    /// character, so the check is exact rather than a list of keys that would go stale.
    #[test]
    fn no_fragment_the_fan_out_pushes_is_ever_missing_its_words() {
        let idle = PlayerView::unreachable();
        let mut waiting = PlayerView::unreachable();
        waiting.online = true;

        let status = crate::machine::MachineStatus {
            connection: Connection {
                online: true,
                // An address is what draws *Open in browser* and the book link, which are two of
                // the keys the report named.
                address: Some("http://127.0.0.1:8177".to_owned()),
                name: Some("Living Room".to_owned()),
                reason: None,
            },
            how: Some("remembered".to_owned()),
            pinned: true,
            songs: 394,
            can_browse: true,
        };

        let mut fragments: Vec<(&str, Vec<(Locale, String)>)> = Vec::new();
        let mut add = |event: &'static str,
                       copies: Result<Vec<(Locale, String)>, askama::Error>| {
            fragments.push((event, copies.expect("the fragment renders")));
        };
        add(
            sse::PLAYER,
            everywhere(|_| PlayerBlock::of(waiting.clone())),
        );
        add(sse::NOWBAR, everywhere(|_| NowBar::of(waiting.clone())));
        add(sse::POSITION, everywhere(|_| PositionBlock::of(&idle)));
        add(
            sse::QUEUE,
            everywhere(|_| QueueBlock {
                rows: Vec::new(),
                online: true,
            }),
        );
        add(sse::QUEUE_COUNT, everywhere(|_| QueueCount { len: 3 }));
        add(sse::CONN, everywhere(|_| Conn { online: false }));
        add(
            sse::BANNER,
            everywhere(|_| Banner {
                connection: Connection {
                    online: false,
                    address: Some("http://127.0.0.1:8177".to_owned()),
                    name: None,
                    reason: Some(crate::machine::codes::NOT_ANSWERING),
                },
            }),
        );
        add(
            sse::MACHINE,
            everywhere(|locale| MachineBlock::of(status.clone(), locale)),
        );

        for (event, copies) in fragments {
            assert_eq!(
                copies.len(),
                Locale::ALL.len(),
                "`{event}` was not rendered in every language"
            );
            for (locale, html) in copies {
                assert!(
                    !html.contains('⟦'),
                    "`{event}` in {} pushes a key rather than a word: {html}",
                    locale.tag()
                );
            }
        }
    }

    /// **`views::render` is the only thing in this crate that renders a template.**
    ///
    /// The test above says the eight fragments are right today; this one says a ninth cannot be
    /// added the way the eight were broken. `Template::render` takes no values store, so any call to
    /// it outside `views.rs` is a fragment with no catalog — which is legible on the page and is
    /// nobody's idea of a remote.
    ///
    /// `views.rs` is exempt because its own tests render deliberately without a locale: one pins the
    /// escaping of a song title, and one pins what the missing-locale marker looks like.
    #[test]
    fn nothing_outside_views_renders_a_template_without_a_locale() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // Spelled rather than written, so this file does not report itself.
        let needle = format!(".{}()", "render");
        let mut offenders = Vec::new();
        for entry in std::fs::read_dir(&src).expect("the source directory is readable") {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_none_or(|ext| ext != "rs")
                || path.file_name().is_some_and(|name| name == "views.rs")
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file is readable");
            for (number, line) in text.lines().enumerate() {
                if line.contains(&needle) {
                    offenders.push(format!(
                        "{}:{}: {}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these render with no catalog, so every `|t` under them becomes ⟦key⟧ \
             — go through `views::render`, or `handlers::everywhere` for a pushed fragment:\n{}",
            offenders.join("\n")
        );
    }
}
