//! Renders a Japanese lyric line as the television would, with and without CJK faces behind the font.
//!
//! **This is the check the unit tests cannot make.** No test in this crate may open a font, so
//! everything about the fallback chain that is worth knowing — whether the glyphs arrive, and
//! whether the syllable boundaries land on characters rather than all in one place — can only be
//! seen by rendering a frame and looking at it. Two PNGs come out; the difference between them is
//! the feature.
//!
//! ```sh
//! cargo run -p km-display --example cjk -- target/cjk
//! ```
//!
//! **Worth running on more than one platform**, because what it exercises is a list of paths that
//! differ per system and cannot be checked from anywhere else. Two of the three Linux entries in
//! `text::CJK_CANDIDATES` were wrong until a container was asked. Inside the `.deb` build image —
//! which carries DejaVu and no CJK font, so this also proves a *mixed* chain rather than one font
//! answering everything:
//!
//! ```sh
//! . tools/platform/linux/image-tag.sh
//! docker run --rm -v "$PWD":/src -v karaokemachine-deb-build:/build -w /src "$IMAGE" \
//!   bash -c 'apt-get update -qq && apt-get install -y -qq fonts-noto-cjk &&
//!            cargo run -p km-display --example cjk -- /src/local/cjk-linux'
//! ```
//!
//! Swap the distribution to check a path: `archlinux` wants `pacman -Sy noto-fonts-cjk` and
//! `fedora` wants `dnf install google-noto-sans-cjk-fonts`, and each puts the same file somewhere
//! else.
//!
//! On macOS a hand-typed `cargo` has to supply the CMake shim SDL3_ttf's vendored FreeType needs,
//! which `task` sets for itself:
//!
//! ```sh
//! CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo run -p km-display --example cjk -- local/cjk-macos
//! ```
//!
//! **Read the printed chain and `scripts-cjk.png` before believing the other two images.** The lyric
//! fixture is Japanese, so a chain that answers Japanese and nothing else renders it perfectly — and
//! a macOS list with no Chinese face in it looked correct for exactly that reason. The third image
//! puts four scripts on one line and the probe below names each of them, so a hole is a run of boxes
//! in a known place and a line of output, rather than a picture nobody thought to make.

use std::path::PathBuf;

use km_display::draw::{Frame, Screen, SongInfo};
use km_display::lyrics::LyricView;
use km_display::numbers::NumberEntry;
use km_display::offscreen::{Backdrop, render_to_image};
use km_display::text::{Fonts, find_cjk_fonts};
use km_display::theme::Theme;
use km_song::{ParseOptions, Song, testing};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

/// Part way into the first line, so the wipe is mid-syllable and its boundary is visible.
///
/// A face with no glyphs draws every syllable as the same box, so every boundary measures to the
/// same width and the highlight steps in equal jumps that do not follow the words. That is the
/// failure worth being able to see, and a still frame shows it.
const TICK: u32 = 700;

/// One line of each script the chain is meant to cover, drawn as a title.
///
/// **Coverage is per font and not per script, so this is four questions and not one.** In order:
/// Kana, four characters that exist only in Simplified Chinese, three Traditional forms, and Hangul.
/// A face that answers one says nothing about the rest — Hiragino Sans W3 draws the first and third
/// groups and neither of the other two — and `text`'s cap opens at most two files, so which two is
/// the whole of the design.
const SCRIPTS: &str = "こんにちは 语们这爱 愛們麼 안녕";

