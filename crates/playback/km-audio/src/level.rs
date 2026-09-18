//! The level the chosen output runs at, in the operating system's own mixer.
//!
//! This is the gain between the synthesizer and the amplifier, and it is not one the audio callback
//! applies. [`crate::player::Player::fill`] multiplies by `music_volume * song_gain` and the limiter
//! bounds what leaves the callback; everything here sits downstream of both, in the sound card.
//!
//! **A percentage on such a control is a position on its range, not a loudness.** An ALSA control
//! spanning `0 - 128` with a floor of −128 dB puts −20 dB at "84%", which reads like a device that
//! is nearly all the way up and is a tenth of the voltage. So this module speaks **dB** and never
//! percent, and a caller drawing a slider draws it over the dB range the device reports.
//!
//! **A device may have no level at all, and that is not a failure.** An HDMI or S/PDIF output
//! carries the samples to a receiver that holds the volume itself, so the card has nothing to
//! attenuate and offers a switch where an analog path offers a knob. [`read`] answers `Ok(None)`
//! for one, which is a different thing from an error and is drawn differently.
//!
//! **The device is named by its identifier, never by a handle.** [`crate::device::resolve`] hands
//! back a [`cpal::Device`] to be dropped with the stream, because cpal's WASAPI backend caches an
//! uninitialized `IAudioClient` on one. Everything here takes the same `id` string the setting
//! holds, so reading a level costs no open stream and cannot outlive one.
//!
//! **What has a level is asked of the hardware where the hardware knows.** CoreAudio carries the
//! answer as a property, so the macOS half asks a device whether it has a volume and takes the
//! answer; ALSA carries no such thing, so the Linux half infers it from `/proc` and from the names
//! a driver gave its controls.
//!
//! **The policy is a pure function.** [`control_for`] picks which of a card's controls governs an
//! output and [`pcm_kind_from_proc`] says whether the output has a level to pick a control for;
//! [`volume_target`] picks which elements of a CoreAudio device its volume sits on and
//! [`coreaudio_uid`] reads the device out of an identifier. All four take text, slices and plain
//! values rather than devices, so every branch is tested on a machine with no sound card at all —
//! the same shape [`crate::device::decide`] is in.

use crate::audio::AudioError;

/// The level an output is running at, and the range it can be moved within.
///
/// Every field is dB, and `db` is what the hardware reports rather than what was asked for: a
/// control with 1 dB steps answers a request for −20.5 dB with −20 dB, and a caller that echoed the
/// request would draw a number the card disagrees with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputLevel {
    /// What it is set to now.
    pub db: f32,
    /// The quietest it goes. Often far below anything audible.
    pub db_min: f32,
    /// The loudest it goes, which is unity on every control seen so far.
    pub db_max: f32,
    /// The smallest move the control can make, or the resolution it is drawn at where the control
    /// is continuous and reports no step of its own.
    pub step_db: f32,
}

/// Whether an output has a level of its own, or hands the question to something downstream.
///
/// This and the two items below are the policy half, compiled where the parser that feeds them is:
/// on Linux, and under `test` everywhere, so the rules are exercised on all three platforms.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PcmKind {
    /// The card converts to analog and can attenuate on the way.
    Analog,
    /// The card passes samples on untouched, so the receiver holds the volume.
    Digital,
}

/// One playback control a card offers.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ControlInfo {
    /// ALSA's own name for it, such as `Master` or `PCM`.
    pub name: String,
    /// Whether it attenuates. A control with only a switch mutes and cannot be turned down.
    pub has_volume: bool,
}

/// Which control governs an output, best first.
///
/// `Master` before `PCM` because a card offering both puts the output stage on the former and the
/// stream mix on the latter, and turning the stream down leaves the output stage where it was.
/// `PCM` second covers a USB interface, which typically offers that one and nothing else.
///
/// **A preference, not a whitelist.** ALSA control names come from the driver and there is no
/// registry of them, so a list of the ones anybody has seen would silently give a machine no level
/// at all on the first card it did not recognise. What is not here falls through to the rule below.
#[cfg(any(target_os = "linux", test))]
const PREFERENCE: &[&str] = &["Master", "PCM", "Speaker", "Headphone", "Line Out", "Front"];

/// Words that mark a playback control as attenuating something other than what the machine plays.
///
/// A card's inputs are monitored *through* it, so line-in, a microphone and the PC speaker's beep
/// all carry a playback volume without being the output — and one of them being the first control a
/// card happens to list is the failure the fallback would otherwise have.
///
/// Matched as substrings and upper-cased first, so `Headphone Mic Boost` is caught by `MIC` while
/// `Headphone` on its own is not.
#[cfg(any(target_os = "linux", test))]
const NOT_THE_OUTPUT: &[&str] = &[
    "MIC", "BEEP", "CAPTURE", "LOOPBACK", "AUX", "VIDEO", "PHONE", "MODEM",
];

/// The control that governs this output, or `None` if nothing here can attenuate it.
///
/// **A digital output is refused before any control is examined**, and that is the whole reason
/// `kind` is a parameter. A card carrying both an analog jack and three HDMI outputs offers one
/// `Master` between them, which governs the jack: picking it for a stream leaving over HDMI would
/// move a knob nobody can hear and report success. There is no control that governs an HDMI output,
/// so the honest answer is that there is none.
///
/// **An unfamiliar card gets its first plausible playback control rather than nothing.** The
/// preference above is the conventional naming and covers the cards anybody is likely to meet; a
/// driver is free to call its output something else entirely, and a machine that answered *this
/// output has no level* to every such card would be wrong in the one direction that cannot be
/// noticed — the page would say the hardware has no control, which reads exactly like an HDMI
/// output and is not. What the fallback will not take is a control whose name says it governs an
/// input being monitored.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn control_for(controls: &[ControlInfo], kind: PcmKind) -> Option<&ControlInfo> {
    if kind == PcmKind::Digital {
        return None;
    }
    let known = PREFERENCE.iter().find_map(|wanted| {
        controls
            .iter()
            .find(|control| control.has_volume && control.name == *wanted)
    });
    known.or_else(|| {
        controls.iter().find(|control| {
            let name = control.name.to_ascii_uppercase();
            control.has_volume && !NOT_THE_OUTPUT.iter().any(|word| name.contains(word))
        })
    })
}

