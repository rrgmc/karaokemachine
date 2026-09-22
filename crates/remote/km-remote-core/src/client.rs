//! Talking to a karaoke machine.
//!
//! Two halves. [`Api`] is plain request/response over HTTP — search is not here, because search is
//! answered from the mirror, so what is left is the queue, the transport, the settings, and the two
//! calls the catalog import needs. [`MachineClient`] wraps it as the remote's `Machine` trait and
//! adds the part a browser cannot do for itself: **one long-lived WebSocket to the machine's event
//! stream**, re-broadcast to every open page as SSE. See `km_remote_pages::sse` for why a page cannot hold
//! that connection itself.
//!
//! **The machine being away is the normal case, not a fault.** The offline app spends most of its
//! life with the television off. So the connection loop reconnects on its own with a backoff, the
//! pages say so through the banner, and a command sent while it is away fails immediately with
//! something a page can say, rather than hanging until a timeout.
//!
//! **A code typed on the Setup tab buys a token, and this client sends it on every request.** A room
//! below the queue or control level refuses a write with a 401 or a 403, and `check` maps both onto
//! [`RemoteError::Unauthorized`]. The page layer turns that into a toast pointing at the code box.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::StreamExt;
use km_api::discover::Discovery;
use km_api::dto::{
    AccessDto, AccessGrantDto, AddToQueueRequest, AddedToQueueDto, LoginRequest, MoveRequest,
    PackagesDto, QueueDto, SettingsPatchDto, SongDto, StateDto,
};
use km_api::events::Event;
use km_remote_pages::machine::{Connection, Machine, RemoteError, Transport, codes};
use km_songcode::SongCode;
use tokio::sync::{broadcast, watch};

/// How long a command waits before the page is told it was sent but not confirmed.
///
/// Three seconds, as the Go remote settled on. Long enough for a machine that is busy starting a
/// song, short enough that nobody standing with a microphone thinks the remote has died.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

/// How long the catalog import will wait for one page.
///
/// Much longer than a command: a page is up to five thousand rows off a box that may be reading them
/// from a spinning disk, and the person who asked for a refresh is watching a progress line rather
/// than holding a microphone.
const IMPORT_TIMEOUT: Duration = Duration::from_secs(60);

/// How far apart the event stream's reconnection attempts grow.
const BACKOFF_START: Duration = Duration::from_secs(1);
/// And where they stop growing.
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// How long the event stream's handshake may take before it counts as a machine that is not there.
///
/// `connect_async` has no timeout of its own, where every HTTP call above has had one since the
/// first of them. A dropped SYN — a firewall that refuses by saying nothing, which is the ordinary
/// way a Windows box refuses — otherwise leaves the attempt outstanding for as long as the operating
/// system feels like retransmitting, and the banner says "Connecting…" for all of it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a connected stream may say nothing before it counts as gone.
///
/// **Twenty missed heartbeats.** `km_api::events::run_state_ticker` publishes a `state` event every
/// [`km_api::events::STATE_INTERVAL`] whatever the machine is doing — its own doc insists on the
/// "whatever", because an idle machine's remote still has to learn that the queue emptied — so
/// silence for this long is a machine that is gone rather than one with nothing to say.
///
/// Derived from that constant rather than written as a number, so that a machine which ever stops
/// ticking breaks this loudly instead of leaving remotes flapping.
///
/// Without it a socket that is never closed is never noticed: a box switched off at the wall sends
/// no FIN and no RST, so the read below simply never returns, and the whole reconnection loop —
/// including its chance to notice a *different* machine having been chosen — sits behind it for ever.
const STREAM_IDLE_TIMEOUT: Duration = km_api::events::STATE_INTERVAL.saturating_mul(20);

/// How many events may queue up for the pages before the oldest are dropped.
const EVENT_CAPACITY: usize = 64;

/// Plain HTTP to one machine.
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    /// The access token a code bought from this machine, sent on every request.
    ///
    /// **One per machine, and shared by every clone of this `Api`.** The offline remote is one
    /// person's app, so the code they typed is the level every page of it acts at. A different
    /// machine is a new `Api`, and starts at that machine's room level.
    token: Arc<RwLock<Option<String>>>,
}

