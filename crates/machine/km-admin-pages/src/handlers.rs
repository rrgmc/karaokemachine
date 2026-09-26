//! What each control on the owner's page actually does.
//!
//! **Every handler calls the machine directly** — `km_api::ops::*`, the `Controller` and the
//! `Catalog` — rather than issuing HTTP against the machine's own API from inside the machine's
//! own process. That is the `One implementation of each operation` decision, and `km-remote-pages`
//! makes the same call for the same reason: an HTTP hop here would be a second code path to the
//! same operation, free to answer differently.
//!
//! The exception that proves it is the guard, which *does* go through `ApiState::authorize` with a
//! reconstructed `Authorization` header — because there being one authorization path is the point.

use axum::Form;
use axum::extract::{Multipart, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

use crate::guard::{Caller, PeerAddr, Refusal};
use crate::views::{
    self, BankRow, Chrome, ConfirmPage, Fact, FaultRow, LoginPage, MachinePage, Notice, PackageRow,
    Pane, PicturesPage, ProblemsPage, RefusedRow, SongsPage, SoundPage, Tab,
};
use crate::{APP_CSS, ASSET_VERSION, Admin, STATIC_CACHE};

/// The cookie the admin token rides in.
///
/// The same name `km-remote-pages` uses, deliberately: one machine, one token, one cookie. A browser
/// that signed in on the singer's remote is signed in here, which is right — they are the same
/// password and the same ACL.
pub const TOKEN_COOKIE: &str = "km_token";

/// Refuses anything the caller may not do, before the handler runs.
///
/// **Deny by default**, which is the inversion from `km-remote-pages` and the whole reason this
/// middleware is written out rather than borrowed. Three branches, and in that crate all three are
/// early passes:
///
/// * a request with no `MatchedPath` — refused, because a router that cannot say which pattern
///   matched cannot say what permission the request needs;
/// * a pattern on the open list — allowed, and the list is two entries written out by hand;
/// * anything else — refused without a valid token, the open list being [`crate::guard::is_open`].
pub async fn authorize(
    State(state): State<Admin>,
    PeerAddr(peer): PeerAddr,
    request: Request,
    next: Next,
) -> Response {
    let matched = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|matched| matched.as_str().to_owned());
    let Some(matched) = matched else {
        // Not a pass. A request that reached a handler with no matched path is a router this code
        // does not understand, and the safe reading of that is "refuse", not "allow".
        tracing::warn!(target: crate::LOG_TARGET, "a request arrived with no matched path");
        return refused(
            &state,
            Refusal::Failed("This page could not be checked.".to_owned()),
        )
        .await;
    };

    if crate::guard::is_open(&matched) {
        return next.run(request).await;
    }

    // Every other page here is an admin action, so there is one question rather than a table of
    // them. A route nobody wrote down lands here too, and asks for the password — the safe way to
    // fail, and what `is_open` being an allow-list buys.
    let caller = caller_of(request.headers(), peer);
    match state.guard.allows(&caller).await {
        Ok(()) => next.run(request).await,
        Err(refusal) => refused(&state, refusal).await,
    }
}

/// What a refused request gets.
///
/// A redirect to the login page rather than a 401 with a body, because these are page loads in a
/// browser: somebody who is not signed in wants the box to type a password into, not a status code.
///
/// **One case, because every machine has a password.** A box somebody cannot fill in would imply
/// they had forgotten something, and there is no machine in that state: a PIN is generated at first
/// start. So the redirect is always the right answer.
async fn refused(_state: &Admin, refusal: Refusal) -> Response {
    match refusal {
        Refusal::NeedsPassword(_) => Redirect::to("/admin/login").into_response(),
        // The machine could not answer at all — a poisoned lock, or a controller that failed. Not
        // something a password fixes, so it is not sent to the login page.
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response(),
    }
}

/// Who is asking, from the cookie and the socket.
fn caller_of(headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> Caller {
    Caller {
        token: cookie(headers, TOKEN_COOKIE),
        peer,
    }
}

/// One cookie's value out of a `Cookie` header.
///
/// Written out rather than pulled in, for the reason `km-remote-pages`' own does: a cookie jar crate
/// for one header this crate only ever reads is a dependency bought for nothing.
pub(crate) fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_owned())
}

/// The chrome every page carries.
///
/// # Nothing here fails, and that is deliberate rather than lazy
///
/// **The chrome is the frame that carries a failure message**, so a chrome that could fail would
/// leave a page with nowhere to print one. Each read falls back instead: an unreachable machine gets
/// an empty heading and every switch off, and whatever the page inside is drawing — a refusal, a
/// login form — is what actually tells the reader what happened.
///
/// This matters only in a host that talks over a network. In the machine's own process none of these
/// can fail, and the fallbacks are unreachable.
pub(crate) async fn chrome(state: &Admin, tab: Tab, locale: km_locale::Locale) -> Chrome {
    // One call for every switch, which is what the trait method is shaped for: the Debugging pane
    // draws all three of these together and the console's state is only legible beside debugging's.
    let switches = state.switches.read().await.unwrap_or_default();
    chrome_from(state, tab, switches, locale).await
}

/// The same, for a page that has already read the switches.
///
/// **One read per page rather than one per reader.** The Machine tab wants the demo pair, which the
/// chrome does not carry, so it read them itself and then built a chrome that read them again — two
/// identical sets of requests on the host where each set is four of them.
pub(crate) async fn chrome_from(
    state: &Admin,
    tab: Tab,
    switches: crate::machine::SwitchState,
    locale: km_locale::Locale,
) -> Chrome {
    // **Asked all at once, because none of them is an input to another.** In the machine's own
    // process these are memory reads and the shape costs nothing either way; over HTTP they are
    // separate requests, and in sequence a page that cannot reach its machine waits out one timeout
    // per question before it can say so.
    let (name, factory_password, problems) = tokio::join!(
        state.machine.name(),
        state.guard.factory_password(),
        problem_count(state),
    );
    // The title is the machine's name in a sentence, so it waits for the name rather than joining
    // beside it.
    let machine_name = name.unwrap_or_default();
    Chrome {
        tab,
        assets: ASSET_VERSION,
        title: crate::words::messages(locale)
            .msg_with("page-title", &[("machine", machine_name.clone().into())])
            .into_owned(),
        lang: locale.tag(),
        machine_name,
        // Through the guard rather than off the state, because the guard is this crate's one seam
        // to the machine's idea of who is who — and asking two different objects the same question
        // is how they come to disagree.
        factory_password,
        debug_enabled: switches.debug_running,
        debug_stored: switches.debug_stored,
        dev_remote_stored: switches.dev_remote_stored,
        dev_remote_served: switches.dev_remote_served,
        performance_overlay: switches.performance_overlay,
        // Counted on every page, not only on the Problems tab, because a badge nobody is looking at
        // is the only kind that does any good. Three cheap reads and deliberately no `off_runtime`
        // hop: `package_problems` is a clone out of its own mutex and never touches the catalog
        // lock an install holds for seconds, and the other two are the same snapshot reads the
        // Pictures and Sound tabs already do inline on every load.
        problems,
        capabilities: state.capabilities,
        front_door: false,
        program: state.program,
        scripts: state.scripts,
    }
}

/// How many things are wrong with this machine, for the badge on every tab.
///
/// **Not counted where the badge is not drawn**, and on a host reaching a machine over HTTP that is
/// the difference between one request per page and five. In the machine's own process the reads are
/// cheap enough that this guard buys little; it is here because the crate cannot know which host it
/// is in, which is the whole premise.
async fn problem_count(state: &Admin) -> usize {
    if !state.capabilities.problems {
        return 0;
    }
    let (refused, faults) = tokio::join!(refused_count(state), faults(state));
    refused + faults.len()
}

/// How many package files the machine found and could not use.
///
/// **Counted on every page and not only on the Problems tab**, because a badge nobody is looking at
/// is the only kind that does any good. Zero on a host that cannot answer, which is the honest
/// number: not *no problems*, but *none this surface can see*.
async fn refused_count(state: &Admin) -> usize {
    match &state.problems {
        Some(problems) => problems.refused().await.map(|rows| rows.len()).unwrap_or(0),
        None => 0,
    }
}

