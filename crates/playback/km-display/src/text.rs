//! Fonts, measurement, and drawing text over a wallpaper.
//!
//! Two things here are not obvious.
//!
//! **Syllable positions are measured, not divided.** To wipe the highlight across a line, the
//! renderer needs the x position of each syllable boundary. Those come from measuring the *prefix* of
//! the line up to each boundary, which accounts for kerning between syllables. Dividing the line
//! width by the syllable count — or measuring syllables individually and summing — puts the boundary
//! in the wrong place and the highlight visibly drifts off the words.
//!
//! **Lyrics are outlined, not shadowed.** The words sit over an arbitrary photograph. A drop shadow
//! helps on one side only, and light text over a bright patch disappears regardless. The glyphs in a
//! dark color at eight surrounding offsets make the text readable over anything.
//!
//! **A string is composed before it is uploaded, so it costs one blit however thick its ring.** The
//! eight dark copies and the glyphs over them are one texture, and an opacity is a modulation of
//! that texture rather than a change to the two colors it was built from. See
//! [`compose_outlined`].

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use sdl3::pixels::{Color, PixelFormat};

use sdl3::render::{BlendMode, Canvas, FRect, RenderTarget, Texture, TextureCreator};
use sdl3::surface::Surface;
use sdl3::ttf::{Font, Sdl3TtfContext};

use crate::theme::Theme;

/// Fonts at the three sizes the display uses.
pub struct Fonts {
    /// Lyrics: the largest, read from across a room.
    pub lyric: Font<'static>,
    /// Narrower lyric faces, largest first, for a line too wide for [`Fonts::lyric`].
    ///
    /// See [`Fonts::fit_lyric`], which is the only thing that should be choosing between these.
    pub lyric_narrow: Vec<Font<'static>>,
    /// Titles and ordinary interface text.
    pub text: Font<'static>,
    /// Secondary detail.
    pub small: Font<'static>,
    /// CJK faces standing behind each of the above, one per size per font, or empty.
    ///
    /// **Last, because the field order is the drop order and SDL_ttf holds these by pointer.**
    /// `TTF_AddFallbackFont` borrows: a fallback closed before the face that names it leaves that
    /// face with a dangling pointer. Rust drops fields in declaration order, so every primary above
    /// is closed before anything here is.
    ///
    /// Nothing reads this. It exists to own the faces for exactly as long as the primaries that
    /// were pointed at them.
    fallbacks: Vec<Font<'static>>,
}

/// Why fonts could not be prepared.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    /// No usable font file was found.
    #[error("no font found; set the font path in settings (looked for: {0})")]
    NoFont(String),
    /// SDL could not load the font file.
    #[error("could not load font {path}: {message}")]
    Load {
        /// The file that failed.
        path: String,
        /// What SDL reported.
        message: String,
    },
}

/// Font files to try, in order, when none is configured.
///
/// A bundled font is the eventual answer — the same problem as the SoundFont, and the same solution
/// of fetching it at release time rather than committing it. Until then the display borrows one from
/// the system, which is why the list covers every platform we run on.
///
/// Android is in the list because it has to be: nothing is bundled yet, and without a system
/// fallback there the display cannot render a single character. `Roboto-Regular.ttf` has been at that
/// path since Android 4, and `NotoSans-Regular.ttf` and `DroidSans.ttf` cover the alternatives.
const CANDIDATES: &[&str] = &[
    // Windows
    "C:/Windows/Fonts/segoeui.ttf",
    "C:/Windows/Fonts/arial.ttf",
    "C:/Windows/Fonts/calibri.ttf",
    // macOS
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/Library/Fonts/Arial.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    // Linux
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    // Android, including Android TV
    "/system/fonts/Roboto-Regular.ttf",
    "/system/fonts/NotoSans-Regular.ttf",
    "/system/fonts/DroidSans.ttf",
];

/// Which of the four scripts a fallback file answers for, as a bitmask.
///
/// Simplified and Traditional Chinese are two entries and not one because a font routinely has the
/// one and not the other — Hiragino Sans W3 draws 愛 and not 爱 — and treating them as "Chinese"
/// is how a chain ends up with a face for a script nobody asked about and none for the one they did.
type Scripts = u8;

/// Kana and the kanji that go with them.
const JA: Scripts = 1 << 0;
/// Simplified Chinese.
const SC: Scripts = 1 << 1;
/// Traditional Chinese.
const TC: Scripts = 1 << 2;
/// Hangul.
const KO: Scripts = 1 << 3;
/// A face that answers for all four, which ends the search however much of the cap is left.
const ALL: Scripts = JA | SC | TC | KO;

