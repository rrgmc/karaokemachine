//! Sending a program's log to the [ECAppLog](https://github.com/RangelReale/ecapplog) viewer, for
//! the run somebody is watching happen.
//!
//! Every program here says what it is doing through `tracing`, and three destinations already take
//! that stream: a console, a file behind `--log-file`, and the ring
//! [`km_logtap`](../km_logtap/index.html) holds for the machine's own log route. This is the fourth,
//! and it is the only one that is *live and filterable while the program runs* — a window beside the
//! program with a tab per crate, a level to colour by and a details pane, where a terminal offers a
//! scrollback and a grep.
//!
//! # It replaces the console rather than joining it
//!
//! Both at once would print every line twice, and a viewer is a better console than a console is.
//! The file and the ring are untouched, because where the detail goes and how much of it there is
//! are different questions — see `A log that goes to a viewer instead of a console` in
//! `docs/decisions/distribution.md`.
//!
//! **The verbosity ladder still decides what goes in it.** The layer this hands back sets no level
//! of its own; the registry's `EnvFilter` governs it exactly as it governs the three beside it, so
//! `-v` and `RUST_LOG` reach the viewer unchanged.
//!
//! # The viewer does not have to be there
//!
//! Entries queue while it is unreachable and are delivered as soon as it appears, so a program can
//! be started first and attached to later, and the viewer can be restarted mid-run. Past the queue's
//! capacity the oldest entry goes, which keeps the most recent context.
//!
//! ```no_run
//! use tracing_subscriber::layer::SubscriberExt as _;
//! use tracing_subscriber::util::SubscriberInitExt as _;
//!
//! let viewer = km_ecapplog::EcAppLog::open(km_ecapplog::DEFAULT_ADDRESS, "KaraokeMachine");
//! tracing_subscriber::registry()
//!     .with(tracing_subscriber::EnvFilter::new("info"))
//!     .with(viewer.layer())
//!     .init();
//! km_ecapplog::install(viewer);
//! ```

use std::sync::OnceLock;
use std::time::Duration;

use tracing_subscriber::Layer;
use tracing_subscriber::registry::LookupSpan;

/// Where the viewer listens unless a run names somewhere else.
///
/// Loopback, which is what the viewer itself opens unless its own `listen_all_interfaces` setting is
/// turned on. A run reaching a viewer on another computer says so.
pub const DEFAULT_ADDRESS: &str = ecapplog::DEFAULT_ADDRESS;

/// The environment variable that turns this on without a flag.
///
/// **`1` for the default address, an address for anywhere else, `0` or absent for no.** The `0` rule
/// is `KM_LOG_FILE`'s, and it is what lets a checkout turn this on for every program at once —
/// `.cargo/config.toml`'s `[env]` block — while one command still says no: a plain entry there does
/// not override a variable already in the environment.
pub const ENV_VAR: &str = "KM_ECAPPLOG";

/// How long a drain may take before the process goes anyway.
///
/// **A ceiling that is only ever reached by the failure**, which is what makes a second generous.
/// Draining to a connected viewer is a loopback write of a few kilobytes and finishes in
/// microseconds however long the queue is; the whole of this budget is spent only where the viewer
/// is not there, and then it is spent on every exit. A program somebody has finished with must not
/// pause on a window nobody is looking at, and a lost entry addressed to a viewer that is not
/// running is not a loss anybody can observe.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// Reads the address a run asked for, as a clap `value_parser`.
///
/// **A port is required, and that is the whole of the check.** A bare host is the mistake worth
/// catching, because the protocol has no default port a client could fill in and the viewer would
/// simply never be reached; anything past that is a question for the connection itself, which
/// retries and so can answer it better than a parser guessing at a hostname.
///
/// # Errors
///
/// When the value names no port, or names one that is not a number.
pub fn parse_address(value: &str) -> Result<String, String> {
    // From the right, because an IPv6 literal is full of colons and only the last one is the port's.
    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| format!("`{value}` names no port; write `{DEFAULT_ADDRESS}`"))?;
    if host.is_empty() {
        return Err(format!(
            "`{value}` names no host; write `{DEFAULT_ADDRESS}`"
        ));
    }
    port.parse::<u16>()
        .map_err(|_| format!("`{port}` is not a port number; write `{DEFAULT_ADDRESS}`"))?;
    Ok(value.to_owned())
}

