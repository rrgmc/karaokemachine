//! The template structs.
//!
//! One struct per rendered thing, page or fragment. The compile-time checking is why askama was
//! chosen over a runtime engine, and it matters more here than in a developer tool: a field renamed
//! and not updated in the markup is a build failure rather than a blank card discovered by somebody
//! holding a microphone.
//!
//! **Every fragment has a page that contains it, and both render through the same struct.** A row
//! swapped back after a star, a queue list arriving over the event stream, and the same list drawn
//! during a full page load all go through one template — so the two can never disagree, which is the
//! failure mode a hand-written "update this bit" path always eventually has.
//!
//! Nesting is by holding the child struct as a field and writing `{{ child|safe }}`: askama's derive
//! implements `Display`, so a template renders inside another with no glue. `|safe` is correct there
//! and nowhere else — the child's own escaping already ran.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use km_locale::Locale;
use km_songcode::SongCode;

use crate::Capabilities;
use crate::machine::{
    ArtistRow, Connection, FolderRow, LanguageRow, MachineStatus, RemoteError, TagRow,
};
use crate::model::{Mode, PlayerView, QueueRow, SongRow};
use crate::words;

/// **What makes `{{ "tab-songs"|t }}` compile.** askama resolves a custom filter against a module
/// called `filters` in the scope the template was derived in, which is this one — so this single
/// import is the whole of the wiring for every template in this crate.
use km_locale::filters;

/// Renders a template into an HTML response, turning a template failure into a 500 with the reason.
///
/// A template cannot normally fail — the markup was checked when this was built — so reaching the
/// error arm means something like a formatter panic, and saying so beats a blank page.
///
/// **The locale goes in here and nowhere else.** askama 0.16 carries a values store through
/// `render_with_values` and into every nested `{{ child|safe }}`, so putting it in at the one place
/// a response is built reaches every fragment underneath — which is why no template struct has a
/// locale field and no construction site had to change. See [`km_locale::filters`].
pub fn page<T: Template>(template: &T, locale: Locale) -> Response {
    match render(template, locale) {
        Ok(body) => Html(body).into_response(),
        Err(error) => template_error(&error),
    }
}

/// Renders one template in a locale.
///
/// The single point every response here goes through, so a fragment cannot be rendered without one
/// by accident — a page that forgot would draw `⟦tab-songs⟧`, which is legible but is nobody's idea
/// of a remote.
///
/// **`pub(crate)` because the fan-out renders too**, and for a while it did not come through here:
/// the pump called `Template::render` directly, so every fragment it pushed arrived in exactly the
/// brackets this comment describes — correct on the page as it loaded and overwritten a second
/// later. See `handlers::everywhere`.
pub(crate) fn render<T: Template>(template: &T, locale: Locale) -> askama::Result<String> {
    template.render_with_values(&km_locale::filters::values(crate::words::messages(locale)))
}

/// Renders a fragment and appends an out-of-band toast to it.
///
/// The toast rides *outside* whatever was swapped, in its own `hx-swap-oob` element, so a message
/// lands on the page wherever the action happened to target — a star, a badge, a queue list, or
/// nothing at all. Making it part of the swapped fragment would tie what is said to what changed,
/// and the two are not the same thing: the most important message a remote gives is the one for an
/// action that changed nothing.
pub fn with_toast<T: Template>(template: &T, toast: Toast, locale: Locale) -> Response {
    let body = match render(template, locale) {
        Ok(body) => body,
        Err(error) => return template_error(&error),
    };
    Html(format!("{body}{}", oob(toast, locale))).into_response()
}

/// A toast and nothing else, for an action whose result is not on screen.
pub fn toast_only(toast: Toast, locale: Locale) -> Response {
    Html(oob(toast, locale)).into_response()
}

/// A template, a toast, and one more fragment that swaps itself.
///
/// The three answers `/machine/rescan` and `/machine/use` have to give at once: the card in the
/// target htmx aimed at, the offer block which is somewhere else on the page, and the sentence
/// saying what happened. The offer carries its own `id` and `hx-swap-oob`, exactly as the toast
/// wrapper does — see [`MachineFound`].
pub fn with_toast_and_oob<T: Template, O: Template>(
    template: &T,
    oob_template: &O,
    toast: Toast,
    locale: Locale,
) -> Response {
    let body = match render(template, locale) {
        Ok(body) => body,
        Err(error) => return template_error(&error),
    };
    let extra = match render(oob_template, locale) {
        Ok(extra) => extra,
        Err(error) => return template_error(&error),
    };
    Html(format!("{body}{extra}{}", oob(toast, locale))).into_response()
}

/// Wraps a toast in the out-of-band element `#toasts` receives.
fn oob(toast: Toast, locale: Locale) -> String {
    let rendered = render(&toast, locale).unwrap_or_default();
    format!(r#"<div id="toasts" hx-swap-oob="afterbegin">{rendered}</div>"#)
}

fn template_error(error: &askama::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("template error: {error}"),
    )
        .into_response()
}

/// A page that could not be drawn at all.
///
/// Distinct from a toast on purpose. A toast says "that did not happen"; this says "this cannot be
/// shown". They are different sentences and they belong in different places. `{{ message }}` is
/// escaped, which is why this is a template rather than a `format!`.
#[derive(Template)]
#[template(path = "_failure.html")]
pub struct Failure {
    /// What went wrong, worded for a person.
    pub message: String,
}

/// Turns a failure into something a page can show.
///
/// Nothing here produces a status other than 200 for an ordinary refusal, and that is deliberate:
/// htmx does not swap on an error response, so a red toast returned as a 500 would leave the button
/// that produced it visually dead and the reason nowhere on screen.
///
/// **Nothing the machine wrote reaches the page.** Every arm renders from this crate's own catalog,
/// in the language the viewer asked for; the machine's sentence goes to the log, which is where
/// whoever is diagnosing wants it and where its being English costs nobody anything.
///
/// **Nor does anything `km-remote-core` wrote.** In-process is not translated: that crate has no
/// catalog and no viewer to ask, so a sentence it composes — `The karaoke machine is not
/// answering.` — lands inside otherwise Portuguese pages. [`RemoteError::Offline`] therefore
/// travels as one of [`crate::machine::codes`], and so do [`RemoteError::Refused`] and
/// `Connection::reason`.
///
/// [`RemoteError::Rejected`] is the one arm that still carries prose, and it is not rendered. A 400
/// is aimed at whatever built the request rather than at the person holding the phone, so it goes to
/// the log and the page says the generic failure — the same treatment [`RemoteError::Unavailable`]'s
/// message gets, for the same reason.
pub fn message_for(error: &RemoteError, locale: Locale) -> Toast {
    let words = crate::words::messages(locale);
    match error {
        RemoteError::NotAcknowledged => Toast::info(words.msg(words::ERROR_NOT_ACKNOWLEDGED)),
        RemoteError::Offline(code) => Toast::bad(words.msg(coded(code))),
        RemoteError::Refused(code) => Toast::warn(words.msg(coded(code))),
        RemoteError::QueueFull => Toast::warn(words.msg(words::ERROR_QUEUE_FULL)),
        RemoteError::Unavailable { code, message } => {
            let key = words::refusal_key(code);
            if key == words::ERROR_UNAVAILABLE && code != words::ERROR_UNAVAILABLE {
                // Worth a line: it means this build met a refusal a newer machine has a name for,
                // and the singer got the generic sentence. The machine's own words are here and
                // nowhere else.
                tracing::debug!(
                    target: crate::LOG_TARGET,
                    code,
                    message,
                    "a refusal code this build does not know"
                );
            }
            Toast::warn(words.msg_with(key, &[("kind", words::refusal_kind(code).into())]))
        }
        RemoteError::Unauthorized => Toast::bad(words.msg(words::ERROR_UNAUTHORIZED)),
        RemoteError::NotFound => Toast::bad(words.msg(words::ERROR_NOT_FOUND)),
        // The machine's own words about a request it would not take. They go to the log for the
        // same reason `Unavailable`'s do — they are English, aimed at whatever built the request,
        // and the person reading this page is neither.
        RemoteError::Rejected(why) => {
            tracing::debug!(target: crate::LOG_TARGET, why, "the machine refused a request");
            Toast::bad(words.msg(words::ERROR_FAILED))
        }
        RemoteError::Failed(_) => Toast::bad(words.msg(words::ERROR_FAILED)),
    }
}