/// Fonts to stand behind the chosen one for Han, Kana and Hangul, each with what it answers for.
///
/// **A chain rather than one entry, because coverage is per-font and not per-script**, and the mask
/// is here because that sentence is not decoration. MS Gothic has no Hangul, so Korean stays a row of
/// boxes behind it however good the Japanese looks. SDL_ttf consults the faces in the order they were
/// added and takes the first with a glyph.
///
/// **The mask is what [`MAX_FALLBACKS`] is spent against.** A flat list plus a cap picks the first two
/// files that exist, which on a stock Windows 11 is MS Gothic and Yu Gothic — two Japanese faces, with
/// Chinese and Korean never reached. [`pick_fallbacks`] takes a file only when it answers for
/// something nothing chosen already does, so a second slot cannot go to a script the first slot has.
///
/// **Each mask is the conservative reading.** Under-claiming costs at most one more candidate looked
/// at; over-claiming leaves a script with no face and nothing saying so, which is the failure this
/// list has now produced twice. So `msgothic.ttc` is marked Japanese although JIS carries traditional
/// forms too, and Hiragino the same.
///
/// **Every path here is skipped when it is not there**, exactly as [`CANDIDATES`] is, so a list that
/// is wrong about one platform costs nothing on it — but a list that is wrong about *all* of one
/// platform is a black television with a reason nobody reads, which is what
/// [`CANDIDATES`] already does on Fedora.
///
/// **The three Linux entries are one package in three places, and two of them were guessed wrong
/// before they were checked.** `fonts-noto-cjk` on Debian, `noto-fonts-cjk` on Arch and
/// `google-noto-sans-cjk-fonts` on Fedora install the same `NotoSansCJK-Regular.ttc` under three
/// unrelated directories; all three below were read out of a container running that distribution
/// rather than out of documentation. The Windows entries were verified present on a stock
/// Windows 11.
///
/// **macOS cost a font too, and for the third time it was a path taken from documentation.**
/// `PingFang.ttc` is not in `/System/Library/Fonts` on macOS 26 — the family is still installed, but
/// its file sits inside `FontServices.framework`, which is not a path to name. Behind the cap that one
/// absence left **Simplified Chinese with no face at all**, because the entry that carries it was
/// fourth. The macOS masks below were read out of the `cmap` of face 0 of each file; the others are
/// what the fonts are for.
///
/// Japanese first throughout: the corpus scan found Shift-JIS in 189 files and Big5 in 28, and no
/// EUC-KR or GB18030 at all, so the first font opened should be the one most likely to be asked for.
const CJK_CANDIDATES: &[(Scripts, &str)] = &[
    // Windows. `.ttc` collections, of which `TTF_OpenFont` takes face 0 — which is the regular
    // weight of each of these, and is what is wanted. Nothing Windows ships covers all four, so the
    // two slots go to the two scripts the corpus actually holds: Japanese, then Traditional Chinese.
    //
    // **Yu Gothic stays second and costs nothing there.** It answers the same script as MS Gothic, so
    // it is passed over whenever MS Gothic was taken and used when it was not — which is a real
    // arrangement, the Japanese supplemental font pack being optional. Moved below the Chinese
    // entries instead, a box without MS Gothic draws Simplified and Traditional Chinese and no
    // Japanese at all, which is the corpus backwards.
    (JA, "C:/Windows/Fonts/msgothic.ttc"),
    (JA, "C:/Windows/Fonts/YuGothM.ttc"),
    (TC, "C:/Windows/Fonts/msjh.ttc"),
    (SC, "C:/Windows/Fonts/msyh.ttc"),
    (KO, "C:/Windows/Fonts/malgun.ttf"),
    // macOS. Two files answer everything between them: Hiragino draws Kana and kanji, and Arial
    // Unicode draws both Chinese variants and Hangul out of one 23 MB file.
    //
    // **The first literal is NFC and the file on disk is NFD** — `ギ` here is U+30AE, and on disk it
    // is U+30AD followed by the combining dakuten U+3099. The two are different bytes and it resolves
    // anyway, because APFS is normalization-insensitive.
    (JA, "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"),
    (ALL, "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"),
    // Reached only where one of those two is missing, and each covers a part rather than the whole.
    (JA | SC | TC, "/System/Library/Fonts/Hiragino Sans GB.ttc"),
    (JA | KO, "/System/Library/Fonts/AppleSDGothicNeo.ttc"),
    // Linux: Debian, Arch, Fedora. Same file, three directories. Face 0 carries Hangul and both
    // Chinese variants as well as Japanese, so one file ends the search.
    (
        ALL,
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ),
    (ALL, "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
    (
        ALL,
        "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
    ),
    // Android, including Android TV
    (ALL, "/system/fonts/NotoSansCJK-Regular.ttc"),
    (SC | TC, "/system/fonts/DroidSansFallback.ttf"),
];

/// How many CJK faces to put behind the chosen font.
///
/// **Two, and the cap is the point.** Each entry is another `TTF_Font` per size — six of them, since
/// a face is bound to its point size — opened from a file of 8 to 20 MB. On Android the low-memory
/// killer has already taken this app down once, at about 1 GB, so an uncapped walk of
/// [`CJK_CANDIDATES`] on a machine that has all five Windows fonts installed is not a cost worth
/// paying for scripts the corpus does not contain. Two covers Japanese and one of the Chinese
/// variants, which is what the scan actually found — and [`pick_fallbacks`] is what makes it two
/// *different* scripts rather than the first two files on the disk.
const MAX_FALLBACKS: usize = 2;

/// The CJK files to stand behind the main font, longest-standing first.
///
/// A configured path wins outright and alone: somebody who names a font has answered the question,
/// and adding guesses behind their answer would only make the memory cost unpredictable.
pub fn find_cjk_fonts(configured: Option<&Path>) -> Vec<PathBuf> {
    if let Some(path) = configured
        && path.is_file()
    {
        return vec![path.to_path_buf()];
    }
    pick_fallbacks(CJK_CANDIDATES, |path| Path::new(path).is_file())
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

/// Chooses up to [`MAX_FALLBACKS`] files from `candidates`, never two for the same script.
///
/// **The rule is that a file has to earn its slot.** A candidate is taken only when it answers for a
/// script nothing already taken does, so the cap is spent on coverage rather than on list position —
/// which is the difference between a Windows box drawing Japanese twice and one drawing Japanese and
/// Chinese. The search stops early when the four scripts are covered, so a face like
/// `NotoSansCJK-Regular.ttc` is opened alone rather than joined by a second file that adds nothing.
///
/// `present` is a parameter so the choosing can be tested without a filesystem to arrange; the only
/// caller passes [`Path::is_file`].
fn pick_fallbacks<'a>(
    candidates: &[(Scripts, &'a str)],
    present: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
    let mut chosen = Vec::new();
    let mut covered: Scripts = 0;
    for (scripts, path) in candidates {
        if chosen.len() == MAX_FALLBACKS || covered == ALL {
            break;
        }
        if scripts & !covered == 0 || !present(path) {
            continue;
        }
        covered |= scripts;
        chosen.push(*path);
    }
    chosen
}

/// Finds a usable font file.
///
/// A path in settings wins. Failing that, `bundled` if the caller named one and it exists, then the
/// first system font that does.
///
/// The bundled path is a parameter rather than a constant because this crate cannot know where the
/// embedding application installed its assets — and a working-directory-relative guess resolves
/// against `/` on Android.
pub fn find_font(configured: Option<&Path>, bundled: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured
        && path.is_file()
    {
        return Some(path.to_path_buf());
    }
    if let Some(path) = bundled
        && path.is_file()
    {
        return Some(path.to_path_buf());
    }
    CANDIDATES
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
}

/// Points `face` at each of `behind` for the glyphs it does not have.
///
/// **The order matters and so does the timing.** SDL_ttf consults fallbacks in the order they were
/// added, and — the part that is not obvious — it caches rasterized glyphs per font, so a face that
/// has already drawn `.notdef` for a codepoint goes on drawing it. `TTF_AddFallbackFont` then
/// returns true having changed nothing. This is therefore called at load, before the face has
/// rendered a single string, and a face that is already in use is rebuilt rather than amended.
#[expect(
    unsafe_code,
    reason = "TTF_AddFallbackFont has no safe wrapper in sdl3 0.18.4; the borrow it takes is what \
              `Fonts::fallbacks` is declared last to outlive"
)]
fn attach_fallbacks(face: &Font<'static>, behind: &[Font<'static>]) {
    for fallback in behind {
        // SAFETY: both faces live in the same `Fonts`, which drops `fallbacks` after every primary,
        // so the pointer SDL_ttf keeps here outlives the face that holds it. Neither raw handle
        // escapes this loop.
        let ok = unsafe { sdl3_ttf_sys::ttf::TTF_AddFallbackFont(face.raw(), fallback.raw()) };
        if !ok {
            // Not fatal, and not worth a frame: the face still draws everything it has glyphs for,
            // which is every screen this product shows in the languages it ships.
            tracing::warn!(error = %sdl3::get_error(), "could not attach a CJK fallback font");
        }
    }
}

impl Fonts {
    /// Loads every size from one font file, scaled to the screen height.
    ///
    /// `cjk` names files to stand behind it for Han, Kana and Hangul; empty is the ordinary case and
    /// costs nothing. A CJK file that will not open is logged and skipped rather than failing the
    /// load — a display that will not start is worse than one that draws boxes for 0.3% of a corpus.
    pub fn load(
        ttf: &Sdl3TtfContext,
        path: &Path,
        cjk: &[PathBuf],
        theme: &Theme,
        screen_height: u32,
    ) -> Result<Self, TextError> {
        let open = |size: u16| {
            ttf.load_font(path, f32::from(size))
                .map_err(|e| TextError::Load {
                    path: path.display().to_string(),
                    message: e.to_string(),
                })
        };
        // **One fallback face per size per file**, because a `TTF_Font` is bound to its point size:
        // the face standing behind the lyric font cannot also stand behind the small one.
        let open_fallbacks = |size: u16| -> Vec<Font<'static>> {
            cjk.iter()
                .filter_map(
                    |candidate| match ttf.load_font(candidate, f32::from(size)) {
                        Ok(face) => Some(face),
                        Err(error) => {
                            tracing::warn!(
                                path = %candidate.display(), %error,
                                "could not open a CJK fallback font; skipping it"
                            );
                            None
                        }
                    },
                )
                .collect()
        };

        let mut fallbacks = Vec::new();
        let mut with_fallbacks = |size: u16| -> Result<Font<'static>, TextError> {
            let face = open(size)?;
            let behind = open_fallbacks(size);
            attach_fallbacks(&face, &behind);
            fallbacks.extend(behind);
            Ok(face)
        };

        let lyric = with_fallbacks(theme.lyric_px(screen_height))?;
        let mut lyric_narrow = Vec::new();
        for size in theme.lyric_fallback_px(screen_height) {
            lyric_narrow.push(with_fallbacks(size)?);
        }
        let text = with_fallbacks(theme.text_px(screen_height))?;
        let small = with_fallbacks(theme.small_px(screen_height))?;

        Ok(Self {
            lyric,
            lyric_narrow,
            text,
            small,
            fallbacks,
        })
    }

    /// Loads from whichever font can be found.
    ///
    /// `cjk_configured` is `display.font_cjk`, for a platform [`CJK_CANDIDATES`] does not cover.
    /// `with_cjk` is what decides whether any of that is opened at all — see
    /// [`crate::words::is_cjk`] and the display's rebuild.
    pub fn discover(
        ttf: &Sdl3TtfContext,
        configured: Option<&Path>,
        bundled: Option<&Path>,
        cjk_configured: Option<&Path>,
        with_cjk: bool,
        theme: &Theme,
        screen_height: u32,
    ) -> Result<Self, TextError> {
        let path = find_font(configured, bundled)
            .ok_or_else(|| TextError::NoFont(CANDIDATES.join(", ")))?;
        let cjk = if with_cjk {
            find_cjk_fonts(cjk_configured)
        } else {
            Vec::new()
        };
        Self::load(ttf, &path, &cjk, theme, screen_height)
    }

    /// Whether CJK faces are standing behind these.
    #[must_use]
    pub fn has_cjk(&self) -> bool {
        !self.fallbacks.is_empty()
    }
}

/// Where the x boundaries of a line's syllables fall.
#[derive(Debug, Clone, PartialEq)]
pub struct LineMetrics {
    /// Total width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
    /// One entry per syllable boundary: `offsets[i]` is where syllable `i` starts, and the last
    /// entry is the end of the line. Always `syllables + 1` long.
    pub offsets: Vec<f32>,
}

