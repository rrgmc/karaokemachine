//! Colors, sizes and layout.
//!
//! Kept in one place because a karaoke display lives or dies on legibility: the words sit over an
//! arbitrary photograph, are read from across a room, and have to be followed while singing. Every
//! value here is a legibility decision, not decoration.

use sdl3::pixels::Color;

/// How much taller a rendered glyph box is than the nominal point size.
///
/// Ascender plus descender plus the font's own line gap; 1.2 is the usual figure and is close enough
/// for deciding how far down the screen the lyrics reach. Deliberately not measured from the font:
/// this number describes the *area to keep calm*, and rounding it up costs nothing while rounding it
/// down would put a busy horizon under a descender.
const GLYPH_BOX: f32 = 1.2;

/// How thick a ring a face carries, as a fraction of its own size.
///
/// Three pixels around the lyric face on a 1080-line screen, which is the ring the words over a
/// photograph are read through and the one the rest follow from. `lyric_size` puts that face at 92
/// pixels there, so the lyrics keep the ring they have and every smaller face carries the same
/// share of its own size rather than the same count of pixels.
const OUTLINE_RATIO: f32 = 3.0 / (0.085 * 1080.0);

/// A color at a fraction of its opacity.
///
/// **The alpha it already carries is scaled rather than replaced**, so a color that is translucent
/// on purpose composes: `panel` at `0xC8` and the position bar's `0x30` track both come back a
/// fraction of what they were rather than jumping to the fraction itself. An `alpha` of 1.0 hands the
/// color back untouched, which is what lets a caller pass the factor unconditionally.
///
/// Here rather than at a draw site because both the text styles and the bar's two fills want it, and
/// SDL's `Color` has nowhere for it to live.
#[must_use]
pub fn fade(color: Color, alpha: f32) -> Color {
    let alpha = alpha.clamp(0.0, 1.0);
    Color::RGBA(
        color.r,
        color.g,
        color.b,
        (f32::from(color.a) * alpha).round() as u8,
    )
}

