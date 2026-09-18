//! The song page's corrections controls, and the three translations behind them.
//!
//! A song's stored list means *the corrections in force*. NULL means nobody has said, and a build
//! then takes whatever detection proposes — which is what lets a detector that learns a new defect
//! reach a corpus nobody is going to re-scan.
//!
//! **Detection runs here rather than at scan time, and that is the reason.** A scanned column would
//! freeze one afternoon's detector into hundreds of thousands of rows; parsing one file to draw one
//! song's page costs a few milliseconds and is always current.
//!
//! **One control, on the Advanced tab.** It carries the whole file: one row per channel, every
//! correction on it, and what the melody detector made of it. A save sends the whole list, so what
//! is written is always the complete set in force.
//!
//! **A correction that carries a value needs a control that has one.** A checkbox says on or off,
//! which is the whole of a mute and the whole of a suppressed bank select; an instrument is a choice
//! among a hundred and twenty-eight, so a re-voice is a select.

use std::path::Path;

use km_fixes::Fix;
use km_song::{ParseOptions, Song};
use km_suitability::{ChannelStats, MelodyEvidence, Thresholds};

/// One instrument in a re-voice select.
#[derive(Debug, Clone)]
pub struct ProgramOption {
    /// What the option posts back, `force_program:channel:program`, and empty for the first, which
    /// is the file's own instrument and so the absence of a fix.
    pub value: String,
    /// What it says on the page.
    pub name: String,
    /// Whether it is the one in force.
    pub selected: bool,
}

/// One checkbox in the channel table.
#[derive(Debug, Clone)]
pub struct Check {
    /// What the box posts back, `name:channel`.
    pub value: String,
    /// Whether it is in force now.
    pub checked: bool,
    /// Whether detection proposes it, which the cell says so a hint is not read as a fact.
    pub detected: bool,
}

/// One channel's row in the Advanced table.
#[derive(Debug, Clone)]
pub struct ChannelRow {
    /// The channel, 0-based.
    pub channel: u8,
    /// Whether this is channel 9, where a program change names a kit.
    pub drums: bool,
    /// What the file plays this channel on.
    pub instrument: String,
    /// How many notes it sounds, which is how a reader tells a part from a stray event.
    pub notes: u32,
    /// The tracks that put notes here, joined, for a file that names them.
    pub track: String,
    /// The melody detector's evidence as a percentage, empty where the channel was never weighed.
    pub melody: String,
    /// Whether that evidence is strong enough to read as an answer rather than as a candidate.
    ///
    /// The column is a set of numbers to compare, and above nine tenths there is nothing left to
    /// compare: colouring it says *this one* without a second column saying so in words.
    pub melody_strong: bool,
    /// Whether this is the channel detection settled on.
    pub found: bool,
    /// Whether this is the channel somebody chose, which is what the radio comes back ticked on.
    pub chosen: bool,
    /// Whether the drum channel can be the melody, which it cannot.
    ///
    /// Channel 9 sounds a kit rather than a part, so it is never a melody candidate and never gets a
    /// radio. Drawn as a field rather than read off [`Self::drums`] in the markup, because the two
    /// happen to agree and are not the same claim.
    pub melody_candidate: bool,
    /// The bank-select suppression, offered only where something proposes it or it is in force.
    pub bank: Option<Check>,
    /// The mute.
    pub mute: Check,
    /// Returning a bend the file left off centre, offered only where detection suggests it or it is
    /// in force. Its [`Check::detected`] means *suggested*: this fix never applies itself.
    pub recentre: Option<Check>,
    /// The instruments on offer, empty on the drum channel.
    pub programs: Vec<ProgramOption>,
}

