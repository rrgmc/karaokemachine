//! The on-screen keypad: where the buttons are, and what a touch at a point means.
//!
//! Pure geometry. No SDL, no drawing, no state — a layout for a screen size and a hit test against
//! it, so the awkward parts (a key falling off a small screen, two keys overlapping, a finger landing
//! in a gap) are testable without a window.
//!
//! **Why this exists.** Android has no keyboard, and until this module there was no way to enter a
//! song number on a device without one — `input::action_for` maps `Keycode` and nothing else. It
//! resolves to the same [`DisplayAction`] the keys produce, so nothing downstream of the event loop
//! changes: touch and a keyboard are the same thing by the time anybody acts on them.
//!
//! **The number pad is now drawn only where there is no keyboard** — Android by default, and
//! anywhere `display.number_pad` asks for it. `km-app` decides that; this module is geometry and
//! draws whatever it is asked for.
//!
//! The case for keeping it everywhere is not silly: a
//! real karaoke machine has a front panel, and a pad on the idle screen tells somebody they can type
//! a number, which an empty screen saying "Enter a song number" does not if they cannot find the
//! keyboard. What decided it is that on a desktop the keyboard is *already in front of them* — the
//! affordance the pad substitutes for is not missing there — and the pad spends the left half of
//! the idle screen to say so. On Android the argument is not needed, because there the pad
//! is not an affordance for typing but the only way to type at all.
//!
//! **The transport strip is not covered by that and still appears on every platform.** It names
//! actions — `REPEAT`, `KEY +`, `MELODY` — rather than duplicating keys somebody can see, and it is
//! transient, so it costs the screen nothing when nobody is using it.
//!
//! **What is on the strip is [`crate::input::TRANSPORT_COMMANDS`]**, not a list written out here,
//! because the same table carries the function key that presses each one. A key may say so — `F1`
//! under `PAUSE` — where the caller asks for hints, which on a device with no keyboard it does not.
//!
//! **Labels are words, not symbols.** Complex-script shaping is out of scope, so the fonts here are
//! Latin-coverage only and a glyph like `⏎` or `⏸` may render as a blank box. What
//! bounds a label is therefore [`crate::words::is_drawable`] — Latin-1 and a named handful — and not
//! ASCII, which the screen had already outgrown by drawing `…`.
//!
//! **A label is a message id, and a digit is not.** `PAUSA` and `TOM +` are what this strip says on a
//! machine set to Brazilian Portuguese; `7` is `7` everywhere, so the number pad draws its digits
//! directly rather than through a catalog somebody could get wrong. See [`Label`].

use crate::input::{Direction, DisplayAction};

/// Smallest side a touch target may have, in pixels.
///
/// 44 px is the figure both Apple and Google settled on for a finger, and it is a floor rather than a
/// preference: keys scale with the screen and only hit this on something very small.
pub const MIN_KEY_PX: f32 = 44.0;

/// Gap between keys, as a fraction of a key's side.
const GAP_FRACTION: f32 = 0.14;

/// A key's side on the number pad, as a fraction of the **smaller** screen dimension.
///
/// The smaller one, not the height: on a phone in portrait, height is 2400 px and a key sized from
/// it would be a quarter of the screen wide.
const IDLE_KEY_FRACTION: f32 = 0.11;

/// How much of the screen's width the number pad may claim.
///
/// The idle connect panel is right-anchored and 55% of the width (see `draw::draw_connect_panel`), so
/// the pad lives in the left 45% and the two cannot collide. This is a real coupling between two
/// modules and a test holds it: the first version of this keypad was drawn straight over the QR code.
const IDLE_MAX_WIDTH_FRACTION: f32 = 0.45;

/// A key's height on the transport strip, as a fraction of screen height.
const STRIP_KEY_HEIGHT_FRACTION: f32 = 0.085;

/// A transport key is this many times as wide as it is tall, so `MELODY` fits.
const STRIP_ASPECT: f32 = 2.4;

/// The safe inset from the screen's edges, as a fraction of **each** axis.
///
/// **A television does not show the edges of the picture it is given.** It overscans by a percentage
/// of each axis independently, so an inset taken off the shorter side is right on neither: on a
/// 16:9 panel one number is four per cent of the height and two and a quarter of the width, and the
/// narrow half is inside what the set eats. A build number placed there loses its leading letter to
/// the bezel.
///
/// Equal to [`Theme::margin`], which is the same inset seen from the module that has a theme —
/// `the_safe_inset_is_the_one_the_rest_of_the_screen_uses` is what stops the two drifting. Both are
/// the 5% the broadcast safe area has always been.
const MARGIN_FRACTION: f32 = 0.05;

/// Keys across the number pad. Three, because a telephone is three.
const COLUMNS: usize = 3;

/// Rows of digits: `1..9` and then `CLR 0 OK`.
const DIGIT_ROWS: usize = 4;

/// What is drawn on a key.
///
/// **Two kinds, because only one of them is a word.** A digit is the same mark in every language
/// this product renders — Brazilian Portuguese counts in the same Western Arabic numerals English
/// does — so putting `7` in a catalog would create a way to break the keypad and buy nothing. A
/// word is a message like any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// A digit, drawn as itself.
    Digit(char),
    /// A message id, resolved against the catalog when the key is drawn.
    Word(&'static str),
}

impl std::fmt::Display for Label {
    /// How a *test* names a key, which is not what a screen draws on one.
    ///
    /// A word shows as its message id, because that is the only name it has without a catalog —
    /// and a failure reading `transport-melody overlaps transport-key-up` says which key moved
    /// without pretending to know what language anybody is reading it in.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Label::Digit(digit) => write!(f, "{digit}"),
            Label::Word(id) => f.write_str(id),
        }
    }
}

