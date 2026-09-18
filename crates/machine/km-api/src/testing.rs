//! An in-memory karaoke machine, for driving the API without hardware.
//!
//! Behind the `testing` feature so it is available to this crate's integration tests and to
//! `km-app`'s, but is not compiled into a release binary.
//!
//! It is a working machine rather than a set of canned answers: queueing then playing really does
//! take the song off the front of the queue and set `now_playing`, transposing really does change
//! what a later `GET /state` reports. That matters, because the bugs worth catching in an HTTP layer
//! are the ones where a handler reads the wrong thing or writes nothing at all, and a stub that
//! returns a fixed snapshot cannot see either.
//!
//! It also records what it was told to do, so a test can assert that a request reached the machine
//! and not merely that it returned 200.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, atomic};

use km_catalog::search::SearchQuery;
use km_catalog::{CatalogSong, DuplicateContent, InstallReport, InstalledPackage};
use km_queue::Transport;
use km_queue::mics::{MicBus, MicChannel, MicPatch, MicRegistry};
use km_queue::queue::{Queue, QueueEntry, QueueFull, QueueRequest};
use km_song::{ParseOptions, Song};
use km_songcode::SongCode;

use crate::machine::{
    AudioOutput, AudioOutputs, Audition, Catalog, CatalogError, ControlError, Controller,
    MAX_DEMO_DELAY_SECS, NowPlaying, Origin, OutputLevel, PackageProblem, Picture, Refusal,
    Settings, SettingsPatch, Snapshot, SoundFontChoice, SoundFontStatus, SoundKind,
    TransportCommand, WallpaperState,
};

/// What a curation tool had settled about a song, as the test machine received it.
///
/// The owned twin of [`crate::machine::Audition`], which borrows. Everything a test wants to assert
/// about a preview is here, so a route that quietly stopped carrying one field fails on the field
/// rather than on a shape.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decided {
    /// The title it was told to show.
    pub title: Option<String>,
    /// The performer it was told to show.
    pub artist: Option<String>,
    /// The key it was told to play in.
    pub transpose: Option<i8>,
    /// The corrections it was told to play with.
    pub fixes: Option<Vec<km_fixes::Fix>>,
    /// The melody channel it was told to play with, in [`Audition::melody`]'s three states.
    pub melody: Option<Option<u8>>,
    /// The UltraStar words it was told to play the audio with.
    pub lyrics: Option<km_song::LyricTimeline>,
}

/// Something the machine was asked to do.
///
/// Recorded in order so a test can assert on the sequence — that `POST /transport/skip` reached the
/// machine as a skip and not as a stop, for instance.
///
/// `PartialEq` without `Eq` since [`Recorded::PlayFixes`] arrived, for the reason
/// [`crate::dto::PlayFileRequest`] gives: an unreadable fix is held as JSON, and JSON has numbers in
/// it. Every assertion here is an equality or a `matches!`, and neither wants the stronger bound.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    /// A transport command arrived.
    Transport(TransportCommand),
    /// A file was asked to play directly.
    PlayFile(PathBuf),
    /// A staging folder was opened for an uploaded song.
    OpenAudition,
    /// An uploaded song was asked to play, by the bare name it was staged under.
    PlayAudition(String),
    /// Everything a curation tool had already settled about an audition, pushed beside the two
    /// above.
    ///
    /// A variant of its own rather than fields on each of those, so that a test which only cares
    /// which file played goes on matching one tuple. Each field is separately `None`, which is a
    /// caller that decided nothing — a different thing from a caller that decided on none.
    Decided(Decided),
    /// A package was installed from a path.
    Install(PathBuf),
    /// A package was uninstalled.
    Uninstall(String),
    /// A refused package's file was deleted, by the problem id naming it.
    DeletedProblemFile(String),
    /// The packages folders were asked to be read again.
    Rescan,
    /// The wallpaper was advanced by hand.
    NextWallpaper,
    /// A picture was removed from the folder, by id.
    DeleteWallpaper(String),
    /// The audio output device was changed.
    SetAudioOutput(String),
    /// The output's own level was moved, in hundredths of a decibel as the caller asked for it.
    ///
    /// **What was asked for, not where it landed.** A test asserting on this is asserting that the
    /// request reached the machine; what the hardware then did with it is the answer's business.
    SetOutputLevel(i32),
    /// The box was asked to power itself off.
    ShutDown,
    /// The application was asked to end so that a supervisor would start it again.
    RestartApplication,
    /// The session epoch was handed back for persisting.
    SessionEpochSet(u64),
    /// Debugging mode was turned on or off.
    DebugEnabledSet(bool),
    /// The development console switch was turned on or off.
    DevRemoteEnabledSet(bool),
    /// The frame-statistics panel was switched.
    PerformanceOverlaySet(bool),
    /// The admin password was set, or cleared when this is `None`.
    ///
    /// The hash and the factory PIN, in that order. The hash rather than the password, which is
    /// what crosses this seam -- and recording it at
    /// all is what stops the persisting half being deleted with every test still green, exactly as
    /// for the two rows below.
    AdminPasswordSet(Option<String>, Option<String>),
    /// An owner upload was accepted: what kind, and the file it landed as.
    UploadAccepted(crate::machine::Upload, String),
    /// The machine was renamed, and the name it was asked to write down.
    ///
    /// Recorded for `AclChanged`'s reason: the running name lives in `ApiState` and the durable one
    /// only reaches `settings.json` through the controller, so a fake that accepted a rename and
    /// forgot it would let the persisting half be deleted with every test still green.
    MachineRenamed(String),
    /// Demo mode was switched, and whether the switch was asked to stick.
    ///
    /// Both halves recorded, because `persist` is the only thing separating a change that lasts the
    /// evening from one that survives a power cut — a fake that dropped it would let the route stop
    /// forwarding it and no test would notice.
    SetDemo {
        /// What it was set to.
        enabled: bool,
        /// Whether it was asked to be written down.
        persist: bool,
    },
    /// The demo delay was set.
    ///
    /// One half, where [`Self::SetDemo`] has two, and the asymmetry is the route's: a delay is
    /// always written down, so there is no `persist` to have dropped.
    SetDemoDelay {
        /// What it was set to, in seconds.
        delay_secs: u32,
    },
    /// One demo song was asked for by hand.
    ///
    /// Recorded with nothing beside it because there is nothing to say: the route is bodyless, and
    /// what it does to this double is invisible — the real machine sets a flag its poll thread reads,
    /// and there is no poll thread here. So the record *is* the observation, and without it a route
    /// that silently stopped calling the controller would pass every test.
    DemoStarted,
}

/// How the machine should misbehave, so error paths are reachable from a test.
#[derive(Debug, Clone, Default)]
pub struct Faults {
    /// Search fails with this message.
    pub search: Option<String>,
    /// Loading a song's MIDI fails with this message.
    pub load: Option<String>,
    /// Installing fails with this message.
    pub install: Option<String>,
    /// Uninstalling fails this way.
    ///
    /// The **variant**, not a message, unlike every other field here: what this exists to test is
    /// which status a refusal turns into, and a `String` could only ever produce one of them.
    pub uninstall: Option<CatalogError>,
    /// Packages listed here answer `why_not_removable` with this sentence.
    ///
    /// Keyed by id, and separate from `uninstall` above: one is a route that refuses, the other is
    /// the question a page asks before offering the control. A test that wants both sides of a
    /// package the machine will not delete sets both, which is what the real machine does by
    /// asking one predicate twice.
    pub unremovable: Vec<(String, String)>,
    /// Refused packages whose **file** the machine will not delete, keyed by path.
    ///
    /// Separate from `unremovable` above rather than sharing it, and keyed differently on purpose:
    /// that one is keyed by the package id `uninstall` takes, and a refused package's control is
    /// named by a *derived* id instead. Keying this by path is what saves a test author from
    /// computing a fingerprint by hand to arrange a refusal.
    pub unremovable_problems: Vec<(String, String)>,
    /// The queue reports itself full.
    pub queue_full: bool,
    /// `play_file` refuses the path.
    pub play_file: Option<String>,
    /// `open_audition` refuses with this message — a machine that does not take uploads.
    pub uploads: Option<String>,
}

