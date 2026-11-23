//! The exported entry points, the folders and the log.
//!
//! **Two exports and one body.** `km_probe_run` has the probe app's signature, so M2 to M4 load the
//! machine over the probe app's port the way M1's probes were loaded; its argument string carries
//! the data folder and the rest. `km_tizen_run` is what M5's own .NET host calls, with the data
//! folder as an argument of its own. Both publish the [`Host`], start the log, write the carried
//! assets and call [`crate::run`], so M5 changes only the host. When `run` returns, both end the
//! application (see [`end_application`]); `exit=0` in `km_probe_run`'s arguments leaves it open.
//!
//! **Neither lets a panic out.** Unwinding across `extern "C"` aborts the process, which on the set
//! takes the host with it and says nothing, so the body runs under `catch_unwind` as `km-machine-ios`
//! does, and the panic hook installed with the log has already said what happened.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::io::{self, Write};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::args::Args;
use super::{Host, HostLog};
use crate::Options;
use crate::settings::{Paths, Settings};

/// The filter the log starts with, from `log=`; read by [`crate::install_logging`].
static LOG_FILTER: OnceLock<String> = OnceLock::new();

/// The tracing filter this run asked for.
pub(crate) fn log_filter() -> String {
    LOG_FILTER
        .get()
        .cloned()
        .unwrap_or_else(|| super::args::DEFAULT_LOG.to_owned())
}

/// The machine, loaded by the probe app's `POST /load`.
///
/// The signature is the probe app's `RunFn`. `args` is `key=value,...`; see [`super::args`] for the
/// keys, of which `data=` is required. Returns 0 when the machine stopped cleanly, 1 otherwise.
///
/// # Safety
///
/// `args` is null or a NUL-terminated string alive for the call. `ecore_window` is the host
/// window's `Ecore_Wl2_Window *`, and this is called on the host's UI thread.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the probe app looks up; the one raw pointer read is the \
              argument string, which the host keeps alive for the call"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn km_probe_run(
    args: *const c_char,
    ecore_window: *mut c_void,
    width: c_int,
    height: c_int,
    log: Option<HostLog>,
) -> c_int {
    let text = if args.is_null() {
        String::new()
    } else {
        // SAFETY: non-null, NUL-terminated by the host, and alive until this call returns.
        unsafe { CStr::from_ptr(args) }
            .to_string_lossy()
            .into_owned()
    };
    guarded(log, || {
        let args = Args::parse(&text);
        start(ecore_window.addr(), width, height, args, log)
    })
}

/// The machine, called by M5's .NET host on its UI thread.
///
/// `data_dir` is the application's data folder (`Application.Current.DirectoryInfo.Data`). The API
/// takes its default port here, and the log `info`. Returns 0 when the machine stopped cleanly, 1
/// otherwise.
///
/// # Safety
///
/// `data_dir` is null or a NUL-terminated string alive for the call. `ecore_window` is the host
/// window's `Ecore_Wl2_Window *`, and this is called on the host's UI thread.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol the .NET host calls; the one raw pointer read is the data \
              folder, which the host keeps alive for the call"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn km_tizen_run(
    ecore_window: *mut c_void,
    width: c_int,
    height: c_int,
    data_dir: *const c_char,
    log: Option<HostLog>,
) -> c_int {
    let data = if data_dir.is_null() {
        None
    } else {
        // SAFETY: non-null, NUL-terminated by the host, and alive until this call returns.
        let text = unsafe { CStr::from_ptr(data_dir) }.to_string_lossy();
        (!text.is_empty()).then(|| PathBuf::from(text.as_ref()))
    };
    guarded(log, || {
        let args = Args {
            data,
            ..Args::default()
        };
        start(ecore_window.addr(), width, height, args, log)
    })
}

/// Runs `body`, turning a panic into 1.
fn guarded(log: Option<HostLog>, body: impl FnOnce() -> c_int) -> c_int {
    match std::panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(code) => code,
        Err(_) => {
            // The hook has logged the panic itself, if the log was up by then; this line is for a
            // panic that came before it.
            host_line(log, "the machine panicked; see the lines above");
            1
        }
    }
}

/// The body both exports share.
fn start(
    ecore_window: usize,
    width: c_int,
    height: c_int,
    args: Args,
    log: Option<HostLog>,
) -> c_int {
    let Some(data_dir) = args.data.clone() else {
        // Before the log, so said straight to the host: there is nowhere yet to keep a catalog.
        host_line(
            log,
            "km: no data folder given (data=<folder>); not starting",
        );
        return 1;
    };
    let _ = LOG_FILTER.set(args.log.clone());
    if !super::publish(Host {
        ecore_window,
        width,
        height,
        data_dir,
        log,
    }) {
        host_line(
            log,
            "km: this library already ran a machine; load a fresh copy",
        );
        return 1;
    }
    crate::install_logging();
    for complaint in &args.complaints {
        tracing::warn!("{complaint}");
    }

    let paths = Paths::discover();
    let host = super::host().expect("published above");
    tracing::info!(
        window = format!("{:#x}", host.ecore_window),
        width = host.width,
        height = host.height,
        port = args.port,
        settings = %paths.settings_file().display(),
        assets = %paths.asset_dir.display(),
        "starting on a Tizen television"
    );
    // Before `run`, because the display opens the font and the wallpapers as files.
    super::assets::unpack(&paths, carried());
    if km_api::discover::mdns_declined() {
        tracing::info!("mDNS declined by the environment");
    } else {
        // The answer comes from the API's advertiser, a few seconds in: "advertising over mDNS"
        // if the set lets an app open the multicast socket, a warning if not.
        tracing::info!("mDNS will be tried; its outcome follows from the API");
    }

    let (settings, _) = Settings::load(&paths);
    let options = Options {
        // Every interface, so a phone on the network reaches it; the port is this run's only.
        api_bind: Some(std::net::SocketAddr::from(([0, 0, 0, 0], args.port))),
        dev_remote: args.dev,
        ..Options::default()
    };
    let code = match crate::run(paths, settings, options) {
        Ok(()) => {
            // `run` says no more than this: Back on the idle screen, the set asking the app to
            // quit (Ecore's exit signal ends the display's loop), or a stop through the API.
            tracing::info!(
                "the machine stopped normally: its display closed, the set asked the app to \
                 quit, or it was told to stop"
            );
            0
        }
        Err(error) => {
            tracing::error!(
                error = format!("{error:#}"),
                "the machine stopped with an error"
            );
            1
        }
    };
    if args.exit {
        end_application();
    } else {
        tracing::info!("exit=0: leaving the application open");
    }
    code
}

