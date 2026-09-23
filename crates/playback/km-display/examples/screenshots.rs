//! Renders the display pictures the README publishes.
//!
//! ```text
//! cargo run -p km-display --example screenshots -- <song.kar> [out_dir] [language-name] [songs,packages] [title]
//! ```
//!
//! **Not the same job as `examples/preview.rs`, which is why it is not the same example.** That one
//! is a diagnostic contact sheet: twenty frames covering the states that are easy to get wrong, on a
//! flat background, meant to be scrolled past quickly in a pull request. This one is three pictures
//! meant to be looked at, over a real wallpaper, of a real song — and their names are published, so
//! they may never be renumbered.
//!
//! The contracts differ too. `preview.rs` must work on a clean checkout with no assets, which is why
//! its hostile wallpaper is generated in-process. This one is allowed to insist on
//! `assets/wallpapers/default-wallpapers.zip` and on a song with words in it, and **fails loudly**
//! rather than quietly publishing a picture of a test fixture.
//!
//! Two things are deliberately fabricated rather than real, because the output is published:
//!
//! * **The addresses in the connect panel.** They are made up. A screenshot of somebody's actual LAN
//!   is a screenshot of somebody's actual LAN, and the QR code encodes it.
//! * **The queue.** A party, not whatever happened to be queued when this was run: a named singer,
//!   another, an anonymous entry, and a title long enough to have to be cut. English, like
//!   everything else a published picture shows, and carrying the numbers the demo catalog really
//!   gives those songs -- fabricated is not the same as arbitrary, and a number that means one song
//!   here and a different one in `remote-browse.png` is a contradiction a reader can see.
//!
//! The wallpaper is the one a fresh install shows, on purpose: the first image of the shipped pack,
//! picked by the same name ordering `Playlist` gives the running machine. A README that showed a
//! background the product does not ship would be a picture of nothing anybody receives. The
//! *generated* gradients are the tempting alternative, on the grounds that a photograph needs a
//! license — `The wallpapers the machine ships` settles that the other way, and the pack is CC0
//! precisely so it can be redistributed and published. `examples/wallpapers.rs` builds the
//! gradients and is the fallback for the day a photograph has to be withdrawn.

use std::path::{Path, PathBuf};

use km_display::catalog::CatalogSummary;
use km_display::connect::ConnectInfo;
use km_display::draw::{Frame, Screen, SongInfo};
use km_display::lyrics::LyricView;
use km_display::numbers::NumberEntry;
use km_display::text::Fonts;
use km_display::theme::Theme;
use km_display::{Backdrop, render_to_image};
use km_queue::QueueEntry;
use km_song::{ParseOptions, Song};

