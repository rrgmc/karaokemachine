//! Talking to the running karaoke machine.
//!
//! Four calls, all plain HTTP: play a file to hear whether it is any good, **send** one to a machine
//! that cannot see this box's disk, install a package that has just been built, and ask whether the
//! machine is there at all.
//!
//! **The first two are the same button, and which one it presses is decided by the address.** A
//! machine on loopback is handed a path: it is instant, it copies nothing, and a loose video song is
//! hundreds of megabytes. A machine anywhere else is handed the bytes, because the folder being
//! curated here does not exist over there and never will — which is what made the Play button a
//! same-computer feature until this existed. See `Test-playing to a machine that is not this one` in
//! `docs/decisions/curation.md`.
//!
//! The interesting part is the first refusal, and there are now two of them for the same reason.
//! `km-app` will not play a path outside `settings.debug.play_file_roots`, and it will not take an
//! upload at all unless `settings.debug.enabled` is on; **both are off in a shipped
//! configuration**, so the very first test-play on any machine comes back 400 either way. Handing
//! that through as an opaque error would cost somebody an afternoon, so [`Client::play_file`](crate::app::Client::play_file) and
//! [`Client::play_upload`](crate::app::Client::play_upload) each recognize their own and answer with the setting to change and where
//! the file holding it lives.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

/// The setting `km-app` checks before playing an arbitrary file.
const ROOTS_SETTING: &str = "debug.play_file_roots";

/// The setting `km-app` checks before taking a song sent in the request body.
const UPLOADS_SETTING: &str = "debug.enabled";

/// The form field naming what both halves of an uploaded song are staged under.
///
/// Taken from `km-api` rather than spelled again, which is the whole reason this tool depends on
/// that crate at all — the same arrangement as `discover::browse` taking the service name from the
/// crate that advertises it.
const STEM_FIELD: &str = km_api::handlers::STEM_FIELD;

/// The form field carrying the corrections an uploaded song is to play with.
///
/// Taken from `km-api` for [`STEM_FIELD`]'s reason.
const FIXES_FIELD: &str = km_api::handlers::FIXES_FIELD;

/// The form field carrying the title an uploaded song is to be shown under.
const TITLE_FIELD: &str = km_api::handlers::TITLE_FIELD;

/// The form field carrying its performer.
const ARTIST_FIELD: &str = km_api::handlers::ARTIST_FIELD;

/// The form field carrying an UltraStar or LRC song's words, beside its MP3.
const LYRICS_FIELD: &str = km_api::handlers::LYRICS_FIELD;

/// The form field naming which of the two files the words were read from.
const LYRICS_KIND_FIELD: &str = km_api::handlers::LYRICS_KIND_FIELD;

/// The form field carrying the key it is to play in.
const TRANSPOSE_FIELD: &str = km_api::handlers::TRANSPOSE_FIELD;

/// The form field carrying its melody channel, or `none`.
const MELODY_FIELD: &str = km_api::handlers::MELODY_FIELD;

/// The form field the owner's three uploads carry their file in.
///
/// Taken from `km-api` for [`STEM_FIELD`]'s reason. One name for three routes over there, because
/// each takes exactly one file and the route says what kind it is.
const PACKAGE_FIELD: &str = km_api::handlers::FILE_FIELD;

/// How long an upload may take.
///
/// Minutes rather than [`Client::new`]'s five seconds, and the two numbers are answering different
/// questions. That one is "how long may a page wait for a machine that may not be there"; this is
/// "how long does 300 MB take over a house's Wi-Fi", and the answer is not five seconds. Applied to
/// the one request rather than to a second client, so the comment on the client's own timeout goes
/// on being true of every other call.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// What went wrong reaching the machine.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The machine could not be reached at all.
    #[error("the karaoke machine is not answering at {url} ({source})")]
    Unreachable {
        /// The base URL that was tried.
        url: String,
        /// The transport failure.
        source: reqwest::Error,
    },
    /// The machine answered, and said no.
    #[error("{0}")]
    Refused(String),
    /// The machine answered with something unexpected.
    #[error("the karaoke machine answered {status}: {body}")]
    Unexpected {
        /// HTTP status.
        status: u16,
        /// Whatever it said.
        body: String,
    },
}

impl AppError {
    /// What a page says about this, in the language it is being drawn in.
    ///
    /// `Display` stays English, being what the log carries. [`Self::Refused`] is the machine's own
    /// answer and is shown as it arrived — a refusal another process worded, which this tool has no
    /// key for.
    pub fn say(&self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Unreachable { url, source } => words
                .msg_with(
                    "app-error-unreachable",
                    &[
                        ("url", url.as_str().into()),
                        ("why", source.to_string().as_str().into()),
                    ],
                )
                .into_owned(),
            Self::Refused(said) => said.clone(),
            Self::Unexpected { status, body } => words
                .msg_with(
                    "app-error-unexpected",
                    &[
                        ("status", i64::from(*status).into()),
                        ("body", body.as_str().into()),
                    ],
                )
                .into_owned(),
        }
    }
}