/// Layout and color for the whole display.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Lyric text not yet sung.
    pub lyric_pending: Color,
    /// Lyric text already sung — the wipe color.
    pub lyric_sung: Color,
    /// The line that is not currently being sung, shown dimmer so the eye knows where it is.
    pub lyric_upcoming: Color,
    /// Outline drawn around lyric glyphs.
    ///
    /// Text over a photograph needs an outline, not just a shadow: a bright patch behind light text
    /// makes it vanish, and a drop shadow only helps on one side.
    pub lyric_outline: Color,
    /// Panel backgrounds.
    pub panel: Color,
    /// Ordinary text on a panel.
    pub text: Color,
    /// De-emphasised text.
    pub text_dim: Color,
    /// Highlight for the current queue entry and for accents.
    pub accent: Color,
    /// A second accent, and the one color here that no screen draws.
    ///
    /// It exists because this palette is where the *application icons* get their color too. Each of
    /// the three programs' tiles leads with one hue, and paints the **M** of `KM` in it, so three
    /// programs sitting on one desktop can be told apart in a taskbar (see `examples/icon.rs`). The
    /// machine has the sung-lyric amber and the package builder has [`accent`](Self::accent); the
    /// offline remote is the third, and there was no third hue.
    ///
    /// It is here rather than as a literal in that example for the same reason the other two are not
    /// literals: a color this project uses belongs to the palette, so that a theme change reaches
    /// the icons instead of leaving them looking like the last one. Green because it is the furthest
    /// from both of the others at 16 pixels, which is the size where telling them apart matters —
    /// and 16 is the size at which the palette is doing the telling *on its own*, because two
    /// letters in a twelve-pixel plate are a smear.
    pub accent_alt: Color,
    /// The darkest color in the application icons' ground.
    ///
    /// A second dark color, and it is here because the icons stopped being able to use the first.
    /// The ground behind the mark used to start from [`background`](Self::background), and the two
    /// shared a value for as long as nobody asked them for different things. They are asked for
    /// different things. A *television* ground is a fallback behind a photograph, and every point of
    /// luminance it spends is spent against the lyrics on top of it — so it is right for it to be
    /// near-black. A *launcher* ground is a two-centimeter tile with nothing over it at all, and
    /// near-black there reads as a hole in the dock rather than as a surface. `background` keeps its
    /// own argument untouched; this one answers the other question.
    ///
    /// Deep violet, and it is one end of a range rather than a flat fill: a ground with two ends is
    /// what stops a tile reading as a swatch. [`icon_glow`](Self::icon_glow) is the other end.
    ///
    /// **The icon does not use this value neat any more, and the reason is worth keeping.** The
    /// tile is three bands now with a near-black plate over them, and straight `icon_ground` behind
    /// that plate measured **1.2:1** — not a dark corner but a missing one, with the plate's edge
    /// simply not there. The darkest band is this color lifted partway toward `icon_glow`, which
    /// reads about 2.2:1 where the two meet. Both ends of the range are still exactly these two
    /// fields, so a change here still reaches the icons.
    pub icon_ground: Color,
    /// The brightest color in that same ground — where the light in an icon comes from.
    ///
    /// Magenta, which is the color a room is lit in when somebody is singing in it, and this is the
    /// one place in the product where saying so costs nothing: an icon has no words over it to keep
    /// legible.
    ///
    /// Here rather than as a `const` in `examples/icon.rs` for exactly the reason
    /// [`accent_alt`](Self::accent_alt) is here — a color this project uses belongs to the palette,
    /// so a theme change reaches the icons instead of leaving them looking like the last one.
    ///
    /// **This has a floor rather than a ceiling.** A mark standing directly on the ground makes the
    /// ground's luminance a contrast budget spent three times over, capped by the palest mark —
    /// which is what forces [`accent`](Self::accent) up from `#4FC3F7` once the ground is lifted off
    /// near-black. The mark stands on a plate, so what constrains the ground is how dark it may be
    /// before the plate stops being an object. `src/icon.rs` holds both ends — 4.5:1 for the letters
    /// against the plate, 1.8:1 for the plate against every band. Neither `accent` nor `accent_alt`
    /// needs to move if this changes; they are the *bright* end of a band rather than something
    /// standing on one.
    pub icon_glow: Color,
    /// Warnings and errors.
    pub alert: Color,
    /// Fallback background when there is no wallpaper.
    ///
    /// **It is also the application icons' plate**, the near-black square the monogram stands on,
    /// and that is not the two roles being confused again. This color answers "how dark is a dark
    /// thing here"; a television ground and a plate are both dark *things*. What the icons could not
    /// use it for is a *ground* — see [`icon_ground`](Self::icon_ground), which exists because of
    /// exactly that and is still a separate value.
    pub background: Color,

    /// Lyric height as a fraction of screen height.
    pub lyric_size: f32,
    /// Top edge of the first lyric row, as a fraction of screen height.
    ///
    /// Here rather than as a literal in `draw`, because it is half of the answer to *where on the
    /// screen must a wallpaper stay calm* — see [`Theme::lyric_band`].
    pub lyric_row_top: f32,
    /// Distance between lyric row tops, as a fraction of screen height.
    pub lyric_row_height: f32,
    /// Narrower lyric faces, as fractions of [`Theme::lyric_size`], for a line too wide to fit.
    ///
    /// A lyric cannot be shortened and it cannot wrap -- the row below it holds the *upcoming*
    /// line, and `lyric_row_height` leaves only about a third of a line's slack over `lyric_size`
    /// -- so the size is what gives way, exactly as it does for the product's name on the idle
    /// screen. These are a ladder rather than a computed size because a face has to be *loaded*
    /// to measure with, and loading one per frame is not something a draw can do.
    ///
    /// Deliberately not `text_size`, which is what the idle title falls back to: at 0.038 against
    /// 0.085 that is a 55% drop, so a line one character too wide would come out at not much over
    /// half height. The steps here are gentle enough that the usual answer is the first of them.
    pub lyric_fallbacks: [f32; 3],
    /// Ordinary text height as a fraction of screen height.
    pub text_size: f32,
    /// Small text height as a fraction of screen height.
    pub small_size: f32,
    /// How thick a ring a face carries, as a fraction of that face's own size.
    ///
    /// **A fraction of the face and not of the screen**, because a ring is part of a glyph. One
    /// pixel count for every face on screen is a ring sized for the largest of them: three pixels
    /// around a 92-pixel lyric is a thirtieth of it and around a 28-pixel row is a tenth, which
    /// fills the counters of `a`, `e` and `o` and leaves the row reading heavier than the title
    /// above it. A fraction gives every row the same weight of ring.
    pub outline: f32,
    /// Margin as a fraction of screen width.
    pub margin: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Near-white rather than pure white: pure white over a bright wallpaper glares, and the
            // sung color needs somewhere brighter to go.
            lyric_pending: Color::RGB(0xEC, 0xEF, 0xF4),
            // Warm amber for sung text. High contrast against the pending color in both hue and
            // brightness, so the boundary is obvious even to a color-blind viewer.
            lyric_sung: Color::RGB(0xFF, 0xC1, 0x07),
            lyric_upcoming: Color::RGB(0xA6, 0xAC, 0xBA),
            lyric_outline: Color::RGB(0x08, 0x0A, 0x10),
            // Translucent, so the wallpaper still reads through a panel.
            panel: Color::RGBA(0x0C, 0x0F, 0x18, 0xC8),
            text: Color::RGB(0xEC, 0xEF, 0xF4),
            text_dim: Color::RGB(0x8A, 0x92, 0xA4),
            // Both accents are a notch brighter than a plain `#4FC3F7` and `#5FD68F`, and the reason
            // is the screen's own: these are its accents, and they read well there. The icon does
            // not need the extra brightness — the mark stands on a near-black plate, where this
            // blue measures 10.6:1 against the 4.5:1 floor `src/icon.rs` holds — so a change to
            // either is the screen's argument to have rather than the icon's. The green matches
            // the blue so the three stay a set rather than two bright and one flat.
            accent: Color::RGB(0x5F, 0xD3, 0xFF),
            accent_alt: Color::RGB(0x57, 0xE7, 0x9A),
            // The two ends of the icons' ground: see the fields. Every band of a tile is mixed
            // between these, and the darkest of them has to stay clear of `background` — which is
            // the plate — or the plate stops being an object.
            icon_ground: Color::RGB(0x2A, 0x0E, 0x42),
            icon_glow: Color::RGB(0xC4, 0x3A, 0x8E),
            alert: Color::RGB(0xFF, 0x6E, 0x6E),
            background: Color::RGB(0x0A, 0x0C, 0x14),

            lyric_size: 0.085,
            lyric_fallbacks: [0.80, 0.65, 0.50],
            lyric_row_top: 0.42,
            lyric_row_height: 0.13,
            text_size: 0.038,
            small_size: 0.026,
            outline: OUTLINE_RATIO,
            margin: 0.05,
        }
    }
}

