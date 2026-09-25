//! Drawing the machine's screen for an encoder instead of for a television.
//!
//! This is [`crate::display`]'s sibling: the same [`km_display::draw::draw`], the same machine
//! state, and a software surface where that one has a window. What it produces is an HLS stream in a
//! directory, which [`km_api::watch`] serves.
//!
//! # The frame is built here rather than shared with the display
//!
//! **A machine with no screen has no keyboard either**, and that is what makes two frame builders
//! the right number rather than a duplication to remove. The display's [`km_display::draw::Frame`]
//! carries a keypad, a flash raised by a key press, and the panels `F2` and `F3` put up — all of
//! them driven by somebody standing at the machine, and none of them reachable here. What is left is
//! a deliberate subset, and writing it out is what lets the display's own frame stay woven into the
//! caches and deadlines its loop needs.
//!
//! # The picture behind the words
//!
//! Whichever kind of song is playing puts its own picture where the wallpaper goes, exactly as on a
//! television. A video song's picture arrives as planes and is converted; an MP3+G song's is already
//! packed; a MIDI song has none and the wallpaper stands.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use km_api::ApiState;
use km_api::machine::Controller;

use km_display::draw::{Frame, Screen, SongInfo};
use km_display::lyrics::LyricView;
use km_display::numbers::NumberEntry;
use km_display::text::Fonts;
use km_display::theme::Theme;
use km_display::wallpaper::{self, Loader, Playlist, WallpaperConfig};
use km_display::{Offscreen, RgbaImage};
use km_queue::Transport;

use crate::machine::Machine;

/// Everything the stream needs that settings and the command line decide.
#[derive(Debug, Clone)]
pub(crate) struct StreamConfig {
    /// Where the playlist and its segments go.
    pub dir: PathBuf,
    /// The size the screen is drawn and encoded at.
    pub width: u32,
    /// See [`StreamConfig::width`].
    pub height: u32,
    /// Frames a second.
    pub fps: u32,
    /// Video bits a second.
    pub bitrate: usize,
    /// Which ffmpeg encoder to use, by name.
    pub encoder: String,
    /// Seconds of video in each segment.
    pub segment_seconds: u32,
    /// How many segments the playlist names at once.
    pub playlist_size: u32,
    /// The lyric font, or `None` to look for one.
    pub font: Option<PathBuf>,
    /// The font that ships beside the executable.
    pub bundled_font: Option<PathBuf>,
    /// A font covering Han, Kana and Hangul.
    pub font_cjk: Option<PathBuf>,
    /// Samples a second the machine renders at.
    pub sample_rate: u32,
    /// Audio bits a second.
    pub audio_bitrate: usize,
    /// Where the wallpapers come from and how they are shown.
    pub wallpaper: WallpaperConfig,
    /// Where the same stream goes as fragments, for the watch page's socket.
    pub live: km_api::watch::Live,
}

