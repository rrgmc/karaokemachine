//! What the controls on each page do.
//!
//! The views render; these act. The split is `km-package-builder`'s and exists so that a page's
//! shape and a page's behavior are not one file by the time there are three of them.

use axum::Form;
use axum::extract::State as AxumState;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::machine::Refused;
use crate::server::State;
use crate::views::{BankRow, Candidate, ProviderRow, Review};

/// Every row of the bank table, joined against what is here and what the machine has.
///
/// The second half of the answer is whether the machine could be asked at all — a page that showed
/// every row as "not installed" because nothing answered would be stating a falsehood confidently.
pub async fn bank_rows(state: &State, locale: km_locale::Locale) -> (Vec<BankRow>, bool) {
    // What this program has already downloaded is asked of the disk rather than of a record, for the
    // reason `soundfont::installed` reads its folder every time: a file somebody moved in or deleted
    // by hand is then simply true. `landed_bank` is the one definition of it, so the badge on a row,
    // the send route and the remove route cannot come to disagree about whether a bank is here.
    let data_dir = state.data_dir().to_path_buf();

    // What the machine has. `audio.read` ships public, so this usually answers; a machine that is
    // off or has closed it leaves the column unknown rather than wrong.
    let installed = match state.client() {
        Some(client) => client.soundfonts().await.ok(),
        None => None,
    };
    let asked_the_machine = installed.is_some();
    let on_machine: Vec<String> = installed
        .map(|dto| dto.banks.into_iter().map(|bank| bank.name).collect())
        .unwrap_or_default();

    let mut rows: Vec<BankRow> = km_banks::catalog()
        .iter()
        // The bundled bank is on every machine by construction, so a row offering to fetch it would
        // be inviting somebody to download the thing they are already listening to.
        .filter(|bank| !bank.bundled)
        .map(|bank| BankRow {
            id: bank.id,
            name: bank.name,
            size: bank.size,
            license: bank.license,
            note: bank.note,
            rank: bank.rank,
            recommended: bank.recommended,
            fetchable: km_banks::fetchable(bank),
            page: bank.page,
            can_be_sent: crate::bank::can_be_sent(bank),
            here: landed_bank(&data_dir, bank).is_some(),
            on_the_machine: on_machine.iter().any(|name| name == stem_of(bank.name)),
            remove_confirm: crate::words::messages(locale)
                .msg_with("bank-remove-confirm", &[("bank", bank.name.into())])
                .into_owned(),
        })
        .collect();

    downloaded_first(&mut rows);

    (rows, asked_the_machine)
}

/// Floats the banks this program already has to the top of the table.
///
/// **The table is long, most of it is banks nobody here has, and the rows somebody acts on
/// repeatedly — Send it to a machine, Remove it — are exactly the downloaded ones.** They sat
/// wherever catalog order put them, so using this page twice meant finding the same row twice.
///
/// **The sort is stable, and that is load-bearing rather than incidental.** Within each group the
/// order is still [`km_banks::catalog`]'s — the nine ranked banks in rank order, then the rest by
/// ascending loudness spread — which is argued for in that crate's own header and pinned by a test
/// there. A sort that also reshuffled inside the groups would be throwing an answer away to give a
/// different one.
///
/// Split out from [`bank_rows`] so the ordering can be checked without a `State`, a data directory
/// or a machine to ask: the rest of that function is joins against the disk and the network, and
/// this is the only part with a rule in it.
fn downloaded_first(rows: &mut [BankRow]) {
    rows.sort_by_key(|row| !row.here);
}

/// A bank's filename without its extension, which is how the machine names one.
///
/// **The two sides of this join do not spell a bank the same way, and finding that out took sending
/// one.** The table's `name` is a filename — `GXSCC_gm_033.sf2` — because that is what lands on
/// disk; `SoundFontBankDto.name` is "the file as they named it", which `bank_name` builds from
/// `file_stem`. Comparing them directly makes every row read "not on the machine" no matter how many
/// times somebody sends one, which is a wrong answer stated confidently.
fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// `POST /admin/sound/fetch/{id}/get` — fetch a bank into this program's folder.
///
/// **It does not send what it fetched**, which is what makes a download reusable: fetching and
/// sending in one act chooses the machine a gigabyte lands on, so the second machine in the house
/// costs the download again. [`send_bank`] is the other half.
///
/// **Returns as soon as the job exists.** A gigabyte over a slow connection is minutes, and a
/// request that waited for it would look like a hung page — which is the whole reason
/// [`crate::job`] exists.
pub async fn get_bank(
    AxumState(state): AxumState<State>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let Some(bank) = km_banks::bank(&id) else {
        return sound_says(locale, "bad", "bank-unknown");
    };

    // **Already here is already done.** Sending is its own button, so re-downloading a file that is
    // on the disk and verified against its digest is work with no result — and a bank runs to a
    // gibibyte. Remove it first to fetch it again.
    if landed_bank(state.data_dir(), bank).is_some() {
        return Redirect::to(crate::views::SOUND_PAGE).into_response();
    }

    let job = match state.start_sound_job(crate::job::phase::STARTING) {
        Ok(job) => job,
        Err(key) => return sound_says(locale, "warn", key),
    };

    let data_dir = state.data_dir().to_path_buf();
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .user_agent(concat!("km-admin/", env!("CARGO_PKG_VERSION")))
            .build()
        {
            Ok(http) => http,
            Err(error) => {
                job.failed_with(format!("could not build an HTTP client: {error}"));
                return;
            }
        };

        match crate::bank::fetch(&http, bank, &data_dir, &job).await {
            Ok(_) => job.done_with(format!(
                "{} is in this program's folder, ready to send.",
                bank.name
            )),
            Err(why) => job.failed_with(why),
        }
    });

    Redirect::to(crate::views::SOUND_PAGE).into_response()
}

/// `POST /admin/sound/fetch/{id}/send` — put a bank that is already here on the machine.
///
/// **One `reqwest` call with no loop of ours inside it**, so it claims the *unstoppable* half of the
/// section's job slot and the page draws no Stop button — the rule [`crate::job::Job::stoppable`]
/// states, kept here because a control that never does anything teaches somebody to disbelieve the
/// ones that do.
pub async fn send_bank(
    AxumState(state): AxumState<State>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let Some(bank) = km_banks::bank(&id) else {
        return sound_says(locale, "bad", "bank-unknown");
    };
    let Some(landed) = landed_bank(state.data_dir(), bank) else {
        return sound_says(locale, "bad", "bank-not-here");
    };

    let Some(client) = state.client() else {
        return needs_machine(locale);
    };
    // A remembered password is spent here, this being the moment a token is wanted -- see
    // `State::log_in_if_remembered`. Before the check below, or a run that could have logged itself
    // in would refuse and name a box somebody had already filled in.
    state.log_in_if_remembered(&client).await;
    // Before the job slot is claimed, not after: a refusal that has already claimed one leaves a job
    // that exists only to fail.
    if !client.has_token() {
        return needs_password(locale);
    }

    let job = match state.start_sound_send(crate::job::phase::SENDING) {
        Ok(job) => job,
        Err(key) => return sound_says(locale, "warn", key),
    };

    tokio::spawn(async move {
        match client
            .upload(
                crate::machine::Call::Send(km_api::machine::Upload::SoundFont),
                &landed,
            )
            .await
        {
            Ok(said) => job.done_with(said),
            Err(error) => job.failed_with(format!(
                "{} is still in this program's folder, but sending it failed: {error}",
                bank.name
            )),
        }
    });

    Redirect::to(crate::views::SOUND_PAGE).into_response()
}

