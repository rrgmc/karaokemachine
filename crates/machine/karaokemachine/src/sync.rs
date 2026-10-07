//! The lyric sync editor: a song plays, a person taps each syllable as it is sung, and a `.kar`
//! file is written with the words at those ticks.
//!
//! It is a loop of its own and builds no [`crate::machine::Machine`], no API and no catalog. It
//! lives in this crate for what the machine already has: the bank the owner chose, the audio device,
//! the fonts, the position smoothing and the screen the result is checked on.
//!
//! **The source file is read and never written.** The words go into a new file, through
//! [`km_song::kar_write`].
//!
//! [`Session`] is the editing state and holds no window and no sound, so it is what the tests
//! drive.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use km_audio::audio::{Command, Load, SharedState};
use km_audio::{Bank, OutputStream, SoundFontSource, TestToneSource};
use km_display::draw::{Background, Frame, Screen, SongInfo, draw};
use km_display::lyrics::{LyricView, shift_ticks};
use km_display::numbers::NumberEntry;
use km_display::text::{Align, Fonts, TextCache, TextStyle, draw_text, measure_line};
use km_display::theme::Theme;
use km_queue::Transport;
use km_song::kar_write::{KarWords, split_words, write_soft_karaoke};
use km_song::timeline::{LineBreak, RawSyllable};
use km_song::{ParseOptions, Song};
use sdl3::event::Event as SdlEvent;
use sdl3::keyboard::{Keycode, Mod};
use sdl3::pixels::Color;
use sdl3::render::{BlendMode, Canvas, FRect};
use sdl3::video::{Window, WindowContext};

use crate::display::StepSmoother;
use crate::settings::{Paths, Settings};

/// The window's size. The editor is a desk tool and is never the fullscreen show.
const WINDOW: (u32, u32) = (1280, 720);

/// How far a snap may move a syllable to reach a note.
const SNAP_WINDOW_MS: u32 = 120;

/// One press of an arrow key in review, and the same with Shift held.
const NUDGE_MS: i32 = 10;
const NUDGE_COARSE_MS: i32 = 50;

/// How far the seek keys move while tapping.
const SEEK_MS: u32 = 5_000;

/// How much music plays before the words when a line is replayed, so the person hears it coming.
const RUN_UP_MS: u32 = 2_000;

/// How long a message stays up.
const MESSAGE_FOR: Duration = Duration::from_secs(4);

/// How long the melody mark stays lit after one of the chosen channel's notes starts.
const NOTE_MARK_MS: u32 = 120;

/// What the command line asked the editor for.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    /// The MIDI file to play. Read, and never written.
    pub song: PathBuf,
    /// The words as typed text.
    pub words: PathBuf,
    /// Where the `.kar` goes. `None` puts it beside the song.
    pub out: Option<PathBuf>,
    /// The title to write, where the song's own is not wanted.
    pub title: Option<String>,
    /// The artist to write.
    pub artist: Option<String>,
    /// The language to write, in the four-letter form (`ENGL`).
    pub language: Option<String>,
    /// The channel to snap to, numbered from 1 as a person counts them.
    pub melody_channel: Option<u8>,
    /// The tap offset in milliseconds, where the room's lyric offset is not wanted.
    pub tap_offset_ms: Option<i16>,
    /// Whether an existing output file is replaced.
    pub force: bool,
}

/// Which of the editor's two screens is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Syllables are still waiting for a tick.
    Tapping,
    /// Every syllable has a tick, and the song plays with the words as they will be written.
    Review,
}

/// The words and the ticks given to them so far.
#[derive(Debug, Clone)]
struct Session {
    /// Every syllable, in singing order. The first `stamped` of them hold a real tick.
    syllables: Vec<RawSyllable>,
    /// Which syllables make each line.
    lines: Vec<Range<usize>>,
    /// How many syllables have been tapped. They are always the first ones.
    stamped: usize,
    /// The syllable the review keys act on.
    selected: usize,
    /// Whether there is a tick the output file does not hold.
    dirty: bool,
    /// The ticks as they stood before the last snap, for taking it back.
    before_snap: Option<Vec<u32>>,
}

impl Session {
    fn new(syllables: Vec<RawSyllable>) -> Self {
        let mut lines: Vec<Range<usize>> = Vec::new();
        for (index, syllable) in syllables.iter().enumerate() {
            match lines.last_mut() {
                Some(line) if syllable.break_before == LineBreak::None => line.end = index + 1,
                _ => lines.push(index..index + 1),
            }
        }
        Self {
            syllables,
            lines,
            stamped: 0,
            selected: 0,
            dirty: false,
            before_snap: None,
        }
    }

    fn phase(&self) -> Phase {
        if self.stamped == self.syllables.len() {
            Phase::Review
        } else {
            Phase::Tapping
        }
    }

