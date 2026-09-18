//! The route handlers.
//!
//! Two shapes, and the distinction matters for how a page behaves. A `GET` returns either a full page
//! or the one fragment htmx asked for. A `POST` returns a [`MessageFragment`] — a line of text saying
//! what happened — which is swapped into a slot on the page. Nothing redirects, because a redirect
//! after an htmx post reloads the whole page and loses the filter somebody spent a minute setting up.

use std::path::PathBuf;

use axum::extract::{Path as UrlPath, Query, RawQuery, State as AxumState};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};

use km_song::{ParseOptions, Song};

use crate::app::Client;
use crate::db::{
    AddedFilter, CopiesFilter, DbError, FavoritedFilter, Filter, Initial, LanguageFilter,
    LyricSearch, ScoreFilter, SongEdit, Sort, SuitabilityFilter, VersionsFilter,
};
use crate::form::Fields;
use crate::model::{PackageRow, SongKind};
use crate::scan::ScanOptions;
use crate::server::{DEFAULT_APP_URL, LAST_PLAYED_SETTING, SimilarNarrowing, State};
use crate::views::{
    ActiveFilter, Choice, Chrome, DiscoveredFragment, DiscoveredMachine, DuplicatesPage,
    FavoritesPage, FilterForm, LyricHits, LyricSearchPage, LyricsFragment, MachineAccessFragment,
    MessageFragment, OpenFolders, OpenListing, OpenPage, OpenProgress, PackagePage, PackagesPage,
    PlayedFragment, ProgressFragment, RawFragment, RecentView, ScanPage, SettingsPage, SimilarHits,
    SimilarPage, SongPage, SongRowFragment, SongRows, SongsPage, Toast, page,
};

/// Encodings offered when re-decoding lyrics by hand.
///
/// The ones a real corpus actually contains, from `km-song`'s own measurements, rather than every
/// label `encoding_rs` knows. A list of two hundred is not a choice anybody can make.
const ENCODINGS: &[&str] = &[
    "UTF-8",
    "windows-1252",
    "windows-1250",
    "windows-1251",
    "windows-1254",
    "ISO-8859-2",
    "ISO-8859-15",
    "Shift_JIS",
    "EUC-KR",
    "Big5",
    "GBK",
    "windows-874",
];

/// How many rows a page of the browse table holds.
///
/// It was a hundred, and halving it is about what a page is *for* rather than about how fast one
/// renders. A row here is not read, it is judged — a title, a suitability, a length, and five
/// buttons that each do something to that song — so a page is a batch of work somebody finishes,
/// and a hundred is more of that than fits on a screen or in a sitting. Fifty is about two screens,
/// which is where the pager's *next* stops being a scroll to the bottom of a page you have given up
/// on. The cost is more page turns and it is small: the count is carried rather than recomputed
/// (see [`FilterQuery::total`]), so a turn is one keyset query.
const PAGE_SIZE: u32 = 50;

/// `GET /`
///
/// Still `/songs`. The middleware turns this into the Open page when nothing is open, which keeps
/// the rule in one place: a request that needs a folder is redirected, and `/` is one of them.
pub async fn index() -> Redirect {
    Redirect::to("/songs")
}

// -- opening a folder -----------------------------------------------------------------------

/// `GET /open`
pub async fn open_page(AxumState(state): AxumState<State>, query: Query<OpenQuery>) -> Response {
    let locale = state.locale();
    let recent = state
        .recent()
        .folders
        .into_iter()
        .map(|entry| {
            let present = entry.path.is_dir();
            // Asked once, and only of a folder that is there. `browse::indexed` would answer `No`
            // for an absent one anyway, but that answer would be about a drive that is unplugged
            // rather than about what is in the folder — two states the row says differently.
            let indexed = if present {
                crate::browse::indexed(&entry.path)
            } else {
                crate::browse::Indexed::No
            };
            let mut row = RecentView {
                name: entry
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    // A drive root has no final segment, and its whole path is short enough to be
                    // the name.
                    .unwrap_or_else(|| entry.path.display().to_string()),
                path: entry.path.display().to_string(),
                counts: String::new(),
                songs: entry.songs,
                files: entry.files,
                present,
                indexed,
            };
            row.say_counts(locale);
            row
        })
        .collect();

    // Landing in the open corpus's own folder is what makes "switch to the album next door" one
    // click.
    let start = browse_start(query.at.as_ref(), &state);

    page(
        &OpenPage {
            locale: locale.tag(),
            // A job may already be running before this page is first drawn — the startup reopen, and a
            // double-clicked corpus on macOS. See the field's own note.
            opening: state.opening(),
            recent,
            // The path, and no directory read: the browser is closed until somebody presses Browse, and
            // `GET /open/list` works this out the same way when they do.
            start: start.map(|path| path.display().to_string()),
            current: state.root().map(|root| root.display().to_string()),
            windowed: state.is_windowed(),
        },
        locale,
    )
}

/// Where the folder browser starts: what was asked for, else the folder already open, else home.
///
/// Shared by the page and by `GET /open/list`, because the button that opens the browser sends no
/// `at` and has to land where the page says it would. They disagreed until now — the page fell back
/// to home and the listing route fell back to the drives — which was invisible only because the
/// listing was always rendered with the page's answer already in it.
fn browse_start(asked: Option<&String>, state: &State) -> Option<PathBuf> {
    asked
        .map(PathBuf::from)
        .or_else(|| state.root())
        .or_else(crate::browse::start)
}

/// Where the folder browser should look.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct OpenQuery {
    /// The folder to list. Absent means wherever [`browse_start`] says to start.
    #[serde(default)]
    pub at: Option<String>,
    /// Whether to list the drives rather than a folder.
    ///
    /// **A presence flag, and `Option<String>` rather than `bool` on purpose.** Only whether the key
    /// is there is ever read; the value is not. A `bool` here would demand the query string spell
    /// `true` or `false`, because that is the only thing serde's bool deserializer accepts — and no
    /// `hx-get` in this crate spells a flag that way. This one was a `bool`, the crumb sent
    /// `?drives=1`, and every press of ⏶ was answered with a deserialize failure swapped into the
    /// listing. `confirm`, `unpackaged`, `picking` and `editing` are all the same shape for the same
    /// reason; this was the odd one out.
    #[serde(default)]
    pub drives: Option<String>,
    /// Show only folders whose name holds this. Empty is every folder.
    ///
    /// A value rather than a presence flag, so it is a `String` and not the `Option<String>` above
    /// — the distinction that comment draws is about flags, and reading this one as a flag would
    /// make an emptied box mean *narrow by nothing typed* instead of *stop narrowing*.
    #[serde(default)]
    pub filter: String,
    /// Which page of the folders, as a row index. Absent is the first.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Whether to answer with the folders alone rather than the whole listing.
    ///
    /// A presence flag, so `Option<String>` for the reason `drives` above gives at length.
    #[serde(default)]
    pub rows: Option<String>,
}

/// `GET /open/list`
///
/// Bare, this is the Browse button: no `at`, so it starts where [`browse_start`] says, which is the
/// same answer the page's own placeholder shows. `?drives=1` is the ⏶ crumb and means the drives
/// themselves, which is why "absent" cannot mean that as well.
///
/// **Off the runtime, which the rest of this file gets through `State::blocking`.** That helper
/// wants a workspace and the picker is the one page reached without one, so this spawns its own
/// blocking task. It is not a formality: a listing reads a directory per folder it draws, and the
/// executor thread it used to run on is the one every other request is waiting for.
pub async fn open_list(AxumState(state): AxumState<State>, query: Query<OpenQuery>) -> Response {
    let ask = crate::browse::Ask {
        here: match query.drives.is_some() {
            true => None,
            false => browse_start(query.at.as_ref(), &state),
        },
        filter: query.filter.clone(),
        offset: query.offset.unwrap_or(0),
    };
    let folders_only = query.rows.is_some();
    match tokio::task::spawn_blocking(move || crate::browse::list(&ask)).await {
        Ok(mut listing) if folders_only => {
            listing.say_range(state.locale());
            page(&OpenFolders { listing }, state.locale())
        }
        Ok(mut listing) => {
            listing.say_range(state.locale());
            page(&OpenListing { listing }, state.locale())
        }
        Err(error) => failure(
            DbError::Rejected(
                crate::words::messages(state.locale())
                    .msg_with(
                        "said-folder-not-listed",
                        &[("why", error.to_string().as_str().into())],
                    )
                    .into_owned(),
            ),
            state.locale(),
        ),
    }
}

/// `POST /open/open`
///
/// Starts the job and returns at once; the page polls `/open/progress`. See `State::begin_open` for
/// why opening cannot be done inside this request.
pub async fn open_folder(AxumState(state): AxumState<State>, body: String) -> Response {
    let fields = Fields::parse(&body);
    let Some(path) = fields.one("path").filter(|path| !path.trim().is_empty()) else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-no-folder-given"),
        );
    };
    let create = fields.one("create").is_some();
    let root = crate::model::tidy(&PathBuf::from(path.trim()));

    match state.begin_open(root, create) {
        Ok(()) => page(
            &OpenProgress::new(state.opening(), false, state.locale()),
            state.locale(),
        ),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `GET /open/progress`
pub async fn open_progress(AxumState(state): AxumState<State>) -> Response {
    page(
        &OpenProgress::new(state.opening(), state.workspace().is_some(), state.locale()),
        state.locale(),
    )
}

/// `POST /open/forget`
pub async fn open_forget(AxumState(state): AxumState<State>, body: String) -> Response {
    let fields = Fields::parse(&body);
    let Some(path) = fields.one("path") else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-no-folder-given"),
        );
    };
    state.forget_recent(&PathBuf::from(path.trim()));
    MessageFragment::ok(crate::words::messages(state.locale()).msg("said-forgotten"))
}

/// `POST /open/close`
pub async fn open_close(AxumState(state): AxumState<State>) -> Response {
    state.close_folder();
    (
        [("hx-redirect", crate::server::OPEN_PATH)],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

/// `POST /quit`
///
/// The way out for a tool with no terminal. See the shutdown arm in `main`.
///
/// **The header only offers this in a browser.** In this tool's own window, closing the window asks
/// for exactly this shutdown already, so the button was a second X beside the X. The route stays
/// regardless — it costs nothing, it is what a `--browser` run and every Linux build use, and
/// removing a route because one page stopped drawing a button for it is how a thing becomes
/// unreachable by anything else.
pub async fn quit(AxumState(state): AxumState<State>) -> Response {
    state.ask_to_quit();
    page(
        &MessageFragment {
            text: "Stopping. You can close this window.".to_owned(),
            ok: true,
        },
        state.locale(),
    )
}

/// `POST /browser`
///
/// Hands the page to whatever browser this machine considers default. Only the windowed build offers
/// it, and only because a webview is not a browser: there is no address bar to copy the address out
/// of, no bookmark to make, and no second window to open — so somebody who wants the tool in a real
/// tab had to find the address printed in a console that a double-click never gave them.
///
/// A server round-trip rather than a link, and it stays one even now that the window denies a new
/// window request and hands the address out itself (`desktop::open_outside`). That fixes
/// `target="_blank"`; it does not give this button a URL to put in an `href`, because the one it
/// wants is *this page* and only the browser knows what that is after a filter has been pushed into
/// the address bar. So the page says where it is and the server opens it — the same `km_osopen` the
/// *Open in OS* button on a row uses, with the same fact holding: it opens on the machine running
/// this tool, which for a loopback tool is the machine looking at it.
///
/// **All four answers are toasts, where the Quit button beside it keeps its slot.** This says a
/// thing happened and is over — a window opened somewhere else — which is what a toast is for, and
/// the slot it used sits inside the header, where a sentence naming an address widens the one strip
/// every page is measured against. *Stopping. You can close this window.* is the opposite claim: it
/// describes the state the page is now in for good, so it must not fade after eight seconds onto a
/// header that has gone quiet.
///
/// **The address is checked against our own before it is opened.** It arrives from the page, and it
/// ends up as an argument to `cmd /c start`, which opens files and programs as readily as pages. A
/// prefix match on [`State::url`] is the whole guard: anything else silently becomes the plain
/// address, because a person who pressed *Open in browser* wants a browser rather than an
/// explanation.
pub async fn open_in_browser(AxumState(state): AxumState<State>, body: String) -> Response {
    let base = state.url();
    if base.is_empty() {
        return crate::views::toast_only(&Toast::bad(
            crate::words::messages(state.locale()).msg("said-no-address-yet"),
        ));
    }
    let url = browser_target(Fields::parse(&body).one("url"), &base);
    match tokio::task::spawn_blocking(move || km_osopen::open_url(&url)).await {
        Ok(Ok(())) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale()).msg("said-opened-in-browser"),
        )),
        Ok(Err(error)) => {
            crate::views::toast_only(&Toast::bad(format!("Could not open a browser: {error}")))
        }
        Err(error) => {
            crate::views::toast_only(&Toast::bad(format!("The worker thread died: {error}")))
        }
    }
}

/// Which address [`open_in_browser`] actually hands over: what the page asked for if it is one of
/// ours, else the plain address.
///
/// Split out because it is the whole of the security of that route and the only part of it that can
/// be asserted without launching a browser. `base` ends in `/` (`lib.rs` builds it that way), so a
/// prefix match cannot be satisfied by a host that merely *starts* with ours.
fn browser_target(asked: Option<&str>, base: &str) -> String {
    match asked {
        Some(asked) if asked.starts_with(base) => asked.to_owned(),
        _ => base.to_owned(),
    }
}

// -- browsing -------------------------------------------------------------------------------

/// The filter as it arrives from the query string.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct FilterQuery {
    #[serde(default)]
    q: String,
    /// Which band of the automatic suitability: `8-10` · `5-7` · `0-4`, or empty for any.
    ///
    /// **One spelling, and it is a band.** No `min_score` field sits here folding a `≥ N` ladder onto
    /// the band holding N: nothing has shipped that could hold such a bookmark. `duplicates` below
    /// keeps a wider shape for its own reason, which is a control the *Duplicates* page reaches.
    #[serde(default)]
    suitability: String,
    #[serde(default)]
    user_score: String,
    #[serde(default)]
    initial: String,
    /// Exactly this artist, folded — normally arrived at by clicking one in a row.
    ///
    /// **The name is `artist` here and `row_artist` in a row**, which is the `row_language` rule read
    /// the same way round: the filter bar owns the plain spelling and a control inside `#rows` takes
    /// the prefix, because `#rows` rides in the same body as this bar and serde answers a repeated
    /// known key with `duplicate_field`. See [`Self::from_body`].
    #[serde(default)]
    artist: String,
    /// Whether a song has to be filed: `in` · `out`, or empty for either.
    ///
    /// A string and not the presence flag `unpackaged` beside it is, because there are three
    /// answers rather than two -- see [`FavoritedFilter`].
    #[serde(default)]
    favorited: String,
    #[serde(default)]
    favorite: String,
    #[serde(default)]
    melody: String,
    #[serde(default)]
    encoding_source: String,
    #[serde(default)]
    granularity: String,
    /// `midi`, `video`, or empty for both.
    #[serde(default)]
    kind: String,
    /// An ISO 639-1 code, or `unset` / `set`, or empty for any.
    #[serde(default)]
    language: String,
    /// The tags being narrowed by, comma-joined — `rock,brasil`.
    ///
    /// One scalar and never a repeated key, for the reason the doc comment on
    /// [`FilterQuery::from_body`] records at length: `#filters` rides in the same body as every bulk
    /// action, `serde_urlencoded` answers a repeated known key with a 400, and htmx does not swap on
    /// an error — so the control would simply stop working with nothing said anywhere. A comma
    /// cannot occur inside a slug, so the join is unambiguous.
    #[serde(default)]
    tags: String,
    /// A tag being *added* to that set, from the bar's picker.
    ///
    /// `add_tag` rather than `tags` because the current set rides the same form as a hidden field,
    /// and two `tags` keys in one request is the 400 above. Cleared on every render, the set having
    /// already absorbed it.
    #[serde(default)]
    add_tag: String,
    /// How many copies on disk: `1` · `2-10` · `10+`, or empty for any.
    ///
    /// **The `has copies` checkbox this replaced is no longer read**, and it went with the `2+`
    /// bucket rather than on its own: *two or more* was the only thing that checkbox could mean, so
    /// with that bucket gone from the bar there is nothing left for `?duplicates=1` to fold onto.
    /// Such a link now shows the whole corpus. The *Duplicates* page is where that question went.
    #[serde(default)]
    copies: String,
    /// How long ago the song was added: `1d` · `7d` · `30d` · `30d+`, or empty for any.
    #[serde(default)]
    added: String,
    /// Whether to show every version of a recording: `all`, or empty to collapse them.
    #[serde(default)]
    versions: String,
    #[serde(default)]
    unpackaged: Option<String>,
    /// Not a filter: whether each row shows the name of its file beside its title. It travels with
    /// the filters because it is set while browsing and has to survive a page turn, and because the
    /// one form on the page is what sends all of them.
    ///
    /// Read through [`Self::filenames`], which is the one place the default lives.
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    sort: String,
    #[serde(default)]
    offset: Option<u32>,
    /// How many songs this filter matched, carried back by the page it was counted on.
    ///
    /// **Not a filter, and not trusted for anything that decides what is on the page.** It exists so
    /// that turning a page does not re-count the corpus: the count is the same for every page of one
    /// filter, so it is computed on the first render and then travels in the paging links. Absent —
    /// a fresh page load, or any change to the filters, which is exactly when it would be wrong — it
    /// is computed again.
    ///
    /// What it may affect is the `page 2 of N` label and where the five-pages-on button clamps.
    /// Whether a next page *exists* comes from the database on every request (`Db::songs_page`), so
    /// a hand-edited or stale value cannot hide a page that is there or offer one that is not.
    #[serde(default)]
    total: Option<u32>,
}

impl FilterQuery {
    /// The filter as the bar is set **right now**, read out of a POST body.
    ///
    /// This is the source of truth for every action that works on "everything the filter matches",
    /// and it exists because the obvious alternative is wrong in a way that took a package of
    /// hundreds of thousands of songs to notice.
    ///
    /// The obvious alternative is to bake the filter into each action's `hx-post` attribute as a
    /// query string, rendered with the page. But **the filter bar never re-renders the page** — it
    /// swaps `#rows` and nothing else — so the moment somebody picks a filter from the bar, that
    /// attribute describes a page that is gone. Narrow the list to seventeen thousand songs, press
    /// *Make*, and the server is asked about the whole corpus and answers honestly: *the whole
    /// corpus, because no filter is narrowing it*.
    ///
    /// So the bar's own fields ride in the body (`hx-include="#filters"`) and are read here. A form
    /// body is a query string in every respect that matters, so this is the same deserializer
    /// `Query<FilterQuery>` uses and the two cannot drift.
    ///
    /// **A repeated key is an error, not a last-one-wins**, which is why no other form included in
    /// the same body may reuse one of these names — the bulk language set's own select is
    /// `set_language` for exactly this reason, and the same rule is what keeps `#rows` (which sends
    /// `song_id`, `score`, `title`, `row_artist`, `row_language` and `name`) safe to include beside
    /// the bar. Reporting a repeat beats guessing, because guessing writes to the wrong set of songs.
    /// Keys this struct does not know are ignored, repeats and all, which is what makes that
    /// inclusion legal.
    ///
    /// **That list is not static, and it is what makes adding a filter here a two-file change.** The
    /// row's artist box was plain `artist` until this struct grew an `artist` filter; the two would
    /// then have collided in every body carrying both, but *only while a row happened to be open for
    /// editing* — a 400 that comes and goes with something that looks unrelated. So a new filter
    /// whose name a row already uses renames the **row**, which is the third time that has been the
    /// answer here. Before adding one, look at what `#rows` sends.
    ///
    /// **`offset` and `total` are not in the bar and must not be**: `#rows` rides in the same body,
    /// and a hidden `offset` on the bar would arrive twice in it for the `duplicate_field` reason
    /// above. A body therefore carries a page only when something outside the bar puts one in it, and
    /// one thing does — `static/ui.js` copies `#rows`'s `data-offset` onto *Title from file name*, so
    /// that writing the ticked rows and redrawing the list leaves the reader on the page they were
    /// working through. A `data-` attribute is not a field and repeats nothing.
    ///
    /// It is deliberately fallible and there is deliberately nothing to fall back **to**: an
    /// unreadable filter is not an empty one, and an empty one here is the whole corpus.
    fn from_body(body: &str) -> Result<Self, String> {
        serde_urlencoded::from_str(body)
            .map_err(|error| format!("could not read the filter from the form: {error}"))
    }

    /// Which band of the automatic suitability a song has to be in.
    ///
    /// **A band, and nothing folding onto it.** A `min_score` fold would exist to keep a bookmark
    /// written against a `≥ N` ladder narrowing the list; nothing has shipped, so there is no such
    /// bookmark, and the field would keep a second spelling of one filter alive for nobody. See `No compatibility aliases` in docs/decisions/songs.md.
    fn suitability(&self) -> SuitabilityFilter {
        SuitabilityFilter::parse(&self.suitability)
    }

    /// Whether each row shows the name of its file beside its title.
    ///
    /// **Off unless the box says otherwise.** The argument for on — that a great many of this
    /// corpus's titles say less than the name on disk — is true, and is not what the default is
    /// deciding. What it decides is how wide a row is: the chip carries a whole base name with its
    /// extension, on every line, and `td.actions` is `width: 1%`, so the space comes out of Title and
    /// Artist until the table stops fitting and `.scroll` starts scrolling sideways. In the tool's
    /// own window that is the usual case rather than the narrow one. A box that is turned on when it
    /// is wanted costs one click; a table that overflows costs the two columns you were reading.
    ///
    /// An absent key is therefore *off*, which is what an unticked checkbox sends — so no
    /// `filename_set` marker is needed to tell the two apart, and this is one line rather than a
    /// state machine. `filename=1` from the bar, from a paging link or from a
    /// typed URL all mean the same thing.
    fn filenames(&self) -> bool {
        self.filename.is_some()
    }

    /// The tags being narrowed by, with `add_tag` merged in — folded, sorted, de-duplicated.
    ///
    /// Merging here rather than at the one call site is what keeps the picker a *filter* and not a
    /// second state: `to_filter`, the chips and `rebuild` all ask this question and all get the
    /// same answer, so a tag just added cannot be in the rows and missing from the chip strip.
    fn chosen_tags(&self) -> Vec<String> {
        let mut chosen = km_kmpkg::tag::parse_list(&self.tags);
        if let Some(added) = km_kmpkg::Tag::parse(&self.add_tag)
            && !chosen.contains(&added)
        {
            chosen.push(added);
            chosen.sort();
        }
        chosen.into_iter().map(km_kmpkg::Tag::into_string).collect()
    }

    fn to_filter(&self) -> Filter {
        Filter {
            query: (!self.q.trim().is_empty()).then(|| self.q.clone()),
            suitability: self.suitability(),
            user_score: ScoreFilter::parse(&self.user_score),
            initial: Initial::parse(&self.initial),
            // Trimmed, because this box is usually filled by a click and occasionally by hand, and a
            // trailing space is not a different artist. The fold happens in `Filter::to_sql`, so what
            // is carried here is still what a chip has to say back to somebody.
            artist: (!self.artist.trim().is_empty()).then(|| self.artist.trim().to_owned()),
            favorited: FavoritedFilter::parse(&self.favorited),
            favorite: self.favorite.parse().ok(),
            melody: match self.melody.as_str() {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            },
            encoding_source: (!self.encoding_source.is_empty())
                .then(|| self.encoding_source.clone()),
            copies: CopiesFilter::parse(&self.copies),
            added: AddedFilter::parse(&self.added),
            versions: VersionsFilter::parse(&self.versions),
            unpackaged: self.unpackaged.is_some(),
            in_package: None,
            granularity: (!self.granularity.is_empty()).then(|| self.granularity.clone()),
            kind: SongKind::filter(&self.kind),
            language: LanguageFilter::parse(&self.language),
            tags: self.chosen_tags(),
            sort: Sort::parse(&self.sort),
            limit: PAGE_SIZE,
            offset: self.offset.unwrap_or(0),
        }
    }

    /// The bar as the page should draw it.
    ///
    /// `present` is what `Db::languages_present` found, and every language picker on the page is
    /// built from it. Passed in rather than looked up here because this is a pure function of the
    /// query and has no database to ask.
    fn to_form(&self, present: &[km_kmpkg::Language], known_tags: &[String]) -> FilterForm {
        FilterForm {
            // English until `in_language` says otherwise; the one caller that draws a page does.
            locale: km_locale::Locale::English,
            present: present.to_vec(),
            tags: self.chosen_tags(),
            known_tags: known_tags.to_vec(),
            // Marked by `with_hints`, which the one caller with settings to read applies. Empty
            // here means every offered tag is one the corpus actually holds.
            suggested_tags: Vec::new(),
            q: self.q.clone(),
            artist: self.artist.clone(),
            // Through the enum and back, so a nonsense band in a hand-typed URL shows as *any*
            // rather than leaving a select with no option selected.
            suitability: self.suitability().as_str().to_owned(),
            // Through the enum and back, so a nonsense value in a hand-typed URL shows as *any*
            // rather than leaving a select with no option selected.
            user_score: ScoreFilter::parse(&self.user_score).as_str(),
            // Through the enum and back, for the same reason the two filters above are — and here it
            // also folds a link written when the bar had ten digit buttons onto the one that replaced
            // them, so `initial=7` checks `0-9` instead of leaving the strip blank.
            initial: Initial::parse(&self.initial).as_str(),
            // Through the enum and back, for the same reason the two filters above are — and here it
            // also folds the `1` a checkbox sent onto `in`, so such a link shows the select reading
            // *in any favorite* rather than leaving it with nothing chosen.
            favorited: FavoritedFilter::parse(&self.favorited).as_str().to_owned(),
            favorite: self.favorite.clone(),
            melody: self.melody.clone(),
            encoding_source: self.encoding_source.clone(),
            granularity: self.granularity.clone(),
            kind: self.kind.clone(),
            // Through the enum and back, for the same reason the two filters above are.
            language: LanguageFilter::parse(&self.language).as_str(),
            // Through the enum and back too, which is what makes a retired `?copies=2%2B` link show
            // the select reading *any* rather than leaving it with nothing selected.
            copies: CopiesFilter::parse(&self.copies).as_str().to_owned(),
            added: AddedFilter::parse(&self.added).as_str().to_owned(),
            versions: VersionsFilter::parse(&self.versions).as_str().to_owned(),
            unpackaged: self.unpackaged.is_some(),
            filename: self.filenames(),
            sort: Sort::parse(&self.sort).as_str().to_owned(),
        }
    }

    /// Every filter currently narrowing the list, in the order the bar shows them.
    ///
    /// `favorites` is needed only to name the chosen one; a chip reading `favorite: 7` would say
    /// less than no chip at all.
    ///
    /// **Every filter has to appear here, for the same reason they all have to appear in
    /// [`Self::rebuild`]:** one left out is one that goes on narrowing the list with nothing on the
    /// page admitting it, which is indistinguishable from the corpus being smaller than it is.
    fn active(
        &self,
        favorites: &[crate::model::FavoriteNode],
        locale: km_locale::Locale,
    ) -> Vec<ActiveFilter> {
        let words = crate::words::messages(locale);
        let mut chips = Vec::new();
        let mut chip = |key: &str, label: String| {
            chips.push(ActiveFilter {
                label,
                remove: self.without(key),
            });
        };

        if !self.q.trim().is_empty() {
            chip("q", format!("\u{201c}{}\u{201d}", self.q.trim()));
        }
        // *by* rather than quotes, which is what separates this chip from the one above it: they are
        // both text somebody supplied, and the difference between "matched anywhere" and "is exactly
        // this performer" is the only thing that tells them apart at a glance.
        if !self.artist.trim().is_empty() {
            chip(
                "artist",
                words
                    .msg_with("chip-by", &[("artist", self.artist.trim().into())])
                    .into_owned(),
            );
        }
        match Initial::parse(&self.initial) {
            Initial::Any => {}
            other => chip("initial", other.describe(locale)),
        }
        match self.suitability() {
            SuitabilityFilter::Any => {}
            other => chip("suitability", other.describe(locale)),
        }
        {
            let name = "your score";
            match ScoreFilter::parse(&self.user_score).as_str().as_str() {
                "" => {}
                "set" => chip(
                    "user_score",
                    words
                        .msg_with("chip-score-set", &[("name", name.into())])
                        .into_owned(),
                ),
                "unset" => chip(
                    "user_score",
                    words
                        .msg_with("chip-score-unset", &[("name", name.into())])
                        .into_owned(),
                ),
                number => chip(
                    "user_score",
                    words
                        .msg_with(
                            "chip-score-at-least",
                            &[("name", name.into()), ("score", number.into())],
                        )
                        .into_owned(),
                ),
            }
        }
        match self.melody.as_str() {
            "yes" => chip("melody", words.msg("chip-melody-found").into_owned()),
            "no" => chip("melody", words.msg("chip-melody-abstained").into_owned()),
            _ => {}
        }
        match self.kind.as_str() {
            "midi" => chip("kind", words.msg("chip-midi-only").into_owned()),
            "video" => chip("kind", words.msg("chip-video-only").into_owned()),
            "cdg" => chip("kind", words.msg("chip-cdg-only").into_owned()),
            _ => {}
        }
        // The name, not the code: a chip is prose, and `ja` is not a word.
        match LanguageFilter::parse(&self.language) {
            LanguageFilter::Any => {}
            other => chip("language", other.describe(locale)),
        }
        // **One chip per tag, not one for all of them**, which is the only filter on this bar that
        // works that way — and it has to, because the tag filter is the only one that holds a set.
        // A single `tags: rock, brasil` chip would offer nothing but *drop both*, and dropping one
        // is the ordinary act. `tag:<slug>` is what `rebuild` reads to take off exactly one.
        for tag in self.chosen_tags() {
            chip(
                &format!("tag:{tag}"),
                words
                    .msg_with("chip-tag", &[("tag", tag.into())])
                    .into_owned(),
            );
        }
        if !self.granularity.is_empty() {
            let key = match self.granularity.as_str() {
                "syllablelevel" => "chip-lyrics-per-syllable",
                "linelevel" => "chip-lyrics-per-line",
                "none" => "chip-no-lyrics",
                // A value nothing here recognizes can only come from a hand-edited query string, and
                // shows itself rather than being worded.
                other => {
                    chip("granularity", other.to_owned());
                    return chips;
                }
            };
            chip("granularity", words.msg(key).into_owned());
        }
        if !self.encoding_source.is_empty() {
            let words = match self.encoding_source.as_str() {
                "fallback" => "guessed",
                "detected" => "detected",
                "utf8" => "UTF-8",
                "declared" => "pinned",
                other => other,
            };
            chip("encoding_source", format!("encoding {words}"));
        }
        if !self.favorite.is_empty() {
            let named = favorites
                .iter()
                .find(|f| f.id.to_string() == self.favorite)
                .map_or_else(|| self.favorite.clone(), |f| f.name.clone());
            chip(
                "favorite",
                words
                    .msg_with("chip-in-favorite", &[("name", named.into())])
                    .into_owned(),
            );
        }
        match FavoritedFilter::parse(&self.favorited) {
            FavoritedFilter::Any => {}
            other => chip("favorited", other.describe(locale)),
        }
        // Keyed on `copies`, which is what `rebuild` writes, so the × on this chip is a complete
        // removal: there is no second key left holding the same filter.
        match CopiesFilter::parse(&self.copies) {
            CopiesFilter::Any => {}
            other => chip("copies", other.describe(locale)),
        }
        match AddedFilter::parse(&self.added) {
            AddedFilter::Any => {}
            other => chip("added", other.describe(locale)),
        }
        match VersionsFilter::parse(&self.versions) {
            VersionsFilter::Collapsed => {}
            other => chip("versions", other.describe(locale)),
        }
        if self.unpackaged.is_some() {
            chip("unpackaged", words.msg("songs-not-packaged").into_owned());
        }
        chips
    }

    /// The chips strip for this filter: the page's own copy, or the one that replaces it.
    ///
    /// Both come from here so that the strip a filter change brings back cannot say something
    /// different from the strip a page load draws.
    fn chips(
        &self,
        favorites: &[crate::model::FavoriteNode],
        oob: bool,
        locale: km_locale::Locale,
    ) -> crate::views::FilterChips {
        crate::views::FilterChips {
            active: self.active(favorites, locale),
            cleared: self.only_view(),
            oob,
        }
    }

    /// The query with every narrowing filter gone and the two view settings kept.
    fn only_view(&self) -> String {
        let mut parts = Vec::new();
        if !self.sort.is_empty() {
            parts.push(format!("sort={}", crate::model::encode(&self.sort)));
        }
        // Said only when it is on, which is all it takes now that off is the default again. See
        // `filenames`.
        if self.filenames() {
            parts.push("filename=1".to_owned());
        }
        parts.join("&")
    }

    /// The same query with a different offset, for the paging buttons.
    /// The same query at another offset, carrying the total forward so the next page need not
    /// re-count what this one already counted.
    fn with_offset(&self, offset: u32, total: u32) -> String {
        self.rebuild(offset, "", Some(total))
    }

    /// The same query with one filter dropped and paging reset, for a *clear* link.
    ///
    /// The total is deliberately **not** carried: dropping a filter changes which songs match, so
    /// the number counted under the old one is not merely stale, it is answering a different
    /// question. Passing `None` is what makes the next render count again.
    fn without(&self, dropped: &str) -> String {
        self.rebuild(0, dropped, None)
    }

    /// Rebuilds the query string.
    ///
    /// Every filter has to be listed here. One left out is not a visible bug — it is a filter that
    /// silently disappears the moment somebody turns a page, which is how it is discovered, three
    /// pages into a corpus of hundreds of thousands of files.
    fn rebuild(&self, offset: u32, dropped: &str, total: Option<u32>) -> String {
        let mut parts = Vec::new();
        let mut push = |key: &str, value: &str| {
            if !value.is_empty() && key != dropped {
                parts.push(format!("{key}={}", crate::model::encode(value)));
            }
        };
        push("q", &self.q);
        // The *normalized* band, not the raw field, so a nonsense value does not survive a page
        // turn. It is also what makes `without("suitability")` a complete removal — there is no
        // second key left holding the same filter. The same rule `copies` follows below.
        push("suitability", self.suitability().as_str());
        push("user_score", &self.user_score);
        // Normalized, like `copies` below: a link written when the bar had ten digit buttons says
        // `initial=7`, and every link written from here on says `initial=0-9`. One filter, one
        // spelling — the old one is read forever and propagated no further than the page it arrived
        // on.
        push("initial", &Initial::parse(&self.initial).as_str());
        push("artist", self.artist.trim());
        push("favorite", &self.favorite);
        push("melody", &self.melody);
        push("encoding_source", &self.encoding_source);
        push("granularity", &self.granularity);
        push("kind", &self.kind);
        push("language", &self.language);
        // The *merged* set, so a tag just added from the picker survives the next page turn — and
        // so `add_tag` itself never propagates: it has done its work by the time this runs, and a
        // link still carrying it would re-add the tag on every press.
        //
        // `dropped` names one tag rather than the key when it starts with `tag:`, which is what
        // makes a chip's ✕ take off exactly one where every other chip takes off a whole filter.
        // The key spelling still works, and clears the lot.
        let one_off = dropped.strip_prefix("tag:");
        let kept: Vec<String> = self
            .chosen_tags()
            .into_iter()
            .filter(|tag| one_off != Some(tag.as_str()))
            .collect();
        if dropped != "tags" {
            push("tags", &kept.join(","));
        }
        // The *normalized* value, not the raw field, so a retired `copies=2+` does not survive the
        // first page turn as a bucket the dropdown cannot show.
        push("copies", CopiesFilter::parse(&self.copies).as_str());
        push("added", AddedFilter::parse(&self.added).as_str());
        push("versions", VersionsFilter::parse(&self.versions).as_str());
        // Normalized like `copies` above, which is also what gives the `1` a checkbox sent one page
        // turn to live and no more.
        push(
            "favorited",
            FavoritedFilter::parse(&self.favorited).as_str(),
        );
        push("sort", &self.sort);
        if self.unpackaged.is_some() && dropped != "unpackaged" {
            parts.push("unpackaged=1".to_owned());
        }
        // Not `push` only because the value is a constant rather than a field: on says `filename=1`
        // and off says nothing, which is what the default being off buys.
        if self.filenames() && dropped != "filename" {
            parts.push("filename=1".to_owned());
        }
        if offset > 0 {
            parts.push(format!("offset={offset}"));
        }
        if let Some(total) = total {
            parts.push(format!("total={total}"));
        }
        parts.join("&")
    }
}