#[derive(Debug)]
struct Inner {
    songs: Vec<CatalogSong>,
    packages: Vec<InstalledPackage>,
    problems: Vec<PackageProblem>,
    queue: Queue,
    transport: Transport,
    now_playing: Option<NowPlaying>,
    position_ms: u32,
    settings: Settings,
    mics: MicRegistry,
    wallpapers: WallpaperState,
    /// The files the wallpaper folder pretends to hold.
    ///
    /// Three shapes, because a page has to draw three: a removable loose picture, a removable zip
    /// standing for a whole pack, and one that is refused with a sentence. The refused one is named
    /// by a pretend `debug.wallpapers`, which is the refusal a *real* machine gives most often for a
    /// file that is otherwise in the owner's own folder.
    pictures: Vec<Picture>,
    /// Every output device this machine pretends to have.
    outputs: Vec<AudioOutput>,
    /// What settings ask for; `None` means nothing has ever been chosen.
    selected_output: Option<String>,
    /// The folder the last `open_audition` handed out, which `play_audition` joins a name onto.
    audition_dir: Option<PathBuf>,
    /// Which bank this machine pretends to be playing through.
    soundfont: SoundFontStatus,
    soundfonts: crate::machine::SoundFontBanks,
    /// What `set_machine_locale` last recorded, so `machine_locale` reads it back.
    locale: km_locale::Locale,
    /// The rows only `?all=true` reaches, appended to `soundfonts.offers` when it is asked for.
    ///
    /// Two lists rather than one filtered by a `rank` field, because this double has no catalog
    /// and inventing one would be modeling the machine's table rather than the route's behavior.
    /// What the route promises is that `all` is a *widening*, and two lists say exactly that.
    unranked_offers: Vec<crate::machine::SoundFontOffer>,
    /// How many times the catalog has changed, as the real library counts it.
    ///
    /// Kept here rather than derived from `packages.len()`, because the property the export tests
    /// need is that it moves on *every* change and never goes backwards — which a count of what is
    /// currently installed does not have.
    catalog_version: u64,
    /// Demo mode, as two independent flags.
    ///
    /// Two, not one, because that is the distinction the route exists to make: `PUT /demo` without
    /// `persist` moves only the first, and a fake that collapsed them would report the running
    /// value as if it had been written down — which is the one thing about this route worth
    /// asserting.
    demo_enabled: bool,
    demo_stored: bool,
    /// The demo delay, which has no run-against-stored split to model: the route that sets it
    /// always writes it down.
    demo_delay_secs: u32,
    /// Whether the frame-statistics panel is on.
    ///
    /// One field and not two, unlike the pair above: this switch has nothing to persist, so a
    /// running value is the only value there is.
    performance_overlay: bool,
    /// What debugging and the development console are set to in settings.
    ///
    /// **Stored only, with no running twin**, which is the opposite arrangement to the panel above
    /// and the same one the demo pair models: both take effect at the machine's next start, so the
    /// running value belongs to `ApiConfig` and a test moves it by building a different one. The
    /// Debugging pane reads these to say which of two things an owner still has to do — press the
    /// other switch, or restart — so a fake that only recorded them left that reading untestable.
    debug_stored: bool,
    dev_remote_stored: bool,
    /// The level the active output reports, or `None` for a machine whose output has none.
    ///
    /// `None` by default, which is the accurate description of a machine with no sound card: a
    /// test that wants the control has to arrange one with
    /// [`TestMachine::set_output_level_range`].
    output_level: Option<OutputLevel>,
    recorded: Vec<Recorded>,
    faults: Faults,
}

impl Inner {
    /// The same rule the real machine applies: nothing loaded and nothing waiting.
    fn output_change_allowed(&self) -> bool {
        self.transport == Transport::Idle && self.queue.is_empty()
    }

    /// What `GET /audio/outputs` reports, and what `PUT /audio/output` answers with.
    ///
    /// The active device is derived rather than stored: a selection that is present is what is
    /// playing, and one that is absent means the machine fell back — which is exactly the state a
    /// remote needs to be able to show, and the reason this is not simply `selected`.
    fn describe_outputs(&self) -> AudioOutputs {
        let present = self
            .selected_output
            .as_ref()
            .and_then(|id| self.outputs.iter().find(|d| d.id == *id && d.available));
        let (active_id, active_name, fell_back) = match present {
            Some(device) => (device.id.clone(), device.name.clone(), false),
            None => (
                "system".to_owned(),
                "Follow the system default".to_owned(),
                self.selected_output.is_some(),
            ),
        };
        AudioOutputs {
            devices: self.outputs.clone(),
            selected: self.selected_output.clone(),
            active_id,
            active_name,
            fell_back,
            changeable: self.output_change_allowed(),
            level: self.output_level,
        }
    }
}

/// A karaoke machine that exists only in memory.
#[derive(Debug)]
pub struct TestMachine {
    inner: Mutex<Inner>,
}

