//! Keys to actions.
//!
//! Kept as a pure mapping so the bindings are testable and stated in one place rather than buried in
//! an event loop. The machine is meant to be driven by a numeric remote, so digits and enter carry
//! the weight; the letter keys are for whoever is standing at the machine itself.

use sdl3::keyboard::Keycode;

/// How far one press or one key moves the position within a song, in seconds.
///
/// Ten seconds is a verse-ish jump: long enough to be worth pressing, short enough that two presses
/// are still a correction rather than a guess. A karaoke song is three minutes, so this is a
/// twentieth of it.
pub const SEEK_STEP_SECS: i32 = 10;

/// Something the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayAction {
    /// A digit was typed on the keypad.
    Digit(char),
    /// Queue whatever number has been typed.
    Submit,
    /// Delete the last digit.
    Backspace,
    /// Clear the keypad.
    Clear,
    /// Start or pause playback.
    TogglePause,
    /// Leave the current song, and let the machine choose what follows.
    ///
    /// **What follows is the machine's to decide, and a queue is only its most ordinary answer.**
    /// This crate cannot see one, so naming the next song here would be a claim it has no way to
    /// make.
    Skip,
    /// Restart the current song.
    Restart,
    /// Move the position within the current song, in seconds, forwards or back.
    ///
    /// Not a *skip*: `Skip` leaves the song and this stays in it. The distinction matters on the
    /// transport strip, where a key labeled ambiguously would lose somebody the rest of their song
    /// mid-verse. The caller clamps to the song's length — the display knows the delta, and only the
    /// machine knows where the song ends.
    SeekBy(i32),
    /// Raise the key by a semitone.
    TransposeUp,
    /// Lower the key by a semitone.
    TransposeDown,
    /// Reset the key to the written one.
    TransposeReset,
    /// Turn the guide melody on or off.
    ToggleMelody,
    /// Move to the next wallpaper now.
    NextWallpaper,
    /// Show or hide the connect panel.
    ToggleConnect,
    /// Show or hide the queue.
    ToggleQueue,
    /// Turn demo mode on or off, and ask for a song when it goes on.
    ///
    /// **`D`, a bare letter, because the function row has nothing left and nothing to pair with.**
    /// The two modified keys here that are not debugging controls are carried by what they pair
    /// with — `F10` shows you where songs go and `Ctrl+F10` picks up what you put there; `Ctrl+F11`
    /// is the same key as `F11` on a platform that swallows the bare one. Demo mode pairs with none
    /// of `F10`, `F11` or `F12`, so a `Ctrl+F<n>` would be an arbitrary number sitting beside an
    /// unrelated key. The header above says the letters are for whoever is standing at the machine
    /// itself, which is exactly who this is for.
    ///
    /// **Turning it on asks for a song immediately**, which is the caller's job and the reason this
    /// is one action rather than two. The machine's own clock counts *idleness*, so a
    /// press that only flipped the switch would leave a silent room silent for another whole delay
    /// and read as a key that does nothing.
    ///
    /// No platform gate, as with [`Self::TogglePerformance`]: there is nothing outside the machine
    /// to reach, and a build with no keyboard simply cannot press it.
    ToggleDemo,
    /// Move the on-screen keypad's focus.
    ///
    /// The arrow keys, which on a television are the remote's D-pad: SDL translates
    /// `AKEYCODE_DPAD_UP` and friends into exactly these, and `DPAD_CENTER` into Return. So a TV
    /// remote needs no handling of its own — it arrives here as arrows and [`Self::Submit`].
    Focus(Direction),
    /// Enter or leave fullscreen.
    ///
    /// Desktop only in effect: an appliance build has nothing to return to, so it refuses this.
    ToggleFullscreen,
    /// Leave fullscreen, or quit if already windowed.
    Escape,
    /// Show the packages folder in the operating system's file manager.
    ///
    /// The one binding here that reaches outside the machine. It answers *where do I put my songs?*
    /// from in front of the machine rather than from a command line: every other route to that
    /// folder — `--show-paths`, a `POST` — assumes somebody who has
    /// already found it, and the fourth, dragging a package onto the window, assumes they have the
    /// file in hand.
    ///
    /// **Desktop only in effect**, and decided by the caller rather than here: this crate knows
    /// nothing about where packages live, and a mapping that changed shape per platform would be a
    /// worse table than one that always says what the key means. `km-app`'s `FILE_MANAGER` is what
    /// declines it where there is nothing to open a folder in.
    ///
    /// `F10`, because it is the first key past the strip's nine. It was `F12` until the meter
    /// wanted a key: the argument for `F12` was that `F10` and `F11` were both worth leaving
    /// alone — `F11` because it is the fullscreen key on every desktop there is, even though this
    /// machine uses `F`. Deference to a convention this application does not implement is a thin
    /// reason to leave a key doing nothing, and it went when there were three things to bind and
    /// three keys left. See [`Self::OpenRemote`], which took `F11` and owes that argument an
    /// answer.
    OpenPackagesFolder,
    /// Open the singer's remote in whatever this computer uses for web pages.
    ///
    /// The second binding here that reaches outside the machine, and the same shape as
    /// [`Self::OpenPackagesFolder`]: this crate knows nothing about where the remote is, so the
    /// caller decides both the address and whether there is a browser to send it to.
    ///
    /// It answers the question the connect panel raises and cannot finish. `I` puts the address and
    /// a QR code on the television, which is the right answer for somebody holding a phone and no
    /// answer at all for somebody sitting at the machine — who would have to read a URL off a
    /// screen and type it into a browser on the same box. One key does it.
    ///
    /// `F11` **is** the fullscreen key almost everywhere, and taking it is what this binding costs.
    /// Three things make it payable: this application's fullscreen is `F`, with `Escape` to leave;
    /// `F11` carries no other meaning here, so nothing is taken away from anybody; and what it does
    /// is open a window, which is visible, harmless and undone by closing it. A key that does
    /// nothing teaches nobody the convention it is being reserved for.
    ///
    /// **`Ctrl+F11` reaches it too, everywhere**, because on macOS the bare key never arrives: the
    /// window server takes `F11` for *Show Desktop* before an application is offered it. The second
    /// spelling costs a line in the table and is what the panel names on that platform; which key
    /// the panel names is the caller's, through `ConnectInfo::browser_key`.
    OpenRemote,
    /// Show or hide what the frame meter has been measuring.
    ///
    /// `F12`, and a diagnostic rather than a product control — it states nothing, changes nothing
    /// and reports only what the machine is already doing, which is what keeps it clear of
    /// `Front end scope`.
    ///
    /// **Drawing is not logging.** `--frame-stats` asks for a line a second in the log and this
    /// asks for a panel on the screen; turning one on must not turn the other on. Somebody
    /// diagnosing a stutter from the sofa wants the numbers where the stutter is, and should not
    /// have to discover afterwards that they have been writing to the journal all evening.
    ///
    /// No platform gate, unlike the two above: there is nothing outside the machine to reach, and a
    /// build with no keyboard simply cannot press it.
    TogglePerformance,
    /// Read the packages folders again, so a file copied in by hand takes effect now.
    ///
    /// **`Ctrl+F10`, beside the key that opens the folder**: `F10` shows you where songs go,
    /// `Ctrl+F10` picks up what you put there. A modifier rather than a key of its own because
    /// there is no bare function key left — the strip owns `F1`…`F9`, `F10` is the folder, `F11`
    /// goes to the remote and `F12` to the performance overlay.
    ///
    /// **The pairing is the point, not the digit.** This was written as `Ctrl+F12` against a keymap
    /// where `F12` opened the folder, and moved when that key did; if the function row is
    /// rearranged again it should follow the folder rather than the number.
    ///
    /// Dropping a `.kmpkg` on the window already installs one without a restart. This is for the
    /// file that arrived some other way — copied in with a file manager, pushed to a device, or put
    /// there by a deploy — where the folder is right and only the machine has not looked yet.
    ///
    /// **The work must not run on the display thread.** Installing holds the library's mutex for
    /// seconds per package, and a rescan is that once per package in the folders; the caller hands
    /// it to the same worker a drop goes to.
    RescanPackages,
    /// Go up one level, and out of the machine from the top.
    ///
    /// A television remote's BACK button, and a phone's back gesture. Unbound, it leaves the Android
    /// app with **no way out at all** — no window chrome, no Escape, no on-screen key.
    ///
    /// **Which key that is depends on the hardware, not on SDL's table.** SDL maps Android's
    /// `AKEYCODE_BACK` to `AC_BACK`, and a phone's gesture arrives that way — but a real Google TV
    /// remote sends `Escape` instead. Both are mapped here, and `km-app` turns Escape into this action
    /// on an appliance build where there is no fullscreen for Escape to leave.
    ///
    /// Deliberately not a plain quit. Android's TV quality guidelines require that back exits without
    /// a confirmation prompt, which removes the obvious guard against a stray press ending the
    /// evening mid-song. Going up a level instead is what the guidelines actually describe — back
    /// "gives users a way to return to the previous view" — so it is compliant *and* costs one song
    /// rather than the party. What each level means is decided by the caller, which is the only place
    /// that knows what is on screen.
    Back,
    /// Keep the window in front of every other application, or let it fall back into the stack.
    ///
    /// `T`, beside `F`. Both are window-mode keys, and the letters are for whoever is standing at
    /// the machine — which is exactly who this is for. A modified function key was the alternative
    /// and is the wrong shelf: the `Ctrl+F<n>` row is where the debugging controls live, and this
    /// is a product control an owner is meant to reach for.
    ///
    /// It exists for the machine that shares a screen with something else. A karaoke machine on a
    /// television owns the whole panel and never needs this; one on a desk beside a browser and a
    /// chat window is behind them the moment somebody clicks, and the words go with it. Fullscreen
    /// is not the same answer, because the point is to keep the other windows *usable*.
    ///
    /// **A window property rather than a drawing one**, so the caller applies it to the window and
    /// this crate never learns whether it took. Where there are no windows there is nothing to
    /// raise, and the caller is the only place that knows.
    ToggleAlwaysOnTop,
    /// Hold the transport strip on screen, or give it back its timer.
    ///
    /// `Ctrl+F12`, beside the frame meter: `F12` draws the diagnostic panel and `Ctrl+F12` stops
    /// the strip disappearing while somebody works on it. Both are diagnostics, and the modified
    /// function keys are where the diagnostics are.
    ///
    /// **The strip's six seconds are the feature**, and that is what this is for. It sits over the
    /// lyrics, so it goes away — which leaves anybody changing its layout, its labels or its
    /// hit-testing pressing a key every six seconds to look at the thing they are working on, and
    /// measuring a button they can only see for as long as it takes to reach the mouse.
    ///
    /// **Nothing is written down.** A pinned strip that survived a restart would be a bank of
    /// buttons somebody left over the words for a month, and a `settings.json` entry would make it
    /// a setting rather than the diagnostic it is. The same property [`Self::TogglePerformance`]
    /// rests on.
    ToggleStripPin,
    /// Play through a numbered SoundFont slot from now on.
    ///
    /// `Ctrl+1`…`Ctrl+9`, and the only binding here that needs a modifier at all — which is why
    /// [`action_for`] takes one. The bare digits are the song-number keypad and were bound long
    /// before this, so an unmodified `1` must go on meaning `Digit('1')`.
    ///
    /// **A debugging control, and not a product one.** Slot 1 is the bank the machine resolved for
    /// itself and the rest come from `debug.soundfonts`; where that is empty the caller does nothing
    /// at all. This crate knows none of that, in keeping with the rest of the table: a mapping says
    /// what a key means, and whether it means anything today is the caller's business.
    SelectSoundFont(u8),
    /// Quit.
    Quit,
}

