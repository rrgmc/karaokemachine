//! The song-number keypad.
//!
//! A commercial machine is driven by typing a number and pressing enter, and that has to keep
//! working when the only input is a remote's digit keys. Pure state, so the awkward cases —
//! overlong input, a number that does not exist, a stale error still on screen — are testable.
//!
//! **The song's name appears while it is being dialled**, which is how somebody catches a wrong
//! number before queueing another table's song. The catalog lives on the other side of this crate,
//! so the answer is pushed in through [`NumberEntry::set_lookup`] rather than fetched — the same
//! seam `SongInfo::language` uses, and the reason `km-display` needs no catalog dependency.
//!
//! **A number nobody has draws nothing at all while typing.** `1`, `10` and `102` on the way to
//! `10234` are legitimately not songs, so treating them as mistakes would put an error on screen for
//! most of every entry and teach the singer to ignore the line. The `no song NNNN` alert belongs on
//! *submit*, where somebody has actually asked for it, and that is where it stayed.
//!
//! **A message goes away by itself; the digits do not.** The two halves of this line look alike and
//! are the opposite case from each other. A message is something the machine said, so nobody is
//! waiting to act on it and leaving it up costs the prompt — one sat over an idle screen for a whole
//! evening before [`NumberEntry::tick`] existed, because the only things that cleared it were key
//! presses nobody had a reason to make. Half-typed digits are something a *person* said and that
//! person is standing there, so a number that vanished under their hand would be the worse machine.

use std::time::Duration;

use km_songcode::{MAX_DIGITS, SongCode};

/// How long a message stays on the keypad line.
///
/// Longer than the display loop's `FLASH_DONE`, which reports a result to notice, and shorter than
/// its `FLASH_FAILED`, which is the only account anybody gets of why the songs are not there. A
/// keypad message is a reason to read like the second, but unlike either of them it is sitting *in*
/// the prompt — the flash band has a row of its own and can afford to linger.
pub const MESSAGE_LINGER: Duration = Duration::from_secs(8);

/// The song behind what has been typed, for showing while somebody dials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongPreview {
    /// The song's title.
    pub title: String,
    /// The performer, when the catalog has one.
    pub artist: Option<String>,
}

/// A catalog answer, together with the input it answers.
///
/// The key is what makes a stale preview impossible rather than something every mutator has to
/// remember to clear: an answer resolves only while it still describes what is on screen, so
/// `push_digit`, `backspace`, `clear`, `submit` and `show_message` need no invalidation at all.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lookup {
    key: String,
    /// `None` means *looked up, and there is no such song*. That is not an error and draws nothing —
    /// it exists so a number nobody has is not asked about again on every frame.
    song: Option<SongPreview>,
}

/// A message on the keypad line, and how much of its time is left.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    text: String,
    /// Counted down by [`NumberEntry::tick`] rather than compared against a clock, which is what
    /// keeps this module free of `Instant::now()` and its tests free of sleeping. The same shape
    /// [`crate::wallpaper::Schedule`] uses, and the same reason.
    remaining: Duration,
}

impl Message {
    /// A message with its full time ahead of it.
    ///
    /// Every route to the line goes through here, so [`MESSAGE_LINGER`] is stated once. The
    /// alternative — a deadline each caller passes in — is the obligation this module's doc argues
    /// against for stale previews, and there are eight callers.
    fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            remaining: MESSAGE_LINGER,
        }
    }
}

/// What the keypad is showing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NumberEntry {
    digits: String,
    /// A message shown instead of the digits, such as an unknown number.
    message: Option<Message>,
    /// The catalog's answer for some earlier state of the input, which may no longer be current.
    lookup: Option<Lookup>,
}

/// What the caller should do after a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberAction {
    /// Nothing to do; only the display changed.
    None,
    /// Look this code up and queue it.
    Submit(SongCode),
}

impl NumberEntry {
    /// An empty keypad.
    pub fn new() -> Self {
        Self::default()
    }

    /// The digits typed so far.
    pub fn digits(&self) -> &str {
        &self.digits
    }

    /// A message to show in place of the digits, if any.
    pub fn message(&self) -> Option<&str> {
        self.message.as_ref().map(|message| message.text.as_str())
    }

    /// Ages a message by one frame, and drops it once its time is up.
    ///
    /// **The clock is a delta rather than an `Instant`**, so this is exact under a test and needs no
    /// sleeping, and a caller that draws without a loop — the contact sheet, an off-screen render —
    /// simply never calls it and keeps whatever it set.
    ///
    /// The digits are deliberately untouched: see the module doc for why the two halves of this line
    /// get opposite answers.
    pub fn tick(&mut self, delta: Duration) {
        if let Some(message) = &mut self.message {
            message.remaining = message.remaining.saturating_sub(delta);
            if message.remaining.is_zero() {
                self.message = None;
            }
        }
    }

