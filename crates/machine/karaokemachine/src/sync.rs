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

use std::io::Read as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use km_audio::audio::{Command, Load, SharedState};
use km_audio::{AudioError, Bank, OutputStream, SoundFontSource, TestToneSource};
use km_display::draw::{Background, Frame, Screen, SongInfo, draw};
use km_display::lyrics::{LyricView, shift_ticks};
use km_display::numbers::NumberEntry;
use km_display::text::{Align, Fonts, TextCache, TextError, TextStyle, draw_text, measure_line};
use km_display::theme::Theme;
use km_locale::{Catalog, Catalogs, Locale};
use km_queue::Transport;
use km_song::kar_write::{KarWords, split_words, write_soft_karaoke};
use km_song::timeline::{LineBreak, RawSyllable};
use km_song::{LyricTimeline, ParseOptions, SYLLABLE_DIVIDER, Song, WordEnds};
use sdl3::event::{Event as SdlEvent, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::pixels::Color;
use sdl3::render::{BlendMode, Canvas, FRect};
use sdl3::ttf::Sdl3TtfContext;
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
    pub words: Option<PathBuf>,
    /// Whether the typed words keep the ticks the song file already holds for the leading ones.
    pub resume: bool,
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
    /// Whether review was asked for while words are still waiting. Review needs no asking once
    /// every word is tapped.
    reviewing: bool,
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
            reviewing: false,
        }
    }

    /// The same words with the first `tapped` of them holding the ticks they came with.
    fn with_tapped(mut self, tapped: usize) -> Self {
        self.stamped = tapped.min(self.syllables.len());
        self.selected = self.stamped.saturating_sub(1);
        self
    }

    fn phase(&self) -> Phase {
        if self.stamped == self.syllables.len() || (self.reviewing && self.stamped > 0) {
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

    /// Takes back every tap and every end, so tapping starts at the first word.
    fn clear(&mut self) -> bool {
        if self.stamped == 0 {
            return false;
        }
        for syllable in &mut self.syllables[..self.stamped] {
            syllable.end_tick = None;
        }
        self.stamped = 0;
        self.selected = 0;
        self.reviewing = false;
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

    /// Selects the tapped syllable being sung at `tick`: the last one that starts at or before
    /// it, and the first one while the song is before every word.
    fn select_at(&mut self, tick: u32) {
        let started = self.syllables[..self.stamped].partition_point(|s| s.tick <= tick);
        self.selected = started.saturating_sub(1);
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

/// How many taps the editor waits for before it reads a vocal line out of them.
const GUESS_AFTER_TAPS: usize = 24;

/// Which channel the taps follow, when one does.
///
/// A tap follows a channel when one of its notes starts within [`SNAP_WINDOW_MS`] of the tap. A
/// busy part has a note near every moment, so each channel is judged on how far it beats the share
/// its own density would reach by chance. The answer is `None` unless most taps land on a
/// channel, well above chance.
///
/// **This is the editor's own measure, and not `km_suitability::melody::detect`.** That one asks
/// for a note on each syllable, and a person tapping whole words gives it one tap for several notes.
fn vocal_line_under(taps_ms: &[u32], channels_ms: &[Vec<u32>]) -> Option<usize> {
    let (&first, &last) = (taps_ms.first()?, taps_ms.last()?);
    if taps_ms.len() < GUESS_AFTER_TAPS {
        return None;
    }
    let from = first.saturating_sub(SNAP_WINDOW_MS);
    let to = last.saturating_add(SNAP_WINDOW_MS);
    let span = (to - from).max(1) as f32;

    let mut scored: Vec<(usize, f32, f32)> = channels_ms
        .iter()
        .enumerate()
        .map(|(index, onsets)| {
            let hits = taps_ms
                .iter()
                .filter(|&&tap| {
                    nearest(onsets, tap).is_some_and(|onset| onset.abs_diff(tap) <= SNAP_WINDOW_MS)
                })
                .count();
            let hit = hits as f32 / taps_ms.len() as f32;
            let notes =
                onsets.partition_point(|&o| o <= to) - onsets.partition_point(|&o| o < from);
            // Notes scattered at random would cover this share of the span with their windows.
            // Windows overlap, so the share approaches one and never passes it.
            let covered = notes as f32 * 2.0 * SNAP_WINDOW_MS as f32 / span;
            let chance = 1.0 - (-covered).exp();
            (index, hit, hit - chance)
        })
        .collect();
    // A sung line is often doubled on a second channel that plays more besides. Among the channels
    // the taps follow, the one furthest above chance is the sparser, and the likelier to be the
    // voice alone. A part carrying the tune inside a harmony has more notes for a snap to find,
    // and more wrong ones.
    scored.retain(|&(_, hit, lead)| hit >= 0.7 && lead >= 0.25);
    scored.sort_by(|a, b| b.2.total_cmp(&a.2).then(b.1.total_cmp(&a.1)));
    scored.first().map(|&(index, _, _)| index)
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
    /// The channel as a person names it, numbered from 1, with the track name the file gives it.
    fn label(&self, words: &Catalog) -> String {
        let number = i64::from(self.channel) + 1;
        match &self.name {
            Some(name) => words.msg_with(
                "sync-channel-named",
                &[("number", number.into()), ("name", name.as_str().into())],
            ),
            None => words.msg_with("sync-channel", &[("number", number.into())]),
        }
        .into_owned()
    }
}

/// Every channel that plays notes and is not the drums, in channel order.
///
/// The order is the one a person steps through, so it is the one they can predict. Which channel
/// is likeliest is [`vocal_line_under`]'s question, and it asks the taps.
fn melody_choices(song: &Song) -> Vec<MelodyChoice> {
    let thresholds = km_suitability::Thresholds::default();
    let mut choices: Vec<MelodyChoice> = km_suitability::channel::measure(song, &thresholds)
        .into_iter()
        .filter(|s| s.note_count > 0 && s.channel != km_suitability::DRUM_CHANNEL)
        .map(|s| MelodyChoice {
            channel: s.channel,
            name: s.track_names.first().cloned(),
            onsets: s.onset_ticks,
        })
        .collect();
    choices.sort_by_key(|choice| choice.channel);
    choices
}

/// The editor's own words, one catalog per locale.
///
/// Compiled in, as the screen's catalog is, so a build cannot travel without them.
static CATALOGS: Catalogs = Catalogs::new(
    "sync",
    &[
        (Locale::English, include_str!("../i18n/en.ftl")),
        (
            Locale::BrazilianPortuguese,
            include_str!("../i18n/pt-BR.ftl"),
        ),
    ],
);

/// The editor's messages for one locale, parsed once.
fn messages(locale: Locale) -> &'static Catalog {
    CATALOGS.get(locale)
}

/// The sound, which outlives any one open device.
///
/// A device can go away while a song plays. The stream is then dropped and opened again, and the
/// bank and the state the position is read from stay.
struct Sound {
    /// The open stream, or `None` between a loss and the next successful open.
    stream: Option<OutputStream>,
    shared: Arc<SharedState>,
    /// The owner's bank, parsed once. `None` plays the test tone.
    bank: Option<Bank>,
    /// The device named in settings, asked for again at each open.
    want: Option<String>,
    volume: f32,
    /// The earliest moment the next open is tried.
    retry_at: Instant,
}

impl Sound {
    /// How long to wait between two attempts to open a device that is not there.
    const RETRY_EVERY: Duration = Duration::from_secs(2);

    /// Opens the sound: the owner's bank, or the test tone when there is none to load.
    fn open(paths: &Paths, settings: &Settings) -> anyhow::Result<Self> {
        let selected = crate::soundfont::resolve(paths, settings.audio.soundfont.as_deref());
        let bank = crate::engine::resolve_soundfont(selected.path.as_deref(), paths)
            .and_then(|path| Bank::load(&path).map_err(|error| error.to_string()));
        if let Err(reason) = &bank {
            tracing::warn!(%reason, "no SoundFont, so the song plays as a test tone");
        }
        let mut sound = Self {
            stream: None,
            shared: Arc::new(SharedState::default()),
            bank: bank.ok(),
            want: settings.audio.output_device.clone(),
            volume: settings.audio.music_volume,
            retry_at: Instant::now(),
        };
        sound.stream = Some(sound.open_stream().context("no audio output")?);
        sound.send(Command::SetMusicVolume(sound.volume));
        Ok(sound)
    }

    fn open_stream(&self) -> Result<OutputStream, AudioError> {
        let shared = Arc::clone(&self.shared);
        let want = self.want.as_deref();
        match &self.bank {
            Some(bank) => {
                OutputStream::open(shared, want, |rate| SoundFontSource::from_bank(bank, rate))
            }
            None => OutputStream::open(shared, want, |rate| Ok(TestToneSource::new(rate))),
        }
    }

    /// Sends a command to the open stream. With no stream there is nothing to hear it.
    fn send(&mut self, command: Command) {
        if let Some(stream) = &mut self.stream {
            stream.send(command);
        }
    }

    fn collect_retired(&mut self) {
        if let Some(stream) = &mut self.stream {
            stream.collect_retired();
        }
    }

    /// Drops a stream that reported an error. Answers whether one was dropped.
    fn drop_failed(&mut self) -> bool {
        if self.stream.is_none() || !self.shared.stream_failed() {
            return false;
        }
        self.shared.publish_lost();
        self.stream = None;
        true
    }

    /// Tries to open a stream when there is none. Answers whether one opened.
    fn reopen(&mut self, now: Instant) -> bool {
        if self.stream.is_some() || now < self.retry_at {
            return false;
        }
        match self.open_stream() {
            Ok(stream) => {
                self.stream = Some(stream);
                self.send(Command::SetMusicVolume(self.volume));
                true
            }
            Err(error) => {
                tracing::debug!(%error, "the audio output is still not there");
                self.retry_at = now + Self::RETRY_EVERY;
                false
            }
        }
    }
}

/// Loads the song and puts it where it was, with `silence` as the channel that does not sound.
///
/// The engine silences one channel, the one a song is loaded with as its guide melody. A load
/// starts the song over, so the position, the tempo and the transport are sent after it.
fn load_song(
    sound: &mut Sound,
    song: &Arc<Song>,
    silence: Option<u8>,
    at_ms: u32,
    playing: bool,
    tempo_ratio: f32,
) {
    sound.send(Command::Load(Load::Midi {
        song: Arc::clone(song),
        melody_channel: silence,
        fixes: km_fixes::ChannelFixes::default(),
    }));
    sound.send(Command::SetMelodyEnabled(false));
    sound.send(Command::SetTempoRatio(tempo_ratio));
    sound.send(Command::SeekMs(at_ms));
    if playing {
        sound.send(Command::Play);
    }
}

/// The fonts for a window of this height, from the owner's settings.
fn open_fonts(
    ttf: &Sdl3TtfContext,
    settings: &Settings,
    bundled: &Path,
    with_cjk: bool,
    theme: &Theme,
    height: u32,
) -> Result<Fonts, TextError> {
    Fonts::discover(
        ttf,
        settings.display.font.as_deref(),
        Some(bundled),
        settings.display.font_cjk.as_deref(),
        with_cjk,
        theme,
        height,
    )
}

/// The words to edit, and how many of them come with a tick already.
///
/// Three cases, chosen by what the person passed and never by looking at the file:
///
/// * words and no `continue`: the typed words, none tapped. A file's own words are replaced.
/// * no words: the file's own words with the ticks it gives them, all tapped.
/// * words and `continue`: the typed words, with the file's ticks on the leading ones.
fn starting_words(song: &Song, request: &Request) -> anyhow::Result<(Vec<RawSyllable>, usize)> {
    let held = syllables_of(&song.lyrics, u32::from(song.ticks_per_quarter.max(1)));
    let Some(path) = &request.words else {
        anyhow::ensure!(
            !held.is_empty(),
            "{} holds no words; pass --sync-words with the words to tap",
            request.song.display()
        );
        let tapped = held.len();
        return Ok((held, tapped));
    };
    let typed = split_words(&read_words(path)?);
    anyhow::ensure!(!typed.is_empty(), "{} holds no words", path.display());
    if !request.resume {
        return Ok((typed, 0));
    }
    continue_from(typed, &held).map_err(|fault| {
        anyhow::anyhow!(
            "{} does not continue from {}: {fault}",
            path.display(),
            request.song.display()
        )
    })
}

/// A song's words as the syllables an editor holds: each with its tick, its break and its end.
///
/// A syllable takes an end only where the file gave it one, which is where it stops short of the
/// next. The text is the file's own spacing, with the dividers the reader drew taken back out.
fn syllables_of(lyrics: &LyricTimeline, beat: u32) -> Vec<RawSyllable> {
    let mut out: Vec<RawSyllable> = Vec::new();
    let mut page = None;
    for line in &lyrics.lines {
        for (index, syllable) in line.syllables.iter().enumerate() {
            let break_before = match (index, page) {
                (0, Some(before)) if before == line.page => LineBreak::Line,
                (0, _) => LineBreak::Page,
                _ => LineBreak::None,
            };
            // The one before this ended early if it stops before this one starts.
            if let Some(before) = out.last_mut()
                && before
                    .end_tick
                    .is_some_and(|end| end >= syllable.start_tick)
            {
                before.end_tick = None;
            }
            let text = match lyrics.word_ends {
                WordEnds::AsWritten => syllable.text.clone(),
                WordEnds::EverySyllableSpaced => syllable.text.replace(SYLLABLE_DIVIDER, " "),
                WordEnds::NoneSpaced => syllable.text.replace(SYLLABLE_DIVIDER, ""),
            };
            out.push(RawSyllable {
                tick: syllable.start_tick,
                text,
                break_before,
                end_tick: Some(syllable.end_tick),
            });
        }
        page = Some(line.page);
    }
    // The last syllable of a song has nothing after it to stop short of. A file that gives it no
    // end has it held for one beat, so one beat is no end.
    if let Some(last) = out.last_mut()
        && last.end_tick == Some(last.tick.saturating_add(beat))
    {
        last.end_tick = None;
    }
    out
}

/// Gives the typed words the ticks a file already holds for the leading ones.
///
/// The file's words must be the start of the typed words, compared without case or outer space. A
/// word that differs is named, since ticks on the wrong words are worse than none.
fn continue_from(
    mut typed: Vec<RawSyllable>,
    held: &[RawSyllable],
) -> Result<(Vec<RawSyllable>, usize), String> {
    if held.len() > typed.len() {
        return Err(format!(
            "the file times {} words and the text has {}",
            held.len(),
            typed.len()
        ));
    }
    for (index, (mine, theirs)) in typed.iter_mut().zip(held).enumerate() {
        let same = mine.text.trim().to_lowercase() == theirs.text.trim().to_lowercase();
        if !same {
            return Err(format!(
                "word {} is \"{}\" in the file and \"{}\" in the text",
                index + 1,
                theirs.text.trim(),
                mine.text.trim()
            ));
        }
        mine.tick = theirs.tick;
        mine.end_tick = theirs.end_tick;
    }
    Ok((typed, held.len()))
}

/// The typed words: a file, or standard input when the path is `-`.
///
/// Standard input is for a program that starts the editor with words it holds and has no file for.
fn read_words(path: &Path) -> anyhow::Result<String> {
    if path == Path::new("-") {
        let mut typed = String::new();
        std::io::stdin()
            .read_to_string(&mut typed)
            .context("reading the words from standard input")?;
        return Ok(typed);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Runs the editor until its window closes.
pub(crate) fn run(paths: &Paths, settings: &Settings, request: &Request) -> anyhow::Result<()> {
    let source = std::fs::read(&request.song)
        .with_context(|| format!("reading {}", request.song.display()))?;
    let song = Arc::new(
        Song::parse(&source, &ParseOptions::default())
            .with_context(|| format!("{} is not a MIDI file", request.song.display()))?,
    );
    let (syllables, already_tapped) = starting_words(&song, request)?;

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

    let locale = settings.machine.locale();
    let words = messages(locale);
    let t = |key: &str| words.msg(key).into_owned();

    let choices = melody_choices(&song);
    // No channel is chosen until somebody names one or the taps point at one. A file with no words
    // gives a ranking nothing to tie a channel to the singing with.
    let mut choice: Option<usize> = request
        .melody_channel
        .and_then(|wanted| choices.iter().position(|c| c.channel + 1 == wanted));
    let onsets_ms: Vec<Vec<u32>> = choices
        .iter()
        .map(|c| {
            c.onsets
                .iter()
                .map(|&t| song.tempo_map.tick_to_ms(t))
                .collect()
        })
        .collect();
    let offset_ms = request
        .tap_offset_ms
        .unwrap_or_else(|| settings.display.lyric_offset());

    let mut sound = Sound::open(paths, settings)?;
    let state = Arc::clone(&sound.shared);
    load_song(&mut sound, &song, None, 0, false, 1.0);

    let sdl = sdl3::init().map_err(|error| anyhow::anyhow!("SDL would not start: {error}"))?;
    let video = sdl
        .video()
        .map_err(|error| anyhow::anyhow!("no video: {error}"))?;
    let ttf =
        sdl3::ttf::init().map_err(|error| anyhow::anyhow!("SDL_ttf would not start: {error}"))?;
    let window_title = words
        .msg_with("sync-window-title", &[("title", title.as_str().into())])
        .into_owned();
    let mut window = video
        .window(&window_title, WINDOW.0, WINDOW.1)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()?;
    km_display::set_window_icon(&mut window);
    let mut canvas = window.into_canvas();
    crate::display::request_vsync(&canvas);
    let mut cache = TextCache::new(canvas.texture_creator());
    let (width, height) = canvas.output_size().unwrap_or(WINDOW);
    let theme = Theme::default();
    let bundled = paths.asset(crate::settings::FONT_SUBPATH);
    // Without CJK faces to start with, as the machine starts. They are opened when a word asks.
    let mut with_cjk = false;
    let mut fonts =
        open_fonts(&ttf, settings, &bundled, with_cjk, &theme, height).map_err(|error| {
            anyhow::anyhow!("no usable font: {error}. Set display.font in settings.json")
        })?;
    let mut font_sizes = crate::display::font_sizes_for(&theme, height);
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
    let mut screen = Layout {
        width: width as f32,
        height: height as f32,
    };

    let mut session = Session::new(syllables).with_tapped(already_tapped);
    // Tapping goes on from the word after the last one the file timed, with a run-up.
    if session.phase() == Phase::Tapping && already_tapped > 0 {
        let last = session.syllables[already_tapped - 1].tick;
        let ms = song.tempo_map.tick_to_ms(last).saturating_sub(RUN_UP_MS);
        sound.send(Command::SeekMs(ms));
    }
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
    let mut was_in_review = false;
    // Whether the vocal line is silenced, and which channel the loaded song can silence.
    let mut silenced = false;
    let mut silencing: Option<u8> = None;
    // Where the song was, and whether it played, when its device went away.
    let mut lost: Option<(u32, bool)> = None;
    let opening = match (already_tapped, session.phase()) {
        (0, _) => t("sync-start"),
        (_, Phase::Review) => t("sync-reopened"),
        (kept, Phase::Tapping) => words
            .msg_with("sync-continued", &[("tapped", (kept as i64).into())])
            .into_owned(),
    };
    let mut message: Option<(String, Instant)> = Some((opening, Instant::now()));

    'frames: loop {
        let now = Instant::now();
        let playing = state.transport() == Transport::Playing;
        let reported = smoother.smooth(state.position_ticks(), state.period_ms(), playing, now);
        // The tick a person is hearing, which is behind the tick the engine has rendered by what
        // the room's lyric offset measures.
        let tick = shift_ticks(&song.tempo_map, reported, offset_ms, tempo_ratio);
        let phase = session.phase();

        let mut say = |text: String| message = Some((text, now));

        // A device that went away takes the stream with it. The song goes back where it was once
        // a device opens, playing if it was.
        if sound.drop_failed() {
            lost = Some((state.position_ms(), want_playing));
            say(t("sync-no-audio"));
        }
        if let Some((at_ms, was_playing)) = lost
            && sound.reopen(now)
        {
            let silence = silencing.filter(|_| silenced);
            silencing = silence;
            load_song(&mut sound, &song, silence, at_ms, was_playing, tempo_ratio);
            lost = None;
            say(t("sync-audio-back"));
        }

        for event in events.poll_iter() {
            let (key, keymod, repeat) = match event {
                SdlEvent::Quit { .. } => {
                    if session.dirty && !leaving {
                        leaving = true;
                        say(t("sync-unsaved-close"));
                        continue;
                    }
                    break 'frames;
                }
                SdlEvent::Window {
                    win_event: WindowEvent::PixelSizeChanged(new_width, new_height),
                    ..
                } => {
                    let new_height = new_height.max(1) as u32;
                    screen.width = new_width.max(1) as f32;
                    screen.height = new_height as f32;
                    // The same seam the machine's resize goes through. A drag that moves no rounded
                    // point size opens no font file.
                    let wanted = crate::display::font_sizes_for(&theme, new_height);
                    if wanted != font_sizes
                        && let Ok(rebuilt) =
                            open_fonts(&ttf, settings, &bundled, with_cjk, &theme, new_height)
                    {
                        fonts = rebuilt;
                        font_sizes = wanted;
                        // The cache's keys carry the address of the font that drew each string.
                        cache.clear();
                    }
                    continue;
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
                        say(t("sync-unsaved-esc"));
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
                            let file = out.display().to_string();
                            say(words
                                .msg_with(
                                    "sync-saved",
                                    &[
                                        ("tapped", (session.stamped as i64).into()),
                                        ("total", (session.syllables.len() as i64).into()),
                                        ("file", file.as_str().into()),
                                    ],
                                )
                                .into_owned());
                        }
                        Err(error) => {
                            let reason = error.to_string();
                            say(words
                                .msg_with("sync-not-saved", &[("reason", reason.as_str().into())])
                                .into_owned());
                        }
                    }
                }
                (_, Keycode::Minus | Keycode::KpMinus) => {
                    tempo_ratio = (tempo_ratio - 0.05).max(0.5);
                    sound.send(Command::SetTempoRatio(tempo_ratio));
                }
                (_, Keycode::Equals | Keycode::Plus | Keycode::KpPlus) => {
                    tempo_ratio = (tempo_ratio + 0.05).min(1.5);
                    sound.send(Command::SetTempoRatio(tempo_ratio));
                }
                (_, Keycode::H) if !repeat => show_help = !show_help,
                (_, Keycode::R) if !repeat => {
                    let all = session.stamped == session.syllables.len();
                    if session.stamped == 0 {
                        say(t("sync-nothing-to-review"));
                    } else if all {
                        say(t("sync-all-tapped-already"));
                    } else if session.reviewing {
                        session.reviewing = false;
                        // Back to where the taps stopped, with a run-up to the next word.
                        let last = session.syllables[session.stamped - 1].tick;
                        let ms = song.tempo_map.tick_to_ms(last).saturating_sub(RUN_UP_MS);
                        sound.send(Command::SeekMs(ms));
                        say(t("sync-tapping-again"));
                    } else {
                        session.reviewing = true;
                        session.selected = session.selected.min(session.stamped - 1);
                        preview_stale = true;
                        say(words
                            .msg_with(
                                "sync-review-so-far",
                                &[("tapped", (session.stamped as i64).into())],
                            )
                            .into_owned());
                    }
                }
                (_, Keycode::M) if !choices.is_empty() => {
                    // Up the channel numbers, and down them with Shift.
                    let count = choices.len();
                    let next = match (choice, shift) {
                        (None, false) => 0,
                        (None, true) => count - 1,
                        (Some(c), false) => (c + 1) % count,
                        (Some(c), true) => (c + count - 1) % count,
                    };
                    choice = Some(next);
                    let channel = choices[next].label(words);
                    if silenced {
                        // The silence follows the choice, so each press is heard.
                        silencing = Some(choices[next].channel);
                        load_song(
                            &mut sound,
                            &song,
                            silencing,
                            state.position_ms(),
                            playing,
                            tempo_ratio,
                        );
                        say(words
                            .msg_with(
                                "sync-vocal-is-silenced",
                                &[("channel", channel.as_str().into())],
                            )
                            .into_owned());
                    } else {
                        say(words
                            .msg_with("sync-vocal-is", &[("channel", channel.as_str().into())])
                            .into_owned());
                    }
                }
                (_, Keycode::V) if !repeat => match choice {
                    None => say(t("sync-no-vocal")),
                    Some(chosen) if silenced => {
                        silenced = false;
                        sound.send(Command::SetMelodyEnabled(true));
                        let channel = choices[chosen].label(words);
                        say(words
                            .msg_with("sync-sounds-again", &[("channel", channel.as_str().into())])
                            .into_owned());
                    }
                    Some(chosen) => {
                        silenced = true;
                        let channel = choices[chosen].channel;
                        // The engine silences the one channel a song was loaded with. A channel
                        // chosen since the load needs the song loaded again.
                        if silencing == Some(channel) {
                            sound.send(Command::SetMelodyEnabled(false));
                        } else {
                            silencing = Some(channel);
                            load_song(
                                &mut sound,
                                &song,
                                silencing,
                                state.position_ms(),
                                playing,
                                tempo_ratio,
                            );
                        }
                        let channel = choices[chosen].label(words);
                        say(words
                            .msg_with("sync-silenced", &[("channel", channel.as_str().into())])
                            .into_owned());
                    }
                },

                (Phase::Tapping, Keycode::Space) if !repeat => {
                    if playing {
                        session.tap(tick);
                        preview_stale = true;
                        // Only while nobody has chosen: a channel named by a person or found
                        // earlier is never replaced by a later guess.
                        if choice.is_none() {
                            let taps_ms: Vec<u32> = session
                                .tapped()
                                .iter()
                                .map(|s| song.tempo_map.tick_to_ms(s.tick))
                                .collect();
                            choice = vocal_line_under(&taps_ms, &onsets_ms);
                            if let Some(found) = choice {
                                let channel = choices[found].label(words);
                                say(words
                                    .msg_with(
                                        "sync-taps-follow",
                                        &[("channel", channel.as_str().into())],
                                    )
                                    .into_owned());
                            }
                        }
                        if session.phase() == Phase::Review {
                            say(t("sync-all-tapped"));
                        }
                    } else {
                        say(t("sync-paused-tap"));
                    }
                }
                (_, Keycode::E) if !repeat => {
                    // While tapping it is the word just tapped; in review, the selected one.
                    let index = match phase {
                        Phase::Tapping => session.stamped.saturating_sub(1),
                        Phase::Review => session.selected,
                    };
                    if playing && session.end_at(index, tick) {
                        preview_stale = true;
                        let word = session.syllables[index].text.trim().to_owned();
                        say(words
                            .msg_with("sync-word-ends", &[("word", word.as_str().into())])
                            .into_owned());
                    } else {
                        say(t("sync-end-refused"));
                    }
                }
                (Phase::Tapping, Keycode::Return | Keycode::KpEnter) => {
                    want_playing = !playing;
                    sound.send(if playing {
                        Command::Pause
                    } else {
                        Command::Play
                    });
                }
                (Phase::Tapping, Keycode::Backspace) => {
                    preview_stale |= session.undo();
                }
                (Phase::Tapping, Keycode::Up) => {
                    let from = session.retap_line();
                    preview_stale = true;
                    let ms = song.tempo_map.tick_to_ms(from).saturating_sub(RUN_UP_MS);
                    sound.send(Command::SeekMs(ms));
                }
                (Phase::Tapping, Keycode::Left) => {
                    sound.send(Command::SeekMs(state.position_ms().saturating_sub(SEEK_MS)));
                }
                (Phase::Tapping, Keycode::Right) => {
                    sound.send(Command::SeekMs(state.position_ms().saturating_add(SEEK_MS)));
                }

                (Phase::Review, Keycode::Space) if !repeat => {
                    want_playing = !playing;
                    sound.send(if playing {
                        Command::Pause
                    } else {
                        Command::Play
                    });
                }
                (Phase::Review, Keycode::Up) => session.select(false),
                (Phase::Review, Keycode::Down) => session.select(true),
                (Phase::Review, Keycode::C) if !ctrl => session.select_at(tick),
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
                    sound.send(Command::SeekMs(ms));
                    sound.send(Command::Play);
                    want_playing = true;
                }
                (Phase::Review, Keycode::N) => match choice.and_then(|c| choices.get(c)) {
                    Some(melody) => {
                        let map = &song.tempo_map;
                        let moved = session.snap(&melody.onsets, |a, b| {
                            map.tick_to_ms(a).abs_diff(map.tick_to_ms(b)) <= SNAP_WINDOW_MS
                        });
                        preview_stale = true;
                        let channel = melody.label(words);
                        say(words
                            .msg_with(
                                "sync-snapped",
                                &[
                                    ("moved", (moved as i64).into()),
                                    ("channel", channel.as_str().into()),
                                ],
                            )
                            .into_owned());
                    }
                    None => say(t("sync-no-vocal")),
                },
                (Phase::Review, Keycode::Z) if ctrl => {
                    if session.undo_snap() {
                        preview_stale = true;
                        say(t("sync-snap-undone"));
                    }
                }
                // Two modifiers, because one stray key must not take a whole song's taps.
                (Phase::Review, Keycode::Backspace) if ctrl && shift => {
                    if !repeat && session.clear() {
                        preview_stale = true;
                        sound.send(Command::Pause);
                        sound.send(Command::SeekMs(0));
                        want_playing = false;
                        say(t("sync-cleared"));
                    }
                }
                (Phase::Review, Keycode::Backspace) => {
                    preview_stale |= session.undo();
                }
                _ => {}
            }
        }

        // Review opens on the first word, with a run-up, and playing. The song was wherever the
        // taps left it, which is past everything there is to check.
        let in_review = session.phase() == Phase::Review;
        if in_review && !was_in_review {
            let first = session.syllables[0].tick;
            let ms = song.tempo_map.tick_to_ms(first).saturating_sub(RUN_UP_MS);
            sound.send(Command::SeekMs(ms));
            sound.send(Command::Play);
            want_playing = true;
        }
        was_in_review = in_review;

        // A song that ran out. Review goes round again, and tapping waits for the person.
        let ended = state.songs_ended();
        if ended != ended_seen {
            ended_seen = ended;
            if in_review && want_playing {
                sound.send(Command::Restart);
                sound.send(Command::Play);
            } else {
                want_playing = false;
            }
        }
        sound.collect_retired();

        if preview_stale && in_review {
            preview = write_soft_karaoke(&source, &words_for(&session))
                .ok()
                .and_then(|bytes| Song::parse(&bytes, &ParseOptions::default()).ok());
            preview_stale = false;
        }

        cache.begin_frame();
        let melody = choice.and_then(|c| choices.get(c));
        let note_lit = melody.is_some_and(|m| {
            let since = song
                .tempo_map
                .ms_to_tick(song.tempo_map.tick_to_ms(tick).saturating_sub(NOTE_MARK_MS));
            let first = m.onsets.partition_point(|&onset| onset < since);
            m.onsets.get(first).is_some_and(|&onset| onset <= tick)
        });
        let position = clock(song.tempo_map.tick_to_ms(tick));
        let length = clock(song.duration_ms());
        let transport = t(if playing {
            "sync-playing"
        } else {
            "sync-paused"
        });
        let mut status = words
            .msg_with(
                "sync-status",
                &[
                    ("transport", transport.as_str().into()),
                    ("position", position.as_str().into()),
                    ("length", length.as_str().into()),
                    (
                        "tempo",
                        i64::from((tempo_ratio * 100.0).round() as i32).into(),
                    ),
                    ("tapped", (session.stamped as i64).into()),
                    ("total", (session.syllables.len() as i64).into()),
                ],
            )
            .into_owned();
        if session.dirty {
            status = words
                .msg_with("sync-status-unsaved", &[("status", status.as_str().into())])
                .into_owned();
        }

        match (session.phase(), &preview) {
            (Phase::Review, Some(shown)) => {
                let frame = Frame {
                    locale,
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
                let help_top = screen.help(
                    &mut canvas,
                    &mut cache,
                    &fonts,
                    &theme,
                    words,
                    &REVIEW_KEYS,
                    show_help,
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
                let time = clock(song.tempo_map.tick_to_ms(selected.tick));
                // The selected word sits on the key list, wherever that ends.
                let line_height = measure_line(&fonts.text, &["Ag"]).height;
                screen.note(
                    &mut canvas,
                    &mut cache,
                    &fonts,
                    &theme,
                    (help_top - line_height * 1.6) / screen.height,
                    &words.msg_with(
                        "sync-selected-at",
                        &[
                            ("line", around.trim().into()),
                            ("time", time.as_str().into()),
                        ],
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
                    words,
                    &TAPPING_KEYS,
                    show_help,
                );
            }
        }

        let mode = t(match session.phase() {
            Phase::Tapping => "sync-mode-tapping",
            Phase::Review => "sync-mode-review",
        });
        screen.status(
            &mut canvas,
            &mut cache,
            &fonts,
            &theme,
            session.phase(),
            &mode,
            &status,
        );
        let vocal = match melody {
            Some(melody) => {
                let channel = melody.label(words);
                let key = if silenced {
                    "sync-vocal-label-silenced"
                } else {
                    "sync-vocal-label"
                };
                words
                    .msg_with(key, &[("channel", channel.as_str().into())])
                    .into_owned()
            }
            None => t("sync-vocal-label-none"),
        };
        screen.melody(&mut canvas, &mut cache, &fonts, &theme, &vocal, note_lit);
        if let Some((text, since)) = &message {
            if now.duration_since(*since) < MESSAGE_FOR {
                screen.note(&mut canvas, &mut cache, &fonts, &theme, 0.12, text);
            } else {
                message = None;
            }
        }
        canvas.present();

        // A word the open faces cannot draw. The fonts are opened again on a CJK face, after the
        // frame is on screen, as the machine does it. That frame shows boxes and the next does not.
        if !with_cjk && cache.saw_cjk() {
            with_cjk = true;
            let height = screen.height as u32;
            match open_fonts(&ttf, settings, &bundled, with_cjk, &theme, height) {
                Ok(rebuilt) => {
                    if !rebuilt.has_cjk() {
                        tracing::warn!(
                            "the words want CJK glyphs and no CJK font was found; \
                             set display.font_cjk in settings.json"
                        );
                    }
                    fonts = rebuilt;
                    cache.clear();
                }
                Err(error) => {
                    tracing::warn!(%error, "could not open the fonts again with a CJK face");
                }
            }
        }
    }

    sound.send(Command::Stop);
    Ok(())
}

/// A position as minutes, seconds and tenths.
fn clock(ms: u32) -> String {
    format!("{}:{:02}.{}", ms / 60_000, ms / 1_000 % 60, ms / 100 % 10)
}

/// One row of the key list: the message for what its keys act on, then each key cap with the
/// message for what it does.
type HelpRow = (&'static str, &'static [(&'static str, &'static str)]);

/// The keys while syllables are still waiting for a tick.
///
/// The keys pressed on every word come first, the transport second and the rare ones last. The
/// longest entry of a row closes it, so it widens no column a shorter entry shares.
const TAPPING_KEYS: [HelpRow; 3] = [
    (
        "sync-row-tap",
        &[
            ("Space", "sync-key-next-word"),
            ("E", "sync-key-end-word"),
            ("Backspace", "sync-key-undo"),
            ("Up", "sync-key-tap-line-again"),
        ],
    ),
    (
        "sync-row-song",
        &[
            ("Enter", "sync-key-play-pause"),
            ("Left Right", "sync-key-seek"),
            ("-  +", "sync-key-tempo"),
            ("V", "sync-key-silence"),
            ("M", "sync-key-vocal-next"),
        ],
    ),
    (
        "sync-row-file",
        &[
            ("Ctrl+S", "sync-key-save"),
            ("Esc", "sync-key-leave"),
            ("H", "sync-key-hide-keys"),
            ("R", "sync-key-review-so-far"),
        ],
    ),
];

/// The keys once every syllable has a tick.
const REVIEW_KEYS: [HelpRow; 4] = [
    (
        "sync-row-word",
        &[
            ("Up Down", "sync-key-select"),
            ("C", "sync-key-select-sung"),
            ("E", "sync-key-end-here"),
            ("Backspace", "sync-key-undo"),
            ("Left Right", "sync-key-move"),
        ],
    ),
    (
        "sync-row-song",
        &[
            ("Space", "sync-key-play-pause"),
            ("Enter", "sync-key-play-line"),
            ("-  +", "sync-key-tempo"),
        ],
    ),
    // The keys that work together sit together: one names the channel and the others use it.
    (
        "sync-row-notes",
        &[
            ("V", "sync-key-silence"),
            ("M", "sync-key-vocal-next"),
            ("N", "sync-key-snap"),
        ],
    ),
    (
        "sync-row-file",
        &[
            ("Ctrl+S", "sync-key-save"),
            ("Esc", "sync-key-leave"),
            ("H", "sync-key-hide-keys"),
            ("R", "sync-key-back-to-tapping"),
            ("Ctrl+Shift+Backspace", "sync-key-clear"),
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
    #[allow(clippy::too_many_arguments)]
    fn help(
        &self,
        canvas: &mut Screenful,
        cache: &mut Cache,
        fonts: &Fonts,
        theme: &Theme,
        words: &Catalog,
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
            let text = format!("H  {}", words.msg("sync-key-show-keys"));
            draw_text(canvas, cache, font, &text, (self.width * 0.98, top), &hint);
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

        // Every row worded once, so a width is measured on the words that are drawn.
        let worded: Vec<(String, Vec<(&str, String)>)> = rows
            .iter()
            .map(|(name, keys)| {
                (
                    words.msg(name).into_owned(),
                    keys.iter()
                        .map(|(cap, action)| (*cap, words.msg(action).into_owned()))
                        .collect(),
                )
            })
            .collect();

        // The same columns in every row, each as wide as its widest entry. A window too narrow for
        // that lets each row keep its own widths, which is uneven and still readable.
        let entry = |(cap, action): &(&str, String)| wide(cap) + en + wide(action);
        let label = worded
            .iter()
            .map(|(name, _)| wide(name))
            .fold(0.0, f32::max)
            + en * 2.0;
        let columns = worded.iter().map(|(_, keys)| keys.len()).max().unwrap_or(0);
        let widths: Vec<f32> = (0..columns)
            .map(|column| {
                worded
                    .iter()
                    .filter_map(|(_, keys)| keys.get(column))
                    .map(entry)
                    .fold(0.0, f32::max)
            })
            .collect();
        let gap = en * 3.0;
        let aligned =
            left + label + widths.iter().sum::<f32>() + gap * columns as f32 <= self.width * 0.98;

        for (row, (name, keys)) in worded.iter().enumerate() {
            let y = top + pitch * row as f32;
            draw_text(canvas, cache, font, name, (left, y), &dim);
            let mut x = left + label;
            for (column, pair) in keys.iter().enumerate() {
                let (cap, action) = pair;
                let bright = TextStyle::outlined(theme.accent, theme, Align::Left);
                let cap_width = draw_text(canvas, cache, font, cap, (x, y), &bright);
                draw_text(canvas, cache, font, action, (x + cap_width + en, y), &dim);
                x += gap + if aligned { widths[column] } else { entry(pair) };
            }
        }
        top
    }

    /// The transport line at the head of the window.
    #[allow(clippy::too_many_arguments)]
    fn status(
        &self,
        canvas: &mut Screenful,
        cache: &mut Cache,
        fonts: &Fonts,
        theme: &Theme,
        phase: Phase,
        mode: &str,
        text: &str,
    ) {
        // The mode in a color of its own, one for each, so a change of mode is seen and not read.
        let color = match phase {
            Phase::Tapping => theme.accent,
            Phase::Review => theme.accent_alt,
        };
        let at = (self.width * 0.02, self.height * 0.02);
        let font = &fonts.small;
        let mode_width = draw_text(
            canvas,
            cache,
            font,
            mode,
            at,
            &TextStyle::outlined(color, theme, Align::Left),
        );
        let gap = measure_line(font, &["n"]).width * 3.0;
        draw_text(
            canvas,
            cache,
            font,
            text,
            (at.0 + mode_width + gap, at.1),
            &TextStyle::outlined(theme.text, theme, Align::Left),
        );
    }

    /// The chosen vocal line, with a mark that lights on each of its notes.
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
    fn review_can_be_asked_for_once_a_word_is_tapped_and_ends_with_the_last_undo() {
        let mut s = session("la la la");
        s.reviewing = true;
        assert_eq!(s.phase(), Phase::Tapping);
        s.tap(100);
        assert_eq!(s.phase(), Phase::Review);
        assert_eq!(ticks(&s), [100]);
        s.undo();
        assert_eq!(s.phase(), Phase::Tapping);
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
    fn the_syllable_being_sung_is_the_last_one_that_has_started() {
        let mut s = session("a b c d");
        for tick in [100, 200, 300] {
            s.tap(tick);
        }
        for (tick, selected) in [(0, 0), (100, 0), (250, 1), (300, 2), (9_000, 2)] {
            s.select_at(tick);
            assert_eq!(s.selected, selected, "at tick {tick}");
        }
    }

    #[test]
    fn clearing_takes_back_every_tap_and_end_and_opens_tapping() {
        let mut s = session("a b");
        assert!(!s.clear());
        s.tap(100);
        assert!(s.end_at(0, 150));
        s.tap(200);
        s.selected = 1;
        assert_eq!(s.phase(), Phase::Review);
        assert!(s.clear());
        assert_eq!(s.phase(), Phase::Tapping);
        assert_eq!((s.stamped, s.selected), (0, 0));
        assert!(
            s.syllables
                .iter()
                .all(|syllable| syllable.end_tick.is_none())
        );
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

    /// Thirty taps a second apart, each 40 ms late.
    fn taps() -> Vec<u32> {
        (0..30).map(|i| 10_000 + i * 1_000 + 40).collect()
    }

    #[test]
    fn the_channel_the_taps_land_on_is_the_vocal_line() {
        let sung: Vec<u32> = (0..30).map(|i| 10_000 + i * 1_000).collect();
        // A bass on the off-beats, which no tap is near.
        let bass: Vec<u32> = (0..30).map(|i| 10_500 + i * 1_000).collect();
        assert_eq!(vocal_line_under(&taps(), &[bass, sung]), Some(1));
    }

    #[test]
    fn a_busy_part_is_near_every_tap_and_is_not_chosen_for_it() {
        // A note every 100 ms is within reach of any tap at all.
        let busy: Vec<u32> = (0..400).map(|i| 5_000 + i * 100).collect();
        assert_eq!(vocal_line_under(&taps(), std::slice::from_ref(&busy)), None);
        // Beside it, the line the taps were made to is still found.
        let sung: Vec<u32> = (0..30).map(|i| 10_000 + i * 1_000).collect();
        assert_eq!(vocal_line_under(&taps(), &[busy, sung]), Some(1));
    }

    #[test]
    fn a_part_with_the_tune_inside_more_notes_loses_to_the_tune_alone() {
        let sung: Vec<u32> = (0..30).map(|i| 10_000 + i * 1_000).collect();
        // The same notes with a second voice between them: every tap lands, at twice the density.
        let mut thick = sung.clone();
        thick.extend(sung.iter().map(|t| t + 400));
        thick.sort_unstable();
        // The tune alone, resting for three words, so fewer taps land on it.
        let alone = sung[..27].to_vec();
        assert_eq!(vocal_line_under(&taps(), &[thick, alone]), Some(1));
    }

    #[test]
    fn of_two_channels_the_taps_follow_the_one_further_above_chance_is_chosen() {
        let sung: Vec<u32> = (0..30).map(|i| 10_000 + i * 1_000).collect();
        // A doubling that rests for the last five words.
        let doubled = sung[..25].to_vec();
        assert_eq!(vocal_line_under(&taps(), &[doubled, sung]), Some(1));
    }

    #[test]
    fn a_few_taps_choose_nothing() {
        let sung: Vec<u32> = (0..30).map(|i| 10_000 + i * 1_000).collect();
        assert_eq!(vocal_line_under(&taps()[..5], &[sung]), None);
    }

    /// A file's words come back as the editor wrote them: ticks, breaks, spacing and ends.
    #[test]
    fn a_saved_files_words_are_read_back_as_they_were_tapped() {
        let mut typed = split_words("Twin-kle lit-tle star\nhow I won-der\n\nUp a-bove");
        for (index, syllable) in typed.iter_mut().enumerate() {
            syllable.tick = 480 + 240 * u32::try_from(index).unwrap();
        }
        // "star" closes its line and is ended before the next line starts.
        typed[4].end_tick = Some(typed[4].tick + 100);
        let saved = write_soft_karaoke(
            &km_song::testing::instrumental(),
            &KarWords {
                title: "A Song".to_owned(),
                syllables: typed.clone(),
                ..KarWords::default()
            },
        )
        .unwrap();
        let song = Song::parse(&saved, &ParseOptions::default()).unwrap();
        let held = syllables_of(&song.lyrics, u32::from(song.ticks_per_quarter));
        assert_eq!(held, typed);
    }

    #[test]
    fn typed_words_take_a_files_ticks_for_the_words_it_already_times() {
        let typed = split_words("one two\nthree four");
        let mut held = split_words("One two");
        held[0].tick = 100;
        held[1].tick = 200;
        held[1].end_tick = Some(250);

        let (words, tapped) = continue_from(typed.clone(), &held).unwrap();
        assert_eq!(tapped, 2);
        assert_eq!((words[0].tick, words[1].tick), (100, 200));
        assert_eq!(words[1].end_tick, Some(250));
        // The typed text is what is kept, and the untapped words are untouched.
        assert_eq!(words[0].text, "one");
        assert_eq!(words[2..], typed[2..]);

        let session = Session::new(words).with_tapped(tapped);
        assert_eq!(session.phase(), Phase::Tapping);
        assert_eq!(session.stamped, 2);
    }

    #[test]
    fn a_file_whose_words_are_not_the_start_of_the_text_is_refused_by_name() {
        let typed = split_words("one two three");
        let mut held = split_words("one too");
        held[1].tick = 200;
        let fault = continue_from(typed.clone(), &held).unwrap_err();
        assert!(fault.contains("word 2") && fault.contains("too"), "{fault}");
        let longer = split_words("one two three four");
        assert!(continue_from(typed, &longer).is_err());
    }

    /// Both locales hold the same keys, and every key the source names is one of them.
    #[test]
    fn the_catalogs_agree_and_hold_every_key_the_editor_asks_for() {
        if let Err(fault) = km_locale::check_catalogs(messages) {
            panic!("{fault}");
        }
        let english = messages(Locale::English);
        let source = include_str!("sync.rs");
        let mut asked = 0;
        for piece in source.split('"').filter(|piece| piece.starts_with("sync-")) {
            // The module name in `Catalogs::new` and a prefix in this test are not keys.
            if piece == "sync-" {
                continue;
            }
            asked += 1;
            assert!(english.keys().contains(piece), "no message for {piece}");
        }
        assert!(asked > 40, "only {asked} keys found in the source");
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