/// A client for one karaoke machine.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    /// The admin token, once somebody has typed the machine's password.
    ///
    /// **In memory and never written down**, which is the same policy `km-admin`'s own client keeps
    /// and for the same reason: a bearer token in a file is a credential this tool would be
    /// responsible for and has nowhere good to put. That is still true of the *token* now that the
    /// *password* can be remembered — a remembered password is exchanged for a fresh token at the
    /// moment one is needed, and `crate::passwords` is the only thing here that touches a disk.
    ///
    /// It exists because installing a package became an admin action. That route was public for as
    /// long as this tool had nowhere to keep a password — it names a path the caller must already
    /// be able to write to, which was the argument — and closing it without giving the tool a
    /// password would have left the curation tool unable to send anything at all.
    ///
    /// **The `Arc` is shared with `State` rather than owned here**, because this tool builds a fresh
    /// client for every request: a token owned by the client would be thrown away between the form
    /// that obtained it and the button that needs it. See [`Client::sharing_token`].
    token: Arc<Mutex<Option<String>>>,
}

/// The machine's answer to `GET /api/v1/discover`.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    /// The machine's name.
    #[serde(default)]
    pub name: String,
    /// Its instance id — the identity this tool anchors on, so a machine that changes address is
    /// still recognized as the one somebody chose. See `State::machine_answered`.
    pub id: String,
    /// How many songs it has installed, when it says.
    #[serde(default)]
    pub song_count: Option<u32>,
    /// Whether the machine is in debugging mode, which is what mounts the two play routes.
    ///
    /// See [`Client::play_upload`](crate::app::Client::play_upload) for why this is asked before
    /// anything is sent.
    pub debug_enabled: bool,
}

/// The error body every `km-api` failure carries.
#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: String,
}

impl Client {
    /// A client pointed at a base URL such as `http://127.0.0.1:8177`.
    pub fn new(base: &str) -> Self {
        Self {
            // Short, because every one of these happens while somebody waits for a page. A machine
            // that has gone away should say so in a moment, not in thirty seconds.
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            base: base.trim_end_matches('/').to_owned(),
            token: Arc::new(Mutex::new(None)),
        }
    }