/// One touch target.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    /// What pressing it means. The same type a key press produces.
    pub action: DisplayAction,
    /// What to draw on it.
    ///
    /// **Never parsed back into an action.** A digit table written as labels, with the action
    /// derived by matching `"CLR"` and `"OK"`, makes the printed word a control value: translating
    /// it changes what the key does. The table names the action, and this says only what to draw.
    pub label: Label,
    /// The keyboard key that does the same thing, drawn small under the label — `"F1"`.
    ///
    /// `None` on every key of the number pad, and on the transport strip where the caller declined
    /// hints: a hint is only worth the space where somebody has a keyboard to press, so Android
    /// asks for none. See `Keypad::playing`.
    pub hint: Option<&'static str>,
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Key {
    /// Whether a point is inside this key.
    ///
    /// Half-open on the far edges, so two keys sharing a boundary cannot both claim a touch — which
    /// matters because a gap of zero is a legitimate layout on a small screen.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// The center, which is where a label goes and where a test aims.
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// A laid-out set of touch targets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keypad {
    keys: Vec<Key>,
    /// Which key a remote's D-pad has moved to, if any.
    ///
    /// `None` until somebody presses a direction, and that is deliberate: on a desktop the keypad is
    /// a mouse target and a highlighted key nobody asked for would be noise. Pressing a direction is
    /// what opts in — and on a television it is the first thing that happens.
    focus: Option<usize>,
}

impl Keypad {
    /// Nothing to touch.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The number pad, for the idle screen.
    ///
    /// Bottom-**left**, because the connect panel and its QR code are bottom-right — see
    /// [`IDLE_MAX_WIDTH_FRACTION`].
    ///
    /// `reserve` is what the corner underneath already claims, in pixels, and the grid sits that far
    /// above the margin it would otherwise rest on. The build number is what claims it, and
    /// [`crate::draw::version_reserve`] is what measures it — a number rather than a flag, because
    /// this module is geometry and has no business knowing what is written down there. A screen with
    /// no room for the grid *and* the reserve gets no pad, which is what the refusal above already
    /// says about anything that will not fit.
    ///
    /// Laid out **like a telephone** — 1 at the top left, 0 at the bottom — not like a calculator.
    /// Song numbers are read aloud and typed the way a phone number is, and every karaoke machine
    /// and remote control anybody has used is arranged this way.
    ///
    /// **Twelve keys and nothing else.** There was a row of package-prefix keys above these, with a
    /// `123` key for the songs that had none — gone with the prefixes themselves, because a song
    /// number carries its package inside it now and there is nothing left to choose between. That
    /// row also had a written-down upgrade path for when more than about five packages stopped
    /// fitting; it is retired rather than outstanding.
    pub fn idle(width: f32, height: f32, reserve: f32) -> Self {
        let side = (width.min(height) * IDLE_KEY_FRACTION).max(MIN_KEY_PX);
        let gap = side * GAP_FRACTION;
        let margin_x = margin_x(width);
        let margin_y = margin_y(height);

        let grid_w = side * COLUMNS as f32 + gap * (COLUMNS as f32 - 1.0);
        let grid_h = side * DIGIT_ROWS as f32 + gap * (DIGIT_ROWS as f32 - 1.0);
        // Refuse rather than overlap. A screen too small for a usable pad gets none, and the
        // keyboard still works; half a keypad off the edge, or one over the QR code, is worse.
        if grid_w + margin_x > width * IDLE_MAX_WIDTH_FRACTION
            || grid_h + reserve + margin_y * 2.0 > height
        {
            return Self::empty();
        }

        let left = margin_x;
        let top = height - margin_y - reserve - grid_h;
        let mut keys = Vec::with_capacity(12);

        // **The action is what the table says, and the label follows from it** — not the other way
        // round. Deriving the action by matching on the printed word made the word a control value,
        // so `CLR` could not be translated without changing what the key did.
        const ROWS: [[DisplayAction; COLUMNS]; DIGIT_ROWS] = [
            [
                DisplayAction::Digit('1'),
                DisplayAction::Digit('2'),
                DisplayAction::Digit('3'),
            ],
            [
                DisplayAction::Digit('4'),
                DisplayAction::Digit('5'),
                DisplayAction::Digit('6'),
            ],
            [
                DisplayAction::Digit('7'),
                DisplayAction::Digit('8'),
                DisplayAction::Digit('9'),
            ],
            [
                DisplayAction::Clear,
                DisplayAction::Digit('0'),
                DisplayAction::Submit,
            ],
        ];
        for (row, actions) in ROWS.iter().enumerate() {
            for (column, action) in actions.iter().enumerate() {
                let action = *action;
                let label = match action {
                    DisplayAction::Digit(digit) => Label::Digit(digit),
                    DisplayAction::Clear => Label::Word(crate::words::KEYPAD_CLEAR),
                    _ => Label::Word(crate::words::KEYPAD_SUBMIT),
                };
                keys.push(Key {
                    action,
                    label,
                    // No hint on the number pad. Every key here is a digit, `CLR` or `OK`, and the
                    // keyboard key for each is the one with the same character printed on it —
                    // there is nothing a hint could tell somebody that the label does not.
                    hint: None,
                    x: left + column as f32 * (side + gap),
                    y: top + row as f32 * (side + gap),
                    w: side,
                    h: side,
                });
            }
        }
        Self { keys, focus: None }
    }

