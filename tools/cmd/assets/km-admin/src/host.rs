//! This program answering the owner's page, over HTTP against a machine on the network.
//!
//! **The other implementation of `km_admin_pages::machine`.** The machine implements those traits in
//! its own process, calling its `Controller` directly; this one implements them against
//! `/api/v1`, so both surfaces draw one set of templates rather than a second `layout.html`, a
//! second `machine.html` and a second output picker kept in step with the machine's by hand.
//!
//! # Every call goes through [`crate::machine::Call`]
//!
//! Not one method here builds a path. `Call` is a verb and a path together as a type, with **no
//! `&str` door**, and its own note says why: *"a `&str` is how this program spent its whole life
//! sending files nowhere."* Three upload routes and a rename had been posting to paths the machine
//! does not mount, with four green tests saying otherwise, because the tests mounted their mock on
//! the client's own constants. `every_call_this_program_makes_is_a_route_the_machine_mounts` sweeps
//! `Call::ALL` against `km_api::routes::SURFACE` and is the test that found it.
//!
//! # What a capability being off means here
//!
//! `Capabilities::desktop()` has the installed-package table, the wallpaper rotation and the
//! machine's bank list **on**, so the eleven methods behind them talk to `/api/v1` like every other
//! one. See `What it is not is a second /admin/`.
//!
//! Three things stay refused or empty, and each is a different kind of *cannot*:
//!
//! * **`set_password(None)`** — a reset draws a new PIN on a television this program is not beside.
//! * **the power controls** — switching a machine off is a thing to do next to it.
//! * **`Problems`** — not implemented here at all, because its rows are identified by a path
//!   `PackageProblemDto` refuses to publish. `Admin::problems` is `None` for this host.
//!
//! And two values cross in a lossy direction, both for the same published rule — *the page prints
//! the sentence, a remote gets the flag*. A bank's and a picture's `why_not_removable` become a
//! sentence this program words itself (`not_this_ones_to_delete`), because the machine's own names a
//! path; and a package's **size** does not cross at all, `PackageDto` carrying no byte count.

use km_admin_pages::machine::{
    AdminError, Identity, Listing, Machine, Pictures, Songs, Sound, SwitchState, Switches, Uploads,
};

use crate::machine::Refused;
use crate::server::State;

/// A machine on the network, answering the owner's page.
///
/// Holds the whole [`State`] rather than a client, because the client is *replaceable*: somebody
/// picks a different machine on the *This machine* pane and `State::set_machine` swaps it. A host
/// that had captured a client at construction would go on talking to the machine chosen at startup —
/// which is `km-remote-host`'s `client_slot` bug one program over, and the reason that crate's
/// `machine()` reads a live client rather than a snapshot.
pub struct RemoteMachine(State);

impl RemoteMachine {
    /// Over whatever machine this program is currently pointed at.
    #[must_use]
    pub fn new(state: State) -> Self {
        Self(state)
    }

    /// The client for the machine chosen now, or the fault that says none is.
    fn client(&self) -> Result<crate::machine::Client, AdminError> {
        self.0.client().ok_or(AdminError::Offline("no machine"))
    }

    /// The same, having spent a remembered password if one is held and no token is.
    ///
    /// **For the calls that need a token, and not for the reads beside them.** `/discover`,
    /// `/packages`, `/wallpapers`, `/audio/soundfonts`, `/demo`, `/debug` and the rest of the panel's
    /// reads all ship public, so logging in before one would put this program on the network to
    /// fetch something it could have had for nothing — and `Switches::read` alone is four concurrent
    /// reads, which would be four concurrent logins on the first page load.
    ///
    /// `State::log_in_if_remembered` is where the *moment to spend it* is argued.
    async fn writing(&self) -> Result<crate::machine::Client, AdminError> {
        let client = self.client()?;
        self.0.log_in_if_remembered(&client).await;
        Ok(client)
    }
}

