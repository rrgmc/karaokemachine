//! The icon in the bar, for a run with no window.
//!
//! **A streaming machine is otherwise invisible.** It draws on no television and opens no window,
//! so on a desktop it is a process with no face: nothing says it is there, nothing says where to
//! watch it, and nothing but Task Manager stops it. An icon in the bar is the platform's own answer
//! to all three, and it is the same one `km-package-builder` and `km-remote` already give a server
//! that gets out of the way.
//!
//! **The two platforms wear different pictures, and the bar each hangs them in is why.** Windows
//! takes the badged tile out of the executable's own resources, because the notification area draws
//! a full-color icon beside other full-color icons. macOS takes the letters alone as a silhouette,
//! because a menu bar is a row of template images the system paints itself, and a tile among them is
//! a dark object among glyphs. Neither ever wears the plain machine's mark — this is reached only
//! from a streaming run.
//!
//! # Which thread runs what, and why round that way
//!
//! **The event loop belongs to the main thread and the stream does not.** An icon has to be created
//! on a thread running the platform's loop — `tray-icon` says so for macOS, and on Windows a menu
//! needs a message pump — whereas drawing and encoding need no particular thread at all, only to be
//! left alone. So the stream goes to a thread of its own and the loop stays here.
//!
//! # The Dock, on macOS
//!
//! **A streaming machine gives up its Dock tile the moment its icon is in the bar, and keeps it
//! when there is none.** There is no window for a tile to raise, so on a machine that has an icon
//! the tile is a door onto nothing; on one that has not, it is the only thing announcing the run and
//! the only way to reach it. Taking it away unconditionally would trade a small untidiness for a
//! process nothing announces, which is the fault the icon exists to prevent.
//!
//! **The policy is set at runtime rather than declared in a manifest.** `LSUIElement` cannot say
//! *once the icon is there*, and the machine's bundle is the one macOS reads anyway — the streaming
//! launcher hands over to it — so a key in the launcher's own manifest would be read by nothing.
//!
//! # What it offers
//!
//! The address in the tooltip, then one entry for each of the three pages a streaming machine
//! serves — *Remote*, *Watch*, *Setup* — and *Quit*. `Show` is absent rather than greyed:
//! [`km_tray::Spec::has_a_window`] is false, because there is no window to bring forward.
//!
//! **Named for the pages and not for the act**, unlike the tools' single *Open in browser*: with
//! three of them, which one you want is a real question. The addresses are built here and the menu
//! is built in `km-tray`, which is the division that keeps that crate about an icon.
//!
//! # The address is asked for again
//!
//! **What the icon names is the address a phone can route to**, not the one this box reaches itself
//! by — the three pages are for hands that are not on this machine. That address moves: Wi-Fi
//! arrives after the machine does, a lease renews, a cable comes out. So the loop asks
//! [`crate::connect::reachable_url`] again rather than holding the answer it was given at the start,
//! and the rule for when there is no such address is that function's.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use km_api::ApiState;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopWindowTargetExtMacOS as _};
use tao::platform::run_return::EventLoopExtRunReturn;

/// The product's name, as the tooltip shows it.
const TITLE: &str = "KaraokeMachine";

/// The mark, read on macOS and ignored on Windows, which has it in the executable already.
///
/// **The letters alone, as a silhouette, because a macOS menu bar draws template images.** The
/// system reads one for its alpha and paints the shape itself, so the icon is dark on a light bar,
/// light on a dark one and inverted while its menu is open — which is what every glyph beside it
/// does. The badged tile cannot do any of that: its silhouette is the tile, so it would be a solid
/// square, and drawn as it is it is a near-black object in a bar that is often near-black too.
///
/// **No badge on it, although this is only ever in the bar for a streaming run.** A badge separates
/// two things standing next to each other, and nothing else this program has ever puts an icon up
/// there. `icon/README.md` has the same argument for the favicons and the window icon.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../../../../icon/karaokemachine-bar.png");

/// Which of the executable's icons the badged tile is, for Windows, which reads a resource rather
/// than the PNG above.
///
/// **Two icons in one executable, so this is not `km_tray::DEFAULT_ICON_ORDINAL`.** `build.rs`
/// attaches the plain mark at the default ordinal — which is the one the shell draws for
/// `karaokemachine.exe` itself — and the badged one here, at the next. Changing this number means
/// changing `build.rs` with it.
const STREAM_ICON_ORDINAL: u16 = 2;

/// How often the loop looks at the two flags it is really waiting on.
///
/// **A poll rather than a proxy**, and it is the smaller arrangement: the two things this waits for
/// — the stream ending, and a signal — are already flags somebody else sets, so waking to read them
/// costs one comparison five times a second and needs no channel between the loop and either.
const LOOK: Duration = Duration::from_millis(200);

