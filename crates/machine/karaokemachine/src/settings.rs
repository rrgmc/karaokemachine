//! `settings.json`, and where everything lives on disk.
//!
//! One file under the platform config directory, holding everything the machine remembers between
//! runs. Two rules shape it:
//!
//! **Every field has a default and nothing is required.** A settings file written by an older build,
//! hand-edited into an odd shape, or deleted entirely must still start the machine. A karaoke
//! machine that refuses to boot because a JSON key is missing is a broken appliance, so unknown keys
//! are ignored and missing ones fall back.
//!
//! **The defaults are the ones that make the product work, not the ones that make it inert.** The
//! machine binds `0.0.0.0` and serves every endpoint without a password, because a karaoke machine
//! whose remote control nobody's phone can reach is the broken case rather than the safe one — the
//! brief calls a phone acting as the remote the intended use, and mDNS, the QR code and the connect
//! panel all exist to serve it. Anybody in the room being able to queue a song is the design.
//!
//! **One endpoint is admin by default, and it is the exception that shows the rule.** `audio.write`
//! chooses which socket the machine's sound comes out of: configuration set once when the box goes
//! under the television, not a knob that belongs to a performance. "Anybody in the room may queue a
//! song" does not extend to "anybody in the room may move the sound somewhere nobody is listening".
//!
//! That is a judgment about a **home LAN**, and it stops being right the moment the port is
//! reachable from outside one. The machine answers that itself now rather than leaving it to whoever
//! reads this: every route that writes lives under `/api/v1/admin/` and demands the password, a
//! machine always has one, and installing a package or playing a file off the disk are both behind
//! the debugging switch on top of that. An owner forwarding 8177 through a router should still not,
//! but nothing is left unguarded while they decide.
//!
//! (This paragraph named an ACL and four permission strings until 2026-09-07. There is no access
//! list any more — the URL prefix is the permission; see `km_api::routes`.)

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use km_display::lyrics::MAX_LYRIC_OFFSET_MS;
use km_display::wallpaper::Fit;
use serde::{Deserialize, Serialize};

/// Where everything lives on this machine — see the module's own header.
mod paths;

// Re-exported flat, so `settings::Paths` and `settings::APP_NAME` are still what every caller
// writes. The submodule is private for that reason: one public path per item, and it is the one that
// already existed.
pub use self::paths::*;

/// How the HTTP API is set up.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    /// What to listen on. `0.0.0.0:8177` by default, so a phone on the same network can connect;
    /// `127.0.0.1:8177` to shut the remote out of everything but this machine.
    pub bind: String,
    /// `argon2` hash of the admin password. Never the password itself.
    ///
    /// **`None` only until the first start.** [`Settings::load`] generates a factory PIN and hashes
    /// it into this field, so a running machine always has a password — which is what makes every
    /// route under `/api/v1/admin/` demand one, rather than the marks lying dormant as they used to.
    pub admin_password_hash: Option<String>,
    /// The generated PIN, in plain text, for as long as it is still the one in force.
    ///
    /// **Plain text on purpose, and bounded.** The machine has to be able to draw it on its own
    /// screen, which a hash cannot do; the data directory is `0700`; and anybody who can read this
    /// file can read the hash beside it and owns the box either way. `Some` *is* the definition of
    /// "still on the factory password" — setting one of the owner's own clears it, and nothing puts
    /// it back except a reset.
    pub admin_factory_pin: Option<String>,
    /// How long an admin token lasts.
    pub token_ttl_secs: u64,
    /// Bumped to end every admin session at once.
    ///
    /// A token is an HMAC over the password hash *and* this, so moving it invalidates all of them
    /// without changing the password. It has to be here rather than in memory: an epoch that did not
    /// survive a restart would quietly un-revoke every session an owner had just ended.
    pub session_epoch: u64,
    /// Whether to advertise over mDNS.
    ///
    /// **Defaults off on iOS, and that is the platform rather than a preference.** `mdns-sd` opens a
    /// raw multicast socket, and Apple has required `com.apple.developer.networking.multicast` for
    /// multicast since iOS 14 and grants it only after a manually reviewed request. Without the
    /// entitlement the packets are dropped and the application is not told, so a machine would
    /// announce nothing while every log line said it had — which is the failure shape
    /// `Finding a machine without multicast` refuses on the remote's side of the same problem.
    ///
    /// **Costing less than it sounds**, because the iOS remote finds a machine by unicast sweep
    /// against `GET /api/v1/discover` rather than by browsing. The Android remote does browse, and
    /// is given the address by hand.
    ///
    /// Still a setting rather than a `cfg!`, on this file's usual bargain: somebody who obtains the
    /// entitlement turns it on without a rebuild, and the seam `run_advertiser` already reads is
    /// where an `NWListener` would arrive.
    pub advertise_mdns: bool,
    /// Whether to serve the development remote at `/dev/`.
    ///
    /// `None` means the default, which is **off**. Set it to `true` to serve the page, or pass
    /// `--dev-remote` for one run.
    pub serve_dev_remote: Option<bool>,
    /// Whether to serve the singer-facing remote at `/`.
    ///
    /// `None` means the default, which is **on**: it is the product surface every phone in the room
    /// reaches, and a machine that served nothing at its own address would be a machine whose QR code
    /// leads to a landing page. Set it to `false` and `/` explains where things are instead.
    pub serve_remote: Option<bool>,
    /// Origins allowed to call the API cross-origin. Empty unless developing a remote separately.
    pub cors_origins: Vec<String>,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self {
            bind: format!("0.0.0.0:{}", km_api::DEFAULT_PORT),
            admin_password_hash: None,
            admin_factory_pin: None,
            token_ttl_secs: km_api::auth::DEFAULT_TOKEN_TTL.as_secs(),
            session_epoch: 0,
            advertise_mdns: !cfg!(target_os = "ios"),
            serve_dev_remote: None,
            serve_remote: None,
            cors_origins: Vec::new(),
        }
    }
}

impl ApiSettings {
    /// The bind address, or loopback on the default port when it will not parse.
    ///
    /// A typo here would otherwise stop the machine starting. Reported and worked around: loopback
    /// is the safe fallback, and the connect panel will say the remote is local-only, so the
    /// operator sees the consequence on screen rather than in a log they never read.
    pub fn socket_addr(&self) -> SocketAddr {
        match self.bind.parse() {
            Ok(addr) => addr,
            Err(_) => {
                tracing::error!(
                    bind = %self.bind,
                    "could not parse the bind address; falling back to loopback"
                );
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), km_api::DEFAULT_PORT)
            }
        }
    }
}

/// Audio output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// The **bank id** of a `.sf2` in the SoundFont folder, to use instead of the bundled one.
    ///
    /// An id — the slug [`crate::soundfont::bank_id`] makes of a filename — and **not a path**.
    /// The id is looked up in [`crate::soundfont::installed`] on every read,
    /// exactly as `bank_id`'s own doc says ids are for, so the folder is what says which banks
    /// exist and this says which of them was chosen. `None` means the bundled bank.
    ///
    /// **A path here would be the same fault naming individual package files is**, with a worse
    /// symptom. Deleting the file it named does not fall back: the engine refuses, the machine comes
    /// up on a **sine test tone**, the reason reaches only a log line and the API's `problem` field
    /// — and the picker then reports *"Bundled"* as selected, because the row for a missing file is
    /// dropped and the lookup falls through to the default. So the one screen that could say what is
    /// wrong says the opposite. With an id, a name that matches nothing falls back to bundled
    /// **with a reason**, which is a working machine that says what it did.
    ///
    /// **The JSON key is `soundfont` and holds an id.** A path and an id are both a JSON string, so
    /// nothing at parse time can tell one from the other — which is what [`CURRENT_SETTINGS_VERSION`]
    /// answers: a file at the current version means an id here.
    ///
    /// To point the machine at a bank that is *not* in the folder, use `debug.soundfonts` —
    /// [`DebugSettings::soundfonts`] still takes paths, because naming a file is what `debug.` is
    /// for. `--set-soundfont` takes a path too and **installs** the bank into the folder before
    /// choosing it by id.
    pub soundfont: Option<String>,
    /// Backing-track level, 0.0 to 1.0.
    pub music_volume: f32,
    /// Seconds of idleness after which the output device is handed back to the system.
    ///
    /// `0` never releases it, which is the behavior up to now and the right one for a box under a
    /// television whose speakers nothing else uses. The default of five seconds exists for the
    /// opposite case: on a laptop with Bluetooth headphones, holding the endpoint for the life of
    /// the process was enough to stop every other application making a sound.
    pub idle_release_secs: u32,
    /// Which output to play through, as a `cpal::DeviceId` in `Display` form.
    ///
    /// Three states, and the difference between the first two is the whole point:
    ///
    /// * **Absent** — nobody has chosen, and this is the ordinary state rather than a transient
    ///   one. The machine follows the system, except on Linux, where it prefers a USB interface if
    ///   there is one. That preference is re-applied on **every** start and is never written here:
    ///   nothing but a person ever puts a value in this field.
    /// * **`"system"`** — follow whatever the system calls the default, *chosen deliberately*. That
    ///   is why it is stored rather than left absent: it is the only way to say "do not prefer USB"
    ///   and have it stick.
    /// * **An identifier** — that device, and nothing else. If it is not present the machine falls
    ///   back to the system default for that run and **leaves this alone**, so unplugging the
    ///   interface for one evening does not lose the choice.
    ///
    /// An identifier rather than a name, unlike the microphones' `device_hint`: a name is a hint and
    /// this has to be right. See `km_audio::device`.
    pub output_device: Option<String>,
    /// Bring a video or MP3+G song down to the level a MIDI song plays at.
    ///
    /// **On, and the switch exists because this changes how every media song sounds.** An owner who
    /// has already balanced their amplifier around the old behaviour needs a way back that is not
    /// rebuilding their packages, and a machine whose measurements turn out wrong needs one too.
    ///
    /// Off means every song plays at whatever level it was published at, which is what happened
    /// before this existed. It does not un-measure anything: the numbers stay in the packages and
    /// turning it on again uses them.
    ///
    /// **Not on the API**, unlike `music_volume` beside it. It is a statement about how this machine
    /// is set up rather than an adjustment to a performance — the same standing as
    /// [`Self::idle_release_secs`] — and the four things a guest's phone may change are the key, the
    /// tempo, the volume and the guide melody. See `Video and MP3+G play at the MIDI reference
    /// level` in `docs/decisions/audio.md`.
    pub normalize_media: bool,

    /// Bring a MIDI song to the level its bank renders the corpus at, raising a quiet one.
    ///
    /// **On, and a key of its own rather than a share of [`Self::normalize_media`].** The two are
    /// separate decisions about two kinds of song: an owner who has balanced a room around
    /// unlevelled media may still want their quiet MIDI files brought up, and the argument the other
    /// way holds as well. This is also the riskier half, because it is the only levelling that can
    /// make a song *louder*, so it needs a way back that does not take media levelling with it.
    ///
    /// Off means every MIDI song plays at whatever its own file and the bank produce, which is a
    /// spread of about 13 LU across the corpus.
    ///
    /// **Not on the API**, on the same standing as the key above.
    pub normalize_midi: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            soundfont: None,
            music_volume: 1.0,
            idle_release_secs: 5,
            output_device: None,
            normalize_media: true,
            normalize_midi: true,
        }
    }
}

impl AudioSettings {
    /// The idle delay, or `None` to hold the device for the whole session.
    pub fn idle_release(&self) -> Option<Duration> {
        (self.idle_release_secs > 0).then(|| Duration::from_secs(u64::from(self.idle_release_secs)))
    }
}

/// The window and the fonts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    /// Whether to start fullscreen.
    ///
    /// **Defaults to off, and the setup programs are what turn it on.** A karaoke machine drives a
    /// television, so an *installed* machine wants the whole screen — but a default reaches an
    /// install with no settings file, and the other thing with no settings file is a checkout
    /// somebody has just built. Defaulting on meant every `cargo run` took the whole screen away
    /// from the person debugging it, with no flag to say otherwise.
    ///
    /// So the two cases are separated where they actually differ, which is who put the machine
    /// there. A build run out of a checkout or a portable folder opens a window; a machine a setup
    /// program installed gets `{"display": {"fullscreen": true}}` written for it, and only where
    /// there is no settings file to overwrite. See `The setup programs pre-write a settings file`
    /// in `docs/decisions/distribution.md`.
    ///
    /// **Written back when the display closes**, so `F` outlives the process: a machine left
    /// fullscreen starts fullscreen. Only where the value actually changed, and only on a run that
    /// was not told what to do — `--fullscreen` and `--windowed` override it for one run and write
    /// nothing down, which is the same discipline `--api-bind` keeps and is what stops a debugging
    /// run against an appliance's `--data-dir` from rewriting it. Android never writes it: fullscreen
    /// is fixed there. See `Windowed mode` in `docs/decisions/interface.md`.
    pub fullscreen: bool,
    /// Whether the window stays in front of every other application.
    ///
    /// **For the machine that shares a screen.** One driving a television owns the panel and will
    /// never want this; one on a desk beside a browser and a chat window goes behind them on the
    /// first click, taking the words with it. Fullscreen is not the same answer, because what is
    /// wanted here is the other windows still being usable.
    ///
    /// Defaults to off, and no setup program pre-writes it: an installed machine is the television
    /// case, which is the one that does not want it.
    ///
    /// **Written back when the display closes**, so `T` outlives the process, and only where the
    /// value actually changed — the same shape as [`DisplaySettings::fullscreen`] next door, minus
    /// the flag half. There is no `--always-on-top`, so nothing declares a run temporary and every
    /// run may speak for the machine. Android never writes it: there are no windows to stack.
    /// See `The window can be told to stay in front` in `docs/decisions/interface.md`.
    pub always_on_top: bool,
    /// Window width when not fullscreen.
    ///
    /// **Written back when the display closes in a window**, with [`Self::height`], [`Self::x`] and
    /// [`Self::y`], so the window opens where it was left. Only where the rect changed, and on any
    /// run: `--fullscreen` and `--windowed` override fullscreen and nothing else. Android never
    /// writes it. See `The window opens where it was left` in `docs/decisions/interface.md`.
    pub width: u32,
    /// Window height when not fullscreen. See [`Self::width`].
    pub height: u32,
    /// The left edge of the window when not fullscreen, in desktop coordinates. `None` centers it.
    ///
    /// Only a pair with [`Self::y`] places the window, and a pair that lands on no connected
    /// display is ignored, so a monitor that has been unplugged cannot hold the window off screen.
    pub x: Option<i32>,
    /// The top edge of the window when not fullscreen. See [`Self::x`].
    pub y: Option<i32>,
    /// A `.ttf` to use instead of the bundled or platform font.
    pub font: Option<PathBuf>,
    /// A `.ttf` or `.ttc` to draw Han, Kana and Hangul with, behind whichever font is in use.
    ///
    /// **Only opened once a song has actually asked for one of those glyphs**, and only needed where
    /// `km_display::text::find_cjk_fonts`'s built-in list misses — a Linux distribution that puts
    /// Noto CJK somewhere else, say. Naming one here replaces that list rather than adding to it.
    ///
    /// This is a path and not a folder, which the `Outside `debug.`, `settings.json` never persists
    /// the path of an individual content file` non-goal permits: a font is not content. `font`
    /// beside it has always been a path for the same reason.
    pub font_cjk: Option<PathBuf>,
    /// Whether to show the on-screen keypad at all — the number pad and the transport strip both.
    ///
    /// On a touch device it is the only way to work the machine, so it defaults on. An owner driving
    /// it entirely from a keyboard or a phone can turn it off. This is the master switch; a press
    /// lands nowhere when it is false. See also [`DisplaySettings::number_pad`].
    pub keypad: bool,
    /// Whether the idle screen carries the song-number pad. **Defaults to on where there is no
    /// keyboard, which means Android and iOS, and on an Android television too.**
    ///
    /// The pad exists because a phone has no keyboard, and there it is the only way to enter a song
    /// number. On a desktop the keyboard is in front of you and typing a number already works, so
    /// the pad is chrome — and it is chrome on the half of the screen the idle layout otherwise
    /// leaves clear. The transport strip is not covered by this and still appears on every platform,
    /// because it is transient and names actions rather than duplicating a key somebody can see.
    ///
    /// **On Android that includes a television, and the exception that used to exclude one is
    /// gone.** It read: "the pad was drawn across half the idle screen where nothing could ever
    /// press it", and asked `SDL_IsTV()` so a Google TV box got no pad. The premise was false. The
    /// D-pad focus work — `Keypad::move_focus` and `activate` — was built for exactly that remote,
    /// and a physical Google TV remote has since been confirmed driving it: arrows move the
    /// highlight, OK presses what is highlighted. So a pad on a television *is* pressable, and
    /// removing it did not save a screen from useless chrome — it left a machine that could not be
    /// asked for a song at all without a second device.
    ///
    /// **That is the cost that decided it.** A pad costs half an idle screen; no pad costs the
    /// ability to start the machine. The first is a layout opinion and the second is whether the
    /// product works when nobody can find their phone.
    ///
    /// Still a setting rather than a bare `cfg!`, because the remaining cases come apart too: a
    /// desktop touchscreen, or a machine driven by a phone alone, wants the pad and can ask for it,
    /// and a tablet plugged into a keyboard can decline it. The platform decides the default and
    /// nothing else.
    ///
    /// **iOS takes the same default for the same reason, and is the case that shows why it is a
    /// setting.** An iPad with a keyboard attached is an ordinary thing and the platform cannot be
    /// asked which one this is, so the guess is made where a wrong one costs a pad somebody can
    /// turn off rather than a machine nobody can enter a number on.
    pub number_pad: bool,
    /// Whether to show the display at all.
    ///
    /// Off gives a headless machine: the API, the engine and the catalog, no window. Useful on a
    /// box with no screen attached, and the only way to run on a machine with no GPU.
    pub enabled: bool,
    /// How far ahead of the audio to draw the lyric highlight, in milliseconds.
    ///
    /// Positive means the lyrics lead, which is what compensates a late picture: a television adds
    /// panel processing, and a rig that takes audio out of the HDMI chain early -- which is what
    /// mixing microphones in hardware requires -- shortens the audio path and leaves the video path
    /// alone. Realistic values are 0-60; the range is +/-500.
    ///
    /// It calibrates a *room*, so unlike transpose and tempo it is not reset between songs. It moves
    /// nothing but this machine's own drawing: not the audio, not the engine, and not the
    /// `lyric_line` events the API publishes, which other clients time against their own screens.
    ///
    /// Clamped wherever it is read rather than rejected on load, so a hand-edited file cannot stop
    /// the machine booting. See "The lyric timing offset" in `docs/ARCHITECTURE.md`.
    pub lyric_offset_ms: i16,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            // Off. A default reaches only a machine with no settings file: an appliance under a
            // television keeps the fullscreen it was given on its first start, and what a default
            // decides is what a *fresh* start assumes — far more often a checkout than a television.
            fullscreen: false,
            // Off, and a key of its own, so `#[serde(default)]` fills it on every file.
            always_on_top: false,
            width: 1280,
            height: 720,
            // Centered until a window has been closed somewhere. New keys, so `#[serde(default)]`
            // fills them on every file ever written.
            x: None,
            y: None,
            font: None,
            font_cjk: None,
            keypad: true,
            // `cfg!` rather than `#[cfg]` so both branches typecheck on every platform, the same
            // way `display::FULLSCREEN_IS_FIXED` is written.
            number_pad: cfg!(any(target_os = "android", target_os = "ios")),
            enabled: true,
            lyric_offset_ms: 0,
        }
    }
}

