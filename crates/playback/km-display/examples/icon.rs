//! Renders the application icons, for every platform that wants one.
//!
//! ```text
//! cargo run -p km-display --example icon
//! ```
//!
//! Writes `icon/` at the repository root, plus Android's launcher resources under
//! `ports/machine/android/app/src/main/res/`. The results are committed: they are assets rather than build
//! products — Gradle needs Android's copies present in `res/`, `build.rs` needs the `.ico` before it
//! can put it in the executable, and nobody should need a Rust toolchain to build an APK. Rendering
//! is deterministic, so regenerating produces byte-identical files and re-running this is not a diff.
//!
//! **Three layers: shards, plate, monogram.** Angular bands of color fill the tile, a near-black
//! square sits over them, and `KM` stands on the square — the K in the theme's near-white, the M in
//! the hue that names the program.
//!
//! **The M is colored because the wordmark colors it.** `site/index.html` sets the name as
//! `Karaoke<span>Machine</span>` and `site/style.css` gives that span the amber. The machine's icon
//! is therefore its own wordmark with everything but the initials taken away, and the other two are
//! the same idea in their own hue. Nothing here invents a treatment.
//!
//! **What this replaced, and why.** It was the ordinary microphone pictogram on a soft
//! violet-into-magenta radial wash, in three hues — a stock symbol on a gradient, where the shape
//! said "audio" rather than "karaoke" and nothing in it was drawn for this product. A drawn figure
//! was tried in between (a singer, microphone at a lifted chin) and rejected: it read at 48 pixels
//! and up and was a smudge below that, where a two-letter monogram is still two letters.
//!
//! **One drawing, three palettes.** Each program's shards *lead* with its own hue — the machine
//! with the theme's sung-lyric amber, the package builder with the accent blue, the offline remote
//! with the second accent green — over the same violet and magenta, which is what holds the three
//! together as one product. See [`machine_lead`], [`builder_lead`] and [`remote_lead`]. It means
//! [`render`] takes one color, and nothing else here knows which program it is drawing for.
//!
//! **And one badge, which is a second axis and not a fifth palette.** A hue says which program a
//! mark is; the badge says how that program was *started*, for the one launcher that starts the
//! machine streaming. It wears the lead it is given rather than a hue of its own, so the axes stay
//! separate and a mark reads as *this program, that way round*. See [`Badge`].
//!
//! **The ground is tinted per mark rather than shared.** A shared ground is right while the mark is
//! a colored glyph on a wash — the glyph carries the difference and a second wash would be a second
//! design. It is not right for two letters, one of which is the same near-white in all three: the
//! color has to reach further than one letter, and the shards are where it goes.
//!
//! **Every color comes from [`Theme`]**, and none of them is new: the leads are `lyric_sung`,
//! `accent` and `accent_alt`, the supporting shards are `icon_ground` and `icon_glow`, the plate is
//! `background` and the K is `lyric_pending`. `background` as the *plate* does not fold the icons'
//! ground back into the television's — see [`Theme::icon_ground`], which the darkest band is still
//! derived from. The plate is a dark **object** sitting on a lit ground, and a television's
//! near-black is the right value for one.
//!
//! **Why distance fields rather than a bitmap that gets scaled down.** An icon is not one image at
//! several resolutions. Margins can change as it shrinks — see [`PLATE_SMALL`], where the plate
//! *grows* toward the tile edges so the letters keep their pixels — and distance fields give exact
//! antialiasing from one sample per pixel, which is what keeps a 16-pixel icon crisp instead of
//! soft.
//!
//! **One drawing at every size, at nominal weight.** This is the file's oldest lesson, so do not
//! spend the afternoon on the alternatives: a pictogram drawn a second way for the small sizes is
//! worse whichever clever thing it does. Dropping elements makes it a trophy; widening every stroke
//! to a two-pixel floor makes it a blob. There is no `Detail` enum here for the same reason —
//! nothing in the drawing wants gating by size.
//!
//! **16 pixels is where this design is weakest, and that is a trade taken with eyes open.** Two
//! letters in a twelve-pixel plate are a smear; what identifies the icon there is the palette, not
//! the letters. Everything from 32 up reads `KM` cleanly.
//!
//! **What is not here.** No drop shadow under the macOS squircle. Apple's own templates carry one;
//! omitting it makes the icon look slightly flatter than a system app's and costs nothing else.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{ImageFormat, Rgba, RgbaImage};
use km_display::theme::Theme;
use sdl3::pixels::Color;

/// Where the platform-neutral icons go.
const ICON_DIR: &str = "icon";

/// Where Android's launcher resources go.
const ANDROID_RES: &str = "ports/machine/android/app/src/main/res";

/// Where the *remote's* Android launcher resources go.
///
/// A second Android application, so a second res tree. It gets the same three files per density that
/// the machine does and nothing else — no `drawable-xhdpi/banner.png`, because a banner is the tile a
/// television's home row draws and a remote never reaches one.
const ANDROID_REMOTE_RES: &str = "ports/remote/android/app/src/main/res";

/// Where the *remote's* iOS app icon goes.
///
/// One file, where the two Android trees get three per density each: iOS applies its own
/// superellipse mask and resamples one image for every size it needs, so there is no layer split
/// and no ladder. See the `ios` layout and [`opaque_png`] for the two things that are different
/// about drawing it.
const IOS_REMOTE_ASSETS: &str = "ports/remote/ios/KaraokeRemote/Assets.xcassets/AppIcon.appiconset";

/// The machine's lead: the theme's sung-lyric amber.
///
/// The warmest of the three and the color the application is about — a lit room rather than a
/// piece of software. It is also the one of the three with contrast to spare: the blue and the
/// green sit at the lift a contrast floor asks for, and the amber clears it by a margin.
fn machine_lead() -> Color {
    Theme::default().lyric_sung
}

/// The package builder's lead: the theme's accent blue.
///
/// **Same shards, same plate, same letters — a different hue.** These programs are run side by side
/// on one desktop, where two windows, two taskbar buttons, two Explorer entries or two Dock icons
/// wearing the identical icon cannot be told apart. A palette is the smallest thing that separates
/// them while still saying they belong together; a second *drawing* would say there are two
/// products. The blue is the theme's existing accent rather than something invented here, for the
/// same reason the amber is the theme's sung-lyric color.
fn builder_lead() -> Color {
    Theme::default().accent
}

/// The offline remote's lead: the theme's second accent, green.
///
/// The third program, on the same reasoning as [`builder_lead`] — and the one that makes the
/// reasoning bite. An offline remote wearing the machine's *identical* icon is defensible only
/// while it reads as the machine's remote, and this one has a window, a mirror of its own catalog
/// and a life with the machine switched off.
///
/// Green because it is the furthest hue from both of the others, which is what a 16-pixel favicon
/// has to survive. It is [`Theme::accent_alt`] rather than a literal here, so the rule that these
/// colors come out of the display's palette still holds for all three.
fn remote_lead() -> Color {
    Theme::default().accent_alt
}