impl Theme {
    /// The vertical span the lyrics occupy, as fractions of screen height.
    ///
    /// **This is what a wallpaper has to stay calm behind**, and it is the reason the number lives
    /// here rather than being read off `draw`: `tools/cmd/assets/km-wallpaper-pack` measures contrast and busyness
    /// inside exactly this band, and a picture chosen against the wrong band is a picture chosen
    /// against nothing. `config.example.toml` there quotes these values and
    /// `the_lyric_band_is_what_the_wallpaper_tool_measures` below is what stops the two drifting.
    ///
    /// The rows are drawn from their top edge, and a glyph box is a little taller than the nominal
    /// font size, so the band runs from the first row's top to the last row's top plus one line.
    pub fn lyric_band(&self) -> (f32, f32) {
        let rows = crate::lyrics::ROWS.max(1) as f32;
        let top = self.lyric_row_top;
        let bottom = top + (rows - 1.0) * self.lyric_row_height + self.lyric_size * GLYPH_BOX;
        (top, bottom)
    }

    /// How far down the screen a line of this nominal size actually reaches, as a fraction of height.
    ///
    /// Text is drawn from its top edge, so a line at `y` occupies `y ..= y + glyph_box(size)`. This
    /// is what a caller asks when it needs to know whether two lines collide — [`Self::lyric_band`]
    /// above is the same arithmetic for the lyric rows, and `draw.rs` uses this one to prove the idle
    /// screen's four stacked lines stay clear of each other at every size.
    pub fn glyph_box(&self, size: f32) -> f32 {
        size * GLYPH_BOX
    }