impl Api {
    /// A client for a machine at a base URL — `http://192.168.1.5:8177`.
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(COMMAND_TIMEOUT)
                .build()
                .unwrap_or_default(),
            base: base.into().trim_end_matches('/').to_owned(),
            token: Arc::new(RwLock::new(None)),
        }
    }

    /// The token held, if a code bought one.
    fn token(&self) -> Option<String> {
        self.token
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Keeps a token, or forgets the one held.
    fn set_token(&self, token: Option<String>) {
        *self
            .token
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = token;
    }

    /// Where this points.
    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    /// What the machine says about itself. Always public, always cheap.
    pub async fn discover(&self) -> Result<Discovery, RemoteError> {
        self.read(self.http.get(self.url("/discover"))).await
    }

    /// The installed packages, which is where the mirror gets their names. Public, like the export.
    pub async fn packages(&self) -> Result<PackagesDto, RemoteError> {
        self.read(self.http.get(self.url("/packages")).timeout(IMPORT_TIMEOUT))
            .await
    }

    /// One page of the catalog.
    ///
    /// Returns the songs and the catalog version the machine reported while serving them, so an
    /// import can notice a package being installed halfway through and start again rather than
    /// stitching two catalogs together.
    pub async fn export(
        &self,
        after: Option<SongCode>,
        limit: usize,
    ) -> Result<(Vec<SongDto>, Option<u64>), RemoteError> {
        let mut url = format!("{}?limit={limit}", self.url("/songs/export"));
        if let Some(after) = after {
            url.push_str(&format!("&after={after}"));
        }
        let response = self
            .http
            .get(url)
            .timeout(IMPORT_TIMEOUT)
            .send()
            .await
            .map_err(transport_error)?;
        let response = check(response).await?;
        let version = response
            .headers()
            .get("x-km-catalog-version")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let body = response.text().await.map_err(transport_error)?;

        let mut songs = Vec::new();
        for line in body.lines() {
            if line.trim().is_empty() {
                continue;
            }
            // A line that will not parse is a version mismatch, not a corrupt song, and importing
            // the rest would leave a catalog that is quietly missing whatever this build does not
            // understand. Better to stop and say so.
            let song: SongDto = serde_json::from_str(line)
                .map_err(|error| RemoteError::Failed(format!("unreadable song row: {error}")))?;
            songs.push(song);
        }
        Ok((songs, version))
    }

    async fn read<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, RemoteError> {
        let request = match self.token() {
            Some(token) => request.bearer_auth(token),
            None => request,
        };
        let response = request.send().await.map_err(transport_error)?;
        let response = check(response).await?;
        response.json().await.map_err(transport_error)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, RemoteError> {
        self.read(self.http.get(self.url(path))).await
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&impl serde::Serialize>,
    ) -> Result<T, RemoteError> {
        let request = self.http.post(self.url(path));
        let request = match body {
            Some(body) => request.json(body),
            None => request,
        };
        self.read(request).await
    }

    async fn put<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, RemoteError> {
        self.read(self.http.put(self.url(path)).json(body)).await
    }

    async fn delete<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, RemoteError> {
        self.read(self.http.delete(self.url(path))).await
    }
}

/// Turns a status into something a page can say.
///
/// The API's stable `error` codes are what this reads, not the status alone: `409` is both "the
/// queue is full" and "this song cannot do that", and those are different sentences.
async fn check(response: reqwest::Response) -> Result<reqwest::Response, RemoteError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let parsed: Option<km_api::dto::ErrorDto> = serde_json::from_str(&body).ok();
    let (code, message) = match parsed {
        Some(error) => (error.error, error.message),
        None => (String::new(), body),
    };
    Err(match (status.as_u16(), code.as_str()) {
        (401, _) | (403, _) => RemoteError::Unauthorized,
        (404, _) => RemoteError::NotFound,
        (_, "queue_full") => RemoteError::QueueFull,
        // **Every 409 that is not the queue**, rather than only the ones this build has a name for.
        // The code travels through unread; `km-remote-pages` is what turns it into a sentence, and a
        // name this build does not know renders as the generic refusal there. Matching known codes
        // here instead would mean an offline remote that met a newer machine fell all the way to
        // `Failed` — a red toast for something that is not a fault.
        (409, _) => RemoteError::Unavailable {
            code: code.clone(),
            message,
        },
        (400..=499, _) => RemoteError::Rejected(message),
        _ => RemoteError::Failed(message),
    })
}

/// Turns a transport failure into something a page can say.
///
/// The distinction that matters: a **timeout** is not a failure. The command almost certainly
/// arrived and what is missing is the acknowledgment, so the page says "sent" rather than coloring
/// it red — reporting a red failure for a song that is in fact in the queue is worse than saying
/// less. Anything else means the machine is not there.
fn transport_error(error: reqwest::Error) -> RemoteError {
    if error.is_timeout() {
        return RemoteError::NotAcknowledged;
    }
    RemoteError::Offline(codes::NOT_ANSWERING)
}

/// The machine, as the remote's pages see it.
///
/// **The address is a `watch` rather than a lock**, and that is what makes [`point_at`] mean
/// something to the event stream. A lock can only be read, so the follower learned of a new machine
/// when it next happened to look — which is *after* the stream it is on ends, and a stream to a
/// machine that is still running never does. A watch can be waited on, so pointing somewhere else
/// interrupts the follow rather than being noticed by it eventually.
///
/// [`point_at`]: MachineClient::point_at
#[derive(Clone)]
pub struct MachineClient {
    api: Arc<watch::Sender<Option<Api>>>,
    events: broadcast::Sender<Event>,
    connection: Arc<RwLock<Connection>>,
    wake: Arc<tokio::sync::Notify>,
    /// A second, broadcast signal that somebody is paying attention again.
    ///
    /// **Separate from `wake` rather than shared with it, because `notify_one` stores a permit and
    /// hands it to exactly one waiter.** Two listeners on that one would race, and the loser would
    /// be the event stream — so a page coming back on screen would sometimes fail to do the one
    /// thing this whole mechanism exists for. `notify_waiters` wakes every listener and stores
    /// nothing, which is right for a signal whose loss costs at most one fallback tick.
    attention: Arc<tokio::sync::Notify>,
}

