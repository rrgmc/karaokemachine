//! Which output device the machine plays through, and how it decides.
//!
//! The machine used to take `cpal`'s default and have no say. On a desktop that is right — follow
//! the system. On an appliance it is a fault: the ALSA card order on the box under the television
//! **moved between boots**, so the output relocated itself from the USB interface feeding the mixer
//! to an onboard jack nobody is plugged into. `RUNNING` in `/proc/asound`, a healthy service,
//! nothing wrong in the log, and silence in the room.
//!
//! So the machine names its output, by an identifier that survives a reboot.
//!
//! **The identifier is `cpal::DeviceId`, not a name.** cpal 0.18 documents it as stable across
//! application restarts and gives it `Display`/`FromStr`, so one string is the whole setting and
//! [`cpal::traits::HostTrait::device_by_id`] turns it back into a device. That is worth stating
//! because the obvious alternative — matching on the human-readable name — is what the microphone
//! `device_hint` does, and it is not good enough for something that has to be *right* rather than
//! merely helpful.
//!
//! **The policy is a pure function.** [`decide`] takes a list of devices and returns which one to
//! open, so every branch of it is tested with no audio hardware present — the same shape as
//! `should_release` in `km-app`, which governs hardware and is tested without any.
//!
//! Three things about the backends that this module exists to paper over:
//!
//! * **cpal does not classify a Linux device's interface.** `InterfaceType::Usb` is populated by
//!   WASAPI and by Android's AAudio, and the ALSA backend never sets it at all. So the USB rule
//!   below reads `/proc/asound` rather than asking cpal.
//! * **ALSA's `default` device is listed like any other.** cpal maps a hint with no IOID to
//!   `Duplex`, and `supports_output` is a direction match with no probe, so `alsa:default` comes
//!   back with `pulse`, `null`, `jack` and the rest of the routing PCMs. "Follow the system" is a
//!   synthetic entry here even so, because a sentinel that cannot collide with a device id is worth
//!   more than one that happens not to.
//! * **ALSA enumeration mixes stable and unstable identifiers.** Hint PCMs carry the card's *id
//!   string* (`CARD=Device`); a second probing pass adds `hw:CARD=<index>,DEV=<n>` with the card
//!   *index*, which is exactly the number that moved. [`rank`] prefers the former, which is what
//!   makes a saved identifier outlive the reordering this module was written for.
//! * **What arrives is alsa-lib's configuration, not the operating system's device list.** Those
//!   are different things and conflating them is what made the chooser unusable. The kernel's own
//!   enumeration is already concise — one entry per `(card, device)`, which is what `aplay -l`
//!   prints — but alsa-lib layers channel maps (`front:`, `surround51:`), plugin chains
//!   (`sysdefault:`, `dmix:`), aliases with a numbering of their own (`hdmi:`, `iec958:`) and
//!   routing PCMs (`default`, `pulse`, `null`) on top, and cpal enumerates all of it, then repeats
//!   the hardware entries under the card *index* as well as its id string. Every one of them is
//!   described with the card's own words, so a four-output box arrives as thirty-odd rows carrying a
//!   handful of names between them and "which one is my headphones" has no answer on the screen.
//!   [`device_of`] draws the line — `hw:` and `plughw:` are the kernel's devices and everything else
//!   is configuration over one of them — and [`preferred`] marks one row per device, choosing
//!   between the two spellings with the same [`rank`] the USB preference uses. Nothing is dropped;
//!   see [`OutputDevice::preferred`] for why the rest stay listed.

use std::collections::{HashMap, HashSet};

use cpal::traits::{DeviceTrait, HostTrait};

use crate::audio::AudioError;

/// The identifier meaning "follow whatever the system calls the default".
///
/// It cannot collide with a real device: every [`cpal::DeviceId`] renders as `host:device`, and this
/// has no colon. Stored rather than left absent because absent means *nobody has said*, which on
/// Linux is answered by preferring USB — so this is the only way to say "follow the system, and do
/// not prefer USB" and have it stick. Nothing but a person ever writes it.
pub const SYSTEM_DEFAULT: &str = "system";

/// Where the machine looks to find out which ALSA cards are USB.
#[cfg(target_os = "linux")]
const PROC_ASOUND_CARDS: &str = "/proc/asound/cards";

/// One output the machine could play through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    /// A [`cpal::DeviceId`] in `Display` form, or [`SYSTEM_DEFAULT`].
    pub id: String,
    /// What to show a person.
    pub name: String,
    /// Whether this is the device the system currently calls its default.
    ///
    /// **Not** the same as being the [`SYSTEM_DEFAULT`] entry, and the difference is worth keeping:
    /// this marks the real device that "follow the system" resolves to *today*, so somebody choosing
    /// the sentinel can see what they are actually choosing. The sentinel row itself is identified
    /// by its `id` and carries `false` here — marking both would say "default" twice about one
    /// thing and tell a reader nothing.
    pub system_default: bool,
    /// Whether it is a USB interface. Best effort, and on Linux the only reliable signal.
    pub usb: bool,
    /// Whether it is present right now.
    ///
    /// A saved device that has been unplugged is listed with this `false` rather than dropped, so a
    /// remote can say "USB Audio CODEC (not present)" instead of silently forgetting the choice.
    pub available: bool,
    /// Whether this is the entry to *show* for the hardware it addresses.
    ///
    /// The marked rows are the operating system's own device list — one per `(card, device)`, the
    /// same set `aplay -l` prints. The unmarked ones are alsa-lib configuration layered over them:
    /// channel maps, plugin chains, aliases and routing PCMs, each described with the card's own
    /// words, which is why an unfiltered list is thirty rows repeating a handful of names.
    ///
    /// **Marked rather than filtered, and that is the same bargain [`available`] makes.** The
    /// alternates are all openable, one of them may be what somebody deliberately saved years ago,
    /// and a machine that stopped listing a device it is willing to play through would be lying
    /// about itself to save a person some scrolling. So the list stays whole, the machine says which
    /// rows it would put in front of someone, and the client decides how much to show.
    ///
    /// [`available`]: Self::available
    pub preferred: bool,
}

/// What resolving a request actually produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    /// The identifier that was opened, or [`SYSTEM_DEFAULT`].
    pub id: String,
    /// Its name, for the log and the API.
    pub name: String,
    /// Whether something other than what was asked for had to be used.
    pub fell_back: bool,
}

/// What [`decide`] concluded, before any device is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Open this identifier.
    Use {
        /// The [`cpal::DeviceId`] to open, in `Display` form.
        id: String,
        /// Its name, for the log and the API.
        name: String,
        /// Whether this is a second choice. Always `false` here: a device that was asked for and
        /// found is not a fallback, and a fallback never lands on a named device.
        fell_back: bool,
    },
    /// Open whatever the host calls the default.
    SystemDefault {
        /// Whether the system default is a second choice — `true` only when something else was
        /// asked for and is not present.
        fell_back: bool,
    },
}

impl Outcome {
    /// Whether the request could not be honored.
    pub fn fell_back(&self) -> bool {
        match self {
            Self::Use { fell_back, .. } | Self::SystemDefault { fell_back } => *fell_back,
        }
    }
}

