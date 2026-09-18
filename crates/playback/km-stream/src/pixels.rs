//! Turning a drawn screen into the planes an encoder wants.
//!
//! **This exists because swscale is switched off by build decision.** `tools/setup/ffmpeg-pin.sh`
//! configures `--disable-swscale` along with avdevice, avfilter and postproc, those four being the
//! sublibraries that reach for ffmpeg's optional external dependencies — so there is no library call
//! here that converts a packed picture into planes, and this is the conversion written out.
//!
//! It costs nothing to keep it that way, because the screen is drawn at the size it is encoded at.
//! Scaling is what swscale would really have been wanted for, and there is none to do.
//!
//! # BT.709, limited range
//!
//! The matrix is Rec. 709 and the levels are limited range — luma 16 to 235, chroma 16 to 240 —
//! which is what a television expects and what an H.264 stream is read as when it says nothing. The
//! encoder is told the same thing, so the two cannot disagree: a full-range picture tagged limited
//! comes out washed, and the reverse comes out crushed, and neither looks like a conversion
//! mistake.

/// Fixed-point fraction bits. Sixteen leaves every coefficient below exact to under a part in a
/// thousand, which is far inside a single step of an 8-bit channel.
const SHIFT: i32 = 16;
const ONE: i32 = 1 << SHIFT;

/// Rounds a BT.709 coefficient, already scaled for limited range, into fixed point.
const fn k(numerator: f64) -> i32 {
    (numerator * ONE as f64 + 0.5) as i32
}

// Luma, on the 219-step limited range scale.
const Y_R: i32 = k(0.182_586);
const Y_G: i32 = k(0.614_231);
const Y_B: i32 = k(0.062_007);

// Chroma, on the 224-step scale. Blue and red each carry exactly half their own difference, which
// is why `U_B` and `V_R` are the same number.
const U_R: i32 = k(-0.100_644);
const U_G: i32 = k(-0.338_572);
const U_B: i32 = k(0.439_216);
const V_R: i32 = k(0.439_216);
const V_G: i32 = k(-0.398_942);
const V_B: i32 = k(-0.040_274);

/// The floor limited-range luma sits on.
const Y_OFFSET: i32 = 16 << SHIFT;
/// Neutral chroma.
const C_OFFSET: i32 = 128 << SHIFT;
/// Rounding, added before the shift back down.
const HALF: i32 = ONE / 2;

/// Bytes per pixel in the packed source.
const BGRA: usize = 4;

/// One plane of the destination, and how far apart its rows are.
///
/// **A stride rather than a plain width**, because a decoder's planes are padded for alignment and
/// an encoder's frame is allocated the same way. Writing rows at `width` into a buffer whose rows
/// are `width + padding` apart shears the picture diagonally, which looks like a corrupt decoder
/// rather than a wrong number.
pub struct Plane<'a> {
    /// Where the plane's bytes go.
    pub bytes: &'a mut [u8],
    /// Distance in bytes from one row to the next.
    pub stride: usize,
}

