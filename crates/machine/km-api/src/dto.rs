//! The wire format.
//!
//! Every request body and response body the API speaks, defined here and nowhere else. That costs a
//! layer of mapping from the internal types, and buys two things worth more than the layer: the
//! whole public contract is readable in one file, and renaming a field inside `km-audio` or
//! `km-catalog` cannot silently change what a remote sees.
//!
//! Conventions: `snake_case` keys, milliseconds for time, `null` for "not applicable" rather than a
//! sentinel, and every collection response wrapped in an object so a field can be added later
//! without breaking a client that expected an array.
//!
//! **Every type here derives both halves of serde**, whichever direction it actually travels in.
//! That is not symmetry for its own sake: `km-remote` is a client of this API written in this
//! workspace, so it parses the responses this server only ever sends and sends the requests this
//! server only ever parses. The alternative is a second set of structs describing the same wire
//! format, free to drift from this file — exactly the failure the first paragraph exists to prevent,
//! reintroduced from the other end. `tools/cmd/km-pack` being a library as well as a command is the same
//! argument, already settled once.
//!
//! One consequence worth knowing: a request type with `deny_unknown_fields` serializes its absent
//! fields as `null` rather than omitting them. That round-trips correctly — `null` reads back as
//! `None` — but it means a patch on the wire names every field it could have set.

use km_catalog::search::{MAX_LIMIT, SortOrder};
use km_catalog::{CatalogSong, DuplicateContent, InstallReport, InstalledPackage};
use km_queue::Transport;
use km_queue::mics::{MicChannel, MicPatch};
use km_queue::queue::QueueEntry;
use km_song::Song;
use km_song::timeline::LyricGranularity;
use km_songcode::SongCode;
use serde::{Deserialize, Serialize};

use crate::machine::{
    AudioOutputs, DemoState, NowPlaying, Origin, OutputLevel, Settings, SettingsPatch, Snapshot,
    SoundFontBanks, SoundFontChoice, SoundFontStatus, SoundKind, WallpaperSource, WallpaperState,
};

/// What the transport is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportDto {
    /// Nothing loaded.
    Idle,
    /// Playing.
    Playing,
    /// Loaded and positioned, not advancing.
    Paused,
    /// Loaded and positioned at the start.
    Stopped,
}

impl From<Transport> for TransportDto {
    fn from(transport: Transport) -> Self {
        match transport {
            Transport::Idle => Self::Idle,
            Transport::Playing => Self::Playing,
            Transport::Paused => Self::Paused,
            Transport::Stopped => Self::Stopped,
        }
    }
}

/// Where the loaded song came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OriginDto {
    /// Queued from the catalog.
    Catalog {
        /// The queueing number.
        number: SongCode,
        /// The queue entry it came from.
        entry_id: u64,
    },
    /// Loaded straight from disk, bypassing the catalog.
    File {
        /// Where from.
        path: String,
    },
    /// Chosen by the machine itself, because nobody was singing.
    ///
    /// Carries no `entry_id`: a demo song was never queued. A client showing this should say that
    /// **skipping**, not waiting, is what lets somebody sing — see [`Origin::Demo`].
    Demo {
        /// The queueing number.
        number: SongCode,
    },
}

impl From<&Origin> for OriginDto {
    fn from(origin: &Origin) -> Self {
        match origin {
            Origin::Catalog { number, entry_id } => Self::Catalog {
                number: *number,
                entry_id: *entry_id,
            },
            Origin::File { path } => Self::File { path: path.clone() },
            Origin::Demo { number } => Self::Demo { number: *number },
        }
    }
}

/// The loaded song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NowPlayingDto {
    /// Where it came from.
    pub origin: OriginDto,
    /// Title.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// What it is sung in, as an ISO 639-1 code, or `null`.
    pub language: Option<String>,
    /// Who asked for it.
    pub singer: Option<String>,
    /// `"midi"`, `"video"`, `"cdg"`, `"ultrastar"` or `"lrc"`.
    ///
    /// **The enum, not its spelling.** This was a `String` while `km_kmpkg::SongKind` was an enum
    /// on both sides of the wire, so the remote and the mirror compared string literals where the
    /// compiler could have checked a match. The manifest keeps a `String`-ish openness for its own
    /// reason -- files on disk written by older builds -- and that reason does not reach here: this
    /// is a live payload between two builds that ship together. `#[serde(other)] Unknown` on the
    /// enum keeps the wire just as forgiving.
    pub kind: km_catalog::SongKind,
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// The detected melody channel, `null` when detection abstained.
    pub melody_channel: Option<u8>,
    /// Whether a melody toggle should be offered at all.
    ///
    /// Derived from `melody_channel` rather than left for the client to infer, because getting this
    /// wrong means offering a control that mutes an arbitrary instrument.
    pub melody_available: bool,
    /// Whether a key change should be offered at all.
    ///
    /// False for a video or MP3+G song: there is no key to shift. Carried rather than left to be
    /// inferred, for the same reason as `melody_available` — a client that offers this on a video
    /// offers a control that answers 409, which is worse than not offering it.
    pub transpose_available: bool,
    /// Whether a tempo change should be offered at all. False for a video or MP3+G song.
    pub tempo_available: bool,
    /// Whether the file has lyrics to draw.
    ///
    /// False for a video or MP3+G song. Both certainly have words — a video's are pixels in somebody
    /// else's picture, an MP3+G song's are tiles this application draws itself — but neither has a
    /// timeline to highlight, and CD+G tiles have no character data behind them to search.
    pub has_lyrics: bool,
}

impl From<&NowPlaying> for NowPlayingDto {
    fn from(now: &NowPlaying) -> Self {
        Self {
            origin: OriginDto::from(&now.origin),
            title: now.title.clone(),
            artist: now.artist.clone(),
            language: now.language.clone(),
            singer: now.singer.clone(),
            kind: now.kind,
            duration_ms: now.duration_ms,
            melody_channel: now.melody_channel,
            melody_available: now.melody_channel.is_some(),
            transpose_available: now.kind.is_midi(),
            tempo_available: now.kind.is_midi(),
            has_lyrics: now.has_lyrics,
        }
    }
}

/// Playback settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SettingsDto {
    /// Semitones of transposition.
    pub transpose: i8,
    /// Tempo multiplier.
    pub tempo_ratio: f32,
    /// Whether the guide melody sounds.
    pub melody_enabled: bool,
    /// Backing-track level.
    pub music_volume: f32,
    /// How far ahead of the audio the machine's own display draws the lyric highlight, in
    /// milliseconds. Positive means the lyrics lead.
    pub lyric_offset_ms: i16,
}

impl From<Settings> for SettingsDto {
    fn from(settings: Settings) -> Self {
        // **Destructured rather than field-accessed, and every conversion in this file does the
        // same.** The `Self { … }` literal below already catches one direction: add a field to the
        // DTO and this stops compiling. The other direction had nothing — add a field to the domain
        // type and it simply never reaches the wire, silently, on a surface whose whole job is to
        // carry it. A `let Domain { .. } = value` with no `..` closes that: a new field is a compile
        // error here, and whoever adds it decides *then* whether a client should see it.
        //
        // This is why the plan's "collapse the identity DTOs into one type" was not taken. Only two
        // of the seven are identities — `PictureDto` reduces a reason that names a filesystem path
        // to a bool, `AudioOutputsDto` renames and rewraps, `SoundFontDto` wraps two nested enums —
        // so deleting the twins would have meant deleting boundaries that earn their keep, for a
        // guarantee this pattern gives all seven of them.
        let Settings {
            transpose,
            tempo_ratio,
            melody_enabled,
            music_volume,
            lyric_offset_ms,
        } = settings;
        Self {
            transpose,
            tempo_ratio,
            melody_enabled,
            music_volume,
            lyric_offset_ms,
        }
    }
}

/// A partial settings change.
///
/// `deny_unknown_fields` so a client that misspells `transpose` is told, rather than watching its
/// request succeed and change nothing.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatchDto {
    /// New transposition, in semitones.
    pub transpose: Option<i8>,
    /// New tempo multiplier.
    pub tempo_ratio: Option<f32>,
    /// Whether the guide melody should sound.
    pub melody_enabled: Option<bool>,
    /// New backing-track level.
    pub music_volume: Option<f32>,
    /// New lyric display offset in milliseconds, positive meaning the lyrics lead the audio.
    /// Clamped rather than refused; see `docs/ARCHITECTURE.md`.
    pub lyric_offset_ms: Option<i16>,
}

impl SettingsPatchDto {
    /// Converts to the internal patch, quantising the floats.
    pub fn to_patch(self) -> SettingsPatch {
        SettingsPatch {
            transpose: self.transpose,
            tempo_milli: self.tempo_ratio.map(to_milli),
            melody_enabled: self.melody_enabled,
            music_volume_milli: self.music_volume.map(to_milli),
            lyric_offset_ms: self.lyric_offset_ms,
        }
    }
}

/// `PUT /machine/name`, both ways.
///
/// The same shape going in and coming back, and the reply is **what the machine now says** rather
/// than an echo of the request — the distinction `put_accept_uploads` already makes, and it earns
/// its keep here because the name is tidied on the way in. A client that sent sixty-four bytes of
/// name gets back the sixty-three that were kept, so nothing has to guess what the rules were.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineNameRequest {
    /// What to call this machine on the network.
    pub name: String,
}

/// `GET /locale` and `PUT /machine/locale`, both ways.
///
/// [`MachineNameRequest`]'s shape and for its reason: the reply is **what the machine now says**,
/// so a client that sent `pt` reads back the `pt-BR` it resolved to rather than having to know the
/// matching rules. A tag is refused rather than resolved when nothing matches at all, which is
/// [`crate::handlers::put_machine_locale`]'s branch.
///
/// **A tag rather than an enum on the wire.** `km_locale::Locale` is a closed set this build knows,
/// and the wire is read by programs built from other revisions of it — a machine speaking a locale
/// this client has no name for must deserialize, so that the client can say *some language I do not
/// have* rather than fail the whole request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineLocaleRequest {
    /// The BCP 47 tag the television draws in — `en`, `pt-BR`.
    pub locale: String,
}

/// What an upload did, in a sentence meant for a person.
///
/// **A sentence and not a set of fields**, unlike [`InstallReportDto`] beside it, and the difference
/// is who is asking. That one answers a *program* -- `km-package-builder` reads `songs_added` --
/// while these three routes exist to put a line on the owner's page, and what that line should say
/// differs completely by kind: a package reports songs, a wallpaper reports how many pictures are
/// now in the rotation, a bank reports its name. Three shapes squeezed into one struct would be
/// three optional fields and a client deciding which to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadReportDto {
    /// What happened, worded for the page: `installed "Carols 1999" \u{b7} 16 songs`.
    pub report: String,
}

/// `POST /admin/password`.
///
/// `null` resets the password to a freshly generated PIN, which is `--reset-password`'s meaning said
/// over HTTP. It is `Option` rather than an empty string precisely so that resetting is deliberate:
/// a client that sent `""` by accident gets a refusal about length, not a password nobody knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminPasswordRequest {
    /// The new password, or `null` to reset it to a freshly generated PIN.
    ///
    /// **`null` no longer removes it.** There is no state with no password to remove it *to*; what
    /// it means now is "I have forgotten mine", and the machine answers with a new PIN on its own
    /// screen. Still `null` rather than an empty string, for the reason it always was: a form
    /// submitted by accident with nothing typed must not be the destructive act.
    pub password: Option<String>,
}