impl MachineClient {
    /// A client that has not found a machine yet.
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            api: Arc::new(watch::Sender::new(None)),
            events,
            connection: Arc::new(RwLock::new(Connection {
                online: false,
                address: None,
                name: None,
                reason: Some(codes::LOOKING),
            })),
            wake: Arc::new(tokio::sync::Notify::new()),
            attention: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Try the machine now, rather than when the backoff next comes round.
    ///
    /// **Somebody is looking at a page again.** A phone that has been in a pocket froze this whole
    /// process along with its screen, so the wait between attempts is one the machine's absence was
    /// measured *before* the interruption — and the reported fault is precisely that: coming back to
    /// the Android remote and watching a red strip for ten seconds while a machine that was up all
    /// along went unasked.
    ///
    /// The precedent is the `/dev/` remote's Reconnect button, and its words fit unchanged:
    /// *somebody pressing it is saying they think the machine is back, which is a better guess than
    /// the interval a run of failures had arrived at.* A page coming back on screen is that same
    /// statement, made without a button.
    ///
    /// **Cheap, non-blocking, infallible and safe to call at any time.** It shortens the sleep
    /// between attempts and nothing else: it cannot interrupt a handshake that is in flight, which
    /// is what reusing the address watch for this would have done — see [`Self::redirects`]. A poke
    /// that arrives while the stream is healthy leaves a permit behind and so shortens the *next*
    /// wait by up to a second, which after [`next_backoff`] is a wait of one second anyway. Ten
    /// pokes collapse into one attempt, and that coalescing is the reason this is a `Notify` rather
    /// than a counter.
    /// **It also wakes the machine watch, and on Android that is what makes discovery work at all.**
    /// `MainActivity` takes the `WifiManager.MulticastLock` in `onStart` and releases it in
    /// `onStop`, so queries sent while the application was backgrounded went nowhere. `onStart`
    /// retakes the lock *before* the WebView resumes, the resumed page reopens its event stream, and
    /// the handler for that calls this — so by the time the watch asks the network again, the lock
    /// is held. No upcall into Java, no seventh FFI function, and the existing decision
    /// `Coming back to a page is a reason to try the machine now` covers discovery as well as
    /// reconnection without being widened.
    pub fn wake(&self) {
        self.wake.notify_one();
        self.attention.notify_waiters();
    }

    /// Notice somebody paying attention again — see [`Self::wake`].
    pub(crate) fn attention(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.attention)
    }

    /// Points this at a machine. Safe to call again when one is found somewhere else.
    ///
    /// The send is what stops the previous machine's stream; see [`spawn_event_stream`].
    ///
    /// **The name is dropped here and nowhere else.** This is the one call that can mean "a
    /// different machine", and a card that went on heading the new address with the old machine's
    /// name would be wrong in the way hardest to notice. The refresh that follows every caller of
    /// this puts one back within a request.
    pub fn point_at(&self, api: Api) {
        let address = api.base().to_owned();
        self.api.send_replace(Some(api));
        self.set_connection(Connection {
            online: false,
            address: Some(address),
            name: None,
            reason: Some(codes::CONNECTING),
        });
    }

    /// Records what the machine calls itself, leaving the rest of the connection alone.
    ///
    /// Called from the catalog refresh, which is where `/discover` is read — see
    /// [`sync::Refreshed`](crate::sync::Refreshed). It is a separate setter rather than a field on
    /// the two below because the two answer different questions on different clocks: reachability
    /// changes whenever the event stream drops, which on a machine under a television is most of the
    /// day, and rebuilding the whole connection there is what would silently throw the name away
    /// every time somebody switched the television off and on.
    pub fn set_machine_name(&self, name: Option<String>) {
        let mut connection = self
            .connection
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        connection.name = name;
    }

    /// Records whether the machine is answering, keeping whatever name it has already given.
    ///
    /// **Only the event stream below has any business calling this**, because it is the only thing
    /// that knows: reachability here means a socket that is open, not a request that succeeded. It
    /// is `pub(crate)` rather than private so that [`link`](crate::link)'s tests can put a link into
    /// the state that matters to them — a machine that is *answering*, which is what decides whether
    /// a rescan offers or moves, and which nothing outside this module can otherwise produce.
    pub(crate) fn set_reachability(
        &self,
        online: bool,
        address: Option<String>,
        reason: Option<&'static str>,
    ) {
        let mut connection = self
            .connection
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        connection.online = online;
        connection.address = address;
        connection.reason = reason;
    }

    /// The machine this points at, if any.
    pub fn api(&self) -> Option<Api> {
        // Cloned straight out: a `watch` borrow is a read guard, and holding one across an await
        // would block every later `point_at`.
        self.api.borrow().clone()
    }

    /// Notice when the machine this points at is replaced.
    ///
    /// Version-tracked rather than edge-triggered, so a redirect that lands between two of the
    /// follower's awaits is still seen. **A `Notify` cannot carry the address**, which is the real
    /// reason, and not that it would drop the signal: `notify_one` stores a permit when nobody is
    /// waiting, and [`Self::wake`] leans on exactly that. What a permit cannot tell the follower is
    /// *which machine*, and re-reading the watch is how it finds out.
    ///
    /// The two are therefore not interchangeable in either direction. A redirect must abort the
    /// stream it interrupts, because that stream is to the machine being left; a wake must not,
    /// because the stream it would abort is the one it is hoping for.
    fn redirects(&self) -> watch::Receiver<Option<Api>> {
        self.api.subscribe()
    }

    /// Notice somebody asking for an attempt now. See [`Self::wake`].
    fn wakes(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.wake)
    }

    fn set_connection(&self, connection: Connection) {
        *self
            .connection
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = connection;
    }

    async fn with_api<T, F, Fut>(&self, work: F) -> Result<T, RemoteError>
    where
        F: FnOnce(Api) -> Fut,
        Fut: std::future::Future<Output = Result<T, RemoteError>>,
    {
        let Some(api) = self.api() else {
            return Err(RemoteError::Offline(codes::NONE_FOUND));
        };
        work(api).await
    }
}