/// How often the loop asks what the machine's address is now.
///
/// **A slower beat than [`LOOK`], and each is proportional to what it waits for.** That one compares
/// two flags somebody may set at any moment; this one takes a read lock and copies a list of
/// addresses, chasing a fact `km_api::server::CONNECT_REFRESH` only moves every thirty seconds. At
/// this beat the icon is behind the machine by a couple of seconds at worst, and the wait somebody
/// actually feels is the refresher's.
const ASK_THE_ADDRESS: Duration = Duration::from_secs(2);

/// What the icon's menu produced.
#[derive(Debug)]
enum Wake {
    Tray(km_tray::Command),
}

/// The three pages the menu opens, built together from one base.
///
/// **One value rather than three bindings**, so the menu's label and what pressing an entry does
/// cannot come apart: an address that moves replaces all three at once or none of them.
#[derive(Debug, Default)]
struct Doors {
    /// The singer's remote, at the root — and the address the menu draws as its label.
    remote: Option<String>,
    /// The page the stream is on.
    watch: Option<String>,
    /// The owner's page.
    setup: Option<String>,
}

impl Doors {
    /// Builds the three from a base, or none of them where the machine has no address.
    ///
    /// Each spelling skips a redirect the router would otherwise serve: `/watch/` with the trailing
    /// slash, because `/watch` redirects onto it, and `/admin` without one, because `/admin/`
    /// redirects the other way. Both are correct either way and a browser would follow them; naming
    /// the address the machine actually answers on saves a round trip on a set that has just been
    /// switched on.
    fn from(base: Option<String>) -> Self {
        let Some(base) = base else {
            return Self::default();
        };
        Self {
            watch: Some(format!("{base}{}/", km_api::watch::WATCH_PATH)),
            setup: Some(format!("{base}{}", km_api::routes::ADMIN_PAGE_PATH)),
            remote: Some(base),
        }
    }
}

/// Holds an icon in the bar until the run ends.
///
/// Returns when *Quit* was chosen, when `shutdown` was set from anywhere else, or when `finished`
/// says the stream has stopped on its own. Setting `shutdown` is how this asks the stream to stop;
/// joining it is the caller's business.
///
/// **The API rather than a finished address**, because the address is asked for again — see the
/// module documentation for what moves it.
/// **`run_return` and never `run`.** `EventLoop::run` diverges — it ends the process where it
/// stands — and everything the machine does on the way out is after this returns: the API is asked
/// to stop, the watchdog is given its budget, and `persist` writes down what somebody changed from
/// a phone. Quitting from an icon would otherwise throw all three away, silently, and only for the
/// people who used the icon.
pub(crate) fn hold(api: &ApiState, shutdown: &Arc<AtomicBool>, finished: &Arc<AtomicBool>) {
    let mut event_loop: EventLoop<Wake> = EventLoopBuilder::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // **Built inside the loop rather than before it**, which `km-tray` asks for: on macOS the
    // earliest safe moment is the equivalent of `StartCause::Init`, and an icon made before that
    // misbehaves against a full-screen application.
    //
    // **Held rather than read**: dropping a `Tray` takes the icon back out of the bar, so what keeps
    // it there is this binding living as long as the loop does.
    //
    // Three states in one value, which is why it is an `Option` of an `Option`: not tried yet,
    // tried and there is no icon bar, and holding one. A separate `tried` flag beside it would be
    // the same three states written twice, and the pair can disagree.
    let mut tray: Option<Option<km_tray::Tray>> = None;
    let shutdown = Arc::clone(shutdown);
    let finished = Arc::clone(finished);
    let api = api.clone();
    let mut doors = Doors::from(crate::connect::reachable_url(&api.connect_info()));
    let mut asked = Instant::now();

    event_loop.run_return(move |event, target, control_flow| {
        // The Dock is macOS's alone, and so is the one thing this is for. Windows puts a windowless
        // run nowhere but the notification area already, and Linux has neither.
        #[cfg(not(target_os = "macos"))]
        let _ = target;
        *control_flow = ControlFlow::WaitUntil(Instant::now() + LOOK);

        if tray.is_none() {
            let made = build(doors.remote.clone(), &proxy);
            // **The tile goes when the icon arrives, and only then.** A streaming run has no window
            // to put in the Dock, so the tile is a door onto nothing — but it is also the whole of
            // what a run whose icon failed to build has left, and giving it up before knowing would
            // leave such a run with no face at all. So the machine asks for a tile, takes the icon,
            // and then stops being an application with a Dock tile.
            //
            // At runtime rather than through `LSUIElement`, which cannot express *once the icon is
            // there* and which this process would not be read for anyway: the launcher hands over
            // to the machine's own bundle, so the manifest macOS reads is that one's.
            #[cfg(target_os = "macos")]
            if made.is_some() {
                target.set_activation_policy_at_runtime(ActivationPolicy::Accessory);
            }
            // Said once, and it is the one thing worth knowing about the icon: Linux has none by
            // decision, and a desktop that somehow has none should not leave somebody wondering
            // whether the machine started at all.
            tracing::debug!(icon = made.is_some(), "the icon in the bar");
            tray = Some(made);
        }

        // Either flag ends the run, and the order does not matter: one is somebody asking and the
        // other is the stream having already stopped, and both mean this loop has nothing left to
        // hold an icon for.
        if shutdown.load(Ordering::Acquire) || finished.load(Ordering::Acquire) {
            *control_flow = ControlFlow::Exit;
            return;
        }

        // **A machine that has no address at this moment keeps the last one it had**, which is what
        // makes this an `if let` over the answer rather than a comparison of two `Option`s. A menu
        // entry pointing at a page that will not load says more than one pointing nowhere, and the
        // icon is the only thing on a streaming desktop saying the machine is there at all.
        if asked.elapsed() >= ASK_THE_ADDRESS {
            asked = Instant::now();
            if let Some(url) = crate::connect::reachable_url(&api.connect_info())
                && Some(&url) != doors.remote.as_ref()
            {
                tracing::debug!(%url, "the icon names a different address");
                if let Some(Some(tray)) = tray.as_ref() {
                    tray.set_url(&url);
                }
                doors = Doors::from(Some(url));
            }
        }

        if let Event::UserEvent(Wake::Tray(command)) = event {
            match command {
                km_tray::Command::OpenTheRemote => open(doors.remote.as_deref()),
                km_tray::Command::OpenTheWatchPage => open(doors.watch.as_deref()),
                km_tray::Command::OpenTheSetupPage => open(doors.setup.as_deref()),
                // Never offered: this menu names its three pages, so the tools' generic entry is
                // not in it. Spelled rather than left to a wildcard, so a fourth page is a compile
                // error here rather than a menu entry that quietly does nothing.
                km_tray::Command::OpenInBrowser => {}
                km_tray::Command::Quit => {
                    // The stream is told to stop and the loop ends; the caller joins the thread,
                    // and the ordinary shutdown after it is the same one a signal takes.
                    shutdown.store(true, Ordering::Release);
                    *control_flow = ControlFlow::Exit;
                }
                // Never offered, because `has_a_window` is false.
                km_tray::Command::Show => {}
            }
        }
    });
}

