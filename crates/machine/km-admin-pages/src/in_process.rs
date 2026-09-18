//! This machine answering its own owner's page, from inside its own process.
//!
//! **The operations, not a second copy of them.** Every method here goes through the `Controller` or
//! `km_api::ops::*` — the same code the JSON routes call — which is the
//! `One implementation of each operation` decision, and the same call `km-remote-pages`' online mode
//! makes for the same reason.
//!
//! # Why this lives beside the seam rather than in the host
//!
//! **`km-remote-pages` puts its online implementation in `karaokemachine/src/remote.rs`, and this
//! crate deliberately does not follow it.** There the implementation needs the machine's *guts* —
//! the catalog behind `spawn_blocking`, the event broadcaster, the controller — so the host is where
//! it belongs and that crate's tests use hand-written stubs instead.
//!
//! Here the whole implementation is `km_api::ApiState`, which this crate already depends on. Putting
//! it in the host would leave the page tests unable to reach it, and they assert against a *real*
//! `ApiState` built from `km_api::testing::TestMachine` — `nested_with_state` hands the state back
//! so a test can ask what the machine ended up with. The three ways out were a `testing` feature
//! exporting fakes, hand-written stubs that no longer prove the machine was actually changed, or a
//! third copy of these hundred and fifty lines in a test file. **A third copy of the thing this
//! whole exercise exists to delete is not a trade worth making**, so the implementation sits here,
//! beside the traits rather than behind them, and every host and every test drives the one copy.
//!
//! What the seam still buys is unchanged: nothing in `handlers` or `views` names `ApiState`, so a
//! host with no machine in its process implements the same traits over HTTP and the pages cannot
//! tell.

use crate::machine::{
    AdminError, Identity, Listing, Machine, Pictures, Problems, Refused, Songs, Sound, SwitchState,
    Switches, Uploads,
};
use km_api::ApiState;

/// This machine, answering from inside its own process.
///
/// # Two writes make a rename, and that is why this is a seam
///
/// `Controller::set_machine_name` writes `settings.json` and `ApiState::set_machine_name` moves the
/// running value that answers `/discover`. **A rename that reached one and not the other is the bug
/// found on the appliance in 2026-08**: the machine came back under its old name after a power cut.
/// The page had to know that and now does not — a host over HTTP sends one `PUT` and this one does
/// both, which is precisely the kind of difference a trait is for. The password does the same thing
/// one field over, and for the same reason.
pub struct ThisMachine(ApiState);

impl ThisMachine {
    /// Over this machine's own state.
    #[must_use]
    pub fn new(state: ApiState) -> Self {
        Self(state)
    }
}

/// A controller refusal, in the machine's own words.
///
/// **`Refused` and never a code.** The machine is the authority on what a name may be and on what a
/// delay may be, and it has already worded the answer — `AdminError::Refused` is the variant that
/// carries a sentence through untouched, for exactly this.
fn refused(error: impl std::fmt::Display) -> AdminError {
    AdminError::Refused(error.to_string())
}

/// An `ApiError` from `km_api::ops::*`, as the fault a page reports.
///
/// **Mapped once rather than at each call site**, because the interesting part is which variants
/// become a *code* and which keep the machine's sentence. `NotFound` and `Unauthorized` are codes:
/// the page can word those itself, and a remote host will produce them from a status line with no
/// sentence attached at all. Everything else arrives already worded by the machine and stays that
/// way — a `BadRequest` here is *"that is not a bank this machine has"*, which is better than
/// anything this crate could compose without knowing what was asked for.
fn from_api(error: km_api::ApiError) -> AdminError {
    match error {
        km_api::ApiError::NotFound(_) | km_api::ApiError::UnknownEndpoint(_) => {
            AdminError::NotFound
        }
        km_api::ApiError::Unauthorized(_) | km_api::ApiError::Forbidden(_) => {
            AdminError::Unauthorized
        }
        other => AdminError::Refused(other.to_string()),
    }
}

#[async_trait::async_trait]
impl Machine for ThisMachine {
    async fn name(&self) -> Result<String, AdminError> {
        Ok(self.0.machine_name())
    }

    async fn identity(&self) -> Result<Identity, AdminError> {
        let discovery = self.0.discovery();
        Ok(Identity {
            urls: discovery.urls.clone(),
            songs: discovery.song_count,
            version: discovery.version.to_string(),
            locale: self.0.controller().machine_locale(),
            // Read off the state rather than through `GET /api/v1/admin/power`. This runs inside the
            // machine, the same way it reads `debug_enabled`, so asking the host directly is one
            // question instead of a request to itself that would need a token.
            power: self.0.power().is_some(),
        })
    }

