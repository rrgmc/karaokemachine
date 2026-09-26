//! Running a player on a real audio device.
//!
//! The [`Player`] lives on the audio thread. Everything else talks to it through a lock-free
//! command queue, and reads its position back through atomics — no mutex is ever taken on the
//! real-time thread.
//!
//! One detail that is easy to miss: when a new song replaces an old one, the old `Arc<Song>` would
//! be dropped *on the audio thread*, and freeing a few megabytes of event vector is exactly the kind
//! of unbounded work a real-time callback must not do. Retired songs are therefore handed back to
//! the control thread through a second queue and dropped there. A video song's audio feed goes back
//! the same way, for the same reason — see [`crate::player::Retired`].
//!
//! **The device is not held while nothing is playing.** A stream can be opened and dropped many
//! times over one run, which is why [`SharedState`] is created by the caller and handed in: the
//! machine holds that `Arc` for its whole life, so a reopen that made a fresh one would leave every
//! position and transport reading permanently dead. [`OutputStream::probe`] answers "is there a
//! device, and at what rate?" without opening anything, so the no-device case is still known at
//! startup.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use km_queue::Transport;
use km_song::Song;

use crate::device::{self, Chosen};
use crate::player::{Player, PlayerEvent, Retired};
use crate::source::{AudioSource, SourceError};
use crate::track::TrackPlayer;

/// What to load: the two kinds of song a machine can play.
///
/// Both arrive ready to play. A MIDI song is parsed on the control thread and a video song's audio
/// is already connected to its decoder, because neither parsing nor opening a file can happen where
/// this is applied.
#[derive(Debug)]
pub enum Load {
    /// A parsed MIDI song, with the melody channel and the corrections recorded for it.
    Midi {
        /// The parsed song.
        song: Arc<Song>,
        /// The melody channel, when packaging detected one confidently.
        melody_channel: Option<u8>,
        /// Corrections for defects in this song's own events.
        ///
        /// **Carried inline rather than boxed, which the variant beside it is not.** A box reaching
        /// the sequencer would be freed where the sequencer is, and [`crate::player::Player::retire`]
        /// drops that on the audio thread — handing back only the `Arc<Song>`, on the grounds that
        /// a sequencer is small. Keeping the table flat is what keeps that true. It costs the ring
        /// its width times [`COMMAND_CAPACITY`], which is a few kilobytes once, at startup.
        fixes: km_fixes::ChannelFixes,
    },
    /// A video song's audio, reading from a feed a decoder is filling.
    ///
    /// Boxed to keep [`Command`] small: these sit in a fixed-size ring, so the largest variant sets
    /// the cost of every one of them.
    Track(Box<TrackPlayer>),
}

/// Commands the control thread sends to the audio thread.
#[derive(Debug)]
pub enum Command {
    /// Play this song.
    Load(Load),
    /// Drop the current song.
    Unload,
    /// Start or resume.
    Play,
    /// Stop advancing, silencing held notes.
    Pause,
    /// Return to the start and stop.
    Stop,
    /// Return to the start and keep playing.
    Restart,
    /// Jump to a position in milliseconds.
    SeekMs(u32),
    /// Set the transposition in semitones. This is the tone adjustment control.
    SetTranspose(i8),
    /// Set playback speed as a multiple of the written tempo.
    SetTempoRatio(f32),
    /// Turn the guide melody on or off.
    SetMelodyEnabled(bool),
    /// Set the music volume, 0.0 to 1.0. The owner's level.
    SetMusicVolume(f32),
    /// Set the gain that levels this song against the reference, 0.0 to 1.0.
    ///
    /// Sent at every song start, `1.0` included, and never left to persist. `Sticky` replays the
    /// last value it saw into each new stream, so a MIDI song following a video would inherit the
    /// video's attenuation if a start were allowed to send nothing.
    SetSongGain(f32),
    /// Nothing for the player to do — a hint that sound is imminent.
    ///
    /// The device takes time to come up, and on a Bluetooth link that time is on the order of a
    /// second. This is how the machine says "somebody is choosing a song", so that opening the
    /// device overlaps with them choosing it rather than with the first bar.
    Wake,
}

