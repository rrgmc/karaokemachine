//! The seam between the API and the machine it controls.
//!
//! The HTTP surface has to be testable in CI, and CI has no audio device, no SoundFont and no
//! catalog on disk. So `km-api` never touches the engine or the library directly — it drives two
//! traits, and `km-app` supplies the real implementations in M7 while the tests here supply stubs.
//!
//! The split is by ownership rather than by verb, which is why queries and mutations appear in both:
//!
//! * [`Catalog`] is everything backed by SQLite and the package files. Slow-ish, fallible, safe to
//!   do on a request thread.
//! * [`Controller`] is everything backed by the control thread and the audio callback. Fast,
//!   non-blocking, and the only thing that may touch playback.
//!
//! Both take `&self`. The real implementations sit behind a mutex or a command channel; making the
//! trait `&mut self` would put a write lock on the whole router for the length of a request.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use km_catalog::search::SearchQuery;
use km_catalog::{CatalogSong, InstallReport, InstalledPackage, SongKind};
use km_queue::Transport;
use km_queue::mics::{MicChannel, MicPatch};
use km_queue::queue::{QueueEntry, QueueRequest};
use km_song::Song;
use km_songcode::SongCode;

/// A refusal, and the stable name for it when it has one.
///
/// **This exists because a message is not renderable by anybody but the machine that wrote it.**
/// `km-api`'s own header already says the `message` is "for a person and is explicitly not stable";
/// what it did not say, because nothing needed it yet, is that the person reading it may not be
/// reading English. A remote that shows the machine's sentence shows an English sentence inside a
/// Portuguese page.
///
/// So a refusal a *singer* can provoke carries a code, and the surface showing it looks the sentence
/// up in its own catalog. A refusal only the owner can reach — a SoundFont that will not load, a
/// path outside the allowed roots — carries `None` and travels as prose, which is the right answer
/// there: those are diagnostics, they are read beside a log, and a code per case would say less than
/// the sentence does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The stable name a client may render from, or `None` for a refusal with no catalog entry.
    pub code: Option<&'static str>,
    /// What happened, in the machine's own words. Never stable, and never the only channel.
    pub message: String,
}

impl Refusal {
    /// A refusal a client can render for itself.
    #[must_use]
    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: Some(code),
            message: message.into(),
        }
    }
}

impl From<String> for Refusal {
    /// A refusal with no code, carrying only its prose.
    fn from(message: String) -> Self {
        Self {
            code: None,
            message,
        }
    }
}

impl From<&str> for Refusal {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Why a catalog operation failed.
///
/// Deliberately coarse. The API turns each variant into one status code, and a remote can do
/// nothing useful with a distinction finer than these — the finer thing it *can* use is
/// [`Refusal::code`], which rides inside two of them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// No song has that number, or no package has that id.
    ///
    /// **Carries what was missing**, because a 404 whose whole body said `"not found"` is what this
    /// used to produce — and two handlers had to intercept the variant and put the subject back by
    /// hand to make `uninstall_package` and `set_package_bank` readable. The fix belonged to the
    /// type rather than to those two call sites, and applying it there is what left every *other*
    /// route saying nothing.
    ///
    /// Phrase it as the thing, not as a sentence: `ApiError::not_found` prefixes `no such `.
    #[error("no such {0}")]
    NotFound(String),
    /// The request was understood but is not acceptable — a path outside the allowed roots, a
    /// package that would collide with installed numbers.
    #[error("{0}")]
    Rejected(String),
    /// The machine is not in a state where this means anything, and the caller should retry later.
    ///
    /// A 409 rather than a 400, exactly as [`ControlError::Unavailable`] is and for the same reason:
    /// nothing is wrong with the request. Moving a package to another bank while a song is queued is
    /// case that needed it — the queue names codes, and re-keying under it would leave entries for
    /// songs that no longer exist.
    #[error("{0}")]
    Unavailable(Refusal),
    /// Something went wrong that is not the caller's fault.
    #[error("{0}")]
    Failed(String),
}

/// A package the machine found and could not install.
///
/// **A fault, not a decision**, which is why it is remembered separately from the ignore list and
/// reported every start rather than once: an uninstall is something the owner chose and a package
/// that will not open is something to fix. Until this existed the only trace was one line in a log,
/// which on an appliance under a television is indistinguishable from the package not being there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageProblem {
    /// Where the file is, so it can be found and fixed.
    pub path: String,
    /// Its identifier, when the package opened far enough to have one.
    pub package_id: Option<String>,
    /// What went wrong, in a sentence somebody can act on.
    ///
    /// **A sentence somebody can act on, rather than data to be formatted.** Knowing that five
    /// numbers collided is not actionable; knowing that 500 is already `vol1`'s *Águas de Março* is.
    /// Two packages cannot claim one number — each is in a thousand of its own — so the faults that
    /// reach here are a package that will not open and a bank already taken, and both of those are
    /// one sentence with one remedy.
    pub reason: String,
}

impl PackageProblem {
    /// A stable name for this refused file, for a page that offers to delete it.
    ///
    /// **Derived rather than stored**, which is the whole point: [`Self::path`] is also the key
    /// `record_package_problem` de-dupes on, and an id kept beside it as a field would be free to
    /// disagree with it — including in a test double, which is handed whole `PackageProblem`s and
    /// could be given an id that names a different file than the one it holds.
    ///
    /// **The file's name is not enough on its own.** Problems are gathered from `debug.packages`
    /// and from *every* folder in `package_dirs`, so two of them genuinely can be called
    /// `volume-1.kmpkg` — and the case that produced this was tamer and just as bad: one package in
    /// two folders, listed twice, and a single control that would have deleted whichever the loop
    /// reached first. So the name, and then eight hex digits of FNV-1a over the whole path.
    ///
    /// The name keeps the URL legible; the fingerprint makes it unique; the path itself stays out
    /// of it, because a URL is pasted, screenshotted and logged and the name is already published
    /// on three surfaces where the folder deliberately is not.
    ///
    /// It does **not** round-trip, exactly as a picture id does not: nothing turns one of these
    /// back into a path. A delete matches it against a freshly read list of problems, which is what
    /// keeps an id a browser invented off the filesystem.
    pub fn id(&self) -> String {
        // `file_name_of` and not `Path::file_name`, for the reason its own doc gives: these are
        // strings the machine wrote, and a Windows path read by a build that is not Windows comes
        // back whole from the `Path` version — which would put the folder in the id.
        let mut name = String::new();
        for ch in crate::dto::file_name_of(&self.path).chars() {
            if ch.is_ascii_alphanumeric() {
                name.push(ch.to_ascii_lowercase());
            } else if !name.ends_with('-') {
                name.push('-');
            }
        }
        // FNV-1a. Written out rather than taken from `std`: `DefaultHasher`'s value is explicitly
        // not guaranteed between toolchains, and a test asserting a literal id would then break on
        // a compiler bump for no reason anybody could see.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in self.path.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let hash = format!("{:08x}", hash as u32);
        let name = name.trim_matches('-');
        // A file name that slugs away to nothing still gets an addressable id — the hash alone is a
        // poor label and a perfectly good name, and the files that are strangest are exactly the
        // ones somebody most needs the control for.
        if name.is_empty() {
            hash
        } else {
            format!("{name}-{hash}")
        }
    }
}

/// What a machine that does not delete refused packages says, in one place.
///
/// Quoted by both halves of the defaulted pair on [`Catalog`] — the question a page asks and the
/// answer the operation gives — so the two cannot come to describe one machine two ways.
const NO_PROBLEM_DELETION: &str = "this machine does not delete package files";

/// What one reading of the packages folders did.
///
/// Defined here rather than in `km-catalog` because it describes a *pass over the folders*, which
/// is the machine's business — the library knows only about rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RescanReport {
    /// How many packages went in, new and re-indexed alike.
    pub installed: usize,
    /// Packages whose rows were dropped, because nothing offered them this pass.
    pub removed: Vec<String>,
    /// Packages gone from the folders that were left in the catalog for now.
    ///
    /// Non-empty means the machine was busy: something is playing or queued, and removing rows a
    /// queued number points at would lose somebody their turn in silence. They go at the next idle
    /// rescan or the next start.
    pub deferred: Vec<String>,
    /// How many packages the machine is complaining about after the pass.
    pub problems: usize,
}

/// Why a control operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    /// No queue entry, mic or song with that identifier.
    ///
    /// Carries what was missing, for [`CatalogError::NotFound`]'s reason. Phrase it as the thing —
    /// `queue entry 7`, `microphone 'left'` — not as a sentence.
    #[error("no such {0}")]
    NotFound(String),
    /// The queue is at its limit.
    #[error("the queue is full")]
    QueueFull,
    /// The machine cannot do this right now — seeking with nothing loaded, toggling a melody on a
    /// song where detection abstained.
    #[error("{0}")]
    Unavailable(Refusal),
    /// The value was out of range or otherwise unusable.
    ///
    /// **No [`Refusal`], unlike [`Self::Unavailable`], and the asymmetry is the point.** A 400 says
    /// *fix what you sent*, so it is aimed at whatever built the request rather than at the person
    /// holding the phone — the remote clamps a key change before it asks, so a singer never sees
    /// one. A 409 says *the machine cannot do that*, which is a sentence somebody in the room reads.
    #[error("{0}")]
    Rejected(String),
    /// Something went wrong that is not the caller's fault.
    #[error("{0}")]
    Failed(String),
}

/// What is playing, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A catalog song, queued by number.
    Catalog {
        /// The queueing number.
        number: SongCode,
        /// The queue entry it came from, so a remote can correlate the two.
        entry_id: u64,
    },
    /// A file loaded directly, bypassing the catalog. The `--play` debug path.
    File {
        /// Where it came from, for the operator's benefit.
        path: String,
    },
    /// A catalog song the machine chose for itself, because nobody was singing.
    ///
    /// **No `entry_id`, and the absence is deliberate.** A demo song is never queued — it is loaded
    /// straight onto the deck — so there is no entry to correlate it with, nothing to remove, and
    /// no place in the queue for it to have.
    ///
    /// **This variant is also the whole of "a demo stands aside for a queued song".** `queue_add`
    /// tests for *this* origin, gives the deck up and publishes
    /// [`EndReason::Yielded`](crate::events::EndReason::Yielded), so "is a demo playing?" is
    /// answered by what is loaded rather than by a flag beside it: one place knows, and it is this.
    ///
    /// A client should say so. Somebody who wants to sing queues, and their song takes the deck; a
    /// machine playing music with an empty queue and no explanation looks broken.
    Demo {
        /// The queueing number, so it can be shown and written down.
        number: SongCode,
    },
}

