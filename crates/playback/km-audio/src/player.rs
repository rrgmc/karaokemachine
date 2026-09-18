//! The audio-thread player: sequencer plus synthesizer, filling audio buffers.
//!
//! Everything in here runs on the real-time thread, so it obeys the hard rules from
//! `docs/ARCHITECTURE.md`: no allocation, no file I/O, no locking, no parsing. Songs arrive
//! pre-parsed as `Arc<Song>` and scratch buffers are sized once at construction.
//!
//! The awkward part it exists to solve: the audio device asks for an arbitrary number of frames,
//! while the synthesizer renders a fixed block (64 frames by default). Leftovers from a partly
//! consumed block are carried between callbacks rather than rounding the request up or down, which
//! would drift.
//!
//! # Two kinds of song, one output stream
//!
//! A song is a MIDI file or a video file, and the difference lives in [`Program`] — the *loaded
//! thing* — rather than in the source the player was built around. The synthesizer stays exactly
//! where it was, because it is constructed when the device opens and nothing knows yet what will be
//! played.
//!
//! This is what lets a mixed queue cost nothing. `OutputStream::open` monomorphises this player once
//! and fixes the source type for the life of the stream, so making the *source* the enum would have
//! meant reopening the device whenever the kind changed — reintroducing the up-to-a-second stall on
//! Bluetooth that the `Holding the audio device` decision was written to characterise. Making the
//! program the enum leaves every existing MIDI path untouched and needs no reopen at all.

use std::sync::Arc;

use km_fixes::ChannelFixes;
use km_queue::Transport;
use km_song::Song;

// `MidiSink` is not imported: its methods reach us through `AudioSource`'s supertrait bound.
use crate::sequencer::{PlaybackSettings, Sequencer};
use crate::source::AudioSource;
use crate::track::TrackPlayer;

/// The most [`Player::set_song_gain`] will accept: +12 dB.
///
/// It matches `km_loudness::MAX_MIDI_GAIN`, which is where the figure is argued. Spelled again here
/// rather than taken as a dependency, because the audio path deliberately knows nothing about
/// loudness measurement and this is the boundary that would otherwise carry it in.
pub const MAX_SONG_GAIN: f32 = 3.98;

/// Something the player has to tell the control thread about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEvent {
    /// The song reached its end on its own.
    SongEnded,
}

/// Something the player has finished with, on its way somewhere else to be dropped.
///
/// Freeing a parsed song means freeing a few megabytes of event vector, which is exactly the kind of
/// unbounded work a real-time callback must not do; a video feed's ring is smaller but is no more
/// welcome there. Both are handed back to the control thread instead. See [`Player::retire`].
#[derive(Debug)]
pub enum Retired {
    /// A parsed MIDI song.
    Song(Arc<Song>),
    /// A video song's audio feed.
    Track(Box<TrackPlayer>),
}

/// What is loaded: the two kinds of song a machine can play.
///
/// Not public. Callers load a MIDI song with [`Player::load`] and a video song's audio with
/// [`Player::load_track`], and everything else about the transport is the same either way.
enum Program {
    /// A MIDI file, driven into the synthesizer by the sequencer.
    Midi(Sequencer),
    /// Decoded audio arriving from elsewhere — the audio half of a video song.
    Track(TrackPlayer),
}

/// Drives a song into an audio buffer.
pub struct Player<S: AudioSource> {
    source: S,
    program: Option<Program>,
    transport: Transport,
    /// Applied to the rendered mix. The owner's level, and the only one anybody sets:
    /// `rustysynth`'s own master volume is left at its hardcoded 0.5, and raising it would
    /// invalidate every loudness figure the bundled bank was chosen against
    /// (`docs/architecture/audio.md`) and clip at `music_volume: 1.0`.
    music_volume: f32,
    /// Applied on top of [`Self::music_volume`], to bring one song to the reference level.
    ///
    /// **A second gain rather than a busier `music_volume`, and the separation is the point.**
    /// `music_volume` is reported by `GET /api/v1/settings` and drawn as a slider on every remote;
    /// folding a per-song correction into it would make that slider jump between songs and would
    /// write a measurement into a key an owner sets by hand.
    ///
    /// `1.0` for a song nothing has a level for. A media song is only ever brought down; a MIDI song
    /// moves either way, up to [`MAX_SONG_GAIN`].
    song_gain: f32,
    /// Settings to apply to the next song loaded, so a transpose set while idle is not lost.
    pending_settings: PlaybackSettings,