/// Why the audio device could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// No output device is available.
    #[error("no audio output device is available")]
    NoDevice,
    /// The device would not report its configuration.
    #[error("could not read the device configuration: {0}")]
    Config(String),
    /// The device's sample format is not one we handle.
    #[error("unsupported sample format {0:?}")]
    SampleFormat(cpal::SampleFormat),
    /// The stream could not be created or started.
    #[error("could not start the audio stream: {0}")]
    Stream(String),
    /// The synthesizer could not be built for the device's sample rate.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// The operating system's mixer would not answer.
    ///
    /// A device with no level to report is not this: [`crate::level::read`] answers `Ok(None)` for
    /// one, and a caller that read the two as the same thing would tell an owner their HDMI output
    /// was broken.
    #[error("could not reach the output level: {0}")]
    Mixer(String),
}

/// Transport state as a number, for the atomic.
fn transport_code(transport: Transport) -> u8 {
    match transport {
        Transport::Idle => 0,
        Transport::Playing => 1,
        Transport::Paused => 2,
        Transport::Stopped => 3,
    }
}

fn transport_from_code(code: u8) -> Transport {
    match code {
        1 => Transport::Playing,
        2 => Transport::Paused,
        3 => Transport::Stopped,
        _ => Transport::Idle,
    }
}

/// State the audio thread publishes and everyone else reads.
///
/// Plain atomics rather than a lock: the display reads this once per frame and the audio thread
/// writes it once per block, and neither may ever wait for the other.
#[derive(Debug, Default)]
pub struct SharedState {
    position_ticks: AtomicU32,
    position_ms: AtomicU32,
    transport: AtomicU8,
    /// Incremented each time a song runs to its end, so the control thread can notice without
    /// needing a channel it might miss.
    songs_ended: AtomicU32,
    /// The rate of the stream currently open, or the last one that was.
    ///
    /// Lives here rather than only on [`OutputStream`] because the device is released while idle
    /// and reopened later, possibly onto a different default device at a different rate. A rate
    /// captured once at startup would be a lie by the second song.
    sample_rate: AtomicU32,
    /// The channel count, on the same terms as `sample_rate`. `u32` so it pairs with it.
    channels: AtomicU32,
    /// How much audio the device asks for in one callback, in milliseconds; 0 until a stream has run.
    ///
    /// This is the *granularity of everything below*, not a detail of the device: `position_ms` moves
    /// once per callback, so a display drawing from it is stepping by exactly this much however fast
    /// it redraws. It is published so the display can smooth over it instead of showing the steps.
    /// Learned from the first callback rather than the config, because `BufferSize::Default` means
    /// the backend decides and only the callback knows what it decided.
    period_ms: AtomicU32,
    /// How long the song has been silent for want of samples, in milliseconds; 0 on a healthy song.
    ///
    /// Milliseconds rather than the output frames [`Player::starved_ms`](crate::player::Player::starved_ms) counts, for two
    /// reasons. It is the unit the number is read in — "the song stopped for 3 s" is the report, and
    /// 144,000 frames is not — and it keeps this struct's every field an `AtomicU32`, which
    /// `armeabi-v7a` reaches with a single instruction where a 64-bit one costs a pair.
    ///
    /// **Cumulative for the loaded song, not a rate.** It only ever grows while a song plays and
    /// returns to 0 with the next one, so a reader that wants "did this song stall" compares it
    /// against zero and one that wants "is it stalling now" has to difference it itself.
    starved_ms: AtomicU32,
    /// Recoverable stream errors since the stream opened — in practice, device underruns.
    ///
    /// **This is the only way a MIDI song can fail for want of CPU, and until now nothing counted
    /// it.** A video or MP3+G song reads from a decoder feed, so falling behind shows up as
    /// [`SharedState::starved_ms`]; a MIDI song has no feed, because `rustysynth` renders inside the
    /// callback itself. When *that* misses its deadline the device underruns instead, and the
    /// synthesizer is measured at 82% of one core on the appliance's Cortex-A55 against a 117 ms
    /// period — less headroom than the video decoder had, on the one thread that cannot be given
    /// more cores.
    ///
    /// Counted rather than merely logged because the existing line fires once per event and says
    /// nothing about rate: three in a second and three in an hour read identically in a log and are
    /// entirely different faults. `DeviceNotAvailable` is excluded — that ends the stream and has its
    /// own flag; this is only the ones it carries on through.
    xruns: AtomicU32,
    /// Set by the stream's error callback.
    ///
    /// A stream whose device has died goes on existing and stops draining its command queue, which
    /// without this flag shows up only as "the audio command queue is full". The control thread
    /// watches it and drops the corpse, so the next song opens a live stream instead.
    stream_failed: AtomicBool,
}