/// KaraokeMachine Admin's lead: the theme's magenta, which is also the icons' own glow.
///
/// **The fourth program, and the first one whose lead was already spoken for.** The rule the other
/// three keep — the lead comes out of [`Theme`] and none of those colors is new — left no unclaimed
/// hue by the time a fourth arrived: `alert` means *warning*, `icon_ground` is a ground and too dark
/// to lead, and the two `lyric_*` colors are the words on a screen. What was left is
/// [`Theme::icon_glow`], and it is the right one on the argument that picked green for the remote:
/// magenta is the furthest hue from amber, blue and green, which is what a 16-pixel favicon has to
/// survive.
///
/// **What it costs is one collision, and the collision is deliberately not designed away.**
/// `icon_glow` is the bright end of the two supporting bands in every mark, so this icon's lead band
/// and its middle band are the same *hue*. They are not the same *value* — [`Shards::for_lead`]
/// darkens the lead band's start to 0.76 where the middle band's end is lifted 8% toward white — so
/// the tile still reads as three steps, one of which is a fold rather than a hue change.
///
/// The alternative was to tint this icon's supporting bands, and that is the thing not to do:
/// `the_hue_reaches_two_places_and_nowhere_else` holds that the deep purple, the magenta, the plate
/// and the K are byte-for-byte identical in every mark, which is what stops four programs becoming
/// four designs. **A fourth mark that shares a hue with its own band is a smaller price than a
/// fourth mark that shares nothing with the other three.**
/// **It is the theme's magenta *lifted*, and the lift is not a decoration.** Straight `icon_glow`
/// measures **4.00:1** for the M against the near-black plate, under the 4.5:1 floor
/// `the_letters_clear_the_plate_and_the_plate_clears_its_bands` holds — and that is not bad luck.
/// `Theme::icon_glow` carries a floor and not a ceiling, because the mark stands on a plate rather
/// than on the ground. Making it a *mark* color puts the ceiling of a mark back on it, and it
/// fails.
///
/// Lifting toward white is the established answer rather than a new one — it is what holds the
/// contrast floor for the blue and the green as well. This is that move applied where the palette
/// is *read* rather than to the palette itself:
/// lifting `icon_glow` in `Theme` would change the supporting bands of all four marks to fix one
/// letter.
fn admin_lead() -> Color {
    let lifted = toward_white(channels(Theme::default().icon_glow), MAGENTA_LIFT);
    Color::RGB(
        (lifted[0] * 255.0).round() as u8,
        (lifted[1] * 255.0).round() as u8,
        (lifted[2] * 255.0).round() as u8,
    )
}

/// How far the magenta is lifted toward white to clear the letter floor.
///
/// Measured rather than chosen, and bisected rather than guessed: straight `icon_glow` gives the M
/// **4.00:1** against the plate where the floor is 4.5:1, a lift of 0.04 gives **4.23:1** and still
/// fails, and 0.08 is the first tried that clears.
///
/// **0.12 rather than 0.08, deliberately.** A value chosen to sit exactly on a floor is one rounding
/// change away from failing, and the thing it would fail is legibility. The margin is cheap here
/// because the cost of lifting further is only that the M drifts from the magenta its own band is
/// drawn in — and at 0.12 the two are still plainly one color, which is what makes a shared hue
/// read as a design rather than a mistake.
const MAGENTA_LIFT: f32 = 0.12;

/// The sizes committed as loose PNGs.
///
/// Not an arbitrary ladder — every one of these has a consumer. 16 and 32 are the freedesktop icon
/// theme's small sizes and 32 is what the two web pages use as a favicon; 256 is compiled into the
/// binary as the window icon; 48 through 512 fill out `hicolor` so a Linux desktop never has to
/// scale one. The sizes inside the `.ico` and the `.icns` are rendered straight into those
/// containers and are deliberately not committed twice.
///
/// **1024 is the odd one, and its reader is not a desktop.** It is the Debian package's Plymouth
/// theme — `usr/share/plymouth/themes/karaokemachine/logo.png` — which draws the mark on a
/// television during boot rather than in a taskbar. It goes no further into `hicolor`: the package's
/// asset list names every destination one by one, so a size added here does not silently acquire a
/// theme directory. An exact 1024 render rather than an upscale of the 512, which costs nothing
/// worth counting: everything in this file is signed distance functions over a unit square, so a
/// bigger tile is a bigger drawing and not a bigger copy.
const TILE_SIZES: &[u32] = &[16, 32, 48, 64, 128, 256, 512, 1024];

/// The sizes the package builder's icon is rendered at as loose PNGs.
///
/// Six where the machine has seven, and every one of them has a reader: 32 is the favicon
/// `km-package-builder` serves at `/static/icon.png`, and 16 through 256 are what `--register`
/// installs into `hicolor` so its `.desktop` entry finally has the icon it has always named. There
/// is no 512 because nothing would read one as a loose file — the builder has no Debian package. The
/// macOS bundle reads `km-package-builder.icns`, which carries its own 512 and 1024 and is written
/// from [`ICNS_MEMBERS`] rather than from this list.
const BUILDER_TILE_SIZES: &[u32] = &[16, 32, 48, 64, 128, 256];

/// The sizes the offline remote's icon is rendered at as loose PNGs.
///
/// Two, and the same rule as the list above decides which: 32 is the favicon `km-remote-pages` serves at
/// `/static/icon.png`, and 256 is what `km_display::icon`'s test samples to check the three palettes
/// are still three palettes. There is no 16-to-128 ladder because the remote installs nothing into
/// `hicolor` — it has no `--register` and no `.desktop` entry — and no 512 for the same reason the
/// builder has none. Add sizes here when something starts reading them, not before.
const REMOTE_TILE_SIZES: &[u32] = &[32, 256];

/// The loose sizes KaraokeMachine Admin needs.
///
/// The same two the remote takes, and for the same two readers: 32 is the favicon `km-admin` serves
/// at `/static/icon.png` and the PNG its tray reads on macOS, and 256 is what `km_display::icon`'s
/// tests sample. It registers a `.desktop` entry but installs no `hicolor` ladder — the entry names
/// the executable's own icon — so the 16-to-128 rung the builder has is not wanted here.
const ADMIN_TILE_SIZES: &[u32] = &[32, 256];

/// The loose sizes the streaming launcher's mark needs.
///
/// **Not a fifth program, so not a fifth palette — the machine's own mark with a badge on it.** The
/// same rule about readers picks the list: 16 through 256 are the `hicolor` ladder the Debian
/// package and `--register` install for the `.desktop` file's stream action, and 32 is also the PNG
/// the macOS menu bar is handed for a streaming run. 256 is what `km_display::icon`'s test samples.
///
/// No 512 and no 1024. Those two belong to the machine because its package fills `hicolor` to 512
/// and carries a Plymouth theme, and a boot splash is drawn before anything has been started one
/// way or the other.
const STREAM_TILE_SIZES: &[u32] = &[16, 32, 48, 64, 128, 256];

/// How tall the macOS menu bar mark is drawn, in pixels. Its width follows from the drawing.
///
/// **One file rather than a ladder, because it has one reader.** `km-tray` decodes this and resizes
/// it for the bar, so every other size would be a file nothing opens.
///
/// **128 and not 44.** The resize is Lanczos, a filter for making things smaller, so a source drawn
/// at the size the bar wants would be exact on a Retina bar and upscaled on every other. The mark is
/// drawn large and let down.
const BAR_ICON_HEIGHT: u32 = 128;

/// How much of the menu bar mark's height is ink, the rest being clear space around it.
///
/// **A menu bar item is 22 points tall and nothing in one is 22 points of ink.** `tray-icon` scales
/// whatever it is handed to 18 points, so a file whose letters run edge to edge puts 18 points of
/// letter beside glyphs drawn at about 13, and the mark reads as shouting. This is the margin that
/// every other icon up there has built into it.
///
/// It is a fraction rather than a pixel count so that changing [`BAR_ICON_HEIGHT`] changes the
/// resolution and nothing else — the two are separate questions and a pixel margin ties them
/// together.
const BAR_ICON_INK: f32 = 0.74;

/// What goes inside the Windows `.ico`.
///
/// 24 is here and nowhere else: Windows asks for it in a few places (the small taskbar, some list
/// views) and synthesises a poor one from 32 if it is missing.
const ICO_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];

/// What goes inside the macOS `.icns`, as (four-character type, pixels).
///
/// The duplication is the format's, not ours: macOS wants the same pixel size under two type codes,
/// one meaning "N points at 1x" and the other "N/2 points at 2x", and a Retina display picks the
/// second. `iconutil` emits exactly this set.
const ICNS_MEMBERS: &[(&[u8; 4], u32)] = &[
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];

