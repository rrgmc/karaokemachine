//! Drawing a frame.
//!
//! Generic over the render target, which is what lets the whole display be rendered to an off-screen
//! surface and saved as a PNG. A karaoke screen is judged by eye, and being able to look at a frame
//! without a monitor attached is the difference between checking the layout and hoping.
//!
//! Everything drawn here comes from a [`Frame`], which the caller assembles from the engine's
//! published state. This module reads no atomics and owns no state of its own.

use std::borrow::Cow;

use km_queue::QueueEntry;
use km_song::{KaraokeFlavor, LyricTimeline};
use sdl3::pixels::Color;
use sdl3::render::{BlendMode, Canvas, FRect, RenderTarget, Texture};
use sdl3::ttf::Font;

use crate::catalog::CatalogSummary;
use crate::connect::{ConnectInfo, ConnectPanel, QrMatrix};
use crate::keypad::{Keypad, Label};
use crate::lyrics::{LyricFrame, ROWS};
use crate::numbers::NumberEntry;
use crate::performance::FrameStats;
use crate::song_stats::{GainSource, SongMedia, SongStats};
use crate::text::{
    Align, Fonts, TextCache, TextStyle, WipeStyle, WipedLine, draw_text, draw_wiped_line,
    measure_line,
};
use crate::theme::Theme;

/// Which screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    /// Nothing playing: attract screen with the song-number prompt and the connect panel.
    #[default]
    Idle,
    /// A song is playing.
    Playing,
}

/// A short-lived message across the top of whichever screen is up.
///
/// **Distinct from [`Frame::faults`], and the difference is what each is *about*.** A fault line is
/// a standing statement about the catalog: it is true until somebody fixes it, it belongs to the
/// idle screen because that is where the catalog is the subject, and it stays there. A flash reports
/// something that just *happened* — a package dropped onto the window, and what became of it — so it
/// has to appear over a song as readily as over the idle screen, and it has to go away on its own.
///
/// The caller owns the clock. This crate is handed a string and a kind and draws them; how long a
/// message lives is a question about the machine, not about drawing, and `km-app` already keeps two
/// other deadlines of exactly this shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flash<'a> {
    /// What to say. One line — it is ellipsized rather than wrapped, because this band sits over a
    /// song's title and growing downwards would put it over the words.
    pub text: &'a str,
    /// How it went, which decides the color.
    pub kind: FlashKind,
}

/// How a [`Flash`] went.
///
/// Three states and not two: an install of a large package takes seconds, and a band that appeared
/// only once it had finished would leave the interval between dropping a file and anything
/// happening looking exactly like a file that was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlashKind {
    /// Under way, and nothing has gone wrong yet.
    #[default]
    Working,
    /// It worked.
    Done,
    /// It did not.
    Failed,
}

/// What is known about the song on screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SongInfo {
    /// Catalog number, when it came from a package.
    pub number: Option<km_songcode::SongCode>,
    /// Title, or the file name for a debug load.
    pub title: String,
    /// Performer, when the file says.
    pub artist: Option<String>,
    /// What it is sung in, **already as an English name** — `"Japanese"`, not `ja`.
    ///
    /// A word rather than the code, because that is what goes on a television: `ja` across a room is
    /// noise. Resolved by the caller, which is what keeps this crate free of a dependency on
    /// `km-kmpkg` purely to look up a name; it also means a code from a later build that this one
    /// does not know arrives as `None` rather than as `xx` drawn on screen.
    pub language: Option<String>,
}

/// Where a standing fault falls, for the one word the screen says about it.
///
/// The vocabulary the owner's Problems tab already uses, because one machine must not name one fault
/// two ways — see `A clash warns rather than only logging`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultArea {
    /// A package that would not install.
    Packages,
    /// No bank, a bank that would not parse, or no audio device at all.
    Sound,
}

impl FaultArea {
    /// The message id naming this area.
    fn id(self) -> &'static str {
        match self {
            Self::Packages => crate::words::FAULT_PACKAGES,
            Self::Sound => crate::words::FAULT_SOUND,
        }
    }
}

/// What is wrong with the machine, counted by area.
///
/// **The screen says how many and where, never why.** The whole of a reason is in `GET /packages`,
/// on the online remote and in `/admin/`'s Problems tab — and the last of those can *act* on it,
/// which no television can. Reproducing a diagnostic in the one place with the least room for it
/// meant the informative half was always the half that got cut.
///
/// Fields rather than a `Vec`, so a frame costs no allocation and a third area is one field and one
/// arm rather than a new shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Faults {
    /// Packages that would not install.
    pub packages: usize,
    /// Whatever `SoundFontStatus::complaint` has to say: 0 or 1.
    pub sound: usize,
}

impl Faults {
    /// How many there are, across every area.
    #[must_use]
    pub fn total(self) -> usize {
        self.packages + self.sound
    }

    /// Whether the machine has nothing to complain about.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.total() == 0
    }

    /// The areas with something in them, in a fixed order.
    ///
    /// Packages first, and the order is decided rather than incidental: a package problem means
    /// songs are *absent* and names a file somebody can act on now, where a sound problem is a
    /// setting or a device on a machine that is still playing.
    fn areas(self) -> impl Iterator<Item = FaultArea> {
        [
            (self.packages, FaultArea::Packages),
            (self.sound, FaultArea::Sound),
        ]
        .into_iter()
        .filter_map(|(count, area)| (count > 0).then_some(area))
    }

    /// The whole line: a count, then the areas it falls in.
    ///
    /// `None` on a machine with nothing wrong. Worded here rather than by the caller — which is a
    /// change from every other sentence on this screen, and the reason is that this one no longer
    /// needs to know what a package *is*. A count and an area are a number and an enum, so the words
    /// can live in this crate's catalog and stop being the one thing on the television that ignores
    /// the locale setting.
    #[must_use]
    pub fn line(self, locale: km_locale::Locale) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let words = crate::words::messages(locale);
        // Joined with `", "` for both shipped locales. A locale wanting another separator gets it
        // here, which is why the areas are resolved one at a time rather than as one message.
        let areas = self
            .areas()
            .map(|area| words.msg(area.id()).into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        Some(
            words
                .msg_with(
                    crate::words::NOTICE_FAULTS,
                    &[
                        ("count", (self.total() as i64).into()),
                        ("areas", areas.into()),
                    ],
                )
                .into_owned(),
        )
    }
}

/// Everything needed to draw one frame.
pub struct Frame<'a> {
    /// What language this screen speaks.
    ///
    /// **One machine, one locale, and it is on the frame rather than in this module's state**
    /// because this module has no state — everything drawn is decided by the caller and passed in,
    /// which is what lets the offscreen renderer and the contact sheet draw whatever they like.
    ///
    /// It reaches the words through [`Frame::words`]. The sentences the caller resolves for itself
    /// — [`Self::demo`], [`Self::soundfont_label`] and [`SongInfo::language`] —
    /// are already in this language by the time they arrive, and this crate never learns how.
    pub locale: km_locale::Locale,
    /// Which screen.
    pub screen: Screen,
    /// The song, when one is loaded.
    pub song: Option<&'a SongInfo>,
    /// Its lyrics, for the text of each visible line.
    pub timeline: Option<&'a LyricTimeline>,
    /// Which lines to show and where the highlight has reached.
    pub lyrics: LyricFrame,
    /// Position in milliseconds, for the progress bar.
    pub position_ms: u32,
    /// Song length in milliseconds.
    pub duration_ms: u32,
    /// Current transposition in semitones.
    pub transpose: i8,
    /// Current tempo as a multiple of the written tempo.
    pub tempo_ratio: f32,
    /// Guide melody: `None` when the song declares no melody channel, so the control is hidden
    /// rather than shown doing nothing.
    pub melody: Option<bool>,
    /// Whether the background layer carries the song's own words.
    ///
    /// True for a video song, whose words are pixels in somebody else's picture, and for an MP3+G
    /// song, whose words this application draws itself from CD+G tiles. In both cases there is no
    /// lyric timeline to highlight and no text to lay out, so it suppresses the same two things it
    /// always did: the lyric rows — including the "(no lyrics in this file)" fallback, which is a
    /// true and useful thing to say about a MIDI file and a misleading one over a picture — and the
    /// key and tempo badges, which name adjustments neither kind of song has.
    ///
    /// Named for what it *does* rather than for the one kind that first needed it: it was `video`,
    /// and a second kind wanting exactly the same treatment is what showed that to be the wrong
    /// question.
    pub picture: bool,
    /// Whether the position bar is drawn this frame.
    ///
    /// The bar visits the two ends of a song and the words have the screen between them, so the
    /// whole of that rule arrives here as one fact rather than as a term this crate composes.
    ///
    /// **The caller owns it**, the same seam [`Self::show_connect_overlay`] uses, and for a reason
    /// this crate cannot work around: through the first frames of a song [`Self::position_ms`] still
    /// reports the song before it, so how far into *this* song the audio has reached is a question
    /// only the frame loop can answer.
    ///
    /// **Not [`Self::keypad`] asked a second way**: a machine whose `display.keypad` is off has no
    /// transport strip, and a bar that waited for one would wait for ever.
    pub show_position: bool,
    /// Where the web UI is.
    pub connect: Option<&'a ConnectInfo>,
    /// How much is installed, for the idle screen to state.
    ///
    /// `Option` rather than a bare summary, and `None` is not the same as an empty catalog: it
    /// means *nobody has told this frame*, which is what the caller passes while the library is busy
    /// with an install. Drawing `No songs installed` for a second in the middle of installing four
    /// thousand of them would be a lie the counter told about itself. Idle only, like [`Self::faults`]
    /// — there is no catalog question while somebody is singing.
    pub catalog: Option<CatalogSummary>,
    /// The song-number keypad.
    pub number_entry: &'a NumberEntry,
    /// What is queued next, if anything.
    pub next_up: Option<&'a str>,
    /// Why music is playing when nobody asked for it, when that is the case.
    ///
    /// Resolved by the caller into a finished sentence, the same seam [`Self::soundfont_label`]
    /// and [`SongInfo::language`] use — this crate has no idea what a demo
    /// is and does not need one.
    ///
    /// It exists because the state it describes is otherwise indistinguishable from a fault. Music
    /// is coming out of the machine, the queue is empty, and nobody in the room started it; without
    /// a line saying so, the reasonable conclusion is that something is stuck. It also has to carry
    /// the one instruction that is not guessable — that **skipping**, not waiting, is what lets
    /// somebody sing — because the song has no queue entry to run out.
    pub demo: Option<&'a str>,
    /// Whether to show the connect panel over the playing screen.
    pub show_connect_overlay: bool,
    /// Everything waiting, in order. Empty when nothing is queued.
    pub queue: &'a [QueueEntry],
    /// Whether to show the queue over whatever screen is up.
    ///
    /// An overlay rather than a third [`Screen`]. A queue *screen* would replace the lyrics to show
    /// them — and the moment anybody wants the queue
    /// is mid-song, when somebody asks who is next. Drawn over the top, the answer costs nothing.
    pub show_queue_overlay: bool,
    /// Standing faults said out loud on the idle screen, counted by area.
    ///
    /// **Not a sentence the caller resolves**, unlike [`Self::demo`] and [`Self::soundfont_label`]
    /// beside it. A count and an area need no idea what a package is, so this crate can say them in
    /// the locale it is already drawing everything else in — see [`Faults::line`].
    ///
    /// Idle only: it is a statement about the catalog, and there is no catalog question while
    /// somebody is singing. [`Faults::default`] — nothing wrong — is also what the caller passes to
    /// suppress the line while a [`Self::flash`] is up, the two being indistinguishable on screen.
    pub faults: Faults,
    /// A message about something that has just happened, over whichever screen is up.
    ///
    /// Drawn as a band across the top, over the idle screen's fault line and over a song's title
    /// alike — a band rather than a floating panel because the playing screen already has text in
    /// both top corners, and something that overlapped them would be two messages in one place
    /// instead of one. It takes the [`Self::faults`] line's slot on the idle screen and the caller
    /// suppresses that line while it is up, on the same reasoning that keeps a dialled title away
    /// from an error: two sentences in one place read as one sentence.
    pub flash: Option<Flash<'a>>,
    /// On-screen touch targets to draw, when there are any.
    ///
    /// Drawn last, so it sits over everything else, and only when present: a keypad permanently
    /// covering the lyrics would be worse than reaching for a remote. The caller decides when — see
    /// [`crate::keypad`].
    pub keypad: Option<&'a Keypad>,
    /// Which SoundFont bank is playing, while the bank switcher is configured.
    ///
    /// `None` on every ordinary machine, and that is the switcher being off rather than the label
    /// being suppressed — see `debug.soundfonts`. When it is on this is drawn on **both** screens
    /// and is never timed out, which is the point: the reason to draw it at all is so that a
    /// recording or a photograph of the screen says which bank was being heard, and a label that
    /// faded would be absent from exactly the frame somebody kept.
    ///
    /// Resolved by the caller into a finished string, the same seam [`SongInfo::language`] uses. This crate knows nothing about banks or slots.
    pub soundfont_label: Option<&'a str>,
    /// What this machine has open that a machine in a living room should not, or `None` when
    /// nothing.
    ///
    /// **The one thing on this screen that exists so it cannot be left on unnoticed.** Debugging
    /// mode lets anybody on the network play a file off the machine's disk, and the development
    /// console goes further — its own copy of the API asks for no password at all. Both are settings
    /// somebody turns on to get something done and has no reason to remember afterwards, and neither
    /// left any mark anywhere a person in the room could see. So this one does, on **both** screens
    /// and for as long as it is true.
    ///
    /// **A state and not a sentence, unlike [`Self::demo`] and [`Self::soundfont_label`]** — the
    /// same call [`Self::faults`] makes and for its reason. Those two carry a *value* the caller
    /// holds and this crate could not name; two fixed states need no idea what a switch is, so they
    /// are worded here, in the locale everything else on this screen is already drawn in.
    pub developer_mode: Option<DeveloperMode>,
    /// What the frame meter measured, while somebody has asked to see it.
    ///
    /// `None` is the key not having been pressed. By value rather than by reference, unlike every
    /// other resolved field here — see [`FrameStats`] for why.
    ///
    /// **Drawn on both screens and never timed out**, for the reason the bank label above is: the
    /// thing being diagnosed may take a minute to happen again, and a panel that faded would be
    /// absent from exactly the second somebody was waiting for.
    pub performance: Option<FrameStats>,
    /// What the machine did to the loaded song, while somebody has asked to see it.
    ///
    /// Moves with [`Self::performance`] — one key, one panel — and is `None` for the second reason
    /// as well as the first: the idle screen has no song to describe, so the block is absent there
    /// however the key is set.
    ///
    /// By value, and never timed out, for both of that field's reasons.
    pub song_stats: Option<SongStats>,
    /// Which build this machine is, for the corner of the idle screen.
    ///
    /// Resolved by the caller into the finished string, the same seam [`Self::soundfont_label`] and
    /// [`Self::demo`] use. This crate could read its own `CARGO_PKG_VERSION` and be right, since one
    /// version number covers the whole repository — and it would still be the wrong crate to ask.
    /// What is drawn here is a fact about the *program*, and every other program-wide fact on this
    /// frame arrives the same way.
    ///
    /// `None` draws nothing, which is what the published screenshots pass: they render through this
    /// same code, and a number baked into them would change every picture in the README at every
    /// release.
    ///
    /// Idle only. A song is the show, and the playing screen never draws it however this is set.
    pub version: Option<&'a str>,
}

impl<'a> Frame<'a> {
    /// An idle frame with nothing loaded, in English.
    ///
    /// The locale is a field rather than an argument here because this constructor is a starting
    /// point somebody adjusts — every caller already sets several fields on the way past, and one
    /// that speaks another language sets this one too.
    pub fn idle(number_entry: &'a NumberEntry) -> Self {
        Self {
            locale: km_locale::Locale::default(),
            screen: Screen::Idle,
            song: None,
            timeline: None,
            lyrics: LyricFrame::default(),
            position_ms: 0,
            duration_ms: 0,
            transpose: 0,
            tempo_ratio: 1.0,
            melody: None,
            picture: false,
            show_position: false,
            connect: None,
            catalog: None,
            number_entry,
            next_up: None,
            demo: None,
            show_connect_overlay: false,
            queue: &[],
            show_queue_overlay: false,
            faults: Faults::default(),
            flash: None,
            keypad: None,
            soundfont_label: None,
            developer_mode: None,
            performance: None,
            song_stats: None,
            version: None,
        }
    }

    /// This screen's words.
    #[must_use]
    pub fn words(&self) -> &'static km_locale::Catalog {
        crate::words::messages(self.locale)
    }
}

/// The wallpaper layer for a frame.
#[derive(Default)]
pub struct Background<'a> {
    /// The image coming in.
    pub current: Option<&'a mut Texture>,
    /// The image going out, during a crossfade.
    pub outgoing: Option<&'a mut Texture>,
    /// Opacity of the incoming image, 0.0 to 1.0.
    pub fade: f32,
    /// How much to darken the result, so lyrics stay readable.
    pub dim: f32,
    /// The shape the picture must keep, as a width-to-height ratio.
    ///
    /// `None` — the wallpaper case — stretches to fill the screen, which is right because wallpapers
    /// are already resized to fit when they are decoded. `Some` letterboxes instead, preserving the
    /// ratio: a video is whatever shape it was filmed in, and stretching somebody's karaoke video to
    /// a 21:9 television would be worse than bars at the sides.
    ///
    /// **A ratio and not a size, which is a distinction the video path could afford to ignore.** A
    /// video passes its own pixel dimensions, and that works only because video pixels are square.
    /// CD+G pixels are not: the visible area is 288x192, which is 3:2, but it was drawn for a 4:3
    /// television, so passing its pixel size would stretch every word about 12% too wide. An
    /// anamorphic video would want the same treatment, and this is where that fix would go. See the
    /// `A CD+G pixel is not square` decision in `docs/decisions/`.
    pub shape: Option<(u32, u32)>,
}

/// Geometry shared by the drawing functions.
#[derive(Debug, Clone, Copy)]
struct Layout {
    w: f32,
    h: f32,
    margin: f32,
}

/// Draws a whole frame.
pub fn draw<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    background: Background<'_>,
) {
    let (width, height) = canvas.output_size().unwrap_or((1920, 1080));
    let layout = Layout {
        w: width as f32,
        h: height as f32,
        margin: theme.margin_px(width),
    };

    draw_background(canvas, theme, layout, background);

    match frame.screen {
        Screen::Idle => draw_idle(canvas, cache, fonts, theme, frame, layout),
        Screen::Playing => draw_playing(canvas, cache, fonts, theme, frame, layout),
    }

    // Here rather than inside either screen, for the reason the bank label below is: how many are
    // waiting is a property of the machine and not of what happens to be playing. It goes first of
    // the two because it is the top row and the label the second, and they cannot collide.
    //
    // Under every overlay, like the label, and **not suppressed under a flash** — which was the
    // other candidate and is the wrong one. The band ends two pixels below this row's top edge at
    // every screen size, so it crosses only the apex of the pill, where a circle is a few pixels
    // wide: the same two-pixel overlap the song title and the badge run have always taken. Hiding
    // the count for the twelve seconds a refused drop is up would cost more than that sliver, and it
    // would be the one overlay that took a standing fact off the screen rather than covering it.
    draw_queue_pill(canvas, cache, fonts, theme, frame, layout);

    // Here rather than inside either screen, because it has to appear on both: the bank is a
    // property of the machine and not of what happens to be playing, and a label that vanished
    // between songs would be missing from the idle screen somebody photographs to report a fault.
    //
    // Under the three overlays below, which is deliberate. Each of them is something a person asked
    // to see this instant, and none of them stays up; a standing label has no business covering
    // one.
    if let Some(label) = frame.soundfont_label {
        draw_soundfont_label(canvas, cache, fonts, theme, label, layout);
    }

    // The row under it, on the same terms and for a stronger version of the same reason: this is a
    // property of the machine rather than of a song, so it belongs on both screens and for as long
    // as it is true. Under the overlays too — none of them stays up, and a person who asked to see
    // one this instant is not the reader this is for.
    if let Some(mode) = frame.developer_mode {
        draw_developer_label(canvas, cache, fonts, theme, mode, frame.locale, layout);
    }

    // Over the screen, under the keypad: the queue answers a question, and the keypad has to stay
    // pressable while it is being answered.
    if frame.show_queue_overlay {
        draw_queue_overlay(
            canvas,
            cache,
            fonts,
            theme,
            frame.queue,
            layout,
            frame.words(),
        );
    }

    // Over the queue, because it reports something that just happened and the queue is a standing
    // answer to a question; under the keypad, for the same reason the queue is.
    if let Some(flash) = frame.flash {
        draw_flash(canvas, cache, fonts, theme, flash, layout);
    }

    // Over the flash and under the keypad. It is a standing panel rather than something that just
    // happened, so it must not cover a message the machine is trying to deliver — and like every
    // other overlay it must not cover a touch target.
    if let Some(stats) = frame.performance {
        draw_performance(
            canvas,
            cache,
            fonts,
            theme,
            Diagnostics {
                stats: &stats,
                song: frame.song_stats.as_ref(),
                position_ms: frame.position_ms,
                duration_ms: frame.duration_ms,
            },
            layout,
            frame.words(),
        );
    }

    // Last, so touch targets are never drawn under something they would be pressed through.
    if let Some(keypad) = frame.keypad {
        draw_keypad(canvas, cache, fonts, theme, keypad, layout, frame.words());
    }
}

/// Draws a [`Flash`] as a band across the top of the screen.
///
/// **A full-width band and not a floating panel.** The playing screen writes the song's title into
/// the top left and its key/tempo badges into the top right, both at `0.05h`, so anything centered
/// there would land between two pieces of text and read as part of one of them. A band covers the
/// row instead, which is what a notification bar does everywhere else and what makes it obvious that
/// the machine is speaking rather than the song.
///
/// One line, ellipsized. Growing downwards would reach the lyrics, and there is nothing this band
/// says that needs two lines — the API and the remotes carry the whole of a failure, exactly as they
/// do for a standing notice.
fn draw_flash<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    flash: Flash<'_>,
    layout: Layout,
) {
    let small_px = f32::from(theme.small_px(layout.h as u32));
    let band_h = small_px * FLASH_BAND_LINES;

    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(theme.panel);
    let _ = canvas.fill_rect(FRect::new(0.0, 0.0, layout.w, band_h));
    // A rule under it, in the message's own color: the panel is only ~78% opaque, so over a bright
    // wallpaper the band's lower edge can be hard to find, and a message with no visible boundary
    // reads as text that belongs to whatever is beneath it.
    canvas.set_draw_color(flash_color(theme, flash.kind));
    let _ = canvas.fill_rect(FRect::new(0.0, band_h - 2.0, layout.w, 2.0));

    let width = layout.w - layout.margin * 2.0;
    let text = ellipsize(flash.text, fit_chars(width, small_px));
    draw_text(
        canvas,
        cache,
        &fonts.small,
        &text,
        (layout.w / 2.0, (band_h - small_px) / 2.0),
        &TextStyle::outlined(flash_color(theme, flash.kind), theme, Align::Center),
    );
}

/// What color a [`FlashKind`] is drawn in.
///
/// `accent_alt` finally has a screen that draws it. It was added for the application icons and has
/// been unused by the display since — and success is exactly what it is for: the alert red already
/// means *something is wrong*, and reporting a package that installed perfectly well in the color
/// reserved for faults would teach that color a second meaning.
fn flash_color(theme: &Theme, kind: FlashKind) -> Color {
    match kind {
        FlashKind::Working => theme.text,
        FlashKind::Done => theme.accent_alt,
        FlashKind::Failed => theme.alert,
    }
}

/// How much taller than its two lines of text a key must be to carry a hint.
///
/// **Exactly one, and the number was arrived at by looking rather than by taste.** The rule is the
/// honest one — the label and the hint must physically fit inside the key — and it can be that tight
/// because a font's line height already carries its own leading, so two lines that just fit are two
/// lines with air between the glyphs. A comfortable-looking clearance was tried first and dropped
/// the hints at 1280x720, where a transport key is 54 px against about 51 px of text: an ordinary
/// desktop window, which is precisely the machine the hints exist for.
///
/// What it still has to catch is the strip shrunk to fit a phone in portrait. There the two lines
/// come to several times the key's height and the question is not close.
const HINT_CLEARANCE: f32 = 1.0;

