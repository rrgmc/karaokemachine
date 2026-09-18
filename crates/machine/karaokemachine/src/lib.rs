//! The karaoke machine, as a library.
//!
//! The machine lives here rather than in `main.rs` for one reason: **Android loads a shared library
//! and calls into it, it does not execute a binary.** So the thing that runs the machine has to be
//! callable from a library entry point, and the command-line front end becomes a thin caller in
//! `main.rs` rather than the only way in.
//!
//! Startup order is the design, and it is the same on both platforms:
//!
//! 1. **Settings**, so everything else knows what it is doing.
//! 2. **The audio thread**, which reports what output it managed to open.
//! 3. **The machine** — the catalog and playback state.
//! 4. **Bind the API**, which claims the port and resolves the reachable address. This happens
//!    *before* the display so the connect panel has a URL from the first frame; a machine showing an
//!    idle screen with no address on it is the failure that whole feature exists to prevent. If
//!    binding fails, the failure itself goes on screen.
//! 5. **The watchdog**, which advances the queue when a song ends on its own.
//! 6. **`READY=1` to the service manager**, if there is one — everything that loads has loaded, and
//!    the screen deliberately is not waited for. On the appliance this is what lets the boot splash
//!    stay up until this moment instead of going four seconds early; see `notify.rs`.
//! 7. **The display**, on the calling thread, because macOS requires SDL on the main thread — and on
//!    Android `SDL_main` is already the thread SDL expects.
//!
//! Nothing in steps 2 through 4 is fatal. No SoundFont, no audio device, no free port, no network —
//! each degrades to something the machine reports rather than something that stops it starting.

#[cfg(any(target_os = "android", test))]
mod androidassets;
#[cfg(target_os = "android")]
mod androidctx;
// Public, unlike `androidctx` beside it, and the difference is who calls it. Android's context
// arrives through `JNI_OnLoad`, which the loader calls inside this library; iOS's directories arrive
// from Swift through `km-machine-ios`, which is a separate crate and needs a name it can reach.
#[cfg(target_os = "ios")]
pub mod ioscfg;
// The catalog of fetchable banks, which is a crate of its own now — `crates/machine/km-banks`.
// **Re-exported under the name it had**, so that every `crate::banks::…` call site is untouched by
// the move: there are seven of them across `fetch.rs`, `firstrun.rs` and `machine.rs`, and a move
// that rewrote all seven would have made the diff about this crate rather than about the new one.
use km_banks as banks;
mod cdg;
// One machine per data directory, for as long as this one runs. See the module for why the
// directory rather than the port is what cannot be shared.
mod claim;
// The command line, public because two binary crate roots call into it and neither can see a
// `main.rs` module tree. It is the front end and not the machine: `run` below is what Android reaches
// without going near it. See `src/cli.rs` for why the binaries are two.
mod admin;
pub mod cli;
mod connect;
mod display;
mod dropped;
mod engine;
mod fetch;
mod firstrun;
mod handed;
mod machine;
mod notify;
mod power;
mod register;
mod remote;
pub mod settings;
mod soundfont;
// The run that draws for an encoder instead of a television. Behind `video` because it encodes with
// ffmpeg, which is the same dependency the decoder beside it needs.
#[cfg(feature = "video")]
mod stream;
// The icon in the bar, for a streaming run that has no window to be its face.
#[cfg(all(feature = "video", feature = "tray"))]
mod tray;
mod ultrastar;
mod video;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use km_api::events::Events;
use km_api::{ApiState, ConnectInfo};

use crate::engine::Engine;
use crate::machine::Machine;
use crate::settings::{Paths, Settings};

/// How often the machine does its own housekeeping — advancing the queue, announcing lyric lines.
///
/// 20 Hz. Fast enough that the gap between songs is not heard, slow enough to be free. The 4 Hz API
/// state tick is too slow for this: a quarter-second of silence between songs is what a party
/// notices.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// How long a shutdown waits for the poll thread to finish its pass.
///
/// **Sized against systemd, not against the poll.** An ordinary pass is over in well under a
/// millisecond and the wait costs nothing; what this bounds is a pass that reached `advance` and is
/// opening a song. Five seconds sits inside `TimeoutStopSec=30s` with the three the API wait may
/// take in front of it, so the machine reports a slow stop in its own words and exits, rather than
/// being killed with nothing said.
pub const WATCHDOG_STOP_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// How often it looks in the first instants, when what it is waiting for is a handover.
///
/// **A tenth of a second, and it is worth a constant of its own because of a one-millisecond race.**
/// On the appliance the boot splash holds DRM master until the machine says it is ready — see
/// `notify.rs` — and then releases it. Measured: the machine's first attempt at the screen came at
/// 11.476 s and Plymouth finished letting go at 11.477. Losing by a millisecond then cost *two
/// seconds* of black television, because the next rung of the ladder was the one below.
///
/// The thing being waited for here resolves in milliseconds, so the interval should be measured in
/// them. Twenty extra probes in the first [`DISPLAY_RETRY_HANDOVER_FOR`] is the whole price, each one
/// an `SDL_Init(VIDEO)` that fails immediately, and only on a box that has not got a screen yet.
const DISPLAY_RETRY_HANDOVER: Duration = Duration::from_millis(100);

/// How long the instant retry lasts before it becomes the merely quick one.
///
/// Long enough to cover a splash letting go and a connector finishing its probe, short enough that
/// a box with genuinely no display spends almost none of its life in this rung.
const DISPLAY_RETRY_HANDOVER_FOR: Duration = Duration::from_secs(3);

/// How often the machine looks again for a display it did not find, while the failure is fresh.
///
/// Two seconds, from [`DISPLAY_RETRY_HANDOVER_FOR`] to [`DISPLAY_RETRY_EARLY_FOR`]. That window is
/// the cold boot: on the appliance the connector became usable about 1.1 s after `/dev/dri/card0`
/// did, so this is the interval that turns the measured race into one extra probe rather than a
/// black television.
const DISPLAY_RETRY_EARLY: Duration = Duration::from_secs(2);

/// How long the quick retry lasts before it slows down.
const DISPLAY_RETRY_EARLY_FOR: Duration = Duration::from_secs(30);

/// ...and how often it looks after that, for as long as the machine runs.
///
/// **For ever is the requirement.** The case this exists for is a
/// television switched off at the wall: HDMI hotplug detect goes with it, so the connector reads
/// `disconnected` at boot and no `ExecStartPre` can wait for information that arrives at nine in the
/// evening. The cost is one connector probe every fifteen seconds, which is less often than the
/// kernel's own `drm_kms_helper_poll` already asks the same connector; on a box with no `/dev/dri`
/// at all SDL finds nothing to open and the probe is a `readdir`.
const DISPLAY_RETRY_LATE: Duration = Duration::from_secs(15);

