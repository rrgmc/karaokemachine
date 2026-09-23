//! The real machine: `km-catalog` and `km-kmpkg` under [`Catalog`], `km-audio` under
//! [`Controller`].
//!
//! M6 deliberately left these as two traits so the API could be tested with no hardware. This is the
//! other side of that seam, and it is where the awkward realities live: a queued song whose package
//! was uninstalled while it waited, a settings file that survives a restart, a song that ends on its
//! own with nobody having asked for anything.
//!
//! **Locking.** Three mutexes over data — settings, library, playback state — and one rule: never
//! hold more than one at a time. Every operation that needs both reads from one, drops it, then
//! takes the other. Loading a song reads the catalog and the archive *before* touching playback
//! state, which also keeps file I/O and MIDI parsing off the lock the display polls sixty times a
//! second.
//!
//! **And a fourth over an operation, which is a different kind of thing.** `advancing` is held for
//! the whole of [`Machine::advance`] — pop, parse, start — precisely because the rule above means
//! the state lock is *released* across the slow middle. Without it two threads each read "nothing
//! is playing", each popped a song, and the second overwrote the first: measured at eight singers
//! queueing at once leaving one song, with nothing in the log. It is always taken first and the
//! three above are taken inside it, so it does not make this a four-lock ordering problem.

//!
//! **Nothing here is real-time.** The audio callback is behind [`crate::engine`], reached only by
//! sending a command; a mutex on this side cannot stall it.

use anyhow::Context as _;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use km_api::events::{EndReason, Event, Events};
use km_api::machine::{
    AudioOutput, AudioOutputs, Catalog, CatalogError, ControlError, Controller, NowPlaying, Origin,
    PackageProblem, Refusal, RescanReport, Settings as ApiSettings, SettingsPatch, Snapshot,
    SoundFontChoice, SoundFontStatus, SoundKind, TransportCommand, Upload, WallpaperState,
};
use km_audio::TrackPlayer;
use km_audio::audio::Command;
use km_catalog::search::SearchQuery;
use km_catalog::{CatalogSong, InstallReport, InstalledPackage, Library, SongKind};
use km_display::lyrics::MAX_LYRIC_OFFSET_MS;
use km_kmpkg::Package;
use km_queue::Transport;
use km_queue::mics::{MicBus, MicChannel, MicPatch, MicRegistry};
use km_queue::queue::{Queue, QueueEntry, QueueFull, QueueRequest};
use km_song::{ParseOptions, Song};
use km_songcode::SongCode;

use crate::audiofocus;
use crate::cdg::CdgSong;
use crate::engine::{Engine, Sound};
use crate::settings::{Paths, Settings, tidy};
use crate::timed::TimedSong;
use crate::video::VideoSong;

/// What the machine does about SoundFonts — a second `impl Machine`, in a file of its own.
///
/// See its own header for why it is a module rather than a `SoundFonts` type. Nothing is re-exported
/// from here: the methods land on [`Machine`] exactly as though they were written below.
mod soundfont;

/// The two kinds of thing a loaded song can be.
///
/// Held on the control thread for as long as the song is loaded. The audio half of each lives
/// elsewhere — inside the player the callback owns — and is retired through `km-audio`'s
/// retirement queue; this is the half the display reads.
#[derive(Debug, Clone)]
enum Media {
    /// A parsed MIDI song, kept so the display can render its lyric timeline and so `lyric_line`
    /// events can name the words.
    Midi(Arc<Song>),
    /// A running video decoder. Dropping the last reference stops it.
    ///
    /// Unreadable in a build without the `video` feature, and unreachable too: `VideoSong::open`
    /// always fails there, so nothing ever constructs this. The variant still exists so that the
    /// loading path is one path rather than two.
    #[cfg_attr(
        not(feature = "video"),
        expect(
            dead_code,
            reason = "only the display reads this, and the display's video path is compiled out"
        )
    )]
    Video(Arc<VideoSong>),
    /// A running MP3+G song: the MP3 decoder and the CD+G picture source.
    ///
    /// **No `cfg` on this one**, unlike the variant above: `km-cdg` is pure Rust, so every build
    /// that can list one of these can play it.
    Cdg(Arc<CdgSong>),
    /// A running UltraStar or LRC song: the MP3 decoder, and the words the display draws.
    Timed(Arc<TimedSong>),
}

/// What is loaded, and enough about it to describe it without going back to the catalog.
#[derive(Debug, Clone)]
struct Loaded {
    origin: Origin,
    title: String,
    artist: Option<String>,
    /// What it is sung in, from the catalog row this was loaded from.
    ///
    /// `None` for the two debug play paths: a file handed to the machine directly has no catalog
    /// row, and the only value in the file itself is the raw `@L` header.
    language: Option<String>,
    singer: Option<String>,
    /// Whether this is a MIDI song or a video song.
    ///
    /// Carried on the loaded song rather than looked up again, because everything that reads it —
    /// the API snapshot, the controls, the display — is on a path that must not touch the
    /// catalog.
    kind: SongKind,
    duration_ms: u32,
    melody_channel: Option<u8>,
    /// Whether this song plays with none of its words drawn.
    ///
    /// Carried here for the reason [`Self::kind`] is: everything that reads it is on a path that
    /// must not touch the catalog. A song played from a package takes what the package says; a
    /// file played through the debug endpoints takes what the curator sending it said, and
    /// measures the file where they said nothing.
    lyrics_hidden: bool,
    /// Corrections for defects in this song's own events, resolved from the list it was stored with.
    ///
    /// Carried here for the reason [`Self::melody_channel`] is, and for one more: switching the bank
    /// while a song plays rebuilds its load command out of this struct, and a fix left behind there
    /// would come undone in the middle of the song that needed it.
    fixes: km_fixes::ChannelFixes,
    /// How loud this song's audio was measured to be, in LUFS, when its package measured it.
    ///
    /// Carried here for the reason [`Self::kind`] is: `start` turns it into a gain, and `start` is
    /// on a path that must not touch the catalog. `None` for every MIDI song, for a package built
    /// before levelling existed, and for the two debug play paths, which have no catalog row — and
    /// `None` means gain 1.0, which is how every song played before.
    loudness_lufs: Option<f32>,
    /// The factor the audio thread is applying to this song, and which derivation produced it.
    ///
    /// **Recorded here rather than worked out again**, and that is what makes it truthful: `start`
    /// sends one `SetSongGain` and the engine replays that value into every stream it rebuilds, so
    /// a bank switch mid-song leaves it in force. Anything deriving a gain afresh against the bank
    /// sounding now would report a number the sound is not being given.
    ///
    /// `1.0` and [`km_display::GainSource::Unmeasured`] for a song nothing levelled, which is a
    /// description of what happened rather than a missing value.
    gain: (f32, km_display::GainSource),
    /// The song itself.
    media: Media,
}

impl Loaded {
    /// What [`Loaded::gain`] holds until [`Machine::start`] works the real one out.
    ///
    /// **A description and not a sentinel**: a song nothing has levelled is a song at unity gain
    /// from nothing measured, so the placeholder is true of it at every moment it is in force. The
    /// alternative — a fifth state meaning "not decided yet" — would be a state the panel could
    /// draw, and there is no second in which that would be the honest thing to say.
    const UNLEVELLED: (f32, km_display::GainSource) = (1.0, km_display::GainSource::Unmeasured);

    fn describe(&self) -> NowPlaying {
        NowPlaying {
            origin: self.origin.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            language: self.language.clone(),
            singer: self.singer.clone(),
            kind: self.kind,
            duration_ms: self.duration_ms,
            melody_channel: self.melody_channel,
            // A video certainly has words, but they are pixels in somebody else's picture. An
            // MP3+G song's words this application draws itself — and still has no timeline, because
            // CD+G words are one-bit tiles with no character data behind them. Neither has anything
            // to highlight or to search.
            has_lyrics: self
                .lyric_song()
                .is_some_and(|song| !song.lyrics.is_empty()),
            lyrics_hidden: self.lyrics_hidden,
        }
    }

    /// The parsed MIDI song, if this is one.
    ///
    /// **MIDI only, and an UltraStar or LRC song is not one**: switching the bank reloads whatever this
    /// returns into the synthesizer. The words of either kind are [`Self::lyric_song`].
    fn song(&self) -> Option<&Arc<Song>> {
        match &self.media {
            Media::Midi(song) => Some(song),
            Media::Video(_) | Media::Cdg(_) | Media::Timed(_) => None,
        }
    }

    /// The song whose lyric timeline the machine draws: a MIDI song, or an UltraStar or LRC song's
    /// words.
    ///
    /// **`None` for a song whose words are turned off, and this is the only place that says so.**
    /// The television's rows, the streamed screen's rows, `announce_lyric_line` and
    /// [`NowPlaying::has_lyrics`] all read through here, so one answer serves four surfaces and
    /// none of them can drift from the others.
    fn lyric_song(&self) -> Option<&Arc<Song>> {
        if self.lyrics_hidden {
            return None;
        }
        match &self.media {
            Media::Midi(song) => Some(song),
            Media::Timed(song) => Some(song.song()),
            Media::Video(_) | Media::Cdg(_) => None,
        }
    }
}

/// A song loaded from the catalog and ready to hand to the engine.
///
/// Separate from [`Media`] because the engine's half can only be sent once: a [`TrackPlayer`] is
/// moved into the audio thread, whereas the [`VideoSong`] that keeps its decoder alive stays here.
enum LoadedMedia {
    /// A parsed MIDI song.
    Midi(Arc<Song>),
    /// A video song: the decoder to keep, and the audio track to send.
    Video {
        /// Kept on the control thread; dropping it stops decoding.
        song: VideoSong,
        /// Sent to the audio thread.
        track: TrackPlayer,
    },
    /// An MP3+G song, split the same way and for the same reason.
    Cdg {
        /// Kept on the control thread; dropping it stops decoding.
        song: CdgSong,
        /// Sent to the audio thread.
        track: TrackPlayer,
    },
    /// An UltraStar or LRC song, split the same way and for the same reason.
    Timed {
        /// Kept on the control thread; dropping it stops decoding.
        song: TimedSong,
        /// Sent to the audio thread.
        track: TrackPlayer,
    },
}

/// Splits a freshly loaded song into the half the control thread keeps and the half the audio
/// thread takes.
///
/// The two halves of a video song are genuinely different objects with different lifetimes: the
/// decoder must be dropped where blocking is allowed, and the track must be dropped where it is not.
/// The corrections in force on a song about to be played, and a log line for each.
///
/// **Applied whole rather than filtered.** A stored list has already been through detection and
/// through whoever curated the package, so a fix in it is one somebody agreed to; filtering here
/// would silently drop the channel mute a curator set and re-decide a question already answered.
fn fixes_for(stored: &[km_fixes::Fix], describe_as: &str) -> km_fixes::ChannelFixes {
    for line in km_fixes::describe(stored) {
        tracing::debug!(song = describe_as, "{line}");
    }
    km_fixes::resolve(stored)
}

fn split_media(
    media: LoadedMedia,
    melody_channel: Option<u8>,
    fixes: km_fixes::ChannelFixes,
) -> (Media, km_audio::audio::Load) {
    match media {
        LoadedMedia::Midi(song) => (
            Media::Midi(Arc::clone(&song)),
            km_audio::audio::Load::Midi {
                song,
                melody_channel,
                fixes,
            },
        ),
        LoadedMedia::Video { song, track } => (
            Media::Video(Arc::new(song)),
            km_audio::audio::Load::Track(Box::new(track)),
        ),
        // The same `Load::Track` the video path uses, which is why `km-audio` needed no change at
        // all for a third song kind: `Program::Track` already means "decoded audio from elsewhere".
        LoadedMedia::Cdg { song, track } => (
            Media::Cdg(Arc::new(song)),
            km_audio::audio::Load::Track(Box::new(track)),
        ),
        LoadedMedia::Timed { song, track } => (
            Media::Timed(Arc::new(song)),
            km_audio::audio::Load::Track(Box::new(track)),
        ),
    }
}

/// What a best-effort catalog lookup for the keypad's live preview found.
///
/// Three outcomes rather than an `Option`, because two of them would collapse into `None` and must
/// not be treated alike: **`Missing` is an answer** and stops the number being asked about again,
/// while **`Busy` is not** and leaves the caller to retry on the next key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SongPreviewLookup {
    /// The catalog could not be read this instant — an install is holding it, or it errored.
    /// Nothing was learned, so nothing should be recorded.
    Busy,
    /// Looked up, and no song has that number. Draws nothing; see `km_display::numbers`.
    Missing,
    /// What the catalog says the song is called.
    Found {
        /// The song's title.
        title: String,
        /// The performer, when there is one.
        artist: Option<String>,
    },
}

/// What a best-effort read of the catalog's size found.
///
/// Three outcomes for the reason [`SongPreviewLookup`] has three: **`Unchanged` is an answer** —
/// the counts on screen are still right and nothing needs redrawing — while **`Busy` is not**, and
/// leaves the caller showing whatever it last knew until the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogCounts {
    /// The catalog could not be read this instant. Nothing was learned.
    Busy,
    /// Read, and it has not moved since the version the caller passed in. Nothing was counted.
    Unchanged,
    /// Counted, at this version.
    Counted {
        /// The catalog version these counts were taken at.
        version: u64,
        /// Songs across every installed package.
        songs: usize,
        /// Packages installed.
        packages: usize,
    },
}

/// What the API says when a performance control is asked for on a song that has no such thing.
///
/// Three separate messages rather than one, because "this song has no key" and "this song has no
/// guide melody" are different facts and a remote may want to show either. All three arrive as a 409
/// with the code `unavailable`: the request is perfectly well formed, the machine just is not in a
/// state where it means anything.
///
/// **Built from the kind rather than fixed**, since a second kind lacks all three for its own
/// reasons: a video's words are pixels in somebody else's picture, an MP3+G song's are tiles this
/// application draws itself. Neither has a channel to mute or a key to shift, and a message naming
/// the wrong one would send somebody looking for a video song they never queued.
/// The stable names for "this kind of song has no key to change".
///
/// **One code per kind rather than one code and a kind beside it.** A surface rendering this has to
/// say `uma música em vídeo` where English says `a video song` — an article that agrees with the
/// noun, which `article_name()` bakes into English and no translation of a finished sentence can
/// undo. The kind therefore has to reach the page, and the code is the only thing on this wire that
/// is stable enough to carry it: adding a song-shaped field to a refusal envelope would put the
/// wrong noun in it, and reading the kind back out of the English sentence would be a parser for
/// prose that is documented as unstable.
fn no_key_code(kind: SongKind) -> &'static str {
    match kind {
        SongKind::Midi => "no_key_midi",
        SongKind::Video => "no_key_video",
        SongKind::Cdg => "no_key_cdg",
        SongKind::UltraStar => "no_key_ultrastar",
        SongKind::Lrc => "no_key_lrc",
        SongKind::Unknown => "no_key",
    }
}

/// The stable names for "this kind of song has no tempo to change". See [`no_key_code`].
fn no_tempo_code(kind: SongKind) -> &'static str {
    match kind {
        SongKind::Midi => "no_tempo_midi",
        SongKind::Video => "no_tempo_video",
        SongKind::Cdg => "no_tempo_cdg",
        SongKind::UltraStar => "no_tempo_ultrastar",
        SongKind::Lrc => "no_tempo_lrc",
        SongKind::Unknown => "no_tempo",
    }
}

/// The stable names for "this kind of song has no guide melody". See [`no_key_code`].
fn no_melody_code(kind: SongKind) -> &'static str {
    match kind {
        SongKind::Midi => "no_melody_midi",
        SongKind::Video => "no_melody_video",
        SongKind::Cdg => "no_melody_cdg",
        SongKind::UltraStar => "no_melody_ultrastar",
        SongKind::Lrc => "no_melody_lrc",
        SongKind::Unknown => "no_melody",
    }
}
/// The stable name for a transport command sent with nothing playing.
///
/// **Three of these rather than one**, and they are not the same sentence: pausing needs something
/// *playing*, seeking needs something *loaded* — which a paused song still is — and pressing play on
/// an empty machine needs something in the queue. A singer meets all three by pressing a button on a
/// remote a moment after a song ended.
pub const NOTHING_PLAYING: &str = "nothing_playing";
/// The stable name for a transport command sent with no song loaded.
pub const NOTHING_LOADED: &str = "nothing_loaded";
/// The stable name for pressing play with an empty machine and an empty queue.
pub const NOTHING_QUEUED: &str = "nothing_queued";
/// The stable name for a machine that cannot make sound at all.
///
/// A singer meets this by queueing a song on a machine whose bank never loaded, so it earns a code
/// even though what it carries is a diagnostic: the sentence beside it names which bank and how far
/// it got, and a remote showing the code's own words loses none of that — the detail is on the
/// television, beside the machine somebody has to go and fix.
pub const NO_SOUND: &str = "no_sound";
/// The stable name for a MIDI song whose melody channel could not be identified.
///
/// **Different from [`NO_MELODY_CHANNEL`]**: that one is a video or MP3+G song, which never had a melody
/// channel to find; this is a MIDI song where detection abstained, and muting a channel nobody is
/// sure of would take out an arbitrary instrument.
pub const NO_MELODY_CHANNEL: &str = "no_melody_channel";

fn has_no_key(kind: SongKind) -> Refusal {
    Refusal::coded(
        no_key_code(kind),
        format!("{} has no key to change", kind.article_name()),
    )
}

fn has_no_tempo(kind: SongKind) -> Refusal {
    Refusal::coded(
        no_tempo_code(kind),
        format!("{} has no tempo to change", kind.article_name()),
    )
}

fn has_no_melody(kind: SongKind) -> Refusal {
    Refusal::coded(
        no_melody_code(kind),
        format!("{} has no guide melody", kind.article_name()),
    )
}

/// What the API says when the output device is asked to change mid-performance.
///
/// The same 409 shape, and for the same reason: the request is well formed and the machine is not in
/// a state where it means anything. Changing the device drops the audio stream, and the player --
/// with the loaded song inside it -- lives there.
const OUTPUT_DEVICE_BUSY: &str =
    "the output device can only be changed when nothing is playing or queued";

/// What the switcher's slot 1 is called on screen.
///
/// Not the filename. Slot 1 is whatever the machine resolved for itself, which is `gm.sf2` on an
/// installed build and one of three names in a checkout — so the filename would be both uninformative
/// and inconsistent, where "bundled" is the thing the slot actually means.
const BUNDLED_BANK_NAME: &str = "bundled";

/// How many demo picks to remember, so consecutive songs differ.
///
/// Twenty is enough to make a repeat within one evening unlikely and small enough that a modest
/// catalog still has somewhere to go. It is not a guarantee: a catalog smaller than this falls
/// back to playing something recent rather than refusing to play at all.
const DEMO_RECENT: usize = 20;

/// How many songs one demo draw asks for.
///
/// `ORDER BY RANDOM()` is a full scan, so the scan is the whole cost and the extra rows are free.
/// Asking for a handful means a draw that lands on something recently played has somewhere to go
/// without a second trip to SQLite.
const DEMO_CANDIDATES: usize = 8;

/// How long to wait before looking again at a staged audition that would not delete.
///
/// The engine is not what holds the file — `VideoSong` and `CdgSong` own the reader on this side,
/// and dropping one joins the decoder thread — so the only holder that outlives the displacement is
/// the display thread's per-frame `Arc` clone, which lasts one drawn frame. A quarter second is
/// `km-audio`'s own housekeeping interval and comfortably longer than that, and it is five poll
/// ticks, so a folder that is genuinely stuck costs one `read_dir` per five ticks rather than one
/// per tick.
const AUDITION_RETRY: Duration = Duration::from_millis(250);

/// How many times to look again before leaving a staged audition to the backstops.
///
/// Two seconds. Everything in this process that can legitimately hold the handle is gone in well
/// under one; past that the holder is an antivirus scan, an open explorer window or an `adb pull`,
/// and retrying for ever would mean a `read_dir` every quarter second for the rest of the evening on
/// a folder that is never going to go. The sweep on the way in and the purge at the next start are
/// what catch it instead.
const AUDITION_TRIES: u8 = 8;

/// A staged audition that would not delete, and when to try it again.
#[derive(Debug, Clone, Copy)]
struct AuditionSweep {
    due: Instant,
    tries: u8,
}

/// Everything about playback that is not inside the engine.
#[derive(Debug)]
struct State {
    queue: Queue,
    loaded: Option<Loaded>,
    settings: ApiSettings,
    mics: MicRegistry,
    wallpapers: WallpaperState,
    /// The engine's song-ended counter as last observed, so [`Machine::poll`] can tell a song ending
    /// from having already handled it.
    songs_ended_seen: u32,
    /// The lyric line last announced, so `lyric_line` fires on change rather than every poll.
    announced_line: Option<usize>,
    /// Set by the API, cleared by the display: "advance the wallpaper now".
    wallpaper_requested: bool,
    /// Set when a wallpaper arrives, cleared by the display: "work out which folder to watch again".
    ///
    /// **A second flag rather than a stronger version of the one above, and the difference is the
    /// whole bug.** `Paths::wallpaper_dir` picks between the owner's folder, an overlay and the
    /// shipped set **by contents**, once, at startup — so on every machine whose `wallpapers/` was
    /// empty when it booted, which is every machine before its first upload, the display is watching
    /// the *shipped* folder. Advancing the playlist there would rescan a directory the new picture
    /// is not in, for ever, and nothing in any log would say so.
    wallpaper_dir_stale: bool,
    /// Whether demo mode is on **for this run**.
    ///
    /// Separate from `settings.demo.enabled`, which is what a restart would find. `PUT /demo` moves
    /// this one always and that one only when asked to persist, which is the difference between a
    /// switch that lasts the evening and one that lasts.
    demo_enabled: bool,
    /// The earliest moment a demo song may start.
    ///
    /// The whole timing policy is this one deadline plus the rules that move it — see
    /// [`demo_resume_after`]. It is consulted only while nothing is loaded and nothing is queued, so
    /// a stale value costs nothing.
    demo_resume_at: Instant,
    /// Somebody asked for one demo song by hand: start one on the next poll, whatever the mode says.
    ///
    /// **Separate from `demo_enabled` because it is a one-shot rather than a mode**, and separate
    /// from `demo_resume_at` because it does not wait — a press is not a deadline arriving early, it
    /// is a person saying now. Setting the deadline instead would look equivalent and is not: on a
    /// machine where the mode is on and somebody skipped ten seconds ago, a trigger that found
    /// nothing to play would also have canceled the silence they had just bought.
    ///
    /// Cleared by `arm_demo(DemoEvent::Somebody)`, which is both the attempt that spends it and the
    /// deliberate act that cancels it — see [`demo_once_after`].
    demo_once: bool,
    /// The last few demo picks, so the machine does not play the same song twice running.
    ///
    /// A short ring rather than a full history: over an evening the point is only that consecutive
    /// draws differ, and remembering every song a demo ever played would eventually leave nothing to
    /// pick from in a small catalog.
    demo_recent: VecDeque<SongCode>,
    /// The folder handed to an upload that has not started playing yet.
    ///
    /// **The one thing about an audition that cannot be re-derived**, which is why it is remembered
    /// where [`Controller::play_audition`] deliberately re-reads everything else. A curator sending a
    /// second video while the first is still playing spends minutes on the wire; when the first song
    /// ends, "keep only what is playing" would take the folder being written into and the upload
    /// would fail or play truncated. No later request can tell that folder from an abandoned one.
    ///
    /// Two concurrent uploads still leave the older one's staging unprotected, and that is accepted:
    /// this is a single-curator route, and the first upload would lose the race to play anyway.
    audition_staging: Option<PathBuf>,
    /// When to look again at a staged audition that would not delete, and how many tries are left.
    ///
    /// `None` — the ordinary state, and the state of every machine that never auditions — is what
    /// keeps [`Machine::settle_auditions`] to a lock and a comparison at 20 Hz.
    audition_sweep: Option<AuditionSweep>,
}

/// The karaoke machine.
pub struct Machine {
    paths: Paths,
    settings: Mutex<Settings>,
    library: Mutex<Library>,
    /// Open archives, by package id. A package's manifest is parsed once rather than on every song.
    packages: Mutex<HashMap<String, Arc<Package>>>,
    engine: Engine,
    events: Events,
    state: Mutex<State>,
    /// Serializes the whole load-next-song transaction.
    ///
    /// **`state` cannot do this job, because of a deliberate property of [`Machine::advance`].**
    /// Advancing pops under the state lock, then spends tens of milliseconds to seconds parsing an
    /// archive with that lock *released* — which is right, because holding it would stall the
    /// display and every search for the length of a video open — and then takes it again to write
    /// `loaded`. So two threads can each pop before either writes, and the second `start` overwrites
    /// the first: one song plays, the other is gone, and nothing logs it.
    ///
    /// Three threads reach it — the 20 Hz poll when a song ends, and any number of API threads
    /// through `queue_add` and `transport` — so this is a party's opening minute, not a corner case.
    /// Measured before this lock existed: eight singers queueing at once left **one** song.
    ///
    /// **It guards an operation rather than data**, which is why it coexists with the module
    /// header's "never hold more than one lock at a time" rule instead of breaking it: it is always
    /// taken first and `state`, `settings` and `library` are taken inside it, never the reverse.
    /// Nothing reachable from `advance` takes it again, so it needs no re-entrancy.
    advancing: Mutex<()>,
    /// Packages found and refused, so something other than the log can say so.
    ///
    /// In memory only, and deliberately: it describes what *this* start found, and a package fixed
    /// between two runs should leave no trace. There is no companion list of packages the owner
    /// chose to ignore, either: an uninstall deletes the file, so there is nothing left for a
    /// rescan to put back.
    problems: Mutex<Vec<PackageProblem>>,
    /// Which SoundFont slot the switcher is on, when it is on at all.
    ///
    /// In memory only, like `problems` and for a stronger version of the same reason: the switcher
    /// is run-only by decision, so nothing it does may outlive the process. An evening of
    /// comparing banks must not leave the machine playing the last one somebody pressed.
    soundfont_slot: Mutex<SoundFontSlot>,
    /// Fetches a bank when somebody asks for one, and says how it is going.
    ///
    /// **Reached from exactly two places, and both are somebody asking.** `POST
    /// /audio/soundfont/fetch`, and [`Machine::start_first_run_soundfont`] carrying forward a tick
    /// box from a setup program. Nothing on a timer and nothing in the course of playing a song
    /// calls into it, which is what keeps `Nothing downloads`'s surviving half true: a machine
    /// whose owner asked for nothing is a machine that never reaches this field.
    downloader: crate::fetch::Downloader,
    /// A setup program's tick box, while it is being carried out.
    ///
    /// In memory only, like `problems` and `soundfont_slot`: the durable half is the request file
    /// [`crate::firstrun`] owns, and this is only what *this* start is doing about it.
    first_run: Mutex<FirstRun>,
    /// Whether the machine is on the screen.
    ///
    /// **True everywhere but Android**, where SDL is the only thing that knows: a desktop window
    /// that is behind another window is still a machine somebody is standing at, and no platform
    /// but a mobile one takes the screen away outright. Nothing ever sets this false on Windows,
    /// macOS or Linux, because SDL sends the events that do only on mobile.
    ///
    /// **An atomic rather than a field of [`State`], and that is the whole design.** It is written
    /// from SDL's event watch, which runs on Android's UI thread — the thread that must never block,
    /// because five seconds on it is an ANR. A store cannot block; `lock_state()` could.
    foreground: AtomicBool,
    /// The machine has just left the screen and the music has not been stopped yet.
    ///
    /// A one-shot handed from the event watch to [`Machine::poll`], because stopping the music takes
    /// the locks the watch is not allowed to wait on. `poll` runs at [`crate::POLL_INTERVAL`], so
    /// the music stops within fifty milliseconds of the app going away — far below what anybody
    /// hears as a delay, and on the thread that already does every other piece of this work.
    background_pending: AtomicBool,
    /// The machine has just come back to the screen and the server has not been told yet.
    ///
    /// [`Machine::background_pending`]'s twin, handed from the same event watch to the same poll
    /// for the same reason: what it leads to takes a lock, and the watch runs on the thread that
    /// must not wait for one.
    foreground_pending: AtomicBool,
    /// Whether the machine has asked Android for the sound and been given it.
    ///
    /// **Held only while a song is actually playing**, which is what leaves
    /// `Leaving the screen stops the music` standing: an application holding focus is exempt from
    /// Android's cached-application freezer, so a machine that kept it in the background would throw
    /// away the backstop that entry names.
    ///
    /// False on every platform but Android, where [`crate::audiofocus::request`] answers true
    /// without asking anybody, because nothing there arbitrates the sound.
    holds_focus: AtomicBool,
    /// A song the machine stopped because something else needed the sound, and owes itself.
    ///
    /// **The one thing that can make the machine play without being asked**, and it is deliberately
    /// narrow. A call takes the sound, the machine pauses, the call ends and the song goes on. The
    /// screen coming back sets nothing here, so it resumes nothing, which is the rule
    /// `Leaving the screen stops the music` settled.
    owes_resume: AtomicBool,
    /// How to ask the API server to take its port again.
    ///
    /// Empty in every program that builds a machine without an API, which is most tests and both
    /// examples. A `OnceLock` because the two are built in the order that forces it: the state is
    /// built around the machine, so the machine cannot be handed the handle it will later need.
    relisten: OnceLock<km_api::Relisten>,
    /// Whether the frame-statistics panel is on the screen.
    ///
    /// **`F12`'s state, moved out of the display loop so that a page can reach it.** The panel
    /// existed because *the screen looks choppy* is reported from a sofa while the measurement is in
    /// a log on a box under the television — and the appliance sharpens that rather than softening
    /// it: it has no keyboard, so on the machine that needs this most `F12` is not a key anybody can
    /// press. Three surfaces can now, and one flag is what keeps the key and the switches from
    /// disagreeing.
    ///
    /// **In memory only, like `soundfont_slot` and for its reason.** A diagnostic that survives a
    /// restart is a panel left on for a month. It is also the property the overlay's own decision
    /// rests on — it states nothing and changes nothing — and writing it down would make it a
    /// setting instead.
    ///
    /// An atomic rather than a field of [`State`], for `foreground`'s reason turned around: this is
    /// *read* once per frame on the thread that must not stall, and a load cannot block.
    performance_overlay: AtomicBool,
}

/// What a first-start bank request is doing now.
#[derive(Debug, Default)]
struct FirstRun {
    /// The table id of the bank being fetched, while a fetch this started is running.
    ///
    /// `None` covers both "there was no request" and "it is finished", which is what makes
    /// [`Machine::settle_first_run_soundfont`] free on every ordinary poll.
    fetching: Option<String>,
    /// The percentage last put on the television, so a 261.9 MiB download does not rewrite it
    /// twenty times a second.
    announced: Option<u8>,
    /// The next sentence for the display to draw, if there is one.
    notice: Option<crate::firstrun::Notice>,
}

/// Where the SoundFont switcher has got to.
///
/// `slot` is 1 for the bank the machine resolved for itself and 2… for `debug.soundfonts`, which is
/// the numbering the keys use rather than an index — a slot is what somebody pressed, and reading a
/// zero-based index off a label would be its own small bug.
#[derive(Debug, Clone)]
struct SoundFontSlot {
    /// The slot in force.
    slot: u8,
    /// What to call it on screen.
    name: String,
    /// A bank assigned but not yet heard, because a video or MP3+G song holds the stream.
    ///
    /// Set when the swap could not close the stream, cleared the moment one is opened around the
    /// new bank. The label says so while it is set: a bank that has been chosen and a bank that is
    /// sounding are different claims, and only one of them is what you are listening to.
    pending: bool,
}

impl std::fmt::Debug for Machine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Machine")
            .field("sound", &self.engine.sound())
            .finish_non_exhaustive()
    }
}

impl Machine {
    /// Opens the catalog and assembles the machine.
    pub fn new(
        mut paths: Paths,
        settings: Settings,
        engine: Engine,
        events: Events,
    ) -> anyhow::Result<Self> {
        // The owner's extra package folders, joined to the ones the platform decides. Done here
        // because a `Paths` exists before there is a settings file to read — it is what names it.
        paths.extra_package_dirs.clone_from(&settings.package_dirs);
        paths.create().with_context(|| {
            format!(
                "could not create {} and {}",
                paths.config_dir.display(),
                paths.data_dir.display()
            )
        })?;
        // Nothing staged for an audition outlives the run that played it. This is the only moment
        // the whole folder can go rather than all-but-the-newest — a machine that has just started
        // has nothing open, where every later sweep has to work around a video being read as it
        // plays. Best-effort: a scratch folder that will not go is not a reason not to start.
        purge_auditions(&paths.data_dir.join(crate::settings::AUDITION_SUBDIR));

        let library = Library::open(paths.library_file())
            .with_context(|| format!("could not open {}", paths.library_file().display()))?;
        let mics = settings.mic_registry();
        let playback = ApiSettings {
            transpose: settings.playback.transpose,
            tempo_ratio: settings.playback.tempo_ratio,
            melody_enabled: settings.playback.melody_enabled,
            music_volume: settings.audio.music_volume,
            // Clamped on the way in rather than validated on load, so a hand-edited settings file
            // holding a nonsense number still boots and simply behaves as if it held the limit.
            lyric_offset_ms: settings.display.lyric_offset(),
        };
        let wallpapers = WallpaperState {
            current: None,
            count: 0,
            interval_secs: settings.wallpaper.interval_secs,
            shuffle: settings.wallpaper.shuffle,
            on_song_change: settings.wallpaper.on_song_change,
            problem: None,
            // Resolved here rather than left at the default, because a page that reports the wrong
            // one is worse than a page that reports nothing: "these are the shipped pictures" is
            // precisely the sentence somebody needs after adding one and not seeing it. Through
            // `WallpaperSettings::folder` rather than `Paths::wallpaper_dir`, so that a folder named
            // by `wallpaper.dir` is reported as the setting that it is instead of as whichever rule
            // it overrode. The display reports the real answer on its first pass and re-reports it
            // whenever the choice moves.
            source: api_wallpaper_source(settings.wallpaper.folder(&paths).1),
        };
        // Built before `paths` is moved into the machine. It holds its own copy because a download
        // runs on a thread that outlives the call that started it.
        let downloader = crate::fetch::Downloader::new(paths.clone());

        // Read before `settings` is moved in, and from the same function `run` handed to
        // `Engine::start`, so the label and the bank in the audio thread describe one slot.
        let restored_slot = settings.restored_soundfont_slot().map_or_else(
            || SoundFontSlot {
                slot: 1,
                name: BUNDLED_BANK_NAME.to_owned(),
                pending: false,
            },
            |(slot, bank)| SoundFontSlot {
                slot,
                name: bank.name.clone(),
                pending: false,
            },
        );

        let demo_enabled = settings.demo.enabled;
        // A full delay from startup rather than from zero, so a machine that boots into demo mode
        // does not start singing at whoever is still plugging it in.
        let demo_resume_at =
            demo_resume_after(DemoEvent::Somebody, Instant::now(), settings.demo.delay());

        Ok(Self {
            paths,
            settings: Mutex::new(settings),
            library: Mutex::new(library),
            packages: Mutex::new(HashMap::new()),
            problems: Mutex::new(Vec::new()),
            // Slot 1 unless a slot was remembered, and the two are read from the same place
            // `Engine::start` read it from — so the label on the screen and the bank in the audio
            // thread cannot disagree about which slot this is. Where nothing was remembered:
            // whatever the engine resolved at startup *is* the bundled slot, whether or not
            // `audio.soundfont` pointed it somewhere else. The switcher's numbering describes what
            // the keys reach, not how the bank was chosen.
            soundfont_slot: Mutex::new(restored_slot),
            downloader,
            first_run: Mutex::new(FirstRun::default()),
            // On the screen until something says otherwise, which only a phone or a tablet does.
            foreground: AtomicBool::new(true),
            background_pending: AtomicBool::new(false),
            foreground_pending: AtomicBool::new(false),
            // Nothing has been asked for yet, and nothing is owed.
            holds_focus: AtomicBool::new(false),
            owes_resume: AtomicBool::new(false),
            relisten: OnceLock::new(),
            performance_overlay: AtomicBool::new(false),
            engine,
            events,
            advancing: Mutex::new(()),
            state: Mutex::new(State {
                queue: Queue::new(),
                loaded: None,
                settings: playback,
                mics,
                wallpapers,
                songs_ended_seen: 0,
                announced_line: None,
                wallpaper_requested: false,
                wallpaper_dir_stale: false,
                demo_enabled,
                demo_resume_at,
                demo_once: false,
                demo_recent: VecDeque::new(),
                audition_staging: None,
                audition_sweep: None,
            }),
        })
    }