/// Which way focus moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Up.
    Up,
    /// Down.
    Down,
    /// Left.
    Left,
    /// Right.
    Right,
}

/// One command on the transport strip: what it says, which function key reaches it, and what it
/// does.
///
/// **This is the one place the strip is written down.** [`TRANSPORT_COMMANDS`] feeds three things
/// that would otherwise have to be kept in step by hand — the strip's own layout
/// (`Keypad::playing`), the function-key bindings in [`action_for`], and the hint drawn on each key
/// — and a table that three consumers read cannot disagree with itself about which key does what.
/// A hint that named a key bound to something else is worse than no hint at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportCommand {
    /// The message id for what is drawn on the key. See [`crate::keypad::Label`].
    pub label_id: &'static str,
    /// The function key that reaches it, as it is written on a keyboard: `"F1"`.
    pub hint: &'static str,
    /// What pressing it means.
    pub action: DisplayAction,
    /// Whether the command exists only for a song that declares a melody channel.
    pub needs_melody: bool,
}

/// The transport strip, left to right, and the function keys that reach it.
///
/// **Fixed per command rather than positional.** `MELODY` is absent from the strip for a song that
/// declares no melody channel, so a key numbered by what happens to be on screen would mean one
/// thing during one song and another during the next — and would mean nothing at all while the
/// strip is timed out, which is most of the time. Numbering the *table* instead costs one dead key
/// (`F9`, where there is no melody) and buys a keyboard whose keys never move.
pub const TRANSPORT_COMMANDS: &[TransportCommand] = &[
    TransportCommand {
        label_id: "transport-pause",
        hint: "F1",
        action: DisplayAction::TogglePause,
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-back",
        hint: "F2",
        action: DisplayAction::SeekBy(-SEEK_STEP_SECS),
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-forward",
        hint: "F3",
        action: DisplayAction::SeekBy(SEEK_STEP_SECS),
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-next",
        hint: "F4",
        action: DisplayAction::Skip,
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-again",
        hint: "F5",
        action: DisplayAction::Restart,
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-queue",
        hint: "F6",
        action: DisplayAction::ToggleQueue,
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-key-down",
        hint: "F7",
        action: DisplayAction::TransposeDown,
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-key-up",
        hint: "F8",
        action: DisplayAction::TransposeUp,
        needs_melody: false,
    },
    TransportCommand {
        label_id: "transport-melody",
        hint: "F9",
        action: DisplayAction::ToggleMelody,
        needs_melody: true,
    },
];