/// Android's density buckets, as (resource suffix, legacy launcher pixels, adaptive layer pixels).
///
/// The two sizes are different things. The legacy `ic_launcher` is 48 dp of finished icon. An
/// adaptive layer is 108 dp, of which the system may show as little as the middle 72 dp and crops
/// the rest to whatever mask the launcher has chosen — so the layers are more than twice the area
/// and most of it is margin.
const ANDROID_DENSITIES: &[(&str, u32, u32)] = &[
    ("mdpi", 48, 108),
    ("hdpi", 72, 162),
    ("xhdpi", 96, 216),
    ("xxhdpi", 144, 324),
    ("xxxhdpi", 192, 432),
];

// -- the monogram ---------------------------------------------------------------------------------

/// The letters, in the coordinates of the plate they sit on: 0 at the left and top, 1 at the right
/// and bottom.
///
/// **`KM`, and the same two letters on all three icons.** They are three programs and one product,
/// and the product is what a monogram names; what tells them apart is the palette behind them. Per
/// program initials would be a second thing saying the same thing, and two of the three would be
/// initials nobody has ever seen written down.
///
/// **Every stroke is a capsule, and the band is what squares them off.** The letters are cut to a
/// slab running from [`CAP_TOP`] to [`BASELINE`], so a stroke that runs past it comes back with a
/// flat end aligned to its neighbors, while the joins *inside* a letter keep their round caps —
/// which is where round is wanted. That is one intersection standing in for a whole set of oriented
/// boxes, and it is why there is no rotation anywhere in this file.
mod monogram {
    /// How much of its own drawing the mark is set at, about the middle of the plate.
    ///
    /// **One number rather than a nudge per constant.** Everything below is one coordinate system,
    /// so moving the cap height without moving the arms, the waist and the vertex with it does not
    /// make the letters smaller — it makes them a different pair of letters. Scaling the *input* to
    /// [`super::monogram_distance`] moves all of it at once and cannot break a join.
    ///
    /// At 1.0 the letters left 10% of the plate clear on each side, which read as a plate the mark
    /// was too big for at every size and on every platform. This is that gap widened to about a
    /// sixth. The vertical margin was never the tight one — the band is 41% of the plate tall — so
    /// the shrink is felt on the sides, which is where it was wanted.
    pub const SCALE: f32 = 0.85;

    /// The band every terminal is cut to. Symmetric about the middle of the plate.
    pub const CAP_TOP: f32 = 0.295;
    pub const BASELINE: f32 = 0.705;

    /// Half the weight of a stroke.
    ///
    /// A quarter of the cap height, which is heavier than a text face would ever be set and about
    /// right for a mark: a stroke has to survive being a pixel and a half wide at 32 pixels, and
    /// the counters have to survive being about as thin.
    pub const WEIGHT: f32 = 0.052;

    /// How far a stroke runs past the band before the band cuts it.
    ///
    /// Only has to exceed [`WEIGHT`], so that the round cap it is there to hide clears the edge.
    pub const OVERRUN: f32 = 0.070;

    /// **K**: an upright, and two arms meeting it at the waist.
    pub const K_STEM: f32 = 0.152;
    pub const K_WAIST: (f32, f32) = (0.152, 0.500);
    /// Where each arm crosses the band. They are given as the point on the band and extended
    /// outward from the waist, so moving the band moves the letter rather than breaking it.
    pub const K_ARM_TIP: (f32, f32) = (0.343, CAP_TOP);
    pub const K_LEG_TIP: (f32, f32) = (0.360, BASELINE);

    /// **M**: two uprights, and a V between them.
    ///
    /// Each diagonal starts at the top of its own upright, so the two joins at the cap line need no
    /// help; the round caps meeting at [`M_VERTEX`] are what closes the V at the bottom.
    pub const M_LEFT: f32 = 0.560;
    pub const M_RIGHT: f32 = 0.848;
    pub const M_VERTEX: (f32, f32) = (0.704, 0.612);
}

/// Signed distance to the **K** and to the **M** separately, in plate units. Negative inside.
///
/// Two distances rather than one because the two letters are not the same color: the site's
/// wordmark sets `Karaoke` in the text color and `Machine` in the amber, and this is that wordmark
/// with everything but the initials taken away. [`render`] colors them and does not otherwise care
/// that there are two.
fn monogram_distance(q: (f32, f32)) -> (f32, f32) {
    use monogram::*;

    // The mark is set at `SCALE` about the middle of the plate, and the cheapest place to do that
    // is here: the point being asked about is moved into the letters' own coordinates, and the
    // distance that comes back is scaled to undo it. Every constant below stays the drawing it
    // always was, and a caller measuring in plate units still gets plate units.
    let q = ((q.0 - 0.5) / SCALE + 0.5, (q.1 - 0.5) / SCALE + 0.5);

    let upright = |x: f32| capsule(q, (x, CAP_TOP - OVERRUN), (x, BASELINE + OVERRUN), WEIGHT);
    let arm = |tip: (f32, f32)| capsule(q, K_WAIST, extended(tip, K_WAIST, OVERRUN), WEIGHT);
    let diagonal = |top: f32| {
        capsule(
            q,
            extended((top, CAP_TOP), M_VERTEX, OVERRUN),
            M_VERTEX,
            WEIGHT,
        )
    };

    let k = upright(K_STEM).min(arm(K_ARM_TIP)).min(arm(K_LEG_TIP));
    let m = upright(M_LEFT)
        .min(upright(M_RIGHT))
        .min(diagonal(M_LEFT))
        .min(diagonal(M_RIGHT));

    // The band, as an intersection. Wide enough that only its top and bottom edges can ever bite,
    // which is the whole point: it squares off terminals and never touches a letter's sides.
    let band = rounded_rect(q, (-1.0, CAP_TOP), (2.0, BASELINE), 0.0);
    (k.max(band) * SCALE, m.max(band) * SCALE)
}

/// `from`, pushed `amount` further away from `to` along the line through both.
fn extended(from: (f32, f32), to: (f32, f32), amount: f32) -> (f32, f32) {
    let (dx, dy) = (from.0 - to.0, from.1 - to.1);
    let length = dx.hypot(dy);
    (from.0 + dx / length * amount, from.1 + dy / length * amount)
}

/// What the mark carries besides the letters.
///
/// **A badge says how the program was started, where the hue says which program it is.** The four
/// leads are four programs; this is one program with two launchers, and a fifth hue would say it
/// was a fifth program. See [`Badge::Stream`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Badge {
    /// Nothing. The mark as the four programs wear it.
    None,
    /// A source and the waves leaving it, for the launcher that starts the machine streaming.
    Stream,
}

/// The badge, in the coordinates of the plate it sits on, like [`monogram`].
///
/// **A source at the lower left of it and two waves opening up and to the right**, which is the
/// way every platform already draws this and therefore the only way worth drawing it: a mirrored
/// one is a shape nobody has seen, and the whole value of a stock glyph is that it is stock. The
/// badge sits in the plate's bottom right corner, because the strip under
/// [`monogram::BASELINE`] is the part of the plate the letters leave empty.
///
/// **Two waves rather than three.** A third would have to reach past the letters' baseline, and a
/// badge that touches the M is a badge drawn on top of the mark rather than beside it. Two is also
/// what survives the shrink: at 16 pixels this is three pixels of amber in a corner, and what it
/// says there is *not the plain machine*, which is the whole of what a notification area needs it
/// to say.
mod badge {
    /// Where the waves come from.
    ///
    /// Low enough to leave the letters alone and far enough from the corner that the widest wave
    /// clears the plate's own rounded edge, whose curve starts at `1 - PLATE_RADIUS` on both axes.
    pub const SOURCE: (f32, f32) = (0.780, 0.912);

    /// The source itself.
    pub const DOT: f32 = 0.027;