/// The song currently loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlaying {
    /// Where it came from.
    pub origin: Origin,
    /// Title.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// What it is sung in, as an ISO 639-1 code, when the catalog knows.
    ///
    /// `None` for the two debug play paths: the only value available there is the raw `@L` header,
    /// and putting `ENGL` on a wire where everything else is a code is worse than saying nothing.
    pub language: Option<String>,
    /// Who asked for it.
    pub singer: Option<String>,
    /// Whether this is a MIDI song or a video song.
    ///
    /// The three performance adjustments — key, tempo and guide melody — exist only for a MIDI one,
    /// so this is what decides whether a client is offered them at all.
    pub kind: SongKind,
    /// Length in milliseconds, as the file reports it.
    pub duration_ms: u32,
    /// The detected melody channel, or `None` when detection abstained.
    ///
    /// `None` is why the melody toggle is *hidden* rather than disabled: claiming a melody channel
    /// that was never confidently detected and muting the wrong instrument ruins the song.
    pub melody_channel: Option<u8>,
    /// Whether the file has lyrics at all.
    pub has_lyrics: bool,
}

/// Playback settings a remote may change.
///
/// Not the whole settings file — that is `km-app`'s business in M7. These are the knobs that belong
/// to a performance and that a phone in somebody's hand should be able to turn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Semitones of transposition.
    pub transpose: i8,
    /// Tempo multiplier.
    pub tempo_ratio: f32,
    /// Whether the guide melody sounds.
    pub melody_enabled: bool,
    /// Backing-track level, `0.0..=1.0`.
    pub music_volume: f32,
    /// How far ahead of the audio the local display draws the lyric highlight, in milliseconds.
    ///
    /// Positive means the lyrics lead. It calibrates the screen this machine is plugged into, so it
    /// is reported here for a remote to *set*, not for one to time against -- the `lyric_line`
    /// events carry the true tick, and a phone across the room has its own latency.
    pub lyric_offset_ms: i16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            transpose: 0,
            tempo_ratio: 1.0,
            melody_enabled: false,
            music_volume: 1.0,
            lyric_offset_ms: 0,
        }
    }
}

/// A partial change to [`Settings`]. Absent fields are left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsPatch {
    /// New transposition.
    pub transpose: Option<i8>,
    /// New tempo multiplier, in thousandths, so the patch stays `Eq`.
    pub tempo_milli: Option<u32>,
    /// Whether the guide melody should sound.
    pub melody_enabled: Option<bool>,
    /// New backing-track level, in thousandths.
    pub music_volume_milli: Option<u32>,
    /// New lyric display offset, in milliseconds. An integer already, so unlike the two above it
    /// needs no quantising to keep this type `Eq`.
    pub lyric_offset_ms: Option<i16>,
}

impl SettingsPatch {
    /// Whether this would change nothing.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// What the machine is doing, in one read.
///
/// One struct rather than several endpoints because a remote redrawing its screen needs all of it at
/// once, and two round trips can disagree with each other.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Whether it is playing, paused or idle.
    pub transport: Transport,
    /// The loaded song, if any.
    pub now_playing: Option<NowPlaying>,
    /// Position within the song.
    pub position_ms: u32,
    /// Songs waiting.
    pub queue_len: usize,
    /// Current settings.
    pub settings: Settings,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            transport: Transport::Idle,
            now_playing: None,
            position_ms: 0,
            queue_len: 0,
            settings: Settings::default(),
        }
    }
}

/// What to do to the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportCommand {
    /// Start or resume. With nothing loaded, take the next queued song.
    Play,
    /// Pause where it is.
    Pause,
    /// Abandon the current song and take the next. With nothing loaded, start a demo song if demo
    /// mode is on.
    ///
    /// **Skip asks for the next thing, so on a machine choosing songs for itself the next thing is
    /// the machine's to pick.** With the mode off there is nothing to ask for and it refuses. Either
    /// way it carries one refusal, `nothing_playing`: the reasons a demo cannot start belong to
    /// [`Controller::start_demo_song`], which is public and gives them.
    Skip,
    /// Start the current song again from the top.
    Restart,
    /// Stop and unload.
    Stop,
    /// Jump to a position in milliseconds.
    Seek {
        /// Milliseconds from the start.
        ms: u32,
    },
}

/// The longest demo delay [`Controller::set_demo_delay`] will accept, in seconds — an hour.
///
/// **A bound on what a route may be told, not on what a settings file may hold.** An hour is already
/// past the point where anybody could tell a demo from a machine that never performs, so a number
/// above it is far likelier to be a typo than an intention — and the box that sends one is on a page
/// an owner reaches from a phone. The setting itself is uncapped: somebody editing their own
/// machine's file is being deliberate, and clamping that would be the product overruling them in the
/// one place it has no business doing so.
///
/// **Here rather than in `karaokemachine::settings`** because it is part of what the route promises,
/// and this crate is where the route is described. Every `Controller` enforces it, and each one that
/// does is asserting the same contract rather than inventing a policy.
pub const MAX_DEMO_DELAY_SECS: u32 = 3600;

/// What the settings file says about the two switches that open a developer surface.
///
/// **Stored values only, and that is the whole reason this type exists.** The running values are on
/// [`crate::ApiConfig`], where they were put at start and cannot move — both switches take effect at
/// the next restart, because what they change is which routes get mounted. So a page drawing a
/// switch has to draw this, and a page explaining *why the console is not up* has to draw both
/// halves of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeveloperSwitches {
    /// Whether `debug.enabled` says debugging mode is on.
    pub debug: bool,
    /// Whether `api.serve_dev_remote` says the development console is asked for.
    pub dev_remote: bool,
}

/// Demo mode, as the API reports it.
///
/// Two booleans rather than one, because "on right now" and "on after a restart" are genuinely
/// different questions and a single field would answer neither reliably: `PUT /demo` can change the
/// running machine without touching the settings file, which is the whole point of its `persist`
/// flag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DemoState {
    /// Whether demo mode is on **for this run**.
    pub enabled: bool,
    /// Whether the settings file says it is on, and so whether it survives a restart.
    pub stored: bool,
    /// Seconds of silence before a demo song starts. `0` means "as soon as the machine is idle".
    pub delay_secs: u32,
    /// The suitability floor a demo song must clear, out of ten. `None` for no filter.
    pub min_suitability: Option<u8>,
    /// Whether what is loaded right now is a demo song.
    ///
    /// Derivable from [`Origin::Demo`] on the snapshot, and stated anyway for the reason
    /// [`NowPlayingDto::melody_available`] is stated: a client that has to infer it will sometimes
    /// infer it wrong, and this is what decides whether it tells somebody to press skip.
    ///
    /// [`NowPlayingDto::melody_available`]: crate::dto::NowPlayingDto::melody_available
    pub playing: bool,
    /// How long until a demo song starts, when one is due to.
    ///
    /// `None` when demo mode is off, when something is already playing, when songs are queued, or
    /// when the wait has already elapsed and the next poll will start one. It is a countdown for a
    /// screen to show, not a promise — a queued song cancels it.
    pub starts_in_secs: Option<u32>,
}

/// The wallpaper cycle, as the API reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WallpaperState {
    /// The image on screen, as a file name rather than a full path — the full path is the
    /// operator's filesystem layout and no remote's business.
    pub current: Option<String>,
    /// How many images the folder yielded.
    pub count: usize,
    /// Seconds between changes.
    pub interval_secs: u32,
    /// Whether the order is shuffled.
    pub shuffle: bool,
    /// Whether a song starting changes the picture, on top of the interval.
    ///
    /// Reported beside [`Self::interval_secs`] because the two together are the whole answer to
    /// *when does the picture change*, and a page that showed only the interval would be stating
    /// half of it as all of it.
    pub on_song_change: bool,
    /// Why there are no images, when there are none.
    pub problem: Option<String>,
    /// Which rule chose the folder the images come from.
    ///
    /// The answer to the one question an owner who has just added a wallpaper actually asks: *why
    /// isn't my picture on the screen?* Without it a page can only show a list of names that does
    /// not include the file somebody just added, which explains nothing.
    ///
    /// **The provenance is publishable exactly where the path is not.** [`Self::current`] refuses to
    /// carry a full path because that is the operator's filesystem layout; *which rule won* is not.
    /// With [`Self::count`] and [`Self::problem`] beside it a client can answer the question
    /// completely: `bundled` with a count of four means the owner's folder held nothing when it was
    /// last looked at — which is now every cycle rather than once at startup.
    pub source: WallpaperSource,
}

/// One file in the wallpaper folder, as the API reports it.
///
/// **A file rather than an image**, which is why a zip has a `count` above one. See
/// [`Controller::wallpaper_pictures`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// A stable identifier, opaque to a remote and safe to store.
    ///
    /// Slugged from the file name the way a bank's is, and resolved back to a path only by matching
    /// a fresh scan — never by joining it onto the folder. That is [`Controller::delete_soundfont`]'s
    /// rule and it is what stops a crafted id reaching the filesystem.
    pub id: String,
    /// The file name, which is what a page shows. Never a full path — the operator's filesystem
    /// layout is no remote's business, exactly as [`WallpaperState::current`] says.
    pub name: String,
    /// How many images this file contributes: one for a picture, and an archive's entry count.
    pub images: usize,
    /// Size in bytes.
    pub bytes: u64,
    /// Why [`Controller::delete_wallpaper`] would refuse this file, or `None` if it would not.
    ///
    /// The picture twin of [`SoundFontBank::why_not_removable`], with the same **permanent refusals
    /// only** rule: a page spends this by leaving the control off the row rather than drawing one
    /// that can only ever be refused.
    pub why_not_removable: Option<String>,
}