    /// Everything typed, as the one string a song is asked for by.
    ///
    /// This is what a lookup is keyed on and what `submit` parses, so an answer that arrives for an
    /// earlier state of the input stops resolving by itself. It is the digits and nothing else now
    /// — it used to prepend a package prefix, and the indirection is kept because it is what makes a
    /// stale preview structurally impossible rather than something five mutators must each clear.
    pub fn key(&self) -> &str {
        &self.digits
    }

    /// What still needs a catalog answer, if anything.
    ///
    /// `None` once the current input has been answered, which is what keeps the caller from
    /// querying on every frame — the answer is asked for once per key press and then remembered.
    pub fn pending_lookup(&self) -> Option<String> {
        if self.digits.is_empty() || self.message.is_some() {
            return None;
        }
        let key = self.key();
        match &self.lookup {
            Some(lookup) if lookup.key == key => None,
            _ => Some(key.to_owned()),
        }
    }

    /// Records what the catalog said about `key`.
    ///
    /// An answer for anything but the current input is **dropped**: a lookup that arrives after two
    /// more digits have been typed describes a song nobody is asking for, and drawing it would be
    /// worse than drawing nothing.
    pub fn set_lookup(&mut self, key: &str, song: Option<SongPreview>) {
        if key != self.key() {
            return;
        }
        self.lookup = Some(Lookup {
            key: key.to_owned(),
            song,
        });
    }

    /// The song behind what is on screen, when it has been looked up and there is one.
    pub fn preview(&self) -> Option<&SongPreview> {
        let lookup = self.lookup.as_ref()?;
        if lookup.key != self.key() {
            return None;
        }
        lookup.song.as_ref()
    }

    /// Whether anything is being typed or shown.
    ///
    /// `docs/decisions/interface.md`'s `Leaving the app` decision requires every press of BACK to do something
    /// visible, so this has to be true for anything that is on screen.
    pub fn is_active(&self) -> bool {
        !self.digits.is_empty() || self.message.is_some()
    }

    /// Appends a digit, ignoring anything past the limit.
    pub fn push_digit(&mut self, digit: char) -> NumberAction {
        if !digit.is_ascii_digit() {
            return NumberAction::None;
        }
        // Typing clears a stale message, so an error never sits over live input.
        self.message = None;
        if self.digits.len() < MAX_DIGITS {
            self.digits.push(digit);
        }
        NumberAction::None
    }

    /// Removes the last digit, or dismisses a message.
    pub fn backspace(&mut self) -> NumberAction {
        if self.message.take().is_some() {
            return NumberAction::None;
        }
        self.digits.pop();
        NumberAction::None
    }

    /// Clears everything. `CLR` means start over; backspace is what deletes one digit.
    pub fn clear(&mut self) -> NumberAction {
        self.digits.clear();
        self.message = None;
        NumberAction::None
    }

    /// Submits what has been typed.
    ///
    /// Takes a locale because the one thing it can say is a refusal, and a refusal is read by
    /// somebody standing at the machine.
    pub fn submit(&mut self, locale: km_locale::Locale) -> NumberAction {
        if self.digits.is_empty() {
            return NumberAction::None;
        }
        match self.key().parse::<SongCode>() {
            Ok(code) => {
                self.digits.clear();
                self.message = None;
                NumberAction::Submit(code)
            }
            // Reachable in one ordinary way: `0`, and `000`, which a keypad will happily accept and
            // which is not a song number. Failing silently here would be a keypad that ignores the
            // enter key.
            Err(_) => {
                self.digits.clear();
                self.message = Some(Message::new(
                    crate::words::messages(locale).msg(crate::words::NUMBER_INVALID),
                ));
                NumberAction::None
            }
        }
    }

    /// Shows a message, which is always something that went wrong.
    ///
    /// **Failures only, and that is what lets the prompt draw this in one color.** It is painted
    /// `theme.alert` with no kind to choose by, so a caller passing good news here gets it in the
    /// color of bad news, which is why a queued song's title does not come through this line. A
    /// success has the screen itself to speak with: the song starts, or it shows as `next: …`.
    ///
    /// It stays for [`MESSAGE_LINGER`] and then goes, so nothing has to remember to take it away —
    /// which is just as well, because the only things that ever did were key presses.
    pub fn show_message(&mut self, message: impl Into<String>) {
        self.digits.clear();
        self.message = Some(Message::new(message));
    }
}

