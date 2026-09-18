//! Turning a song into MIDI messages at the right moment.
//!
//! This is the heart of playback and it deliberately knows nothing about audio. It advances by
//! *microseconds of song time* and emits messages into a [`MidiSink`], which makes every interesting
//! behavior — timing, transposition, the guide-melody mute, seeking — testable without a sound
//! card or a SoundFont. The synthesizer is a thin adapter behind that trait.
//!
//! Advancing in microseconds rather than ticks is what makes tempo work correctly. An audio block
//! is a fixed number of samples, never a whole number of ticks, and a song's own tempo map can
//! change the relationship at any point. Converting through [`km_song::TempoMap`] on every block
//! keeps embedded tempo changes and the user's tempo adjustment independent and exact.

use std::sync::Arc;

use km_fixes::ChannelFixes;
use km_fixes::recentre_bend::{BEND_CENTRE, stranded_from};
use km_queue::limits::{MAX_TEMPO_RATIO, MAX_TRANSPOSE, MIN_TEMPO_RATIO};
use km_song::{EventKind, Song};

/// Somewhere MIDI messages can be sent.
///
/// Implemented by the synthesizer for playback and by a recorder for tests.
pub trait MidiSink {
    /// Start a note.
    fn note_on(&mut self, channel: u8, key: u8, velocity: u8);
    /// Stop a note.
    fn note_off(&mut self, channel: u8, key: u8);
    /// Continuous controller change.
    fn control_change(&mut self, channel: u8, controller: u8, value: u8);
    /// Instrument selection.
    fn program_change(&mut self, channel: u8, program: u8);
    /// Pitch bend, as a 14-bit value centered on 8192.
    fn pitch_bend(&mut self, channel: u8, value: u16);
    /// Whole-channel pressure, which behaves like a controller rather than like a note.
    fn channel_aftertouch(&mut self, channel: u8, value: u8);
    /// Silence everything immediately.
    fn all_notes_off(&mut self);
    /// Silence everything *and* return every channel to its General MIDI defaults.
    ///
    /// [`MidiSink::all_notes_off`] kills voices and touches no channel state, so volume, pan,
    /// expression, the hold pedal, the patch and the pitch bend all survive it. That is what a
    /// pause wants and what a *new song* must not have: a file that fades out on CC7 leaves those
    /// channels at 1 or 2 of 127, and the next song is inaudible on them unless it sets CC7 itself.
    fn reset(&mut self);
}

/// The drum channel, where note numbers select instruments rather than pitches.
///
/// Stays here where the two playback limits below it went to [`km_queue::limits`]: this one is a
/// fact about General MIDI that only something emitting MIDI messages has any use for, and nothing
/// outside this crate has ever asked for it.
pub const DRUM_CHANNEL: u8 = 9;

/// How the song should sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackSettings {
    /// Semitones to shift every pitched note by. This is the "tone adjustment" control.
    pub transpose: i8,
    /// Playback speed as a multiple of the written tempo.
    pub tempo_ratio: f32,
    /// Whether the guide melody is audible. Has no effect when the song declares no melody channel.
    pub melody_enabled: bool,
    /// The melody channel as recorded in the package, if detection was confident about one.
    pub melody_channel: Option<u8>,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            transpose: 0,
            tempo_ratio: 1.0,
            // Off by default: a singer wants the backing track, not the tune played over them.
            melody_enabled: false,
            melody_channel: None,
        }
    }
}

impl PlaybackSettings {
    /// Clamps every field into its supported range.
    pub fn clamped(mut self) -> Self {
        self.transpose = self.transpose.clamp(-MAX_TRANSPOSE, MAX_TRANSPOSE);
        self.tempo_ratio = self.tempo_ratio.clamp(MIN_TEMPO_RATIO, MAX_TEMPO_RATIO);
        self
    }
}

/// A note the sequencer has started and not yet stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SoundingNote {
    channel: u8,
    /// The note as written in the file.
    written_key: u8,
    /// The note actually sent, after transposition.
    played_key: u8,
}

/// Where no note can start on a stranded bend, because the channel's bend is at centre.
const NEVER_STRANDED: u32 = u32::MAX;

/// Reset All Controllers, which returns a channel's bend to centre.
const CC_RESET_ALL_CONTROLLERS: u8 = 121;

/// Which pair of selector controllers a data entry belongs to.
///
/// The four selectors share the two data entries between them, and the pair touched most recently
/// owns them. A data entry arriving before either pair has been touched belongs to neither and is
/// discarded, which is the state a reset leaves a channel in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selected {
    Neither,
    Registered,
    NonRegistered,
}

/// The parameters one channel established, and the selector state it left behind.
///
/// **A seek cannot replay a data entry the way it replays a volume.** CC 7 carries its own meaning;
/// CC 6 carries a number whose meaning is whichever parameter the four selectors last chose, so a
/// data entry replayed without its selector lands on the null parameter and is discarded in
/// silence. A file that asks for a twelve-semitone pitch bend range then plays every bend at two,
/// which is a part out of tune rather than a part missing — the whole of it, for the rest of the
/// song.
///
/// So the scan tracks what each data entry *meant* and the replay states the parameter with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Parameters {
    /// The registered selector, MSB then LSB, at the null parameter a reset leaves.
    registered: (u8, u8),
    /// Whether the file ever moved the registered selector.
    ///
    /// Separate from its value, because a file that selects a parameter and then leaves the null one
    /// ends where it started while having changed which pair owns a data entry. Replaying only the
    /// pairs that moved keeps that distinction.
    touched_registered: bool,
    /// The non-registered selector, MSB then LSB.
    non_registered: (u8, u8),
    /// Whether the file ever moved the non-registered selector.
    touched_non_registered: bool,
    /// Which pair owns a data entry now.
    selected: Selected,
    /// RPN 0, pitch bend range in semitones: coarse, then fine.
    bend_range: (Option<u8>, Option<u8>),
    /// RPN 1, channel fine tune.
    fine_tune: (Option<u8>, Option<u8>),
    /// RPN 2, channel coarse tune. Coarse alone, because the parameter has no fine half.
    coarse_tune: Option<u8>,
}

/// The null parameter, which both selectors hold until a file chooses one.
const PARAMETER_NULL: (u8, u8) = (0x7F, 0x7F);

/// Registered parameter selector, MSB and LSB.
const CC_RPN_MSB: u8 = 101;
const CC_RPN_LSB: u8 = 100;
/// Non-registered parameter selector, MSB and LSB.
const CC_NRPN_MSB: u8 = 99;
const CC_NRPN_LSB: u8 = 98;
/// Data entry, coarse and fine.
const CC_DATA_COARSE: u8 = 6;
const CC_DATA_FINE: u8 = 38;

/// The six controllers whose value is meaningless without the others, so the plain replay skips them.
const PARAMETER_CONTROLLERS: [u8; 6] = [
    CC_DATA_COARSE,
    CC_DATA_FINE,
    CC_NRPN_LSB,
    CC_NRPN_MSB,
    CC_RPN_LSB,
    CC_RPN_MSB,
];

/// Roland GS non-registered parameter 18H, drum instrument pitch coarse.
const NRPN_DRUM_PITCH_COARSE: u8 = 0x18;

impl Default for Parameters {
    fn default() -> Self {
        Self {
            registered: PARAMETER_NULL,
            touched_registered: false,
            non_registered: PARAMETER_NULL,
            touched_non_registered: false,
            selected: Selected::Neither,
            bend_range: (None, None),
            fine_tune: (None, None),
            coarse_tune: None,
        }
    }
}

