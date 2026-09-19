//! The template structs.
//!
//! One struct per rendered thing, page or fragment. The compile-time checking is the reason askama
//! was chosen over a runtime engine: a field renamed here and not in the template is a build failure,
//! not a 500 discovered by somebody halfway through curating a package.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
/// askama resolves `|t` against a module called `filters` in the scope the template was derived in,
/// which is this one. The line every crate with a catalog writes once.
use km_locale::filters;

use crate::db::{ClusterCounts, Counts, Initial, SongDetail, SuitabilityFilter};
use crate::model::{FavoriteNode, LyricHit, PackageMember, PackageRow, SongRow};
use crate::scan::ProgressView;

/// The markers `Db::lyric_search` asks FTS5 to wrap each matched term in.
///
/// Control characters, chosen because they cannot be mistaken for markup. The alternative — asking
/// FTS5 for `<b>` and `</b>` and rendering the result unescaped — would have put arbitrary bytes out
/// of an arbitrary corpus file straight into the page.
const MATCH_START: char = '\u{2}';
const MATCH_END: char = '\u{3}';

/// Cuts a marked passage into runs of text, each flagged with whether it matched.
///
/// The point is that the template renders every run through askama's ordinary escaping and sets the
/// emphasis with an element of its own, so a lyric containing `<script>` is a lyric that reads
/// `<script>`. Returning pairs rather than a string of HTML is what makes that the only possibility
/// rather than the thing to remember.
///
/// A file whose lyrics genuinely contain one of the two markers can mis-split a passage. That is a
/// wrong highlight on one row, and it was preferred to the class of bug the other approach invites.
pub fn highlight(passage: &str) -> Vec<(String, bool)> {
    let mut runs = Vec::new();
    let mut matched = false;
    for piece in passage.split_inclusive([MATCH_START, MATCH_END]) {
        let (text, next) = match piece.chars().last() {
            Some(MATCH_START) => (&piece[..piece.len() - MATCH_START.len_utf8()], true),
            Some(MATCH_END) => (&piece[..piece.len() - MATCH_END.len_utf8()], false),
            // The tail after the last marker, which carries no marker of its own.
            _ => (piece, matched),
        };
        if !text.is_empty() {
            runs.push((text.to_owned(), matched));
        }
        matched = next;
    }
    runs
}

/// Renders a template in one language, turning a template failure into a 500 with the reason.
///
/// A template cannot normally fail — the markup was checked when this was built — so reaching the
/// error arm means something like a formatter panic, and saying so beats a blank page.
///
/// **The locale goes in once here and reaches every fragment below.** askama carries the values
/// store into a nested `{{ child|safe }}` render on its own, so no template struct holds a locale
/// and no construction site has to pass one. A render with none draws `⟦the-key⟧`, which is
/// `km_locale::filters`' answer and is legible on the page rather than a blank 500.
pub fn page<T: Template>(template: &T, locale: km_locale::Locale) -> Response {
    match render(template, locale) {
        Ok(body) => Html(body).into_response(),
        Err(error) => template_error(&error),
    }
}

/// The same, under a status that is not 200.
///
/// **For a page that *is* the refusal**, which is `handlers::failed_page` and nothing else so far. A
/// navigation that could not be answered is drawn as a page so there is something to press, and the
/// status has to go on saying what happened — a 503 answered 200 would tell a proxy, a reload and
/// any future caller that the corpus was read after all.
pub fn page_with_status<T: Template>(
    template: &T,
    locale: km_locale::Locale,
    status: StatusCode,
) -> Response {
    match render(template, locale) {
        Ok(body) => (status, Html(body)).into_response(),
        Err(error) => template_error(&error),
    }
}

/// One template, in one language. The line the five helpers here share.
///
/// **The store is a local binding and the `String` is what leaves**, which is load-bearing rather
/// than style: `filters::values` hands back a `&dyn Any`, and that is neither `Send` nor `Sync`. A
/// handler holding one across an `.await` stops being `Send`, and axum reports that as `Handler` not
/// implemented for the whole function, naming neither the line nor the value. Every caller here is
/// synchronous for the same reason.
fn render<T: Template>(template: &T, locale: km_locale::Locale) -> askama::Result<String> {
    let values = filters::values(crate::words::messages(locale));
    template.render_with_values(&values)
}

/// The bits every full page needs.
#[derive(Debug, Clone)]
pub struct Chrome {
    /// Which nav entry is current.
    pub tab: &'static str,
    /// The folder being curated.
    pub root: String,
    /// Headline counts.
    pub counts: Counts,
    /// Whether the page is in this tool's own window rather than in a browser.
    ///
    /// It decides which of two buttons the header ends with, and the two are the same decision seen
    /// from either side: in a window, closing the window is already what quitting means, so Quit is
    /// a second way to do what the X does and the useful button is the one that hands the page to a
    /// real browser. In a tab there is no X that stops anything, and Quit is the only way out of a
    /// tool that was started by double-clicking a corpus.
    pub windowed: bool,
    /// Which machine this tool would install into, and what that machine calls itself.
    ///
    /// **`None` is a real answer and the header says so**, rather than leaving a gap: *no machine
    /// set* is a state somebody needs to be able to see, because Play and Install are the two
    /// controls in this tool that reach outside it and both are refused in it.
    ///
    /// The name is `None` until something has answered at that address, which is the ordinary state
    /// of a machine that is switched off — and the header must not go and ask, since it is drawn on
    /// every page. See `State::machine_shown`.
    pub machine: Option<(String, Option<String>)>,
    /// Which language this page is drawn in, for `<html lang>`.
    ///
    /// A field rather than a `|t`, because an attribute's value is a tag and not a message: a
    /// catalog holding `en` under a key would be a string a translator could change, and changing it
    /// is the one thing that must not happen.
    pub locale: &'static str,
    /// The counts strip, worded where a plural can be chosen by the number beside it.
    pub said: HeaderCounts,
    /// What the header's machine tag says in its tooltip, where a machine is set.
    pub machine_title: Option<String>,
    /// What `static/ui.js` says, put on `<body>` for it to read.
    pub script: ScriptWords,
    /// The filter the songs page was last looking through, as a query string with no leading `?`.
    ///
    /// On the chrome rather than on the songs page, because the two places that read it are the nav
    /// — drawn on every page, including the six that are not the songs page — and the song detail
    /// page's way back to the list. See [`Chrome::songs_href`] and `State::songs_filter`.
    pub songs_filter: String,
    /// Whether this header was built without reading the corpus.
    ///
    /// **Set by [`Chrome::bare`], and what it hides is the two facts that come out of the
    /// database**: the counts strip and the machine tag. The page that needs it is the one drawn
    /// *because* a read failed, so asking again for the header would fail the same way —
    /// `Db::counts` is the call that refused, and the machine tag is a second read behind
    /// `State::chosen_machine`.
    ///
    /// **Left out rather than guessed.** `Counts::default` is four zeroes, and a header reading
    /// *0 songs · 0 files* on a corpus of hundreds of thousands is a lie a reader has no way to
    /// see through. *No machine set* is the same lie one field over, and that one is about the two
    /// controls that reach outside this tool.
    ///
    /// Everything else the header draws is in memory — the nav, the folder, the version, and the
    /// button at the end — so a sparse header is still the whole way back out of a failed page.
    pub sparse: bool,
}

impl Chrome {
    /// The header, with the three things that have to be worded rather than counted.
    ///
    /// **One place composes them**, so what a page draws and what a test renders are the same
    /// sentences. The counts are plurals, the machine tag's tooltip carries an address, and the
    /// script's five are read off `<body>` — each needs a catalog, and none of them is a `|t` a
    /// template could spend.
    pub fn new(
        tab: &'static str,
        locale: km_locale::Locale,
        root: String,
        counts: Counts,
        windowed: bool,
        machine: Option<(String, Option<String>)>,
        songs_filter: String,
    ) -> Self {
        let words = crate::words::messages(locale);
        let said = |key: &str, count: u32| {
            words
                .msg_with(key, &[("count", i64::from(count).into())])
                .into_owned()
        };
        Self {
            tab,
            locale: locale.tag(),
            root,
            said: HeaderCounts {
                songs: said("header-songs", counts.songs),
                files: said("header-files", counts.files),
                // Nothing at all at zero, which is what the strip has always done: a count that can
                // reach zero and is still drawn stops being news.
                failed: (counts.failed > 0).then(|| said("header-failed", counts.failed)),
                favorites: said("header-favorites", counts.favorites),
            },
            machine_title: machine.as_ref().map(|(url, _)| {
                words
                    .msg_with("header-machine-title", &[("address", url.as_str().into())])
                    .into_owned()
            }),
            script: ScriptWords {
                // `{what}` and `{status}` go in as the arguments, so the catalog decides the word
                // order and `ui.js` fills the two values it is the only one to have.
                answered: words
                    .msg_with(
                        "js-answered",
                        &[("what", "{what}".into()), ("status", "{status}".into())],
                    )
                    .into_owned(),
                unreachable: words.msg("js-unreachable").into_owned(),
                timed_out: words
                    .msg_with("js-timed-out", &[("what", "{what}".into())])
                    .into_owned(),
                swap_failed: words
                    .msg_with("js-swap-failed", &[("what", "{what}".into())])
                    .into_owned(),
                the_tool: words.msg("js-the-tool").into_owned(),
            },
            counts,
            windowed,
            machine,
            songs_filter,
            sparse: false,
        }
    }

    /// The same header, for a page drawn when the corpus could not be read.
    ///
    /// **It asks the database nothing**, which is the whole point of it: the caller is
    /// `handlers::failed_page`, and what sent it there is a read that refused. `Chrome::new` would
    /// take `Db::counts` and `State::chosen_machine`, and both go back to the connection that just
    /// said no — so a header built the ordinary way would fail to draw the page reporting the
    /// failure.
    ///
    /// What it keeps is everything held in memory: the nav, the folder, whether this is the tool's
    /// own window, the remembered filter, and the sentences `static/ui.js` reads off `<body>`. What
    /// it drops is named on [`Chrome::sparse`].
    pub fn bare(
        tab: &'static str,
        locale: km_locale::Locale,
        root: String,
        windowed: bool,
        songs_filter: String,
    ) -> Self {
        Self {
            sparse: true,
            ..Self::new(
                tab,
                locale,
                root,
                Counts::default(),
                windowed,
                None,
                songs_filter,
            )
        }
    }

    /// Where the nav's Songs tab points, and where a song page goes back to.
    ///
    /// **`/songs` plus whatever was last being looked through**, which is what makes leaving the
    /// page and coming back not a reset. A method rather than a branch in the template because two
    /// templates ask, and because the `?` belongs to whether there is a filter at all rather than to
    /// either caller.
    pub fn songs_href(&self) -> String {
        if self.songs_filter.is_empty() {
            "/songs".to_owned()
        } else {
            format!("/songs?{}", self.songs_filter)
        }
    }
    /// What the header's machine tag says: the machine's name where one is known, else its address.
    ///
    /// A method rather than a branch in the template, because a name that has gone empty and a name
    /// that was never learned are the same thing to a reader and should be one case here.
    pub fn machine_label(&self) -> Option<&str> {
        self.machine
            .as_ref()
            .map(|(url, name)| name.as_deref().unwrap_or(url.as_str()))
    }
}

/// The header's counts strip, worded rather than counted.
///
/// **Four sentences rather than four numbers**, because each is a plural over the number beside it
/// and Portuguese chooses its own form. `km_locale::filters` keeps markup to a key and nothing else
/// for exactly this: *a plural is arithmetic*, and arithmetic in markup is where it stops being
/// testable.
#[derive(Debug, Clone)]
pub struct HeaderCounts {
    /// Distinct recordings.
    pub songs: String,
    /// Files on disk.
    pub files: String,
    /// Files that did not parse, or `None` where none did.
    ///
    /// `None` rather than a zero to hide in the template, because the strip draws nothing at all
    /// when the count is zero — a number that can reach zero and still be shown is a permanent
    /// fixture of the header.
    pub failed: Option<String>,
    /// Songs marked as favorites.
    pub favorites: String,
}

/// What `static/ui.js` says when a request fails.
///
/// **Composed here and read off `<body>`**, because a static script cannot go through the `|t`
/// filter and an English sentence inside it would appear on a Portuguese page with nothing to catch
/// it: the catalog scanner reads templates, and a page test never fetches this file.
/// `km-remote-pages`' `scan.js` reads its own words the same way.
///
/// **Two carry `{what}` and `{status}`**, which are values only the browser has — the path that
/// failed and the status it answered. Each arrives with those two words standing in for them, so the
/// sentence is ordered by the catalog and the browser does one string replace. That is not a
/// template engine and must not become one; nothing here builds markup.
#[derive(Debug, Clone)]
pub struct ScriptWords {
    /// A request the server refused, with `{what}` and `{status}` still in it.
    pub answered: String,
    /// Nothing answered at all, which here almost always means the tool was quit.
    pub unreachable: String,
    /// A request given up on, with `{what}` still in it.
    pub timed_out: String,
    /// A reply that could not be put on the page, with `{what}` still in it.
    pub swap_failed: String,
    /// What to call this tool where a request carried no path to name.
    pub the_tool: String,
}

/// `GET /songs`
#[derive(Template)]
#[template(path = "songs.html")]
pub struct SongsPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The rows and their paging.
    pub rows: SongRows,
    /// Every favorite, for the filter dropdown.
    pub favorites: Vec<FavoriteNode>,
    /// Every package, for the "add selection to" dropdown.
    pub packages: Vec<PackageRow>,
    /// The filter bar's current values, so the controls keep their state.
    pub query: FilterForm,
    /// What is narrowing the list. Drawn from here on a page load and replaced out of band by every
    /// `/songs/rows` after it.
    pub chips: FilterChips,
    /// The filters somebody named, with a favorite that has gone taken out of each.
    pub saved: SavedFilters,
}

impl SongsPage {
    /// Whether any favorite is a working list, which is when the filter draws their group.
    pub fn has_working_lists(&self) -> bool {
        self.favorites.iter().any(|f| f.temporary)
    }
}

/// `GET /songs/rows` — the table body alone.
#[derive(Template)]
#[template(path = "song_rows.html")]
pub struct SongRows {
    /// The rows themselves.
    pub songs: Vec<SongRow>,
    /// How many match the filter in total.
    pub total: u32,
    /// Rows skipped.
    pub offset: u32,
    /// The query string for the previous page, empty when there is none.
    pub previous: String,
    /// The query string for the next page, empty when there is none.
    pub next: String,
    /// The query string for the first page, empty when the window already reaches it.
    pub first_page: String,
    /// The query string for the last page, empty when the window already reaches it.
    pub last_page: String,
    /// A window of numbered pages around the one being shown -- see `handlers::page_links`.
    pub pages: Vec<crate::handlers::PageNumber>,
    /// The rating options every row's select is built from.
    ///
    /// Built once for the page rather than once per row: a page of rows would otherwise allocate the
    /// same eleven strings two hundred times.
    pub ratings: Vec<Choice>,
    /// The languages this corpus holds, for every row's language select.
    ///
    /// Built once for the page for the reason `ratings` is, and the reason bites much harder here:
    /// the standard has 186 codes, so a full picker in each row of a page would be thousands of
    /// `<option>` elements and a large fraction of a megabyte of markup, for a control most
    /// rows never touch. What a corpus actually holds is a handful, and the row's `more…` option is
    /// what reaches the rest.
    pub languages: Vec<Choice>,
    /// Always empty here. The full picker, which only a row in its language-choosing state draws.
    pub all_languages: Vec<Choice>,
    /// Always false here — the list never draws a row in its language-choosing state.
    pub choosing_language: bool,
    /// Always false here — the list never draws a row in its editing state; only the single-row
    /// fragment does. Present because `song_row.html` is included by both and asks for it.
    pub editing: bool,
    /// Always false here, for the same reason as `editing`: only a single row is ever asked which
    /// favorite it should go in.
    pub picking: bool,
    /// Always empty here. The favorites a picking row offers.
    pub favorites: Vec<PickerFavorite>,
    /// The song most recently sent to the karaoke machine, so its play button can say so.
    pub last_played: Option<String>,
    /// Whether a scan is committing rows underneath this page.
    ///
    /// The total is then a reading rather than a fact — it climbs between one click of *next* and
    /// the next, and rows shift between pages as they are inserted. Both are correct behavior and
    /// neither is guessable from a bare number, which is what made the count look broken.
    pub scanning: bool,
    /// Whether the block is asked to show each song's file name beside its title.
    ///
    /// Rendered as a class on `#rows` rather than passed down to each row: the name is in every
    /// row's markup already, and a row re-rendered on its own — by a route that never sees this
    /// query — is swapped back inside `#rows` and inherits the answer from where it lands.
    pub show_filename: bool,
    /// Whether the block is asked to show what the analysis had to say against each song.
    ///
    /// A class on `#rows` too, and for the reason above word for word: the chips are in every row's
    /// markup already, and a row redrawn on its own has to inherit the answer from where it lands
    /// rather than lose it.
    pub show_warnings: bool,
    /// Which page of rows this is, out of how many, as the pager says it. Set by [`Self::say_range`].
    pub range: String,
}

impl SongRows {
    /// The classes on `#rows`, which are the two view boxes and nothing else.
    ///
    /// **Composed here rather than as two conditionals in the markup.** The second one would have
    /// to know whether the first had already opened the attribute and whether a separating space
    /// was owed, which is a rule about HTML syntax living in a template; a third box later would
    /// have to know it about both.
    ///
    /// Empty when neither is on, which is what the markup tests for: a bare `class=""` on every
    /// page would be an attribute that says nothing.
    pub fn block_class(&self) -> String {
        let mut classes = Vec::new();
        if self.show_filename {
            classes.push("filenames");
        }
        if self.show_warnings {
            classes.push("warnings");
        }
        classes.join(" ")
    }

    /// Which page of rows this is, out of how many, as the pager says it. See [`say_page`].
    ///
    /// **Two sentences rather than a `~` in front of the counts**, because a scan writing rows
    /// underneath makes the total a reading rather than a fact, and a mark a reader has to decode is
    /// not the same as being told.
    pub fn say_range(&mut self, locale: km_locale::Locale, page_size: u32) {
        let key = if self.scanning {
            "songs-range-scanning"
        } else {
            "songs-range"
        };
        self.range = say_page(
            locale,
            key,
            self.offset.into(),
            self.total.into(),
            page_size.into(),
        );
    }
}

/// One row of the browse table, on its own.
///
/// The same markup as a row inside the list — `song_row.html` is included by both — so a row that has
/// just been edited cannot drift from the others around it.
#[derive(Template)]
#[template(path = "song_row_fragment.html")]
pub struct SongRowFragment {
    /// The row.
    pub song: SongRow,
    /// Whether the title and artist are shown as inputs.
    pub editing: bool,
    /// Whether the row is asking which favorite the song should go in.
    pub picking: bool,
    /// The favorites to choose from, when picking, each with what its button offers.
    pub favorites: Vec<PickerFavorite>,
    /// The song most recently sent to the karaoke machine, so its play button can say so.
    pub last_played: Option<String>,
    /// The eleven rating options plus unset, for the row's rating select.
    pub ratings: Vec<Choice>,
    /// The languages this corpus holds, for the row's language select.
    pub languages: Vec<Choice>,
    /// Every language there is, drawn only when `choosing_language` is set.
    pub all_languages: Vec<Choice>,
    /// Whether the row is asking which language, from the whole standard rather than the short list.
    pub choosing_language: bool,
}

impl SongRowFragment {
    /// Builds the fragment for a row, with the rating options its selects need.
    ///
    /// `languages` is what `Db::languages_present` found. A row swapped back on its own has to draw
    /// the same select as the others around it, so it needs the same list — which is why every
    /// route that answers with a row loads it.
    pub fn new(song: SongRow, editing: bool, languages: Vec<Choice>) -> Self {
        Self {
            ratings: rating_choices(),
            song,
            editing,
            picking: false,
            favorites: Vec::new(),
            last_played: None,
            languages,
            all_languages: Vec::new(),
            choosing_language: false,
        }
    }

    /// The same row, turned into the full language picker.
    ///
    /// The third row state, beside editing and picking a favorite, and it exists for the same reason
    /// the short list does: the standard's 186 codes cannot be in every row, but they have to be
    /// reachable from one — otherwise the first song of a language a corpus does not yet hold could
    /// not be classified from the list at all.
    pub fn choosing_language(song: SongRow, present: &[km_kmpkg::Language]) -> Self {
        let current = song.language.clone();
        Self {
            all_languages: Choice::languages(current.as_deref()),
            choosing_language: true,
            ..Self::new(
                song,
                false,
                Choice::languages_in(present, current.as_deref()),
            )
        }
    }

    /// The same row, told which song was last sent to the machine.
    pub fn with_last_played(mut self, last_played: Option<String>) -> Self {
        self.last_played = last_played;
        self
    }

    /// The same row, with the "which favorite?" chooser opened under it.
    ///
    /// A second line in the song's own `<tbody>` rather than a popup beside the row: it needs no
    /// positioning, no script and no second place for the row's identity to live, and the song being
    /// filed stays readable while the choice is made.
    pub fn picking(
        song: SongRow,
        favorites: Vec<FavoriteNode>,
        member_of: Vec<i64>,
        languages: Vec<Choice>,
        locale: km_locale::Locale,
    ) -> Self {
        Self {
            picking: true,
            favorites: PickerFavorite::list(favorites, &member_of, locale),
            ..Self::new(song, false, languages)
        }
    }
}

/// One favorite as the chooser under a row offers it.
///
/// **A view row rather than the list itself**, because the button's tooltip names the list and says
/// which way pressing it goes — one sentence per favorite, and a sentence carrying a value is
/// composed in Rust.
#[derive(Debug, Clone)]
pub struct PickerFavorite {
    /// Row id, which the button posts to.
    pub id: i64,
    /// What the list is called, which the button also shows.
    pub name: String,
    /// Whether this song is already in it.
    pub filed: bool,
    /// What pressing it would do.
    pub title: String,
    /// Whether a line separates this button from the filings before it: set on the first working list
    /// when at least one filing comes first.
    pub rule_before: bool,
}

impl PickerFavorite {
    /// Every favorite, each knowing whether this song is in it: the filings first, then the working
    /// lists, each group in the order it was given.
    ///
    /// **A working list is where a song waits instead of being filed**, so the lists that file it are
    /// the ones read first. A flag on the button rather than a second list, because `song_row.html` is
    /// included by three views and each would need the second field.
    pub fn list(
        favorites: Vec<FavoriteNode>,
        member_of: &[i64],
        locale: km_locale::Locale,
    ) -> Vec<PickerFavorite> {
        let words = crate::words::messages(locale);
        let (filings, working): (Vec<_>, Vec<_>) =
            favorites.into_iter().partition(|node| !node.temporary);
        let filed_count = filings.len();
        filings
            .into_iter()
            .chain(working)
            .enumerate()
            .map(|(index, node)| {
                let filed = member_of.contains(&node.id);
                let key = if filed {
                    "row-take-out-of"
                } else {
                    "row-put-in"
                };
                PickerFavorite {
                    title: words
                        .msg_with(key, &[("name", node.name.as_str().into())])
                        .into_owned(),
                    rule_before: index > 0 && index == filed_count,
                    id: node.id,
                    name: node.name,
                    filed,
                }
            })
            .collect()
    }
}

/// `GET /lyrics`
#[derive(Template)]
#[template(path = "lyric_search.html")]
pub struct LyricSearchPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The hits and their paging.
    pub hits: LyricHits,
    /// Every favorite, for a hit's "which favorite?" chooser.
    pub favorites: Vec<FavoriteNode>,
    /// The words that were typed, echoed back into the box.
    pub q: String,
}

/// `GET /lyrics/hits` — the results alone.
///
/// Carries the whole cast `song_row.html` asks for, because a hit *is* a browse row: the same
/// template draws it, so everything on it works and nothing about it can drift from the songs page.
#[derive(Template)]
#[template(path = "lyric_hits.html")]
pub struct LyricHits {
    /// The hits themselves.
    pub hits: Vec<LyricHit>,
    /// Whether anything was typed at all. An empty box is not a search that found nothing.
    pub searched: bool,
    /// Whether any song's words are in the index yet.
    ///
    /// The difference between the two ways of finding nothing. A database scanned by a version
    /// before lyrics were stored has an empty index and needs a full re-read, not a shorter phrase,
    /// and a page that said "nothing matches" to that would be sending somebody to retype forever.
    pub indexed: bool,
    /// How many songs match in total.
    pub total: u32,
    /// Rows skipped.
    pub offset: u32,
    /// The query string for the previous page, empty when there is none.
    pub previous: String,
    /// The query string for the next page, empty when there is none.
    pub next: String,
    /// The first page, empty when the window already reaches it.
    pub first_page: String,
    /// The last page, empty when the window already reaches it.
    pub last_page: String,
    /// A window of numbered pages around the one being shown, the current one included.
    pub pages: Vec<crate::handlers::PageNumber>,
    /// Which page of hits this is, out of how many, as the pager says it. Set by [`Self::say_range`].
    pub range: String,
    /// The rating options every row's select is built from, made once for the page.
    pub ratings: Vec<Choice>,
    /// The languages this corpus holds, for every row's language select. Made once, like `ratings`.
    pub languages: Vec<Choice>,
    /// Always empty: the list never draws a row in its language-choosing state.
    pub all_languages: Vec<Choice>,
    /// Always false, for the same reason.
    pub choosing_language: bool,
    /// Always false: the list never draws a row in its editing state.
    pub editing: bool,
    /// Always false, for the same reason.
    pub picking: bool,
    /// The favorites a picking row would offer. Always empty here.
    pub favorites: Vec<PickerFavorite>,
    /// The song most recently sent to the karaoke machine.
    pub last_played: Option<String>,
}

impl LyricHits {
    /// Which page of hits this is, out of how many, as the pager says it. See [`say_page`].
    pub fn say_range(&mut self, locale: km_locale::Locale, page_size: u32) {
        self.range = say_page(
            locale,
            "hits-range",
            self.offset.into(),
            self.total.into(),
            page_size.into(),
        );
    }
}

/// `GET /similar`
#[derive(Template)]
#[template(path = "similar.html")]
pub struct SimilarPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The matches.
    pub hits: SimilarHits,
    /// Every favorite, for the "put the ticked songs in" form.
    pub favorites: Vec<FavoriteNode>,
    /// The title searched for, echoed back into its box.
    pub title: String,
    /// The artist searched for, echoed back into its box.
    pub artist: String,
    /// The song the search started from, which heads the list.
    pub from: String,
    /// The narrowing controls, echoed back so they keep their state. Only the suitability, kind,
    /// lyrics and copies fields are drawn.
    pub query: FilterForm,
}

/// `GET /similar/hits` — the matches alone.
///
/// Carries the whole cast `song_row.html` asks for, for [`LyricHits`]'s reason.
#[derive(Template)]
#[template(path = "similar_hits.html")]
pub struct SimilarHits {
    /// The matches, likeliest first.
    pub hits: Vec<SongRow>,
    /// Whether the boxes held a name to search for. Empty boxes are not a search that found nothing.
    pub searched: bool,
    /// The rating options every row's select is built from, made once for the page.
    pub ratings: Vec<Choice>,
    /// The languages this corpus holds, for every row's language select.
    pub languages: Vec<Choice>,
    /// Always empty: the list never draws a row in its language-choosing state.
    pub all_languages: Vec<Choice>,
    /// Always false, for the same reason.
    pub choosing_language: bool,
    /// Always false: the list never draws a row in its editing state.
    pub editing: bool,
    /// Always false, for the same reason.
    pub picking: bool,
    /// The favorites a picking row would offer. Always empty here.
    pub favorites: Vec<PickerFavorite>,
    /// The song most recently sent to the karaoke machine.
    pub last_played: Option<String>,
}

/// `GET /similar-words`
#[derive(Template)]
#[template(path = "similar_words.html")]
pub struct SimilarWordsPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The matches.
    pub hits: SimilarWordsHits,
    /// Every favorite, for the "put the ticked songs in" form.
    pub favorites: Vec<FavoriteNode>,
    /// The song the search started from, which heads the list. The only thing the address carries,
    /// because the subject is that song's whole lyric body and no box could hold it.
    pub from: String,
    /// The narrowing controls, echoed back so they keep their state.
    pub query: FilterForm,
}

/// `GET /similar-words/hits` — the matches alone.
///
/// Carries the whole cast `song_row.html` asks for, for [`LyricHits`]'s reason.
#[derive(Template)]
#[template(path = "similar_words_hits.html")]
pub struct SimilarWordsHits {
    /// The matches, likeliest first, headed by the song searched from.
    pub hits: Vec<SongRow>,
    /// Whether any song's words have been indexed at all. False sends somebody to the scan page
    /// rather than leaving them to conclude their corpus holds nothing like this song.
    pub indexed: bool,
    /// Whether the song searched from has enough words to find another by. False is an instrumental,
    /// or a file whose lyric track carries nothing but the sequencer's card — a different thing to
    /// say from *nothing matched*, and with a different answer.
    pub comparable: bool,
    /// The rating options every row's select is built from, made once for the page.
    pub ratings: Vec<Choice>,
    /// The languages this corpus holds, for every row's language select.
    pub languages: Vec<Choice>,
    /// Always empty: the list never draws a row in its language-choosing state.
    pub all_languages: Vec<Choice>,
    /// Always false, for the same reason.
    pub choosing_language: bool,
    /// Always false: the list never draws a row in its editing state.
    pub editing: bool,
    /// Always false, for the same reason.
    pub picking: bool,
    /// The favorites a picking row would offer. Always empty here.
    pub favorites: Vec<PickerFavorite>,
    /// The song most recently sent to the karaoke machine.
    pub last_played: Option<String>,
}

/// Which page `offset` is on and how many pages `total` rows make, both counting from one.
///
/// **The page count is at least the page being shown**, for the reason `handlers::page_links` clamps
/// its last button: a total carried from before a scan wrote more rows can lag the offset, and
/// *page 21 of 20* reads as a broken page. An empty list is page one of one.
pub fn page_of(offset: u64, total: u64, page_size: u64) -> (u64, u64) {
    let page = offset / page_size + 1;
    (page, total.div_ceil(page_size).max(page))
}

/// A pager's label, *page 18 of 34 (1680 songs)*, said by `key` in `locale`.
///
/// Composed in Rust rather than in the markup because the count of rows chooses a plural, and a
/// plural is arithmetic.
pub fn say_page(
    locale: km_locale::Locale,
    key: &str,
    offset: u64,
    total: u64,
    page_size: u64,
) -> String {
    let n = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let (page, pages) = page_of(offset, total, page_size);
    crate::words::messages(locale)
        .msg_with(
            key,
            &[
                ("page", n(page).into()),
                ("pages", n(pages).into()),
                ("total", n(total).into()),
            ],
        )
        .into_owned()
}

/// The eleven scores, as `<option>` values. Selection is decided in the template against the row's
/// own value, because one list serves the select on every row.
pub fn rating_choices() -> Vec<Choice> {
    Choice::list(
        ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
        None,
    )
}

