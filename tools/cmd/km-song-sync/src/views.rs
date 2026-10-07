//! The pages, as askama templates, and the values each one is drawn from.
//!
//! **Every sentence that carries a number or a name is composed here**, through `msg_with`, and
//! arrives in a template as a finished string. A template names only keys that take no variables.
//! That is the rule `km_locale::filters` states for every page in this repository.

use std::path::Path;

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use km_locale::{Catalog, Locale, filters};

use crate::app::{Editor, Outcome};
use crate::rows::{Holds, Row};

/// Renders a template in one language.
///
/// **The values store is a local and the `String` is what leaves**, for the reason
/// `km-package-builder`'s `views::render` gives: the store is not `Send`, and a handler holding one
/// across an `.await` stops compiling with an error that names neither.
pub fn render<T: Template>(template: &T, locale: Locale) -> Response {
    let values = filters::values(crate::words::messages(locale));
    match template.render_with_values(&values) {
        Ok(body) => Html(body).into_response(),
        Err(error) => {
            tracing::error!(%error, "a page would not render");
            (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    }
}

/// What every full page carries.
pub struct Chrome {
    /// The page's language, as `<html lang>` takes it.
    pub lang: &'static str,
    /// Every language the pages speak, for the picker.
    pub locales: Vec<LocaleOption>,
    /// Whether the page is inside this tool's own window, where closing it is the quit.
    pub windowed: bool,
}

/// One entry in the language picker.
pub struct LocaleOption {
    /// The tag the picker posts.
    pub tag: &'static str,
    /// The language's own name for itself.
    pub name: &'static str,
    /// Whether it is the one in use.
    pub current: bool,
}

impl Chrome {
    /// The chrome for a page in `locale`.
    #[must_use]
    pub fn new(locale: Locale, windowed: bool) -> Self {
        Self {
            lang: locale.tag(),
            locales: Locale::ALL
                .iter()
                .map(|candidate| LocaleOption {
                    tag: candidate.tag(),
                    name: candidate.endonym(),
                    current: *candidate == locale,
                })
                .collect(),
            windowed,
        }
    }
}

/// The one page: the words box, the editor's state and the file browser.
#[derive(Template)]
#[template(path = "home.html")]
pub struct HomePage {
    /// The page's frame.
    pub chrome: Chrome,
    /// Whether the machine was found. Without it nothing can be started.
    pub machine: bool,
    /// What the editor is doing.
    pub editor: EditorFragment,
    /// The file browser.
    pub browser: BrowserFragment,
}

/// What the editor is doing, polled while it is open.
#[derive(Template)]
#[template(path = "_editor.html")]
pub struct EditorFragment {
    /// Whether the editor is open, which keeps the poll going and every Start waiting.
    pub running: bool,
    /// What happened, as a sentence. Empty when nothing has been started.
    pub said: String,
    /// Why a start was refused or an editor failed, as a sentence.
    pub error: Option<String>,
    /// Whether there is a written file to show in its folder.
    pub saved: bool,
}

/// The editor's state as the page shows it, with one refusal on top where a start was refused.
#[must_use]
pub fn editor(state: &Editor, refused: Option<&str>, words: &Catalog) -> EditorFragment {
    let name = |path: &Path| -> String {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    let mut fragment = EditorFragment {
        running: false,
        said: String::new(),
        error: refused.map(|key| words.msg(key).into_owned()),
        saved: false,
    };
    match state {
        Editor::Idle => {}
        Editor::Running { song } => {
            fragment.running = true;
            fragment.said = words
                .msg_with("editor-running", &[("song", name(song).into())])
                .into_owned();
        }
        Editor::Ended(Outcome::Saved { out }) => {
            fragment.saved = true;
            fragment.said = words
                .msg_with("editor-saved", &[("file", name(out).into())])
                .into_owned();
        }
        Editor::Ended(Outcome::Nothing { song }) => {
            fragment.said = words
                .msg_with("editor-saved-nothing", &[("song", name(song).into())])
                .into_owned();
        }
        Editor::Ended(Outcome::Failed { song, said }) => {
            fragment.said = words
                .msg_with("editor-failed", &[("song", name(song).into())])
                .into_owned();
            // A refusal to start outranks the last run's fault: it is what was just pressed.
            fragment.error.get_or_insert_with(|| said.clone());
        }
    }
    fragment
}

/// The file browser: where it is, the way up, the filter box and one page of rows.
#[derive(Template)]
#[template(path = "_browser.html")]
pub struct BrowserFragment {
    /// Where the browser is. `None` is the list of drives.
    pub here: Option<String>,
    /// The folder above, if there is one.
    pub parent: Option<String>,
    /// Why the listing is short, when it is.
    pub error: Option<String>,
    /// The folders, the songs and their pager.
    pub rows: RowsFragment,
}

/// One page of folders and songs and the pager, swapped on its own so the filter box keeps what
/// is typed.
#[derive(Template)]
#[template(path = "_rows.html")]
pub struct RowsFragment {
    /// The query string that draws this same page again, for when the editor closes.
    pub again: String,
    /// The folders on this page.
    pub folders: Vec<km_folders::Folder>,
    /// The songs on this page.
    pub songs: Vec<SongView>,
    /// What is being narrowed by.
    pub filter: String,
    /// `1 to 100 of 130`, or empty when there is nothing.
    pub range: String,
    /// The query string for the page before, empty when this is the first.
    pub previous: String,
    /// The query string for the page after, empty when this is the last.
    pub next: String,
    /// Whether the machine was found. Without it every Start is switched off.
    pub machine: bool,
}

/// One song row.
pub struct SongView {
    /// The file's name.
    pub name: String,
    /// Its path, which the Start button posts.
    pub path: String,
    /// What the row says under the name, as finished sentences.
    pub notes: Vec<String>,
    /// Whether the synced copy exists, which puts a box on the row to say replace it.
    pub out_exists: bool,
    /// Whether the editor can open the file at all.
    pub readable: bool,
    /// Whether the row needs words pasted, having none of its own and none beside it.
    pub needs_words: bool,
}

/// A song row as the page shows it.
fn song(row: Row, words: &Catalog) -> SongView {
    let mut notes = Vec::new();
    match (&row.sidecar, row.holds) {
        (_, Holds::NotMidi) => notes.push(words.msg("row-not-midi").into_owned()),
        (Some(name), _) => notes.push(
            words
                .msg_with("row-uses-text-file", &[("file", name.clone().into())])
                .into_owned(),
        ),
        (None, Holds::Words) => notes.push(words.msg("row-own-words").into_owned()),
        (None, Holds::NoWords) => notes.push(words.msg("row-needs-words").into_owned()),
    }
    if row.out_exists {
        notes.push(
            words
                .msg_with("row-output-exists", &[("file", row.out.clone().into())])
                .into_owned(),
        );
    }
    SongView {
        name: row.name,
        path: row.path,
        notes,
        out_exists: row.out_exists,
        readable: row.holds != Holds::NotMidi,
        needs_words: row.holds == Holds::NoWords && row.sidecar.is_none(),
    }
}

/// The browser drawn from a listing and what was read about its songs.
#[must_use]
pub fn browser(
    listing: km_folders::Listing,
    rows: Vec<Row>,
    machine: bool,
    words: &Catalog,
) -> BrowserFragment {
    BrowserFragment {
        here: listing.here.clone(),
        parent: listing.parent.clone(),
        error: listing.error.clone(),
        rows: browser_rows(listing, rows, machine, words),
    }
}

/// The page of rows and its pager, drawn from a listing and what was read about its songs.
#[must_use]
pub fn browser_rows(
    listing: km_folders::Listing,
    rows: Vec<Row>,
    machine: bool,
    words: &Catalog,
) -> RowsFragment {
    let shown = listing.rows.len() + listing.files.len();
    let range = if listing.total == 0 {
        String::new()
    } else {
        words
            .msg_with(
                "browse-range",
                &[
                    ("first", (listing.offset + 1).into()),
                    ("last", (listing.offset + shown).into()),
                    ("count", listing.total.into()),
                ],
            )
            .into_owned()
    };
    let mut again: Vec<(&str, String)> = Vec::new();
    match &listing.here {
        Some(here) => again.push(("at", here.clone())),
        None => again.push(("drives", "1".to_owned())),
    }
    if !listing.filter.is_empty() {
        again.push(("filter", listing.filter.clone()));
    }
    if listing.offset > 0 {
        again.push(("offset", listing.offset.to_string()));
    }
    RowsFragment {
        again: serde_urlencoded::to_string(&again).unwrap_or_default(),
        folders: listing.rows,
        songs: rows.into_iter().map(|row| song(row, words)).collect(),
        filter: listing.filter,
        range,
        previous: listing.previous,
        next: listing.next,
        machine,
    }
}