/// What the machine says about its door afterwards.
///
/// **Never the password an owner typed**, which is the whole reason this is not an echo: a reply
/// carrying what was just sent would put it in a proxy log, a browser cache and this crate's own
/// test output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminPasswordDto {
    /// Whether this machine now has a password. Always true; kept so a client need not infer it.
    pub password_set: bool,
    /// The PIN the machine just generated, when this call asked it to generate one.
    ///
    /// **The one exception to the rule above, and it is not a contradiction.** A caller that asked
    /// for a reset has no other way to learn the new PIN over the wire, and the alternative — go and
    /// read it off the television — is exactly what somebody resetting a password remotely cannot
    /// do. `None` whenever an owner supplied their own, which is the case the rule is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factory_pin: Option<String>,
}

/// `PUT /demo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDemoRequest {
    /// Whether the machine should perform for itself when nobody is singing.
    pub enabled: bool,
    /// Whether to write the answer into settings so it survives a restart.
    ///
    /// **Defaults to `false`, so the plain body changes the running machine and nothing else.** That
    /// is the safer default of the two: a switch that lasted only for the evening is a smaller
    /// surprise than one that turned out to be permanent, and an owner who means it says so.
    #[serde(default)]
    pub persist: bool,
}

/// `PUT /admin/demo/delay`.
///
/// **One field, and no `persist` beside it.** The delay is installation configuration and is always
/// written down — see [`crate::machine::Controller::set_demo_delay`] for why it is not a third field
/// on [`SetDemoRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDemoDelayRequest {
    /// Seconds of silence before a demo song starts. `0` means "as soon as the machine is idle".
    pub delay_secs: u32,
}

/// `GET /demo`, and the answer to `PUT /demo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemoDto {
    /// Whether demo mode is on for this run.
    pub enabled: bool,
    /// Whether the settings file says so, and so whether it survives a restart.
    pub stored: bool,
    /// Seconds of silence before a demo song starts. `0` means "as soon as the machine is idle".
    pub delay_secs: u32,
    /// The suitability floor a demo song must clear, out of ten, or `null` for no filter.
    pub min_suitability: Option<u8>,
    /// Whether what is loaded right now is a demo song.
    pub playing: bool,
    /// Seconds until a demo song starts, or `null` when none is due — see [`DemoState`].
    ///
    /// [`DemoState`]: crate::machine::DemoState
    pub starts_in_secs: Option<u32>,
}

impl From<&DemoState> for DemoDto {
    fn from(demo: &DemoState) -> Self {
        // Destructured; see `From<Settings> for SettingsDto`.
        let DemoState {
            enabled,
            stored,
            delay_secs,
            min_suitability,
            playing,
            starts_in_secs,
        } = *demo;
        Self {
            enabled,
            stored,
            delay_secs,
            min_suitability,
            playing,
            starts_in_secs,
        }
    }
}

/// `POST /transport/seek`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeekRequest {
    /// Milliseconds from the start.
    pub ms: u32,
}

/// The whole machine state, in one read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateDto {
    /// Playing, paused, stopped or idle.
    pub transport: TransportDto,
    /// The loaded song.
    pub now_playing: Option<NowPlayingDto>,
    /// Position within it.
    pub position_ms: u32,
    /// Songs waiting.
    pub queue_len: usize,
    /// Current settings.
    pub settings: SettingsDto,
}

impl From<&Snapshot> for StateDto {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            transport: snapshot.transport.into(),
            now_playing: snapshot.now_playing.as_ref().map(NowPlayingDto::from),
            position_ms: snapshot.position_ms,
            queue_len: snapshot.queue_len,
            settings: snapshot.settings.into(),
        }
    }
}

/// A catalog song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SongDto {
    /// The queueing number — how a singer asks for it.
    pub number: SongCode,
    /// Title.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// What the song is sung in, as an ISO 639-1 code — `"pt"`, `"ja"` — or `null`.
    ///
    /// Packaging settles it to a code, so `?language=` can narrow to one — whatever the file wrote
    /// is `ENGL` beside `eng` beside `PORT` on a real corpus. `null` means nobody said and nothing
    /// could be worked out, which a package built by this version cannot contain; `"und"` is a
    /// curator saying they looked and could not tell.
    pub language: Option<String>,
    /// `"midi"` or `"video"`.
    ///
    /// A video song plays and queues like any other, and has none of the adjustments a MIDI song
    /// has — see [`NowPlayingDto`], which says so per control rather than leaving a client to work
    /// it out from this.
    ///
    /// The enum, for [`NowPlayingDto::kind`]'s reason.
    pub kind: km_catalog::SongKind,
    /// Length in milliseconds.
    pub duration_ms: u32,
    /// The 0-10 suitability.
    ///
    /// `null` only when the package predates it. A video or MP3+G song reports **10**: suitability
    /// asks how good a file is as a karaoke source, and those were manufactured to be sung to.
    /// See the `Suitability, for a song that was made to be sung to` decision in `docs/decisions/`.
    ///
    /// Nothing rewrites an installed catalog, so a package whose videos were written as `null`
    /// reports `null` until it is rebuilt.
    pub suitability: Option<u8>,
    /// Whether a melody channel was confidently detected.
    pub melody_available: bool,
    /// Default transposition stored with the song.
    pub default_transpose: i8,
    /// Which package it came from.
    pub package_id: String,
    /// Hash of the song's content, as the package recorded it.
    ///
    /// **Here so that a favorite can rejoin its song by what the song *is*, rather than by the
    /// number it happened to be dialled under.** A number carries a bank, and a bank is assigned by
    /// the machine rather than by the package — so it moves when the owner re-banks a package, when
    /// two packages collide on one machine and not another, and when a rebuild re-flows the slots.
    /// The pair `(package_id, content_hash)` moves for none of those, which is what the offline
    /// remote's favorites now rejoin on.
    ///
    /// **A candidate key with the package id, and not on its own.** Two songs in one package with
    /// the same hash are a `ManifestProblem::DuplicateContent`, which is a hard refusal in both
    /// `km_kmpkg::Package::open` and `PackageBuilder::write`; two songs in *different* packages with
    /// the same hash are legitimate and merely reported. So the pair identifies a song and the hash
    /// alone identifies a recording, and a resolver wanting the second must pick between the rows
    /// it finds rather than assume there is one.
    ///
    /// `None` for a song whose package predates the field or whose manifest was written by hand —
    /// `Manifest::problems` skips those in its own duplicate check, so anything reading this must
    /// fall back to the number rather than treat a missing hash as a mismatch.
    ///
    /// **`serde(default)` is load-bearing for the reason given on [`Self::lyric_preview`] below**,
    /// and `skip_serializing_if` for the other one: the export pages five thousand rows at a time
    /// and thirty-two hex characters a row is not free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// The song's first line or two, when its package carries them.
    ///
    /// Empty for a video and an MP3+G song — their words are pixels — and for a song whose package
    /// carries none.
    ///
    /// **`serde(default)` is load-bearing here and is not decoration.** This type is not only a
    /// response: `km-remote-core` deserializes it back out of `GET /api/v1/songs/export`, one row
    /// per line, and refuses the whole page if a line will not parse. `skip_serializing_if` leaves the
    /// key out of a song with no preview — the export pages five thousand rows at a time, and such a
    /// song should not spend bytes saying so — so the reader needs the default to read that row at
    /// all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lyric_preview: Vec<String>,
    /// What somebody filed this song under, sorted — `["brasil", "rock"]`.
    ///
    /// Empty is the ordinary case: nothing detects a tag, so a song has one only because a person
    /// said so in the builder.
    ///
    /// **`serde(default)` is load-bearing for the reason given on [`Self::lyric_preview`] above**:
    /// the offline remote reads this type back out of the export, and `skip_serializing_if` leaves
    /// the key out of every untagged song.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl From<&CatalogSong> for SongDto {
    fn from(song: &CatalogSong) -> Self {
        Self {
            number: song.number,
            title: song.title.clone(),
            artist: song.artist.clone(),
            language: song.language.clone(),
            kind: song.kind,
            duration_ms: song.duration_ms,
            suitability: song.suitability,
            melody_available: song.melody_channel.is_some(),
            default_transpose: song.default_transpose,
            package_id: song.package_id.clone(),
            content_hash: song.content_hash.clone(),
            lyric_preview: song.lyric_preview.clone(),
            tags: song.tags.clone(),
        }
    }
}

/// A page of search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    /// The matches.
    pub songs: Vec<SongDto>,
    /// How many were skipped.
    pub offset: usize,
    /// The limit actually applied, after capping at [`MAX_LIMIT`].
    ///
    /// Reported rather than echoed, so a client that asked for a million rows can see it did not
    /// get them instead of concluding the catalog is small.
    pub limit: usize,
    /// Whether another page may exist.
    ///
    /// `true` when the page came back full. A cheap "probably more" beats a `COUNT(*)` over an
    /// FTS match on a catalog with hundreds of thousands of rows.
    pub more: bool,
}

impl SearchResponse {
    /// Builds a page from the rows the catalog returned.
    pub fn new(songs: &[CatalogSong], offset: usize, limit: usize) -> Self {
        Self {
            songs: songs.iter().map(SongDto::from).collect(),
            offset,
            limit,
            more: songs.len() >= limit,
        }
    }
}

/// How a search should be ordered, as spelled on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDto {
    /// Best text match first.
    #[default]
    Relevance,
    /// By title.
    Title,
    /// By artist, then title.
    Artist,
    /// By number.
    Number,
    /// Highest suitability first.
    ///
    /// **`suitability` on the wire, and nothing else.** `sort=score` was accepted for a while, on
    /// the reasoning that the old spelling ends up in bookmarks and shell scripts — which is true of
    /// a product somebody has, and this one has never been released. The alias was keeping a second
    /// spelling alive for nobody, and the one place it demonstrably reached was
    /// `tools/dev/remote/api-walkthrough.sh`, which went on teaching the retired name because it
    /// still worked. See `No compatibility aliases` in docs/decisions/songs.md.
    Suitability,
}

impl From<SortDto> for SortOrder {
    fn from(sort: SortDto) -> Self {
        match sort {
            SortDto::Relevance => Self::Relevance,
            SortDto::Title => Self::Title,
            SortDto::Artist => Self::Artist,
            SortDto::Number => Self::Number,
            SortDto::Suitability => Self::Suitability,
        }
    }
}

