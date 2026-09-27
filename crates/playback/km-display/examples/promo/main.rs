//! Renders the playing screen for the promotional video, singing the song written for it.
//!
//! ```text
//! cargo run -p km-display --example promo -- <out_dir> [<end_ms> [fps]]
//! ```
//!
//! It always writes two files into `<out_dir>`. `song.kar` is the song [`song::compose`] writes,
//! which the audio renderer plays. `bars.json` gives the second each bar starts at, which the
//! video's scenes cut on. With an end it also renders `frame-0000.png` onwards, from the start.
//!
//! **The frames and the sound come from one song file and one tempo map**, so the wipe keeps time
//! with the music in the video exactly as it does on a television.
//!
//! The rules of `screen_animation.rs` hold here too: English, no build number and no queue.

use std::path::PathBuf;

use km_display::draw::{Frame, Screen, SongInfo};
use km_display::lyrics::LyricView;
use km_display::numbers::NumberEntry;
use km_display::text::Fonts;
use km_display::theme::Theme;
use km_display::{Backdrop, render_to_image};
use km_kmpkg::Language;
use km_song::{ParseOptions, Song};

// `hero_tick` serves the stills. This example takes its span from the caller instead.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
mod song;

use common::{DIM, HEIGHT, WIDTH, shipped_wallpaper};

/// The video's frame rate, unless the caller names another.
const DEFAULT_FPS: u32 = 30;

/// The number the header draws, as the first song of a package in bank 1.
const SHOWN_NUMBER: u32 = 1001;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out_dir) = args.first() else {
        return Err("usage: promo <out_dir> [<end_ms> [fps]]\n\n\
             Writes the promotional video's song and its bar times, and renders the playing \
             screen singing it."
            .into());
    };
    let end_ms: Option<u32> = match args.get(1) {
        Some(value) => Some(
            value
                .parse()
                .map_err(|_| format!("{value}: not a number of milliseconds"))?,
        ),
        None => None,
    };
    let fps: u32 = match args.get(2) {
        Some(value) => value
            .parse()
            .ok()
            .filter(|fps| (1..=60).contains(fps))
            .ok_or_else(|| format!("{value}: not a frame rate from 1 to 60"))?,
        None => DEFAULT_FPS,
    };
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir)?;

    let bytes = song::compose();
    let song = Song::parse(&bytes, &ParseOptions::default())?;
    std::fs::write(out_dir.join("song.kar"), &bytes)?;

    let bars: Vec<String> = (0..=song::END_BAR)
        .map(|bar| {
            let ms = song.tempo_map.tick_to_ms(bar * song::BAR);
            format!("{:.3}", f64::from(ms) / 1000.0)
        })
        .collect();
    std::fs::write(
        out_dir.join("bars.json"),
        format!("[{}]\n", bars.join(", ")),
    )?;
    println!(
        "{}: {} lines, {} ms long",
        song::TITLE,
        song.lyrics.line_count(),
        song.duration_ms()
    );

    let Some(end_ms) = end_ms else {
        return Ok(());
    };

    let _sdl = sdl3::init()?;
    let ttf = sdl3::ttf::init()?;
    let theme = Theme::default();
    let fonts = Fonts::discover(&ttf, None, None, None, false, &theme, HEIGHT)?;
    let wallpaper = shipped_wallpaper()?;

    let info = SongInfo {
        number: Some(km_songcode::SongCode::new(SHOWN_NUMBER)),
        title: song::TITLE.to_owned(),
        artist: Some(song::ARTIST.to_owned()),
        language: Language::parse("en").map(|language| language.name().to_owned()),
    };
    let view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    let empty_entry = NumberEntry::new();

    // Frame `n` shows the song at `n / fps` seconds, computed from `n` so that no rounding error
    // builds up over the span.
    let frames = u64::from(end_ms) * u64::from(fps) / 1000;
    for index in 0..frames {
        let position_ms = (index * 1000 / u64::from(fps)) as u32;
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