/// One filter currently narrowing the list, and the link that takes it back off.
///
/// **The bar has fourteen controls and every one of them defaults to *any*, so a set filter looks
/// exactly like an unset one two feet away.** Browsing a corpus of hundreds of thousands of songs, the
/// question that actually gets asked is *why am I seeing so few of these*, and reading fourteen
/// dropdowns to answer it is the wrong shape of work. These say it in one line.
///
/// The folder tag did precisely this for one filter already; this is that idea applied to the rest,
/// and the folder is now one of them rather than a special case in the markup.
#[derive(Debug, Clone)]
pub struct ActiveFilter {
    /// What it narrows to, as a person would say it: `suitability >= 7`, `video only`.
    pub label: String,
    /// The query string with this one filter dropped and paging reset.
    pub remove: String,
}

/// The strip that says what is narrowing the list, rendered on its own so the rows can bring a fresh
/// one back with them.
///
/// A fragment rather than markup inside the page for one reason: the filter bar swaps `#rows` and
/// leaves the rest of the page alone, so anything drawn from the filter that lives *outside* `#rows`
/// describes the page as it was loaded and not as it is. That was a cosmetic lie here and a
/// dangerous one two forms further down — see [`crate::handlers::FilterQuery::from_body`].
#[derive(Template)]
#[template(path = "filter_chips.html")]
pub struct FilterChips {
    /// Every filter currently narrowing the list, each with the link that removes it.
    pub active: Vec<ActiveFilter>,
    /// The query with every narrowing filter dropped but the sort and the file-name toggle kept.
    ///
    /// Those two are properties of the *view*, not of the set of songs, so *clear* leaving them
    /// alone is the difference between clearing a filter and undoing the way somebody set the page
    /// up to work.
    pub cleared: String,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

/// One named filter, with the two sentences its controls say about it by name.
///
/// **A view row rather than the model**, because both sentences name the filter and so have to be
/// composed — and composing them where the rows are read would put a locale in the database layer.
/// `handlers::offerable` is where a saved filter is already adjusted for what the page can show.
#[derive(Debug, Clone)]
pub struct SavedFilterRow {
    /// Row id, which the rename, update and forget routes take.
    pub id: i64,
    /// What somebody called it.
    pub name: String,
    /// The query string with no leading `?`. Empty means the whole corpus.
    pub query: String,
    /// What the update button says it would do, which names the filter it would overwrite.
    pub update_title: String,
    /// What the browser asks before forgetting it, which names the filter it would forget.
    pub forget_confirm: String,
}

impl SavedFilterRow {
    /// A saved filter, worded for the language the page is in.
    pub fn new(filter: crate::model::SavedFilter, locale: km_locale::Locale) -> Self {
        let words = crate::words::messages(locale);
        let name = [("name", filter.name.as_str().into())];
        Self {
            update_title: words.msg_with("saved-update-title", &name).into_owned(),
            forget_confirm: words.msg_with("saved-forget-confirm", &name).into_owned(),
            id: filter.id,
            name: filter.name,
            query: filter.query,
        }
    }
}

/// The strip of filters somebody named, under the bar.
///
/// [`FilterChips`]'s shape and for its reason: it is drawn from the page and replaced out of band by
/// the two routes that change what is in it, so it never goes on saying what was there when the page
/// loaded.
#[derive(Template)]
#[template(path = "saved_filters.html")]
pub struct SavedFilters {
    /// Every saved filter, each with the link that restores it.
    pub filters: Vec<SavedFilterRow>,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
    /// Always false here, and the field is what lets one template draw a chip in both places:
    /// `saved_filter_chip.html` reads it, and an `include` shares this scope. [`SongRows`] carries
    /// `picking` for the same reason.
    pub renaming: bool,
}

/// One named filter on its own, as a chip or as the box that renames it.
#[derive(Template)]
#[template(path = "saved_filter_chip.html")]
pub struct SavedFilterChip {
    /// The filter this chip draws.
    pub filter: SavedFilterRow,
    /// Whether it is open for renaming.
    pub renaming: bool,
}

/// Saving under a name that is already taken.
#[derive(Template)]
#[template(path = "saved_filter_confirm.html")]
pub struct SavedFilterConfirm {
    /// The name being written over.
    pub name: String,
    /// What it would hold afterwards.
    pub query: String,
    /// What it holds now, so the sentence can show both.
    pub replacing: String,
}

/// The filter form's current values, echoed back so the controls keep their state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterForm {
    /// Which language the bar's own words are in.
    ///
    /// **A field rather than an argument to [`Self::initials`]**, because the markup calls that
    /// method and a template has no locale to hand it. English until [`Self::in_language`] says
    /// otherwise, which is what a test that builds a form directly gets.
    pub locale: km_locale::Locale,
    /// Free text.
    pub q: String,
    /// Exactly one artist, echoed back into its box.
    ///
    /// Not put through an enum on the way, unlike the pickers below it: there is no set of valid
    /// artists to normalize against, and the corpus's answer to a name nobody in it goes by is an
    /// empty list with a chip saying whose — which is the honest one.
    pub artist: String,
    /// Which suitabilities the automatic score may hold: `` · `8-10` · `5-7` · `0-4` · a range.
    pub suitability: String,
    /// What the person's own rating must be: `` · `unset` · `set` · a number.
    pub user_score: String,
    /// The chosen initial letter, empty for any.
    pub initial: String,
    /// The folder being browsed, empty for all of them. Its *clear* link is the chip
    /// [`ActiveFilter::remove`] builds, like every other filter's.
    pub folder: String,
    /// Whether a song has to be filed: `` · `in` · `out`.
    pub favorited: String,
    /// The chosen favorite id, as text, empty for any.
    pub favorite: String,
    /// Melody found / abstained / either.
    pub melody: String,
    /// Encoding source.
    pub encoding_source: String,
    /// Lyric granularity.
    pub granularity: String,
    /// `midi`, `video`, or empty for both.
    pub kind: String,
    /// The chosen language: a code, `unset`, `set`, or empty for any.
    pub language: String,
    /// The languages left out, as codes, sorted — what the hidden `language_not` field carries.
    pub language_not: Vec<km_kmpkg::Language>,
    /// How many copies on disk: `1` · `2-10` · `10+`, or empty for any.
    pub copies: String,
    /// How long ago the song was added: `1d` · `7d` · `30d` · `30d+`, or empty for any.
    pub added: String,
    /// `all` to show every version of a recording, or empty to collapse them to one row.
    pub versions: String,
    /// The languages this corpus actually holds, from `Db::languages_present`.
    ///
    /// On the form rather than on the page because two of the three pickers that need it are drawn
    /// from `query.` in the markup, and a second field beside them would be a second thing to
    /// remember to pass.
    pub present: Vec<km_kmpkg::Language>,
    /// The tags narrowing the list, as slugs, sorted.
    pub tags: Vec<String>,
    /// Every tag offerable here: what the corpus holds, plus the suggestions from settings.
    ///
    /// On the form for `present`'s reason — the bar's picker, the bulk control and the datalist all
    /// draw from it.
    pub known_tags: Vec<String>,
    /// Of those, the ones no song here carries — offered as a hint rather than as a fact.
    ///
    /// Drawn differently wherever a tag is drawn, because the two are different claims: one says
    /// *songs are filed under this*, the other says *this is the sort of word to use*. Confusing
    /// them would have somebody filtering by a suggestion and concluding their corpus is empty.
    pub suggested_tags: Vec<String>,
    /// Only songs in no package.
    pub unpackaged: bool,
    /// Whether the list is what has been thrown away rather than the corpus.
    pub deleted: bool,
    /// Whether each row shows its file name beside its title. One of the two boxes in this form
    /// that narrow nothing; they are here because they are set while browsing and have to survive a
    /// page turn.
    pub filename: bool,
    /// Whether each row shows what the analysis had to say against the song. The other one.
    pub warnings: bool,
    /// Sort key.
    pub sort: String,
}

impl FilterForm {
    /// The suitability bands: *any*, `8-10`, `5-7` and `<5`, each knowing whether it is the current
    /// one, and a range where the address names one.
    ///
    /// Built here rather than written out in the markup for the reason [`Self::initials`] is, and
    /// for one this control has of its own: the low band's value and its label differ — `0-4` in the
    /// URL against `<5` on screen — which is precisely the split [`Choice`] exists for, and precisely
    /// the pair that a hand-written `<option>` gets the wrong way round.
    ///
    /// **A range comes last and only while it is in force.** The four bands are what the control
    /// offers, and a fifth standing option would overlap them. What a range still owes the page is a
    /// control agreeing with the rows: a select reading *any* over a narrowed list is the fault
    /// [`SuitabilityFilter`] draws a chip to prevent, one element further down the bar. Picking a
    /// band submits the bar and the extra option goes with it.
    pub fn suitabilities(&self) -> Vec<Choice> {
        let current = SuitabilityFilter::parse(&self.suitability);
        let bands = [
            SuitabilityFilter::Any,
            SuitabilityFilter::High,
            SuitabilityFilter::Middle,
            SuitabilityFilter::Low,
        ];
        bands
            .into_iter()
            .chain(matches!(current, SuitabilityFilter::Range { .. }).then_some(current))
            .map(|band| Choice {
                selected: band == current,
                label: band.label(),
                value: band.as_str(),
            })
            .collect()
    }

    /// Whether the personal-score filter is set to this option.
    ///
    /// A method because askama iterates a list of `&str` and comparing that against a `String` in the
    /// template does not typecheck. Doing it here also keeps the eleven `<option>`s in the markup a
    /// loop rather than eleven copies of the same line.
    pub fn user_score_is(&self, value: &str) -> bool {
        self.user_score == value
    }

    /// Whether the language filter is set to this value, for the `<option>` markup.
    ///
    /// A method rather than a comparison in the template for the same reason the rating one is: the
    /// askama typecheck is what stops a renamed field from becoming a silently unselected option.
    pub fn language_is(&self, value: &str) -> bool {
        self.language == value
    }

    /// The languages the filter's dropdown offers, marked against the current one.
    ///
    /// **Only what this corpus holds.** This select can only ever narrow, so a language nothing is in
    /// is an option whose only possible result is an empty page — and offering 186 of those buries the
    /// three or four that mean anything. The bulk set below is the opposite case and gets both lists;
    /// see [`Self::every_language`].
    pub fn languages(&self) -> Vec<Choice> {
        Choice::languages_in(&self.present, Some(self.language.as_str()))
    }

    /// The tags the bar's picker offers: everything known, minus what is already chosen.
    ///
    /// An option that changes nothing is not a choice, so a tag already on the bar is not offered
    /// again. When this is empty the picker is not drawn at all — which is what an untagged corpus
    /// with no defaults set looks like.
    pub fn tag_choices(&self) -> Vec<String> {
        self.known_tags
            .iter()
            .filter(|tag| !self.tags.contains(tag))
            .cloned()
            .collect()
    }

    /// The chosen tags, comma-joined — what the hidden `tags` field carries.
    pub fn tags_value(&self) -> String {
        self.tags.join(",")
    }

    /// The excluded languages, comma-joined — what the hidden `language_not` field carries.
    pub fn language_not_value(&self) -> String {
        self.language_not
            .iter()
            .map(|language| language.code())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The languages the exclusion picker offers: everything the corpus holds, minus those already
    /// left out.
    ///
    /// Drawn from what is *present* rather than from the whole table, for the reason the positive
    /// picker beside it is: a corpus holds a dozen languages and the standard has 184, and offering
    /// to exclude one no song is in is an option that changes nothing.
    pub fn language_not_choices(&self) -> Vec<Choice> {
        self.present
            .iter()
            .filter(|language| !self.language_not.contains(language))
            .map(|language| Choice {
                value: language.code().to_owned(),
                label: language.name().to_owned(),
                selected: false,
            })
            .collect()
    }

    /// Whether a tag is offered as a hint rather than because a song here carries it.
    pub fn is_suggestion(&self, tag: &str) -> bool {
        self.suggested_tags.iter().any(|held| held == tag)
    }

    /// Marks which of the offered tags are suggestions.
    ///
    /// A separate step from [`crate::handlers::FilterQuery::to_form`] because that function is a
    /// pure function of the query and has no settings to read — the same reason `present` is passed
    /// in rather than looked up there.
    #[must_use]
    pub fn with_hints(mut self, suggested: Vec<String>) -> Self {
        self.suggested_tags = suggested;
        self
    }

    /// The language the bar's own words are in. [`Self::with_hints`]'s twin.
    pub fn in_language(mut self, locale: km_locale::Locale) -> Self {
        self.locale = locale;
        self
    }

    /// The corpus's own languages, for the first group of a picker that *sets* one.
    ///
    /// Nothing is marked selected: these pickers set a value rather than showing one.
    pub fn corpus_languages(&self) -> Vec<Choice> {
        Choice::languages_in(&self.present, None)
    }

    /// Every language there is, for the second group of a picker that sets one.
    ///
    /// The short list above is the answer nine times in ten, but a picker that *only* offered it
    /// could never classify the first Portuguese song in a corpus — the language would have to
    /// already be there to be choosable, which is a picker that cannot be used to fix an empty
    /// column. So the standard is still reachable, one group down, and costs nothing: this list is
    /// rendered once per page rather than once per row.
    pub fn every_language(&self) -> Vec<Choice> {
        Choice::languages(None)
    }

    /// The copies filter's options, marked against the current one.
    pub fn copies_is(&self, value: &str) -> bool {
        self.copies == value
    }

    /// The added-date filter's options, marked against the current one.
    pub fn added_is(&self, value: &str) -> bool {
        self.added == value
    }

    /// Whether every version of a recording is being shown.
    pub fn every_version(&self) -> bool {
        self.versions == "all"
    }

    /// The favorited filter's three options, marked against the current one.
    pub fn favorited_is(&self, value: &str) -> bool {
        self.favorited == value
    }

    /// The alphabet bar: *any*, `A`–`Z`, `0-9` and *symbol*, each knowing whether it is the current
    /// one.
    ///
    /// Built here rather than written out in the markup because twenty-nine hand-written radio
    /// buttons is twenty-nine chances to mistype one, and because "which is selected" is the same
    /// question [`Choice`] already exists to answer.
    ///
    /// **Ten digit buttons collapsed into one, and `#` learned to say what it is.** Nobody browses a
    /// corpus for the songs beginning with 7, and a bar of thirty-eight chips spent a quarter of its
    /// width saying so. The value is still what the query string carries and the label is what the
    /// button reads, which is the split [`Choice`] already has for the language picker.
    pub fn initials(&self) -> Vec<Choice> {
        let current = Initial::parse(&self.initial);
        std::iter::once(Initial::Any)
            .chain(('A'..='Z').map(Initial::Letter))
            .chain([Initial::Digits, Initial::Symbol])
            .map(|initial| Choice {
                selected: initial == current,
                label: initial.label(self.locale),
                value: initial.as_str(),
            })
            .collect()
    }
}

/// The sentences on a song's page that carry a value.
///
/// **One struct rather than a dozen fields**, because they are all the same kind of thing: a count,
/// a channel, a code the file declared. Each is composed where a test can reach it, which is the
/// rule `km_locale::filters` states for anything markup would otherwise assemble.
#[derive(Debug, Clone, Default)]
pub struct SongSaid {
    /// Which witness the file gave for a language nobody chose, or empty where somebody did.
    ///
    /// **`detected`, not `guess`.** The file made a statement and this reads it; a guess is what
    /// the sentence below carries, and one word for the two would make the page unable to say which
    /// of them put a language in the box.
    pub language_detected: String,
    /// What the song's own words read as, where nothing stronger spoke.
    pub language_guessed: String,
    /// What the file's own header declared, where it is not a code this build knows.
    pub language_unknown_code: String,
    /// The four numbers behind the suitability.
    pub suitability_parts: String,
    /// Which channel the melody is on, and how sure the detector was.
    pub melody: String,
    /// What the analysis concludes about drawing the words, named in the *Automatic* option.
    ///
    /// Said whether or not the automatic answer is the one in force, because the option has to read
    /// as a choice somebody can make rather than as a report of what is happening.
    pub lyrics_automatic: String,
    /// How many notes, over how many channels, and how much text.
    pub content: String,
    /// How long the words run, for an MP3+G pair.
    pub cdg_length: String,
    /// How many channels the audio has.
    pub cdg_audio: String,
    /// How many tiles and packets the graphics hold.
    pub cdg_graphics: String,
    /// How many packets use an instruction this build has not implemented, where any do.
    pub cdg_unknown: Option<String>,
    /// The Files heading, which counts the identical copies where there is more than one.
    pub files_heading: String,
    /// The Other versions heading, which counts them where there are any.
    pub versions_heading: String,
}

/// `GET /songs/{id}`
#[derive(Template)]
#[template(path = "song.html")]
pub struct SongPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The song.
    pub song: SongDetail,
    /// Other files that look like this same recording, best first, this one excluded.
    ///
    /// Empty for a song in no cluster, and empty on a corpus where nobody has run the suggestion
    /// pass — the two read the same on the page, deliberately: *no other file looks like this one*
    /// is the honest answer in both cases, and a page that hedged it would be a page that explained
    /// a feature instead of answering a question.
    pub versions: Vec<SongRow>,
    /// Every favorite, for the checkboxes.
    pub favorites: Vec<FavoriteNode>,
    /// Every package, for the "add to" dropdown.
    pub packages: Vec<PackageRow>,
    /// Every volume of every package, sourced ones included, for the replace control.
    pub volumes: Vec<PackageRow>,
    /// Encodings offered in the re-decode dropdown, each with whether it is the current one.
    pub encodings: Vec<Choice>,
    /// Every language, by name, marked against what a person chose — never against what was
    /// detected. Pre-selecting a detected value would turn it into a hand-set one the moment
    /// somebody opened the page and pressed Save, which is the provenance `apply_edits` guards.
    pub languages: Vec<Choice>,
    /// The same picker's first group: only the languages this corpus holds.
    ///
    /// Nothing here is ever marked selected — every code in it appears again in `languages`, and that
    /// copy carries the mark. See the note in `song.html`.
    pub corpus_languages: Vec<Choice>,
    /// Every tag offerable in the box: what the corpus holds, plus the suggestions from settings.
    pub known_tags: Vec<String>,
    /// Of those, the ones no song here carries — marked, so a hint is not read as a fact.
    pub suggested_tags: Vec<String>,
    /// This song's own tags, as the fragment both halves of the editor swap.
    pub tag_list: TagList,
    /// The eleven rating options, each with whether it is the current one.
    pub ratings: Vec<Choice>,
    /// One row per channel, for the Advanced tab. Empty on a song with no MIDI events to correct.
    pub channels: Vec<crate::fixes::ChannelRow>,
    /// Whether the melody radios come back on *no melody*, which no row can say for itself.
    ///
    /// True where somebody said the song has none, and also where detection abstained and nobody
    /// has said otherwise: the radios show the answer in force, and *no channel* is an answer.
    pub melody_is_none: bool,
    /// The YouTube search URL, empty when there is nothing worth searching for.
    pub youtube: String,
    /// The sentences on this page that carry a value.
    pub said: SongSaid,
}

impl SongPage {
    /// Whether a tag in the box is offered as a hint rather than because a song here carries it.
    pub fn is_suggestion(&self, tag: &str) -> bool {
        self.suggested_tags.iter().any(|held| held == tag)
    }

    /// Whether the Advanced tab is offered at all.
    ///
    /// **Here rather than twice in the template**, which is what the label and the pane each need:
    /// a tab whose label is drawn and whose pane is not opens onto nothing, and two copies of a
    /// condition are two chances to change only one.
    ///
    /// Channels *or* words, because the tab holds two unrelated things now. A MIDI song has a
    /// channel table; an UltraStar song has none and still has words somebody may want turned off.
    /// A video or an MP3+G song has neither, so it is offered no tab, which is what it was.
    pub fn shows_advanced(&self) -> bool {
        !self.channels.is_empty() || self.song.kind.draws_words()
    }

    /// Which of the three answers the words control stands on.
    ///
    /// The template compares a string rather than a nested `Option`, which Askama has no graceful
    /// spelling for. `auto` is nobody having said.
    pub fn lyrics_hidden_choice(&self) -> &'static str {
        match self.song.lyrics_hidden {
            Some(true) => "hide",
            Some(false) => "show",
            None => "auto",
        }
    }

    /// The replace control's value for one volume.
    pub fn slot(&self, volume: &PackageRow) -> String {
        crate::handlers::replace_slot(&volume.id, volume.volume)
    }
}

/// `GET /folders`
#[derive(Template)]
#[template(path = "folders.html")]
pub struct FoldersPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The folder being listed, `""` for the root. Ends in `/`.
    pub path: String,
    /// Its immediate children.
    pub folders: Vec<crate::db::FolderNode>,
    /// Every step of `path`, as `(label, path)`, for the breadcrumb.
    pub crumbs: Vec<(String, String)>,
    /// The parent's path, `""` at the root. Empty string at the root means the "up" link is the root
    /// link, which is where it should go.
    pub parent: String,
}

/// One `<option>`: its value and whether it is the one currently in force.
///
/// Precomputed in Rust rather than decided in the template. Askama has no closures and no `*`, so
/// working out "is this the selected one" in the markup means contorting both sides of a comparison
/// until it typechecks — and that logic is easier to read, and to test, here.
#[derive(Debug, Clone)]
pub struct Choice {
    /// The option's value — what the form posts.
    pub value: String,
    /// What the option reads as. The same as `value` for everything but the language picker, where
    /// a person chooses "Portuguese" and the form posts `pt`.
    pub label: String,
    /// Whether it is currently selected.
    pub selected: bool,
}

impl Choice {
    /// Builds the option list for a set of values, marking whichever equals `current`.
    pub fn list<'a>(values: impl IntoIterator<Item = &'a str>, current: Option<&str>) -> Vec<Self> {
        values
            .into_iter()
            .map(|value| Self {
                selected: current == Some(value),
                label: value.to_owned(),
                value: value.to_owned(),
            })
            .collect()
    }

    /// Every language, by name, marking whichever is `current`.
    ///
    /// The picker is built from `km_kmpkg`'s table rather than from a list written out here, so
    /// the vocabulary the tool offers and the vocabulary a package accepts cannot drift apart.
    pub fn languages(current: Option<&str>) -> Vec<Self> {
        Self::from_languages(km_kmpkg::Language::by_name(), current)
    }

    /// Only the languages a corpus actually holds, by name, marking whichever is `current`.
    ///
    /// What every language picker in the tool shows first. 186 codes is right for a table and useless
    /// as a dropdown: a corpus sorted into `Brasil/`, `Ingles/` and `japanese/` has three, and finding
    /// them among the standard is the interaction this removes. The set comes from
    /// [`crate::db::Db::languages_present`].
    ///
    /// **`current` is added when it is not in the set**, which is not a nicety: a hand-typed
    /// `?language=cy` in a corpus with no Welsh song would otherwise leave the select with no option
    /// selected, showing the first language in the list while filtering by another. The chip says what
    /// is really in force either way, but a control that lies about its own value is worse than a long
    /// one.
    pub fn languages_in(present: &[km_kmpkg::Language], current: Option<&str>) -> Vec<Self> {
        let mut languages = present.to_vec();
        if let Some(current) = current
            && let Some(language) = km_kmpkg::Language::parse(current)
            && !languages.iter().any(|held| held.code() == language.code())
        {
            languages.push(language);
        }
        languages.sort_by_key(|language| language.name());
        Self::from_languages(languages, current)
    }

    fn from_languages(languages: Vec<km_kmpkg::Language>, current: Option<&str>) -> Vec<Self> {
        languages
            .into_iter()
            .map(|language| Self {
                selected: current == Some(language.code()),
                label: format!("{}  ({})", language.name(), language.code()),
                value: language.code().to_owned(),
            })
            .collect()
    }
}

/// `GET /songs/{id}/lyrics`
#[derive(Template)]
#[template(path = "lyrics.html")]
pub struct LyricsFragment {
    /// The encoding the text was decoded with.
    pub encoding: String,
    /// How that encoding was chosen.
    pub source: String,
    /// One entry per lyric line: its time and its text.
    pub lines: Vec<(String, String)>,
    /// Set when the file has no lyrics at all.
    pub empty: bool,
    /// The song, so the "pin this encoding" button knows what to write.
    pub song_id: String,
    /// Whether this encoding differs from what is already pinned or detected.
    pub pinnable: bool,
}

/// `GET /songs/{id}/raw`
#[derive(Template)]
#[template(path = "raw.html")]
pub struct RawFragment {
    /// Track, tick, kind and text of every text-bearing meta event.
    pub events: Vec<(usize, u32, String, String)>,
}

/// `GET /duplicates`
#[derive(Template)]
#[template(path = "duplicates.html")]
pub struct DuplicatesPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// What the last grouping pass left behind.
    ///
    /// **The page carries a count and no list.** A real corpus produces thousands of groups, and a
    /// list of thousands is not work anybody does — which is the whole reason the grouping is acted
    /// on where a curator already is rather than reviewed here.
    pub counts: ClusterCounts,
    /// What the grouping pass found, counted: how many groups, and how many songs they hide.
    ///
    /// Two counts in one sentence, so it is composed where a test can reach it.
    pub found: String,
}

/// `GET /favorites`
#[derive(Template)]
#[template(path = "favorites.html")]
pub struct FavoritesPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The lists, pre-rendered so a write can swap an identical table in.
    pub table: FavoritesTable,
}

/// The favorites, as a table that can be swapped on its own.
///
/// **A fragment because all five of this page's writes change it** — making a list, renaming one,
/// setting one aside, tidying one and deleting one — and each answers with a sentence rather than a
/// new page. Without it the list somebody just made is absent and the one they just deleted is still
/// there until they reload, which is not something this tool asks for anywhere else.
#[derive(Template)]
#[template(path = "favorites_table.html")]
pub struct FavoritesTable {
    /// Every favorite, by name, each with what its two destructive buttons ask first.
    pub favorites: Vec<FavoriteRow>,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

/// One favorite list, as the Favorites page draws it.
///
/// **A view row rather than the model**, because Tidy and Delete each ask a question that names the
/// list and, for Tidy, counts what it would drop. Both are sentences and so are composed.
#[derive(Debug, Clone)]
pub struct FavoriteRow {
    /// The list itself.
    pub node: FavoriteNode,
    /// What Tidy asks before dropping the second copies.
    pub tidy_confirm: String,
    /// What Delete asks before taking the list away.
    pub delete_confirm: String,
    /// Whether a rule separates this row from the filings above it: set on the first working list
    /// when at least one filing comes first.
    pub rule_before: bool,
}

impl FavoriteRow {
    /// A list, worded for the language the page is in.
    ///
    /// **`sourcing` is what makes Delete say more than *the songs are not touched*.** A list a
    /// package draws on goes out of that package's sources silently, through the cascade, and the
    /// next sync then takes every song it put there back out — so the packages are named here, in
    /// front of the button, rather than discovered later by somebody wondering where a volume went.
    pub fn new(node: FavoriteNode, sourcing: &[String], locale: km_locale::Locale) -> Self {
        let words = crate::words::messages(locale);
        let mut delete_confirm = words
            .msg_with(
                "favorites-delete-confirm",
                &[("name", node.name.as_str().into())],
            )
            .into_owned();
        if !sourcing.is_empty() {
            delete_confirm.push(' ');
            delete_confirm.push_str(&words.msg_with(
                "favorites-delete-confirm-sourcing",
                &[
                    (
                        "count",
                        i64::try_from(sourcing.len()).unwrap_or(i64::MAX).into(),
                    ),
                    ("packages", sourcing.join(", ").into()),
                ],
            ));
        }
        Self {
            tidy_confirm: words
                .msg_with(
                    "favorites-tidy-confirm",
                    &[
                        ("count", i64::from(node.second_copies).into()),
                        ("name", node.name.as_str().into()),
                    ],
                )
                .into_owned(),
            delete_confirm,
            rule_before: false,
            node,
        }
    }
}

/// `GET /packages`
#[derive(Template)]
#[template(path = "packages.html")]
pub struct PackagesPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// Every package curated here, pre-rendered so a write can swap an identical table in.
    pub table: PackagesTable,
    /// How a song's number inside a package works, which names the highest one.
    pub numbering: String,
}

