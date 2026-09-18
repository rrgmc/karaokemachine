//! The pages, as askama templates.
//!
//! Struct per template, rendered by the derive. Templates live in `templates/` beside the manifest
//! and are compiled in, so a built executable carries its own pages.

use askama::Template;
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
/// askama resolves `|t` against a module called `filters` in the scope the template was derived in,
/// which is this one. The line every crate with a catalog writes once.
use km_locale::filters;

use crate::server::{Pages, State};

/// Renders a template, or says plainly that it could not.
///
/// **A failed render is a 500 with the reason in it**, not an empty page. askama failures are
/// compile-time in almost every case; the ones that survive to run time are a formatter erroring,
/// and swallowing that would leave a blank page and nothing to search for.
fn render<T: Template>(template: &T, locale: km_locale::Locale) -> Response {
    match template.render_with_values(&filters::values(crate::words::messages(locale))) {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            tracing::error!(%error, "could not render a page");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not render this page: {error}"),
            )
                .into_response()
        }
    }
}

/// One of this program's own pages, inside the owner page's shared chrome.
///
/// # Why this program renders a body and hands it over
///
/// **The chrome is asked for rather than made.** A heading, a tab strip and a factory-password
/// banner built here as well as on the machine's own page are what
/// `Two admin surfaces, one vocabulary` would be holding in step by hand.
///
/// askama cannot `{% extends %}` across a crate, which is why the seam is a wrapper and not a
/// template: this renders its own body and [`km_admin_pages::Admin::shell`] puts it inside the
/// shared frame. **Read that contract before adding a page here** — render a template, pass its
/// output, never build the string.
///
/// **Two catalogs on one page.** This body is rendered from this program's own words and the chrome
/// around it from `km-admin-pages`', each beside the markup that spends it. One locale, read from
/// the request, feeds both. See [`crate::words`].
async fn shell<T: Template>(
    pages: &Pages,
    tab: km_admin_pages::views::Tab,
    template: &T,
    locale: km_locale::Locale,
    notice: Option<km_admin_pages::views::Notice>,
) -> Response {
    // **The render goes in a `let` and not into the `match`.** `filters::values` hands back a
    // `&dyn Any`, which is neither `Send` nor `Sync`, and a `match` scrutinee's temporary lives to
    // the end of the `match` — across the `.await` below, which stops the future being `Send`. axum
    // reports that as `Handler` not being implemented, naming neither the await nor the value.
    let rendered = template.render_with_values(&filters::values(crate::words::messages(locale)));
    match rendered {
        Ok(body) => pages.admin.shell(tab, locale, notice, body).await,
        Err(error) => {
            tracing::error!(%error, "could not render a page");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not render this page: {error}"),
            )
                .into_response()
        }
    }
}

/// This program's front door, in the shared `<head>` and stylesheet and no chrome.
///
/// **[`shell`]'s twin over [`km_admin_pages::Admin::door`]**, and the same contract: render a
/// template here, pass the output, never build the string. What differs is that the wrapper it
/// reaches draws no strip, no heading about a machine and no factory-password banner — and makes no
/// trait call to find out what any of them would say. That argument is written where the wrapper is.
fn door<T: Template>(
    pages: &Pages,
    template: &T,
    locale: km_locale::Locale,
    notice: Option<km_admin_pages::views::Notice>,
) -> Response {
    match template.render_with_values(&filters::values(crate::words::messages(locale))) {
        Ok(body) => pages.admin.door(locale, notice, body),
        Err(error) => {
            tracing::error!(%error, "could not render the front door");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not render this page: {error}"),
            )
                .into_response()
        }
    }
}

/// What a redirect asked the front door to say.
///
/// **Its own type rather than the shared crate's**, which keeps that one private: what reaches this
/// page comes from two places with two catalogs — a refusal composed in `km-admin-pages`, and this
/// program's own answer to its form — and the shape they travel in is the query string both write,
/// not a Rust type either could name.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NoticeParams {
    /// `good`, `warn` or `bad`, and anything else is a fault.
    #[serde(default)]
    pub kind: Option<String>,
    /// What to say.
    #[serde(default)]
    pub said: Option<String>,
}