    async fn set_name(&self, name: &str) -> Result<(), AdminError> {
        // Settings first, then the running value. A failed write must not leave the advert naming a
        // machine whose settings file says something else -- see the type's own note.
        self.0
            .controller()
            .set_machine_name(name)
            .map_err(refused)?;
        self.0.set_machine_name(name.to_owned());
        Ok(())
    }

    async fn set_locale(&self, tag: &str) -> Result<(), AdminError> {
        let Some(chosen) = km_locale::Locale::parse(tag) else {
            // A tag this build has no catalog for. The control is a `<select>` of exactly the tags
            // there are, so anything else was typed by hand -- and `NotFound` is the honest answer
            // rather than a refusal the machine never made.
            return Err(AdminError::NotFound);
        };
        self.0
            .controller()
            .set_machine_locale(chosen)
            .map_err(refused)
    }

    async fn set_password(&self, password: Option<&str>) -> Result<(), AdminError> {
        // **`None` puts the machine back on a freshly generated PIN**, which it then draws on its
        // own television -- the one place reading it means being in the room. Only the host that is
        // beside that screen offers it, which is why this takes an `Option` rather than two methods.
        let pin = password.is_none().then(km_api::auth::generate_factory_pin);
        let plaintext = password.unwrap_or_else(|| pin.as_deref().unwrap_or_default());
        let hash = km_api::AdminAuth::hash_password(plaintext).map_err(refused)?;

        // Settings first, then the running value, for `set_name`'s reason: the token check is keyed
        // on the running hash, and a failed write must not leave it enforcing a password the
        // settings file does not hold.
        self.0
            .controller()
            .set_admin_password(Some(hash.clone()), pin.clone())
            .map_err(refused)?;
        self.0.set_admin_password(Some(hash), pin.is_some());
        Ok(())
    }

    async fn reset_sessions(&self) -> Result<(), AdminError> {
        let next = self.0.session_epoch().saturating_add(1);
        // Persist first, then move the running value -- a failed write must not leave the machine
        // enforcing an epoch its settings file does not hold.
        self.0
            .controller()
            .set_session_epoch(next)
            .map_err(refused)?;
        self.0.set_session_epoch(next);
        Ok(())
    }

    async fn shut_down(&self) -> Result<(), AdminError> {
        let Some(power) = self.0.power() else {
            return Err(AdminError::Refused(
                "This machine cannot switch itself off.".to_owned(),
            ));
        };
        // The operating system's own sentence, which is the diagnosis -- and nothing happened, so
        // the machine is still up and there is a page to put it on.
        power.shut_down().map_err(refused)
    }

    async fn restart(&self) -> Result<(), AdminError> {
        let Some(power) = self.0.power() else {
            return Err(AdminError::Refused(
                "This machine has no power controls.".to_owned(),
            ));
        };
        power.restart_application().map_err(refused)
    }
}

#[async_trait::async_trait]
impl Songs for ThisMachine {
    async fn packages(&self) -> Result<Vec<Listing>, AdminError> {
        // **Off the runtime, and this is the method that most needs it.** Installing a package holds
        // the catalog mutex for *seconds*, so a page load that blocked a worker behind one would
        // stall every other request too -- and with as many workers as the box has cores, the API,
        // the pages and the event stream would stop together.
        //
        // The reason each row cannot be removed is asked *inside the same hop* as the listing, so a
        // row and the reason its Remove control is missing are one consistent view of the machine
        // rather than two taken a moment apart.
        km_api::ops::off_runtime(&self.0, |state| {
            state
                .catalog()
                .packages()
                .map(|packages| {
                    packages
                        .into_iter()
                        .map(|package| Listing {
                            why_not_removable: state.catalog().why_not_removable(&package),
                            // Not measured here. See `Listing::bytes`.
                            bytes: None,
                            id: package.id,
                            name: package.name,
                            version: package.version,
                            bank: package.bank,
                            song_count: package.song_count,
                        })
                        .collect()
                })
                .map_err(km_api::ApiError::from)
        })
        .await
        .map_err(from_api)
    }

    async fn package(&self, id: &str) -> Result<Listing, AdminError> {
        let id = id.to_owned();
        km_api::ops::off_runtime(&self.0, move |state| {
            let found = state
                .catalog()
                .packages()
                .map_err(km_api::ApiError::from)?
                .into_iter()
                .find(|package| package.id == id);
            let Some(package) = found else {
                return Err(km_api::ApiError::NotFound(format!("package '{id}'")));
            };
            Ok(Listing {
                why_not_removable: state.catalog().why_not_removable(&package),
                // Measured here and nowhere else: this is the page asking whether to delete it.
                bytes: state.catalog().package_bytes(&package),
                id: package.id,
                name: package.name,
                version: package.version,
                bank: package.bank,
                song_count: package.song_count,
            })
        })
        .await
        .map_err(from_api)
    }