impl SharedState {
    /// Current position in ticks, which the display interpolates the lyric highlight from.
    pub fn position_ticks(&self) -> u32 {
        self.position_ticks.load(Ordering::Relaxed)
    }

    /// Current position in milliseconds.
    pub fn position_ms(&self) -> u32 {
        self.position_ms.load(Ordering::Relaxed)
    }

    /// How long the loaded song has been silent for want of samples, in milliseconds.
    ///
    /// Zero on a healthy song, and on every MIDI one. See the field.
    pub fn starved_ms(&self) -> u32 {
        self.starved_ms.load(Ordering::Relaxed)
    }

    /// Recoverable stream errors since the stream opened; the MIDI half of `starved_ms`.
    ///
    /// Cumulative for the *stream* rather than the song, because the callback that counts them has
    /// no idea what is loaded — so a reader wanting "did this song underrun" differences it.
    pub fn xruns(&self) -> u32 {
        self.xruns.load(Ordering::Relaxed)
    }

    /// What the transport is doing.
    pub fn transport(&self) -> Transport {
        transport_from_code(self.transport.load(Ordering::Relaxed))
    }

    /// How many songs have ended since the engine started.
    ///
    /// A counter rather than a flag, so the control thread cannot miss one by polling late.
    pub fn songs_ended(&self) -> u32 {
        self.songs_ended.load(Ordering::Acquire)
    }

    /// The rate of the device last opened, or the probe's answer before the first open; 0 if
    /// nothing is known.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Relaxed)
    }

    /// The channel count, on the same terms as [`SharedState::sample_rate`].
    pub fn channels(&self) -> u16 {
        u16::try_from(self.channels.load(Ordering::Relaxed)).unwrap_or(u16::MAX)
    }

    /// Records that playback has stopped, when no callback is left alive to record it.
    ///
    /// The transport atomic is written from inside the device callback, so a stream dropped while a
    /// song was playing freezes it at `Playing` -- and the position with it. The machine then reports
    /// a song playing forever, at a standstill, which is the worst way to fail in front of a room.
    /// Only the abnormal path calls this; an ordinary stop has a callback to speak for it.
    pub fn publish_stopped(&self) {
        self.transport
            .store(transport_code(Transport::Stopped), Ordering::Relaxed);
    }

    /// How much audio one callback covers, in milliseconds; 0 if no stream has run yet.
    ///
    /// See the field: this is the step size of [`SharedState::position_ms`], and the reason a display
    /// can look like it is running at six frames a second while drawing sixty.
    pub fn period_ms(&self) -> u32 {
        self.period_ms.load(Ordering::Relaxed)
    }

    /// Whether the open stream has reported an error and should be dropped.
    pub fn stream_failed(&self) -> bool {
        self.stream_failed.load(Ordering::Acquire)
    }

    /// Records what a device said about itself, from a probe or from an open.
    pub fn publish_device(&self, sample_rate: u32, channels: u16) {
        self.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.channels.store(u32::from(channels), Ordering::Relaxed);
    }
}

/// What an output device would give us, learned without opening a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The rate the device would negotiate.
    pub sample_rate: u32,
    /// How many channels it would take.
    pub channels: u16,
    /// Which device that was, and whether it is the one that was asked for.
    pub chosen: Chosen,
}

/// A running audio output stream with its player.
pub struct OutputStream {
    // Dropped last, stopping the callback before the queues it uses go away.
    _stream: cpal::Stream,
    commands: rtrb::Producer<Command>,
    retired: rtrb::Consumer<Retired>,
    shared: Arc<SharedState>,
    sample_rate: u32,
    channels: u16,
    chosen: Chosen,
}

/// Commands that can be queued before the audio thread drains them.
const COMMAND_CAPACITY: usize = 64;

/// Retired songs and video feeds that can be waiting to be freed.
const RETIRED_CAPACITY: usize = 8;

/// How long to wait for the device to activate before giving up.
///
/// cpal's default is `None`, which blocks forever. That was tolerable when the only open happened
/// inside the ten-second startup budget; it is not, now that a Bluetooth device can be opened in the
/// middle of an evening and a wedged driver would stall the audio thread for the rest of it.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);