impl LineMetrics {
    /// The x position the highlight should have reached.
    ///
    /// `syllable` indexes the syllable being sung and `progress` is how far through it, 0.0 to 1.0.
    pub fn wipe_x(&self, syllable: Option<usize>, progress: f32) -> f32 {
        let Some(index) = syllable else {
            return 0.0;
        };
        let start = self.offsets.get(index).copied().unwrap_or(0.0);
        let end = self.offsets.get(index + 1).copied().unwrap_or(self.width);
        start + (end - start) * progress.clamp(0.0, 1.0)
    }
}

/// The same text, with what SDL_ttf cannot be handed taken out of it.
///
/// **A NUL inside a string is not a refused measurement, it is a dead process.** SDL_ttf takes a C
/// string and `sdl3` builds one with an `unwrap`, so the failure arrives as a panic rather than as
/// the `Err` the callers below are written to absorb -- and it arrives on the display thread, the
/// one thread whose panic ends the machine. A `.kar` writer that stores its words in fixed-length
/// fields puts the terminator in with them, which is how one reaches a font in the first place.
///
/// **This is the floor, not the fix.** Words are cleaned where they are read, by
/// `km_song::karaoke::clean_lyric_text`; what this covers is everything else that reaches a font --
/// a title out of a package, a name somebody typed into a phone, a file name off a disk -- none of
/// which any single parser owns.
///
/// Borrowed unless there is something to take out, which is every ordinary string.
fn drawable(text: &str) -> Cow<'_, str> {
    if text.contains('\0') {
        Cow::Owned(text.replace('\0', ""))
    } else {
        Cow::Borrowed(text)
    }
}

/// Measures a line and the position of every syllable boundary within it.
///
/// Prefixes are measured rather than individual syllables, so kerning between them is included and
/// the boundaries land where the glyphs actually are.
pub fn measure_line(font: &Font<'static>, syllables: &[&str]) -> LineMetrics {
    let mut offsets = Vec::with_capacity(syllables.len() + 1);
    let mut prefix = String::new();
    offsets.push(0.0);

    let mut height = f32::from(font.height() as i16);
    for syllable in syllables {
        prefix.push_str(&drawable(syllable));
        match font.size_of(&prefix) {
            Ok((width, h)) => {
                offsets.push(width as f32);
                height = height.max(h as f32);
            }
            // An unmeasurable string is not worth failing a frame over; hold the last boundary so
            // the wipe stalls rather than jumping to the wrong place.
            Err(_) => offsets.push(offsets.last().copied().unwrap_or(0.0)),
        }
    }
    let width = offsets.last().copied().unwrap_or(0.0);
    LineMetrics {
        width,
        height,
        offsets,
    }
}

/// Picks the first width that fits, or the last one when none does.
///
/// Split out from [`Fonts::fit_lyric`] because no test in this crate may open a font — so the
/// choosing is kept where it can be checked, and the measuring is the only part that needs SDL.
/// `widths` runs largest first.
pub(crate) fn first_fitting(widths: &[f32], available: f32) -> usize {
    widths
        .iter()
        .position(|&width| width <= available)
        .unwrap_or(widths.len().saturating_sub(1))
}

impl Fonts {
    /// The largest lyric face this line fits in, and its metrics.
    ///
    /// **A lyric line has no horizontal fitting anywhere else.** It is centered on `layout.w / 2.0`
    /// with its true measured width, so one wider than the screen puts its left edge at a negative
    /// x and overhangs both sides while SDL clips them — which is what a hymn line 80 characters
    /// long did, and what any of the corpus lines up to 1,667 characters would do.
    ///
    /// The size gives way rather than the words, following `draw_idle`'s treatment of the product's
    /// name: a lyric can no more be abbreviated than the name can, and it cannot wrap either,
    /// because the row beneath it is already spoken for by the upcoming line.
    ///
    /// Measured rather than estimated, because only the font that was actually found knows how wide
    /// its glyphs are. At most four `size_of` passes for an over-wide line and one for an ordinary
    /// one; no textures are made and no font is opened.
    ///
    /// When even the narrowest face will not fit, that one is returned and SDL clips it. There is no
    /// legible rendering of a 1,667-character line and pretending otherwise would only pick a
    /// different way to be unreadable.
    ///
    /// # Not cached, and a measurement is why
    ///
    /// This runs per lyric row per frame and caches nothing, which reads like the obvious next
    /// thing to put in [`TextCache`] — the invalidation rule is even already there, since that is
    /// cleared on every font rebuild. It is left alone deliberately.
    ///
    /// What the appliance measurements in `docs/architecture/audio.md` say is that the display cost
    /// **88% of a core** and that caching the *rendered strings* took it to 10.7%. The expensive
    /// thing is rasterizing glyphs, and that is what [`TextCache`] holds. `size_of` walks a metrics
    /// table and makes no texture, and `TextCache::get` builds no `Key` before a lookup, so there
    /// is no per-frame allocation on this path to save either.
    ///
    /// Caching this too would need a key over the syllables *and* `available`, which means building
    /// a string per lookup — an allocation, to avoid work that has not been shown to cost anything.
    /// If it is picked up, measure first: `KM_FRAME_STATS` and the `FrameMeter` already report where
    /// a frame goes.
    pub fn fit_lyric(&self, syllables: &[&str], available: f32) -> (&Font<'static>, LineMetrics) {
        let mut measured = vec![measure_line(&self.lyric, syllables)];
        for font in &self.lyric_narrow {
            if measured
                .last()
                .is_some_and(|m: &LineMetrics| m.width <= available)
            {
                break;
            }
            measured.push(measure_line(font, syllables));
        }

        let widths: Vec<f32> = measured.iter().map(|m| m.width).collect();
        let chosen = first_fitting(&widths, available);
        let metrics = measured.swap_remove(chosen);
        let font = if chosen == 0 {
            &self.lyric
        } else {
            &self.lyric_narrow[chosen - 1]
        };
        (font, metrics)
    }
}

/// How text is positioned relative to the given point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// `x` is the left edge.
    Left,
    /// `x` is the center.
    Center,
    /// `x` is the right edge.
    Right,
}

impl Align {
    fn left_edge(self, x: f32, width: f32) -> f32 {
        match self {
            Self::Left => x,
            Self::Center => x - width / 2.0,
            Self::Right => x - width,
        }
    }
}

/// Frees a texture that was made for one frame.
///
/// **This has to be explicit.** The `unsafe_textures` feature is on — it is what lets `km-app` keep
/// wallpaper textures in a struct without tying them to the `TextureCreator`'s lifetime — and the
/// price is that `Texture` has no `Drop`. Every texture made per frame therefore leaks unless it is
/// destroyed by hand, and `draw_text` makes one or two of them for every string on screen.
///
/// It went unnoticed because nothing fails: the frames are correct and the process simply grows. On a
/// desktop that is a slow leak nobody lives long enough to see. On Android the low-memory killer took
/// the app down at about 1 GB, with `GL mtrack` at 2.6 GB and climbing, roughly ninety seconds after
/// a song started.
#[expect(
    unsafe_code,
    reason = "Texture::destroy is unsafe only because it must precede its Canvas; the canvas is \
              borrowed by the caller for the whole of this call, so it certainly outlives this"
)]
fn free(texture: Texture) {
    // SAFETY: called while the caller holds `&mut Canvas`, so the renderer that owns this texture is
    // alive, which is the single condition `destroy` requires.
    unsafe { texture.destroy() }
}

/// The one pixel format this crate builds a surface in.
///
/// Named once because everything that reads or writes pixels by hand depends on it and getting it
/// wrong swaps red and blue — a fault that looks like a theme change rather than a byte-order
/// mistake. [`ALPHA`] is the other half of the same fact.
pub(crate) fn argb8888() -> Option<PixelFormat> {
    PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_ARGB8888).ok()
}

/// Where the alpha byte sits within a pixel.
///
/// `ARGB8888` on a little-endian machine puts it last, and everything below reads and writes the
/// other three without asking which of them is which — so this one number is the whole dependency on
/// the channel order. [`crate::offscreen`] names the format for the same reason.
const ALPHA: usize = 3;

/// A rendered image in memory, four bytes to the pixel and tightly packed.
///
/// Owned bytes rather than a borrowed `Surface`, because composition reads one image while writing
/// another and a surface's rows are padded to its pitch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bitmap {
    /// Pixels across.
    pub(crate) width: usize,
    /// Pixels down.
    pub(crate) height: usize,
    /// `width * height * 4` bytes.
    pub(crate) pixels: Vec<u8>,
}