/// The list of packages, as a table that can be swapped on its own.
///
/// **A fragment because three of this page's writes change it** — making a package, deleting one and
/// opening a built file — and each answers with a sentence rather than a new page. Without it the
/// row somebody just made is absent and the row they just deleted is still there until they reload,
/// which is not something this tool asks for anywhere else.
#[derive(Template)]
#[template(path = "packages_table.html")]
pub struct PackagesTable {
    /// Every package curated here, each with what Delete asks first.
    pub packages: Vec<PackageListRow>,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

/// One package as the list draws it.
///
/// **A view row rather than the model**, because Delete names the package it would take away, and a
/// sentence carrying a value is composed in Rust.
#[derive(Debug, Clone)]
pub struct PackageListRow {
    /// The package itself.
    pub row: PackageRow,
    /// What Delete asks before taking it away.
    pub delete_confirm: String,
    /// The favorites it is sourced from, if any. A mark rather than a column: most packages have
    /// none, and a column of dashes is a column nobody reads.
    pub sources: Vec<String>,
    /// How many volumes it has, said only when there are two or more.
    pub volumes: Option<String>,
}

impl PackageListRow {
    /// A package, worded for the language the page is in.
    pub fn new(row: PackageRow, sources: Vec<String>, locale: km_locale::Locale) -> Self {
        Self {
            volumes: (row.volumes > 1).then(|| {
                crate::words::messages(locale)
                    .msg_with(
                        "packages-volumes",
                        &[("count", i64::from(row.volumes).into())],
                    )
                    .into_owned()
            }),
            delete_confirm: crate::words::messages(locale)
                .msg_with("packages-delete-confirm", &[("id", row.id.as_str().into())])
                .into_owned(),
            row,
            sources,
        }
    }
}

/// The links that pick which volume a package's page shows.
///
/// **A fragment because a sync can start a volume**, and a strip left as it was drawn would go on
/// hiding the volume the sentence beside it just announced.
#[derive(Template)]
#[template(path = "package_volumes.html")]
pub struct VolumeStrip {
    /// Which package.
    pub package_id: String,
    /// One link per volume, or none while the package has a single volume.
    pub tabs: Vec<VolumeTab>,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

impl VolumeStrip {
    /// The strip for a package's volumes, marking the one shown.
    pub fn new(
        package_id: String,
        volumes: &[PackageRow],
        current: u32,
        oob: bool,
        locale: km_locale::Locale,
    ) -> Self {
        Self {
            package_id,
            tabs: VolumeTab::strip(volumes, current, locale),
            oob,
        }
    }
}

/// One volume in the strip at the top of a package's page.
#[derive(Debug, Clone)]
pub struct VolumeTab {
    /// Which volume, from 1.
    pub number: u32,
    /// What the link says: the volume and how many songs it holds.
    pub label: String,
    /// Whether the page is showing this one.
    pub current: bool,
}

impl VolumeTab {
    /// The strip for a package, or nothing when it has a single volume.
    ///
    /// **Nothing, rather than one tab**, because a package that never outgrew 999 songs is what a
    /// package has always been, and a strip naming its only volume says a thing the curator has no
    /// use for.
    pub fn strip(volumes: &[PackageRow], current: u32, locale: km_locale::Locale) -> Vec<Self> {
        if volumes.len() < 2 {
            return Vec::new();
        }
        let words = crate::words::messages(locale);
        volumes
            .iter()
            .map(|volume| Self {
                number: volume.volume,
                label: words
                    .msg_with(
                        "package-volume-tab",
                        &[
                            ("number", i64::from(volume.volume).into()),
                            ("count", i64::from(volume.song_count).into()),
                        ],
                    )
                    .into_owned(),
                current: volume.volume == current,
            })
            .collect()
    }
}
/// `GET /packages/{id}`
#[derive(Template)]
#[template(path = "package.html")]
pub struct PackagePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The package, seen through the volume the Details tab shows.
    pub package: PackageRow,
    /// The strip that picks a volume, pre-rendered so a sync that starts one can swap it in.
    pub volume_strip: VolumeStrip,
    /// What it holds, pre-rendered so a sync can swap an identical table in.
    pub members: MembersTable,
    /// The favorites it may draw from and the Sync button, pre-rendered for the same reason.
    pub sourcing: SourcingPanel,
    /// Those same lists as chips over the song list, so the rows say what decides them.
    pub chips: SourceChips,
    /// The languages this corpus holds, for the first group of the default-language picker.
    pub corpus_languages: Vec<Choice>,
    /// Every language there is, for the second group.
    pub every_language: Vec<Choice>,
    /// The re-flow form, pre-rendered so saving a new first number can swap an identical one in.
    pub renumber: RenumberForm,
    /// The Build tab, pre-rendered so picking another volume in it can swap it whole.
    pub build: BuildPane,
}

/// The Build tab: which volume, its version, where the file goes, its description, and the install.
///
/// **A fragment with a volume picker of its own**, because building is done one file at a time and
/// the volume being built is a choice made on this tab rather than one carried over from the song
/// list on Details. Picking another volume swaps the whole tab, since the version, both default paths,
/// the raise label and the install button all belong to the volume.
#[derive(Template)]
#[template(path = "package_build.html")]
pub struct BuildPane {
    /// The package, seen through the volume this tab builds.
    pub package: PackageRow,
    /// Every volume, for the picker. Empty while the package has one, and the picker is then not
    /// drawn.
    pub volumes: Vec<VolumeTab>,
    /// The folder both files are written into by default: the corpus's data folder.
    pub folder: String,
    /// The name the build writes by default, without its folder.
    pub out_file: String,
    /// The name the description is written under by default, without its folder.
    pub spec_file: String,
    /// Whether a build raises the patch number first.
    pub raise_version: bool,
    /// What the label beside the raise-the-version box says. See [`raise_version_says`].
    pub raise_version_label: String,
    /// The install form, pre-rendered so a finished build can swap an identical one in.
    pub install: InstallForm,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

impl BuildPane {
    /// The tab for one volume, with its defaults worked out and its words chosen.
    pub fn new(
        root: &std::path::Path,
        package: PackageRow,
        volumes: &[PackageRow],
        raise_version: bool,
        oob: bool,
        locale: km_locale::Locale,
    ) -> Self {
        Self {
            volumes: VolumeTab::strip(volumes, package.volume, locale),
            folder: crate::db::data_dir(root).display().to_string(),
            out_file: crate::build::file_name(&crate::build::default_out_path(
                root,
                &package,
                raise_version,
            )),
            spec_file: crate::build::file_name(&crate::build::default_spec_path(root, &package)),
            raise_version,
            raise_version_label: raise_version_says(&package, locale),
            install: InstallForm {
                package_id: package.id.clone(),
                volume: package.volume,
                built: package.built_at.is_some(),
                oob: false,
            },
            package,
            oob,
        }
    }
}

/// The favorites a package draws its songs from, and the button that makes it agree with them.
///
/// **The lists it reads are a table, and the rest are a picker.** A corpus carries lists in the
/// dozens, so a box per favorite wraps into a paragraph where a name, its count and its mark each
/// land on a different line from the box they belong to — and the question *which of these does this
/// package read* is answered by reading the whole paragraph. Four sources in four rows answer it at
/// a glance, and adding a fifth is a name picked out of a select.
///
/// **One fragment, because the two halves and the button are one subject**: adding a list moves a
/// name from the picker into the table and changes the number the button names. Swapping the panel
/// is [`RenumberForm`]'s reason for being a fragment, over a control that changes with it rather
/// than beside it.
#[derive(Template)]
#[template(path = "package_sourcing.html")]
pub struct SourcingPanel {
    /// Which package.
    pub package_id: String,
    /// The lists this package reads, in the order the Favorites page shows them.
    pub sources: Vec<SourceRow>,
    /// The lists it could read and does not, for the picker. Working lists are not among them.
    pub choices: Vec<SourceRow>,
    /// The button, which names how many lists the sync would read. Empty when there are none.
    ///
    /// Composed rather than assembled in the markup, because it carries a value — the rule
    /// `km_locale::filters` states, and the one that keeps a sentence somewhere a test can read it.
    pub button: String,
    /// Whether this corpus has any favorite at all, which picks between *add one* and *make one*.
    pub any_favorites: bool,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

impl SourcingPanel {
    /// The panel, split into what the package reads and what it could, with the button worded.
    ///
    /// **One constructor, because the page and both writes build it** — each write swaps an
    /// identical copy out of band, and three spellings of *which lists does this package read* is
    /// two that stop agreeing with the first.
    ///
    /// **A working list is offered to no package**, so it is absent from the picker. One already
    /// among the sources stays in the table and is marked there: the flag is set on the Favorites
    /// page, so a list can become a working one after a package began reading it, and a row that
    /// quietly disappeared would leave a package drawing on a list its own page did not show.
    pub fn new(
        package_id: String,
        favorites: Vec<crate::model::FavoriteNode>,
        chosen: &[(i64, String, bool)],
        oob: bool,
        locale: km_locale::Locale,
    ) -> Self {
        let any_favorites = !favorites.is_empty();
        let (sources, choices): (Vec<SourceRow>, Vec<SourceRow>) = favorites
            .into_iter()
            .map(|node| SourceRow {
                id: node.id,
                name: node.name,
                song_count: node.song_count,
                temporary: node.temporary,
            })
            .partition(|row| chosen.iter().any(|(id, _, _)| *id == row.id));
        let count = u32::try_from(sources.len()).unwrap_or(u32::MAX);
        Self {
            // Absent rather than disabled when there are no sources: there is nothing to tell
            // somebody about a package nobody has given a list to, and a line saying what to do is
            // worth more than a control answering *no*.
            button: if count == 0 {
                String::new()
            } else {
                crate::words::messages(locale)
                    .msg_with("sync-button", &[("count", i64::from(count).into())])
                    .into_owned()
            },
            choices: choices.into_iter().filter(|row| !row.temporary).collect(),
            package_id,
            sources,
            any_favorites,
            oob,
        }
    }
}

/// The lists a package draws on, drawn over the songs they decide.
///
/// **A fragment because the Sources tab changes it from another pane.** Adding a list has to reach
/// the chips as well as the panel that was pressed, and neither is where the button aimed.
#[derive(Template)]
#[template(path = "package_source_chips.html")]
pub struct SourceChips {
    /// The lists, in the order the Favorites page shows them.
    pub sources: Vec<SourceRow>,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

/// What a write on the sources panel puts back: the panel, and the chips on the other tab.
///
/// **One template because a response has one body**, and both halves are out-of-band swaps rather
/// than the thing the button aimed at — which is what [`with_toast`] wants when the sentence itself
/// is going to the corner.
#[derive(Template)]
#[template(path = "package_sourcing_swap.html")]
pub struct SourcingSwap {
    /// The panel that was pressed.
    pub panel: SourcingPanel,
    /// The chips over the song list, which name the same lists.
    pub chips: SourceChips,
}

impl SourcingSwap {
    /// Both halves, from the one answer.
    ///
    /// **The chips take the panel's own list**, so the two cannot come to disagree about which
    /// lists a package reads — the thing that would otherwise go wrong the first time one of them
    /// learned to filter and the other did not.
    pub fn new(
        package_id: String,
        favorites: Vec<crate::model::FavoriteNode>,
        chosen: &[(i64, String, bool)],
        locale: km_locale::Locale,
    ) -> Self {
        let panel = SourcingPanel::new(package_id, favorites, chosen, true, locale);
        Self {
            chips: SourceChips {
                sources: panel.sources.clone(),
                oob: true,
            },
            panel,
        }
    }
}

/// One favorite, as the sources panel draws it in either half.
#[derive(Debug, Clone)]
pub struct SourceRow {
    /// The list.
    pub id: i64,
    /// What it is called.
    pub name: String,
    /// How many songs it holds, so a source can be judged before it is added.
    pub song_count: u32,
    /// Whether it is a working list rather than a filing. Never true in the picker, and marked in
    /// the table, where it means a list somebody set aside after a package began reading it.
    pub temporary: bool,
}

/// What a package holds, as a table that can be swapped on its own.
///
/// **A fragment because a sync rewrites it.** Every other write on the package page touches one row
/// or one number, and the person who pressed it is looking at what changed; a sync adds and removes
/// many at once, and a member list left on screen would contradict the sentence beside it.
#[derive(Template)]
#[template(path = "package_members.html")]
pub struct MembersTable {
    /// Which package.
    pub package_id: String,
    /// Which of its volumes the table shows.
    pub volume: u32,
    /// What it holds, in number order.
    pub members: Vec<PackageMember>,
    /// What the member count says: how many, out of how many a package may hold.
    pub member_count: String,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

impl MembersTable {
    /// The table, with its count worded.
    pub fn new(
        package_id: String,
        volume: u32,
        members: Vec<PackageMember>,
        oob: bool,
        locale: km_locale::Locale,
    ) -> Self {
        Self {
            member_count: crate::words::messages(locale)
                .msg_with(
                    "package-member-count",
                    &[
                        (
                            "count",
                            i64::try_from(members.len()).unwrap_or(i64::MAX).into(),
                        ),
                        ("highest", i64::from(km_songcode::MAX_SLOT).into()),
                    ],
                )
                .into_owned(),
            package_id,
            volume,
            members,
            oob,
        }
    }
}

/// `POST /packages/{id}/sync` before it is confirmed: what it would do, and to which lists.
///
/// **Modeled on [`PackageAddConfirm`], and it carries the half that one has no word for.** An add
/// only adds, so its question is *how many, and is there room*; a sync also takes songs out, and the
/// number leaving is the one somebody must read before pressing rather than in the sentence
/// afterwards.
#[derive(Template)]
#[template(path = "package_sync_confirm.html")]
pub struct PackageSyncConfirm {
    /// Which package.
    pub package_id: String,
    /// What it is called.
    pub name: String,
    /// The lists the union is read from, as chips — the job `filters` does in [`PackageAddConfirm`],
    /// which is making a set legible before it is written. A working list is marked.
    pub sources: Vec<SourceChip>,
    /// *12 songs go in*, composed because the word follows the number.
    pub adding: String,
    /// *3 songs come out*, which no other confirmation here has to say.
    pub removing: String,
    /// *200 stay where they are*.
    pub keeping: String,
    /// Shown only when every volume together has fewer free numbers than would go in, and says how
    /// many volumes the sync would start.
    pub new_volumes: Option<String>,
}

/// `POST /packages/{id}/replace` before it is confirmed: which song leaves the number, and which lists
/// change with it.
///
/// **It names the song leaving**, because a number typed from memory is the likeliest mistake, and
/// the song it points at is what somebody recognises.
#[derive(Template)]
#[template(path = "package_replace_confirm.html")]
pub struct PackageReplaceConfirm {
    /// Which package and volume, as [`crate::handlers::replace_slot`] spells them.
    pub slot: String,
    /// The number being given to another song.
    pub number: u32,
    /// The song taking the number.
    pub song_id: String,
    /// *Number 12 in Brasil is X. Put Y in its place?*, composed because it carries values.
    pub question: String,
    /// The lists that change with it, said only for a package that follows some.
    pub favorites: Option<String>,
}

/// One source list, as the sync confirmation names it.
#[derive(Debug, Clone)]
pub struct SourceChip {
    /// What the list is called.
    pub name: String,
    /// Whether it is a working list, which colors the chip.
    pub temporary: bool,
}

/// The button that re-flows a package's numbers from its first one.
///
/// **A fragment for [`InstallForm`]'s reason, and a sharper case of it**: the button names the number
/// it will start from, and saving that number swaps only the message beside the form — so a button
/// reading *from 999* would go on saying so while pressing it re-flowed from 1. A control that names
/// a value it will not use is worse than one that names none.
#[derive(Template)]
#[template(path = "renumber_form.html")]
pub struct RenumberForm {
    /// Which package.
    pub package_id: String,
    /// Which of its volumes a re-flow moves.
    pub volume: u32,
    /// The button, which names the number the re-flow would start from.
    ///
    /// Composed rather than assembled in the markup, because it carries a value — the rule
    /// `km_locale::filters` states, and the one that keeps a sentence somewhere a test can read it.
    pub button: String,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

impl RenumberForm {
    /// The form, with its button worded for the number it names.
    pub fn new(
        package_id: String,
        volume: u32,
        start_number: u32,
        oob: bool,
        locale: km_locale::Locale,
    ) -> Self {
        Self {
            package_id,
            volume,
            button: crate::words::messages(locale)
                .msg_with(
                    "renumber-button",
                    &[("start", i64::from(start_number).into())],
                )
                .into_owned(),
            oob,
        }
    }
}

/// The button that sends a built package to the running machine.
///
/// **A fragment rather than markup inside the page**, for the reason [`FilterChips`] is one: it
/// describes something that changes without it being re-rendered. Its `disabled` comes from
/// `built_at`, which is read when the page is drawn, and a build swaps only `#build-progress` — so
/// finishing a build left the one button for installing what it had just written grayed out, under a
/// message saying the file was there, until the page was reloaded.
#[derive(Template)]
#[template(path = "install_form.html")]
pub struct InstallForm {
    /// Which package.
    pub package_id: String,
    /// Which of its volumes the button sends.
    pub volume: u32,
    /// Whether there is a built file to install.
    pub built: bool,
    /// Whether this copy is the out-of-band one that replaces the copy already on the page.
    pub oob: bool,
}

impl PackagePage {
    /// Whether a language code is the package's current default.
    ///
    /// A method rather than marking the [`Choice`]s, because the same code appears in both groups of
    /// the picker and only one of the two may carry `selected` — a duplicate would leave the browser
    /// choosing which to show. The `every language` group carries the mark, so exactly one does; the
    /// short list above it is a convenience and never the authority. Same arrangement as the row
    /// editor on the Songs page.
    pub fn is_default_language(&self, code: &str) -> bool {
        self.package.default_language.as_deref() == Some(code)
    }
}

/// `GET /scan`
#[derive(Template)]
#[template(path = "scan.html")]
pub struct ScanPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// Where the last or current run has got to.
    pub progress: ProgressView,
    /// Whether a run is going on now.
    pub running: bool,
    /// When the last run finished.
    pub last_scan: Option<String>,
    /// Files that did not parse, by reason.
    pub failures: Vec<FailureRow>,
    /// The reasons somebody has accepted, for the line offering them back.
    pub dismissed: Vec<FailureRow>,
    /// How many reasons have been accepted, counted. `failures.html` is included here and asks for
    /// it, which is the same arrangement `song_row.html`'s shared fields already have.
    pub dismissed_count: String,
    /// How many songs an older analysis decided, worded, or `None` where none were.
    ///
    /// Worded here rather than counted on the page, because the sentence is a plural that two
    /// languages disagree about the shape of, and a template has no arithmetic to choose with.
    pub stale: Option<String>,
}

/// A navigation that could not be answered, drawn as a page.
///
/// **The page a refusal becomes when nothing else can show it.** An htmx request carries its refusal
/// back to a page that is still on the screen, and `static/ui.js` raises a toast over it. A
/// navigation has no such page: the browser throws the old one away before the answer arrives, so a
/// refusal answered as text is the whole of what is left. In this tool's own window — a webview with
/// no address bar, no Back and no reload — that is a dead end with nothing on it to press.
///
/// So the refusal is drawn inside the ordinary layout, which carries the nav, and the way out is
/// whichever tab you want. See `handlers::failed_page` for which requests get this and which keep
/// the sentence.
#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorPage {
    /// Page chrome, built by [`Chrome::bare`] because the corpus is what refused.
    pub chrome: Chrome,
    /// What went wrong, as `DbError::say` worded it.
    pub said: String,
    /// Whether the page asks the browser to come back by itself.
    ///
    /// Only where the corpus was busy, which is the one refusal that means *ask again*. A song that
    /// is not there would reload for ever and find the same nothing.
    pub retries: bool,
    /// Whether a scan is going on, which is what the progress panel is drawn for.
    pub running: bool,
    /// Where that scan has got to. Read from memory, so it answers while the corpus does not.
    pub progress: ProgressView,
}

/// The failures panel, redrawn when a reason is removed or restored.
///
/// **A fragment rather than markup inside the page**, for [`FilterChips`]'s reason: removing a
/// reason changes both halves of the panel at once — a row leaves the table and joins the line
/// below it — and a swap narrow enough to move only one of them would leave the two disagreeing
/// about what had just happened.
#[derive(Template)]
#[template(path = "failures.html")]
pub struct FailuresPanel {
    /// Files that did not parse, by reason.
    pub failures: Vec<FailureRow>,
    /// The reasons somebody has accepted.
    pub dismissed: Vec<FailureRow>,
    /// How many reasons have been accepted, counted.
    pub dismissed_count: String,
}

/// One reason files did not parse, as the panel draws it.
///
/// **A view row rather than the tally**, because the Remove button's tooltip names the count and so
/// has to be composed. The reason itself is still a key: `{{ reason|t }}` spends it, the `t` filter
/// taking any `&str` — which is how `km-admin`'s job phases reach a page too.
#[derive(Debug, Clone)]
pub struct FailureRow {
    /// The stored `scan_status`, which is what a Remove or a Restore names.
    pub status: String,
    /// The catalog key for the sentence in the Reason column.
    pub reason: String,
    /// How many files failed this way.
    pub count: u32,
    /// One of them, so the reason has something concrete beside it.
    pub example: String,
    /// What Remove says it would accept, which names the count.
    pub accept_title: String,
}

impl FailureRow {
    /// A tally, worded for the language the page is in.
    pub fn new(tally: crate::db::FailureTally, locale: km_locale::Locale) -> Self {
        Self {
            accept_title: crate::words::messages(locale)
                .msg_with(
                    "failures-remove-title",
                    &[("count", i64::from(tally.count).into())],
                )
                .into_owned(),
            status: tally.status,
            reason: tally.reason,
            count: tally.count,
            example: tally.example,
        }
    }
}

/// `GET /scan/progress` — polled while a run is going.
#[derive(Template)]
#[template(path = "progress.html")]
pub struct ProgressFragment {
    /// Where the run has got to.
    pub progress: ProgressView,
    /// Whether a run is going on now.
    pub running: bool,
}

/// `POST /packages/{id}/build` and `GET /packages/{id}/build/progress` — the same fragment.
///
/// One template for starting and for polling, the arrangement `progress.html` already uses: the POST
/// that starts a build renders the first frame, and every frame after it replaces itself. The
/// polling attributes are emitted only while `running`, so the last frame stops the loop by being
/// the last frame rather than by anything having to switch it off.
#[derive(Template)]
#[template(path = "build_progress.html")]
pub struct BuildProgressFragment {
    /// Which package's page this is, for the poll's own URL.
    pub package_id: String,
    /// Which of its volumes, for the same URL.
    pub volume: u32,
    /// Where the build has got to, if there has been one.
    pub progress: Option<crate::build::BuildProgressView>,
    /// Whether a build is going on now.
    pub running: bool,
    /// What to say when it is over, rendered by the same code the synchronous build used.
    pub message: Option<String>,
    /// Whether that message is a refusal rather than a result.
    pub failed: bool,
    /// How many songs are done, out of how many.
    pub done_said: String,
    /// How many are in the package so far.
    pub written_said: String,
    /// How many were left out.
    pub skipped_said: String,
    /// How far through re-encoding the video being re-encoded now.
    pub encoding_said: String,
    /// Which volume a run over every volume is on, or empty for a build of one.
    pub volume_said: String,
}

impl BuildProgressFragment {
    /// The fragment, with every count in it worded.
    ///
    /// A constructor because the four sentences all read the same snapshot, and a caller assembling
    /// them one at a time is a caller that can word three and forget the fourth.
    pub fn new(
        package_id: String,
        volume: u32,
        progress: Option<crate::build::BuildProgressView>,
        running: bool,
        message: Option<String>,
        failed: bool,
        locale: km_locale::Locale,
    ) -> Self {
        let words = crate::words::messages(locale);
        let n = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
        let mut progress = progress;
        let mut said = (String::new(), String::new(), String::new(), String::new());
        let volume_said = progress
            .as_ref()
            .filter(|view| view.volume == 0 && view.building_volume > 0)
            .map(|view| {
                words
                    .msg_with(
                        "build-volume-now",
                        &[("number", i64::from(view.building_volume).into())],
                    )
                    .into_owned()
            })
            .unwrap_or_default();
        if let Some(view) = progress.as_mut() {
            view.say_phase(locale);
            said.0 = words
                .msg_with(
                    "build-done",
                    &[
                        ("done", n(view.done).into()),
                        ("total", n(view.total).into()),
                        ("percent", n(view.percent).into()),
                    ],
                )
                .into_owned();
            said.1 = words
                .msg_with("build-written", &[("count", n(view.written).into())])
                .into_owned();
            said.2 = words
                .msg_with("build-skipped", &[("count", n(view.skipped).into())])
                .into_owned();
            said.3 = words
                .msg_with(
                    "build-encoding",
                    &[("percent", n(view.encoding.unwrap_or(0)).into())],
                )
                .into_owned();
        }
        Self {
            package_id,
            volume,
            progress,
            running,
            message,
            failed,
            done_said: said.0,
            written_said: said.1,
            skipped_said: said.2,
            encoding_said: said.3,
            volume_said,
        }
    }
}

/// `GET /open` — the folder picker.
///
/// The one page in the tool that renders without a database, and therefore the one that carries no
/// [`Chrome`]: there are no counts to show and no tab it belongs to, because the nav goes to pages
/// that need a folder. Its own small header instead.
#[derive(Template)]
#[template(path = "open.html")]
pub struct OpenPage {
    /// Which language this page is drawn in, for `<html lang>`.
    ///
    /// Its own field because this page has its own shell rather than extending `layout.html`: it is
    /// drawn before there is a workspace to build a [`Chrome`] from.
    pub locale: &'static str,
    /// The folder being opened, if one is.
    ///
    /// **The picker has to show a job it did not start**, because two of the three ways of starting
    /// one happen before any page exists: the startup reopen of last time's folder, and a
    /// double-clicked `.kmbuild` on macOS. Without this the page rendered as though nothing were
    /// happening, and clicking the folder it was already loading answered `already opening …`.
    pub opening: Option<crate::server::OpeningView>,
    /// Folders curated before, newest first.
    pub recent: Vec<RecentView>,
    /// Where the browser would start, if it is asked to start.
    ///
    /// **The path, not the listing, and that is the whole of this page's cost.** The browser used to
    /// be drawn open, so every arrival read a directory — home, or the corpus's own folder, which on
    /// a working machine is a folder of hundreds of thousands of files — and put the whole of it on
    /// screen underneath the two lists somebody actually came here for. Now the section is a button
    /// and nothing is read until it is pressed. This field is kept because the `Or type a path` box
    /// uses it as its placeholder, which is a string and never needed the read.
    pub start: Option<String>,
    /// The folder open now, if the page was reached from one.
    pub current: Option<String>,
    /// Whether the page is in this tool's own window rather than in a browser.
    ///
    /// The same field [`Chrome`] carries and for the same reason — this page has no `Chrome`,
    /// because a nav of eight entries all needing a folder is exactly what it cannot draw. It picks
    /// the same one of two buttons: Quit in a browser, where there is no X that stops anything, and
    /// *Open in browser* in a window, where closing the window already means quitting.
    pub windowed: bool,
}

impl OpenPage {
    /// Whether a folder is being opened right now.
    ///
    /// **What this hides is the whole picker**, and the bug it fixes is that a tool starting up
    /// correctly looked broken. Reopening last time's folder lands on this page — there is no
    /// workspace yet, so `require_workspace` sends every route here — and the page drew the progress
    /// line *and*, underneath it, the Recent list, the Browse button and the type-a-path box. For the
    /// whole ten seconds of a large corpus's open, the thing on screen was the folder chooser, which
    /// is what this program shows when it has failed to open anything.
    ///
    /// It was never a race: `begin_open` fills the slot before `axum::serve` is reachable, so
    /// [`Self::opening`] is never transiently `None`. It was a template drawing two states at once.
    ///
    /// A *finished* job does not count. Success redirects and failure needs the picker back, which is
    /// the same answer: show it.
    pub fn opening_now(&self) -> bool {
        self.opening.as_ref().is_some_and(|job| !job.finished)
    }
}

/// One row of the recent list, flattened for the template.
///
/// A struct of its own rather than the stored [`crate::recent::Entry`], because the template needs
/// two things the file does not hold: the path as a displayable string, and whether it is still
/// there. A folder on a drive that is not plugged in is **dimmed and kept**, never dropped — an
/// external disk being unplugged is not a reason to forget a year of curation.
#[derive(Debug, Clone)]
pub struct RecentView {
    /// Where it is.
    pub path: String,
    /// The last segment, which is what a person recognizes.
    pub name: String,
    /// Songs indexed when it was last open.
    pub songs: u32,
    /// Files indexed when it was last open.
    pub files: u32,
    /// Whether it is reachable now.
    pub present: bool,
    /// What is in it, as far as this tool cares.
    ///
    /// **The same enum the folder browser reads and the same one the startup reopen tests.**
    /// `folder_to_reopen` skips anything that is not [`Indexed::Yes`], so a row asking a question of
    /// its own could offer a folder as the obvious thing to press that the reopen would silently
    /// pass over. Reading one enum in both places is what stops the two drifting.
    ///
    /// [`Indexed::No`] means two different things depending on [`Self::present`] — nothing there, or
    /// a folder with nothing in it — which is why that flag stays. [`Self::openable`] and
    /// [`Self::adoptable`] combine the pair so no template has to.
    pub indexed: crate::browse::Indexed,
    /// What the row's counts say, worded rather than counted.
    ///
    /// Two plurals over two numbers, so the sentence is composed where a test can reach it.
    pub counts: String,
}

impl RecentView {
    /// Words the row's two counts, in the language the page is being drawn in.
    pub fn say_counts(&mut self, locale: km_locale::Locale) {
        self.counts = crate::words::messages(locale)
            .msg_with(
                "open-recent-counts",
                &[
                    ("songs", i64::from(self.songs).into()),
                    ("files", i64::from(self.files).into()),
                ],
            )
            .into_owned();
    }

    /// Whether pressing this row could actually open something.
    ///
    /// A row that is not this is drawn as text rather than as a button: an action whose only outcome
    /// is a refusal is worse than no action, because the refusal arrives after the press and the row
    /// still looks the same afterwards.
    pub fn openable(&self) -> bool {
        self.present && self.indexed.openable()
    }

    /// Whether the folder is there but holds nothing this tool can open.
    ///
    /// A state that needs words of its own. Not the same as *not found*: the songs are still on the
    /// disk and only the curation went, so what is lost is the tags, the packages and the favorites
    /// — which is a different sentence from a drive that is unplugged.
    pub fn unindexed(&self) -> bool {
        self.present && !self.indexed.openable()
    }
}

/// `GET /open/list` — the directory listing, swapped in as the browser walks.
#[derive(Template)]
#[template(path = "open_list.html")]
pub struct OpenListing {
    /// Where we are and what is under it.
    pub listing: crate::browse::Listing,
}

/// `GET /open/list?rows=1` — the folders and their pager, without the crumbs or the filter box.
///
/// **The half that narrowing and paging replace.** Swapping the whole listing for either would take
/// the search box with it, and a box replaced four hundred milliseconds after a keystroke is a box
/// that cannot be typed into. Same arrangement as `#filters` and `#rows` on the browse page.
#[derive(Template)]
#[template(path = "open_folders.html")]
pub struct OpenFolders {
    /// Where we are and what is under it.
    pub listing: crate::browse::Listing,
}

/// `GET /open/progress` — how the folder being opened is getting on.
#[derive(Template)]
#[template(path = "open_progress.html")]
pub struct OpenProgress {
    /// The job, if there is one.
    pub opening: Option<crate::server::OpeningView>,
    /// Whether a folder is open now, which is what the page reloads on.
    pub open: bool,
    /// What the line under the bar says: which folder, which phase, and how long so far.
    ///
    /// Three values in one sentence, so it is composed in Rust. Empty when there is no job.
    pub said: String,
    /// How far through the running rung, beside its name on the checklist.
    ///
    /// `None` for a rung with nothing inside it to count, which is nine of the eleven. Worded here
    /// rather than in the template for the reason `said` is: three numbers in one line, and the
    /// snapshot it comes from was taken on a thread with no language in reach.
    pub count_said: Option<String>,
    /// Where the rung an open failed in sits, or `usize::MAX` where none did.
    ///
    /// **What tells the two meanings of a dash apart.** A rung before the failure is one the open
    /// climbed past because there was nothing to do; a rung after it is one the open never reached.
    /// A number rather than an `Option` because the template compares it and has no `unwrap`.
    pub failed_at: usize,
}

impl OpenProgress {
    /// The fragment, with its line worded for the page asking.
    pub fn new(
        opening: Option<crate::server::OpeningView>,
        open: bool,
        locale: km_locale::Locale,
    ) -> Self {
        let words = crate::words::messages(locale);
        let said = opening.as_ref().map_or_else(String::new, |job| {
            words
                .msg_with(
                    "open-progress",
                    &[
                        ("root", job.root.as_str().into()),
                        ("phase", job.phase.say(locale).into()),
                        (
                            "seconds",
                            i64::try_from(job.elapsed_secs).unwrap_or(i64::MAX).into(),
                        ),
                    ],
                )
                .into_owned()
        });
        let count_said = opening.as_ref().and_then(|job| {
            let (done, total) = job.phase.counted()?;
            let n = |value: usize| i64::try_from(value).unwrap_or(i64::MAX);
            Some(
                words
                    .msg_with(
                        "opening-step-count",
                        &[
                            ("done", n(done).into()),
                            ("total", n(total).into()),
                            ("percent", i64::from(job.percent.unwrap_or_default()).into()),
                        ],
                    )
                    .into_owned(),
            )
        });
        let failed_at = opening
            .as_ref()
            .and_then(|job| job.steps.iter().position(|step| step.state == "failed"))
            .unwrap_or(usize::MAX);
        Self {
            opening,
            open,
            said,
            count_said,
            failed_at,
        }
    }
}

/// One machine found advertising itself on the network.
pub struct DiscoveredMachine {
    /// What to call it — "Living Room".
    pub name: String,
    /// The base URL, which is what clicking it puts in the box.
    pub url: String,
    // **There is no `needs_password` here.** Every machine has a password, so a flag saying so on
    // every row would carry no information, and this tool can send one, so it would warn about
    // nothing.
    /// Whether this is the machine already configured, so the list can say so rather than inviting
    /// somebody to set what is already set.
    pub current: bool,
}

/// What a look on the network turned up, swapped in under the Discover button.
///
/// **Never applied automatically**, which is the whole design of this fragment: it lists what is
/// there and every entry is a button somebody has to press. A tool that silently re-pointed itself
/// at whatever answered first would be a tool that quietly installs a package on the wrong machine.
#[derive(Template)]
#[template(path = "discovered.html")]
pub struct DiscoveredFragment {
    /// What was found, best name first.
    pub machines: Vec<DiscoveredMachine>,
}

/// What the label beside the raise-the-version box says.
///
/// **The concrete number, not the rule.** A box saying "raise the version" leaves the reader to work
/// out which of the three digits moves, whether this build or the next one is the one that moves it,
/// and what happens to a version that is not `X.Y.Z` — so it says the answer instead. The three
/// states are the three the build itself has, and the label is the only place a person meets the
/// last of them before pressing the button.
///
/// **The number comes from [`crate::version::next`]**, which is also where the default file name
/// beside it gets one, so the label and the box cannot come to name two different builds.
///
/// A free function rather than only a method, because it needs a row and nothing else — and a test
/// that had to build a whole page to read one sentence would not be written.
pub fn raise_version_says(package: &crate::model::PackageRow, locale: km_locale::Locale) -> String {
    let words = crate::words::messages(locale);
    if package.built_at.is_none() {
        return words.msg("package-raise-first-build").into_owned();
    }
    let raised = crate::version::next(&package.version, true, true);
    if raised == package.version {
        return words
            .msg_with(
                "package-raise-not-three-numbers",
                &[("version", package.version.as_str().into())],
            )
            .into_owned();
    }
    words
        .msg_with("package-raise-to", &[("version", raised.as_str().into())])
        .into_owned()
}

/// The box naming the file the next build writes.
///
/// **A fragment because the version is in the name.** The tick box beside it decides whether the
/// build raises the version, so moving it changes what the file will be called — and a box left
/// showing the name for the other answer would name a file that never exists. The label already
/// carries the number for the same reason; this keeps the two saying one thing.
#[derive(Template)]
#[template(path = "build_out.html")]
pub struct BuildOutBox {
    /// The name of the file the next build writes, without its folder.
    pub file: String,
}

/// A one-line result of an action, swapped into a slot on the page.
#[derive(Template)]
#[template(path = "message.html")]
pub struct MessageFragment {
    /// What to say.
    pub text: String,
    /// Whether it went well.
    pub ok: bool,
}

impl MessageFragment {
    /// A success message.
    pub fn ok(text: impl Into<String>) -> Response {
        Self {
            text: text.into(),
            ok: true,
        }
        .respond()
    }