/// The message id for one of this device's own codes, or the generic failure.
///
/// A toast has to say *something*, so a code this build has no message for falls back to the
/// sentence every refusal produced before any of them had a name — see [`crate::words::code_key`],
/// whose other caller draws nothing instead.
fn coded(code: &str) -> &'static str {
    words::code_key(code).unwrap_or(words::ERROR_FAILED)
}

/// A line of text that appears over the page and goes away.
#[derive(Template)]
#[template(path = "_toast.html")]
pub struct Toast {
    /// `""`, `good`, `warn` or `bad`.
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

    /// Something happened, with a caveat. Also what an unacknowledged command gets — see
    /// [`RemoteError::NotAcknowledged`].
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            level: "",
            text: text.into(),
        }
    }

    /// Something did not happen, and it is worth knowing.
    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            level: "toast-warn",
            text: text.into(),
        }
    }

    /// Something is wrong.
    pub fn bad(text: impl Into<String>) -> Self {
        Self {
            level: "toast-bad",
            text: text.into(),
        }
    }
}

/// The tab-bar connection dot.
///
/// Its own fragment, separate from [`Banner`], because they live in different parts of the document
/// and one event cannot be swapped into two places.
#[derive(Template)]
#[template(path = "_conn.html")]
pub struct Conn {
    /// Whether the machine answers.
    pub online: bool,
}

/// The strip across the top saying the machine cannot be reached.
///
/// Renders to an empty element when everything is fine — an element and not nothing, because it is a
/// swap target and something has to be there for the next event to replace.
#[derive(Template)]
#[template(path = "_banner.html")]
pub struct Banner {
    /// The connection, as the machine layer reports it.
    pub connection: Connection,
}

impl Banner {
    /// The message id for why the machine is not reachable, where there is one to give.
    ///
    /// `Connection::reason` is a code — see [`crate::machine::codes`]. A code this build has no
    /// message for draws nothing, because the line above it already says the machine is not
    /// reachable and a wrong reason is worse than none.
    pub fn reason_key(&self) -> Option<&'static str> {
        self.connection.reason.and_then(crate::words::code_key)
    }
}

/// The queue badge on the tab bar.
#[derive(Template)]
#[template(path = "_queuecount.html")]
pub struct QueueCount {
    /// How many are waiting.
    pub len: usize,
}

/// The bits every full page needs.
pub struct Chrome {
    /// Which tab is current: `browse`, `now`, `queue` or `setup`.
    pub tab: &'static str,
    /// What this mode offers.
    pub capabilities: Capabilities,
    /// The badge.
    pub queue_count: QueueCount,
    /// The dot.
    pub conn: Conn,
    /// The strip.
    pub banner: Banner,
    /// Whose phone this is, when they have said.
    pub singer: Option<String>,
    /// What to hang on the end of a `/static/` URL so the browser may cache it for ever.
    ///
    /// See [`crate::ASSET_VERSION`]. It is on the chrome rather than passed to `layout.html`
    /// separately because every full page already carries one, and the stamp is wanted in exactly
    /// the place the chrome is.
    pub assets: &'static str,
    /// What `<html lang>` says.
    ///
    /// **A field rather than a message id**, because it is data and not a sentence: a catalog entry
    /// for it would be a string a translator could get wrong in a way nothing on screen would show,
    /// and what it must equal is the tag of the locale the page is being rendered in.
    pub lang: &'static str,
    /// The languages this remote can be read in, for the picker on the Setup tab.
    pub locales: Vec<LocaleChoice>,
    /// Which build is serving this page, for the line at the foot of the Setup tab.
    ///
    /// **The version of the process, not of the catalog it is showing.** These pages are mounted by
    /// five hosts: in the offline app it is `km-remote`'s build, and on the machine it is the
    /// machine's. Both are true statements about the thing answering the request, which is what
    /// somebody comparing two surfaces needs — and both are the one repository version anyway, since
    /// every crate here takes `version.workspace = true`.
    ///
    /// **A field rather than a message id**, for [`Chrome::lang`]'s reason: it is data, and a catalog
    /// entry for it would be a number a translator could get wrong.
    pub version: &'static str,
}

/// One option in the language picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleChoice {
    /// The BCP 47 tag, which is what the form posts.
    pub tag: &'static str,
    /// What this language calls itself.
    ///
    /// **Not translated, and that is the point** — see [`Locale::endonym`]. A picker offering
    /// `Portuguese` names the language to somebody who by definition cannot read the label.
    pub name: &'static str,
    /// Whether this is the one in use.
    pub chosen: bool,
}

impl LocaleChoice {
    /// Every language, with the one in use marked.
    #[must_use]
    pub fn all(current: Locale) -> Vec<Self> {
        Locale::ALL
            .iter()
            .map(|locale| Self {
                tag: locale.tag(),
                name: locale.endonym(),
                chosen: *locale == current,
            })
            .collect()
    }
}

/// `GET /`
#[derive(Template)]
#[template(path = "browse.html")]
pub struct BrowsePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The whole browse block.
    pub browse: BrowseBlock,
}