/// Whose wallpapers are on screen, and by which rule.
///
/// Deliberately not `Option<String>`: there are exactly four answers and a page renders each
/// differently. A `None` meaning "a setting named it" would be `null` meaning two things — the shape
/// [`crate::dto::AudioOutputRequest`] already refuses — and `--show-paths` has always rendered the
/// setting case as its own line.
///
/// `km-app`'s own `settings::WallpaperSource` is the same four and is what this is built from —
/// repeated here because `km-api` describes the machine to a client without depending on the
/// machine, which is the same reason [`AudioOutputs`] is not `cpal`'s vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperSource {
    /// `wallpaper.dir` in settings named the folder, which beats all three rules below.
    ///
    /// The fourth answer, and the one the other three cannot express: a setting that overrides them
    /// would otherwise be reported as whichever rule *would* have won, which is a folder the machine
    /// is not reading.
    Setting,
    /// The set that ships with the machine. The default, and what a fresh install shows.
    #[default]
    Bundled,
    /// A folder the platform's packaging laid down over the bundled one.
    Overlay,
    /// The owner's own folder, which wins outright the moment it holds one image.
    Owner,
}

/// What an owner is uploading through the browser.
///
/// Three kinds and not a filename to guess from: the route says which, because the route is what
/// carried the size limit and the allowed extensions that got the file this far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upload {
    /// A `.kmpkg`, installed into the catalog once it lands.
    Package,
    /// An image, or a `.zip` of images, joining the wallpaper rotation.
    Wallpaper,
    /// A `.sf2` bank, which becomes selectable.
    SoundFont,
}

/// Where the machine's sound goes, as the API reports it.
///
/// The vocabulary is this crate's own rather than `cpal`'s: `km-api` must not gain an audio
/// dependency to describe an audio device, and a `TestMachine` with no sound card has to be able to
/// answer these questions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioOutputs {
    /// Every device that could be chosen, the system default first.
    pub devices: Vec<AudioOutput>,
    /// What settings ask for. `None` means nothing has ever been chosen.
    pub selected: Option<String>,
    /// The identifier actually in use, which differs from `selected` after a fallback.
    pub active_id: String,
    /// Its name.
    pub active_name: String,
    /// Whether the machine is playing through a second choice because the first is absent.
    pub fell_back: bool,
    /// Whether the device can be changed right now.
    ///
    /// `false` while anything is loaded or queued: the player lives inside the stream a change has
    /// to drop. See `Controller::set_audio_output`.
    pub changeable: bool,
    /// The level the active device is running at, where it has one.
    ///
    /// **The active device's, and no other's.** Reading a level means opening a mixer, and a Linux
    /// box lists one output under a dozen spellings, so a level on every row would be a dozen
    /// mixers opened to answer one page.
    ///
    /// `None` in two different situations that look the same from here and read the same to a
    /// person: this build cannot reach a mixer at all, and the active device has no level to
    /// report because it hands the volume to a receiver downstream. Either way there is nothing to
    /// draw and nothing to set.
    pub level: Option<OutputLevel>,
}

/// What an output is set to, and the range it can be moved within.
///
/// **Hundredths of a decibel, as integers**, which is the unit ALSA itself reports and the same
/// bargain [`MicPatch`] makes with `*_milli`: it keeps [`AudioOutputs`] `Eq` and diffable in a test,
/// where a float would not be. The wire form carries decibels as a number a person would recognise,
/// and `dto` converts.
///
/// **Decibels rather than a percentage, and that is the whole point of the type.** A control
/// spanning `0 - 128` steps with a floor of −128 dB puts −20 dB at "84 %", which reads as almost
/// all the way up and is a tenth of the voltage. A percentage here would describe the position of a
/// knob; this describes what comes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputLevel {
    /// What it is set to now.
    pub db_centi: i32,
    /// The quietest it goes, often far below anything audible.
    pub db_min_centi: i32,
    /// The loudest it goes, which is unity on every control seen so far.
    pub db_max_centi: i32,
    /// The smallest move the control can make.
    pub step_centi: i32,
}

/// The output identifier meaning *follow whatever the system calls the default*.
///
/// **Chosen deliberately rather than absent, which is the whole reason the sentinel exists.** An
/// absent `audio.output_device` means nobody has chosen and lets the machine prefer a USB interface
/// on Linux at every start; this says *do not prefer one*, and sticks.
///
/// **Spelled here as well as in `km_audio::device::SYSTEM_DEFAULT`, and the two must agree.** This
/// crate deliberately does not depend on the audio backend — see [`AudioOutputs`] on why its
/// vocabulary is its own — and a page needs to know which row in a list is the sentinel in order to
/// label it in the reader's language rather than in the backend's English. `karaokemachine` sees
/// both crates and its tests assert the two strings are equal, which is the same bargain
/// `MIN_PASSWORD_CHARS` makes with the three surfaces that read it.
pub const SYSTEM_OUTPUT: &str = "system";

/// What the API says when the active output has no level of its own.
///
/// Coded, because a page has to say it in the reader's language, and it is a sentence somebody in
/// the room reads rather than a fault in the request: an HDMI output hands the volume to a receiver,
/// and no amount of asking differently will give it one.
pub const NO_OUTPUT_LEVEL: &str = "no_output_level";

/// One output device, as the API reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOutput {
    /// A stable identifier, opaque to a remote and safe to store.
    pub id: String,
    /// What to show a person.
    pub name: String,
    /// Whether this is the device the system currently calls its default.
    ///
    /// **Not the same as being the [`SYSTEM_OUTPUT`] row, and the difference has already cost
    /// something.** This marks the real device that *follow the system* resolves to today, so
    /// somebody choosing the sentinel can see what they are actually choosing; the sentinel row
    /// carries `false`, and is identified by its id. Reading this flag as *is the sentinel* is what
    /// labels the onboard card "Follow the system" and leaves the real sentinel showing an
    /// untranslated name.
    pub system_default: bool,
    /// Whether it is a USB interface.
    pub usb: bool,
    /// Whether it is present right now. A saved device that has been unplugged is listed with this
    /// `false` rather than hidden, so a remote can say so.
    pub available: bool,
    /// Whether this is the entry to *show* for the hardware it addresses.
    ///
    /// A backend can spell one physical output many ways and describe every spelling identically —
    /// on the appliance one headphone jack arrives ten times under the same words, and the whole
    /// list runs past thirty rows. `true` marks one row per output; `false` marks another way of
    /// naming one of them.
    ///
    /// Listed rather than dropped, the same bargain [`available`] makes: any of them can be chosen,
    /// one of them may be what settings already name, and `PUT /audio/output` takes all of them.
    /// A client shows the marked rows and puts the rest behind something.
    ///
    /// [`available`]: Self::available
    pub preferred: bool,
}

/// Which General MIDI bank the machine is playing through, and why.
///
/// The HTTP answer to the `soundfont` line `--show-paths` prints, and the one question the audio
/// device report above only implies: a machine whose instruments sound like sine waves is playing
/// through a device perfectly well, and nothing else on this surface says why.
///
/// The vocabulary is this crate's own, for the reason [`AudioOutputs`] gives — `km-api` must not
/// gain an audio dependency to describe a bank, and a `TestMachine` with no synthesizer has to be
/// able to answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SoundFontStatus {
    /// The bank actually loaded. `None` when none is.
    ///
    /// A full path, like [`PackageProblem::path`] and [`Origin::File`]: whoever is looking at this
    /// is diagnosing a machine, and a bare file name cannot tell a bundled bank from an override.
    pub path: Option<String>,
    /// Whether a setting named it or the bundled rule found it. `None` when no bank loaded.
    pub chosen_by: Option<SoundFontChoice>,
    /// What the machine is actually making sound with.
    pub playing: SoundKind,
    /// Why there is no bank, or no device — in the words the engine would log.
    pub problem: Option<String>,
    /// Why a bank the setting named is **not** what is playing, on a machine that is otherwise fine.
    ///
    /// Set when `audio.soundfont` names a bank the SoundFont folder no longer holds: the bundled
    /// bank is playing, everything works, and the setting is stale. `None` on a machine whose
    /// setting resolved and on one that never had a setting.
    ///
    /// **Deliberately not folded into [`Self::problem`]**, which `describe_soundfont` already
    /// settled for the defect list one field above: `problem` means *why there is no bank*, and a
    /// client that shows it would report a perfectly working machine as broken. This is a caveat
    /// about a working machine, so it needs somewhere of its own.
    pub fallback: Option<String>,
}

impl SoundFontStatus {
    /// What is wrong with the sound, in one sentence, or `None` on a machine that is fine.
    ///
    /// **The three-way reading lives here rather than in each surface that needs it.** It was the
    /// idle screen's alone until the owner's page grew somewhere to report faults, and a second
    /// copy of it would have been two crates describing one machine two different ways — for a
    /// distinction that is genuinely easy to get wrong: [`Self::problem`] means *why there is no
    /// bank* and reads as a broken machine, while [`Self::fallback`] is a caveat about a machine
    /// that is working perfectly well on the bundled bank.
    ///
    /// So: what is playing decides. A working synthesizer reports only the stale-setting caveat; a
    /// test tone and silence each report the reason with a phrase that says how bad it is, because
    /// `problem` on its own does not distinguish *instruments will sound wrong* from *nothing will
    /// come out*.
    pub fn complaint(&self) -> Option<String> {
        match self.playing {
            SoundKind::SoundFont => self.fallback.clone(),
            SoundKind::TestTone => Some(format!(
                "no SoundFont, so instruments will sound wrong: {}",
                self.problem
                    .clone()
                    .unwrap_or_else(|| "no bank loaded".to_owned())
            )),
            SoundKind::Silent => Some(format!(
                "no sound: {}",
                self.problem
                    .clone()
                    .unwrap_or_else(|| "no audio device".to_owned())
            )),
        }
    }
}

