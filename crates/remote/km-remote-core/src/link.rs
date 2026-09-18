//! Which machine this remote is talking to, and everything that can change that.
//!
//! **This is a struct because two callers need exactly these six things.**
//! [`Server::machine_watch`](crate::Server::machine_watch) wants the client, the locator, the data
//! directory, the mirror, the force-refresh flag and whether a machine was named — six separate
//! `Server` fields cloned into one `async move` — and the pages want the same six. One value with a
//! name beats a capture list written out twice.
//!
//! What it adds beyond the gathering is that **`pinned` can change while the program runs.** A
//! command-line flag is not the only thing that pins a machine: a person typing an address into a
//! page pins one too, and pressing *Rescan* is how they take it off again. So it is an
//! [`AtomicBool`] read each tick rather than a `bool` captured once.
//!
//! `how` lives here for a related reason. [`Ready::machine`](crate::Ready) is documented as the
//! *startup* report and is right to be a snapshot; a card on a live page needs the answer as it
//! stands, and a shell polling the snapshot waits for ever.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use km_remote_pages::machine::{
    Connect, Copied, CopyOutcome, Machine, MachineStatus, Offer, RemoteError, Scan, Scanned, codes,
};

use km_api::discover::known::{self, Known, Why};

use crate::client::{Api, MachineClient};
use crate::find;
use crate::mirror::Mirror;
use crate::sync;

/// The machine in hand, and how it came to be that one.
pub struct Link {
    /// The client the pages talk through. Re-pointed, never replaced.
    machine: MachineClient,
    /// What the network is saying. See [`Locator`](crate::find::Locator) for why the mechanism is injected, and
    /// [`find::Radar`] for why the *listening* is not the caller's problem.
    radar: Arc<find::Radar>,
    /// Where the machine this device knows gets written down.
    data_dir: PathBuf,
    /// This device's copy of the catalog.
    mirror: Arc<Mutex<Mirror>>,
    /// Whether to re-read the catalog even when the machine says nothing changed.
    force_refresh: bool,
    /// Whether this machine is to be kept whatever the network offers.
    ///
    /// See the module header: it is atomic because *Rescan* clears it and a typed address sets it.
    pinned: AtomicBool,
    /// How the current address was arrived at, worded for a person.
    how: RwLock<Option<Why>>,
    /// The machine this device knows, as last written down.
    ///
    /// Held here as well as on disk because [`find::choose`](km_api::discover::known::choose) is
    /// asked on every tick and every change, and re-reading a file that many times to learn
    /// something this process already knows would be silly.
    known: RwLock<Option<Known>>,
    /// The id `/discover` last reported at the address in hand.
    ///
    /// **`None` is not the same as a machine that answered with a different id**, and rule 2 of
    /// `choose` turns on the difference: one means *nothing has told me what is there*, the other
    /// means *something is there and it is not ours*. Cleared by [`point_at`](Self::point_at),
    /// which is the only thing that can mean a different machine at all — the same reasoning
    /// `Connection::name` already follows.
    answering_id: RwLock<Option<String>>,
}

/// How a run started out, as against what it holds.
///
/// **Three fields in one argument rather than three arguments**, and they belong together on their
/// own terms: each is an answer to *what did this process know before it had asked anybody*.
/// Grouping them also keeps [`Link::new`] at the width it has always had — a positional `bool`
/// between two `Option`s is exactly the call somebody eventually gets the wrong way round.
#[derive(Debug, Default)]
pub struct Origin {
    /// Whether the machine was named rather than found. See [`Link::pinned`].
    pub pinned: bool,
    /// How the address in hand was arrived at.
    pub how: Option<Why>,
    /// The machine this device remembers, if it remembers one.
    pub known: Option<Known>,
}

impl Link {
    /// Gathers what a running remote needs to know about its machine.
    pub fn new(
        machine: MachineClient,
        radar: Arc<find::Radar>,
        data_dir: PathBuf,
        mirror: Arc<Mutex<Mirror>>,
        force_refresh: bool,
        origin: Origin,
    ) -> Self {
        let Origin { pinned, how, known } = origin;
        // A sweep can only look for one machine at a time, so it is told which before it is asked
        // anything. mDNS hears them all and ignores this.
        radar.hunt(known.as_ref().and_then(|known| known.id.as_deref()));
        Self {
            machine,
            radar,
            data_dir,
            mirror,
            force_refresh,
            pinned: AtomicBool::new(pinned),
            how: RwLock::new(how),
            known: RwLock::new(known),
            answering_id: RwLock::new(None),
        }
    }

    /// The client, for a caller that wants to ask the machine something.
    pub fn machine(&self) -> &MachineClient {
        &self.machine
    }

    /// The mirror, for the catalog refresh.
    pub fn mirror(&self) -> &Arc<Mutex<Mirror>> {
        &self.mirror
    }

    /// Where the address that answered is written down.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Whether a refresh re-reads regardless of the version.
    pub fn force_refresh(&self) -> bool {
        self.force_refresh
    }

    /// Whether this machine is being kept whatever the network offers.
    pub fn pinned(&self) -> bool {
        self.pinned.load(Ordering::Relaxed)
    }