impl Bitmap {
    /// A fully transparent image of this size.
    pub(crate) fn transparent(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width * height * 4],
        }
    }

    /// One pixel's four bytes, or `None` past the edge.
    fn at(&self, x: usize, y: usize) -> Option<&[u8]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let start = (y * self.width + x) * 4;
        self.pixels.get(start..start + 4)
    }
}

/// Where each of the nine layers of a composed string sits, as a multiple of the ring's offset.
///
/// Eight around and the glyphs in the middle: four sides and four corners, because fewer leaves
/// gaps on the diagonals and a gapped ring reads as a smudge. The glyphs are last because the layers
/// are composed in the order they stack, and they stack on top.
const LAYERS: [(usize, usize); 9] = [
    (0, 0),
    (1, 0),
    (2, 0),
    (0, 1),
    (2, 1),
    (0, 2),
    (1, 2),
    (2, 2),
    (1, 1),
];

/// One string's glyphs and the ring around them, composed at full strength.
///
/// `ring` is the same glyph run in the outline colour. The result is larger than the run by `offset`
/// on every side, which is what the caller blits from so a ring never moves the words.
///
/// **Composed here, and faded afterwards.** A ring drawn straight onto the canvas in eight
/// separately translucent passes compounds where its copies overlap, and translucent glyphs then
/// blend with the near-opaque dark it left rather than covering it: the glyph body comes out a grey
/// mixed from its own outline and the row reads as a hollow shape. Composing first and modulating
/// the result is what makes yielding an opacity rather than a change of color.
///
/// **Porter-Duff `over` written out rather than handed to a blend mode.** SDL's ordinary blending
/// onto a transparent surface leaves color premultiplied by alpha beside a straight alpha, so an
/// antialiased edge uploaded from it arrives dark; the premultiplied blend that avoids that then
/// needs the color modulated alongside the alpha or a faded string glows instead of thinning. The
/// arithmetic has neither problem, and an opaque layer is the case that matters: at `a = 1` it
/// leaves `c = c_src`, so the glyphs **replace** the ring under them.
///
/// **Nine layers in one pass over the result, not nine passes over the whole of it.** The
/// accumulation carries premultiplied color and divides once at the end, which is one division per
/// pixel rather than one per layer. A lyric line is the largest run on the screen and it is
/// composed again every time the words change, so the pass count is what this costs.
pub(crate) fn compose_outlined(fill: &Bitmap, ring: &Bitmap, offset: usize) -> Bitmap {
    let mut composed = Bitmap::transparent(fill.width + offset * 2, fill.height + offset * 2);
    for y in 0..composed.height {
        for x in 0..composed.width {
            // Premultiplied while it accumulates, because that is the form `over` composes in
            // without a division per layer.
            let mut premultiplied = [0.0f32; ALPHA];
            let mut alpha = 0.0f32;

            for (index, (dx, dy)) in LAYERS.iter().enumerate() {
                let layer = if index + 1 == LAYERS.len() {
                    fill
                } else {
                    ring
                };
                let (Some(sx), Some(sy)) = (x.checked_sub(dx * offset), y.checked_sub(dy * offset))
                else {
                    continue;
                };
                let Some(pixel) = layer.at(sx, sy) else {
                    continue;
                };
                let source = f32::from(pixel[ALPHA]) / 255.0;
                if source <= 0.0 {
                    continue;
                }
                let kept = 1.0 - source;
                for (channel, accumulated) in premultiplied.iter_mut().enumerate() {
                    *accumulated = f32::from(pixel[channel]) * source + *accumulated * kept;
                }
                alpha = source + alpha * kept;
            }

            if alpha <= 0.0 {
                continue;
            }
            let into = (y * composed.width + x) * 4;
            for (channel, accumulated) in premultiplied.iter().enumerate() {
                composed.pixels[into + channel] =
                    (accumulated / alpha).round().clamp(0.0, 255.0) as u8;
            }
            composed.pixels[into + ALPHA] = (alpha * 255.0).round() as u8;
        }
    }
    composed
}

/// A surface's pixels, in `ARGB8888` and with the row padding taken out.
fn bitmap_of(surface: &Surface<'_>) -> Option<Bitmap> {
    let surface = surface.convert_format(argb8888()?).ok()?;
    let width = surface.width() as usize;
    let height = surface.height() as usize;
    let pitch = surface.pitch() as usize;
    let mut pixels = Vec::with_capacity(width * height * 4);
    surface.with_lock(|bytes| {
        for row in 0..height {
            let start = row * pitch;
            let end = start + width * 4;
            if end <= bytes.len() {
                pixels.extend_from_slice(&bytes[start..end]);
            }
        }
    });
    (pixels.len() == width * height * 4).then_some(Bitmap {
        width,
        height,
        pixels,
    })
}

/// What one rendered string is drawn in.
///
/// Grouped for [`TextStyle`]'s reason: a colour, a second colour and a thickness side by side in a
/// call is an easy place to transpose two of them. It is also what [`Key`] holds, so the cache and
/// the renderer cannot disagree about what makes two strings different.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ink {
    color: Color,
    outline: Color,
    outline_px: i32,
}

impl Ink {
    /// Glyphs with no ring, for use on a panel and for the lyric wipe's sung pass.
    fn plain(color: Color) -> Self {
        Self {
            color,
            outline: Color::RGBA(0, 0, 0, 0),
            outline_px: 0,
        }
    }
}

/// Renders one string into a texture, or `None` if it is empty or unrenderable.
///
/// Hands back the **glyph run's** own size and the margin the ring adds on every side, which are two
/// different things: a caller lays out from the size and blits from the margin.
fn make_texture<T>(
    creator: &TextureCreator<T>,
    font: &Font<'static>,
    text: &str,
    ink: Ink,
) -> Option<(Texture, f32, f32, f32)> {
    // Cleaned before the emptiness test, so a string that was nothing but padding is refused here
    // rather than handed to SDL as an empty one.
    let text = drawable(text);
    if text.is_empty() {
        return None;
    }
    let surface = font.render(&text).blended(ink.color).ok()?;
    let width = surface.width() as f32;
    let height = surface.height() as f32;

    let offset = usize::try_from(ink.outline_px).unwrap_or(0);
    // A ring of nothing is the ordinary panel string, and it takes the short path: one surface, one
    // upload, no arithmetic over its pixels.
    let composed = (offset > 0)
        .then(|| {
            let ring = font.render(&text).blended(ink.outline).ok()?;
            let fill = bitmap_of(&surface)?;
            let ring = bitmap_of(&ring)?;
            // The two are the same string in the same face, so they measure the same; a pair that
            // did not would put the ring somewhere the glyphs are not.
            (fill.width == ring.width && fill.height == ring.height)
                .then(|| compose_outlined(&fill, &ring, offset))
        })
        .flatten();

    let Some(composed) = composed else {
        let texture = creator.create_texture_from_surface(&surface).ok()?;
        return Some((texture, width, height, 0.0));
    };

    let mut pixels = composed.pixels;
    let pitch = u32::try_from(composed.width * 4).ok()?;
    let surface = Surface::from_data(
        &mut pixels,
        u32::try_from(composed.width).ok()?,
        u32::try_from(composed.height).ok()?,
        pitch,
        argb8888()?,
    )
    .ok()?;
    let texture = creator.create_texture_from_surface(&surface).ok()?;
    Some((texture, width, height, offset as f32))
}

/// How a string should look.
///
/// Grouped into a struct rather than passed as loose arguments: two colors, a width and an
/// alignment side by side in a call is an easy place to transpose two values and get text that looks
/// almost right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Glyph color.
    pub color: Color,
    /// Outline color. Ignored when `outline_scale` is zero.
    pub outline: Color,
    /// The ring's thickness as a fraction of the face's own size; zero for no ring.
    ///
    /// **A fraction rather than a count, because a style is chosen before a face is.** The closures
    /// `draw_playing` builds serve the title in one face and the two rows under it in another, so a
    /// thickness fixed when the style is made is a thickness right for one of them. [`draw_text`]
    /// resolves it against the font it is handed, which is the only place both are known.
    pub outline_scale: f32,
    /// Position of the text relative to the given point.
    pub align: Align,
    /// How much of its opacity the text keeps, 0.0 to 1.0.
    pub alpha: f32,
}

impl TextStyle {
    /// Plain text with no outline, for use on a panel.
    pub fn plain(color: Color, align: Align) -> Self {
        Self {
            color,
            outline: color,
            outline_scale: 0.0,
            align,
            alpha: 1.0,
        }
    }

