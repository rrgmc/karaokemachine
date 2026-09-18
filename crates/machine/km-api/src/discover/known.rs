//! The machine this device knows about, and how to decide where it is now.
//!
//! # The id is the identity; the address is a cache
//!
//! A **URL** is the one thing about a machine on a home network that does not hold still. A router hands
//! the lease to something else overnight and the address a device woke up holding is somebody's
//! printer, or nothing at all; the machine itself is on, announcing itself, and unreachable to the
//! only client that wanted it.
//!
//! The machine has a stable identity. [`super::new_instance_id`] mints eight random bytes on first
//! run, `karaokemachine` persists them at `machine.instance_id`, and they go out as the `id` TXT
//! record and as [`super::Discovery::id`]. [`Known`] is what remembers one, and [`choose`] is what
//! it is for.
//!
//! **Eight random bytes and not a UUID crate.** 64 bits minted once and never changed is exactly
//! what a UUID would be here, and the id is already published, already persisted and already
//! parsed by three programs. Widening it would break every machine that has one to gain nothing.
//!
//! # Two clocks
//!
//! This module's is `SystemTime`, because it has to survive being written to a file: the question
//! it answers is *when did this machine last answer me*, and a monotonic clock resets every reboot.
//! [`super::watch::Registry`]'s is `Instant`, because its question is *how long since this process
//! heard from it*. Mixing them would make every record read as fresh after a restart.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::watch::Observed;

/// The record's own version, so a future build can tell what it is looking at.
///
/// [`super::Discovery::v`]'s reason, one layer in: a file this build cannot read is no record
/// rather than a crash, and a caller that gets `None` looks again — which is always safe.
pub const RECORD_VERSION: u32 = 1;

/// How long a remembered address is trusted before the network is asked about it eagerly.
///
/// **Six hours, and the number is less important than the shape of being wrong about it.** The
/// event that actually moves a machine's address is not a lease expiring on a schedule; it is the
/// machine being switched off overnight and the router giving the lease away. Six hours is longer
/// than any one evening's use and shorter than a night, so a remote opened twice in an evening
/// never crosses it and one picked up the next morning always has.
///
/// Crossing it too eagerly costs one comparison against a registry that is already up to date.
/// Crossing it too late costs an evening on an address nothing answers — which is the fault this
/// whole module exists for. The asymmetry is why the exact value does not need defending.
///
/// **It changes eagerness and never correctness.** A stale record's address is still used
/// immediately; what staleness buys is that [`choose`] will take the id's real address even while
/// the old one is answering something. See rule 2 there.
pub const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// The machine this device talks to, remembered between runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    /// The record version. See [`RECORD_VERSION`].
    pub v: u32,
    /// Its instance id — the identity.
    ///
    /// **Optional, and that is what makes the migration lossless.** A record converted from one of
    /// the URL-only files that came before has none, and behaves exactly as those files behaved.
    /// So does a machine too old to advertise one. The id is filled in the first time `/discover`
    /// answers, and from then on the record is anchored rather than guessed.
    #[serde(default)]
    pub id: Option<String>,
    /// Where it was, last time anybody looked. A cache, not the identity.
    pub url: String,
    /// What its owner calls it, for a list. Display only.
    #[serde(default)]
    pub name: Option<String>,
    /// When it last answered, in seconds since the epoch. `None` for "never".
    ///
    /// Seconds rather than a formatted timestamp because this crate has no date library and must
    /// not gain one; `SystemTime::duration_since(UNIX_EPOCH)` is the idiom `km-logfile` already
    /// uses. Never having answered counts as stale, which is the right treatment of an address of
    /// unknown age.
    #[serde(default)]
    pub last_connected: Option<u64>,
    /// How this address was arrived at, worded for a person. See [`Why::label`].
    ///
    /// Advisory, and it is what lets one record serve two opposite rules: `km-remote` writes only
    /// what has *answered*, `km-admin` writes what somebody *chose* whether it answered or not.
    /// The file does not need to know which; the program that wrote it does.
    #[serde(default)]
    pub how: Option<String>,
}

