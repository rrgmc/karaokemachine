//! Putting a pitch bend back to centre when a file's return from one stops short.
//!
//! **The defect is a bend gesture whose last step is missing.** A part bends a chord down and ramps
//! back up in steps, and the ramp ends one step before centre. Nothing else moves the bend, so every
//! note the channel plays for the next several bars sounds a fraction of a semitone flat or sharp
//! against the rest of the arrangement. It is heard as beating between that part and the parts
//! doubling it, and as a chord that is wrong without any note in it being wrong.
//!
//! **Every bank agrees about it**, which is what separates it from a bank select: a synthesizer that
//! honours the file plays it out of tune, and so does the hardware module it was written for.
//!
//! **This fix is offered rather than applied.** A note started while a bend is held is usually the
//! defect, and sometimes a guitar part playing into a bend it meant to keep. Nothing in the file
//! tells the two apart with certainty, so the detector proposes the channel and a person agrees.
//!
//! **The detector and the sequencer share one rule**, [`is_stranded`]: a note starting at least a
//! beat after the channel's last bend event, while that bend is off centre. The detector adds two
//! conditions of its own so that it proposes only the clear cases. The bend must be off by at least
//! [`MIN_CENTS`], because a residue of a few cents is inaudible. And it must end a return: its last
//! step moved towards centre, and it sits at no more than half of the furthest point of the movement
//! it ends, a movement being bend events each within a beat of the one before.
//!
//! Each half of that return rule removes a deliberate shape. A bend that climbs to full deflection
//! and stays there is a part holding a bent note. A vibrato written around a held bend ends with a
//! small step towards centre, but close to where it wobbled. A file that detunes a channel once and
//! never moves it again has no movement at all.

use km_song::{EventKind, Song};

use crate::{CHANNELS, DRUM_CHANNEL, Fix};

/// The centre of a 14-bit pitch bend, where a note plays at its written pitch.
pub const BEND_CENTRE: u16 = 8192;

/// The smallest offset, in cents, the detector proposes a correction for.
pub const MIN_CENTS: f32 = 20.0;

/// How many separate note starts a channel must play on a stranded bend before it is proposed.
pub const MIN_ONSETS: usize = 2;

/// Reset All Controllers, which returns a channel's bend to centre on every synthesizer.
const CC_RESET_ALL_CONTROLLERS: u8 = 121;

/// Registered and non-registered parameter selectors, and data entry.
const CC_DATA_COARSE: u8 = 6;
const CC_DATA_FINE: u8 = 38;
const CC_NRPN_LSB: u8 = 98;
const CC_NRPN_MSB: u8 = 99;
const CC_RPN_LSB: u8 = 100;
const CC_RPN_MSB: u8 = 101;

/// Whether a note starting at `note_tick` plays on a bend the file has left behind.
///
/// `last_bend_tick` is the tick of the channel's last pitch bend event, and `None` where it has had
/// none. One beat is the length of the gap: a bend moved within a beat of a note is that note's own
/// expression, and one left a beat or more before it is not.
pub fn is_stranded(
    note_tick: u32,
    bend: u16,
    last_bend_tick: Option<u32>,
    ticks_per_quarter: u16,
) -> bool {
    stranded_from(bend, last_bend_tick, ticks_per_quarter).is_some_and(|from| note_tick >= from)
}

/// The first tick at which a note would start on a stranded bend, or `None` where no note can.
///
/// [`is_stranded`] as one number per channel, for the sequencer: it runs in the audio callback,
/// where a tick is all the state it can afford to keep.
pub fn stranded_from(
    bend: u16,
    last_bend_tick: Option<u32>,
    ticks_per_quarter: u16,
) -> Option<u32> {
    match bend == BEND_CENTRE {
        true => None,
        false => last_bend_tick.map(|tick| tick.saturating_add(u32::from(ticks_per_quarter))),
    }
}

/// How far a bend value sits from centre, in either direction.
fn distance(bend: u16) -> u16 {
    bend.abs_diff(BEND_CENTRE)
}

/// One channel's pitch bend state while walking a song.
#[derive(Clone, Copy)]
struct Channel {
    bend: u16,
    last_bend_tick: Option<u32>,
    /// The furthest from centre any bend of the current movement went.
    peak: u16,
    /// Whether the current bend value finished a return, as the module header defines one.
    ended_return: bool,
    rpn: (u8, u8),
    registered_selected: bool,
    range_coarse: u8,
    range_fine: u8,
    /// The tick of the last note start counted as stranded, so a chord counts once.
    last_onset: Option<u32>,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            bend: BEND_CENTRE,
            last_bend_tick: None,
            peak: 0,
            ended_return: false,
            rpn: (0x7F, 0x7F),
            registered_selected: false,
            // The range General MIDI gives a channel that never sets one.
            range_coarse: 2,
            range_fine: 0,
            last_onset: None,
        }
    }
}

impl Channel {
    /// How far the current bend moves a note, in cents, whichever direction.
    fn offset_cents(&self) -> f32 {
        let range = f32::from(self.range_coarse) * 100.0 + f32::from(self.range_fine);
        (f32::from(self.bend) - f32::from(BEND_CENTRE)).abs() / f32::from(BEND_CENTRE) * range
    }
}

/// One note start on a bend a gesture left behind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stranded {
    /// The channel, 0-based.
    pub channel: u8,
    /// The tick the note starts on.
    pub tick: u32,
    /// The bend it starts on, centred on [`BEND_CENTRE`].
    pub bend: u16,
    /// How far that bend moves the note, in cents, at the channel's bend range.
    pub cents: f32,
    /// The tick of the channel's last bend event.
    pub last_bend_tick: u32,
}