    /// Outlined text, for use over a wallpaper.
    pub fn outlined(color: Color, theme: &Theme, align: Align) -> Self {
        Self {
            color,
            outline: theme.lyric_outline,
            outline_scale: theme.outline,
            align,
            alpha: 1.0,
        }
    }

    /// The same style at a fraction of its opacity, glyphs and outline together.
    ///
    /// **The composed glyph is what yields, not its two colors.** A string is composed with its ring
    /// at full strength and the fraction is applied to the result, so the glyphs cover the ring at
    /// every opacity and what reaches the screen is one image gone thinner. Faded through the colors
    /// instead, the ring's eight passes compound where they overlap and the translucent glyphs then
    /// blend with the near-opaque dark underneath rather than covering it: the row comes out a
    /// hollow outline with a grey middle, which over a pale picture is the whole of what is left.
    ///
    /// It also keeps the opacity out of [`Key`], so a picture song and the machine's own screen
    /// share one texture instead of holding two that differ only in strength.
    #[must_use]
    pub fn faded(self, alpha: f32) -> Self {
        Self {
            alpha: self.alpha * alpha.clamp(0.0, 1.0),
            ..self
        }
    }

    /// What this string is drawn in, which is the part [`TextCache`] keys on.
    ///
    /// `face_px` is the size the font was opened at, and it is what the ring is a fraction of. A
    /// ring that rounds to nothing is a plain string, because a nought-pixel ring composed is the
    /// glyphs on their own.
    fn ink(&self, face_px: f32) -> Ink {
        let outline_px = if self.outline_scale > 0.0 {
            ((face_px * self.outline_scale).round() as i32).max(1)
        } else {
            0
        };
        if outline_px > 0 {
            Ink {
                color: self.color,
                outline: self.outline,
                outline_px,
            }
        } else {
            Ink::plain(self.color)
        }
    }

    /// The opacity as SDL modulates it.
    fn alpha_mod(&self) -> u8 {
        (self.alpha.clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

/// A lyric line and how far the highlight has crossed it.
#[derive(Debug, Clone, Copy)]
pub struct WipedLine<'a> {
    /// The whole line, as one string.
    pub text: &'a str,
    /// Where its syllable boundaries fall.
    pub metrics: &'a LineMetrics,
    /// How far the highlight has reached, in pixels from the start of the line.
    pub wipe_x: f32,
}

/// How a wiped lyric line should look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WipeStyle {
    /// The not-yet-sung part.
    pub pending: TextStyle,
    /// The already-sung part.
    pub sung: Color,
}

/// A rendered string, kept so the next frame does not have to make it again.
///
/// **What this saves is mostly the upload, not the rasterising**, which is the opposite of what it
/// looks like. Profiled on the appliance with `simpleperf` while a MIDI song played, `make_texture`
/// was about 38% of the app's CPU and it split two to one the wrong way round:
/// `SDL_CreateTextureFromSurface` — which is `glTexSubImage2D` and an `ioctl` into the GPU driver —
/// took 25%, and `TTF_RenderText_Blended` took 13%. A cache of rendered *surfaces* would therefore
/// have recovered the smaller third. **This holds the `Texture`.**
///
/// The measurement that sent us here: a screen whose words are pixels costs 23–32% of one core (a
/// video song, an MP3+G pair), and one that renders text costs 87–88% (a MIDI song, the idle
/// screen). Same box, same full-screen background every frame; the difference is text.
///
/// **Every entry must be freed by hand.** `unsafe_textures` is on — it is what lets textures outlive
/// the `TextureCreator` borrow, which is the whole reason a cache is possible — and the price is
/// that `Texture` has no `Drop`. Eviction and teardown both go through [`free`].
struct Entry {
    texture: Texture,
    /// The glyph run's own width, which is what a caller lays out from.
    width: f32,
    /// The glyph run's own height.
    height: f32,
    /// How far the ring reaches past the glyphs on every side, and zero without one.
    ///
    /// The texture is `width + pad * 2` by `height + pad * 2`. Held apart from the size above
    /// because the two answer different questions: where the next word goes, and where this one is
    /// blitted.
    pad: f32,
    /// Which sweep last wanted it. See [`TextCache::sweep`].
    used: u64,
}

impl Entry {
    /// The pixels the texture actually holds, for the budget.
    fn held_px(&self) -> usize {
        let width = self.width + self.pad * 2.0;
        let height = self.height + self.pad * 2.0;
        (width as usize) * (height as usize)
    }
}

/// What makes one rendered string different from another.
///
/// **The font is identified by address, and that is the sharp edge of this whole thing.** `sdl3`'s
/// `Font` keeps its `TTF_Font` handle private with no accessor, so there is nothing else stable to
/// key on: two faces of the same size are indistinguishable by their metrics, and keying on those
/// would draw one font's glyphs for another.
///
/// An address is only an identity while the fonts stay put, and **they do not**: the display rebuilds
/// `Fonts` on a resize, which frees the old faces and can put new ones at the same addresses. The
/// cache would then answer with the previous size's glyphs — text that is subtly, silently wrong.
/// Holding a `&Fonts` borrow would have made that unrepresentable, and it cannot: the display owns
/// its fonts by value and reassigns them. So the rule is a call instead of a type, and it is the one
/// thing a caller must not forget: **[`TextCache::clear`] on every font rebuild.**
///
/// **The ring is part of the identity**, because a string is composed with its ring before it is
/// uploaded: two rows in one face and one colour are still two textures when one of them is ringed
/// more heavily than the other. The opacity is not part of it, and that is the other half of the
/// same arrangement — a faded string is the same pixels modulated on their way to the screen, so a
/// picture song and the machine's own screen share one entry.
#[derive(Debug, PartialEq, Eq, Hash)]
struct Key {
    font: usize,
    text: String,
    color: (u8, u8, u8, u8),
    outline: (u8, u8, u8, u8),
    outline_px: i32,
}

/// Textures for strings already drawn, and the creator that makes new ones.
///
/// **Bounded, because the unbounded version of this is a known crash.** The per-frame textures this
/// replaces leaked once already: `GL mtrack` reached 2.6 GB and Android's low-memory killer took the
/// app down about ninety seconds into a song. A cache is that same failure with a slower fuse, so it
/// evicts by total pixel area and frees what it drops.
///
/// **Nothing here ties it to a particular `Fonts`**, because nothing can: the display owns its
/// fonts by value and rebuilds them on a resize. See [`Key`] for why that makes [`Self::clear`] the
/// caller's responsibility rather than the compiler's.
pub struct TextCache<C> {
    creator: TextureCreator<C>,
    entries: std::collections::HashMap<Key, Entry>,
    /// Pixels currently held. Compared against [`Self::BUDGET_PX`].
    held_px: usize,
    /// Bumped once per frame, so an entry's `used` says how long ago it was wanted.
    sweep: u64,
    /// Whether any string asked for since the last rebuild wanted a CJK glyph.
    ///
    /// **Here because this is the only place every drawn string passes through.** The alternative
    /// was for the display to ask the question of the strings it knows about — the lyric rows, the
    /// title, the queue — and that list is exactly the kind that grows a new member without anyone
    /// remembering this. A miss on the cache is a string nobody has drawn before, which is the one
    /// moment worth spending a scan on; a hit tests nothing.
    saw_cjk: bool,
    /// One reusable [`Key`], so a cache **hit** allocates nothing.
    ///
    /// **The hit path is the whole point of this type, and building an owned `Key` to ask with is
    /// what would cost it.** A lookup is made per string per frame and `draw_wiped_line` makes three,
    /// so a `text.to_owned()` before the question is several allocations a frame on the thread whose
    /// spare capacity the audio callback is competing for. The two fields the ring adds are `Copy`
    /// and change nothing about that.
    ///
    /// Reused rather than removed, because a `HashMap<Key, _>` cannot be looked up by parts without
    /// restructuring it into a map of maps — and this holds one `String` that grows once to the
    /// longest string ever drawn and is then written into in place. A **miss** still clones it, and
    /// that is free by comparison: a miss is about to rasterize a texture.
    lookup: Key,
}

impl<C> TextCache<C> {
    /// How many pixels of rendered text to keep, across every entry.
    ///
    /// Four megapixels — about 16 MB at 32 bits, or two 1080p screens' worth. Chosen against what a
    /// screen actually holds rather than by feel: the idle screen and a lyric line together are a
    /// few hundred thousand pixels, so this holds many screens of history and still cannot approach
    /// the hundreds of megabytes that killed the app before.
    const BUDGET_PX: usize = 4 * 1024 * 1024;