/// Every output the machine could use, with the system default first.
///
/// `saved` is what settings ask for, so a device that is no longer present still appears — see
/// [`OutputDevice::available`].
pub fn list_outputs(saved: Option<&str>) -> Result<Vec<OutputDevice>, AudioError> {
    let host = cpal::default_host();
    let cards = cards();

    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());

    let mut outputs = vec![OutputDevice {
        id: SYSTEM_DEFAULT.to_owned(),
        name: "Follow the system default".to_owned(),
        // See the field's own documentation: this row *is* the sentinel, and the flag marks which
        // real device the sentinel resolves to.
        system_default: false,
        usb: false,
        available: true,
        preferred: true,
    }];

    let devices = host
        .output_devices()
        .map_err(|e| AudioError::Config(e.to_string()))?;
    for device in devices {
        let Ok(id) = device.id() else { continue };
        let id = id.to_string();
        if outputs.iter().any(|existing| existing.id == id) {
            continue;
        }
        let description = device.description().ok();
        let name = description
            .as_ref()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|| id.clone());
        let usb = is_usb(&id, &name, description.as_ref(), &cards);
        outputs.push(OutputDevice {
            id: id.clone(),
            name,
            system_default: default_id.as_deref() == Some(id.as_str()),
            usb,
            available: true,
            // Decided over the whole list once it is built; a spelling cannot know on its own
            // whether it is the best one for its hardware.
            preferred: false,
        });
    }

    // A saved device that is not present is listed as itself rather than dropped. Without this the
    // API would report the machine as following the system default, which is true of what it is
    // *playing* and false about what it is *configured* to play, and the difference is the whole
    // reason a person is looking at the list.
    if let Some(saved) = saved
        && saved != SYSTEM_DEFAULT
        && !outputs.iter().any(|output| output.id == saved)
    {
        outputs.push(OutputDevice {
            id: saved.to_owned(),
            name: saved.to_owned(),
            system_default: false,
            usb: false,
            available: false,
            preferred: false,
        });
    }

    preferred(&mut outputs, saved, &cards.by_index);
    Ok(outputs)
}

/// Marks one entry per piece of hardware, and moves the marked ones to the front.
///
/// This is the answer to a list nobody could read. ALSA hands over every spelling of every output —
/// `front:`, six `surround*:`, `sysdefault:`, `hw:`, `plughw:`, `dmix:`, and then the index-named
/// pass repeating the lot — and gives each of them the *card's* description as its name, so a box
/// with an onboard card and a USB interface produces thirty-odd rows in which the same handful of
/// strings appear over and over. The reported symptom was the honest one: somebody could not find
/// their headphone jack.
///
/// Three rules, and the third is the one that is easy to get wrong:
///
/// **The offered rows are the operating system's own device list, not a guess at one.** The kernel
/// already enumerates exactly what somebody is looking for — one entry per `(card, device)`, which
/// is what `aplay -l` prints and what cpal's probing pass walks through `snd_ctl` — and the reason
/// the chooser was unusable is that this arrives buried in alsa-lib's *configuration*, which layers
/// channel maps, plugin chains and aliases over the same hardware and describes every one of them
/// with the card's own words. [`device_of`] is where the distinction is drawn.
///
/// Three rules:
///
/// 1. An identifier that addresses hardware is grouped with the other spelling of that same
///    `(card, device)` — `hw:` and `plughw:`, each under the card's id string and again under its
///    index. The group is represented by its best member under [`preference_key`], the same order
///    the USB preference uses, so the row somebody picks by hand and the row the machine would have
///    picked for itself are the same row.
/// 2. **A group holding the saved identifier is represented by the saved identifier**, whatever it
///    ranks. Somebody's recorded choice outranks a rule whose only job was to guess on their behalf,
///    and it must not be possible for the list to offer a *different* id under the name of the one
///    that is in settings. The sentinel is always marked too.
/// 3. Everything else is not marked — every plugin, alias and routing PCM. **But only when the list
///    holds hardware at all.** That condition is what keeps this from being an ALSA special case
///    written into a cross-platform function: a WASAPI or CoreAudio identifier addresses no ALSA
///    card either, so off Linux rule 3 never fires and every device stays marked exactly as it was
///    before any of this existed.
///
/// The cost of rule 3 is worth naming: on a desktop running PulseAudio or PipeWire, `pulse` is a
/// perfectly good output and is not marked anyway, because on the box this machine actually lives on
/// it is one of a dozen rows standing between somebody and their headphone socket. Nobody is left
/// without it — [`SYSTEM_DEFAULT`] follows the system, which on such a desktop *is* the sound
/// server, and the rest are one checkbox away.
///
/// Marking rather than filtering: see [`OutputDevice::preferred`].
pub fn preferred(
    outputs: &mut [OutputDevice],
    saved: Option<&str>,
    by_index: &HashMap<String, String>,
) {
    let groups: Vec<Option<(String, String)>> = outputs
        .iter()
        .map(|output| device_of(&output.id, by_index))
        .collect();
    let any_hardware = groups.iter().any(Option::is_some);

    // The best spelling of each group. Tie-broken on the identifier inside `preference_key`, so two
    // spellings that cpal happens to yield in the other order next boot cannot change which row is
    // shown; and the saved identifier beats everything, which is rule 2.
    let mut best: HashMap<&(String, String), usize> = HashMap::new();
    for (index, group) in groups.iter().enumerate() {
        let Some(group) = group.as_ref() else {
            continue;
        };
        let better = match best.get(group) {
            Some(&current) => {
                saved == Some(outputs[index].id.as_str())
                    || (saved != Some(outputs[current].id.as_str())
                        && preference_key(&outputs[index].id)
                            < preference_key(&outputs[current].id))
            }
            None => true,
        };
        if better {
            best.insert(group, index);
        }
    }
    let representatives: HashSet<usize> = best.into_values().collect();

    // Note what is deliberately *not* done here: the "system default today" mark is left where
    // enumeration put it, and is not carried onto a group's representative. On ALSA it would have
    // nowhere to go -- cpal's default output device is the `default` PCM, which is routing and
    // resolves to a card only through alsa-lib configuration this cannot read -- and on every other
    // host the default device is already its own group's representative, so there is nothing to
    // carry. Following the system is what the sentinel is for.
    for (index, output) in outputs.iter_mut().enumerate() {
        let hardware = match &groups[index] {
            Some(_) => representatives.contains(&index),
            None => !any_hardware,
        };
        output.preferred =
            output.id == SYSTEM_DEFAULT || saved == Some(output.id.as_str()) || hardware;
    }

    disambiguate(outputs);
    // Stable, so the marked rows keep enumeration order among themselves — which on ALSA is card
    // order, and on every other host is the order the platform gave them.
    outputs.sort_by_key(|output| !output.preferred);
}

/// Appends the PCM name to any marked entries that would otherwise read identically.
///
/// A safety net rather than an expected case: once the spellings are collapsed the names are
/// normally distinct, because what made them repeat was several spellings of one card and there is
/// now one of those. But two genuinely different outputs *can* carry one description — and two rows
/// with the same words is precisely the complaint this whole change answers, so it is worth spending
/// four lines to make it unable to come back a different way.
fn disambiguate(outputs: &mut [OutputDevice]) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for output in outputs.iter().filter(|output| output.preferred) {
        *counts.entry(output.name.as_str()).or_default() += 1;
    }
    let repeated: HashSet<String> = counts
        .into_iter()
        .filter(|&(_, count)| count > 1)
        .map(|(name, _)| name.to_owned())
        .collect();
    for output in outputs.iter_mut().filter(|output| output.preferred) {
        if repeated.contains(&output.name)
            && let Some((_, pcm)) = output.id.split_once(':')
        {
            output.name = format!("{} ({pcm})", output.name);
        }
    }
}

