//! What to do with a package the operating system handed us.
//!
//! A double-clicked `.kmpkg` arrives as an argument — `src/register.rs` is what arranges that — and
//! this decides what happens next. There are two answers and the hard part is knowing which:
//!
//! 1. **A machine is already running here.** Copy the package into the packages folder and hand the
//!    *destination* to `POST /api/v1/admin/packages` on loopback, signing itself in with the hash in
//!    the settings file it just read. The running machine installs it and
//!    flashes it on its own screen, which is the screen the person is looking at.
//! 2. **Nothing is running.** Copy it in and start normally. A document open is an application
//!    launch, and the startup scan installs whatever is in the folder.
//!
//! **Why the first case has to exist at all**: a second machine cannot have the data directory, so
//! starting one refuses at once — see `claim.rs`. Without the probe below, the person who
//! double-clicked a package would get that refusal instead of their song, having asked for nothing
//! but to add it to the machine already playing.
//!
//! **The copy happens on both paths and happens first**, through `dropped::adopt`, so there is one
//! placement policy in this crate rather than two. That matters more than it looks: `place` is what
//! decides that a package is stored under the name its manifest implies, that a file of the same id
//! is replaced, and that anything else at that name is stepped over to `-2`. A second implementation
//! here would eventually disagree with it, and disagreeing means destroying somebody's only copy of
//! a package.
//!
//! **And the copy is what makes case 1 legal.** `POST /api/v1/admin/packages` takes a path on the
//! machine's own disk, and handing it `~/Downloads/vol1.kmpkg` would install a package the machine
//! would look for again at the next start and not find — precisely the fault
//! `Installing a package by dropping it on the window` records for the drop path. The path handed
//! over is the one inside the packages folder.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::settings::{Paths, Settings};

/// How long to wait for a machine on loopback before deciding there is not one.
///
/// **Short on purpose.** This is a local connection to a port that either has a listener or does
/// not, so the only outcomes are an immediate answer and an immediate refusal — the timeout is for
/// the third case, something else holding the port and never replying, and a person who has just
/// double-clicked a file is watching. Half a second of nothing beats thirty.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Running {
    /// A karaoke machine answered on loopback, at this base URL.
    Machine(String),
    /// Nothing answered, or what answered was not one of ours.
    Nothing,
}

/// Whether this box already has a machine answering, and where.
///
/// **`GET /api/v1/discover` rather than a bare connection**, and the difference is the whole reason
/// this is not three lines. Port 8177 with something listening on it is not evidence of a karaoke
/// machine; `discover`'s own header already warns that a sweep meets whatever happens to be there,
/// a router's admin page included. So the `app` field is checked, and anything that answers without
/// it is treated as nothing.
///
/// Public unconditionally, because it sits outside `/api/v1/admin/` and always will — which is what
/// makes the probe work before this process has minted itself a token.
pub(crate) fn machine_here(settings: &Settings) -> Running {
    // The configured port, on loopback rather than on whatever `api.bind` says. A machine bound to
    // `0.0.0.0` is listening here too, and one bound to a single LAN address is not -- in which case
    // there is genuinely nothing to hand to, and starting normally is the right answer.
    let port = settings.api.socket_addr().port();
    let base = format!("http://127.0.0.1:{port}");
    let url = format!("{base}/api/v1/discover");

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(PROBE_TIMEOUT))
        .build()
        .new_agent();
    let Ok(mut response) = agent.get(&url).call() else {
        return Running::Nothing;
    };
    // Read as text and parsed here rather than through `ureq`'s own `json` feature, which is not
    // enabled: `serde_json` is already a direct dependency of this crate and turning the feature on
    // would add a second path to the same crate for two call sites.
    let Ok(body) = response.body_mut().read_to_string() else {
        return Running::Nothing;
    };
    let Ok(discovery) = serde_json::from_str::<km_api::discover::Discovery>(&body) else {
        return Running::Nothing;
    };
    if discovery.app != km_api::discover::APP {
        tracing::debug!(%url, app = %discovery.app, "something answered, but it is not a machine");
        return Running::Nothing;
    }
    Running::Machine(base)
}