/// How many separate note starts each channel plays on a stranded bend of at least `min_cents`.
///
/// Public so the census can sweep the threshold; [`detect`] is this at [`MIN_CENTS`].
pub fn stranded_onsets(song: &Song, min_cents: f32) -> [usize; CHANNELS] {
    let mut counts = [0usize; CHANNELS];
    for onset in stranded(song, min_cents) {
        counts[usize::from(onset.channel)] += 1;
    }
    counts
}

/// Every separate note start on a stranded bend of at least `min_cents`, in song order.
pub fn stranded(song: &Song, min_cents: f32) -> Vec<Stranded> {
    let mut channels = [Channel::default(); CHANNELS];
    let mut found = Vec::new();
    let beat = u32::from(song.ticks_per_quarter);
    for event in &song.events {
        let Some(state) = channels.get_mut(usize::from(event.kind.channel())) else {
            continue;
        };
        match event.kind {
            EventKind::PitchBend { value, .. } => {
                let continues = state
                    .last_bend_tick
                    .is_some_and(|tick| event.tick.saturating_sub(tick) <= beat);
                state.peak = match continues {
                    true => state.peak.max(distance(state.bend)),
                    false => 0,
                };
                state.ended_return = continues
                    && distance(value) < distance(state.bend)
                    && distance(value).saturating_mul(2) <= state.peak;
                state.bend = value;
                state.last_bend_tick = Some(event.tick);
            }
            EventKind::Controller {
                controller, value, ..
            } => match controller {
                CC_RESET_ALL_CONTROLLERS => {
                    state.bend = BEND_CENTRE;
                    state.ended_return = false;
                }
                CC_RPN_MSB => {
                    state.rpn.0 = value;
                    state.registered_selected = true;
                }
                CC_RPN_LSB => {
                    state.rpn.1 = value;
                    state.registered_selected = true;
                }
                CC_NRPN_MSB | CC_NRPN_LSB => state.registered_selected = false,
                CC_DATA_COARSE if state.registered_selected && state.rpn == (0, 0) => {
                    state.range_coarse = value;
                }
                CC_DATA_FINE if state.registered_selected && state.rpn == (0, 0) => {
                    state.range_fine = value;
                }
                _ => {}
            },
            EventKind::NoteOn {
                channel, velocity, ..
            } if velocity > 0 && channel != DRUM_CHANNEL => {
                if let Some(last_bend_tick) = state.last_bend_tick
                    && state.ended_return
                    && state.last_onset != Some(event.tick)
                    && is_stranded(
                        event.tick,
                        state.bend,
                        Some(last_bend_tick),
                        song.ticks_per_quarter,
                    )
                    && state.offset_cents() >= min_cents
                {
                    state.last_onset = Some(event.tick);
                    found.push(Stranded {
                        channel,
                        tick: event.tick,
                        bend: state.bend,
                        cents: state.offset_cents(),
                        last_bend_tick,
                    });
                }
            }
            _ => {}
        }
    }
    found
}

/// Every channel that plays at least [`MIN_ONSETS`] note starts on a bend of [`MIN_CENTS`] or more
/// that a gesture left behind.
pub fn detect(song: &Song) -> Vec<Fix> {
    stranded_onsets(song, MIN_CENTS)
        .iter()
        .enumerate()
        .filter(|(_, count)| **count >= MIN_ONSETS)
        .map(|(channel, _)| Fix::RecentreBend {
            channel: channel as u8,
        })
        .collect()
}

/// The log line this fix writes when a song starts.
pub fn describe(channel: u8) -> String {
    format!("channel {channel}: a bend left off centre is returned to centre at the next note")
}

#[cfg(test)]
mod tests {
    use km_song::{ParseOptions, Song, testing};

    use super::*;

    fn song(bytes: &[u8]) -> Song {
        Song::parse(bytes, &ParseOptions::default()).expect("fixture parses")
    }

    #[test]
    fn a_return_that_stops_short_is_flagged() {
        let song = song(&testing::bend_left_off_centre());
        assert_eq!(detect(&song), vec![Fix::RecentreBend { channel: 4 }]);
    }

    #[test]
    fn a_bend_returned_to_centre_is_left_alone() {
        // Channel 2 of the fixture makes the same gesture and finishes it.
        let song = song(&testing::bend_left_off_centre());
        assert!(!detect(&song).contains(&Fix::RecentreBend { channel: 2 }));
    }

    #[test]
    fn a_channel_detuned_once_on_purpose_is_left_alone() {
        // Channel 6 of the fixture sets one bend before its first note and never moves it.
        let song = song(&testing::bend_left_off_centre());
        assert!(!detect(&song).contains(&Fix::RecentreBend { channel: 6 }));
    }

    #[test]
    fn a_song_with_nothing_wrong_needs_no_fix() {
        let song = song(&testing::melody_and_accompaniment());
        assert!(detect(&song).is_empty());
    }

    #[test]
    fn the_shared_rule_needs_both_a_gap_and_an_offset() {
        assert!(is_stranded(960, 6784, Some(480), 480));
        assert!(!is_stranded(959, 6784, Some(480), 480));
        assert!(!is_stranded(960, BEND_CENTRE, Some(480), 480));
        assert!(!is_stranded(960, 6784, None, 480));
    }

    #[test]
    fn a_recentre_is_offered_rather_than_applied() {
        assert!(!Fix::RecentreBend { channel: 4 }.applies_itself());
    }
}
