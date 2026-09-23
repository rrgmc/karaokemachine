//! The pages, as askama templates, and the values each one is drawn from.
//!
//! **Every sentence that carries a number or a name is composed here**, through `msg_with`, and
//! arrives in a template as a finished string. A template names only keys that take no variables.
//! That is the rule `km_locale::filters` states for every page in this repository.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use km_locale::{Catalog, Locale, filters};

use crate::app::{Built, Inner, Phase};
use crate::session::{PackageForm, UNDETERMINED};

/// How many song rows one page of the list holds.
///
/// A folder can hold thousands of songs, and a table of thousands of inputs is a page a browser
/// takes seconds to draw. Two hundred is a page somebody reads down in one go.
pub const PAGE_ROWS: usize = 200;

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

/// The first page: which folder to read.
#[derive(Template)]
#[template(path = "home.html")]
pub struct HomePage {
    /// The page's frame.
    pub chrome: Chrome,
    /// The folder read last, offered again.
    pub last_folder: String,
    /// What went wrong last, if anything.
    pub error: Option<String>,
}

/// A job's progress, polled once a second.
#[derive(Template)]
#[template(path = "_progress.html")]
pub struct ProgressFragment {
    /// What is happening, as a sentence.
    pub said: String,
    /// How far through, out of 100, when it is known.
    pub percent: Option<usize>,
}

/// A page with a job running on it.
#[derive(Template)]
#[template(path = "working.html")]
pub struct WorkingPage {
    /// The page's frame.
    pub chrome: Chrome,
    /// The progress, drawn in place and then polled.
    pub progress: ProgressFragment,
}

/// The song list and the package form.
#[derive(Template)]
#[template(path = "songs.html")]
pub struct SongsPage {
    /// The page's frame.
    pub chrome: Chrome,
    /// The folder the songs came from.
    pub folder: String,
    /// What went wrong last, if anything.
    pub error: Option<String>,
    /// The package form.
    pub form: FormView,
    /// The list, which a toggle redraws on its own.
    pub list: SongList,
}

/// The package form's values and choices.
pub struct FormView {
    /// The name box.
    pub name: String,
    /// The version box.
    pub version: String,
    /// The publisher box.
    pub publisher: String,
    /// The folder the packages go to.
    pub out_dir: String,
    /// The language picker, the chosen one marked.
    pub languages: Vec<LanguageOption>,
}

/// One entry in the language picker.
pub struct LanguageOption {
    /// The code the form posts.
    pub code: &'static str,
    /// The language's English name, which is data. See `crate::words`.
    pub name: &'static str,
    /// Whether it is the chosen one.
    pub chosen: bool,
}

/// The list section: counts, rows, pager, and the files that are not songs.
#[derive(Template)]
#[template(path = "_songs.html")]
pub struct SongList {
    /// `412 of 420 songs go in, as 1 package.`
    pub summary: String,
    /// This page's rows.
    pub rows: Vec<RowView>,
    /// Which page this is, from 0.
    pub page: usize,
    /// `Songs 201 to 400 of 420`.
    pub range: String,
    /// The previous page, if there is one.
    pub previous: Option<usize>,
    /// The next page, if there is one.
    pub next: Option<usize>,
    /// `3 files are not songs`, or empty when there are none.
    pub left_heading: String,
    /// Each file that is not a song, and why.
    pub left: Vec<LeftView>,
}

/// One song row.
pub struct RowView {
    /// Its index in the whole list, which the forms post.
    pub index: usize,
    /// Where it lands: `17`, `2 · 17` in a set of volumes, or empty when it is left out.
    pub slot: String,
    /// Its kind, worded.
    pub kind: String,
    /// Its path under the folder.
    pub file: String,
    /// The title box's value.
    pub title: String,
    /// The artist box's value.
    pub artist: String,
    /// What a blank title box falls back to.
    pub stem: String,
    /// Its language code, or a dash.
    pub language: String,
    /// Its suitability out of 10, or a dash.
    pub suitability: String,
    /// Whether it goes in.
    pub kept: bool,
}

/// One file that is not a song.
pub struct LeftView {
    /// Its path under the folder.
    pub file: String,
    /// Why it is not a song, worded.
    pub why: String,
}

/// What a build wrote.
#[derive(Template)]
#[template(path = "built.html")]
pub struct BuiltPage {
    /// The page's frame.
    pub chrome: Chrome,
    /// `Wrote 2 packages, marked uncurated.`
    pub heading: String,
    /// Each file written.
    pub files: Vec<FileView>,
    /// The folder they are in, which Show in folder opens.
    pub out_dir: String,
    /// `2 songs did not go in`, or empty.
    pub skipped_heading: String,
    /// Each song that did not go in, in `km-pack`'s words.
    pub skipped: Vec<String>,
}

/// One package file written.
pub struct FileView {
    /// Its file name.
    pub name: String,
    /// `412 songs`.
    pub songs: String,
}

/// The page the state calls for.
pub fn page(inner: &Inner, locale: Locale, windowed: bool, page: usize) -> Response {
    let words = crate::words::messages(locale);
    let chrome = Chrome::new(locale, windowed);
    match &inner.phase {
        Phase::Empty => render(
            &HomePage {
                chrome,
                last_folder: inner
                    .settings
                    .last_folder
                    .as_ref()
                    .map(|folder| folder.display().to_string())
                    .unwrap_or_default(),
                error: inner.error.clone(),
            },
            locale,
        ),
        Phase::Reading { .. } | Phase::Building { .. } => render(
            &WorkingPage {
                chrome,
                progress: progress(&inner.phase, words),
            },
            locale,
        ),
        Phase::Ready => {
            let (Some(session), Some(form)) = (&inner.session, &inner.form) else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            render(
                &SongsPage {
                    chrome,
                    folder: session.folder.display().to_string(),
                    error: inner.error.clone(),
                    form: form_view(form),
                    list: song_list(inner, words, page),
                },
                locale,
            )
        }
        Phase::Built(built) => render(&built_page(chrome, inner, built, words), locale),
    }
}