/// A client refusal, as the fault a page reports.
///
/// **This is where the seam earns the `AdminError` vocabulary.** Four of these six become *codes*,
/// because a page has to say them in the reader's language and this program is talking to a machine
/// that may not share one; the other two arrive already worded — by the machine, which is the
/// authority on what a name may be, or by this program about its own disk.
fn fault(refused: Refused) -> AdminError {
    match refused {
        // A machine nobody has chosen, and one that will not answer, are the same thing to a page:
        // there is nothing to draw and the *This machine* pane is where it gets fixed.
        Refused::NoMachine => AdminError::Offline("no machine"),
        Refused::Unreachable(_) => AdminError::Offline("unreachable"),
        Refused::Unauthorized | Refused::Rejected(_) => AdminError::Unauthorized,
        // The machine's own sentence, passed through. `Refused` is the variant for exactly this.
        Refused::Said(said) => AdminError::Refused(said),
        // This program's own, about its own disk -- a file it could not read. Shown rather than
        // swallowed, because the alternative is a control that silently does nothing.
        Refused::Local(said) => AdminError::Failed(said),
    }
}

#[async_trait::async_trait]
impl Machine for RemoteMachine {
    async fn name(&self) -> Result<String, AdminError> {
        // **`/discover` and not a cached name.** The heading is on every page, so this is one request
        // per page load -- and it is the same request `Chrome::page` was already making before there
        // was a seam, for the same reason: the machine may have been renamed by somebody else, and a
        // page that showed the name this program was started with would be quietly wrong.
        Ok(self.client()?.discover().await.map_err(fault)?.name)
    }

    async fn identity(&self) -> Result<Identity, AdminError> {
        let discovery = self.client()?.discover().await.map_err(fault)?;
        // **A second call, and the panel is drawn without it rather than not at all.** `/discover`
        // carries no locale, and asking for one is cheap and public -- but a machine that answered
        // the first call and not this one is still worth drawing a page for, so a failure here
        // reads as English and the picker opens there. `GET /debug` makes the same trade for the
        // same reason, on the pane below this one.
        let locale = self.client()?.locale().await.unwrap_or_default();
        // **The identity, learned from a call this page was making anyway.** `/discover` is what says
        // which machine is at the address somebody chose, and recording it is what makes a later
        // change of address recognizable as *that machine moved* rather than as a stranger: the
        // follow keys on the id, and so does the password this computer was told to remember. A
        // machine that has never answered therefore has neither, which is why the remember box is
        // absent rather than drawn and ignored.
        self.0.machine_answered(
            &discovery.id,
            km_api::discover::display_name(&discovery.name).map(str::to_owned),
        );
        Ok(Identity {
            urls: discovery.urls,
            songs: discovery.song_count,
            version: discovery.version.into_owned(),
            locale,
            // **No power controls from here, and that is a decision rather than a gap.** Switching a
            // machine off is a thing to do beside it, not from a laptop across the room -- and the
            // page that offers it is the one somebody reaches on the machine's own address.
            power: false,
        })
    }

    async fn set_name(&self, name: &str) -> Result<(), AdminError> {
        self.writing().await?.rename(name).await.map_err(fault)
    }

    async fn set_locale(&self, tag: &str) -> Result<(), AdminError> {
        // **Parsed here rather than posted as it arrived**, so what crosses the network is one of
        // this build's own tags. The machine checks again and is the authority; this is what stops
        // a host inventing one, which is the check the seam's own note asks for.
        let chosen = km_locale::Locale::parse(tag).ok_or(AdminError::NotFound)?;
        self.writing()
            .await?
            .set_locale(chosen)
            .await
            .map_err(fault)
    }