    /// The waves, as the radius each is drawn at.
    pub const WAVES: [f32; 2] = [0.070, 0.122];

    /// Half the weight of a wave.
    ///
    /// Lighter than [`super::monogram::WEIGHT`], because these are not letters and do not have to
    /// be read — at the sizes where a stroke is a pixel wide the whole badge is a blob either way,
    /// and above them a wave heavy enough to match the M would close the gaps between the three
    /// shapes.
    pub const WEIGHT: f32 = 0.019;
}

/// Signed distance to the badge, in plate units. Negative inside.
///
/// [`Badge::None`] is a distance nothing ever gets inside of rather than a branch in [`render`]:
/// the pixel loop asks one question and the answer is a number, which is the same arrangement the
/// plate's own `None` already has.
fn badge_distance(q: (f32, f32), badge: Badge) -> f32 {
    if badge == Badge::None {
        return f32::MAX;
    }

    let mut distance = disc(q, badge::SOURCE, badge::DOT);
    for radius in badge::WAVES {
        distance = distance.min(wave(q, badge::SOURCE, radius, badge::WEIGHT));
    }
    distance
}

/// Signed distance to a quarter ring: the circle of `radius` about `center`, thickened by `weight`
/// and kept to the quadrant above and to the right of the center.
///
/// **The quadrant is an intersection, exactly as the monogram's band is.** Two half planes cut the
/// ring down to the quarter that is wanted and leave flat ends on the axes, which is the same
/// squared terminal the letters get and the reason there is still no rotation anywhere in this
/// file.
fn wave(p: (f32, f32), center: (f32, f32), radius: f32, weight: f32) -> f32 {
    let ring = ((p.0 - center.0).hypot(p.1 - center.1) - radius).abs() - weight;
    ring.max(center.0 - p.0).max(p.1 - center.1)
}

/// Signed distance to a disc.
///
/// **Not [`capsule`] with both ends in the same place**, which is the obvious spelling and is a
/// division by zero: that function measures how far along the segment a point falls, and a segment
/// of no length gives `NaN`. `min` then returns the *other* operand whenever one is `NaN`, so the
/// shape does not come out wrong — it comes out absent, which is a great deal harder to see.
fn disc(p: (f32, f32), center: (f32, f32), radius: f32) -> f32 {
    (p.0 - center.0).hypot(p.1 - center.1) - radius
}

// -- the tile -------------------------------------------------------------------------------------

/// What sits behind the plate.
#[derive(Debug, Clone, Copy)]
enum Ground {
    /// A rounded square filling the canvas. Windows, Linux, the web pages, the window icon, and
    /// Android's legacy launcher icon.
    Tile,
    /// The same square, inset. macOS draws icons smaller than their canvas and expects the artwork
    /// to leave that room itself.
    Inset,
    /// The whole canvas, corners included: Android's background layer is masked by the launcher, so
    /// rounding it here would round it twice and leave the corners ragged.
    Bleed,
    /// Nothing. Android's foreground layer is transparent by definition — it is composited over the
    /// background layer, which carries the same shards, so the two line up into one image.
    None,
}

/// One rendering: what is behind the plate, and where the plate goes.
struct Layout {
    ground: Ground,
    /// The plate's square box as (origin, size) in canvas fractions, or `None` to draw no plate and
    /// no letters at all.
    ///
    /// One box for both, deliberately: the letters' coordinates are the plate's, so the monogram
    /// cannot drift out of the square it is cropped by.
    plate: Option<(f32, f32)>,
}

/// How much of an ordinary tile the plate takes up.
///
/// Enough that the shards read as a frame around an object rather than as a border drawn on one, and
/// not so much that the color is a hairline. The letters live inside this, so every point given
/// back to the shards is taken off them.
const PLATE: f32 = 0.62;

/// The same, for the small sizes, and the size up to which it applies.
///
/// **The plate grows rather than going away.** A margin costs the same *fraction* at every size, and
/// at 16 pixels that fraction is most of the icon: at [`PLATE`] the letters would have ten pixels to
/// stand in. Trading the shards down to a frame buys it three more in each direction, which is the
/// difference between letters and a smudge.
///
/// The obvious alternative — drop the plate and set the letters straight on the shards — is the
/// wrong way round here, because the plate is where all of their contrast comes from. It would have
/// been right for the colored glyph this replaced, which is presumably why it looks right.
const PLATE_SMALL: f32 = 0.80;
const SMALL_UP_TO: u32 = 32;

/// The plate's corner radius, as a fraction of its own side.
const PLATE_RADIUS: f32 = 0.16;

/// The tile's corner radius, as a fraction of the canvas.
const TILE_RADIUS: f32 = 0.20;

/// How far macOS's rounded square is inset from its canvas: 100 of 1024, which is what Apple's
/// template grid uses.
const MACOS_INSET: f32 = 0.0977;

/// And its corner radius, on the same grid: 185 of 1024.
const MACOS_RADIUS: f32 = 0.1807;

/// How much of an Android adaptive layer the plate takes up.
///
/// Not 66/108 — that is the diameter of the circle the content has to fit *inside*, and a square's
/// corners stick out of a circle drawn through its edges. The plate's furthest point from its own
/// center is `√2 × (0.5 - PLATE_RADIUS) + PLATE_RADIUS = 0.641` plate-units, so the plate can be at
/// most `0.6111 / (2 × 0.641) = 0.477` of the canvas before a round mask starts clipping a corner.
///
/// Smaller than a tall thin pictogram could take, and that is geometry rather than a loss: a square
/// pays more to fit in a circle than a pictogram does.
const ADAPTIVE_PLATE: f32 = 0.476;

impl Layout {
    /// A finished icon, filling its canvas.
    fn tile(size: u32) -> Self {
        let plate = if size <= SMALL_UP_TO {
            PLATE_SMALL
        } else {
            PLATE
        };
        Self {
            ground: Ground::Tile,
            plate: Some(((1.0 - plate) / 2.0, plate)),
        }
    }

    /// The macOS variant: the same tile, inset, with the plate centered inside it.
    fn inset() -> Self {
        let tile = 1.0 - 2.0 * MACOS_INSET;
        let plate = tile * PLATE;
        Self {
            ground: Ground::Inset,
            plate: Some((MACOS_INSET + (tile - plate) / 2.0, plate)),
        }
    }

    /// Android's foreground layer: the plate and the letters, on nothing.
    fn adaptive_foreground() -> Self {
        Self {
            ground: Ground::None,
            plate: Some(((1.0 - ADAPTIVE_PLATE) / 2.0, ADAPTIVE_PLATE)),
        }
    }

    /// Android's background layer: the shards, on their own.
    fn adaptive_background() -> Self {
        Self {
            ground: Ground::Bleed,
            plate: None,
        }
    }

    /// iOS: the whole icon on a full-bleed ground, because the system supplies the mask.
    ///
    /// [`Ground::Bleed`] already carries the argument, in its own words — Android's background
    /// layer is masked by the launcher, so rounding it here would round it twice. iOS says exactly
    /// the same thing about its superellipse. What was missing was a layout pairing that ground
    /// with the plate, since Android splits the two across separate layers and iOS wants one image.
    ///
    /// **[`PLATE`] and deliberately not [`ADAPTIVE_PLATE`].** That smaller fraction is derived from
    /// a *circular* launcher mask, which clips far more aggressively than iOS's rounded square;
    /// using it here would leave a plate visibly smaller than every other icon this file draws, for
    /// a clipping risk that does not exist.
    fn ios() -> Self {
        Self {
            ground: Ground::Bleed,
            plate: Some(((1.0 - PLATE) / 2.0, PLATE)),
        }
    }
}

// -- the shards -----------------------------------------------------------------------------------