impl Default for TestMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl TestMachine {
    /// An idle machine with no songs and no packages.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                songs: Vec::new(),
                packages: Vec::new(),
                problems: Vec::new(),
                catalog_version: 0,
                demo_enabled: false,
                demo_stored: false,
                demo_delay_secs: 60,
                performance_overlay: false,
                debug_stored: false,
                dev_remote_stored: false,
                output_level: None,
                queue: Queue::new(),
                transport: Transport::Idle,
                now_playing: None,
                position_ms: 0,
                settings: Settings::default(),
                mics: MicRegistry::with_two_mics(),
                wallpapers: WallpaperState {
                    current: Some("sunset.jpg".to_owned()),
                    count: 3,
                    interval_secs: 30,
                    shuffle: true,
                    on_song_change: true,
                    problem: None,
                    // The owner's own: the state a machine reaches after its first upload, the one
                    // every test about *changing* wallpapers should start from, and the case the
                    // `dev_server` example is read through.
                    source: crate::machine::WallpaperSource::Owner,
                },
                pictures: vec![
                    Picture {
                        id: "sunset-jpg".to_owned(),
                        name: "sunset.jpg".to_owned(),
                        images: 1,
                        bytes: 240_000,
                        why_not_removable: None,
                    },
                    Picture {
                        id: "beach-zip".to_owned(),
                        name: "beach.zip".to_owned(),
                        images: 2,
                        bytes: 1_400_000,
                        why_not_removable: None,
                    },
                    Picture {
                        id: "pinned-jpg".to_owned(),
                        name: "pinned.jpg".to_owned(),
                        images: 1,
                        bytes: 90_000,
                        why_not_removable: Some(
                            "\"pinned.jpg\" is named by debug.wallpapers, so its file is not the \
                             machine's to remove: take it out of that list first"
                                .to_owned(),
                        ),
                    },
                ],
                // Two devices and the "follow the system" entry, which is the smallest set that
                // makes every branch reachable: a USB interface to prefer, an onboard one to choose
                // instead, and the sentinel. `outputs` is public through `set_audio_outputs` so a
                // test can add an unavailable one.
                outputs: vec![
                    AudioOutput {
                        id: "system".to_owned(),
                        name: "Follow the system default".to_owned(),
                        // The sentinel row is named by its id; the flag below marks which real
                        // device it resolves to.
                        system_default: false,
                        usb: false,
                        available: true,
                        preferred: true,
                    },
                    AudioOutput {
                        id: "alsa:plughw:CARD=Device,DEV=0".to_owned(),
                        name: "USB Audio CODEC".to_owned(),
                        system_default: false,
                        usb: true,
                        available: true,
                        preferred: true,
                    },
                    AudioOutput {
                        id: "alsa:plughw:CARD=PCH,DEV=0".to_owned(),
                        name: "HDA Intel PCH".to_owned(),
                        // The onboard card is what the system points at, which is exactly the
                        // appliance's problem: the interface everyone is plugged into is not it.
                        system_default: true,
                        usb: false,
                        available: true,
                        preferred: true,
                    },
                    AudioOutput {
                        // The same onboard output under another of ALSA's names for it, so
                        // `preferred` is exercised end to end rather than asserted about a list
                        // where it happens to be uniformly true.
                        id: "alsa:front:CARD=PCH,DEV=0".to_owned(),
                        name: "HDA Intel PCH".to_owned(),
                        system_default: false,
                        usb: false,
                        available: true,
                        preferred: false,
                    },
                ],
                selected_output: Some("alsa:plughw:CARD=Device,DEV=0".to_owned()),
                audition_dir: None,
                // A bundled bank that loaded, because that is the ordinary case and the `dev_server`
                // example is what somebody looks at the page through. The two degraded states are
                // reached with `set_soundfont`.
                soundfont: SoundFontStatus {
                    path: Some(
                        "/opt/karaokemachine/assets/soundfont/GeneralUser-GS.sf2".to_owned(),
                    ),
                    chosen_by: Some(SoundFontChoice::Bundled),
                    playing: SoundKind::SoundFont,
                    problem: None,
                    fallback: None,
                },
                // The bundled bank alone, which is what a machine nobody has added a bank to looks
                // like — and the case the picker has to render without offering a choice that is
                // not there.
                locale: km_locale::Locale::default(),
                soundfonts: crate::machine::SoundFontBanks {
                    banks: vec![crate::machine::SoundFontBank {
                        id: "bundled".to_owned(),
                        name: "Bundled".to_owned(),
                        bytes: 32_319_396,
                        bundled: true,
                        why_not_removable: Some(
                            "the bundled SoundFont ships with the machine and cannot be removed"
                                .to_owned(),
                        ),
                    }],
                    selected: "bundled".to_owned(),
                    // Nothing on offer and nothing being fetched, which is what a machine that has
                    // never been asked for one looks like.
                    offers: Vec::new(),
                    fetching: None,
                },
                unranked_offers: Vec::new(),
                recorded: Vec::new(),
                faults: Faults::default(),
            }),
        }
    }

    /// Replaces the device list, so a test can present an absent one.
    pub fn set_audio_outputs(&self, outputs: Vec<AudioOutput>, selected: Option<String>) {
        let mut inner = self.lock();
        inner.outputs = outputs;
        inner.selected_output = selected;
    }

    /// Gives the active output a level, so a test can exercise the control.
    ///
    /// Without this a `TestMachine` reports no level at all, which is the state of a machine with
    /// no sound card and the one every other test wants.
    pub fn set_output_level_range(&self, level: OutputLevel) {
        self.lock().output_level = Some(level);
    }

    /// Replaces the bank report, so a test can present a machine on a test tone or with no device.
    pub fn set_soundfont(&self, soundfont: SoundFontStatus) {
        self.lock().soundfont = soundfont;
    }

    /// Replaces the list of banks that could be chosen, and which one the setting names.
    pub fn set_soundfonts(&self, banks: crate::machine::SoundFontBanks) {
        self.lock().soundfonts = banks;
    }

    /// Adds the offers only a caller asking for the whole catalog sees.
    ///
    /// The narrow list is unaffected, which is the property `?all=true` has to keep: a phone must go
    /// on being shown the shortlist whatever a development page asked for a moment earlier.
    pub fn set_unranked_soundfont_offers(&self, offers: Vec<crate::machine::SoundFontOffer>) {
        self.lock().unranked_offers = offers;
    }

    /// A machine with a small catalog and one installed package.
    ///
    /// Numbers start at 1001, which is deliberately not 1: an off-by-one that treats a number as an
    /// index would pass against a catalog numbered from zero.
    pub fn with_catalog(count: u32) -> Self {
        let machine = Self::new();
        {
            let mut inner = machine.lock();
            for offset in 0..count {
                let number = 1001 + offset;
                inner.songs.push(CatalogSong {
                    number: SongCode::new(number),
                    package_id: "vol1".to_owned(),
                    kind: km_catalog::SongKind::Midi,
                    title: format!("Song {number}"),
                    artist: Some(if offset % 2 == 0 {
                        "Even Artist".to_owned()
                    } else {
                        "Odd Artist".to_owned()
                    }),
                    // A code, not a raw `@L` header: packaging settles the language, so the fixture
                    // has to look like what a real catalog contains. Two of them, so the
                    // `?language=` filter has something to narrow.
                    language: Some(if offset % 2 == 0 { "pt" } else { "ja" }.to_owned()),
                    file: format!("songs/{number}.kar"),
                    duration_ms: 180_000 + offset * 1_000,
                    lyric_encoding: None,
                    default_transpose: 0,
                    fixes: Vec::new(),
                    // Every other song abstained on melody detection, so the "toggle hidden" path
                    // is reachable without constructing a second machine.
                    melody_channel: if offset % 2 == 0 { Some(4) } else { None },
                    suitability: Some(if offset % 2 == 0 { 9 } else { 5 }),
                    content_hash: Some(format!("{number:032x}")),
                    // Every other song has one, so both the present and the absent case are
                    // reachable from the fixture — the absent one is a video, an MP3+G song, and
                    // every song of a package built before previews existed.
                    lyric_preview: if offset % 2 == 0 {
                        vec![
                            format!("First line of {number}"),
                            "and the second".to_owned(),
                        ]
                    } else {
                        Vec::new()
                    },
                    // Three tags across the fixture, so an OR filter has a union to build rather
                    // than something that merely repeats one tag: every song is `karaoke`, half are
                    // `rock`, and a third are `brasil` — so `rock,brasil` is a strict superset of
                    // either alone, which is the property an intersection would get wrong.
                    tags: {
                        let mut tags = vec!["karaoke".to_owned()];
                        if offset % 2 == 0 {
                            tags.push("rock".to_owned());
                        }
                        if offset % 3 == 0 {
                            tags.push("brasil".to_owned());
                        }
                        tags.sort();
                        tags
                    },
                    // Every song here is MIDI, and a MIDI song is the reference rather than
                    // something levelled against it — so none of them carries a measurement, which
                    // is what a real catalog of MIDI songs holds too.
                    loudness_lufs: None,
                });
            }
            inner.packages.push(InstalledPackage {
                id: "vol1".to_owned(),
                name: "Volume 1".to_owned(),
                version: "1.0.4".to_owned(),
                path: "D:/packages/vol1.kmpkg".to_owned(),
                song_count: count as usize,
                installed_at: "2026-08-23T12:00:00Z".to_owned(),
                bank: 1,
            });
        }
        machine
    }

    /// Wraps this machine in an `Arc`, as the router wants it.
    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// Installs a fault so an error path can be exercised.
    pub fn set_faults(&self, faults: Faults) {
        self.lock().faults = faults;
    }

    /// What the machine was asked to do, in order.
    pub fn recorded(&self) -> Vec<Recorded> {
        self.lock().recorded.clone()
    }

    /// Forgets the recording.
    pub fn clear_recorded(&self) {
        self.lock().recorded.clear();
    }

    /// The queue, for asserting directly rather than through the API.
    pub fn queue_ids(&self) -> Vec<u64> {
        self.lock().queue.entries().map(|entry| entry.id).collect()
    }

    /// Moves the playhead, so a `GET /state` can be checked mid-song.
    pub fn set_position_ms(&self, position_ms: u32) {
        self.lock().position_ms = position_ms;
    }

    /// Replaces the wallpaper state, including its failure cases.
    pub fn set_wallpapers(&self, wallpapers: WallpaperState) {
        self.lock().wallpapers = wallpapers;
    }

    /// Replaces what the wallpaper folder holds.
    ///
    /// Separate from [`Self::set_wallpapers`] because the two answer different questions — one is
    /// the cycle, the other is the folder — and a test about a filename shape wants to name exactly
    /// one file without restating the interval and the shuffle to do it.
    pub fn set_pictures(&self, pictures: Vec<Picture>) {
        self.lock().pictures = pictures;
    }

    /// Declares packages the machine refused, so the reporting path can be exercised.
    pub fn set_package_problems(&self, problems: Vec<PackageProblem>) {
        self.lock().problems = problems;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned mutex in a test double means an earlier assertion already failed inside a
        // handler; recovering the guard keeps the real failure visible instead of masking it with a
        // second panic here.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Inner {
    /// Starts the song at the front of the queue, if there is one.
    fn start_next(&mut self) -> bool {
        let Some(entry) = self.queue.pop() else {
            self.transport = Transport::Idle;
            self.now_playing = None;
            self.position_ms = 0;
            return false;
        };
        let song = self
            .songs
            .iter()
            .find(|song| song.number == entry.number)
            .cloned();
        self.now_playing = Some(NowPlaying {
            origin: Origin::Catalog {
                number: entry.number,
                entry_id: entry.id,
            },
            title: entry.title.clone(),
            artist: entry.artist.clone(),
            language: song.as_ref().and_then(|song| song.language.clone()),
            singer: entry.singer.clone(),
            kind: song
                .as_ref()
                .map_or(km_catalog::SongKind::Midi, |song| song.kind),
            duration_ms: song.as_ref().map_or(0, |song| song.duration_ms),
            melody_channel: song.as_ref().and_then(|song| song.melody_channel),
            has_lyrics: true,
        });
        self.transport = Transport::Playing;
        self.position_ms = 0;
        true
    }
}

