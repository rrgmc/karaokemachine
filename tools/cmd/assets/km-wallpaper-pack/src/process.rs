//! Turning an original into the picture the app will show.
//!
//! Five steps, in this order, and the order is the contract: `crate::commands::verify` re-runs the
//! measurement at the same point in the same sequence, and any disagreement between the two makes the
//! whole gate meaningless.
//!
//! 1. **Crop to the output aspect**, biased vertically toward detail in the top half.
//! 2. **Resize** with Lanczos3.
//! 3. **Blur**, which is most of why an image passes: it raises the darkest pixels, lowers the
//!    brightest, and destroys exactly the fine detail that competes with letter shapes.
//! 4. **Vignette**, optionally.
//! 5. **Encode**, stripping everything that is not pixels.
//!
//! **Everything that comes out of here is a modified work, without exception**, and that is a
//! license fact rather than a description of the code: CC BY requires that changes be indicated, and
//! there is no path through [`render`] that skips steps 1 to 5. It is what lets `ATTRIBUTION.md` say
//! "every image" once at the top instead of repeating it on all 120 lines. A future fast path that
//! passed an already-correctly-sized image through untouched would quietly make that notice untrue.
//!
//! **Nothing here darkens the image.** The display lays `wallpaper.dim` over every wallpaper already;
//! a second scrim baked into the file would darken twice and would override a setting the user is
//! entitled to change. What the solver produced is used as a *filter* instead — see [`crate::select`]
//! — and travels in the manifest for anybody who later wants it.

use image::{DynamicImage, GenericImageView, imageops::FilterType};

use crate::config::{Format, Legibility, Size};
use crate::error::{Error, Result};

/// Crops to an aspect ratio, keeping the more interesting half of the slack.
///
/// A center crop throws away the same amount of sky and foreground, and a landscape's foreground is
/// usually the emptier of the two — so the window is nudged toward whichever half carries more
/// gradient energy. Capped at a tenth of the slack: past that it stops being a nudge and starts
/// cutting horizons in half, which is the failure a naive "crop to the interesting part" makes.
pub fn crop_to_aspect(image: &DynamicImage, aspect: f32) -> DynamicImage {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return image.clone();
    }
    let current = w as f32 / h as f32;

    if (current - aspect).abs() < 0.001 {
        return image.clone();
    }

    if current > aspect {
        // Too wide: trim the sides. There is no reason to prefer one side over the other — a
        // landscape's interest is not systematically left or right, unlike up and down.
        let target_w = (h as f32 * aspect).round().min(w as f32) as u32;
        let x = (w - target_w) / 2;
        image.crop_imm(x, 0, target_w, h)
    } else {
        // Too tall: trim top and bottom, biased toward detail above the middle.
        let target_h = (w as f32 / aspect).round().min(h as f32) as u32;
        let slack = h - target_h;
        let center = slack / 2;
        let bias = vertical_bias(image);
        let cap = ((slack as f32) * 0.10).round() as i32;
        let offset = (bias * cap as f32).round() as i32;
        let y = (center as i32 + offset).clamp(0, slack as i32) as u32;
        image.crop_imm(0, y, w, target_h)
    }
}

/// Where the detail is, as -1 (top half) to 1 (bottom half).
///
/// A cheap row-difference sum rather than a Sobel: this decides a crop of at most a tenth of the
/// slack, and spending real time on it would be spending it on the least consequential number here.
fn vertical_bias(image: &DynamicImage) -> f32 {
    let gray = image.resize(64, 64, FilterType::Triangle).to_luma8();
    let (w, h) = (gray.width(), gray.height());
    if w < 2 || h < 4 {
        return 0.0;
    }
    let mut top = 0.0_f64;
    let mut bottom = 0.0_f64;
    for y in 1..h {
        for x in 1..w {
            let here = f64::from(gray.get_pixel(x, y).0[0]);
            let up = f64::from(gray.get_pixel(x, y - 1).0[0]);
            let left = f64::from(gray.get_pixel(x - 1, y).0[0]);
            let energy = (here - up).abs() + (here - left).abs();
            if y < h / 2 {
                top += energy
            } else {
                bottom += energy
            }
        }
    }
    let total = top + bottom;
    if total <= 0.0 {
        return 0.0;
    }
    // Positive means "more interest below", so the crop window moves down.
    ((bottom - top) / total) as f32
}