    /// The same client, holding a token slot somebody else owns.
    ///
    /// **This is what makes a login outlive the request that performed it.** `State::app_client`
    /// builds a client per request — it has to, because the address it should point at is a read of
    /// the database and of what the network is saying — so a token kept inside the client would be
    /// dropped between the Settings form that obtained it and the Install button that needs it. The
    /// slot belongs to `State`; every client of this run reads and writes the one lock.
    pub fn sharing_token(base: &str, token: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            token,
            ..Self::new(base)
        }
    }

    /// The base URL this client uses.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Exchanges the machine's admin password for a token, and keeps it for this run.
    ///
    /// **Try, and ask for the password only if refused** is the shape both admin tools take, and it
    /// has one fewer step than it used to: every machine has a password now, so the first attempt
    /// at an admin route always *will* be refused. What the shape still buys is that nothing asks
    /// for a credential before something needs one — browsing a machine's catalog never does.
    pub async fn log_in(&self, password: &str) -> Result<(), AppError> {
        let url = format!("{}/api/v1/admin/login", self.base);
        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "password": password }))
            .send()
            .await
            .map_err(|source| AppError::Unreachable {
                url: self.base.clone(),
                source,
            })?;

        #[derive(Deserialize)]
        struct Grant {
            token: String,
        }
        let grant: Grant = self.decode(response).await?;
        if let Ok(mut held) = self.token.lock() {
            *held = Some(grant.token);
        }
        Ok(())
    }

    /// Whether this client is holding a token.
    pub fn logged_in(&self) -> bool {
        self.token.lock().is_ok_and(|held| held.is_some())
    }

    /// Forgets the token — what a 401 means, and what choosing another machine means.
    pub fn log_out(&self) {
        if let Ok(mut held) = self.token.lock() {
            *held = None;
        }
    }

    /// Attaches the token to a request, when there is one.
    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.token.lock().ok().and_then(|held| held.clone()) {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    /// Turns debugging mode on or off on the machine.
    ///
    /// **The switch that makes the Play button usable against a shipped machine.** `play_file` and
    /// `play_upload` are not mounted at all while debugging is off — a 404, not a refusal — so a
    /// curator auditioning songs on a real machine has to turn it on, and this is where from.
    pub async fn set_debugging(&self, enabled: bool) -> Result<(), AppError> {
        let url = format!("{}/api/v1/admin/debug", self.base);
        let response = self
            .authorized(self.http.put(&url))
            .json(&serde_json::json!({ "enabled": enabled }))
            .send()
            .await
            .map_err(|source| AppError::Unreachable {
                url: self.base.clone(),
                source,
            })?;
        self.decode::<serde_json::Value>(response).await.map(drop)
    }

    /// Whether the machine is there, and what it calls itself.
    pub async fn discover(&self) -> Result<Discovery, AppError> {
        let url = format!("{}/api/v1/discover", self.base);
        let response =
            self.http
                .get(&url)
                .send()
                .await
                .map_err(|source| AppError::Unreachable {
                    url: self.base.clone(),
                    source,
                })?;
        self.decode(response).await
    }

    /// Asks the machine to play a file straight from disk.
    ///
    /// `path` must be absolute: the machine resolves it against *its own* working directory, which is
    /// not this tool's.
    ///
    /// `curated_root` is the folder this tool is curating. It is here only for the refusal message:
    /// when the machine says the path is not permitted, the folder worth naming is the one that
    /// permits *every* song this tool can offer, not the one this particular file happens to sit in.
    /// `decided` is what a curator has settled about this song, each field absent when they have
    /// not. Without it a preview answers a question nobody asked: the corrections being auditioned
    /// are in no package yet, and the title somebody retyped is in the database and nowhere in the
    /// bytes, so a machine left to work it out plays the song as it was found.
    pub async fn play_file(
        &self,
        path: &Path,
        curated_root: &Path,
        decided: &km_api::Audition<'_>,
    ) -> Result<(), AppError> {
        let url = format!("{}/api/v1/debug/play-file", self.base);
        let mut body = serde_json::json!({ "path": path.display().to_string() });
        if let Some(object) = body.as_object_mut() {
            if let Some(fixes) = decided.fixes {
                object.insert("fixes".to_owned(), serde_json::json!(fixes));
            }
            if let Some(title) = decided.title {
                object.insert("title".to_owned(), serde_json::json!(title));
            }
            if let Some(artist) = decided.artist {
                object.insert("artist".to_owned(), serde_json::json!(artist));
            }
            if let Some(transpose) = decided.transpose {
                object.insert("transpose".to_owned(), serde_json::json!(transpose));
            }
            // `null` is a decision here, that the song has no melody channel.
            if let Some(melody) = decided.melody {
                object.insert("melody".to_owned(), serde_json::json!(melody));
            }
            if let Some(lyrics) = decided.lyrics {
                object.insert("lyrics".to_owned(), serde_json::json!(lyrics));
            }
            if let Some(kind) = decided.lyrics_kind {
                object.insert("lyrics_kind".to_owned(), serde_json::json!(kind));
            }
        }
        let response = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|source| AppError::Unreachable {
                url: self.base.clone(),
                source,
            })?;

        match self.decode::<serde_json::Value>(response).await {
            Ok(_) => Ok(()),
            Err(AppError::Refused(message)) if message.contains(ROOTS_SETTING) => Err(
                AppError::Refused(explain_roots(path, curated_root, &message)),
            ),
            Err(other) => Err(other),
        }
    }

    /// Whether this machine is on the box the tool is running on.
    ///
    /// **The one question that decides which of the two play routes a song takes**, and it is asked
    /// of the address rather than offered as a setting: a path works on loopback and cannot work
    /// anywhere else, so there is a right answer and nothing for anybody to choose between.
    ///
    /// An address that will not parse counts as remote. That is the safe direction of the two — an
    /// upload to a machine on this box works and merely copies a file that did not need copying,
    /// where a path to a machine that is not on this box does not work at all.
    pub fn is_loopback(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base) else {
            return false;
        };
        let Some(host) = url.host_str() else {
            return false;
        };
        // `host_str` keeps the brackets an IPv6 authority is written with, and `IpAddr` will not
        // parse them.
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            // The whole of `127.0.0.0/8` and `::1`, not only the two spellings anybody types.
            return ip.is_loopback();
        }
        // A trailing dot is a fully-qualified spelling of the same name. Nothing longer is:
        // `localhost.example.com` and `127.0.0.1.nip.io` are somebody else's machine.
        host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case("localhost.")
    }

    /// Sends a song to the machine and asks it to play what arrived.
    ///
    /// [`Self::play_file`]'s answer for a machine that is not on this box. `partner` is the other
    /// half of an MP3+G song and `None` for everything else; both halves go in one request, staged
    /// under one `stem`, because that is what lets the machine find the second beside the first.
    ///
    /// `stem` is sent as a form field rather than left to the parts' filenames, and the machine
    /// insists on it: a multipart filename is a header, so its encoding across the wire is not
    /// something either end can rely on, and a corpus is full of accented names.
    ///
    /// **It asks whether the machine takes uploads before it sends one**, and that is not politeness
    /// or an optimization. A machine ships refusing them, so the common case is a refusal — and a
    /// server that answers mid-request and closes leaves this side looking like a connection that
    /// dropped, so the refusal a curator most needs to read arrives as "the machine is not
    /// answering", if it arrives at all. It was a *flaky test* before it was understood; with a
    /// video rather than a four-byte fixture it would have been the normal outcome. One cheap
    /// question first, and the answer is a sentence somebody can act on. The upload of a song that
    /// *will* be taken is unaffected.
    ///
    /// `decided` carries the meaning it has on [`Self::play_file`].
    pub async fn play_upload(
        &self,
        path: &Path,
        partner: Option<&Path>,
        stem: &str,
        decided: &km_api::Audition<'_>,
    ) -> Result<(), AppError> {
        if !self.discover().await?.debug_enabled {
            return Err(AppError::Refused(explain_uploads(
                "this machine is not in debugging mode, so it takes no uploaded songs",
            )));
        }

        let url = format!("{}/api/v1/debug/play-upload", self.base);
        // The stem first, because the machine will not open a file until it has the name to give it
        // — which is what saves it from buffering a video while it waits to find out.
        let mut form = reqwest::multipart::Form::new().text(STEM_FIELD, stem.to_owned());
        if let Some(fixes) = decided.fixes {
            form = form.text(FIXES_FIELD, crate::fixes::encode(fixes));
        }
        if let Some(title) = decided.title {
            form = form.text(TITLE_FIELD, title.to_owned());
        }
        if let Some(artist) = decided.artist {
            form = form.text(ARTIST_FIELD, artist.to_owned());
        }
        if let Some(transpose) = decided.transpose {
            form = form.text(TRANSPOSE_FIELD, transpose.to_string());
        }
        if let Some(melody) = decided.melody {
            form = form.text(
                MELODY_FIELD,
                melody.map_or_else(|| "none".to_owned(), |channel| channel.to_string()),
            );
        }
        if let Some(lyrics) = decided.lyrics {
            let text = serde_json::to_string(lyrics).map_err(|error| {
                AppError::Refused(format!("the words would not encode: {error}"))
            })?;
            form = form.text(LYRICS_FIELD, text);
        }
        if let Some(kind) = decided.lyrics_kind {
            form = form.text(LYRICS_KIND_FIELD, kind.as_str());
        }
        form = form.part("primary", file_part(path).await?);
        if let Some(partner) = partner {
            form = form.part("partner", file_part(partner).await?);
        }

        let response = self
            .http
            .post(&url)
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form)
            .send()
            .await
            .map_err(|source| AppError::Unreachable {
                url: self.base.clone(),
                source,
            })?;

        match self.decode::<serde_json::Value>(response).await {
            Ok(_) => Ok(()),
            Err(AppError::Refused(message)) if message.contains(UPLOADS_SETTING) => {
                Err(AppError::Refused(explain_uploads(&message)))
            }
            Err(other) => Err(other),
        }
    }

    /// Installs a built package into the machine's catalog, by naming the file on its disk.
    ///
    /// **This only works where the machine shares this filesystem**, which is what
    /// [`Self::is_loopback`] is asked before it is called; [`Self::upload_package`] is the answer for
    /// everywhere else. The machine opens the path itself, so a path that means nothing over there
    /// comes back as `No such file or directory (os error 2)` — a message about *its* disk that
    /// reads, on this screen, as though the file this tool had just written were missing.
    ///
    /// **Decoded into the machine's own DTO rather than into a `serde_json::Value`.** `Display` for
    /// a `Value` is the raw JSON, so the message beside the button would read
    /// `Installed into http://…. {"package_id":"favtest1",…}`.
    /// The DTO is `km-api`'s and this crate already depends on that crate for [`STEM_FIELD`], so
    /// there is nothing to add — and [`km_api::dto::InstallReportDto::sentence`] is where the
    /// wording lives, so a path install and an upload say the same thing.
    ///
    /// **And a refusal is explained here as it is there.** This route is admin too, so a curator who
    /// has not signed in gets a 401 — and for a while only [`Self::upload_package`] turned that into
    /// instructions, so the same missing password read as three sentences about what to do on a
    /// machine somewhere else and as `'/api/v1/admin/packages' needs the admin password; send an
    /// admin token` on this one. Same door, same sentence.
    pub async fn install_package(
        &self,
        path: &Path,
    ) -> Result<km_api::dto::InstallReportDto, AppError> {
        if !self.logged_in() {
            return Err(AppError::Refused(explain_upload_refusal(
                &self.base,
                "installing a package needs this machine's admin password",
            )));
        }
        let url = format!("{}/api/v1/admin/packages", self.base);
        let response = self
            .authorized(self.http.post(&url))
            .json(&serde_json::json!({ "path": path.display().to_string() }))
            .send()
            .await
            .map_err(|source| AppError::Unreachable {
                url: self.base.clone(),
                source,
            })?;

        let status = response.status();
        self.decode(response).await.map_err(|error| match error {
            AppError::Refused(message) if status.as_u16() == 401 || status.as_u16() == 403 => {
                AppError::Refused(explain_upload_refusal(&self.base, &message))
            }
            other => other,
        })
    }

    /// Sends a built package to the machine and asks it to install what arrived.
    ///
    /// [`Self::install_package`]'s answer for a machine that is not on this box, and the same split
    /// [`Self::play_file`] and [`Self::play_upload`] already make for a song — decided by the address
    /// rather than offered as a setting, because a path works on loopback and cannot work anywhere
    /// else.
    ///
    /// **The token is checked before a byte is sent, and that is not a nicety.** This route is admin
    /// now, so a caller with no token gets a 401 — and a server that answers mid-request and closes
    /// leaves the sending half looking like a dropped connection, so the refusal a curator most
    /// needs to read is the one least likely to arrive. A 300 MB package never wins that race.
    /// [`Self::play_upload`] asks `/discover` for the same reason; here the answer is already known,
    /// because the token is this client's own.
    ///
    /// It does not cover a token that has *expired* — that still fails on the wire — but that is the
    /// rare case, and the common one is somebody who has not typed the password yet.
    ///
    /// Answers a sentence rather than a body: this route's DTO is `{"report": "installed …"}`,
    /// worded by the machine because only the machine knows what it did with the bytes. See
    /// [`Self::install_package`] for why neither of these decodes into a `serde_json::Value` any
    /// more.
    pub async fn upload_package(&self, path: &Path) -> Result<String, AppError> {
        if !self.logged_in() {
            return Err(AppError::Refused(explain_upload_refusal(
                &self.base,
                "sending a package needs this machine's admin password",
            )));
        }
        // **The path is `km-api`'s to state, not this tool's to spell.** It is right here and was
        // wrong in `km-admin`, which hand-wrote all three of these and reached nothing for as long
        // as it had the feature — so the literal goes even though this copy of it was correct.
        let url = format!(
            "{}{}{}",
            self.base,
            km_api::routes::API_PREFIX,
            km_api::uploads::path_for(km_api::machine::Upload::Package)
        );
        let form = reqwest::multipart::Form::new().part(PACKAGE_FIELD, file_part(path).await?);

        let response = self
            .authorized(self.http.post(&url))
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form)
            .send()
            .await
            .map_err(|source| AppError::Unreachable {
                url: self.base.clone(),
                source,
            })?;

        let status = response.status();
        self.decode::<km_api::dto::UploadReportDto>(response)
            .await
            .map(|report| report.report)
            .map_err(|error| match error {
                AppError::Refused(message) if status.as_u16() == 401 || status.as_u16() == 403 => {
                    AppError::Refused(explain_upload_refusal(&self.base, &message))
                }
                other => other,
            })
    }

    /// Reads a response, turning the machine's own error shape into [`AppError::Refused`].
    async fn decode<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, AppError> {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if status.is_success() {
            return serde_json::from_str(&body).map_err(|_| AppError::Unexpected {
                status: status.as_u16(),
                body: truncate(&body),
            });
        }

        // Every km-api failure carries a stable `error` code and a `message`. Preferring the message
        // means the person reads what the machine actually said rather than a status number.
        if let Ok(api) = serde_json::from_str::<ApiError>(&body)
            && !api.message.is_empty()
        {
            return Err(AppError::Refused(api.message));
        }
        if let Ok(api) = serde_json::from_str::<ApiError>(&body)
            && !api.error.is_empty()
        {
            return Err(AppError::Refused(api.error));
        }
        Err(AppError::Unexpected {
            status: status.as_u16(),
            body: truncate(&body),
        })
    }
}

