//! Generates gradient wallpapers.
//!
//!   cargo run -p km-display --example wallpapers
//!
//! Writes into `assets/wallpapers/`, which is where the machine looks. **The results are not
//! committed and are not the default set** — the machine ships seven CC0 photographs in
//! `default-wallpapers.zip`, built by `km-wallpaper-pack local` and credited in `CREDITS.md` beside
//! it. Regenerating produces byte-identical files.
//!
//! **This is kept, and it is not vestigial.** The gradients are the one wallpaper set that cannot
//! fail the contrast gate, because they are drawn against it rather than measured after the fact —
//! so this is what to reach for if a photograph ever has to be withdrawn, or if a build needs a set
//! that owes nothing to anybody. Its colors come from `Theme`, so they cannot drift from the app.
//!
//! **Why a generated default is a tempting answer.** A photograph needs a license, and a karaoke
//! wallpaper has an unusual job: it must be interesting enough to look at for three minutes and
//! plain enough that white lyrics with a dark outline stay readable over every part of it — which
//! reads like a job most photographs cannot do and free ones certainly cannot. Both halves of that
//! are wrong: a WCAG ratio measured in the lyric band decides the readability question by
//! measurement rather than by taste, and CC0 photographs may be redistributed outright. Of 103
//! candidates measured, 33 cleared the gate. See the `The wallpapers the machine ships` decision.
//!
//! **Everything here is deliberately dark.** The display already lays a 45% scrim over the wallpaper
//! (`WallpaperConfig::dim`), so these are not the last line of defense — but a bright wallpaper plus a
//! scrim is a gray wallpaper, which looks worse than starting dark. The brightest pixel any of these
//! produces is around 25% luminance.

use std::path::PathBuf;

use image::{Rgb, RgbImage};

/// 1080p. The display scales whatever it is given, and the loader downscales to the window, so this
/// is a source size rather than a target: large enough for a 4K panel to have something to work with,
/// small enough that four of them are a megabyte.
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// One wallpaper's color scheme.
struct Scheme {
    name: &'static str,
    /// Color at the brightest point of the wash.
    glow: [f32; 3],
    /// Color everywhere the wash does not reach.
    base: [f32; 3],
    /// Where the glow sits, in fractions of the image.
    center: (f32, f32),
    /// How far the glow spreads, as a fraction of the diagonal.
    spread: f32,
}

/// Four, because a cycle of one is a still image and a cycle of ten is a slideshow nobody asked for.
/// Distinct hues so the change is noticeable at a glance without being a distraction mid-song.
const SCHEMES: &[Scheme] = &[
    Scheme {
        name: "01-dusk",
        glow: [0.16, 0.19, 0.42],
        base: [0.02, 0.02, 0.06],
        center: (0.30, 0.22),
        spread: 0.85,
    },
    Scheme {
        name: "02-ember",
        glow: [0.34, 0.13, 0.08],
        base: [0.04, 0.02, 0.02],
        center: (0.72, 0.86),
        spread: 0.75,
    },
    Scheme {
        name: "03-aurora",
        glow: [0.06, 0.30, 0.26],
        base: [0.01, 0.04, 0.05],
        center: (0.20, 0.80),
        spread: 0.95,
    },
    Scheme {
        name: "04-slate",
        glow: [0.16, 0.18, 0.21],
        base: [0.03, 0.03, 0.04],
        center: (0.50, 0.10),
        spread: 1.10,
    },
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from("assets/wallpapers");
    std::fs::create_dir_all(&out)?;

    for scheme in SCHEMES {
        let image = render(scheme);
        let path = out.join(format!("{}.png", scheme.name));
        image.save(&path)?;
        println!("wrote {} ({WIDTH}x{HEIGHT})", path.display());
    }

    println!("\n{} wallpaper(s) in {}", SCHEMES.len(), out.display());
    Ok(())
}

/// Renders one scheme: a radial wash from `glow` to `base`, then a vignette, then a row dither.
fn render(scheme: &Scheme) -> RgbImage {
    let w = WIDTH as f32;
    let h = HEIGHT as f32;
    let diagonal = (w * w + h * h).sqrt();
    let cx = w * scheme.center.0;
    let cy = h * scheme.center.1;
    let reach = diagonal * scheme.spread;

    RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
        let fx = x as f32;
        let fy = y as f32;

        // The wash. Squared falloff rather than linear: linear leaves a visible edge where it reaches
        // zero, and the eye finds that edge immediately on a large flat panel.
        let distance = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() / reach;
        let wash = (1.0 - distance.clamp(0.0, 1.0)).powi(2);

        // A vignette on top, so the corners stay out of the way of anything drawn over them — the
        // connect panel sits bottom-right and the keypad bottom-left.
        let from_center =
            (((fx - w / 2.0) / (w / 2.0)).powi(2) + ((fy - h / 2.0) / (h / 2.0)).powi(2)).sqrt();
        let vignette = 1.0 - 0.45 * from_center.clamp(0.0, 1.4).powi(2);

        let channel = |index: usize| {
            let value = (scheme.base[index] + (scheme.glow[index] - scheme.base[index]) * wash)
                * vignette
                + dither(y);
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        };
        Rgb([channel(0), channel(1), channel(2)])
    })
}

/// A one-level dither, applied per **row**.
///
/// Eight-bit color cannot hold a gradient this shallow smoothly. Without any dither these images show
/// faint concentric contour rings — the sort of thing that reads as a rendering fault rather than a
/// background, and the 45% scrim makes it worse rather than better by compressing the range further.
///
/// Per row rather than per pixel, and that is the whole trick. A per-pixel dither works beautifully
/// and costs **seven times the file size**: it destroys the horizontal runs PNG's filters rely on, and
/// four wallpapers went from 1.0 MB to 7.5 MB. Offsetting whole rows leaves each row internally smooth,
/// so compression is barely affected, while alternating rows still breaks up contours anywhere they
/// are not perfectly horizontal — which, for a radial wash, is nearly everywhere.
///
/// Deterministic, so regenerating produces byte-identical files and re-running the example is not a
/// diff. A random dither would make every run one.
fn dither(y: u32) -> f32 {
    if y.is_multiple_of(2) {
        0.0
    } else {
        1.0 / 255.0
    }
}