impl NoticeParams {
    /// The banner to draw, if this load was a redirect carrying one.
    ///
    /// **The kind is matched against a fixed list**, because it reaches the markup as a class name.
    /// Anything a caller invents becomes `bad`, which is the safe reading of a message nobody can
    /// account for.
    #[must_use]
    pub fn into_notice(self) -> Option<km_admin_pages::views::Notice> {
        let said = self.said?;
        Some(match self.kind.as_deref() {
            Some("good") => km_admin_pages::views::Notice::good(said),
            Some("warn") => km_admin_pages::views::Notice::warn(said),
            _ => km_admin_pages::views::Notice::bad(said),
        })
    }
}

/// The front door: which machine, and the password for it.
#[derive(Template)]
#[template(path = "connect.html")]
pub struct Connect {
    /// The machine this program is pointed at now, and the only row that opens pre-selected.
    ///
    /// **Read out of `machine.json` and not off the network**, which is what lets this page draw
    /// instantly and correctly for a television box that is unplugged. `None` before anything has
    /// been chosen, which is the state this page exists to get out of.
    pub chosen: Option<String>,
    /// What the record calls that machine, where it has answered once and said.
    pub chosen_name: Option<String>,
    /// Whether this program holds a token the machine will accept.
    pub logged_in: bool,
    /// The sentence saying this computer has this machine's password, or `None` where it has not.
    ///
    /// **Composed rather than a key**, because it names the machine: which machine a password is for
    /// is the question this page was read as leaving open, and a sentence about *this machine* on a
    /// page whose whole subject is picking one answers it only by accident. The record's name where
    /// there is one, and a sentence that names nothing where there is not — a machine gains a name
    /// and an id together, so the nameless case is a machine whose owner never named it.
    pub saved: Option<String>,
    /// Whether the file a password would be written to can be made owner-only on this platform.
    ///
    /// The page says what is true rather than claiming a protection it did not apply.
    pub owner_only: bool,
    /// Whether the box opens on load rather than behind its summary.
    ///
    /// **Every bad notice on this page is about getting in** — an address that is not one, a machine
    /// that did not answer, a password it refused, a write refused three tabs away — and every one of
    /// them is answered by typing a password. So a refusal opens the box it asks somebody to use
    /// rather than pointing at a closed one.
    pub retype: bool,
    /// Whether to offer the *remember it* box beside the password.
    ///
    /// **Drawn where a password could actually be written down**, which is not the same as a
    /// machine being chosen. A password is keyed by the machine id a login records into the chosen
    /// record, so the box belongs where that record is about the machine in force: on a first login
    /// the id does not exist yet and the record does, which is enough. Where it is not — a
    /// `--machine` run pointed somewhere the record does not name — a tick would be taken and then
    /// discarded, so no box is offered. See [`crate::server::State::can_remember`].
    pub can_remember: bool,
    /// Every language this build has, with the one these pages are in marked.
    ///
    /// **This program's language and not the machine's**, which is the *Screen language* pane on
    /// the Machine tab. The door is where this one belongs because the door is the page about this
    /// program rather than about a machine — and because it is the page that draws before anything
    /// on the network has answered, which is when somebody who cannot read it is still looking at
    /// it.
    ///
    /// Drawn from the shared crate's view model rather than a fourth copy of it: a picker names
    /// each language in itself, and where that rule is kept is not a thing to spell twice.
    pub locales: Vec<km_admin_pages::views::LocaleChoice>,
}

/// The machines a browse turned up, as a fragment the door pulls in after it has drawn.
#[derive(Template)]
#[template(path = "_found.html")]
pub struct Found {
    /// Every machine advertising itself, **none of them pre-selected**.
    pub found: Vec<km_api::discover::Sighting>,
    /// The machine already chosen, so a row that is it can say so rather than offering itself twice.
    pub chosen: Option<String>,
}