/// Where this run's log should go, as the command line and the environment between them say.
///
/// `flag` is the program's own `--ecapplog`, and it is read first so that a command line always wins
/// over an inherited setting rather than being overridden by one. `Ok(None)` is nobody having asked,
/// which is the ordinary answer.
///
/// **A settings file is the third rung and is not here**, because only one of the four programs that
/// carry this has one. The machine asks this first and falls back to `logging.ecapplog`; see
/// `A log that goes to a viewer instead of a console` in `docs/decisions/distribution.md`.
///
/// # Errors
///
/// When the variable is set to something that is neither a switch nor an address. It is reported
/// rather than guessed at, and a run that cannot read it runs with its console instead — a typo in
/// an environment that silently became the default address would be a viewer nobody could find.
pub fn asked_for(flag: Option<&str>) -> Result<Option<String>, String> {
    if let Some(address) = flag {
        return Ok(Some(address.to_owned()));
    }
    let Some(value) = std::env::var_os(ENV_VAR) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .ok_or_else(|| format!("{ENV_VAR} is not text"))?;
    from_value(value).map_err(|reason| format!("{ENV_VAR}: {reason}"))
}

/// What one value of [`ENV_VAR`] asks for.
///
/// Split from [`asked_for`] so the rule can be tested without setting a variable the whole test
/// binary shares — which would be a race rather than a test, exactly as it is for `RUST_LOG`.
fn from_value(value: &str) -> Result<Option<String>, String> {
    match value {
        // Absent is handled by the caller; `0` is the spelling that says no where the variable has
        // to exist, which is what a checkout-wide `[env]` entry makes necessary. Empty is the same
        // answer for the same reason, being how a shell spells "set to nothing".
        "0" | "" => Ok(None),
        "1" => Ok(Some(DEFAULT_ADDRESS.to_owned())),
        address => parse_address(address).map(Some),
    }
}

/// The tab a record is filed under, for a target that is a module path.
///
/// **The crate, not the module.** A `tracing` target is the module path it was emitted from, so
/// filing by it outright opens a tab for `km_app::cli`, `km_app::settings`, `km_api::routes` and
/// several dozen more — which is a filter nobody wants at a granularity nobody reads at. The crate
/// is the unit somebody actually asks about: *what was the catalog doing while this played?*
///
/// **A record from the `log` facade is filed the same way**, by the target it named rather than by
/// the crate it passed through. A dependency like `mdns_sd` reaches `tracing` through
/// `tracing-log`'s bridge, which hands a subscriber one target of its own for every such record; the
/// layer recovers the record's own before this sees it, so `mdns_sd::service_daemon` arrives here and
/// `log` never does. The target rather than the module path beside it, because the target is what
/// `RUST_LOG` selects on — so the tab a run shows is the one the filter names.
///
/// None of the names this produces collides with the four the viewer keeps for itself — `ALL`,
/// `ERROR`, `ECAPPLOG` and `<unknown>` — because every crate here is lowercase and prefixed.
fn category(target: &str) -> String {
    target
        .split_once("::")
        .map_or(target, |(crate_name, _)| crate_name)
        .to_owned()
}

/// The viewer this run is sending its log to.
///
/// Held past the layer it builds so that an exit path can drain the queue; see [`flush`].
pub struct EcAppLog {
    client: ecapplog::Client,
    address: String,
}

impl EcAppLog {
    /// Opens the connection, under the name the viewer labels it with.
    ///
    /// **Returns without waiting and cannot fail.** Reaching the viewer is the background thread's
    /// business from here on, and a viewer that is not running yet is the ordinary case rather than
    /// an error — see the note at the top of this file.
    ///
    /// `app_name` is what the viewer writes beside the connection, so it is the program's name as a
    /// person reads it rather than as cargo spells it.
    #[must_use]
    pub fn open(address: &str, app_name: &str) -> Self {
        // No `on_error`. The failure it reports is the viewer being absent, which is a state this
        // is built to sit in, so a hook would fire on every reconnect for the whole of a run that
        // is behaving exactly as intended. And a hook here could not say so through `tracing`
        // anyway: an error reported by the log's own destination is taken by that destination,
        // which `tracing` drops silently rather than recursing -- the trap `km-logtap`'s own note
        // is written about, one layer over.
        let client = ecapplog::Client::builder()
            .app_name(app_name)
            .address(address)
            .build();
        Self {
            client,
            address: address.to_owned(),
        }
    }

    /// Where this run's log is going.
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The `tracing` layer that sends events to it.
    ///
    /// **No `max_level`**, which the layer offers and this deliberately leaves at its default. A
    /// level is the registry's `EnvFilter` to decide, and a second answer here would be a second
    /// place `-v` has to be understood.
    #[must_use]
    pub fn layer<S>(&self) -> Box<dyn Layer<S> + Send + Sync + 'static>
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        // The same client as this handle's, rather than a second connection: the viewer labels each
        // one separately, and one program showing up twice is a puzzle it costs nothing to avoid.
        ecapplog::TracingLayer::builder()
            .client(self.client.clone())
            .category_fn(|metadata| category(metadata.target()))
            .build()
            .boxed()
    }

    /// Waits for the queued entries to be written, up to [`FLUSH_TIMEOUT`].
    pub fn flush(&self) {
        self.client.flush(FLUSH_TIMEOUT);
    }
}

fn slot() -> &'static OnceLock<EcAppLog> {
    static SLOT: OnceLock<EcAppLog> = OnceLock::new();
    &SLOT
}