/// Asks the host application to end, after the machine has.
///
/// **The host cannot end itself once the machine has run.** The display withholds Back from DALi
/// so that it reaches only the machine, and once SDL's window is gone no key reaches the app at
/// all (measured at M1), so nothing would ever close it. Both calls only ask the main loop to
/// quit; it does so after this returns to the host.
fn end_application() {
    if let Some(exit) = lookup(c"libcapi-appfw-application.so.0", c"ui_app_exit") {
        tracing::info!("ending the application with ui_app_exit");
        exit();
    } else if let Some(quit) = lookup(c"libecore.so.1", c"ecore_main_loop_quit") {
        tracing::info!("no ui_app_exit; ending the application with ecore_main_loop_quit");
        quit();
    } else {
        tracing::warn!(
            "neither ui_app_exit nor ecore_main_loop_quit was found; the application stays open"
        );
    }
}

/// A `void (void)` function from a library the process has, or can load.
#[expect(
    unsafe_code,
    reason = "dlopen and dlsym, and a found symbol taken as the `void (void)` both callers name"
)]
fn lookup(library: &CStr, symbol: &CStr) -> Option<extern "C" fn()> {
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    const RTLD_NOW: c_int = 2;
    // SAFETY: two NUL-terminated strings alive for the call. The handle is never closed: the
    // library is the host's own, already loaded, and this runs once as the process ends.
    let found = unsafe {
        let handle = dlopen(library.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return None;
        }
        dlsym(handle, symbol.as_ptr())
    };
    // SAFETY: non-null, and both symbols are `void f(void)` in their headers.
    (!found.is_null())
        .then(|| unsafe { std::mem::transmute::<*mut c_void, extern "C" fn()>(found) })
}

// `CARRIED`, written by `build.rs` from the folder `KM_TIZEN_ASSETS` names.
#[cfg(feature = "tizen-embed-assets")]
include!(concat!(env!("OUT_DIR"), "/tizen_assets.rs"));

/// The assets compiled into this library, or none.
#[cfg(feature = "tizen-embed-assets")]
fn carried() -> &'static [(&'static str, &'static [u8])] {
    CARRIED
}

/// The assets compiled into this library, or none.
#[cfg(not(feature = "tizen-embed-assets"))]
fn carried() -> &'static [(&'static str, &'static [u8])] {
    &[]
}

/// One line to the host's logger, or to stderr when it gave none.
fn host_line(log: Option<HostLog>, line: &str) {
    match log {
        Some(log) => {
            let text = c_line(line.as_bytes());
            log(text.as_ptr());
        }
        None => eprintln!("{line}"),
    }
}

/// A line as a C string: cut at a stray NUL, `\r` dropped.
fn c_line(line: &[u8]) -> std::ffi::CString {
    let end = line.iter().position(|b| *b == 0).unwrap_or(line.len());
    let trimmed: Vec<u8> = line[..end]
        .iter()
        .copied()
        .filter(|b| *b != b'\r')
        .collect();
    std::ffi::CString::new(trimmed).unwrap_or_default()
}

/// The fmt layer's writer: whole lines to the host's logger.
///
/// One per event; the formatter writes an event in several pieces, and the host's logger takes
/// one line per call.
pub(crate) struct HostLogWriter {
    log: Option<HostLog>,
    buf: Vec<u8>,
}

impl HostLogWriter {
    /// A writer over the published host's logger. The `MakeWriter` the subscriber takes.
    pub(crate) fn make() -> Self {
        Self {
            log: super::host().and_then(|host| host.log),
            buf: Vec::new(),
        }
    }

    fn drain(&mut self, all: bool) {
        while let Some(at) = self.buf.iter().position(|b| *b == b'\n') {
            let rest = self.buf.split_off(at + 1);
            let line = std::mem::replace(&mut self.buf, rest);
            host_line_bytes(self.log, &line[..at]);
        }
        if all && !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            host_line_bytes(self.log, &line);
        }
    }
}

fn host_line_bytes(log: Option<HostLog>, line: &[u8]) {
    host_line(log, &String::from_utf8_lossy(line));
}

impl Write for HostLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        self.drain(false);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain(false);
        Ok(())
    }
}

impl Drop for HostLogWriter {
    fn drop(&mut self) {
        self.drain(true);
    }
}

/// Reports a panic through `tracing`, so the host's log and the tap both have it.
///
/// Chained, as on iOS, and with a backtrace forced because nothing sets `RUST_BACKTRACE` on a set.
pub(crate) fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            panic = %info,
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "the machine panicked"
        );
        previous(info);
    }));
}