/// Everything wrong with this machine that is not a package, in the order the tab lists it.
///
/// **Composed here rather than in the template**, following `PicturesPage::whose`: each of these is
/// a sentence the machine already knows how to word, and a template that branched over the cases
/// would be markup deciding what counts as broken.
///
/// Two of the three appear on no other tab at all. The picture fault is the exception and stays on
/// the Pictures tab as well — there it explains an empty rotation, here it answers *what is wrong
/// with this machine*, which are different questions with one answer.
///
/// **Every one of them now carries the control that answers it**, which is [`views::Fix`]'s own
/// documentation to argue. The lists are built here for the reason the sentences are: a template
/// choosing which banks to offer would be markup deciding what a remedy is.
/// # Composed from the other tabs' traits, and given none of its own
///
/// **There is no `Faults` trait.** Every row here is
/// something one of the other tabs already knows: the bank complaint and the output fallback are
/// [`crate::machine::Sound`]'s, the empty rotation is [`crate::machine::Pictures`]'. A trait of its
/// own would have been a second way to ask the same questions, which is the thing this whole seam
/// exists to stop.
///
/// What *is* the Problems tab's own is the refused-package list, which is why
/// [`crate::machine::Problems`] holds only that.
async fn faults(state: &Admin) -> Vec<FaultRow> {
    let mut faults = Vec::new();

    // `SoundFontStatus::complaint` and not this crate's own reading of `problem` and `fallback`:
    // the difference between "there is no bank" and "the bank you chose is gone and the bundled one
    // is playing" is exactly the thing a second copy would get wrong, and the idle screen already
    // words it. One machine, one sentence.
    if let Some(sound) = state
        .sound
        .loaded()
        .await
        .ok()
        .and_then(|status| status.complaint())
    {
        // Every installed bank, and the one in force marked. This is the screen where somebody
        // manages what is installed, and a fault about the bank must be able to offer one that is
        // merely not *offered* by default.
        let banks = state.sound.banks().await.unwrap_or_default();
        let selected = banks.selected.clone();
        faults.push(FaultRow {
            subject: "Sound",
            text: sound,
            tab: "/admin/sound",
            fix: Some(views::Fix::Bank(
                banks
                    .banks
                    .into_iter()
                    .map(|bank| views::Choice {
                        selected: selected == bank.id,
                        id: bank.id,
                        label: bank.name,
                        is_system_choice: false,
                    })
                    .collect(),
            )),
        });
    }

    // The output device's own fallback, which is `SoundFontStatus::fallback`'s exact twin one layer
    // down. It is the fault a link alone cannot answer: the Sound tab lists banks and never
    // outputs, so *Go there* would reach a tab with no control for it. On an
    // appliance this is somebody having unplugged the audio interface, or the ALSA card order having
    // moved between boots, which is a real fault that otherwise announces itself only as silence.
    //
    // An `Err` is treated as nothing to report rather than as a fault of its own: a machine that
    // cannot enumerate its outputs has a problem this page cannot describe, and the Sound tab is
    // where that surfaces.
    if let Ok(outputs) = state.sound.outputs(false).await
        && outputs.fell_back
    {
        faults.push(FaultRow {
            subject: "Sound",
            text: format!(
                "the audio device that was chosen is not there, so “{}” is playing instead",
                outputs.active_name
            ),
            tab: "/admin/sound",
            // The shortened list, exactly as the Sound tab draws it: one row per physical output.
            // Somebody who needs a spelling that is not offered needs the tab and its `?all=1`, and
            // the link beside this control is how they get there.
            fix: Some(views::Fix::Output(
                output_rows(&outputs, false)
                    .into_iter()
                    .map(|row| views::Choice {
                        selected: row.selected,
                        is_system_choice: row.is_system_choice,
                        label: row.name,
                        id: row.id,
                    })
                    .collect(),
            )),
        });
    }

    if let Some(pictures) = state
        .pictures
        .rotation()
        .await
        .ok()
        .and_then(|rotation| rotation.problem)
    {
        faults.push(FaultRow {
            subject: "Pictures",
            text: pictures,
            tab: "/admin/pictures",
            fix: Some(views::Fix::Picture),
        });
    }

    faults
}

/// Which tab a fix applied from somewhere else should return to.
///
/// **A whitelist returning `&'static str`, and that is the point rather than tidiness.** The value
/// comes off a form and [`back_to`] interpolates it into a redirect, so anything caller-controlled
/// reaching that would be an open redirect. Nothing does: an unknown value falls back to the tab
/// the control belongs to, which is where it would have gone before this field existed.
fn back_tab(asked: Option<&str>, fallback: &'static str) -> &'static str {
    match asked {
        Some("problems") => "problems",
        Some("sound") => "sound",
        Some("pictures") => "pictures",
        Some("songs") => "songs",
        Some("machine") => "machine",
        _ => fallback,
    }
}

/// A notice carried across a redirect, in the query string.
///
/// **A query parameter rather than a flash cookie**, which is one fewer piece of state and survives
/// a reload — the cost is that a shared URL carries somebody's last message, and the messages here
/// are all of the form "added a package", which nothing is harmed by.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct NoticeParams {
    /// `good`, `warn` or `bad`.
    #[serde(default)]
    kind: Option<String>,
    /// What to say.
    #[serde(default)]
    said: Option<String>,
    /// Whether a list that is normally shortened should be shown whole.
    ///
    /// **A query parameter and not a control, because this page has no script.** The Sound tab
    /// offers one row per physical output and hides the other spellings of each; this is the link
    /// that shows them. The same shape `GET /audio/soundfonts?all=true` already uses on the API,
    /// which is where the rule it serves is argued.
    ///
    /// On [`NoticeParams`] rather than a struct of its own so that one `Query` extractor still
    /// answers every page: a second would mean each handler taking two, and only one page has a
    /// list to widen.
    #[serde(default)]
    all: Option<String>,
    /// Which of the Machine tab's panes to open.
    ///
    /// **A query parameter, because this page has no script and a redirect is what a `POST` answers
    /// with.** A save comes back to the pane it was made on, and the only thing that survives a
    /// redirect here is the URL. Read through [`back_pane`], never as it arrived.
    #[serde(default)]
    pane: Option<String>,
    /// Whether the Sound tab's level slider is being shown rather than the link that reveals it.
    ///
    /// **A query parameter for [`NoticeParams::all`]'s reason**, and behind a link for a second one:
    /// the level is the gain into the amplifier, so a slider sitting open on the page is one stray
    /// drag away from a room ten times louder. The number it is set to is drawn either way, because
    /// that is the reading somebody came to the page for.
    #[serde(default)]
    level: Option<String>,
}

impl NoticeParams {
    /// Whether the whole list was asked for. Anything but absent and `0` counts.
    fn all(&self) -> bool {
        matches!(self.all.as_deref(), Some(value) if value != "0")
    }

    /// Whether the level slider was asked for. Anything but absent and `0` counts, like `all`.
    fn level_open(&self) -> bool {
        matches!(self.level.as_deref(), Some(value) if value != "0")
    }

    /// Which pane was asked for, if one was. See [`back_pane`].
    fn pane(&self) -> Option<Pane> {
        self.pane.as_deref().map(back_pane)
    }

    fn into_notice(self) -> Option<Notice> {
        let text = self.said?;
        Some(match self.kind.as_deref() {
            Some("bad") => Notice::bad(text),
            Some("warn") => Notice::warn(text),
            _ => Notice::good(text),
        })
    }
}

/// Which of the Machine tab's panes a redirect named, as a whitelist rather than a fall-through.
///
/// [`back_tab`]'s shape and its reason: the value comes off a query string and is interpolated into
/// a redirect, so what leaves here is an enum and the URL is built from [`Pane::id`]. An unknown
/// name opens Debugging, which is the pane that is never empty.
fn back_pane(asked: &str) -> Pane {
    match asked {
        "name" => Pane::Name,
        "password" => Pane::Password,
        "demo" => Pane::Demo,
        "language" => Pane::Language,
        _ => Pane::Debug,
    }
}

/// Back to a tab, carrying something to say.
fn back_to(tab: &str, kind: &str, said: &str) -> Response {
    let said = urlencode(said);
    Redirect::to(&format!("/admin/{tab}?kind={kind}&said={said}")).into_response()
}

/// Back to one of the Machine tab's panes, carrying something to say.
///
/// **Its own function rather than a fifth argument on [`back_to`]**, which every other tab's forty
/// call sites would then have to pass `None` to. The Machine tab is the only page with panes.
fn back_to_pane(pane: Pane, kind: &str, said: &str) -> Response {
    let said = urlencode(said);
    Redirect::to(&format!(
        "/admin/machine?pane={}&kind={kind}&said={said}",
        pane.id()
    ))
    .into_response()
}

/// Percent-encodes what goes in the query string, in form encoding: a space is `+`.
fn urlencode(text: &str) -> String {
    form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

// -- the pages --------------------------------------------------------------------------------------

/// `GET /admin/` and `GET /admin/songs`.
pub async fn songs_page(
    State(state): State<Admin>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    // Getting off the runtime is the implementation's decision now — see `Songs::packages`, where
    // the catalog mutex an install holds for seconds is what makes it one.
    let packages = state.songs.packages().await.unwrap_or_default();

    let total_songs: usize = packages.iter().map(|listing| listing.song_count).sum();
    let words = crate::words::messages(locale);
    let packages = packages
        .into_iter()
        .map(|listing| {
            let title = package_title(&listing);
            PackageRow {
                remove_label: words
                    .msg_with("package-remove", &[("package", title.as_str().into())])
                    .into_owned(),
                bank_label: words
                    .msg_with("package-bank", &[("package", title.as_str().into())])
                    .into_owned(),
                flags: listing
                    .flag_names
                    .iter()
                    .filter_map(|name| flag_label(words, name))
                    .collect(),
                title,
                id: listing.id,
                version: listing.version,
                songs: listing.song_count,
                bank: listing.bank,
                why_not_removable: listing.why_not_removable,
            }
        })
        .collect::<Vec<PackageRow>>();

    let songs_count = words
        .msg_with(
            "songs-count",
            &[
                ("songs", (total_songs as i64).into()),
                ("packages", (packages.len() as i64).into()),
            ],
        )
        .into_owned();

    views::page(
        &SongsPage {
            chrome: chrome(&state, Tab::Songs, locale).await,
            notice: params.into_notice(),
            songs_count,
            packages,
            accepts: km_api::uploads::accept_for(km_api::machine::Upload::Package),
        },
        locale,
    )
}

/// `GET /admin/problems`.
pub async fn problems_page(
    State(state): State<Admin>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let refused = refused_rows(&state, locale).await;
    let faults = faults(&state).await;
    views::page(
        &ProblemsPage {
            chrome: chrome(&state, Tab::Problems, locale).await,
            notice: params.into_notice(),
            refused,
            faults,
        },
        locale,
    )
}

/// The refused packages as the page draws them, each asked about in the same pass.
///
/// The question and the listing in one hop, for `songs_page`'s reason: a row and the reason its
/// Delete control is missing are then one consistent view of the machine rather than two readings
/// that can disagree.
async fn refused_rows(state: &Admin, locale: km_locale::Locale) -> Vec<RefusedRow> {
    let words = crate::words::messages(locale);
    let Some(problems) = &state.problems else {
        // A host that cannot answer this draws no rows at all rather than an empty *Every package
        // was loaded* — which would be a claim, and a false one.
        return Vec::new();
    };
    problems
        .refused()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|problem| RefusedRow {
            delete_label: words
                .msg_with(
                    "problems-delete-label",
                    &[("file", problem.file.as_str().into())],
                )
                .into_owned(),
            id: problem.id,
            file: problem.file,
            folder: problem.folder,
            reason: problem.reason,
            why_not_removable: problem.why_not_removable,
        })
        .collect()
}