/// What a file says about itself, or nothing where it cannot be read.
///
/// A file that has gone from disk is not an error: the page still draws, showing whatever is stored
/// and offering nothing new.
#[derive(Debug, Default)]
pub struct Analysis {
    /// The corrections detection proposes, which a build takes where nobody has said.
    pub detected: Vec<Fix>,
    /// The corrections detection suggests and a person must agree to, which a build never takes
    /// by itself.
    pub suggested: Vec<Fix>,
    /// Every channel that sounds a note.
    pub channels: Vec<ChannelStats>,
    /// Every non-drum channel weighed as a melody candidate, strongest first.
    pub melody: Vec<MelodyEvidence>,
}

/// Reads a stored list, treating an unreadable one as nobody having said.
pub fn stored(json: Option<&str>) -> Option<Vec<Fix>> {
    json.and_then(|text| serde_json::from_str(text).ok())
}

/// Writes a list back for storage.
pub fn encode(fixes: &[Fix]) -> String {
    serde_json::to_string(fixes).unwrap_or_else(|_| "[]".to_owned())
}

/// The one spelling of a fix a form posts, and the one it is read back from.
///
/// A fix carrying an argument spells it after the channel, which is what lets a select's options
/// post into the same field the checkboxes do.
fn spell(fix: &Fix) -> String {
    match fix {
        Fix::ForceProgram { channel, program } => {
            format!("{}:{channel}:{program}", fix.key())
        }
        _ => match fix.channel() {
            Some(channel) => format!("{}:{channel}", fix.key()),
            None => fix.key().to_owned(),
        },
    }
}

/// Turns one posted value back into a fix, ignoring anything this build does not offer.
///
/// Deliberately not a general parser: what is read here is what this page wrote, so a value that
/// does not match one of its own controls is a form that was tampered with rather than a newer
/// build's fix to carry on. An unknown fix is carried by [`selection`] from the stored list instead,
/// which no form value could reconstruct.
fn unspell(value: &str) -> Option<Fix> {
    let mut parts = value.split(':');
    let key = parts.next()?;
    let channel: u8 = parts.next()?.parse().ok()?;
    if usize::from(channel) >= km_fixes::CHANNELS {
        return None;
    }
    let argument = parts.next();
    if parts.next().is_some() {
        return None;
    }
    match (key, argument) {
        ("ignore_bank_select", None) => Some(Fix::IgnoreBankSelect { channel }),
        ("mute_channel", None) => Some(Fix::MuteChannel { channel }),
        ("recentre_bend", None) => Some(Fix::RecentreBend { channel }),
        ("force_program", Some(program)) => {
            let program: u8 = program.parse().ok()?;
            (u16::from(program) < km_fixes::force_program::PROGRAMS
                && km_fixes::force_program::allows(channel))
            .then_some(Fix::ForceProgram { channel, program })
        }
        _ => None,
    }
}

/// What a save should store, given the values that came back, what detection proposes, and what was
/// in force.
///
/// `None` means write NULL — the song goes back to being detected, which is what an untouched page
/// must leave behind. A list that merely restates the proposal is that case: storing it would pin
/// the song to today's detector for good, and a curator who changed nothing has said nothing.
///
/// **A fix this build cannot read is carried from the stored list rather than from the form.** No
/// control could offer one and no posted value could spell one, so a save that took only what came
/// back would delete a correction written by a newer build the first time somebody opened the page.
pub fn selection(posted: &[&str], detected: &[Fix], in_force: &[Fix]) -> Option<Option<String>> {
    let mut chosen: Vec<Fix> = in_force
        .iter()
        .filter(|fix| matches!(fix, Fix::Unknown(_)))
        .cloned()
        .chain(posted.iter().filter_map(|value| unspell(value)))
        .collect();
    chosen.sort_by_key(|fix| (fix.key(), fix.channel()));
    chosen.dedup();
    if chosen == detected {
        Some(None)
    } else {
        Some(Some(encode(&chosen)))
    }
}