/// How tall the flash band is, in lines of the small face.
///
/// One line of text with half a line of air above and below it. Named rather than spelled inline
/// because a test measures the band against the top of the song title.
const FLASH_BAND_LINES: f32 = 2.0;

/// How thick the position bar is, as a fraction of height.
const TIMELINE_THICKNESS: f32 = 0.006;

/// Air between the position bar and the transport strip above it, as a multiple of the bar's own
/// thickness.
const TIMELINE_STRIP_GAP: f32 = 1.0;

/// How thick the position bar is, in pixels.
///
/// The floor is what makes this a function rather than a fraction spelled at each site: a small
/// window's bar is two pixels whatever the fraction says, and [`position_reserve`] has to give up
/// the height the bar will actually take.
fn timeline_thickness(height: f32) -> f32 {
    (height * TIMELINE_THICKNESS).max(2.0)
}

/// Where the position bar sits.
///
/// **Its bottom edge rests on the bottom safe inset**, which is the lowest line a television shows.
/// Lower is not further from the words, it is off the screen: a set overscans by a percentage of
/// each axis, and a bar below the inset goes behind the bezel along with everything else there.
///
/// The 5% side inset stays, so the bar reads as the machine's own object rather than as part of the
/// picture behind it.
fn timeline_rect(layout: Layout) -> FRect {
    let thickness = timeline_thickness(layout.h);
    let bottom = layout.h - crate::keypad::margin_y(layout.h);
    FRect::new(
        layout.margin,
        bottom - thickness,
        layout.w - layout.margin * 2.0,
        thickness,
    )
}

/// How much of the bottom the transport strip gives up so the position bar sits under it.
///
/// The twin of [`version_reserve`] and the same arrangement: the pad stands above the build number
/// on the idle screen, the strip stands above the bar on the playing one. **Taken from the bar's own
/// thickness rather than named**, so a bar that changes takes the strip with it and there is nowhere
/// for the two to disagree.
///
/// Public because the strip is laid out by whoever is drawing, and this is the only thing they have
/// to know about the band under it.
#[must_use]
pub fn position_reserve(height: f32) -> f32 {
    timeline_thickness(height) * (1.0 + TIMELINE_STRIP_GAP)
}

/// Where the playing screen's first row sits, as a fraction of height.
///
/// The title on the left and the key/tempo/melody badges on the right share it, which is what makes
/// it one constant rather than two — and what a collision test has to be able to name. Spelled as a
/// literal `0.05` at both draws until 2026-09-07, and *transcribed a third time into the test that
/// checks the badges clear the label below them*: moving the row would have left that test passing
/// about a number nothing drew any more.
pub(crate) const TITLE_ROW_TOP: f32 = 0.05;

/// Where the playing screen's second row sits, under [`TITLE_ROW_TOP`].
///
/// `artist · language` on the left, and the SoundFont label on the right — see
/// [`SOUNDFONT_LABEL_MAX_WIDTH`], which is the rule that keeps the two from meeting.
pub(crate) const SECOND_ROW_TOP: f32 = 0.11;

/// Where `next: …` sits, under [`SECOND_ROW_TOP`].
///
/// **The third row of the top left, and the top is where it has to be.** A video or MP3+G song's
/// words are pixels in its own picture and they sit low in the frame, so a line along the bottom of
/// the screen stands on the line somebody is singing. This column is the machine's own for three
/// rows on every screen, and nothing a song brought is in it.
///
/// Equal to [`DEVELOPER_LABEL_TOP`] by definition rather than by coincidence: it *is* that row, seen
/// from the left-hand side, the same way [`SOUNDFONT_LABEL_TOP`] is [`SECOND_ROW_TOP`] seen from the
/// right. What keeps the two apart is [`NEXT_UP_MARKED_WIDTH`].
pub(crate) const NEXT_UP_TOP: f32 = 0.17;

/// The most of the width `next:` may take while the developer marker is up, as a fraction.
///
/// **Everything the marker leaves**, so the two share a band rather than a column and can only meet
/// if one grows past its own cap. Defined against [`DEVELOPER_LABEL_MAX_WIDTH`] instead of spelled,
/// because a cap that is the remainder of another cap is not a number that may drift from it.
///
/// The cap applies only while the marker is drawn. The marker is a fault somebody has to see and the
/// line is ambient, so the line gives up the room — and on the machines where no marker is up it
/// keeps the whole row, which is what a queue label full of accented Latin text needs.
const NEXT_UP_MARKED_WIDTH: f32 = 1.0 - DEVELOPER_LABEL_MAX_WIDTH;

/// Where the *no lyrics* line is centred, as a fraction of height.
///
/// Named because the collision test below needs to say "the label must not reach the lyrics", and
/// was spelling `0.45` itself to do it.
pub(crate) const NO_LYRICS_TOP: f32 = 0.45;

/// Where the SoundFont label's top edge sits, as a fraction of height.
///
/// **The second row of the top right**, directly under the key/tempo/melody badges, because that
/// corner is already where this screen puts machine state — and which bank is playing is machine
/// state of exactly that kind rather than anything a singer did.
///
/// Equal to [`SECOND_ROW_TOP`] by definition rather than by coincidence: it *is* that row, seen from
/// the right-hand side.
const SOUNDFONT_LABEL_TOP: f32 = SECOND_ROW_TOP;

/// The most of the width the label may take, as a fraction.
///
/// It shares its row with the playing screen's `artist · language` line, which is drawn from the
/// left and is not itself shortened — the same arrangement the title and the badges have on the row
/// above. Capping the label is what keeps that a shared row rather than a collision: the two can
/// only meet if the artist line alone runs past `1.0 - this`.
const SOUNDFONT_LABEL_MAX_WIDTH: f32 = 0.45;

/// How much taller the pill is than the line of digits in it.
///
/// **A pill is not a box, and this is the number that gets that wrong.** It was 1.0 — the pill
/// exactly the height of its text — and the contact sheet showed why that fails: a rendered line is
/// nearly all glyph, so the digits touched the disc top and bottom. Worse, they *left* it: away from
/// its centre line an ellipse narrows, so at the top and bottom of a `2` the shape has pulled in and
/// the corners of the glyph stand outside the curve. A rectangle would have been fine at 1.0. This
/// one needs the air, and needs it for a reason arithmetic over a bounding box does not show.
///
/// Bounded from above by [`SECOND_ROW_TOP`]: the pill stops before the artist line and the bank
/// label, and the row is only `0.06h` tall. That bound is also why the digits are in the **small**
/// face — 26 pixels of line at 720p against the title face's 36, in a row 43 pixels deep. A pill
/// holding the title face cannot have air and clear the row both, and the air is not optional.
/// `the_queue_pill_owns_the_corner_and_clears_the_second_row` holds the clearance.
const QUEUE_PILL_HEIGHT: f32 = 1.45;

/// The air at each end of the digits, as a fraction of the pill's height.
///
/// **Capped by the promise that one digit comes out round.** The caps are semicircles of radius
/// `h/2`, so a circle needs `digit + 2·pad ≤ h`. A digit in the small face is about half its line
/// height, against a pill 1.45 of that, so the padding has room to be generous — and it has to be,
/// for [`QUEUE_PILL_HEIGHT`]'s reason: the digits must clear the curve, not merely the bounding box.
/// `a_one_digit_count_comes_out_round` holds the circle against a change to either constant.
const QUEUE_PILL_SIDE_PAD: f32 = 0.5;

/// How far the key/tempo/melody run keeps off the pill, as a fraction of width.
///
/// Narrower than the three-space gap *between* badges, deliberately: those three are one run and
/// this is the boundary between the run and something that is not part of it.
const QUEUE_PILL_GAP: f32 = 0.012;

/// Where the demo line sits, as a fraction of height: a row of its own, low on the screen.
///
/// **The row is the whole point, and it is [`DEMO_LABEL_MIN_CHARS`] that buys it.** A line sharing a
/// row is a line capped at part of the width, and this one may not be cut: an instruction ellipsized
/// is a machine that has said there is something to do and not what. A bank name ellipsized is still
/// a bank name, which is why the label one row from the top shares its row happily and this does not.
///
/// **Low, because it speaks to somebody who has just walked in** — the same reason it is never timed
/// out. It is the only line down here for the length of a demo song, and it clears the lyric band
/// above (about 0.66 with the shipped [`Theme`]) and the position bar below;
/// `the_demo_line_clears_the_lyrics_above_it_and_the_position_bar_below` holds both ends.
const DEMO_LABEL_TOP: f32 = 0.82;

/// The most of the width the demo line may take, as a fraction.
///
/// Nearly all of it, because it has the row to itself and the sentence is the point. It is capped at
/// all so that a translation cannot run under the margin, and so that it stops short of the standing
/// connect panel in the corner beside it.
const DEMO_LABEL_MAX_WIDTH: f32 = 0.95;

/// The shortest the demo line may be allowed to come out, in characters.
///
/// `DEMO · QUEUE to sing next` — what `karaokemachine`'s `DEMO_LABEL` holds — is exactly this long,
/// and the wording was chosen against the measurement rather than the other way round. **It is a
/// floor rather than a target**: anything less cuts the instruction, and a demo line that says there
/// is something to do without saying what is worse than no demo line at all.
///
/// **Four characters of headroom, and a rewording spends them.** The portrait phone in `SCREENS`
/// fits 29, so a fuller `DEMO · QUEUE a song to sing` takes three — on the one screen where
/// [`draw_playing`] then stops standing the connect panel beside the line. Measure a new wording
/// here before judging how it reads.
///
/// Named because two things now read it. `the_whole_instruction_fits_on_every_screen` asserts it,
/// and [`draw_playing`] refuses to stand the connect panel beside a line this would cut.
const DEMO_LABEL_MIN_CHARS: usize = 25;

/// The standing connect panel's QR code, as a fraction of screen height.
///
/// Height rather than width, and the QR rather than the panel — see [`PanelSize::rect`]. At 1080 it
/// is 151 pixels against the `Overlay` panel's 124, which is the claim the whole shape rests on: a
/// **smaller card carrying a larger code**, because the text column is gone. Two thirds the area, in
/// numbers, and it was never the fifth this said for as long as it said it.
const STANDING_QR_SIDE: f32 = 0.14;

/// The idle panel's width, and the `I` overlay's, as fractions of screen width.
///
/// **The one axis that is still the screen's rather than the contents'**, and deliberately:
/// [`PanelSize::rect`] answers callers holding no [`ConnectPanel`], and what a text panel's width
/// decides is how much of a URL fits on a line — not what any one message happens to need. Named
/// because [`Card`] and `rect` both want them, and a fraction spelled twice is a fraction that can
/// drift.
const FULL_WIDTH: f32 = 0.55;
const OVERLAY_WIDTH: f32 = 0.36;

/// The most of a panel's inner width its QR code may take.
///
/// **The cap that makes a square in a rectangle safe.** The code is sized from the height and the
/// panel from the width, and on a portrait phone the two disagree violently: at 1080x2400 the idle
/// panel is 594 pixels wide and [`FULL_QR_SIDE`] of that height is 384, which leaves the text column
/// *negative* and draws the detail across the code. Sizing it from the height alone puts the code
/// at 565 there, which is the fault this cap closes.
const QR_MAX_WIDTH: f32 = 0.40;

/// The idle panel's QR code, as a fraction of screen height.
///
/// **A size of its own rather than whatever is left of the box.** Deriving it as `panel_h - pad *
/// 2`, with the pad taken from the panel's *width*, makes the code a by-product of two numbers that
/// have nothing to do with each other — and a box grown to hold its text grows its code with it.
/// `0.16` is what that arithmetic produces at every screen size.
const FULL_QR_SIDE: f32 = 0.16;

/// The `I` overlay's QR code, as a fraction of screen height.
///
/// **The number [`STANDING_QR_SIDE`]'s doc quotes**, and the reason both are named: the standing
/// card's whole argument is that it carries a *larger* code than this one, and a leftover cannot be
/// compared against. `the_standing_panel_holds_a_bigger_qr_than_the_overlay_it_replaces` asserts the
/// two against each other rather than against copies of the old expression.
const OVERLAY_QR_SIDE: f32 = 0.115;

/// How many wrapped lines of detail the connect panel says, and is sized to hold.
///
/// Three, in the small face beside the code. **The panel is built around this number rather than
/// checked against it**: [`Card`] adds three lines to the headline and the address and the box is
/// whatever that comes to, which is what stopped the second and third lines being drawn under the
/// panel's bottom edge. A fourth is cut with the same `…` [`NOTICE_MAX_LINES`] uses, and for the
/// same reason — the API and the log carry the whole of it, and a sentence that stopped dead reads
/// as a message with nothing after its colon.
const DETAIL_MAX_LINES: usize = 3;

/// The most lines the key hint may take, out of [`DETAIL_MAX_LINES`] rather than beside them.
///
/// **The hint shares the panel's small rows and does not add one**, which is the whole of how it
/// fits. All three panels are bottom-anchored under the lyric band and none of them has room to
/// grow: `2560x1080` in `SCREENS` leaves the standing card **nine pixels** of clearance against a
/// row that costs forty-four, and shrinking the code to the overlay's — the one size it may not go
/// below — buys twenty-seven of them.
/// `the_key_hint_shares_the_panels_rows_rather_than_growing_its_box` measures that, so the next
/// person weighing a fourth row finds the number rather than a failing test to work backwards from.
///
/// **The three states that fill all three detail rows are the three failures, and none of them
/// carries a hint** — a panel with one is a reachable machine, whose detail is a count of addresses
/// and a PIN. So the budget is spent detail-first and the hint takes what is left. On the portrait
/// phone, where nine characters fill a line and the detail alone runs to three, what is left is
/// nothing and the line is not drawn; that is the screen which already cuts the loopback sentence.
const HINT_MAX_LINES: usize = 2;

/// The air around the standing panel's QR code, as a fraction of screen height.
///
/// Its own number rather than [`Card`]'s half a line of the small face, because there is nothing in
/// here to separate from anything — one code and one line — and because the quiet zone `draw_qr`
/// draws inside the code is the margin a scanner actually cares about.
const STANDING_PAD: f32 = 0.02;

/// The longest address the standing panel's caption can be asked to print, in characters.
///
/// `http://255.255.255.255:65535` is 28, and it is a genuine ceiling rather than a guess:
/// `km_api::connect` builds every URL as `http://<IPv4>:<port>` and offers no hostname, **because a
/// URL on a QR code has to be typeable** — which is the same reason this caption exists at all.
///
/// The panel is sized to hold it whatever address it is actually given, so the caption is never
/// shortened and the panel never changes width between one machine and the next.
const STANDING_URL_CHARS: usize = 28;

/// Draws the bank the machine is playing through, top right, under the badges.
///
/// Dim and small, in the same face and color as `next:` and the artist line — it is a standing fact
/// about the machine, and it is on screen for the whole song competing with the lyrics, so it is
/// deliberately quieter than the badges above it. Outlined, because it sits over a wallpaper.
fn draw_soundfont_label<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    label: &str,
    layout: Layout,
) {
    let small_px = f32::from(theme.small_px(layout.h as u32));
    let width = (layout.w - layout.margin * 2.0) * SOUNDFONT_LABEL_MAX_WIDTH;
    let text = ellipsize(label, fit_chars(width, small_px));
    draw_text(
        canvas,
        cache,
        &fonts.small,
        &text,
        (layout.w - layout.margin, layout.h * SOUNDFONT_LABEL_TOP),
        &TextStyle::outlined(theme.text_dim, theme, Align::Right),
    );
}

/// What a machine being worked on has open, for the marker on its own screen.
///
/// **Two states and not three.** The console needs debugging mode as well, so *console on, debugging
/// off* is not a state anything is open in — it is a switch waiting for a restart, and the two admin
/// pages are where that gets explained. This is about what is true of the machine drawing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeveloperMode {
    /// Debugging mode is on: anybody on the network can play a file off this machine's disk.
    Debugging,
    /// The development console is being served, so debugging is on **and** the whole API is
    /// reachable a second time with no password.
    Console,
}

impl DeveloperMode {
    /// What to draw, in the machine's own language.
    fn label(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Debugging => words.msg(crate::words::DEVELOPER_DEBUGGING).into_owned(),
            Self::Console => words.msg(crate::words::DEVELOPER_CONSOLE).into_owned(),
        }
    }
}

/// Where the developer marker sits, as a fraction of height.
///
/// **A third row of the top right, under the bank label**, and the argument is
/// [`SOUNDFONT_LABEL_TOP`]'s own: that corner is already where this screen puts machine state, and
/// what this says is machine state of exactly that kind rather than anything a singer did.
///
/// **The other corners were not available, which is worth writing down rather than re-deriving.**
/// The bottom right is the connect panel's for the length of a song, the bottom left is the demo line
/// and the timeline, and the top left is the title on one screen and the frame panel on both. This
/// band is the one place a standing fact can sit on *both* screens without covering something.
///
/// **It shares its band with two things drawn from the left margin, and a column with neither.**
/// [`PERFORMANCE_TOP`] puts the frame panel at 0.16, above this and running down past it; [`NEXT_UP_TOP`]
/// is this row exactly. Both are left-hand things against a right-aligned one, so a meeting takes a
/// growth past a cap: [`DEVELOPER_LABEL_MAX_WIDTH`] from this side, and [`NEXT_UP_MARKED_WIDTH`] from
/// the other. Tests hold both, because all three are on screen at once exactly when somebody is
/// diagnosing something.
const DEVELOPER_LABEL_TOP: f32 = 0.17;

/// The most of the width the developer marker may take, as a fraction.
///
/// [`SOUNDFONT_LABEL_MAX_WIDTH`]'s value, for its reason: it shares a band with something drawn from
/// the left, and a cap is what keeps that a shared band rather than a collision.
const DEVELOPER_LABEL_MAX_WIDTH: f32 = 0.45;

/// Draws what this machine has open that it should not, top right, under the bank label.
///
/// **In `alert` and not `text_dim`**, which is the one way this differs from the bank label it sits
/// under. That one is a fact somebody may want; this is a fact somebody needs, and the whole reason
/// it is drawn is that both switches behind it are easy to leave on for a month. It is the same
/// color the standing fault line uses, because it is the same kind of statement.
fn draw_developer_label<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    mode: DeveloperMode,
    locale: km_locale::Locale,
    layout: Layout,
) {
    let small_px = f32::from(theme.small_px(layout.h as u32));
    let width = (layout.w - layout.margin * 2.0) * DEVELOPER_LABEL_MAX_WIDTH;
    let text = ellipsize(&mode.label(locale), fit_chars(width, small_px));
    draw_text(
        canvas,
        cache,
        &fonts.small,
        &text,
        (layout.w - layout.margin, layout.h * DEVELOPER_LABEL_TOP),
        &TextStyle::outlined(theme.alert, theme, Align::Right),
    );
}

/// The two blocks the diagnostic panel draws, so its contents are one argument rather than two.
///
/// **Grouped because they are one panel and not to shorten a list.** They are measured in different
/// places and mean different things — one is a second of frames, the other is a file — but they
/// share a box, a width and a row pitch, and every decision about any of those is about both.
struct Diagnostics<'a> {
    stats: &'a FrameStats,
    song: Option<&'a SongStats>,
    /// Where playback has reached, from the frame rather than from [`SongStats`]: the stats are
    /// taken once when a song starts, and this moves every frame.
    position_ms: u32,
    /// The song's length, `0` when the file declares none.
    duration_ms: u32,
}

/// How far down the screen the performance panel starts.
///
/// Below the title-and-artist header on the playing screen and below the flash band on both, which
/// is the one thing that must never be covered: a flash is the machine answering a person, and this
/// is a diagnostic nobody pressed anything to dismiss. Chosen by looking at the rendered frame
/// rather than derived — the header's own constants are point sizes, not a bottom edge.
const PERFORMANCE_TOP: f32 = 0.16;

/// How wide the panel is, in characters of the small face.
///
/// `levelled  pkg -18.2 LUFS` is the longest row, so the panel is sized to that and every other row
/// sits inside it. Fixed rather than measured because every row is either a number formatted to a
/// known number of places or a value cut to the column, so the widest row is bounded at compile
/// time and cannot surprise this at run time.
///
/// **What holds the bound with words in the panel is the alignment.** The value is right-aligned at
/// the panel's right edge and grows leftwards, so a value wider than expected can only reach into
/// the panel's own padding; the only thing running rightwards is the label, and labels come from
/// this crate's catalog.
///
/// Thirty-two rather than the twenty-six the frame block alone needed: `damage  cut 1  gone 2
/// notes 47` is the widest row a song can produce, and it is the one worth fitting whole — a file
/// damaged three ways is exactly when somebody is reading this.
const PERFORMANCE_COLUMNS: f32 = 32.0;