/// How the machine should run, beyond what settings say.
///
/// Everything here comes from the command line on a desktop and is left at its default on Android,
/// which has no command line.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Play one MIDI or KAR file directly, bypassing packages.
    pub play: Option<PathBuf>,
    /// Run without a window: the API, the engine and the catalog only.
    pub headless: bool,
    /// Draw the screen for an encoder instead of a television, and serve it as one HLS stream.
    ///
    /// **A way of running the machine rather than a build of it**, on the same terms as
    /// [`Options::headless`] beside it: it overrides `display.enabled` for this process and writes
    /// nothing back, so an appliance debugged this way still opens its television next time.
    ///
    /// A build without the `video` feature has no encoder and refuses the flag by name, rather than
    /// starting a machine whose whole output is missing.
    pub stream: bool,
    /// Fill the screen, or do not, whatever `display.fullscreen` says. `None` leaves it to settings.
    ///
    /// **Deliberately not written back**, for the reason [`Options::api_bind`] gives at length: this
    /// changes what *this process* does and nothing about what the machine does from now on.
    /// `--fullscreen` and `--windowed` are the two spellings; Android reaches [`run`] with
    /// `Options::default()` and so is unaffected either way.
    ///
    /// **`None` carries a second meaning, and it is the one that is easy to miss**: it is also what
    /// entitles the run to write down how the window was *left* when the display closes, so that `F`
    /// outlives the process. `Some` in either direction turns that off along with everything else,
    /// which is the whole of how "this run writes nothing" stays true. See
    /// `DisplayConfig::remember_fullscreen`.
    pub fullscreen: Option<bool>,
    /// Report frame statistics once a second — `fps`, mean and worst draw time, mean and worst
    /// interval — at `info`. Off unless somebody asked; see `FrameMeter` in `display.rs`.
    ///
    /// `KM_FRAME_STATS` turns it on too, and that is not redundancy: Android reaches [`run`] through
    /// `SDL_main` with no command line at all, and on the appliance an `Environment=` line in the
    /// unit is a smaller change than editing `ExecStart=`.
    pub frame_stats: bool,
    /// Bind the API here for this run, instead of wherever `api.bind` in settings.json says.
    ///
    /// **Deliberately not written back.** Every other way to move the API — the settings file, a
    /// `PUT` from the remote — changes where the machine lives from now on; this changes where
    /// *this process* lives and nothing else, which is the whole reason it exists. A second machine
    /// on one box needs its own port and its own `--data-dir`, and a flag that persisted would make
    /// the second run silently rewrite the first one's home.
    pub api_bind: Option<SocketAddr>,
    /// Serve the development console at `/dev/` for this run, whatever `api.serve_dev_remote` says.
    ///
    /// **Deliberately not written back**, like the two above. One-way on purpose: this can only turn
    /// the page *on*, because it exists so that a developer or a curator does not have to edit
    /// settings.json to reach the ACL editor or the uploads switch. A machine that has been told in
    /// settings to serve the console goes on serving it whether or not the flag is given, which is
    /// what `api.serve_dev_remote: true` means.
    pub dev_remote: bool,
}