/// `GET /songs` query string.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SearchParams {
    /// Free text over title and artist.
    pub q: Option<String>,
    /// Restrict to one artist.
    pub artist: Option<String>,
    /// Restrict to one language, as an ISO 639-1 code (`pt`, `ja`, `und`). Matched exactly.
    pub language: Option<String>,
    /// Restrict to songs carrying **any** one of these tags — `?tags=rock,brasil`.
    ///
    /// **One comma-joined value, never a repeated key.** `axum::extract::Query` is
    /// `serde_urlencoded`, which answers `?tags=rock&tags=brasil` with a 400 — so the choice is not
    /// between two spellings that both work. A comma is unambiguous by construction: a tag cannot
    /// contain one, because `km_kmpkg::Tag::parse` folds it to a word break.
    ///
    /// A word that folds to nothing is dropped rather than refused, the same judgement `language`
    /// makes about a code nobody has: a filter naming one real tag and one typo answers with the
    /// real tag's songs, not with an error in the middle of a search.
    pub tags: Option<String>,
    /// Minimum suitability, 0 to 10.
    ///
    /// `min_score` is not read, for the reason [`SortDto::Suitability`] gives about `sort`.
    pub min_suitability: Option<u8>,
    /// Only songs with a detected melody channel.
    pub melody_only: Option<bool>,
    /// Ordering.
    pub sort: Option<SortDto>,
    /// Page size.
    pub limit: Option<usize>,
    /// How many to skip.
    pub offset: Option<usize>,
}

/// One syllable of a lyric line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyllableDto {
    /// When it starts.
    pub start_ms: u32,
    /// When it ends.
    pub end_ms: u32,
    /// The text, already decoded and with break markers stripped.
    pub text: String,
}

/// One lyric line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLineDto {
    /// Position in the timeline.
    pub index: usize,
    /// Which page it belongs to, for files that mark page breaks.
    pub page: u16,
    /// When the line starts.
    pub start_ms: u32,
    /// When it ends.
    pub end_ms: u32,
    /// The whole line, for a client that only wants to display it.
    pub text: String,
    /// The syllables, for a client that wants to follow the singing.
    pub syllables: Vec<SyllableDto>,
}

/// A song's lyrics, in milliseconds.
///
/// Converted from ticks here rather than shipped as ticks, because doing it correctly needs the
/// song's tempo map — including any tempo changes inside it — and no remote should have to
/// reimplement that. This is the one place the API does a non-trivial computation on the way out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsDto {
    /// The song's number, or `null` for a file played directly.
    pub number: Option<SongCode>,
    /// Whether timing is fine enough to follow the words.
    pub granularity: LyricGranularity,
    /// The song's length.
    pub duration_ms: u32,
    /// Which karaoke convention the lyrics came from, named for diagnosis.
    pub flavor: String,
    /// The lines.
    pub lines: Vec<LyricLineDto>,
}

impl LyricsDto {
    /// Converts a parsed song's timeline into milliseconds.
    pub fn from_song(number: Option<SongCode>, song: &Song) -> Self {
        let to_ms = |tick: u32| song.tempo_map.tick_to_ms(tick);
        Self {
            number,
            granularity: song.lyrics.granularity(),
            duration_ms: to_ms(song.duration_ticks),
            flavor: format!("{:?}", song.flavor),
            lines: song
                .lyrics
                .lines
                .iter()
                .enumerate()
                .map(|(index, line)| LyricLineDto {
                    index,
                    page: line.page,
                    start_ms: to_ms(line.start_tick),
                    end_ms: to_ms(line.end_tick),
                    text: line.text(),
                    syllables: line
                        .syllables
                        .iter()
                        .map(|syllable| SyllableDto {
                            start_ms: to_ms(syllable.start_tick),
                            end_ms: to_ms(syllable.end_tick),
                            text: syllable.text.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// A queued song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntryDto {
    /// Opaque, never reused. What a remote must use to remove or move an entry — positions shift
    /// under you the moment somebody else queues a song.
    pub id: u64,
    /// Where it sits right now.
    pub position: usize,
    /// The catalog number.
    pub number: SongCode,
    /// Title.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// Who asked.
    pub singer: Option<String>,
}

/// The queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueDto {
    /// The waiting songs, in play order.
    pub entries: Vec<QueueEntryDto>,
    /// How many are waiting.
    pub len: usize,
    /// The cap, so a remote can show "12 of 200" without hard-coding it.
    pub capacity: usize,
}

impl QueueDto {
    /// Builds the queue response, numbering positions as it goes.
    pub fn new(entries: &[QueueEntry]) -> Self {
        Self {
            entries: entries
                .iter()
                .enumerate()
                .map(|(position, entry)| QueueEntryDto {
                    id: entry.id,
                    position,
                    number: entry.number,
                    title: entry.title.clone(),
                    artist: entry.artist.clone(),
                    singer: entry.singer.clone(),
                })
                .collect(),
            len: entries.len(),
            capacity: km_queue::queue::MAX_QUEUED,
        }
    }
}

/// `POST /queue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddToQueueRequest {
    /// The song number, as punched into the machine.
    pub number: SongCode,
    /// Who is singing it, when a remote knows.
    #[serde(default)]
    pub singer: Option<String>,
}

/// What `POST /queue` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedToQueueDto {
    /// The new entry's opaque id.
    pub entry_id: u64,
    /// Where it landed.
    pub position: usize,
    /// What was queued, so a remote need not re-fetch to show it.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
}

/// `POST /queue/{id}/move`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRequest {
    /// Target position, clamped to the queue's bounds rather than rejected — a remote acting on a
    /// stale view should not get an error for asking to move something to position 9 of 7.
    pub to_index: usize,
}

/// A microphone channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicDto {
    /// Stable id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Which hardware input this is, free-form.
    pub device_hint: Option<String>,
    /// Level, 1.0 being unity.
    pub gain: f32,
    /// Reverb amount.
    pub reverb: f32,
    /// Echo amount.
    pub echo: f32,
    /// Whether it is muted.
    pub muted: bool,
}

impl From<&MicChannel> for MicDto {
    fn from(channel: &MicChannel) -> Self {
        Self {
            id: channel.id.clone(),
            name: channel.name.clone(),
            device_hint: channel.device_hint.clone(),
            gain: channel.gain,
            reverb: channel.reverb,
            echo: channel.echo,
            muted: channel.muted,
        }
    }
}

/// The microphones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicsDto {
    /// Every channel, in registration order.
    pub mics: Vec<MicDto>,
    /// Whether any audio processing happens here.
    ///
    /// Always `false`, and sent anyway: a remote showing a reverb slider should be able to say the
    /// setting is advisory and the mixing happens in hardware. See the decision in `docs/decisions/`.
    pub applies_dsp: bool,
}

impl MicsDto {
    /// Builds the response.
    pub fn new(channels: &[MicChannel]) -> Self {
        Self {
            mics: channels.iter().map(MicDto::from).collect(),
            applies_dsp: false,
        }
    }
}

/// `PUT /mics/{id}`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MicPatchDto {
    /// Rename it.
    pub name: Option<String>,
    /// Re-point it at a hardware input.
    ///
    /// **A blank hint takes the hint away**, and that is the only spelling that does: a field this
    /// patch leaves out is a field it keeps, and JSON `null` is how a client spells *left out*.
    /// An emptied box on a page and a hint nobody ever set are the same state.
    pub device_hint: Option<String>,
    /// New gain.
    pub gain: Option<f32>,
    /// New reverb.
    pub reverb: Option<f32>,
    /// New echo.
    pub echo: Option<f32>,
    /// Mute or unmute.
    pub muted: Option<bool>,
}

impl MicPatchDto {
    /// Converts to the internal patch.
    pub fn to_patch(&self) -> MicPatch {
        MicPatch {
            name: self.name.clone(),
            device_hint: self.device_hint.clone(),
            gain_milli: self.gain.map(to_milli),
            reverb_milli: self.reverb.map(to_milli),
            echo_milli: self.echo.map(to_milli),
            muted: self.muted,
        }
    }
}

/// `GET /audio/outputs`, and the answer to `PUT /audio/output`.
///
/// Not `Eq`, alone among the audio types here, because [`AudioLevelDto`] carries decibels as the
/// floats a client wants to read. The domain [`OutputLevel`] keeps hundredths of a decibel as
/// integers and stays comparable, which is where a test asserts on one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioOutputsDto {
    /// Every device that can be chosen, the "follow the system" entry first.
    pub outputs: Vec<AudioOutputDto>,
    /// What settings ask for. `null` means nothing has ever been chosen, which happens once.
    pub selected: Option<String>,
    /// The identifier actually in use.
    ///
    /// Differs from `selected` when the saved device is not present: the machine follows the system
    /// default for that run and leaves the setting alone, so the two disagree on purpose.
    pub active_id: String,
    /// Its name.
    pub active_name: String,
    /// Whether the machine is playing through a second choice.
    pub fell_back: bool,
    /// The level the active output is running at, where it has one.
    ///
    /// `null` where there is nothing to set: an HDMI or S/PDIF output hands the volume to whatever
    /// is downstream, and a build with no mixer to reach says the same thing. A client draws the
    /// control only when this is present, because unlike `changeable` it will not become available
    /// later.
    pub level: Option<AudioLevelDto>,
    /// Whether `PUT /audio/output` would be accepted right now.
    ///
    /// `false` while anything is loaded or queued. Sent so a remote can gray the control out rather
    /// than offering it and collecting a 409.
    pub changeable: bool,
}

/// One output device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioOutputDto {
    /// The identifier to send back in `PUT /audio/output`. Opaque; do not parse it.
    pub id: String,
    /// What to show a person.
    pub name: String,
    /// Whether this entry means "follow whatever the system calls the default".
    pub system_default: bool,
    /// Whether it is a USB interface.
    pub usb: bool,
    /// Whether it is present. A saved device that has been unplugged is listed with this `false`
    /// rather than hidden, so a remote can say "not present" instead of appearing to forget it.
    pub available: bool,
    /// Whether this is the one settings name.
    pub selected: bool,
    /// Whether this is the entry to show for the hardware it addresses.
    ///
    /// A backend can spell one physical output many ways and give every spelling the same
    /// description — on Linux one headphone jack arrives ten times under identical words and the
    /// list runs past thirty rows, which is a chooser nobody can find anything in. `true` marks one
    /// row per output; `false` marks another way of naming one of them.
    ///
    /// **Every row is still a legal `PUT /audio/output`.** This is a hint about presentation, not a
    /// restriction: show the `true` rows, and put the rest behind a "show everything" control for
    /// the person who has a reason to want one. A `false` row can be `selected` — a saved choice is
    /// always shown whatever spelling it is.
    pub preferred: bool,
}

impl From<&AudioOutputs> for AudioOutputsDto {
    fn from(outputs: &AudioOutputs) -> Self {
        // Destructured; see `From<Settings> for SettingsDto`. This one is *not* an identity
        // mapping — `devices` becomes `outputs`, and each element gains a `selected` the domain type
        // does not carry because it is a fact about the list rather than about the device — which is
        // exactly why the type stays and only the exhaustiveness is borrowed.
        let AudioOutputs {
            devices,
            selected,
            active_id,
            active_name,
            fell_back,
            changeable,
            level,
        } = outputs;
        Self {
            outputs: devices
                .iter()
                .map(|device| AudioOutputDto {
                    id: device.id.clone(),
                    name: device.name.clone(),
                    system_default: device.system_default,
                    usb: device.usb,
                    available: device.available,
                    selected: selected.as_deref() == Some(device.id.as_str()),
                    preferred: device.preferred,
                })
                .collect(),
            selected: selected.clone(),
            active_id: active_id.clone(),
            active_name: active_name.clone(),
            fell_back: *fell_back,
            changeable: *changeable,
            level: level.as_ref().map(AudioLevelDto::from),
        }
    }
}