impl Catalog for TestMachine {
    fn search(&self, query: &SearchQuery) -> Result<Vec<CatalogSong>, CatalogError> {
        let inner = self.lock();
        if let Some(message) = &inner.faults.search {
            return Err(CatalogError::Failed(message.clone()));
        }
        let matches: Vec<CatalogSong> = inner
            .songs
            .iter()
            .filter(|song| {
                query.text.as_ref().is_none_or(|text| {
                    let needle = text.to_lowercase();
                    song.title.to_lowercase().contains(&needle)
                        || song
                            .artist
                            .as_deref()
                            .is_some_and(|artist| artist.to_lowercase().contains(&needle))
                })
            })
            .filter(|song| {
                query.artist.as_ref().is_none_or(|artist| {
                    song.artist
                        .as_deref()
                        .is_some_and(|value| value.to_lowercase().contains(&artist.to_lowercase()))
                })
            })
            .filter(|song| {
                // Exact and case-folded, mirroring `km_catalog::SearchQuery::to_sql` -- this stands
                // in for the real catalog, so a filter it does not implement is a filter the
                // surface tests silently do not exercise.
                query.language.as_ref().is_none_or(|language| {
                    song.language.as_deref() == Some(language.trim().to_lowercase().as_str())
                })
            })
            .filter(|song| !query.exclude_packages.contains(&song.package_id))
            .filter(|song| {
                // **Any** tag, not every — the OR is the filter's whole design, and a fake that got
                // this wrong would let a surface test pass over an intersection. Folded on the way
                // in like the real one, so a query naming `ROCK` matches a song stored as `rock`.
                //
                // No tags at all is no tag filter, where an `any` over nothing is nothing.
                query.tags.is_empty()
                    || query.tags.iter().any(|wanted| {
                        km_kmpkg::Tag::parse(wanted).is_some_and(|wanted| {
                            song.tags.iter().any(|held| held == wanted.as_str())
                        })
                    })
            })
            .filter(|song| {
                query
                    .min_suitability
                    .is_none_or(|minimum| song.suitability.unwrap_or(0) >= minimum)
            })
            .filter(|song| !query.melody_only || song.melody_channel.is_some())
            .skip(query.offset)
            .take(query.effective_limit())
            .cloned()
            .collect();
        Ok(matches)
    }

    fn song(&self, number: SongCode) -> Result<Option<CatalogSong>, CatalogError> {
        Ok(self
            .lock()
            .songs
            .iter()
            .find(|song| song.number == number)
            .cloned())
    }

    fn song_in_package(
        &self,
        package_id: &str,
        content_hash: &str,
    ) -> Result<Option<CatalogSong>, CatalogError> {
        Ok(self
            .lock()
            .songs
            .iter()
            .find(|song| {
                song.package_id == package_id && song.content_hash.as_deref() == Some(content_hash)
            })
            .cloned())
    }

    /// **The lowest number wins, and this double has to say so as loudly as the real catalog
    /// does.** Two packages holding one recording is legitimate, so the choice is real; a stand-in
    /// that returned whichever row happened to be first would let a test pass that the catalog
    /// would fail.
    fn song_by_content(&self, content_hash: &str) -> Result<Option<CatalogSong>, CatalogError> {
        Ok(self
            .lock()
            .songs
            .iter()
            .filter(|song| song.content_hash.as_deref() == Some(content_hash))
            .min_by_key(|song| song.number)
            .cloned())
    }

    fn has_package(&self, package_id: &str) -> Result<bool, CatalogError> {
        Ok(self
            .lock()
            .songs
            .iter()
            .any(|song| song.package_id == package_id))
    }

    fn load(&self, number: SongCode) -> Result<Option<Arc<Song>>, CatalogError> {
        {
            let inner = self.lock();
            if let Some(message) = &inner.faults.load {
                return Err(CatalogError::Failed(message.clone()));
            }
            if !inner.songs.iter().any(|song| song.number == number) {
                return Ok(None);
            }
        }
        // A real parsed song, from the synthetic Soft Karaoke fixture, so the lyrics endpoint is
        // exercised against a genuine tick timeline and tempo map rather than hand-built lines.
        let bytes = km_song::testing::soft_karaoke();
        let song = Song::parse(&bytes, &ParseOptions::default())
            .map_err(|error| CatalogError::Failed(error.to_string()))?;
        Ok(Some(Arc::new(song)))
    }

    fn packages(&self) -> Result<Vec<InstalledPackage>, CatalogError> {
        Ok(self.lock().packages.clone())
    }

    fn package_problems(&self) -> Vec<PackageProblem> {
        self.lock().problems.clone()
    }

    /// Re-keys the package's songs, refusing while anything is playing or queued.
    ///
    /// A working implementation rather than a canned answer, for the reason this whole double
    /// exists: the 409 is the interesting behavior, and a stub that always said yes would let a
    /// handler that forgot to check pass.
    fn set_package_bank(&self, package_id: &str, bank: u16) -> Result<usize, CatalogError> {
        let mut inner = self.lock();
        if !inner.output_change_allowed() {
            return Err(CatalogError::Unavailable(
                "a package's bank cannot change while a song is playing or queued".into(),
            ));
        }
        let mut changed = 0;
        for song in &mut inner.songs {
            if song.package_id == package_id
                && let Some(moved) = SongCode::in_bank(bank, song.number.slot())
            {
                song.number = moved;
                changed += 1;
            }
        }
        for package in &mut inner.packages {
            if package.id == package_id {
                package.bank = bank;
            }
        }
        inner.catalog_version += 1;
        Ok(changed)
    }