/// `GET /admin/problems/{id}/delete` — the question.
pub async fn confirm_delete_problem(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let found = match &state.problems {
        // Resolved against a freshly read list rather than trusted from the page: the page somebody
        // is looking at may be minutes old and the file it named may already have gone.
        Some(problems) => problems
            .refused()
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|problem| problem.id == id),
        None => None,
    };

    match found {
        Some(problem) if problem.why_not_removable.is_none() => {
            let bytes = problem.bytes;
            let file = problem.file.clone();
            let words = crate::words::messages(locale);
            views::page(
                &ConfirmPage {
                    chrome: chrome(&state, Tab::Problems, locale).await,
                    notice: None,
                    heading: words
                        .msg_with("confirm-delete-heading", &[("file", file.as_str().into())])
                        .into_owned(),
                    facts: vec![
                        Fact::new(words.msg("column-folder"), problem.folder.clone()),
                        Fact::new(words.msg("column-file"), mib(bytes.unwrap_or(0))),
                        Fact::new(words.msg("column-what-is-wrong"), problem.reason.clone()),
                    ],
                    // No song count and no numbers, because there are none: that is what being
                    // refused means, and it is why this is a shorter sentence than a package
                    // removal's.
                    warning: words.msg("confirm-delete-problem").into_owned(),
                    // The one thing somebody might not have thought of, and the reason a rebuilt
                    // package is the better answer whenever there is one.
                    extra: Some(words.msg("confirm-delete-problem-rebuild").into_owned()),
                    action: format!("/admin/problems/{}/delete", problem.id),
                    confirm: words.msg("confirm-delete-button").into_owned(),
                    back: "/admin/problems".to_owned(),
                },
                locale,
            )
        }
        // A file the machine will not delete: its own sentence, and a warning rather than an error.
        Some(problem) => back_to(
            "problems",
            "warn",
            problem.why_not_removable.as_deref().unwrap_or_default(),
        ),
        // The page that linked here has gone stale, and the usual cause is the good one: a rescan
        // took the file in, or somebody removed it another way.
        None => back_to(
            "problems",
            "good",
            &crate::words::messages(locale).msg("problems-nothing-refusing"),
        ),
    }
}

/// `POST /admin/problems/{id}/delete` — the answer.
pub async fn delete_problem(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let Some(problems) = &state.problems else {
        return back_to("problems", "bad", "This surface cannot delete that file.");
    };
    // Getting off the runtime is the implementation's — this deletes a file, which can block for as
    // long as the filesystem wants to, and the rule `Songs::packages` states for the catalog lock
    // applies to the disk as well.
    match problems.delete(&id).await {
        Ok(()) => back_to(
            "problems",
            "good",
            &crate::words::messages(locale).msg("problems-file-gone"),
        ),
        Err(error) => refusal(&state, "problems", &error, locale),
    }
}

/// `GET /admin/pictures`.
pub async fn pictures_page(
    State(state): State<Admin>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let wallpapers = state.pictures.rotation().await.unwrap_or_default();
    let words = crate::words::messages(locale);
    let whose = match wallpapers.source {
        km_api::machine::WallpaperSource::Owner => words.msg("pictures-from-owner"),
        km_api::machine::WallpaperSource::Overlay => words.msg("pictures-from-overlay"),
        km_api::machine::WallpaperSource::Bundled => words.msg("pictures-from-bundled"),
        // The sharpest of the four to report, because adding a picture here would change nothing:
        // `wallpaper.dir` names a folder outright and beats the one an upload writes into.
        km_api::machine::WallpaperSource::Setting => words.msg("pictures-from-setting"),
    };
    views::page(
        &PicturesPage {
            chrome: chrome(&state, Tab::Pictures, locale).await,
            notice: params.into_notice(),
            count: wallpapers.count,
            current: wallpapers.current.clone(),
            problem: wallpapers.problem.clone(),
            whose: whose.into_owned(),
            // Only where an upload would actually take over from a set that is showing. Not for
            // `Owner`, which is already the owner's; and not for `Setting`, where it replaces nothing
            // because a named folder wins outright and `whose` says so instead.
            first_upload_replaces: matches!(
                wallpapers.source,
                km_api::machine::WallpaperSource::Overlay
                    | km_api::machine::WallpaperSource::Bundled
            ),
            // One answer to one question, rather than a second row: *when does the picture
            // change* is answered by the interval and the song trigger together, and a page
            // showing only the first would be stating half of it as all of it. The whole
            // sentence goes through the catalog so a translator owns the word order.
            interval: picture_interval(words, &wallpapers),
            pictures: picture_rows(&state, locale).await,
            accepts: km_api::uploads::accept_for(km_api::machine::Upload::Wallpaper),
        },
        locale,
    )
}

/// When the picture changes, in one sentence.
///
/// The interval is the whole answer only while the song trigger is off, so the plural-aware
/// interval is rendered first and then handed to a second message as a value. Nesting rather than
/// concatenating is what lets a translator put the clauses in the order their language wants, and
/// the catalogs are built with Fluent's bidi isolation off, so a rendered string goes into a
/// placeable unmarked.
fn picture_interval(
    words: &km_locale::Catalog,
    wallpapers: &km_api::machine::WallpaperState,
) -> String {
    let interval = words.msg_with(
        "picture-interval",
        &[("seconds", i64::from(wallpapers.interval_secs).into())],
    );
    if !wallpapers.on_song_change {
        return interval.into_owned();
    }
    words
        .msg_with(
            "picture-interval-and-song",
            &[("interval", interval.as_ref().into())],
        )
        .into_owned()
}

/// The folder's files, as the Pictures tab shows them.
///
/// Shared by the tab and its confirmation, so the sentence a row shows in place of a Remove link is
/// the same string the confirmation would answer with — that is the arrangement the Sound tab
/// already keeps, and it is what stops a control being offered and then refused.
async fn picture_rows(state: &Admin, locale: km_locale::Locale) -> Vec<views::PictureRow> {
    let words = crate::words::messages(locale);
    state
        .pictures
        .files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|picture| views::PictureRow {
            remove_label: words
                .msg_with(
                    "picture-remove",
                    &[("picture", picture.name.as_str().into())],
                )
                .into_owned(),
            id: picture.id,
            name: picture.name,
            images: picture.images,
            size: mib(picture.bytes),
            why_not_removable: picture.why_not_removable,
        })
        .collect()
}

/// `GET /admin/sound`.
pub async fn sound_page(
    State(state): State<Admin>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    // Every bank, not only the offered ones: this page is where somebody manages what is installed.
    let status = state.sound.banks().await.unwrap_or_default();
    let banks = status
        .banks
        .into_iter()
        .map(|bank| BankRow {
            current: status.selected == bank.id,
            size: mib(bank.bytes),
            remove_label: crate::words::messages(locale)
                .msg_with("bank-remove", &[("bank", bank.name.as_str().into())])
                .into_owned(),
            id: bank.id,
            name: bank.name,
            bundled: bank.bundled,
            why_not_removable: bank.why_not_removable,
        })
        .collect();

    // **A failure to enumerate leaves the picker out rather than failing the page.** The banks above
    // are what this tab is chiefly for, and a machine that cannot list its outputs has a fault the
    // Problems tab reports; losing the whole tab to it would be the wrong trade.
    let outputs = state.sound.outputs(params.all()).await.ok();
    let show_all = params.all();
    let level_open = params.level_open();
    let messages = crate::words::messages(locale);
    let level = outputs
        .as_ref()
        .and_then(|outputs| outputs.level.as_ref())
        .map(|level| level_control(level, level_open, locale));

    views::page(
        &SoundPage {
            chrome: chrome(&state, Tab::Sound, locale).await,
            notice: params.into_notice(),
            banks,
            outputs: outputs
                .as_ref()
                .map(|outputs| output_rows(outputs, show_all))
                .unwrap_or_default(),
            outputs_all: show_all,
            output_playing: outputs.as_ref().map(|outputs| {
                messages
                    .msg_with(
                        "output-playing",
                        &[("device", outputs.active_name.as_str().into())],
                    )
                    .into_owned()
            }),
            output_fell_back: outputs.as_ref().filter(|it| it.fell_back).map(|outputs| {
                messages
                    .msg_with(
                        "output-fell-back",
                        &[("device", outputs.active_name.as_str().into())],
                    )
                    .into_owned()
            }),
            output_changeable: outputs.as_ref().is_none_or(|outputs| outputs.changeable),
            level,
            accepts: km_api::uploads::accept_for(km_api::machine::Upload::SoundFont),
        },
        locale,
    )
}

/// The level control, as the Sound tab draws it.
///
/// **The slider's floor is not always the control's.** A control reaching −128 dB would spend five
/// sixths of its travel on settings nobody can hear, so the slider starts at
/// `views::LEVEL_FLOOR_CENTI` and the page says the control goes lower. A machine already set below
/// that floor draws the slider from where it actually is, because a slider that cannot show the
/// current value would snap it up the moment somebody saved.
fn level_control(
    level: &km_api::machine::OutputLevel,
    open: bool,
    locale: km_locale::Locale,
) -> views::LevelControl {
    let floor_centi = views::LEVEL_FLOOR_CENTI
        .max(level.db_min_centi)
        .min(level.db_centi);
    views::LevelControl {
        said: crate::words::messages(locale)
            .msg_with("level-now", &[("db", decibels(level.db_centi).into())])
            .into_owned(),
        db_centi: level.db_centi,
        floor_centi,
        ceiling_centi: level.db_max_centi,
        // A control reporting no step of its own would give a slider that only moves in whole
        // decibels, which is the resolution every control seen so far has anyway.
        step_centi: level.step_centi.max(1),
        deeper_than_slider: level.db_min_centi < floor_centi,
        open,
    }
}

/// Hundredths of a decibel as a person reads them, to one place.
///
/// **One decimal, and the sign is the minus a number carries.** The figure is the one `amixer`
/// prints beside the same control, so somebody comparing the page against a terminal is comparing
/// like with like.
fn decibels(db_centi: i32) -> String {
    format!("{:.1}", f64::from(db_centi) / 100.0)
}