    /// The audio output, for reporting what it turned out to be.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Where settings, the catalog and the bundled assets live.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The individual package files the owner named by hand, copied out of the settings.
    ///
    /// Cloned rather than borrowed because the settings are behind a mutex, and every caller wants
    /// it in order to decide what is *not* the machine's to delete — a question `not_mine_to_delete`
    /// and `dropped::sweep_superseded` both ask, and neither may hold the lock while it works.
    pub(crate) fn debug_packages(&self) -> Vec<PathBuf> {
        self.lock_settings().debug.packages.clone()
    }

    /// Which wallpaper folder is in force now, and which rule chose it.
    ///
    /// Asked at every rescan rather than once at startup, because the choice is made **by
    /// contents** and the contents change: an owner's folder that was empty when the machine
    /// started stops being empty the moment they put a picture in it. Delegates to the one
    /// definition of that rule rather than restating it, and takes only the settings lock — the
    /// display loop asks this every wallpaper cycle and cloning the whole `Settings` there would be
    /// a copy of every field to read one.
    pub fn wallpaper_folder(&self) -> (PathBuf, crate::settings::WallpaperSource) {
        self.lock_settings().wallpaper.folder(&self.paths)
    }

    /// The extra wallpapers `debug.wallpapers` names, layered over whichever folder won.
    pub fn debug_wallpapers(&self) -> Vec<PathBuf> {
        self.lock_settings().debug.wallpapers.clone()
    }

    /// A copy of the settings.
    pub fn settings(&self) -> Settings {
        self.lock_settings().clone()
    }

    /// What language this machine speaks, without cloning everything else it remembers.
    ///
    /// **The display asks once a frame**, which is why this exists beside [`Self::settings`] rather
    /// than through it: that one clones a `Settings`, and sixty of those a second to read one
    /// `Copy` enum would be a real cost for a field that changes about once in a machine's life.
    ///
    /// Reading it per frame is what makes the `/admin/` picker take effect without a restart.
    pub fn locale(&self) -> km_locale::Locale {
        self.lock_settings().machine.locale()
    }

    /// Installs everything the machine should be holding: the debug extras, then what the packages
    /// folders hold.
    ///
    /// **The folders are the truth.** Nothing is remembered between starts about which files were
    /// installed — a scan says what is there, and what is there is what is installed. Remembering
    /// individual paths cannot work here: an entry goes stale the moment a file is renamed or tidied
    /// away, and nothing can safely prune one, because a file that is absent looks exactly like a
    /// drive that is unplugged. A folder needs no such judgement — there is no removable media to
    /// model here, so a folder that is not there is simply not there.
    ///
    /// `debug.packages` first, then every scanned folder, which is one everywhere but Android. The
    /// order decides which file wins when two claim one bank, so it is not free to change; and it
    /// means a dead debug path fails while an identical file in a folder still installs under the
    /// same id.
    ///
    /// A package that will not open, or whose bank is taken, is logged and remembered rather than
    /// fatal — the rest of the catalog is still worth having, and
    /// [`Machine::record_package_problem`] puts it above the title until somebody fixes it.
    pub fn install_startup_packages(&self) {
        let report = self.rescan_now();
        tracing::info!(
            installed = report.installed,
            removed = report.removed.len(),
            problems = report.problems,
            "the packages folders have been read"
        );
    }

    /// Reads the folders and makes the catalog agree with them, wherever it safely can.
    ///
    /// **One code path for the startup pass and for a rescan asked for while the machine runs**, so
    /// that "a rescan does what a restart does" is true rather than merely intended. Everything the
    /// startup pass needs is here; `install_startup_packages` only logs and discards the report.
    ///
    /// The two halves are gated differently, because only one of them can take anything away:
    ///
    /// * **Installing always runs.** A package that is new takes a free bank and adds rows; it
    ///   disturbs nothing that is playing or queued, so there is nothing to wait for.
    /// * **Pruning waits for the machine to be idle** — but only when there is something to prune,
    ///   which is the uncommon case. Somebody dropping a file in reaches the empty-`doomed` path and
    ///   is never refused. See [`prunable`] for the rule and what deferring costs.
    ///
    /// At startup the distinction collapses: this runs before the API binds and before the display
    /// thread exists, so the queue is empty by construction and the gate always opens.
    pub fn rescan_now(&self) -> RescanReport {
        let mut installed: BTreeSet<String> = BTreeSet::new();
        let mut count = 0usize;
        for path in self.startup_candidates() {
            if let Some(id) = self.install_and_report(&path) {
                installed.insert(id);
                count += 1;
            }
        }
        let (removed, deferred) = self.reconcile_catalog(&installed);
        RescanReport {
            installed: count,
            removed,
            deferred,
            problems: self.lock_problems().len(),
        }
    }

    /// Removes catalog rows for packages this pass did not install.
    ///
    /// **The other half of the folders being the truth**, and the half that fixes a fault nothing
    /// else ever did: `Machine::uninstall` was the only thing that removed a package, so a `.kmpkg`
    /// taken out of a folder left its songs in the catalog for ever — counted on the idle screen,
    /// dialable, and failing at the moment somebody picked one.
    ///
    /// **Reconciled against what actually installed**, which means a file that is present but will
    /// not open loses its rows too. That is deliberate: the machine cannot serve songs out of an
    /// archive it cannot read, so rows that survived would be exactly the phantoms this exists to
    /// remove — and the fault is not silent, because `install_and_report` has already put the file's
    /// name and the reason above the title.
    ///
    /// Safe to do at all only because a folder that is not there is not an unplugged drive; see
    /// `No removable media`. And safe to do *here* without asking what is playing, because this runs
    /// before the API binds and before the display thread exists, so the queue is empty by
    /// construction. Anything that reconciles while the machine is running has to earn that
    /// separately.
    fn reconcile_catalog(&self, installed: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
        // What is in the catalog and was not installed this pass. Asked before the gate, because
        // whether there is anything to prune is what decides whether the gate matters at all.
        let doomed: Vec<String> = match self.lock_library().packages() {
            Ok(packages) => packages
                .into_iter()
                .map(|package| package.id)
                .filter(|id| !installed.contains(id))
                .collect(),
            Err(error) => {
                tracing::error!(%error, "could not read the catalog to reconcile it");
                return (Vec::new(), Vec::new());
            }
        };
        let (prune, deferred) = prunable(doomed, !self.output_change_allowed());
        for id in &deferred {
            tracing::info!(
                package = %id,
                "leaving a package in the catalog for now: it is gone from the folders, but \
                 something is playing or queued and its numbers may be in the queue"
            );
        }
        if prune.is_empty() {
            return (Vec::new(), deferred);
        }

        let keep: Vec<&str> = installed
            .iter()
            .map(String::as_str)
            .chain(deferred.iter().map(String::as_str))
            .collect();
        let dropped = {
            let mut library = self.lock_library();
            match library.retain_packages(&keep) {
                Ok(dropped) => dropped,
                Err(error) => {
                    tracing::error!(%error, "could not reconcile the catalog against the folders");
                    return (Vec::new(), deferred);
                }
            }
        };
        for id in &dropped {
            // At `warn` and one line each: this is the machine forgetting songs it was holding
            // yesterday, and the log is the only account of it.
            tracing::warn!(
                package = %id,
                "dropping a package from the catalog: it is not in any folder the machine scans"
            );
            self.lock_packages().remove(id);
        }
        (dropped, deferred)
    }

    /// The packages to install this pass, in order, each one only once.
    ///
    /// Splits the filesystem half from the deciding half: this gathers the candidates and hands them
    /// to [`startup_plan`], which is pure and therefore testable without an audio device, a
    /// catalog or a real `.kmpkg`.
    fn startup_candidates(&self) -> Vec<PathBuf> {
        let debug = self.lock_settings().debug.packages.clone();
        let scanned = self
            .paths
            .packages_dirs()
            .into_iter()
            .flat_map(|dir| crate::settings::packages_to_install(&dir));
        startup_plan(debug.into_iter().chain(scanned), package_id_of)
    }

    /// Installs one package, saying what happened either way.
    ///
    /// A failure is **remembered as well as logged**. The scan runs at every start, so a package
    /// that will not open or that collides is a standing fault, and a log line is the one place
    /// nobody standing in front of a television can read. See [`Machine::package_problems`].
    /// Returns the package's id when it went in, so the caller can reconcile against it.
    fn install_and_report(&self, path: &Path) -> Option<String> {
        match self.install(path) {
            Ok(report) => {
                tracing::info!(
                    package = %report.package_id,
                    songs = report.songs_added,
                    "installed"
                );
                // A package that failed and has since been fixed must stop being reported.
                self.forget_package_problem(path);
                Some(report.package_id)
            }
            Err(error) => {
                tracing::error!(path = %path.display(), %error, "could not install a package");
                self.record_package_problem(path, &error);
                None
            }
        }
    }

    /// Notes a package that could not be installed, replacing any earlier note for the same file.
    ///
    /// **One sentence per fault.** Two packages cannot claim one number, so the faults that reach
    /// here are a file that will not open and a bank already taken — each of which is a sentence
    /// with a remedy.
    fn record_package_problem(&self, path: &Path, error: &CatalogError) {
        let package_id = Package::open(path)
            .ok()
            .map(|package| package.manifest().package.id.clone());
        let path = path.display().to_string();
        let problem = PackageProblem {
            reason: reason_without_path(&path, &error.to_string()),
            path,
            package_id,
        };
        let mut problems = self.lock_problems();
        problems.retain(|held| held.path != problem.path);
        problems.push(problem);
    }

    /// Drops any note about a package, because it has just installed.
    ///
    /// (See [`reason_without_path`] for why the reason stored beside it does not name the file.)
    fn forget_package_problem(&self, path: &Path) {
        let path = path.display().to_string();
        self.lock_problems().retain(|held| held.path != path);
    }

    /// The song whose lyric timeline is on the screen: a MIDI song, or an UltraStar or LRC song's
    /// words.
    ///
    /// `None` while a video or an MP3+G song is playing: its words are already in its own picture.
    pub fn current_lyric_song(&self) -> Option<Arc<Song>> {
        self.lock_state()
            .loaded
            .as_ref()
            .and_then(Loaded::lyric_song)
            .map(Arc::clone)
    }

    /// The video song currently loaded, for the display to take pictures from.
    ///
    /// Only in a build that can play video: the display's upload path is compiled out otherwise,
    /// so nothing would call this.
    #[cfg(feature = "video")]
    pub fn current_video(&self) -> Option<Arc<VideoSong>> {
        match &self.lock_state().loaded.as_ref()?.media {
            Media::Video(song) => Some(Arc::clone(song)),
            Media::Midi(_) | Media::Cdg(_) | Media::Timed(_) => None,
        }
    }

    /// The loaded MP3+G song's picture source, for the display thread.
    ///
    /// **No `#[cfg]`**, unlike [`Machine::current_video`]: this one exists in every build.
    pub fn current_cdg(&self) -> Option<Arc<CdgSong>> {
        match &self.lock_state().loaded.as_ref()?.media {
            Media::Cdg(song) => Some(Arc::clone(song)),
            Media::Midi(_) | Media::Video(_) | Media::Timed(_) => None,
        }
    }

    /// What was done to the loaded song, for the diagnostic panel.
    ///
    /// **The half of that panel the display thread cannot reach on its own.** The rest of it comes
    /// from the `Arc<Song>` that thread already holds through [`Machine::current_song`]; these three
    /// live on a private struct behind the state lock, and nothing else has a reason to ask for
    /// them.
    ///
    /// The channel counts are folded out here rather than handed over whole, because the resolved
    /// table is four arrays of sixteen and a panel row is a number. Two of those arrays are what a
    /// stored fix can set; the other two are columns nothing writes yet, so counting them would be
    /// counting fields that cannot move.
    ///
    /// Called once a frame while the panel is up and never otherwise, so a machine nobody is
    /// diagnosing takes no lock for it.
    pub fn song_levelling(&self) -> Option<(f32, km_display::GainSource, u8, u8)> {
        let state = self.lock_state();
        let loaded = state.loaded.as_ref()?;
        let count =
            |flags: &[bool; km_fixes::CHANNELS]| flags.iter().filter(|on| **on).count() as u8;
        Some((
            loaded.gain.0,
            loaded.gain.1,
            count(&loaded.fixes.ignore_bank),
            count(&loaded.fixes.mute),
        ))
    }

    /// Reports the wallpaper cycle's state. The display owns the images; the machine reports them.
    pub fn set_wallpaper_state(
        &self,
        current: Option<String>,
        count: usize,
        problem: Option<String>,
        source: crate::settings::WallpaperSource,
    ) {
        // **Passed in rather than re-read here.** The answer moves — the first picture an owner
        // adds makes their folder non-empty and the rules then choose it over the shipped set — and
        // an earlier version of this re-asked `Paths::wallpaper_dir` at report time to keep up.
        // Taking it as an argument is stronger: the display loop re-resolves the folder at its
        // rescan seam and hands over *the value it actually scanned*, so the report cannot disagree
        // with what is on screen, and a folder named by `wallpaper.dir` is reported as the setting
        // rather than as whichever rule it overrode.
        let source = api_wallpaper_source(source);
        let changed = {
            let mut state = self.lock_state();
            // **The source counts as a change too.** The folder choice is re-made at every rescan
            // now, so it can move — an owner's first picture takes it from the bundled set to their
            // own — and that is worth telling a remote about even in the case where the image on
            // screen happens to be called the same thing.
            let changed = state.wallpapers.current != current || state.wallpapers.source != source;
            state.wallpapers.current = current.clone();
            state.wallpapers.count = count;
            state.wallpapers.problem = problem;
            state.wallpapers.source = source;
            changed
        };
        if changed {
            self.events.publish(Event::WallpaperChanged { current });
        }
    }

    /// Whether the API asked for the next wallpaper since this was last called.
    ///
    /// A flag the display polls rather than a callback into SDL: only the display thread may touch a
    /// texture, and an HTTP handler is on a `tokio` worker.
    pub fn take_wallpaper_request(&self) -> bool {
        std::mem::take(&mut self.lock_state().wallpaper_requested)
    }

    /// Whether the display should work out which wallpaper folder to watch again.
    ///
    /// Polled beside [`Self::take_wallpaper_request`] and for its reason — only the display thread
    /// may touch a texture, and this is set on a `tokio` worker.
    pub fn take_wallpaper_dir_stale(&self) -> bool {
        std::mem::take(&mut self.lock_state().wallpaper_dir_stale)
    }

    /// Plays a MIDI file straight from disk, bypassing the catalog.
    ///
    /// The `--play` debug path. No root check here: somebody at the keyboard already has the disk.
    /// The HTTP route goes through [`Controller::play_file`], which does check.
    pub fn play_path(&self, path: &Path) -> Result<(), ControlError> {
        self.play_path_with(path, &km_api::Audition::default())
    }