    /// The same span in pixels, at the size the font is actually loaded at.
    ///
    /// **Not `glyph_box(size) * height`**, and the difference is the clamp in [`Self::px`]: the face
    /// is opened at a rounded, floored pixel size, so on a small window the text is *taller* than
    /// the fraction says and a box sized from the fraction would be shorter than the words in it.
    /// The two agree from about 385 pixels of height upward, which is every screen anybody watches
    /// this on and not every window somebody can drag.
    ///
    /// [`Card`](crate::draw) sizes the connect panel with this, because that panel's height is the
    /// sum of its rows and a row that lied by a pixel is the whole bug it was written for. A caller
    /// comparing two *fractions* of the screen — the idle screen's stacked lines, the lyric band —
    /// wants [`Self::glyph_box`] instead, and is right to.
    pub fn glyph_box_px(&self, size: f32, screen_height: u32) -> f32 {
        f32::from(self.px(size, screen_height)) * GLYPH_BOX
    }

    /// Lyric font size in pixels for a screen of this height.
    pub fn lyric_px(&self, screen_height: u32) -> u16 {
        self.px(self.lyric_size, screen_height)
    }

    /// The narrower lyric sizes in pixels, largest first.
    ///
    /// Paired with [`Theme::lyric_px`] by [`crate::text::Fonts`], which loads all four and picks
    /// between them per line. A fraction that rounds to the same pixel size as the one above it
    /// still gets its own face; the clamp in [`Theme::px`] makes that possible on a tiny window,
    /// and four identical faces cost a little memory and never a wrong answer.
    pub fn lyric_fallback_px(&self, screen_height: u32) -> [u16; 3] {
        self.lyric_fallbacks
            .map(|fraction| self.px(self.lyric_size * fraction, screen_height))
    }

    /// Ordinary text size in pixels.
    pub fn text_px(&self, screen_height: u32) -> u16 {
        self.px(self.text_size, screen_height)
    }

    /// Small text size in pixels.
    pub fn small_px(&self, screen_height: u32) -> u16 {
        self.px(self.small_size, screen_height)
    }

    /// How thick a ring a face of this size carries, in pixels.
    ///
    /// Resolved against the face rather than against the screen, so a row's ring is the same share
    /// of its own glyphs whatever else is on screen beside it.
    pub fn outline_for(&self, face_px: f32) -> i32 {
        // Always at least one pixel: an outline that rounds to zero leaves text unreadable on a
        // small window, which is where it is needed most.
        ((face_px * self.outline).round() as i32).max(1)
    }

    /// Margin in pixels for a screen of this width.
    pub fn margin_px(&self, screen_width: u32) -> f32 {
        (screen_width as f32 * self.margin).round()
    }