impl Parameters {
    /// Applies one selector or data entry, exactly as a synthesizer reading the file would.
    ///
    /// `drum_key_tune` takes GS 18H, whose LSB is a key number rather than half a parameter id, so
    /// one channel holds up to 128 of them and they cannot live in a field here. `None` is a channel
    /// holding no kit, where the parameter has nothing to name.
    fn apply(&mut self, controller: u8, value: u8, drum_key_tune: Option<&mut [Option<u8>; 128]>) {
        match controller {
            CC_RPN_MSB => {
                self.registered.0 = value;
                self.touched_registered = true;
                self.selected = Selected::Registered;
            }
            CC_RPN_LSB => {
                self.registered.1 = value;
                self.touched_registered = true;
                self.selected = Selected::Registered;
            }
            CC_NRPN_MSB => {
                self.non_registered.0 = value;
                self.touched_non_registered = true;
                self.selected = Selected::NonRegistered;
            }
            CC_NRPN_LSB => {
                self.non_registered.1 = value;
                self.touched_non_registered = true;
                self.selected = Selected::NonRegistered;
            }
            CC_DATA_COARSE => match self.selected {
                Selected::Registered => match self.registered {
                    (0, 0) => self.bend_range.0 = Some(value),
                    (0, 1) => self.fine_tune.0 = Some(value),
                    (0, 2) => self.coarse_tune = Some(value),
                    _ => {}
                },
                Selected::NonRegistered => {
                    if let Some(keys) = drum_key_tune
                        && self.non_registered.0 == NRPN_DRUM_PITCH_COARSE
                    {
                        keys[usize::from(self.non_registered.1 & 0x7F)] = Some(value);
                    }
                }
                Selected::Neither => {}
            },
            CC_DATA_FINE if self.selected == Selected::Registered => match self.registered {
                (0, 0) => self.bend_range.1 = Some(value),
                (0, 1) => self.fine_tune.1 = Some(value),
                _ => {}
            },
            _ => {}
        }
    }

    /// Whether the file said anything this channel would need replaying.
    fn is_empty(&self) -> bool {
        !self.touched_registered && !self.touched_non_registered
    }
}

/// Plays one song, emitting MIDI messages as time passes.
pub struct Sequencer {
    song: Arc<Song>,
    settings: PlaybackSettings,
    /// Corrections for defects in this song's own events.
    ///
    /// Beside the settings rather than inside them, because the settings carry over from the
    /// previous song and this must not: a fix belongs to one file. Flat and `Copy` for the reason
    /// `sounding` is sized once — it is read in the audio callback, where nothing may allocate.
    fixes: ChannelFixes,
    /// Index of the next event to dispatch.
    next_index: usize,
    /// Position in microseconds of song time.
    position_us: f64,
    /// Notes started and not yet stopped, so a note-off can match the pitch that was played.
    ///
    /// **Sized once, at [`crate::source::MAX_POLYPHONY`], and never grown after.** It only ever
    /// shrinks on `clear()`, so a capacity below the synthesizer's voice ceiling — 32 against a pool
    /// of 256, say — means the first dense passage of a song reallocates it, on the audio thread,
    /// which the rule at the top of `player.rs` forbids.
    sounding: Vec<SoundingNote>,
    /// Each channel's bend, for the recentre fix.
    ///
    /// The tick from which a note starts on a bend the file left behind, per channel, by
    /// [`stranded_from`]. One tick rather than the bend and its tick, because `Player` holds a
    /// sequencer inline in an enum beside a video song's much smaller track, and boxing it would
    /// allocate in the audio callback.
    stranded_from: [u32; 16],
    finished: bool,
}

impl Sequencer {
    /// Starts a song at its beginning.
    pub fn new(song: Arc<Song>, settings: PlaybackSettings, fixes: ChannelFixes) -> Self {
        Self {
            song,
            settings: settings.clamped(),
            fixes,
            next_index: 0,
            position_us: 0.0,
            sounding: Vec::with_capacity(crate::source::MAX_POLYPHONY),
            stranded_from: [NEVER_STRANDED; 16],
            finished: false,
        }
    }

    /// The song being played.
    pub fn song(&self) -> &Arc<Song> {
        &self.song
    }

    /// Current settings.
    pub fn settings(&self) -> PlaybackSettings {
        self.settings
    }

    /// Current position in ticks, which is what the display interpolates the lyric highlight from.
    pub fn position_ticks(&self) -> u32 {
        self.song.tempo_map.us_to_tick(self.position_us as u64)
    }

    /// Current position in milliseconds of song time.
    pub fn position_ms(&self) -> u32 {
        u32::try_from((self.position_us / 1_000.0) as u64).unwrap_or(u32::MAX)
    }

    /// Whether every event has been dispatched and the song has run to its end.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Advances by a span of real time, emitting everything due.
    ///
    /// `elapsed_us` is wall-clock time; the tempo ratio is applied here, so a ratio of 1.25 consumes
    /// song time 25% faster than the clock.
    pub fn advance(&mut self, elapsed_us: f64, sink: &mut impl MidiSink) {
        if self.finished {
            return;
        }
        self.position_us += elapsed_us * f64::from(self.settings.tempo_ratio);
        let target_tick = self.song.tempo_map.us_to_tick(self.position_us as u64);

        while let Some(event) = self.song.events.get(self.next_index) {
            if event.tick > target_tick {
                break;
            }
            self.dispatch(event.tick, event.kind, sink);
            self.next_index += 1;
        }

        if self.next_index >= self.song.events.len() && target_tick >= self.song.duration_ticks {
            self.finished = true;
        }
    }

    /// Emits one event, applying transposition and the melody mute.
    fn dispatch(&mut self, tick: u32, kind: EventKind, sink: &mut impl MidiSink) {
        match kind {
            EventKind::NoteOn {
                channel,
                key,
                velocity,
            } => {
                if self.is_muted(channel) {
                    // Deliberately not recorded as sounding: there is nothing to stop later.
                    return;
                }
                if velocity > 0 {
                    self.recentre_if_stranded(tick, channel, sink);
                }
                let played_key = self.transposed(channel, key);
                self.sounding.push(SoundingNote {
                    channel,
                    written_key: key,
                    played_key,
                });
                sink.note_on(channel, played_key, velocity);
            }
            EventKind::NoteOff { channel, key } => {
                // Match the pitch that was actually played. Without this, changing the transpose
                // while a note is held would leave it sounding forever, because the note-off would
                // name a pitch that was never started.
                let played_key = match self
                    .sounding
                    .iter()
                    .rposition(|n| n.channel == channel && n.written_key == key)
                {
                    Some(index) => self.sounding.swap_remove(index).played_key,
                    // Unmatched note-off: either the note was muted, or the file is sloppy. Passing
                    // it on is harmless and never leaves a note stuck.
                    None => self.transposed(channel, key),
                };
                sink.note_off(channel, played_key);
            }
            EventKind::Controller {
                channel,
                controller,
                value,
            } => {
                if self.suppresses_bank_select(channel, controller) {
                    return;
                }
                if controller == CC_RESET_ALL_CONTROLLERS
                    && let Some(from) = self.stranded_from.get_mut(usize::from(channel))
                {
                    *from = NEVER_STRANDED;
                }
                sink.control_change(channel, controller, value);
            }
            EventKind::ProgramChange { channel, program } => {
                sink.program_change(channel, self.program_for(channel, program));
            }
            EventKind::PitchBend { channel, value } => {
                if let Some(from) = self.stranded_from.get_mut(usize::from(channel)) {
                    *from = stranded_from(value, Some(tick), self.song.ticks_per_quarter)
                        .unwrap_or(NEVER_STRANDED);
                }
                sink.pitch_bend(channel, value);
            }
            EventKind::ChannelAftertouch { channel, value } => {
                sink.channel_aftertouch(channel, value);
            }
            // **Poly aftertouch is still dropped, and the reason is the key rather than the rarity.**
            // It addresses a *note*, and this sequencer transposes notes -- so forwarding it means
            // mapping its key through `sounding` exactly as note-off does, and getting that wrong
            // applies pressure to a note nobody is holding. Measured over 25,000 corpus files it is
            // in 0.67% of them against channel pressure's 7.7%, so the trap is most of the cost and
            // almost none of the benefit. Revisit it with the transposition, not without.
            EventKind::PolyAftertouch { .. } => {}
        }
    }