    /// [`Self::play_path`], with the corrections somebody else has already decided on.
    ///
    /// **This is what makes a curation tool's preview worth listening to.** The corrections a
    /// curator has just ticked are not in the file and are in no package yet, so a preview that
    /// detected for itself would play the song as it was found and answer a question nobody asked.
    /// `None` is nobody having decided, and detects.
    pub fn play_path_with(
        &self,
        path: &Path,
        decided: &km_api::Audition<'_>,
    ) -> Result<(), ControlError> {
        if !self.engine.can_play() {
            return Err(ControlError::Unavailable(Refusal::coded(
                NO_SOUND,
                self.engine.sound().describe(),
            )));
        }

        // A loose video file, auditioned before anybody has decided to package it. What the decoder
        // wants is **bytes it can seek**, and a file and a package entry are two ways to supply
        // them; a packaged song hands it a window into the archive and this hands it the file. So
        // the two routes differ in how a name was arrived at rather than in what can be played.
        // Recognized by extension in every build, so one without the `video` feature says it cannot
        // play video rather than reporting a perfectly good MP4 as an unreadable MIDI file.
        if km_kmpkg::is_video_file(path) {
            return self.play_video_path(path);
        }

        // A loose UltraStar or LRC song: its MP3, with the words the sender already read out of its
        // lyrics file. Checked before the MP3+G pair, because an MP3 with words is not half of one.
        if let Some(timeline) = decided.lyrics {
            let kind = decided
                .lyrics_kind
                .filter(SongKind::carries_timeline)
                .unwrap_or(SongKind::UltraStar);
            if !km_kmpkg::is_audio_file(path) {
                return Err(ControlError::Rejected(format!(
                    "{}: {}'s words arrive only with its MP3",
                    path.display(),
                    kind.article_name()
                )));
            }
            return self.play_timed_path(path, kind, timeline, decided);
        }

        // Either half of a loose MP3+G pair, auditioned before anybody has decided to package it.
        // Give it the `.cdg` or give it the `.mp3` — the other is found beside it, tolerantly,
        // because a real folder has mixed-case extensions and at least one stem with a trailing
        // space in it. The same reasoning as the video path above: two files here, two entries in a
        // package, and the same decoder either way.
        if km_kmpkg::is_graphics_file(path) || km_kmpkg::is_audio_file(path) {
            let Some(partner) = km_kmpkg::pair_for(path) else {
                return Err(ControlError::Rejected(format!(
                    "{}: an MP3+G song is a pair, and the other half is not beside it",
                    path.display()
                )));
            };
            let (audio, graphics) = if km_kmpkg::is_audio_file(path) {
                (path.to_path_buf(), partner)
            } else {
                (partner, path.to_path_buf())
            };
            return self.play_cdg_path(&audio, &graphics);
        }

        let bytes = std::fs::read(path).map_err(|error| {
            ControlError::Rejected(format!("could not read {}: {error}", path.display()))
        })?;
        let song = Song::parse(&bytes, &ParseOptions::default())
            .map_err(|error| ControlError::Rejected(format!("{}: {error}", path.display())))?;
        warn_if_damaged(&path.display().to_string(), &song);

        // Nothing has analyzed this file, so the melody channel has to be found now — and only
        // claimed when detection is confident, exactly as at packaging time. A curator's choice wins,
        // including their saying there is none: a detector that abstained is the usual reason one
        // was made.
        let analysis = km_suitability::Analysis::of(&song);
        let melody_channel = decided
            .melody
            .unwrap_or_else(|| analysis.melody.channel().map(|melody| melody.channel));
        // The fixes are found here for the same reason, so a loose file sounds like the same song
        // played out of a package. Only the ones that apply themselves: nobody has been asked about
        // this file, and there is nowhere to record an answer.
        // What somebody decided, else what detection proposes. A decided list is applied whole:
        // filtering it would drop the channel mute that is the very thing being auditioned.
        let in_force = match decided.fixes {
            Some(fixes) => fixes.to_vec(),
            None => km_fixes::automatic(&song),
        };
        for line in km_fixes::describe(&in_force) {
            tracing::debug!(path = %path.display(), "{line}");
        }
        let fixes = km_fixes::resolve(&in_force);
        // What a curator typed, else what the file says, else the file's own name — the same chain
        // the curation tool shows a song under, so its page and the television agree. A corpus
        // file's own title is frequently the arranger's or an abbreviation, which is why somebody
        // retyped it.
        let title = decided
            .title
            .map(ToOwned::to_owned)
            .or_else(|| song.meta.title.clone())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("unknown")
                    .to_owned()
            });
        tracing::info!(
            path = %path.display(),
            %title,
            melody = ?melody_channel,
            suitability = analysis.suitability.value,
            "playing a file directly"
        );

        let song = Arc::new(song);
        let loaded = Loaded {
            origin: Origin::File {
                path: path.display().to_string(),
            },
            title,
            // This arm is reached only for MIDI: a loose video or MP3+G file was recognized by
            // extension further up and never gets here. `--play` takes one file; a song is a
            // package's business.
            kind: SongKind::Midi,
            artist: decided
                .artist
                .map(ToOwned::to_owned)
                .or_else(|| song.meta.artist.clone()),
            // The file itself says only the raw `@L` header, which is not a code and is the
            // editor.s default on most of a real corpus. A debug play is not where catalog
            // quality is decided.
            language: None,
            singer: None,
            duration_ms: song.tempo_map.tick_to_ms(song.duration_ticks),
            melody_channel,
            // What the curator said, else what this file measures — the same chain a package's
            // build follows, so a preview shows the screen the packaged song would draw.
            lyrics_hidden: decided
                .lyrics_hidden
                .unwrap_or_else(|| analysis.suitability.words_cannot_be_followed()),
            fixes,
            // A loose file has no package, so nothing measured it. See `Loaded::loudness_lufs`.
            loudness_lufs: None,
            gain: Loaded::UNLEVELLED,
            media: Media::Midi(Arc::clone(&song)),
        };
        self.start(
            loaded,
            // Where a packaged song's stored transposition goes, so the operator's own default is
            // added on top of it exactly as it would be. 0 is the file's own key, which is what a
            // song nobody has transposed plays in.
            decided.transpose.unwrap_or(0),
            km_audio::audio::Load::Midi {
                song,
                melody_channel,
                fixes,
            },
        )
    }

    /// Plays a loose video file, the video half of [`Self::play_path`].
    ///
    /// Nothing about it comes from a manifest, because there is not one: the title is the file's own
    /// name — which is the only title a downloaded video usually has, and the same thing packaging
    /// would fall back to — and the length comes from the probe.
    fn play_video_path(&self, path: &Path) -> Result<(), ControlError> {
        // Falls back to a common rate when nothing has been probed yet, exactly as the package path
        // does. Reachable only when no device exists at all, which `can_play` has already refused.
        let rate = match self.engine.sample_rate() {
            0 => 48_000,
            rate => rate,
        };
        let (song, track) = VideoSong::open(path, rate)
            .map_err(|error| ControlError::Rejected(format!("{error}")))?;

        let title = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown")
            .to_owned();
        let duration_ms = song.duration_ms();
        tracing::info!(path = %path.display(), %title, duration_ms, "playing a video file directly");

        let loaded = Loaded {
            origin: Origin::File {
                path: path.display().to_string(),
            },
            title,
            kind: SongKind::Video,
            // Nothing about a loose file says who is performing, and a guess from the name would be
            // worse than saying nothing. No container states the language of the singing either.
            artist: None,
            language: None,
            singer: None,
            duration_ms,
            // A video has no channels, so there is no melody channel to claim — the same absence the
            // API reports as `unavailable` rather than hiding.
            melody_channel: None,
            // Its words are pixels in its own picture: there is no timeline to withhold, and
            // withholding the picture would be withholding the song.
            lyrics_hidden: false,
            // Nothing here has MIDI events, so there is nothing a fix could correct.
            fixes: km_fixes::ChannelFixes::default(),
            // A loose file has no package, so nothing measured it. See `Loaded::loudness_lufs`.
            loudness_lufs: None,
            gain: Loaded::UNLEVELLED,
            media: Media::Video(Arc::new(song)),
        };
        self.start(loaded, 0, km_audio::audio::Load::Track(Box::new(track)))
    }

    /// Plays a loose MP3+G pair straight from disk. The other half of the `--play` debug path.
    fn play_cdg_path(&self, audio: &Path, graphics: &Path) -> Result<(), ControlError> {
        let rate = match self.engine.sample_rate() {
            0 => 48_000,
            rate => rate,
        };
        let (song, track) = CdgSong::open(audio, graphics, rate, None)
            .map_err(|error| ControlError::Rejected(format!("{error}")))?;

        // The stem, and only the stem. A packaged song gets a title a person chose and tags that
        // packaging filtered; a loose file auditioned by hand has neither, and reading the ID3 here
        // would mean applying that filtering in a second place. See the
        // `Where an MP3+G song's title and artist come from` decision.
        let title = audio
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown")
            .to_owned();
        let duration_ms = song.duration_ms();
        tracing::info!(
            audio = %audio.display(),
            graphics = %graphics.display(),
            %title,
            duration_ms,
            "playing an MP3+G pair directly"
        );

        let loaded = Loaded {
            origin: Origin::File {
                path: audio.display().to_string(),
            },
            title,
            kind: SongKind::Cdg,
            artist: None,
            language: None,
            singer: None,
            duration_ms,
            melody_channel: None,
            // The same as a video's: this application draws the words, but from one-bit tiles
            // rather than from a timeline, so there is no text to withhold.
            lyrics_hidden: false,
            // Nothing here has MIDI events, so there is nothing a fix could correct.
            fixes: km_fixes::ChannelFixes::default(),
            // A loose file has no package, so nothing measured it. See `Loaded::loudness_lufs`.
            loudness_lufs: None,
            gain: Loaded::UNLEVELLED,
            media: Media::Cdg(Arc::new(song)),
        };
        self.start(loaded, 0, km_audio::audio::Load::Track(Box::new(track)))
    }

    /// Plays a loose UltraStar or LRC song: an MP3 and the words the sender read out of its lyrics
    /// file.
    ///
    /// The machine never reads either file, so the words arrive as the timeline a package stores,
    /// and from here the song plays as a packaged one does.
    fn play_timed_path(
        &self,
        audio: &Path,
        kind: SongKind,
        timeline: &km_song::LyricTimeline,
        decided: &km_api::Audition<'_>,
    ) -> Result<(), ControlError> {
        let rate = match self.engine.sample_rate() {
            0 => 48_000,
            rate => rate,
        };
        let rejected = |error: &dyn std::fmt::Display| {
            ControlError::Rejected(format!("{}: {error}", audio.display()))
        };
        // Probed rather than taken from the words: the recording is the song's length, and the
        // words may end well before it.
        let duration_ms = km_cdg::probe_audio(audio)
            .map_err(|error| rejected(&error))?
            .duration_ms;
        let file = std::fs::File::open(audio).map_err(|error| rejected(&error))?;
        let name = audio.display().to_string();
        let (song, track) = TimedSong::open_from(file, &name, timeline.clone(), rate)
            .map_err(|error| rejected(&error))?;

        let title = decided.title.map_or_else(
            || {
                audio
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("unknown")
                    .to_owned()
            },
            ToOwned::to_owned,
        );
        tracing::info!(
            audio = %audio.display(),
            %title,
            duration_ms,
            kind = kind.as_str(),
            "playing a song with a lyric timeline directly"
        );

        let loaded = Loaded {
            origin: Origin::File { path: name },
            title,
            kind,
            artist: decided.artist.map(ToOwned::to_owned),
            language: None,
            singer: None,
            duration_ms,
            melody_channel: None,
            // Nothing measures this for an UltraStar or LRC song — the three faults that answer it are
            // read from MIDI events — so the curator's word is the only one, and their silence
            // draws the words.
            lyrics_hidden: decided.lyrics_hidden.unwrap_or(false),
            // Nothing here has MIDI events, so there is nothing a fix could correct.
            fixes: km_fixes::ChannelFixes::default(),
            // A loose file has no package, so nothing measured it. See `Loaded::loudness_lufs`.
            loudness_lufs: None,
            gain: Loaded::UNLEVELLED,
            media: Media::Timed(Arc::new(song)),
        };
        self.start(loaded, 0, km_audio::audio::Load::Track(Box::new(track)))
    }

    /// Says whether the machine is on the screen.
    ///
    /// **Called from SDL's event watch on a phone or a tablet, and from nowhere else.** See
    /// [`Machine::foreground`] for why this does nothing but store: the caller is the platform's UI
    /// thread, and it is the one thread in the process that must never wait for a lock.
    ///
    /// Leaving the screen is remembered as well as recorded, because the two facts are used
    /// differently — [`Machine::settle_foreground`] acts on the *edge* to stop the music once, and
    /// [`Machine::maybe_start_demo`] reads the *level* on every tick to keep demo mode from starting
    /// a song nobody could be looking at.
    pub fn set_foreground(&self, on: bool) {
        let was = self.foreground.swap(on, Ordering::AcqRel);
        if was == on {
            return;
        }
        if on {
            self.foreground_pending.store(true, Ordering::Release);
        } else {
            self.background_pending.store(true, Ordering::Release);
        }
        tracing::info!(on_screen = on, "the machine's visibility changed");
    }

    /// Says how to ask the API server to take its port again.
    ///
    /// Called once, by whoever built both this machine and the server around it. A machine nobody
    /// calls this on returns to the screen without asking for anything, which is right for every
    /// program that has no server to ask.
    pub fn set_relisten(&self, relisten: km_api::Relisten) {
        let _ = self.relisten.set(relisten);
    }

    /// Whether the frame-statistics panel is on the screen.
    ///
    /// Read once per frame by the display loop and by the two API routes. See
    /// [`Machine::performance_overlay`] — the field — for why it lives where it does.
    pub fn performance_overlay_on(&self) -> bool {
        self.performance_overlay.load(Ordering::Acquire)
    }

    /// Puts the frame-statistics panel on the screen, or takes it off.
    ///
    /// **This one takes effect on the next frame, unlike the two switches it sits beside on every
    /// page.** Debugging mode and the development console decide which *routes* get mounted, so they
    /// wait for a restart; this decides what one function draws. Worth stating because a page
    /// showing all three together has to say which of them needs a restart and which does not.
    ///
    /// Nothing is written down: see the field.
    pub fn set_performance_overlay(&self, on: bool) {
        if self.performance_overlay.swap(on, Ordering::AcqRel) == on {
            return;
        }
        tracing::info!(on, "the frame statistics panel was switched");
    }

    /// Writes down whether the window was fullscreen when it closed.
    ///
    /// **So that `F` outlives the process.** Fullscreen was a thing somebody could change at the
    /// machine and could not keep: the toggle moved the window and nothing else, so every restart
    /// went back to whatever `display.fullscreen` had always said. Somebody who takes a machine
    /// fullscreen and closes it has said what they want that machine to do.
    ///
    /// **Nothing is written when the answer has not changed**, which is what keeps an ordinary close
    /// from touching settings.json at all. That matters more than the write it saves: `save` writes
    /// the whole file, so a run that rewrote it on every exit would be a run that could lose an
    /// unrelated hand edit to a power cut at the wrong moment.
    ///
    /// The caller decides whether this run may speak for the machine — `--fullscreen` and
    /// `--windowed` are one process and write nothing. See `DisplayConfig::remember_fullscreen`.
    pub fn remember_fullscreen(&self, fullscreen: bool) {
        {
            let mut settings = self.lock_settings();
            if settings.display.fullscreen == fullscreen {
                return;
            }
            settings.display.fullscreen = fullscreen;
        }
        tracing::info!(fullscreen, "remembering how the window was left");
        self.save_settings();
    }

    /// Writes back whether the window was left in front of everything else.
    ///
    /// The same two rules [`Self::remember_fullscreen`] states, and for the same reasons: nothing
    /// is written when the answer has not changed, because `save` writes the whole file and a run
    /// that rewrote it on every exit could lose an unrelated hand edit to a power cut.
    ///
    /// Unlike fullscreen there is no flag that declares a run temporary, so there is no caller
    /// that must be asked whether this one may speak for the machine.
    pub fn remember_always_on_top(&self, always_on_top: bool) {
        {
            let mut settings = self.lock_settings();
            if settings.display.always_on_top == always_on_top {
                return;
            }
            settings.display.always_on_top = always_on_top;
        }
        tracing::info!(always_on_top, "remembering how the window was left");
        self.save_settings();
    }

    /// Writes back where the window was and how large, so the next start opens it there.
    ///
    /// The caller passes a rect only when the window closed in a window; one closed fullscreen
    /// keeps the rect it had before. Nothing is written when the rect has not changed, on
    /// [`Self::remember_fullscreen`]'s reasoning. Any run may call this: `--fullscreen` and
    /// `--windowed` override fullscreen, not where a window sits.
    pub fn remember_window_rect(&self, rect: crate::settings::WindowRect) {
        {
            let mut settings = self.lock_settings();
            if settings.display.window_rect() == rect {
                return;
            }
            settings.display.set_window_rect(rect);
        }
        tracing::info!(?rect, "remembering where the window was left");
        self.save_settings();
    }

    /// Stops the music when the machine has left the screen.
    ///
    /// **This is the fix for a machine that went on singing to an empty room.** Android stops
    /// drawing when the activity stops — SDL parks its main thread — but the audio device is cpal's,
    /// on a thread of its own that SDL has never heard of, so the song simply carried on. Only the
    /// operating system stopped it, by freezing the whole process a minute or so later, and a
    /// freezer is not a contract: an app that holds audio focus is exempt from it.
    ///
    /// **Paused rather than stopped**, so the song and its place in it survive. Coming back finds
    /// the machine where it was, waiting for somebody to press play — which is the one thing it must
    /// not do by itself, that being how a person ends up walking in on a song already running.
    ///
    /// The output device is deliberately **not** handed back here. `should_release` excludes
    /// `Paused` on purpose, and its reasoning holds: reopening would lose the position, so releasing
    /// it would cost exactly the thing pausing was for. What is left open renders silence.
    ///
    /// **Audio focus *is* handed back, and this is the pause that does it.**
    /// [`Machine::settle_audio_focus`] holds focus only while a song plays, so the pause above drops
    /// it on the next tick and the exemption named in the first paragraph never applies.
    fn settle_foreground(&self) {
        if !self.background_pending.swap(false, Ordering::AcqRel) {
            return;
        }
        // **Back already, so there is nothing to stop.** The flag records that the machine left the
        // screen at some point since the last poll; this asks whether it is still gone. A dialog
        // that takes focus and hands it back inside fifty milliseconds raises both events before
        // this runs, and pausing on the strength of the first would stop a song that is on the
        // screen in front of somebody — a fault of exactly the kind this method exists to prevent,
        // pointing the other way.
        if self.foreground.load(Ordering::Acquire) {
            return;
        }
        // Asked before anything is locked, and cheap: a machine that left the screen with nothing
        // playing is the ordinary case, and it should cost one atomic and one comparison.
        if self.engine.transport() != Transport::Playing {
            return;
        }
        match Controller::transport(self, TransportCommand::Pause) {
            Ok(()) => tracing::info!("paused: the machine is no longer on the screen"),
            // Not a fault. The song can end between the two lines above.
            Err(error) => tracing::debug!(%error, "nothing left to pause on leaving the screen"),
        }
    }

    /// Asks the server for its port back, now that the machine is on the screen again.
    ///
    /// **A machine that comes back is a machine whose listening socket may not have.** A platform
    /// that suspends an application destroys it while the application is away, and what the process
    /// gets back can be a descriptor that never accepts and never fails, which no accept loop can
    /// tell from a network nobody is using. Coming back to the screen is the one moment the machine
    /// knows something the server cannot work out for itself, so it says so and the server takes the
    /// port again.
    ///
    /// Unconditional, rather than asked only where the socket looks wrong. Proving a socket still
    /// works costs a connection to it and answers for that instant only, where replacing it is a
    /// bind, and a machine arriving on the screen has nothing in flight worth keeping.
    fn settle_relisten(&self) {
        if !self.foreground_pending.swap(false, Ordering::AcqRel) {
            return;
        }
        // Gone again already, so the socket it would take is one nothing can reach. The next return
        // raises this again, which is the one that matters.
        if !self.foreground.load(Ordering::Acquire) {
            return;
        }
        if let Some(relisten) = self.relisten.get() {
            relisten.request();
        }
    }

    /// Holds the sound while a song plays, and answers whatever the system says about it.
    ///
    /// **Two jobs that belong together, because each is the other's edge.** The machine asks for
    /// focus when a song starts and gives it back when one stops, so that another player yields to
    /// it; and it acts on what Android reports, so that it yields to a call. Splitting them would
    /// mean two places that both have to agree about whether the machine is holding anything.
    ///
    /// **Driven from the transport rather than from the call sites.** Every route into playing — a
    /// press, a queue advancing, a song ending, a seek, the screen going away — moves the transport,
    /// so watching it here catches all of them and none of them learns that focus exists.
    ///
    /// Off Android the whole method is one atomic read and one comparison per tick: nothing
    /// arbitrates the sound there, [`crate::audiofocus::take`] answers nothing, and a request that
    /// is always granted leaves the flag matching the transport after the first song.
    fn settle_audio_focus(&self) {
        // What the system said, before what the machine wants, because losing the sound changes the
        // transport and there is no point asking for what a pause is about to hand back.
        if let Some(change) = audiofocus::take() {
            let owed = self.owes_resume.load(Ordering::Acquire);
            let (command, still_owed) = audiofocus::action(change, owed);
            self.owes_resume.store(still_owed, Ordering::Release);
            if let Some(command) = command {
                match Controller::transport(self, command) {
                    Ok(()) => tracing::info!(?change, ?command, "the sound changed hands"),
                    // Not a fault, and the same reason `settle_foreground` gives: the song can end
                    // between the report and this line.
                    Err(error) => tracing::debug!(%error, ?change, "nothing to do about the sound"),
                }
            }
        }

        let transport = self.engine.transport();
        // **A debt outlives a pause and nothing else.** `Paused` still has the song loaded at its
        // position, which is the thing a call interrupted; `Idle` and `Stopped` mean it went away
        // while the call ran, and getting the sound back then must not start something nobody asked
        // for.
        if self.owes_resume.load(Ordering::Acquire)
            && !matches!(transport, Transport::Playing | Transport::Paused)
        {
            self.owes_resume.store(false, Ordering::Release);
        }

        // **The level, not the edge.** A song that is playing wants the sound, and so does one
        // waiting on a call to end: abandoning the request there would leave the system with nothing
        // to hand back to, and the song would sit paused for good. Asked every tick and acted on
        // only when the answer differs from what is held, so an idle machine costs a few atomics.
        let wants = transport == Transport::Playing || self.owes_resume.load(Ordering::Acquire);
        if wants == self.holds_focus.load(Ordering::Acquire) {
            return;
        }
        if wants {
            let granted = audiofocus::request();
            self.holds_focus.store(granted, Ordering::Release);
            if !granted {
                // Refused rather than broken. The machine plays anyway, which is what it did before
                // it ever asked, and the next tick asks again.
                tracing::info!("the sound was not given to this machine");
            }
        } else {
            audiofocus::abandon();
            self.holds_focus.store(false, Ordering::Release);
        }
    }

    /// One step of the machine's own business: advancing the queue, and announcing lyric lines.
    ///
    /// Called from a watchdog thread at [`crate::POLL_INTERVAL`]. It exists because a song ending is
    /// not something any request caused — without it a finished song would leave the machine sitting
    /// silent with a full queue.
    pub fn poll(&self) {
        // All but free on every ordinary run: one uncontended lock, and it returns the moment it
        // finds no pending swap — which is always, unless somebody pressed a switcher key during a
        // video song.
        self.settle_pending_soundfont();
        // Free in the same way and for the same reason: one uncontended lock that returns at once
        // unless this start began a download of its own, which almost no start ever does.
        self.settle_first_run_soundfont();
        // And again: a lock and a comparison unless a staged audition refused to delete, which needs
        // somebody to have auditioned at all and the file to have still been open when it did.
        self.settle_auditions();
        // One atomic on every platform but a phone or a tablet, where it is one atomic and then, on
        // exactly the tick the app went away, a pause.
        self.settle_foreground();
        // Its twin on the other edge, and one atomic in the same way: on exactly the tick the app
        // came back, a request for the port.
        self.settle_relisten();
        // A few atomics on every platform, and on a phone the two moments the sound changes hands:
        // a song starting asks for it, and a call taking it stops the song.
        self.settle_audio_focus();

        let ended = self.engine.songs_ended();
        let (was_seen, has_song, was_demo) = {
            let mut state = self.lock_state();
            let seen = state.songs_ended_seen;
            state.songs_ended_seen = ended;
            // Read while `loaded` is still here: `advance` takes it, and by then the one thing that
            // decides whether the next demo waits the full delay or none would be gone.
            let was_demo = matches!(
                state.loaded.as_ref().map(|loaded| &loaded.origin),
                Some(Origin::Demo { .. })
            );
            (seen, state.loaded.is_some(), was_demo)
        };

        if ended != was_seen && has_song {
            self.events.publish(Event::SongEnded {
                reason: EndReason::Finished,
            });
            self.arm_demo(if was_demo {
                DemoEvent::DemoEnded
            } else {
                DemoEvent::Somebody
            });
            // Straight into the next one. A pause between songs is what a party notices.
            self.advance();
        }

        self.maybe_start_demo();
        self.announce_lyric_line();
    }

    /// Moves demo mode's deadline, and settles what becomes of a hand-pressed trigger.
    ///
    /// See [`demo_resume_after`] for what each event buys, and [`demo_once_after`] for why the
    /// trigger is cleared here rather than anywhere the word "start" appears: this is called
    /// immediately before every attempt *and* by every deliberate act, which is exactly the two
    /// occasions on which a pending trigger stops being one.
    fn arm_demo(&self, what: DemoEvent) {
        let delay = self.lock_settings().demo.delay();
        let mut state = self.lock_state();
        state.demo_resume_at = demo_resume_after(what, Instant::now(), delay);
        state.demo_once = demo_once_after(what, state.demo_once);
    }

    /// Starts a demo song when one is due.
    ///
    /// Called from [`Self::poll`], so it runs twenty times a second and is nearly always a lock and
    /// a comparison. The work only happens on the tick a demo actually becomes due.
    fn maybe_start_demo(&self) {
        // Read outside the state lock, and it is only an atomic. **This is the condition that
        // matters most on Android**: the watchdog thread this runs on keeps running while the
        // activity is stopped, so without it a backgrounded machine goes on picking songs and
        // playing them to nobody — which is the fault that put the whole condition here.
        let foreground = self.foreground.load(Ordering::Acquire);
        let due = {
            let state = self.lock_state();
            demo_is_due(
                state.demo_enabled,
                state.demo_once,
                state.loaded.is_some(),
                state.queue.is_empty(),
                state.demo_resume_at,
                Instant::now(),
                foreground,
            )
        };
        // `can_play` last: it is the only one of the five that touches the engine, and on a machine
        // with no sound card it is false for ever — there is no point asking it on every tick of
        // every run where demo mode is off.
        if !due || !self.engine.can_play() {
            return;
        }

        // Re-armed *before* the attempt rather than after it. A catalog that is empty, or whose
        // every song fails to load, would otherwise be retried twenty times a second for as long as
        // the machine is switched on — and each retry is a full-table `ORDER BY RANDOM()`. Starting
        // succeeds far more often than not, and when it does the deadline is irrelevant anyway,
        // because nothing is due while a song is loaded.
        self.arm_demo(DemoEvent::Somebody);
        self.start_demo();
    }

    /// Picks a song nobody asked for and plays it.
    fn start_demo(&self) {
        let Some(row) = self.pick_demo_song() else {
            return;
        };
        let number = row.number;
        // The device is handed back after `audio.idle_release_secs`, and demo mode by definition
        // only ever starts after a silence — so this is the one start that can always assume the
        // device is asleep. Waking it before the archive is opened overlaps the two, which on a
        // Bluetooth link is about a second that would otherwise be silence after the decision.
        self.engine.send(Command::Wake);

        let loaded = match self.load_from_catalog(number) {
            Ok(Some((media, row))) => {
                let fixes = fixes_for(&row.fixes, &row.title);
                let (media, load) = split_media(media, row.melody_channel, fixes);
                let loaded = Loaded {
                    origin: Origin::Demo { number },
                    title: row.title,
                    kind: row.kind,
                    artist: row.artist,
                    language: row.language.clone(),
                    // Nobody asked for it, so nobody is down to sing it. Naming a singer here would
                    // put a name on a television beside a song they never chose.
                    singer: None,
                    duration_ms: row.duration_ms,
                    melody_channel: row.melody_channel,
                    lyrics_hidden: row.lyrics_hidden,
                    fixes,
                    loudness_lufs: row.loudness_lufs,
                    gain: Loaded::UNLEVELLED,
                    media,
                };
                Some((loaded, row.default_transpose, load))
            }
            Ok(None) => {
                tracing::warn!(%number, "the demo picked a song that is no longer in the catalog");
                None
            }
            Err(error) => {
                tracing::warn!(%number, %error, "the demo picked a song that would not load");
                None
            }
        };

        let Some((loaded, transpose, load)) = loaded else {
            return;
        };
        if self.start(loaded, transpose, load).is_ok() {
            let mut state = self.lock_state();
            state.demo_recent.push_back(number);
            while state.demo_recent.len() > DEMO_RECENT {
                state.demo_recent.pop_front();
            }
            tracing::debug!(%number, "the demo started a song");
        }
    }

    /// One song for the demo to play, or `None` when the catalog can offer nothing.
    ///
    /// Two searches at worst. The first applies `demo.min_suitability`; if that matches nothing —
    /// a catalog built entirely from rough files, or one whose packages carry no rating at all —
    /// the second drops the filter, because a silent machine is a worse answer than a mediocre song.
    fn pick_demo_song(&self) -> Option<CatalogSong> {
        let min_suitability = self.lock_settings().demo.min_suitability;
        let recent: Vec<SongCode> = self.lock_state().demo_recent.iter().copied().collect();

        // A handful rather than one, so a draw that lands on something recently played has
        // somewhere to go without a second trip to SQLite. `ORDER BY RANDOM()` is a scan, and the
        // scan is the cost — the extra rows are free.
        let draw = |floor: Option<u8>| -> Option<CatalogSong> {
            let query = SearchQuery {
                min_suitability: floor,
                sort: km_catalog::search::SortOrder::Random,
                limit: DEMO_CANDIDATES,
                ..Default::default()
            };
            let songs = match self.lock_library().search(&query) {
                Ok(songs) => songs,
                Err(error) => {
                    tracing::warn!(%error, "the demo could not search the catalog");
                    return None;
                }
            };
            // Falls back to the first candidate rather than giving up when every draw is recent,
            // which is what a catalog smaller than the ring would otherwise do: refuse to play.
            let first = songs.first().cloned();
            songs
                .into_iter()
                .find(|song| !recent.contains(&song.number))
                .or(first)
        };

        draw(min_suitability).or_else(|| {
            // Only worth a second trip when the first one was narrowed. An unfiltered draw that
            // found nothing means the catalog is empty, and asking it again would not change that.
            min_suitability?;
            tracing::debug!(
                "no song clears demo.min_suitability; the demo is drawing from the whole catalog"
            );
            draw(None)
        })
    }

    /// Emits `lyric_line` when the current line changes.
    fn announce_lyric_line(&self) {
        if !self.engine.transport().is_advancing() {
            return;
        }
        let mut state = self.lock_state();
        let Some(loaded) = state.loaded.as_ref() else {
            return;
        };
        // A video or MP3+G song has no lyric timeline to announce lines from, and a song whose
        // words are turned off has one nobody is to be shown. Nothing to say either way — see
        // `Loaded::lyric_song`, which is where the two become one answer.
        let Some(song) = loaded.lyric_song().map(Arc::clone) else {
            return;
        };
        // A MIDI song's clock is the sequencer's tick; an UltraStar or LRC song's is the audio's
        // position, which its millisecond tempo map turns into the timeline's ticks.
        let tick = if loaded.kind.is_midi() {
            self.engine.position_ticks()
        } else {
            song.tempo_map.ms_to_tick(self.engine.position_ms())
        };
        let index = song.lyrics.line_at_tick(tick);
        if index == state.announced_line {
            return;
        }
        // Built while the borrow of `loaded` is still live, then the guard is dropped before
        // publishing — publishing under the lock would let a slow subscriber stall the display.
        let event = index.and_then(|index| {
            song.lyrics.lines.get(index).map(|line| Event::LyricLine {
                index,
                start_ms: song.tempo_map.tick_to_ms(line.start_tick),
                end_ms: song.tempo_map.tick_to_ms(line.end_tick),
                text: line.text(),
            })
        });
        state.announced_line = index;
        drop(state);
        if let Some(event) = event {
            self.events.publish(event);
        }
    }

    /// Takes the next song off the queue and starts it, or goes idle — whatever is playing now.
    ///
    /// Skips entries whose song can no longer be loaded — a package uninstalled while the song sat
    /// in the queue — rather than stopping on one. Bounded by the queue length so a catalog that
    /// cannot load anything ends up idle instead of looping.
    ///
    /// **Unconditional, which is what its two callers want**: a song that has just ended, and Skip.
    /// Both are already holding the deck and are replacing what is on it. Somebody *choosing* a
    /// song wants [`Machine::advance_if_idle`] instead.
    ///
    /// Waits its turn rather than giving up, because for these two there is no next song without
    /// it. See the `advancing` field for why the turn exists at all.
    fn advance(&self) {
        let _turn = self
            .advancing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.advance_holding_the_turn();
    }

    /// Starts the next song, but only if nothing is playing.
    ///
    /// **The idle check belongs inside the turn lock, and that is the whole of this function.** A
    /// caller spelling it out as `if self.lock_state().loaded.is_none() { advance() }` takes the
    /// guard as a temporary, dropped at the end of the condition, so the pop that follows happens
    /// with nothing held: two threads read the same `None` and both take a song.
    ///
    /// **A busy turn means there is nothing to do, so this does not queue behind one.** Another
    /// thread already holds the deck and is loading the next song; waiting for it would only be a
    /// way to reach the idle check after the answer has changed. That also keeps a party's opening
    /// moment from parking eight API threads on one mutex, where [`Machine::advance`]'s callers —
    /// a song ending, and Skip — genuinely do have to wait their turn.
    fn advance_if_idle(&self) {
        let _turn = match self.advancing.try_lock() {
            Ok(turn) => turn,
            // Somebody else has the deck and is loading; there is nothing here to do.
            Err(std::sync::TryLockError::WouldBlock) => return,
            // Recovered rather than treated as busy, for the reason `lock_state` gives: a panic
            // somewhere else does not make this `()` untrustworthy, and a `try_lock` that reads a
            // poisoned lock as "busy" would leave a machine that never starts another song and
            // never says why.
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        if self.lock_state().loaded.is_some() {
            return;
        }
        self.advance_holding_the_turn();
    }

    /// Starts the next song if nothing is playing **or if what is playing is a demo song**.
    ///
    /// [`Machine::advance_if_idle`] with one more way to be free: a machine singing to itself is
    /// not occupied. Queueing is what takes the deck back off it — see
    /// `What the machine does when nobody is singing`, which argues why that is worth the
    /// interruption where taking it off a person would not be.
    ///
    /// **The demo test is inside the turn lock for the same reason the idle test is.** Read outside
    /// it, the answer is about the past: the poll thread advances on its own whenever a song ends,
    /// so a demo seen a statement ago may already have finished and been replaced by the very entry
    /// this call is about to pop. Both tests are one question — *is the deck free?* — and it has one
    /// answer only while the deck is held.
    ///
    /// Returns whether it took the deck **from a demo**, for
    /// [`Event::SongEnded`]`{ reason: `[`EndReason::Yielded`]` }`. Returned rather than published
    /// here so one caller decides an event is owed; `false` on the idle path means "nothing ended"
    /// rather than "nothing happened".
    fn advance_if_idle_or_over_a_demo(&self) -> bool {
        let _turn = match self.advancing.try_lock() {
            Ok(turn) => turn,
            Err(std::sync::TryLockError::WouldBlock) => return false,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let over_a_demo = match self
            .lock_state()
            .loaded
            .as_ref()
            .map(|loaded| &loaded.origin)
        {
            None => false,
            Some(Origin::Demo { .. }) => true,
            // Somebody is singing. This is the case the whole disclosure protects, and the one
            // where the entry just added waits its turn.
            Some(_) => return false,
        };
        self.advance_holding_the_turn();
        over_a_demo
    }

    /// The transaction itself. Call through [`Machine::advance`],
    /// [`Machine::advance_if_idle`] or [`Machine::advance_if_idle_or_over_a_demo`], all of which
    /// hold `advancing` for the whole of it.
    fn advance_holding_the_turn(&self) {
        let attempts = self.lock_state().queue.len();
        for _ in 0..=attempts {
            let Some(entry) = self.lock_state().queue.pop() else {
                break;
            };
            self.publish_queue();
            match self.load_from_catalog(entry.number) {
                Ok(Some((media, row))) => {
                    let fixes = fixes_for(&row.fixes, &row.title);
                    let (media, load) = split_media(media, row.melody_channel, fixes);
                    let loaded = Loaded {
                        origin: Origin::Catalog {
                            number: entry.number,
                            entry_id: entry.id,
                        },
                        title: entry.title,
                        kind: row.kind,
                        artist: entry.artist,
                        language: row.language.clone(),
                        singer: entry.singer,
                        duration_ms: row.duration_ms,
                        melody_channel: row.melody_channel,
                        lyrics_hidden: row.lyrics_hidden,
                        fixes,
                        loudness_lufs: row.loudness_lufs,
                        gain: Loaded::UNLEVELLED,
                        media,
                    };
                    if self.start(loaded, row.default_transpose, load).is_ok() {
                        return;
                    }
                }
                Ok(None) => tracing::warn!(
                    number = %entry.number,
                    "a queued song is no longer in the catalog; skipping it"
                ),
                Err(error) => tracing::error!(
                    number = %entry.number,
                    %error,
                    "a queued song could not be loaded; skipping it"
                ),
            }
        }
        self.go_idle();
    }

    /// Loads a song into the engine and starts it.
    ///
    /// `song_transpose` is the value stored with the song in its package; the operator's own default
    /// is added on top. The live adjustment somebody made during the *previous* song is deliberately
    /// not carried over — a real machine resets the key per song, and inheriting it means the next
    /// singer starts in somebody else's key without being told.
    fn start(
        &self,
        mut loaded: Loaded,
        song_transpose: i8,
        load: km_audio::audio::Load,
    ) -> Result<(), ControlError> {
        if !self.engine.can_play() {
            return Err(ControlError::Unavailable(Refusal::coded(
                NO_SOUND,
                self.engine.sound().describe(),
            )));
        }

        let (
            transpose,
            tempo_ratio,
            melody_enabled,
            volume,
            lyric_offset_ms,
            normalize_media,
            normalize_midi,
            change_wallpaper,
        ) = {
            let settings = self.lock_settings();
            (
                song_transpose
                    .saturating_add(settings.playback.transpose)
                    .clamp(-km_queue::MAX_TRANSPOSE, km_queue::MAX_TRANSPOSE),
                settings.playback.tempo_ratio,
                // A video has no channel to mute, so the melody is off for one whatever the
                // setting says. `melody_channel` is already `None` for every video song, so this
                // needs nothing extra — stated here only because it is easy to read past.
                settings.playback.melody_enabled && loaded.melody_channel.is_some(),
                settings.audio.music_volume,
                // Carried through the reset rather than defaulted. The four above belong to a
                // performance and start again with each song; this one calibrates the room's
                // television, which does not change between songs. `update_settings` mirrors every
                // live change back here, so the stored value is always the current one.
                settings.display.lyric_offset(),
                settings.audio.normalize_media,
                settings.audio.normalize_midi,
                // Read here rather than beside the state lock below, because the two locks are
                // deliberately never held at once anywhere in this function.
                settings.wallpaper.on_song_change,
            )
        };

        // **The gain is worked out here and sent unconditionally, `1.0` included.** `Sticky` replays
        // the last value it saw into every stream it builds, so a start that sent nothing would
        // leave the previous song's attenuation in place — and the case that breaks is the ordinary
        // one: a MIDI song after a video would play at the video's reduction, quietly, for the whole
        // song. The bank switcher already paid for this lesson once; see `volume_for_bank`.
        //
        // The reference comes from the bank that is sounding *now* rather than from one resolved at
        // startup, because `Ctrl+1`…`Ctrl+9` can have replaced it since — and a bank swap changes
        // what a MIDI song renders at, so it changes what a media song has to come down to.
        // **A MIDI song's level is read from its own events, here, rather than from its package.**
        // It costs a fifth of a millisecond on a song that has already been parsed, and taking it
        // here rather than at packaging time is what makes it reach a file played straight from disk
        // and every package built before this. `estimated_loudness_db` is handed the melody channel
        // whatever the melody toggle says, so pressing that button mid-song cannot move the level.
        //
        // No bank means a sine test tone or no device at all — a machine that is already wrong in a
        // way somebody has to fix. Levelling against a test tone would be arithmetic over a number
        // that means nothing, so both kinds are left alone there.
        let has_bank = matches!(self.engine.sound(), Sound::SoundFont { .. });
        let midi_estimate = match &load {
            km_audio::audio::Load::Midi { song, .. } if normalize_midi && has_bank => {
                song.estimated_loudness_db(loaded.melody_channel)
            }
            _ => None,
        };
        // **Whether a MIDI song went unlevelled because nobody asked**, which is the one thing the
        // pair below cannot read off its own two inputs: the estimate is `None` for a song nothing
        // could measure and for a song nothing was allowed to measure, and those are different
        // answers to *why is this one so quiet*.
        let midi_levelling_off =
            matches!(&load, km_audio::audio::Load::Midi { .. }) && !normalize_midi;
        // **The number and the word it is drawn beside come out of one expression**, so they cannot
        // disagree about what happened to this song.
        let (song_gain, gain_source) = match (loaded.loudness_lufs, midi_estimate) {
            // A media song, brought down to the level the bank renders MIDI at. The reference comes
            // from the bank sounding *now* rather than one resolved at startup, because
            // `Ctrl+1`…`Ctrl+9` can have replaced it since.
            (Some(lufs), _) if normalize_media => match self.engine.sound() {
                Sound::SoundFont { path, .. } => (
                    km_loudness::gain_for(reference_lufs(&path), lufs),
                    km_display::GainSource::Package { lufs },
                ),
                Sound::TestTone { .. } | Sound::Silent { .. } => {
                    (1.0, km_display::GainSource::Unmeasured)
                }
            },
            // A MIDI song, moved either way to that same level. The bank cancels out of this one,
            // so no reference is read: see `km_loudness::midi_gain`.
            (None, Some(db)) => (
                km_loudness::midi_gain(db),
                km_display::GainSource::Events { db },
            ),
            // The owner has turned levelling off for this kind of song.
            (Some(_), _) => (1.0, km_display::GainSource::Disabled),
            // Nothing to level with: a song too short or too quiet to estimate, a media song in a
            // package built before levelling existed, or a machine with no bank to level against.
            (None, None) if !midi_levelling_off => (1.0, km_display::GainSource::Unmeasured),
            (None, None) => (1.0, km_display::GainSource::Disabled),
        };
        loaded.gain = (song_gain, gain_source);

        // A song somebody chose is a person being here, so the demo owes them a full silence after
        // it. Done at the *start* rather than at the end so that stopping or skipping mid-song
        // leaves the same deadline as letting it finish — three routes to one rule instead of three
        // copies of it.
        if !matches!(loaded.origin, Origin::Demo { .. }) {
            self.arm_demo(DemoEvent::Somebody);
        }

        let now_playing = loaded.describe();
        {
            let mut state = self.lock_state();
            state.settings = ApiSettings {
                transpose,
                tempo_ratio,
                melody_enabled,
                music_volume: volume,
                lyric_offset_ms,
            };
            state.announced_line = None;
            state.loaded = Some(loaded.clone());
            // **The picture belongs to the song rather than to the clock.** The same flag
            // `POST /wallpapers/next` sets, so a song start takes the display's one change path —
            // re-resolve the folder, rescan it, advance the playlist, ask the loader — instead of
            // a second route that would have to be kept in step with it.
            //
            // Set for every song kind. A video or MP3+G song's own picture covers the wallpaper,
            // so this changes one nobody can see; the alternative is a song-kind test in the one
            // place song kinds are otherwise settled, and the picture still has to be right the
            // moment the song ends.
            if change_wallpaper {
                state.wallpaper_requested = true;
            }
        }

        self.engine.send(Command::Load(load));
        self.engine.send(Command::SetTranspose(transpose));
        self.engine.send(Command::SetTempoRatio(tempo_ratio));
        self.engine.send(Command::SetMelodyEnabled(melody_enabled));
        self.engine.send(Command::SetMusicVolume(volume));
        self.engine.send(Command::SetSongGain(song_gain));
        self.engine.send(Command::Play);
        // `debug!` rather than `info!`: this is one line per song on a machine that plays all
        // evening, and the number is only interesting when somebody is asking why a song sounds the
        // way it does. `km-pack inspect` is where the measurements are read in bulk.
        if let Some(lufs) = loaded.loudness_lufs {
            tracing::debug!(
                lufs,
                song_gain,
                normalize_media,
                "levelled this song against the bank's own loudness"
            );
        } else if let Some(estimated) = midi_estimate {
            tracing::debug!(
                estimated,
                song_gain,
                normalize_midi,
                "levelled this song against the corpus, from its own events"
            );
        }

        self.events.publish(Event::SongStarted {
            now_playing: km_api::dto::NowPlayingDto::from(&now_playing),
        });
        self.publish_settings();
        // Last, after the event is out: the song this one displaced may have been an audition, and
        // no disk work belongs between a song starting and the announcement that it has.
        self.reclaim_auditions(AUDITION_TRIES);
        Ok(())
    }

    /// Unloads and reports an idle machine.
    fn go_idle(&self) {
        let had_song = {
            let mut state = self.lock_state();
            state.announced_line = None;
            state.loaded.take().is_some()
        };
        self.engine.send(Command::Unload);
        if had_song {
            tracing::debug!("nothing left to play");
        }
        // The other half of the pair with `start`: between them they cover every way a song stops
        // being the loaded one, because those are the only two places `state.loaded` is written.
        self.reclaim_auditions(AUDITION_TRIES);
    }

    /// A catalog song's row and the package that holds it, opened.
    fn package_for(
        &self,
        number: SongCode,
    ) -> Result<Option<(CatalogSong, Arc<km_kmpkg::Package>)>, CatalogError> {
        // The library lock is taken and dropped before any file I/O, so a slow archive read never
        // blocks a search.
        let (row, package_path) = {
            let library = self.lock_library();
            let Some(row) = library.song(number).map_err(library_failed)? else {
                return Ok(None);
            };
            let path = library.package_path_for(number).map_err(library_failed)?;
            (row, path)
        };
        let Some(package_path) = package_path else {
            return Err(CatalogError::Failed(format!(
                "song {number} names package '{}', which is not installed",
                row.package_id
            )));
        };

        let package = self.open_package(&row.package_id, Path::new(&package_path))?;
        Ok(Some((row, package)))
    }

    /// Parses a catalog song out of its package.
    fn load_from_catalog(
        &self,
        number: SongCode,
    ) -> Result<Option<(LoadedMedia, CatalogSong)>, CatalogError> {
        let Some((row, package)) = self.package_for(number)? else {
            return Ok(None);
        };

        if row.kind.is_video() {
            // A window into the package, not a file beside it and not bytes in memory: the decoder
            // seeks to the entry's byte range and reads the archive as a file from there.
            let media = package
                .media_reader(u32::from(number.slot()))
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            let name = media.name().to_owned();
            // The device's rate, so the track resamples once on the way out. Falls back to a common
            // rate when nothing has been probed yet, which only happens if no device exists at all —
            // in which case playback is refused before this matters.
            let rate = match self.engine.sample_rate() {
                0 => 48_000,
                rate => rate,
            };
            let (song, track) = VideoSong::open_from(media, &name, rate)
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            return Ok(Some((LoadedMedia::Video { song, track }, row)));
        }

        if row.kind.is_cdg() {
            // Two entries, and both are required: the manifest names the audio and the graphics are
            // found by rule from its name. The audio is seeked into and the graphics read whole,
            // which is `km-cdg`'s own asymmetry rather than one invented here.
            let audio = package
                .media_reader(u32::from(number.slot()))
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            let audio_name = audio.name().to_owned();
            let graphics = package
                .graphics_bytes(u32::from(number.slot()))
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            let graphics_name = km_kmpkg::graphics_entry_for(&audio_name);
            let rate = match self.engine.sample_rate() {
                0 => 48_000,
                rate => rate,
            };
            let (song, track) = CdgSong::open_from(
                audio,
                &audio_name,
                &graphics,
                &graphics_name,
                rate,
                row.duration_ms,
            )
            .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            return Ok(Some((LoadedMedia::Cdg { song, track }, row)));
        }

        if row.kind.carries_timeline() {
            // The audio is seeked into, as an MP3+G song's is, and the words were turned into a
            // timeline when the package was built: nothing here reads an UltraStar or LRC file.
            let slot = u32::from(number.slot());
            let audio = package
                .media_reader(slot)
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            let audio_name = audio.name().to_owned();
            let timeline = package
                .lyric_timeline(slot)
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            let rate = match self.engine.sample_rate() {
                0 => 48_000,
                rate => rate,
            };
            let (song, track) = TimedSong::open_from(audio, &audio_name, timeline, rate)
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            return Ok(Some((LoadedMedia::Timed { song, track }, row)));
        }

        // **The slot, not the whole number**, and the same for the three media lookups above. A
        // package stores its songs under the numbers *it* gave them; the bank is what this machine
        // added on the way into the catalog, and asking a package for song 3500 when it holds 500
        // finds nothing.
        let bytes = package
            .read_song(u32::from(number.slot()))
            .map_err(|error| CatalogError::Failed(error.to_string()))?;

        // The encoding stored with the song overrides detection. That override exists because
        // detection is wrong on some real files, and a curator who fixed one should not have it
        // re-guessed on every playback.
        let options = match &row.lyric_encoding {
            Some(encoding) => ParseOptions::with_encoding(encoding.clone()),
            None => ParseOptions::default(),
        };
        let song = Song::parse(&bytes, &options)
            .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
        warn_if_damaged(&number.to_string(), &song);
        Ok(Some((LoadedMedia::Midi(Arc::new(song)), row)))
    }

    /// An open package, from the cache or freshly opened.
    fn open_package(&self, id: &str, path: &Path) -> Result<Arc<Package>, CatalogError> {
        if let Some(package) = self.lock_packages().get(id) {
            return Ok(Arc::clone(package));
        }
        // The same doubled path `install` above had: `PackageError` names the file itself.
        let package =
            Package::open(path).map_err(|error| CatalogError::Failed(error.to_string()))?;
        let package = Arc::new(package);
        self.lock_packages()
            .insert(id.to_owned(), Arc::clone(&package));
        Ok(package)
    }

    /// Applies a settings change to the engine, returning what now holds.
    fn apply_settings(&self, patch: &SettingsPatch) -> Result<ApiSettings, ControlError> {
        let (melody_available, adjustable, kind) = {
            let state = self.lock_state();
            let loaded = state.loaded.as_ref();
            (
                loaded.is_none_or(|loaded| loaded.melody_channel.is_some()),
                // With nothing loaded these still apply: they are the settings the *next* MIDI song
                // will start with, and a video or MP3+G song playing now must not stop somebody
                // preparing the key for the one after it. Only such a song actually on the machine
                // refuses.
                loaded.is_none_or(|loaded| loaded.kind.is_midi()),
                // Carried so the refusal can name the kind it is refusing for.
                loaded.map_or(SongKind::Midi, |loaded| loaded.kind),
            )
        };

        let mut commands: Vec<Command> = Vec::new();
        let mut state = self.lock_state();

        if let Some(transpose) = patch.transpose {
            if !(-km_queue::MAX_TRANSPOSE..=km_queue::MAX_TRANSPOSE).contains(&transpose) {
                return Err(ControlError::Rejected(format!(
                    "transpose must be between -{max} and {max} semitones",
                    max = km_queue::MAX_TRANSPOSE
                )));
            }
            if !adjustable {
                return Err(ControlError::Unavailable(has_no_key(kind)));
            }
            state.settings.transpose = transpose;
            commands.push(Command::SetTranspose(transpose));
        }
        if let Some(tempo_milli) = patch.tempo_milli {
            let ratio = tempo_milli as f32 / 1000.0;
            if !(km_queue::MIN_TEMPO_RATIO..=km_queue::MAX_TEMPO_RATIO).contains(&ratio) {
                return Err(ControlError::Rejected(format!(
                    "tempo must be between {min} and {max}",
                    min = km_queue::MIN_TEMPO_RATIO,
                    max = km_queue::MAX_TEMPO_RATIO
                )));
            }
            if !adjustable {
                return Err(ControlError::Unavailable(has_no_tempo(kind)));
            }
            state.settings.tempo_ratio = ratio;
            commands.push(Command::SetTempoRatio(ratio));
        }
        if let Some(enabled) = patch.melody_enabled {
            if enabled && !adjustable {
                return Err(ControlError::Unavailable(has_no_melody(kind)));
            }
            if enabled && !melody_available {
                // Refused rather than silently ignored: muting or unmuting a channel that was never
                // confidently identified would take out an arbitrary instrument.
                return Err(ControlError::Unavailable(Refusal::coded(
                    NO_MELODY_CHANNEL,
                    "no melody channel was detected for this song",
                )));
            }
            state.settings.melody_enabled = enabled;
            commands.push(Command::SetMelodyEnabled(enabled));
        }
        if let Some(volume_milli) = patch.music_volume_milli {
            let volume = (volume_milli as f32 / 1000.0).clamp(0.0, 1.0);
            state.settings.music_volume = volume;
            commands.push(Command::SetMusicVolume(volume));
        }
        if let Some(offset_ms) = patch.lyric_offset_ms {
            // Clamped like the volume above rather than refused like the transpose and tempo above
            // that: this is a calibration dial, and the number is meant to be nudged from a remote
            // while a song plays until the highlight lands on the beat.
            //
            // No `adjustable` check either. The three settings that have one are about a MIDI song's
            // notes and refuse while a video plays; this one is about the screen, and an owner
            // calibrating the room must not be told to wait for the video to end.
            //
            // And no `Command`: nothing about this reaches the engine, the audio or the real-time
            // thread. The audio is already right — it is the drawing that is early or late.
            state.settings.lyric_offset_ms =
                offset_ms.clamp(-MAX_LYRIC_OFFSET_MS, MAX_LYRIC_OFFSET_MS);
        }

        let settings = state.settings;
        drop(state);
        for command in commands {
            self.engine.send(command);
        }
        Ok(settings)
    }

    fn publish_queue(&self) {
        let entries: Vec<QueueEntry> = self.lock_state().queue.entries().cloned().collect();
        self.events.publish(Event::QueueChanged {
            queue: km_api::dto::QueueDto::new(&entries),
        });
    }

    fn publish_settings(&self) {
        let settings = self.lock_state().settings;
        self.events.publish(Event::SettingsChanged {
            settings: settings.into(),
        });
    }

    fn lock_state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_settings(&self) -> MutexGuard<'_, Settings> {
        self.settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The title and artist behind a song number, **if the catalog is free this instant**.
    ///
    /// For the keypad's live preview, which runs on the display thread once per key press. It
    /// deliberately try-locks and gives up rather than waiting: [`Catalog::install`] holds the
    /// library mutex for the whole of an install transaction, so a blocking lookup here would
    /// freeze the picture for as long as installing a package takes — a visible stall introduced by
    /// a decoration. Giving up costs nothing, because the caller has not recorded an answer and asks
    /// again on the next key press.
    ///
    /// Inherent rather than a [`Catalog`] method for the same reason: that trait is the API's
    /// contract, where a lookup that may decline to answer would be the wrong promise. A miss and a
    /// busy catalog are both `None` here, and neither is an error worth showing.
    pub fn song_preview(&self, number: SongCode) -> SongPreviewLookup {
        let Ok(library) = self.library.try_lock() else {
            return SongPreviewLookup::Busy;
        };
        match library.song(number) {
            Ok(Some(song)) => SongPreviewLookup::Found {
                title: song.title,
                artist: song.artist,
            },
            Ok(None) => SongPreviewLookup::Missing,
            // A failing catalog is not worth an error on the idle screen; the next press retries.
            Err(_) => SongPreviewLookup::Busy,
        }
    }

    /// Packages this start found and refused, newest last.
    pub fn package_problems(&self) -> Vec<PackageProblem> {
        self.lock_problems().clone()
    }

    /// The bank a package's songs are dialled in, if one has been decided.
    ///
    /// Settings only, and deliberately cheap: no allocation, no library lock. `None` means nobody
    /// has put this package anywhere yet — [`ensure_bank`](Self::ensure_bank) is what answers that.
    pub fn bank_for(&self, package_id: &str) -> Option<u16> {
        self.lock_settings().package_banks.get(package_id).copied()
    }

    /// The bank to install a package into, allocating one if it has none.
    ///
    /// **The machine assigns this, and a package only suggests.** That reverses half of what the
    /// prefix it replaces did, and the reversal has an argument rather than being a convenience: a
    /// prefix was a *name*, chosen for meaning, which two packagers could both want and which could
    /// not be taken back without renumbering; a bank is a *slot* with no meaning, the machine must
    /// fill it either way, and honoring a suggestion that is free takes nothing from anybody and
    /// changes nothing already installed.
    ///
    /// The order is what makes it safe, and each step is load-bearing:
    ///
    /// 1. **Settings**, so a package keeps the bank it was given.
    /// 2. **The catalog**, written back into settings — because a `settings.json` that was lost or
    ///    rescued from a `settings.json.bad` starts from an empty map, and without this step an
    ///    already-installed package could be moved to a different bank under a live queue. With it,
    ///    *automatic banking never moves a package that already has a bank, from either source*.
    /// 3. **The bank the package asks for**, and then the next free one after it — see
    ///    [`choose_bank`], which is where that half lives so it can be tested.
    ///
    /// The taken set is the **union** of what the catalog holds and what settings record, because
    /// settings also hold banks for packages that failed to install — handing one of those out would
    /// move a package the next time its file was fixed.
    ///
    /// **A recorded bank 0 is a stale value, not an assignment**, and is the one thing that reads
    /// past step 1 and step 2. Bank 0 is the machine's own and `Library::install` refuses it, so a
    /// package pinned there by a hand-edited `settings.json` would be refused at every start, which
    /// is a machine that quietly lost songs.
    /// It is banked as though nothing had been recorded, and the move is logged.
    ///
    /// `wanted` comes from [`km_kmpkg::PackageMeta::wanted_bank`], so the answer is the same one
    /// `km-pack book` prints for the same file.
    pub fn ensure_bank(&self, package_id: &str, wanted: u16) -> Result<u16, String> {
        if let Some(bank) = self.bank_for(package_id).filter(|bank| *bank != 0) {
            return Ok(bank);
        }

        let installed = {
            let library = self
                .library
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(bank) = library
                .bank_of(package_id)
                .ok()
                .flatten()
                .filter(|b| *b != 0)
            {
                self.record_bank(package_id, bank);
                return Ok(bank);
            }
            library.banks().unwrap_or_default()
        };

        let mut taken: BTreeSet<u16> = installed.into_iter().collect();
        taken.extend(self.lock_settings().package_banks.values().copied());

        let bank = choose_bank(wanted, &taken).ok_or_else(|| {
            format!(
                "this machine already holds {} packages, which is every bank it can assign",
                km_songcode::MAX_BANK
            )
        })?;
        if self.bank_for(package_id) == Some(0) {
            tracing::warn!(
                package = package_id,
                bank,
                "bank 0 is the machine's own; this package takes another bank"
            );
        }
        self.record_bank(package_id, bank);
        Ok(bank)
    }

    /// Writes a package's bank down, **before** the install that depends on it.
    ///
    /// The same ordering the prefix used and for the same reason: the answer has to survive an
    /// install that then fails, or the next attempt would allocate a different bank.
    fn record_bank(&self, package_id: &str, bank: u16) {
        self.lock_settings()
            .package_banks
            .insert(package_id.to_owned(), bank);
        self.save_settings();
    }

    /// How much is installed, for the idle screen to state — counted only if it has moved.
    ///
    /// Best effort, like [`song_preview`](Self::song_preview) and for exactly the same reason: this runs on
    /// the display thread, `Catalog::install` holds the library mutex for a whole transaction, and
    /// a blocking read here would freeze the picture for the length of an install.
    ///
    /// **`known` is what makes it cheap enough to call every frame.** `SELECT COUNT(*) FROM songs`
    /// on a catalog with hundreds of thousands of rows is a full index scan, and the display loop
    /// runs at up to 125 fps; `catalog_version` is one indexed row from `meta` that moves if and
    /// only if a package was installed or uninstalled. So the version is read first and the counting
    /// is skipped when it agrees — **inside the same lock**, which is the part that could not be
    /// done by the caller: two separate reads could straddle an install and pair a version with
    /// counts from the other side of it.
    pub fn catalog_counts(&self, known: Option<u64>) -> CatalogCounts {
        let Ok(library) = self.library.try_lock() else {
            return CatalogCounts::Busy;
        };
        let Ok(version) = library.catalog_version() else {
            return CatalogCounts::Busy;
        };
        if known == Some(version) {
            return CatalogCounts::Unchanged;
        }
        let (Ok(songs), Ok(packages)) = (library.song_count(), library.package_count()) else {
            return CatalogCounts::Busy;
        };
        CatalogCounts::Counted {
            version,
            songs,
            packages,
        }
    }

    fn lock_problems(&self) -> MutexGuard<'_, Vec<PackageProblem>> {
        self.problems
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_library(&self) -> MutexGuard<'_, Library> {
        self.library
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_packages(&self) -> MutexGuard<'_, HashMap<String, Arc<Package>>> {
        self.packages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Writes settings out now.
    ///
    /// Used for changes that must not be lost to a power cut — installing or removing a package.
    /// Failure is logged, not propagated: the operation itself succeeded, and the caller can do
    /// nothing useful about a read-only config directory.
    fn save_settings(&self) {
        let snapshot = self.lock_settings().clone();
        if let Err(error) = snapshot.save(&self.paths) {
            tracing::error!(%error, "could not write settings");
        }
    }

    /// Whether the output device could be changed this instant.
    ///
    /// Two conditions, and the second is the one that is easy to miss. `Transport::Idle` is the only
    /// state with no song inside the player — `Stopped` and `Paused` both hold one, positioned — so
    /// anything else would have its song dropped along with the stream. And **an idle machine with a
    /// queued song is a machine about to start one**: `poll` loads the next entry the moment the
    /// transport goes idle, so a change accepted in that gap would land in the middle of a song
    /// starting.
    ///
    /// Not a mutex over the whole operation, and it does not need to be: the worst a lost race can
    /// do is drop a stream a song had just begun on, which the machine already survives — it is what
    /// an unplugged device does, and `stream_failed` handles it.
    fn output_change_allowed(&self) -> bool {
        self.engine.transport() == Transport::Idle && self.lock_state().queue.is_empty()
    }

    /// What is in the wallpaper folder, one row per file.
    ///
    /// **Built by scanning here rather than by asking the display**, and that is the design decision
    /// worth stating. The `Playlist` is owned by the render thread and only four scalars cross back
    /// (`set_wallpaper_state`); plumbing a list across would widen that seam for a page. But
    /// `Playlist::scan` is a plain filesystem read that `place_wallpaper` and `holds_wallpapers`
    /// already call off that thread, so the listing can be built here with the boundary untouched.
    ///
    /// The rows are *files*: a loose picture is one row saying one image, an archive is one row
    /// saying however many it holds. `debug.wallpapers` extras are listed too — leaving them out
    /// would show a count that disagrees with the screen — and each carries the sentence saying it
    /// is not the machine's to delete.
    pub fn wallpaper_pictures(&self) -> Vec<km_api::machine::Picture> {
        let (dir, source) = self.wallpaper_folder();
        let extra = self.debug_wallpapers();
        let playlist = km_display::Playlist::scan(&dir, &extra);

        // Grouped by the file each image came out of, in first-seen order — which is scan order,
        // so the page lists files by name. Deliberately not the order the cycle walks them in:
        // `Playlist::scan` is unshuffled, and a list of *files* somebody is looking a name up in
        // has to be somewhere the eye can find it rather than reordered every thirty seconds.
        let mut order: Vec<PathBuf> = Vec::new();
        let mut images: HashMap<PathBuf, usize> = HashMap::new();
        for entry in playlist.entries() {
            let container = entry.container().to_path_buf();
            if images.insert(container.clone(), 0).is_none() {
                order.push(container.clone());
            }
            *images.get_mut(&container).unwrap_or(&mut 0) += 1;
        }

        order
            .into_iter()
            .filter_map(|path| {
                let id = picture_id(&path)?;
                let name = path.file_name()?.to_str()?.to_owned();
                let images = images.get(&path).copied().unwrap_or(1);
                let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                let why_not_removable = self.picture_not_mine_to_delete(&path, &dir, source);
                Some(km_api::machine::Picture {
                    id,
                    name,
                    images,
                    bytes,
                    why_not_removable,
                })
            })
            .collect()
    }

    /// The **permanent** reasons a picture cannot be removed, or `None`.
    ///
    /// The picture twin of [`Self::bank_not_mine_to_delete`], and the same contract: only refusals
    /// that will still be refusals in a minute, so a page can spend this by leaving the control off
    /// the row rather than drawing one that is always refused.
    ///
    /// Two rules, and between them only the owner's own folder yields a removable row:
    ///
    /// * **The folder that won is not the machine's.** A `wallpaper.dir` setting names somebody
    ///   else's folder, an overlay is a checkout's, and the bundled set is an unpacked asset that
    ///   would be back on the next launch — which is exactly the argument
    ///   [`Self::bank_not_mine_to_delete`] makes about the bundled bank: deleting it succeeds and
    ///   convinces nobody.
    /// * **A file `debug.wallpapers` names is not the machine's**, wherever it sits. That is the
    ///   rule `Only \`debug.\` names a file` states — what `debug.` names belongs to the owner — and
    ///   it was written down for this route *before this route existed*, in `The wallpaper folder is
    ///   chosen again, not once`: any such control must refuse for a named file rather than
    ///   silently do nothing.
    ///
    /// Compared with `same_file` rather than `==`, for the reason the bank rule gives: a `debug.`
    /// path is stored exactly as it was typed and never absolutised, so a relative entry would
    /// defeat a literal comparison and leave the file deletable after all.
    fn picture_not_mine_to_delete(
        &self,
        path: &Path,
        dir: &Path,
        source: crate::settings::WallpaperSource,
    ) -> Option<String> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        if self
            .debug_wallpapers()
            .iter()
            .any(|extra| crate::soundfont::same_file(extra, path))
        {
            return Some(format!(
                "\"{name}\" is named by debug.wallpapers, so its file is not the machine's to \
                 remove: take it out of that list first"
            ));
        }
        match source {
            crate::settings::WallpaperSource::Owner => None,
            crate::settings::WallpaperSource::Bundled => Some(format!(
                "\"{name}\" ships with the machine and would be back on the next start"
            )),
            crate::settings::WallpaperSource::Setting => Some(format!(
                "the pictures come from the folder wallpaper.dir names ({}), which is not the \
                 machine's to change",
                dir.display()
            )),
            crate::settings::WallpaperSource::Overlay => Some(format!(
                "\"{name}\" is in a checkout's assets folder, which is not the machine's to change"
            )),
        }
    }

    /// Removes one file from the wallpaper folder.
    ///
    /// **The picture that is showing needs no fallback, unlike a SoundFont**, and that is the one
    /// place this and [`Self::delete_soundfont`] genuinely differ. A bank is *named by a setting*,
    /// so deleting the named one has to choose another first or leave the machine pointing at
    /// nothing; a wallpaper is a position in a list that is rebuilt from the folder, and
    /// `Playlist::refresh` already re-finds the showing image by value and falls to the first entry
    /// when it has gone. So there is nothing to unwind and the file goes **first** here, where a
    /// bank's goes last.
    ///
    /// **Both flags, and each does a different job.** `wallpaper_dir_stale` because removing the
    /// last picture an owner had makes their folder lose the argument and the shipped set come back
    /// — the folder is chosen by contents at every cycle — and `wallpaper_requested` so the deleted
    /// picture leaves the screen at once rather than at the end of its interval. A picture that is
    /// still on the television after being removed reads as a broken button, which is what `The
    /// wallpaper folder is chosen again, not once` asks this route to avoid.
    pub fn delete_wallpaper(&self, id: &str) -> Result<(), ControlError> {
        // Resolved through a fresh scan rather than by rebuilding a path out of the id, which is the
        // rule `delete_soundfont` keeps: ids are slugged and do not round-trip, so a crafted one has
        // nothing to reach. The path deleted is the one the *scan* produced, never `dir.join(name)`
        // — a `debug.wallpapers` extra lives outside the folder, so reconstructing would name a
        // different file than the row the refusal was computed for.
        let (dir, source) = self.wallpaper_folder();
        let extra = self.debug_wallpapers();
        let path = km_display::Playlist::scan(&dir, &extra)
            .entries()
            .iter()
            .map(|entry| entry.container().to_path_buf())
            .find(|path| picture_id(path).as_deref() == Some(id))
            // `Rejected` rather than the payload-free `NotFound`, which is what `delete_soundfont`
            // does one screen up: an id that is not in the list is worth naming, and the sibling
            // routes answering the same shape the same way is worth more than the status code.
            .ok_or_else(|| ControlError::Rejected(format!("no wallpaper called {id}")))?;

        if let Some(reason) = self.picture_not_mine_to_delete(&path, &dir, source) {
            return Err(ControlError::Rejected(reason));
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        std::fs::remove_file(&path).map_err(|error| {
            ControlError::Rejected(format!("{} could not be removed: {error}", path.display()))
        })?;

        {
            let mut state = self.lock_state();
            state.wallpaper_dir_stale = true;
            state.wallpaper_requested = true;
        }
        tracing::info!(picture = %name, id = %id, "removed a wallpaper");
        Ok(())
    }

    /// Which device a level should be read from, given what is active.
    ///
    /// **"Follow the system" is not a device, and a mixer needs one.** The sentinel is what the
    /// engine reports as active whenever nothing has been chosen by hand, which is every machine
    /// out of the box — so resolving it here is what keeps the level from being a thing only a
    /// machine with a hand-picked output has. The device list already marks the real device the
    /// sentinel resolves to today, so this is a lookup rather than a second question to the
    /// backend.
    fn level_device<'a>(active: &'a str, devices: &'a [km_audio::device::OutputDevice]) -> &'a str {
        if active != km_audio::SYSTEM_DEFAULT {
            return active;
        }
        devices
            .iter()
            .find(|device| device.system_default)
            .map_or(active, |device| device.id.as_str())
    }

    /// The level the named output is running at, or `None` where there is none to report.
    ///
    /// **A mixer that will not answer is reported as no level rather than as a failed request.**
    /// The device list is what a person came to this page for, and refusing the whole of it because
    /// one card would not open its mixer would take away the control that chooses a different card.
    /// The reason goes to the log, where somebody debugging a silent box will be reading already.
    fn read_output_level(active_id: &str) -> Option<km_api::machine::OutputLevel> {
        match km_audio::level::read(active_id) {
            Ok(level) => level.map(|level| km_api::machine::OutputLevel {
                db_centi: Self::centi(level.db),
                db_min_centi: Self::centi(level.db_min),
                db_max_centi: Self::centi(level.db_max),
                step_centi: Self::centi(level.step_db),
            }),
            Err(error) => {
                tracing::debug!(%error, device = %active_id, "could not read the output level");
                None
            }
        }
    }

    /// Decibels as the hundredths the API's vocabulary carries.
    fn centi(db: f32) -> i32 {
        if db.is_nan() {
            return 0;
        }
        (db * 100.0).round() as i32
    }

    /// The output devices, and which one is in use.
    fn describe_audio_outputs(&self) -> Result<AudioOutputs, ControlError> {
        let status = self.engine.output_status();
        let devices = self
            .engine
            .output_devices()
            .map_err(|error| ControlError::Failed(error.to_string()))?;
        // Read before the identifier is moved into the answer, and from the *active* device: after
        // a fallback that is not the one settings name, and the level belongs to what is playing.
        let level = Self::read_output_level(Self::level_device(&status.active.id, &devices));
        Ok(AudioOutputs {
            devices: devices
                .into_iter()
                .map(|device| AudioOutput {
                    id: device.id,
                    name: device.name,
                    system_default: device.system_default,
                    usb: device.usb,
                    available: device.available,
                    preferred: device.preferred,
                })
                .collect(),
            selected: status.requested,
            active_id: status.active.id,
            active_name: status.active.name,
            fell_back: status.active.fell_back,
            changeable: self.output_change_allowed(),
            level,
        })
    }

    /// Writes the mic registry and playback defaults back to settings.
    ///
    /// Called on shutdown rather than on every change: a singer nudging a slider should not cause a
    /// disk write, and losing the last adjustment on a hard power cut is not worth avoiding.
    pub fn persist(&self) {
        let (mics, wallpaper_current) = {
            let state = self.lock_state();
            (state.mics.channels(), state.wallpapers.current.clone())
        };
        let _ = wallpaper_current;
        let mut settings = self.lock_settings();
        settings.mics = mics
            .iter()
            .map(crate::settings::MicSettings::from)
            .collect();
        let snapshot = settings.clone();
        drop(settings);
        if let Err(error) = snapshot.save(&self.paths) {
            tracing::error!(%error, "could not write settings on shutdown");
        }
    }
}

/// The first free bank at or after `wanted`, wrapping, or `None` when every one is held.
///
/// **Never bank 0**, which is the machine's own and holds no package by any road — see
/// `Bank 0 is the machine's own`. Nothing derived can land there either
/// ([`km_kmpkg::PackageMeta::suggested_bank`] returns 1 to `MAX_BANK`), and
/// [`km_songcode::SongCode::in_bank`] would refuse the number even if something did, so this
/// function, the book and the catalog agree without any of them knowing about the others.
///
/// **Probing from `wanted` rather than scanning from the bottom**, and the reason is not tidiness. A
/// package that loses a tie lands *beside* where every other machine puts it rather than at the far
/// end of the range, so the book printed from the file is wrong by one thousand rather than by
/// wherever the range happened to be empty.
///
/// Pure, and that is why it exists apart from [`Machine::ensure_bank`]: constructing a `Machine`
/// needs an audio device and an instrument bank, so the allocator went in with no test at all.
fn choose_bank(wanted: u16, taken: &BTreeSet<u16>) -> Option<u16> {
    // `wanted` is clamped rather than trusted: it comes from a manifest by way of `wanted_bank`,
    // which already refuses an out-of-range value, but a caller that got that wrong should probe
    // from somewhere real rather than skip most of the range.
    let start = wanted.clamp(1, km_songcode::MAX_BANK);
    (0..km_songcode::MAX_BANK)
        .map(|step| (start - 1 + step) % km_songcode::MAX_BANK + 1)
        .find(|bank| !taken.contains(bank))
}

/// The API's word for which rule chose the wallpaper folder.
///
/// Two enums rather than one, and it is the seam the whole crate is built on: `km-api` describes a
/// machine without depending on this crate, so it has its own vocabulary and this is the one place
/// the two are joined. The same bargain `SoundKind` and `AudioOutputs` already strike.
fn api_wallpaper_source(
    source: crate::settings::WallpaperSource,
) -> km_api::machine::WallpaperSource {
    match source {
        crate::settings::WallpaperSource::Setting => km_api::machine::WallpaperSource::Setting,
        crate::settings::WallpaperSource::Owner => km_api::machine::WallpaperSource::Owner,
        crate::settings::WallpaperSource::Overlay => km_api::machine::WallpaperSource::Overlay,
        crate::settings::WallpaperSource::Bundled => km_api::machine::WallpaperSource::Bundled,
    }
}

fn library_failed(error: km_catalog::LibraryError) -> CatalogError {
    CatalogError::Failed(error.to_string())
}

/// Which missing packages may be dropped now, and which wait.
///
/// **Split by whether there is anything to prune, not by whether the machine is busy**, and that is
/// the whole of the rule. Refusing a rescan outright while something is playing would gut the
/// feature: the moment it is wanted is a party, when somebody has just handed over a stick and the
/// machine has been going all evening. But almost every rescan has nothing to remove — it is
/// somebody adding a package — so the gate is reached only in the uncommon case and never stands
/// between the owner and a file they just dropped in.
///
/// `busy` is the machine having something loaded or queued.
///
/// **What deferring costs**: a package whose file was taken away mid-party keeps its rows until the
/// machine is idle, so its numbers stay dialable and fail at load. That is what a file deleted under
/// a running machine does anyway — this declines to fix it live rather than causing it. The alternative removes rows a queue entry points
/// at, and a singer losing their turn in silence is worse than a number that says it cannot be
/// opened.
fn prunable(doomed: Vec<String>, busy: bool) -> (Vec<String>, Vec<String>) {
    if doomed.is_empty() || !busy {
        return (doomed, Vec::new());
    }
    (Vec::new(), doomed)
}

/// Why a package's file is not this machine's to delete, in a sentence, or `None` if it is.
///
/// **One copy of the rule, two callers, and that is the point.** [`Machine::uninstall`] turns this
/// into a `Rejected`, and `Catalog::why_not_removable` hands it to a page so the Remove control is
/// never drawn — so a page that leaves the button out leaves it out for exactly the sentence the
/// route would have refused with, rather than for a rule it restated and can get wrong.
///
/// **A sentence rather than a `bool`**, because whoever asks in advance has to say *why* the control
/// is missing, and the only wording that cannot be wrong is the one the refusal itself uses.
///
/// Both refusals are **permanent** — neither becomes allowed by waiting — which is what makes hiding
/// the control right where `AudioOutputs::changeable` grays one out instead. A device that cannot be
/// changed now can be changed when the song ends; a package in `debug.packages` stays there until
/// somebody edits a settings file.
///
/// Free rather than a method, so it can be tested against a [`Paths`] rooted at a scratch directory,
/// with no machine, no catalog and no audio device.
/// The one refused package a delete names, or why it names none.
///
/// **A free function so it can be tested**, which the operation around it cannot be: building a
/// `Machine` needs an audio engine and a library on disk, so the pieces of it worth pinning are
/// pulled out here exactly as [`not_mine_to_delete`] is.
///
/// Two answers matter and only one of them is obvious:
///
/// * **Nothing matched** is [`CatalogError::NotFound`], and it usually means the good thing — a
///   rescan took the file in between a page being drawn and its link being pressed.
/// * **More than one matched** is a refusal rather than a coin toss. The id carries a 32-bit
///   fingerprint of the whole path, so this needs a genuine collision to fire at all and very
///   likely never will — which is precisely why it is written down: the one time it did fire, a
///   bare `find` would silently delete whichever file came first, and that is the worst outcome
///   this operation has available.
fn resolve_problem(problems: &[PackageProblem], id: &str) -> Result<PackageProblem, CatalogError> {
    let mut matched = problems.iter().filter(|problem| problem.id() == id);
    let Some(problem) = matched.next() else {
        return Err(CatalogError::NotFound(format!("package '{id}'")));
    };
    if matched.next().is_some() {
        return Err(CatalogError::Rejected(format!(
            "two of the refused files answer to \"{id}\", so neither was deleted: remove one of \
             them yourself and the next pass will sort the rest out"
        )));
    }
    Ok(problem.clone())
}

fn not_mine_to_delete(paths: &Paths, debug_packages: &[PathBuf], path: &Path) -> Option<String> {
    // **Nothing named in `debug.` is the machine's to remove**, neither the file nor the entry.
    // Refused rather than half-done: dropping the rows alone would not stick, because the next pass
    // reads the same list and puts the package back — which is the rot the folders-are-the-truth
    // change removed. And editing that list from here would put an API writer on the one section
    // whose safety comes from only a person at a keyboard changing it; `debug.soundfonts` has no API
    // writer either, and `DELETE /audio/soundfonts/{id}` deletes a downloaded bank rather than a
    // slot.
    if debug_packages.iter().any(|named| tidy(named) == tidy(path)) {
        return Some(format!(
            "\"{}\" is named in debug.packages, so neither its file nor that entry is the \
             machine's to remove: take it out with --clear-debug-packages",
            path.display()
        ));
    }

    // **And nothing outside a folder this machine owns is deletable at all.** Structural rather than
    // a check somebody has to remember at each call site: what it rules out is `Paths::asset_dir`,
    // the tree that ships with the build and is read-only where it counts — root-owned under `/opt`
    // on Debian, inside a signed bundle on macOS, unpacked from the APK on Android. Nothing today
    // puts a package there, and this is what keeps that true of whatever route somebody adds next.
    if !paths.is_mine_to_delete(path) {
        return Some(format!(
            "\"{}\" is not in a folder this machine owns, so it is not the machine's to delete: \
             remove the file yourself, and the next pass will drop its songs",
            path.display()
        ));
    }

    None
}

/// A package's id, for the startup pass's deduplication. `None` if the file will not open.
///
/// Costs one zip central-directory seek and one manifest parse per candidate, which is nothing
/// beside indexing the songs — and a good deal less than indexing them twice.
fn package_id_of(path: &Path) -> Option<String> {
    Package::open(path)
        .ok()
        .map(|package| package.manifest().package.id.clone())
}

/// Which candidates to install this pass, in order, each package only once.
///
/// The deciding half of the startup pass, kept apart from the filesystem and the installing so it
/// can be tested against a path-to-id map with no `.kmpkg`, no catalog and no audio device — the
/// same split [`crate::settings::packages_to_install`] makes for the same reason.
///
/// **Deduplicated by package id, not by path**, and that is the whole reason this exists. Nothing is
/// remembered between starts any more, so the old channel for this — appending to
/// `settings.packages` and re-reading it per folder — is gone. Deduplicating by path would be
/// cheaper and would miss the case that actually happens: **one package present twice under two
/// names**, in the packages folder and in a `package_dirs` folder, or as one of the `-2` copies
/// `dropped::free_name` mints. [`Library::install`] replaces by id, so installing it twice is
/// *correct* and merely re-indexes a four-thousand-song package for nothing, at every start,
/// invisibly: the digest check means it does not even register as a catalog version move.
///
/// Paths are deduplicated too, ahead of the ids, because it is free and it catches the same file
/// named twice — a `debug.packages` entry spelling a file that is also in a scanned folder, or two
/// `package_dirs` entries that are the same folder. Both sides go through [`tidy`].
///
/// A candidate whose id cannot be read is **kept rather than dropped**, so it reaches
/// [`Machine::install_and_report`] and fails there — one place records a fault, and it is the one
/// that puts it above the title.
fn startup_plan(
    candidates: impl IntoIterator<Item = PathBuf>,
    id_of: impl Fn(&Path) -> Option<String>,
) -> Vec<PathBuf> {
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();
    let mut plan: Vec<PathBuf> = Vec::new();

    for path in candidates {
        if !seen_paths.insert(tidy(&path)) {
            continue;
        }
        if let Some(id) = id_of(&path)
            && !seen_ids.insert(id.clone())
        {
            tracing::info!(
                package = %id,
                path = %path.display(),
                "skipping a second copy of a package already offered this pass"
            );
            continue;
        }
        plan.push(path);
    }

    plan
}

/// Moves one finished upload out of the staging folder and into `dir`, and says what it is called.
///
/// **A rename first and a copy only if that fails**, which is the ordinary shape and worth stating
/// here because the staging folder is inside the data directory: on the same filesystem the rename
/// is atomic and free, and the copy is for a data directory somebody has put across a mount point.
///
/// The name is the staged file's own, which is safe because it is not the client's: the handler
/// sanitised it before a byte was written, and what is on disk is what that produced.
fn move_into(dir: &Path, staged: &Path) -> Result<String, ControlError> {
    let name = staged
        .file_name()
        .ok_or_else(|| ControlError::Rejected("the upload has no name".to_owned()))?
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(dir).map_err(|error| {
        ControlError::Unavailable(format!("could not make {}: {error}", dir.display()).into())
    })?;
    let destination = dir.join(&name);
    if std::fs::rename(staged, &destination).is_err() {
        std::fs::copy(staged, &destination).map_err(|error| {
            ControlError::Unavailable(format!("could not keep the upload: {error}").into())
        })?;
        let _ = std::fs::remove_file(staged);
    }
    Ok(name)
}

/// A stable id for one file in the wallpaper folder.
///
/// **The whole file name, extension included**, where [`crate::soundfont::bank_id`] slugs the stem.
/// The difference is not an inconsistency: a folder holds one `.sf2` per bank, and a wallpaper
/// folder very reasonably holds `sunset.jpg` beside `sunset.png`, which a stem-based id would give
/// one id and make one of them undeletable.
///
/// Like a bank id it does not need to round-trip: [`Machine::delete_wallpaper`] finds the file by
/// matching a fresh scan rather than by joining this onto the folder, which is what stops a crafted
/// id reaching the filesystem.
fn picture_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let mut id = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-').to_owned();
    if id.is_empty() { None } else { Some(id) }
}