/// One half of an uploaded song, streamed from disk rather than read into memory.
///
/// `Part::file` opens it, reads its length and sends that as a `Content-Length`, so the machine can
/// weigh the request against its own limit before taking a byte of it — which is the difference
/// between a 300 MB video being refused in a moment and being refused at the end.
///
/// The filename it attaches is **not** what the file will be called over there. The machine names
/// both halves from the `stem` field and reads a part's filename only for its extension; this is set
/// so the request is a well-formed file part at all.
async fn file_part(path: &Path) -> Result<reqwest::multipart::Part, AppError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("song")
        .to_owned();
    reqwest::multipart::Part::file(path)
        .await
        .map(|part| part.file_name(name))
        .map_err(|error| AppError::Refused(format!("could not read {}: {error}", path.display())))
}

/// Turns the machine's refusal of an *upload* into instructions.
///
/// The twin of [`explain_roots`] below, for the setting that gates the other play route, and it
/// exists for the same reason and repeats the same trap: `debug.accept_uploads` is the setting's
/// name and what the machine's own message calls it, but a `settings.json` key spelled that way is
/// ignored — the key is `accept_uploads`, inside a `debug` object.
///
/// **This one names no folder**, which is the whole difference between the two routes: there is no
/// path to permit, because the song is sent rather than found. That also makes it a single setting
/// somebody turns on once per machine rather than a list they add to.
///
/// **Two ways, and the settings file is the first of them.** The checkbox on the machine's `/dev/`
/// page takes effect at once and is remembered, but that page is off unless somebody asks for it,
/// so reaching the checkbox is itself a thing to explain — hence `--dev-remote`, named here because
/// a curator hitting this message is exactly who that flag is for.
///
/// The settings file is one edit against two, it does not involve serving a console at all, and it
/// is the only way to set this *before* the machine starts. On an Android box or an appliance under a television both routes mean `adb` or
/// `ssh`; there is no spelling of this that is easy there.
pub fn explain_uploads(original: &str) -> String {
    format!(
        "{original}\n\nThis karaoke machine is not on this computer, so the song has to be sent to \
         it rather than named — and the routes that take one are only mounted while the machine is \
         in debugging mode.\n\nThe quickest way is the Debugging button on the machine panel here, \
         which needs that machine's admin password. It takes effect when that machine next \
         restarts, whichever of these three ways turns it on.\n\nOr put this in the debug object of its \
         settings.json:\n\n    \"debug\": {{\n      \"enabled\": true\n    }}\n\nRun \
         karaokemachine --show-paths on that machine to find the file, and restart the app \
         afterwards.\n\nOr open http://<that machine>:8177/admin/, sign in, and turn Debugging on \
         from the This machine tab."
    )
}