impl DisplaySettings {
    /// The lyric offset, held to its range.
    ///
    /// The clamp lives here rather than at the two places that read it, and rather than in `load`,
    /// for the same reason [`MicSettings`] clamps on conversion: a hand-edited settings file can
    /// hold anything, and refusing to boot over one number would be a worse answer than behaving as
    /// if it held the limit. Reading it through this method is what makes that true everywhere.
    pub fn lyric_offset(&self) -> i16 {
        self.lyric_offset_ms
            .clamp(-MAX_LYRIC_OFFSET_MS, MAX_LYRIC_OFFSET_MS)
    }

    /// Where the window goes when it is not fullscreen.
    ///
    /// A position is a pair or nothing: a file holding only `x` centers the window rather than
    /// guessing the other half.
    pub fn window_rect(&self) -> WindowRect {
        WindowRect {
            position: self.x.zip(self.y),
            width: self.width,
            height: self.height,
        }
    }

    /// Takes a window rect back into the four keys it is stored as.
    pub fn set_window_rect(&mut self, rect: WindowRect) {
        self.x = rect.position.map(|(x, _)| x);
        self.y = rect.position.map(|(_, y)| y);
        self.width = rect.width;
        self.height = rect.height;
    }
}

/// A window's place on the desktop when it is not fullscreen.
///
/// Window coordinates rather than pixels, which is what SDL both reports and takes, so a rect read
/// on a high-density display goes back onto it at the same size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRect {
    /// The top-left corner, or `None` for centered on the primary display.
    pub position: Option<(i32, i32)>,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// The wallpaper cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperSettings {
    /// Folder of still images. Rescanned each cycle, so files can be added while running.
    ///
    /// A missing or empty folder is not an error — the display falls back to a generated gradient.
    ///
    /// `None` means the bundled folder, wherever that turns out to be on this platform. It is an
    /// `Option` rather than a default path precisely so that "the bundled one" survives being
    /// written to `settings.json` and read back on a machine whose asset directory is somewhere
    /// else — which is every Android install, and any desktop install that moves.
    pub dir: Option<PathBuf>,
    /// Seconds between changes.
    pub interval_secs: u32,
    /// Crossfade length in milliseconds.
    pub crossfade_ms: u32,
    /// Whether to shuffle.
    pub shuffle: bool,
    /// Whether a song starting changes the picture, on top of the interval.
    ///
    /// Not part of [`km_display::WallpaperConfig`]: the display crate times a cycle and has never
    /// heard of a song, so the trigger belongs to the machine and reaches the display through the
    /// same request flag `POST /wallpapers/next` uses.
    pub on_song_change: bool,
    /// Cover or contain.
    pub fit: Fit,
    /// Darkening scrim, 0.0 to 1.0, so lyrics stay readable over a bright image.
    pub dim: f32,
}

impl Default for WallpaperSettings {
    fn default() -> Self {
        let defaults = km_display::WallpaperConfig::default();
        Self {
            dir: None,
            interval_secs: defaults.interval.as_secs() as u32,
            crossfade_ms: defaults.crossfade.as_millis() as u32,
            shuffle: defaults.shuffle,
            on_song_change: true,
            fit: defaults.fit,
            dim: defaults.dim,
        }
    }
}

impl WallpaperSettings {
    /// The display crate's configuration.
    ///
    /// `paths` resolves the folder when settings name none — the owner's own, the checkout overlay,
    /// or the shipped set, in that order and by contents rather than existence. See
    /// [`Paths::wallpaper_dir`].
    ///
    /// **`wallpaper.dir` still wins outright**, and unconditionally: a folder somebody named is
    /// their answer even when it is empty, because the alternative is a setting that silently stops
    /// applying. The three rules below it are for the case where nobody has said.
    ///
    /// The extras are empty here; [`Settings::wallpaper_config`] is what fills them, because they
    /// come from the `debug.` section which this struct cannot see.
    pub fn to_config(&self, paths: &Paths) -> km_display::WallpaperConfig {
        km_display::WallpaperConfig {
            dir: self.folder(paths).0,
            extra: Vec::new(),
            interval: Duration::from_secs(u64::from(self.interval_secs.max(1))),
            crossfade: Duration::from_millis(u64::from(self.crossfade_ms)),
            shuffle: self.shuffle,
            fit: self.fit,
            dim: self.dim.clamp(0.0, 1.0),
        }
    }

    /// Which folder the wallpapers come from, and which rule said so.
    ///
    /// **One definition with three readers** — this, `Machine::wallpaper_folder` for the display
    /// loop, and `--show-paths` — rather than the rule being restated at each. It was written out
    /// twice before and the two agreed by luck.
    pub fn folder(&self, paths: &Paths) -> (PathBuf, WallpaperSource) {
        match &self.dir {
            Some(named) => (named.clone(), WallpaperSource::Setting),
            None => paths.wallpaper_dir(),
        }
    }
}

/// Playback defaults, applied to each new song.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackSettings {
    /// Semitones, applied on top of a song's own stored default.
    pub transpose: i8,
    /// Tempo multiplier.
    pub tempo_ratio: f32,
    /// Whether the guide melody starts on.
    ///
    /// **On.** The argument against — that the machine's job is the backing track, and a melody
    /// nobody asked for competes with the singer — answers a different question. A guide melody is
    /// what makes a song somebody half-knows singable at all, which is most of what a home machine
    /// is asked to play; the singer who does not want it turns it off, and that is one keypress from
    /// the display or one tap from a remote. Starting it off means the person who needs it most has
    /// to know it exists before they can find it.
    ///
    /// It costs nothing where it cannot help: [`crate::machine::Machine::start`] ands this with
    /// whether the song has a *confidently detected* melody channel, so a file where detection
    /// abstained plays with no guide melody whatever this says.
    pub melody_enabled: bool,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            transpose: 0,
            tempo_ratio: 1.0,
            melody_enabled: true,
        }
    }
}

/// Where this machine's log goes, how much detail is in it, and how much of it is kept.
///
/// **Shared with the package builder and the picture-and-bank tool**, which are the other two
/// programs here that keep a settings file and can be started without a command line. The section
/// is the same in all three, and [`km_logsettings`] says why it is a crate rather than three
/// copies.
pub use km_logsettings::{KeepSetting, LoggingSettings, ViewerSetting};

/// What the stream looks like, for a run that draws for an encoder instead of a television.
///
/// **Carried by every machine, read only by the one streaming.** A machine with a television keeps
/// these and ignores them, which is what lets the stream be set up from `/admin/` on the machine
/// somebody is standing at and then be right when the same data directory is next started with
/// `--stream`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamSettings {
    /// The size the screen is drawn and encoded at. Both must be even.
    ///
    /// **1080p, so a video song maps one pixel to one.** The packaging profile caps a song at
    /// 1080p, so at this size the commonest source is neither scaled up nor down — and scaling is
    /// where most of what one generation of re-encoding costs would otherwise come from.
    pub width: u32,
    /// See [`StreamSettings::width`].
    pub height: u32,
    /// Frames a second.
    ///
    /// **Thirty, matching the packaging profile's ceiling.** Sixty would double both the drawing
    /// and the encoding to carry pictures a packaged song does not have.
    pub fps: u32,
    /// Video bits a second.
    ///
    /// **Generous on purpose.** The clients are on the same house network, so there is nothing to
    /// be gained by compressing hard, and a high rate is what keeps the re-encode away from
    /// anything a person can see.
    pub bitrate: usize,
    /// Which ffmpeg encoder to use, by its own name for it.
    ///
    /// **A name rather than a choice the machine makes**, so a hardware encoder is tried by setting
    /// one and measuring. `h264_nvenc`, `h264_qsv`, `h264_amf` and `h264_mf` are hardware ones a
    /// build may have. A name this build does not have is reported at startup rather than when the
    /// first segment fails to appear.
    ///
    /// `auto` is the one value that is not a name. It takes whichever software H.264 encoder the
    /// ffmpeg behind it carries — `libopenh264` in a build this project makes, `libx264` in a
    /// distribution's own — because that is settled by whoever configured ffmpeg and not by
    /// anything the machine can see beforehand.
    ///
    /// This is not the automatic choice `Which H.264 encoder` refuses: that is about a packaging
    /// run picking for itself, where a missing graphics card fails in the middle of a batch. These
    /// are two software encoders of the same codec, either of which is simply present or absent.
    pub encoder: String,
    /// Seconds of video in each segment.
    ///
    /// **The latency dial.** A client plays two or three segments behind the newest one. So this
    /// sets the delay between pressing pause and the music stopping. One second is the shortest a
    /// playlist can state. An older television whose own player stops to buffer plays smoothly at
    /// two.
    pub segment_seconds: u32,
    /// How many segments the playlist names at once.
    ///
    /// **Twelve seconds of stream at the default segment length.** A client that falls behind still
    /// finds the segment it wants next, rather than one the muxer has deleted.
    pub playlist_size: u32,
    /// Samples a second the machine renders at.
    ///
    /// **Chosen rather than asked for.** A streaming run opens no device, so nothing else has an
    /// opinion about the rate.
    pub sample_rate: u32,
    /// Audio bits a second.
    pub audio_bitrate: usize,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate: 12_000_000,
            // The literal rather than `km_stream::encode::AUTO_ENCODER`, which lives behind the
            // `ffmpeg` feature: settings are read and written by a build with no video in it.
            encoder: "auto".to_owned(),
            segment_seconds: 1,
            playlist_size: 12,
            sample_rate: 48_000,
            audio_bitrate: 192_000,
        }
    }
}

/// Demo mode: what the machine plays when nobody is singing.
///
/// A commercial home unit does not sit silent under a television. Left alone this one used to, and
/// the effect was that a room full of people had no way to hear what the box holds without working
/// out how to drive it first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DemoSettings {
    /// Whether the machine starts in demo mode.
    ///
    /// **Off.** A machine that starts playing music by itself the first time it is switched on is
    /// startling, and this is a mode somebody chooses rather than one they should have to discover
    /// and turn off. `PUT /api/v1/demo` writes here when it is asked to persist; without that, a
    /// switch made over the API lasts for the run and no longer.
    pub enabled: bool,
    /// Seconds of silence before a demo song starts.
    ///
    /// **A minute.** Two was the first answer and it was measured against the wrong worry: a room
    /// that has genuinely stopped singing spends the second minute looking at an idle screen, which
    /// is the silence the feature exists to fill. A minute is still long enough to be somebody
    /// deciding what to sing next rather than the machine interrupting them, and it is short enough
    /// that a person who put the phone down finds out the box has a catalog.
    ///
    /// **Not the `0 == never` convention `idle_release_secs` uses**, and deliberately: `enabled` is
    /// already the off switch, so a second one here would be a way to configure a mode that does
    /// nothing. `0` means *as soon as the machine goes idle*, which is a thing somebody might
    /// actually want for a shop window.
    ///
    /// The delay is what separates a demo from a nuisance. It only ever elapses while nothing is
    /// playing and nothing is queued, and any deliberate act — queueing, skipping, stopping — starts
    /// it again, so it is the answer to "how long after the party stops does the machine take over".
    ///
    /// Capped at [`km_api::machine::MAX_DEMO_DELAY_SECS`] where a *route* sets it. The cap is not
    /// applied here: a settings file is the owner's own and says what it says, where a form is a
    /// number somebody typed.
    pub delay_secs: u32,
    /// Only offer songs rated at least this, out of ten. `null` for no filter.
    ///
    /// **Five.** An unattended machine playing its roughest file is a poor advert, and suitability
    /// is exactly the judgment this wants — it is what `km-pack` measured about the file rather
    /// than anything about a performance. When the filter matches nothing the pick falls back to the
    /// whole catalog, so a library of rough files still demonstrates itself rather than staying
    /// silent.
    ///
    /// Worth knowing before it surprises somebody: **a video or MP3+G song rates a flat 10**, by
    /// what it is rather than by measurement, so this floor *prefers* them. That is right for
    /// showing the machine off and is worth a thought on the appliance, where an unattended 1080p
    /// decode runs for as long as nobody comes home. Raise it past anything MIDI reaches to lean
    /// further that way, or drop it to take the mixture.
    pub min_suitability: Option<u8>,
}

impl Default for DemoSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            delay_secs: 60,
            min_suitability: Some(5),
        }
    }
}

impl DemoSettings {
    /// How long the machine waits before starting a demo song.
    ///
    /// Unlike [`AudioSettings::idle_release`] this is never `None`: zero is a real answer here and
    /// means "at once". See [`Self::delay_secs`].
    pub fn delay(&self) -> Duration {
        Duration::from_secs(u64::from(self.delay_secs))
    }
}

/// A remembered microphone channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicSettings {
    /// Stable id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Which hardware input, free-form.
    #[serde(default)]
    pub device_hint: Option<String>,
    /// Level, 1.0 being unity.
    #[serde(default = "unity")]
    pub gain: f32,
    /// Reverb amount.
    #[serde(default)]
    pub reverb: f32,
    /// Echo amount.
    #[serde(default)]
    pub echo: f32,
    /// Whether it is muted.
    #[serde(default)]
    pub muted: bool,
}

fn unity() -> f32 {
    1.0
}

impl From<&km_queue::MicChannel> for MicSettings {
    fn from(channel: &km_queue::MicChannel) -> Self {
        Self {
            id: channel.id.clone(),
            name: channel.name.clone(),
            device_hint: channel.device_hint.clone(),
            gain: channel.gain,
            reverb: channel.reverb,
            echo: channel.echo,
            muted: channel.muted,
        }
    }
}

impl From<&MicSettings> for km_queue::MicChannel {
    fn from(settings: &MicSettings) -> Self {
        let mut channel = Self {
            id: settings.id.clone(),
            name: settings.name.clone(),
            device_hint: settings.device_hint.clone(),
            gain: settings.gain,
            reverb: settings.reverb,
            echo: settings.echo,
            muted: settings.muted,
        };
        // A hand-edited settings file can hold anything; the registry's own clamp is the guard.
        channel.clamp();
        channel
    }
}