/// `POST /admin/sound/fetch/{id}/remove` — forget a bank this program downloaded.
///
/// **The path is the table's and never the URL's.** `km_banks::bank` turns the id into a row and the
/// row states the filename, so what is deleted is a name this program wrote down itself — the same
/// rule `bank.rs` keeps about never writing under a name the network chose, read backwards.
pub async fn remove_bank(
    AxumState(state): AxumState<State>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let Some(bank) = km_banks::bank(&id) else {
        return sound_says(locale, "bad", "bank-unknown");
    };
    let Some(landed) = landed_bank(state.data_dir(), bank) else {
        return sound_says(locale, "bad", "bank-not-here");
    };

    if let Err(error) = std::fs::remove_file(&landed) {
        return says(
            crate::views::SOUND_PAGE,
            "bad",
            &crate::words::messages(locale).msg_with(
                "bank-remove-failed",
                &[
                    ("bank", bank.name.into()),
                    ("why", error.to_string().into()),
                ],
            ),
        );
    }
    Redirect::to(crate::views::SOUND_PAGE).into_response()
}

/// Where a fetched bank is, when it is here.
fn landed_bank(
    data_dir: &std::path::Path,
    bank: &km_banks::CatalogBank,
) -> Option<std::path::PathBuf> {
    let path = crate::bank::banks_dir(data_dir).join(bank.name);
    path.is_file().then_some(path)
}

/// `POST /admin/sound/stop` — ask the running job to stop.
pub async fn stop_sound(AxumState(state): AxumState<State>) -> Response {
    if let Some(job) = state.sound_job() {
        job.ask_to_stop();
    }
    Redirect::to(crate::views::SOUND_PAGE).into_response()
}

/// The provider chooser's rows.
///
/// **The terms are on the row, above the key field, rather than in a document.** Which of the three
/// sources produces a pack that may be handed on is the thing somebody is choosing between them on,
/// so the place to say it is where the choice is made.
pub fn provider_rows(
    settings: &crate::pictures::Settings,
    keys: &km_wallpaper_pack::providers::Keys,
) -> Vec<ProviderRow> {
    crate::keys::ALL
        .iter()
        .map(|kind| ProviderRow {
            value: kind.as_str(),
            name: match kind {
                km_wallpaper_pack::config::ProviderKind::Openverse => "Openverse",
                km_wallpaper_pack::config::ProviderKind::Pixabay => "Pixabay",
                km_wallpaper_pack::config::ProviderKind::Pexels => "Pexels",
            },
            chosen: *kind == settings.provider,
            needs_key: kind.needs_key(),
            has_key: keys.get(*kind).is_some(),
            key_page: kind.key_page(),
            terms: match kind {
                km_wallpaper_pack::config::ProviderKind::Openverse => {
                    "An aggregator over Wikimedia Commons, Flickr, museum collections and others, \
                     asked only for CC0, Public Domain Mark and CC BY. <strong>A pack built from \
                     here may be passed on.</strong> It answers without an account, inside three \
                     caps: 200 requests a day, 20 results a page, and no search sees past its \
                     first 240 hits. A token of your own lifts the first two."
                }
                km_wallpaper_pack::config::ProviderKind::Pixabay => {
                    "Needs a key of your own. Pixabay's license covers using its photographs and \
                     not passing a pack of them on, so a pack built from here is <strong>for the \
                     machine that built it</strong>."
                }
                km_wallpaper_pack::config::ProviderKind::Pexels => {
                    "Needs a key of your own. Pexels' API guidelines cover using its photographs \
                     and not passing a pack of them on, so a pack built from here is <strong>for \
                     the machine that built it</strong>."
                }
            },
        })
        .collect()
}

/// Every pack this program has built and still has, newest first.
///
/// **A folder read rather than a record kept**, the arrangement `bank_rows` and `last_review`
/// already make here: a pack somebody deleted by hand is then simply gone, and there is no second
/// copy of the truth to keep in step with the disk.
///
/// **A folder is a pack when it holds a zip.** A run killed between creating the folder and moving
/// the zip into it leaves a folder that is not one, and it is better invisible than listed as
/// something that cannot be sent. The manifest is read where it is there and the row survives
/// without it — the zip is the deliverable, and a pack whose sidecar did not move is still one.
pub fn pack_rows(
    data_dir: &std::path::Path,
    locale: km_locale::Locale,
) -> Vec<crate::views::PackRow> {
    let Ok(entries) = std::fs::read_dir(crate::pictures::packs_dir(data_dir)) else {
        return Vec::new();
    };

    let mut rows: Vec<(std::time::SystemTime, crate::views::PackRow)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let dir = entry.path();
            let zip = zip_in(&dir)?;
            let facts = std::fs::metadata(&zip).ok()?;
            let manifest = km_wallpaper_pack::manifest::Manifest::read(&dir).ok();
            Some((
                facts.modified().unwrap_or(std::time::UNIX_EPOCH),
                crate::views::PackRow {
                    id: entry.file_name().to_string_lossy().into_owned(),
                    name: name_of(&zip),
                    images: manifest.as_ref().map(|it| it.images.len()),
                    size: km_api::handlers::bytes_in_words(facts.len()),
                    // The date and not the time: RFC 3339 up to the `T` is what somebody telling two
                    // searches apart needs, and the rest is noise in a table cell.
                    built: manifest.map(|it| {
                        it.generated_at
                            .split_once('T')
                            .map_or(it.generated_at.clone(), |(day, _)| day.to_owned())
                    }),
                    remove_confirm: crate::words::messages(locale)
                        .msg_with(
                            "pack-remove-confirm",
                            &[("pack", name_of(&zip).as_str().into())],
                        )
                        .into_owned(),
                },
            ))
        })
        .collect();

    // Newest first: the one somebody just built is the one they came to send.
    rows.sort_by_key(|(built, _)| std::cmp::Reverse(*built));
    rows.into_iter().map(|(_, row)| row).collect()
}