/// `GET /admin/connect` — this program's front door.
///
/// **No chrome, and no read of the machine to draw it.** See [`km_admin_pages::Admin::door`]: the
/// tab strip's entries all lead to pages about a machine, and this is the page somebody is on before
/// there is one — often *because* the machine is not answering, which is when the chrome's three
/// reads each cost a full ask timeout.
pub async fn connect(
    axum::extract::State(pages): axum::extract::State<Pages>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    headers: HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let words = crate::words::messages(locale);
    let chosen = crate::chosen::load(pages.state.data_dir());
    // The client's address rather than the record's: a `--machine` run is pointed somewhere the
    // record does not know about, and the door has to offer what this run is actually using.
    let address = pages.state.machine();
    let chosen_name = chosen.as_ref().and_then(|known| known.name.clone());
    let remembering = pages.state.remembering();
    let template = Connect {
        saved: remembering
            .filter(|remembering| remembering.on)
            .map(|_| match &chosen_name {
                Some(name) => words
                    .msg_with("door-saved-for", &[("machine", name.as_str().into())])
                    .into_owned(),
                None => words.msg("door-saved-here").into_owned(),
            }),
        owner_only: crate::passwords::owner_only(),
        // Read before `into_notice` takes the params, which is the whole of the plumbing this
        // needed: the notice is already on its way to the wrapper, and what the box wants to know is
        // the kind rather than the words.
        retype: params.kind.as_deref() == Some("bad"),
        chosen_name,
        can_remember: pages.state.can_remember(),
        chosen: address,
        logged_in: pages.state.logged_in(),
        locales: km_admin_pages::views::LocaleChoice::all(locale),
    };
    door(&pages, &template, locale, params.into_notice())
}

/// `GET /admin/connect/found` — the browse, as a fragment.
///
/// **Its own request because it takes three seconds.** `Discovering a machine in the package
/// builder` binds this program to waiting the whole of it rather than stopping at the first answer,
/// and a page that did that inline would be blank for three seconds every launch — on the program
/// whose front door this is. So the door draws at once with the remembered machine and the address
/// box, and this arrives underneath.
pub async fn found(
    axum::extract::State(state): axum::extract::State<crate::server::State>,
    headers: HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    state.look_again();
    // Following is not adopting: this moves a machine somebody *already chose* to a new address when
    // its id turns up there, which is the exception `machine.json` records.
    state.follow_machine();
    render(
        &Found {
            found: state.machines_seen(),
            chosen: state.machine(),
        },
        locale,
    )
}

/// The Sound page: the whole bank table, and what is to be done about each row.
#[derive(Template)]
#[template(path = "sound.html")]
pub struct Sound {
    /// The machine chosen, which decides whether a Send button is drawn at all.
    pub machine: Option<String>,
    /// Every bank the table knows about.
    pub banks: Vec<BankRow>,
    /// Whether the machine could be asked what it already has.
    pub asked_the_machine: bool,
    /// A job, if one is running or has just finished.
    pub job: Option<crate::job::View>,
    /// Which section's job endpoints `_job.html` should poll.
    ///
    /// **The include needs this and cannot read `chrome`**: an askama include resolves names against
    /// the struct it is rendered from, and the fragment is also rendered on its own — from
    /// [`JobFragment`], which has no chrome to read.
    pub section: &'static str,
    /// What the file chooser will accept, from the machine's own list.
    pub accepts: String,
    /// The largest bank the machine will take, in words.
    pub limit: String,
    /// Where to re-read the list of what is on this computer once a job ends.
    ///
    /// See [`JobFragment::list_route`]: `_job.html` is included here and reads this name.
    pub list_route: Option<&'static str>,
}

/// The bank table on its own, so a finished job can redraw it.
#[derive(Template)]
#[template(path = "_banks.html")]
pub struct Banks {
    /// The machine chosen, which decides whether a Send button is drawn at all.
    pub machine: Option<String>,
    /// Every bank the table knows about.
    pub banks: Vec<BankRow>,
    /// Whether the machine could be asked what it already has.
    pub asked_the_machine: bool,
}

/// `GET /admin/sound/list` — the bank table, for the swap a finished job asks for.
///
/// **A fragment and not a page**, so it renders bare rather than through [`shell`]: it is swapped
/// into a document that already has a heading and a strip, and wrapping it again would nest one page
/// inside another.
pub async fn banks(
    axum::extract::State(state): axum::extract::State<State>,
    headers: HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let (banks, asked_the_machine) = crate::handlers::bank_rows(&state, locale).await;
    render(
        &Banks {
            machine: state.machine(),
            banks,
            asked_the_machine,
        },
        locale,
    )
}