/// Draws and encodes until something asks the machine to stop.
///
/// **Returns only when the stream ends**, which is a stop request or a failure the encoder cannot
/// continue past. A failure to *start* is reported to the caller, because a machine asked to stream
/// and unable to is a machine whose whole output is missing.
pub(crate) fn run(
    machine: Arc<Machine>,
    api: ApiState,
    config: StreamConfig,
    mut audio: crate::engine::StreamAudio,
    shutdown: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let ttf = sdl3::ttf::init().map_err(|error| anyhow::anyhow!("no SDL_ttf: {error}"))?;
    let theme = Theme::default();
    let fonts = Fonts::discover(
        &ttf,
        config.font.as_deref(),
        config.bundled_font.as_deref(),
        config.font_cjk.as_deref(),
        // **CJK faces opened up front, unlike the display's.** That one defers them because a
        // rebuild costs one frame on a machine somebody is watching; here a rebuild would stall the
        // encoder, and the frames it is late for are gone rather than merely slow.
        true,
        &theme,
        config.height,
    )
    .map_err(|error| {
        anyhow::anyhow!("no usable font: {error}. Set display.font in settings.json")
    })?;

    let mut offscreen = Offscreen::new(config.width, config.height)
        .map_err(|error| anyhow::anyhow!("could not make a drawing surface: {error}"))?;

    // **The fragments are a copy of what the encoder already made**, handed to the API as they are
    // cut. Publishing never waits, so the frame deadline below cannot be held up by a viewer.
    let live = config.live.clone();
    let fragments: km_stream::Sink = Box::new(move |piece| match piece {
        km_stream::Piece::Init(bytes) => live.publish_init(bytes),
        km_stream::Piece::Fragment { bytes, key } => live.publish_fragment(bytes, key),
    });
    let mut encoder = km_stream::Stream::open_with_fragments(
        &config.dir,
        &km_stream::Config {
            width: config.width,
            height: config.height,
            fps: config.fps,
            bitrate: config.bitrate,
            encoder: config.encoder.clone(),
            segment_seconds: config.segment_seconds,
            playlist_size: config.playlist_size,
            sample_rate: config.sample_rate,
            audio_bitrate: config.audio_bitrate,
        },
        Some(fragments),
    )?;

    let mut walls = Walls::start(&config.wallpaper, config.width, config.height);
    let mut view = LyricView::for_ticks_per_quarter(480);
    let mut loaded_beat_ticks = 0_u16;
    // Nothing types a song number at a machine with no keyboard, so this is the empty one the
    // drawing still asks for.
    let entry = NumberEntry::default();
    let mut picture = Picture::default();
    // Which picture the drawing surface is holding. Nothing is uploaded until this moves.
    let mut uploaded = u64::MAX;
    // The display's own, which re-counts only when the catalog's version moves — so asking every
    // frame costs one comparison on every frame but the few that follow an install.
    let mut summary = crate::display::SummaryCache::new();

    let frame_time = Duration::from_secs_f64(1.0 / f64::from(config.fps.max(1)));
    let mut next_frame = Instant::now();

    // **One frame's worth of sound per frame, and this is the whole of the synchronisation.** The
    // encoder numbers a picture by the count of frames and a packet by the count of samples, so
    // rendering exactly this many samples for every frame drawn puts the words on the beat by
    // arithmetic. Nothing compares two clocks, so nothing can drift between them.
    //
    // A rate the frame rate does not divide would leave a remainder every frame and the two would
    // creep apart; 48 kHz over 30 or 25 or 60 divides exactly, which is what makes the ordinary
    // settings safe. A `sample_rate` chosen to be awkward loses a fraction of a sample per frame.
    let samples_per_frame = (config.sample_rate / config.fps.max(1)) as usize;
    let mut sound = vec![0f32; samples_per_frame * km_stream::encode::CHANNELS];

    tracing::info!(
        width = config.width,
        height = config.height,
        fps = config.fps,
        encoder = %config.encoder,
        dir = %config.dir.display(),
        "streaming"
    );

    while !shutdown.load(Ordering::Acquire) {
        let snapshot = machine.snapshot();
        let song = machine.current_lyric_song();
        let now_kind = snapshot.now_playing.as_ref().map(|now| now.kind);
        let queue = machine.queue();
        let factory_pin = machine.factory_pin();
        let connect = crate::connect::to_display(&api.connect_info(), factory_pin.as_deref());

        if let Some(song) = &song
            && song.beat_ticks() != loaded_beat_ticks
        {
            loaded_beat_ticks = song.beat_ticks();
            view = LyricView::for_ticks_per_quarter(song.beat_ticks());
        }

        let info = snapshot.now_playing.as_ref().map(|now| SongInfo {
            number: match &now.origin {
                km_api::machine::Origin::Catalog { number, .. }
                | km_api::machine::Origin::Demo { number } => Some(*number),
                km_api::machine::Origin::File { .. } => None,
            },
            title: now.title.clone(),
            artist: now.artist.clone(),
            language: now
                .language
                .as_deref()
                .and_then(km_kmpkg::Language::parse)
                .map(|language| language.name().to_owned()),
        });
        let next_up = queue.first().map(km_queue::QueueEntry::label);
        let playing = snapshot.now_playing.is_some();
        let has_own_picture = now_kind.is_some_and(|kind| !kind.draws_words());
        let is_midi = now_kind.is_some_and(|kind| kind.is_midi());

        // **Read from the engine rather than smoothed against a wall clock.** The display
        // interpolates because it draws faster than the position advances and a stepping wipe is
        // visible; here every frame becomes a picture at a fixed rate, and a position invented
        // between two reports would put the words somewhere the sound is not.
        let lyrics = match (&song, snapshot.transport) {
            (Some(song), Transport::Playing | Transport::Paused) => {
                // An UltraStar song follows the audio's position, as the display does.
                let (ticks, tempo_ratio) = if is_midi {
                    (
                        machine.engine().position_ticks(),
                        snapshot.settings.tempo_ratio,
                    )
                } else {
                    (song.tempo_map.ms_to_tick(snapshot.position_ms), 1.0)
                };
                let tick = km_display::lyrics::shift_ticks(
                    &song.tempo_map,
                    ticks,
                    snapshot.settings.lyric_offset_ms,
                    tempo_ratio,
                );
                view.frame(&song.lyrics, tick)
            }
            _ => Default::default(),
        };

        // Only the idle screen draws it, so only the idle screen asks.
        let catalog = (!playing).then(|| summary.get(&machine)).flatten();

        let frame = Frame {
            locale: machine.locale(),
            screen: if playing {
                Screen::Playing
            } else {
                Screen::Idle
            },
            song: info.as_ref(),
            timeline: song.as_ref().map(|song| &song.lyrics),
            lyrics,
            picture: has_own_picture,
            // Always, unlike a television's. The bar there comes and goes because it sits over the
            // words and somebody at the machine can ask for it; nobody watching this can ask, and a
            // song with no way to see how far in it is the thing a singer misses first.
            show_position: playing,
            position_ms: snapshot.position_ms,
            duration_ms: snapshot
                .now_playing
                .as_ref()
                .map_or(0, |now| now.duration_ms),
            transpose: crate::display::drawn_adjustments(now_kind, &snapshot.settings).0,
            tempo_ratio: crate::display::drawn_adjustments(now_kind, &snapshot.settings).1,
            melody: snapshot
                .now_playing
                .as_ref()
                .and_then(|now| now.melody_channel.map(|_| snapshot.settings.melody_enabled)),
            lyrics_hidden: snapshot
                .now_playing
                .as_ref()
                .is_some_and(|now| now.lyrics_hidden),
            connect: Some(&connect),
            catalog,
            number_entry: &entry,
            next_up: next_up.as_deref(),
            demo: snapshot
                .now_playing
                .as_ref()
                .filter(|now| matches!(now.origin, km_api::machine::Origin::Demo { .. }))
                .map(|_| crate::display::DEMO_LABEL),
            // The address is what somebody watching most needs and cannot get any other way, so it
            // stands on the idle screen rather than appearing for a few seconds after a key press.
            show_connect_overlay: !playing,
            queue: &queue,
            show_queue_overlay: false,
            faults: crate::display::faults(&machine),
            version: Some(crate::display::VERSION),
            // Every one of these is raised by somebody at the machine, and there is nobody there.
            flash: None,
            keypad: None,
            soundfont_label: None,
            developer_mode: None,
            performance: None,
            song_stats: None,
        };

        let behind = picture.take(&machine, &snapshot, &mut walls, &config);
        // **Uploaded when the picture changes and not once a frame.** Converting an image and
        // building a texture from it is megabytes of work at this size, which is nothing for a
        // wallpaper that stands for minutes and is the whole frame budget if it is repeated
        // thirty times a second.
        if behind.generation != uploaded {
            uploaded = behind.generation;
            if let Err(error) = offscreen.set_picture(behind.backdrop.image) {
                tracing::warn!(%error, "could not put a picture behind the words");
            }
        }
        offscreen.draw(
            &fonts,
            &theme,
            &frame,
            behind.backdrop.dim,
            behind.backdrop.shape,
        );

        let pushed = offscreen.with_pixels(|bgra| encoder.push(bgra));
        if let Err(error) = pushed {
            tracing::error!(%error, "the encoder stopped; the stream is over");
            break;
        }

        // **Rendered after the frame was drawn and before the next one is.** The position the frame
        // was drawn from is the one this block starts at, so the words on that picture are the words
        // these samples sing — and between songs the machine renders silence, which is what keeps
        // the timeline running when nothing is playing.
        audio.render(&mut sound);
        if let Err(error) = encoder.push_audio(&sound) {
            tracing::error!(%error, "the sound stopped; the stream is over");
            break;
        }

        // **Paced against a running deadline rather than by sleeping a frame's worth.** Sleeping
        // after the work makes every frame late by however long the work took, and the drift
        // accumulates into a stream whose clock runs slow against the sound it will carry.
        next_frame += frame_time;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else if now - next_frame > frame_time {
            // Behind by more than a frame: give up on catching up rather than encoding a burst.
            next_frame = now;
        }
    }

    tracing::info!("closing the stream");
    encoder.finish()?;
    Ok(())
}

