//! What the machine can be asked to do, and the events each of those entails.
//!
//! Every one of these was a handler body until the machine started serving a remote of its own.
//! There are now **two** callers for each: the JSON endpoint in [`handlers`](crate::handlers), and
//! `km-remote-pages`'s pages running in this same process. They must not be two implementations.
//!
//! The half that would have gone wrong is not the operation, it is the **publishing**. Queueing a
//! song is one call to the controller; queueing a song *and telling every open page about it* is two,
//! and the second is invisible if you forget it — the queue changes, nothing on any phone moves, and
//! the fault looks like a stale browser rather than a missing line. Keeping the pair together in one
//! place is the whole point of this module.
//!
//! Everything here is synchronous. The controller is a command channel and the catalog is SQLite;
//! neither awaits, and pretending otherwise would put an `async` on twelve functions that never
//! yield.
//!
//! **What that premise misses, and what [`off_runtime`] is for.** Two of these are not only a
//! command channel: [`enqueue`] on an idle machine, and [`transport`] asked to play or skip, all
//! reach the machine's `advance`, which opens the song's package, reads the entry and parses a MIDI
//! or opens a video decoder before it sends anything. That is file I/O and, for a video, ffmpeg —
//! tens of milliseconds for a small `.kar` and seconds for a large video — run to completion on
//! whatever thread called. On a tokio worker that is a worker not answering anything else, and with
//! only as many workers as the box has cores, a handful of those is the API, the singer's remote at
//! `/` and the event stream all stopping together. So the two that can reach it are called through
//! [`off_runtime`], and the decision lives here rather than at the four call sites, for the same
//! reason the publishing does.

use std::future::Future;

use crate::dto::{
    AddedToQueueDto, QueueDto, RequestDto, RequestsDto, SettingsDto, SingerDto, SingerIssuedDto,
    SingersDto, StateDto,
};
use km_songcode::SongCode;

use crate::error::{ApiError, ApiResult};
use crate::events::{EndReason, Event};
use crate::machine::{SettingsPatch, TransportCommand};
use crate::server::ApiState;
use km_queue::evening::{Proof, Request, RequestedSong};
use km_queue::queue::QueueRequest;
use km_queue::seats::SeatName;

/// Runs one of these operations on a blocking thread instead of on the async runtime.
///
/// **It does not make the operation quicker, and that is not what it is for.** The request that
/// asked waits exactly as long as it did; what changes is that nothing *else* waits with it. See
/// the note in this module's header for which operations need it and why.
///
/// [`ApiState`] is an `Arc` inside, so the clone this takes is a pointer bump rather than a copy of
/// the machine.
pub fn off_runtime<T, F>(
    state: &ApiState,
    operation: F,
) -> impl Future<Output = ApiResult<T>> + use<T, F>
where
    F: FnOnce(&ApiState) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let state = state.clone();
    let task = tokio::task::spawn_blocking(move || operation(&state));
    async move {
        match task.await {
            Ok(result) => result,
            // A blocking task fails only by panicking, and a panic in an operation is a bug rather
            // than a refusal — so it is a 500 naming the panic, not a 409 pretending the machine
            // declined.
            Err(error) => Err(ApiError::Internal(format!(
                "the machine panicked handling that: {error}"
            ))),
        }
    }
}

/// Reads the queue, publishes it, and hands it back so the caller can answer with it.
///
/// **The requests go out beside it, every time.** A queued request carries its entry's position,
/// and an entry that leaves lets a waiting request in, so no change to the queue leaves the
/// request list as it was.
pub fn publish_queue(state: &ApiState) -> QueueDto {
    let entries = state.controller().queue();
    let queue = QueueDto::new(&entries);
    state.events().publish(Event::QueueChanged {
        queue: queue.clone(),
    });
    state.events().publish(Event::RequestsChanged {
        requests: RequestsDto::new(&state.controller().requests(), &entries),
    });
    queue
}

/// Publishes the singers, after one registered, took a name or got a new token.
fn publish_singers(state: &ApiState) {
    state.events().publish(Event::SingersChanged {
        singers: singers(state),
    });
}

/// Every request the evening holds.
pub fn requests(state: &ApiState) -> RequestsDto {
    RequestsDto::new(&state.controller().requests(), &state.controller().queue())
}