/// Every bank the machine could be switched to, and which one it will start on.
///
/// Separate from [`SoundFontStatus`] because they answer different questions and can genuinely
/// disagree: that one says what is sounding *now*, this one says what the setting names. A debug
/// slot swaps the first without touching the second, by decision.
///
/// The vocabulary is this crate's own for the reason [`AudioOutputs`] gives — `km-api` must not gain
/// an audio dependency to describe a bank.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SoundFontBanks {
    /// Every bank that could be chosen now, the bundled one first and the rest by name.
    pub banks: Vec<SoundFontBank>,
    /// The id of the bank the setting names, which is what the machine will come back on.
    pub selected: String,
    /// Banks the machine knows how to fetch and does not have.
    ///
    /// **A separate list rather than more rows in `banks`, because they are not the same kind of
    /// thing**: one is a file on this machine that can be played now, the other is a row in a table
    /// that would have to be downloaded first. Joining them would mean one list whose entries answer
    /// "play me" and "get me" through the same control.
    pub offers: Vec<SoundFontOffer>,
    /// What the downloader is doing, if anything.
    pub fetching: Option<SoundFontFetch>,
}

/// A bank this machine could fetch, from the table it was built with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundFontOffer {
    /// The name in the table — `sc55-v37`. Not the id an installed bank has; these are different
    /// namespaces because they identify different things.
    pub id: String,
    /// The file it would arrive as.
    pub name: String,
    /// Its size, as the research note writes it — `103.4 MiB`.
    pub size: String,
    /// Exact bytes, for a progress bar that has an end before the server says anything.
    pub bytes: u64,
    /// What was found about its terms, in the file's own words where it states any.
    ///
    /// **Shown on the row, not buried**: it is one of the things somebody choosing between sixty-odd
    /// banks is choosing on, so it belongs beside the name rather than a click away.
    pub license: String,
    /// One line of what the research note concluded about how it sounds.
    pub note: String,
    /// Whether the machine can fetch it at all. `false` for a bank the table holds no direct URL
    /// for — the row is shown with its page instead of a button.
    pub fetchable: bool,
    /// Where a person goes for a bank the machine cannot fetch.
    pub page: Option<String>,
    /// The one bank the machine suggests, marked on its row.
    ///
    /// **A mark on a row rather than a shorter list**, so the suggestion arrives beside the size,
    /// the note and the terms rather than in place of them.
    pub recommended: bool,
    /// Whether this is one of the banks the machine *offers*, as against one it merely knows about.
    ///
    /// Always `true` in the default answer, because that answer holds nothing else. It is worth a
    /// field anyway: a caller asking for the whole catalog gets both kinds in one list, and the
    /// difference between "this is what the machine suggests" and "this exists and was measured" is
    /// the difference the `rank` field was added to draw.
    pub offered: bool,
}

/// A download in progress, or the one that just finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundFontFetch {
    /// The table id of the bank.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// `working`, `done` or `failed`.
    pub state: SoundFontFetchState,
    /// Bytes fetched so far, while working.
    pub done: u64,
    /// Bytes expected.
    pub total: u64,
    /// Why it failed, worded for a person.
    pub problem: Option<String>,
}

/// The three things a download can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundFontFetchState {
    /// Running now.
    Working,
    /// Finished, verified and installed.
    Done,
    /// Did not finish, and nothing was installed.
    Failed,
}

/// One bank a machine can be switched to, as the API reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundFontBank {
    /// A stable identifier, opaque to a remote and safe to store.
    pub id: String,
    /// What to show a person.
    pub name: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Whether this is the bank that ships with the machine, which cannot be removed.
    pub bundled: bool,
    /// Why [`Controller::delete_soundfont`] would refuse this bank, or `None` if it would not.
    ///
    /// The bank twin of [`Catalog::why_not_removable`], and it exists for the sharper of the two
    /// reasons: `bundled` alone is not the whole rule, so a bank a `debug.soundfonts` slot names
    /// would otherwise be offered a Remove control that could only ever be refused.
    ///
    /// **Permanent refusals only.** A download in flight is a 409 at the point of use and is not
    /// here; a control that vanishes and comes back is a page that looks broken.
    pub why_not_removable: Option<String>,
}

/// How the bank that is playing came to be the one that is playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundFontChoice {
    /// `audio.soundfont` in settings named it, and the folder holds it.
    Setting,
    /// No setting, so the first bundled candidate that exists won.
    Bundled,
    /// A setting named a bank the folder no longer holds, so the bundled one is playing.
    ///
    /// A variant of its own rather than reporting [`Self::Bundled`], because those are two
    /// different machines: one is on the bundled bank because nobody chose otherwise, and this one
    /// is on it *despite* somebody having chosen. A client switches on this; the sentence to show
    /// is [`SoundFontStatus::fallback`].
    Fallback,
}

/// The three states a karaoke machine's sound can be in.
///
/// A machine with no bank is **not** a machine that cannot play: the lyrics still scroll in time,
/// which is most of what it does. Only the last of these refuses a song.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SoundKind {
    /// A real General MIDI bank is loaded and instruments sound like instruments.
    #[default]
    SoundFont,
    /// No bank, so a sine synthesizer stands in and everything sounds wrong.
    TestTone,
    /// No audio device at all. Playback is refused; everything else works.
    Silent,
}