/// Draws what the frame meter measured, as a panel near the top-left corner.
///
/// **Under the title, not over it.** The playing screen writes the song's title and artist into the
/// top-left corner and its badges into the top-right, and [`PERFORMANCE_TOP`] puts the panel below
/// both, in the band between that header and the lyric ladder. The right-hand side was rejected
/// because the badges are there and the bank label sits directly under them: three things, against
/// one.
///
/// **The one thing it covers is `next:`, and that is the precedence this screen already keeps.** The
/// panel is drawn from the left margin at [`PERFORMANCE_TOP`] and the line sits at [`NEXT_UP_TOP`]
/// just under it, so the two overlap while the panel is up. A panel somebody switched on outranks an
/// ambient line, which is the rule the transport strip is given over the demo line — and a `F12`
/// panel is on screen for as long as somebody is reading it and gone the moment they are not.
///
/// Two columns and no more: the mean, then the worst. Everything is milliseconds except the four
/// counters, which are counts for the window and are drawn **only when one of them is non-zero** —
/// on every MIDI song three of the four are structurally zero, because there is no decoder thread
/// at all, so drawing them always would fill half the panel with noughts meaning "not applicable"
/// rather than "nothing went wrong".
fn draw_performance<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    contents: Diagnostics<'_>,
    layout: Layout,
    words: &km_locale::Catalog,
) {
    let Diagnostics {
        stats,
        song,
        position_ms,
        duration_ms,
    } = contents;
    let small_px = f32::from(theme.small_px(layout.h as u32));
    let row_h = small_px * 1.5;
    let pad = small_px * 0.8;

    // Sized from the text rather than from the screen, and it is the one panel here that is: every
    // row is a known number of characters wide, so a fraction of the width would be too wide on a
    // 21:9 television and too narrow on a phone in portrait. The half-point-size estimate is
    // `fit_chars` read backwards.
    let panel_w = PERFORMANCE_COLUMNS * small_px * 0.5 + pad * 2.0;
    // Each block counts its own rows, so the height is bounded by something a test can ask for
    // without a font. See `FrameStats::rows` and `SongStats::rows`.
    let rows = stats.rows() + song.map_or(0, SongStats::rows);
    let panel_h = pad * 2.0 + row_h * rows as f32;
    let x = layout.margin;
    let y = layout.h * PERFORMANCE_TOP;

    panel(canvas, theme, FRect::new(x, y, panel_w, panel_h));

    let left = x + pad;
    let right = x + panel_w - pad;
    let mut row = y + pad;

    // **Both the canvas and the cache are arguments rather than captures**, which is the one thing
    // the text cache changed about this function: the cache is borrowed mutably and every row uses
    // it, so a closure that held it could be called once and never again.
    let line = |canvas: &mut Canvas<T>,
                cache: &mut TextCache<C>,
                label: &str,
                value: &str,
                color,
                row: f32| {
        draw_text(
            canvas,
            cache,
            &fonts.small,
            label,
            (left, row),
            &TextStyle::plain(theme.text_dim, Align::Left),
        );
        draw_text(
            canvas,
            cache,
            &fonts.small,
            value,
            (right, row),
            &TextStyle::plain(color, Align::Right),
        );
    };

    // The heading carries the state, so a panel that has measured nothing yet says so rather than
    // showing a screenful of zeroes that read as a machine doing nothing.
    let heading = if stats.strained() {
        theme.alert
    } else {
        theme.accent
    };
    // **Labelled rather than returning**, so the two states that end this block early end only this
    // block. A song is loaded or it is not, and the meter having nothing to say yet is no reason to
    // stop saying what was done to the song.
    'frames: {
        if stats.frames == 0 {
            line(
                canvas,
                cache,
                &words.msg(crate::words::FRAMES_HEADING),
                &words.msg(crate::words::FRAMES_MEASURING),
                heading,
                row,
            );
            row += row_h;
            break 'frames;
        }
        line(
            canvas,
            cache,
            &words.msg(crate::words::FRAMES_HEADING),
            &format!("{:.1}/s", stats.fps),
            heading,
            row,
        );
        row += row_h;

        let tight = if stats.frame_is_tight() {
            theme.alert
        } else {
            theme.text
        };
        for (label, mean, worst, color) in [
            ("frames-draw", stats.draw_ms, stats.draw_worst_ms, tight),
            (
                "frames-present",
                stats.present_ms,
                stats.present_worst_ms,
                theme.text,
            ),
            (
                "frames-interval",
                stats.interval_ms,
                stats.interval_worst_ms,
                theme.text,
            ),
        ] {
            line(
                canvas,
                cache,
                &words.msg(label),
                &format!("{mean:.1} / {worst:.1} ms"),
                color,
                row,
            );
            row += row_h;
        }

        if !stats.decoder_complained() {
            break 'frames;
        }
        for (label, value, unit) in [
            ("frames-starved", stats.starved_ms, " ms"),
            ("frames-dropped", stats.dropped, ""),
            ("frames-late", stats.late, ""),
            ("frames-xruns", stats.xruns, ""),
        ] {
            let color = if value > 0 {
                theme.alert
            } else {
                theme.text_dim
            };
            line(
                canvas,
                cache,
                &words.msg(label),
                &format!("{value}{unit}"),
                color,
                row,
            );
            row += row_h;
        }
    }

    let Some(song) = song else {
        return;
    };

    // Every value is cut to whatever the column actually holds, which is what lets the panel's width
    // be a constant with words in it: a translation longer than the English cannot push a row out of
    // the box. Two characters for the gap between the columns.
    let room =
        |label: &str| fit_chars(right - left, small_px).saturating_sub(label.chars().count() + 2);

    let kind = match song.kind {
        SongMedia::Midi => words.msg_with(
            crate::words::SONG_KIND_MIDI,
            &[("tracks", song.tracks.into())],
        ),
        SongMedia::Video => words.msg(crate::words::SONG_KIND_VIDEO),
        SongMedia::Cdg => words.msg(crate::words::SONG_KIND_CDG),
        SongMedia::UltraStar => words.msg(crate::words::SONG_KIND_ULTRASTAR),
    };
    let song_heading = if song.worth_attention() {
        theme.alert
    } else {
        theme.accent
    };
    let heading_label = words.msg(crate::words::SONG_HEADING);
    line(
        canvas,
        cache,
        &heading_label,
        &ellipsize(&kind, room(&heading_label)),
        song_heading,
        row,
    );
    row += row_h;

    // **Under the heading, for every kind of song**: the moment a file goes wrong is what somebody
    // reads off to find it again. A file that declares no length draws the position alone.
    let position = if duration_ms > 0 {
        format!("{} / {}", clock(position_ms), clock(duration_ms))
    } else {
        clock(position_ms)
    };
    let position_label = words.msg(crate::words::SONG_POSITION);
    line(
        canvas,
        cache,
        &position_label,
        &ellipsize(&position, room(&position_label)),
        theme.text,
        row,
    );
    row += row_h;

    // The factor is what the audio thread applies and the decibels are what a person reasons in, so
    // the row carries both rather than making anybody convert between them.
    let gain_label = words.msg(crate::words::SONG_GAIN);
    line(
        canvas,
        cache,
        &gain_label,
        &format!("{:.2} ({:+.1} dB)", song.gain, song.gain_db()),
        if song.gain_is_steep() {
            theme.alert
        } else {
            theme.text
        },
        row,
    );
    row += row_h;

    // **Dim when nothing levelled the song**, because those two states are the machine declining to
    // act rather than a measurement it took, and drawing them in the same weight as a real one would
    // make `off` look like a number.
    let (levelled, levelled_color) = match song.gain_source {
        GainSource::Package { lufs } => (
            words.msg_with(
                crate::words::SONG_LEVELLED_PACKAGE,
                &[("lufs", format!("{lufs:.1}").into())],
            ),
            theme.text,
        ),
        GainSource::Events { db } => (
            words.msg_with(
                crate::words::SONG_LEVELLED_EVENTS,
                &[("db", format!("{db:.1}").into())],
            ),
            theme.text,
        ),
        GainSource::Disabled => (words.msg(crate::words::SONG_LEVELLED_OFF), theme.text_dim),
        GainSource::Unmeasured => (words.msg(crate::words::SONG_LEVELLED_NONE), theme.text_dim),
    };
    let levelled_label = words.msg(crate::words::SONG_LEVELLED);
    line(
        canvas,
        cache,
        &levelled_label,
        &ellipsize(&levelled, room(&levelled_label)),
        levelled_color,
        row,
    );
    row += row_h;

    if song.kind != SongMedia::Midi {
        return;
    }

    // **A tally rather than a count**, because the reader's next question after "yes" is always
    // which kind: muting the wrong channel ruins a song and dropping a bank select does not. Never
    // `alert` either way — a correction in force is the machine doing its job, and the question
    // being answered is whether one was applied.
    let fixes = tally(
        words,
        crate::words::SONG_FIX_TAGS,
        &[song.bank_ignored.into(), song.muted.into()],
    )
    .unwrap_or_else(|| words.msg(crate::words::SONG_FIXES_NONE).into_owned());
    let fixes_label = words.msg(crate::words::SONG_FIXES);
    line(
        canvas,
        cache,
        &fixes_label,
        &ellipsize(&fixes, room(&fixes_label)),
        if song.fixed() {
            theme.text
        } else {
            theme.text_dim
        },
        row,
    );
    row += row_h;

    // The convention, then the two marks the file's own habits turned into markup. They are
    // punctuation in every locale, so they are written here rather than in the catalog.
    let flavor = words.msg(match song.flavor {
        Some(KaraokeFlavor::SoftKaraoke) => crate::words::SONG_FLAVOR_SOFT_KARAOKE,
        Some(KaraokeFlavor::LyricEvents) => crate::words::SONG_FLAVOR_LYRIC_EVENTS,
        Some(KaraokeFlavor::NamedTextTrack) => crate::words::SONG_FLAVOR_NAMED_TEXT_TRACK,
        Some(KaraokeFlavor::None) | None => crate::words::SONG_FLAVOR_NONE,
    });
    let mut marks = String::new();
    if let Some(dialect) = song.dialect {
        if dialect.angle_starts_lines {
            marks.push('<');
        }
        if dialect.annotations_are_marked {
            marks.push('%');
        }
        if dialect.harmonica_tabs {
            marks.push('h');
        }
    }
    let lyrics = if marks.is_empty() {
        flavor.into_owned()
    } else {
        format!("{flavor} {marks}")
    };
    let lyrics_label = words.msg(crate::words::SONG_LYRICS);
    line(
        canvas,
        cache,
        &lyrics_label,
        &ellipsize(&lyrics, room(&lyrics_label)),
        theme.text,
        row,
    );
    row += row_h;

    // Drawn only when a track was lost, the call the four counters above make. A repaired note is
    // detail beside that and never a row of its own; see `SongStats::damaged`.
    if !song.damaged() {
        return;
    }
    let damage = tally(
        words,
        crate::words::SONG_DAMAGE_TAGS,
        &[
            song.truncated_tracks,
            song.missing_tracks,
            song.repaired_notes,
        ],
    )
    .unwrap_or_default();
    let damage_label = words.msg(crate::words::SONG_DAMAGE);
    line(
        canvas,
        cache,
        &damage_label,
        &ellipsize(&damage, room(&damage_label)),
        theme.alert,
        row,
    );
}

/// A time as `m:ss`, or `h:mm:ss` from an hour up.
///
/// Whole seconds, truncated: a person reads the number off a screen to find the moment again, and a
/// tenth would change too fast to read.
fn clock(ms: u32) -> String {
    let total = ms / 1_000;
    let (hours, minutes, seconds) = (total / 3_600, (total % 3_600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Joins the non-zero counts of a tagged tally, or `None` when every one of them is zero.
///
/// **The zeroes are dropped rather than drawn**, which is the rule the decoder counters already
/// follow one row at a time: a nought in a list of what is wrong with a file reads as a category
/// that does not apply rather than as one that is clear.
fn tally(words: &km_locale::Catalog, tags: &[&str], counts: &[usize]) -> Option<String> {
    let joined: Vec<String> = tags
        .iter()
        .zip(counts)
        .filter(|(_, count)| **count > 0)
        .map(|(tag, count)| {
            words
                .msg_with(tag, &[("count", (*count).into())])
                .into_owned()
        })
        .collect();
    (!joined.is_empty()).then(|| joined.join("  "))
}

/// Draws the queue over whatever is behind it.
///
/// One row per waiting song, with the singer's name on the right where there is one — at a party that
/// is the column people actually look at, because the question is "am I next", not "what is next".
fn draw_queue_overlay<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    queue: &[QueueEntry],
    layout: Layout,
    words: &km_locale::Catalog,
) {
    let panel_w = layout.w * 0.72;
    let panel_h = layout.h * 0.66;
    let x = (layout.w - panel_w) / 2.0;
    let y = (layout.h - panel_h) / 2.0;

    // A scrim over the whole screen first, then the panel. Two reasons, and the first was found by
    // looking at the preview rather than by reasoning: `theme.panel` is ~78% opaque, so even a second
    // pass (~95%) left the lyric line ghosting through clearly — a big white glyph with an outline
    // shows at 5% transmission far more than the arithmetic suggests. The second reason is that this
    // *should* dim everything: the queue is a question being answered, and dimming the song behind it
    // is what makes it read as a layer rather than as text tangled up with the lyrics.
    canvas.set_blend_mode(BlendMode::Blend);
    // 82%. Checked on a television rather than chosen: at 72% a lyric line crossing the panel was
    // still legible enough to compete with the queue, because lyrics are drawn large and outlined.
    // Outside the panel the song stays visible but clearly behind, which is the intent.
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 210));
    let _ = canvas.fill_rect(FRect::new(0.0, 0.0, layout.w, layout.h));
    panel(canvas, theme, FRect::new(x, y, panel_w, panel_h));
    panel(canvas, theme, FRect::new(x, y, panel_w, panel_h));

    let pad = panel_h * 0.07;
    let text_x = x + pad;
    let right_x = x + panel_w - pad;

    draw_text(
        canvas,
        cache,
        &fonts.text,
        &words.msg(crate::words::QUEUE_HEADING),
        (text_x, y + pad),
        &TextStyle::plain(theme.text, Align::Left),
    );

    // Omitted when empty: "0 songs waiting" next to "Nothing queued" says the same thing twice.
    if !queue.is_empty() {
        let count = words.msg_with(
            crate::words::QUEUE_WAITING,
            &[("count", (queue.len() as i64).into())],
        );
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &count,
            (right_x, y + pad),
            &TextStyle::plain(theme.text_dim, Align::Right),
        );
    }

    let row_h = f32::from(fonts.text.height() as i16) * 1.45;
    let first_row = y + pad + row_h * 1.4;
    let usable = (y + panel_h - pad) - first_row;

    if queue.is_empty() {
        // Said rather than shown as an empty box: pressing Q on an idle machine is exactly how
        // somebody checks whether their song went in, and a blank panel does not answer that.
        draw_text(
            canvas,
            cache,
            &fonts.text,
            &words.msg(crate::words::QUEUE_EMPTY),
            (x + panel_w / 2.0, first_row + usable / 2.0 - row_h / 2.0),
            &TextStyle::plain(theme.text_dim, Align::Center),
        );
        return;
    }

    // One row is reserved for "and N more" whenever the queue does not fit, so the count is never a
    // lie by omission.
    let rows_that_fit = (usable / row_h).floor().max(1.0) as usize;
    let (shown, overflow) = if queue.len() <= rows_that_fit {
        (queue.len(), 0)
    } else {
        let shown = rows_that_fit.saturating_sub(1);
        (shown, queue.len() - shown)
    };

    // Characters that fit in the left column, from the point size — the same estimate the connect
    // panel uses. Half the singer column is left free so the two never collide.
    let text_px = f32::from(theme.text_px(layout.h as u32));
    let left_chars = (((panel_w - pad * 2.0) * 0.62) / (text_px * 0.5)).max(12.0) as usize;

    for (index, entry) in queue.iter().take(shown).enumerate() {
        let row_y = first_row + row_h * index as f32;

        let mut left = format!("{}.  {}  {}", index + 1, entry.number, entry.title);
        if let Some(artist) = &entry.artist {
            left.push_str(" — ");
            left.push_str(artist);
        }
        draw_text(
            canvas,
            cache,
            &fonts.text,
            &ellipsize(&left, left_chars),
            (text_x, row_y),
            &TextStyle::plain(theme.text, Align::Left),
        );

        if let Some(singer) = &entry.singer {
            draw_text(
                canvas,
                cache,
                &fonts.small,
                singer,
                (right_x, row_y),
                &TextStyle::plain(theme.accent, Align::Right),
            );
        }
    }

    if overflow > 0 {
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &words.msg_with(
                crate::words::QUEUE_OVERFLOW,
                &[("count", (overflow as i64).into())],
            ),
            (text_x, first_row + row_h * shown as f32),
            &TextStyle::plain(theme.text_dim, Align::Left),
        );
    }
}

/// Shortens `text` to `max_chars`, ending in an ellipsis when it had to cut.
///
/// Counts characters rather than bytes: song titles in this corpus are full of accented Latin text,
/// and slicing one of those by byte would panic on a character boundary.
fn ellipsize(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    // One character of the budget goes to the ellipsis, so the result is never *longer* than asked.
    let keep = max_chars.saturating_sub(1);
    text.chars().take(keep).collect::<String>() + "…"
}

/// Draws the on-screen touch targets.
///
/// A filled panel behind the whole set, then each key. The panel matters: over a bright wallpaper,
/// keys alone would be unreadable, and the same scrim reasoning that applies to lyrics applies here.
fn draw_keypad<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    keypad: &Keypad,
    layout: Layout,
    words: &km_locale::Catalog,
) {
    let Some((left, top, width, height)) = keypad.bounds() else {
        return;
    };

    // The backing panel is inset slightly beyond the keys so the outermost keys are not flush
    // against its edge.
    let pad = layout.h * 0.012;
    panel(
        canvas,
        theme,
        FRect::new(left - pad, top - pad, width + pad * 2.0, height + pad * 2.0),
    );

    canvas.set_blend_mode(BlendMode::Blend);
    let focused = keypad.focus();
    for (index, key) in keypad.keys().iter().enumerate() {
        let has_focus = Some(index) == focused;
        canvas.set_draw_color(theme.panel);
        let _ = canvas.fill_rect(FRect::new(key.x, key.y, key.w, key.h));

        if has_focus {
            // The D-pad's position. Filled in the accent color rather than merely outlined: across
            // a room, on a television, a one-pixel border is not visible and the whole point of this
            // is telling somebody with a remote which key they are on.
            canvas.set_draw_color(theme.accent);
            let _ = canvas.fill_rect(FRect::new(key.x, key.y, key.w, key.h));
        }

        // A border rather than a rounded rect: SDL has no rounded primitive, and a visible edge is
        // what makes a key look pressable.
        canvas.set_draw_color(if has_focus {
            theme.text
        } else {
            theme.text_dim
        });
        let _ = canvas.draw_rect(FRect::new(key.x, key.y, key.w, key.h));

        let (center_x, center_y) = key.center();
        // Resolved here rather than carried on the key, so `Keypad` stays pure geometry that no
        // test needs a catalog to build.
        let label = match key.label {
            Label::Digit(digit) => Cow::Owned(digit.to_string()),
            Label::Word(id) => words.msg(id),
        };
        // A digit gets the larger face; a word like MELODY has to fit, so it gets the smaller one.
        // Measured on the *resolved* label, because `REPETIR` is a word wherever `REPEAT` was.
        let font = if label.chars().count() > 2 {
            &fonts.small
        } else {
            &fonts.text
        };
        let label_h = f32::from(font.height() as i16);
        let hint_h = f32::from(fonts.small.height() as i16);
        // A hint is drawn only where the key is tall enough to hold two lines of text with air
        // around them. The strip shrinks to fit — a phone in portrait is the case — and a key that
        // is only just a touch target has room for the label and nothing else. Dropping the hint is
        // the right thing to lose: the device that shrinks the strip that far is the one least
        // likely to have a keyboard at all.
        let hint = key
            .hint
            .filter(|_| key.h >= (label_h + hint_h) * HINT_CLEARANCE);
        // `draw_text` takes the top of the text, not its middle, so the label has to be lifted by
        // half a line to sit in the center of its key. Without this every label rides low, which the
        // geometry tests cannot see and a rendered frame shows immediately. With a hint under it the
        // pair is centered instead, so the two together sit where the label alone used to.
        let label_y = match hint {
            Some(_) => center_y - (label_h + hint_h) / 2.0,
            None => center_y - label_h / 2.0,
        };
        // Dark on the focused key, light on the rest — the accent fill is bright, and a white label
        // on it is legible but weak, which defeats a highlight meant to be read from a sofa. The two
        // need different *styles*, though, not just different colors.
        //
        // An unfocused key keeps its outline because it earns it: `theme.panel` is only ~78% opaque,
        // so a bright wallpaper shows through, and a dark halo is what keeps a light label readable
        // over an arbitrary photograph.
        //
        // The focused key must not have one. Its fill is `theme.accent`, which is fully opaque, so
        // there is nothing to outline *against* — and the outline color (`lyric_outline`, near-black)
        // is almost the same as the focused label color (`background`, also near-black), so the halo
        // does not separate the glyph from anything. It just smears eight dark copies around a dark
        // glyph, which is why a highlighted key looked like black text with a black shadow on the
        // television instead of a crisp dark digit on cyan.
        let style = if has_focus {
            TextStyle::plain(theme.background, Align::Center)
        } else {
            TextStyle::outlined(theme.text, theme, Align::Center)
        };
        draw_text(canvas, cache, font, &label, (center_x, label_y), &style);

        // The keyboard key that does the same thing, dimmer than the label so the eye reads the
        // action first and the shortcut second. On the focused key it takes the same dark treatment
        // the label does, for the reason given above: there is nothing to outline against on an
        // opaque accent fill.
        if let Some(hint) = hint {
            let hint_style = if has_focus {
                TextStyle::plain(theme.background, Align::Center)
            } else {
                TextStyle::outlined(theme.text_dim, theme, Align::Center)
            };
            draw_text(
                canvas,
                cache,
                &fonts.small,
                hint,
                (center_x, label_y + label_h),
                &hint_style,
            );
        }
    }
}

/// Wallpaper, crossfade and the darkening scrim.
/// The largest rectangle of a given aspect ratio that fits inside `bounds`, centered in it.
///
/// Letterboxing rather than cropping: a karaoke video puts its words wherever it likes, often at the
/// very bottom, so filling the screen by cutting the edges off is the one option that can lose the
/// thing the singer is reading.
fn fit_inside(bounds: FRect, source_width: u32, source_height: u32) -> FRect {
    if source_width == 0 || source_height == 0 || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return bounds;
    }
    let scale = (bounds.w / source_width as f32).min(bounds.h / source_height as f32);
    let width = source_width as f32 * scale;
    let height = source_height as f32 * scale;
    FRect::new(
        bounds.x + (bounds.w - width) / 2.0,
        bounds.y + (bounds.h - height) / 2.0,
        width,
        height,
    )
}

/// The alpha an incoming image is blitted at.
///
/// Shared by the blit and by [`needs_gradient`], so the decision and the drawing cannot disagree
/// about what visible means. A fade that is not a number saturates to 0 rather than wrapping, which
/// is the safe direction: an unreadable fade shows the gradient.
fn incoming_alpha(fade: f32) -> u8 {
    (fade.clamp(0.0, 1.0) * 255.0) as u8
}

/// Whether the gradient is needed *behind* whatever images there are.
///
/// The old question was "did we draw anything", asked afterwards, and a texture blitted at alpha 0
/// answered yes — which is how the first wallpaper of a boot rose out of a black rectangle instead
/// of out of the gradient the fallback exists to draw. The question that works is whether anything
/// will be **opaque**: an outgoing image always is, and a `current` at full fade is, but a `current`
/// part-way through a fade-in needs something behind it or it is fading up out of the clear color.
///
/// Skipped whenever an opaque image covers it, which is every frame that is not a fade — 48 filled
/// rectangles at up to 125 fps is not a cost to pay for something nobody can see. That is also what
/// keeps a letterboxed video's bars the theme background rather than a gradient, since the picture
/// path always hands over a full-opacity texture.
fn needs_gradient(has_outgoing: bool, has_current: bool, fade: f32) -> bool {
    // The outgoing image is always drawn whole, so where there is one it is the floor.
    !has_outgoing && !(has_current && incoming_alpha(fade) == 255)
}

/// The fallback background: a vertical gradient is a better first impression than a black
/// rectangle, and it still keeps text readable.
fn draw_gradient<T: RenderTarget>(canvas: &mut Canvas<T>, layout: Layout) {
    let bands = 48;
    for band in 0..bands {
        let t = band as f32 / bands as f32;
        let shade = |base: u8, target: u8| {
            (f32::from(base) + (f32::from(target) - f32::from(base)) * t) as u8
        };
        canvas.set_draw_color(Color::RGB(
            shade(0x14, 0x06),
            shade(0x18, 0x08),
            shade(0x28, 0x10),
        ));
        let _ = canvas.fill_rect(FRect::new(
            0.0,
            layout.h * t,
            layout.w,
            layout.h / bands as f32 + 1.0,
        ));
    }
}

fn draw_background<T: RenderTarget>(
    canvas: &mut Canvas<T>,
    theme: &Theme,
    layout: Layout,
    background: Background<'_>,
) {
    canvas.set_draw_color(theme.background);
    canvas.clear();

    let full = FRect::new(0.0, 0.0, layout.w, layout.h);
    let target = match background.shape {
        Some((width, height)) => fit_inside(full, width, height),
        None => full,
    };
    // Underneath rather than instead of: an image fading in has to fade in out of *something*, and
    // the clear color is near-black. Asked before the blits, which consume the fields.
    if needs_gradient(
        background.outgoing.is_some(),
        background.current.is_some(),
        background.fade,
    ) {
        draw_gradient(canvas, layout);
    }

    if let Some(previous) = background.outgoing {
        previous.set_blend_mode(BlendMode::Blend);
        previous.set_alpha_mod(255);
        let _ = canvas.copy(previous, None, target);
    }
    if let Some(current) = background.current {
        current.set_blend_mode(BlendMode::Blend);
        current.set_alpha_mod(incoming_alpha(background.fade));
        let _ = canvas.copy(current, None, target);
    }

    // The scrim is what makes lyrics readable over an arbitrary photograph.
    let dim = background.dim.clamp(0.0, 1.0);
    if dim > 0.0 {
        canvas.set_blend_mode(BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, (dim * 255.0) as u8));
        let _ = canvas.fill_rect(full);
    }
}

/// A translucent panel behind text.
fn panel<T: RenderTarget>(canvas: &mut Canvas<T>, theme: &Theme, rect: FRect) {
    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(theme.panel);
    let _ = canvas.fill_rect(rect);
}