/// The three bands of color a tile is painted from, as (deep end, bright end).
///
/// Held as resolved channels rather than as [`Color`], because [`shards`] runs once per pixel and
/// [`Theme::default`] should not.
struct Shards {
    violet: ([f32; 3], [f32; 3]),
    magenta: ([f32; 3], [f32; 3]),
    lead: ([f32; 3], [f32; 3]),
}

impl Shards {
    /// The palette for one program: its own hue, over the two every program shares.
    ///
    /// Both ends of every band are derived from a [`Theme`] color rather than written out, so a
    /// palette change reaches the icons instead of leaving them looking like the last one.
    fn for_lead(lead: Color) -> Self {
        let theme = Theme::default();
        let violet = channels(theme.icon_ground);
        let magenta = channels(theme.icon_glow);
        let lead = channels(lead);
        Self {
            // **The dark band is a deep purple and not `icon_ground` itself, and that was
            // measured.** Straight `icon_ground` came out at 1.2:1 against the plate, which is not
            // a dark corner but a missing one: the plate's top-left edge simply disappeared into
            // it. Lifted toward the magenta it reads about 2.3:1 where the plate meets it — enough
            // that the plate is an object with a border all the way round, while the tile still has
            // somewhere quiet to be.
            violet: (mix(violet, magenta, 0.44), mix(violet, magenta, 0.82)),
            magenta: (mix(magenta, violet, 0.22), toward_white(magenta, 0.08)),
            lead: (scaled(lead, 0.76), toward_white(lead, 0.12)),
        }
    }
}

/// Where the first band gives way to the second, and the second to the third, measured along the
/// top-left-to-bottom-right diagonal.
///
/// Unequal on purpose. The lead hue gets the largest share because it is the only thing that says
/// which program this is, and most of the tile is about to be covered by the plate — so what is
/// really being divided here is the frame around it, not the square.
const BAND_ONE: f32 = 0.34;
const BAND_TWO: f32 = 0.58;

/// The ground color at a point: three straight-edged bands running from the bottom-left corner to
/// the top-right, each filled with a gradient of its own.
///
/// **Angular rather than the radial wash this replaced.** A radial gradient has no edges in it, so a
/// tile made of one is a surface and nothing else; bands give the eye something to catch at 16
/// pixels, which is the size at which an icon is either recognized or not. The cost is that a seam
/// is a hard edge and needs antialiasing, which is one `coverage` call — the same machinery the
/// letters use.
fn shards(p: (f32, f32), shards: &Shards, pixel: f32, row: u32) -> [f32; 3] {
    /// How much darker the corners are than the middle.
    ///
    /// Low, and lower than the wash needed: bands already vary across the tile, so this only has to
    /// stop the four corners reading as flat. Do not take it to zero — some falloff is what makes a
    /// rounded square read as a lit object rather than a swatch.
    const VIGNETTE: f32 = 0.12;

    // Across the bands, 0 at the top-left corner and 1 at the bottom-right.
    let across = (p.0 + p.1) / 2.0;
    // And along them, 0 at the bottom-left and 1 at the top-right. Every band's gradient runs on
    // this, so the light in the tile has one direction rather than three.
    let along = (p.0 - p.1 + 1.0) / 2.0;

    // Perpendicular distance to each seam. The factor turns a difference in `across` — which is
    // measured along the diagonal — back into a real distance, which is what `pixel` is in.
    let seam = |at: f32| (across - at) * 2.0 / std::f32::consts::SQRT_2;

    let band = |(deep, bright): ([f32; 3], [f32; 3])| mix(bright, deep, along.clamp(0.0, 1.0));

    let mut color = band(shards.violet);
    color = mix(
        color,
        band(shards.magenta),
        1.0 - coverage(seam(BAND_ONE), pixel),
    );
    color = mix(
        color,
        band(shards.lead),
        1.0 - coverage(seam(BAND_TWO), pixel),
    );

    let from_center = ((p.0 - 0.5).powi(2) + (p.1 - 0.5).powi(2)).sqrt() * 2.0;
    let vignette = 1.0 - VIGNETTE * from_center.clamp(0.0, 1.4).powi(2);

    // The same per-row dither as the wallpapers, for the same reason and with the same trade-off:
    // eight-bit color cannot hold a gradient this shallow without faint contour rings, and
    // offsetting whole rows breaks them up without destroying the horizontal runs PNG compresses.
    let dither = if row.is_multiple_of(2) {
        0.0
    } else {
        1.0 / 255.0
    };

    color.map(|value| value * vignette + dither)
}

// -- putting one together -------------------------------------------------------------------------

/// Renders one icon.
///
/// `lead` is the hue that names the program — see [`machine_lead`], [`builder_lead`] and
/// [`remote_lead`]. It reaches two places: the widest band of the ground, and the **M**. The plate,
/// the K and the drawing itself are deliberately not parameters: one drawing under four palettes is
/// what says these are four programs and one product.
///
/// `badge` is the other axis and is not a palette at all: it says how the program was *started*,
/// and it takes the lead's own hue rather than a hue of its own. See [`Badge`].
fn render(size: u32, layout: &Layout, lead: Color, badge: Badge) -> RgbaImage {
    let theme = Theme::default();
    let shard_palette = Shards::for_lead(lead);
    let plate_color = channels(theme.background);
    // A gentle vertical gradient on each letter. Invisible at 16 pixels and the difference between
    // "designed" and "clip art" at 512. Derived from the theme color rather than written out, so it
    // cannot drift from the palette.
    let gradient = |color: Color| {
        (
            toward_white(channels(color), 0.10),
            scaled(channels(color), 0.90),
        )
    };
    let k_color = gradient(theme.lyric_pending);
    let m_color = gradient(lead);

    // One pixel, in canvas units. Everything below measures distances in those.
    let pixel = 1.0 / size as f32;

    RgbaImage::from_fn(size, size, |x, y| {
        let p = ((x as f32 + 0.5) * pixel, (y as f32 + 0.5) * pixel);

        let ground_alpha = match layout.ground {
            Ground::None => 0.0,
            Ground::Bleed => 1.0,
            Ground::Tile => coverage(rounded_rect(p, (0.0, 0.0), (1.0, 1.0), TILE_RADIUS), pixel),
            Ground::Inset => coverage(
                rounded_rect(
                    p,
                    (MACOS_INSET, MACOS_INSET),
                    (1.0 - MACOS_INSET, 1.0 - MACOS_INSET),
                    MACOS_RADIUS,
                ),
                pixel,
            ),
        };
        let ground = shards(p, &shard_palette, pixel, y);

        let (plate_alpha, letters) = match layout.plate {
            None => (0.0, [(0.0, [0.0; 3]); 3]),
            Some((origin, side)) => {
                let plate = rounded_rect(
                    p,
                    (origin, origin),
                    (origin + side, origin + side),
                    PLATE_RADIUS * side,
                );
                let q = ((p.0 - origin) / side, (p.1 - origin) / side);
                let down = q.1.clamp(0.0, 1.0);
                // The distances come back in plate units; scaling them returns them to canvas
                // units, which is the space `pixel` is measured in. Intersecting with the plate
                // keeps the mark inside the square it belongs to — the letters clear it
                // comfortably, so this costs nothing and guarantees the one thing a future mark
                // might get wrong.
                let (k, m) = monogram_distance(q);
                let letter = |distance: f32, (top, bottom): ([f32; 3], [f32; 3])| {
                    (
                        coverage((distance * side).max(plate), pixel),
                        mix(top, bottom, down),
                    )
                };
                (
                    coverage(plate, pixel),
                    [
                        letter(k, k_color),
                        letter(m, m_color),
                        // The badge is drawn on a letter's terms -- the same units, the same crop
                        // to the plate, the same hue as the M -- because that is what it is:
                        // another shape standing on the plate.
                        letter(badge_distance(q, badge), m_color),
                    ],
                )
            }
        };

        // Letters and badge over plate over ground. None of the three touches another, so which
        // goes on first is arbitrary and the order below is only the order they are read in.
        let (color, alpha) = over(plate_color, plate_alpha, ground, ground_alpha);
        let (color, alpha) = over(letters[2].1, letters[2].0, color, alpha);
        let (color, alpha) = over(letters[1].1, letters[1].0, color, alpha);
        let (color, alpha) = over(letters[0].1, letters[0].0, color, alpha);
        if alpha <= 0.0 {
            // Zero rather than whatever the color happened to be: an invisible pixel's color is
            // arbitrary, and holding it at zero keeps the file both smaller and reproducible.
            return Rgba([0, 0, 0, 0]);
        }
        Rgba([byte(color[0]), byte(color[1]), byte(color[2]), byte(alpha)])
    })
}