    /// The index of the line holding this syllable.
    fn line_of(&self, syllable: usize) -> usize {
        self.lines
            .iter()
            .position(|line| line.contains(&syllable))
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// The earliest tick this syllable may hold: where the one before it starts, or ends.
    fn floor_of(&self, index: usize) -> u32 {
        index.checked_sub(1).map_or(0, |i| {
            let before = &self.syllables[i];
            before
                .end_tick
                .map_or(before.tick, |end| end.max(before.tick))
        })
    }

    /// The latest tick this syllable may hold: its own end, or where the next one starts.
    fn ceiling_of(&self, index: usize) -> u32 {
        let next = if index + 1 < self.stamped {
            self.syllables[index + 1].tick
        } else {
            u32::MAX
        };
        self.syllables[index]
            .end_tick
            .map_or(next, |end| end.min(next))
    }

    /// Gives the next syllable this tick. A tap never lands before the one in front of it.
    fn tap(&mut self, tick: u32) {
        let floor = self.floor_of(self.stamped);
        let Some(next) = self.syllables.get_mut(self.stamped) else {
            return;
        };
        next.tick = tick.max(floor);
        next.end_tick = None;
        self.stamped += 1;
        self.selected = self.stamped - 1;
        self.dirty = true;
        self.before_snap = None;
    }

    /// Says where a tapped syllable stops being sung, so its highlight does not run on through the
    /// pause after it. The end stays after the syllable's start and before the next syllable.
    fn end_at(&mut self, index: usize, tick: u32) -> bool {
        if index >= self.stamped || tick <= self.syllables[index].tick {
            return false;
        }
        let ceiling = if index + 1 < self.stamped {
            self.syllables[index + 1].tick
        } else {
            u32::MAX
        };
        if ceiling <= self.syllables[index].tick {
            return false;
        }
        self.syllables[index].end_tick = Some(tick.min(ceiling));
        self.dirty = true;
        true
    }

    /// Takes back the last thing tapped: the last syllable's end when it has one, or its tap.
    fn undo(&mut self) -> bool {
        let Some(last) = self.stamped.checked_sub(1) else {
            return false;
        };
        if self.syllables[last].end_tick.take().is_none() {
            self.stamped = last;
        }
        self.dirty = true;
        self.before_snap = None;
        true
    }

    /// Takes back every tap in the line being tapped, or in the one before it when this line has
    /// none. Answers the tick the line before that ended on, which is where to play from.
    fn retap_line(&mut self) -> u32 {
        let at = self.stamped.min(self.syllables.len().saturating_sub(1));
        let mut line = self.line_of(at);
        if self.lines[line].start == self.stamped && line > 0 {
            line -= 1;
        }
        let start = self.lines[line].start;
        if start < self.stamped {
            for syllable in &mut self.syllables[start..self.stamped] {
                syllable.end_tick = None;
            }
            self.stamped = start;
            self.dirty = true;
            self.before_snap = None;
        }
        start.checked_sub(1).map_or(0, |i| self.syllables[i].tick)
    }

    /// Moves the selection by one syllable, within the tapped ones.
    fn select(&mut self, forward: bool) {
        let last = self.stamped.saturating_sub(1);
        self.selected = if forward {
            (self.selected + 1).min(last)
        } else {
            self.selected.saturating_sub(1).min(last)
        };
    }

    /// Puts the selected syllable at `tick`, held between its two neighbours.
    fn move_selected(&mut self, tick: u32) {
        if self.selected >= self.stamped {
            return;
        }
        let floor = self.floor_of(self.selected);
        let ceiling = self.ceiling_of(self.selected).max(floor);
        let tick = tick.clamp(floor, ceiling);
        if self.syllables[self.selected].tick != tick {
            self.syllables[self.selected].tick = tick;
            self.dirty = true;
            self.before_snap = None;
        }
    }

    /// Moves each tapped syllable to the nearest of `onsets` that `near` accepts.
    ///
    /// A syllable never passes the one before it or the one after it, so the words stay in order.
    /// Answers how many moved.
    fn snap(&mut self, onsets: &[u32], near: impl Fn(u32, u32) -> bool) -> usize {
        let before: Vec<u32> = self.syllables[..self.stamped]
            .iter()
            .map(|s| s.tick)
            .collect();
        let mut moved = 0;
        for (index, &tick) in before.iter().enumerate() {
            let Some(onset) = nearest(onsets, tick) else {
                break;
            };
            // The next syllable has not moved yet, so its tick here is the one it was tapped at.
            let floor = self.floor_of(index);
            let ceiling = self.ceiling_of(index);
            if onset != tick && near(tick, onset) && (floor..=ceiling).contains(&onset) {
                self.syllables[index].tick = onset;
                moved += 1;
            }
        }
        if moved > 0 {
            self.before_snap = Some(before);
            self.dirty = true;
        }
        moved
    }

    /// Takes back the last snap.
    fn undo_snap(&mut self) -> bool {
        let Some(before) = self.before_snap.take() else {
            return false;
        };
        for (syllable, tick) in self.syllables.iter_mut().zip(before) {
            syllable.tick = tick;
        }
        self.dirty = true;
        true
    }

    /// The tapped syllables, which are what a save writes.
    fn tapped(&self) -> Vec<RawSyllable> {
        self.syllables[..self.stamped].to_vec()
    }
}

/// The value in a sorted list nearest to `to`.
fn nearest(sorted: &[u32], to: u32) -> Option<u32> {
    let after = sorted.partition_point(|&value| value < to);
    let above = sorted.get(after).copied();
    let below = after.checked_sub(1).and_then(|i| sorted.get(i).copied());
    match (below, above) {
        (Some(below), Some(above)) if to - below <= above - to => Some(below),
        (_, Some(above)) => Some(above),
        (below, None) => below,
    }
}

/// Where the output goes when nobody said: beside the song, with the karaoke extension.
fn default_out(song: &Path) -> PathBuf {
    let beside = song.with_extension("kar");
    if beside != song {
        return beside;
    }
    let stem = song.file_stem().unwrap_or_default().to_string_lossy();
    song.with_file_name(format!("{stem}-synced.kar"))
}

/// A channel the words can be snapped to.
struct MelodyChoice {
    /// The MIDI channel, numbered from 0.
    channel: u8,
    /// The track name the file gives it, where it gives one.
    name: Option<String>,
    /// Where each of its notes starts, in order.
    onsets: Vec<u32>,
}

impl MelodyChoice {
    fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("channel {} ({name})", self.channel + 1),
            None => format!("channel {}", self.channel + 1),
        }
    }
}