    /// Whether a channel's notes are currently suppressed.
    fn is_muted(&self, channel: u8) -> bool {
        self.fixes.mute[usize::from(channel)]
            || (!self.settings.melody_enabled && self.settings.melody_channel == Some(channel))
    }

    /// Returns a channel's bend to centre before a note that would start on one the file left behind.
    ///
    /// Only where the recentre fix is in force, and by the rule the detector that proposed the fix
    /// also uses, `km_fixes::recentre_bend::is_stranded`. Once recentred, the channel is at centre
    /// and nothing more is sent until the file bends it again.
    fn recentre_if_stranded(&mut self, tick: u32, channel: u8, sink: &mut impl MidiSink) {
        let index = usize::from(channel);
        if !self
            .fixes
            .recentre_bend
            .get(index)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        if let Some(from) = self.stranded_from.get_mut(index)
            && tick >= *from
        {
            *from = NEVER_STRANDED;
            sink.pitch_bend(channel, BEND_CENTRE);
        }
    }

    /// Whether a controller message selects a bank on a channel whose bank selects are dropped.
    ///
    /// Both halves go. A coarse select passed on with its fine partner suppressed still names a
    /// bank, and the pair is one message written as two.
    fn suppresses_bank_select(&self, channel: u8, controller: u8) -> bool {
        self.fixes.ignore_bank[usize::from(channel)]
            && (controller == km_fixes::ignore_bank_select::CC_BANK_SELECT_MSB
                || controller == km_fixes::ignore_bank_select::CC_BANK_SELECT_LSB)
    }

    /// The program to actually select on a channel, which a fix may have chosen instead.
    fn program_for(&self, channel: u8, program: u8) -> u8 {
        self.fixes.force_program[usize::from(channel)].unwrap_or(program)
    }

    /// The pitch to actually play for a written note.
    fn transposed(&self, channel: u8, key: u8) -> u8 {
        // Never transpose drums: a note number there picks an instrument, so shifting it turns a
        // kick drum into a cowbell.
        if channel == DRUM_CHANNEL || self.settings.transpose == 0 {
            return key;
        }
        i16::from(key)
            .saturating_add(i16::from(self.settings.transpose))
            .clamp(0, 127) as u8
    }

    /// Changes the transposition. Notes already sounding keep the pitch they started on.
    pub fn set_transpose(&mut self, semitones: i8) {
        self.settings.transpose = semitones.clamp(-MAX_TRANSPOSE, MAX_TRANSPOSE);
    }

    /// Changes the playback speed.
    pub fn set_tempo_ratio(&mut self, ratio: f32) {
        self.settings.tempo_ratio = ratio.clamp(MIN_TEMPO_RATIO, MAX_TEMPO_RATIO);
    }

    /// Turns the guide melody on or off.
    ///
    /// Silences any melody note currently sounding when switching off, so a held note does not ring
    /// on after the channel is muted.
    pub fn set_melody_enabled(&mut self, enabled: bool, sink: &mut impl MidiSink) {
        if self.settings.melody_enabled == enabled {
            return;
        }
        self.settings.melody_enabled = enabled;
        if let Some(channel) = self.settings.melody_channel
            && !enabled
        {
            let mut index = 0;
            while index < self.sounding.len() {
                if self.sounding[index].channel == channel {
                    let note = self.sounding.swap_remove(index);
                    sink.note_off(note.channel, note.played_key);
                } else {
                    index += 1;
                }
            }
        }
    }

    /// Sets which channel carries the melody, as recorded in the package.
    pub fn set_melody_channel(&mut self, channel: Option<u8>) {
        self.settings.melody_channel = channel;
    }

    /// Jumps to a position in milliseconds of song time.
    ///
    /// Controller and program state is replayed up to the target, so instruments, volumes and pans
    /// are what they would have been had the song played through. Without that, seeking past a
    /// program change leaves every channel on a piano.
    pub fn seek_ms(&mut self, ms: u32, sink: &mut impl MidiSink) {
        let target_tick = self.song.tempo_map.ms_to_tick(ms);
        self.seek_ticks(target_tick, sink);
    }