/// Fills a pill — a rectangle with semicircular ends — since SDL has no rounded primitive.
///
/// The same answer [`draw_keypad`] reached and declined: there is no rounded rect to call, so a key
/// is a square with a border. A pill cannot be squared off without becoming a different shape, so it
/// is filled by hand instead — the straight middle in one `fill_rect`, then each cap one row of
/// pixels at a time, which is what [`draw_qr`] already does at a coarser grain.
///
/// At 1080p the caps come to about 42 rows, so this is ~84 rectangles in one `fill_rects` call —
/// nothing beside the texture upload a single string costs.
///
/// **The edge is hard**, as every other fill in this crate is: a scanline circle shows the staircase
/// a midpoint circle does. At the size this draws that reads as a mark rather than as a fault. If it
/// ever needs softening the answer is one pre-rendered alpha texture blitted with `copy`, which
/// changes nothing above this line.
///
/// A `w` equal to `h` is a circle, which is what a one-digit count draws.
fn fill_pill<T: RenderTarget>(canvas: &mut Canvas<T>, color: Color, rect: FRect) {
    let r = rect.h / 2.0;
    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(color);
    // The straight middle. Zero-width when the pill is a circle, which SDL takes as a no-op.
    let _ = canvas.fill_rect(FRect::new(rect.x + r, rect.y, rect.w - rect.h, rect.h));

    let rows = rect.h.round().max(1.0) as i32;
    let mut caps = Vec::with_capacity(rows as usize * 2);
    for row in 0..rows {
        let y = rect.y + row as f32;
        // The row's center against the circle's, as a fraction of the radius.
        let offset = (y + 0.5 - (rect.y + r)) / r;
        let half = (1.0 - offset * offset).max(0.0).sqrt() * r;
        caps.push(FRect::new(rect.x + r - half, y, half, 1.0));
        caps.push(FRect::new(rect.x + rect.w - r, y, half, 1.0));
    }
    let _ = canvas.fill_rects(&caps);
}

/// The queue count's digits and the box they go in.
///
/// Called twice a frame and stateless, which is why it hands back both: [`draw_playing`] asks so it
/// can keep the badge run off the pill, and [`draw`] asks so it can fill it. Threading the rect
/// through `draw_playing`'s signature was the alternative and would have coupled the two to the
/// order they happen to be called in.
fn queue_pill(fonts: &Fonts, layout: Layout, count: usize) -> (String, FRect, f32) {
    let digits = count.to_string();
    // **The measured line height, not `Theme::glyph_box_px`.** The two disagree by more than they
    // look: at 720p the small face reports a 26-pixel line where the glyph box computes 22.8, and a
    // pill sized from the smaller number is one the digits stand outside of. `glyph_box_px` is the
    // right answer for stacking rows, which is what its callers do; this is a box drawn *around*
    // text, and it has to be the size the text actually came out.
    let metrics = measure_line(&fonts.small, &[&digits]);
    (
        digits,
        queue_pill_rect(layout, metrics.width, metrics.height),
        metrics.height,
    )
}

/// Where the queue-count pill sits, given how wide its digits came out.
///
/// **Pure geometry, so the collision tests can ask without opening a font** — the split
/// [`wrap_prompt`] is, for the same rule.
///
/// The top edge is [`TITLE_ROW_TOP`] exactly, and *not* the pill centered on the badge text beside
/// it. The flash band ends `small_px * FLASH_BAND_LINES` down, which is two pixels *below* that row
/// rather than clear of it — 56 against 54 at 1080p, 124 against 120 on a phone in portrait. So the
/// band abuts this row instead of covering it, which is why the caller suppresses the fault line
/// rather than trusting the band to hide it. A pill centered on the digits would stand six pixels
/// proud of the band and be clipped by its lower edge; a top-anchored one takes the same two-pixel
/// overlap the title and the badges have always taken.
///
/// **Both the digits' measurements are the caller's**, which is what keeps this arithmetic testable
/// and is also the only way to get it right: see [`queue_pill`] on why a measured line height and
/// `Theme::glyph_box_px` are not interchangeable here.
///
/// The digits are the **small** face, and that is the row's doing rather than a preference. The row
/// from [`TITLE_ROW_TOP`] to [`SECOND_ROW_TOP`] is `0.06h` — 43 pixels at 720p — and the title face
/// alone is a 36-pixel line there. A pill around it could clear the row or have air, not both, and
/// [`QUEUE_PILL_HEIGHT`] says why the air is not the half to give up.
fn queue_pill_rect(layout: Layout, digits_w: f32, digits_h: f32) -> FRect {
    let height = digits_h * QUEUE_PILL_HEIGHT;
    let width = (digits_w + height * QUEUE_PILL_SIDE_PAD).max(height);
    FRect::new(
        layout.w - layout.margin - width,
        layout.h * TITLE_ROW_TOP,
        width,
        height,
    )
}

/// Draws how many songs are waiting, in the top right corner of either screen.
///
/// **Drawn always, zero included**, which is what separates it from the key/tempo/melody run beside
/// it: those three appear only when they differ from the default, because each is a state that
/// *changed during* a song. This is a standing counter, and a counter that vanished at zero would
/// leave *nothing is queued* and *this machine does not have that* looking identical.
///
/// The fill answers the count instead. `accent` — the badge row's own color, so the corner keeps one
/// meaning — when something is waiting, and the ordinary panel backing when nothing is: at zero the
/// pill reads as furniture and at one it lights up. An accent disc holding a `0` up to an empty room
/// is the noise `catalog-empty` exists to avoid.
///
/// **The digits are not in the catalog**, for the reason `words.rs` gives about `Label::Digit`: `4`
/// is the same mark in every language this renders, and an entry for one would only be a way to
/// break it.
fn draw_queue_pill<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    layout: Layout,
) {
    let (digits, rect, digits_h) = queue_pill(fonts, layout, frame.queue.len());
    // **The disc is always solid and the color is what answers the count.** `theme.panel` was tried
    // for the empty state and is wrong here for a reason worth writing down rather than rediscovering
    // over a wallpaper: it is `#0C0F18` at 78% over a `#0A0C14` background, which is that background.
    // On a panel with rows of text in it that invisibility is the point; as the *whole* of a mark it
    // leaves a `0` floating in the corner with no pill under it, which is not what this draws.
    //
    // So `text_dim` — the same grey the idle screen states its catalog counts in, and for the same
    // reason: a standing fact the eye should be able to find and should not be pulled to. Against
    // `accent`, which is the queue's own color everywhere else on this screen, the two states are a
    // glance apart without either being hard to see.
    //
    // Full strength over a song that brought its own picture, along with the rest of this screen's
    // furniture: a count somebody has to hunt for is a count that says nothing, and this one is a
    // mark in a corner rather than a row across the frame.
    let fill = if frame.queue.is_empty() {
        theme.text_dim
    } else {
        theme.accent
    };
    fill_pill(canvas, fill, rect);
    // Dark on light in both states, because both fills are light. One ink rather than two is what
    // keeps the *color* the only thing that changes between them.
    let ink = theme.background;
    // Centered in the pill rather than sharing the badges' top edge: the fill is what the digits are
    // read against, so they belong in the middle of it.
    //
    // **Plain, not outlined**, which is `draw_keypad`'s judgment about its focused key: an opaque
    // fill leaves an outline nothing to work against, and `lyric_outline` is close enough to
    // `background` that eight offset copies of a dark glyph would smear rather than separate. It
    // also keeps the width arithmetic honest — an outline grows the glyph by `outline` pixels in
    // every direction, which [`queue_pill_rect`] does not account for.
    // Centered on the line's *measured* height, the same number the pill was sized from — so the air
    // above the digits equals the air below whatever the face turns out to be.
    draw_text(
        canvas,
        cache,
        &fonts.small,
        &digits,
        (rect.x + rect.w / 2.0, rect.y + (rect.h - digits_h) / 2.0),
        &TextStyle::plain(ink, Align::Center),
    );
}

fn draw_idle<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    layout: Layout,
) {
    let centered = |color| TextStyle::outlined(color, theme, Align::Center);

    // Above the title, where nothing else ever draws: that something is wrong with the catalog
    // belongs on screen rather than only in a log, because the person who can go and look is the one
    // standing here.
    if let Some(notice) = frame.faults.line(frame.locale) {
        let small_px = f32::from(theme.small_px(layout.h as u32));
        // Half the pill's reserve taken off each side, so the line stays centered on the screen and
        // still clears the corner. Symmetric because a line centered in a box that is not centered
        // reads as a line that has slipped.
        let (_, pill, _) = queue_pill(fonts, layout, frame.queue.len());
        let reserve = (layout.w - layout.margin - pill.x) + QUEUE_PILL_GAP * layout.w;
        let width = layout.w - (layout.margin + reserve) * 2.0;
        let per_line = fit_chars(width, small_px);
        // Cut with an ellipsis rather than stopping dead: a line that ran past two lines read as a
        // message with nothing after the colon, which is exactly how a doubled path in a reason went
        // unnoticed — the evidence was on the line that was thrown away. `wrap_capped` is where that
        // argument lives now, because the keypad message and the connect panel both make it too.
        //
        // It has not been reachable since this line became a count and an area rather than a
        // quoted reason. Kept because a cap is exactly what a longer locale on a narrow screen
        // wants, and removing the net to celebrate not needing it is how it comes back.
        let lines = wrap_capped(&notice, per_line, NOTICE_MAX_LINES);
        for (index, line) in lines.iter().enumerate() {
            draw_text(
                canvas,
                cache,
                &fonts.small,
                line,
                (
                    layout.w / 2.0,
                    layout.h * TITLE_ROW_TOP + small_px * 1.25 * index as f32,
                ),
                &centered(theme.alert),
            );
        }
    }

    // The product's name may not be shortened, so when it will not fit the *size* is what gives
    // way. A television has width to spare; a phone held in portrait does not -- at 1080x2400 the
    // lyric face is 204px and `KaraokeMachine` is fourteen characters, which is half again wider
    // than the screen. Measured rather than estimated, because `size_of` costs no texture and the
    // answer depends on the font that was actually found.
    let title_font = if measure_line(&fonts.lyric, &[TITLE]).width <= layout.w - layout.margin * 2.0
    {
        &fonts.lyric
    } else {
        &fonts.text
    };

    // The name is one word in two colors, set touching -- so it still reads as the single word it
    // is while carrying the two-tone mark the Android TV banner and the website already use. This
    // is `examples/banner.rs`'s treatment brought to the screen the machine actually shows.
    //
    // **`measure_line` measures prefixes**, which is the whole reason it is used here rather than
    // two separate `size_of` calls: `offsets[1]` is where `Machine` begins with the kerning between
    // the halves already in it. Measuring the halves apart and adding the widths would open a gap
    // the width of whatever kern pair sits between `e` and `M`, and a gap is exactly what this must
    // not have.
    //
    // Drawn from the left edge rather than centered, because two centered draws would each center
    // themselves and overlap. The pair is centered by placing the first half half a line's width to
    // the left of the middle.
    let metrics = measure_line(title_font, &TITLE_HALVES);
    let left = layout.w / 2.0 - metrics.width / 2.0;
    let from_left = |color| TextStyle::outlined(color, theme, Align::Left);
    draw_text(
        canvas,
        cache,
        title_font,
        TITLE_HALVES[0],
        (left, layout.h * TITLE_TOP),
        &from_left(theme.lyric_pending),
    );
    draw_text(
        canvas,
        cache,
        title_font,
        TITLE_HALVES[1],
        (left + metrics.offsets[1], layout.h * TITLE_TOP),
        // The amber, and not `accent`, which this was until the icon settled it. Every other place
        // the name is written sets this half in the sung-lyric amber -- the website's `<h1>`, the
        // Android TV banner, and the icon's own `M`. A screen full of the app's blue accents is not
        // a reason for the *wordmark* to be blue on this one surface.
        &from_left(theme.lyric_sung),
    );

    // How much there is to sing, under the title. Dim, because it is a standing fact rather than
    // something to look at: the eye should go to the prompt below it.
    if let Some(catalog) = frame.catalog {
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &catalog.line(frame.locale),
            (layout.w / 2.0, layout.h * CATALOG_SUMMARY_TOP),
            &centered(theme.text_dim),
        );
    }

    // The keypad prompt, whatever has been typed, or an error.
    //
    // **`theme.alert` for every message, which is right only because a message is always a
    // failure** — see `NumberEntry::show_message`. There is no kind to choose by here, so this line
    // is the reason that invariant matters: while a queued song's title also came through it, a
    // success was drawn in the color of a fault and read as one.
    let (prompt, color) = match (frame.number_entry.message(), frame.number_entry.key()) {
        (Some(message), _) => (message.to_owned(), theme.alert),
        (None, typed) if !typed.is_empty() => (typed.to_owned(), theme.accent),
        _ => (
            frame.words().msg(crate::words::IDLE_PROMPT).into_owned(),
            theme.text_dim,
        ),
    };
    let small_px = f32::from(theme.small_px(layout.h as u32));
    let (prompt_font, prompt_lines) =
        fit_prompt(fonts, &prompt, layout.w - layout.margin * 2.0, small_px);
    for (index, line) in prompt_lines.iter().enumerate() {
        draw_text(
            canvas,
            cache,
            prompt_font,
            line,
            (
                layout.w / 2.0,
                layout.h * PROMPT_TOP + small_px * 1.25 * index as f32,
            ),
            &centered(color),
        );
    }

    // The song being dialled, named before anybody presses OK. Never beside a message: an error and
    // a title in the same place would read as one sentence.
    if frame.number_entry.message().is_none()
        && let Some(song) = frame.number_entry.preview()
    {
        let half = idle_preview_half_width(frame, theme, layout);
        let text_px = f32::from(theme.text_px(layout.h as u32));

        draw_text(
            canvas,
            cache,
            &fonts.text,
            &ellipsize(&song.title, fit_chars(half * 2.0, text_px)),
            (layout.w / 2.0, layout.h * PREVIEW_TITLE_TOP),
            &centered(theme.text),
        );
        if let Some(artist) = &song.artist {
            draw_text(
                canvas,
                cache,
                &fonts.small,
                &ellipsize(artist, fit_chars(half * 2.0, small_px)),
                (layout.w / 2.0, layout.h * PREVIEW_ARTIST_TOP),
                &centered(theme.text_dim),
            );
        }
    }

    if let Some(connect) = frame.connect {
        draw_connect_panel(
            canvas,
            cache,
            fonts,
            theme,
            &connect.panel(frame.locale),
            layout,
            PanelSize::Full,
        );
    }

    // Which build this is, in the corner opposite the panel. The quietest thing on the screen and
    // the only one that says the same words all evening, so it is drawn in the dim grey the catalog
    // summary uses and outlined, because a wallpaper is behind it.
    if let Some(version) = frame.version {
        let (x, y) = version_origin(theme, layout.w, layout.h);
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &ellipsize(version, fit_chars(layout.w * VERSION_MAX_WIDTH, small_px)),
            (x, y),
            &TextStyle::outlined(theme.text_dim, theme, Align::Left),
        );
    }
}

/// The most of the width the build number may take, as a fraction.
///
/// `v` and a semantic version is six or seven characters, so this is headroom rather than a budget —
/// and headroom is what makes the clearance a proof instead of a coincidence:
/// `the_build_number_clears_the_number_pad_and_the_connect_panel` holds the *cap* clear of both
/// obstacles, so no string a caller passes can reach either one.
const VERSION_MAX_WIDTH: f32 = 0.2;

/// Air between the build number and the number pad standing above it, as a fraction of the line.
const VERSION_PAD_GAP: f32 = 0.5;

/// Where the build number's text starts: the bottom-left of the safe area.
///
/// **On the safe inset of each axis**, which is the corner a television actually shows. A set
/// overscans by a percentage of each axis on its own, so the inset is two different lengths and a
/// corner placed with one of them puts the shorter side inside the bezel: this line lost its leading
/// `v` to it. The pad above stands on the same two numbers, so the two still read as one object in
/// one corner.
///
/// Named rather than spelled at the draw site because [`version_reserve`] and the test that holds
/// the pad clear of it need the very same point, and a third copy of one expression is how the three
/// come to disagree.
pub(crate) fn version_origin(theme: &Theme, width: f32, height: f32) -> (f32, f32) {
    let line = theme.glyph_box_px(theme.small_size, height as u32);
    (
        crate::keypad::margin_x(width),
        height - crate::keypad::margin_y(height) - line,
    )
}

/// How much room the number pad gives up so the build number can sit under it.
///
/// A line and the air over it, because the pad rests on the same margin the number does — but
/// **taken from [`version_origin`] rather than named**, so a corner that moves takes the pad with
/// it. The two are one arrangement and there is nowhere for them to disagree.
///
/// Public because the pad is laid out by whoever is drawing — `km-app` per frame, the contact sheet
/// per picture — and this is the only thing either of them has to know about the corner.
#[must_use]
pub fn version_reserve(theme: &Theme, width: f32, height: f32) -> f32 {
    let (_, top) = version_origin(theme, width, height);
    let gap = theme.glyph_box_px(theme.small_size, height as u32) * VERSION_PAD_GAP;
    (height - crate::keypad::margin_y(height) - top + gap).max(0.0)
}

/// How many lines of a standing fault line fit above the title without reaching it.
///
/// Two, at [`TITLE_ROW_TOP`] in the small face, and short of the `0.22h` [`TITLE`] sits at.
///
/// **Headroom now rather than a budget.** It was sized to hold a package's name and the reason
/// beside it, back when this line quoted one; [`Faults::line`] is a count and a list of areas, which
/// fits one line on every screen a television is and wraps to two on a phone held in portrait. Kept
/// at two because a longer translation on a narrow screen is exactly what a cap is for, and because
/// three other doc comments point here for the marked-cut argument below.
///
/// **The cut is marked with an ellipsis, not made silently.** A line that simply stopped read as a
/// message with nothing after its colon rather than as a message with more to it, and that is how a
/// package error that printed its path twice went unread: the reason was on the third line, which
/// nothing said had been dropped. That particular failure is now impossible here — this line has no
/// reason in it to lose — and the reasoning is kept because [`DETAIL_MAX_LINES`] and
/// [`PROMPT_MAX_LINES`] both cite it for surfaces where it is still live.
const NOTICE_MAX_LINES: usize = 2;

/// What the machine calls itself on the idle screen.
///
/// The product's name, one word, as `docs/decisions/foundations.md`'s `What the product is called` has it -- not the
/// crate name and not the command. Named rather than spelled inline because the fallback above
/// measures the very string it is about to draw, and the width test measures it too.
const TITLE: &str = "KaraokeMachine";

/// [`TITLE`] split where its two colors meet.
///
/// The name is drawn as two touching pieces so `Karaoke` and `Machine` can differ in color without
/// a space between them — the same treatment as the Android TV banner and the website's `<h1>`.
/// It is still one word, and `the_title_halves_spell_the_title` is what keeps that true: these two
/// strings and [`TITLE`] are the same fourteen characters, and a rename that touched only one of
/// them would otherwise draw a different name from the one the fit test measured.
const TITLE_HALVES: [&str; 2] = ["Karaoke", "Machine"];

/// Where [`TITLE`] sits, as a fraction of screen height.
///
/// Named rather than spelled inline because the catalog summary below it is positioned against it,
/// and a heading that moved without the summary following would put one over the other.
const TITLE_TOP: f32 = 0.22;

/// Where the number prompt sits.
///
/// The other half of the same coupling: the summary has to end above this.
const PROMPT_TOP: f32 = 0.42;

/// Where the catalog summary sits, between the title and the prompt.
///
/// Both neighbors are in the lyric face, which is much taller than this line's, so the gap it sits
/// in is real: [`TITLE`] ends at `TITLE_TOP + lyric_size * GLYPH_BOX` and the prompt begins at
/// `PROMPT_TOP`. `the_catalog_summary_sits_between_the_title_and_the_prompt` computes all three
/// from [`Theme`] rather than trusting this number, so a theme change fails a test instead of
/// quietly overlapping two lines.
const CATALOG_SUMMARY_TOP: f32 = 0.34;

/// Where the dialled song's title sits, as a fraction of screen height.
///
/// Below the digits at [`PROMPT_TOP`] and the glyph box under them, and above the connect panel,
/// whose top edge is at `1 - margin/h - 0.26`. See [`idle_preview_half_width`] for the horizontal
/// half of the same problem.
const PREVIEW_TITLE_TOP: f32 = 0.52;

/// Where the performer sits, under the title.
const PREVIEW_ARTIST_TOP: f32 = 0.575;

/// How many characters of a given point size fit in a width.
///
/// The same estimate the connect panel and the queue overlay make — half the point size per
/// character — rather than measuring, because measuring means building the texture to find out.
fn fit_chars(width: f32, point_size: f32) -> usize {
    (width / (point_size * 0.5)).max(8.0) as usize
}

/// How many lines a wrapped keypad message may take.
///
/// Two, in the small face at [`PROMPT_TOP`], which ends far above both the dialled title at
/// [`PREVIEW_TITLE_TOP`] and the connect panel — and neither of those is ever drawn beside a message
/// anyway. A reason longer than two lines of that face is cut with the same `…` the standing notice
/// uses, on the same argument: the API and the remotes carry the whole of it, and a sentence that
/// stopped dead reads as a message with nothing after its colon.
const PROMPT_MAX_LINES: usize = 2;

/// The keypad line, and the face wide enough to say it in.
///
/// **Until this existed it was the one piece of text in this crate with no width bound of any
/// kind.** The line carries whatever the caller passes — the digits, a dialled title, and every
/// refusal the machine can make — in the lyric face at `0.085h`, centered on its true measured width.
/// `no melody channel was detected for this song` is 44 characters, which at 720p is wider than the
/// screen: the left edge lands at a negative x, SDL clips both ends, and what is left reads as a
/// fault in the machine rather than as a sentence about a song. It was reported from a real run.
///
/// **The size gives way first**, through [`Fonts::fit_lyric`] — the same four rungs a lyric line
/// walks down and the same argument for measuring rather than estimating, that only the font that
/// was actually found knows how wide its glyphs are. The ordinary cases are unmoved by this: `10234`
/// and `no song 9999999` fit the top rung and are drawn exactly as they always were.
///
/// **Then the shape gives way**, which a lyric line may never do because the row beneath one is
/// already spoken for by the next line of the song. Here that row is empty, so a message too long
/// for even the narrowest face wraps into the small face instead of being clipped.
fn fit_prompt<'f>(
    fonts: &'f Fonts,
    prompt: &str,
    available: f32,
    small_px: f32,
) -> (&'f Font<'static>, Vec<String>) {
    let (font, metrics) = fonts.fit_lyric(&[prompt], available);
    if metrics.width <= available {
        return (font, vec![prompt.to_owned()]);
    }
    (
        &fonts.small,
        wrap_prompt(prompt, fit_chars(available, small_px)),
    )
}

/// The wrapping half of [`fit_prompt`], which no font is needed to check.
///
/// Split out for the reason [`crate::text::first_fitting`] is: no test in this crate may open a
/// font, so the part that decides what the words do is kept where it can be tested and the
/// measuring stays the only part that needs SDL.
fn wrap_prompt(prompt: &str, per_line: usize) -> Vec<String> {
    wrap_capped(prompt, per_line, PROMPT_MAX_LINES)
}

/// Wraps into at most `max_lines`, and **marks the cut rather than making it silently**.
///
/// Three places wanted exactly this and two of them had written it out: the standing notice, the
/// keypad message, and now the connect panel's detail. The argument is the same at all three and is
/// spelled at [`NOTICE_MAX_LINES`] — a message that simply stops reads as one with nothing after its
/// colon rather than as one with more to it, and that is how a package error printing its path twice
/// went unread. The whole of it is always in the log, the API and the remotes.
fn wrap_capped(text: &str, per_line: usize, max_lines: usize) -> Vec<String> {
    let mut lines = wrap(text, per_line);
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            *last = ellipsize(&format!("{last}…"), per_line);
        }
    }
    lines
}

