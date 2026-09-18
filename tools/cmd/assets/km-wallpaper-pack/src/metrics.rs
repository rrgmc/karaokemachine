//! Whether lyrics can be read over a photograph, measured rather than judged.
//!
//! Everything here answers one question: **at the darkening the app already applies, is white-ish
//! lyric text over this picture readable?** The answer is a WCAG contrast ratio, which is a defined
//! quantity, so a hundred wallpapers can be accepted or rejected without anybody squinting at them.
//!
//! Three things about the model are load-bearing, and each was a way to get it wrong:
//!
//! * **The compositing model and the luminance model must agree.** The app darkens by drawing black
//!   at `dim` alpha over the wallpaper, which multiplies the *sRGB* channels by `1 - alpha`.
//!   Relative luminance is defined on *linearised* channels. Darken in sRGB, measure in linear; do
//!   it the other way round and every answer is wrong in the direction of too light.
//! * **The text is not white.** `km_display::Theme::lyric_pending` is `#ECEFF4`, chosen because pure
//!   white glares over a bright wallpaper. Its relative luminance is about 0.84, so a solver that
//!   assumes 1.0 overstates every ratio by roughly a seventh.
//! * **The band is where the lyrics are, not where the picture is boring.** See
//!   [`crate::config::Legibility`] and `Theme::lyric_band` in `km-display`.
//!
//! The 3-pixel outline the display draws around every glyph is deliberately *not* modeled. It only
//! ever helps, so leaving it out makes the gate conservative — which is the right direction for a
//! measurement nobody will re-check by eye.

use image::{DynamicImage, GenericImageView, GrayImage, imageops::FilterType};

use crate::config::{Legibility, Rgb};

/// Longest side of the grayscale copy every statistic is computed on.
///
/// Small enough to be fast over thousands of images, large enough that a 512-pixel-wide Sobel still
/// sees the texture that matters. The statistics here — entropy, mean gradient, per-cell means — are
/// scale-stable, so measuring the downscale and measuring the original agree to within noise.
pub const ANALYSIS_SIDE: u32 = 512;

/// Cells the lyric band is divided into: 8 across, 3 down.
///
/// Per-cell rather than one mean over the whole band, because a mean hides the case that actually
/// ruins a line of text — a bright cloud behind three words with dark rock behind the rest. The
/// brightest cell is what the contrast solver is given.
pub const BAND_CELLS_X: u32 = 8;
/// Rows of cells down the band. See [`BAND_CELLS_X`].
pub const BAND_CELLS_Y: u32 = 3;

/// Everything measured about one candidate.
///
/// Serializable because it is memoised: [`crate::cache::Measurement`] keeps one of these per image in
/// `metrics.jsonl`, so re-tuning a filter threshold does not re-decode four thousand JPEGs. Every
/// field is an `f32`, which serde_json round-trips exactly through its shortest representation —
/// pinned by `metrics_survive_a_json_round_trip` below, because a cache that returned *nearly* the
/// same numbers would give a different pack depending on whether it happened to be warm.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImageMetrics {
    /// Mean relative luminance over the whole frame, 0..1.
    pub mean_luma: f32,
    /// Shannon entropy of the 256-bin luma histogram, in bits.
    pub entropy: f32,
    /// Mean Sobel magnitude inside the lyric band, normalized to 0..1.
    pub band_busyness: f32,
    /// Relative luminance of the brightest cell in the band — the worst case for light text.
    pub band_max_cell_luma: f32,
    /// The darkening needed to reach the target contrast, or `None` if no amount within the app's
    /// scrim would do it.
    pub required_alpha: Option<f32>,
    /// The contrast ratio actually achieved at the app's scrim, when one is reachable.
    ///
    /// `None` is the gate's verdict, and `crate::commands::verify` reads it as one. For the ratio
    /// itself, reachable or not, use [`ImageMetrics::contrast_at_assumed_dim`].
    pub measured_contrast: Option<f32>,
    /// The contrast ratio at the app's scrim, whether or not it clears the target.
    ///
    /// The same arithmetic as `measured_contrast` without the verdict attached, so a rejected image
    /// can still say how far short it fell. Derivable from the two fields either side of it only if
    /// you keep the brightest cell's *color*, which nothing downstream does — darkening happens in
    /// sRGB and luminance is defined on linearised channels, so a luminance cannot be re-darkened
    /// after the fact.
    pub contrast_at_assumed_dim: f32,
    /// The darkening this image would need with no cap at all.
    ///
    /// Recorded even — especially — when it exceeds `assumed_dim`, which is the entire point: an
    /// image rejected at 0.45 but solvable at 0.47 is a threshold question, and one solvable only at
    /// 0.90 is not. Without it `contrast_unreachable` is a verdict with no margin attached, and the
    /// only way to tune the gate is to change a number and re-run for forty minutes.
    ///
    /// `None` only when a full black scrim still cannot reach the target, which needs the text
    /// itself to be darker than the target demands.
    pub required_alpha_uncapped: Option<f32>,
}