    /// How the current address was arrived at, as a stable code the card words for itself.
    ///
    /// **A code and not a sentence** — see [`Why::code`]. This travels to `km-remote-pages`, which is
    /// rendered in the viewer's language and cannot put a sentence from here on the page.
    pub fn how(&self) -> Option<String> {
        self.why().map(|why| why.code().to_owned())
    }

    /// How the current address was arrived at.
    pub fn why(&self) -> Option<Why> {
        *self
            .how
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// What the network is saying, and how to ask it again.
    pub fn radar(&self) -> &Arc<find::Radar> {
        &self.radar
    }

    /// The machine this device knows, as last written down.
    pub fn known(&self) -> Option<Known> {
        self.known
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The id reported by whatever is answering at the address in hand.
    pub fn answering_id(&self) -> Option<String> {
        self.answering_id
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Looks on the network, off the runtime's threads.
    ///
    /// [`Locator::look`](crate::find::Locator::look) is blocking by design, so [`find::Radar`] is the one place that knows to
    /// get off the executor for it — the same reason [`crate::Server::open`] does.
    pub async fn look(&self) -> Vec<km_api::discover::Sighting> {
        self.radar.look().await
    }

    /// Points at a machine, saying how it was arrived at.
    ///
    /// **It does not write the address down, and that omission is load-bearing rather than an
    /// oversight.** The rule is *nothing is remembered until it answers* — an address recorded
    /// without ever having been reached short-circuits the look for ever, which is how a remote
    /// spent an evening on a machine switched off in another room. `machine_watch` writes it down
    /// as soon as it comes online, so a real machine is remembered and a typo is forgotten by the
    /// next start.
    ///
    /// It also forgets what was answering at the old address, because that is the one thing this
    /// call is certain to have invalidated.
    pub fn point_at(&self, url: &str, how: Why) {
        self.machine.point_at(Api::new(url));
        *self
            .how
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(how);
        *self
            .answering_id
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Records what the machine at the current address said about itself.
    ///
    /// Called after every `/discover`, which `sync::refresh` already makes on every connect, every
    /// rescan and every periodic refresh — so the identity is learned on a request that was
    /// happening anyway. That is the same argument
    /// `The card says which machine, in the owner's words` used for the *name*, and it points the
    /// same way for the id.
    ///
    /// # A machine answering at a *remembered* address does not get to become the machine
    ///
    /// **This refusal is the whole of what makes an identity worth keeping**, and leaving it out
    /// made the feature quietly defeat itself — found by running it rather than by any test here.
    /// A remote holding a record for machine X, opening on the address it last saw X at, and
    /// meeting machine Y there would overwrite its own record with Y before
    /// [`choose`](km_api::discover::known::choose) ever ran. `choose` would then look for Y, find Y
    /// exactly where it was, and correctly conclude nothing had moved — so the one case an identity
    /// exists to catch was the one case it could not catch.
    ///
    /// **The distinction is who chose the address.** `asked for` is a person saying *that one* and
    /// `found on the network` is the network offering this machine, so in both what answers is
    /// legitimately the machine. `remembered` is neither: it is this device's own guess from
    /// yesterday, and a guess must not be allowed to rewrite the fact it was a guess about.
    ///
    /// A record with **no** id yet is not anchored to anything, so it always takes what answers —
    /// which is how a first connection, and an address somebody typed, learns its machine at all.
    ///
    /// The cost is one bounded, recoverable case: a machine reinstalled at the same address gets a
    /// new instance id, and this remote will keep a dead one in its record. It goes on working,
    /// because the *address* is still right and `choose` leaves a connection that answers alone;
    /// pressing Rescan or typing the address adopts the new identity, since neither arrives here as
    /// `remembered`.
    pub fn answered(&self, id: &str, name: Option<String>) {
        *self
            .answering_id
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(id.to_owned());

        let Some(url) = self.machine.api().map(|api| api.base().to_owned()) else {
            return;
        };

        let anchored_elsewhere = matches!(self.why(), Some(Why::Remembered))
            && self
                .known()
                .and_then(|known| known.id)
                .is_some_and(|known| known != id);
        if anchored_elsewhere {
            tracing::info!(
                answering = %id,
                "something else is answering at the remembered address; keeping the machine we know"
            );
            // **Learning that what is here is not ours is a reason to look now**, and without this
            // the correction waits for the fallback tick — measured at twenty seconds of a remote
            // showing the wrong machine's name and catalog on screen. It is the same argument
            // `Coming back to a page is a reason to try the machine now` makes about somebody
            // returning to a page: a fact has just arrived that beats the interval a timer had
            // arrived at. It cannot spin, because the only way back here is another `/discover`,
            // and those happen on connects and refreshes rather than on a clock.
            self.machine.wake();
            return;
        }

        let record = Known::at(&url, self.why().unwrap_or(Why::Remembered)).answered(
            id,
            name,
            std::time::SystemTime::now(),
        );

        // **Written when something a reader would notice has changed, or when the timestamp has
        // gone stale enough to be worth moving.** A refresh happens on every reconnect and every
        // periodic sync, so writing each time would be a write a second for a whole evening to
        // record a fact that has not moved.
        //
        // The second half of that condition is not a nicety: `last_connected` is what `is_stale`
        // reads, so a record only ever written when the *address* changed would freeze at the first
        // connection and a machine in continuous use would read as stale six hours later — which is
        // precisely backwards. Refreshing it at most once an hour keeps the file honest for a
        // measurement made in hours, at a cost of a handful of writes a day.
        let unchanged = self.known().is_some_and(|known| {
            known.id.as_deref() == Some(id) && known.url == record.url && known.name == record.name
        });
        let recent = self.known().is_some_and(|known| {
            known::age(&known, std::time::SystemTime::now())
                .is_some_and(|age| age < find::REMEMBER_INTERVAL)
        });
        if unchanged && recent {
            return;
        }

        self.radar.hunt(Some(id));
        find::write_known(&self.data_dir, &record);
        *self
            .known
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(record);
    }

    /// Sets or clears the pin. See the module header.
    pub fn set_pinned(&self, pinned: bool) {
        self.pinned.store(pinned, Ordering::Relaxed);
    }

    /// How many songs this device's copy holds. Zero if it cannot be read.
    fn songs(&self) -> usize {
        self.mirror
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .count()
            .unwrap_or(0)
    }

    /// Brings the copy up to date from the machine in hand, and reports what happened.
    ///
    /// **Facts, not a sentence.** Answering `Copied 1204 songs from the machine.` from here is US
    /// English composed in a crate with no catalog and no viewer to ask, and it appears verbatim on
    /// a page in any language. `km-remote-pages` writes the words; see [`Copied`].
    async fn refresh_from(&self, api: &Api) -> Result<CopyOutcome, RemoteError> {
        match sync::refresh(api, &self.mirror, self.force_refresh).await {
            Ok(refreshed) => {
                // Recorded before the sentence is built, and on both outcomes: a refresh that found
                // nothing to download still asked `/discover`, and that is the call that names the
                // machine. Reporting the name only after a download would leave it absent on almost
                // every refresh there ever is.
                self.machine.set_machine_name(refreshed.name.clone());
                // The identity arrives on the same call, and this is what anchors the record on it.
                if let Some(id) = &refreshed.id {
                    self.answered(id, refreshed.name);
                }
                Ok(match refreshed.outcome {
                    sync::Outcome::AlreadyCurrent { songs } => CopyOutcome::AlreadyCurrent(songs),
                    sync::Outcome::Imported { songs, .. } => CopyOutcome::Imported(songs),
                })
            }
            Err(error) => Err(error),
        }
    }
}

#[async_trait::async_trait]
impl Connect for Link {
    async fn status(&self) -> MachineStatus {
        MachineStatus {
            connection: self.machine.connection(),
            how: self.how(),
            pinned: self.pinned(),
            songs: self.songs(),
            can_browse: self.radar.can_browse(),
        }
    }

    async fn connect_to(&self, address: &str) -> Result<Copied, RemoteError> {
        if address.trim().is_empty() {
            return Err(RemoteError::Refused(codes::ADDRESS_NEEDED));
        }
        let url = find::normalize(address);
        self.point_at(&url, Why::AskedFor);
        self.set_pinned(true);
        tracing::info!(machine = %url, "pointed at a machine somebody typed");

        // The catalog is of whichever machine was in hand, so pointing somewhere else makes the
        // copy the wrong one. Reported rather than awaited-and-failed: a machine that is switched off
        // is the normal reason this fails, and the address is still the one this device now holds.
        let api = Api::new(&url);
        // A machine that is switched off is the normal reason this fails, and the address is still
        // the one this device now holds — so it is an outcome rather than an error.
        Ok(Copied {
            moved_to: Some(url.to_string()),
            outcome: self
                .refresh_from(&api)
                .await
                .unwrap_or(CopyOutcome::NotAnswering),
        })
    }

    async fn rescan(&self) -> Result<Scan, RemoteError> {
        // Un-pinned first and unconditionally, which is the whole reason this action exists beside
        // the address box: it is the *only* way back from "that one, whatever happens" to following
        // the network, and `find.rs` says a host offering the first owes the second.
        //
        // **Unconditional includes the branches below that move nothing.** Clearing the pin is what
        // the button is for; moving is what it used to also do. A rescan that only cleared the pin
        // when it also switched would leave a connected, pinned remote with no way to unpin at all.
        self.set_pinned(false);

        let found = self.look().await;
        if found.is_empty() {
            return Ok(Scan {
                outcome: Scanned::Nothing,
                others: Vec::new(),
            });
        }

        // **Is this sighting the machine in hand?** By identity where both ends have one, by address
        // otherwise — which is `known::choose`'s own distinction, applied to a list rather than to a
        // move. It is what stops a machine that has merely changed address being offered back to
        // itself as though it were a stranger; following that move is the background watch's job,
        // not a button's.
        //
        // The address comparison is as strings, which is safe here and would not be for an address
        // somebody typed: `Connection::address` is `Api::base`, which trims a trailing slash, and a
        // look produces `http://<IPv4>:<port>` and nothing else — `The advert names the address the
        // machine chose` shape-checks the TXT record to exactly that, and the sweep builds it
        // itself.
        let connection = self.machine.connection();
        let here = connection.address.clone();
        let mine = self.answering_id();
        let in_hand =
            |sighting: &km_api::discover::Sighting| match (mine.as_deref(), sighting.id.as_deref())
            {
                (Some(mine), Some(seen)) => mine == seen,
                _ => here.as_deref() == Some(sighting.url.as_str()),
            };

        if connection.online {
            // **Nothing moves.** This device is talking to a machine that answers, and walking off
            // it because something else also answered is a working evening interrupted by a button
            // pressed to *look*. What the press earns is a list.
            let others = offers(found.iter().filter(|sighting| !in_hand(sighting)));
            let answered = found.iter().any(in_hand);
            let outcome = match here.filter(|_| answered) {
                Some(url) => Scanned::Already(url),
                // Unreachable in practice — a non-empty `found` with no match means at least one
                // other — but expressible, and the handler prints `Nothing`'s sentence for it rather
                // than inventing a fifth case.
                None => Scanned::Kept,
            };
            tracing::info!(others = others.len(), "looked and kept the machine in hand");
            return Ok(Scan { outcome, others });
        }

        // Nothing to disturb, so this moves — to the machine this device already knows, and to no
        // other. **The rest are still offered**: a remote that takes machine A while B is on the
        // wire must not be left knowing nothing about B, which is the same fault in its second
        // location.
        //
        // **A device that knows its machine and cannot see it keeps what it has.** A press means
        // *what else is out there?*, and a machine switched off for the evening is not an
        // instruction to go and live on the one that is on. The offers below are how somebody says
        // otherwise, and pressing one of them is the saying.
        //
        // A remote that has never met a machine still takes the first, because there is nothing it
        // could be walked away from — `discover::known::anchored` is that question in one word, and
        // `choose` asks it in the same words so the button and the background watch cannot disagree.
        let record = self.known();
        let pick = if km_api::discover::known::anchored(record.as_ref()) {
            let mine = record.as_ref().and_then(|known| known.id.as_deref());
            found
                .iter()
                .find(|sighting| sighting.id.as_deref() == mine)
                .cloned()
        } else {
            found.first().cloned()
        };
        let Some(pick) = pick else {
            let others = offers(found.iter());
            tracing::info!(
                others = others.len(),
                "looked, and the machine this remote knows was not among what answered"
            );
            return Ok(Scan {
                outcome: Scanned::Kept,
                others,
            });
        };
        let others = offers(found.iter().filter(|sighting| sighting.url != pick.url));

        if here.as_deref() == Some(pick.url.as_str()) {
            return Ok(Scan {
                outcome: Scanned::Already(pick.url),
                others,
            });
        }

        self.point_at(&pick.url, Why::Adopted);
        tracing::info!(machine = %pick.url, others = others.len(), "found a machine on the network; using it instead");
        let api = Api::new(&pick.url);
        let _ = self.refresh_from(&api).await;
        Ok(Scan {
            outcome: Scanned::Using(pick.url),
            others,
        })
    }

    async fn use_found(&self, url: &str) -> Result<Copied, RemoteError> {
        // No `normalize` and no pin: this address came off a look, which produces the finished
        // form, and taking the network's answer is not the instruction `--machine` is.
        //
        // **`Chosen` and not `Adopted`.** Nothing else points this remote at a machine it does not
        // know, so pressing a row is the act that changes which machine this is — the card says
        // *chosen*, and `Link::answered` learns the new identity because only `remembered` is
        // treated as a guess.
        self.point_at(url, Why::Chosen);
        tracing::info!(machine = %url, "moved to a machine somebody picked off a rescan");
        let api = Api::new(url);
        // A machine that is switched off is the normal reason this fails, and the address is still
        // the one this device now holds — so it is an outcome rather than an error.
        Ok(Copied {
            moved_to: Some(url.to_string()),
            outcome: self
                .refresh_from(&api)
                .await
                .unwrap_or(CopyOutcome::NotAnswering),
        })
    }

    async fn refresh(&self) -> Result<Copied, RemoteError> {
        let Some(api) = self.machine.api() else {
            return Err(RemoteError::Offline(codes::NONE_FOUND));
        };
        Ok(Copied {
            // A refresh does not move, so there is no address to report and nothing for the page to
            // put in front of the sentence.
            moved_to: None,
            outcome: self.refresh_from(&api).await?,
        })
    }
}

/// Sightings turned into offers: named where a name is worth showing, sorted, one per address.
///
/// **Sorted and deduplicated here rather than trusted from the locator.** `Registry::sightings`
/// already orders by name then url, but `Sweep` answers in whatever order its probes finished — and
/// a list that reshuffled between two presses of the same button reads as a page that cannot make up
/// its mind. Deduplication is for the same reason: mDNS and a sweep can both see one machine.
///
/// **A name that is really an id is dropped.** `Registry::observed` falls back to the registry key
/// when a machine advertises nothing, and a hexadecimal id printed where a name goes is worse on a
/// card than no name at all.
fn offers<'a>(sightings: impl Iterator<Item = &'a km_api::discover::Sighting>) -> Vec<Offer> {
    let mut offers: Vec<Offer> = sightings
        .map(|sighting| Offer {
            url: sighting.url.clone(),
            name: km_api::discover::display_name(&sighting.name)
                .filter(|name| Some(*name) != sighting.id.as_deref())
                .map(str::to_owned),
        })
        .collect();
    offers.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.url.cmp(&right.url))
    });
    offers.dedup_by(|left, right| left.url == right.url);
    offers
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use crate::find::Locator;