    async fn set_password(&self, password: Option<&str>) -> Result<(), AdminError> {
        let Some(password) = password else {
            // **A reset draws the new PIN on the machine's own television**, so the act only makes
            // sense beside the screen that will show it. `Capabilities::desktop` leaves the button
            // out; this is the same answer one layer down.
            return Err(AdminError::Refused(
                "Resetting the password shows a new PIN on the machine's screen, so it is done on \
                 the machine's own page."
                    .to_owned(),
            ));
        };
        self.writing()
            .await?
            .set_password(password)
            .await
            .map_err(fault)
    }

    async fn reset_sessions(&self) -> Result<(), AdminError> {
        self.writing().await?.reset_sessions().await.map_err(fault)
    }

    async fn shut_down(&self) -> Result<(), AdminError> {
        Err(no_power())
    }

    async fn restart(&self) -> Result<(), AdminError> {
        Err(no_power())
    }
}

#[async_trait::async_trait]
impl km_admin_pages::guard::Guard for RemoteMachine {
    async fn allows(
        &self,
        _caller: &km_admin_pages::guard::Caller,
    ) -> Result<(), km_admin_pages::guard::Refusal> {
        // **The caller is ignored, and that is the difference between the two hosts.** On the machine
        // this question is *may this browser act as the owner*, asked of a token in its cookie. Here
        // the browser is on loopback and this program has no password of its own; what the page needs
        // to know is whether **the machine** will accept us, which is whether a token is held.
        //
        // So the answer is about this program's session rather than the browser's, and the standing
        // shape is *try, and ask for the password only if refused*: reads ship public, so a page that
        // demanded a credential up front would be asking before there was anything to spend it on.
        //
        // **What asks this is the Machine tab, once per load, and no write waits on the answer.**
        // `Capabilities::desktop()` installs no middleware, so on this surface nothing local
        // authorizes a write and the machine's own 401 is what refuses one — see `gate_every_route`
        // for why a per-handler check here would be the wrong repair. What the page needs from this
        // is which way to draw its *Log in* pane: a password box, or the sentence saying there is
        // nothing left to type.
        let Some(client) = self.0.client() else {
            return Err(km_admin_pages::guard::Refusal::Failed(
                "No machine is selected. Choose one first.".to_owned(),
            ));
        };
        if client.has_token() {
            Ok(())
        } else {
            Err(km_admin_pages::guard::Refusal::NeedsPassword(
                "this machine requires a password".to_owned(),
            ))
        }
    }

    async fn factory_password(&self) -> bool {
        // **Read off `/discover`, which this program was reading anyway** to point at a machine at
        // all — so the nag costs nothing it was not already spending. A machine that will not answer
        // is not nagged about: the banner would be a claim about a machine nobody can reach.
        match self.0.client() {
            Some(client) => client.on_a_factory_password().await.unwrap_or(false),
            None => false,
        }
    }

    async fn remembering(&self) -> Option<km_admin_pages::guard::Remembering> {
        // `None` when nothing has answered at this address yet, which is what `State::remembering`
        // reports: a password is keyed by the machine's id, and there is none to key one under.
        self.0.remembering()
    }

    async fn forget_password(&self) {
        self.0.remember_password(None);
    }

    async fn login(
        &self,
        password: &str,
        _caller: &km_admin_pages::guard::Caller,
        remember: bool,
    ) -> Result<km_admin_pages::guard::LoggedIn, km_admin_pages::guard::Refusal> {
        let Some(client) = self.0.client() else {
            return Err(km_admin_pages::guard::Refusal::Failed(
                "No machine is selected. Choose one first.".to_owned(),
            ));
        };
        client
            .log_in(password)
            .await
            .map_err(|error| km_admin_pages::guard::Refusal::NeedsPassword(error.to_string()))?;
        // **Only a password the machine accepted is written down**, and an unticked box forgets:
        // the box is a statement about what this computer should be remembering rather than an act
        // taken once. Ordered after the login so a typo is never stored.
        self.0
            .remember_password(if remember { Some(password) } else { None });
        // **`Kept`, because the token went into the client** — this program's own session, for the
        // life of the process, not this browser's. A cookie here would claim a session this page is
        // not the keeper of. See `guard::LoggedIn`.
        Ok(km_admin_pages::guard::LoggedIn::Kept)
    }
}

