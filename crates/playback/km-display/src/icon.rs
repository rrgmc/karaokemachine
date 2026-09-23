//! The window icon.
//!
//! What a desktop shows for the app in a taskbar, a dock, an alt-tab switcher and the corner of a
//! title bar. It is a separate thing from the icon an *installer* puts on disk — the Windows `.ico`
//! inside the executable and the `.desktop` file's `Icon=` line — because those two are read by the
//! shell before the process exists, and neither of them helps a build that was copied into a folder
//! and run from there, which is exactly how the portable Windows build is meant to be used.

use sdl3::pixels::PixelFormat;
use sdl3::surface::Surface;
use sdl3::video::Window;

/// The icon, compiled in.
///
/// Embedded rather than loaded from `assets/`, under the `Bundling assets` rule in
/// `docs/decisions/distribution.md`: a file that has to travel beside an executable is a file a
/// copy can leave behind. 40 KB against a 12 MB binary, and
/// the alternative failure — a window with a default icon and nothing in the log — is the kind
/// nobody ever gets round to diagnosing.
///
/// 256 pixels because that is the largest size any of these platforms asks for; SDL hands the
/// surface to the window system, which scales it down to whatever it actually needs.
const ICON_PNG: &[u8] = include_bytes!("../../../../icon/icon-256.png");