impl OutputStream {
    /// Whether an output device exists, and what it would negotiate — without opening it.
    ///
    /// This is how the machine still knows at startup whether it can play anything at all, now that
    /// it no longer opens the device until there is something to play. It asks for the default
    /// device and its default configuration and stops there: no `IAudioClient::Initialize`, no
    /// `Start`, so on Windows no audio session appears and a Bluetooth link is not brought up.
    ///
    /// The `cpal::Device` is confined to this function on purpose. cpal's WASAPI backend caches an
    /// uninitialized `IAudioClient` inside the device, and keeping one alive for the whole session
    /// is the same shape of mistake the lazy opening exists to undo.
    ///
    /// `want` is `settings.audio.output_device` — `None` when nothing has ever been chosen, which is
    /// the only time [`device::decide`]'s USB preference applies. Because this runs at startup, it is
    /// also where that rule is *settled*: the returned [`Chosen`] is what the caller writes back into
    /// settings, so the guess is made once and kept rather than re-made on every open.
    pub fn probe(want: Option<&str>) -> Result<DeviceInfo, AudioError> {
        let (device, chosen) = device::resolve(want)?;
        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Config(e.to_string()))?;
        Ok(DeviceInfo {
            sample_rate: supported.sample_rate(),
            channels: supported.channels(),
            chosen,
        })
    }

    /// Opens the default output device against an existing [`SharedState`], and starts playing
    /// silence.
    ///
    /// The state object is handed in rather than made here because the device is opened and dropped
    /// many times over one run: the machine holds this `Arc` for its whole life, and a reopen that
    /// made a fresh one would zero `songs_ended` — which the queue's watchdog compares against a
    /// counter of its own, so it would read as a song having just finished and skip the next one.
    ///
    /// `make_source` is called with the device's sample rate, so the synthesizer is built to match
    /// rather than resampled — which is also why a source cannot simply be carried across a reopen:
    /// the new device may run at a different rate.
    ///
    /// `want` is the same setting [`probe`](Self::probe) takes, and is re-resolved on every open
    /// rather than remembered: a device can be unplugged between one song and the next, and the
    /// answer to that is a fresh fallback, not a stale handle.
    pub fn open<F, S>(
        shared: Arc<SharedState>,
        want: Option<&str>,
        make_source: F,
    ) -> Result<Self, AudioError>
    where
        F: FnOnce(u32) -> Result<S, SourceError>,
        S: AudioSource + Send + 'static,
    {
        let (device, chosen) = device::resolve(want)?;
        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Config(e.to_string()))?;

        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let sample_rate = config.sample_rate;
        let channels = config.channels;

        let source = make_source(sample_rate)?;
        let mut player = Player::new(source);

        let (command_producer, mut command_consumer) = rtrb::RingBuffer::new(COMMAND_CAPACITY);
        let (mut retired_producer, retired_consumer) = rtrb::RingBuffer::new(RETIRED_CAPACITY);
        shared.publish_device(sample_rate, channels);
        shared.stream_failed.store(false, Ordering::Release);
        let audio_shared = Arc::clone(&shared);

        let channel_count = usize::from(channels);
        // Reported once per stream rather than per block: it does not change while a stream is open,
        // and a real-time callback is the wrong place to log from sixty times a second.
        let mut measured = false;
        let mut process = move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
            if !measured {
                measured = true;
                let frames = (out.len() / channel_count.max(1)) as u32;
                let stamps = info.timestamp();
                // How long until the *first* sample of this block is actually heard. cpal gets this
                // from the backend, so it is the device's real output latency rather than a guess
                // from the buffer size -- and the two differ, because a running ALSA stream normally
                // has a period already queued ahead of the one being filled.
                // Saturates to zero rather than returning an Option: a backend that cannot answer
                // reports no latency, which is the right conservative default here.
                let latency = stamps.playback.duration_since(stamps.callback);
                let period_ms = frames * 1000 / sample_rate.max(1);
                audio_shared.period_ms.store(period_ms, Ordering::Relaxed);
                tracing::info!(
                    period_frames = frames,
                    period_ms,
                    latency_ms = latency.as_millis() as u64,
                    "the output stream's timing"
                );
            }

            while let Ok(command) = command_consumer.pop() {
                apply(&mut player, command, &mut retired_producer);
            }

            fill_and_publish(&mut player, &audio_shared, out, channel_count);
        };

        // Flagged as well as logged. A stream whose device has gone away keeps existing and stops
        // draining its command queue, which without this surfaces only as "the audio command
        // queue is full" — a symptom, several seconds late, of something the stream already knew.
        // The control thread watches this flag and drops the stream, so the next song opens a live
        // one.
        let error_shared = Arc::clone(&shared);
        let on_error = move |error: cpal::Error| {
            // **Not every reported error is a dead device, and treating them alike broke playback on
            // an entirely ordinary configuration.** cpal's ALSA backend reports an underrun *and then
            // recovers from it itself* -- `ErrorKind::Xrun => { error_callback(err); prepare();
            // start(); }` -- and goes on running. `DeviceNotAvailable` is the one that ends the
            // stream, and its worker returns straight after reporting.
            //
            // Flagging on all of them meant the first xrun killed a perfectly live stream. On the
            // appliance that made the machine unable to play through ALSA's `dmix` at all: dmix
            // underruns once while it primes, cpal shrugged it off, and this dropped the stream about
            // 200 ms in. `dmix` is what a stock `default` on a shared card *is*, so the fault was not
            // exotic. Worse, dropping a stream mid-song left the queue believing it was still
            // playing, with the position frozen -- a silent hang in front of a microphone.
            match error.kind() {
                cpal::ErrorKind::DeviceNotAvailable => {
                    tracing::error!(%error, "the audio device is gone; dropping the stream");
                    error_shared.stream_failed.store(true, Ordering::Release);
                }
                // Logged, and deliberately not fatal. If one of these ever does turn out to leave a
                // stream unusable, the symptom is silence with this line repeating -- which is a much
                // better place to start than a song that stopped for no stated reason.
                kind => {
                    // Counted as well as logged: the log says one happened, the counter says how
                    // often, and only the second distinguishes a stream priming from a synthesizer
                    // that cannot hold its deadline. See the field.
                    error_shared.xruns.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(%error, ?kind, "audio stream error; the stream carries on");
                }
            }
        };

        // The mixer renders `f32`. A device that asks for anything else gets cpal's own conversion
        // of it, so every integer and float format it names opens. The DSD formats carry a
        // bitstream rather than samples and are reported rather than filled with noise.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |data: &mut [f32], info| process(data, info),
                on_error,
                Some(ACTIVATION_TIMEOUT),
            ),
            cpal::SampleFormat::I8 => converting::<i8>(&device, config, process, on_error),
            cpal::SampleFormat::I16 => converting::<i16>(&device, config, process, on_error),
            cpal::SampleFormat::I24 => converting::<cpal::I24>(&device, config, process, on_error),
            cpal::SampleFormat::I32 => converting::<i32>(&device, config, process, on_error),
            cpal::SampleFormat::I64 => converting::<i64>(&device, config, process, on_error),
            cpal::SampleFormat::U8 => converting::<u8>(&device, config, process, on_error),
            cpal::SampleFormat::U16 => converting::<u16>(&device, config, process, on_error),
            cpal::SampleFormat::U24 => converting::<cpal::U24>(&device, config, process, on_error),
            cpal::SampleFormat::U32 => converting::<u32>(&device, config, process, on_error),
            cpal::SampleFormat::U64 => converting::<u64>(&device, config, process, on_error),
            cpal::SampleFormat::F64 => converting::<f64>(&device, config, process, on_error),
            other => return Err(AudioError::SampleFormat(other)),
        }
        .map_err(|e| AudioError::Stream(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioError::Stream(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            commands: command_producer,
            retired: retired_consumer,
            shared,
            sample_rate,
            channels,
            chosen,
        })
    }

    /// Queues a command for the audio thread.
    ///
    /// Returns `false` if the queue is full, which means the audio thread has stalled; the caller
    /// can retry rather than block.
    pub fn send(&mut self, command: Command) -> bool {
        self.commands.push(command).is_ok()
    }

    /// Frees what the audio thread has finished with — parsed songs and video feeds alike.
    ///
    /// Call periodically from the control thread. Skipping it leaks them; doing it on the audio
    /// thread would mean freeing megabytes inside a real-time callback.
    /// It is also the one place a starved video song can be reported from, which is why the
    /// summary lives here rather than in the machine. The retired [`TrackPlayer`] *is* the counter —
    /// it arrives exactly once per song, on this thread, at the moment nothing can add to it any
    /// more. Reading the published `starved_ms` instead would be a race against the next song
    /// resetting it, and reading it from the callback would mean logging on the audio thread.
    ///
    /// **Reported unconditionally, at `warn`, and not behind `--frame-stats`.** A starved feed is
    /// not a diagnostic somebody opted into: the song audibly stopped, and the owner heard it. It
    /// costs nothing on a healthy machine because a healthy song starves for zero milliseconds and
    /// nothing is logged.
    pub fn collect_retired(&mut self) -> usize {
        let mut freed = 0;
        while let Ok(retired) = self.retired.pop() {
            if let Retired::Track(track) = &retired {
                let starved_ms = track.starved_ms();
                if starved_ms > 0 {
                    // **Not "the video song's", which is what this said and was wrong.** A
                    // `TrackPlayer` is whatever plays from a decoder feed, and that is a video song
                    // *or* an MP3+G pair — `km-cdg` builds one from the same `audio_feed` with the
                    // same lookahead. An MP3+G song that ran dry would have been reported as a video
                    // one. Neither kind is named here because this side cannot tell them apart, and
                    // guessing would put the wrong word in a fault report.
                    tracing::warn!(
                        starved_ms,
                        "the song's audio ran dry — the decoder could not keep up"
                    );
                }
            }
            drop(retired);
            freed += 1;
        }
        freed
    }

    /// The state the audio thread publishes.
    pub fn state(&self) -> &Arc<SharedState> {
        &self.shared
    }

    /// The device's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The device's channel count.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Which device this stream actually opened.
    ///
    /// Not the same as what settings asked for: a saved device that has been unplugged since the
    /// last song leaves this reporting the system default with `fell_back` set.
    pub fn chosen(&self) -> &Chosen {
        &self.chosen
    }
}