/// Puts an uploaded wallpaper into the owner's folder, and says how many pictures are now in it.
///
/// **A zip is moved in whole and never unpacked.** Unpacking is the obvious thing to write here and
/// would be wrong twice over. `km_display::Playlist` already reads
/// the images *inside* an archive on every scan — that is the `Zipped wallpapers` decision, and a
/// pack is the natural unit for a set of pictures — so unpacking would produce a folder holding a
/// zip's worth of loose files beside the zip and count everything twice. It would also mean writing
/// files under names an archive somebody else made chose, which is the oldest file-writing bug
/// there is.
///
/// The count is of the folder afterwards rather than of what arrived, because that is the number an
/// owner is actually asking about: how many pictures are in the rotation now. It is also the only
/// honest answer for a zip, whose contents this deliberately never opens.
fn place_wallpaper(dir: &Path, staged: &Path) -> Result<usize, ControlError> {
    move_into(dir, staged)?;
    // The folder's own count, with no `debug.wallpapers` extras: this answers "what did my upload
    // land beside", and an extra is not in the folder and is not something an upload can affect.
    Ok(km_display::Playlist::scan(dir, &[]).len())
}

/// The reason a package was refused, with the file it happened to taken off the front.
///
/// [`PackageProblem`] carries the path in a field of its own, and **all three things that show a
/// problem to somebody deliberately show the file's name instead of it** — the idle screen
/// (`display::package_notice`), the machine's own remote (`remote.rs`) and `PackageProblemDto::file`,
/// each of which says so in a comment. The reason then put the whole path back past all three. On a
/// television that is not merely redundant: the notice gets two lines above the title, its word wrap can
/// only break on whitespace, and one Windows path is one unbreakable word — so the path took a whole
/// line and the reason fell off the end, leaving a sentence ending in a bare colon.
///
/// Stripping rather than rendering the error a second way, because there is only one place the two
/// spellings can disagree: both strings come from `Path::display` on the same path, in this same
/// function. Rebuilding a path-free message instead would mean a second copy of every `#[error]`
/// string in `PackageError` — and the CLI tools that print those *want* the file named, so the
/// attribute cannot simply lose it.
///
/// Every variant begins with the path and then either `: ` (`Io`, `Manifest`) or a space and a verb
/// (`Archive`, `NoManifest`, `Invalid`), so both joins are taken off. A reason that does not begin
/// with the path is returned untouched.
pub(crate) fn reason_without_path(path: &str, reason: &str) -> String {
    let Some(rest) = reason.strip_prefix(path) else {
        return reason.to_owned();
    };
    let rest = rest.strip_prefix(": ").unwrap_or(rest).trim_start();
    // Never leave nothing at all: a reason that was only the path is worse than the path.
    if rest.is_empty() {
        return reason.to_owned();
    }
    rest.to_owned()
}