/// A token for the machine running here, minted from the hash in the settings file this process read.
///
/// **Not a login**, which would need the password rather than the hash and there is no password to
/// be had. `AdminAuth` verifies a token by recomputing an HMAC keyed on the stored hash, so building
/// one from the same hash produces exactly what that machine will accept — the two processes agree
/// because they read the same file.
fn mint_own_token(settings: &Settings) -> Result<String, String> {
    let hash = settings.api.admin_password_hash.clone().ok_or_else(|| {
        "this machine has no admin password in its settings file, so nothing can be handed to it"
            .to_owned()
    })?;
    km_api::AdminAuth::with_hash(hash)
        .with_epoch(settings.api.session_epoch)
        .issue()
        .map(|grant| grant.token)
        .ok_or_else(|| "could not build a token from this machine's own settings".to_owned())
}

/// Asks a running machine to install a package already sitting in its packages folder.
///
/// The path is the machine's own, which is why [`crate::dropped::adopt`] runs first on both paths.
fn ask_to_install(base: &str, settings: &Settings, path: &Path) -> Result<String, String> {
    let url = format!("{base}/api/v1/admin/packages");
    let agent = ureq::Agent::config_builder()
        // Not [`PROBE_TIMEOUT`]: installing reads a package and writes catalog rows, and a large
        // one takes a while. The probe has already established that something is answering, so the
        // thing this guards against is a machine that is busy rather than one that is absent.
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .new_agent();

    // **This signs itself in, and the reason it may is the reason the cold path needs no permission
    // at all.** Installing became an admin action, and without this a double-click would work while
    // the machine was *off* and fail while it was on — which would be the least explicable behaviour
    // in the product. What makes it legal rather than a hole: this is the same binary on the same
    // box reading the same settings file, so whoever ran it could have read the hash by opening the
    // file. `Only somebody standing at the machine` is the standing position, and this is that
    // person with a file in their hand.
    let token = mint_own_token(settings)?;

    let body = serde_json::json!({ "path": path.display().to_string() }).to_string();
    let mut response = agent
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .send(&body)
        .map_err(|error| match error {
            // Reaching this means the token was refused, which after the signing above is a machine
            // whose settings file has changed since this process read it -- somebody changing the
            // password, or ending every session, between the double-click and the request.
            ureq::Error::StatusCode(401 | 403) => {
                "the machine already running here would not accept this program's own credentials, \
                 which usually means its password changed a moment ago"
                    .to_owned()
            }
            other => format!("could not hand it to the machine already running here: {other}"),
        })?;

    let answered = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("the machine answered something unreadable: {error}"))?;
    let report: km_api::dto::InstallReportDto = serde_json::from_str(&answered)
        .map_err(|error| format!("the machine answered something unreadable: {error}"))?;

    // The wording is [`km_api::dto::InstallReportDto::sentence`]'s, the same one a drop and an upload
    // reach: three roads to an install, one sentence, and the machine on the other end of this
    // request is the same program that would have said it had it been handed the file itself.
    Ok(report.sentence())
}

/// What the caller should do once a handed-over package has been dealt with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Next {
    /// A machine already running took it. There is nothing left for this process to do.
    Done,
    /// The package is in the folder and the ordinary startup scan will install it.
    StartNormally,
}

/// Deals with a package the operating system handed this process.
///
/// Copies it in, then either hands it to the machine already running here or leaves it for startup.
pub(crate) fn take(paths: &Paths, settings: &Settings, path: &Path) -> anyhow::Result<Next> {
    let path = absolute(path);
    let (destination, id) = crate::dropped::adopt(paths, &settings.debug.packages, &path)
        .map_err(|reason| anyhow::anyhow!("{} could not be taken in: {reason}", path.display()))?;

    match machine_here(settings) {
        Running::Machine(base) => {
            tracing::info!(machine = %base, package = %id, "handing the package to the machine already running");
            let said = ask_to_install(&base, settings, &destination).map_err(|reason| {
                // The file is left where it is rather than tidied away. It is a valid package in the
                // packages folder, so the next start installs it -- which turns a failure here into
                // a delay rather than a loss, and is the better half of the two paths to fall back
                // to.
                anyhow::anyhow!(
                    "{reason}. It is in the packages folder, so the next start will take it"
                )
            })?;
            km_console::say(format!("  {said}"));
            km_console::say("  The machine already running has it; nothing new was started.");
            Ok(Next::Done)
        }
        Running::Nothing => {
            tracing::info!(package = %id, "took a package in; starting the machine");
            Ok(Next::StartNormally)
        }
    }
}