/// Runs the machine until the display closes, Ctrl-C arrives, or something sets the shutdown flag.
///
/// Blocks the calling thread. On a desktop that is `main`; on Android it is the thread `SDL_main`
/// was called on, which is the one SDL expects to own the window either way.
pub fn run(paths: Paths, config: Settings, options: Options) -> anyhow::Result<()> {
    // **Before anything that reads or writes the directory**, which is the whole point: a second
    // machine over one machine's state must be stopped while it has still done nothing, rather than
    // after it has opened the catalog and started answering for a queue the first one owns.
    //
    // **Bound for the whole function**, because dropping the claim releases it. `_claim` and not
    // `_`: the second spelling takes the claim and gives it straight back, which would compile and
    // do nothing.
    let _claim = crate::claim::Claim::take(&paths)?;

    // Said out loud because it changes what the machine sounds like and looks like, and because
    // nothing ever removes the folder. A developer who forgets an override bank is installed should
    // find the reason in the log rather than wondering why a release build sounds different.
    // `None` for every installed build -- see `Paths::overlay_asset_dir`.
    if let Some(overlay) = &paths.overlay_asset_dir {
        tracing::info!(
            dir = %overlay.display(),
            "using local assets from a checkout; these are not part of any release"
        );
    }

    // -- audio -----------------------------------------------------------------------------------
    // The switcher slot this machine was left on, opened instead of the resolved bank so that a
    // bank nobody is going to hear is not parsed first. `None` for every machine that has never
    // configured `debug.soundfonts`, which is every shipped one.
    let restored = config.restored_soundfont_slot();
    if let Some((slot, bank)) = restored {
        tracing::info!(
            slot,
            bank = %bank.name,
            "starting on the SoundFont switcher slot this machine was left on"
        );
    }
    // **A streaming run opens no device**, so it takes the engine that renders instead. The device
    // is what paces a television's machine; here the stream loop does, which is what lets one frame
    // be drawn for every fixed number of samples.
    let (audio, streamed_audio) = start_audio(
        &config.audio,
        &paths,
        restored.map(|(_, bank)| bank),
        &options,
        config.stream.sample_rate,
    );

    // **Nothing is written down here, and that is a decision.** Copying whatever `Engine::start`
    // resolved into `audio.output_device` and saving it — so that the Linux USB preference "stops
    // being a guess" — has it backwards. Writing it down pins one concrete identifier —
    // `alsa:plughw:CARD=Device,DEV=0` — so a machine that merely *preferred* USB would instead
    // *name* a device, which is the thing the preference exists to avoid on a box whose card order
    // moves between boots. Worse, with the interface absent on the one boot that counts, what gets
    // written is the `"system"` sentinel, which means *chosen deliberately* and so switches the
    // preference off for good on exactly the machines that need it.
    //
    // **`config` is bound immutably, and that is the guard.** There is no test that can watch this
    // spot — `run` needs a display, an audio device and a catalog — so what stops the write-back
    // coming back is that putting it back does not compile without also re-adding `mut` to the
    // signature above, which is a visible change in a diff rather than four quiet lines here.
    //
    // So `km_audio::device::decide` simply runs every start: a preference, applied afresh, never
    // recorded. An interface unplugged for an evening is preferred again the moment it returns, and
    // `audio.output_device` holds only what a person actually chose — through `PUT /audio/output`
    // or by hand. See `Choosing the audio output device` in docs/decisions/audio.md.

    // Bound once rather than asked four times: the bank can be changed while the machine runs now,
    // so four calls could in principle describe two different banks in one log line.
    let sound = audio.sound();
    match &sound {
        crate::engine::Sound::SoundFont { defects, .. } if defects.is_empty() => {
            // The rate is what the device said when it was asked, not what a stream negotiated:
            // none is open yet, and none will be until there is a song to play.
            tracing::info!(sample_rate = audio.sample_rate(), "{}", sound.describe())
        }
        // A bank that loaded with records missing is on the same footing as the other two: the
        // machine works, and something the owner would want to know is true anyway. It plays --
        // refusing it would throw away the whole point of loading it leniently -- but an instrument
        // that never sounds is not something to find out from a song.
        crate::engine::Sound::SoundFont { .. } => {
            tracing::warn!(sample_rate = audio.sample_rate(), "{}", sound.describe())
        }
        // Both of the other two are things the owner needs to know about, so they are warnings.
        _ => tracing::warn!("{}", sound.describe()),
    }

    // Said at startup rather than only when a video song is reached, so an owner whose catalog
    // holds video has been told before the queue skips one.
    if crate::video::AVAILABLE {
        tracing::info!("video songs can be played by this build");
    } else {
        tracing::info!(
            "video songs will be catalogd and searchable, but this build cannot play them"
        );
    }

    // -- the machine -----------------------------------------------------------------------------
    // One event channel, shared: the API publishes what requests caused, the machine publishes what
    // nothing asked for — a song ending, the next one starting.
    let events = Events::new();
    let dev_remote = dev_remote_dir(&paths);
    let serve_remote = config.serve_remote();
    let mut api_config = config.api_config(dev_remote.clone());
    // `--api-bind`, applied to the configuration and never to `config`, so nothing reaches
    // settings.json. `config` is saved twice below — once if this run chose an audio device, and
    // once at shutdown — and either would otherwise make a one-off override permanent.
    if let Some(bind) = options.api_bind {
        tracing::info!(%bind, from_settings = %api_config.bind, "binding where the command line says");
        api_config.bind = bind;
    }
    // `--dev-remote`, applied the same way and for the same reason. Or-ed rather than assigned: the
    // flag can only turn the console on, so a machine whose settings already say `true` is not
    // turned off by a run that did not think to ask for it.
    //
    // **It turns debugging on for this run as well, and that is deliberate rather than convenient.**
    // The console is served only when both switches are on — see `km_api::routes::dev_console_served`
    // — and there is no `--debug` flag to pair this with, so a flag that set only its own half would
    // be a flag that silently did nothing on the machines it exists for. Neither half is written
    // down; both die with the process.
    if options.dev_remote && !km_api::routes::dev_console_served(&api_config) {
        tracing::info!(
            "serving the development console at /dev/ because the command line asked, with \
             debugging mode on for this run to go with it"
        );
        api_config.serve_dev_remote = true;
        api_config.debug_enabled = true;
    }
    let bind_port = api_config.bind.port();

    let machine = Arc::new(Machine::new(paths, config, audio, events.clone())?);
    machine.install_startup_packages();
    // The tick box a setup program was given, carried out once. After the packages rather than
    // before, because installing them is what the machine is *for* and a 261.9 MiB download must not
    // stand in front of a catalog appearing — this returns as soon as the download has started,
    // and the poll loop follows it from there.
    machine.start_first_run_soundfont();
    // Decided here as well as below, because the message about an empty catalog offers a route
    // that only exists when there is a window to drag onto — and a headless machine told to drag
    // something onto a window it does not have is worse than one that never mentioned it.
    // **A run that streams has no window**, whatever `display.enabled` says. The two are not
    // alternatives an owner chooses between in settings: streaming is what this *process* was asked
    // to do, so it overrides a setting exactly as `--headless` does and writes nothing back.
    let windowed = machine.settings().display.enabled && !options.headless && !options.stream;
    if display::catalog_is_empty(&machine) {
        // Every folder that is actually scanned, because naming only the private one on Android
        // would point somebody at the directory they cannot put anything into.
        let dirs = machine.paths().packages_dirs();
        let named = dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        // The shortest route first, where there is one. It is the only one of the four that needs
        // neither a restart nor a path typed anywhere, so somebody reading this because they have
        // just built their first package should meet it before the folder.
        let dragging = if windowed && display::DRAG_AND_DROP {
            "drag it onto this window to install it, or "
        } else {
            ""
        };
        tracing::warn!(
            dirs = %named,
            "the catalog is empty — build a package with `km-pack build`, then {dragging}drop \
             the .kmpkg into the packages folder and restart. It can also be installed without a \
             restart from the Songs tab at /admin/, or with POST /api/v1/admin/packages"
        );
    }

    let api = ApiState::from_machine_with_events(Arc::clone(&machine), api_config, events);

    // **The machine is told how to ask for the port back**, which it cannot be handed any earlier:
    // the state is built around the machine, so this is the first line at which both exist. A
    // platform that suspends an application destroys the listening socket while it is away, and the
    // machine coming back to the screen is the only signal that it should be taken again. See
    // [`km_api::listener`].
    machine.set_relisten(api.relisten());

    // -- the one way out -------------------------------------------------------------------------
    // Declared here rather than beside the watchdog that was its first reader, because the power
    // controls below hand it to the API and the router is built before that thread is spawned. It
    // is still the *only* stop signal: the display loop, the watchdog and now a restart request all
    // set this one flag, so there is one exit and not three.
    let shutdown = Arc::new(AtomicBool::new(false));

    // -- power controls --------------------------------------------------------------------------
    // `None` on a desktop, on Windows, on macOS and on Android — and then the three power routes are
    // not mounted at all, rather than mounted and refusing. Installed before the router is built,
    // because that is when it is read.
    if let Some(power) = power::detect(Arc::clone(&shutdown)) {
        api.set_power(power);
    }

    // -- the machine's own log -------------------------------------------------------------------
    // Whatever set the subscriber up left the tap here, because the two callers that do it have
    // nothing in common to pass it through: `cli::main` has a command line and `SDL_main` has none.
    // `None` only where neither ran, which is a test or an example.
    //
    // Installed before the router is built, for the same reason the power controls are: that is
    // when it is read, and a tap arriving later would leave a machine keeping a log with no route
    // saying so.
    if let Some(tap) = km_logtap::installed() {
        api.set_log_tap(tap);
    }

    // -- the API ---------------------------------------------------------------------------------
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("km-api")
        .build()?;

    // -- stopping ---------------------------------------------------------------------------------
    // **The machine hears a stop itself, rather than being told about one by SDL.**
    //
    // `wait_for_signal` below is the whole of the program's own signal handling and it is reached
    // only when there is no display — headless, or a screen that refused to start. On the appliance
    // there is always a display, so on the one box where `systemctl stop` is the *only* way the
    // process is ever asked to stop, nothing in this program was listening for it. What ended it was
    // SDL, which installs handlers for SIGINT and SIGTERM of its own accord and turns them into a
    // quit event; the loop reads that event and breaks. Working by the courtesy of a library that
    // documents a hint to switch it off (`SDL_HINT_NO_SIGNAL_HANDLERS`) is not the same as working.
    //
    // **And it only takes effect when the loop next polls for events**, so a frame that blocks —
    // a long seek, an audio device that will not write — is a stop nobody hears until systemd's
    // `TimeoutStopSec` runs out and the process is killed where it stands, with `machine.persist()`
    // never reached and whatever the owner set from their phone lost.
    //
    // Setting the flag every other stop already uses means one answer for every route: the display
    // loop tests it once a frame, `wait_for_signal_for` selects on it, and the power controls in
    // `power.rs` set the same one.
    {
        let shutdown = Arc::clone(&shutdown);
        runtime.spawn(async move {
            let interrupt = tokio::signal::ctrl_c();
            let terminate = terminate_requested();
            tokio::pin!(interrupt, terminate);
            tokio::select! {
                _ = &mut interrupt => {}
                _ = &mut terminate => {}
            }
            tracing::info!("asked to stop");
            shutdown.store(true, Ordering::Release);
        });
    }

    // -- the singer's remote ---------------------------------------------------------------------
    // Built before the server binds, because the router has to be handed to `bind_with`, and its
    // pump has to be spawned on the runtime that will serve the pages it feeds.
    let remote = serve_remote.then(|| remote::build(api.clone()));
    let remote_router = remote.as_ref().map(|(_, router)| router.clone());
    if let Some((state, _)) = &remote {
        let state = state.clone();
        runtime.spawn(async move {
            km_remote_pages::handlers::spawn_pump(state);
        });
    }

    // -- the owner's page ------------------------------------------------------------------------
    // Unconditional, where the remote above is `api.serve_remote`'s to withhold. There is no setting
    // for this and deliberately so: the machine's own configuration surface is not something an
    // owner should be able to turn off and then need it. It is gated by the access list like
    // everything else, which is the control that actually means something.
    let admin_router = admin::build(api.clone());

    let (stop_api, api_stopped) = tokio::sync::oneshot::channel::<()>();
    let extras = km_api::Extras {
        remote: remote_router,
        admin: Some(admin_router),
        // **Mounted for the run that streams and absent otherwise.** A machine drawing on a
        // television has no playlist to serve, so both paths are genuinely not there rather than
        // present and answering with an explanation — the shape `Power is a capability of the host`
        // settles for anything a host may not be able to do.
        stream: options
            .stream
            .then(|| km_api::watch::router(&machine.paths().stream_dir())),
    };
    let listening = match runtime.block_on(km_api::bind_with(api.clone(), extras)) {
        Ok(listening) => {
            report_address(&listening);
            Some(listening)
        }
        Err(error) => {
            // The single most likely startup failure, and the one the connect panel was designed
            // for: the address is already in use. The machine still plays songs, and the screen says
            // exactly why no remote can connect instead of showing a URL that will never answer.
            tracing::error!(%error, "the API could not start");
            api.set_connect_info(ConnectInfo::server_failed(error.to_string(), bind_port));
            None
        }
    };
    let serving = listening.map(|listening| {
        runtime.spawn(async move {
            if let Err(error) = listening
                .serve_with_shutdown(async move {
                    let _ = api_stopped.await;
                })
                .await
            {
                tracing::error!(%error, "the API stopped unexpectedly");
            }
        })
    });

    // -- the debug single-file path --------------------------------------------------------------
    if let Some(file) = &options.play {
        // Straight to `play_path`, not through the API's `play_file`: the allowed-roots check guards
        // the network endpoint, and somebody at the keyboard already has the disk.
        if let Err(error) = machine.play_path(file) {
            tracing::error!(path = %file.display(), %error, "could not play the file");
        }
    }

    // -- the watchdog ----------------------------------------------------------------------------
    // `shutdown` is declared above, beside the power controls that also hold it.
    // **The channel is how the shutdown waits for this thread, rather than `JoinHandle::join`.**
    // `join` has no timed form, and one pass of this loop is not bounded by `POLL_INTERVAL`: `poll`
    // reaches `advance`, which opens the next song's file and builds a decoder for it. A video on a
    // spinning disk is seconds of that. A shutdown that joined would wait all of it, and a `poll`
    // that never returned would hold the process until `TimeoutStopSec` ran out and systemd killed
    // it. The send is the thread's last act, so receiving it means the loop is done.
    let (stopped_tx, stopped) = std::sync::mpsc::channel::<()>();
    let watchdog = {
        let machine = Arc::clone(&machine);
        let shutdown = Arc::clone(&shutdown);
        std::thread::Builder::new()
            .name("km-poll".to_owned())
            .spawn(move || {
                while !shutdown.load(Ordering::Acquire) {
                    machine.poll();
                    std::thread::sleep(POLL_INTERVAL);
                }
                // Nothing is listening on a run that is not shutting down, and a closed receiver is
                // not a fault here.
                let _ = stopped_tx.send(());
            })?
    };

    // -- the display, or waiting ------------------------------------------------------------------
    let settings = machine.settings();
    if windowed {
        let display_config = display::DisplayConfig {
            // The flag if one was given, settings otherwise.
            fullscreen: options.fullscreen.unwrap_or(settings.display.fullscreen),
            // **And the window's state at close goes back to settings, unless a flag chose it.**
            // `F` is a thing somebody changes at the machine and could not keep; a flag is one
            // process by decision and must not rewrite an appliance's settings from a debugging run.
            // So the presence of the override is exactly the question, and `is_none` is the answer.
            remember_fullscreen: options.fullscreen.is_none(),
            // Settings alone. There is no flag over this one, so there is no override to consult
            // and no run that must write nothing back.
            always_on_top: settings.display.always_on_top,
            window: settings.display.window_rect(),
            font: settings.display.font.clone(),
            bundled_font: Some(machine.paths().asset(crate::settings::FONT_SUBPATH)),
            // No bundled counterpart: nothing ships a CJK font, and the platforms whose own fonts
            // cover it are Windows, macOS and Android. See `A font, in the tarball only`.
            font_cjk: settings.display.font_cjk.clone(),
            keypad: settings.display.keypad,
            number_pad: settings.display.number_pad,
            wallpaper: settings.wallpaper_config(machine.paths()),
            // Resolved here rather than in `main.rs` so that the two ways in meet in one place.
            // The environment variable is the only one Android has -- `SDL_main` calls `run` with
            // `Options::default()` and never sees an argument vector.
            frame_stats: options.frame_stats
                || std::env::var_os("KM_FRAME_STATS").is_some_and(|value| value != "0"),
            // The same value the router was built from, a few lines up. Two readers of one setting
            // rather than two settings: a display that thought there was a remote when there was
            // not would open the API's landing page and call it the remote.
            remote_served: serve_remote,
        };
        // **The machine keeps looking for a screen for as long as it runs.**
        //
        // It used to try exactly once, and a failure meant headless for the life of the process —
        // which on the appliance meant a black television reporting `active (running)` after every
        // power cut, until somebody noticed and restarted the service. `systemd` cannot rescue that
        // either: the process does not exit, so `Restart=` has nothing to act on.
        //
        // Two things arrive after startup and neither can be waited for beforehand: the graphics
        // stack finishing its connector probe (about 1.1 s after the device node appears, measured),
        // and somebody switching the television on at nine in the evening — which drops and restores
        // HDMI hotplug detect, so at boot there was genuinely no display and timing out was correct.
        // `linux/wait-for-drm.sh` handles the first and cannot handle the second. This handles both.
        // **Said here, one line before the display, and the position is the whole point.**
        //
        // Everything that loads is loaded: the audio thread and its bank, the catalog, the packages
        // folder and the API. What has *not* happened is the screen, and it deliberately is not
        // waited for — see the paragraph above for why a television can arrive minutes late.
        //
        // On the appliance this is what releases the boot splash. Plymouth holds DRM master while it
        // draws, so it has to go before the loop below can take the screen; Debian quits it at
        // `multi-user.target`, which measured four seconds before the machine had a first frame --
        // four seconds of black on a set that had just been showing the mark. `plymouth-quit.service`
        // is ordered after this service instead, and `Type=notify` is what makes "after this
        // service" mean *after the machine has loaded* rather than after the process forked.
        //
        // A no-op everywhere else: no `NOTIFY_SOCKET`, no datagram. See `notify.rs`.
        crate::notify::ready();

        let started_waiting = std::time::Instant::now();
        let mut said_it_once = false;
        loop {
            match display::run(
                Arc::clone(&machine),
                api.clone(),
                display_config.clone(),
                Arc::clone(&shutdown),
            ) {
                // The display ran and somebody closed it. An ordinary stop.
                Ok(()) => break,
                // Nothing that waiting will mend: no usable font, SDL_ttf missing, a renderer that
                // would not create. A machine with no screen is still a machine — it has an API and
                // a queue — so this still falls back to headless rather than exiting, exactly as it
                // always did, and the reason is logged rather than swallowed.
                Err(display::DisplayError::Failed(error)) => {
                    tracing::error!(%error, "the display could not start; continuing without one");
                    wait_for_signal(&runtime, &shutdown);
                    break;
                }
                // No screen *yet*. Everything else — the API, the queue, the catalog, the audio —
                // started above this and is already running, so the machine goes on being a machine
                // and keeps looking.
                Err(display::DisplayError::Unavailable(reason)) => {
                    if said_it_once {
                        tracing::debug!(%reason, "still no display");
                    } else {
                        // Exactly one line, and then silence. A box that genuinely has no screen
                        // must not write a journal entry every fifteen seconds for a week.
                        tracing::warn!(
                            %reason,
                            "no display yet — the machine is running and will take the screen when \
                             one appears"
                        );
                        said_it_once = true;
                    }
                    // Three rungs, and the top one exists because of a race measured on the
                    // appliance rather than because three felt tidier: the boot splash lets go of
                    // the screen a millisecond after the machine asks for it, and on two rungs that
                    // millisecond cost two seconds of black television.
                    let waited = started_waiting.elapsed();
                    let interval = if waited < DISPLAY_RETRY_HANDOVER_FOR {
                        DISPLAY_RETRY_HANDOVER
                    } else if waited < DISPLAY_RETRY_EARLY_FOR {
                        DISPLAY_RETRY_EARLY
                    } else {
                        DISPLAY_RETRY_LATE
                    };
                    if wait_for_signal_for(&runtime, &shutdown, Some(interval)) == Woke::Stop {
                        break;
                    }
                }
            }
        }
    } else if options.stream {
        // The same line, in the same position and for the same reason the windowed arm gives: what
        // has loaded has loaded, and the encoder is what has not. A supervised run is told the
        // machine is up before the first segment exists, because a client that arrives early
        // retries and a service manager that waits does not.
        crate::notify::ready();
        // **Taken rather than borrowed**, because a renderer is the one thing here that cannot be
        // shared: it owns the player, and two of anything driving one player would interleave
        // blocks. `None` is unreachable — `start_audio` makes one whenever this arm is taken — and
        // is reported rather than unwrapped, since the cost of being wrong is a machine that plays
        // nothing with no line saying why.
        match streamed_audio
            .ok_or_else(|| anyhow::anyhow!("a streaming run started with no renderer"))
            .and_then(|audio| stream_run(&machine, &api, &settings, audio, &shutdown))
        {
            Ok(()) => {}
            // A machine that cannot stream has no output at all, unlike one that cannot find a
            // screen — there is no room to walk into and no television to notice. So this reports
            // and stops rather than falling back to headless and looking like it is working.
            Err(error) => tracing::error!(%error, "the stream could not run"),
        }
    } else {
        tracing::info!("running headless; press Ctrl-C to stop");
        wait_for_signal(&runtime, &shutdown);
    }

    // -- shutdown --------------------------------------------------------------------------------
    tracing::info!("shutting down");
    shutdown.store(true, Ordering::Release);
    let _ = stop_api.send(());
    if let Some(serving) = serving {
        // Bounded: a client holding a WebSocket open must not stop the machine from exiting.
        let _ = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(3), serving).await
        });
    }
    // **First, and before anything that waits on another thread.** This is the whole point of
    // handling a stop at all — what the owner set from their phone, written down. Nothing here
    // depends on the watchdog having finished: that thread moves the queue and the deck, and the
    // settings this writes are changed from the API and the display.
    machine.persist();

    // **Bounded, and what the bound protects is the process exiting at all.** `poll` reaches
    // `advance`, so one pass can be a video being opened; a pass that never returns would otherwise
    // hold the shutdown until systemd's `TimeoutStopSec` ran out and killed the process.
    if watchdog_finished(&stopped, WATCHDOG_STOP_BUDGET) {
        // Already finished — the send is its last act — so this returns at once and is here for the
        // thread to be reaped rather than for the wait.
        let _ = watchdog.join();
        // After the watchdog has stopped, so nothing can arm a retry behind this. Shutdown never
        // goes idle, so the song still playing is still holding its file: on Linux and Android the
        // unlink succeeds anyway and the space comes back now, and on Windows it refuses and the
        // purge in `Machine::new` gets it at the next start, exactly as it does after a kill.
        machine.clear_auditions();
    } else {
        // **Skipped rather than done anyway, because its one precondition is not met.** Clearing
        // while the watchdog still runs is the retry-armed-behind-it race the arm above exists to
        // avoid, and the cost of skipping is already written down: `Machine::new` purges at the next
        // start, exactly as it does after a kill. Dropping the handle detaches the thread, which is
        // what a process about to exit wants.
        drop(watchdog);
        tracing::warn!(
            budget_ms = WATCHDOG_STOP_BUDGET.as_millis(),
            "the poll thread did not stop in time; staged auditions are left for the next start"
        );
    }
    // Dropping the runtime here, rather than letting it fall out of scope mid-shutdown, so the
    // audio thread's own drop runs last and the device is released cleanly.
    drop(runtime);
    Ok(())
}

