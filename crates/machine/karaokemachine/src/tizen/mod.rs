//! The machine on a Samsung Tizen television: what the .NET host hands down, and the two halves
//! that use it.
//!
//! **The host calls in on its UI thread and the machine does not return until it ends.** Each frame
//! the machine lets Ecore do its work, which is the nested loop `docs/architecture/tizen.md` describes and
//! `docs/research/tizen.md` §3 measured. So everything the host knows is published here once, before
//! [`crate::run`] starts, the way `ioscfg` publishes an iPhone's folders before `SDL_main`.
//!
//! - [`entry`] is the exported entry point, the folders and the log.
//! - [`window`] is SDL's window inside the host's, and the per-frame Ecore pump.
//! - [`args`] and [`assets`] are `entry`'s argument string and carried assets, which have no FFI
//!   and are also built and tested on every host (see `lib.rs`).

use std::ffi::c_char;
use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) mod args;
pub(crate) mod assets;
pub mod entry;
pub(crate) mod soundfocus;
pub(crate) mod window;

/// The host's line logger: one UTF-8, NUL-terminated line per call, on any thread.
pub type HostLog = extern "C" fn(*const c_char);

/// What the host hands down, once per process.
#[derive(Debug)]
pub(crate) struct Host {
    /// The address of the `Ecore_Wl2_Window *` behind the host's NUI window, kept as an address so
    /// nothing needs `unsafe impl Send`. Only ever turned back into a pointer on the UI thread.
    pub(crate) ecore_window: usize,
    /// The host window's size in pixels, which is the screen's.
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// The application's writable data folder: settings, the catalog, the packages.
    pub(crate) data_dir: PathBuf,
    /// The read-only folder the package's assets are in (`res/assets` of the `.tpk`), when the
    /// host has one. `None` for the probe app, whose library carries its assets and writes them
    /// into `data_dir/assets`.
    pub(crate) asset_dir: Option<PathBuf>,
    /// Where the line logger writes; `None` when the host gave none.
    pub(crate) log: Option<HostLog>,
}

static HOST: OnceLock<Host> = OnceLock::new();

/// Records what the host handed down. A second call is ignored: one process runs one machine.
pub(crate) fn publish(host: Host) -> bool {
    HOST.set(host).is_ok()
}

/// What the host handed down, once [`publish`] has run.
pub(crate) fn host() -> Option<&'static Host> {
    HOST.get()
}