/// The level the active output is running at, in decibels.
///
/// **Decibels on the wire, where the domain type carries hundredths of one.** A client draws a
/// slider and prints a number, and both want the unit a person reads; the integer form exists so
/// the domain type stays comparable. `dto` is where units are translated, the same way
/// `SettingsPatchDto`'s floats become `*_milli` crossing the other way.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioLevelDto {
    /// What it is set to now.
    pub db: f32,
    /// The quietest it goes. Often far below anything audible, so a client drawing a slider over
    /// the whole range puts everything useful at one end of it.
    pub db_min: f32,
    /// The loudest it goes.
    pub db_max: f32,
    /// The smallest move the control can make.
    pub step_db: f32,
}

impl From<&OutputLevel> for AudioLevelDto {
    fn from(level: &OutputLevel) -> Self {
        let OutputLevel {
            db_centi,
            db_min_centi,
            db_max_centi,
            step_centi,
        } = level;
        Self {
            db: *db_centi as f32 / 100.0,
            db_min: *db_min_centi as f32 / 100.0,
            db_max: *db_max_centi as f32 / 100.0,
            step_db: *step_centi as f32 / 100.0,
        }
    }
}

/// `PUT /admin/audio/level`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioLevelRequest {
    /// Where to put it, in decibels.
    ///
    /// Clamped into the control's own range rather than refused, because a client drawing a slider
    /// from an earlier reading can be a step out of date without being wrong.
    pub db: f32,
}

/// `PUT /audio/output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioOutputRequest {
    /// An `id` from `GET /audio/outputs`.
    ///
    /// Required rather than nullable: `null` would have to mean both "follow the system" and "forget
    /// that anything was chosen", and those are different. The system default has an `id` of its own
    /// in the list, so asking for it is the same shape as asking for anything else.
    pub id: String,
}

/// `GET /audio/soundfonts` query string.
///
/// No `deny_unknown_fields`, matching [`SearchParams`] and [`ExportParams`]: an unknown query
/// parameter is ignored rather than refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SoundFontsParams {
    /// Widen `offers` from the shortlist the machine offers to the whole catalog it knows.
    ///
    /// **This is a width, not a permission.** The route is `audio.read` at either width, and
    /// `POST /audio/soundfont/fetch` has never been gated by rank — what the shortlist governs is
    /// what somebody is *shown* without asking. A page for working on the machine asks; the Setup
    /// tab on a phone does not, and gets the nine.
    pub all: Option<bool>,
}

/// `GET /audio/soundfonts` — every bank this machine can be switched to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundFontsDto {
    /// Every bank that could be chosen, the bundled one first and the rest by name.
    pub banks: Vec<SoundFontBankDto>,
    /// Banks the machine knows how to fetch and does not have.
    pub offers: Vec<SoundFontOfferDto>,
    /// What the downloader is doing, if anything. `null` until something is asked for.
    pub fetching: Option<SoundFontFetchDto>,
    /// The `id` of the bank `audio.soundfont` names, or `"bundled"`.
    ///
    /// **What the machine will come back on, not necessarily what is sounding this second.** A debug
    /// slot (`Ctrl+1`…`Ctrl+9`) is run-only by decision and never writes the setting, so during an
    /// A/B the two genuinely differ — and a picker that moved its tick to follow a keypress would be
    /// claiming a choice nobody made. `GET /audio/soundfont` is the one that answers "what is
    /// sounding".
    pub selected: String,
}

/// One bank that can be chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundFontBankDto {
    /// The identifier to send back in `PUT /audio/soundfont`. Opaque; do not parse it.
    pub id: String,
    /// What to show a person: the file as they named it.
    pub name: String,
    /// Size in bytes. Worth showing because these run from 6 MiB to a gigabyte, and because it is
    /// the only thing on the row that hints at why one takes a moment to load and another does not.
    pub bytes: u64,
    /// Whether this is the bank that ships with the machine.
    ///
    /// It is always present, and it is one of the two reasons a bank cannot be removed — see
    /// `removable`, which is the field to branch on. This one is for saying *which* bank it is.
    pub bundled: bool,
    /// Whether `DELETE /audio/soundfonts/{id}` would go through.
    ///
    /// `false` for the bundled bank and for one a `debug.soundfonts` slot names, both of which are
    /// permanent: a remote leaves the delete control off the row rather than offering one that can
    /// only be refused. It is not `!bundled`, which is the bug this field replaces — the slot case
    /// was offered a button and refused with a 400.
    ///
    /// **A boolean, and the machine's sentence stays behind.** Every reason names a full path, and
    /// the directory layout of the machine under the television is nobody's business but the
    /// owner's — the same rule that keeps an archive's path out of [`PackageDto`].
    pub removable: bool,
    /// Whether this is the one `selected` names.
    pub selected: bool,
}

/// One bank the machine could fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundFontOfferDto {
    /// The id to send to `POST /audio/soundfont/fetch`. **A different namespace from an installed
    /// bank's id**, because these identify different things: a row in a table against a file on
    /// disk.
    pub id: String,
    /// The file it would arrive as.
    pub name: String,
    /// Its size in words — `103.4 MiB`.
    pub size: String,
    /// Exact bytes.
    pub bytes: u64,
    /// What was found about its terms, in the file's own words where it states any. Shown on the row
    /// rather than buried: it is one of the things a person is choosing between banks on.
    pub license: String,
    /// One line on how it sounds.
    pub note: String,
    /// Whether the machine can fetch it, or whether it has to be got by hand.
    pub fetchable: bool,
    /// Where to get one the machine cannot fetch.
    pub page: Option<String>,
    /// The one bank the machine suggests. At most one offer carries it.
    ///
    /// **Defaulted so an older client is not broken by it**, which is the same courtesy every other
    /// field added to this surface has had: a client that does not know the key sees the list it
    /// always saw, without a recommendation rather than with a wrong one.
    #[serde(default)]
    pub recommended: bool,
    /// Whether the machine *offers* this bank, or merely knows about it.
    ///
    /// **Always `true` unless `?all=true` was asked for**, since the default list holds nothing
    /// else. It exists so that the wider list can be read: a page showing the whole catalog needs
    /// to say which rows are the shortlist a phone would have been given.
    ///
    /// Defaulted `false` rather than `true`, which looks like the wrong way round and is not: an
    /// older server does not send the key at all, and a client that filled it in as `true` would be
    /// claiming that server had answered a question it was never asked.
    #[serde(default)]
    pub offered: bool,
}

/// A download in progress, or the one that just finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundFontFetchDto {
    /// The table id of the bank being fetched.
    pub id: String,
    /// What to call it on a screen.
    pub name: String,
    /// Whether it is running, finished or failed.
    pub state: SoundFontFetchStateDto,
    /// Bytes so far.
    pub done: u64,
    /// Bytes expected.
    pub total: u64,
    /// Why it failed, in words for a person.
    pub problem: Option<String>,
}

/// What a download is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundFontFetchStateDto {
    /// Running now; `done` and `total` say how far.
    Working,
    /// Finished, verified and installed — it is in the `banks` list and can be chosen.
    Done,
    /// Did not finish. `problem` says why, and nothing was installed.
    Failed,
}

/// `POST /audio/soundfont/fetch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundFontFetchRequest {
    /// An `id` from the `offers` list.
    pub id: String,
}

impl From<&SoundFontBanks> for SoundFontsDto {
    fn from(banks: &SoundFontBanks) -> Self {
        Self {
            banks: banks
                .banks
                .iter()
                .map(|bank| SoundFontBankDto {
                    id: bank.id.clone(),
                    name: bank.name.clone(),
                    bytes: bank.bytes,
                    bundled: bank.bundled,
                    removable: bank.why_not_removable.is_none(),
                    selected: bank.id == banks.selected,
                })
                .collect(),
            offers: banks
                .offers
                .iter()
                .map(|offer| SoundFontOfferDto {
                    id: offer.id.clone(),
                    name: offer.name.clone(),
                    size: offer.size.clone(),
                    bytes: offer.bytes,
                    license: offer.license.clone(),
                    note: offer.note.clone(),
                    fetchable: offer.fetchable,
                    page: offer.page.clone(),
                    recommended: offer.recommended,
                    offered: offer.offered,
                })
                .collect(),
            fetching: banks.fetching.as_ref().map(|fetch| SoundFontFetchDto {
                id: fetch.id.clone(),
                name: fetch.name.clone(),
                state: match fetch.state {
                    crate::machine::SoundFontFetchState::Working => SoundFontFetchStateDto::Working,
                    crate::machine::SoundFontFetchState::Done => SoundFontFetchStateDto::Done,
                    crate::machine::SoundFontFetchState::Failed => SoundFontFetchStateDto::Failed,
                },
                done: fetch.done,
                total: fetch.total,
                problem: fetch.problem.clone(),
            }),
            selected: banks.selected.clone(),
        }
    }
}

/// `PUT /audio/soundfont`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundFontRequest {
    /// An `id` from `GET /audio/soundfonts`.
    ///
    /// Required rather than nullable, for the reason [`AudioOutputRequest::id`] gives: the bundled
    /// bank has an id of its own, so asking for it is the same shape as asking for anything else,
    /// and `null` would have to mean two different things.
    pub id: String,
}

/// `GET /audio/soundfont` — which General MIDI bank is playing, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundFontDto {
    /// The bank actually loaded, as a full path. `null` when none is.
    pub path: Option<String>,
    /// `"setting"` when `audio.soundfont` named it, `"bundled"` when the rule found it. `null` when
    /// no bank loaded, because then neither of them chose anything.
    pub chosen_by: Option<SoundFontChoiceDto>,
    /// What the machine is making sound with. Only `"silent"` refuses a song.
    pub playing: SoundKindDto,
    /// Why there is no bank, or no device, in the words the machine would log.
    pub problem: Option<String>,
    /// Why a bank the setting named is not what is playing, on a machine that is otherwise fine.
    ///
    /// Set when `audio.soundfont` names a bank the folder no longer holds: the bundled bank is
    /// playing and everything works, and the setting is stale. **Not `problem`**, which means *why
    /// there is no bank* — a client that showed this there would report a working machine as broken.
    pub fallback: Option<String>,
}

/// How the bank that is playing was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundFontChoiceDto {
    /// `audio.soundfont` in settings named a bank the folder holds.
    Setting,
    /// The first bundled candidate that exists.
    Bundled,
    /// A setting named a bank that is not in the folder, so the bundled one is playing.
    ///
    /// Its own value rather than `"bundled"`, because those are two different machines: one is on
    /// the bundled bank because nobody chose otherwise, this one is on it *despite* a choice.
    Fallback,
}

/// What the machine is making sound with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundKindDto {
    /// A real bank: instruments sound like instruments.
    ///
    /// Renamed explicitly, because `snake_case` over a Rust name spelled with two words gives
    /// `sound_font` — a spelling of the file format nobody uses, on a route called `soundfont`.
    #[serde(rename = "soundfont")]
    SoundFont,
    /// A sine synthesizer standing in. The lyrics still scroll in time.
    TestTone,
    /// No audio device at all; playback is refused.
    Silent,
}