/// Catalog queries and package management.
pub trait Catalog: Send + Sync + 'static {
    /// Searches the catalog.
    fn search(&self, query: &SearchQuery) -> Result<Vec<CatalogSong>, CatalogError>;

    /// One song by number.
    fn song(&self, number: SongCode) -> Result<Option<CatalogSong>, CatalogError>;

    /// One song by the package it came from and what its content hashes to.
    ///
    /// At most one row: two songs in one package with the same hash are a
    /// `ManifestProblem::DuplicateContent`, which is refused when a package is opened and when one
    /// is written. **Neither half of this key moves when a bank does**, which is what it is for —
    /// it is how a favorite is found again after a package has been renumbered underneath it.
    fn song_in_package(
        &self,
        package_id: &str,
        content_hash: &str,
    ) -> Result<Option<CatalogSong>, CatalogError>;

    /// One *recording* by what it hashes to, wherever it is filed.
    ///
    /// Two packages holding the same recording is reported at install rather than refused, so this
    /// may have several rows to choose between and must choose the same one every time: the lowest
    /// number.
    fn song_by_content(&self, content_hash: &str) -> Result<Option<CatalogSong>, CatalogError>;

    /// Whether any song came from this package.
    ///
    /// Asked only on the way to reporting a miss, to tell *"that package is not installed"* apart
    /// from *"that package does not hold that recording"* — two misses with different remedies.
    fn has_package(&self, package_id: &str) -> Result<bool, CatalogError>;

    /// Parses a catalog song out of its package.
    ///
    /// Separate from [`Catalog::song`] because it is far more expensive — it opens the archive and
    /// parses the MIDI — and only the lyrics endpoint needs it.
    fn load(&self, number: SongCode) -> Result<Option<Arc<Song>>, CatalogError>;

    /// Every installed package.
    fn packages(&self) -> Result<Vec<InstalledPackage>, CatalogError>;

    /// Packages the machine found and could not install, with the reason for each.
    ///
    /// Defaults to none, so a test double that has no notion of a packages folder needs no answer.
    fn package_problems(&self) -> Vec<PackageProblem> {
        Vec::new()
    }

    /// Why this refused package's file is not the machine's to delete, or `None` if it is.
    ///
    /// The same question, asked the same way and answered with the same sentence, as
    /// [`Catalog::why_not_removable`] asks about an *installed* package — and it is the same
    /// predicate underneath, so a page that leaves a control out leaves it out for exactly the
    /// reason [`Catalog::delete_problem_file`] would have refused with.
    ///
    /// **The default is a refusal, where [`Catalog::why_not_removable`]'s is `None`, and the
    /// asymmetry is the point.** That one is safe to default open because `uninstall` has no
    /// default at all, so every implementor writes it. This pair both have defaults, so a host that
    /// answered [`Catalog::package_problems`] and nothing else would otherwise draw a Delete
    /// control on every row that could only ever be refused — which is the exact shape
    /// `A control that can only be refused is left out, not grayed` forbids. The two defaults are
    /// written to agree, and the sentence is the same one.
    fn why_problem_not_removable(&self, problem: &PackageProblem) -> Option<String> {
        let _ = problem;
        Some(NO_PROBLEM_DELETION.to_owned())
    }

    /// How large a refused package's file is, for a page about to offer to delete it.
    ///
    /// [`Catalog::package_bytes`]' twin, and read at the same moment and for the same reason: the
    /// difference between 34 KiB and twenty gigabytes matters only when somebody is being asked.
    fn problem_bytes(&self, problem: &PackageProblem) -> Option<u64> {
        let _ = problem;
        None
    }

    /// Deletes a refused package's file, and forgets the note about it.
    ///
    /// **The one thing that was missing.** A refused package never entered the catalog, so
    /// [`Catalog::uninstall`] cannot reach it — that one looks a path up from rows which do not
    /// exist and answers `NotFound`. Without this, a file that will not open is refused again at
    /// every scan and goes away only by reaching the machine's filesystem, which is exactly what
    /// the appliance this is for cannot offer.
    ///
    /// **Named for the file and not for the note**, because `Machine::forget_package_problem`
    /// already exists and means only the second: this deletes something off a disk, and a name the
    /// two could share would be the worst kind of neighbor.
    ///
    /// `id` is a [`PackageProblem::id`], resolved against a fresh list rather than joined onto a
    /// folder. The variants each mean one thing here:
    ///
    /// - [`CatalogError::NotFound`] — *nothing is refusing that any more*. The file may have been
    ///   fixed and installed, or removed by other means, since a page was drawn. All the cases are
    ///   indistinguishable and all of them are fine, so it is worded as news rather than as a fault.
    /// - [`CatalogError::Rejected`] — the file is not the machine's to delete, in the very sentence
    ///   [`Catalog::why_problem_not_removable`] gives.
    /// - [`CatalogError::Failed`] — the file is there and would not go: permissions, a read-only
    ///   mount, a lock.
    /// - [`CatalogError::Unavailable`] is **never** returned. `uninstall` needs it because re-keying
    ///   can race the queue; nothing queued can name a package that never installed.
    fn delete_problem_file(&self, id: &str) -> Result<(), CatalogError> {
        let _ = id;
        Err(CatalogError::Rejected(NO_PROBLEM_DELETION.to_owned()))
    }

    /// Installs a package that is **already in a folder the machine scans**, indexing it in place.
    ///
    /// The startup pass and `debug.packages` use this. Anything handed the machine from outside
    /// wants [`Catalog::install_copied`] instead, so that it lands somewhere a later pass will
    /// find it.
    fn install(&self, path: &Path) -> Result<InstallReport, CatalogError>;

    /// Installs a package the machine has been **handed**, copying it into the packages folder first.
    ///
    /// The difference from [`Catalog::install`] is which of two verbs is meant — *index the file
    /// in my folder*, or *take this file* — and it must not be folded into one. Copying
    /// unconditionally would copy `debug.packages` entries too, which is the machine duplicating a
    /// build output the owner is only pointing at, possibly tens of gigabytes of it.
    ///
    /// A file that is already directly inside a scanned folder is installed where it lies rather
    /// than copied beside itself, so this is safe to call for something that turns out to be in the
    /// right place already — which is what lets the appliance's deploy keep posting paths after it
    /// has moved the files in itself.
    ///
    /// The package is opened for its id **before anything is copied**, so a file that will not
    /// open costs a manifest read rather than a copied archive left in the folder for the next pass
    /// to trip over. The id is what decides whether an existing file of the same name is an upgrade
    /// or a different package needing a name of its own.
    fn install_copied(&self, path: &Path) -> Result<InstallReport, CatalogError>;

    /// Reads the packages folders again and makes the catalog agree with them.
    ///
    /// **What a restart does, without the restart.** Adding always happens; *removing* waits for the
    /// machine to be idle, and only when there is something to remove — so somebody who has just
    /// dropped a file into the folder is never made to wait, while a package whose file was taken
    /// away mid-party keeps its rows until nothing is playing. Those ids come back as
    /// [`RescanReport::deferred`] rather than being silently held, so a client can tell "nothing was
    /// missing" from "not yet".
    ///
    /// Not a 409 when work is deferred: the additive half succeeded, and reporting nothing happened
    /// when something did is the worse answer.
    fn rescan(&self) -> Result<RescanReport, CatalogError>;

    /// Removes a package and the songs it contributed. Returns how many songs went.
    ///
    /// **This deletes the `.kmpkg` file**, and the file goes before the rows do. The folders a
    /// machine scans are what say what is installed, so removing the rows while the file stayed
    /// would mean the next pass putting the package straight back — an uninstall that lasted until
    /// the next restart, which is not what any remote's button says. If the file cannot be removed
    /// nothing is uninstalled at all, and the error says so.
    ///
    /// Refused for a package reached through `debug.packages`: that names a file the owner keeps
    /// somewhere of their own, and neither it nor the entry is the machine's to remove. Refused
    /// rather than half-done, because dropping the rows alone would not stick — the next pass reads
    /// the same list and puts the package back.
    fn uninstall(&self, package_id: &str) -> Result<usize, CatalogError>;

    /// Why [`Catalog::uninstall`] would refuse this package, asked **before** offering to do it.
    ///
    /// `None` means it would go through. A sentence means it would not, and it is the same sentence
    /// the refusal carries — the implementation asks the one function `uninstall` asks, so a page
    /// cannot describe a rule differently from the route that enforces it.
    ///
    /// **A page spends this by leaving the control out**, not by graying it: both refusals are
    /// permanent, and a button that is always refused teaches somebody to ignore the row it is in.
    /// That is the opposite of [`AudioOutputs::changeable`], which grays a control that will become
    /// available when the song ends.
    ///
    /// **Takes the row rather than an id**, because the caller already has it and the path is the
    /// whole input: asking by id would make the implementation read the package list again, once
    /// per row, under a lock a page load takes care to stay off.
    ///
    /// **The sentence names a path, so it does not go on the wire.** `PackageDto` carries a
    /// `removable` boolean and nothing else — the same rule that keeps the archive's path off
    /// [`PackageDto`](crate::dto::PackageDto) and reduces a package problem to a bare file name.
    /// The owner's own page reads this in-process and may say all of it.
    ///
    /// Defaulted to `None` so a double with no packages folder — and no notion of a `debug.` section
    /// — needs to know nothing about this.
    fn why_not_removable(&self, package: &InstalledPackage) -> Option<String> {
        let _ = package;
        None
    }

    /// How large a package's archive is on disk, for a page about to offer to delete it.
    ///
    /// `None` when the file cannot be measured. Not a field on [`InstalledPackage`], which is a row
    /// out of an index of songs and has never counted bytes: a stored number goes stale, and a
    /// `metadata` call inside [`Catalog::packages`] would put one filesystem touch per package on
    /// a read path that has none. Read instead at the moment somebody is being asked to confirm,
    /// which is the only moment the difference between 34 KiB and twenty gigabytes matters.
    fn package_bytes(&self, package: &InstalledPackage) -> Option<u64> {
        let _ = package;
        None
    }

    /// Moves a package to another block of a thousand.
    ///
    /// Returns how many songs were re-keyed. **Every song in the package changes its number**, which
    /// is why the implementation refuses while anything is playing or queued — the same rule, for
    /// the same reason, that guards changing the audio output device.
    fn set_package_bank(&self, package_id: &str, bank: u16) -> Result<usize, CatalogError> {
        let _ = (package_id, bank);
        Err(CatalogError::Rejected(
            "this machine cannot move a package to another bank".to_owned(),
        ))
    }

    /// How many songs are catalogd.
    fn song_count(&self) -> Result<usize, CatalogError>;

    /// One page of the whole catalog, in number order, for a client keeping its own copy.
    ///
    /// `after` is the last number of the previous page — **keyset paging, never an offset**, because
    /// SQLite walks an offset row by row and a six-figure catalog paged that way costs time
    /// proportional to the square of its size.
    fn export(
        &self,
        after: Option<SongCode>,
        limit: usize,
    ) -> Result<Vec<CatalogSong>, CatalogError>;

    /// How many times the catalog has changed.
    ///
    /// Moves on every install and uninstall, and on nothing else, so a client that mirrored the
    /// catalog can tell whether re-reading it would find anything new. Meaningless between two
    /// machines — see the note on [`km_catalog::Library::catalog_version`].
    fn catalog_version(&self) -> Result<u64, CatalogError>;

    /// The artists, with how many songs each has, optionally narrowed by name.
    ///
    /// For a remote's artist list. Not derivable from [`search`](Self::search): that pages over
    /// *songs*, so building this from it would mean reading the whole catalog to count what is
    /// already one `GROUP BY`.
    ///
    /// `hidden` leaves out the songs of packages a person hid on their remote, as
    /// [`km_catalog::SearchQuery::exclude_packages`] does for a search. It means the same in
    /// [`Self::languages`] and [`Self::tags`].
    fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<(String, usize)>, CatalogError>;

    /// The languages present, with how many songs are in each.
    ///
    /// Ordered by how much of the catalog each accounts for, which is the order a picker wants —
    /// the one language nearly everything is in belongs at the top, not wherever its ISO code
    /// happens to sort.
    fn languages(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, CatalogError>;

    /// The tags present, with how many songs carry each.
    ///
    /// Ordered like [`Self::languages`] and for its reason. There is no table of legal tags
    /// anywhere, so this question asked of the songs is the only statement of what the vocabulary
    /// *is* — which is what the remotes draw their picker from.
    fn tags(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, CatalogError>;
}

/// What a curation tool has already settled about a song it is sending to be heard.
///
/// **Every field is separately absent, and absent is not empty.** A machine given nothing works the
/// song out from the file, exactly as it does for one played from a command line; a machine given a
/// value uses it. The distinction is load-bearing for the corrections in particular: a song whose
/// corrections were deliberately turned off has to sound different from a song nobody has touched,
/// which is what somebody unticking a box is listening for.
///
/// **It is a struct because a preview is a growing list of things the file does not know.** A
/// curator's title, their performer and their corrections live in a database and nowhere in the
/// bytes, and a fourth is more likely than not — three loose parameters on two trait methods is
/// where that stops being readable.
#[derive(Debug, Clone, Copy, Default)]
pub struct Audition<'a> {
    /// The title to show, in place of whatever the file calls itself.
    pub title: Option<&'a str>,
    /// The performer to show, in place of whatever the file claims.
    pub artist: Option<&'a str>,
    /// The key to play it in, as the song's own stored transposition.
    ///
    /// It lands where a packaged song's does, so the operator's own default is added on top of it
    /// and a preview sits in the same key the song would once it is in a package. `None` is the
    /// file's own key, which is what a song nobody has transposed plays in.
    pub transpose: Option<i8>,
    /// The corrections to play it with, in place of whatever this machine would detect.
    pub fixes: Option<&'a [km_fixes::Fix]>,
    /// The melody channel to play it with, 0-based, in place of whatever this machine would detect.
    ///
    /// Three states, as in a package description: `None` detects, `Some(None)` says the song has no
    /// melody channel, and `Some(Some(channel))` names one. The guide-melody toggle is offered only
    /// on a song with a channel, so both inner states change what the preview offers.
    pub melody: Option<Option<u8>>,
    /// The words of an UltraStar song, read out of its `.txt` by the sender.
    ///
    /// Present only with the song's MP3, which is then played as an UltraStar song. The machine never
    /// reads an UltraStar file, so the words arrive as the timeline a package stores.
    pub lyrics: Option<&'a km_song::LyricTimeline>,
}

impl Audition<'_> {
    /// Whether nothing at all has been decided, so there is nothing for a request to carry.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.transpose.is_none()
            && self.fixes.is_none()
            && self.melody.is_none()
            && self.lyrics.is_none()
    }
}