    /// Reports what this double is already holding: it has no folders to read.
    ///
    /// Recorded, so a test can assert the route reached the machine at all — which is the only
    /// thing about it a fake can honestly answer. Whether pruning waits for an idle machine is
    /// `prunable`'s business and is tested there, against no machine at all.
    fn rescan(&self) -> Result<crate::machine::RescanReport, CatalogError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::Rescan);
        Ok(crate::machine::RescanReport {
            installed: inner.packages.len(),
            removed: Vec::new(),
            deferred: Vec::new(),
            problems: inner.problems.len(),
        })
    }

    /// Delegates to [`Self::install`]: this double has no packages folder to copy anything into.
    ///
    /// Recorded as an ordinary install, so a test asserting *what was installed* sees the same
    /// thing either way. A test that needs to tell the two routes apart wants a real machine.
    fn install_copied(&self, path: &Path) -> Result<InstallReport, CatalogError> {
        self.install(path)
    }

    fn install(&self, path: &Path) -> Result<InstallReport, CatalogError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::Install(path.to_path_buf()));
        if let Some(message) = &inner.faults.install {
            return Err(CatalogError::Rejected(message.clone()));
        }
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("package")
            .to_owned();
        inner.catalog_version += 1;
        let replaced = inner.packages.iter().any(|package| package.id == id);
        if !replaced {
            inner.packages.push(InstalledPackage {
                id: id.clone(),
                name: id.clone(),
                version: "1".to_owned(),
                path: path.display().to_string(),
                song_count: 2,
                installed_at: "2026-08-23T12:00:00Z".to_owned(),
                bank: 5,
            });
        }
        Ok(InstallReport {
            package_name: id.clone(),
            package_id: id,
            songs_added: 2,
            replaced_existing: replaced,
            duplicate_content: vec![DuplicateContent {
                number: SongCode::new(5001),
                existing_number: SongCode::new(1001),
                existing_package: "vol1".to_owned(),
            }],
        })
    }

    fn why_not_removable(&self, package: &InstalledPackage) -> Option<String> {
        self.lock()
            .faults
            .unremovable
            .iter()
            .find(|(id, _)| *id == package.id)
            .map(|(_, why)| why.clone())
    }

    /// Keyed by **path**, where [`Faults::unremovable`] is keyed by package id.
    ///
    /// The asymmetry is deliberate and is about who has to do arithmetic. There, the key *is* the
    /// argument `uninstall` takes. Here the argument is a derived id, and a test author setting up
    /// a file the machine will not delete should be naming the file, not working out its
    /// fingerprint by hand.
    fn why_problem_not_removable(&self, problem: &PackageProblem) -> Option<String> {
        self.lock()
            .faults
            .unremovable_problems
            .iter()
            .find(|(path, _)| *path == problem.path)
            .map(|(_, why)| why.clone())
    }

    /// A working implementation, for the reason the double exists at all.
    ///
    /// It really drops the problem, so a handler that forgot to ask — or asked with an id it built
    /// itself rather than the one the machine gives — fails here instead of passing against a stub
    /// that always said yes. The same argument `set_package_bank` makes one method up.
    fn delete_problem_file(&self, id: &str) -> Result<(), CatalogError> {
        let mut inner = self.lock();
        inner
            .recorded
            .push(Recorded::DeletedProblemFile(id.to_owned()));
        let Some(index) = inner.problems.iter().position(|problem| problem.id() == id) else {
            return Err(CatalogError::NotFound(format!("package '{id}'")));
        };
        // Refused before anything is dropped, exactly as the real machine asks before it touches
        // the file — so a test can assert the problem is still listed after a refusal.
        let path = inner.problems[index].path.clone();
        if let Some((_, why)) = inner
            .faults
            .unremovable_problems
            .iter()
            .find(|(named, _)| *named == path)
        {
            return Err(CatalogError::Rejected(why.clone()));
        }
        inner.problems.remove(index);
        Ok(())
    }

    fn uninstall(&self, package_id: &str) -> Result<usize, CatalogError> {
        let mut inner = self.lock();
        inner
            .recorded
            .push(Recorded::Uninstall(package_id.to_owned()));
        if let Some(fault) = inner.faults.uninstall.clone() {
            return Err(fault);
        }
        let Some(index) = inner
            .packages
            .iter()
            .position(|package| package.id == package_id)
        else {
            return Err(CatalogError::NotFound(format!("package '{package_id}'")));
        };
        let removed = inner.packages.remove(index);
        inner.songs.retain(|song| song.package_id != package_id);
        inner.catalog_version += 1;
        Ok(removed.song_count)
    }

    fn song_count(&self) -> Result<usize, CatalogError> {
        Ok(self.lock().songs.len())
    }

    fn export(
        &self,
        after: Option<SongCode>,
        limit: usize,
    ) -> Result<Vec<CatalogSong>, CatalogError> {
        let inner = self.lock();
        // Sorted, because the real one is `ORDER BY number` and keyset paging is only correct over an
        // ordered set. A stub that handed pages back in insertion order would let a test pass that
        // the real catalog would fail.
        let mut songs = inner.songs.clone();
        songs.sort_by_key(|song| song.number);
        Ok(songs
            .into_iter()
            // `after` is a cursor, so "everything" is expressed by having none rather than by a
            // code below the lowest — there is no such code, since zero is not a song number.
            .filter(|song| after.is_none_or(|cursor| song.number > cursor))
            .take(limit)
            .collect())
    }

    fn catalog_version(&self) -> Result<u64, CatalogError> {
        Ok(self.lock().catalog_version)
    }

    fn artists(
        &self,
        contains: Option<&str>,
        hidden: &[String],
    ) -> Result<Vec<(String, usize)>, CatalogError> {
        let inner = self.lock();
        let mut counted: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for song in inner
            .songs
            .iter()
            .filter(|song| !hidden.contains(&song.package_id))
        {
            let Some(artist) = song.artist.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            if let Some(needle) = contains.map(str::trim).filter(|value| !value.is_empty())
                && !artist.to_lowercase().contains(&needle.to_lowercase())
            {
                continue;
            }
            *counted.entry(artist.to_owned()).or_default() += 1;
        }
        // A `BTreeMap` so the order is the real one's — by name — rather than whatever a hash gave.
        Ok(counted.into_iter().collect())
    }

    fn languages(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, CatalogError> {
        let inner = self.lock();
        let mut counted: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for song in inner
            .songs
            .iter()
            .filter(|song| !hidden.contains(&song.package_id))
        {
            if let Some(language) = song.language.as_deref().filter(|code| !code.is_empty()) {
                *counted.entry(language.to_owned()).or_default() += 1;
            }
        }
        // Commonest first, as the real one orders it, so a test that asserts on the order is
        // asserting on the same thing a picker would show.
        let mut languages: Vec<(String, usize)> = counted.into_iter().collect();
        languages.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(languages)
    }

    fn tags(&self, hidden: &[String]) -> Result<Vec<(String, usize)>, CatalogError> {
        let inner = self.lock();
        let mut counted: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for song in inner
            .songs
            .iter()
            .filter(|song| !hidden.contains(&song.package_id))
        {
            for tag in song.tags.iter().filter(|tag| !tag.is_empty()) {
                *counted.entry(tag.clone()).or_default() += 1;
            }
        }
        let mut tags: Vec<(String, usize)> = counted.into_iter().collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(tags)
    }
}

impl Controller for TestMachine {
    fn snapshot(&self) -> Snapshot {
        let inner = self.lock();
        Snapshot {
            transport: inner.transport,
            now_playing: inner.now_playing.clone(),
            position_ms: inner.position_ms,
            queue_len: inner.queue.len(),
            settings: inner.settings,
        }
    }

    fn queue(&self) -> Vec<QueueEntry> {
        self.lock().queue.entries().cloned().collect()
    }

    fn queue_add(&self, request: QueueRequest) -> Result<u64, ControlError> {
        let mut inner = self.lock();
        if inner.faults.queue_full {
            return Err(ControlError::QueueFull);
        }
        inner
            .queue
            .add(request)
            .map_err(|QueueFull::Full| ControlError::QueueFull)
    }

    fn queue_remove(&self, id: u64) -> Result<QueueEntry, ControlError> {
        self.lock()
            .queue
            .remove(id)
            .ok_or_else(|| ControlError::NotFound(format!("queue entry {id}")))
    }

    fn queue_move(&self, id: u64, to_index: usize) -> Result<(), ControlError> {
        if self.lock().queue.move_to(id, to_index) {
            Ok(())
        } else {
            Err(ControlError::NotFound(format!("queue entry {id}")))
        }
    }

    fn queue_clear(&self) -> Result<usize, ControlError> {
        let mut inner = self.lock();
        let removed = inner.queue.len();
        while inner.queue.pop().is_some() {}
        Ok(removed)
    }