    /// A cache over these fonts and this creator.
    pub fn new(creator: TextureCreator<C>) -> Self {
        Self {
            creator,
            entries: std::collections::HashMap::new(),
            held_px: 0,
            sweep: 0,
            saw_cjk: false,
            lookup: Key {
                font: 0,
                text: String::new(),
                color: (0, 0, 0, 0),
                outline: (0, 0, 0, 0),
                outline_px: 0,
            },
        }
    }

    /// Marks the start of a frame, so eviction can tell old entries from live ones.
    ///
    /// **Called once a frame and not per string**, so a string drawn twice in one frame — which is
    /// what an outline does, and what the lyric wipe does — counts as one use.
    pub fn begin_frame(&mut self) {
        self.sweep = self.sweep.wrapping_add(1);
    }

    /// How many strings are held. For tests and for the frame meter.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether anything is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a string wanting CJK glyphs has been drawn since the fonts were last built.
    ///
    /// The display asks this once a frame and rebuilds its fonts the first time it is true, which is
    /// what keeps a machine that never shows a Japanese title from opening a Japanese font. One
    /// frame is drawn as boxes before the rebuild lands; at sixty a second that is not a thing
    /// anybody sees, and paying for it the other way round is a cost every machine carries forever.
    #[must_use]
    pub fn saw_cjk(&self) -> bool {
        self.saw_cjk
    }

    /// The size the string will draw at, warming the cache on the way.
    ///
    /// Separate from [`Self::get`] because the caller needs the width to place the text *before* it
    /// draws the outline, and the outline is a different entry — so holding a texture borrow across
    /// that decision would stop the outline being fetched at all.
    fn size_of(&mut self, font: &Font<'static>, text: &str, ink: Ink) -> Option<(f32, f32)> {
        let (_, width, height, _) = self.get(font, text, ink)?;
        Some((width, height))
    }

    /// Renders `text`, or hands back the texture from last time.
    ///
    /// The texture comes back `&mut` because every draw sets its blend mode and its alpha, which is
    /// how the furniture yields over a picture. See [`draw_text`].
    fn get(
        &mut self,
        font: &Font<'static>,
        text: &str,
        ink: Ink,
    ) -> Option<(&mut Texture, f32, f32, f32)> {
        // Written into the scratch key rather than into a fresh one; see the `lookup` field.
        self.lookup.font = std::ptr::from_ref(font) as usize;
        self.lookup.color = (ink.color.r, ink.color.g, ink.color.b, ink.color.a);
        self.lookup.outline = (ink.outline.r, ink.outline.g, ink.outline.b, ink.outline.a);
        self.lookup.outline_px = ink.outline_px;
        self.lookup.text.clear();
        self.lookup.text.push_str(text);
        let sweep = self.sweep;

        if !self.entries.contains_key(&self.lookup) {
            // The miss path, and the only one that allocates. It is about to rasterize a glyph run,
            // so one `String` clone is not what this costs — and neither is one scan of it.
            self.saw_cjk |= crate::words::needs_cjk(text);
            let key = self.lookup.clone();
            let (texture, width, height, pad) = make_texture(&self.creator, font, text, ink)?;
            let entry = Entry {
                texture,
                width,
                height,
                pad,
                used: sweep,
            };
            self.held_px += entry.held_px();
            self.entries.insert(key.clone(), entry);
            self.evict_to_budget(&key);
        }

        let entry = self.entries.get_mut(&self.lookup)?;
        entry.used = sweep;
        Some((&mut entry.texture, entry.width, entry.height, entry.pad))
    }

    /// Drops least-recently-wanted entries until the budget is met.
    ///
    /// **Never the entry just inserted**, whatever the budget says: the caller is about to draw it,
    /// and a cache that frees a texture its caller still holds is a use-after-free rather than a
    /// tight budget. A single string larger than the whole budget therefore stays for one frame and
    /// goes on the next, which is the right way round.
    fn evict_to_budget(&mut self, keep: &Key) {
        while self.held_px > Self::BUDGET_PX {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(key, _)| *key != keep)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.held_px = self.held_px.saturating_sub(entry.held_px());
                free(entry.texture);
            }
        }
    }

    /// Frees every texture held.
    ///
    /// **Not a `Drop` impl, and that is deliberate.** `Texture::destroy` is only sound while the
    /// renderer that made it is alive, and nothing in a `Drop` can promise that ordering. The
    /// display calls this while it still holds the canvas.
    pub fn clear(&mut self) {
        for (_, entry) in self.entries.drain() {
            free(entry.texture);
        }
        self.held_px = 0;
        // Cleared with the textures, because the question it answers is "since the fonts were built"
        // and this call is what follows a build. Leaving it set would have the display rebuild once
        // per frame forever on a screen that had shown one Japanese title.
        self.saw_cjk = false;
    }
}

impl Clone for Key {
    fn clone(&self) -> Self {
        Self {
            font: self.font,
            text: self.text.clone(),
            color: self.color,
            outline: self.outline,
            outline_px: self.outline_px,
        }
    }
}
/// Draws a string, with an outline when the style asks for one.
///
/// Returns the width drawn, so callers can lay out following text.
pub fn draw_text<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    font: &Font<'static>,
    text: &str,
    at: (f32, f32),
    style: &TextStyle,
) -> f32 {
    let (x, y) = at;
    // The face's own size, asked of the font rather than passed alongside it: a caller that handed
    // one face and the size of another would get a ring sized for a row it is not drawing.
    let ink = style.ink(font.get_size().unwrap_or_default());
    // Asked for first to learn the size, which decides `left` before anything is drawn. The borrow
    // is dropped immediately so the blit below can take the texture mutably.
    let Some((width, _)) = cache.size_of(font, text, ink) else {
        return 0.0;
    };
    // **Whole pixels.** The rows of this screen are fractions of the height, so most of them land on
    // a half — and a texture blitted 1:1 onto a half-pixel is resampled across two of them, which
    // softens the glyphs and smears the ring. Rounded here rather than at each caller because every
    // string goes through this one place.
    let left = style.align.left_edge(x, width).round();
    let top = y.round();
    let alpha = style.alpha_mod();

    if let Some((texture, glyphs, height, pad)) = cache.get(font, text, ink) {
        // Set on every draw rather than when it changes: a texture is shared between the screens
        // that fade it and the screens that do not, and a modulation left behind is a row drawn at
        // the strength the previous frame wanted. The wallpaper crossfade sets both the same way.
        texture.set_blend_mode(BlendMode::Blend);
        texture.set_alpha_mod(alpha);
        let rect = FRect::new(
            left - pad,
            top - pad,
            glyphs + pad * 2.0,
            height + pad * 2.0,
        );
        let _ = canvas.copy(&*texture, None, rect);
    }
    width
}