/// sRGB channel to linear light, per WCAG 2.x.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an sRGB color whose channels are 0..1.
pub fn relative_luminance(color: Rgb) -> f32 {
    0.2126 * srgb_to_linear(color.r)
        + 0.7152 * srgb_to_linear(color.g)
        + 0.0722 * srgb_to_linear(color.b)
}

/// The WCAG contrast ratio between two relative luminances, either way round.
pub fn contrast_ratio(a: f32, b: f32) -> f32 {
    let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// One color with black composited over it at `alpha`, in sRGB space.
///
/// This is what an alpha-blended black rectangle does, which is exactly what the display draws — see
/// `draw::draw_background` in `km-display`.
pub fn darken(color: Rgb, alpha: f32) -> Rgb {
    let keep = 1.0 - alpha.clamp(0.0, 1.0);
    Rgb {
        r: color.r * keep,
        g: color.g * keep,
        b: color.b * keep,
    }
}

/// The contrast between `text` and `background` once the background is darkened by `alpha`.
pub fn contrast_at(background: Rgb, text: Rgb, alpha: f32) -> f32 {
    contrast_ratio(
        relative_luminance(darken(background, alpha)),
        relative_luminance(text),
    )
}

/// The smallest darkening that reaches `target` contrast, or `None` if `max_alpha` is not enough.
///
/// Solved numerically because relative luminance is not linear in the sRGB channel values, so there
/// is no closed form once the compositing happens in sRGB space. Twenty bisections over a bounded
/// interval settle it to about a millionth, which is far finer than anything downstream can see.
///
/// Monotonicity is what makes bisection valid here: darkening a background can only reduce its
/// luminance, and light text on a darker background can only gain contrast. A background *brighter*
/// than the text would invert that, which is why the early return checks the target at `max_alpha`
/// rather than assuming the ends of the interval bracket a root.
pub fn required_alpha(background: Rgb, text: Rgb, target: f32, max_alpha: f32) -> Option<f32> {
    if contrast_at(background, text, 0.0) >= target {
        return Some(0.0);
    }
    if contrast_at(background, text, max_alpha) < target {
        return None;
    }
    let mut low = 0.0_f32;
    let mut high = max_alpha;
    for _ in 0..20 {
        let mid = (low + high) / 2.0;
        if contrast_at(background, text, mid) >= target {
            high = mid;
        } else {
            low = mid;
        }
    }
    Some(high)
}

/// Measures one image against the legibility settings.
///
/// `image` is the picture as it will be *shown*: cropped, resized and blurred. Measuring anything
/// else is measuring a different picture — blur in particular raises the darkest pixels and lowers
/// the brightest, which is most of why it is applied at all. `crate::process` and
/// `crate::commands::verify` therefore call this at the same point in the same order.
///
/// **The perceptual hash is deliberately not taken here**, because it answers a different question
/// about a different picture. Legibility is a property of the treated image; identity is a property
/// of the photograph. Hashing the treated image made every candidate look alike — the vignette
/// stamps the same radial gradient on all of them, and a gradient hash reads exactly that. See
/// [`perceptual_hash`] and `crate::process::canonical`.
pub fn measure(image: &DynamicImage, legibility: &Legibility) -> ImageMetrics {
    let gray = grayscale_for_analysis(image);
    let (top, bottom) = legibility.band_rows(gray.height());

    let entropy = entropy_bits(&gray);
    let mean_luma = mean_luma(&gray);
    let band_busyness = band_busyness(&gray, top, bottom);

    // The brightest cell decides, because the whole line has to be readable and not merely its
    // average. Measured on the color image: two colors of equal grayscale value can differ in
    // relative luminance, and it is luminance the contrast ratio is defined on.
    let cells = band_cells(image, legibility);
    let brightest = cells
        .into_iter()
        .max_by(|a, b| relative_luminance(*a).total_cmp(&relative_luminance(*b)))
        .unwrap_or(Rgb::BLACK);

    let text = legibility.text_color;
    let required = required_alpha(
        brightest,
        text,
        legibility.target_contrast,
        legibility.assumed_dim,
    );
    let at_dim = contrast_at(brightest, text, legibility.assumed_dim);
    let measured = required.map(|_| at_dim);
    // The same solver with the cap lifted, so a rejection carries a margin rather than a verdict.
    let uncapped = required_alpha(brightest, text, legibility.target_contrast, 1.0);

    ImageMetrics {
        mean_luma,
        entropy,
        band_busyness,
        band_max_cell_luma: relative_luminance(brightest),
        required_alpha: required,
        measured_contrast: measured,
        contrast_at_assumed_dim: at_dim,
        required_alpha_uncapped: uncapped,
    }
}

/// The grayscale downscale every statistic is computed on.
pub fn grayscale_for_analysis(image: &DynamicImage) -> GrayImage {
    let (w, h) = image.dimensions();
    let longest = w.max(h);
    let scaled = if longest > ANALYSIS_SIDE {
        let factor = ANALYSIS_SIDE as f32 / longest as f32;
        image.resize(
            (w as f32 * factor).round().max(1.0) as u32,
            (h as f32 * factor).round().max(1.0) as u32,
            FilterType::Triangle,
        )
    } else {
        image.clone()
    };
    scaled.to_luma8()
}

/// Shannon entropy of the luma histogram, in bits.
///
/// Rejects both ends of the useless range at once: a smooth gradient has almost no entropy, and so
/// does a blown-out white frame. A photograph worth looking at for three minutes has several bits.
pub fn entropy_bits(gray: &GrayImage) -> f32 {
    let mut histogram = [0u32; 256];
    for pixel in gray.pixels() {
        histogram[pixel.0[0] as usize] += 1;
    }
    let total = gray.width() as f32 * gray.height() as f32;
    if total <= 0.0 {
        return 0.0;
    }
    -histogram
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let p = *count as f32 / total;
            p * p.log2()
        })
        .sum::<f32>()
}