impl Default for MachineClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Machine for MachineClient {
    async fn state(&self) -> Result<StateDto, RemoteError> {
        self.with_api(|api| async move { api.get("/state").await })
            .await
    }

    async fn queue(&self) -> Result<QueueDto, RemoteError> {
        self.with_api(|api| async move { api.get("/queue").await })
            .await
    }

    /// The token a page's cookie holds wins over the one this client keeps, so a code typed on
    /// one page of the app is not undone by a stale cookie on another.
    async fn access(&self, token: Option<&str>) -> Result<AccessDto, RemoteError> {
        let token = token.map(str::to_owned);
        self.with_api(|api| async move {
            let request = api.http.get(api.url("/access"));
            let request = match token.or_else(|| api.token()) {
                Some(token) => request.bearer_auth(token),
                None => request,
            };
            let response = request.send().await.map_err(transport_error)?;
            check(response).await?.json().await.map_err(transport_error)
        })
        .await
    }

    async fn log_in(
        &self,
        code: &str,
        _from: Option<std::net::IpAddr>,
    ) -> Result<AccessGrantDto, RemoteError> {
        let request = LoginRequest {
            password: code.to_owned(),
        };
        self.with_api(|api| async move {
            let grant: AccessGrantDto = api.post("/login", Some(&request)).await?;
            api.set_token(Some(grant.token.clone()));
            Ok(grant)
        })
        .await
    }

    async fn enqueue(
        &self,
        number: SongCode,
        singer: Option<&str>,
    ) -> Result<AddedToQueueDto, RemoteError> {
        let request = AddToQueueRequest {
            number,
            singer: singer.map(str::to_owned),
        };
        self.with_api(|api| async move { api.post("/queue", Some(&request)).await })
            .await
    }

    async fn dequeue(&self, entry_id: u64) -> Result<QueueDto, RemoteError> {
        self.with_api(|api| async move { api.delete(&format!("/queue/{entry_id}")).await })
            .await
    }

    async fn move_entry(&self, entry_id: u64, to_index: usize) -> Result<QueueDto, RemoteError> {
        let request = MoveRequest { to_index };
        self.with_api(|api| async move {
            api.post(&format!("/queue/{entry_id}/move"), Some(&request))
                .await
        })
        .await
    }

    async fn clear_queue(&self) -> Result<QueueDto, RemoteError> {
        self.with_api(|api| async move { api.delete("/queue").await })
            .await
    }

    async fn transport(&self, command: Transport) -> Result<StateDto, RemoteError> {
        let verb = match command {
            Transport::Play => "play",
            Transport::Pause => "pause",
            Transport::Skip => "skip",
            Transport::Restart => "restart",
            Transport::Stop => "stop",
        };
        self.with_api(|api| async move {
            api.post::<StateDto>(&format!("/transport/{verb}"), None::<&()>)
                .await
        })
        .await
    }

    /// The answer is thrown away on purpose: it describes the machine before the song it just asked
    /// for has started. `check()` has already turned a refusal into `RemoteError::Unavailable`
    /// carrying the machine's own sentence, which is the only part of the reply worth anything here.
    async fn start_demo(&self) -> Result<(), RemoteError> {
        self.with_api(|api| async move {
            api.post::<km_api::dto::DemoDto>("/demo/start", None::<&()>)
                .await
        })
        .await
        .map(|_| ())
    }