/// The same four questions as [`SCRIPTS`], one representative character each, asked of the font
/// rather than of the picture.
///
/// **`fonts.has_cjk()` is not this question and cannot be.** It turns true the moment any fallback
/// opens, so two faces that both answer Japanese report `true` while Simplified Chinese draws boxes —
/// which is what a macOS chain of Hiragino and Apple SD Gothic Neo said about itself. `find_glyph`
/// walks the fallbacks the same way drawing does, so it answers per codepoint.
const PROBES: &[(&str, char)] = &[
    ("Kana", 'こ'),
    ("Simplified Chinese", '这'),
    ("Traditional Chinese", '愛'),
    ("Hangul", '안'),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/cjk"), PathBuf::from);
    std::fs::create_dir_all(&out_dir)?;

    let _sdl = sdl3::init()?;
    let ttf = sdl3::ttf::init()?;
    let theme = Theme::default();

    // First, because a wrong path shortens this list silently and nothing downstream says so. Two
    // faces that answer the same script are the failure to look for, not an empty list.
    let chain = find_cjk_fonts(None);
    if chain.is_empty() {
        println!("chain: nothing found — set display.font_cjk, or this run proves nothing");
    }
    for (i, path) in chain.iter().enumerate() {
        println!("chain[{i}]: {}", path.display());
    }

    let song = Song::parse(&testing::soft_karaoke_japanese(), &ParseOptions::default())?;
    println!(
        "fixture: {} lyric line(s), {} syllable(s), decoded as {}",
        song.lyrics.line_count(),
        song.lyrics.syllable_count(),
        song.decoder.name()
    );
    let title = song.meta.title.clone().unwrap_or_else(|| "?".to_owned());
    println!("title: {title}");

    let info = SongInfo {
        number: Some(km_songcode::SongCode::new(10_234)),
        title,
        artist: song.meta.artist.clone(),
        language: Some("Japanese".to_owned()),
    };
    let view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    let entry = NumberEntry::new();

    for (label, with_cjk, all_scripts) in [
        ("without", false, false),
        ("with", true, false),
        ("scripts", true, true),
    ] {
        let fonts = Fonts::discover(&ttf, None, None, None, with_cjk, &theme, HEIGHT)?;
        println!(
            "{label} CJK fallbacks: fonts.has_cjk() = {}",
            fonts.has_cjk()
        );
        let info = if all_scripts {
            for (script, ch) in PROBES {
                let drawn = fonts.text.find_glyph(*ch).is_some();
                println!(
                    "  {script} ({ch}): {}",
                    if drawn { "drawn" } else { "MISSING — a box" }
                );
            }
            SongInfo {
                title: SCRIPTS.to_owned(),
                ..info.clone()
            }
        } else {
            info.clone()
        };
        let frame = Frame {
            locale: km_locale::Locale::English,
            screen: Screen::Playing,
            faults: km_display::Faults::default(),
            flash: None,
            song: Some(&info),
            timeline: Some(&song.lyrics),
            picture: false,
            show_position: true,
            lyrics: view.frame(&song.lyrics, TICK),
            position_ms: song.tempo_map.tick_to_ms(TICK),
            duration_ms: song.duration_ms(),
            transpose: 0,
            tempo_ratio: 1.0,
            melody: None,
            connect: None,
            catalog: None,
            number_entry: &entry,
            next_up: None,
            demo: None,
            show_connect_overlay: false,
            queue: &[],
            show_queue_overlay: false,
            keypad: None,
            performance: None,
            song_stats: None,
            soundfont_label: None,
            developer_mode: None,
            // A playing screen never draws it.
            version: None,
        };
        let image = render_to_image(
            WIDTH,
            HEIGHT,
            &fonts,
            &theme,
            &frame,
            Backdrop {
                image: None,
                dim: 0.35,
                shape: None,
            },
        )?;
        let path = out_dir.join(format!("{label}-cjk.png"));
        image.save(&path)?;
        println!("  wrote {}", path.display());
    }

    check_the_trigger(&ttf, &theme)?;
    Ok(())
}

/// Proves the thing that decides whether any of the above ever happens on a real machine.
///
/// `TextCache::saw_cjk` is what the display polls once a frame, and it is set on the cache's *miss*
/// path — so it cannot be checked without drawing, and drawing needs a font, which no test in this
/// crate may open. Here instead, going through `draw_text` exactly as every screen does.
fn check_the_trigger(
    ttf: &sdl3::ttf::Sdl3TtfContext,
    theme: &Theme,
) -> Result<(), Box<dyn std::error::Error>> {
    use km_display::text::{Align, TextCache, TextStyle, draw_text};
    use sdl3::pixels::{Color, PixelFormat};

    let format = PixelFormat::try_from(sdl3::sys::pixels::SDL_PIXELFORMAT_ARGB8888)?;
    let mut canvas = sdl3::surface::Surface::new(320, 64, format)?.into_canvas()?;
    let fonts = Fonts::discover(ttf, None, None, None, false, theme, HEIGHT)?;
    let mut cache = TextCache::new(canvas.texture_creator());
    let style = TextStyle::plain(Color::RGBA(255, 255, 255, 255), Align::Left);

    draw_text(
        &mut canvas,
        &mut cache,
        &fonts.text,
        "Príliš Coração 12345",
        (0.0, 0.0),
        &style,
    );
    println!(
        "after Latin only:      saw_cjk() = {} (must be false)",
        cache.saw_cjk()
    );
    assert!(
        !cache.saw_cjk(),
        "Latin text must never be what opens a CJK font"
    );

    draw_text(
        &mut canvas,
        &mut cache,
        &fonts.text,
        "こんにちは",
        (0.0, 0.0),
        &style,
    );
    println!(
        "after Japanese:        saw_cjk() = {} (must be true)",
        cache.saw_cjk()
    );
    assert!(cache.saw_cjk(), "a Japanese string must ask for a rebuild");

    cache.clear();
    println!(
        "after clear (rebuild): saw_cjk() = {} (must be false)",
        cache.saw_cjk()
    );
    assert!(
        !cache.saw_cjk(),
        "clearing is what follows a rebuild; leaving this set rebuilds every frame forever"
    );

    // The cache holds textures made by its own creator, and `Texture` has no `Drop` under
    // `unsafe_textures`. Freed while the canvas that made them is still alive, as `TextCache`
    // documents.
    cache.clear();
    Ok(())
}