/// What a person said the melody channel is, read from the column that stores it.
///
/// Three states and not two, for the reason [`selection`] gives about `fixes`: a column that could
/// only say *a channel* or *nothing* would make a song somebody had deliberately marked as having no
/// melody indistinguishable from one nobody had opened, and the next detector to learn something
/// would overrule the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MelodyChoice {
    /// Somebody said this song has no melody channel.
    None,
    /// Somebody named the channel, 0-based.
    Channel(u8),
}

impl MelodyChoice {
    /// The stored spelling, which is also what the form posts.
    pub fn as_str(self) -> String {
        match self {
            Self::None => "none".to_owned(),
            Self::Channel(channel) => channel.to_string(),
        }
    }

    /// Reads the column, or the posted field. Anything unrecognized is nobody having said.
    ///
    /// A channel outside the sixteen is refused the way the browse bar refuses a value it does not
    /// know: the answer is *no filter* rather than an empty page with no explanation, and here it is
    /// *nobody has said* rather than a page pointing at a channel that is not there.
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value?.trim() {
            "" => None,
            "none" => Some(Self::None),
            channel => channel
                .parse::<u8>()
                .ok()
                .filter(|channel| usize::from(*channel) < km_fixes::CHANNELS)
                .map(Self::Channel),
        }
    }

    /// The channel this choice names, which is nothing where it says there is none.
    pub fn channel(self) -> Option<u8> {
        match self {
            Self::None => Option::None,
            Self::Channel(channel) => Some(channel),
        }
    }
}

/// The melody channel a build should use: what somebody said, else what detection found.
pub fn melody_in_force(detected: Option<u8>, chosen: Option<MelodyChoice>) -> Option<u8> {
    match chosen {
        Some(choice) => choice.channel(),
        None => detected,
    }
}

/// What a save should store for the melody channel, given what came back and what detection found.
///
/// [`selection`]'s rule over one value: an answer that agrees with detection stores NULL, so opening
/// a song and pressing Save on the channel already shown records nothing and leaves that song still
/// learning from the detector. It is also the whole of *clear this*: picking the detected channel,
/// or picking *no melody* on a song detection abstained about, is how somebody hands the question
/// back.
pub fn melody_selection(posted: Option<&str>, detected: Option<u8>) -> Option<String> {
    let chosen = MelodyChoice::parse(posted)?;
    let agrees = match chosen {
        MelodyChoice::None => detected.is_none(),
        MelodyChoice::Channel(channel) => detected == Some(channel),
    };
    match agrees {
        true => None,
        false => Some(chosen.as_str()),
    }
}

/// Everything the song page asks of the file itself.
pub fn analyze(path: Option<&Path>) -> Analysis {
    let Some(song) = path
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| Song::parse(&bytes, &ParseOptions::default()).ok())
    else {
        return Analysis::default();
    };
    let thresholds = Thresholds::default();
    let channels = km_suitability::channel::measure(&song, &thresholds);
    let melody = km_suitability::melody::rank(&song, &channels, &thresholds);
    Analysis {
        detected: km_fixes::automatic(&song),
        suggested: km_fixes::suggested(&song),
        channels,
        melody,
    }
}