    async fn update_settings(&self, patch: &SettingsPatchDto) -> Result<StateDto, RemoteError> {
        let patch = *patch;
        // `PUT /settings` answers with the settings, not the whole state, so the state is re-read.
        // Re-reading rather than patching a cached copy is what makes a clamped value — a stepper
        // walked past what the machine accepts — show what is really set.
        self.with_api(|api| async move {
            let _: km_api::dto::SettingsDto = api.put("/settings", &patch).await?;
            api.get("/state").await
        })
        .await
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn connection(&self) -> Connection {
        self.connection
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn wake(&self) {
        MachineClient::wake(self);
    }
}

/// Follows the machine's event stream, for as long as the process lives.
///
/// Reconnects with a backoff that grows to half a minute, and reports every transition through
/// [`Machine::connection`], which is what the banner and the tab-bar dot draw from. Losing the
/// connection is not an error and is not logged as one: a machine under a television is switched off
/// most of the day.
///
/// **Every wait in here loses to a redirect**, and that is the point rather than a refinement.
/// Reading the address once per iteration means learning of a machine somebody has chosen only
/// after the stream already in hand ends — and a stream to a machine that is still running never
/// ends. Choosing a second machine while the first is up then leaves the follower on the first for
/// ever: HTTP goes to the new one, so the remote works, while the banner says "Connecting…" and the
/// Now tab shows the first machine's playback. Waiting on the address instead of re-reading it is
/// what makes *Use this* take effect at once, and it spends none of the thirty-second backoff a
/// machine that has been away had built up.
///
/// **The wait between attempts also loses to a wake**, which is the same idea one step weaker: a
/// redirect says *that machine instead*, a wake says *the one you have, now*. It reaches only the
/// backoff and never a handshake — see [`MachineClient::wake`] and [`wait_before_retry`].
pub fn spawn_event_stream(client: MachineClient) {
    tokio::spawn(async move {
        let mut redirects = client.redirects();
        let wakes = client.wakes();
        let mut backoff = BACKOFF_START;
        loop {
            // `borrow_and_update` rather than `client.api()`: taking the address and marking it seen
            // in one step is what stops the wait below firing immediately on the redirect that
            // brought us here.
            let current = redirects.borrow_and_update().clone();
            let Some(api) = current else {
                // No sleep here: there is exactly one thing that can end this wait, and it is the
                // arrival of an address. **And no wake either**, deliberately -- there is nothing
                // to retry, and finding a machine in the first place is the locator's business, on
                // its own recovery interval.
                if redirects.changed().await.is_err() {
                    return;
                }
                backoff = BACKOFF_START;
                continue;
            };

            let outcome = tokio::select! {
                outcome = follow(&client, &api, STREAM_IDLE_TIMEOUT) => outcome,
                // Pointed somewhere else. The socket to the machine being left is dropped with the
                // future, which is the tidy half of what this arm is for.
                changed = redirects.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    backoff = BACKOFF_START;
                    continue;
                }
            };

            // **Read before the reachability below overwrites it**, because it is the only record
            // of whether this attempt got as far as a working stream: `follow` sets it the instant
            // the handshake completes. See [`next_backoff`], which is the whole reason it is read.
            let had_connected = client.connection().online;

            match outcome {
                // A machine that closes the stream politely is still a machine that has gone, and
                // saying so here is what stops the dot staying green for a backoff's worth of
                // seconds after the television was switched off.
                Ok(()) => {
                    tracing::debug!("the machine closed the event stream");
                    client.set_reachability(
                        false,
                        Some(api.base().to_owned()),
                        Some(codes::STREAM_CLOSED),
                    );
                }
                Err(reason) => {
                    tracing::debug!(%reason, had_connected, "the event stream is down");
                    // The reason is a transport diagnostic and it is on the line above, where the
                    // person who can act on it will look. What the banner gets is the fact.
                    client.set_reachability(
                        false,
                        Some(api.base().to_owned()),
                        Some(codes::NOT_ANSWERING),
                    );
                }
            }
            backoff = next_backoff(backoff, had_connected);

            match wait_before_retry(backoff, &mut redirects, &wakes).await {
                Retry::Slept => {}
                // Both mean the same thing about the wait — do not spend the rest of it — and
                // differ in nothing this loop has left to do, since the address is re-read at the
                // top of every iteration anyway.
                Retry::Woken | Retry::Redirected => backoff = BACKOFF_START,
                Retry::Gone => return,
            }
        }
    });
}

/// What ended the wait between two attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retry {
    /// The backoff ran its course.
    Slept,
    /// Somebody came back to a page and asked for an attempt now. See [`MachineClient::wake`].
    Woken,
    /// This remote was pointed at a different machine.
    Redirected,
    /// The client has been dropped; there is nothing left to follow.
    Gone,
}

/// Waits out the backoff, unless something better turns up first.
///
/// Extracted from [`spawn_event_stream`] for the reason [`follow`] takes its deadline as an
/// argument: it is the one part of the loop a test can drive, and it touches no socket, so a test
/// of it races nothing.
async fn wait_before_retry(
    backoff: Duration,
    redirects: &mut watch::Receiver<Option<Api>>,
    wake: &tokio::sync::Notify,
) -> Retry {
    tokio::select! {
        _ = tokio::time::sleep(backoff) => Retry::Slept,
        _ = wake.notified() => Retry::Woken,
        changed = redirects.changed() => match changed {
            Ok(()) => Retry::Redirected,
            Err(_) => Retry::Gone,
        },
    }
}

/// How long to wait before trying the machine again.
///
/// **A stream that had connected starts over at [`BACKOFF_START`]**, and that half was missing. The
/// `backoff` in [`spawn_event_stream`] outlives an iteration, and every real-world drop arrives as
/// an `Err` — the idle deadline says "The karaoke machine stopped answering." and a socket error says
/// "The connection dropped (…)" — so the doubling used to apply to attempts that had *worked*. A
/// machine that blinked through an evening therefore walked the wait out to half a minute, and each
/// later blink cost up to thirty seconds of "not reachable" for a box that was answering again within
/// one. It presents as the fault getting worse the longer the remote is left running, which is the
/// hardest kind to report.
///
/// Doubling is for a machine that is **not there**: it is what stops a remote hammering an address
/// nothing is listening at, and it has no business measuring a connection that came up. Only the
/// handshake failing — a timeout, a refusal — is a reason to ask less often.
fn next_backoff(current: Duration, had_connected: bool) -> Duration {
    if had_connected {
        BACKOFF_START
    } else {
        (current * 2).min(BACKOFF_MAX)
    }
}