/// Applies one command on the audio thread.
/// Renders one block and publishes what it did.
///
/// **The two things that can drive a player share this**, so a stream feeding a sound card and one
/// feeding an encoder cannot come to disagree about what a block means. Everything a reader outside
/// sees — the position the screen draws from, the transport the queue watches, the count of songs
/// that ended — is written here and nowhere else.
fn fill_and_publish<S: AudioSource>(
    player: &mut Player<S>,
    shared: &SharedState,
    out: &mut [f32],
    channels: usize,
) {
    let event = player.fill(out, channels);
    // Every format, once, here -- rather than in the two branches that happen to scale and the one
    // that does not. See [`limit`] for what a bank can otherwise send to a PA.
    limit(out);

    shared
        .position_ticks
        .store(player.position_ticks(), Ordering::Relaxed);
    shared
        .position_ms
        .store(player.position_ms(), Ordering::Relaxed);
    shared
        .transport
        .store(transport_code(player.transport()), Ordering::Relaxed);
    shared.starved_ms.store(
        u32::try_from(player.starved_ms()).unwrap_or(u32::MAX),
        Ordering::Relaxed,
    );
    if event == Some(PlayerEvent::SongEnded) {
        shared.songs_ended.fetch_add(1, Ordering::AcqRel);
    }
}