/// The piece of hardware an identifier addresses: a card and a device index on it.
///
/// **This is the operating system's own list, not a heuristic over names.** The kernel enumerates
/// sound cards and the PCM devices on each one — it is what `aplay -l` prints, what
/// `/proc/asound/cards` and `snd_ctl` describe, and what cpal's second enumeration pass walks
/// directly. That enumeration is already concise: one entry per output, four on the appliance. It
/// arrives here spelled `hw:CARD=<card>,DEV=<n>` and `plughw:CARD=<card>,DEV=<n>`, and **only those
/// two spellings are hardware.**
///
/// Everything else ALSA offers is alsa-lib configuration layered over one of them, which is why
/// there are thirty rows instead of four:
///
/// * `front:`, `surround21/40/41/50/51/71:` — the same device with a channel map applied.
/// * `sysdefault:`, `dmix:`, `dsnoop:` — the same device through a plugin chain.
/// * `hdmi:`, `iec958:` — *aliases* with a numbering of their own, so `hdmi:CARD=PCH,DEV=1` is
///   hardware device 7. That renumbering is the reason this function answers `None` for them rather
///   than trying to place them: alsa-lib holds the mapping in its card configuration and nothing
///   here can compute it. They do not need placing, because the device they alias is enumerated in
///   its own right.
/// * `default`, `pulse`, `pipewire`, `jack`, `null` — routing, and not a card at all.
///
/// So `None` means "not a piece of hardware", and that is also the answer on every host that is not
/// ALSA, where no identifier names a card and [`preferred`] therefore leaves the list alone.
///
/// `by_index` canonicalises the card. cpal's hint pass names cards by their *id string* and its
/// probing pass by their *index*, so one device arrives as both `plughw:CARD=PCH,DEV=0` and
/// `plughw:CARD=1,DEV=0`; without translating the index back, one output would be two rows. When
/// `/proc/asound/cards` could not be read the map is empty and it *is* two rows — one too many
/// rather than a wrong one, and [`disambiguate`] makes each say which card it names.
pub(crate) fn device_of(id: &str, by_index: &HashMap<String, String>) -> Option<(String, String)> {
    let (host, pcm) = id.split_once(':')?;
    if !pcm.starts_with("hw:") && !pcm.starts_with("plughw:") {
        return None;
    }
    let card = card_id_of(pcm)?;
    let card = by_index.get(card).map_or(card, String::as_str);
    let device = pcm.split_once("DEV=")?.1;
    let device = device.split(',').next().unwrap_or(device);
    Some((format!("{host}:CARD={card}"), device.to_owned()))
}

/// How much this identifier deserves to be the one offered, lower being better.
///
/// The USB preference and the list a person is shown must agree, or the machine would play through
/// a different spelling than the row somebody chose. One key, used by both [`best_usb`] and
/// [`preferred`], so they cannot drift apart.
fn preference_key(id: &str) -> (u8, &str) {
    (rank(id), id)
}

/// Opens the device the machine should be playing through.
///
/// `want` is `settings.audio.output_device`: `None` when nothing has ever been chosen, which is the
/// only time the USB preference in [`decide`] applies.
///
/// The [`cpal::Device`] is returned rather than kept anywhere, and callers are expected to drop it
/// with the stream — cpal's WASAPI backend caches an uninitialized `IAudioClient` on a device, so one
/// held for the life of the session is the same mistake the lazy opening exists to undo.
pub fn resolve(want: Option<&str>) -> Result<(cpal::Device, Chosen), AudioError> {
    let host = cpal::default_host();
    let listed = list_outputs(want)?;
    let outcome = decide(want, &listed, cfg!(target_os = "linux"));

    match outcome {
        Outcome::Use {
            id,
            name,
            fell_back,
        } => {
            let parsed = id.parse::<cpal::DeviceId>().ok();
            let device = parsed.and_then(|parsed| host.device_by_id(&parsed));
            match device {
                Some(device) => Ok((
                    device,
                    Chosen {
                        id,
                        name,
                        fell_back,
                    },
                )),
                // decide() said it was in the list, so this is the device disappearing between the
                // enumeration and the open. Rare, and the answer is the same as never having had it.
                None => system_default(&host, want, true),
            }
        }
        Outcome::SystemDefault { fell_back } => system_default(&host, want, fell_back),
    }
}

/// The host's own default, and what to call it.
fn system_default(
    host: &cpal::Host,
    want: Option<&str>,
    fell_back: bool,
) -> Result<(cpal::Device, Chosen), AudioError> {
    let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
    let name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "the system default".to_owned());
    if fell_back {
        tracing::warn!(
            wanted = want.unwrap_or(SYSTEM_DEFAULT),
            using = %name,
            "the saved output device is not present; following the system default instead. \
             The setting is left alone, so it is used again as soon as it comes back."
        );
    } else {
        tracing::info!(using = %name, "the output device follows the system default");
    }
    Ok((
        device,
        Chosen {
            id: SYSTEM_DEFAULT.to_owned(),
            name,
            fell_back,
        },
    ))
}

/// The whole selection policy, over a list rather than over hardware.
///
/// `linux` is a parameter and not a `cfg!` so that the USB preference can be tested from any
/// platform; [`resolve`] passes the real answer.
///
/// In order:
///
/// 1. A saved identifier that is present is used. Nothing else is considered — an owner who chose
///    the HDMI output meant it, and being briefly unplugged does not revoke the choice.
/// 2. A saved identifier that is absent falls through to the system default, **without rewriting
///    the setting**, so the device is picked up again the moment it returns.
/// 3. [`SYSTEM_DEFAULT`] follows the system, and is not a fallback — it was asked for.
/// 4. Nothing saved at all, on Linux, prefers a USB interface. A karaoke machine's output is nearly
///    always the USB box feeding the mixer. **This runs on every start, and nothing writes the
///    answer down** — it is a preference, not a choice. It used to run once, with the caller
///    recording what it produced; that turned "prefer whatever USB interface is here" into "always
///    use this exact PCM", which is what the preference existed to avoid on a box whose card order
///    moves between boots, and it wrote [`SYSTEM_DEFAULT`] — meaning *chosen deliberately* — on any
///    machine whose interface happened to be unplugged for that one boot. Applied afresh, an
///    interface unplugged for an evening is preferred again as soon as it is back.
/// 5. Otherwise the system default.
pub fn decide(want: Option<&str>, listed: &[OutputDevice], linux: bool) -> Outcome {
    match want {
        Some(SYSTEM_DEFAULT) => Outcome::SystemDefault { fell_back: false },
        Some(id) => match listed.iter().find(|d| d.id == id && d.available) {
            Some(device) => Outcome::Use {
                id: device.id.clone(),
                name: device.name.clone(),
                fell_back: false,
            },
            None => Outcome::SystemDefault { fell_back: true },
        },
        None if linux => match best_usb(listed) {
            Some(device) => Outcome::Use {
                id: device.id.clone(),
                name: device.name.clone(),
                fell_back: false,
            },
            None => Outcome::SystemDefault { fell_back: false },
        },
        None => Outcome::SystemDefault { fell_back: false },
    }
}