/// Remembers the viewer this process is feeding, so that an exit path need not be handed one.
///
/// **A process-wide slot rather than a return value**, the shape [`km_logtap::install`] already
/// takes here and for a related reason. A layer handed to a global subscriber is never dropped, so
/// the queue is drained by somebody calling [`flush`] on the way out — and the place a program ends
/// is rarely the place it built its subscriber. Threading a handle between the two through four
/// programs' startup would buy nothing a slot does not.
///
/// Returns whether it took. A second call is a program opening two viewers, which is a bug rather
/// than a race.
///
/// [`km_logtap::install`]: ../km_logtap/fn.install.html
pub fn install(viewer: EcAppLog) -> bool {
    slot().set(viewer).is_ok()
}

/// Where the installed viewer is, or `None` where no run asked for one.
///
/// What a startup banner reads to say where this run's log went. It is read there rather than
/// printed when the connection opens because the subscriber is built before a program has settled
/// whether it has a console to print on — and on Windows the answer *no* makes a `println!` a panic
/// rather than a discarded line.
#[must_use]
pub fn address() -> Option<&'static str> {
    slot().get().map(|viewer| viewer.address.as_str())
}

/// Drains whatever [`install`] was given, and does nothing where nothing was.
///
/// Called on the way out. Without it the queue dies with the process and the entries lost are the
/// last ones, which are the ones somebody reading a log is there for. Draining twice is free, so an
/// exit path that cannot tell whether another already ran should call it anyway.
pub fn flush() {
    if let Some(viewer) = slot().get() {
        viewer.flush();
    }
}

/// Drains on the way out of the scope it is bound in.
///
/// **For the ordinary exits, which are the many.** A program that has to answer `--version`, refuse
/// a bad argument and finish a run has one place all three pass through and a dozen they leave
/// from; binding this in that one place covers every `return` and every `?` at once.
///
/// **It does not cover an exit that skips destructors**, which is what the two windowed shells here
/// do: `tao`'s event loop calls `process::exit`, so those call [`flush`] by name where they already
/// do the rest of their shutdown.
#[must_use = "binding this to `_` drains immediately; bind it to a name"]
pub struct FlushOnDrop;

impl Drop for FlushOnDrop {
    fn drop(&mut self) {
        flush();
    }
}

/// Drains the installed viewer when the returned value goes out of scope.
///
/// See [`FlushOnDrop`], including what it does not reach.
#[must_use = "binding this to `_` drains immediately; bind it to a name"]
pub fn flush_on_drop() -> FlushOnDrop {
    FlushOnDrop
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_needs_a_host_and_a_port() {
        assert_eq!(
            parse_address("127.0.0.1:13991").as_deref(),
            Ok("127.0.0.1:13991")
        );
        assert_eq!(parse_address("viewer:13991").as_deref(), Ok("viewer:13991"));
        assert_eq!(parse_address("[::1]:13991").as_deref(), Ok("[::1]:13991"));
        assert!(parse_address("127.0.0.1").is_err());
        assert!(parse_address("127.0.0.1:").is_err());
        assert!(parse_address(":13991").is_err());
        assert!(parse_address("127.0.0.1:no").is_err());
        assert!(parse_address("127.0.0.1:99999").is_err());
    }

    #[test]
    fn the_default_address_parses() {
        assert!(parse_address(DEFAULT_ADDRESS).is_ok());
    }

    /// The variable says yes, no, or where.
    #[test]
    fn the_environment_switches_or_names_an_address() {
        assert_eq!(from_value("1"), Ok(Some(DEFAULT_ADDRESS.to_owned())));
        assert_eq!(from_value("0"), Ok(None));
        assert_eq!(from_value(""), Ok(None));
        assert_eq!(from_value("1.2.3.4:99"), Ok(Some("1.2.3.4:99".to_owned())));
        // Reported rather than taken for the default: a typo that quietly became `127.0.0.1:13991`
        // is a viewer somebody would look for on the machine they meant to reach.
        assert!(from_value("yes").is_err());
        assert!(from_value("1.2.3.4").is_err());
    }

    /// A command line wins over an inherited setting, and does not read one.
    ///
    /// The environment is not touched here — it is shared by every test in this binary, so setting
    /// it would be a race. This asserts the half that can be asserted: a flag short-circuits before
    /// the variable is looked at.
    #[test]
    fn a_flag_wins_over_the_environment() {
        assert_eq!(
            asked_for(Some("1.2.3.4:99")),
            Ok(Some("1.2.3.4:99".to_owned()))
        );
    }

    #[test]
    fn a_category_is_the_crate_a_target_names() {
        assert_eq!(category("km_app::cli"), "km_app");
        assert_eq!(category("km_api::routes::songs"), "km_api");
        // A target somebody wrote by hand, rather than a module path. Left alone: it is already the
        // name they chose to file it under.
        assert_eq!(category("km_app"), "km_app");
    }
}