/// Every channel that plays notes and is not the drums, likeliest melody first.
fn melody_choices(song: &Song) -> Vec<MelodyChoice> {
    let thresholds = km_suitability::Thresholds::default();
    let stats = km_suitability::channel::measure(song, &thresholds);
    let ranked = km_suitability::melody::rank(song, &stats, &thresholds);
    ranked
        .iter()
        .filter_map(|evidence| stats.iter().find(|s| s.channel == evidence.channel))
        .filter(|s| s.note_count > 0 && s.channel != km_suitability::DRUM_CHANNEL)
        .map(|s| MelodyChoice {
            channel: s.channel,
            name: s.track_names.first().cloned(),
            onsets: s.onset_ticks.clone(),
        })
        .collect()
}

/// Opens the sound: the owner's bank, or the test tone when there is none to load.
fn open_audio(paths: &Paths, settings: &Settings) -> anyhow::Result<OutputStream> {
    let selected = crate::soundfont::resolve(paths, settings.audio.soundfont.as_deref());
    let bank = crate::engine::resolve_soundfont(selected.path.as_deref(), paths)
        .and_then(|path| Bank::load(&path).map_err(|error| error.to_string()));
    let shared = Arc::new(SharedState::default());
    let want = settings.audio.output_device.as_deref();
    let stream = match &bank {
        Ok(bank) => OutputStream::open(shared, want, |rate| SoundFontSource::from_bank(bank, rate)),
        Err(reason) => {
            tracing::warn!(%reason, "no SoundFont, so the song plays as a test tone");
            OutputStream::open(shared, want, |rate| Ok(TestToneSource::new(rate)))
        }
    };
    stream.context("no audio output")
}