    left: Vec<f32>,
    right: Vec<f32>,
    /// Rendered frames not yet handed to the device, interleaved.
    spill: Vec<f32>,
    /// How much of `spill` has been consumed.
    spill_read: usize,
    /// Duration of one rendered block in microseconds.
    block_us: f64,
    ended_reported: bool,
}

impl<S: AudioSource> Player<S> {
    /// Builds a player around an audio source.
    pub fn new(source: S) -> Self {
        let block_size = source.block_size().max(1);
        let block_us = block_size as f64 * 1_000_000.0 / f64::from(source.sample_rate().max(1));
        Self {
            source,
            program: None,
            transport: Transport::Idle,
            music_volume: 1.0,
            song_gain: 1.0,
            pending_settings: PlaybackSettings::default(),
            left: vec![0.0; block_size],
            right: vec![0.0; block_size],
            // Two channels per frame.
            spill: vec![0.0; block_size * 2],
            spill_read: block_size * 2,
            block_us,
            ended_reported: false,
        }
    }

    /// Gives the audio source back, for a caller that wants to ask it what a render left behind.
    ///
    /// A synthesizer holds channel state a rendered buffer cannot show — which patch, which pitch
    /// bend range — and that state is the answer to whether a file's setup messages were acted on.
    pub fn into_source(self) -> S {
        self.source
    }

    /// The transport state.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Position in ticks, or 0 with no song loaded.
    ///
    /// Always 0 for a video song: ticks are a MIDI file's own unit of time, and a video has no
    /// timeline to express in them. The lyric wipe rides on this, and a video carries its words as
    /// pixels, so there is nothing for it to drive.
    pub fn position_ticks(&self) -> u32 {
        match &self.program {
            Some(Program::Midi(sequencer)) => sequencer.position_ticks(),
            Some(Program::Track(_)) | None => 0,
        }
    }

    /// Position in milliseconds, or 0 with no song loaded.
    pub fn position_ms(&self) -> u32 {
        match &self.program {
            Some(Program::Midi(sequencer)) => sequencer.position_ms(),
            Some(Program::Track(track)) => track.position_ms(),
            None => 0,
        }
    }

    /// How long the loaded song has been silent for want of samples, or 0 with no song loaded.
    ///
    /// Always 0 for a MIDI song, and not because the counter was left out: a sequencer renders from
    /// a parsed file that is already in memory, so there is nothing for it to be starved *by*. Only
    /// a video song reads from a decoder that can fall behind.
    pub fn starved_ms(&self) -> u64 {
        match &self.program {
            Some(Program::Track(track)) => track.starved_ms(),
            Some(Program::Midi(_)) | None => 0,
        }
    }

    /// The MIDI song currently loaded, if the loaded song is a MIDI one.
    pub fn song(&self) -> Option<&Arc<Song>> {
        match &self.program {
            Some(Program::Midi(sequencer)) => Some(sequencer.song()),
            Some(Program::Track(_)) | None => None,
        }
    }

    /// The settings that are or will be in force.
    ///
    /// A video song has none of them, so what comes back is what the *next* MIDI song will get.
    pub fn settings(&self) -> PlaybackSettings {
        match &self.program {
            Some(Program::Midi(sequencer)) => sequencer.settings(),
            Some(Program::Track(_)) | None => self.pending_settings,
        }
    }

    /// Whether the loaded song is a video's audio rather than a MIDI file.
    pub fn is_track(&self) -> bool {
        matches!(self.program, Some(Program::Track(_)))
    }

    /// Takes what is loaded out of the player, so it can be dropped off the audio thread.
    ///
    /// Leaves the player with nothing loaded but does not touch the transport — the caller is
    /// about to load something else or to [`Player::unload`], and both settle it.
    pub fn retire(&mut self) -> Option<Retired> {
        match self.program.take() {
            // The `Arc` is cloned out and the sequencer itself dropped here, which is what already
            // happened before a second kind of song existed: a sequencer is small, and it is the
            // song behind it that must not be freed on this thread.
            Some(Program::Midi(sequencer)) => Some(Retired::Song(Arc::clone(sequencer.song()))),
            Some(Program::Track(track)) => Some(Retired::Track(Box::new(track))),
            None => None,
        }
    }