/// Mean relative luminance, approximated from the grayscale copy.
fn mean_luma(gray: &GrayImage) -> f32 {
    let total: f64 = gray.pixels().map(|p| f64::from(p.0[0])).sum();
    let count = gray.width() as f64 * gray.height() as f64;
    if count <= 0.0 {
        return 0.0;
    }
    (total / count / 255.0) as f32
}

/// Mean Sobel gradient magnitude inside the band, normalized to 0..1.
///
/// Busyness is rejected separately from contrast because they fail differently. A calm bright sky
/// can be darkened into readability; a detailed hedge cannot, at any alpha — the letters compete
/// with edges of their own size, and the eye loses the word shapes even though the average
/// luminance is fine.
pub fn band_busyness(gray: &GrayImage, top: u32, bottom: u32) -> f32 {
    let (w, h) = (gray.width(), gray.height());
    if w < 3 || h < 3 {
        return 0.0;
    }
    let first = top.max(1);
    let last = bottom.min(h - 1);
    if last <= first {
        return 0.0;
    }

    let at = |x: u32, y: u32| f32::from(gray.get_pixel(x, y).0[0]) / 255.0;
    let mut total = 0.0_f32;
    let mut count = 0.0_f32;
    for y in first..last {
        for x in 1..w - 1 {
            let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
            let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
            // Divided by the largest magnitude a Sobel kernel can produce on 0..1 input, so the
            // number means the same thing at any bit depth and any scale.
            total += (gx * gx + gy * gy).sqrt() / (4.0 * std::f32::consts::SQRT_2);
            count += 1.0;
        }
    }
    if count == 0.0 { 0.0 } else { total / count }
}