    /// The transport strip, for while a song is playing.
    ///
    /// Bottom-center and deliberately short: the screen is showing lyrics, and a permanent bank of
    /// buttons over them would be worse than reaching for a remote. `km-app` shows this only for a
    /// few seconds after a touch.
    ///
    /// `melody_available` is honored rather than ignored: when detection abstained there is no
    /// melody key at all, for the same reason the indicator is hidden — offering a control that
    /// would mute an arbitrary instrument is worse than offering nothing.
    ///
    /// **What is on the strip, and in what order, is [`crate::input::TRANSPORT_COMMANDS`]** rather
    /// than a list written out here. That table also carries the function key each command answers
    /// to, so the strip and the keyboard cannot come to disagree about what `F4` does — which they
    /// would, silently, the first time somebody reordered one of two lists.
    ///
    /// `hints` decides whether each key says which function key presses it. **Desktop only in
    /// practice**: on Android there is no keyboard to press, so the hint would be a line of text
    /// pointing at a key that does not exist. The caller decides, on the same reasoning that made
    /// the number pad a caller's decision — "Android" stands in for "no keyboard", and the two do
    /// come apart.
    ///
    /// `reserve` is how much of the bottom the strip gives up, the same argument
    /// [`Self::idle`] takes and for its reason: the position bar rests on the bottom safe inset and
    /// the strip stands above it rather than over it. `crate::draw::position_reserve` is what the
    /// machine passes, so a bar that moves takes the strip with it.
    pub fn playing(
        width: f32,
        height: f32,
        melody_available: bool,
        hints: bool,
        reserve: f32,
    ) -> Self {
        let commands: Vec<&'static crate::input::TransportCommand> =
            crate::input::TRANSPORT_COMMANDS
                .iter()
                .filter(|command| melody_available || !command.needs_melody)
                .collect();

        let count = commands.len() as f32;
        let mut key_h = (height * STRIP_KEY_HEIGHT_FRACTION).max(MIN_KEY_PX);
        let mut key_w = (key_h * STRIP_ASPECT).max(MIN_KEY_PX);
        let mut gap = key_h * GAP_FRACTION;
        let margin_x = margin_x(width);
        let margin_y = margin_y(height);

        // Shrink to fit rather than run off the edge. A phone in portrait is the case that needs it.
        let available = width - margin_x * 2.0;
        let wanted = key_w * count + gap * (count - 1.0);
        if wanted > available {
            let scale = available / wanted;
            key_w *= scale;
            key_h *= scale;
            gap *= scale;
        }
        if key_h < MIN_KEY_PX * 0.75 || key_w < MIN_KEY_PX {
            // Past this the keys are too small to hit reliably, so offer none.
            return Self::empty();
        }

        let total = key_w * count + gap * (count - 1.0);
        let left = (width - total) / 2.0;
        let top = height - margin_y - reserve - key_h;
        let keys = commands
            .into_iter()
            .enumerate()
            .map(|(index, command)| Key {
                action: command.action,
                label: Label::Word(command.label_id),
                hint: hints.then_some(command.hint),
                x: left + index as f32 * (key_w + gap),
                y: top,
                w: key_w,
                h: key_h,
            })
            .collect();
        Self { keys, focus: None }
    }

    /// What a touch at this point means, if anything.
    pub fn hit(&self, x: f32, y: f32) -> Option<DisplayAction> {
        self.keys
            .iter()
            .find(|key| key.contains(x, y))
            .map(|key| key.action)
    }

    /// Which key the D-pad is on, if any.
    pub fn focus(&self) -> Option<usize> {
        self.focus
    }

    /// The focused key.
    pub fn focused(&self) -> Option<&Key> {
        self.keys.get(self.focus?)
    }

    /// Restores a focus index, clamped to what this layout actually has.
    ///
    /// Needed because the caller rebuilds the keypad when the screen resizes or the screen changes,
    /// and losing the D-pad's position on a resize would be maddening on a television.
    pub fn restore_focus(&mut self, focus: Option<usize>) {
        self.focus = focus.filter(|index| *index < self.keys.len());
    }

    /// What the focused key would do, if one is focused.
    ///
    /// This is what OK on a remote activates. When nothing is focused it returns `None`, and the
    /// caller falls back to whatever Return means without a keypad — which is how the same key can
    /// submit a typed number on a desktop and press a button on a television.
    pub fn activate(&self) -> Option<DisplayAction> {
        self.focused().map(|key| key.action)
    }

    /// Moves the focus one step in a direction. Returns whether it moved.
    ///
    /// The first press focuses the key nearest the edge the press came *from* — pressing up focuses
    /// the bottom-most key — so the pad is entered from the direction the remote was pushed rather
    /// than jumping to an arbitrary corner.
    ///
    /// After that, the nearest key whose center lies in that direction wins, scored by distance along
    /// the direction first and sideways drift second. A plain "closest center" would let a diagonal
    /// neighbor steal a press meant for the key directly alongside.
    pub fn move_focus(&mut self, direction: Direction) -> bool {
        if self.keys.is_empty() {
            return false;
        }
        let Some(current) = self.focus else {
            self.focus = Some(self.entry_key(direction));
            return true;
        };

        let (from_x, from_y) = self.keys[current].center();
        let mut best: Option<(usize, f32)> = None;
        for (index, key) in self.keys.iter().enumerate() {
            if index == current {
                continue;
            }
            let (x, y) = key.center();
            let (along, across) = match direction {
                Direction::Up => (from_y - y, (x - from_x).abs()),
                Direction::Down => (y - from_y, (x - from_x).abs()),
                Direction::Left => (from_x - x, (y - from_y).abs()),
                Direction::Right => (x - from_x, (y - from_y).abs()),
            };
            // Strictly in the direction asked for, or it is not a move.
            if along <= 0.0 {
                continue;
            }
            // Sideways drift dominates, so a key in the same row or column always beats a diagonal.
            let score = across * 4.0 + along;
            if best.is_none_or(|(_, previous)| score < previous) {
                best = Some((index, score));
            }
        }
        match best {
            Some((index, _)) => {
                self.focus = Some(index);
                true
            }
            // Already at the edge. Staying put is right: wrapping around would move the highlight to
            // the far side of the pad, which reads as a glitch rather than a move.
            None => false,
        }
    }