    async fn remove(&self, id: &str) -> Result<usize, AdminError> {
        let id = id.to_owned();
        km_api::ops::off_runtime(&self.0, move |state| {
            state
                .catalog()
                .uninstall(&id)
                .map_err(km_api::ApiError::from)
        })
        .await
        .map_err(from_api)
    }

    async fn set_bank(&self, id: &str, bank: u16) -> Result<usize, AdminError> {
        let id = id.to_owned();
        km_api::ops::off_runtime(&self.0, move |state| {
            state
                .catalog()
                .set_package_bank(&id, bank)
                .map_err(km_api::ApiError::from)
        })
        .await
        .map_err(from_api)
    }
}

#[async_trait::async_trait]
impl Pictures for ThisMachine {
    async fn rotation(&self) -> Result<km_api::machine::WallpaperState, AdminError> {
        Ok(self.0.controller().wallpapers())
    }

    async fn files(&self) -> Result<Vec<km_api::machine::Picture>, AdminError> {
        Ok(self.0.controller().wallpaper_pictures())
    }

    async fn next(&self) -> Result<(), AdminError> {
        self.0
            .controller()
            .next_wallpaper()
            .map(|_| ())
            .map_err(refused)
    }

    async fn delete(&self, id: &str) -> Result<(), AdminError> {
        self.0
            .controller()
            .delete_wallpaper(id)
            .map(|_| ())
            .map_err(refused)
    }
}