/// Mean color of each cell of the lyric band.
pub fn band_cells(image: &DynamicImage, legibility: &Legibility) -> Vec<Rgb> {
    let rgb = image.to_rgb8();
    let (w, h) = rgb.dimensions();
    let (top, bottom) = legibility.band_rows(h);
    if w == 0 || bottom <= top {
        return Vec::new();
    }

    let mut cells = Vec::with_capacity((BAND_CELLS_X * BAND_CELLS_Y) as usize);
    let band_height = bottom - top;
    for cy in 0..BAND_CELLS_Y {
        for cx in 0..BAND_CELLS_X {
            let x0 = w * cx / BAND_CELLS_X;
            let x1 = (w * (cx + 1) / BAND_CELLS_X).max(x0 + 1).min(w);
            let y0 = top + band_height * cy / BAND_CELLS_Y;
            let y1 = (top + band_height * (cy + 1) / BAND_CELLS_Y)
                .max(y0 + 1)
                .min(h);

            let mut sum = (0.0_f64, 0.0_f64, 0.0_f64);
            let mut count = 0.0_f64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = rgb.get_pixel(x, y).0;
                    sum.0 += f64::from(p[0]);
                    sum.1 += f64::from(p[1]);
                    sum.2 += f64::from(p[2]);
                    count += 1.0;
                }
            }
            if count > 0.0 {
                cells.push(Rgb {
                    r: (sum.0 / count / 255.0) as f32,
                    g: (sum.1 / count / 255.0) as f32,
                    b: (sum.2 / count / 255.0) as f32,
                });
            }
        }
    }
    cells
}

/// Bits in a perceptual hash.
///
/// The `u64` the whole deduplication path passes around is exactly full, so nothing is truncated and
/// [`crate::config::Filters::phash_distance`] means what the near-duplicate literature means by it.
/// [`crate::config::Config::validate`] checks the configured distance against this.
pub const PHASH_BITS: usize = 64;

/// A 64-bit perceptual hash, for spotting the same photograph under a different search term.
///
/// **The size is load-bearing and was wrong.** `Gradient` at 8×8 resizes to 9×8 and emits `(9-1)*8`
/// = 64 comparisons — exactly a `u64`. The previous configuration, `DoubleGradient` at `(8, 4)`,
/// looked like "8×4 = 32, doubled = 64" and was nothing of the sort: `DoubleGradient` resizes to
/// `(W/2+1, H/2+1)`, so `(8, 4)` became a 5×3 grid emitting `(5-1)*3 + 5*(3-1)` = **22** bits, which
/// the `take(8)` below accepted in silence. A Hamming threshold of 10 over 22 bits is looser than
/// chance — two unrelated hashes sit 11 bits apart on average, and `P(distance ≤ 10)` is 0.42 — so
/// 1,608 legible photographs collapsed into 6 clusters and the pack shipped 6 wallpapers.
///
/// `DoubleGradient` cannot reach 64 at any size: `(w-1)*h + w*(h-1)` over that grid steps
/// 60 → 66 → 67 → 71 straight past it. Plain `Gradient` lands on it exactly, which is why the
/// algorithm changed along with the size.
pub fn perceptual_hash(image: &DynamicImage) -> u64 {
    let hasher = image_hasher::HasherConfig::new()
        .hash_alg(image_hasher::HashAlg::Gradient)
        .hash_size(8, 8)
        .to_hasher();
    let bytes = hasher.hash_image(image).as_bytes().to_vec();
    debug_assert_eq!(
        bytes.len() * 8,
        PHASH_BITS,
        "the hash must fill the u64 the deduplication path assumes"
    );
    let mut out = 0u64;
    for (index, byte) in bytes.iter().take(PHASH_BITS / 8).enumerate() {
        out |= u64::from(*byte) << (index * 8);
    }
    out
}