impl Catalog for Machine {
    fn search(&self, query: &SearchQuery) -> Result<Vec<CatalogSong>, CatalogError> {
        self.lock_library().search(query).map_err(library_failed)
    }

    fn song(&self, number: SongCode) -> Result<Option<CatalogSong>, CatalogError> {
        self.lock_library().song(number).map_err(library_failed)
    }

    fn song_in_package(
        &self,
        package_id: &str,
        content_hash: &str,
    ) -> Result<Option<CatalogSong>, CatalogError> {
        self.lock_library()
            .song_in_package(package_id, content_hash)
            .map_err(library_failed)
    }

    fn song_by_content(&self, content_hash: &str) -> Result<Option<CatalogSong>, CatalogError> {
        self.lock_library()
            .song_by_content(content_hash)
            .map_err(library_failed)
    }

    fn has_package(&self, package_id: &str) -> Result<bool, CatalogError> {
        self.lock_library()
            .has_package(package_id)
            .map_err(library_failed)
    }

    fn load(&self, number: SongCode) -> Result<Option<Arc<Song>>, CatalogError> {
        // This exists to serve the lyrics endpoint, so the kind is checked *before* anything is
        // loaded: opening a video decoder in order to answer "there are no lyrics" would spawn a
        // thread and read a file for nothing. A video song has no lyric timeline at all — its words
        // are pixels in its own picture — so the endpoint reports it as having none.
        let Some(row) = self.lock_library().song(number).map_err(library_failed)? else {
            return Ok(None);
        };
        // A song whose words are turned off reports none, before anything is opened. The endpoint
        // answers a video the same way and for the same reason: what it is asked is whether there
        // are words to read, and a client that fetched the timeline would draw what the television
        // is refusing to.
        if row.lyrics_hidden {
            return Ok(None);
        }
        // An UltraStar or LRC song's timeline is read out of its package without its audio: loading
        // the song whole would start a decoder thread to answer a question about its words.
        if row.kind.carries_timeline() {
            let Some((_, package)) = self.package_for(number)? else {
                return Ok(None);
            };
            let timeline = package
                .lyric_timeline(u32::from(number.slot()))
                .map_err(|error| CatalogError::Failed(format!("song {number}: {error}")))?;
            return Ok(Some(Arc::new(km_song::recording::song_from_timeline(
                timeline,
            ))));
        }
        // A video and an MP3+G song have no timeline, and both would otherwise start a decoder
        // thread here just to be told so.
        if !row.kind.is_midi() {
            return Ok(None);
        }
        Ok(self
            .load_from_catalog(number)?
            .and_then(|(media, _)| match media {
                LoadedMedia::Midi(song) => Some(song),
                LoadedMedia::Video { .. } | LoadedMedia::Cdg { .. } | LoadedMedia::Timed { .. } => {
                    None
                }
            }))
    }

    fn packages(&self) -> Result<Vec<InstalledPackage>, CatalogError> {
        self.lock_library().packages().map_err(library_failed)
    }

    fn package_problems(&self) -> Vec<PackageProblem> {
        Machine::package_problems(self)
    }

    /// Moves a package to another block of a thousand.
    ///
    /// **Refused while anything is playing or queued.** Every song in the package changes its
    /// number, and the queue holds numbers — so a queue built before the change would name songs
    /// that no longer exist, and the song on screen would be one nobody could ask for again. The
    /// same rule and the same 409 as changing the audio output device, for the same shape of reason:
    /// the request is well formed and the machine is not in a state where it means anything.
    ///
    /// **This is the only thing that ever moves an installed package**, which is what makes
    /// [`Machine::ensure_bank`] safe to run at every start: automatic banking never moves a package
    /// that already has a bank, so a printed book and a phone's favorites go stale only when
    /// somebody deliberately asks for it.
    ///
    /// The assignment is written to settings **first**, so that it survives a restart even if the
    /// catalog write fails, and so that a package which is not installed yet is banked where it
    /// was asked to be when it next goes in.
    fn set_package_bank(&self, package_id: &str, bank: u16) -> Result<usize, CatalogError> {
        if !self.output_change_allowed() {
            return Err(CatalogError::Unavailable(
                "a package's bank cannot change while a song is playing or queued: every song in \
                 it would be renumbered under the queue"
                    .into(),
            ));
        }
        if let Some(owner) = self
            .lock_library()
            .package_holding(bank)
            .map_err(library_failed)?
            && owner != package_id
        {
            return Err(CatalogError::Rejected(format!(
                "bank {bank} already belongs to the package {owner}"
            )));
        }

        self.lock_settings()
            .package_banks
            .insert(package_id.to_owned(), bank);
        self.save_settings();

        // A package that is not installed has nothing to re-key, and that is a success rather than a
        // 404: the assignment is recorded and takes effect when it next installs.
        let changed = self
            .lock_library()
            .set_package_bank(package_id, bank)
            .map_err(library_failed)?;

        // The archive's manifest is unchanged, but a cached one is keyed by package id and its songs
        // now answer to different numbers; drop it rather than reason about what is still valid.
        self.lock_packages().remove(package_id);
        Ok(changed)
    }

    fn rescan(&self) -> Result<RescanReport, CatalogError> {
        // Infallible in itself: a folder that cannot be read is a warning and an empty scan, not a
        // failed request, exactly as it is at startup. The `Result` is the trait's, for a double
        // that wants to refuse.
        Ok(self.rescan_now())
    }

    fn install_copied(&self, path: &Path) -> Result<InstallReport, CatalogError> {
        // **One copy policy, three callers.** `dropped::adopt` opens the package for its name and
        // its id before anything is copied, then applies the naming rules — a name the manifest
        // decides, `.part` then rename, an older file of the same package swept away — and puts it
        // in the write folder. A dropped file reaches it directly, a double-clicked `.kmpkg` reaches
        // it from `cli` before there is a machine at all, and this is how the API route reaches it
        // from another crate.
        let (destination, _id) = crate::dropped::adopt(&self.paths, &self.debug_packages(), path)
            .map_err(CatalogError::Rejected)?;
        self.install(&destination)
    }

    fn install(&self, path: &Path) -> Result<InstallReport, CatalogError> {
        // No `could not open {path}:` in front of it: every `PackageError` variant already names the
        // file, so a prefix here printed the path twice. That was not merely untidy — the idle
        // screen gives a notice two lines, `wrap` cannot break a Windows path, and the second copy
        // pushed the actual reason off the end. What reached the television was a sentence ending in
        // a bare colon, which reads as no reason at all.
        let package =
            Package::open(path).map_err(|error| CatalogError::Rejected(error.to_string()))?;
        let now = timestamp();
        // Where it goes, decided before the install because the install writes the bank into every
        // song's number. Taken outside the library lock — it takes that lock itself, and the rule
        // here is never to hold two.
        //
        // `wanted_bank()` and not `meta.bank`: a package that names no bank asks for the one its id
        // implies, which is the same answer `km-pack book` prints for the same file. Reading the
        // field raw here is what made the two disagree.
        let meta = &package.manifest().package;
        let bank = self
            .ensure_bank(&meta.id, meta.wanted_bank())
            .map_err(CatalogError::Rejected)?;
        let report = {
            let mut library = self.lock_library();
            library
                .install(&package, bank, &now)
                .map_err(|error| match error {
                    // A bank another package holds is the caller's problem to resolve rather than a
                    // server fault — and it is now the *only* clash an install can have, since two
                    // packages in different thousands cannot claim one number.
                    taken @ km_catalog::LibraryError::BankTaken { .. } => {
                        CatalogError::Rejected(taken.to_string())
                    }
                    other => CatalogError::Failed(other.to_string()),
                })?
        };

        // A reinstall replaces the archive under the same id, so the cached manifest is stale.
        self.lock_packages().remove(&report.package_id);

        // A package that had been refused and has now gone in must stop being complained about,
        // whichever route installed it. Cleared here rather than only in `install_and_report` so
        // that fixing the collision and installing through the API takes the notice off the screen.
        self.forget_package_problem(path);

        // **Nothing is written down, and that is the change.** Installing used to append the path
        // to `settings.packages` so the package would come back after a restart; the folders say
        // that now, and a scan at every pass is what brings it back. So there is no list to keep,
        // no temporary path to decline to write, and no ignore entry to clear — an install that
        // touches no settings at all cannot leave one disagreeing with the other.
        Ok(report)
    }

    fn why_not_removable(&self, package: &InstalledPackage) -> Option<String> {
        not_mine_to_delete(
            &self.paths,
            &self.lock_settings().debug.packages,
            Path::new(&package.path),
        )
    }

    fn package_bytes(&self, package: &InstalledPackage) -> Option<u64> {
        std::fs::metadata(&package.path).ok().map(|meta| meta.len())
    }

    fn why_problem_not_removable(&self, problem: &PackageProblem) -> Option<String> {
        // The same predicate `why_not_removable` asks one method above, and deliberately not a
        // second rule that happens to agree: a refused package sits in the very folders an
        // installed one does, and `debug.packages` is exactly where a package being worked on and
        // therefore failing is most likely to be.
        not_mine_to_delete(
            &self.paths,
            &self.lock_settings().debug.packages,
            Path::new(&problem.path),
        )
    }

    fn problem_bytes(&self, problem: &PackageProblem) -> Option<u64> {
        std::fs::metadata(&problem.path).ok().map(|meta| meta.len())
    }

    fn delete_problem_file(&self, id: &str) -> Result<(), CatalogError> {
        // Resolved out of the list as it is now, never joined onto a folder — `PackageProblem::id`
        // says why, and it is the rule `delete_wallpaper` and `delete_soundfont` already follow.
        // Cloned out so the lock is not held across the filesystem work below.
        let problem = resolve_problem(&self.lock_problems(), id)?;
        let path = PathBuf::from(&problem.path);

        // **The refusal, and not the resolution, is what keeps this safe.** Resolving against the
        // machine's own list already stops an invented id reaching the filesystem — but the list
        // includes `debug.packages` entries, which name files anywhere on the disk. So the order
        // matters: found first, then asked whether it is ours at all.
        if let Some(reason) =
            not_mine_to_delete(&self.paths, &self.lock_settings().debug.packages, &path)
        {
            return Err(CatalogError::Rejected(reason));
        }

        // **Simpler than `uninstall` in exactly one way.** That one
        // has to delete the file before dropping the rows, because the folders are the truth and
        // rows left behind a live file come straight back on the next pass. There are no rows here
        // at all — the package never opened far enough to have any — so this is a file and a note,
        // and nothing can be left disagreeing about them.
        match std::fs::remove_file(&path) {
            // At `warn` for `uninstall`'s reason: it is the only account anywhere of a file the
            // machine destroyed.
            Ok(()) => tracing::warn!(
                path = %path.display(),
                "deleted a refused package's file at the owner's request"
            ),
            // Somebody's file manager got there first, which is the outcome this was asked for.
            // Dropping the note is all that is left to do.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => tracing::info!(
                path = %path.display(),
                "the refused package's file was already gone; forgetting it"
            ),
            Err(error) => {
                return Err(CatalogError::Failed(format!(
                    "\"{}\" could not be removed, so nothing was deleted: {error}",
                    path.display()
                )));
            }
        }

        self.forget_package_problem(&path);

        // **A refusal can have left a bank reserved, and nothing else would ever release it.**
        // `install` calls `ensure_bank` *before* `library.install`, deliberately, so that the answer
        // survives an install that then fails — which means a package that opened and failed later
        // holds a thousand numbers it has no rows in. `package_id` is `Some` exactly when the
        // package opened, so it is precisely the case that can have a reservation.
        //
        // The argument is `uninstall`'s own, word for word: deleting the file is the owner saying it
        // is not coming back. This is the second exception to *a package that merely stopped being
        // found keeps its reservation*, and the rule is intact — that one is about a file that is
        // still somewhere, and this one is about a file this machine has just destroyed.
        if let Some(package_id) = &problem.package_id
            && self
                .lock_settings()
                .package_banks
                .remove(package_id)
                .is_some()
        {
            self.save_settings();
        }
        Ok(())
    }

    fn uninstall(&self, package_id: &str) -> Result<usize, CatalogError> {
        // The archive's path is read before the rows go, since that is where it is recorded.
        let path = self.lock_library().packages().ok().and_then(|packages| {
            packages
                .into_iter()
                .find(|package| package.id == package_id)
                .map(|package| PathBuf::from(package.path))
        });

        let Some(path) = path else {
            return Err(CatalogError::NotFound(format!("package '{package_id}'")));
        };

        // Both refusals live in [`not_mine_to_delete`], which is also what
        // `Catalog::why_not_removable` answers with — so what a page is *told* and what this route
        // *does* cannot drift apart, and a page that omits its Remove button omits it for the very
        // sentence this would have failed with.
        if let Some(reason) =
            not_mine_to_delete(&self.paths, &self.lock_settings().debug.packages, &path)
        {
            return Err(CatalogError::Rejected(reason));
        }

        // **The file goes first, and a failure here fails the whole operation.** The catalog is
        // reconciled against the folders at every pass, so dropping the rows while the file stayed
        // would mean the next pass putting the package straight back — a loop whose cause the owner
        // cannot see from the sofa. Refusing leaves the machine exactly as it was and says what to
        // do about it.
        //
        // This is the alternative `docs/architecture/persistence.md` considered and rejected, in as
        // many words: "an API call that destroys the owner's only copy of a package". It is the
        // decision now, and what was traded away for it is the ignore list — which had no job left
        // once the folders became the truth. What the sharper behavior obliges instead is that
        // `packages.uninstall` is admin by default, and that this is said at `warn`: it is the only
        // account anywhere of a file the machine destroyed.
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::warn!(
                package = package_id,
                path = %path.display(),
                "deleted a package's file: uninstalling is what does that now"
            ),
            // Somebody's file manager got there first. Nothing to undo, and the rows are all that
            // is left to remove.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => tracing::info!(
                path = %path.display(),
                "the package's file was already gone; removing its rows"
            ),
            Err(error) => {
                return Err(CatalogError::Failed(format!(
                    "\"{}\" could not be removed, so nothing was uninstalled: {error}",
                    path.display()
                )));
            }
        }

        let removed = {
            let mut library = self.lock_library();
            library.uninstall(package_id).map_err(library_failed)?
        };
        if removed == 0 {
            return Err(CatalogError::NotFound(format!("package '{package_id}'")));
        }
        self.lock_packages().remove(package_id);
        self.forget_package_problem(&path);

        // An explicit uninstall is the owner saying it is not coming back, so the bank goes with it.
        // A package that merely stopped being found keeps its reservation — see
        // [`Library::retain_packages`] — because that is what lets it come back with the same song
        // numbers rather than whatever is free next.
        if self
            .lock_settings()
            .package_banks
            .remove(package_id)
            .is_some()
        {
            self.save_settings();
        }
        Ok(removed)
    }

    fn song_count(&self) -> Result<usize, CatalogError> {
        self.lock_library().song_count().map_err(library_failed)
    }

    fn export(
        &self,
        after: Option<SongCode>,
        limit: usize,
    ) -> Result<Vec<CatalogSong>, CatalogError> {
        self.lock_library()
            .export_after(after, limit)
            .map_err(library_failed)
    }

    fn catalog_version(&self) -> Result<u64, CatalogError> {
        self.lock_library()
            .catalog_version()
            .map_err(library_failed)
    }

    fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<(String, usize)>, CatalogError> {
        self.lock_library()
            .artists(contains, hidden)
            .map_err(library_failed)
    }

    fn languages(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, CatalogError> {
        self.lock_library()
            .languages(hidden)
            .map_err(library_failed)
    }

    fn tags(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, CatalogError> {
        self.lock_library().tags(hidden).map_err(library_failed)
    }
}

impl Machine {
    /// How much audio one device callback covers, in milliseconds; 0 before any stream has run.
    ///
    /// The PIN this machine generated for itself, while it is still the password in force.
    ///
    /// **Deliberately not on `Snapshot` and not in `/discover`**, for [`Self::period_ms`]'s reason
    /// read the other way: that one is local because a remote has no use for it, and this one is
    /// local because a remote must not have it. What crosses the network is the single bit *that*
    /// the machine is on a factory password; the PIN itself is drawn on the television, where
    /// reading it means being in the room.
    pub fn factory_pin(&self) -> Option<String> {
        self.lock_settings().api.admin_factory_pin.clone()
    }

    /// Deliberately **not** on [`Snapshot`]: that type crosses the API, and how coarsely this machine
    /// happens to sample its own clock is a local drawing concern rather than something a remote has
    /// any use for. The display is the only caller.
    pub fn period_ms(&self) -> u32 {
        self.engine.period_ms()
    }

    /// How long the loaded song has been silent for want of samples; 0 on a healthy song.
    ///
    /// Off [`Snapshot`] for the same reason as `period_ms` above, and one more: a remote showing
    /// this would be showing a fault to the wrong person. The singer cannot act on it and the owner
    /// reads it in the log.
    pub fn starved_ms(&self) -> u32 {
        self.engine.starved_ms()
    }

    /// Recoverable stream errors since the stream opened; off [`Snapshot`] for the same reasons.
    ///
    /// This is the only number here that says anything about a **MIDI** song's timing, since one has
    /// no decoder feed and therefore no starvation to report.
    pub fn xruns(&self) -> u32 {
        self.engine.xruns()
    }

    /// Where uploads are staged. Resolved by rule from the data directory, never named by a setting.
    fn audition_root(&self) -> PathBuf {
        self.paths.data_dir.join(crate::settings::AUDITION_SUBDIR)
    }

    /// The staging folders a sweep may not take: what is playing, and what is still arriving.
    fn audition_keep(&self) -> Vec<PathBuf> {
        let root = self.audition_root();
        let state = self.lock_state();
        audition_in_use(state.loaded.as_ref().map(|loaded| &loaded.origin), &root)
            .into_iter()
            .chain(state.audition_staging.clone())
            .collect()
    }

    /// Removes every staged audition that is no longer wanted, and arms a retry if any refused.
    ///
    /// **This is the whole feature: an audition goes when it stops being the song that is playing,
    /// not when the next upload arrives.** It can never be played again — nothing catalogs it and
    /// `play_audition` only ever resolves the folder of the upload in hand — so the moment it is
    /// displaced it is a gigabyte of scratch nobody will ever read. On a television that is an
    /// app-private folder with no other reclamation at all.
    ///
    /// `budget` is how many further looks this is allowed to ask for; a displacement starts a fresh
    /// [`AUDITION_TRIES`] and [`Self::settle_auditions`] passes on what is left.
    fn reclaim_auditions(&self, budget: u8) {
        let stuck = sweep_auditions(&self.audition_root(), &self.audition_keep());
        let next = next_sweep(budget, stuck, Instant::now());
        self.lock_state().audition_sweep = next;
        if stuck > 0 && next.is_none() {
            // Not an error: the two backstops still have it. Said once rather than every quarter
            // second, which is why it is here and not in the sweep.
            tracing::warn!(
                folders = stuck,
                "a staged audition would not delete; leaving it to the next upload or restart"
            );
        }
    }

    /// Looks again at a staged audition that would not delete.
    ///
    /// Called from [`Self::poll`], so it runs twenty times a second and is nearly always one lock and
    /// one comparison — the same bargain [`Self::maybe_start_demo`] makes. The disk is touched only
    /// on the tick a retry actually falls due, which on a machine that never auditions is never.
    fn settle_auditions(&self) {
        let now = Instant::now();
        let Some(sweep) = self.lock_state().audition_sweep else {
            return;
        };
        if sweep.due > now {
            return;
        }
        self.reclaim_auditions(sweep.tries);
    }

    /// Removes everything staged for an audition. For shutdown, beside `persist`.
    ///
    /// Best-effort by design. Unlinking an open file succeeds on Linux and Android, so a stop
    /// reclaims the space even mid-song; on Windows the file being played refuses and the purge in
    /// [`Machine::new`] gets it at the next start. Nothing reaches here after a `SIGKILL`, a power cut
    /// or a low-memory kill, which is why that purge stays the real backstop.
    pub fn clear_auditions(&self) {
        purge_auditions(&self.audition_root());
    }
}

impl Controller for Machine {
    fn snapshot(&self) -> Snapshot {
        let state = self.lock_state();
        Snapshot {
            transport: self.engine.transport(),
            now_playing: state.loaded.as_ref().map(Loaded::describe),
            position_ms: self.engine.position_ms(),
            queue_len: state.queue.len(),
            settings: state.settings,
        }
    }

    fn queue(&self) -> Vec<QueueEntry> {
        self.lock_state().queue.entries().cloned().collect()
    }

    fn queue_add(&self, request: QueueRequest) -> Result<u64, ControlError> {
        let id = self
            .lock_state()
            .queue
            .add(request)
            .map_err(|QueueFull::Full| ControlError::QueueFull)?;
        // Somebody is here, so the next demo waits the full delay. It also spends a hand-pressed
        // one-shot that has not fired yet, so a *play something* press followed by a queue starts
        // one song rather than two.
        self.arm_demo(DemoEvent::Somebody);
        // Somebody has chosen a song, so sound is coming. The device is not held while the machine
        // is idle, and bringing a Bluetooth link back up takes about a second — this starts that
        // now, so it overlaps with whatever else has to happen before the first note.
        self.engine.send(Command::Wake);
        // Queue a song on an idle machine and it should start, the way pressing a number on a real
        // unit does — **and a machine singing to itself counts as idle.** Somebody who queued while
        // a *person* was singing gets it in turn.
        //
        // Both halves of that test live inside `advance_if_idle_or_over_a_demo`, under the same
        // lock as the pop they guard. Spelled here they are two separate acts, and eight singers
        // queueing at once then leave one song. `can_play` stays out here because it is a standing
        // fact about the machine rather than a race — a box with no sound card refuses every time.
        if self.engine.can_play() && self.advance_if_idle_or_over_a_demo() {
            // A demo ended, for a reason no other `SongEnded` carries: not finished, not skipped,
            // not stopped, but stood aside from. Published from the answer rather than from a guess
            // made before the lock, because only the locked section can see the demo.
            self.events.publish(Event::SongEnded {
                reason: EndReason::Yielded,
            });
        }
        Ok(id)
    }

    fn queue_remove(&self, id: u64) -> Result<QueueEntry, ControlError> {
        self.arm_demo(DemoEvent::Somebody);
        self.lock_state()
            .queue
            .remove(id)
            .ok_or_else(|| ControlError::NotFound(format!("queue entry {id}")))
    }

    fn queue_move(&self, id: u64, to_index: usize) -> Result<(), ControlError> {
        self.arm_demo(DemoEvent::Somebody);
        if self.lock_state().queue.move_to(id, to_index) {
            Ok(())
        } else {
            Err(ControlError::NotFound(format!("queue entry {id}")))
        }
    }

    fn queue_clear(&self) -> Result<usize, ControlError> {
        self.arm_demo(DemoEvent::Somebody);
        // `Queue::clear` rather than popping until empty, which is what this did while that method
        // sat unused beside it. Two spellings of one operation, and the queue's own is the one that
        // stays right if the container underneath ever changes.
        Ok(self.lock_state().queue.clear().len())
    }

    fn transport(&self, command: TransportCommand) -> Result<(), ControlError> {
        if !self.engine.can_play() {
            return Err(ControlError::Unavailable(Refusal::coded(
                NO_SOUND,
                self.engine.sound().describe(),
            )));
        }
        // Read before `advance` takes it, the way `poll` reads it: the origin is what decides
        // whether the next demo waits, and a statement later it is gone.
        let over_a_demo = matches!(
            self.lock_state()
                .loaded
                .as_ref()
                .map(|loaded| &loaded.origin),
            Some(Origin::Demo { .. })
        );
        // Every command, including the ones that refuse below. Somebody pressing stop on a demo song
        // and getting another one two seconds later would be the machine ignoring them, and pausing
        // is not consent to be played at either.
        //
        // **Skip on a demo is the one command that is the demo ending rather than a person here.**
        // Whoever pressed it asked for a different song, not for quiet, so the next one follows with
        // no gap — exactly as it would have when this one ran out.
        self.arm_demo(
            if over_a_demo && matches!(command, TransportCommand::Skip) {
                DemoEvent::DemoEnded
            } else {
                DemoEvent::Somebody
            },
        );
        let loaded = self.lock_state().loaded.is_some();
        match command {
            TransportCommand::Play => {
                if loaded {
                    self.engine.send(Command::Play);
                } else if self.lock_state().queue.is_empty() {
                    return Err(ControlError::Unavailable(Refusal::coded(
                        NOTHING_QUEUED,
                        "nothing is loaded and nothing is queued",
                    )));
                } else {
                    // Same reason as in `queue_add`: get the device coming up before the song is
                    // parsed off disk rather than after it.
                    self.engine.send(Command::Wake);
                    // ...and `advance_if_idle` for `queue_add`'s other reason. `loaded` above was
                    // read and released several statements ago, so by here it is a claim about the
                    // past; the function re-tests it under the lock that owns the answer.
                    self.advance_if_idle();
                }
            }
            TransportCommand::Pause => {
                if !loaded {
                    return Err(ControlError::Unavailable(Refusal::coded(
                        NOTHING_PLAYING,
                        "nothing is playing",
                    )));
                }
                self.engine.send(Command::Pause);
            }
            TransportCommand::Restart => {
                if !loaded {
                    return Err(ControlError::Unavailable(Refusal::coded(
                        NOTHING_LOADED,
                        "nothing is loaded",
                    )));
                }
                self.lock_state().announced_line = None;
                self.engine.send(Command::Restart);
            }
            TransportCommand::Skip => {
                if !loaded {
                    // **Skip answers silence with a song while demo mode is on.** Skip asks for the
                    // next thing, and on a machine that is choosing songs for itself the next thing
                    // is the machine's to pick — so the press is answered by getting on with it
                    // rather than by a sentence about an empty deck. With the mode off nothing is
                    // choosing anything, and the refusal below is the whole answer.
                    //
                    // **The running switch rather than `settings.demo.enabled`**, which is what a
                    // restart would find: this press asks about the machine in the room.
                    //
                    // **A one-shot rather than the deadline, which is why the `arm_demo` above
                    // stands.** That call armed `Somebody` — `over_a_demo` is read from `loaded`, so
                    // a skip into silence cannot be a demo ending — and it cleared any trigger
                    // already pending, so the one set here is the only one standing. `demo_is_due`
                    // lets `once` past the clock, which leaves the silence a person bought a
                    // statement ago theirs if no song can start. `DemoEnded` would say a demo ended
                    // where none did, and would park the deadline in the past: a press refused off
                    // the screen would then fire a song the instant the activity came forward.
                    //
                    // **`start_demo_song` keeps the four conditions rather than a copy of them.**
                    // Two of the four can refuse from here, and they are the two that must: a queue
                    // plays first, and a machine off the screen stays quiet. Both mean *not yet*,
                    // and with nothing on the deck "nothing is playing" is true and is the sentence
                    // every client already renders — so a skip carries one refusal code whichever
                    // way it goes. Whoever wants the demo's own reason asks for the demo:
                    // `POST /api/v1/demo/start` is public and gives it.
                    let demo_enabled = self.lock_state().demo_enabled;
                    if demo_enabled {
                        match self.start_demo_song() {
                            Ok(_) => return Ok(()),
                            Err(error) => {
                                tracing::debug!(%error, "a skip into silence found no demo to start")
                            }
                        }
                    }
                    return Err(ControlError::Unavailable(Refusal::coded(
                        NOTHING_PLAYING,
                        "nothing is playing",
                    )));
                }
                self.events.publish(Event::SongEnded {
                    reason: EndReason::Skipped,
                });
                self.advance();
            }
            TransportCommand::Stop => {
                self.engine.send(Command::Stop);
                if loaded {
                    self.events.publish(Event::SongEnded {
                        reason: EndReason::Stopped,
                    });
                }
                self.go_idle();
            }
            TransportCommand::Seek { ms } => {
                if !loaded {
                    return Err(ControlError::Unavailable(Refusal::coded(
                        NOTHING_LOADED,
                        "nothing is loaded",
                    )));
                }
                self.lock_state().announced_line = None;
                self.engine.send(Command::SeekMs(ms));
            }
        }
        Ok(())
    }

    fn update_settings(&self, patch: &SettingsPatch) -> Result<ApiSettings, ControlError> {
        let settings = self.apply_settings(patch)?;
        // Mirror the live values into the persisted defaults, so a key the owner likes survives a
        // restart. The per-song reset in `start` still applies within a session.
        {
            let mut stored = self.lock_settings();
            stored.playback.transpose = settings.transpose;
            stored.playback.tempo_ratio = settings.tempo_ratio;
            stored.playback.melody_enabled = settings.melody_enabled;
            stored.audio.music_volume = settings.music_volume;
            stored.display.lyric_offset_ms = settings.lyric_offset_ms;
        }
        Ok(settings)
    }

    fn mics(&self) -> Vec<MicChannel> {
        self.lock_state().mics.channels()
    }

    fn update_mic(&self, id: &str, patch: &MicPatch) -> Result<MicChannel, ControlError> {
        self.lock_state()
            .mics
            .apply(id, patch)
            // The only way `apply` fails is an id nothing matches, and now that `NotFound` carries
            // its subject the discarded error is a `MicError` with nothing else in it to lose.
            .map_err(|_| ControlError::NotFound(format!("microphone '{id}'")))
    }

    fn audio_outputs(&self) -> Result<AudioOutputs, ControlError> {
        self.describe_audio_outputs()
    }

    fn soundfont(&self) -> SoundFontStatus {
        self.describe_soundfont()
    }

    fn soundfonts(&self, all: bool) -> km_api::machine::SoundFontBanks {
        let (banks, selected) = self.soundfont_banks();
        let (offers, fetching) = self.soundfont_offers(all);
        km_api::machine::SoundFontBanks {
            banks: banks
                .into_iter()
                .map(|bank| km_api::machine::SoundFontBank {
                    // Asked per row, and cheap: the bundled test is a field, and the slot test only
                    // walks `debug.soundfonts`, which is empty on every machine but a developer's.
                    why_not_removable: self.bank_not_mine_to_delete(&bank),
                    id: bank.id,
                    name: bank.name,
                    bytes: bank.bytes,
                    bundled: bank.bundled,
                })
                .collect(),
            selected,
            offers,
            fetching,
        }
    }

    fn fetch_soundfont(&self, id: &str) -> Result<(), ControlError> {
        Machine::fetch_soundfont(self, id)
    }

    fn set_soundfont(&self, id: &str) -> Result<(), ControlError> {
        Machine::select_soundfont(self, id)
    }

    fn delete_soundfont(&self, id: &str) -> Result<(), ControlError> {
        Machine::delete_soundfont(self, id)
    }

    fn set_audio_output(&self, id: &str) -> Result<AudioOutputs, ControlError> {
        if !self.output_change_allowed() {
            // The player lives inside the audio stream a change has to drop, so this is a refusal
            // rather than a delay. See `Held::close` in engine.rs.
            return Err(ControlError::Unavailable(OUTPUT_DEVICE_BUSY.into()));
        }

        if id != km_audio::SYSTEM_DEFAULT {
            let known = self
                .engine
                .output_devices()
                .map_err(|error| ControlError::Failed(error.to_string()))?
                .into_iter()
                .any(|device| device.id == id && device.available);
            if !known {
                return Err(ControlError::NotFound(format!("audio output '{id}'")));
            }
        }

        // Stored as the sentinel rather than as an absent key when the system default is asked for:
        // absent means "never chosen", which would let the USB preference override the choice on the
        // next start. See `AudioSettings::output_device`.
        self.lock_settings().audio.output_device = Some(id.to_owned());
        // Written now rather than at shutdown. This is installation configuration — somebody is
        // standing at the machine getting the sound to come out of the right socket, and a power cut
        // before the next clean stop must not undo it. Same reasoning as a package install.
        self.save_settings();

        // The identifier verbatim, sentinel included, and never `None`. `None` on that call means
        // "nothing has ever been chosen", which is what makes the USB preference fire — passing it
        // here to mean "follow the system" would collapse two different states into one and report
        // a deliberate choice as an absent one.
        self.engine.set_output_device(Some(id.to_owned()));
        tracing::info!(device = id, "the audio output device was changed");
        self.describe_audio_outputs()
    }

    fn output_level_supported(&self) -> bool {
        km_audio::level::supported()
    }

    fn set_output_level(&self, db_centi: i32) -> Result<AudioOutputs, ControlError> {
        // The device in use rather than the one settings name, which is the one a level belongs to:
        // after a fallback those differ, and moving the level of an absent card would change
        // nothing anybody can hear. `level_device` resolves "follow the system" to whatever that is
        // today, for the reason given there.
        let status = self.engine.output_status();
        let devices = self
            .engine
            .output_devices()
            .map_err(|error| ControlError::Failed(error.to_string()))?;
        let active = Self::level_device(&status.active.id, &devices).to_owned();
        let db = db_centi as f32 / 100.0;
        match km_audio::level::set(&active, db) {
            // Nothing to move. An HDMI output hands the volume to the receiver, so this is a
            // sentence somebody reads rather than a fault in what they sent.
            Ok(None) => Err(ControlError::Unavailable(km_api::machine::Refusal::coded(
                km_api::machine::NO_OUTPUT_LEVEL,
                "this output has no level to set",
            ))),
            Ok(Some(level)) => {
                // Where it landed rather than what was asked for: a control with coarse steps puts
                // a request between two of them on one of the two.
                tracing::info!(device = %active, db = level.db, "the output level was changed");
                self.describe_audio_outputs()
            }
            Err(error) => Err(ControlError::Failed(error.to_string())),
        }
    }

    fn demo(&self) -> km_api::machine::DemoState {
        let (delay_secs, min_suitability, stored) = {
            let settings = self.lock_settings();
            (
                settings.demo.delay_secs,
                settings.demo.min_suitability,
                settings.demo.enabled,
            )
        };
        let state = self.lock_state();
        let now = Instant::now();
        km_api::machine::DemoState {
            enabled: state.demo_enabled,
            stored,
            delay_secs,
            min_suitability,
            playing: matches!(
                state.loaded.as_ref().map(|loaded| &loaded.origin),
                Some(Origin::Demo { .. })
            ),
            starts_in_secs: demo_starts_in(
                state.demo_enabled,
                state.loaded.is_some(),
                state.queue.is_empty(),
                state.demo_resume_at,
                now,
                self.foreground.load(Ordering::Acquire),
            ),
        }
    }

    /// Switches demo mode, and writes it down when asked.
    ///
    /// **The deadline is deliberately left where it is**, which is what makes switching demo mode on
    /// from a phone do something visible: the clock counts *idleness*, not time since the switch, so
    /// a machine that has been quiet for ten minutes is already past its deadline and starts a song
    /// on the next poll. Arming a fresh delay here would mean somebody flips the switch, nothing
    /// happens for a minute, and they conclude it is broken.
    ///
    /// Switching it *off* stops the next song and leaves the current one playing. That is the same
    /// bargain every other control here makes — this is a mode, not a transport command, and
    /// somebody who wants the music to stop now has `POST /transport/stop` for exactly that.
    fn set_demo(
        &self,
        enabled: bool,
        persist: bool,
    ) -> Result<km_api::machine::DemoState, ControlError> {
        self.lock_state().demo_enabled = enabled;
        if persist {
            // Straight to disk rather than waiting for `persist()` on shutdown, for the reason
            // `set_accept_uploads` gives: this is somebody standing in front of the machine saying
            // what it should do when they are not, and a power cut must not undo it.
            self.lock_settings().demo.enabled = enabled;
            self.save_settings();
        }
        tracing::info!(enabled, persist, "demo mode was changed");
        Ok(self.demo())
    }

    /// Sets the delay, writes it down, and moves the deadline that was already running.
    ///
    /// **Written to disk immediately and with no `persist` to ask about**, which is the difference
    /// between this and its neighbour: a delay is installation configuration, in the same family as
    /// the machine's name, where the switch is *tonight or for good*.
    ///
    /// **The armed deadline is shifted by the difference rather than re-armed from now**, and that
    /// is [`Self::set_demo`]'s rule seen from the other side. Both say the clock counts *idleness*.
    /// The deadline is `the last thing somebody did + delay`, so changing the delay moves it by
    /// exactly the change and leaves the idle moment where it was: quiet for fifty seconds and told
    /// to wait sixty leaves ten to go, and told to wait thirty means the next poll starts a song.
    /// Re-arming from now would make *shortening* the delay lengthen the wait, once, in front of
    /// whoever had just shortened it to see what would happen.
    fn set_demo_delay(&self, delay_secs: u32) -> Result<km_api::machine::DemoState, ControlError> {
        if delay_secs > km_api::machine::MAX_DEMO_DELAY_SECS {
            return Err(ControlError::Rejected(format!(
                "a demo delay of {delay_secs} seconds is longer than the {} this accepts",
                km_api::machine::MAX_DEMO_DELAY_SECS
            )));
        }
        let was = {
            let mut settings = self.lock_settings();
            let was = settings.demo.delay_secs;
            settings.demo.delay_secs = delay_secs;
            was
        };
        // Straight to disk, for `set_demo`'s reason: this is somebody saying what the machine should
        // do when they are not in front of it.
        self.save_settings();
        {
            let mut state = self.lock_state();
            state.demo_resume_at = demo_deadline_moved(state.demo_resume_at, was, delay_secs);
        }
        tracing::info!(delay_secs, was, "the demo delay was changed");
        Ok(self.demo())
    }

    /// Asks for one demo song now.
    ///
    /// **A flag rather than a song, and the reason is exclusivity rather than the cost of the
    /// work.** Every demo start happens on the one poll thread, which is what makes
    /// [`Self::maybe_start_demo`]'s check-then-start safe with no lock spanning the load. Starting a
    /// song from a request thread would race that check, and two starts means the second
    /// `Command::Load` cuts the first song off a few milliseconds after it began. Setting a flag and
    /// letting the next poll act on it keeps one writer, and costs the fifty milliseconds nobody can
    /// hear.
    ///
    /// The four refusals are [`why_no_demo`]'s, and what it cannot refuse is described there.
    fn start_demo_song(&self) -> Result<km_api::machine::DemoState, ControlError> {
        // Asked outside the state lock, because it takes the engine's. Last, as in
        // `maybe_start_demo` — the two are the same rule and should read as it.
        let can_play = self.engine.can_play();
        let foreground = self.foreground.load(Ordering::Acquire);
        {
            let state = self.lock_state();
            if let Some(why) = why_no_demo(
                state.loaded.is_some(),
                state.queue.is_empty(),
                can_play,
                foreground,
            ) {
                return Err(ControlError::Unavailable(why.into()));
            }
        }
        self.lock_state().demo_once = true;
        tracing::info!("a demo song was asked for by hand");
        Ok(self.demo())
    }

    fn set_session_epoch(&self, epoch: u64) -> Result<(), ControlError> {
        // Straight to disk rather than waiting for `persist()` on shutdown. An epoch is the one
        // setting whose whole purpose is to hold after the machine is switched off and on: a hard
        // power cut is exactly the event that would otherwise un-revoke every session an owner had
        // just ended, and they would have no way of knowing.
        self.lock_settings().api.session_epoch = epoch;
        self.save_settings();
        tracing::info!(
            epoch,
            "every admin session was ended, and the epoch written down"
        );
        Ok(())
    }

    fn set_debug_enabled(&self, enabled: bool) -> Result<(), ControlError> {
        // Straight to disk for the same reason, and one more: this one opens a route that plays any
        // file the machine can read, so a machine that forgot it had been turned *off* would be
        // worse than one that forgot it had been turned on.
        self.lock_settings().debug.enabled = Some(enabled);
        self.save_settings();
        tracing::info!(
            enabled,
            "debugging mode was written to settings; the debug routes follow it at the next start"
        );
        Ok(())
    }

    fn set_dev_remote_enabled(&self, enabled: bool) -> Result<(), ControlError> {
        // Straight to disk for `set_debug_enabled`'s reason and the sharper form of it: with
        // debugging on as well, this mounts the whole API again with no password on any of it, so a
        // machine that came back having forgotten it was turned *off* would be the worst of the
        // outcomes a lost write can produce here.
        self.lock_settings().api.serve_dev_remote = Some(enabled);
        self.save_settings();
        tracing::info!(
            enabled,
            "the development console was written to settings; it follows at the next start, and \
             only while debugging mode is on too"
        );
        Ok(())
    }

    fn performance_overlay(&self) -> bool {
        Machine::performance_overlay_on(self)
    }

    fn set_performance_overlay(&self, on: bool) -> Result<(), ControlError> {
        // Nothing to persist and nothing that can fail: one atomic the display loop reads at the top
        // of every frame. The `Result` is the trait's shape rather than this call's.
        Machine::set_performance_overlay(self, on);
        Ok(())
    }

    fn developer_switches(&self) -> km_api::machine::DeveloperSwitches {
        // Read off the settings rather than off the running `ApiConfig`, which is the whole point of
        // this call: both switches take effect at the next start, so what a page can usefully draw
        // is what is written down.
        let settings = self.lock_settings();
        km_api::machine::DeveloperSwitches {
            debug: settings.debug_enabled(),
            dev_remote: settings.serve_dev_remote(),
        }
    }

    fn set_machine_name(&self, name: &str) -> Result<(), ControlError> {
        // Straight to disk, for the reason `set_session_epoch` above gives: a deliberate, durable act,
        // and `persist()` on shutdown is exactly what a power cut does not run. A machine that came
        // back under its old name would look like a rename that silently failed.
        self.lock_settings().machine.name = name.to_owned();
        self.save_settings();
        tracing::info!(name, "the machine was renamed");
        Ok(())
    }

    fn machine_locale(&self) -> km_locale::Locale {
        self.locale()
    }

    fn set_machine_locale(&self, locale: km_locale::Locale) -> Result<(), ControlError> {
        // Straight to disk, for the reason the rename above gives.
        //
        // **Nothing has to be told.** The display reads the locale off `DisplayConfig` once per
        // frame, so the next one drawn is in the new language; there is no cached catalog to
        // invalidate, because a catalog is immutable and one exists per locale already.
        self.lock_settings().machine.locale = locale.tag().to_owned();
        self.save_settings();
        tracing::info!(%locale, "the machine changed what language it speaks");
        Ok(())
    }

    fn wallpapers(&self) -> WallpaperState {
        self.lock_state().wallpapers.clone()
    }

    fn next_wallpaper(&self) -> Result<(), ControlError> {
        self.lock_state().wallpaper_requested = true;
        Ok(())
    }

    fn wallpaper_pictures(&self) -> Vec<km_api::machine::Picture> {
        Machine::wallpaper_pictures(self)
    }

    fn delete_wallpaper(&self, id: &str) -> Result<(), ControlError> {
        Machine::delete_wallpaper(self, id)
    }

    fn set_admin_password(
        &self,
        hash: Option<String>,
        factory_pin: Option<String>,
    ) -> Result<(), ControlError> {
        // Straight to disk, for the reason `set_session_epoch` and `set_machine_name` give: a
        // deliberate, durable act, and `persist()` on shutdown is exactly what a power cut does not
        // run. A machine that came back on the old password after its owner had just changed it
        // would be open to somebody they had meant to shut out.
        //
        // **The PIN is written in the same breath, and that is what keeps the two honest.** They
        // answer different questions -- what the password is, and whose it is -- and a hash stored
        // without clearing a stale PIN would leave the machine telling everybody on its own screen
        // to try a code that no longer works.
        {
            let mut settings = self.lock_settings();
            settings.api.admin_password_hash = hash.clone();
            settings.api.admin_factory_pin = factory_pin.clone();
        }
        self.save_settings();
        tracing::info!(
            generated = factory_pin.is_some(),
            "the admin password was written to settings"
        );
        Ok(())
    }

    fn open_upload(&self) -> Result<PathBuf, ControlError> {
        // The audition folder's neighbor, under the same subdirectory, so one sweep at startup
        // clears both and a machine that was killed mid-upload leaves nothing behind for ever.
        let dir = self
            .paths
            .data_dir
            .join(crate::settings::AUDITION_SUBDIR)
            .join("upload");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|error| {
            ControlError::Unavailable(
                format!("could not make a folder for the upload: {error}").into(),
            )
        })?;
        Ok(dir)
    }

    fn accept_upload(&self, kind: Upload, staged: &Path) -> Result<String, ControlError> {
        match kind {
            // The one placement policy this crate has, and the reason it is reached from here rather
            // than reimplemented: the manifest names the file, the same id replaces it, `.part` and
            // rename. See `dropped::place`. It matters most on this route — what a client called the
            // file it sent decides nothing about where it lands.
            Upload::Package => {
                let (destination, _id) =
                    crate::dropped::adopt(&self.paths, &self.debug_packages(), staged)
                        .map_err(ControlError::Rejected)?;
                let report = self
                    .install(&destination)
                    .map_err(|error| ControlError::Unavailable(error.to_string().into()))?;
                // One spelling of the sentence, in `km-api`'s DTO — `dropped::install` is the other
                // caller and reaches the same conversion.
                Ok(km_api::dto::InstallReportDto::from(&report).sentence())
            }
            Upload::Wallpaper => {
                let dir = self.paths.wallpapers_dir();
                let total = place_wallpaper(&dir, staged)?;
                {
                    let mut state = self.lock_state();
                    // Both flags, and both are needed. The first makes the display re-resolve which
                    // folder it watches -- see `wallpaper_dir_stale` -- and the second makes it
                    // change picture now, so the upload is visible rather than merely present.
                    state.wallpaper_dir_stale = true;
                    state.wallpaper_requested = true;
                }
                let plural = if total == 1 { "picture" } else { "pictures" };
                Ok(format!("added \u{2014} {total} {plural} now showing"))
            }
            // Nothing to arrange: `soundfont::installed` reads the directory on every call, so a
            // bank is selectable the moment its file lands.
            Upload::SoundFont => {
                let dir = self.paths.soundfonts_dir();
                let name = move_into(&dir, staged)?;
                Ok(format!("added \"{name}\""))
            }
        }
    }

    fn play_file(&self, path: &Path, decided: &km_api::Audition<'_>) -> Result<(), ControlError> {
        // The allowed roots are settings, not this crate's business to guess: only the owner knows
        // where they keep their files, and an unrestricted version of this endpoint reads anything
        // on the machine's disk. Empty roots refuse everything, which is the shipped default.
        if !self.lock_settings().debug_path_allowed(path) {
            return Err(ControlError::Rejected(format!(
                "{} is not inside an allowed folder; set debug.play_file_roots to permit it",
                path.display()
            )));
        }
        self.play_path_with(path, decided)
    }

    fn open_audition(&self) -> Result<PathBuf, ControlError> {
        // The same sentence `play_file` above answers with, about the same kind of thing: an
        // endpoint that reaches the disk is off until the owner says otherwise, and the refusal
        // names the setting rather than leaving somebody to find it.
        if !self.lock_settings().debug_enabled() {
            return Err(ControlError::Rejected(
                "this machine is not in debugging mode; turn it on from the owner's page, or set \
                 debug.enabled in settings, to audition an uploaded song"
                    .to_owned(),
            ));
        }

        let root = self.audition_root();
        std::fs::create_dir_all(&root)
            .map_err(|error| ControlError::Failed(format!("{}: {error}", root.display())))?;
        // A backstop rather than the main reclamation, which happens when a song is displaced. This
        // is what catches whatever a killed run left behind — and what catches a folder that was
        // still open when its song ended and had gone quiet again by now.
        sweep_auditions(&root, &self.audition_keep());

        // Named by the clock rather than by a counter, so two runs of the machine cannot collide
        // and the sweep above can put them in age order by sorting the names.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        let dir = root.join(format!("{stamp:039}"));
        std::fs::create_dir(&dir)
            .map_err(|error| ControlError::Failed(format!("{}: {error}", dir.display())))?;
        // Remembered from here until `play_audition` takes it, because for as long as the upload is
        // arriving there is nothing on disk that distinguishes it from an abandoned folder — and a
        // song ending on the other side of a gigabyte upload would otherwise sweep it away.
        self.lock_state().audition_staging = Some(dir.clone());
        Ok(dir)
    }

    fn play_audition(
        &self,
        name: &str,
        decided: &km_api::Audition<'_>,
    ) -> Result<(), ControlError> {
        // **The whole containment rule, and it is one line because the argument is one sentence**:
        // a name that is its own last component has no separator in it, so joining it onto a folder
        // cannot leave that folder. Refused rather than sanitised — a caller that sent a path meant
        // something this cannot do, and quietly playing a different file is the worse answer.
        if Path::new(name).file_name() != Some(name.as_ref()) {
            return Err(ControlError::Rejected(format!(
                "{name} is not a bare file name"
            )));
        }
        let root = self.audition_root();
        // The folder the upload wrote into, which stops being protected the moment it starts
        // playing: from here on it is the loaded song that keeps it, and when that changes it goes.
        // Falling back to the newest keeps the original behavior for anything that never staged —
        // and the newest *is* the one just written, since the names are nanosecond stamps.
        let dir = self
            .lock_state()
            .audition_staging
            .take()
            .filter(|dir| dir.is_dir())
            .or_else(|| newest_audition(&root))
            .ok_or_else(|| {
                ControlError::Rejected("there is no staged audition to play".to_owned())
            })?;
        self.play_path_with(&dir.join(name), decided)
    }
}