/// Turns a locked door into instructions, for a door this tool can now open.
///
/// **Both send routes ship admin, and this tool can hold a password**, which is what changed: for a
/// long time `packages.install` was deliberately public *because* the builder had nowhere to put a
/// token, and that was the whole of the argument for the split. It has somewhere now — the Password
/// box on the machine panel — so the first way out this names is a control on the page somebody is
/// already looking at.
///
/// The other two are for somebody who does not know the password or does not want to type it here,
/// and the last is the honest one: copy the file across and install it by path.
///
/// **Written as plain prose, deliberately.** This lands in a `.message`, which is `pre-wrap` and
/// escaped — so a `**bold**` renders as four asterisks and a backtick renders as a backtick. Blank
/// lines are the only formatting this string has.
pub fn explain_upload_refusal(base: &str, original: &str) -> String {
    format!(
        "{original}\n\nSending a package to a machine, and installing one it can already see, are \
         both admin actions — so this tool needs that machine's password before either will \
         work.\n\nType it into the Password box on the machine panel here, under Settings. The tool \
         exchanges it for a token that lives in memory until it stops; the password itself is \
         written down only if you tick the box beside it.\n\nTwo other ways, if you would rather \
         not. Open {base}/admin/ on that machine and add the package there, which is the same \
         upload with somebody logged in. Or copy the .kmpkg onto the machine yourself and install \
         it by path — karaokemachine --show-paths names its packages folder, and anything dropped \
         in there is picked up by Rescan."
    )
}