    /// The key to focus when the D-pad is first pressed in a direction.
    fn entry_key(&self, direction: Direction) -> usize {
        let mut best = 0;
        let mut best_value = f32::INFINITY;
        for (index, key) in self.keys.iter().enumerate() {
            let (x, y) = key.center();
            let value = match direction {
                // Pressing up enters from the bottom, and so on.
                Direction::Up => -y,
                Direction::Down => y,
                Direction::Left => -x,
                Direction::Right => x,
            };
            if value < best_value {
                best_value = value;
                best = index;
            }
        }
        best
    }

    /// The keys, for drawing.
    pub fn keys(&self) -> &[Key] {
        &self.keys
    }

    /// Whether there is anything to draw or touch.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The rectangle enclosing every key, for drawing a backing panel behind them.
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let first = self.keys.first()?;
        let mut left = first.x;
        let mut top = first.y;
        let mut right = first.x + first.w;
        let mut bottom = first.y + first.h;
        for key in &self.keys[1..] {
            left = left.min(key.x);
            top = top.min(key.y);
            right = right.max(key.x + key.w);
            bottom = bottom.max(key.y + key.h);
        }
        Some((left, top, right - left, bottom - top))
    }
}

/// The safe inset from the left and right edges, in pixels.
pub(crate) fn margin_x(width: f32) -> f32 {
    width * MARGIN_FRACTION
}

/// The safe inset from the top and bottom edges, in pixels.
///
/// Its own function rather than [`margin_x`] applied to the other number, because the two are
/// different lengths on every screen that is not square and a caller reaching for the wrong one
/// gets an answer that looks plausible.
pub(crate) fn margin_y(height: f32) -> f32 {
    height * MARGIN_FRACTION
}

/// A pixel density that can be trusted.
///
/// SDL can report zero, or a NaN, for a display it has not measured yet. Scaling a click by that
/// puts every press in the top-left corner or nowhere at all, so an implausible reading becomes 1.0
/// — which is the truth on every platform that deals in physical pixels to begin with.
pub fn pixel_density(raw: f32) -> f32 {
    if raw.is_finite() && raw > 0.0 {
        raw
    } else {
        1.0
    }
}