    /// Jumps to a tick.
    ///
    /// A full reset, not just a silence, for the reason [`Sequencer::restart`] gives: the replay
    /// below re-establishes program, controllers, bend and pressure from tick 0, so resetting first
    /// loses nothing and clears what the replay cannot reach. A hold pedal is the case that matters
    /// -- CC64 is only replayed if the file mentions it *before* the target, so a pedal pressed
    /// before the seek-from point and never touched again stayed down, and `rustysynth` holds every
    /// released voice while it is. The next note-off on that channel then defers indefinitely.
    pub fn seek_ticks(&mut self, target_tick: u32, sink: &mut impl MidiSink) {
        sink.reset();
        self.sounding.clear();
        self.position_us = self.song.tempo_map.tick_to_us(target_tick) as f64;
        self.finished = false;

        // Last value wins for each (channel, controller) and each channel's program, bend and
        // pressure. Channel pressure is replayed for the same reason CC7 is: it is channel *state*
        // that persists until something changes it, so a seek past the point where a file leant on
        // it would otherwise play the rest of the song without it.
        //
        // **A fixed table rather than a `Vec`, because this runs in the cpal data callback.**
        // `apply()` reaches here from `Command::SeekMs`, and the rule at the top of `player.rs`
        // forbids allocation there outright. What was here grew a `Vec<(u8, u8, u8)>` and searched
        // it linearly for every controller event before the target — so the cost was
        // O(events x controllers) *inside the callback*, and seeking into a dense file meant tens
        // of thousands of events against a growing vector while the audio thread had a buffer to
        // fill. That is an xrun rather than a glitch, and `SharedState::xruns` was already counting
        // them with nothing connecting the two.
        //
        // 4 KiB of stack, and O(events) with no allocation at all. The bound is the MIDI standard's
        // own: sixteen channels, a hundred and twenty-eight controllers.
        let mut controllers = [[None::<u8>; 128]; 16];
        let mut programs: [Option<u8>; 16] = [None; 16];
        let mut bends: [Option<u16>; 16] = [None; 16];
        let mut stranded: [u32; 16] = [NEVER_STRANDED; 16];
        let mut pressures: [Option<u8>; 16] = [None; 16];
        let mut parameters = [Parameters::default(); 16];
        // GS 18H, and for the drum channel alone. Each key there is a separate instrument, so the
        // tune is per key and a channel-wide one cannot express it — 128 slots for one channel
        // against 4 KiB for sixteen. One channel is enough because the GS message that moves the
        // rhythm part elsewhere is SysEx, and `km_song::Song` carries no SysEx, so channel 9 is the
        // only kit the synthesizer can hold.
        let mut drum_key_tune = [None::<u8>; 128];

        let mut index = 0;
        while let Some(event) = self.song.events.get(index) {
            if event.tick >= target_tick {
                break;
            }
            match event.kind {
                EventKind::Controller {
                    channel,
                    controller,
                    value,
                } => {
                    if PARAMETER_CONTROLLERS.contains(&controller) {
                        if let Some(slot) = parameters.get_mut(usize::from(channel)) {
                            let keys = (channel == DRUM_CHANNEL).then_some(&mut drum_key_tune);
                            slot.apply(controller, value, keys);
                        }
                    }
                    // Indexed rather than searched, and bounds-checked rather than masked: a
                    // channel or controller outside the standard's range comes from a malformed
                    // file, and dropping it is what the linear scan effectively did too.
                    else if let Some(slot) = controllers
                        .get_mut(usize::from(channel))
                        .and_then(|channel| channel.get_mut(usize::from(controller)))
                    {
                        *slot = Some(value);
                    }
                }
                EventKind::ProgramChange { channel, program } => {
                    if let Some(slot) = programs.get_mut(usize::from(channel)) {
                        *slot = Some(program);
                    }
                }
                EventKind::PitchBend { channel, value } => {
                    if let Some(slot) = bends.get_mut(usize::from(channel)) {
                        *slot = Some(value);
                    }
                    if let Some(slot) = stranded.get_mut(usize::from(channel)) {
                        *slot = stranded_from(value, Some(event.tick), self.song.ticks_per_quarter)
                            .unwrap_or(NEVER_STRANDED);
                    }
                }
                EventKind::ChannelAftertouch { channel, value } => {
                    if let Some(slot) = pressures.get_mut(usize::from(channel)) {
                        *slot = Some(value);
                    }
                }
                _ => {}
            }
            index += 1;
        }
        self.next_index = index;

        // Channel-major, controller ascending. Last value wins, so the order within a channel says
        // nothing except for the six parameter controllers, which carry no value of their own and
        // are replayed as parameters below instead. Bank Select still lands before its Program
        // Change, because programs are emitted after this.
        //
        // **The fixes are applied a second time here, and that is not a duplication to remove.**
        // This path emits into the sink directly rather than through `dispatch`, so a suppression
        // that lives only there survives exactly until the first seek — which would re-establish
        // the bank select the fix exists to remove, and leave the rest of the song playing a drum
        // kit that playing straight through never reaches.
        for (channel, values) in controllers.iter().enumerate() {
            for (controller, value) in values.iter().enumerate() {
                if self.suppresses_bank_select(channel as u8, controller as u8) {
                    continue;
                }
                if let Some(value) = value {
                    sink.control_change(channel as u8, controller as u8, *value);
                }
            }
        }

        // Each parameter stated with the selector it belongs to, then the selector state the file
        // left, so a data entry later in the song lands where a straight play would have put it.
        for (channel, params) in parameters.iter().enumerate() {
            if params.is_empty() {
                continue;
            }
            let channel = channel as u8;
            let mut state_registered = |selector: (u8, u8), entries: (Option<u8>, Option<u8>)| {
                if entries == (None, None) {
                    return;
                }
                sink.control_change(channel, CC_RPN_MSB, selector.0);
                sink.control_change(channel, CC_RPN_LSB, selector.1);
                if let Some(coarse) = entries.0 {
                    sink.control_change(channel, CC_DATA_COARSE, coarse);
                }
                if let Some(fine) = entries.1 {
                    sink.control_change(channel, CC_DATA_FINE, fine);
                }
            };
            state_registered((0, 0), params.bend_range);
            state_registered((0, 1), params.fine_tune);
            state_registered((0, 2), (params.coarse_tune, None));

            if channel == DRUM_CHANNEL {
                for (key, value) in drum_key_tune.iter().enumerate() {
                    if let Some(value) = value {
                        sink.control_change(channel, CC_NRPN_MSB, NRPN_DRUM_PITCH_COARSE);
                        sink.control_change(channel, CC_NRPN_LSB, key as u8);
                        sink.control_change(channel, CC_DATA_COARSE, *value);
                    }
                }
            }

            // The pair the file touched last is restored last, so a data entry after the target
            // belongs to the same one it would have belonged to.
            let registered = (
                params.touched_registered,
                params.registered,
                CC_RPN_MSB,
                CC_RPN_LSB,
            );
            let non_registered = (
                params.touched_non_registered,
                params.non_registered,
                CC_NRPN_MSB,
                CC_NRPN_LSB,
            );
            let order = if params.selected == Selected::NonRegistered {
                [registered, non_registered]
            } else {
                [non_registered, registered]
            };
            for (touched, selector, msb, lsb) in order {
                if !touched {
                    continue;
                }
                sink.control_change(channel, msb, selector.0);
                sink.control_change(channel, lsb, selector.1);
            }
        }
        for (channel, program) in programs.iter().enumerate() {
            if let Some(program) = program {
                sink.program_change(channel as u8, self.program_for(channel as u8, *program));
            }
        }
        for (channel, bend) in bends.iter().enumerate() {
            if let Some(bend) = bend {
                sink.pitch_bend(channel as u8, *bend);
            }
        }
        // From the bends just replayed, so the first note after the seek meets the recentre rule
        // exactly as it would have playing straight through.
        self.stranded_from = stranded;
        for (channel, pressure) in pressures.iter().enumerate() {
            if let Some(pressure) = pressure {
                sink.channel_aftertouch(channel as u8, *pressure);
            }
        }
    }

    /// Returns to the start of the song.
    ///
    /// A full reset, not just a silence: the song is about to replay from tick 0, so whatever its
    /// own last bars left on the channels must not still be in force. A file ending on a CC7 fade
    /// restarted silently before this.
    pub fn restart(&mut self, sink: &mut impl MidiSink) {
        sink.reset();
        self.sounding.clear();
        self.stranded_from = [NEVER_STRANDED; 16];
        self.next_index = 0;
        self.position_us = 0.0;
        self.finished = false;
    }

    /// Silences everything without moving the position, for pausing and stopping.
    pub fn silence(&mut self, sink: &mut impl MidiSink) {
        sink.all_notes_off();
        self.sounding.clear();
    }