/// Which entry of [`TRANSPORT_COMMANDS`] a function key names, if it names one.
///
/// `F1` is the first, and the run stops where the table does — `F10` and beyond are unbound rather
/// than wrapping round, so a key that does nothing goes on doing nothing when the strip grows.
fn transport_command_for(key: Keycode) -> Option<&'static TransportCommand> {
    use Keycode as K;
    let index = match key {
        K::F1 => 0,
        K::F2 => 1,
        K::F3 => 2,
        K::F4 => 3,
        K::F5 => 4,
        K::F6 => 5,
        K::F7 => 6,
        K::F8 => 7,
        K::F9 => 8,
        _ => return None,
    };
    TRANSPORT_COMMANDS.get(index)
}

/// Maps a key press to an action, or `None` if the key is not bound.
///
/// `ctrl` is whether either Control key was held. A `bool` rather than SDL's modifier bitflags
/// because Control is the only modifier any binding consults, and one that took the whole set would
/// invite a second: shift is already spoken for by the layout — `Plus` and `Less` arrive here as
/// themselves — and this table would be a worse place to decide that than the keyboard is.
///
/// **The modifier is read here rather than in the event loop**, deliberately, and it is the reason
/// this signature changed. `Ctrl+1` and `1` are different bindings that share a keycode, so
/// something has to decide which wins; doing it in the caller would leave that order implicit and
/// split across two files, where here it is the order of the arms below.
pub fn action_for(key: Keycode, ctrl: bool) -> Option<DisplayAction> {
    use Keycode as K;
    // Above everything, because these keycodes are the keypad's and would otherwise be claimed by
    // it. Nine slots because there are nine digits worth reaching blind; a tenth bank is a bank
    // somebody has to look up, which is what a settings file is for.
    if ctrl {
        return match key {
            K::_1 | K::Kp1 => Some(DisplayAction::SelectSoundFont(1)),
            K::_2 | K::Kp2 => Some(DisplayAction::SelectSoundFont(2)),
            K::_3 | K::Kp3 => Some(DisplayAction::SelectSoundFont(3)),
            K::_4 | K::Kp4 => Some(DisplayAction::SelectSoundFont(4)),
            K::_5 | K::Kp5 => Some(DisplayAction::SelectSoundFont(5)),
            K::_6 | K::Kp6 => Some(DisplayAction::SelectSoundFont(6)),
            K::_7 | K::Kp7 => Some(DisplayAction::SelectSoundFont(7)),
            K::_8 | K::Kp8 => Some(DisplayAction::SelectSoundFont(8)),
            K::_9 | K::Kp9 => Some(DisplayAction::SelectSoundFont(9)),
            // Beside the key that opens the folder: F10 shows you where songs go, Ctrl+F10 picks up
            // what you put there.
            K::F10 => Some(DisplayAction::RescanPackages),
            // The same key as the bare `F11`, and the only binding here that repeats one. macOS
            // takes `F11` for Show Desktop in the window server, so the press never reaches an
            // application and the panel names this spelling there; everywhere else it is a second
            // way to a key that already works, which costs a line and takes nothing.
            K::F11 => Some(DisplayAction::OpenRemote),
            // Beside the frame meter, and a diagnostic like it: F12 draws what the meter measures,
            // Ctrl+F12 stops the strip vanishing from under whoever is working on it.
            K::F12 => Some(DisplayAction::ToggleStripPin),
            // **`Ctrl+Q` and not `Q`**, which is taken: plain `Q` shows the queue, is in the README,
            // and is the kind of binding people learn without being told. Taking it would mean the
            // letter somebody presses to glance at what is next ends the evening instead.
            //
            // Control-Q is the conventional quit on this platform and is already this project's own
            // spelling for it — `km-tray` puts `⌘/Ctrl+Q` on its Quit item, so the machine now agrees
            // with the two tools that have a menu.
            K::Q => Some(DisplayAction::Quit),
            // Every other key with Control held is unbound rather than falling through to its
            // unmodified meaning. `Ctrl+Space` pausing the song would be a binding nobody wrote.
            _ => None,
        };
    }
    Some(match key {
        // The keypad. Both the number row and the numeric keypad, because a remote may send either.
        K::_0 | K::Kp0 => DisplayAction::Digit('0'),
        K::_1 | K::Kp1 => DisplayAction::Digit('1'),
        K::_2 | K::Kp2 => DisplayAction::Digit('2'),
        K::_3 | K::Kp3 => DisplayAction::Digit('3'),
        K::_4 | K::Kp4 => DisplayAction::Digit('4'),
        K::_5 | K::Kp5 => DisplayAction::Digit('5'),
        K::_6 | K::Kp6 => DisplayAction::Digit('6'),
        K::_7 | K::Kp7 => DisplayAction::Digit('7'),
        K::_8 | K::Kp8 => DisplayAction::Digit('8'),
        K::_9 | K::Kp9 => DisplayAction::Digit('9'),
        K::Return | K::Return2 | K::KpEnter => DisplayAction::Submit,
        K::Backspace => DisplayAction::Backspace,
        K::Delete => DisplayAction::Clear,

        // Transport.
        K::Space => DisplayAction::TogglePause,
        K::N => DisplayAction::Skip,
        K::R => DisplayAction::Restart,
        // Position within the song. `,` and `.` are what every video player uses for frame-stepping
        // and sit under two fingers; the arrows are already the D-pad and cannot be borrowed.
        K::Comma | K::Less => DisplayAction::SeekBy(-SEEK_STEP_SECS),
        K::Period | K::Greater => DisplayAction::SeekBy(SEEK_STEP_SECS),

        // Tone adjustment. Plus and minus on both the main keyboard and the keypad; equals is
        // included because plus needs shift on most layouts and singers reaching for it will miss.
        K::Plus | K::Equals | K::KpPlus => DisplayAction::TransposeUp,
        K::Minus | K::KpMinus => DisplayAction::TransposeDown,
        K::K => DisplayAction::TransposeReset,
        K::M => DisplayAction::ToggleMelody,

        // The D-pad on a television remote, and the arrow keys on a keyboard: the same events.
        K::Up => DisplayAction::Focus(Direction::Up),
        K::Down => DisplayAction::Focus(Direction::Down),
        K::Left => DisplayAction::Focus(Direction::Left),
        K::Right => DisplayAction::Focus(Direction::Right),

        // Display.
        K::W => DisplayAction::NextWallpaper,
        K::I => DisplayAction::ToggleConnect,
        K::Q => DisplayAction::ToggleQueue,
        K::F => DisplayAction::ToggleFullscreen,
        // Beside `F`, because the two are the same question about the same window: how much of the
        // screen the machine takes, and whether it stays in front of what else is on it.
        K::T => DisplayAction::ToggleAlwaysOnTop,
        K::D => DisplayAction::ToggleDemo,

        K::Escape => DisplayAction::Escape,
        // A TV remote's BACK button and a phone's back gesture both arrive here. SDL's Android
        // surface consumes the key and forwards it as `AC_BACK` unconditionally, so this needs no
        // hint and no change to SDL's Java.
        K::AcBack => DisplayAction::Back,

        // The three that are not the strip. Above the fall-through, and deliberately **not** entries
        // in `TRANSPORT_COMMANDS`: that table is the strip drawn over a playing song, numbered from
        // itself and carrying a `needs_melody` axis, and none of these is a transport control or
        // something to draw there. Keeping them out is what leaves `F1`…`F9` a contiguous run of the
        // table and `transport_command_for` unchanged.
        //
        // **The strip cannot grow to a tenth entry**, which is what these three cost. `F10` is
        // taken, so a tenth `TRANSPORT_COMMANDS` row would be a button on the screen that no key
        // presses — which is why the test below asserts the
        // table's length rather than merely asserting that these three keys are not in it.
        K::F10 => DisplayAction::OpenPackagesFolder,
        K::F11 => DisplayAction::OpenRemote,
        K::F12 => DisplayAction::TogglePerformance,

        // The transport strip, by number. Last, so nothing above can be shadowed by the table, and
        // written as a fall-through rather than as nine arms because the strip is defined once in
        // `TRANSPORT_COMMANDS` and this has to follow it.
        //
        // **These work whether or not the strip is on screen.** It is transient by design, so a
        // binding that only applied while it happened to be visible would be a binding that mostly
        // did nothing.
        _ => return transport_command_for(key).map(|command| command.action),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_map_from_both_the_number_row_and_the_keypad() {
        assert_eq!(
            action_for(Keycode::_7, false),
            Some(DisplayAction::Digit('7'))
        );
        assert_eq!(
            action_for(Keycode::Kp7, false),
            Some(DisplayAction::Digit('7'))
        );
        assert_eq!(
            action_for(Keycode::_0, false),
            Some(DisplayAction::Digit('0'))
        );
        assert_eq!(
            action_for(Keycode::Kp0, false),
            Some(DisplayAction::Digit('0'))
        );
    }

    #[test]
    fn every_digit_is_bound() {
        let rows = [
            Keycode::_0,
            Keycode::_1,
            Keycode::_2,
            Keycode::_3,
            Keycode::_4,
            Keycode::_5,
            Keycode::_6,
            Keycode::_7,
            Keycode::_8,
            Keycode::_9,
        ];
        for (expected, key) in rows.into_iter().enumerate() {
            let digit = char::from_digit(expected as u32, 10).expect("digit");
            assert_eq!(
                action_for(key, false),
                Some(DisplayAction::Digit(digit)),
                "digit {digit} should be bound"
            );
        }
    }

    #[test]
    fn enter_submits_from_either_enter_key() {
        assert_eq!(
            action_for(Keycode::Return, false),
            Some(DisplayAction::Submit)
        );
        assert_eq!(
            action_for(Keycode::KpEnter, false),
            Some(DisplayAction::Submit)
        );
    }

    #[test]
    fn transpose_up_works_without_holding_shift() {
        // Plus requires shift on most layouts, so equals has to work too or the key control is
        // effectively unreachable for whoever is singing.
        assert_eq!(
            action_for(Keycode::Equals, false),
            Some(DisplayAction::TransposeUp)
        );
        assert_eq!(
            action_for(Keycode::Plus, false),
            Some(DisplayAction::TransposeUp)
        );
        assert_eq!(
            action_for(Keycode::KpPlus, false),
            Some(DisplayAction::TransposeUp)
        );
    }

    #[test]
    fn transport_and_display_keys_are_bound() {
        assert_eq!(
            action_for(Keycode::Space, false),
            Some(DisplayAction::TogglePause)
        );
        assert_eq!(action_for(Keycode::N, false), Some(DisplayAction::Skip));
        assert_eq!(
            action_for(Keycode::M, false),
            Some(DisplayAction::ToggleMelody)
        );
        assert_eq!(
            action_for(Keycode::W, false),
            Some(DisplayAction::NextWallpaper)
        );
        assert_eq!(
            action_for(Keycode::I, false),
            Some(DisplayAction::ToggleConnect)
        );
        assert_eq!(
            action_for(Keycode::Q, false),
            Some(DisplayAction::ToggleQueue)
        );
        assert_eq!(
            action_for(Keycode::F, false),
            Some(DisplayAction::ToggleFullscreen)
        );
        assert_eq!(
            action_for(Keycode::D, false),
            Some(DisplayAction::ToggleDemo)
        );
        assert_eq!(
            action_for(Keycode::Escape, false),
            Some(DisplayAction::Escape)
        );
    }

    #[test]
    fn the_back_button_is_bound_and_is_not_the_same_as_escape() {
        // The regression this guards is a serious one: while `AC_BACK` was unbound, the Android
        // build could not be exited at all — there is no window chrome there, Escape never arrives,
        // and no on-screen key quits.
        assert_eq!(
            action_for(Keycode::AcBack, false),
            Some(DisplayAction::Back)
        );
        // Distinct on purpose. Escape's job is fullscreen first, quit second, which is a desktop
        // notion; Back goes up a level and only leaves from the top.
        assert_ne!(
            action_for(Keycode::AcBack, false),
            action_for(Keycode::Escape, false)
        );
    }

    #[test]
    fn unbound_keys_are_ignored_rather_than_doing_something_surprising() {
        // A function key past the end of the table, and asserting it stays unbound is what holds
        // `transport_command_for` to stopping rather than wrapping round.
        //
        // `F13` is the only witness left: `F10` goes to the packages folder, `F11` to the remote
        // and `F12` to the frame meter, and the run of function keys a keyboard actually has ends
        // here — so there is no fourth binding coming and no second witness to be had.
        assert_eq!(action_for(Keycode::F13, false), None);
        assert_eq!(action_for(Keycode::Tab, false), None);
        assert_eq!(action_for(Keycode::LShift, false), None);
    }

    #[test]
    fn ctrl_and_a_digit_selects_a_soundfont_slot() {
        for slot in 1..=9u8 {
            let digit = char::from(b'0' + slot);
            let row = Keycode::from_name(&digit.to_string()).expect("a digit keycode");
            assert_eq!(
                action_for(row, true),
                Some(DisplayAction::SelectSoundFont(slot)),
                "Ctrl+{digit} should select slot {slot}"
            );
        }
    }

    #[test]
    fn a_bare_digit_is_still_a_song_number() {
        // The regression this whole modifier argument exists to prevent. `Ctrl+1` and `1` share a
        // keycode, and a switcher that claimed the keypad would take the machine's primary input
        // away — on a build where `debug.soundfonts` is empty and the switcher does nothing at all.
        assert_eq!(
            action_for(Keycode::_1, false),
            Some(DisplayAction::Digit('1'))
        );
        assert_eq!(
            action_for(Keycode::Kp1, false),
            Some(DisplayAction::Digit('1'))
        );
    }

    #[test]
    fn ctrl_does_not_borrow_the_meaning_of_an_unmodified_key() {
        // Holding Control must not pause the song, skip it or leave fullscreen. Every binding that
        // is not a slot digit is unbound while it is held, rather than falling through.
        assert_eq!(action_for(Keycode::Space, true), None);
        assert_eq!(action_for(Keycode::N, true), None);
        assert_eq!(action_for(Keycode::F, true), None);
        assert_eq!(action_for(Keycode::F1, true), None);
        // Demo mode is the newest of them and the one with the most to undo: `Ctrl+D` must not
        // start the machine singing by itself.
        assert_eq!(action_for(Keycode::D, true), None);
        // And the window must not go in front of everything because somebody reached for a
        // Control-key shortcut their browser has and this application has not.
        assert_eq!(action_for(Keycode::T, true), None);
        // And zero, which is a keypad digit with no slot behind it — nine banks, not ten.
        assert_eq!(action_for(Keycode::_0, true), None);
    }

    #[test]
    fn the_three_keys_past_the_strip_are_bound_and_are_not_transport_commands() {
        assert_eq!(
            action_for(Keycode::F10, false),
            Some(DisplayAction::OpenPackagesFolder)
        );
        assert_eq!(
            action_for(Keycode::F11, false),
            Some(DisplayAction::OpenRemote)
        );
        assert_eq!(
            action_for(Keycode::F12, false),
            Some(DisplayAction::TogglePerformance)
        );
        // The half that is easy to lose, and it stopped being hypothetical when `F10` was taken.
        // All three are bound above the fall-through, so a tenth entry in `TRANSPORT_COMMANDS`
        // could not silently claim one — but it would also never be *reached*, so the strip would
        // draw a `F10` button that no key presses. The table must not be the thing that decides
        // what these three mean, and it can no longer grow at all.
        for key in [Keycode::F10, Keycode::F11, Keycode::F12] {
            assert!(
                transport_command_for(key).is_none(),
                "the strip's table has grown into {key:?}, which is bound to something else"
            );
        }
        assert!(
            TRANSPORT_COMMANDS.len() <= 9,
            "the strip owns F1..=F9 and F10 is the packages folder, so a tenth command would be a              button on screen that no key can press"
        );
    }

    /// `Ctrl+F10` reads the folders again, and the bare key still opens them.
    ///
    /// The pairing is the binding's whole justification — `F10` shows you where songs go,
    /// `Ctrl+F10` picks up what you put there — so both halves are asserted **together**, on the
    /// same key. That is what makes this test the thing that notices if the function row moves
    /// again: it was written for `F12`, when `F12` was the folder, and it moved with it.
    #[test]
    fn ctrl_f10_reads_the_folders_beside_the_key_that_opens_them() {
        assert_eq!(
            action_for(Keycode::F10, true),
            Some(DisplayAction::RescanPackages)
        );
        assert_eq!(
            action_for(Keycode::F10, false),
            Some(DisplayAction::OpenPackagesFolder),
            "the modifier must not take the bare key's meaning with it"
        );
    }

    /// `Ctrl+F11` opens the remote, because on macOS the bare key never arrives.
    ///
    /// The one binding in this table with two spellings, and the pair is asserted together for the
    /// reason the `F10` pair above is: the second spelling exists only to say the same thing, so a
    /// change that takes the bare key away from `OpenRemote` and leaves this one behind would be a
    /// panel naming a key that means something else.
    #[test]
    fn ctrl_f11_reaches_the_remote_where_the_bare_key_cannot() {
        assert_eq!(
            action_for(Keycode::F11, true),
            Some(DisplayAction::OpenRemote)
        );
        assert_eq!(
            action_for(Keycode::F11, false),
            Some(DisplayAction::OpenRemote)
        );
    }

    /// `Ctrl+F12` pins the strip beside the key that draws the frame meter.
    ///
    /// The same pairing test as `Ctrl+F10`'s, on the same terms: both halves on one key, so the
    /// modifier cannot quietly take the bare key's meaning and so the pairing is what moves if the
    /// function row is rearranged again.
    #[test]
    fn ctrl_f12_pins_the_strip_beside_the_meter_it_sits_under() {
        assert_eq!(
            action_for(Keycode::F12, true),
            Some(DisplayAction::ToggleStripPin)
        );
        assert_eq!(
            action_for(Keycode::F12, false),
            Some(DisplayAction::TogglePerformance),
            "the modifier must not take the bare key's meaning with it"
        );
    }

    /// The two window-mode keys are neighbors and stay distinct.
    ///
    /// `F` and `T` answer different questions about the same window, and a table that ever
    /// answered both with one action would be a machine where asking for one gave you the other.
    #[test]
    fn the_window_keys_sit_together_and_mean_different_things() {
        assert_eq!(
            action_for(Keycode::F, false),
            Some(DisplayAction::ToggleFullscreen)
        );
        assert_eq!(
            action_for(Keycode::T, false),
            Some(DisplayAction::ToggleAlwaysOnTop)
        );
        assert!(transport_command_for(Keycode::T).is_none());
    }

    /// The whole reason quit is spelled with a modifier: `Q` was already the queue, is in the
    /// README, and is a binding people learn without being told. If these two ever answer the same
    /// action, somebody glancing at what is next has ended the evening instead.
    #[test]
    fn quit_takes_the_modifier_and_leaves_the_queue_alone() {
        assert_eq!(action_for(Keycode::Q, true), Some(DisplayAction::Quit));
        assert_eq!(
            action_for(Keycode::Q, false),
            Some(DisplayAction::ToggleQueue)
        );
    }

    #[test]
    fn no_letter_key_is_bound_to_two_different_actions() {
        // Guards against a binding being added twice with different meanings, where the first match
        // silently wins and the second looks broken.
        let keys = [
            Keycode::N,
            Keycode::R,
            Keycode::K,
            Keycode::M,
            Keycode::W,
            Keycode::I,
            Keycode::Q,
            Keycode::F,
            Keycode::P,
            Keycode::D,
        ];
        let mut actions: Vec<DisplayAction> = keys
            .into_iter()
            .filter_map(|key| action_for(key, false))
            .collect();
        let before = actions.len();
        actions.sort_by_key(|action| format!("{action:?}"));
        actions.dedup();
        assert_eq!(before, actions.len(), "two keys map to the same action");
    }

    #[test]
    fn every_transport_command_has_the_function_key_it_advertises() {
        // The whole point of one table: the hint drawn on a key and the key that presses it cannot
        // disagree, because a wrong hint teaches somebody a binding that does something else.
        for (index, command) in TRANSPORT_COMMANDS.iter().enumerate() {
            let expected = format!("F{}", index + 1);
            assert_eq!(
                command.hint, expected,
                "{} advertises {} but is number {index} on the strip",
                command.label_id, command.hint
            );
        }
    }

    #[test]
    fn the_function_keys_reach_the_strip_in_order() {
        let keys = [
            Keycode::F1,
            Keycode::F2,
            Keycode::F3,
            Keycode::F4,
            Keycode::F5,
            Keycode::F6,
            Keycode::F7,
            Keycode::F8,
            Keycode::F9,
        ];
        assert_eq!(
            keys.len(),
            TRANSPORT_COMMANDS.len(),
            "a command was added to the strip without a function key to reach it"
        );
        for (key, command) in keys.into_iter().zip(TRANSPORT_COMMANDS) {
            assert_eq!(
                action_for(key, false),
                Some(command.action),
                "{} should be reached by {}",
                command.label_id,
                command.hint
            );
        }
    }

    #[test]
    fn a_function_key_means_the_same_thing_as_the_letter_that_shares_its_action() {
        // Two ways to the same place, deliberately: the letters are for whoever learned them, the
        // function keys for whoever is reading the strip. Neither may drift from the other.
        assert_eq!(
            action_for(Keycode::F1, false),
            action_for(Keycode::Space, false)
        );
        assert_eq!(
            action_for(Keycode::F4, false),
            action_for(Keycode::N, false)
        );
        assert_eq!(
            action_for(Keycode::F5, false),
            action_for(Keycode::R, false)
        );
        assert_eq!(
            action_for(Keycode::F6, false),
            action_for(Keycode::Q, false)
        );
        assert_eq!(
            action_for(Keycode::F8, false),
            action_for(Keycode::Plus, false)
        );
        assert_eq!(
            action_for(Keycode::F9, false),
            action_for(Keycode::M, false)
        );
    }

    #[test]
    fn only_the_last_command_needs_a_melody_channel() {
        // `MELODY` is appended, so it is the only command that can be absent — which is what makes a
        // fixed numbering possible at all. A second conditional command in the middle would
        // renumber everything after it, and this is what would say so.
        let conditional: Vec<&str> = TRANSPORT_COMMANDS
            .iter()
            .filter(|command| command.needs_melody)
            .map(|command| command.label_id)
            .collect();
        assert_eq!(conditional, ["transport-melody"]);
        assert!(
            TRANSPORT_COMMANDS
                .last()
                .is_some_and(|command| command.needs_melody)
        );
    }

    #[test]
    fn every_label_and_hint_is_drawable_in_every_language() {
        // **`is_ascii()` on the label would mean nothing**: a label is a message id, and an id is
        // ASCII by construction, while the word actually drawn is whatever a translator wrote. So
        // this checks what reaches the screen, in every locale.
        //
        // A hint is a function key's name and is not translated; it stays ASCII by inspection.
        for command in TRANSPORT_COMMANDS {
            assert!(command.hint.is_ascii(), "hint {}", command.hint);
            for locale in km_locale::Locale::ALL {
                let catalog = crate::words::messages(*locale);
                let label = catalog.msg(command.label_id);
                assert!(
                    catalog.keys().contains(command.label_id),
                    "{locale} has no message called `{}`",
                    command.label_id
                );
                for character in label.chars() {
                    assert!(
                        crate::words::is_drawable(character),
                        "{locale} `{}` = {label:?}: {character:?} has no picture in the bundled font",
                        command.label_id
                    );
                }
            }
        }
    }
}