    fn transport(&self, command: TransportCommand) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::Transport(command));
        match command {
            TransportCommand::Play => {
                if inner.now_playing.is_some() {
                    inner.transport = Transport::Playing;
                } else if !inner.start_next() {
                    return Err(ControlError::Unavailable(
                        "nothing loaded and nothing queued".into(),
                    ));
                }
            }
            TransportCommand::Pause => {
                if inner.now_playing.is_none() {
                    return Err(ControlError::Unavailable("nothing is playing".into()));
                }
                inner.transport = Transport::Paused;
            }
            TransportCommand::Skip => {
                if inner.now_playing.is_none() {
                    // A skip into silence asks demo mode for a song, so the double answers the way
                    // the machine does: the mode on with nothing waiting is a demo, and anything
                    // else is the refusal. The record is the observation here, for the reason
                    // [`Recorded::DemoStarted`] gives — there is no poll thread to load anything.
                    //
                    // Inlined rather than calling `start_demo_song`, which takes the lock this arm
                    // is holding. The two conditions it would apply are both readable from here.
                    if inner.demo_enabled && inner.queue.is_empty() {
                        inner.recorded.push(Recorded::DemoStarted);
                        return Ok(());
                    }
                    return Err(ControlError::Unavailable("nothing is playing".into()));
                }
                inner.start_next();
            }
            TransportCommand::Restart => {
                if inner.now_playing.is_none() {
                    return Err(ControlError::Unavailable("nothing is loaded".into()));
                }
                inner.position_ms = 0;
                inner.transport = Transport::Playing;
            }
            TransportCommand::Stop => {
                inner.transport = Transport::Idle;
                inner.now_playing = None;
                inner.position_ms = 0;
            }
            TransportCommand::Seek { ms } => {
                let Some(now) = inner.now_playing.as_ref() else {
                    return Err(ControlError::Unavailable("nothing is loaded".into()));
                };
                // Clamped rather than refused: a remote seeking past the end of a song it has a
                // slightly stale duration for should land at the end, not get an error.
                inner.position_ms = ms.min(now.duration_ms);
            }
        }
        Ok(())
    }

    fn update_settings(&self, patch: &SettingsPatch) -> Result<Settings, ControlError> {
        let mut inner = self.lock();
        if let Some(transpose) = patch.transpose {
            if !(-km_queue::MAX_TRANSPOSE..=km_queue::MAX_TRANSPOSE).contains(&transpose) {
                return Err(ControlError::Rejected(format!(
                    "transpose must be between -{max} and {max} semitones",
                    max = km_queue::MAX_TRANSPOSE
                )));
            }
            inner.settings.transpose = transpose;
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
            inner.settings.tempo_ratio = ratio;
        }
        if let Some(enabled) = patch.melody_enabled {
            let available = inner
                .now_playing
                .as_ref()
                .is_none_or(|now| now.melody_channel.is_some());
            if enabled && !available {
                return Err(ControlError::Unavailable(
                    "no melody channel was detected for this song".into(),
                ));
            }
            inner.settings.melody_enabled = enabled;
        }
        if let Some(volume_milli) = patch.music_volume_milli {
            inner.settings.music_volume = (volume_milli as f32 / 1000.0).clamp(0.0, 1.0);
        }
        Ok(inner.settings)
    }

    fn mics(&self) -> Vec<MicChannel> {
        self.lock().mics.channels()
    }

    fn update_mic(&self, id: &str, patch: &MicPatch) -> Result<MicChannel, ControlError> {
        self.lock()
            .mics
            .apply(id, patch)
            .map_err(|_| ControlError::NotFound(format!("microphone '{id}'")))
    }

    fn audio_outputs(&self) -> Result<AudioOutputs, ControlError> {
        Ok(self.lock().describe_outputs())
    }

    fn soundfonts(&self, all: bool) -> crate::machine::SoundFontBanks {
        let inner = self.lock();
        let mut banks = inner.soundfonts.clone();
        if all {
            banks.offers.extend(inner.unranked_offers.iter().cloned());
        }
        banks
    }

    fn set_soundfont(&self, id: &str) -> Result<(), ControlError> {
        let mut inner = self.lock();
        if !inner.soundfonts.banks.iter().any(|bank| bank.id == id) {
            return Err(ControlError::Rejected(format!("no SoundFont called {id}")));
        }
        inner.soundfonts.selected = id.to_owned();
        Ok(())
    }

    /// Refuses an id nothing offers, and otherwise says the fetch started.
    ///
    /// **Spelled out where it used to fall through to a trait default**, which is the point of that
    /// default being gone: an in-memory machine with no downloader is a fact about *this* double,
    /// and saying so here is a decision rather than something inherited by omission.
    fn fetch_soundfont(&self, id: &str) -> Result<(), ControlError> {
        let inner = self.lock();
        if !inner.soundfonts.offers.iter().any(|offer| offer.id == id) {
            return Err(ControlError::Rejected(format!("no SoundFont called {id}")));
        }
        // Nothing to download to, and nothing that waits on it: the surface tests care that the
        // route accepted the id, and a real fetch is `km-app`'s downloader.
        Ok(())
    }

    /// Remembers the locale, so `machine_locale` reads back what was set.
    fn set_machine_locale(&self, locale: km_locale::Locale) -> Result<(), ControlError> {
        self.lock().locale = locale;
        Ok(())
    }

    fn machine_locale(&self) -> km_locale::Locale {
        self.lock().locale
    }

    /// Mirrors the machine's own rules, because the surface tests are what check them: an unknown id
    /// and the bundled row are refused, and deleting the selected bank falls back to the bundled one
    /// rather than leaving `selected` naming a row that is gone.
    fn delete_soundfont(&self, id: &str) -> Result<(), ControlError> {
        let mut inner = self.lock();
        let Some(bank) = inner.soundfonts.banks.iter().find(|bank| bank.id == id) else {
            return Err(ControlError::Rejected(format!("no SoundFont called {id}")));
        };
        if bank.bundled {
            return Err(ControlError::Rejected(
                "the bundled SoundFont ships with the machine and cannot be removed".to_owned(),
            ));
        }
        inner.soundfonts.banks.retain(|bank| bank.id != id);
        if inner.soundfonts.selected == id {
            inner.soundfonts.selected = "bundled".to_owned();
        }
        Ok(())
    }

    fn soundfont(&self) -> SoundFontStatus {
        self.lock().soundfont.clone()
    }

    fn set_audio_output(&self, id: &str) -> Result<AudioOutputs, ControlError> {
        let mut inner = self.lock();
        if !inner.output_change_allowed() {
            return Err(ControlError::Unavailable(
                "the output device can only be changed when nothing is playing or queued".into(),
            ));
        }
        if !inner
            .outputs
            .iter()
            .any(|device| device.id == id && device.available)
        {
            return Err(ControlError::NotFound(format!("audio output '{id}'")));
        }
        inner.selected_output = Some(id.to_owned());
        inner.recorded.push(Recorded::SetAudioOutput(id.to_owned()));
        Ok(inner.describe_outputs())
    }

    /// **`true`, so the route is mounted and the tests that drive it have something to drive.**
    /// Whether the active output has a level is the separate question `output_level` answers, and
    /// it is `None` until a test arranges one — which is how the "this output has no level" path
    /// gets exercised without a second kind of double.
    fn output_level_supported(&self) -> bool {
        true
    }

    fn set_output_level(&self, db_centi: i32) -> Result<AudioOutputs, ControlError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::SetOutputLevel(db_centi));
        let Some(level) = inner.output_level else {
            return Err(ControlError::Unavailable(Refusal::coded(
                crate::machine::NO_OUTPUT_LEVEL,
                "this output has no level to set",
            )));
        };
        // Clamped rather than refused, which is what the real one does, so a test asserting the
        // answer to an out-of-range request asserts the same rule the machine applies.
        inner.output_level = Some(OutputLevel {
            db_centi: db_centi.clamp(level.db_min_centi, level.db_max_centi),
            ..level
        });
        Ok(inner.describe_outputs())
    }

    fn set_session_epoch(&self, epoch: u64) -> Result<(), ControlError> {
        // Kept rather than dropped: a test controller that silently swallows this lets a
        // persistence gap through with no test noticing.
        self.lock().recorded.push(Recorded::SessionEpochSet(epoch));
        Ok(())
    }

    /// Held as well as recorded, and the two halves answer different questions.
    ///
    /// The record is that the value crossed the seam; the field is what a page reads back to say
    /// what is still outstanding. See [`Control::developer_switches`] below.
    fn set_debug_enabled(&self, enabled: bool) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner.debug_stored = enabled;
        inner.recorded.push(Recorded::DebugEnabledSet(enabled));
        Ok(())
    }

    fn set_dev_remote_enabled(&self, enabled: bool) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner.dev_remote_stored = enabled;
        inner.recorded.push(Recorded::DevRemoteEnabledSet(enabled));
        Ok(())
    }

    /// What settings say, which is not what this run is doing.
    ///
    /// **The default implementation answers both-off for ever**, so a page drawn over it could not
    /// tell *the switch was never pressed* from *the switch was pressed and awaits a restart* —
    /// which is the whole distinction the Debugging pane's two banners make.
    fn developer_switches(&self) -> crate::machine::DeveloperSwitches {
        let inner = self.lock();
        crate::machine::DeveloperSwitches {
            debug: inner.debug_stored,
            dev_remote: inner.dev_remote_stored,
        }
    }

    fn performance_overlay(&self) -> bool {
        self.lock().performance_overlay
    }

    /// Held as well as recorded, unlike the two switches beside it.
    ///
    /// Those write to settings and take effect at a restart, so what a test can check is that the
    /// value crossed the seam. This one takes effect at once, so `GET` after `PUT` has to answer the
    /// new value — and a fake that only recorded would let the read half rot unnoticed.
    fn set_performance_overlay(&self, on: bool) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner.performance_overlay = on;
        inner.recorded.push(Recorded::PerformanceOverlaySet(on));
        Ok(())
    }

    fn set_admin_password(
        &self,
        hash: Option<String>,
        factory_pin: Option<String>,
    ) -> Result<(), ControlError> {
        self.lock()
            .recorded
            .push(Recorded::AdminPasswordSet(hash, factory_pin));
        Ok(())
    }

    fn open_upload(&self) -> Result<PathBuf, ControlError> {
        // A real folder, because the handler streams into it and a fake path would make every
        // upload test a test of `File::create` failing.
        let dir = std::env::temp_dir().join(format!(
            "km-test-upload-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|error| {
            ControlError::Unavailable(
                format!("could not make a folder for the upload: {error}").into(),
            )
        })?;
        Ok(dir)
    }

    fn accept_upload(
        &self,
        kind: crate::machine::Upload,
        staged: &Path,
    ) -> Result<String, ControlError> {
        let name = staged
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Read before it is recorded, so a test can assert that the bytes actually arrived rather
        // than only that a path was handed over -- which is the half of an upload route that can
        // silently do nothing.
        let bytes = std::fs::metadata(staged)
            .map(|meta| meta.len())
            .unwrap_or(0);
        self.lock()
            .recorded
            .push(Recorded::UploadAccepted(kind, name.clone()));
        let _ = std::fs::remove_file(staged);
        Ok(format!("took {name} ({bytes} bytes)"))
    }

    fn set_machine_name(&self, name: &str) -> Result<(), ControlError> {
        // The trait defaults this to a refusal, for a host that keeps no settings. This one does
        // keep them, so it must say so — leaving the default in place would make every surface
        // sweep of `PUT /machine/name` pass on a 503 and prove nothing about the route.
        self.lock()
            .recorded
            .push(Recorded::MachineRenamed(name.to_owned()));
        Ok(())
    }

    fn wallpapers(&self) -> WallpaperState {
        self.lock().wallpapers.clone()
    }

    fn next_wallpaper(&self) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::NextWallpaper);
        inner.wallpapers.current = Some("next.jpg".to_owned());
        Ok(())
    }

    fn wallpaper_pictures(&self) -> Vec<Picture> {
        self.lock().pictures.clone()
    }

    fn delete_wallpaper(&self, id: &str) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner
            .recorded
            .push(Recorded::DeleteWallpaper(id.to_owned()));
        let Some(picture) = inner.pictures.iter().find(|p| p.id == id).cloned() else {
            return Err(ControlError::Rejected(format!("no wallpaper called {id}")));
        };
        // The refusal comes from the same field the listing publishes as `removable: false`, so a
        // test cannot see a row offered and refused, or hidden and accepted.
        if let Some(reason) = picture.why_not_removable {
            return Err(ControlError::Rejected(reason));
        }
        inner.pictures.retain(|p| p.id != id);
        inner.wallpapers.count = inner.wallpapers.count.saturating_sub(picture.images);
        // What a real machine does through `wallpaper_requested`: the deleted picture leaves the
        // screen at once rather than at the end of its interval.
        inner.wallpapers.current = Some("next.jpg".to_owned());
        Ok(())
    }

    fn play_file(&self, path: &Path, decided: &Audition<'_>) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::PlayFile(path.to_path_buf()));
        record_decided(&mut inner, decided);
        if let Some(message) = &inner.faults.play_file {
            return Err(ControlError::Rejected(message.clone()));
        }
        begin_file(&mut inner, path);
        Ok(())
    }

    /// Reports the two flags, and derives the rest the way the real machine does.
    ///
    /// `starts_in_secs` is always `None` here: this double has no clock and no poll thread, so
    /// inventing a countdown would be modeling the machine's timer rather than the route's shape.
    /// What the surface tests need from this is that the flags travel, and they do.
    fn demo(&self) -> crate::machine::DemoState {
        let inner = self.lock();
        crate::machine::DemoState {
            enabled: inner.demo_enabled,
            stored: inner.demo_stored,
            delay_secs: inner.demo_delay_secs,
            min_suitability: Some(5),
            playing: matches!(
                inner.now_playing.as_ref().map(|now| &now.origin),
                Some(Origin::Demo { .. })
            ),
            starts_in_secs: None,
        }
    }

    /// Moves the running flag always, and the stored one only when asked.
    ///
    /// That asymmetry is the route's whole point, so the fake keeps it rather than writing both:
    /// a test that sends `persist: false` and reads `stored: true` back would be reporting a bug
    /// that is not there, and one that never checked would miss the bug that is.
    fn set_demo(
        &self,
        enabled: bool,
        persist: bool,
    ) -> Result<crate::machine::DemoState, ControlError> {
        {
            let mut inner = self.lock();
            inner.demo_enabled = enabled;
            if persist {
                inner.demo_stored = enabled;
            }
            inner.recorded.push(Recorded::SetDemo { enabled, persist });
        }
        Ok(self.demo())
    }

    /// Applies the cap and keeps the number, and there is no `persist` to model.
    ///
    /// **The cap is repeated here rather than borrowed**, because `km-api` does not depend on
    /// `karaokemachine` — the dependency runs the other way. A double that accepted what the real
    /// machine refuses would let the route stop forwarding the refusal and no surface test would
    /// notice, which is the one thing about this route worth asserting.
    ///
    /// What it cannot model is the deadline shift, since this double has no clock. That belongs to
    /// `karaokemachine::machine::demo_deadline_moved` and is tested where it lives.
    fn set_demo_delay(&self, delay_secs: u32) -> Result<crate::machine::DemoState, ControlError> {
        {
            let mut inner = self.lock();
            if delay_secs > MAX_DEMO_DELAY_SECS {
                return Err(ControlError::Rejected(format!(
                    "a demo delay of {delay_secs} seconds is longer than the \
                     {MAX_DEMO_DELAY_SECS} this accepts"
                )));
            }
            inner.demo_delay_secs = delay_secs;
            inner.recorded.push(Recorded::SetDemoDelay { delay_secs });
        }
        Ok(self.demo())
    }

    /// Applies the two refusals a double can see, and records the press.
    ///
    /// The third — a machine with no sound — has nothing to model here: this double always can play.
    /// The two that are kept are the two the *route* is about, and they are separate branches rather
    /// than one so that both sentences are reachable from a test.
    fn start_demo_song(&self) -> Result<crate::machine::DemoState, ControlError> {
        {
            let mut inner = self.lock();
            if inner.now_playing.is_some() {
                return Err(ControlError::Unavailable(
                    "a song is already playing".into(),
                ));
            }
            if !inner.queue.is_empty() {
                return Err(ControlError::Unavailable(
                    "there are songs in the queue to play first".into(),
                ));
            }
            inner.recorded.push(Recorded::DemoStarted);
        }
        Ok(self.demo())
    }

    /// A real folder under the system temporary directory, and it has to be a real one: the upload
    /// route streams into whatever this returns, so a fictional path would make the whole endpoint
    /// untestable through the HTTP surface — which is the one thing `tests/surface.rs` is for.
    fn open_audition(&self) -> Result<PathBuf, ControlError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::OpenAudition);
        if let Some(message) = &inner.faults.uploads {
            return Err(ControlError::Rejected(message.clone()));
        }
        // Unique per call as well as per machine, so two auditions in one test do not share a
        // folder and the sweep-and-recreate the real machine does is not accidentally relied on.
        let seq = NEXT_AUDITION.fetch_add(1, atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("km-api-audition-{seq}"));
        std::fs::create_dir_all(&dir)
            .map_err(|error| ControlError::Failed(format!("{}: {error}", dir.display())))?;
        inner.audition_dir = Some(dir.clone());
        Ok(dir)
    }

    fn play_audition(&self, name: &str, decided: &Audition<'_>) -> Result<(), ControlError> {
        let mut inner = self.lock();
        inner.recorded.push(Recorded::PlayAudition(name.to_owned()));
        record_decided(&mut inner, decided);
        // The same rule the real machine enforces, asserted here too: a caller that starts handing
        // this a path should fail against the test machine and not only on the appliance.
        if std::path::Path::new(name).file_name() != Some(name.as_ref()) {
            return Err(ControlError::Rejected(format!("{name} is not a bare name")));
        }
        let path = inner.audition_dir.clone().unwrap_or_default().join(name);
        begin_file(&mut inner, &path);
        Ok(())
    }
}