/// The staging folders under `root`, newest last.
///
/// Sorted by name, which is an age order because [`Controller::open_audition`] names them by the
/// clock. Nothing here fails: a root that cannot be read is a root with nothing in it.
fn audition_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs
}

/// The most recently opened staging folder.
fn newest_audition(root: &Path) -> Option<PathBuf> {
    audition_dirs(root).pop()
}

/// Removes everything staged for an audition. For startup, where nothing can be open yet.
///
/// Best-effort, like [`sweep_auditions`] and for a weaker version of the same reason: this is
/// scratch, and a machine that cannot empty its scratch folder should still start.
fn purge_auditions(root: &Path) {
    if let Err(error) = std::fs::remove_dir_all(root)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(dir = %root.display(), %error, "could not clear the audition folder");
    }
}

/// The staging folder a loaded song is being read out of, when it is an audition.
///
/// Takes the origin rather than the [`Loaded`], so it can be checked without media to load. All
/// three audition media — MIDI, video and either half of an MP3+G pair — record [`Origin::File`]
/// with the full path, so one rule covers them.
///
/// The parent's own parent must **equal** `root` rather than start with it: a stray path further
/// down cannot then pin a folder it does not live in, and a file sitting loose in the root is not a
/// staging folder. Nothing is canonicalised, and that is deliberate — both sides are built from the
/// same `paths.data_dir`, so they agree whether or not it was given as a relative `--data-dir`, and
/// canonicalising would ask the disk a question that is already answered.
fn audition_in_use(origin: Option<&Origin>, root: &Path) -> Option<PathBuf> {
    let Some(Origin::File { path }) = origin else {
        return None;
    };
    let dir = Path::new(path).parent()?;
    (dir.parent() == Some(root)).then(|| dir.to_path_buf())
}

/// Removes every staged audition except the ones named, and says how many would not go.
///
/// **Every failure is tolerated, and every failure is counted.** A video is played by reading its
/// file as it goes and Windows will not delete an open file — so a folder that refuses is a folder
/// to look at again, which is what the count is for. A rule of *keep the newest* would need no
/// count and is the wrong rule: it keeps whatever happens to be playing, and keeps it long after
/// the song has finished.
///
/// `keep` is two things and both are needed. The folder the loaded song is reading from cannot go
/// while it plays, and the folder an upload is still streaming into must not go at all — see
/// `State::audition_staging` for what happens when it does.
fn sweep_auditions(root: &Path, keep: &[PathBuf]) -> usize {
    let mut stuck = 0;
    for dir in audition_dirs(root) {
        if keep.contains(&dir) {
            continue;
        }
        if std::fs::remove_dir_all(&dir).is_err() && dir.exists() {
            stuck += 1;
        }
    }
    stuck
}

/// When to look again after a sweep, given what it left behind.
///
/// `None` means there is nothing outstanding, or the budget is spent and the backstops have it from
/// here. Pure so the policy can be read on its own, like [`demo_is_due`].
fn next_sweep(tries_left: u8, stuck: usize, now: Instant) -> Option<AuditionSweep> {
    if stuck == 0 || tries_left == 0 {
        return None;
    }
    Some(AuditionSweep {
        due: now + AUDITION_RETRY,
        tries: tries_left - 1,
    })
}

/// An ISO-8601 timestamp, to the second, in UTC.
///
/// Hand-rolled because the only place the machine needs a wall-clock time is stamping an install,
/// and a date library for one line is not worth the dependency. Days-since-epoch converted with the
/// civil-from-days algorithm.
fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (days, seconds) = (now / 86_400, now % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    let (hour, minute, second) = (seconds / 3_600, (seconds % 3_600) / 60, seconds % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 to a calendar date. Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// What just happened, as far as demo mode's clock is concerned.
///
/// The whole timing policy is which of these is reported and what [`demo_resume_after`] does with
/// it, which is why they are named for the event rather than for the delay they produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DemoEvent {
    /// A demo song ended with nobody taking a turn: it ran out, or somebody skipped it.
    /// **The only event that does not buy a wait.**
    DemoEnded,
    /// A song somebody asked for ended, or somebody touched the machine at all.
    Somebody,
}

/// When a demo song may next start.
///
/// **One asymmetry carries the entire feature**, and it is worth stating plainly because it is the
/// thing a reader will otherwise assume is a bug: a demo song ending starts the next one *at once*,
/// and everything else buys the full delay.
///
/// That is the difference between a machine that fills a silence and a machine that will not stop
/// talking. A demo chaining with no gap is what a real unit does — the sound never drops out, so
/// nobody has to wonder whether it broke. But the moment a person is involved at all, the machine
/// has to get out of the way for long enough that they can decide what to do next: queue a song,
/// leave, or turn the thing off. Two minutes of silence after somebody sings is not the machine
/// being slow, it is the machine not interrupting.
///
/// `Somebody` covers ending a real song *and* every deliberate act — queueing, stopping, reordering,
/// skipping somebody's song — because they are the same fact from the clock's point of view: a person
/// is here. Collapsing them into one variant is deliberate, and the alternative (a variant per call
/// site) would be a policy table where every row said the same thing.
///
/// **Skipping a *demo* is the one deliberate act on the other side of the line, and it is a person
/// asking for a different song rather than for quiet.** Stop is the press that asks for quiet, and it
/// keeps the full delay; skip asks for the next thing, and a machine that answers it with a minute of
/// silence has misheard. Nobody's turn ends either way, which is what makes this a demo ending early
/// rather than a person taking one.
fn demo_resume_after(what: DemoEvent, now: Instant, delay: Duration) -> Instant {
    match what {
        DemoEvent::DemoEnded => now,
        DemoEvent::Somebody => now + delay,
    }
}

/// Where a running deadline lands when the delay itself changes.
///
/// **A shift, not a re-arm**, and the difference is the whole of [`Machine::set_demo_delay`]'s
/// argument: `resume_at` is `the last thing somebody did + was`, so subtracting the old delay
/// recovers that moment and adding the new one is the answer. Nothing here reads the clock, which is
/// what makes it a pure function of the deadline and the two numbers.
///
/// **Saturating in both directions and neither is a special case.** A deadline that lands in the
/// past is a demo due on the next poll, which is exactly what shortening the delay past the time
/// already spent quiet should do; `Instant` arithmetic that would run off either end simply clamps,
/// and the clamped value means the same thing the real one would.
fn demo_deadline_moved(resume_at: Instant, was: u32, now_secs: u32) -> Instant {
    let idle_since = resume_at
        .checked_sub(Duration::from_secs(u64::from(was)))
        .unwrap_or(resume_at);
    idle_since
        .checked_add(Duration::from_secs(u64::from(now_secs)))
        .unwrap_or(idle_since)
}

/// Whether a demo song should start right now.
///
/// Five conditions, and the two in the middle are the ones that make queueing behave the way the
/// feature promises:
///
/// * **Nothing loaded.** A demo never interrupts — not a person's song and not its own.
///
///   **This is not the same statement in reverse, and the asymmetry is deliberate.** A demo will
///   not start over anything, and a queued song *will* start over a demo: see
///   [`Machine::advance_if_idle_or_over_a_demo`]. What is protected is somebody's turn, and a
///   machine singing to itself has not got one.
/// * **The queue is empty.** A queued song is somebody's answer to "what next", and the machine has
///   no business talking over it. This is what stops a demo starting between two real songs.
/// * The deadline has passed, and demo mode is on at all.
/// * **The machine is on the screen**, which only Android can answer with anything but yes.
///
/// **`once` is a hand-pressed trigger and it bypasses the first and the last of those four**, which
/// is the whole difference between the mode and the button: somebody pressing `Play something` has
/// said *now*, so there is no deadline left to wait for, and they have said it about one song rather
/// than about the mode. It does **not** bypass the two in the middle — a trigger that interrupted a
/// song or talked over a queue would be the one thing this feature has always promised not to do,
/// and it is refused at the route long before it reaches here. See [`why_no_demo`].
///
/// **`foreground` is the one condition nothing bypasses**, hand-pressed trigger included. Every
/// other refusal here is about politeness to a person in the room; this one is about there being no
/// room. A machine that is not on the screen and starts singing anyway is the fault the whole
/// parameter was added for — see [`Machine::settle_foreground`] — and a trigger arriving from a
/// remote does not make an invisible song any better.
fn demo_is_due(
    enabled: bool,
    once: bool,
    loaded: bool,
    queue_empty: bool,
    resume_at: Instant,
    now: Instant,
    foreground: bool,
) -> bool {
    foreground && (enabled || once) && !loaded && queue_empty && (once || now >= resume_at)
}

/// Why a hand-pressed demo cannot start, worded for somebody to read, or `None` when it can.
///
/// The same conditions [`demo_is_due`] applies, minus the two about time, because a trigger is
/// somebody saying *now*. **All four are knowable without touching the catalog**, which is what
/// lets the route refuse synchronously while the song itself starts a moment later on the poll
/// thread.
///
/// **Off the screen is asked first**, because it is the only one of the four that is about the
/// machine rather than about the moment. The other three tell somebody to wait; this one tells them
/// the machine is not somewhere a song could be heard, which is the more useful thing to read and
/// the only one that would otherwise be reported as a song "already playing" — a paused song being
/// a loaded one.
///
/// **A catalog with nothing playable in it is deliberately not among them.** Discovering that
/// means the two full-table draws [`Machine::pick_demo_song`] makes, which is exactly the work this
/// arrangement exists to keep off a request thread — and a machine with no songs says so on every
/// screen a person could be looking at. The `tracing::warn!` in
/// [`Machine::start_demo`] stays the only report of it.
fn why_no_demo(
    loaded: bool,
    queue_empty: bool,
    can_play: bool,
    foreground: bool,
) -> Option<&'static str> {
    if !foreground {
        return Some("the machine is not on the screen");
    }
    if loaded {
        return Some("a song is already playing");
    }
    if !queue_empty {
        return Some("there are songs in the queue to play first");
    }
    if !can_play {
        return Some("this machine has no sound");
    }
    None
}

/// Whether a hand-pressed trigger survives an event.
///
/// **Only a demo song ending leaves one standing, and one line covers both halves of the rule.**
/// [`Machine::maybe_start_demo`] arms `Somebody` immediately before it starts a song, so a trigger is
/// spent by the attempt it causes — including an attempt that found nothing to play, which is what
/// stops an empty catalog being retried twenty times a second. And `Somebody` is what every
/// deliberate act arms, so a trigger is canceled by anybody who queues, skips or stops before the
/// next poll fires it.
fn demo_once_after(what: DemoEvent, pending: bool) -> bool {
    pending && matches!(what, DemoEvent::DemoEnded)
}

/// How long until a demo song starts, for a client to show. `None` when none is coming.
///
/// Deliberately `None` rather than `Some(0)` once the deadline has passed: zero would read as a
/// countdown that has stalled, where the truth is that the next poll will start a song. It is also
/// `None` whenever [`demo_is_due`]'s other conditions fail, because a countdown shown while a
/// song is queued is a promise the machine is not making — and a countdown shown while the machine
/// is off the screen is the same promise, broken in the same way, since nothing will fire it.
fn demo_starts_in(
    enabled: bool,
    loaded: bool,
    queue_empty: bool,
    resume_at: Instant,
    now: Instant,
    foreground: bool,
) -> Option<u32> {
    if !foreground || !enabled || loaded || !queue_empty {
        return None;
    }
    let remaining = resume_at.checked_duration_since(now)?;
    // A zero remainder means "the next poll starts one", which `None` says and `Some(0)` does not.
    (!remaining.is_zero())
        .then(|| remaining.as_secs().saturating_add(1).min(u32::MAX as u64) as u32)
}

/// Whether a bank swap may drop the audio stream, given what is loaded.
///
/// **The safety rule of the whole feature, and the one that is not obvious.** Dropping the stream is
/// how a new bank is heard now rather than next song — the synthesizer is inside it — and the caller
/// puts the song back afterwards. It can only put back a song it is able to rebuild:
///
/// * nothing loaded — safe, and there is no song to lose.
/// * a MIDI song — safe, because the parsed `Arc<Song>` is still on the control thread.
/// * a video or MP3+G song — **not** safe. Its audio is a `TrackPlayer` that was moved into the
///   audio thread, and `VideoSong::open` bound the feed writer to the decoder, so there is no way to
///   mint a second one from the decoder still running here. Dropping the stream would end the song
///   for good, and it would buy nothing: those songs run `Program::Track` and never touch the
///   synthesizer, so there is no difference to hear.
fn rebuild_stream_for(has_song: bool, is_midi: bool) -> bool {
    !has_song || is_midi
}

/// Parses a bank, in the words a caller can hand to somebody choosing it.
///
/// One function because two paths into the swap both need it and both have to refuse the same way —
/// the synthesizer's own message rather than a paraphrase, since "the RIFF chunk was not found" is a
/// fact about this synthesizer and not about the file being broken.
fn load_bank(path: &Path) -> Result<km_audio::Bank, ControlError> {
    km_audio::Bank::load(path).map_err(|error| {
        ControlError::Rejected(format!("{} will not open: {error}", path.display()))
    })
}

/// How far a download has got, as a whole percentage, or `None` when there is nothing to divide by.
///
/// **Clamped, because the two numbers do not always measure the same thing.** For a bank published
/// inside a zip the bytes being counted are the archive's while the total is the extracted member's,
/// so the ratio can overshoot; a bar that reads 104% is worse than one that sits at 100 for a moment.
fn percentage(done: u64, total: u64) -> Option<u8> {
    (total > 0).then(|| (done.saturating_mul(100) / total).min(100) as u8)
}

/// The `music_volume` the research note measured for a bank, matched by filename.
///
/// **The filename is the only thing a bank on disk and a row in the table reliably share.** A `.sf2`
/// carries an internal name, and the note found it routinely names whatever bank the file was cut
/// from rather than the file itself, so it cannot be used to identify one. Size would work and is
/// worse: reading it says nothing a name does not, and a bank that has been re-cut has the wrong one
/// while still being the bank the level was measured for.
///
/// A bank the table does not know returns `None`, which leaves the owner's own level alone. Guessing
/// is the one thing this must not do: a level is a measurement, and inventing one for an unmeasured
/// bank would make the machine quieter for no reason anybody could look up.
fn measured_level(path: &Path) -> Option<f32> {
    let name = path.file_name()?.to_str()?;
    crate::banks::catalog()
        .iter()
        .find(|row| row.name == name)
        .and_then(|row| row.volume)
}

/// The loudness a bank renders at, which is the level a video or MP3+G song is brought down to.
///
/// Joined to the table by filename, the same rule and for the same reason as [`measured_level`]
/// beside it — a `.sf2`'s internal name routinely names whatever bank the file was cut from.
///
/// **A bank the table has not measured falls back rather than abstaining**, which is the opposite of
/// what `measured_level` does with an unknown bank, and the two are right for opposite reasons.
/// There, guessing a `music_volume` would make the machine quieter for a reason nobody could look
/// up. Here, abstaining is not neutral: it would mean *no levelling*, so a video would go on playing
/// ten decibels above the MIDI songs around it — the whole complaint. A fallback is wrong by less
/// than that.
///
/// **How much less depends on the bank, and the range is wider than it looks.** The fifteen banks
/// measured for this span −12.4 to −26.6 LUFS: the fallback is within a decibel or two of most of
/// them and nine out for the loudest. That is an argument for a `lufs` on every row somebody might
/// actually choose rather than for a cleverer fallback — a bank in the table is measured, and a bank
/// nobody has measured is the only case this answers.
fn reference_lufs(path: &Path) -> f32 {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| {
            crate::banks::catalog()
                .iter()
                .find(|row| row.name == name)
                .and_then(|row| row.lufs)
        })
        .unwrap_or(km_loudness::DEFAULT_REFERENCE_LUFS)
}

/// The level to send after a bank swap: the slot's own, or the machine's where the slot has none.
///
/// **A bank with no measured level has to put the owner's level back rather than leave the last
/// one's reduction in place**, and sending nothing does not do that. `Sticky` replays the last
/// volume it saw on every stream rebuild, so a slot that stays quiet inherits whatever the previous
/// bank asked for — which turns one leveled slot into a 4 dB reduction on every unleveled slot
/// pressed after it, slot 1 included. Slot 1 is the fixed reference the rest are compared against,
/// so contaminating it costs the whole comparison rather than one bank.
///
/// The machine's own level is the right thing to return to because nothing on this path ever
/// changes it: `switch_debug_soundfont` sends `SetMusicVolume` straight to the engine and never
/// through the settings patch, so `audio.music_volume` still holds what the owner set — including a
/// level they moved from a remote, which is mirrored back into it.
///
/// This is the switcher's version of what `--set-soundfont` already does across a restart, where
/// `soundfont-override.json` stashes the previous level for the same reason.
fn volume_for_bank(slot_volume: Option<f32>, machine_volume: f32) -> f32 {
    slot_volume.unwrap_or(machine_volume).clamp(0.0, 1.0)
}

/// Says once, in the log, that a file did not read all the way through.
///
/// `Song::parse` cannot do this itself: it takes bytes and has no idea what they were called, and a
/// warning nobody can trace back to a file is not a warning. It is deliberately not an error — the
/// song plays, and 2.01% of the parsed corpus is truncated somewhere — but a catalog holding
/// a damaged file should be findable without re-deriving which one it was.
///
/// Nothing is said about a repaired note on its own. A note-off synthesized in an otherwise
/// well-formed file is the parser doing its job on 2.91% of that corpus, and a line per song in
/// thirty-four would be noise; it is worth reporting only as detail beside a truncation.
fn warn_if_damaged(identity: &str, song: &Song) {
    if song.truncated_tracks.is_empty() && song.missing_tracks == 0 {
        return;
    }
    tracing::warn!(
        song = %identity,
        truncated = ?song.truncated_tracks,
        missing = song.missing_tracks,
        repaired = song.repaired_notes,
        "the file did not read to the end; everything past the damage is lost"
    );
}