/// Runs the editor until its window closes.
pub(crate) fn run(paths: &Paths, settings: &Settings, request: &Request) -> anyhow::Result<()> {
    let source = std::fs::read(&request.song)
        .with_context(|| format!("reading {}", request.song.display()))?;
    let song = Arc::new(
        Song::parse(&source, &ParseOptions::default())
            .with_context(|| format!("{} is not a MIDI file", request.song.display()))?,
    );
    let typed = std::fs::read_to_string(&request.words)
        .with_context(|| format!("reading {}", request.words.display()))?;
    let syllables = split_words(&typed);
    anyhow::ensure!(
        !syllables.is_empty(),
        "{} holds no words",
        request.words.display()
    );

    let out = request
        .out
        .clone()
        .unwrap_or_else(|| default_out(&request.song));
    anyhow::ensure!(
        out != request.song,
        "the output is the song itself, which is never written"
    );
    anyhow::ensure!(
        request.force || !out.exists(),
        "{} exists; pass --sync-force to replace it",
        out.display()
    );

    let stem = request
        .song
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let title = request
        .title
        .clone()
        .or_else(|| song.meta.title.clone())
        .unwrap_or(stem);
    let artist = request
        .artist
        .clone()
        .or_else(|| song.meta.artist.clone())
        .unwrap_or_default();
    let language = request
        .language
        .clone()
        .or_else(|| song.meta.language.clone())
        .unwrap_or_default();
    let words_for = |session: &Session| KarWords {
        title: title.clone(),
        artist: artist.clone(),
        language: language.clone(),
        syllables: session.tapped(),
    };

    let choices = melody_choices(&song);
    let mut choice = request
        .melody_channel
        .and_then(|wanted| choices.iter().position(|c| c.channel + 1 == wanted))
        .unwrap_or(0);
    let offset_ms = request
        .tap_offset_ms
        .unwrap_or_else(|| settings.display.lyric_offset());

    let mut stream = open_audio(paths, settings)?;
    let state = Arc::clone(stream.state());
    stream.send(Command::SetMusicVolume(settings.audio.music_volume));
    stream.send(Command::Load(Load::Midi {
        song: Arc::clone(&song),
        melody_channel: None,
        fixes: km_fixes::ChannelFixes::default(),
    }));

    let sdl = sdl3::init().map_err(|error| anyhow::anyhow!("SDL would not start: {error}"))?;
    let video = sdl
        .video()
        .map_err(|error| anyhow::anyhow!("no video: {error}"))?;
    let ttf =
        sdl3::ttf::init().map_err(|error| anyhow::anyhow!("SDL_ttf would not start: {error}"))?;
    let mut window = video
        .window(&format!("Lyric sync - {title}"), WINDOW.0, WINDOW.1)
        .position_centered()
        .high_pixel_density()
        .build()?;
    km_display::set_window_icon(&mut window);
    let mut canvas = window.into_canvas();
    crate::display::request_vsync(&canvas);
    let mut cache = TextCache::new(canvas.texture_creator());
    let (width, height) = canvas.output_size().unwrap_or(WINDOW);
    let theme = Theme::default();
    let bundled = paths.asset(crate::settings::FONT_SUBPATH);
    let fonts = Fonts::discover(
        &ttf,
        settings.display.font.as_deref(),
        Some(&bundled),
        settings.display.font_cjk.as_deref(),
        false,
        &theme,
        height,
    )
    .map_err(|error| {
        anyhow::anyhow!("no usable font: {error}. Set display.font in settings.json")
    })?;
    let mut events = sdl
        .event_pump()
        .map_err(|error| anyhow::anyhow!("no event pump: {error}"))?;

    let view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    let info = SongInfo {
        number: None,
        title: title.clone(),
        artist: (!artist.is_empty()).then(|| artist.clone()),
        language: None,
    };
    let entry = NumberEntry::new();
    let screen = Layout {
        width: width as f32,
        height: height as f32,
    };

    let mut session = Session::new(syllables);
    // The words as the output file will hold them, parsed back, so review draws what a machine
    // will draw from the file.
    let mut preview: Option<Song> = None;
    let mut preview_stale = true;
    let mut smoother = StepSmoother::new();
    let mut tempo_ratio = 1.0f32;
    let mut want_playing = false;
    let mut ended_seen = state.songs_ended();
    let mut leaving = false;
    let mut show_help = true;
    let mut message: Option<(String, Instant)> = Some((
        "Press Enter to start the song, then Space on each syllable".to_owned(),
        Instant::now(),
    ));

    'frames: loop {
        let now = Instant::now();
        let playing = state.transport() == Transport::Playing;
        let reported = smoother.smooth(state.position_ticks(), state.period_ms(), playing, now);
        // The tick a person is hearing, which is behind the tick the engine has rendered by what
        // the room's lyric offset measures.
        let tick = shift_ticks(&song.tempo_map, reported, offset_ms, tempo_ratio);
        let phase = session.phase();

        let mut say = |text: String| message = Some((text, now));
        let seek_to_ms = |stream: &mut OutputStream, ms: u32| {
            stream.send(Command::SeekMs(ms));
        };

        for event in events.poll_iter() {
            let (key, keymod, repeat) = match event {
                SdlEvent::Quit { .. } => {
                    if session.dirty && !leaving {
                        leaving = true;
                        say("Not saved. Ctrl+S saves, closing again leaves".to_owned());
                        continue;
                    }
                    break 'frames;
                }
                SdlEvent::KeyDown {
                    keycode: Some(key),
                    keymod,
                    repeat,
                    ..
                } => (key, keymod, repeat),
                _ => continue,
            };
            let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
            let shift = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
            if key != Keycode::Escape {
                leaving = false;
            }

            match (phase, key) {
                (_, Keycode::Escape) => {
                    if session.dirty && !leaving {
                        leaving = true;
                        say("Not saved. Ctrl+S saves, Esc again leaves".to_owned());
                    } else {
                        break 'frames;
                    }
                }
                (_, Keycode::S) if ctrl => {
                    let saved = write_soft_karaoke(&source, &words_for(&session))
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| Ok(std::fs::write(&out, bytes)?));
                    match saved {
                        Ok(()) => {
                            session.dirty = false;
                            say(format!(
                                "Saved {} of {} syllables to {}",
                                session.stamped,
                                session.syllables.len(),
                                out.display()
                            ));
                        }
                        Err(error) => say(format!("Not saved: {error}")),
                    }
                }
                (_, Keycode::Minus | Keycode::KpMinus) => {
                    tempo_ratio = (tempo_ratio - 0.05).max(0.5);
                    stream.send(Command::SetTempoRatio(tempo_ratio));
                }
                (_, Keycode::Equals | Keycode::Plus | Keycode::KpPlus) => {
                    tempo_ratio = (tempo_ratio + 0.05).min(1.5);
                    stream.send(Command::SetTempoRatio(tempo_ratio));
                }
                (_, Keycode::H) if !repeat => show_help = !show_help,
                (_, Keycode::M) if !choices.is_empty() => {
                    choice = (choice + 1) % choices.len();
                    say(format!(
                        "N will snap words to {}. The sound does not change",
                        choices[choice].label()
                    ));
                }

                (Phase::Tapping, Keycode::Space) if !repeat => {
                    if playing {
                        session.tap(tick);
                        preview_stale = true;
                        if session.phase() == Phase::Review {
                            say("Every syllable is tapped. Ctrl+S saves".to_owned());
                        }
                    } else {
                        say("The song is paused. Enter plays it".to_owned());
                    }
                }
                (Phase::Tapping, Keycode::Return | Keycode::KpEnter) => {
                    want_playing = !playing;
                    stream.send(if playing {
                        Command::Pause
                    } else {
                        Command::Play
                    });
                }
                (_, Keycode::E) if !repeat => {
                    // While tapping it is the word just tapped; in review, the selected one.
                    let index = match phase {
                        Phase::Tapping => session.stamped.saturating_sub(1),
                        Phase::Review => session.selected,
                    };
                    if playing && session.end_at(index, tick) {
                        preview_stale = true;
                        say(format!(
                            "\"{}\" ends here",
                            session.syllables[index].text.trim()
                        ));
                    } else {
                        say("E ends a word after it starts, while the song plays".to_owned());
                    }
                }
                (Phase::Tapping, Keycode::Backspace) => {
                    preview_stale |= session.undo();
                }
                (Phase::Tapping, Keycode::Up) => {
                    let from = session.retap_line();
                    preview_stale = true;
                    let ms = song.tempo_map.tick_to_ms(from).saturating_sub(RUN_UP_MS);
                    seek_to_ms(&mut stream, ms);
                }
                (Phase::Tapping, Keycode::Left) => {
                    seek_to_ms(&mut stream, state.position_ms().saturating_sub(SEEK_MS));
                }
                (Phase::Tapping, Keycode::Right) => {
                    seek_to_ms(&mut stream, state.position_ms().saturating_add(SEEK_MS));
                }

                (Phase::Review, Keycode::Space) if !repeat => {
                    want_playing = !playing;
                    stream.send(if playing {
                        Command::Pause
                    } else {
                        Command::Play
                    });
                }
                (Phase::Review, Keycode::Up) => session.select(false),
                (Phase::Review, Keycode::Down) => session.select(true),
                (Phase::Review, Keycode::Left | Keycode::Right) => {
                    let step = if shift { NUDGE_COARSE_MS } else { NUDGE_MS };
                    let step = if key == Keycode::Left { -step } else { step };
                    let at = song
                        .tempo_map
                        .tick_to_ms(session.syllables[session.selected].tick);
                    let to = at.saturating_add_signed(step);
                    session.move_selected(song.tempo_map.ms_to_tick(to));
                    preview_stale = true;
                }
                (Phase::Review, Keycode::Return | Keycode::KpEnter) => {
                    let line = &session.lines[session.line_of(session.selected)];
                    let from = session.syllables[line.start].tick;
                    let ms = song.tempo_map.tick_to_ms(from).saturating_sub(RUN_UP_MS);
                    seek_to_ms(&mut stream, ms);
                    stream.send(Command::Play);
                    want_playing = true;
                }
                (Phase::Review, Keycode::N) => match choices.get(choice) {
                    Some(melody) => {
                        let map = &song.tempo_map;
                        let moved = session.snap(&melody.onsets, |a, b| {
                            map.tick_to_ms(a).abs_diff(map.tick_to_ms(b)) <= SNAP_WINDOW_MS
                        });
                        preview_stale = true;
                        say(format!(
                            "{moved} syllables moved onto notes of {}. Ctrl+Z takes it back",
                            melody.label()
                        ));
                    }
                    None => say("No channel plays notes to snap to".to_owned()),
                },
                (Phase::Review, Keycode::Z) if ctrl => {
                    if session.undo_snap() {
                        preview_stale = true;
                        say("The snap is taken back".to_owned());
                    }
                }
                (Phase::Review, Keycode::Backspace) => {
                    preview_stale |= session.undo();
                }
                _ => {}
            }
        }

        // A song that ran out. Review goes round again, and tapping waits for the person.
        let ended = state.songs_ended();
        if ended != ended_seen {
            ended_seen = ended;
            if session.phase() == Phase::Review && want_playing {
                stream.send(Command::Restart);
                stream.send(Command::Play);
            } else {
                want_playing = false;
            }
        }
        stream.collect_retired();

        if preview_stale && session.phase() == Phase::Review {
            preview = write_soft_karaoke(&source, &words_for(&session))
                .ok()
                .and_then(|bytes| Song::parse(&bytes, &ParseOptions::default()).ok());
            preview_stale = false;
        }

        cache.begin_frame();
        let melody = choices.get(choice);
        let note_lit = melody.is_some_and(|m| {
            let since = song
                .tempo_map
                .ms_to_tick(song.tempo_map.tick_to_ms(tick).saturating_sub(NOTE_MARK_MS));
            let first = m.onsets.partition_point(|&onset| onset < since);
            m.onsets.get(first).is_some_and(|&onset| onset <= tick)
        });
        let status = format!(
            "{}   {} / {}   tempo {:.0}%   {} of {} tapped{}",
            if playing { "Playing" } else { "Paused" },
            clock(song.tempo_map.tick_to_ms(tick)),
            clock(song.duration_ms()),
            tempo_ratio * 100.0,
            session.stamped,
            session.syllables.len(),
            if session.dirty { "   not saved" } else { "" },
        );

        match (session.phase(), &preview) {
            (Phase::Review, Some(shown)) => {
                let frame = Frame {
                    screen: Screen::Playing,
                    song: Some(&info),
                    timeline: Some(&shown.lyrics),
                    lyrics: view.frame(&shown.lyrics, tick),
                    position_ms: song.tempo_map.tick_to_ms(tick),
                    duration_ms: song.duration_ms(),
                    tempo_ratio,
                    show_position: true,
                    ..Frame::idle(&entry)
                };
                draw(
                    &mut canvas,
                    &mut cache,
                    &fonts,
                    &theme,
                    &frame,
                    Background::default(),
                );
                let selected = &session.syllables[session.selected];
                let line = &session.lines[session.line_of(session.selected)];
                let around: String = session.syllables[line.clone()]
                    .iter()
                    .enumerate()
                    .map(|(offset, s)| {
                        if line.start + offset == session.selected {
                            format!("[{}]", s.text.trim())
                        } else {
                            s.text.clone()
                        }
                    })
                    .collect();
                let help_top = screen.help(
                    &mut canvas,
                    &mut cache,
                    &fonts,
                    &theme,
                    &REVIEW_KEYS,
                    show_help,
                );
                // The selected word sits on the key list, wherever that ends.
                let line_height = measure_line(&fonts.text, &["Ag"]).height;
                screen.note(
                    &mut canvas,
                    &mut cache,
                    &fonts,
                    &theme,
                    (help_top - line_height * 1.6) / screen.height,
                    &format!(
                        "{}   at {}",
                        around.trim(),
                        clock(song.tempo_map.tick_to_ms(selected.tick))
                    ),
                );
            }
            _ => {
                canvas.set_draw_color(theme.background);
                canvas.clear();
                screen.tapping(&mut canvas, &mut cache, &fonts, &theme, &session);
                screen.help(
                    &mut canvas,
                    &mut cache,
                    &fonts,
                    &theme,
                    &TAPPING_KEYS,
                    show_help,
                );
            }
        }

        screen.status(&mut canvas, &mut cache, &fonts, &theme, &status);
        if let Some(melody) = melody {
            screen.melody(
                &mut canvas,
                &mut cache,
                &fonts,
                &theme,
                &format!("Snap to {}", melody.label()),
                note_lit,
            );
        }
        if let Some((text, since)) = &message {
            if now.duration_since(*since) < MESSAGE_FOR {
                screen.note(&mut canvas, &mut cache, &fonts, &theme, 0.12, text);
            } else {
                message = None;
            }
        }
        canvas.present();
    }

    stream.send(Command::Stop);
    Ok(())
}