/// Whether the poll thread finished within `budget`.
///
/// **A named function for three characters of polarity.** `true` is what permits
/// `Machine::clear_auditions`, and the whole reason that call waits is that clearing while the poll
/// thread still runs lets it arm a retry behind the clear. An inverted answer would therefore do the
/// unsafe thing in exactly the case the wait exists for, and would look right in passing.
fn watchdog_finished(stopped: &std::sync::mpsc::Receiver<()>, budget: Duration) -> bool {
    stopped.recv_timeout(budget).is_ok()
}

/// Blocks until the process is asked to stop, or until something else sets the shutdown flag.
///
/// **Two signals, because two different things stop this process.** Ctrl-C is how a person stops it
/// at a terminal. **SIGTERM is how systemd stops it, and on the appliance that is the only way it is
/// ever stopped** — `systemctl restart`, `systemctl stop` and every reboot send it and nothing else.
/// A process that handles only the first takes the default disposition for the second: it dies where
/// it stands and the `machine.persist()` below never runs, so nothing the owner set from their phone
/// survives a restart. Measured on the appliance — transpose set to 3 through the API,
/// `systemctl restart`, `settings.json` still reading 0.
///
/// **This is the path with no display**, headless or a screen that refused to start. A machine that
/// has one never reaches here, and the task spawned beside the runtime is what hears a stop for it.
/// Both set the same flag, so either way there is one answer.
///
/// Both futures are created once, before the loop, and polled by reference. Tokio's signal handler
/// is process-wide and permanent, but a *stream* that does not exist at the moment a signal arrives
/// never learns about it — so building these afresh on each pass would leave a gap the width of the
/// poll interval in which a signal is installed, delivered to tokio and dropped on the floor.
/// Starts whichever engine this run needs.
///
/// **Two bodies behind one name**, the same arrangement [`stream_run`] uses and for the same
/// reason: a build with no encoder has nothing to render into, so it cannot have the streaming one
/// even to refuse with.
#[cfg(feature = "video")]
fn start_audio(
    audio: &crate::settings::AudioSettings,
    paths: &Paths,
    restored: Option<&crate::settings::DebugBank>,
    options: &Options,
    sample_rate: u32,
) -> (Engine, Option<crate::engine::StreamAudio>) {
    if options.stream {
        let (engine, streamed) = Engine::streaming(
            audio,
            paths,
            restored,
            sample_rate,
            km_stream::encode::CHANNELS,
        );
        (engine, Some(streamed))
    } else {
        (Engine::start(audio, paths, restored), None)
    }
}