/// Moves a point from logical window coordinates into the pixel space the keys are laid out in.
///
/// Mouse events arrive in window coordinates, which are not pixels on a high-density display; the
/// keys are placed in the backbuffer's pixels. On a screen where the two agree this is the identity.
pub fn window_to_pixels(x: f32, y: f32, density: f32) -> (f32, f32) {
    (x * density, y * density)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    /// Standard formats spanning the shapes a screen can be: 720p, 1080p, UHD 4K, DCI 4K, 21:9,
    /// and a phone in portrait.
    ///
    /// Named sizes rather than fractions because the geometry these tests guard is only a fraction
    /// until it is rounded and clamped — `Theme::px` clamps to 10..400, keys stop shrinking at
    /// `MIN_KEY_PX`, and how many characters fit needs a rasterised face and a real pixel width.
    /// Those bite at particular sizes, so a size has to be named to reach them.
    ///
    /// Chosen for shape, never for hardware: 0.45 through 2.37 in aspect ratio, with the two 4K
    /// entries differing only in width so a test that confuses width with height fails on one and
    /// passes on the other. Both extremes are load-bearing and neither is anybody's screen.
    const SCREENS: [(f32, f32); 6] = [
        (1280.0, 720.0),
        (1920.0, 1080.0),
        (3840.0, 2160.0),
        (4096.0, 2160.0),
        (2560.0, 1080.0),
        (1080.0, 2400.0),
    ];

    /// The number pad as the screen actually lays it out.
    ///
    /// The reserve is not a number a test may choose: the pad's whole relationship with the build
    /// number below it is that the two are measured from one expression, and a test passing a
    /// convenient constant would prove the arrangement it invented rather than the one that ships.
    fn idle_pad(width: f32, height: f32) -> Keypad {
        Keypad::idle(
            width,
            height,
            crate::draw::version_reserve(&crate::theme::Theme::default(), width, height),
        )
    }

    #[test]
    fn the_number_pad_is_laid_out_like_a_telephone_not_a_calculator() {
        let pad = idle_pad(1920.0, 1080.0);
        let labels: Vec<Label> = pad.keys().iter().map(|key| key.label).collect();
        // 1 first, 0 near the end: how every phone, remote and karaoke machine is arranged. A
        // calculator would put 7 first, and somebody typing a song number would hit the wrong key.
        //
        // Asserted on the label rather than the action, because it is the *arrangement* under
        // somebody's thumb that this is about — the digit drawn where they expect it.
        assert_eq!(
            labels,
            [
                Label::Digit('1'),
                Label::Digit('2'),
                Label::Digit('3'),
                Label::Digit('4'),
                Label::Digit('5'),
                Label::Digit('6'),
                Label::Digit('7'),
                Label::Digit('8'),
                Label::Digit('9'),
                Label::Word(crate::words::KEYPAD_CLEAR),
                Label::Digit('0'),
                Label::Word(crate::words::KEYPAD_SUBMIT),
            ]
        );
    }

    #[test]
    fn every_key_stays_on_screen() {
        for (width, height) in SCREENS {
            for pad in [
                idle_pad(width, height),
                Keypad::playing(
                    width,
                    height,
                    true,
                    true,
                    crate::draw::position_reserve(height),
                ),
            ] {
                for key in pad.keys() {
                    assert!(key.x >= 0.0, "{width}x{height}: {} off the left", key.label);
                    assert!(key.y >= 0.0, "{width}x{height}: {} off the top", key.label);
                    assert!(
                        key.x + key.w <= width,
                        "{width}x{height}: {} off the right",
                        key.label
                    );
                    assert!(
                        key.y + key.h <= height,
                        "{width}x{height}: {} off the bottom",
                        key.label
                    );
                }
            }
        }
    }

    #[test]
    fn no_two_keys_overlap() {
        for (width, height) in SCREENS {
            for pad in [
                idle_pad(width, height),
                Keypad::playing(
                    width,
                    height,
                    true,
                    true,
                    crate::draw::position_reserve(height),
                ),
            ] {
                let keys = pad.keys();
                for (index, key) in keys.iter().enumerate() {
                    for other in &keys[index + 1..] {
                        let separated = key.x + key.w <= other.x
                            || other.x + other.w <= key.x
                            || key.y + key.h <= other.y
                            || other.y + other.h <= key.y;
                        assert!(
                            separated,
                            "{width}x{height}: {} overlaps {}",
                            key.label, other.label
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn keys_are_big_enough_for_a_finger() {
        for (width, height) in SCREENS {
            for key in idle_pad(width, height).keys() {
                assert!(
                    key.w >= MIN_KEY_PX && key.h >= MIN_KEY_PX,
                    "{width}x{height}: {} is {}x{}",
                    key.label,
                    key.w,
                    key.h
                );
            }
        }
    }

    #[test]
    fn keys_grow_with_the_screen() {
        let small = idle_pad(1280.0, 720.0).keys()[0].w;
        let large = idle_pad(3840.0, 2160.0).keys()[0].w;
        assert!(large > small * 2.5, "{small} -> {large}");
    }

    #[test]
    fn a_touch_at_a_keys_center_produces_its_action() {
        let pad = idle_pad(1920.0, 1080.0);
        for key in pad.keys() {
            let (x, y) = key.center();
            assert_eq!(pad.hit(x, y), Some(key.action), "{}", key.label);
        }
    }

    #[test]
    fn a_touch_in_the_middle_of_the_screen_hits_nothing() {
        let pad = idle_pad(1920.0, 1080.0);
        // The pad sits in a corner; the rest of the screen must stay untouchable, or somebody
        // reaching for the lyrics would queue a song.
        assert_eq!(pad.hit(100.0, 100.0), None);
        assert_eq!(pad.hit(960.0, 200.0), None);
    }

    #[test]
    fn a_touch_in_the_gap_between_keys_hits_nothing() {
        let pad = idle_pad(1920.0, 1080.0);
        let first = &pad.keys()[0];
        let second = &pad.keys()[1];
        let gap_x = (first.x + first.w + second.x) / 2.0;
        assert!(gap_x > first.x + first.w && gap_x < second.x);
        // Better a missed press than the wrong song.
        assert_eq!(pad.hit(gap_x, first.center().1), None);
    }

    #[test]
    fn a_screen_too_small_for_a_usable_pad_gets_none() {
        let pad = idle_pad(200.0, 150.0);
        assert!(pad.is_empty());
        assert_eq!(pad.hit(100.0, 75.0), None);
        // Half a keypad running off the edge would be worse than no keypad; the keyboard still works.
        assert_eq!(pad.bounds(), None);
    }

    #[test]
    fn the_transport_strip_omits_the_melody_key_when_detection_abstained() {
        let with = Keypad::playing(
            1920.0,
            1080.0,
            true,
            true,
            crate::draw::position_reserve(1080.0),
        );
        let without = Keypad::playing(
            1920.0,
            1080.0,
            false,
            true,
            crate::draw::position_reserve(1080.0),
        );
        assert!(
            with.keys()
                .iter()
                .any(|key| key.label == Label::Word("transport-melody"))
        );
        assert!(
            !without
                .keys()
                .iter()
                .any(|key| key.label == Label::Word("transport-melody"))
        );
        // Offering a control that would mute an arbitrary instrument is worse than offering none.
        assert_eq!(without.keys().len(), with.keys().len() - 1);
    }

    #[test]
    fn a_transport_key_carries_the_function_key_from_the_one_table() {
        let strip = Keypad::playing(
            1920.0,
            1080.0,
            true,
            true,
            crate::draw::position_reserve(1080.0),
        );
        let hints: Vec<Option<&str>> = strip.keys().iter().map(|key| key.hint).collect();
        assert_eq!(
            hints,
            vec![
                Some("F1"),
                Some("F2"),
                Some("F3"),
                Some("F4"),
                Some("F5"),
                Some("F6"),
                Some("F7"),
                Some("F8"),
                Some("F9"),
            ]
        );
        // And they are the table's, not a second list that happens to agree today.
        for (key, command) in strip.keys().iter().zip(crate::input::TRANSPORT_COMMANDS) {
            assert_eq!(key.label, Label::Word(command.label_id));
            assert_eq!(key.hint, Some(command.hint));
            assert_eq!(key.action, command.action);
        }
    }

    #[test]
    fn the_numbering_does_not_shift_when_the_melody_key_is_absent() {
        // The reason the table is numbered rather than the strip: `KEY -` is `F7` whether or not
        // there is a melody key after it, so a keyboard learned during one song still works during
        // the next.
        let without = Keypad::playing(
            1920.0,
            1080.0,
            false,
            true,
            crate::draw::position_reserve(1080.0),
        );
        for key in without.keys() {
            let expected = crate::input::TRANSPORT_COMMANDS
                .iter()
                .find(|command| Label::Word(command.label_id) == key.label)
                .map(|command| command.hint);
            assert_eq!(key.hint, expected, "{} moved", key.label);
        }
    }

    #[test]
    fn a_device_with_no_keyboard_gets_no_hints() {
        let strip = Keypad::playing(
            1920.0,
            1080.0,
            true,
            false,
            crate::draw::position_reserve(1080.0),
        );
        assert!(strip.keys().iter().all(|key| key.hint.is_none()));
        // The strip itself is unchanged — the hint is the only thing declined.
        let with = Keypad::playing(
            1920.0,
            1080.0,
            true,
            true,
            crate::draw::position_reserve(1080.0),
        );
        assert_eq!(strip.keys().len(), with.keys().len());
        for (bare, hinted) in strip.keys().iter().zip(with.keys()) {
            assert_eq!(bare.label, hinted.label);
            assert_eq!(bare.action, hinted.action);
            assert_eq!(
                (bare.x, bare.y, bare.w, bare.h),
                (hinted.x, hinted.y, hinted.w, hinted.h)
            );
        }
    }

    #[test]
    fn the_number_pad_never_carries_a_hint() {
        // Every key there is a digit, `CLR` or `OK`, and the keyboard key for each is the one with
        // the same character on it. A hint would repeat the label.
        let pad = idle_pad(1920.0, 1080.0);
        assert!(!pad.is_empty());
        assert!(pad.keys().iter().all(|key| key.hint.is_none()));
    }

    #[test]
    fn the_transport_strip_fits_a_phone_in_portrait() {
        let pad = Keypad::playing(
            1080.0,
            2400.0,
            true,
            true,
            crate::draw::position_reserve(2400.0),
        );
        assert!(!pad.is_empty(), "a phone should get a transport strip");
        let (_, _, width, _) = pad.bounds().expect("bounds");
        assert!(
            width <= 1080.0,
            "the strip is {width} wide on a 1080 screen"
        );
    }

    #[test]
    fn the_transport_strip_is_centered_and_at_the_bottom() {
        let (width, height) = (1920.0, 1080.0);
        let pad = Keypad::playing(
            width,
            height,
            true,
            true,
            crate::draw::position_reserve(height),
        );
        let (left, top, strip_w, strip_h) = pad.bounds().expect("bounds");
        // Centered within a pixel.
        assert!(((left + strip_w / 2.0) - width / 2.0).abs() < 1.0);
        // Bottom half of the screen, below the lyrics.
        assert!(top + strip_h <= height);
        assert!(top > height * 0.8, "the strip is at y={top}");
    }

    /// The safe inset is one number, and two modules spell it.
    ///
    /// [`Theme::margin`] is what the title, the lyrics and the connect panel sit on; this module has
    /// no theme to ask and carries [`MARGIN_FRACTION`] instead. They are the same inset and the
    /// screen looks wrong when they are not: furniture in a corner would stand a different distance
    /// from the edge than everything lined up above it.
    #[test]
    fn the_safe_inset_is_the_one_the_rest_of_the_screen_uses() {
        let theme = Theme::default();
        assert_eq!(
            MARGIN_FRACTION, theme.margin,
            "the keypad's safe inset and the theme's have come apart"
        );
        // And the horizontal half is what the theme's own pixel figure returns, so nothing rounds
        // one of them a pixel off the other.
        for (width, height) in SCREENS {
            let _ = height;
            assert!(
                (margin_x(width) - theme.margin_px(width as u32)).abs() <= 1.0,
                "at width {width} the keypad says {} and the theme says {}",
                margin_x(width),
                theme.margin_px(width as u32)
            );
        }
    }

    #[test]
    fn the_number_pad_sits_bottom_left_clear_of_the_screen_edges() {
        let (width, height) = (1920.0, 1080.0);
        let pad = idle_pad(width, height);
        let (left, top, _, pad_h) = pad.bounds().expect("bounds");
        // The two insets are different lengths on a 16:9 panel, which is the whole reason they are
        // two functions: asking for the wrong one here would pass on a square screen and nowhere
        // else.
        assert!(
            (left - margin_x(width)).abs() < 1.0,
            "left edge at {left}, safe inset at {}",
            margin_x(width)
        );
        // As low as the corner underneath leaves room for, so the pad reads as a front panel rather
        // than as something floating in the middle of the screen.
        let floor = height
            - margin_y(height)
            - crate::draw::version_reserve(&Theme::default(), width, height);
        assert!(
            (top + pad_h - floor).abs() < 1.0,
            "bottom edge at {}, floor at {floor}",
            top + pad_h
        );
    }

    /// The pad stands above the build number rather than over it.
    ///
    /// Both sides of the coupling are read from the crate rather than restated — the pad from
    /// `bounds`, the number from `draw::version_origin` — because the whole arrangement is that one
    /// expression decides both, and a test naming its own figure would go on passing after they
    /// stopped agreeing.
    #[test]
    fn the_number_pad_leaves_room_for_the_build_number() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let Some((_, top, _, pad_h)) = idle_pad(width, height).bounds() else {
                continue;
            };
            let (_, version_top) = crate::draw::version_origin(&theme, width, height);
            assert!(
                top + pad_h <= version_top,
                "{width}x{height}: the pad ends at {} and the build number starts at {version_top}",
                top + pad_h
            );
        }
    }

    #[test]
    fn the_number_pad_never_reaches_the_connect_panel() {
        // The regression this exists for: the first version anchored bottom-right and was drawn
        // straight over the QR code. The connect panel is right-anchored and 55% wide, so the pad
        // must stay inside the left 45%.
        for (width, height) in SCREENS {
            let pad = idle_pad(width, height);
            if let Some((left, _, pad_w, _)) = pad.bounds() {
                assert!(
                    left + pad_w <= width * IDLE_MAX_WIDTH_FRACTION,
                    "{width}x{height}: the pad reaches {} of a {width} screen, into the \
                     connect panel",
                    left + pad_w
                );
            }
        }
    }

    #[test]
    fn a_portrait_phone_does_not_get_quarter_width_keys() {
        // Sized from the smaller dimension, not the height. From the height, a 2400 px tall phone
        // would get 264 px keys on a 1080 px wide screen.
        let pad = idle_pad(1080.0, 2400.0);
        let key = &pad.keys()[0];
        assert!(
            key.w < 1080.0 * 0.16,
            "a key is {} wide on a 1080 px screen",
            key.w
        );
    }

    #[test]
    fn every_key_draws_something_the_bundled_font_has() {
        // **The label is data again**, which is what this test was worth more against before the
        // prefix keys went — except that now every word on the pad comes from a catalog, in a
        // language nobody writing this file is reading. `is_ascii()` on the label could not see
        // that: an id is ASCII whatever `TOM +` turns out to be.
        //
        // No font is opened, per the rule for this crate. `is_drawable` is a fact about the font's
        // repertoire, written down where a test can reach it.
        for pad in [
            idle_pad(1920.0, 1080.0),
            Keypad::playing(
                1920.0,
                1080.0,
                true,
                true,
                crate::draw::position_reserve(1080.0),
            ),
        ] {
            for key in pad.keys() {
                for locale in km_locale::Locale::ALL {
                    let catalog = crate::words::messages(*locale);
                    let drawn = match key.label {
                        Label::Digit(digit) => digit.to_string(),
                        Label::Word(id) => {
                            assert!(
                                catalog.keys().contains(id),
                                "{locale} has no message called `{id}`"
                            );
                            catalog.msg(id).into_owned()
                        }
                    };
                    assert!(!drawn.is_empty(), "{locale} `{}` is blank", key.label);
                    for character in drawn.chars() {
                        assert!(
                            crate::words::is_drawable(character),
                            "{locale} `{}` = {drawn:?}: {character:?} has no picture in the \
                             bundled font",
                            key.label
                        );
                    }
                }
            }
        }
    }

    // -- D-pad focus, which is how a television remote drives this --------------------------------

    #[test]
    fn nothing_is_focused_until_a_direction_is_pressed() {
        // On a desktop the pad is a mouse target, and a highlight nobody asked for is noise.
        let pad = idle_pad(1920.0, 1080.0);
        assert_eq!(pad.focus(), None);
        assert!(pad.focused().is_none());
        assert_eq!(pad.activate(), None);
    }

    #[test]
    fn the_pad_is_entered_from_the_direction_the_remote_was_pushed() {
        // Pressing up should land on the bottom row, not jump to an arbitrary corner.
        let mut up = idle_pad(1920.0, 1080.0);
        assert!(up.move_focus(Direction::Up));
        let label = up.focused().expect("focused").label;
        assert!(
            [
                Label::Word(crate::words::KEYPAD_CLEAR),
                Label::Digit('0'),
                Label::Word(crate::words::KEYPAD_SUBMIT)
            ]
            .contains(&label),
            "pressing up entered at {label}, not the bottom row"
        );

        let mut down = idle_pad(1920.0, 1080.0);
        down.move_focus(Direction::Down);
        assert!(
            [Label::Digit('1'), Label::Digit('2'), Label::Digit('3')]
                .contains(&down.focused().expect("focused").label)
        );

        let mut right = idle_pad(1920.0, 1080.0);
        right.move_focus(Direction::Right);
        assert_eq!(right.focused().expect("focused").label, Label::Digit('1'));
    }

    #[test]
    fn focus_walks_the_grid_one_key_at_a_time() {
        let mut pad = idle_pad(1920.0, 1080.0);
        pad.restore_focus(Some(0)); // "1", top-left
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('1'));

        pad.move_focus(Direction::Right);
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('2'));
        pad.move_focus(Direction::Down);
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('5'));
        pad.move_focus(Direction::Down);
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('8'));
        pad.move_focus(Direction::Down);
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('0'));
        pad.move_focus(Direction::Right);
        assert_eq!(
            pad.focused().expect("focused").label,
            Label::Word(crate::words::KEYPAD_SUBMIT)
        );
        pad.move_focus(Direction::Left);
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('0'));
        pad.move_focus(Direction::Up);
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('8'));
    }

    #[test]
    fn a_diagonal_neighbor_never_steals_a_straight_press() {
        // The bug a plain nearest-center search would have: from "1", pressing down must reach "4"
        // and not "5", even though "5" is only slightly further away.
        let mut pad = idle_pad(1920.0, 1080.0);
        pad.restore_focus(Some(0));
        pad.move_focus(Direction::Down);
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('4'));
    }

    #[test]
    fn focus_stops_at_the_edge_rather_than_wrapping() {
        let mut pad = idle_pad(1920.0, 1080.0);
        pad.restore_focus(Some(0)); // "1", the top-left corner
        // Wrapping would put the highlight on the far side of the pad, which reads as a glitch.
        assert!(!pad.move_focus(Direction::Up));
        assert!(!pad.move_focus(Direction::Left));
        assert_eq!(pad.focused().expect("focused").label, Label::Digit('1'));
    }

    #[test]
    fn ok_on_the_remote_activates_whatever_is_focused() {
        let mut pad = idle_pad(1920.0, 1080.0);
        pad.restore_focus(Some(0));
        assert_eq!(pad.activate(), Some(DisplayAction::Digit('1')));
        pad.move_focus(Direction::Down);
        pad.move_focus(Direction::Down);
        pad.move_focus(Direction::Down);
        assert_eq!(pad.activate(), Some(DisplayAction::Clear));
    }

    #[test]
    fn focus_survives_a_resize_but_cannot_point_past_the_new_layout() {
        let mut pad = idle_pad(1920.0, 1080.0);
        pad.restore_focus(Some(11)); // "OK", the last key
        assert_eq!(
            pad.focused().expect("focused").label,
            Label::Word(crate::words::KEYPAD_SUBMIT)
        );

        // The transport strip has fewer keys; a stale index must not survive into it.
        let mut strip = Keypad::playing(
            1920.0,
            1080.0,
            false,
            true,
            crate::draw::position_reserve(1080.0),
        );
        strip.restore_focus(Some(11));
        assert_eq!(strip.focus(), None);

        // ...but an index that still fits is kept, so a resize does not lose the remote's place.
        // Whichever key index 2 happens to be: the property is that the index survives, and pinning
        // it to a label made this fail when the strip gained its position keys.
        strip.restore_focus(Some(2));
        assert!(strip.focused().is_some());
        assert_eq!(strip.focus(), Some(2));
    }

    #[test]
    fn the_transport_strip_moves_sideways_and_not_vertically() {
        let mut strip = Keypad::playing(
            1920.0,
            1080.0,
            true,
            true,
            crate::draw::position_reserve(1080.0),
        );
        strip.restore_focus(Some(0));
        assert_eq!(
            strip.focused().expect("focused").label,
            Label::Word("transport-pause")
        );
        assert!(strip.move_focus(Direction::Right));
        let second = strip.focused().expect("focused").label;
        assert_ne!(
            second,
            Label::Word("transport-pause"),
            "focus moved along the strip"
        );
        // One row, so up and down have nowhere to go.
        assert!(!strip.move_focus(Direction::Up));
        assert!(!strip.move_focus(Direction::Down));
        assert_eq!(strip.focused().expect("focused").label, second);
    }

    #[test]
    fn the_strip_can_move_the_position_within_a_song_in_both_directions() {
        let strip = Keypad::playing(
            1920.0,
            1080.0,
            true,
            true,
            crate::draw::position_reserve(1080.0),
        );
        let seeks: Vec<_> = strip
            .keys()
            .iter()
            .filter(|key| matches!(key.action, DisplayAction::SeekBy(_)))
            .collect();
        assert_eq!(seeks.len(), 2, "back and forward");
        assert_eq!(
            seeks[0].action,
            DisplayAction::SeekBy(-crate::input::SEEK_STEP_SECS),
            "back comes first, reading left to right"
        );
        assert_eq!(
            seeks[1].action,
            DisplayAction::SeekBy(crate::input::SEEK_STEP_SECS)
        );
        // Seconds, not arrows: `>>` next to `NEXT` reads as a faster next song. Asserted against
        // what every catalog draws, because the seconds are the part a translation must keep — the
        // number of them is the whole content of the key, and `-10s` is not a phrase to translate.
        for locale in km_locale::Locale::ALL {
            let catalog = crate::words::messages(*locale);
            for key in &seeks {
                let Label::Word(id) = key.label else {
                    panic!("a seek key drew a bare digit");
                };
                let drawn = catalog.msg(id);
                assert!(
                    drawn.contains("10s"),
                    "{locale} `{id}` = {drawn:?} stopped saying how far it seeks"
                );
            }
        }
    }

    #[test]
    fn moving_focus_on_an_empty_keypad_does_nothing() {
        let mut pad = Keypad::empty();
        assert!(!pad.move_focus(Direction::Up));
        assert_eq!(pad.focus(), None);
    }

    #[test]
    fn every_key_is_reachable_from_every_other_by_d_pad() {
        // The property that matters on a television: no key is stranded. Walk to the far corner in
        // each direction and check the whole grid can be covered.
        let mut pad = idle_pad(1920.0, 1080.0);
        let count = pad.keys().len();
        pad.restore_focus(Some(0));
        let mut seen = std::collections::BTreeSet::new();
        // Snake through: across each row, then down.
        for row in 0..4 {
            for _ in 0..3 {
                seen.insert(pad.focus().expect("focused"));
                pad.move_focus(Direction::Right);
            }
            seen.insert(pad.focus().expect("focused"));
            for _ in 0..3 {
                pad.move_focus(Direction::Left);
            }
            if row < 3 {
                pad.move_focus(Direction::Down);
            }
        }
        assert_eq!(
            seen.len(),
            count,
            "only reached {} of {count} keys",
            seen.len()
        );
    }

    #[test]
    fn an_empty_keypad_is_harmless() {
        let pad = Keypad::empty();
        assert!(pad.is_empty());
        assert!(pad.keys().is_empty());
        assert_eq!(pad.hit(0.0, 0.0), None);
        assert_eq!(pad.bounds(), None);
    }

    #[test]
    fn the_digit_keys_map_to_the_digits_they_show() {
        for key in idle_pad(1920.0, 1080.0).keys() {
            if let DisplayAction::Digit(digit) = key.action {
                assert_eq!(key.label, Label::Digit(digit), "{} maps wrong", key.label);
            }
        }
    }

    #[test]
    fn an_implausible_pixel_density_falls_back_to_one() {
        // SDL reports zero for a display it has not measured yet. Scaling a click by that sends
        // every press to the top-left corner, which looks like a dead keypad rather than a bad
        // reading — so an unusable number has to become the harmless one.
        assert_eq!(pixel_density(0.0), 1.0);
        assert_eq!(pixel_density(-2.0), 1.0);
        assert_eq!(pixel_density(f32::NAN), 1.0);
        assert_eq!(pixel_density(f32::INFINITY), 1.0);
        assert_eq!(pixel_density(1.0), 1.0);
        assert_eq!(pixel_density(2.0), 2.0);
    }

    #[test]
    fn a_density_of_one_leaves_a_point_where_it_was() {
        assert_eq!(window_to_pixels(640.0, 360.0, 1.0), (640.0, 360.0));
        assert_eq!(window_to_pixels(640.0, 360.0, 2.0), (1280.0, 720.0));
    }

    #[test]
    fn a_click_on_a_retina_window_still_lands_on_the_key_it_looks_like() {
        // The regression the density scaling exists for. The pad is laid out in backbuffer pixels
        // — 2560x1440 here — but the click arrives in the 1280x720 window coordinates the user is
        // actually pointing at. Feeding those through unscaled misses, which is what the display
        // did before `window_to_pixels`.
        let density = 2.0;
        let pad = idle_pad(2560.0, 1440.0);
        assert!(!pad.is_empty(), "a 1440p pad should exist");

        for key in pad.keys() {
            let (pixel_x, pixel_y) = key.center();
            // What the pointer would report for that same spot on screen.
            let (window_x, window_y) = (pixel_x / density, pixel_y / density);
            let (x, y) = window_to_pixels(window_x, window_y, density);
            assert_eq!(
                pad.hit(x, y),
                Some(key.action),
                "{} should claim a click at its own center",
                key.label
            );
        }
    }
}