/// The devices to offer, the chosen one always among them.
///
/// **Three rules, and each is one the machine's own decision already states.** The sentinel leads,
/// because *follow the system* is the answer for anybody who has not got a reason; only `preferred`
/// rows are offered unless asked, because one physical output is spelled many ways and a
/// thirty-row list is not a choice; and **whatever is selected is always shown whatever its
/// spelling**, because a choice somebody has to go looking for is a choice they cannot change.
fn output_rows(outputs: &km_api::machine::AudioOutputs, all: bool) -> Vec<views::OutputRow> {
    outputs
        .devices
        .iter()
        .filter(|device| {
            all || device.preferred || outputs.selected.as_deref() == Some(device.id.as_str())
        })
        .map(|device| views::OutputRow {
            selected: outputs.selected.as_deref() == Some(device.id.as_str()),
            is_system_choice: device.id == km_api::machine::SYSTEM_OUTPUT,
            id: device.id.clone(),
            name: device.name.clone(),
            available: device.available,
            system_default: device.system_default,
        })
        .collect()
}

/// A byte count as a person reads it.
///
/// **Three units, and the largest of them is why this grew one.** It was written for SoundFont
/// banks, which stop at about 300 MiB, and it now renders a package's size on a confirmation — where
/// a twenty-gigabyte library would have read as `20480.0 MiB`, which is the one number the whole
/// confirmation exists to make legible.
fn mib(bytes: u64) -> String {
    if bytes == 0 {
        return "—".to_owned();
    }
    let mib = bytes as f64 / (1024.0 * 1024.0);
    if mib < 1.0 {
        format!("{:.0} KiB", bytes as f64 / 1024.0)
    } else if mib < 1024.0 {
        format!("{mib:.1} MiB")
    } else {
        format!("{:.1} GiB", mib / 1024.0)
    }
}

/// `GET /admin/machine`.
pub async fn machine_page(
    State(state): State<Admin>,
    axum::extract::Query(params): axum::extract::Query<NoticeParams>,
    PeerAddr(peer): PeerAddr,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    // **The one place a page asks the guard a question rather than the middleware asking it.** On a
    // host that gates every route, a caller without a token never reaches this handler, so the
    // answer is yes by arrival. On a tool nothing gates, and this asks what the *program* holds --
    // which is what decides whether the pane below draws a password box or a sentence. It is still
    // not asked before a write: the machine is the authority on whether one may happen, and
    // `Capabilities::gate_every_route` argues why a local check there would be the wrong repair.
    let logged_in = if state.capabilities.choose_machine {
        state.guard.allows(&caller_of(&headers, peer)).await.is_ok()
    } else {
        true
    };
    // **The panel's facts and the switches are two reads, not one.** `identity` is the information
    // panel, which is built entirely from what a caller needs no password for; `read` is the five
    // errands below the strip. Keeping them apart is what lets the panel draw on a surface nobody
    // has logged in to yet.
    //
    // **Together rather than one after the other**, for the reason `chrome_from` gives: they are
    // two questions with no dependency between them, and over HTTP a page that waits out the first
    // before starting the second cannot say *the machine is not answering* until it has waited
    // twice.
    let (identity, switches) = tokio::join!(state.machine.identity(), state.switches.read());
    let identity = identity.unwrap_or_default();
    let switches = switches.unwrap_or_default();
    // **The pane asked for, and Debugging otherwise**, which is the pane that is never empty. A
    // tool holding no token still opens the pane it was sent to: what it is missing is on the front
    // door, and the sentence above the strip says so and links there.
    let pane = params
        .pane()
        .unwrap_or(Pane::Debug)
        .drawn(state.capabilities);
    views::page(
        &MachinePage {
            chrome: chrome_from(&state, Tab::Machine, switches, locale).await,
            notice: params.into_notice(),
            pane,
            logged_in,
            open_hint: crate::words::messages(locale)
                .msg_with(
                    "machine-open-hint",
                    &[("addresses", (identity.urls.len() as i64).into())],
                )
                .into_owned(),
            urls: identity.urls,
            songs: identity.songs,
            version: identity.version,
            locales: views::LocaleChoice::all(identity.locale),
            demo_enabled: switches.demo_enabled,
            demo_stored: switches.demo_stored,
            demo_delay_secs: switches.demo_delay_secs,
            demo_delay_max: km_api::machine::MAX_DEMO_DELAY_SECS,
            power: identity.power,
        },
        locale,
    )
}

/// `GET /admin/machine/power/off`.
///
/// **Shutting down asks first and restarting does not**, which extends the confirmation rule rather
/// than breaking it: `ConfirmPage` says only the three controls that destroy a file get one, and the
/// property that earned them is that nothing on the page undoes them. This destroys no file and has
/// that property more strongly than any of the three — bringing the machine back needs somebody to
/// walk to the box. A restart, by contrast, mends itself in ten seconds, which puts it in the same
/// class as showing the next picture.
pub async fn confirm_shut_down(
    State(state): State<Admin>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let words = crate::words::messages(locale);
    if !state.machine.identity().await.is_ok_and(|it| it.power) {
        return back_to("machine", "bad", "This machine cannot switch itself off.");
    }
    views::page(
        &views::ConfirmPage {
            chrome: chrome(&state, Tab::Machine, locale).await,
            notice: None,
            heading: words.msg("confirm-shutdown-heading").into_owned(),
            // Nothing to list: what goes is the machine, which the heading has already named. An
            // empty `facts` renders as an empty grid, which is what it should be.
            facts: Vec::new(),
            warning: words.msg("confirm-shutdown").into_owned(),
            extra: None,
            action: "/admin/machine/power/off".to_owned(),
            confirm: words.msg("confirm-shutdown-button").into_owned(),
            back: "/admin/machine".to_owned(),
        },
        locale,
    )
}

/// `POST /admin/machine/power/off`.
pub async fn shut_down(State(state): State<Admin>, headers: axum::http::HeaderMap) -> Response {
    farewell(state, headers, Farewell::ShuttingDown).await
}

/// `POST /admin/machine/power/restart`.
pub async fn restart_application(
    State(state): State<Admin>,
    headers: axum::http::HeaderMap,
) -> Response {
    farewell(state, headers, Farewell::Restarting).await
}

/// Which of the two just happened.
#[derive(Debug, Clone, Copy)]
enum Farewell {
    ShuttingDown,
    Restarting,
}

/// How long the tab waits before going back to the Machine tab after a restart.
///
/// `RestartSec=2` plus a startup that reads a catalog and takes a screen. Ten seconds is long
/// enough that the reload usually finds a machine answering, and short enough that somebody holding
/// a phone does not conclude the button did nothing. Getting it wrong costs one manual reload.
const RESTART_REFRESH_SECS: u32 = 10;

async fn farewell(state: Admin, headers: axum::http::HeaderMap, which: Farewell) -> Response {
    let locale = state.locale(&headers);
    let words = crate::words::messages(locale);
    // **No availability check here, unlike the confirmation above.** The seam refuses a host with no
    // power controls in its own words, which is one place rather than two — and this handler is
    // reached by a `POST` that may have come from a page drawn before anything was known, so the
    // refusal has to be the machine's answer rather than this page's guess.
    // The keys are spelled out in each arm rather than chosen as `&str` and looked up once below.
    // That is not repetition for its own sake: `no_message_is_left_unused` finds a key by scanning
    // this file for a literal inside a `msg` call, so a catalog entry reached through a variable
    // reads to it as an entry nothing asks for — and the next person to run the tests would delete
    // the translation. (The scanner reads comments too, which is why this sentence does not spell
    // the pattern out.)
    let (heading, said, refresh_in) = match which {
        Farewell::ShuttingDown => (
            words.msg("farewell-shutdown-heading").into_owned(),
            words.msg("farewell-shutdown").into_owned(),
            // Nothing to come back to. A page that kept retrying a box which is off would spend the
            // evening showing a connection error.
            None,
        ),
        Farewell::Restarting => (
            words.msg("farewell-restart-heading").into_owned(),
            words.msg("farewell-restart").into_owned(),
            Some(RESTART_REFRESH_SECS),
        ),
    };
    // **Rendered before the machine is touched, and that ordering is the whole reason this is not
    // three lines shorter.** Restarting sets the shutdown flag, which the display loop notices
    // within one frame; the response then has to survive on the API's graceful-shutdown grace, and
    // it should not also be waiting on askama, a `chrome()` that counts this machine's problems, and
    // a Fluent lookup. Building the page first costs nothing when the request fails — a discarded
    // `String` — and removes the question entirely when it does not.
    let page = views::page(
        &views::FarewellPage {
            chrome: chrome(&state, Tab::Machine, locale).await,
            notice: None,
            heading,
            said,
            refresh_in,
        },
        locale,
    );

    match which {
        Farewell::ShuttingDown => state.machine.shut_down().await,
        Farewell::Restarting => state.machine.restart().await,
    }
    // The operating system's own sentence, which is the diagnosis — and nothing happened, so the
    // machine is still up and there is a page to put it on.
    .map_or_else(
        |error| refusal(&state, "machine", &error, locale),
        |()| page,
    )
}

// -- the controls -----------------------------------------------------------------------------------

/// `POST /admin/songs/upload`.
///
/// **The headers come before the `Multipart`**, and on all three of these, because an extractor that
/// consumes the body has to be last. axum says so with a compile error rather than at run time,
/// which is the one thing about this ordering that needs no comment of its own.
pub async fn upload_package(
    State(state): State<Admin>,
    headers: HeaderMap,
    form: Multipart,
) -> Response {
    match state
        .uploads
        .receive(km_api::machine::Upload::Package, form)
        .await
    {
        Ok(said) => back_to("songs", "good", &said),
        Err(error) => refusal(&state, "songs", &error, state.locale(&headers)),
    }
}

/// `POST /admin/pictures/upload`.
/// **`back` arrives in the query here and in the body everywhere else**, and the difference is the
/// body: this route reads `multipart/form-data` through `uploads::receive`, which consumes the whole
/// of it, so a field beside the file would have to be plucked out of a stream that function owns.
/// A query parameter on a `POST` costs nothing and keeps that function's contract intact.
pub async fn upload_wallpaper(
    State(state): State<Admin>,
    axum::extract::Query(params): axum::extract::Query<BackParams>,
    headers: HeaderMap,
    form: Multipart,
) -> Response {
    let tab = back_tab(params.back.as_deref(), "pictures");
    match state
        .uploads
        .receive(km_api::machine::Upload::Wallpaper, form)
        .await
    {
        Ok(said) => back_to(tab, "good", &said),
        Err(error) => refusal(&state, tab, &error, state.locale(&headers)),
    }
}