/// The USB interface the USB preference should pick, if there is one.
fn best_usb(listed: &[OutputDevice]) -> Option<&OutputDevice> {
    listed
        .iter()
        .filter(|d| d.usb && d.available && d.id != SYSTEM_DEFAULT)
        .min_by_key(|d| preference_key(&d.id))
}

/// How much this identifier deserves to be preferred, lower being better.
///
/// One ALSA card produces several PCM names for the same hardware, and they are not equally good to
/// save:
///
/// * `plughw:CARD=<name>` converts sample formats and rates in software and names the card by its
///   **id string**, which does not move between boots. Best of both.
/// * `sysdefault:CARD=<name>` and `front:CARD=<name>` are also stable, but route through more of the
///   card's own configuration than is wanted here.
/// * `hw:CARD=<name>` is stable but refuses anything the hardware cannot do natively, which for a
///   USB CODEC frequently means refusing the format cpal asked for.
/// * Anything carrying `CARD=<number>` is the enumeration pass that uses the card *index* — the
///   number that moved. Worst, and deliberately below the plain-name forms.
fn rank(id: &str) -> u8 {
    let Some((_, pcm)) = id.split_once(':') else {
        return 9;
    };
    let indexed = card_id_of(pcm).is_some_and(|card| card.parse::<u32>().is_ok());
    let base = if pcm.starts_with("plughw:") {
        0
    } else if pcm.starts_with("sysdefault:") {
        1
    } else if pcm.starts_with("front:") {
        2
    } else if pcm.starts_with("hw:") {
        3
    } else {
        4
    };
    // An index-named entry is always worse than any name-named one.
    if indexed { base + 5 } else { base }
}

/// The card an ALSA PCM name belongs to, as ALSA spells it.
///
/// `plughw:CARD=Device,DEV=0` gives `Device`; `hw:CARD=1,DEV=0` gives `1`. `None` for anything that
/// does not name a card, which includes every non-ALSA identifier.
fn card_id_of(pcm: &str) -> Option<&str> {
    let after = pcm.split_once("CARD=")?.1;
    Some(after.split(',').next().unwrap_or(after))
}

/// Whether an identifier belongs to a USB interface.
///
/// On Linux `cards` is authoritative and the name is only consulted when `/proc/asound` could not be
/// read. Elsewhere cpal's own classification is used, and the name test is the same last resort.
///
/// **The card is canonicalised through [`Cards::by_index`] first**, which fixes a fault that was
/// latent until the spellings were grouped: `/proc/asound/cards` names USB cards by their id string,
/// so `hw:CARD=0,DEV=0` — the index-named form cpal's second enumeration pass produces for the same
/// card — was not in the set and was reported as not USB. It never reached the USB preference, which
/// had the id-string spelling to prefer, but it did put the wrong mark on half the rows.
fn is_usb(
    id: &str,
    name: &str,
    description: Option<&cpal::DeviceDescription>,
    cards: &Cards,
) -> bool {
    if let Some(pcm) = id.split_once(':').map(|(_, pcm)| pcm)
        && let Some(card) = card_id_of(pcm)
    {
        let card = cards.by_index.get(card).map_or(card, String::as_str);
        if cards.usb.contains(card) {
            return true;
        }
        // A card that is known not to be USB is settled; only fall through to guessing when
        // /proc/asound told us nothing at all. `by_index` is the test for that rather than the
        // USB set, which is also empty on a box whose cards are simply all onboard.
        if !cards.by_index.is_empty() {
            return false;
        }
    }
    if description.is_some_and(|d| d.interface_type() == cpal::InterfaceType::Usb) {
        return true;
    }
    name.to_ascii_lowercase().contains("usb")
}

/// What `/proc/asound/cards` says about the machine's sound cards.
///
/// Two questions with one answer, because they are two fields of the same line and reading the file
/// twice to get them would be the only reason to keep them apart.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Cards {
    /// The id strings of the USB cards.
    usb: HashSet<String>,
    /// Card index to card id string — `"0"` to `"PCH"`.
    ///
    /// The index is the number that moves between boots, and cpal's second enumeration pass names
    /// cards by it. Translating it back is what lets [`device_of`] see `hw:CARD=0,DEV=0` and
    /// `plughw:CARD=PCH,DEV=0` as one jack.
    by_index: HashMap<String, String>,
}

/// Read by [`crate::level`], which is the only caller that needs a card's number rather than its
/// name, and exists on Linux alone.
#[cfg(target_os = "linux")]
impl Cards {
    /// Card index to card id string, which is what [`device_of`] translates through.
    pub(crate) fn by_index(&self) -> &HashMap<String, String> {
        &self.by_index
    }

    /// The index ALSA currently gives this card, by its id string.
    ///
    /// The reverse of [`Cards::by_index`], and the direction `/proc/asound/card<n>/` needs: every
    /// path under there is numbered, while everything a setting holds is named. **The number is read
    /// each time rather than kept**, because it is the one that moves between boots.
    pub(crate) fn index_of(&self, card: &str) -> Option<&str> {
        self.by_index
            .iter()
            .find(|(_, id)| id.as_str() == card)
            .map(|(index, _)| index.as_str())
    }
}

/// What this box's sound cards are, as ALSA describes them.
#[cfg(target_os = "linux")]
pub(crate) fn cards() -> Cards {
    match std::fs::read_to_string(PROC_ASOUND_CARDS) {
        Ok(text) => cards_from_proc(&text),
        Err(error) => {
            tracing::debug!(%error, "could not read {PROC_ASOUND_CARDS}; guessing USB from names");
            Cards::default()
        }
    }
}

/// Nothing to read anywhere else; cpal's own classification carries these platforms, and no
/// identifier on them names a card for [`device_of`] to translate.
#[cfg(not(target_os = "linux"))]
pub(crate) fn cards() -> Cards {
    Cards::default()
}