/// The same, in a build with no encoder linked.
#[cfg(not(feature = "video"))]
fn start_audio(
    audio: &crate::settings::AudioSettings,
    paths: &Paths,
    restored: Option<&crate::settings::DebugBank>,
    options: &Options,
    _sample_rate: u32,
) -> (Engine, Option<()>) {
    // **`Some` for a streaming run, on the same terms the twin above answers `Some`.** The unit
    // stands in for a renderer this build cannot make, and answering `None` instead would take the
    // refusal out of `stream_run`'s hands: the caller reports a missing renderer, which is an
    // internal state nobody asked about, where the arm below names the encoder and the feature that
    // supplies it.
    (
        Engine::start(audio, paths, restored),
        options.stream.then_some(()),
    )
}

/// Draws the machine's screen into an encoder until something asks it to stop.
///
/// **Two bodies behind one name, and the `cfg` is the whole of the difference.** A build without
/// the `video` feature has no encoder linked, so the run stops here, naming the encoder and the
/// feature that supplies it, rather than producing a machine that runs and streams nothing.
#[cfg(feature = "video")]
fn stream_run(
    machine: &Arc<Machine>,
    api: &ApiState,
    settings: &Settings,
    audio: crate::engine::StreamAudio,
    shutdown: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let stream = &settings.stream;
    let config = crate::stream::StreamConfig {
        dir: machine.paths().stream_dir(),
        width: stream.width,
        height: stream.height,
        fps: stream.fps,
        bitrate: stream.bitrate,
        encoder: stream.encoder.clone(),
        segment_seconds: stream.segment_seconds,
        playlist_size: stream.playlist_size,
        font: settings.display.font.clone(),
        bundled_font: Some(machine.paths().asset(crate::settings::FONT_SUBPATH)),
        font_cjk: settings.display.font_cjk.clone(),
        sample_rate: stream.sample_rate,
        audio_bitrate: stream.audio_bitrate,
        wallpaper: settings.wallpaper_config(machine.paths()),
    };

    // **The stream goes to a thread of its own, whether or not there is an icon to keep it
    // company.** With the `tray` feature the main thread has to run the platform's event loop —
    // an icon can be created nowhere else — and without it the main thread simply waits. One shape
    // rather than two, so the ordinary run is the one that is exercised either way.
    let finished = Arc::new(AtomicBool::new(false));
    let thread = {
        let (machine, api, shutdown, finished) = (
            Arc::clone(machine),
            api.clone(),
            Arc::clone(shutdown),
            Arc::clone(&finished),
        );
        std::thread::Builder::new()
            .name("km-stream".to_owned())
            .spawn(move || {
                let outcome = crate::stream::run(machine, api, config, audio, shutdown);
                // **The flag and nothing else.** What went wrong is reported once, by whoever joins
                // this thread; saying it here as well would put every stream failure in the log
                // twice, which reads as two faults.
                finished.store(true, Ordering::Release);
                outcome
            })?
    };

    hold_until_stopped(api, shutdown, &finished);

    // The thread's own failure is the run's, and a panic inside it is reported rather than
    // swallowed: a machine whose whole output vanished should say so.
    match thread.join() {
        Ok(outcome) => outcome,
        Err(_) => Err(anyhow::anyhow!("the stream thread panicked")),
    }
}

