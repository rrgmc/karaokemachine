//! What the published display pictures share: the frame size, the scrim, the wallpaper they are
//! taken over, and the tick the highlight is caught at.
//!
//! One module rather than a copy in each example, so a still and the animation beside it cannot
//! disagree about the background or about which line they show.

use std::path::Path;

use km_display::lyrics::LyricView;
use km_song::Song;

/// The display's real target, and the wallpaper's native size, so nothing is resampled.
pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;

/// The scrim, matching `WallpaperSettings::dim`'s default.
pub const DIM: f32 = 0.45;

/// The folder the machine's own bundled wallpapers live in.
pub const WALLPAPER_DIR: &str = "assets/wallpapers";

/// The pack every published picture is taken over: the seven CC0 photographs the machine ships.
///
/// Named, rather than trusting whatever `Playlist::scan` puts first, because the folder is a place
/// anything may be dropped into -- a loose `01-dusk.png` left by `examples/wallpapers.rs` sorts
/// ahead of this and would quietly put the old gradient back into the README.
pub const WALLPAPER_PACK: &str = "default-wallpapers.zip";

/// Decodes the wallpaper the machine would be showing a minute after it was switched on.
///
/// Goes through `Playlist` and `Loader` rather than reading the zip here, so this is not a second
/// opinion about what the display draws -- same scan, same name ordering, same `Fit::Cover` resize.
/// The pack's images are already 1920x1080, so at [`WIDTH`]x[`HEIGHT`] that resize is a no-op and
/// nothing is resampled; asking for it anyway is what keeps this honest if the pack is ever rebuilt
/// at another size.
///
/// The entry is picked by walking the playlist to the first image belonging to [`WALLPAPER_PACK`]
/// rather than by naming a file: the pack's names carry a content hash
/// (`scenery-001-17193f9f-1920x1080.jpg`), so a repack would rename every one of them.
pub fn shipped_wallpaper() -> Result<image::RgbaImage, String> {
    let mut playlist = km_display::Playlist::scan(WALLPAPER_DIR, &[]);
    let mut source = None;
    for _ in 0..playlist.len() {
        if let Some(entry) = playlist.current()
            && file_name(entry.container()) == WALLPAPER_PACK
        {
            source = Some(entry.clone());
            break;
        }
        playlist.advance();
    }
    let source = source.ok_or_else(|| {
        format!(
            "{WALLPAPER_DIR}/{WALLPAPER_PACK} holds no images (run tools/setup/fetch-assets.sh)"
        )
    })?;
    println!("  wallpaper {}", source.name());

    let loader = km_display::Loader::start();
    if !loader.request(source, WIDTH, HEIGHT, km_display::Fit::Cover) {
        return Err("the wallpaper loader stopped before it was asked for anything".to_owned());
    }
    // The loader is a thread with a channel, and `poll` never blocks. Two seconds is far longer than
    // one JPEG takes and still bounded, so a wedged decode fails the run instead of hanging it.
    for _ in 0..400 {
        match loader.poll() {
            Some(Ok(image)) => {
                return image::RgbaImage::from_raw(image.width, image.height, image.rgba)
                    .ok_or_else(|| "the decoded wallpaper was not the size it claimed".to_owned());
            }
            Some(Err(e)) => return Err(e),
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    Err(format!(
        "{WALLPAPER_PACK}: the wallpaper did not decode within two seconds"
    ))
}

/// The last component of a path, for comparing an archive against [`WALLPAPER_PACK`].
pub fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
}

/// Where to freeze the highlight — searched for, not guessed at.
///
/// A published picture has to show the two things the screen is *for*: a syllable caught part-way
/// through its wipe, and **both rows occupied**, so the reader can see that the next line is already
/// waiting. Neither is a property of a tick you can work out in your head — whether a second line is
/// on screen depends on [`LyricView`]'s own lookahead — so rather than compute a plausible tick and
/// hope, this tries candidates and keeps the first that actually satisfies both.
///
/// Candidates are the middle syllables of each line long enough to read as a lyric, taken 60% of the
/// way across. First and last syllables are skipped deliberately: at either end the wipe sits on a
/// word boundary, which is exactly the frame that fails to demonstrate a wipe at all.
///
/// Falling back to the first line's midpoint keeps this total. A song where nothing qualifies still
/// renders, and the picture is then merely ordinary rather than absent.
pub fn hero_tick(song: &Song, view: &LyricView) -> u32 {
    let mut fallback = None;
    for line in song.lyrics.lines.iter().filter(|l| l.syllables.len() >= 6) {
        for syllable in &line.syllables[1..line.syllables.len() - 1] {
            let span = syllable.end_tick.saturating_sub(syllable.start_tick);
            if span == 0 {
                continue;
            }
            let tick = syllable.start_tick + span * 3 / 5;
            fallback.get_or_insert(tick);
            let frame = view.frame(&song.lyrics, tick);
            let mid_wipe = frame.current().is_some_and(|l| {
                l.syllable.is_some() && (0.05..0.95).contains(&l.syllable_progress)
            });
            if frame.lines.len() >= 2 && mid_wipe {
                return tick;
            }
        }
    }
    fallback.unwrap_or(0)
}