/// The `#browse` block: the bar, and the list under it.
///
/// Swapped as a whole when the *mode* changes, because a mode change alters the toggle, the search
/// box's placeholder, which filters exist and the list all at once.
#[derive(Template)]
#[template(path = "_browse.html")]
pub struct BrowseBlock {
    /// What this mode offers.
    pub capabilities: Capabilities,
    /// Which list is showing.
    pub mode: Mode,
    /// What is typed in the box.
    pub query: String,
    /// The chosen language, if any.
    pub language: Option<String>,
    /// Every language, for the picker.
    pub languages: Vec<LanguageRow>,
    /// The tags chosen, sorted — drawn as chips under the filter row.
    pub tags: Vec<String>,
    /// The tags **not** chosen, for the add-select.
    ///
    /// Already filtered, so the template never has to ask whether an option is already on: a picker
    /// that offered a tag somebody has picked would let them add it twice, and the second add is a
    /// request that changes nothing.
    pub tag_choices: Vec<TagRow>,
    /// The chosen initial, if any.
    pub initial: Option<char>,
    /// The artist being browsed inside, if any.
    pub artist: Option<String>,
    /// The folder being browsed inside, if any.
    pub folder: Option<FolderRow>,
    /// Whether the extra row actions are revealed.
    ///
    /// Read by `browse.html` and by nothing else: it checks the `#extra-actions` box on arrival, and
    /// the stylesheet does the revealing from there. The rows themselves no longer branch on it —
    /// `↑ next` and `▶ now` are always rendered — because a preference about how a row *draws* has
    /// no business re-fetching the list. See `_song_actions.html`.
    pub extra: bool,
    /// Packages the machine refused, one sentence each.
    ///
    /// Empty for the offline remote, which mirrors a catalog and so cannot see a package that
    /// never entered one — see [`crate::machine::Machine::package_problems`].
    pub package_problems: Vec<String>,
    /// The list.
    pub list: ListBlock,
}

impl BrowseBlock {
    /// Whether this view is inside something, and so should offer a way back out.
    pub fn is_nested(&self) -> bool {
        self.artist.is_some() || self.folder.is_some()
    }

    /// The share link for the folder being browsed inside, where there is one.
    ///
    /// `None` outside a folder, and `None` inside an **artist** — the two nested views share one
    /// head and only one of them has anything to share. A screen never offers a control whose
    /// question its own path has not already answered, and *which folder?* is answered by being in
    /// one. The backup is the same rule read the other way and is therefore not here at all: it is
    /// the whole collection, so it lives on the Setup tab where nothing has narrowed it.
    pub fn share_href(&self) -> Option<String> {
        self.folder
            .as_ref()
            .map(|folder| share_path(folder.id))
            .filter(|_| self.capabilities.favorites)
    }

    /// What the thing being browsed inside is called.
    pub fn nested_name(&self) -> &str {
        if let Some(artist) = &self.artist {
            return artist;
        }
        match &self.folder {
            Some(folder) => &folder.name,
            None => "",
        }
    }

    /// Whether the A–Z picker is worth drawing here.
    ///
    /// **The songs list and the artists list, and nowhere else.** The favorites folders are short —
    /// an initial filter over twelve of them is furniture rather than a tool — and the artists list
    /// was assumed to be short for the same reason, which a real corpus disagrees with: twelve
    /// thousand songs are several thousand artists, and paging to `T` for Tom Jobim is the scroll the
    /// songs list already had a letter for.
    ///
    /// `artist.is_none()` is why this is four terms and not three: *inside* an artist the mode is
    /// still [`Mode::Artists`] but the rows are that artist's songs, which is a short list with a
    /// heading, and where the chosen initial travels as a hidden field instead. That was true before
    /// and is now load-bearing rather than incidental.
    pub fn shows_initials(&self) -> bool {
        self.capabilities.initial_filter
            && (self.mode == Mode::Songs || self.mode == Mode::Artists)
            && self.folder.is_none()
            && self.artist.is_none()
    }

    /// Whether the filter row has anything in it at all.
    ///
    /// Three controls, each of which can be absent for its own reason — a catalog in one language,
    /// a build with no initial filter, a list whose rows are not songs — so the row itself has to ask
    /// rather than being drawn around whichever one happened to be checked first. An empty row is
    /// 0.4rem of nothing between the search box and the results.
    pub fn shows_filters(&self) -> bool {
        !self.languages.is_empty()
            || self.shows_tags()
            || self.shows_initials()
            || self.shows_extras()
    }

    /// Whether the tag picker is worth drawing here.
    ///
    /// There has to be something left to add. A catalog nobody has tagged draws no picker at all —
    /// which is most catalogs, since nothing detects a tag — and one whose every tag is already
    /// chosen draws none either, because the only options left would be ones that change nothing.
    ///
    /// **This term has to be in [`Self::shows_filters`]**, or a catalog with tags and one language
    /// would draw no filter row and the picker would have nowhere to be.
    pub fn shows_tags(&self) -> bool {
        !self.tag_choices.is_empty()
    }

    /// This same view with one tag taken off, for that chip's ✕.
    ///
    /// **A swap and not a plain link**, unlike the package builder's chips, and the rule is this
    /// template's own: a control that only refilters the list swaps `#list`, and only a change of
    /// *which* list you are looking at is a full page load. Removing a tag changes the chip strip
    /// as well as the rows, so it asks for `#browse` exactly as the search box's ✕ does — and, for
    /// that button's reason, it names its own `fragment` rather than inheriting one.
    ///
    /// Where the remaining tags go, and why they are not on `hx-include`, is
    /// [`Self::href_with_tags`], which this and [`Self::tags_clear_href`] are two callers of.
    pub fn tag_remove_href(&self, tag: &str) -> String {
        let remaining: Vec<&str> = self
            .tags
            .iter()
            .map(String::as_str)
            .filter(|held| *held != tag)
            .collect();
        self.href_with_tags(&remaining)
    }

    /// This same view with the tags it already has, for the picker to append one to.
    ///
    /// **The picker used to ride the enclosing form, and that was the whole bug.** The form asks for
    /// `#list` — the rows and nothing else — which is right for the search box and the two
    /// `<select>`s that only narrow. Adding a tag is not one of those: it changes the chip strip as
    /// well as the rows, exactly as *removing* one does, and [`Self::tag_remove_href`] has always
    /// said so and asked for `#browse`. Only half of that reasoning had been applied.
    ///
    /// Three things were wrong at once and they all had this one cause. No chip appeared, because
    /// the strip was never re-rendered. The picker went on offering the tag it had just applied, and
    /// showing it as selected, because the browser held that value and nothing replaced it. And the
    /// hidden `tags` field still held the *old* set — so choosing a second tag sent the first one's
    /// absence, and replaced it rather than adding to it. That last one is what looked like the
    /// picker clearing everything.
    ///
    /// This is the URL *before* the new tag: htmx appends the `<select>`'s own `add_tag` to it, and
    /// `BrowseParams::tags` merges the two. The language and the initial are not in here and ride
    /// `hx-include` instead, for the reason [`Self::href_with_tags`] gives — they are live
    /// `<select>` values, and what is on screen beats what the last render said.
    pub fn tag_add_href(&self) -> String {
        let held: Vec<&str> = self.tags.iter().map(String::as_str).collect();
        self.href_with_tags(&held)
    }

    /// This same view with every tag taken off, for the strip's *clear all*.
    ///
    /// **Its own control, because there was no other way to find it.** The picker's first option is
    /// a placeholder and selecting it does nothing — `BrowseParams::tags` merges `add_tag` only when
    /// `Tag::parse` accepts it — and the search box's ✕ deliberately keeps the tags, since it clears
    /// a *search*. So with four tags on, the only way back was four presses, and the control that
    /// looked like it might do it did not.
    ///
    /// Drawn whenever any tag is on rather than only for two or more. With one tag it is the same
    /// press as that chip's ✕, which is a duplicate — and a strip whose clear-all appears only once
    /// a second tag is added teaches nobody it exists, because the state somebody wants it in is the
    /// one they got into by not noticing it.
    pub fn tags_clear_href(&self) -> String {
        self.href_with_tags(&[])
    }