    /// Loads a song, replacing whatever was playing, and leaves it stopped at the start.
    ///
    /// Settings carry over, except the melody channel and the fixes, which belong to the song.
    pub fn load(&mut self, song: Arc<Song>, melody_channel: Option<u8>, fixes: ChannelFixes) {
        self.source.reset();
        self.discard_spill();
        let mut settings = self.settings();
        settings.melody_channel = melody_channel;
        self.pending_settings = settings;
        self.program = Some(Program::Midi(Sequencer::new(song, settings, fixes)));
        self.transport = Transport::Stopped;
        self.ended_reported = false;
    }

    /// Loads a decoded audio track — the audio half of a video song — and leaves it stopped.
    ///
    /// The synthesizer is silenced rather than unloaded: it stays for the next MIDI song, which is
    /// the whole point of keeping the source and varying the program.
    pub fn load_track(&mut self, track: TrackPlayer) {
        self.source.reset();
        self.discard_spill();
        self.program = Some(Program::Track(track));
        self.transport = Transport::Stopped;
        self.ended_reported = false;
    }

    /// Unloads the current song.
    pub fn unload(&mut self) {
        self.source.reset();
        self.discard_spill();
        self.program = None;
        self.transport = Transport::Idle;
    }

    /// Starts or resumes playback. Does nothing with no song loaded.
    pub fn play(&mut self) {
        if self.program.is_some() {
            self.transport = Transport::Playing;
        }
    }

    /// Stops advancing and silences held notes.
    ///
    /// Notes are cut rather than sustained, as on a commercial machine; the reverb tail is still
    /// rendered, so pausing does not click.
    pub fn pause(&mut self) {
        if self.transport == Transport::Playing {
            self.transport = Transport::Paused;
            if let Some(Program::Midi(sequencer)) = &mut self.program {
                sequencer.silence(&mut self.source);
            }
        }
    }

    /// Returns to the start and stops.
    pub fn stop(&mut self) {
        match &mut self.program {
            Some(Program::Midi(sequencer)) => sequencer.restart(&mut self.source),
            Some(Program::Track(track)) => track.restart(),
            None => {}
        }
        if self.program.is_some() {
            self.transport = Transport::Stopped;
            self.ended_reported = false;
        }
        self.discard_spill();
    }

    /// Returns to the start and keeps playing.
    pub fn restart(&mut self) {
        match &mut self.program {
            Some(Program::Midi(sequencer)) => sequencer.restart(&mut self.source),
            Some(Program::Track(track)) => track.restart(),
            None => {}
        }
        if self.program.is_some() {
            self.transport = Transport::Playing;
            self.ended_reported = false;
        }
    }

    /// Jumps to a position in milliseconds.
    ///
    /// Exact for a MIDI song. For a video, the decoder can only start again at a keyframe, so this
    /// lands within one keyframe interval of where it was asked for — which is why the transcode
    /// profile forces a keyframe every second.
    pub fn seek_ms(&mut self, ms: u32) {
        match &mut self.program {
            Some(Program::Midi(sequencer)) => sequencer.seek_ms(ms, &mut self.source),
            Some(Program::Track(track)) => track.seek_ms(ms),
            None => {}
        }
        if self.program.is_some() {
            self.ended_reported = false;
        }
        self.discard_spill();
    }

    /// Sets the transposition in semitones. This is the tone adjustment control.
    ///
    /// Inert while a video song is loaded — there is no key to shift — but still recorded, so it
    /// takes effect on the next MIDI song exactly as a transpose set while idle does.
    pub fn set_transpose(&mut self, semitones: i8) {
        self.pending_settings.transpose = semitones;
        self.pending_settings = self.pending_settings.clamped();
        if let Some(Program::Midi(sequencer)) = &mut self.program {
            sequencer.set_transpose(semitones);
        }
    }

    /// Sets playback speed as a multiple of the written tempo.
    ///
    /// Inert while a video song is loaded, for the reason given on [`Player::set_transpose`].
    pub fn set_tempo_ratio(&mut self, ratio: f32) {
        self.pending_settings.tempo_ratio = ratio;
        self.pending_settings = self.pending_settings.clamped();
        if let Some(Program::Midi(sequencer)) = &mut self.program {
            sequencer.set_tempo_ratio(ratio);
        }
    }

    /// Turns the guide melody on or off.
    ///
    /// Inert while a video song is loaded, for the reason given on [`Player::set_transpose`].
    pub fn set_melody_enabled(&mut self, enabled: bool) {
        self.pending_settings.melody_enabled = enabled;
        if let Some(Program::Midi(sequencer)) = &mut self.program {
            sequencer.set_melody_enabled(enabled, &mut self.source);
        }
    }