/// Takes a singer's request for a song by number.
///
/// **The number is resolved before anything is held**, for [`enqueue`]'s reason. The answer says
/// where the request stands: queued with its place, or waiting.
pub fn add_request(
    state: &ApiState,
    singer: u64,
    proof: Proof<'_>,
    number: SongCode,
) -> ApiResult<RequestDto> {
    let song = state
        .catalog()
        .song(number)?
        .ok_or_else(|| ApiError::not_found(format!("song {number}")))?;
    let request = state.controller().request_add(
        singer,
        proof,
        RequestedSong {
            number: song.number,
            title: song.title,
            artist: song.artist,
            duration_ms: song.duration_ms,
        },
    )?;
    publish_queue(state);
    Ok(answer_for(state, &request))
}

/// Takes a request back, for the singer whose token this is.
pub fn withdraw_request(state: &ApiState, id: u64, token: &str) -> ApiResult<RequestDto> {
    let request = state.controller().request_withdraw(id, token)?;
    publish_queue(state);
    Ok(answer_for(state, &request))
}

/// One request as an answer, read again after the queue went out.
///
/// A request accepted onto an idle machine starts at once, so its entry may already be gone. The
/// answer then says the song played, which is what the list says.
fn answer_for(state: &ApiState, request: &Request) -> RequestDto {
    let current = state
        .controller()
        .requests()
        .into_iter()
        .find(|held| held.id == request.id);
    RequestDto::new(
        current.as_ref().unwrap_or(request),
        &state.controller().queue(),
    )
}

/// Queues a song by number.
///
/// **The number is resolved before anything is queued**, so a wrong one is "no such song" rather
/// than a queue entry with an empty title that fails when its turn comes — which is what punching a
/// wrong number into a real machine should feel like.
pub fn enqueue(state: &ApiState, number: SongCode) -> ApiResult<AddedToQueueDto> {
    let song = state
        .catalog()
        .song(number)?
        .ok_or_else(|| ApiError::not_found(format!("song {number}")))?;

    let entry_id = state.controller().queue_add(QueueRequest {
        number: song.number,
        title: song.title.clone(),
        artist: song.artist.clone(),
        // A song with no singer. A singer's song is a request, and `add_request` takes it.
        singer: None,
        duration_ms: song.duration_ms,
        seat: None,
    })?;

    let queue = publish_queue(state);
    let position = queue
        .entries
        .iter()
        .position(|entry| entry.id == entry_id)
        .unwrap_or(queue.entries.len().saturating_sub(1));

    Ok(AddedToQueueDto {
        entry_id,
        position,
        title: song.title,
        artist: song.artist,
    })
}

/// Bytes of randomness in a seat's token.
///
/// The machine stores the token and compares it, so it needs no signature and no parts. Sixteen
/// bytes is what a level's token draws for its nonce.
const SEAT_TOKEN_BYTES: usize = 16;

/// A typed name, with the key two names are compared by, or `None` for nothing typed.
///
/// The key ignores case, surrounding spaces and accents, which is the catalog's own fold. So a
/// second `ana` cannot stand beside `Ana` on the screen.
fn seat_name(typed: &str) -> Option<SeatName> {
    let shown = typed.trim();
    (!shown.is_empty()).then(|| SeatName {
        shown: shown.to_owned(),
        key: km_song::text::fold(shown),
    })
}

/// The evening's singers.
pub fn singers(state: &ApiState) -> SingersDto {
    SingersDto {
        singers: state.controller().seats().iter().map(Into::into).collect(),
        capacity: km_queue::MAX_SEATS,
        evening: state.evening().to_owned(),
    }
}

/// Registers a singer for a device, and hands back the seat with its token.
///
/// A name of spaces alone is no name, so that singer registers unnamed.
pub fn register_singer(
    state: &ApiState,
    device: &str,
    name: Option<&str>,
) -> ApiResult<SingerIssuedDto> {
    let issued = state.controller().seat_register(
        device.trim(),
        name.and_then(seat_name),
        crate::auth::random_hex(SEAT_TOKEN_BYTES),
    )?;
    // The id and never the token: this line reaches the log, and the log reaches the admin page.
    tracing::info!(singer = issued.seat.id, "a singer registered");
    publish_singers(state);
    Ok(SingerIssuedDto {
        singer: (&issued.seat).into(),
        token: issued.token,
        evening: state.evening().to_owned(),
    })
}