/// One row per channel, for the Advanced table.
///
/// Every channel that sounds, plus any channel carrying a correction that has since stopped
/// sounding: a mute nobody can see is a mute nobody can take off.
pub fn channel_rows(
    analysis: &Analysis,
    melody_channel: Option<u8>,
    chosen: Option<MelodyChoice>,
    in_force: &[Fix],
) -> Vec<ChannelRow> {
    let mut wanted: Vec<u8> = analysis
        .channels
        .iter()
        .map(|stats| stats.channel)
        .collect();
    for fix in in_force {
        if let Some(channel) = fix.channel()
            && !wanted.contains(&channel)
        {
            wanted.push(channel);
        }
    }
    // And the channel somebody named, for the reason a corrected channel is here: a choice nobody
    // can see is a choice nobody can take back. A channel that sounds nothing is an odd thing to
    // name and is exactly the sort of answer somebody would want to find again.
    if let Some(channel) = chosen.and_then(MelodyChoice::channel)
        && !wanted.contains(&channel)
    {
        wanted.push(channel);
    }
    wanted.sort_unstable();

    // What the radios come back ticked on: what somebody said, else what detection found. Where the
    // two agree nothing is stored, so this is one answer rather than two marks competing.
    let in_force_melody = melody_in_force(melody_channel, chosen);

    wanted
        .into_iter()
        .map(|channel| {
            let stats = analysis
                .channels
                .iter()
                .find(|stats| stats.channel == channel);
            let drums = channel == km_fixes::DRUM_CHANNEL;
            let evidence = analysis.melody.iter().find(|row| row.channel == channel);
            let bank = Fix::IgnoreBankSelect { channel };
            let offer_bank = analysis.detected.contains(&bank) || in_force.contains(&bank);
            let recentre = Fix::RecentreBend { channel };
            let offer_recentre =
                analysis.suggested.contains(&recentre) || in_force.contains(&recentre);
            ChannelRow {
                channel,
                drums,
                instrument: instrument_of(analysis, channel),
                notes: stats.map_or(0, |stats| stats.note_count),
                track: stats
                    .map(|stats| stats.track_names.join(", "))
                    .unwrap_or_default(),
                melody: evidence
                    .map(|row| format!("{}%", (row.evidence * 100.0).round() as u32))
                    .unwrap_or_default(),
                melody_strong: evidence.is_some_and(|row| row.evidence > 0.9),
                found: melody_channel == Some(channel),
                chosen: in_force_melody == Some(channel),
                // Every channel that is not the drum channel, whether or not the detector weighed
                // it: a part it gave up on is exactly the one somebody opens this tab to name.
                melody_candidate: !drums,
                bank: offer_bank.then(|| Check {
                    value: spell(&bank),
                    checked: in_force.contains(&bank),
                    detected: analysis.detected.contains(&bank),
                }),
                mute: Check {
                    value: spell(&Fix::MuteChannel { channel }),
                    checked: in_force.contains(&Fix::MuteChannel { channel }),
                    detected: analysis.detected.contains(&Fix::MuteChannel { channel }),
                },
                recentre: offer_recentre.then(|| Check {
                    value: spell(&recentre),
                    checked: in_force.contains(&recentre),
                    detected: analysis.suggested.contains(&recentre),
                }),
                programs: match drums {
                    true => Vec::new(),
                    false => program_options(channel, forced(in_force, channel)),
                },
            }
        })
        .collect()
}

/// The forced program in force on one channel.
fn forced(in_force: &[Fix], channel: u8) -> Option<u8> {
    in_force.iter().find_map(|fix| match fix {
        Fix::ForceProgram {
            channel: on,
            program,
        } if *on == channel => Some(*program),
        _ => None,
    })
}

/// What the file itself plays a channel on.
///
/// The first program selected, because that is what the channel starts on and what a person judging
/// a re-voice is hearing. A channel the file changes later says so rather than pretending the first
/// is the whole story.
fn instrument_of(analysis: &Analysis, channel: u8) -> String {
    let Some(stats) = analysis
        .channels
        .iter()
        .find(|stats| stats.channel == channel)
    else {
        return String::new();
    };
    if stats.channel == km_fixes::DRUM_CHANNEL {
        return "a drum kit".to_owned();
    }
    match stats.programs.split_first() {
        None => "the file never says".to_owned(),
        Some((first, [])) => km_fixes::force_program::program_name(*first),
        Some((first, _)) => format!(
            "{}, and changes later",
            km_fixes::force_program::program_name(*first)
        ),
    }
}