    /// This same view narrowed by exactly these tags.
    ///
    /// The tags go in the URL rather than riding `hx-include`, because the hidden `tags` field still
    /// holds the old set at the moment a chip is pressed. The language and the initial do the
    /// opposite for the opposite reason: those are live `<select>` values.
    fn href_with_tags(&self, tags: &[&str]) -> String {
        let mut url = format!("/?mode={}", self.mode.as_str());
        if !self.query.trim().is_empty() {
            url.push_str(&format!("&q={}", crate::prefs::encode(&self.query)));
        }
        if !tags.is_empty() {
            url.push_str(&format!("&tags={}", crate::prefs::encode(&tags.join(","))));
        }
        if let Some(artist) = &self.artist {
            url.push_str(&format!("&artist={}", crate::prefs::encode(artist)));
        }
        if let Some(folder) = &self.folder {
            url.push_str(&format!("&folder={}", folder.id));
        }
        url.push_str("&fragment=browse");
        url
    }

    /// The tags chosen, comma-joined — what the hidden `tags` field carries.
    pub fn tags_value(&self) -> String {
        self.tags.join(",")
    }

    /// Whether the ⋯ toggle belongs here.
    ///
    /// It reveals controls that act on a *song* row, so it is drawn wherever the list is made of
    /// them — the songs list, inside an artist, inside a folder — and not over the artists or the
    /// folders, whose rows are links into another list and have no actions to reveal.
    pub fn shows_extras(&self) -> bool {
        self.mode == Mode::Songs || self.artist.is_some() || self.folder.is_some()
    }

    /// Every mode this build offers, in tab order.
    pub fn modes(&self) -> Vec<Mode> {
        let mut modes = vec![Mode::Songs, Mode::Artists];
        if self.capabilities.favorites {
            modes.push(Mode::Favorites);
        }
        modes
    }

    /// The A–Z picker's options: `#` first, because a corpus starts with numbered songs.
    pub fn initials(&self) -> Vec<char> {
        std::iter::once('#').chain('A'..='Z').collect()
    }

    /// What one option says.
    ///
    /// Only `#` differs from itself. The **value** stays `#` — it is what `BrowseParams::initial`
    /// parses, what `prefs::browse_state` writes into the `km_browse` cookie and what every existing
    /// bookmark carries — while the label says the thing a reader can act on. `#` beside twenty-six
    /// initials in a list is a hash character; `0–9` is a bucket.
    pub fn initial_label(&self, initial: &char) -> String {
        match initial {
            '#' => "0\u{2013}9".to_owned(),
            other => other.to_string(),
        }
    }

    /// The link that switches to another mode, keeping what is typed in the box.
    ///
    /// Keeping the query and dropping everything else is the whole rule, and it is a judgment about
    /// what a mode change means: the words somebody has typed are what they are looking for, and the
    /// artist or folder they had opened belongs to the list they are leaving.
    pub fn mode_href(&self, mode: &Mode) -> String {
        let mut url = format!("/?mode={}", mode.as_str());
        if !self.query.trim().is_empty() {
            url.push_str(&format!("&q={}", crate::prefs::encode(&self.query)));
        }
        if let Some(language) = &self.language {
            url.push_str(&format!("&language={}", crate::prefs::encode(language)));
        }
        if !self.tags.is_empty() {
            url.push_str(&format!(
                "&tags={}",
                crate::prefs::encode(&self.tags.join(","))
            ));
        }
        url
    }

    /// Whether a mode is the one showing.
    pub fn is_mode(&self, mode: &Mode) -> bool {
        *mode == self.mode
    }

    /// This same view with the search box emptied, for the ✕ beside it.
    ///
    /// Everything except `q`, and deliberately **not** the language or the initial — those are live
    /// `<select>` values that ride along through `hx-include`, and taking them from here instead
    /// would send whatever the last render said rather than what is on screen now.
    ///
    /// **"Live `<select>` values" is true of the language and was only half true of the initial**,
    /// which is what made that reasoning quietly wrong for two of the four views. Where the picker is
    /// not drawn — inside an artist, inside a folder — the initial travels as a hidden field instead,
    /// and `hx-include="#language, #initial"` matched nothing there because that field had no id. So
    /// clearing a search inside an artist dropped an initial chosen in the songs list, silently and
    /// with wrong rows to show for it. The field carries `id="initial"` now; the two are mutually
    /// exclusive, so the selector still names exactly one element. **Adding the initial here instead
    /// would have been the wrong repair** — where the `<select>` *is* drawn, `hx-include` would send
    /// it too, and a duplicate `initial` is the same 400 the `fragment` collision produced.
    ///
    /// It asks for the whole `browse` fragment rather than the list, because the box's value is
    /// server-rendered: swapping only `#list` would empty the results and leave the words sitting in
    /// the field that no longer describes them.
    ///
    /// **That trailing `&fragment=browse` is the reason the enclosing form may not carry an
    /// `hx-vals`**, and this is where the two halves meet: htmx inherits `hx-vals` down into this
    /// button and appends it to whatever the URL already says, so a form-level `fragment=list` made
    /// every press ask for the key twice and be refused with a 400 before it reached a handler. The
    /// claim above — that the parameter set is decided here rather than by whichever of htmx's
    /// sources wins a merge — was the right intention and was not true until the form stopped
    /// carrying one. See the comment above the `<form>` in `_browse.html`.
    pub fn clear_href(&self) -> String {
        let mut url = format!("/?mode={}", self.mode.as_str());
        if let Some(artist) = &self.artist {
            url.push_str(&format!("&artist={}", crate::prefs::encode(artist)));
        }
        if let Some(folder) = &self.folder {
            url.push_str(&format!("&folder={}", folder.id));
        }
        url.push_str("&fragment=browse");
        url
    }

    /// Whether an initial is the one currently chosen.
    pub fn initial_is(&self, initial: &char) -> bool {
        self.initial == Some(*initial)
    }

    /// Whether no initial is chosen, so the `All` option is the selected one.
    pub fn no_initial(&self) -> bool {
        self.initial.is_none()
    }

    /// The link back out of an artist or a folder.
    pub fn up_href(&self) -> String {
        self.mode_href(&self.mode)
    }

    /// The query string that names this exact view, for a link that has to come back to it.
    pub fn context(&self) -> String {
        let mut parts = vec![format!("mode={}", self.mode.as_str())];
        if !self.query.trim().is_empty() {
            parts.push(format!("q={}", crate::prefs::encode(&self.query)));
        }
        if let Some(language) = &self.language {
            parts.push(format!("language={}", crate::prefs::encode(language)));
        }
        if !self.tags.is_empty() {
            parts.push(format!(
                "tags={}",
                crate::prefs::encode(&self.tags.join(","))
            ));
        }
        if let Some(initial) = self.initial {
            parts.push(format!(
                "initial={}",
                crate::prefs::encode(&initial.to_string())
            ));
        }
        if let Some(artist) = &self.artist {
            parts.push(format!("artist={}", crate::prefs::encode(artist)));
        }
        if let Some(folder) = &self.folder {
            parts.push(format!("folder={}", folder.id));
        }
        parts.join("&")
    }
}