/// `GET /songs`
pub async fn songs(
    AxumState(state): AxumState<State>,
    RawQuery(raw): RawQuery,
    Query(query): Query<FilterQuery>,
) -> Response {
    // **An address with no `?` at all is answered with the remembered filter**, which is what makes
    // a folder come back to what it was left on. Nothing that has a filter to state arrives here
    // bare: `/` redirects, the Open page's *back* link and the fragment that follows an open both
    // send a plain `/songs`, and so does a typed address. Each of those means *the songs page* and
    // none of them means *the whole corpus*.
    //
    // **`/songs?` means the whole corpus and is left alone**, which is the distinction the two
    // states of `RawQuery` carry: *clear all* is `href="/songs?{cleared}"` and clearing the last
    // filter leaves that empty. Somebody who has just pressed it must not be handed back what they
    // cleared.
    if raw.is_none() {
        let remembered = state.songs_filter();
        if !remembered.is_empty() {
            return Redirect::to(&format!("/songs?{remembered}")).into_response();
        }
    }
    let rows = match rows_for(&state, &query).await {
        Ok(rows) => rows,
        Err(error) => return failure(error, state.locale()),
    };
    // Recorded here as well as in `song_rows`, because this is how a filter arrives that the bar
    // never set: Favorites links `/songs?favorite=…` and a package's page links
    // `/songs?unpackaged=1`. Somebody who follows one of those and then goes
    // to look at something else should come back to what they were sent to, not to the corpus.
    //
    // After the rows and before `chrome`: the offset written down is the one being *shown*, which a
    // request landing past the end of the corpus has had clamped (see `rows_for`), and `chrome` is
    // what turns the record into this render's own nav link.
    //
    // **With the total it just counted**, because this string is what the nav tab links to and what
    // a bare `/songs` is redirected to. Without it every arrival at the tab re-counted the filtered
    // corpus — a bare `COUNT(*)` with no `LIMIT` on it — to label a page the count does not decide.
    // It is the same reuse the paging links already make, and `FilterQuery::total`'s own contract is
    // what makes it safe: `has_more` comes from the query on every request, and `rows_for` raises a
    // stale total to what is demonstrably on screen.
    state.remember_songs_filter(query.rebuild(rows.offset, "", Some(rows.total)));
    let chrome = match chrome(&state, "songs").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    let extras = state
        .reading(|db| {
            Ok((
                db.favorites()?,
                // The packages a song may still be added to one at a time. A sourced package holds
                // what its favorites hold, so a song put into it from here would be taken straight
                // back out by the next sync with nothing to say why.
                db.packages_taking_songs()?,
                db.languages_present()?,
                db.tags_present()?,
                // In the same closure as the rest, so the strip costs no extra round trip.
                db.saved_filters()?,
            ))
        })
        .await;
    let (favorites, packages, present, corpus_tags, saved) = match extras {
        Ok(found) => found,
        Err(error) => return failure(error, state.locale()),
    };
    // What the corpus holds, then the suggestions it does not -- facts before advice. The two are
    // told apart in the markup so a hint is never read as a fact; see `settings::Settings`.
    let settings = state.settings();
    let known_tags = settings.offerable_tags(&corpus_tags);
    let suggested_tags = settings.hints(&corpus_tags);

    page(
        &SongsPage {
            chrome,
            rows,
            chips: query.chips(&favorites, false, state.locale()),
            saved: crate::views::SavedFilters {
                filters: offerable(saved, &favorites, state.locale()),
                oob: false,
                renaming: false,
            },
            favorites,
            packages,
            query: query
                .to_form(&present, &known_tags)
                .with_hints(suggested_tags)
                .in_language(state.locale()),
        },
        state.locale(),
    )
}

/// `GET /songs/rows`
/// This is the only route the filter bar and the paging buttons call, so it is the only place that
/// knows the filter has changed — which makes it responsible for the two things on the page that are
/// drawn from the filter and live outside `#rows`:
///
/// * the chips strip, sent back out of band, because otherwise the page goes on saying what was
///   narrowing the list when it loaded;
/// * the address bar, pushed, so that a reload, a bookmark or the back button keep the filter
///   somebody set. It used to lose all of it, silently, and land back on the whole corpus.
///
/// The two forms that *act* on the filter are no longer in this list, and deliberately: they read
/// the bar's own fields at the moment of the click. See [`FilterQuery::from_body`].
pub async fn song_rows(
    AxumState(state): AxumState<State>,
    Query(query): Query<FilterQuery>,
) -> Response {
    let rows = match rows_for(&state, &query).await {
        Ok(rows) => rows,
        Err(error) => return failure(error, state.locale()),
    };
    // …and a third thing, of the same kind as the second: the nav, which is seven bare hrefs and
    // cannot see the pushed URL, because that only ever existed in the browser. The page travels
    // with the filter, so a page turn is written down as well and coming back is coming back to the
    // page somebody was on. `total` is not: it is a count this render happens to be carrying rather
    // than part of what was asked for.
    state.remember_songs_filter(query.rebuild(rows.offset, "", None));
    // The rows are the answer and they are in hand; a favorite chip that cannot be named is not
    // worth losing them over. It reads `in 7` for one render and comes back right on the next.
    let favorites: Vec<crate::model::FavoriteNode> =
        state.reading(|db| db.favorites()).await.unwrap_or_default();
    crate::views::rows_with_chips(
        &rows,
        &query.chips(&favorites, true, state.locale()),
        &format!(
            "/songs?{}",
            query.rebuild(query.offset.unwrap_or(0), "", None)
        ),
        state.locale(),
    )
}

async fn rows_for(state: &State, query: &FilterQuery) -> Result<SongRows, DbError> {
    let filter = query.to_filter();
    let counting = filter.clone();
    // Carried by the paging links, absent on a fresh load or a filter change. See `FilterQuery::total`
    // for why turning a page is entitled to reuse it and changing a filter is not.
    let carried = query.total;
    // The last played song comes along with the rows, so the highlight is there on a fresh page and
    // survives a filter change or a page turn rather than living only in the response to a click.
    let ((songs, has_more), total, last_played, present) = state
        .reading(move |db| {
            let page = db.songs_page(&filter)?;
            let total = match carried {
                Some(total) => total,
                None => db.song_count(&counting)?,
            };
            Ok((
                page,
                total,
                db.setting(LAST_PLAYED_SETTING)?,
                db.languages_present()?,
            ))
        })
        .await?;

    let offset = query.offset.unwrap_or(0);
    // **A page above the end of the corpus is answered with the last page**, rather than with an
    // empty list under a working *previous* button. A page number travels with the filter, so it
    // outlives the run that set it and comes back to a corpus a scan may have taken rows out of —
    // the staleness a favorite id is already refused for, one field over. A bookmark and a
    // hand-typed offset land here too.
    //
    // Costs a second query, and only in the case that would otherwise draw nothing. Backwards only:
    // a carried `total` that a scan has left behind can put the last page *after* the empty one, and
    // one empty page is enough.
    let last_page = total.saturating_sub(1) / PAGE_SIZE * PAGE_SIZE;
    let (songs, has_more, offset) = if songs.is_empty() && total > 0 && last_page < offset {
        let mut filter = query.to_filter();
        filter.offset = last_page;
        let (songs, has_more) = state.reading(move |db| db.songs_page(&filter)).await?;
        (songs, has_more, last_page)
    } else {
        (songs, has_more, offset)
    };
    // Never let the label contradict the rows under it. A carried total can lag a scan that is still
    // writing, and `page 3 of 2` is the kind of nonsense that reads as the page being broken
    // rather than as the number being a moment old. The label and the pager both count pages from this, so
    // raising it to what is demonstrably on screen is enough.
    let total = total.max(offset + songs.len() as u32);
    let links = page_links(offset, total, PAGE_SIZE, has_more, |page| {
        query.with_offset(page * PAGE_SIZE, total)
    });

    // Here rather than in the query, because where a song sits in this run's quality hint is not a
    // fact about the song. Doing it once for both `/songs` and `/songs/rows` is what makes a badge
    // survive a page turn and a filter change: the row draws its number wherever that song appears.
    let mut songs = songs;
    state.mark_hints(&mut songs);
    // A page of rows never draws the chooser; only the single-row fragment does.
    state.say_rows(&mut songs, false);

    let mut rows = SongRows {
        songs,
        total,
        offset,
        previous: links.previous,
        next: links.next,
        first_page: links.first,
        last_page: links.last,
        pages: links.pages,
        ratings: crate::views::rating_choices(),
        // No `current`: one list serves a page of rows, and which option each has selected is decided
        // in the template against the row's own `language_text()` — the way `ratings` already works.
        languages: crate::views::Choice::languages_in(&present, None),
        all_languages: Vec::new(),
        choosing_language: false,
        editing: false,
        picking: false,
        favorites: Vec::new(),
        last_played,
        scanning: state.scan_running(),
        show_filename: query.filenames(),
        // After the rest, because it counts what is in there and reads whether a scan is running.
        range: String::new(),
    };
    rows.say_range(state.locale(), PAGE_SIZE);
    Ok(rows)
}

// -- searching the words --------------------------------------------------------------------

/// How many hits a page of lyric search holds.
///
/// Smaller than [`PAGE_SIZE`]: every hit is two rows rather than one, and a screen of passages is
/// read rather than scanned, so a browse page of them is a wall. Twenty-five is about a screenful.
const LYRIC_PAGE_SIZE: u32 = 25;

/// The lyric search as it arrives from the query string.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct LyricQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    offset: Option<u32>,
}

impl LyricQuery {
    fn to_search(&self) -> LyricSearch {
        LyricSearch {
            query: self.q.clone(),
            limit: LYRIC_PAGE_SIZE,
            offset: self.offset.unwrap_or(0),
        }
    }

    /// The same search at a different offset, for the paging buttons.
    ///
    /// One field to carry, so unlike [`FilterQuery::rebuild`] there is nothing here to forget.
    fn with_offset(&self, offset: u32) -> String {
        let mut parts = vec![format!("q={}", crate::model::encode(&self.q))];
        if offset > 0 {
            parts.push(format!("offset={offset}"));
        }
        parts.join("&")
    }
}

/// `GET /lyrics`
pub async fn lyric_search(
    AxumState(state): AxumState<State>,
    Query(query): Query<LyricQuery>,
) -> Response {
    let chrome = match chrome(&state, "lyrics").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    let hits = match hits_for(&state, &query).await {
        Ok(hits) => hits,
        Err(error) => return failure(error, state.locale()),
    };
    let favorites = match state.reading(|db| db.favorites()).await {
        Ok(favorites) => favorites,
        Err(error) => return failure(error, state.locale()),
    };

    page(
        &LyricSearchPage {
            chrome,
            hits,
            favorites,
            q: query.q.clone(),
        },
        state.locale(),
    )
}

/// `GET /lyrics/hits`
pub async fn lyric_hits(
    AxumState(state): AxumState<State>,
    Query(query): Query<LyricQuery>,
) -> Response {
    match hits_for(&state, &query).await {
        Ok(hits) => page(&hits, state.locale()),
        Err(error) => failure(error, state.locale()),
    }
}

async fn hits_for(state: &State, query: &LyricQuery) -> Result<LyricHits, DbError> {
    let search = query.to_search();
    let counting = search.clone();
    let searched = !query.q.trim().is_empty();
    let (hits, total, indexed, last_played, present) = state
        .reading(move |db| {
            Ok((
                db.lyric_search(&search)?,
                db.lyric_search_count(&counting)?,
                // Only asked when the answer will be shown, which is when a search came back with
                // nothing. On a corpus that does have lyrics this is a question with an obvious
                // answer, and asking it on every keystroke would be work done to say so.
                db.lyrics_indexed()?,
                db.setting(LAST_PLAYED_SETTING)?,
                db.languages_present()?,
            ))
        })
        .await?;

    // A hit *is* a browse row, drawn from the same template, so its tooltips are worded the same
    // way a page of rows is.
    let mut hits = hits;
    for hit in &mut hits {
        hit.song.say(state.locale(), false);
    }

    let offset = query.offset.unwrap_or(0);
    // The browse list's pager, over this list's page size and this list's route. A lyric search is
    // ordered by how well the words matched, so page eleven is where a half-remembered line stops
    // being the best answer and starts being a coincidence — which is a place somebody goes back to,
    // and a strip of *next* buttons is eleven presses to reach it.
    //
    // No `scanning` tag comes with it, and nothing here clamps a stale total: `lyric_search_count`
    // is an exact `COUNT(*)` over the index at the moment it is asked, where the browse list's is a
    // reading taken while a scan may be writing rows underneath it.
    let links = page_links(
        offset,
        total,
        LYRIC_PAGE_SIZE,
        offset + LYRIC_PAGE_SIZE < total,
        |page| query.with_offset(page * LYRIC_PAGE_SIZE),
    );
    let mut page = LyricHits {
        previous: links.previous,
        next: links.next,
        first_page: links.first,
        last_page: links.last,
        pages: links.pages,
        hits,
        searched,
        indexed,
        total,
        offset,
        ratings: crate::views::rating_choices(),
        // The same list every browse row gets: a hit *is* a browse row, drawn from the same template,
        // so its language select has to offer the same options or the page would quietly disagree
        // with itself about what a corpus holds.
        languages: crate::views::Choice::languages_in(&present, None),
        all_languages: Vec::new(),
        choosing_language: false,
        editing: false,
        picking: false,
        favorites: Vec::new(),
        last_played,
        // After the rest, because it counts what is in there.
        range: String::new(),
    };
    page.say_range(state.locale(), LYRIC_PAGE_SIZE);
    Ok(page)
}

// -- searching by a name like this one ------------------------------------------------------

/// The similar-names search as it arrives from the query string.
///
/// A title and an artist rather than a song id, so the two boxes can be edited: a name garbled past
/// matching is loosened by hand. `from` is the song the search started from, which heads the list
/// whatever its likeness.
///
/// The five narrowing fields carry the Songs bar's names and spellings, so one parser reads both.
/// Each is empty for *any*, and an empty `versions` is one row per recording. None of them in the
/// address at all means *as last set*, which is what a ≈ link sends; see [`SimilarQuery::narrowed`].
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SimilarQuery {
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    from: String,
    /// Which band of the automatic suitability: `8-10` · `5-7` · `0-4`.
    suitability: Option<String>,
    /// `midi`, `video` or `cdg`.
    kind: Option<String>,
    /// Lyric granularity: `syllablelevel` · `linelevel` · `none`.
    granularity: Option<String>,
    /// How many copies on disk: `1` · `2-10` · `10+`.
    copies: Option<String>,
    /// `all` for every version of a recording. A checkbox, so the bar sends nothing when it is clear.
    versions: Option<String>,
}

impl SimilarQuery {
    /// The search as the similar-names page's bar posts it beside the ticks.
    ///
    /// Read with [`Fields`] because the ticks repeat `song_id`, and first value wins because the bar
    /// is included ahead of `#hits`: a row open for renaming has a `title` box of its own. The bar's
    /// selects always send their fields, so the narrowing is the bar speaking.
    fn from_fields(fields: &Fields) -> Self {
        let named = |key: &str| Some(fields.text(key).to_owned());
        Self {
            title: fields.text("title").to_owned(),
            artist: fields.text("artist").to_owned(),
            from: fields.text("from").to_owned(),
            suitability: named("suitability"),
            kind: named("kind"),
            granularity: named("granularity"),
            copies: named("copies"),
            versions: fields
                .has("versions")
                .then(|| fields.text("versions").to_owned()),
        }
    }

    /// The query with its narrowing settled against what this run last used.
    ///
    /// A query naming any of the five is the bar speaking, so it is used as sent and remembered,
    /// empty fields included. A query naming none is a ≈ link, which carries only a name, so it
    /// takes the remembered five. The bar's selects always send their fields, which is what lets a
    /// cleared checkbox, sending nothing, still count as the bar speaking.
    fn narrowed(mut self, state: &State) -> Self {
        let given = [
            &self.suitability,
            &self.kind,
            &self.granularity,
            &self.copies,
            &self.versions,
        ]
        .iter()
        .any(|field| field.is_some());
        if given {
            state.remember_similar_narrowing(SimilarNarrowing {
                suitability: self.suitability().to_owned(),
                kind: self.kind().to_owned(),
                granularity: self.granularity().to_owned(),
                copies: self.copies().to_owned(),
                versions: self.versions().as_str().to_owned(),
            });
        } else {
            let remembered = state.similar_narrowing();
            self.suitability = Some(remembered.suitability);
            self.kind = Some(remembered.kind);
            self.granularity = Some(remembered.granularity);
            self.copies = Some(remembered.copies);
            self.versions = Some(remembered.versions);
        }
        self
    }

    fn suitability(&self) -> &str {
        self.suitability.as_deref().unwrap_or("")
    }

    fn kind(&self) -> &str {
        self.kind.as_deref().unwrap_or("")
    }

    fn granularity(&self) -> &str {
        self.granularity.as_deref().unwrap_or("")
    }

    fn copies(&self) -> &str {
        self.copies.as_deref().unwrap_or("")
    }

    fn versions(&self) -> VersionsFilter {
        VersionsFilter::parse(self.versions.as_deref().unwrap_or(""))
    }

    /// What narrows the candidates.
    ///
    /// One row per recording unless every version is asked for, as on the Songs page: a hidden
    /// version's primary stands for it, and the primary's versions count leads to it.
    fn to_filter(&self) -> Filter {
        Filter {
            suitability: SuitabilityFilter::parse(self.suitability()),
            kind: SongKind::filter(self.kind()),
            granularity: (!self.granularity().is_empty()).then(|| self.granularity().to_owned()),
            copies: CopiesFilter::parse(self.copies()),
            versions: self.versions(),
            ..Filter::default()
        }
    }

    /// The bar as the page should draw it, each value through its enum and back, so a nonsense
    /// value in a hand-typed URL shows as *any* rather than leaving a select with nothing chosen.
    fn to_form(&self) -> FilterForm {
        FilterForm {
            suitability: SuitabilityFilter::parse(self.suitability())
                .as_str()
                .to_owned(),
            kind: SongKind::filter(self.kind())
                .map_or("", SongKind::as_str)
                .to_owned(),
            granularity: self.granularity().to_owned(),
            copies: CopiesFilter::parse(self.copies()).as_str().to_owned(),
            versions: self.versions().as_str().to_owned(),
            ..FilterForm::default()
        }
    }
}

/// `GET /similar`
pub async fn similar(
    AxumState(state): AxumState<State>,
    Query(query): Query<SimilarQuery>,
) -> Response {
    let query = query.narrowed(&state);
    let chrome = match chrome(&state, "songs").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    let hits = match similar_for(&state, &query).await {
        Ok(hits) => hits,
        Err(error) => return failure(error, state.locale()),
    };
    let favorites = match state.reading(|db| db.favorites()).await {
        Ok(favorites) => favorites,
        Err(error) => return failure(error, state.locale()),
    };

    page(
        &SimilarPage {
            chrome,
            hits,
            favorites,
            title: query.title.clone(),
            artist: query.artist.clone(),
            from: query.from.clone(),
            query: query.to_form(),
        },
        state.locale(),
    )
}

/// `GET /similar/hits`
pub async fn similar_hits(
    AxumState(state): AxumState<State>,
    Query(query): Query<SimilarQuery>,
) -> Response {
    let query = query.narrowed(&state);
    match similar_for(&state, &query).await {
        Ok(hits) => page(&hits, state.locale()),
        Err(error) => failure(error, state.locale()),
    }
}

async fn similar_for(state: &State, query: &SimilarQuery) -> Result<SimilarHits, DbError> {
    let searched = crate::similar::match_query(&query.title, &query.artist).is_some();
    let asked = query.clone();
    let filter = query.to_filter();
    let (hits, last_played, present) = state
        .reading(move |db| {
            Ok((
                db.similar_names(&asked.title, &asked.artist, &asked.from, &filter)?,
                db.setting(LAST_PLAYED_SETTING)?,
                db.languages_present()?,
            ))
        })
        .await?;

    // A match *is* a browse row, drawn from the same template, so its tooltips are worded the same
    // way a page of rows is.
    let mut hits = hits;
    // A match carries its place in this run's quality hint, so a narrowed list keeps its numbers.
    state.mark_hints(&mut hits);
    for song in &mut hits {
        song.say(state.locale(), false);
    }

    // One page and no pager: past a hundred, a list ordered by likeness is coincidence, and nothing
    // could count the rest without scoring every candidate the index holds.
    Ok(SimilarHits {
        hits,
        searched,
        ratings: crate::views::rating_choices(),
        languages: crate::views::Choice::languages_in(&present, None),
        all_languages: Vec::new(),
        choosing_language: false,
        editing: false,
        picking: false,
        favorites: Vec::new(),
        last_played,
    })
}

/// The paging links: the two steps, the two ends, and a window of numbered pages.
///
/// Every string is empty when the control it draws would not move, which is what the template tests
/// for rather than working out again.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PageLinks {
    /// One page back.
    pub previous: String,
    /// One page on.
    pub next: String,
    /// The first page, empty when the window already reaches it.
    pub first: String,
    /// The last page, empty when the window already reaches it.
    pub last: String,
    /// A window of numbered pages around the one being shown, the current one included.
    pub pages: Vec<PageNumber>,
}

/// One numbered page in the pager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageNumber {
    /// What it says: the page number, counting from one.
    pub number: u32,
    /// The query string that goes to it. Empty for the page being shown, which is drawn as a label.
    pub query: String,
    /// Whether this is the page being shown.
    pub current: bool,
}

/// How many pages either side of the current one the window reaches.
///
/// One number for both lists. What a person is doing on either is placing themselves in a list too
/// long to hold in mind, and how far a window has to reach for that is a fact about reading a strip
/// of numbers rather than about what the rows are.
const PAGE_WINDOW: u32 = 5;

/// Works out where the paging controls go.
///
/// **Numbered pages, because the total is exact.** `Db::song_count` is a plain `COUNT(*)` over the
/// filter, so *page 37 of 4,805* is a fact and can be turned into buttons; a `« 5` / `5 »` pair was
/// what a pager offers when it does not know how many pages there are. Five either side, plus the
/// two ends, which is enough to place yourself without a row of four thousand numbers.
///
/// `has_more` comes from the query having fetched one row past the page ([`Db::songs_page`](crate::db::Db::songs_page)) and is
/// what decides whether *next* exists. `offset + PAGE_SIZE < total` is the same answer arrived at
/// by counting the whole corpus, and is what makes turning a page cost a `COUNT(*)` over every
/// row. The numbers below do read `total`, which is the one thing they cannot be drawn
/// without; it is already computed for the label beside them and is carried between page turns.
///
/// **A `total` that lags is handled rather than trusted.** A scan writing rows underneath makes the
/// count a reading rather than a fact — the pager says so with a `~` and a tag — so the last page is
/// clamped to *at least the page being shown*, or a stale count would draw a *last* button landing
/// behind the page it was pressed from.
///
/// **Both lists page through here**, which is what `page_size` and `at` are for: the browse list and
/// the lyric hits differ in how many rows a page holds and in which route a button aims at, and in
/// nothing else. The arithmetic that decides where a window sits and which of the four ends is live
/// is the same question asked of two lists, and one copy of it is what stops the two answering it
/// differently.
fn page_links(
    offset: u32,
    total: u32,
    page_size: u32,
    has_more: bool,
    at: impl Fn(u32) -> String,
) -> PageLinks {
    let here = offset / page_size;
    // At least `here`, for the stale-total reason above; and at least `here + 1` while there is
    // demonstrably another page, so the window never ends on the page somebody is looking at while
    // *next* is still live.
    let last = total
        .saturating_sub(1)
        .checked_div(page_size)
        .unwrap_or(0)
        .max(here)
        .max(if has_more { here + 1 } else { here });
    let from = here.saturating_sub(PAGE_WINDOW);
    let to = (here + PAGE_WINDOW).min(last);

    PageLinks {
        previous: match here > 0 {
            true => at(here - 1),
            false => String::new(),
        },
        next: match has_more {
            true => at(here + 1),
            false => String::new(),
        },
        // The ends are drawn only where the window does not already reach them, so a short list has
        // no *first* and *last* sitting beside `1` and `3`.
        first: match from > 0 {
            true => at(0),
            false => String::new(),
        },
        last: match to < last {
            true => at(last),
            false => String::new(),
        },
        pages: (from..=to)
            .map(|page| PageNumber {
                number: page + 1,
                // The page being shown is a label rather than a button: a control that reloads what
                // is already on screen is one more thing to press by mistake.
                query: if page == here {
                    String::new()
                } else {
                    at(page)
                },
                current: page == here,
            })
            .collect(),
    }
}

/// `GET /songs/{id}/row`
///
/// One row of the browse table, re-rendered. Every change made from the list answers with this, so a
/// row that has just been scored or renamed shows what the database now holds rather than what the
/// browser guessed. `?editing=1` returns the same row with its title and artist as inputs.
pub async fn song_row(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<RowQuery>,
) -> Response {
    if query.picking.is_some() {
        return picking_response(&state, &id).await;
    }
    if query.language.is_some() {
        return language_response(&state, &id).await;
    }
    row_response(&state, &id, query.editing.is_some()).await
}