/// Where a fix applied from another tab should return to, when it cannot ride in the body.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct BackParams {
    /// The tab the control was pressed on. Whitelisted by [`back_tab`].
    #[serde(default)]
    back: Option<String>,
}

/// `POST /admin/sound/upload`.
pub async fn upload_soundfont(
    State(state): State<Admin>,
    headers: HeaderMap,
    form: Multipart,
) -> Response {
    match state
        .uploads
        .receive(km_api::machine::Upload::SoundFont, form)
        .await
    {
        Ok(said) => back_to("sound", "good", &said),
        Err(error) => refusal(&state, "sound", &error, state.locale(&headers)),
    }
}

/// `GET /admin/songs/{id}/remove` — the question. A `POST` to the same address is the answer.
///
/// **The confirmation is a courtesy and not the guard.** Nothing here is a precondition for the
/// `POST`, which refuses on its own exactly as it did before this page existed — a token or a
/// `confirm=1` field would invent a second enforcement point that the JSON API does not have and
/// that `/dev/` would not send.
///
/// A package the machine will not delete is answered with the machine's own sentence rather than
/// with a confirmation, which is what a stale page or a typed-in URL gets. That is the payoff for
/// `why_not_removable` returning the reason instead of a `bool`.
pub async fn confirm_remove_package(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let found = state.songs.package(&id).await;

    match found {
        Ok(
            ref listing @ crate::machine::Listing {
                why_not_removable: None,
                bytes,
                ..
            },
        ) => {
            let name = package_title(listing);
            let package = listing;
            let songs = package.song_count;
            let words = crate::words::messages(locale);
            views::page(
                &ConfirmPage {
                    chrome: chrome(&state, Tab::Songs, locale).await,
                    notice: None,
                    heading: words
                        .msg_with("confirm-remove-heading", &[("name", name.as_str().into())])
                        .into_owned(),
                    facts: vec![
                        // **First, because it is the half the heading cannot carry.** The name
                        // above names the package; a package holds one build at a time and the
                        // file on disk is not named after it, so this is where somebody about to
                        // delete tens of gigabytes reads which build goes with it.
                        Fact::new(words.msg("column-version"), package.version.clone()),
                        Fact::new(words.msg("column-songs"), songs.to_string()),
                        Fact::new(
                            words.msg("column-numbers"),
                            format!("{}001 to {}999", package.bank, package.bank),
                        ),
                        Fact::new(words.msg("column-file"), mib(bytes.unwrap_or(0))),
                    ],
                    warning: words
                        .msg_with(
                            "confirm-remove-package",
                            &[("songs", (songs as i64).into())],
                        )
                        .into_owned(),
                    extra: None,
                    action: format!("/admin/songs/{}/remove", package.id),
                    confirm: words.msg("confirm-remove-button").into_owned(),
                    back: "/admin/songs".to_owned(),
                },
                locale,
            )
        }
        // A package the machine will not delete: its own sentence, and a warning rather than an
        // error, because nothing is wrong — this is the machine declining.
        Ok(crate::machine::Listing {
            why_not_removable: Some(why),
            ..
        }) => back_to("songs", "warn", &why),
        // **A missing package is worded here rather than by `AdminError::NotFound`'s catalog entry**,
        // because *Not found* names nothing and this page knows what was being looked for.
        Err(crate::machine::AdminError::NotFound) => back_to(
            "songs",
            "bad",
            &crate::words::messages(locale).msg("no-such-package"),
        ),
        Err(error) => refusal(&state, "songs", &error, locale),
    }
}

/// `POST /admin/songs/{id}/remove`.
pub async fn remove_package(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match state.songs.remove(&id).await {
        Ok(songs) => back_to("songs", "good", &format!("Removed {songs} songs.")),
        Err(error) => refusal(&state, "songs", &error, state.locale(&headers)),
    }
}

/// `POST /admin/songs/{id}/bank`.
pub async fn set_bank(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<BankForm>,
) -> Response {
    let bank = form.bank;
    // `km_songcode::MAX_BANK` and not a number typed here, for the reason the password floor is
    // `km_api::MIN_PASSWORD_CHARS`: one limit written down twice is two rules free to disagree.
    // The floor is 1 because bank 0 is the machine's own.
    if bank == 0 || bank > km_songcode::MAX_BANK {
        return back_to(
            "songs",
            "bad",
            &format!("A block runs from 1 to {}.", km_songcode::MAX_BANK),
        );
    }
    match state.songs.set_bank(&id, bank).await {
        Ok(songs) => back_to(
            "songs",
            "good",
            &format!("{songs} songs are now numbered {bank}001 upwards."),
        ),
        Err(error) => refusal(&state, "songs", &error, state.locale(&headers)),
    }
}

/// Which thousand a package's songs are numbered in.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct BankForm {
    /// The block, 1 to `km_songcode::MAX_BANK`.
    pub bank: u16,
}

/// `POST /admin/pictures/next`.
pub async fn next_picture(State(state): State<Admin>, headers: HeaderMap) -> Response {
    match state.pictures.next().await {
        Ok(()) => back_to("pictures", "good", "Showing the next one."),
        Err(error) => refusal(&state, "pictures", &error, state.locale(&headers)),
    }
}

/// `POST /admin/sound/use`.
///
/// **The id is a form field rather than a path segment.** The Problems tab offers this same
/// act as a `<select>` — a fault about the bank is answered by choosing another one — and a
/// `<select>` cannot feed a path segment on a page with no script. Two routes for one act would be
/// worse than moving one, so the Sound tab's per-row buttons carry the id as a hidden field instead.
///
/// `back` is what lets a fix applied from the Problems tab return to the Problems tab. It is
/// whitelisted by [`back_tab`], because it reaches a redirect.
pub async fn use_bank(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<UseBankForm>,
) -> Response {
    let tab = back_tab(form.back.as_deref(), "sound");
    match state.sound.use_bank(&form.id).await {
        Ok(()) => back_to(tab, "good", "That bank is playing now."),
        Err(error) => refusal(&state, tab, &error, state.locale(&headers)),
    }
}

/// Which bank should be playing, and where to go afterwards.
///
/// Not `BankForm`, which is taken: that one is the *thousand* a package is numbered in.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UseBankForm {
    /// The bank's id.
    pub id: String,
    /// The tab this was pressed on, when it was not the Sound tab.
    #[serde(default)]
    pub back: Option<String>,
}

/// `POST /admin/sound/output`.
///
/// **The id arrives as a form field rather than in the path**, unlike [`use_bank`] beside it, and
/// the reason is the control: this is a `<select>`, and a `<select>` cannot feed a path segment on a
/// page with no script. `POST /admin/machine/locale` already has this shape for the same reason.
///
/// A refusal while something is playing comes back as a **warning and not a fault**: nothing is
/// wrong, and it will work when the room stops singing. The machine's own sentence says so — see
/// `OUTPUT_DEVICE_BUSY` — and this page prints it rather than inventing a second wording.
pub async fn use_output(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<OutputForm>,
) -> Response {
    let words = crate::words::messages(state.locale(&headers));
    let tab = back_tab(form.back.as_deref(), "sound");
    match state.sound.set_output(&form.id).await {
        // **Two sentences, because the sentinel is not a device and cannot be named in one.** *The
        // sound comes out of Follow the system now* is what one message with a substitution
        // produces, and it reads as a fault in the page.
        Ok(_) if form.id == km_api::machine::SYSTEM_OUTPUT => {
            back_to(tab, "good", &words.msg("output-changed-system"))
        }
        Ok(outputs) => back_to(
            tab,
            "good",
            &words.msg_with(
                "output-changed",
                // **The device that was chosen, not `active_name`.** That field is what is
                // *sounding*, and `Holding the audio device` means nothing is: the machine hands the
                // endpoint back five seconds after the last song, so the engine reports the
                // placeholder `not yet opened` — and the confirmation read "The sound comes out of
                // not yet opened now." Found by pressing the button on an idle machine, which is
                // every machine somebody is configuring.
                &[("device", chosen_name(&outputs, &form.id).into())],
            ),
        ),
        // Busy reads as a warning and everything else as an error, and `refusal` is what knows
        // which — see `AdminError::severity`. This handler used to make that call itself by matching
        // `ControlError::Unavailable`, which is a distinction only this page happened to draw.
        Err(error) => refusal(&state, tab, &error, state.locale(&headers)),
    }
}

/// `POST /admin/sound/level`.
///
/// **A rise of more than six decibels asks first; a drop never does.** Six decibels is a doubling of
/// voltage, and this is the gain into an amplifier somebody has balanced a room around — a drag from
/// one end of the slider to the other is the accident worth one extra press. Turning it down is
/// audible, costs nothing and is undone by the same slider.
///
/// **The question is the page's, not the route's.** `PUT /api/v1/admin/audio/level` asks nothing, so
/// a script or a command line is unimpeded, which is the arrangement `Removing a bank` already
/// describes: pages ask first and the route does not.
///
/// **The value arrives in the form, or in the query when it is the confirmation coming back.** The
/// confirm page posts an empty body to the address it was given, so what was asked for has to
/// survive in the URL.
pub async fn use_level(
    State(state): State<Admin>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<LevelParams>,
    Form(form): Form<LevelForm>,
) -> Response {
    let locale = state.locale(&headers);
    let words = crate::words::messages(locale);
    let tab = back_tab(form.back.as_deref(), "sound");
    let Some(db) = form.db.or(params.db).filter(|db| db.is_finite()) else {
        return back_to(tab, "bad", &words.msg("level-unreadable"));
    };
    let wanted = (db * 100.0).round() as i32;

    // What it is set to now, which is the only way to know whether this is a rise and by how much.
    // A machine that will not say is let through rather than refused: the control it would be
    // guarding is one the machine is about to answer for itself.
    let now = state
        .sound
        .outputs(false)
        .await
        .ok()
        .and_then(|outputs| outputs.level);
    if !params.confirmed()
        && let Some(now) = now
        && wanted - now.db_centi > views::LEVEL_CONFIRM_RISE_CENTI
    {
        return views::page(
            &ConfirmPage {
                chrome: chrome(&state, Tab::Sound, locale).await,
                notice: None,
                heading: words.msg("confirm-level-heading").into_owned(),
                facts: vec![
                    Fact::new(
                        words.msg("level-confirm-from"),
                        decibels_said(now.db_centi, locale),
                    ),
                    Fact::new(words.msg("level-confirm-to"), decibels_said(wanted, locale)),
                ],
                warning: words.msg("confirm-level").into_owned(),
                extra: None,
                // The value rides in the address because the confirm page posts an empty body.
                action: format!("/admin/sound/level?db={db}&confirmed=1"),
                confirm: words.msg("confirm-level-button").into_owned(),
                back: "/admin/sound".to_owned(),
            },
            locale,
        );
    }

    match state.sound.set_level(wanted).await {
        // **Where it landed, not what was asked for.** A control with coarse steps puts a request
        // between two of them on one of the two, so echoing the request would print a number the
        // card disagrees with.
        Ok(outputs) => {
            let said = outputs
                .level
                .map_or_else(|| decibels(wanted), |level| decibels(level.db_centi));
            back_to(
                tab,
                "good",
                &words.msg_with("level-changed", &[("db", said.as_str().into())]),
            )
        }
        Err(error) => refusal(&state, tab, &error, locale),
    }
}