/// Whether the PCM described by a `/proc/asound/card<n>/pcm<d>p/info` file carries its own level.
///
/// The file is a block of `key: value` lines; `id` and `name` both carry the driver's word for the
/// output — `HDMI 0`, `USB Audio`, or a codec's own name for its analog pair. Either naming a
/// digital carrier settles it, and anything else is taken as analog.
///
/// Compiled everywhere rather than on Linux alone, so the parser is exercised by the test suite on
/// all three platforms. [`crate::device::cards_from_proc`] is the same arrangement for the same
/// reason.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn pcm_kind_from_proc(info: &str) -> PcmKind {
    let digital = info
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| matches!(key.trim(), "id" | "name"))
        .any(|(_, value)| {
            let value = value.to_ascii_uppercase();
            ["HDMI", "IEC958", "SPDIF", "S/PDIF", "DISPLAYPORT"]
                .iter()
                .any(|mark| value.contains(mark))
        });
    if digital {
        PcmKind::Digital
    } else {
        PcmKind::Analog
    }
}

/// Whether a macOS output carries a volume, and on which elements.
///
/// **CoreAudio answers this itself**, which is where this backend parts company with the ALSA one
/// above: a device is asked whether it has the property rather than recognised by the name of a
/// control. An HDMI device, an aggregate device and an AirPlay speaker each speak for themselves,
/// so there is no list of names here to fall behind the hardware.
///
/// Compiled under `test` everywhere, for the reason the two items above are.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VolumeSupport {
    /// Whether the device's own element carries one volume over the whole output.
    pub main: bool,
    /// Whether the left element of the stereo pair carries one.
    pub left: bool,
    /// Whether the right element does.
    pub right: bool,
    /// The pair's element numbers, which the device names for itself.
    pub pair: (u32, u32),
}

/// The element or elements one output's volume is read and written through.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolumeTarget {
    /// One property governs the whole device.
    Main,
    /// A volume per channel, and none over the two.
    Pair(u32, u32),
}

/// Which elements govern this output, or `None` where nothing on it attenuates.
///
/// **The device's own element first.** A device offering one turns the whole output down with a
/// single property, and reaching past it to write two channels would move something narrower than
/// what was asked for. An interface that trims each channel and offers nothing over the two is the
/// second case, and both of its channels move together.
///
/// **Half a pair is not a control.** A device answering for one channel and not the other would
/// leave the other where it was, which changes the balance rather than the level.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn volume_target(support: &VolumeSupport) -> Option<VolumeTarget> {
    if support.main {
        Some(VolumeTarget::Main)
    } else if support.left && support.right {
        Some(VolumeTarget::Pair(support.pair.0, support.pair.1))
    } else {
        None
    }
}

/// Whether a device's decibel reading is reporting its own control.
///
/// **A device is free to carry the property and not answer through it.** A Bluetooth headset answers
/// every decibel read with unity while its own position sits at 43 per cent, so a machine taking
/// that road would print `0.0 dB` about an output running 20 dB down, which is the reading this
/// whole module exists to make truthful. The two properties describe one control, so either
/// they agree about being at the top or the decibels are describing something else.
///
/// The margins are a hundredth of the position and a twentieth of a decibel, which is finer than
/// any control moves and coarser than the rounding of two floats that were computed apart.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn decibels_track(db: f32, db_max: f32, scalar: f32) -> bool {
    (scalar >= 0.99) == (db >= db_max - 0.05)
}

/// A position between 0 and 1 as the decibels its range makes of it.
///
/// **Linear over the range, which is CoreAudio's own answer.** Its conversion property maps 0.375
/// of a −63.5 dB range to −39.69 dB and 0.5 of a −40 dB one to −20 dB, so this reproduces what the
/// platform does with a device's position rather than inventing a second taper for it. What the
/// device does with the position afterwards is the device's, and a device that knows the difference
/// reports decibels itself and never reaches here.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn decibels_at(scalar: f32, db_min: f32, db_max: f32) -> f32 {
    db_min + scalar.clamp(0.0, 1.0) * (db_max - db_min)
}

/// Decibels as the position on that range, which is the mapping above read backwards.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn position_of(db: f32, db_min: f32, db_max: f32) -> f32 {
    if db_max <= db_min {
        return 1.0;
    }
    ((db - db_min) / (db_max - db_min)).clamp(0.0, 1.0)
}

/// The CoreAudio device UID an identifier names, or `None` for anything that is not one.
///
/// cpal renders a macOS identifier as `coreaudio:<uid>`, and a UID is the driver's own text which
/// may carry a colon of its own, so only the first one divides the two. The sentinel meaning
/// *follow the system* has no colon and is refused here as well, because it is resolved to a real
/// device before this module is reached.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn coreaudio_uid(id: &str) -> Option<&str> {
    let (host, uid) = id.split_once(':')?;
    (host == "coreaudio" && !uid.is_empty()).then_some(uid)
}

/// Whether this build can reach a mixer at all.
///
/// What it answers is a property of the build, not of the moment: a Linux binary can always ask
/// ALSA, and an Android one never can. Whether *this* device has a level is [`read`]'s answer, and
/// whether the card is plugged in is neither's.
pub fn supported() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "windows",
        target_os = "macos"
    ))
}