#[async_trait::async_trait]
impl Problems for ThisMachine {
    async fn refused(&self) -> Result<Vec<Refused>, AdminError> {
        // **No `off_runtime` hop here, unlike its neighbours.**
        // `package_problems` is a clone out of its own mutex and never touches the catalog lock an
        // install holds for seconds -- and this is counted on *every* page for the nav badge, so a
        // hop per page load would be paying for a thread switch to read a `Vec`.
        let catalog = self.0.catalog();
        Ok(catalog
            .package_problems()
            .into_iter()
            .map(|problem| Refused {
                id: problem.id(),
                file: file_name_of(&problem.path),
                folder: folder_of(&problem.path),
                why_not_removable: catalog.why_problem_not_removable(&problem),
                // Measured only where somebody is being asked to delete it. `problem_bytes` is a
                // filesystem touch and this list is read on every page.
                bytes: None,
                reason: problem.reason,
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<(), AdminError> {
        let id = id.to_owned();
        // Off the runtime because this deletes a file, which can block for as long as the
        // filesystem wants to -- the rule `Songs::packages` states for the catalog lock applies to
        // the disk as well.
        km_api::ops::off_runtime(&self.0, move |state| {
            state
                .catalog()
                .delete_problem_file(&id)
                .map_err(km_api::ApiError::from)
        })
        .await
        .map_err(from_api)
    }
}

/// The last component of a path, whichever separator the machine wrote.
///
/// Both separators, for the reason `km_api::dto`'s own copy of this gives: these strings are made by
/// the machine and read here, and a `Path::file_name` on a build that is not Windows hands a Windows
/// path back whole.
///
/// **It moved here from `handlers` with the seam**, and that is the right side of it: splitting a
/// path the machine wrote is something only a host reading that machine's own disk has to do, and a
/// host over HTTP is handed a file name and a folder already apart.
fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// Everything in front of the last component, for the row that has to tell two files apart.
///
/// Empty when there is nothing in front of it, which the template renders as an empty line rather
/// than as the word "none": a bare file name with no folder is a path the machine wrote, and saying
/// so at length would be more confusing than showing nothing.
fn folder_of(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(cut) => path[..cut].to_owned(),
        None => String::new(),
    }
}

#[async_trait::async_trait]
impl Uploads for ThisMachine {
    async fn receive(
        &self,
        kind: km_api::machine::Upload,
        form: axum::extract::Multipart,
    ) -> Result<String, AdminError> {
        // **`km_api::uploads::receive` and nothing of its own**, which is the whole reason that
        // module is public: this page serves forms carrying the same three kinds of file the JSON
        // routes take, and it must not grow a second multipart handler. The size cap, the extension
        // list, the name sanitising and the streaming are all stated once, over there.
        km_api::uploads::receive(&self.0, form, kind)
            .await
            .map_err(from_api)
    }
}

#[async_trait::async_trait]
impl Switches for ThisMachine {
    async fn read(&self) -> Result<SwitchState, AdminError> {
        // One read for the pane, which is what the trait method is shaped for: the Debugging pane
        // draws all three together and the console's state is only legible beside debugging's.
        let developer = self.0.controller().developer_switches();
        let demo = self.0.controller().demo();
        Ok(SwitchState {
            debug_running: self.0.config().debug_enabled,
            debug_stored: developer.debug,
            dev_remote_stored: developer.dev_remote,
            dev_remote_served: km_api::routes::dev_console_served(self.0.config()),
            performance_overlay: self.0.controller().performance_overlay(),
            demo_enabled: demo.enabled,
            demo_stored: demo.stored,
            demo_delay_secs: demo.delay_secs,
        })
    }

    async fn set_debug(&self, on: bool) -> Result<(), AdminError> {
        self.0
            .controller()
            .set_debug_enabled(on)
            .map(|_| ())
            .map_err(refused)
    }

    async fn set_dev_remote(&self, on: bool) -> Result<(), AdminError> {
        self.0
            .controller()
            .set_dev_remote_enabled(on)
            .map(|_| ())
            .map_err(refused)
    }

    async fn set_performance(&self, on: bool) -> Result<(), AdminError> {
        self.0
            .controller()
            .set_performance_overlay(on)
            .map(|_| ())
            .map_err(refused)
    }

    async fn set_demo(&self, enabled: bool, persist: bool) -> Result<(), AdminError> {
        self.0
            .controller()
            .set_demo(enabled, persist)
            .map(|_| ())
            .map_err(refused)
    }

    async fn set_demo_delay(&self, secs: u32) -> Result<u32, AdminError> {
        // **The machine's own number back, not the one that was asked for.** The route caps it, and
        // the notice quotes what was actually stored -- a page that echoed the request would tell
        // somebody their 9999 was saved.
        self.0
            .controller()
            .set_demo_delay(secs)
            .map(|demo| demo.delay_secs)
            .map_err(refused)
    }
}

#[async_trait::async_trait]
impl Sound for ThisMachine {
    async fn outputs(&self, all: bool) -> Result<km_api::machine::AudioOutputs, AdminError> {
        // **`all` is not passed down**, because it is not a question the machine answers:
        // `Controller::audio_outputs` answers with every device it knows and marks the `preferred`
        // ones, so widening the list is a filter over one answer. `views::output_rows` is where that
        // filter lives, because which rows to draw is a question about a page.
        let _ = all;
        self.0.controller().audio_outputs().map_err(refused)
    }

    async fn set_output(&self, id: &str) -> Result<km_api::machine::AudioOutputs, AdminError> {
        self.0.controller().set_audio_output(id).map_err(|error| {
            // **`Unavailable` is the 409 -- busy, not broken**, and it is the one refusal on this
            // page that reads as *try again when the song ends* rather than as *something is wrong
            // with what you sent*. Everything else is the machine declining an identifier it does
            // not have.
            match error {
                km_api::machine::ControlError::Unavailable(said) => {
                    AdminError::Busy(said.to_string())
                }
                other => refused(other),
            }
        })
    }

    async fn set_level(&self, db_centi: i32) -> Result<km_api::machine::AudioOutputs, AdminError> {
        self.0
            .controller()
            .set_output_level(db_centi)
            .map_err(|error| match error {
                // **An output with no level reads as *busy* here, and the word is wrong for it.**
                // `AdminError::Busy` is this page's 409, and a 409 is what the machine answers:
                // the request was well formed and the machine declined it. The sentence the page
                // shows is the machine's own, which says there is no level rather than to wait.
                km_api::machine::ControlError::Unavailable(said) => {
                    AdminError::Busy(said.to_string())
                }
                other => refused(other),
            })
    }

    async fn banks(&self) -> Result<km_api::machine::SoundFontBanks, AdminError> {
        // **`true`, the whole catalog.** This page lists what is installed so it can be used or
        // removed, which is not the shortlist a picker wants -- and `soundfonts` is read-only and
        // unfailing, reporting the bundled bank alone for a folder it cannot read, which is the
        // truth about what will play.
        Ok(self.0.controller().soundfonts(true))
    }

    async fn loaded(&self) -> Result<km_api::machine::SoundFontStatus, AdminError> {
        Ok(self.0.controller().soundfont())
    }

    async fn use_bank(&self, id: &str) -> Result<(), AdminError> {
        // **Through `ops` and not the controller**, which is the half that would have diverged:
        // choosing a bank is one call, and choosing a bank *and telling every open page about it* is
        // two -- of which the second is invisible when forgotten.
        km_api::ops::set_soundfont(&self.0, id)
            .map(|_| ())
            .map_err(from_api)
    }

    async fn delete_bank(&self, id: &str) -> Result<(), AdminError> {
        km_api::ops::delete_soundfont(&self.0, id)
            .map(|_| ())
            .map_err(from_api)
    }
}