/// The box the menu bar mark is cropped to: the letters, and nothing else.
///
/// **Derived from the drawing rather than written down.** Every number here is already a constant of
/// [`monogram`], so moving a letter moves this with it; a box typed out by hand would be a second
/// description of the same shape, and the two would part company the first time either moved.
fn bar_box() -> ((f32, f32), (f32, f32)) {
    // The letters are set at `SCALE` about the middle of the plate, so their own constants have to
    // be moved the same way before they describe where the ink is.
    let set = |v: f32| 0.5 + (v - 0.5) * monogram::SCALE;

    let min = (
        set(monogram::K_STEM - monogram::WEIGHT),
        set(monogram::CAP_TOP),
    );
    let max = (
        set(monogram::M_RIGHT + monogram::WEIGHT),
        set(monogram::BASELINE),
    );
    (min, max)
}

/// The mark the macOS menu bar is given: the letters, as a silhouette.
///
/// **A template image, which is what every glyph beside it in that bar is.** macOS reads one for its
/// alpha alone and paints it itself, so a single file is right on a light bar, on a dark one, and
/// inverted while its menu is open. None of the marks above can be drawn that way — each is a
/// full-bleed rounded tile, so its silhouette is the tile — and this one can only because the plate
/// and the bands are left out.
///
/// **Leaving them out is what makes it belong there.** A plate is a dark object, and a dark object
/// in a dark menu bar is a hole; the letters are the mark's content, and content is what the other
/// glyphs up there are. So this is the same drawing with the tile taken away rather than a second
/// design — `monogram_distance`, the same call the tile makes, over the same coordinates.
///
/// **No badge, although this is only ever in a bar for a streaming run.** A badge says how the
/// machine was started, and saying it needs something beside it to be told apart from: the launchers
/// sit next to each other in a Finder window and in a Start menu, and a notification area draws the
/// television machine's mark too. Nothing else this program has ever puts an icon in the macOS menu
/// bar, so there is no pair here for a badge to separate — the same reason the window icon, the
/// favicons and the phone launchers go without one. What it buys is the letters at the bar's full
/// height rather than two thirds of it.
///
/// **Cropped to [`bar_box`] and therefore not square**, then given [`BAR_ICON_INK`]'s margin back on
/// all four sides. Cropping first is what makes the margin controllable: the letters' own box is
/// about twice as wide as it is tall, so a square canvas would already be a third margin before
/// anything was asked for, and none of it where it was wanted.
///
/// `height` is the pixel count, not the size it is shown at. See [`BAR_ICON_HEIGHT`].
fn render_bar(height: u32) -> RgbaImage {
    let ((x0, y0), (x1, y1)) = bar_box();
    // Plate units per pixel. One number for both axes, so the mark cannot come out stretched.
    let unit = (y1 - y0) / (height as f32 * BAR_ICON_INK);
    // The same clear space on all four sides, in pixels.
    let pad = height as f32 * (1.0 - BAR_ICON_INK) / 2.0;
    let width = ((x1 - x0) / unit + 2.0 * pad).round() as u32;

    RgbaImage::from_fn(width, height, |x, y| {
        let q = (
            x0 + (x as f32 + 0.5 - pad) * unit,
            y0 + (y as f32 + 0.5 - pad) * unit,
        );
        // Two distances because the tile colors the letters differently; here they are one shape, so
        // the nearer of the two is the answer.
        let (k, m) = monogram_distance(q);
        // They come back in plate units; dividing by `unit` puts them in pixels, which is the space
        // a pixel is one of.
        let distance = k.min(m) / unit;
        let alpha = coverage(distance, 1.0);
        if alpha <= 0.0 {
            return Rgba([0, 0, 0, 0]);
        }
        // Black, and thrown away: a template image is read for its alpha. Written black rather than
        // in the machine's amber so that a reader opening the file sees the silhouette macOS draws.
        Rgba([0, 0, 0, byte(alpha)])
    })
}

/// Straight-alpha source-over: `src` at `src_alpha`, on top of `dst` at `dst_alpha`.
fn over(src: [f32; 3], src_alpha: f32, dst: [f32; 3], dst_alpha: f32) -> ([f32; 3], f32) {
    let alpha = src_alpha + dst_alpha * (1.0 - src_alpha);
    if alpha <= 0.0 {
        return ([0.0; 3], 0.0);
    }
    let mut out = [0.0; 3];
    for (index, value) in out.iter_mut().enumerate() {
        *value = (src[index] * src_alpha + dst[index] * dst_alpha * (1.0 - src_alpha)) / alpha;
    }
    (out, alpha)
}

// -- the primitives -------------------------------------------------------------------------------

/// Signed distance to a rounded rectangle, given by two opposite corners.
///
/// A plain rectangle is this with `radius` zero.
fn rounded_rect(p: (f32, f32), min: (f32, f32), max: (f32, f32), radius: f32) -> f32 {
    let center = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let half = (
        (max.0 - min.0) / 2.0 - radius,
        (max.1 - min.1) / 2.0 - radius,
    );
    let d = (
        (p.0 - center.0).abs() - half.0,
        (p.1 - center.1).abs() - half.1,
    );
    let outside = (d.0.max(0.0).powi(2) + d.1.max(0.0).powi(2)).sqrt();
    let inside = d.0.max(d.1).min(0.0);
    outside + inside - radius
}

/// Signed distance to a capsule: the segment `a`–`b`, thickened by `radius`.
///
/// Every stroke of the monogram is one of these, diagonals included. That is why there is no
/// rotation helper in this file: a rotated stroke is just a segment whose two ends are somewhere
/// else, and the round caps a capsule leaves are wanted at the joins and cut off everywhere else by
/// the band (see [`monogram`]).
fn capsule(p: (f32, f32), a: (f32, f32), b: (f32, f32), radius: f32) -> f32 {
    let (px, py) = (p.0 - a.0, p.1 - a.1);
    let (bx, by) = (b.0 - a.0, b.1 - a.1);
    let along = ((px * bx + py * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
    (px - bx * along).hypot(py - by * along) - radius
}

/// How much of a pixel a shape covers, from the signed distance at its center.
///
/// This is the whole antialiasing story, and it is exact for a straight edge: the distance field
/// says how far the edge is, and an edge half a pixel away covers none of it while one passing
/// through the center covers half.
fn coverage(distance: f32, pixel: f32) -> f32 {
    (0.5 - distance / pixel).clamp(0.0, 1.0)
}

/// An SDL color as three 0..1 channels.
fn channels(color: Color) -> [f32; 3] {
    [
        color.r as f32 / 255.0,
        color.g as f32 / 255.0,
        color.b as f32 / 255.0,
    ]
}

fn toward_white(color: [f32; 3], amount: f32) -> [f32; 3] {
    color.map(|value| value + (1.0 - value) * amount)
}

fn scaled(color: [f32; 3], factor: f32) -> [f32; 3] {
    color.map(|value| value * factor)
}

fn mix(from: [f32; 3], to: [f32; 3], t: f32) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (index, value) in out.iter_mut().enumerate() {
        *value = from[index] + (to[index] - from[index]) * t;
    }
    out
}

fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

// -- the containers -------------------------------------------------------------------------------

/// Encodes an image as PNG in memory.
fn png(image: &RgbaImage) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    image.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
    Ok(bytes)
}