/// The instruments a channel may be given, the file's own first.
fn program_options(channel: u8, chosen: Option<u8>) -> Vec<ProgramOption> {
    let mut programs = vec![ProgramOption {
        value: String::new(),
        name: "the instrument in the file".to_owned(),
        selected: chosen.is_none(),
    }];
    programs.extend((0..km_fixes::force_program::PROGRAMS).map(|program| {
        let program = program as u8;
        ProgramOption {
            value: spell(&Fix::ForceProgram { channel, program }),
            name: km_fixes::force_program::program_name(program),
            selected: chosen == Some(program),
        }
    }));
    programs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bank() -> Fix {
        Fix::IgnoreBankSelect { channel: 4 }
    }

    fn mute() -> Fix {
        Fix::MuteChannel { channel: 2 }
    }

    /// A file with notes on two melodic channels and the drum channel, named so the melody detector
    /// has something to weigh.
    /// The analysis of one fixture, read through a scratch directory of this thread's own.
    ///
    /// **`Scratch` and not a path in the temp folder**, which is the hazard that module was written
    /// to end and this helper is the last place holding it: eight tests here call this and cargo
    /// runs them in parallel, so a fixed file name is one thread deleting the file another is
    /// reading. `Scratch` keys its directory on the process and the thread, which is what makes the
    /// name unshared in both directions.
    fn analysis() -> Analysis {
        let scratch = crate::testing::Scratch::new("fixes-analysis");
        scratch.write(
            "analysis.mid",
            &km_song::testing::melody_and_accompaniment(),
        );
        analyze(Some(&scratch.0.join("analysis.mid")))
    }

    #[test]
    fn a_page_left_alone_stores_nothing() {
        // The rule the whole column depends on. A curator who opened a song and pressed Save has
        // said nothing, and pinning the song to today's detector would be putting words in their
        // mouth.
        assert_eq!(
            selection(&["ignore_bank_select:4"], &[bank()], &[bank()]),
            Some(None)
        );
    }

    #[test]
    fn unticking_the_only_proposal_is_a_decision() {
        assert_eq!(
            selection(&[], &[bank()], &[bank()]),
            Some(Some("[]".to_owned()))
        );
    }

    #[test]
    fn adding_a_mute_keeps_the_proposal_beside_it() {
        let stored = selection(&["ignore_bank_select:4", "mute_channel:2"], &[bank()], &[])
            .expect("a decision")
            .expect("a list");
        assert_eq!(super::stored(Some(&stored)), Some(vec![bank(), mute()]));
    }

    #[test]
    fn a_value_this_build_does_not_offer_is_dropped() {
        assert_eq!(
            selection(
                &["mute_channel:99", "invented:1", "mute_channel:2"],
                &[],
                &[]
            ),
            Some(Some(encode(&[mute()])))
        );
    }

    #[test]
    fn an_instrument_the_wire_cannot_carry_is_dropped() {
        // Against a proposal, so that dropping the value is visible: an empty list beside a
        // proposal is a decision, where an empty list beside nothing is silence.
        let dropped = Some(Some("[]".to_owned()));
        assert_eq!(selection(&["force_program:2:128"], &[bank()], &[]), dropped);
        assert_eq!(
            selection(
                &[&format!("force_program:{}:52", km_fixes::DRUM_CHANNEL)],
                &[bank()],
                &[]
            ),
            dropped
        );
        assert_eq!(selection(&["force_program:2"], &[bank()], &[]), dropped);
        assert_eq!(
            selection(&["force_program:2:52:1"], &[bank()], &[]),
            dropped
        );
    }

    #[test]
    fn a_fix_this_build_cannot_read_survives_a_save() {
        // Nothing on the page can offer one and no posted value can spell one, so the stored list is
        // the only place it can come from.
        let unknown: Fix =
            serde_json::from_str(r#"{"fix":"swap_channels","from":3,"to":11}"#).expect("reads");
        let in_force = vec![unknown.clone(), mute()];
        let stored = selection(&["mute_channel:2"], &[], &in_force)
            .expect("a decision")
            .expect("a list");
        assert_eq!(super::stored(Some(&stored)), Some(vec![mute(), unknown]));
    }

    #[test]
    fn every_sounding_channel_gets_a_row_and_the_drum_channel_takes_no_instrument() {
        let analysis = analysis();
        let rows = channel_rows(&analysis, Some(0), None, &[]);
        assert!(!rows.is_empty());
        assert!(
            rows.windows(2)
                .all(|pair| pair[0].channel < pair[1].channel)
        );
        for row in &rows {
            assert_eq!(row.programs.is_empty(), row.drums);
            assert!(row.mute.value.starts_with("mute_channel:"));
        }
    }

    #[test]
    fn a_channel_that_has_stopped_sounding_keeps_its_row() {
        let rows = channel_rows(&analysis(), None, None, &[Fix::MuteChannel { channel: 13 }]);
        let row = rows
            .iter()
            .find(|row| row.channel == 13)
            .expect("the muted channel is still offered");
        assert!(row.mute.checked);
        assert_eq!(row.notes, 0);
    }

    #[test]
    fn the_bank_suppression_is_offered_where_something_says_so_and_not_otherwise() {
        let rows = channel_rows(&analysis(), None, None, &[bank()]);
        assert!(
            rows.iter()
                .find(|row| row.channel == 4)
                .is_some_and(|row| row.bank.as_ref().is_some_and(|bank| bank.checked))
        );
        assert!(
            rows.iter()
                .filter(|row| row.channel != 4)
                .all(|row| row.bank.is_none())
        );
    }

    #[test]
    fn a_suggested_recentre_is_offered_unticked_and_a_tick_is_stored() {
        let scratch = crate::testing::Scratch::new("fixes-recentre");
        scratch.write("bend.mid", &km_song::testing::bend_left_off_centre());
        let analysis = analyze(Some(&scratch.0.join("bend.mid")));
        let recentre = Fix::RecentreBend { channel: 4 };
        assert!(analysis.suggested.contains(&recentre));
        assert!(!analysis.detected.contains(&recentre));

        let rows = channel_rows(&analysis, None, None, &analysis.detected);
        let offered = rows
            .iter()
            .find(|row| row.channel == 4)
            .and_then(|row| row.recentre.as_ref())
            .expect("offered on the channel detection suggests");
        assert!(!offered.checked && offered.detected);
        assert!(
            rows.iter()
                .filter(|row| row.channel != 4)
                .all(|row| row.recentre.is_none())
        );

        let stored = selection(&[&offered.value], &analysis.detected, &analysis.detected)
            .expect("a decision")
            .expect("a list");
        assert_eq!(super::stored(Some(&stored)), Some(vec![recentre]));
    }

    /// The evidence is a number to compare, and the channel detection settled on is marked.
    ///
    /// Nothing names the strongest candidate in words: the column is a set of percentages read
    /// against each other, and a second column saying which is largest repeats what they already
    /// say. What the numbers cannot say is which one detection *took*, so that is the mark.
    #[test]
    fn the_melody_column_is_evidence_and_the_one_found_is_marked() {
        let analysis = analysis();
        let rows = channel_rows(&analysis, Some(0), None, &[]);
        assert!(rows.iter().any(|row| row.found && row.channel == 0));
        assert_eq!(
            rows.iter().filter(|row| row.found).count(),
            1,
            "detection settles on one channel"
        );
        assert!(
            rows.iter()
                .find(|row| row.channel == 0)
                .is_some_and(|row| row.melody.ends_with('%'))
        );
        assert!(
            rows.iter()
                .find(|row| row.drums)
                .is_some_and(|row| row.melody.is_empty()),
            "the drum channel is never weighed"
        );
        assert!(
            rows.iter()
                .find(|row| row.drums)
                .is_some_and(|row| !row.melody_candidate),
            "and so is never offered as the melody"
        );
    }

    /// The radios come back on the answer in force, which is a choice where there is one.
    #[test]
    fn a_chosen_channel_is_what_the_radios_are_ticked_on() {
        let analysis = analysis();

        // Nobody has said, so the radios follow detection.
        let rows = channel_rows(&analysis, Some(0), None, &[]);
        let ticked = |rows: &[ChannelRow]| -> Vec<u8> {
            rows.iter()
                .filter(|row| row.chosen)
                .map(|row| row.channel)
                .collect()
        };
        assert_eq!(ticked(&rows), [0]);

        // Somebody disagreed. The mark saying what detection *found* stays where it was: the two
        // are different claims, and a page that moved both would lose the disagreement it exists to
        // show.
        let rows = channel_rows(&analysis, Some(0), Some(MelodyChoice::Channel(2)), &[]);
        assert_eq!(ticked(&rows), [2]);
        assert!(rows.iter().any(|row| row.found && row.channel == 0));

        // Somebody said there is none, so no row is ticked and the head's radio carries it.
        let rows = channel_rows(&analysis, Some(0), Some(MelodyChoice::None), &[]);
        assert!(ticked(&rows).is_empty());
    }

    /// Saving the answer already shown stores nothing, which is what *clear this* is made of.
    #[test]
    fn a_melody_that_agrees_with_detection_stores_nothing() {
        // The rule `selection` holds for the fix list, over one value: a curator who opened a song
        // and pressed Save has said nothing, and storing it would pin the song to today's detector.
        assert_eq!(melody_selection(Some("3"), Some(3)), None);
        // Picking the detected channel back is therefore the whole of handing the question over
        // again — there is no separate control to clear it with.
        assert_eq!(melody_selection(Some("5"), Some(3)), Some("5".to_owned()));
        assert_eq!(melody_selection(Some("3"), Some(5)), Some("3".to_owned()));

        // *No melody* is a decision where detection found one, and an agreement where it did not.
        assert_eq!(
            melody_selection(Some("none"), Some(3)),
            Some("none".to_owned())
        );
        assert_eq!(melody_selection(Some("none"), None), None);
        // And naming a channel on a song detection abstained about is the case the control exists
        // for: the machine offers no guide melody at all until somebody says where it is.
        assert_eq!(melody_selection(Some("7"), None), Some("7".to_owned()));

        // Nothing posted leaves the column alone, which is what the Details form's save must do.
        assert_eq!(melody_selection(None, Some(3)), None);
    }

    /// A value the page could not have written is nobody having said, not a page pointing nowhere.
    #[test]
    fn an_unreadable_melody_choice_is_read_as_nobody_having_said() {
        assert_eq!(
            MelodyChoice::parse(Some("4")),
            Some(MelodyChoice::Channel(4))
        );
        assert_eq!(MelodyChoice::parse(Some("none")), Some(MelodyChoice::None));
        for nonsense in ["", "16", "255", "-1", "yes", "channel 4"] {
            assert_eq!(MelodyChoice::parse(Some(nonsense)), None, "{nonsense}");
        }
        assert_eq!(MelodyChoice::parse(None), None);

        // The round trip the column depends on: what is stored is what comes back.
        for choice in [
            MelodyChoice::None,
            MelodyChoice::Channel(0),
            MelodyChoice::Channel(15),
        ] {
            assert_eq!(MelodyChoice::parse(Some(&choice.as_str())), Some(choice));
        }
    }

    /// What a build uses: the answer somebody gave, else the one detection found.
    #[test]
    fn a_choice_stands_over_detection_and_silence_leaves_it() {
        assert_eq!(melody_in_force(Some(3), None), Some(3));
        assert_eq!(
            melody_in_force(Some(3), Some(MelodyChoice::Channel(7))),
            Some(7)
        );
        assert_eq!(melody_in_force(Some(3), Some(MelodyChoice::None)), None);
        assert_eq!(
            melody_in_force(None, Some(MelodyChoice::Channel(7))),
            Some(7)
        );
        assert_eq!(melody_in_force(None, None), None);
    }
}