/// Waits for the stream to end, with an icon in the bar if this build has one.
#[cfg(all(feature = "video", feature = "tray"))]
fn hold_until_stopped(api: &ApiState, shutdown: &Arc<AtomicBool>, finished: &Arc<AtomicBool>) {
    crate::tray::hold(api, shutdown, finished);
}

/// The same where there is no icon: wait for the stream, or for somebody to stop it.
///
/// **A poll rather than a join**, because a signal has to end the wait too, and the flag the signal
/// handler sets is the one thing both endings have in common.
#[cfg(all(feature = "video", not(feature = "tray")))]
fn hold_until_stopped(api: &ApiState, shutdown: &Arc<AtomicBool>, finished: &Arc<AtomicBool>) {
    let _ = api;
    while !shutdown.load(Ordering::Acquire) && !finished.load(Ordering::Acquire) {
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// The same, in a build with no encoder linked.
#[cfg(not(feature = "video"))]
fn stream_run(
    _machine: &Arc<Machine>,
    _api: &ApiState,
    _settings: &Settings,
    // `start_audio`'s own twin answers `Option<()>` where the video build answers
    // `Option<StreamAudio>`, so the caller hands this one a unit. Both bodies take what the one call
    // site passes, which is what keeps the `cfg` to two bodies rather than two call sites.
    _audio: (),
    _shutdown: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    Err(anyhow::anyhow!(
        "this build has no encoder, so it cannot stream. A build with the `video` feature can — \
         it is the same ffmpeg either way"
    ))
}

fn wait_for_signal(runtime: &tokio::runtime::Runtime, shutdown: &Arc<AtomicBool>) {
    let _ = wait_for_signal_for(runtime, shutdown, None);
}

/// Why a wait ended.
#[derive(Debug, PartialEq, Eq)]
enum Woke {
    /// A signal arrived, or `shutdown` was set. The machine is stopping.
    Stop,
    /// The time limit ran out and nothing has asked the machine to stop.
    Elapsed,
}

/// The same wait, bounded. `None` is the original: wait for ever.
///
/// The bounded form exists for the display retry, which has to sleep between attempts *without*
/// becoming the thing that makes Ctrl-C or `systemctl stop` take a quarter of a minute. Sleeping on
/// the same `select!` the unbounded wait already uses means a stop is heard immediately whichever
/// state the machine is in.
fn wait_for_signal_for(
    runtime: &tokio::runtime::Runtime,
    shutdown: &Arc<AtomicBool>,
    limit: Option<Duration>,
) -> Woke {
    runtime.block_on(async {
        let interrupt = tokio::signal::ctrl_c();
        let terminate = terminate_requested();
        let deadline = async {
            match limit {
                Some(limit) => tokio::time::sleep(limit).await,
                // Never resolving rather than returning at once, the same contract
                // `terminate_requested` keeps and for the same reason: a `select!` branch that
                // completed immediately would spin the loop.
                None => std::future::pending().await,
            }
        };
        tokio::pin!(interrupt, terminate, deadline);
        loop {
            tokio::select! {
                _ = &mut interrupt => return Woke::Stop,
                _ = &mut terminate => return Woke::Stop,
                _ = &mut deadline => return Woke::Elapsed,
                _ = tokio::time::sleep(POLL_INTERVAL) => {
                    if shutdown.load(Ordering::Acquire) {
                        return Woke::Stop;
                    }
                }
            }
        }
    })
}

/// Resolves when the operating system asks the process to terminate.
///
/// On Unix that is SIGTERM. On Windows there is no signal of that shape to handle — a console close
/// or a `TerminateProcess` gives no chance to run anything — so this never resolves and the branch
/// selecting on it simply never fires. Never resolving rather than returning immediately is the
/// whole contract: a `select!` branch that completes at once would spin the loop.
async fn terminate_requested() {
    #[cfg(unix)]
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut signals) => {
            signals.recv().await;
        }
        Err(error) => {
            // Not fatal, and said out loud rather than swallowed, because the consequence is
            // specific: settings stop being saved when systemd stops the machine.
            tracing::warn!(%error, "could not listen for SIGTERM; settings will not be saved on a systemd stop");
            std::future::pending::<()>().await;
        }
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await;
}

/// Prints where the machine can be reached, and says so honestly when it cannot.
fn report_address(listening: &km_api::Listening) {
    tracing::info!(addr = %listening.local_addr, "the API is listening");
    match listening.connect.primary_url() {
        Some(url) => {
            tracing::info!(%url, "remote control");
            for alternate in listening.connect.alternate_urls() {
                tracing::info!(url = %alternate, "also reachable at");
            }
        }
        None => tracing::warn!("no reachable address — a remote cannot connect"),
    }
    if !listening.connect.reachable {
        tracing::warn!(problem = ?listening.connect.problem, "remote control is unavailable");
    }
    // **Nothing is said about mDNS here, and that absence is the fix.** This runs before `serve`,
    // so it was always a snapshot of a decision taken moments earlier — and on this machine's own
    // appliance that decision was "no address yet, so no", taken about three seconds before dhcpcd
    // had a lease and never revisited. The line it printed was therefore either redundant or wrong,
    // and it was wrong on exactly the boots that mattered.
    //
    // `km_api::server::run_advertiser` announces itself instead, at the moment it actually succeeds,
    // which is a moment that no longer has anything to do with startup.
}

/// Where the development remote's files are, if they can be found.
///
/// Looked up rather than configured, because in a checkout it is at a known relative path and in an
/// installed build it is not there at all. `api.serve_dev_remote` still governs whether it is served.
///
/// The asset directory is checked first, and it is the only candidate that works on Android: the
/// other two are the repository-relative path — which resolves against `/` there — and a sibling of
/// the executable, which for a library loaded by `SDLActivity` is somewhere in `/system/bin`. Without
/// it the machine logs "enabled but no directory is configured" on every device start, which is
/// exactly the sort of thing that reads as a bug in the API rather than a missing folder.
fn dev_remote_dir(paths: &Paths) -> Option<PathBuf> {
    let candidates = [
        paths.asset("remote-dev"),
        PathBuf::from("tools/dev/remote"),
        // Beside the executable, for a build that ships it deliberately.
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("remote-dev")))
            .unwrap_or_default(),
    ];
    candidates
        .into_iter()
        .find(|dir| dir.join("index.html").is_file())
}