    /// A locator that answers with whatever it was told to, and counts how often it was asked.
    struct Stub {
        machines: Vec<km_api::discover::Sighting>,
        asked: AtomicUsize,
    }

    impl Stub {
        /// One machine with an address and no identity — a build from before ids, which is the
        /// shape most of these tests want because they are about the pin and the offer.
        fn at(url: Option<&str>) -> Self {
            Self::seeing(url.into_iter().map(|url| machine(None, url)).collect())
        }

        /// Whatever is on the network, identities and all.
        fn seeing(machines: Vec<km_api::discover::Sighting>) -> Self {
            Self {
                machines,
                asked: AtomicUsize::new(0),
            }
        }
    }

    fn machine(id: Option<&str>, url: &str) -> km_api::discover::Sighting {
        km_api::discover::Sighting {
            name: "Living Room".to_owned(),
            id: id.map(str::to_owned),
            url: url.to_owned(),
        }
    }

    impl Locator for Stub {
        fn look(&self) -> Vec<km_api::discover::Sighting> {
            self.asked.fetch_add(1, Ordering::Relaxed);
            self.machines.clone()
        }
    }

    use crate::testing::Scratch;

    /// A link over a directory of this test's own.
    ///
    /// **The directory comes first so that it goes last.** Destructured into two locals, these drop
    /// in reverse order of declaration — so a caller writing `let (dir, link) = linked(..)` drops the
    /// link, and with it the mirror's connection, before the directory is removed. The other way
    /// round the removal runs while SQLite still has the file open: on Windows that fails outright,
    /// and where it does not, the closing connection writes the file back afterwards. Either way the
    /// directory survives, which is how thousands of them came to be in the temp folder.
    fn linked(name: &str, locator: Arc<dyn Locator>, pinned: bool) -> (Scratch, Link) {
        linked_knowing(name, locator, pinned, None)
    }