/// Opens one of the three pages, or says in the log why it could not.
///
/// **`None` is an ordinary argument rather than a mistake**: a machine whose listener never bound
/// has no address for any of them, and the menu is still there to quit from. A failure is a warning
/// and not a fault for the reason the icon itself is not one — nothing about the stream depends on a
/// browser having opened.
fn open(url: Option<&str>) {
    if let Some(url) = url
        && let Err(error) = km_osopen::open_url(url)
    {
        tracing::warn!(%error, %url, "could not open the page");
    }
}

/// Puts the icon in the bar, or explains in the log why there is none.
///
/// **A failure here is a blemish rather than a fault.** A machine with no icon still streams, still
/// answers its API and still stops on a signal; refusing to run over a missing icon bar would be
/// the tail wagging the dog. Linux reaches this and gets `None`, because `km-tray` compiles to a
/// stub there — see that crate for why its backend is one this project will not take on.
fn build(
    url: Option<String>,
    proxy: &tao::event_loop::EventLoopProxy<Wake>,
) -> Option<km_tray::Tray> {
    let spec = km_tray::Spec {
        title: TITLE,
        // A machine with no reachable address still deserves an icon; what it cannot offer is a
        // page to open, and the menu's own entry is harmless with nowhere to go.
        url: url.unwrap_or_default(),
        icon_png: TRAY_ICON_PNG,
        // The only `true` among the four specs: the mark above is a silhouette, so macOS may paint
        // it and the icon follows a light bar into a dark one.
        icon_is_template: true,
        icon_ordinal: STREAM_ICON_ORDINAL,
        // **Absent rather than greyed.** There is no window in a streaming run, and an entry that
        // is always refused is worse than one that is not there.
        has_a_window: false,
        // **Both of the other two are always mounted in the run this icon belongs to**, so neither
        // entry is conditional on anything checked here: the owner's page is served unconditionally,
        // and the stream's page is served whenever `--stream` was given — which is the only way this
        // function is reached.
        pages: km_tray::Pages::RemoteWatchAndSetup,
    };
    let proxy = proxy.clone();
    match km_tray::build(spec, move |command| {
        // Forwarded rather than acted on where it arrives: the handler runs on the platform's own
        // event thread, which is not where a browser should be started from.
        let _ = proxy.send_event(Wake::Tray(command));
    }) {
        Ok(tray) => Some(tray),
        Err(error) => {
            // **A warning rather than a debug line, and it is the only record there will be.** A
            // streaming machine has no screen and no window, so an icon that failed to build takes
            // the run's three pages and its way out with it, and leaves nothing behind saying so.
            // Linux reaches this on every run by decision, and that is the one case where the line
            // is noise; it is a platform with no bar to look at, so nobody is reading it wondering
            // where the icon went.
            tracing::warn!(%error, "no icon in the bar; the machine is streaming all the same");
            None
        }
    }
}