/// What the last run made of the pictures it looked at.
///
/// Read from `analysis.json`, which `analyze` writes — so this survives the job being replaced, and
/// a page reloaded an hour later still shows what was decided.
pub fn last_review(state: &State, locale: km_locale::Locale) -> Option<Review> {
    let path = crate::pictures::build_dir(state.data_dir()).join("analysis.json");
    let text = std::fs::read_to_string(path).ok()?;
    let analysis: km_wallpaper_pack::commands::Analysis = serde_json::from_str(&text).ok()?;

    // **Counted by kind and not by label.** A label carries the detail the decision was made from —
    // which photograph was kept, which term filled up — so counting the whole string puts every
    // duplicate in a bucket of one, and the answer somebody came to this line for is which threshold
    // is doing the rejecting. `Rejection::kind_of_label` is that rule, spelled where the enum lives.
    let mut counted: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for rejection in &analysis.rejected {
        *counted
            .entry(km_wallpaper_pack::select::Rejection::kind_of_label(
                &rejection.reason,
            ))
            .or_default() += 1;
    }
    let mut rejected: Vec<(String, usize)> = counted
        .into_iter()
        .map(|(kind, count)| (kind.to_owned(), count))
        .collect();
    // Commonest reason first: what somebody wants to know is which threshold is doing the rejecting.
    rejected.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

    let looked_at = analysis.chosen.len() + analysis.rejected.len();
    // Whether the pack may be passed on is a property of the images in it, derived per image rather
    // than per site — an aggregator's answer differs row by row.
    let redistributable = !analysis.chosen.is_empty()
        && analysis.chosen.iter().all(|chosen| {
            matches!(
                chosen.image.license().redistribution(),
                km_wallpaper_pack::license::Redistribution::Granted
            )
        });

    Some(Review {
        chosen: analysis
            .chosen
            .iter()
            .map(|chosen| Candidate {
                provider: chosen.image.provider.as_str().to_owned(),
                id: chosen.image.id.clone(),
                author: chosen.image.author.clone(),
                license: chosen.image.license().name.clone(),
                contrast: chosen.measured_contrast,
                alt: crate::words::messages(locale)
                    .msg_with(
                        "picture-alt",
                        &[("author", chosen.image.author.as_str().into())],
                    )
                    .into_owned(),
            })
            .collect(),
        rejected,
        looked_at,
        redistributable,
        // Two counts and a plural, so it is composed here rather than in the markup — see
        // `words::COMPOSED`.
        verdict: crate::words::messages(locale)
            .msg_with(
                "review-verdict",
                &[
                    ("chosen", analysis.chosen.len().into()),
                    ("looked_at", looked_at.into()),
                ],
            )
            .into_owned(),
    })
}

/// What the picture search form sends.
#[derive(Debug, Deserialize)]
pub struct SearchForm {
    /// What to call the pack this builds. Empty for none.
    ///
    /// `Option` because it is a late addition: a page cached from before it existed posts a form
    /// without the field, and a missing key would 422 the whole save.
    pub name: Option<String>,
    /// Which provider.
    pub provider: String,
    /// The search terms, one per line.
    pub terms: String,
    /// Pages per term.
    pub pages: u32,
    /// How many pictures the pack should hold.
    pub target_count: usize,
    /// The contrast an image must reach.
    pub target_contrast: f32,
    /// Smallest width to ask a provider for.
    pub min_source_width: u32,
    /// Smallest width the downloaded file may be.
    pub min_decoded_width: u32,
    /// A key for the chosen provider, if one was typed.
    pub key: Option<String>,
    /// Whether to write that key down.
    pub remember: Option<String>,
}

/// `POST /admin/pictures/settings` — remember what to search for, and any key that was typed.
pub async fn set_search(
    AxumState(state): AxumState<State>,
    headers: axum::http::HeaderMap,
    Form(form): Form<SearchForm>,
) -> Response {
    let locale = crate::words::locale(&headers);
    save_search(&state, &form, locale)
        .unwrap_or_else(|| Redirect::to(crate::views::PICTURES_PAGE).into_response())
}

/// Writes down what the search form said, for whichever button sent it.
///
/// **One function because there is one form and two buttons on it.** *Save* and
/// *Search, measure and build* post the same fields to two actions, so a second copy of this would
/// be the way the two came to store a name differently — and a run that read only what had been
/// saved is the bug that made the name box look broken.
/// Answers `Some` only when it refused, so each caller decides what a success looks like.
fn save_search(state: &State, form: &SearchForm, locale: km_locale::Locale) -> Option<Response> {
    let mut settings = state.pictures_settings();
    // Slugged as it is stored rather than as it is used, so what comes back into the field is what a
    // pack will actually be called. A name that slugs away to nothing — punctuation, an accented word
    // with no ASCII in it — is the same as no name, which is the honest answer and is visible
    // immediately.
    settings.name = form.name.as_deref().and_then(crate::pictures::slug);
    settings.provider = match form.provider.as_str() {
        "pixabay" => km_wallpaper_pack::config::ProviderKind::Pixabay,
        "pexels" => km_wallpaper_pack::config::ProviderKind::Pexels,
        _ => km_wallpaper_pack::config::ProviderKind::Openverse,
    };
    settings.set_terms(&form.terms);
    settings.pages = form.pages.clamp(1, 20);
    settings.target_count = form.target_count.clamp(1, 500);
    settings.target_contrast = form.target_contrast.clamp(1.0, 21.0);
    settings.min_source_width = form.min_source_width.clamp(320, 8000);
    settings.min_decoded_width = form.min_decoded_width.clamp(320, 8000);
    state.set_pictures_settings(settings);

    // A key typed into the field replaces the one for that provider and nothing else, so setting a
    // Pexels key cannot silently clear a Pixabay one.
    if let Some(typed) = form.key.as_deref().map(str::trim)
        && !typed.is_empty()
    {
        let mut keys = state.keys();
        match settings_provider(&form.provider) {
            km_wallpaper_pack::config::ProviderKind::Pixabay => {
                keys.pixabay = Some(typed.to_owned());
            }
            km_wallpaper_pack::config::ProviderKind::Pexels => keys.pexels = Some(typed.to_owned()),
            km_wallpaper_pack::config::ProviderKind::Openverse => {
                keys.openverse = Some(typed.to_owned());
            }
        }
        state.set_keys(keys.clone());
        if form.remember.is_some()
            && let Err(error) = crate::keys::remember(state.data_dir(), &keys)
        {
            return Some(says(
                crate::views::PICTURES_PAGE,
                "bad",
                &crate::words::messages(locale)
                    .msg_with("keys-not-remembered", &[("why", error.into())]),
            ));
        }
    }

    None
}

/// Which provider a form value names.
fn settings_provider(value: &str) -> km_wallpaper_pack::config::ProviderKind {
    match value {
        "pixabay" => km_wallpaper_pack::config::ProviderKind::Pixabay,
        "pexels" => km_wallpaper_pack::config::ProviderKind::Pexels,
        _ => km_wallpaper_pack::config::ProviderKind::Openverse,
    }
}