mod common;
use common::{DIM, HEIGHT, WIDTH, hero_tick, shipped_wallpaper};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(song_path) = args.next() else {
        return Err(
            "usage: screenshots <song.kar> [out_dir] [language-name] [songs,packages] [title]\n\n\
             Renders the three display pictures the README publishes. The song is required and \
             wants real words in it: these are published, so there is no fixture fallback."
                .into(),
        );
    };
    let out_dir = args
        .next()
        .map_or_else(|| PathBuf::from("docs/images"), PathBuf::from);
    // The English *name*, not the file's `@L` tag: `SongInfo::language` is resolved by the caller
    // precisely so this crate needs no `km-kmpkg` dependency to turn `pt` into `Portuguese`. Here
    // the caller is a person, so it is an argument.
    // Filtered like `title` below rather than taken raw: an empty argument is how both callers say
    // "nobody knows" -- `tools/dev/screenshots.sh` passes `""` in degraded mode and again when it has
    // no name for a language code, and says the picture will then show no language. Unfiltered it
    // showed `Dire Straits  ·  ` instead, a separator with nothing after it.
    let language = args.next().filter(|value| !value.trim().is_empty());
    // `songs,packages`, from the catalog actually being photographed — `tools/dev/screenshots.sh` reads
    // them off the running machine's own `GET /api/v1/packages`. **Real, not fabricated**, because
    // the `What the README may show of a catalog` rule in docs/decisions/repository.md fabricates only what would
    // leak a machine: the LAN addresses and the queue. How many songs somebody has leaks nothing.
    // Absent means the line is not drawn at all rather than drawn with a number nobody counted.
    let catalog = args.next().and_then(|pair| counts(&pair));
    // The title the *catalog* gives this song, for the same reason the language above is an
    // argument: this crate cannot reach a catalog, and the caller can. Optional, and the file's own
    // meta is the fallback — but a real corpus writes `SULTANS OF SWING` into a MIDI as often as not,
    // and the published pictures would then disagree with each other about the name of one song,
    // since every other surface shows the corrected title from the packager's index.
    let title = args.next().filter(|value| !value.trim().is_empty());
    std::fs::create_dir_all(&out_dir)?;

    let wallpaper = shipped_wallpaper()?;

    let bytes = std::fs::read(&song_path).map_err(|e| format!("{song_path}: {e}"))?;
    let song = Song::parse(&bytes, &ParseOptions::default())?;
    if song.lyrics.line_count() < 2 {
        return Err(format!(
            "{song_path} has {} lyric line(s); the playing picture needs at least two, because \
             showing one row empty is exactly what the screen does not normally look like",
            song.lyrics.line_count()
        )
        .into());
    }
    println!(
        "{song_path}: {} lyric line(s), {} syllable(s)",
        song.lyrics.line_count(),
        song.lyrics.syllable_count()
    );

    let _sdl = sdl3::init()?;
    let ttf = sdl3::ttf::init()?;
    let theme = Theme::default();
    let fonts = Fonts::discover(&ttf, None, None, None, false, &theme, HEIGHT)?;

    let info = SongInfo {
        // Pinned at the other end too: the demo catalog's index.csv gives this song 1019, and
        // `tools/dev/screenshots.sh` warns if the song it is playing is not that number. Otherwise 1019
        // here is some other song's number over there, in two pictures published side by side.
        number: Some(km_songcode::SongCode::new(1019)),
        title: title
            .or_else(|| song.meta.title.clone())
            .unwrap_or_else(|| "Untitled".to_owned()),
        artist: song.meta.artist.clone(),
        language,
    };
    let view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    let tick = hero_tick(&song, &view);
    println!(
        "  the highlight is at tick {tick} ({} ms in)",
        song.tempo_map.tick_to_ms(tick)
    );

    // Made up, and deliberately so — see the module comment.
    // No `factory_pin`, so the panel draws the address and nothing else: a published picture must
    // not carry a PIN, even an invented one, and a machine whose owner has set their own password
    // is the state these pictures are of.
    let connect = {
        let mut info =
            ConnectInfo::reachable(vec!["http://192.168.1.42:8177".to_owned()], "0.0.0.0:8177");
        // **Set rather than taken from this build**, because these pictures are drawn on Linux by
        // `tools/dev/screenshots.sh` and the key hint is a Windows and macOS line. A published
        // picture showing less than most readers will see is a picture that misleads about the
        // panel, which is the same reason the address above is invented rather than real.
        info.browser_key = Some(km_display::BrowserKey::F11);
        info
    };
    let mut typing = NumberEntry::new();
    for digit in "1019".chars() {
        typing.push_digit(digit);
    }
    let empty_entry = NumberEntry::new();

    // A party: a named singer, another, an anonymous entry, and a title that has to be cut.
    //
    // English, like everything a published picture shows, and these are the first four of the party
    // `tools/dev/screenshots.sh` really queues, with the numbers that script's index.csv pins them to.
    // Still fabricated, and deliberately so -- see the module comment -- but a reader comparing this
    // with `remote-queue.png` now finds agreement rather than two songs sharing one number.
    //
    // **The fourth title has to be long, and 28 characters is the threshold.** The overlay ellipsizes
    // each row at 38 characters and the prefix `4.  1008  ` takes ten of them, so a shorter title is
    // cut inside the *artist* and the picture stops demonstrating the thing it is there for. This one
    // is 30. Check it in the rendered picture rather than by counting: the `…` must fall in the title.
    let waiting = [
        queued(1, 1036, "Superstition", Some("Stevie Wonder"), Some("Ana")),
        queued(2, 1013, "Fields of Gold", Some("Sting"), Some("Beto")),
        queued(3, 1033, "Englishman in New York", Some("Sting"), None),
        queued(
            4,
            1008,
            "The Devil Went Down to Georgia",
            Some("The Charlie Daniels Band"),
            Some("Marina"),
        ),
    ];
    debug_assert!(
        waiting[3].title.chars().count() >= 28,
        "the fourth queue title must be long enough for the overlay to cut it"
    );

    let playing = Frame {
        // **Pinned, not inherited.** These pictures are published in the README, which is in
        // English — see `What the README may show of a catalog`. A machine set to another language
        // would otherwise republish a screenshot in it, and a stale picture looks exactly like a
        // fresh one.
        locale: km_locale::Locale::English,
        screen: Screen::Playing,
        faults: km_display::Faults::default(),
        flash: None,
        song: Some(&info),
        timeline: Some(&song.lyrics),
        picture: false,
        show_position: true,
        lyrics: view.frame(&song.lyrics, tick),
        position_ms: song.tempo_map.tick_to_ms(tick),
        duration_ms: song.duration_ms(),
        transpose: -2,
        tempo_ratio: 1.0,
        melody: Some(true),
        lyrics_hidden: false,
        connect: None,
        catalog: None,
        number_entry: &empty_entry,
        next_up: Some("1036  Superstition"),
        // The published pictures show a party, not an empty room: this song was queued by somebody
        // and the queue behind it is theirs. A demo label here would be a caption on a state the
        // rest of the picture contradicts.
        demo: None,
        show_connect_overlay: false,
        queue: &waiting,
        show_queue_overlay: false,
        keypad: None,
        // The published pictures show no bank label. The switcher is a debugging control, and a
        // README picture carrying one would advertise a machine nobody is shipped.
        performance: None,
        song_stats: None,
        soundfont_label: None,
        developer_mode: None,
        // A playing screen never draws it.
        version: None,
    };

    write(
        &out_dir,
        "screen-playing",
        &fonts,
        &theme,
        &playing,
        &wallpaper,
    )?;
    write(
        &out_dir,
        "screen-queue",
        &fonts,
        &theme,
        &Frame {
            show_queue_overlay: true,
            ..playing
        },
        &wallpaper,
    )?;
    write(
        &out_dir,
        "screen-idle-connect",
        &fonts,
        &theme,
        &Frame {
            connect: Some(&connect),
            catalog,
            // The one screen that draws a build number, and the picture of it carries none. A
            // number in frame invites a reader to compare it against the current release and read
            // the project as behind, which says when the run happened and nothing about the
            // product. Written out rather than left to `Frame::idle`, because this is the frame the
            // rule is about; see `What a published picture says about the build it was taken from`
            // in docs/decisions/repository.md.
            version: None,
            ..Frame::idle(&typing)
        },
        &wallpaper,
    )?;

    println!("\nwrote 3 pictures to {}", out_dir.display());
    Ok(())
}

