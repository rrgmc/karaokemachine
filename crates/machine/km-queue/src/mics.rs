//! The microphone registry — state, not audio.
//!
//! Mic audio is mixed in hardware (a standing decision in `docs/decisions/`), so there is no input
//! stream here and no DSP. What this holds is the honest configuration an external mixer or
//! controller can act on: which mics exist, what they are called, their gain and effect settings,
//! whether they are muted.
//!
//! Deferred out of M3 on purpose. The registry is persisted configuration surfaced over the API, so
//! it needs the API's shape to exist first; writing it against an interface that did not exist yet
//! would have meant guessing twice.
//!
//! Everything is behind [`MicBus`] so that a future opt-in passthrough feature — cpal input into a
//! ring buffer into a mixer stage — can implement the same operations without the API changing.

/// Loudest gain a mic may be set to. `1.0` is unity.
///
/// Capped rather than unbounded because a remote that can set gain to 50 can produce feedback loud
/// enough to damage a speaker, and no legitimate use needs more than a few decibels of makeup.
pub const MAX_GAIN: f32 = 2.0;

/// How many mics may be registered.
///
/// Real machines have two to four. The cap exists so a buggy enumeration cannot fill the registry.
pub const MAX_MICS: usize = 8;

/// One microphone channel.
#[derive(Debug, Clone, PartialEq)]
pub struct MicChannel {
    /// Stable identifier, chosen by whatever registered the mic.
    pub id: String,
    /// What to call it on screen — "Mic 1", "Wireless".
    pub name: String,
    /// Which hardware input it corresponds to, for whoever is doing the mixing. Free-form: this
    /// crate never opens it.
    ///
    /// **A hint with nothing in it is no hint**, so a blank one is stored as `None`. That is what
    /// takes a hint away: a patch leaves alone every field it does not name, so absent already
    /// means *keep this*, and without a spelling for *none* a hint could be re-pointed but never
    /// removed.
    pub device_hint: Option<String>,
    /// Level, `1.0` being unity. Clamped to `0.0..=`[`MAX_GAIN`].
    pub gain: f32,
    /// Reverb amount, `0.0..=1.0`.
    pub reverb: f32,
    /// Echo amount, `0.0..=1.0`.
    pub echo: f32,
    /// Whether it is muted.
    pub muted: bool,
}

impl MicChannel {
    /// A channel at sensible defaults: unity gain, no effects, unmuted.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            device_hint: None,
            gain: 1.0,
            reverb: 0.0,
            echo: 0.0,
            muted: false,
        }
    }

    /// Names the hardware input. Chainable.
    pub fn with_device_hint(mut self, hint: impl Into<String>) -> Self {
        self.device_hint = Some(hint.into());
        self
    }

    /// Forces every value into range.
    ///
    /// Called after any change, so a value out of range can never be stored — including one loaded
    /// from a hand-edited settings file.
    pub fn clamp(&mut self) {
        // NaN would survive a plain clamp, and a NaN gain silently means "no sound" to whatever is
        // mixing. Treat it as unity, which is the value somebody would have wanted.
        if !self.gain.is_finite() {
            self.gain = 1.0;
        }
        if !self.reverb.is_finite() {
            self.reverb = 0.0;
        }
        if !self.echo.is_finite() {
            self.echo = 0.0;
        }
        self.gain = self.gain.clamp(0.0, MAX_GAIN);
        self.reverb = self.reverb.clamp(0.0, 1.0);
        self.echo = self.echo.clamp(0.0, 1.0);
        // Here rather than at each caller because every road into a channel ends at this function:
        // a patch, a registration, and a channel read back out of the settings file.
        if self
            .device_hint
            .as_deref()
            .is_some_and(|hint| hint.trim().is_empty())
        {
            self.device_hint = None;
        }
    }
}