/// A position as minutes, seconds and tenths.
fn clock(ms: u32) -> String {
    format!("{}:{:02}.{}", ms / 60_000, ms / 1_000 % 60, ms / 100 % 10)
}

/// One row of the key list: what its keys act on, then each key with what it does.
type HelpRow = (&'static str, &'static [(&'static str, &'static str)]);

/// The keys while syllables are still waiting for a tick.
///
/// The keys pressed on every word come first, the transport second and the rare ones last. The
/// longest entry of a row closes it, so it widens no column a shorter entry shares.
const TAPPING_KEYS: [HelpRow; 3] = [
    (
        "TAP",
        &[
            ("Space", "next word"),
            ("E", "end the word"),
            ("Backspace", "undo"),
            ("Up", "tap this line again"),
        ],
    ),
    (
        "SONG",
        &[
            ("Enter", "play / pause"),
            ("Left Right", "5 s"),
            ("M", "select channel to snap to"),
            ("-  +", "slower / faster"),
        ],
    ),
    (
        "FILE",
        &[
            ("Ctrl+S", "save"),
            ("Esc", "leave"),
            ("H", "hide these keys"),
        ],
    ),
];

/// The keys once every syllable has a tick.
const REVIEW_KEYS: [HelpRow; 3] = [
    (
        "WORD",
        &[
            ("Up Down", "select"),
            ("E", "end here"),
            ("Backspace", "undo"),
            ("Left Right", "move 10 ms (Shift 50)"),
        ],
    ),
    (
        "SONG",
        &[
            ("Space", "play / pause"),
            ("M", "select channel to snap to"),
            ("-  +", "slower / faster"),
            ("Enter", "play this line"),
        ],
    ),
    (
        "FILE",
        &[
            ("Ctrl+S", "save"),
            ("Esc", "leave"),
            ("H", "hide these keys"),
            ("N", "snap words to its notes (Ctrl+Z undoes)"),
        ],
    ),
];