/// Half the width the dialled song's name may claim, measured from the center of the screen.
///
/// **This is a real coupling and it is computed rather than assumed.** The number pad is bottom-left
/// and the connect panel bottom-right, and the preview sits between and below the digits — so the
/// first version of anything drawn here will collide with one of them on some screen size. The pad's
/// own `IDLE_MAX_WIDTH_FRACTION` exists because an earlier keypad was drawn straight over the QR
/// code; this asks both obstacles where they actually are instead of repeating a fraction that could
/// drift from them.
///
/// Only an obstacle whose rows overlap the preview's own is allowed to narrow it: the connect panel
/// normally sits well below the title and must not shrink it for nothing.
fn idle_preview_half_width(frame: &Frame<'_>, theme: &Theme, layout: Layout) -> f32 {
    let center = layout.w / 2.0;
    let top = layout.h * PREVIEW_TITLE_TOP;
    // The artist line is the lower of the two, so the band runs to the bottom of its glyph box.
    let bottom = layout.h * PREVIEW_ARTIST_TOP + layout.h * 0.04;

    let mut half = center - layout.margin;

    let mut avoid = |x: f32, y: f32, w: f32, h: f32| {
        if y >= bottom || y + h <= top {
            return;
        }
        // Whichever side of the screen it is on, keep clear of the edge nearest the center.
        let gap = if x + w / 2.0 < center {
            center - (x + w)
        } else {
            x - center
        };
        half = half.min(gap.max(0.0));
    };

    if let Some((x, y, w, h)) = frame.keypad.and_then(Keypad::bounds) {
        avoid(x, y, w, h);
    }
    if frame.connect.is_some() {
        let panel = PanelSize::Full.rect(theme, layout);
        avoid(panel.x, panel.y, panel.w, panel.h);
    }
    half
}

fn draw_playing<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    layout: Layout,
) {
    // **One style for every piece of furniture this screen draws for itself**, because these three
    // closures are what it all goes through and a title that read differently from the `next:` under
    // it would be two kinds of statement rather than one screen.
    //
    // **Full strength over a song that brought its own picture, as over one the machine drew.** What
    // these rows say is read from a sofa for the whole of a song, and a mark thin enough to stand
    // back off a picture is a mark nobody reads. The ring is what carries them over an arbitrary
    // frame; standing back as well leaves the ring carrying them alone.
    let styled = |color, align| TextStyle::outlined(color, theme, align);
    let left = |color| styled(color, Align::Left);
    let right = |color| styled(color, Align::Right);
    let center = |color| styled(color, Align::Center);

    // The face every cut on this screen is measured in, read once for the same reason the three
    // closures above are written once: four places ask `fit_chars` how much fits, and they have to
    // be asking about one face.
    let small_px = f32::from(theme.small_px(layout.h as u32));

    // --- title and artist, top left ---
    if let Some(song) = frame.song {
        let heading = match song.number {
            Some(number) => format!("{number}  {}", song.title),
            None => song.title.clone(),
        };
        draw_text(
            canvas,
            cache,
            &fonts.text,
            &heading,
            (layout.margin, layout.h * TITLE_ROW_TOP),
            &left(theme.text),
        );
        // Artist and language share the second line, because the block already has one and this
        // costs no layout — nothing to position, and nothing that can collide with the badge run at
        // the top right or with the lyrics below.
        //
        // The **name**, not the code: `ja` across a room is noise and "Japanese" is a word. What it
        // buys is a room seeing "Korean" under a romanised title and understanding why the words on
        // screen are not the ones they expected, which is real on a mixed catalog.
        //
        // Deliberately not a badge in the run at the top right. Those three — key, tempo, melody —
        // are states that *change during* a song and are drawn only when they differ from the
        // default. A language does neither, and putting it there would teach that row a second
        // meaning.
        let second_line = match (&song.artist, &song.language) {
            (Some(artist), Some(language)) => Some(format!("{artist}  \u{b7}  {language}")),
            (Some(artist), None) => Some(artist.clone()),
            (None, Some(language)) => Some(language.clone()),
            (None, None) => None,
        };
        if let Some(second_line) = second_line {
            draw_text(
                canvas,
                cache,
                &fonts.small,
                &second_line,
                (layout.margin, layout.h * SECOND_ROW_TOP),
                &left(theme.text_dim),
            );
        }
    }

    // --- next up, the third row of the same column ---
    //
    // **Shortened to the room it has.** A queue label is a title, an em dash and a singer's name;
    // measured against nothing, a long one overhangs the right margin and SDL clips it. The room is
    // the whole row, except while the developer marker is right-aligned across the band from it —
    // see `NEXT_UP_MARKED_WIDTH`, which is everything that marker leaves.
    if let Some(next) = frame.next_up {
        let width = (layout.w - layout.margin * 2.0)
            * if frame.developer_mode.is_some() {
                NEXT_UP_MARKED_WIDTH
            } else {
                1.0
            };
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &ellipsize(
                &frame
                    .words()
                    .msg_with(crate::words::NEXT_UP, &[("title", next.into())]),
                fit_chars(width, small_px),
            ),
            (layout.margin, layout.h * NEXT_UP_TOP),
            &left(theme.text_dim),
        );
    }

    // --- key, tempo and melody indicators, top right ---
    //
    // None of the three exists for a video song, and a badge reading "key +2" over a video whose key
    // has not moved would be a plain lie — the settings behind these are the ones the *next* MIDI
    // song will use.
    let words = frame.words();
    let mut badges: Vec<String> = Vec::new();
    if frame.transpose != 0 && !frame.picture {
        // The sign is formatted here and passed as text, not as a number: `+2` is what a singer
        // reads, and Fluent would render an integer argument as `2`.
        badges.push(
            words
                .msg_with(
                    crate::words::BADGE_KEY,
                    &[("semitones", format!("{:+}", frame.transpose).into())],
                )
                .into_owned(),
        );
    }
    if (frame.tempo_ratio - 1.0).abs() > 0.001 && !frame.picture {
        badges.push(
            words
                .msg_with(
                    crate::words::BADGE_TEMPO,
                    &[("ratio", format!("{:.2}", frame.tempo_ratio).into())],
                )
                .into_owned(),
        );
    }
    // Shown only when the song actually has a melody channel, so the display never advertises a
    // control that would do nothing.
    if frame.melody == Some(true) {
        badges.push(words.msg(crate::words::BADGE_MELODY).into_owned());
    }
    if !badges.is_empty() {
        // Anchored off the queue pill rather than off the margin: the corner is the pill's now, and
        // this run ends where the pill begins. Asked for rather than passed in, because `draw` fills
        // the pill *after* this function runs and threading the rect through would tie the two to
        // that order — see [`queue_pill`].
        let (_, pill, _) = queue_pill(fonts, layout, frame.queue.len());
        let right_edge = pill.x - QUEUE_PILL_GAP * layout.w;
        // **And cut to fit, which this run never was.** It was the last right-aligned text in the
        // crate drawn with no `ellipsize` and no `fit_chars` — the third instance of the defect
        // `A lyric line that will not fit` and the keypad line each found before it, and it survived
        // because the contact sheet's badge fixtures are short on a 1920-wide frame. It was true
        // before the pill and only just: `key -12   tempo 0.50x   melody` is 30 characters, about
        // 930px in a 62px face, against the 972px between the margins of a phone in portrait. Taking
        // the corner away for the pill is what makes it reachable, so the cap lands with it.
        let run = ellipsize(
            &badges.join("   "),
            fit_chars(right_edge - layout.margin, small_px),
        );
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &run,
            (right_edge, layout.h * TITLE_ROW_TOP),
            &right(theme.accent),
        );
    }

    // --- the lyrics ---
    //
    // Skipped entirely for a video song, the "(no lyrics in this file)" fallback included. That
    // message is a true and useful thing to say about a MIDI file and a misleading one over a video,
    // whose words are on screen already — in its own picture, where the machine did not put them.
    if frame.picture {
        // Nothing to draw: the video is the words.
    } else if let Some(timeline) = frame.timeline {
        // From the theme, because `tools/cmd/assets/km-wallpaper-pack` measures legibility inside the band these
        // two numbers define. See `Theme::lyric_band`.
        let row_height = layout.h * theme.lyric_row_height;
        let first_row_y = layout.h * theme.lyric_row_top;

        for row in 0..ROWS {
            let Some(visible) = frame.lyrics.line_in_row(row) else {
                continue;
            };
            let Some(line) = timeline.lines.get(visible.index) else {
                continue;
            };
            let y = first_row_y + row as f32 * row_height;
            let text = line.text();
            if text.is_empty() {
                continue;
            }

            // **Each row is fitted on its own**, so a long line never shrinks the short one above
            // or below it. See `Fonts::fit_lyric` for why the size gives way rather than the words.
            let available = layout.w - layout.margin * 2.0;

            if visible.is_current {
                let syllables: Vec<&str> = line.syllables.iter().map(|s| s.text.as_str()).collect();
                // The same face has to measure and draw, or `wipe_x` -- an offset into these
                // metrics -- would land somewhere in the middle of the wrong glyph.
                let (font, metrics) = fonts.fit_lyric(&syllables, available);
                let wiped = WipedLine {
                    text: &text,
                    metrics: &metrics,
                    wipe_x: metrics.wipe_x(visible.syllable, visible.syllable_progress),
                };
                draw_wiped_line(
                    canvas,
                    cache,
                    font,
                    &wiped,
                    (layout.w / 2.0, y),
                    &WipeStyle {
                        pending: center(theme.lyric_pending),
                        sung: theme.lyric_sung,
                    },
                );
            } else {
                // The upcoming line is dimmer, so the eye knows which one is live. It is measured
                // for the first time here: it used to go straight to `draw_text`, which asks the
                // rendered surface how wide it came out and so could never have noticed.
                let (font, _) = fonts.fit_lyric(&[text.as_str()], available);
                draw_text(
                    canvas,
                    cache,
                    font,
                    &text,
                    (layout.w / 2.0, y),
                    &center(theme.lyric_upcoming),
                );
            }
        }
    } else if frame.song.is_some() {
        draw_text(
            canvas,
            cache,
            &fonts.text,
            &frame.words().msg(crate::words::NO_LYRICS),
            (layout.w / 2.0, layout.h * NO_LYRICS_TOP),
            &center(theme.text_dim),
        );
    }

    // --- the standing connect panel, and the room it leaves ---
    //
    // Whether the address stands in the bottom-right corner for the length of this song. Decided
    // here rather than where it is drawn, because the two lines below have to know: they are the
    // only things on this screen that would otherwise run underneath it.
    //
    // **A demo song is the one time the television has to say how to reach the remote.** The whole
    // point of demo mode is that a room can hear what the box holds without first working out how to
    // drive it — and the one thing that would let them drive it was on the idle screen they are not
    // looking at and behind a key nobody in the room knows about.
    //
    // Four things take it away, and each is already the rule for something else here:
    //
    // * the `I` overlay, which is bigger and carries the detail lines, and which somebody asked for;
    // * the transport strip, which the demo line yields to for the same reason;
    // * a number being dialled, whose box is `0.5w` centered and so reaches exactly this panel's left
    //   edge — the overlay overlaps it and gets away with it by being gone in eight seconds;
    // * a song somebody queued, because this is a demo-song panel and not a playing-screen one.
    //
    // A fifth takes it away too, and it is `standing_panel_fits`: the demo line outranks the panel,
    // so a screen with no room for both keeps the sentence. On the four landscape screens in
    // `SCREENS` there is room to spare; the portrait phone is the one that gives the panel up, and a
    // window that shape is not one anybody watches a demo song on from a sofa.
    let standing = frame.demo.is_some()
        && frame.connect.is_some_and(ConnectInfo::has_url)
        && !frame.show_connect_overlay
        && frame.keypad.is_none()
        && !frame.number_entry.is_active()
        && standing_panel_fits(theme, layout);

    // How much of the bottom row belongs to the left column. The demo line is drawn from the margin,
    // and the standing panel is the only thing that ever stands to its right — so this is the whole
    // width whenever the panel is not up.
    let bottom_left_width = if standing {
        standing_bottom_left_width(theme, layout)
    } else {
        layout.w - layout.margin * 2.0
    };

    // --- demo mode ---
    //
    // Its own row, directly above `next:` and aligned with it, so the two read as a pair: what is
    // coming, and how to get to it. Hidden while the transport strip is up, for the reason `next:`
    // is — both live along the bottom and the strip is centered over them.
    //
    // Not timed out, unlike a flash: the whole reason it is drawn is that somebody walking into the
    // room mid-song needs to know why the machine is singing to itself, and a label that had faded
    // would be gone from exactly the moment it is wanted. That is `soundfont_label`'s argument.
    if let Some(demo) = frame.demo.filter(|_| frame.keypad.is_none()) {
        let width = bottom_left_width * DEMO_LABEL_MAX_WIDTH;
        let text = ellipsize(demo, fit_chars(width, small_px));
        // **`theme.text` and deliberately not `left(theme.text_dim)`** — which is otherwise exactly
        // this style. This is the one line that has to reach somebody who has just walked into the
        // room, so it is stated in the color the title is rather than in the dimmer grey the rows
        // under it share. The standing connect panel beside it is stated at full weight for the same
        // reason.
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &text,
            (layout.margin, layout.h * DEMO_LABEL_TOP),
            &TextStyle::outlined(theme.text_dim, theme, Align::Left),
        );
    }

    draw_progress(canvas, theme, layout, frame);

    // --- number being typed ---
    if frame.number_entry.is_active() {
        let text = frame
            .number_entry
            .message()
            .map(str::to_owned)
            .unwrap_or_else(|| frame.number_entry.key().to_owned());
        let color = if frame.number_entry.message().is_some() {
            theme.alert
        } else {
            theme.accent
        };
        // The box grows only when there is more than digits to put in it, so a machine with nothing
        // dialled draws exactly what it always did over the lyrics. A dialled name and a message are
        // the two things that need the room, and they cannot both be here: `show_message` clears the
        // digits, and a preview only resolves for digits that are still on screen.
        let preview = frame.number_entry.preview();
        let message = frame.number_entry.message().is_some();
        let (box_w, box_h) = if preview.is_some() || message {
            (layout.w * 0.5, layout.h * 0.18)
        } else {
            (layout.w * 0.3, layout.h * 0.12)
        };
        panel(
            canvas,
            theme,
            FRect::new((layout.w - box_w) / 2.0, layout.h * 0.72, box_w, box_h),
        );
        let text_px = f32::from(theme.text_px(layout.h as u32));
        // Wrapped into the box, the way the idle screen's copy of this line is wrapped into the
        // screen. The digits never need it — six of them at most — and a message always might: the
        // same 44-character refusal that overhung the idle screen ran out of both sides of this box,
        // and cutting it at the width left `no melody channel was det…`, which is a message about
        // nothing.
        let lines = if message {
            wrap_prompt(&text, fit_chars(box_w * 0.92, text_px))
        } else {
            vec![text]
        };
        for (index, line) in lines.iter().enumerate() {
            draw_text(
                canvas,
                cache,
                &fonts.text,
                line,
                (
                    layout.w / 2.0,
                    layout.h * 0.75 + text_px * 1.25 * index as f32,
                ),
                &TextStyle::plain(color, Align::Center),
            );
        }
        if let Some(song) = preview {
            let line = match &song.artist {
                Some(artist) => format!("{}  —  {artist}", song.title),
                None => song.title.clone(),
            };
            draw_text(
                canvas,
                cache,
                &fonts.small,
                &ellipsize(&line, fit_chars(box_w * 0.92, small_px)),
                (layout.w / 2.0, layout.h * 0.815),
                &TextStyle::plain(theme.text_dim, Align::Center),
            );
        }
    }

    if frame.show_connect_overlay
        && let Some(connect) = frame.connect
    {
        draw_connect_panel(
            canvas,
            cache,
            fonts,
            theme,
            &connect.panel(frame.locale),
            layout,
            PanelSize::Overlay,
        );
    } else if standing && let Some(connect) = frame.connect {
        // The `else` is one of the four conditions in `standing` written twice, and it is written
        // twice on purpose: two panels in one corner is the failure this arrangement exists to
        // prevent, and a reader should not have to hold four booleans in their head to be sure of it.
        draw_connect_panel(
            canvas,
            cache,
            fonts,
            theme,
            &connect.panel(frame.locale),
            layout,
            PanelSize::Standing,
        );
    }
}

fn draw_progress<T: RenderTarget>(
    canvas: &mut Canvas<T>,
    theme: &Theme,
    layout: Layout,
    frame: &Frame<'_>,
) {
    if !frame.show_position {
        return;
    }
    let bar = timeline_rect(layout);

    // **The track is translucent and the bar over it is not**, which is a statement about the two
    // halves of one mark rather than about what is behind them: a track is where the bar will reach
    // and the bar is where it has got to, so the one that carries the answer is the solid one. A
    // line four pixels tall has no mass to give up in either case, and it only visits.
    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0xFF, 0xFF, 0xFF, 0x30));
    let _ = canvas.fill_rect(bar);

    if frame.duration_ms > 0 {
        let progress = (frame.position_ms as f32 / frame.duration_ms as f32).clamp(0.0, 1.0);
        canvas.set_draw_color(theme.accent);
        let _ = canvas.fill_rect(FRect::new(bar.x, bar.y, bar.w * progress, bar.h));
    }
}

/// The connect panel: the URL to type, and a QR code to scan instead.
/// The three sizes the connect panel is drawn at, and the only three.
///
/// It was a `bool` while there were two. The third is what made a name necessary: `large: false` was
/// already the less readable of the two call sites, and `large: false` meaning *the middle one* would
/// have been worse than either.
///
/// **All three are anchored to the bottom-right corner**, which is one expression rather than three:
/// the panel is the thing this screen puts in that corner, and a size that moved would be a second
/// rule about where to look for the address.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PanelSize {
    /// The idle screen. Headline, the URL in the text face, the QR and [`DETAIL_MAX_LINES`] lines of
    /// detail — and a box [`Card`] sizes to hold all of them.
    Full,
    /// `I`, and the first few seconds after startup. The same contents, smaller, over a song.
    Overlay,
    /// A demo song, for as long as one is on the deck. The QR and the URL beneath it, and nothing
    /// else — see [`draw_connect_panel`].
    Standing,
}

impl PanelSize {
    /// The box the panel occupies, bottom-right anchored.
    ///
    /// **Read by more than the drawing.** `idle_preview_half_width` asks what the panel takes so a
    /// dialled title can keep clear of it, and it used to answer by spelling `0.55` and `0.26` a
    /// second time — two numbers a change to the panel could not reach. `draw_playing` asks the same
    /// question about the two lines along the bottom.
    fn rect(self, theme: &Theme, layout: Layout) -> FRect {
        let (w, h) = match self {
            // **The width is the screen's and the height is the contents'**, which is the same split
            // `Standing` makes below. The width is what decides how much of a URL fits on a line and
            // there is no measuring it out of a font; the height is the rows added up, and a
            // fraction of the screen guessing at that is what drew the detail under the panel's own
            // bottom edge. The old fractions stay as a floor, so a box never comes out smaller than
            // the one that has been on the screen all along.
            Self::Full => {
                let it = Card::of(theme, layout, self);
                (it.width, it.height().max(layout.h * 0.26))
            }
            Self::Overlay => {
                let it = Card::of(theme, layout, self);
                (it.width, it.height().max(layout.h * 0.18))
            }
            // **Sized from the QR outwards, and from *height*, where the other two are fractions of
            // the width.** That is not an inconsistency: those two are text panels whose useful
            // dimension is how much of a URL fits on a line, and this one is a square with a caption.
            // A fraction of the width makes a square panel a different shape on every screen — at
            // 0.20w it is 384 wide on a 16:9 television and 216 on a portrait phone, where the face
            // beneath it is more than twice as tall.
            Self::Standing => {
                let it = Standing::of(theme, layout);
                (
                    it.width + it.pad * 2.0,
                    it.qr + it.gap + it.text_h + it.pad * 2.0,
                )
            }
        };
        FRect::new(
            layout.w - layout.margin - w,
            layout.h - layout.margin - h,
            w,
            h,
        )
    }
}

/// The idle panel's and the `I` overlay's rows, in pixels.
///
/// **The type exists so that the box and its contents are sized by one rule**, and a box does not
/// have to be much smaller than what goes in it for the last line to be drawn through its bottom
/// edge. Rows as fractions of the box's height — `pad + panel_h * (0.55 + 0.15 * index)` — over a
/// padding taken from the box's *width*, with the box a fraction of the screen that nothing derives
/// from the rows, overhang by a couple of pixels on the second detail line and by tens on the third:
/// a sentence with its descenders shaved off, and the line that says how to fix the problem missing
/// entirely.
///
/// So the rows come first here and [`PanelSize::rect`] adds them up, exactly as [`Standing`] does
/// for the third size. Everything is measured off [`Theme`] — the same `glyph_box` the idle
/// screen proves its four stacked lines with, and the same `small_px * 1.25` step the notice and the
/// keypad message wrap by — so a change to a font size moves the box rather than overflowing it.
struct Card {
    /// Air inside the panel's edge.
    ///
    /// Half a line of the small face. It was `panel_w * 0.05`, which is a *width* deciding how much
    /// room there is above and below the text: at 1080p that is 53 pixels of a 281-pixel box, more
    /// than a third of the panel's height spent on air at top and bottom while the rows ran out of
    /// the bottom of it.
    pad: f32,
    /// Between the headline, the address and the detail.
    gap: f32,
    /// The headline's glyph box, which is also a detail line's — both are the small face.
    line_h: f32,
    /// The address's glyph box: the text face on the idle screen, the small one in the overlay.
    url_h: f32,
    /// Top to top between detail lines.
    step: f32,
    /// The QR code's side, when there is an address to encode.
    qr: f32,
    /// The panel's width, which is the screen's rather than the rows'.
    width: f32,
    /// The rows added up: headline, address and [`DETAIL_MAX_LINES`] lines of detail.
    ///
    /// The key hint is drawn out of those same rows rather than after them — see
    /// [`HINT_MAX_LINES`] — so it adds nothing here.
    column: f32,
}

impl Card {
    fn of(theme: &Theme, layout: Layout, size: PanelSize) -> Self {
        let full = size == PanelSize::Full;
        let height_px = layout.h as u32;
        let small_px = f32::from(theme.small_px(height_px));
        // Pixels rather than `layout.h * glyph_box(..)`, because the face is opened at a rounded,
        // floored pixel size and this box is the sum of its rows — see `Theme::glyph_box_px`.
        let line_h = theme.glyph_box_px(theme.small_size, height_px);
        // The address is the one line the idle screen prints large, because it is the thing somebody
        // across the room is trying to read. The overlay says it in the small face like everything
        // else it carries.
        let url_h = if full {
            theme.glyph_box_px(theme.text_size, height_px)
        } else {
            line_h
        };
        let step = small_px * 1.25;
        let gap = small_px * 0.5;
        let width = layout.w * if full { FULL_WIDTH } else { OVERLAY_WIDTH };
        // **Capped against the panel's width as well as given a height**, which is the one case a
        // square in a box sized by two different rules cannot survive: at 1080x2400 the idle panel
        // is 594 pixels wide and `FULL_QR_SIDE` of that height is 384, which left *minus sixty*
        // pixels for the text and drew the detail straight across the code. The old leftover
        // arithmetic was worse there, not better.
        let qr = (layout.h * if full { FULL_QR_SIDE } else { OVERLAY_QR_SIDE })
            .min((width - gap * 2.0) * QR_MAX_WIDTH);
        Self {
            pad: gap,
            gap,
            line_h,
            url_h,
            step,
            qr,
            width,
            // The last detail line's top is `step * (DETAIL_MAX_LINES - 1)` below the first, and it
            // occupies a glyph box from there — so the column ends at the bottom of the last line
            // rather than at its top, which is the pixel the old arithmetic never accounted for.
            column: line_h + gap + url_h + gap + step * (DETAIL_MAX_LINES - 1) as f32 + line_h,
        }
    }

    /// How tall this panel's rows come out for what it has actually been given to say.
    ///
    /// A state with no address does not reserve a line for one, and one with a two-line sentence
    /// does not reserve a third: a panel is sized for the worst case because [`PanelSize::rect`]
    /// answers callers with no panel to look at, but what is *drawn* is what there is.
    /// The detail and the hint are **one stack, not two**, so they count as one block: the hint
    /// picks up at the step after the last detail line and is drawn in the same face.
    fn content(&self, has_url: bool, detail_lines: usize, hint_lines: usize) -> f32 {
        let mut h = self.line_h;
        if has_url {
            h += self.gap + self.url_h;
        }
        let rows = detail_lines + hint_lines;
        if rows > 0 {
            h += self.gap + self.step * (rows - 1) as f32 + self.line_h;
        }
        h
    }