/// Numbers the staging folders [`TestMachine::open_audition`] hands out.
static NEXT_AUDITION: atomic::AtomicU64 = atomic::AtomicU64::new(0);

/// Records what a curation tool decided, shared by the two routes that carry it.
///
/// Pushed unconditionally, including when everything in it is absent: a route that stopped carrying
/// one of these would otherwise fail no test at all.
fn record_decided(inner: &mut Inner, decided: &Audition<'_>) {
    inner.recorded.push(Recorded::Decided(Decided {
        title: decided.title.map(ToOwned::to_owned),
        artist: decided.artist.map(ToOwned::to_owned),
        transpose: decided.transpose,
        fixes: decided.fixes.map(<[_]>::to_vec),
        melody: decided.melody,
        lyrics: decided.lyrics.cloned(),
    }));
}

/// Puts a loose file on the transport, shared by the two routes that play one.
///
/// Both of them end in the same state — a `NowPlaying` whose origin is a file rather than a
/// catalog row — and the reason they are one function is that a test comparing the upload route
/// against the path route should be comparing the routes and not two hand-written copies of this.
fn begin_file(inner: &mut Inner, path: &Path) {
    inner.now_playing = Some(NowPlaying {
        origin: Origin::File {
            path: path.display().to_string(),
        },
        title: path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("file")
            .to_owned(),
        artist: None,
        // A file played straight from disk has no catalog row to take one from.
        language: None,
        singer: None,
        kind: km_catalog::SongKind::Midi,
        duration_ms: 60_000,
        melody_channel: None,
        has_lyrics: true,
    });
    inner.transport = Transport::Playing;
    inner.position_ms = 0;
}

/// A host that can be asked to power off and to restart, and does neither.
///
/// **It must never fork, exit, or raise a signal, and that is not caution — it is the same decision
/// `docs/architecture/appliance.md` records about not unit-testing the SIGTERM path.** A process is
/// one thing shared by every test in the binary: a fake that really called `systemctl poweroff`
/// would try to switch off whatever machine ran `cargo test`, and one that really called
/// `std::process::exit` would end the suite with whatever tests had not run yet reported as passing.
/// So this records what it was asked and returns, and any "improvement" that makes it do the real
/// thing is a change to be reverted rather than reviewed.
///
/// Recording is what makes it worth having at all: it is how a test says the request *reached* the
/// host rather than merely that the route answered 202, which is the only interesting failure in a
/// handler whose whole job is to pass a message on.
#[derive(Debug, Default)]
pub struct TestPower {
    inner: Mutex<PowerInner>,
}