/// A player with no device, rendered by whoever wants the samples.
///
/// **The counterpart to [`OutputStream`], for a machine whose sound goes into an encoder rather than
/// out of a speaker.** A sound card pulls blocks on its own schedule and paces everything behind it;
/// here the caller pulls them, which is what lets one video frame be drawn for every fixed number of
/// samples and the two arrive in step by arithmetic.
///
/// **Nothing here is real-time**, which is the other half of the difference. There is no callback
/// deadline to miss, so commands arrive on an ordinary channel and a retired song is dropped where
/// it is found rather than handed to another thread to free.
pub struct Renderer<S: AudioSource> {
    player: Player<S>,
    shared: Arc<SharedState>,
    channels: usize,
    /// Where [`apply`] puts a displaced song, drained straight after.
    ///
    /// A ring with both ends held here, because `apply` is shared with the real-time path and takes
    /// one. What it buys there — a drop that happens off the callback — is not needed here, so it
    /// is emptied in place.
    retired_producer: rtrb::Producer<Retired>,
    retired_consumer: rtrb::Consumer<Retired>,
}

impl<S: AudioSource> Renderer<S> {
    /// A renderer over `source`, publishing into `shared`.
    pub fn new(source: S, shared: Arc<SharedState>, sample_rate: u32, channels: usize) -> Self {
        shared.publish_device(sample_rate, u16::try_from(channels).unwrap_or(2));
        shared.stream_failed.store(false, Ordering::Release);
        let (retired_producer, retired_consumer) = rtrb::RingBuffer::new(RETIRED_CAPACITY);
        Self {
            player: Player::new(source),
            shared,
            channels,
            retired_producer,
            retired_consumer,
        }
    }

    /// Does what one command says.
    pub fn apply(&mut self, command: Command) {
        apply(&mut self.player, command, &mut self.retired_producer);
        // Emptied here rather than left for a housekeeping pass: nothing about this path is
        // real-time, so the thread that displaced a song is the right one to free it.
        while self.retired_consumer.pop().is_ok() {}
    }

