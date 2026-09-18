//! The audio thread, and how the machine copes without one.
//!
//! One thread opens the cpal output stream, owns it, and never lets it move. That is not laziness
//! about `Send`: whether `cpal::Stream` is `Send` depends on the backend, so a design that moves it
//! compiles on Windows and fails on some other platform. Creating it on the thread that keeps it
//! sidesteps the question everywhere.
//!
//! Commands arrive over an `mpsc` channel and are forwarded into the engine's lock-free queue. The
//! extra hop costs nothing that matters — commands are things a person did, a few per minute, not
//! per-sample work — and it buys portability plus a natural home for the periodic housekeeping the
//! real-time thread is not allowed to do itself.
//!
//! **Three states, all reported.** A karaoke machine with no sound is broken, but the honest failure
//! is far better than a silent one, and the three cases have genuinely different consequences:
//!
//! * a real SoundFont: instruments sound like instruments;
//! * no SoundFont found: a sine synthesizer, so the machine works and the lyrics scroll in time —
//!   said out loud, because a singer would otherwise think the machine is faulty rather than
//!   unconfigured;
//! * no audio device at all: playback is refused. Search and the queue still work, which is a real
//!   mode on a box with no sound card.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use km_audio::audio::{Command, SharedState};
use km_audio::device;
use km_audio::sequencer::PlaybackSettings;
use km_audio::{
    AudioError, Bank, Chosen, OutputDevice, OutputStream, SoundFontSource, TestToneSource,
};
use km_queue::Transport;

use crate::settings::{AudioSettings, Paths, SOUNDFONT_SUBPATHS};

/// How often the audio thread wakes up when no command has arrived.
///
/// Its only job on a quiet tick is to hand retired songs back for freeing. That queue holds eight
/// entries, and nobody changes songs eight times in 250 ms.
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_millis(250);

/// What the audio output turned out to be.
///
/// `SoundFont` repeats the enum's name, and clippy is usually right to object — but SoundFont is the
/// name of a file format rather than a restatement of "sound", and every shorter alternative (`Bank`,
/// `Real`) says less at the call site.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sound {
    /// A real General MIDI bank is loaded.
    SoundFont {
        /// Which file.
        path: PathBuf,
        /// What the synthesizer dropped to load it, which is usually nothing.
        ///
        /// Carried here rather than left in the bank because this is the value the machine reports
        /// itself by, and a bank that loaded *incomplete* is exactly as much the owner's business as
        /// one that did not load at all — the instruments it dropped will simply never sound.
        defects: km_audio::BankDefects,
    },
    /// No bank was found, so a sine synthesizer stands in.
    TestTone {
        /// Why there is no bank, in words worth showing a person.
        reason: String,
    },
    /// No audio device. Playback is refused; everything else works.
    Silent {
        /// Why.
        reason: String,
    },
}

impl Sound {
    /// Whether a song can actually be played.
    pub fn can_play(&self) -> bool {
        !matches!(self, Self::Silent { .. })
    }

    /// A line for the log and for the connect panel's neighborhood.
    pub fn describe(&self) -> String {
        match self {
            // The defect clause is appended rather than replacing the line, because the bank *is*
            // playing and which file it is remains the first thing anybody wants to read.
            Self::SoundFont { path, defects } if !defects.is_empty() => {
                format!("SoundFont: {} — {defects}", path.display())
            }
            Self::SoundFont { path, .. } => format!("SoundFont: {}", path.display()),
            Self::TestTone { reason } => {
                format!("no SoundFont ({reason}) — using a test tone, instruments will sound wrong")
            }
            Self::Silent { reason } => format!("no audio output ({reason}) — playback unavailable"),
        }
    }
}

/// What the control thread is asked to do.
///
/// Two kinds of work reach this thread and only one of them is the player's business. Wrapping them
/// keeps [`km_audio::audio::Command`] alone: its variants cross a lock-free ring into the real-time
/// callback, so a `String` payload would mean allocating and freeing inside an audio callback, which
/// the engine's rules forbid outright. `SetOutputDevice` never goes near the ring — it is entirely a
/// statement about which device the *next* open should use.
#[derive(Debug)]
enum Job {
    /// Forward this to the player, opening the device first if it means sound.
    Player(Command),
    /// Use this device from now on, and drop the current stream so the next open picks it up.
    SetOutputDevice(Option<String>),
    /// Play through this bank from now on, and optionally drop the stream so it takes effect now.
    ///
    /// Carries an already-parsed [`Bank`] because parsing one is tens of megabytes of work and this
    /// thread has a 250 ms housekeeping cadence to keep; the control thread does the reading. The
    /// `close` flag is the whole difference between a swap somebody hears immediately and one that
    /// waits for the next song: a video or MP3+G song's audio lives inside the stream and cannot be
    /// rebuilt, so its bank changes without the stream being touched.
    SetSoundFont {
        /// The parsed bank, and the file it came from.
        bank: Box<Bank>,
        /// Where it was read from, for the status line.
        path: PathBuf,
        /// Whether to drop the stream so the next open uses this bank.
        close: bool,
    },
}

/// Where the machine's sound is going, and where it was asked to go.
///
/// Written by the control thread and read by the API. Deliberately not part of [`SharedState`],
/// which is atomics-only because the real-time callback writes it; nothing here is touched by the
/// callback, so an ordinary mutex is the honest tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputStatus {
    /// What settings ask for. `None` means nothing has ever been chosen.
    pub requested: Option<String>,
    /// What was actually opened, or probed, most recently.
    pub active: Chosen,
}

/// What is reported before anything has been opened, and after the device is changed.
///
/// Between a change and the next song there is genuinely no answer to "what is it playing through",
/// and saying so beats naming the device it is *about* to open as though it already had.
fn unknown_output() -> Chosen {
    Chosen {
        id: km_audio::SYSTEM_DEFAULT.to_owned(),
        name: "not yet opened".to_owned(),
        fell_back: false,
    }
}

/// A handle on the audio thread.
///
/// Cloneable-by-`Arc` at the call site; the handle itself is held once by the machine. Dropping it
/// closes the command channel, which stops the thread and releases the device.
#[derive(Debug)]
pub struct Engine {
    commands: Option<mpsc::Sender<Job>>,
    shared: Arc<SharedState>,
    /// What is playing the notes, which the switcher can change while the machine runs.
    ///
    /// Behind a mutex for the same reason `output` is: `Ctrl+2` can replace the bank while the
    /// machine runs, and a plain field decided once at startup would leave every surface that names
    /// the bank — the log line, `GET /audio/soundfont`, the on-screen label — naming the one the
    /// machine started with.
    sound: Arc<Mutex<Sound>>,
    stopping: Arc<AtomicBool>,
    /// Shared with the control thread, which is the only writer.
    output: Arc<Mutex<OutputStatus>>,
}