/// The `#list` block: the rows, and the count that goes with them.
#[derive(Template)]
#[template(path = "_list.html")]
pub struct ListBlock {
    /// What to say when there is nothing. Composed in Rust rather than branched in markup, because
    /// it is three list kinds times four reasons and that is a table, not a template.
    pub empty: String,
    /// How many match altogether, where that is cheap to know.
    ///
    /// The count sits *inside* this block rather than up in the filter bar, which is where the Go
    /// remote put it. That one had to be patched into place with an out-of-band swap on every
    /// search, since the bar is not what a search replaces; here the thing that changes and the
    /// thing that reports it are the same fragment, and no second swap exists to get wrong.
    pub total: Option<usize>,
    /// The rows.
    pub rows: RowsBlock,
    /// `112 shown` — or `50 of 112` where counting them all did not mean a second full scan.
    ///
    /// **A field rather than a method, because it is a sentence.** It was arithmetic in markup's
    /// clothing: two numbers and a word, composed where no catalog could reach it, so the count under
    /// a Portuguese list read `112 shown`. Composed in [`crate::handlers`] now, which is where this
    /// module's own rule has always said a thing that computes belongs.
    pub summary: String,
    /// What this folder holds and the machine in hand cannot show, a sentence to a line.
    ///
    /// **Here rather than in [`RowsBlock`]**, and the difference is what each swap does: `#list` is
    /// replaced whole on every search, while *Load more* appends another `_rows.html` inside it. A
    /// note in the rows would therefore be drawn again under every page somebody loaded.
    ///
    /// Empty in every mode but an unfiltered folder, and empty there whenever the catalog can place
    /// the lot.
    pub not_here: Vec<String>,
}

/// One artist's line: the row, and how many songs it holds said as a sentence.
///
/// Beside [`SongLine`] and for the same reason — the row is data out of a catalog, and the count
/// under it is a plural, which is arithmetic and belongs in Rust. The markup used to write
/// `{{ artist.songs }} song{% if artist.songs != 1 %}s{% endif %}`, an English plural spelled out in
/// a template, which is exactly what the catalog rule forbids.
pub struct ArtistLine {
    /// The artist.
    pub row: ArtistRow,
    /// `12 songs`.
    pub count: String,
}

/// One folder's line. See [`ArtistLine`].
pub struct FolderLine {
    /// The folder.
    pub row: FolderRow,
    /// `12 songs`.
    pub count: String,
}

/// One song's line, with the two fragments that can be swapped independently of it.
///
/// The star and the action buttons are built here rather than written out again inside the row
/// markup, because both are also rendered on their own — the star when the picker closes, the
/// actions when a song is queued — and a second copy of either would be free to drift from the one
/// the page first drew.
pub struct SongLine {
    /// The song.
    pub row: SongRow,
    /// Its star, when this mode has favorites.
    pub star: Option<Star>,
    /// Its action buttons.
    pub actions: SongActions,
}

/// The rows themselves, and the button that asks for more of them.
///
/// One template for three shapes of row. `kind` says which, and only the matching vector is
/// populated — an enum in a template costs more legibility than it saves here, where the three
/// branches are three `<li>` shapes and nothing else differs.
#[derive(Template)]
#[template(path = "_rows.html")]
pub struct RowsBlock {
    /// `songs`, `artists` or `folders`.
    pub kind: &'static str,
    /// The songs, in songs, top and drill-down modes.
    pub songs: Vec<SongLine>,
    /// The artists, in artists mode.
    pub artists: Vec<ArtistLine>,
    /// The folders, in favorites mode.
    pub folders: Vec<FolderLine>,
    /// The query string that fetches the next page, when there is one.
    pub next: Option<String>,
    /// Whether the star is drawn.
    pub favorites: bool,
    /// Whether a row can be removed from what is being browsed — only inside a folder.
    pub in_folder: Option<i64>,
    /// The query string every row's links must carry to come back to this same view.
    pub context: String,
    /// Which list these rows are, so `live.js` can tell an anchor taken here from one taken
    /// elsewhere. Always rendered — every search re-renders `#rows`, so the stamp cannot go stale
    /// relative to the rows it is on.
    pub list_tag: String,
    /// The row to come back to, on the one request that is restoring a position.
    ///
    /// `Some` only on a bare full-page load, which is what tapping the Songs tab produces. `Load
    /// more` is never bare, so only the outermost `<ul id="rows">` can carry it — which matters,
    /// because that button swaps a whole `_rows.html` in and a page that has loaded four of them
    /// holds four elements with that id.
    pub anchor: Option<SongCode>,
}

impl RowsBlock {
    /// Whether this is a list of songs.
    pub fn is_songs(&self) -> bool {
        self.kind == "songs"
    }

    /// Whether this is a list of artists.
    pub fn is_artists(&self) -> bool {
        self.kind == "artists"
    }

    /// Whether this is a list of folders.
    pub fn is_folders(&self) -> bool {
        self.kind == "folders"
    }

    /// How many rows are showing.
    pub fn len(&self) -> usize {
        self.songs.len() + self.artists.len() + self.folders.len()
    }

    /// Whether there is nothing at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The per-row action buttons — the swap target an action adds its badge to.
#[derive(Template)]
#[template(path = "_song_actions.html")]
pub struct SongActions {
    /// Which song.
    pub number: SongCode,
    /// The folder to offer removal from, when inside one.
    pub in_folder: Option<i64>,
    /// The query string that brings a removal back to this view.
    pub context: String,
    /// The confirmation a removal asks for, naming the song.
    ///
    /// Composed rather than assembled in the attribute, for the reason this module already gives
    /// about `ListBlock::summary`: a title inside a sentence is a sentence, and one written in
    /// markup could only ever be English. It was `Remove “{{ title }}” from this folder?`.
    pub confirm: String,
}

/// What an action leaves in front of the buttons for a moment.
#[derive(Template)]
#[template(path = "_badge.html")]
pub struct Badge {
    /// `badge`, `badge-good` or `badge-warn`.
    pub level: &'static str,
    /// One or two words: `queued`, `next up`, `sent`.
    pub text: &'static str,
}

/// The star, which opens the folder picker.
#[derive(Template)]
#[template(path = "_star.html")]
pub struct Star {
    /// Which song.
    pub number: SongCode,
    /// Whether it is filed anywhere.
    pub on: bool,
    /// Whether this is being swapped back into a page rather than rendered in place.
    pub oob: bool,
}

/// The folder picker.
#[derive(Template)]
#[template(path = "_sheet.html")]
pub struct Sheet {
    /// Which song is being filed.
    pub number: SongCode,
    /// What it is called.
    pub title: String,
    /// Its artist, for telling two songs of one name apart.
    pub artist: Option<String>,
    /// Every folder, with whether this song is in it.
    pub folders: Vec<(FolderRow, bool)>,
    /// Whether the sheet stays open after a tap.
    ///
    /// Rides on each request rather than being stored, so **every fresh opening starts in the
    /// one-tap way**. Filing a song in several folders is the rarer thing and asks for itself.
    pub multi: bool,
    /// What went wrong with the last thing tried in here, if anything.
    pub error: Option<String>,
}

/// `GET /now`
#[derive(Template)]
#[template(path = "now.html")]
pub struct NowPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The card, and the whole of the page.
    pub player: PlayerBlock,
}