/// Why this program does not switch a machine off. See [`Machine::identity`]'s `power`.
fn no_power() -> AdminError {
    AdminError::Refused(
        "Switching the machine off is done on the machine's own page, beside the machine."
            .to_owned(),
    )
}

#[async_trait::async_trait]
impl Switches for RemoteMachine {
    async fn read(&self) -> Result<SwitchState, AdminError> {
        let client = self.client()?;

        // **Four reads for one pane, and the shape of the trait is what hides that.** In the
        // machine's own process this is four field accesses; here it is four requests, and the
        // Debugging pane needs all of them -- the console's state is only legible beside debugging's,
        // and the pane says *which* switch is missing rather than only reporting success.
        //
        // `debug` is read as well as `/discover`, and that is not redundant: `/discover` knows only
        // the **running** value, and a switch drawn from it read *Turn debugging on* both before the
        // press and after it. See `SwitchState::debug_stored`.
        //
        // **Concurrent, because none of the four is an input to another.** In sequence they cost
        // four round trips for a pane that needs one, and against a machine that is not answering
        // four whole timeouts -- which is the difference between a page that says so and a page that
        // looks broken. `try_join!` also stops at the first refusal rather than waiting out the
        // other three.
        let (debug, dev_remote, performance, demo) = tokio::try_join!(
            client.debug(),
            client.dev_remote(),
            client.performance(),
            client.demo(),
        )
        .map_err(fault)?;

        Ok(SwitchState {
            debug_running: debug.enabled,
            debug_stored: debug.stored,
            dev_remote_stored: dev_remote.enabled,
            dev_remote_served: dev_remote.served,
            performance_overlay: performance.enabled,
            demo_enabled: demo.enabled,
            demo_stored: demo.stored,
            demo_delay_secs: demo.delay_secs,
        })
    }

    async fn set_debug(&self, on: bool) -> Result<(), AdminError> {
        self.writing().await?.set_debugging(on).await.map_err(fault)
    }

    async fn set_dev_remote(&self, on: bool) -> Result<(), AdminError> {
        self.writing()
            .await?
            .set_dev_remote(on)
            .await
            .map(|_| ())
            .map_err(fault)
    }

    async fn set_performance(&self, on: bool) -> Result<(), AdminError> {
        self.writing()
            .await?
            .set_performance(on)
            .await
            .map(|_| ())
            .map_err(fault)
    }

    async fn set_demo(&self, enabled: bool, persist: bool) -> Result<(), AdminError> {
        self.writing()
            .await?
            .set_demo(enabled, persist)
            .await
            .map(|_| ())
            .map_err(fault)
    }

    async fn set_demo_delay(&self, secs: u32) -> Result<u32, AdminError> {
        // The machine's own number back, because the route caps it -- see the trait.
        self.writing()
            .await?
            .set_demo_delay(secs)
            .await
            .map(|demo| demo.delay_secs)
            .map_err(fault)
    }
}

#[async_trait::async_trait]
impl Sound for RemoteMachine {
    async fn outputs(&self, _all: bool) -> Result<km_api::machine::AudioOutputs, AdminError> {
        let dto = self.client()?.audio_outputs().await.map_err(fault)?;
        Ok(outputs_from(dto))
    }

    async fn set_output(&self, id: &str) -> Result<km_api::machine::AudioOutputs, AdminError> {
        let dto = self
            .writing()
            .await?
            .set_audio_output(id)
            .await
            .map_err(fault)?;
        Ok(outputs_from(dto))
    }