    /// Sets the music volume, 0.0 to 1.0.
    pub fn set_music_volume(&mut self, volume: f32) {
        self.music_volume = volume.clamp(0.0, 1.0);
    }

    /// The music volume.
    pub fn music_volume(&self) -> f32 {
        self.music_volume
    }

    /// Sets the per-song levelling gain, 0.0 to [`MAX_SONG_GAIN`].
    ///
    /// **The ceiling is above `1.0` because a MIDI song may be quiet as well as loud**, and a song
    /// that plays too low is the fault this exists for. It is still a clamp rather than a trust: a
    /// bad measurement must not be able to send an arbitrary multiplier into the callback, and the
    /// limiter downstream flattens an overshoot rather than reporting it.
    ///
    /// A media song never reaches past `1.0` and is held there by `km_loudness::gain_for` instead,
    /// because a finished master has no headroom to give. See `docs/decisions/audio.md`.
    pub fn set_song_gain(&mut self, gain: f32) {
        self.song_gain = gain.clamp(0.0, MAX_SONG_GAIN);
    }

    /// The per-song levelling gain.
    pub fn song_gain(&self) -> f32 {
        self.song_gain
    }

    /// Fills an interleaved output buffer, returning any event that occurred.
    ///
    /// `channels` is the device's channel count. Mono is a downmix; anything above two puts the
    /// stereo pair in the first two and **silences the rest**.
    ///
    /// This paragraph said the opposite until 2026-09-07 — "repeats the stereo pair, which is wrong
    /// for surround but never silent" — and the code below has always zeroed. The code is the one
    /// that was right, so the sentence went rather than the behaviour. Copying a correlated pair
    /// into centre, LFE and the surrounds is not a safe fallback: it smears a centre image that
    /// belongs in front, and sends full-range music to an output a receiver expects to be low
    /// frequencies only. Front left and right with silence behind is what every other player does,
    /// and what a receiver's own upmix expects to be handed.
    pub fn fill(&mut self, out: &mut [f32], channels: usize) -> Option<PlayerEvent> {
        let channels = channels.max(1);
        let mut event = None;

        if self.program.is_none() {
            out.fill(0.0);
            return None;
        }

        let mut written = 0;
        while written < out.len() {
            if self.spill_read >= self.spill.len()
                && let Some(new_event) = self.render_block()
            {
                event = event.or(Some(new_event));
            }

            // One frame at a time from the spill buffer, fanned out to the device's channels.
            let available_frames = (self.spill.len() - self.spill_read) / 2;
            let wanted_frames = (out.len() - written) / channels;
            let frames = available_frames.min(wanted_frames);
            if frames == 0 {
                // Not a whole frame's worth of room left; pad the remainder rather than looping.
                out[written..].fill(0.0);
                break;
            }

            // One multiplier for the two gains, computed per block rather than per sample: the
            // owner's level and this song's levelling are independent settings and one product.
            let gain = self.music_volume * self.song_gain;
            for _ in 0..frames {
                let l = self.spill[self.spill_read] * gain;
                let r = self.spill[self.spill_read + 1] * gain;
                self.spill_read += 2;
                match channels {
                    1 => {
                        out[written] = (l + r) * 0.5;
                        written += 1;
                    }
                    _ => {
                        out[written] = l;
                        out[written + 1] = r;
                        for slot in &mut out[written + 2..written + channels] {
                            *slot = 0.0;
                        }
                        written += channels;
                    }
                }
            }
        }
        event
    }

    /// Advances time by one block and renders it into the spill buffer.
    fn render_block(&mut self) -> Option<PlayerEvent> {
        let mut event = None;
        let advancing = self.transport.is_advancing();
        let mut ended = false;

        match &mut self.program {
            Some(Program::Midi(sequencer)) => {
                if advancing {
                    sequencer.advance(self.block_us, &mut self.source);
                    ended = sequencer.is_finished();
                }
                // Rendered even when paused, so a held note's tail decays instead of cutting to
                // silence.
                self.source.render(&mut self.left, &mut self.right);
            }
            Some(Program::Track(track)) => {
                // The track fills the buffers itself, and the synthesizer is left alone: a video
                // song has no notes to render a tail for. `advancing` is passed rather than checked
                // here because the track still has to service a pending seek while paused.
                track.render(&mut self.left, &mut self.right, advancing);
                ended = track.is_finished();
            }
            None => {
                self.left.fill(0.0);
                self.right.fill(0.0);
            }
        }

        if ended && !self.ended_reported {
            self.ended_reported = true;
            // A backstop for anything still sounding, whatever put it there -- a file whose
            // note-offs the parser could not reach, or one routed here without the repair. Nothing
            // else would ever lift it: the sequencer early-returns for ever once it is finished, and
            // the machine's watchdog is a poll interval away and absent altogether from a bare
            // `Player`.
            //
            // `all_notes_off` and not `reset`, deliberately: it kills the voices and leaves the
            // reverb and chorus tails alone, so the decay the line below protects still happens. It
            // is also immediate rather than a release, which is what cuts through a hold pedal left
            // down -- a CC123 would be deferred by one and silence nothing.
            self.source.all_notes_off();
            // Left in Playing so a reverb tail finishes; the control thread decides what happens
            // next.
            event = Some(PlayerEvent::SongEnded);
        }

        for (frame, (l, r)) in self.left.iter().zip(self.right.iter()).enumerate() {
            self.spill[frame * 2] = *l;
            self.spill[frame * 2 + 1] = *r;
        }
        self.spill_read = 0;
        event
    }