    fn linked_knowing(
        name: &str,
        locator: Arc<dyn Locator>,
        pinned: bool,
        known: Option<Known>,
    ) -> (Scratch, Link) {
        let dir = Scratch::new(name);
        let mirror = Mirror::open(&dir.0).expect("open the mirror");
        let link = Link::new(
            MachineClient::new(),
            Arc::new(find::Radar::new(locator)),
            dir.0.clone(),
            Arc::new(Mutex::new(mirror)),
            false,
            Origin {
                pinned,
                known,
                ..Origin::default()
            },
        );
        (dir, link)
    }

    /// Typing an address means *that one*, which is what `--machine` has always meant.
    #[tokio::test]
    async fn a_typed_address_is_normalized_and_pinned() {
        let (_dir, link) = linked("typed", Arc::new(find::NoLocator), false);
        assert!(!link.pinned());

        link.connect_to("192.168.1.5").await.expect("accepted");

        assert!(link.pinned(), "a typed address pins the machine");
        assert_eq!(
            link.machine().api().map(|api| api.base().to_owned()),
            Some("http://192.168.1.5:8177".to_owned()),
            "the port and the scheme are filled in by `find::normalize`"
        );
        assert_eq!(link.how().as_deref(), Some("asked-for"));
    }

    /// A typed address is **not** written down until something answers at it. See [`Link::point_at`].
    #[tokio::test]
    async fn a_typed_address_is_not_remembered_until_it_answers() {
        let (dir, link) = linked("unremembered", Arc::new(find::NoLocator), false);
        link.connect_to("10.0.0.9").await.expect("accepted");
        assert_eq!(
            find::known(&dir.0),
            None,
            "an address nothing has answered at must not survive a restart"
        );
    }