impl From<&SoundFontStatus> for SoundFontDto {
    fn from(status: &SoundFontStatus) -> Self {
        // Destructured; see `From<Settings> for SettingsDto`. Not an identity mapping either -- both
        // `chosen_by` and `playing` are enums that exist twice, once as the machine's vocabulary and
        // once as the wire's, and `SoundKindDto::SoundFont` renames itself for the route it appears
        // on. Those are the boundary doing its job.
        let SoundFontStatus {
            path,
            chosen_by,
            playing,
            problem,
            fallback,
        } = status;
        Self {
            path: path.clone(),
            chosen_by: chosen_by.map(|choice| match choice {
                SoundFontChoice::Setting => SoundFontChoiceDto::Setting,
                SoundFontChoice::Bundled => SoundFontChoiceDto::Bundled,
                SoundFontChoice::Fallback => SoundFontChoiceDto::Fallback,
            }),
            playing: match playing {
                SoundKind::SoundFont => SoundKindDto::SoundFont,
                SoundKind::TestTone => SoundKindDto::TestTone,
                SoundKind::Silent => SoundKindDto::Silent,
            },
            problem: problem.clone(),
            fallback: fallback.clone(),
        }
    }
}

/// The wallpaper cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallpapersDto {
    /// The file name on screen, without its directory.
    pub current: Option<String>,
    /// How many images the folder yielded.
    pub count: usize,
    /// Seconds between changes.
    pub interval_secs: u32,
    /// Whether the order is shuffled.
    pub shuffle: bool,
    /// Whether a song starting changes the picture, on top of the interval.
    ///
    /// **`serde(default)` for the reason [`Self::pictures`] carries one**: `km-admin` reads this
    /// type off a machine that may be an older build, and one absent field must not cost the page
    /// every other thing the machine said. `false` is also the honest reading of a machine that
    /// has never heard of the trigger, so the default is the answer rather than a placeholder.
    #[serde(default)]
    pub on_song_change: bool,
    /// Why there are none, when there are none.
    pub problem: Option<String>,
    /// Which rule chose the folder these come from.
    ///
    /// **The rule's name, never the path.** `current` deliberately carries a bare file name because
    /// the folder is the operator's filesystem layout; which of four rules won is not, and it is the
    /// fastest answer to *why isn't my picture on the screen*. `"bundled"` with a `count` of four
    /// means the owner's folder held nothing when it was last looked at — which is now every cycle
    /// rather than once at startup.
    pub source: WallpaperSourceDto,
    /// The files in the folder, one row each.
    ///
    /// **Files, not images**, so a zip is one entry saying how many pictures it holds — see
    /// [`PictureDto`]. Empty on a host that keeps no folder, and empty is not the same claim as
    /// [`Self::count`] being zero: a machine may well be showing four bundled pictures it will not
    /// list, because they are not the owner's to manage.
    #[serde(default)]
    pub pictures: Vec<PictureDto>,
}

/// One file in the wallpaper folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PictureDto {
    /// Stable identifier, safe to store and to put in a URL.
    pub id: String,
    /// The file name, never a path — for [`WallpapersDto::current`]'s reason.
    pub name: String,
    /// How many images it contributes: one for a picture, an archive's entry count for a zip.
    pub images: usize,
    /// Size in bytes.
    pub bytes: u64,
    /// Whether `DELETE /wallpapers/{id}` would take it.
    ///
    /// **The boolean travels and the sentence does not**, which is the rule
    /// [`SoundFontBankDto::removable`] already keeps: the reason names a path, and a path is the
    /// operator's filesystem layout. The owner's page is in-process and reads the sentence from the
    /// controller directly.
    pub removable: bool,
}

/// Which rule produced the wallpaper folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperSourceDto {
    /// `wallpaper.dir` in settings named it, which beats the three rules below.
    Setting,
    /// The owner's own folder in the data directory.
    Owner,
    /// A checkout's local overlay. Never reachable from an installed build.
    Overlay,
    /// The set that shipped with the build.
    Bundled,
}

impl From<&WallpaperState> for WallpapersDto {
    fn from(state: &WallpaperState) -> Self {
        Self {
            current: state.current.clone(),
            count: state.count,
            interval_secs: state.interval_secs,
            shuffle: state.shuffle,
            on_song_change: state.on_song_change,
            problem: state.problem.clone(),
            source: match state.source {
                WallpaperSource::Setting => WallpaperSourceDto::Setting,
                WallpaperSource::Owner => WallpaperSourceDto::Owner,
                WallpaperSource::Overlay => WallpaperSourceDto::Overlay,
                WallpaperSource::Bundled => WallpaperSourceDto::Bundled,
            },
            pictures: Vec::new(),
        }
    }
}

impl WallpapersDto {
    /// The cycle plus what is in the folder.
    ///
    /// A constructor rather than a wider `From`, for the reason [`PackageDto::new`] is one: the
    /// listing costs a directory read and every zip's central directory, and the two callers that
    /// only want the cycle — the 4 Hz state broadcast is not one of them, but `POST
    /// /wallpapers/next` is — should not pay for it.
    #[must_use]
    pub fn with_pictures(state: &WallpaperState, pictures: Vec<crate::machine::Picture>) -> Self {
        Self {
            pictures: pictures
                .into_iter()
                .map(|picture| {
                    // Destructured; see `From<Settings> for SettingsDto`. The clearest case in the
                    // file for why these types are not collapsed: `why_not_removable` is a sentence
                    // naming a path on the machine's disk, and what crosses is the boolean it
                    // implies. Deleting the twin would have put the path on every phone in the room.
                    let crate::machine::Picture {
                        id,
                        name,
                        images,
                        bytes,
                        why_not_removable,
                    } = picture;
                    PictureDto {
                        id,
                        name,
                        images,
                        bytes,
                        removable: why_not_removable.is_none(),
                    }
                })
                .collect(),
            ..Self::from(state)
        }
    }
}

/// An installed package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDto {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Version string.
    pub version: String,
    /// How many songs it contributed.
    pub song_count: usize,
    /// When it was installed.
    pub installed_at: String,
    /// The block of a thousand its songs are dialled in.
    ///
    /// Included where the prefix it replaces never was, because a bank is genuinely actionable: it
    /// says a package's songs run `bank * 1000 + 1` upwards, so a page can tell somebody where to
    /// look without asking for a song list.
    pub bank: u16,
    /// Whether `DELETE /packages/{id}` would go through.
    ///
    /// `false` for a package reached through `debug.packages` and for one outside every folder the
    /// machine owns — both permanent, so a page leaves the Remove control off the row rather than
    /// offering one that can only be refused. `Catalog::why_not_removable` is what answers it.
    ///
    /// **A boolean, and the machine's sentence stays behind**, for the reason the archive's path
    /// does: every refusal names one. A page running inside the machine reads the sentence in
    /// process and prints it; a remote gets the flag.
    pub removable: bool,
    /// The package header's flags word, unknown bits included.
    ///
    /// `0` from a machine that predates it, which is also what a package with no flag carries.
    #[serde(default)]
    pub flags: u32,
    /// The set bits this machine has a name for, such as `uncurated`, in bit order.
    ///
    /// A page reads these rather than the number. A bit with no name here is still in [`Self::flags`].
    /// See `A package's header carries flags, and an unknown one is kept` in
    /// `docs/decisions/packaging.md`.
    #[serde(default)]
    pub flag_names: Vec<String>,
}

impl PackageDto {
    /// Builds a row, given the one thing the row itself cannot know.
    ///
    /// **A constructor rather than a `From`**, deliberately: `removable` is a question only the
    /// machine can answer, and a conversion that could not ask it would have to default the flag —
    /// which is exactly the silent wrong answer the flag exists to prevent.
    ///
    /// The archive's path is not included: it is the operator's filesystem layout and tells a remote
    /// nothing it can act on.
    pub fn new(package: &InstalledPackage, removable: bool) -> Self {
        Self {
            id: package.id.clone(),
            name: package.name.clone(),
            version: package.version.clone(),
            song_count: package.song_count,
            installed_at: package.installed_at.clone(),
            bank: package.bank,
            removable,
            flags: package.flags.bits(),
            flag_names: package.flags.names().map(str::to_owned).collect(),
        }
    }
}

/// The last component of a path, whichever separator the machine writes.
///
/// Not `Path::file_name`: these strings are produced on the machine and read here, and a Windows
/// path arriving at a Linux build would come back whole, which is exactly the leak this exists to
/// prevent. Both separators are checked for that reason.
pub(crate) fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// A package the machine found and refused to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageProblemDto {
    /// The file's own name, **not** the path it sits at.
    ///
    /// The same rule [`PackageDto`] states for an installed package, and it matters more here: this
    /// one reaches a banner on every phone in the room, and the directory layout of the machine
    /// under the television is nobody's business but the owner's. The name is what identifies which
    /// file to go and fix, and the full path is in the machine's own log for whoever is at it.
    pub file: String,
    /// Its identifier, when it opened far enough to have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    /// What went wrong.
    ///
    /// **One sentence.** A song number carries its package's bank inside it and a bank is unique per
    /// package, so two packages cannot claim one number — the only faults are a package that will
    /// not open and a bank already taken, and each of those is one sentence.
    pub reason: String,
}

impl From<&crate::machine::PackageProblem> for PackageProblemDto {
    fn from(problem: &crate::machine::PackageProblem) -> Self {
        // Destructured; see `From<Settings> for SettingsDto`. `path` becomes `file` on purpose --
        // see the field's own doc: a full path is the owner's directory layout and this reaches a
        // banner on every phone in the room.
        let crate::machine::PackageProblem {
            path,
            package_id,
            reason,
        } = problem;
        Self {
            file: file_name_of(path),
            package_id: package_id.clone(),
            reason: reason.clone(),
        }
    }
}

/// `PUT /packages/{id}/bank`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BankRequest {
    /// The block of a thousand to move this package's songs into, 1 to `MAX_BANK`.
    ///
    /// Range-checked by the handler rather than by a type, so a bank outside the range is a 400
    /// naming the limit rather than a serde message about an unparseable field.
    pub bank: u16,
}

/// What moving a package did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankDto {
    /// The package.
    pub package_id: String,
    /// The bank it is in now.
    pub bank: u16,
    /// How many songs changed their number.
    pub songs_renumbered: usize,
}

/// The installed packages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackagesDto {
    /// Every package.
    pub packages: Vec<PackageDto>,
    /// Songs across all of them.
    pub song_count: usize,
    /// Packages that are present and were **not** installed, with the reason for each.
    ///
    /// Reported beside the successes rather than at a route of its own: "what songs has this
    /// machine got?" and "what did it fail to take?" are the same question asked by anybody looking
    /// at a catalog that is missing an album. Empty in the ordinary case and omitted from the
    /// JSON entirely, so nothing changes for a machine with nothing wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<PackageProblemDto>,
}

/// `POST /packages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRequest {
    /// A path on the machine's own disk. Not an upload: packages are hundreds of megabytes and the
    /// operator is standing at the machine.
    pub path: String,
}