/// A level as the confirmation's fact list prints it — the number with its unit.
fn decibels_said(db_centi: i32, locale: km_locale::Locale) -> String {
    crate::words::messages(locale)
        .msg_with("level-decibels", &[("db", decibels(db_centi).into())])
        .into_owned()
}

/// Where the level should go, and where to go afterwards.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LevelForm {
    /// Decibels, as the slider sent them.
    ///
    /// Optional because the confirmation posts an empty body and carries the value in the query
    /// instead. See [`LevelParams`].
    #[serde(default)]
    pub db: Option<f32>,
    /// The tab this was pressed on, when it was not the Sound tab.
    #[serde(default)]
    pub back: Option<String>,
}

/// The half of a level change that survives the confirmation page.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LevelParams {
    /// Decibels, when this is the confirmation coming back.
    #[serde(default)]
    pub db: Option<f32>,
    /// Whether the question has already been asked and answered.
    #[serde(default)]
    pub confirmed: Option<String>,
}

impl LevelParams {
    /// Whether the rise was confirmed. Anything but absent and `0` counts, like `all`.
    fn confirmed(&self) -> bool {
        matches!(self.confirmed.as_deref(), Some(value) if value != "0")
    }
}

/// What to call the device that was just chosen.
///
/// Never the sentinel, which its caller answers with a sentence of its own. A device the machine has
/// accepted is always in the list it answered with, so the fallback is unreachable — and it is the id
/// rather than a guess, because an id is at least true.
fn chosen_name<'a>(outputs: &'a km_api::machine::AudioOutputs, id: &'a str) -> &'a str {
    outputs
        .devices
        .iter()
        .find(|device| device.id == id)
        .map_or(id, |device| device.name.as_str())
}

/// Which output the machine should play through, and where to go afterwards.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OutputForm {
    /// The device's identifier, or `system` to follow the system default.
    ///
    /// Opaque here on purpose: `km_api::machine::SYSTEM_OUTPUT` is the sentinel's spelling and the
    /// machine is what knows it. This page offers whatever it was handed and sends it back.
    pub id: String,
    /// The tab this was pressed on, when it was not the Sound tab.
    #[serde(default)]
    pub back: Option<String>,
}

/// `GET /admin/sound/{id}/remove` — the question, and the twin of [`confirm_remove_package`].
///
/// Its own second sentence: removing the bank that is playing is **allowed**, and falls back to the
/// bundled one. That consequence is real and is invisible on the tab today, so the confirmation is
/// where it gets said.
pub async fn confirm_remove_bank(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let status = state.sound.banks().await.unwrap_or_default();
    let selected = status.selected.clone();
    let words = crate::words::messages(locale);
    let Some(bank) = status.banks.into_iter().find(|bank| bank.id == id) else {
        return back_to("sound", "bad", &words.msg("no-such-bank"));
    };
    if let Some(why) = bank.why_not_removable {
        return back_to("sound", "warn", &why);
    }

    views::page(
        &ConfirmPage {
            chrome: chrome(&state, Tab::Sound, locale).await,
            notice: None,
            heading: words
                .msg_with(
                    "confirm-remove-heading",
                    &[("name", bank.name.as_str().into())],
                )
                .into_owned(),
            facts: vec![Fact::new(words.msg("column-size"), mib(bank.bytes))],
            warning: words.msg("confirm-remove-bank").into_owned(),
            extra: (selected == bank.id)
                .then(|| words.msg("confirm-remove-bank-playing").into_owned()),
            action: format!("/admin/sound/{}/remove", bank.id),
            confirm: words.msg("confirm-remove-button").into_owned(),
            back: "/admin/sound".to_owned(),
        },
        locale,
    )
}

/// `POST /admin/sound/{id}/remove`.
pub async fn remove_bank(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match state.sound.delete_bank(&id).await {
        Ok(()) => back_to("sound", "good", "That bank is gone."),
        Err(error) => refusal(&state, "sound", &error, state.locale(&headers)),
    }
}

/// `GET /admin/pictures/{id}/remove`.
///
/// The picture twin of [`confirm_remove_bank`], and it re-asks `why_not_removable` for that one's
/// reason: a page left open in a tab, or a typed URL, must meet the same refusal the row would have
/// shown rather than a confirmation the `POST` then denies.
///
/// **The warning differs from a bank's in the one way that matters to an owner.** A bank falls back
/// to the bundled one and the machine goes on making sound; removing the last picture in the owner's
/// folder hands the rotation back to the set that shipped, because the folder is chosen by contents
/// at every cycle. That is not a failure and it surprises people, so it is said before the click.
pub async fn confirm_remove_picture(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = state.locale(&headers);
    let rows = picture_rows(&state, locale).await;
    let Some(picture) = rows.iter().find(|row| row.id == id).cloned() else {
        return back_to(
            "pictures",
            "bad",
            &crate::words::messages(locale).msg("no-such-picture"),
        );
    };
    if let Some(why) = picture.why_not_removable {
        return back_to("pictures", "warn", &why);
    }
    let last = rows
        .iter()
        .filter(|row| row.why_not_removable.is_none())
        .count()
        == 1;

    let words = crate::words::messages(locale);
    let mut facts = vec![Fact::new(words.msg("column-size"), picture.size.clone())];
    if picture.images > 1 {
        // A zip is one file and several pictures, and the count is the whole of why the row is
        // worth confirming: "remove beach.zip" reads much smaller than "remove seven pictures".
        facts.push(Fact::new(
            words.msg("pictures-in-it"),
            picture.images.to_string(),
        ));
    }

    views::page(
        &ConfirmPage {
            chrome: chrome(&state, Tab::Pictures, locale).await,
            notice: None,
            heading: words
                .msg_with(
                    "confirm-remove-heading",
                    &[("name", picture.name.as_str().into())],
                )
                .into_owned(),
            facts,
            warning: words.msg("confirm-remove-picture").into_owned(),
            extra: last.then(|| words.msg("confirm-remove-picture-last").into_owned()),
            action: format!("/admin/pictures/{}/remove", picture.id),
            confirm: words.msg("confirm-remove-button").into_owned(),
            back: "/admin/pictures".to_owned(),
        },
        locale,
    )
}

/// `POST /admin/pictures/{id}/remove`.
pub async fn remove_picture(
    State(state): State<Admin>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match state.pictures.delete(&id).await {
        Ok(()) => back_to("pictures", "good", "That picture is gone."),
        Err(error) => refusal(&state, "pictures", &error, state.locale(&headers)),
    }
}

/// What to call a package on the page.
///
/// **A package with no name is shown by its id rather than by an empty cell**, because the id is what
/// every control on that row names it by anyway.
///
/// **One function rather than the same three lines in two places**, which is what it was: the Songs
/// row worked the title out and so did its confirmation, and a page that named the same package two
/// ways in two steps of one errand is exactly the kind of thing nobody notices until they hit it.
/// The badge a package flag is drawn as, or `None` for one this page has no word for.
///
/// **One arm per flag, with the key spelled out**, so the catalog parity test sees every key. A key
/// built from the name would hide a missing translation until somebody installed such a package.
fn flag_label(words: &km_locale::Catalog, name: &str) -> Option<String> {
    match name {
        "uncurated" => Some(words.msg("package-flag-uncurated").into_owned()),
        _ => None,
    }
}

fn package_title(listing: &crate::machine::Listing) -> String {
    if listing.name.is_empty() {
        listing.id.clone()
    } else {
        listing.name.clone()
    }
}

/// A refusal from the seam, sent back to the tab it came from in the reader's language.
///
/// **One place decides whether a fault is a code or a sentence**, so no handler has to remember the
/// rule: `AdminError::Refused` carries the machine's own words and is shown as it arrived, and
/// everything else is a code this crate has a catalog entry for. See `machine::say`.
/// **A refusal that wants the password goes to the door that holds one**, wherever it was made. That
/// is [`crate::machine::AdminError::wants_password`]'s purpose: on a tool, a delete refused on the
/// Songs tab is a delete nothing on the Songs tab can mend, and a message naming a page somebody
/// then has to go and find is a message that names the fix without applying it. The machine's own
/// page keeps the tab it was on and the `error-unauthorized` wording, which there is the honest
/// answer: its whole surface is behind that password already.
fn refusal(
    state: &Admin,
    tab: &str,
    error: &crate::machine::AdminError,
    locale: km_locale::Locale,
) -> Response {
    // **`severity` and not a literal `"bad"`**, so *busy* keeps reading as a warning wherever it
    // turns up. Choosing an output while a song is loaded is the case that earned the distinction,
    // and a handler that hard-coded the kind would quietly lose it.
    if error.wants_password() && state.capabilities.choose_machine {
        return back_to_door(
            error.severity(),
            &crate::words::messages(locale).msg("login-needed"),
        );
    }
    back_to(tab, error.severity(), &crate::machine::say(error, locale))
}