    async fn set_level(&self, db_centi: i32) -> Result<km_api::machine::AudioOutputs, AdminError> {
        let dto = self
            .writing()
            .await?
            .set_audio_level(db_centi as f32 / 100.0)
            .await
            .map_err(fault)?;
        Ok(outputs_from(dto))
    }

    async fn banks(&self) -> Result<km_api::machine::SoundFontBanks, AdminError> {
        // **The machine's installed banks, which is a different list from this program's own.**
        // `views::banks` draws the sixty-odd banks this computer could *fetch*; this is the handful
        // the machine already has and can play now. Joining them would be one table whose rows
        // answer "get me" and "play me" through the same control.
        let dto = self.client()?.soundfonts().await.map_err(fault)?;
        Ok(km_api::machine::SoundFontBanks {
            banks: dto
                .banks
                .into_iter()
                .map(|bank| km_api::machine::SoundFontBank {
                    id: bank.id,
                    name: bank.name,
                    bytes: bank.bytes,
                    bundled: bank.bundled,
                    why_not_removable: (!bank.removable).then(not_this_ones_to_delete),
                })
                .collect(),
            selected: dto.selected,
            // **Not carried across, and neither is a mistake.** `offers` is the machine's own fetch
            // table and `fetching` its downloader -- both belong to `A fourth program`'s other half,
            // which is this program's whole reason for existing and is drawn from its own state.
            // Passing the machine's through would put two fetchers on one page.
            offers: Vec::new(),
            fetching: None,
        })
    }

    async fn loaded(&self) -> Result<km_api::machine::SoundFontStatus, AdminError> {
        let dto = self.client()?.loaded_bank().await.map_err(fault)?;
        Ok(status_from(dto))
    }

    async fn use_bank(&self, id: &str) -> Result<(), AdminError> {
        self.writing().await?.use_bank(id).await.map_err(fault)
    }

    async fn delete_bank(&self, id: &str) -> Result<(), AdminError> {
        self.writing().await?.remove_bank(id).await.map_err(fault)
    }
}

/// What a write behind an absent control answers.
///
/// **Refused rather than silently done**, because a request for a control this surface does not draw
/// was not sent by a page this program served — and `What it is not is a second /admin/` is the
/// decision that says which controls those are. The sentence names where the act lives instead,
/// because a refusal that only says *no* leaves somebody with nowhere to go.
/// What this host puts where the machine would have put its own sentence.
///
/// # The one string on a shared page that this program words
///
/// **Because the reason is a path and a path does not travel.** `SoundFontBankDto::removable` and
/// `PictureDto::removable` are booleans on purpose — *"the boolean travels and the sentence does
/// not… the reason names a path, and a path is the operator's filesystem layout"* — and the shared
/// row prints a sentence in place of the control it leaves out. So a flag has to become prose
/// somewhere, and the only place holding the flag is here.
///
/// **It is deliberately vaguer than the machine's own**, which says *which* rule protects the file
/// and names it. This cannot, and pretending otherwise would be inventing a reason.
///
/// English, like the rest of this program's own words, and it is the one string that will *not* be
/// fixed by giving this program a catalog: it is spent by a template in the shared crate, which
/// renders in the reader's language, and the trait method that returns it is handed no locale. The
/// honest repair is a code rather than a sentence, the way `AdminError` already travels — noted here
/// rather than done, because it changes a type three hosts' worth of rows read.
fn not_this_ones_to_delete() -> String {
    "The machine will not delete this one.".to_owned()
}

/// `WallpapersDto` back into the shape the pages read.
fn rotation_from(dto: km_api::dto::WallpapersDto) -> km_api::machine::WallpaperState {
    use km_api::dto::WallpaperSourceDto as From;
    use km_api::machine::WallpaperSource as To;
    km_api::machine::WallpaperState {
        current: dto.current,
        count: dto.count,
        interval_secs: dto.interval_secs,
        shuffle: dto.shuffle,
        on_song_change: dto.on_song_change,
        problem: dto.problem,
        source: match dto.source {
            From::Setting => To::Setting,
            From::Owner => To::Owner,
            From::Overlay => To::Overlay,
            From::Bundled => To::Bundled,
        },
    }
}