    /// Where the headline's top edge sits, measured from the panel's top edge.
    ///
    /// **Centred in whatever room the box has**, which is the difference between a panel and a
    /// panel with a hole in it. The box is sized for the worst case and most states are not it — a
    /// reachable machine says three rows where a failed bind says two — so rows pinned to the top
    /// padding leave the slack in one lump along the bottom edge. The old fractions spread it by
    /// accident and by drawing outside the box; this spreads it on purpose.
    fn top(&self, box_h: f32, has_url: bool, detail_lines: usize, hint_lines: usize) -> f32 {
        ((box_h - self.content(has_url, detail_lines, hint_lines)) / 2.0).max(self.pad)
    }

    /// Where the address sits, given the headline's top.
    fn url_top(&self, top: f32) -> f32 {
        top + self.line_h + self.gap
    }

    /// Where a detail line's top edge sits, given the headline's top.
    ///
    /// The one place the stack is spelled out, and both the drawing and the test that pins the rows
    /// inside the box read it — a test that re-derived this would be testing its own copy of it.
    fn detail_top(&self, top: f32, has_url: bool, index: usize) -> f32 {
        let first = if has_url {
            self.url_top(top) + self.url_h + self.gap
        } else {
            top + self.line_h + self.gap
        };
        first + self.step * index as f32
    }

    /// Where a hint line's top edge sits: the next step of the same stack the detail is in.
    ///
    /// **One row-counter rather than two**, which is what makes the hint free: it comes out of
    /// [`DETAIL_MAX_LINES`] instead of adding to it, so the box does not have to grow — and there
    /// is nowhere for it to grow to. See [`HINT_MAX_LINES`].
    fn hint_top(&self, top: f32, has_url: bool, detail_lines: usize, index: usize) -> f32 {
        self.detail_top(top, has_url, detail_lines + index)
    }

    /// Where the code's top edge sits: centred too, for the same reason.
    fn qr_top(&self, box_h: f32) -> f32 {
        ((box_h - self.qr) / 2.0).max(self.pad)
    }

    /// The text column's width — **the whole panel where there is no code**.
    ///
    /// The three states that carry a sentence at all are exactly the three with no address to
    /// encode: loopback-only, no network, and a bind that failed. Reserving a square for the code
    /// none of them draws wraps the sentence into two thirds of the panel for nothing, which is
    /// more lines in the one place lines are in short supply.
    fn text_w(&self, with_qr: bool) -> f32 {
        if with_qr {
            self.width - self.pad * 2.0 - self.qr - self.gap
        } else {
            self.width - self.pad * 2.0
        }
    }

    /// The box's height: the taller of the text column and the code, plus air.
    ///
    /// The code is a square and the text is a stack, and which of them is the taller depends on the
    /// size — so the panel is as tall as whichever needs the room, rather than as tall as one of
    /// them and hoping about the other.
    fn height(&self) -> f32 {
        self.pad * 2.0 + self.column.max(self.qr)
    }
}

/// The standing panel's parts, in pixels.
///
/// One type because [`PanelSize::rect`] and [`draw_connect_panel`] need all of them and a panel whose
/// box and whose contents disagreed would be a code with its quiet zone cut off.
///
/// **The two axes are decided by different things, and that is the whole shape.** The height is the
/// code's, because a QR is a square and what makes one scannable from a sofa is how big it is. The
/// width is the *caption's*, because a code nobody's camera will focus on is only useful if the
/// address under it can be read and typed — and the first version of this panel, sized on both axes
/// from the code, printed `http://19…`, which is worse than printing nothing.
struct Standing {
    /// Air inside the panel's edge.
    pad: f32,
    /// The QR code's side.
    qr: f32,
    /// Between the code and its caption.
    gap: f32,
    /// The caption's glyph box.
    text_h: f32,
    /// Inside the padding: the wider of the code and the longest address that can appear under it.
    width: f32,
}

impl Standing {
    fn of(theme: &Theme, layout: Layout) -> Self {
        let pad = layout.h * STANDING_PAD;
        let qr = layout.h * STANDING_QR_SIDE;
        let small_px = f32::from(theme.small_px(layout.h as u32));
        Self {
            pad,
            qr,
            gap: pad * 0.5,
            text_h: layout.h * theme.glyph_box(theme.small_size),
            // The same half-the-point-size estimate `fit_chars` measures with, so the caption cannot
            // be ellipsized by a panel that was sized to hold it.
            width: qr.max(STANDING_URL_CHARS as f32 * small_px * 0.5),
        }
    }
}

/// The width the playing screen's two bottom-left lines have while the standing panel is up.
///
/// The margin twice over: one for the left edge, and one again as the gap between the longest the
/// line may come out and the panel's left edge.
fn standing_bottom_left_width(theme: &Theme, layout: Layout) -> f32 {
    // **`false` costs nothing here, because only the left edge is read.** The key hint wraps into
    // the width the address already bought and moves the card's height alone — see `Standing::of` —
    // so the answer is the same either way, and `the_key_hint_does_not_move_the_standing_cards_edge`
    // is what keeps that true.
    (PanelSize::Standing.rect(theme, layout).x - layout.margin * 2.0).max(0.0)
}

/// Whether the standing panel can be drawn without cutting the demo line it stands beside.
///
/// **The demo line outranks the panel**, which is the one thing about this feature that is a rule
/// rather than a layout. `The demo line, and why it does not share a row` settled that the
/// instruction may not be shortened — a machine saying there is something to do without saying what
/// is worse than one saying nothing — and the panel is only ever up while that line is.
///
/// A function rather than an expression inside [`draw_playing`] so that the tests can ask exactly
/// what the drawing asks. A test that re-derived this would be testing its own copy of it.
fn standing_panel_fits(theme: &Theme, layout: Layout) -> bool {
    let width = standing_bottom_left_width(theme, layout) * DEMO_LABEL_MAX_WIDTH;
    fit_chars(width, f32::from(theme.small_px(layout.h as u32))) >= DEMO_LABEL_MIN_CHARS
}

/// Draws the address of the remote, with a QR code of it.
///
/// [`PanelSize::Standing`] is the odd one and is a different panel rather than a smaller one: **the
/// QR and the URL under it, no headline and no detail lines.** It is on screen for a whole demo song
/// rather than for eight seconds, so what it has to do is be scannable from a sofa and otherwise stay
/// out of the way — and dropping the text column is what pays for that. In a panel two thirds the
/// area of [`PanelSize::Overlay`] the code comes out *larger* than the overlay's, which is the whole
/// argument for the shape.
///
/// It also draws **nothing at all** where [`ConnectPanel::url`] is `None` — loopback-only, no usable
/// interface, a bind that failed. Those are exactly the cases whose honest one-line explanation is
/// the thing this size has no room for, and a card reading *No network* for the length of every demo
/// song is noise. The idle screen and the overlay both still say it.
///
/// **[`ConnectPanel::hint`] is the same answer for the same reason.** The other two sizes carry the
/// key hint out of rows they already had; this one has no rows to share, and nowhere to put another
/// — see [`HINT_MAX_LINES`] for the nine pixels that settle it. Somebody sitting at the machine
/// during a demo song presses `I` and gets the overlay, which says it.
fn draw_connect_panel<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    fonts: &Fonts,
    theme: &Theme,
    info: &ConnectPanel,
    layout: Layout,
    size: PanelSize,
) {
    // The box is reserved for whatever this build can say, and `hint` being `Some` is that build
    // constant arriving as a message — `ConnectInfo::browser_key` is what put it there.
    let box_ = size.rect(theme, layout);
    let (x, y, panel_w, panel_h) = (box_.x, box_.y, box_.w, box_.h);

    if size == PanelSize::Standing {
        let Some(url) = &info.url else {
            return;
        };
        let it = Standing::of(theme, layout);
        let small_px = f32::from(theme.small_px(layout.h as u32));
        panel(canvas, theme, FRect::new(x, y, panel_w, panel_h));
        // Centered rather than left-aligned, because the panel is as wide as the longest address
        // there could be and this one is usually shorter — a code pinned to the left edge under a
        // centered caption would read as a mistake.
        draw_qr(canvas, url, x + (panel_w - it.qr) / 2.0, y + it.pad, it.qr);
        // `ellipsize` cannot fire on any address `km_api::connect` produces — the panel is sized to
        // `STANDING_URL_CHARS`. It is here for the same reason the cap on the demo line is: so that
        // nothing can run out of the panel unnoticed if that ever stops being true.
        draw_text(
            canvas,
            cache,
            &fonts.small,
            &ellipsize(url, fit_chars(panel_w - it.pad * 2.0, small_px)),
            (x + panel_w / 2.0, y + it.pad + it.qr + it.gap),
            &TextStyle::plain(theme.text, Align::Center),
        );
        return;
    }

    let large = size == PanelSize::Full;
    let it = Card::of(theme, layout, size);
    let small_px = f32::from(theme.small_px(layout.h as u32));
    panel(canvas, theme, FRect::new(x, y, panel_w, panel_h));

    let has_url = info.url.is_some();
    let text_x = x + it.pad;
    let per_line = fit_chars(it.text_w(has_url), small_px);
    // Wrapped before anything is placed, because how many lines the sentence comes to is what says
    // where the block starts: the box holds three and most states say fewer.
    let detail = info
        .detail
        .as_deref()
        .map(|detail| wrap_capped(detail, per_line, DETAIL_MAX_LINES));
    let detail_lines = detail.as_ref().map_or(0, Vec::len);
    // **The detail is wrapped first and the hint takes what is left**, out of the same
    // `DETAIL_MAX_LINES` rows — the box cannot grow, and a panel that said which key opens the
    // remote by dropping half of what the machine had to report would have its priorities backwards.
    //
    // **Whole or not at all, which is the one place this departs from every other capped block on
    // this screen.** Those cut with an `…` because the reader can go and find the rest; this is the
    // only instruction on the panel, and `F11 opens the remo…` spends a row to name no key. So it is
    // wrapped uncapped and drawn only if it fits.
    let hint = info.hint.as_deref().and_then(|hint| {
        let room = DETAIL_MAX_LINES
            .saturating_sub(detail_lines)
            .min(HINT_MAX_LINES);
        let lines = wrap(hint, per_line);
        (lines.len() <= room).then_some(lines)
    });
    let top = it.top(
        panel_h,
        has_url,
        detail_lines,
        hint.as_ref().map_or(0, Vec::len),
    );

    draw_text(
        canvas,
        cache,
        &fonts.small,
        // Shortened to its column like every other line on this screen. Nothing had ever compared
        // this one to the room it has: `Remote control unavailable` is the longest the machine says
        // itself, and it fits, but a translation is free to be longer and would have run out of the
        // panel's side with SDL clipping it.
        &ellipsize(&info.headline, per_line),
        (text_x, y + top),
        &TextStyle::plain(
            if info.is_problem {
                theme.alert
            } else {
                theme.text_dim
            },
            Align::Left,
        ),
    );

    if let Some(url) = &info.url {
        draw_text(
            canvas,
            cache,
            if large { &fonts.text } else { &fonts.small },
            url,
            (text_x, y + it.url_top(top)),
            &TextStyle::plain(theme.text, Align::Left),
        );
        draw_qr(
            canvas,
            url,
            x + panel_w - it.pad - it.qr,
            y + it.qr_top(panel_h),
            it.qr,
        );
    }

    if let Some(lines) = &detail {
        // A `ServerFailed` detail is the operating system's own words about a socket, and there is
        // no bound on how many of them there are — so the cut is marked, like every other capped
        // block on this screen.
        for (index, chunk) in lines.iter().enumerate() {
            draw_text(
                canvas,
                cache,
                &fonts.small,
                chunk,
                (text_x, y + it.detail_top(top, has_url, index)),
                &TextStyle::plain(theme.text_dim, Align::Left),
            );
        }
    }

    // Last, under everything else on the panel: the address and its count are facts about *this
    // machine*, and this is an instruction about the keyboard in front of the reader. A row that
    // came between them would read as a third thing about the address.
    if let Some(lines) = &hint {
        for (index, chunk) in lines.iter().enumerate() {
            draw_text(
                canvas,
                cache,
                &fonts.small,
                chunk,
                (text_x, y + it.hint_top(top, has_url, detail_lines, index)),
                &TextStyle::plain(theme.text_dim, Align::Left),
            );
        }
    }
}

/// Draws a QR code as filled squares, with the quiet zone a scanner needs.
fn draw_qr<T: RenderTarget>(canvas: &mut Canvas<T>, url: &str, x: f32, y: f32, side: f32) {
    let Some(qr) = QrMatrix::encode(url) else {
        return;
    };
    // Four modules of quiet zone is what the spec asks for; two is the practical minimum and keeps
    // the code large enough to scan in a small panel.
    let quiet = 2usize;
    let modules = qr.size + quiet * 2;
    let cell = side / modules as f32;

    canvas.set_blend_mode(BlendMode::None);
    canvas.set_draw_color(Color::RGB(0xFF, 0xFF, 0xFF));
    let _ = canvas.fill_rect(FRect::new(x, y, side, side));
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    for row in 0..qr.size {
        for column in 0..qr.size {
            if qr.is_dark(column, row) {
                let _ = canvas.fill_rect(FRect::new(
                    x + (column + quiet) as f32 * cell,
                    y + (row + quiet) as f32 * cell,
                    // A hair over one cell, so rounding cannot leave hairline gaps that confuse a
                    // scanner.
                    cell + 0.5,
                    cell + 0.5,
                ));
            }
        }
    }
}