/// Turns the machine's refusal into instructions.
///
/// The machine is right to refuse — an unrestricted version of that endpoint reads anything on its
/// disk — so the answer is not to work around it but to say exactly what to add and where.
///
/// The snippet shows the **nested** shape the file actually uses. `debug.play_file_roots` is the
/// setting's name and is what the machine's own message calls it, but a `settings.json` key spelled
/// that way is simply ignored: the key is `play_file_roots`, inside a `debug` object. Printing the
/// dotted form as if it were the key would send somebody to edit the file and find nothing changed.
///
/// **The folder named is the curated root, not the file's own parent.** A corpus is browsed from its
/// top and its files are scattered thousands of folders deep; naming the parent answers only for the
/// song that happened to be clicked, and the next one refuses again from a different folder. One
/// entry at the root permits everything this tool can offer, which is the setting somebody actually
/// wants. The parent is used only when the file is somehow not under the root at all.
pub fn explain_roots(path: &Path, curated_root: &Path, original: &str) -> String {
    let parent = path
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let root = curated_root.display().to_string();
    let folder = if !root.is_empty() && parent.starts_with(&root) {
        root
    } else {
        parent
    };
    let escaped = escape_for_json(&folder);
    format!(
        "{original}\n\nThe karaoke app only plays files inside the folders named in \
         {ROOTS_SETTING}, and that list is empty until somebody fills it in. Put the folder being \
         curated in the debug object of its settings.json — one entry covers every song here:\
         \n\n    \"debug\": {{\n      \"play_file_roots\": [\"{escaped}\"]\n    }}\n\nRun \
         karaokemachine --show-paths to find that file, and restart the app afterwards."
    )
}