/// One row of the bank table, as the page shows it.
pub struct BankRow {
    /// The id a control sends back.
    pub id: &'static str,
    /// The file it arrives as.
    pub name: &'static str,
    /// Its size in words.
    pub size: &'static str,
    /// What the research note found about its terms.
    pub license: &'static str,
    /// One line on how it sounds.
    pub note: &'static str,
    /// Where this bank stands in the shortlist, if it is in it.
    pub rank: Option<u8>,
    /// The one bank the project suggests.
    pub recommended: bool,
    /// Whether it can be fetched at all, or must be got by hand.
    pub fetchable: bool,
    /// Where to get one that cannot be fetched.
    pub page: Option<&'static str>,
    /// Whether it could be sent to a machine once fetched.
    pub can_be_sent: bool,
    /// Whether this program has already downloaded it.
    ///
    /// **Independent of [`BankRow::on_the_machine`], and not an `else if` between them.** Branching
    /// would stop a sent bank saying it is here, and the two states are the ones the buttons are
    /// about: *here but not sent* is the state the Send button
    /// exists for, and *here and sent* is the one that says a second machine costs no download.
    pub here: bool,
    /// Whether the machine already has it.
    pub on_the_machine: bool,
    /// The sentence the Remove control asks before it deletes this program's copy.
    pub remove_confirm: String,
}

/// `GET /admin/sound/fetch` — the banks this program can go and get.
///
/// # Its own page under the Sound tab, and why
///
/// **The output picker is on the shared page**, beside the machine's own Sound tab, which is what
/// `Two admin surfaces, one vocabulary` asks for and what a second copy could only approximate. What
/// this page holds is the half that is genuinely this program's: sixty-three rows it can *fetch*,
/// which the machine has no business offering
/// because `A fourth program, rather than a fourth tab on the owner's page` says it must not search.
///
/// So this is reached by a link from the shared Sound page rather than being that page.
/// `Capabilities::searching` draws the link, and the searching cannot move into the shared crate —
/// the review grid and the job need `km-wallpaper-pack`'s own types, and a trait with exactly one
/// possible implementation forever is a seam that buys nothing.
pub async fn sound(
    axum::extract::State(pages): axum::extract::State<Pages>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    headers: HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let (banks, asked_the_machine) = crate::handlers::bank_rows(&pages.state, locale).await;
    shell(
        &pages,
        km_admin_pages::views::Tab::Sound,
        &Sound {
            machine: pages.state.machine(),
            banks,
            asked_the_machine,
            job: pages.state.sound_job().map(|job| job.view()),
            section: "sound",
            accepts: crate::handlers::accepts(km_api::machine::Upload::SoundFont),
            limit: crate::handlers::limit_in_words(km_api::machine::Upload::SoundFont),
            list_route: Some(SOUND_LIST),
        },
        locale,
        params.into_notice(),
    )
    .await
}

/// Where each section's "what is on this computer" list is re-read from.
///
/// Named once because two places have to agree: the route the router declares and the `hx-get` a
/// finished job carries.
pub const SOUND_LIST: &str = "/admin/sound/list";
/// The pictures half of [`SOUND_LIST`].
pub const PICTURES_LIST: &str = "/admin/pictures/packs";

/// Where a control that has done its work sends the browser next.
///
/// **Named for the same reason the lists are, and it is the prefix that makes it worth naming.**
/// Everything this program serves is under `/admin` while the routers declare their paths without
/// it, so a `Location` is one of the few places the prefix is spelled by hand — and a redirect to a
/// path nothing mounts is a 404 the button that caused it cannot report. `own_paths_are_routes_this_program_mounts`
/// sweeps these.
pub const SOUND_PAGE: &str = "/admin/sound/fetch";
/// The pictures half of [`SOUND_PAGE`].
pub const PICTURES_PAGE: &str = "/admin/pictures/find";

/// This program's front door, where `/` and [`crate::server`]'s door layer both send a browser and
/// what a shell opens.
///
/// **Named for the reason above and for one more**: a shell opens a page rather than the origin, so
/// this is the one path a browser is handed directly.
pub const CONNECT_PAGE: &str = "/admin/connect";

/// Every path this program writes out for itself that no template holds.
///
/// **The sweep cannot see these**, which is why they are collected here: a const reaches markup
/// through a struct field — `_job.html`'s `hx-get="{{ list_route }}"` — or through a `Location`
/// header, and neither is text a scan of `templates/` can find.
pub const OWN_PATHS: &[&str] = &[
    CONNECT_PAGE,
    SOUND_PAGE,
    PICTURES_PAGE,
    SOUND_LIST,
    PICTURES_LIST,
];