/// Converts one BGRA screen into planar YUV 4:2:0.
///
/// `source` is the drawn screen in memory order — blue, green, red, alpha — which is what an
/// `ARGB8888` SDL surface holds on a little-endian machine.
///
/// **Width and height must be even.** H.264 in 4:2:0 has no way to represent an odd edge, so a size
/// that could not be encoded is refused here rather than half-converted: every chroma sample stands
/// for exactly one 2x2 block of pixels, and a trailing row or column would have no block to average.
///
/// # Panics
///
/// If the dimensions are odd, or if any buffer is too small for the size claimed.
pub fn bgra_to_yuv420p(
    source: &[u8],
    width: usize,
    height: usize,
    luma: &mut Plane<'_>,
    blue: &mut Plane<'_>,
    red: &mut Plane<'_>,
) {
    assert!(
        width.is_multiple_of(2) && height.is_multiple_of(2),
        "4:2:0 needs even dimensions; {width}x{height} has no chroma block for its last edge"
    );
    assert!(
        source.len() >= width * height * BGRA,
        "the screen is smaller than the {width}x{height} claimed for it"
    );

    // Two rows at a time: a chroma sample is one 2x2 block, so the pass that averages it wants both
    // of its rows in hand. Luma is written for all four pixels on the way past, which is what keeps
    // this one read of the source rather than two.
    for row in (0..height).step_by(2) {
        let top = row * width * BGRA;
        let bottom = top + width * BGRA;
        let luma_top = row * luma.stride;
        let luma_bottom = luma_top + luma.stride;
        let chroma_row = (row / 2) * blue.stride;

        for column in (0..width).step_by(2) {
            let left = column * BGRA;
            let right = left + BGRA;

            let pixels = [
                &source[top + left..top + left + BGRA],
                &source[top + right..top + right + BGRA],
                &source[bottom + left..bottom + left + BGRA],
                &source[bottom + right..bottom + right + BGRA],
            ];

            luma.bytes[luma_top + column] = luma_of(pixels[0]);
            luma.bytes[luma_top + column + 1] = luma_of(pixels[1]);
            luma.bytes[luma_bottom + column] = luma_of(pixels[2]);
            luma.bytes[luma_bottom + column + 1] = luma_of(pixels[3]);

            // Averaged before the matrix rather than after it. The matrix is linear, so the two
            // orders agree to within rounding, and averaging first is one matrix instead of four.
            let mut sum = [0i32; 3];
            for pixel in pixels {
                sum[0] += i32::from(pixel[2]);
                sum[1] += i32::from(pixel[1]);
                sum[2] += i32::from(pixel[0]);
            }
            let (r, g, b) = ((sum[0] + 2) / 4, (sum[1] + 2) / 4, (sum[2] + 2) / 4);

            let column_of_chroma = column / 2;
            blue.bytes[chroma_row + column_of_chroma] =
                clamp((U_R * r + U_G * g + U_B * b + C_OFFSET + HALF) >> SHIFT);
            red.bytes[chroma_row + column_of_chroma] =
                clamp((V_R * r + V_G * g + V_B * b + C_OFFSET + HALF) >> SHIFT);
        }
    }
}

/// The luma of one BGRA pixel.
fn luma_of(pixel: &[u8]) -> u8 {
    let (b, g, r) = (
        i32::from(pixel[0]),
        i32::from(pixel[1]),
        i32::from(pixel[2]),
    );
    clamp((Y_R * r + Y_G * g + Y_B * b + Y_OFFSET + HALF) >> SHIFT)
}

// ...and back the other way, for a picture that arrives already in planes.
//
// Coefficients are the inverse of the matrix above, with the limited range expanded back out.

/// Luma, expanded from 219 steps to 255.
const R_Y: i32 = k(1.164_384);
const R_V: i32 = k(1.792_741);
const G_U: i32 = k(-0.213_249);
const G_V: i32 = k(-0.532_909);
const B_U: i32 = k(2.112_402);

/// Bytes per pixel in the packed destination.
const RGBA: usize = 4;

/// One plane being read, and how far apart its rows are.
///
/// [`Plane`]'s counterpart for the other direction. A decoder hands out its planes by shared
/// reference — several things may be looking at one frame — so a writable plane cannot name them.
pub struct Source<'a> {
    /// The plane's bytes.
    pub bytes: &'a [u8],
    /// Distance in bytes from one row to the next.
    pub stride: usize,
}

/// Converts planar YUV 4:2:0 into packed RGBA.
///
/// **This is the direction a video song needs.** A decoded picture arrives in planes, and the
/// compositor draws in RGB — so a song whose picture goes behind the words is converted here, drawn
/// under them, and converted back by [`bgra_to_yuv420p`] on the way to the encoder.
///
/// **Rec. 709 for every picture, which is right for nearly all of them and not for all.** The
/// packaging profile fixes what a song may hold at H.264 4:2:0 no larger than 1080p, and an encoder
/// producing that tags Rec. 709 unless it was given standard-definition material to start from. A
/// frame that was really Rec. 601 comes out slightly off in saturation rather than wrong, and
/// `km_video::Frame` carries no colour metadata to tell one from the other.
///
/// `out` is filled with `width * height` pixels in red, green, blue, alpha order, alpha always
/// opaque.
///
/// # Panics
///
/// If the dimensions are odd, or if `out` is too small for the size claimed.
pub fn yuv420p_to_rgba(
    luma: &Source<'_>,
    blue: &Source<'_>,
    red: &Source<'_>,
    width: usize,
    height: usize,
    out: &mut [u8],
) {
    assert!(
        width.is_multiple_of(2) && height.is_multiple_of(2),
        "4:2:0 needs even dimensions; {width}x{height} has a chroma sample standing for half a block"
    );
    assert!(
        out.len() >= width * height * RGBA,
        "the destination is smaller than the {width}x{height} claimed for it"
    );

    for row in 0..height {
        let luma_row = row * luma.stride;
        let chroma_row = (row / 2) * blue.stride;
        for column in 0..width {
            // One chroma sample per 2x2 block, repeated rather than interpolated. Interpolating
            // would be sharper on a gradient and costs a second pass over both planes; what is
            // behind the words here is a moving picture at a television's size, where the
            // difference is not one anybody watching can see.
            let chroma = column / 2;
            let y = (i32::from(luma.bytes[luma_row + column]) - 16) * R_Y;
            let u = i32::from(blue.bytes[chroma_row + chroma]) - 128;
            let v = i32::from(red.bytes[chroma_row + chroma]) - 128;

            let at = (row * width + column) * RGBA;
            out[at] = clamp((y + R_V * v + HALF) >> SHIFT);
            out[at + 1] = clamp((y + G_U * u + G_V * v + HALF) >> SHIFT);
            out[at + 2] = clamp((y + B_U * u + HALF) >> SHIFT);
            out[at + 3] = 255;
        }
    }
}