/// The same recording under two numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateDto {
    /// The number just installed.
    pub number: SongCode,
    /// The number that already had this content.
    pub existing_number: SongCode,
    /// Which package that one came from.
    pub existing_package: String,
}

impl From<&DuplicateContent> for DuplicateDto {
    fn from(duplicate: &DuplicateContent) -> Self {
        Self {
            number: duplicate.number,
            existing_number: duplicate.existing_number,
            existing_package: duplicate.existing_package.clone(),
        }
    }
}

/// What one reading of the packages folders did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RescanReportDto {
    /// How many packages went in, new and re-indexed alike.
    pub installed: usize,
    /// Packages whose songs left the catalog, by id.
    pub removed: Vec<String>,
    /// Packages gone from the folders that are still in the catalog for now.
    ///
    /// **Non-empty is not a failure.** It means something was playing or queued, so removing rows a
    /// queued number points at was declined; they go at the next idle rescan or the next start.
    /// Reported rather than held back silently, because a client otherwise cannot tell "nothing was
    /// missing" from "not yet".
    pub deferred: Vec<String>,
    /// How many packages the machine is complaining about afterwards.
    ///
    /// `GET /api/v1/packages` carries the reasons; this is the count, so a client knows whether to
    /// go and look.
    pub problems: usize,
}

impl From<&crate::machine::RescanReport> for RescanReportDto {
    fn from(report: &crate::machine::RescanReport) -> Self {
        // Destructured; see `From<Settings> for SettingsDto`. This one *is* a true identity mapping
        // — one of only two among the seven — and it is kept as a separate type anyway, because the
        // alternative is putting serde on `machine::RescanReport` and so on the trait module that
        // defines what a machine can do. A four-field struct is not worth that.
        let crate::machine::RescanReport {
            installed,
            removed,
            deferred,
            problems,
        } = report;
        Self {
            installed: *installed,
            removed: removed.clone(),
            deferred: deferred.clone(),
            problems: *problems,
        }
    }
}

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallReportDto {
    /// The package's id.
    pub package_id: String,
    /// The package's human-readable name, and what [`Self::sentence`] quotes.
    ///
    /// Empty for a manifest that names nothing, which [`Self::sentence`] answers with the id.
    pub package_name: String,
    /// Songs added.
    pub songs_added: usize,
    /// Whether this replaced an earlier install of the same package.
    pub replaced_existing: bool,
    /// Songs whose content already existed under another number. Reported, not refused.
    pub duplicate_content: Vec<DuplicateDto>,
}

impl InstallReportDto {
    /// What happened, worded for a person: `installed "Carols 1999" · 16 songs`.
    ///
    /// **The name in the quotes and not the id.** An id is sixteen hexadecimal characters from the
    /// operating system's entropy, which tells somebody standing at the machine nothing about what
    /// went in; the readable half is what every surface shows — see `A package's id is generated,
    /// not typed` in `docs/decisions/packaging.md`. The id is behind it for the manifest that names
    /// nothing, because a package with no label still has to be reported as *something*.
    ///
    /// The name is quoted as it was typed rather than folded through `km_kmpkg::name_slug`: that
    /// fold exists to make a file name, and this is a sentence.
    ///
    /// **The one place that sentence is written**, and it was written three times before this: the
    /// drop path in `karaokemachine::dropped`, the upload route in `karaokemachine::machine`, and —
    /// once `km-package-builder` stopped printing the raw JSON body at whoever pressed Install — it
    /// would have been written a third time there. It lives here because this is the type all three
    /// already have or can cheaply get: the two in the machine hold an [`InstallReport`] and
    /// `From<&InstallReport>` is directly below, and the builder decodes this DTO off the wire.
    ///
    /// It is the twin of [`UploadReportDto`]'s `report` field rather than a competitor to it. That
    /// one is the sentence a machine composes for a client that sent it bytes; this is the same
    /// sentence composed by whoever is holding the fields. Both roads to an install therefore say
    /// the same thing, which is what stops the message depending on whether the machine happened to
    /// be on this box.
    pub fn sentence(&self) -> String {
        let verb = if self.replaced_existing {
            "updated"
        } else {
            "installed"
        };
        let songs = self.songs_added;
        let plural = if songs == 1 { "song" } else { "songs" };
        let name = self.package_name.trim();
        let label = if name.is_empty() {
            self.package_id.as_str()
        } else {
            name
        };
        format!("{verb} \"{label}\" \u{b7} {songs} {plural}")
    }
}

impl From<&InstallReport> for InstallReportDto {
    fn from(report: &InstallReport) -> Self {
        Self {
            package_id: report.package_id.clone(),
            package_name: report.package_name.clone(),
            songs_added: report.songs_added,
            replaced_existing: report.replaced_existing,
            duplicate_content: report
                .duplicate_content
                .iter()
                .map(DuplicateDto::from)
                .collect(),
        }
    }
}

/// What an uninstall did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallDto {
    /// The package that went.
    pub package_id: String,
    /// How many songs went with it.
    pub songs_removed: usize,
}

/// `POST /admin/login`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// The shared admin password.
    pub password: String,
}

/// A granted session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginResponse {
    /// The bearer token to send as `Authorization: Bearer <token>`.
    pub token: String,
    /// How long it lasts.
    pub expires_in_secs: u64,
}

/// The reply to `POST /login`: a token, and the level it opens.
///
/// The request is a [`LoginRequest`], because the admin password is one of the things it takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGrantDto {
    /// The bearer token to send as `Authorization: Bearer <token>`.
    pub token: String,
    /// How long it lasts.
    pub expires_in_secs: u64,
    /// The level the token opens: `queue`, `control` or `admin`.
    pub access: crate::access::Access,
}

/// `GET /access`, and the reply to the three routes under `/admin/access`.
///
/// **Whether each code is set, and never the code.** A remote offers a code box only where a code
/// exists, and the owner's page says *set* or *not set*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessDto {
    /// The caller's own level: the higher of the room's and the token's.
    pub access: crate::access::Access,
    /// What everybody gets with no code at all.
    pub room: crate::access::Access,
    /// Whether the owner has set a queue code.
    pub queue_code: bool,
    /// Whether the owner has set a control code.
    pub control_code: bool,
}

/// `PUT /admin/access`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomAccessRequest {
    /// `view`, `queue` or `control`. A room is never given `admin`.
    pub room: crate::access::Access,
}

/// `PUT /admin/access/queue-code` and `PUT /admin/access/control-code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessCodeRequest {
    /// The new code, or `null` to clear it. A cleared code ends every token it granted.
    pub code: Option<String>,
}

/// `GET /debug` and the reply to `PUT /admin/debug`.
///
/// **`enabled` rather than `accept`**, unlike the upload switch this replaced: that one had the
/// machine agreeing to take something, where this turns a whole surface on. When it is off the two
/// debug routes are not mounted at all and the whole `debug.` section of settings is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugDto {
    /// Whether debugging mode is on **for this run**.
    pub enabled: bool,
    /// Whether the settings file says so, and so what it will be after a restart.
    ///
    /// **The `enabled`/`stored` pair [`DemoDto`] already uses, and this route needed it for a
    /// sharper reason than that one does.** The two debug routes are mounted at
    /// router-construction time, so a change takes effect at the next start — and the running value
    /// is a snapshot taken then, which meant a switch drawn from `enabled` alone did not move when
    /// it was pressed. A control that reports the state it is *not* changing is a puzzle rather than
    /// a switch.
    pub stored: bool,
}

/// `GET /admin/power`.
///
/// **Both fields are `true` whenever this answers at all**, and it is still two fields rather than
/// an empty body. A machine with no power control does not have this route — it answers 404, which
/// is what tells a client there is nothing here — so the body exists to be *read* by a client that
/// already knows the route is there, and a shape with room for one of the two to become false is
/// what stops that client having to be rewritten if one ever does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerDto {
    /// Whether the box can be asked to turn itself off.
    pub shutdown: bool,
    /// Whether the application can be ended and started again.
    pub restart: bool,
}

/// `PUT /admin/debug`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebugRequest {
    /// What to set it to.
    pub enabled: bool,
}

/// `GET /dev-remote` and the reply to `PUT /admin/dev-remote`.
///
/// **Two fields and not one, because this switch is half of a condition.** The console is served
/// only when debugging mode is on as well, so a switch reporting its own position alone would leave
/// an owner who had turned it on looking at a `/dev/` that answers 404 with nothing on any page
/// explaining why. `served` is what actually happened; `enabled` is what this switch says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevRemoteDto {
    /// Whether the switch is on, as the settings file holds it. What a control toggles.
    pub enabled: bool,
    /// Whether the console and its API are actually up this run — this switch **and** debugging.
    pub served: bool,
}

/// `PUT /admin/dev-remote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevRemoteRequest {
    /// What to set it to.
    pub enabled: bool,
}

/// `GET /performance` and the reply to `PUT /admin/performance`.
///
/// **One field, unlike the two switches beside it, and the single field is the point.** Those decide
/// which routes get mounted and so report a running value and a stored one that can disagree; this
/// takes effect on the next frame and is never written down, so there is only ever one answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceDto {
    /// Whether the frame-statistics panel is on the machine's screen.
    pub enabled: bool,
}

/// `PUT /admin/performance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceRequest {
    /// What to set it to.
    pub enabled: bool,
}

/// `POST /debug/play-file`.
///
/// `PartialEq` without `Eq` since the corrections arrived: a fix this build cannot read is held as
/// the JSON it came in as, and JSON carries numbers, which are not `Eq`. Nothing compares these for
/// total equality — the derive was there because every field happened to allow it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayFileRequest {
    /// A path on the machine's own disk.
    pub path: String,
    /// The corrections to play it with, in place of whatever this machine would detect.
    ///
    /// **Absent is not the same as empty.** Absent means nobody has decided, so the machine detects
    /// as it does for any loose file; an empty list means somebody decided there should be none. A
    /// curation tool that could only send a list would make a song with its corrections turned off
    /// sound exactly like a song nobody had touched, which is the one thing the person listening is
    /// trying to tell apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixes: Option<Vec<km_fixes::Fix>>,
    /// The title to show, in place of whatever the file calls itself.
    ///
    /// A corpus file's own title is frequently the arranger's, an abbreviation, or absent — which is
    /// why a curator retypes it. A preview showing the file's version says one thing on the tool's
    /// page and another on the television, and the person checking is looking at both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The performer to show, on the same terms as [`Self::title`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    /// The key to play it in, as the song's own stored transposition in semitones.
    ///
    /// Treated exactly as a packaged song's is, so the operator's own default is added on top. A
    /// curator checking whether two semitones down is enough is listening for the key the song
    /// would be in once it is in a package, not for the file's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transpose: Option<i8>,
    /// The melody channel to play it with, 0-based, in place of whatever this machine would detect.
    ///
    /// **Absent, `null` and a number are three answers.** Absent detects; `null` says the song has
    /// no melody channel; a number names one. The machine offers the guide-melody toggle only on a
    /// song with a channel, so a detector that abstained leaves the preview without one until a
    /// curator's choice arrives here.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub melody: Option<Option<u8>>,
    /// The words of an UltraStar or LRC song, already read out of its lyrics file by the sender.
    ///
    /// Present only when `path` is the song's MP3. The machine never reads either file, so the
    /// timeline arrives in the form a package stores it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<km_song::LyricTimeline>,
    /// Which file [`Self::lyrics`] was read from: `"ultrastar"` or `"lrc"`.
    ///
    /// Absent is `"ultrastar"`, so a sender that names no kind plays its words as an UltraStar
    /// song. It decides only what the song is called, and which refusal a key change gets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics_kind: Option<km_catalog::SongKind>,
    /// Play the song and draw none of its words, in place of whatever this machine would detect.
    ///
    /// **Absent is not the same as `false`**, on [`Self::fixes`]'s terms: absent leaves the machine
    /// to measure the file as it does for any loose file, and `false` is a curator saying the words
    /// are to be drawn on a song that measurement silences. A preview is for seeing what the
    /// packaged song will look like, and a curator who has just overruled detection is checking
    /// precisely that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics_hidden: Option<bool>,
}