/// `POST /admin/pictures/keys/forget` — delete the remembered keys.
pub async fn forget_keys(
    AxumState(state): AxumState<State>,
    headers: axum::http::HeaderMap,
) -> Response {
    state.set_keys(km_wallpaper_pack::providers::Keys::default());
    match crate::keys::forget(state.data_dir()) {
        Ok(()) => Redirect::to(crate::views::PICTURES_PAGE).into_response(),
        Err(error) => says(
            crate::views::PICTURES_PAGE,
            "bad",
            &crate::words::messages(crate::words::locale(&headers))
                .msg_with("keys-not-forgotten", &[("why", error.into())]),
        ),
    }
}

/// `POST /admin/pictures/run` — search, measure and build, then send the pack.
///
/// **One job for all three phases**, because they are one errand: somebody asked for pictures. The
/// phase name is what says which part is running, and `analyze` and `build` are cheap enough on a
/// warm cache that splitting them into three buttons would be three ways to be half-finished.
pub async fn run_pictures(
    AxumState(state): AxumState<State>,
    headers: axum::http::HeaderMap,
    Form(form): Form<SearchForm>,
) -> Response {
    let locale = crate::words::locale(&headers);
    // **This button carries the form, and that is the whole of what makes the name box work.** The
    // two buttons sit on one form and post to two actions, so what a run uses is what is on the
    // page rather than what was last saved. A run that read only the saved settings built a pack
    // named after whatever had been there before, or after nothing.
    if let Some(refused) = save_search(&state, &form, locale) {
        return refused;
    }
    let settings = state.pictures_settings();
    let keys = state.keys();
    if let Err(why) = settings.ready(keys.get(settings.provider).is_some()) {
        return says(
            crate::views::PICTURES_PAGE,
            "bad",
            &say_not_ready(why, locale),
        );
    }

    let job = match state.start_pictures_job(crate::job::phase::STARTING) {
        Ok(job) => job,
        Err(key) => return pictures_says(locale, "warn", key),
    };

    // **No machine is captured here.** Sending the pack the moment it is built would decide *which
    // machine gets it* an hour before anybody could have chosen one, and make a pack something
    // installed once. It is kept and listed; sending is `send_pack`, and it can be pressed twice
    // against two machines.
    let data_dir = state.data_dir().to_path_buf();
    tokio::spawn(async move {
        match crate::run::pictures(settings, keys, &data_dir, std::sync::Arc::clone(&job)).await {
            Ok(pack) => job.done_with(format!(
                "{} is in this program's folder, ready to send.",
                name_of(&pack)
            )),
            Err(why) => job.failed_with(why),
        }
    });

    Redirect::to(crate::views::PICTURES_PAGE).into_response()
}

/// `POST /admin/pictures/packs/{id}/send` — put a pack that is already here on the machine.
///
/// **Sending is its own act, a day later and twice if wanted.** It takes two calls — the section's
/// unstoppable job, then `upload` on a file already on this disk — so it reports the way a send
/// inside a build would, against whichever machine is chosen when the button is pressed.
pub async fn send_pack(
    AxumState(state): AxumState<State>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let Some(zip) = pack_zip(state.data_dir(), &id) else {
        return pictures_says(locale, "bad", "pack-unknown");
    };

    let Some(client) = state.client() else {
        return needs_machine(locale);
    };
    state.log_in_if_remembered(&client).await;
    // Before the job slot is claimed, for `send_bank`'s reason one screen over.
    if !client.has_token() {
        return needs_password(locale);
    }

    let job = match state.start_pictures_send(crate::job::phase::SENDING) {
        Ok(job) => job,
        Err(key) => return pictures_says(locale, "warn", key),
    };

    tokio::spawn(async move {
        match client
            .upload(
                crate::machine::Call::Send(km_api::machine::Upload::Wallpaper),
                &zip,
            )
            .await
        {
            Ok(said) => job.done_with(said),
            Err(error) => job.failed_with(format!(
                "{} is still in this program's folder, but sending it failed: {error}",
                name_of(&zip)
            )),
        }
    });

    Redirect::to(crate::views::PICTURES_PAGE).into_response()
}

/// `POST /admin/pictures/packs/{id}/remove` — forget a pack this program built.
///
/// **A folder nothing prunes needs a way to prune it**, and the people this program is for are the
/// ones whose machine has no file manager that reaches where files land. Nothing is taken from the
/// machine: a pack that was sent is the machine's now, and removing this copy is removing the copy.
pub async fn remove_pack(
    AxumState(state): AxumState<State>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let locale = crate::words::locale(&headers);
    let Some(dir) = crate::pictures::pack_dir(state.data_dir(), &id).filter(|dir| dir.is_dir())
    else {
        return pictures_says(locale, "bad", "pack-unknown");
    };

    if let Err(error) = std::fs::remove_dir_all(&dir) {
        return says(
            crate::views::PICTURES_PAGE,
            "bad",
            &crate::words::messages(locale).msg_with(
                "pack-remove-failed",
                &[
                    ("pack", id.as_str().into()),
                    ("why", error.to_string().into()),
                ],
            ),
        );
    }
    Redirect::to(crate::views::PICTURES_PAGE).into_response()
}

/// The zip inside one kept pack's folder, when there is one.
fn pack_zip(data_dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    let dir = crate::pictures::pack_dir(data_dir, id)?;
    zip_in(&dir)
}

/// The one zip a pack folder holds, or `None` for a folder that is not a pack.
///
/// **A folder is a pack because it contains a zip**, which is what makes a half-written one — a run
/// killed between `create_dir_all` and the move — invisible rather than a row that cannot be sent.
fn zip_in(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let path = entry.path();
            (path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
                && path.is_file())
            .then_some(path)
        })
}

/// A path's filename, for a sentence somebody reads.
fn name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the pack".to_owned())
}

/// `POST /admin/pictures/stop` — ask the running job to stop.
pub async fn stop_pictures(AxumState(state): AxumState<State>) -> Response {
    if let Some(job) = state.pictures_job() {
        job.ask_to_stop();
    }
    Redirect::to(crate::views::PICTURES_PAGE).into_response()
}

/// `GET /admin/pictures/thumb/{provider}/{id}` — a small version of one candidate.
///
/// **Made once and kept in the cache beside the original**, because the review grid draws a hundred
/// and twenty of these at once and the originals are several hundred megabytes between them.
///
/// **The one handler here that keeps a status and a sentence**, against the rule [`says`] states.
/// What asks for this is an `<img src>`, so there is no document to redirect: a `Location` in an
/// image slot fetches a page into a picture frame, and the browser's own alt text is the failure a
/// reader can actually see.
pub async fn thumbnail(
    AxumState(state): AxumState<State>,
    axum::extract::Path((provider, id)): axum::extract::Path<(String, String)>,
) -> Response {
    let kind = settings_provider(&provider);
    let cache =
        match km_wallpaper_pack::cache::Cache::open(crate::pictures::cache_dir(state.data_dir())) {
            Ok(cache) => cache,
            Err(error) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    error.to_string(),
                )
                    .into_response();
            }
        };

    let bytes = tokio::task::spawn_blocking(move || crate::run::thumbnail(&cache, kind, &id)).await;
    match bytes {
        Ok(Ok(bytes)) => (
            [(axum::http::header::CONTENT_TYPE, "image/jpeg")],
            // A thumbnail is a picture of an immutable original, so it never goes stale.
            bytes,
        )
            .into_response(),
        Ok(Err(why)) => (axum::http::StatusCode::NOT_FOUND, why).into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            error.to_string(),
        )
            .into_response(),
    }
}