/// Gives a singer a name, or another name. The seat's token is the proof.
pub fn rename_singer(state: &ApiState, id: u64, token: &str, name: &str) -> ApiResult<SingerDto> {
    let name = seat_name(name)
        .ok_or_else(|| ApiError::BadRequest("a singer's name cannot be empty".to_owned()))?;
    let seat = state.controller().seat_rename(id, token, name)?;
    publish_singers(state);
    // The seat's queued songs took the name, and the screen draws it from the entry.
    publish_queue(state);
    Ok((&seat).into())
}

/// Gives a singer a new token, for a caller whose level is the check.
pub fn reissue_singer(state: &ApiState, id: u64) -> ApiResult<SingerIssuedDto> {
    let issued = state
        .controller()
        .seat_reissue(id, crate::auth::random_hex(SEAT_TOKEN_BYTES))?;
    tracing::info!(singer = issued.seat.id, "a singer's token was reissued");
    Ok(SingerIssuedDto {
        singer: (&issued.seat).into(),
        token: issued.token,
        evening: state.evening().to_owned(),
    })
}

/// Takes an entry out of the queue.
pub fn dequeue(state: &ApiState, entry_id: u64) -> ApiResult<QueueDto> {
    state
        .controller()
        // A plain `?`. Flattening every refusal into a 404 naming the entry -- which is what a
        // `ControlError::NotFound` carrying no subject of its own invites -- tells the client
        // the entry does not exist the first time `queue_remove` refuses for any other reason.
        .queue_remove(entry_id)?;
    Ok(publish_queue(state))
}

/// Moves an entry to a position.
pub fn move_entry(state: &ApiState, entry_id: u64, to_index: usize) -> ApiResult<QueueDto> {
    state
        .controller()
        // See `dequeue`: an out-of-range index would have read as a missing entry.
        .queue_move(entry_id, to_index)?;
    Ok(publish_queue(state))
}

/// Empties the queue.
pub fn clear_queue(state: &ApiState) -> ApiResult<QueueDto> {
    state.controller().queue_clear()?;
    Ok(publish_queue(state))
}

/// Asks the machine for one demo song, now.
///
/// **Nothing to publish, deliberately**: the machine has not started anything yet, and an event
/// saying it had would be a promise kept fifty milliseconds later by the `SongStarted` its poll
/// thread already sends. No [`off_runtime`] either — the work that could block
/// a runtime thread is on that thread by construction. See `handlers::start_demo`.
///
/// Here rather than inline in the handler because it has two callers: the JSON API and the remote
/// pages mounted inside the machine.
pub fn start_demo(state: &ApiState) -> ApiResult<crate::machine::DemoState> {
    Ok(state.controller().start_demo_song()?)
}

/// Runs a transport command and publishes what changed.
///
/// Two events can come out of one press and both matter to a remote showing "up next": the song
/// that started or ended, and the queue, because playing or skipping takes a song off the front.
pub fn transport(state: &ApiState, command: TransportCommand) -> ApiResult<StateDto> {
    let before = state.controller().snapshot();
    state.controller().transport(command)?;
    let after = state.controller().snapshot();

    if after.now_playing != before.now_playing {
        match &after.now_playing {
            Some(now) => state.events().publish(Event::SongStarted {
                now_playing: crate::dto::NowPlayingDto::from(now),
            }),
            None => state.events().publish(Event::SongEnded {
                reason: match command {
                    TransportCommand::Skip => EndReason::Skipped,
                    TransportCommand::Stop => EndReason::Stopped,
                    _ => EndReason::Finished,
                },
            }),
        };
    }
    if after.queue_len != before.queue_len {
        publish_queue(state);
    }
    Ok(StateDto::from(&after))
}

/// Applies a settings patch and publishes the result.
pub fn apply_settings(state: &ApiState, patch: SettingsPatch) -> ApiResult<SettingsDto> {
    let settings = SettingsDto::from(state.controller().update_settings(&patch)?);
    state.events().publish(Event::SettingsChanged { settings });
    Ok(settings)
}

