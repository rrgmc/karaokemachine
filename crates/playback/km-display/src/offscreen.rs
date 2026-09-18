//! Drawing a frame without a window.
//!
//! The display normally draws into a window on a television. This renders exactly the same
//! [`Frame`] into an off-screen surface, which is what makes three things possible that a windowed
//! renderer cannot do:
//!
//! * **Reviewing a layout in a pull request.** `examples/preview.rs` writes a contact sheet of the
//!   states that are easy to get wrong, so a lyric row colliding with the transport strip is caught
//!   by looking rather than by sitting in front of the machine.
//! * **The README's pictures.** `examples/screenshots.rs` writes the handful of images the
//!   repository actually publishes.
//! * **A machine whose screen is a stream**, where every frame is drawn here and handed to an
//!   encoder rather than to a television.
//!
//! It lives here rather than once in each of them, so they cannot drift apart and so "render a
//! frame with no screen attached" is a capability of the crate with a test behind it rather than a
//! trick inside an example.
//!
//! It needs no video subsystem: `SDL_Init` is never asked for one, and the software renderer draws
//! into a plain surface. That is why it runs where CI does, and over SSH on a box with no monitor.
//!
//! # Two entry points, and which to reach for
//!
//! [`render_to_image`] draws one frame and hands back an image. Everything it needs — a surface, a
//! canvas, a texture creator, a text cache — is built for that frame and dropped with it, which is
//! right for a picture somebody asked for and wrong for a stream: a cache that lives one frame is a
//! cache of nothing, and rendered text is the largest single cost in drawing a screen.
//!
//! [`Offscreen`] keeps all four across frames and is what a stream drives. The picture behind the
//! frame is uploaded by [`Offscreen::set_picture`] rather than passed to each draw, because only the
//! caller knows whether it has changed — a wallpaper stands for minutes while a video's frames are
//! recycled buffers that hold new pixels at the same address.

use image::RgbaImage;
use sdl3::pixels::PixelFormat;
use sdl3::render::{Canvas, Texture};
use sdl3::surface::{Surface, SurfaceContext};

use crate::draw::{Background, Frame, draw};
use crate::text::{Fonts, TextCache};
use crate::theme::Theme;

/// Why an off-screen render could not be produced.
///
/// Every variant is an SDL failure carrying SDL's own message. They are separated rather than
/// collapsed into one string because the *stage* is the useful part when one happens: a surface
/// that cannot be created is a build or platform problem, whereas a readback that fails after a
/// successful draw is a renderer problem.
#[derive(Debug)]
pub enum OffscreenError {
    /// The off-screen surface or its canvas could not be created.
    Surface(String),
    /// The wallpaper could not be turned into a texture.
    Wallpaper(String),
    /// The drawn pixels could not be read back.
    Readback(String),
    /// The pixels came back in a size that does not match the surface.
    SizeMismatch {
        /// Width the readback reported.
        width: u32,
        /// Height the readback reported.
        height: u32,
        /// How many bytes actually arrived.
        len: usize,
    },
}

impl std::fmt::Display for OffscreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Surface(why) => write!(f, "creating the off-screen surface: {why}"),
            Self::Wallpaper(why) => write!(f, "uploading the wallpaper: {why}"),
            Self::Readback(why) => write!(f, "reading the rendered pixels back: {why}"),
            Self::SizeMismatch { width, height, len } => write!(
                f,
                "the pixel buffer ({len} bytes) does not match the {width}x{height} surface"
            ),
        }
    }
}

impl std::error::Error for OffscreenError {}

/// ARGB8888 little-endian, which is BGRA in memory order.
///
/// Named once because both directions below depend on it and getting it wrong swaps red and blue —
/// a bug that looks like a theme change rather than a byte-order mistake.
fn argb8888() -> Result<PixelFormat, OffscreenError> {
    crate::text::argb8888()
        .ok_or_else(|| OffscreenError::Surface("ARGB8888 is not a format SDL knows".to_owned()))
}