/// What a file chooser should accept for one kind, from the machine's own list.
///
/// **`km_api::uploads::accept_for` is the one place the attribute is composed.** Building the string
/// here out of `extensions_for` would be this program's own copy of the rule, agreeing with the
/// owner's page only until somebody changed the table. The media-type note belongs there for the
/// same reason: a picture leads with `image/*` so a phone opens its camera roll, and that reasoning
/// is no more this program's than the extension list is.
#[must_use]
pub fn accepts(kind: km_api::machine::Upload) -> String {
    km_api::uploads::accept_for(kind)
}

/// The largest one kind may be, in words, for the line above the chooser.
///
/// **The machine's own wording**, not a second copy of it: `km_api::uploads::limit_in_words` is
/// what its refusal says too, so "Up to 64 MB" above the chooser and the sentence that comes back
/// when a file is too big cannot come to disagree about one number.
#[must_use]
pub fn limit_in_words(kind: km_api::machine::Upload) -> String {
    km_api::uploads::limit_in_words(kind)
}

/// How many songs the machine has, and in how many packages — when it will say.
///
/// **Asked for and allowed to fail**, exactly as [`machine_status`] treats pictures and banks:
/// `packages.read` ships public, but an owner may have closed it, and a blank line is better than a
/// number that is not true. It is the only thing on the Songs page that answers *did it land*.
pub async fn installed_songs(state: &State) -> Option<(usize, usize)> {
    let dto = state.client()?.packages().await.ok()?;
    Some((dto.song_count, dto.packages.len()))
}

/// Where each kind goes on the machine, which of this program's sections reports on it, and where
/// that section's list of what is on this computer is re-read from.
///
/// **No path is written here.** [`crate::machine::Call::Send`] carries it, from
/// `km_api::uploads::path_for`; a table spelling all three itself is a table that agrees with itself
/// and with nothing else, and it would have to remember the `/admin` every one of them needs. What
/// is left is the part that is genuinely this program's: which of its own pages is watching.
///
/// **Songs has no list**, and needs none: a package passes through and no copy is kept, so there is
/// nothing for a finished send to redraw.
fn destination(
    kind: km_api::machine::Upload,
) -> (
    crate::machine::Call<'static>,
    &'static str,
    Option<&'static str>,
) {
    let call = crate::machine::Call::Send(kind);
    match kind {
        km_api::machine::Upload::Package => (call, "songs", None),
        km_api::machine::Upload::Wallpaper => (call, "pictures", Some(crate::views::PICTURES_LIST)),
        km_api::machine::Upload::SoundFont => (call, "sound", Some(crate::views::SOUND_LIST)),
    }
}

/// A send with no password behind it, answered on the page that holds the password box.
///
/// **`Refused::Unauthorized`'s own wording is deliberately not reused.** That is the wire's
/// vocabulary and it is right where it is drawn — on the machine panel, beside the password box it
/// is asking somebody to use. Sound and Pictures are pages with no password box, so the sentence
/// has to name where one is.
///
/// **The door is where it says, and where this lands.** The password this program logs in with is
/// typed on the front door and nowhere else: the *This machine* tab carries a status banner about
/// the machine's own password, which is a different thing somebody sent here would not find. A
/// redirect puts the sentence and the box on one screen, which is what
/// `km-admin-pages`' `back_to_door` already does with a refusal from the shared half.
fn needs_password(locale: km_locale::Locale) -> Response {
    door_says(
        &crate::words::messages(locale).msg("send-needs-password"),
        "bad",
    )
}

/// A send with no machine chosen, answered on the page that chooses one.
fn needs_machine(locale: km_locale::Locale) -> Response {
    door_says(
        &crate::words::messages(locale).msg("send-needs-machine"),
        "bad",
    )
}

/// Why a part could not be staged.
///
/// # A type rather than a sentence
///
/// A staging fault is answered as a notice on a page, whose status is the redirect's, so a fault
/// that arrived already rendered would be one that could only be rendered the other way.
///
/// **The three cases stay apart on the type**, which is the part worth not losing: a part past the
/// ceiling, axum's own complaint about the envelope, and this program's disk are three different
/// things to be told, and flattening them to a string is how they would come to read alike.
enum StagingFault {
    /// Past the machine's own ceiling for this kind. Carries the limit, in bytes.
    TooLarge(usize),
    /// axum's own complaint, already read through `body_text()` rather than `Display`.
    Multipart(String),
    /// This program's disk, in its words with the system's underneath.
    Local(String),
}

impl StagingFault {
    /// What to say, in a sentence.
    fn sentence(&self) -> String {
        match self {
            Self::TooLarge(limit) => format!(
                "that is larger than this machine accepts — the limit is {}",
                km_api::handlers::bytes_in_words(*limit as u64)
            ),
            Self::Multipart(said) | Self::Local(said) => said.clone(),
        }
    }
}

impl From<StagingFault> for Refused {
    /// **`Local` whichever it was**, because the owner's page shows the sentence and has no use for
    /// the distinction: none of the three is the *machine* refusing, which is what `Refused::Said`
    /// means and what a page must not misreport.
    fn from(fault: StagingFault) -> Self {
        Refused::Local(fault.sentence())
    }
}

/// Streams one part onto disk, refusing anything past the machine's own ceiling.
async fn stage_field(
    state: &State,
    mut field: axum::extract::multipart::Field<'_>,
    extension: &'static str,
    limit: usize,
) -> Result<crate::staging::Staged, StagingFault> {
    use tokio::io::AsyncWriteExt as _;

    let staged = crate::staging::stage(state.data_dir(), extension)
        .map_err(|error| went_wrong("could not make room for it", &error))?;
    let mut file = tokio::fs::File::create(staged.path())
        .await
        .map_err(|error| went_wrong("could not write it down", &error))?;

    // The route's `DefaultBodyLimit` bounds the *request*; this bounds what reaches the disk, and
    // they are not the same bound once the multipart envelope is counted.
    let mut total = 0_usize;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => return Err(staging_fault(&error, limit)),
        };
        total = total.saturating_add(chunk.len());
        if total > limit {
            return Err(StagingFault::TooLarge(limit));
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| went_wrong("could not write it down", &error))?;
    }
    file.flush()
        .await
        .map_err(|error| went_wrong("could not write it down", &error))?;

    Ok(staged)
}