    /// A failure message.
    ///
    /// Deliberately still a 200: htmx does not swap the response of a failed request by default, so
    /// returning 400 here would leave the person clicking a button that visibly does nothing. The
    /// message itself says it failed.
    pub fn failed(text: impl Into<String>) -> Response {
        Self {
            text: text.into(),
            ok: false,
        }
        .respond()
    }

    /// Rendered with no catalog, because this template holds no key.
    ///
    /// `message.html` is one line of `{{ text }}`, and the sentence in it was worded against the
    /// catalog where it was composed — which is the rule for every sentence this tool says: markup
    /// carries a key, Rust carries a sentence. So no locale has to reach here, and none of the
    /// hundred-odd callers has to carry one.
    fn respond(&self) -> Response {
        match self.render() {
            Ok(body) => Html(body).into_response(),
            Err(error) => template_error(&error),
        }
    }
}

/// A line of text over the page, for an action taken from a list.
///
/// **The server renders a toast, and `static/ui.js` narrows rather than forbids that.** The
/// load-bearing half of its rule stands: `/songs/rows` answers a database error with a 500 and an
/// unswappable body, because a 200 there would paint the error over the rows somebody was reading,
/// and the browser is the only thing that can say so. A toast is the *other* case — an action that
/// succeeded, taken from a list whose message slot is above a page of rows and is off the screen by
/// the time the button at the bottom of them is pressed.
///
/// It travels as an out-of-band swap into `#toasts` rather than as an `HX-Trigger` header, and the
/// reason is encoding rather than taste: `XMLHttpRequest.getResponseHeader` decodes a header as
/// ISO-8859-1, and every message here is liable to carry a Portuguese title or a path out of the
/// corpus. A body is escaped by askama and arrives as the bytes that were written.
///
/// Copied from `km-remote-pages`'s `views::toast_only` rather than shared, for the reason the toast CSS
/// was already copied: this crate does not depend on that one.
#[derive(Template)]
#[template(path = "toast.html")]
pub struct Toast {
    /// `toast-good`, `toast-warn` or `toast-bad`, matching the border colors in `style.css`.
    pub level: &'static str,
    /// What it says.
    pub text: String,
}

impl Toast {
    /// Something happened, and it was fine.
    pub fn good(text: impl Into<String>) -> Self {
        Self {
            level: "toast-good",
            text: text.into(),
        }
    }