    /// Rescan is the way back from a pin, and the only one.
    #[tokio::test]
    async fn a_rescan_clears_the_pin() {
        let (_dir, link) = linked(
            "unpin",
            Arc::new(Stub::at(Some("http://10.0.0.2:8177"))),
            true,
        );

        let found = link.rescan().await.expect("a browse is never an error");

        assert_eq!(
            found.outcome,
            Scanned::Using("http://10.0.0.2:8177".to_owned())
        );
        assert!(found.others.is_empty(), "one machine answered");
        assert!(
            !link.pinned(),
            "rescanning goes back to following the network"
        );
        assert_eq!(link.how().as_deref(), Some("adopted"));
    }

    /// Puts a link on a machine that is answering, which nothing else in these tests can.
    ///
    /// Reachability is the event stream's to set — see [`MachineClient::set_reachability`] — and it
    /// is the one thing a rescan branches on, so a test about that branch has to say it.
    fn answering(link: &Link, address: &str) {
        link.point_at(address, Why::Remembered);
        link.machine()
            .set_reachability(true, Some(address.to_owned()), None);
    }

    /// A rescan on a remote that already has a working machine offers, and takes nothing.
    ///
    /// **The pin still comes off**, which is the half most likely to be lost in a refactor: clearing
    /// it is what the button is *for*, and moving is what it used to also do. A rescan that only
    /// unpinned when it also switched would leave a connected, pinned remote with no way back.
    #[tokio::test]
    async fn a_rescan_on_a_connected_remote_offers_and_still_unpins() {
        let (_dir, link) = linked(
            "offered",
            Arc::new(Stub::at(Some("http://10.0.0.2:8177"))),
            true,
        );
        answering(&link, "http://10.0.0.1:8177");

        let found = link.rescan().await.expect("a browse is never an error");

        // Nothing on the wire answered as the machine in hand, so there is no `Already` to give —
        // and the stranger that did answer is offered rather than dropped.
        assert_eq!(found.outcome, Scanned::Kept);
        assert_eq!(
            found
                .others
                .iter()
                .map(|o| o.url.as_str())
                .collect::<Vec<_>>(),
            ["http://10.0.0.2:8177"]
        );
        assert!(!link.pinned(), "the pin comes off whether or not it moved");
        assert_eq!(
            link.machine().api().map(|api| api.base().to_owned()),
            Some("http://10.0.0.1:8177".to_owned()),
            "a look at the network does not take the machine somebody is using"
        );
        assert_eq!(
            link.how().as_deref(),
            Some("remembered"),
            "how it was arrived at is unchanged, because it was not arrived at again"
        );
    }