/// Which state a row should come back in: plain, editable, asking for a favorite, or asking for a
/// language from the whole standard.
#[derive(Debug, Default, serde::Deserialize)]
pub struct RowQuery {
    #[serde(default)]
    editing: Option<String>,
    #[serde(default)]
    picking: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

/// Loads one row as the "which favorite?" chooser.
///
/// The favorites and this song's memberships are loaded here, for one row, rather than being carried
/// by every row of the list: a page of rows would each need a query to answer a question only the row
/// somebody clicked is asking.
async fn picking_response(state: &State, id: &str) -> Response {
    let lookup = id.to_owned();
    let loaded = state
        .reading(move |db| {
            Ok((
                db.song_row(&lookup)?,
                db.favorites()?,
                db.favorites_for(&lookup)?,
                db.languages_present()?,
            ))
        })
        .await;
    match loaded {
        Ok((mut song, favorites, member_of, present)) => {
            state.mark_hint(&mut song);
            // The chooser is open, which is what the favorites button offers to close.
            state.say_row(&mut song, true);
            let languages = crate::views::Choice::languages_in(&present, song_language(&song));
            page(
                &SongRowFragment::picking(
                    song,
                    favorites,
                    member_of.into_iter().map(|(id, _)| id).collect(),
                    languages,
                    state.locale(),
                ),
                state.locale(),
            )
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// Loads one row as the full language picker.
///
/// The whole standard, for the one row that asked. Every other row on the page keeps the short list —
/// see `SongRows::languages` for why that is not merely a preference.
async fn language_response(state: &State, id: &str) -> Response {
    let lookup = id.to_owned();
    let loaded = state
        .reading(move |db| Ok((db.song_row(&lookup)?, db.languages_present()?)))
        .await;
    match loaded {
        Ok((mut song, present)) => {
            state.mark_hint(&mut song);
            state.say_row(&mut song, false);
            page(
                &SongRowFragment::choosing_language(song, &present),
                state.locale(),
            )
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// This row's language as a `&str`, for marking the picker's options.
fn song_language(song: &crate::model::SongRow) -> Option<&str> {
    song.language.as_deref()
}

/// Loads one row and renders it, or says why it could not.
async fn row_response(state: &State, id: &str, editing: bool) -> Response {
    let lookup = id.to_owned();
    // The last played song comes with it: a row re-rendered after a score change must not lose the
    // highlight, or scoring a song would look like it had un-played it.
    //
    // So do the corpus's languages, and for the same kind of reason: this row is swapped back into a
    // block of the others, and one drawn from a different list of options would be a row that
    // silently stopped offering what its neighbors offer.
    match state
        .reading(move |db| {
            Ok((
                db.song_row(&lookup)?,
                db.setting(LAST_PLAYED_SETTING)?,
                db.languages_present()?,
            ))
        })
        .await
    {
        Ok((mut song, last_played, present)) => {
            // The number comes back with the row, for the reason the highlight does: a row swapped
            // back after a score change must not lose a badge, or the edit would look as if it had
            // taken the song out of the hint.
            state.mark_hint(&mut song);
            state.say_row(&mut song, false);
            let languages = crate::views::Choice::languages_in(&present, song_language(&song));
            page(
                &SongRowFragment::new(song, editing, languages).with_last_played(last_played),
                state.locale(),
            )
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `GET /songs/{id}`
pub async fn song(AxumState(state): AxumState<State>, UrlPath(id): UrlPath<String>) -> Response {
    let chrome = match chrome(&state, "songs").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    let lookup = id.clone();
    let loaded = state
        .reading(move |db| {
            Ok((
                db.song(&lookup)?,
                db.favorites()?,
                // The ones no favorite sources; the Songs page's own closure says why.
                db.packages_taking_songs()?,
                db.package_volumes_all()?,
                db.languages_present()?,
                db.tags_present()?,
                db.tags_of(&lookup)?,
                db.versions_of(&lookup)?,
                // Under the root rather than relative to it, because what reads it opens it. A song
                // whose every copy has gone from disk has none, and the corrections control then
                // offers what is in force and nothing new.
                db.best_file(&lookup).map(|(_, path)| path).ok(),
            ))
        })
        .await;
    let (song, favorites, packages, volumes, present, corpus_tags, tags, versions, readable) =
        match loaded {
            Ok(loaded) => loaded,
            Err(error) => return failure(error, state.locale()),
        };
    let settings = state.settings();
    let known_tags = settings.offerable_tags(&corpus_tags);
    let suggested_tags = settings.hints(&corpus_tags);

    let path = song
        .files
        .first()
        .map(|file| file.path.clone())
        .unwrap_or_default();
    let youtube = crate::model::youtube_query(
        &song.effective_title(),
        song.effective_artist().as_deref(),
        &path,
    )
    .map(|query| {
        format!(
            "https://www.youtube.com/results?search_query={}",
            crate::model::encode(&query)
        )
    })
    .unwrap_or_default();

    const SCORES: [&str; 11] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"];
    let ratings = Choice::list(
        SCORES,
        song.user_score.map(|score| score.to_string()).as_deref(),
    );
    let encodings = Choice::list(ENCODINGS.iter().copied(), song.detected_encoding());
    // Marked against what a person chose, and deliberately not against `det_language_tag`: the
    // hint beside the box is where a detected value is reported, so that saving the form cannot
    // quietly promote it to a hand-set one.
    let languages = Choice::languages(song.language.as_deref());
    // The short group. Unmarked on purpose — see the note on `SongPage::corpus_languages`.
    let corpus_languages = Choice::languages_in(&present, None);

    let tag_list = crate::views::TagList {
        song_id: song.id.clone(),
        tags,
    };

    // Only a MIDI song has events to correct. The file is parsed here rather than read from a
    // scanned column so that a detector which learns a new defect reaches a corpus nobody is going
    // to scan again; see the module note.
    let (channels, melody_is_none) = if song.kind.is_midi() {
        let analysis = crate::fixes::analyze(readable.as_deref());
        let in_force = crate::fixes::stored(song.fixes.as_deref())
            .unwrap_or_else(|| analysis.detected.clone());
        // The scanned channel rather than a fresh one, so the select names the same channel the
        // badge above it and the package built from this row both do.
        let melody = song.midi.as_ref().and_then(|midi| midi.melody_channel);
        let chosen = crate::fixes::MelodyChoice::parse(song.melody_chosen.as_deref());
        (
            crate::fixes::channel_rows(&analysis, melody, chosen, &in_force),
            crate::fixes::melody_in_force(melody, chosen).is_none(),
        )
    } else {
        (Vec::new(), false)
    };

    page(
        &SongPage {
            said: song_said(&song, versions.len(), state.locale()),
            chrome,
            song,
            favorites,
            packages,
            volumes,
            corpus_languages,
            known_tags,
            suggested_tags,
            tag_list,
            encodings,
            languages,
            ratings,
            melody_is_none,
            channels,
            youtube,
            versions,
        },
        state.locale(),
    )
}

/// The sentences a song's page says that carry a value.
///
/// Gathered in one place, because each reads the detail the page was built from and a caller
/// assembling a dozen of them one at a time is a caller that words eleven.
fn song_said(
    song: &crate::db::SongDetail,
    versions: usize,
    locale: km_locale::Locale,
) -> crate::views::SongSaid {
    let words = crate::words::messages(locale);
    let n = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let mut said = crate::views::SongSaid::default();

    if song.language_is_detected() {
        let name = song.detected_language_name().to_owned();
        said.language_guess = match song.det_language.as_deref() {
            Some(declared) if song.declaration_is_default() => words
                .msg_with(
                    "song-language-declared-default",
                    &[("name", name.as_str().into()), ("code", declared.into())],
                )
                .into_owned(),
            Some(declared) => words
                .msg_with(
                    "song-language-declared",
                    &[("name", name.as_str().into()), ("code", declared.into())],
                )
                .into_owned(),
            None => words
                .msg_with(
                    "song-language-from-encoding",
                    &[("name", name.as_str().into())],
                )
                .into_owned(),
        };
    } else if let Some(declared) = song.det_language.as_deref() {
        said.language_unknown_code = words
            .msg_with("song-language-unknown-code", &[("code", declared.into())])
            .into_owned();
    }

    if let Some(midi) = song.midi.as_ref() {
        said.suitability_parts = words
            .msg_with(
                "song-suitability-parts",
                &[
                    ("lyrics", i64::from(midi.suitability_lyrics).into()),
                    ("sync", i64::from(midi.suitability_sync).into()),
                    ("channels", i64::from(midi.suitability_channels).into()),
                    (
                        "arrangement",
                        i64::from(midi.suitability_arrangement).into(),
                    ),
                ],
            )
            .into_owned();
        said.melody = match midi.melody_channel {
            Some(channel) => match midi.melody_confidence {
                Some(confidence) => words
                    .msg_with(
                        "song-melody-channel-confidence",
                        &[
                            ("channel", i64::from(channel).into()),
                            ("confidence", f64::from(confidence).into()),
                        ],
                    )
                    .into_owned(),
                None => words
                    .msg_with(
                        "song-melody-channel",
                        &[("channel", i64::from(channel).into())],
                    )
                    .into_owned(),
            },
            None => match midi.melody_abstained.as_deref() {
                Some(why) => words
                    .msg_with("song-melody-not-found-why", &[("why", why.into())])
                    .into_owned(),
                None => words.msg("song-melody-not-found").into_owned(),
            },
        };
        said.content = words
            .msg_with(
                "song-content",
                &[
                    ("notes", n(u64::from(midi.note_count)).into()),
                    ("channels", i64::from(midi.channel_count).into()),
                    ("lines", n(u64::from(midi.line_count)).into()),
                    ("syllables", n(u64::from(midi.syllable_count)).into()),
                ],
            )
            .into_owned();
    }

    if let Some(cdg) = song.cdg.as_ref() {
        said.cdg_length = words
            .msg_with(
                "song-cdg-length",
                &[("words", cdg.graphics_summary().as_str().into())],
            )
            .into_owned();
        said.cdg_audio = words
            .msg_with(
                "song-cdg-audio",
                &[("channels", i64::from(cdg.channels).into())],
            )
            .into_owned();
        said.cdg_graphics = words
            .msg_with(
                "song-cdg-graphics",
                &[
                    ("tiles", n(u64::from(cdg.tiles_written)).into()),
                    ("packets", n(u64::from(cdg.packets)).into()),
                ],
            )
            .into_owned();
        said.cdg_unknown = (cdg.unknown_instructions > 0).then(|| {
            words
                .msg_with(
                    "song-cdg-unknown",
                    &[("count", n(u64::from(cdg.unknown_instructions)).into())],
                )
                .into_owned()
        });
    }

    said.files_heading = words
        .msg_with(
            "song-files-heading",
            &[(
                "count",
                i64::try_from(song.files.len()).unwrap_or(i64::MAX).into(),
            )],
        )
        .into_owned();
    said.versions_heading = words
        .msg_with(
            "song-versions-heading",
            &[("count", i64::try_from(versions).unwrap_or(i64::MAX).into())],
        )
        .into_owned();
    said
}

/// What encoding to decode lyrics with, when the person picks one.
#[derive(Debug, Default, serde::Deserialize)]
pub struct EncodingQuery {
    #[serde(default)]
    encoding: String,
}

/// `GET /songs/{id}/lyrics`
pub async fn lyrics(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<EncodingQuery>,
) -> Response {
    let chosen = (!query.encoding.is_empty()).then(|| query.encoding.clone());
    let loaded = load_song(&state, &id, chosen.clone()).await;
    let (song, pinned) = match loaded {
        Ok(pair) => pair,
        Err(error) => return MessageFragment::failed(error),
    };

    let lines = song
        .lyrics
        .lines
        .iter()
        .map(|line| {
            let ms = song.tempo_map.tick_to_ms(line.start_tick);
            (
                format!("{}:{:02}", ms / 60_000, (ms / 1000) % 60),
                line.text(),
            )
        })
        .collect::<Vec<_>>();

    page(
        &LyricsFragment {
            encoding: song.decoder.name().to_owned(),
            source: format!("{:?}", song.decoder.source()).to_lowercase(),
            empty: lines.is_empty(),
            lines,
            song_id: id,
            // Offering to pin what is already pinned is a button that does nothing.
            pinnable: pinned.as_deref() != Some(song.decoder.name()),
        },
        state.locale(),
    )
}

/// `GET /songs/{id}/raw`
pub async fn raw(AxumState(state): AxumState<State>, UrlPath(id): UrlPath<String>) -> Response {
    let bytes = match song_bytes(&state, &id).await {
        Ok((_, bytes)) => bytes,
        Err(error) => return MessageFragment::failed(error),
    };
    match km_song::text_events(&bytes, None) {
        Ok(events) => page(
            &RawFragment {
                events: events
                    .into_iter()
                    .map(|event| (event.track, event.tick, event.kind.to_owned(), event.text))
                    .collect(),
            },
            state.locale(),
        ),
        Err(error) => MessageFragment::failed(format!("could not read the file: {error}")),
    }
}

/// `GET /songs/{id}/download`
///
/// The counterpart to opening a file in the OS: that only works when the browser and this tool are on
/// the same machine, and this works either way.
pub async fn download(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    let (path, bytes) = match song_bytes(&state, &id).await {
        Ok(pair) => pair,
        Err(error) => return (StatusCode::NOT_FOUND, error).into_response(),
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{id}.kar"));

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "audio/midi".parse().expect("a valid header value"),
    );
    // The filename is quoted and its quotes stripped: a corpus filename can contain almost anything,
    // and a stray quote here would let it break out of the header value.
    let safe = name.replace(['"', '\\', '\r', '\n'], "_");
    if let Ok(value) = format!("attachment; filename=\"{safe}\"").parse() {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    (headers, bytes).into_response()
}

// -- editing --------------------------------------------------------------------------------

/// `POST /songs/{id}/edit`
pub async fn edit_song(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    // Every box is present in the form, so every field is written. An empty box means "nobody has
    // said", which is exactly `NULL` — and is what makes clearing a wrong artist possible.
    let edit = SongEdit {
        title: Some(form.one("title").map(ToOwned::to_owned)),
        artist: Some(form.one("artist").map(ToOwned::to_owned)),
        language: Some(form.one("language").map(ToOwned::to_owned)),
        lyric_encoding: None,
        default_transpose: Some(form.parsed::<i8>("transpose").map(|v| v.clamp(-12, 12))),
        notes: Some(form.one("notes").map(ToOwned::to_owned)),
        // Written by `save_corrections` and by nothing else. `SongEdit` leaves a `None` field alone,
        // so the two forms on this page cannot write over each other.
        fixes: None,
        melody_chosen: None,
    };
    match state.blocking(move |db| db.edit_song(&id, &edit)).await {
        Ok(()) => MessageFragment::ok(crate::words::messages(state.locale()).msg("said-saved")),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/corrections`
///
/// **A route of its own, because the Advanced table may not post into the details form.** That form
/// writes the title, the artist and the language from what came back, so a corrections save through
/// it would clear all three. The table sends the whole list, so what is written is complete.
pub async fn save_corrections(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let lookup = id.clone();
    let loaded = state
        .blocking(move |db| {
            let song = db.song(&lookup)?;
            Ok((
                db.best_file(&lookup).map(|(_, path)| path).ok(),
                song.fixes,
                song.midi.as_ref().and_then(|midi| midi.melody_channel),
            ))
        })
        .await;
    let (path, held, detected_melody) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    let analysis = crate::fixes::analyze(path.as_deref());
    let in_force =
        crate::fixes::stored(held.as_deref()).unwrap_or_else(|| analysis.detected.clone());
    let edit = SongEdit {
        fixes: crate::fixes::selection(&form.all("fix"), &analysis.detected, &in_force),
        // **Only the form that carries the radios writes this.** The Details tab's select posts
        // here too and sends no `melody` field, which must leave the column alone rather than clear
        // it — so an absent field is `None` and not `Some(None)`, the same rule `SongEdit` follows
        // everywhere. An unticked radio group cannot arise: one radio is always checked.
        melody_chosen: form
            .one("melody")
            .map(|posted| crate::fixes::melody_selection(Some(posted), detected_melody)),
        ..SongEdit::default()
    };
    match state.blocking(move |db| db.edit_song(&id, &edit)).await {
        // A toast, where a refusal keeps the slot: the Save button sits under a table as tall as
        // the file has channels, and a saved list is news about something that is over.
        Ok(()) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale()).msg("said-corrections-saved"),
        )),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// Whether the bulk language set has been confirmed yet.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ConfirmQuery {
    #[serde(default)]
    confirm: Option<String>,
}

/// Which volume of a package a page or a write is about.
///
/// **In the query string on every route that acts on one file**, which is what lets the package page
/// be a link per volume: the version, the first number, the members, a build and an install all
/// follow it. Absent means the first volume, which is the only one most packages have.
#[derive(Debug, Default, serde::Deserialize)]
pub struct VolumeQuery {
    #[serde(default)]
    volume: Option<u32>,
}

impl VolumeQuery {
    /// The volume asked for, from 1.
    fn number(&self) -> u32 {
        self.volume.unwrap_or(1).max(1)
    }
}

/// What a bulk action was asked to do, before it knows which action it is.
///
/// **The invariant this exists to hold in one place: the write is over exactly the set that was
/// counted.** Every bulk action here is two passes — count and show, then write — and getting the
/// two to describe the same songs is the whole of its correctness. Three handlers restated that by
/// hand in about twenty-five identical lines each, and getting it wrong writes to the wrong
/// hundreds of thousands of songs.
///
/// The restatement was not academic. Two of those three carry the *same warning comment*, reworded,
/// about the filter bar riding in the same body: it has `language` and `tags` fields of its own, so
/// an action reading `language` rather than `set_language` would set the language of a set chosen by
/// the language it was setting. That trap is a property of the shared body, and it now has one home.
///
/// The parts that genuinely differ per action — `set_language`, `set_tag`, a package name — are read
/// from [`BulkAction::form`] by the handler that knows about them.
pub struct BulkAction {
    /// The filter to act on.
    ///
    /// On the confirmed pass this is the **counted** query, taken off the URL the confirmation
    /// wrote; on the first pass it is the bar as it stands. That swap is the invariant.
    pub query: FilterQuery,
    /// Whether the write covers the whole filter rather than the ticked rows.
    ///
    /// **Ticked is the default**, because the smaller act should be: acting on a whole filter is
    /// right for a set somebody has already described, and while it was the *only* thing these
    /// controls could do, changing three songs meant describing them in the bar first.
    pub whole_filter: bool,
    /// The ticked song ids — from the rows on the first pass, and from the confirmation's own hidden
    /// fields on the second, which is how the ticked half keeps the same guarantee the frozen query
    /// string gives the filter-wide half.
    pub ticked: Vec<String>,
    /// Whether this is the confirmed pass.
    pub confirmed: bool,
    /// The rest of the submitted form, for whatever this particular action needs from it.
    pub form: Fields,
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for BulkAction {
    /// The failure fragment itself, because that is what these handlers answer with anyway.
    type Rejection = Response;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        use axum::extract::FromRequestParts as _;

        let (mut parts, body) = request.into_parts();
        let counted = Query::<FilterQuery>::from_request_parts(&mut parts, state)
            .await
            .map(|Query(query)| query)
            .unwrap_or_default();
        let confirm = Query::<ConfirmQuery>::from_request_parts(&mut parts, state)
            .await
            .map(|Query(confirm)| confirm)
            .unwrap_or_default();
        let body = String::from_request(axum::extract::Request::from_parts(parts, body), state)
            .await
            .map_err(|error| MessageFragment::failed(error.body_text()))?;

        let form = Fields::parse(&body);
        let confirmed = confirm.confirm.is_some();
        // Confirmed: the counted set, off the URL the confirmation wrote. Not confirmed: the bar,
        // live. This is the line the three handlers each had a copy of.
        let query = if confirmed {
            counted
        } else {
            FilterQuery::from_body(&body).map_err(MessageFragment::failed)?
        };
        let ticked = form
            .all("song_id")
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        Ok(Self {
            query,
            whole_filter: form.one("scope") == Some("matching"),
            ticked,
            confirmed,
            form,
        })
    }
}

/// `POST /songs/language-bulk`
///
/// Sets the language of every song the *current filter* matches — not of the ticked rows, which is
/// what the selection form below does. The filter is read from the bar's own fields, so
/// `Filter::to_sql` produces the identical `WHERE` and the action cannot act on a different set from
/// the one on screen.
///
/// Two steps on purpose. Without `confirm`, it answers with the count and the filters that produced
/// it; with it, it writes. This is the one control here that can change hundreds of thousands of
/// rows, and the filter that decides which is fourteen controls further up the page. The two steps
/// read the filter from two different places, for the reasons set out on [`package_from_filter`].
pub async fn bulk_language(AxumState(state): AxumState<State>, action: BulkAction) -> Response {
    let BulkAction {
        query,
        whole_filter,
        ticked,
        confirmed,
        form,
    } = action;
    // `set_language`, not `language`: the filter bar rides in the same body and has a `language`
    // select of its own, and two spellings of one key is how this action would come to set the
    // language of a set chosen by the language it was being set to. See `FilterQuery::from_body`.
    // An empty choice is a deliberate clear, exactly as it is on the song page.
    let language = match form.one("set_language") {
        Some(value) => match km_kmpkg::Language::parse(value) {
            Some(language) => Some(language),
            None => return MessageFragment::failed(format!("{value:?} is not a language code")),
        },
        None => None,
    };

    // Narrows the *write* and not the list, which is the whole of why it is here rather than left to
    // the bar's own `language=unset`: that one would take the rows being looked at off the page.
    // It replaces the bar's language rather than intersecting with it — two contradictory language
    // constraints would count zero, which the confirmation then says out loud.
    let only_unset = form.has("only_unset");
    let mut filter = query.to_filter();
    if only_unset {
        filter.language = crate::db::LanguageFilter::Unset;
    }

    if !confirmed {
        // The favorites come along because this fragment's whole job is making the filter legible,
        // and without them a favorite chip reads `in 7` — which says less than no chip at all.
        let counting = filter.clone();
        let ids = ticked.clone();
        let counted = state
            .blocking(move |db| {
                let count = if whole_filter {
                    db.count_matching(&counting)?
                } else if ids.is_empty() {
                    0
                } else {
                    // Counted through the same narrowing the write will use, so *only where nobody
                    // has said* is a number somebody can check rather than a promise.
                    db.count_of(&ids, only_unset)?
                };
                Ok((count, db.favorites()?))
            })
            .await;
        let (count, favorites) = match counted {
            Ok(pair) => pair,
            Err(error) => return MessageFragment::failed(error.say(state.locale())),
        };
        if count == 0 {
            return MessageFragment::failed(match (whole_filter, ticked.is_empty()) {
                (false, true) => "Nothing is ticked.".to_owned(),
                (false, false) => "Nothing ticked has an empty language.".to_owned(),
                (true, _) => "Nothing matches that filter.".to_owned(),
            });
        }
        let mut filters: Vec<String> = if whole_filter {
            query
                .active(&favorites, state.locale())
                .into_iter()
                .map(|c| c.label)
                .collect()
        } else {
            vec![format!("{} ticked", ticked.len())]
        };
        if only_unset {
            filters.push("no language yet".to_owned());
        }
        let (subject, confirm) =
            confirm_words(state.locale(), count, "confirm-songs", "confirm-set");
        return page(
            &crate::views::BulkLanguageConfirm {
                subject,
                confirm,
                // Never *the whole corpus* for a ticked write, which is what an empty list says: a
                // ticked set is narrow by construction and the count is the description.
                whole_corpus: whole_filter && filters.is_empty(),
                filters,
                language: language.map_or_else(
                    || "unknown".to_owned(),
                    |language| language.name().to_owned(),
                ),
                // Handed back so the confirm button posts to the same set that was just counted, rather
                // than to whatever the filter bar happens to say by the time it is clicked.
                query: query.rebuild(0, "", None),
                // The ticked ids travel as hidden fields inside the confirmation, which sits inside
                // `#bulk-language` — the element the confirm button already includes.
                songs: if whole_filter { Vec::new() } else { ticked },
            },
            state.locale(),
        );
    }

    // A toast for the result and not for the confirmation above it: the confirmation is a form with
    // a button on it and has to stay where it was put, whereas this is a sentence that is finished
    // being read after eight seconds and would otherwise sit in the bar until the next bulk action.
    match state
        .blocking(move |db| match whole_filter {
            true => db.set_language_for(&filter, language),
            false => db.set_language_of(&ticked, language, only_unset),
        })
        .await
    {
        Ok(0) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale()).msg("said-nothing-change"),
        )),
        Ok(count) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale())
                .msg_with("said-language-set", &[("count", i64::from(count).into())]),
        )),
        Err(error) => crate::views::toast_only(&Toast::bad(error.say(state.locale()))),
    }
}

/// `GET /songs/language-bulk/cancel` — clears the confirmation without doing anything.
pub async fn bulk_language_cancel() -> Response {
    MessageFragment::ok("")
}

/// `POST /songs/tag-bulk`
///
/// Adds or removes one tag over the ticked rows or the whole filter. [`bulk_language`] end to end —
/// the same two steps, the same scope select with the same default, the same frozen query string —
/// with one difference that is the whole design rather than a detail.
///
/// **`add` and `remove`, and there is no `replace`.** The language control *sets*, because a song
/// has one language and writing it is a complete statement. A song has many tags, so a set would
/// silently destroy tagging work done elsewhere — and this is an action a filter can point at
/// hundreds of thousands of rows in one click. Both acts here are additive judgements about a set
/// somebody has already described, which is what makes the filter-wide half defensible at all. See
/// `Assigning tags in bulk` in `docs/decisions/curation.md`.
///
/// **There is no `only_unset` twin either.** *Nobody has said* is a state a language has and a tag
/// does not: every song starts with no tags and most end that way, so the equivalent narrowing is
/// the bar's own tag filter — which, unlike `language=unset`, does not take the rows being looked at
/// off the page.
pub async fn bulk_tag(AxumState(state): AxumState<State>, action: BulkAction) -> Response {
    let BulkAction {
        query,
        whole_filter,
        ticked,
        confirmed,
        form,
    } = action;

    // `set_tag`, not `tags`: the bar rides in the same body and has a `tags` field of its own, and
    // two spellings of one key is how this action would come to tag a set chosen by the tag it was
    // being given. The same rule `set_language` follows above.
    //
    // Read through `Tag`, so a typed word becomes a slug here rather than reaching the database as
    // whatever somebody happened to press. An empty box is a refusal and not a clear — unlike a
    // language, there is nothing a blank could mean.
    let Some(tag) = form.one("set_tag").and_then(km_kmpkg::Tag::parse) else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-type-a-tag"),
        );
    };
    let removing = form.one("tag_action") == Some("remove");
    let filter = query.to_filter();

    if !confirmed {
        let counting = filter.clone();
        let ids = ticked.clone();
        let counted = state
            .blocking(move |db| {
                let count = if whole_filter {
                    db.count_matching(&counting)?
                } else {
                    u32::try_from(ids.len()).unwrap_or(u32::MAX)
                };
                Ok((count, db.favorites()?))
            })
            .await;
        let (count, favorites) = match counted {
            Ok(pair) => pair,
            Err(error) => return MessageFragment::failed(error.say(state.locale())),
        };
        if count == 0 {
            return MessageFragment::failed(match whole_filter {
                false => "Nothing is ticked.".to_owned(),
                true => "Nothing matches that filter.".to_owned(),
            });
        }
        let filters: Vec<String> = if whole_filter {
            query
                .active(&favorites, state.locale())
                .into_iter()
                .map(|chip| chip.label)
                .collect()
        } else {
            vec![format!("{} ticked", ticked.len())]
        };
        let (subject, confirm) = confirm_words(
            state.locale(),
            count,
            "confirm-songs",
            if removing {
                "confirm-untag"
            } else {
                "confirm-tag"
            },
        );
        return page(
            &crate::views::BulkTagConfirm {
                subject,
                confirm,
                whole_corpus: whole_filter && filters.is_empty(),
                filters,
                tag: tag.into_string(),
                removing,
                query: query.rebuild(0, "", None),
                songs: if whole_filter { Vec::new() } else { ticked },
            },
            state.locale(),
        );
    }

    match state
        .blocking(move |db| match (whole_filter, removing) {
            (true, false) => db.add_tag_for(&filter, &tag),
            (true, true) => db.remove_tag_for(&filter, &tag),
            (false, false) => db.add_tag_of(&ticked, &tag),
            (false, true) => db.remove_tag_of(&ticked, &tag),
        })
        .await
    {
        // Zero is ordinary here in a way it is not for a language set: adding a tag to songs that
        // already carry it writes nothing, and that is success rather than a fault.
        Ok(0) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale()).msg("said-nothing-change"),
        )),
        Ok(count) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale()).msg_with(
                if removing {
                    "said-tag-removed"
                } else {
                    "said-tag-set"
                },
                &[("count", i64::from(count).into())],
            ),
        )),
        Err(error) => crate::views::toast_only(&Toast::bad(error.say(state.locale()))),
    }
}

/// `GET /songs/tag-bulk/cancel` — clears the confirmation without doing anything.
pub async fn bulk_tag_cancel() -> Response {
    MessageFragment::ok("")
}

/// Saved filters with a `favorite=` nothing carries any more taken out of each.
///
/// **Scrubbed as they are drawn, and the row is never rewritten.** A saved filter can sit unused for
/// months, so it meets the fault [`crate::server::without_missing_favorite`] exists for: a favorite
/// deleted since is an empty page with no explanation, and once ids start being reused it is a chip
/// naming the wrong favorite confidently. Editing the stored row instead would
/// be the tool quietly changing something somebody typed — and would be the wrong answer anyway the
/// day that favorite comes back from a backup.
///
/// Every other filter in a saved query is text — an artist, a language code, a tag slug — and at
/// worst matches nothing while saying so in its own words.
fn offerable(
    saved: Vec<crate::model::SavedFilter>,
    favorites: &[crate::model::FavoriteNode],
    locale: km_locale::Locale,
) -> Vec<crate::views::SavedFilterRow> {
    saved
        .into_iter()
        .map(|mut filter| {
            filter.query = crate::server::without_missing_favorite(&filter.query, favorites);
            crate::views::SavedFilterRow::new(filter, locale)
        })
        .collect()
}

/// How many a confirmation is about, and what its button says.
///
/// **The count is said twice on every one of these fragments** — once as the subject and once on the
/// button that goes ahead — so both are worded in one place, each a plural over the number beside
/// it.
fn confirm_words(
    locale: km_locale::Locale,
    count: u32,
    subject: &str,
    confirm: &str,
) -> (String, String) {
    let words = crate::words::messages(locale);
    let counted = |key: &str| {
        words
            .msg_with(key, &[("count", i64::from(count).into())])
            .into_owned()
    };
    (counted(subject), counted(confirm))
}

/// The strip as it should look right now, for the out-of-band half of a save or a forget.
async fn saved_strip(state: &State) -> Result<crate::views::SavedFilters, DbError> {
    let (saved, favorites) = state
        .reading(|db| Ok((db.saved_filters()?, db.favorites()?)))
        .await?;
    Ok(crate::views::SavedFilters {
        filters: offerable(saved, &favorites, state.locale()),
        oob: true,
        renaming: false,
    })
}

/// The same query with the page number taken off.
///
/// On the string rather than through a [`FilterQuery`] round trip, for the reason
/// `without_missing_favorite` gives: a parse and a [`FilterQuery::rebuild`] would make this a second
/// place that has to know every key, and one left out is a filter that silently disappears. `total`
/// needs no arm — what is written down is `rebuild(offset, "", None)`, which never carries one.
fn without_page(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| !pair.starts_with("offset="))
        .collect::<Vec<_>>()
        .join("&")
}

/// `POST /songs/saved-filters`
///
/// **The one action on the songs page that does not send the filter bar with it.** Everything else
/// that acts on the filter reads it out of the body, for the reason [`FilterQuery::from_body`] gives
/// at length: an `hx-post` attribute is rendered once and the bar never re-renders the page. That
/// argument does not reach here. Every change to the bar goes through [`song_rows`], which writes
/// the canonical query string into the state before it answers — so the server already holds the
/// exact string the address bar is showing, `offset` clamped and all. Reading the body instead would
/// buy nothing and would walk into the `duplicate_field` hazard a fourth time.
///
/// What that costs is a dependency worth naming: a route that re-renders `#rows` and does not write
/// the filter down leaves this saving a page nobody is on. There are three such routes and a test
/// holds each.
///
/// Two passes when the name is taken, one when it is not. The second pass writes what the
/// confirmation showed rather than the filter as it now stands, because the bar is live while a
/// confirmation sits on screen.
pub async fn save_filter(
    AxumState(state): AxumState<State>,
    Query(confirm): Query<ConfirmQuery>,
    body: String,
) -> Response {
    let fields = Fields::parse(&body);
    let confirmed = confirm.confirm.is_some();
    let Some(name) = fields.one("saved_name").map(ToOwned::to_owned) else {
        return saved_filter_failed(
            crate::words::messages(state.locale())
                .msg("said-name-first")
                .into_owned(),
        );
    };

    // Frozen by the confirmation on the second pass, read live on the first. `one` treats a blank as
    // absent, and a saved whole-corpus filter is exactly that — so the empty string has to come back
    // as itself rather than as "nothing was sent".
    let query = match confirmed {
        true => fields.text("saved_query").to_owned(),
        false => {
            let live = state.songs_filter();
            match fields.has("keep_page") {
                true => live,
                false => without_page(&live),
            }
        }
    };

    if !confirmed {
        let taken = {
            let name = name.clone();
            state.blocking(move |db| db.saved_filter_named(&name)).await
        };
        match taken {
            Ok(Some(existing)) => {
                return page(
                    &crate::views::SavedFilterConfirm {
                        name,
                        query,
                        replacing: existing.query,
                    },
                    state.locale(),
                );
            }
            Ok(None) => {}
            Err(error) => return saved_filter_failed(error.say(state.locale())),
        }
    }

    let now = crate::scan::timestamp();
    let written = {
        let (name, query) = (name.clone(), query.clone());
        state
            .blocking(move |db| db.save_filter(&name, &query, &now))
            .await
    };
    if let Err(error) = written {
        return saved_filter_failed(error.say(state.locale()));
    }
    let strip = match saved_strip(&state).await {
        Ok(strip) => strip,
        // The write happened, so a redraw that fails is its own message: reported as a failed save
        // it would send somebody to press the button a second time.
        Err(error) => {
            let said = crate::words::messages(state.locale()).msg_with(
                "said-filter-saved-not-drawn",
                &[
                    ("name", name.as_str().into()),
                    ("error", error.say(state.locale()).into()),
                ],
            );
            return saved_filter_failed(said.into_owned());
        }
    };
    let said = crate::words::messages(state.locale())
        .msg_with("said-filter-saved", &[("name", name.as_str().into())])
        .into_owned();
    crate::views::with_toast(&strip, &crate::views::Toast::good(said), state.locale())
}

/// `GET /songs/saved-filters/cancel` — clears the confirmation without doing anything.
pub async fn save_filter_cancel() -> Response {
    MessageFragment::ok("")
}

/// A refusal from the saved strip, as a toast with the slot emptied.
///
/// The slot is where a confirmation sits, not where a sentence does — see [`with_saved_strip`] for
/// why nothing here writes a message into it. Emptying it is the point as much as the toast is: a
/// refusal arriving while the replace confirmation is on screen must take that confirmation away,
/// or the button it offers goes on promising a write that was just refused.
fn saved_filter_failed(said: String) -> Response {
    crate::views::toast_only(&crate::views::Toast::bad(said))
}

/// `POST /songs/saved-filters/{id}/update`
///
/// Writes the filter on screen into a name that already exists, and says what it replaced.
///
/// **It does not ask, where saving under a name that is taken does**, and the difference is how the
/// row was arrived at rather than how much is at stake. A save reaches an existing row by colliding
/// with it, so the confirmation's work is to show *which* row that is; this button is drawn on the
/// row it writes. What is still owed is the sentence, and it names both queries.
///
/// **The page comes or does not come by what the row already holds.** The save box has a *keep the
/// page* tick and a chip has nowhere to put one, so the answer is read off the filter being
/// rewritten: one that carries an `offset=` is a place somebody works from and is rewritten with
/// one, and one that does not is a question and stays a question.
///
/// The live filter comes from [`State::songs_filter`] rather than from the body, for the reason
/// [`save_filter`] gives at length.
pub async fn update_saved_filter(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
) -> Response {
    let live = state.songs_filter();
    let now = crate::scan::timestamp();
    let written = state
        .blocking(move |db| {
            let Some(existing) = db.saved_filter(id)? else {
                return Ok(None);
            };
            let query = match existing.query.contains("offset=") {
                true => live,
                false => without_page(&live),
            };
            let written = db.update_saved_filter(id, &query, &now)?;
            Ok(written.then_some(existing.name))
        })
        .await;
    let said = match written {
        // The name and not the queries. A saved filter is a name somebody gave a question, and the
        // question spelled out is a line of `language=pt&favorited=out` that says nothing to the
        // person who just pressed the button on the chip they were looking at.
        Ok(Some(name)) => crate::words::messages(state.locale())
            .msg_with("said-filter-updated", &[("name", name.as_str().into())])
            .into_owned(),
        // Two tabs on one strip is the ordinary way to reach this, and the redraw below is the
        // answer: the chip that was pressed is gone from the page.
        Ok(None) => crate::words::messages(state.locale())
            .msg("said-filter-already-gone")
            .into_owned(),
        Err(error) => return saved_filter_failed(error.say(state.locale())),
    };
    with_saved_strip(&state, said).await
}

/// `GET /songs/saved-filters/{id}/chip`
///
/// One chip, redrawn. `?renaming=1` returns it as the box that renames it, which is `song_row`'s
/// arrangement one route over and for its reason: the two states are one element in two shapes, so
/// one route answers for both and the markup lives in one template.
pub async fn saved_filter_chip(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
    Query(query): Query<ChipQuery>,
) -> Response {
    let renaming = query.renaming.is_some();
    match state.reading(move |db| db.saved_filter(id)).await {
        Ok(Some(filter)) => page(
            &crate::views::SavedFilterChip {
                filter: crate::views::SavedFilterRow::new(filter, state.locale()),
                renaming,
            },
            state.locale(),
        ),
        // The chip has gone since the page was drawn, and answering with nothing is what takes it
        // off the strip -- which is what the page should show.
        Ok(None) => MessageFragment::ok(""),
        // **A message and not a toast**, alone among the strip's routes, because this one targets
        // the chip itself with `outerHTML`: a body holding nothing but an out-of-band toast swaps
        // the empty remainder into the chip, so reporting the failure would take the chip off the
        // page. The sentence lands where the chip was, which is beside the thing it is about.
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// Whether a chip was asked for open for renaming.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ChipQuery {
    #[serde(default)]
    renaming: Option<String>,
}

/// `POST /songs/saved-filters/{id}/rename`
///
/// **A name that is taken is refused rather than replaced**, which is the opposite of what saving
/// does and is the same act pointed the other way: a save writes a query somebody is looking at into
/// a name, so showing the two queries is a fair question to put. A rename writes a name over a query
/// that is not on screen, where the confirmation would have nothing to show and a row would go.
///
/// The answer is the whole strip, not this one chip: the strip is ordered by the fold of the name,
/// so a rename can move a chip past its neighbors.
pub async fn rename_saved_filter(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
    body: String,
) -> Response {
    let Some(name) = Fields::parse(&body)
        .one("saved_name")
        .map(ToOwned::to_owned)
    else {
        return saved_filter_failed(
            crate::words::messages(state.locale())
                .msg("said-name-first")
                .into_owned(),
        );
    };
    let renamed = {
        let name = name.clone();
        state
            .blocking(move |db| db.rename_saved_filter(id, &name))
            .await
    };
    if let Err(error) = renamed {
        return saved_filter_failed(error.say(state.locale()));
    }
    let said = crate::words::messages(state.locale())
        .msg_with("said-filter-renamed", &[("name", name.as_str().into())])
        .into_owned();
    with_saved_strip(&state, said).await
}

/// One sentence and the strip as it now stands, which is what every write to the strip answers with.
///
/// **The sentence is a toast, and so is a refusal.** These buttons sit in a strip above a page of
/// rows, and the slot they would otherwise write into has nobody watching it and nothing to clear
/// it — so a rename from an hour ago stays on the page under the chips, and reads as something that
/// just happened. A toast is seen where it is raised and then goes. What stays in the slot is the
/// confirmation a collision raises, which is a fragment carrying buttons rather than a sentence.
async fn with_saved_strip(state: &State, said: String) -> Response {
    match saved_strip(state).await {
        Ok(strip) => {
            crate::views::with_toast(&strip, &crate::views::Toast::good(said), state.locale())
        }
        Err(error) => saved_filter_failed(error.say(state.locale())),
    }
}

/// `POST /songs/saved-filters/{id}/delete`
///
/// Guarded by an `hx-confirm` on the button rather than by a fragment, which is what the favorites
/// page does for the same weight of act: this touches no song, and saving it again is typing a name.
pub async fn delete_saved_filter(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
) -> Response {
    let gone = state.blocking(move |db| db.delete_saved_filter(id)).await;
    let said = match gone {
        Ok(true) => crate::words::messages(state.locale())
            .msg("said-forgotten")
            .into_owned(),
        // Two tabs showing the same strip is the ordinary way this happens, and the answer is the
        // page that no longer draws it rather than a failure.
        Ok(false) => crate::words::messages(state.locale())
            .msg("said-filter-already-gone")
            .into_owned(),
        Err(error) => return saved_filter_failed(error.say(state.locale())),
    };
    with_saved_strip(&state, said).await
}

/// `POST /songs/favorite-bulk`
///
/// Files the ticked songs, or every song the filter matches, into one favorite — or takes them out
/// of it. Shaped exactly like [`bulk_tag`] and for the same reasons: same scope select with the same
/// default, same two steps, same frozen query on the confirmed pass.
///
/// **Which way it goes is named rather than inferred**, and that is what makes taking songs out
/// safe to offer here at all. A control that put a song in or out depending on whether it was
/// already in would mean one tick doing two opposite things, and an evening's filing could go in a
/// click nobody could see coming. Said out loud, and then counted and confirmed, *take these out* is
/// an ordinary act: a list somebody over-filled is as much work to fix as one they under-filled.
///
/// **`favorite_id`, not the bar's own `favorite`.** The filter bar rides in the same body and
/// narrows by favorite, so two spellings of one key is how this would come to file a set chosen by
/// the favorite it was filing into — `set_language` and `set_tag` above avoid the same trap.
pub async fn bulk_favorite(AxumState(state): AxumState<State>, action: BulkAction) -> Response {
    let BulkAction {
        query,
        whole_filter,
        ticked,
        confirmed,
        form,
    } = action;

    let Some(favorite) = form.parsed::<i64>("favorite_id") else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-choose-a-favorite"),
        );
    };
    let removing = form.one("favorite_action") == Some("remove");
    let filter = query.to_filter();

    if !confirmed {
        let counting = filter.clone();
        let ids = ticked.clone();
        let counted = state
            .blocking(move |db| {
                let count = if whole_filter {
                    db.count_matching(&counting)?
                } else {
                    u32::try_from(ids.len()).unwrap_or(u32::MAX)
                };
                Ok((count, db.favorites()?))
            })
            .await;
        let (count, favorites) = match counted {
            Ok(pair) => pair,
            Err(error) => return MessageFragment::failed(error.say(state.locale())),
        };
        if count == 0 {
            return MessageFragment::failed(match whole_filter {
                false => "Nothing is ticked.".to_owned(),
                true => "Nothing matches that filter.".to_owned(),
            });
        }
        // Named rather than numbered, because the confirmation is where somebody catches the wrong
        // list before a whole filter goes into it.
        let Some(named) = favorites.iter().find(|f| f.id == favorite) else {
            return MessageFragment::failed(
                crate::words::messages(state.locale()).msg("said-favorite-gone"),
            );
        };
        let name = named.name.clone();
        let filters: Vec<String> = if whole_filter {
            query
                .active(&favorites, state.locale())
                .into_iter()
                .map(|chip| chip.label)
                .collect()
        } else {
            vec![format!("{} ticked", ticked.len())]
        };
        let (subject, confirm) = confirm_words(
            state.locale(),
            count,
            "confirm-songs",
            if removing {
                "confirm-unfile"
            } else {
                "confirm-file"
            },
        );
        return page(
            &crate::views::BulkFavoriteConfirm {
                subject,
                confirm,
                whole_corpus: whole_filter && filters.is_empty(),
                filters,
                favorite: name,
                removing,
                query: query.rebuild(0, "", None),
                songs: if whole_filter { Vec::new() } else { ticked },
            },
            state.locale(),
        );
    }

    match state
        .blocking(move |db| match whole_filter {
            true => db.set_favorites_for(&filter, favorite, !removing),
            false => db.set_favorites(&ticked, favorite, !removing),
        })
        .await
    {
        // Zero is ordinary, exactly as it is for a tag: filing songs already in the favorite writes
        // nothing, and that is success rather than a fault.
        Ok(0) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale()).msg("said-nothing-change"),
        )),
        Ok(count) => crate::views::toast_only(&Toast::good(
            crate::words::messages(state.locale()).msg_with(
                if removing {
                    "said-took-out"
                } else {
                    "said-filed"
                },
                &[("count", i64::from(count).into())],
            ),
        )),
        Err(error) => crate::views::toast_only(&Toast::bad(error.say(state.locale()))),
    }
}

/// `GET /songs/favorite-bulk/cancel` — clears the confirmation without doing anything.
pub async fn bulk_favorite_cancel() -> Response {
    MessageFragment::ok("")
}