/// How many numbers the wallpapers are given before a song's own pictures begin.
///
/// Nothing counts this high; it exists so the two sources cannot collide on their first picture.
const SONG_GENERATIONS: u64 = 1 << 32;

/// What goes behind the words this frame, and whether it is the one already uploaded.
///
/// **The generation is what keeps a wallpaper off the critical path.** Handing a picture to
/// [`Offscreen::set_picture`] converts the whole image and builds a texture from it, which at 1080p
/// is megabytes of work — right for a picture that has changed and ruinous once a frame for one that
/// stands for minutes. Every source below numbers its pictures, and the loop uploads only when the
/// number moves.
struct Behind<'a> {
    backdrop: km_display::Backdrop<'a>,
    generation: u64,
}

/// What goes behind the words this frame.
#[derive(Default)]
struct Picture {
    /// The image that was made from it.
    image: Option<RgbaImage>,
    /// Bumped whenever [`Picture::image`] is replaced.
    generation: u64,
    /// Whether the last frame drew a song's picture, so the wallpaper is reinstated when it ends.
    had_song_picture: bool,
}

impl Picture {
    /// Takes whichever picture belongs behind this frame.
    ///
    /// **One lifetime over both sources, because the answer comes from whichever has a picture.**
    /// A song's own is held here and a wallpaper is held by the loader, so a backdrop borrowing from
    /// one of two places has to promise the caller only that both outlive it.
    fn take<'a>(
        &'a mut self,
        machine: &Machine,
        snapshot: &km_api::machine::Snapshot,
        walls: &'a mut Walls,
        config: &StreamConfig,
    ) -> Behind<'a> {
        if let Some(shape) = self.song_picture(machine, snapshot) {
            self.had_song_picture = true;
            return Behind {
                backdrop: km_display::Backdrop {
                    image: self.image.as_ref(),
                    // No scrim over a song's own picture: the scrim keeps *drawn* words legible
                    // over an arbitrary photograph, and darkening a song's own words only makes
                    // them harder.
                    dim: 0.0,
                    shape: Some(shape),
                },
                // **Offset past the wallpaper's numbers**, so that going from one to the other is
                // always a change even if both happen to be on their first picture.
                generation: SONG_GENERATIONS + self.generation,
            };
        }
        if self.had_song_picture {
            self.had_song_picture = false;
            self.image = None;
        }
        let (image, generation) = walls.current();
        Behind {
            backdrop: km_display::Backdrop {
                image,
                dim: config.wallpaper.dim,
                shape: None,
            },
            generation,
        }
    }

    /// Converts the playing song's own picture, and says what shape to letterbox it to.
    fn song_picture(
        &mut self,
        machine: &Machine,
        snapshot: &km_api::machine::Snapshot,
    ) -> Option<(u32, u32)> {
        if let Some(song) = machine.current_video() {
            let frames = song.frames();
            if let Some(frame) = frames.take_frame_for(snapshot.position_ms) {
                let (width, height) = (frame.width as usize, frame.height as usize);
                // **Converted straight into the picture already held**, which is what keeps a video
                // song from allocating. A 1080p frame is eight megabytes packed, and building a new
                // image thirty times a second is the churn `video.md` names: it surfaces later as a
                // stutter nobody can place. One song's frames never change size, so the buffer is
                // made once and written over from then on.
                let fits = self.image.as_ref().is_some_and(|held| {
                    held.width() == frame.width && held.height() == frame.height
                });
                if !fits {
                    self.image =
                        RgbaImage::from_raw(frame.width, frame.height, vec![0; width * height * 4]);
                }
                let (y, y_stride) = frame.y();
                let (u, u_stride) = frame.u();
                let (v, v_stride) = frame.v();
                let shape = (frame.width, frame.height);
                if let Some(image) = self.image.as_mut() {
                    km_stream::yuv420p_to_rgba(
                        &km_stream::Source {
                            bytes: y,
                            stride: y_stride,
                        },
                        &km_stream::Source {
                            bytes: u,
                            stride: u_stride,
                        },
                        &km_stream::Source {
                            bytes: v,
                            stride: v_stride,
                        },
                        width,
                        height,
                        image,
                    );
                    // Numbered even though the buffer did not move: what is behind the words is a
                    // new picture every frame, and the drawing surface has to be told.
                    self.generation += 1;
                }
                // Straight back to the decoder's pool, so the next picture reuses these buffers.
                frames.recycle(frame);
                return Some(shape);
            }
            // Between pictures, so the one already held stands.
            return self
                .image
                .as_ref()
                .map(|held| (held.width(), held.height()));
        }

        if let Some(song) = machine.current_cdg() {
            let frames = song.frames();
            if let Some(frame) = frames.take_frame_for(snapshot.position_ms) {
                let (width, height) = (frame.width(), frame.height());
                // Already packed, and native-endian `ARGB8888` — which is BGRA in memory, so the
                // channels are swapped on the way into an RGBA image.
                let (pixels, _) = frame.pixels();
                let mut rgba = vec![0u8; pixels.len()];
                for (from, to) in pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(rgba.as_chunks_mut::<4>().0)
                {
                    *to = [from[2], from[1], from[0], from[3]];
                }
                self.image = RgbaImage::from_raw(width, height, rgba);
                self.generation += 1;
                // A CD+G pixel is not square: the picture is presented at 4:3, not at the 3:2 its
                // own size would imply, because it was drawn for a television.
                return Some(km_cdg::DISPLAY_ASPECT);
            }
            return self.image.as_ref().map(|_| km_cdg::DISPLAY_ASPECT);
        }

        None
    }
}