/// `SoundFontDto` back into the shape the pages read.
///
/// **Both `problem` and `fallback` cross**, and keeping them apart is the point the DTO makes
/// itself: `problem` is *why there is no bank*, `fallback` is *why the bank playing is not the one
/// the setting names on a machine that is otherwise fine*. Reporting the second as the first would
/// draw a working machine as broken, which is the Problems tab's whole failure mode.
fn status_from(dto: km_api::dto::SoundFontDto) -> km_api::machine::SoundFontStatus {
    use km_api::dto::SoundFontChoiceDto as Chose;
    use km_api::dto::SoundKindDto as Kind;
    use km_api::machine::SoundFontChoice as ToChose;
    use km_api::machine::SoundKind as ToKind;
    km_api::machine::SoundFontStatus {
        path: dto.path,
        chosen_by: dto.chosen_by.map(|chose| match chose {
            Chose::Setting => ToChose::Setting,
            Chose::Bundled => ToChose::Bundled,
            Chose::Fallback => ToChose::Fallback,
        }),
        playing: match dto.playing {
            Kind::SoundFont => ToKind::SoundFont,
            Kind::TestTone => ToKind::TestTone,
            Kind::Silent => ToKind::Silent,
        },
        problem: dto.problem,
        fallback: dto.fallback,
    }
}

// `machines_own_page` lived here and answered the eleven writes this host would not make --
// *"Removing and rearranging what is on the machine is done on the machine's own page."* It has no
// callers left: `What it is not is a second /admin/` was rewritten and every one of them is a real
// call now. Deleted rather than kept for a rainy day, because a refusal nothing produces is a
// sentence that would be translated, reviewed and maintained for no reader.
//
// The two things this host still refuses each say so for their own reason, in their own method:
// `set_password(None)` (a PIN on a television) and the two power controls (`no_power`).

/// `AudioOutputsDto` back into the shape the pages read.
///
/// **A faithful round trip, and it was checked rather than assumed.** The DTO is the domain type with
/// one field added — each device gains a `selected`, which is a fact about the list rather than about
/// the device — so nothing is lost coming back. `preferred` in particular survives, which matters:
/// it is what `views::output_rows` filters the short list on, and a lossy conversion here would have
/// shown thirty rows where the machine's page shows four.
fn outputs_from(dto: km_api::dto::AudioOutputsDto) -> km_api::machine::AudioOutputs {
    km_api::machine::AudioOutputs {
        devices: dto
            .outputs
            .into_iter()
            .map(|device| km_api::machine::AudioOutput {
                id: device.id,
                name: device.name,
                system_default: device.system_default,
                usb: device.usb,
                available: device.available,
                preferred: device.preferred,
            })
            .collect(),
        selected: dto.selected,
        active_id: dto.active_id,
        active_name: dto.active_name,
        fell_back: dto.fell_back,
        changeable: dto.changeable,
        // Decibels on the wire become the hundredths the vocabulary carries, which is the same
        // translation `dto` makes in the other direction.
        level: dto.level.map(|level| km_api::machine::OutputLevel {
            db_centi: centi(level.db),
            db_min_centi: centi(level.db_min),
            db_max_centi: centi(level.db_max),
            step_centi: centi(level.step_db),
        }),
    }
}

/// Decibels as the hundredths the API's vocabulary carries.
fn centi(db: f32) -> i32 {
    if db.is_nan() {
        return 0;
    }
    (db * 100.0).round() as i32
}