/// Chooses a SoundFont, publishes what it moved, and answers with the new list.
///
/// Here rather than in the handler because two callers need it and there must be one translation of
/// `ControlError` into `ApiError`, not two: `PUT /audio/soundfont` is one, and the machine serving
/// its own remote pages is the other, which reaches the controller without going through HTTP at
/// all. That second caller is exactly what the note on `km-app`'s `api_failed` is about.
///
/// **Publishes `SettingsChanged` rather than an event of its own**, and it is not a stand-in:
/// choosing a bank really does move `music_volume`, because a bank measured to clip at `1.0` carries
/// the level it wants. A player card still showing the old level would be wrong until something else
/// happened to change.
pub fn set_soundfont(state: &ApiState, id: &str) -> ApiResult<crate::dto::SoundFontsDto> {
    state.controller().set_soundfont(id)?;
    let settings = SettingsDto::from(state.controller().snapshot().settings);
    state.events().publish(Event::SettingsChanged { settings });
    // The shortlist: this answers a picker, and a caller that wants the whole catalog asks for it
    // by name on the route that takes the parameter.
    Ok(crate::dto::SoundFontsDto::from(
        &state.controller().soundfonts(false),
    ))
}

/// Removes an installed SoundFont, publishes what it moved, and answers with the new list.
///
/// Here rather than in the handler for the same two-callers reason [`set_soundfont`] gives, and it
/// publishes `SettingsChanged` for the same reason too — deleting the bank the setting names falls
/// back to the bundled one, which moves both `audio.soundfont` and `music_volume`. Publishing
/// unconditionally is deliberate: a delete that moved nothing costs one event that says nothing
/// changed, where working out whether it moved would mean reading the settings twice to save it.
pub fn delete_soundfont(state: &ApiState, id: &str) -> ApiResult<crate::dto::SoundFontsDto> {
    state.controller().delete_soundfont(id)?;
    let settings = SettingsDto::from(state.controller().snapshot().settings);
    state.events().publish(Event::SettingsChanged { settings });
    Ok(crate::dto::SoundFontsDto::from(
        &state.controller().soundfonts(false),
    ))
}

/// Sets what the room gets with no code, written down first and applied second.
///
/// Here rather than in the handler because the owner's page at `/admin/` sets it too. A room is
/// given one of [`crate::Access::ROOM`] and no other level.
pub fn set_room_access(state: &ApiState, room: crate::Access) -> ApiResult<()> {
    if !room.is_room_level() {
        return Err(ApiError::BadRequest(format!(
            "a room is never given the {room} level"
        )));
    }
    state.controller().set_room_access(room)?;
    state.set_room_access(room);
    tracing::info!(%room, "the room access level was changed");
    Ok(())
}

/// Sets or clears the code for one level, written down first and applied second.
///
/// **A code may not be the admin password or the other code.** The login tries the admin password
/// first, so a code equal to it would hand out the admin level. A code equal to the other code would
/// open only the higher of the two, and the owner would not be told.
pub fn set_access_code(
    state: &ApiState,
    level: crate::Access,
    code: Option<&str>,
) -> ApiResult<()> {
    if !level.has_code() {
        return Err(ApiError::BadRequest(format!(
            "the {level} level has no code"
        )));
    }
    let hash = match code.map(str::trim) {
        None => None,
        Some(code) => {
            let min = crate::MIN_PASSWORD_CHARS;
            if code.chars().count() < min {
                return Err(ApiError::BadRequest(format!(
                    "a code wants at least {min} characters"
                )));
            }
            match state.auth().level_of(code)? {
                Some(crate::Access::Admin) => {
                    return Err(ApiError::BadRequest(
                        "that is the admin password; choose a different code".to_owned(),
                    ));
                }
                Some(other) if other != level => {
                    return Err(ApiError::BadRequest(format!(
                        "that is already the {other} code; choose a different one"
                    )));
                }
                _ => {}
            }
            Some(crate::AdminAuth::hash_password(code).map_err(|error| {
                ApiError::Internal(format!("the code could not be stored: {error}"))
            })?)
        }
    };
    state.controller().set_access_code(level, hash.clone())?;
    state.auth().set_code(level, hash);
    tracing::info!(%level, set = state.auth().code_set(level), "an access code was changed");
    Ok(())
}