/// Encodes an image as PNG with **no alpha channel at all**.
///
/// iOS rejects an app icon that carries one — the mask is the system's, and a transparent pixel
/// would be a hole punched in it. Everything else here writes RGBA, so this is a second encoder
/// rather than a flag: it is one platform's rule, and the other six should not have to think about
/// it.
///
/// Composited against [`Theme::icon_ground`] rather than against white, so that a partly
/// transparent pixel resolves towards the darkest shard instead of towards a bright edge. With
/// [`Ground::Bleed`] nothing is actually transparent, so this is the guarantee rather than the fix —
/// which is the right way round: the guarantee should not depend on the layout.
fn opaque_png(image: &RgbaImage) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let base = channels(Theme::default().icon_ground);
    let flat = image::RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let Rgba([r, g, b, a]) = *image.get_pixel(x, y);
        let alpha = f32::from(a) / 255.0;
        let over = |value: u8, index: usize| {
            byte(f32::from(value) / 255.0 * alpha + base[index] * (1.0 - alpha))
        };
        image::Rgb([over(r, 0), over(g, 1), over(b, 2)])
    });
    let mut bytes = Vec::new();
    flat.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
    Ok(bytes)
}

/// Builds a Windows `.ico` holding PNG-compressed members.
///
/// Written by hand rather than through a crate because the format is a six-byte header, a sixteen
/// byte directory entry per image and the images themselves — less code than the dependency, and no
/// new encoder feature on `image` for every crate that links it.
///
/// PNG members rather than the older BMP ones: Windows has read them since Vista, and a 256×256 BMP
/// member is 256 KB where the PNG is a few.
fn ico(images: &[RgbaImage]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let payloads: Vec<Vec<u8>> = images.iter().map(png).collect::<Result<_, _>>()?;

    let count = u16::try_from(images.len())?;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 = icon, 2 = cursor
    out.extend_from_slice(&count.to_le_bytes());

    // The directory is fixed width, so every payload's offset is known before any of them is written.
    let mut offset = u32::try_from(6 + 16 * images.len())?;
    for (image, payload) in images.iter().zip(&payloads) {
        // A dimension is one byte, and 256 does not fit in one. Zero means 256; there is no way to
        // express anything larger, which is why the ladder stops there.
        let dimension = |value: u32| -> u8 { if value >= 256 { 0 } else { value as u8 } };
        out.push(dimension(image.width()));
        out.push(dimension(image.height()));
        out.push(0); // palette entries; 0 for a true-color image
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // color planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        let length = u32::try_from(payload.len())?;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += length;
    }
    for payload in &payloads {
        out.extend_from_slice(payload);
    }
    Ok(out)
}

/// Builds a macOS `.icns` holding PNG members.
///
/// Also by hand, and the format is even simpler than the `.ico`: a magic word, a total length, then
/// a run of type-tagged chunks. Everything is big-endian, and a chunk's length counts its own
/// eight-byte header.
///
/// No `TOC ` chunk. `iconutil` writes one — it is an index letting a reader find a member without
/// walking the file — and it is optional; every reader handles its absence, because an `.icns` is a
/// few hundred kilobytes and walking it is free.
fn icns(members: &[(&[u8; 4], RgbaImage)]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut chunks = Vec::new();
    for (kind, image) in members {
        let payload = png(image)?;
        chunks.extend_from_slice(*kind);
        chunks.extend_from_slice(&u32::try_from(8 + payload.len())?.to_be_bytes());
        chunks.extend_from_slice(&payload);
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&u32::try_from(8 + chunks.len())?.to_be_bytes());
    out.extend_from_slice(&chunks);
    Ok(out)
}