    /// **The bug a live network found and none of these tests did.**
    ///
    /// A remote holding a record for one machine, opening on the address it last saw that machine
    /// at, and meeting a *different* one there must keep the machine it knows. Overwriting the
    /// record with whatever answered made the identity worthless: `choose` would then look for the
    /// stranger, find it exactly where it was, and correctly conclude nothing had moved — so the
    /// one case an id exists to catch became the one case it could not.
    #[tokio::test]
    async fn a_stranger_at_a_remembered_address_does_not_become_the_machine() {
        let record = Known::at("http://10.0.0.1:8177", Why::Remembered).answered(
            "ours",
            Some("Living Room".to_owned()),
            std::time::SystemTime::now(),
        );
        let (_dir, link) =
            linked_knowing("stranger", Arc::new(Stub::at(None)), false, Some(record));
        link.point_at("http://10.0.0.1:8177", Why::Remembered);

        link.answered("stranger", Some("Spare Box".to_owned()));

        assert_eq!(
            link.known().and_then(|known| known.id).as_deref(),
            Some("ours"),
            "a guess from yesterday must not rewrite the fact it was a guess about"
        );
        assert_eq!(
            link.answering_id().as_deref(),
            Some("stranger"),
            "...but what actually replied is recorded, which is what `choose` turns on"
        );
    }

    /// The other side of it: an address a **person** named, or one the network offered, does say
    /// which machine this is. Only `remembered` is a guess.
    #[tokio::test]
    async fn a_machine_at_an_address_somebody_named_is_the_machine() {
        let record = Known::at("http://10.0.0.1:8177", Why::Remembered).answered(
            "ours",
            None,
            std::time::SystemTime::now(),
        );
        let (_dir, link) = linked_knowing("named", Arc::new(Stub::at(None)), false, Some(record));

        link.point_at("http://10.0.0.2:8177", Why::AskedFor);
        link.answered("another", Some("Spare Box".to_owned()));

        assert_eq!(
            link.known().and_then(|known| known.id).as_deref(),
            Some("another"),
            "typing an address is somebody saying *that one*"
        );
    }

    /// And a record with no identity yet is not anchored to anything, so it takes what answers —
    /// which is how a first connection, and an address somebody typed, learns its machine at all.
    #[tokio::test]
    async fn a_record_with_no_identity_takes_the_one_that_answers() {
        let (_dir, link) = linked_knowing(
            "unanchored",
            Arc::new(Stub::at(None)),
            false,
            Some(Known::at("http://10.0.0.1:8177", Why::Remembered)),
        );
        link.point_at("http://10.0.0.1:8177", Why::Remembered);

        link.answered("first", None);

        assert_eq!(
            link.known().and_then(|known| known.id).as_deref(),
            Some("first")
        );
    }