    /// Fills `out` with interleaved samples and publishes what the block did.
    pub fn render(&mut self, out: &mut [f32]) {
        fill_and_publish(&mut self.player, &self.shared, out, self.channels);
    }

    /// The rate this was built for.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.shared.sample_rate()
    }

    /// How many channels a block carries.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The state this publishes into, for a caller building a second renderer over it.
    #[must_use]
    pub fn state(&self) -> &Arc<SharedState> {
        &self.shared
    }

    /// Whether anything is loaded.
    ///
    /// What a caller asks before replacing the source: a bank swapped under a playing song would
    /// take the song with it.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.player.song().is_none() && !self.player.is_track()
    }

    /// Plays through a different source from now on, keeping the settings the old one had.
    ///
    /// The song is not carried across, because a source is what renders it: a bank swap is a new
    /// player, exactly as it is on the device path where the stream is dropped and rebuilt.
    pub fn replace_source(&mut self, source: S) {
        let volume = self.player.music_volume();
        let settings = self.player.settings();
        self.player = Player::new(source);
        self.player.set_music_volume(volume);
        self.player.set_transpose(settings.transpose);
        self.player.set_tempo_ratio(settings.tempo_ratio);
        self.player.set_melody_enabled(settings.melody_enabled);
    }
}

fn apply<S: AudioSource>(
    player: &mut Player<S>,
    command: Command,
    retired: &mut rtrb::Producer<Retired>,
) {
    match command {
        Command::Load(load) => {
            let previous = player.retire();
            match load {
                Load::Midi {
                    song,
                    melody_channel,
                    fixes,
                } => player.load(song, melody_channel, fixes),
                Load::Track(track) => player.load_track(*track),
            }
            if let Some(previous) = previous {
                // Hand the old song back rather than dropping it here. If the queue is full the
                // drop happens on this thread, which is a glitch but never a leak.
                let _ = retired.push(previous);
            }
        }
        Command::Unload => {
            let previous = player.retire();
            player.unload();
            if let Some(previous) = previous {
                let _ = retired.push(previous);
            }
        }
        Command::Play => player.play(),
        Command::Pause => player.pause(),
        Command::Stop => player.stop(),
        Command::Restart => player.restart(),
        Command::SeekMs(ms) => player.seek_ms(ms),
        Command::SetTranspose(semitones) => player.set_transpose(semitones),
        Command::SetTempoRatio(ratio) => player.set_tempo_ratio(ratio),
        Command::SetMelodyEnabled(enabled) => player.set_melody_enabled(enabled),
        Command::SetMusicVolume(volume) => player.set_music_volume(volume),
        Command::SetSongGain(gain) => player.set_song_gain(gain),
        // Nothing to do here by design: `Wake` is addressed to the thread that owns the device, and
        // its whole effect happened before it reached the queue.
        Command::Wake => {}
    }
}

/// Frames of `f32` a converting stream holds before its first callback.
///
/// Larger than any block a desktop or phone backend hands over, so the scratch buffer is allocated
/// here on the control thread and not in the callback. A backend that asks for more still plays: the
/// buffer grows once on the audio thread and keeps that size.
const SCRATCH_FRAMES: usize = 16_384;

/// Opens a stream in a sample format other than `f32`, rendering into a scratch buffer and
/// converting each sample with cpal's own `FromSample`.
///
/// The clamp stops just short of `+1.0`. cpal's conversion assumes `-1.0 <= s < 1.0`, and a 24-bit
/// sample at exactly `+1.0` wraps to full negative scale instead of saturating.
fn converting<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut process: impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut scratch: Vec<f32> =
        Vec::with_capacity(SCRATCH_FRAMES * usize::from(config.channels.max(1)));
    device.build_output_stream(
        config,
        move |data: &mut [T], info| {
            scratch.resize(data.len(), 0.0);
            process(&mut scratch, info);
            for (out, sample) in data.iter_mut().zip(scratch.iter()) {
                *out = T::from_sample(sample.clamp(-1.0, 1.0 - f32::EPSILON));
            }
        },
        on_error,
        Some(ACTIVATION_TIMEOUT),
    )
}