/// Reads a key that is present, so `null` arrives as `Some(None)` rather than as the key's absence.
///
/// Absence is `#[serde(default)]`'s job: this runs only when the key is there.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// One line the machine said, as a reader of its log sees it.
///
/// **The time is a number and not a stamp.** Whoever draws this has a clock and a locale; the
/// machine has neither to spare and no date library to get them, and a page turning milliseconds
/// into a local time is one line of JavaScript.
///
/// **`fields` is `null` rather than an empty string** when the event carried nothing but its
/// message, which is the common case — the file's `null` for "not applicable" convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecordDto {
    /// Which record this is, counting from one for the life of the run.
    ///
    /// **What joins the tail to the live stream.** A reader is given both and they overlap on
    /// purpose; this is how it drops the overlap rather than drawing a line twice.
    pub seq: u64,
    /// When it was said, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// How serious it is: `error`, `warn`, `info`, `debug` or `trace`.
    pub level: String,
    /// Which crate or module said it, spelled as a filter directive would name it.
    pub target: String,
    /// What was said, without the fields.
    pub message: String,
    /// The event's other fields as `name=value` pairs, or `null` where it had none.
    pub fields: Option<String>,
}

impl From<&km_logtap::Record> for LogRecordDto {
    fn from(record: &km_logtap::Record) -> Self {
        Self {
            seq: record.seq,
            at_ms: record.at_ms,
            // **The wire spelling is decided here and not in `km-logtap`.** That crate keeps a
            // `tracing::Level`, which is the honest type for a level; how one is spelled to a
            // client is this file's business, and lower case is what every other enum here uses.
            level: record.level.as_str().to_lowercase(),
            target: record.target.to_owned(),
            message: record.message.clone(),
            fields: (!record.fields.is_empty()).then(|| record.fields.clone()),
        }
    }
}

/// `GET /admin/logs` — what the machine has kept.
///
/// **`dropped` is what says this is a tail and not the run.** A reader with five hundred records
/// and no count cannot tell a machine that has just started from one that has been running all
/// evening, and the difference decides whether looking further back is worth doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogTailDto {
    /// The records held, oldest first.
    pub records: Vec<LogRecordDto>,
    /// How many the machine keeps at once.
    pub capacity: usize,
    /// How many have fallen off the front since the machine started.
    pub dropped: u64,
    /// The filter directive this run is keeping records under, where the machine said.
    ///
    /// **What stops a quiet pane being a mystery.** A machine started without `-v` keeps nothing
    /// below `info`, and a reader looking for `debug` lines that were never taken needs to be told
    /// that rather than left to conclude the feature is broken.
    pub filter: Option<String>,
}

/// One frame of `GET /admin/logs/stream`.
///
/// **A tagged frame rather than a bare record**, so that falling behind can be said out loud. The
/// alternative was a synthetic record carrying the notice, and that forges a line the machine never
/// emitted: indistinguishable from a real one the moment somebody pastes the pane into a report,
/// and filtered away by a reader that is hiding everything below `warn`. The machine's event stream
/// draws the same distinction with `desync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LogFrameDto {
    /// Something the machine said.
    Record {
        /// The line.
        record: LogRecordDto,
    },
    /// Records went past while this reader was behind, and are gone.
    Lagged {
        /// How many were missed.
        skipped: u64,
    },
}

/// An error, as a client sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDto {
    /// A stable machine-readable code — `not_found`, `queue_full`, `unauthorized`.
    pub error: String,
    /// A sentence for a human. Not stable; do not match on it.
    pub message: String,
}

impl ErrorDto {
    /// Builds an error body.
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}

/// Clamps a page size to what the library will actually serve.
pub fn effective_limit(requested: Option<usize>) -> usize {
    requested.unwrap_or(50).clamp(1, MAX_LIMIT)
}

/// The largest page `GET /songs/export` will hand back.
///
/// Ten times the search cap, and the difference is the difference between the two routes. A search
/// page is read by a person, and 500 is already more than anybody scrolls; an export page is read by
/// a program copying the whole catalog, where every page is a round trip on somebody's home Wi-Fi.
/// At this size a hundred-thousand-song catalog is twenty requests.
///
/// It is a cap and not a promise: a row is a few hundred bytes, so a full page is a couple of
/// megabytes, which is a reasonable thing to hold in memory once and not a reasonable thing to raise
/// much further.
pub const EXPORT_MAX_LIMIT: usize = 5_000;

/// Clamps an export page size.
pub fn export_limit(requested: Option<usize>) -> usize {
    requested.unwrap_or(1_000).clamp(1, EXPORT_MAX_LIMIT)
}

/// What `GET /songs/export` accepts.
///
/// No `deny_unknown_fields`, matching [`SearchParams`]: an unknown query parameter is ignored rather
/// than refused, so a client built against a later version can pass something this one has not heard
/// of and still get its catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExportParams {
    /// The last number of the previous page. Omitted for the first.
    pub after: Option<SongCode>,
    /// How many rows to return, capped at [`EXPORT_MAX_LIMIT`].
    pub limit: Option<usize>,
}