/// The level this output is running at, or `None` where it has none to report.
///
/// Blocking: it opens a mixer. Callers on an async runtime own getting that off it.
pub fn read(id: &str) -> Result<Option<OutputLevel>, AudioError> {
    platform::read(id)
}

/// Moves this output to `db`, and reports where it landed.
///
/// The request is clamped into the control's own range rather than refused, because a caller
/// drawing a slider from a previous reading can be a step out of date without being wrong. `None`
/// means the output has no level, exactly as it does in [`read`].
///
/// Blocking, for [`read`]'s reason.
pub fn set(id: &str, db: f32) -> Result<Option<OutputLevel>, AudioError> {
    platform::set(id, db)
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{ControlInfo, OutputLevel, PcmKind, control_for, pcm_kind_from_proc};
    use crate::audio::AudioError;
    use crate::device;

    use alsa::mixer::{MilliBel, Mixer, Selem, SelemChannelId, SelemId};

    /// The card and PCM device an identifier addresses, as ALSA spells them.
    ///
    /// `None` for anything that is not one of the kernel's own devices, which is every routing PCM
    /// and plugin chain [`device::device_of`] already declines to treat as hardware.
    fn addressed(id: &str) -> Option<(String, u32)> {
        let cards = device::cards();
        let (card, pcm) = device::device_of(id, cards.by_index())?;
        // `device_of` answers `host:CARD=<name>`, and a mixer wants the card alone.
        let card = card.split_once("CARD=")?.1.to_owned();
        Some((card, pcm.parse().ok()?))
    }

    /// What the driver says about this PCM, which is how a digital output is told from an analog one.
    ///
    /// A card whose `/proc` entry cannot be read is treated as analog: the controls are then the
    /// only evidence, and a card with no volume control answers `None` on its own.
    fn kind_of(card: &str, pcm: u32) -> PcmKind {
        let cards = device::cards();
        let Some(index) = cards.index_of(card) else {
            return PcmKind::Analog;
        };
        let path = format!("/proc/asound/card{index}/pcm{pcm}p/info");
        std::fs::read_to_string(path).map_or(PcmKind::Analog, |info| pcm_kind_from_proc(&info))
    }

    /// Every playback control the card offers, in the order ALSA lists them.
    fn controls(mixer: &Mixer) -> Vec<ControlInfo> {
        mixer
            .iter()
            .filter_map(|element| {
                let selem = Selem::new(element)?;
                // `get_id` hands back an owned id, and the name borrows from it, so it needs a name
                // of its own to outlive the statement.
                let id = selem.get_id();
                let name = id.get_name().ok()?.to_owned();
                Some(ControlInfo {
                    name,
                    has_volume: selem.has_playback_volume(),
                })
            })
            .collect()
    }

    fn open(card: &str) -> Result<Mixer, AudioError> {
        Mixer::new(&format!("hw:CARD={card}"), false).map_err(|error| {
            AudioError::Mixer(format!("could not open the mixer for {card}: {error}"))
        })
    }

    /// The level of the named control, read back from the hardware.
    fn level_of(mixer: &Mixer, name: &str) -> Result<Option<OutputLevel>, AudioError> {
        let Some(selem) = mixer.find_selem(&SelemId::new(name, 0)) else {
            return Ok(None);
        };
        let (raw_min, raw_max) = selem.get_playback_db_range();
        // `mono()` is `FrontLeft`, so one read covers a joined control and the left of a stereo
        // pair alike. A control whose two channels differ is reported by its left, which is what
        // every mixer that draws one slider does.
        let value = selem
            .get_playback_vol_db(SelemChannelId::mono())
            .map_err(|error| AudioError::Mixer(format!("could not read {name}: {error}")))?;
        // The dB range is continuous and the raw range is the number of positions in it, so the
        // step is the one divided by the other. ALSA reports no step of its own.
        let (raw_lo, raw_hi) = selem.get_playback_volume_range();
        let steps = (raw_hi - raw_lo).max(1) as f32;
        let db_min = raw_min.to_db();
        let db_max = raw_max.to_db();
        Ok(Some(OutputLevel {
            db: value.to_db(),
            db_min,
            db_max,
            step_db: ((db_max - db_min) / steps).abs(),
        }))
    }

    pub(super) fn read(id: &str) -> Result<Option<OutputLevel>, AudioError> {
        let Some((card, pcm)) = addressed(id) else {
            return Ok(None);
        };
        let mixer = open(&card)?;
        let controls = controls(&mixer);
        let Some(control) = control_for(&controls, kind_of(&card, pcm)) else {
            return Ok(None);
        };
        level_of(&mixer, &control.name)
    }

    pub(super) fn set(id: &str, db: f32) -> Result<Option<OutputLevel>, AudioError> {
        let Some((card, pcm)) = addressed(id) else {
            return Ok(None);
        };
        let mixer = open(&card)?;
        let controls = controls(&mixer);
        let Some(control) = control_for(&controls, kind_of(&card, pcm)) else {
            return Ok(None);
        };
        let name = control.name.clone();
        let Some(selem) = mixer.find_selem(&SelemId::new(&name, 0)) else {
            return Ok(None);
        };
        let (raw_min, raw_max) = selem.get_playback_db_range();
        let wanted = MilliBel::from_db(db).0.clamp(raw_min.0, raw_max.0);
        // **A request between two steps lands on the nearer one, and which one is not this code's
        // to choose.** ALSA takes a direction of −1, 0 or +1; the `alsa` crate's `Round` can spell
        // only 0 and 1, so `Floor` here is its name for 0 rather than a rounding this asked for.
        // What makes that harmless is the read-back below: the answer is what the hardware reports,
        // so no caller is told a level the card disagrees with.
        selem
            .set_playback_db_all(MilliBel(wanted), alsa::Round::Floor)
            .map_err(|error| AudioError::Mixer(format!("could not set {name}: {error}")))?;
        level_of(&mixer, &name)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::OutputLevel;
    use crate::audio::AudioError;

    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };
    use windows::core::HSTRING;

    /// WASAPI's endpoint volume is a scalar, and the dB range is reported separately.
    ///
    /// **This is the volume Windows itself shows for the endpoint**, so moving it moves the level
    /// for every application playing to that device rather than for this one alone. That is what a
    /// box under a television wants and what a laptop has to be told, which the owner's page does.
    #[expect(
        unsafe_code,
        reason = "three COM calls with no safe wrapper: CoCreateInstance, IMMDeviceEnumerator::GetDevice and IMMDevice::Activate"
    )]
    fn endpoint(id: &str) -> Result<Option<IAudioEndpointVolume>, AudioError> {
        // cpal renders a WASAPI id as `wasapi:<endpoint id>`, and the endpoint id is what
        // `IMMDeviceEnumerator::GetDevice` takes.
        let Some((host, endpoint_id)) = id.split_once(':') else {
            return Ok(None);
        };
        if host != "wasapi" {
            return Ok(None);
        }
        // SAFETY: COM is initialised on this thread by the `Com` guard the callers hold. Each call
        // takes borrowed values that outlive it and returns a refcounted interface the `windows`
        // crate releases on drop; none takes a pointer this code allocated.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|error| AudioError::Mixer(format!("no device enumerator: {error}")))?;
            let device = enumerator
                .GetDevice(&HSTRING::from(endpoint_id))
                .map_err(|error| AudioError::Mixer(format!("no such endpoint: {error}")))?;
            let volume: IAudioEndpointVolume = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|error| AudioError::Mixer(format!("no endpoint volume: {error}")))?;
            Ok(Some(volume))
        }
    }

    /// Holds COM initialised for the length of one call.
    ///
    /// Initialised per call rather than once for the process: this is reached from a blocking pool
    /// whose threads are not this crate's to set up, and `RPC_E_CHANGED_MODE` on a thread somebody
    /// else already initialised is not an error worth failing over.
    struct Com(bool);

    impl Com {
        #[expect(
            unsafe_code,
            reason = "CoInitializeEx has no safe wrapper and must run on the calling thread"
        )]
        fn enter() -> Self {
            // SAFETY: takes no pointer this code owns. A thread somebody else already initialised
            // answers RPC_E_CHANGED_MODE, which is why the result decides whether to uninitialise
            // rather than being unwrapped: this pairs its own call and no other.
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            Self(result.is_ok())
        }
    }

    impl Drop for Com {
        #[expect(
            unsafe_code,
            reason = "CoUninitialize balances the CoInitializeEx above and has no safe wrapper"
        )]
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: reached only when this guard's own `CoInitializeEx` succeeded, on the
                // thread that made it, and every interface obtained through it has been dropped by
                // now because none outlives the call that took the guard.
                unsafe { CoUninitialize() };
            }
        }
    }

    #[expect(
        unsafe_code,
        reason = "IAudioEndpointVolume's getters are unsafe and GetVolumeRange writes through three out-parameters"
    )]
    fn level_of(volume: &IAudioEndpointVolume) -> Result<OutputLevel, AudioError> {
        let (mut db_min, mut db_max, mut step_db) = (0.0f32, 0.0f32, 0.0f32);
        // SAFETY: the three pointers are to live stack locals of exactly the `f32` the signature
        // asks for, and the call writes each once and keeps none.
        unsafe {
            let db = volume
                .GetMasterVolumeLevel()
                .map_err(|error| AudioError::Mixer(format!("could not read the level: {error}")))?;
            volume
                .GetVolumeRange(&mut db_min, &mut db_max, &mut step_db)
                .map_err(|error| AudioError::Mixer(format!("no level range: {error}")))?;
            Ok(OutputLevel {
                db,
                db_min,
                db_max,
                step_db,
            })
        }
    }

    pub(super) fn read(id: &str) -> Result<Option<OutputLevel>, AudioError> {
        let _com = Com::enter();
        let Some(volume) = endpoint(id)? else {
            return Ok(None);
        };
        level_of(&volume).map(Some)
    }

    #[expect(
        unsafe_code,
        reason = "IAudioEndpointVolume::SetMasterVolumeLevel has no safe wrapper"
    )]
    pub(super) fn set(id: &str, db: f32) -> Result<Option<OutputLevel>, AudioError> {
        let _com = Com::enter();
        let Some(volume) = endpoint(id)? else {
            return Ok(None);
        };
        let current = level_of(&volume)?;
        let wanted = db.clamp(current.db_min, current.db_max);
        // SAFETY: `wanted` is a plain f32 already clamped into the range the endpoint reported, and
        // the null context means "no originating event", which is what a caller with no session to
        // exclude passes.
        unsafe {
            volume
                .SetMasterVolumeLevel(wanted, std::ptr::null())
                .map_err(|error| AudioError::Mixer(format!("could not set the level: {error}")))?;
        }
        level_of(&volume).map(Some)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{
        OutputLevel, VolumeSupport, VolumeTarget, coreaudio_uid, decibels_at, decibels_track,
        position_of, volume_target,
    };
    use crate::audio::AudioError;

    use std::ptr::{NonNull, null};

    use objc2_core_audio::{
        AudioObjectGetPropertyData, AudioObjectHasProperty, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectSetPropertyData,
        kAudioDevicePropertyPreferredChannelsForStereo, kAudioDevicePropertyVolumeDecibels,
        kAudioDevicePropertyVolumeRangeDecibels, kAudioDevicePropertyVolumeScalar,
        kAudioHardwareNoError, kAudioHardwarePropertyTranslateUIDToDevice,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, kAudioObjectUnknown,
    };
    use objc2_core_audio_types::AudioValueRange;
    use objc2_core_foundation::{CFRetained, CFString};

    /// The step reported for a control that has none of its own.
    ///
    /// CoreAudio's volume is continuous on both of the roads below, so there is no smallest move to
    /// report. A tenth of a decibel is the resolution the reading is drawn at, and a slider
    /// stepping by it therefore never lands on a value the figure beside it rounds away.
    const CONTINUOUS_STEP_DB: f32 = 0.1;

    /// Where a stereo pair sits on a device that does not name its channels.
    const STEREO_PAIR: (u32, u32) = (1, 2);

    /// Which property a device's volume is reached through.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Domain {
        /// The device reports decibels itself.
        Decibels,
        /// The device reports a position between 0 and 1, and CoreAudio converts it.
        Scalar,
    }

    /// One property of one element of the output.
    fn address(selector: u32, element: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: element,
        }
    }

    /// A status from the HAL as this module's error.
    ///
    /// The selector is reported as the four characters CoreAudio's own documentation names it by,
    /// so a status in the log can be looked up rather than only read.
    fn outcome(status: i32, selector: u32, act: &str) -> Result<(), AudioError> {
        if status == kAudioHardwareNoError {
            return Ok(());
        }
        let name: String = selector
            .to_be_bytes()
            .iter()
            .map(|b| char::from(*b))
            .collect();
        Err(AudioError::Mixer(format!(
            "could not {act} {name}: status {status}"
        )))
    }

    /// Whether this element carries this property at all.
    ///
    /// This is the whole of the digital-output question on macOS: an output that hands the volume
    /// to something downstream carries no volume property, and says so itself.
    #[expect(
        unsafe_code,
        reason = "AudioObjectHasProperty takes the address by pointer and has no safe wrapper"
    )]
    fn has(device: AudioObjectID, selector: u32, element: u32) -> bool {
        let address = address(selector, element);
        // SAFETY: the pointer is to a live stack local of exactly the struct the signature asks
        // for, and the call reads it once and keeps nothing.
        unsafe { AudioObjectHasProperty(device, NonNull::from(&address)) }
    }

    /// One property of one element, in the type that selector answers in.
    ///
    /// `value` carries the answer out, and for the two conversion selectors it carries the number
    /// to convert in as well: those answer in the buffer they read from.
    #[expect(
        unsafe_code,
        reason = "AudioObjectGetPropertyData writes the answer through a raw out-pointer and has no safe wrapper"
    )]
    fn get<T>(
        device: AudioObjectID,
        selector: u32,
        element: u32,
        value: &mut T,
    ) -> Result<(), AudioError> {
        let address = address(selector, element);
        let mut size = size_of::<T>() as u32;
        // SAFETY: `value` is a live borrow of exactly the type this selector reports and `size` is
        // its own size; `address` and `size` are live stack locals of the types the signature asks
        // for. The call writes each once and keeps no pointer. No qualifier, because every
        // property reached through here takes none.
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::from(value).cast(),
            )
        };
        outcome(status, selector, "read")
    }

    /// Writes one property of one element.
    #[expect(
        unsafe_code,
        reason = "AudioObjectSetPropertyData takes the value by pointer and has no safe wrapper"
    )]
    fn put<T>(
        device: AudioObjectID,
        selector: u32,
        element: u32,
        value: &T,
    ) -> Result<(), AudioError> {
        let address = address(selector, element);
        // SAFETY: `value` is a live borrow of exactly the type this selector takes and `size_of`
        // is its size; the call reads it once and keeps neither pointer.
        let status = unsafe {
            AudioObjectSetPropertyData(
                device,
                NonNull::from(&address),
                0,
                null(),
                size_of::<T>() as u32,
                NonNull::from(value).cast(),
            )
        };
        outcome(status, selector, "set")
    }

    /// The device this UID names, or `None` where nothing on the system answers to it.
    ///
    /// A device unplugged since the identifier was saved is not a failure: the HAL says so with
    /// `kAudioObjectUnknown`, and an output that cannot be reached has no level to report.
    #[expect(
        unsafe_code,
        reason = "the UID translation reads a CFStringRef qualifier and writes an AudioObjectID through a raw out-pointer, and has no safe wrapper"
    )]
    fn device_for(uid: &str) -> Result<Option<AudioObjectID>, AudioError> {
        let uid = CFString::from_str(uid);
        // The qualifier is the CFStringRef itself, so what the call reads is a buffer holding that
        // pointer rather than the string's own bytes.
        let uid_ref: NonNull<CFString> = CFRetained::as_ptr(&uid);
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyTranslateUIDToDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut device = kAudioObjectUnknown;
        let mut size = size_of::<AudioObjectID>() as u32;
        // SAFETY: `uid_ref` points into `uid`, which outlives the call, and the qualifier size is
        // that pointer's own; `address`, `size` and `device` are live stack locals of the types the
        // signature asks for. The call writes `device` once and keeps nothing.
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject.cast_unsigned(),
                NonNull::from(&address),
                size_of::<NonNull<CFString>>() as u32,
                std::ptr::from_ref(&uid_ref).cast(),
                NonNull::from(&mut size),
                NonNull::from(&mut device).cast(),
            )
        };
        outcome(status, kAudioHardwarePropertyTranslateUIDToDevice, "reach")?;
        Ok((device != kAudioObjectUnknown).then_some(device))
    }

    /// The two elements this device calls its stereo pair.
    fn stereo_pair(device: AudioObjectID) -> (u32, u32) {
        let mut pair = [STEREO_PAIR.0, STEREO_PAIR.1];
        match get(
            device,
            kAudioDevicePropertyPreferredChannelsForStereo,
            kAudioObjectPropertyElementMain,
            &mut pair,
        ) {
            Ok(()) => (pair[0], pair[1]),
            Err(_) => STEREO_PAIR,
        }
    }

    /// Which elements carry this selector's volume.
    fn support(device: AudioObjectID, selector: u32) -> VolumeSupport {
        let pair = stereo_pair(device);
        VolumeSupport {
            main: has(device, selector, kAudioObjectPropertyElementMain),
            left: has(device, selector, pair.0),
            right: has(device, selector, pair.1),
            pair,
        }
    }

    /// How this device's level is reached, or `None` where it has none.
    ///
    /// **Decibels before the position**, because a device reporting decibels reports the gain it is
    /// applying, where a position is only a place on a range. A device offering nothing but a
    /// position takes the second road rather than being refused: saying an output has no level
    /// where it has one is the answer nobody can tell from an HDMI output.
    ///
    /// **A decibel property that does not track the control is not one.** The check costs three
    /// reads on a page load, and what it buys is that a device whose decibels are stuck at unity is
    /// read through its position instead of being reported as wide open.
    fn resolve(device: AudioObjectID) -> Option<(Domain, VolumeTarget)> {
        if let Some(target) = volume_target(&support(device, kAudioDevicePropertyVolumeDecibels))
            && reports_decibels(device, target)
        {
            return Some((Domain::Decibels, target));
        }
        volume_target(&support(device, kAudioDevicePropertyVolumeScalar))
            .map(|target| (Domain::Scalar, target))
    }

    /// Whether this device answers its decibel property with its own level.
    ///
    /// A device with no position to compare against is taken at its word: what this catches is a
    /// device that answers both and disagrees with itself.
    fn reports_decibels(device: AudioObjectID, target: VolumeTarget) -> bool {
        let (element, _) = elements_of(target);
        let mut db = 0.0f32;
        let Ok(range) = decibel_range(device, element) else {
            return false;
        };
        if get(device, kAudioDevicePropertyVolumeDecibels, element, &mut db).is_err() {
            return false;
        }
        let mut scalar = 0.0f32;
        if get(
            device,
            kAudioDevicePropertyVolumeScalar,
            element,
            &mut scalar,
        )
        .is_err()
        {
            return true;
        }
        decibels_track(db, range.1, scalar)
    }

    /// The quietest and loudest this element goes.
    fn decibel_range(device: AudioObjectID, element: u32) -> Result<(f32, f32), AudioError> {
        let mut range = AudioValueRange {
            mMinimum: 0.0,
            mMaximum: 0.0,
        };
        get(
            device,
            kAudioDevicePropertyVolumeRangeDecibels,
            element,
            &mut range,
        )?;
        Ok((range.mMinimum as f32, range.mMaximum as f32))
    }

    /// The element to read, and the second one to write where a pair governs.
    fn elements_of(target: VolumeTarget) -> (u32, Option<u32>) {
        match target {
            VolumeTarget::Main => (kAudioObjectPropertyElementMain, None),
            VolumeTarget::Pair(left, right) => (left, Some(right)),
        }
    }

    /// What this device reports now.
    ///
    /// A pair is reported by its left, which is what every mixer drawing one slider does and what
    /// the ALSA half above reports for a stereo control.
    fn level_of(
        device: AudioObjectID,
        domain: Domain,
        target: VolumeTarget,
    ) -> Result<Option<OutputLevel>, AudioError> {
        let (element, _) = elements_of(target);
        let (db_min, db_max) = decibel_range(device, element)?;
        let db = match domain {
            Domain::Decibels => {
                let mut db = 0.0f32;
                get(device, kAudioDevicePropertyVolumeDecibels, element, &mut db)?;
                db
            }
            Domain::Scalar => {
                let mut scalar = 0.0f32;
                get(
                    device,
                    kAudioDevicePropertyVolumeScalar,
                    element,
                    &mut scalar,
                )?;
                decibels_at(scalar, db_min, db_max)
            }
        };
        let level = OutputLevel {
            db,
            db_min,
            db_max,
            step_db: CONTINUOUS_STEP_DB,
        };
        // **A range nothing can draw is no level.** A device is free to answer that silence has no
        // decibel value, and a floor of minus infinity would reach a page as a position on a slider
        // rather than as the sentence an output with no level gets.
        Ok(drawable(level))
    }

    /// The level where its own range can be drawn and moved within.
    fn drawable(level: OutputLevel) -> Option<OutputLevel> {
        let sound = level.db.is_finite() && level.db_min.is_finite() && level.db_max.is_finite();
        (sound && level.db_min < level.db_max).then_some(level)
    }

    pub(super) fn read(id: &str) -> Result<Option<OutputLevel>, AudioError> {
        let Some(uid) = coreaudio_uid(id) else {
            return Ok(None);
        };
        let Some(device) = device_for(uid)? else {
            return Ok(None);
        };
        let Some((domain, target)) = resolve(device) else {
            return Ok(None);
        };
        level_of(device, domain, target)
    }

    pub(super) fn set(id: &str, db: f32) -> Result<Option<OutputLevel>, AudioError> {
        let Some(uid) = coreaudio_uid(id) else {
            return Ok(None);
        };
        let Some(device) = device_for(uid)? else {
            return Ok(None);
        };
        let Some((domain, target)) = resolve(device) else {
            return Ok(None);
        };
        let Some(current) = level_of(device, domain, target)? else {
            return Ok(None);
        };
        let wanted = db.clamp(current.db_min, current.db_max);
        let (first, second) = elements_of(target);
        for element in std::iter::once(first).chain(second) {
            match domain {
                Domain::Decibels => {
                    put(device, kAudioDevicePropertyVolumeDecibels, element, &wanted)?;
                }
                Domain::Scalar => {
                    let scalar = position_of(wanted, current.db_min, current.db_max);
                    put(device, kAudioDevicePropertyVolumeScalar, element, &scalar)?;
                }
            }
        }
        // **The road is chosen again, because the device has moved.** Which one a device is on is
        // read out of what it answers now, and an output sitting at the top of its range answers
        // the same on both: a stuck decibel property is only visible once the control is below
        // unity. So a write from full scale is made through the road that looked right and read
        // back through the road that is.
        let Some((domain, target)) = resolve(device) else {
            return Ok(None);
        };
        // What comes back is what the hardware reports rather than what was asked for, which is the
        // rule `OutputLevel::db` states and the other two backends follow.
        level_of(device, domain, target)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod platform {
    use super::OutputLevel;
    use crate::audio::AudioError;

    /// Nothing to ask, so nothing is reported and nothing fails.
    ///
    /// The same signature as the two real ones, so the callers above are not `cfg`-split. What
    /// keeps this from looking like a device with no level is [`super::supported`], which the host
    /// reads to decide whether to offer the control at all.
    pub(super) fn read(_id: &str) -> Result<Option<OutputLevel>, AudioError> {
        Ok(None)
    }

    pub(super) fn set(_id: &str, _db: f32) -> Result<Option<OutputLevel>, AudioError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(name: &str, has_volume: bool) -> ControlInfo {
        ControlInfo {
            name: name.to_owned(),
            has_volume,
        }
    }

    /// A USB interface typically offers one control, and it is the one to take.
    #[test]
    fn a_card_with_one_control_offers_it() {
        let controls = vec![control("PCM", true)];
        let chosen = control_for(&controls, PcmKind::Analog).expect("a control");
        assert_eq!(chosen.name, "PCM");
    }

    /// An onboard codec offers several, and `Master` governs the output stage the rest feed.
    #[test]
    fn master_is_preferred_over_the_stream_mix() {
        let controls = vec![
            control("Headphone", true),
            control("Speaker", true),
            control("PCM", true),
            control("Master", true),
            control("Line Out", true),
        ];
        let chosen = control_for(&controls, PcmKind::Analog).expect("a control");
        assert_eq!(chosen.name, "Master");
    }

    /// A control that only mutes is not a control that turns down.
    #[test]
    fn a_switch_without_a_volume_is_not_offered() {
        let controls = vec![control("Master", false), control("PCM", true)];
        let chosen = control_for(&controls, PcmKind::Analog).expect("a control");
        assert_eq!(chosen.name, "PCM");
    }

    /// The case the whole `kind` parameter exists for: the card has a `Master`, and it governs the
    /// analog jack rather than the HDMI stream leaving it.
    #[test]
    fn a_digital_output_is_refused_before_any_control_is_looked_at() {
        let controls = vec![control("Master", true), control("PCM", true)];
        assert!(control_for(&controls, PcmKind::Digital).is_none());
    }

    #[test]
    fn a_card_offering_only_input_monitoring_has_no_control() {
        // Every one of these carries a playback volume and none of them is the output: a card is
        // monitored *through*, so its inputs have playback levels of their own.
        let controls = vec![
            control("Beep", true),
            control("Capture", true),
            control("Mic Boost", true),
            control("Aux", true),
        ];
        assert!(control_for(&controls, PcmKind::Analog).is_none());
    }

    /// The rule that keeps this working on hardware nobody here has seen.
    ///
    /// ALSA control names come from the driver and there is no registry of them, so a card naming
    /// its output something unanticipated has to get that control rather than be told it has none.
    #[test]
    fn an_unfamiliar_output_control_is_taken_rather_than_refused() {
        for name in ["Digital", "Analogue Out", "DAC", "Wave"] {
            let controls = vec![control(name, true)];
            let chosen = control_for(&controls, PcmKind::Analog)
                .unwrap_or_else(|| panic!("{name} should be taken as the output"));
            assert_eq!(chosen.name, name);
        }
    }

    /// ...and the fallback still will not reach past a conventional name to do it.
    #[test]
    fn a_familiar_name_still_wins_over_an_unfamiliar_one_listed_first() {
        let controls = vec![control("Digital", true), control("Master", true)];
        let chosen = control_for(&controls, PcmKind::Analog).expect("a control");
        assert_eq!(chosen.name, "Master");
    }

    /// The fallback picks an output, not the first thing with a volume on it.
    #[test]
    fn the_fallback_steps_over_an_input_to_reach_the_output() {
        let controls = vec![
            control("Beep", true),
            control("Headphone Mic Boost", true),
            control("Digital", true),
        ];
        let chosen = control_for(&controls, PcmKind::Analog).expect("a control");
        assert_eq!(chosen.name, "Digital");
    }

    /// A digital output is refused whatever a card calls its controls.
    #[test]
    fn the_fallback_does_not_rescue_a_digital_output() {
        let controls = vec![control("Digital", true)];
        assert!(control_for(&controls, PcmKind::Digital).is_none());
    }

    /// The three shapes a playback PCM's `info` comes in: a digital carrier, a codec's analog pair,
    /// and a USB interface.
    #[test]
    fn hdmi_is_digital_and_the_other_two_are_not() {
        let hdmi = "card: 1\ndevice: 3\nstream: PLAYBACK\nid: HDMI 0\nname: HDMI 0\n";
        let analog =
            "card: 1\ndevice: 0\nstream: PLAYBACK\nid: Onboard Analog\nname: Onboard Analog\n";
        let usb = "card: 0\ndevice: 0\nstream: PLAYBACK\nid: USB Audio\nname: USB Audio\n";
        assert_eq!(pcm_kind_from_proc(hdmi), PcmKind::Digital);
        assert_eq!(pcm_kind_from_proc(analog), PcmKind::Analog);
        assert_eq!(pcm_kind_from_proc(usb), PcmKind::Analog);
    }

    #[test]
    fn the_other_digital_carriers_are_recognised_too() {
        for name in ["IEC958", "S/PDIF Out", "DisplayPort 1", "hdmi 2"] {
            let info = format!("stream: PLAYBACK\nid: {name}\nname: {name}\n");
            assert_eq!(
                pcm_kind_from_proc(&info),
                PcmKind::Digital,
                "{name} carries no level of its own"
            );
        }
    }

    /// A field that merely contains the word elsewhere must not decide it.
    #[test]
    fn only_the_naming_fields_are_read() {
        let info = "stream: PLAYBACK\nid: USB Audio\nname: USB Audio\nsubname: HDMI\n";
        assert_eq!(pcm_kind_from_proc(info), PcmKind::Analog);
    }

    #[test]
    fn a_file_that_says_nothing_is_taken_as_analog() {
        assert_eq!(pcm_kind_from_proc(""), PcmKind::Analog);
    }

    fn support(main: bool, left: bool, right: bool) -> VolumeSupport {
        VolumeSupport {
            main,
            left,
            right,
            pair: (1, 2),
        }
    }

    /// One volume over the whole device is the one to move, whatever the channels also offer.
    #[test]
    fn the_device_s_own_volume_is_taken_over_its_channels() {
        assert_eq!(
            volume_target(&support(true, true, true)),
            Some(VolumeTarget::Main)
        );
    }

    /// An interface that trims each channel and offers nothing over the two moves both.
    #[test]
    fn a_device_with_only_a_channel_pair_moves_both_of_them() {
        assert_eq!(
            volume_target(&support(false, true, true)),
            Some(VolumeTarget::Pair(1, 2))
        );
    }

    /// Moving one channel of a pair changes the balance, which is not what was asked for.
    #[test]
    fn half_a_pair_is_not_a_control() {
        assert!(volume_target(&support(false, true, false)).is_none());
        assert!(volume_target(&support(false, false, true)).is_none());
    }

    /// What an HDMI output answers on this platform, and it answers it itself.
    #[test]
    fn a_device_with_no_volume_property_has_no_control() {
        assert!(volume_target(&support(false, false, false)).is_none());
    }

    /// The pair is where the device says it is, not where a stereo pair usually sits.
    #[test]
    fn the_pair_is_the_one_the_device_named() {
        let support = VolumeSupport {
            main: false,
            left: true,
            right: true,
            pair: (3, 4),
        };
        assert_eq!(volume_target(&support), Some(VolumeTarget::Pair(3, 4)));
    }

    /// The reading that sent this rule in: a device answering unity while its own position sits
    /// at 43 per cent is not reporting its control.
    #[test]
    fn decibels_stuck_at_unity_do_not_track_the_control() {
        assert!(!decibels_track(0.0, 0.0, 0.4331));
    }

    /// A device below unity in both is reporting itself, however far apart the two numbers look:
    /// a real control has a taper of its own, and 0.375 of the way up is not 37.5 per cent of the
    /// range.
    #[test]
    fn decibels_below_unity_track_whatever_the_position_says() {
        assert!(decibels_track(-24.61, 0.0, 0.375));
        assert!(decibels_track(-32.0, 0.0, 0.5));
    }

    /// An output all the way up answers the same either way, so there is nothing to tell apart.
    #[test]
    fn an_output_at_the_top_tracks() {
        assert!(decibels_track(0.0, 0.0, 1.0));
    }

    /// A position converted the way CoreAudio's own conversion property converts it, checked against
    /// what two real devices answered.
    #[test]
    fn a_position_is_read_as_decibels_over_its_range() {
        assert!((decibels_at(0.375, -63.5, 0.0) - -39.69).abs() < 0.01);
        assert!((decibels_at(0.4331, -40.0, 0.0) - -22.68).abs() < 0.01);
        assert!((decibels_at(0.5, -40.0, 0.0) - -20.0).abs() < 0.01);
    }

    /// What is written and what is read back are one mapping, so a level survives the round trip.
    #[test]
    fn a_level_written_as_a_position_reads_back_as_itself() {
        for db in [-40.0, -30.0, -22.8, -6.0, 0.0] {
            let position = position_of(db, -40.0, 0.0);
            assert!(
                (decibels_at(position, -40.0, 0.0) - db).abs() < 0.01,
                "{db} dB did not survive the round trip"
            );
        }
    }

    /// Past either end lands on the end, which is what the control does with it anyway.
    #[test]
    fn a_position_past_either_end_stays_on_the_range() {
        assert!((position_of(20.0, -40.0, 0.0) - 1.0).abs() < f32::EPSILON);
        assert!((position_of(-400.0, -40.0, 0.0) - 0.0).abs() < f32::EPSILON);
        assert!((decibels_at(2.0, -40.0, 0.0) - 0.0).abs() < f32::EPSILON);
    }

    /// A UID is the driver's own text and may carry a colon, so only the first one divides an
    /// identifier.
    #[test]
    fn a_uid_carrying_a_colon_survives_the_split() {
        assert_eq!(
            coreaudio_uid("coreaudio:AppleUSBAudioEngine:Maker:Interface:1:2"),
            Some("AppleUSBAudioEngine:Maker:Interface:1:2")
        );
        assert_eq!(
            coreaudio_uid("coreaudio:BuiltInSpeakerDevice"),
            Some("BuiltInSpeakerDevice")
        );
    }

    /// Another platform's identifier and the sentinel are both refused, and refusing is not failing.
    #[test]
    fn nothing_but_a_coreaudio_identifier_names_a_device() {
        for id in [
            "alsa:hw:CARD=PCH,DEV=0",
            "wasapi:{0.0.0.00000000}",
            crate::device::SYSTEM_DEFAULT,
            "coreaudio:",
            "",
        ] {
            assert!(
                coreaudio_uid(id).is_none(),
                "{id} does not name a CoreAudio device"
            );
        }
    }
}