/// Greedy word wrap.
fn wrap(text: &str, max_chars: usize) -> Vec<String> {
    let max = max_chars.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= max {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use km_locale::Locale;

    use super::*;
    use crate::connect::ConnectProblem;

    /// The two halves of the name are the name.
    ///
    /// [`TITLE`] is what the fit test measures and [`TITLE_HALVES`] is what actually gets drawn, so
    /// they are one string written twice — the arrangement `docs/decisions/` calls a duplication
    /// that needs a guard rather than a duplication to remove, because the halves exist precisely so
    /// the two can be colored apart. Without this, renaming the product in one place and not the
    /// other would size the title against a name nobody sees.
    #[test]
    fn the_title_halves_spell_the_title() {
        assert_eq!(
            TITLE_HALVES.concat(),
            TITLE,
            "the idle screen would draw a different name from the one it measured"
        );
        assert!(
            !TITLE_HALVES.iter().any(|half| half.is_empty()),
            "an empty half would draw the whole name in one color and hide that this broke"
        );
    }

    /// The same six sizes `keypad.rs` pins its geometry against.
    const SCREENS: [(f32, f32); 6] = [
        (1280.0, 720.0),
        (1920.0, 1080.0),
        (3840.0, 2160.0),
        (4096.0, 2160.0),
        (2560.0, 1080.0),
        (1080.0, 2400.0),
    ];

    fn layout_for(theme: &Theme, width: f32, height: f32) -> Layout {
        Layout {
            w: width,
            h: height,
            margin: theme.margin_px(width as u32),
        }
    }

    /// The build number sits in the one bottom corner the connect panel does not hold, and the
    /// number pad stands above it rather than over it.
    ///
    /// **The cap is what is checked, not the string.** [`VERSION_MAX_WIDTH`] bounds anything a
    /// caller can pass, so proving the cap clear proves every version this will ever be handed —
    /// including a longer one than the six characters it is handed today.
    ///
    /// Both obstacles are asked where they are, the way
    /// `the_dialled_song_never_reaches_the_number_pad_or_the_connect_panel` asks them, rather than
    /// re-deriving a fraction that could drift from either.
    #[test]
    fn the_build_number_clears_the_number_pad_and_the_connect_panel() {
        let theme = Theme::default();

        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let (x, y) = version_origin(&theme, width, height);
            let line = theme.glyph_box_px(theme.small_size, height as u32);
            let right = x + width * VERSION_MAX_WIDTH;
            let bottom = y + line;

            assert!(
                x >= 0.0,
                "{width}x{height}: the build number is off the left"
            );
            assert!(
                bottom <= height,
                "{width}x{height}: the build number runs off the bottom at {bottom}"
            );

            let pad = Keypad::idle(width, height, version_reserve(&theme, width, height));
            if let Some((pad_x, pad_y, pad_w, pad_h)) = pad.bounds() {
                assert!(
                    x >= pad_x + pad_w || right <= pad_x || y >= pad_y + pad_h || bottom <= pad_y,
                    "{width}x{height}: the build number runs into the number pad"
                );
            }

            // Asked of `rect` for the reason the dialled title asks it: a transcribed fraction is a
            // second statement of where the panel is, and the two would drift.
            let panel = PanelSize::Full.rect(&theme, layout);
            assert!(
                x >= panel.x + panel.w
                    || right <= panel.x
                    || y >= panel.y + panel.h
                    || bottom <= panel.y,
                "{width}x{height}: the build number runs into the connect panel"
            );
        }
    }

    /// The idle screen stacks four things down the middle, and three of them are in the tall lyric
    /// face. This asserts the gaps rather than the numbers: every bound is computed from [`Theme`],
    /// so changing a font size fails here instead of putting two lines through each other on a
    /// television nobody is standing in front of.
    ///
    /// Fractions of height, not pixels — every one of these positions is a fraction, so the answer
    /// is the same at 720p and at 4K and there is nothing to loop over.
    #[test]
    fn the_catalog_summary_sits_between_the_title_and_the_prompt() {
        let theme = Theme::default();

        let title_bottom = TITLE_TOP + theme.glyph_box(theme.lyric_size);
        let summary_bottom = CATALOG_SUMMARY_TOP + theme.glyph_box(theme.small_size);

        assert!(
            title_bottom <= CATALOG_SUMMARY_TOP,
            "the summary starts at {CATALOG_SUMMARY_TOP} and {TITLE} runs to {title_bottom}"
        );
        assert!(
            summary_bottom <= PROMPT_TOP,
            "the summary runs to {summary_bottom} and the prompt starts at {PROMPT_TOP}"
        );
    }

    /// The message that produced all of this: 44 characters, which the lyric face draws wider than
    /// a 720p screen. What matters is that every line it becomes fits the width it was wrapped to —
    /// the failure being fixed is text that is simply wider than the screen it is centered on.
    #[test]
    fn a_message_no_face_can_hold_is_wrapped_to_the_width_it_has() {
        const MESSAGE: &str = "no melody channel was detected for this song";

        for (w, h) in SCREENS {
            let theme = Theme::default();
            let available = w - theme.margin_px(w as u32) * 2.0;
            let per_line = fit_chars(available, f32::from(theme.small_px(h as u32)));
            let lines = wrap_prompt(MESSAGE, per_line);

            assert!(!lines.is_empty(), "{w}x{h}: the message did not survive");
            assert!(
                lines.len() <= PROMPT_MAX_LINES,
                "{w}x{h}: {} lines is past the budget",
                lines.len()
            );
            for line in &lines {
                assert!(
                    line.chars().count() <= per_line,
                    "{w}x{h}: {line:?} is past {per_line} characters"
                );
            }
        }
    }

    /// A cut message says that it was cut. The standing notice's argument, in the other place a
    /// sentence can run past its room: one that simply stopped read as a message with nothing after
    /// its colon rather than as a message with more to it.
    #[test]
    fn a_message_past_two_lines_is_cut_with_an_ellipsis() {
        let lines = wrap_prompt(
            "could not install this package because the folder it names cannot be read, \
             and here is a great deal more about it than anybody wants on a television",
            24,
        );
        assert_eq!(lines.len(), PROMPT_MAX_LINES);
        assert!(
            lines.last().is_some_and(|last| last.ends_with('…')),
            "the cut is marked: {lines:?}"
        );
    }

    /// Wrapping downwards is only allowed because the rows under the prompt are free — which they
    /// are, and this is what says so if either of the two things below it ever moves up.
    #[test]
    fn a_wrapped_message_stops_above_everything_under_the_prompt() {
        let theme = Theme::default();

        let last_line_top = PROMPT_TOP + theme.small_size * 1.25 * (PROMPT_MAX_LINES - 1) as f32;
        let bottom = last_line_top + theme.glyph_box(theme.small_size);

        assert!(
            bottom <= PREVIEW_TITLE_TOP,
            "a message runs to {bottom} and the dialled title starts at {PREVIEW_TITLE_TOP}"
        );
    }

    /// The gradient stands behind anything that is not opaque, so the first wallpaper of a boot
    /// fades up out of it rather than out of the near-black clear color. Its old form asked
    /// whether a texture had been *handed over*, which a texture at alpha zero satisfies, and that
    /// is the whole of the bug.
    #[test]
    fn the_gradient_stands_behind_anything_that_is_not_opaque() {
        assert!(needs_gradient(false, false, 1.0), "nothing to draw at all");
        assert!(
            needs_gradient(false, true, 0.0),
            "an image at alpha zero hides nothing"
        );
        assert!(
            needs_gradient(false, true, 0.5),
            "half-way through a fade-in, the gradient is what it fades up out of"
        );
        assert!(
            !needs_gradient(false, true, 1.0),
            "a fully faded-in image covers the screen"
        );
        assert!(
            !needs_gradient(true, false, 0.0),
            "the outgoing image is always drawn at full alpha"
        );
        assert!(
            !needs_gradient(true, true, 0.0),
            "a crossfade always has the outgoing image underneath it"
        );
    }

    /// The cast to `u8` saturates, so a fade that is not a number becomes alpha 0 — an invisible
    /// image. That is only safe because the gradient goes behind it, and this pins both halves
    /// agreeing, which is the whole reason they share [`incoming_alpha`].
    #[test]
    fn a_fade_outside_zero_to_one_cannot_hide_the_gradient() {
        assert_eq!(incoming_alpha(0.0), 0);
        assert_eq!(incoming_alpha(1.0), 255);
        assert_eq!(incoming_alpha(-1.0), 0, "below the range is invisible");
        assert_eq!(incoming_alpha(2.0), 255, "above it is opaque");
        assert_eq!(incoming_alpha(f32::NAN), 0, "and not a number is invisible");

        assert!(
            needs_gradient(false, true, f32::NAN),
            "an image drawn at alpha zero needs the gradient however it got there"
        );
        assert!(!needs_gradient(false, true, 2.0), "an opaque image covers");
    }

    /// The closest a test gets to watching the screen: the render loop's wallpaper section run for
    /// several changes, with a decode that takes frames to finish, asserting what the eye was
    /// actually reporting — that the picture never drops out.
    ///
    /// Neither test above catches this alone. `Schedule`'s says the fade does not start early;
    /// `needs_gradient`'s says an invisible texture does not suppress the wash. The blink lived in
    /// the *join*, where a schedule already fading met a loop still holding one image, and this is
    /// the only test that puts the two together.
    #[test]
    fn once_a_wallpaper_is_up_the_screen_never_falls_back_to_the_gradient() {
        use crate::wallpaper::Schedule;
        use std::time::Duration;

        const FRAME: Duration = Duration::from_millis(16);
        const DECODE_FRAMES: u32 = 5;

        let mut schedule = Schedule::new(Duration::from_millis(100), Duration::from_millis(200));
        // Whether a texture exists is all the drawing cares about.
        let (mut current, mut outgoing) = (false, false);
        // Startup asks for the first image before the loop, exactly as `run_with` does.
        let mut decoding = Some(DECODE_FRAMES);
        let mut arrivals = 0;
        let mut settled = false;

        for frame in 0..400 {
            // The loader delivering.
            match decoding {
                Some(0) => {
                    outgoing = current;
                    current = true;
                    schedule.start_crossfade();
                    decoding = None;
                    arrivals += 1;
                }
                Some(left) => decoding = Some(left - 1),
                None => {}
            }

            // `Walls::request` is a no-op while one is in flight, and answers `true`.
            if schedule.tick(FRAME) && decoding.is_none() {
                decoding = Some(DECODE_FRAMES);
            }
            let presentation = schedule.presentation();
            if !presentation.crossfading {
                outgoing = false;
            }

            // The first image has nothing to fade from, so it is allowed to rise out of the
            // gradient. Every change after it is not.
            settled |= arrivals >= 1 && !presentation.crossfading;
            assert!(
                !settled || !needs_gradient(outgoing, current, presentation.fade_in),
                "frame {frame}: the picture dipped back to the gradient — current {current}, \
                 outgoing {outgoing}, fade {}",
                presentation.fade_in
            );
        }

        assert!(
            arrivals >= 3,
            "the simulation should have changed image a few times, got {arrivals}"
        );
    }

    /// The bank label is drawn on every frame the switcher is on for, so unlike the overlays it has
    /// no moment at which the screen is otherwise empty: it has to sit clear of its row's
    /// neighbors, above and below.
    ///
    /// Fractions of height throughout, like the summary test above, because all these positions are.
    #[test]
    fn the_soundfont_label_sits_below_the_badges_and_above_the_lyrics() {
        let theme = Theme::default();
        let line = theme.glyph_box(theme.small_size);

        // The badges share the title's row. **Named rather than transcribed**, which is the whole
        // change here: this said `0.05` and the lyric bound below said `0.45`, both copied out of
        // `draw_playing` — so moving a row left this test passing about numbers nothing drew.
        let badges_bottom = TITLE_ROW_TOP + line;
        assert!(
            badges_bottom <= SOUNDFONT_LABEL_TOP,
            "the badges run to {badges_bottom} and the bank label starts at {SOUNDFONT_LABEL_TOP}"
        );

        // And it must not reach the lyrics, which are centered far below — the real neighbor is the
        // flash band, which takes the top of the screen when it is up and would cover the badges
        // too. This is the cheap half of that: nothing between the two rows.
        let label_bottom = SOUNDFONT_LABEL_TOP + line;
        assert!(
            label_bottom < NO_LYRICS_TOP,
            "the bank label runs to {label_bottom}, which reaches the lyric line at {NO_LYRICS_TOP}"
        );
    }

    /// The developer marker takes the third row of the top right and collides with nothing.
    ///
    /// **It is on screen for as long as the state is true**, on both screens, which is stronger than
    /// the bank label above it: that one appears only while the switcher is configured, and this one
    /// is drawn precisely on the machines somebody is working on — which are the machines where the
    /// frame panel is also likely to be up. So every neighbor is asserted, above and beside.
    #[test]
    fn the_developer_marker_clears_the_bank_label_the_lyrics_and_the_frame_panel() {
        let theme = Theme::default();
        let line = theme.glyph_box(theme.small_size);

        let label_bottom = SOUNDFONT_LABEL_TOP + line;
        assert!(
            label_bottom <= DEVELOPER_LABEL_TOP,
            "the bank label runs to {label_bottom} and the marker starts at {DEVELOPER_LABEL_TOP}"
        );

        let marker_bottom = DEVELOPER_LABEL_TOP + line;
        assert!(
            marker_bottom < NO_LYRICS_TOP,
            "the marker runs to {marker_bottom}, which reaches the lyric line at {NO_LYRICS_TOP}"
        );

        // **The one that is not about rows, and they genuinely overlap vertically.** The frame panel
        // starts at `PERFORMANCE_TOP`, which is 0.16 — *above* this marker's 0.17 — and runs down
        // from there, so there is no vertical gap to assert and looking for one would be looking for
        // the wrong thing. What keeps them apart is that the panel is drawn from the left margin and
        // the marker is right-aligned and capped: they share a band and not a column.
        //
        // Measured on 720p, the floor a television gets, against the box `draw_performance`
        // actually fills — no test in this crate may open a font, and this needs none, because the
        // panel's width is a constant and not a measurement of its text.
        //
        // **That the text cannot push the box is the property this rests on, and it is the
        // alignment that gives it.** Every value is right-aligned at the panel's right edge and
        // grows leftwards, so a row wider than expected reaches into the panel's own padding rather
        // than toward the marker; the only thing running rightwards is the label, and labels come
        // from this crate's own catalog. `ellipsize` against `fit_chars` cuts them both anyway.
        let width = 1280.0_f32;
        let small_px = f32::from(theme.small_px(720));
        let pad = small_px * 0.8;
        let panel_right = width * 0.02 + PERFORMANCE_COLUMNS * small_px * 0.5 + pad * 2.0;
        let marker_left = width - width * 0.02 - (width - width * 0.04) * DEVELOPER_LABEL_MAX_WIDTH;
        assert!(
            panel_right < marker_left,
            "the frame panel reaches {panel_right}px and the marker may start at {marker_left}px"
        );

        // **And `next:` is on that row exactly, not merely in the band.** It is drawn from the left
        // margin and cut to `NEXT_UP_MARKED_WIDTH` while the marker is up, which is *defined* as
        // everything the marker's own cap leaves — so that the two cannot meet is arithmetic the
        // constant already carries, and asserting it here would assert a definition.
        //
        // What is worth holding is that the remainder is a row somebody can read. Twelve characters
        // is the bank label's floor one row up, for its reasoning: below that a line is present and
        // useless. `next:` is a title and a singer, so a cut this tight ellipsizes on a phone in
        // portrait — a bound rather than a target, the machine holding itself in landscape.
        let chars = fit_chars((width - width * 0.04) * NEXT_UP_MARKED_WIDTH, small_px);
        assert!(
            chars >= 12,
            "with the marker up only {chars} characters of `next:` fit, which will not name a song"
        );
    }

    /// A position reads as minutes and seconds, and gains an hour column only when it needs one.
    #[test]
    fn a_clock_is_minutes_and_seconds_until_an_hour() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(59_999), "0:59");
        assert_eq!(clock(83_400), "1:23");
        assert_eq!(clock(3_599_000), "59:59");
        assert_eq!(clock(3_723_000), "1:02:03");
    }

    /// The panel at its tallest still stops above the row the demo line has to itself.
    ///
    /// **The assertion this panel's height rests on.** Its rows vary with the song, so how far down
    /// it reaches is not a number anybody can hold in their head while adding one — and the thing it
    /// must not reach is `DEMO_LABEL_TOP`, a full-width row carrying an instruction. Every other
    /// overlap on the way down is with the lyric ladder, which this diagnostic is allowed to sit
    /// over.
    ///
    /// Both row counts come from the two structs rather than from the drawing, which is what lets
    /// this be asked without a font or a screen.
    #[test]
    fn the_panel_at_its_tallest_stays_above_the_demo_line() {
        // A decoder complaining over a damaged MIDI file: the worst either block gets, and the two
        // are independent, so the worst panel is genuinely both at once.
        let frames = FrameStats {
            frames: 60,
            starved_ms: 412,
            ..FrameStats::default()
        };
        let song = SongStats {
            kind: SongMedia::Midi,
            gain: 1.0,
            gain_source: GainSource::Unmeasured,
            bank_ignored: 0,
            muted: 0,
            flavor: None,
            dialect: None,
            tracks: 9,
            truncated_tracks: 1,
            missing_tracks: 0,
            repaired_notes: 47,
        };
        assert_eq!(frames.rows(), 8);
        assert_eq!(song.rows(), 7);

        // The fractions `draw_performance` works in: `small_size` of the height per nominal row,
        // times 1.5 for the row pitch, plus the padding at both ends.
        let theme = Theme::default();
        let row_h = theme.small_size * 1.5;
        let pad = theme.small_size * 0.8;
        let rows = (frames.rows() + song.rows()) as f32;
        let bottom = PERFORMANCE_TOP + pad * 2.0 + row_h * rows;
        assert!(
            bottom < DEMO_LABEL_TOP,
            "the panel reaches {bottom} of the height and the demo line sits at {DEMO_LABEL_TOP}"
        );
    }

    /// How wide a digit is, in ems, in the widest face `text::CANDIDATES` can turn up — DejaVu
    /// Sans, whose digits all advance 1303/2048. Segoe UI, Arial and Roboto are narrower, so this is
    /// the pessimistic answer, which is the one a bound wants. The idiom, and the reason for it, are
    /// `TITLE_EM`'s below: no test in this crate may open a font.
    const DIGIT_EM: f32 = 0.636;

    /// How tall a line is against its nominal size, pessimistically.
    ///
    /// **Not `Theme::glyph_box`, and the gap is the point.** That constant is 1.2 and the small face
    /// measured 26 pixels at a nominal 19 — 1.368 — so a bound computed from the glyph box would be
    /// 14% under the truth and could pass while the real pill collided. 1.45 is the measured ratio
    /// with headroom, and the direction of the error is the safe one: the test reasons about a pill
    /// larger than any face has produced.
    const LINE_EM: f32 = 1.45;

    /// One waiting song is a circle, and three digits stretch it sideways without making it taller.
    ///
    /// **The circle is a promise the padding can break**, which is the whole point of testing it:
    /// [`queue_pill_rect`] floors the width at the height, so a single digit is round *provided* the
    /// digit and its padding fit inside that floor. The last assertion is that inequality itself, so
    /// raising [`QUEUE_PILL_SIDE_PAD`] past what a circle can hold fails here rather than quietly
    /// producing a slightly oval `4` nobody looks at twice.
    #[test]
    fn a_one_digit_count_comes_out_round() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let small = f32::from(theme.small_px(height as u32));
            let (digit, line) = (small * DIGIT_EM, small * LINE_EM);

            let one = queue_pill_rect(layout, digit, line);
            assert!(
                (one.w - one.h).abs() < 0.5,
                "at {width}x{height} a one-digit pill is {}x{}, which is not round",
                one.w,
                one.h
            );

            let three = queue_pill_rect(layout, digit * 3.0, line);
            assert!(
                three.w > one.w && (three.h - one.h).abs() < f32::EPSILON,
                "at {width}x{height} three digits gave {}x{} against one digit's {}x{}",
                three.w,
                three.h,
                one.w,
                one.h
            );
        }

        // The bound the circle rests on, stated once rather than per screen: a digit plus its
        // padding at both ends has to fit inside the pill's own height, which is a glyph box grown
        // by `QUEUE_PILL_HEIGHT`. Written as the inequality rather than as the number it comes to,
        // so raising *either* constant is what fails here.
        let pill_em = theme.glyph_box(1.0) * QUEUE_PILL_HEIGHT;
        assert!(
            DIGIT_EM + pill_em * QUEUE_PILL_SIDE_PAD <= pill_em,
            "a digit ({DIGIT_EM} em) padded by {QUEUE_PILL_SIDE_PAD} of a {pill_em}-em pill \
             cannot fit in a circle"
        );
    }

    /// The pill stands in the corner, and stops before the row beneath it.
    ///
    /// Pixels rather than fractions, unlike the label test above, because
    /// [`Theme::glyph_box_px`] rounds the face's size — so the answer genuinely differs per screen
    /// and there is something to loop over.
    #[test]
    fn the_queue_pill_owns_the_corner_and_clears_the_second_row() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            // The widest it ever gets: `MAX_QUEUED` is 200, so three digits.
            let small = f32::from(theme.small_px(height as u32));
            let widest = queue_pill_rect(layout, small * DIGIT_EM * 3.0, small * LINE_EM);

            assert!(
                (widest.x + widest.w - (layout.w - layout.margin)).abs() < 0.5,
                "at {width}x{height} the pill's right edge is {}, not the margin at {}",
                widest.x + widest.w,
                layout.w - layout.margin
            );
            assert!(
                (widest.y - layout.h * TITLE_ROW_TOP).abs() < 0.5,
                "at {width}x{height} the pill's top is {}, not the title row at {}",
                widest.y,
                layout.h * TITLE_ROW_TOP
            );

            let second_row = layout.h * SECOND_ROW_TOP;
            assert!(
                widest.y + widest.h < second_row,
                "at {width}x{height} the pill runs to {} and the artist line and the bank label \
                 start at {second_row}",
                widest.y + widest.h
            );
        }
    }

    /// What is left of the badge run once the pill has taken the corner.
    ///
    /// **A floor rather than a target**, in [`DEMO_LABEL_MIN_CHARS`]'s sense. Sixteen characters is
    /// `key -12   melody`, the shortest run of two badges worth drawing. The binding case is a phone
    /// held in portrait, which gets 27; every landscape screen gets over a hundred. The machine holds
    /// itself in landscape on Android, so portrait is a bound rather than a screen anybody sings at.
    #[test]
    fn the_badge_run_keeps_enough_of_its_row_to_be_worth_drawing() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let small = f32::from(theme.small_px(height as u32));
            let pill = queue_pill_rect(layout, small * DIGIT_EM * 3.0, small * LINE_EM);
            let right_edge = pill.x - QUEUE_PILL_GAP * layout.w;
            assert!(
                right_edge > layout.margin,
                "at {width}x{height} the pill leaves the badge run no row at all"
            );

            let small_px = f32::from(theme.small_px(height as u32));
            let chars = fit_chars(right_edge - layout.margin, small_px);
            assert!(
                chars >= 16,
                "at {width}x{height} the badge run has {chars} characters, too few for \
                 `key -12   melody`"
            );
        }
    }

    /// The fault line stays centered on the screen and still clears the pill.
    ///
    /// The reserve is taken off **both** sides, so the sentence keeps the axis the title, the
    /// catalog summary and the prompt are all centered on. A line centered in the space left over
    /// beside the pill would read as a line that had slipped rather than as one making room.
    #[test]
    fn the_standing_fault_line_clears_the_queue_pill() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let small = f32::from(theme.small_px(height as u32));
            let pill = queue_pill_rect(layout, small * DIGIT_EM * 3.0, small * LINE_EM);
            let reserve = (layout.w - layout.margin - pill.x) + QUEUE_PILL_GAP * layout.w;
            let budget = layout.w - (layout.margin + reserve) * 2.0;
            assert!(
                budget > 0.0,
                "at {width}x{height} the pill leaves the fault line no width at all"
            );
            assert!(
                layout.w / 2.0 + budget / 2.0 <= pill.x,
                "at {width}x{height} the fault line reaches {} and the pill starts at {}",
                layout.w / 2.0 + budget / 2.0,
                pill.x
            );
        }
    }

    /// Nothing wrong is the default, and both areas are named when both have something.
    #[test]
    fn faults_are_counted_and_named_by_area() {
        assert!(Faults::default().is_empty());
        assert_eq!(Faults::default().line(Locale::English), None);

        let both = Faults {
            packages: 3,
            sound: 1,
        };
        assert_eq!(both.total(), 4);
        assert_eq!(
            both.areas().collect::<Vec<_>>(),
            vec![FaultArea::Packages, FaultArea::Sound],
            "packages are named first — the order `display.rs` used to arbitrate its one slot with"
        );

        assert_eq!(
            both.line(Locale::English).as_deref(),
            Some("4 problems: packages, sound")
        );
        assert_eq!(
            Faults {
                packages: 1,
                sound: 0
            }
            .line(Locale::English)
            .as_deref(),
            Some("1 problem: packages"),
            "the singular, because `1 problems` is what makes somebody doubt the rest of the screen"
        );
        // The line the television could not say in Portuguese until it stopped quoting a reason.
        assert_eq!(
            both.line(Locale::BrazilianPortuguese).as_deref(),
            Some("4 problemas: pacotes, som")
        );
    }

    /// The row is shared with the `artist · language` line, so the label may not claim so much of
    /// the width that the two are bound to meet.
    ///
    /// Looped over [`SCREENS`] rather than done in fractions, because the cap is a fraction of
    /// *width* and the margin is too, while the font size comes from *height* — so the number of
    /// characters that survive genuinely differs by aspect ratio, which is what an ultrawide and a
    /// phone are in this list to catch.
    #[test]
    fn the_soundfont_label_leaves_the_artist_line_most_of_its_row() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let usable = width - layout.margin * 2.0;
            let label_width = usable * SOUNDFONT_LABEL_MAX_WIDTH;

            assert!(
                label_width < usable / 2.0,
                "at {width}x{height} the bank label may take {label_width} of {usable}, which is \
                 at least half the row"
            );

            // And enough of it to say something. A cap so tight that the slot number and a short
            // bank name could not both fit would be a label that is present and useless.
            //
            // Twelve, which is `sf 3/5 sc55` and not much more, because the portrait phone in
            // `SCREENS` is the binding case: the face is sized from height and the row from width,
            // so 1080x2400 gets the largest text on the narrowest row. The machine holds itself in
            // landscape on Android, so that combination is a bound rather than a target — but it is
            // the bound, and a longer bank name is ellipsized there.
            let chars = fit_chars(label_width, f32::from(theme.small_px(height as u32)));
            assert!(
                chars >= 12,
                "at {width}x{height} only {chars} characters fit, which will not hold a slot and a \
                 bank name"
            );
        }
    }

    /// `next:` clears the `artist · language` line above it and the lyric ladder below.
    ///
    /// **The third row of a column whose other two rows are a song's own name**, so both neighbors
    /// are the ones a singer reads and neither may be touched. The lyric bound comes from
    /// [`Theme::lyric_band`] rather than from a fraction copied here, because a theme that lifted
    /// the ladder would otherwise leave this test passing about a number nothing draws.
    #[test]
    fn next_up_sits_below_the_artist_line_and_above_the_lyrics() {
        let theme = Theme::default();
        let line = theme.glyph_box(theme.small_size);
        let (lyric_top, _) = theme.lyric_band();

        let artist_bottom = SECOND_ROW_TOP + line;
        assert!(
            artist_bottom <= NEXT_UP_TOP,
            "the artist line runs to {artist_bottom} and `next:` starts at {NEXT_UP_TOP}"
        );

        let next_bottom = NEXT_UP_TOP + line;
        assert!(
            next_bottom < lyric_top,
            "`next:` runs to {next_bottom} and the lyrics start at {lyric_top}"
        );
    }

    /// The demo line clears the lyrics above it and the position bar below, and touches neither.
    ///
    /// This is what makes the row-of-its-own decision real rather than a number that happens to
    /// look free today: if the lyric band grows downwards or the bar thickens, one of these fails
    /// instead of two things quietly overlapping on a television.
    ///
    /// The bar's top is taken from [`timeline_rect`] rather than from a fraction, because the bar
    /// rests on the bottom safe inset and its thickness has a floor: a fraction transcribed here
    /// would measure a gap nothing draws.
    #[test]
    fn the_demo_line_clears_the_lyrics_above_it_and_the_position_bar_below() {
        let theme = Theme::default();
        let line = theme.glyph_box(theme.small_size);
        let lyrics_bottom = theme.lyric_row_top
            + (crate::lyrics::ROWS as f32 - 1.0) * theme.lyric_row_height
            + theme.glyph_box(theme.lyric_size);

        assert!(
            DEMO_LABEL_TOP > lyrics_bottom,
            "the demo line starts at {DEMO_LABEL_TOP} and the lyrics run to {lyrics_bottom}"
        );

        for (width, height) in SCREENS {
            let bar_top = timeline_rect(layout_for(&theme, width, height)).y / height;
            assert!(
                DEMO_LABEL_TOP + line <= bar_top,
                "at {width}x{height} the demo line runs to {} and the position bar starts at \
                 {bar_top}",
                DEMO_LABEL_TOP + line
            );
        }
    }

    /// The whole instruction has to fit on every screen, which is why it does not share a row.
    ///
    /// A line sharing a row is a line capped at part of the width, and the portrait phone in
    /// `SCREENS` is the binding case — the face is sized from height and the row from width, so half
    /// of one fits fifteen characters. `DEMO · press S` is worse than nothing: it says there is
    /// something to do and not what.
    #[test]
    fn the_whole_instruction_fits_on_every_screen() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let usable = width - layout.margin * 2.0;
            let label_width = usable * DEMO_LABEL_MAX_WIDTH;

            assert!(
                label_width <= usable,
                "at {width}x{height} the demo line would run under the margin"
            );

            // Long enough for the instruction, not merely for the state. A line saying only `DEMO`
            // tells somebody the machine is not broken and still leaves them waiting for a song
            // that has no queue entry to run out. [`DEMO_LABEL_MIN_CHARS`] is `DEMO · QUEUE to sing
            // next`, which is what `karaokemachine`'s `DEMO_LABEL` holds — and the portrait phone in
            // `SCREENS` fits 29, so this is the bound the wording was chosen against rather than a
            // number picked to pass. `draw_playing` reads the same constant when it decides whether
            // the standing connect panel fits beside this line.
            let chars = fit_chars(label_width, f32::from(theme.small_px(height as u32)));
            assert!(
                chars >= DEMO_LABEL_MIN_CHARS,
                "at {width}x{height} only {chars} characters fit, which will cut the one \
                 instruction a demo song has to give"
            );
        }
    }

    /// The bar is inside the safe area, which is the whole reason it moved.
    ///
    /// **A television does not show the edges of the picture it is handed.** The bar's body sat
    /// below the bottom safe inset and the set ate it, along with the build number's leading letter
    /// in the corner beside it. Measured against `keypad::margin_y` rather than against a fraction
    /// copied here, so the two cannot come apart.
    #[test]
    fn the_position_bar_is_inside_the_safe_area() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let bar = timeline_rect(layout);
            let safe_bottom = height - crate::keypad::margin_y(height);
            let safe_left = crate::keypad::margin_x(width);

            assert!(
                bar.y + bar.h <= safe_bottom + 0.01,
                "at {width}x{height} the bar runs to {} and the safe area ends at {safe_bottom}",
                bar.y + bar.h
            );
            assert!(
                bar.x >= safe_left - 0.01 && bar.x + bar.w <= width - safe_left + 0.01,
                "at {width}x{height} the bar spans {}..{} against a safe area of \
                 {safe_left}..{}",
                bar.x,
                bar.x + bar.w,
                width - safe_left
            );
        }
    }

    /// The transport strip stands above the bar rather than over it.
    ///
    /// The same arrangement `the_number_pad_leaves_room_for_the_build_number` holds one screen over,
    /// and read the same way: the strip through its own `bounds` and the bar through
    /// [`timeline_rect`], because one expression decides each and a test naming its own figure would
    /// go on passing after the two stopped agreeing.
    ///
    /// Both `melody_available` and both `hints`, since either changes how many keys there are and
    /// how tall they end up after the shrink-to-fit.
    #[test]
    fn the_transport_strip_stands_above_the_position_bar() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let bar = timeline_rect(layout);
            for melody in [false, true] {
                for hints in [false, true] {
                    let strip =
                        Keypad::playing(width, height, melody, hints, position_reserve(height));
                    let Some((_, top, _, strip_h)) = strip.bounds() else {
                        continue;
                    };
                    assert!(
                        top + strip_h <= bar.y + 0.01,
                        "at {width}x{height} (melody {melody}, hints {hints}) the strip runs to \
                         {} and the bar starts at {}",
                        top + strip_h,
                        bar.y
                    );
                }
            }
        }
    }

    /// The furniture stands at full strength, over a picture as over a screen the machine drew.
    ///
    /// A row read from a sofa for the whole of a song is a row that has to be readable, and what
    /// carries these over an arbitrary frame is the ring rather than a thinner ink. The guarantee
    /// worth pinning is that nothing on this screen asks for less: a style drawn a shade off what
    /// the theme names is a row somebody has to hunt for.
    #[test]
    fn the_furniture_stands_at_full_strength_over_a_picture() {
        let theme = Theme::default();
        let style = TextStyle::outlined(theme.text, &theme, crate::text::Align::Left);
        assert_eq!(style.alpha, 1.0);
        assert_eq!(
            TextStyle::plain(theme.background, crate::text::Align::Center).alpha,
            1.0
        );

        // The opacity a style can carry scales what it already has rather than replacing it, which
        // is what lets a caller pass a factor unconditionally. `theme::fade` says the same of a
        // color, and serves the position bar's two fills, which have no glyph to compose.
        assert_eq!(style.faded(1.0), style, "full strength is a true identity");
        let panel = crate::theme::fade(theme.panel, 0.6);
        assert!(
            panel.a < theme.panel.a,
            "the panel came back at {:#04x}, which is not scaled from {:#04x}",
            panel.a,
            theme.panel.a
        );
    }

    /// The panel that stands through a demo song sits in the gap the demo line already found.
    ///
    /// Computed from the [`Theme`] rather than from [`STANDING_QR_SIDE`] and friends read back, so
    /// that changing the small face — which sets the caption's line, which sets the panel's height —
    /// fails here rather than putting a QR code through the lyrics on a television.
    #[test]
    fn the_standing_panel_clears_the_lyrics_and_the_progress_bar() {
        let theme = Theme::default();
        let (_, lyrics_bottom) = theme.lyric_band();

        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            if !standing_panel_fits(&theme, layout) {
                continue;
            }
            let panel = PanelSize::Standing.rect(&theme, layout);

            assert!(
                panel.y / height > lyrics_bottom,
                "at {width}x{height} the standing panel starts at {} and the lyrics run to \
                 {lyrics_bottom}",
                panel.y / height
            );
            let bar = timeline_rect(layout);
            assert!(
                panel.y + panel.h <= bar.y,
                "at {width}x{height} the standing panel runs to {} and the position bar starts at \
                 {}",
                panel.y + panel.h,
                bar.y
            );
        }
    }

    /// A smaller card carrying a bigger code, which is the whole argument for the shape.
    ///
    /// If this ever fails there is no reason to have a third size at all: the `Overlay` panel would
    /// be both more scannable and more informative, and the standing one would be a smaller box that
    /// says less.
    #[test]
    fn the_standing_panel_holds_a_bigger_qr_than_the_overlay_it_replaces() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            if !standing_panel_fits(&theme, layout) {
                continue;
            }

            let standing_side = Standing::of(&theme, layout).qr;
            // The size the overlay actually draws, asked of the same type that draws it. It used to
            // be re-derived here as `overlay.h - (overlay.w * 0.05) * 2.0` — a copy of an expression
            // that no longer exists, and one that would have gone on passing while the overlay's box
            // grew a code out from under this claim.
            let overlay_side = Card::of(&theme, layout, PanelSize::Overlay).qr;

            assert!(
                standing_side > overlay_side,
                "at {width}x{height} the standing code is {standing_side} against the overlay's \
                 {overlay_side}, so dropping the text column bought nothing"
            );
        }
    }

    /// The address under the code is readable, which is the half a QR code cannot do.
    ///
    /// **This test is the reason the panel is not square.** Sized on both axes from the code, at
    /// 1280x720 it came out 130 pixels wide and printed `http://19…` — a caption that is worse than
    /// no caption, because somebody whose camera will not focus is exactly who it is for.
    #[test]
    fn the_standing_panel_prints_the_whole_address_under_the_code() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            if !standing_panel_fits(&theme, layout) {
                continue;
            }
            // `Standing::width` is what is inside the padding, which is exactly the room the caption
            // is drawn into — `draw_connect_panel` takes the same two pads off the box again.
            let it = Standing::of(&theme, layout);
            let chars = fit_chars(it.width, f32::from(theme.small_px(height as u32)));

            assert!(
                chars >= STANDING_URL_CHARS,
                "at {width}x{height} the caption fits {chars} characters, and the longest address \
                 `km_api::connect` can hand it is {STANDING_URL_CHARS}"
            );
        }
    }

    /// The key hint is said whole on every screen the machine is used on, in both languages.
    ///
    /// **The `Overlay` is the narrow one of the two panels that carry it**, and its text column is
    /// what a translation has to survive — the hint is drawn whole or not at all, so a sentence that
    /// does not fit is a sentence nobody ever sees. The catalog is where that gets fixed and this is
    /// what says so, in the shape `the_connect_panel_says_the_whole_of_its_own_messages` already
    /// uses for the machine's own explanations.
    ///
    /// **The portrait phone is the exception, as it is for every other block on this panel.** Nine
    /// characters fill a line there and the address count alone runs to three, so the hint is not
    /// drawn; asserted as an exact list, so a second screen joining it is a failure and not a
    /// silence.
    ///
    /// **Both spellings of the key are measured, and the modified one is five characters longer.**
    /// macOS is the platform that gets `Ctrl+F11`, and a line that fits one way and not the other
    /// would be a panel that says less on the platform it says most to. The two lists are asserted
    /// separately so a screen that only the longer sentence loses is named as itself.
    #[test]
    fn the_key_hint_is_said_whole_or_not_at_all() {
        let theme = Theme::default();
        let mut info = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://10.0.0.5:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        info.factory_pin = Some("482913".to_owned());

        for key in [
            crate::connect::BrowserKey::F11,
            crate::connect::BrowserKey::CtrlF11,
        ] {
            info.browser_key = Some(key);
            let mut dropped = Vec::new();

            for locale in [Locale::English, Locale::BrazilianPortuguese] {
                let panel = info.panel(locale);
                let (detail, hint) = (
                    panel.detail.expect("a count and a PIN"),
                    panel.hint.expect("a build with the key says so"),
                );
                for (width, height) in SCREENS {
                    let layout = layout_for(&theme, width, height);
                    for size in [PanelSize::Full, PanelSize::Overlay] {
                        let it = Card::of(&theme, layout, size);
                        let per_line =
                            fit_chars(it.text_w(true), f32::from(theme.small_px(height as u32)));
                        let used = wrap_capped(&detail, per_line, DETAIL_MAX_LINES).len();
                        let room = DETAIL_MAX_LINES.saturating_sub(used).min(HINT_MAX_LINES);
                        if wrap(&hint, per_line).len() > room && !dropped.contains(&(width, height))
                        {
                            dropped.push((width, height));
                        }
                    }
                }
            }

            assert_eq!(
                dropped,
                vec![(1080.0, 2400.0)],
                "exactly one screen in SCREENS has no room left for the {key:?} hint, and it is \
                 the portrait phone"
            );
        }
    }

    /// The hint takes a row the panel already had, because there is no room for one it did not.
    ///
    /// **This is the measurement the design rests on, kept rather than remembered.** Both panels are
    /// bottom-anchored under the lyric band, and the clearance is not the same on every screen: the
    /// ultrawide leaves the standing card **nine pixels** against a row of small text that costs
    /// forty-four, so a fourth row puts a QR code through the words being sung. Growing the box is
    /// not a thing this panel can do, and the next person to consider it should find the number here
    /// rather than in a failing test they have to work backwards from.
    #[test]
    fn the_key_hint_shares_the_panels_rows_rather_than_growing_its_box() {
        let theme = Theme::default();
        let (_, lyrics_bottom) = theme.lyric_band();
        let mut tightest = f32::MAX;

        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            if !standing_panel_fits(&theme, layout) {
                continue;
            }
            let card = PanelSize::Standing.rect(&theme, layout);
            let it = Standing::of(&theme, layout);
            // What a row actually costs: the air above it as well as the glyph box.
            let row = it.gap + it.text_h;
            let clearance = card.y - lyrics_bottom * height;
            assert!(
                clearance < row,
                "at {width}x{height} the standing card has {clearance} of clearance against a row \
                 of {row}, so it could carry the hint on a line of its own after all"
            );
            tightest = tightest.min(clearance / height);
        }

        assert!(
            tightest < 0.01,
            "the tightest screen leaves {tightest} of the height, and the nine pixels this test \
             was written about were 0.0083"
        );
    }

    /// Every row the panel draws is inside the box it draws them on.
    ///
    /// **This is the failure [`Card`] was written for.** The box was `0.26h` and `0.18h` of the
    /// screen and the rows were `0.55`, `0.70` and `0.85` of *the box*, over a padding taken from
    /// the box's **width** — three rules that never met. The second detail line overhung the bottom
    /// edge by a couple of pixels and the third by tens, so a machine bound to loopback said
    /// `Listening on 127.0.0.1:8177.` and then a half-height line where the fix should have been.
    ///
    /// Per screen rather than as a fraction, because the rows are pixel quantities: the face is
    /// opened at a rounded pixel size and the step between lines is measured in them.
    #[test]
    fn every_row_of_the_connect_panel_stays_inside_its_box() {
        let theme = Theme::default();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            for size in [PanelSize::Full, PanelSize::Overlay] {
                let it = Card::of(&theme, layout, size);
                let box_ = size.rect(&theme, layout);

                // And across: the code inside the right edge, the text column a positive width.
                assert!(
                    it.qr_top(box_.h) >= it.pad && it.qr_top(box_.h) + it.qr <= box_.h - it.pad,
                    "at {width}x{height} {size:?} puts a {}-tall code in a {}-tall box",
                    it.qr,
                    box_.h
                );
                assert!(
                    it.qr + it.pad * 2.0 <= box_.w,
                    "at {width}x{height} {size:?} draws a {}-wide code in a {}-wide box",
                    it.qr,
                    box_.w
                );
                assert!(
                    it.text_w(true) > 0.0 && it.text_w(true) <= it.text_w(false),
                    "at {width}x{height} {size:?} leaves {} for the words beside the code",
                    it.text_w(true)
                );

                // The whole panel is on the screen, which is the outer half of the same claim.
                assert!(
                    box_.x >= 0.0
                        && box_.y >= 0.0
                        && box_.x + box_.w <= width
                        && box_.y + box_.h <= height,
                    "at {width}x{height} {size:?} hangs off the screen"
                );
            }
        }
    }

    /// The sentence the machine actually has to say fits in the lines it is given.
    ///
    /// The cut with an ellipsis is the backstop for a translation nobody has written yet, not the
    /// normal path for the messages this crate ships — and it very nearly was: the overlay wrapped
    /// the loopback explanation to **four** lines and dropped the fourth silently, because the text
    /// column had a square subtracted from it for a QR code that state never draws.
    ///
    /// The strings are the real ones, from the catalog, in both languages.
    ///
    /// **The portrait phone is the exception, as it is for the demo line.** Its small face is sized
    /// from a height more than twice its width, so sixteen characters fill a line and no panel this
    /// shape holds a sentence; the ellipsis is doing exactly the job it is there for. Asserted as an
    /// exact list rather than skipped, so a second screen joining it is a failure and not a silence.
    #[test]
    fn the_connect_panel_says_the_whole_of_its_own_messages() {
        let theme = Theme::default();
        let info = ConnectInfo::unreachable(
            ConnectProblem::LoopbackOnly,
            Some("127.0.0.1:8177".to_owned()),
        );
        let mut cut = Vec::new();

        for locale in [Locale::English, Locale::BrazilianPortuguese] {
            let detail = info.panel(locale).detail.expect("loopback explains itself");
            for (width, height) in SCREENS {
                let layout = layout_for(&theme, width, height);
                for size in [PanelSize::Full, PanelSize::Overlay] {
                    let it = Card::of(&theme, layout, size);
                    // No address, so no code: the sentence has the whole column, which is the point.
                    let per_line =
                        fit_chars(it.text_w(false), f32::from(theme.small_px(height as u32)));
                    let lines = wrap(&detail, per_line);
                    if lines.len() > DETAIL_MAX_LINES && !cut.contains(&(width, height)) {
                        cut.push((width, height));
                    }
                }
            }
        }

        assert_eq!(
            cut,
            vec![(1080.0, 2400.0)],
            "exactly one screen in SCREENS cannot hold the machine's own explanation, and it is \
             the portrait phone"
        );
    }

    /// The overlay grew to hold its third line, and stopped short of the words being sung.
    ///
    /// It is bottom-anchored, so the whole of that growth went upward — from `0.18h` to about
    /// `0.21h` — and the thing above it on a playing screen is the lyrics. The clearance is thinnest
    /// on the ultrawide, where the margin is a fraction of a much larger width: this is what fails
    /// if the panel's padding is ever loosened.
    #[test]
    fn the_overlay_panel_stops_above_the_lyrics() {
        let theme = Theme::default();
        let (_, lyrics_bottom) = theme.lyric_band();
        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            let top = PanelSize::Overlay.rect(&theme, layout).y / height;
            assert!(
                top > lyrics_bottom,
                "at {width}x{height} the overlay starts at {top} and the lyrics run to \
                 {lyrics_bottom}"
            );
        }
    }

    /// The demo line outranks the panel, and this is where that is decided.
    ///
    /// `draw_playing` stands the panel only where the line beside it still fits
    /// [`DEMO_LABEL_MIN_CHARS`]. This walks the same arithmetic over `SCREENS` and pins both halves
    /// of the answer: every landscape screen keeps the panel *and* the whole instruction, and the
    /// portrait phone — where the small face is sized from a height more than twice the width —
    /// gives the panel up rather than the sentence.
    #[test]
    fn the_standing_panel_never_cuts_the_instruction_it_stands_beside() {
        let theme = Theme::default();
        let mut dropped = Vec::new();

        for (width, height) in SCREENS {
            let layout = layout_for(&theme, width, height);
            if standing_panel_fits(&theme, layout) {
                // And where it is drawn, the line beside it still says the whole thing. This is the
                // half that would go unnoticed: a panel that fitted by shortening the sentence would
                // pass a clearance test and fail the feature.
                let chars = fit_chars(
                    standing_bottom_left_width(&theme, layout) * DEMO_LABEL_MAX_WIDTH,
                    f32::from(theme.small_px(height as u32)),
                );
                assert!(
                    chars >= DEMO_LABEL_MIN_CHARS,
                    "at {width}x{height} the panel stands and leaves only {chars} characters"
                );
            } else {
                dropped.push((width, height));
            }
        }

        assert_eq!(
            dropped,
            vec![(1080.0, 2400.0)],
            "exactly one screen in SCREENS gives the panel up, and it is the portrait phone"
        );
    }

    /// The title is the one line on this screen that may not be shortened, so the fallback in
    /// [`draw_idle`] has to be reachable: this asserts that the lyric face *would* overrun a narrow
    /// screen, which is what makes the smaller face a real branch rather than dead code.
    ///
    /// It uses the same half-the-point-size estimate [`fit_chars`] does rather than measuring,
    /// because a test cannot open a font. `draw_idle` measures, so this is the weaker claim of the
    /// two on purpose — it says the branch exists, not where exactly it trips.
    #[test]
    fn the_title_outgrows_the_lyric_face_on_a_narrow_screen() {
        let theme = Theme::default();
        // How wide `TITLE` is, in ems, in the widest face `text::CANDIDATES` can turn up (DejaVu
        // Sans, summed from its advance widths). Segoe UI, Arial and Roboto are a few per cent
        // narrower, so this is the pessimistic answer -- which is the one a bound wants. The screen
        // measures for real; this is only what lets a test that cannot open a font say which face
        // each size gets.
        const TITLE_EM: f32 = 8.3;
        let estimate = |height: f32| f32::from(theme.lyric_px(height as u32)) * TITLE_EM;

        let (portrait_w, portrait_h) = (1080.0, 2400.0);
        assert!(
            SCREENS.contains(&(portrait_w, portrait_h)),
            "the portrait screen this is about has gone from SCREENS"
        );
        assert!(
            estimate(portrait_h) > portrait_w - theme.margin_px(portrait_w as u32) * 2.0,
            "{TITLE} fits the lyric face at {portrait_w}x{portrait_h}, so draw_idle never falls back"
        );

        // ...and the televisions this is normally read on have room to spare, so the fallback is
        // not silently the only branch anybody ever sees.
        for (width, height) in SCREENS {
            if (width, height) == (portrait_w, portrait_h) {
                continue;
            }
            assert!(
                estimate(height) <= width - theme.margin_px(width as u32) * 2.0,
                "{TITLE} needs {} of {width}x{height}, so even a television drops to the small face",
                estimate(height)
            );
        }
    }

    /// The line above the summary, for completeness: a two-line notice must not grow into it either.
    /// `NOTICE_MAX_LINES` is what bounds this, and it is the reason that constant exists.
    #[test]
    fn a_standing_notice_never_reaches_the_title_or_the_summary() {
        let theme = Theme::default();
        // The notice is drawn at `0.05h`, stepping by `small_px * 1.25` per line — which is a pixel
        // step, so this is the one bound that has to be checked per screen size rather than as a
        // fraction.
        for (width, height) in SCREENS {
            let small_px = f32::from(theme.small_px(height as u32));
            let last_top = height * 0.05 + small_px * 1.25 * (NOTICE_MAX_LINES - 1) as f32;
            let bottom = last_top + theme.glyph_box(theme.small_size) * height;
            assert!(
                bottom <= height * TITLE_TOP,
                "at {width}x{height} a {NOTICE_MAX_LINES}-line notice runs to {bottom}, \
                 and {TITLE} starts at {}",
                height * TITLE_TOP
            );
        }
    }

    #[test]
    fn the_flash_band_covers_the_top_row_and_stops_well_short_of_the_lyrics() {
        // Two assertions with opposite senses, which is the whole design of this band. It has to
        // *cover* the row at `0.05h` — the playing screen's song title on the left and its badges on
        // the right — because a message that merely sat between them would read as belonging to one
        // of them. And it has to stop a long way above the lyrics, which is what keeps it one line
        // rather than a wrapped block that could grow into the words.
        let theme = Theme::default();
        let (lyric_top, _) = theme.lyric_band();
        for (width, height) in SCREENS {
            let small_px = f32::from(theme.small_px(height as u32));
            let band_bottom = small_px * FLASH_BAND_LINES;
            assert!(
                band_bottom >= height * 0.05,
                "at {width}x{height} the band ends at {band_bottom}, above the title row at {}",
                height * 0.05
            );
            assert!(
                band_bottom < height * lyric_top,
                "at {width}x{height} the band ends at {band_bottom} and the lyrics start at {}",
                height * lyric_top
            );
        }
    }

    #[test]
    fn the_dialled_song_never_reaches_the_number_pad_or_the_connect_panel() {
        let theme = Theme::default();
        let connect =
            ConnectInfo::reachable(vec!["http://192.168.1.5:8177".to_owned()], "0.0.0.0:8177");
        let mut entry = NumberEntry::new();
        entry.push_digit('1');
        entry.set_lookup(
            "1",
            Some(crate::numbers::SongPreview {
                title: "A Title Long Enough To Run Off Both Sides Of Any Screen At All".to_owned(),
                artist: Some("And A Performer With A Very Long Name Indeed".to_owned()),
            }),
        );

        // One pad, checked at every size.
        for (width, height) in SCREENS {
            let keypad = Keypad::idle(width, height, version_reserve(&theme, width, height));
            let mut frame = Frame::idle(&entry);
            frame.connect = Some(&connect);
            frame.keypad = (!keypad.is_empty()).then_some(&keypad);

            let layout = layout_for(&theme, width, height);
            let half = idle_preview_half_width(&frame, &theme, layout);
            assert!(half > 0.0, "{width}x{height}: no room for the title at all");

            let left = width / 2.0 - half;
            let right = width / 2.0 + half;
            let top = height * PREVIEW_TITLE_TOP;
            let bottom = height * PREVIEW_ARTIST_TOP + height * 0.04;

            if let Some((x, y, w, h)) = keypad.bounds() {
                let overlaps = left < x + w && right > x && top < y + h && bottom > y;
                assert!(
                    !overlaps,
                    "{width}x{height}: the title runs into the keypad"
                );
            }
            // Asked of `rect`, not transcribed from it: `idle_preview_half_width` calls the same
            // function, and a `0.55` and a `0.26` written out here asserted a relationship between
            // two copies rather than between the title and the panel.
            let panel = PanelSize::Full.rect(&theme, layout);
            let (panel_w, panel_h) = (panel.w, panel.h);
            let (px, py) = (
                width - layout.margin - panel_w,
                height - layout.margin - panel_h,
            );
            let overlaps = left < px + panel_w && right > px && top < py + panel_h && bottom > py;
            assert!(
                !overlaps,
                "{width}x{height}: the title runs into the connect panel"
            );
        }
    }

    #[test]
    fn the_dialled_song_claims_the_whole_width_when_nothing_is_in_its_way() {
        let theme = Theme::default();
        let entry = NumberEntry::new();
        let frame = Frame::idle(&entry);
        let layout = layout_for(&theme, 1920.0, 1080.0);
        // No keypad and no connect panel: only the screen margin narrows it.
        assert_eq!(
            idle_preview_half_width(&frame, &theme, layout),
            1920.0 / 2.0 - layout.margin
        );
    }

    /// A block too long for its cap ends in an ellipsis rather than stopping mid-sentence.
    ///
    /// The three callers — the standing notice, the keypad message and the connect panel's detail —
    /// share one implementation now, so this is asserted once instead of nowhere.
    #[test]
    fn a_capped_block_marks_the_cut_it_makes() {
        let text = "the quick brown fox jumps over the lazy dog and keeps on going";
        let lines = wrap_capped(text, 12, 2);
        assert_eq!(lines.len(), 2, "capped to two: {lines:?}");
        assert!(
            lines.last().expect("two lines").ends_with('…'),
            "the cut is unmarked: {lines:?}"
        );
        for line in &lines {
            assert!(line.chars().count() <= 12, "too long: {line:?}");
        }

        // And a block that fits is untouched — no ellipsis on a complete sentence.
        let short = wrap_capped("the quick brown fox", 12, 2);
        assert_eq!(short, vec!["the quick".to_owned(), "brown fox".to_owned()]);
    }

    #[test]
    fn wrapping_breaks_on_words_and_keeps_everything() {
        let lines = wrap("the quick brown fox jumps over the lazy dog", 12);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(line.chars().count() <= 12, "too long: {line:?}");
        }
        assert_eq!(
            lines.join(" "),
            "the quick brown fox jumps over the lazy dog"
        );
    }

    #[test]
    fn wrapping_handles_a_word_longer_than_the_limit() {
        let lines = wrap("short verylongwordthatcannotbebroken", 10);
        // The long word gets its own line rather than being lost or looping forever.
        assert!(lines.iter().any(|l| l.contains("verylongword")));
    }

    #[test]
    fn wrapping_empty_text_yields_nothing() {
        assert!(wrap("", 20).is_empty());
        assert!(wrap("   ", 20).is_empty());
    }

    #[test]
    fn an_absurdly_small_limit_still_makes_progress() {
        assert!(!wrap("a b c d e f", 1).is_empty());
    }

    /// The notice's own cut, said rather than silent. Mirrors what `draw_idle` does, since the
    /// drawing itself needs a canvas: over-long input keeps two lines and says it kept two.
    ///
    /// **The fixture is invented rather than real.** A package's own refusal, at the length one
    /// actually is, is not input this notice can see — what it carries is a count and an area — so a
    /// fixture in that shape would test the cut against something that never reaches it. What is
    /// held here is the mechanism, against the case it exists for: a translation of
    /// [`Faults::line`] longer than any locale has yet produced, on a screen narrower than any
    /// television. See `NOTICE_MAX_LINES`.
    #[test]
    fn an_over_long_notice_is_cut_and_says_so() {
        let per_line = 40;
        let notice = "4 problems: packages, sound, pictures, and a fourth area nobody has \
                      thought of yet, at the length a translation might reach";
        let mut lines = wrap(notice, per_line);
        assert!(
            lines.len() > NOTICE_MAX_LINES,
            "the fixture must overflow to test the cut"
        );
        lines.truncate(NOTICE_MAX_LINES);
        let last = lines.last_mut().unwrap();
        *last = ellipsize(&format!("{last}…"), per_line);
        assert!(last.ends_with('…'), "the cut should be visible: {last}");
        assert!(
            last.chars().count() <= per_line,
            "the marked line must still fit: {last}"
        );
    }

    /// ...and a notice that fits is not marked, which is the case that would cry wolf.
    #[test]
    fn a_notice_that_fits_carries_no_ellipsis() {
        let lines = wrap("\"fx.kmpkg\" was not installed: file not found", 40);
        assert!(lines.len() <= NOTICE_MAX_LINES);
        assert!(!lines.last().unwrap().ends_with('…'));
    }

    #[test]
    fn a_title_that_fits_is_left_alone() {
        assert_eq!(ellipsize("Planeta Sonho", 40), "Planeta Sonho");
        // Exactly at the limit is still "fits" — an ellipsis here would lose a character for nothing.
        assert_eq!(ellipsize("abcde", 5), "abcde");
    }

    #[test]
    fn a_long_title_is_cut_and_marked() {
        let cut = ellipsize("Bola de Meia, Bola de Gude (Milton Nascimento)", 20);
        assert!(cut.ends_with('…'), "the cut should be visible: {cut}");
        assert_eq!(
            cut.chars().count(),
            20,
            "the result must not exceed the budget it was given"
        );
    }

    #[test]
    fn cutting_counts_characters_rather_than_bytes() {
        // This is the case that would panic if the implementation sliced by byte: every one of these
        // characters is multi-byte, and the corpus this machine reads is full of them.
        let cut = ellipsize("Não Vou Ficar — Coração Acelerado", 10);
        assert_eq!(cut.chars().count(), 10);
        assert!(cut.starts_with("Não"), "got {cut}");
    }
}
