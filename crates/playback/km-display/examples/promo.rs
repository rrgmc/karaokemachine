//! Renders the playing screen for the promotional video: one carol, sung over a span of its length.
//!
//! ```text
//! cargo run -p km-display --example promo -- <pack.kmpkg> <song> <out_dir> [<start_ms> <end_ms> [fps]]
//! ```
//!
//! It always writes two files into `<out_dir>`. `song.kar` holds the song's own bytes, which the
//! audio renderer reads. `lines.tsv` gives each lyric line's start, end and words in milliseconds.
//! `LINE` in `tools/dev/promo/promo.html` is read off it, and a new carol means reading it again.
//! With a span it also renders `frame-0000.png` onwards.
//!
//! **The frames and the sound come from one song file and one tempo map**, so the wipe keeps time
//! with the music in the video exactly as it does on a television.
//!
//! The rules of `screen_animation.rs` hold here too. The song is a released public-domain carol,
//! the language is English, and the frame names no build and no queue.

use std::fmt::Write as _;
use std::path::PathBuf;

use km_display::draw::{Frame, Screen, SongInfo};
use km_display::lyrics::LyricView;
use km_display::numbers::NumberEntry;
use km_display::text::Fonts;
use km_display::theme::Theme;
use km_display::{Backdrop, render_to_image};
use km_kmpkg::{Language, Package};
use km_song::{ParseOptions, Song};

// `hero_tick` serves the stills. This example takes its span from the caller instead.
#[allow(dead_code)]
mod common;
use common::{DIM, HEIGHT, WIDTH, shipped_wallpaper};

/// The video's frame rate, unless the caller names another.
const DEFAULT_FPS: u32 = 30;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(pack_path), Some(number), Some(out_dir)) = (args.first(), args.get(1), args.get(2))
    else {
        return Err(
            "usage: promo <pack.kmpkg> <song> <out_dir> [<start_ms> <end_ms> [fps]]\n\n\
             Writes the song and its line timings, and renders the playing screen over a span."
                .into(),
        );
    };
    let number: u32 = number
        .parse()
        .map_err(|_| format!("{number}: not a song number"))?;
    let parse_ms = |value: &String| -> Result<u32, String> {
        value
            .parse()
            .map_err(|_| format!("{value}: not a number of milliseconds"))
    };
    let span = match (args.get(3), args.get(4)) {
        (Some(start), Some(end)) => Some((parse_ms(start)?, parse_ms(end)?)),
        (None, None) => None,
        _ => return Err("a span needs both a start and an end".into()),
    };
    let fps: u32 = match args.get(5) {
        Some(value) => value
            .parse()
            .ok()
            .filter(|fps| (1..=60).contains(fps))
            .ok_or_else(|| format!("{value}: not a frame rate from 1 to 60"))?,
        None => DEFAULT_FPS,
    };
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir)?;

    let package = Package::open(pack_path)?;
    let entry = package
        .manifest()
        .song(number)
        .ok_or_else(|| format!("{pack_path} holds no song {number}"))?
        .clone();
    if entry.language.as_deref() != Some("en") {
        return Err(format!("{}: not filed as English", entry.title).into());
    }
    let bytes = package.read_song(number)?;
    let song = Song::parse(&bytes, &ParseOptions::default())?;
    std::fs::write(out_dir.join("song.kar"), &bytes)?;

    let mut lines = String::new();
    for line in &song.lyrics.lines {
        let words: String = line.syllables.iter().map(|s| s.text.as_str()).collect();
        let _ = writeln!(
            lines,
            "{}\t{}\t{}",
            song.tempo_map.tick_to_ms(line.start_tick),
            song.tempo_map.tick_to_ms(line.end_tick),
            words.trim()
        );
    }
    std::fs::write(out_dir.join("lines.tsv"), lines)?;
    println!(
        "{}: {} lines, {} ms long",
        entry.title,
        song.lyrics.line_count(),
        song.duration_ms()
    );

    let Some((start_ms, end_ms)) = span else {
        return Ok(());
    };
    if end_ms <= start_ms {
        return Err(format!("the span {start_ms}..{end_ms} ms is empty").into());
    }

    let _sdl = sdl3::init()?;
    let ttf = sdl3::ttf::init()?;
    let theme = Theme::default();
    let fonts = Fonts::discover(&ttf, None, None, None, false, &theme, HEIGHT)?;
    let wallpaper = shipped_wallpaper()?;

    let info = SongInfo {
        number: Some(km_songcode::SongCode::new(1000 + number)),
        title: entry.title.clone(),
        artist: entry.artist.clone(),
        language: entry
            .language
            .as_deref()
            .and_then(Language::parse)
            .map(|language| language.name().to_owned()),
    };
    let view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    let empty_entry = NumberEntry::new();

    // Frame `n` shows the song at `start_ms + n / fps` seconds, computed from `n` so that no
    // rounding error builds up over a long span.
    let frames = u64::from(end_ms - start_ms) * u64::from(fps) / 1000;
    for index in 0..frames {
        let position_ms = start_ms + (index * 1000 / u64::from(fps)) as u32;
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
    println!(
        "wrote {frames} frames at {fps} fps to {}",
        out_dir.display()
    );
    Ok(())
}