/// Brings a block of samples into range before it is handed to the driver.
///
/// **This is the only thing standing between a bad SoundFont and the loudest noise the hardware can
/// make**, and until the second soundfont survey nothing did it on the path almost everybody takes.
/// The branches of [`OutputStream::open`] that convert to another sample format clamp on their way out because they have to
/// scale anyway; `F32` passed the buffer to cpal exactly as the player left it — and `F32` is the
/// native format on Windows, and on most current ALSA and CoreAudio configurations.
///
/// The survey is what showed this was reachable rather than theoretical. `Roland_SC-55.sf2` loads
/// cleanly, plays five of the note's seven songs at an ordinary level, and on the other two produces
/// a peak of **2.2e19** — not merely over full scale but diverging. A machine wired into a PA plays
/// that at whatever the amplifier will do.
///
/// **`clamp` alone is not enough**, which is the part worth keeping: `f32::clamp` returns NaN for a
/// NaN input, so a divergence that reaches the other end of the same failure would pass straight
/// through it. A sample that is not finite becomes silence, because silence is the only safe reading
/// of a number that is not a number.
fn limit(out: &mut [f32]) {
    for sample in out.iter_mut() {
        *sample = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_leaves_the_callback_outside_full_scale() {
        let mut block = [0.5, -0.5, 1.5, -1.5, 2.2e19, -2.2e19];
        limit(&mut block);
        assert_eq!(block, [0.5, -0.5, 1.0, -1.0, 1.0, -1.0]);
    }

    /// A NaN is the case `clamp` on its own gets wrong: it returns the NaN.
    #[test]
    fn a_sample_that_is_not_a_number_becomes_silence() {
        let mut block = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.25];
        limit(&mut block);
        assert_eq!(block, [0.0, 0.0, 0.0, 0.25]);
    }

    #[test]
    fn a_stream_that_goes_away_mid_song_does_not_leave_it_reading_as_playing() {
        let shared = SharedState::default();
        // What the callback leaves behind when the device disappears under it.
        shared
            .transport
            .store(transport_code(Transport::Playing), Ordering::Relaxed);
        shared.position_ms.store(42_000, Ordering::Relaxed);
        assert_eq!(shared.transport(), Transport::Playing);

        shared.publish_stopped();

        // The position is deliberately left where it was -- it is the last true thing known about the
        // song, and inventing a new one would be no better than the frozen `Playing` this replaces.
        assert_eq!(shared.transport(), Transport::Stopped);
        assert_eq!(shared.position_ms(), 42_000);
    }

    #[test]
    fn transport_codes_round_trip() {
        for transport in [
            Transport::Idle,
            Transport::Playing,
            Transport::Paused,
            Transport::Stopped,
        ] {
            assert_eq!(transport_from_code(transport_code(transport)), transport);
        }
    }

    #[test]
    fn an_unknown_transport_code_reads_as_idle() {
        assert_eq!(transport_from_code(99), Transport::Idle);
    }

    #[test]
    fn shared_state_starts_empty() {
        let state = SharedState::default();
        assert_eq!(state.position_ticks(), 0);
        assert_eq!(state.position_ms(), 0);
        assert_eq!(state.transport(), Transport::Idle);
        assert_eq!(state.songs_ended(), 0);
        // Zero rather than a guess: nothing has been opened, so no rate is known yet.
        assert_eq!(state.sample_rate(), 0);
        assert_eq!(state.channels(), 0);
        assert!(!state.stream_failed());
    }

    #[test]
    fn a_device_can_be_published_more_than_once() {
        let state = SharedState::default();
        state.publish_device(44_100, 2);
        assert_eq!(state.sample_rate(), 44_100);
        assert_eq!(state.channels(), 2);
        // Reopening onto a different default device is an ordinary event now, not a restart.
        state.publish_device(48_000, 6);
        assert_eq!(state.sample_rate(), 48_000);
        assert_eq!(state.channels(), 6);
    }

    #[test]
    fn publishing_a_device_leaves_the_song_counter_alone() {
        // The reopen invariant, as a test. `songs_ended` is what the queue's watchdog compares
        // against a count of its own; a reopen that disturbed it would skip a song.
        let state = SharedState::default();
        state.songs_ended.fetch_add(3, Ordering::AcqRel);
        state.publish_device(48_000, 2);
        assert_eq!(state.songs_ended(), 3);
    }

    // Opening a real device is not tested: CI has no audio hardware. The behavior that matters --
    // sequencing, transposition, muting, seeking, buffer filling -- is covered by the sequencer,
    // player and offline tests, which need neither a device nor a SoundFont. `probe` is untested
    // here for the same reason, and the policy deciding *when* to open and release lives in
    // `km-app`'s audio thread as a pure function, where every branch of it is.
}