type Screenful = Canvas<Window>;
type Cache = TextCache<WindowContext>;

/// Where the editor's own text goes, as fractions of the window.
struct Layout {
    width: f32,
    height: f32,
}

impl Layout {
    /// A centered line of small text at this fraction of the height.
    fn note(
        &self,
        canvas: &mut Screenful,
        cache: &mut Cache,
        fonts: &Fonts,
        theme: &Theme,
        at: f32,
        text: &str,
    ) {
        draw_text(
            canvas,
            cache,
            &fonts.text,
            text,
            (self.width / 2.0, self.height * at),
            &TextStyle::outlined(theme.accent, theme, Align::Center),
        );
    }

    /// The key list along the foot of the window. Answers where its top edge is, so whatever sits
    /// above it knows how much room is left.
    fn help(
        &self,
        canvas: &mut Screenful,
        cache: &mut Cache,
        fonts: &Fonts,
        theme: &Theme,
        rows: &[HelpRow],
        shown: bool,
    ) -> f32 {
        let font = &fonts.small;
        let wide = |text: &str| measure_line(font, &[text]).width;
        let en = wide("n");
        let pitch = measure_line(font, &["Ag"]).height * 1.3;
        let left = self.width * 0.02;
        let foot = self.height * 0.98;
        let dim = TextStyle::outlined(theme.text_dim, theme, Align::Left);

        if !shown {
            // Hidden, with the one key that brings it back.
            let top = foot - pitch;
            let hint = TextStyle::outlined(theme.text_dim, theme, Align::Right);
            draw_text(
                canvas,
                cache,
                font,
                "H  keys",
                (self.width * 0.98, top),
                &hint,
            );
            return top;
        }

        let top = foot - pitch * rows.len() as f32;
        canvas.set_blend_mode(BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(
            theme.background.r,
            theme.background.g,
            theme.background.b,
            200,
        ));
        let _ = canvas.fill_rect(FRect::new(
            0.0,
            top - pitch * 0.3,
            self.width,
            self.height - top + pitch * 0.3,
        ));

        // The same columns in every row, each as wide as its widest entry. A window too narrow for
        // that lets each row keep its own widths, which is uneven and still readable.
        let entry = |(key, action): &(&str, &str)| wide(key) + en + wide(action);
        let label = rows.iter().map(|(name, _)| wide(name)).fold(0.0, f32::max) + en * 2.0;
        let columns = rows.iter().map(|(_, keys)| keys.len()).max().unwrap_or(0);
        let widths: Vec<f32> = (0..columns)
            .map(|column| {
                rows.iter()
                    .filter_map(|(_, keys)| keys.get(column))
                    .map(entry)
                    .fold(0.0, f32::max)
            })
            .collect();
        let gap = en * 3.0;
        let aligned =
            left + label + widths.iter().sum::<f32>() + gap * columns as f32 <= self.width * 0.98;

        for (row, (name, keys)) in rows.iter().enumerate() {
            let y = top + pitch * row as f32;
            draw_text(canvas, cache, font, name, (left, y), &dim);
            let mut x = left + label;
            for (column, pair) in keys.iter().enumerate() {
                let (key, action) = pair;
                let bright = TextStyle::outlined(theme.accent, theme, Align::Left);
                let key_width = draw_text(canvas, cache, font, key, (x, y), &bright);
                draw_text(canvas, cache, font, action, (x + key_width + en, y), &dim);
                x += gap + if aligned { widths[column] } else { entry(pair) };
            }
        }
        top
    }