/// A path a file manager handed over, made absolute.
///
/// **A double-click delivers an absolute path everywhere**, so this is for the person typing
/// `karaokemachine vol1.kmpkg` at a shell. It matters because the path is written into the machine's
/// catalog as where the package came from, and a relative one is a different file the moment
/// anything changes directory.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|dir| dir.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relative path is resolved against the working directory, and an absolute one is left alone.
    #[test]
    fn a_typed_relative_path_is_made_absolute_and_an_absolute_one_is_untouched() {
        let relative = absolute(Path::new("vol1.kmpkg"));
        assert!(relative.is_absolute());
        assert!(relative.ends_with("vol1.kmpkg"));

        let already = if cfg!(windows) {
            PathBuf::from(r"D:\tunes\karaoke\vol1.kmpkg")
        } else {
            PathBuf::from("/tunes/karaoke/vol1.kmpkg")
        };
        assert_eq!(absolute(&already), already);
    }

    /// Nothing is listening on a port nothing was started on, and that is not an error.
    ///
    /// The refusal path rather than the timeout one, which is what a closed loopback port gives: the
    /// answer has to be `Nothing` promptly, because it is the common case — most double-clicks
    /// happen with no machine running.
    #[test]
    fn a_box_with_no_machine_on_it_says_so_rather_than_waiting() {
        let mut settings = Settings::default();
        // A port from the ephemeral range that nothing in this repository ever binds.
        settings.api.bind = "127.0.0.1:1".to_owned();
        assert_eq!(machine_here(&settings), Running::Nothing);
    }

    /// The hand-off's own token is one the running machine will accept.
    ///
    /// **The whole of why a double-click still works while the machine is on.** Installing became an
    /// admin action, so without this a package handed to a running machine would be refused — and
    /// the same double-click against a machine that was *off* would succeed, which is the least
    /// explicable pair of behaviours the product could have.
    ///
    /// Both sides are built from the same settings, which is what the two processes really share: a
    /// file. The second `AdminAuth` here stands for the machine that is already running.
    #[test]
    fn the_hand_off_mints_a_token_the_running_machine_accepts() {
        let mut settings = Settings::default();
        settings.ensure_password();
        settings.api.session_epoch = 3;

        let token = mint_own_token(&settings).expect("a token is minted from the stored hash");

        let running = km_api::AdminAuth::with_hash(
            settings
                .api
                .admin_password_hash
                .clone()
                .expect("a hash was generated"),
        )
        .with_epoch(3);
        assert!(running.verify(&token), "the running machine must accept it");
    }

    /// ...and a machine whose sessions have since been ended does not accept it.
    ///
    /// The honest limit of reading a file rather than logging in: this process's copy of the epoch is
    /// from the moment it read the settings, and the refusal it gets says so.
    #[test]
    fn a_hand_off_token_does_not_survive_a_sign_out_everywhere() {
        let mut settings = Settings::default();
        settings.ensure_password();

        let token = mint_own_token(&settings).expect("a token is minted");
        let running = km_api::AdminAuth::with_hash(
            settings
                .api
                .admin_password_hash
                .clone()
                .expect("a hash was generated"),
        )
        .with_epoch(settings.api.session_epoch + 1);
        assert!(!running.verify(&token));
    }

    /// A settings file with no hash at all is refused with a sentence rather than a panic.
    #[test]
    fn a_machine_with_no_stored_password_is_refused_in_words() {
        let settings = Settings::default();
        let error = mint_own_token(&settings).expect_err("there is no hash to mint from");
        assert!(error.contains("no admin password"), "{error}");
    }
}
