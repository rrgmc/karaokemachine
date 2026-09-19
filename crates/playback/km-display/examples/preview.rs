//! Renders display frames to PNG files without opening a window.
//!
//! ```text
//! cargo run -p km-display --example preview -- out_dir [song.kar]
//! ```
//!
//! A karaoke screen is judged by eye. Rendering to an off-screen surface means the layout can be
//! looked at — and reviewed in a pull request — without a monitor, a window manager, or a person
//! sitting in front of the machine. It also runs where CI does.
//!
//! Frames cover the cases that are easy to get wrong: the idle screen, the highlight part-way
//! through a syllable, both lyric rows occupied, a song with no lyrics at all, and each connect-panel
//! failure state.

use std::path::PathBuf;

use km_display::catalog::CatalogSummary;
use km_display::connect::{ConnectInfo, ConnectProblem};
use km_display::draw::{
    Flash, FlashKind, Frame, Screen, SongInfo, position_reserve, version_reserve,
};
use km_display::keypad::Keypad;
use km_display::lyrics::LyricView;
use km_display::numbers::{NumberEntry, SongPreview};
use km_display::text::Fonts;
use km_display::theme::Theme;
use km_display::{Backdrop, render_to_image};
use km_queue::QueueEntry;
use km_song::{LyricLine, LyricTimeline, ParseOptions, Song, Syllable, WordEnds, testing};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .map_or_else(|| PathBuf::from("target/preview"), PathBuf::from);
    let song_path = args.next();
    std::fs::create_dir_all(&out_dir)?;

    let _sdl = sdl3::init()?;
    let ttf = sdl3::ttf::init()?;
    let theme = Theme::default();
    let fonts = Fonts::discover(&ttf, None, None, None, false, &theme, HEIGHT)?;

    // A real file when given one, otherwise a fixture, so the preview works on a clean checkout.
    let (song, label) = match &song_path {
        Some(path) => {
            let bytes = std::fs::read(path)?;
            (Song::parse(&bytes, &ParseOptions::default())?, path.clone())
        }
        None => (
            Song::parse(&testing::soft_karaoke(), &ParseOptions::default())?,
            "fixture: soft_karaoke.kar".to_owned(),
        ),
    };
    println!("previewing {label}");
    println!(
        "  {} lyric line(s), {} syllable(s)",
        song.lyrics.line_count(),
        song.lyrics.syllable_count()
    );

    let info = SongInfo {
        number: Some(km_songcode::SongCode::new(10_234)),
        title: song
            .meta
            .title
            .clone()
            .unwrap_or_else(|| "Untitled".to_owned()),
        artist: song.meta.artist.clone(),
        // Already the name rather than the code — see `SongInfo::language`. Set here so the playing
        // screen's second line renders with both halves, which is the only way to see it without a
        // catalog and a monitor.
        language: Some("Portuguese".to_owned()),
    };
    let view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    let empty_entry = NumberEntry::new();

    let reachable = {
        let mut info = ConnectInfo::reachable(
            vec![
                "http://192.168.1.42:8177".to_owned(),
                "http://10.0.0.5:8177".to_owned(),
            ],
            "0.0.0.0:8177",
        );
        info.factory_pin = Some("482913".to_owned());
        // What a Windows build says. This example is run on whatever is to hand, and the row this
        // adds is one of the things worth looking at. The macOS spelling is longer and is the one
        // that tests the geometry; this is the one that shows the ordinary panel.
        info.browser_key = Some(km_display::BrowserKey::F11);
        info
    };

    // A machine with no address to give out. The idle screen and the `I` overlay say so in words;
    // the standing panel has no room for words and so draws nothing at all — which is what frame
    // `06c` is for.
    let stranded = ConnectInfo::unreachable(ConnectProblem::NoNetwork, None);

    let mut typing = NumberEntry::new();
    for digit in "10234".chars() {
        typing.push_digit(digit);
    }
    let mut not_found = NumberEntry::new();
    not_found.show_message("no song 9999999");

    // The keypad line is not only the keypad's: every refusal the machine can make arrives here, and
    // those are sentences rather than numbers. This one was on a real screen, in the lyric face,
    // wider than the window and clipped at both ends — the fourteen characters above are what the
    // contact sheet used to test with, and they are why nobody saw it.
    let mut long_message = NumberEntry::new();
    long_message.show_message("no melody channel was detected for this song");

    // The same digits, with the catalog's answer on hand: the name shows before OK is pressed.
    let mut typing_matched = NumberEntry::new();
    for digit in "10234".chars() {
        typing_matched.push_digit(digit);
    }
    // Bound first: `key()` borrows the entry that `set_lookup` is about to take mutably.
    let dialled = typing_matched.key().to_owned();
    typing_matched.set_lookup(
        &dialled,
        Some(SongPreview {
            title: "Tempo Perdido".to_owned(),
            artist: Some("Legião Urbana".to_owned()),
        }),
    );

    // The case the layout has to survive rather than the case it was designed for.
    let mut typing_long = NumberEntry::new();
    for digit in "429".chars() {
        typing_long.push_digit(digit);
    }
    let dialled_long = typing_long.key().to_owned();
    typing_long.set_lookup(
        &dialled_long,
        Some(SongPreview {
            title: "A Song With An Unreasonably Long Title That Nobody Would Ever Print".to_owned(),
            artist: Some("An Orchestra With A Similarly Unreasonable Name".to_owned()),
        }),
    );

    let pad_reserve = version_reserve(&theme, WIDTH as f32, HEIGHT as f32);
    let number_pad = Keypad::idle(WIDTH as f32, HEIGHT as f32, pad_reserve);
    // What a television remote looks like: the D-pad has walked to "5".
    let mut focused_pad = Keypad::idle(WIDTH as f32, HEIGHT as f32, pad_reserve);
    focused_pad.restore_focus(Some(4));
    let strip_reserve = position_reserve(HEIGHT as f32);
    let transport = Keypad::playing(WIDTH as f32, HEIGHT as f32, true, true, strip_reserve);
    let transport_no_melody =
        Keypad::playing(WIDTH as f32, HEIGHT as f32, false, true, strip_reserve);

    // A plausible queue: a named singer, an anonymous entry, and a title long enough to need cutting
    // — all three of which look different and all three of which happen at a party.
    let waiting = [
        queued(1, 4, "Planeta Sonho", Some("14 Bis"), Some("Beto")),
        queued(2, 12, "Bohemian Rhapsody", Some("Queen"), None),
        queued(
            3,
            871,
            "Bola de Meia, Bola de Gude (Milton Nascimento/Fernando Brant)",
            Some("14 Bis"),
            Some("Ana"),
        ),
    ];
    // More than the panel can hold, so the "and N more" line has to appear.
    let crowd: Vec<QueueEntry> = (1..=24)
        .map(|n| {
            queued(
                n,
                100 + n as u32,
                "A Song Somebody Queued",
                None,
                Some("Guest"),
            )
        })
        .collect();

    // Ticks chosen to land mid-syllable rather than on a boundary, which is where a wipe bug hides.
    let mid_first = song
        .lyrics
        .lines
        .first()
        .and_then(|line| line.syllables.get(2))
        .map(|s| s.start_tick + (s.end_tick - s.start_tick) / 3)
        .unwrap_or(0);
    let mid_second = song
        .lyrics
        .lines
        .get(1)
        .and_then(|line| line.syllables.get(1))
        .map(|s| s.start_tick + (s.end_tick - s.start_tick) / 2)
        .unwrap_or(0);

    let empty_timeline = LyricTimeline::default();

    // What the idle screen says it holds. A four-figure count on purpose, so the thousands separator
    // is exercised by the pictures rather than only by a unit test.
    let catalog = CatalogSummary::new(1284, 3);

    let frames: Vec<(&str, Frame<'_>)> = vec![
        // No keypad, which since `display.number_pad` became Android-only is what a **desktop** idle
        // screen looks like rather than merely one option of two. Frame 13 is the other platform.
        (
            "01-idle",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            // A machine nobody has put a package into yet. The state the summary line exists for:
            // without it this picture is identical to the one above, and the first thing anybody
            // learns is that a number they typed does not work.
            "01b-idle-empty-catalog",
            Frame {
                connect: Some(&reachable),
                catalog: Some(CatalogSummary::default()),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            "02-idle-typing",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                ..Frame::idle(&typing)
            },
        ),
        (
            "02b-idle-typing-match",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                ..Frame::idle(&typing_matched)
            },
        ),
        (
            // With the pad up, which is the arrangement the title has to stay clear of.
            "02c-idle-typing-long-title",
            Frame {
                connect: Some(&reachable),
                keypad: Some(&number_pad),
                catalog: Some(catalog),
                ..Frame::idle(&typing_long)
            },
        ),
        (
            // One area, which is the ordinary shape of a fault: a package would not install and the
            // sound is fine. What this frame is for is the length — the line was a quoted refusal
            // until 2026-09-08 and wrapped to two, and this is what it costs now.
            "14-idle-faults",
            Frame {
                connect: Some(&reachable),
                faults: km_display::Faults {
                    packages: 1,
                    sound: 0,
                },
                catalog: Some(catalog),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            // Both areas, and several of one of them. The case the old single-sentence slot could
            // not show at all: it held one complaint, so a machine with a refused package *and* no
            // SoundFont said only the first and left the second to a log.
            "14a-idle-faults-both",
            Frame {
                connect: Some(&reachable),
                faults: km_display::Faults {
                    packages: 3,
                    sound: 1,
                },
                catalog: Some(catalog),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            // A package dropped on the window, going in. The band is up and the standing fault line
            // is deliberately absent beneath it — `km-app` suppresses it, and this frame is what
            // says whether that reads as one message or as a gap.
            "14b-idle-drop-installing",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                flash: Some(Flash {
                    text: "installing brasil-vol2.kmpkg…",
                    kind: FlashKind::Working,
                }),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            "14c-idle-drop-installed",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                flash: Some(Flash {
                    text: "installed \"brasil-vol2\" \u{b7} 240 songs",
                    kind: FlashKind::Done,
                }),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            // A refusal at the length one really is, so the ellipsis is exercised rather than
            // assumed: this is one line and it is not allowed to grow into the title.
            "14d-idle-drop-refused",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                flash: Some(Flash {
                    text: "\"brasil-vol2.kmpkg\" was not installed: the archive has no manifest",
                    kind: FlashKind::Failed,
                }),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            "03-idle-not-found",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                ..Frame::idle(&not_found)
            },
        ),
        (
            // The same line with a sentence in it rather than a number. The size gives way and then
            // the shape does; what to look at is whether it stays inside the margins and clear of
            // the title above and the connect panel below.
            "03b-idle-long-message",
            Frame {
                connect: Some(&reachable),
                catalog: Some(catalog),
                ..Frame::idle(&long_message)
            },
        ),
        (
            "04-playing-first-line",
            playing(&info, &song.lyrics, &view, mid_first, &empty_entry, &song),
        ),
        (
            // And over a song, where the same string has a panel to stay inside instead of a screen.
            "04b-playing-long-message",
            playing(&info, &song.lyrics, &view, mid_first, &long_message, &song),
        ),
        (
            "05-playing-second-line",
            playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song),
        ),
        (
            "06-playing-transposed",
            Frame {
                transpose: -3,
                tempo_ratio: 1.1,
                melody: Some(true),
                next_up: Some("10235  Another Song"),
                ..playing(&info, &song.lyrics, &view, mid_first, &empty_entry, &song)
            },
        ),
        (
            // The demo line and `next:` are the two ends of the same sentence and the two ends of
            // the screen, so the interesting case is both at once: a demo song playing with
            // somebody's song already waiting behind it.
            //
            // **And with the address standing in the corner**, because that is what a demo song
            // actually looks like: the machine always has a `connect`, so a demo frame without one
            // was showing a state the running machine never reaches. The two lines on the left are
            // shortened to the room the panel leaves them, and this is the frame that says whether
            // that came out looking deliberate.
            "06b-playing-demo",
            Frame {
                next_up: Some("10235  Another Song"),
                demo: Some("DEMO · QUEUE to sing next"),
                connect: Some(&reachable),
                ..playing(&info, &song.lyrics, &view, mid_first, &empty_entry, &song)
            },
        ),
        (
            // The same song on a machine with nowhere to advertise. The standing panel draws
            // *nothing* rather than a card explaining itself, so this should be frame 06b with an
            // empty corner and both lines back at full width.
            "06c-playing-demo-no-network",
            Frame {
                next_up: Some("10235  Another Song"),
                demo: Some("DEMO · QUEUE to sing next"),
                connect: Some(&stranded),
                ..playing(&info, &song.lyrics, &view, mid_first, &empty_entry, &song)
            },
        ),
        (
            "07-playing-no-lyrics",
            Frame {
                timeline: None,
                ..playing(&info, &empty_timeline, &view, 0, &empty_entry, &song)
            },
        ),
        (
            // The same band, emptied for the opposite reason, and this is the pair to judge side by
            // side with 07. That one is a file with no words and says so in the middle of the
            // screen; this one is a file whose words somebody withheld, where that sentence would
            // be false — so the band is bare and the corner carries the word instead. The key and
            // the melody stand beside it, because neither is affected by the words going.
            "07b-playing-lyrics-withheld",
            Frame {
                timeline: None,
                lyrics_hidden: true,
                transpose: -2,
                ..playing(&info, &empty_timeline, &view, 0, &empty_entry, &song)
            },
        ),
        (
            "08-playing-connect-overlay",
            Frame {
                show_connect_overlay: true,
                connect: Some(&reachable),
                ..playing(&info, &song.lyrics, &view, mid_first, &empty_entry, &song)
            },
        ),
        // The touch keypad. Judged by eye because the tests only prove the geometry is sane: whether
        // the pad crowds the prompt, and whether the transport strip covers a lyric line, are
        // questions a PNG answers and an assertion does not.
        //
        // The number pad below is now what **Android** shows and frame 01 is what a desktop shows;
        // the transport strip in 14 and 15 is still every platform. Both pads are rendered here
        // regardless of what this host would draw, because this file is where the layout is judged
        // and Android is not a platform anybody previews on.
        (
            "13-idle-keypad",
            Frame {
                connect: Some(&reachable),
                keypad: Some(&number_pad),
                catalog: Some(catalog),
                ..Frame::idle(&typing)
            },
        ),
        (
            "14-playing-transport-strip",
            Frame {
                melody: Some(false),
                keypad: Some(&transport),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        (
            // A drop reported over a song, which is the case the band's shape was chosen for: the
            // playing screen writes the title into the top left and the badges into the top right,
            // both on the row a centered message would land in. This is the frame that says whether
            // covering that row reads better than sitting between the two.
            "14e-playing-drop-installed",
            Frame {
                melody: Some(false),
                keypad: Some(&transport),
                flash: Some(Flash {
                    text: "installed \"brasil-vol2\" \u{b7} 240 songs",
                    kind: FlashKind::Done,
                }),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        (
            "16-idle-keypad-dpad-focus",
            Frame {
                connect: Some(&reachable),
                keypad: Some(&focused_pad),
                catalog: Some(catalog),
                ..Frame::idle(&typing)
            },
        ),
        // The queue pill with something in it, on both screens and with **no overlay over it** —
        // which is the only way to see it. Every other frame on this sheet carries the empty pill,
        // and the two `show_queue_overlay` frames below dim theirs behind an 82% scrim by design.
        //
        // The playing one is the frame the whole corner rearrangement exists for: three badges out
        // at once, beside a two-digit pill, and the question is whether the run clears it and still
        // reads as a run.
        (
            "16a-idle-queue-waiting",
            Frame {
                queue: &waiting,
                catalog: Some(catalog),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            "16b-playing-queue-and-badges",
            Frame {
                queue: &crowd,
                transpose: -3,
                tempo_ratio: 1.1,
                melody: Some(true),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        // The queue overlay, over both screens. Judged by eye: whether the singer column reads at a
        // glance, whether a long title truncates rather than colliding with it, and — on the playing
        // frame — whether the double-strength panel really does stop the lyrics showing through.
        (
            "17-queue-over-playing",
            Frame {
                queue: &waiting,
                show_queue_overlay: true,
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        (
            "18-queue-empty",
            Frame {
                show_queue_overlay: true,
                catalog: Some(catalog),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            "19-queue-overflowing",
            Frame {
                queue: &crowd,
                show_queue_overlay: true,
                catalog: Some(catalog),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            "15-playing-transport-strip-no-melody",
            Frame {
                melody: None,
                keypad: Some(&transport_no_melody),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        // The bank label, on the screen where it has the most to collide with: the badges sit above
        // it, the artist line runs at it from the left, and `next:` is the row under both. The
        // geometry test proves the gaps arithmetically; this is where somebody checks it reads as an
        // aside rather than as something the singer should act on.
        (
            "20-playing-soundfont-label",
            Frame {
                transpose: -3,
                melody: Some(true),
                next_up: Some("10235  Another Song"),
                soundfont_label: Some("sf 3/5 MuseScore General — from the next MIDI song"),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        (
            "21-idle-soundfont-label",
            Frame {
                catalog: Some(catalog),
                soundfont_label: Some("sf 1/5 bundled"),
                ..Frame::idle(&empty_entry)
            },
        ),
        // The performance panel over a song, which is the screen it was positioned against: it has
        // to miss the flash band above it, the lyric ladder in the middle and the badges opposite,
        // and it deliberately does *not* miss the title. This is where somebody checks that the
        // trade reads as intended rather than as the panel having landed on something.
        (
            "22-playing-performance",
            Frame {
                transpose: -3,
                melody: Some(true),
                performance: Some(km_display::FrameStats {
                    frames: 60,
                    fps: 59.9,
                    draw_ms: 3.4,
                    draw_worst_ms: 9.1,
                    present_ms: 12.8,
                    present_worst_ms: 16.2,
                    interval_ms: 16.7,
                    interval_worst_ms: 21.4,
                    ..km_display::FrameStats::default()
                }),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        // The developer marker, and it is drawn **with** the bank label and the frame panel rather
        // than alone. That is the whole point of previewing it: it takes a third row of a corner
        // that already has two things in it, and it shares a band with a panel drawn from the
        // opposite margin. The collision test asserts the arithmetic; this is where somebody checks
        // that three lines stacked in one corner read as three facts rather than as a pile.
        (
            "22a-playing-developer-console",
            Frame {
                transpose: -3,
                melody: Some(true),
                soundfont_label: Some("sf 3/5 MuseScore General"),
                developer_mode: Some(km_display::DeveloperMode::Console),
                performance: Some(km_display::FrameStats {
                    frames: 60,
                    fps: 59.9,
                    draw_ms: 3.4,
                    draw_worst_ms: 9.1,
                    present_ms: 12.8,
                    present_worst_ms: 16.2,
                    interval_ms: 16.7,
                    interval_worst_ms: 21.4,
                    ..km_display::FrameStats::default()
                }),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        // The quieter of the two states, on the screen a machine spends most of its life on.
        (
            "22b-idle-developer-debugging",
            Frame {
                developer_mode: Some(km_display::DeveloperMode::Debugging),
                ..Frame::idle(&empty_entry)
            },
        ),
        // The two states the panel has that the healthy one does not show. The first is what the
        // first second after the key looks like — it must read as *working on it* rather than as a
        // machine reporting nothing. The second is the fault the counters exist for and the timings
        // cannot see: 60 fps exactly, and the picture has stopped.
        (
            "23-idle-performance-measuring",
            Frame {
                catalog: Some(catalog),
                performance: Some(km_display::FrameStats::default()),
                ..Frame::idle(&empty_entry)
            },
        ),
        (
            "24-playing-performance-starved",
            Frame {
                melody: Some(true),
                performance: Some(km_display::FrameStats {
                    frames: 60,
                    fps: 60.0,
                    draw_ms: 3.1,
                    draw_worst_ms: 4.0,
                    present_ms: 13.2,
                    present_worst_ms: 14.1,
                    interval_ms: 16.7,
                    interval_worst_ms: 17.0,
                    starved_ms: 412,
                    dropped: 18,
                    late: 7,
                    xruns: 2,
                }),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        // An ordinary MIDI song with the panel up, which is what pressing the key actually looks
        // like. The point of previewing it is the height: the song block has to read as a second
        // table about a different subject rather than as the frame table having grown four rows.
        (
            "26-playing-performance-song",
            Frame {
                transpose: -3,
                melody: Some(true),
                performance: Some(km_display::FrameStats {
                    frames: 60,
                    fps: 59.9,
                    draw_ms: 3.4,
                    draw_worst_ms: 9.1,
                    present_ms: 12.8,
                    present_worst_ms: 16.2,
                    interval_ms: 16.7,
                    interval_worst_ms: 21.4,
                    ..km_display::FrameStats::default()
                }),
                song_stats: Some(km_display::SongStats {
                    kind: km_display::SongMedia::Midi,
                    gain: 0.71,
                    gain_source: km_display::GainSource::Events { db: -4.8 },
                    bank_ignored: 3,
                    muted: 2,
                    flavor: Some(km_song::KaraokeFlavor::SoftKaraoke),
                    dialect: Some(km_song::Dialect {
                        angle_starts_lines: true,
                        annotations_are_marked: true,
                        harmonica_tabs: false,
                    }),
                    tracks: 9,
                    truncated_tracks: 0,
                    missing_tracks: 0,
                    repaired_notes: 0,
                }),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        // A video song, which fills in three rows and no more. Previewed because a block that short
        // has to read as complete rather than as one that was cut off.
        (
            "26a-playing-performance-song-video",
            Frame {
                picture: true,
                performance: Some(km_display::FrameStats {
                    frames: 60,
                    fps: 59.9,
                    draw_ms: 3.4,
                    draw_worst_ms: 9.1,
                    present_ms: 12.8,
                    present_worst_ms: 16.2,
                    interval_ms: 16.7,
                    interval_worst_ms: 21.4,
                    ..km_display::FrameStats::default()
                }),
                song_stats: Some(km_display::SongStats {
                    kind: km_display::SongMedia::Video,
                    gain: 0.34,
                    gain_source: km_display::GainSource::Package { lufs: -12.7 },
                    bank_ignored: 0,
                    muted: 0,
                    flavor: None,
                    dialect: None,
                    tracks: 0,
                    truncated_tracks: 0,
                    missing_tracks: 0,
                    repaired_notes: 0,
                }),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
        // **The tallest the panel gets**: a decoder complaining over a file the parser could not
        // finish, with the levelling hauling the song a long way. Nobody should agree to this
        // layout without looking at this picture — the height test bounds it, and a bound is not a
        // judgement about whether it reads.
        (
            "26b-playing-performance-worst",
            Frame {
                melody: Some(true),
                performance: Some(km_display::FrameStats {
                    frames: 60,
                    fps: 60.0,
                    draw_ms: 3.1,
                    draw_worst_ms: 4.0,
                    present_ms: 13.2,
                    present_worst_ms: 14.1,
                    interval_ms: 16.7,
                    interval_worst_ms: 17.0,
                    starved_ms: 412,
                    dropped: 18,
                    late: 7,
                    xruns: 2,
                }),
                song_stats: Some(km_display::SongStats {
                    kind: km_display::SongMedia::Midi,
                    gain: 2.24,
                    gain_source: km_display::GainSource::Events { db: -8.8 },
                    bank_ignored: 0,
                    muted: 1,
                    flavor: Some(km_song::KaraokeFlavor::LyricEvents),
                    dialect: Some(km_song::Dialect::default()),
                    tracks: 12,
                    truncated_tracks: 1,
                    missing_tracks: 2,
                    repaired_notes: 47,
                }),
                ..playing(&info, &song.lyrics, &view, mid_second, &empty_entry, &song)
            },
        ),
    ];

    // The build number goes on every idle frame, here rather than on each one above. What the
    // machine passes is a constant, so a frame that had to remember it would eventually be the one
    // frame the corner went unreviewed in — and the corner is exactly the kind of thing this sheet
    // is for. The playing frames are left alone, because that screen never draws it.
    let mut frames = frames;
    for (_, frame) in &mut frames {
        if frame.screen == Screen::Idle {
            frame.version = Some(concat!("v", env!("CARGO_PKG_VERSION")));
        }
    }

    for (name, frame) in &frames {
        render(&out_dir, name, &fonts, &theme, frame, theme_dim())?;
    }

    // Each connect-panel failure state, because a wrong URL on screen is worse than none.
    for (name, problem, bound) in [
        (
            "09-connect-loopback",
            ConnectProblem::LoopbackOnly,
            Some("127.0.0.1:8177".to_owned()),
        ),
        ("10-connect-no-network", ConnectProblem::NoNetwork, None),
        (
            "11-connect-server-failed",
            ConnectProblem::ServerFailed("address already in use (port 8177)".to_owned()),
            None,
        ),
    ] {
        let info = ConnectInfo::unreachable(problem, bound);
        let frame = Frame {
            connect: Some(&info),
            catalog: Some(catalog),
            ..Frame::idle(&empty_entry)
        };
        render(&out_dir, name, &fonts, &theme, &frame, theme_dim())?;
    }

    // The legibility test that actually matters: lyrics over a bright, busy image. This is the only
    // reason the glyph outline exists, so it gets its own frame rather than being assumed.
    let harsh = harsh_wallpaper(WIDTH, HEIGHT);
    render_over(
        &out_dir,
        "12-playing-over-bright-wallpaper",
        &fonts,
        &theme,
        &playing(&info, &song.lyrics, &view, mid_first, &empty_entry, &song),
        Backdrop {
            image: Some(&harsh),
            dim: theme_dim(),
            shape: None,
        },
    )?;
    // And the same with no scrim at all, to show what the scrim is buying.
    render_over(
        &out_dir,
        "13-playing-over-bright-wallpaper-no-scrim",
        &fonts,
        &theme,
        &playing(&info, &song.lyrics, &view, mid_first, &empty_entry, &song),
        Backdrop {
            image: Some(&harsh),
            dim: 0.0,
            shape: None,
        },
    )?;

    // A line no face can hold at full size. Before `Fonts::fit_lyric` this drew off both edges of
    // the screen with SDL clipping the ends, and nothing in this sheet would have shown it.
    let wide = overlong_lyrics();
    let wide_view = LyricView::for_ticks_per_quarter(song.ticks_per_quarter.max(1));
    render(
        &out_dir,
        "14-playing-overlong-lyrics",
        &fonts,
        &theme,
        &playing(&info, &wide, &wide_view, 2_000, &empty_entry, &song),
        theme_dim(),
    )?;

    // A song that brought its own words, which nothing in this sheet drew before — so nothing in it
    // could show the machine's furniture standing on somebody else's lyrics. The picture is handed
    // over at 4:3, so it is pillarboxed to the full height of this 16:9 frame exactly as an MP3+G
    // song is on a television, and its bottom rows are under everything this screen draws low.
    //
    // No scrim, because the machine passes none over a picture: the 45% exists to keep *drawn*
    // lyrics legible over a photograph, and darkening a song's own words would only make them harder
    // to sing.
    let cdg = cdg_test_picture();
    let cdg_shape = Some((4, 3));
    let picture_song = |show_position, keypad| {
        let mut frame = playing(&info, &empty_timeline, &view, 0, &empty_entry, &song);
        frame.picture = true;
        frame.show_position = show_position;
        frame.keypad = keypad;
        frame.next_up = Some("Bola de Meia, Bola de Gude — Ana");
        frame.queue = &waiting;
        // Part way through, because a bar at zero draws only its track and the track alone says
        // nothing about how far along the song is. Two fifths shows both halves of it.
        frame.duration_ms = 240_000;
        frame.position_ms = 96_000;
        frame
    };
    // What the room sees for all but the first seconds of the song: the picture, and the machine
    // standing back off it.
    render_over(
        &out_dir,
        "25-picture-song-bar-waiting",
        &fonts,
        &theme,
        &picture_song(false, None),
        Backdrop {
            image: Some(&cdg),
            dim: 0.0,
            shape: cdg_shape,
        },
    )?;
    // A song that has just started, or a transport key just pressed: the bar on the bottom safe
    // inset, under the strip rather than behind it.
    render_over(
        &out_dir,
        "25b-picture-song-bar-wanted",
        &fonts,
        &theme,
        &picture_song(true, Some(&transport)),
        Backdrop {
            image: Some(&cdg),
            dim: 0.0,
            shape: cdg_shape,
        },
    )?;
    // The furniture standing on a pale picture, which is where a glyph that mixed with its own ring
    // has nowhere to hide. A picture nobody can look at is half a check, so the sheet carries the
    // cell and `check_the_furniture_over_a_pale_picture` carries the number.
    let pale = pale_test_picture();
    render_over(
        &out_dir,
        "25c-picture-song-pale-ground",
        &fonts,
        &theme,
        &picture_song(false, None),
        Backdrop {
            image: Some(&pale),
            dim: 0.0,
            shape: cdg_shape,
        },
    )?;
    check_the_furniture_over_a_pale_picture(&ttf, &theme, &picture_song(false, None), &pale)?;
    println!(
        "\nwrote {} frames to {}",
        frames.len() + 8,
        out_dir.display()
    );
    Ok(())
}

fn theme_dim() -> f32 {
    0.45
}

/// A song whose lines are far wider than the screen, to prove the face gives way.
///
/// **Nothing in this contact sheet ever rendered a long lyric before**, which is how the display
/// came to have no horizontal fitting at all: the fixture's lines are 27 and 25 characters and the
/// bug needs about sixty. The first line here is a real one from the Christmas carol pack before it
/// was re-flowed, and the second is past anything the corpus holds outside a file with no line
/// markers at all.
fn overlong_lyrics() -> LyricTimeline {
    let lines = [
        "Hark! the herald angels sing, Glory to the newborn King;",
        "Peace on earth and mercy mild, God and sinners reconciled, joyful all ye nations rise",
    ];
    LyricTimeline {
        lines: lines
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let start = index as u32 * 4_000;
                LyricLine {
                    start_tick: start,
                    end_tick: start + 3_800,
                    page: 0,
                    // One syllable per word, so the wipe has somewhere to land.
                    syllables: split_words(text, start),
                    // Hand-written preview text, not a file: nothing to take out of it.
                    contact_redacted: false,
                }
            })
            .collect(),
        // Hand-written words, spaced the way they are meant to read.
        word_ends: WordEnds::AsWritten,
        // The lines are where this file puts them, which is what a marked file means.
        lines_are_marked: true,
    }
}

/// Splits a line into one syllable per word, spread evenly across the line's ticks.
fn split_words(text: &str, start: u32) -> Vec<Syllable> {
    let words: Vec<&str> = text.split_inclusive(' ').collect();
    let step = 3_800 / u32::try_from(words.len().max(1)).unwrap_or(1);
    words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            let at = start + u32::try_from(index).unwrap_or(0) * step;
            Syllable {
                text: (*word).to_owned(),
                start_tick: at,
                end_tick: at + step,
            }
        })
        .collect()
}

fn playing<'a>(
    song_info: &'a SongInfo,
    timeline: &'a LyricTimeline,
    view: &LyricView,
    tick: u32,
    entry: &'a NumberEntry,
    song: &Song,
) -> Frame<'a> {
    Frame {
        // The contact sheet is a developer's picture of the layout, and English is what this file
        // is written in.
        locale: km_locale::Locale::English,
        screen: Screen::Playing,
        faults: km_display::Faults::default(),
        flash: None,
        song: Some(song_info),
        timeline: Some(timeline),
        picture: false,
        show_position: true,
        lyrics: view.frame(timeline, tick),
        position_ms: song.tempo_map.tick_to_ms(tick),
        duration_ms: song.duration_ms(),
        transpose: 0,
        tempo_ratio: 1.0,
        melody: None,
        lyrics_hidden: false,
        connect: None,
        catalog: None,
        number_entry: entry,
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
    }
}

/// Renders one frame to an off-screen surface and writes it as a PNG.
fn render(
    dir: &std::path::Path,
    name: &str,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    dim: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    render_over(
        dir,
        name,
        fonts,
        theme,
        frame,
        Backdrop {
            image: None,
            dim,
            shape: None,
        },
    )
}

/// Renders a frame, optionally over a supplied wallpaper image, and writes it as a PNG.
///
/// The drawing itself is [`km_display::render_to_image`], shared with `examples/screenshots.rs`.
/// What stays here is only this example's own convention: one file per frame, named by the frame.
///
/// `shape` is what makes a song's own picture different from a wallpaper: `None` fills the frame,
/// `Some` letterboxes to the ratio the way the machine does.
fn render_over(
    dir: &std::path::Path,
    name: &str,
    fonts: &Fonts,
    theme: &Theme,
    frame: &Frame<'_>,
    backdrop: Backdrop<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let buffer = render_to_image(WIDTH, HEIGHT, fonts, theme, frame, backdrop)?;
    let path = dir.join(format!("{name}.png"));
    buffer.save(&path)?;
    println!("  wrote {}", path.display());
    Ok(())
}

/// A stand-in for an MP3+G screen: 288x192, the visible area of a CD+G disc, with blocky words on
/// the rows a real one writes them on.
///
/// **Synthetic rather than a real `.cdg`.** What this picture has to be right about is its *shape*
/// and which of its sixteen rows carry words, and both of those are geometry. It is drawn at the
/// pixel size and handed over as 4:3, which is the pair the machine passes, because CD+G pixels are
/// not square.
///
/// Rows 0 and 1 carry the title a disc puts at the top, and rows 14 and 15 the lower lyric line and
/// the credit under it. Those four are what the machine's own furniture has to be judged against:
/// the title, the artist line and `next:` land on the first two, and the position bar on the last
/// two — which is the frame that says whether standing back off a picture is enough.
fn cdg_test_picture() -> image::RgbaImage {
    const VISIBLE_WIDTH: u32 = 288;
    const VISIBLE_HEIGHT: u32 = 192;
    const TILE_W: u32 = 6;
    const TILE_H: u32 = 12;
    // Which of the sixteen rows hold words, and how many tiles of each are inked.
    const WORDS: [(u32, u32, u32); 4] = [(0, 4, 30), (1, 8, 22), (14, 3, 40), (15, 10, 18)];

    image::RgbaImage::from_fn(VISIBLE_WIDTH, VISIBLE_HEIGHT, |x, y| {
        let row = y / TILE_H;
        let column = x / TILE_W;
        let inked = WORDS.iter().any(|&(word_row, first, last)| {
            // A gap every fifth tile, so the row reads as words rather than as a rule.
            row == word_row && column >= first && column < last && column % 5 != 4
        });
        if inked {
            // The white a disc writes its words in, on the deep blue it grounds them on.
            image::Rgba([0xFF, 0xFF, 0xFF, 0xFF])
        } else {
            image::Rgba([0x11, 0x22, 0x55, 0xFF])
        }
    })
}

/// Asserts that a row of furniture over a pale picture is its own color at six tenths.
///
/// **Here rather than in a unit test, because no test in `km-display` may open a font.** The crate's
/// own tests hold the arithmetic; this holds the machinery that has to agree with it — the
/// composition, the upload, the modulation and the blit, over a real face and a real picture.
///
/// **Rendered at 1080 rather than at the sheet's 720.** `small_px` is 28 there against 19, so a stem
/// has a body to measure; at 720 the answer would depend on which face `find_font` turned up.
///
/// Judged as a histogram over the band the `artist · language` row occupies, never at a named
/// coordinate: the example runs against whatever font the machine has, so nothing may assume where a
/// glyph is.
fn check_the_furniture_over_a_pale_picture(
    ttf: &sdl3::ttf::Sdl3TtfContext,
    theme: &Theme,
    frame: &Frame<'_>,
    picture: &image::RgbaImage,
) -> Result<(), Box<dyn std::error::Error>> {
    const TALL: u32 = 1080;
    const WIDE: u32 = 1920;
    /// How far from a brightness a pixel may sit and still count as it.
    ///
    /// Narrow, because the ramp an antialiased edge walks passes through every value between the
    /// ring and the fill and would otherwise be counted as both.
    const SLACK: f32 = 6.0;

    let fonts = Fonts::discover(ttf, None, None, None, false, theme, TALL)?;
    // **Full bleed rather than the 4:3 the cell uses.** A letterboxed picture puts the theme's own
    // near-black down the sides, and the left margin of this screen is inside that bar — so a band
    // measured across it would be reading the pillarbox rather than the picture. The shape is what
    // the cell above is for; this is what the number is for.
    let rendered = render_to_image(
        WIDE,
        TALL,
        &fonts,
        theme,
        frame,
        Backdrop {
            image: Some(picture),
            dim: 0.0,
            shape: None,
        },
    )?;

    let ground = f32::from(PALE_GROUND);
    let brightness = |color: sdl3::pixels::Color| {
        (f32::from(color.r) + f32::from(color.g) + f32::from(color.b)) / 3.0
    };

    // The row's own color and its own ring, both at the strength the theme names them. What is being
    // held is the decision: this row stands over a picture rather than through it, so its ink is the
    // ink and not a mix of it with the ground or with its own outline.
    let wanted = brightness(theme.text_dim);
    let ring = brightness(theme.lyric_outline);

    let top = (f32::from(u16::try_from(TALL)?) * 0.11).round() as u32;
    let bottom = top + theme.glyph_box_px(theme.small_size, TALL).round() as u32;
    let left = theme.margin_px(WIDE) as u32;
    let right = WIDE / 2;

    let mut near_fill = 0usize;
    let mut near_ring = 0usize;
    for y in top..bottom.min(TALL) {
        for x in left..right {
            let pixel = rendered.get_pixel(x, y);
            let brightness =
                (f32::from(pixel[0]) + f32::from(pixel[1]) + f32::from(pixel[2])) / 3.0;
            if (brightness - wanted).abs() <= SLACK {
                near_fill += 1;
            }
            if (brightness - ring).abs() <= SLACK {
                near_ring += 1;
            }
        }
    }

    println!(
        "  pale ground: {near_fill} px at the row's own color ({wanted:.0}), \
         {near_ring} px at its ring ({ring:.0}), on a ground of {ground:.0}"
    );
    if near_fill < 40 {
        return Err(format!(
            "the artist row came back with {near_fill} pixels at {wanted:.0}, which is the color \
             the theme names it in; a ring covering the fill, or a fill mixed with the ground or \
             with its own outline, all land somewhere else"
        )
        .into());
    }
    if near_ring < 40 {
        return Err(format!(
            "the artist row came back with {near_ring} pixels at {ring:.0}, so its ring is not at \
             the strength the theme names; the ring is what carries this row over a pale picture"
        )
        .into());
    }
    // Legibility is the whole point of standing at full strength, and over a pale picture it is the
    // ground rather than the ring that a light ink disappears into.
    if ground - wanted < 60.0 {
        return Err(format!(
            "the row's ink at {wanted:.0} and the picture at {ground:.0} are within {:.0} of each \
             other, which is a row nobody reads",
            ground - wanted
        )
        .into());
    }
    Ok(())
}

/// The same disc geometry on a pale ground, which is the case the furniture is hardest on.
///
/// **The top four rows are left flat.** That band is where the machine's own three rows land, so a
/// picture that puts nothing there gives the furniture one known color to stand on and makes
/// [`check_the_furniture_over_a_pale_picture`] an arithmetic question rather than a guess about
/// where a glyph is. A disc writes its own words on rows 14 and 15, and those are kept.
///
/// A ground this light is what separates a glyph that yields from one that goes hollow: a dark ring
/// keeps its contrast against pale whatever it is drawn at, so a fill that mixed with it has nowhere
/// to hide.
fn pale_test_picture() -> image::RgbaImage {
    const VISIBLE_WIDTH: u32 = 288;
    const VISIBLE_HEIGHT: u32 = 192;
    const TILE_W: u32 = 6;
    const TILE_H: u32 = 12;
    const WORDS: [(u32, u32, u32); 2] = [(14, 3, 40), (15, 10, 18)];

    image::RgbaImage::from_fn(VISIBLE_WIDTH, VISIBLE_HEIGHT, |x, y| {
        let row = y / TILE_H;
        let column = x / TILE_W;
        let inked = WORDS.iter().any(|&(word_row, first, last)| {
            row == word_row && column >= first && column < last && column % 5 != 4
        });
        if inked {
            image::Rgba([0x33, 0x44, 0x66, 0xFF])
        } else {
            image::Rgba([PALE_GROUND, PALE_GROUND, PALE_GROUND, 0xFF])
        }
    })
}

/// The one gray [`pale_test_picture`] grounds on, which every number below is measured against.
const PALE_GROUND: u8 = 0xF0;

/// A deliberately hostile wallpaper: bright, high-contrast, with light and dark regions.
///
/// This is the case the lyric outline exists for. Text that reads over this reads over anything, and
/// text that does not would be invisible over a real photograph.
fn harsh_wallpaper(width: u32, height: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = x as f32 / width as f32;
        let fy = y as f32 / height as f32;
        // Broad bright bands crossing a dark field, plus a near-white blowout in the middle where
        // the lyrics sit.
        let band = ((fx * 6.0).sin() * (fy * 4.0).cos()).abs();
        let center = 1.0 - ((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt() * 2.0;
        let value = (band * 140.0 + center.max(0.0) * 200.0).clamp(0.0, 255.0) as u8;
        image::Rgba([value, value.saturating_sub(20), 255 - value / 3, 255])
    })
}

/// One queue entry, for the overlay frames.
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