    /// The transport line at the head of the window.
    fn status(
        &self,
        canvas: &mut Screenful,
        cache: &mut Cache,
        fonts: &Fonts,
        theme: &Theme,
        text: &str,
    ) {
        draw_text(
            canvas,
            cache,
            &fonts.small,
            text,
            (self.width * 0.02, self.height * 0.02),
            &TextStyle::outlined(theme.text, theme, Align::Left),
        );
    }

    /// The chosen melody channel, with a mark that lights on each of its notes.
    fn melody(
        &self,
        canvas: &mut Screenful,
        cache: &mut Cache,
        fonts: &Fonts,
        theme: &Theme,
        text: &str,
        lit: bool,
    ) {
        let side = self.height * 0.025;
        let right = self.width * 0.98;
        let top = self.height * 0.02;
        canvas.set_draw_color(if lit { theme.accent } else { theme.panel });
        let _ = canvas.fill_rect(FRect::new(right - side, top, side, side));
        draw_text(
            canvas,
            cache,
            &fonts.small,
            text,
            (right - side * 1.5, top),
            &TextStyle::outlined(theme.text, theme, Align::Right),
        );
    }

    /// The tapping screen: the line being tapped, with the one before it and the one after.
    fn tapping(
        &self,
        canvas: &mut Screenful,
        cache: &mut Cache,
        fonts: &Fonts,
        theme: &Theme,
        session: &Session,
    ) {
        let next = session.stamped.min(session.syllables.len() - 1);
        let current = session.line_of(next);
        let rows = [
            (current.checked_sub(1), 0.28),
            (Some(current), 0.45),
            (
                current.checked_add(1).filter(|&l| l < session.lines.len()),
                0.62,
            ),
        ];
        for (line, at) in rows {
            let Some(line) = line else { continue };
            let range = session.lines[line].clone();
            let texts: Vec<&str> = session.syllables[range.clone()]
                .iter()
                .enumerate()
                .map(|(offset, s)| {
                    if offset == 0 {
                        s.text.trim_start()
                    } else {
                        s.text.as_str()
                    }
                })
                .collect();
            // The widest face the line fits in, as the machine chooses one.
            let font = std::iter::once(&fonts.lyric)
                .chain(fonts.lyric_narrow.iter())
                .find(|font| measure_line(font, &texts).width <= self.width * 0.94)
                .or(fonts.lyric_narrow.last())
                .unwrap_or(&fonts.lyric);
            let metrics = measure_line(font, &texts);
            let left = (self.width - metrics.width) / 2.0;
            for (offset, text) in texts.iter().enumerate() {
                let index = range.start + offset;
                let color = match index.cmp(&session.stamped) {
                    std::cmp::Ordering::Less => theme.lyric_sung,
                    std::cmp::Ordering::Equal => theme.accent,
                    std::cmp::Ordering::Greater => theme.lyric_pending,
                };
                let color = if line == current { color } else { dim(color) };
                // A syllable that opens a word carries its space, which the offsets already count.
                let shown = text.trim_start();
                let lead = measure_line(font, &[&text[..text.len() - shown.len()]]).width;
                draw_text(
                    canvas,
                    cache,
                    font,
                    shown,
                    (left + metrics.offsets[offset] + lead, self.height * at),
                    &TextStyle::outlined(color, theme, Align::Left),
                );
            }
        }
    }
}