#[async_trait::async_trait]
impl Songs for RemoteMachine {
    async fn packages(&self) -> Result<Vec<Listing>, AdminError> {
        let dto = self.client()?.packages().await.map_err(fault)?;
        Ok(dto
            .packages
            .into_iter()
            .map(|package| Listing {
                id: package.id,
                name: package.name,
                version: package.version,
                bank: package.bank,
                song_count: package.song_count,
                // The flag becomes prose here, because `PackageDto` publishes `removable` and keeps
                // the machine's sentence -- *"the page prints the sentence, a remote gets the
                // flag."* See `not_this_ones_to_delete`.
                why_not_removable: (!package.removable).then(not_this_ones_to_delete),
                // **Not available, rather than not asked for.** `PackageDto` carries no size at
                // all: id, name, version, song count, installed-at, bank and `removable`. So the
                // table this surface draws has an empty Size column where the machine's own has a
                // number, and `PackageRow` already renders `None` as nothing for that reason.
                // Publishing it would be an additive change to that DTO and a decision of its own.
                bytes: None,
            })
            .collect())
    }

    async fn package(&self, id: &str) -> Result<Listing, AdminError> {
        // **Asked by the removal confirmation, which names what it is about to delete.** There is no
        // single-package route, and there does not need to be: `GET /packages` is one call either
        // way and the confirmation is not a page anybody loads in a loop.
        self.packages()
            .await?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or(AdminError::NotFound)
    }

    async fn remove(&self, id: &str) -> Result<usize, AdminError> {
        self.writing()
            .await?
            .remove_package(id)
            .await
            .map_err(fault)
    }

    async fn set_bank(&self, id: &str, bank: u16) -> Result<usize, AdminError> {
        self.writing()
            .await?
            .set_package_bank(id, bank)
            .await
            .map_err(fault)
    }
}

#[async_trait::async_trait]
impl Pictures for RemoteMachine {
    async fn rotation(&self) -> Result<km_api::machine::WallpaperState, AdminError> {
        let dto = self.client()?.wallpapers().await.map_err(fault)?;
        Ok(rotation_from(dto))
    }

    async fn files(&self) -> Result<Vec<km_api::machine::Picture>, AdminError> {
        // **The same `GET /wallpapers` as `rotation`, asked again.** The trait keeps the two apart
        // because the in-process host reads them from two different places; here one call answers
        // both, and paying for a second request is cheaper than a cache this program would then have
        // to know when to invalidate.
        let dto = self.client()?.wallpapers().await.map_err(fault)?;
        Ok(dto
            .pictures
            .into_iter()
            .map(|picture| km_api::machine::Picture {
                id: picture.id,
                name: picture.name,
                images: picture.images,
                bytes: picture.bytes,
                why_not_removable: (!picture.removable).then(not_this_ones_to_delete),
            })
            .collect())
    }

    async fn next(&self) -> Result<(), AdminError> {
        self.writing().await?.next_wallpaper().await.map_err(fault)
    }

    async fn delete(&self, id: &str) -> Result<(), AdminError> {
        self.writing()
            .await?
            .remove_wallpaper(id)
            .await
            .map_err(fault)
    }
}

#[async_trait::async_trait]
impl Uploads for RemoteMachine {
    async fn receive(
        &self,
        kind: km_api::machine::Upload,
        form: axum::extract::Multipart,
    ) -> Result<String, AdminError> {
        // **Staged to disk and then forwarded, which is a lifetime rather than a preference.** axum's
        // multipart `Field` borrows the request, so streaming it straight to the machine would hold
        // the browser's request open for the whole machine-side transfer -- an hour-long upload
        // timeout applied to a browser, and a failure reported after a gigabyte with nowhere to put
        // it. Staging also restores `Content-Length`, which is what lets the machine refuse an
        // oversized upload before reading any of it.
        //
        // The whole of that already exists here, and this is the same call the program's own send
        // form has always made -- see `crate::staging` for the two-halves cleanup and why one guard
        // is not enough.
        crate::handlers::stage_and_send(&self.0, kind, form)
            .await
            .map_err(fault)
    }
}