    fn px(&self, fraction: f32, screen_height: u32) -> u16 {
        let size = (screen_height as f32 * fraction).round();
        // Clamped so a tiny window still gets a loadable font size rather than zero.
        size.clamp(10.0, 400.0) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The band `tools/cmd/assets/km-wallpaper-pack` measures, and the one thing that keeps the two in step.
    ///
    /// `config.example.toml` there cannot import this crate — that would drag SDL3-from-source into a
    /// build-time asset tool — so it quotes these numbers instead, and this test is what turns moving
    /// a lyric row into a failure here rather than into a pack of pictures chosen against a band the
    /// app no longer uses. **Change one and change the other.**
    #[test]
    fn the_lyric_band_is_what_the_wallpaper_tool_measures() {
        let theme = Theme::default();
        let (top, bottom) = theme.lyric_band();

        // The band the pack measures, from its `config.example.toml`. Wider than this one on purpose:
        // it is a margin, not a disagreement, and asserting containment rather than equality is what
        // lets the margin exist while still failing if a row moves out from under it.
        const PACK_TOP: f32 = 0.40;
        const PACK_BOTTOM: f32 = 0.68;
        assert!(
            PACK_TOP <= top && bottom <= PACK_BOTTOM,
            "the lyrics ({top:.3}..{bottom:.3}) have moved outside the band \
             tools/cmd/assets/km-wallpaper-pack/config.example.toml measures ({PACK_TOP}..{PACK_BOTTOM}). \
             Change one and change the other."
        );
        // And the margin has not grown so wide that the pack is judging most of the picture.
        assert!(
            top - PACK_TOP < 0.08 && PACK_BOTTOM - bottom < 0.08,
            "the pack's band is now much larger than the lyrics: {top:.3}..{bottom:.3}"
        );

        // Whatever `ROWS` becomes, every row that is drawn is inside the band.
        let last_row_top =
            theme.lyric_row_top + (crate::lyrics::ROWS as f32 - 1.0) * theme.lyric_row_height;
        assert!(bottom > last_row_top, "the last row starts inside the band");
    }

    #[test]
    fn sizes_scale_with_the_screen() {
        let theme = Theme::default();
        let small = theme.lyric_px(720);
        let large = theme.lyric_px(2160);
        assert!(large > small, "lyrics must grow with the screen");
        // 8.5% of 1080 is about 92 pixels.
        assert!((85..=100).contains(&theme.lyric_px(1080)));
    }

    #[test]
    fn a_tiny_window_still_gets_a_usable_font_size() {
        let theme = Theme::default();
        assert!(theme.lyric_px(1) >= 10);
        assert!(theme.text_px(1) >= 10);
        assert!(theme.small_px(1) >= 10);
    }

    #[test]
    fn an_absurdly_large_screen_does_not_ask_for_an_absurd_font() {
        let theme = Theme::default();
        assert!(theme.lyric_px(100_000) <= 400);
    }

    #[test]
    fn the_outline_never_rounds_away_to_nothing() {
        let theme = Theme::default();
        assert!(theme.outline_for(f32::from(theme.lyric_px(1080))) >= 1);
        assert!(
            theme.outline_for(f32::from(theme.small_px(240))) >= 1,
            "a small window needs the outline most"
        );
        assert!(
            theme.outline_for(f32::from(theme.lyric_px(2160)))
                > theme.outline_for(f32::from(theme.lyric_px(1080)))
        );
    }

    /// Every face carries the same share of its own size, and the lyrics keep the ring they have.
    #[test]
    fn a_ring_is_a_fraction_of_the_face_it_surrounds() {
        let theme = Theme::default();
        assert_eq!(
            theme.outline_for(f32::from(theme.lyric_px(1080))),
            3,
            "the lyric face is what the ratio is anchored on"
        );
        assert!(
            theme.outline_for(f32::from(theme.small_px(1080)))
                < theme.outline_for(f32::from(theme.lyric_px(1080))),
            "a row a third the height of a lyric cannot carry the same ring"
        );
        // A television is where the faces are largest, and the ring grows with them rather than
        // thinning away to a hairline.
        assert_eq!(theme.outline_for(f32::from(theme.lyric_px(2160))), 6);
    }

    #[test]
    fn lyrics_are_larger_than_ordinary_text() {
        let theme = Theme::default();
        assert!(theme.lyric_px(1080) > theme.text_px(1080));
        assert!(theme.text_px(1080) > theme.small_px(1080));
    }

    #[test]
    fn sung_and_pending_lyric_colors_differ_in_brightness_not_only_hue() {
        // A color-blind viewer has to see the wipe boundary too, so the two colors must differ in
        // luminance as well as hue.
        let theme = Theme::default();
        let luminance =
            |c: Color| 0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b);
        let difference = (luminance(theme.lyric_sung) - luminance(theme.lyric_pending)).abs();
        assert!(
            difference > 15.0,
            "sung and pending differ by only {difference} in luminance"
        );
    }

    /// The icons' ground is not the television's, and is the brighter of the two.
    ///
    /// These were one color once, and the obvious tidy-up is to make them one again. This is the
    /// test that answers that: they look like a duplication and they are two answers to two
    /// questions. A television's ground sits behind a photograph with lyrics over it, and every
    /// point of luminance it spends is spent against the words; an icon's ground is a small tile
    /// with nothing over it, where the same near-black reads as a hole in the dock.
    ///
    /// **Whether the ground is bright *enough* is not decided here**, deliberately. That question is
    /// about the rendered pixels — the bands, the vignette and the letters' own gradients all sit
    /// between these two colors and what an icon actually shows — so it is measured against the
    /// generated PNGs in `src/icon.rs`, which is where both floors live.
    #[test]
    fn the_icons_ground_is_not_the_televisions_background() {
        let theme = Theme::default();
        let luminance =
            |c: Color| 0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b);
        assert_ne!(
            (
                theme.icon_ground.r,
                theme.icon_ground.g,
                theme.icon_ground.b
            ),
            (theme.background.r, theme.background.g, theme.background.b),
            "the icons' ground and the television's have been made one color again"
        );
        assert!(
            luminance(theme.icon_ground) > luminance(theme.background),
            "the icons' ground is meant to be the brighter of the two"
        );
    }

    #[test]
    fn the_margin_scales_with_width() {
        let theme = Theme::default();
        assert_eq!(theme.margin_px(1000), 50.0);
        assert!(theme.margin_px(3840) > theme.margin_px(1920));
    }
}