/// `POST /songs/reanalyze`
///
/// Re-reads the ticked songs, or every song the filter matches, and writes what the analysis says
/// now. The same two steps and the same scope select as the three bulk actions above.
///
/// **It re-reads the files rather than recomputing from the database**, because there is nothing
/// stored to recompute from: a suitability is derived from notes, channels and lyric timings, and
/// the database keeps the conclusion rather than the evidence. So this is a scan of a named set of
/// files — the same code path, the same worker threads, the same progress bar and the same Stop
/// button, which is why it starts a job and answers with where to watch it instead of blocking on
/// a read that can take hours.
///
/// **What it cannot touch is what somebody typed.** A scan writes the `det_` columns and the
/// analysis; a title, an artist, a language, a rating and a note are the person's and are left
/// exactly as they are. That split is `schema.sql`'s and this inherits it rather than restating it.
pub async fn reanalyze(AxumState(state): AxumState<State>, action: BulkAction) -> Response {
    let BulkAction {
        query,
        whole_filter,
        ticked,
        confirmed,
        ..
    } = action;
    let filter = query.to_filter();

    if state.scan_running() {
        return MessageFragment::failed(
            "A scan is already running. Watch it on the Scan page, or stop it there first.",
        );
    }

    // The paths are read on both passes: counted on the first, handed to the job on the second.
    // Counting songs rather than paths would be a number that does not survive the second read — a
    // song whose every copy has gone from disk has nothing to re-read and must not be promised.
    let counting = filter.clone();
    let ids = ticked.clone();
    let paths = match state
        .blocking(move |db| match whole_filter {
            true => db.paths_matching(&counting),
            false => db.paths_of(&ids),
        })
        .await
    {
        Ok(paths) => paths,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    if paths.is_empty() {
        let words = crate::words::messages(state.locale());
        return MessageFragment::failed(match whole_filter {
            false => words.msg("said-nothing-ticked-on-disk"),
            true => words.msg("said-nothing-matching-on-disk"),
        });
    }

    if !confirmed {
        let favorites = match state.blocking(|db| db.favorites()).await {
            Ok(favorites) => favorites,
            Err(error) => return MessageFragment::failed(error.say(state.locale())),
        };
        let filters: Vec<String> = if whole_filter {
            query
                .active(&favorites, state.locale())
                .into_iter()
                .map(|chip| chip.label)
                .collect()
        } else {
            vec![format!("{} ticked", ticked.len())]
        };
        let count = u32::try_from(paths.len()).unwrap_or(u32::MAX);
        let (subject, confirm) =
            confirm_words(state.locale(), count, "confirm-files", "confirm-reread");
        return page(
            &crate::views::ReanalyzeConfirm {
                subject,
                confirm,
                whole_corpus: whole_filter && filters.is_empty(),
                filters,
                query: query.rebuild(0, "", None),
                songs: if whole_filter { Vec::new() } else { ticked },
            },
            state.locale(),
        );
    }

    let count = paths.len();
    let Some(_) = state.start_scan(crate::scan::ScanOptions::only(
        paths.into_iter().collect::<std::collections::HashSet<_>>(),
    )) else {
        return MessageFragment::failed(DbError::NoWorkspace.to_string());
    };
    crate::views::toast_only(&Toast::good(
        crate::words::messages(state.locale()).msg_with(
            "said-re-reading",
            &[("count", i64::try_from(count).unwrap_or(i64::MAX).into())],
        ),
    ))
}

/// `GET /songs/reanalyze/cancel` — clears the confirmation without doing anything.
pub async fn reanalyze_cancel() -> Response {
    MessageFragment::ok("")
}

/// What one song's tag editor is asking for: which way, and which tag.
#[derive(Debug, Default, serde::Deserialize)]
pub struct TagAction {
    /// `add` or `remove`.
    #[serde(default)]
    action: String,
    /// The tag, as typed. Folded on the way in.
    #[serde(default)]
    tag: String,
}

/// `POST /songs/{id}/tags` — puts one tag on one song, or takes it off.
///
/// **The whole editor, both directions, one route.** A tag has no third state — a song carries it or
/// does not — so there is nothing for a separate `DELETE` to express that `action=remove` does not,
/// and one route means the fragment that comes back is composed in one place.
///
/// Read from the query string *and* the body, because the two halves of the editor send it two ways:
/// the ✕ on a chip has nothing but a URL, and the add form has fields. Both spellings are the same
/// question, so both are accepted rather than the ✕ being made into a form of its own.
///
/// Answers with the tag list rather than a toast: what changed is on screen, so saying so in words
/// as well would be saying it twice.
pub async fn song_tags(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<TagAction>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let action = match form.one("action") {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => query.action.clone(),
    };
    let raw = match form.one("tag") {
        Some(value) if !value.trim().is_empty() => value.to_owned(),
        _ => query.tag.clone(),
    };
    let Some(tag) = km_kmpkg::Tag::parse(&raw) else {
        // Said in full rather than as a bare refusal: whoever is reading this typed the word, and a
        // string that folds to nothing looks identical to one that folds to a letter `fold` has no
        // ASCII for.
        return MessageFragment::failed(
            crate::words::messages(state.locale())
                .msg_with("said-not-a-tag", &[("typed", raw.as_str().into())]),
        );
    };

    let ids = vec![id.clone()];
    let lookup = id.clone();
    let removing = action == "remove";
    match state
        .blocking(move |db| {
            if removing {
                db.remove_tag_of(&ids, &tag)?;
            } else {
                db.add_tag_of(&ids, &tag)?;
            }
            db.tags_of(&lookup)
        })
        .await
    {
        Ok(tags) => page(&crate::views::TagList { song_id: id, tags }, state.locale()),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/titles-from-filename`
///
/// Puts each ticked song's own file name in its title, and empties its artist. On the ticked rows
/// and **not** on the whole filter, unlike the language above: a language is shared by a set of songs
/// and can honestly be set a filter at a time, whereas whether a file's name beats its declared
/// title is a judgment about that file, made by looking at it.
///
/// It answers with the rows themselves rather than a message, which no other action here does. The
/// reason is that this is the one bulk action whose whole result is *on the screen already*: leave
/// the table alone and the titles somebody just changed go on showing their old values until
/// something else happens to redraw them. The toast rides along out of band and says how many moved.
/// The ticks are lost with the swap, which is the ordinary consequence of re-rendering and is what
/// somebody who has just finished with this selection wanted anyway. **The page is not**: this
/// changes no filter, so page twelve still means page twelve, and `static/ui.js` sends the offset
/// along from `#rows` to say so.
pub async fn titles_from_filename(
    AxumState(state): AxumState<State>,
    Query(reply): Query<ReplyQuery>,
    body: String,
) -> Response {
    // The bar rides along in the body so the redraw is of the list that is on screen. A query
    // string rendered into the button brings the table back showing whatever the filter was when
    // the page loaded. See `FilterQuery::from_body`.
    let redraw = match Redraw::from_body(&reply, &body) {
        Ok(redraw) => redraw,
        Err(error) => return crate::views::toast_only(&Toast::bad(error)),
    };
    let songs: Vec<String> = Fields::parse(&body)
        .all("song_id")
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    if songs.is_empty() {
        return crate::views::toast_only(&Toast::bad(
            crate::words::messages(state.locale()).msg("said-nothing-was-ticked"),
        ));
    }

    let count = songs.len();
    let changed = match state
        .blocking(move |db| db.set_names_from_stem(&songs))
        .await
    {
        Ok(changed) => changed,
        Err(error) => return crate::views::toast_only(&Toast::bad(error.say(state.locale()))),
    };

    let words = crate::words::messages(state.locale());
    let changed_n = i64::try_from(changed).unwrap_or(i64::MAX);
    let said = match changed == count {
        true => words
            .msg_with("said-titles-taken", &[("count", changed_n.into())])
            .into_owned(),
        // The gap is a song with no `stem`, which only a database written before that column existed
        // and not yet reopened can have. Saying so beats a count that quietly does not add up.
        false => words
            .msg_with(
                "said-titles-taken-short",
                &[
                    ("count", changed_n.into()),
                    (
                        "short",
                        i64::try_from(count - changed).unwrap_or(i64::MAX).into(),
                    ),
                ],
            )
            .into_owned(),
    };
    redraw_over_the_write(&state, redraw, said).await
}

/// `POST /songs/fix-name-case`
///
/// Puts capitals back into the title and artist of each ticked song. The rule is
/// [`crate::casing::recase`]'s, which column it reaches is [`Db::fix_name_case`]'s, and what is here
/// is the answer: on the ticked rows, redrawn in place, for the reasons the button beside it gives.
///
/// **The Titles actions are one shape and share their tail deliberately.** Each writes the ticked
/// rows and each answers with the table, so each owes the filter the same write-down — and a second
/// copy of that rule is a second place to leave it out. See [`redraw_over_the_write`].
///
/// A song already carrying capitals of its own is not a failure and is not reported as one. The count
/// it is short by is names somebody has already decided about, which is what the sentence says.
pub async fn fix_name_case(
    AxumState(state): AxumState<State>,
    Query(reply): Query<ReplyQuery>,
    body: String,
) -> Response {
    let redraw = match Redraw::from_body(&reply, &body) {
        Ok(redraw) => redraw,
        Err(error) => return crate::views::toast_only(&Toast::bad(error)),
    };
    let songs: Vec<String> = Fields::parse(&body)
        .all("song_id")
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    if songs.is_empty() {
        return crate::views::toast_only(&Toast::bad(
            crate::words::messages(state.locale()).msg("said-nothing-was-ticked"),
        ));
    }

    let count = songs.len();
    let changed = match state.blocking(move |db| db.fix_name_case(&songs)).await {
        Ok(changed) => changed,
        Err(error) => return crate::views::toast_only(&Toast::bad(error.say(state.locale()))),
    };

    let words = crate::words::messages(state.locale());
    let changed_n = i64::try_from(changed).unwrap_or(i64::MAX);
    let said = match changed == count {
        true => words
            .msg_with("said-capitals-fixed", &[("count", changed_n.into())])
            .into_owned(),
        false => words
            .msg_with(
                "said-capitals-fixed-short",
                &[
                    ("count", changed_n.into()),
                    (
                        "short",
                        i64::try_from(count - changed).unwrap_or(i64::MAX).into(),
                    ),
                ],
            )
            .into_owned(),
    };
    redraw_over_the_write(&state, redraw, said).await
}

/// `POST /songs/split-artist-from-title`
///
/// Takes the artist out of the title of each ticked song whose artist is blank. The rule is
/// [`crate::names::artist_and_title`]'s, which rows it reaches is [`Db::split_artist_from_title`]'s,
/// and what is here is the answer: on the ticked rows, redrawn in place, for the reasons the two
/// buttons beside it give.
///
/// The third action of one shape, sharing the tail the other two share. See [`redraw_over_the_write`].
///
/// A row that named an artist already, or whose title holds no seam, is not a failure. The count it
/// is short by is rows there was nothing to take out of, which is what the sentence says.
pub async fn split_artist_from_title(
    AxumState(state): AxumState<State>,
    Query(reply): Query<ReplyQuery>,
    body: String,
) -> Response {
    let redraw = match Redraw::from_body(&reply, &body) {
        Ok(redraw) => redraw,
        Err(error) => return crate::views::toast_only(&Toast::bad(error)),
    };
    let songs: Vec<String> = Fields::parse(&body)
        .all("song_id")
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    if songs.is_empty() {
        return crate::views::toast_only(&Toast::bad(
            crate::words::messages(state.locale()).msg("said-nothing-was-ticked"),
        ));
    }

    let count = songs.len();
    let changed = match state
        .blocking(move |db| db.split_artist_from_title(&songs))
        .await
    {
        Ok(changed) => changed,
        Err(error) => return crate::views::toast_only(&Toast::bad(error.say(state.locale()))),
    };

    let words = crate::words::messages(state.locale());
    let changed_n = i64::try_from(changed).unwrap_or(i64::MAX);
    let said = match changed == count {
        true => words
            .msg_with("said-artist-split", &[("count", changed_n.into())])
            .into_owned(),
        false => words
            .msg_with(
                "said-artist-split-short",
                &[
                    ("count", changed_n.into()),
                    (
                        "short",
                        i64::try_from(count - changed).unwrap_or(i64::MAX).into(),
                    ),
                ],
            )
            .into_owned(),
    };
    redraw_over_the_write(&state, redraw, said).await
}

/// Which list the three Titles actions draw again: the Songs page's rows, or the similar-names
/// matches.
///
/// Read before the write, so a filter that cannot be read stops the action rather than leaving names
/// changed and nothing redrawn.
enum Redraw {
    Rows(Box<FilterQuery>),
    Hits(SimilarQuery),
}

impl Redraw {
    fn from_body(reply: &ReplyQuery, body: &str) -> Result<Self, String> {
        if reply.wants_hits() {
            Ok(Self::Hits(SimilarQuery::from_fields(&Fields::parse(body))))
        } else {
            FilterQuery::from_body(body).map(|query| Self::Rows(Box::new(query)))
        }
    }
}

/// The answer all three Titles actions give: the list drawn again where it was, with a toast over it.
///
/// **These are the routes that re-render `#rows` outside the browse pair, and therefore the ones
/// that owe the filter a write-down** — which is what keeps the nav's Songs link and a filter saved
/// afterwards pointing at the page somebody is looking at. `rows.offset` and not the one asked for: a
/// write here can take a song out of a filter that reads titles or artists, and `rows_for` clamps a
/// page that has fallen past the end back onto the last one.
///
/// The write has already happened when this is reached, so a redraw that fails is reported as itself
/// and with the sentence the write earned still in it. Dropping that sentence would send somebody
/// looking for names that did change.
///
/// **The similar-names matches owe the filter nothing**: they are not `#rows`, and the search they
/// were drawn for is posted again as it stands in the bar.
async fn redraw_over_the_write(state: &State, redraw: Redraw, said: String) -> Response {
    let query = match redraw {
        Redraw::Rows(query) => query,
        Redraw::Hits(query) => {
            let query = query.narrowed(state);
            return match similar_for(state, &query).await {
                Ok(hits) => crate::views::with_toast(&hits, &Toast::good(said), state.locale()),
                Err(error) => crate::views::toast_only(&Toast::bad(format!(
                    "{said} The list could not be drawn again: {error}"
                ))),
            };
        }
    };
    let query = &query;
    let rows = match rows_for(state, query).await {
        Ok(rows) => rows,
        Err(error) => {
            return crate::views::toast_only(&Toast::bad(format!(
                "{said} The list could not be drawn again: {error}"
            )));
        }
    };
    state.remember_songs_filter(query.rebuild(rows.offset, "", None));
    crate::views::with_toast(&rows, &Toast::good(said), state.locale())
}

/// `POST /songs/quality-hint`
///
/// Numbers the ticked songs 1, 2, 3 … in the order most likely to end at the first one, so a curator
/// with six files of one song knows which to play first. The order is
/// [`crate::hint::order`]'s and the reasoning behind each key is there.
///
/// **It reads nothing off disk.** Every key is a column a scan already wrote, which is what makes
/// this a button that answers at once rather than a second *Recalculate suitability*. A corpus whose
/// analysis is stale gets a hint from the stale numbers, and correcting those is asked for rather
/// than automatic — the rule `Suitability` in `docs/decisions/songs.md` already states.
///
/// **It writes nothing either**, neither to the database nor to the rows on screen. What it changes
/// is a list held for the run, and what it answers with is the badges that list has just moved.
///
/// [`BulkAction`] is not used here: what that extractor adds is a whole-filter scope and a
/// confirmation pass, and this has neither. *Every matching song* would be a hint over a quarter of
/// a million rows, which is a sort of the corpus wearing a badge, and there is nothing to confirm
/// about a number that is rubbed out by pressing the other button.
pub async fn quality_hint(AxumState(state): AxumState<State>, body: String) -> Response {
    let ticked: Vec<String> = Fields::parse(&body)
        .all("song_id")
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    if ticked.is_empty() {
        return crate::views::toast_only(&Toast::bad(
            crate::words::messages(state.locale()).msg("said-nothing-is-ticked"),
        ));
    }

    let asked = ticked.len();
    let hinted = match state.blocking(move |db| db.quality_hint(&ticked)).await {
        Ok(hinted) => hinted,
        Err(error) => return crate::views::toast_only(&Toast::bad(error.say(state.locale()))),
    };
    if hinted.is_empty() {
        return crate::views::toast_only(&Toast::bad(
            crate::words::messages(state.locale()).msg("said-nothing-ticked-is-midi"),
        ));
    }

    let skipped = asked - hinted.len();
    let previous = state.set_quality_hint(hinted.clone());
    let words = crate::words::messages(state.locale());
    let numbered = i64::try_from(hinted.len()).unwrap_or(i64::MAX);
    let said = match skipped {
        0 => words
            .msg_with("said-numbered", &[("count", numbered.into())])
            .into_owned(),
        // Named rather than counted away: somebody who ticked eight rows and sees six numbers is
        // owed the reason, and it is a fact about what those files are rather than a fault.
        skipped => words
            .msg_with(
                "said-numbered-skipped",
                &[
                    ("count", numbered.into()),
                    ("skipped", i64::try_from(skipped).unwrap_or(i64::MAX).into()),
                ],
            )
            .into_owned(),
    };
    crate::views::with_toast(
        &marks_for(&hinted, &previous),
        &Toast::good(said),
        state.locale(),
    )
}

/// `POST /songs/quality-hint/clear`
///
/// Rubs the numbers out. Nothing else about the page moves: the filter, the page number and the
/// ticks are all where they were, which is the whole reason the hint is a mark rather than a sort.
pub async fn quality_hint_clear(AxumState(state): AxumState<State>) -> Response {
    let previous = state.clear_quality_hint();
    if previous.is_empty() {
        return crate::views::toast_only(&Toast::bad(
            crate::words::messages(state.locale()).msg("said-no-numbers-to-clear"),
        ));
    }
    let count = previous.len();
    crate::views::with_toast(
        &marks_for(&[], &previous),
        &Toast::good(crate::words::messages(state.locale()).msg_with(
            "said-cleared-numbers",
            &[("count", i64::try_from(count).unwrap_or(i64::MAX).into())],
        )),
        state.locale(),
    )
}

/// The badges a hint change has to send: the new numbers, and an empty one per row that lost its.
///
/// A song in both lists appears once, carrying its new number — sending it twice would be two
/// out-of-band swaps of one element, the second of which wins by arriving last rather than by being
/// right.
fn marks_for(hinted: &[String], previous: &[String]) -> crate::views::HintMarks {
    let mut marks: Vec<(String, String)> = hinted
        .iter()
        .enumerate()
        .map(|(at, id)| (id.clone(), (at + 1).to_string()))
        .collect();
    marks.extend(
        previous
            .iter()
            .filter(|id| !hinted.contains(id))
            .map(|id| (id.clone(), String::new())),
    );
    crate::views::HintMarks { marks }
}

/// Where a change made from the list should put its answer.
///
/// The song page and the browse list post to the same routes and want different answers: the page
/// has a message slot and no row, the list has a row and putting "Rated 7/10." somewhere in a
/// hundred-row table helps nobody. One query parameter rather than two sets of routes, because the
/// *action* is the same and only the reply differs.
///
/// `as=toast` is the third answer and the newest. It is for the list's actions that have no row to
/// come back to — play, add-to-package, add-to-favorite — whose message slot is `#action-result`
/// *above* the table: at the bottom of a page of rows that is off the screen, so the result of
/// pressing a button appeared nowhere a person was looking. A page with the slot beside the button
/// asks for neither and gets the message.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ReplyQuery {
    #[serde(default, rename = "as")]
    reply_as: Option<String>,
}

impl ReplyQuery {
    fn wants_row(&self) -> bool {
        self.reply_as.as_deref() == Some("row")
    }

    fn wants_toast(&self) -> bool {
        self.reply_as.as_deref() == Some("toast")
    }

    /// `as=hits` is the similar-names page asking a Titles action for its matches rather than the
    /// Songs page's rows.
    fn wants_hits(&self) -> bool {
        self.reply_as.as_deref() == Some("hits")
    }
}

/// The result of an action, put where the caller asked for it.
///
/// The one place that decides between a toast and a message, so the two can never disagree about
/// which is which. Everything that can be reached from both the browse list and a page ends here.
fn said(reply: &ReplyQuery, ok: bool, text: String) -> Response {
    match (reply.wants_toast(), ok) {
        (true, true) => crate::views::toast_only(&Toast::good(text)),
        (true, false) => crate::views::toast_only(&Toast::bad(text)),
        (false, true) => MessageFragment::ok(text),
        (false, false) => MessageFragment::failed(text),
    }
}

/// `POST /songs/{id}/user-score`
pub async fn user_score(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(reply): Query<ReplyQuery>,
    body: String,
) -> Response {
    let score = Fields::parse(&body).parsed::<u8>("score");
    let lookup = id.clone();
    match state
        .blocking(move |db| db.set_user_score(&lookup, score))
        .await
    {
        Ok(()) if reply.wants_row() => row_response(&state, &id, false).await,
        Ok(()) => said(&reply, true, {
            let words = crate::words::messages(state.locale());
            match score {
                Some(value) => words
                    .msg_with("said-rated", &[("value", i64::from(value).into())])
                    .into_owned(),
                None => words.msg("said-rating-cleared").into_owned(),
            }
        }),
        Err(error) => said(&reply, false, error.say(state.locale())),
    }
}

/// `POST /songs/{id}/rename`
///
/// Title and artist only, from the row's inline editor. Its own route rather than a smaller form
/// posted to [`edit_song`]: that handler treats every box on the form as authoritative, so a form
/// carrying two of the six fields would clear the other four.
///
/// **The artist box is `row_artist`, not `artist`, and the title box beside it is still `title`.**
/// The asymmetry is deliberate: a row lives inside `#rows`, which is
/// `hx-include`d beside `#filters` by *Title from file name* and by the ticked-song actions, and
/// serde answers a repeated *known* key with `duplicate_field`. `FilterQuery` has an `artist` filter
/// and no `title` one, so `artist` is the only name here that would collide — and a row named
/// `artist` would turn those buttons into a 400 whenever a row happened to be open for editing.
/// The prefix goes on what collides and nowhere else, exactly as for `row_language`.
/// See [`FilterQuery::from_body`].
pub async fn rename(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let edit = SongEdit {
        title: Some(form.one("title").map(ToOwned::to_owned)),
        artist: Some(form.one("row_artist").map(ToOwned::to_owned)),
        ..SongEdit::default()
    };
    let lookup = id.clone();
    match state.blocking(move |db| db.edit_song(&lookup, &edit)).await {
        Ok(()) => row_response(&state, &id, false).await,
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/language`
///
/// One song's language, from the select in its row. Its own route rather than a one-field form posted
/// to [`edit_song`], for the reason [`rename`] gives: that handler treats every box on its form as
/// authoritative, so a form carrying one of the six fields would clear the other five.
///
/// The field is **`row_language`, not `language`**, and that naming is load-bearing rather than
/// stylistic. The select lives inside `#rows`, which is `hx-include`d beside `#filters` by *Title
/// from file name* and by the ticked-song actions; `FilterQuery` has a `language` key of its own;
/// and serde answers a repeated known key with `duplicate_field`. A row named `language` would
/// therefore turn two working buttons into a 400 — the identical trap that made the bulk set's
/// select `set_language`. See [`FilterQuery::from_body`].
///
/// `more` is not a language. It is the short list's escape hatch, and it writes nothing: the row comes
/// back as the full picker instead. Recognizing it here rather than giving it a route of its own keeps
/// the whole interaction one `change` on one select.
///
/// An unknown tag is refused by `Db::edit_song`, which is where that rule already lives.
pub async fn set_song_language(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let chosen = form.one("row_language");
    if chosen == Some("more") {
        return language_response(&state, &id).await;
    }
    // An empty choice is a deliberate clear — `— unknown` — exactly as it is on the song page.
    let edit = SongEdit {
        language: Some(chosen.map(ToOwned::to_owned)),
        ..SongEdit::default()
    };
    let lookup = id.clone();
    match state.blocking(move |db| db.edit_song(&lookup, &edit)).await {
        Ok(()) => row_response(&state, &id, false).await,
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/favorites/{favorite}`
///
/// Puts one song in one favorite, or takes it back out. This is what the star in the song list
/// answers to, once it has asked *which* favorite — there is no favoriting that does not name one.
pub async fn toggle_favorite(
    AxumState(state): AxumState<State>,
    UrlPath((id, favorite)): UrlPath<(String, i64)>,
    Query(reply): Query<ReplyQuery>,
) -> Response {
    let lookup = id.clone();
    match state
        .blocking(move |db| db.toggle_favorite(&lookup, favorite))
        .await
    {
        // The row, not a message: the star has to show the new count, and the chooser the click came
        // from has to stop being a chooser.
        Ok(_) if reply.wants_row() => row_response(&state, &id, false).await,
        Ok(true) => MessageFragment::ok(
            crate::words::messages(state.locale()).msg("said-added-to-favorite"),
        ),
        Ok(false) => MessageFragment::ok(
            crate::words::messages(state.locale()).msg("said-removed-from-favorite"),
        ),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/favorites/new`
///
/// Creates a favorite and puts this song in it, from the chooser in the song list. Two steps in one
/// route because the first favorite has to be creatable at the moment somebody wants it: a chooser
/// offering nothing, on a page with no way to add one, is a dead end.
pub async fn favorite_into_new(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(reply): Query<ReplyQuery>,
    body: String,
) -> Response {
    let name = Fields::parse(&body)
        .one("name")
        .unwrap_or_default()
        .to_owned();
    let lookup = id.clone();
    let result = state
        .blocking(move |db| {
            let favorite = db.create_favorite(&name)?;
            db.set_favorite(&lookup, favorite, true)?;
            Ok(name)
        })
        .await;

    match result {
        Ok(_) if reply.wants_row() => row_response(&state, &id, false).await,
        Ok(name) => MessageFragment::ok(format!("Created {name} and filed it there.")),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/favorites`
pub async fn song_favorites(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    // A checkbox that is off sends nothing at all, so the set of ticked boxes is the whole answer and
    // everything else must be removed. Every ticked box arrives under the same name, which is the
    // shape a serde form cannot express — see `form.rs`.
    let ticked: Vec<i64> = Fields::parse(&body)
        .all("favorite")
        .iter()
        .filter_map(|value| value.parse().ok())
        .collect();

    let result = state
        .blocking(move |db| {
            for (favorite, _) in db.favorites_for(&id)? {
                db.set_favorite(&id, favorite, ticked.contains(&favorite))?;
            }
            for favorite in &ticked {
                db.set_favorite(&id, *favorite, true)?;
            }
            Ok(ticked.len())
        })
        .await;

    match result {
        Ok(0) => {
            MessageFragment::ok(crate::words::messages(state.locale()).msg("said-in-no-favorites"))
        }
        Ok(count) => MessageFragment::ok(format!("In {count} favorite(s).")),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/pin-encoding`
pub async fn pin_encoding(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    let encoding = Fields::parse(&body)
        .one("encoding")
        .unwrap_or_default()
        .to_owned();
    let edit = SongEdit {
        lyric_encoding: Some((!encoding.is_empty()).then(|| encoding.clone())),
        ..SongEdit::default()
    };
    match state.blocking(move |db| db.edit_song(&id, &edit)).await {
        Ok(()) => MessageFragment::ok(crate::words::messages(state.locale()).msg_with(
            "said-encoding-pinned",
            &[("encoding", encoding.as_str().into())],
        )),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/unmerge`
pub async fn unmerge(AxumState(state): AxumState<State>, UrlPath(id): UrlPath<String>) -> Response {
    match state.blocking(move |db| db.unmerge(&id)).await {
        Ok(()) => MessageFragment::ok(crate::words::messages(state.locale()).msg("said-unmerged")),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

// -- listening ------------------------------------------------------------------------------

/// `POST /songs/{id}/play`
pub async fn play(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(reply): Query<ReplyQuery>,
    body: String,
) -> Response {
    let lookup = id.clone();
    // The song's own row as well as its file, because a preview that worked everything out for
    // itself would play the song as it was found — which is the one thing somebody pressing Play to
    // check what they just typed is not asking to hear. The corrections are in no package yet and
    // the title somebody retyped is in this database and nowhere in the bytes.
    let looked_up = state
        .blocking(move |db| Ok((db.best_file(&lookup)?, db.song(&lookup)?)))
        .await;
    let (path, detail) = match looked_up {
        Ok(((_, path), detail)) => (path, detail),
        Err(error) => return said(&reply, false, error.say(state.locale())),
    };
    let fixes = crate::fixes::stored(detail.fixes.as_deref());
    // The effective values, which is what the page shows this song under — so the tool and the
    // television agree about what is playing. `effective_title` falls back to the file's own name,
    // which is what the machine would have arrived at anyway.
    let title = detail.effective_title();
    let artist = detail.effective_artist();
    // Absolute either way, and for two reasons rather than one. A machine on this box resolves the
    // path against its *own* working directory, which is not this tool's; a machine elsewhere never
    // sees the path at all, and this is simply how the file gets opened here to be read.
    let absolute = crate::model::tidy(&path);
    // An UltraStar song is found from its `.txt`, and the machine never reads one: the words are
    // read here, and the machine is sent the MP3 the header names with the words beside it. The
    // stem stays the `.txt`'s, so a title the machine falls back to is the one this page shows.
    let ultrastar = if km_pack::is_ultrastar_candidate(&absolute) {
        let text = absolute.clone();
        let read = tokio::task::spawn_blocking(move || km_pack::read_ultrastar(&text)).await;
        match read {
            Ok(Ok(source)) => Some(source),
            // Worded as the scan reported the same refusal, which is the sentence on this row already.
            Ok(Err(refusal)) => {
                return said(&reply, false, format!("{}: {refusal}", absolute.display()));
            }
            Err(join) => return said(&reply, false, join.to_string()),
        }
    } else {
        None
    };
    let decided = km_api::Audition {
        title: Some(title.as_str()),
        artist: artist.as_deref(),
        // Clamped where the description clamps it, so a preview and the package built from the same
        // row are in the same key. The machine clamps again after adding the operator's own
        // default, which is a different limit for a different reason.
        transpose: detail
            .default_transpose
            .map(|value| value.clamp(-12, 12) as i8),
        fixes: fixes.as_deref(),
        // What a package built from this row would carry, so the guide-melody toggle a preview
        // offers is the one the package will.
        melody: crate::fixes::MelodyChoice::parse(detail.melody_chosen.as_deref())
            .map(crate::fixes::MelodyChoice::channel),
        lyrics: ultrastar.as_ref().map(|source| &source.song.timeline),
    };
    // The file the machine plays: an UltraStar song's MP3, and every other song's own file.
    let played = ultrastar
        .as_ref()
        .map_or(absolute.as_path(), |source| source.audio.as_path());

    // Only the path route's refusal message uses this, but a workspace with no root is an error on
    // both branches — so it stays where it is rather than becoming conditional.
    let root = match root_of(&state) {
        Ok(root) => root,
        Err(error) => return said(&reply, false, error.say(state.locale())),
    };
    let client = state.app_client().await;
    // **The whole of the local/remote split, and it is decided by the address.** A machine on this
    // box is handed the path: instant, and it copies nothing — which matters most for exactly the
    // songs the bytes route handles worst, a loose video of a few hundred megabytes. A machine
    // anywhere else cannot open a path from this disk, so it gets the song itself.
    let sent = if client.is_loopback() {
        client.play_file(played, &root, &decided).await
    } else {
        // Found here rather than over there: an MP3+G song is a `.mp3` and a `.cdg` sharing a stem,
        // and the half beside this one is on *this* disk. Both travel, staged under one name. An
        // UltraStar song's MP3 has no partner: its words travel as a field.
        let partner = if ultrastar.is_some() {
            None
        } else {
            km_kmpkg::pair_for(&absolute)
        };
        let stem = absolute
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("song");
        client
            .play_upload(played, partner.as_deref(), stem, &decided)
            .await
    };
    if let Err(error) = sent {
        return said(&reply, false, error.say(state.locale()));
    }

    // Recorded only once the machine has actually taken it, so a refused play does not move the
    // highlight. Kept in the database rather than in memory: which song was last tried is exactly the
    // thing somebody wants to know after closing the browser and coming back to the same list.
    let stored = id.clone();
    // The play worked; failing to remember it is not worth turning that into an error.
    if let Err(error) = state
        .blocking(move |db| db.set_setting(LAST_PLAYED_SETTING, &stored))
        .await
    {
        tracing::warn!(%error, "could not record the last played song");
    }

    // What to un-light is what the page sent as lit, not the song recorded above. That setting is
    // shared by every tab, so a play in another tab moves it to a song this page may not show, and
    // this page's own lit button would stay lit. See `play_button.html`.
    let mut unlit: Vec<String> = Vec::new();
    for lit in Fields::parse(&body).all("lit") {
        // The same song again is not a change of highlight, and asking htmx to swap one element
        // twice in one response would replace the new button with the old one.
        if lit != id && !unlit.iter().any(|seen| seen == lit) {
            unlit.push(lit.to_owned());
        }
    }

    // The two routes are told apart on purpose. A remote play copies the song across the network
    // first, so somebody who waited twenty seconds for a video has been told why — and somebody
    // whose machine is not the one they meant can see it in the sentence.
    let text = match client.is_loopback() {
        true => format!("Playing on {}.", client.base()),
        false => format!("Sent to {} and playing.", client.base()),
    };
    let buttons = PlayedFragment {
        played_id: id,
        unlit,
    };

    // A toast from the list and from a song's own page alike: a play that started is news about
    // something that is over. A refusal goes where `said` puts it. The buttons go out of band either
    // way, and are inert on a song's page, which has no `play-{id}` span to find.
    crate::views::with_toast(&buttons, &Toast::good(text), state.locale())
}

/// `POST /songs/{id}/reveal`
pub async fn reveal(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(reply): Query<ReplyQuery>,
) -> Response {
    let lookup = id.clone();
    let path = match state.blocking(move |db| db.best_file(&lookup)).await {
        Ok((_, path)) => path,
        Err(error) => return said(&reply, false, error.say(state.locale())),
    };
    let display = path.display().to_string();
    let words = crate::words::messages(state.locale());
    match tokio::task::spawn_blocking(move || km_osopen::open(&path)).await {
        Ok(Ok(())) => said(
            &reply,
            true,
            words
                .msg_with(
                    "said-handed-to-opener",
                    &[("file", display.as_str().into())],
                )
                .into_owned(),
        ),
        Ok(Err(error)) => said(
            &reply,
            false,
            words
                .msg_with(
                    "said-could-not-open",
                    &[("why", error.to_string().as_str().into())],
                )
                .into_owned(),
        ),
        Err(error) => said(
            &reply,
            false,
            words
                .msg_with(
                    "said-worker-died",
                    &[("why", error.to_string().as_str().into())],
                )
                .into_owned(),
        ),
    }
}

// -- favorites -----------------------------------------------------------------------------

/// `GET /favorites`
pub async fn favorites(AxumState(state): AxumState<State>) -> Response {
    let chrome = match chrome(&state, "favorites").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    match favorites_table(&state, false).await {
        Ok(table) => page(&FavoritesPage { chrome, table }, state.locale()),
        Err(error) => failure(error, state.locale()),
    }
}

/// The lists, drawn for the page or for the copy a write swaps in.
///
/// **One reader, because six callers draw the same table** — the page, and the five writes that
/// change what is in it.
///
/// Both queries in one closure, so naming the packages a list feeds costs no second round trip and
/// no query per row: the links are read once and grouped here.
async fn favorites_table(
    state: &State,
    oob: bool,
) -> Result<crate::views::FavoritesTable, crate::db::DbError> {
    let (favorites, links) = state
        .reading(|db| Ok((db.favorites()?, db.package_sources_all()?)))
        .await?;
    let locale = state.locale();
    // The filings first and the working lists after a rule, as the row's chooser orders them.
    let (filings, working): (Vec<_>, Vec<_>) =
        favorites.into_iter().partition(|node| !node.temporary);
    let filed_count = filings.len();
    Ok(crate::views::FavoritesTable {
        favorites: filings
            .into_iter()
            .chain(working)
            .enumerate()
            .map(|(index, node)| {
                let sourcing: Vec<String> = links
                    .iter()
                    .filter(|link| link.favorite_id == node.id)
                    .map(|link| link.package_name.clone())
                    .collect();
                let mut row = crate::views::FavoriteRow::new(node, &sourcing, locale);
                row.rule_before = index > 0 && index == filed_count;
                row
            })
            .collect(),
        oob,
    })
}

/// A write on the Favorites page: its sentence, and the table it has just changed.
///
/// **The table rides back rather than a line asking for a reload**, which is what the Packages page
/// does beside it and what this tool asks for nowhere else.
async fn said_with_favorites(state: &State, ok: bool, text: String) -> Response {
    if !ok {
        return MessageFragment::failed(text);
    }
    match favorites_table(state, true).await {
        Ok(table) => {
            crate::views::with_toast(&table, &crate::views::Toast::good(text), state.locale())
        }
        // The write happened; only the redraw did not, and its own sentence would replace the one
        // the write earned.
        Err(_) => crate::views::toast_only(&crate::views::Toast::good(text)),
    }
}

/// `POST /favorites`
pub async fn create_favorite(AxumState(state): AxumState<State>, body: String) -> Response {
    let form = Fields::parse(&body);
    let name = form.one("name").unwrap_or_default().to_owned();
    let shown = name.clone();
    match state.blocking(move |db| db.create_favorite(&name)).await {
        Ok(_) => {
            let said = crate::words::messages(state.locale())
                .msg_with("said-favorite-created", &[("name", shown.as_str().into())])
                .into_owned();
            said_with_favorites(&state, true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /favorites/add`
///
/// Puts every ticked song on the browse page in one favorite. The counterpart of "add ticked songs
/// to a package", and the reason favoriting no longer means opening a hundred song pages.
///
/// It only ever adds. A bulk action that could also *un*file would need the same tick to mean two
/// opposite things depending on state, and getting it wrong would quietly empty a favorite somebody
/// spent an evening filling.
pub async fn add_to_favorite(
    AxumState(state): AxumState<State>,
    Query(reply): Query<ReplyQuery>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let Some(favorite) = form.parsed::<i64>("favorite_id") else {
        return said(&reply, false, "Choose a favorite first.".to_owned());
    };
    let songs: Vec<String> = form
        .all("song_id")
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    if songs.is_empty() {
        return said(&reply, false, "Nothing was ticked.".to_owned());
    }
    let count = songs.len();

    let result = state
        .blocking(move |db| {
            // One transaction: a loop of `set_favorite` was a commit and an fsync per ticked song.
            db.set_favorites(&songs, favorite, true)?;
            Ok(())
        })
        .await;

    match result {
        Ok(()) => said(&reply, true, format!("Filed {count} song(s).")),
        Err(error) => said(&reply, false, error.say(state.locale())),
    }
}

/// `POST /favorites/{id}/rename`
pub async fn rename_favorite(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
    body: String,
) -> Response {
    let name = Fields::parse(&body)
        .one("name")
        .unwrap_or_default()
        .to_owned();
    let shown = name.clone();
    match state
        .blocking(move |db| db.rename_favorite(id, &name))
        .await
    {
        Ok(()) => {
            let said = crate::words::messages(state.locale())
                .msg_with("said-favorite-renamed", &[("name", shown.as_str().into())])
                .into_owned();
            said_with_favorites(&state, true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /favorites/{id}/temporary`
///
/// Says whether a list is scaffolding for a later pass or a filing.
///
/// An unticked checkbox sends nothing at all, so the presence of the field is the whole answer —
/// the reading `keep_page` on the save box already relies on, and the reason there is no hidden
/// `temporary=0` beside the box: `serde_urlencoded` answers a repeated known key with a 400, so
/// *ticking* it would fail.
///
/// The sentence names the list, because the box is one of a column of boxes and the row it sits in
/// is the only thing saying which.
pub async fn set_favorite_temporary(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
    body: String,
) -> Response {
    let temporary = Fields::parse(&body).has("temporary");
    let named = state
        .blocking(move |db| {
            db.set_favorite_temporary(id, temporary)?;
            Ok(db
                .favorites()?
                .into_iter()
                .find(|favorite| favorite.id == id)
                .map(|favorite| favorite.name))
        })
        .await;
    let name = match named {
        Ok(Some(path)) => path,
        Ok(None) => "That favorite".to_owned(),
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    let words = crate::words::messages(state.locale());
    let key = if temporary {
        "said-now-a-working-list"
    } else {
        "said-now-a-filing"
    };
    let said = words
        .msg_with(key, &[("name", name.as_str().into())])
        .into_owned();
    said_with_favorites(&state, true, said).await
}

/// `POST /favorites/{id}/delete`
pub async fn delete_favorite(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
) -> Response {
    match state.blocking(move |db| db.delete_favorite(id)).await {
        Ok(()) => {
            let said = crate::words::messages(state.locale())
                .msg("said-favorite-deleted")
                .into_owned();
            said_with_favorites(&state, true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /favorites/{id}/tidy`
pub async fn tidy_favorite(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<i64>,
) -> Response {
    match state.blocking(move |db| db.tidy_favorite(id)).await {
        Ok(0) => {
            MessageFragment::ok(crate::words::messages(state.locale()).msg("said-nothing-to-drop"))
        }
        Ok(removed) => {
            let said = crate::words::messages(state.locale())
                .msg_with(
                    "said-dropped-second-copies",
                    &[("count", i64::try_from(removed).unwrap_or(i64::MAX).into())],
                )
                .into_owned();
            said_with_favorites(&state, true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `GET /duplicates`
pub async fn duplicates(AxumState(state): AxumState<State>) -> Response {
    let chrome = match chrome(&state, "duplicates").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    match state.reading(|db| db.cluster_counts()).await {
        Ok(counts) => {
            let locale = state.locale();
            let found = crate::words::messages(locale)
                .msg_with(
                    "duplicates-found",
                    &[
                        ("groups", i64::from(counts.clusters).into()),
                        ("hidden", i64::from(counts.set_aside).into()),
                    ],
                )
                .into_owned();
            page(
                &DuplicatesPage {
                    chrome,
                    counts,
                    found,
                },
                locale,
            )
        }
        Err(error) => failure(error, state.locale()),
    }
}

/// `POST /duplicates/suggest`
///
/// **Asked for, rather than run by every scan.** Bucketing hundreds of thousands of fingerprints is a
/// whole-`songs` read, and a scan that found one file that moved would pay it to produce suggestions
/// almost identical to the stored ones. Here the cost falls on somebody who wants the answer. A full
/// re-analysis is the other way of asking, and ends with this same pass.
///
/// **The bucketing runs outside the lock**, between two short ones. It is pure CPU over a vector it
/// owns and touches no database, and the tool has no reason to be unanswerable for the length of it.
pub async fn suggest_duplicates(AxumState(state): AxumState<State>) -> Response {
    let prints = match state.blocking(|db| db.fingerprints()).await {
        Ok(prints) => prints,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    let pairs = tokio::task::spawn_blocking(move || crate::dupes::suggest(&prints)).await;
    let Ok(pairs) = pairs else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-suggestion-pass-unfinished"),
        );
    };
    let stored = state
        .blocking(move |db| {
            let added = db.store_candidates(&pairs)?;
            // Same button, second pass. Grouping is what the rest of the tool reads -- the pairs
            // themselves are only ever an input to it -- so a suggestion stored without being
            // grouped would change nothing anybody can see.
            let counts = db.cluster()?;
            Ok((added, counts))
        })
        .await;
    match stored {
        // The clusters are what the sentence leads with, because they are what a curator will see.
        // The pair count is this pass talking about itself -- what it proposed, less what somebody
        // has already passed a verdict on -- and it goes last for that reason.
        Ok((added, counts)) => {
            MessageFragment::ok(crate::words::messages(state.locale()).msg_with(
                "said-grouping-done",
                &[
                    ("groups", i64::from(counts.clusters).into()),
                    ("hidden", i64::from(counts.set_aside).into()),
                    ("pairs", i64::try_from(added).unwrap_or(i64::MAX).into()),
                ],
            ))
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/release`
///
/// **This song's own row and no other.** *This file is not that recording* and *stop grouping any
/// of these* are different requests, and somebody looking at one song is making the first. The next
/// suggestion pass puts it back, which is why the pair's Different button sits beside this one:
/// releasing is for looking, dismissing is for good.
pub async fn release(AxumState(state): AxumState<State>, UrlPath(id): UrlPath<String>) -> Response {
    match state.blocking(move |db| db.release_from_cluster(&id)).await {
        Ok(()) => {
            MessageFragment::ok(crate::words::messages(state.locale()).msg("said-shown-again"))
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /songs/{id}/not-the-same/{other}`
pub async fn not_the_same(
    AxumState(state): AxumState<State>,
    UrlPath((id, other)): UrlPath<(String, String)>,
) -> Response {
    match state.blocking(move |db| db.dismiss_pair(&id, &other)).await {
        Ok(()) => {
            MessageFragment::ok(crate::words::messages(state.locale()).msg("said-pair-dismissed"))
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

// -- packages -------------------------------------------------------------------------------

/// The list of packages, drawn for the page or for the copy a write swaps in.
///
/// **One reader, because four callers draw the same table** — the page, and the three writes that
/// change what is in it. Two spellings of *which packages are there and which lists decide them* is
/// one that stops agreeing with the other.
///
/// `packages` and not `packages_taking_songs`: this page lists every package, and the sourced ones
/// are marked with the lists that decide them rather than left out.
async fn packages_table(
    state: &State,
    oob: bool,
) -> Result<crate::views::PackagesTable, crate::db::DbError> {
    let (packages, links) = state
        .reading(|db| Ok((db.packages()?, db.package_sources_all()?)))
        .await?;
    let locale = state.locale();
    Ok(crate::views::PackagesTable {
        packages: packages
            .into_iter()
            .map(|row| {
                let sources: Vec<String> = links
                    .iter()
                    .filter(|link| link.package_id == row.id)
                    .map(|link| link.favorite_name.clone())
                    .collect();
                crate::views::PackageListRow::new(row, sources, locale)
            })
            .collect(),
        oob,
    })
}

/// A write on the Packages page: its sentence, and the table it has just changed.
///
/// **The table rides back rather than a line asking for a reload.** Making a package, deleting one
/// and opening a built file each change which rows are there, and this tool reloads nothing anywhere
/// else — so the row somebody just made appears, and the row they just deleted goes.
async fn said_with_packages(state: &State, ok: bool, text: String) -> Response {
    if !ok {
        return MessageFragment::failed(text);
    }
    match packages_table(state, true).await {
        Ok(table) => {
            crate::views::with_toast(&table, &crate::views::Toast::good(text), state.locale())
        }
        // The write happened; only the redraw did not. Its own sentence would replace the one the
        // write earned, so the table is left as it was and the reader is told what they did.
        Err(_) => crate::views::toast_only(&crate::views::Toast::good(text)),
    }
}

/// `GET /packages`
pub async fn packages(AxumState(state): AxumState<State>) -> Response {
    let chrome = match chrome(&state, "packages").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    match packages_table(&state, false).await {
        Ok(table) => {
            let locale = state.locale();
            page(
                &PackagesPage {
                    chrome,
                    table,
                    numbering: crate::words::messages(locale)
                        .msg_with(
                            "packages-numbering",
                            &[("highest", i64::from(km_songcode::MAX_SLOT).into())],
                        )
                        .into_owned(),
                },
                locale,
            )
        }
        Err(error) => failure(error, state.locale()),
    }
}

/// Refuses a typed version that is not `X.Y.Z`, and says what one looks like.
///
/// **The two forms a person types into, and nowhere else.** `build::import` writes straight to the
/// database with whatever a `.kmpkg` built elsewhere carries, because refusing an import over a
/// label would throw away the songs to save the string. The build page says such a version cannot
/// be raised, and one edit here makes it one that can.
fn version_refusal(version: &str, locale: km_locale::Locale) -> Option<Response> {
    if crate::version::parse(version).is_some() {
        return None;
    }
    Some(MessageFragment::failed(
        crate::words::messages(locale)
            .msg_with("said-version-refused", &[("version", version.into())]),
    ))
}

/// Refuses a typed first number that is not a number a song can carry, and says what one is.
///
/// **The same division as [`version_refusal`]**: the `max` on the box saves a round trip and this is
/// the authority, because a start number also reaches a row from `build::import`, which reads one out
/// of a manifest and has no form to be answered. [`package_row`] clamps for that caller; a person
/// typing into a box is told instead, because a number silently lowered to the last slot makes a
/// package that takes one song and says nothing about why.
///
/// `None` for a form with no such key at all, which is what leaves the make-a-package-from-a-filter
/// form — which does not ask — alone.
fn start_number_refusal(form: &Fields, locale: km_locale::Locale) -> Option<Response> {
    let number = form.parsed::<u32>("start_number")?;
    if number >= 1 && number <= u32::from(km_songcode::MAX_SLOT) {
        return None;
    }
    Some(MessageFragment::failed(
        crate::words::messages(locale).msg_with(
            "said-start-number-refused",
            &[
                ("highest", i64::from(km_songcode::MAX_SLOT).into()),
                ("number", i64::from(number).into()),
            ],
        ),
    ))
}

/// Reads the package form into a row, defaulting the fields somebody left blank.
fn package_row(form: &Fields, id: &str) -> PackageRow {
    PackageRow {
        id: id.to_owned(),
        // A package with no name shows as its id, which is at least something to recognize.
        name: form.one("name").unwrap_or(id).to_owned(),
        version: form.one("version").unwrap_or("1.0.0").to_owned(),
        publisher: form.one("publisher").map(ToOwned::to_owned),
        // Blank means `vol{n}`, the format a package starts with.
        volume_format: form
            .one("volume_format")
            .map(str::trim)
            .filter(|format| !format.is_empty())
            .unwrap_or(crate::model::DEFAULT_VOLUME_FORMAT)
            .to_owned(),
        // Clamped at both ends rather than lifted at the low one only. A number above the limit
        // cannot be dialled, and silently taking it would push the whole package past the ceiling
        // one song at a time.
        //
        // **For the caller with no form to answer**, which is `build::import` reading a start number
        // out of a manifest: a person who typed one meets `start_number_refusal` first and is told.
        start_number: form
            .parsed::<u32>("start_number")
            .unwrap_or(1)
            .clamp(1, u32::from(km_songcode::MAX_SLOT)),
        // **Absent and blank mean different things here**, which is the distinction the file-name
        // toggle on the browse bar already turns on. A form that carries the key and leaves it empty
        // is somebody choosing *no default*, and the build then refuses a song with no language —
        // the behavior this tool had before the field existed. A form with no such key at all is
        // the Create form, which does not ask, and that gets the same `en` the column's own default
        // gives a package whose form never asked.
        default_language: if form.has("default_language") {
            form.one("default_language")
                .filter(|value| km_kmpkg::Language::parse(value).is_some())
                .map(ToOwned::to_owned)
        } else {
            Some("en".to_owned())
        },
        ..PackageRow::new(id, id)
    }
}

/// The id a create form supplied, if it supplied a usable one.
///
/// Its own function so the "generated unless given" rule has a test that does not need a database
/// behind it — [`create_package`] is the only caller, and the branch it guards is the whole of this
/// change to how a package is identified.
///
/// `None` means *generate one*. A field that is absent, empty or whitespace all mean the same thing,
/// because a form that posts `id=` is indistinguishable from one that omits it and neither is a
/// request for a package called nothing.
fn supplied_id(form: &Fields) -> Option<&str> {
    form.one("id").map(str::trim).filter(|id| !id.is_empty())
}

/// `POST /packages`
///
/// **The id is generated, not asked for**, and the form no longer carries the field. An id is what
/// the machine keys an install on, so two packagers who both reach for `karaoke-vol1` do not get a
/// clash they can see — the second install is taken for an *upgrade* and replaces a volume that has
/// nothing to do with it. See `PackageMeta::new_id`.
///
/// A supplied id is still honored, because `import` comes through here with the one it read out of
/// a built `.kmpkg` and must not be given a new identity. Nothing in the page sends one.
pub async fn create_package(AxumState(state): AxumState<State>, body: String) -> Response {
    let form = Fields::parse(&body);
    let supplied = supplied_id(&form);
    // A name is required from the *form* now, and only from the form: it used to fall back to the
    // id, which was a name somebody chose and is now sixteen hex characters. `Db::create_package`
    // keeps that fallback so `import` can still open a package whose manifest names nothing.
    if supplied.is_none() && form.one("name").is_none_or(|name| name.trim().is_empty()) {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-package-needs-a-name"),
        );
    }
    let id = supplied.map_or_else(km_kmpkg::PackageMeta::new_id, ToOwned::to_owned);
    let row = package_row(&form, &id);
    // Taken off the row rather than off the form, so the sentence says the name the package was
    // created under — which is the id when a manifest named nothing, and `Db::create_package`'s
    // fallback is the only thing that knows.
    let name = row.name.clone();
    if let Some(refusal) = version_refusal(&row.version, state.locale()) {
        return refusal;
    }
    if let Some(refusal) = start_number_refusal(&form, state.locale()) {
        return refusal;
    }
    let now = crate::scan::timestamp();
    match state
        .blocking(move |db| db.create_package(&row, &now))
        .await
    {
        // **The name and not the id.** An id is generated and sixteen hexadecimal characters, and
        // the one thing somebody has just typed is the name — so a sentence confirming what they
        // did says it back. The id is on the package's own page, which is where it is wanted.
        Ok(()) => {
            let said = crate::words::messages(state.locale())
                .msg_with("said-package-created", &[("name", name.as_str().into())])
                .into_owned();
            said_with_packages(&state, true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /packages/{id}/settings`
pub async fn package_settings(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let row = package_row(&form, &id);
    // **Refused without `{n}`**, because a format that does not write the number gives every volume
    // one name and one file, and the second build would overwrite the first.
    if !row.volume_format.contains(crate::model::VOLUME_NUMBER) {
        return MessageFragment::failed(crate::words::messages(state.locale()).msg_with(
            "said-volume-format-refused",
            &[("format", row.volume_format.as_str().into())],
        ));
    }
    // Said back, because it is the one setting on this form that changes what the package *contains*
    // rather than what it is called, and the difference between "en" and blank is a build that
    // succeeds and a build that refuses.
    let words = crate::words::messages(state.locale());
    let said = match row.default_language.as_deref() {
        Some(code) => words
            .msg_with("said-default-language-set", &[("code", code.into())])
            .into_owned(),
        None => words.msg("said-default-language-cleared").into_owned(),
    };
    match state
        .blocking(move |db| {
            db.update_package_details(
                &row.id,
                &row.name,
                row.publisher.as_deref(),
                row.default_language.as_deref(),
                &row.volume_format,
            )
        })
        .await
    {
        Ok(()) => crate::views::toast_only(&crate::views::Toast::good(said)),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /packages/{id}/volume?volume=` — one volume's first number, or its version.
///
/// **Two small forms post here, and each sends only its own field.** The first number sits over the
/// member table it numbers, and the version sits on the Build tab beside the file it names; a field
/// a form did not send is left as it is.
///
/// What rides back is what the field changed: a first number re-words the Re-flow button that names
/// it, and a version re-draws the Build tab, whose output name and raise label both carry it.
pub async fn package_volume_settings(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let volume = volume.number();
    let version = form.one("version").map(|value| value.trim().to_owned());
    if let Some(version) = &version
        && let Some(refusal) = version_refusal(version, state.locale())
    {
        return refusal;
    }
    if let Some(refusal) = start_number_refusal(&form, state.locale()) {
        return refusal;
    }
    let start_number = form.parsed::<u32>("start_number");
    let lookup = id.clone();
    let written = version.clone();
    let saved = state
        .blocking(move |db| {
            db.update_volume(&lookup, volume, written.as_deref(), start_number)?;
            db.package_volume(&lookup, volume)
        })
        .await;
    let package = match saved {
        Ok(package) => package,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    let toast = crate::views::Toast::good(
        crate::words::messages(state.locale())
            .msg("said-saved")
            .into_owned(),
    );
    if version.is_some() {
        return match build_pane_for(&state, package, true).await {
            Ok(pane) => crate::views::with_toast(&pane, &toast, state.locale()),
            Err(error) => MessageFragment::failed(error.say(state.locale())),
        };
    }
    // The re-flow button rides back, because it names the first number and this is the form that
    // changes it. Left alone it would go on naming the number it was drawn with while pressing it
    // re-flowed from the one just saved.
    crate::views::with_toast(
        &crate::views::RenumberForm::new(id, volume, package.start_number, true, state.locale()),
        &toast,
        state.locale(),
    )
}

/// `GET /packages/{id}/build/pane?volume=` — the Build tab, for the volume its picker names.
pub async fn build_pane(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
) -> Response {
    let volume = volume.number();
    let package = match state
        .reading(move |db| db.package_volume(&id, volume))
        .await
    {
        Ok(package) => package,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    match build_pane_for(&state, package, false).await {
        Ok(pane) => page(&pane, state.locale()),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// The Build tab for one volume of a package: the one constructor the page and both swaps go through.
async fn build_pane_for(
    state: &State,
    package: PackageRow,
    oob: bool,
) -> Result<crate::views::BuildPane, DbError> {
    let lookup = package.id.clone();
    let (volumes, raise_version) = state
        .reading(move |db| Ok((db.package_volumes(&lookup)?, db.raise_version(&lookup)?)))
        .await?;
    let root = root_of(state)?;
    Ok(crate::views::BuildPane::new(
        &root,
        package,
        &volumes,
        raise_version,
        oob,
        state.locale(),
    ))
}

/// `POST /packages/{id}/delete`
pub async fn delete_package(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    match state.blocking(move |db| db.delete_package(&id)).await {
        Ok(()) => {
            let said = crate::words::messages(state.locale())
                .msg("said-package-deleted")
                .into_owned();
            said_with_packages(&state, true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `GET /packages/{id}`
pub async fn package(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
) -> Response {
    let chrome = match chrome(&state, "packages").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    let lookup = id.clone();
    let volume = volume.number();
    let loaded = state
        .reading(move |db| {
            Ok((
                db.package_volume(&lookup, volume)?,
                db.package_volumes(&lookup)?,
                db.package_members(&lookup, volume)?,
                db.languages_present()?,
                db.raise_version(&lookup)?,
                // In the same closure as the rest, so the sources editor costs no extra round trip.
                db.favorites()?,
                db.package_sources(&lookup)?,
            ))
        })
        .await;
    let (package, volumes, members, present, raise_version, favorites, sources) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => return failure(error, state.locale()),
    };
    let root = match root_of(&state) {
        Ok(root) => root,
        Err(error) => return failure(error, state.locale()),
    };
    // Built before the page, because the chips over the song list take this panel's own list — two
    // sights of which favorites a package reads, and one answer behind both.
    let sourcing = crate::views::SourcingPanel::new(
        package.id.clone(),
        favorites,
        &sources,
        false,
        state.locale(),
    );
    page(
        &PackagePage {
            build: crate::views::BuildPane::new(
                &root,
                package.clone(),
                &volumes,
                raise_version,
                false,
                state.locale(),
            ),
            volume_strip: crate::views::VolumeStrip::new(
                package.id.clone(),
                &volumes,
                volume,
                false,
                state.locale(),
            ),
            members: crate::views::MembersTable::new(
                package.id.clone(),
                volume,
                members,
                false,
                state.locale(),
            ),
            chips: crate::views::SourceChips {
                sources: sourcing.sources.clone(),
                oob: false,
            },
            sourcing,
            chrome,
            // Only the second group marks the current value; see `PackagePage::is_default_language`.
            corpus_languages: crate::views::Choice::languages_in(&present, None),
            every_language: crate::views::Choice::languages(None),
            renumber: crate::views::RenumberForm::new(
                package.id.clone(),
                volume,
                package.start_number,
                false,
                state.locale(),
            ),
            package,
        },
        state.locale(),
    )
}

/// `POST /packages/{id}/sources/add`
///
/// Puts one more favorite among the lists a package draws on. **It syncs nothing**: pointing a
/// package at another list is a decision, and what that would do to its songs is the question the
/// button below it asks in counts first.
pub async fn add_package_source(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    sourced(state, id, &body, true).await
}

/// `POST /packages/{id}/sources/remove`
///
/// Takes one list back out. The songs it put there stay until the next sync, which is what the
/// sentence says: removing a source is not removing songs.
pub async fn remove_package_source(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    sourced(state, id, &body, false).await
}

/// Adds or removes one source and answers with the panel and a sentence.
///
/// **One function, because the two differ by a word.** The direction is named rather than toggled —
/// the rule `Filing into a favorite … takes songs out as well` states one page over — and everything
/// after the write is the same: read what the package reads now, redraw the panel, say what changed.
async fn sourced(state: State, id: String, body: &str, member: bool) -> Response {
    let words = crate::words::messages(state.locale());
    let Some(favorite) = Fields::parse(body).parsed::<i64>("favorite") else {
        return MessageFragment::failed(words.msg("said-name-a-list").into_owned());
    };
    let package_id = id.clone();
    let written = state
        .blocking(move |db| {
            let draws_on = db.set_package_source(&package_id, favorite, member)?;
            Ok((draws_on, db.favorites()?, db.package_sources(&package_id)?))
        })
        .await;
    let (draws_on, favorites, chosen) = match written {
        Ok(written) => written,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    // The name is read out of the favorites the panel is about to be drawn from, so a list deleted
    // between the click and the write leaves a sentence with no name rather than a second query.
    let name = favorites
        .iter()
        .find(|node| node.id == favorite)
        .map(|node| node.name.clone())
        .unwrap_or_default();

    // **Asking what is true afterwards, rather than assuming the write did what it was told.** A
    // working list is no package's source, and the statement that refuses one is the only thing that
    // knows — so an add that did not take is the sentence that says why.
    let (ok, said) = if member && !draws_on {
        (
            false,
            words
                .msg_with(
                    "said-source-is-a-working-list",
                    &[("name", name.as_str().into())],
                )
                .into_owned(),
        )
    } else if member {
        (
            true,
            words
                .msg_with("said-source-added", &[("name", name.as_str().into())])
                .into_owned(),
        )
    } else if chosen.is_empty() {
        // Not a smaller version of removing any other one: the package stops being sourced at all
        // and goes back into the selects it had left.
        (true, words.msg("said-sources-cleared").into_owned())
    } else {
        (
            true,
            words
                .msg_with("said-source-removed", &[("name", name.as_str().into())])
                .into_owned(),
        )
    };

    // A refusal keeps the slot beside the button, because it names a remedy somebody has to act on
    // and must not fade; what succeeded is news about something finished, which is a toast.
    if !ok {
        return MessageFragment::failed(said);
    }
    // The panel rides back out of band, because the Sync button inside it names how many lists it
    // would read and this is what changes that number — `package_settings` and the re-flow button,
    // one tab over and for the same reason. The chips over the song list come with it, being the
    // same lists seen from the pane where the rows are.
    crate::views::with_toast(
        &crate::views::SourcingSwap::new(id, favorites, &chosen, state.locale()),
        &crate::views::Toast::good(said),
        state.locale(),
    )
}

/// `POST /packages/{id}/sync`
///
/// **The one write on a package that takes songs out as well as putting them in**, which is why it
/// counts first and asks — the discipline `Acting on a whole filter` states for a set nobody can see
/// in full. The set here is not a filter, but it is the same kind of thing: a union of lists another
/// page away, whose removals are rows that are not on the screen.
///
/// It never happens on its own. A build packages what the package holds.
pub async fn sync_package(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(confirm): Query<ConfirmQuery>,
    body: String,
) -> Response {
    let words = crate::words::messages(state.locale());
    if confirm.confirm.is_some() {
        let package_id = id.clone();
        let now = crate::scan::timestamp();
        // The volume the page is showing, which the button includes. Absent on a package of one
        // volume, where the strip that carries the field is not drawn.
        let volume = Fields::parse(&body)
            .parsed::<u32>("volume")
            .unwrap_or(1)
            .max(1);
        return match state
            .blocking(move |db| {
                let synced = db.sync_package(&package_id, &now)?;
                Ok((
                    synced,
                    db.package_members(&package_id, volume)?,
                    db.package_volumes(&package_id)?,
                ))
            })
            .await
        {
            Err(error) => MessageFragment::failed(error.say(state.locale())),
            Ok((synced, members, volumes)) => {
                let text = sync_says(&synced, state.locale());
                // The table comes back with the sentence. A sync is the one write on this page whose
                // result is not the row in front of the person who pressed it: it adds and removes
                // many at once, and a member list left as it was would contradict the count beside
                // it. The strip rides with it, because a sync is what starts a volume.
                crate::views::with_toasts(
                    &crate::views::MembersTable::new(
                        id.clone(),
                        volume,
                        members,
                        true,
                        state.locale(),
                    ),
                    Some(&crate::views::VolumeStrip::new(
                        id,
                        &volumes,
                        volume,
                        true,
                        state.locale(),
                    )),
                    &[crate::views::Toast::good(text)],
                    state.locale(),
                )
            }
        };
    }

    let package_id = id.clone();
    let planned = state
        .reading(move |db| Ok((db.package(&package_id)?, db.package_sync_plan(&package_id)?)))
        .await;
    let (package, plan) = match planned {
        Ok(planned) => planned,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    if plan.sources.is_empty() {
        return MessageFragment::failed(words.msg("said-no-sources-to-sync").into_owned());
    }
    // A green sentence rather than a question with nothing to answer: a package that already holds
    // its lists is a package doing its job, and a confirmation offering to change nothing is a
    // dialog nobody reads.
    if plan.is_quiet() {
        return crate::views::toast_only(&crate::views::Toast::good(
            words
                .msg_with(
                    "said-nothing-to-sync",
                    &[("count", i64::from(plan.kept).into())],
                )
                .into_owned(),
        ));
    }
    let counted = |key: &str, count: u32| -> String {
        words
            .msg_with(key, &[("count", i64::from(count).into())])
            .into_owned()
    };
    page(
        &crate::views::PackageSyncConfirm {
            package_id: id,
            name: package.name,
            sources: plan
                .sources
                .iter()
                .map(|(_, name, temporary)| crate::views::SourceChip {
                    name: name.clone(),
                    temporary: *temporary,
                })
                .collect(),
            adding: counted("sync-adding", plan.would_add),
            removing: counted("sync-removing", plan.would_remove),
            keeping: counted("sync-keeping", plan.kept),
            // Said only when it is news: what every volume together has no number for starts one.
            new_volumes: (plan.new_volumes > 0)
                .then(|| counted("sync-starts-volumes", plan.new_volumes)),
        },
        state.locale(),
    )
}

/// Refuses an add whose target is sourced from favorites, and says where the songs belong instead.
///
/// **Told rather than silently obeyed.** The write would succeed and the next sync would undo it, so
/// the failure would arrive days later as songs that will not stay in a package. The sentence names
/// the lists, because putting the song in one of those is what the person was trying to do.
async fn sourced_refusal(state: &State, package_id: &str, reply: &ReplyQuery) -> Option<Response> {
    let lookup = package_id.to_owned();
    let sources = match state.reading(move |db| db.package_sources(&lookup)).await {
        Ok(sources) => sources,
        Err(error) => return Some(said(reply, false, error.say(state.locale()))),
    };
    if sources.is_empty() {
        return None;
    }
    let names: Vec<&str> = sources.iter().map(|(_, name, _)| name.as_str()).collect();
    Some(said(
        reply,
        false,
        crate::words::messages(state.locale())
            .msg_with(
                "said-package-is-sourced",
                &[
                    (
                        "count",
                        i64::try_from(names.len()).unwrap_or(i64::MAX).into(),
                    ),
                    ("favorites", names.join(", ").into()),
                ],
            )
            .into_owned(),
    ))
}

/// What a sync says it did.
///
/// [`add_says`]'s rules, in the keyed spelling [`made_says`] uses: clauses rather than one of a dozen
/// sentences, because the facts are independent, and nothing subtracted — each count says why a song
/// went in, stayed or left, and the remedies differ.
///
/// **A sync always reads as a success**, which is where this parts company with `add_says`: an add
/// that added nothing is a batch that did not happen, where a sync of a package that already agrees
/// with its lists is the answer being yes.
fn sync_says(result: &crate::db::Synced, locale: km_locale::Locale) -> String {
    let words = crate::words::messages(locale);
    let mut text = words
        .msg_with(
            "said-synced",
            &[
                ("added", i64::from(result.placed.added).into()),
                ("removed", i64::from(result.removed).into()),
                ("kept", i64::from(result.kept).into()),
            ],
        )
        .into_owned();
    // No clause for songs left out: what every volume together cannot hold starts a volume, so a sync
    // places everything its lists name.
    if result.new_volumes > 0 {
        text.push(' ');
        text.push_str(&words.msg_with(
            "said-sync-new-volumes",
            &[("count", i64::from(result.new_volumes).into())],
        ));
    }
    if result.placed.clashed > 0 {
        text.push(' ');
        text.push_str(&words.msg_with(
            "said-sync-clashed",
            &[("count", i64::from(result.placed.clashed).into())],
        ));
    }
    text
}

/// `POST /packages/add`
///
/// Takes the songs ticked on the browse page, the one named on a detail page, or **everything the
/// browse filter matches**. Every tick arrives as another `song_id=` — the repeated-key shape that
/// made `form.rs` necessary.
///
/// **The ticked scope asks nothing and the filter-wide scope asks first**, which is the one place
/// this crate's bulk actions differ from each other. The three rules in `Acting on a whole filter`
/// bind the filter-wide half exactly as they bind the other four, and the ticked half is outside
/// them: its set is the rows in front of somebody, and this route is also what a song's own page and
/// the Lyrics page post to, neither of which has a filter to count or a question to put.
pub async fn add_to_package(
    AxumState(state): AxumState<State>,
    Query(reply): Query<ReplyQuery>,
    action: BulkAction,
) -> Response {
    let BulkAction {
        query,
        whole_filter,
        ticked,
        confirmed,
        form,
    } = action;
    let Some(package_id) = form.one("package_id").map(ToOwned::to_owned) else {
        return said(&reply, false, "Choose a package first.".to_owned());
    };

    // **Before either scope, because a select is not a guard.** Both pages that post here draw their
    // list when they load, so a tab open since before somebody gave the package a source still offers
    // it — and a song added to a sourced package is one the next sync takes straight back out with
    // nothing to say why. The refusal is here and not in `Db::add_to_package`, the split
    // `version_refusal` draws: `build::import` reaches that method with what a `.kmpkg` carries and
    // has to go on opening one.
    if let Some(refusal) = sourced_refusal(&state, &package_id, &reply).await {
        return refusal;
    }

    if !whole_filter {
        if ticked.is_empty() {
            return said(&reply, false, "Nothing was ticked.".to_owned());
        }
        return added(&state, &reply, package_id, ticked, false).await;
    }

    let filter = query.to_filter();
    if !confirmed {
        // The favorites come along because this fragment's job is making the filter legible, and
        // without them a favorite chip reads `in 7`. The name is what somebody picked out of the
        // select and the only spelling of the package they have seen, so it is what the sentence
        // uses; the id is the `.kmpkg`'s business and says nothing here.
        let counting = filter.clone();
        let named = package_id.clone();
        let counted = state
            .blocking(move |db| {
                Ok((
                    db.count_matching(&counting)?,
                    db.favorites()?,
                    db.package_room(&named)?,
                    db.package(&named)?.name,
                ))
            })
            .await;
        let (count, favorites, room, name) = match counted {
            Ok(four) => four,
            Err(error) => return MessageFragment::failed(error.say(state.locale())),
        };
        if count == 0 {
            return MessageFragment::failed(
                crate::words::messages(state.locale()).msg("said-nothing-matches-filter"),
            );
        }
        if room == 0 {
            return MessageFragment::failed(crate::words::messages(state.locale()).msg_with(
                "said-package-full",
                &[
                    ("name", name.as_str().into()),
                    ("highest", i64::from(km_songcode::MAX_SLOT).into()),
                ],
            ));
        }
        return page(
            &crate::views::PackageAddConfirm {
                subject: crate::words::messages(state.locale())
                    .msg_with("confirm-songs", &[("count", i64::from(count).into())])
                    .into_owned(),
                room_left: crate::words::messages(state.locale())
                    .msg_with("package-add-room-left", &[("room", i64::from(room).into())])
                    .into_owned(),
                count,
                filters: query
                    .active(&favorites, state.locale())
                    .into_iter()
                    .map(|c| c.label)
                    .collect(),
                name,
                room,
                // Handed back so the confirmed write is over the set that was just counted, rather than
                // over whatever the filter bar says by the time the button is clicked.
                query: query.rebuild(0, "", None),
            },
            state.locale(),
        );
    }

    // **The numbers the package has free, not the ceiling**, which is the arithmetic
    // `package_from_filter` states at length: the `ORDER BY` already decides which matches go in, so
    // asking for as many as the package can take is the same answer as reading a quarter of a
    // million ids to use four hundred. One more than fits is what tells a filter that matched
    // exactly the free numbers from one the limit cut.
    let reading = filter.clone();
    let named = package_id.clone();
    let picked = state
        .blocking(move |db| {
            let room = db.package_room(&named)?;
            let mut songs = db.matching_ids(&reading, Some(room + 1))?;
            let matched_more = songs.len() > room as usize;
            songs.truncate(room as usize);
            Ok((songs, matched_more))
        })
        .await;
    let (songs, matched_more) = match picked {
        Ok(pair) => pair,
        Err(error) => return said(&reply, false, error.say(state.locale())),
    };
    added(&state, &reply, package_id, songs, matched_more).await
}

/// The write both scopes end in, and the sentence it answers with.
///
/// One function, so a filter of eleven thousand and a page of forty ticks cannot come to describe
/// the same outcome in two different vocabularies.
async fn added(
    state: &State,
    reply: &ReplyQuery,
    package_id: String,
    songs: Vec<String>,
    matched_more: bool,
) -> Response {
    let now = crate::scan::timestamp();
    match state
        .blocking(move |db| db.add_to_package(&package_id, &songs, &now))
        .await
    {
        Err(error) => said(reply, false, error.say(state.locale())),
        Ok(result) => {
            let (ok, mut text) = add_says(&result);
            if matched_more {
                text.push_str(&format!(
                    " The filter matched more songs than this package has room for, so the first {} \
                     in this order went in.",
                    result.added
                ));
            }
            said(reply, ok, text)
        }
    }
}

/// What an add says it did, and whether it reads as a success.
///
/// **One function for both**, so the wording and the color of the toast cannot disagree: a batch that
/// placed nothing is a failure however politely it is put, and a green toast saying zero is read as
/// nothing having been ticked.
///
/// Built as clauses rather than as one of a dozen sentences, because the facts are independent: a
/// batch can hold songs the package already had, songs it had no number for, *and* a second version
/// of something, and spelling every combination out is how one of them ends up unsaid.
///
/// **Nothing here subtracts.** Each count says why a song did or did not go in, and the remedy
/// differs: a package that already holds the song is doing its job, one whose numbers have run to the
/// end is re-flowed, and one holding every slot can only be split.
fn add_says(result: &crate::db::Added) -> (bool, String) {
    let mut text = format!("Added {}.", result.added);
    if result.already > 0 {
        text.push_str(&format!(
            " {} {} already in it.",
            result.already,
            were(result.already)
        ));
    }
    if result.no_room > 0 {
        if result.full {
            text.push_str(&format!(
                " {} did not fit. A volume holds {} songs and this package's last one is full. Source \
                 the package from favorites and a sync starts the next volume.",
                result.no_room,
                km_songcode::MAX_SLOT
            ));
        } else {
            text.push_str(&format!(
                " {} did not fit. This package's numbers already reach {}, the last one a singer can \
                 dial. Give it a lower first number on its own page and press Re-flow, which moves \
                 every song down and frees the numbers above.",
                result.no_room,
                km_songcode::MAX_SLOT
            ));
        }
    }
    if result.clashed > 0 {
        text.push_str(&format!(
            " {} of them {} another file of a song this package already had — kept, because two \
             takes of one song can be two songs.",
            result.clashed,
            if result.clashed == 1 { "is" } else { "are" }
        ));
    }
    (result.added > 0, text)
}

/// The verb for a count, where the crate's `song(s)` shape cannot help.
///
/// A noun takes a bracketed plural and reads as a form somebody has to fill in; a verb does not, and
/// *1 were already in it* is the sentence that comes of pretending otherwise.
fn were(count: u32) -> &'static str {
    if count == 1 { "was" } else { "were" }
}

/// Turns a package's name into an id.
///
/// An id has to survive being a file name on three platforms and a path inside a zip, so it is the
/// fold rule `km_kmpkg::name_slug` holds — which is also what names the file the machine stores a
/// package under, and one rule spelled once is what keeps those two answers alike.
///
/// **Empty rather than `None`**, because the caller has a sentence to say about a name that leaves
/// nothing, and it names the name. No length bound either: that one belongs to a file name, and an
/// id is checked for being taken before anything is created.
///
/// The *name* keeps every letter it had and is what the tool shows.
fn slug(name: &str) -> String {
    km_kmpkg::name_slug(name).unwrap_or_default()
}

/// Whether a package already has a name that makes this file-name stem.
fn stem_taken(db: &crate::db::Db, stem: &str) -> Result<bool, DbError> {
    Ok(db
        .packages()?
        .iter()
        .any(|package| slug(&package.name) == stem))
}

/// `POST /packages/from-filter`
///
/// Makes a package out of **everything the current filter matches**, rather than out of the ticked
/// rows. The filter is what makes it worth having: turning an artist, a tag or a language into a
/// package would otherwise be a hundred pages of ticking.
///
/// The same two-step shape as [`bulk_language`], and for the same reason — the filter that decides
/// what goes in is fourteen controls further up the page, so the count and the chips are shown next
/// to the button before anything is written.
///
/// **Where the filter comes from is different in the two steps, and deliberately so.** The count
/// reads the bar as it is at the moment of the click, out of the body ([`FilterQuery::from_body`]);
/// the write reads the query string the confirmation handed back, which is the set that was counted.
/// Neither may be swapped for the other: the first because a filter rendered into the page is stale
/// the moment the bar changes, the second because a bar changed between the count and the click
/// would otherwise write a set nobody was shown.
///
/// It **creates** rather than adding to something that exists — `POST /packages/add` is the one that
/// adds — so a name whose id is taken is refused by name rather than merged into.
pub async fn package_from_filter(
    AxumState(state): AxumState<State>,
    action: BulkAction,
) -> Response {
    // This one acts on the whole filter always -- a package *is* the filter -- so `whole_filter` and
    // `ticked` are not read. The counted-set invariant is the same one, and it is `query` that
    // carries it.
    let BulkAction {
        query,
        confirmed,
        form,
        ..
    } = action;
    let name = form.one("name").unwrap_or_default().trim().to_owned();
    if name.is_empty() {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-name-the-package"),
        );
    }
    // **The stem, and never the id.** An id is generated, because `Manifest::problems` refuses one
    // that is not, and a package built out of a name-shaped id is a package that cannot be built. The
    // stem is what can still collide: it names the `.kmpkg` a build writes, so two packages sharing
    // one overwrite each other's file.
    let stem = slug(&name);
    if stem.is_empty() {
        return MessageFragment::failed(crate::words::messages(state.locale()).msg_with(
            "said-name-has-no-file-name",
            &[("name", name.as_str().into())],
        ));
    }

    let filter = query.to_filter();
    // The one list this filter narrows by, when it narrows by nothing else — read before `filter`
    // goes into the closure below, and the only condition under which the package may be sourced.
    let only_favorite = filter.only_this_favorite();
    if !confirmed {
        // The favorites come along because this fragment's job is making the filter legible, and
        // without them a favorite chip reads `in 7` — which says less than no chip at all.
        let taken = stem.clone();
        let counted = state
            .blocking(move |db| {
                Ok((
                    db.count_matching(&filter)?,
                    db.favorites()?,
                    stem_taken(db, &taken)?,
                ))
            })
            .await;
        let (count, favorites, taken) = match counted {
            Ok(triple) => triple,
            Err(error) => return MessageFragment::failed(error.say(state.locale())),
        };
        if count == 0 {
            return MessageFragment::failed(
                crate::words::messages(state.locale()).msg("said-nothing-matches-filter"),
            );
        }
        // Said here rather than after the button, because the file name is derived from the name and
        // two different names can produce one — so the person who has to change it should be told
        // while the box they would change is still in front of them.
        if taken {
            return MessageFragment::failed(
                crate::words::messages(state.locale())
                    .msg_with("said-package-name-taken", &[("name", name.as_str().into())]),
            );
        }
        let (subject, _) = confirm_words(state.locale(), count, "confirm-songs", "confirm-set");
        // The favorites are already in hand for the chips, so naming the one the bar narrows by
        // costs no query — and a list the filter names but that has since gone leaves no box, which
        // is the answer `without_missing_favorite` gives a saved filter in the same position.
        //
        // **A working list leaves no box either**, by the rule that offers one to no package: a
        // volume built from *decide about these later* ships songs nobody has decided about. The
        // statement behind `Db::set_package_source` is what actually holds it; this is what keeps a
        // curator from being offered something that would then not happen.
        let source = only_favorite
            .and_then(|wanted| favorites.iter().find(|node| node.id == wanted))
            .filter(|node| !node.temporary)
            .map(|node| crate::views::SourceChip {
                name: node.name.clone(),
                temporary: node.temporary,
            });
        return page(
            &crate::views::PackageFromFilterConfirm {
                subject,
                filters: query
                    .active(&favorites, state.locale())
                    .into_iter()
                    .map(|c| c.label)
                    .collect(),
                source,
                name,
                // Handed back so the confirmed write is over the set that was just counted, rather than
                // over whatever the filter bar says by the time the button is clicked.
                query: query.rebuild(0, "", None),
            },
            state.locale(),
        );
    }

    let id = km_kmpkg::PackageMeta::new_id();
    let row = package_row(&form, &id);
    let now = crate::scan::timestamp();
    let taken = stem;
    // **Both halves, because either alone is a lie.** The box is only drawn when the filter is that
    // one list, and it is only obeyed when the filter still is: a page open since before somebody
    // narrowed the bar further would otherwise source a package from a list it does not hold.
    let source = only_favorite.filter(|_| form.has("source"));
    // Worded here rather than inside the closure, which runs on a blocking thread with no request
    // and so no language in reach.
    let taken_says = crate::words::messages(state.locale())
        .msg_with("said-package-name-taken", &[("name", name.as_str().into())])
        .into_owned();
    let created = state
        .blocking(move |db| {
            // Checked again, and not only in the confirmation above: between the count and the click
            // is however long somebody left the page open.
            if stem_taken(db, &taken)? {
                return Err(DbError::Rejected(taken_says));
            }
            // Created first, so the row exists before hundreds of thousands of ids are read out of the
            // corpus and there is something for them to go into.
            db.create_package(&row, &now)?;
            // **The free numbers, not the ceiling.** The package was created a line ago, so it has
            // no members to skip past and its first number is what decides how many it can take —
            // asking for `MAX_SLOT` of them where it starts at 500 reads 999 ids to place 500.
            // Without any limit this read every match -- every id in the owner's own corpus --
            // to use a few hundred of them.
            let free = u32::from(km_songcode::MAX_SLOT).saturating_sub(row.start_number) + 1;
            // One more than fits, which is what tells a filter that matched exactly the free
            // numbers from one the limit cut. The extra id is dropped before anything is written.
            let mut songs = db.matching_ids(&filter, Some(free + 1))?;
            let matched_more = songs.len() > free as usize;
            songs.truncate(free as usize);
            // A package made from a filter cannot clash with itself, because the filter that fed it
            // collapses a group to one song before it is read.
            let added = db.add_to_package(&row.id, &songs, &now)?;
            // **After the songs, and it syncs nothing.** The package already holds what the list
            // holds, because the filter that filled it was that list; writing the source is saying
            // so, and the first press of Sync is what will answer for anything starred since.
            //
            // **The answer decides the sentence rather than a value read here.** A working list is
            // no package's source, and the statement is what refuses one — so a box that reached
            // this from a page opened before somebody set the list aside creates an ordinary package
            // and says so, instead of claiming a source the package does not have.
            let sourced = match source {
                Some(favorite) => db.set_package_source(&row.id, favorite, true)?,
                None => false,
            };
            Ok((added, matched_more, sourced))
        })
        .await;

    match created {
        Ok((result, matched_more, sourced)) => MessageFragment::ok(made_says(
            &name,
            &result,
            matched_more,
            sourced,
            state.locale(),
        )),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// What making a package out of a filter says it did.
///
/// **A filter that matched more songs than a package holds is said out loud**, because the count in
/// the sentence is otherwise read as what the filter found: a broad filter over a corpus answers with
/// the cap and looks like a filter that happened to match exactly that many.
///
/// The `no_room` clause is the one that should never be reached, since the caller asks for the
/// numbers the package has free. It is written anyway, because a count that quietly does not add up
/// is worse than a sentence nobody sees.
fn made_says(
    name: &str,
    result: &crate::db::Added,
    matched_more: bool,
    sourced: bool,
    locale: km_locale::Locale,
) -> String {
    let words = crate::words::messages(locale);
    let added = i64::from(result.added);
    let mut text = words
        .msg_with(
            "said-package-made",
            &[("package", name.into()), ("count", added.into())],
        )
        .into_owned();
    // Said out loud, because being sourced is what takes the package out of every *add to a package*
    // select — a change to a control two pages away, from a box ticked here.
    if sourced {
        text.push(' ');
        text.push_str(&words.msg("said-package-made-sourced"));
    }
    if matched_more {
        text.push(' ');
        text.push_str(&words.msg_with("said-package-took-the-first", &[("count", added.into())]));
    }
    if result.no_room > 0 {
        text.push(' ');
        text.push_str(&words.msg_with(
            "said-package-no-room",
            &[("count", i64::from(result.no_room).into())],
        ));
    }
    text
}

/// `POST /packages/{id}/number`
pub async fn package_number(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    body: String,
) -> Response {
    let form = Fields::parse(&body);
    let Some(number) = form.parsed::<u32>("number") else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-not-a-number"),
        );
    };
    let song_id = form.one("song_id").unwrap_or_default().to_owned();
    match state
        .blocking(move |db| db.set_package_number(&id, &song_id, number))
        .await
    {
        Ok(()) => crate::views::toast_only(&crate::views::Toast::good(
            crate::words::messages(state.locale())
                .msg_with("said-now-number", &[("number", i64::from(number).into())])
                .into_owned(),
        )),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /packages/{id}/remove`
pub async fn package_remove(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
    body: String,
) -> Response {
    let song_id = Fields::parse(&body)
        .one("song_id")
        .unwrap_or_default()
        .to_owned();
    let package_id = id.clone();
    match state
        .blocking(move |db| db.remove_from_package(&package_id, &song_id))
        .await
    {
        Ok(()) => {
            let said = crate::words::messages(state.locale())
                .msg("said-removed")
                .into_owned();
            said_with_members(&state, id, volume.number(), true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// A package and one of its volumes as one select value, `<volume>#<package id>`.
///
/// **One field, because a select sends one value**, and a number is only unique inside a volume. The
/// volume comes first so the split is at the first `#`, whatever the id holds.
pub fn replace_slot(package_id: &str, volume: u32) -> String {
    format!("{volume}#{package_id}")
}

/// Reads [`replace_slot`] back.
fn parse_replace_slot(slot: &str) -> Option<(String, u32)> {
    let (volume, package_id) = slot.split_once('#')?;
    let volume = volume.parse().ok().filter(|volume| *volume > 0)?;
    (!package_id.is_empty()).then(|| (package_id.to_owned(), volume))
}

/// `POST /packages/replace` — a song takes the number another song has in a package.
///
/// Asks first, naming the song that leaves the number, and writes on `?confirm=1`. Refusals and the
/// outcome are worded here out of [`crate::db::ReplaceRefusal`] and [`crate::db::Replacement`],
/// because the database has no language.
pub async fn package_replace(
    AxumState(state): AxumState<State>,
    Query(confirm): Query<ConfirmQuery>,
    body: String,
) -> Response {
    let words = crate::words::messages(state.locale());
    let form = Fields::parse(&body);
    let Some((package_id, volume)) = form.one("slot").and_then(parse_replace_slot) else {
        return MessageFragment::failed(words.msg("said-choose-a-package"));
    };
    let Some(number) = form.parsed::<u32>("number") else {
        return MessageFragment::failed(words.msg("said-not-a-number"));
    };
    let song_id = form.one("song_id").unwrap_or_default().to_owned();
    let confirmed = confirm.confirm.is_some();

    let (package, new_song) = (package_id.clone(), song_id.clone());
    let now = crate::scan::timestamp();
    let outcome = state
        .blocking(move |db| {
            if confirmed {
                db.replace_in_package(&package, volume, number, &new_song, &now)
            } else {
                db.replacement_at(&package, volume, number, &new_song)
            }
        })
        .await;
    let replacement = match outcome {
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
        Ok(Err(refusal)) => {
            let number = i64::from(number);
            return MessageFragment::failed(match refusal {
                crate::db::ReplaceRefusal::Empty(name) => words.msg_with(
                    "said-replace-empty",
                    &[("number", number.into()), ("package", name.as_str().into())],
                ),
                crate::db::ReplaceRefusal::SameSong => {
                    words.msg_with("said-replace-same-song", &[("number", number.into())])
                }
                crate::db::ReplaceRefusal::AlreadyIn(name, held) => words.msg_with(
                    "said-replace-already-in",
                    &[
                        ("number", i64::from(held).into()),
                        ("package", name.as_str().into()),
                    ],
                ),
                crate::db::ReplaceRefusal::Merged => words.msg("said-replace-merged"),
            });
        }
        Ok(Ok(replacement)) => replacement,
    };

    let named = |(_, title, artist): &(String, String, Option<String>)| match artist {
        Some(artist) if !artist.is_empty() => format!("{title} — {artist}"),
        _ => title.clone(),
    };
    let values = [
        ("number", i64::from(replacement.number).into()),
        ("package", replacement.volume_name.as_str().into()),
        ("old", named(&replacement.old).into()),
        ("new", named(&replacement.new).into()),
    ];
    let favorites = (!replacement.favorites.is_empty()).then(|| {
        (
            i64::try_from(replacement.favorites.len()).unwrap_or(i64::MAX),
            replacement.favorites.join(", "),
        )
    });

    if confirmed {
        let mut text = words.msg_with("said-replaced", &values).into_owned();
        if let Some((_, names)) = favorites {
            text.push(' ');
            text.push_str(
                &words.msg_with("said-replaced-in-favorites", &[("favorites", names.into())]),
            );
        }
        return MessageFragment::ok(text);
    }
    page(
        &crate::views::PackageReplaceConfirm {
            slot: replace_slot(&package_id, volume),
            number,
            song_id,
            question: words.msg_with("confirm-replace", &values).into_owned(),
            favorites: favorites.map(|(count, names)| {
                words
                    .msg_with(
                        "confirm-replace-favorites",
                        &[
                            ("count", count.into()),
                            ("favorites", names.into()),
                            ("new", named(&replacement.new).into()),
                        ],
                    )
                    .into_owned()
            }),
        },
        state.locale(),
    )
}

/// A write on a package's member table: its sentence, and the table it has just changed.
///
/// **The table rides back rather than a line asking for a reload**, which is what a sync already
/// does and what this tool asks for nowhere else. The row somebody removed goes, and the numbers a
/// re-flow moved are the numbers on the screen.
async fn said_with_members(
    state: &State,
    package_id: String,
    volume: u32,
    ok: bool,
    text: String,
) -> Response {
    if !ok {
        return MessageFragment::failed(text);
    }
    let lookup = package_id.clone();
    match state
        .reading(move |db| db.package_members(&lookup, volume))
        .await
    {
        Ok(members) => crate::views::with_toast(
            &crate::views::MembersTable::new(package_id, volume, members, true, state.locale()),
            &crate::views::Toast::good(text),
            state.locale(),
        ),
        // The write happened; only the redraw did not, and its own sentence would replace the one
        // the write earned.
        Err(_) => crate::views::toast_only(&crate::views::Toast::good(text)),
    }
}

/// `POST /packages/{id}/renumber`
pub async fn package_renumber(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
) -> Response {
    let package_id = id.clone();
    let volume = volume.number();
    match state
        .blocking(move |db| db.renumber_package(&package_id, volume))
        .await
    {
        Ok(count) => {
            let said = crate::words::messages(state.locale())
                .msg_with(
                    "said-renumbered",
                    &[("count", i64::try_from(count).unwrap_or(i64::MAX).into())],
                )
                .into_owned();
            said_with_members(&state, id, volume, true, said).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /packages/{id}/build/all?volume=` — every volume holding a song, each under its default name.
///
/// The Build form's own fields: the folder, and the two tick boxes. The file name box is not read,
/// because it names one volume's file and this writes several. `?volume=` is only the tab the answer
/// is drawn into.
pub async fn build_all(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
    body: String,
) -> Response {
    if state.build_running() {
        return MessageFragment::failed(
            "A build is already running. Wait for it to finish — the bar on the page it was started \
             from is following it.",
        );
    }
    let fields = Fields::parse(&body);
    let raise = fields.has("raise_version");
    let write_listing = fields.has("write_listing");
    let folder = fields
        .one("folder")
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
        .map(PathBuf::from);
    let package = id.clone();
    if let Err(error) = state
        .blocking(move |db| db.set_raise_version(&package, raise))
        .await
    {
        return failure(error, state.locale());
    }
    if state
        .start_build_all(id.clone(), folder, write_listing)
        .is_none()
    {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-no-folder-open"),
        );
    }
    build_fragment(&state, id, volume.number(), true)
}

/// `POST /packages/{id}/build`
///
/// Starts a build on a thread of its own and answers with the first frame of the progress fragment,
/// which then polls itself. Doing the build inside this request holds the database's one mutex
/// throughout, so every other page and every poll queues behind it, and the answer arrives after
/// however many minutes as a line of text.
pub async fn build_package(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
    body: String,
) -> Response {
    let volume = volume.number();
    // Refused rather than queued, and one build at a time is deliberate: two threads writing
    // packages would both record what they did, and this page has one place to show a bar.
    if state.build_running() {
        return MessageFragment::failed(
            "A build is already running. Wait for it to finish — the bar on the page it was started \
             from is following it.",
        );
    }
    let fields = Fields::parse(&body);
    // One form posts this route and the box is always in it, so an absent key is an unticked box —
    // the whole of how HTML says `false`.
    //
    // Read before the out path, because the default out path is built from the version this decides.
    let raise = fields.has("raise_version");
    // Ticked on the page, so an absent key here is a caller that is not the page — `km-pack build`'s
    // own default, and the honest answer for a request that never saw the box.
    let write_listing = fields.has("write_listing");
    let out = match chosen_path(&fields, root_of(&state).ok().as_deref()) {
        Some(out) => out,
        // The form always sends the file name, so this is for a caller that is not the page. It costs a
        // query the page's own path does not, which is the right way round: the default has to be
        // the same one the page would have shown, and only the row can say what that is.
        None => {
            let lookup = id.clone();
            let root = match root_of(&state) {
                Ok(root) => root,
                Err(error) => return failure(error, state.locale()),
            };
            match state
                .blocking(move |db| db.package_volume(&lookup, volume))
                .await
            {
                Ok(package) => crate::build::default_out_path(&root, &package, raise),
                Err(error) => return failure(error, state.locale()),
            }
        }
    };
    let package = id.clone();
    if let Err(error) = state
        .blocking(move |db| db.set_raise_version(&package, raise))
        .await
    {
        return failure(error, state.locale());
    }
    if state
        .start_build(id.clone(), volume, out, write_listing)
        .is_none()
    {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-no-folder-open"),
        );
    }
    build_fragment(&state, id, volume, true)
}

/// Where a Build tab form asks for a file to go: a folder and a file name, or a whole path.
///
/// **The folder is one field for both files and the boxes hold only names**, because a full path in
/// a box puts the part somebody is looking for past its right edge. A blank folder is the corpus's
/// data folder, which is where both defaults point. `out` is the whole path a caller that is not the
/// page may send, and it wins. `None` when neither is given, which each route answers with its own
/// default.
fn chosen_path(fields: &Fields, root: Option<&std::path::Path>) -> Option<PathBuf> {
    if let Some(out) = fields
        .one("out")
        .map(str::trim)
        .filter(|out| !out.is_empty())
    {
        return Some(PathBuf::from(out));
    }
    let file = fields
        .one("file")
        .map(str::trim)
        .filter(|file| !file.is_empty())?;
    let folder = match fields
        .one("folder")
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
    {
        Some(folder) => PathBuf::from(folder),
        None => crate::db::data_dir(root?),
    };
    Some(folder.join(file))
}

/// `GET /packages/{id}/build/out` — the file name the next build writes, for the box on the form.
///
/// **Asked again when the raise-the-version box moves**, because the version is in the name. The box
/// is rendered once when the page is drawn, and unticking the raise afterwards would otherwise leave
/// it naming a file that never exists — which is exactly what the label beside it already goes out
/// of its way to avoid.
///
/// The raise state comes from the form rather than from the database: what the reader is looking at
/// is the answer, and the stored preference is not written until the build itself runs.
pub async fn build_out(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let root = match root_of(&state) {
        Ok(root) => root,
        Err(error) => return failure(error, state.locale()),
    };
    let lookup = id.clone();
    let volume = query
        .get("volume")
        .and_then(|volume| volume.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let package = match state
        .reading(move |db| db.package_volume(&lookup, volume))
        .await
    {
        Ok(package) => package,
        Err(error) => return failure(error, state.locale()),
    };
    // An absent key is an unticked box, the whole of how HTML says `false` — the same reading
    // `build_package` gives the same field.
    let raise = query.contains_key("raise_version");
    page(
        &crate::views::BuildOutBox {
            file: crate::build::file_name(&crate::build::default_out_path(&root, &package, raise)),
        },
        state.locale(),
    )
}

/// `GET /packages/{id}/build/progress` — polled once a second while a build is going.
///
/// `poll=1` is what the running fragment's own poll sends, and it is what may toast a finished build.
/// The tab's first load sends none, so opening the page does not announce a build that ended before.
pub async fn build_progress(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    build_fragment(&state, id, volume.number(), query.contains_key("poll"))
}

/// The progress fragment, for both the POST that starts a build and every poll after it.
///
/// One function so the frame that starts a run and the frame that reports it cannot disagree — the
/// arrangement `scan_progress` already uses.
fn build_fragment(state: &State, id: String, volume: u32, announce: bool) -> Response {
    let progress = state.build_progress();
    // A build of some *other* package, or of another volume of this one, is not this page's business
    // to draw. Showing its numbers here would put a moving bar on a file that is not building. A run
    // over every volume is every volume's business.
    let mine = progress
        .filter(|view| view.package_id == id && (view.volume == volume || view.volume == 0));
    let running = state.build_running() && mine.is_some();

    // One sentence per file, and whether it is a refusal.
    let said = mine
        .as_ref()
        .filter(|view| view.finished)
        .map(|view| build_sentences(view, state.locale()))
        .unwrap_or_default();

    // **What was written is a toast, and what was refused stays on the tab.** A written file is news
    // that needs no answer, which is what the corner is for. A refusal lists the songs to go and fix,
    // and a toast that leaves after eight seconds would take the list with it.
    //
    // **Toasted only by the frame that saw the build end**: the POST that started it, or the poll.
    // The tab asks for this fragment again whenever it is drawn, and a toast on every visit would
    // announce last week's build each time somebody opened the page.
    let toasts: Vec<crate::views::Toast> = if announce {
        said.iter()
            .filter(|(_, refused)| !refused)
            .map(|(text, _)| crate::views::Toast::good(text.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let refusals: Vec<&str> = said
        .iter()
        .filter(|(_, refused)| *refused)
        .map(|(text, _)| text.as_str())
        .collect();
    let message = (!refusals.is_empty()).then(|| refusals.join("\n\n"));

    // **The last frame of a build that wrote something brings the install button back with it.**
    // That form is a sibling of `#build-progress` and nothing was re-rendering it, so its `disabled`
    // went on saying what `built_at` had said when the page was drawn. The build's own report is the
    // authority here rather than a second read of the database. After a run over every volume, that
    // report is the one for the volume this tab shows.
    let built = match mine.as_ref().filter(|view| view.finished) {
        Some(view) if view.volume == 0 => view
            .volumes_built
            .iter()
            .any(|(built, report)| *built == volume && !build_message(report, state.locale()).1),
        Some(_) => said.iter().any(|(_, refused)| !refused),
        None => false,
    };
    let fragment = crate::views::BuildProgressFragment::new(
        id.clone(),
        volume,
        mine,
        running,
        message,
        !refusals.is_empty(),
        state.locale(),
    );
    let install = built.then_some(crate::views::InstallForm {
        package_id: id,
        volume,
        built: true,
        oob: true,
    });
    crate::views::with_toasts(&fragment, install.as_ref(), &toasts, state.locale())
}

/// What a finished build says, one sentence per file it wrote or refused, each marked refused or not.
///
/// A run over every volume says one per volume, named by its number; a build of one volume says one.
/// An error or a stop before anything was written is a refusal of its own.
fn build_sentences(
    view: &crate::build::BuildProgressView,
    locale: km_locale::Locale,
) -> Vec<(String, bool)> {
    let words = crate::words::messages(locale);
    let mut said: Vec<(String, bool)> = if view.volume == 0 {
        view.volumes_built
            .iter()
            .map(|(number, report)| {
                let (text, refused) = build_message(report, locale);
                let named = words.msg_with(
                    "said-build-volume",
                    &[("number", i64::from(*number).into())],
                );
                (format!("{named} {text}"), refused)
            })
            .collect()
    } else {
        view.report
            .iter()
            .map(|report| build_message(report, locale))
            .collect()
    };
    if let Some(error) = &view.error {
        said.push((error.clone(), true));
    } else if said.is_empty() {
        said.push((words.msg("said-build-stopped").into_owned(), true));
    }
    said
}

/// What to say about a finished build, and whether it is a refusal.
///
/// Lifted out of the old synchronous handler unchanged, so a backgrounded build says exactly what an
/// in-request one said. The order of the arms is the order of the reasons: nothing readable, then a
/// manifest that will not validate, then the language gate, then success.
fn build_message(report: &crate::build::BuildReport, locale: km_locale::Locale) -> (String, bool) {
    let words = crate::words::messages(locale);
    if report.is_empty() {
        let mut text = words.msg("said-build-nothing-readable").into_owned();
        text.push_str(&skipped_detail(&report.skipped, locale));
        return (text, true);
    }
    if !report.problems.is_empty() {
        let mut text = words.msg("said-build-manifest-problems").into_owned();
        text.push_str("\n  ");
        text.push_str(&report.problems.join("\n  "));
        return (text, true);
    }
    if !report.unlanguaged.is_empty() {
        let mut text = words
            .msg_with(
                "said-build-unlanguaged",
                &[(
                    "count",
                    i64::try_from(report.unlanguaged.len())
                        .unwrap_or(i64::MAX)
                        .into(),
                )],
            )
            .into_owned();
        for (number, title) in report.unlanguaged.iter().take(10) {
            text.push_str(&format!("\n  {number}: {title}"));
        }
        if report.unlanguaged.len() > 10 {
            text.push_str("\n  ");
            text.push_str(
                &words.msg_with(
                    "said-and-more",
                    &[(
                        "count",
                        i64::try_from(report.unlanguaged.len() - 10)
                            .unwrap_or(i64::MAX)
                            .into(),
                    )],
                ),
            );
        }
        // Naming both ways out, because on a real corpus this will be most of a package the first
        // time somebody hits it, and either answer is one action rather than N. The package-level
        // default is the honest one for a volume that really is all one language; the bulk set is
        // the one that leaves the corpus classified for every later package.
        text.push_str("\n\n");
        text.push_str(&words.msg("said-build-unlanguaged-ways-out"));
        return (text, true);
    }
    let mut text = words
        .msg_with(
            "said-build-written",
            &[
                // The name and not the path: the folder is the one on the Build tab, and a path in a
                // toast is mostly the part nobody is reading for.
                (
                    "file",
                    crate::build::file_name(std::path::Path::new(&report.out_path)).into(),
                ),
                ("version", report.version.as_str().into()),
                (
                    "count",
                    i64::try_from(report.written).unwrap_or(i64::MAX).into(),
                ),
            ],
        )
        .into_owned();
    // Named because it is the second file the build wrote, and a package handed over without it is
    // a package handed over without the page saying what is in it.
    text.push_str(&build_detail(report, locale));
    text.push('.');
    if !report.listing_path.is_empty() {
        text.push(' ');
        text.push_str(&words.msg_with(
            "said-build-listing-written",
            &[(
                "file",
                crate::build::file_name(std::path::Path::new(&report.listing_path)).into(),
            )],
        ));
    }
    text.push_str(&skipped_detail(&report.skipped, locale));
    (text, false)
}

/// What a build spent its time on, when there is something to say.
///
/// A re-encode is minutes and a copy is seconds, so "5 songs written" after twenty minutes reads as
/// though the tool hung and then lied about it. Silent when the package is all MIDI, which is the
/// case where none of this says anything.
fn build_detail(report: &crate::build::BuildReport, locale: km_locale::Locale) -> String {
    let words = crate::words::messages(locale);
    let mut parts = Vec::new();
    let counted = |key: &str, count: usize| {
        words
            .msg_with(
                key,
                &[("count", i64::try_from(count).unwrap_or(i64::MAX).into())],
            )
            .into_owned()
    };
    if report.videos_transcoded > 0 {
        parts.push(counted("said-build-re-encoded", report.videos_transcoded));
    }
    if report.videos_copied > 0 {
        parts.push(counted("said-build-copied", report.videos_copied));
    }
    if report.cdg_written > 0 {
        parts.push(counted("said-build-cdg-pairs", report.cdg_written));
    }
    if report.ultrastar_written > 0 {
        parts.push(counted("said-build-ultrastar", report.ultrastar_written));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(" ({})", parts.join(", "))
}

/// `POST /packages/{id}/spec`
///
/// Writes the description this package would be built from. Synchronous, unlike the build beside it,
/// because it is a database read and a file write — the expensive half of a build is everything that
/// happens *after* the description exists.
pub async fn write_package_spec(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
    body: String,
) -> Response {
    let volume = volume.number();
    let out = match chosen_path(&Fields::parse(&body), root_of(&state).ok().as_deref()) {
        Some(out) => out,
        // The row and not only the id, because the default is named for the package the way the
        // `.kmpkg` beside it is.
        None => {
            let root = match root_of(&state) {
                Ok(root) => root,
                Err(error) => return failure(error, state.locale()),
            };
            let wanted = id.clone();
            match state
                .blocking(move |db| db.package_volume(&wanted, volume))
                .await
            {
                Ok(package) => crate::build::default_spec_path(&root, &package),
                Err(error) => return failure(error, state.locale()),
            }
        }
    };
    let shown = crate::build::file_name(&out);
    match state
        .blocking(move |db| crate::build::write_spec(db, &id, volume, &out))
        .await
    {
        Ok(songs) => crate::views::toast_only(&crate::views::Toast::good(
            crate::words::messages(state.locale())
                .msg_with(
                    "said-spec-written",
                    &[
                        ("file", shown.as_str().into()),
                        ("count", i64::try_from(songs).unwrap_or(i64::MAX).into()),
                    ],
                )
                .into_owned(),
        )),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

fn skipped_detail(skipped: &[(u32, String)], locale: km_locale::Locale) -> String {
    let heading = crate::words::messages(locale).msg_with(
        "said-build-left-out",
        &[(
            "count",
            i64::try_from(skipped.len()).unwrap_or(i64::MAX).into(),
        )],
    );
    first_ten(
        &heading,
        skipped
            .iter()
            .map(|(number, reason)| format!("{number}: {reason}")),
        locale,
    )
}

/// A heading, the first ten of a list, and a tally for the rest — or nothing at all when the list is
/// empty.
///
/// Three callers now, which is what promoted it: the build's skipped songs, the import's unmatched
/// entries, and the restore's report. Ten because a message slot is read at a glance and a hundred
/// lines in it is a wall nobody reads rather than a hundred lines of information.
fn first_ten(
    heading: &str,
    lines: impl IntoIterator<Item = String>,
    locale: km_locale::Locale,
) -> String {
    let lines: Vec<String> = lines.into_iter().collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!("\n{heading}");
    for line in lines.iter().take(10) {
        out.push_str(&format!("\n  {line}"));
    }
    if lines.len() > 10 {
        out.push_str("\n  ");
        out.push_str(&crate::words::messages(locale).msg_with(
            "said-and-more",
            &[(
                "count",
                i64::try_from(lines.len() - 10).unwrap_or(i64::MAX).into(),
            )],
        ));
    }
    out
}

/// `POST /packages/{id}/install`
pub async fn install_package(
    AxumState(state): AxumState<State>,
    UrlPath(id): UrlPath<String>,
    Query(volume): Query<VolumeQuery>,
) -> Response {
    let lookup = id.clone();
    let volume = volume.number();
    let package = match state
        .blocking(move |db| db.package_volume(&lookup, volume))
        .await
    {
        Ok(package) => package,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };
    let Some(out) = package.out_path else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-build-it-first"),
        );
    };
    let path = PathBuf::from(&out);
    let absolute = crate::model::tidy(&path);

    // **Asked here rather than left to the machine, because the machine's answer is about the wrong
    // disk.** `tidy` falls through to the path unchanged when the file is not there, so a package
    // whose `.kmpkg` was moved, deleted or written by an older layout sent a path that resolves
    // nowhere — and the sentence that came back was `No such file or directory (os error 2)`, which
    // on this screen reads as though the build had produced nothing.
    if !absolute.is_file() {
        return MessageFragment::failed(crate::words::messages(state.locale()).msg_with(
            "said-build-gone",
            &[("file", absolute.display().to_string().as_str().into())],
        ));
    }

    let client = state.app_client().await;
    // **Both branches are admin now**, so a token is wanted either way. A password this computer was
    // told to remember is a standing instruction to sign in, spent at the moment one is needed
    // rather than at startup — see `State::sign_in_if_remembered`. Silent on failure: what follows
    // is the machine's refusal and the sentence saying how to sign in, which is the better answer.
    state.sign_in_if_remembered(&client).await;

    // **A path or the bytes, decided by the address**, which is the split `play` already makes and
    // for the same reason: the machine opens a path on *its own* filesystem, so naming one is only
    // meaningful where that filesystem is this one. Pointing `--machine` at the appliance and
    // pressing this sends it a `D:\…` it cannot open, and the os error 2 that comes back names
    // neither the machine nor the reason.
    //
    // **Both branches end in the same sentence.** The two routes answer different shapes — one a
    // set of fields for a program, one a line of prose for a page — and pasting either into the
    // message as raw JSON prints `Installed into http://…:8177. {"report":"installed
    // \"favtest1\" · 155 songs"}` at somebody. The wording is `km-api`'s, in one place, and which
    // branch a machine's address picked is invisible in what it says.
    let sent = if client.is_loopback() {
        // The path branch is the one that knows about duplicates: `POST /packages` reports them and
        // the upload route has nowhere to put them. Said here rather than dropped, because two
        // copies of one recording under two numbers is a catalog defect the person who just built
        // the package is the only one placed to fix.
        client.install_package(&absolute).await.map(|report| {
            format!(
                "{}{}",
                report.sentence(),
                duplicate_detail(&report, state.locale())
            )
        })
    } else {
        client.upload_package(&absolute).await
    };

    match sent {
        Ok(said) => MessageFragment::ok(format!("Installed into {}. {said}", client.base())),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// The songs that arrived already in the catalog under another number, or nothing at all.
///
/// [`first_ten`]'s fourth caller and the same reasoning: a message slot is read at a glance, and a
/// package that duplicates a hundred songs would otherwise fill the screen with the least
/// surprising half of what it did.
fn duplicate_detail(report: &km_api::dto::InstallReportDto, locale: km_locale::Locale) -> String {
    first_ten(
        &crate::words::messages(locale).msg_with(
            "said-install-already-in-catalog",
            &[(
                "count",
                i64::try_from(report.duplicate_content.len())
                    .unwrap_or(i64::MAX)
                    .into(),
            )],
        ),
        report
            .duplicate_content
            .iter()
            .map(|dupe| format!("{}: already here as {}", dupe.number, dupe.existing_number)),
        locale,
    )
}

/// `POST /packages/import`
pub async fn import_package(AxumState(state): AxumState<State>, body: String) -> Response {
    let raw = PathBuf::from(Fields::parse(&body).one("path").unwrap_or_default());
    // A relative path is taken against the folder a package built here lands in, rather than against
    // whatever directory the tool happens to have been started in. See `resolve_read_path`.
    let path = match root_of(&state) {
        Ok(root) => resolve_read_path(&root, raw),
        Err(error) => return failure(error, state.locale()),
    };

    match state
        .blocking(move |db| crate::build::import(db, &path))
        .await
    {
        Ok(report) if report.unmatched.is_empty() && report.unreadable_language.is_empty() => {
            let said = crate::words::messages(state.locale()).msg_with(
                "said-imported-all",
                &[
                    ("package", report.package_id.as_str().into()),
                    (
                        "count",
                        i64::try_from(report.matched).unwrap_or(i64::MAX).into(),
                    ),
                ],
            );
            said_with_packages(&state, true, said.into_owned()).await
        }
        Ok(report) => {
            let mut text = crate::words::messages(state.locale())
                .msg_with(
                    "said-imported",
                    &[
                        ("package", report.package_id.as_str().into()),
                        (
                            "count",
                            i64::try_from(report.matched).unwrap_or(i64::MAX).into(),
                        ),
                    ],
                )
                .into_owned();
            text.push_str(&first_ten(
                &crate::words::messages(state.locale()).msg_with(
                    "said-import-unmatched",
                    &[(
                        "count",
                        i64::try_from(report.unmatched.len())
                            .unwrap_or(i64::MAX)
                            .into(),
                    )],
                ),
                report
                    .unmatched
                    .iter()
                    .map(|(number, title)| format!("{number}: {title}")),
                state.locale(),
            ));
            // A package written before language was a code, or by a later build than this one.
            // Reported rather than dropped in silence: a correction that did not come back is
            // exactly the thing nobody notices.
            text.push_str(&first_ten(
                &crate::words::messages(state.locale()).msg_with(
                    "said-import-unreadable-language",
                    &[(
                        "count",
                        i64::try_from(report.unreadable_language.len())
                            .unwrap_or(i64::MAX)
                            .into(),
                    )],
                ),
                report
                    .unreadable_language
                    .iter()
                    .map(|(number, raw)| format!("{number}: {raw}")),
                state.locale(),
            ));
            said_with_packages(&state, true, text).await
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

// -- scanning -------------------------------------------------------------------------------

/// The scan's progress, with its counts worded for the page asking.
///
/// Every route that draws the panel goes through here, so the seven numbers are put into a sentence
/// in one place rather than three.
fn progress_now(state: &State) -> crate::scan::ProgressView {
    let mut progress = state.scan_progress();
    progress.say_counts(state.locale());
    progress
}

/// `GET /scan`
pub async fn scan_page(AxumState(state): AxumState<State>) -> Response {
    let chrome = match chrome(&state, "scan").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    let loaded = state
        .reading(|db| {
            Ok((
                db.setting("last_scan")?,
                db.failure_tally()?,
                db.dismissed_tally()?,
                db.stale_analysis_count()?,
            ))
        })
        .await;
    let (last_scan, failures, dismissed, stale) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => return failure(error, state.locale()),
    };
    let (failures, dismissed, dismissed_count) = failure_rows(failures, dismissed, state.locale());
    // Said only when there are any: a folder whose every song this build decided has nothing to
    // report, and a nought beside a sentence about being out of date is a line that makes somebody
    // look for work that is not there.
    let stale = (stale > 0).then(|| {
        crate::words::messages(state.locale())
            .msg_with("scan-stale-analysis", &[("count", stale.into())])
            .into_owned()
    });

    page(
        &ScanPage {
            chrome,
            progress: progress_now(&state),
            running: state.scan_running(),
            last_scan,
            failures,
            dismissed,
            dismissed_count,
            stale,
        },
        state.locale(),
    )
}

/// The failures panel's rows and its summary line, worded together.
///
/// Both the Scan page and the fragment that redraws the panel need them, and both would otherwise
/// spell the same three lookups.
fn failure_rows(
    failures: Vec<crate::db::FailureTally>,
    dismissed: Vec<crate::db::FailureTally>,
    locale: km_locale::Locale,
) -> (
    Vec<crate::views::FailureRow>,
    Vec<crate::views::FailureRow>,
    String,
) {
    let worded = |tallies: Vec<crate::db::FailureTally>| {
        tallies
            .into_iter()
            .map(|tally| crate::views::FailureRow::new(tally, locale))
            .collect::<Vec<_>>()
    };
    let count = i64::try_from(dismissed.len()).unwrap_or(i64::MAX);
    let summary = crate::words::messages(locale)
        .msg_with("failures-reasons-removed", &[("count", count.into())])
        .into_owned();
    (worded(failures), worded(dismissed), summary)
}

/// `POST /scan/failures/remove`
///
/// Accepts every file failing this way now. The reason itself is not dismissed — see
/// `Db::dismiss_failures` for why the difference is the whole point.
pub async fn remove_failures(AxumState(state): AxumState<State>, body: String) -> Response {
    let status = Fields::parse(&body)
        .one("status")
        .unwrap_or_default()
        .to_owned();
    let now = crate::scan::timestamp();
    match state
        .blocking(move |db| db.dismiss_failures(&status, &now))
        .await
    {
        Ok(_) => failures_panel(&state).await,
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /scan/failures/restore`
pub async fn restore_failures(AxumState(state): AxumState<State>, body: String) -> Response {
    let status = Fields::parse(&body)
        .one("status")
        .unwrap_or_default()
        .to_owned();
    match state.blocking(move |db| db.restore_failures(&status)).await {
        Ok(_) => failures_panel(&state).await,
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// The failures panel, redrawn from the database after a removal or a restore.
///
/// One function for both, so the two cannot come to disagree about what the panel looks like — the
/// arrangement `build_fragment` already uses for the two ways a build reports itself.
async fn failures_panel(state: &State) -> Response {
    match state
        .reading(|db| Ok((db.failure_tally()?, db.dismissed_tally()?)))
        .await
    {
        Ok((failures, dismissed)) => {
            let locale = state.locale();
            let (failures, dismissed, dismissed_count) = failure_rows(failures, dismissed, locale);
            page(
                &crate::views::FailuresPanel {
                    failures,
                    dismissed,
                    dismissed_count,
                },
                locale,
            )
        }
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /scan`
pub async fn start_scan(AxumState(state): AxumState<State>, body: String) -> Response {
    // The "re-analyze everything" button carries `force`; the plain one carries nothing. Presence is
    // the whole signal, which is how every checkbox and named button works.
    let force = Fields::parse(&body).has("force");
    if state.scan_running() {
        return page(
            &ProgressFragment {
                progress: progress_now(&state),
                running: true,
            },
            state.locale(),
        );
    }

    let Some(progress) = state.start_scan(ScanOptions {
        force,
        ..ScanOptions::default()
    }) else {
        return failure(DbError::NoWorkspace, state.locale());
    };

    page(
        &ProgressFragment {
            progress: progress.snapshot(),
            running: true,
        },
        state.locale(),
    )
}

/// `POST /scan/stop`
///
/// Asks the run to stop and answers with the panel at once, which then says *stopping* until the run
/// has written what it read. A press with no run going draws the panel as it stands.
pub async fn stop_scan(AxumState(state): AxumState<State>) -> Response {
    state.ask_scan_to_stop();
    page(
        &ProgressFragment {
            progress: progress_now(&state),
            running: state.scan_running(),
        },
        state.locale(),
    )
}

/// `GET /scan/progress`
pub async fn scan_progress(AxumState(state): AxumState<State>) -> Response {
    page(
        &ProgressFragment {
            progress: progress_now(&state),
            running: state.scan_running(),
        },
        state.locale(),
    )
}

// -- settings -------------------------------------------------------------------------------

/// `GET /settings`
pub async fn settings(AxumState(state): AxumState<State>) -> Response {
    let chrome = match chrome(&state, "settings").await {
        Ok(chrome) => chrome,
        Err(error) => return failure(error, state.locale()),
    };
    let hand_set_songs = match state.reading(|db| db.hand_set_count()).await {
        Ok(count) => count,
        Err(error) => return failure(error, state.locale()),
    };
    // Stamped when the page is drawn rather than when the button is pressed, which is a difference
    // of seconds in an editable box whose value is what the file is called.
    let (default_backup_out, newest_backup) = match root_of(&state) {
        Ok(root) => (
            crate::backup::default_path(&root).display().to_string(),
            crate::backup::newest(&root).unwrap_or_default(),
        ),
        Err(error) => return failure(error, state.locale()),
    };

    let client = state.app_client().await;
    let mut debug_enabled = false;
    let (status, reachable) = match client.discover().await {
        Ok(info) => {
            state.machine_answered(&info.id, &info.name).await;
            debug_enabled = info.debug_enabled;
            let words = crate::words::messages(state.locale());
            let mut said = words
                .msg_with(
                    "said-machine-reached-at",
                    &[
                        ("name", info.name.as_str().into()),
                        ("url", client.base().into()),
                    ],
                )
                .into_owned();
            if let Some(count) = info.song_count {
                said.push(' ');
                said.push_str(
                    &words.msg_with("said-machine-holds", &[("count", i64::from(count).into())]),
                );
            }
            (said, true)
        }
        // **What the six-hour clock is for here, and it is a sentence rather than a move.**
        // `STALE_AFTER` tunes `choose`'s rule 2 for a program that knows whether its address is
        // answering, and this one holds no connection — so what `last_connected` buys is the state a
        // corpus carried to another house is in, where the address in the `.kmbuild` is now
        // somebody's printer. Without it that is a failure with no explanation.
        Err(error) => {
            let stale = state.chosen_machine().await.is_some_and(|known| {
                km_api::discover::known::is_stale(&known, std::time::SystemTime::now())
            });
            let said = match stale {
                true => {
                    let mut said = error.say(state.locale());
                    said.push(' ');
                    said.push_str(
                        &crate::words::messages(state.locale()).msg("said-machine-stale"),
                    );
                    said
                }
                false => error.say(state.locale()),
            };
            (said, false)
        }
    };

    // **Asked after the `/discover` above**, which is what fills the id in: a machine that has never
    // answered has none, and `crate::passwords` has nothing to key a password by until it does.
    let machine_id = state.machine_id().await;

    page(
        &SettingsPage {
            machine: client.base().to_owned(),
            status,
            reachable,
            on_this_box: client.is_loopback(),
            signed_in: state.signed_in(),
            can_remember: machine_id.is_some(),
            remembered: machine_id.as_deref().is_some_and(|id| state.remembers(id)),
            owner_only: crate::passwords::owner_only(),
            debug_enabled,
            // A fresh page has nothing to report; the three routes below fill this in when they answer
            // with the fragment on its own.
            message: String::new(),
            ok: true,
            default_backup_out,
            newest_backup,
            default_tags: state.settings().default_tags.join(", "),
            settings_file: crate::settings::Settings::file()
                .map(|file| file.display().to_string())
                .unwrap_or_default(),
            locales: crate::views::LocaleChoice::all(state.locale()),
            hand_set_said: crate::words::messages(state.locale())
                .msg_with(
                    "settings-hand-set",
                    &[("count", i64::from(hand_set_songs).into())],
                )
                .into_owned(),
            failed_said: (chrome.counts.failed > 0).then(|| {
                crate::words::messages(state.locale())
                    .msg_with(
                        "settings-did-not-parse",
                        &[("count", i64::from(chrome.counts.failed).into())],
                    )
                    .into_owned()
            }),
            favorites_said: crate::words::messages(state.locale())
                .msg_with(
                    "confirm-songs",
                    &[("count", i64::from(chrome.counts.favorites).into())],
                )
                .into_owned(),
            // Last, because the three sentences above read its counts.
            chrome,
        },
        state.locale(),
    )
}

/// `POST /settings/tags` — the suggested tag vocabulary.
///
/// **Its own route rather than a field on `POST /settings`**, because the two write to different
/// places for a reason worth keeping visible: that one saves a machine address into *this corpus's*
/// database, and this one saves a preference into *this person's* config directory. One form
/// writing both would make a corpus-scoped page quietly edit a user-scoped file.
///
/// Answers with what was stored rather than what was typed, which is where somebody learns that
/// `Rock & Roll` became `rock-roll`.
pub async fn save_default_tags(AxumState(state): AxumState<State>, body: String) -> Response {
    let raw = Fields::parse(&body)
        .one("default_tags")
        .unwrap_or("")
        .to_owned();
    let stored = state.set_default_tags(&raw);
    if stored.is_empty() {
        return MessageFragment::ok(
            crate::words::messages(state.locale()).msg("said-saved-no-tags"),
        );
    }
    MessageFragment::ok(format!("Saved: {}.", stored.join(", ")))
}

/// `POST /settings/locale` — which language these pages are drawn in.
///
/// **Answers with a refresh rather than a fragment**, and it is the one control in this tool that
/// does. Every word on the document changes — the nav, the counts strip, `<html lang>` and the
/// sentences `static/ui.js` reads off `<body>` — so there is no target a swap could aim at. The
/// redraw is the confirmation, and it arrives in the language just chosen, which is what somebody
/// who picked the wrong one needs in order to find their way back.
///
/// A tag this build has no catalog for is ignored rather than refused. It can only come from a
/// hand-made request, and the page an error would replace is the page somebody is reading.
pub async fn save_locale(AxumState(state): AxumState<State>, body: String) -> Response {
    let Some(chosen) = Fields::parse(&body)
        .one("locale")
        .and_then(km_locale::Locale::parse)
    else {
        return StatusCode::NO_CONTENT.into_response();
    };
    state.set_locale(chosen);
    (StatusCode::OK, [("hx-refresh", "true")]).into_response()
}

// **There is no `DISCOVER_TIMEOUT` any more, and its argument is what removed it.** It was three
// seconds, and waiting the *whole* of it was required rather than optional: a browse that returned
// at the first answer would list one machine in a house that has two. A registry that has been
// listening since the tool opened satisfies that by construction — it has heard from both — so the
// button now costs nothing and is more complete than the wait ever was.

/// `POST /settings/discover`
///
/// Lists the machines advertising themselves. **Sets nothing** — see [`DiscoveredFragment`].
pub async fn discover_machines(AxumState(state): AxumState<State>) -> Response {
    // **The list is already there, so the page answers at once.** Opening a daemon on
    // `spawn_blocking`, waiting out a timeout and shutting it down again costs three seconds a
    // press — and the argument for waiting the *whole* timeout is satisfied by construction here: a
    // registry that has been listening since the tool started has heard from both machines in a
    // house with two, where a browse that stopped at the first answer would not.
    // Pressing the button also puts a fresh query on the wire, for somebody who has just switched a
    // machine on and pressed again.
    state.look_again();

    // `app_client` follows the chosen machine to a new address if it has moved, so the row marked
    // *current* is the machine somebody picked rather than a stale address beside it.
    let current = state.app_client().await.base().to_owned();
    page(
        &DiscoveredFragment {
            machines: state
                .machines_seen()
                .into_iter()
                .map(|sighting| DiscoveredMachine {
                    current: sighting.url == current,
                    name: sighting.name,
                    url: sighting.url,
                })
                .collect(),
        },
        state.locale(),
    )
}

/// Draws the password panel as it stands, with a sentence about what just happened.
///
/// The three routes below all end here, so the state they leave the page in is worked out in one
/// place rather than three — including the two facts a handler cannot know without asking: whether
/// the machine has an identity to remember a password under, and whether it is in debugging mode.
///
/// **The `/discover` is skipped when nothing is signed in**, because the only thing it is read for
/// is the Debugging switch, which lives in the signed-in half of the fragment. A failed sign-in
/// should answer at once with the reason rather than after a second round trip.
async fn machine_access(state: &State, message: String, ok: bool) -> Response {
    let signed_in = state.signed_in();
    let debug_enabled = match signed_in {
        true => state
            .app_client()
            .await
            .discover()
            .await
            .is_ok_and(|info| info.debug_enabled),
        false => false,
    };
    let machine_id = state.machine_id().await;
    page(
        &MachineAccessFragment {
            signed_in,
            can_remember: machine_id.is_some(),
            remembered: machine_id.as_deref().is_some_and(|id| state.remembers(id)),
            owner_only: crate::passwords::owner_only(),
            debug_enabled,
            message,
            ok,
        },
        state.locale(),
    )
}

/// `POST /settings/login` — the machine's admin password, exchanged for a token this run holds.
///
/// **The token never touches a disk, and the password does only if the box is ticked.** That split
/// is `Where a key somebody typed into a page lives` in `docs/decisions/repository.md`, which
/// `km-admin` already follows for provider keys, and it is why the two are kept by two different
/// things: `State::token` for the run, [`crate::passwords`] for the file.
///
/// **A blank box means the password this computer has saved.** The token is bought at the moment one
/// is wanted, so a launch with a password saved opens signed out — and a box that had to be filled in
/// there would be asking for something this program is already holding. A pass with neither a typed
/// password nor a saved one is turned away here.
///
/// **The tick rides with a typed password and says nothing about one that was not.** The checkbox is
/// a statement about what this computer should be remembering, and it is spent on the password in
/// hand: the panel draws no box where nothing is being typed, so reading its absence as *forget*
/// would delete a credential as a side effect of using it. *Forget it* is the control that means
/// that, and it sits beside the sentence saying there is something to forget — in both halves of the
/// panel now, which is what lets this rule be the narrow one.
pub async fn log_in_to_machine(AxumState(state): AxumState<State>, body: String) -> Response {
    let fields = Fields::parse(&body);
    let typed = fields.one("password").unwrap_or_default().trim().to_owned();
    // A checkbox is in the body only when it is ticked, which is the whole of how HTML says `false`.
    let remember = fields.one("remember").is_some();

    // **Only under an id**, which a machine has once something has answered at its address. There is
    // nothing to key a password by before that, and a row shared by every anonymous machine would be
    // a password handed to whichever answered next.
    let machine_id = state.machine_id().await;
    let saved = machine_id
        .as_deref()
        .and_then(|id| state.remembered_password(id));
    let password = match typed.is_empty() {
        true => saved.unwrap_or_default(),
        false => typed.clone(),
    };
    if password.is_empty() {
        return machine_access(
            &state,
            crate::words::messages(state.locale())
                .msg("said-type-the-password")
                .into_owned(),
            false,
        )
        .await;
    }

    let client = state.app_client().await;
    if let Err(error) = client.log_in(&password).await {
        return machine_access(&state, error.say(state.locale()), false).await;
    }

    // Each branch names its own key, rather than a key travelling out of the match as a variable:
    // `rust_keys` finds what this program says by reading the literal beside each lookup, and a
    // sentence it cannot see is one `no_message_is_left_unused` deletes.
    let words = crate::words::messages(state.locale());
    let said = match (typed.is_empty(), &machine_id) {
        (true, _) => words.msg("said-signed-in-with-saved"),
        (false, Some(id)) => {
            state.remember_password(id, remember.then_some(password.as_str()));
            match remember {
                true => words.msg("said-signed-in-remembered"),
                false => words.msg("said-signed-in-not-written"),
            }
        }
        (false, None) => words.msg("said-signed-in-not-written"),
    };
    machine_access(&state, said.into_owned(), true).await
}

/// `POST /settings/logout` — drops the token and deletes anything remembered for this machine.
///
/// **One button for both**, because two would be a distinction nobody wants to make: somebody
/// pressing "Forget it" beside a line saying the password is remembered means the password, not a
/// token they cannot see.
pub async fn log_out_of_machine(AxumState(state): AxumState<State>) -> Response {
    state.app_client().await.log_out();
    state.forget_token();
    if let Some(id) = state.machine_id().await {
        state.remember_password(&id, None);
    }
    machine_access(
        &state,
        crate::words::messages(state.locale())
            .msg("said-signed-out")
            .into_owned(),
        true,
    )
    .await
}

/// `POST /settings/debugging` — the switch that mounts the machine's two play routes.
///
/// **This is the control `explain_uploads` has been naming.** A machine ships with debugging off, so
/// the first test-play against one that is not this computer is refused — and the refusal has always
/// said the quickest way through is a button on this panel, which until now did not exist.
///
/// **It takes effect at the machine's next restart, and the sentence has to say so.** `km-api` mounts
/// the two debug routes at router-construction time, so that a machine with debugging off answers a
/// 404 — genuinely not there — rather than carrying a disabled handler; `put_debug` over there makes
/// the same point about its own response. So `/discover` goes on reporting the *running* mode, which
/// is what this button's label reads, and pressing it does not flip that label. A message claiming
/// the machine "will now take a song sent to it" would be contradicted by the very next test-play.
/// The machine's own `/admin/` page says "It takes effect at the next restart"; this is that sentence
/// in this tool's voice.
pub async fn set_debugging(AxumState(state): AxumState<State>, body: String) -> Response {
    let enabled = Fields::parse(&body)
        .one("enabled")
        .is_some_and(|v| v == "1");
    let client = state.app_client().await;
    state.sign_in_if_remembered(&client).await;
    match client.set_debugging(enabled).await {
        Ok(()) => {
            let said = crate::words::messages(state.locale()).msg(match enabled {
                true => "said-debugging-on",
                false => "said-debugging-off",
            });
            machine_access(&state, said.into_owned(), true).await
        }
        Err(error) => machine_access(&state, error.say(state.locale()), false).await,
    }
}

/// `POST /settings`
pub async fn save_settings(AxumState(state): AxumState<State>, body: String) -> Response {
    let url = Fields::parse(&body)
        .one("machine")
        .unwrap_or(DEFAULT_APP_URL)
        .to_owned();
    let stored = url.clone();
    // **The address `--machine` already named is not a new choice**, so saving the form on it keeps
    // the pin, the token and the record as they are. Any other address unpins the run and is saved
    // like any choice.
    let pinned_here = state
        .pinned_machine()
        .is_some_and(|pinned| pinned.url == url.trim());
    if !pinned_here {
        state.unpin_machine();
        // **Choosing a machine signs out of the last one.** A token is one machine's, and sending it
        // to another is a 401 carrying a confusing sentence rather than the prompt to sign in that
        // the person needs. This is the *choosing* half of the rule `State::forget_token` states; a
        // machine that merely moved keeps its token, because it is the same machine.
        state.forget_token();
        // **The identity goes with the address, and that is the guard the follow needs.** An address
        // somebody typed has not said what it is yet, and inheriting the previous machine's id would
        // let the follow drag this address straight back off to wherever that machine is — the
        // opposite of what typing one means. `chosen::save` writes a record with no id for exactly
        // that; it is filled in below, from a `/discover` that answered *at this address*.
        if let Err(error) = state
            .blocking(move |db| crate::chosen::save(db, &stored))
            .await
        {
            return MessageFragment::failed(error.say(state.locale()));
        }
    }

    match Client::new(&url).discover().await {
        Ok(info) => {
            state.machine_answered(&info.id, &info.name).await;
            MessageFragment::ok(crate::words::messages(state.locale()).msg_with(
                "said-machine-reached",
                &[
                    ("name", info.name.as_str().into()),
                    ("url", url.as_str().into()),
                ],
            ))
        }
        Err(error) => MessageFragment::ok(crate::words::messages(state.locale()).msg_with(
            "said-machine-saved-no-answer",
            &[
                ("url", url.as_str().into()),
                ("why", error.say(state.locale()).as_str().into()),
            ],
        )),
    }
}

/// `POST /settings/backup`
///
/// Writes everything hand-typed in this folder to a JSON file.
///
/// Synchronous, like the description beside it on the package page and unlike a build: it is a
/// handful of indexed reads over the rows somebody has touched and one file write. The predicate is
/// what keeps it that way — see `Db::hand_set_songs`.
pub async fn write_backup(AxumState(state): AxumState<State>, body: String) -> Response {
    let out = match Fields::parse(&body).one("out") {
        Some(out) => PathBuf::from(out),
        None => match root_of(&state) {
            Ok(root) => crate::backup::default_path(&root),
            Err(error) => return failure(error, state.locale()),
        },
    };
    let shown = out.display().to_string();
    match state
        .blocking(move |db| crate::backup::write(db, &out))
        .await
    {
        Ok(counts) => MessageFragment::ok(crate::words::messages(state.locale()).msg_with(
            "said-backup-written",
            &[
                ("file", shown.as_str().into()),
                (
                    "songs",
                    i64::try_from(counts.songs).unwrap_or(i64::MAX).into(),
                ),
                (
                    "favorites",
                    i64::try_from(counts.favorites).unwrap_or(i64::MAX).into(),
                ),
            ],
        )),
        Err(error) => MessageFragment::failed(error.say(state.locale())),
    }
}

/// `POST /settings/restore`
///
/// Reads a backup back into this folder's database.
///
/// The `overwrite` checkbox is read by presence, the way every checkbox here is: an unticked box
/// sends nothing. Without it the restore only fills what is blank, which is the direction that
/// cannot take work away.
pub async fn restore_backup(AxumState(state): AxumState<State>, body: String) -> Response {
    let fields = Fields::parse(&body);
    let policy = if fields.has("overwrite") {
        crate::backup::Policy::Overwrite
    } else {
        crate::backup::Policy::FillBlanks
    };
    let Some(raw) = fields.one("path").map(PathBuf::from) else {
        return MessageFragment::failed(
            crate::words::messages(state.locale()).msg("said-name-the-backup"),
        );
    };
    // A relative path is taken against the folder a backup written here lands in, rather than against
    // whatever directory the tool happens to have been started in. See `resolve_read_path`.
    let path = match root_of(&state) {
        Ok(root) => resolve_read_path(&root, raw),
        Err(error) => return failure(error, state.locale()),
    };

    let report = state
        .blocking(move |db| {
            let backup = crate::backup::Backup::read(&path)?;
            crate::backup::restore(db, &backup, policy)
        })
        .await;
    let report = match report {
        Ok(report) => report,
        Err(error) => return MessageFragment::failed(error.say(state.locale())),
    };

    let words = crate::words::messages(state.locale());
    let mut text = words
        .msg_with(
            "said-restored",
            &[
                (
                    "songs",
                    i64::try_from(report.songs_applied)
                        .unwrap_or(i64::MAX)
                        .into(),
                ),
                (
                    "filed",
                    i64::try_from(report.memberships_applied)
                        .unwrap_or(i64::MAX)
                        .into(),
                ),
                (
                    "favorites",
                    i64::try_from(report.favorites_created)
                        .unwrap_or(i64::MAX)
                        .into(),
                ),
                (
                    "merges",
                    i64::try_from(report.merges_applied)
                        .unwrap_or(i64::MAX)
                        .into(),
                ),
            ],
        )
        .into_owned();
    if let Some(format) = report.from_a_later_format {
        text.push('\n');
        text.push_str(&words.msg_with(
            "said-restored-later-format",
            &[("format", i64::from(format).into())],
        ));
    }
    text.push_str(&first_ten(
        &words.msg_with(
            "said-restore-unmatched",
            &[(
                "count",
                i64::try_from(report.songs_unmatched.len())
                    .unwrap_or(i64::MAX)
                    .into(),
            )],
        ),
        report
            .songs_unmatched
            .iter()
            .map(|(_, name)| name.to_owned()),
        state.locale(),
    ));
    text.push_str(&first_ten(
        &words.msg_with(
            "said-restore-rejected",
            &[(
                "count",
                i64::try_from(report.rejected.len())
                    .unwrap_or(i64::MAX)
                    .into(),
            )],
        ),
        report.rejected.iter().cloned(),
        state.locale(),
    ));
    MessageFragment::ok(text)
}

// -- shared ---------------------------------------------------------------------------------

/// The curated folder, or the error that turns into a redirect.
///
/// Every caller sits behind the middleware that redirects when nothing is open, so in practice this
/// always succeeds. It is fallible anyway because the folder can be closed between that check and
/// this call, and a handler that unwrapped would turn a rare race into a crash — which on a desktop
/// build means a window that dies with nowhere for the panic to be read.
fn root_of(state: &State) -> Result<PathBuf, DbError> {
    state.root().ok_or(DbError::NoWorkspace)
}

/// Turns a path somebody typed into a file to read, for the two routes that take one.
///
/// An absolute path is theirs and is used as given. A relative one is taken against the corpus's
/// data folder, which is where the thing they are naming was written by default — not against
/// whatever directory the tool happened to be started in, which on a double-click is `C:\` or `/`.
/// The three defaults write into [`crate::db::data_dir`] unconditionally, so that is the one place a
/// relative name can mean.
fn resolve_read_path(root: &std::path::Path, raw: PathBuf) -> PathBuf {
    if raw.is_absolute() {
        return raw;
    }
    crate::db::data_dir(root).join(&raw)
}

/// Loads the chrome every full page needs.
async fn chrome(state: &State, tab: &'static str) -> Result<Chrome, DbError> {
    let counts = state.reading(|db| db.counts()).await?;
    Ok(Chrome::new(
        tab,
        state.locale(),
        state
            .root()
            .map(|root| root.display().to_string())
            .unwrap_or_default(),
        counts,
        state.is_windowed(),
        // Two settings, no network. See `State::machine_shown`: the header is drawn on every page,
        // and a machine that is switched off is the normal state here.
        state.machine_shown().await,
        state.songs_filter(),
    ))
}

/// Turns a database failure into a page rather than a blank response.
///
/// [`DbError::NoWorkspace`] is the one that is not a page at all. It can now reach any handler — the
/// folder can be closed between the middleware's check and the query — and rendering "no folder is
/// open" as a 500 would leave somebody staring at an error on a page with no way out of it. So it
/// becomes the same redirect the middleware issues, in the same two flavors: `HX-Redirect` for htmx,
/// which understands it as "navigate", and a 302 for anything else. Sent without knowing which kind
/// of request this was, an htmx swap would put the whole Open page inside a table cell.
///
/// **Everything else stays a real error status, and that is the opposite of what [`MessageFragment::
/// failed`] does.** The rule there — answer 200, because htmx will not swap a failure and a button
/// that visibly does nothing is worse than a red line — is right where a refusal has a fragment to
/// come back as. Here it has none. A `/songs/rows` failure answered 200 would be swapped into
/// `#rows`, and the error text would replace the rows that are still perfectly good: the
/// price of being told would be losing what you were reading. So the status is honest, htmx leaves
/// the page alone, and `static/ui.js` puts the reason on the screen as a toast. Do not "fix" this to
/// a 200 — the silent failure that motivated the toast was fixed in the browser, on purpose.
fn failure(error: DbError, locale: km_locale::Locale) -> Response {
    if matches!(error, DbError::NoWorkspace) {
        return (
            [("hx-redirect", crate::server::OPEN_PATH)],
            axum::response::Redirect::to(crate::server::OPEN_PATH),
        )
            .into_response();
    }
    let status = match error {
        DbError::NotFound(_) => StatusCode::NOT_FOUND,
        // `Rejected` is this crate's word for "the caller asked for something the data will not
        // allow" — a blank favorite name, a package id already taken, a folder holding two
        // databases. Those are the person's input, not the machine's fault, and they used to fall
        // through to a 500 beside a corrupt SQLite file. The page looked the same either way
        // because `ui.js` toasts anything non-2xx; what it cost was the log, where the one line
        // worth finding during a real fault sat among a hundred blank-name refusals.
        DbError::Rejected(_) => StatusCode::BAD_REQUEST,
        // Neither the person's input nor a fault, so neither of the two above: the database is
        // being written to and this write may have it in a moment. 503 is the code that says come
        // back, and it keeps a busy moment during a scan out of the log lines that mean something
        // is broken.
        DbError::Busy => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    // The body is what `static/ui.js` puts on the screen as a toast, so it is worded; the log keeps
    // the `Display` sentence, which is English.
    tracing::debug!(%error, "a request failed");
    (status, error.say(locale)).into_response()
}

/// Reads a song's bytes off disk, by way of its best surviving file.
async fn song_bytes(state: &State, id: &str) -> Result<(PathBuf, Vec<u8>), String> {
    let lookup = id.to_owned();
    let path = state
        .reading(move |db| db.best_file(&lookup))
        .await
        .map(|(_, path)| path)
        .map_err(|error| error.say(state.locale()))?;

    let reading = path.clone();
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&reading))
        .await
        .map_err(|error| format!("the worker thread died: {error}"))?
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((path, bytes))
}

/// Parses a song, optionally forcing an encoding, and reports what is pinned for it.
async fn load_song(
    state: &State,
    id: &str,
    encoding: Option<String>,
) -> Result<(Song, Option<String>), String> {
    let lookup = id.to_owned();
    let pinned = state
        .reading(move |db| db.song(&lookup).map(|song| song.lyric_encoding))
        .await
        .map_err(|error| error.say(state.locale()))?;

    let (_, bytes) = song_bytes(state, id).await?;
    let declared = encoding.or_else(|| pinned.clone());
    let options = ParseOptions {
        declared_encoding: declared,
        inference: None,
    };
    let song = tokio::task::spawn_blocking(move || Song::parse(&bytes, &options))
        .await
        .map_err(|error| format!("the worker thread died: {error}"))?
        .map_err(|error| format!("could not parse the file: {error}"))?;
    Ok((song, pinned))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An add names the reason a song did not go in, because the reasons have different remedies.
    ///
    /// The sentence is built from counts and never from a subtraction, which is the whole of what
    /// this pins: a shortfall reported as *already in it* sends somebody to look at a package that
    /// is doing exactly what it should.
    #[test]
    fn an_add_names_the_reason_a_song_did_not_go_in() {
        let all_in = add_says(&crate::db::Added {
            added: 3,
            ..Default::default()
        });
        assert_eq!(all_in, (true, "Added 3.".to_owned()));

        let one_there = add_says(&crate::db::Added {
            added: 1,
            already: 1,
            ..Default::default()
        })
        .1;
        assert!(
            one_there.contains("1 was already in it."),
            "a verb has no bracketed plural: {one_there}"
        );
        let two_there = add_says(&crate::db::Added {
            added: 1,
            already: 2,
            ..Default::default()
        })
        .1;
        assert!(two_there.contains("2 were already in it."), "{two_there}");

        let no_room = add_says(&crate::db::Added {
            added: 1,
            no_room: 2,
            ..Default::default()
        })
        .1;
        assert!(no_room.contains("2 did not fit."), "{no_room}");
        assert!(
            no_room.contains("Re-flow"),
            "numbers that ran to the end are re-flowed: {no_room}"
        );
        assert!(
            !no_room.contains("already in it"),
            "a song nobody could number was never in it: {no_room}"
        );

        let full = add_says(&crate::db::Added {
            no_room: 1,
            full: true,
            ..Default::default()
        })
        .1;
        assert!(full.contains("is full."), "{full}");
        assert!(
            !full.contains("Re-flow"),
            "re-flowing a package of every slot frees nothing: {full}"
        );

        // Independent clauses: a batch can hold songs the package had and a second version of one.
        let both = add_says(&crate::db::Added {
            added: 2,
            already: 1,
            clashed: 1,
            ..Default::default()
        })
        .1;
        assert!(both.contains("1 was already in it."), "{both}");
        assert!(both.contains("1 of them is another file"), "{both}");
    }

    /// A batch that placed nothing reads as a failure, whatever the sentence says.
    ///
    /// A green toast reporting zero is read as nothing having been ticked, which is a true sentence
    /// about the wrong subject.
    #[test]
    fn a_batch_that_placed_nothing_reads_as_a_failure() {
        assert!(
            !add_says(&crate::db::Added {
                no_room: 2,
                ..Default::default()
            })
            .0
        );
        assert!(
            !add_says(&crate::db::Added {
                already: 2,
                ..Default::default()
            })
            .0
        );
        assert!(
            add_says(&crate::db::Added {
                added: 1,
                no_room: 1,
                ..Default::default()
            })
            .0
        );
    }

    /// A package made from a filter says when the filter matched more than it could hold.
    #[test]
    fn a_package_made_from_a_filter_says_when_more_matched_than_fits() {
        let cut = made_says(
            "brasil",
            &crate::db::Added {
                added: 999,
                ..Default::default()
            },
            true,
            false,
            km_locale::Locale::English,
        );
        assert!(cut.contains("with 999 songs."), "{cut}");
        assert!(
            cut.contains("matched more songs than a package holds"),
            "the count is otherwise read as what the filter found: {cut}"
        );

        let whole = made_says(
            "brasil",
            &crate::db::Added {
                added: 1,
                ..Default::default()
            },
            false,
            false,
            km_locale::Locale::English,
        );
        assert!(whole.contains("with 1 song."), "{whole}");
        assert!(!whole.contains("matched more"), "{whole}");
    }

    /// A first number a song could not carry is refused, and one it could is not.
    #[test]
    fn the_first_number_is_refused_when_no_song_could_carry_it() {
        let refused = |value: &str| {
            start_number_refusal(
                &Fields::parse(&format!("start_number={value}")),
                km_locale::Locale::English,
            )
            .is_some()
        };
        assert!(!refused("1"));
        assert!(!refused(&km_songcode::MAX_SLOT.to_string()));
        assert!(refused("0"));
        assert!(refused(&(u32::from(km_songcode::MAX_SLOT) + 1).to_string()));
        assert!(refused("1001"));
        // A form that does not ask is left alone: making a package from a filter has no such box.
        assert!(
            start_number_refusal(&Fields::parse("name=Brasil"), km_locale::Locale::English)
                .is_none()
        );
    }

    /// The number boxes offer only numbers a package can use.
    ///
    /// A box whose default its own handler will not take is what put a package's songs at the last
    /// slot and left room for one more, so the guard is over the markup rather than over a path
    /// through it.
    #[test]
    fn the_number_boxes_offer_only_numbers_a_package_can_use() {
        let packages = include_str!("../templates/packages.html");
        let package = include_str!("../templates/package.html");
        for (name, markup) in [("packages.html", packages), ("package.html", package)] {
            assert!(
                !markup.contains("999999"),
                "{name} types a ceiling the code owns"
            );
            assert!(
                !markup.contains(r#"max="99"#),
                "{name} still spells a ceiling out"
            );
        }
        assert!(
            packages.contains(r#"min="1" max="{{ km_songcode::MAX_SLOT }}" value="1""#),
            "the create form offers a first number a package can start at"
        );
    }

    /// The re-flow form is out of band only when it is the copy replacing one already on the page.
    #[test]
    fn the_reflow_form_swaps_out_of_band_only_when_asked() {
        use askama::Template;
        let english = km_locale::Locale::English;
        let inline = crate::views::RenumberForm::new("vol1".to_owned(), 1, 1, false, english);
        let swapped = crate::views::RenumberForm::new("vol1".to_owned(), 1, 12, true, english);
        assert!(!inline.render().expect("render").contains("hx-swap-oob"));
        let swapped = swapped.render().expect("render");
        assert!(swapped.contains(r#"hx-swap-oob="true""#));
        assert!(
            swapped.contains("Re-flow every number from 12"),
            "the button names the number it will use: {swapped}"
        );
    }

    /// A relative path means the data folder, because that is where the three defaults write.
    #[test]
    fn a_relative_path_is_read_from_the_data_folder() {
        let corpus = crate::testing::Scratch::new("relative-read-path");
        let root = &corpus.0;
        std::fs::create_dir_all(crate::db::data_dir(root)).expect("the data folder");

        // A file of the same name in the root is not consulted.
        std::fs::write(root.join("backup.kmbackup.json"), "{}").expect("write the root copy");
        assert_eq!(
            resolve_read_path(root, PathBuf::from("backup.kmbackup.json")),
            crate::db::data_dir(root).join("backup.kmbackup.json")
        );

        // A name matching nothing reports the data folder, which is where to go looking for it.
        assert_eq!(
            resolve_read_path(root, PathBuf::from("nope.kmbackup.json")),
            crate::db::data_dir(root).join("nope.kmbackup.json")
        );

        // An absolute path is the caller's own and is never rewritten.
        let elsewhere = root.join("somewhere").join("theirs.kmpkg");
        assert_eq!(resolve_read_path(root, elsewhere.clone()), elsewhere);
    }

    fn query() -> FilterQuery {
        FilterQuery {
            q: "jobim".to_owned(),
            artist: "Tom Jobim".to_owned(),
            suitability: "8-10".to_owned(),
            initial: "C".to_owned(),
            favorited: "out".to_owned(),
            favorite: "3".to_owned(),
            language: "ja".to_owned(),
            copies: "2-10".to_owned(),
            added: "7d".to_owned(),
            filename: Some("1".to_owned()),
            sort: "title".to_owned(),
            ..FilterQuery::default()
        }
    }

    /// [`page_links`] as the browse list calls it, which is what these tests are about.
    fn browse_links(offset: u32, total: u32, has_more: bool) -> PageLinks {
        let query = query();
        page_links(offset, total, PAGE_SIZE, has_more, |page| {
            query.with_offset(page * PAGE_SIZE, total)
        })
    }

    /// Every filter has to survive a page turn.
    ///
    /// This is the failure that is invisible until it has already wasted somebody's time: the list
    /// looks right, and the filter quietly evaporates on the click of *next* — three pages into a
    /// corpus of hundreds of thousands of files, with no error anywhere.
    #[test]
    fn paging_keeps_every_filter_it_was_given() {
        let links = browse_links(100, 1000, true);
        let numbered: Vec<&String> = links.pages.iter().map(|page| &page.query).collect();
        for link in [&links.previous, &links.next, &links.first, &links.last]
            .into_iter()
            .chain(numbered)
            // The page being shown carries no query at all, by design: it is a label.
            .filter(|link| !link.is_empty())
        {
            assert!(link.contains("q=jobim"), "{link}");
            assert!(link.contains("artist=Tom+Jobim"), "{link}");
            assert!(link.contains("initial=C"), "{link}");
            assert!(link.contains("favorited=out"), "{link}");
            assert!(link.contains("favorite=3"), "{link}");
            assert!(link.contains("language=ja"), "{link}");
            assert!(link.contains("copies=2-10"), "{link}");
            assert!(link.contains("added=7d"), "{link}");
            assert!(link.contains("sort=title"), "{link}");
            // Not a filter, but it evaporates on the click of *next* in exactly the same way, and
            // a list that stops showing file names halfway through browsing is the same defect.
            assert!(link.contains("filename=1"), "{link}");
        }
    }

    /// Every filter has to survive the trip through a form body, too.
    ///
    /// The obligation this covers is the one the crate keeps rediscovering: every filter must appear
    /// in `to_filter`, in `to_form`, in `active` and in `rebuild`, and a fifth list to keep in step
    /// would be a fifth chance to leave one out. There is no fifth list — [`FilterQuery::from_body`]
    /// is serde reading the same struct — and this is what says so. Write a field out with `rebuild`
    /// and read it back with `from_body`, and a field missing from either fails here rather than as
    /// a package built over the wrong set of songs.
    #[test]
    fn a_filter_written_to_a_query_reads_back_the_same_from_a_body() {
        let original = query();
        let round = FilterQuery::from_body(&original.rebuild(0, "", None)).expect("read it back");
        assert_eq!(round.to_form(&[], &[]), original.to_form(&[], &[]));

        // And as the *bar* sends it, which is not what `rebuild` writes: a form submits every
        // control it has, so the empty ones arrive as empty strings rather than being left out.
        // That shape is the regression itself, so it is the shape worth pinning.
        let bar = "q=&artist=&suitability=&user_score=&initial=&favorite=&melody=\
                   &encoding_source=&granularity=&kind=&language=&copies=&added=&sort=";
        assert_eq!(
            FilterQuery::from_body(bar)
                .expect("read the bar")
                .to_form(&[], &[]),
            FilterQuery::default().to_form(&[], &[])
        );
    }

    /// A body of ticked rows is still a body the filter can be read out of.
    ///
    /// `hx-include="#rows, #filters"` is what lets *Title from file name* redraw the list it was
    /// used on, and it is only legal because a key this struct does not know is ignored however many
    /// times it arrives. Asserted rather than assumed: the alternative is a 400 on a button nobody
    /// tests by hand.
    #[test]
    fn the_ticked_rows_riding_along_do_not_stop_the_filter_being_read() {
        let body = "song_id=a&score=1&song_id=b&score=&title=x&row_artist=y&name=z\
                    &song_id=c&suitability=8-10&tags=bossa";
        let query = FilterQuery::from_body(body).expect("the filter survives the company it keeps");
        assert_eq!(query.suitability, "8-10");
        assert_eq!(query.tags, "bossa");

        // The other half of the same rule: a repeated key this struct *does* know is refused, which
        // is why no form beside the bar may reuse one of its names.
        assert!(FilterQuery::from_body("tags=a&tags=b").is_err());
    }

    /// The row language select must not be named `language`.
    ///
    /// Every row of `#rows` now carries one, and `#rows` rides in the same body as `#filters` for
    /// *Title from file name* and for the ticked-song actions. `FilterQuery` has a `language` key;
    /// serde answers a repeated known key with `duplicate_field`. So a page of rows named `language`
    /// would turn two working buttons into a 400 — the identical trap that made the bulk set's
    /// select `set_language`, found here before it could be found by clicking.
    #[test]
    fn a_row_language_select_can_ride_in_the_same_body_as_the_filter_bar() {
        let body = "song_id=a&row_language=pt&song_id=b&row_language=&song_id=c&row_language=ja\
                    &language=ja&tags=bossa";
        let query = FilterQuery::from_body(body).expect("the rows' own selects are ignored");
        assert_eq!(query.language, "ja", "the bar's own value, not a row's");
        assert_eq!(query.tags, "bossa");

        // What the name would have cost, spelled out: this is the same body with the rows spelling
        // it the wrong way, and it is a 400.
        assert!(FilterQuery::from_body("language=pt&language=ja&tags=bossa").is_err());
    }

    /// The row's artist box must not be named `artist`, for the reason its language select must not
    /// be named `language`.
    ///
    /// **This one would have failed intermittently, which is what makes it worth its own test.** A
    /// row's editor is in the DOM only while somebody has a row open, so a page of rows named
    /// `artist` sends nothing extra most of the time — and then turns every ticked-song action into a
    /// 400 for as long as one row is being renamed. A bug that comes and goes with something that
    /// looks unrelated is the expensive kind.
    #[test]
    fn a_row_artist_box_can_ride_in_the_same_body_as_the_filter_bar() {
        let body = "song_id=a&title=Wave&row_artist=Tom+Jobim\
                    &artist=Dire+Straits&tags=bossa";
        let query = FilterQuery::from_body(body).expect("the row's own box is ignored");
        assert_eq!(
            query.artist, "Dire Straits",
            "the bar's own value, not the row being edited"
        );
        assert_eq!(query.tags, "bossa");

        // What the name would have cost, spelled out.
        assert!(FilterQuery::from_body("artist=Tom+Jobim&artist=Dire+Straits").is_err());
    }

    /// `?copies=2+` narrows, and `?duplicates=1` does not.
    ///
    /// **The two are not the same retired spelling.** `2+` is a real bucket the bar cannot show —
    /// it is the union of the two below it rather than a fourth of them — and it parses because the
    /// Duplicates page links to it, where *more than one copy* is the whole question. A link whose
    /// value fell through to `Any` would quietly answer that with the entire corpus.
    ///
    /// `duplicates=1` is a checkbox with nothing left to fold onto, and **showing the whole corpus
    /// is the deliberate answer** for it: the alternative is a select with nothing selected sitting
    /// above a list that is narrowed anyway, which is a control disagreeing with the page it
    /// controls.
    #[test]
    fn the_two_or_more_bucket_narrows_and_the_retired_checkbox_does_not() {
        let linked = FilterQuery::from_body("copies=2%2B&tags=bossa").expect("read it");
        assert_eq!(
            CopiesFilter::parse(&linked.copies),
            CopiesFilter::AtLeastTwo
        );
        assert_eq!(linked.to_form(&[], &[]).copies, "2+");
        // It survives a page turn, because a bucket the page can reach has to outlive the first
        // link the pager writes.
        assert!(linked.rebuild(0, "", None).contains("copies=2%2B"));

        let checkbox = FilterQuery::from_body("duplicates=1&tags=bossa").expect("read it");
        assert_eq!(CopiesFilter::parse(&checkbox.copies), CopiesFilter::Any);
        assert_eq!(checkbox.to_form(&[], &[]).copies, "");
        let rebuilt = checkbox.rebuild(0, "", None);
        assert!(!rebuilt.contains("duplicates"), "{rebuilt}");
        // The rest of the filter is untouched — only that one checkbox was retired.
        assert!(rebuilt.contains("tags=bossa"), "{rebuilt}");

        // The three the bar itself offers are unaffected.
        let kept = FilterQuery::from_body("copies=2-10").expect("read it");
        assert_eq!(CopiesFilter::parse(&kept.copies), CopiesFilter::TwoToTen);
    }

    /// `min_score` is not read at all, and the rest of the filter still is.
    ///
    /// It used to fold onto the band holding N, so a bookmark written when this control was a
    /// `≥ N` ladder still narrowed the list. Nothing has shipped, so there is no such bookmark, and
    /// the field was a second spelling of one filter kept for nobody. What matters now is that it
    /// is *ignored* rather than mistaken for something: an unknown key must not take the page with
    /// it, and the keys beside it must still arrive.
    #[test]
    fn an_old_min_score_link_is_ignored_rather_than_read() {
        let old = FilterQuery::from_body("min_score=9&tags=bossa").expect("read it");
        assert_eq!(
            old.suitability(),
            SuitabilityFilter::Any,
            "the retired ladder no longer narrows anything"
        );
        assert_eq!(old.to_form(&[], &[]).suitability, "");

        // The filter it arrived beside is untouched, and nothing writes the old key back out.
        let rebuilt = old.rebuild(0, "", None);
        assert!(rebuilt.contains("tags=bossa"), "{rebuilt}");
        assert!(!rebuilt.contains("min_score"), "{rebuilt}");

        // A hand-made URL carrying both reads only the one the page draws.
        let both = FilterQuery::from_body("min_score=9&suitability=0-4").expect("read it");
        assert_eq!(both.suitability(), SuitabilityFilter::Low);
    }

    /// The band shows as a chip that says which band, and the × takes it off.
    ///
    /// There was no chip test for this filter while it was a ladder, which is how it could have gone
    /// on narrowing the list with the strip above saying nothing.
    #[test]
    fn the_suitability_band_shows_as_a_removable_chip() {
        let query = FilterQuery {
            suitability: "0-4".to_owned(),
            ..FilterQuery::default()
        };
        let chips = query.active(&[], km_locale::Locale::English);
        assert_eq!(chips.len(), 1, "{chips:?}");
        assert_eq!(chips[0].label, "suitability under 5");
        assert!(!chips[0].remove.contains("suitability"), "{:?}", chips[0]);

        // *any* is the absence of a filter, so it is not a chip reading "suitability any".
        assert!(
            FilterQuery::default()
                .active(&[], km_locale::Locale::English)
                .is_empty()
        );
    }

    /// Every filter has to show as a chip, or it narrows the list with nothing admitting it.
    ///
    /// The other half of the obligation the test above covers: a filter that survives a page turn
    /// but never appears in the bar is one somebody cannot find to remove, which reads as the corpus
    /// being smaller than it is.
    #[test]
    fn the_language_filter_shows_as_a_removable_chip() {
        let chips = FilterQuery {
            language: "ja".to_owned(),
            ..FilterQuery::default()
        }
        .active(&[], km_locale::Locale::English);
        let chip = chips
            .iter()
            .find(|chip| chip.label == "Japanese")
            .expect("a chip naming the language; got {chips:?}");
        assert!(
            !chip.remove.contains("language="),
            "its remove link drops the filter: {}",
            chip.remove
        );

        // Prose, so the name and not the code -- and the two sentinels have to read as sentences
        // rather than as a language nobody has heard of.
        for (value, label) in [("unset", "language unset"), ("set", "any language")] {
            let chips = FilterQuery {
                language: value.to_owned(),
                ..FilterQuery::default()
            }
            .active(&[], km_locale::Locale::English);
            assert!(
                chips.iter().any(|chip| chip.label == label),
                "{value} should read as {label:?}; got {chips:?}"
            );
        }

        // A hand-typed nonsense value narrows nothing, so it must not leave a chip behind either.
        let chips = FilterQuery {
            language: "Klingon".to_owned(),
            ..FilterQuery::default()
        }
        .active(&[], km_locale::Locale::English);
        assert!(chips.is_empty(), "got {chips:?}");
    }

    /// The artist filter shows as a chip that says whose, and comes off again.
    ///
    /// **The chip is what makes a bare link honest.** Clicking an artist in a row replaces the whole
    /// filter rather than adding to it — a row cannot know what else is narrowing the list, because
    /// the same markup is swapped back in by routes that never see the browse query — so the strip
    /// is the only thing on the page that says what happened. Without it the click would read as the
    /// corpus having shrunk.
    #[test]
    fn the_artist_filter_shows_as_a_removable_chip() {
        let chips = FilterQuery {
            artist: "Tom Jobim".to_owned(),
            ..FilterQuery::default()
        }
        .active(&[], km_locale::Locale::English);
        let chip = chips
            .iter()
            .find(|chip| chip.label == "by Tom Jobim")
            .unwrap_or_else(|| panic!("a chip naming the artist; got {chips:?}"));
        assert!(
            !chip.remove.contains("artist="),
            "its remove link drops the filter: {}",
            chip.remove
        );

        // Whitespace is not an artist, and must not leave a chip narrowing nothing.
        let chips = FilterQuery {
            artist: "   ".to_owned(),
            ..FilterQuery::default()
        }
        .active(&[], km_locale::Locale::English);
        assert!(chips.is_empty(), "got {chips:?}");
    }

    /// The two filters this change added, and the one it retired, all read as sentences.
    #[test]
    fn the_letter_and_copies_filters_show_as_removable_chips() {
        let chip_for = |query: FilterQuery, label: &str| {
            let chips = query.active(&[], km_locale::Locale::English);
            chips
                .iter()
                .find(|chip| chip.label == label)
                .unwrap_or_else(|| panic!("a chip reading {label:?}; got {chips:?}"))
                .clone()
        };

        // Each reads as a sentence: a bare `starts with #` would say nothing.
        for (value, label) in [
            ("C", "starts with C"),
            ("0-9", "starts with a number"),
            ("#", "starts with a symbol"),
        ] {
            let chip = chip_for(
                FilterQuery {
                    initial: value.to_owned(),
                    ..FilterQuery::default()
                },
                label,
            );
            assert!(!chip.remove.contains("initial="), "{}", chip.remove);
        }

        for (value, label) in [
            ("1", "one copy"),
            ("2-10", "2\u{2013}10 copies"),
            ("10+", "more than 10 copies"),
            // The bucket the bar cannot show still gets a chip, because the Duplicates page links
            // to it: a filter narrowing the list with nothing on screen saying so is the one thing
            // the bar may not do, and a chip is how it says so.
            ("2+", "more than one copy"),
        ] {
            let chip = chip_for(
                FilterQuery {
                    copies: value.to_owned(),
                    ..FilterQuery::default()
                },
                label,
            );
            assert!(!chip.remove.contains("copies="), "{}", chip.remove);
        }

        for (value, label) in [
            ("1d", "added in the last day"),
            ("7d", "added in the last 7 days"),
            ("30d", "added in the last 30 days"),
            ("30d+", "added more than 30 days ago"),
        ] {
            let chip = chip_for(
                FilterQuery {
                    added: value.to_owned(),
                    ..FilterQuery::default()
                },
                label,
            );
            assert!(!chip.remove.contains("added="), "{}", chip.remove);
        }

        // Nonsense narrows nothing, so it leaves no chip — the rule the language filter follows.
        for query in [
            FilterQuery {
                initial: "ZZ".to_owned(),
                ..FilterQuery::default()
            },
            FilterQuery {
                copies: "lots".to_owned(),
                ..FilterQuery::default()
            },
        ] {
            assert!(query.active(&[], km_locale::Locale::English).is_empty());
        }
    }

    /// Both filed arms read as sentences, and the retired spelling folds onto the one it meant.
    ///
    /// The `1` matters here rather than only in `filter.rs`: a chip is what says a filter is on, so
    /// a link the parser reads and the strip does not would narrow the list in silence — which is
    /// the fault every chip test in this file exists to prevent.
    #[test]
    fn both_filed_filters_show_as_removable_chips() {
        for (value, label) in [
            ("in", "in any favorite"),
            ("1", "in any favorite"),
            ("out", "in no favorite"),
        ] {
            let query = FilterQuery {
                favorited: value.to_owned(),
                ..FilterQuery::default()
            };
            let chips = query.active(&[], km_locale::Locale::English);
            assert_eq!(chips.len(), 1, "{value:?}: {chips:?}");
            assert_eq!(chips[0].label, label, "{value:?}");
            assert!(!chips[0].remove.contains("favorited"), "{:?}", chips[0]);
        }

        // The page turn normalizes, so `1` reaches the next request as the spelling the select can
        // show — one filter, one spelling.
        let old = FilterQuery::from_body("favorited=1").expect("read it");
        let rebuilt = old.rebuild(0, "", None);
        assert!(rebuilt.contains("favorited=in"), "{rebuilt}");
        assert!(!rebuilt.contains("favorited=1"), "{rebuilt}");
        assert_eq!(old.to_form(&[], &[]).favorited, "in");

        for nonsense in ["0", "yes", "either"] {
            let query = FilterQuery {
                favorited: nonsense.to_owned(),
                ..FilterQuery::default()
            };
            assert!(
                query.active(&[], km_locale::Locale::English).is_empty(),
                "{nonsense}"
            );
        }
    }

    #[test]
    fn the_ends_of_the_list_hide_the_buttons_that_would_not_move() {
        let first = browse_links(0, 1000, true);
        assert!(first.previous.is_empty());
        // The window starts at page 1, so there is no *first* button beside it.
        assert!(first.first.is_empty());
        assert!(first.next.contains("offset=50"));

        // 1000 rows at fifty a page is offsets 0..950, so 950 is the last one.
        let last = browse_links(950, 1000, false);
        assert!(last.next.is_empty());
        assert!(last.last.is_empty(), "the window already reaches the end");
        assert!(last.previous.contains("offset=900"));

        // One page of results: one number, which is the page you are on, and no buttons at all.
        let only = browse_links(0, 40, false);
        assert_eq!(only.pages.len(), 1);
        assert!(only.pages[0].current);
        assert!(only.pages[0].query.is_empty());
        assert_eq!(
            PageLinks {
                pages: Vec::new(),
                ..only
            },
            PageLinks::default()
        );
    }

    /// The numbers are a window of five either side, with the ends drawn only beyond it.
    ///
    /// The corpus this is for is hundreds of thousands of files, which is thousands of pages — so the
    /// window is the whole of what makes numbered pages possible, and the two ends are what make it
    /// navigable rather than a way of moving five pages at a time for ever.
    #[test]
    fn the_numbers_are_a_window_around_the_page_being_shown() {
        // Page 21 of 100: 1000 offsets of 50 into 5000 rows.
        let links = browse_links(1000, 5000, true);
        let numbers: Vec<u32> = links.pages.iter().map(|page| page.number).collect();
        assert_eq!(numbers, (16..=26).collect::<Vec<_>>());

        let here: Vec<&PageNumber> = links.pages.iter().filter(|page| page.current).collect();
        assert_eq!(here.len(), 1, "exactly one page is the one being shown");
        assert_eq!(here[0].number, 21);
        assert!(here[0].query.is_empty(), "and it is a label, not a button");

        // Both ends are out of the window, so both buttons are drawn — and they are the ends
        // themselves rather than five pages away.
        assert!(links.first.is_empty() || !links.first.contains("offset="));
        assert!(links.last.contains("offset=4950"), "{}", links.last);

        // A window that runs past the start is clamped rather than wrapped.
        let near_start = browse_links(100, 5000, true);
        let numbers: Vec<u32> = near_start.pages.iter().map(|page| page.number).collect();
        assert_eq!(numbers, (1..=8).collect::<Vec<_>>());
        assert!(near_start.first.is_empty(), "page 1 is already in view");
    }

    /// A total that lags a scan cannot draw a *last* button behind the page it was pressed from.
    ///
    /// The count is carried between page turns so a turn need not re-count the corpus, and a scan
    /// writing rows underneath makes it a reading rather than a fact. The pager says so with a `~`;
    /// what it must not do is offer to jump backwards.
    #[test]
    fn a_stale_total_never_points_the_last_button_backwards() {
        // The total says 100 rows — two pages — while the reader is on the eleventh and there is
        // demonstrably another.
        let links = browse_links(500, 100, true);
        let numbers: Vec<u32> = links.pages.iter().map(|page| page.number).collect();
        assert_eq!(
            numbers.last(),
            Some(&12),
            "the window has to reach past the page being shown: {numbers:?}"
        );
        assert!(links.last.is_empty(), "the end is inside the window");
    }

    /// Whether there is a next page is the database's answer, not the carried total's.
    ///
    /// The total travels in the query string so a page turn need not re-count the corpus, which
    /// means it can be stale — a scan writing underneath the reader, or somebody editing the URL. It
    /// is allowed to be wrong about the *label*. It is not allowed to be wrong about whether a page
    /// exists, because that is how a reader gets stranded three pages from the end with no button.
    #[test]
    fn a_stale_total_cannot_hide_a_page_that_is_there() {
        // The total says this is the last page; the query found a row past it. The row wins.
        let links = browse_links(950, 1000, true);
        assert!(links.next.contains("offset=1000"), "{}", links.next);

        // And the other way: the total promises thousands more, the query found none. Still the end.
        let links = browse_links(100, 99_999, false);
        assert!(links.next.is_empty(), "{}", links.next);
        // The window still reaches the end the total claims, because the label does; what it must
        // not do is offer a *next* the query says is not there.
        assert!(links.last.contains("offset="), "{}", links.last);
    }

    /// A page turn carries the count forward; clearing a filter throws it away.
    ///
    /// The saving is the whole point of the field — one `COUNT(*)` per filter rather than one per
    /// page turn — and so is the exception: a different filter matches a different number of songs,
    /// so carrying it there would not be stale, it would be answering another question.
    #[test]
    fn the_count_travels_with_the_pages_but_not_past_a_filter_change() {
        // Offset 500 of 1000 rows is page 11 of 20, which is far enough in that both ends are out of
        // the window and every control this pager can draw is drawn.
        let links = browse_links(500, 1000, true);
        assert!(links.next.contains("total=1000"), "{}", links.next);
        assert!(links.previous.contains("total=1000"), "{}", links.previous);
        assert!(links.first.contains("total=1000"), "{}", links.first);
        assert!(links.last.contains("total=1000"), "{}", links.last);
        for page in &links.pages {
            assert!(
                page.query.is_empty() || page.query.contains("total=1000"),
                "{}",
                page.query
            );
        }

        let mut query = query();
        query.total = Some(1000);
        assert!(
            !query.without("language").contains("total="),
            "{}",
            query.without("language")
        );
    }

    /// The numbers never stop behind the page being shown, against a total that lags a scan.
    ///
    /// The last page is computed from a total that may be a reading rather than a fact, and clamping
    /// to that alone would draw a window ending before the page somebody is on — a pager offering
    /// only ways backwards from a page it says does not exist.
    #[test]
    fn the_window_never_ends_behind_the_page_it_is_drawn_for() {
        // The reader is at offset 5000 — page 101 — but the total carried in says the corpus ends at
        // 1000, which is page 20.
        let links = browse_links(5_000, 1_000, true);
        let numbers: Vec<u32> = links.pages.iter().map(|page| page.number).collect();
        assert!(numbers.contains(&101), "{numbers:?}");
        assert_eq!(numbers.last(), Some(&102), "{numbers:?}");
        assert!(links.next.contains("offset=5050"), "{}", links.next);
    }

    /// Clearing the language leaves every other filter alone, and puts the reader back on page one.
    #[test]
    fn clearing_one_filter_keeps_the_others_and_starts_again_at_the_top() {
        let mut query = query();
        query.offset = Some(300);
        let cleared = query.without("language");
        assert!(!cleared.contains("language="), "{cleared}");
        assert!(!cleared.contains("offset="), "{cleared}");
        assert!(cleared.contains("q=jobim"), "{cleared}");
        assert!(cleared.contains("initial=C"), "{cleared}");
        assert!(cleared.contains("filename=1"), "{cleared}");
    }

    /// *Open in browser* opens the page somebody is looking at, and only if it is one of ours.
    ///
    /// The first half is the fault being fixed: the filter is in the address bar and nowhere the
    /// server can see, so opening `state.url()` opened the whole corpus. The second is why the page
    /// is not simply believed — the string it sends becomes an argument to the platform's opener,
    /// which opens files and programs as readily as pages, so anything that is not this server falls
    /// back to the plain address rather than being handed over or refused with an explanation.
    #[test]
    fn open_in_browser_takes_the_page_it_is_given_only_if_it_is_ours() {
        let base = "http://127.0.0.1:8178/";

        assert_eq!(
            browser_target(Some("http://127.0.0.1:8178/songs?q=jobim&offset=150"), base),
            "http://127.0.0.1:8178/songs?q=jobim&offset=150"
        );

        // A page that said nothing — an older template, or a browser with JavaScript off.
        assert_eq!(browser_target(None, base), base);
        assert_eq!(browser_target(Some(""), base), base);
        // Somewhere else. The trailing slash on `base` is what makes the third of these a miss.
        assert_eq!(browser_target(Some("https://example.com/"), base), base);
        assert_eq!(
            browser_target(Some("file:///C:/Windows/notepad.exe"), base),
            base
        );
        assert_eq!(browser_target(Some("http://127.0.0.1:81780/x"), base), base);
    }

    /// The file-name box says how the list is drawn and nothing about which songs are in it.
    #[test]
    fn showing_file_names_is_not_a_filter() {
        let plain = FilterQuery::default().to_filter();
        let asked = FilterQuery {
            filename: Some("1".to_owned()),
            ..FilterQuery::default()
        };
        // Same rows, same order, same count — only the drawing changes.
        assert_eq!(format!("{:?}", asked.to_filter()), format!("{plain:?}"));
        assert!(asked.to_form(&[], &[]).filename);
    }

    /// The file names are off until the box asks for them, which is what an absent key means again.
    ///
    /// Both cases are the whole of `filenames` now. It was three while the default was on, and the
    /// middle one — *the bar sent an unticked box* — needed a `filename_set` marker to be tellable
    /// from a fresh page load at all. Off by default collapses the two into one answer, which is the
    /// half of that reversal worth pinning: an absent key must not turn the chips on.
    #[test]
    fn file_names_are_hidden_until_the_box_asks_for_them() {
        // Nobody has said: a bare `/songs`, or a page turn from one.
        assert!(!FilterQuery::default().filenames());
        assert!(!FilterQuery::default().to_form(&[], &[]).filename);

        // The box was ticked, from the bar or from a link.
        let on = FilterQuery {
            filename: Some("1".to_owned()),
            ..FilterQuery::default()
        };
        assert!(on.filenames());
        assert!(on.to_form(&[], &[]).filename);
    }

    /// Turning them on survives a page turn and a *clear all*; leaving them off says nothing.
    ///
    /// The second half is the one that regressed last time in the other direction: while off was the
    /// state that had to be spelled out, a link that said nothing turned the chips back on at the
    /// first click of *next*. Now silence is the default and there is nothing to leak.
    #[test]
    fn the_file_name_box_survives_paging_and_clearing() {
        let on = FilterQuery {
            filename: Some("1".to_owned()),
            q: "jobim".to_owned(),
            ..FilterQuery::default()
        };
        let next = on.with_offset(100, 4200);
        assert!(next.contains("filename=1"), "{next}");

        let cleared = on.only_view();
        assert!(cleared.contains("filename=1"), "{cleared}");
        assert!(!cleared.contains("q="), "{cleared}");

        // And the other way, which is now the ordinary case.
        let off = FilterQuery::default().with_offset(100, 4200);
        assert!(!off.contains("filename"), "{off}");
        assert!(!FilterQuery::default().only_view().contains("filename"));
    }

    /// A package's name becomes something that can be a file on three platforms.
    ///
    /// The empty answer is the case worth pinning: it is not a panic and not a silent `.kmpkg` with
    /// no name — the handler refuses and says which name did it, because a person who typed one in
    /// an alphabet this rule drops entirely has to be told rather than left with a package called
    /// nothing.
    #[test]
    fn a_package_name_becomes_a_file_name() {
        assert_eq!(slug("Brasil vol 2"), "brasil-vol-2");
        assert_eq!(slug("  Rock & Roll  "), "rock-roll");
        assert_eq!(slug("Músicas Brasileiras"), "msicas-brasileiras");
        assert_eq!(slug("a//b"), "ab", "nothing that could be a path separator");
        assert_eq!(slug("-- x --"), "x", "no leading or trailing dashes");
        assert_eq!(
            slug("日本語"),
            "",
            "and a name this rule cannot spell is empty"
        );
    }

    /// The create form no longer sends an id, and an absent one means *generate*.
    ///
    /// The three empty shapes are one case on purpose: a page that posts `id=` and a page that omits
    /// the field are indistinguishable to a form parser, and neither is somebody asking for a package
    /// identified by the empty string.
    #[test]
    fn a_create_form_that_names_no_id_is_asking_for_a_generated_one() {
        for body in ["name=Rock", "name=Rock&id=", "name=Rock&id=%20%20"] {
            assert_eq!(
                supplied_id(&Fields::parse(body)),
                None,
                "{body} should be asking for a generated id"
            );
        }
    }

    /// ...and an id that *is* supplied is honored, because `build::import` comes through the same
    /// handler carrying the one it read out of a built `.kmpkg`. Giving that package a new identity
    /// would make the machine treat a re-import as a different package.
    #[test]
    fn an_id_that_was_supplied_is_kept_rather_than_replaced() {
        assert_eq!(
            supplied_id(&Fields::parse("id=classic-rock-01&name=Rock")),
            Some("classic-rock-01")
        );
        // Trimmed, so a stray space in an imported manifest does not become part of the identity.
        assert_eq!(
            supplied_id(&Fields::parse("id=%20a1b2c3%20")),
            Some("a1b2c3")
        );
    }

    /// The Build tab names a folder once and a file per form, a blank folder is the data folder, and a
    /// whole path from a caller that is not the page wins.
    #[test]
    fn a_build_path_is_a_folder_and_a_file_name() {
        let root = std::path::Path::new("/corpus");
        assert_eq!(
            chosen_path(
                &Fields::parse("folder=%2Fout&file=brasil-1.0.0.kmpkg"),
                Some(root)
            ),
            Some(PathBuf::from("/out").join("brasil-1.0.0.kmpkg"))
        );
        assert_eq!(
            chosen_path(
                &Fields::parse("folder=&file=brasil.kmspec.yaml"),
                Some(root)
            ),
            Some(crate::db::data_dir(root).join("brasil.kmspec.yaml"))
        );
        assert_eq!(
            chosen_path(
                &Fields::parse("out=%2Fwhole%2Fpath.kmpkg&file=ignored"),
                None
            ),
            Some(PathBuf::from("/whole/path.kmpkg"))
        );
        assert_eq!(
            chosen_path(&Fields::parse("folder=%2Fout&file="), Some(root)),
            None
        );
    }

    /// A version a person types is three numbers, and the two forms that take one say so.
    ///
    /// **The refusal is what keeps the tick box honest**: a build can only raise a version it can
    /// read, so a tool that stored `2024-spring` would draw a box that quietly did nothing. The
    /// two-part `1.0` is the one to keep in the list — it is what a hand-written description
    /// carries and what somebody types when they mean "the first one".
    #[test]
    fn a_typed_version_is_three_numbers_or_it_is_refused() {
        for good in ["1.0.0", "0.0.0", "12.34.56"] {
            assert!(
                version_refusal(good, km_locale::Locale::English).is_none(),
                "{good} should have been accepted"
            );
        }
        for bad in [
            "1.0",
            "1",
            "",
            "v1.0.0",
            "1.0.0-beta",
            "2024-spring",
            "01.0.0",
        ] {
            assert!(
                version_refusal(bad, km_locale::Locale::English).is_some(),
                "{bad} should have been refused"
            );
        }
    }

    /// A form that leaves the version out is asking for the default, not for a refusal.
    ///
    /// `package_row` fills a blank with `1.0.0`, so the check above never sees the empty string
    /// from that direction — which is what stops the refusal firing on a form somebody submitted
    /// without touching the field.
    #[test]
    fn a_create_form_with_no_version_takes_the_default() {
        let row = package_row(&Fields::parse("name=Rock"), "a1b2c3");
        assert_eq!(row.version, "1.0.0");
        assert!(version_refusal(&row.version, km_locale::Locale::English).is_none());
    }

    #[test]
    fn only_an_explicit_request_gets_a_row_back_instead_of_a_message() {
        assert!(
            ReplyQuery {
                reply_as: Some("row".to_owned())
            }
            .wants_row()
        );
        assert!(!ReplyQuery::default().wants_row());
        assert!(
            !ReplyQuery {
                reply_as: Some("something else".to_owned())
            }
            .wants_row()
        );
    }
}