/// `GET /setup`
///
/// The appliance and this device's preferences. Present in both modes, and the two of them do not
/// hold the same things: what is always here is the singer's name and the language, because those
/// are about whoever is holding the phone rather than about a machine.
#[derive(Template)]
#[template(path = "setup.html")]
pub struct SetupPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// Which machine this device is talking to. `None` in the online mode.
    pub machine: Option<MachineBlock>,
    /// Whether the Setup and Packages tabs are drawn at the top. Only when the catalog holds two or
    /// more packages: hiding the only package leaves nothing to search.
    pub packages_tab: bool,
}

/// The Setup tab's Packages page: which packages this phone's song list leaves out.
#[derive(Template)]
#[template(path = "setup_packages.html")]
pub struct PackagesPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// Every package the catalog holds.
    pub packages: Vec<PackageSetting>,
}

/// One package on the Setup tab, and whether this phone shows it.
pub struct PackageSetting {
    /// The package id, which is what the form posts.
    pub id: String,
    /// What its packager named it.
    pub name: String,
    /// `120 songs`, in the viewer's language.
    pub songs: String,
    /// Whether its songs are in this phone's song list.
    pub shown: bool,
    /// Whether it was built straight from a folder, which the row marks with a badge.
    ///
    /// **One field per flag the page draws**, so each label is a key the template names and the
    /// catalog parity tests can see.
    pub uncurated: bool,
}

/// The `#machine` block: which machine, how it was found, and what can be done about it.
///
/// Republished by the pump, so that a card left open follows the background watch switching machines
/// on its own. **The address input is not in here** — see the template, and `sse.rs`'s header.
#[derive(Template)]
#[template(path = "_machine.html")]
pub struct MachineBlock {
    /// Which machine, and how much of it is held here.
    ///
    /// **`status.connection.address` is also what the *Open in browser* link opens**, so the card
    /// needs nothing else for it: the address it prints and the address it hands to a browser are
    /// one field read twice, and cannot come apart.
    pub status: MachineStatus,
    /// `394 songs in this copy`, in the language this card is being built for.
    ///
    /// **The one field that makes this card built per language rather than only rendered per
    /// language.** It is a plural over a count, so it is composed in Rust through `msg_with` — the
    /// rule the catalog states about anything that computes. Every other pushed fragment is one
    /// struct rendered twice; see `handlers::everywhere`.
    pub songs_line: String,
}

impl MachineBlock {
    /// The card for one machine, worded for one reader.
    #[must_use]
    pub fn of(status: MachineStatus, locale: Locale) -> Self {
        let songs_line = crate::words::messages(locale)
            .msg_with(
                "machine-songs-copied",
                &[("count", (status.songs as i64).into())],
            )
            .into_owned();
        Self { status, songs_line }
    }

    /// The message id for how this machine's address was arrived at.
    ///
    /// **The card gets a key, not the sentence the record holds.** `how` arrives as a stable code
    /// from `km-remote-core`, exactly as a refusal does — see [`crate::words::how_key`]. It used to
    /// arrive as English prose and go straight onto the card, so a Portuguese reader was told their
    /// machine was `remembered`.
    #[must_use]
    pub fn how_key(&self) -> Option<&'static str> {
        self.status.how.as_deref().and_then(crate::words::how_key)
    }

    /// What to prefill the address box with.
    ///
    /// The address as it stands, or empty before anything has been found — never a placeholder
    /// dressed up as a value, which is what a box prefilled with `192.168.1.5` would be. The
    /// placeholder is the template's job.
    pub fn address_value(&self) -> &str {
        self.status.connection.address.as_deref().unwrap_or("")
    }
}

/// The `#machine-found` block: the machines a rescan turned up and did not move to.
///
/// **It lives outside `#machine` and is swapped out of band**, which is not decoration. The pump
/// republishes `#machine` every time the connection changes, so an offer rendered inside the card
/// would be wiped by the next tick — the same reason the address `<details>` is in `now.html` and
/// not in the card's own template.
///
/// Always rendered, empty and all: the element has to be in the document for htmx to have something
/// to swap, and an empty one is how the offers are taken away again.
///
/// **A list rather than one offer.** A browse in a house with three machines has three answers, and
/// picking between them is the person's — which was the point of offering rather than taking. The
/// old shape held one, so `Rescan` in a two-machine house said *the machine you are
/// already using is the one on the network* and dropped the other.
#[derive(Template)]
#[template(path = "_found.html")]
pub struct MachineFound {
    /// Every machine that answered and is not the one in hand.
    pub offers: Vec<crate::machine::Offer>,
}

/// The player card: everything on the Now page that is a control.
///
/// Republished over the event stream only when the song or the settings change. What moves
/// constantly is [`PositionBlock`], which is why nothing here needs `hx-preserve`.
#[derive(Template)]
#[template(path = "_player.html")]
pub struct PlayerBlock {
    /// The state, formatted.
    pub view: PlayerView,
    /// The position, which is nested inside the card on the page and replaced separately after.
    pub position: PositionBlock,
}

/// What is playing, at the top of the Queue page.
///
/// The player card's header and its transport row and **nothing else** — no progress bar, no
/// steppers, no volume. It is now the *only* place the four transport buttons are; see `The
/// transport is behind a gear, and only on the Queue tab` in `docs/decisions/remotes.md`.
///
/// Its own SSE event ([`crate::sse::NOWBAR`]) rather than a second target for `player`, on the same
/// reasoning that splits the queue list from the queue badge: one event carries one rendered
/// fragment, and this is not the card.
#[derive(Template)]
#[template(path = "_nowbar.html")]
pub struct NowBar {
    /// The state, formatted.
    pub view: PlayerView,
    /// The four transport buttons.
    pub transport: TransportBlock,
}

/// The four buttons that drive what is playing.
///
/// **One caller now, where there were two.** It carried a `target` and a `query` — "two halves of
/// one fact", which fragment a press is answered with — because the Now tab's card drew the same
/// four buttons and wanted them back as `#player`. The Now tab has no transport any more, so both
/// fields collapsed into the two constants in the template and there is nothing left for the two
/// callers to disagree about.
#[derive(Template)]
#[template(path = "_transport.html")]
pub struct TransportBlock {
    /// The state, formatted.
    pub view: PlayerView,
}

/// The elapsed time and the bar.
#[derive(Template)]
#[template(path = "_position.html")]
pub struct PositionBlock {
    /// `1:07`.
    pub elapsed: String,
    /// `3:42`.
    pub total: String,
    /// 0–100.
    pub percent: u32,
}