    /// Something did not happen, and it is worth knowing.
    pub fn bad(text: impl Into<String>) -> Self {
        Self {
            level: "toast-bad",
            text: text.into(),
        }
    }
}

/// Wraps a toast in the out-of-band element `#toasts` receives.
///
/// `afterbegin`, matching `ui.js`'s own `prepend`, so the tray is newest-first in the DOM whichever
/// source filled it — which is what puts the newest at the top of the strip under `flex-direction:
/// column`, and what makes the last child the oldest for `ui.js`'s cap to drop.
fn oob(toast: &Toast) -> String {
    let rendered = toast.render().unwrap_or_default();
    let open = "<div id=\"toasts\" hx-swap-oob=\"afterbegin\">";
    format!("{open}{rendered}</div>")
}

/// A toast and nothing else, for an action whose result is not on screen.
///
/// The body is *only* the out-of-band element, so whatever the caller aimed `hx-target` at is
/// emptied — which is what clears a message left over from the previous action, and what
/// `.message:empty { display: none }` in `style.css` was already written for.
pub fn toast_only(toast: &Toast) -> Response {
    Html(oob(toast)).into_response()
}

/// A rendered fragment with a toast riding along beside it.
///
/// Both are out of band here — the fragment because it is [`PlayedFragment`], which is nothing but
/// out-of-band play buttons — so the caller's own `hx-target` is swapped with an empty string and
/// whatever the last action left in it goes away. That is deliberate: a stale "Playing on …" sitting
/// in the slot while a toast says something newer is the one arrangement worse than either alone.
pub fn with_toast<T: Template>(template: &T, toast: &Toast, locale: km_locale::Locale) -> Response {
    match render(template, locale) {
        Ok(body) => Html(format!("{body}{}", oob(toast))).into_response(),
        Err(error) => template_error(&error),
    }
}

/// A rendered fragment, perhaps a second one riding out of band beside it, and any number of toasts.
///
/// For a build's last frame, which can bring the install button back and has one sentence per volume
/// it wrote.
pub fn with_toasts<T: Template, O: Template>(
    template: &T,
    extra: Option<&O>,
    toasts: &[Toast],
    locale: km_locale::Locale,
) -> Response {
    let body = match render(template, locale) {
        Ok(body) => body,
        Err(error) => return template_error(&error),
    };
    let extra = match extra.map(|extra| render(extra, locale)).transpose() {
        Ok(extra) => extra.unwrap_or_default(),
        Err(error) => return template_error(&error),
    };
    let toasts: String = toasts.iter().map(oob).collect();
    Html(format!("{body}{extra}{toasts}")).into_response()
}

/// The rows, the chips strip that goes with them, and the address the page should now be at.
///
/// The chips ride out of band because they are not where the caller aimed: `#rows` is the swap and
/// `#chips` is fifty lines above it, outside the element being replaced. `HX-Push-Url` is here for a
/// related reason — the browser's address bar is the third thing outside `#rows` that a filter
/// change ought to move, and the only one no swap can reach.
pub fn rows_with_chips<T: Template>(
    rows: &T,
    chips: &FilterChips,
    url: &str,
    locale: km_locale::Locale,
) -> Response {
    let (rows, chips) = match (render(rows, locale), render(chips, locale)) {
        (Ok(rows), Ok(chips)) => (rows, chips),
        (Err(error), _) | (_, Err(error)) => return template_error(&error),
    };
    let mut response = Html(format!("{rows}{chips}")).into_response();
    // A URL a person could have typed. Anything else here is worse than nothing: htmx puts the value
    // in the address bar verbatim, so a bad one is a page that cannot be reloaded.
    if let Ok(value) = url.parse() {
        response.headers_mut().insert("HX-Push-Url", value);
    }
    response
}

/// The 500 a template failure becomes. See [`page`], which says why it can be reached at all.
fn template_error(error: &askama::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("template error: {error}"),
    )
        .into_response()
}

/// `POST /songs/language-bulk` before it is confirmed: what would be written, and to how many.
#[derive(Template)]
#[template(path = "bulk_language_confirm.html")]
pub struct BulkLanguageConfirm {
    /// Whether that is the whole corpus, which the fragment says out loud rather than leaving to be
    /// inferred from a large number.
    ///
    /// **A field rather than `filters.is_empty()`**, because a ticked write with no narrowing is not
    /// the corpus and would otherwise be labelled as one.
    pub whole_corpus: bool,
    /// What is narrowing it, in the words the chips use -- the filter for a filter-wide write, and
    /// `N ticked` for a ticked one.
    pub filters: Vec<String>,
    /// What they would be set to, named rather than coded — this is prose.
    pub language: String,
    /// What is being changed, counted: *155 songs*.
    ///
    /// Composed rather than a number beside a word, because the word follows the number and each
    /// language chooses its own form.
    pub subject: String,
    /// The button that goes ahead, which names the count again.
    pub confirm: String,
    /// The query string that was counted, so the confirmed write is over exactly that set.
    pub query: String,
    /// The ticked ids, rendered as hidden fields so the confirmed write is over exactly the set that
    /// was counted. Empty for a filter-wide write, where the frozen query string does that job.
    pub songs: Vec<String>,
}

/// One song's tags, as chips with a remove link.
///
/// Its own fragment because both halves of the song page's editor swap it — adding replaces it, and
/// so does each ✕. A form that swapped itself would take its own text box out from under whoever is
/// typing in it.
#[derive(Template)]
#[template(path = "tag_list.html")]
pub struct TagList {
    /// Which song, for the remove links.
    pub song_id: String,
    /// The tags it carries, sorted.
    pub tags: Vec<String>,
}

/// `POST /songs/tag-bulk` before it is confirmed: which tag, which way, and to how many.
///
/// [`BulkLanguageConfirm`] with the language swapped for a tag and a direction. There is no
/// `only_unset` field, because there is no such state for a tag — see [`crate::handlers::bulk_tag`].
#[derive(Template)]
#[template(path = "bulk_tag_confirm.html")]
pub struct BulkTagConfirm {
    /// Whether that is the whole corpus, said out loud rather than left to be inferred from a large
    /// number. A field rather than `filters.is_empty()`, for [`BulkLanguageConfirm`]'s reason.
    pub whole_corpus: bool,
    /// What is narrowing it, in the words the chips use.
    pub filters: Vec<String>,
    /// The tag, as the slug it will be stored as — which is not always what was typed, and is worth
    /// showing for that reason: this is where somebody finds out `Rock & Roll` became `rock-roll`.
    pub tag: String,
    /// What is being changed, counted: *155 songs*.
    ///
    /// Composed rather than a number beside a word, because the word follows the number and each
    /// language chooses its own form.
    pub subject: String,
    /// The button that goes ahead, which names the count again.
    pub confirm: String,
    /// Whether this takes the tag off rather than putting it on.
    pub removing: bool,
    /// The query string that was counted, so the confirmed write is over exactly that set.
    pub query: String,
    /// The ticked ids, as hidden fields. Empty for a filter-wide write.
    pub songs: Vec<String>,
}

/// `POST /songs/delete-bulk` before it is confirmed: how many songs, which way, and how many of
/// them a package names.
///
/// [`BulkTagConfirm`] with the tag swapped for a sentence about packages. That sentence is the one
/// thing here the other confirmations have no equivalent of: every other bulk action writes onto
/// songs that stay in the lists they are in, where this takes them out of the list a package is
/// rebuilt from.
#[derive(Template)]
#[template(path = "bulk_delete_confirm.html")]
pub struct BulkDeleteConfirm {
    /// Whether that is the whole corpus, said out loud rather than left to be inferred from a large
    /// number. A field rather than `filters.is_empty()`, for [`BulkLanguageConfirm`]'s reason.
    pub whole_corpus: bool,
    /// What is narrowing it, in the words the chips use.
    pub filters: Vec<String>,
    /// What is being changed, counted: *155 songs*.
    pub subject: String,
    /// The button that goes ahead, which names the count again.
    pub confirm: String,
    /// Whether this brings songs back rather than throwing them away.
    pub restoring: bool,
    /// How many of them a package names, composed and counted, or empty when none do.
    ///
    /// Composed in Rust rather than a number the markup puts a word beside, for [`Self::subject`]'s
    /// reason: the word follows the number and each language chooses its own form. Empty is the
    /// ordinary case and draws nothing at all.
    pub packaged: String,
    /// The query string that was counted, carrying the page it was pressed on, so the confirmed
    /// write is over exactly that set and the list comes back where it was.
    pub query: String,
    /// The ticked ids, as hidden fields. Empty for a filter-wide write.
    pub songs: Vec<String>,
}

/// `POST /songs/favorite-bulk` before it is confirmed: which songs, and in or out of what.
#[derive(Template)]
#[template(path = "bulk_favorite_confirm.html")]
pub struct BulkFavoriteConfirm {
    /// Whether that is the whole corpus, said out loud rather than left to be inferred from a large
    /// number. A field rather than `filters.is_empty()`, for [`BulkLanguageConfirm`]'s reason.
    pub whole_corpus: bool,
    /// What is narrowing it, in the words the chips use.
    pub filters: Vec<String>,
    /// The favorite's full path, which is what names it in the tree and the only spelling that tells
    /// two `rock` apart under different parents.
    pub favorite: String,
    /// What is being changed, counted: *155 songs*.
    ///
    /// Composed rather than a number beside a word, because the word follows the number and each
    /// language chooses its own form.
    pub subject: String,
    /// The button that goes ahead, which names the count again.
    pub confirm: String,
    /// Whether this takes the songs out rather than putting them in.
    pub removing: bool,
    /// The query string that was counted, so the confirmed write is over exactly that set.
    pub query: String,
    /// The ticked ids, as hidden fields. Empty for a filter-wide write.
    pub songs: Vec<String>,
}

/// `POST /songs/reanalyze` before it is confirmed: how many files would be read again.
#[derive(Template)]
#[template(path = "reanalyze_confirm.html")]
pub struct ReanalyzeConfirm {
    /// Whether that is the whole corpus, said out loud rather than left to be inferred from a large
    /// number. A field rather than `filters.is_empty()`, for [`BulkLanguageConfirm`]'s reason.
    pub whole_corpus: bool,
    /// What is narrowing it, in the words the chips use.
    pub filters: Vec<String>,
    /// What is being changed, counted: *155 songs*.
    ///
    /// Composed rather than a number beside a word, because the word follows the number and each
    /// language chooses its own form.
    pub subject: String,
    /// The button that goes ahead, which names the count again.
    pub confirm: String,
    /// The query string that was counted, so the confirmed read is over exactly that set.
    pub query: String,
    /// The ticked ids, as hidden fields. Empty for a filter-wide read.
    pub songs: Vec<String>,
}

/// `POST /packages/from-filter` before it is confirmed: what would go in, and under what name.
#[derive(Template)]
#[template(path = "package_from_filter_confirm.html")]
pub struct PackageFromFilterConfirm {
    /// The filters narrowing it, in the words the chips use. Empty means the whole corpus, which the
    /// fragment says out loud rather than leaving to be inferred from a large number.
    pub filters: Vec<String>,
    /// What is being changed, counted: *155 songs*.
    ///
    /// Composed rather than a number beside a word, because the word follows the number and each
    /// language chooses its own form.
    pub subject: String,
    /// What the package would be called.
    pub name: String,
    /// The query string that was counted, so the confirmed write is over exactly that set.
    pub query: String,
    /// The list to keep the package sourced from, when the bar names one and narrows by nothing
    /// else. `None` leaves the box out altogether rather than drawing an unticked one, because a
    /// filter that carries other terms makes a package the list alone would not — see
    /// [`crate::db::Filter::only_this_favorite`].
    ///
    /// **Ticked when it is there.** Somebody who narrowed to one list and asked for a package of
    /// every matching song has said what the package is; unticking it is one click and the package
    /// is an ordinary one.
    pub source: Option<SourceChip>,
}

/// `POST /packages/add` with the filter-wide scope, before it is confirmed.
///
/// **The ticked scope has no confirmation and therefore no fragment.** Its set is the rows in front
/// of somebody, and the route is shared with a song's own page and the Lyrics page, neither of which
/// has a question to put. See `Acting on a whole filter` in `docs/decisions/curation.md`.
#[derive(Template)]
#[template(path = "package_add_confirm.html")]
pub struct PackageAddConfirm {
    /// How many songs the filter matches.
    pub count: u32,
    /// How many songs the write is about, counted: *155 songs*.
    pub subject: String,
    /// What room the package has left, where it has less than the write would put in.
    pub room_left: String,
    /// The filters narrowing it, in the words the chips use. Empty means the whole corpus, which the
    /// fragment says out loud rather than leaving to be inferred from a large number.
    pub filters: Vec<String>,
    /// What the package is called, which is what somebody picked from the select and the only
    /// spelling of it they have seen.
    pub name: String,
    /// How many numbers the package has left. Shown only when it is fewer than the filter matched,
    /// which is the case a count alone would let somebody walk into. [`PackageFromFilterConfirm`]
    /// needs no such field: a package made from a filter has every number it will ever have.
    pub room: u32,
    /// The query string that was counted, so the confirmed write is over exactly that set.
    pub query: String,
}

/// `POST /songs/{id}/play` — the play buttons that changed, and nothing else.
///
/// The buttons ride out of band, because "which song did I just send?" is a question about a row
/// somebody is looking at, and the answer has to appear on that row rather than in a line of text
/// under a page of others. What to *say* is no longer in here: the handler pairs this with a
/// [`Toast`] or a [`MessageFragment`] depending on where the click came from.
#[derive(Template)]
#[template(path = "played.html")]
pub struct PlayedFragment {
    /// The song just sent, which gains the highlight.
    pub played_id: String,
    /// The songs the page showed lit, which lose the highlight. Empty on the first play, or when it
    /// is the same song again. More than one when another tab's play left this page behind.
    pub unlit: Vec<String>,
}

/// `POST /songs/quality-hint` — the numbered badges that changed, and nothing else.
///
/// **Out of band rather than a redraw of `#rows`**, for two reasons that point the same way. A
/// badge is a few characters inside a row that is otherwise untouched, and redrawing the rows to
/// place it would put this route under the rule every route that redraws them lives by: it would
/// have to write `State::songs_filter` down or leave *save this filter* pointing at a page nobody
/// is on. And a hinted song that is not on the page being looked at has no element to swap, which
/// htmx answers by dropping the swap — exactly the right behavior, and one a redraw could not
/// offer.
#[derive(Template)]
#[template(path = "hint_marks.html")]
pub struct HintMarks {
    /// Each song and the number it now carries, empty for one whose badge is being rubbed out.
    ///
    /// Rendered here rather than matched in the template, so the badge is one expression and the
    /// element it lands on is emptied rather than made to hold a space — which `.place:empty` reads
    /// as a badge that is still there.
    pub marks: Vec<(String, String)>,
}

/// `GET /settings` — the small panel for the karaoke app's address.
#[derive(Template)]
#[template(path = "settings.html")]
pub struct SettingsPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The app's base URL.
    pub machine: String,
    /// What the app said when asked, or why it could not be reached.
    pub status: String,
    /// Whether that status is good news.
    pub reachable: bool,
    /// Whether that machine is on this box, and so which way a test-play sends a song.
    ///
    /// On the page because the button does two visibly different things and nothing else would say
    /// which — the same argument `Discovering a machine in the package builder` makes for listing
    /// rather than setting: somebody should be able to see what the tool is about to do.
    pub on_this_box: bool,
    /// Whether this run holds an admin token for that machine.
    ///
    /// **Which of the two states the panel draws**, and the reason the box is on this page at all:
    /// sending a package and installing one are admin actions, and the refusal they answer names
    /// this control by name. See `Installing a package makes the same choice test-play does` in
    /// `docs/decisions/curation.md`.
    pub signed_in: bool,
    /// Whether the machine has said who it is, so a password could be remembered under its id.
    ///
    /// `false` for a machine that has never answered — `crate::passwords` is keyed by identity, and
    /// there is none yet. The checkbox is not offered in that state rather than offered and ignored.
    pub can_remember: bool,
    /// Whether this computer is already remembering that machine's password.
    pub remembered: bool,
    /// Whether the remembered-password file can be made owner-only on this platform.
    ///
    /// The sentence beside the checkbox says what was actually done, which on Windows is nothing
    /// beyond the profile directory. See `Where a key somebody typed into a page lives` in
    /// `docs/decisions/repository.md`: claiming the protection would be worse than not having it.
    pub owner_only: bool,
    /// Whether the machine is in debugging mode, which is what mounts its two play routes.
    pub debug_enabled: bool,
    /// What the last sign-in, sign-out or Debugging press said, or empty on a fresh page.
    ///
    /// Here because `machine_access.html` is included by this page and returned on its own by the
    /// three routes that change it, and an included template is rendered against the *including*
    /// struct — so both have to carry every field the fragment names. Empty renders nothing.
    pub message: String,
    /// Whether [`Self::message`] is good news.
    pub ok: bool,
    /// Where a backup goes if nobody says otherwise.
    pub default_backup_out: String,
    /// The most recent backup in the data folder, as the restore box suggests it. Empty for none.
    ///
    /// A hint rather than a value, and that is the whole of the reasoning: restoring in the
    /// overwrite direction takes work away, so a real path sitting in the box one click from
    /// *Restore* is a sharper edge than the confirmation covers.
    pub newest_backup: String,
    /// The suggested tags, comma-joined, as the box shows them.
    pub default_tags: String,
    /// Where the settings file is, or empty when this run keeps none.
    ///
    /// Shown so somebody can find it: the list is also a plain JSON file, which is the version that
    /// goes in a dotfiles repository or gets copied to a second machine.
    pub settings_file: String,
    /// The languages these pages can be drawn in, and which one is current.
    pub locales: Vec<LocaleChoice>,
    /// How many songs carry something a person typed, counted.
    pub hand_set_said: String,
    /// How many files did not parse, where any did.
    pub failed_said: Option<String>,
    /// How many songs are in a favorite, counted.
    pub favorites_said: String,
}

/// One language a picker offers.
#[derive(Debug, Clone)]
pub struct LocaleChoice {
    /// The BCP 47 tag the form posts back.
    pub tag: &'static str,
    /// What this language calls itself.
    ///
    /// **Its own name rather than its English one.** The one person who has to read a language
    /// picker is by definition the one who cannot read the page around it, so `Português (Brasil)`
    /// is legible to exactly whoever would choose it. It comes out of `Locale::endonym`, which keeps
    /// it in code where no catalog can translate it.
    pub endonym: &'static str,
    /// Whether this is the one in force.
    pub chosen: bool,
}

impl LocaleChoice {
    /// Every language, in the order a picker shows them.
    pub fn all(current: km_locale::Locale) -> Vec<LocaleChoice> {
        km_locale::Locale::ALL
            .iter()
            .map(|locale| LocaleChoice {
                tag: locale.tag(),
                endonym: locale.endonym(),
                chosen: *locale == current,
            })
            .collect()
    }
}

/// The password box and the Debugging switch, swapped back on their own.
///
/// The same markup as the block inside the Settings page — `machine_access.html` is included by
/// both — so the panel a sign-in swaps in cannot drift from the one the page was drawn with. See
/// [`SongRowFragment`] for the arrangement and why it is worth the second struct.
#[derive(Template)]
#[template(path = "machine_access.html")]
pub struct MachineAccessFragment {
    /// Whether this run holds an admin token for the machine in force.
    pub signed_in: bool,
    /// Whether the machine has said who it is, so a password could be remembered under its id.
    pub can_remember: bool,
    /// Whether this computer is already remembering that machine's password.
    pub remembered: bool,
    /// Whether the remembered-password file can be made owner-only on this platform.
    pub owner_only: bool,
    /// Whether the machine is in debugging mode.
    pub debug_enabled: bool,
    /// What just happened, or empty.
    pub message: String,
    /// Whether that is good news.
    pub ok: bool,
}

/// Renders a suitability with a class that says whether it is good.
pub fn suitability_class(suitability: &Option<u8>) -> &'static str {
    match suitability {
        // A video song, which has no automatic suitability at all. Drawn like the dash in an
        // empty artist cell rather than like a bad one: `low` would color it as though something
        // had been measured and found wanting.
        None => "none",
        Some(0..=4) => "low",
        Some(5..=7) => "mid",
        Some(_) => "high",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discovered(machines: Vec<DiscoveredMachine>) -> String {
        DiscoveredFragment { machines }
            .in_english()
            .expect("the fragment renders")
    }

    /// A pager counts pages from one, and never says it is past its own last page.
    #[test]
    fn a_pager_counts_pages_from_one() {
        assert_eq!(page_of(0, 100, 50), (1, 2), "an exact multiple");
        assert_eq!(
            page_of(850, 1680, 50),
            (18, 34),
            "a short last page still counts"
        );
        assert_eq!(
            page_of(0, 0, 50),
            (1, 1),
            "an empty list is page one of one"
        );
        assert_eq!(
            page_of(1000, 60, 50),
            (21, 21),
            "a total that lags the offset stretches to the page shown"
        );
        assert_eq!(
            say_page(km_locale::Locale::English, "songs-range", 850, 1680, 50),
            "page 18 of 34 (1680 songs)"
        );
        assert_eq!(
            say_page(km_locale::Locale::English, "songs-range", 0, 1, 50),
            "page 1 of 1 (1 song)"
        );
    }

    /// The label and the file name beside it name the same version.
    ///
    /// They are two readings of one question — *what will this build write* — and a page that
    /// answered it twice would put a number in the label that the file on disk never carried. Both
    /// go through `version::next`, and this is what says so.
    #[test]
    fn the_label_and_the_file_name_name_one_version() {
        let row = |version: &str, built: bool| crate::model::PackageRow {
            id: "1f4a9c8e2b7d0356".to_owned(),
            name: "Brasil Volume 1".to_owned(),
            version: version.to_owned(),
            publisher: None,
            start_number: 1,
            default_language: Some("en".to_owned()),
            out_path: None,
            built_at: built.then(|| "2026-09-10T00:00:00Z".to_owned()),
            song_count: 3,
            ..crate::model::PackageRow::new("", "")
        };

        let rebuilt = row("1.0.0", true);
        assert_eq!(
            raise_version_says(&rebuilt, km_locale::Locale::English),
            "raise the version to 1.0.1 on this build"
        );
        let named = crate::build::default_out_path(std::path::Path::new("/corpus"), &rebuilt, true);
        assert!(
            named.to_string_lossy().contains("1.0.1"),
            "the label says 1.0.1 and the box says {}",
            named.display()
        );

        // The other two states the build has: a first build spends nothing, and a version this tool
        // cannot read is one it says it cannot raise.
        let first = row("1.0.0", false);
        assert_eq!(
            raise_version_says(&first, km_locale::Locale::English),
            "raise the version on every build after this one"
        );
        assert!(
            crate::build::default_out_path(std::path::Path::new("/corpus"), &first, true)
                .to_string_lossy()
                .contains("1.0.0")
        );

        let odd = row("2024-spring", true);
        assert!(
            raise_version_says(&odd, km_locale::Locale::English).contains("is not three numbers")
        );
        assert!(
            crate::build::default_out_path(std::path::Path::new("/corpus"), &odd, true)
                .to_string_lossy()
                .contains("2024-spring")
        );
    }

    /// The property the owner asked for, as a test: **listing is not setting**.
    ///
    /// Every machine found is a button somebody has to press. The fragment carries no `hx-trigger`
    /// that could fire on its own and nothing that submits without a click, so a look at the network
    /// cannot re-point this tool at a machine by itself — which would be a package installed
    /// somewhere nobody chose.
    #[test]
    fn discovering_a_machine_never_sets_it_by_itself() {
        let html = discovered(vec![DiscoveredMachine {
            name: "Living Room".to_owned(),
            url: "http://192.168.1.42:8177".to_owned(),
            current: false,
        }]);

        assert!(html.contains("Living Room"));
        assert!(html.contains("http://192.168.1.42:8177"));
        assert!(
            html.contains("hx-post=\"/settings\""),
            "the address is saved by pressing the button and by nothing else"
        );
        assert!(
            !html.contains("hx-trigger"),
            "nothing here may fire without a click: {html}"
        );
    }

    /// Finding nothing is an ordinary answer and has to say what to do next, because the likeliest
    /// cause is a machine that is switched off rather than anything wrong with this tool.
    #[test]
    fn finding_no_machines_explains_itself_rather_than_showing_an_empty_list() {
        let html = discovered(Vec::new());
        assert!(!html.contains("<li"), "no rows: {html}");
        assert!(html.contains("advertise_mdns"));
    }

    /// The machine already configured is marked, so nobody sets what is already set.
    #[test]
    fn the_machine_already_in_use_is_marked_as_such() {
        let html = discovered(vec![DiscoveredMachine {
            name: "Living Room".to_owned(),
            url: "http://192.168.1.42:8177".to_owned(),
            current: true,
        }]);
        assert!(html.contains("already set"));
        // **No password note any more.** It came from an mDNS TXT record that no longer exists —
        // every machine has a password, so a flag saying so on every row carried no information.
        assert!(
            !html.contains("cannot send"),
            "the row must not still apologise for a limit this tool no longer has: {html}"
        );
    }

    /// Rendered with the English catalog, these tests being about which controls a fragment draws.
    ///
    /// **askama's own `render` is what must not be called here.** With no catalog behind it, every
    /// key on the page draws `⟦nav-songs⟧` — so a test written that way asserts against the failure
    /// mode rather than against the page, and passes for as long as it is looking for something
    /// unkeyed.
    trait InEnglish: Template {
        fn in_english(&self) -> askama::Result<String> {
            self.render_with_values(&filters::values(crate::words::messages(
                km_locale::Locale::English,
            )))
        }
    }

    impl<T: Template> InEnglish for T {}

    /// The same, for a fragment whose render is not the thing under test.
    fn html<T: Template>(template: &T) -> String {
        template.in_english().expect("the fragment renders")
    }

    fn chrome() -> Chrome {
        // A machine that has been set and has answered, since that is the shape most of these pages
        // are rendered against. `the_header_says_which_machine` covers the other two.
        chrome_for(Some((
            "http://192.168.1.5:8177".to_owned(),
            Some("Living Room".to_owned()),
        )))
    }

    /// The same header against one machine or none.
    ///
    /// **Built rather than patched with `..chrome()`**, because the tag's tooltip is composed from
    /// the machine's address: a struct update expression would leave a header naming one machine and
    /// pointing at another, which is a shape no page can reach and no test should assert on.
    fn chrome_for(machine: Option<(String, Option<String>)>) -> Chrome {
        Chrome::new(
            "songs",
            // English, which is what a fresh install and every test run speak.
            km_locale::Locale::English,
            "/corpus".to_owned(),
            Counts::default(),
            // A browser, which is what a `cargo test` build is: the `desktop` feature is off by
            // default and only a real window ever sets this.
            false,
            machine,
            // Nothing remembered, which is what every page renders against until somebody has been
            // to the songs page. `the_nav_carries_the_remembered_filter` covers the other case.
            String::new(),
        )
    }

    fn row(title: &str, artist: Option<&str>, path: &str) -> SongRow {
        let mut row = SongRow {
            id: "abc123".to_owned(),
            title: title.to_owned(),
            artist: artist.map(ToOwned::to_owned),
            language: Some("pt".to_owned()),
            warnings: "[]".to_owned(),
            tags: Vec::new(),
            hint: None,
            artist_title: String::new(),
            melody_title: String::new(),
            versions_title: String::new(),
            favorite_title: String::new(),
            path_said: String::new(),
            likeness: None,
            searched_from: false,
            edited: false,
            duration_ms: 200_000,
            suitability: Some(8),
            kind: crate::model::SongKind::Midi,
            user_score: None,
            favorite_count: 0,
            permanent_count: 0,
            melody_channel: Some(3),
            file_count: 2,
            version_count: 1,
            duplicate_of: None,
            path: path.to_owned(),
            paths: path.to_owned(),
            from_filename: title.is_empty(),
            has_words: false,
        };
        // Worded as a page would, so a test asserting on a tooltip sees the sentence and not a gap.
        row.say(km_locale::Locale::English, false);
        row
    }

    /// One saved filter, worded in English, as a page would hand it to the chip.
    fn saved_row(id: i64, name: &str, query: &str) -> SavedFilterRow {
        SavedFilterRow::new(
            crate::model::SavedFilter {
                id,
                name: name.to_owned(),
                query: query.to_owned(),
            },
            km_locale::Locale::English,
        )
    }

    fn rows(songs: Vec<SongRow>) -> SongRows {
        let total = songs.len() as u32;
        let mut rows = SongRows {
            songs,
            total,
            offset: 0,
            previous: String::new(),
            next: String::new(),
            first_page: String::new(),
            last_page: String::new(),
            pages: Vec::new(),
            ratings: rating_choices(),
            languages: Choice::languages_in(&[], None),
            all_languages: Vec::new(),
            choosing_language: false,
            editing: false,
            picking: false,
            favorites: Vec::new(),
            last_played: None,
            scanning: false,
            show_filename: false,
            show_warnings: false,
            range: String::new(),
        };
        rows.say_range(km_locale::Locale::English, 50);
        rows
    }

    /// An unfiltered page's chips strip: present, and saying nothing.
    fn no_chips() -> FilterChips {
        FilterChips {
            active: Vec::new(),
            cleared: String::new(),
            oob: false,
        }
    }

    /// A page nobody has saved a filter on.
    fn no_saved() -> SavedFilters {
        SavedFilters {
            filters: Vec::new(),
            oob: false,
            renaming: false,
        }
    }

    /// The strip is on the page before anything is in it.
    ///
    /// Two things depend on that and the second is the load-bearing one: the control that saves the
    /// first filter lives in here, and an out-of-band swap can only replace an element that is
    /// already on the page — so a strip hidden while empty could never be filled.
    #[test]
    fn the_saved_strip_is_drawn_before_anything_is_saved() {
        let empty = no_saved().in_english().expect("render");
        assert!(empty.contains(r#"id="saved-filters""#), "{empty}");
        assert!(empty.contains("Nothing saved yet"), "{empty}");
        assert!(empty.contains(r#"name="saved_name""#), "{empty}");
        // Drawn from the page, so it replaces nothing.
        assert!(!empty.contains("hx-swap-oob"), "{empty}");

        let filled = SavedFilters {
            filters: vec![saved_row(4, "Portuguese", "language=pt")],
            oob: true,
            renaming: false,
        }
        .in_english()
        .expect("render");
        assert!(filled.contains(r#"hx-swap-oob="true""#), "{filled}");
        assert!(filled.contains(r#"href="/songs?language=pt""#), "{filled}");
        assert!(!filled.contains("Nothing saved yet"), "{filled}");
        assert!(
            filled.contains("/songs/saved-filters/4/delete"),
            "and a way to forget it: {filled}"
        );
    }

    /// A chip carries all three things that can be done to the filter it names.
    ///
    /// The rewrite carries no `hx-confirm`, and that absence is asserted: it is the only control on
    /// the page that changes what a name means without asking, because the chip it sits on has
    /// already said which name.
    #[test]
    fn a_chip_offers_rewriting_renaming_and_forgetting() {
        let html = SavedFilterChip {
            filter: saved_row(7, "Portuguese", "language=pt"),
            renaming: false,
        }
        .in_english()
        .expect("render");
        assert!(html.contains(r#"id="saved-7""#), "{html}");
        assert!(html.contains(r#"href="/songs?language=pt""#), "{html}");
        assert!(html.contains("/songs/saved-filters/7/update"), "{html}");
        assert!(
            html.contains("/songs/saved-filters/7/chip?renaming=1"),
            "{html}"
        );
        assert!(html.contains("/songs/saved-filters/7/delete"), "{html}");
        assert!(
            !html.contains(r#"hx-post="/songs/saved-filters/7/update" hx-confirm"#),
            "the rewrite asks nothing: {html}"
        );
    }

    /// Renaming opens inside the chip, so nothing on the strip moves while a name is typed.
    ///
    /// The link is gone while the box is open: a chip that is both a name being edited and a link
    /// that leaves the page is a click away from losing what was typed.
    #[test]
    fn a_chip_being_renamed_holds_the_name_and_not_the_link() {
        let html = SavedFilterChip {
            filter: saved_row(7, "Portuguese", "language=pt"),
            renaming: true,
        }
        .in_english()
        .expect("render");
        assert!(html.contains(r#"id="saved-7""#), "{html}");
        assert!(html.contains(r#"value="Portuguese""#), "{html}");
        assert!(html.contains(r#"name="saved_name""#), "{html}");
        assert!(html.contains("/songs/saved-filters/7/rename"), "{html}");
        assert!(!html.contains("href="), "{html}");
        assert!(
            html.contains(r#"hx-get="/songs/saved-filters/7/chip""#),
            "and a way back out: {html}"
        );
    }

    /// The bar ends on the saved strip and then the chips, both inside the box and both outside the
    /// form.
    ///
    /// **Inside the box and outside the form, because each alone is a page that is wrong in its own
    /// way.** Outside the box either reads as a stray row under the bar rather than the last band of
    /// it. Inside the *form*, the saved strip would be a `<form>` a browser drops and a text box
    /// firing a filter request per keystroke — which is why the chrome moved up a level instead of
    /// the strip moving in.
    ///
    /// **The chips are last**, against the rows they describe rather than among the controls that
    /// produced them.
    #[test]
    fn the_chips_are_the_last_band_in_the_filter_box() {
        let html = songs_page(Vec::new(), Vec::new());
        let box_at = html.find(r#"<div class="filter-box">"#).expect("the box");
        let form_ends = html.find("</form>").expect("the filter form ends");
        let strip_at = html.find(r#"id="saved-filters""#).expect("the strip");
        let chips_at = html.find(r#"id="filter-chips""#).expect("the chips");
        assert!(
            box_at < form_ends && form_ends < strip_at && strip_at < chips_at,
            "the bar does not end on the saved strip and then the chips: {html}"
        );

        // The stylesheet holds up the half the markup cannot state: the box has the border, and the
        // form inside it has given its own up.
        let css = include_str!("../static/style.css");
        assert!(
            css.contains(".filter-box {") && css.contains(".filter-box > form.filters.banded {"),
            "the box carries no chrome, or the form inside it still carries its own"
        );
        // And the band rules reach the chips out here, which the ones scoped to the form no longer
        // do. Without this the strip draws as a stack of text with no column down the left.
        assert!(
            css.contains(".filter-box > .band,") && css.contains(".filter-box > .chips.nothing {"),
            "a band outside the form is unstyled, or an unfiltered page still draws a chip strip"
        );
    }

    /// The name a saved filter sends cannot collide with the one a row sends.
    ///
    /// `#rows` and `#filters` ride in one body for four of this page's actions, and serde answers a
    /// repeated known key with `duplicate_field`. Nothing includes this form today; the spelling is
    /// what stops the next thing that does.
    #[test]
    fn the_save_box_does_not_send_a_name_anything_else_sends() {
        let html = no_saved().in_english().expect("render");
        assert!(html.contains(r#"name="saved_name""#), "{html}");
        assert!(!html.contains(r#"name="name""#), "{html}");
    }

    /// A rendered Songs page over one row, for the tests that read its curation tabs.
    fn songs_page(favorites: Vec<FavoriteNode>, packages: Vec<PackageRow>) -> String {
        SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]),
            favorites,
            packages,
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        }
        .in_english()
        .expect("render")
    }

    /// The same, for the Lyrics page — which carries two of the same tabs against `#hits`.
    fn lyric_search_page(favorites: Vec<FavoriteNode>) -> String {
        let songs = rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]);
        LyricSearchPage {
            chrome: chrome(),
            hits: LyricHits {
                hits: Vec::new(),
                searched: false,
                indexed: true,
                total: 0,
                offset: 0,
                previous: String::new(),
                next: String::new(),
                first_page: String::new(),
                last_page: String::new(),
                pages: Vec::new(),
                range: String::new(),
                ratings: songs.ratings,
                languages: songs.languages,
                all_languages: Vec::new(),
                choosing_language: false,
                editing: false,
                picking: false,
                favorites: Vec::new(),
                last_played: None,
            },
            favorites,
            q: String::new(),
        }
        .in_english()
        .expect("render")
    }

    /// The header names the machine Play and Install would reach, and says when there is none.
    ///
    /// Those two controls are the only things in this tool that leave it, and until now the only way
    /// to find out where they pointed was to open Settings — so a song test-played into somebody
    /// else's machine, or into nothing, looked exactly like a song test-played correctly.
    #[test]
    fn the_header_says_which_machine() {
        let draw = |machine| {
            html(&SongsPage {
                chrome: chrome_for(machine),
                rows: rows(Vec::new()),
                favorites: Vec::new(),
                packages: Vec::new(),
                query: FilterForm::default(),
                chips: no_chips(),
                saved: no_saved(),
            })
        };

        // Named, with the address still in reach.
        let page = draw(Some((
            "http://192.168.1.5:8177".to_owned(),
            Some("Living Room".to_owned()),
        )));
        assert!(page.contains("Living Room"), "{page}");
        assert!(page.contains("http://192.168.1.5:8177"), "{page}");

        // Set but never answered — the address is a complete answer on its own.
        let page = draw(Some(("http://192.168.1.5:8177".to_owned(), None)));
        assert!(page.contains("http://192.168.1.5:8177"), "{page}");

        // And none at all says so, rather than leaving a gap somebody has to notice.
        let page = draw(None);
        let said = crate::words::messages(km_locale::Locale::English).msg("header-machine-none");
        assert!(page.contains(said.as_ref()), "{page}");
    }

    /// Leaving the songs page and coming back through the nav is not a reset.
    ///
    /// The pushed URL that already covered a reload and the back button lives only in the browser,
    /// so the nav — seven bare hrefs rendered server-side — could not see it. This is the seam that
    /// fixes that, and it is worth pinning on a page that is *not* the songs page: the whole point
    /// is that the link is right while somebody is looking at something else.
    #[test]
    fn the_nav_carries_the_remembered_filter() {
        let draw = |songs_filter: &str| {
            PackagesPage {
                chrome: Chrome {
                    tab: "packages",
                    songs_filter: songs_filter.to_owned(),
                    ..chrome()
                },
                table: crate::views::PackagesTable {
                    packages: Vec::new(),
                    oob: false,
                },
                numbering: String::new(),
            }
            .in_english()
            .expect("render")
        };

        // `&#38;` rather than `&amp;` because that is what askama's escaper writes; asserted as it
        // is rendered rather than as it would be hand-typed.
        let html = draw("folder=Ingles&language=en");
        assert!(
            html.contains("href=\"/songs?folder=Ingles&#38;language=en\""),
            "{html}"
        );

        // Nothing remembered is a bare `/songs`, not a trailing `?`.
        let html = draw("");
        assert!(html.contains("href=\"/songs\""), "{html}");
    }

    #[test]
    fn the_browse_page_renders() {
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(html.contains("Corcovado"));
        assert!(html.contains("Tom Jobim"));
    }

    /// Every action that works on "everything the filter matches" has to take the filter from the
    /// **bar**, not from a query string rendered into the page.
    ///
    /// **This test used to assert the exact opposite**, and the story is worth keeping because both
    /// versions were written from a real fault. The first was `hx-include="#filters"` with the
    /// handler reading `Query<FilterQuery>` — htmx puts an included form's fields in the *body* of a
    /// POST, so the filter was silently dropped and the action would have written the whole corpus.
    /// The remedy was to render the query string into `hx-post`, and this test pinned it.
    ///
    /// That remedy had a fault of its own, and a quieter one: **the filter bar never re-renders the
    /// page.** It swaps `#rows` and leaves everything else as the page loaded. So the attribute this
    /// test was guarding froze the moment it was written, and picking a filter from the bar narrowed
    /// the list while these two forms went on asking about the corpus. Reported as a list of 17,722
    /// songs and a confirmation offering to package hundreds of thousands.
    ///
    /// So the include is right and always was; what was missing is the other half — the handler
    /// reading the body ([`crate::handlers::FilterQuery::from_body`]). The one thing that must not
    /// come back is a filter rendered into these two attributes, which is what the last assertion
    /// says.
    #[test]
    fn the_bulk_actions_take_the_filter_from_the_bar() {
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        // The language set includes `#rows` as well, so the ticked boxes reach it — legal for the
        // reason `FilterQuery::from_body` gives, and the same inclusion *Title from file name*
        // already makes.
        for (post, include) in [
            ("/songs/language-bulk", "#filters, #rows"),
            ("/songs/tag-bulk", "#filters, #rows"),
            ("/songs/favorite-bulk", "#filters, #rows"),
            ("/songs/reanalyze", "#filters, #rows"),
            ("/packages/from-filter", "#filters"),
        ] {
            assert!(
                html.contains(&format!(r##"hx-post="{post}" hx-include="{include}""##)),
                "{post} must read the bar as it is now:\n{html}"
            );
            assert!(
                !html.contains(&format!(r#"hx-post="{post}?"#)),
                "a filter rendered into {post} is one frozen at page load:\n{html}"
            );
        }
        // **Adding to a package is on this list and cannot be in the loop above**, because the one
        // thing that loop forbids is the one thing this form legitimately carries: a `?` in the
        // attribute. `as=toast` is the reply channel and not a filter — see `ReplyQuery` — so the
        // rule it has to meet is stated directly instead.
        assert!(
            html.contains(r##"hx-post="/packages/add?as=toast" hx-include="#filters, #rows""##),
            "adding to a package must read the bar as it is now:\n{html}"
        );
        assert!(
            !html.contains(r#"hx-post="/packages/add?as=toast&"#),
            "nothing may ride in that attribute beside the reply channel:\n{html}"
        );
        // Its own select cannot be `language`: the bar has one, and a repeated key is refused.
        assert!(html.contains(r#"name="set_language""#), "{html}");
        // And nor can the two controls beside it. `scope` and `only_unset` are names the bar does
        // not use, which is what keeps them safe in the same body. `scope` appears once per bulk
        // form and each form submits only its own, which is what the split into tabs bought.
        assert!(html.contains(r#"name="scope""#), "{html}");
        assert!(html.contains(r#"name="only_unset""#), "{html}");
        // The favorite tab's own select is `favorite_id`; the bar's is `favorite`. One key spelled
        // two ways is how this would file a set chosen by the favorite it was filing into.
        assert!(html.contains(r#"name="favorite_id""#), "{html}");
        assert!(html.contains(r#"name="favorite_action""#), "{html}");
    }

    /// The lists a package reads are a table, and the rest are a picker.
    #[test]
    fn the_package_page_lists_its_sources_and_offers_the_rest() {
        let unsourced = package_page(&[]);
        assert!(
            unsourced.contains(r#"<option value="1">Bossa nova (3)</option>"#),
            "a list it does not read is offered, with what it holds:\n{unsourced}"
        );
        assert!(
            !unsourced.contains("/sources/remove"),
            "and there is nothing to take away:\n{unsourced}"
        );
        assert!(
            !unsourced.contains("/sync\""),
            "a package with no list has nothing to sync:\n{unsourced}"
        );

        let sourced = package_page(&[(1, "Bossa nova".to_owned(), false)]);
        assert!(
            sourced.contains(r#"hx-vals='{"favorite": 1}'"#),
            "the list it reads is a row with a Remove:\n{sourced}"
        );
        assert!(
            !sourced.contains(r#"<option value="1">"#),
            "and is not offered a second time in the picker:\n{sourced}"
        );
        assert!(
            sourced.contains(r#"hx-post="/packages/vol1/sync""#),
            "Sync is offered:\n{sourced}"
        );
        // The button names how many lists it would read, which is the fact adding one changes.
        assert!(sourced.contains("Sync from 1 list"), "{sourced}");
        // The member table stays a table: a sourced package is still one somebody can nudge.
        assert!(sourced.contains("/packages/vol1/remove"), "{sourced}");
        assert!(sourced.contains(r#"name="number""#), "{sourced}");
    }

    /// A member's Lang cell is the code, and a member with none says so.
    ///
    /// The code and not the name, as the browse row draws it: a column of "Portuguese (Brazil)" is
    /// 22 characters wide in a table already fighting for room, and the code is the vocabulary a
    /// curator types into the filter. The name is the tooltip, so nothing is lost.
    #[test]
    fn a_member_carries_its_language_as_a_code_and_its_name_as_the_tooltip() {
        let html = package_page(&[]);
        assert!(
            html.contains(
                r#"<th title="What language it is sung in, as an ISO 639-1 code">Lang</th>"#
            ),
            "the member table has no Lang column:\n{html}"
        );
        assert!(
            html.contains(r#"title="Portuguese">pt</span>"#),
            "the code is not drawn, or its tooltip does not name the language:\n{html}"
        );
        // Nobody has said, which is the row the column is read for: the package's default language
        // is what such a song is written under, and the form that sets it is on the same pane.
        assert!(
            html.contains(r#"<td class="lang"><span class="dim">&mdash;</span></td>"#),
            "a member with no language draws no dash:\n{html}"
        );
    }

    /// Every tab on a package's page has the four parts a tab is made of, and its rule.
    ///
    /// The song page's and the settings page's assertion, over the same `.songtabs` arrangement: a
    /// name that agrees in three of the four is a tab that draws itself, takes the click and shows
    /// nothing at all, because `.songtabs .pane { display: none }` is unconditional.
    #[test]
    fn every_package_tab_has_a_radio_a_label_a_pane_and_a_rule() {
        let html = package_page(&[(1, "Bossa nova".to_owned(), false)]);
        let css = include_str!("../static/style.css");
        for tab in ["songs", "sources", "build"] {
            assert!(
                html.contains(&format!(r#"id="package-tab-{tab}""#)),
                "no radio for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"for="package-tab-{tab}""#)),
                "no label for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"class="pane {tab}""#)),
                "no pane for the {tab} tab:\n{html}"
            );
            assert!(
                css.contains(&format!("#package-tab-{tab}:checked ~ .pane.{tab}")),
                "nothing shows the {tab} pane, so the tab is a label over an empty page"
            );
            assert!(
                css.contains(&format!(
                    r#"#package-tab-{tab}:checked ~ .tabstrip label[for="package-tab-{tab}"]"#
                )),
                "the {tab} tab is not underlined when it is the open one"
            );
        }
        // A pane before the last radio is a pane nothing can show: the rules are sibling selectors.
        let last_radio = html.rfind(r#"name="package-tab""#).expect("a radio");
        let first_pane = html.find(r#"class="pane "#).expect("a pane");
        assert!(last_radio < first_pane, "{html}");
        // And the slot every pane answers into is above the strip, so a refusal lands in one place
        // whichever tab raised it.
        let slot = html.find(r#"id="action-result""#).expect("the slot");
        assert!(slot < last_radio, "{html}");
    }

    /// The lists a package reads are named over its songs, and they are the panel's own answer.
    ///
    /// **Two sights of one fact, so they are built from one list.** The chips are on the Details
    /// pane and the panel is on Sources, and a page where those two disagreed about which favorites
    /// decide the rows would be a page nobody could act on.
    #[test]
    fn the_songs_are_headed_by_the_lists_that_decide_them() {
        let none = package_page(&[]);
        assert!(
            !none.contains(r#"class="tag pill"#),
            "a package nothing sources heads its songs with nothing:\n{none}"
        );

        let sourced = package_page(&[(1, "Bossa nova".to_owned(), false)]);
        assert!(
            sourced.contains(r#"href="/songs?favorite=1""#),
            "each is the link to the songs that list holds:\n{sourced}"
        );
        assert!(sourced.contains(r#"class="tag pill good""#), "{sourced}");
        // Above the rows and inside the pane that holds them, not off on the Sources tab.
        let chips = sourced.find(r#"id="package-source-chips""#).expect("chips");
        let table = sourced.find(r#"id="package-members""#).expect("the table");
        let sources_pane = sourced.find(r#"class="pane sources""#).expect("the pane");
        assert!(chips < table && table < sources_pane, "{sourced}");
    }

    /// A working list is offered to no package, and one already read is shown marked.
    ///
    /// **Both halves, because the flag is set somewhere else.** The picker leaves it out, so nobody
    /// points a package at one; the table keeps it, because a list somebody set aside *after* a
    /// package began reading it is a source only that row can take away.
    #[test]
    fn a_working_list_is_not_offered_and_is_marked_where_it_is_already_a_source() {
        let plain = package_page(&[]);
        assert!(
            !plain.contains(r#"<option value="2">"#),
            "the working list is not in the picker:\n{plain}"
        );
        assert!(
            plain.contains(r#"<option value="1">"#),
            "and the filing beside it still is:\n{plain}"
        );

        let inherited = package_page(&[(2, "to check".to_owned(), true)]);
        assert!(
            inherited.contains(r#"<span class="tag warn">Working list</span>"#),
            "one already read says what it has become:\n{inherited}"
        );
        assert!(
            inherited.contains(r#"hx-vals='{"favorite": 2}'"#),
            "and can be taken away:\n{inherited}"
        );
    }

    /// A sourced package is not in either select that adds songs to a package.
    ///
    /// **Both pages, because both post to the same route.** A song put into a sourced package goes
    /// out again on its next sync, so the package is absent rather than present-and-refused —
    /// `handlers::sourced_refusal` is what catches the page that was already open.
    #[test]
    fn neither_select_offers_a_package_that_is_sourced() {
        // The handlers feed both selects from `Db::packages_taking_songs`, which leaves a sourced
        // package out; what this says is that the select draws exactly what it is handed, so the
        // one place the rule lives is that query.
        let html = songs_page(Vec::new(), vec![package_row("vol2", "Volume 2")]);
        assert!(html.contains(r#"<option value="vol2">"#), "{html}");
        assert!(!html.contains(r#"<option value="vol1">"#), "{html}");
    }

    /// The sync confirmation names the lists and both directions before anything is written.
    #[test]
    fn the_sync_confirmation_names_the_lists_and_what_would_leave() {
        let html = PackageSyncConfirm {
            package_id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            sources: vec![
                SourceChip {
                    name: "Bossa nova".to_owned(),
                    temporary: false,
                },
                SourceChip {
                    name: "to check".to_owned(),
                    temporary: true,
                },
            ],
            adding: "12 go in".to_owned(),
            removing: "3 come out".to_owned(),
            keeping: "200 stay where they are".to_owned(),
            new_volumes: Some("this starts a new volume".to_owned()),
        }
        .in_english()
        .expect("render");
        assert!(html.contains("Bossa nova"), "{html}");
        assert!(html.contains("to check"), "{html}");
        assert!(
            html.contains("3 come out"),
            "the half an add never says:\n{html}"
        );
        assert!(html.contains("this starts a new volume"), "{html}");
        assert!(
            html.contains(r#"hx-post="/packages/vol1/sync?confirm=1""#),
            "the button writes what was counted:\n{html}"
        );
    }

    /// Making a package of one list offers to keep it sourced from that list, ticked.
    ///
    /// **And offers nothing when the filter carries more**, which is the half worth pinning: a box
    /// drawn over a narrower filter would source a package from a list it does not hold.
    #[test]
    fn making_a_package_of_one_list_offers_to_keep_it_sourced() {
        let confirm = |source: Option<SourceChip>| {
            PackageFromFilterConfirm {
                subject: "40 songs".to_owned(),
                filters: vec!["in Bossa nova".to_owned()],
                name: "Brasil Volume 2".to_owned(),
                query: "favorite=7".to_owned(),
                source,
            }
            .in_english()
            .expect("render")
        };

        let offered = confirm(Some(SourceChip {
            name: "Bossa nova".to_owned(),
            temporary: false,
        }));
        assert!(
            offered.contains(r#"name="source" value="1" checked"#),
            "ticked, because narrowing to one list has said what the package is:\n{offered}"
        );
        assert!(offered.contains("Bossa nova"), "{offered}");
        // The box lives in this fragment, so the button has to reach for it by name — the form it
        // already includes is the one holding the package's name.
        assert!(
            offered.contains(r##"hx-include="#package-from-filter, #from-filter-source""##),
            "an unticked box that never reaches the route is a box that does nothing:\n{offered}"
        );

        let plain = confirm(None);
        assert!(
            !plain.contains(r#"name="source""#),
            "a filter that is not one list gets no box at all:\n{plain}"
        );
    }

    /// A package row, for a page that only needs one to point at.
    fn package_row(id: &str, name: &str) -> PackageRow {
        PackageRow {
            id: id.to_owned(),
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            start_number: 1,
            default_language: Some("en".to_owned()),
            out_path: None,
            built_at: None,
            song_count: 0,
            ..crate::model::PackageRow::new("", "")
        }
    }

    /// The package page, over two favorites and whichever of them it is sourced from.
    fn package_page(sources: &[(i64, String, bool)]) -> String {
        let favorites = vec![
            FavoriteNode {
                id: 1,
                name: "Bossa nova".to_owned(),
                song_count: 3,
                second_copies: 0,
                temporary: false,
            },
            FavoriteNode {
                id: 2,
                name: "to check".to_owned(),
                song_count: 9,
                second_copies: 0,
                temporary: true,
            },
            // A third, so a page with one source still has something left in the picker — which is
            // the state both halves of the panel are drawn in at once.
            FavoriteNode {
                id: 3,
                name: "Samba".to_owned(),
                song_count: 7,
                second_copies: 0,
                temporary: false,
            },
        ];
        let package = package_row("vol1", "Volume 1");
        let sourcing = SourcingPanel::new(
            package.id.clone(),
            favorites,
            sources,
            false,
            km_locale::Locale::English,
        );
        PackagePage {
            build: BuildPane::new(
                std::path::Path::new("/corpus"),
                package.clone(),
                &[],
                true,
                false,
                km_locale::Locale::English,
            ),
            volume_strip: VolumeStrip::new(
                package.id.clone(),
                &[],
                1,
                false,
                km_locale::Locale::English,
            ),
            members: MembersTable::new(
                package.id.clone(),
                1,
                vec![
                    PackageMember {
                        number: 1,
                        song_id: "abc123".to_owned(),
                        title: "Corcovado".to_owned(),
                        artist: None,
                        language: Some("pt".to_owned()),
                        suitability: Some(8),
                        user_score: Some(9),
                        melody_channel: Some(3),
                        duration_ms: 200_000,
                        path: Some("a/CORCOVAD.kar".to_owned()),
                    },
                    // A second, carrying no language, because that is the row the Lang column is
                    // read for: the one the package's default language stands in for.
                    PackageMember {
                        number: 2,
                        song_id: "def456".to_owned(),
                        title: "Wave".to_owned(),
                        artist: None,
                        language: None,
                        suitability: Some(6),
                        user_score: None,
                        melody_channel: None,
                        duration_ms: 180_000,
                        path: Some("b/WAVE.kar".to_owned()),
                    },
                ],
                false,
                km_locale::Locale::English,
            ),
            chips: SourceChips {
                sources: sourcing.sources.clone(),
                oob: false,
            },
            sourcing,
            chrome: chrome(),
            corpus_languages: Vec::new(),
            every_language: Choice::languages(None),
            renumber: RenumberForm::new(
                package.id.clone(),
                1,
                1,
                false,
                km_locale::Locale::English,
            ),
            package,
        }
        .in_english()
        .expect("render")
    }

    /// Every tab has the four parts a tab is made of, and they agree about its name.
    ///
    /// A radio, a label pointing at it, a panel, and the stylesheet's arm naming all three. The
    /// strip is CSS over a radio group and `.curate .panel { display: none }` is unconditional, so a
    /// name that agrees in three of the four is a tab that draws itself, takes the click and shows
    /// nothing at all — there is no rule to turn its panel back on.
    ///
    /// **The stylesheet is asserted against the shipped file**, the way the two tests on the
    /// progress panels are: a selector cannot be rendered, and one arm per tab has to be written out
    /// because a selector cannot ask which radio is checked without naming it. That is three more
    /// places a tab's name is spelled, none of them reachable from a template.
    ///
    /// This list is where a tab is named. A new one fails here until its arms exist.
    #[test]
    fn every_curation_tab_has_a_radio_a_label_a_panel_and_a_rule() {
        let html = songs_page(Vec::new(), Vec::new());
        let css = include_str!("../static/style.css");
        for tab in [
            "language",
            "tags",
            "package",
            "favorites",
            "titles",
            "analysis",
        ] {
            assert!(
                html.contains(&format!(r#"id="curate-{tab}""#)),
                "no radio for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"for="curate-{tab}""#)),
                "no label for the {tab} tab:\n{html}"
            );
            assert!(
                html.contains(&format!(r#"class="panel {tab}""#)),
                "no panel for the {tab} tab:\n{html}"
            );
            assert!(
                css.contains(&format!("#curate-{tab}:checked ~ .panel.{tab}")),
                "nothing shows the {tab} panel, so the tab is a label over an empty page"
            );
            assert!(
                css.contains(&format!(
                    r#"#curate-{tab}:checked ~ .tabstrip label[for="curate-{tab}"]"#
                )),
                "the {tab} tab is not underlined when it is the open one"
            );
            assert!(
                css.contains(&format!(
                    r#"#curate-{tab}:focus-visible ~ .tabstrip label[for="curate-{tab}"]"#
                )),
                "the {tab} tab takes no focus ring, and its radio is hidden"
            );
        }
    }

    /// Every tab radio comes before every panel, because `~` only reaches forward.
    ///
    /// The rule that shows a panel is `#curate-x:checked ~ .panel.x`, and the general sibling
    /// combinator matches nothing that precedes the element it starts from. So the obvious tidy-up —
    /// moving each input inside its own label in the strip, the way the alphabet bar does it — is
    /// what breaks this, and it breaks it silently and completely: every panel would be
    /// `display: none` with nothing able to turn one back on.
    ///
    /// The alternative that survives that move is `:has()`, and `The curation actions are tabs`
    /// says why it is refused here: this rule has to fail towards showing something.
    #[test]
    fn the_tab_radios_come_before_every_panel() {
        let html = songs_page(Vec::new(), Vec::new());
        let last_radio = html
            .rfind(r#"name="curate""#)
            .expect("the tabs are radios:\n{html}");
        let first_panel = html
            .find(r#"class="panel "#)
            .expect("the tabs have panels:\n{html}");
        assert!(
            last_radio < first_panel,
            "a tab radio is drawn after a panel, so `~` cannot reach it:\n{html}"
        );
    }

    /// The actions on the ticked rows are one form each, and each carries the ticks itself.
    ///
    /// They were one bar, `#selection`, holding three unrelated actions divided by two dim pipes —
    /// so the button that filed songs into a favorite had to `hx-include` its own form to reach the
    /// select two controls to its left. One tab per action forced the split; this pins the property
    /// the split bought, which is that no button reaches sideways for a field.
    ///
    /// Filing into a favorite and adding to a package are not on this list, because they read the
    /// bar as well and are asserted with the bulk actions that do.
    #[test]
    fn each_ticked_song_action_carries_the_ticks_itself() {
        let html = songs_page(Vec::new(), Vec::new());
        for post in [
            "/songs/titles-from-filename",
            "/songs/fix-name-case",
            "/songs/split-artist-from-title",
        ] {
            assert!(
                html.contains(&format!(r##"hx-post="{post}" hx-include="#rows"##)),
                "{post} must take the ticks out of `#rows` itself:\n{html}"
            );
        }
        assert!(
            !html.contains(r#"id="selection""#),
            "the one bar the three came out of is gone:\n{html}"
        );
    }

    /// Every Titles action asks for its page back, and asks in its own markup.
    ///
    /// `static/ui.js` copies `#rows`'s offset into the body of any form carrying this attribute, so
    /// a form that loses it is a button that silently drops the reader at the top of a corpus they
    /// had paged into. The other end of the rule — that `ui.js` still reads the attribute — is
    /// `the_static_files_are_embedded_and_not_empty` in `server`.
    #[test]
    fn the_titles_actions_ask_for_their_page_back() {
        let html = songs_page(Vec::new(), Vec::new());
        for form in [
            "titles-from-filename",
            "fix-name-case",
            "split-artist-from-title",
        ] {
            assert!(
                html.contains(&format!(r#"id="{form}" data-keeps-the-page"#)),
                "{form} must ask for its page back:\n{html}"
            );
        }
    }

    /// Both tabs that offer a list are drawn even when the list is empty.
    ///
    /// Wrapping the favorites control in an `{% if %}` leaves it absent on a corpus nobody has
    /// filed anything in. That is fine for a group inside a bar of other controls and wrong for a
    /// tab, which would be a label promising something over an empty panel.
    #[test]
    fn a_tab_offering_an_empty_list_says_so_rather_than_being_empty() {
        let html = songs_page(Vec::new(), Vec::new());
        assert!(html.contains(r#"name="package_id""#), "{html}");
        assert!(html.contains(r#"name="favorite_id""#), "{html}");
        assert!(html.contains("No packages yet"), "{html}");
        assert!(html.contains("No favorites yet"), "{html}");
    }

    /// The Lyrics page offers the one action that means anything there, and posts against `#hits`.
    ///
    /// Filing the hits is what a lyric search is for: the question it answers is *which song goes
    /// like this?*, and the answer is set aside for the pass that decides. Every other action the
    /// Songs page carries either reads a filter bar this page does not have or corrects a name, which
    /// is a judgment made while looking at one and this list does not show them. What
    /// the one that is here must not do is post against `#rows`: the ticks are the same `song_id`
    /// boxes drawn by the same `song_row.html`, in a list this page numbers its own way.
    #[test]
    fn the_lyrics_page_offers_the_one_action_that_means_anything_there() {
        let html = lyric_search_page(Vec::new());
        assert!(
            html.contains(r##"hx-post="/favorites/add?as=toast" hx-include="#hits"##),
            "filing must take the ticks out of `#hits`:\n{html}"
        );
        // One action is not a strip, so there is no tab markup to draw and no package select to
        // fill: a package is built from a filter, and this page has none.
        for absent in [
            "tabstrip",
            "curate-package",
            "curate-language",
            "curate-tags",
            "curate-titles",
            "curate-analysis",
            "/packages/add",
            r#"name="package_id""#,
        ] {
            assert!(
                !html.contains(absent),
                "{absent} has nothing to read here:\n{html}"
            );
        }
        assert!(!html.contains("#rows"), "there is no `#rows` here:\n{html}");
    }

    #[test]
    fn the_columns_are_named_for_what_they_hold() {
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(
            html.contains(">Suitability<"),
            "the automatic rating is headed Suitability"
        );
        // Neither the count of packages a song is already in nor the abbreviation it wore is on the
        // row: the list is read a column at a time and this was one column too many for what it
        // said. The Packages page is where a package's contents are looked at. Asserted on the
        // column's own tooltip rather than on its name, which the navigation also carries.
        assert!(
            !html.contains("How many packages already contain it"),
            "{html}"
        );
        assert!(!html.contains(">In<"), "{html}");
        // The encoding column is gone from the list; the filter that uses it is not.
        assert!(!html.contains(">Encoding<"), "{html}");
        assert!(html.contains("name=\"encoding_source\""), "{html}");
        // The alphabet bar draws each button's *label*, which is not its value for the last two.
        // It drew the value, so `symbol` was renamed everywhere except on the page — a change that
        // every test asserting on `Choice` passed and that only looking at it caught.
        assert!(html.contains(">symbol<"), "{html}");
        assert!(html.contains(">0-9<"), "{html}");
        assert!(
            html.contains(">any<"),
            "the empty value still reads as a word: {html}"
        );
        assert!(!html.contains(">#<"), "{html}");
    }

    #[test]
    fn a_row_renders_the_same_alone_as_it_does_in_the_list() {
        let song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");
        let alone = SongRowFragment::new(song.clone(), false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        let list = rows(vec![song]).in_english().expect("render");
        // Not a substring test for its own sake: this is the property that lets a row be swapped in
        // after an edit without the edited row drifting from the others around it.
        assert!(
            list.contains(alone.trim()),
            "alone:\n{alone}\nlist:\n{list}"
        );
    }

    #[test]
    fn a_row_being_edited_offers_inputs_instead_of_links() {
        let song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");
        let html = SongRowFragment::new(song, true, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(html.contains("name=\"title\""), "{html}");
        // `row_artist`, never `artist`: this form sits inside `#rows`, which rides in the same body
        // as `#filters`, and the bar has an `artist` filter of its own. See `FilterQuery::from_body`.
        assert!(html.contains("name=\"row_artist\""), "{html}");
        assert!(!html.contains("name=\"artist\""), "{html}");
        assert!(html.contains("/rename"), "{html}");
        // And a way back out that does not save.
        assert!(html.contains("Cancel"), "{html}");
    }

    /// A song with no title of its own opens its edit box holding its file name.
    ///
    /// A blank box in exactly this case makes the commonest correction on a real corpus -- turning
    /// `CORCOVAD` into `Corcovado` -- start by retyping the whole name from the row that was just
    /// clicked.
    #[test]
    fn a_song_named_after_its_file_opens_its_edit_box_holding_that_name() {
        let mut song = row("CORCOVAD", None, "Brasil/CORCOVAD.kar");
        song.from_filename = true;
        let html = SongRowFragment::new(song, true, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(
            html.contains("name=\"title\"") && html.contains("value=\"CORCOVAD\""),
            "the title box should open with the name the row was showing: {html}"
        );
        // And the artist beside it stays empty, because no artist is ever invented. Read from the
        // field's own name to the end of its tag, so the assertion is about the markup rather than
        // about how the template happens to be indented.
        let at = html.find("name=\"row_artist\"").expect("the artist box");
        let tag = &html[at..][..html[at..].find('>').expect("the tag ends")];
        assert!(
            tag.contains("value=\"\""),
            "a missing artist must not be filled in from anywhere: {tag}"
        );
    }

    #[test]
    fn the_play_button_shows_which_song_went_to_the_machine_last() {
        let song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");

        let plain = SongRowFragment::new(song.clone(), false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(
            plain.contains("id=\"play-abc123\""),
            "the button is addressable, which is what lets it be swapped later: {plain}"
        );
        assert!(plain.contains("play it on the karaoke machine"), "{plain}");

        let played = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .with_last_played(Some("abc123".to_owned()))
            .in_english()
            .expect("render");
        // Inside the play span, not merely somewhere on the row: the favorite star is also a
        // `primary` button when it is lit, so a whole-row search would pass on the wrong element.
        let span = played
            .split_once("id=\"play-abc123\"")
            .and_then(|(_, rest)| rest.split_once("</span>"))
            .map(|(inside, _)| inside)
            .unwrap_or_default();
        assert!(
            span.contains("class=\"primary\"") && span.contains("/play"),
            "the play button is the one highlighted: {played}"
        );
        assert!(
            played.contains("the last song sent to the karaoke machine"),
            "and it says why it is lit: {played}"
        );
    }

    #[test]
    fn playing_a_song_lights_its_button_and_puts_out_the_last_one() {
        let fragment = PlayedFragment {
            played_id: "new-song".to_owned(),
            unlit: vec!["old-song".to_owned()],
        };
        let html = fragment.in_english().expect("render");

        // Both buttons ride along out of band, and only the new one is lit.
        assert!(
            html.contains("id=\"play-new-song\" hx-swap-oob=\"true\""),
            "{html}"
        );
        assert!(
            html.contains("id=\"play-old-song\" hx-swap-oob=\"true\""),
            "{html}"
        );
        let new_button = html
            .split("id=\"play-old-song\"")
            .next()
            .unwrap_or_default();
        assert!(new_button.contains("class=\"primary\""), "{html}");
        let old_button = html
            .split("id=\"play-old-song\"")
            .nth(1)
            .unwrap_or_default();
        assert!(
            !old_button.contains("class=\"primary\""),
            "the previous button loses the highlight: {html}"
        );
    }

    #[test]
    fn playing_the_same_song_twice_swaps_one_button_and_not_two() {
        // Asking htmx to swap the same id twice in one response would replace the newly lit button
        // with the unlit one, so the handler passes nothing and this is what proves it renders that way.
        let html = PlayedFragment {
            played_id: "same-song".to_owned(),
            unlit: Vec::new(),
        }
        .in_english()
        .expect("render");
        assert_eq!(html.matches("hx-swap-oob").count(), 1, "{html}");
        assert!(html.contains("class=\"primary\""), "{html}");
    }

    #[test]
    fn a_play_puts_out_every_button_the_page_showed_lit() {
        // A page left behind by a play in another tab can show a song lit that the server no longer
        // records, and the page's own list is what reaches it.
        let html = PlayedFragment {
            played_id: "new-song".to_owned(),
            unlit: vec!["this-tab".to_owned(), "other-tab".to_owned()],
        }
        .in_english()
        .expect("render");
        assert_eq!(html.matches("hx-swap-oob").count(), 3, "{html}");
        assert_eq!(html.matches("class=\"primary\"").count(), 1, "{html}");
        for song in ["this-tab", "other-tab"] {
            assert!(
                html.contains(&format!("id=\"play-{song}\" hx-swap-oob=\"true\"")),
                "{html}"
            );
        }
    }

    #[test]
    fn only_a_lit_button_tells_the_next_play_it_is_lit() {
        let html = PlayedFragment {
            played_id: "new-song".to_owned(),
            unlit: vec!["old-song".to_owned()],
        }
        .in_english()
        .expect("render");
        assert_eq!(html.matches("class=\"lit-play\"").count(), 1, "{html}");
        assert!(
            html.contains("name=\"lit\" value=\"new-song\""),
            "the lit button carries its own id: {html}"
        );
        assert!(
            html.matches("hx-include=\".lit-play\"").count() == 2,
            "every button sends what is lit: {html}"
        );
    }

    /// The header ends with exactly one of Quit and Open in browser, and which is the window.
    ///
    /// Both halves are worth pinning because both are a way of being stuck. A window that still
    /// offers Quit has two buttons doing the same thing, which is only untidy — but a *browser* that
    /// stopped offering it leaves a tool started by double-clicking a corpus with no way out but the
    /// task manager, which is the failure the button was added for in the first place.
    #[test]
    fn the_header_offers_quit_in_a_browser_and_a_browser_in_a_window() {
        let page = |windowed: bool| {
            SongsPage {
                chrome: Chrome {
                    windowed,
                    ..chrome()
                },
                rows: rows(Vec::new()),
                favorites: Vec::new(),
                packages: Vec::new(),
                query: FilterForm::default(),
                chips: no_chips(),
                saved: no_saved(),
            }
            .in_english()
            .expect("render")
        };

        let tab = page(false);
        assert!(tab.contains(r#"hx-post="/quit""#), "{tab}");
        assert!(!tab.contains(r#"hx-post="/browser""#), "{tab}");

        let window = page(true);
        assert!(window.contains(r#"hx-post="/browser""#), "{window}");
        assert!(
            !window.contains(r#"hx-post="/quit""#),
            "closing the window already is quitting: {window}"
        );
    }

    /// A toast is one escaped line inside the element `#toasts` receives out of band.
    ///
    /// The escaping is the half worth pinning: this carries titles and paths out of a corpus nobody
    /// wrote, and a file called `<script>.kar` must read as a name rather than run as one. The
    /// out-of-band wrapper is the other half — without it the toast would swap into whatever the
    /// caller's `hx-target` was, which for a browse action is a slot above a page of rows.
    #[test]
    fn a_toast_is_escaped_and_lands_in_the_tray_out_of_band() {
        let response = toast_only(&Toast::good("Playing <b>Águas</b> on 127.0.0.1."));
        let html = std::str::from_utf8(&response_body(response))
            .expect("utf-8")
            .to_owned();

        assert!(
            html.contains("id=\"toasts\" hx-swap-oob=\"afterbegin\""),
            "it has to reach the tray rather than the caller's target: {html}"
        );
        assert!(html.contains("class=\"toast-item toast-good\""), "{html}");
        // askama spells its escapes numerically (`&#60;`), so what is pinned here is the thing that
        // actually matters: no `<b>` reaches the page as an element.
        assert!(
            !html.contains("<b>"),
            "markup out of a corpus reads as text: {html}"
        );
        // The accent survives as itself. It would not through an `HX-Trigger` header, which is why
        // this is a body -- see the `Toast` documentation.
        assert!(html.contains("Águas"), "{html}");
    }

    /// A play answers with the buttons *and* a toast, in one response.
    #[test]
    fn a_play_can_move_the_buttons_and_say_so_at_once() {
        let response = with_toast(
            &PlayedFragment {
                played_id: "new-song".to_owned(),
                unlit: Vec::new(),
            },
            &Toast::good("Playing on http://127.0.0.1:8177."),
            km_locale::Locale::English,
        );
        let html = std::str::from_utf8(&response_body(response))
            .expect("utf-8")
            .to_owned();
        assert!(
            html.contains("id=\"play-new-song\" hx-swap-oob=\"true\""),
            "{html}"
        );
        assert!(html.contains("Playing on http://127.0.0.1:8177."), "{html}");
    }

    /// The bytes of a response, for the two tests above.
    fn response_body(response: Response) -> Vec<u8> {
        futures_lite_block_on(async {
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body")
                .to_vec()
        })
    }

    /// Runs one future to completion on a current-thread runtime.
    ///
    /// These tests are synchronous and the only async thing in them is reading a body out of an
    /// `axum::Response`, so a runtime is cheaper than making every caller `#[tokio::test]`.
    fn futures_lite_block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    #[test]
    fn the_tooltip_names_the_best_copy_and_counts_the_rest() {
        let mut song = row("Corcovado", Some("Tom Jobim"), "z/CORCOVAD.kar");
        song.file_count = 3;
        song.paths = "z/CORCOVAD.kar\na/Corcovado - Tom Jobim.kar\nm/corcovado.kar".to_owned();
        // Re-worded: the paths and the count just changed, and the tooltip says both.
        song.say(km_locale::Locale::English, false);
        let html = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");

        assert!(
            html.contains("title=\"a/Corcovado - Tom Jobim.kar\n3 copies\""),
            "the recognizable name, then how many there are: {html}"
        );
        // The Copies column already carries the number, so nothing else in the row repeats it.
        assert!(
            !html.contains("+2 copies"),
            "no second badge next to the title: {html}"
        );
        // The other copies are not named. Six near-identical paths are what this replaced.
        for path in ["z/CORCOVAD.kar", "m/corcovado.kar"] {
            assert!(
                !html.contains(path),
                "{path} should not be listed in {html}"
            );
        }
    }

    /// The block says where in the list it is, because nothing else on the page knows.
    ///
    /// The filter bar deliberately carries no offset — that is what makes changing a filter start
    /// again at the top, which is right, because a different filter matches different songs. The two
    /// controls in that bar that change neither *which* songs match nor how many (the sort, and the
    /// file-name box) put the offset back on their own request, and this is where they read it from.
    /// It is a `data-` attribute rather than a hidden field on purpose: `#rows` is `hx-include`d
    /// whole by two actions, and a field here would arrive in both of their bodies.
    #[test]
    fn the_rows_block_says_which_page_it_is() {
        let mut block = rows(vec![row("Corcovado", Some("Tom Jobim"), "x/c.kar")]);
        block.offset = 150;
        block.total = 4200;
        let html = block.in_english().expect("render");
        assert!(html.contains("data-offset=\"150\""), "{html}");
        assert!(html.contains("data-total=\"4200\""), "{html}");
    }

    /// A page of rows can be ticked in one gesture, and the box that does it is never submitted.
    ///
    /// The `name` is the half worth pinning. `#rows` is `hx-include`d wholesale by the ticked-song
    /// actions and by *Title from file name*, so a named input in this table's head would ride along
    /// in both of those bodies as a field nobody wrote a reader for.
    #[test]
    fn the_table_head_offers_a_box_that_ticks_the_page() {
        let html = rows(vec![row("Corcovado", Some("Tom Jobim"), "x/c.kar")])
            .in_english()
            .expect("render");
        assert!(html.contains("class=\"select-all\""), "{html}");
        let head = html.split("</thead>").next().expect("a table head");
        assert!(
            !head.contains("name="),
            "the head's box must not be submitted:\n{head}"
        );
        // The other way of taking more than one row has nowhere else to be named: it is a gesture
        // rather than a control, so the box beside it is what a person hovers to find it.
        assert!(head.contains("shift-click"), "{head}");
        // And the rows it acts on are still the ones that are.
        assert!(html.contains("name=\"song_id\""), "{html}");
    }

    /// The switch is a class on the block, and the name itself is in every row either way.
    ///
    /// That is the whole mechanism: a row re-rendered on its own after an edit or a score is swapped
    /// back inside `#rows` and inherits the answer, so it cannot come back looking different from the
    /// others around it — which is what would happen if the row had to be told.
    #[test]
    fn file_names_are_shown_by_a_class_on_the_block_and_not_by_the_row() {
        let song = row("Corcovado", Some("Tom Jobim"), "Brasil/CORCOVAD.kar");

        let off = rows(vec![song.clone()]).in_english().expect("render");
        assert!(!off.contains("class=\"filenames\""), "{off}");

        let mut asked = rows(vec![song.clone()]);
        asked.show_filename = true;
        let on = asked.in_english().expect("render");
        assert!(on.contains("id=\"rows\" class=\"filenames\""), "{on}");

        // The name is in the markup whether or not it is being shown, and it is the base name
        // rather than the path.
        for html in [&off, &on] {
            assert!(html.contains(">CORCOVAD.kar<"), "{html}");
            assert!(!html.contains(">Brasil/CORCOVAD.kar<"), "{html}");
        }
        // Including in the fragment, which is never told which list it is going back into.
        let alone = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(alone.contains(">CORCOVAD.kar<"), "{alone}");
    }

    /// The warnings are the same switch, and the two boxes compose into one class attribute.
    ///
    /// **The both-on case is what this exists for.** Each box drawn by its own conditional in the
    /// markup would have the second one needing to know whether the first had opened the attribute
    /// and owed a space — so the names are joined in Rust and the markup asks once.
    #[test]
    fn warnings_are_shown_by_a_class_on_the_block_and_compose_with_file_names() {
        let mut song = row("Corcovado", Some("Tom Jobim"), "Brasil/CORCOVAD.kar");
        song.warnings =
            "[{\"code\":\"no_lyrics\",\"message\":\"nothing to sing from\"}]".to_owned();

        let off = rows(vec![song.clone()]).in_english().expect("render");
        assert!(!off.contains("class=\"warnings\""), "{off}");

        let mut asked = rows(vec![song.clone()]);
        asked.show_warnings = true;
        let on = asked.in_english().expect("render");
        assert!(on.contains("id=\"rows\" class=\"warnings\""), "{on}");

        let mut both = rows(vec![song.clone()]);
        both.show_filename = true;
        both.show_warnings = true;
        let html = both.in_english().expect("render");
        assert!(
            html.contains("id=\"rows\" class=\"filenames warnings\""),
            "{html}"
        );

        // The chips are in the markup whichever way the box is set, which is what lets a row
        // redrawn on its own inherit the answer. The code is the chip and the message its tooltip.
        for html in [&off, &on] {
            assert!(html.contains(">no_lyrics<"), "{html}");
            assert!(html.contains("nothing to sing from"), "{html}");
        }
        let alone = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(alone.contains(">no_lyrics<"), "{alone}");
    }

    /// A column that is not a warning list draws no chips rather than failing the page.
    ///
    /// Fifty rows are rendered at a time, so a row whose column cannot be read has to cost its own
    /// chips and nothing else — the rule the song's own page follows over the same text.
    #[test]
    fn a_warnings_column_that_cannot_be_read_draws_nothing() {
        let mut song = row("Corcovado", Some("Tom Jobim"), "Brasil/CORCOVAD.kar");
        song.warnings = "not json at all".to_owned();
        assert!(song.parsed_warnings().is_empty());

        let mut asked = rows(vec![song]);
        asked.show_warnings = true;
        let html = asked.in_english().expect("render");
        assert!(html.contains("Corcovado"), "{html}");
    }

    /// A song whose title already *is* its file name does not say so twice.
    #[test]
    fn a_song_named_after_its_file_gets_no_file_name_chip() {
        let mut song = row("CORCOVAD", None, "Brasil/CORCOVAD.kar");
        song.from_filename = true;
        let html = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        // The tag that explains where the title came from stays; the chip repeating it does not.
        assert!(html.contains(">file name<"), "{html}");
        assert!(!html.contains("class=\"tag mono filename\""), "{html}");
    }

    /// The count comes from `file_count`, not from counting the paths, so a scan that left the two
    /// out of step still agrees with the Copies column rather than quietly telling a different story.
    #[test]
    fn the_count_follows_the_copies_column_and_not_the_path_list() {
        let mut song = row("Corcovado", None, "Brasil/Corcovado - Tom Jobim.kar");
        song.file_count = 2;
        song.paths = "Brasil/Corcovado - Tom Jobim.kar".to_owned();
        // Re-worded: the paths and the count just changed, and the tooltip says both.
        song.say(km_locale::Locale::English, false);
        let html = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(
            html.contains("title=\"Brasil/Corcovado - Tom Jobim.kar\n2 copies\""),
            "{html}"
        );
    }

    #[test]
    fn the_star_asks_which_favorite_rather_than_filing_under_none() {
        let song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");
        let plain = SongRowFragment::new(song.clone(), false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        // The star opens the chooser. It must not post anything by itself: there is no favoriting
        // that does not name a favorite.
        assert!(plain.contains("/row?picking=1"), "{plain}");

        let favorites = vec![
            FavoriteNode {
                id: 1,
                name: "Bossa".to_owned(),
                song_count: 3,
                second_copies: 0,
                temporary: false,
            },
            FavoriteNode {
                id: 2,
                name: "Parties".to_owned(),
                song_count: 9,
                second_copies: 0,
                temporary: false,
            },
        ];
        let html = SongRowFragment::picking(
            song,
            favorites,
            vec![2],
            Choice::languages_in(&[], None),
            km_locale::Locale::English,
        )
        .in_english()
        .expect("render");
        assert!(html.contains("/songs/abc123/favorites/1?as=row"), "{html}");
        assert!(html.contains("/songs/abc123/favorites/2?as=row"), "{html}");
        assert!(
            html.contains("Bossa"),
            "each favorite is offered by the name it was made under: {html}"
        );
        assert!(
            html.contains("take it out of Parties"),
            "the one it is already in offers to unfile it: {html}"
        );
        assert!(
            html.contains("/favorites/new?as=row"),
            "and the first favorite can be made right here: {html}"
        );
    }

    /// The chooser opens under the song, and the song's own line stays on the page.
    ///
    /// **The row being filed has to be readable while the choice is made**, which is what the one
    /// `<tbody>` per song buys and the only thing the two `<tr>`s are for. The title is deliberately
    /// not repeated inside the chooser: it is on the line directly above it.
    #[test]
    fn the_favorite_chooser_opens_under_the_row_rather_than_over_it() {
        let song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");
        let html = SongRowFragment::picking(
            song,
            vec![FavoriteNode {
                id: 1,
                name: "Bossa".to_owned(),
                song_count: 3,
                second_copies: 0,
                temporary: false,
            }],
            vec![1],
            Choice::languages_in(&[], None),
            km_locale::Locale::English,
        )
        .in_english()
        .expect("render");

        assert!(html.contains(r#"<tbody id="row-abc123">"#), "{html}");
        assert_eq!(html.matches("<tr").count(), 2, "two lines, not one: {html}");
        // The song's own line is marked while its chooser is under it, so the pair takes one ground
        // and the rule between them goes -- `td.picker, tr.picked td` in style.css.
        assert!(html.contains(r#"<tr class="picked">"#), "{html}");
        let song_line = html.find("Corcovado").expect("the song's own line");
        let chooser = html.find("in which favorite?").expect("the chooser");
        assert!(song_line < chooser, "the song is still readable: {html}");
        assert_eq!(
            html.matches("Corcovado").count(),
            3,
            "the title is on the row and in its two search links, and not repeated in the chooser: {html}"
        );

        // Every control swaps the group, or half of the pair is left behind.
        assert!(!html.contains("closest tr"), "{html}");
        assert_eq!(html.matches("closest tbody").count(), 7, "{html}");

        // A favorite the song is in takes the star's gold; the accent is left to mean the action.
        assert!(html.contains(r#"class="filed""#), "{html}");
        // `&#38;` rather than `&amp;`: the label comes out of the catalog now, and askama escapes an
        // ampersand numerically. The two are the same character to a reader.
        assert!(
            html.contains(r#"<button class="primary">Create &#38; file</button>"#),
            "{html}"
        );
        assert!(html.contains(r#"class="row picker-actions""#), "{html}");

        // And the star that opened it closes it again.
        assert!(
            html.contains(r#"hx-get="/songs/abc123/row""#) && !html.contains("row?picking=1"),
            "{html}"
        );
    }

    /// The chooser offers the filings first and the working lists after a rule.
    ///
    /// A working list is where a song waits instead of being filed, so it is not mixed in among the
    /// lists that file it. The rule appears only where there is something on both sides of it.
    #[test]
    fn the_favorite_chooser_puts_working_lists_after_a_rule() {
        let node = |id, name: &str, temporary| FavoriteNode {
            id,
            name: name.to_owned(),
            song_count: 1,
            second_copies: 0,
            temporary,
        };
        let render = |favorites| {
            SongRowFragment::picking(
                row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar"),
                favorites,
                vec![],
                Choice::languages_in(&[], None),
                km_locale::Locale::English,
            )
            .in_english()
            .expect("render")
        };

        let html = render(vec![
            node(1, "Aside", true),
            node(2, "Bossa", false),
            node(3, "Parties", false),
        ]);
        let at = |needle: &str| html.find(needle).expect(needle);
        assert_eq!(html.matches("picker-rule").count(), 1, "{html}");
        assert!(
            html.contains(">working lists<"),
            "the divider names what follows it: {html}"
        );
        assert!(
            at("/favorites/2?as=row") < at("/favorites/3?as=row"),
            "{html}"
        );
        assert!(at("/favorites/3?as=row") < at("picker-rule"), "{html}");
        assert!(at("picker-rule") < at("/favorites/1?as=row"), "{html}");

        let filings = render(vec![node(1, "Bossa", false), node(2, "Parties", false)]);
        assert!(!filings.contains("picker-rule"), "{filings}");
        let working = render(vec![node(1, "Aside", true), node(2, "Later", true)]);
        assert!(!working.contains("picker-rule"), "{working}");
    }

    /// The favorite filter offers the filings first and the working lists in a group of their own.
    ///
    /// The group is drawn only when there is a working list to put in it.
    #[test]
    fn the_favorite_filter_groups_working_lists_after_the_filings() {
        let node = |id, name: &str, temporary| FavoriteNode {
            id,
            name: name.to_owned(),
            song_count: 1,
            second_copies: 0,
            temporary,
        };
        // The Favorites tab carries a select over the same lists, so read the filter's alone.
        let filter = |html: String| {
            let start = html
                .find(r#"<select name="favorite">"#)
                .expect("the filter");
            let end = start + html[start..].find("</select>").expect("its end");
            html[start..end].to_owned()
        };

        let html = filter(songs_page(
            vec![
                node(1, "Aside", true),
                node(2, "Bossa", false),
                node(3, "Parties", false),
            ],
            vec![],
        ));
        let at = |needle: &str| html.find(needle).expect(needle);
        assert!(at(">Bossa<") < at(">Parties<"), "{html}");
        assert!(at(">Parties<") < at("<optgroup"), "{html}");
        assert!(at("<optgroup") < at(">Aside<"), "{html}");
        assert!(
            html.contains(r#"<optgroup label="working lists">"#),
            "the group names what it holds: {html}"
        );

        let filings = filter(songs_page(
            vec![node(1, "Bossa", false), node(2, "Parties", false)],
            vec![],
        ));
        assert!(!filings.contains("<optgroup"), "{filings}");
    }

    /// The artist column comes first, in the head and in every row, and the sort does not follow it.
    ///
    /// **Two assertions and not one**, because the two halves live in different templates: a header
    /// swapped without the cells under it, or the reverse, is a table whose columns are labelled
    /// wrongly and reads as a corpus full of songs performed by their own titles.
    #[test]
    fn the_artist_column_comes_before_the_title_and_the_sort_stays_on_the_title() {
        let html = rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")])
            .in_english()
            .expect("render");

        let artist_head = html.find("<th>Artist</th>").expect("an Artist header");
        let title_head = html.find("<th>Title</th>").expect("a Title header");
        assert!(
            artist_head < title_head,
            "the headers are the old way round"
        );

        let artist_cell = html.find("Tom Jobim").expect("the artist in a row");
        let title_cell = html.find("Corcovado").expect("the title in a row");
        assert!(
            artist_cell < title_cell,
            "the cells did not follow their headers: {html}"
        );

        // The column order says what the page is for; the sort says what order the answers arrive
        // in. Moving the first must not move the second.
        assert_eq!(crate::db::Sort::default(), crate::db::Sort::Title);
    }

    #[test]
    fn a_row_in_no_favorite_shows_an_empty_star_and_one_in_several_shows_the_count() {
        let mut song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");
        let empty = SongRowFragment::new(song.clone(), false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(empty.contains("&#9734;"), "an outline star: {empty}");
        assert!(
            empty.contains(r#"class="star""#),
            "unfiled, so it takes the row's ordinary color: {empty}"
        );

        song.favorite_count = 3;
        song.permanent_count = 1;
        let filled = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(
            filled.contains("&#9733;3"),
            "filled, with the count: {filled}"
        );
        // The color and not only the fill. ★ against ☆ at row height is a few pixels of interior,
        // and this is the mark a whole column gets scanned for.
        assert!(
            filled.contains(r#"class="star filed""#),
            "filed, and gold for it: {filled}"
        );
    }

    /// A song set aside is a song still to do, and gold down that column means done.
    ///
    /// The two halves of the star come apart here and nowhere else: the fill answers *is this in a
    /// list*, which is true, and the color answers *has this been filed*, which is not.
    #[test]
    fn a_song_in_nothing_but_working_lists_gets_a_filled_star_and_no_gold() {
        let mut song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");
        song.favorite_count = 2;
        song.permanent_count = 0;
        // Re-worded, because the counts just changed and the button's sentence counts them. A page
        // does this in `State::say_rows`, after everything a row is going to be told.
        song.say(km_locale::Locale::English, false);
        let html = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(html.contains("&#9733;2"), "filled, with the count: {html}");
        assert!(
            html.contains(r#"class="star""#) && !html.contains(r#"class="star filed""#),
            "in two lists and filed in none, so no gold: {html}"
        );
        assert!(html.contains("working list"), "and it says so: {html}");
    }

    #[test]
    fn an_unrated_song_leaves_its_score_cell_blank() {
        let song = row("Corcovado", None, "a/CORCOVAD.kar");
        let html = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        // The empty option is the selected one, and there is no dash anywhere pretending to be a
        // value.
        assert!(html.contains("<option value=\"\"></option>"), "{html}");
        // …and the cell is not filled, which is the whole of what the fill means.
        assert!(
            html.contains(r#"class="mini""#),
            "an unrated score takes no fill: {html}"
        );
    }

    /// Runs of whitespace collapsed to one space, so an assertion is about the markup rather than
    /// about where the template happens to wrap a line.
    fn squeeze(html: &str) -> String {
        html.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn a_scored_song_shows_its_score_selected() {
        let mut song = row("Corcovado", None, "a/CORCOVAD.kar");
        song.user_score = Some(7);
        let html = squeeze(
            &SongRowFragment::new(song, false, Choice::languages_in(&[], None))
                .in_english()
                .expect("render"),
        );
        assert!(
            html.contains("<option value=\"7\" selected>7</option>"),
            "{html}"
        );
        // And only that one: one option, not eleven of them lit up.
        assert_eq!(html.matches("selected>").count(), 1, "{html}");
        // The cell carries the fill, which is what makes a rated song findable down a page of fifty
        // rows almost none of which anybody has rated.
        assert!(
            html.contains(r#"class="mini set""#),
            "a rated score is filled: {html}"
        );
    }

    #[test]
    fn a_title_that_is_only_a_filename_says_so_in_words_and_not_in_color() {
        let mut song = row("CORCOVAD", None, "a/CORCOVAD.kar");
        song.from_filename = true;
        let html = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(
            html.contains("file name"),
            "the tag is the whole signal: {html}"
        );
        // The dimming went: a gray link in a list of blue ones reads as *visited*, which is a
        // stronger and quite different claim from *this song has no title of its own*. Checked on the
        // title's own anchor, because `dim` is still the right class elsewhere in the row — the
        // em-dash standing in for a missing artist is one.
        let anchor = html
            .split_once("<a href=\"/songs/abc123\"")
            .and_then(|(_, rest)| rest.split_once("</a>"))
            .map(|(inside, _)| inside)
            .unwrap_or_default();
        assert!(
            !anchor.contains("dim"),
            "every title is the same color now: {anchor}"
        );
    }

    #[test]
    fn hovering_a_title_shows_where_the_file_is() {
        // One copy: the one path there is.
        let mut song = row("Corcovado", None, "Brasil/Corcovado - Tom Jobim.kar");
        song.file_count = 1;
        song.paths = "Brasil/Corcovado - Tom Jobim.kar".to_owned();
        // Re-worded, because the paths and the count just changed and the tooltip says both.
        song.say(km_locale::Locale::English, false);
        let html = SongRowFragment::new(song.clone(), false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(
            html.contains("title=\"Brasil/Corcovado - Tom Jobim.kar\""),
            "{html}"
        );

        // Several: the copy whose name reads like a title, and the count. Not all of them — the
        // Copies column says how many, and the hover says which one is worth knowing about.
        song.file_count = 2;
        song.paths = "z/CORCOVAD.KAR\nBrasil/Corcovado - Tom Jobim.kar".to_owned();
        // Re-worded: the paths and the count just changed, and the tooltip says both.
        song.say(km_locale::Locale::English, false);
        let html = SongRowFragment::new(song, false, Choice::languages_in(&[], None))
            .in_english()
            .expect("render");
        assert!(
            html.contains("title=\"Brasil/Corcovado - Tom Jobim.kar\n2 copies\""),
            "{html}"
        );
    }

    #[test]
    fn the_alphabet_bar_offers_every_letter_and_marks_the_chosen_one() {
        let form = FilterForm {
            initial: "C".to_owned(),
            ..FilterForm::default()
        };
        let initials = form.initials();
        // any, A-Z, one digit bucket, symbol — and not `1 + 26 + 10 + 1`: a button per digit slices
        // a bucket nobody browses that way, and costs a quarter of the bar's width to do it.
        assert_eq!(initials.len(), 1 + 26 + 1 + 1);
        assert!(initials[0].value.is_empty());
        assert_eq!(
            initials.iter().filter(|choice| choice.selected).count(),
            1,
            "exactly one is current"
        );
        assert!(
            initials
                .iter()
                .any(|choice| choice.value == "C" && choice.selected)
        );
        // The value is what the query string carries and the label is what the button reads. They are
        // the same for a letter and deliberately not for the last one.
        let last = initials.last().expect("the symbol bucket");
        assert_eq!((last.value.as_str(), last.label.as_str()), ("#", "symbol"));
        assert!(initials.iter().any(|choice| choice.value == "0-9"));
    }

    /// The language cell is a control, and its name is the load-bearing part.
    #[test]
    fn the_language_cell_is_editable_and_cannot_collide_with_the_filter_bar() {
        let portuguese = km_kmpkg::Language::parse("pt").expect("pt");
        let mut block = rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]);
        block.languages = Choice::languages_in(&[portuguese], None);
        let html = block.in_english().expect("render");

        assert!(
            html.contains("hx-post=\"/songs/abc123/language?as=row\""),
            "the cell posts to the per-song route: {html}"
        );
        // **Never `name="language"`.** This select rides inside `#rows`, which is included beside
        // `#filters` by *Title from file name* and by the ticked-song actions, and the bar has a
        // `language` key of its own — serde answers a repeated known key with `duplicate_field`, so
        // the wrong name here is a 400 on two buttons. A handlers-side test asserts the same rule
        // from the other end; this one stops the markup drifting back.
        assert!(html.contains("name=\"row_language\""), "{html}");
        assert!(!html.contains("name=\"language\""), "{html}");
        // The row's own language is preselected, from the short list rather than the standard.
        // Whitespace collapsed first: the assertion is about the markup, not about how the template
        // happens to be wrapped today.
        let flat = html.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("value=\"pt\" selected"), "{flat}");
        assert!(!html.contains("value=\"ja\""), "not in this corpus: {html}");
        // ...and the standard is still one click away.
        assert!(html.contains("value=\"more\""), "{html}");
    }

    /// A row asked for the full picker gets it, and only that row.
    #[test]
    fn the_language_row_mode_offers_the_whole_standard() {
        let song = row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar");
        let html = SongRowFragment::choosing_language(song, &[])
            .in_english()
            .expect("render");
        assert!(html.contains("value=\"ja\""), "every language: {html}");
        assert!(html.contains("value=\"cy\""), "{html}");
        // Not the short list's escape hatch — this *is* where it leads.
        assert!(!html.contains("value=\"more\""), "{html}");
        // And a way back that writes nothing.
        assert!(html.contains("hx-get=\"/songs/abc123/row\""), "{html}");
    }

    /// The suitability select offers four bands, marks one, and spells the low band both ways.
    ///
    /// The value/label split is the point: `0-4` is what the URL carries because a `<` is escaped by
    /// everything that touches one, and `<5` is what the option reads because that is how somebody
    /// thinks about it. Getting them the wrong way round is invisible until a link is copied.
    #[test]
    fn the_suitability_select_offers_the_three_bands_and_marks_the_chosen_one() {
        let form = FilterForm {
            suitability: "0-4".to_owned(),
            ..FilterForm::default()
        };
        let bands = form.suitabilities();
        assert_eq!(
            bands.iter().map(|b| b.value.as_str()).collect::<Vec<_>>(),
            ["", "8-10", "5-7", "0-4"]
        );
        assert_eq!(
            bands.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            ["any", "8-10", "5-7", "<5"]
        );
        assert_eq!(bands.iter().filter(|b| b.selected).count(), 1);
        assert!(bands.iter().any(|b| b.value == "0-4" && b.selected));

        // An unset filter marks *any* rather than leaving the select with nothing selected.
        let bands = FilterForm::default().suitabilities();
        assert!(bands.iter().any(|b| b.value.is_empty() && b.selected));
        assert_eq!(bands.iter().filter(|b| b.selected).count(), 1);

        // And the label reaches the markup escaped, which is the one thing `<5` risks.
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(html.contains("name=\"suitability\""), "{html}");
        assert!(html.contains("value=\"0-4\""), "{html}");
        // `&#60;` and not `&lt;` — askama escapes to numeric entities. Either is correct HTML; the
        // assertion is that the label is escaped at all, since it is the one option text here that
        // would otherwise start a tag.
        assert!(html.contains("&#60;5"), "{html}");
        assert!(
            !html.contains("<5<"),
            "the label reached the markup raw: {html}"
        );
        assert!(!html.contains("min_score"), "the old key is gone: {html}");
    }

    /// A range the address names is the fifth option, and the four bands are untouched.
    ///
    /// What this pins is the pair: the control has to agree with the rows, so a range draws itself,
    /// and the four standing options have to be the same four whatever the address says, so nobody
    /// reads a corpus-wide question off a select that quietly grew an option.
    #[test]
    fn a_range_in_force_draws_itself_beside_the_bands() {
        let form = FilterForm {
            suitability: "2-5".to_owned(),
            ..FilterForm::default()
        };
        let bands = form.suitabilities();
        assert_eq!(
            bands.iter().map(|b| b.value.as_str()).collect::<Vec<_>>(),
            ["", "8-10", "5-7", "0-4", "2-5"]
        );
        assert_eq!(
            bands.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            ["any", "8-10", "5-7", "<5", "2-5"]
        );
        assert_eq!(bands.iter().filter(|b| b.selected).count(), 1);
        assert!(bands.iter().any(|b| b.value == "2-5" && b.selected));

        // A range whose ends are a band's ends is that band, so no fifth option is drawn for it.
        let folded = FilterForm {
            suitability: "8-".to_owned(),
            ..FilterForm::default()
        };
        let bands = folded.suitabilities();
        assert_eq!(bands.len(), 4);
        assert!(bands.iter().any(|b| b.value == "8-10" && b.selected));

        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: form,
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(html.contains("value=\"2-5\" selected"), "{html}");
    }

    #[test]
    fn a_language_picker_offers_the_corpus_first_and_the_standard_below() {
        let portuguese = km_kmpkg::Language::parse("pt").expect("pt");
        let form = FilterForm {
            present: vec![portuguese],
            ..FilterForm::default()
        };
        // The filter select can only narrow, so it gets the short list alone.
        let filtering = form.languages();
        assert_eq!(filtering.len(), 1);
        assert_eq!(filtering[0].value, "pt");

        // The pickers that *set* a language get both, because otherwise the first Japanese song in
        // this corpus could never be classified — the language would have to be there already.
        assert_eq!(form.corpus_languages().len(), 1);
        assert!(form.every_language().len() > 100);
        assert!(
            form.every_language().iter().any(|c| c.value == "ja"),
            "a language the corpus does not hold is still reachable"
        );
    }

    #[test]
    fn a_language_named_in_the_url_is_offered_even_when_nothing_is_in_it() {
        // Otherwise the select shows the first option while the list is filtered by another, which is
        // a control lying about its own value. The chip is right either way; the select would not be.
        let form = FilterForm {
            present: vec![km_kmpkg::Language::parse("pt").expect("pt")],
            language: "cy".to_owned(),
            ..FilterForm::default()
        };
        let offered = form.languages();
        assert!(
            offered
                .iter()
                .any(|choice| choice.value == "cy" && choice.selected),
            "got {offered:?}"
        );
    }

    #[test]
    fn a_song_with_nothing_to_search_for_gets_no_youtube_link() {
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("CORCOVAD", None, "a/CORCOVAD.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(!html.contains("youtube.com"), "{html}");
    }

    #[test]
    fn a_song_with_an_artist_gets_one() {
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("CORCOVAD", Some("Tom Jobim"), "a/CORCOVAD.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(html.contains("youtube.com"), "{html}");
        assert!(html.contains("Tom+Jobim"), "{html}");
    }

    /// An artist in a row is a link to every other song by them; a row without one is not.
    ///
    /// **The em dash is the assertion that matters.** `artist_url` answers with an empty string for a
    /// song nobody recorded an artist for, and a template that linked anyway would put
    /// `href="/songs?artist="` on a dash — a filter matching nothing, on a control that looks like it
    /// leads somewhere. No artist is ever invented, and that includes inventing a link to one.
    #[test]
    fn a_hidden_version_is_marked_and_leads_to_the_shown_one() {
        let draw = |duplicate_of: Option<&str>| {
            let mut song = row("Corcovado", None, "a/Corcovado.kar");
            song.version_count = 3;
            song.duplicate_of = duplicate_of.map(ToOwned::to_owned);
            song.say(km_locale::Locale::English, false);
            SongsPage {
                chrome: chrome(),
                rows: rows(vec![song]),
                favorites: Vec::new(),
                packages: Vec::new(),
                query: FilterForm::default(),
                chips: no_chips(),
                saved: no_saved(),
            }
            .in_english()
            .expect("render")
        };

        let html = draw(Some("best1"));
        assert!(html.contains(r#"href="/songs/best1">hidden</a>"#), "{html}");
        assert!(html.contains("one of 3 files"), "{html}");

        let html = draw(None);
        assert!(!html.contains("hidden-version"), "{html}");
        assert!(html.contains("this row stands for 3 files"), "{html}");
    }

    #[test]
    fn an_artist_in_a_row_leads_to_every_song_by_them() {
        let draw = |artist: Option<&str>| {
            SongsPage {
                chrome: chrome(),
                rows: rows(vec![row("CORCOVAD", artist, "a/CORCOVAD.kar")]),
                favorites: Vec::new(),
                packages: Vec::new(),
                query: FilterForm::default(),
                chips: no_chips(),
                saved: no_saved(),
            }
            .in_english()
            .expect("render")
        };

        let html = draw(Some("Tom Jobim"));
        assert!(html.contains(r#"href="/songs?artist=Tom+Jobim""#), "{html}");

        let html = draw(None);
        assert!(!html.contains("/songs?artist="), "{html}");
    }

    /// An artist whose name carries a `&` or a quote reaches the filter intact.
    ///
    /// Two escapings meet in one attribute and neither can be skipped: the name is percent-encoded so
    /// it survives being a query parameter, and then HTML-escaped so it survives being an attribute.
    /// `Hall & Oates` is the case that fails visibly if the first is missing — `&Oates` starts a
    /// second parameter — and `Guns N' Roses` the one that fails if the second is.
    #[test]
    fn an_artist_with_punctuation_survives_both_escapings() {
        let draw = |artist: &str| {
            SongsPage {
                chrome: chrome(),
                rows: rows(vec![row("Song", Some(artist), "a/S.kar")]),
                favorites: Vec::new(),
                packages: Vec::new(),
                query: FilterForm::default(),
                chips: no_chips(),
                saved: no_saved(),
            }
            .in_english()
            .expect("render")
        };

        let html = draw("Hall & Oates");
        assert!(html.contains("artist=Hall+%26+Oates"), "{html}");

        let html = draw("Guns N' Roses");
        assert!(html.contains("artist=Guns+N%27+Roses"), "{html}");
    }

    #[test]
    fn markup_in_a_title_is_escaped_rather_than_rendered() {
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(vec![row("<script>x</script>", None, "a/b.kar")]),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(!html.contains("<script>x</script>"), "{html}");
        assert!(html.contains("&#60;script&#62;"), "{html}");
    }

    #[test]
    fn an_empty_result_says_so_rather_than_showing_a_bare_table() {
        let page = SongsPage {
            chrome: chrome(),
            rows: rows(Vec::new()),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        };
        let html = page.in_english().expect("render");
        assert!(html.contains("No matches"), "{html}");
        // No table means no page to turn to, at either end. The pager lives inside the same
        // `{% else %}` as the table for exactly this, and an include in the wrong branch would put
        // two empty button rows around a sentence saying there is nothing here.
        assert!(!html.contains("/songs/rows?"), "{html}");
    }

    /// The page turner is above the table **and** below it, from one included template.
    ///
    /// A page of rows is more than a screen, so a pager only at the bottom means scrolling the
    /// length of a page you have just read in order to leave it. Both copies come from
    /// `songs_pager.html`, and the count here is what says so: every button rendered twice, and half
    /// that number means somebody inlined one of them again.
    #[test]
    fn the_pager_is_above_the_table_and_below_it() {
        let mut block = rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]);
        block.total = 900;
        block.offset = 500;
        block.previous = "offset=400".to_owned();
        block.next = "offset=600".to_owned();
        block.first_page = "offset=0".to_owned();
        block.last_page = "offset=850".to_owned();
        // Page 11 of 18, so the window is 6..16 — ten buttons and the current page as a label.
        block.pages = (6..=16)
            .map(|number| crate::handlers::PageNumber {
                number,
                query: if number == 11 {
                    String::new()
                } else {
                    format!("offset={}", (number - 1) * 50)
                },
                current: number == 11,
            })
            .collect();

        let html = block.in_english().expect("render");
        assert_eq!(
            html.matches("hx-get=\"/songs/rows?").count(),
            28,
            "first, previous, ten numbers, next and last — twice over: {html}"
        );

        let table = html.find("<table").expect("a table");
        let closed = html.find("</table>").expect("a closed table");
        let first = html.find("/songs/rows?").expect("a first pager");
        let last = html.rfind("/songs/rows?").expect("a last pager");
        assert!(
            first < table,
            "the first pager is not above the table: {html}"
        );
        assert!(
            last > closed,
            "the last pager is not below the table: {html}"
        );
    }

    /// The same for the lyric search, numbered pages and all.
    ///
    /// A lyric search is ordered by how well the words matched, so the interesting place in it is
    /// where a half-remembered line stops being the best answer — which is a page somebody goes back
    /// to, and a strip of *next* buttons is as many presses as pages to reach. It pages 25 hits at a
    /// time against the browse list's 50, which is the only thing about the two that differs.
    #[test]
    fn a_lyric_search_pages_from_both_ends_too() {
        // Ninety hits at twenty-five a page is four pages, and this is the second. The window
        // reaches both ends, so there are no jump-to-the-end buttons beside the numbers.
        let numbered = |number: u32, current: bool| crate::handlers::PageNumber {
            number,
            query: match current {
                true => String::new(),
                false => format!("q=stars&offset={}", (number - 1) * 25),
            },
            current,
        };
        let hits = LyricHits {
            hits: vec![crate::model::LyricHit {
                song: row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar"),
                passage: "quiet nights of quiet stars".to_owned(),
            }],
            searched: true,
            indexed: true,
            total: 90,
            offset: 25,
            previous: "q=stars&offset=0".to_owned(),
            next: "q=stars&offset=50".to_owned(),
            first_page: String::new(),
            last_page: String::new(),
            pages: vec![
                numbered(1, false),
                numbered(2, true),
                numbered(3, false),
                numbered(4, false),
            ],
            range: String::new(),
            ratings: rating_choices(),
            languages: Choice::languages_in(&[], None),
            all_languages: Vec::new(),
            choosing_language: false,
            editing: false,
            picking: false,
            favorites: Vec::new(),
            last_played: None,
        };

        let html = hits.in_english().expect("render");
        assert_eq!(
            html.matches("hx-get=\"/lyrics/hits?").count(),
            10,
            "previous, next and three numbers, twice over: {html}"
        );
        // The page being shown is a label and not a button, by the browse pager's rule: a control
        // that reloads what is already on screen is one more thing to press by mistake.
        assert_eq!(
            html.matches(r#"<span class="here">2</span>"#).count(),
            2,
            "{html}"
        );
        let table = html.find("<table").expect("a table");
        let closed = html.find("</table>").expect("a closed table");
        assert!(
            html.find("/lyrics/hits?").expect("a first pager") < table,
            "the first pager is not above the table: {html}"
        );
        assert!(
            html.rfind("/lyrics/hits?").expect("a last pager") > closed,
            "the last pager is not below the table: {html}"
        );
    }

    /// Nothing in either pager carries an `id`, because both are in the document twice.
    ///
    /// Cheap to write and easy to undo by accident: an `id` added for a stylesheet or an `hx-target`
    /// would be duplicated the moment it was rendered, and a duplicate id is the kind of fault that
    /// shows up as one of the two pagers quietly not working rather than as an error.
    #[test]
    fn neither_pager_carries_an_id_to_be_duplicated() {
        let mut block = rows(vec![row("Corcovado", Some("Tom Jobim"), "a/CORCOVAD.kar")]);
        block.next = "offset=100".to_owned();
        block.previous = "offset=0".to_owned();
        let html = block.in_english().expect("render");

        // Everything after the table is the bottom pager and the two tags that close the block. The
        // rows themselves carry ids — `row-<id>`, `play-<id>` — which is why this looks at a slice
        // rather than counting across the whole thing.
        let below = &html[html.find("</table>").expect("a closed table")..];
        assert!(below.contains("/songs/rows?"), "the slice missed the pager");
        assert!(
            !below.contains("id=\""),
            "the pager declares an id: {below}"
        );
    }

    /// A folder opening at startup does not also offer the folder picker.
    ///
    /// **This is the bug, as a test.** Reopening last time's folder lands on the Open page, because
    /// there is no workspace yet and `require_workspace` sends every route here — and the page drew
    /// the progress line *and*, underneath it, the Recent list, the Browse button and the type-a-path
    /// box. Ten seconds of a large corpus's open with the folder chooser on screen, which is what
    /// this program shows when it has failed to open anything. A tool starting up correctly looked
    /// broken.
    ///
    /// It was never a race — `begin_open` fills the slot before the server answers a request — so the
    /// state and the page were both right and the template drew both at once.
    #[test]
    fn a_folder_opening_at_startup_does_not_also_offer_the_picker() {
        let page = |opening: Option<crate::server::OpeningView>| {
            OpenPage {
                locale: "en",
                opening,
                current: None,
                windowed: false,
                recent: vec![RecentView {
                    counts: String::new(),
                    name: "Kar".to_owned(),
                    path: "/tunes/karaoke".to_owned(),
                    songs: 12,
                    files: 40,
                    present: true,
                    indexed: crate::browse::Indexed::Yes,
                }],
                start: None,
            }
            .in_english()
            .expect("render")
        };

        let opening = page(Some(crate::server::OpeningView {
            root: "/tunes/karaoke".to_owned(),
            elapsed_secs: 0,
            phase: crate::db::OpeningPhase::UpToDate,
            finished: false,
            error: None,
            ..Default::default()
        }));
        assert!(
            opening.contains(r#"<div class="chooser" hidden>"#),
            "the picker is drawn under a running open: {opening}"
        );
        assert!(opening.contains("opening a folder"), "{opening}");
        assert!(
            !opening.contains("choose a folder of karaoke files"),
            "the header said the same wrong thing as the chooser: {opening}"
        );

        // Nothing running: the picker is the page, exactly as it was.
        let idle = page(None);
        assert!(idle.contains(r#"<div class="chooser">"#), "{idle}");
        assert!(idle.contains("choose a folder of karaoke files"), "{idle}");

        // A job that ended: whichever way it ended, the picker is wanted — success redirects away and
        // a failure needs somewhere to go next.
        let failed = page(Some(crate::server::OpeningView {
            root: "/tunes/karaoke".to_owned(),
            elapsed_secs: 0,
            phase: crate::db::OpeningPhase::Database,
            finished: true,
            error: Some("no database there".to_owned()),
            ..Default::default()
        }));
        assert!(failed.contains(r#"<div class="chooser">"#), "{failed}");
        // **And the reason is on the page, not only in the poll that would have carried it.** A job
        // that failed before anything could poll — one refused before it started, or one whose page
        // has been reloaded since — is drawn from here or nowhere.
        assert!(
            failed.contains(r#"<p class="message error">no database there</p>"#),
            "the page drew a chooser and no reason to be looking at it: {failed}"
        );
    }

    /// Each of the four things a remembered folder can be says which one it is, and only one is a
    /// button.
    ///
    /// **The third state had no words and drew as the first.** The row asked two questions — is it
    /// there, does it need renaming — so a folder still on disk whose `.kmbuild` had been deleted
    /// fell into the *else* and drew as an ordinary openable row, counts and all. Pressing it
    /// produced a refusal and left the row looking exactly the same.
    ///
    /// **It was already disagreeing with the startup reopen**, which skips anything that is not
    /// `Indexed::Yes` — so that folder was silently not reopened while this page offered it as the
    /// obvious thing to press. Both read one enum now, and this is what says so.
    #[test]
    fn every_state_a_remembered_folder_can_be_in_says_which_one_it_is() {
        let row = |present: bool, indexed: crate::browse::Indexed| {
            let mut row = RecentView {
                counts: String::new(),
                name: "Kar".to_owned(),
                path: "/tunes/karaoke".to_owned(),
                songs: 12,
                files: 40,
                present,
                indexed,
            };
            row.say_counts(km_locale::Locale::English);
            row
        };
        let draw = |row: RecentView| {
            OpenPage {
                locale: "en",
                opening: None,
                current: None,
                windowed: false,
                recent: vec![row],
                start: None,
            }
            .in_english()
            .expect("render")
        };

        // **The name cell is what says pressable, and `hx-post="/open/open"` is not the way to ask.**
        // The type-a-path form at the foot of the page posts there too, so every render contains that
        // string whatever the row is — which is how the first version of this test passed the row it
        // was meant to catch. The row's own button is the only `<button class="link">` on the page.
        let pressable = |html: &str| html.contains(r#"<button class="link" hx-post="/open/open""#);

        // Ready: the only one that is a button, and the only one showing counts.
        let ready = draw(row(true, crate::browse::Indexed::Yes));
        assert!(pressable(&ready), "{ready}");
        assert!(ready.contains("12 songs"), "{ready}");
        assert!(!ready.contains("class=\"warn\""), "{ready}");

        // The folder is there and the curation is gone. **Not** "not found": the songs are still on
        // the disk, and somebody who reads that goes looking for a missing drive.
        let gone = draw(row(true, crate::browse::Indexed::No));
        assert!(gone.contains("its curation database is gone"), "{gone}");
        assert!(!gone.contains("not found"), "{gone}");
        assert!(
            !pressable(&gone),
            "an action whose only outcome is a refusal is worse than no action: {gone}"
        );
        assert!(
            !gone.contains("12 songs"),
            "the counts are what it had, and it no longer has them: {gone}"
        );
        assert!(gone.contains("li class=\"missing\""), "{gone}");

        // The drive is unplugged. Kept and dimmed — an unplugged disk is not a reason to forget a
        // year of curation — and it says the other thing.
        let absent = draw(row(false, crate::browse::Indexed::No));
        assert!(absent.contains("not found"), "{absent}");
        assert!(
            !absent.contains("its curation database is gone"),
            "{absent}"
        );
        assert!(!pressable(&absent), "{absent}");
        assert!(absent.contains("li class=\"missing\""), "{absent}");
    }

    /// The row and the startup reopen must agree about which folders are openable.
    ///
    /// `folder_to_reopen` tests `browse::indexed(..) == Indexed::Yes`. This asserts the predicate the
    /// page uses is the same question, over every value the enum has.
    #[test]
    fn a_row_is_openable_exactly_when_the_startup_reopen_would_take_it() {
        for indexed in [crate::browse::Indexed::Yes, crate::browse::Indexed::No] {
            let row = RecentView {
                counts: String::new(),
                name: "Kar".to_owned(),
                path: "/tunes/karaoke".to_owned(),
                songs: 0,
                files: 0,
                present: true,
                indexed,
            };
            assert_eq!(
                row.openable(),
                indexed == crate::browse::Indexed::Yes,
                "{indexed:?} is offered as a press but would not be reopened, or the other way round"
            );
        }
    }

    /// An open that ends without a redirect puts the picker back, by name.
    ///
    /// **The fragment swaps into `#opening` and the chooser is its sibling**, so hiding the chooser
    /// server-side is only half of it: without these the page would be left showing one red sentence
    /// and nothing to act on — a worse state than the one this whole change is fixing.
    #[test]
    fn a_failed_open_brings_the_picker_back() {
        let progress = |opening: Option<crate::server::OpeningView>, open: bool| {
            OpenProgress::new(opening, open, km_locale::Locale::English)
                .in_english()
                .expect("render")
        };

        let failed = progress(
            Some(crate::server::OpeningView {
                root: "/tunes/karaoke".to_owned(),
                elapsed_secs: 0,
                phase: crate::db::OpeningPhase::Database,
                finished: true,
                error: Some("no database there".to_owned()),
                ..Default::default()
            }),
            false,
        );
        assert!(failed.contains("no database there"), "{failed}");
        assert!(failed.contains(r#"removeAttribute("hidden")"#), "{failed}");

        // Success goes to the songs list, so it has no picker to restore.
        let done = progress(
            Some(crate::server::OpeningView {
                root: "/tunes/karaoke".to_owned(),
                elapsed_secs: 0,
                phase: crate::db::OpeningPhase::Database,
                finished: true,
                error: None,
                ..Default::default()
            }),
            false,
        );
        assert!(done.contains(r#"window.location = "/songs""#), "{done}");
        assert!(!done.contains("removeAttribute"), "{done}");

        // Still running: it polls itself and touches nothing.
        let running = progress(
            Some(crate::server::OpeningView {
                root: "/tunes/karaoke".to_owned(),
                elapsed_secs: 0,
                phase: crate::db::OpeningPhase::UpToDate,
                finished: false,
                error: None,
                ..Default::default()
            }),
            false,
        );
        assert!(running.contains("/open/progress"), "{running}");
        assert!(!running.contains("removeAttribute"), "{running}");
    }

    /// Every page shell carries the tray and the script, or a failure has nowhere to be said.
    ///
    /// The failure this pins is silent in both directions: `ui.js` bails when `#toasts` is missing,
    /// and a page with the tray but no script has an empty div. Either way a request that failed
    /// changes nothing on screen — which is the exact state the script was written to end, and it
    /// would come back by way of an edit to a template that has nothing to do with errors.
    ///
    /// Both shells, because `open.html` deliberately does not extend `layout.html` and so is the one
    /// page that can lose this without anything else noticing. It is also the page where opening a
    /// folder that has gone away is most likely to fail.
    #[test]
    fn both_page_shells_carry_the_toast_tray_and_the_script() {
        let browse = SongsPage {
            chrome: chrome(),
            rows: rows(Vec::new()),
            favorites: Vec::new(),
            packages: Vec::new(),
            query: FilterForm::default(),
            chips: no_chips(),
            saved: no_saved(),
        }
        .in_english()
        .expect("render");

        let picker = OpenPage {
            locale: "en",
            opening: None,
            current: None,
            // A browser, which is what a `cargo test` build is.
            windowed: false,
            recent: Vec::new(),
            start: None,
        }
        .in_english()
        .expect("render");

        for (name, html) in [("layout.html", &browse), ("open.html", &picker)] {
            assert!(
                html.contains("id=\"toasts\""),
                "{name} has no tray for a toast to land in"
            );
            assert!(
                html.contains("/static/ui.js"),
                "{name} does not load the script that fills it"
            );
        }
    }

    #[test]
    fn a_failed_message_still_renders_its_text() {
        let fragment = MessageFragment {
            text: "the karaoke machine is not answering".to_owned(),
            ok: false,
        };
        let html = fragment.in_english().expect("render");
        assert!(html.contains("not answering"));
        assert!(html.contains("error"), "{html}");
    }

    /// The one that makes the lyric page safe: a passage is pairs, never markup.
    #[test]
    fn a_marked_passage_becomes_runs_of_matched_and_unmatched_text() {
        let passage = format!("was {MATCH_START}white{MATCH_END} as snow");
        assert_eq!(
            highlight(&passage),
            vec![
                ("was ".to_owned(), false),
                ("white".to_owned(), true),
                (" as snow".to_owned(), false),
            ]
        );

        // Several matches in one passage, and a passage that starts on one.
        let two = format!("{MATCH_START}Mary{MATCH_END} had a little {MATCH_START}lamb{MATCH_END}");
        assert_eq!(
            highlight(&two),
            vec![
                ("Mary".to_owned(), true),
                (" had a little ".to_owned(), false),
                ("lamb".to_owned(), true),
            ]
        );

        // Nothing marked at all — a passage FTS5 elided around, or the ellipsis on its own.
        assert_eq!(
            highlight("\u{2026}nothing here\u{2026}"),
            vec![("\u{2026}nothing here\u{2026}".to_owned(), false)]
        );
        assert!(highlight("").is_empty());
    }

    /// Markup in a lyric stays text, which is the whole reason the passage is not HTML.
    ///
    /// A large corpus somebody else made is exactly where a `<script>` in a
    /// title event turns up, and the page is rendered into a browser.
    #[test]
    fn a_lyric_that_looks_like_markup_is_escaped_not_run() {
        let hits = LyricHits {
            hits: vec![LyricHit {
                song: row("<script>alert(1)</script>", None, "evil.kar"),
                passage: format!("sings {MATCH_START}<script>{MATCH_END} loudly"),
            }],
            searched: true,
            indexed: true,
            total: 1,
            offset: 0,
            previous: String::new(),
            next: String::new(),
            first_page: String::new(),
            last_page: String::new(),
            pages: Vec::new(),
            range: String::new(),
            ratings: rating_choices(),
            languages: Choice::languages_in(&[], None),
            all_languages: Vec::new(),
            choosing_language: false,
            editing: false,
            picking: false,
            favorites: Vec::new(),
            last_played: None,
        };
        let html = hits.in_english().expect("render");
        assert!(
            !html.contains("<script>"),
            "a lyric became markup, which is the bug this design exists to make impossible:\n{html}"
        );
        // askama escapes with numeric entities, so this is what "shown as text" looks like.
        assert!(html.contains("&#60;script&#62;"), "{html}");
        // The emphasis is still there — escaping must not have cost the highlight.
        assert!(html.contains("<mark>"), "{html}");
    }

    /// The three ways of showing nothing say three different things.
    #[test]
    fn an_empty_result_says_which_kind_of_empty_it_is() {
        let empty = |searched: bool, indexed: bool| LyricHits {
            hits: Vec::new(),
            searched,
            indexed,
            total: 0,
            offset: 0,
            previous: String::new(),
            next: String::new(),
            first_page: String::new(),
            last_page: String::new(),
            pages: Vec::new(),
            range: String::new(),
            ratings: rating_choices(),
            languages: Choice::languages_in(&[], None),
            all_languages: Vec::new(),
            choosing_language: false,
            editing: false,
            picking: false,
            favorites: Vec::new(),
            last_played: None,
        };

        let untouched = empty(false, true).in_english().expect("render");
        assert!(untouched.contains("Type a line"), "{untouched}");

        // The one that matters: an old database needs a full re-scan, and telling it to try a
        // shorter phrase would send somebody retyping forever.
        let unindexed = empty(true, false).in_english().expect("render");
        assert!(unindexed.contains("re-read every file"), "{unindexed}");
        assert!(unindexed.contains("/scan"), "{unindexed}");

        let no_match = empty(true, true).in_english().expect("render");
        assert!(no_match.contains("Not found"), "{no_match}");
        assert!(!no_match.contains("re-read every file"), "{no_match}");
    }

    #[test]
    fn suitabilities_are_classed_by_how_good_they_are() {
        assert_eq!(suitability_class(&Some(0)), "low");
        assert_eq!(suitability_class(&Some(4)), "low");
        assert_eq!(suitability_class(&Some(5)), "mid");
        assert_eq!(suitability_class(&Some(8)), "high");
        // A video song, which has no automatic suitability at all. Its own class rather than `low`: a
        // color that says "measured and found wanting" would be a lie about a thing never measured.
        assert_eq!(suitability_class(&None), "none");
    }

    /// The Open page draws no directory listing, and offers a button that fetches one.
    ///
    /// **Drawn open**, arriving at the picker would read a directory and put the whole of it on
    /// screen below the two lists people actually use — and the directory it reads is the corpus's
    /// own folder, which on a working machine holds hundreds of thousands of files.
    #[test]
    fn the_open_page_offers_the_browser_rather_than_drawing_it() {
        let html = OpenPage {
            locale: "en",
            opening: None,
            recent: Vec::new(),
            start: Some(r"D:\tunes".to_owned()),
            current: None,
            // A browser, which is what a `cargo test` build is.
            windowed: false,
        }
        .in_english()
        .expect("render");

        assert!(html.contains(r#"hx-get="/open/list""#), "{html}");
        assert!(
            !html.contains(r#"class="folders""#),
            "the walker is not drawn until it is asked for: {html}"
        );
        // The typed-path box still shows where the browser would start, which is a string and never
        // needed the directory read.
        assert!(html.contains(r#"placeholder="D:\tunes""#), "{html}");
    }

    /// The Build tab picks its own volume, and names the file and the route for the one it picked.
    #[test]
    fn a_package_of_two_volumes_picks_the_one_to_build_on_the_build_tab() {
        let volume = |number: u32, count: u32| PackageRow {
            volume: number,
            volumes: 2,
            song_count: count,
            volume_id: format!("{number}f4a9c8e2b7d0356"),
            ..PackageRow::new("1f4a9c8e2b7d0356", "Brasil")
        };
        let volumes = [volume(1, 999), volume(2, 12)];
        let pane = BuildPane::new(
            std::path::Path::new("/corpus"),
            volumes[1].clone(),
            &volumes,
            true,
            false,
            km_locale::Locale::English,
        )
        .in_english()
        .expect("render");
        assert!(pane.contains(r#"id="build-volume""#), "{pane}");
        assert!(
            pane.contains(r#"<option value="2" selected>Volume 2 · 12 songs</option>"#),
            "{pane}"
        );
        assert!(pane.contains("brasil-vol2-1.0.0.kmpkg"), "{pane}");
        assert!(
            pane.contains(r#"hx-post="/packages/1f4a9c8e2b7d0356/build?volume=2""#),
            "{pane}"
        );

        // One volume draws no picker at all.
        let single = BuildPane::new(
            std::path::Path::new("/corpus"),
            PackageRow::new("1f4a9c8e2b7d0356", "Brasil"),
            &[PackageRow::new("1f4a9c8e2b7d0356", "Brasil")],
            true,
            false,
            km_locale::Locale::English,
        )
        .in_english()
        .expect("render");
        assert!(!single.contains("build-volume"), "{single}");
    }

    /// The install button comes back enabled with the last frame of a build that wrote something.
    ///
    /// **Its `disabled` is read when the page is drawn and a build swaps only `#build-progress`**, so
    /// without this a finished build said in as many words that the file was there and left the one
    /// button for installing it grayed out until somebody reloaded.
    #[test]
    fn a_finished_build_brings_the_install_button_back_enabled() {
        let form = |built, oob| {
            InstallForm {
                package_id: "vol1".to_owned(),
                volume: 1,
                built,
                oob,
            }
            .in_english()
            .expect("render")
        };

        let fresh = form(false, false);
        assert!(fresh.contains("disabled"), "{fresh}");
        assert!(
            !fresh.contains("hx-swap-oob"),
            "in band on the page: {fresh}"
        );

        let after = form(true, true);
        assert!(!after.contains("disabled"), "{after}");
        assert!(
            after.contains(r#"id="install-form" hx-swap-oob="true""#),
            "it has to name where it lands: {after}"
        );
    }

    /// Every `hx-vals` on the Open page has to be JSON a browser can parse, backslashes and all.
    ///
    /// **This is a bug that shipped**, and it is invisible on a machine whose paths use `/`. An
    /// `hx-vals` attribute is JSON, so a path concatenated into one meets askama's HTML escaper,
    /// which leaves a backslash alone — right for HTML — and `D:\tunes\karaoke` reaches htmx as
    /// `{"path": "D:\tunes\karaoke"}`, where `\t` is a tab and `\k` is not an escape at all.
    /// `JSON.parse` threw, htmx swallowed it and sent the POST with an **empty body**, and the
    /// handler answered "No folder given." on every click of a remembered folder. Typing the same
    /// path into the box below worked, because that is a real form field and never JSON.
    ///
    /// **Both shapes are here on purpose, because only one of them fails loudly.** A path whose
    /// escapes all happen to be *legal* JSON — `\n` in `…\new` — parses fine and posts a silently
    /// **mangled** path, which presents as "is not a folder" instead. A test that only asked
    /// whether the attribute parses would pass on it.
    #[test]
    fn a_windows_path_survives_the_hx_vals_attribute() {
        let page = OpenPage {
            locale: "en",
            opening: None,
            recent: vec![RecentView {
                counts: String::new(),
                path: r"D:\tunes\karaoke".to_owned(),
                name: "karaoke".to_owned(),
                songs: 12,
                files: 34,
                present: true,
                indexed: crate::browse::Indexed::Yes,
            }],
            start: Some(r"D:\tunes".to_owned()),
            current: None,
            // A browser, which is what a `cargo test` build is.
            windowed: false,
        };
        let listing = OpenListing {
            listing: crate::browse::Listing {
                here: Some(r"D:\tunes".to_owned()),
                parent: Some(r"D:\".to_owned()),
                rows: vec![crate::browse::Row {
                    name: "new".to_owned(),
                    path: r"D:\tunes\new".to_owned(),
                    indexed: crate::browse::Indexed::Yes,
                }],
                indexed: crate::browse::Indexed::Yes,
                error: None,
                total: 1,
                ..Default::default()
            },
        };
        // **Two renders, because the page and the listing are two templates.** The same defect
        // reaches the recent list and the walker alike, so both have to be covered: the fragment
        // is rendered beside the page rather than the walker losing its half.
        let html = format!(
            "{}{}",
            page.in_english().expect("render"),
            listing.in_english().expect("render")
        );

        // The `json` filter escapes `'` as `\u0027`, so the attribute's own delimiter cannot appear
        // inside a value and splitting on it is exact rather than approximate.
        let mut seen = Vec::new();
        for tail in html.split("hx-vals='").skip(1) {
            let value = tail.split('\'').next().expect("an unterminated hx-vals");
            let parsed: serde_json::Value = serde_json::from_str(value)
                .unwrap_or_else(|error| panic!("hx-vals is not JSON: {value} — {error}"));
            if let Some(path) = parsed.get("path").and_then(|path| path.as_str()) {
                seen.push(path.to_owned());
            }
        }

        // Not a length check for its own sake: markup that stopped carrying `hx-vals` at all would
        // otherwise pass this test by leaving it nothing to look at.
        assert!(seen.len() >= 3, "no folder buttons rendered: {seen:?}");
        for expected in [r"D:\tunes\karaoke", r"D:\tunes", r"D:\tunes\new"] {
            assert!(
                seen.iter().any(|path| path == expected),
                "{expected} did not survive the attribute: {seen:?}"
            );
        }
    }

    /// The picker starts polling a job it did not start — and only such a job.
    ///
    /// **The negative halves are the point.** Two of the three ways to open a folder begin before
    /// any page exists (the startup reopen, and a double-clicked corpus on macOS), so the picker has
    /// to pick up a running job on load; but an *unconditional* trigger would fetch
    /// `/open/progress` for somebody who reached this page deliberately from an open folder, and
    /// that fragment's no-job-but-one-is-open arm would bounce them straight back to `/songs`. The
    /// picker would be unreachable while a folder is open, which is a worse fault than the one being
    /// fixed and would look nothing like this change.
    #[test]
    fn the_picker_polls_a_job_it_did_not_start_and_nothing_else() {
        let picker = |opening| {
            OpenPage {
                locale: "en",
                opening,
                current: None,
                // A browser, which is what a `cargo test` build is.
                windowed: false,
                recent: Vec::new(),
                start: None,
            }
            .in_english()
            .expect("render")
        };
        let job = |finished| {
            Some(crate::server::OpeningView {
                root: r"D:\tunes\karaoke".to_owned(),
                elapsed_secs: 0,
                phase: crate::db::OpeningPhase::Database,
                finished,
                error: None,
                ..Default::default()
            })
        };

        let running = picker(job(false));
        assert!(
            running.contains(r#"hx-get="/open/progress""#),
            "the picker does not poll a folder that is being opened: {running}"
        );
        assert!(
            running.contains(r"D:\tunes\karaoke"),
            "the picker does not name the folder being opened: {running}"
        );

        for (what, html) in [
            ("no job", picker(None)),
            ("a finished job", picker(job(true))),
        ] {
            assert!(
                !html.contains("/open/progress"),
                "the picker polls with {what}, which bounces anyone who came here from an open \
                 folder: {html}"
            );
        }
    }

    /// A folder that is opening takes the page over, and the chooser is there to be taken over.
    ///
    /// The complaint: opening the owner's corpus runs the migrations, the browse indexes, three
    /// backfills and a full `ANALYZE`, and the page went on showing the Recent list and the file
    /// browser under a one-line message — so a press that would take minutes read as a press that
    /// had done nothing.
    ///
    /// **What is asserted here is the two halves the CSS rule joins**, since a stylesheet is not
    /// something a template test can evaluate: the marker class on the running arm, and a `.chooser`
    /// for the rule to hide. Either one renamed on its own breaks the page silently.
    #[test]
    fn a_folder_that_is_opening_takes_over_the_picker() {
        let picker = |opening| {
            OpenPage {
                locale: "en",
                opening,
                current: None,
                windowed: false,
                recent: Vec::new(),
                start: None,
            }
            .in_english()
            .expect("render")
        };
        let job = |finished| {
            Some(crate::server::OpeningView {
                root: r"D:\tunes\karaoke".to_owned(),
                elapsed_secs: 0,
                phase: crate::db::OpeningPhase::Database,
                finished,
                error: None,
                ..Default::default()
            })
        };

        let running = picker(job(false));
        assert!(
            running.contains(r#"class="message opening-now polling""#),
            "nothing marks the page as busy, so the chooser stays under it — and `polling` is what \
             keeps the panel from dimming on every poll: {running}"
        );
        assert!(
            running.contains(r#"class="chooser""#),
            "there is nothing for the rule to hide: {running}"
        );

        // A page with nothing running keeps the chooser and carries no marker, which is what makes
        // the rule a no-op the rest of the time.
        let idle = picker(None);
        assert!(idle.contains(r#"class="chooser""#), "{idle}");
        assert!(!idle.contains("opening-now"), "{idle}");

        // And the stylesheet holds up its half. Asserted against the shipped file rather than
        // trusted, because the two names are written three times between them and nothing else
        // would notice a rename.
        let css = include_str!("../static/style.css");
        assert!(
            css.contains("main.open:has(#opening .opening-now) .chooser"),
            "the rule that hides the chooser is not in the stylesheet"
        );
    }

    /// A panel that polls for itself does not wear the pressed dim.
    ///
    /// **Both halves, because either alone reads as correct.** htmx marks the requesting element
    /// `htmx-request`, and on these three panels the requesting element is the panel — so
    /// `.htmx-request`'s 55% lands on the only thing on the screen, once per poll, for the length
    /// of an open. Dropping the class off a template or the exemption out of the stylesheet each
    /// brings that blink back, and a render test reading only one of them sees neither.
    #[test]
    fn a_panel_that_polls_for_itself_is_not_dimmed_as_though_it_were_pressed() {
        let css = include_str!("../static/style.css");
        assert!(
            css.contains(".polling.htmx-request { opacity: 1; }"),
            "the exemption is gone from the stylesheet, so every progress panel blinks again"
        );

        // The Open page's two arms are asserted against *rendered* output, because they are the
        // ones the reported blink was on and a class in a template is not proof it reaches a
        // browser: both sit behind a `{% match %}` on a running job.
        let job = Some(crate::server::OpeningView {
            root: r"D:\tunes\karaoke".to_owned(),
            elapsed_secs: 0,
            phase: crate::db::OpeningPhase::Database,
            finished: false,
            error: None,
            ..Default::default()
        });
        let fragment = OpenProgress::new(job, false, km_locale::Locale::English)
            .in_english()
            .expect("render");
        assert!(
            fragment.contains(r#"class="message opening-now polling""#),
            "the polling arm of /open/progress does not claim the exemption: {fragment}"
        );

        // And the other two, which poll the same way at 1s and were fixed with it. Rendered at
        // `running: true` and again at `false`, because on these the class shares the `{% if %}`
        // with the polling attributes: a panel that is not polling has nothing to be exempted from,
        // and a class left on it would be a claim nobody could check.
        let scanning = |running| {
            ProgressFragment {
                progress: crate::scan::ProgressView::default(),
                running,
            }
            .in_english()
            .expect("render")
        };
        let building = |running| {
            BuildProgressFragment::new(
                "vol1".to_owned(),
                1,
                None,
                running,
                None,
                false,
                km_locale::Locale::English,
            )
            .in_english()
            .expect("render")
        };

        for (what, panel, class) in [
            ("scan", scanning(true), r#"class="panel polling""#),
            ("build", building(true), r#"class="polling""#),
        ] {
            assert!(
                panel.contains(class),
                "the {what} panel polls without claiming the exemption, so it dims once a second: \
                 {panel}"
            );
        }
        for (what, panel) in [("scan", scanning(false)), ("build", building(false))] {
            assert!(
                !panel.contains("polling"),
                "the {what} panel is not polling, so it must not claim an exemption: {panel}"
            );
        }
    }

    /// The scan panel lists the steps to come, offers Stop while it can, and says when it is stopping.
    #[test]
    fn the_scan_panel_lists_its_steps_and_offers_stop_while_running() {
        let draw = |progress: crate::scan::ProgressView, running| {
            ProgressFragment { progress, running }
                .in_english()
                .expect("render")
        };
        let mut progress = crate::scan::ProgressView {
            phase: crate::scan::phase::READING.to_owned(),
            steps: [
                (crate::scan::phase::LOOKING, "done", false),
                (crate::scan::phase::READING, "running", false),
                (crate::scan::phase::MEASURING, "waiting", true),
            ]
            .into_iter()
            .map(|(key, state, if_changed)| crate::step::StepView {
                key: key.to_owned(),
                state,
                took: (state != "waiting").then(|| "2.0 s".to_owned()),
                if_changed,
            })
            .collect(),
            rate: Some(17.3),
            remaining: Some(std::time::Duration::from_secs(600)),
            ..crate::scan::ProgressView::default()
        };
        progress.say_counts(km_locale::Locale::English);

        let running = draw(progress.clone(), true);
        assert!(running.contains(r#"class="step done""#), "{running}");
        assert!(running.contains(r#"class="step waiting""#), "{running}");
        assert!(
            running.contains("measuring the corpus for the query planner"),
            "the step to come is named: {running}"
        );
        assert!(running.contains("only if something changed"), "{running}");
        assert!(running.contains("17 files a second"), "{running}");
        assert!(running.contains("about 10 m left"), "{running}");
        assert!(running.contains(r#"hx-post="/scan/stop""#), "{running}");

        progress.stopping = true;
        let stopping = draw(progress.clone(), true);
        assert!(!stopping.contains("/scan/stop"), "{stopping}");
        assert!(stopping.contains("Stopping"), "{stopping}");

        let over = draw(progress.clone(), false);
        assert!(
            !over.contains("/scan/stop"),
            "a run that is over has nothing to stop: {over}"
        );
        assert!(!over.contains("succeeded"), "{over}");
    }

    /// Only a run that reached its end is drawn as done: a stopped or failed one ended too.
    #[test]
    fn the_scan_panel_marks_only_a_run_that_reached_its_end() {
        let draw = |canceled, error: Option<&str>| {
            ProgressFragment {
                progress: crate::scan::ProgressView {
                    phase: crate::scan::phase::FINISHED.to_owned(),
                    finished: true,
                    canceled,
                    error: error.map(ToOwned::to_owned),
                    ..crate::scan::ProgressView::default()
                },
                running: false,
            }
            .in_english()
            .expect("render")
        };
        assert!(draw(false, None).contains(r#"class="panel succeeded""#));
        assert!(!draw(true, None).contains("succeeded"));
        assert!(!draw(false, Some("the disk went away")).contains("succeeded"));
    }

    /// The Open panel says the tool is alive in three ways, and only while it is.
    ///
    /// **The fault this holds shut is a page that cannot be told from a hang.** A migration is
    /// minutes inside one phase, so the sentence stands still; the seconds and the bar are what fill
    /// that window, and each covers the other's failure — a browser that has throttled the animation
    /// still counts, and somebody who has not read the sentence still sees the bar move.
    ///
    /// The finished arms carry neither, which is what stops a redirecting page ending on a bar that
    /// claims work is still going on.
    #[test]
    fn the_open_panel_keeps_moving_while_a_folder_is_opening() {
        let panel = |finished, error| {
            OpenProgress::new(
                Some(crate::server::OpeningView {
                    root: r"D:\tunes\karaoke".to_owned(),
                    elapsed_secs: 137,
                    phase: crate::db::OpeningPhase::UpToDate,
                    finished,
                    error,
                    ..Default::default()
                }),
                false,
                km_locale::Locale::English,
            )
            .in_english()
            .expect("render")
        };

        let running = panel(false, None);
        assert!(
            running.contains(r#"class="bar working""#),
            "nothing on the page moves, so a three-minute migration reads as a hang: {running}"
        );
        assert!(
            running.contains("137s"),
            "the panel has no count on it, so a browser that will not run the animation has nothing \
             left that changes: {running}"
        );
        assert!(
            running.contains("bringing the database up to date"),
            "the phase is not on the panel: {running}"
        );

        for (what, ended) in [
            ("the opened arm", panel(true, None)),
            (
                "the failed arm",
                panel(true, Some("no database there".to_owned())),
            ),
        ] {
            assert!(
                !ended.contains("bar working") && !ended.contains("137s"),
                "{what} goes on claiming work is running: {ended}"
            );
        }

        // And the stylesheet holds up its half, both ways. The resting opacity is the half that is
        // easy to lose: without it a tab the browser has stopped painting shows a bar left wherever
        // the keyframe stopped, which on a page whose only job is to say *still working* is the
        // worst state it can be in. The reduced-motion block is asserted for the opposite reason —
        // the usual way to honor that setting is `animation: none` on everything, and here the bar
        // itself has to stay.
        let css = include_str!("../static/style.css");
        assert!(
            css.contains(".bar.working > span { width: 100%; opacity: 0.8;"),
            "the indeterminate bar has no resting opacity, so a throttled tab shows it faded out"
        );
        assert!(
            css.contains("@media (prefers-reduced-motion: reduce) {\n    .bar.working > span { animation: none; }"),
            "reduced motion is not honored, or it takes the bar away with the movement"
        );
        // And the one that is invisible until it is missing. `.message` keeps its newlines; this
        // panel is the only one holding blocks, so without the override every line break in the
        // template between the bar and the sentence is drawn as a blank line.
        assert!(
            css.contains("white-space: normal;"),
            "the panel keeps `.message`'s newlines, so the template's own indentation is drawn"
        );
    }

    /// The Open panel lists every rung, and a rung it climbed past is one it did not need.
    ///
    /// **This is what the sentence and the seconds cannot say.** They name the step in hand, and a
    /// corpus that spends four minutes building indexes says nothing at all about the rungs still to
    /// come — while a rung the open had nothing to do on would sit waiting for the length of the
    /// job.
    #[test]
    fn the_open_panel_lists_its_rungs_and_marks_the_ones_it_climbed_past() {
        let job = crate::server::Opening::new(std::path::Path::new(r"D:\tunes\karaoke"));
        job.set_phase(crate::db::OpeningPhase::Folding { done: 3, total: 4 });

        let panel = OpenProgress::new(Some(job.snapshot()), false, km_locale::Locale::English)
            .in_english()
            .expect("render");

        let rung = |name: &str| rung_state(&panel, name);
        assert_eq!(
            rung("folding titles for the browse order"),
            "running",
            "the rung in hand is not the one marked: {panel}"
        );
        assert_eq!(
            rung("building the indexes"),
            "skipped",
            "a rung the open climbed past is left waiting for ever: {panel}"
        );
        assert_eq!(
            rung("gathering statistics over the whole corpus"),
            "waiting",
            "a rung still to come is not offered as still to come: {panel}"
        );
        assert!(
            panel.contains("not needed this time"),
            "a skipped rung does not say why it carries a dash: {panel}"
        );
    }

    /// The bar draws a proportion where the rung counts, and says only *working* where it does not.
    ///
    /// **A partial bar beside a rung with nothing inside it to count would be a proportion nobody
    /// stated.** Nine of the eleven are one statement each; the two that go in chunks are the two
    /// that take the minutes, and those are exactly the ones worth a real figure.
    #[test]
    fn the_bar_is_a_proportion_only_where_the_rung_counts_what_it_does() {
        let panel = |phase| {
            let job = crate::server::Opening::new(std::path::Path::new(r"D:\tunes\karaoke"));
            job.set_phase(phase);
            OpenProgress::new(Some(job.snapshot()), false, km_locale::Locale::English)
                .in_english()
                .expect("render")
        };

        let counted = panel(crate::db::OpeningPhase::ReadingWords { done: 3, total: 4 });
        assert!(
            counted.contains(r#"style="width: 75%""#) && !counted.contains("bar working"),
            "a rung that knows how far through it is draws no proportion: {counted}"
        );
        assert!(
            counted.contains("3 of 4 · 75%"),
            "the counts are nowhere beside the rung they belong to: {counted}"
        );

        let uncounted = panel(crate::db::OpeningPhase::GatheringStatistics);
        assert!(
            uncounted.contains("bar working") && !uncounted.contains("style=\"width"),
            "a single `ANALYZE` is drawn as a proportion of something: {uncounted}"
        );
    }

    /// An open that failed keeps its list, and the list is what says which rung broke.
    ///
    /// A reason alone names the fault. Which rung it happened in is the difference between a
    /// database that would not open and one whose index was rewritten halfway.
    ///
    /// **And a dash means two things here**, which is why both are asserted: above the failure it is
    /// a rung the open climbed past and did not need, below it a rung it was never going to reach.
    #[test]
    fn a_failed_open_says_which_rung_it_failed_in() {
        let job = crate::server::Opening::new(std::path::Path::new(r"D:\tunes\karaoke"));
        job.set_phase(crate::db::OpeningPhase::Indexing { missing: 6 });
        job.finish(Some("the disk went away".to_owned()));

        let panel = OpenProgress::new(Some(job.snapshot()), false, km_locale::Locale::English)
            .in_english()
            .expect("render");

        assert!(panel.contains("the disk went away"), "{panel}");
        assert!(
            panel.contains(r#"class="step failed""#)
                && rung_state(&panel, "building the indexes") == "failed",
            "the rung it failed in is not marked: {panel}"
        );
        let said = |name: &str| {
            let at = panel.find(name).expect(name);
            let end = panel[at..].find("</li>").expect("its row's end");
            panel[at..at + end].to_owned()
        };
        assert!(
            said("bringing the database up to date").contains("not needed this time"),
            "a rung the open climbed past before the failure reads as one it never reached: {panel}"
        );
        assert!(
            said("gathering statistics over the whole corpus").contains("not reached"),
            "a rung after the failure reads as one there was nothing to do: {panel}"
        );
        assert!(
            panel.contains(r#"removeAttribute("hidden")"#),
            "the list came at the cost of the chooser coming back: {panel}"
        );
    }

    /// A refusal made before any rung was climbed draws no checklist.
    ///
    /// **Eleven rungs reading *not needed this time* would say the open considered each one and
    /// declined it.** `report_failed_open` leaves a folder that is not there, holds no database or
    /// holds two as a job that finished without starting, and the honest page for that is the
    /// reason by itself.
    #[test]
    fn a_refusal_that_never_started_shows_no_rungs() {
        let job = crate::server::Opening::new(std::path::Path::new(r"D:\tunes\karaoke"));
        job.finish(Some("it holds two databases".to_owned()));

        let panel = OpenProgress::new(Some(job.snapshot()), false, km_locale::Locale::English)
            .in_english()
            .expect("render");

        assert!(panel.contains("it holds two databases"), "{panel}");
        assert!(
            !panel.contains("<ol class=\"steps\""),
            "a job that climbed nothing lists rungs it never considered: {panel}"
        );
    }

    /// The class on the row a step's name sits in.
    #[cfg(test)]
    fn rung_state(panel: &str, name: &str) -> String {
        let at = panel
            .find(name)
            .unwrap_or_else(|| panic!("{name} is not on the panel: {panel}"));
        let opened = panel[..at].rfind(r#"<li class="step "#).expect("its row");
        panel[opened + r#"<li class="step "#.len()..]
            .split('"')
            .next()
            .expect("its state")
            .to_owned()
    }
}