/// What an [`Engine::recording`] engine was told, in the order it was told it.
///
/// **Drained on demand rather than by a thread of its own, and that is what makes it usable for an
/// assertion.** A draining thread would leave every count racing the scheduler: a test that has
/// just watched `send` return has no way to know the other end has run. Here the receiver lives in
/// this structure, so by the time [`CommandLog::count`] is called, every send that has already
/// returned is sitting in the channel waiting to be read.
///
/// Holding the receiver also keeps the channel open, which is the other half: `send` reports
/// failure by returning `false`, and a double whose sends quietly stop working the moment the
/// receiver is dropped is worse than no double at all.
///
/// Commands are kept as their `Debug` text rather than as `Command` values, so this needs neither
/// `Clone` nor `PartialEq` from `km_audio` — a test asks how many songs were loaded, not which
/// `Arc<Song>` it was.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct CommandLog(Arc<Mutex<LogInner>>);

#[cfg(test)]
#[derive(Debug)]
struct LogInner {
    seen: Vec<String>,
    incoming: mpsc::Receiver<Job>,
}

#[cfg(test)]
impl CommandLog {
    fn new(incoming: mpsc::Receiver<Job>) -> Self {
        Self(Arc::new(Mutex::new(LogInner {
            seen: Vec::new(),
            incoming,
        })))
    }

    /// How many commands whose `Debug` text starts with `name` have arrived.
    ///
    /// Matched on the name rather than the whole value so that `count("Load")` counts a load
    /// however its payload prints.
    pub(crate) fn count(&self, name: &str) -> usize {
        let mut inner = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Ok(job) = inner.incoming.try_recv() {
            if let Job::Player(command) = job {
                inner.seen.push(format!("{command:?}"));
            }
        }
        inner
            .seen
            .iter()
            .filter(|line| line.starts_with(name))
            .count()
    }
}