/// One connection's worth of following, until it drops.
///
/// `idle` is how long the machine may say nothing before this gives up on it. Passed in rather than
/// read from [`STREAM_IDLE_TIMEOUT`] because it is the one thing here a test has to be able to
/// shorten: proving that silence is noticed otherwise means a test that waits five real seconds, and
/// a paused clock cannot help — `tokio` advances a paused clock whenever the runtime is idle, which
/// during a socket handshake it is.
async fn follow(client: &MachineClient, api: &Api, idle: Duration) -> Result<(), String> {
    let url = format!("{}/api/v1/events", api.base()).replacen("http", "ws", 1);
    let (socket, _) = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| "The karaoke machine is not answering.".to_owned())?
        .map_err(|error| format!("The karaoke machine is not answering ({error})."))?;

    client.set_reachability(true, Some(api.base().to_owned()), None);
    tracing::info!(machine = api.base(), "connected to the karaoke machine");

    let (_, mut incoming) = socket.split();
    // **Read with a deadline, because a socket that is never closed is never noticed.** A box
    // switched off at the wall sends neither FIN nor RST, so a bare `incoming.next()` waits on it
    // until the process ends. See [`STREAM_IDLE_TIMEOUT`] for why silence is unambiguous here.
    loop {
        let message = match tokio::time::timeout(idle, incoming.next()).await {
            Ok(Some(message)) => message,
            Ok(None) => return Ok(()),
            Err(_) => return Err("The karaoke machine stopped answering.".to_owned()),
        };
        let message = message.map_err(|error| format!("The connection dropped ({error})."))?;
        let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
            // Ping, pong and close are the library's business, not ours.
            continue;
        };
        match serde_json::from_str::<Event>(&text) {
            // A send with no subscribers is not an error: nobody has a page open, which is the usual
            // state of a remote sitting in somebody's pocket.
            Ok(event) => {
                let _ = client.events.send(event);
            }
            // An event this build does not know about is a newer machine, not a fault. Skipping it
            // keeps the rest of the stream working, which is the whole reason the enum is tagged.
            Err(error) => {
                tracing::debug!(%error, "skipping an event this build does not understand")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_url_loses_its_trailing_slash_so_paths_do_not_double_up() {
        let api = Api::new("http://192.168.1.5:8177/");
        assert_eq!(api.base(), "http://192.168.1.5:8177");
        assert_eq!(api.url("/state"), "http://192.168.1.5:8177/api/v1/state");
    }

    #[test]
    fn a_client_with_no_machine_says_so_rather_than_looking_connected() {
        let client = MachineClient::new();
        let connection = client.connection();
        assert!(!connection.online);
        assert!(connection.reason.is_some());
    }

    #[tokio::test]
    async fn a_command_with_no_machine_fails_at_once_instead_of_waiting() {
        let client = MachineClient::new();
        let error = client.state().await.expect_err("no machine");
        assert!(matches!(error, RemoteError::Offline(_)), "{error:?}");
    }

    /// A television switched off and on again must not cost the machine its name.
    ///
    /// This is the whole reason `set_reachability` exists rather than a fourth `Connection` literal.
    /// The event stream drops and reconnects whenever the machine is switched off, which on a box
    /// under a television is most of the day, and each transition rebuilds the connection — so a
    /// rebuild that did not carry the name would show one for a few minutes after a refresh and
    /// then lose it silently, which is the worst of the three possible behaviors.
    #[test]
    fn losing_and_regaining_the_connection_keeps_the_name_the_machine_gave() {
        let client = MachineClient::new();
        client.point_at(Api::new("http://192.168.1.5:8177"));
        client.set_machine_name(Some("Living Room".to_owned()));

        client.set_reachability(true, Some("http://192.168.1.5:8177".to_owned()), None);
        assert_eq!(client.connection().name.as_deref(), Some("Living Room"));

        client.set_reachability(
            false,
            Some("http://192.168.1.5:8177".to_owned()),
            Some(codes::NOT_ANSWERING),
        );
        let connection = client.connection();
        assert!(!connection.online);
        assert_eq!(
            connection.name.as_deref(),
            Some("Living Room"),
            "the machine did not stop being called that by going quiet"
        );
    }

    /// Being pointed somewhere else *does* cost it the name, and that is the point.
    ///
    /// The inverse of the test above, and the one case where dropping it is correct: this is the
    /// only call that can mean a different machine, and heading a new address with the old
    /// machine's name is wrong in the way hardest to notice.
    #[test]
    fn pointing_somewhere_else_drops_the_name_rather_than_carrying_it_over() {
        let client = MachineClient::new();
        client.point_at(Api::new("http://192.168.1.5:8177"));
        client.set_machine_name(Some("Living Room".to_owned()));

        client.point_at(Api::new("http://192.168.1.42:8177"));
        assert_eq!(client.connection().name, None);
    }

    #[test]
    fn pointing_at_a_machine_records_where_it_is_before_it_answers() {
        let client = MachineClient::new();
        client.point_at(Api::new("http://192.168.1.5:8177"));
        let connection = client.connection();
        assert!(!connection.online, "not until the stream connects");
        assert_eq!(
            connection.address.as_deref(),
            Some("http://192.168.1.5:8177")
        );
    }

    /// The one distinction this module exists to make: a timeout is not a failure, because the
    /// command almost certainly arrived and only the confirmation is missing.
    #[test]
    fn an_unacknowledged_command_is_not_reported_as_a_fault() {
        assert!(!RemoteError::NotAcknowledged.is_fault());
    }

    /// **A blink is not a reason to ask less often**, and this is the reported fault: the red strip
    /// appearing over and over through an evening and taking longer to clear each time.
    ///
    /// The doubling used to apply to any `Err`, and every ordinary drop is one — the idle deadline
    /// and a socket error both. So a machine that flapped walked the wait to [`BACKOFF_MAX`] and
    /// each later blink cost half a minute of "not reachable" for a box that was back in one.
    #[test]
    fn a_stream_that_had_connected_does_not_lengthen_the_wait() {
        assert_eq!(next_backoff(Duration::from_secs(16), true), BACKOFF_START);
        assert_eq!(next_backoff(BACKOFF_MAX, true), BACKOFF_START);
    }

    /// The other half, which is what the doubling is actually for: an address nothing is listening
    /// at must not be hammered four times a second for an evening.
    #[test]
    fn a_machine_that_never_answered_is_asked_less_and_less_often() {
        let first = next_backoff(BACKOFF_START, false);
        assert_eq!(first, BACKOFF_START * 2);
        assert_eq!(next_backoff(first, false), BACKOFF_START * 4);
    }

    #[test]
    fn the_wait_stops_growing_at_half_a_minute() {
        assert_eq!(next_backoff(BACKOFF_MAX, false), BACKOFF_MAX);
        assert_eq!(
            next_backoff(BACKOFF_MAX / 2 + BACKOFF_START, false),
            BACKOFF_MAX
        );
    }

    /// **The reported fault, at the one place it can be measured.** Coming back to the Android
    /// remote meant watching a red strip while a machine that was up went unasked, because the
    /// process had been frozen mid-backoff and the wait it resumed was one measured before the
    /// interruption.
    ///
    /// **No clock is manipulated and none is needed.** The obvious spelling —
    /// `#[tokio::test(start_paused = true)]` — needs tokio's `test-util` feature that
    /// this workspace does not carry, and [`follow`]'s doc explains why a paused clock is a poor
    /// tool here anyway. A stored permit makes `notified()` ready on its first poll, so a wake that
    /// works returns at once and a wake that does not hangs; the `timeout` is what turns the second
    /// into a failed assertion instead of half a minute of a test suite doing nothing.
    #[tokio::test]
    async fn a_wake_ends_the_wait_between_attempts_at_once() {
        let client = MachineClient::new();
        let mut redirects = client.redirects();
        let wakes = client.wakes();

        client.wake();
        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            wait_before_retry(BACKOFF_MAX, &mut redirects, &wakes),
        )
        .await
        .expect("a wake does not wait out the backoff");

        assert_eq!(
            outcome,
            Retry::Woken,
            "half a minute of the wait was still to run"
        );
    }

    /// **The stored permit is what the resume race depends on**, so it is asserted rather than
    /// assumed. A page reopens its stream the moment it is back on screen, which may be while the
    /// follower is still inside a handshake and not yet waiting — and a signal dropped there would
    /// leave exactly the ten seconds of red this was written to remove.
    ///
    /// It is also the sentence [`MachineClient::redirects`] used to get wrong in the other
    /// direction, which is why this test names it.
    #[tokio::test]
    async fn a_wake_that_arrives_before_anybody_is_waiting_is_not_lost() {
        let client = MachineClient::new();
        let mut redirects = client.redirects();
        let wakes = client.wakes();

        // Nobody is inside `wait_before_retry` yet, which is the case a `Notify` is claimed to
        // survive and an edge-triggered signal would not.
        client.wake();
        client.wake();

        let woken = tokio::time::timeout(
            Duration::from_secs(1),
            wait_before_retry(BACKOFF_MAX, &mut redirects, &wakes),
        )
        .await
        .expect("the permit outlived the gap");
        assert_eq!(woken, Retry::Woken);

        // ...and the second collapses into the first rather than buying a second free attempt,
        // which is the coalescing that makes a burst of pokes harmless. A millisecond rather than
        // `BACKOFF_START`, so proving that this one *does* wait costs no wall clock worth having.
        assert_eq!(
            wait_before_retry(Duration::from_millis(1), &mut redirects, &wakes).await,
            Retry::Slept,
            "two pokes are one attempt, not two"
        );
    }

    /// Waking a remote that has never found a machine is not a search, and must not look like one.
    #[test]
    fn waking_a_client_with_no_machine_changes_nothing() {
        let client = MachineClient::new();
        client.wake();
        let connection = client.connection();
        assert!(!connection.online);
        assert_eq!(connection.address, None);
    }

    /// The silence this gives up on is measured in the machine's own heartbeats, so a machine that
    /// stops ticking is a broken build here rather than a remote that flaps.
    #[test]
    fn the_idle_deadline_is_a_count_of_the_machines_heartbeats() {
        assert_eq!(
            STREAM_IDLE_TIMEOUT,
            km_api::events::STATE_INTERVAL * 20,
            "twenty missed heartbeats"
        );
    }

    /// A silent listener that completes the WebSocket handshake and then says nothing at all.
    ///
    /// **Loopback only**, which is a rule rather than a habit here: Windows keys a firewall rule on
    /// the full image path, and a test binary's path carries a build hash, so a test that binds
    /// anything else raises a fresh prompt and leaves a dead rule behind on every rebuild.
    ///
    /// The listener is returned so a caller can choose what the machine does with it: [`hold_one`]
    /// answers one connection and then says nothing, and a caller that never accepts at all leaves
    /// every attempt hanging.
    ///
    /// **Do not drop it to make a connection fail.** An ephemeral port this process has let go is
    /// one the OS may hand straight to another test in the same run, and a test asserting that
    /// nothing answers there then fails whenever something does. See
    /// [`a_machine_that_never_answers_does_not_hold_the_attempt_open`], which is where that was
    /// found.
    async fn silent_machine() -> (tokio::net::TcpListener, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = format!("http://{}", listener.local_addr().expect("bound"));
        (listener, address)
    }

    /// Accepts one connection, upgrades it, and holds it open without ever sending anything.
    fn hold_one(listener: tokio::net::TcpListener) {
        tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(socket) = tokio_tungstenite::accept_async(stream).await else {
                return;
            };
            // Held, not dropped: dropping would close the socket and the follower would call that a
            // connection that ended rather than one that went quiet.
            std::future::pending::<()>().await;
            drop(socket);
        });
    }

    /// How long [`until`] waits before it calls the wait a failure.
    ///
    /// **It has to clear one whole failed attempt, or a loaded box fails the test instead of the
    /// code.** An attempt that gets no answer costs [`CONNECT_TIMEOUT`], and the retry after it
    /// waits [`BACKOFF_START`] — six seconds together, which is exactly what this budget used to
    /// be, so one slow handshake was enough to end a run. Patience is free when nothing goes wrong:
    /// `until` returns the moment its predicate holds, not when the budget expires.
    const PATIENCE: Duration = Duration::from_secs(30);

    /// Waits for the connection to satisfy `done`, giving up rather than hanging the suite.
    async fn until(
        client: &MachineClient,
        what: &str,
        done: impl Fn(&Connection) -> bool,
    ) -> Connection {
        // A deadline, not a count of naps. Each `sleep` overshoots under load, so a budget spelled
        // as iterations is shortest in precisely the conditions it exists to survive.
        let give_up_at = tokio::time::Instant::now() + PATIENCE;
        loop {
            let connection = client.connection();
            if done(&connection) {
                return connection;
            }
            assert!(
                tokio::time::Instant::now() < give_up_at,
                "timed out waiting for {what} after {PATIENCE:?}: {:?}",
                client.connection()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The fault this loop guards, and it needs no unreachable machine to show it.
    ///
    /// A remote following a machine that is **still running** has to be able to leave it. Re-read
    /// the address only after the stream ends and it never will: choosing a second machine swaps
    /// the address every HTTP call uses — so the remote appears to work — while the follower stays
    /// on the first, the Now tab goes on showing the first machine's playback, and the banner reads
    /// "Connecting…" for the rest of the evening.
    #[tokio::test]
    async fn choosing_another_machine_leaves_the_one_being_followed() {
        let (first, first_address) = silent_machine().await;
        let (second, second_address) = silent_machine().await;
        hold_one(first);
        hold_one(second);

        let client = MachineClient::new();
        client.point_at(Api::new(&first_address));
        spawn_event_stream(client.clone());

        let connection = until(&client, "the first machine", |c| c.online).await;
        assert_eq!(connection.address.as_deref(), Some(first_address.as_str()));

        // The first is still up and still connected. Nothing ends it; the redirect has to.
        client.point_at(Api::new(&second_address));

        let connection = until(&client, "the second machine", |c| {
            c.online && c.address.as_deref() == Some(second_address.as_str())
        })
        .await;
        assert_eq!(connection.reason, None, "connected, so nothing to say");
    }

    /// A machine switched off at the wall sends neither FIN nor RST, so the read has to have a
    /// deadline of its own or the connection state freezes on whatever it last said.
    #[tokio::test]
    async fn a_machine_that_goes_quiet_is_given_up_on() {
        let (listener, address) = silent_machine().await;
        hold_one(listener);

        let client = MachineClient::new();
        let api = Api::new(&address);
        let outcome = follow(&client, &api, Duration::from_millis(50)).await;

        assert_eq!(
            outcome,
            Err("The karaoke machine stopped answering.".to_owned())
        );
        assert!(
            client.connection().online,
            "follow reports the drop to its caller; the loop is what writes it down"
        );
    }

    /// The handshake gets a deadline too. Nothing answers on this port, and on a host that refuses
    /// by saying nothing that is otherwise an attempt with no end to it.
    ///
    /// **The listener is held and never accepted from, rather than dropped.** Dropping it freed the
    /// port, another test in the same run bound it, and this one failed saying the client was
    /// online — which it was, to somebody else's machine. Holding it also makes the test match its
    /// own name: a closed port is refused at once and proves no deadline at all, where a listener
    /// that never accepts leaves the handshake with nothing but [`CONNECT_TIMEOUT`] to end it.
    #[tokio::test]
    async fn a_machine_that_never_answers_does_not_hold_the_attempt_open() {
        // Bound for the length of the test and never accepted from, so the port stays this test's
        // and every attempt on it hangs.
        let (_never_accepted, address) = silent_machine().await;

        let client = MachineClient::new();
        client.point_at(Api::new(&address));
        spawn_event_stream(client.clone());

        let connection = until(&client, "the attempt to settle", |c| {
            c.reason != Some(codes::CONNECTING)
        })
        .await;
        assert!(!connection.online);
        assert_eq!(connection.address.as_deref(), Some(address.as_str()));
    }
}