    /// How many notes are currently sounding, for tests and diagnostics.
    pub fn sounding_count(&self) -> usize {
        self.sounding.len()
    }
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, testing};

    use super::*;

    /// Records everything sent, so a test can assert on exact messages.
    #[derive(Debug, Default)]
    struct Recorder {
        messages: Vec<Message>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Message {
        NoteOn(u8, u8, u8),
        NoteOff(u8, u8),
        Control(u8, u8, u8),
        Program(u8, u8),
        Bend(u8, u16),
        Pressure(u8, u8),
        AllNotesOff,
        Reset,
    }

    impl MidiSink for Recorder {
        fn note_on(&mut self, channel: u8, key: u8, velocity: u8) {
            self.messages.push(Message::NoteOn(channel, key, velocity));
        }
        fn note_off(&mut self, channel: u8, key: u8) {
            self.messages.push(Message::NoteOff(channel, key));
        }
        fn control_change(&mut self, channel: u8, controller: u8, value: u8) {
            self.messages
                .push(Message::Control(channel, controller, value));
        }
        fn program_change(&mut self, channel: u8, program: u8) {
            self.messages.push(Message::Program(channel, program));
        }
        fn pitch_bend(&mut self, channel: u8, value: u16) {
            self.messages.push(Message::Bend(channel, value));
        }
        fn channel_aftertouch(&mut self, channel: u8, value: u8) {
            self.messages.push(Message::Pressure(channel, value));
        }
        fn all_notes_off(&mut self) {
            self.messages.push(Message::AllNotesOff);
        }
        fn reset(&mut self) {
            self.messages.push(Message::Reset);
        }
    }

    impl Recorder {
        fn note_ons(&self) -> Vec<(u8, u8)> {
            self.messages
                .iter()
                .filter_map(|m| match m {
                    Message::NoteOn(channel, key, _) => Some((*channel, *key)),
                    _ => None,
                })
                .collect()
        }

        fn note_offs(&self) -> Vec<(u8, u8)> {
            self.messages
                .iter()
                .filter_map(|m| match m {
                    Message::NoteOff(channel, key) => Some((*channel, *key)),
                    _ => None,
                })
                .collect()
        }

        /// Every control change on one channel, in the order it was sent.
        fn controls_on(&self, wanted: u8) -> Vec<(u8, u8)> {
            self.messages
                .iter()
                .filter_map(|m| match m {
                    Message::Control(channel, controller, value) if *channel == wanted => {
                        Some((*controller, *value))
                    }
                    _ => None,
                })
                .collect()
        }

        /// The control changes on one channel that carry a parameter, selectors and data entries
        /// alike. Everything else a file sets is noise to a test about parameters.
        fn parameter_controls_on(&self, wanted: u8) -> Vec<(u8, u8)> {
            self.controls_on(wanted)
                .into_iter()
                .filter(|(controller, _)| PARAMETER_CONTROLLERS.contains(controller))
                .collect()
        }
    }

    fn song(bytes: &[u8]) -> Arc<Song> {
        Arc::new(Song::parse(bytes, &ParseOptions::default()).expect("fixture parses"))
    }

    /// One second of wall-clock time, in microseconds.
    const ONE_SECOND_US: f64 = 1_000_000.0;

    fn play_all(sequencer: &mut Sequencer, sink: &mut Recorder, seconds: u32) {
        // Advance in 64-sample blocks at 44.1 kHz, as the audio callback does, so the tests
        // exercise the same stepping the real player uses.
        let block_us = 64.0 * 1_000_000.0 / 44_100.0;
        let blocks = (f64::from(seconds) * ONE_SECOND_US / block_us) as u32;
        for _ in 0..blocks {
            sequencer.advance(block_us, sink);
        }
    }

    #[test]
    fn a_song_plays_its_notes_in_order() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 6);

        // Fourteen notes in the fixture, one per syllable.
        assert_eq!(sink.note_ons().len(), 14);
        assert_eq!(sink.note_offs().len(), 14);
        assert!(sequencer.is_finished());
        assert_eq!(sequencer.sounding_count(), 0, "every note was stopped");
    }

    #[test]
    fn nothing_is_emitted_before_its_time() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        // The fixture's notes are a quarter note apart at 120 BPM, so 250 ms.
        sequencer.advance(100_000.0, &mut sink);
        assert_eq!(sink.note_ons().len(), 1, "only the note at tick 0");
    }

    #[test]
    fn position_advances_in_song_time() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        sequencer.advance(ONE_SECOND_US, &mut sink);
        assert!(
            (990..=1_010).contains(&sequencer.position_ms()),
            "expected about 1000 ms, got {}",
            sequencer.position_ms()
        );
        // 480 ticks per quarter at 120 BPM means 960 ticks in a second.
        assert!(
            (950..=970).contains(&sequencer.position_ticks()),
            "expected about 960 ticks, got {}",
            sequencer.position_ticks()
        );
    }

    #[test]
    fn a_faster_tempo_ratio_consumes_song_time_faster() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        sequencer.set_tempo_ratio(1.25);
        sequencer.advance(ONE_SECOND_US, &mut sink);
        assert!(
            (1_230..=1_270).contains(&sequencer.position_ms()),
            "expected about 1250 ms of song time, got {}",
            sequencer.position_ms()
        );
    }

    #[test]
    fn the_tempo_ratio_is_clamped_to_the_supported_range() {
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        sequencer.set_tempo_ratio(4.0);
        assert_eq!(sequencer.settings().tempo_ratio, MAX_TEMPO_RATIO);
        sequencer.set_tempo_ratio(0.1);
        assert_eq!(sequencer.settings().tempo_ratio, MIN_TEMPO_RATIO);
    }

    #[test]
    fn transposition_shifts_pitched_notes() {
        let mut sink = Recorder::default();
        let settings = PlaybackSettings {
            transpose: 3,
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            settings,
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 6);

        // The fixture's tune starts on middle C.
        assert_eq!(sink.note_ons().first(), Some(&(0, 63)));
    }

    #[test]
    fn transposition_never_touches_the_drum_channel() {
        let mut sink = Recorder::default();
        let settings = PlaybackSettings {
            transpose: 5,
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(
            song(&testing::melody_and_accompaniment()),
            settings,
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 8);

        let drum_keys: Vec<u8> = sink
            .note_ons()
            .into_iter()
            .filter(|(channel, _)| *channel == DRUM_CHANNEL)
            .map(|(_, key)| key)
            .collect();
        assert!(!drum_keys.is_empty(), "the fixture has drums");
        assert!(
            drum_keys.iter().all(|&key| key == 36 || key == 38),
            "drum notes must be untouched, got {drum_keys:?}"
        );
    }

    #[test]
    fn transposition_is_clamped_to_the_supported_range() {
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        sequencer.set_transpose(50);
        assert_eq!(sequencer.settings().transpose, MAX_TRANSPOSE);
        sequencer.set_transpose(-50);
        assert_eq!(sequencer.settings().transpose, -MAX_TRANSPOSE);
    }

    #[test]
    fn changing_the_transpose_mid_note_still_stops_that_note() {
        // The case that leaves a note stuck forever if note-offs are transposed naively.
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );

        sequencer.advance(10_000.0, &mut sink);
        let started = sink.note_ons();
        assert_eq!(started.first(), Some(&(0, 60)), "note started untransposed");

        sequencer.set_transpose(4);
        play_all(&mut sequencer, &mut sink, 6);

        assert!(
            sink.note_offs().contains(&(0, 60)),
            "the held note must be stopped at the pitch it started on, got {:?}",
            sink.note_offs()
        );
        assert_eq!(sequencer.sounding_count(), 0, "nothing left sounding");
    }

    #[test]
    fn a_muted_melody_channel_emits_no_notes() {
        let mut sink = Recorder::default();
        let settings = PlaybackSettings {
            melody_enabled: false,
            melody_channel: Some(0),
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(
            song(&testing::melody_and_accompaniment()),
            settings,
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 8);

        assert!(
            !sink.note_ons().iter().any(|(channel, _)| *channel == 0),
            "channel 0 is the melody and should be silent"
        );
        assert!(
            sink.note_ons().iter().any(|(channel, _)| *channel == 1),
            "the accompaniment must still play"
        );
    }

    #[test]
    fn enabling_the_melody_lets_it_through() {
        let mut sink = Recorder::default();
        let settings = PlaybackSettings {
            melody_enabled: true,
            melody_channel: Some(0),
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(
            song(&testing::melody_and_accompaniment()),
            settings,
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 8);
        assert!(sink.note_ons().iter().any(|(channel, _)| *channel == 0));
    }

    #[test]
    fn muting_the_melody_mid_note_silences_what_is_already_sounding() {
        let mut sink = Recorder::default();
        let settings = PlaybackSettings {
            melody_enabled: true,
            melody_channel: Some(0),
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(
            song(&testing::melody_and_accompaniment()),
            settings,
            ChannelFixes::default(),
        );

        sequencer.advance(10_000.0, &mut sink);
        assert!(sequencer.sounding_count() > 0);

        let before = sequencer.sounding_count();
        sequencer.set_melody_enabled(false, &mut sink);

        assert!(
            !sink.note_offs().is_empty(),
            "a held melody note must be stopped, not left ringing"
        );
        // Only the melody stops. Muting the guide melody must never silence the accompaniment,
        // which is the whole point of the control.
        assert!(
            sequencer.sounding_count() < before,
            "melody notes should have been released"
        );
        assert!(
            sequencer.sounding_count() > 0,
            "the accompaniment must keep sounding, {} notes remain",
            sequencer.sounding_count()
        );
        assert!(
            sink.note_offs().iter().all(|(channel, _)| *channel == 0),
            "only channel 0 should have been stopped, got {:?}",
            sink.note_offs()
        );
    }

    #[test]
    fn the_melody_mute_does_nothing_when_no_channel_is_declared() {
        let mut sink = Recorder::default();
        let settings = PlaybackSettings {
            melody_enabled: false,
            melody_channel: None,
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(
            song(&testing::melody_and_accompaniment()),
            settings,
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 8);
        // With no melody channel recorded, nothing is suppressed.
        assert!(sink.note_ons().iter().any(|(channel, _)| *channel == 0));
    }

    #[test]
    fn seeking_replays_program_changes_so_instruments_are_right() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::melody_and_accompaniment()),
            Default::default(),
            ChannelFixes::default(),
        );

        // Jump past the program changes at the top of the file.
        sequencer.seek_ms(2_000, &mut sink);

        let programs: Vec<(u8, u8)> = sink
            .messages
            .iter()
            .filter_map(|m| match m {
                Message::Program(channel, program) => Some((*channel, *program)),
                _ => None,
            })
            .collect();
        assert!(
            programs.contains(&(0, 73)) && programs.contains(&(1, 0)),
            "both channels' instruments must be restored, got {programs:?}"
        );
        // Not an `all_notes_off`, which is what this was: that touches no channel state, so a hold
        // pedal pressed before the seek-from point and never mentioned again stayed down and held
        // every later note-off on its channel. The replay above is what makes the reset free.
        assert_eq!(
            sink.messages.first(),
            Some(&Message::Reset),
            "seeking must clear the channels, not only silence them"
        );
    }

    /// Channel pressure reaches the synthesizer; per-note pressure deliberately does not.
    ///
    /// Dropping both, on the grounds that they are rare, does not survive the measurement: the
    /// synthesizer honors them and channel pressure is in 7.7% of corpus files. The one reason to
    /// drop anything is poly pressure's key, which this sequencer transposes and which would
    /// therefore have to be mapped rather than passed.
    #[test]
    fn channel_pressure_is_forwarded_and_per_note_pressure_is_not() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::channel_pressure()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 8);

        let pressures: Vec<(u8, u8)> = sink
            .messages
            .iter()
            .filter_map(|m| match m {
                Message::Pressure(channel, value) => Some((*channel, *value)),
                _ => None,
            })
            .collect();
        assert_eq!(
            pressures,
            vec![(0, 64), (0, 127)],
            "both channel-pressure events, in order"
        );
    }

    /// A seek restores the last channel pressure, for the reason it restores CC7.
    ///
    /// Pressure persists on the channel until something changes it, so a jump past the point where
    /// a file leant on it would otherwise play the remainder unpressed — the same defect as seeking
    /// past a program change and landing on a piano.
    #[test]
    fn seeking_replays_the_last_channel_pressure() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::channel_pressure()),
            Default::default(),
            ChannelFixes::default(),
        );

        // Past both pressure events, which sit inside the first beat.
        sequencer.seek_ms(1_500, &mut sink);

        let pressures: Vec<(u8, u8)> = sink
            .messages
            .iter()
            .filter_map(|m| match m {
                Message::Pressure(channel, value) => Some((*channel, *value)),
                _ => None,
            })
            .collect();
        assert_eq!(
            pressures,
            vec![(0, 127)],
            "the last value only, not every one on the way"
        );
    }

    /// A straight play hands the five control changes of a registered parameter over in file order.
    #[test]
    fn a_registered_parameter_reaches_the_sink_in_file_order() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::pitch_bend_range()),
            Default::default(),
            ChannelFixes::default(),
        );

        play_all(&mut sequencer, &mut sink, 4);

        assert_eq!(
            sink.parameter_controls_on(8),
            vec![(101, 0), (100, 0), (6, 12), (101, 127), (100, 127)]
        );
    }

    /// Seeking past a pitch bend range re-establishes it, selector first.
    ///
    /// **The defect this exists for made a whole part play out of tune.** Replaying controllers by
    /// number puts the data entry, CC 6, ahead of the selectors that give it meaning, so a
    /// synthesizer receives a number for the null parameter and discards it. The channel keeps the
    /// General MIDI default of two semitones while the file's bends are written for twelve, and
    /// every bent note lands a fraction of a semitone from where it belongs — for the rest of the
    /// song, and in every key.
    #[test]
    fn seeking_re_establishes_a_pitch_bend_range() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::pitch_bend_range()),
            Default::default(),
            ChannelFixes::default(),
        );

        // Past the parameter and the first bend, and short of the closing note.
        sequencer.seek_ms(2_000, &mut sink);

        assert_eq!(
            sink.parameter_controls_on(8),
            vec![(101, 0), (100, 0), (6, 12), (101, 127), (100, 127)],
            "the parameter stated with its own selector, then the null parameter the file left"
        );
    }

    /// A channel that never sets a parameter is sent none, rather than a bare data entry.
    #[test]
    fn seeking_sends_no_parameter_for_a_channel_that_set_none() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::pitch_bend_range()),
            Default::default(),
            ChannelFixes::default(),
        );

        sequencer.seek_ms(2_000, &mut sink);

        assert!(
            sink.parameter_controls_on(2).is_empty(),
            "channel 2 bends at the default range and asks for nothing: {:?}",
            sink.parameter_controls_on(2)
        );
    }

    /// No data entry is ever sent before the selector that says what it is for.
    ///
    /// Broader than the two tests above and the one that actually pins the shape: it holds for every
    /// channel of every fixture, so a controller replayed by number can never creep back.
    #[test]
    fn seeking_never_sends_a_data_entry_before_its_selector() {
        for (name, fixture) in testing::FIXTURES {
            let song = song(&fixture());
            for target in [500u32, 2_000, 10_000] {
                let mut sink = Recorder::default();
                let mut sequencer = Sequencer::new(
                    Arc::clone(&song),
                    Default::default(),
                    ChannelFixes::default(),
                );
                sequencer.seek_ms(target, &mut sink);

                for channel in 0..16u8 {
                    let mut selected = false;
                    for (controller, _) in sink.parameter_controls_on(channel) {
                        match controller {
                            CC_RPN_MSB | CC_RPN_LSB | CC_NRPN_MSB | CC_NRPN_LSB => selected = true,
                            CC_DATA_COARSE | CC_DATA_FINE => assert!(
                                selected,
                                "{name} at {target} ms, channel {channel}: \
                                 a data entry with no parameter selected"
                            ),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// The Roland GS per-key drum tune survives a seek, key by key.
    ///
    /// A channel-wide tune cannot stand in for it: each key of a kit is a separate instrument, so
    /// the file states one parameter per key and all of them have to come back.
    #[test]
    fn seeking_re_establishes_the_drum_key_tune() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::drum_key_tune()),
            Default::default(),
            ChannelFixes::default(),
        );

        sequencer.seek_ms(3_000, &mut sink);

        assert_eq!(
            sink.parameter_controls_on(DRUM_CHANNEL),
            vec![
                (99, 0x18),
                (98, 36),
                (6, 55),
                (99, 0x18),
                (98, 38),
                (6, 67),
                (99, 127),
                (98, 127),
            ]
        );
    }

    /// A seek leaves a channel holding exactly what a straight play to the same point would.
    ///
    /// The parameters are the point, and the assertion is deliberately over the whole controller
    /// stream: last-value-wins makes the two orders differ, so this compares the *state* each
    /// leaves rather than the messages each sent.
    #[test]
    fn a_seek_leaves_the_same_parameters_as_playing_there() {
        for (name, fixture) in testing::FIXTURES {
            let song = song(&fixture());

            let mut played = Recorder::default();
            let mut sequencer = Sequencer::new(
                Arc::clone(&song),
                Default::default(),
                ChannelFixes::default(),
            );
            play_all(&mut sequencer, &mut played, 3);

            let mut seeked = Recorder::default();
            let mut sequencer = Sequencer::new(
                Arc::clone(&song),
                Default::default(),
                ChannelFixes::default(),
            );
            sequencer.seek_ms(3_000, &mut seeked);

            for channel in 0..16u8 {
                assert_eq!(
                    parameters_after(&played, channel),
                    parameters_after(&seeked, channel),
                    "{name}, channel {channel}"
                );
            }
        }
    }

    /// Runs a recorded control stream through the parameter state machine, as a synthesizer would.
    fn parameters_after(sink: &Recorder, channel: u8) -> (Parameters, [Option<u8>; 128]) {
        let mut params = Parameters::default();
        let mut keys = [None::<u8>; 128];
        for (controller, value) in sink.controls_on(channel) {
            params.apply(controller, value, Some(&mut keys));
        }
        (params, keys)
    }

    /// A seek over a dense controller stream replays the last value on every slot, once each.
    ///
    /// **The fault this guards was a real-time one, and it is written as a correctness test because
    /// that is what a unit test can honestly assert.** `seek_ticks` runs inside the cpal data
    /// callback, and it used to collect controllers into a `Vec` searched linearly for every event
    /// before the target — so the cost was O(events × controllers) *with an allocation in it*, on
    /// the one thread that must never allocate. The two fixtures the other seek tests use are small
    /// enough that neither the cost nor the allocation was visible.
    ///
    /// Two full sweeps of the standard's whole space, so every slot is written and then overwritten:
    /// 4,096 events before the seek point, where the old shape did ~8.4 million comparisons and a
    /// run of reallocations. What is asserted is the *result*: sixteen channels by the hundred and
    /// twenty-two controllers that carry a value of their own, each replayed exactly once and each
    /// carrying the second sweep's value. Channel 15 and controller 127 are in there, which is the
    /// indexing boundary.
    ///
    /// The other six are the parameter controllers, and a sweep is exactly the stream that proves
    /// they cannot be replayed this way: the two data entries arrive ahead of every selector, so
    /// they name no parameter and mean nothing. They are dropped, and the four selectors come back
    /// as the state the sweep left.
    #[test]
    fn seeking_over_a_dense_controller_stream_replays_the_last_value_once_per_slot() {
        let mut base = Song::parse(&testing::soft_karaoke(), &ParseOptions::default())
            .expect("fixture parses");

        let mut events = Vec::new();
        for pass in 1..=2u8 {
            for channel in 0..16u8 {
                for controller in 0..128u8 {
                    events.push(km_song::TimedEvent {
                        tick: u32::from(pass),
                        track: 0,
                        kind: EventKind::Controller {
                            channel,
                            controller,
                            value: pass,
                        },
                    });
                }
            }
        }
        base.events = events;
        base.duration_ticks = 1_000;

        let mut sink = Recorder::default();
        let mut sequencer =
            Sequencer::new(Arc::new(base), Default::default(), ChannelFixes::default());
        sequencer.seek_ticks(10, &mut sink);

        let mut replayed: Vec<(u8, u8, u8)> = sink
            .messages
            .iter()
            .filter_map(|m| match m {
                Message::Control(channel, controller, value) => {
                    Some((*channel, *controller, *value))
                }
                _ => None,
            })
            .collect();
        // 122 controllers carrying their own value, plus the four selectors put back.
        assert_eq!(
            replayed.len(),
            16 * (122 + 4),
            "every slot replayed exactly once, and no slot twice"
        );
        assert!(
            replayed.iter().all(|(_, _, value)| *value == 2),
            "the second sweep's value must win everywhere"
        );
        assert!(
            !replayed
                .iter()
                .any(|(_, controller, _)| *controller == CC_DATA_COARSE
                    || *controller == CC_DATA_FINE),
            "a data entry that named no parameter carries nothing to replay"
        );
        replayed.sort_unstable();
        replayed.dedup();
        assert_eq!(
            replayed.len(),
            16 * (122 + 4),
            "every (channel, controller) pair must be distinct"
        );
        assert!(
            replayed.contains(&(15, 127, 2)),
            "the last channel and the last controller are the indexing boundary"
        );
    }

    #[test]
    fn seeking_positions_the_event_cursor_correctly() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );

        sequencer.seek_ms(2_000, &mut sink);
        assert!((1_990..=2_010).contains(&sequencer.position_ms()));

        sink.messages.clear();
        play_all(&mut sequencer, &mut sink, 4);
        // Only the notes after the seek point, not the whole song.
        assert!(
            sink.note_ons().len() < 14,
            "seeking should skip earlier notes, got {}",
            sink.note_ons().len()
        );
    }

    #[test]
    fn seeking_backwards_replays_from_the_start() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 6);
        assert!(sequencer.is_finished());

        sequencer.seek_ms(0, &mut sink);
        assert!(!sequencer.is_finished(), "seeking back reopens the song");
        sink.messages.clear();
        play_all(&mut sequencer, &mut sink, 6);
        assert_eq!(sink.note_ons().len(), 14);
    }

    #[test]
    fn restarting_replays_the_whole_song() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 6);

        sequencer.restart(&mut sink);
        assert_eq!(sequencer.position_ms(), 0);
        sink.messages.clear();
        play_all(&mut sequencer, &mut sink, 6);
        assert_eq!(sink.note_ons().len(), 14);
    }

    #[test]
    fn restarting_resets_the_channels_rather_than_only_silencing() {
        // A song that ends on a CC7 fade to zero -- which real files do -- restarted silently while
        // this was an `all_notes_off`, because that touches no channel state.
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 6);

        sink.messages.clear();
        sequencer.restart(&mut sink);
        assert_eq!(sink.messages.first(), Some(&Message::Reset));
    }

    #[test]
    fn pausing_silences_without_resetting_the_channels() {
        // The counterpart: a reset would mute the reverb tail and lose the state the song is in.
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        sequencer.advance(ONE_SECOND_US, &mut sink);

        sink.messages.clear();
        sequencer.silence(&mut sink);
        assert!(sink.messages.contains(&Message::AllNotesOff));
        assert!(!sink.messages.contains(&Message::Reset));
    }

    #[test]
    fn silencing_stops_notes_without_moving_the_position() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        sequencer.advance(ONE_SECOND_US, &mut sink);
        let position = sequencer.position_ms();

        sequencer.silence(&mut sink);
        assert!(sink.messages.contains(&Message::AllNotesOff));
        assert_eq!(sequencer.position_ms(), position);
        assert_eq!(sequencer.sounding_count(), 0);
    }

    #[test]
    fn a_finished_sequencer_emits_nothing_further() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::soft_karaoke()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 6);
        assert!(sequencer.is_finished());

        let count = sink.messages.len();
        play_all(&mut sequencer, &mut sink, 6);
        assert_eq!(sink.messages.len(), count, "nothing more should be emitted");
    }

    #[test]
    fn an_embedded_tempo_change_is_honored() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::tempo_change()),
            Default::default(),
            ChannelFixes::default(),
        );

        // The fixture runs at 120 BPM for two beats, then 60 BPM. After two seconds of real time,
        // song position is 2 s, which is one second past the change.
        sequencer.advance(2.0 * ONE_SECOND_US, &mut sink);
        let ticks = sequencer.position_ticks();
        // 960 ticks in the first second at 120 BPM, then 480 more in the second at 60 BPM.
        assert!(
            (1_420..=1_460).contains(&ticks),
            "expected about 1440 ticks, got {ticks}"
        );
    }

    #[test]
    fn a_song_with_no_events_finishes_immediately_rather_than_hanging() {
        let mut sink = Recorder::default();
        // Lyrics but no notes.
        let mut sequencer = Sequencer::new(
            song(&testing::lyric_events()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 10);
        assert!(sequencer.is_finished());
    }

    /// The fix table for the one channel the kit-bank fixture has a defect on.
    fn ignoring_bank_on(channel: u8) -> ChannelFixes {
        let mut fixes = ChannelFixes::default();
        fixes.ignore_bank[usize::from(channel)] = true;
        fixes
    }

    /// Every bank select sent on a channel, coarse and fine alike.
    fn bank_selects_on(sink: &Recorder, channel: u8) -> Vec<(u8, u8)> {
        sink.controls_on(channel)
            .into_iter()
            .filter(|(controller, _)| *controller == 0 || *controller == 32)
            .collect()
    }

    #[test]
    fn a_suppressed_bank_select_never_reaches_the_sink() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            ignoring_bank_on(4),
        );
        play_all(&mut sequencer, &mut sink, 6);

        assert!(bank_selects_on(&sink, 4).is_empty());
        // The program change the file sends is still made, so the channel selects an instrument
        // from the default bank rather than falling silent.
        assert!(sink.messages.contains(&Message::Program(4, 17)));
    }

    #[test]
    fn a_seek_does_not_restore_a_suppressed_bank_select() {
        // **This is the regression the fix is worthless without.** `seek_ticks` emits into the sink
        // directly rather than through `dispatch`, so a suppression written only there survives
        // until somebody seeks — and the first seek would put back exactly the message the fix
        // exists to remove, for the rest of the song.
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            ignoring_bank_on(4),
        );
        sequencer.seek_ms(1_500, &mut sink);

        assert!(bank_selects_on(&sink, 4).is_empty());
        assert!(sink.messages.contains(&Message::Program(4, 17)));
    }

    #[test]
    fn a_seek_replays_a_bank_select_that_is_not_suppressed() {
        // The control for the pair above: without the fix the replay does put it back, so those
        // two are asserting an absence that is genuinely caused.
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            ChannelFixes::default(),
        );
        sequencer.seek_ms(1_500, &mut sink);

        assert_eq!(bank_selects_on(&sink, 4), vec![(0, 127)]);
    }

    #[test]
    fn a_muted_channel_sounds_nothing_and_leaves_the_rest_alone() {
        let mut sink = Recorder::default();
        let mut fixes = ChannelFixes::default();
        fixes.mute[4] = true;
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            fixes,
        );
        play_all(&mut sequencer, &mut sink, 6);

        assert!(!sink.note_ons().iter().any(|(channel, _)| *channel == 4));
        assert!(sink.note_ons().iter().any(|(channel, _)| *channel == 2));
    }

    /// Where each message of a kind sits in what was sent, so a test can assert on order.
    fn position_of(sink: &Recorder, message: &Message) -> Option<usize> {
        sink.messages.iter().position(|sent| sent == message)
    }

    fn recentring_channel_4() -> ChannelFixes {
        let mut fixes = ChannelFixes::default();
        fixes.recentre_bend[4] = true;
        fixes
    }

    #[test]
    fn a_bend_left_off_centre_is_recentred_once_before_the_next_note() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::bend_left_off_centre()),
            Default::default(),
            recentring_channel_4(),
        );
        play_all(&mut sequencer, &mut sink, 4);

        let left = position_of(&sink, &Message::Bend(4, 6784)).expect("the file's own bend");
        let centre = position_of(&sink, &Message::Bend(4, BEND_CENTRE)).expect("recentred");
        let note = position_of(&sink, &Message::NoteOn(4, 55, 100)).expect("the next note");
        assert!(left < centre && centre < note);
        // The second note finds the bend already at centre, so nothing is sent twice.
        let recentres = sink
            .messages
            .iter()
            .filter(|sent| **sent == Message::Bend(4, BEND_CENTRE))
            .count();
        assert_eq!(recentres, 1);
    }

    #[test]
    fn without_the_fix_a_bend_left_off_centre_stays_where_the_file_put_it() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::bend_left_off_centre()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut sink, 4);

        assert!(position_of(&sink, &Message::Bend(4, BEND_CENTRE)).is_none());
    }

    #[test]
    fn recentring_one_channel_leaves_a_channel_detuned_on_purpose_alone() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::bend_left_off_centre()),
            Default::default(),
            recentring_channel_4(),
        );
        play_all(&mut sequencer, &mut sink, 4);

        assert!(position_of(&sink, &Message::Bend(6, BEND_CENTRE)).is_none());
    }

    #[test]
    fn a_seek_past_a_bend_left_off_centre_still_recentres_before_the_next_note() {
        let mut sink = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::bend_left_off_centre()),
            Default::default(),
            recentring_channel_4(),
        );
        // Tick 1152: past the last bend at 960, before the note at 1440.
        sequencer.seek_ms(1_200, &mut sink);
        play_all(&mut sequencer, &mut sink, 2);

        let left = position_of(&sink, &Message::Bend(4, 6784)).expect("replayed by the seek");
        let centre = position_of(&sink, &Message::Bend(4, BEND_CENTRE)).expect("recentred");
        let note = position_of(&sink, &Message::NoteOn(4, 55, 100)).expect("the next note");
        assert!(left < centre && centre < note);
    }

    #[test]
    fn a_forced_program_replaces_the_files_own_through_playing_and_seeking() {
        let mut fixes = ChannelFixes::default();
        fixes.force_program[4] = Some(52);

        let mut played = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            fixes,
        );
        play_all(&mut sequencer, &mut played, 6);

        let mut seeked = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            fixes,
        );
        sequencer.seek_ms(1_500, &mut seeked);

        for sink in [&played, &seeked] {
            assert!(sink.messages.contains(&Message::Program(4, 52)));
            assert!(!sink.messages.contains(&Message::Program(4, 17)));
        }
    }

    #[test]
    fn a_fix_on_one_channel_changes_nothing_on_another() {
        let mut plain = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            ChannelFixes::default(),
        );
        play_all(&mut sequencer, &mut plain, 6);

        let mut fixed = Recorder::default();
        let mut sequencer = Sequencer::new(
            song(&testing::kit_bank_on_a_melodic_channel()),
            Default::default(),
            ignoring_bank_on(4),
        );
        play_all(&mut sequencer, &mut fixed, 6);

        for channel in [2u8, 5, 9] {
            assert_eq!(plain.controls_on(channel), fixed.controls_on(channel));
        }
        assert_eq!(plain.note_ons(), fixed.note_ons());
    }

    #[test]
    fn every_fixture_seeks_to_the_same_bank_state_it_plays_to() {
        // The pair of sweeps below assert this for parameters; a suppressed bank select is the
        // same claim for a message the replay path emits from its own table.
        for (name, fixture) in testing::FIXTURES {
            let bytes = fixture();
            let parsed = song(&bytes);
            let fixes = ignoring_bank_on(4);

            let mut played = Recorder::default();
            let mut sequencer = Sequencer::new(Arc::clone(&parsed), Default::default(), fixes);
            play_all(&mut sequencer, &mut played, 3);

            let mut seeked = Recorder::default();
            let mut sequencer = Sequencer::new(parsed, Default::default(), fixes);
            sequencer.seek_ms(3_000, &mut seeked);

            assert!(
                bank_selects_on(&played, 4).is_empty(),
                "{name} sent a suppressed bank select while playing"
            );
            assert!(
                bank_selects_on(&seeked, 4).is_empty(),
                "{name} restored a suppressed bank select on a seek"
            );
        }
    }
}