/// Out to the host's own front door, carrying something to say.
///
/// **The second route this crate names and does not own**, `Capabilities::searching`'s two being the
/// first. A host that sets `choose_machine` serves `/admin/connect`; without it a refusal for want
/// of a password would land on a page with no way to type one.
fn back_to_door(kind: &str, said: &str) -> Response {
    let said = urlencode(said);
    Redirect::to(&format!("/admin/connect?kind={kind}&said={said}")).into_response()
}

/// The same, for the Machine tab, whose errands are panes rather than pages.
///
/// A refusal comes back to the pane the control is on, so the message and the control it is about
/// are on screen together. A refusal wanting the password still leaves for the front door, because
/// that is the one place any of these can be mended.
fn refusal_on(
    state: &Admin,
    pane: Pane,
    error: &crate::machine::AdminError,
    locale: km_locale::Locale,
) -> Response {
    if error.wants_password() && state.capabilities.choose_machine {
        return refusal(state, "machine", error, locale);
    }
    back_to_pane(pane, error.severity(), &crate::machine::say(error, locale))
}

/// `POST /admin/machine/name`.
pub async fn set_name(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<NameForm>,
) -> Response {
    // Tidied here rather than behind the seam, because `km_api::discover::tidy_name` is the
    // machine's own function and both hosts can call it — what must not be duplicated is a *rule*,
    // and this is the machine's rule being applied where the form is.
    let Some(name) = km_api::discover::tidy_name(&form.name) else {
        return back_to_pane(Pane::Name, "bad", "A machine's name cannot be blank.");
    };
    if let Err(error) = state.machine.set_name(&name).await {
        return refusal_on(&state, Pane::Name, &error, state.locale(&headers));
    }
    back_to_pane(Pane::Name, "good", &format!("This machine is now {name}."))
}

/// What to call the machine.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NameForm {
    /// The typed name, before tidying.
    pub name: String,
}

/// `POST /admin/machine/locale` — what language the television speaks.
///
/// **This is the only way to set it but by hand in `settings.json`**, which is why the picker is
/// here at all: what language a *page* is in follows the browser and can be changed by anybody
/// looking at it, and what language the *screen in the room* is in is a decision about the machine.
///
/// A tag this build has no catalog for is refused rather than silently ignored: the control is a
/// `<select>` of exactly the tags there are, so anything else was typed by hand and saying so beats
/// a Save button that appears to work.
pub async fn set_machine_locale(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<LocaleForm>,
) -> Response {
    let words = crate::words::messages(state.locale(&headers));
    // Parsed here as well as behind the seam, because this is the branch that has a *sentence* for
    // it: `set_locale` answers `NotFound` for a tag it does not know, and `no-such-locale` says what
    // that means on this page. The seam's check is the one that stops a host inventing a tag.
    let Some(chosen) = km_locale::Locale::parse(&form.locale) else {
        return back_to_pane(Pane::Language, "bad", &words.msg("no-such-locale"));
    };
    if let Err(error) = state.machine.set_locale(chosen.tag()).await {
        return refusal_on(&state, Pane::Language, &error, state.locale(&headers));
    }
    // Said in the language just chosen, which is the one honest way to confirm this: somebody who
    // picked the wrong one sees that immediately rather than after walking to the television.
    back_to_pane(
        Pane::Language,
        "good",
        &crate::words::messages(chosen).msg("locale-changed"),
    )
}

/// What language the television should speak.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LocaleForm {
    /// The chosen BCP 47 tag.
    pub locale: String,
}

/// `POST /admin/machine/demo` — turn demo mode on or off.
///
/// **Two checkboxes and not one**, because `PUT /api/v1/demo` takes two answers and a page that
/// hid the second would be unable to say what the route does. `enabled` is what the machine does
/// tonight; `persist` is whether it goes on doing it after a restart. See `Turning demo mode on is
/// an owner's act` in docs/decisions/api-and-network.md.
///
/// An unchecked box posts nothing at all, which is what the `Option`s below are for: absent is
/// `false`, and a form that posted `enabled=off` would be a browser this one does not have to
/// support.
pub async fn set_demo(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<DemoForm>,
) -> Response {
    let words = crate::words::messages(state.locale(&headers));
    let enabled = form.enabled.is_some();
    let persist = form.persist.is_some();
    match state.switches.set_demo(enabled, persist).await {
        Err(error) => refusal_on(&state, Pane::Demo, &error, state.locale(&headers)),
        // The four keys are spelled out rather than chosen into a variable, because
        // `words::tests::no_message_is_left_unused` scans this file for quoted keys passed to the
        // lookup, and one it cannot see is one it reports as dead.
        Ok(_) => {
            let said = match (enabled, persist) {
                (true, true) => words.msg("demo-on-stored"),
                (true, false) => words.msg("demo-on-run"),
                (false, true) => words.msg("demo-off-stored"),
                (false, false) => words.msg("demo-off-run"),
            };
            back_to_pane(Pane::Demo, "good", &said)
        }
    }
}

/// `POST /admin/machine/demo-delay` — how long the quiet has to last.
///
/// **One box and no `persist` beside it**, which is the difference between this form and the one
/// above: a delay is installation configuration and is always
/// written down. `Controller::set_demo_delay` carries the argument, and the two forms in one card
/// are what it costs.
///
/// **A number that is not one is refused here and not by the browser.** `type="number"` and `max`
/// on the input are a courtesy that a phone keyboard, an old browser or a hand-made request all
/// decline in their own way, so the parse and the route's own cap are what actually hold.
pub async fn set_demo_delay(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<DemoDelayForm>,
) -> Response {
    let words = crate::words::messages(state.locale(&headers));
    let Ok(delay_secs) = form.delay_secs.trim().parse::<u32>() else {
        return back_to_pane(Pane::Demo, "bad", &words.msg("demo-delay-bad"));
    };
    match state.switches.set_demo_delay(delay_secs).await {
        Err(error) => refusal_on(&state, Pane::Demo, &error, state.locale(&headers)),
        // **The number the machine stored, not the one that was asked for.** The route caps it, and
        // a page that echoed the request would tell somebody their 9999 had been saved.
        Ok(stored) => back_to_pane(
            Pane::Demo,
            "good",
            &words.msg_with("demo-delay-saved", &[("seconds", i64::from(stored).into())]),
        ),
    }
}

/// `POST /admin/machine/sessions` — sign out everywhere.
///
/// **Password-independent, and that is the point.** An owner who wants every phone in the house
/// logged out should not have to change the password and then tell the house the new one. It ends
/// this browser's session too, which is why the notice says to sign in again rather than leaving it
/// as a surprise on the next click.
pub async fn reset_sessions(State(state): State<Admin>, headers: HeaderMap) -> Response {
    // The epoch arithmetic and the two writes it needs are the implementation's — see
    // `ThisMachine::reset_sessions`, where the ordering is what stops a failed write leaving the
    // machine enforcing an epoch its settings file does not hold.
    if let Err(error) = state.machine.reset_sessions().await {
        return refusal_on(&state, Pane::Password, &error, state.locale(&headers));
    }
    back_to_pane(
        Pane::Password,
        "warn",
        "Every session has ended, including this one. Sign in again to carry on.",
    )
}

/// `POST /admin/machine/debug` — turn debugging mode on or off.
///
/// **It takes effect at the next start, and the notice says so.** The two debug routes are mounted
/// when the router is built rather than checked per request, so a machine with debugging off answers
/// 404 — genuinely not there — instead of carrying a disabled handler. The `debug.` settings section
/// follows the same switch and does take effect immediately, which is why the wording names the
/// routes rather than the mode.
pub async fn set_debug(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<DebugForm>,
) -> Response {
    let enabled = form.enabled.as_deref() == Some("yes");
    if let Err(error) = state.switches.set_debug(enabled).await {
        return refusal_on(&state, Pane::Debug, &error, state.locale(&headers));
    }
    if enabled {
        back_to_pane(
            Pane::Debug,
            "warn",
            "Debugging is on. Anyone on this network can play a file from this machine's disk \
             once it restarts.",
        )
    } else {
        back_to_pane(
            Pane::Debug,
            "good",
            "Debugging is off. The debug routes go away at the next restart.",
        )
    }
}

/// Whether debugging mode should be on.
///
/// Shared by the three switches on the Debugging pane — debugging, the development console and the
/// frame-statistics panel — because all three post exactly one `enabled` field and a struct per
/// switch would be three names for one shape.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DebugForm {
    /// `yes` to turn it on; anything else turns it off.
    #[serde(default)]
    pub enabled: Option<String>,
}

/// `POST /admin/machine/dev-remote` — turn the development console on or off.
///
/// **Beside the debugging switch and not on a pane of its own, because it does not work without
/// it.** `/dev/` and its passwordless API need both, so a pane that separated them would let
/// somebody turn this on, see nothing happen, and have nowhere to find out why. The notice says
/// which switch is still missing rather than only reporting success.
pub async fn set_dev_remote(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<DebugForm>,
) -> Response {
    let enabled = form.enabled.as_deref() == Some("yes");
    if let Err(error) = state.switches.set_dev_remote(enabled).await {
        return refusal_on(&state, Pane::Debug, &error, state.locale(&headers));
    }
    if !enabled {
        return back_to_pane(
            Pane::Debug,
            "good",
            "The development console is off. It goes away at the next restart.",
        );
    }
    // Read back rather than assumed: what this switch does depends on the other one, and an owner
    // who has only ticked this must be told so here rather than at a 404. A read that fails is
    // treated as debugging being off, which is the answer that says more.
    if state
        .switches
        .read()
        .await
        .is_ok_and(|switches| switches.debug_stored)
    {
        back_to_pane(
            Pane::Debug,
            "warn",
            "The development console is on. From the next restart, anyone on this network can \
             change anything on this machine without the password.",
        )
    } else {
        back_to_pane(
            Pane::Debug,
            "warn",
            "The development console is on, but debugging is off, so it will not be served. It \
             needs both.",
        )
    }
}