/// Holds a converted sample inside what a byte can carry.
///
/// Limited range leaves headroom at both ends, so ordinary colours never reach a clamp. It is here
/// for the ones that do: a fully saturated primary lands a fraction outside, and wrapping a byte
/// would turn the brightest red in a wallpaper into a dark cyan speck.
fn clamp(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the three planes for a `width` x `height` picture, tightly packed.
    fn planes(width: usize, height: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            vec![0u8; width * height],
            vec![0u8; width * height / 4],
            vec![0u8; width * height / 4],
        )
    }

    /// Converts a picture of one repeated colour and hands back the sample each plane holds.
    fn one_colour(b: u8, g: u8, r: u8) -> (u8, u8, u8) {
        let (width, height) = (2, 2);
        let source: Vec<u8> = [b, g, r, 255].repeat(width * height);
        let (mut y, mut u, mut v) = planes(width, height);
        bgra_to_yuv420p(
            &source,
            width,
            height,
            &mut Plane {
                bytes: &mut y,
                stride: width,
            },
            &mut Plane {
                bytes: &mut u,
                stride: width / 2,
            },
            &mut Plane {
                bytes: &mut v,
                stride: width / 2,
            },
        );
        (y[0], u[0], v[0])
    }

    /// Black and white land on the limited-range floor and ceiling, with neutral chroma.
    ///
    /// These two are the whole of the range agreement with the encoder. A full-range conversion
    /// would put black at 0 and white at 255 here, the picture would be tagged limited anyway, and
    /// every television would show it with crushed shadows.
    #[test]
    fn black_and_white_sit_on_the_limited_range_ends() {
        assert_eq!(one_colour(0, 0, 0), (16, 128, 128), "black");
        assert_eq!(one_colour(255, 255, 255), (235, 128, 128), "white");
    }

    /// Grey of any level keeps chroma neutral, which is what says the matrix rows sum correctly.
    ///
    /// A coefficient mistyped in any of the six chroma constants shows up here, where it would be
    /// invisible in a picture: a small chroma bias on a grey screen reads as a warm or cool display
    /// rather than as a bug.
    #[test]
    fn grey_is_colourless_at_every_level() {
        for level in [32u8, 64, 96, 128, 160, 192, 224] {
            let (_, u, v) = one_colour(level, level, level);
            assert_eq!((u, v), (128, 128), "grey at {level} picked up a colour");
        }
    }

    /// Blue and red pull the plane each is named for, and pull it the right way.
    ///
    /// This is what catches the conversion reading its source as RGBA: the planes would still be
    /// full of plausible numbers, and the picture would come out with red and blue exchanged.
    ///
    /// The two bars differ because the matrix is lopsided rather than because one case is weaker.
    /// A primary carries exactly half of its *own* difference, so it drives that plane to the top of
    /// the range; what it does to the other plane is only the small cross-term — ten steps for blue
    /// on red — so the far side is asserted as "below neutral" and nothing stronger.
    #[test]
    fn each_primary_pulls_its_own_plane() {
        let (_, u, v) = one_colour(255, 0, 0);
        assert!(
            u > 200,
            "blue must raise the blue-difference plane, got {u}"
        );
        assert!(v < 128, "blue must lower the red-difference plane, got {v}");

        let (_, u, v) = one_colour(0, 0, 255);
        assert!(v > 200, "red must raise the red-difference plane, got {v}");
        assert!(u < 128, "red must lower the blue-difference plane, got {u}");
    }

    /// Green is the brightest primary and blue the dimmest, which is Rec. 709 showing through.
    ///
    /// Pins the luma row in the order a person can check by eye, so a transposed pair of
    /// coefficients cannot pass.
    #[test]
    fn luma_weights_green_most_and_blue_least() {
        let green = one_colour(0, 255, 0).0;
        let red = one_colour(0, 0, 255).0;
        let blue = one_colour(255, 0, 0).0;
        assert!(
            green > red && red > blue,
            "Rec. 709 orders the primaries green {green}, red {red}, blue {blue}"
        );
    }

    /// A padded plane is written row by row, not as one run.
    ///
    /// A stride ignored would shear the picture diagonally, which reads as a broken decoder rather
    /// than as a wrong number here.
    #[test]
    fn rows_land_on_their_stride() {
        let (width, height) = (2, 2);
        let source: Vec<u8> = [0, 0, 0, 255].repeat(width * height);
        let padding = 5;
        let mut y = vec![0xAAu8; (width + padding) * height];
        let (mut u, mut v) = (vec![0xAAu8; 1 + padding], vec![0xAAu8; 1 + padding]);
        bgra_to_yuv420p(
            &source,
            width,
            height,
            &mut Plane {
                bytes: &mut y,
                stride: width + padding,
            },
            &mut Plane {
                bytes: &mut u,
                stride: 1 + padding,
            },
            &mut Plane {
                bytes: &mut v,
                stride: 1 + padding,
            },
        );
        assert_eq!(&y[..2], &[16, 16], "the first row");
        assert_eq!(
            &y[width..width + padding],
            &[0xAA; 5],
            "padding must be left alone"
        );
        assert_eq!(
            &y[width + padding..width + padding + 2],
            &[16, 16],
            "the second row starts one stride in"
        );
    }

    /// A screen survives the round trip to planes and back.
    ///
    /// **Within a tolerance, because 4:2:0 is lossy by construction**: four pixels share one chroma
    /// sample, so a colour that differs across a 2x2 block cannot come back. A flat colour has no
    /// such loss and is what this uses, which leaves the matrix itself as the only thing that can
    /// move — and a matrix that is its own inverse is the property both directions rest on.
    #[test]
    fn a_flat_colour_survives_the_round_trip() {
        for colour in [
            [0u8, 0, 0],
            [255, 255, 255],
            [128, 128, 128],
            [200, 30, 60],
            [12, 190, 240],
        ] {
            let (width, height) = (4, 4);
            let source: Vec<u8> = [colour[0], colour[1], colour[2], 255].repeat(width * height);
            let (mut y, mut u, mut v) = planes(width, height);
            bgra_to_yuv420p(
                &source,
                width,
                height,
                &mut Plane {
                    bytes: &mut y,
                    stride: width,
                },
                &mut Plane {
                    bytes: &mut u,
                    stride: width / 2,
                },
                &mut Plane {
                    bytes: &mut v,
                    stride: width / 2,
                },
            );
            let mut back = vec![0u8; width * height * RGBA];
            yuv420p_to_rgba(
                &Source {
                    bytes: &y,
                    stride: width,
                },
                &Source {
                    bytes: &u,
                    stride: width / 2,
                },
                &Source {
                    bytes: &v,
                    stride: width / 2,
                },
                width,
                height,
                &mut back,
            );
            // The source is BGRA and the result is RGBA, so the comparison is the swap.
            let (b, g, r) = (colour[0], colour[1], colour[2]);
            for (index, expected) in [r, g, b].into_iter().enumerate() {
                let got = back[index];
                assert!(
                    got.abs_diff(expected) <= 2,
                    "channel {index} of {colour:?} came back {got}, wanted about {expected}"
                );
            }
            assert_eq!(back[3], 255, "alpha is opaque");
        }
    }

    /// An odd size is refused rather than half-converted.
    #[test]
    #[should_panic(expected = "4:2:0 needs even dimensions")]
    fn an_odd_size_is_refused() {
        let source = vec![0u8; 3 * 3 * BGRA];
        let (mut y, mut u, mut v) = (vec![0u8; 9], vec![0u8; 4], vec![0u8; 4]);
        bgra_to_yuv420p(
            &source,
            3,
            3,
            &mut Plane {
                bytes: &mut y,
                stride: 3,
            },
            &mut Plane {
                bytes: &mut u,
                stride: 2,
            },
            &mut Plane {
                bytes: &mut v,
                stride: 2,
            },
        );
    }
}