/// The debug single-file path, and the banks the SoundFont switcher offers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugSettings {
    /// Directories `POST /debug/play-file` may read from.
    ///
    /// Empty means the endpoint is refused outright. That is the deliberate default: the endpoint
    /// reads an arbitrary path off the machine's disk, and an owner who wants it has to say where.
    /// `--play` on the command line is not affected — somebody at the keyboard already has the disk.
    pub play_file_roots: Vec<PathBuf>,
    /// Whether debugging mode is on — the master switch for **everything in this struct**.
    ///
    /// **Three states, and [`Settings::debug_enabled`] is what reads them.** `Some` is an owner's
    /// answer and always wins. **Absent means nobody has said**, which follows the build: on while
    /// debugging, off in anything shipped. Read that accessor rather than this field.
    ///
    /// **Off, every other field here is ignored** — not just the two routes it mounts.
    /// `play_file_roots`, `packages`, `wallpapers`, `soundfonts` and `soundfont_slot` are the places
    /// `settings.json` is allowed to name an individual file, which makes them exactly the entries
    /// that point the machine at arbitrary paths. One switch governing all of them is easier to
    /// reason about than five that each mean something slightly different, and a populated section
    /// with the switch off is logged rather than silently dropped — see
    /// [`Settings::warn_about_ignored_debug_settings`].
    ///
    /// **It replaced `accept_uploads`, which was narrower and is gone.** That field existed only
    /// because there was no master switch: a machine on `0.0.0.0` with no password had to be stopped
    /// from taking bytes from a stranger, and the access list could not do it without breaking the
    /// curation tool at the same time. Every route that writes to disk is behind `/api/v1/admin/`
    /// now and a password always exists, so the narrow switch had nothing left to do.
    ///
    /// **`Option` rather than a `bool` with a clever default, and that is not cosmetic.**
    /// [`Settings::save`] serializes this whole struct, so a `bool` defaulting to
    /// `cfg!(debug_assertions)` would write `true` into the device's `settings.json` on its first
    /// debug run — and stay there when the same device is upgraded to a release build. `None`
    /// serializes as `null` and is still `None` on the way back in, so the build decides every time
    /// and nothing is baked in behind anybody's back.
    pub enabled: Option<bool>,
    /// Banks `Ctrl+2`…`Ctrl+9` switch between, in slot order.
    ///
    /// **Non-empty is what turns the switcher on**, keys and on-screen label together. Empty means
    /// the keys do nothing, nothing is drawn and no bank but the resolved one is ever opened — the
    /// same bargain [`crate::display::FrameMeter`] strikes, and for the same reason: a diagnostic
    /// is something you ask for by name.
    ///
    /// `Ctrl+1` is not in here. It is always the bank the machine would have resolved for itself,
    /// so there is a fixed reference to compare every entry against and one digit that cannot be
    /// misconfigured. Written by `--set-debug-soundfonts`, which opens every bank before it writes.
    pub soundfonts: Vec<DebugBank>,
    /// The switcher slot in force when this machine was last stopped.
    ///
    /// **An index into [`Self::soundfonts`]'s numbering, never a path** — which is what keeps this
    /// inside `Only `debug.` names a file` rather than pushing at it, and what makes a stale value
    /// harmless: a list that has been shortened simply no longer answers, and the machine comes up
    /// on its own bank.
    ///
    /// **`None` is slot 1**, the bank the machine resolves for itself, so a session that pressed
    /// `Ctrl+1` last leaves no entry here at all. That is why a machine that has never used the
    /// switcher writes nothing.
    ///
    /// **This is the half of the switcher that *is* remembered, and the other half still is not.**
    /// `audio.soundfont` is never written by a switch: clearing [`Self::soundfonts`] therefore
    /// returns the machine to its own bank, which is what made leaving the slots configured safe.
    /// What changes is that an evening of comparing banks now resumes where it stopped instead of
    /// starting again at slot 1 after every restart — see `Switching the bank while it plays` in
    /// docs/decisions/audio.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soundfont_slot: Option<u8>,
    /// Extra `.kmpkg` files to install at every pass, **on top of** the packages folders.
    ///
    /// Additive and never a replacement, exactly as [`Self::soundfonts`] is additive over the bank
    /// the machine resolves for itself: every [`Paths::packages_dirs`] folder is scanned regardless,
    /// and these go in beside what it finds — first, so an entry naming a file that is also in a
    /// folder is the one that installs.
    ///
    /// **Behind `debug.`, because naming a file is what `debug.` is for.**
    /// Each entry is a full path to one file and is replayed at every pass, so a file moved or
    /// tidied away is a fault reported at every pass until somebody clears it. That fragility is
    /// exactly why it lives here; [`Settings::package_dirs`] is the answer for anything that is not
    /// a one-off, because a folder is stable and a package taken out of one simply stops being
    /// installed.
    ///
    /// **Nothing here is the machine's to delete.** `DELETE /api/v1/packages/{id}` refuses for a
    /// package reached through this list, and no API route writes to it: `--set-debug-packages` and
    /// `--clear-debug-packages` are the only writers, exactly as [`Self::soundfonts`] has only
    /// `--set-debug-soundfonts` and `--clear-debug-soundfonts`. Only a person at a keyboard changes
    /// this section, and that is what makes it a safe escape hatch rather than a second catalog.
    ///
    /// A path already inside a scanned folder is not installed twice — the pass dedupes by package
    /// **id** — so such an entry is harmless and does nothing the scan would not have done.
    ///
    /// Omitted from a file that has none, unlike [`Self::soundfonts`]: a shipped machine's
    /// `settings.json` should not advertise an escape hatch it is not using.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PathBuf>,
    /// Extra images or `.zip` packs to show, **on top of** whichever wallpaper folder won.
    ///
    /// Additive and never a replacement, the same shape as [`Self::soundfonts`] and
    /// [`Self::packages`]: the three-candidate rule still picks one folder, and these go in beside
    /// what it holds. Either kind of file the folder itself takes — a `.zip` here is a folder of
    /// wallpapers exactly as it is in the folder.
    ///
    /// **Nothing here is the machine's to delete**, as everywhere else in this section. There is no
    /// route that removes a wallpaper at all today; this is written down before there is one,
    /// because the cheapest moment to state a constraint on a route is before it exists. Any such
    /// control must skip these and refuse rather than silently do nothing — an image that stays on
    /// screen after being removed reads as a broken button.
    ///
    /// Picked up at the next rescan rather than needing a restart, so editing this and pressing the
    /// wallpaper-advance key is enough to see it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wallpapers: Vec<PathBuf>,
}

/// One bank the switcher can reach, as `debug.soundfonts` records it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugBank {
    /// The `.sf2` file.
    pub path: PathBuf,
    /// What the on-screen label calls it.
    ///
    /// A name rather than the path because the label is read across a room, and because the file
    /// is often `gm.sf2` in a per-bank cache folder — the filename alone would say nothing.
    pub name: String,
    /// The level this bank was measured to want, if it was measured.
    ///
    /// Banks differ enough in level to clip, which is why `tools/setup/soundfont-banks.sh` records
    /// one per bank and why the override written by `--set-soundfont` carries it. An A/B in which
    /// one bank is simply louder answers a question nobody asked, so the switcher applies this on
    /// the way in. `None` leaves the level alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_volume: Option<f32>,
}

/// What this machine is called on the network, and what language it speaks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineSettings {
    /// The name a phone shows in a list.
    pub name: String,
    /// Stable instance id.
    ///
    /// Empty on a fresh install; [`Settings::load`] fills it in and saves, so it is generated once
    /// in the machine's life and a remote can recognize the machine it talked to yesterday.
    pub instance_id: String,
    /// What language the television speaks, as a BCP 47 tag — `en`, `pt-BR`.
    ///
    /// **`locale`, not `language`.** `language` is the language a *song* is sung in everywhere else
    /// in this product, and the two are independent: a machine set to `pt-BR` still has an English
    /// section in its song book. See `One spelling per concept, across every surface`.
    ///
    /// **One machine, one locale**, unlike a web page, which each viewer negotiates for themselves.
    /// The television is in a room, and the room has one language.
    ///
    /// Beside `name` because it is the same kind of fact: something the owner set about this
    /// machine rather than about a request. A tag this build does not have falls back to English
    /// rather than refusing to start — see [`MachineSettings::locale`].
    pub locale: String,
}

impl Default for MachineSettings {
    fn default() -> Self {
        Self {
            name: "KaraokeMachine".to_owned(),
            instance_id: String::new(),
            locale: km_locale::Locale::default().tag().to_owned(),
        }
    }
}

impl MachineSettings {
    /// The locale this machine speaks.
    ///
    /// **A hand-edited file cannot stop the machine starting.** `settings.json` is a file somebody
    /// opens, and a tag this build has no catalog for — a typo, or one written by a later version —
    /// falls back to English. The alternative is an appliance under a television that will not come
    /// up, with the reason in a log nobody is reading.
    #[must_use]
    pub fn locale(&self) -> km_locale::Locale {
        km_locale::Locale::best_match(&self.locale).unwrap_or_default()
    }
}

/// What language the machine speaks, read without loading everything else.
///
/// **For `--song-book`, which runs before [`Settings::load`] on purpose** — that path repairs the
/// settings and its caller saves them, so a book taken against a fresh `--data-dir` would leave an
/// install behind for a command that only wanted to print a list. Printing a book in the machine's
/// own language should not change that, so this reads the one field and writes nothing.
///
/// English for a file that is absent or unreadable, which is the same answer
/// [`MachineSettings::locale`] gives a tag it does not know: a book in the wrong language beats no
/// book and a stack trace.
#[must_use]
pub fn machine_locale(paths: &Paths) -> km_locale::Locale {
    peek_machine(paths).map_or_default(|machine| machine.locale())
}

/// What the machine is called, read without loading everything else.
///
/// [`machine_locale`]'s twin, for [`machine_locale`]'s reason and on the same one command: a book
/// says whose machine it is on every page, and a machine named `Living Room` should print that
/// whether the book came from the API or from `--song-book`.
///
/// `None` for a file that is absent or unreadable, which is the same fallback the locale takes and
/// means the same thing to the caller: print what the book said before there was a name in it.
#[must_use]
pub fn machine_name(paths: &Paths) -> Option<String> {
    peek_machine(paths).map(|machine| machine.name)
}

/// The `machine` section of `settings.json`, read and thrown away.
///
/// **Deliberately not [`Settings::load`]** — that path repairs the settings and its caller saves
/// them, so a book taken against a fresh `--data-dir` would leave an install behind for a command
/// that only wanted to print a list.
fn peek_machine(paths: &Paths) -> Option<MachineSettings> {
    std::fs::read_to_string(paths.settings_file())
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .map(|settings| settings.machine)
}

/// The settings shape this build reads and writes, stamped into every `settings.json` it saves.
///
/// **A change that needs an existing file repaired is the next number**, and one repair step keyed
/// on it; the counter never starts again. A file naming a lower number is set aside rather than read
/// — see [`Settings::load`] — by the rule in `A store opens at its current version or is refused`.
pub const CURRENT_SETTINGS_VERSION: u32 = 5;

/// The version a file with no `settings_version` is read as: the current one.
///
/// **An installer writes exactly such a file** — `display.fullscreen` and nothing else — and it is a
/// new file, not an old one. Reading it as current is what lets it take the defaults for everything
/// it does not say.
fn current_settings_version() -> u32 {
    CURRENT_SETTINGS_VERSION
}

/// Everything the machine remembers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which settings shape this file was written in. See [`CURRENT_SETTINGS_VERSION`].
    ///
    /// Every field is optional, so a file that names a field and not the rest is ordinary; what this
    /// number answers is whether a field that *is* named means what this build means by it.
    #[serde(default = "current_settings_version")]
    pub settings_version: u32,
    /// What this machine is called.
    pub machine: MachineSettings,
    /// The HTTP API.
    pub api: ApiSettings,
    /// Audio output.
    pub audio: AudioSettings,
    /// The window.
    pub display: DisplaySettings,
    /// The wallpaper cycle.
    pub wallpaper: WallpaperSettings,
    /// Playback defaults.
    pub playback: PlaybackSettings,
    /// What the machine plays when nobody is singing.
    pub demo: DemoSettings,
    /// What the stream looks like, for a run that draws for an encoder. See [`StreamSettings`].
    #[serde(default)]
    pub stream: StreamSettings,
    /// Where this run's log goes, for a machine nobody types at.
    #[serde(default)]
    pub logging: LoggingSettings,
    /// Remembered microphone channels.
    pub mics: Vec<MicSettings>,
    /// Extra folders to scan for packages, beside the one in the data directory.
    ///
    /// The answer to a library that lives somewhere else — a second drive or a share — without
    /// naming every file in it. Each is scanned exactly as the data directory's own `packages`
    /// folder is: `*.kmpkg` at the top level, at every pass, and what it holds is what is installed.
    ///
    /// **This is now the only way to say where packages are**, and it is folder-granular on
    /// purpose. The list of individual files it used to defer to is gone: every entry there was a
    /// standing obligation replayed at every start, so a file renamed or moved inside its own folder
    /// became a failure the owner had to go and clear — while a folder is stable, and a package
    /// taken out of one simply stops being installed. `debug.packages` is what remains for naming a
    /// single file, and it is behind `debug.` because that fragility is what it is for.
    ///
    /// Empty by default, and omitted from a file that has none, so nothing changes for a machine
    /// that never sets it.
    ///
    /// **Naming a folder here makes it the machine's to delete from**, which is the half of this
    /// setting that does not announce itself. [`crate::settings::Paths::is_mine_to_delete`] treats
    /// every scanned folder as the machine's own territory, so uninstalling a package takes its file
    /// out of one of these exactly as it would out of the machine's own folder. The asymmetry with
    /// `debug.packages` — which names a *file* and is refused — is deliberate: a folder handed to
    /// the machine is a folder given to it, and a file the owner keeps somewhere of their own is
    /// not. `--show-paths` says so beside each one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_dirs: Vec<PathBuf>,
    /// The block of a thousand each package's songs are dialled in, by package **id**.
    ///
    /// `{"brasil-vol2": 3}` makes that package's song 500 into 3500, which is how two packages that
    /// both number a song 500 are made to fit — see the `A song number is a bank and a slot`
    /// decision.
    ///
    /// **Here rather than only in the catalog, and that is load-bearing**: the bank has to be
    /// known *before* the install, because the install writes it into every song's number. It is in
    /// the catalog too, and [`crate::machine::Machine::ensure_bank`] reads that as its second
    /// source — a settings file that was lost or rescued from a `settings.json.bad` would otherwise
    /// let an already-installed package be moved to a different bank under a live queue.
    ///
    /// **Keyed by id and not by path**, because a package that is moved or renamed is the same
    /// package and must keep its bank; the id is in its manifest and does not move.
    ///
    /// **Nothing releases a reservation except an explicit uninstall.** A package that merely
    /// stopped being found keeps its bank, which is what lets it come back with the same song
    /// numbers rather than whatever is free next — see [`km_catalog::Library::retain_packages`].
    /// One leak is accepted and named rather than fixed: a `.kmpkg` that will not open reserves a
    /// bank before the failing install, and if the owner deletes the file instead of mending it
    /// there is no id left to uninstall. Pruning reservations the scan did not see would break the
    /// fix-it-next-week case this field exists to protect.
    #[serde(default)]
    pub package_banks: BTreeMap<String, u16>,
    /// The debug path.
    pub debug: DebugSettings,
}

impl Settings {
    /// What the settings file says about logging, without writing one or reporting on one.
    ///
    /// **A second, narrower read rather than a call to [`Self::load`], and both halves of that are
    /// load-bearing.** The subscriber has to exist before anything can be said, so this runs before
    /// the settings are loaded — and `load` *writes*, so asking it where the log goes would create a
    /// `settings.json` for a machine that has never run, which is exactly what the ordering in
    /// `cli::main` was arranged to avoid.
    ///
    /// **Silent about a file it cannot read**, because there is nowhere to say it yet and `load` is
    /// a moment away from saying it properly — with the rename to `settings.json.bad` that goes with
    /// it. A machine whose settings will not parse starts with no log file rather than no log file
    /// *and* a duplicate complaint.
    pub fn peek_logging(paths: &Paths) -> LoggingSettings {
        km_logsettings::peek(paths.settings_file())
    }

    /// Loads settings, writing defaults if there is no file yet.
    ///
    /// Returns the settings and whether the file was written. A parse failure is **not** fatal: the
    /// broken file is kept as `settings.json.bad` and defaults are used, because losing a
    /// configuration is bad and refusing to start is worse.
    ///
    /// **A file naming an older [`CURRENT_SETTINGS_VERSION`] takes the same path**, with both numbers
    /// in the log. Nothing in this build reads its shape, and a field read as meaning what it means
    /// now, in a file written when it meant something else, is a machine configured by accident.
    pub fn load(paths: &Paths) -> (Self, bool) {
        let file = paths.settings_file();
        let mut settings = match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(settings) if settings.settings_version < CURRENT_SETTINGS_VERSION => {
                    let refused = file.with_extension("json.bad");
                    tracing::error!(
                        found = settings.settings_version,
                        reads = CURRENT_SETTINGS_VERSION,
                        kept = %refused.display(),
                        "settings.json is in an older settings version than this build reads; \
                         starting from defaults"
                    );
                    let _ = std::fs::rename(&file, &refused);
                    Self::default()
                }
                Ok(settings) => settings,
                Err(error) => {
                    let broken = file.with_extension("json.bad");
                    tracing::error!(
                        %error,
                        kept = %broken.display(),
                        "settings.json could not be read; starting from defaults"
                    );
                    let _ = std::fs::rename(&file, &broken);
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(file = %file.display(), "no settings yet; writing defaults");
                Self::default()
            }
            Err(error) => {
                tracing::error!(%error, "settings.json could not be opened; using defaults");
                Self::default()
            }
        };

