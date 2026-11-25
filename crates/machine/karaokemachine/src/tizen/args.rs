//! What `km_probe_run`'s argument string says: `key=value,...`.
//!
//! Plain parsing with no FFI in it, so it is compiled and tested on every host, not only in the
//! Tizen image (see `lib.rs`).
//!
//! | Key | Meaning | Default |
//! |---|---|---|
//! | `data` | the application's writable data folder | none; required |
//! | `port` | the API's port, bound on `0.0.0.0` | 8277, because the probe app holds 8177 |
//! | `log` | a tracing filter, `;` standing for `,` | `info` |
//! | `dev` | `1` serves the development console at `/dev/` | off |
//! | `exit` | `0` leaves the application open when the machine stops, for debugging | `1` |
//! | `stats` | `1` logs frame statistics once a second, as `KM_FRAME_STATS` does elsewhere | off |
//!
//! `;` stands for `,` in `log` because the comma already separates the arguments, and a filter
//! such as `info,km_app=debug` needs one.

use std::path::PathBuf;

/// The API's port when `port=` is not given. 8177 is the probe app's own listener.
pub(crate) const DEFAULT_PORT: u16 = 8277;

/// The tracing filter when `log=` is not given.
pub(crate) const DEFAULT_LOG: &str = "info";

/// The parsed arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Args {
    pub(crate) data: Option<PathBuf>,
    pub(crate) port: u16,
    pub(crate) log: String,
    pub(crate) dev: bool,
    /// Log frame statistics once a second; the probe app has no environment to set.
    pub(crate) stats: bool,
    /// End the application when the machine stops.
    pub(crate) exit: bool,
    /// What could not be understood, said in the log once there is one.
    pub(crate) complaints: Vec<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            data: None,
            port: DEFAULT_PORT,
            log: DEFAULT_LOG.to_owned(),
            dev: false,
            stats: false,
            exit: true,
            complaints: Vec::new(),
        }
    }
}

impl Args {
    /// Parses `key=value,...`. A bad or unknown pair is kept as a complaint rather than refused:
    /// the run is one `/load`, and a typo should cost a warning, not the run.
    pub(crate) fn parse(text: &str) -> Self {
        let mut args = Self::default();
        for pair in text
            .split(',')
            .map(str::trim)
            .filter(|pair| !pair.is_empty())
        {
            let Some((key, value)) = pair.split_once('=') else {
                args.complaints
                    .push(format!("ignored `{pair}`: not key=value"));
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "data" if !value.is_empty() => args.data = Some(PathBuf::from(value)),
                "port" => match value.parse::<u16>() {
                    Ok(port) if port != 0 => args.port = port,
                    _ => args
                        .complaints
                        .push(format!("ignored port `{value}`; using {DEFAULT_PORT}")),
                },
                "log" if !value.is_empty() => args.log = value.replace(';', ","),
                "dev" => args.dev = is_on(value),
                "stats" => args.stats = is_on(value),
                "exit" => args.exit = is_on(value),
                _ => args.complaints.push(format!("ignored `{pair}`")),
            }
        }
        args
    }
}

/// `1`, `true`, `on` or `yes`; anything else is off.
fn is_on(value: &str) -> bool {
    matches!(value, "1" | "true" | "on" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_given_is_the_defaults() {
        let args = Args::parse("");
        assert_eq!(args, Args::default());
        assert_eq!(args.port, 8277);
    }

    #[test]
    fn every_key_is_read() {
        let args = Args::parse(
            "data=/opt/usr/apps/x/data/, port=8300,log=info;km_app=debug,dev=1,exit=0,stats=1",
        );
        assert_eq!(args.data, Some(PathBuf::from("/opt/usr/apps/x/data/")));
        assert_eq!(args.port, 8300);
        assert_eq!(args.log, "info,km_app=debug");
        assert!(args.dev);
        assert!(!args.exit);
        assert!(args.stats);
        assert!(args.complaints.is_empty(), "{:?}", args.complaints);
    }

    #[test]
    fn a_bad_pair_is_a_complaint_and_not_a_failure() {
        let args = Args::parse("port=99999,bogus,colour=red,data=");
        assert_eq!(args.port, DEFAULT_PORT);
        assert_eq!(args.data, None);
        assert_eq!(args.complaints.len(), 4, "{:?}", args.complaints);
    }
}
