//! Renders the frames of the one animated picture the README publishes: two lines of a carol, sung.
//!
//! ```text
//! cargo run -p km-display --example screen_animation -- <pack.kmpkg> <song> <out_dir> [shown-number]
//! ```
//!
//! `<song>` is the song's number inside the package, and `[shown-number]` is the number drawn in
//! the header. It defaults to the package number in bank 1, which is what the machine shows once
//! the package is given that bank.
//!
//! **A still cannot show what the screen does.** `screen-playing.png` catches one syllable part-way
//! through its wipe. This example renders two whole lines being sung, frame by frame. A reader sees
//! the wipe move in time and the screen turn a line over. `tools/dev/screen-animation.sh` joins the
//! frames into `docs/images/screen-singing.webp`.
//!
//! **It reads a released package, and never a corpus.** A clip publishes whole lines of words in
//! motion, so the song has to be one whose words anybody may publish. The carol pack is the only
//! such package, and it is a release asset anybody can download. The `The animated picture is of a
//! public-domain carol` decision in docs/decisions/repository.md holds the argument.
//!
//! Everything a still keeps out, this keeps out as well: no build number, no debugging label, and
//! English. It draws no queue either. A carol sung alone needs no invented party behind it.

use std::path::PathBuf;

use km_display::draw::{Frame, Screen, SongInfo};
use km_display::lyrics::LyricView;
use km_display::numbers::NumberEntry;
use km_display::text::Fonts;
use km_display::theme::Theme;
use km_display::{Backdrop, render_to_image};
use km_kmpkg::{Language, Package};
use km_song::{ParseOptions, Song};

mod common;
use common::{DIM, HEIGHT, WIDTH, hero_tick, shipped_wallpaper};

/// Frames per second. Twelve is the lowest rate at which a wipe reads as a movement.
const FPS: u32 = 12;

/// How long the clip runs before the first line's first syllable, in milliseconds.
const LEAD_MS: u32 = 400;

/// How long the clip runs after the second line's last syllable, in milliseconds.
///
/// Long enough to see the last syllable finish before the clip starts again.
const TAIL_MS: u32 = 900;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(pack_path), Some(number), Some(out_dir)) = (args.next(), args.next(), args.next())
    else {
        return Err(
            "usage: screen_animation <pack.kmpkg> <song> <out_dir> [shown-number]\n\n\
             Renders the frames of the animated playing screen from one song of a package."
                .into(),
        );
    };
    let number: u32 = number
        .parse()
        .map_err(|_| format!("{number}: not a song number"))?;
    let shown: u32 = match args.next() {
        Some(value) => value
            .parse()
            .map_err(|_| format!("{value}: not a song number"))?,
        None => 1000 + number,
    };
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir)?;

    let package = Package::open(&pack_path)?;
    let entry = package
        .manifest()
        .song(number)
        .ok_or_else(|| format!("{pack_path} holds no song {number}"))?
        .clone();
    // The README is in English, and so is every picture it shows. See `What the README may show of
    // a catalog`.
    if entry.language.as_deref() != Some("en") {
        return Err(format!("{}: not filed as English", entry.title).into());
    }
    let bytes = package.read_song(number)?;
    let song = Song::parse(&bytes, &ParseOptions::default())?;
    if song.lyrics.line_count() < 2 {
        return Err(format!("{}: fewer than two lyric lines", entry.title).into());
    }

    let _sdl = sdl3::init()?;
    let ttf = sdl3::ttf::init()?;
    let theme = Theme::default();
    let fonts = Fonts::discover(&ttf, None, None, None, false, &theme, HEIGHT)?;
    let wallpaper = shipped_wallpaper()?;

    let info = SongInfo {
        number: Some(km_songcode::SongCode::new(shown)),
        title: entry.title.clone(),
        artist: entry.artist.clone(),
        language: entry
            .language
            .as_deref()
            .and_then(Language::parse)
            .map(|language| language.name().to_owned()),
    };

    // The line the still of this song would show, so a reader comparing the two sees one line.
    let view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    let tick = hero_tick(&song, &view);
    let lines = &song.lyrics.lines;
    let index = lines
        .iter()
        .position(|line| {
            line.syllables.first().is_some_and(|s| s.start_tick <= tick)
                && line.syllables.last().is_some_and(|s| tick <= s.end_tick)
        })
        .ok_or("no lyric line holds the highlight")?;
    // That line and the one after it. The clip then shows the second row sung while the first turns
    // over.
    let last_line = &lines[(index + 1).min(lines.len() - 1)];
    let (Some(first), Some(last)) = (lines[index].syllables.first(), last_line.syllables.last())
    else {
        return Err("the chosen lines have no syllables".into());
    };
    let start_ms = song
        .tempo_map
        .tick_to_ms(first.start_tick)
        .saturating_sub(LEAD_MS);
    let end_ms = song.tempo_map.tick_to_ms(last.end_tick) + TAIL_MS;
    let step_ms = 1000 / FPS;
    let frames = (end_ms - start_ms) / step_ms + 1;
    println!(
        "{}: {frames} frames from {start_ms} ms to {end_ms} ms, {step_ms} ms apart",
        entry.title
    );

    let empty_entry = NumberEntry::new();
    for index in 0..frames {
        let position_ms = start_ms + index * step_ms;
        let tick = song.tempo_map.ms_to_tick(position_ms);
        let frame = Frame {
            locale: km_locale::Locale::English,
            screen: Screen::Playing,
            faults: km_display::Faults::default(),
            flash: None,
            song: Some(&info),
            timeline: Some(&song.lyrics),
            picture: false,
            show_position: true,
            lyrics: view.frame(&song.lyrics, tick),
            position_ms,
            duration_ms: song.duration_ms(),
            transpose: 0,
            tempo_ratio: 1.0,
            melody: Some(true),
            lyrics_hidden: false,
            connect: None,
            catalog: None,
            number_entry: &empty_entry,
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
            version: None,
        };
        let image = render_to_image(
            WIDTH,
            HEIGHT,
            &fonts,
            &theme,
            &frame,
            Backdrop {
                image: Some(&wallpaper),
                dim: DIM,
                shape: None,
            },
        )?;
        image.save(out_dir.join(format!("frame-{index:04}.png")))?;
    }

    // The encoder reads the interval from here, so the frame rate is stated once.
    std::fs::write(out_dir.join("frame-ms"), format!("{step_ms}\n"))?;
    println!("wrote {frames} frames to {}", out_dir.display());
    Ok(())
}