/// The progress fragment for a running job.
#[must_use]
pub fn progress(phase: &Phase, words: &Catalog) -> ProgressFragment {
    let percent = |done: usize, total: usize| (total > 0).then(|| done * 100 / total);
    match phase {
        Phase::Reading { done, total, .. } => ProgressFragment {
            said: words
                .msg_with(
                    "progress-reading",
                    &[("done", (*done).into()), ("total", (*total).into())],
                )
                .into_owned(),
            percent: percent(*done, *total),
        },
        Phase::Building {
            volume,
            volumes,
            done,
            total,
        } => ProgressFragment {
            said: words
                .msg_with(
                    "progress-building",
                    &[
                        ("volume", (*volume).into()),
                        ("volumes", (*volumes).into()),
                        ("done", (*done).into()),
                        ("total", (*total).into()),
                    ],
                )
                .into_owned(),
            percent: percent(*done, *total),
        },
        _ => ProgressFragment {
            said: String::new(),
            percent: None,
        },
    }
}

fn form_view(form: &PackageForm) -> FormView {
    // `und` first, because it is what the form offers and what an unreviewed folder most often is.
    let mut all = km_kmpkg::Language::by_name();
    if let Some(at) = all
        .iter()
        .position(|language| language.code() == UNDETERMINED)
    {
        let undetermined = all.remove(at);
        all.insert(0, undetermined);
    }
    let languages = all
        .into_iter()
        .map(|language| LanguageOption {
            code: language.code(),
            name: language.name(),
            chosen: form.language == language.code(),
        })
        .collect();
    FormView {
        name: form.name.clone(),
        version: form.version.clone(),
        publisher: form.publisher.clone().unwrap_or_default(),
        out_dir: form.out_dir.display().to_string(),
        languages,
    }
}

/// The list section for one page of rows.
#[must_use]
pub fn song_list(inner: &Inner, words: &Catalog, page: usize) -> SongList {
    let Some(session) = &inner.session else {
        return SongList {
            summary: String::new(),
            rows: Vec::new(),
            page: 0,
            range: String::new(),
            previous: None,
            next: None,
            left_heading: String::new(),
            left: Vec::new(),
        };
    };
    let total = session.rows.len();
    let pages = total.div_ceil(PAGE_ROWS).max(1);
    let page = page.min(pages - 1);
    let start = page * PAGE_ROWS;
    let end = (start + PAGE_ROWS).min(total);
    let volumes = session.volumes();
    let slots = session.slots();

    let rows = (start..end)
        .map(|index| {
            let row = &session.rows[index];
            RowView {
                index,
                slot: match slots[index] {
                    Some(slot) if volumes > 1 => format!("{} · {}", slot.volume, slot.number),
                    Some(slot) => slot.number.to_string(),
                    None => String::new(),
                },
                kind: words.msg(row.kind.key()).into_owned(),
                file: row.song.file.clone(),
                title: row.song.title.clone().unwrap_or_default(),
                artist: row.song.artist.clone().unwrap_or_default(),
                stem: row.stem(),
                language: row.song.language.clone().unwrap_or_else(|| "—".to_owned()),
                suitability: row
                    .suitability
                    .map_or_else(|| "—".to_owned(), |value| value.to_string()),
                kept: row.kept,
            }
        })
        .collect();

    SongList {
        summary: words
            .msg_with(
                "songs-summary",
                &[
                    ("kept", session.kept().into()),
                    ("count", total.into()),
                    ("volumes", volumes.into()),
                ],
            )
            .into_owned(),
        rows,
        page,
        range: words
            .msg_with(
                "songs-range",
                &[
                    ("first", (start + 1).min(total).into()),
                    ("last", end.into()),
                    ("count", total.into()),
                ],
            )
            .into_owned(),
        previous: page.checked_sub(1),
        next: (page + 1 < pages).then_some(page + 1),
        left_heading: if session.left.is_empty() {
            String::new()
        } else {
            words
                .msg_with("left-heading", &[("count", session.left.len().into())])
                .into_owned()
        },
        left: session
            .left
            .iter()
            .map(|left| LeftView {
                file: left.file.clone(),
                why: match left.why.detail() {
                    Some(detail) => words
                        .msg_with(left.why.key(), &[("detail", detail.into())])
                        .into_owned(),
                    None => words.msg(left.why.key()).into_owned(),
                },
            })
            .collect(),
    }
}

fn built_page(chrome: Chrome, inner: &Inner, built: &Built, words: &Catalog) -> BuiltPage {
    BuiltPage {
        chrome,
        heading: words
            .msg_with("built-heading", &[("count", built.files.len().into())])
            .into_owned(),
        files: built
            .files
            .iter()
            .map(|(path, songs)| FileView {
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                songs: words
                    .msg_with("count-songs", &[("count", (*songs).into())])
                    .into_owned(),
            })
            .collect(),
        out_dir: inner
            .form
            .as_ref()
            .map(|form| form.out_dir.display().to_string())
            .unwrap_or_default(),
        skipped_heading: if built.skipped.is_empty() {
            String::new()
        } else {
            words
                .msg_with("built-skipped", &[("count", built.skipped.len().into())])
                .into_owned()
        },
        skipped: built
            .skipped
            .iter()
            .map(|skipped| format!("{}: {}", skipped.source.display(), skipped.why))
            .collect(),
    }
}