    /// **A rescan looks for the machine this device knows before it looks for any machine.**
    ///
    /// Without that, a house with two machines answers a press of the button with whichever one the
    /// network happened to mention first — so the machine somebody has been using all evening, now
    /// on a new address, is offered as though it were a stranger while the stranger is taken. The
    /// stub deliberately lists the other one first, because the order is the part that used to
    /// decide it.
    ///
    /// **And the stranger is offered rather than discarded**, which is where this test inverted: it
    /// used to assert that everything but the pick was dropped. A remote that had nothing, took
    /// machine A and was never told about B is the reported fault in its second location.
    #[tokio::test]
    async fn a_rescan_prefers_the_machine_this_device_already_knows() {
        let record = Known::at("http://10.0.0.1:8177", Why::Remembered).answered(
            "ours",
            None,
            std::time::SystemTime::now(),
        );
        let (_dir, link) = linked_knowing(
            "prefers",
            Arc::new(Stub::seeing(vec![
                machine(Some("stranger"), "http://10.0.0.9:8177"),
                machine(Some("ours"), "http://10.0.0.2:8177"),
            ])),
            false,
            Some(record),
        );

        let found = link.rescan().await.expect("not an error");
        assert_eq!(
            found.outcome,
            Scanned::Using("http://10.0.0.2:8177".to_owned()),
            "the machine it knows, at its new address — not the first one on the wire"
        );
        assert_eq!(
            found
                .others
                .iter()
                .map(|o| o.url.as_str())
                .collect::<Vec<_>>(),
            ["http://10.0.0.9:8177"],
            "and the one it did not take is offered"
        );
        assert_eq!(
            link.machine().api().map(|api| api.base().to_owned()),
            Some("http://10.0.0.2:8177".to_owned())
        );
    }

    /// **A remote that knows its machine and cannot find it keeps what it has, and offers the rest.**
    ///
    /// The press means *what else is out there?*, and a machine switched off for the evening is not
    /// an instruction to go and live on the one that is on. The strangers are still listed, because
    /// pressing one of them is how somebody says otherwise.
    #[tokio::test]
    async fn a_rescan_that_cannot_find_the_machine_this_device_knows_offers_rather_than_moves() {
        let record = Known::at("http://10.0.0.1:8177", Why::Remembered).answered(
            "ours",
            None,
            std::time::SystemTime::now(),
        );
        let (_dir, link) = linked_knowing(
            "unfound",
            Arc::new(Stub::seeing(vec![
                machine(Some("stranger"), "http://10.0.0.9:8177"),
                machine(Some("another"), "http://10.0.0.8:8177"),
            ])),
            false,
            Some(record),
        );
        link.point_at("http://10.0.0.1:8177", Why::Remembered);

        let found = link.rescan().await.expect("not an error");
        assert_eq!(found.outcome, Scanned::Kept);
        assert_eq!(
            found
                .others
                .iter()
                .map(|offer| offer.url.as_str())
                .collect::<Vec<_>>(),
            ["http://10.0.0.8:8177", "http://10.0.0.9:8177"],
            "everything that answered is offered, since none of it was taken"
        );
        assert_eq!(
            link.machine().api().map(|api| api.base().to_owned()),
            Some("http://10.0.0.1:8177".to_owned()),
            "and the machine in hand is where it was"
        );
        assert!(!link.pinned(), "the pin still comes off in every branch");
    }

    /// A remote that has never met a machine has nothing to be walked away from, so it still takes
    /// the first — which is how a first run finds the machine in the room at all.
    #[tokio::test]
    async fn a_rescan_on_a_remote_that_knows_no_machine_still_takes_what_it_finds() {
        let (_dir, link) = linked(
            "cold",
            Arc::new(Stub::seeing(vec![machine(
                Some("stranger"),
                "http://10.0.0.9:8177",
            )])),
            false,
        );

        let found = link.rescan().await.expect("not an error");
        assert_eq!(
            found.outcome,
            Scanned::Using("http://10.0.0.9:8177".to_owned())
        );
    }

    /// Finding the machine already in hand is its own answer, not an offer to switch to itself.
    #[tokio::test]
    async fn a_rescan_that_finds_the_machine_in_hand_says_so() {
        let (_dir, link) = linked(
            "already",
            Arc::new(Stub::at(Some("http://10.0.0.1:8177"))),
            false,
        );
        answering(&link, "http://10.0.0.1:8177");

        let found = link.rescan().await.expect("not an error");
        assert_eq!(
            found.outcome,
            Scanned::Already("http://10.0.0.1:8177".to_owned())
        );
        assert!(found.others.is_empty(), "there was nothing else out there");
    }

    /// **The reported fault, as a test.**
    ///
    /// Two machines on the network and one of them in hand. The old shape collapsed the whole list
    /// to a single sighting before comparing, so this answered *"the machine you are already using
    /// is the one on the network"* and offered nothing — with a second machine two meters away.
    #[tokio::test]
    async fn a_rescan_names_the_machine_in_hand_and_offers_the_others() {
        let (_dir, link) = linked(
            "both",
            Arc::new(Stub::seeing(vec![
                machine(Some("ours"), "http://10.0.0.1:8177"),
                machine(Some("stranger"), "http://10.0.0.9:8177"),
            ])),
            false,
        );
        answering(&link, "http://10.0.0.1:8177");
        link.answered("ours", Some("Living Room".to_owned()));

        let found = link.rescan().await.expect("not an error");
        assert_eq!(
            found.outcome,
            Scanned::Already("http://10.0.0.1:8177".to_owned())
        );
        assert_eq!(
            found
                .others
                .iter()
                .map(|o| o.url.as_str())
                .collect::<Vec<_>>(),
            ["http://10.0.0.9:8177"],
            "the other machine has to be reachable from the page"
        );
        assert_eq!(
            link.machine().api().map(|api| api.base().to_owned()),
            Some("http://10.0.0.1:8177".to_owned()),
            "and nothing moved"
        );
    }