/// A partial change to a channel. Absent fields are left alone.
///
/// A patch rather than a whole channel so two remotes adjusting different knobs do not overwrite
/// each other, which is exactly what happens at a party with two phones.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MicPatch {
    /// Rename it.
    pub name: Option<String>,
    /// Re-point it at a hardware input, or take the hint away with a blank one.
    pub device_hint: Option<String>,
    /// New gain, in thousandths, so the patch stays `Eq` and comparable in tests.
    pub gain_milli: Option<u32>,
    /// New reverb, in thousandths.
    pub reverb_milli: Option<u32>,
    /// New echo, in thousandths.
    pub echo_milli: Option<u32>,
    /// Mute or unmute.
    pub muted: Option<bool>,
}

impl MicPatch {
    /// Whether this patch would change nothing.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Sets the gain from a float.
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain_milli = Some(to_milli(gain));
        self
    }

    /// Sets the reverb from a float.
    pub fn reverb(mut self, reverb: f32) -> Self {
        self.reverb_milli = Some(to_milli(reverb));
        self
    }

    /// Sets the echo from a float.
    pub fn echo(mut self, echo: f32) -> Self {
        self.echo_milli = Some(to_milli(echo));
        self
    }

    /// Sets the mute flag.
    pub fn mute(mut self, muted: bool) -> Self {
        self.muted = Some(muted);
        self
    }
}

fn to_milli(value: f32) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value * 1000.0).round() as u32
}

fn from_milli(value: u32) -> f32 {
    value as f32 / 1000.0
}

/// Why a mic operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MicError {
    /// No channel with that id.
    #[error("no microphone with id '{0}'")]
    Unknown(String),
    /// A channel with that id is already registered.
    #[error("a microphone with id '{0}' is already registered")]
    Duplicate(String),
    /// The registry is full.
    #[error("no room for another microphone ({MAX_MICS} registered)")]
    Full,
}

/// The operations the API needs, so a future real mixer can replace this one.
pub trait MicBus: Send + Sync {
    /// Every channel, in registration order.
    fn channels(&self) -> Vec<MicChannel>;

    /// One channel.
    fn channel(&self, id: &str) -> Option<MicChannel>;

    /// Applies a partial change, returning the channel as it now stands.
    fn apply(&mut self, id: &str, patch: &MicPatch) -> Result<MicChannel, MicError>;
}

/// The mic channels this machine knows about.
///
/// Order is registration order, which is the order they should appear on screen: "Mic 1" before
/// "Mic 2" regardless of what the ids sort like.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MicRegistry {
    channels: Vec<MicChannel>,
}

impl MicRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Two unmuted mics at unity gain — what a home unit ships with.
    pub fn with_two_mics() -> Self {
        let mut registry = Self::new();
        // Cannot fail: an empty registry has room for two.
        let _ = registry.register(MicChannel::new("mic1", "Mic 1"));
        let _ = registry.register(MicChannel::new("mic2", "Mic 2"));
        registry
    }

    /// How many channels are registered.
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Adds a channel.
    pub fn register(&mut self, mut channel: MicChannel) -> Result<(), MicError> {
        if self.channels.len() >= MAX_MICS {
            return Err(MicError::Full);
        }
        if self
            .channels
            .iter()
            .any(|existing| existing.id == channel.id)
        {
            return Err(MicError::Duplicate(channel.id));
        }
        channel.clamp();
        self.channels.push(channel);
        Ok(())
    }

    /// Removes a channel. Returns it if it was there.
    pub fn unregister(&mut self, id: &str) -> Option<MicChannel> {
        let index = self.channels.iter().position(|channel| channel.id == id)?;
        Some(self.channels.remove(index))
    }

    /// Mutes or unmutes everything — what a panic button on the display would do.
    pub fn set_all_muted(&mut self, muted: bool) {
        for channel in &mut self.channels {
            channel.muted = muted;
        }
    }
}

impl MicBus for MicRegistry {
    fn channels(&self) -> Vec<MicChannel> {
        self.channels.clone()
    }