#[cfg(test)]
mod tests {
    use image::{Rgb as ImageRgb, RgbImage};

    use super::*;
    use crate::config::{Filters, Format, Legibility, Size};

    #[test]
    fn metrics_survive_a_json_round_trip() {
        // The measurement cache stores these as JSON and hands them back to the scorer as if they had
        // just been computed. If a single f32 came back one ulp out, a warm run and a cold run could
        // fall on opposite sides of a threshold and produce different packs — the exact failure the
        // cache exists to make impossible. serde_json writes the shortest string that round-trips;
        // this asserts it, rather than assuming it.
        let measured = measure(&photograph(7), &Legibility::default());
        let json = serde_json::to_string(&measured).expect("serialize");
        let back: ImageMetrics = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(measured, back, "a measurement must survive being cached");

        // Including the awkward values, which a real corpus produces and a synthetic photograph may
        // not: `None` where no scrim reaches the target, and a zero.
        let edges = ImageMetrics {
            mean_luma: 0.0,
            entropy: 6.123_456_7,
            band_busyness: f32::MIN_POSITIVE,
            band_max_cell_luma: 0.999_999_94,
            required_alpha: None,
            measured_contrast: None,
            contrast_at_assumed_dim: 1.0,
            required_alpha_uncapped: Some(0.1),
        };
        let json = serde_json::to_string(&edges).expect("serialize");
        assert_eq!(
            edges,
            serde_json::from_str::<ImageMetrics>(&json).expect("deserialize")
        );
    }

    /// A synthetic photograph: smooth, structured, and different for every seed.
    ///
    /// Deliberately not noise. A gradient hash compares the cells of a 9×8 downscale, and noise
    /// averages to flat gray at that scale — every seed would hash alike and the tests below would
    /// prove nothing about the hash while appearing to.
    fn photograph(seed: u32) -> DynamicImage {
        // Four low-frequency components with pseudo-random frequency, phase and weight. A plainer
        // `sin(seed * k)` generator aliases: two seeds land on the same picture and the tests below
        // then measure a distance of zero between "unrelated" photographs, which is a bug in the
        // fixture wearing the costume of a bug in the hash.
        let mut waves = [[0.0_f32; 4]; 4];
        let mut state = seed.wrapping_mul(2_654_435_761) | 1;
        for wave in &mut waves {
            for value in wave.iter_mut() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *value = state as f32 / u32::MAX as f32;
            }
        }