/// The on-screen label for the bank in force.
///
/// Short because it is on screen for the whole song beside the lyrics, and it leads with the slot
/// because the slot is the thing somebody just pressed. The suffix is only ever there while a bank
/// has been chosen but is not yet being heard — a distinction the room cannot otherwise make.
fn soundfont_label(slot: u8, total: usize, name: &str, pending: bool) -> String {
    let mut label = format!("sf {slot}/{total} {name}");
    if pending {
        label.push_str(" — from the next MIDI song");
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(banks: &[u16]) -> BTreeSet<u16> {
        banks.iter().copied().collect()
    }

    /// A MIDI song loaded from nothing but bytes, for the two questions the words flag answers.
    fn loaded_midi(lyrics_hidden: bool) -> Loaded {
        let song = Arc::new(
            km_song::Song::parse(
                &km_song::testing::soft_karaoke(),
                &km_song::ParseOptions::default(),
            )
            .expect("fixture parses"),
        );
        Loaded {
            origin: Origin::File {
                path: "a.kar".to_owned(),
            },
            title: "T".to_owned(),
            artist: None,
            language: None,
            singer: None,
            kind: SongKind::Midi,
            duration_ms: 1_000,
            melody_channel: None,
            lyrics_hidden,
            fixes: km_fixes::ChannelFixes::default(),
            loudness_lufs: None,
            gain: Loaded::UNLEVELLED,
            media: Media::Midi(song),
        }
    }

    /// Turning a song's words off is one answer that four surfaces read.
    ///
    /// `lyric_song` is the funnel — the television's rows, the streamed screen's rows, the
    /// `lyric_line` events and `has_lyrics` all go through it — so a song with words in it reports
    /// exactly what a song with none does, and nothing downstream has to know why.
    #[test]
    fn a_song_whose_words_are_turned_off_offers_no_timeline_and_reports_no_lyrics() {
        let drawn = loaded_midi(false);
        assert!(drawn.lyric_song().is_some());
        assert!(
            drawn.describe().has_lyrics,
            "the fixture has words, or the other half of this test says nothing"
        );

        let silenced = loaded_midi(true);
        assert!(silenced.lyric_song().is_none());
        assert!(!silenced.describe().has_lyrics);
        assert!(
            silenced.describe().lyrics_hidden,
            "and the television needs to know it is a decision rather than an empty file"
        );
    }

    /// One listed output, with only the two fields the level lookup reads.
    fn listed(id: &str, system_default: bool) -> km_audio::device::OutputDevice {
        km_audio::device::OutputDevice {
            id: id.to_owned(),
            name: id.to_owned(),
            system_default,
            usb: false,
            available: true,
            preferred: true,
        }
    }

    /// A level needs a card, and *follow the system* is not one.
    ///
    /// **This is what keeps the control from being a thing only a hand-configured machine has.**
    /// The sentinel is what the engine reports as active whenever nothing has been chosen, which is
    /// every machine out of the box; reading a level from that string finds no card and reports
    /// none, which on a perfectly ordinary sound card reads as *this output has no level*.
    #[test]
    fn following_the_system_reads_the_level_of_whatever_that_is_today() {
        let devices = [
            listed(km_audio::SYSTEM_DEFAULT, false),
            listed("alsa:plughw:CARD=Onboard,DEV=0", true),
            listed("alsa:plughw:CARD=Other,DEV=0", false),
        ];
        assert_eq!(
            Machine::level_device(km_audio::SYSTEM_DEFAULT, &devices),
            "alsa:plughw:CARD=Onboard,DEV=0"
        );
    }

    /// A device chosen by hand is read as itself, sentinel or no sentinel in the list.
    #[test]
    fn a_named_output_is_the_one_its_level_comes_from() {
        let devices = [listed("alsa:plughw:CARD=Onboard,DEV=0", true)];
        assert_eq!(
            Machine::level_device("alsa:plughw:CARD=Other,DEV=0", &devices),
            "alsa:plughw:CARD=Other,DEV=0"
        );
    }

    /// Nothing to resolve to is answered with the sentinel, which finds no card and reports no
    /// level — the same honest answer, rather than a panic or a guess at the first row.
    #[test]
    fn a_list_naming_no_system_default_leaves_the_sentinel_alone() {
        let devices = [listed("alsa:plughw:CARD=Onboard,DEV=0", false)];
        assert_eq!(
            Machine::level_device(km_audio::SYSTEM_DEFAULT, &devices),
            km_audio::SYSTEM_DEFAULT
        );
    }

    /// The sentinel is spelled the same in the audio backend and in the API's vocabulary.
    ///
    /// **The drift guard for two constants that must not be one dependency**, exactly as the
    /// refusal-code test below is. `km-api` does not depend on `km-audio` and should not; a page
    /// under `km-admin-pages` needs the string in order to label the *follow the system* row in the
    /// reader's own language instead of the backend's English. This crate is the only one that sees
    /// both, so this is where they are held together — and a rename on either side that missed the
    /// other would otherwise show as a picker whose first row is untranslated and whose real
    /// hardware is labelled as the system default.
    #[test]
    fn the_system_output_sentinel_is_spelled_one_way() {
        assert_eq!(km_api::machine::SYSTEM_OUTPUT, km_audio::SYSTEM_DEFAULT);
    }

    /// Every refusal code this machine sends reaches a real sentence on the singer's remote.
    ///
    /// **The drift guard for two vocabularies that must not be one dependency.** `km-remote-pages`
    /// spells these codes itself, because the dependency runs the other way — the machine links the
    /// pages — so nothing but this test stops a rename here from silently turning into the generic
    /// refusal there. It fails loudly on the one thing that would otherwise be invisible: a singer
    /// pressing KEY+ on a video song and being told only that the machine cannot do that.
    #[test]
    fn every_refusal_this_machine_sends_is_a_sentence_the_remote_has() {
        let codes = [
            no_key_code(SongKind::Midi),
            no_key_code(SongKind::Video),
            no_key_code(SongKind::Cdg),
            no_key_code(SongKind::UltraStar),
            no_key_code(SongKind::Lrc),
            no_key_code(SongKind::Unknown),
            no_tempo_code(SongKind::Midi),
            no_tempo_code(SongKind::Video),
            no_tempo_code(SongKind::Cdg),
            no_tempo_code(SongKind::UltraStar),
            no_tempo_code(SongKind::Lrc),
            no_tempo_code(SongKind::Unknown),
            no_melody_code(SongKind::Midi),
            no_melody_code(SongKind::Video),
            no_melody_code(SongKind::Cdg),
            no_melody_code(SongKind::UltraStar),
            no_melody_code(SongKind::Lrc),
            no_melody_code(SongKind::Unknown),
            NO_MELODY_CHANNEL,
            NOTHING_PLAYING,
            NOTHING_LOADED,
            NOTHING_QUEUED,
            NO_SOUND,
        ];
        for code in codes {
            let key = km_remote_pages::words::refusal_key(code);
            assert_ne!(
                key,
                km_remote_pages::words::ERROR_UNAVAILABLE,
                "`{code}` fell through to the generic refusal on the remote"
            );
        }
    }

    /// A kind reaches the remote, so the article can agree with the noun.
    #[test]
    fn a_refusal_about_a_song_says_which_kind_of_song() {
        for (kind, expected) in [
            (SongKind::Midi, "midi"),
            (SongKind::Video, "video"),
            (SongKind::Cdg, "cdg"),
            (SongKind::UltraStar, "ultrastar"),
            (SongKind::Lrc, "lrc"),
            (SongKind::Unknown, "other"),
        ] {
            assert_eq!(
                km_remote_pages::words::refusal_kind(no_key_code(kind)),
                expected,
                "{kind:?}"
            );
        }
    }

    /// A picture id carries the extension, which is what keeps it off `/wallpapers/next`.
    ///
    /// **Two things at once, and the second is the load-bearing one.** A folder may hold
    /// `sunset.jpg` beside `sunset.png` — a stem-based id, which is what a SoundFont uses, would
    /// give those one id and make one of them undeletable. And because the extension is always
    /// there, no id can equal the static segment `next` in the route table; axum matches the path
    /// before the method, so a collision would answer 405 to a Remove button for one unlucky
    /// filename and nothing else.
    #[test]
    fn a_picture_id_carries_the_extension_so_it_cannot_be_a_route_segment() {
        assert_eq!(
            picture_id(Path::new("sunset.jpg")).as_deref(),
            Some("sunset-jpg")
        );
        assert_ne!(
            picture_id(Path::new("sunset.jpg")),
            picture_id(Path::new("sunset.png")),
            "one stem, two files, two ids"
        );
        // The whole point: `next.jpg` is not `next`.
        assert_eq!(
            picture_id(Path::new("next.jpg")).as_deref(),
            Some("next-jpg")
        );
        // Punctuation collapses to single dashes and the ends are trimmed, as a bank id does.
        assert_eq!(
            picture_id(Path::new("A Beach — 2019 (large).JPEG")).as_deref(),
            Some("a-beach-2019-large-jpeg")
        );
        // Nothing usable left is `None` rather than an empty id that every file would share.
        assert_eq!(picture_id(Path::new("---")), None);
    }

    /// The asymmetry the whole of demo mode rests on.
    ///
    /// A demo chaining with no gap is what a real unit does — the sound never drops out, so nobody
    /// wonders whether it broke. Everything a *person* does buys the full silence back, because the
    /// machine's job at that moment is to get out of their way. Which of the two a skip reports is
    /// `Machine::transport`'s to decide, and
    /// [`skipping_a_demo_starts_another_and_skipping_a_singers_song_does_not`] is where that lives.
    #[test]
    fn a_demo_chains_at_once_and_a_person_buys_the_silence_back() {
        let now = Instant::now();
        let delay = Duration::from_secs(120);

        assert_eq!(
            demo_resume_after(DemoEvent::DemoEnded, now, delay),
            now,
            "one demo song must run straight into the next"
        );
        assert_eq!(
            demo_resume_after(DemoEvent::Somebody, now, delay),
            now + delay,
            "after a person, the machine waits the full delay"
        );
    }

    /// Changing the delay moves the running deadline by the change, and re-arms nothing.
    ///
    /// **This is the switch's rule seen from the other side.** `set_demo` deliberately leaves the
    /// deadline alone because the clock counts *idleness*, and the same sentence decides this:
    /// `resume_at` is `the moment somebody last did something + the delay`, so a new delay measures
    /// from that same moment. Re-arming from now would make shortening the delay lengthen the wait
    /// — once, in front of whoever had just shortened it to see whether it worked.
    #[test]
    fn changing_the_delay_moves_the_deadline_and_does_not_restart_it() {
        let idle_since = Instant::now();
        let armed = idle_since + Duration::from_secs(120);

        assert_eq!(
            demo_deadline_moved(armed, 120, 60),
            idle_since + Duration::from_secs(60),
            "sixty seconds from when the room went quiet, not from now"
        );
        assert_eq!(
            demo_deadline_moved(armed, 120, 300),
            idle_since + Duration::from_secs(300),
            "and lengthening measures from the same moment"
        );
        assert_eq!(
            demo_deadline_moved(armed, 120, 0),
            idle_since,
            "zero means the next poll, which is a deadline already in the past"
        );
        assert_eq!(
            demo_deadline_moved(armed, 120, 120),
            armed,
            "saving the number it already had must not move anything"
        );
    }

    /// A zero delay is "at once", not "never" — the opposite of `audio.idle_release_secs`.
    #[test]
    fn a_zero_delay_means_the_demo_starts_as_soon_as_the_machine_is_idle() {
        let now = Instant::now();
        assert_eq!(
            demo_resume_after(DemoEvent::Somebody, now, Duration::ZERO),
            now
        );
    }

    /// The five conditions, each failed on its own.
    #[test]
    fn a_demo_starts_only_when_nothing_is_loaded_and_nothing_is_waiting() {
        let now = Instant::now();
        let past = now - Duration::from_secs(1);
        let future = now + Duration::from_secs(60);

        assert!(
            demo_is_due(true, false, false, true, past, now, true),
            "on, idle, empty queue, deadline passed"
        );

        assert!(
            !demo_is_due(false, false, false, true, past, now, true),
            "mode is off"
        );
        // The two that make queueing behave as promised.
        assert!(
            !demo_is_due(true, false, true, true, past, now, true),
            "a demo must never interrupt a song, its own included"
        );
        assert!(
            !demo_is_due(true, false, false, false, past, now, true),
            "a queued song is somebody's answer to `what next`; the machine must not talk over it"
        );
        assert!(
            !demo_is_due(true, false, false, true, future, now, true),
            "still waiting"
        );
        assert!(
            !demo_is_due(true, false, false, true, past, now, false),
            "a machine that is not on the screen has nobody to sing to"
        );

        // Exactly on the deadline counts, so a delay of zero is not a delay of one poll.
        assert!(demo_is_due(true, false, false, true, now, now, true));
    }

    /// Off the screen is the one refusal a hand-pressed trigger does not buy its way past.
    ///
    /// Every other condition here is about being polite to somebody in the room. This one is about
    /// there being no room: on Android the watchdog thread goes on running after the activity stops,
    /// so without it a backgrounded machine picks a song and plays it to nobody — which is the fault
    /// the whole parameter exists for. A trigger arriving from a remote does not improve that.
    #[test]
    fn nothing_starts_a_demo_on_a_machine_that_is_not_on_the_screen() {
        let now = Instant::now();
        let past = now - Duration::from_secs(1);

        assert!(
            !demo_is_due(true, false, false, true, past, now, false),
            "the mode is on and everything else is right, but the screen is gone"
        );
        assert!(
            !demo_is_due(false, true, false, true, past, now, false),
            "a hand-pressed trigger does not bypass this one"
        );
        assert_eq!(
            why_no_demo(false, true, true, false),
            Some("the machine is not on the screen"),
            "and the route says so rather than refusing for one of the other three reasons"
        );
        assert_eq!(
            why_no_demo(true, false, false, false),
            Some("the machine is not on the screen"),
            "asked first, so it is what a paused song off-screen reports"
        );
        assert_eq!(
            demo_starts_in(true, false, true, now + Duration::from_secs(31), now, false),
            None,
            "and no countdown is shown for a song that will not start"
        );
    }

    /// A hand-pressed trigger bypasses the mode and the clock, and neither of the other two.
    ///
    /// Those two bypasses are the whole of what the button is: somebody has said *now*, so there is
    /// no deadline left to wait for, and they have said it about one song rather than about the mode.
    #[test]
    fn a_hand_pressed_demo_starts_one_song_although_the_mode_is_off_and_the_clock_is_not_up() {
        let now = Instant::now();
        let future = now + Duration::from_secs(60);

        assert!(
            demo_is_due(false, true, false, true, future, now, true),
            "mode off and a delay still to run, and it starts anyway"
        );
        assert!(
            !demo_is_due(false, false, false, true, future, now, true),
            "and without the trigger it does not"
        );
    }

    /// The two conditions a trigger may not bypass, which is what stops the button being a skip.
    #[test]
    fn a_hand_pressed_demo_still_refuses_to_interrupt_a_song_or_a_queue() {
        let now = Instant::now();
        let past = now - Duration::from_secs(1);

        assert!(
            !demo_is_due(false, true, true, true, past, now, true),
            "loaded"
        );
        assert!(
            !demo_is_due(false, true, false, false, past, now, true),
            "somebody is waiting"
        );
    }

    /// A trigger is spent by the attempt it causes and canceled by anybody who acts first.
    ///
    /// One line covers both, because `maybe_start_demo` arms `Somebody` immediately before it starts
    /// a song and every deliberate act arms the same event. The `DemoEnded` case is what lets a
    /// triggered song chain when the mode is on — and it can only ever be reached from a demo that
    /// was already playing, which a trigger cannot have started twice.
    #[test]
    fn a_trigger_is_spent_by_the_attempt_it_causes_and_canceled_by_anybody_who_acts() {
        assert!(!demo_once_after(DemoEvent::Somebody, true), "spent");
        assert!(
            demo_once_after(DemoEvent::DemoEnded, true),
            "a demo ending is not somebody acting"
        );
        assert!(
            !demo_once_after(DemoEvent::DemoEnded, false),
            "and nothing conjures one that was never pressed"
        );
    }

    /// The three refusals a trigger can give, in the order it gives them.
    ///
    /// **All three are answerable without touching the catalog**, which is the property that lets
    /// the route say no synchronously while the song starts later on the poll thread. A loaded song
    /// is named before a queue because it is the more immediate answer to "why did nothing happen":
    /// somebody is singing.
    #[test]
    fn the_refusals_a_trigger_can_give_are_all_knowable_without_the_catalog() {
        assert_eq!(why_no_demo(false, true, true, true), None);
        assert_eq!(
            why_no_demo(true, true, true, true),
            Some("a song is already playing")
        );
        assert_eq!(
            why_no_demo(false, false, true, true),
            Some("there are songs in the queue to play first")
        );
        assert_eq!(
            why_no_demo(false, true, false, true),
            Some("this machine has no sound")
        );
        // A loaded song leads, even when the queue is not empty either.
        assert_eq!(
            why_no_demo(true, false, false, true),
            Some("a song is already playing")
        );
    }

    /// The countdown a client shows, and the three states where showing one would be a lie.
    #[test]
    fn the_countdown_is_absent_whenever_no_demo_is_actually_coming() {
        let now = Instant::now();
        let soon = now + Duration::from_secs(30);

        assert_eq!(demo_starts_in(true, false, true, soon, now, true), Some(31));

        assert_eq!(
            demo_starts_in(false, false, true, soon, now, true),
            None,
            "off"
        );
        assert_eq!(
            demo_starts_in(true, true, true, soon, now, true),
            None,
            "something is playing, so nothing is counting down"
        );
        assert_eq!(
            demo_starts_in(true, false, false, soon, now, true),
            None,
            "a queued song cancels it, and a countdown would promise otherwise"
        );
        // Past the deadline the next poll starts a song. `None` says that; `Some(0)` would read as
        // a counter that had stalled.
        assert_eq!(
            demo_starts_in(true, false, true, now - Duration::from_secs(5), now, true),
            None
        );
        assert_eq!(demo_starts_in(true, false, true, now, now, true), None);
    }

    /// A refused package at a path, for the resolution tests.
    fn refused(path: &str) -> PackageProblem {
        PackageProblem {
            path: path.to_owned(),
            package_id: None,
            reason: "could not read manifest.json".to_owned(),
        }
    }

    /// The two files sharing a name resolve to one row each, and neither shadows the other.
    ///
    /// The observed case: the same package in two of the folders the machine scans.
    #[test]
    fn each_of_two_refused_files_with_one_name_resolves_to_itself() {
        let problems = vec![
            refused("/data/packages/carols.kmpkg"),
            refused("/tunes/karaoke/carols.kmpkg"),
        ];
        for problem in &problems {
            let found = resolve_problem(&problems, &problem.id()).expect("resolved");
            assert_eq!(found.path, problem.path);
        }
    }

    /// An id nothing answers to is a miss, and a crafted one is only ever a miss.
    ///
    /// **This is what "never joined onto a folder" means in practice.** The id is compared against
    /// ids computed from paths the machine itself recorded, so a browser cannot name a file the
    /// machine never found — there is no traversal to attempt, because there is no join.
    #[test]
    fn an_id_that_names_no_problem_is_not_found() {
        let problems = vec![refused("/data/packages/carols.kmpkg")];
        for id in [
            "nothing-00000000",
            "../../etc/passwd",
            "..%2f..%2fetc%2fpasswd",
            "/data/packages/carols.kmpkg",
        ] {
            assert!(
                matches!(
                    resolve_problem(&problems, id),
                    Err(CatalogError::NotFound(_))
                ),
                "{id} resolved to something"
            );
        }
    }

    /// Two files answering to one id delete neither, and say why.
    ///
    /// Needs a fingerprint collision to happen for real, which is why it is arranged here by hand:
    /// the branch has to be reachable to be worth having, and the alternative to having it is
    /// deleting an arbitrary one of the two.
    #[test]
    fn two_files_answering_to_one_id_delete_neither() {
        let one = refused("/data/packages/carols.kmpkg");
        // The same path twice is the only way to force a collision without knowing one, and it
        // exercises the same branch: `record_package_problem` de-dupes by path, so a real list
        // cannot hold this — which is the point. The guard is against the case nobody arranged.
        let problems = vec![one.clone(), one.clone()];
        let why = match resolve_problem(&problems, &one.id()) {
            Err(CatalogError::Rejected(why)) => why,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert!(why.contains("two of the refused files"), "{why}");
    }

    /// A package in the folder the machine owns is the machine's to delete.
    #[test]
    fn a_package_in_the_packages_folder_is_the_machines_to_delete() {
        let paths = Paths::rooted_at("/data");
        let file = paths.packages_dir().join("vol1.kmpkg");
        assert_eq!(not_mine_to_delete(&paths, &[], &file), None);
    }

    /// A file `debug.packages` names is refused, wherever it happens to sit.
    ///
    /// **Including inside the packages folder**, which is the case the order of the two checks
    /// decides: the entry is what makes the package come back at the next pass, so removing the file
    /// alone would not stick and the sentence has to name the entry rather than the folder.
    #[test]
    fn a_package_named_in_debug_packages_is_not() {
        let paths = Paths::rooted_at("/data");
        let outside = PathBuf::from("/somewhere/of/their/own/vol1.kmpkg");
        let why =
            not_mine_to_delete(&paths, std::slice::from_ref(&outside), &outside).expect("refused");
        assert!(why.contains("debug.packages"), "{why}");
        assert!(why.contains("--clear-debug-packages"), "{why}");

        let inside = paths.packages_dir().join("vol1.kmpkg");
        let why = not_mine_to_delete(&paths, std::slice::from_ref(&inside), &inside)
            .expect("refused even in the machine's own folder");
        assert!(why.contains("debug.packages"), "{why}");
    }

    /// A package outside every folder the machine owns is not the machine's to delete either.
    ///
    /// What this rules out is the shipped asset tree — root-owned under `/opt`, inside a signed
    /// bundle, unpacked from an APK. Nothing puts a package there today, and this is what keeps that
    /// true of whatever route somebody adds next.
    #[test]
    fn a_package_outside_the_machines_folders_is_not_either() {
        let paths = Paths::rooted_at("/data");
        let why = not_mine_to_delete(&paths, &[], Path::new("/opt/karaoke/assets/vol1.kmpkg"))
            .expect("refused");
        assert!(why.contains("not in a folder this machine owns"), "{why}");
        // It says what to do instead, because refusing with no remedy is a dead end for somebody
        // holding a D-pad.
        assert!(why.contains("remove the file yourself"), "{why}");
    }

    /// The rule that keeps a bank swap from ending a video song.
    #[test]
    fn only_a_midi_song_or_no_song_lets_the_stream_be_rebuilt() {
        // Nothing loaded: safe, and nothing to put back.
        assert!(rebuild_stream_for(false, false));
        // A MIDI song: safe, because the parsed song is still here to re-send.
        assert!(rebuild_stream_for(true, true));
        // A video or MP3+G song: its audio cannot be rebuilt, and there is nothing to gain by
        // trying — no synthesizer is in the path at all.
        assert!(!rebuild_stream_for(true, false));
    }

    /// A bank the note measured brings its level with it, wherever the file was put.
    ///
    /// **The numbers here are hundredths since §13.** The rule is unchanged — the largest step that
    /// keeps the bank's peak under full scale — but tenths were discarding up to a whole step, which
    /// a listening session caught: Chorium clears at 0.69 and was being handed 0.6.
    #[test]
    fn a_measured_bank_is_recognized_by_its_filename() {
        // Found in the folder an owner drops banks into.
        assert_eq!(
            measured_level(Path::new(
                "/var/lib/karaoke/soundfonts/Roland SC-55 v3.7.sf2"
            )),
            Some(0.61)
        );
        assert_eq!(
            measured_level(Path::new("D:/banks/MuseScore_General.sf2")),
            Some(0.75)
        );
        assert_eq!(measured_level(Path::new("SGM-V2.01.sf2")), Some(0.83));
        // A bank the table knows and that needs no reduction has none to bring.
        assert_eq!(measured_level(Path::new("TimGM6mb.sf2")), None);
        // And one it has never heard of leaves the owner's level alone rather than guessing.
        assert_eq!(measured_level(Path::new("SomebodysOwnBank.sf2")), None);
        // A renamed copy is not recognized, which is the honest answer: the table names files.
        assert_eq!(measured_level(Path::new("sc55.sf2")), None);
    }

    /// The reference a media song is levelled to, and the one place it differs from the rule above.
    ///
    /// **An unmeasured bank falls back here where `measured_level` abstains**, and the contrast is
    /// the whole point of having two functions rather than one. Abstaining on a `music_volume` means
    /// *leave the owner's level alone*, which is right. Abstaining on a reference would mean *do not
    /// level anything*, which is the complaint the feature exists to answer — so this returns a
    /// figure in the right region instead.
    #[test]
    fn the_reference_follows_the_bank_and_falls_back_rather_than_abstaining() {
        assert_eq!(
            reference_lufs(Path::new("/var/lib/karaoke/soundfonts/GeneralUser-GS.sf2")),
            -21.9
        );
        // A bank measured much louder than the bundled one moves the reference with it, which is
        // what stops a bank swap leaving every video 9 dB out.
        assert_eq!(
            reference_lufs(Path::new("D:/banks/Soundfont_SOMSAK_2016-V2.8.SF2")),
            -12.4
        );
        // Somebody's own bank, and a renamed copy, both fall back rather than abstaining.
        assert_eq!(
            reference_lufs(Path::new("SomebodysOwnBank.sf2")),
            km_loudness::DEFAULT_REFERENCE_LUFS
        );
        assert_eq!(
            reference_lufs(Path::new("generaluser.sf2")),
            km_loudness::DEFAULT_REFERENCE_LUFS
        );
    }

    /// What a real video is attenuated by, end to end, through the numbers this machine holds.
    ///
    /// Not arithmetic for its own sake: it is the one place the two halves meet — a bank's measured
    /// loudness from the table and a song's from a manifest — and the assertion is that a
    /// commercially mastered karaoke video comes down by about the gap the complaint described.
    #[test]
    fn a_commercial_video_is_brought_down_to_the_bundled_banks_level() {
        let reference = reference_lufs(Path::new("GeneralUser-GS.sf2"));

        // A loud karaoke master. 10.9 dB above the reference, so about 0.285 of full scale.
        let hot = km_loudness::gain_for(reference, -11.0);
        assert!((hot - 0.285).abs() < 0.01, "a -11 LUFS video got {hot}");

        // A quieter one, and a video already below the reference, which is left exactly alone.
        assert!((km_loudness::gain_for(reference, -16.0) - 0.507).abs() < 0.01);
        assert_eq!(km_loudness::gain_for(reference, -25.0), 1.0);
    }

    /// The rule that keeps one bank's reduction off the next bank.
    #[test]
    fn a_bank_with_no_level_of_its_own_puts_the_machine_s_level_back() {
        // A measured bank uses its own level, whatever the machine is set to.
        assert_eq!(volume_for_bank(Some(0.6), 1.0), 0.6);
        assert_eq!(volume_for_bank(Some(0.6), 0.9), 0.6);
        // A bank with no level returns the machine's own, which is the whole fix: the previous
        // bank's 0.6 must not survive the swap just because this slot asks for nothing.
        assert_eq!(volume_for_bank(None, 1.0), 1.0);
        assert_eq!(volume_for_bank(None, 0.9), 0.9);
        // Slot 1 is the case that matters most -- it is the fixed reference, and it never carries a
        // level of its own, so before this it inherited whatever was pressed before it.
        assert_eq!(volume_for_bank(None, 1.0), 1.0);
        // Clamped on both sides, because a hand-edited settings file or slot spec reaches here.
        assert_eq!(volume_for_bank(Some(1.4), 1.0), 1.0);
        assert_eq!(volume_for_bank(Some(-0.2), 1.0), 0.0);
        assert_eq!(volume_for_bank(None, 1.9), 1.0);
    }

    #[test]
    fn the_label_leads_with_the_slot_and_says_when_a_bank_is_only_pending() {
        assert_eq!(soundfont_label(1, 5, "bundled", false), "sf 1/5 bundled");
        assert_eq!(
            soundfont_label(3, 5, "musescore", true),
            "sf 3/5 musescore — from the next MIDI song"
        );
        // The suffix is the only difference between the two, so the same bank reads the same way
        // once it is actually sounding.
        assert_eq!(
            soundfont_label(3, 5, "musescore", false),
            "sf 3/5 musescore"
        );
    }

    /// The shape that put a bare colon on a television: the reason is stored beside the path, so it
    /// must not repeat it. These are the real `PackageError` spellings, both joins.
    #[test]
    fn a_stored_reason_does_not_repeat_the_path_it_sits_beside() {
        // At the length one really is, because the length is the point: a path this long is a single
        // unbreakable word to the notice's wrap, so a second copy of it took the reason off screen.
        let path = r"D:\tunes\karaoke\build\scratch\fx.kmpkg";
        assert_eq!(
            reason_without_path(
                path,
                &format!("{path}: The system cannot find the file specified. (os error 2)")
            ),
            "The system cannot find the file specified. (os error 2)"
        );
        assert_eq!(
            reason_without_path(
                path,
                &format!("{path} is not a readable package: bad signature")
            ),
            "is not a readable package: bad signature"
        );
    }

    /// The same package under two names goes in once, and the copy that is skipped is the later one.
    ///
    /// This is the case a path-keyed dedupe cannot see, and it is the one that happens: one volume
    /// in the packages folder and the same volume in a `package_dirs` folder, or one of the `-2`
    /// copies `dropped::free_name` mints. Installing it twice is *correct* and merely re-indexes
    /// every song in it for nothing, at every start, without saying so.
    #[test]
    fn the_same_package_under_two_names_is_installed_once() {
        let first = PathBuf::from(r"D:\tunes\karaoke\vol1.kmpkg");
        let second = PathBuf::from(r"D:\tunes\elsewhere\vol1-2.kmpkg");
        let plan = startup_plan(vec![first.clone(), second], |_| Some("vol1".to_owned()));
        assert_eq!(
            plan,
            vec![first],
            "the first spelling wins and the second is skipped"
        );
    }

    /// A `debug.packages` entry naming a file that the scan also finds installs once, not twice.
    ///
    /// Naming a file that is already in the packages folder is the easy mistake with that list, so
    /// it is an ordinary shape rather than a corner case.
    #[test]
    fn a_debug_path_that_is_also_in_the_packages_folder_is_installed_once() {
        let named = PathBuf::from(r"D:\tunes\karaoke\vol1.kmpkg");
        let plan = startup_plan(vec![named.clone(), named.clone()], |_| {
            Some("vol1".to_owned())
        });
        assert_eq!(plan, vec![named], "one path named twice is one package");
    }

    /// The debug list is offered before the scan, so an entry there decides which copy installs.
    #[test]
    fn the_debug_list_goes_first_and_the_scan_cannot_displace_it() {
        let debug = PathBuf::from(r"D:\builds\vol1.kmpkg");
        let scanned = PathBuf::from(r"D:\tunes\karaoke\vol1.kmpkg");
        let plan = startup_plan(vec![debug.clone(), scanned], |_| Some("vol1".to_owned()));
        assert_eq!(plan, vec![debug]);
    }

    /// A file that will not open is still offered, so that one place records the fault.
    ///
    /// Dropping it here would lose it silently: [`Machine::install_and_report`] is what turns a
    /// refusal into a note above the title, and it cannot do that for something it never saw.
    #[test]
    fn a_package_that_will_not_open_is_still_offered() {
        let broken = PathBuf::from(r"D:\tunes\karaoke\truncated.kmpkg");
        let plan = startup_plan(vec![broken.clone()], |_| None);
        assert_eq!(plan, vec![broken]);
    }

    /// Two packages with different ids both go in — so the tests above are not passing vacuously.
    #[test]
    fn two_different_packages_both_install() {
        let first = PathBuf::from(r"D:\tunes\karaoke\vol1.kmpkg");
        let second = PathBuf::from(r"D:\tunes\karaoke\vol2.kmpkg");
        let plan = startup_plan(vec![first.clone(), second.clone()], |path| {
            Some(path.file_stem()?.to_string_lossy().into_owned())
        });
        assert_eq!(plan, vec![first, second]);
    }

    /// The common rescan — somebody added a package — is never gated on anything.
    ///
    /// This is the case the whole split exists for. A rescan that removes nothing cannot disturb a
    /// queue, so making it wait for an idle machine would refuse the feature at exactly the moment
    /// it is wanted: a party, with a stick just handed over.
    #[test]
    fn an_empty_prune_set_needs_no_gate_even_while_playing() {
        let (prune, deferred) = prunable(Vec::new(), true);
        assert!(prune.is_empty());
        assert!(
            deferred.is_empty(),
            "nothing to defer and nothing to refuse"
        );
    }

    /// A busy machine defers removals rather than refusing the rescan.
    ///
    /// Not a 409: the additive half succeeded, and telling the caller nothing happened when
    /// something did is worse than telling it what is still outstanding.
    #[test]
    fn a_busy_machine_defers_removals_rather_than_refusing() {
        let (prune, deferred) = prunable(vec!["vol1".to_owned()], true);
        assert!(prune.is_empty(), "nothing is taken away under a live queue");
        assert_eq!(
            deferred,
            vec!["vol1".to_owned()],
            "and the caller is told so"
        );
    }

    /// An idle machine prunes there and then, exactly as a restart would.
    #[test]
    fn an_idle_machine_prunes_live() {
        let (prune, deferred) = prunable(vec!["vol1".to_owned(), "vol2".to_owned()], false);
        assert_eq!(prune, vec!["vol1".to_owned(), "vol2".to_owned()]);
        assert!(deferred.is_empty());
    }

    /// A reason that is about something other than this file is left exactly as it is — a bank
    /// collision names two packages and neither join is at the front.
    #[test]
    fn a_reason_that_does_not_name_the_path_is_untouched() {
        let reason = "every bank from 1 to 999 is taken";
        assert_eq!(reason_without_path("/songs/vol2.kmpkg", reason), reason);
        // And the degenerate case: stripping must never leave an empty sentence behind.
        assert_eq!(
            reason_without_path("/songs/vol2.kmpkg", "/songs/vol2.kmpkg"),
            "/songs/vol2.kmpkg"
        );
    }

    /// A package that asks for a free bank gets it, which is the case that makes numbers the same
    /// on every machine.
    #[test]
    fn a_free_bank_is_given_to_whoever_asks_for_it() {
        assert_eq!(choose_bank(831, &held(&[])), Some(831));
        assert_eq!(choose_bank(831, &held(&[1, 2, 611])), Some(831));
    }

    /// A tie sends the loser to the *next* bank, not to the bottom of the range.
    #[test]
    fn a_taken_bank_probes_forward_rather_than_falling_to_the_lowest_free() {
        assert_eq!(choose_bank(500, &held(&[500])), Some(501));
        assert_eq!(choose_bank(500, &held(&[500, 501, 502])), Some(503));
        // The point of probing: bank 1 is free throughout and is deliberately not the answer.
        assert!(!held(&[500, 501, 502]).contains(&1));
    }

    #[test]
    fn probing_wraps_round_the_end_of_the_range() {
        let taken = held(&[km_songcode::MAX_BANK]);
        assert_eq!(choose_bank(km_songcode::MAX_BANK, &taken), Some(1));
    }

    /// Bank 0 is the machine's own and nothing here may hand it out.
    ///
    /// Asserted over the whole range rather than for one input, because the failure would be a
    /// silent one: a package would simply be sitting on the three-digit numbers.
    #[test]
    fn bank_zero_is_never_given_out_however_full_the_machine_is() {
        let everything: BTreeSet<u16> = (1..km_songcode::MAX_BANK).collect();
        assert_eq!(
            choose_bank(1, &everything),
            Some(km_songcode::MAX_BANK),
            "the last free bank is found by wrapping, and it is not 0"
        );
        for wanted in [0, 1, 500, km_songcode::MAX_BANK] {
            let chosen = choose_bank(wanted, &held(&[])).expect("a bank");
            assert_ne!(chosen, 0, "wanted {wanted}");
        }
    }

    /// Every assignable bank held is the one refusal left, and the count is what the message says.
    #[test]
    fn a_machine_holding_every_assignable_bank_has_none_to_give() {
        let full: BTreeSet<u16> = (1..=km_songcode::MAX_BANK).collect();
        assert_eq!(full.len(), usize::from(km_songcode::MAX_BANK));
        assert_eq!(choose_bank(1, &full), None);
        assert_eq!(choose_bank(742, &full), None);
        // ...and bank 0 being free is not a way out: it is not this function's to give.
        assert!(!full.contains(&0));
    }

    /// The two halves agree: what a package asks for is what it is given on an empty machine.
    ///
    /// This is the property that was broken — the book derived a bank from the id and the machine
    /// did not — so it is worth pinning end to end rather than trusting the two call sites to stay
    /// in step.
    #[test]
    fn what_a_package_asks_for_is_what_an_empty_machine_gives_it() {
        for id in ["brasil-vol1", "brasil-vol2", "ingles", "a", ""] {
            let meta = km_kmpkg::PackageMeta {
                id: id.to_owned(),
                name: id.to_owned(),
                version: "1.0.0".to_owned(),
                publisher: None,
                created: None,
                volume: None,
            };
            assert_eq!(
                choose_bank(meta.wanted_bank(), &held(&[])),
                Some(meta.wanted_bank()),
                "id {id:?}"
            );
        }
    }

    #[test]
    fn a_timestamp_looks_like_an_iso_8601_instant() {
        let stamp = timestamp();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'));
        assert_eq!(stamp.as_bytes()[4], b'-');
        assert_eq!(stamp.as_bytes()[10], b'T');
        // Somewhere this side of 2020 and the other side of 2100, which is all this needs to be.
        let year: i32 = stamp[..4].parse().expect("a year");
        assert!((2020..2100).contains(&year), "{stamp}");
    }

    #[test]
    fn known_dates_convert_correctly() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        // A leap day, which is where a hand-rolled conversion goes wrong.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_688), (2026, 8, 23));
    }

    // -- staged auditions ------------------------------------------------------------------------

    /// A scratch root under the system temporary directory, removed when the test ends.
    fn audition_scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("km-audition-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    fn staged(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("staged folder");
        std::fs::write(dir.join("song.kar"), b"MThd").expect("staged file");
        dir
    }

    /// Sorted by name, and the names are nanosecond stamps — so this is age order.
    #[test]
    fn staging_folders_come_back_oldest_first() {
        let root = audition_scratch("order");
        staged(&root, "000000000000000000000000000000000000002");
        staged(&root, "000000000000000000000000000000000000001");
        let dirs = audition_dirs(&root);
        assert_eq!(dirs.len(), 2);
        assert!(dirs[0].ends_with("000000000000000000000000000000000000001"));
        assert_eq!(newest_audition(&root), Some(dirs[1].clone()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **What the old "keep the newest" rule could not express.** The folder that stays is the one
    /// being played, even when it is the oldest on disk — and the newest goes, because an audition
    /// that has finished can never be played again.
    #[test]
    fn a_sweep_keeps_the_audition_that_is_playing_and_takes_the_rest() {
        let root = audition_scratch("sweep-playing");
        let playing = staged(&root, "000000000000000000000000000000000000001");
        let older = staged(&root, "000000000000000000000000000000000000002");
        let newest = staged(&root, "000000000000000000000000000000000000003");

        assert_eq!(sweep_auditions(&root, std::slice::from_ref(&playing)), 0);

        assert!(playing.exists(), "the song being played must stay");
        assert!(!older.exists());
        assert!(!newest.exists(), "newest is not a reason to keep it");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Going idle keeps nothing: the machine is not reading any of them.
    #[test]
    fn a_sweep_with_nothing_playing_takes_every_staged_audition() {
        let root = audition_scratch("sweep-idle");
        staged(&root, "000000000000000000000000000000000000001");
        staged(&root, "000000000000000000000000000000000000002");

        assert_eq!(sweep_auditions(&root, &[]), 0);

        assert!(root.exists(), "the root itself is not the scratch");
        assert_eq!(audition_dirs(&root).len(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **The regression this design is built around.** A curator sends a second video while the
    /// first is still playing; the first ends mid-upload, and the folder being written into must
    /// survive a sweep that is otherwise right to take everything.
    #[test]
    fn an_upload_still_arriving_is_not_swept() {
        let root = audition_scratch("sweep-arriving");
        let playing = staged(&root, "000000000000000000000000000000000000001");
        let finished = staged(&root, "000000000000000000000000000000000000002");
        let arriving = staged(&root, "000000000000000000000000000000000000003");

        sweep_auditions(&root, &[playing.clone(), arriving.clone()]);

        assert!(playing.exists());
        assert!(!finished.exists());
        assert!(arriving.exists(), "an upload in flight must not be deleted");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Which folder a loaded song pins, by rule rather than by anything remembered.
    #[test]
    fn the_folder_of_the_loaded_song_is_the_one_that_is_kept() {
        let root = Path::new("/data/auditions");
        let file = |path: &str| Origin::File {
            path: path.to_owned(),
        };

        assert_eq!(
            audition_in_use(Some(&file("/data/auditions/0001/song.kar")), root),
            Some(PathBuf::from("/data/auditions/0001")),
            "a staged file pins the folder it is in"
        );
        for elsewhere in [
            // Loose in the root: not a staging folder, and nothing to keep.
            "/data/auditions/song.kar",
            // Deeper than a staging folder, so no folder directly under the root is implicated.
            "/data/auditions/0001/sub/song.kar",
            // `--play` and `debug.play_file` reach files that have nothing to do with auditions.
            "/tunes/karaoke/song.kar",
        ] {
            assert_eq!(
                audition_in_use(Some(&file(elsewhere)), root),
                None,
                "{elsewhere} should pin nothing"
            );
        }
        assert_eq!(
            audition_in_use(
                Some(&Origin::Catalog {
                    number: SongCode::new(1019),
                    entry_id: 1,
                }),
                root
            ),
            None
        );
        assert_eq!(
            audition_in_use(
                Some(&Origin::Demo {
                    number: SongCode::new(1019),
                }),
                root
            ),
            None
        );
        assert_eq!(audition_in_use(None, root), None, "idle pins nothing");
    }

    /// A sweep that cleared everything is the end of it; one that refused looks again, until it has
    /// asked as often as it is allowed to and the backstops take over.
    #[test]
    fn a_refused_sweep_looks_again_until_its_budget_is_spent() {
        let now = Instant::now();

        assert!(
            next_sweep(AUDITION_TRIES, 0, now).is_none(),
            "nothing stuck, nothing to come back for"
        );
        assert!(
            next_sweep(0, 1, now).is_none(),
            "a spent budget stops asking"
        );

        let again = next_sweep(3, 1, now).expect("a stuck folder is worth another look");
        assert_eq!(again.tries, 2, "each look costs one");
        assert_eq!(again.due, now + AUDITION_RETRY);
    }

    /// Nothing staged for an audition outlives the run that played it.
    #[test]
    fn a_purge_takes_the_whole_folder_including_the_last_audition() {
        let root = audition_scratch("purge");
        staged(&root, "000000000000000000000000000000000000001");
        staged(&root, "000000000000000000000000000000000000002");

        purge_auditions(&root);

        assert!(!root.exists());
    }

    /// A machine that has never taken an upload has no folder, and starting must not care.
    #[test]
    fn purging_a_folder_that_was_never_made_is_not_an_error() {
        let root = std::env::temp_dir().join("km-audition-test-absent");
        let _ = std::fs::remove_dir_all(&root);
        purge_auditions(&root);
        assert!(!root.exists());
    }

    /// The containment rule, which is the whole reason `play_audition` takes a name and not a path.
    ///
    /// **The backslash case is split by platform, and that is the rule working rather than a hole in
    /// it.** `\` is a separator on Windows and an ordinary filename character everywhere else, so
    /// `..\song.kar` is a traversal attempt on one platform and a single, legal, if peculiar, file
    /// name on the other. [`Path::file_name`] answers each correctly, and so does the check in
    /// `play_audition` that calls it: joining a one-component name onto the staging folder cannot
    /// leave that folder, whatever characters the component is spelled with. Asserting the Windows
    /// answer unconditionally is what one flat list did, and it failed on the other two platforms
    /// for a property neither of them has.
    #[test]
    fn only_a_bare_name_is_a_staged_audition() {
        for bare in ["song.kar", "Sultans of Swing.mp3", "x.cdg"] {
            assert_eq!(
                Path::new(bare).file_name(),
                Some(bare.as_ref()),
                "{bare} should be bare"
            );
        }
        for path in ["../song.kar", "sub/song.kar", "/etc/passwd", "..", ""] {
            assert_ne!(
                Path::new(path).file_name(),
                Some(path.as_ref()),
                "{path} should not pass as a bare name"
            );
        }

        // Refused on Windows, where it reads as `..` and then a file in it.
        #[cfg(windows)]
        assert_ne!(
            Path::new("..\\song.kar").file_name(),
            Some("..\\song.kar".as_ref()),
            "..\\song.kar should not pass as a bare name on Windows"
        );
        // Bare everywhere else, where it is one file whose name happens to contain a backslash --
        // and taking it is correct, because it still cannot address anything outside the folder.
        #[cfg(not(windows))]
        assert_eq!(
            Path::new("..\\song.kar").file_name(),
            Some("..\\song.kar".as_ref()),
            "..\\song.kar is a bare name where the backslash is not a separator"
        );
    }

    // ---- the load-next-song transaction ------------------------------------------------------
    //
    // Everything below builds a whole `Machine` over a scratch tree, because a transaction is what
    // these test. Extracting a rule into a pure function and testing it is the right way to test a
    // *rule*, and it is not a way to test a *transaction* -- which is where the race in `advance`
    // hid.
    //
    // These are slow by the standards of the tests above -- a real package on disk, a real SQLite
    // catalog, real MIDI parsed out of an archive. That is the point. The thing being tested is
    // what happens while a load is in flight, and a load that costs nothing has no window to race
    // inside.

    use crate::engine::CommandLog;

    /// The eight synthetic fixtures these tests build a package from.
    ///
    /// Distinct contents rather than eight copies of one, so the install reports no duplicates and
    /// a failure here cannot be an artifact of two songs hashing the same.
    fn fixtures() -> [Vec<u8>; 8] {
        [
            km_song::testing::soft_karaoke(),
            km_song::testing::soft_karaoke_real_layout(),
            km_song::testing::lyric_events(),
            km_song::testing::named_text_track(),
            km_song::testing::unmarked_lyrics(),
            km_song::testing::tempo_change(),
            km_song::testing::velocity_zero_note_offs(),
            km_song::testing::melody_and_accompaniment(),
        ]
    }

    /// A song entry with the fields none of these tests care about filled in.
    fn an_entry(number: u32) -> km_kmpkg::SongEntry {
        km_kmpkg::SongEntry {
            number,
            kind: km_kmpkg::SongKind::Midi,
            title: format!("Song {number}"),
            artist: Some("A Singer".to_owned()),
            language: Some("eng".to_owned()),
            // Both filled in by `PackageBuilder::add`.
            file: String::new(),
            content_hash: None,
            duration_ms: 200_000,
            lyric_encoding: None,
            default_transpose: 0,
            lyrics_hidden: false,
            fixes: Vec::new(),
            melody: None,
            melody_abstained: None,
            suitability: None,
            lyric_preview: Vec::new(),
            tags: Vec::new(),
            loudness: None,
            edited: Vec::new(),
        }
    }

    /// A machine over a scratch tree with eight playable songs, in the bank the package id implies.
    ///
    /// [`Engine::recording`] rather than [`Engine::silent`] for the reason that constructor's own
    /// doc gives: a silent engine's `can_play` is false, so `queue_add` never reaches `advance` and
    /// a test built on one proves nothing about the queue at all.
    fn a_machine_with_eight_songs(name: &str) -> (Arc<Machine>, CommandLog, Vec<SongCode>) {
        let root = std::env::temp_dir().join(format!("km-machine-tests-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a scratch tree");

        let package_path = root.join("vol1.kmpkg");
        let mut builder = km_kmpkg::PackageBuilder::new(km_kmpkg::PackageMeta {
            id: km_kmpkg::EXAMPLE_ID.to_owned(),
            name: "Volume One".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            created: None,
            volume: None,
        });
        for (index, midi) in fixtures().into_iter().enumerate() {
            let number = u32::try_from(index).expect("eight fits") + 1;
            builder.add(an_entry(number), midi).expect("add a song");
        }
        builder.write(&package_path).expect("write the package");

        let (engine, log) = Engine::recording();
        let machine = Arc::new(
            Machine::new(
                Paths::rooted_at(&root),
                Settings::default(),
                engine,
                Events::new(),
            )
            .expect("build a machine"),
        );
        let report = machine.install(&package_path).expect("install the package");
        assert_eq!(report.songs_added, 8, "all eight songs should install");

        // What an empty machine gives the package is the bank its id implies: `choose_bank` hands back
        // what was asked for when nothing holds it.
        let bank = km_kmpkg::PackageMeta::suggested_bank(km_kmpkg::EXAMPLE_ID);
        let codes = (1..=8)
            .map(|slot| SongCode::in_bank(bank, slot).expect("a code in the package's bank"))
            .collect();
        (machine, log, codes)
    }

    fn a_request(number: SongCode) -> QueueRequest {
        QueueRequest {
            number,
            title: format!("Song {}", number.slot()),
            artist: Some("A Singer".to_owned()),
            singer: None,
        }
    }

    /// Nothing is loaded and nothing is queued, so the first song queued should take the deck.
    #[test]
    fn queueing_a_song_on_an_idle_machine_starts_it() {
        let (machine, log, codes) = a_machine_with_eight_songs("idle-start");

        assert!(machine.lock_state().loaded.is_none(), "starts idle");
        machine.queue_add(a_request(codes[0])).expect("queue");

        assert!(
            machine.lock_state().loaded.is_some(),
            "queueing on an idle machine should start the song"
        );
        assert_eq!(log.count("Load"), 1, "exactly one song should be loaded");
        assert!(machine.queue().is_empty(), "and it should leave the queue");
    }

    /// **A song starting asks for a new picture, and asks once per song.**
    ///
    /// Through the same flag `POST /wallpapers/next` sets, which is what `take_wallpaper_request`
    /// drains — so this asserts the trigger reaches the one place the display looks, rather than
    /// asserting a field that nothing reads. Two songs, because a trigger that only fired for the
    /// first song of an evening would pass a one-song test.
    #[test]
    fn a_song_starting_asks_for_a_new_picture() {
        let (machine, _log, codes) = a_machine_with_eight_songs("wallpaper-per-song");

        assert!(
            !machine.take_wallpaper_request(),
            "an idle machine has asked for nothing"
        );

        machine.queue_add(a_request(codes[0])).expect("queue one");
        assert!(
            machine.take_wallpaper_request(),
            "the song that took the deck should have asked for a picture"
        );
        assert!(
            !machine.take_wallpaper_request(),
            "and asked once, so the display does not advance the playlist twice"
        );

        // A second `start` on a machine that has already played.
        machine.queue_add(a_request(codes[1])).expect("queue two");
        Controller::transport(machine.as_ref(), TransportCommand::Skip)
            .expect("skip to the second song");
        assert!(
            machine.take_wallpaper_request(),
            "and so should the song after it"
        );
    }

    /// **`wallpaper.on_song_change` turned off leaves the picture to the interval.**
    #[test]
    fn a_machine_told_not_to_change_the_picture_per_song_does_not() {
        let (machine, _log, codes) = a_machine_with_eight_songs("wallpaper-per-song-off");
        machine.lock_settings().wallpaper.on_song_change = false;

        machine.queue_add(a_request(codes[0])).expect("queue one");
        assert!(
            !machine.take_wallpaper_request(),
            "the setting is off, so only the interval changes the picture"
        );
    }

    /// **Every song start sends the levelling gain, and a MIDI song sends `1.0`.**
    ///
    /// This is the subtlest way the feature can break, and it breaks quietly. `Sticky` replays the
    /// last gain it saw into every stream it builds, so a start that sent nothing would leave the
    /// previous song's attenuation in place — and a MIDI song following a commercially mastered
    /// video would play 16 dB quiet for its whole length with nothing in the log and nothing on the
    /// screen. Two songs, so the second start is covered as well as the first: a send that only
    /// happened for the first song of an evening would pass a one-song test.
    ///
    /// The value is asserted through the command's `Debug` text, which is what this log records.
    #[test]
    fn every_song_start_sends_a_levelling_gain_and_a_midi_song_sends_one() {
        let (machine, log, codes) = a_machine_with_eight_songs("song-gain-sent");

        machine.queue_add(a_request(codes[0])).expect("queue one");
        assert_eq!(
            log.count("SetSongGain(1.0)"),
            1,
            "a MIDI song is the reference, so it must be sent unattenuated rather than not sent"
        );

        // Skip to the next song, which is a second `start` on a machine that has already played.
        machine.queue_add(a_request(codes[1])).expect("queue two");
        Controller::transport(machine.as_ref(), TransportCommand::Skip)
            .expect("skip to the second song");
        assert_eq!(
            log.count("SetSongGain(1.0)"),
            2,
            "the second start must send it too"
        );
        assert_eq!(
            log.count("SetSongGain"),
            2,
            "and nothing else may have sent a different gain"
        );
    }

    /// **What the panel draws is what the engine was told**, which is the whole property it rests
    /// on.
    ///
    /// A test asserting only the derivation would pass while the number drifted, so this reads the
    /// recorded gain back and checks it against the command log — the same `SetSongGain` the
    /// audio path is actually applying. These test machines have no bank, so nothing can be
    /// measured and the gain is unity; the assertion that matters is that the two agree.
    #[test]
    fn the_gain_recorded_for_the_panel_is_the_gain_the_engine_was_sent() {
        let (machine, log, codes) = a_machine_with_eight_songs("song-gain-recorded");

        assert!(
            machine.song_levelling().is_none(),
            "nothing is loaded, so there is nothing to describe"
        );

        machine.queue_add(a_request(codes[0])).expect("queue one");
        let (gain, source, bank_ignored, muted) =
            machine.song_levelling().expect("a song is loaded");
        assert_eq!(
            log.count(&format!("SetSongGain({gain:?})")),
            1,
            "the recorded gain must be the one the engine was sent"
        );
        assert!(
            matches!(
                source,
                km_display::GainSource::Unmeasured | km_display::GainSource::Disabled
            ),
            "with no bank there is nothing to level against, so it must not claim a measurement"
        );
        assert_eq!(
            (bank_ignored, muted),
            (0, 0),
            "these fixtures carry no stored correction"
        );
    }

    /// The deck is occupied, so the second song waits rather than displacing the first.
    #[test]
    fn a_second_queued_song_waits_its_turn() {
        let (machine, log, codes) = a_machine_with_eight_songs("waits-its-turn");

        machine.queue_add(a_request(codes[0])).expect("queue one");
        machine.queue_add(a_request(codes[1])).expect("queue two");

        assert_eq!(
            log.count("Load"),
            1,
            "the second song should not have been loaded over the first"
        );
        assert_eq!(machine.queue().len(), 1, "it should be waiting");
    }

    /// A demo song stands aside for a queued song; a person's song does not.
    ///
    /// **Both halves in one test, because the line between them is the rule.** A machine singing to
    /// itself has no turn to lose, so queueing takes the deck off it at once; somebody at a
    /// microphone does, so queueing behind them waits. A test asserting only the first passes just
    /// as well on a machine that interrupts a singer, which is the fault worth being unable to
    /// ship.
    ///
    /// It drives the demo the way the machine does — the one-shot flag, then a poll — rather than
    /// reaching for `start_demo`, so the arrangement under test is the one that runs.
    #[test]
    fn queueing_during_a_demo_takes_the_deck_and_queueing_over_a_singer_does_not() {
        let (machine, log, codes) = a_machine_with_eight_songs("demo-yields");
        let mut events = machine.events.subscribe();

        machine.lock_state().demo_once = true;
        machine.poll();
        assert!(
            matches!(
                machine.lock_state().loaded.as_ref().map(|it| &it.origin),
                Some(Origin::Demo { .. })
            ),
            "the harness did not get a demo song onto the deck, so nothing below is being tested"
        );
        let loads = log.count("Load");

        machine
            .queue_add(a_request(codes[0]))
            .expect("queue over the demo");
        assert!(
            matches!(
                machine.lock_state().loaded.as_ref().map(|it| &it.origin),
                Some(Origin::Catalog { .. })
            ),
            "the queued song should have taken the deck from the demo"
        );
        assert_eq!(
            log.count("Load"),
            loads + 1,
            "and it should have been loaded exactly once"
        );
        assert!(machine.queue().is_empty(), "so nothing is left waiting");

        // The demo's ending has to go out, and in a word no other ending uses: a remote that heard
        // `Skipped` here would tell a room somebody's turn had been taken away.
        let mut yielded = false;
        while let Ok(event) = events.try_recv() {
            if matches!(
                event,
                Event::SongEnded {
                    reason: EndReason::Yielded
                }
            ) {
                yielded = true;
            }
            assert!(
                !matches!(
                    event,
                    Event::SongEnded {
                        reason: EndReason::Skipped
                    }
                ),
                "nobody skipped anything"
            );
        }
        assert!(yielded, "the demo ended and no `Yielded` was published");

        // And now a person is singing, so the next one waits exactly as it always did.
        machine
            .queue_add(a_request(codes[1]))
            .expect("queue behind the singer");
        assert_eq!(
            log.count("Load"),
            loads + 1,
            "nothing may be loaded over somebody's turn"
        );
        assert_eq!(machine.queue().len(), 1, "it waits");
    }

    /// Skipping a demo asks for a different song; skipping your own song asks to be left alone.
    ///
    /// **Both halves in one test, because the line between them is the rule.** Nobody's turn ends
    /// when a demo is skipped, so the next one follows with no gap; a singer who skips their own
    /// song has ended a turn, and the machine owes them the full silence. A test asserting only the
    /// first passes just as well on a machine that talks over the person who pressed the key.
    ///
    /// It drives the demo the way the machine does — the one-shot flag, then a poll — so the
    /// arrangement under test is the one that runs.
    #[test]
    fn skipping_a_demo_starts_another_and_skipping_a_singers_song_does_not() {
        let (machine, log, codes) = a_machine_with_eight_songs("demo-skip-chains");
        machine.lock_state().demo_enabled = true;

        machine.lock_state().demo_once = true;
        machine.poll();
        assert!(
            matches!(
                machine.lock_state().loaded.as_ref().map(|it| &it.origin),
                Some(Origin::Demo { .. })
            ),
            "the harness did not get a demo song onto the deck, so nothing below is being tested"
        );
        let loads = log.count("Load");

        Controller::transport(machine.as_ref(), TransportCommand::Skip).expect("skip the demo");
        machine.poll();
        assert!(
            matches!(
                machine.lock_state().loaded.as_ref().map(|it| &it.origin),
                Some(Origin::Demo { .. })
            ),
            "a skipped demo must be followed by another one"
        );
        assert_eq!(
            log.count("Load"),
            loads + 1,
            "and by exactly one, on the poll after the press"
        );

        // Now a person's song, which ends a turn. The queue behind it is empty, so the only thing
        // that could start is a demo — and it must not, for a full delay.
        machine
            .queue_add(a_request(codes[0]))
            .expect("queue a song");
        assert!(
            matches!(
                machine.lock_state().loaded.as_ref().map(|it| &it.origin),
                Some(Origin::Catalog { .. })
            ),
            "the queued song should have taken the deck from the demo"
        );
        Controller::transport(machine.as_ref(), TransportCommand::Skip)
            .expect("skip the singer's song");
        machine.poll();
        assert!(
            machine.lock_state().loaded.is_none(),
            "skipping somebody's song buys the full silence back"
        );
    }

    /// Skip into silence asks demo mode for a song, and the clock has nothing to do with it.
    ///
    /// **The delay is asserted to be still running, which is what makes this a test of the press.**
    /// A machine that started a song here because its deadline happened to have passed would
    /// satisfy every assertion below while proving none of them.
    ///
    /// **`demo_once` is asserted before the poll**, because the ordering is what the arrangement
    /// rests on: `arm_demo` runs at the top of `transport` and clears a pending trigger, so a flag
    /// set any earlier than that arm would be wiped by it and the song would wait out the delay.
    #[test]
    fn a_skip_into_silence_starts_a_demo_when_the_mode_is_on() {
        let (machine, log, _codes) = a_machine_with_eight_songs("skip-into-silence");
        let mut events = machine.events.subscribe();
        machine.lock_state().demo_enabled = true;

        assert!(
            machine.lock_state().loaded.is_none(),
            "the deck has to be empty for this to be the case under test"
        );
        assert!(
            machine.lock_state().demo_resume_at > Instant::now(),
            "and the delay still running, or the poll below would start a song by itself"
        );
        let loads = log.count("Load");

        Controller::transport(machine.as_ref(), TransportCommand::Skip).expect("skip into silence");
        assert!(
            machine.lock_state().demo_once,
            "the press has to leave a trigger the arm above cannot clear"
        );

        machine.poll();
        assert!(
            matches!(
                machine.lock_state().loaded.as_ref().map(|it| &it.origin),
                Some(Origin::Demo { .. })
            ),
            "a skip into silence should be answered with a demo song"
        );
        assert_eq!(
            log.count("Load"),
            loads + 1,
            "and with exactly one, on the poll after the press"
        );

        // Nothing ended, so nothing may say one did.
        while let Ok(event) = events.try_recv() {
            assert!(
                !matches!(event, Event::SongEnded { .. }),
                "there was no song on the deck to end"
            );
        }
    }

    /// With the mode off the press is refused, in the code and the words every client renders.
    ///
    /// **The sentence is asserted beside the code**, because the two are separate contracts: the
    /// code is what a remote looks up in its own catalog, and the words are what the keypad prints
    /// where there is no catalog to look in.
    #[test]
    fn a_skip_into_silence_with_the_mode_off_says_nothing_is_playing() {
        let (machine, log, _codes) = a_machine_with_eight_songs("skip-into-silence-off");
        let loads = log.count("Load");

        let refused = Controller::transport(machine.as_ref(), TransportCommand::Skip)
            .expect_err("nothing to skip, and no mode to ask");
        assert!(
            matches!(
                &refused,
                ControlError::Unavailable(refusal)
                    if refusal.code == Some(NOTHING_PLAYING)
                        && refusal.message == "nothing is playing"
            ),
            "the code and the sentence every client already renders: {refused}"
        );

        assert!(
            !machine.lock_state().demo_once,
            "and no trigger left for the next poll to act on"
        );
        machine.poll();
        assert!(
            machine.lock_state().loaded.is_none(),
            "so the room stays quiet"
        );
        assert_eq!(log.count("Load"), loads, "and nothing was loaded");
    }

    /// A queue waiting on an empty deck refuses, and the remote's `▶ now` rests on that.
    ///
    /// **`km-remote-pages` sends `Skip` and falls back to `Play` when it is refused**, having just
    /// put a song at the front of the queue. A demo starting there would answer that handler with
    /// `Ok`, so the fallback would not run and the song somebody asked for would sit at the front
    /// under a `Playing now` badge. The queue is one of `why_no_demo`'s four conditions, which is
    /// why the press delegates to it rather than asking about the deck alone.
    #[test]
    fn a_skip_with_somebody_waiting_refuses_rather_than_starting_a_demo() {
        let (machine, log, codes) = a_machine_with_eight_songs("skip-over-a-queue");
        machine.lock_state().demo_enabled = true;

        machine.queue_add(a_request(codes[0])).expect("queue one");
        machine.queue_add(a_request(codes[1])).expect("queue two");
        // `go_idle` takes the loaded song and leaves the queue, which is the state the handler
        // reaches: nothing on the deck, and somebody's song at the front of it.
        Controller::transport(machine.as_ref(), TransportCommand::Stop).expect("stop the first");
        assert!(
            machine.lock_state().loaded.is_none(),
            "the deck should be empty"
        );
        assert!(
            !machine.lock_state().queue.is_empty(),
            "and the queue should not be"
        );
        let loads = log.count("Load");

        let refused = Controller::transport(machine.as_ref(), TransportCommand::Skip)
            .expect_err("a queue plays before anything the machine would choose");
        assert!(
            matches!(
                &refused,
                ControlError::Unavailable(refusal) if refusal.code == Some(NOTHING_PLAYING)
            ),
            "the refusal the remote falls back on: {refused}"
        );
        assert!(
            !machine.lock_state().demo_once,
            "and no trigger to talk over the queue on the next poll"
        );
        assert_eq!(log.count("Load"), loads, "nothing was loaded by the press");
    }

    /// A hand-pressed demo skipped with the mode off is still exactly one song.
    ///
    /// **`demo.enabled` is what buys the chaining**, and the trigger deliberately does not turn it
    /// on. So the deadline a skip leaves is irrelevant here: there is no mode to act on it, and the
    /// machine goes quiet the way it would have when the song ran out.
    #[test]
    fn a_hand_pressed_demo_skipped_with_the_mode_off_leaves_silence() {
        let (machine, _log, _codes) = a_machine_with_eight_songs("demo-skip-one-shot");

        machine.lock_state().demo_once = true;
        machine.poll();
        assert!(
            matches!(
                machine.lock_state().loaded.as_ref().map(|it| &it.origin),
                Some(Origin::Demo { .. })
            ),
            "the harness did not get a demo song onto the deck"
        );

        Controller::transport(machine.as_ref(), TransportCommand::Skip).expect("skip the demo");
        machine.poll();
        assert!(
            machine.lock_state().loaded.is_none(),
            "a trigger is one song, and skipping it does not make it two"
        );
    }

    /// Eight singers pressing a number at once must lose none of their songs.
    ///
    /// **This is the test the race was hiding behind.** `queue_add`'s guard reads
    /// `if self.lock_state().loaded.is_none()` -- and the guard the `if` takes is a temporary,
    /// dropped at the end of the condition, so `advance` re-locks to pop. Between the two, another
    /// thread runs the same check and gets the same answer. Both pop, both load, and the second
    /// `start` overwrites the first: one song is playing, one is gone, and nothing anywhere says
    /// so. The window is the whole of `load_from_catalog` -- an archive read and a MIDI parse.
    ///
    /// The assertion is conservation rather than a count of loads, because conservation is what a
    /// singer actually cares about: eight songs went in, so eight are either playing or waiting.
    /// It also holds whatever interleaving the scheduler picks, which a count of loads does not.
    #[test]
    fn eight_threads_queueing_at_once_lose_no_songs() {
        let (machine, _log, codes) = a_machine_with_eight_songs("no-lost-songs");
        let gate = Arc::new(std::sync::Barrier::new(codes.len()));

        let mut singers = Vec::new();
        for code in codes.iter().copied() {
            let machine = Arc::clone(&machine);
            let gate = Arc::clone(&gate);
            singers.push(std::thread::spawn(move || {
                gate.wait();
                machine.queue_add(a_request(code)).expect("queue");
            }));
        }
        for singer in singers {
            singer.join().expect("a singer's thread panicked");
        }

        let waiting = machine.queue().len();
        let playing = usize::from(machine.lock_state().loaded.is_some());
        assert_eq!(
            waiting + playing,
            8,
            "eight songs were queued; {waiting} are waiting and {playing} is playing, \
             so {} went missing between the guard and the pop",
            8 - (waiting + playing)
        );
    }

    /// A song's stored corrections reach the load command rather than being dropped on the way.
    ///
    /// The hop nothing else asserts. The catalog is tested for what it stores and the sequencer for
    /// what it does with a table, and between them sits one function that could hand the audio
    /// thread an empty one with everything still compiling and every other test still passing.
    #[test]
    fn a_songs_corrections_reach_the_load_command() {
        let song = Arc::new(
            Song::parse(
                &km_song::testing::kit_bank_on_a_melodic_channel(),
                &ParseOptions::default(),
            )
            .expect("fixture parses"),
        );
        let stored = [km_fixes::Fix::MuteChannel { channel: 2 }];
        let resolved = fixes_for(&stored, "a song");

        let (_, load) = split_media(LoadedMedia::Midi(song), Some(7), resolved);
        match load {
            km_audio::audio::Load::Midi {
                melody_channel,
                fixes,
                ..
            } => {
                assert_eq!(melody_channel, Some(7));
                assert!(fixes.mute[2], "the curator's mute did not reach the player");
            }
            km_audio::audio::Load::Track(_) => panic!("a MIDI song became a track"),
        }
    }

    /// A stored list is applied whole, including the half a detector would never propose.
    ///
    /// Filtering at playback would re-decide a question the package answered, and the first thing
    /// it would throw away is exactly the fix somebody set by hand.
    #[test]
    fn a_stored_list_is_not_filtered_by_whether_a_fix_applies_itself() {
        let stored = [
            km_fixes::Fix::IgnoreBankSelect { channel: 4 },
            km_fixes::Fix::MuteChannel { channel: 2 },
        ];
        let resolved = fixes_for(&stored, "a song");
        assert!(resolved.ignore_bank[4]);
        assert!(resolved.mute[2]);
    }

    /// A video song has no MIDI events, so nothing is carried and nothing is claimed.
    #[test]
    fn a_song_with_no_events_carries_no_corrections() {
        assert!(fixes_for(&[], "a video").is_empty());
    }
}