/// Stages one uploaded file and sends it, **waiting for the machine**.
///
/// # Why it waits, where a send from this program's own pages spawns
///
/// **The shared page posts an ordinary form and answers with a redirect carrying a notice**, so it
/// needs the machine's answer before it can say anything. The send controls on this program's own
/// two pages have a job slot and a fragment that polls itself, and spawn into it; this one has
/// neither, both being markup that also serves the machine, where there is no second hop to report
/// on.
///
/// **What it gives up is a percentage, not a report.** The page says an upload is in flight for as
/// long as this takes — see `An upload that waits says it is waiting` in
/// docs/decisions/distribution.md — and what it cannot say is how far along, neither hop's number
/// being the one somebody is waiting on.
///
/// The staging is the sharp half: `Staged` is dropped when this
/// function returns, however it returns, and `crate::staging` says why one guard is not enough.
pub async fn stage_and_send(
    state: &State,
    kind: km_api::machine::Upload,
    mut form: axum::extract::Multipart,
) -> Result<String, Refused> {
    let route = destination(kind).0;
    let limit = km_api::uploads::limit_for(kind);
    let extensions = km_api::uploads::extensions_for(kind);

    let Some(client) = state.client() else {
        return Err(Refused::NoMachine);
    };
    // The same floor the two send controls keep, and for its reason: a gigabyte streamed to disk
    // and then refused for want of a password is an hour somebody did not have to spend.
    if !client.has_token() {
        return Err(Refused::Unauthorized);
    }

    let mut sent: Option<(crate::staging::Staged, String)> = None;
    loop {
        let field = match form.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return Err(staging_fault(&error, limit).into()),
        };
        if field.name() != Some(km_api::uploads::FILE_FIELD) {
            continue;
        }
        if sent.is_some() {
            return Err(Refused::Local("send one file at a time".to_owned()));
        }
        let wire_name = field.file_name().unwrap_or_default().to_owned();
        let Some(extension) = allowed_extension(&wire_name, extensions) else {
            return Err(Refused::Local(format!(
                "the machine takes {} here, and that is {}",
                spelled_out(extensions),
                described(&wire_name)
            )));
        };
        let staged = stage_field(state, field, extension, limit).await?;
        sent = Some((staged, wire_name));
    }

    let Some((staged, wire_name)) = sent else {
        return Err(Refused::Local("there was no file in that".to_owned()));
    };
    // `staged` is still alive across this await and dropped on the way out, which is what keeps a
    // failed send from leaving a copy behind.
    client.upload_as(route, staged.path(), &wire_name).await
}

/// This program's own words about its own disk, with the system's underneath.
fn went_wrong(said: &str, error: &std::io::Error) -> StagingFault {
    StagingFault::Local(format!("{said}: {error}"))
}

/// A multipart failure, **preferring axum's status over its `Display`**.
///
/// That `Display` is the fixed and useless *"Error parsing `multipart/form-data` request"* whatever
/// actually went wrong — the string that cost the machine's own page a debugging session over an
/// 85 MB package that was perfectly well formed and simply too big. `status()` and `body_text()` are
/// the accessors that tell the cases apart.
fn staging_fault(error: &axum::extract::multipart::MultipartError, limit: usize) -> StagingFault {
    if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        return StagingFault::TooLarge(limit);
    }
    StagingFault::Multipart(error.body_text().to_string())
}