    /// The machine in hand is recognized by its id, so its own new address is never offered back.
    ///
    /// The alternative is a card saying *also on the network* about the machine printed directly
    /// above it, which is the fault `Already` was invented to avoid, arrived at from a new
    /// direction. Following a move of the same machine stays the background watch's job.
    #[tokio::test]
    async fn the_machine_in_hand_is_not_offered_back_to_itself_at_a_new_address() {
        let (_dir, link) = linked(
            "moved",
            Arc::new(Stub::seeing(vec![
                machine(Some("ours"), "http://10.0.0.2:8177"),
                machine(Some("stranger"), "http://10.0.0.9:8177"),
            ])),
            false,
        );
        answering(&link, "http://10.0.0.1:8177");
        link.answered("ours", None);

        let found = link.rescan().await.expect("not an error");
        assert_eq!(
            found.outcome,
            Scanned::Already("http://10.0.0.1:8177".to_owned())
        );
        assert_eq!(
            found
                .others
                .iter()
                .map(|o| o.url.as_str())
                .collect::<Vec<_>>(),
            ["http://10.0.0.9:8177"],
            "our own machine at its new address is not a stranger"
        );
    }

    /// Every machine that answered is offered once, in an order that does not move between presses.
    ///
    /// A `Sweep` answers in whatever order its probes finished, and mDNS and a sweep can both see
    /// one machine — so a list that reshuffled or repeated would read as a page that cannot make up
    /// its mind. The name leads the sort because it is what the card shows.
    #[tokio::test]
    async fn every_machine_that_answered_is_offered_once() {
        let mut twice = machine(Some("stranger"), "http://10.0.0.9:8177");
        twice.name = "Kitchen".to_owned();
        let mut named = machine(Some("attic"), "http://10.0.0.5:8177");
        named.name = "Attic".to_owned();
        let (_dir, link) = linked(
            "sorted",
            Arc::new(Stub::seeing(vec![
                twice.clone(),
                machine(Some("ours"), "http://10.0.0.1:8177"),
                named,
                twice,
            ])),
            false,
        );
        answering(&link, "http://10.0.0.1:8177");
        link.answered("ours", None);

        let found = link.rescan().await.expect("not an error");
        assert_eq!(
            found
                .others
                .iter()
                .map(|o| (o.name.as_deref(), o.url.as_str()))
                .collect::<Vec<_>>(),
            [
                (Some("Attic"), "http://10.0.0.5:8177"),
                (Some("Kitchen"), "http://10.0.0.9:8177"),
            ]
        );
    }

    /// Accepting an offer moves the machine, calls it chosen, and leaves it un-pinned.
    ///
    /// The pin is the point: taking what the network offered is not the instruction `--machine` is,
    /// and a remote pinned to it could never be recovered from by the background watch. **`chosen`
    /// is the other half**: nothing else points this remote at a machine it does not know, so a
    /// press on a row is the act that changes which machine this is.
    #[tokio::test]
    async fn accepting_an_offer_moves_without_pinning() {
        let (_dir, link) = linked("accepted", Arc::new(find::NoLocator), false);

        link.use_found("http://10.0.0.2:8177")
            .await
            .expect("never an error for a machine that is merely switched off");

        assert_eq!(
            link.machine().api().map(|api| api.base().to_owned()),
            Some("http://10.0.0.2:8177".to_owned())
        );
        assert!(!link.pinned(), "the network's answer is not somebody's");
        assert_eq!(link.how().as_deref(), Some("chosen"));
    }

    /// Nothing on the network is an answer, not a failure — and it must not lose the machine in hand.
    #[tokio::test]
    async fn a_rescan_that_finds_nothing_keeps_the_address_it_had() {
        let (_dir, link) = linked("fruitless", Arc::new(Stub::at(None)), false);
        link.connect_to("10.0.0.1").await.expect("accepted");

        let found = link.rescan().await.expect("not an error");
        assert_eq!(found.outcome, Scanned::Nothing);
        assert!(found.others.is_empty());
        assert_eq!(
            link.machine().api().map(|api| api.base().to_owned()),
            Some("http://10.0.0.1:8177".to_owned()),
            "a fruitless browse leaves the remote where it was"
        );
    }

    /// A build that cannot browse says so once, rather than offering an action that always fails.
    #[tokio::test]
    async fn a_locator_that_finds_nothing_by_construction_offers_no_rescan() {
        let (_dir, blind) = linked("blind", Arc::new(find::NoLocator), false);
        assert!(!blind.status().await.can_browse);

        let (_dir, real) = linked("real", Arc::new(Stub::at(None)), false);
        assert!(
            real.status().await.can_browse,
            "a real locator that happened to find nothing can still be asked again"
        );
    }

    /// Refreshing with no machine at all is a refusal with a reason, not a panic and not a silence.
    #[tokio::test]
    async fn refreshing_before_a_machine_is_found_says_so() {
        let (_dir, link) = linked("norefresh", Arc::new(find::NoLocator), false);
        assert!(matches!(link.refresh().await, Err(RemoteError::Offline(_))));
    }
}