impl PlayerBlock {
    /// The whole card for one state.
    pub fn of(view: PlayerView) -> Self {
        let position = PositionBlock::of(&view);
        Self { view, position }
    }

    /// The card for a machine that has not answered.
    ///
    /// An idle card, not an error: the offline app spends most of its life here, and a page that
    /// refused to draw would make "the television is off" look like a fault in the remote. The
    /// banner is what says the machine is away, and it says it once.
    pub fn unreachable() -> Self {
        Self::of(PlayerView::unreachable())
    }
}

impl NowBar {
    /// The bar for one state.
    ///
    /// The transport it holds asks for `?fragment=nowbar`, which is what makes a press from the
    /// Queue tab come back as a bar rather than as the whole card. That is now the only answer there
    /// is — see [`TransportBlock`].
    pub fn of(view: PlayerView) -> Self {
        let transport = TransportBlock { view: view.clone() };
        Self { view, transport }
    }

    /// The bar for a machine that has not answered. See [`PlayerBlock::unreachable`].
    pub fn unreachable() -> Self {
        Self::of(PlayerView::unreachable())
    }
}

impl PositionBlock {
    /// Pulls the moving parts out of a whole state.
    pub fn of(view: &PlayerView) -> Self {
        Self {
            elapsed: view.elapsed(),
            total: view.total(),
            percent: view.percent(),
        }
    }
}

/// `GET /queue`
#[derive(Template)]
#[template(path = "queue.html")]
pub struct QueuePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// What is playing, above the list.
    pub nowbar: NowBar,
    /// The list.
    pub queue: QueueBlock,
}

/// The queue list.
#[derive(Template)]
#[template(path = "_queue.html")]
pub struct QueueBlock {
    /// Who is waiting.
    pub rows: Vec<QueueRow>,
    /// Whether the machine is reachable, which decides whether the buttons do anything.
    pub online: bool,
}

// -- sharing a folder, and carrying the collection to a file -------------------------------------
//
// Eight screens across two flows, and every one of them is a full page with the ordinary three-tab
// bar: these live under the Songs tab, because favorites is a browse *mode* rather than a place of
// its own. `layout.html` is untouched, so `en.ftl`'s "Three tabs and there is no fourth" holds by
// construction rather than by anybody remembering it.
//
// **Every count and every sentence naming a folder arrives here already composed.** That is
// `ListBlock::summary`'s rule applied twice over: arithmetic and interpolation happen in Rust where
// a test can reach them, and the markup carries a value. It matters more here than there, because a
// plural in Portuguese is not the same set of cases as a plural in English and `{ $count ->` is the
// only place that can be said once.

/// The header every share and backup screen wears.
///
/// **A way out to the folder list on all eight**, beside the step-back that only some of them have.
/// Both flows run several screens deep, and backing out one at a time is not what "done" feels like.
///
/// One template for both, where the sibling project keeps two near-identical copies: this reads
/// `head`, a field every screen in both flows carries, so there is no `.Sync`-or-`.Backup` to thread
/// through pages that have only one of them.
#[derive(Template)]
#[template(path = "_stephead.html")]
pub struct StepHead {
    /// Where `‹` goes, or `None` on the first screen of a flow.
    pub back: Option<String>,
    /// The heading, which names the folder where there is one.
    pub title: String,
}

/// `GET /favorites/share/{folder}` — which side of the exchange this phone is.
///
/// **The folder is not a question here and cannot be**, because sharing is reachable only from
/// inside one. That single constraint deletes both "which folder?" prompts: the folder you are in is
/// the answer for the phone showing a code and for the phone reading one.
#[derive(Template)]
#[template(path = "share.html")]
pub struct SharePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// The folder this is about, on this device.
    pub folder: FolderRow,
    /// `Read a code and add its songs to Rock`.
    pub receive_line: String,
    /// The paragraph saying a share goes one way and only ever adds.
    pub one_way_line: String,
}

impl SharePage {
    /// The share root for this folder, which every screen in the flow builds on.
    pub fn path(&self) -> String {
        share_path(self.folder.id)
    }
}

/// `GET /favorites/share/{folder}/send`
#[derive(Template)]
#[template(path = "share_send.html")]
pub struct ShareSendPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// The folder whose code this is.
    pub folder: FolderRow,
    /// The digits, for the *Can't scan it?* box. Empty where the folder could not be encoded.
    pub code: String,
    /// A folder whose code is valid and larger than one QR can hold.
    ///
    /// **Asked here as well as in the image handler**, because a page that links an image the
    /// handler will refuse shows a broken icon and no reason — where the text underneath works
    /// perfectly well, so the page opens that box instead of drawing an image.
    pub too_dense: bool,
    /// `12 songs`.
    pub count: String,
    /// `Code for Rock`, the image's alternative text.
    pub image_alt: String,
    /// Why there is no code at all, worded.
    pub error: Option<String>,
}

impl ShareSendPage {
    /// Where the image comes from.
    pub fn image_href(&self) -> String {
        format!("{}/code.svg", share_path(self.folder.id))
    }
}

/// `GET /favorites/share/{folder}/receive`
#[derive(Template)]
#[template(path = "share_receive.html")]
pub struct ShareReceivePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// The folder the songs will go into.
    pub folder: FolderRow,
    /// `Point this at the code shown on the other device. Its songs go into Rock.`
    pub target_line: String,
    /// What went wrong with the last code read or pasted.
    pub error: Option<String>,
}

impl ShareReceivePage {
    /// The share root, which both forms on this page post back to.
    pub fn path(&self) -> String {
        share_path(self.folder.id)
    }
}

/// `POST /favorites/share/{folder}/receive` — what was read, before anything is written.
#[derive(Template)]
#[template(path = "share_confirm.html")]
pub struct ShareConfirmPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// The folder the songs will go into.
    pub folder: FolderRow,
    /// The name the code carried, where it carried one. **Shown, and the code is not.**
    pub scanned_name: Option<String>,
    /// `142 songs`.
    pub scanned_count: String,
    /// That same code verbatim, carried to the merge in a hidden field.
    ///
    /// **Named apart from [`ShareSendPage::code`] on purpose.** The two never appear on one screen,
    /// and calling them the same thing invites exactly the mix-up that would send the wrong songs
    /// somewhere.
    pub raw: String,
    /// `This code came from Party, and you are adding it to Rock.` `None` when the names agree.
    ///
    /// Not an error — merging one folder into another is a fair thing to want — but said out loud,
    /// because names not matching is also what a mis-scan looks like.
    pub mismatch: Option<String>,
    /// `Add to Rock`.
    pub add_label: String,
}

impl ShareConfirmPage {
    /// The share root, which the merge form posts under.
    pub fn path(&self) -> String {
        share_path(self.folder.id)
    }
}

/// `POST /favorites/share/{folder}/merge`
#[derive(Template)]
#[template(path = "share_done.html")]
pub struct ShareDonePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// The folder the songs went into.
    pub folder: FolderRow,
    /// `18 songs added, 4 already here`.
    pub outcome: String,
    /// `3 songs are not in this device's song list, so they were left out.` `None` when none were.
    pub left_out: Option<String>,
    /// `Open Rock`.
    pub open_label: String,
}