/// The Pictures page.
#[derive(Template)]
#[template(path = "pictures.html")]
pub struct Pictures {
    /// The machine chosen, which decides whether a Send button is drawn at all.
    pub machine: Option<String>,
    /// What is currently set to be searched for.
    pub settings: crate::pictures::Settings,
    /// The providers, and what is known about a key for each.
    pub providers: Vec<ProviderRow>,
    /// Whether this platform can restrict the key file to its owner.
    pub can_restrict: bool,
    /// A job, if one is running or has just finished.
    pub job: Option<crate::job::View>,
    /// Which section's job endpoints to poll.
    pub section: &'static str,
    /// The last run's verdicts, if there are any to show.
    pub review: Option<Review>,
    /// Every pack this program has built and still has.
    pub packs: Vec<PackRow>,
    /// What the file chooser will accept, from the machine's own list.
    pub accepts: String,
    /// The largest picture or pack the machine will take, in words.
    pub limit: String,
    /// Where to re-read the pack list once a job ends.
    pub list_route: Option<&'static str>,
}

/// The pack list on its own, so a finished build can redraw it.
#[derive(Template)]
#[template(path = "_packs.html")]
pub struct Packs {
    /// The machine chosen, which decides whether a Send button is drawn at all.
    pub machine: Option<String>,
    /// Every pack in this program's folder, newest first.
    pub packs: Vec<PackRow>,
}

/// One kept pack, as the list shows it.
pub struct PackRow {
    /// The folder it lives in, which is what a control sends back.
    pub id: String,
    /// The zip's name, which is also what the machine will call it.
    pub name: String,
    /// How many pictures are in it, when the manifest traveled with it.
    pub images: Option<usize>,
    /// How big the zip is, in words.
    pub size: String,
    /// The day it was built, when the manifest traveled with it.
    pub built: Option<String>,
    /// The sentence the Remove control asks before it deletes the zip.
    ///
    /// Composed in `handlers` rather than interpolated in the markup, which is the rule
    /// `km_locale::filters` states and `km-admin-pages` already follows for its own row labels: a
    /// message with a name in it is put together where a test can reach it.
    pub remove_confirm: String,
}

/// `GET /admin/pictures/packs` — the pack list, for the swap a finished job asks for.
pub async fn packs(
    axum::extract::State(state): axum::extract::State<State>,
    headers: HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    render(
        &Packs {
            machine: state.machine(),
            packs: crate::handlers::pack_rows(state.data_dir(), locale),
        },
        locale,
    )
}

/// One provider, as the chooser shows it.
pub struct ProviderRow {
    /// Its value in the form.
    pub value: &'static str,
    /// What to call it.
    pub name: &'static str,
    /// Whether it is the one selected.
    pub chosen: bool,
    /// Whether a search needs a key.
    pub needs_key: bool,
    /// Whether this program has one.
    pub has_key: bool,
    /// Where to get one.
    pub key_page: &'static str,
    /// What somebody should know before using this source.
    ///
    /// **HTML, and rendered with `|safe`** — the emphasis in it is the license conclusion, whether a
    /// pack may be passed on, which is the one thing somebody skims this column for. It was written
    /// as Markdown for a template that escapes, so it printed its own asterisks for as long as it
    /// existed.
    ///
    /// **`&'static str` is the safety argument, not a detail.** Every value is a literal in
    /// `provider_rows` with nothing interpolated into it, so there is no input to escape. A provider
    /// whose terms have to name something learned at run time cannot be added by widening this to
    /// `String` — that would put an unescaped value on the page — and wants escaping at the point it
    /// is built.
    pub terms: &'static str,
}

/// What the last run made of the pictures it looked at.
pub struct Review {
    /// The pictures that will be in the pack.
    pub chosen: Vec<Candidate>,
    /// A count per reason, worst first.
    pub rejected: Vec<(String, usize)>,
    /// How many were looked at in total.
    pub looked_at: usize,
    /// Whether the pack that came out may be passed on.
    pub redistributable: bool,
    /// How many of how many came through, as one sentence.
    ///
    /// Composed rather than built in markup because it carries two counts and a plural, and
    /// `filters`' own note is the reason: *"a plural is arithmetic, which is where it stops being
    /// testable."*
    pub verdict: String,
}