/// The wallpaper, loaded and rotated.
///
/// **Simpler than the display's**, and the difference is what nobody watching can do: there is no
/// key to press for the next picture, so there is no crossfade to run and no request to service
/// out of turn. One picture stands until the interval moves to the next.
struct Walls {
    playlist: Playlist,
    loader: Loader,
    current: Option<RgbaImage>,
    /// Bumped whenever a new picture arrives, so the drawing surface uploads once per picture.
    generation: u64,
    next_change: Instant,
    interval: Duration,
    width: u32,
    height: u32,
    fit: wallpaper::Fit,
    waiting: bool,
}

impl Walls {
    fn start(config: &WallpaperConfig, width: u32, height: u32) -> Self {
        let playlist = Playlist::scan(&config.dir, &config.extra);
        let mut walls = Self {
            playlist,
            loader: Loader::start(),
            current: None,
            generation: 0,
            next_change: Instant::now(),
            interval: config.interval,
            width,
            height,
            fit: config.fit,
            waiting: false,
        };
        walls.ask();
        walls
    }

    /// Asks the loader for whatever the playlist is pointing at.
    fn ask(&mut self) {
        if let Some(source) = self.playlist.current() {
            self.waiting = self
                .loader
                .request(source.clone(), self.width, self.height, self.fit);
        }
    }

    /// The picture to draw and which one it is, taking whatever the loader has finished.
    fn current(&mut self) -> (Option<&RgbaImage>, u64) {
        if self.waiting
            && let Some(loaded) = self.loader.poll()
        {
            self.waiting = false;
            match loaded {
                Ok(image) => {
                    self.current = RgbaImage::from_raw(image.width, image.height, image.rgba);
                    self.generation += 1;
                    self.next_change = Instant::now() + self.interval;
                }
                Err(error) => {
                    tracing::warn!(%error, "a wallpaper would not load");
                    self.next_change = Instant::now() + self.interval;
                }
            }
        }
        if !self.waiting && Instant::now() >= self.next_change && self.playlist.len() > 1 {
            self.playlist.advance();
            self.ask();
        }
        (self.current.as_ref(), self.generation)
    }
}