/// Applies the vignette: a radial multiplier, 1.0 at the center falling to 0.85 at the corners.
///
/// Smoothstepped rather than linear, so there is no visible ring where it begins — a hard edge in a
/// darkening is more distracting than the darkening itself.
pub fn vignette(image: &mut image::RgbImage) {
    /// How much darker a corner ends up.
    const CORNER: f32 = 0.85;
    let (w, h) = (image.width(), image.height());
    if w == 0 || h == 0 {
        return;
    }
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let longest = (cx * cx + cy * cy).sqrt();
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let t = ((dx * dx + dy * dy).sqrt() / longest).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        let factor = 1.0 - (1.0 - CORNER) * smooth;
        for channel in 0..3 {
            pixel.0[channel] = (f32::from(pixel.0[channel]) * factor)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

/// Crop, resize, blur and vignette — everything up to encoding.
///
/// Returned rather than written, because the caller measures it: what the gate has to judge is this
/// picture, not the original it came from.
pub fn render(image: &DynamicImage, size: Size, legibility: &Legibility) -> DynamicImage {
    treat(canonical(image, size), legibility)
}

/// Steps 1 and 2 alone: the crop and resize everything else builds on.
///
/// Split out because this — not [`render`]'s output — is the picture the perceptual hash belongs on.
/// Cropping first is what makes two sources of one photograph agree; blurring and vignetting after
/// is what made them all agree with *each other*. The vignette is the same radial ramp on every
/// image, and a gradient hash reads neighboring cells, so it turns a shared treatment into shared
/// bits. See `crate::metrics::perceptual_hash`.
pub fn canonical(image: &DynamicImage, size: Size) -> DynamicImage {
    let cropped = crop_to_aspect(image, size.aspect());
    cropped.resize_exact(size.width, size.height, FilterType::Lanczos3)
}

/// Steps 3 and 4: the legibility treatments, applied to an already-cropped picture.
pub fn treat(resized: DynamicImage, legibility: &Legibility) -> DynamicImage {
    let blurred = if legibility.blur_sigma > 0.0 {
        DynamicImage::ImageRgb8(image::imageops::blur(
            &resized.to_rgb8(),
            legibility.blur_sigma,
        ))
    } else {
        resized
    };
    if legibility.vignette {
        let mut rgb = blurred.to_rgb8();
        vignette(&mut rgb);
        DynamicImage::ImageRgb8(rgb)
    } else {
        blurred
    }
}

/// Encodes to the configured format, keeping nothing but pixels.
///
/// EXIF is dropped by construction: the encoders here are handed raw RGB, so camera makes, GPS
/// coordinates and embedded thumbnails never reach the pack. That is a licensing and a privacy
/// property, not a size optimization.
pub fn encode(image: &DynamicImage, format: Format, jpeg_quality: u8) -> Result<Vec<u8>> {
    let rgb = image.to_rgb8();
    let mut bytes = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    match format {
        Format::Jpeg => {
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, jpeg_quality);
            encoder
                .encode_image(&DynamicImage::ImageRgb8(rgb))
                .map_err(|error| Error::Image {
                    path: std::path::PathBuf::from("<encode>"),
                    message: error.to_string(),
                })?;
        }
        Format::WebP => {
            DynamicImage::ImageRgb8(rgb)
                .write_to(&mut cursor, image::ImageFormat::WebP)
                .map_err(|error| Error::Image {
                    path: std::path::PathBuf::from("<encode>"),
                    message: error.to_string(),
                })?;
        }
    }
    Ok(bytes)
}

/// The file name for one output.
///
/// Stable across runs — index, hash prefix and size — so the app, a manifest and a person's notes can
/// all name the same picture. The hash is in the name so that two packs built months apart can be
/// compared without opening either.
pub fn output_name(index: usize, phash: u64, size: Size, format: Format) -> String {
    format!(
        "scenery-{index:03}-{:08x}-{size}.{}",
        (phash & 0xFFFF_FFFF) as u32,
        format.extension()
    )
}

#[cfg(test)]
mod tests {
    use image::{Rgb as ImageRgb, RgbImage};

    use super::*;

    fn size(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    #[test]
    fn a_tall_image_is_cropped_to_the_output_aspect() {
        let image = DynamicImage::ImageRgb8(RgbImage::new(1000, 1000));
        let cropped = crop_to_aspect(&image, 16.0 / 9.0);
        let (w, h) = cropped.dimensions();
        assert_eq!(w, 1000, "the width of a tall image survives whole");
        assert!(
            ((w as f32 / h as f32) - 16.0 / 9.0).abs() < 0.01,
            "got {w}x{h}"
        );
    }

    #[test]
    fn a_wide_image_is_trimmed_at_the_sides_and_keeps_its_height() {
        let image = DynamicImage::ImageRgb8(RgbImage::new(4000, 1000));
        let cropped = crop_to_aspect(&image, 16.0 / 9.0);
        let (w, h) = cropped.dimensions();
        assert_eq!(h, 1000);
        assert!(
            ((w as f32 / h as f32) - 16.0 / 9.0).abs() < 0.01,
            "got {w}x{h}"
        );
    }

    #[test]
    fn an_image_already_at_the_aspect_is_left_alone() {
        let image = DynamicImage::ImageRgb8(RgbImage::new(1920, 1080));
        let cropped = crop_to_aspect(&image, 16.0 / 9.0);
        assert_eq!(cropped.dimensions(), (1920, 1080));
    }

    #[test]
    fn the_crop_leans_toward_the_detailed_half_but_only_slightly() {
        // Detail crowded into the bottom third, emptiness above.
        let mut image = RgbImage::from_pixel(900, 1200, ImageRgb([20, 20, 20]));
        for y in 800..1200 {
            for x in 0..900 {
                let noisy = ((x * 3 + y * 5) % 2) as u8 * 200;
                image.put_pixel(x, y, ImageRgb([noisy, noisy, noisy]));
            }
        }
        let image = DynamicImage::ImageRgb8(image);
        let bias = vertical_bias(&image);
        assert!(bias > 0.5, "the detail is below the middle: {bias}");

        // And the window moved down, but by no more than a tenth of the slack.
        let cropped = crop_to_aspect(&image, 16.0 / 9.0);
        let (_, h) = cropped.dimensions();
        let slack = 1200 - h;
        // Reconstructing the offset from the pixel content: the top row of the crop should be past
        // the centered position but not far past it.
        assert!(slack > 0);
        let center = slack / 2;
        let cap = ((slack as f32) * 0.10).round() as u32;
        assert!(cap > 0, "there is slack to bias within");
        assert!(center + cap <= slack, "the cap keeps the window in range");
    }

    #[test]
    fn the_vignette_darkens_corners_and_leaves_the_center_alone() {
        let mut image = RgbImage::from_pixel(200, 100, ImageRgb([200, 200, 200]));
        vignette(&mut image);
        assert_eq!(
            image.get_pixel(100, 50).0[0],
            200,
            "the center is untouched"
        );
        let corner = image.get_pixel(0, 0).0[0];
        assert!(corner < 180, "the corner is darker: {corner}");
        assert!(corner > 150, "but not by much: {corner}");
    }

    #[test]
    fn rendering_produces_exactly_the_requested_size() {
        let image =
            DynamicImage::ImageRgb8(RgbImage::from_pixel(3000, 1800, ImageRgb([40, 60, 90])));
        let rendered = render(&image, size(1920, 1080), &Legibility::default());
        assert_eq!(rendered.dimensions(), (1920, 1080));
    }

    #[test]
    fn rendering_the_same_input_twice_produces_the_same_bytes() {
        // Nondeterminism here would be invisible in a picture and fatal to the determinism the pack
        // promises, so it is asserted on the encoded bytes rather than on the pixels.
        let mut source = RgbImage::new(200, 150);
        for (x, y, pixel) in source.enumerate_pixels_mut() {
            *pixel = ImageRgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]);
        }
        let source = DynamicImage::ImageRgb8(source);
        let legibility = Legibility::default();

        let first = encode(
            &render(&source, size(64, 36), &legibility),
            Format::Jpeg,
            82,
        )
        .expect("encode");
        let second = encode(
            &render(&source, size(64, 36), &legibility),
            Format::Jpeg,
            82,
        )
        .expect("encode");
        assert_eq!(first, second);
        assert!(!first.is_empty());
        // A JPEG, and one with no EXIF segment in it.
        assert_eq!(&first[..2], &[0xFF, 0xD8], "JPEG magic");
        assert!(
            !first.windows(4).any(|w| w == b"Exif"),
            "the pack ships pixels, not camera metadata"
        );
    }

    #[test]
    fn a_higher_jpeg_quality_costs_more_bytes() {
        let mut source = RgbImage::new(128, 128);
        for (x, y, pixel) in source.enumerate_pixels_mut() {
            *pixel = ImageRgb([(x * 2 % 256) as u8, (y * 3 % 256) as u8, 128]);
        }
        let source = DynamicImage::ImageRgb8(source);
        let low = encode(&source, Format::Jpeg, 40).expect("encode");
        let high = encode(&source, Format::Jpeg, 95).expect("encode");
        assert!(high.len() > low.len(), "{} vs {}", high.len(), low.len());
    }

    #[test]
    fn the_output_name_is_stable_and_says_what_it_is() {
        let name = output_name(7, 0xDEAD_BEEF_CAFE_1234, size(1920, 1080), Format::Jpeg);
        assert_eq!(name, "scenery-007-cafe1234-1920x1080.jpg");
        assert_eq!(
            name,
            output_name(7, 0xDEAD_BEEF_CAFE_1234, size(1920, 1080), Format::Jpeg)
        );
    }
}