impl Known {
    /// A record for a machine at `url`, arrived at in the way `how` says.
    #[must_use]
    pub fn at(url: impl Into<String>, how: Why) -> Self {
        Self {
            v: RECORD_VERSION,
            id: None,
            url: url.into(),
            name: None,
            last_connected: None,
            how: Some(how.label().to_owned()),
        }
    }

    /// Records what the machine said about itself when it answered.
    #[must_use]
    pub fn answered(
        mut self,
        id: impl Into<String>,
        name: Option<String>,
        now: SystemTime,
    ) -> Self {
        self.id = Some(id.into());
        self.name = name;
        self.last_connected = now
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|since| since.as_secs());
        self
    }
}

/// How long since this machine last answered, where it ever has.
#[must_use]
pub fn age(known: &Known, now: SystemTime) -> Option<Duration> {
    let last = known.last_connected?;
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(now.saturating_sub(last)))
}

/// Whether this device knows which machine is its own.
///
/// **The id and not the record.** A record carrying no id names an address and no machine — it is
/// what a conversion from a one-line address file produces, and what a machine too old to advertise
/// an id leaves behind — so it anchors nothing, and the module's standing rule that such a record
/// behaves exactly as a bare address does holds through [`choose`] unchanged.
///
/// A device that is anchored may follow its own machine anywhere and may take no other; one that is
/// not takes what the network offers. That is the whole of the split, and both [`choose`] and the
/// offline remote's *Rescan* ask it in these words so the two cannot drift apart.
#[must_use]
pub fn anchored(known: Option<&Known>) -> bool {
    known.is_some_and(|known| known.id.is_some())
}

/// Whether the address in this record should be treated as a hint rather than an answer.
///
/// A record that has never connected is stale: nothing knows when that address last worked.
#[must_use]
pub fn is_stale(known: &Known, now: SystemTime) -> bool {
    age(known, now).is_none_or(|age| age >= STALE_AFTER)
}

/// Reads the record at `path`.
///
/// A file that is missing, unreadable, blank, malformed or of a version this build does not know
/// all mean the same thing to a caller: there is nothing remembered, so look. Refusing to start
/// because a convenience file is corrupt would be absurd, and looking is always safe.
#[must_use]
pub fn read(path: &Path) -> Option<Known> {
    let raw = std::fs::read_to_string(path).ok()?;
    let known: Known = serde_json::from_str(&raw).ok()?;
    if known.v != RECORD_VERSION || known.url.trim().is_empty() {
        return None;
    }
    Some(known)
}

/// Writes the record to `path`.
///
/// **Through a temporary file and a rename**, so an interrupted write leaves the old record rather
/// than half of a new one. Failing to remember is a warning and never a refusal: the cost is one
/// look on the network next time.
pub fn write(path: &Path, known: &Known) {
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("tmp");
        let body = serde_json::to_string_pretty(known)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        std::fs::write(&temporary, body)?;
        std::fs::rename(&temporary, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    };
    if let Err(error) = write() {
        tracing::warn!(%error, path = %path.display(), "the machine was not remembered");
    }
}

/// Forgets the record at `path`. A file that was not there is not a failure.
pub fn forget(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "the machine was not forgotten")
        }
    }
}

/// Makes what somebody typed into a base URL.
///
/// A bare `192.168.1.5` and a bare `karaoke.local` are both what a person types, and neither is a
/// URL. The port is added only when none was given, so `192.168.1.5:9000` survives.
///
/// **Here rather than in each client**, which is where it was: `km_remote_core::find` and
/// `km_admin::machine` each carried a copy, and the second one's comment already said it was
/// keeping to "the same rule". Two spellings of a rule about ports is how a client ends up
/// searching the network for the wrong thing.
#[must_use]
pub fn normalize(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    // Look for a port after the host, not after the scheme's own colon.
    let host = with_scheme.split_once("://").map_or("", |(_, rest)| rest);
    if host.contains(':') {
        with_scheme
    } else {
        format!("{with_scheme}:{}", crate::connect::DEFAULT_PORT)
    }
}