    fn channel(&self, id: &str) -> Option<MicChannel> {
        self.channels
            .iter()
            .find(|channel| channel.id == id)
            .cloned()
    }

    fn apply(&mut self, id: &str, patch: &MicPatch) -> Result<MicChannel, MicError> {
        let channel = self
            .channels
            .iter_mut()
            .find(|channel| channel.id == id)
            .ok_or_else(|| MicError::Unknown(id.to_owned()))?;
        if let Some(name) = &patch.name {
            channel.name = name.clone();
        }
        if let Some(hint) = &patch.device_hint {
            channel.device_hint = Some(hint.clone());
        }
        if let Some(gain) = patch.gain_milli {
            channel.gain = from_milli(gain);
        }
        if let Some(reverb) = patch.reverb_milli {
            channel.reverb = from_milli(reverb);
        }
        if let Some(echo) = patch.echo_milli {
            channel.echo = from_milli(echo);
        }
        if let Some(muted) = patch.muted {
            channel.muted = muted;
        }
        channel.clamp();
        Ok(channel.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_channel_is_unity_gain_and_unmuted() {
        let channel = MicChannel::new("mic1", "Mic 1");
        assert_eq!(channel.gain, 1.0);
        assert_eq!(channel.reverb, 0.0);
        assert!(!channel.muted);
        assert_eq!(channel.device_hint, None);
    }

    #[test]
    fn the_default_registry_has_two_mics_in_order() {
        let registry = MicRegistry::with_two_mics();
        let ids: Vec<_> = registry
            .channels()
            .into_iter()
            .map(|channel| channel.id)
            .collect();
        assert_eq!(ids, ["mic1", "mic2"]);
    }

    #[test]
    fn registering_the_same_id_twice_is_refused() {
        let mut registry = MicRegistry::new();
        registry
            .register(MicChannel::new("mic1", "Mic 1"))
            .expect("first");
        assert_eq!(
            registry.register(MicChannel::new("mic1", "Duplicate")),
            Err(MicError::Duplicate("mic1".to_owned()))
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn the_registry_is_capped() {
        let mut registry = MicRegistry::new();
        for index in 0..MAX_MICS {
            registry
                .register(MicChannel::new(format!("mic{index}"), "Mic"))
                .expect("within the cap");
        }
        assert_eq!(
            registry.register(MicChannel::new("extra", "Extra")),
            Err(MicError::Full)
        );
    }

    #[test]
    fn unregistering_removes_one_and_keeps_the_order_of_the_rest() {
        let mut registry = MicRegistry::new();
        for id in ["a", "b", "c"] {
            registry
                .register(MicChannel::new(id, id))
                .expect("register");
        }
        assert_eq!(
            registry.unregister("b").map(|channel| channel.id),
            Some("b".to_owned())
        );
        let ids: Vec<_> = registry.channels().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["a", "c"]);
        assert!(registry.unregister("b").is_none());
    }

    #[test]
    fn a_patch_touches_only_what_it_names() {
        let mut registry = MicRegistry::with_two_mics();
        let patched = registry
            .apply("mic1", &MicPatch::default().mute(true))
            .expect("apply");
        assert!(patched.muted);
        // Gain, name and effects are untouched.
        assert_eq!(patched.gain, 1.0);
        assert_eq!(patched.name, "Mic 1");
        assert_eq!(patched.reverb, 0.0);
        // The other mic is untouched too.
        assert!(!registry.channel("mic2").expect("mic2").muted);
    }

    #[test]
    fn an_empty_patch_changes_nothing() {
        let mut registry = MicRegistry::with_two_mics();
        let before = registry.channel("mic1").expect("mic1");
        assert!(MicPatch::default().is_empty());
        let after = registry.apply("mic1", &MicPatch::default()).expect("apply");
        assert_eq!(before, after);
    }

    #[test]
    fn patching_an_unknown_mic_says_which_one() {
        let mut registry = MicRegistry::with_two_mics();
        assert_eq!(
            registry.apply("mic9", &MicPatch::default().mute(true)),
            Err(MicError::Unknown("mic9".to_owned()))
        );
    }

    #[test]
    fn gain_is_clamped_rather_than_trusted() {
        let mut registry = MicRegistry::with_two_mics();
        let loud = registry
            .apply("mic1", &MicPatch::default().gain(99.0))
            .expect("apply");
        assert_eq!(loud.gain, MAX_GAIN);
        let quiet = registry
            .apply("mic1", &MicPatch::default().gain(-5.0))
            .expect("apply");
        assert_eq!(quiet.gain, 0.0);
    }

    #[test]
    fn effects_are_clamped_to_unit_range() {
        let mut registry = MicRegistry::with_two_mics();
        let patched = registry
            .apply("mic1", &MicPatch::default().reverb(4.0).echo(-1.0))
            .expect("apply");
        assert_eq!(patched.reverb, 1.0);
        assert_eq!(patched.echo, 0.0);
    }

    #[test]
    fn a_non_finite_gain_becomes_unity_rather_than_silence() {
        let mut channel = MicChannel::new("mic1", "Mic 1");
        channel.gain = f32::NAN;
        channel.clamp();
        assert_eq!(channel.gain, 1.0);
    }

    #[test]
    fn a_channel_registered_out_of_range_is_stored_in_range() {
        let mut registry = MicRegistry::new();
        let mut channel = MicChannel::new("mic1", "Mic 1");
        channel.gain = 500.0;
        registry.register(channel).expect("register");
        assert_eq!(registry.channel("mic1").expect("mic1").gain, MAX_GAIN);
    }

    #[test]
    fn a_blank_hint_takes_the_hint_away() {
        let mut registry = MicRegistry::new();
        registry
            .register(MicChannel::new("mic1", "Mic 1").with_device_hint("Headset"))
            .expect("register");
        for blank in ["", "   "] {
            registry
                .apply(
                    "mic1",
                    &MicPatch {
                        device_hint: Some("Headset".to_owned()),
                        ..MicPatch::default()
                    },
                )
                .expect("point it somewhere first");
            let cleared = registry
                .apply(
                    "mic1",
                    &MicPatch {
                        device_hint: Some(blank.to_owned()),
                        ..MicPatch::default()
                    },
                )
                .expect("apply");
            assert_eq!(cleared.device_hint, None, "blank was {blank:?}");
        }
    }

    #[test]
    fn a_patch_that_does_not_name_the_hint_keeps_it() {
        let mut registry = MicRegistry::new();
        registry
            .register(MicChannel::new("mic1", "Mic 1").with_device_hint("Headset"))
            .expect("register");
        let patched = registry
            .apply("mic1", &MicPatch::default().mute(true))
            .expect("apply");
        assert_eq!(patched.device_hint.as_deref(), Some("Headset"));
    }

    #[test]
    fn a_channel_registered_with_a_blank_hint_has_none() {
        let mut registry = MicRegistry::new();
        registry
            .register(MicChannel::new("mic1", "Mic 1").with_device_hint(" "))
            .expect("register");
        assert_eq!(registry.channel("mic1").expect("mic1").device_hint, None);
    }

    #[test]
    fn muting_everything_hits_every_channel() {
        let mut registry = MicRegistry::with_two_mics();
        registry.set_all_muted(true);
        assert!(registry.channels().iter().all(|channel| channel.muted));
        registry.set_all_muted(false);
        assert!(registry.channels().iter().all(|channel| !channel.muted));
    }

    #[test]
    fn gain_survives_a_round_trip_through_thousandths() {
        let mut registry = MicRegistry::with_two_mics();
        let patched = registry
            .apply("mic1", &MicPatch::default().gain(1.25))
            .expect("apply");
        assert_eq!(patched.gain, 1.25);
    }
}