/// What goes behind the frame.
///
/// **One fact rather than three loose arguments**, grouped for [`crate::text::TextStyle`]'s reason:
/// a float and two options side by side in a call is an easy place to pass the wrong one. It is also
/// what the three have in common — a wallpaper and a song's own picture differ in nothing else.
#[derive(Debug, Clone, Copy, Default)]
pub struct Backdrop<'a> {
    /// The image, or `None` for the theme's flat background.
    ///
    /// With one, the render goes through the same texture-and-scrim path the real display uses
    /// rather than a shortcut, so what comes back is what a television would show.
    pub image: Option<&'a RgbaImage>,
    /// The scrim, on the same scale [`Theme`] uses.
    ///
    /// Zero over a song's own picture: the scrim exists to keep *drawn* lyrics legible over an
    /// arbitrary photograph, and darkening a song's own words would only make them harder to sing.
    pub dim: f32,
    /// The ratio to letterbox to, or `None` to fill the surface.
    ///
    /// [`Background::shape`], and what lets a frame be rendered over a song's own picture rather
    /// than over a wallpaper. The furniture a picture song's screen draws is placed against the
    /// bottom of that picture, so a cell that passed `None` for one would be a picture of the wrong
    /// arrangement.
    pub shape: Option<(u32, u32)>,
}

/// A surface, a canvas, a text cache and a picture, kept across frames.
///
/// **The text cache is the reason this type exists.** Rendered text is the largest single cost in
/// drawing a screen, and a cache that is built and dropped per frame never serves a hit — so a
/// stream driving [`render_to_image`] thirty times a second would pay the full cost of every glyph
/// on every frame.
///
/// The canvas and its texture creator are kept for the same reason one step down: both are cheap to
/// hold and not cheap to make.
pub struct Offscreen {
    canvas: Canvas<Surface<'static>>,
    cache: TextCache<SurfaceContext<'static>>,
    /// What [`Self::set_picture`] last uploaded, drawn where the wallpaper goes.
    picture: Option<Texture>,
    width: u32,
    height: u32,
}

impl Offscreen {
    /// A renderer that draws `width` x `height` frames.
    pub fn new(width: u32, height: u32) -> Result<Self, OffscreenError> {
        let format = argb8888()?;
        let surface = Surface::new(width, height, format)
            .map_err(|e| OffscreenError::Surface(e.to_string()))?;
        let canvas = surface
            .into_canvas()
            .map_err(|e| OffscreenError::Surface(e.to_string()))?;
        let cache = TextCache::new(canvas.texture_creator());
        Ok(Self {
            canvas,
            cache,
            picture: None,
            width,
            height,
        })
    }

    /// The size every frame is drawn at.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Replaces the picture drawn behind the frame.
    ///
    /// **Called when the picture changes rather than once a frame**, because only the caller knows
    /// whether it has: a wallpaper stands for minutes, and a video's frames arrive in recycled
    /// buffers that hold new pixels at the same address, so nothing here could tell one case from
    /// the other by looking.
    pub fn set_picture(&mut self, image: Option<&RgbaImage>) -> Result<(), OffscreenError> {
        let Some(source) = image else {
            self.picture = None;
            return Ok(());
        };
        let format = argb8888()?;
        let mut staging = Surface::new(source.width(), source.height(), format)
            .map_err(|e| OffscreenError::Wallpaper(e.to_string()))?;
        staging.with_lock_mut(|bytes| swap_rb_into(source.as_raw(), bytes));
        let creator = self.canvas.texture_creator();
        self.picture = Some(
            creator
                .create_texture_from_surface(&staging)
                .map_err(|e| OffscreenError::Wallpaper(e.to_string()))?,
        );
        Ok(())
    }

    /// Draws one frame over whatever [`Self::set_picture`] last put behind it.
    ///
    /// `dim` and `shape` are [`Backdrop`]'s other two fields; the picture itself is already held.
    pub fn draw(
        &mut self,
        fonts: &Fonts,
        theme: &Theme,
        frame: &Frame<'_>,
        dim: f32,
        shape: Option<(u32, u32)>,
    ) {
        draw(
            &mut self.canvas,
            &mut self.cache,
            fonts,
            theme,
            frame,
            Background {
                current: self.picture.as_mut(),
                outgoing: None,
                fade: 1.0,
                dim,
                shape,
            },
        );
        self.canvas.present();
    }