/// The extension a filename ends in, if the machine accepts it.
///
/// **Returns the `&'static str` from the list rather than the client's own bytes**, which is what
/// makes it safe for [`crate::staging::stage`] to put it in a path. The same trick the machine's
/// `allowed_extension` uses, and the reason this does not simply lowercase what it was given.
fn allowed_extension(file_name: &str, allowed: &[&'static str]) -> Option<&'static str> {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    allowed.iter().copied().find(|known| *known == extension)
}

/// `.kmpkg`, or `.jpg, .jpeg, .png, .webp, .bmp or .zip` — for a sentence, not for an `accept=`.
fn spelled_out(extensions: &[&'static str]) -> String {
    let dotted: Vec<String> = extensions.iter().map(|e| format!(".{e}")).collect();
    match dotted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
}

/// What the thing somebody picked appears to be, for the other half of that sentence.
fn described(file_name: &str) -> String {
    match std::path::Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some(extension) => format!("a .{}", extension.to_ascii_lowercase()),
        None => "a file with no extension at all".to_owned(),
    }
}

// -- the front door ------------------------------------------------------------------------------

/// Which machine the door was submitted with, and the password for it.
#[derive(Debug, Clone, Deserialize)]
pub struct EnterForm {
    /// The row that was picked: `chosen`, `typed`, or a discovered machine's URL.
    ///
    /// **A radio group and not a free field**, so that submitting without picking is impossible and
    /// every discovered machine has to be pressed. The two fixed names are the two rows whose
    /// address is not in the form: the machine this run is already pointed at, and the box below.
    pub row: String,
    /// What was typed, read only when `row` is `typed`.
    ///
    /// **Opaque here**, exactly as the box it replaces was: what an address may be is
    /// [`crate::machine::normalize`]'s rule, and it is the one that has to reach it.
    #[serde(default)]
    pub typed: String,
    /// The machine's password, or empty to spend the way in this program already has.
    ///
    /// **Blank is not *no password*, it is *the one already held*** — remembered on this computer,
    /// or a token bought earlier this run — and a pass with neither to fall back on is turned away
    /// at the door. See [`enter`].
    #[serde(default)]
    pub password: String,
    /// Present when the *remember it* box was ticked, absent when it was not.
    ///
    /// **Read only where a password was typed**, the box being drawn only there.
    #[serde(default)]
    pub remember: Option<String>,
}

/// `POST /admin/connect/use` — point this program at a machine and let it in.
///
/// # The four rules this keeps, each of them decided elsewhere
///
/// **Listing is not setting.** Nothing here is reached except by a row somebody picked and a button
/// they pressed. See `Discovering a machine in the package builder`.
///
/// **`--machine` wins for the run and is not written down.** The *chosen* row is whatever client
/// this run holds, which under `--machine` is the command line's address — so entering on it must
/// not write a record. **Setting is skipped where the address is the one already held**, which
/// covers that and one more case worth having: a re-entry after a refused write keeps the token it
/// already has instead of throwing it away with the client.
///
/// **The door does not open without a password the machine accepted.** The alternative is a program
/// whose every write refuses three screens later, which reads as a broken machine rather than as a
/// question nobody answered. See `The front door asks for a password` in
/// docs/decisions/distribution.md, which is also where the cost is written down: a machine that is
/// switched off can no longer be entered.
///
/// **A password is stored only after it has been accepted**, so a typo is never written down — the
/// order here, not a check afterwards.
///
/// **The tick rides with a typed password and says nothing about one that was not.** The box is a
/// statement about what this computer should remember, and it is spent on the password in hand — so
/// a pass that types one applies it, unticked included, and a pass that spends what is already
/// remembered leaves the store alone. The door draws no box where nothing is being typed, and an
/// entry that quietly forgot what it had just used would be the page's own convenience deleting a
/// credential. *Forget it* is the control that means that, and it is beside the sentence saying
/// there is something to forget.
pub async fn enter(
    AxumState(state): AxumState<State>,
    headers: axum::http::HeaderMap,
    Form(form): Form<EnterForm>,
) -> Response {
    let locale = crate::words::locale(&headers);
    let words = crate::words::messages(locale);
    let held = state.machine();

    let address = match form.row.as_str() {
        "chosen" => held.clone(),
        "typed" => Some(form.typed.trim().to_owned()).filter(|typed| !typed.is_empty()),
        url => Some(url.to_owned()),
    };
    let Some(address) = address else {
        return door_says(&words.msg("door-needs-an-address"), "bad");
    };

    // Not `set_machine` where it would write down what the command line said for this run only.
    if Some(crate::machine::normalize(&address)) != held {
        state.set_machine(Some(address));
    }

    let Some(client) = state.client() else {
        return door_says(&words.msg("door-no-machine"), "bad");
    };

    // **What is spent: the password typed, or the one this computer remembers for this machine.**
    // A blank box is not an answer of its own — it means *use what you already have* — so a run
    // with nothing to fall back on is turned away here rather than three screens later.
    let typed = form.password.trim();
    let remembered = state.remembered_password();
    let password = if typed.is_empty() {
        remembered.as_deref().unwrap_or_default()
    } else {
        typed
    };
    if password.is_empty() {
        // **A token in hand is a password the machine accepted**, which is the whole of what this
        // door asks for. Somebody who logged in without ticking the box and came back here — from a
        // tab, or from a write refused while the token was still good — has nothing to type and
        // nothing remembered, and refusing them would be this page demanding an answer it already
        // holds. Only ever the address this run is already on: the branch above replaced the client
        // for any other, and a fresh client holds no token.
        if client.has_token() {
            return Redirect::to("/admin/machine").into_response();
        }
        return door_says(&words.msg("door-needs-a-password"), "bad");
    }

    match client.log_in(password).await {
        Ok(()) => {
            // **The identity, learned here because this is the first moment it can be.** A password
            // is keyed by the machine's id and nothing on this page has read one: the door asks the
            // machine nothing so that it draws at once. A login is proof that this machine is up —
            // it just answered — so the `/discover` that records the id costs one request.
            if let Ok(discovery) = client.discover().await {
                state.machine_answered(
                    &discovery.id,
                    km_api::discover::display_name(&discovery.name).map(str::to_owned),
                );
            }
        }
        // **The two refusals are told apart, because the answers differ.** A machine that is not
        // answering is a machine to switch on; a password it refused is a password to retype.
        Err(crate::machine::Refused::Unreachable(_)) => {
            return door_says(&words.msg("door-unreachable"), "bad");
        }
        Err(error) => {
            tracing::debug!("the machine did not accept that password: {error}");
            return door_says(&words.msg("door-password-refused"), "bad");
        }
    }

    // **After the login and outside it**, so that two promises hold at once: a password the machine
    // refused is never written to this computer, and an unticked box forgets whichever password was
    // spent — including the remembered one, which is the case a blank box makes.
    if !typed.is_empty() {
        state.remember_password(form.remember.is_some().then_some(password));
    }

    Redirect::to("/admin/machine").into_response()
}

/// `POST /admin/connect/locale` — what language **this program's pages** are in.
///
/// # Why this program needs a picker of its own where the machine's `/admin/` does not
///
/// **Both surfaces read the `km_locale` cookie, and only one of them is on an origin that ever
/// gets one written.** On the machine the singer's remote is mounted at `/` and the owner's pages
/// at `/admin/`, so a viewer who chose Portuguese on the remote meets Portuguese on both — one
/// cookie, one origin. This program is a fourth program on a loopback port of its own, with no
/// remote beside it: nothing could write that cookie here, so its pages followed `Accept-Language`
/// and there was no way to disagree with the browser.
///
/// **On the door rather than on a tab**, because the door is this program's own page about this
/// program — which machine, which password — where every tab is a page about a machine. The
/// *Screen language* pane on the Machine tab is the other language and says so: that one is the
/// television's, in a room, and this one is this browser's.
///
/// **The redraw is the confirmation, and it is worded in the language just chosen** — the rule the
/// machine's own pane and the package builder's both keep, for the reason that somebody who picked
/// the wrong one finds out at once rather than by reading a sentence they cannot read.
///
/// **It reads no request locale**, alone among the handlers here, and that is the one thing worth
/// noticing about its signature: what a page in the old language would have said is never wanted,
/// because either the choice took — and the answer belongs in the new one — or nothing happened.
pub async fn set_page_locale(Form(form): Form<LocaleForm>) -> Response {
    let Some(chosen) = km_locale::Locale::parse(&form.locale) else {
        // Nothing changed and nothing to say. The control is a `<select>` of exactly the tags this
        // build has, so a toast here would be a sentence about a request no browser makes.
        return Redirect::to(crate::views::CONNECT_PAGE).into_response();
    };
    let mut response = door_says(
        &crate::words::messages(chosen).msg("door-locale-changed"),
        "good",
    );
    // **Set on the redirect rather than on the page it lands on**, so the door that draws is
    // already the first page in the new language. `km-locale` builds the value, because the remote
    // writes this same cookie and one spelling is what makes the two agree.
    if let Ok(value) = km_locale::set_cookie(chosen).parse() {
        response
            .headers_mut()
            .append(axum::http::header::SET_COOKIE, value);
    }
    response
}

/// What language this program's pages should be in.
#[derive(Debug, Clone, Deserialize)]
pub struct LocaleForm {
    /// The chosen BCP 47 tag.
    pub locale: String,
}

/// Back to one of this program's pages, carrying something to say.
///
/// # Why a refusal is a redirect rather than a status with a sentence in it
///
/// **Every control on these pages is an ordinary form, so the browser navigates.** A status with a
/// body becomes the whole document: an unstyled line of text with no chrome, no strip and no way
/// back, on a surface where the thing that would mend it is a tab away. The page is what a person
/// can act on, so the answer goes to a page.
///
/// The exception is a fragment htmx asked for, which keeps its status because `static/ui.js` is
/// what words those, and the thumbnail route, which answers an image slot. Both say so where they
/// are.
fn says(page: &str, kind: &str, said: &str) -> Response {
    Redirect::to(&format!("{page}?kind={kind}&said={}", urlencoding(said))).into_response()
}

/// Back to the door, carrying something to say.
fn door_says(said: &str, kind: &str) -> Response {
    says(crate::views::CONNECT_PAGE, kind, said)
}

/// Back to the Sound page, carrying a sentence from a key.
fn sound_says(locale: km_locale::Locale, kind: &str, key: &str) -> Response {
    says(
        crate::views::SOUND_PAGE,
        kind,
        &crate::words::messages(locale).msg(key),
    )
}

/// Back to the Pictures page, carrying a sentence from a key.
fn pictures_says(locale: km_locale::Locale, kind: &str, key: &str) -> Response {
    says(
        crate::views::PICTURES_PAGE,
        kind,
        &crate::words::messages(locale).msg(key),
    )
}

/// What a search that cannot start yet says about itself.
fn say_not_ready(why: crate::pictures::NotReady, locale: km_locale::Locale) -> String {
    let words = crate::words::messages(locale);
    match why {
        crate::pictures::NotReady::NoName => words.msg("pack-name-needed").into_owned(),
        crate::pictures::NotReady::NoTerms => words.msg("search-needs-terms").into_owned(),
        crate::pictures::NotReady::NeedsKey(provider) => words
            .msg_with("search-needs-key", &[("provider", provider.into())])
            .into_owned(),
    }
}

/// Percent-encodes what goes in the query string, in form encoding as `km-admin-pages` does.
fn urlencoding(text: &str) -> String {
    form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pack's verdict on being passed on is the license's, per image, and fails closed.
    ///
    /// **The two sources that need different answers are the whole point of this section.** A pack
    /// built from Openverse under CC0 may be handed on; one built from Pixabay may not, whatever it
    /// cost to make. The page marks which, so getting it backwards would say the opposite of what
    /// the source's own terms do.
    #[test]
    fn a_pixabay_pack_is_never_marked_as_one_that_may_be_passed_on() {
        use km_wallpaper_pack::config::ProviderKind;
        use km_wallpaper_pack::license::Redistribution;

        // Pixabay's images carry no per-image license — its parser leaves the field `None`
        // deliberately — so the answer comes from the site's own terms.
        let pixabay = ProviderKind::Pixabay
            .whole_site_license()
            .expect("pixabay states one license for the whole site");
        assert!(
            !matches!(pixabay.redistribution(), Redistribution::Granted),
            "a Pixabay pack is for the machine that built it"
        );

        let pexels = ProviderKind::Pexels
            .whole_site_license()
            .expect("pexels states one license for the whole site");
        assert!(!matches!(pexels.redistribution(), Redistribution::Granted));

        // Openverse states one per image, so it has no site-wide answer at all — which is the shape
        // that made the license a property of the image rather than of the source.
        assert!(ProviderKind::Openverse.whole_site_license().is_none());
    }

    #[test]
    fn a_banks_row_is_matched_to_the_machine_by_its_stem() {
        // The machine reports `GXSCC_gm_033` for a file the table calls `GXSCC_gm_033.sf2`. This
        // was found by sending a bank and watching the row still say it was not there.
        assert_eq!(stem_of("GXSCC_gm_033.sf2"), "GXSCC_gm_033");
        // Real bank filenames have dots in them, so only the last one is the extension.
        assert_eq!(stem_of("Roland SC-55 v3.7.sf2"), "Roland SC-55 v3.7");
        // Nothing to strip is not an error.
        assert_eq!(stem_of("Bundled"), "Bundled");
    }

    /// A row with nothing on it but the two things the ordering reads.
    fn bank(id: &'static str, here: bool) -> BankRow {
        BankRow {
            id,
            name: id,
            size: "",
            license: "",
            note: "",
            rank: None,
            recommended: false,
            fetchable: true,
            page: None,
            can_be_sent: true,
            here,
            on_the_machine: false,
            remove_confirm: String::new(),
        }
    }

    #[test]
    fn banks_already_on_this_computer_come_first() {
        let mut rows = vec![
            bank("first", false),
            bank("downloaded", true),
            bank("second", false),
            bank("also-downloaded", true),
        ];
        downloaded_first(&mut rows);
        let order: Vec<&str> = rows.iter().map(|row| row.id).collect();
        assert_eq!(
            order,
            ["downloaded", "also-downloaded", "first", "second"],
            "the two here float up, and neither group is otherwise disturbed"
        );
    }

    /// The stability is the whole reason this is a `sort_by_key` and not something cleverer.
    ///
    /// `km_banks::catalog()` is the nine ranked banks in rank order and then the rest by ascending
    /// loudness spread. Reordering inside a group would discard that, and it would do it silently —
    /// the page would still look sorted.
    #[test]
    fn catalog_order_survives_inside_each_group() {
        let mut rows = vec![bank("a", true), bank("b", true), bank("c", false)];
        downloaded_first(&mut rows);
        let order: Vec<&str> = rows.iter().map(|row| row.id).collect();
        assert_eq!(order, ["a", "b", "c"], "already in order, and left alone");

        // And with nothing downloaded at all, the table is exactly the catalog.
        let mut none = vec![bank("a", false), bank("b", false), bank("c", false)];
        downloaded_first(&mut none);
        let order: Vec<&str> = none.iter().map(|row| row.id).collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn the_pack_list_is_newest_first_and_holds_only_packs() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let packs = crate::pictures::packs_dir(home.path());

        for id in ["wallpapers-aaaaaaaa", "wallpapers-bbbbbbbb"] {
            let dir = packs.join(id);
            std::fs::create_dir_all(&dir).expect("a pack");
            std::fs::write(dir.join(format!("{id}.zip")), b"PK").expect("a zip");
            // Coarse filesystem timestamps make two writes in the same millisecond a coin toss, so
            // the order is set rather than raced for.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // A run killed between making the folder and moving the zip into it. It is not a pack, and
        // listing it would offer a Send that could only fail.
        std::fs::create_dir_all(packs.join("wallpapers-cccccccc")).expect("a half-written one");
        // And a file loose in the packs folder, which is not a pack either.
        std::fs::write(packs.join("stray.txt"), b"not a pack").expect("a stray");

        let rows = pack_rows(home.path(), km_locale::Locale::English);
        assert_eq!(rows.len(), 2, "only the two with a zip in them");
        assert_eq!(rows[0].name, "wallpapers-bbbbbbbb.zip", "newest first");
        assert_eq!(rows[1].name, "wallpapers-aaaaaaaa.zip");
        // No manifest traveled with either, and a pack is still worth sending without one.
        assert!(rows[0].images.is_none() && rows[0].built.is_none());
    }

    #[test]
    fn a_folder_that_is_not_a_pack_is_listed_by_nothing_and_sent_by_nothing() {
        let home = tempfile::tempdir().expect("a temporary folder");
        std::fs::create_dir_all(crate::pictures::packs_dir(home.path()).join("empty"))
            .expect("a folder");
        assert!(pack_rows(home.path(), km_locale::Locale::English).is_empty());
        assert!(pack_zip(home.path(), "empty").is_none());
    }
}