/// One picture in the review grid.
pub struct Candidate {
    /// Which provider, for the thumbnail URL.
    pub provider: String,
    /// Its id there.
    pub id: String,
    /// Who took it.
    pub author: String,
    /// The license it carries.
    pub license: String,
    /// The contrast it will be read at.
    pub contrast: f32,
    /// What a screen reader announces for the thumbnail, naming the photographer.
    pub alt: String,
}

/// `GET /admin/pictures/find` — the searching, on its own page under the Pictures tab.
///
/// Its own page for the reason [`sound`] is: the machine's Pictures tab draws the rotation and takes
/// an upload, and **this draws the half the machine must never have** — the provider chooser, a key
/// somebody pasted, and a few thousand JPEG decodes. `A fourth program, rather than a fourth tab on
/// the owner's page` is why, and `Capabilities::searching` is the link that reaches it.
pub async fn pictures(
    axum::extract::State(pages): axum::extract::State<Pages>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    headers: HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let state = &pages.state;
    let settings = state.pictures_settings();
    let keys = state.keys();
    let page = Pictures {
        machine: state.machine(),
        providers: crate::handlers::provider_rows(&settings, &keys),
        review: crate::handlers::last_review(state, locale),
        packs: crate::handlers::pack_rows(state.data_dir(), locale),
        settings,
        can_restrict: crate::keys::can_restrict(),
        job: state.pictures_job().map(|job| job.view()),
        section: "pictures",
        accepts: crate::handlers::accepts(km_api::machine::Upload::Wallpaper),
        limit: crate::handlers::limit_in_words(km_api::machine::Upload::Wallpaper),
        list_route: Some(PICTURES_LIST),
    };
    shell(
        &pages,
        km_admin_pages::views::Tab::Pictures,
        &page,
        locale,
        params.into_notice(),
    )
    .await
}

/// `GET /admin/pictures/progress`
pub async fn pictures_progress(
    axum::extract::State(state): axum::extract::State<State>,
    headers: HeaderMap,
) -> Response {
    render(
        &JobFragment {
            job: state.pictures_job().map(|job| job.view()),
            section: "pictures",
            list_route: Some(PICTURES_LIST),
        },
        crate::words::locale(&headers),
    )
}

/// The progress fragment, polled while a job runs.
#[derive(Template)]
#[template(path = "_job.html")]
pub struct JobFragment {
    /// The job, if there is one.
    pub job: Option<crate::job::View>,
    /// Where to poll, and where to send a stop.
    pub section: &'static str,
    /// Where the section's "on this computer" list is, for a job that has just ended.
    ///
    /// **The list is stale the moment a job finishes, and only this fragment knows when that is.**
    /// A download ends and the row it belongs to still says *Get*, because the poll swaps `#job` and
    /// nothing else — which was survivable while every button redirected and is not now that the
    /// point of finishing a download is the Send button appearing. So the finished fragment carries
    /// one `hx-get` that fires on load and replaces `#local`.
    ///
    /// `None` for Songs, which lists nothing of its own: a file passing through is not kept, so
    /// there is nothing for it to redraw.
    pub list_route: Option<&'static str>,
}

/// `GET /admin/sound/progress`
pub async fn sound_progress(
    axum::extract::State(state): axum::extract::State<State>,
    headers: HeaderMap,
) -> Response {
    render(
        &JobFragment {
            job: state.sound_job().map(|job| job.view()),
            section: "sound",
            list_route: Some(SOUND_LIST),
        },
        crate::words::locale(&headers),
    )
}