fn to_milli(value: f32) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use km_queue::queue::QueueEntry;
    use km_song::testing;
    use km_song::{ParseOptions, Song};

    use super::*;

    #[test]
    fn transport_names_are_lowercase_on_the_wire() {
        let json = serde_json::to_string(&TransportDto::from(Transport::Playing)).expect("json");
        assert_eq!(json, "\"playing\"");
        let json = serde_json::to_string(&TransportDto::from(Transport::Idle)).expect("json");
        assert_eq!(json, "\"idle\"");
    }

    #[test]
    fn state_of_an_idle_machine_has_a_null_song() {
        let json = serde_json::to_value(StateDto::from(&Snapshot::default())).expect("serialize");
        assert_eq!(json["transport"], "idle");
        assert!(json["now_playing"].is_null());
        assert_eq!(json["queue_len"], 0);
        assert_eq!(json["settings"]["transpose"], 0);
    }

    #[test]
    fn a_song_without_a_detected_melody_reports_the_toggle_as_unavailable() {
        let now = NowPlaying {
            origin: Origin::Catalog {
                number: SongCode::new(1),
                entry_id: 1,
            },
            kind: km_catalog::SongKind::Midi,
            title: "Song".to_owned(),
            artist: None,
            language: Some("pt".to_owned()),
            singer: None,
            duration_ms: 1000,
            melody_channel: None,
            has_lyrics: true,
            lyrics_hidden: false,
        };
        let dto = NowPlayingDto::from(&now);
        assert!(!dto.melody_available);

        let with_melody = NowPlaying {
            melody_channel: Some(4),
            ..now
        };
        assert!(NowPlayingDto::from(&with_melody).melody_available);
    }

    /// An export line for an untagged song, which carries no `tags` key, still parses.
    ///
    /// The guard behind `serde(default)` on [`SongDto::tags`], and it is about the *offline remote*
    /// rather than about JSON: `km-remote-core` reads this type back out of
    /// `GET /api/v1/songs/export`, one row per line, and refuses the whole page if a line will not
    /// parse. `skip_serializing_if` leaves the key out of every untagged song.
    #[test]
    fn an_exported_song_with_no_tags_key_still_parses() {
        let older = r#"{"number":"1001","title":"One","artist":null,"language":"pt",
            "kind":"midi","duration_ms":1000,"suitability":null,"melody_available":false,
            "default_transpose":0,"package_id":"vol1"}"#;
        let parsed: SongDto = serde_json::from_str(older).expect("a pre-tags row still parses");
        assert!(parsed.tags.is_empty());

        // And the same in the other direction: a song with no tags spends no bytes saying so, which
        // is what `skip_serializing_if` is for on a route that pages five thousand rows at a time.
        let json = serde_json::to_string(&parsed).expect("serialize");
        assert!(!json.contains("\"tags\""), "got {json}");
    }

    #[test]
    fn origin_is_tagged_so_a_client_can_switch_on_it() {
        let json = serde_json::to_value(OriginDto::from(&Origin::File {
            path: "a.kar".to_owned(),
        }))
        .expect("serialize");
        assert_eq!(json["kind"], "file");
        assert_eq!(json["path"], "a.kar");
    }

    #[test]
    fn a_queue_response_numbers_positions_from_zero() {
        let entries = vec![
            QueueEntry {
                id: 7,
                number: SongCode::new(100),
                title: "First".to_owned(),
                artist: None,
                singer: Some("Ana".to_owned()),
            },
            QueueEntry {
                id: 9,
                number: SongCode::new(200),
                title: "Second".to_owned(),
                artist: Some("Band".to_owned()),
                singer: None,
            },
        ];
        let dto = QueueDto::new(&entries);
        assert_eq!(dto.len, 2);
        assert_eq!(dto.entries[0].position, 0);
        assert_eq!(dto.entries[0].id, 7);
        assert_eq!(dto.entries[1].position, 1);
        assert_eq!(dto.capacity, km_queue::queue::MAX_QUEUED);
    }

    #[test]
    fn a_full_page_of_results_says_there_may_be_more() {
        let songs: Vec<CatalogSong> = (0..3).map(|n| song_row(SongCode::new(n + 1))).collect();
        assert!(SearchResponse::new(&songs, 0, 3).more);
        assert!(!SearchResponse::new(&songs, 0, 10).more);
    }

    #[test]
    fn a_page_size_is_clamped_rather_than_trusted() {
        assert_eq!(effective_limit(None), 50);
        assert_eq!(effective_limit(Some(0)), 1);
        assert_eq!(effective_limit(Some(10)), 10);
        assert_eq!(effective_limit(Some(1_000_000)), MAX_LIMIT);
    }

    #[test]
    fn a_package_response_does_not_leak_the_operators_paths() {
        let package = InstalledPackage {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1".to_owned(),
            path: "D:/private/place/vol1.kmpkg".to_owned(),
            song_count: 12,
            installed_at: "2026-08-23".to_owned(),
            bank: 1,
            flags: km_kmpkg::PackageFlags::NONE,
        };
        let json = serde_json::to_string(&PackageDto::new(&package, false)).expect("json");
        assert!(!json.contains("private"));
        assert!(json.contains("vol1"));
        // Not removable, and the *reason* stays behind with the path — every one of them names a
        // full path, which is the thing this test exists to keep off the wire.
        assert!(json.contains("\"removable\":false"));
        assert!(!json.contains("debug.packages"));
    }

    /// The word goes out whole, and the names say only the bits this build knows.
    #[test]
    fn a_package_row_carries_its_flags_as_a_word_and_as_names() {
        let package = InstalledPackage {
            id: "vol1".to_owned(),
            name: "Volume 1".to_owned(),
            version: "1".to_owned(),
            path: String::new(),
            song_count: 1,
            installed_at: "2026-08-23".to_owned(),
            bank: 1,
            flags: km_kmpkg::PackageFlags::from_bits(0b1001),
        };
        let json = serde_json::to_string(&PackageDto::new(&package, true)).expect("json");
        assert!(json.contains("\"flags\":9"), "{json}");
        assert!(json.contains("\"flag_names\":[\"uncurated\"]"), "{json}");

        // A machine that predates the field sends neither, and the row still reads.
        let older: PackageDto = serde_json::from_str(
            r#"{"id":"a","name":"A","version":"1","song_count":1,"installed_at":"x","bank":1,
                "removable":true}"#,
        )
        .expect("an older row");
        assert_eq!(older.flags, 0);
        assert!(older.flag_names.is_empty());
    }

    /// The sentence quotes the name, and an id is exactly what it must not say.
    ///
    /// Sixteen hexadecimal characters across a television tell the person who has just dropped a
    /// file nothing about which one went in.
    #[test]
    fn an_install_is_reported_by_name_and_never_by_id() {
        let report = InstallReportDto {
            package_id: "b190ee05299ede16".to_owned(),
            package_name: "Carols 1999".to_owned(),
            songs_added: 205,
            replaced_existing: false,
            duplicate_content: Vec::new(),
        };
        assert_eq!(
            report.sentence(),
            "installed \"Carols 1999\" \u{b7} 205 songs"
        );
        assert!(!report.sentence().contains("b190ee05299ede16"));

        // A second copy of the same package is an upgrade, and says so.
        let again = InstallReportDto {
            replaced_existing: true,
            songs_added: 1,
            ..report
        };
        assert_eq!(again.sentence(), "updated \"Carols 1999\" \u{b7} 1 song");
    }

    /// A manifest that names nothing is reported under its id rather than under nothing.
    ///
    /// The same fallback `PackageMeta::file_stem` takes.
    #[test]
    fn a_package_with_no_name_falls_back_to_its_id() {
        let older = r#"{"package_id":"b190ee05299ede16","package_name":"","songs_added":2,
            "replaced_existing":false,"duplicate_content":[]}"#;
        let report: InstallReportDto =
            serde_json::from_str(older).expect("a report naming nothing parses");
        assert_eq!(
            report.sentence(),
            "installed \"b190ee05299ede16\" \u{b7} 2 songs"
        );

        let blank = InstallReportDto {
            package_name: "   ".to_owned(),
            ..report
        };
        assert_eq!(
            blank.sentence(),
            "installed \"b190ee05299ede16\" \u{b7} 2 songs"
        );
    }

    /// **The running value and the stored one, which on this route can genuinely disagree.** A
    /// client reading this back after a `PUT` should not have to guess whether it got an echo of the
    /// request or the machine's own answer: it gets both, and `enabled != stored` is exactly the
    /// state "you have turned it on and it starts working after a restart".
    #[test]
    fn the_debug_switch_reports_what_is_running_and_what_is_written_down() {
        let json = serde_json::to_string(&DebugDto {
            enabled: false,
            stored: true,
        })
        .expect("serializes");
        assert_eq!(json, r#"{"enabled":false,"stored":true}"#);

        let request: DebugRequest = serde_json::from_str(r#"{"enabled":false}"#).expect("parses");
        assert!(!request.enabled);
        // `deny_unknown_fields`, so the old spelling is refused rather than silently ignored.
        assert!(serde_json::from_str::<DebugRequest>(r#"{"accept":true}"#).is_err());
    }

    /// The console's switch says both what it is and whether it did anything.
    ///
    /// `enabled` without `served` is the state that produces the support question this pair exists
    /// to answer — the switch is on, `/dev/` answers 404, and nothing says debugging is the reason.
    #[test]
    fn the_console_switch_says_whether_it_is_actually_serving_anything() {
        let asked_but_not_served = serde_json::to_string(&DevRemoteDto {
            enabled: true,
            served: false,
        })
        .expect("serializes");
        assert_eq!(asked_but_not_served, r#"{"enabled":true,"served":false}"#);

        let request: DevRemoteRequest =
            serde_json::from_str(r#"{"enabled":true}"#).expect("parses");
        assert!(request.enabled);
        assert!(serde_json::from_str::<DevRemoteRequest>(r#"{"served":true}"#).is_err());
    }

    #[test]
    fn a_settings_patch_rejects_a_misspelled_field() {
        let error = serde_json::from_str::<SettingsPatchDto>(r#"{"transpoze": 2}"#);
        assert!(error.is_err());
    }

    #[test]
    fn a_settings_patch_quantises_floats_into_the_internal_patch() {
        let patch = serde_json::from_str::<SettingsPatchDto>(r#"{"tempo_ratio": 1.25}"#)
            .expect("parse")
            .to_patch();
        assert_eq!(patch.tempo_milli, Some(1250));
        assert_eq!(patch.transpose, None);
    }

    #[test]
    fn a_non_finite_float_becomes_zero_rather_than_poisoning_the_engine() {
        let patch = SettingsPatchDto {
            music_volume: Some(f32::NAN),
            ..Default::default()
        }
        .to_patch();
        assert_eq!(patch.music_volume_milli, Some(0));
    }

    #[test]
    fn a_settings_patch_carries_the_lyric_offset_through_unquantised() {
        // An integer on the wire and an integer internally, so unlike the tempo it needs no
        // thousandths to keep `SettingsPatch` comparable.
        let patch = serde_json::from_str::<SettingsPatchDto>(r#"{"lyric_offset_ms": -40}"#)
            .expect("parse")
            .to_patch();
        assert_eq!(patch.lyric_offset_ms, Some(-40));
    }

    #[test]
    fn a_patch_of_only_the_lyric_offset_leaves_every_other_field_absent() {
        // The property the whole partial-patch design rests on: setting the one number a remote's
        // offset box owns must not quietly restate transpose, tempo, melody or volume.
        let patch = serde_json::from_str::<SettingsPatchDto>(r#"{"lyric_offset_ms": 40}"#)
            .expect("parse")
            .to_patch();
        assert_eq!(patch.lyric_offset_ms, Some(40));
        assert_eq!(patch.transpose, None);
        assert_eq!(patch.tempo_milli, None);
        assert_eq!(patch.melody_enabled, None);
        assert_eq!(patch.music_volume_milli, None);
    }

    #[test]
    fn the_lyric_offset_reaches_the_settings_dto() {
        let dto = SettingsDto::from(Settings {
            lyric_offset_ms: 40,
            ..Settings::default()
        });
        assert_eq!(dto.lyric_offset_ms, 40);
        let json = serde_json::to_string(&dto).expect("serialize");
        assert!(json.contains("\"lyric_offset_ms\":40"), "{json}");
    }

    #[test]
    fn queueing_accepts_a_body_without_a_singer() {
        let request: AddToQueueRequest =
            serde_json::from_str(r#"{"number": "1234"}"#).expect("parse");
        assert_eq!(request.number, SongCode::new(1234));
        assert_eq!(request.singer, None);

        // A banked song is asked for the same way, because a code is one string whatever thousand
        // it is in.
        let banked: AddToQueueRequest =
            serde_json::from_str(r#"{"number": "3500"}"#).expect("parse");
        assert_eq!(banked.number.bank(), 3);
        assert_eq!(banked.number.slot(), 500);
    }

    #[test]
    fn a_bare_integer_is_refused_rather_than_read_as_a_number() {
        // **One spelling on the wire.** The reason is no longer ambiguity — a bare integer names
        // exactly one song now — but that there is one spelling at all: every route, element id,
        // template and script already sends a string, and a second accepted shape is what this rule
        // exists to prevent.
        let refused = serde_json::from_str::<AddToQueueRequest>(r#"{"number": 1234}"#);
        assert!(refused.is_err(), "{refused:?}");
    }

    #[test]
    fn lyrics_are_converted_from_ticks_to_milliseconds() {
        let song = parse(testing::soft_karaoke());
        let dto = LyricsDto::from_song(Some(SongCode::new(42)), &song);
        assert_eq!(dto.number, Some(SongCode::new(42)));
        assert!(!dto.lines.is_empty());
        // Line and syllable times are ordered and inside the song.
        let mut previous_end = 0;
        for line in &dto.lines {
            assert!(line.start_ms >= previous_end || line.start_ms == 0);
            assert!(line.end_ms >= line.start_ms);
            assert!(line.end_ms <= dto.duration_ms);
            for syllable in &line.syllables {
                assert!(syllable.start_ms >= line.start_ms);
                assert!(syllable.end_ms <= line.end_ms);
            }
            previous_end = line.end_ms;
        }
        // The whole-line text is the syllables joined, so a simple client and a syllable-following
        // client agree about what the words are.
        for line in &dto.lines {
            let joined: String = line.syllables.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(line.text, joined);
        }
    }

    #[test]
    fn lyrics_of_a_song_without_any_are_an_empty_list_not_an_error() {
        let song = parse(testing::instrumental());
        let dto = LyricsDto::from_song(None, &song);
        assert!(dto.lines.is_empty());
        assert_eq!(dto.granularity, LyricGranularity::None);
    }

    #[test]
    fn an_error_body_carries_a_stable_code_and_a_human_sentence() {
        let json = serde_json::to_value(ErrorDto::new("queue_full", "the queue is full"))
            .expect("serialize");
        assert_eq!(json["error"], "queue_full");
        assert_eq!(json["message"], "the queue is full");
    }

    #[test]
    fn mics_always_say_they_apply_no_processing() {
        let dto = MicsDto::new(&[MicChannel::new("mic1", "Mic 1")]);
        assert!(!dto.applies_dsp);
        assert_eq!(dto.mics[0].gain, 1.0);
    }

    fn parse(bytes: Vec<u8>) -> Song {
        Song::parse(&bytes, &ParseOptions::default()).expect("fixture parses")
    }

    fn song_row(number: SongCode) -> CatalogSong {
        CatalogSong {
            number,
            package_id: "vol1".to_owned(),
            kind: km_catalog::SongKind::Midi,
            title: format!("Song {number}"),
            artist: None,
            language: None,
            file: "songs/a.kar".to_owned(),
            duration_ms: 1000,
            lyric_encoding: None,
            default_transpose: 0,
            lyrics_hidden: false,
            fixes: Vec::new(),
            melody_channel: None,
            suitability: Some(8),
            content_hash: None,
            lyric_preview: Vec::new(),
            tags: Vec::new(),
            // MIDI, so nothing measured it -- a MIDI song is the reference. See `CatalogSong`.
            loudness_lufs: None,
        }
    }
}