    /// Hands the drawn pixels to `f` as BGRA, without copying them.
    ///
    /// The bytes are the canvas's own surface, which is where the software renderer draws, so this
    /// costs nothing per frame — which is the whole point on a path that runs at a video's frame
    /// rate. [`Self::to_image`] is the copying counterpart, for a caller that wants an owned image.
    pub fn with_pixels<R>(&mut self, f: impl FnOnce(&[u8]) -> R) -> R {
        let mut out = None;
        self.canvas.surface_mut().with_lock(|bytes| {
            out = Some(f(bytes));
        });
        // `with_lock` runs its closure unconditionally, so the `Option` is always filled. It exists
        // to carry a value out of a closure that returns nothing, not to model a failure.
        out.expect("with_lock always runs its closure")
    }

    /// The drawn pixels as an owned image.
    pub fn to_image(&mut self) -> Result<RgbaImage, OffscreenError> {
        let (width, height) = (self.width, self.height);
        let rgba = self.with_pixels(|bytes| {
            let mut out = vec![0u8; bytes.len()];
            swap_rb_into(bytes, &mut out);
            out
        });
        let len = rgba.len();
        RgbaImage::from_raw(width, height, rgba).ok_or(OffscreenError::SizeMismatch {
            width,
            height,
            len,
        })
    }

    /// Empties the text cache.
    ///
    /// The caller's obligation after rebuilding [`Fonts`], for the reason [`TextCache`] gives:
    /// nothing ties a cached texture to the face it was rendered from, so entries made by the old
    /// fonts would go on being served.
    pub fn clear_text_cache(&mut self) {
        self.cache.clear();
    }
}

impl Drop for Offscreen {
    /// Frees the cached textures while the canvas that made them is still alive.
    ///
    /// `unsafe_textures` is what allows a cache to hold textures beside the canvas at all, and its
    /// one obligation is this ordering. A `Drop` rather than a rule for the caller to remember,
    /// because this type owns both halves.
    fn drop(&mut self) {
        self.cache.clear();
    }
}

/// Swaps red and blue between packed 32-bit pixels, which converts RGBA to BGRA and back.
///
/// The same pass serves both directions because the swap is its own inverse. Alpha and green stay
/// where they are.
fn swap_rb_into(source: &[u8], out: &mut [u8]) {
    let pairs = source
        .as_chunks::<4>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<4>().0);
    for (from, to) in pairs {
        *to = [from[2], from[1], from[0], from[3]];
    }
}

/// Renders one frame at `width` x `height` and returns it as an image.
///
/// One frame and everything it needed dropped with it. [`Offscreen`] is what a caller drawing more
/// than one frame reaches for.
pub fn render_to_image(
    width: u32,
    height: u32,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    backdrop: Backdrop<'_>,
) -> Result<RgbaImage, OffscreenError> {
    let mut offscreen = Offscreen::new(width, height)?;
    offscreen.set_picture(backdrop.image)?;
    offscreen.draw(fonts, theme, frame, backdrop.dim, backdrop.shape);
    offscreen.to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Swapping red and blue is its own inverse, which is what lets one pass serve both directions.
    ///
    /// Without this the two conversions could drift apart and a picture would come back with its
    /// red and blue exchanged — a fault that reads as a theme change rather than a byte-order
    /// mistake, which is why it is worth a test of its own.
    #[test]
    fn swapping_red_and_blue_twice_is_the_original() {
        let source: Vec<u8> = (0..64).collect();
        let mut once = vec![0u8; source.len()];
        let mut twice = vec![0u8; source.len()];
        swap_rb_into(&source, &mut once);
        swap_rb_into(&once, &mut twice);
        assert_ne!(once, source, "a swap that changes nothing is not a swap");
        assert_eq!(
            twice, source,
            "two swaps must leave every pixel where it began"
        );
    }

    /// The pixels handed out are the canvas's own surface, at the size asked for.
    ///
    /// This is the property the streamed path rests on: it reads the drawn bytes without copying
    /// them, so a surface that did not hold what was drawn would be a blank stream rather than a
    /// failure anybody could see.
    #[test]
    fn the_pixels_handed_out_are_the_whole_surface() {
        // A machine with no working SDL renderer has nothing to say about this property.
        let Ok(mut offscreen) = Offscreen::new(64, 32) else {
            return;
        };
        assert_eq!(offscreen.size(), (64, 32));
        let len = offscreen.with_pixels(<[u8]>::len);
        assert_eq!(
            len,
            64 * 32 * 4,
            "ARGB8888 is four bytes a pixel, and the whole surface is what a frame is read from"
        );
    }
}