        // Repairs on the way in, so the rest of the program can assume all of them.
        let mut changed = false;
        if settings.machine.instance_id.is_empty() {
            settings.machine.instance_id = km_api::discover::new_instance_id();
            changed = true;
        }
        changed |= settings.ensure_password();
        settings.warn_about_ignored_debug_settings();
        settings.warn_about_unread_settings();

        let written = if changed || !file.exists() {
            match settings.save(paths) {
                Ok(()) => true,
                Err(error) => {
                    // Not fatal. The machine runs from what is in memory; it just will not remember
                    // the instance id next time, which costs a remote its recognition and nothing
                    // else.
                    tracing::error!(%error, "could not write settings");
                    false
                }
            }
        } else {
            false
        };
        (settings, written)
    }

    /// Gives a machine with no password a generated one, and reports whether it did.
    ///
    /// **This is what makes "a machine always has a password" true**, and everything about the
    /// permission system rests on it: every route under `/api/v1/admin/` demands a token, a token
    /// can only come from a password, so a machine without one is a machine whose owner is locked
    /// out of it. The old design answered that by making those routes public, which is the failure
    /// this replaced.
    ///
    /// The PIN is kept in plain text beside the hash, because the machine has to draw it on its own
    /// screen and a hash cannot be drawn. It is cleared the moment the owner sets a password of
    /// their own — [`ApiSettings::admin_factory_pin`] being `Some` *is* what "still on the factory
    /// password" means.
    ///
    /// A machine whose hash was hand-edited out of the file gets a fresh PIN rather than an error:
    /// the alternative is a box under a television that will not start, and the PIN is on the screen
    /// where its owner can read it.
    pub fn ensure_password(&mut self) -> bool {
        if self.api.admin_password_hash.is_some() {
            return false;
        }
        let pin = km_api::auth::generate_factory_pin();
        match km_api::AdminAuth::hash_password(&pin) {
            Ok(hash) => {
                self.api.admin_password_hash = Some(hash);
                self.api.admin_factory_pin = Some(pin);
                tracing::info!(
                    "this machine had no admin password, so it generated one; it is on the screen \
                     and in settings.json until you change it"
                );
                true
            }
            Err(error) => {
                // Nothing sensible to fall back to: a machine that invented a password it could not
                // hash would be one nobody can log into, and a hardcoded one would be worse.
                tracing::error!(%error, "could not hash a generated admin password");
                false
            }
        }
    }

    /// Writes settings out, stamped [`CURRENT_SETTINGS_VERSION`].
    ///
    /// Temp file then rename, so an interrupted write cannot leave a half-parsed settings file —
    /// which would be read as corrupt on the next start and moved aside.
    ///
    /// **Stamped here, whatever the value in memory says**, because this build only ever writes its
    /// own shape. `Default` is derived and starts the number at 0, so a file written straight from
    /// it would otherwise name a version below the current one and be set aside on the next start.
    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        paths.create()?;
        let target = paths.settings_file();
        let temp = target.with_extension("json.tmp");
        let stamped = Self {
            settings_version: CURRENT_SETTINGS_VERSION,
            ..self.clone()
        };
        let text = serde_json::to_string_pretty(&stamped)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(text.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, &target)
    }

    /// Whether to serve the development remote at `/dev/`.
    ///
    /// **Off unless somebody asks for it**, by `--dev-remote` for one run or
    /// `api.serve_dev_remote: true` for good.
    ///
    /// This has been all three answers, and the middle one is why the current one is not a
    /// reversion. It first followed the build — on in debug, off in release — on the grounds that a
    /// page driving `debug/play-file` and the ACL is a development tool rather than a product
    /// surface. That did not survive contact with the machine: with no end-user remote yet, `/dev/`
    /// was the *only* way to search, queue and control the thing from a phone, and a release build
    /// that hid it left an owner with a keypad. So it went on by default, and the argument was that
    /// it withheld a page rather than a permission.
    ///
    /// **Both halves of that have since stopped being true.** `/` serves the singer's remote and
    /// `/admin/` serves the owner's, so nothing an owner needs is behind this page any more; and a
    /// developer's console reachable by default on every machine on the LAN is a page nobody
    /// operating a karaoke machine ever means to publish. It still reaches no route the ACL does not
    /// already govern — that was never the point. Not shipping a console is.
    ///
    /// **What is left behind it is real and is the cost accepted**: the ACL editor and the
    /// `debug.accept_uploads` switch have no other screen, so a curator whose package builder is
    /// refused an upload now has a flag to type. `km-package-builder` says so in its refusal.
    pub fn serve_dev_remote(&self) -> bool {
        self.api.serve_dev_remote.unwrap_or(false)
    }

    /// Whether the machine will take a song sent to `POST /debug/play-upload`.
    ///
    /// An owner's answer wins. **Absent follows the build**: on in a debug one, off in anything
    /// shipped. The route writes to the disk and then decodes what it wrote, so "off unless told"
    /// is right for a machine sitting in somebody's front room on `0.0.0.0` with no password — and
    /// wrong for the two cases where it is off *because nobody could reach the setting*. Those are
    /// a checkout, where the answer is `cargo run` and the person running it is the owner; and an
    /// Android build, which has no command line and keeps its `settings.json` in app-private
    /// storage reachable only through `adb`. Both are `cfg!(debug_assertions)`, which is true for
    /// `cargo run` and for the default `task build:android`, and false for `--release` and for
    /// `RELEASE=1`. So a shipped machine is exactly as closed as it was.
    ///
    /// Nothing is written by reading this — see [`DebugSettings::enabled`] for why the field
    /// is an `Option` and not a `bool` with this expression as its default.
    pub fn debug_enabled(&self) -> bool {
        self.debug.enabled.unwrap_or(cfg!(debug_assertions))
    }

    /// Whether the `debug.` section holds anything an owner would notice being ignored.
    ///
    /// Deliberately does **not** count [`DebugSettings::enabled`] itself: the question is whether
    /// turning the switch on would change what this machine does.
    pub fn debug_section_is_populated(&self) -> bool {
        !self.debug.play_file_roots.is_empty()
            || !self.debug.packages.is_empty()
            || !self.debug.wallpapers.is_empty()
            || !self.debug.soundfonts.is_empty()
            || self.debug.soundfont_slot.is_some()
    }

    /// Says out loud what the settings file asked for and this machine could not read.
    ///
    /// Here rather than in [`Self::peek_logging`] because that runs before there is a subscriber to
    /// say it to. **A value nobody can read is worse than a wrong one**: the machine goes on doing
    /// what it did and the person who set it goes on believing otherwise, which they find out on
    /// the evening they go looking for the run that broke.
    pub fn warn_about_unread_settings(&self) {
        self.logging.warn_about_unread();
    }

    /// Says once, loudly, what is being ignored because debugging is off.
    ///
    /// **Silence here is the trap**, and it is the one the settings rules in
    /// `docs/architecture/persistence.md` warn about for dropped values generally: an owner whose
    /// machine quietly stopped honouring `debug.soundfonts` concludes the soundfont switcher broke,
    /// not that a switch they have never heard of is off.
    pub fn warn_about_ignored_debug_settings(&self) {
        if self.debug_enabled() || !self.debug_section_is_populated() {
            return;
        }
        let mut ignored: Vec<&str> = Vec::new();
        if !self.debug.play_file_roots.is_empty() {
            ignored.push("play_file_roots");
        }
        if !self.debug.packages.is_empty() {
            ignored.push("packages");
        }
        if !self.debug.wallpapers.is_empty() {
            ignored.push("wallpapers");
        }
        if !self.debug.soundfonts.is_empty() {
            ignored.push("soundfonts");
        }
        if self.debug.soundfont_slot.is_some() {
            ignored.push("soundfont_slot");
        }
        tracing::warn!(
            ignored = ignored.join(", "),
            "debugging is off, so these debug settings do nothing; set debug.enabled to true, \
             or turn it on from the owner's page, to use them"
        );
    }

    /// Whether the singer-facing remote is served at `/`.
    ///
    /// On by default. It is the surface every phone in the room reaches, and the QR code on the
    /// television points at it — a machine serving nothing there would be a machine whose own screen
    /// tells people to go somewhere with nothing on it.
    pub fn serve_remote(&self) -> bool {
        self.api.serve_remote.unwrap_or(true)
    }

    /// The mic registry to start from — the remembered channels, or two mics on a fresh install.
    pub fn mic_registry(&self) -> km_queue::MicRegistry {
        if self.mics.is_empty() {
            return km_queue::MicRegistry::with_two_mics();
        }
        let mut registry = km_queue::MicRegistry::new();
        for mic in &self.mics {
            if let Err(error) = registry.register(km_queue::MicChannel::from(mic)) {
                tracing::warn!(id = %mic.id, %error, "ignoring a microphone from settings");
            }
        }
        registry
    }

    /// The display crate's wallpaper configuration, extras and all.
    ///
    /// [`WallpaperSettings::to_config`] cannot reach `debug.wallpapers` — it is a different section
    /// of the file — so this is the one that produces a complete answer, and the loop reads it.
    pub fn wallpaper_config(&self, paths: &Paths) -> km_display::WallpaperConfig {
        km_display::WallpaperConfig {
            extra: if self.debug_enabled() {
                self.debug.wallpapers.clone()
            } else {
                Vec::new()
            },
            ..self.wallpaper.to_config(paths)
        }
    }

    /// Builds the API's configuration from these settings.
    pub fn api_config(&self, dev_remote_dir: Option<PathBuf>) -> km_api::ApiConfig {
        km_api::ApiConfig {
            bind: self.api.socket_addr(),
            admin_password_hash: self.api.admin_password_hash.clone(),
            factory_password: self.api.admin_factory_pin.is_some(),
            session_epoch: self.api.session_epoch,
            token_ttl: Duration::from_secs(self.api.token_ttl_secs.max(60)),
            debug_enabled: self.debug_enabled(),
            machine_name: self.machine.name.clone(),
            locale: self.machine.locale(),
            instance_id: self.machine.instance_id.clone(),
            advertise_mdns: self.api.advertise_mdns,
            serve_dev_remote: self.serve_dev_remote(),
            dev_remote_dir,
            cors_origins: self.api.cors_origins.clone(),
        }
    }

    /// Whether a path is inside one of the allowed debug roots.
    ///
    /// Both sides are canonicalised first, so `..` cannot walk out of a root and a symlink cannot
    /// point past it. An empty root list refuses everything.
    pub fn debug_path_allowed(&self, path: &Path) -> bool {
        if !self.debug_enabled() || self.debug.play_file_roots.is_empty() {
            return false;
        }
        let Ok(target) = path.canonicalize() else {
            return false;
        };
        self.debug
            .play_file_roots
            .iter()
            .filter_map(|root| root.canonicalize().ok())
            .any(|root| target.starts_with(&root))
    }

    /// Whether the SoundFont switcher is on at all.
    ///
    /// One question asked in three places — the keys, the label and the switch itself — so that
    /// "configured" cannot come to mean two different things between them.
    pub fn soundfont_switcher_on(&self) -> bool {
        self.debug_enabled() && !self.debug.soundfonts.is_empty()
    }

    /// The bank in a switcher slot, or `None` if that slot holds nothing.
    ///
    /// Slot 1 is deliberately absent here rather than absent by accident: it is the bank the
    /// machine resolves for itself, which is not a settings entry and cannot be one. Callers ask
    /// [`crate::engine::resolve_soundfont`] for it instead.
    pub fn soundfont_slot(&self, slot: u8) -> Option<&DebugBank> {
        if !self.soundfont_switcher_on() || slot < 2 {
            return None;
        }
        self.debug.soundfonts.get(usize::from(slot) - 2)
    }

    /// The slot this machine was left on, and the bank it names — if that slot still answers.
    ///
    /// **Every way of it not answering falls back to slot 1 silently**, and there are three: the
    /// switcher was turned off by emptying `debug.soundfonts`, the list was shortened past the
    /// remembered index, or the value is 1 or 0. None of them is a fault worth a message — the
    /// machine comes up on its own bank, which is where it would have come up before this was
    /// remembered at all.
    pub fn restored_soundfont_slot(&self) -> Option<(u8, &DebugBank)> {
        let slot = self.debug.soundfont_slot?;
        Some((slot, self.soundfont_slot(slot)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loading a settings file and saving it back changes nothing.
    ///
    /// **The guard on every field this process does not itself set.** A run that resolves something
    /// it could not find — an audio output on a machine that has no devices, which is what a
    /// streaming run is — and then writes the fallback down has taken away a choice the owner made,
    /// with nothing reporting it. `run` binds its settings immutably so that cannot happen through
    /// the engine, and this is the same promise about the file as a whole: whatever came in goes
    /// back out.
    ///
    /// It is written against the *serialized* bytes rather than against a `PartialEq`, because that
    /// is what a later machine reads: a field that round-trips through the struct but serializes
    /// differently is still a changed file.
    #[test]
    fn loading_and_saving_changes_nothing() {
        let scratch = Scratch::new("round-trip");
        let paths = scratch.paths();

        // The first load writes a complete file: defaults, a fresh instance id and the PIN this
        // machine gives itself. That is the file a second run has to leave alone.
        let (first, _) = Settings::load(&paths);
        let written = std::fs::read_to_string(paths.settings_file()).expect("the settings file");

        // A second load must find nothing to change — no migration, no password to mint — and
        // saving what it read must produce the same bytes.
        let (second, _) = Settings::load(&paths);
        let after = std::fs::read_to_string(paths.settings_file()).expect("the settings file");
        assert_eq!(
            written, after,
            "a second load rewrote the file it had just read"
        );

        second.save(&paths).expect("saving what was loaded");
        let saved = std::fs::read_to_string(paths.settings_file()).expect("the settings file");
        assert_eq!(written, saved, "saving a loaded file changed it");

        // ...and the thing a streaming run most easily loses: a device it cannot see.
        assert_eq!(
            first.api.admin_password_hash, second.api.admin_password_hash,
            "the password a machine gave itself was minted twice"
        );
    }

    /// A device somebody chose survives a run that has no devices to choose from.
    ///
    /// This is the streaming case written down: the name in the file is one this machine cannot
    /// resolve, and the rule is that an unresolvable name is still the owner's answer.
    #[test]
    fn a_named_audio_device_survives_a_machine_that_cannot_see_it() {
        let scratch = Scratch::new("device");
        let paths = scratch.paths();
        let (mut settings, _) = Settings::load(&paths);
        settings.audio.output_device = Some("a device this machine has never had".to_owned());
        settings.save(&paths).expect("writing the chosen device");

        let (reloaded, _) = Settings::load(&paths);
        assert_eq!(
            reloaded.audio.output_device.as_deref(),
            Some("a device this machine has never had"),
            "a device that cannot be resolved is still the one that was chosen"
        );
    }

    /// The default encoder name is spelled here and understood there, so the two must agree.
    ///
    /// Settings compile without video, so this file carries the literal rather than the constant.
    /// A build that has both is where the two can be put beside each other.
    #[cfg(feature = "video")]
    #[test]
    fn the_default_encoder_is_the_name_the_stream_searches_on() {
        assert_eq!(
            StreamSettings::default().encoder,
            km_stream::encode::AUTO_ENCODER,
            "a settings file's default encoder must be the one km-stream treats as a search"
        );
    }

    /// A scratch directory that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-settings-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).expect("make the scratch directory");
            Self(dir)
        }

        fn paths(&self) -> Paths {
            Paths::rooted_at(&self.0)
        }

        /// The scratch root itself, for a test that needs a second folder beside the data one.
        fn root(&self) -> &Path {
            &self.0
        }

        /// A checkout-shaped tree: `assets/` and `local/assets/`, both real directories.
        fn checkout(&self) -> &Path {
            std::fs::create_dir_all(self.0.join("assets")).expect("the bundled tree");
            std::fs::create_dir_all(self.0.join("local").join("assets")).expect("the overlay");
            &self.0
        }

        /// Writes a file below the scratch root, making its parents.
        fn write(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("make the parents");
            std::fs::write(&path, b"not a real asset, and not parsed here").expect("write");
            path
        }

        /// A real zip holding one picture, which is what a wallpaper folder has to have in it.
        ///
        /// **A folder of pictures is not one**, so a test about which wallpaper folder wins cannot
        /// write a `.png` and be done: `holds_wallpapers` asks `Playlist::scan`, and that reads a
        /// folder's archives. The entry's *bytes* are still junk — the scan reads an archive's
        /// directory and never decodes — so only the zip itself has to be real.
        fn write_pack(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("make the parents");
            let file = std::fs::File::create(&path).expect("create the pack");
            let mut zip = zip::ZipWriter::new(file);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            zip.start_file("01.png", options).expect("start an entry");
            std::io::Write::write_all(&mut zip, b"not decoded here").expect("write the entry");
            zip.finish().expect("finish the pack");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The number pad follows the platform, and the master switch does not follow it.
    ///
    /// Worth a test despite being one line of `Default`, because the failure it guards is somebody
    /// "tidying" the `cfg!` into a literal `true` — which is correct on the platform they are
    /// standing on and wrong on the other four, and which no other test would notice.
    ///
    /// **It is every Android, televisions included, and an `&& !is_television()` here would be
    /// wrong.** A television has no keyboard and no touchscreen, so it is the platform that needs
    /// the pad *most*, and a physical Google TV remote's D-pad is what presses one. See
    /// `DisplaySettings::number_pad`.
    /// A container's packages folder is `Documents/packages`, and the word appears once.
    ///
    /// **This is a regression test for a fault only a device showed.** `Paths::in_container` took
    /// the `Documents` directory and `packages_dirs` appends `packages` to it, exactly as it does
    /// to Android's public directory — so a constructor that helpfully appended the subdirectory
    /// itself produced `Documents/packages/packages`. Nothing failed: the machine scanned a folder
    /// that will always be empty, beside the one the Files app shows somebody dropping a `.kmpkg`
    /// into, and said so in a log line nobody on a phone reads.
    #[test]
    fn a_containers_packages_folder_is_named_once() {
        let paths = Paths::in_container(Path::new("/c/Support"), Path::new("/c/Documents"));
        let dirs = paths.packages_dirs();
        assert!(
            dirs.contains(&PathBuf::from("/c/Documents/packages")),
            "the Files-app folder is not scanned; got {dirs:?}",
        );
        assert!(
            !dirs.iter().any(|d| d.ends_with("packages/packages")),
            "the subdirectory was appended twice; got {dirs:?}",
        );
    }

    /// The machine announces itself everywhere except where the packets would be dropped in
    /// silence.
    ///
    /// Asserted against the same `cfg!` for the reason the pad's tests are: a reader tidying this
    /// into a literal is right on the platform in front of them and wrong on the other one, and the
    /// failure it would hide is a machine that says it is advertising and is not.
    #[test]
    fn a_machine_advertises_itself_unless_the_platform_would_drop_the_packets() {
        let api = ApiSettings::default();
        assert_eq!(api.advertise_mdns, !cfg!(target_os = "ios"));
    }

    #[test]
    fn the_number_pad_defaults_to_on_wherever_there_is_no_keyboard() {
        let display = DisplaySettings::default();
        assert_eq!(
            display.number_pad,
            cfg!(any(target_os = "android", target_os = "ios"))
        );
        assert!(
            display.keypad,
            "the master switch is on everywhere; only the pad is conditional"
        );
    }

    /// A settings file written before this field existed still loads, and takes the platform's
    /// answer rather than `false`.
    ///
    /// `#[serde(default)]` on the struct is what provides this, and it is the sort of attribute that
    /// gets dropped in a refactor. Every existing installation's `settings.json` is such a file.
    #[test]
    fn a_settings_file_from_before_the_number_pad_takes_the_platform_default() {
        let display: DisplaySettings =
            serde_json::from_str(r#"{"fullscreen": false, "keypad": true}"#)
                .expect("an older display block must still load");
        assert_eq!(
            display.number_pad,
            cfg!(any(target_os = "android", target_os = "ios"))
        );
        assert!(
            !display.fullscreen,
            "the fields that were there still apply"
        );
    }

    #[test]
    fn a_fresh_install_hands_the_sound_card_back_when_it_is_idle() {
        let audio = AudioSettings::default();
        assert_eq!(audio.idle_release_secs, 5);
        assert_eq!(audio.idle_release(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn zero_seconds_means_hold_the_device_for_the_whole_session() {
        let audio = AudioSettings {
            idle_release_secs: 0,
            ..AudioSettings::default()
        };
        // Not "release immediately". This is the escape hatch for a box whose speakers nothing else
        // wants, and reading it the other way would release the device between songs and every time
        // somebody paused.
        assert_eq!(audio.idle_release(), None);
    }

    #[test]
    fn a_settings_file_written_before_the_key_existed_still_gets_the_default() {
        // No migration exists for this key and none should: `serde(default)` fills it on every file
        // ever written, which is the difference between a *new key* and a *changed default*.
        let settings: Settings =
            serde_json::from_str(r#"{"audio":{"music_volume":0.5}}"#).expect("parse");
        assert_eq!(settings.audio.music_volume, 0.5);
        assert_eq!(settings.audio.idle_release_secs, 5);
        assert_eq!(
            serde_json::from_str::<Settings>("{}")
                .expect("parse")
                .audio
                .idle_release_secs,
            5
        );
    }

    /// A fresh install does not start playing music by itself.
    #[test]
    fn demo_mode_is_off_out_of_the_box() {
        let demo = DemoSettings::default();
        assert!(!demo.enabled, "a machine must not perform uninvited");
        assert_eq!(demo.delay_secs, 60);
        assert_eq!(demo.delay(), Duration::from_secs(60));
        assert_eq!(demo.min_suitability, Some(5));
        // And an old settings file gets all three, since this is a new key rather than a changed
        // default -- the same reasoning as `idle_release_secs` above.
        let settings: Settings = serde_json::from_str("{}").expect("parse");
        assert!(!settings.demo.enabled);
        assert_eq!(settings.demo.delay_secs, 60);
    }

    /// A file that already says `120` keeps it, because a written value is an answer.
    ///
    /// **A changed default is not a new settings version**, which is the distinction
    /// `settings_version` exists to make: a new version is for a setting whose *meaning* changed
    /// under it, and a preference somebody may simply hold is not that. A two-minute delay in a file
    /// is a machine doing exactly what its file says.
    #[test]
    fn a_written_delay_survives_the_default_moving_under_it() {
        let settings: Settings =
            serde_json::from_str(r#"{"demo":{"delay_secs":120}}"#).expect("parse");
        assert_eq!(settings.demo.delay_secs, 120);
    }

    /// Zero here means "at once", where `idle_release_secs` reads zero as "never".
    ///
    /// The two conventions differ on purpose and the reason is in [`DemoSettings::delay_secs`]:
    /// `enabled` is already this feature's off switch, so a second one would only ever configure a
    /// mode that does nothing.
    #[test]
    fn a_zero_demo_delay_starts_as_soon_as_the_machine_is_idle() {
        let demo = DemoSettings {
            delay_secs: 0,
            ..DemoSettings::default()
        };
        assert_eq!(demo.delay(), Duration::ZERO);
    }

    /// A fresh install is reachable from a phone, which is the whole point of the remote.
    ///
    /// The machine used to bind loopback, so out of the box nobody's phone could reach it and the
    /// connect panel said `LoopbackOnly` — the broken case dressed up as the safe one.
    #[test]
    fn a_fresh_install_is_reachable_from_the_network() {
        let settings = Settings::default();
        let address = settings.api.socket_addr();
        assert!(
            address.ip().is_unspecified(),
            "a fresh install must listen on every interface, not {address}"
        );
        assert_eq!(address.port(), km_api::DEFAULT_PORT);
    }

    /// A fresh install gives itself a password rather than shipping without one.
    ///
    /// **The shipped state is closed.** `load` generates a six-digit PIN, hashes it, and puts the
    /// plain text on the machine's own screen, so a machine out of the box is not an
    /// unauthenticated one reachable from the whole house.
    ///
    /// It does listen on every interface, and that half is right: a karaoke machine whose remote
    /// nobody can connect to is the broken case, not the safe one.
    #[test]
    fn a_fresh_install_gives_itself_a_password() {
        let mut settings = Settings::default();
        assert!(
            settings.api.admin_password_hash.is_none(),
            "the struct default holds none; `load` is what fills it in"
        );

        assert!(
            settings.ensure_password(),
            "a password should have been made"
        );
        let hash = settings
            .api
            .admin_password_hash
            .as_deref()
            .expect("a hash was stored");
        assert!(hash.starts_with("$argon2"), "{hash}");

        let pin = settings
            .api
            .admin_factory_pin
            .as_deref()
            .expect("the PIN is kept so the screen can show it");
        assert_eq!(pin.len(), 6, "{pin}");
        assert!(pin.chars().all(|c| c.is_ascii_digit()), "{pin}");
        assert!(!pin.starts_with('0'), "{pin} would be mangled as a number");
        assert!(
            !hash.contains(pin),
            "the hash must not carry the PIN: {hash}"
        );

        // Idempotent: a machine that already has one keeps it.
        let before = settings.api.admin_password_hash.clone();
        assert!(!settings.ensure_password());
        assert_eq!(settings.api.admin_password_hash, before);
    }

    /// A machine whose owner has set their own password is not on a factory one.
    #[test]
    fn an_owners_password_clears_the_factory_pin() {
        let mut settings = Settings::default();
        settings.ensure_password();
        assert!(settings.api.admin_factory_pin.is_some());

        // What the controller does when an owner supplies one.
        settings.api.admin_password_hash =
            Some(km_api::AdminAuth::hash_password("carols1975").expect("hash"));
        settings.api.admin_factory_pin = None;
        assert!(!settings.api_config(None).factory_password);
    }

    /// Everything in the `debug.` section is inert while the switch is off.
    ///
    /// One switch over five fields, rather than five that each mean something slightly different —
    /// and these are exactly the entries that point the machine at arbitrary paths.
    #[test]
    fn the_debug_section_does_nothing_while_debugging_is_off() {
        let scratch = Scratch::new("debug-off");
        let paths = scratch.paths();
        let mut settings = Settings::default();
        settings.debug.soundfonts = vec![DebugBank {
            path: PathBuf::from("D:/banks/sc55.sf2"),
            name: "SC-55".to_owned(),
            music_volume: None,
        }];
        settings.debug.wallpapers = vec![PathBuf::from("D:/pictures/beach.jpg")];
        settings.debug.play_file_roots = vec![PathBuf::from("D:/tunes")];

        settings.debug.enabled = Some(false);
        assert!(!settings.soundfont_switcher_on(), "the switcher stays off");
        assert!(
            settings.wallpaper_config(&paths).extra.is_empty(),
            "the extra pictures stay out of the rotation"
        );
        assert!(
            !settings.debug_path_allowed(Path::new("D:/tunes/song.kar")),
            "and no path is inside an allowed root"
        );
        assert!(
            settings.debug_section_is_populated(),
            "...but the section is not empty, which is what the warning is for"
        );

        settings.debug.enabled = Some(true);
        assert!(settings.soundfont_switcher_on());
        assert_eq!(settings.wallpaper_config(&paths).extra.len(), 1);
    }

    /// A file with no `settings_version` is current, because an installer writes exactly such a file.
    #[test]
    fn a_settings_file_with_no_version_reads_as_current() {
        let settings: Settings =
            serde_json::from_str(r#"{"display":{"fullscreen":true}}"#).expect("parse");
        assert_eq!(settings.settings_version, CURRENT_SETTINGS_VERSION);
    }

    /// A file naming an older settings version is set aside rather than read, and the machine starts.
    ///
    /// The same path an unparseable file takes: a box under a television still comes up, and the
    /// file is kept beside it as `settings.json.bad` for whoever wants what was in it.
    #[test]
    fn a_settings_file_naming_an_older_version_is_set_aside() {
        let scratch = Scratch::new("older-settings");
        let paths = scratch.paths();
        paths.create().expect("create");
        std::fs::write(
            paths.settings_file(),
            format!(
                r#"{{"settings_version":{},"machine":{{"name":"Sala"}}}}"#,
                CURRENT_SETTINGS_VERSION - 1
            ),
        )
        .expect("write");

        let (loaded, written) = Settings::load(&paths);
        assert_ne!(
            loaded.machine.name, "Sala",
            "an older file must not be read"
        );
        assert!(written, "the defaults are written in its place");
        assert!(
            paths.settings_file().with_extension("json.bad").exists(),
            "and the older file is kept beside them"
        );
        let (again, _) = Settings::load(&paths);
        assert_eq!(again.settings_version, CURRENT_SETTINGS_VERSION);
    }

    /// A developer's console is not something a karaoke machine publishes on the LAN by default.
    /// It was on once, when it was the only way to drive the machine from a phone; `/` and
    /// `/admin/` are what answer that now.
    #[test]
    fn the_dev_remote_is_withheld_unless_somebody_asks_for_it() {
        assert!(!Settings::default().serve_dev_remote());

        let mut settings = Settings::default();
        settings.api.serve_dev_remote = Some(true);
        assert!(settings.serve_dev_remote());

        settings.api.serve_dev_remote = Some(false);
        assert!(!settings.serve_dev_remote());
    }

    #[test]
    fn the_debug_endpoint_is_refused_until_an_owner_names_a_folder() {
        let settings = Settings::default();
        assert!(settings.debug.play_file_roots.is_empty());
        assert!(!settings.debug_path_allowed(Path::new(".")));
    }

    /// The mirror of the test above, for the other endpoint that reaches the disk.
    ///
    /// **This cannot assert that the default is closed, and the reason matters before somebody
    /// "fixes" it.** `cargo test` builds with `debug_assertions` on, so `accept_uploads()` is
    /// `true` here by design — a checkout *is* the debugging case. What can be pinned, and what
    /// actually carries the guarantee, is that nothing has been *written*: a shipped build reads
    /// the same absent field and answers `false`, because the only input to that answer is the
    /// build. Whether the release side is closed is settled by the compiler, not by a test that
    /// cannot run in a release profile.
    #[test]
    fn debugging_is_nobodys_answer_until_somebody_gives_one() {
        assert!(
            Settings::default().debug.enabled.is_none(),
            "the shipped default is 'nobody has said', which is what lets the build decide"
        );
        assert_eq!(
            Settings::default().debug_enabled(),
            cfg!(debug_assertions),
            "and with nobody having said, the build is the whole of the answer"
        );
    }

    /// An owner's answer wins over the build, in both directions.
    ///
    /// The `false` half is the one with a bug behind it: turning uploads off must record `Some(false)`
    /// rather than clearing the field, or a debug machine would go on taking them and the switch
    /// would look broken. See `Machine::set_accept_uploads`.
    #[test]
    fn an_owner_who_has_said_is_obeyed_whichever_way_they_said_it() {
        let mut settings = Settings::default();

        settings.debug.enabled = Some(true);
        assert!(settings.debug_enabled());

        settings.debug.enabled = Some(false);
        assert!(
            !settings.debug_enabled(),
            "an explicit refusal must survive a debug build"
        );
    }

    /// An existing `settings.json` predates the field, and reads as nobody having said.
    ///
    /// Absent follows the build rather than reading as the *closed* answer: a file written before
    /// the field existed carries no opinion, which is correct — it was never asked. A file carrying
    /// an explicit `false` means `false`.
    #[test]
    fn a_settings_file_written_before_the_switch_existed_carries_no_opinion() {
        let settings: Settings =
            serde_json::from_str(r#"{"debug":{"play_file_roots":["/tunes/karaoke"]}}"#)
                .expect("parse");
        assert_eq!(settings.debug.play_file_roots.len(), 1);
        assert!(settings.debug.enabled.is_none());

        let refused: Settings =
            serde_json::from_str(r#"{"debug":{"enabled":false}}"#).expect("parse");
        assert_eq!(refused.debug.enabled, Some(false));
        assert!(!refused.debug_enabled());
    }

    /// A debug run must not leave `true` behind for a release build to inherit.
    ///
    /// This is the whole reason the field is an `Option`. `save` serializes the struct, so a `bool`
    /// defaulting to `cfg!(debug_assertions)` would write `"accept_uploads": true` into the device's
    /// settings.json on its first debug run — and an Android device upgraded from a `task
    /// build:android` APK to a `RELEASE=1` one keeps its data directory, so it would go on taking
    /// uploads for ever with nobody having chosen that.
    #[test]
    fn nothing_is_written_down_by_a_build_that_merely_defaults_to_debugging() {
        let scratch = Scratch::new("uploads-not-baked-in");
        let paths = scratch.paths();
        Settings::default().save(&paths).expect("save");

        let text = std::fs::read_to_string(paths.settings_file()).expect("read it back");
        assert!(
            text.contains("\"enabled\": null"),
            "the absent answer must round-trip as absent, not as this build's answer: {text}"
        );

        let (reloaded, _) = Settings::load(&paths);
        assert!(reloaded.debug.enabled.is_none());
    }

    #[test]
    fn a_first_run_writes_a_file_and_mints_an_instance_id() {
        let scratch = Scratch::new("first-run");
        let (settings, written) = Settings::load(&scratch.paths());
        assert!(written);
        assert!(!settings.machine.instance_id.is_empty());
        assert!(scratch.paths().settings_file().is_file());
    }

    #[test]
    fn the_instance_id_survives_a_restart() {
        let scratch = Scratch::new("stable-id");
        let (first, _) = Settings::load(&scratch.paths());
        let (second, written) = Settings::load(&scratch.paths());
        // A remote should recognize the machine it talked to yesterday, so this must not change.
        assert_eq!(first.machine.instance_id, second.machine.instance_id);
        assert!(!written, "a second load should not need to rewrite");
    }

    /// The guide melody starts on, and a machine that has said otherwise is still obeyed.
    ///
    /// Both halves matter. The default is what an install with no settings file gets, and it changed:
    /// a guide melody is what makes a half-known song singable, and somebody who does not want it
    /// turns it off in one press. But `update_settings` mirrors a live toggle into the stored
    /// defaults on purpose, so a machine whose owner has already turned it off must keep it off — a
    /// new default that overrode a stated preference would be a bug wearing a feature's clothes.
    #[test]
    fn the_guide_melody_starts_on_unless_this_machine_has_said_otherwise() {
        assert!(PlaybackSettings::default().melody_enabled);

        let scratch = Scratch::new("melody");
        scratch.paths().create().expect("directories");
        std::fs::write(
            scratch.paths().settings_file(),
            r#"{"playback":{"melody_enabled":false}}"#,
        )
        .expect("write");
        let (settings, _) = Settings::load(&scratch.paths());
        assert!(!settings.playback.melody_enabled);
    }

    #[test]
    fn a_partial_settings_file_loads_with_defaults_for_the_rest() {
        let scratch = Scratch::new("partial");
        scratch.paths().create().expect("directories");
        std::fs::write(
            scratch.paths().settings_file(),
            r#"{"machine":{"name":"Sala"},"playback":{"transpose":-2}}"#,
        )
        .expect("write");

        let (settings, _) = Settings::load(&scratch.paths());
        assert_eq!(settings.machine.name, "Sala");
        assert_eq!(settings.playback.transpose, -2);
        // Everything unmentioned is the default, including a freshly minted instance id.
        assert_eq!(settings.playback.tempo_ratio, 1.0);
        assert!(!settings.display.fullscreen);
        assert!(!settings.machine.instance_id.is_empty());
    }

    /// The file a setup program writes, read back the way the machine will read it.
    ///
    /// **This is the guard on `The setup programs pre-write a settings file`.** That decision rests
    /// on a two-key `settings.json` being a whole one: it names no `settings_version`, reads as the
    /// current one by [`current_settings_version`], and takes the defaults for everything it does not
    /// say.
    ///
    /// So the assertion is not "fullscreen came out true". It is **"nothing else moved"**: the
    /// result is the defaults with one field flipped. The day a load starts doing something to a
    /// fresh file, this fails, and whoever wrote it has to decide what an installer's file should
    /// say rather than finding out from an appliance under somebody's television.
    ///
    /// Compared as serialized JSON rather than with `PartialEq`, which `Settings` does not derive —
    /// and which would be the weaker check anyway, since the JSON is what actually gets written.
    #[test]
    fn the_file_a_setup_program_writes_changes_only_fullscreen() {
        let scratch = Scratch::new("setup-written");
        scratch.paths().create().expect("directories");
        std::fs::write(
            scratch.paths().settings_file(),
            r#"{"display":{"fullscreen":true}}"#,
        )
        .expect("write");

        let (settings, written) = Settings::load(&scratch.paths());
        assert!(
            written,
            "the two-key file is filled in and written back whole"
        );
        assert!(
            settings.display.fullscreen,
            "the machine a setup program installed drives a television"
        );

        // `instance_id` is minted per install and the password is generated on the way in, so those
        // are carried across rather than asserted away; `settings_version` is what `save` stamps.
        let mut expected = Settings::default();
        expected.display.fullscreen = true;
        expected.settings_version = CURRENT_SETTINGS_VERSION;
        expected.machine.instance_id = settings.machine.instance_id.clone();
        expected.api.admin_password_hash = settings.api.admin_password_hash.clone();
        expected.api.admin_factory_pin = settings.api.admin_factory_pin.clone();
        assert_eq!(
            serde_json::to_value(&settings).expect("serialize what was loaded"),
            serde_json::to_value(&expected).expect("serialize the defaults"),
            "a setup program's file moved something other than fullscreen"
        );
    }

    #[test]
    fn a_settings_file_from_before_locales_reads_as_english() {
        // Every field is optional and the default is the source language, so nothing about an
        // existing install changes — which is what makes this not a migration.
        let scratch = Scratch::new("locale-absent");
        scratch.paths().create().expect("directories");
        std::fs::write(
            scratch.paths().settings_file(),
            r#"{"machine":{"name":"Sala"}}"#,
        )
        .expect("write");
        let (settings, _) = Settings::load(&scratch.paths());
        assert_eq!(settings.machine.locale(), km_locale::Locale::English);
    }

    #[test]
    fn a_locale_nobody_has_a_catalog_for_does_not_stop_the_machine() {
        // `settings.json` is a file somebody opens. A typo in it should cost the language, not the
        // evening — an appliance under a television that will not come up is the worse failure.
        let scratch = Scratch::new("locale-nonsense");
        scratch.paths().create().expect("directories");
        std::fs::write(
            scratch.paths().settings_file(),
            r#"{"machine":{"locale":"klingon"}}"#,
        )
        .expect("write");
        let (settings, _) = Settings::load(&scratch.paths());
        assert_eq!(settings.machine.locale(), km_locale::Locale::English);
    }

    #[test]
    fn the_book_can_read_the_locale_without_writing_a_settings_file() {
        // `--song-book` runs before `Settings::load` on purpose, so that printing a list does not
        // leave an install behind. Reading the language it should print in must not either.
        let scratch = Scratch::new("locale-readonly");
        scratch.paths().create().expect("directories");
        let file = scratch.paths().settings_file();
        std::fs::write(&file, r#"{"machine":{"locale":"pt-BR"}}"#).expect("write");
        let before = std::fs::read_to_string(&file).expect("read back");

        assert_eq!(
            machine_locale(&scratch.paths()),
            km_locale::Locale::BrazilianPortuguese
        );
        assert_eq!(
            std::fs::read_to_string(&file).expect("read back"),
            before,
            "reading the locale rewrote settings.json"
        );
    }

    /// The same rule one section over: the subscriber is built before the settings are loaded, so
    /// asking where the log goes must not create a settings file for a machine that has never run.
    #[test]
    fn the_log_setting_is_read_without_writing_a_settings_file() {
        let scratch = Scratch::new("logging-readonly");
        scratch.paths().create().expect("directories");
        let file = scratch.paths().settings_file();

        // The case that matters most: no file at all, which is a first start.
        assert_eq!(
            Settings::peek_logging(&scratch.paths()),
            LoggingSettings::default()
        );
        assert!(!file.exists(), "peeking made a settings.json");

        std::fs::write(&file, r#"{"logging":{"file":true,"keep":"all"}}"#).expect("write");
        let before = std::fs::read_to_string(&file).expect("read back");

        let found = Settings::peek_logging(&scratch.paths());
        assert!(found.file);
        assert_eq!(found.keep(), Some(km_logfile::KEEP_ALL));
        assert_eq!(
            std::fs::read_to_string(&file).expect("read back"),
            before,
            "reading the log setting rewrote settings.json"
        );
    }

    /// Both shapes are accepted, and a word nobody can read leaves the default standing.
    #[test]
    fn how_many_to_keep_is_a_number_or_the_word_all() {
        let all = LoggingSettings {
            keep: Some(KeepSetting::Named("all".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(all.keep(), Some(km_logfile::KEEP_ALL));

        let counted = LoggingSettings {
            keep: Some(KeepSetting::Count(200)),
            ..LoggingSettings::default()
        };
        assert_eq!(counted.keep(), Some(200));

        // Not an answer, so the built-in number stands rather than a guess being made. The warning
        // is `warn_about_unread_settings`, which runs once there is somewhere to put it.
        let nonsense = LoggingSettings {
            keep: Some(KeepSetting::Named("lots".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(nonsense.keep(), None);

        assert_eq!(LoggingSettings::default().keep(), None);
    }

    /// Both shapes are accepted, `false` is a written-down no, and an address nobody can read leaves
    /// the console standing.
    #[test]
    fn the_viewer_is_a_switch_or_an_address() {
        let on = LoggingSettings {
            ecapplog: Some(ViewerSetting::On(true)),
            ..LoggingSettings::default()
        };
        assert_eq!(on.ecapplog().as_deref(), Some(km_ecapplog::DEFAULT_ADDRESS));

        let elsewhere = LoggingSettings {
            ecapplog: Some(ViewerSetting::At("192.168.1.5:13991".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(elsewhere.ecapplog().as_deref(), Some("192.168.1.5:13991"));

        // A no that is written down, so somebody can turn this off without deleting the line.
        let off = LoggingSettings {
            ecapplog: Some(ViewerSetting::On(false)),
            ..LoggingSettings::default()
        };
        assert_eq!(off.ecapplog(), None);

        // Not an answer, so the console stands rather than a guess being made. The warning is
        // `warn_about_unread_settings`, which runs once there is somewhere to put it.
        let nonsense = LoggingSettings {
            ecapplog: Some(ViewerSetting::At("nope".to_owned())),
            ..LoggingSettings::default()
        };
        assert_eq!(nonsense.ecapplog(), None);

        assert_eq!(LoggingSettings::default().ecapplog(), None);
    }

    /// The two shapes are told apart by what was written, rather than one swallowing the other.
    ///
    /// `#[serde(untagged)]` tries its variants in order, so a bare `true` must not be read as a
    /// string and an address must not be read as a switch.
    #[test]
    fn the_viewer_key_reads_both_of_its_spellings() {
        let on: LoggingSettings =
            serde_json::from_str(r#"{"ecapplog":true}"#).expect("a switch parses");
        assert_eq!(on.ecapplog, Some(ViewerSetting::On(true)));

        let at: LoggingSettings =
            serde_json::from_str(r#"{"ecapplog":"192.168.1.5:13991"}"#).expect("an address parses");
        assert_eq!(
            at.ecapplog,
            Some(ViewerSetting::At("192.168.1.5:13991".to_owned()))
        );
    }

    /// A settings file that will not parse costs the log file, not the start.
    #[test]
    fn a_broken_settings_file_peeks_as_no_opinion_at_all() {
        let scratch = Scratch::new("logging-broken");
        scratch.paths().create().expect("directories");
        std::fs::write(scratch.paths().settings_file(), "{ not json").expect("write");

        assert_eq!(
            Settings::peek_logging(&scratch.paths()),
            LoggingSettings::default()
        );
    }

    /// The section survives a round trip, and an unset count leaves no key behind.
    #[test]
    fn the_log_section_is_written_back_as_it_was_read() {
        let mut settings = Settings::default();
        settings.logging.file = true;

        let written = serde_json::to_string(&settings).expect("serialize");
        assert!(written.contains(r#""logging":{"file":true}"#), "{written}");

        settings.logging.keep = Some(KeepSetting::Count(200));
        let written = serde_json::to_string(&settings).expect("serialize");
        assert!(
            written.contains(r#""logging":{"file":true,"keep":200}"#),
            "{written}"
        );
    }

    #[test]
    fn a_book_taken_against_a_data_dir_with_no_settings_is_still_in_english() {
        let scratch = Scratch::new("locale-no-file");
        scratch.paths().create().expect("directories");
        assert_eq!(machine_locale(&scratch.paths()), km_locale::Locale::English);
        assert!(
            !scratch.paths().settings_file().exists(),
            "asking for the locale created a settings file"
        );
    }

    #[test]
    fn an_unknown_key_is_ignored_rather_than_refused() {
        let scratch = Scratch::new("unknown-key");
        scratch.paths().create().expect("directories");
        std::fs::write(
            scratch.paths().settings_file(),
            r#"{"machine":{"name":"Sala"},"from_a_future_version":{"x":1}}"#,
        )
        .expect("write");
        let (settings, _) = Settings::load(&scratch.paths());
        assert_eq!(settings.machine.name, "Sala");
    }

    #[test]
    fn a_corrupt_settings_file_is_kept_aside_and_the_machine_still_starts() {
        let scratch = Scratch::new("corrupt");
        scratch.paths().create().expect("directories");
        std::fs::write(scratch.paths().settings_file(), "{ this is not json").expect("write");

        let (settings, _) = Settings::load(&scratch.paths());
        // Defaults, and the machine runs. Losing a configuration is bad; refusing to boot is worse.
        assert_eq!(settings.machine.name, "KaraokeMachine");
        assert!(
            scratch
                .paths()
                .settings_file()
                .with_extension("json.bad")
                .is_file(),
            "the unreadable file should be kept for the owner to look at"
        );
    }

    /// A file with nothing to change is not rewritten on every start.
    ///
    /// **The failure this guards is a settings file rewritten for ever.** What decides whether
    /// `load` writes is the instance id and `ensure_password`, and neither must keep finding work.
    #[test]
    fn a_settings_file_with_nothing_to_do_is_left_alone() {
        let scratch = Scratch::new("settled");
        scratch.paths().create().expect("directories");

        // First load writes: it mints an instance id and a password.
        let (_, written) = Settings::load(&scratch.paths());
        assert!(written);

        // Second load has nothing left to do.
        let (_, written) = Settings::load(&scratch.paths());
        assert!(
            !written,
            "a settled settings file must not be rewritten at every start"
        );
    }

    #[test]
    fn the_lyric_offset_starts_at_zero() {
        // The shipped default has to be exactly 0, because that is the value `shift_ticks` short
        // circuits on -- a machine nobody has calibrated draws the tick the engine published.
        assert_eq!(DisplaySettings::default().lyric_offset_ms, 0);
        assert_eq!(DisplaySettings::default().lyric_offset(), 0);
    }

    #[test]
    fn a_hand_edited_lyric_offset_is_clamped_rather_than_refused() {
        // The file is meant to be edited by hand, so a number outside the range has to behave as the
        // limit rather than stop the machine booting.
        let mut display = DisplaySettings {
            lyric_offset_ms: 30_000,
            ..DisplaySettings::default()
        };
        assert_eq!(display.lyric_offset(), MAX_LYRIC_OFFSET_MS);
        display.lyric_offset_ms = -30_000;
        assert_eq!(display.lyric_offset(), -MAX_LYRIC_OFFSET_MS);
        // And the raw field is left as written, so saving the file back does not silently rewrite
        // what somebody typed.
        assert_eq!(display.lyric_offset_ms, -30_000);
    }

    #[test]
    fn a_window_position_is_a_pair_or_nothing() {
        // Half a position centers the window rather than inventing the other half.
        let display = DisplaySettings {
            x: Some(40),
            ..DisplaySettings::default()
        };
        assert_eq!(display.window_rect().position, None);

        let mut display = DisplaySettings::default();
        let rect = WindowRect {
            position: Some((-1600, 120)),
            width: 900,
            height: 500,
        };
        display.set_window_rect(rect);
        assert_eq!((display.x, display.y), (Some(-1600), Some(120)));
        assert_eq!(display.window_rect(), rect);
    }

    #[test]
    fn settings_survive_a_round_trip_through_the_file() {
        let scratch = Scratch::new("round-trip");
        let mut settings = Settings::default();
        settings.machine.name = "Sala de Estar".to_owned();
        settings.api.bind = "0.0.0.0:9000".to_owned();
        settings.playback.transpose = 3;
        settings.display.lyric_offset_ms = -40;
        settings.mics = vec![MicSettings {
            id: "mic1".to_owned(),
            name: "Wireless".to_owned(),
            device_hint: Some("USB Audio".to_owned()),
            gain: 1.25,
            reverb: 0.4,
            echo: 0.1,
            muted: true,
        }];
        settings.save(&scratch.paths()).expect("save");

        let (loaded, _) = Settings::load(&scratch.paths());
        assert_eq!(loaded.machine.name, "Sala de Estar");
        assert_eq!(loaded.api.socket_addr().port(), 9000);
        assert!(loaded.api.socket_addr().ip().is_unspecified());
        assert_eq!(loaded.playback.transpose, 3);
        assert_eq!(loaded.display.lyric_offset_ms, -40);
        assert_eq!(loaded.mics.len(), 1);
        assert_eq!(loaded.mics[0].gain, 1.25);
        assert!(loaded.mics[0].muted);
    }

    #[test]
    fn an_unparseable_bind_address_falls_back_instead_of_stopping_the_machine() {
        let settings = ApiSettings {
            bind: "not an address".to_owned(),
            ..Default::default()
        };
        let addr = settings.socket_addr();
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), km_api::DEFAULT_PORT);
    }

    #[test]
    fn remembered_mics_are_restored_and_clamped() {
        let settings = Settings {
            mics: vec![MicSettings {
                id: "mic1".to_owned(),
                name: "Mic 1".to_owned(),
                // A hand-edited file can hold anything.
                device_hint: Some("  ".to_owned()),
                gain: 99.0,
                reverb: -1.0,
                echo: 0.0,
                muted: false,
            }],
            ..Default::default()
        };
        let registry = settings.mic_registry();
        let channel = km_queue::MicBus::channel(&registry, "mic1").expect("restored");
        assert_eq!(channel.gain, km_queue::MAX_GAIN);
        assert_eq!(channel.reverb, 0.0);
        assert_eq!(channel.device_hint, None);
    }

    #[test]
    fn a_fresh_install_gets_two_mics() {
        let registry = Settings::default().mic_registry();
        assert_eq!(km_queue::MicBus::channels(&registry).len(), 2);
    }

    #[test]
    fn a_debug_path_inside_an_allowed_root_is_permitted_and_one_outside_is_not() {
        let scratch = Scratch::new("debug-roots");
        let allowed = scratch.0.join("songs");
        std::fs::create_dir_all(&allowed).expect("make it");
        let inside = allowed.join("a.kar");
        std::fs::write(&inside, b"not really a midi").expect("write");
        let outside = scratch.0.join("elsewhere.kar");
        std::fs::write(&outside, b"not really a midi").expect("write");

        let settings = Settings {
            debug: DebugSettings {
                play_file_roots: vec![allowed.clone()],
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(settings.debug_path_allowed(&inside));
        assert!(!settings.debug_path_allowed(&outside));
    }

    #[test]
    fn the_soundfont_switcher_is_off_until_a_bank_is_named() {
        let settings = Settings::default();
        assert!(!settings.soundfont_switcher_on());
        // Including slot 1. The switcher being off means the keys do nothing at all, not that the
        // bundled bank is reachable and the rest are not — there is nothing to compare it against.
        assert!(settings.soundfont_slot(1).is_none());
        assert!(settings.soundfont_slot(2).is_none());
    }

    #[test]
    fn soundfont_slots_are_numbered_from_two_because_one_is_the_bundled_bank() {
        let settings = Settings {
            debug: DebugSettings {
                soundfonts: vec![
                    DebugBank {
                        path: PathBuf::from("/tunes/a.sf2"),
                        name: "First".to_owned(),
                        music_volume: None,
                    },
                    DebugBank {
                        path: PathBuf::from("/tunes/b.sf2"),
                        name: "Second".to_owned(),
                        music_volume: Some(0.8),
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(settings.soundfont_switcher_on());
        // Slot 1 is never in the list, however full it is: it is resolved rather than configured.
        assert!(settings.soundfont_slot(1).is_none());
        assert_eq!(
            settings.soundfont_slot(2).map(|b| b.name.as_str()),
            Some("First")
        );
        assert_eq!(
            settings.soundfont_slot(3).map(|b| b.name.as_str()),
            Some("Second")
        );
        assert_eq!(
            settings.soundfont_slot(3).and_then(|b| b.music_volume),
            Some(0.8)
        );
        // Past the end is empty rather than wrapping round, the same rule the transport strip's
        // function keys follow: a key that does nothing goes on doing nothing as the list grows.
        assert!(settings.soundfont_slot(4).is_none());
        assert!(settings.soundfont_slot(9).is_none());
        // And slot 0 does not exist. No key produces it, but nothing in the type says so.
        assert!(settings.soundfont_slot(0).is_none());
    }

    /// The slot a session ended on comes back, and every way of it not answering is slot 1.
    ///
    /// The three ways are the point: the switcher turned off between two runs, the list shortened
    /// past the remembered index, and a value that is not a switcher slot at all. None of them may
    /// be a failure to start — the machine simply comes up on the bank it resolves for itself.
    #[test]
    fn the_slot_a_session_ended_on_comes_back_unless_it_no_longer_answers() {
        let banks = vec![
            DebugBank {
                path: PathBuf::from("/tunes/a.sf2"),
                name: "First".to_owned(),
                music_volume: None,
            },
            DebugBank {
                path: PathBuf::from("/tunes/b.sf2"),
                name: "Second".to_owned(),
                music_volume: Some(0.8),
            },
        ];
        let with = |slot: Option<u8>, soundfonts: Vec<DebugBank>| Settings {
            debug: DebugSettings {
                soundfonts,
                soundfont_slot: slot,
                ..Default::default()
            },
            ..Default::default()
        };

        let settings = with(Some(3), banks.clone());
        let (slot, bank) = settings
            .restored_soundfont_slot()
            .expect("slot 3 is in the list");
        assert_eq!(slot, 3);
        assert_eq!(bank.name, "Second");
        // The level travels with the bank, which is what a restart has to reproduce as faithfully
        // as a keypress does.
        assert_eq!(bank.music_volume, Some(0.8));

        // Nothing was remembered.
        assert!(
            with(None, banks.clone())
                .restored_soundfont_slot()
                .is_none()
        );
        // The list was shortened past it.
        assert!(
            with(Some(3), banks[..1].to_vec())
                .restored_soundfont_slot()
                .is_none()
        );
        // The switcher was turned off.
        assert!(
            with(Some(3), Vec::new())
                .restored_soundfont_slot()
                .is_none()
        );
        // And slot 1 is the machine's own bank, which is not a thing this can name.
        assert!(with(Some(1), banks).restored_soundfont_slot().is_none());
    }

    /// A machine that has never touched the switcher writes no slot.
    #[test]
    fn a_machine_that_never_switched_writes_no_slot() {
        let json = serde_json::to_string(&Settings::default()).expect("settings serialize");
        assert!(
            !json.contains("soundfont_slot"),
            "a default machine should not advertise a slot it is not on: {json}"
        );
    }

    /// The added field has to parse against every settings file that predates it, which is what
    /// makes this need no `settings_version` migration.
    #[test]
    fn a_settings_file_written_before_the_switcher_existed_still_reads() {
        let json = r#"{ "debug": { "play_file_roots": ["/tunes/karaoke"] } }"#;
        let settings: Settings = serde_json::from_str(json).expect("it should still parse");
        assert_eq!(settings.debug.play_file_roots.len(), 1);
        assert!(settings.debug.soundfonts.is_empty());
        assert!(!settings.soundfont_switcher_on());
    }

    /// A bank with no measured level round-trips without inventing one.
    #[test]
    fn a_bank_with_no_level_writes_no_level() {
        let settings = Settings {
            debug: DebugSettings {
                soundfonts: vec![DebugBank {
                    path: PathBuf::from("/tunes/a.sf2"),
                    name: "Plain".to_owned(),
                    music_volume: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(
            !json.contains("music_volume\":null"),
            "an absent level should be absent rather than null: {json}"
        );
        let back: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.debug.soundfonts[0].music_volume, None);
    }

    #[test]
    fn dot_dot_cannot_walk_out_of_an_allowed_root() {
        let scratch = Scratch::new("traversal");
        let allowed = scratch.0.join("songs");
        std::fs::create_dir_all(&allowed).expect("make it");
        let secret = scratch.0.join("secret.kar");
        std::fs::write(&secret, b"not really a midi").expect("write");

        let settings = Settings {
            debug: DebugSettings {
                play_file_roots: vec![allowed.clone()],
                ..Default::default()
            },
            ..Default::default()
        };
        // The path resolves outside the root, so canonicalising both sides catches it.
        assert!(!settings.debug_path_allowed(&allowed.join("../secret.kar")));
    }

    #[test]
    fn a_path_that_does_not_exist_is_refused_rather_than_guessed_at() {
        let scratch = Scratch::new("missing");
        let settings = Settings {
            debug: DebugSettings {
                play_file_roots: vec![scratch.0.clone()],
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!settings.debug_path_allowed(&scratch.0.join("nothing-here.kar")));
    }

    #[test]
    fn the_api_config_carries_the_persisted_instance_id() {
        let mut settings = Settings::default();
        settings.machine.instance_id = "abc123".to_owned();
        let config = settings.api_config(None);
        assert_eq!(config.instance_id, "abc123");
        assert_eq!(config.machine_name, "KaraokeMachine");
    }

    #[test]
    fn a_silly_token_lifetime_is_floored_rather_than_making_login_useless() {
        let settings = Settings {
            api: ApiSettings {
                token_ttl_secs: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(settings.api_config(None).token_ttl >= Duration::from_secs(60));
    }

    #[test]
    fn wallpaper_settings_map_onto_the_display_crates_config() {
        let settings = WallpaperSettings {
            interval_secs: 45,
            crossfade_ms: 800,
            dim: 2.0,
            ..Default::default()
        };
        let config = settings.to_config(&Paths::rooted_at("/install"));
        assert_eq!(config.interval, Duration::from_secs(45));
        assert_eq!(config.crossfade, Duration::from_millis(800));
        // Clamped, because a scrim above 1.0 would black the screen out.
        assert_eq!(config.dim, 1.0);
    }

    #[test]
    fn a_zero_wallpaper_interval_becomes_one_second_rather_than_a_stall() {
        let settings = WallpaperSettings {
            interval_secs: 0,
            ..Default::default()
        };
        assert_eq!(
            settings.to_config(&Paths::rooted_at("/install")).interval,
            Duration::from_secs(1)
        );
    }

    #[test]
    fn an_unset_wallpaper_folder_resolves_against_the_asset_directory() {
        // The regression this guards: a default of the relative path `assets/wallpapers` is written
        // into settings.json and read back on a machine whose assets are somewhere else entirely --
        // on Android, relative to `/`.
        let settings = WallpaperSettings::default();
        assert!(
            settings.dir.is_none(),
            "the default must mean 'the bundled folder', not a fixed path"
        );
        let paths = Paths::rooted_at("/install");
        assert_eq!(
            settings.to_config(&paths).dir,
            paths.asset(WALLPAPER_SUBDIR)
        );
    }

    #[test]
    fn a_configured_wallpaper_folder_is_used_as_given() {
        let settings = WallpaperSettings {
            dir: Some(PathBuf::from("/photos/holiday")),
            ..Default::default()
        };
        assert_eq!(
            settings.to_config(&Paths::rooted_at("/install")).dir,
            PathBuf::from("/photos/holiday"),
            "an explicit folder must not be reinterpreted as an asset sub-path"
        );
    }

    // -- the checkout overlay ---------------------------------------------------------------------

    /// Builds `Paths` over a scratch tree with the overlay switched on.
    fn overlaid(scratch: &Scratch) -> Paths {
        let mut paths = scratch.paths();
        paths.overlay_asset_dir = checkout_overlay(scratch.checkout());
        assert!(
            paths.overlay_asset_dir.is_some(),
            "the scratch tree should look like a checkout"
        );
        paths
    }

    #[test]
    fn an_overlay_file_wins_over_the_bundled_one() {
        let scratch = Scratch::new("overlay-wins");
        scratch.write("assets/soundfont/gm.sf2");
        let local = scratch.write("local/assets/soundfont/gm.sf2");
        assert_eq!(overlaid(&scratch).asset("soundfont/gm.sf2"), local);
    }

    /// The per-path rule, which is the one a reader most wants proved: an overlay holding only a
    /// SoundFont must leave the bundled font and the bundled wallpapers exactly where they were.
    #[test]
    fn an_asset_the_overlay_does_not_hold_still_comes_from_the_bundle() {
        let scratch = Scratch::new("overlay-partial");
        scratch.write("local/assets/soundfont/gm.sf2");
        let paths = overlaid(&scratch);
        assert_eq!(
            paths.asset(FONT_SUBPATH),
            paths.asset_dir.join(FONT_SUBPATH)
        );
        assert_eq!(
            paths.asset(WALLPAPER_SUBDIR),
            paths.asset_dir.join(WALLPAPER_SUBDIR)
        );
    }

    /// A fence against the new branch changing what the old behavior was.
    #[test]
    fn no_overlay_resolves_exactly_as_it_always_did() {
        let paths = Paths::rooted_at("/install");
        assert_eq!(
            paths.asset("soundfont/gm.sf2"),
            paths.asset_dir.join("soundfont/gm.sf2")
        );
    }

    /// An overlay wallpaper *folder* replaces the bundled one rather than adding to it, so the
    /// committed set is not shown while a pack is sitting there. That is the accepted cost of
    /// `WallpaperConfig::dir` being one path; not creating the folder is how somebody chooses
    /// otherwise. See the `Local assets in a checkout` decision in docs/decisions/.
    ///
    /// The overlay fixture is a PNG and cannot be a `pack.zip`: `Scratch::write` writes thirty-odd
    /// bytes of prose, which is a perfectly good stand-in for a PNG that nothing decodes and
    /// **not** a stand-in for a zip, because a zip is the one thing here that gets opened. The rule
    /// is "holds pictures" rather than "exists", so the fixture has to hold one.
    #[test]
    fn an_overlay_wallpaper_folder_replaces_the_bundled_one() {
        let scratch = Scratch::new("overlay-wallpapers");
        scratch.write_pack("assets/wallpapers/default-wallpapers.zip");
        scratch.write_pack("local/assets/wallpapers/01-pack.zip");
        let paths = overlaid(&scratch);
        let resolved = WallpaperSettings::default().to_config(&paths).dir;
        assert_eq!(
            resolved,
            scratch.0.join("local/assets").join(WALLPAPER_SUBDIR)
        );
        assert_ne!(resolved, paths.asset_dir.join(WALLPAPER_SUBDIR));
    }

    /// The trap the whole `holds_wallpapers` rule exists to avoid.
    ///
    /// `Paths::create` makes the owner's folder empty on first start so that it can be *found*, and
    /// `Playlist` holds exactly one directory with no merging. An existence test would therefore
    /// hand the display an empty folder the moment that line ran, and the machine would come up on a
    /// black screen with nothing in any log to explain it.
    #[test]
    fn an_empty_wallpaper_folder_falls_through_to_the_bundled_set() {
        let scratch = Scratch::new("empty-wallpapers");
        scratch.write_pack("assets/wallpapers/default-wallpapers.zip");
        let paths = scratch.paths();
        // Exactly what a first start leaves behind.
        paths.create().expect("make the writable directories");
        assert!(
            paths.wallpapers_dir().is_dir(),
            "the owner's folder must be made, or nobody finds it"
        );

        let (resolved, source) = paths.wallpaper_dir();
        assert_eq!(source, WallpaperSource::Bundled);
        assert_eq!(resolved, paths.asset_dir.join(WALLPAPER_SUBDIR));
    }

    /// One debug extra must not make an empty owner folder look as though it holds wallpapers.
    ///
    /// **The trap above, arriving by a different door, and the reason `holds_wallpapers` passes
    /// `&[]`.** `debug.wallpapers` names files layered *on top of* whichever folder wins; counting
    /// them while deciding which folder wins would let one entry suppress the shipped set — turning
    /// an additive setting into a replacing one, and blanking the screen down to that single
    /// picture with nothing in any log.
    #[test]
    fn a_debug_extra_does_not_make_an_empty_folder_look_full() {
        let scratch = Scratch::new("extra-not-contents");
        scratch.write_pack("assets/wallpapers/default-wallpapers.zip");
        let named = scratch.write("elsewhere/named.png");
        let paths = scratch.paths();
        paths.create().expect("make the writable directories");

        let settings = Settings {
            debug: DebugSettings {
                wallpapers: vec![named.clone()],
                ..DebugSettings::default()
            },
            ..Settings::default()
        };

        let (resolved, source) = settings.wallpaper.folder(&paths);
        assert_eq!(
            source,
            WallpaperSource::Bundled,
            "an extra is not the owner's folder having contents"
        );
        assert_eq!(resolved, paths.asset_dir.join(WALLPAPER_SUBDIR));

        // ...and it *is* carried, on top of that folder rather than instead of it.
        let config = settings.wallpaper_config(&paths);
        assert_eq!(config.dir, resolved);
        assert_eq!(config.extra, vec![named]);
    }

    /// `wallpaper.dir` wins outright, and says which rule won.
    #[test]
    fn a_named_wallpaper_folder_reports_itself_as_the_setting() {
        let scratch = Scratch::new("named-wallpapers");
        scratch.write_pack("assets/wallpapers/default-wallpapers.zip");
        scratch.write_pack("wallpapers/holiday.zip");
        let paths = scratch.paths();

        let named = scratch.root().join("somewhere-else");
        let settings = WallpaperSettings {
            dir: Some(named.clone()),
            ..WallpaperSettings::default()
        };
        let (resolved, source) = settings.folder(&paths);
        assert_eq!(resolved, named, "a folder somebody named is their answer");
        assert_eq!(
            source,
            WallpaperSource::Setting,
            "and the rule that won is reported as its own answer rather than as one of the three"
        );
    }

    /// The point of the folder: put pictures in it and they are the ones shown.
    #[test]
    fn the_owners_own_wallpaper_folder_replaces_the_shipped_set() {
        let scratch = Scratch::new("owner-wallpapers");
        scratch.write_pack("assets/wallpapers/default-wallpapers.zip");
        scratch.write_pack("wallpapers/holiday.zip");
        let paths = scratch.paths();

        let (resolved, source) = paths.wallpaper_dir();
        assert_eq!(source, WallpaperSource::Owner);
        assert_eq!(resolved, paths.wallpapers_dir());
        assert_eq!(WallpaperSettings::default().to_config(&paths).dir, resolved);
    }

    /// A folder is empty if nothing in it would reach the screen, which is not the same as having no
    /// files in it. A zip that will not open contributes no images to a scan, so treating its mere
    /// presence as "the owner has wallpapers here" would blank the screen on a corrupt download —
    /// the failure this rule exists to prevent, arriving by a different door.
    #[test]
    fn a_wallpaper_folder_holding_nothing_that_draws_falls_through() {
        let scratch = Scratch::new("undrawable-wallpapers");
        scratch.write_pack("assets/wallpapers/default-wallpapers.zip");
        scratch.write("wallpapers/pack.zip"); // not a zip; thirty bytes of prose
        scratch.write("wallpapers/notes.txt");
        let paths = scratch.paths();

        let (resolved, source) = paths.wallpaper_dir();
        assert_eq!(source, WallpaperSource::Bundled);
        assert_eq!(resolved, paths.asset_dir.join(WALLPAPER_SUBDIR));
    }

    /// The owner's folder beats the checkout overlay, which beats the shipped set. Asserted together
    /// because the order is the whole rule and three separate tests would not pin it.
    #[test]
    fn the_owners_folder_beats_the_overlay_which_beats_the_bundled_set() {
        let scratch = Scratch::new("wallpaper-order");
        scratch.write_pack("assets/wallpapers/default-wallpapers.zip");
        scratch.write_pack("local/assets/wallpapers/01-pack.zip");
        scratch.write_pack("wallpapers/holiday.zip");
        let paths = overlaid(&scratch);
        assert_eq!(paths.wallpaper_dir().1, WallpaperSource::Owner);

        // Take the owner's pictures away and the overlay is next, not the bundled set.
        std::fs::remove_dir_all(paths.wallpapers_dir()).expect("remove the owner's folder");
        assert_eq!(paths.wallpaper_dir().1, WallpaperSource::Overlay);
    }

    /// `wallpaper.dir` wins over all three, and **unconditionally** — a folder somebody named is
    /// their answer even when it is empty. The alternative is a setting that silently stops applying,
    /// which is a worse thing to debug than an empty screen you asked for.
    #[test]
    fn a_configured_wallpaper_folder_beats_the_owners_folder_even_when_empty() {
        let scratch = Scratch::new("configured-beats-owner");
        scratch.write_pack("wallpapers/holiday.zip");
        let settings = WallpaperSettings {
            dir: Some(PathBuf::from("/photos/empty")),
            ..Default::default()
        };
        assert_eq!(
            settings.to_config(&scratch.paths()).dir,
            PathBuf::from("/photos/empty")
        );
    }

    /// The three settings levers still win outright over both directories.
    #[test]
    fn a_configured_wallpaper_folder_still_beats_the_overlay() {
        let scratch = Scratch::new("overlay-configured");
        scratch.write("local/assets/wallpapers/pack.zip");
        let settings = WallpaperSettings {
            dir: Some(PathBuf::from("/photos/holiday")),
            ..Default::default()
        };
        assert_eq!(
            settings.to_config(&overlaid(&scratch)).dir,
            PathBuf::from("/photos/holiday")
        );
    }

    /// The hazard this design is built around.
    ///
    /// `rooted_at` is what the tests use and what a portable install uses, and the test suite runs
    /// with the workspace root as its working directory — which on a developer's machine has both
    /// `assets/` and `local/assets/`. A `rooted_at` that computed an overlay would therefore read a
    /// real 206 MiB bank here, and CI, which has no `local/assets/`, would never notice.
    #[test]
    fn rooted_at_never_enables_the_overlay() {
        assert!(Paths::rooted_at("/install").overlay_asset_dir.is_none());
        let here = std::env::current_dir().expect("a working directory");
        assert!(Paths::rooted_at(&here).overlay_asset_dir.is_none());
    }

    /// `--data-dir` moves the data and leaves the assets where they are.
    ///
    /// The two constructors differ in exactly one way and it is this one. `rooted_at` puts assets
    /// under the root, which is right for a test and for a portable install; `data_rooted_at` is
    /// what the command line takes, and a flag whose whole promise is "keep a run out of the real
    /// install" must not also take away the SoundFont, the font and the wallpapers — which it did,
    /// leaving a scratch run on a sine test tone over a plain gradient.
    #[test]
    fn a_data_dir_moves_the_data_and_not_the_assets() {
        let root = PathBuf::from("/somewhere/scratch");
        let paths = Paths::data_rooted_at(&root);

        assert_eq!(paths.config_dir, root, "settings follow --data-dir");
        assert_eq!(paths.data_dir, root, "the catalog follows --data-dir");
        assert!(
            paths.packages_dir().starts_with(&root),
            "packages follow --data-dir, at {}",
            paths.packages_dir().display()
        );

        assert!(
            !paths.asset_dir.starts_with(&root),
            "assets must NOT follow --data-dir, but landed at {}",
            paths.asset_dir.display()
        );
        assert_eq!(
            paths.asset_dir,
            Paths::discover_asset_dirs().0,
            "they are whatever a run with no --data-dir would have used"
        );

        // And the contrast, so that the difference between the two is pinned rather than implied.
        assert_eq!(Paths::rooted_at(&root).asset_dir, root.join("assets"));
    }

    #[test]
    fn the_overlay_needs_both_directories() {
        let scratch = Scratch::new("overlay-predicate");
        let root = &scratch.0;
        assert_eq!(checkout_overlay(root), None, "neither directory");

        std::fs::create_dir_all(root.join("assets")).expect("the bundled tree");
        assert_eq!(checkout_overlay(root), None, "no overlay to use");

        std::fs::remove_dir_all(root.join("assets")).expect("undo");
        std::fs::create_dir_all(root.join("local").join("assets")).expect("the overlay");
        assert_eq!(
            checkout_overlay(root),
            None,
            "an overlay may only supplement a base that is really there"
        );

        std::fs::create_dir_all(root.join("assets")).expect("the bundled tree");
        assert_eq!(
            checkout_overlay(root),
            Some(root.join("local").join("assets"))
        );
    }

    /// The claim that an installed build cannot reach the overlay, made testable rather than left as
    /// a comment: every carrier lands `assets/` beside the executable, and that is this branch.
    #[test]
    fn an_exe_sibling_asset_dir_gets_no_overlay() {
        let scratch = Scratch::new("overlay-exe-sibling");
        let exe_dir = scratch.0.join("install");
        std::fs::create_dir_all(exe_dir.join("assets")).expect("an installed layout");
        let (asset_dir, overlay) = Paths::asset_dirs_from(Some(&exe_dir), Some(scratch.checkout()));
        assert_eq!(asset_dir, exe_dir.join("assets"));
        assert_eq!(
            overlay, None,
            "a build with assets beside it must never consult a working directory"
        );
    }

    /// The macOS bundle branch, same claim.
    #[test]
    fn a_macos_bundle_asset_dir_gets_no_overlay() {
        let scratch = Scratch::new("overlay-bundle");
        let exe_dir = scratch.0.join("Karaoke Machine.app/Contents/MacOS");
        std::fs::create_dir_all(&exe_dir).expect("the bundle");
        let resources = scratch
            .0
            .join("Karaoke Machine.app/Contents/Resources/assets");
        std::fs::create_dir_all(&resources).expect("the resources");
        let (asset_dir, overlay) = Paths::asset_dirs_from(Some(&exe_dir), Some(scratch.checkout()));
        assert_eq!(asset_dir, resources.canonicalize().unwrap_or(resources));
        assert_eq!(overlay, None, "a bundle must never consult a checkout");
    }

    /// The deliberate crossover, pinned so nobody "fixes" it: a `Contents/MacOS` executable with no
    /// `Resources/assets` is the documented "developer running the binary straight out of a bundle"
    /// case, falls through to the working directory, and may therefore overlay.
    #[test]
    fn a_bundle_without_resources_falls_through_and_may_overlay() {
        let scratch = Scratch::new("overlay-bundle-bare");
        let exe_dir = scratch.0.join("Karaoke Machine.app/Contents/MacOS");
        std::fs::create_dir_all(&exe_dir).expect("the bundle");
        let cwd = scratch.checkout().to_path_buf();
        let (asset_dir, overlay) = Paths::asset_dirs_from(Some(&exe_dir), Some(&cwd));
        assert_eq!(asset_dir, cwd.join("assets"));
        assert_eq!(overlay, Some(cwd.join("local").join("assets")));
    }

    #[test]
    fn a_tidied_path_is_absolute_and_free_of_the_windows_long_path_prefix() {
        // `Cargo.toml` exists relative to the crate, so this exercises the canonicalising branch.
        let tidied = tidy(Path::new("Cargo.toml"));
        assert!(tidied.is_absolute());
        assert!(
            !tidied.to_string_lossy().starts_with(r"\\?\"),
            "settings.json is hand-edited; {} would invite somebody to 'fix' it",
            tidied.display()
        );
        assert!(tidied.ends_with("Cargo.toml"));
    }

    #[test]
    fn a_path_that_does_not_exist_is_left_as_it_was() {
        // Canonicalising fails, and inventing an absolute path would be worse than keeping what the
        // caller said.
        let given = Path::new("no/such/package.kmpkg");
        assert_eq!(tidy(given), given.to_path_buf());
    }

    /// Makes an empty file, which is all the scan looks at — it reads names, never contents.
    fn touch(path: &Path) {
        std::fs::write(path, b"").expect("write a file");
    }

    #[test]
    fn packages_live_beside_the_catalog_and_not_among_the_assets() {
        // The guard against somebody later "making this consistent" with wallpapers and the
        // SoundFont. It is not an asset: the asset tree is read-only where it counts -- root-owned
        // under /opt on Debian, inside a signed bundle on macOS -- and packages are the owner's own
        // files, which accumulate and are read in place for as long as they stay installed.
        let paths = Paths::rooted_at("/install");
        assert_eq!(paths.packages_dir(), PathBuf::from("/install/packages"));
        assert_ne!(paths.packages_dir(), paths.asset(PACKAGE_SUBDIR));
        assert_eq!(
            paths.packages_dir().parent(),
            paths.library_file().parent(),
            "the packages folder belongs beside library.sqlite"
        );
    }

    #[test]
    fn starting_the_machine_makes_the_folder_songs_are_dropped_into() {
        // Unlike the asset directory, which is deliberately never created. A folder an owner has to
        // work out the existence of before they can use it is no use to them.
        let scratch = Scratch::new("packages-created");
        let paths = scratch.paths();
        assert!(!paths.packages_dir().is_dir());
        paths.create().expect("directories");
        assert!(paths.packages_dir().is_dir());
    }

    #[test]
    fn a_folder_that_is_not_there_is_made_rather_than_being_an_error() {
        let scratch = Scratch::new("packages-missing");
        let dir = scratch.paths().packages_dir();
        assert_eq!(packages_to_install(&dir), Vec::<PathBuf>::new());
        assert!(dir.is_dir(), "the scan should have made it");
    }

    #[test]
    fn only_packages_are_picked_up_and_the_order_is_by_name() {
        let scratch = Scratch::new("packages-order");
        let dir = scratch.paths().packages_dir();
        std::fs::create_dir_all(&dir).expect("the folder");
        // Made out of order, so a scan that just returned what the filesystem said would be caught
        // on at least one platform.
        touch(&dir.join("vol2.kmpkg"));
        touch(&dir.join("vol1.kmpkg"));
        // Neither of these is a package: one is the wrong extension, and the other is what the
        // curation tool leaves beside a build.
        touch(&dir.join("notes.txt"));
        touch(&dir.join("vol1.kmpkg.bak"));
        // A directory is never a package, whatever it is called. Nothing puts one beside a package
        // any more — media lives inside the `.kmpkg` — but a folder somebody made by hand, or left
        // behind by an interrupted copy, must still be passed over rather than opened.
        std::fs::create_dir_all(dir.join("vol1.kmpkg.d")).expect("a stray directory");

        let found = packages_to_install(&dir);
        let names: Vec<_> = found
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert_eq!(names, vec!["vol1.kmpkg", "vol2.kmpkg"]);
        assert!(
            found.iter().all(|path| path.is_absolute()),
            "install takes a path, not a name: {found:?}"
        );
    }

    #[test]
    fn the_extension_is_matched_whatever_case_it_was_copied_in() {
        // The folder is somewhere a person copies files into, and Windows hands back whatever case
        // the copy had.
        let scratch = Scratch::new("packages-case");
        let dir = scratch.paths().packages_dir();
        std::fs::create_dir_all(&dir).expect("the folder");
        touch(&dir.join("SHOUTED.KMPKG"));
        assert_eq!(packages_to_install(&dir).len(), 1);
    }

    #[test]
    fn folders_the_owner_named_are_scanned_after_the_machines_own() {
        let scratch = Scratch::new("packages-extra-dirs");
        let mut paths = scratch.paths();
        assert_eq!(
            paths.packages_dirs(),
            vec![paths.packages_dir()],
            "with none set, nothing may change for anybody"
        );

        let mine = PathBuf::from("/tunes/karaoke");
        let also = PathBuf::from("/tunes/more-karaoke");
        paths.extra_package_dirs = vec![mine.clone(), also.clone()];
        // Order is the assertion, not an incidental: the machine's own folder stays first, so which
        // of two packages wanting one bank keeps it does not move for an owner who adds a folder.
        assert_eq!(
            paths.packages_dirs(),
            vec![paths.packages_dir(), mine, also]
        );
    }

    /// The setting is optional and absent from a file that does not use it, so an existing
    /// `settings.json` neither gains a key nor fails to load.
    #[test]
    fn extra_package_folders_are_absent_from_a_file_that_names_none() {
        let settings = Settings::default();
        assert!(settings.package_dirs.is_empty());
        let json = serde_json::to_string(&settings).expect("settings serialize");
        assert!(
            !json.contains("package_dirs"),
            "an unused key must not be written into everybody's settings file: {json}"
        );
        // ...and a file written before this existed still loads.
        let older = serde_json::json!({ "packages": [], "packages_ignored": [] });
        serde_json::from_value::<Settings>(older).expect("a settings file from before this key");
    }

    #[test]
    fn what_the_folder_holds_is_what_is_offered() {
        // The old pair of tests here asserted that a package already in `settings.packages` was not
        // offered twice, and that one the owner had uninstalled stayed out while its file remained.
        // Neither is this function's business any more: deduplication is by package **id** in
        // `machine::startup_plan`, because the same file can be in two folders under two names; and
        // an uninstall deletes the file, so there is nothing left to keep out.
        let scratch = Scratch::new("packages-folder-is-truth");
        let dir = scratch.paths().packages_dir();
        std::fs::create_dir_all(&dir).expect("the folder");
        let package = dir.join("vol1.kmpkg");
        touch(&package);
        assert_eq!(packages_to_install(&dir), vec![tidy(&package)]);
    }

    /// With no second candidate — which is every platform but Android — a handed-in file is written
    /// into the folder the machine scans first, exactly as it was before the two questions were
    /// separated.
    #[test]
    fn the_write_folder_is_the_private_one_when_there_is_no_other() {
        let scratch = Scratch::new("write-dir-plain");
        let paths = scratch.paths();
        assert_eq!(paths.packages_write_dir(), paths.packages_dir());
        assert_eq!(paths.soundfonts_write_dir(), paths.soundfonts_dir());
    }

    /// Android's external folder is preferred for *writing* while staying second for *scanning*.
    ///
    /// The asymmetry is the point and is why the two are separate methods: a package reaches tens of
    /// gigabytes and internal storage is the smaller volume, but nothing dropped onto shared storage
    /// may displace what the machine already installed.
    #[test]
    fn a_writable_second_folder_takes_the_writes_but_not_the_first_scan() {
        let scratch = Scratch::new("write-dir-shared");
        let mut paths = scratch.paths();
        let shared = scratch.root().join("shared");
        paths.extra_data_dir = Some(shared.clone());
        paths.extra_data_writable = true;

        assert_eq!(paths.packages_write_dir(), shared.join(PACKAGE_SUBDIR));
        assert_eq!(paths.soundfonts_write_dir(), shared.join(SOUNDFONT_SUBDIR));
        assert_eq!(
            paths.packages_dirs().first(),
            Some(&paths.packages_dir()),
            "the private folder is still scanned first"
        );
    }

    /// A folder that is there but cannot be written to still contributes what it holds.
    ///
    /// Read and write are separate bits in the state Android reports, and conflating them would
    /// either stop scanning a read-only volume or send every copy at one that must fail.
    #[test]
    fn a_read_only_second_folder_is_scanned_but_not_written_to() {
        let scratch = Scratch::new("write-dir-readonly");
        let mut paths = scratch.paths();
        let shared = scratch.root().join("shared");
        paths.extra_data_dir = Some(shared.clone());
        paths.extra_data_writable = false;

        assert_eq!(paths.packages_write_dir(), paths.packages_dir());
        assert!(
            paths.packages_dirs().contains(&shared.join(PACKAGE_SUBDIR)),
            "a read-only folder is still worth scanning"
        );
    }

    /// The asset tree is never the machine's to delete from, whatever route reaches it.
    #[test]
    fn the_shipped_asset_tree_is_not_the_machines_to_delete_from() {
        let scratch = Scratch::new("delete-guard");
        let paths = scratch.paths();

        assert!(paths.is_mine_to_delete(&paths.packages_dir().join("vol1.kmpkg")));
        assert!(paths.is_mine_to_delete(&paths.soundfonts_dir().join("gm.sf2")));
        assert!(!paths.is_mine_to_delete(&paths.asset_dir.join("soundfont/gm.sf2")));
        assert!(!paths.is_mine_to_delete(Path::new("/tunes/karaoke/vol1.kmpkg")));
    }

    #[test]
    fn a_debug_package_list_is_absent_from_a_file_that_names_none() {
        let settings = Settings::default();
        assert!(settings.debug.packages.is_empty());
        let json = serde_json::to_string(&settings).expect("settings serialize");
        assert!(
            !json.contains("\"packages\""),
            "a shipped machine must not advertise an escape hatch it is not using: {json}"
        );
    }
}