    /// Drops any rendered-but-unplayed audio, so a seek or load does not emit stale sound.
    fn discard_spill(&mut self) {
        self.spill_read = self.spill.len();
    }
}

#[cfg(test)]
mod tests {
    use km_song::{EventKind, ParseOptions, testing};

    use super::*;
    use crate::source::TestToneSource;

    const SAMPLE_RATE: u32 = 44_100;

    fn song(bytes: &[u8]) -> Arc<Song> {
        Arc::new(Song::parse(bytes, &ParseOptions::default()).expect("fixture parses"))
    }

    fn player() -> Player<TestToneSource> {
        Player::new(TestToneSource::new(SAMPLE_RATE))
    }

    /// Renders `seconds` of stereo audio, returning it interleaved along with any events seen.
    fn render(player: &mut Player<TestToneSource>, seconds: f32) -> (Vec<f32>, Vec<PlayerEvent>) {
        let frames = (seconds * SAMPLE_RATE as f32) as usize;
        let mut out = vec![0.0f32; frames * 2];
        let mut events = Vec::new();
        // 512-frame callbacks, a realistic device buffer that is not a multiple of the 64-frame
        // render block boundary in a helpful way.
        for chunk in out.chunks_mut(512 * 2) {
            if let Some(event) = player.fill(chunk, 2) {
                events.push(event);
            }
        }
        (out, events)
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
    }

    #[test]
    fn an_idle_player_outputs_silence() {
        let mut player = player();
        let (audio, events) = render(&mut player, 0.1);
        assert_eq!(peak(&audio), 0.0);
        assert!(events.is_empty());
        assert_eq!(player.transport(), Transport::Idle);
    }

    #[test]
    fn loading_resets_the_synthesizer_rather_than_only_silencing_it() {
        // `all_notes_off` kills voices and leaves every channel's volume, pan, hold pedal and patch
        // where the previous song left them -- so a song ending on a CC7 fade to zero made the next
        // one silent on those channels.
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        assert_eq!(player.source.resets(), 1);
        player.unload();
        assert_eq!(player.source.resets(), 2);
    }