/// Playback, the queue, and the settings that belong to a performance.
///
/// # Every method is required, and none of them refuses by default
///
/// **A capability declined by *omission* is indistinguishable from one somebody forgot**, and what
/// the client is told is a 409 either way. So no method carries a default body returning
/// `ControlError::Unavailable("this machine cannot …")`, and a silent no-op is not on offer
/// anywhere: an implementor writes all fourteen, the real machine included and the in-memory double
/// with it.
///
/// Splitting this into capability supertraits (`Playback` required, `SoundFonts`, `Wallpapers`,
/// `Demo` optional, reached through `fn soundfonts(&self) -> Option<&dyn …>`) is rejected on the
/// same ground: it is structure for a variation with no instances. If a host genuinely cannot do
/// one of these, the honest shape is a route that is **not mounted** — which
/// [`crate::routes`] already does for `debug/play-file`, so a machine without debugging answers a
/// real 404 rather than a 409 about a method that quietly did nothing.
pub trait Controller: Send + Sync + 'static {
    /// Everything a remote needs to redraw, in one read.
    fn snapshot(&self) -> Snapshot;

    /// The queue, in play order.
    fn queue(&self) -> Vec<QueueEntry>;

    /// Queues a song. Returns the new entry's id.
    fn queue_add(&self, request: QueueRequest) -> Result<u64, ControlError>;

    /// Drops a queued song by id.
    fn queue_remove(&self, id: u64) -> Result<QueueEntry, ControlError>;

    /// Moves a queued song to a position, clamped to the queue's bounds.
    fn queue_move(&self, id: u64, to_index: usize) -> Result<(), ControlError>;

    /// Empties the queue without touching what is playing.
    fn queue_clear(&self) -> Result<usize, ControlError>;

    /// Drives the transport.
    fn transport(&self, command: TransportCommand) -> Result<(), ControlError>;

    /// Applies a partial settings change, returning the settings as they now stand.
    fn update_settings(&self, patch: &SettingsPatch) -> Result<Settings, ControlError>;

    /// The microphone channels.
    fn mics(&self) -> Vec<MicChannel>;

    /// Applies a partial change to one microphone.
    fn update_mic(&self, id: &str, patch: &MicPatch) -> Result<MicChannel, ControlError>;

    /// Which output devices exist, and which one the machine is using.
    fn audio_outputs(&self) -> Result<AudioOutputs, ControlError>;

    /// Chooses the output device, and remembers the choice.
    ///
    /// `id` is either an identifier from [`Controller::audio_outputs`] or the "follow the system"
    /// sentinel. Unlike everything else on this trait, this is *installation* configuration rather
    /// than a knob that belongs to a performance — which is why it is a route of its own, and why
    /// the shipped ACL makes it admin.
    ///
    /// **Refused unless nothing is playing**, with [`ControlError::Unavailable`]. The player lives
    /// inside the audio stream that a change has to drop, so a change mid-song would take the song
    /// with it; and an idle machine with a queued song is a machine about to start one.
    fn set_audio_output(&self, id: &str) -> Result<AudioOutputs, ControlError>;

    /// Whether this host can reach the operating system's mixer at all.
    ///
    /// Read once, while the router is built, to decide whether `PUT /admin/audio/level` is mounted:
    /// a host with no mixer to reach answers 404 rather than carrying a route that could only
    /// refuse. The same arrangement `Capabilities::power` has, and for the same reason.
    ///
    /// **Not the same question as whether the active device has a level**, which is
    /// [`AudioOutputs::level`] and moves with the device. A machine can be able to ask and be
    /// playing through an HDMI output that has nothing to answer.
    ///
    /// Defaulted to `false`, so a host that has not thought about it is described accurately.
    fn output_level_supported(&self) -> bool {
        false
    }

    /// Moves the active output's own level, and reports where it landed.
    ///
    /// Installation configuration, like [`Controller::set_audio_output`] above it and for the same
    /// reason: this is the gain between the machine and the amplifier, set once when a room is
    /// balanced, not a knob anybody reaches for between songs. The four things a guest's phone may
    /// change stay the key, the tempo, the volume and the guide melody.
    ///
    /// **Not refused while a song plays.** Unlike the device, the level is a property of the sound
    /// card rather than of the stream, so moving it disturbs nothing and needs no `changeable`.
    ///
    /// **The request is clamped into the control's range rather than refused**, because a caller
    /// drawing a slider from an earlier reading can be a step out of date without being wrong. What
    /// comes back is what the hardware reports afterwards, which a control with coarse steps will
    /// round.
    ///
    /// [`ControlError::Unavailable`] where the active device has no level, which is the honest
    /// answer for an HDMI output: the receiver holds the volume and the card has nothing to
    /// attenuate.
    ///
    /// **Defaulted to that refusal**, so a host with no mixer to reach — every test double, and a
    /// build for a platform whose applications cannot move one — says so rather than claiming a
    /// level it does not have.
    fn set_output_level(&self, db_centi: i32) -> Result<AudioOutputs, ControlError> {
        let _ = db_centi;
        Err(ControlError::Unavailable(Refusal::coded(
            NO_OUTPUT_LEVEL,
            "this output has no level to set",
        )))
    }

    /// Which bank is playing, and why.
    ///
    /// Read-only and unfailing, like [`Controller::wallpapers`]: it is a report on a decision the
    /// machine made when it started, and there is nothing here to refuse.
    ///
    /// **Deliberately has no default implementation.** A default would have to claim a bank is
    /// loaded or that none is, and both are lies a test double would then tell quietly — which is
    /// the whole failure this exists to make visible.
    fn soundfont(&self) -> SoundFontStatus;

    /// Every bank that could be chosen, and which one the setting names.
    ///
    /// Read-only and unfailing, like [`Controller::soundfont`] above it. A folder that cannot be
    /// read is reported as the bundled bank alone, which is the truth about what will play.
    ///
    /// Defaulted, unlike its neighbor, and the asymmetry is deliberate: an empty list is not a lie.
    /// A controller that has no banks to offer — every test double, and any future host that does
    /// not keep a folder of them — is accurately described by "nothing to choose from", where
    /// `soundfont`'s default would have had to claim a bank was or was not loaded.
    ///
    /// **`all` widens `offers` to the whole catalog and changes nothing else.** The default answer
    /// is the shortlist the machine offers — nine ranked banks, minus the bundled one and minus
    /// whatever is already installed — because that is what a singer should be shown without
    /// asking. A caller that says `all` gets the rows the shortlist leaves out as well, each marked
    /// by [`SoundFontOffer::offered`]; the bundled row and the installed ones stay filtered out,
    /// since neither is something to fetch. Nothing about it is privileged: it is the same
    /// `audio.read` answer at a different width, and `fetch_soundfont` has never been gated by rank
    /// either.
    fn soundfonts(&self, _all: bool) -> SoundFontBanks {
        SoundFontBanks::default()
    }

    /// Chooses a bank and keeps the choice, putting it in force without stopping the song.
    ///
    /// Refuses with [`ControlError::Rejected`] for an id that is not in the list, and for a bank
    /// that will not open — which is a real case rather than a defensive one, since this
    /// synthesizer refuses banks other players accept and the file may have changed on disk since
    /// the list was built.
    ///
    fn set_soundfont(&self, id: &str) -> Result<(), ControlError>;

    /// Starts fetching one of the banks [`soundfonts`](Controller::soundfonts) offers.
    ///
    /// Returns as soon as the download has *started*; how it goes is reported through
    /// `soundfonts().fetching`. A network failure is therefore never this function's error — by the
    /// time it happens the request that asked for it has long been answered.
    ///
    /// Refuses with [`ControlError::Unavailable`] when one is already running, and with
    /// [`ControlError::Rejected`] for a `manual` row: there is no URL for those, because the
    /// publisher serves the file through a page rather than a direct address.
    ///
    fn fetch_soundfont(&self, id: &str) -> Result<(), ControlError>;

    /// Removes an installed bank from disk.
    ///
    /// **The id is one of [`SoundFontBank::id`], not one of [`SoundFontOffer::id`]** — this deletes
    /// a file that is here, not a row in the catalog, and the two are different namespaces.
    ///
    /// Refuses with [`ControlError::Rejected`] for an id that is not in the list and for the
    /// bundled bank, which is an unpacked asset that would return on the next launch; and with
    /// [`ControlError::Unavailable`] while a download is running, so a delete cannot race the
    /// rename that finishes one.
    ///
    /// **Deleting the bank the setting names is allowed, and falls back to the bundled one.** The
    /// alternative — refusing until something else is selected — reads tidier and is wrong for the
    /// case this exists for: on a television the selected bank may *be* the 262 MiB mistake, and a
    /// refusal is a dead end for somebody holding a D-pad. The fallback goes through the same
    /// selection path as choosing the bundled bank by hand, so the level protocol is the existing
    /// one rather than a second copy of it.
    ///
    fn delete_soundfont(&self, id: &str) -> Result<(), ControlError>;

    /// Whether the machine performs for itself when nobody is singing, and what it is set to.
    ///
    /// Read-only and unfailing, like [`Controller::wallpapers`]. Defaulted to "off, and nothing to
    /// report", which is an accurate description of a host that has no demo mode rather than a lie
    /// about one — the asymmetry with [`Controller::soundfont`], which deliberately has no default,
    /// is the same one [`Controller::soundfonts`] draws: an empty answer is honest here and would
    /// not be there.
    fn demo(&self) -> DemoState {
        DemoState::default()
    }

    /// Turns demo mode on or off.
    ///
    /// **`persist` is the whole shape of this route.** Without it the change lasts for the run: a
    /// party is a run, and somebody switching the machine on for an evening should not have to
    /// remember to switch it back. With it the answer is written to settings and survives a
    /// restart. Both are deliberate acts and neither is a default, so the caller says which.
    ///
    /// Written to disk immediately when asked, for the reason [`Self::set_debug_enabled`] gives:
    /// somebody is standing in front of the thing changing what it does, and a power cut before the
    /// next clean stop must not quietly undo it.
    ///
    /// Returns the state as it now stands, so a client needs no second request to redraw.
    ///
    fn set_demo(&self, enabled: bool, persist: bool) -> Result<DemoState, ControlError>;

    /// Sets how long the machine waits, in seconds, before it performs for itself.
    ///
    /// **Its own call rather than a third argument to [`Self::set_demo`], and `persist` is the
    /// reason.** That route's whole shape is *for tonight or for good*, because a party is a run.
    /// A delay is not a party: it is installation configuration, in the same family as the
    /// machine's name and its locale, and it is **always written to settings**. Folding it into a
    /// body where its neighbour obeys `persist` and it did not would make one field of two mean
    /// something the other does not.
    ///
    /// **An armed countdown is shifted rather than re-armed**, which is the same rule
    /// [`Self::set_demo`] states from the other side: the clock counts *idleness*. A machine that
    /// has been quiet for fifty seconds and is told to wait sixty has ten to go — not sixty — and a
    /// machine told to wait thirty has already missed its deadline and sings on the next poll.
    /// Re-arming from now would mean shortening the delay made the wait longer, exactly once, in
    /// the moment somebody was watching to see whether it worked.
    ///
    /// Refuses with [`ControlError::Rejected`] above [`MAX_DEMO_DELAY_SECS`]. Zero is accepted and
    /// means *as soon as the machine goes idle* — `enabled` is this feature's off switch and a
    /// second one would only configure a mode that does nothing.
    ///
    /// Returns the state as it now stands, so a client needs no second request to redraw.
    ///
    /// Defaulted to a refusal, so a `Controller` written before this route existed still compiles
    /// and answers honestly rather than silently accepting a number it will never act on.
    fn set_demo_delay(&self, delay_secs: u32) -> Result<DemoState, ControlError> {
        let _ = delay_secs;
        Err(ControlError::Unavailable(
            "this machine has no demo mode to delay".into(),
        ))
    }

    /// Plays one song the machine chooses, now.
    ///
    /// **A one-shot and not the mode**, which is why it does not need [`Self::set_demo`] to have been
    /// called first: chaining is what `demo.enabled` buys, so with the mode off this is exactly one
    /// song and the machine goes quiet again afterwards. That difference is what makes this
    /// something anybody in the room may ask for where turning the mode on is an owner's act. That
    /// is why `POST /api/v1/demo/start` sits outside `/api/v1/admin/` and `PUT /api/v1/admin/demo`
    /// does not — see [`crate::routes`].
    ///
    /// Refuses with [`ControlError::Unavailable`] when something is loaded, when the queue is not
    /// empty, or when the machine cannot play at all. All three synchronously, even though the song
    /// itself starts a moment later on the machine's own thread — **what it cannot refuse is a
    /// catalog with nothing playable in it**, because discovering that means a full-table draw and
    /// this route exists to keep that off the request path. A machine with no songs says so
    /// everywhere else a client looks.
    ///
    /// Returns the state as it now stands, so a client learns whether the song it just asked for
    /// will be followed by another.
    fn start_demo_song(&self) -> Result<DemoState, ControlError>;

    /// The session epoch moved; write it somewhere it survives a restart.
    ///
    /// **A restart must not undo a sign-out.** A token is an HMAC over the stored password hash and
    /// this number, so an epoch that lived only in memory would come back as the old one after a
    /// power cut and every token an owner had just invalidated would start verifying again. That is
    /// the general rule for anything reached this way: a change that reaches the running machine and
    /// not the file is a security decision quietly reverted.
    ///
    /// Deliberately has no default implementation. A default would be a no-op, and a no-op here is
    /// exactly the bug this exists to close.
    fn set_session_epoch(&self, epoch: u64) -> Result<(), ControlError>;

    /// Turn debugging mode on or off, and write it down.
    ///
    /// Governs whether the two `debug/play-*` routes are mounted at all, and whether the whole
    /// `debug.` section of settings does anything. Persisted immediately, for
    /// [`Self::set_session_epoch`]'s reason.
    ///
    /// Deliberately has no default implementation, for that reason too: silently failing to record
    /// that somebody opened the debug surface is the wrong way round to fail.
    fn set_debug_enabled(&self, enabled: bool) -> Result<(), ControlError>;

    /// Turn the development console on or off, and write it down.
    ///
    /// Governs whether `/dev/` and the whole of [`crate::routes::DEV_API_PREFIX`] are mounted —
    /// together with debugging mode, which has to be on as well. Persisted immediately, for
    /// [`Self::set_session_epoch`]'s reason, and for one more: what this opens is an API that asks
    /// for no password at all, so a machine that forgot it had been turned *off* would be worse
    /// than one that forgot it had been turned on.
    ///
    /// Deliberately has no default implementation, for [`Self::set_debug_enabled`]'s reason.
    fn set_dev_remote_enabled(&self, enabled: bool) -> Result<(), ControlError>;

    /// Whether the frame-statistics panel is on the machine's screen.
    ///
    /// Defaulted off, following the reads beside it.
    fn performance_overlay(&self) -> bool {
        false
    }

    /// Put the frame-statistics panel on the machine's screen, or take it off.
    ///
    /// **The one switch on these pages that takes effect at once, and nothing is written down.**
    /// Its two neighbours decide which routes get mounted and so wait for a restart; this decides
    /// what one drawing function does with the next frame. Nothing is persisted because a
    /// diagnostic that survives a restart is a panel somebody left on for a month — and because the
    /// overlay's own decision rests on it stating nothing and changing nothing, which a settings
    /// entry would stop being true.
    ///
    /// It exists because `F12` is not a key the appliance has. *The screen looks choppy* is reported
    /// from a sofa and the box under the television has no keyboard at all, so the machine that most
    /// needs this had no way in.
    ///
    /// Has a default that does nothing, unlike the two writes above: a host with no display — the
    /// in-memory machine behind `dev_server`, an embedder using `km-api` alone — has no panel to
    /// draw, and a missing implementation there loses nothing an owner would notice.
    fn set_performance_overlay(&self, on: bool) -> Result<(), ControlError> {
        let _ = on;
        Ok(())
    }

    /// What the settings file says about the two developer switches.
    ///
    /// **The stored side of both, in one call.** A running machine's own values are on
    /// [`crate::ApiConfig`] and are a snapshot taken at start; these are what will be true after a
    /// restart, which is the only thing a switch for either of them can usefully draw. One method
    /// because the two are read together everywhere they are read at all — the console needs both,
    /// and so does the marker the television draws.
    ///
    /// Defaulted to both off, following the reads beside it: a test double should not have to know
    /// about a switch it never touches, and off is what a machine nobody has asked says.
    fn developer_switches(&self) -> DeveloperSwitches {
        DeveloperSwitches::default()
    }

    /// Write the machine's new name down, so it survives a restart.
    ///
    /// The same division [`Self::set_session_epoch`] draws, and the same failure it exists to
    /// prevent:
    /// `ApiState` holds the running name because it is what answers `/discover`, and only the
    /// controller knows where `settings.json` is. A rename that reached the advert and not the file
    /// would come back under the old name after a power cut, which is the shape of the bug found on
    /// the appliance in 2026-08.
    ///
    /// **Saved immediately rather than at shutdown**, joining the package install and the session-epoch write
    /// for their reason: it is a deliberate, durable act, and a shutdown hook is exactly what a
    /// power cut does not run.
    ///
    /// **Required rather than defaulted, and not a no-op either.** A host that keeps no settings has
    /// to say it cannot be renamed rather than accept a rename and forget it -- and, since the trait
    /// no longer lets a capability be declined by omission, it has to say so out loud.
    fn set_machine_name(&self, name: &str) -> Result<(), ControlError>;

    /// What language the television speaks now.
    ///
    /// Defaulted to English, which is what a host that keeps no settings speaks: every message is
    /// written in it first, so it is the answer rather than an absence.
    fn machine_locale(&self) -> km_locale::Locale {
        km_locale::Locale::default()
    }

    /// What language the television speaks, saved.
    ///
    /// **The same shape as [`set_machine_name`](Controller::set_machine_name), and beside it for the
    /// same reason**: both are facts the owner sets about this machine rather than about a request,
    /// both are written to `settings.json` immediately, and both are refused by a host that keeps no
    /// settings rather than accepted and forgotten.
    ///
    /// It takes effect on the next frame the display draws — every `Frame` carries the locale, so
    /// there is nothing to restart and nothing cached to invalidate.
    fn set_machine_locale(&self, locale: km_locale::Locale) -> Result<(), ControlError>;

    /// The wallpaper cycle.
    fn wallpapers(&self) -> WallpaperState;

    /// Advances to the next wallpaper immediately.
    fn next_wallpaper(&self) -> Result<(), ControlError>;

    /// What is in the wallpaper folder, one row per file.
    ///
    /// **Files, not images**, which is the whole of the model: a loose picture is one row and a zip
    /// of twenty is one row that says twenty. That is the same rule a package keeps — you remove the
    /// package, never a song inside it — and `Zipped wallpapers` in `docs/decisions/interface.md`
    /// already argued it from the other end: a collection arrives as one file to drop in, so it is
    /// one file to remove.
    ///
    /// Defaulted to empty, so a host that has no folder to show — the test harness, until it is
    /// given one — reports nothing rather than being made to implement a listing it has no use for.
    fn wallpaper_pictures(&self) -> Vec<Picture> {
        Vec::new()
    }

    /// Removes one file from the wallpaper folder.
    ///
    /// Never a no-op, for [`Self::set_machine_name`]'s reason: a host that cannot do this should say
    /// so rather than answer 200 and delete nothing.
    fn delete_wallpaper(&self, id: &str) -> Result<(), ControlError>;

    /// Loads and plays a MIDI file from the machine's own disk, bypassing the catalog.
    ///
    /// The debug path from the brief. Gated by the `debug.play_file` route id so it can be turned
    /// admin-only, and by the implementation's own idea of which directories are allowed — this
    /// crate does not decide that, because only `km-app` knows where the corpus lives.
    /// `decided` is what a curation tool has already settled about this file; every field of it is
    /// separately absent, and absent means the implementation works it out from the file exactly as
    /// it does for one played from a command line.
    fn play_file(&self, path: &Path, decided: &Audition<'_>) -> Result<(), ControlError>;

    // **There is no `accepts_uploads`/`set_accept_uploads` pair here, and debugging mode is why.** A
    // switch of its own for taking bytes is what a machine with no master switch needs: one on
    // `0.0.0.0` with no password has to be stopped from taking bytes from a stranger, and an access
    // list cannot do that without also breaking the curation tool. Every route that writes to disk
    // is behind `/api/v1/admin/` and a password always exists, so one switch — `set_debug_enabled`
    // — answers both questions, and `GET /discover` reports it as `debug_enabled`.

    /// A fresh, empty folder for one audition's files.
    ///
    /// The other half of [`Self::play_file`]'s division of labor, and split for the same reason:
    /// this crate does the HTTP and the streaming, and the implementation says *where*, because only
    /// `km-app` knows what its data directory is. Choosing the folder, creating it and sweeping what
    /// the last audition left are all its business.
    ///
    /// A machine that does not take uploads refuses with [`ControlError::Rejected`] naming whatever
    /// setting would permit them, exactly as [`Self::play_file`] refuses a path outside its allowed
    /// folders — the two are the same sentence about the same thing, and the wording belongs to the
    /// implementation because the setting does.
    fn open_audition(&self) -> Result<PathBuf, ControlError>;

    /// Writes the admin password's hash down, or removes it.
    ///
    /// **A hash and not a password**, because hashing is this crate's business — `AdminAuth` owns
    /// the argon2 parameters and the verification, and an implementation that was handed a plain
    /// password would have to know them to store it. `None` clears it.
    ///
    /// The same division [`Self::set_session_epoch`] and [`Self::set_machine_name`] draw, and the same
    /// reason: only the implementation knows where `settings.json` is. Saved immediately, joining
    /// them and the package install — a password that was lost to a power cut would leave a machine
    /// its owner believed was closed standing open.
    /// **Two arguments, because a password and *whose* password are different facts.**
    /// `factory_pin` is `Some` only when the machine generated this one for itself, and it is the
    /// plain text — the machine has to be able to draw it on its own screen, which a hash cannot do.
    /// `None` means an owner chose it, and the implementation clears any PIN it was holding. The two
    /// move together on purpose: a hash written without clearing the old PIN would leave a machine
    /// claiming to be on a factory password it no longer has.
    fn set_admin_password(
        &self,
        _hash: Option<String>,
        _factory_pin: Option<String>,
    ) -> Result<(), ControlError>;

    /// A fresh, empty folder to stream one owner upload into.
    ///
    /// [`Self::open_audition`]'s sibling and split for its reason: this crate does the HTTP and the
    /// streaming, and only the implementation knows where its data directory is.
    ///
    /// **Separate from the audition folder, and not gated the way that one is.** An audition is a
    /// song a curator is listening to before deciding, and `debug.accept_uploads` exists because
    /// that route is public by ACL and a machine on `0.0.0.0` with no password must not take bytes
    /// from a stranger. These are installation configuration and ship `admin`, so on a machine with
    /// a password they are already closed and on one without it every `admin` route is open
    /// anyway — a second setting would be one switch meaning two things, which is the fault
    /// `A machine takes no uploaded song until it is told to` was written to avoid.
    fn open_upload(&self) -> Result<PathBuf, ControlError>;

    /// Puts a file that has finished arriving where it belongs, and says what happened.
    ///
    /// `staged` is a path inside the folder [`Self::open_upload`] returned, so the containment rule
    /// is kept the way [`Self::play_audition`]'s is: the caller never names a destination.
    ///
    /// **Everything that has to happen afterwards happens here**, because it differs per kind and
    /// every one of them is the implementation's business: a package is installed into the
    /// catalog, a wallpaper has to make the display re-resolve which folder it is watching, and a
    /// SoundFont needs nothing at all because `soundfont::installed` reads the directory on every
    /// call. Doing this in the handler would put three different pieces of machine knowledge in the
    /// crate that deliberately has none.
    ///
    /// The sentence returned is for a person: `installed "Carols 1999" · 16 songs`.
    fn accept_upload(&self, kind: Upload, staged: &Path) -> Result<String, ControlError>;

    /// Plays a file previously written into the folder [`Self::open_audition`] returned.
    ///
    /// **`name` is a bare file name and nothing else** — no separator, no `..`, nothing absolute.
    /// That is what keeps the containment rule in one place: the caller never names a path, the
    /// implementation joins this onto the folder it chose itself, and a name with no separator in it
    /// cannot escape a join. A name that is not bare is refused rather than sanitised.
    ///
    /// One name, even for an MP3+G song. Both halves are written into that folder before this is
    /// called, and the machine finds the second beside the first exactly as it does for a pair
    /// sitting in a corpus — which is the whole reason an upload is staged in a folder at all.
    /// `decided` carries the same meaning it has on [`Self::play_file`].
    fn play_audition(&self, name: &str, decided: &Audition<'_>) -> Result<(), ControlError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refused package at a path, for the id tests below.
    fn refused(path: &str) -> PackageProblem {
        PackageProblem {
            path: path.to_owned(),
            package_id: None,
            reason: "could not read manifest.json".to_owned(),
        }
    }

    /// **The case the fingerprint exists for**, and an observed one: the same package in two of the
    /// folders the machine scans, listed twice with identical text. One id between them would mean
    /// one Delete control deleting whichever file was found first.
    #[test]
    fn two_problems_with_the_same_name_in_different_folders_get_different_ids() {
        assert_ne!(
            refused("/data/packages/carols.kmpkg").id(),
            refused("/tunes/karaoke/carols.kmpkg").id(),
        );
    }

    /// The id is a pure function of the path, which is what the GET-then-POST flow rests on.
    #[test]
    fn the_same_path_gives_the_same_id_every_time() {
        assert_eq!(
            refused("/data/packages/carols.kmpkg").id(),
            refused("/data/packages/carols.kmpkg").id(),
        );
    }

    /// It is safe in a URL and names no folder — the two things it is allowed to leak nothing about.
    #[test]
    fn a_problem_id_is_url_safe_and_names_no_folder() {
        let id = refused("/data/packages/Brasil Vol 2 (2019).kmpkg").id();
        assert!(
            id.chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
            "{id}"
        );
        assert!(!id.contains("data"), "{id}");
        assert!(!id.contains("packages"), "{id}");
        assert!(id.starts_with("brasil-vol-2-2019-kmpkg-"), "{id}");
    }

    /// A Windows path read by a build that is not Windows still yields only the file name.
    ///
    /// `Path::file_name` hands such a path back **whole**, which would put the folder in the id —
    /// the leak `file_name_of` exists to prevent, now load-bearing for a URL as well.
    #[test]
    fn a_windows_path_contributes_only_its_file_name_to_the_id() {
        let id = refused("D:\\tunes\\karaoke\\vol2.kmpkg").id();
        assert!(id.starts_with("vol2-kmpkg-"), "{id}");
        assert!(!id.contains("tunes"), "{id}");
    }

    /// A file name that slugs away to nothing is still addressable.
    ///
    /// The files that are strangest are exactly the ones somebody most needs the control for, so a
    /// row with no id — and therefore no way to delete it — would be the wrong answer.
    #[test]
    fn a_file_name_that_slugs_to_nothing_still_gets_an_id() {
        let id = refused("/data/packages/....").id();
        assert!(!id.is_empty());
        assert_eq!(id.len(), 8, "the fingerprint alone: {id}");
    }

    /// A machine that is playing normally has nothing to complain about.
    #[test]
    fn a_working_machine_has_no_sound_complaint() {
        let status = SoundFontStatus {
            playing: SoundKind::SoundFont,
            ..Default::default()
        };
        assert_eq!(status.complaint(), None);
    }

    /// A stale setting is a caveat about a machine that works, not a report that it is broken.
    #[test]
    fn a_stale_bank_setting_is_reported_without_being_called_a_failure() {
        let status = SoundFontStatus {
            playing: SoundKind::SoundFont,
            fallback: Some("the bank you chose is gone".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            status.complaint().as_deref(),
            Some("the bank you chose is gone")
        );
    }

    /// Silence and a test tone are different degrees, and the wording says which.
    ///
    /// `problem` alone cannot distinguish *instruments will sound wrong* from *nothing comes out*,
    /// which is the whole reason this reading lives in one place rather than in each surface.
    #[test]
    fn no_bank_and_no_device_are_told_apart_in_words() {
        let tone = SoundFontStatus {
            playing: SoundKind::TestTone,
            problem: Some("the bank would not load".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            tone.complaint().as_deref(),
            Some("no SoundFont, so instruments will sound wrong: the bank would not load")
        );

        let silent = SoundFontStatus {
            playing: SoundKind::Silent,
            ..Default::default()
        };
        assert_eq!(
            silent.complaint().as_deref(),
            Some("no sound: no audio device")
        );
    }

    #[test]
    fn default_settings_are_neutral() {
        let settings = Settings::default();
        assert_eq!(settings.transpose, 0);
        assert_eq!(settings.tempo_ratio, 1.0);
        // The guide melody is off by default: a karaoke machine's job is the backing track, and a
        // melody nobody asked for competes with the singer.
        assert!(!settings.melody_enabled);
        assert_eq!(settings.music_volume, 1.0);
    }

    #[test]
    fn a_default_snapshot_is_an_idle_machine() {
        let snapshot = Snapshot::default();
        assert_eq!(snapshot.transport, Transport::Idle);
        assert!(snapshot.now_playing.is_none());
        assert_eq!(snapshot.queue_len, 0);
    }

    #[test]
    fn an_empty_settings_patch_is_recognized_as_empty() {
        assert!(SettingsPatch::default().is_empty());
        assert!(
            !SettingsPatch {
                transpose: Some(0),
                ..Default::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn origin_distinguishes_a_catalog_song_from_a_debug_file() {
        let queued = Origin::Catalog {
            number: SongCode::new(1234),
            entry_id: 7,
        };
        let debug = Origin::File {
            path: "fixtures/sample.kar".to_owned(),
        };
        assert_ne!(queued, debug);
    }
}