// -- writing it all out ---------------------------------------------------------------------------

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let icons = PathBuf::from(ICON_DIR);
    let res = PathBuf::from(ANDROID_RES);
    let mut written = 0;

    // -- the machine ------------------------------------------------------------------------------

    // The loose PNGs.
    for &size in TILE_SIZES {
        let path = icons.join(format!("icon-{size}.png"));
        write(
            &path,
            &png(&render(
                size,
                &Layout::tile(size),
                machine_lead(),
                Badge::None,
            ))?,
        )?;
        written += 1;
    }

    // Windows.
    let members: Vec<RgbaImage> = ICO_SIZES
        .iter()
        .map(|&size| render(size, &Layout::tile(size), machine_lead(), Badge::None))
        .collect();
    write(&icons.join("karaokemachine.ico"), &ico(&members)?)?;
    written += 1;

    // macOS. The small members use the plain tile rather than the inset square: at 16 and 32 pixels
    // the inset would leave nine pixels of plate, which is not an icon, and Apple's own small sizes
    // are likewise less inset than their large ones.
    let members: Vec<(&[u8; 4], RgbaImage)> = ICNS_MEMBERS
        .iter()
        .map(|&(kind, size)| {
            let layout = if size <= SMALL_UP_TO {
                Layout::tile(size)
            } else {
                Layout::inset()
            };
            (kind, render(size, &layout, machine_lead(), Badge::None))
        })
        .collect();
    write(&icons.join("karaokemachine.icns"), &icns(&members)?)?;
    written += 1;

    // Android. The legacy icon is what an API 25 device would show and is still what the manifest
    // names; the two layers are what everything from API 26 up actually draws, through whatever mask
    // the launcher has chosen. `mipmap-anydpi-v26/ic_launcher.xml` puts them together and is
    // committed by hand — it is markup, not a rendering.
    for &(density, launcher, layer) in ANDROID_DENSITIES {
        let dir = res.join(format!("mipmap-{density}"));
        write(
            &dir.join("ic_launcher.png"),
            &png(&render(
                launcher,
                &Layout::tile(launcher),
                machine_lead(),
                Badge::None,
            ))?,
        )?;
        write(
            &dir.join("ic_launcher_foreground.png"),
            &png(&render(
                layer,
                &Layout::adaptive_foreground(),
                machine_lead(),
                Badge::None,
            ))?,
        )?;
        write(
            &dir.join("ic_launcher_background.png"),
            &png(&render(
                layer,
                &Layout::adaptive_background(),
                machine_lead(),
                Badge::None,
            ))?,
        )?;
        written += 3;
    }

    // -- the package builder ----------------------------------------------------------------------
    //
    // The same drawing led by the accent blue. Three outputs, because the builder is a desktop tool
    // rather than an application on five platforms: it has no Debian package and no launcher, so a
    // 512-pixel loose PNG here would be a file with no reader. It does have a macOS bundle, which is
    // what the `.icns` below is for.

    for &size in BUILDER_TILE_SIZES {
        let path = icons.join(format!("km-package-builder-{size}.png"));
        write(
            &path,
            &png(&render(
                size,
                &Layout::tile(size),
                builder_lead(),
                Badge::None,
            ))?,
        )?;
        written += 1;
    }

    let members: Vec<RgbaImage> = ICO_SIZES
        .iter()
        .map(|&size| render(size, &Layout::tile(size), builder_lead(), Badge::None))
        .collect();
    write(&icons.join("km-package-builder.ico"), &ico(&members)?)?;
    written += 1;

    // macOS. The same small/large split as the machine's above, and for the same reason.
    //
    // `Info.package-builder.plist` names this file in three places -- `CFBundleIconFile`, and the
    // document type's `CFBundleTypeIconFile` and `UTTypeIconFile`, so a `.kmbuild` in the Finder
    // wears the mark of the program that opens it. The name without its extension is what those
    // three strings hold; renaming this file means changing them with it.
    let members: Vec<(&[u8; 4], RgbaImage)> = ICNS_MEMBERS
        .iter()
        .map(|&(kind, size)| {
            let layout = if size <= SMALL_UP_TO {
                Layout::tile(size)
            } else {
                Layout::inset()
            };
            (kind, render(size, &layout, builder_lead(), Badge::None))
        })
        .collect();
    write(&icons.join("km-package-builder.icns"), &icns(&members)?)?;
    written += 1;

    // -- the offline remote -----------------------------------------------------------------------
    //
    // The same drawing led by the second accent, green. No Debian package and so no `hicolor`
    // ladder, but a macOS bundle and an Android launcher entry, which is what the rest of this is
    // for.

    for &size in REMOTE_TILE_SIZES {
        let path = icons.join(format!("km-remote-{size}.png"));
        write(
            &path,
            &png(&render(
                size,
                &Layout::tile(size),
                remote_lead(),
                Badge::None,
            ))?,
        )?;
        written += 1;
    }

    let members: Vec<RgbaImage> = ICO_SIZES
        .iter()
        .map(|&size| render(size, &Layout::tile(size), remote_lead(), Badge::None))
        .collect();
    write(&icons.join("km-remote.ico"), &ico(&members)?)?;
    written += 1;

    // macOS. The same small/large split as the two above.
    //
    // `Info.remote.plist` names this file once, in `CFBundleIconFile`, and only once -- unlike the
    // builder's, which names it three times because it also dresses a document type. The remote
    // declares none.
    let members: Vec<(&[u8; 4], RgbaImage)> = ICNS_MEMBERS
        .iter()
        .map(|&(kind, size)| {
            let layout = if size <= SMALL_UP_TO {
                Layout::tile(size)
            } else {
                Layout::inset()
            };
            (kind, render(size, &layout, remote_lead(), Badge::None))
        })
        .collect();
    write(&icons.join("km-remote.icns"), &icns(&members)?)?;
    written += 1;

    // Android: the remote is an application on a phone, so it has a launcher entry and therefore
    // needs the same three files per density the machine has.
    //
    // Generated rather than drawn, for the reason this whole file exists: a hand-made PNG beside
    // three generated ones is exactly the drift one drawing in three palettes prevents.
    let remote_res = PathBuf::from(ANDROID_REMOTE_RES);
    for &(density, launcher, layer) in ANDROID_DENSITIES {
        let dir = remote_res.join(format!("mipmap-{density}"));
        write(
            &dir.join("ic_launcher.png"),
            &png(&render(
                launcher,
                &Layout::tile(launcher),
                remote_lead(),
                Badge::None,
            ))?,
        )?;
        write(
            &dir.join("ic_launcher_foreground.png"),
            &png(&render(
                layer,
                &Layout::adaptive_foreground(),
                remote_lead(),
                Badge::None,
            ))?,
        )?;
        write(
            &dir.join("ic_launcher_background.png"),
            &png(&render(
                layer,
                &Layout::adaptive_background(),
                remote_lead(),
                Badge::None,
            ))?,
        )?;
        written += 3;
    }

    // iOS, and it is one file where Android is fifteen. That is the platform being simpler rather
    // than this being incomplete: the system applies its own mask and resamples the one image for
    // every place it appears, so there is no layer split and no density ladder to fill. Xcode has
    // accepted a single-size universal `AppIcon` since 14.
    //
    // `Contents.json` beside it is committed by hand, on the precedent `mipmap-anydpi-v26/
    // ic_launcher.xml` already sets in this tree: it is a manifest, not a rendering, and generating
    // twelve lines that never change would want a JSON writer in an example that has none.
    write(
        &PathBuf::from(IOS_REMOTE_ASSETS).join("icon-1024.png"),
        &opaque_png(&render(1024, &Layout::ios(), remote_lead(), Badge::None))?,
    )?;
    written += 1;

    // The same drawing led by the magenta. A desktop program only: a window, a taskbar, an icon in
    // the bar and a macOS bundle, with no phone shell and no Debian package — so it takes the
    // remote's shape rather than the machine's, minus the Android and iOS halves.

    for &size in ADMIN_TILE_SIZES {
        let path = icons.join(format!("km-admin-{size}.png"));
        write(
            &path,
            &png(&render(
                size,
                &Layout::tile(size),
                admin_lead(),
                Badge::None,
            ))?,
        )?;
        written += 1;
    }

    let members: Vec<RgbaImage> = ICO_SIZES
        .iter()
        .map(|&size| render(size, &Layout::tile(size), admin_lead(), Badge::None))
        .collect();
    write(&icons.join("km-admin.ico"), &ico(&members)?)?;
    written += 1;

    // macOS. The same small/large split as the three above, and `Info.admin.plist` names this file
    // once — it declares no document type, as the remote does not.
    let members: Vec<(&[u8; 4], RgbaImage)> = ICNS_MEMBERS
        .iter()
        .map(|&(kind, size)| {
            let layout = if size <= SMALL_UP_TO {
                Layout::tile(size)
            } else {
                Layout::inset()
            };
            (kind, render(size, &layout, admin_lead(), Badge::None))
        })
        .collect();
    write(&icons.join("km-admin.icns"), &icns(&members)?)?;
    written += 1;

    // -- the machine, started streaming -----------------------------------------------------------
    //
    // **The machine's own palette with a badge on it, because this is the machine.** Every other
    // mark here is a program; this one is a way of starting one, and the four programs have already
    // taken the hues a fifth would have to come from. What it says is not *which program* but *which
    // launcher*, and a badge is the axis that says that.
    //
    // Three shapes and no Android, no iOS, no `hicolor` above 256 and no Plymouth logo: a phone
    // launcher starts no stream, and a boot splash is drawn before anything has been started at all.

    for &size in STREAM_TILE_SIZES {
        let path = icons.join(format!("karaokemachine-stream-{size}.png"));
        write(
            &path,
            &png(&render(
                size,
                &Layout::tile(size),
                machine_lead(),
                Badge::Stream,
            ))?,
        )?;
        written += 1;
    }

    // Windows. Read twice and by two different things: `build.rs` puts it in `karaokemachine.exe`
    // beside the plain mark, where the notification area finds it by ordinal for a `--stream` run,
    // and the installer stages the file itself so the Start Menu entry that passes `--stream` can
    // name it.
    let members: Vec<RgbaImage> = ICO_SIZES
        .iter()
        .map(|&size| render(size, &Layout::tile(size), machine_lead(), Badge::Stream))
        .collect();
    write(&icons.join("karaokemachine-stream.ico"), &ico(&members)?)?;
    written += 1;

    // macOS. The same small/large split as the four above. `Info.stream.plist` names this file, and
    // it is the whole of what `KM Stream.app` carries that the bundle beside it does
    // not.
    let members: Vec<(&[u8; 4], RgbaImage)> = ICNS_MEMBERS
        .iter()
        .map(|&(kind, size)| {
            let layout = if size <= SMALL_UP_TO {
                Layout::tile(size)
            } else {
                Layout::inset()
            };
            (kind, render(size, &layout, machine_lead(), Badge::Stream))
        })
        .collect();
    write(&icons.join("karaokemachine-stream.icns"), &icns(&members)?)?;
    written += 1;

    // The macOS menu bar, which takes neither of the two above: the letters with the tile taken away,
    // as a template image the system colors itself. That bar is a row of silhouettes, and a tile in
    // it is a dark object among glyphs.
    // `crates/machine/karaokemachine/src/tray.rs` compiles it in; nothing else reads it, and no
    // `.ico` accompanies it because the Windows notification area draws the colored mark and has no
    // such convention.
    write(
        &icons.join("karaokemachine-bar.png"),
        &png(&render_bar(BAR_ICON_HEIGHT))?,
    )?;
    written += 1;

    println!("\n{written} file(s)");
    Ok(())
}

/// Writes one file, making its directory first, and says so.
fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    println!("wrote {} ({} bytes)", path.display(), bytes.len());
    Ok(())
}