    #[test]
    fn a_loaded_song_is_stopped_until_told_to_play() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        assert_eq!(player.transport(), Transport::Stopped);
        let (audio, _) = render(&mut player, 0.2);
        assert_eq!(peak(&audio), 0.0, "a stopped song must be silent");
    }

    #[test]
    fn playing_produces_audio_and_advances_the_position() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        let (audio, _) = render(&mut player, 0.5);
        assert!(peak(&audio) > 0.0, "playback should be audible");
        assert!(player.position_ms() > 400, "position should advance");
    }

    #[test]
    fn the_song_ended_event_fires_once_when_the_song_runs_out() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        let (_, events) = render(&mut player, 8.0);
        assert_eq!(
            events
                .iter()
                .filter(|e| **e == PlayerEvent::SongEnded)
                .count(),
            1,
            "exactly one end-of-song event"
        );
    }

    /// A song that reaches the player still holding a note is silenced when it ends.
    ///
    /// The parser repairs a dangling note-on now, so this fixture is deliberately *un*-repaired
    /// first: the point of the backstop is the song that did not come through that path. Without it
    /// the note sounds for the life of the process -- `Sequencer::advance` early-returns for ever
    /// once `finished` is set, and a bare `Player` has no watchdog to call `load` or `unload`.
    #[test]
    fn a_song_that_ends_holding_a_note_falls_silent() {
        let mut song = Song::parse(&testing::unbalanced_note_on(), &ParseOptions::default())
            .expect("fixture parses");
        assert_eq!(song.repaired_notes, 1, "the parser repaired it");
        song.events
            .retain(|e| !matches!(e.kind, EventKind::NoteOff { key: 67, .. }));

        let mut player = player();
        player.load(Arc::new(song), None, ChannelFixes::default());
        player.play();
        let (_, events) = render(&mut player, 4.0);

        assert!(
            events.contains(&PlayerEvent::SongEnded),
            "the song has to have ended for the backstop to run"
        );
        assert_eq!(
            player.source.active_voices(),
            0,
            "nothing may still be sounding after the song ends"
        );
    }

    /// Ending a song kills the voices without resetting the channels.
    ///
    /// `reset` would also mute the reverb and chorus tails, and the tail is the thing the transport
    /// is deliberately left in `Playing` to finish.
    #[test]
    fn ending_a_song_silences_the_voices_but_does_not_reset_the_channels() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        let resets_after_load = player.source.resets();
        player.play();
        render(&mut player, 8.0);
        assert_eq!(
            player.source.resets(),
            resets_after_load,
            "the end of a song is not a reset"
        );
    }

    #[test]
    fn pausing_stops_the_position_and_silences_notes() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        render(&mut player, 0.5);
        let position = player.position_ms();

        player.pause();
        assert_eq!(player.transport(), Transport::Paused);
        let (audio, _) = render(&mut player, 0.5);
        assert_eq!(
            player.position_ms(),
            position,
            "paused time must not advance"
        );

        // Audio already rendered before the pause is still played out -- one block at most. That is
        // deliberate: discarding it would cut mid-waveform and click. What matters is that nothing
        // new is produced, so the tail is silent.
        let tail = &audio[audio.len() / 2..];
        assert_eq!(peak(tail), 0.0, "a paused player must fall silent");
    }

    #[test]
    fn pausing_carries_out_at_most_one_rendered_block_of_audio() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        render(&mut player, 0.5);
        player.pause();

        // 64 frames is one render block; anything much beyond that would mean notes still sounding.
        let mut out = vec![0.0f32; 64 * 2];
        player.fill(&mut out, 2);
        let mut next = vec![0.0f32; 64 * 2];
        player.fill(&mut next, 2);
        assert_eq!(peak(&next), 0.0, "the block after the spill must be silent");
    }

    #[test]
    fn resuming_continues_from_where_it_paused() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        render(&mut player, 0.5);
        player.pause();
        let paused_at = player.position_ms();
        player.play();
        render(&mut player, 0.5);
        assert!(player.position_ms() > paused_at + 400);
    }

    #[test]
    fn stopping_returns_to_the_start() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        render(&mut player, 1.0);
        player.stop();
        assert_eq!(player.transport(), Transport::Stopped);
        assert_eq!(player.position_ms(), 0);
    }

    #[test]
    fn seeking_moves_the_position_without_emitting_stale_audio() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        render(&mut player, 0.5);
        player.seek_ms(3_000);
        assert!((2_990..=3_010).contains(&player.position_ms()));
    }

    #[test]
    fn volume_scales_the_output_and_zero_is_silent() {
        let mut loud = player();
        loud.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        loud.play();
        let (loud_audio, _) = render(&mut loud, 0.5);

        let mut quiet = player();
        quiet.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        quiet.set_music_volume(0.25);
        quiet.play();
        let (quiet_audio, _) = render(&mut quiet, 0.5);

        assert!(peak(&quiet_audio) < peak(&loud_audio));

        let mut silent = player();
        silent.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        silent.set_music_volume(0.0);
        silent.play();
        let (silent_audio, _) = render(&mut silent, 0.5);
        assert_eq!(peak(&silent_audio), 0.0);
    }

    /// The two gains multiply, and neither disturbs the other.
    ///
    /// **The product is what matters, and it is why they are two fields rather than one.** The
    /// owner's level and a song's levelling are set by different things at different times — a
    /// slider on a phone, and a measurement in a package — so folding them together at the point of
    /// setting would mean one of them could not be changed without knowing the other.
    #[test]
    fn the_song_gain_multiplies_the_owners_volume() {
        let render_at = |volume: f32, gain: f32| {
            let mut player = player();
            player.load(
                song(&testing::soft_karaoke()),
                None,
                ChannelFixes::default(),
            );
            player.set_music_volume(volume);
            player.set_song_gain(gain);
            player.play();
            let (audio, _) = render(&mut player, 0.5);
            peak(&audio)
        };

        let full = render_at(1.0, 1.0);
        // Halving either one halves the output, and the two agree to the sample.
        let half_by_volume = render_at(0.5, 1.0);
        let half_by_gain = render_at(1.0, 0.5);
        assert!((half_by_volume - half_by_gain).abs() < 1e-6);
        assert!((half_by_gain - full * 0.5).abs() < 1e-3);

        // ...and together they compound rather than one winning.
        let quarter = render_at(0.5, 0.5);
        assert!((quarter - full * 0.25).abs() < 1e-3);

        // A gain of zero is silence whatever the volume, which is the property the clamp protects.
        assert_eq!(render_at(1.0, 0.0), 0.0);
    }

    /// A boost is allowed up to the cap and no further, and a negative gain is silence.
    ///
    /// The ceiling is a clamp rather than a trust: a MIDI song may be raised because a quiet one has
    /// the headroom, and a bad measurement must still not be able to send an arbitrary multiplier
    /// into the callback, where the limiter would flatten the overshoot without reporting it.
    #[test]
    fn a_song_gain_is_clamped_to_the_cap_at_both_ends() {
        let mut player = player();
        player.set_song_gain(2.0);
        assert_eq!(player.song_gain(), 2.0);
        player.set_song_gain(40.0);
        assert_eq!(player.song_gain(), MAX_SONG_GAIN);
        player.set_song_gain(-1.0);
        assert_eq!(player.song_gain(), 0.0);
    }

    /// A boost reaches the samples, which is the half a clamp test cannot show.
    #[test]
    fn a_boost_raises_the_rendered_mix() {
        let peak_at = |gain: f32| {
            let mut player = player();
            player.load(
                song(&testing::soft_karaoke()),
                None,
                ChannelFixes::default(),
            );
            // Half volume, so doubling the gain has somewhere to go before the limiter would.
            player.set_music_volume(0.5);
            player.set_song_gain(gain);
            player.play();
            let (audio, _) = render(&mut player, 0.5);
            peak(&audio)
        };
        let plain = peak_at(1.0);
        let raised = peak_at(2.0);
        assert!(
            (raised - plain * 2.0).abs() < 1e-3,
            "{raised} should be twice {plain}"
        );
    }

    /// A fresh player is unlevelled, so nothing is attenuated until a song says to be.
    #[test]
    fn a_fresh_player_has_no_song_gain() {
        assert_eq!(player().song_gain(), 1.0);
    }

    #[test]
    fn output_never_clips_on_a_dense_song() {
        let mut player = player();
        player.load(
            song(&testing::high_quality_song()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        let (audio, _) = render(&mut player, 2.0);
        assert!(peak(&audio) <= 1.0, "peak was {}", peak(&audio));
    }

    #[test]
    fn settings_set_before_a_song_loads_are_applied_to_it() {
        let mut player = player();
        player.set_transpose(4);
        player.set_tempo_ratio(1.2);
        player.load(
            song(&testing::soft_karaoke()),
            Some(0),
            ChannelFixes::default(),
        );
        assert_eq!(player.settings().transpose, 4);
        assert_eq!(player.settings().tempo_ratio, 1.2);
        assert_eq!(player.settings().melody_channel, Some(0));
    }

    #[test]
    fn the_melody_channel_comes_from_the_song_not_the_carried_settings() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            Some(3),
            ChannelFixes::default(),
        );
        assert_eq!(player.settings().melody_channel, Some(3));
        // A song with no detected melody clears it rather than inheriting the last one.
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        assert_eq!(player.settings().melody_channel, None);
    }

    #[test]
    fn a_mono_device_gets_a_downmix_rather_than_silence() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        let mut out = vec![0.0f32; 4_410];
        let mut audible = false;
        for _ in 0..10 {
            player.fill(&mut out, 1);
            audible |= peak(&out) > 0.0;
        }
        assert!(audible, "mono output must still produce sound");
    }

    #[test]
    fn an_odd_buffer_length_does_not_desynchronize_the_channels() {
        // Deliberately not a multiple of the render block or the channel count.
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        let mut out = vec![0.0f32; 1_001];
        for _ in 0..40 {
            player.fill(&mut out, 2);
        }
        assert!(player.position_ms() > 0, "playback continued");
    }

    #[test]
    fn loading_a_new_song_replaces_the_old_one_cleanly() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        render(&mut player, 1.0);

        player.load(
            song(&testing::melody_and_accompaniment()),
            Some(0),
            ChannelFixes::default(),
        );
        assert_eq!(player.position_ms(), 0);
        assert_eq!(player.transport(), Transport::Stopped);
    }

    #[test]
    fn unloading_returns_to_idle_silence() {
        let mut player = player();
        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        render(&mut player, 0.5);
        player.unload();
        assert_eq!(player.transport(), Transport::Idle);
        let (audio, _) = render(&mut player, 0.2);
        assert_eq!(peak(&audio), 0.0);
    }

    #[test]
    fn a_muted_melody_is_quieter_than_an_audible_one() {
        let mut audible = player();
        audible.load(
            song(&testing::melody_and_accompaniment()),
            Some(0),
            ChannelFixes::default(),
        );
        audible.set_melody_enabled(true);
        audible.play();
        let (with_melody, _) = render(&mut audible, 1.0);

        let mut muted = player();
        muted.load(
            song(&testing::melody_and_accompaniment()),
            Some(0),
            ChannelFixes::default(),
        );
        muted.set_melody_enabled(false);
        muted.play();
        let (without_melody, _) = render(&mut muted, 1.0);

        let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
        assert!(
            energy(&without_melody) < energy(&with_melody),
            "muting the melody must remove audio"
        );
    }

    /// A track carrying one constant value, loud enough to tell from a synthesized tone.
    fn track(seconds: f32, value: f32) -> TrackPlayer {
        let frames = (seconds * SAMPLE_RATE as f32) as usize;
        let (mut writer, feed) = crate::track::audio_feed(SAMPLE_RATE, frames + 1);
        let samples: Vec<f32> =
            std::iter::repeat_n(value, frames * crate::track::FEED_CHANNELS).collect();
        assert_eq!(
            writer.push(&samples),
            samples.len(),
            "the ring holds it all"
        );
        writer.finish();
        TrackPlayer::new(feed, SAMPLE_RATE)
    }

    #[test]
    fn a_track_plays_its_samples_and_then_reports_the_end() {
        let mut player = player();
        player.load_track(track(0.05, 0.5));
        player.play();

        let (audio, events) = render(&mut player, 0.2);
        assert!(peak(&audio) > 0.4, "the track's samples reach the output");
        assert_eq!(events, vec![PlayerEvent::SongEnded]);
        assert_eq!(
            player.transport(),
            Transport::Playing,
            "the tail is left to finish"
        );
    }

    #[test]
    fn a_video_song_has_no_ticks_and_no_midi_song() {
        let mut player = player();
        player.load_track(track(0.05, 0.5));
        player.play();
        render(&mut player, 0.02);

        assert!(player.is_track());
        assert!(
            player.song().is_none(),
            "there is no parsed MIDI to hand out"
        );
        assert_eq!(player.position_ticks(), 0, "a video has no tick timeline");
        assert!(player.position_ms() > 0, "but it does have a position");
    }

    /// The claim the whole design rests on: one player, one open stream, both kinds of song.
    #[test]
    fn midi_and_video_songs_alternate_in_one_player() {
        let mut player = player();

        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        let (midi_first, _) = render(&mut player, 0.2);
        assert!(peak(&midi_first) > 0.0, "the synthesizer plays");

        player.load_track(track(0.2, 0.5));
        player.play();
        let (video, _) = render(&mut player, 0.1);
        assert!(peak(&video) > 0.4, "the track plays");
        assert!(player.is_track());

        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        player.play();
        let (midi_again, _) = render(&mut player, 0.2);
        assert!(
            peak(&midi_again) > 0.0,
            "the synthesizer is still there afterwards"
        );
        assert!(!player.is_track());
        assert!(player.song().is_some());
    }

    #[test]
    fn transpose_set_during_a_video_song_reaches_the_next_midi_song() {
        let mut player = player();
        player.load_track(track(0.05, 0.5));
        player.set_transpose(4);
        assert_eq!(
            player.settings().transpose,
            4,
            "recorded even with nothing to apply it to"
        );

        player.load(
            song(&testing::soft_karaoke()),
            None,
            ChannelFixes::default(),
        );
        assert_eq!(
            player.settings().transpose,
            4,
            "and applied when a MIDI song arrives"
        );
    }

    #[test]
    fn seeking_a_video_song_moves_its_position() {
        let mut player = player();
        player.load_track(track(0.05, 0.5));
        player.seek_ms(30_000);
        assert_eq!(player.position_ms(), 30_000);
    }
}