/// The cards named by the text of `/proc/asound/cards`.
///
/// The format is two lines per card, the first being
///
/// ```text
///  0 [Device         ]: USB-Audio - USB Audio CODEC
/// ```
///
/// where the leading number is the card index, the bracketed field is the id string that appears in
/// `CARD=`, and the word after the colon is the driver. `USB-Audio` is the USB driver, and matching
/// on it is why this does not need to stat `/proc/asound/card*/usbid` for every card in turn.
///
/// Compiled on every platform under `test`, so the parser is exercised in CI on all three rather
/// than only where the file it parses exists.
#[cfg(any(target_os = "linux", test))]
fn cards_from_proc(text: &str) -> Cards {
    let mut cards = Cards::default();
    for line in text.lines() {
        let Some((head, tail)) = line.split_once(']') else {
            continue;
        };
        let Some((index, id)) = head.split_once('[') else {
            continue;
        };
        let Some(driver) = tail.trim_start().strip_prefix(':') else {
            continue;
        };
        let (index, id) = (index.trim(), id.trim());
        // The continuation line has no index and no driver, so this also rejects it.
        if index.parse::<u32>().is_err() {
            continue;
        }
        cards.by_index.insert(index.to_owned(), id.to_owned());
        if driver.trim_start().starts_with("USB-Audio") {
            cards.usb.insert(id.to_owned());
        }
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/proc/asound/cards` for an appliance with a USB interface and on-board HDA audio, on a boot
    /// where the USB card came up first.
    ///
    /// The continuation line — the indented one carrying the bus address and the IRQ — is here
    /// because `cards_from_proc` has to skip it: it has no index, so the parse of the leading number
    /// fails and the line is dropped. A fixture of first lines only would never exercise that.
    const CARDS_USB_FIRST: &str = "\
 0 [Device         ]: USB-Audio - USB Audio CODEC
                      USB Audio CODEC at usb-0000:00:00.0-0, full speed
 1 [PCH            ]: HDA-Intel - HDA Intel PCH
                      HDA Intel PCH at 0x00000000 irq 0
";

    /// The same two cards, the other way round — which is the reordering this whole module is about.
    const CARDS_USB_SECOND: &str = "\
 0 [PCH            ]: HDA-Intel - HDA Intel PCH
                      HDA Intel PCH at 0x00000000 irq 0
 1 [Device         ]: USB-Audio - USB Audio CODEC
                      USB Audio CODEC at usb-0000:00:00.0-0, full speed
";

    fn device(id: &str, name: &str) -> OutputDevice {
        OutputDevice {
            id: id.to_owned(),
            name: name.to_owned(),
            system_default: false,
            usb: false,
            available: true,
            // What `list_outputs` hands to `preferred`, which is what decides it.
            preferred: false,
        }
    }

    fn usb(id: &str, name: &str) -> OutputDevice {
        OutputDevice {
            usb: true,
            ..device(id, name)
        }
    }

    fn system() -> OutputDevice {
        OutputDevice {
            id: SYSTEM_DEFAULT.to_owned(),
            name: "Follow the system default".to_owned(),
            system_default: true,
            usb: false,
            available: true,
            preferred: true,
        }
    }

    /// The ids of the rows a chooser would put in front of somebody, in order.
    fn offered(outputs: &[OutputDevice]) -> Vec<&str> {
        outputs
            .iter()
            .filter(|output| output.preferred)
            .map(|output| output.id.as_str())
            .collect()
    }

    #[test]
    fn the_card_id_survives_the_reordering_that_the_index_does_not() {
        // The point of the whole module in one assertion: the same USB card, under two different
        // indices, is found by the same id string both times.
        assert_eq!(
            cards_from_proc(CARDS_USB_FIRST).usb,
            cards_from_proc(CARDS_USB_SECOND).usb
        );
        assert!(cards_from_proc(CARDS_USB_FIRST).usb.contains("Device"));
    }

    #[test]
    fn only_the_usb_driver_counts_as_usb() {
        let cards = cards_from_proc(CARDS_USB_FIRST).usb;
        assert!(cards.contains("Device"));
        assert!(!cards.contains("PCH"));
        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn a_proc_file_that_makes_no_sense_yields_no_cards() {
        assert_eq!(cards_from_proc(""), Cards::default());
        assert_eq!(cards_from_proc("--- no soundcards ---"), Cards::default());
        assert_eq!(cards_from_proc("0 [Device]  USB-Audio"), Cards::default());
    }

    #[test]
    fn a_card_is_found_by_its_index_as_well_as_by_its_id() {
        // The same fact the test above asserts about ids, from the other direction: the index is
        // what moved, so it is only useful as a way *back* to the id string.
        let first = cards_from_proc(CARDS_USB_FIRST).by_index;
        assert_eq!(first.get("0").map(String::as_str), Some("Device"));
        assert_eq!(first.get("1").map(String::as_str), Some("PCH"));

        let second = cards_from_proc(CARDS_USB_SECOND).by_index;
        assert_eq!(second.get("0").map(String::as_str), Some("PCH"));
        assert_eq!(second.get("1").map(String::as_str), Some("Device"));
    }

    #[test]
    fn an_index_named_usb_card_is_still_usb() {
        // Latent until the spellings were grouped: /proc/asound names cards by id string, so the
        // index-named spelling of a USB card was not in the set and was marked as not USB.
        let cards = cards_from_proc(CARDS_USB_FIRST);
        assert!(is_usb("alsa:plughw:CARD=Device,DEV=0", "", None, &cards));
        assert!(is_usb("alsa:hw:CARD=0,DEV=0", "", None, &cards));
        assert!(!is_usb("alsa:hw:CARD=1,DEV=0", "", None, &cards));
    }

    #[test]
    fn a_card_is_named_by_its_id_or_its_index() {
        assert_eq!(card_id_of("plughw:CARD=Device,DEV=0"), Some("Device"));
        assert_eq!(card_id_of("hw:CARD=1,DEV=0"), Some("1"));
        assert_eq!(card_id_of("sysdefault:CARD=PCH"), Some("PCH"));
        assert_eq!(card_id_of("default"), None);
        assert_eq!(card_id_of("{0.0.0.00000000}.{guid}"), None);
    }

    #[test]
    fn a_named_card_beats_a_numbered_one() {
        assert!(rank("alsa:plughw:CARD=Device,DEV=0") < rank("alsa:plughw:CARD=0,DEV=0"));
        assert!(rank("alsa:hw:CARD=Device,DEV=0") < rank("alsa:hw:CARD=0,DEV=0"));
        // Even the worst name-form beats the best index-form, which is the ordering that matters.
        assert!(rank("alsa:hw:CARD=Device,DEV=0") < rank("alsa:plughw:CARD=0,DEV=0"));
    }

    #[test]
    fn plughw_is_preferred_to_bare_hw() {
        assert!(rank("alsa:plughw:CARD=Device,DEV=0") < rank("alsa:sysdefault:CARD=Device"));
        assert!(rank("alsa:sysdefault:CARD=Device") < rank("alsa:hw:CARD=Device,DEV=0"));
    }

    #[test]
    fn a_saved_device_that_is_present_is_used() {
        let listed = vec![
            system(),
            usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC"),
        ];
        assert_eq!(
            decide(Some("alsa:plughw:CARD=Device,DEV=0"), &listed, true),
            Outcome::Use {
                id: "alsa:plughw:CARD=Device,DEV=0".to_owned(),
                name: "USB Audio CODEC".to_owned(),
                fell_back: false,
            }
        );
    }

    #[test]
    fn a_saved_device_that_is_gone_falls_back_and_says_so() {
        // list_outputs keeps the saved device in the list marked unavailable, so this is the shape
        // decide() actually meets rather than a contrived one.
        let mut missing = usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC");
        missing.available = false;
        let listed = vec![
            system(),
            device("alsa:plughw:CARD=PCH,DEV=0", "HDA Intel"),
            missing,
        ];
        let outcome = decide(Some("alsa:plughw:CARD=Device,DEV=0"), &listed, true);
        assert_eq!(outcome, Outcome::SystemDefault { fell_back: true });
        assert!(outcome.fell_back());
    }

    #[test]
    fn falling_back_never_reaches_for_the_usb_rule() {
        // The USB preference must not fire here: an owner who chose the HDMI output and unplugged
        // it for one evening would otherwise be moved onto USB and never moved back.
        let listed = vec![
            system(),
            usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC"),
        ];
        assert_eq!(
            decide(Some("alsa:hw:CARD=HDMI,DEV=3"), &listed, true),
            Outcome::SystemDefault { fell_back: true }
        );
    }

    #[test]
    fn choosing_the_system_default_is_a_choice_not_a_fallback() {
        let listed = vec![
            system(),
            usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC"),
        ];
        // A USB device is present and is deliberately not chosen: the USB preference is over.
        assert_eq!(
            decide(Some(SYSTEM_DEFAULT), &listed, true),
            Outcome::SystemDefault { fell_back: false }
        );
    }

    #[test]
    fn the_first_run_prefers_usb_on_linux() {
        let listed = vec![
            system(),
            device("alsa:plughw:CARD=PCH,DEV=0", "HDA Intel PCH"),
            usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC"),
        ];
        assert_eq!(
            decide(None, &listed, true),
            Outcome::Use {
                id: "alsa:plughw:CARD=Device,DEV=0".to_owned(),
                name: "USB Audio CODEC".to_owned(),
                fell_back: false,
            }
        );
    }

    #[test]
    fn the_first_run_prefers_the_stable_spelling_of_the_same_usb_card() {
        // One card, four PCM names, only two of which survive a reboot. This is the assertion the
        // appliance failure turns on.
        let listed = vec![
            system(),
            usb("alsa:hw:CARD=0,DEV=0", "USB Audio CODEC"),
            usb("alsa:plughw:CARD=0,DEV=0", "USB Audio CODEC"),
            usb("alsa:hw:CARD=Device,DEV=0", "USB Audio CODEC"),
            usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC"),
        ];
        let Outcome::Use { id, .. } = decide(None, &listed, true) else {
            panic!("expected a device");
        };
        assert_eq!(id, "alsa:plughw:CARD=Device,DEV=0");
    }

    #[test]
    fn the_first_run_follows_the_system_off_linux() {
        let listed = vec![system(), usb("wasapi:{guid}", "Speakers (USB Audio CODEC)")];
        assert_eq!(
            decide(None, &listed, false),
            Outcome::SystemDefault { fell_back: false }
        );
    }

    #[test]
    fn the_first_run_follows_the_system_when_no_usb_is_present() {
        let listed = vec![
            system(),
            device("alsa:plughw:CARD=PCH,DEV=0", "HDA Intel PCH"),
        ];
        assert_eq!(
            decide(None, &listed, true),
            Outcome::SystemDefault { fell_back: false }
        );
    }

    #[test]
    fn an_unplugged_usb_card_does_not_win_the_first_run() {
        let mut gone = usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC");
        gone.available = false;
        let listed = vec![system(), gone];
        assert_eq!(
            decide(None, &listed, true),
            Outcome::SystemDefault { fell_back: false }
        );
    }

    #[test]
    fn proc_asound_beats_a_name_that_says_usb() {
        // "USB" in a card's name is not evidence: an onboard codec may mention it, and on the box
        // that matters the driver field is the thing that is actually true.
        let cards = cards_from_proc(CARDS_USB_FIRST);
        assert!(!is_usb(
            "alsa:plughw:CARD=PCH,DEV=0",
            "HDA Intel PCH with USB headers",
            None,
            &cards
        ));
        assert!(is_usb(
            "alsa:plughw:CARD=Device,DEV=0",
            "USB Audio CODEC",
            None,
            &cards
        ));
    }

    #[test]
    fn the_name_is_the_last_resort_when_proc_asound_said_nothing() {
        let none = Cards::default();
        assert!(is_usb(
            "alsa:plughw:CARD=Device,DEV=0",
            "USB Audio CODEC",
            None,
            &none
        ));
        assert!(!is_usb(
            "alsa:plughw:CARD=PCH,DEV=0",
            "HDA Intel PCH",
            None,
            &none
        ));
    }

    /// What ALSA and cpal 0.18.2 actually hand over for the appliance: one HDA Intel card with an
    /// analogue jack and three HDMI outputs, and one USB CODEC.
    ///
    /// Every hint PCM, then the second probing pass that repeats the hardware ones by card *index*.
    /// The plugin hints that vary between installations are trimmed; nothing else is invented. Note
    /// the names, which are the whole problem: cpal takes them from the hint description, which
    /// describes the card, so six rows here read `HDA Intel PCH, ALC3234 Analog` and five read
    /// `USB Audio CODEC`.
    fn an_alsa_box() -> Vec<OutputDevice> {
        let analog = "HDA Intel PCH, ALC3234 Analog";
        let codec = "USB Audio CODEC";
        let mut outputs = vec![
            system(),
            device("alsa:default", "Default Audio Device"),
            device("alsa:null", "Discard all samples"),
            device("alsa:pulse", "PulseAudio Sound Server"),
        ];
        // The onboard card's analogue jack, said six ways.
        for pcm in [
            "sysdefault:CARD=PCH",
            "front:CARD=PCH,DEV=0",
            "surround40:CARD=PCH,DEV=0",
            "surround51:CARD=PCH,DEV=0",
            "dmix:CARD=PCH,DEV=0",
            "hw:CARD=PCH,DEV=0",
            "plughw:CARD=PCH,DEV=0",
        ] {
            outputs.push(device(&format!("alsa:{pcm}"), analog));
        }
        // Its three HDMI outputs, under the alias namespace and under the real device numbers.
        for (alias, real, name) in [
            ("hdmi:CARD=PCH,DEV=0", 3, "HDA Intel PCH, HDMI 0"),
            ("hdmi:CARD=PCH,DEV=1", 7, "HDA Intel PCH, HDMI 1"),
            ("hdmi:CARD=PCH,DEV=2", 8, "HDA Intel PCH, HDMI 2"),
        ] {
            outputs.push(device(&format!("alsa:{alias}"), name));
            outputs.push(device(&format!("alsa:hw:CARD=PCH,DEV={real}"), name));
            outputs.push(device(&format!("alsa:plughw:CARD=PCH,DEV={real}"), name));
        }
        // The USB interface, said five ways.
        for pcm in [
            "sysdefault:CARD=Device",
            "front:CARD=Device,DEV=0",
            "dmix:CARD=Device,DEV=0",
            "hw:CARD=Device,DEV=0",
            "plughw:CARD=Device,DEV=0",
        ] {
            outputs.push(usb(&format!("alsa:{pcm}"), codec));
        }
        // cpal's second pass, naming both cards by the index that moves between boots.
        for pcm in ["hw:CARD=0,DEV=0", "plughw:CARD=0,DEV=0"] {
            outputs.push(usb(&format!("alsa:{pcm}"), codec));
        }
        for (pcm, name) in [
            ("hw:CARD=1,DEV=0", analog),
            ("plughw:CARD=1,DEV=0", analog),
            ("hw:CARD=1,DEV=3", "HDA Intel PCH, HDMI 0"),
            ("plughw:CARD=1,DEV=3", "HDA Intel PCH, HDMI 0"),
        ] {
            outputs.push(device(&format!("alsa:{pcm}"), name));
        }
        outputs
    }

    /// `/proc/asound/cards` for [`an_alsa_box`] — the USB CODEC is card 0 today.
    fn alsa_box_cards() -> HashMap<String, String> {
        cards_from_proc(CARDS_USB_FIRST).by_index
    }

    #[test]
    fn one_physical_output_is_one_row() {
        // The whole complaint, in one assertion: thirty-one rows in, six offered, and every one of
        // the six is a different piece of hardware.
        let mut outputs = an_alsa_box();
        assert_eq!(outputs.len(), 31);
        preferred(&mut outputs, None, &alsa_box_cards());
        assert_eq!(
            offered(&outputs),
            [
                SYSTEM_DEFAULT,
                "alsa:plughw:CARD=PCH,DEV=0",
                "alsa:plughw:CARD=PCH,DEV=3",
                "alsa:plughw:CARD=PCH,DEV=7",
                "alsa:plughw:CARD=PCH,DEV=8",
                "alsa:plughw:CARD=Device,DEV=0",
            ]
        );
    }

    #[test]
    fn the_offered_names_are_the_ones_somebody_is_hunting_for() {
        // The point of collapsing is not the row count, it is that the rows say different things.
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        let names: Vec<&str> = outputs
            .iter()
            .filter(|output| output.preferred)
            .map(|output| output.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Follow the system default",
                "HDA Intel PCH, ALC3234 Analog",
                "HDA Intel PCH, HDMI 0",
                "HDA Intel PCH, HDMI 1",
                "HDA Intel PCH, HDMI 2",
                "USB Audio CODEC",
            ]
        );
    }

    #[test]
    fn nothing_is_lost() {
        // Marked, never filtered. Every identifier that went in comes out, exactly once.
        let before = an_alsa_box();
        let mut after = before.clone();
        preferred(&mut after, None, &alsa_box_cards());
        let mut ids_before: Vec<&str> = before.iter().map(|o| o.id.as_str()).collect();
        let mut ids_after: Vec<&str> = after.iter().map(|o| o.id.as_str()).collect();
        ids_before.sort_unstable();
        ids_after.sort_unstable();
        assert_eq!(ids_before, ids_after);
    }

    #[test]
    fn the_row_offered_is_the_spelling_the_machine_would_pick_itself() {
        // Requirement asserted against the actual USB preference rather than against a restatement
        // of it: choosing by hand and choosing by rule must not be able to disagree.
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        assert_eq!(
            decide(None, &outputs, true),
            Outcome::Use {
                id: "alsa:plughw:CARD=Device,DEV=0".to_owned(),
                name: "USB Audio CODEC".to_owned(),
                fell_back: false,
            }
        );
    }

    #[test]
    fn every_spelling_of_one_jack_folds_into_it() {
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        for id in [
            "alsa:sysdefault:CARD=PCH",
            "alsa:front:CARD=PCH,DEV=0",
            "alsa:surround40:CARD=PCH,DEV=0",
            "alsa:surround51:CARD=PCH,DEV=0",
            "alsa:dmix:CARD=PCH,DEV=0",
            "alsa:hw:CARD=PCH,DEV=0",
        ] {
            let output = outputs.iter().find(|o| o.id == id).expect(id);
            assert!(!output.preferred, "{id} should be an alternate");
        }
    }

    #[test]
    fn the_index_named_spelling_is_the_same_card() {
        // Without `by_index` this is the bug in miniature: `CARD=0` and `CARD=Device` look like two
        // sound cards, and the USB interface is offered twice under one name.
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        for id in ["alsa:plughw:CARD=0,DEV=0", "alsa:hw:CARD=1,DEV=0"] {
            let output = outputs.iter().find(|o| o.id == id).expect(id);
            assert!(!output.preferred, "{id} should be an alternate");
        }
    }

    #[test]
    fn without_proc_asound_an_index_named_row_is_extra_rather_than_wrong() {
        // The map is empty when /proc/asound/cards could not be read. The index spelling then gets
        // a row of its own -- one too many -- and `disambiguate` makes it say which card it is,
        // rather than two rows with the same words on them.
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &HashMap::new());
        let analog: Vec<&str> = outputs
            .iter()
            .filter(|o| o.preferred && o.name.starts_with("HDA Intel PCH, ALC3234 Analog"))
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(
            analog,
            [
                "HDA Intel PCH, ALC3234 Analog (plughw:CARD=PCH,DEV=0)",
                "HDA Intel PCH, ALC3234 Analog (plughw:CARD=1,DEV=0)",
            ]
        );
    }

    #[test]
    fn a_wasapi_list_comes_back_untouched() {
        // The no-op proof. No identifier here names a card, so rule 3 never fires and every device
        // stays offered exactly as it was before any of this existed.
        let mut outputs = vec![
            system(),
            device("wasapi:{0.0.0.00000000}.{aaaa}", "Speakers (Realtek Audio)"),
            device(
                "wasapi:{0.0.0.00000000}.{bbbb}",
                "Headphones (Realtek Audio)",
            ),
            device(
                "wasapi:{0.0.0.00000000}.{cccc}",
                "Display Audio (NVIDIA HDMI)",
            ),
        ];
        let before: Vec<String> = outputs.iter().map(|o| o.id.clone()).collect();
        preferred(&mut outputs, None, &HashMap::new());
        assert!(outputs.iter().all(|o| o.preferred));
        assert_eq!(
            outputs.iter().map(|o| o.id.clone()).collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn the_saved_spelling_is_the_one_offered() {
        // A recorded choice outranks a rule whose only job was to guess on somebody's behalf -- and
        // the group must still produce exactly one row, or the jack appears twice.
        let mut outputs = an_alsa_box();
        preferred(
            &mut outputs,
            Some("alsa:hw:CARD=PCH,DEV=0"),
            &alsa_box_cards(),
        );
        assert!(offered(&outputs).contains(&"alsa:hw:CARD=PCH,DEV=0"));
        assert!(!offered(&outputs).contains(&"alsa:plughw:CARD=PCH,DEV=0"));
        assert_eq!(
            offered(&outputs)
                .iter()
                .filter(|id| id.contains("CARD=PCH,DEV=0"))
                .count(),
            1
        );
    }

    #[test]
    fn a_saved_plugin_pcm_is_offered_beside_the_hardware_rather_than_instead_of_it() {
        // Somebody who deliberately saved `front:` gets to see and keep it -- but it is genuinely
        // not the same thing as the device (it applies a channel map), nothing can prove which
        // device it resolves to, and dropping the hardware row to make space would hide an output.
        // So both are offered, and `disambiguate` makes them tell each other apart.
        let mut outputs = an_alsa_box();
        preferred(
            &mut outputs,
            Some("alsa:front:CARD=PCH,DEV=0"),
            &alsa_box_cards(),
        );
        assert!(offered(&outputs).contains(&"alsa:front:CARD=PCH,DEV=0"));
        assert!(offered(&outputs).contains(&"alsa:plughw:CARD=PCH,DEV=0"));
        let names: Vec<&str> = outputs
            .iter()
            .filter(|o| o.preferred && o.name.starts_with("HDA Intel PCH, ALC3234 Analog"))
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                // Enumeration order inside the offered band, which is ALSA's own hint order.
                "HDA Intel PCH, ALC3234 Analog (front:CARD=PCH,DEV=0)",
                "HDA Intel PCH, ALC3234 Analog (plughw:CARD=PCH,DEV=0)",
            ]
        );
    }

    #[test]
    fn a_saved_device_that_is_absent_is_still_offered() {
        // The synthetic row `list_outputs` appends for an unplugged interface. It has to be on the
        // first screen: a choice somebody has to go looking for is a choice they cannot change.
        let mut outputs = an_alsa_box();
        let mut absent = device("alsa:plughw:CARD=Gone,DEV=0", "alsa:plughw:CARD=Gone,DEV=0");
        absent.available = false;
        outputs.push(absent);
        preferred(
            &mut outputs,
            Some("alsa:plughw:CARD=Gone,DEV=0"),
            &alsa_box_cards(),
        );
        assert!(offered(&outputs).contains(&"alsa:plughw:CARD=Gone,DEV=0"));
    }

    #[test]
    fn a_routing_pcm_is_not_an_answer_to_which_one_is_my_headphones() {
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        for id in ["alsa:default", "alsa:null", "alsa:pulse"] {
            let output = outputs.iter().find(|o| o.id == id).expect(id);
            assert!(!output.preferred, "{id} should be an alternate");
        }
    }

    #[test]
    fn a_box_with_no_cards_at_all_still_offers_its_sound_server() {
        // Rule 3 fires only when the list holds hardware. A machine whose only output is PulseAudio
        // must not be told it has nothing worth offering.
        let mut outputs = vec![
            system(),
            device("alsa:default", "Default Audio Device"),
            device("alsa:pulse", "PulseAudio Sound Server"),
        ];
        preferred(&mut outputs, None, &HashMap::new());
        assert!(outputs.iter().all(|o| o.preferred));
    }

    #[test]
    fn a_saved_routing_pcm_is_still_offered() {
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, Some("alsa:null"), &alsa_box_cards());
        assert!(offered(&outputs).contains(&"alsa:null"));
    }

    #[test]
    fn the_sentinel_is_first_and_the_alternates_are_last() {
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        assert_eq!(outputs[0].id, SYSTEM_DEFAULT);
        let first_alternate = outputs
            .iter()
            .position(|o| !o.preferred)
            .expect("there are alternates");
        assert!(outputs[first_alternate..].iter().all(|o| !o.preferred));
    }

    #[test]
    fn an_alias_is_never_offered_and_never_has_to_be_placed() {
        // `hdmi:` and `iec958:` carry a numbering of alsa-lib's own -- `hdmi:CARD=PCH,DEV=1` is
        // hardware device 7 -- so nothing here could file them against the right device even if it
        // wanted to. It does not have to: they are configuration over a device the kernel already
        // enumerated, so they are simply not hardware, and the device itself keeps its own row.
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        for id in [
            "alsa:hdmi:CARD=PCH,DEV=0",
            "alsa:hdmi:CARD=PCH,DEV=1",
            "alsa:hdmi:CARD=PCH,DEV=2",
        ] {
            let output = outputs.iter().find(|o| o.id == id).expect(id);
            assert!(!output.preferred, "{id} should never represent a group");
        }
        assert_eq!(
            offered(&outputs)
                .iter()
                .filter(|id| id.starts_with("alsa:plughw:CARD=PCH,DEV="))
                .count(),
            4,
            "the analogue jack and three HDMI outputs each keep a row"
        );
    }

    #[test]
    fn two_identical_dongles_are_told_apart() {
        // cpal gives both the card's description and the card's description is the same string, so
        // without this the list says "USB Audio CODEC" twice and answers nothing.
        let mut outputs = vec![
            system(),
            usb("alsa:plughw:CARD=Device,DEV=0", "USB Audio CODEC"),
            usb("alsa:plughw:CARD=Device_1,DEV=0", "USB Audio CODEC"),
        ];
        preferred(&mut outputs, None, &HashMap::new());
        let names: Vec<&str> = outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Follow the system default",
                "USB Audio CODEC (plughw:CARD=Device,DEV=0)",
                "USB Audio CODEC (plughw:CARD=Device_1,DEV=0)",
            ]
        );
    }

    #[test]
    fn the_ordinary_machine_sees_no_parentheses() {
        // `disambiguate` touches only names that actually collide.
        let mut outputs = an_alsa_box();
        preferred(&mut outputs, None, &alsa_box_cards());
        assert!(
            outputs
                .iter()
                .filter(|o| o.preferred)
                .all(|o| !o.name.contains('(')),
        );
    }

    #[test]
    fn a_windows_default_endpoint_keeps_its_mark() {
        // Off Linux the device the system points at is its own group's representative, so the mark
        // survives collapsing with nothing having to carry it. On ALSA there is deliberately nothing
        // to carry: cpal's default output device is the `default` PCM, and which card that reaches
        // is alsa-lib configuration nothing here can read. Following the system is the sentinel's
        // job, which is why the sentinel exists as a synthetic row at all.
        let mut outputs = vec![
            system(),
            device("wasapi:{0.0.0.00000000}.{aaaa}", "Speakers (Realtek Audio)"),
            device(
                "wasapi:{0.0.0.00000000}.{bbbb}",
                "Headphones (Realtek Audio)",
            ),
        ];
        outputs[2].system_default = true;
        preferred(&mut outputs, None, &HashMap::new());
        let shown = outputs
            .iter()
            .find(|o| o.id == "wasapi:{0.0.0.00000000}.{bbbb}")
            .expect("the endpoint is offered");
        assert!(shown.preferred);
        assert!(shown.system_default);
    }

    #[test]
    fn hardware_is_a_card_and_a_device_and_nothing_else_is() {
        let cards = alsa_box_cards();
        // The two spellings the kernel's own enumeration produces are one device...
        assert_eq!(
            device_of("alsa:hw:CARD=PCH,DEV=0", &cards),
            device_of("alsa:plughw:CARD=PCH,DEV=0", &cards),
        );
        // ...under either name for the card, which is the point of `by_index`.
        assert_eq!(
            device_of("alsa:hw:CARD=1,DEV=0", &cards),
            device_of("alsa:hw:CARD=PCH,DEV=0", &cards),
        );
        assert_ne!(
            device_of("alsa:plughw:CARD=PCH,DEV=0", &cards),
            device_of("alsa:plughw:CARD=PCH,DEV=3", &cards),
        );
        // Everything alsa-lib layers on top is configuration, not a device.
        for pcm in [
            "sysdefault:CARD=PCH",
            "front:CARD=PCH,DEV=0",
            "surround51:CARD=PCH,DEV=0",
            "dmix:CARD=PCH,DEV=0",
            "hdmi:CARD=PCH,DEV=1",
            "iec958:CARD=PCH,DEV=1",
            "default",
            "pulse",
            "null",
        ] {
            assert_eq!(device_of(&format!("alsa:{pcm}"), &cards), None, "{pcm}");
        }
        // And no identifier off Linux addresses an ALSA card, which is what makes this a no-op.
        assert_eq!(device_of("wasapi:{0.0.0.00000000}.{aaaa}", &cards), None);
        assert_eq!(device_of(SYSTEM_DEFAULT, &cards), None);
    }

    #[test]
    fn the_system_default_sentinel_cannot_be_mistaken_for_a_device() {
        // Every cpal::DeviceId renders as "host:device", so the sentinel is unambiguous by
        // construction rather than by hoping nobody has a device called "system".
        assert!(!SYSTEM_DEFAULT.contains(':'));
        assert!(SYSTEM_DEFAULT.parse::<cpal::DeviceId>().is_err());
    }
}