/// The progress fragment on its own, for a handler that has just started a job.
///
/// **A named seam rather than making [`render`] public.** An upload answers with the same fragment
/// `/{section}/progress` returns, so the browser gets a bar that polls itself to the end instead of
/// a page that has to be reloaded to find out what happened — and that is the only thing outside
/// this module that needs to render anything.
pub fn job_fragment(
    job: &crate::job::Job,
    section: &'static str,
    list_route: Option<&'static str>,
    locale: km_locale::Locale,
) -> Response {
    render(
        &JobFragment {
            job: Some(job.view()),
            section,
            list_route,
        },
        locale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bank(here: bool, on_the_machine: bool) -> BankRow {
        BankRow {
            id: "generaluser",
            name: "GeneralUser-GS.sf2",
            size: "30 MB",
            license: "its own",
            note: "a good general bank",
            rank: None,
            recommended: false,
            fetchable: true,
            page: None,
            can_be_sent: true,
            here,
            on_the_machine,
            // Composed in `handlers::bank_rows` from the catalog; these tests are about which
            // controls a row draws, so an empty sentence is honest here.
            remove_confirm: String::new(),
        }
    }

    /// Rendered with a catalog: a bare `render()` leaves every `|t` as `⟦bank-here⟧`, which is what
    /// `km_locale::filters` answers when no locale was passed, and a test asserting on those
    /// brackets would be asserting the failure mode. English, these tests being about which controls
    /// a row draws.
    fn banks_html(row: BankRow, machine: Option<&str>) -> String {
        Banks {
            machine: machine.map(str::to_owned),
            banks: vec![row],
            asked_the_machine: true,
        }
        .render_with_values(&filters::values(crate::words::messages(
            km_locale::Locale::English,
        )))
        .expect("the fragment renders")
    }

    /// **The regression this file exists to hold down.** The two badges were one `else if`, so a
    /// bank that had been sent stopped saying it was on this computer — and with sending now its own
    /// button, that is the row saying the download would have to be repeated when it would not.
    #[test]
    fn a_bank_that_is_here_and_sent_says_both() {
        let html = banks_html(bank(true, true), Some("http://box:8177"));
        assert!(html.contains("on this computer"), "{html}");
        assert!(html.contains("on the machine"), "{html}");
    }

    /// Fetching and sending are two steps, and a row shows the one it is up to.
    #[test]
    fn a_bank_offers_getting_until_it_is_here_and_sending_after() {
        let not_here = banks_html(bank(false, false), Some("http://box:8177"));
        assert!(
            not_here.contains(r#"action="/admin/sound/fetch/generaluser/get""#),
            "{not_here}"
        );
        assert!(
            !not_here.contains("/send"),
            "nothing to send yet: {not_here}"
        );
        assert!(
            !not_here.contains("/remove"),
            "nothing to remove: {not_here}"
        );

        let here = banks_html(bank(true, false), Some("http://box:8177"));
        assert!(
            here.contains(r#"action="/admin/sound/fetch/generaluser/send""#),
            "{here}"
        );
        assert!(
            here.contains(r#"action="/admin/sound/fetch/generaluser/remove""#),
            "{here}"
        );
        assert!(
            !here.contains(r#"action="/admin/sound/fetch/generaluser/get""#),
            "the download is done and does not repeat itself: {here}"
        );
    }

    /// The rule `_send_one.html` already keeps: no control where it could only ever be refused.
    /// Removing is still offered — that one needs no machine.
    #[test]
    fn a_bank_here_with_no_machine_can_be_removed_and_not_sent() {
        let html = banks_html(bank(true, false), None);
        assert!(!html.contains("/send"), "{html}");
        assert!(
            html.contains(r#"action="/admin/sound/fetch/generaluser/remove""#),
            "{html}"
        );
    }

    fn job_html(job: crate::job::View, list_route: Option<&'static str>) -> String {
        JobFragment {
            job: Some(job),
            section: "sound",
            list_route,
        }
        .render()
        .expect("the fragment renders")
    }

    /// **The poll swaps `#job` and nothing else**, so the list a job has just changed would go on
    /// saying what was true before it ran. The finished fragment asks for the list once; the running
    /// one must not, or every second of a download would redraw a table underneath it.
    #[test]
    fn only_a_finished_job_asks_for_the_list_again() {
        let running = crate::job::Job::new("downloading");
        let html = job_html(running.view(), Some(SOUND_LIST));
        assert!(
            !html.contains(SOUND_LIST),
            "a running job redraws nothing: {html}"
        );

        let done = crate::job::Job::new("downloading");
        done.done_with("it is in this program's folder");
        let html = job_html(done.view(), Some(SOUND_LIST));
        assert!(html.contains(SOUND_LIST), "{html}");
        assert!(html.contains(r##"hx-target="#local""##), "{html}");
        assert!(
            !html.contains(r#"hx-trigger="every 1s""#),
            "and it has stopped polling: {html}"
        );
    }

    /// Songs keeps nothing, so it has nothing to redraw — and a fragment that asked anyway would be
    /// a request to a route that does not exist.
    #[test]
    fn a_section_that_keeps_nothing_asks_for_no_list() {
        let done = crate::job::Job::unstoppable("sending");
        done.done_with("it is on the machine");
        let html = job_html(done.view(), None);
        assert!(!html.contains("hx-target"), "{html}");
    }
}