#[derive(Debug, Default)]
struct PowerInner {
    recorded: Vec<Recorded>,
    /// When set, both requests fail this way instead of being recorded.
    ///
    /// One knob for both, because what a test is exercising here is the *mapping* of a refusal to a
    /// status and a sentence, and that mapping cannot differ between the two calls — a second field
    /// would be structure for a variation with no instances.
    refusal: Option<crate::power::PowerError>,
}

impl TestPower {
    /// A host that accepts both requests.
    pub fn new() -> Self {
        Self::default()
    }

    /// A host whose operating system refuses, in its own words.
    pub fn refusing(refusal: crate::power::PowerError) -> Self {
        Self {
            inner: Mutex::new(PowerInner {
                recorded: Vec::new(),
                refusal: Some(refusal),
            }),
        }
    }

    /// What it was asked to do, in order.
    pub fn recorded(&self) -> Vec<Recorded> {
        self.lock().recorded.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PowerInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record(&self, what: Recorded) -> Result<(), crate::power::PowerError> {
        let mut inner = self.lock();
        if let Some(refusal) = inner.refusal.clone() {
            return Err(refusal);
        }
        inner.recorded.push(what);
        Ok(())
    }
}

impl crate::power::Power for TestPower {
    fn shut_down(&self) -> Result<(), crate::power::PowerError> {
        self.record(Recorded::ShutDown)
    }

    fn restart_application(&self) -> Result<(), crate::power::PowerError> {
        self.record(Recorded::RestartApplication)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(number: u32) -> QueueRequest {
        QueueRequest {
            number: SongCode::new(number),
            title: format!("Song {number}"),
            artist: None,
            singer: None,
        }
    }

    #[test]
    fn a_fresh_machine_is_idle_with_nothing_queued() {
        let machine = TestMachine::new();
        let snapshot = machine.snapshot();
        assert_eq!(snapshot.transport, Transport::Idle);
        assert!(snapshot.now_playing.is_none());
        assert_eq!(snapshot.queue_len, 0);
    }

    #[test]
    fn playing_takes_the_song_off_the_front_of_the_queue() {
        let machine = TestMachine::with_catalog(3);
        let id = machine.queue_add(request(1001)).expect("queued");
        assert_eq!(machine.snapshot().queue_len, 1);

        machine
            .transport(TransportCommand::Play)
            .expect("plays what was queued");
        let snapshot = machine.snapshot();
        assert_eq!(snapshot.transport, Transport::Playing);
        assert_eq!(snapshot.queue_len, 0);
        assert_eq!(
            snapshot.now_playing.expect("a song").origin,
            Origin::Catalog {
                number: SongCode::new(1001),
                entry_id: id
            }
        );
    }

    #[test]
    fn playing_an_empty_machine_says_there_is_nothing_to_play() {
        let machine = TestMachine::new();
        assert!(matches!(
            machine.transport(TransportCommand::Play),
            Err(ControlError::Unavailable(_))
        ));
    }

    #[test]
    fn skipping_the_last_song_leaves_the_machine_idle() {
        let machine = TestMachine::with_catalog(2);
        machine.queue_add(request(1001)).expect("queued");
        machine.transport(TransportCommand::Play).expect("plays");
        machine.transport(TransportCommand::Skip).expect("skips");
        let snapshot = machine.snapshot();
        assert_eq!(snapshot.transport, Transport::Idle);
        assert!(snapshot.now_playing.is_none());
    }

    /// A skip into silence is a demo press while the mode is on, and a refusal while it is off.
    ///
    /// **Both halves, because the mode is the whole of the difference.** The record is what says a
    /// demo was asked for, there being no poll thread to load one.
    #[test]
    fn a_skip_into_silence_asks_for_a_demo_only_while_the_mode_is_on() {
        let machine = TestMachine::with_catalog(2);
        assert!(matches!(
            machine.transport(TransportCommand::Skip),
            Err(ControlError::Unavailable(_))
        ));
        assert!(
            !machine.recorded().contains(&Recorded::DemoStarted),
            "nothing may ask for a demo with the mode off"
        );

        machine.set_demo(true, false).expect("the mode goes on");
        machine
            .transport(TransportCommand::Skip)
            .expect("a skip into silence asks for a demo");
        assert!(
            machine.recorded().contains(&Recorded::DemoStarted),
            "and the press has to reach the controller"
        );
    }

    #[test]
    fn a_seek_past_the_end_lands_at_the_end() {
        let machine = TestMachine::with_catalog(1);
        machine.queue_add(request(1001)).expect("queued");
        machine.transport(TransportCommand::Play).expect("plays");
        let duration = machine.snapshot().now_playing.expect("a song").duration_ms;
        machine
            .transport(TransportCommand::Seek { ms: u32::MAX })
            .expect("seeks");
        assert_eq!(machine.snapshot().position_ms, duration);
    }

    #[test]
    fn commands_are_recorded_in_the_order_they_arrive() {
        let machine = TestMachine::with_catalog(1);
        machine.queue_add(request(1001)).expect("queued");
        machine.transport(TransportCommand::Play).expect("plays");
        machine.transport(TransportCommand::Pause).expect("pauses");
        assert_eq!(
            machine.recorded(),
            [
                Recorded::Transport(TransportCommand::Play),
                Recorded::Transport(TransportCommand::Pause),
            ]
        );
    }

    #[test]
    fn search_filters_the_way_the_real_one_does() {
        let machine = TestMachine::with_catalog(4);
        let all = machine.search(&SearchQuery::default()).expect("search");
        assert_eq!(all.len(), 4);

        let by_text = machine.search(&SearchQuery::text("Even")).expect("search");
        assert_eq!(by_text.len(), 2);

        let good = machine
            .search(&SearchQuery {
                min_suitability: Some(8),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(good.len(), 2);

        let melodic = machine
            .search(&SearchQuery {
                melody_only: true,
                ..Default::default()
            })
            .expect("search");
        assert_eq!(melodic.len(), 2);
    }

    #[test]
    fn loading_a_song_yields_a_real_parsed_timeline() {
        let machine = TestMachine::with_catalog(1);
        let song = machine
            .load(SongCode::new(1001))
            .expect("load")
            .expect("present");
        assert!(!song.lyrics.lines.is_empty());
        assert!(machine.load(SongCode::new(9999)).expect("load").is_none());
    }

    #[test]
    fn a_transpose_outside_the_engines_range_is_refused() {
        let machine = TestMachine::new();
        let error = machine
            .update_settings(&SettingsPatch {
                transpose: Some(100),
                ..Default::default()
            })
            .expect_err("out of range");
        assert!(matches!(error, ControlError::Rejected(_)));
        // ...and nothing was applied.
        assert_eq!(machine.snapshot().settings.transpose, 0);
    }

    #[test]
    fn enabling_the_melody_on_a_song_without_one_is_refused() {
        let machine = TestMachine::with_catalog(2);
        // Song 1002 is the one detection abstained on.
        machine.queue_add(request(1002)).expect("queued");
        machine.transport(TransportCommand::Play).expect("plays");
        let error = machine
            .update_settings(&SettingsPatch {
                melody_enabled: Some(true),
                ..Default::default()
            })
            .expect_err("no melody channel");
        assert!(matches!(error, ControlError::Unavailable(_)));
    }

    #[test]
    fn faults_make_the_error_paths_reachable() {
        let machine = TestMachine::with_catalog(1);
        machine.set_faults(Faults {
            search: Some("the index is corrupt".to_owned()),
            queue_full: true,
            ..Default::default()
        });
        assert!(machine.search(&SearchQuery::default()).is_err());
        assert_eq!(
            machine.queue_add(request(1001)),
            Err(ControlError::QueueFull)
        );
    }

    #[test]
    fn uninstalling_removes_the_package_and_its_songs() {
        let machine = TestMachine::with_catalog(3);
        assert_eq!(machine.song_count().expect("count"), 3);
        assert_eq!(machine.uninstall("vol1").expect("uninstall"), 3);
        assert_eq!(machine.song_count().expect("count"), 0);
        assert_eq!(
            machine.uninstall("vol1"),
            Err(CatalogError::NotFound("package 'vol1'".to_owned()))
        );
    }

    #[test]
    fn playing_a_file_directly_reports_a_file_origin() {
        let machine = TestMachine::new();
        machine
            .play_file(Path::new("fixtures/sample.kar"), &Audition::default())
            .expect("plays");
        let now = machine.snapshot().now_playing.expect("a song");
        assert!(matches!(now.origin, Origin::File { .. }));
        assert_eq!(now.title, "sample");
    }
}