/// Where a record lives beside a data directory.
#[must_use]
pub fn path_in(dir: &Path, file: &str) -> PathBuf {
    dir.join(file)
}

/// How an address was arrived at, worded for the card that prints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Somebody named it — a flag, or an address typed into a page. Pins.
    AskedFor,
    /// The record said so.
    Remembered,
    /// The machine this device knows is announcing itself somewhere else now.
    MachineMoved,
    /// Nothing was known, and this is what the network offered.
    Adopted,
    /// Somebody picked it off a list. `km-admin`'s rule: chosen, whether it answered or not.
    Chosen,
}

impl Why {
    /// A stable name for this reason, for a surface that has to word it itself.
    ///
    /// **Not [`Self::label`], and the difference is the point.** A label is English prose written for
    /// a log; a code is a name a translated page can look up, which is the bargain `refusal_key`
    /// already makes one wire over. The singer's remote prints this line on its machine card, and
    /// for as long as it received the label a Portuguese reader was told their machine was
    /// `remembered`.
    ///
    /// Kebab-case and never a sentence, so it survives a rewording of the sentence.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::AskedFor => "asked-for",
            Self::Remembered => "remembered",
            Self::MachineMoved => "machine-moved",
            Self::Adopted => "adopted",
            Self::Chosen => "chosen",
        }
    }

    /// What a log says. The first four are the wordings that were already on screen.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::AskedFor => "asked for",
            Self::Remembered => "remembered",
            Self::MachineMoved => "same machine, new address",
            Self::Adopted => "found on the network",
            Self::Chosen => "chosen",
        }
    }
}

/// Everything the decision below turns on.
#[derive(Debug, Clone, Copy)]
pub struct Situation<'a> {
    /// Whether somebody said *that one*. Beats everything.
    pub pinned: bool,
    /// Whether the address in hand is answering.
    pub online: bool,
    /// The address in hand, if there is one.
    pub current: Option<&'a str>,
    /// What is remembered.
    pub known: Option<&'a Known>,
    /// What the network is saying, presence included.
    pub seen: &'a [Observed],
    /// The id the machine at [`current`](Self::current) last reported for itself.
    ///
    /// `None` where nothing has answered there yet — which is a different thing from a machine that
    /// answered and gave a different id, and rule 2 turns on the difference.
    pub answering_id: Option<&'a str>,
    /// [`is_stale`] for the record, computed by the caller so this function needs no clock.
    pub stale: bool,
    /// Whether this program may point itself at a machine nobody chose, on a device that knows of
    /// none.
    ///
    /// **False for every tool that writes to a machine.** `Discovering a machine in the package
    /// builder` in docs/decisions/curation.md settles it: that tool installs packages, so a version
    /// of it that re-pointed itself at whatever answered a browse would eventually install
    /// somebody's package on the wrong machine. The offline remote may, because the worst thing that
    /// happens to it is mirroring the wrong catalog.
    ///
    /// **It answers only the question [`anchored`] leaves open.** A device that has met a machine
    /// takes no other whatever this says; what remains is a device that has met none, and there this
    /// separates a remote — which opens on whatever is in the room — from a tool, which waits to be
    /// pointed at something.
    ///
    /// **It gates adopting, never *following*.** Rule 2 — the remembered identity, wherever it is
    /// announcing itself from — runs whatever this says, because following an id is not choosing a
    /// machine: it is the machine somebody already chose, at a new address. That is the exception
    /// the curation decision grants in as many words, and it is what makes the guarantee stronger
    /// than an address rule rather than a softening of it.
    ///
    /// **The flag is here rather than at the call site**, which is the whole of why it exists. Three
    /// programs had three follows — `choose`, the package builder's `moved_to` and `km-admin`'s
    /// `follow_machine` — and the two that "may not adopt" expressed that by not calling the policy
    /// at all. Two of them then disagreed with `choose` about something else: both followed an id
    /// unconditionally whenever it appeared at a different address, where rule 2 stays put when the
    /// address in hand is answering and reports the *same* id. One function, one flag.
    pub adopts: bool,
}