        let mut canvas = RgbImage::new(320, 180);
        for (x, y, pixel) in canvas.enumerate_pixels_mut() {
            let fx = x as f32 / 320.0;
            let fy = y as f32 / 180.0;
            let mut level = 0.5_f32;
            for wave in &waves {
                let across = wave[0] * 3.0 + 0.5;
                let down = wave[1] * 3.0 + 0.5;
                let phase = wave[2] * std::f32::consts::TAU;
                let weight = wave[3] * 0.18 + 0.06;
                level += weight * ((fx * across + fy * down) * std::f32::consts::TAU + phase).sin();
            }
            let level = (level * 255.0).clamp(0.0, 255.0) as u8;
            *pixel = ImageRgb([level, level.saturating_sub(12), level.saturating_add(9)]);
        }
        DynamicImage::ImageRgb8(canvas)
    }

    /// Mean pairwise Hamming distance over a set of hashes.
    fn mean_distance(hashes: &[u64]) -> f32 {
        let mut total = 0u32;
        let mut pairs = 0u32;
        for (index, one) in hashes.iter().enumerate() {
            for other in &hashes[index + 1..] {
                total += (one ^ other).count_ones();
                pairs += 1;
            }
        }
        total as f32 / pairs.max(1) as f32
    }

    fn flat(color: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 64, ImageRgb(color)))
    }

    #[test]
    fn the_luminance_model_matches_the_published_figures() {
        // The three WCAG reference points. Black is 0, white is 1, and mid gray is famously *not*
        // 0.5 — which is the whole reason the solver cannot be linear.
        assert!((relative_luminance(Rgb::BLACK) - 0.0).abs() < 1e-6);
        assert!((relative_luminance(Rgb::WHITE) - 1.0).abs() < 1e-6);
        let mid = relative_luminance(Rgb {
            r: 0.5,
            g: 0.5,
            b: 0.5,
        });
        assert!(
            (0.21..0.22).contains(&mid),
            "50% sRGB gray is about 0.214 relative luminance, got {mid}"
        );
        // White on black is the maximum the ratio can reach.
        assert!((contrast_ratio(1.0, 0.0) - 21.0).abs() < 0.01);
    }

    #[test]
    fn black_needs_no_darkening_and_white_can_never_be_darkened_enough() {
        let text = Rgb::from_hex("#ECEFF4").expect("hex");
        assert_eq!(required_alpha(Rgb::BLACK, text, 7.0, 0.45), Some(0.0));
        assert_eq!(
            required_alpha(Rgb::WHITE, text, 7.0, 0.45),
            None,
            "a white sky cannot be rescued by a 45% scrim"
        );
        // And with a scrim of nearly one, it can.
        assert!(required_alpha(Rgb::WHITE, text, 7.0, 0.99).is_some());
    }

    #[test]
    fn the_solved_alpha_is_the_smallest_one_that_works() {
        let text = Rgb::from_hex("#ECEFF4").expect("hex");
        // A mid-gray sky: needs some darkening, but not all of it.
        let sky = Rgb {
            r: 0.45,
            g: 0.55,
            b: 0.70,
        };
        let alpha = required_alpha(sky, text, 7.0, 0.95).expect("solvable");
        assert!(alpha > 0.0 && alpha < 0.95, "{alpha}");
        assert!(
            contrast_at(sky, text, alpha) >= 7.0,
            "the answer has to actually reach the target"
        );
        // Just below the answer must fail, which is what "smallest" means.
        assert!(contrast_at(sky, text, alpha - 0.01) < 7.0);
    }

    /// The property the whole gate rests on, over the color cube rather than one lucky sky.
    #[test]
    fn every_solvable_color_lands_just_above_the_target() {
        let text = Rgb::from_hex("#ECEFF4").expect("hex");
        let mut solved = 0;
        // A deterministic sweep rather than a random one: same coverage, and a failure names the
        // color that caused it instead of a seed.
        for r in 0..10 {
            for g in 0..10 {
                for b in 0..10 {
                    let color = Rgb {
                        r: r as f32 / 9.0,
                        g: g as f32 / 9.0,
                        b: b as f32 / 9.0,
                    };
                    if let Some(alpha) = required_alpha(color, text, 7.0, 1.0) {
                        solved += 1;
                        let got = contrast_at(color, text, alpha);
                        if alpha == 0.0 {
                            // Already dark enough unaided, so the answer is "no darkening" and the
                            // contrast can be anything up to 21:1.
                            assert!(got >= 7.0, "{color:?} passes unaided at {got}");
                        } else {
                            assert!(
                                (7.0..7.05).contains(&got),
                                "{color:?} solved to {alpha} giving {got}"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            solved > 500,
            "most of the cube should be solvable: {solved}"
        );
    }

    #[test]
    fn entropy_names_the_flat_and_the_busy_apart() {
        let flat = grayscale_for_analysis(&flat([40, 40, 40]));
        assert!(entropy_bits(&flat) < 0.01, "one value is zero bits");

        // A checkerboard of two values is exactly one bit, which is the arithmetic worth pinning.
        let mut board = RgbImage::new(64, 64);
        for (x, y, pixel) in board.enumerate_pixels_mut() {
            *pixel = if (x + y) % 2 == 0 {
                ImageRgb([0, 0, 0])
            } else {
                ImageRgb([255, 255, 255])
            };
        }
        let board = grayscale_for_analysis(&DynamicImage::ImageRgb8(board));
        let bits = entropy_bits(&board);
        assert!((bits - 1.0).abs() < 0.05, "expected one bit, got {bits}");
    }

    #[test]
    fn busyness_separates_a_calm_sky_from_a_hedge() {
        let legibility = Legibility::default();
        let calm = grayscale_for_analysis(&flat([90, 110, 140]));
        let (top, bottom) = legibility.band_rows(calm.height());
        assert!(band_busyness(&calm, top, bottom) < 0.01);

        // Detail at a few pixels' pitch, not one. A *single*-pixel checkerboard is invisible to a
        // Sobel kernel — the columns either side of any pixel are identical, so both gradients are
        // zero — which is a genuine blind spot and a fixture that would have proved nothing.
        let mut hedge = RgbImage::new(256, 256);
        for (x, y, pixel) in hedge.enumerate_pixels_mut() {
            let noisy = if (x / 3 + y / 3) % 2 == 0 { 20 } else { 220 };
            *pixel = ImageRgb([noisy, noisy, noisy]);
        }
        let hedge = grayscale_for_analysis(&DynamicImage::ImageRgb8(hedge));
        let busyness = band_busyness(&hedge, top, bottom);
        assert!(
            busyness > 0.1,
            "fine detail in the band is what makes lyrics unreadable: {busyness}"
        );
    }

    #[test]
    fn the_brightest_cell_decides_rather_than_the_average() {
        let legibility = Legibility::default();
        // Dark everywhere except a bright patch inside the band, one cell wide: on average this is a
        // dark image, and the words over that patch would be unreadable.
        let dark = RgbImage::from_pixel(320, 180, ImageRgb([10, 10, 12]));
        let mut patched = dark.clone();
        let (top, bottom) = legibility.band_rows(180);
        let cell = 320 / BAND_CELLS_X;
        for y in top..bottom {
            for x in 0..cell {
                patched.put_pixel(x, y, ImageRgb([250, 250, 250]));
            }
        }

        let plain = measure(&DynamicImage::ImageRgb8(dark), &legibility);
        assert_eq!(
            plain.required_alpha,
            Some(0.0),
            "the same frame without the patch needs no darkening at all"
        );

        let metrics = measure(&DynamicImage::ImageRgb8(patched), &legibility);
        assert!(
            metrics.mean_luma < 0.2,
            "the frame really is dark on average: {}",
            metrics.mean_luma
        );
        assert_eq!(
            metrics.required_alpha, None,
            "and it is still rejected, because of the patch"
        );
    }

    /// The bug that shipped a pack of six.
    ///
    /// `DoubleGradient` at `(8, 4)` emitted 22 bits into a `u64`, and nothing anywhere noticed —
    /// `dedupe`'s tests all hand-write 64-bit literals, so they exercised a bit space the real
    /// hasher never produced.
    #[test]
    fn the_perceptual_hash_uses_all_sixty_four_bits() {
        let hashes: Vec<u64> = (0..24)
            .map(|seed| perceptual_hash(&photograph(seed)))
            .collect();
        let union = hashes.iter().fold(0u64, |all, hash| all | hash);
        assert!(
            union > (1 << 22),
            "the hash never sets a bit above 21, so it is not 64 bits wide: {union:#018x}"
        );
        assert!(
            union.count_ones() >= 56,
            "only {} of {PHASH_BITS} bits are ever set ({union:#018x}); the hash is narrower \
             than the u64 that carries it",
            union.count_ones()
        );
    }

    /// Why the width matters, stated as the property the threshold depends on.
    ///
    /// `phash_distance` is a Hamming distance, so it is only meaningful against the width of the
    /// hash. Unrelated hashes sit half the width apart on average: 32 bits for a real 64-bit hash,
    /// and 11 for the 22-bit one this replaced — under which a threshold of 10 matched 42% of
    /// unrelated photographs and collapsed 1,608 candidates into 6 clusters.
    #[test]
    fn unrelated_photographs_sit_far_apart_in_hash_space() {
        let size = Size {
            width: 320,
            height: 180,
        };
        // Hashed the way `analyze` hashes: the cropped and resized picture, before any treatment.
        let hashes: Vec<u64> = (0..16)
            .map(|seed| perceptual_hash(&crate::process::canonical(&photograph(seed), size)))
            .collect();

        let threshold = Filters::default().phash_distance;
        let closest = hashes
            .iter()
            .enumerate()
            .flat_map(|(index, one)| {
                hashes[index + 1..]
                    .iter()
                    .map(move |other| (one ^ other).count_ones())
            })
            .min()
            .expect("more than one photograph");
        assert!(
            closest > threshold,
            "two unrelated photographs are {closest} bits apart, at or under the {threshold} that \
             means 'the same photograph'"
        );

        let mean = mean_distance(&hashes);
        assert!(
            mean > 20.0,
            "unrelated hashes average {mean:.1} bits apart; a 64-bit hash should sit near 32, and \
             anything near 11 is the 22-bit hash back again"
        );
    }

    /// The other half of the property: strictness must not be bought by matching nothing.
    #[test]
    fn the_same_photograph_survives_a_re_encoding() {
        let original = photograph(7);
        // What a second stock provider serves: the same picture at another size, through JPEG.
        let resized = original.resize_exact(256, 144, FilterType::Lanczos3);
        let jpeg = crate::process::encode(&resized, Format::Jpeg, 60).expect("encodes");
        let decoded = image::load_from_memory(&jpeg).expect("decodes");

        let distance = (perceptual_hash(&original) ^ perceptual_hash(&decoded)).count_ones();
        let threshold = Filters::default().phash_distance;
        assert!(
            distance <= threshold,
            "one photograph re-encoded hashes {distance} bits away, past the {threshold} that would \
             cluster it: the pack would ship the same picture twice"
        );
    }

    /// Why the hash is taken before `process::treat` rather than after.
    ///
    /// The vignette lays the same radial ramp over every candidate and the blur removes the fine
    /// detail that distinguishes them. A gradient hash reads neighboring cells, so a shared
    /// treatment becomes shared bits — the pack's candidates were all dark, all blurred, all
    /// vignetted and all 16:9, and they hashed alike for reasons that had nothing to do with what
    /// they were photographs of.
    #[test]
    fn treating_a_photograph_before_hashing_it_makes_them_all_alike() {
        let size = Size {
            width: 320,
            height: 180,
        };
        let legibility = Legibility::default();
        let canonical: Vec<u64> = (0..12)
            .map(|seed| perceptual_hash(&crate::process::canonical(&photograph(seed), size)))
            .collect();
        let treated: Vec<u64> = (0..12)
            .map(|seed| {
                let ready = crate::process::canonical(&photograph(seed), size);
                perceptual_hash(&crate::process::treat(ready, &legibility))
            })
            .collect();

        let before = mean_distance(&canonical);
        let after = mean_distance(&treated);
        assert!(
            before > after,
            "hashing after the blur and vignette should spread photographs less, not more: \
             {before:.1} bits before, {after:.1} after. If this ever inverts, the reason \
             `canonical` is split out of `render` has gone away."
        );
    }

    #[test]
    fn a_dark_photograph_passes_and_reports_the_contrast_it_will_be_read_at() {
        let legibility = Legibility::default();
        let metrics = measure(&flat([30, 34, 44]), &legibility);
        assert_eq!(metrics.required_alpha, Some(0.0), "no scrim needed at all");
        let measured = metrics.measured_contrast.expect("reachable");
        assert!(
            measured > legibility.target_contrast,
            "the app's own scrim only improves it: {measured}"
        );
    }
}