/// A color at half strength, for the lines that are not the one being tapped.
fn dim(color: Color) -> Color {
    Color::RGBA(color.r / 2, color.g / 2, color.b / 2, color.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(typed: &str) -> Session {
        Session::new(split_words(typed))
    }

    fn ticks(session: &Session) -> Vec<u32> {
        session.tapped().iter().map(|s| s.tick).collect()
    }

    #[test]
    fn taps_fill_the_syllables_in_order_and_the_last_one_opens_review() {
        let mut s = session("la la\nla");
        assert_eq!(s.lines, [0..2, 2..3]);
        s.tap(100);
        s.tap(200);
        assert_eq!(s.phase(), Phase::Tapping);
        s.tap(300);
        assert_eq!(s.phase(), Phase::Review);
        assert_eq!(ticks(&s), [100, 200, 300]);
        s.tap(400);
        assert_eq!(ticks(&s), [100, 200, 300]);
    }

    #[test]
    fn a_tap_never_lands_before_the_one_in_front_of_it() {
        let mut s = session("la la");
        s.tap(500);
        s.tap(400);
        assert_eq!(ticks(&s), [500, 500]);
    }

    #[test]
    fn undo_takes_back_one_tap_and_nothing_when_there_is_none() {
        let mut s = session("la la");
        assert!(!s.undo());
        s.tap(100);
        assert!(s.undo());
        assert_eq!(s.stamped, 0);
    }

    #[test]
    fn a_word_is_ended_after_its_start_and_the_next_tap_comes_after_the_end() {
        let mut s = session("la la la");
        s.tap(100);
        // Not before the word starts, and not on a word nobody has tapped.
        assert!(!s.end_at(0, 100));
        assert!(!s.end_at(1, 500));
        assert!(s.end_at(0, 300));
        assert_eq!(s.syllables[0].end_tick, Some(300));
        s.tap(250);
        assert_eq!(ticks(&s), [100, 300]);
        // An end set in review stops at the next word.
        s.tap(600);
        assert!(s.end_at(1, 900));
        assert_eq!(s.syllables[1].end_tick, Some(600));
    }

    #[test]
    fn undo_takes_back_an_end_before_it_takes_back_the_tap() {
        let mut s = session("la la");
        s.tap(100);
        s.end_at(0, 300);
        assert!(s.undo());
        assert_eq!((s.stamped, s.syllables[0].end_tick), (1, None));
        assert!(s.undo());
        assert_eq!(s.stamped, 0);
    }

    #[test]
    fn a_moved_syllable_stays_before_its_own_end() {
        let mut s = session("a b");
        s.tap(100);
        s.end_at(0, 200);
        s.tap(400);
        s.selected = 0;
        s.move_selected(350);
        assert_eq!(ticks(&s), [200, 400]);
    }

    #[test]
    fn tapping_a_line_again_clears_that_line_and_plays_from_the_one_before() {
        let mut s = session("one two\nthree four\nfive");
        for tick in [100, 200, 300] {
            s.tap(tick);
        }
        // Part-way through the second line: that line goes.
        assert_eq!(s.retap_line(), 200);
        assert_eq!(s.stamped, 2);
        // At the start of a line with no taps: the line before goes.
        assert_eq!(s.retap_line(), 0);
        assert_eq!(s.stamped, 0);
    }

    #[test]
    fn a_moved_syllable_stays_between_its_neighbours() {
        let mut s = session("a b c");
        for tick in [100, 200, 300] {
            s.tap(tick);
        }
        s.selected = 1;
        s.move_selected(50);
        assert_eq!(ticks(&s), [100, 100, 300]);
        s.move_selected(900);
        assert_eq!(ticks(&s), [100, 300, 300]);
        s.selected = 2;
        s.move_selected(900);
        assert_eq!(ticks(&s), [100, 300, 900]);
    }

    #[test]
    fn a_snap_moves_near_syllables_onto_notes_and_is_taken_back_whole() {
        let mut s = session("a b c d");
        for tick in [98, 215, 290, 800] {
            s.tap(tick);
        }
        let moved = s.snap(&[100, 200, 300, 400], |a, b| a.abs_diff(b) <= 20);
        // 800 has no note within reach and stays.
        assert_eq!(moved, 3);
        assert_eq!(ticks(&s), [100, 200, 300, 800]);
        assert!(s.undo_snap());
        assert_eq!(ticks(&s), [98, 215, 290, 800]);
        assert!(!s.undo_snap());
    }

    #[test]
    fn a_snap_never_carries_a_syllable_past_its_neighbour() {
        let mut s = session("a b");
        s.tap(190);
        s.tap(195);
        // Both are nearest to 200. The first would pass the second to reach it, so it stays.
        let moved = s.snap(&[100, 200], |_, _| true);
        assert_eq!(ticks(&s), [190, 200]);
        assert_eq!(moved, 1);
    }

    #[test]
    fn the_output_goes_beside_the_song_and_is_never_the_song() {
        assert_eq!(
            default_out(Path::new("a/song.mid")),
            Path::new("a/song.kar")
        );
        assert_eq!(
            default_out(Path::new("a/song.kar")),
            Path::new("a/song-synced.kar")
        );
    }

    #[test]
    fn the_nearest_value_is_found_on_either_side() {
        assert_eq!(nearest(&[], 5), None);
        assert_eq!(nearest(&[10, 20], 5), Some(10));
        assert_eq!(nearest(&[10, 20], 14), Some(10));
        assert_eq!(nearest(&[10, 20], 16), Some(20));
        assert_eq!(nearest(&[10, 20], 99), Some(20));
    }
}