/// What to do about the address in hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Leave it alone.
    Stay,
    /// Point at this instead.
    Use {
        /// The address, normalized.
        url: String,
        /// What to tell the person looking at the card.
        why: Why,
    },
}

/// Which machine this device should be talking to.
///
/// **Pure, and separate from the loop that calls it**, because the loop is a timer around a network
/// call and this is the whole of the policy. It is `find::recovery` grown an identity — that
/// function is gone, and its five cases are the last rule here, asserted unchanged.
///
/// The rules, in order:
///
/// 1. **A pin beats everything**, including a machine that has demonstrably moved. Somebody saying
///    *that one* is an instruction about an address, and following an id away from it would be
///    disobeying rather than recovering.
///
/// 2. **A machine announcing itself under the remembered id is where that machine is.** This is the
///    rule the whole module exists for and it is the only one that can fire while the connection is
///    working, because a machine that has moved has moved. It splits four ways once the addresses
///    differ:
///    - nothing answering here → take it;
///    - something answering here that reports a **different** id → take it. That address belongs to
///      another machine now, which is the overnight-DHCP case and the one only an id can see;
///    - something answering here that reports the **same** id → stay. One machine, two addresses,
///      and the one in hand works;
///    - something answering here that has not said what it is → stay while the record is fresh,
///      take it while the record is [stale](is_stale). That is the whole of what age changes.
///
/// 3. **A device that is not [anchored](anchored) does what `recovery` did**: a working connection
///    is never disturbed, nothing found changes nothing, and the same address found again is churn
///    rather than recovery. A machine on the network is otherwise taken, which is how a remote that
///    has never met one opens on whatever is in the room.
///
///    **An anchored device stops at the first line of it.** Rule 2 has already looked for its
///    machine and not found it announcing itself, and a machine that is not on the network is not a
///    reason to go and live on somebody else's: a box switched off for the evening, and a remote
///    carried to another house, are the same situation seen twice. Moving to another machine from
///    there is a person's act, through *Rescan*'s offers or the address box.
///
/// **[`Situation::adopts`] gates rule 3 and nothing else**, plus the same `Adopted` answer in the
/// nothing-in-hand tail — and only where [`anchored`] has not already refused. Rules 1 and 2 run
/// whatever it says, because a pin is an instruction and following an id is the machine somebody
/// already chose at a new address. A tool that writes to a machine passes `false`, and the two
/// answers that would point it somewhere nobody named become [`Choice::Stay`].
///
/// **An absent sighting never moves anything.** Every rule above requires a machine that is
/// announcing itself *now*; a row the registry has marked gone stays a usable cache of an address
/// and may not cause a move. See [`super::watch::Registry::removed_at`] for the Android case that
/// makes this load-bearing rather than tidy.
#[must_use]
pub fn choose(situation: &Situation<'_>) -> Choice {
    if situation.pinned {
        return Choice::Stay;
    }

    let current = situation.current.map(normalize);
    let present = |observed: &&Observed| observed.present;

    // Rule 2 — the remembered identity, wherever it is announcing itself from.
    if let Some(id) = situation.known.and_then(|known| known.id.as_deref())
        && let Some(seen) = situation
            .seen
            .iter()
            .filter(present)
            .find(|observed| observed.sighting.id.as_deref() == Some(id))
    {
        let there = normalize(&seen.sighting.url);
        if current.as_deref() == Some(there.as_str()) {
            return Choice::Stay;
        }
        let why = if current.is_none() {
            // Not a move: this device had nothing in hand and the network has just said where the
            // machine it remembers lives.
            Why::Remembered
        } else {
            Why::MachineMoved
        };
        let take = || Choice::Use {
            url: there.clone(),
            why,
        };
        if !situation.online {
            return take();
        }
        return match situation.answering_id {
            Some(answering) if answering != id => take(),
            Some(_) => Choice::Stay,
            None if situation.stale => take(),
            None => Choice::Stay,
        };
    }

    // Rule 3 — nothing anchors this device, so this is `recovery` as it was.
    if let Some(current) = current.as_deref() {
        // A device that knows which machine is its own stops here, and so does a tool that writes to
        // a machine. What is left below this line is adopting one nobody named, and the address in
        // hand is the one somebody did name.
        if situation.online || !situation.adopts || anchored(situation.known) {
            return Choice::Stay;
        }
        let Some(found) = situation.seen.iter().find(present) else {
            return Choice::Stay;
        };
        let there = normalize(&found.sighting.url);
        if current == there {
            return Choice::Stay;
        }
        return Choice::Use {
            url: there,
            why: Why::Adopted,
        };
    }

    // Nothing in hand at all. **Remembering is not adopting**, so this runs whatever `adopts` says:
    // it is the address this device wrote down after something answered at it.
    if let Some(known) = situation.known {
        return Choice::Use {
            url: normalize(&known.url),
            why: Why::Remembered,
        };
    }
    if !situation.adopts {
        return Choice::Stay;
    }
    let mut present_now = situation.seen.iter().filter(present);
    match (present_now.next(), present_now.next()) {
        (Some(only), None) => Choice::Use {
            url: normalize(&only.sighting.url),
            why: Why::Adopted,
        },
        // More than one machine and nothing remembered is a person's choice, not a policy's.
        _ => Choice::Stay,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::discover::Sighting;

    fn seen(id: Option<&str>, url: &str, present: bool) -> Observed {
        let now = Instant::now();
        Observed {
            sighting: Sighting {
                name: "Living Room".to_owned(),
                id: id.map(std::borrow::ToOwned::to_owned),
                url: url.to_owned(),
            },
            present,
            first_seen: now,
            last_seen: now,
        }
    }

    fn known(id: Option<&str>, url: &str) -> Known {
        Known {
            v: RECORD_VERSION,
            id: id.map(std::borrow::ToOwned::to_owned),
            url: url.to_owned(),
            name: None,
            last_connected: None,
            how: None,
        }
    }

    /// A `Situation` with nothing in it, so each test says only what it is about.
    fn situation<'a>(known: Option<&'a Known>, seen: &'a [Observed]) -> Situation<'a> {
        Situation {
            pinned: false,
            online: false,
            current: None,
            known,
            seen,
            answering_id: None,
            stale: false,
            // **The remote's answer, so every rule below is asserted unchanged by the flag.** The
            // three tests that pass `false` say so themselves; everything else here is about the
            // policy the offline remote follows, and this helper existing before `adopts` did is
            // what makes it the right default for them.
            adopts: true,
        }
    }

    /// Somebody saying *that one* is an instruction, and following an id away from it would be
    /// disobeying rather than recovering.
    #[test]
    fn a_pin_beats_a_machine_that_moved() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", true)];
        let situation = Situation {
            pinned: true,
            current: Some("http://192.168.1.9:8177"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(choose(&situation), Choice::Stay);
    }

    /// The plain case: the machine we know is somewhere else and this address is dead.
    #[test]
    fn the_remembered_id_at_a_new_address_wins_when_nothing_answers() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", true)];
        let situation = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(
            choose(&situation),
            Choice::Use {
                url: "http://192.168.1.42:8177".to_owned(),
                why: Why::MachineMoved,
            }
        );
    }

    /// The overnight DHCP shuffle, and the one case no amount of address-watching could reach:
    /// something *is* answering at the remembered address, and it is not our machine.
    #[test]
    fn a_different_id_answering_at_the_remembered_address_means_the_address_is_wrong() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", true)];
        let situation = Situation {
            online: true,
            current: Some("http://192.168.1.9:8177"),
            answering_id: Some("someone-else"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(
            choose(&situation),
            Choice::Use {
                url: "http://192.168.1.42:8177".to_owned(),
                why: Why::MachineMoved,
            }
        );
    }

    /// One machine reachable two ways is not a machine that moved.
    #[test]
    fn the_same_id_answering_here_is_a_reason_to_stay() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", true)];
        let situation = Situation {
            online: true,
            current: Some("http://192.168.1.9:8177"),
            answering_id: Some("abc123"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(choose(&situation), Choice::Stay);
    }

    /// A working evening is not interrupted on a suspicion.
    #[test]
    fn a_fresh_record_is_left_alone_while_something_answers() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", true)];
        let situation = Situation {
            online: true,
            current: Some("http://192.168.1.9:8177"),
            stale: false,
            ..situation(Some(&record), &network)
        };
        assert_eq!(choose(&situation), Choice::Stay);
    }

    /// ...and the whole of what age changes: the same situation, one night older.
    #[test]
    fn a_stale_record_follows_its_id_even_though_the_old_address_answers() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", true)];
        let situation = Situation {
            online: true,
            current: Some("http://192.168.1.9:8177"),
            stale: true,
            ..situation(Some(&record), &network)
        };
        assert_eq!(
            choose(&situation),
            Choice::Use {
                url: "http://192.168.1.42:8177".to_owned(),
                why: Why::MachineMoved,
            }
        );
    }

    /// **The Android-in-a-pocket case.** The registry keeps a machine it has stopped hearing from,
    /// because on a phone that has been backgrounded silence means the multicast lock was released
    /// and not that anything moved. An absent row is a cache and may never cause a move.
    #[test]
    fn an_absent_sighting_never_moves_the_connection() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", false)];
        let situation = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(choose(&situation), Choice::Stay);
    }

    // ---------------------------------------------------------------------------------------
    // `find::recovery`'s five cases, unchanged. A record with no id is a record from before this
    // module, and nothing about how it behaves may move.
    // ---------------------------------------------------------------------------------------

    /// A machine that is answering is never swapped out from under whoever is using it.
    #[test]
    fn a_working_connection_is_never_switched_away_from() {
        let network = [seen(None, "http://10.0.0.2:8177", true)];
        let situation = Situation {
            online: true,
            current: Some("http://10.0.0.1:8177"),
            ..situation(None, &network)
        };
        assert_eq!(choose(&situation), Choice::Stay);
    }

    /// A remembered address for a machine that is switched off, and the machine that is actually on
    /// the network at a different one. **The record names no machine**, so nothing is being walked
    /// away from — the test below is the same evening once one is known.
    #[test]
    fn an_unreachable_machine_is_replaced_by_one_found_on_the_network() {
        let record = known(None, "http://192.168.1.9:8177");
        let network = [seen(None, "http://192.168.1.42:8177", true)];
        let situation = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(
            choose(&situation),
            Choice::Use {
                url: "http://192.168.1.42:8177".to_owned(),
                why: Why::Adopted,
            }
        );
    }

    /// ...and the same evening on a device that knows which machine is its own. Its machine is not
    /// announcing itself, and a stranger being there instead is not a reason to go and live on it.
    #[test]
    fn an_anchored_device_does_not_take_a_stranger_while_its_own_machine_is_away() {
        let record = known(Some("ours"), "http://192.168.1.9:8177");
        let network = [seen(Some("stranger"), "http://192.168.1.42:8177", true)];
        let looking = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(choose(&looking), Choice::Stay);
        assert_eq!(
            choose(&Situation {
                stale: true,
                ..looking
            }),
            Choice::Stay,
            "and a night of it is not a way round the rule"
        );
    }

    /// The half that must survive the rule above: an anchored device still follows its own machine.
    #[test]
    fn an_anchored_device_still_follows_its_own_machine_past_a_stranger() {
        let record = known(Some("ours"), "http://192.168.1.9:8177");
        let network = [
            seen(Some("stranger"), "http://192.168.1.42:8177", true),
            seen(Some("ours"), "http://192.168.1.43:8177", true),
        ];
        let situation = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(
            choose(&situation),
            Choice::Use {
                url: "http://192.168.1.43:8177".to_owned(),
                why: Why::MachineMoved,
            }
        );
    }

    /// What anchors a device is the id, so a record with none is still a bare address and still
    /// takes what it finds.
    #[test]
    fn a_record_naming_no_machine_anchors_nothing() {
        assert!(!anchored(None));
        assert!(!anchored(Some(&known(None, "http://192.168.1.9:8177"))));
        assert!(anchored(Some(&known(
            Some("ours"),
            "http://192.168.1.9:8177"
        ))));
    }

    /// Re-pointing at the address that is already failing would be churn, not recovery.
    #[test]
    fn the_same_address_found_again_changes_nothing() {
        let network = [seen(None, "192.168.1.9", true)];
        let situation = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(None, &network)
        };
        assert_eq!(
            choose(&situation),
            Choice::Stay,
            "the browse answers a full URL and the remembered form may be bare; one address"
        );
    }

    /// A remote that has never found anything takes whatever turns up.
    #[test]
    fn a_remote_with_no_machine_at_all_takes_what_it_finds() {
        let network = [seen(None, "http://192.168.1.42:8177", true)];
        assert_eq!(
            choose(&situation(None, &network)),
            Choice::Use {
                url: "http://192.168.1.42:8177".to_owned(),
                why: Why::Adopted,
            }
        );
    }

    /// Nothing on the network is the ordinary case for a remote in a pocket, and changes nothing.
    #[test]
    fn finding_nothing_leaves_the_address_alone() {
        let situation = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(None, &[])
        };
        assert_eq!(choose(&situation), Choice::Stay);
    }

    /// With nothing in hand, the record is where to start — no waiting for the network.
    #[test]
    fn a_remembered_machine_is_used_when_there_is_nothing_in_hand() {
        let record = known(None, "http://192.168.1.9:8177");
        assert_eq!(
            choose(&situation(Some(&record), &[])),
            Choice::Use {
                url: "http://192.168.1.9:8177".to_owned(),
                why: Why::Remembered,
            }
        );
    }

    /// Two machines and nothing remembered is a person's choice, not a policy's.
    #[test]
    fn two_machines_and_nothing_remembered_is_not_a_policys_choice() {
        let network = [
            seen(Some("aaa"), "http://192.168.1.8:8177", true),
            seen(Some("zzz"), "http://192.168.1.9:8177", true),
        ];
        assert_eq!(choose(&situation(None, &network)), Choice::Stay);
    }

    // ---------------------------------------------------------------------------------------
    // `adopts`: the flag that makes one policy serve a remote and two tools that write to a machine.
    // ---------------------------------------------------------------------------------------

    /// **The decision, as a test: listing is not setting.**
    ///
    /// `Discovering a machine in the package builder` binds every tool that *writes* to a machine.
    /// A browse that turns up a stranger while the address in hand is dead is precisely the case
    /// where the remote adopts and `km-package-builder` may not — the worst thing that happens there
    /// is mirroring the wrong catalog, and here it is a package installed on somebody else's box.
    #[test]
    fn a_tool_that_may_not_adopt_stays_put_with_a_stranger_on_the_network() {
        let network = [seen(Some("stranger"), "http://192.168.1.42:8177", true)];
        let looking = Situation {
            current: Some("http://192.168.1.9:8177"),
            ..situation(None, &network)
        };

        // The remote takes it, exactly as it always has.
        assert_eq!(
            choose(&looking),
            Choice::Use {
                url: "http://192.168.1.42:8177".to_owned(),
                why: Why::Adopted,
            }
        );

        // A tool that installs packages does not.
        assert_eq!(
            choose(&Situation {
                adopts: false,
                ..looking
            }),
            Choice::Stay
        );
    }

    /// And with nothing in hand at all, which is the second `Adopted` in this function.
    #[test]
    fn a_tool_that_may_not_adopt_takes_nothing_from_an_empty_start() {
        let network = [seen(Some("stranger"), "http://192.168.1.42:8177", true)];
        assert!(matches!(
            choose(&situation(None, &network)),
            Choice::Use { .. }
        ));
        assert_eq!(
            choose(&Situation {
                adopts: false,
                ..situation(None, &network)
            }),
            Choice::Stay
        );
    }

    /// **Following an id is not adopting**, so the flag does not touch it.
    ///
    /// This is the exception the curation decision grants in as many words: the machine somebody
    /// chose, at a new address. Refusing to follow it would leave the tool installing into nothing
    /// while that machine sat two meters away announcing itself.
    #[test]
    fn a_tool_that_may_not_adopt_still_follows_the_machine_it_was_told_about() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        let network = [seen(Some("abc123"), "http://192.168.1.42:8177", true)];
        let moved = Situation {
            adopts: false,
            current: Some("http://192.168.1.9:8177"),
            ..situation(Some(&record), &network)
        };
        assert_eq!(
            choose(&moved),
            Choice::Use {
                url: "http://192.168.1.42:8177".to_owned(),
                why: Why::MachineMoved,
            }
        );
    }

    /// Nor is remembering. An address this device wrote down is one something answered at.
    #[test]
    fn nothing_in_hand_and_no_permission_to_adopt_still_uses_what_was_remembered() {
        let record = known(Some("abc123"), "http://192.168.1.9:8177");
        assert_eq!(
            choose(&Situation {
                adopts: false,
                ..situation(Some(&record), &[])
            }),
            Choice::Use {
                url: "http://192.168.1.9:8177".to_owned(),
                why: Why::Remembered,
            }
        );
    }

    // ---------------------------------------------------------------------------------------
    // The record itself.
    // ---------------------------------------------------------------------------------------

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("km-known-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_record_survives_a_restart() {
        let dir = scratch("roundtrip");
        let path = path_in(&dir, "machine.json");
        let record = Known::at("http://192.168.1.42:8177", Why::Adopted).answered(
            "abc123",
            Some("Living Room".to_owned()),
            SystemTime::now(),
        );
        write(&path, &record);

        assert_eq!(read(&path).as_ref(), Some(&record));
        forget(&path);
        assert!(read(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A record this build cannot read is no record rather than a crash, and never an empty one.
    #[test]
    fn a_record_this_build_cannot_read_is_no_record() {
        let dir = scratch("unreadable");
        let path = path_in(&dir, "machine.json");

        for bad in [
            "",
            "   ",
            "not json at all",
            r#"{"v":99,"url":"http://192.168.1.9:8177"}"#,
            r#"{"v":1,"url":"   "}"#,
        ] {
            std::fs::write(&path, bad).expect("write");
            assert!(read(&path).is_none(), "{bad:?} should read as nothing");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A write leaves the record and nothing else.
    #[test]
    fn a_write_leaves_no_temporary_file_behind() {
        let dir = scratch("tidy");
        let path = path_in(&dir, "machine.json");
        write(&path, &Known::at("http://192.168.1.9:8177", Why::Chosen));

        let left: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["machine.json".to_owned()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_record_that_never_connected_is_stale_and_six_hours_is_where_it_turns() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let never = known(Some("abc123"), "http://192.168.1.9:8177");
        assert!(is_stale(&never, now), "an address of unknown age is a hint");
        assert_eq!(age(&never, now), None);

        let at = |seconds_ago: u64| Known {
            last_connected: Some(1_000_000 - seconds_ago),
            ..known(Some("abc123"), "http://192.168.1.9:8177")
        };
        assert!(!is_stale(&at(60 * 60), now), "an hour ago is fresh");
        assert!(!is_stale(&at(STALE_AFTER.as_secs() - 1), now));
        assert!(is_stale(&at(STALE_AFTER.as_secs()), now));
        assert_eq!(age(&at(90), now), Some(Duration::from_secs(90)));
    }

    #[test]
    fn what_somebody_types_becomes_a_base_url() {
        assert_eq!(normalize("192.168.1.5"), "http://192.168.1.5:8177");
        assert_eq!(normalize("  192.168.1.5/ "), "http://192.168.1.5:8177");
        assert_eq!(normalize("192.168.1.5:9000"), "http://192.168.1.5:9000");
        assert_eq!(normalize("karaoke.local"), "http://karaoke.local:8177");
        assert_eq!(
            normalize("http://192.168.1.5:8177"),
            "http://192.168.1.5:8177"
        );
    }
}