// -- Android and iOS ----------------------------------------------------------------------------

/// Everything a mobile entry point does, on the thread SDL intends to own.
///
/// **One body for both mobile platforms, because SDL asks the same thing of each.** What differs is
/// only who calls it: on Android `SDLActivity` loads this library and calls the `SDL_main` below, and
/// on iOS `main.m` hands `SDL_RunApp` the one `km-machine-ios` exports. Either way this must block
/// for the life of the application exactly as `main` does on a desktop.
///
/// **The C symbol is not here on iOS, and that is a linking matter rather than a preference.** A
/// `no_mangle` symbol defined in an upstream rlib is not guaranteed to reach a `staticlib` that
/// never references it, and the failure is an undefined `_SDL_main` at the point Xcode links the
/// app. So the shim crate defines its own and calls this, which is a symbol it cannot drop.
///
/// There is no command line, so [`Options`] stays at its defaults: no `--play`, and windowed rather
/// than headless. Everything else comes from the same `settings.json`, in the private directory
/// [`settings::Paths::discover`] resolves — through SDL on Android, and from the pair Swift published
/// on iOS.
///
/// **What the two platforms do not share is the two steps below the subscriber.** Android publishes a
/// JavaVM cpal expects somebody else to have published, and unpacks assets that are not files. A
/// bundle needs neither: cpal's CoreAudio backend asks for no context, and an `.app` is an ordinary
/// filesystem.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn run_on_phone() -> std::ffi::c_int {
    install_logging();
    // Before `run`, because the audio thread it starts is the first thing to ask for this.
    #[cfg(target_os = "android")]
    androidctx::publish();

    let paths = Paths::discover();
    tracing::info!(
        settings = %paths.settings_file().display(),
        assets = %paths.asset_dir.display(),
        packages = ?paths.extra_data_dir,
        "starting on a phone"
    );
    // Before `run`, because everything it starts expects the assets to be files already: the engine
    // opens the SoundFont, the display opens the font and the wallpapers, and the API serves the dev
    // remote out of the same directory. A bundle needs none of it: the tree is already files.
    #[cfg(target_os = "android")]
    androidassets::unpack(&paths);
    let (settings, _) = Settings::load(&paths);
    match run(paths, settings, Options::default()) {
        Ok(()) => {
            // Said, because a clean return here ends the app — and on a device that looks
            // identical to a crash: the window simply vanishes.
            tracing::info!("the machine stopped normally");
            0
        }
        Err(error) => {
            // Nothing above this frame will print it: returning non-zero to SDL is the only other
            // signal, and it is not one anybody reads.
            tracing::error!(%error, "the machine stopped with an error");
            1
        }
    }
}