/// Doubles the backslashes in a path so it can go inside the JSON snippet.
///
/// JSON has no raw strings, so a Windows separator has to be doubled or the file somebody pastes it
/// into is invalid. Its own function because it is *string* work with nothing platform-specific about
/// it — which is what lets it be tested with a Windows path on a Linux runner, where `Path` would not
/// read one as a path at all.
fn escape_for_json(folder: &str) -> String {
    folder.replace('\\', "\\\\")
}

/// Keeps an unexpected body short enough to put on a page.
fn truncate(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.len() <= 300 {
        return trimmed.to_owned();
    }
    let mut cut = 300;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &trimmed[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_on_the_base_url_is_harmless() {
        assert_eq!(Client::new("http://x:8177/").base(), "http://x:8177");
        assert_eq!(Client::new("http://x:8177").base(), "http://x:8177");
    }

    /// The question that picks the play route, so every one of these is a real behavior.
    #[test]
    fn a_machine_on_this_box_is_told_from_one_that_is_not() {
        for here in [
            "http://127.0.0.1:8177",
            "http://127.0.0.1:8177/",
            // The whole of `127.0.0.0/8`, not only the address anybody types.
            "http://127.42.0.9:8177",
            "http://localhost:8177",
            "http://LOCALHOST:8177",
            // A fully-qualified spelling of the same name.
            "http://localhost.:8177",
            "http://[::1]:8177",
        ] {
            assert!(Client::new(here).is_loopback(), "{here} is this box");
        }

        for elsewhere in [
            "http://192.168.1.50:8177",
            // A bare hostname, which resolves to somebody else's box on the LAN.
            "http://media-box:8177",
            // The two that look like loopback and are somebody else's machine. A prefix match or a
            // `contains` would get both of these wrong.
            "http://localhost.example.com:8177",
            "http://127.0.0.1.nip.io:8177",
            "http://[fe80::1]:8177",
        ] {
            assert!(
                !Client::new(elsewhere).is_loopback(),
                "{elsewhere} is not this box"
            );
        }
    }

    /// An address that will not parse counts as remote, which is the safe direction: an upload to a
    /// machine on this box merely copies a file needlessly, where a path to one that is not does
    /// not work at all.
    #[test]
    fn an_unparseable_address_is_treated_as_somewhere_else() {
        assert!(!Client::new("not a url").is_loopback());
        assert!(!Client::new("").is_loopback());
    }

    #[test]
    fn the_debug_refusal_names_the_setting_and_the_nested_key() {
        let explained = explain_uploads(
            "this machine is not in debugging mode; set debug.enabled to permit it",
        );
        assert!(explained.contains(UPLOADS_SETTING));
        assert!(explained.contains("--show-paths"));
        // The same trap `explain_roots` was written around: the dotted form is the setting's name,
        // and a key spelled that way in settings.json is silently ignored.
        assert!(explained.contains("\"debug\": {"), "{explained}");
        assert!(explained.contains("\"enabled\": true"), "{explained}");
        assert!(
            !explained.contains("\"debug.enabled\":"),
            "the dotted form is the setting's name, not a key that exists in the file"
        );
        // The machine's own words are kept, not only this gloss on them.
        assert!(explained.contains("not in debugging mode"));
        // **The three ways through, nearest first**, and the dev remote is not among them: the
        // switch it carries is on the owner's page, which every machine serves and which needs no
        // flag to reach. Telling somebody to open a page a default machine does not serve is the
        // failure this assertion is here to catch.
        assert!(explained.contains("Debugging"), "{explained}");
        assert!(explained.contains("/admin/"), "{explained}");
        assert!(
            !explained.contains("--dev-remote"),
            "the dev remote is no longer where this switch lives: {explained}"
        );
    }

    /// **None of the three is markdown, and nothing renders them as if they were.**
    ///
    /// They land in a `.message`, which `message.html` escapes and `style.css` gives `pre-wrap` —
    /// so a blank line is a paragraph and everything else is literal. `**Password**` was on the
    /// screen as four asterisks and a word for as long as that string existed, next to a backticked
    /// `karaokemachine --show-paths` that read as though the backticks were part of the command.
    /// The JSON snippets are indentation, which survives `pre-wrap` intact and is the one piece of
    /// shape these messages are allowed.
    #[test]
    fn no_refusal_is_written_in_markdown() {
        let refusals = [
            explain_uploads("this machine is not in debugging mode"),
            explain_upload_refusal(
                "http://192.168.1.42:8177",
                "sending a package needs a password",
            ),
            explain_roots(
                Path::new("D:/tunes/karaoke/song.kar"),
                Path::new("D:/tunes/karaoke"),
                "not inside an allowed folder",
            ),
        ];
        for refusal in refusals {
            // Not `_`: `play_file_roots` is a setting's real name and the JSON snippet has to
            // spell it.
            for marker in ['*', '`'] {
                assert!(
                    !refusal.contains(marker),
                    "{marker} renders literally in a .message: {refusal}"
                );
            }
        }
    }

    #[test]
    fn the_roots_refusal_names_the_setting_the_folder_and_the_file() {
        let explained = explain_roots(
            Path::new("D:/tunes/karaoke/song.kar"),
            Path::new("D:/tunes/karaoke"),
            "D:/tunes/karaoke/song.kar is not inside an allowed folder; set debug.play_file_roots to permit it",
        );
        assert!(explained.contains(ROOTS_SETTING));
        assert!(explained.contains("D:/tunes/karaoke"));
        assert!(explained.contains("--show-paths"));
        // The snippet must be the shape the file really has: a `play_file_roots` key inside a
        // `debug` object, not one key literally named `debug.play_file_roots`.
        assert!(explained.contains("\"debug\": {"), "{explained}");
        assert!(explained.contains("\"play_file_roots\": ["), "{explained}");
        assert!(
            !explained.contains("\"debug.play_file_roots\":"),
            "the dotted form is the setting's name, not a key that exists in the file"
        );
        // The original refusal is kept: the person should see what the machine said, not only our
        // gloss on it.
        assert!(explained.contains("is not inside an allowed folder"));
    }

    /// A Windows path, escaped, without asking `Path` to read one.
    ///
    /// These three tests used to hand `Path::new(r"D:\tunes\karaoke\song.kar")` to `explain_roots` and
    /// assert on the result. On Windows that works; everywhere else `\` is an ordinary character, so
    /// the whole string is one component, `parent()` is the empty path, and the snippet names no
    /// folder at all. They passed on the machine the strings were written for and failed in CI on
    /// Linux and macOS. The escaping is plain string work and is tested as such; **which** folder is
    /// named is tested below with paths in the shape this platform actually has.
    #[test]
    fn a_backslash_is_doubled_so_the_snippet_is_valid_json() {
        assert_eq!(
            escape_for_json(r"D:\tunes\karaoke"),
            r"D:\\tunes\\karaoke",
            "a settings.json carrying a single backslash will not parse"
        );
        assert_eq!(
            escape_for_json("/tunes/karaoke"),
            "/tunes/karaoke",
            "and a path with none is left exactly as it is"
        );
    }

    /// A corpus root and a song several folders under it, in this platform's own path syntax.
    #[cfg(windows)]
    const ROOT: &str = r"D:\tunes\karaoke";
    #[cfg(windows)]
    const DEEP: &str = r"D:\tunes\karaoke\albums\live\Artist-Song.mid";
    #[cfg(windows)]
    const LEAF: &str = r"albums\\live";
    #[cfg(windows)]
    const OUTSIDE: &str = r"D:\elsewhere\odd.kar";
    #[cfg(windows)]
    const OUTSIDE_FOLDER: &str = r"D:\\elsewhere";

    #[cfg(not(windows))]
    const ROOT: &str = "/tunes/karaoke";
    #[cfg(not(windows))]
    const DEEP: &str = "/tunes/karaoke/albums/live/Artist-Song.mid";
    #[cfg(not(windows))]
    const LEAF: &str = "albums/live";
    #[cfg(not(windows))]
    const OUTSIDE: &str = "/elsewhere/odd.kar";
    #[cfg(not(windows))]
    const OUTSIDE_FOLDER: &str = "/elsewhere";

    /// The folder to allow is the corpus root, not the folder the clicked song happens to sit in.
    ///
    /// Naming the parent, over a corpus browsed from its top, sends somebody to add a new root for
    /// every subfolder they test-play from — a setting they would be editing all afternoon instead
    /// of once.
    #[test]
    fn the_folder_offered_is_the_curated_root_not_the_songs_own_folder() {
        let explained = explain_roots(Path::new(DEEP), Path::new(ROOT), "refused");
        assert!(
            explained.contains(&format!("[\"{}\"]", escape_for_json(ROOT))),
            "the root is the folder worth allowing: {explained}"
        );
        assert!(
            !explained.contains(LEAF),
            "naming the leaf folder answers for one song only: {explained}"
        );
    }

    /// A file somehow outside the curated folder still gets usable advice rather than a root that
    /// would not permit it.
    #[test]
    fn a_file_outside_the_root_falls_back_to_its_own_folder() {
        let explained = explain_roots(Path::new(OUTSIDE), Path::new(ROOT), "refused");
        assert!(
            explained.contains(&format!("[\"{OUTSIDE_FOLDER}\"]")),
            "{explained}"
        );
    }

    #[test]
    fn a_long_body_is_cut_without_splitting_a_character() {
        let body = "á".repeat(400);
        let short = truncate(&body);
        assert!(short.len() <= 303, "{}", short.len());
        assert!(short.ends_with('…'));
    }
}