/// Gives a window the application icon.
///
/// Reported rather than fatal, like `apply_fullscreen`: an app with the wrong icon is a blemish, and
/// an app that refuses to open a window over one is a bug.
pub fn set_window_icon(window: &mut Window) {
    let decoded = match image::load_from_memory(ICON_PNG) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            tracing::warn!(%error, "the compiled-in window icon would not decode");
            return;
        }
    };
    let (width, height) = decoded.dimensions();
    // `RGBA32` is the format whose *memory* order is R, G, B, A whatever the machine's endianness,
    // which is what `image` produces. Naming a channel order instead (`ABGR8888`) would be right on
    // this machine and silently wrong on a big-endian one.
    let format = match PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_RGBA32) {
        Ok(format) => format,
        Err(error) => {
            tracing::warn!(%error, "no RGBA pixel format; leaving the default window icon");
            return;
        }
    };

    let mut pixels = decoded.into_raw();
    let pitch = width * 4;
    match Surface::from_data(&mut pixels, width, height, pitch, format) {
        // SDL copies the pixels into the window, so the surface and the buffer behind it can go out
        // of scope here. Nothing has to be kept alive for the lifetime of the window.
        Ok(surface) => {
            if !window.set_icon(&surface) {
                tracing::warn!("the window system would not take an icon");
            }
        }
        Err(error) => tracing::warn!(%error, "could not wrap the window icon for SDL"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Where the 256-pixel renderings are sampled.
    //
    // Fixed points rather than a search, because the drawing is deterministic and its geometry is
    // written down in `examples/icon.rs`: a test that hunted for "a light pixel somewhere" would
    // pass on a rendering that had gone wrong in every way except brightness. The plate is 0.62 of
    // the canvas and centered, so at this size it covers 49 to 207 — which is what puts the three
    // band samples outside it and the first two inside.
    //
    // The two letter samples also carry `monogram::SCALE`, which sets the mark at 0.85 of its own
    // drawing about the middle of the plate: the K's upright is at 0.152 of the letters' own space,
    // so 0.5 + (0.152 - 0.5) × 0.85 = 0.204 of the plate, which is x = 81 here. Moving that number
    // moves these two, and the failure is loud — the sample lands on bare plate and the contrast
    // assertion reports 1.00:1.

    /// Inside the K's upright, halfway down the letters. Near-white in all four.
    const LETTER_K: (u32, u32) = (81, 128);
    /// Inside the M's left upright, on the same row. This one takes the program's own hue.
    const LETTER_M: (u32, u32) = (136, 128);
    /// Bare plate, above the letters and clear of them.
    const PLATE: (u32, u32) = (70, 70);
    /// The deep purple band, just above the plate's top edge.
    const BAND_DARK: (u32, u32) = (100, 40);
    /// The magenta band, just left of the plate.
    const BAND_MID: (u32, u32) = (40, 180);
    /// The band that leads with this program's own hue, past the plate's bottom-right corner.
    const BAND_LEAD: (u32, u32) = (215, 215);

    fn decode(png: &[u8], who: &str) -> image::RgbaImage {
        let decoded = image::load_from_memory(png)
            .unwrap_or_else(|error| panic!("{who}: the icon must be a decodable image: {error}"))
            .to_rgba8();
        assert_eq!(
            decoded.dimensions(),
            (256, 256),
            "{who}: these tests sample the 256-pixel rendering"
        );
        decoded
    }

    fn at(image: &image::RgbaImage, (x, y): (u32, u32)) -> image::Rgba<u8> {
        *image.get_pixel(x, y)
    }

    /// `include_bytes!` makes a *missing* icon a build failure, which is most of the guarantee. What
    /// it cannot catch is the icon being regenerated into something wrong — an empty file, or a
    /// rendering that came out blank — and a window with a transparent icon looks like a platform
    /// quirk rather than like a broken asset.
    #[test]
    fn the_window_icon_is_embedded_and_looks_like_the_icon() {
        let decoded = decode(ICON_PNG, "machine");

        // The K, which is the theme's near-white on a near-black plate. A blank or transparent
        // rendering fails both of these.
        let letter = at(&decoded, LETTER_K);
        assert_eq!(letter[3], 255, "the letters must be opaque");
        assert!(
            letter[0] > 200 && letter[1] > 200 && letter[2] > 200,
            "this should be a stroke of the K, not {letter:?}"
        );

        // And a corner, which is outside the rounded tile and so must be fully transparent. Together
        // with the above this pins the tile's shape as well as its color.
        assert_eq!(
            at(&decoded, (0, 0))[3],
            0,
            "the tile's corners must be transparent, not square"
        );
    }

    /// The four 256-pixel icons.
    ///
    /// Named once rather than repeated inside each test, because every one of them has to say the
    /// same files: the whole point is that these are one drawing, and a test that checked three of
    /// them would be a test that let the fourth drift.
    const ALL_MARKS: &[(&str, &[u8])] = &[
        ("machine", include_bytes!("../../../../icon/icon-256.png")),
        (
            "package builder",
            include_bytes!("../../../../icon/km-package-builder-256.png"),
        ),
        (
            "offline remote",
            include_bytes!("../../../../icon/km-remote-256.png"),
        ),
        (
            "assets",
            include_bytes!("../../../../icon/km-admin-256.png"),
        ),
    ];

    /// Each program's hue reaches both places it is meant to: the widest band, and the **M**.
    ///
    /// **Where the identity lives is the thing worth pinning.** A colored glyph on a shared ground
    /// lets a test sample the mark and read off which program it is. Here the K is the same
    /// near-white in all four, so a test that sampled *it* would pass on four identical icons —
    /// exactly the defect these programs exist not to have, being run side by side in one taskbar.
    ///
    /// The M is colored because the site's wordmark colors it: `Karaoke` in the text color and
    /// `Machine` in the amber (`site/style.css`, `.hero h1 span`). The machine's icon is therefore
    /// that wordmark exactly, and the other three are the same idea in their own hue.
    ///
    /// Asserted from here rather than from each program's own crate for the reason the machine's
    /// already was: the alternative is a PNG decoder in `km-package-builder`'s dev-dependencies to
    /// read one pixel. That crate asserts the cheap half, that its favicon is not byte-identical to
    /// the machine's, and so does `km-remote-pages`.
    #[test]
    fn each_programs_hue_reaches_its_band_and_its_m() {
        // Amber is red-dominant and far more red than blue; the blue is the mirror of it; green
        // needs *both* neighbors excluded where blue needed one, because the amber is strong in
        // green too, so `> red` is what says which of those two it is; and the magenta is
        // red-dominant like the amber, so what tells those two apart is the blue -- the one channel
        // the amber has almost none of.
        //
        // **Widened to `i32` before any of them is asked.** These read `c[0] > c[2] + 100`, and on
        // `u8` that addition overflows the moment a channel is over 155 -- which never happened
        // while each predicate only ever saw its own mark, and happens immediately once every mark
        // is put to every hue below. A panic rather than a wrong answer, so it was found at once;
        // the point is that the arithmetic was always this fragile and the old shape hid it.
        type Test = fn([i32; 3]) -> bool;
        let expected: [(Test, &str); 4] = [
            (|c| c[0] > 140 && c[0] > c[2] + 100, "the sung-lyric amber"),
            (|c| c[2] > 140 && c[2] > c[0] + 80, "the accent blue"),
            (
                |c| c[1] > 140 && c[1] > c[0] + 60 && c[1] > c[2] + 40,
                "the second accent green",
            ),
            (
                |c| c[0] > 140 && c[2] > 100 && c[2] > c[1] + 40,
                "the icon-glow magenta",
            ),
        ];

        // **Zipped against `ALL_MARKS` rather than naming the marks a second time, and the two
        // lengths asserted before anything is sampled.** This test listed its own three and let the
        // fourth drift for two releases: `km-admin` was in `ALL_MARKS` from the day it arrived and
        // is sampled by every other test in this module, and the one thing never asserted was the
        // thing its icon exists for -- that its lead is the magenta and not somebody else's hue.
        // `zip` stops at the shorter side and says nothing, which is exactly how that stayed
        // invisible, so the count is the guard: a fifth mark is a failing test rather than a quiet
        // four-fifths. It is the same argument `ALL_MARKS`'s own comment already makes.
        assert_eq!(
            ALL_MARKS.len(),
            expected.len(),
            "a mark was added without the hue that says which program it is"
        );

        // **Every mark is put to every hue, not only to its own.** "A test can sample the mark and
        // read off which program it is" is a claim about telling them apart, and one that only ever
        // asked `is this mine?` cannot see two predicates that both say yes -- which is the way this
        // set actually degrades, the amber and the magenta being the two red-dominant ones. So each
        // sample must match its own and fail the other three, and the predicates police each other.
        for (index, (who, png)) in ALL_MARKS.iter().copied().enumerate() {
            let decoded = decode(png, who);
            for (point, what) in [(BAND_LEAD, "lead band"), (LETTER_M, "M")] {
                let sample = at(&decoded, point);
                assert_eq!(sample[3], 255, "{who}: the {what} must be opaque");
                let channels = [
                    i32::from(sample[0]),
                    i32::from(sample[1]),
                    i32::from(sample[2]),
                ];

                for (other, (is_the_hue, name)) in expected.iter().copied().enumerate() {
                    let its_own = other == index;
                    assert_eq!(
                        is_the_hue(channels),
                        its_own,
                        "{who}: the {what} {} {name}, and it is {sample:?}",
                        if its_own {
                            "should be"
                        } else {
                            "should not be"
                        }
                    );
                }
            }
        }

        // And the remote is not the machine's icon under another name. Nothing else here would
        // notice: a remote wearing the machine's icon looks like a remote.
        assert_ne!(
            ALL_MARKS[2].1, ALL_MARKS[0].1,
            "the offline remote is serving the machine's icon again"
        );
    }

    /// The hue reaches those two places and **nowhere else**.
    ///
    /// **This is what is left of "the ground is deliberately shared and not tinted per mark", and
    /// keeping the surviving half is the point.** That rule was whole while the mark carried the
    /// difference; it had to give when the K went white, because four white marks on four
    /// identical grounds are four identical icons. What did *not* have to give is that these are
    /// one product: the deep purple, the magenta, the plate and the K are byte-for-byte the same in
    /// all four, so nobody seeing two of them doubts they belong together.
    ///
    /// A future "give the builder its own purple as well" would be a second design arriving without
    /// a decision, and this is what stops it.
    #[test]
    fn the_hue_reaches_two_places_and_nowhere_else() {
        let machine = decode(ALL_MARKS[0].1, "machine");

        for (who, png) in &ALL_MARKS[1..] {
            let decoded = decode(png, who);

            for (point, what) in [
                (BAND_DARK, "deep purple band"),
                (BAND_MID, "magenta band"),
                (PLATE, "plate"),
                (LETTER_K, "K"),
            ] {
                assert_eq!(
                    at(&decoded, point),
                    at(&machine, point),
                    "{who}: the {what} is meant to be the same in all four"
                );
            }

            for (point, what) in [(BAND_LEAD, "lead band"), (LETTER_M, "M")] {
                assert_ne!(
                    at(&decoded, point),
                    at(&machine, point),
                    "{who}: the {what} is one of the two things that tell the icons apart, and it \
                     matches the machine's"
                );
            }
        }
    }

    /// The machine's mark as the streaming launcher wears it.
    ///
    /// **Deliberately not in [`ALL_MARKS`]**, which is four programs and asserts its own length so a
    /// fifth is a failing test. This is not a fifth program: it is the machine, and it leads with
    /// the machine's amber. What tells it apart is the badge, which is a different axis and gets the
    /// test below rather than a hue predicate it could never satisfy.
    const STREAM_MARK: &[u8] = include_bytes!("../../../../icon/karaokemachine-stream-256.png");

    /// Inside the outer wave of the badge, on the diagonal out from its source.
    const BADGE_WAVE: (u32, u32) = (186, 179);
    /// The source the waves leave, at the badge's lower left.
    const BADGE_SOURCE: (u32, u32) = (172, 193);
    /// Everything the badge can reach, as (left, top, right, bottom) inclusive.
    ///
    /// The drawing's own numbers with a few pixels of margin: `badge::SOURCE` is at (0.780, 0.912)
    /// of a plate covering 49 to 207 here, and the widest wave reaches `0.122 + 0.019` from it. The
    /// margin is there so an antialiased edge cannot fail this by a pixel; what the box is for is
    /// the assertion around it, that *outside* it nothing moved at all.
    const BADGE_BOX: (u32, u32, u32, u32) = (164, 167, 200, 202);

    /// The stream mark is the machine's mark and a badge, and the badge is all of the difference.
    ///
    /// **Two claims, and the second is the one worth a whole-image comparison.** That the badge is
    /// there is easy to check and easy to keep true. That nothing *else* moved is what stops the
    /// streaming launcher's icon quietly becoming a second design — a shifted letter, a retuned
    /// band, a plate a shade darker — which is the same thing
    /// `the_hue_reaches_two_places_and_nowhere_else` holds for the four programs and for the same
    /// reason: somebody seeing both of these has to read them as one machine started two ways.
    #[test]
    fn the_stream_mark_is_the_machines_mark_and_a_badge() {
        let stream = decode(STREAM_MARK, "stream");
        let machine = decode(ALL_MARKS[0].1, "machine");

        // The badge is the machine's own amber, on the plate, opaque. Sampled at the source and on a
        // wave, because those are two shapes and a drawing that lost one of them still has the
        // other.
        for (point, what) in [(BADGE_SOURCE, "source"), (BADGE_WAVE, "wave")] {
            let sample = at(&stream, point);
            assert_eq!(sample[3], 255, "the badge's {what} must be opaque");
            assert!(
                sample[0] > 140 && i32::from(sample[0]) > i32::from(sample[2]) + 100,
                "the badge's {what} should be the sung-lyric amber the M is, and it is {sample:?}"
            );
            assert_ne!(
                sample,
                at(&machine, point),
                "the badge's {what} is bare plate, exactly as it is on the machine's own mark"
            );
        }

        // And it reads against what it stands on, on the letters' floor rather than the plate's:
        // this is a mark somebody has to recognize, not an edge they have to see.
        let plate = at(&stream, PLATE);
        let ratio = contrast(&at(&stream, BADGE_WAVE), &plate);
        assert!(
            ratio >= 4.5,
            "the badge is only {ratio:.2}:1 against the plate under it; the floor is 4.5:1"
        );

        // Everything outside the badge's own corner, pixel for pixel.
        let (left, top, right, bottom) = BADGE_BOX;
        for (x, y, pixel) in stream.enumerate_pixels() {
            if (left..=right).contains(&x) && (top..=bottom).contains(&y) {
                continue;
            }
            assert_eq!(
                pixel,
                machine.get_pixel(x, y),
                "the stream mark differs from the machine's at ({x}, {y}), which the badge does \
                 not reach: this mark is that one with a badge on it and nothing else"
            );
        }
    }

    /// The simple package builder's mark: the package builder's, with a bolt.
    ///
    /// **Not in [`ALL_MARKS`]**, for the stream mark's reason. It leads with the builder's blue,
    /// because the theme has no hue left, and the bolt is what tells the two apart.
    const SIMPLE_MARK: &[u8] = include_bytes!("../../../../icon/km-package-simple-256.png");

    /// On the bolt's middle stroke, which joins its two slanted ones.
    const BOLT_MIDDLE: (u32, u32) = (179, 183);
    /// Everything the bolt can reach, as (left, top, right, bottom) inclusive.
    ///
    /// The drawing's own numbers with a few pixels of margin: the bolt's corners run from
    /// (0.775, 0.740) to (0.870, 0.945) of a plate covering 49 to 207 here, and each stroke is
    /// `0.030` thick on either side.
    const BOLT_BOX: (u32, u32, u32, u32) = (162, 157, 196, 207);

    /// The simple mark is the builder's mark and a bolt, and the bolt is all of the difference.
    ///
    /// Somebody who sees both on one desktop has to read them as two package tools of one family,
    /// so nothing but the bolt may move.
    #[test]
    fn the_simple_mark_is_the_builders_mark_and_a_bolt() {
        let simple = decode(SIMPLE_MARK, "simple package builder");
        let builder = decode(ALL_MARKS[1].1, "package builder");

        let sample = at(&simple, BOLT_MIDDLE);
        assert_eq!(sample[3], 255, "the bolt must be opaque");
        assert!(
            i32::from(sample[2]) > i32::from(sample[0]) + 60,
            "the bolt should be the accent blue the M is, and it is {sample:?}"
        );
        assert_ne!(
            sample,
            at(&builder, BOLT_MIDDLE),
            "the bolt is bare plate, exactly as it is on the builder's own mark"
        );

        let ratio = contrast(&sample, &at(&simple, PLATE));
        assert!(
            ratio >= 4.5,
            "the bolt is only {ratio:.2}:1 against the plate under it; the floor is 4.5:1"
        );

        let (left, top, right, bottom) = BOLT_BOX;
        for (x, y, pixel) in simple.enumerate_pixels() {
            if (left..=right).contains(&x) && (top..=bottom).contains(&y) {
                continue;
            }
            assert_eq!(
                pixel,
                builder.get_pixel(x, y),
                "the simple mark differs from the builder's at ({x}, {y}), which the bolt does \
                 not reach"
            );
        }
    }

    /// WCAG relative luminance, from an RGBA pixel.
    ///
    /// The sRGB transfer function rather than the weighted-sum shortcut `theme.rs` uses for the
    /// lyric wipe. That one is comparing two *known* light colors and only needs to know they
    /// differ; this one is comparing a light mark against a dark ground, which is exactly where a
    /// linear approximation is furthest wrong.
    fn luminance(pixel: &image::Rgba<u8>) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(pixel[0]) + 0.7152 * channel(pixel[1]) + 0.0722 * channel(pixel[2])
    }

    fn contrast(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Every band is a lit, colored surface and not a hole.
    ///
    /// **The regression the lit ground exists to prevent.** A tile of `Theme::background` washed
    /// with a dim indigo and darkened 30% at the edges puts a sample here at `rgb(8, 10, 19)` — a
    /// color that is black to look at. Every other test samples the *mark*, so all of them pass
    /// happily on a mark floating on nothing.
    ///
    /// Two assertions because there are two ways back. Luminance catches a return to near-black; the
    /// channel spread catches the other one, a ground lifted to a neutral gray, which would clear a
    /// brightness floor while losing the whole point.
    #[test]
    fn every_band_is_a_lit_surface_and_not_a_hole() {
        for (who, png) in ALL_MARKS {
            let decoded = decode(png, who);

            for (point, what) in [
                (BAND_DARK, "deep purple"),
                (BAND_MID, "magenta"),
                (BAND_LEAD, "lead"),
            ] {
                let band = at(&decoded, point);
                assert_eq!(band[3], 255, "{who}: the {what} band must be opaque");

                let lit = luminance(&band);
                assert!(
                    lit > 0.020,
                    "{who}: the {what} band has gone near-black ({band:?}, luminance {lit:.4})"
                );

                let (low, high) = (
                    band[0].min(band[1]).min(band[2]),
                    band[0].max(band[1]).max(band[2]),
                );
                assert!(
                    high - low >= 30,
                    "{who}: the {what} band has gone gray ({band:?}); it is meant to have a hue"
                );
            }
        }
    }

    /// The letters clear the plate by 4.5:1, and the plate clears every band it sits on.
    ///
    /// **Two floors, and they pull in opposite directions — which is the whole change.** The old
    /// version of this test was a *ceiling* on how bright the ground could go, because the mark
    /// stood directly on it: one wash under three colored marks made the wash's luminance a budget
    /// spent three times over, and it is what forced `Theme::accent` up from `#4FC3F7`. The mark
    /// stands on the plate now, so the ground has no ceiling left. What it has instead is a
    /// **floor**: too dark a band and the plate stops being an object and becomes a hole in the
    /// tile, which is how the deep purple came to be lifted off `Theme::icon_ground` — straight
    /// `icon_ground` measured 1.2:1 against the plate and the plate's top-left edge simply was not
    /// there.
    ///
    /// 1.8:1 for the plate rather than the 4.5:1 the letters get, because these are different jobs.
    /// A letter has to be *read*; a plate edge only has to be *seen*, and 1.8:1 against a
    /// near-black is a visible boundary. The darkest band measures about 2.2:1 where it meets the
    /// plate, so this fails on a real darkening rather than on a rounding difference.
    #[test]
    fn the_letters_clear_the_plate_and_the_plate_clears_its_bands() {
        const LETTER_FLOOR: f32 = 4.5;
        const PLATE_FLOOR: f32 = 1.8;

        for (who, png) in ALL_MARKS {
            let decoded = decode(png, who);
            let plate = at(&decoded, PLATE);

            for (point, what) in [(LETTER_K, "K"), (LETTER_M, "M")] {
                let letter = at(&decoded, point);
                let ratio = contrast(&letter, &plate);
                assert!(
                    ratio >= LETTER_FLOOR,
                    "{who}: the {what} ({letter:?}) is only {ratio:.2}:1 against the plate under it \
                     ({plate:?}); the floor is {LETTER_FLOOR}:1"
                );
            }

            for (point, what) in [
                (BAND_DARK, "deep purple"),
                (BAND_MID, "magenta"),
                (BAND_LEAD, "lead"),
            ] {
                let band = at(&decoded, point);
                let ratio = contrast(&plate, &band);
                assert!(
                    ratio >= PLATE_FLOOR,
                    "{who}: the plate ({plate:?}) is only {ratio:.2}:1 against the {what} band \
                     beside it ({band:?}); the floor is {PLATE_FLOOR}:1, and below it the plate \
                     stops being an object and becomes a hole in the tile"
                );
            }
        }
    }
}