/// Draws a lyric line with the sung portion in a different color.
///
/// The line is rendered once and drawn twice: whole, in the pending color, then clipped to the wipe
/// position in the sung color. Rendering two separate strings instead would break kerning at the
/// boundary and make the text jitter as the highlight crossed it.
pub fn draw_wiped_line<T: RenderTarget, C>(
    canvas: &mut Canvas<T>,
    cache: &mut TextCache<C>,
    font: &Font<'static>,
    line: &WipedLine<'_>,
    at: (f32, f32),
    style: &WipeStyle,
) {
    let (x, y) = at;
    // Rounded here as well as inside `draw_text`, so the sung pass below lands on the same pixel the
    // pending pass did rather than half of one to its left.
    let left = style.pending.align.left_edge(x, line.metrics.width).round();
    let top = y.round();
    // The sung half yields with the pending half. A highlight at full strength over a line that had
    // gone thin would read as two lines rather than one line being crossed.
    let alpha = style.pending.alpha_mod();

    // Drawn from the already-resolved left edge, so the two passes cannot disagree about where the
    // line starts.
    let pending = TextStyle {
        align: Align::Left,
        ..style.pending
    };
    draw_text(canvas, cache, font, line.text, (left, top), &pending);

    let wipe = line.wipe_x.clamp(0.0, line.metrics.width);
    if wipe <= 0.0 {
        return;
    }
    // **The wipe is a clip, not a re-render, and the cache is what makes that free.** The sung
    // color is one more entry that lives as long as the line is on screen; the highlight crossing
    // it changes only the source rectangle, so a line costs two textures for its whole life rather
    // than two a frame.
    //
    // **The sung pass carries no ring**, because the pending pass under it already drew one and a
    // second ring at the same place would only thicken it. The sung color is opaque, so it covers
    // the pending glyphs it is drawn over.
    let Some((sung, width, height, _)) = cache.get(font, line.text, Ink::plain(style.sung)) else {
        return;
    };
    sung.set_blend_mode(BlendMode::Blend);
    sung.set_alpha_mod(alpha);
    // The source rectangle is in the texture's own pixels, which match the drawn size here, so the
    // same width can be used for both. **The wipe itself keeps its fraction**: the clipped edge is
    // what makes the highlight glide across a syllable, and rounding it makes it step.
    let source = FRect::new(0.0, 0.0, wipe.min(width), height);
    let destination = FRect::new(left, top, wipe.min(width), height);
    let _ = canvas.copy(&*sung, source, destination);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widths four faces of one line would measure, largest first, at 1080p.
    ///
    /// Real numbers rather than invented ones: `lyric_fallbacks` is `[0.80, 0.65, 0.50]`, so a line
    /// that comes out 1200px in the full face comes out at these in the rest.
    const LADDER: [f32; 4] = [1200.0, 960.0, 780.0, 600.0];

    /// The string `sdl3` cannot make a C string of, and what is left of it.
    ///
    /// Asserts the borrow as well as the content: this runs per syllable per frame, so a clean line
    /// allocating a copy of itself would be a cost paid by every song for the benefit of none.
    #[test]
    fn a_string_reaches_a_font_without_the_nul_that_would_kill_it() {
        assert!(matches!(drawable("plain words"), Cow::Borrowed(_)));
        assert_eq!(drawable("%SOL\0"), "%SOL");
        assert_eq!(drawable("\0\0"), "");
        // Nothing else is touched: a lyric is measured as it is written.
        assert_eq!(drawable("não  é\tsó"), "não  é\tsó");
    }

    #[test]
    fn the_full_size_is_taken_whenever_the_line_fits_it() {
        assert_eq!(first_fitting(&LADDER, 1920.0), 0);
        assert_eq!(
            first_fitting(&LADDER, 1200.0),
            0,
            "exactly filling the width fits"
        );
    }

    #[test]
    fn the_first_face_that_fits_wins_rather_than_the_smallest() {
        assert_eq!(first_fitting(&LADDER, 1000.0), 1);
        assert_eq!(first_fitting(&LADDER, 800.0), 2);
        assert_eq!(first_fitting(&LADDER, 700.0), 3);
    }

    /// A 1,667-character line exists in the corpus and has no legible rendering at any size. The
    /// narrowest face is used and SDL clips it, which is a deliberate choice and not a fallthrough.
    #[test]
    fn a_line_that_fits_nowhere_takes_the_narrowest_face() {
        assert_eq!(first_fitting(&LADDER, 10.0), 3);
        assert_eq!(first_fitting(&LADDER, 0.0), 3);
    }

    #[test]
    fn a_ladder_of_one_is_still_answerable() {
        assert_eq!(first_fitting(&[500.0], 1000.0), 0);
        assert_eq!(first_fitting(&[500.0], 100.0), 0);
        assert_eq!(
            first_fitting(&[], 100.0),
            0,
            "and an empty one cannot panic"
        );
    }

    #[test]
    fn wipe_position_interpolates_between_syllable_boundaries() {
        let metrics = LineMetrics {
            width: 300.0,
            height: 40.0,
            offsets: vec![0.0, 100.0, 200.0, 300.0],
        };
        assert_eq!(metrics.wipe_x(Some(0), 0.0), 0.0);
        assert_eq!(metrics.wipe_x(Some(0), 0.5), 50.0);
        assert_eq!(metrics.wipe_x(Some(1), 0.0), 100.0);
        assert_eq!(metrics.wipe_x(Some(1), 0.5), 150.0);
        assert_eq!(metrics.wipe_x(Some(2), 1.0), 300.0);
    }

    #[test]
    fn no_current_syllable_means_no_highlight() {
        let metrics = LineMetrics {
            width: 300.0,
            height: 40.0,
            offsets: vec![0.0, 150.0, 300.0],
        };
        assert_eq!(metrics.wipe_x(None, 0.9), 0.0);
    }

    #[test]
    fn a_syllable_index_past_the_end_stops_at_the_line_width() {
        let metrics = LineMetrics {
            width: 300.0,
            height: 40.0,
            offsets: vec![0.0, 150.0, 300.0],
        };
        // Should clamp to the line rather than running off the edge.
        assert_eq!(metrics.wipe_x(Some(99), 1.0), 300.0);
    }

    #[test]
    fn progress_outside_the_unit_range_is_clamped() {
        let metrics = LineMetrics {
            width: 200.0,
            height: 40.0,
            offsets: vec![0.0, 100.0, 200.0],
        };
        assert_eq!(metrics.wipe_x(Some(0), -5.0), 0.0);
        assert_eq!(metrics.wipe_x(Some(0), 5.0), 100.0);
    }

    #[test]
    fn alignment_places_the_left_edge_correctly() {
        assert_eq!(Align::Left.left_edge(100.0, 40.0), 100.0);
        assert_eq!(Align::Center.left_edge(100.0, 40.0), 80.0);
        assert_eq!(Align::Right.left_edge(100.0, 40.0), 60.0);
    }

    #[test]
    fn font_discovery_prefers_a_configured_path_and_reports_when_nothing_is_found() {
        // A configured path that does not exist must not be returned.
        let missing = Path::new("definitely/not/a/font.ttf");
        let found = find_font(Some(missing), None);
        assert_ne!(found.as_deref(), Some(missing));

        // On this machine at least one system font should exist; if not, the error names what was
        // tried, which is the useful behavior.
        match find_font(None, None) {
            Some(path) => assert!(path.is_file()),
            None => assert!(!CANDIDATES.is_empty()),
        }
    }

    #[test]
    fn a_bundled_font_that_does_not_exist_is_skipped_rather_than_returned() {
        // The Android case: nothing is bundled yet, so the named path is absent and the system
        // fallback has to carry it. Returning the missing path would hand SDL a file to fail on.
        let absent = Path::new("definitely/not/an/asset/dir/fonts/karaoke.ttf");
        let found = find_font(None, Some(absent));
        assert_ne!(found.as_deref(), Some(absent));
        if let Some(path) = found {
            assert!(path.is_file());
        }
    }

    /// A configured CJK font replaces the built-in list rather than joining it.
    #[test]
    fn a_configured_cjk_font_is_used_alone() {
        // Any real file will do; nothing here opens it. Cargo runs tests from the package root.
        let existing = Path::new("Cargo.toml");
        assert_eq!(
            find_cjk_fonts(Some(existing)),
            vec![existing.to_path_buf()],
            "an answer replaces the guesses rather than being added in front of them"
        );
    }

    /// A configured path that is not there falls back to the list, exactly as `font` does.
    #[test]
    fn a_configured_cjk_font_that_is_absent_is_skipped() {
        let absent = Path::new("definitely/not/a/font.ttc");
        let found = find_cjk_fonts(Some(absent));
        assert!(!found.iter().any(|path| path == absent));
        for path in &found {
            assert!(
                path.is_file(),
                "{} was returned but is not there",
                path.display()
            );
        }
    }

    /// The cap is what keeps a Windows box with all five CJK fonts from opening thirty faces.
    #[test]
    fn no_more_than_two_fallbacks_are_ever_opened() {
        assert!(find_cjk_fonts(None).len() <= MAX_FALLBACKS);
    }

    /// The real list, checked as the platform it cannot be checked on.
    ///
    /// **This is the test that would have caught the fault twice over.** A stock Windows 11 has all
    /// five of these, and the answer must not be two Japanese faces; a Mac has four of the five macOS
    /// entries, and the answer must not be Japanese and Korean with nothing that draws 语. Neither
    /// needed a font opened, only a mask and a name.
    #[test]
    fn each_platform_spends_its_two_slots_on_two_different_scripts() {
        for (platform, marker) in [
            ("Windows", "C:/Windows/Fonts/"),
            ("macOS", "/System/Library/Fonts/"),
        ] {
            let chosen = pick_fallbacks(CJK_CANDIDATES, |path| path.starts_with(marker));
            let mut covered: Scripts = 0;
            for path in &chosen {
                let scripts = CJK_CANDIDATES
                    .iter()
                    .find(|(_, candidate)| candidate == path)
                    .expect("chosen path came from the list")
                    .0;
                assert!(
                    scripts & !covered != 0,
                    "{platform} spent a slot on {path}, which answers for nothing new"
                );
                covered |= scripts;
            }
            assert_eq!(chosen.len(), MAX_FALLBACKS, "{platform} filled both slots");
            assert!(
                covered & JA != 0,
                "{platform} must draw Japanese: the corpus is 189 Shift-JIS files against 28 Big5"
            );
            assert!(
                covered & (SC | TC) != 0,
                "{platform} must draw one of the Chinese variants"
            );
        }
    }

    /// A face that answers everything is taken alone, which is Linux and Android.
    #[test]
    fn a_face_that_covers_all_four_scripts_ends_the_search() {
        let chosen = pick_fallbacks(CJK_CANDIDATES, |path| path.starts_with("/usr/share/fonts/"));
        assert_eq!(
            chosen,
            vec!["/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"],
            "a second file behind Noto CJK would be six more faces buying no glyph"
        );
    }

    /// The spare Japanese entry is what a machine without the first one falls through to.
    ///
    /// **Written after getting the order wrong.** With Yu Gothic below the Chinese entries this
    /// answered MS JhengHei and MS YaHei — Traditional and Simplified Chinese, no Japanese — on a box
    /// whose corpus is 189 Shift-JIS files. Position still matters; the mask only stops a slot being
    /// spent twice on one script.
    #[test]
    fn a_missing_file_falls_through_to_another_answering_the_same_script() {
        let chosen = pick_fallbacks(CJK_CANDIDATES, |path| {
            path.starts_with("C:/Windows/Fonts/") && path != "C:/Windows/Fonts/msgothic.ttc"
        });
        assert_eq!(
            chosen,
            vec!["C:/Windows/Fonts/YuGothM.ttc", "C:/Windows/Fonts/msjh.ttc"],
            "Yu Gothic is reached exactly when MS Gothic is not installed"
        );
    }

    /// Nothing present is an empty answer rather than a panic or a path that is not there.
    #[test]
    fn a_system_with_none_of_them_gets_nothing() {
        assert!(pick_fallbacks(CJK_CANDIDATES, |_| false).is_empty());
    }

    /// A solid block of one opaque color, standing in for the body of a stem.
    fn block(width: usize, height: usize, color: [u8; 3]) -> Bitmap {
        let mut bitmap = Bitmap::transparent(width, height);
        for pixel in bitmap.pixels.as_chunks_mut::<4>().0 {
            pixel[..ALPHA].copy_from_slice(&color);
            pixel[ALPHA] = 255;
        }
        bitmap
    }

    /// One layer over another, in straight alpha, as a canvas composes them.
    fn over(source: [f32; 3], alpha: f32, under: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|c| source[c] * alpha + under[c] * (1.0 - alpha))
    }

    /// How light a color is, which is all these assertions are about.
    fn brightness(color: [f32; 3]) -> f32 {
        (color[0] + color[1] + color[2]) / 3.0
    }

    /// The ring reaches `offset` past the glyphs on every side, so the run keeps its own size.
    #[test]
    fn a_composed_string_grows_by_its_ring_and_the_glyphs_stay_where_they_were() {
        let composed = compose_outlined(&block(4, 6, [255; 3]), &block(4, 6, [0; 3]), 3);
        assert_eq!((composed.width, composed.height), (10, 12));
        assert_eq!(composed.pixels.len(), 10 * 12 * 4);
    }

    /// Glyphs cover the ring under them rather than mixing with it.
    #[test]
    fn the_body_of_a_glyph_is_its_fill_and_none_of_its_ring() {
        const FILL: [u8; 3] = [0xEC, 0xEF, 0xF4];
        const OUTLINE: [u8; 3] = [0x08, 0x0A, 0x10];
        let composed = compose_outlined(&block(3, 3, FILL), &block(3, 3, OUTLINE), 1);

        let middle = composed.at(2, 2).expect("the middle of a 5x5 composition");
        assert_eq!(&middle[..ALPHA], &FILL, "the glyphs sit on top of the ring");
        assert_eq!(middle[ALPHA], 255, "an opaque run composes opaque");

        let corner = composed.at(0, 0).expect("the outermost ring pixel");
        assert_eq!(
            &corner[..ALPHA],
            &OUTLINE,
            "only the ring reaches the corner"
        );
        assert_eq!(corner[ALPHA], 255);
    }

    /// Six tenths of a glyph's opacity is six tenths of its own color.
    ///
    /// The number the decision asks for, against the number the other order gives. Faded through the
    /// colors, the ring's eight passes compound to all but opaque and the glyphs then blend with
    /// that instead of covering it, which lands the body of a stem near the midpoint between its
    /// fill and its outline. Composed first, the body is the fill and the fade is what a room sees.
    #[test]
    fn a_glyph_that_yields_keeps_its_own_color() {
        const ALPHA_OVER_PICTURE: f32 = 0.6;
        let fill = [0x8A as f32, 0x92 as f32, 0xA4 as f32];
        let outline = [0x08 as f32, 0x0A as f32, 0x10 as f32];
        let ground = [240.0, 240.0, 240.0];

        // Composed at full strength, then the composition faded once.
        let composed = compose_outlined(
            &block(3, 3, [0x8A, 0x92, 0xA4]),
            &block(3, 3, [0x08, 0x0A, 0x10]),
            1,
        );
        let body = composed.at(2, 2).expect("the middle of a 5x5 composition");
        let drawn = over(
            std::array::from_fn(|c| f32::from(body[c])),
            f32::from(body[ALPHA]) / 255.0 * ALPHA_OVER_PICTURE,
            ground,
        );

        // Faded through the colors: eight ring passes, then the glyphs, each one translucent.
        let mut through_colors = ground;
        for _ in 0..8 {
            through_colors = over(outline, ALPHA_OVER_PICTURE, through_colors);
        }
        through_colors = over(fill, ALPHA_OVER_PICTURE, through_colors);

        let wanted = over(fill, ALPHA_OVER_PICTURE, ground);
        assert!(
            (brightness(drawn) - brightness(wanted)).abs() <= 1.0,
            "a composed glyph came back at {:.0} where six tenths of its own color is {:.0}",
            brightness(drawn),
            brightness(wanted)
        );
        assert!(
            brightness(drawn) - brightness(through_colors) > 60.0,
            "the two orders came back at {:.0} and {:.0}, which is not the fault this describes",
            brightness(drawn),
            brightness(through_colors)
        );
    }

    /// Full strength is SDL's identity, so a MIDI screen is drawn exactly as its texture holds it.
    #[test]
    fn full_strength_modulates_by_nothing() {
        let style = TextStyle::plain(Color::RGB(0xEC, 0xEF, 0xF4), Align::Left);
        assert_eq!(style.alpha_mod(), 255);
        assert_eq!(style.faded(0.6).alpha_mod(), 153);
        assert_eq!(style.faded(0.0).alpha_mod(), 0);
    }

    /// An opacity scales what a style already carries rather than replacing it.
    #[test]
    fn an_opacity_scales_rather_than_replacing() {
        let style = TextStyle::plain(Color::RGB(0xEC, 0xEF, 0xF4), Align::Left);
        assert_eq!(
            style.faded(1.0),
            style,
            "a screen the machine owns has to come back bit for bit"
        );
        assert!((style.faded(0.6).faded(0.5).alpha - 0.3).abs() < 1e-6);
        assert_eq!(
            style.faded(0.6).color,
            style.color,
            "this is an opacity, not a second palette"
        );
    }

    /// A ring is part of what makes two strings different, and a plain string asks for none.
    #[test]
    fn a_plain_string_and_a_ringed_one_cannot_answer_for_each_other() {
        let color = Color::RGB(0xEC, 0xEF, 0xF4);
        let theme = Theme::default();
        let lyric = f32::from(theme.lyric_px(1080));
        let plain = TextStyle::plain(color, Align::Left).ink(lyric);
        let ringed = TextStyle::outlined(color, &theme, Align::Left).ink(lyric);
        assert_eq!(plain.outline_px, 0);
        assert_ne!(plain, ringed);

        // The ring follows the face, so one style drawn in two faces is two entries rather than one
        // ring sized for whichever face was asked first.
        let small =
            TextStyle::outlined(color, &theme, Align::Left).ink(f32::from(theme.small_px(1080)));
        assert!(small.outline_px < ringed.outline_px);
        assert!(small.outline_px >= 1, "a ring never rounds away to nothing");
    }

    #[test]
    fn a_bundled_font_wins_over_the_system_ones() {
        // Any real file will do: the point is the order, not the glyphs. Cargo runs tests from the
        // package root, so this one exists; `file!()` would not, being workspace-root-relative.
        let existing = Path::new("Cargo.toml");
        assert_eq!(
            find_font(None, Some(existing)).as_deref(),
            Some(existing),
            "a bundled font that exists must be preferred to a system font"
        );
    }
}