#[cfg(test)]
mod tests {
    use km_locale::Locale;

    use super::*;

    const EN: Locale = Locale::English;

    #[test]
    fn digits_accumulate() {
        let mut entry = NumberEntry::new();
        for digit in "1234".chars() {
            assert_eq!(entry.push_digit(digit), NumberAction::None);
        }
        assert_eq!(entry.digits(), "1234");
        assert!(entry.is_active());
    }

    #[test]
    fn non_digits_are_ignored() {
        let mut entry = NumberEntry::new();
        entry.push_digit('a');
        entry.push_digit('-');
        assert_eq!(entry.digits(), "");
        assert!(!entry.is_active());
    }

    /// Taken from `MAX_DIGITS` rather than spelled out, so widening a bank does not silently leave
    /// the keypad a digit short of the numbers the catalog can now hold.
    #[test]
    fn input_stops_at_the_digit_limit() {
        let mut entry = NumberEntry::new();
        for digit in "1234567890".chars() {
            entry.push_digit(digit);
        }
        assert_eq!(entry.digits(), &"1234567890"[..MAX_DIGITS]);
        assert_eq!(
            km_songcode::MAX_NUMBER.to_string().len(),
            MAX_DIGITS,
            "the keypad must reach every number the catalog can hold"
        );
    }

    #[test]
    fn submitting_yields_the_number_and_resets() {
        let mut entry = NumberEntry::new();
        for digit in "10234".chars() {
            entry.push_digit(digit);
        }
        assert_eq!(
            entry.submit(EN),
            NumberAction::Submit(SongCode::new(10_234))
        );
        assert_eq!(entry.digits(), "");
        assert!(!entry.is_active());
    }

    #[test]
    fn submitting_nothing_does_nothing() {
        let mut entry = NumberEntry::new();
        assert_eq!(entry.submit(EN), NumberAction::None);
    }

    #[test]
    fn leading_zeros_are_accepted() {
        let mut entry = NumberEntry::new();
        for digit in "007".chars() {
            entry.push_digit(digit);
        }
        assert_eq!(entry.submit(EN), NumberAction::Submit(SongCode::new(7)));
    }

    #[test]
    fn backspace_removes_one_digit() {
        let mut entry = NumberEntry::new();
        entry.push_digit('1');
        entry.push_digit('2');
        entry.backspace();
        assert_eq!(entry.digits(), "1");
    }

    #[test]
    fn a_message_is_cleared_by_typing_rather_than_sitting_over_the_input() {
        let mut entry = NumberEntry::new();
        entry.show_message("no song 9999999");
        assert_eq!(entry.message(), Some("no song 9999999"));

        entry.push_digit('1');
        assert_eq!(entry.message(), None);
        assert_eq!(entry.digits(), "1");
    }

    #[test]
    fn backspace_dismisses_a_message() {
        let mut entry = NumberEntry::new();
        entry.push_digit('5');
        entry.show_message("not found");
        entry.backspace();
        assert_eq!(entry.message(), None);
        // show_message cleared the digits, so there is nothing left to delete.
        assert_eq!(entry.digits(), "");
    }

    fn preview(title: &str) -> SongPreview {
        SongPreview {
            title: title.to_owned(),
            artist: Some("Somebody".to_owned()),
        }
    }

    #[test]
    fn a_preview_shows_for_the_digits_it_was_looked_up_against() {
        let mut entry = NumberEntry::new();
        entry.push_digit('7');
        assert_eq!(entry.pending_lookup().as_deref(), Some("7"));

        entry.set_lookup("7", Some(preview("Seven")));
        assert_eq!(
            entry.preview().map(|song| song.title.as_str()),
            Some("Seven")
        );
        // Answered, so nothing more to ask until the input changes.
        assert_eq!(entry.pending_lookup(), None);
    }

    #[test]
    fn typing_another_digit_hides_the_preview_until_it_is_answered_again() {
        let mut entry = NumberEntry::new();
        entry.push_digit('7');
        entry.set_lookup("7", Some(preview("Seven")));

        entry.push_digit('0');
        assert_eq!(entry.preview(), None, "the preview described 7, not 70");
        assert_eq!(entry.pending_lookup().as_deref(), Some("70"));
    }