/// Renders one picture and writes it under its published name.
fn write(
    dir: &Path,
    name: &str,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    wallpaper: &image::RgbaImage,
) -> Result<(), Box<dyn std::error::Error>> {
    let image = render_to_image(
        WIDTH,
        HEIGHT,
        fonts,
        theme,
        frame,
        Backdrop {
            image: Some(wallpaper),
            dim: DIM,
            shape: None,
        },
    )?;
    let path = dir.join(format!("{name}.png"));
    image.save(&path)?;
    println!("  wrote {}", path.display());
    Ok(())
}

/// Parses a `songs,packages` argument.
///
/// Anything unparsable reads as absent, so a caller that could not reach the machine draws no line
/// rather than a wrong one — the same choice the language argument above makes.
fn counts(pair: &str) -> Option<CatalogSummary> {
    let (songs, packages) = pair.split_once(',')?;
    Some(CatalogSummary::new(
        songs.trim().parse().ok()?,
        packages.trim().parse().ok()?,
    ))
}

/// One queue entry.
fn queued(
    id: u64,
    number: u32,
    title: &str,
    artist: Option<&str>,
    singer: Option<&str>,
) -> QueueEntry {
    QueueEntry {
        id,
        number: km_songcode::SongCode::new(number),
        title: title.to_owned(),
        artist: artist.map(str::to_owned),
        singer: singer.map(str::to_owned),
    }
}