/// Installs the subscriber, the tap and the panic hook, once.
///
/// **Idempotent, because two callers reach it on iOS and the second `init()` would panic.**
/// `km_machine_configure` calls it so that a complaint about the container's directories has
/// somewhere to be read, and [`run_on_phone`] calls it because on Android nothing else does. A
/// `Once` is what lets either order work.
///
/// Public for the same reason [`ioscfg`] is: the iOS shell is a separate crate.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn install_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(install_logging_now);
}

/// The body of [`install_logging`], run under its `Once`.
#[cfg(any(target_os = "android", target_os = "ios"))]
fn install_logging_now() {
    // This has to happen here, and it is not the same call `main.rs` makes. Without a subscriber
    // every `tracing` event is dropped on the floor, and the default one writes to a stdout that
    // Android has pointed at /dev/null — either way the device fails in complete silence. See
    // `km-androidlog`, which is shared with the offline remote's Android shell and takes the tag
    // because that is the only thing the two applications differ in.
    //
    // **A `registry` and not the `fmt()` builder, because there are two layers here.** That builder
    // describes one destination and has nowhere to put a second; logcat is still the first, and the
    // tap beside it is what lets a phone on the same network read this device's log instead of
    // somebody finding a cable and `adb`. Which is the host where that is worth the most: a
    // television has no console at all.
    //
    // `km_app`, not `karaokemachine`. The package is named `karaokemachine` but the library is
    // `km_app`, and a tracing target is the module path — which is rooted at the *library* name.
    // `karaokemachine=debug` therefore matched nothing at all, and every `tracing::debug!` in this
    // crate was discarded on both platforms. It cost a round of Android debugging to notice, because
    // the symptom is silence rather than an error.
    //
    // `info` and not `info,km_app=debug`: the debug stream is a developer's, and there is no `-v` to
    // ask for it here, so `RUST_LOG` is how it is turned on. See `Cli::log_filter` in `cli.rs` for
    // the ladder the desktop offers instead.
    // Imported here rather than at the top of the file: this is the only Android-only code that
    // needs them, and a module-level `use` would be an unused import on every other platform.
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    let tap = km_logtap::LogTap::new().with_filter(filter.clone());
    let registry = tracing_subscriber::registry().with(tracing_subscriber::EnvFilter::new(filter));
    // **The destination is the one thing the two platforms genuinely disagree about here.** Android
    // has pointed stdout at /dev/null, so a line only exists if logcat is written to; iOS leaves
    // stderr working and Xcode's console reads it, which is why there is no `km-ioslog` beside
    // `km-androidlog` and why the offline remote's iOS shell writes to stderr too.
    #[cfg(target_os = "android")]
    registry
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(km_androidlog::Logcat::new(c"karaokemachine"))
                // No terminal on the other end; escape codes would just be noise in logcat.
                .with_ansi(false)
                // Nor a clock worth printing: logcat stamps every line already.
                .without_time(),
        )
        .with(tap.layer())
        .init();
    #[cfg(target_os = "ios")]
    registry
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                // Xcode's console renders escape codes literally. The timestamp is kept, unlike
                // logcat's, because nothing else on this path stamps a line.
                .with_ansi(false),
        )
        .with(tap.layer())
        .init();
    km_logtap::install(tap);
    // After the subscriber, because the hook reports through it — and so the panic it reports is
    // taken by the tap as well, which is the one line somebody reading a device's log most wants.
    #[cfg(target_os = "android")]
    km_androidlog::install_panic_logger();
    #[cfg(target_os = "ios")]
    ioscfg::install_panic_logger();
}

/// The entry point Android calls.
///
/// `SDLActivity` loads this library and calls this symbol on the thread it intends SDL to own. The
/// body is [`run_on_phone`], shared with the iOS shell; nothing platform-specific belongs here.
///
/// **iOS has no counterpart in this crate**, for the linking reason [`run_on_phone`] gives:
/// `km-machine-ios` defines its own so that the symbol is one the staticlib cannot drop.
#[cfg(target_os = "android")]
#[expect(
    unsafe_code,
    reason = "exporting a C symbol for SDLActivity to call is the only way in on Android; the \
              signature is SDL's and the body touches no raw pointers"
)]
#[unsafe(no_mangle)]
pub extern "C" fn SDL_main(
    _argc: std::ffi::c_int,
    _argv: *mut *mut std::ffi::c_char,
) -> std::ffi::c_int {
    run_on_phone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The poll thread's send is what permits the audition clear, and its absence is what forbids it.
    ///
    /// **Both directions, because only the polarity can be wrong here.** The wait itself is
    /// `recv_timeout`; what a change could invert is which answer means "the thread has finished",
    /// and an inverted one would clear staged auditions in precisely the case the wait exists to
    /// prevent — while the poll thread is still running and can arm a retry behind the clear.
    #[test]
    fn a_finished_poll_thread_is_told_apart_from_one_that_ran_on() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        tx.send(()).expect("the thread's last act");
        assert!(
            watchdog_finished(&rx, Duration::from_secs(5)),
            "a thread that sent has finished"
        );

        let (_held, rx) = std::sync::mpsc::channel::<()>();
        let waited = std::time::Instant::now();
        assert!(
            !watchdog_finished(&rx, Duration::from_millis(50)),
            "a thread that never sends has not finished"
        );
        assert!(
            waited.elapsed() < Duration::from_secs(1),
            "and the wait is bounded by the budget rather than by the thread"
        );
    }

    /// A sender dropped without sending is a thread gone without saying so, and is not finished.
    ///
    /// `recv_timeout` answers `Disconnected` here rather than `Timeout`, and both are the same
    /// answer to the only question being asked. Worth pinning because they are different variants
    /// and a match written on one of them would let this case through.
    #[test]
    fn a_poll_thread_that_vanished_is_not_treated_as_finished() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        drop(tx);
        assert!(!watchdog_finished(&rx, Duration::from_secs(5)));
    }
}