    #[test]
    fn backspacing_back_to_an_answered_number_asks_again_rather_than_reviving_a_stale_answer() {
        let mut entry = NumberEntry::new();
        entry.push_digit('7');
        entry.set_lookup("7", Some(preview("Seven")));
        entry.push_digit('0');
        entry.set_lookup("70", Some(preview("Seventy")));

        entry.backspace();
        // The answer on hand describes "70"; "7" has to be asked for again.
        assert_eq!(entry.preview(), None);
        assert_eq!(entry.pending_lookup().as_deref(), Some("7"));
    }

    #[test]
    fn an_answer_for_other_digits_is_dropped() {
        let mut entry = NumberEntry::new();
        entry.push_digit('1');
        entry.push_digit('2');
        // A lookup for "1" arriving after "2" was typed describes a song nobody asked for.
        entry.set_lookup("1", Some(preview("One")));
        assert_eq!(entry.preview(), None);
        assert_eq!(entry.pending_lookup().as_deref(), Some("12"));
    }

    #[test]
    fn a_number_nobody_has_is_answered_once_and_shows_nothing() {
        let mut entry = NumberEntry::new();
        entry.push_digit('9');
        entry.set_lookup("9", None);

        assert_eq!(entry.preview(), None, "a miss while typing is not an error");
        assert_eq!(entry.message(), None);
        // Answered, so it is not asked about again on every frame — the reason `None` is recorded.
        assert_eq!(entry.pending_lookup(), None);
    }

    #[test]
    fn a_message_and_a_submit_both_stop_a_preview_resolving() {
        let mut entry = NumberEntry::new();
        entry.push_digit('7');
        entry.set_lookup("7", Some(preview("Seven")));
        entry.show_message("no song 7");
        assert_eq!(entry.preview(), None);
        assert_eq!(
            entry.pending_lookup(),
            None,
            "a message is not an input to look up"
        );

        let mut entry = NumberEntry::new();
        entry.push_digit('7');
        entry.set_lookup("7", Some(preview("Seven")));
        entry.submit(EN);
        assert_eq!(entry.preview(), None);
        assert_eq!(entry.pending_lookup(), None);
    }

    #[test]
    fn nothing_typed_is_nothing_to_look_up() {
        let entry = NumberEntry::new();
        assert_eq!(entry.pending_lookup(), None);
        assert_eq!(entry.preview(), None);
    }

    #[test]
    fn clearing_resets_everything() {
        let mut entry = NumberEntry::new();
        entry.push_digit('9');
        entry.show_message("something");
        entry.clear();
        assert!(!entry.is_active());
        assert_eq!(entry.message(), None);
    }

    /// A frame at sixty, which is what the display loop hands `tick`.
    const FRAME: Duration = Duration::from_millis(16);

    #[test]
    fn a_message_goes_away_by_itself() {
        let mut entry = NumberEntry::new();
        entry.show_message("no melody channel was detected for this song");

        entry.tick(MESSAGE_LINGER - FRAME);
        assert!(
            entry.message().is_some(),
            "still inside its time, so still on screen"
        );

        entry.tick(FRAME);
        assert_eq!(entry.message(), None);
        assert!(
            !entry.is_active(),
            "and BACK has nothing left to dismiss, so it means what it did before the message"
        );
    }

    #[test]
    fn a_message_survives_a_long_stall_of_one_frame_no_better_than_many() {
        // A delta far past the deadline arrives after a slow load or a resize, and saturating
        // arithmetic is what keeps that from wrapping into another eight seconds on screen.
        let mut entry = NumberEntry::new();
        entry.show_message("no song 9999999");
        entry.tick(Duration::from_secs(3_600));
        assert_eq!(entry.message(), None);
    }

    #[test]
    fn ticking_does_not_age_the_digits() {
        let mut entry = NumberEntry::new();
        for digit in "102".chars() {
            entry.push_digit(digit);
        }
        entry.tick(MESSAGE_LINGER * 4);
        assert_eq!(
            entry.digits(),
            "102",
            "somebody mid-dial is standing there; the number is theirs, not the machine's"
        );
        assert!(entry.is_active());
    }

    #[test]
    fn a_second_message_gets_its_own_full_time() {
        let mut entry = NumberEntry::new();
        entry.show_message("nothing is playing");
        entry.tick(MESSAGE_LINGER - FRAME);

        entry.show_message("no melody channel was detected for this song");
        entry.tick(MESSAGE_LINGER - FRAME);
        assert_eq!(
            entry.message(),
            Some("no melody channel was detected for this song"),
            "the second message inherited none of the first's spent time"
        );
    }

    #[test]
    fn ticking_an_empty_keypad_does_nothing() {
        let mut entry = NumberEntry::new();
        entry.tick(MESSAGE_LINGER);
        assert!(!entry.is_active());
        assert_eq!(entry.message(), None);
    }
}