impl ShareDonePage {
    /// Back to the folder that was just filled.
    pub fn folder_href(&self) -> String {
        format!("/?mode=favorites&folder={}", self.folder.id)
    }

    /// The return leg: show this device's code so the other one can read it.
    pub fn send_href(&self) -> String {
        format!("{}/send", share_path(self.folder.id))
    }
}

/// `GET /favorites/backup`
///
/// **Offered on the folder list, where sharing is offered only inside a folder.** The same rule
/// twice: a screen never asks a question its own path already answered. A backup is the whole
/// collection, so offering it inside one folder would invite the reading that it backs up that
/// folder.
#[derive(Template)]
#[template(path = "backup.html")]
pub struct BackupPage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// Whether there is anything to save at all.
    pub any: bool,
    /// `128 songs in 4 folders, as one file` — counted over what the file will hold, so an empty
    /// folder is in neither number.
    pub holds: String,
}

/// `GET /favorites/backup/restore`
#[derive(Template)]
#[template(path = "backup_restore.html")]
pub struct RestorePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// What was wrong with the last file offered.
    pub error: Option<String>,
}

/// `POST /favorites/backup/restore`
#[derive(Template)]
#[template(path = "backup_done.html")]
pub struct RestoreDonePage {
    /// Page chrome.
    pub chrome: Chrome,
    /// The header.
    pub head: StepHead,
    /// `142 songs added`.
    pub added: String,
    /// `168 read from 4 folders, 26 already here, 1 folder created`.
    pub detail: String,
    /// Songs this device's catalog cannot show, dropped before the write.
    pub left_out: Option<String>,
    /// Which of those were left out, and what would fix each — one line per remedy.
    ///
    /// Empty in the ordinary case. Beside [`Self::left_out`] rather than replacing it: that line
    /// says how many went missing in total, these say which ones can be explained.
    pub unplaced: Vec<String>,
    /// Lines in the file that were not song numbers at all.
    pub unreadable: Option<String>,
    /// `That file was written by a newer version, and this one read what it could.`
    ///
    /// `None` normally — a format number is reported, never enforced.
    pub format_note: Option<String>,
}

/// The share root for a folder.
///
/// One function, because five screens and three links build on it and a sixth spelling of
/// `/favorites/share/{id}` is a route that quietly stops matching.
fn share_path(folder: i64) -> String {
    format!("/favorites/share/{folder}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toast_carries_its_level_into_the_markup() {
        let html = Toast::good("Queued: Tempo Perdido")
            .render()
            .expect("render");
        assert!(html.contains("toast-good"), "{html}");
        assert!(html.contains("Queued: Tempo Perdido"), "{html}");
    }

    /// The one place user text and markup meet. A song whose title is `<script>` must read
    /// `<script>`, and askama's default escaping is what makes that so — this pins it.
    ///
    /// The assertion is on the angle brackets being gone rather than on any particular spelling of
    /// the escape: askama writes numeric entities (`&#60;`), a detail of its escaper and not
    /// something this crate should be pinning.
    #[test]
    fn text_out_of_a_corpus_cannot_become_markup() {
        let html = Toast::bad("<script>alert(1)</script>")
            .render()
            .expect("render");
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("</script>"), "{html}");
        assert!(html.contains("script"), "{html}");
    }

    #[test]
    fn an_unacknowledged_command_is_not_colored_as_a_failure() {
        let toast = message_for(&RemoteError::NotAcknowledged, km_locale::Locale::English);
        assert_eq!(toast.level, "");
        assert!(!RemoteError::NotAcknowledged.is_fault());
    }

    #[test]
    fn the_online_mode_offers_two_browse_modes_and_the_offline_one_three() {
        let block = |capabilities| BrowseBlock {
            capabilities,
            mode: Mode::Songs,
            query: String::new(),
            language: None,
            languages: Vec::new(),
            tags: Vec::new(),
            tag_choices: Vec::new(),
            initial: None,
            artist: None,
            folder: None,
            extra: false,
            package_problems: Vec::new(),
            list: ListBlock {
                total: None,
                empty: String::new(),
                summary: String::new(),
                not_here: Vec::new(),
                rows: RowsBlock {
                    kind: "songs",
                    songs: Vec::new(),
                    artists: Vec::new(),
                    folders: Vec::new(),
                    next: None,
                    favorites: false,
                    in_folder: None,
                    context: String::new(),
                    list_tag: String::new(),
                    anchor: None,
                },
            },
        };
        assert_eq!(block(Capabilities::online()).modes().len(), 2);
        assert_eq!(block(Capabilities::offline()).modes().len(), 3);

        // The one capability that runs the other way: the book is the machine's own route, so the
        // remote the machine serves can link to it and the one running on a phone cannot.
        let online = block(Capabilities::online()).render().expect("render");
        let offline = block(Capabilities::offline()).render().expect("render");
        assert!(online.contains("/api/v1/songs/book.pdf"), "{online}");
        assert!(
            !offline.contains("book.pdf"),
            "the offline remote has no machine to ask: {offline}"
        );
    }

    #[test]
    fn a_refusal_is_said_in_the_language_the_page_is_in() {
        let refusal = RemoteError::Unavailable {
            code: "no_key_video".to_owned(),
            message: "a video song has no key to change".to_owned(),
        };
        assert_eq!(
            message_for(&refusal, Locale::English).text,
            "A video song has no key to change."
        );
        // The article agrees with the noun, which is why the kind is an argument and not part of a
        // sentence the machine composed.
        assert_eq!(
            message_for(&refusal, Locale::BrazilianPortuguese).text,
            "Uma música em vídeo não tem tom para mudar."
        );
    }

    #[test]
    fn the_machines_own_words_never_reach_the_page() {
        // The whole point of the code. A machine composes in English because it has no idea who is
        // reading; putting its sentence on the page is how English ends up inside a Portuguese one.
        for (code, message) in [
            ("no_key_video", "a video song has no key to change"),
            ("nothing_playing", "nothing is playing"),
            ("invented_by_a_later_version", "some new sentence"),
        ] {
            let toast = message_for(
                &RemoteError::Unavailable {
                    code: code.to_owned(),
                    message: message.to_owned(),
                },
                Locale::BrazilianPortuguese,
            );
            assert!(
                !toast.text.contains(message),
                "`{code}` put the machine's own words on the page: {}",
                toast.text
            );
            assert!(!toast.text.is_empty(), "`{code}` said nothing at all");
        }
    }

    #[test]
    fn a_code_from_a_newer_machine_degrades_to_what_it_used_to_say() {
        // Not a fault and not a blank: `error-unavailable` is the sentence every one of these had
        // before any of them had a name, which is exactly what an older build should fall back to.
        let toast = message_for(
            &RemoteError::Unavailable {
                code: "invented_by_a_later_version".to_owned(),
                message: "whatever it said".to_owned(),
            },
            Locale::BrazilianPortuguese,
        );
        assert_eq!(toast.text, "A máquina não pode fazer isso agora.");
        assert_eq!(toast.level, "toast-warn", "a refusal is not a red failure");
    }
}