impl Engine {
    /// Starts the audio thread, or reports why it could not.
    ///
    /// Never fails: a machine that will not start because a `.sf2` is missing is a worse product
    /// than one that says so and keeps working.
    ///
    /// `restored` is the switcher slot this machine was left on, if it still answers. It is opened
    /// **instead of** the resolved bank rather than after it: the alternative is parsing tens of
    /// megabytes of a bank nobody is going to hear, once per start, on the machines that use the
    /// switcher most. Its measured level travels with it, exactly as it does for a keypress — and a
    /// slot with no measured level leaves `audio.music_volume` alone, which is the half that is easy
    /// to lose. See `Switching the bank while it plays` in docs/decisions/audio.md.
    pub fn start(
        audio: &AudioSettings,
        paths: &Paths,
        restored: Option<&crate::settings::DebugBank>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<Job>();
        // The thread reports what it found, so `start` returns the truth rather than a guess — and
        // hands back the state object the callbacks write, so the handle reads the very atomics the
        // audio thread writes rather than a copy that could go stale. That object outlives any one
        // stream, which is what lets the device be released and reopened underneath it.
        let (ready_tx, ready_rx) = mpsc::channel::<Ready>();
        let stopping = Arc::new(AtomicBool::new(false));

        // Two steps, and the split is the point. The setting names a **bank id**, so the folder
        // is asked which file that is; then the existing rule picks between that answer and the
        // bundled candidates. A stale id resolves to nothing and therefore falls through to
        // bundled, with `missing` carrying the reason for something to say.
        let selected = crate::soundfont::resolve(paths, audio.soundfont.as_deref());
        let (chosen, music_volume) = match restored {
            // A remembered slot names a file, so it goes through the same "named explicitly and not
            // there" branch every other configured path does: a bank deleted between two runs says
            // so rather than quietly becoming the bundled one.
            Some(bank) => (
                resolve_soundfont(Some(&bank.path), paths),
                bank.music_volume.unwrap_or(audio.music_volume),
            ),
            None => (
                resolve_soundfont(selected.path.as_deref(), paths),
                audio.music_volume,
            ),
        };
        let idle_release = audio.idle_release();
        let want = audio.output_device.clone();
        let output = Arc::new(Mutex::new(OutputStatus {
            requested: want.clone(),
            active: unknown_output(),
        }));
        let thread_stopping = Arc::clone(&stopping);
        let thread_output = Arc::clone(&output);
        let spawned = std::thread::Builder::new()
            .name("km-audio".to_owned())
            .spawn(move || {
                run(
                    chosen,
                    music_volume,
                    idle_release,
                    want,
                    command_rx,
                    ready_tx,
                    thread_stopping,
                    thread_output,
                );
            });

        if let Err(error) = spawned {
            return Self::silent(format!("the audio thread would not start: {error}"));
        }

        // A bounded wait, because a wedged audio driver must not wedge startup. Ten seconds is
        // generous for opening a device and parsing a bank of tens of megabytes.
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ready {
                sound,
                shared: Some(shared),
            }) => Self {
                commands: Some(command_tx),
                shared,
                sound: Arc::new(Mutex::new(sound)),
                stopping,
                output,
            },
            // The thread reported a failure rather than a device.
            Ok(Ready { sound, .. }) => Self {
                commands: None,
                shared: Arc::new(SharedState::default()),
                sound: Arc::new(Mutex::new(sound)),
                stopping,
                output,
            },
            // These two were one arm, both reported as a ten-second timeout, and the wrong one of
            // them is what an Android device actually produced — 76 ms after startup, with the log
            // insisting it had waited ten seconds. They are opposite failures and deserve opposite
            // messages.
            Err(RecvTimeoutError::Timeout) => {
                Self::silent("the audio device did not open within ten seconds")
            }
            Err(RecvTimeoutError::Disconnected) => Self::silent(
                "the audio thread stopped before it opened a device, which means it panicked \
                 — the panic itself is logged separately",
            ),
        }
    }

    /// An engine whose sound is rendered by its caller rather than pushed to a device.
    ///
    /// **No thread and no device**, which is the whole of the difference. A sound card pulls blocks
    /// on its own schedule and a thread exists to serve it; here the caller pulls them, so the jobs
    /// this handle sends are drained by whoever is rendering and the housekeeping that opens,
    /// closes and reopens a device has nothing to do.
    ///
    /// The returned [`StreamAudio`] is the other half and must be driven, or nothing sounds and the
    /// position never moves.
    #[cfg(feature = "video")]
    pub fn streaming(
        audio: &AudioSettings,
        paths: &Paths,
        restored: Option<&crate::settings::DebugBank>,
        sample_rate: u32,
        channels: usize,
    ) -> (Self, StreamAudio) {
        let (command_tx, command_rx) = mpsc::channel::<Job>();
        let selected = crate::soundfont::resolve(paths, audio.soundfont.as_deref());
        let (chosen, music_volume) = match restored {
            Some(bank) => (
                resolve_soundfont(Some(&bank.path), paths),
                bank.music_volume.unwrap_or(audio.music_volume),
            ),
            None => (
                resolve_soundfont(selected.path.as_deref(), paths),
                audio.music_volume,
            ),
        };
        let (instrument, sound) = choose_instrument(chosen);
        let shared = Arc::new(SharedState::default());
        let mut renderer = StreamRenderer::open(&instrument, &shared, sample_rate, channels);
        renderer.apply(Command::SetMusicVolume(music_volume));

        let engine = Self {
            commands: Some(command_tx),
            shared,
            sound: Arc::new(Mutex::new(sound)),
            stopping: Arc::new(AtomicBool::new(false)),
            // **No device, so nothing to report about one.** The audio routes the API offers are
            // about where sound leaves the box, and here it leaves as a stream.
            output: Arc::new(Mutex::new(OutputStatus {
                requested: None,
                active: unknown_output(),
            })),
        };
        (
            engine,
            StreamAudio {
                renderer,
                jobs: command_rx,
                pending: None,
            },
        )
    }

    /// An engine with no audio at all, for tests and headless runs.
    pub fn silent(reason: impl Into<String>) -> Self {
        Self {
            commands: None,
            shared: Arc::new(SharedState::default()),
            sound: Arc::new(Mutex::new(Sound::Silent {
                reason: reason.into(),
            })),
            stopping: Arc::new(AtomicBool::new(false)),
            output: Arc::new(Mutex::new(OutputStatus {
                requested: None,
                active: unknown_output(),
            })),
        }
    }

    /// An engine that reports sound and remembers what it was told, for tests.
    ///
    /// **[`Engine::silent`] cannot stand in for a machine that plays**, and that is why this
    /// exists: `can_play` is false there, so every path that loads a song refuses before
    /// `Machine::advance` ever reaches the queue. A test built on `silent` therefore exercises the
    /// refusal and nothing else — which is how the load-next-song transaction came to have no test
    /// at all.
    ///
    /// [`Sound::TestTone`] rather than [`Sound::SoundFont`] because it is the honest description:
    /// there is no bank here, and `can_play` distinguishes only [`Sound::Silent`] from the rest.
    ///
    /// The returned [`CommandLog`] owns the receiving end — see its own note for why that is what
    /// makes a count assertable rather than a race.
    #[cfg(test)]
    pub(crate) fn recording() -> (Self, CommandLog) {
        let (sender, receiver) = mpsc::channel();
        let log = CommandLog::new(receiver);
        let engine = Self {
            commands: Some(sender),
            shared: Arc::new(SharedState::default()),
            sound: Arc::new(Mutex::new(Sound::TestTone {
                reason: "a recording engine, for tests".to_owned(),
            })),
            stopping: Arc::new(AtomicBool::new(false)),
            output: Arc::new(Mutex::new(OutputStatus {
                requested: None,
                active: unknown_output(),
            })),
        };
        (engine, log)
    }

    /// What the output turned out to be.
    ///
    /// Cloned rather than borrowed because the switcher can replace it mid-run. The clone is a
    /// `PathBuf` and a short defect list, and every caller either formats it or matches it once.
    pub fn sound(&self) -> Sound {
        self.sound
            .lock()
            .map(|sound| sound.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    /// Whether a song can be played.
    pub fn can_play(&self) -> bool {
        self.sound().can_play() && self.commands.is_some()
    }

    /// The device's sample rate, or 0 when nothing is known.
    ///
    /// Read live rather than remembered: the device is opened when a song starts and released when
    /// the machine goes quiet, so the default output can change between one song and the next, and a
    /// rate captured at startup would be a different device's. Before the first song this is what
    /// the probe answered.
    pub fn sample_rate(&self) -> u32 {
        self.shared.sample_rate()
    }

    /// Sends a command. Returns whether it was accepted.
    pub fn send(&self, command: Command) -> bool {
        self.dispatch(Job::Player(command))
    }

    /// Play through this device from now on.
    ///
    /// `None` means nothing has been chosen and the USB preference applies;
    /// [`km_audio::SYSTEM_DEFAULT`] means follow the system deliberately.
    ///
    /// The stream is dropped rather than rebuilt in place, so the *next* thing that means sound
    /// opens the new device. That is the same path an idle release already takes many times an
    /// evening, which is why switching costs nothing new. **The caller is responsible for refusing
    /// this while a song is loaded** — the player lives inside the stream being dropped.
    pub fn set_output_device(&self, want: Option<String>) -> bool {
        // Both fields, and synchronously. The audio thread will set them again when it picks the job
        // up, but that is a tick away and the caller answers an HTTP request *now* — leaving `active`
        // naming the device that was just replaced would make the response to a successful change
        // report the old device as the one in use.
        if let Ok(mut status) = self.output.lock() {
            status.requested = want.clone();
            status.active = unknown_output();
        }
        self.dispatch(Job::SetOutputDevice(want))
    }

    /// Play through this bank from now on.
    ///
    /// Takes a bank the caller has already opened, because parsing one is tens of megabytes of work
    /// that has no business on the audio thread. `close` drops the current stream so the next thing
    /// that means sound rebuilds the synthesizer around the new bank — which is the only way it can
    /// be rebuilt, since `rustysynth`'s `Synthesizer` takes its bank in `new` and offers no setter.
    ///
    /// **Pass `close: false` while a video or MP3+G song is playing**, and re-send the song yourself
    /// after a `close: true`. The player lives inside the stream being dropped, exactly as it does
    /// for [`Engine::set_output_device`] — but where that refuses mid-song, this one is *for*
    /// mid-song, so the caller owns putting the song back. [`Sticky`] replays the four playback
    /// settings on reopen; the song and its position are the caller's to restore.
    pub fn set_soundfont(&self, bank: Bank, path: PathBuf, close: bool) -> bool {
        // Synchronously, for the same reason `set_output_device` writes `output` here rather than
        // leaving it to the audio thread: the keypress that caused this is about to draw a label
        // naming the bank, and a tick's lag would draw the previous one.
        if let Ok(mut sound) = self.sound.lock() {
            *sound = Sound::SoundFont {
                path: path.clone(),
                defects: bank.defects().clone(),
            };
        }
        self.dispatch(Job::SetSoundFont {
            bank: Box::new(bank),
            path,
            close,
        })
    }

    /// Every output the machine could play through.
    ///
    /// Enumerated on the calling thread rather than the audio one: it opens nothing, and routing it
    /// through the command channel would mean a request waiting behind whatever housekeeping the
    /// audio thread happens to be doing.
    pub fn output_devices(&self) -> Result<Vec<OutputDevice>, AudioError> {
        device::list_outputs(self.output_status().requested.as_deref())
    }

    /// Where the sound is going, and where it was asked to go.
    pub fn output_status(&self) -> OutputStatus {
        self.output
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    fn dispatch(&self, job: Job) -> bool {
        match &self.commands {
            Some(sender) => sender.send(job).is_ok(),
            None => false,
        }
    }

    /// Where playback has reached, in milliseconds.
    pub fn position_ms(&self) -> u32 {
        self.shared.position_ms()
    }

    /// How much audio one device callback covers, in milliseconds; 0 before any stream has run.
    ///
    /// The step size of [`Engine::position_ms`], and therefore of anything drawn from it. The display
    /// uses it to smooth the staircase; nothing else needs it.
    pub fn period_ms(&self) -> u32 {
        self.shared.period_ms()
    }

    /// How long the loaded song has been silent for want of samples; 0 on a healthy song.
    ///
    /// Cumulative for the song and reset by the next one, so the frame meter differences it. The
    /// authoritative per-song total is reported by `collect_retired` instead — this is the live
    /// reading, which exists so a measurement session can see a stall as it happens rather than
    /// only once the song is over.
    pub fn starved_ms(&self) -> u32 {
        self.shared.starved_ms()
    }

    /// Recoverable stream errors since the stream opened; how a MIDI song reports falling behind.
    ///
    /// The counterpart to `starved_ms` for the one song kind that has no decoder feed to run dry.
    /// Cumulative for the stream rather than the song, so a caller wanting a rate differences it.
    pub fn xruns(&self) -> u32 {
        self.shared.xruns()
    }

    /// Where playback has reached, in ticks. What the display's lyric highlight rides on.
    pub fn position_ticks(&self) -> u32 {
        self.shared.position_ticks()
    }

    /// What the transport is doing, as the audio thread last left it.
    pub fn transport(&self) -> Transport {
        self.shared.transport()
    }

    /// How many songs have run to their end since startup.
    ///
    /// A counter rather than a flag, so a poller that misses a tick still notices. This is how the
    /// queue learns to advance.
    pub fn songs_ended(&self) -> u32 {
        self.shared.songs_ended()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        // Closing the channel is what actually stops the thread; the flag only shortens the wait on
        // a quiet tick.
        self.commands = None;
    }
}

/// Picks a SoundFont: the configured path, then the bundled candidates.
///
/// The candidates are resolved against `paths.asset_dir` rather than the working directory, which is
/// the difference between finding a bundled bank and silently falling back to the test tone on any
/// launch that did not start in the install root.
pub(crate) fn resolve_soundfont(
    configured: Option<&Path>,
    paths: &Paths,
) -> Result<PathBuf, String> {
    if let Some(path) = configured {
        return if path.is_file() {
            Ok(path.to_path_buf())
        } else {
            // Named explicitly and not there. Said precisely, rather than quietly falling back to a
            // bundled bank the operator did not ask for.
            Err(format!("{} is not a file", path.display()))
        };
    }
    let candidates: Vec<PathBuf> = SOUNDFONT_SUBPATHS
        .iter()
        .map(|name| paths.asset(name))
        .collect();
    if let Some(found) = candidates.iter().find(|path| path.is_file()) {
        return Ok(found.clone());
    }
    // The full paths, not the bare names: "none of soundfont/gm.sf2, … exist" would leave an
    // operator guessing which directory the machine actually looked in.
    Err(format!(
        "none of {} exist and none was configured",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// What the audio thread reports once it knows what it found.
struct Ready {
    sound: Sound,
    /// The state object the callbacks write, which outlives any one stream. `None` when there is
    /// no device and no stream will ever be opened.
    shared: Option<Arc<SharedState>>,
}

/// What each open builds its synthesizer from.
///
/// The bank is parsed once, at startup — which is what the ten-second budget in [`Engine::start`]
/// was always for — and every subsequent open clones an `Arc` instead of re-reading tens of
/// megabytes from disk.
enum Instrument {
    Bank(Bank),
    TestTone,
}

impl Instrument {
    fn open(
        &self,
        shared: &Arc<SharedState>,
        want: Option<&str>,
    ) -> Result<OutputStream, AudioError> {
        match self {
            Self::Bank(bank) => OutputStream::open(Arc::clone(shared), want, |rate| {
                SoundFontSource::from_bank(bank, rate)
            }),
            Self::TestTone => OutputStream::open(Arc::clone(shared), want, |rate| {
                Ok(TestToneSource::new(rate))
            }),
        }
    }
}

/// The machine's sound, for a run that renders it instead of playing it.
///
/// **Driven by whoever wants the samples**, which is what makes a streamed run's picture and sound
/// agree: the caller renders a fixed number of samples, draws one frame, and the two advance
/// together because one loop does both.
#[cfg(feature = "video")]
pub(crate) struct StreamAudio {
    renderer: StreamRenderer,
    jobs: mpsc::Receiver<Job>,
    /// A bank that arrived while a song was playing, waiting for the deck to empty.
    pending: Option<Instrument>,
}

#[cfg(feature = "video")]
impl StreamAudio {
    /// Does whatever has been asked since the last block, then fills `out`.
    pub(crate) fn render(&mut self, out: &mut [f32]) {
        while let Ok(job) = self.jobs.try_recv() {
            match job {
                Job::Player(command) => self.renderer.apply(command),
                // **Deferred to the next song, whatever `close` asked for.** On the device path a
                // bank swap that asks to be heard at once drops the stream, which takes the playing
                // song with it, and the machine sends the song again. Here that would be a stream
                // that stopped mid-song in front of people with no way to see why, so the swap
                // waits for the deck to empty — which is at most one song away.
                Job::SetSoundFont { bank, path, close } => {
                    tracing::info!(path = %path.display(), close, "the SoundFont was changed");
                    self.pending = Some(Instrument::Bank(*bank));
                }
                // Nothing leaves this machine through a device, so there is none to change.
                Job::SetOutputDevice(_) => {
                    tracing::info!("a streaming machine has no output device to change");
                }
            }
        }
        if let Some(instrument) = self.pending.take() {
            if self.renderer.is_idle() {
                self.renderer.replace(&instrument);
            } else {
                self.pending = Some(instrument);
            }
        }
        self.renderer.render(out);
    }
}

/// One of the two things a renderer can be playing.
///
/// **An enum rather than a boxed source**, because the two sources are different types and a
/// `Renderer` names the one it holds. Two variants and three forwarding methods is less machinery
/// than making a synthesizer object-safe for the sake of a switch that happens once a run.
#[cfg(feature = "video")]
enum StreamRenderer {
    Bank(km_audio::Renderer<SoundFontSource>),
    TestTone(km_audio::Renderer<TestToneSource>),
}

#[cfg(feature = "video")]
impl StreamRenderer {
    /// A renderer over whichever instrument was chosen.
    ///
    /// **A bank that will not build falls back to the test tone**, which is the same degraded mode
    /// `choose_instrument` already describes: a machine that says what it is playing beats one that
    /// refuses to start.
    fn open(
        instrument: &Instrument,
        shared: &Arc<SharedState>,
        sample_rate: u32,
        channels: usize,
    ) -> Self {
        match instrument {
            Instrument::Bank(bank) => match SoundFontSource::from_bank(bank, sample_rate) {
                Ok(source) => Self::Bank(km_audio::Renderer::new(
                    source,
                    Arc::clone(shared),
                    sample_rate,
                    channels,
                )),
                Err(error) => {
                    tracing::error!(%error, "the bank would not build; playing a test tone");
                    Self::TestTone(km_audio::Renderer::new(
                        TestToneSource::new(sample_rate),
                        Arc::clone(shared),
                        sample_rate,
                        channels,
                    ))
                }
            },
            Instrument::TestTone => Self::TestTone(km_audio::Renderer::new(
                TestToneSource::new(sample_rate),
                Arc::clone(shared),
                sample_rate,
                channels,
            )),
        }
    }

    fn apply(&mut self, command: Command) {
        match self {
            Self::Bank(renderer) => renderer.apply(command),
            Self::TestTone(renderer) => renderer.apply(command),
        }
    }

    fn render(&mut self, out: &mut [f32]) {
        match self {
            Self::Bank(renderer) => renderer.render(out),
            Self::TestTone(renderer) => renderer.render(out),
        }
    }

    fn is_idle(&self) -> bool {
        match self {
            Self::Bank(renderer) => renderer.is_idle(),
            Self::TestTone(renderer) => renderer.is_idle(),
        }
    }

    /// Plays through a different bank from now on.
    ///
    /// Only reached while nothing is loaded, so there is no song to carry across; what the old
    /// source's settings were is carried by the renderer itself.
    fn replace(&mut self, instrument: &Instrument) {
        let (rate, channels) = match self {
            Self::Bank(renderer) => (renderer.sample_rate(), renderer.channels()),
            Self::TestTone(renderer) => (renderer.sample_rate(), renderer.channels()),
        };
        match (&mut *self, instrument) {
            (Self::Bank(renderer), Instrument::Bank(bank)) => {
                match SoundFontSource::from_bank(bank, rate) {
                    Ok(source) => renderer.replace_source(source),
                    Err(error) => {
                        tracing::error!(%error, "the new bank would not build; keeping the old one")
                    }
                }
            }
            // Changing which *kind* of instrument is playing means a new renderer, and the settings
            // the old one carried go with the source rather than with the switch.
            _ => {
                let shared = match self {
                    Self::Bank(renderer) => renderer.state(),
                    Self::TestTone(renderer) => renderer.state(),
                };
                *self = Self::open(instrument, &Arc::clone(shared), rate, channels);
            }
        }
    }
}

/// Picks the instrument and says which of the three audio states the machine is in.
///
/// Separate from opening a device on purpose: a bank that exists but will not parse used to reach
/// `OutputStream::open` as an `AudioError::Source` and be reported as `Sound::Silent` — "no audio
/// output", playback refused — when the documented degraded mode for a bad bank is the test tone.
fn choose_instrument(soundfont: Result<PathBuf, String>) -> (Instrument, Sound) {
    match soundfont {
        Ok(path) => match Bank::load(&path) {
            Ok(bank) => {
                let defects = bank.defects().clone();
                (Instrument::Bank(bank), Sound::SoundFont { path, defects })
            }
            Err(error) => (
                Instrument::TestTone,
                Sound::TestTone {
                    reason: error.to_string(),
                },
            ),
        },
        Err(reason) => (Instrument::TestTone, Sound::TestTone { reason }),
    }
}

/// Whether a command means the machine is about to make a sound.
///
/// `Load` is on the list and it is the one that matters: the machine always sends `Load` before
/// `Play`. The `Set*` commands are not — they are knobs a person turns while nothing is playing,
/// and opening the sound card to record a volume change is the whole fault being fixed here.
/// `Unload`, `Pause` and `Stop` are not either: with no device there is nothing to stop.
fn needs_device(command: &Command) -> bool {
    match command {
        Command::Wake
        | Command::Load(..)
        | Command::Play
        | Command::Restart
        | Command::SeekMs(_) => true,
        Command::Unload
        | Command::Pause
        | Command::Stop
        | Command::SetTranspose(_)
        | Command::SetTempoRatio(_)
        | Command::SetMelodyEnabled(_)
        | Command::SetMusicVolume(_)
        // Beside the volume, and for the same reason: it is a level, not a sound. It happens to
        // arrive with a `Load` at every song start, and the `Load` is what opens the device.
        | Command::SetSongGain(_) => false,
    }
}

/// Whether the output device should be handed back now.
///
/// Pure on purpose: CI has no audio hardware, so the policy is a function of four values and every
/// branch of it is tested.
///
/// **Only `Idle` releases.** `Stopped` and `Paused` both mean a song is loaded *inside the player
/// the stream owns*, so closing would drop it while the machine went on showing its title — and the
/// next `Play` would reach a fresh player with no song and do nothing at all. Silent, permanent, and
/// indistinguishable from a broken app. Releasing those would mean remembering the song and its
/// position here and replaying `Load` and `SeekMs` on reopen, which trades a certain bug for a
/// subtle one. It costs nothing to exclude them: this machine never rests in `Stopped` — every
/// `Stop` is followed by an `Unload` — and a song left paused is a person standing at the machine,
/// not the idle process the fault is about.
///
/// `release_after` of `None` means never, which is `audio.idle_release_secs: 0`.
fn should_release(
    transport: Transport,
    idle_for: Duration,
    release_after: Option<Duration>,
    work_pending: bool,
) -> bool {
    let Some(after) = release_after else {
        return false;
    };
    !work_pending && transport == Transport::Idle && idle_for >= after
}

/// Settings that live in the player, and so would be lost when the device is handed back.
///
/// A fresh `Player` starts at unity volume and default playback settings, so these four are
/// replayed after every open. In the ordinary path that is redundant — the machine sends all four
/// itself between `Load` and `Play` — but depending on a caller's ordering for correctness is how
/// this would quietly break the first time that changed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sticky {
    transpose: i8,
    tempo_ratio: f32,
    melody_enabled: bool,
    music_volume: f32,
    /// The levelling gain of the song that is loaded.
    ///
    /// **Here for the same reason the four above are, and it matters most for a video.** A stream
    /// rebuilt under a MIDI song is followed by a seek that puts the song back where it was; the
    /// gain has to come with it or the song resumes at the wrong level halfway through.
    song_gain: f32,
}

impl Sticky {
    /// Where a freshly built player starts.
    ///
    /// Taken from `PlaybackSettings::default` rather than restated, because the point of this type
    /// is to match what a new `Player` begins with — and a copy of those values here would be
    /// correct right up until somebody changed one of them.
    fn new(music_volume: f32) -> Self {
        let fresh = PlaybackSettings::default();
        Self {
            transpose: fresh.transpose,
            tempo_ratio: fresh.tempo_ratio,
            melody_enabled: fresh.melody_enabled,
            music_volume,
            // A fresh `Player` starts unlevelled, and so does this: nothing is loaded yet, so there
            // is no song for a gain to belong to.
            song_gain: 1.0,
        }
    }

    /// Notes anything that would have to be replayed after a reopen.
    fn remember(&mut self, command: &Command) {
        match *command {
            Command::SetTranspose(semitones) => self.transpose = semitones,
            Command::SetTempoRatio(ratio) => self.tempo_ratio = ratio,
            Command::SetMelodyEnabled(enabled) => self.melody_enabled = enabled,
            Command::SetMusicVolume(volume) => self.music_volume = volume,
            Command::SetSongGain(gain) => self.song_gain = gain,
            _ => {}
        }
    }

    /// The commands that put a fresh player back where the last one was.
    fn replay(self) -> [Command; 5] {
        [
            Command::SetTranspose(self.transpose),
            Command::SetTempoRatio(self.tempo_ratio),
            Command::SetMelodyEnabled(self.melody_enabled),
            Command::SetMusicVolume(self.music_volume),
            Command::SetSongGain(self.song_gain),
        ]
    }
}

/// How long to wait before retrying an open that failed, by attempt.
///
/// A Bluetooth link caught mid-reconnect needs a moment; four seconds of trying is enough for that
/// and short enough that a song which is never going to start says so while somebody is still
/// standing there.
const OPEN_RETRIES: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(3),
];

/// The output device, held only while there is something to play.
struct Held {
    stream: Option<OutputStream>,
    sticky: Sticky,
    /// When the transport last became releasable, or `None` while it is not.
    idle_since: Option<Instant>,
    /// A command that arrived while the device would not open, and when to try again.
    deferred: Option<(Command, Instant)>,
    attempts: usize,
    /// Which device to open, as `settings.audio.output_device` spells it.
    ///
    /// Re-read on every open rather than resolved once: a device can be unplugged between one song
    /// and the next, and the answer to that is a fresh fallback rather than a stale handle.
    want: Option<String>,
    /// Shared with the [`Engine`] handle so the API can say where the sound is going.
    output: Arc<Mutex<OutputStatus>>,
}

impl Held {
    fn new(music_volume: f32, want: Option<String>, output: Arc<Mutex<OutputStatus>>) -> Self {
        Self {
            stream: None,
            sticky: Sticky::new(music_volume),
            idle_since: None,
            deferred: None,
            attempts: 0,
            want,
            output,
        }
    }

    /// Records which device is now in use, for the API to read.
    fn publish(&self, active: Chosen) {
        if let Ok(mut status) = self.output.lock() {
            status.requested = self.want.clone();
            status.active = active;
        }
    }

    /// Opens the device if it is not already open, replaying the sticky settings onto the new
    /// player. Returns whether a stream is available afterwards.
    fn ensure_open(&mut self, instrument: &Instrument, shared: &Arc<SharedState>) -> bool {
        if self.stream.is_some() {
            return true;
        }
        match instrument.open(shared, self.want.as_deref()) {
            Ok(mut stream) => {
                for command in self.sticky.replay() {
                    stream.send(command);
                }
                tracing::debug!(
                    sample_rate = stream.sample_rate(),
                    channels = stream.channels(),
                    device = %stream.chosen().name,
                    "opened the output device"
                );
                self.publish(stream.chosen().clone());
                self.stream = Some(stream);
                self.attempts = 0;
                // Start the idle clock again. Without this a `Wake` would be pointless: it opens the
                // device without loading anything, so the transport is still `Idle`, and a machine
                // that had been quiet for an hour would close the device on the very next tick —
                // just in time for the song it was opened for.
                self.idle_since = None;
                true
            }
            Err(error) => {
                tracing::warn!(%error, "could not open the output device");
                false
            }
        }
    }

    /// Hands the device back.
    fn close(&mut self, why: &'static str) {
        if self.stream.take().is_some() {
            // The drop joins cpal's worker thread, so this blocks for about one device period. That
            // is fine here and would not be anywhere near the callback -- which is why it is here.
            tracing::debug!(why, "released the output device");
        }
        self.idle_since = None;
    }

    /// Sends a command, or holds on to it if the device could not be opened.
    fn forward(&mut self, command: Command) {
        match &mut self.stream {
            Some(stream) => {
                if !stream.send(command) {
                    tracing::error!("the audio command queue is full; the device may have stopped");
                }
            }
            None => {
                if needs_device(&command) {
                    let wait = OPEN_RETRIES[self.attempts.min(OPEN_RETRIES.len() - 1)];
                    self.deferred = Some((command, Instant::now() + wait));
                    self.attempts += 1;
                } // Anything else is a knob turned with no device; `sticky` already has it.
            }
        }
    }
}

/// The audio thread.
#[allow(clippy::too_many_arguments)]
fn run(
    soundfont: Result<PathBuf, String>,
    music_volume: f32,
    idle_release: Option<Duration>,
    want: Option<String>,
    commands: mpsc::Receiver<Job>,
    ready: mpsc::Sender<Ready>,
    stopping: Arc<AtomicBool>,
    output: Arc<Mutex<OutputStatus>>,
) {
    let shared = Arc::new(SharedState::default());

    // Is there a device at all? Asked without opening one, so an idle machine holds nothing.
    //
    // This is also where the USB preference applies: `probe` resolves `want`, and where nothing has
    // been chosen that means preferring a USB interface on Linux. What it picked is published below
    // for the log and the API to read — and deliberately not written into settings. It is a
    // preference, re-made on every start, so an interface that was unplugged last night is used
    // again tonight. See `Choosing the audio output device` in docs/decisions/audio.md.
    match OutputStream::probe(want.as_deref()) {
        Ok(device) => {
            shared.publish_device(device.sample_rate, device.channels);
            if let Ok(mut status) = output.lock() {
                status.requested = want.clone();
                status.active = device.chosen;
            }
        }
        Err(error) => {
            // No device *at all* — not merely not the one that was asked for, which falls back
            // inside `probe` and never reaches here. Report it and stop; the machine runs, without
            // playback.
            let _ = ready.send(Ready {
                sound: Sound::Silent {
                    reason: error.to_string(),
                },
                shared: None,
            });
            return;
        }
    }

    let (mut instrument, sound) = choose_instrument(soundfont);
    let reported = ready.send(Ready {
        sound,
        shared: Some(Arc::clone(&shared)),
    });
    if reported.is_err() {
        // Nobody is listening any more: the handle was dropped during startup.
        return;
    }

    let mut held = Held::new(music_volume, want, output);

    loop {
        match commands.recv_timeout(HOUSEKEEPING_INTERVAL) {
            Ok(Job::Player(command)) => {
                held.sticky.remember(&command);
                if needs_device(&command) {
                    held.ensure_open(&instrument, &shared);
                }
                held.forward(command);
            }
            Ok(Job::SetOutputDevice(next)) => {
                // Dropping the stream is the whole mechanism: the next thing that means sound opens
                // the new device, exactly as it does after an idle release. Nothing is reopened
                // eagerly, because a machine told to change device is by definition not playing —
                // the caller refuses otherwise, since the player lives inside this stream.
                tracing::info!(
                    device = next.as_deref().unwrap_or("(unset)"),
                    "the output device was changed"
                );
                held.want = next;
                held.deferred = None;
                held.attempts = 0;
                held.close("the output device was changed");
                held.publish(unknown_output());
            }
            Ok(Job::SetSoundFont { bank, path, close }) => {
                tracing::info!(path = %path.display(), close, "the SoundFont was changed");
                instrument = Instrument::Bank(*bank);
                if close {
                    // The same three lines as the device change above, and for the same reason: a
                    // pending open would reopen on the bank being replaced. What is deliberately
                    // *not* here is `publish(unknown_output())` — the device has not changed, and
                    // saying "not yet opened" would make a bank swap look like a routing change.
                    held.deferred = None;
                    held.attempts = 0;
                    held.close("the SoundFont was changed");
                }
                // `publish_stopped()` is not called either. The caller re-sends the song and its
                // position, so the transport atomic staying at `Playing` across the gap is correct
                // — and the queue, which advances on `songs_ended` alone, never notices.
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // Freeing a song's event vector is exactly the unbounded work the callback must not do, so
        // it hands the Arc back through a second queue and this thread drops it.
        if let Some(stream) = &mut held.stream {
            stream.collect_retired();
        }
        // The flag stays set until something opens a stream again, so the test is "is there still a
        // stream to drop?" rather than the flag alone.
        if held.stream.is_some() && shared.stream_failed() {
            // The device is gone -- and only that, now: a recoverable underrun no longer sets this
            // flag, because cpal recovers from one itself and killing the stream over it made the
            // machine unplayable through ALSA's `dmix`. See `on_error` in km-audio.
            //
            // Say playback has stopped before dropping the stream. The transport atomic is written
            // from inside the callback, so a stream that goes away mid-song freezes it at `Playing`
            // with the position stuck beside it, and the machine reports a song playing forever at a
            // standstill. Nobody can tell that from a hang, which is exactly what it was.
            if shared.transport() == km_queue::Transport::Playing {
                tracing::warn!("the audio device went away mid-song; reporting playback stopped");
                shared.publish_stopped();
            }
            held.close("the device is gone");
        }
        tick(
            &mut held,
            &instrument,
            &shared,
            idle_release,
            Instant::now(),
        );

        if stopping.load(Ordering::Acquire) {
            break;
        }
    }
    tracing::debug!("the audio thread is stopping");
}

/// When the machine went quiet, given when it had gone quiet before.
///
/// Anything but `Idle` stops the clock. `Idle` starts it if it was not already running, and leaves
/// it alone if it was — the delay is measured from when the machine *became* quiet, not from the
/// last time anybody looked.
fn idle_clock(previous: Option<Instant>, transport: Transport, now: Instant) -> Option<Instant> {
    match transport {
        Transport::Idle => previous.or(Some(now)),
        _ => None,
    }
}

/// One housekeeping pass: retry a deferred open, then release the device if it has gone quiet.
fn tick(
    held: &mut Held,
    instrument: &Instrument,
    shared: &Arc<SharedState>,
    idle_release: Option<Duration>,
    now: Instant,
) {
    if let Some((_, due)) = &held.deferred
        && *due <= now
    {
        let (command, _) = held.deferred.take().expect("just matched");
        if held.ensure_open(instrument, shared) {
            held.forward(command);
        } else if held.attempts >= OPEN_RETRIES.len() {
            tracing::error!("gave up opening the output device; the song will not start");
            held.attempts = 0;
        } else {
            held.forward(command);
        }
    }

    let transport = shared.transport();
    held.idle_since = idle_clock(held.idle_since, transport, now);
    let idle_for = held
        .idle_since
        .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
    if held.stream.is_some()
        && should_release(transport, idle_for, idle_release, held.deferred.is_some())
    {
        held.close("idle");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_engine_refuses_to_play_but_is_still_usable() {
        let engine = Engine::silent("no device in CI");
        assert!(!engine.can_play());
        assert_eq!(engine.transport(), Transport::Idle);
        assert_eq!(engine.position_ms(), 0);
        // Commands are dropped rather than panicking, so every caller need not special-case it.
        assert!(!engine.send(Command::Play));
    }

    /// The two `Option` states that are not the same thing, and once were.
    ///
    /// `None` means "nothing has ever been chosen", which is what makes the USB preference fire.
    /// `Some(SYSTEM_DEFAULT)` means "follow the system, deliberately", which stops it applying at all.
    /// Collapsing the second into the first is a real bug that got as far as the wire: a successful
    /// `PUT {"id":"system"}` answered `"selected": null`, so a remote could not tell a deliberate
    /// choice from an unconfigured machine — and the next start would have overridden it.
    #[test]
    fn following_the_system_is_a_choice_and_not_an_absent_one() {
        let engine = Engine::silent("no device in CI");
        assert_eq!(engine.output_status().requested, None);

        engine.set_output_device(Some(km_audio::SYSTEM_DEFAULT.to_owned()));
        assert_eq!(
            engine.output_status().requested.as_deref(),
            Some(km_audio::SYSTEM_DEFAULT)
        );

        // The status is updated even with no audio thread to accept the job, because the API answers
        // from it on the same call that made the change.
        engine.set_output_device(Some("alsa:plughw:CARD=Device,DEV=0".to_owned()));
        let status = engine.output_status();
        assert_eq!(
            status.requested.as_deref(),
            Some("alsa:plughw:CARD=Device,DEV=0")
        );
        // ...and nothing is claimed to be playing through it yet.
        assert_eq!(status.active, unknown_output());
    }

    #[test]
    fn a_silent_engine_says_why_in_words_worth_showing_someone() {
        let engine = Engine::silent("no audio output device is available");
        let text = engine.sound().describe();
        assert!(text.contains("no audio output"));
        assert!(text.contains("playback unavailable"));
    }

    /// A `Paths` rooted somewhere that certainly holds no SoundFont.
    fn nowhere() -> Paths {
        Paths::rooted_at("definitely/not/an/install/root")
    }

    /// A path handed to this that is not there is still refused — as a guard, not as the answer to
    /// a missing bank.
    ///
    /// **This is a race check rather than the product behavior.** `audio.soundfont` names a bank
    /// **id**, and `soundfont::resolve` turns it into a path only if the folder actually holds one
    /// — so the only way to reach this branch is a file deleted between the scan and the load.
    /// Worth keeping for exactly that, and worth not mistaking for the fallback rule, which lives
    /// in `soundfont::resolve`.
    #[test]
    fn a_missing_configured_soundfont_is_named_rather_than_silently_replaced() {
        let error = resolve_soundfont(Some(Path::new("nowhere/absent.sf2")), &nowhere())
            .expect_err("the file does not exist");
        // The operator asked for a specific bank; falling back without saying so would leave them
        // wondering why everything sounds like a sine wave.
        assert!(error.contains("absent.sf2"));
    }

    /// Five seconds: the default release delay.
    const RELEASE: Option<Duration> = Some(Duration::from_secs(5));

    #[test]
    fn an_idle_machine_gives_the_device_back() {
        assert!(should_release(
            Transport::Idle,
            Duration::from_secs(5),
            RELEASE,
            false
        ));
    }

    #[test]
    fn only_an_idle_machine_gives_the_device_back() {
        // Stopped and Paused both mean a song is loaded inside the player the stream owns. Closing
        // would drop it while the screen still showed its title, and the next Play would reach a
        // fresh player with nothing in it and do nothing at all.
        for transport in [Transport::Playing, Transport::Paused, Transport::Stopped] {
            assert!(
                !should_release(transport, Duration::from_secs(3600), RELEASE, false),
                "{transport:?} must keep the device"
            );
        }
    }

    #[test]
    fn the_delay_has_to_have_elapsed() {
        assert!(!should_release(
            Transport::Idle,
            Duration::from_millis(4_999),
            RELEASE,
            false
        ));
    }

    #[test]
    fn no_delay_configured_means_never_rather_than_immediately() {
        // The reading that would be a disaster: releasing the device between every buffer. Zero
        // seconds in the settings is how a dedicated box says "this sound card is mine".
        assert!(!should_release(
            Transport::Idle,
            Duration::from_secs(86_400),
            None,
            false
        ));
    }

    #[test]
    fn a_pending_open_is_not_released_out_from_under_itself() {
        // A song is on its way in; the transport has not moved yet because nothing is playing. The
        // release timer must not win that race.
        assert!(!should_release(
            Transport::Idle,
            Duration::from_secs(60),
            RELEASE,
            true
        ));
    }

    #[test]
    fn the_idle_clock_runs_from_when_the_machine_went_quiet() {
        let start = Instant::now();
        let later = start + Duration::from_secs(30);

        let clock = idle_clock(None, Transport::Idle, start);
        assert_eq!(clock, Some(start));
        // Measured from when it went quiet, not from the last time anybody looked -- otherwise
        // every housekeeping tick would push the deadline out and the device would never be freed.
        assert_eq!(idle_clock(clock, Transport::Idle, later), Some(start));

        for transport in [Transport::Playing, Transport::Paused, Transport::Stopped] {
            assert_eq!(idle_clock(clock, transport, later), None, "{transport:?}");
        }
    }

    #[test]
    fn only_the_commands_that_make_sound_open_the_device() {
        assert!(needs_device(&Command::Wake));
        assert!(needs_device(&Command::Play));
        assert!(needs_device(&Command::Restart));
        assert!(needs_device(&Command::SeekMs(0)));
        // Turning a knob while nothing is playing must not wake the sound card -- that is the whole
        // fault being fixed.
        assert!(!needs_device(&Command::SetTranspose(2)));
        assert!(!needs_device(&Command::SetTempoRatio(1.1)));
        assert!(!needs_device(&Command::SetMelodyEnabled(true)));
        assert!(!needs_device(&Command::SetMusicVolume(0.5)));
        // With no device there is nothing to stop.
        assert!(!needs_device(&Command::Unload));
        assert!(!needs_device(&Command::Pause));
        assert!(!needs_device(&Command::Stop));
    }

    #[test]
    fn the_sticky_settings_are_the_last_value_of_each() {
        let mut sticky = Sticky::new(1.0);
        for command in [
            Command::SetTranspose(2),
            Command::SetMusicVolume(0.3),
            Command::Play,
            Command::SetTranspose(-1),
        ] {
            sticky.remember(&command);
        }
        assert_eq!(sticky.transpose, -1);
        assert_eq!(sticky.music_volume, 0.3);
        // Untouched by anything in that stream, so still the defaults.
        assert_eq!(sticky.tempo_ratio, 1.0);
        assert!(!sticky.melody_enabled);
    }

    #[test]
    fn replaying_the_sticky_settings_sends_only_settings() {
        let sticky = Sticky::new(0.8);
        let replayed = sticky.replay();
        // Five since levelling: the four playback settings and the loaded song's own gain.
        assert_eq!(replayed.len(), 5);
        // A replay that carried a Load or a Play would restart a song the machine did not ask for.
        for command in &replayed {
            assert!(
                !needs_device(command),
                "the replay must not wake the device by itself: {command:?}"
            );
        }
    }

    #[test]
    fn a_bank_that_will_not_parse_falls_back_to_the_test_tone() {
        let dir = std::env::temp_dir().join("km-app-engine-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("corrupt.sf2");
        std::fs::write(&path, b"this is not a SoundFont").expect("write");

        let (_, sound) = choose_instrument(Ok(path.clone()));
        // It used to reach `OutputStream::open` as a source error and be reported as "no audio
        // output", refusing playback. A bad bank is the documented test-tone case, not the
        // no-device case.
        assert!(
            matches!(sound, Sound::TestTone { .. }),
            "a corrupt bank should degrade to the test tone, got {sound:?}"
        );
        assert!(sound.can_play());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn with_nothing_configured_the_bundled_candidates_are_named_in_the_error() {
        let error = resolve_soundfont(None, &nowhere()).expect_err("nothing is installed there");
        // The message must name the directory actually searched, not just the file names: the whole
        // point of resolving against the asset directory is that it is not the working directory,
        // and an error that hides which one it used sends the operator to the wrong folder.
        assert!(
            error.contains("not/an/install/root"),
            "the error should name the asset directory it searched: {error}"
        );
        assert!(error.contains("gm.sf2"), "and the names it tried: {error}");
    }

    #[test]
    fn the_bundled_candidates_are_resolved_against_the_asset_directory() {
        // The regression this guards: working-directory-relative candidates leave an install whose
        // assets live elsewhere -- every Android install, since the working directory there is `/`
        // -- silently falling back to the test tone.
        let dir = std::env::temp_dir().join("km-soundfont-resolution-test/assets/soundfont");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let bank = dir.join("gm.sf2");
        std::fs::write(&bank, b"not a real bank, and not parsed here").expect("write");

        let paths = Paths::rooted_at(std::env::temp_dir().join("km-soundfont-resolution-test"));
        let chosen = resolve_soundfont(None, &paths).expect("the bundled bank should be found");
        assert_eq!(chosen, bank);

        std::fs::remove_file(&bank).ok();
    }

    #[test]
    fn a_configured_soundfont_that_exists_is_chosen() {
        // Any existing file will do: this checks the selection, not the parsing.
        let existing = Path::new("Cargo.toml");
        assert_eq!(
            resolve_soundfont(Some(existing), &nowhere()).expect("chosen"),
            existing.to_path_buf()
        );
    }

    /// The checkout overlay beats the bundled bank, which is what makes
    /// `tools/setup/fetch-assets.sh --bank <name>` need no `settings.json` edit at all. It also pins the
    /// reason that script installs an override as `gm.sf2`: that name is first in
    /// `SOUNDFONT_SUBPATHS`, so it wins over a bundled `GeneralUser-GS.sf2` with no ordering logic.
    #[test]
    fn an_overlay_bank_is_preferred_to_the_bundled_one() {
        let root = std::env::temp_dir().join("km-soundfont-overlay-test");
        let bundled = root.join("assets/soundfont/GeneralUser-GS.sf2");
        let overlay = root.join("local/assets/soundfont/gm.sf2");
        for file in [&bundled, &overlay] {
            std::fs::create_dir_all(file.parent().expect("a parent")).expect("temp dir");
            std::fs::write(file, b"not a real bank, and not parsed here").expect("write");
        }

        let mut paths = Paths::rooted_at(&root);
        paths.overlay_asset_dir = Some(root.join("local/assets"));
        assert_eq!(
            resolve_soundfont(None, &paths).expect("a bank should be found"),
            overlay
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// `audio.soundfont` still wins outright — the overlay is a rule, not a fourth setting.
    #[test]
    fn a_configured_soundfont_still_beats_an_overlay() {
        let root = std::env::temp_dir().join("km-soundfont-overlay-configured");
        let overlay = root.join("local/assets/soundfont/gm.sf2");
        std::fs::create_dir_all(overlay.parent().expect("a parent")).expect("temp dir");
        std::fs::write(&overlay, b"not a real bank").expect("write");

        let mut paths = Paths::rooted_at(&root);
        paths.overlay_asset_dir = Some(root.join("local/assets"));
        let existing = Path::new("Cargo.toml");
        assert_eq!(
            resolve_soundfont(Some(existing), &paths).expect("chosen"),
            existing.to_path_buf()
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_three_sounds_differ_in_whether_playback_is_possible() {
        assert!(
            Sound::SoundFont {
                path: PathBuf::from("gm.sf2"),
                defects: km_audio::BankDefects::default(),
            }
            .can_play()
        );
        // A test tone is wrong-sounding, not unusable: the lyrics still scroll in time, which is
        // most of what a karaoke machine does.
        assert!(
            Sound::TestTone {
                reason: "none found".to_owned()
            }
            .can_play()
        );
        assert!(
            !Sound::Silent {
                reason: "no device".to_owned()
            }
            .can_play()
        );
    }

    #[test]
    fn a_test_tone_says_instruments_will_sound_wrong() {
        let text = Sound::TestTone {
            reason: "no bank installed".to_owned(),
        }
        .describe();
        // A singer hearing sine waves should learn the machine is unconfigured, not faulty.
        assert!(text.contains("test tone"));
        assert!(text.contains("sound wrong"));
    }
}