/// `POST /admin/machine/performance` — draw the frame statistics on the machine's screen, or stop.
///
/// **The one switch on this pane that needs no restart**, and the notice is where that difference
/// gets said: its two neighbours decide which routes are mounted, and this decides what the next
/// frame draws.
pub async fn set_performance(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<DebugForm>,
) -> Response {
    let enabled = form.enabled.as_deref() == Some("yes");
    if let Err(error) = state.switches.set_performance(enabled).await {
        return refusal_on(&state, Pane::Debug, &error, state.locale(&headers));
    }
    if enabled {
        back_to_pane(
            Pane::Debug,
            "good",
            "The frame statistics are on the machine's screen now.",
        )
    } else {
        back_to_pane(
            Pane::Debug,
            "good",
            "The frame statistics are off the screen.",
        )
    }
}

/// Whether the machine performs for itself, and whether that survives a restart.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DemoForm {
    /// Present when the box is ticked, absent when it is not.
    pub enabled: Option<String>,
    /// The same, for the second box.
    pub persist: Option<String>,
}

/// How long the machine waits before performing for itself.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DemoDelayForm {
    /// Seconds, as typed.
    ///
    /// **A `String` rather than a `u32`, and that is the whole reason this type exists.** Axum
    /// answers a `u32` it cannot parse with a bare 422 and no page at all, which for somebody who
    /// typed `two minutes` into the box is the admin surface breaking rather than declining. Parsed
    /// below so a bad number comes back as a notice on the card it was typed into, which is what
    /// every other refusal on this page does.
    #[serde(default)]
    pub delay_secs: String,
}

/// `POST /admin/machine/password` — change or remove the password. Never set the first one.
///
/// **The refusal below is the guard; the template hiding the form is only the courtesy.** On a
/// machine with no password every `admin` mark is dormant, so this route is public — which is what
/// makes a Set button here a way for the first stranger to find the address to claim the machine.
/// Leaving the check to the template would mean a hand-made POST could still do it, which is not a
/// guard at all. The first password comes from `--set-password` or `POST /api/v1/admin/password`.
pub async fn set_password(
    State(state): State<Admin>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    // **Clearing is a separate field and not an empty password box**, so that a form somebody
    // submitted by accident with nothing typed does not take the door off. The template's Remove
    // button carries `clear=yes`; the Change button does not.
    //
    // Checked before the refusal below on purpose: clearing a password that is not there is a no-op
    // rather than something to refuse, and answering it with "this page cannot set one" would say
    // the wrong thing about the wrong act.
    // **Reset, not remove.** A machine always has a password, so the destructive act is going back
    // to a freshly generated PIN rather than taking the door off. The `clear=yes` field is kept
    // because it is still destructive and a form submitted by accident with an empty box must not
    // do it.
    if form.clear.is_some() {
        // `None` is the reset. Generating the PIN, hashing it and moving both the stored and the
        // running value are the implementation's — see `ThisMachine::set_password`.
        if let Err(error) = state.machine.set_password(None).await {
            return refusal_on(&state, Pane::Password, &error, state.locale(&headers));
        }
        // The PIN itself is deliberately not in this message: it is on the machine's own screen,
        // which is the one place reading it means being in the room.
        return back_to_pane(
            Pane::Password,
            "warn",
            "The password is back to a new one the machine generated. It is on the machine's screen.",
        );
    }

    let password = form.password.trim();
    // **The floor is `km_api`'s, not a second copy of it.** This card and `POST /api/v1/admin/password`
    // set the same password on the same machine, so a number written down twice is two rules that can
    // disagree -- and they did, in a way nothing would have reported: this counted *bytes* where the
    // API counts characters, so a two-character CJK password was stored here and refused there.
    let min = km_api::MIN_PASSWORD_CHARS;
    if password.chars().count() < min {
        return back_to_pane(
            Pane::Password,
            "bad",
            &format!("A password wants at least {min} characters."),
        );
    }
    if let Err(error) = state.machine.set_password(Some(password)).await {
        return refusal_on(&state, Pane::Password, &error, state.locale(&headers));
    }
    // Every token was just revoked, this browser's included, so the next page load lands on the
    // login page. Said plainly rather than left as a surprise.
    back_to_pane(
        Pane::Password,
        "good",
        "The password is changed. Sign in again to carry on.",
    )
}

/// A new password, or an instruction to remove the one there is.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PasswordForm {
    /// The typed password. Empty when the Remove button was pressed.
    #[serde(default)]
    pub password: String,
    /// Present only from the Remove button.
    #[serde(default)]
    pub clear: Option<String>,
}

// -- signing in -------------------------------------------------------------------------------------

/// `GET /admin/login`.
pub async fn login_page(State(state): State<Admin>, headers: axum::http::HeaderMap) -> Response {
    let locale = state.locale(&headers);
    // **No early redirect.** A box that cannot be filled in would imply somebody had forgotten
    // something, and there is always a password to fill it with — the factory PIN off the
    // television, where nobody has changed it.
    let _ = &state;
    views::page(
        &LoginPage {
            assets: ASSET_VERSION,
            notice: None,
        },
        locale,
    )
}

/// `POST /admin/login`.
pub async fn login(
    State(state): State<Admin>,
    PeerAddr(peer): PeerAddr,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let caller = caller_of(&headers, peer);
    let remember = form.remember.is_some();
    match state.guard.login(&form.password, &caller, remember).await {
        Ok(crate::guard::LoggedIn::Cookie(grant)) => {
            let cookie = format!(
                "{TOKEN_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
                grant.token, grant.expires_in_secs
            );
            // Not `Secure`: this machine is reached over plain HTTP on a home LAN and always has
            // been — the same call `km-remote-pages` makes, and marking the cookie `Secure` would
            // make signing in silently impossible rather than more private.
            ([(header::SET_COOKIE, cookie)], Redirect::to("/admin/songs")).into_response()
        }
        // **A host that keeps the token itself sets no cookie**, and a page inventing one would be
        // claiming a session it is not the keeper of. See `guard::LoggedIn`.
        //
        // Such a host normally logs in from its own front door and never reaches this route at all —
        // its password box and its address box are one form, because choosing a machine and being
        // let in to it are one errand. This arm is what a token kept rather than set as a cookie
        // answers with if one ever does, carrying a notice: a bare redirect said nothing at all,
        // which on the one control whose whole purpose is to change what the rest of the page can do
        // is the least affordable silence on it.
        Ok(crate::guard::LoggedIn::Kept) => back_to(
            "machine",
            "good",
            &crate::words::messages(state.locale(&headers)).msg("login-yes"),
        ),
        Err(refusal) => views::page(
            &LoginPage {
                assets: ASSET_VERSION,
                notice: Some(Notice::bad(refusal.to_string())),
            },
            state.locale(&headers),
        ),
    }
}

/// The password box.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LoginForm {
    /// What was typed.
    pub password: String,
    /// Present when the box beside it was ticked, absent when it was not.
    ///
    /// **Absent means forget**, which is what makes the box a statement about what this computer
    /// should be remembering rather than an action taken once.
    #[serde(default)]
    pub remember: Option<String>,
}

/// `POST /admin/machine/password/forget` — stop remembering this machine's password.
///
/// **The way out that does not run through logging in.** Somebody who wants to stop remembering is
/// already logged in, so an unticked box on the login form cannot be the only route: it would ask
/// them to type the password they are trying to have forgotten.
///
/// Mounted on both hosts and doing nothing on the machine's, which is the arrangement the power
/// routes take: the path is only ever reached from a control this page draws, so what a typed URL
/// deserves where no box exists is the page saying nothing happened rather than a bare 404.
pub async fn forget_password(State(state): State<Admin>, headers: HeaderMap) -> Response {
    state.guard.forget_password().await;
    let said = crate::words::messages(state.locale(&headers)).msg("login-forgotten");
    if state.capabilities.choose_machine {
        return back_to_door("good", &said);
    }
    back_to("machine", "good", &said)
}

// -- the one static file ------------------------------------------------------------------------------

/// `GET /admin/static/admin.css`.
pub async fn stylesheet() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, STATIC_CACHE),
        ],
        APP_CSS,
    )
        .into_response()
}

/// `GET /admin/static/icon.png` — the mark in the browser tab.
///
/// **The host's, not this crate's**, and not the machine's `/icon.png` either: that route exists in
/// one host and the layout asked every host for it. See [`Admin::icon_png`].
///
/// Open like the stylesheet, and for the same reason — it touches nothing. A favicon behind a login
/// would also be a browser asking for a password before it could draw a tab.
pub async fn favicon(State(state): State<Admin>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, STATIC_CACHE),
        ],
        state.icon_png,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallpapers(interval_secs: u32, on_song_change: bool) -> km_api::machine::WallpaperState {
        km_api::machine::WallpaperState {
            interval_secs,
            on_song_change,
            ..km_api::machine::WallpaperState::default()
        }
    }

    /// **The page answers *when does the picture change* once, with both triggers in it.**
    ///
    /// The interval alone was the whole answer while it was the only trigger, and a sentence that
    /// went on saying so would be stating half of it as all of it — on the one page an owner opens
    /// to find out why the screen behaves as it does.
    #[test]
    fn the_pictures_page_names_both_triggers_when_both_are_on() {
        let words = crate::words::messages(km_locale::Locale::English);

        let timer_only = picture_interval(words, &wallpapers(30, false));
        assert_eq!(timer_only, "30 seconds");

        let both = picture_interval(words, &wallpapers(30, true));
        assert!(both.starts_with("30 seconds"), "{both}");
        assert!(both.contains("song"), "{both}");
    }

    /// The interval keeps its plural inside the longer sentence.
    #[test]
    fn one_second_stays_singular_with_the_song_clause_after_it() {
        let words = crate::words::messages(km_locale::Locale::English);
        assert!(
            picture_interval(words, &wallpapers(1, true)).starts_with("1 second,"),
            "the plural rule has to survive being nested"
        );
    }

    /// Every catalog says both halves, so no locale drops the trigger silently.
    #[test]
    fn every_locale_says_both_triggers() {
        for locale in km_locale::Locale::ALL {
            let words = crate::words::messages(*locale);
            let both = picture_interval(words, &wallpapers(30, true));
            let timer_only = picture_interval(words, &wallpapers(30, false));
            assert!(
                both.len() > timer_only.len(),
                "{locale} says no more with the trigger on than with it off: {both}"
            );
        }
    }
}
