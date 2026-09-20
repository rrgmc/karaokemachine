//! Audio focus: asking Android for the sound, and giving it back.
//!
//! **Android arbitrates who is heard, and an application that does not join in is both rude and
//! unprotected.** A machine that never asks is one another player talks over, and a machine that
//! never listens is one that goes on singing under a phone call. Both halves are here.
//!
//! **The policy is a pure function.** [`action`] takes the change the system reported and what the
//! machine remembers, and says what to do about it, so every branch is tested on a desktop with no
//! Android in sight. What is left is a request, an abandon and a callback, and only those three are
//! platform code.
//!
//! **Focus is held only while a song plays.** That is what keeps this from undoing
//! `Leaving the screen stops the music`: an application holding focus is exempt from Android's
//! cached-application freezer, so a machine that kept focus in the background would lose the
//! backstop that decision names. It pauses off the screen, the pause drops the focus, and the
//! freezer applies exactly as before.

use km_api::machine::TransportCommand;

/// What Android said about the sound.
///
/// The four cases the system reports, named for what they mean rather than for their constants. A
/// value the platform adds later is not one of these, and [`from_code`] declines it rather than
/// guessing.
///
/// **Only Android builds a value of this**, and only the tests do so anywhere else, which is what
/// the attribute is for: the type and its policy are ordinary Rust so that they can be read and
/// tested on a desktop, and a desktop has nothing that would construct one.
#[cfg_attr(
    not(target_os = "android"),
    allow(
        dead_code,
        reason = "constructed by the Android half and by the tests below"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Somebody else owns the sound now, and is not giving it back.
    Lost,
    /// Somebody else needs it for a moment. A call is the ordinary case.
    LostForNow,
    /// Somebody else needs to be heard over the top, and the system will lower the music itself.
    Ducked,
    /// It is the machine's again.
    Regained,
}

/// Android's `AudioManager` constants, which are stable API and are not going to move.
#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "read by the Android half and by the tests below")
)]
mod code {
    /// `AUDIOFOCUS_GAIN`.
    pub const GAIN: i32 = 1;
    /// `AUDIOFOCUS_LOSS`.
    pub const LOSS: i32 = -1;
    /// `AUDIOFOCUS_LOSS_TRANSIENT`.
    pub const LOSS_TRANSIENT: i32 = -2;
    /// `AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK`.
    pub const LOSS_TRANSIENT_CAN_DUCK: i32 = -3;
}

/// Reads the integer Android hands the listener.
///
/// Unknown values answer `None`. The platform is free to add one, and a machine that mapped a
/// stranger onto the nearest case it knew would pause a song for a reason nobody could name.
#[cfg_attr(
    not(target_os = "android"),
    allow(
        dead_code,
        reason = "called by the Android half and by the tests below"
    )
)]
pub fn from_code(value: i32) -> Option<Change> {
    match value {
        code::GAIN => Some(Change::Regained),
        code::LOSS => Some(Change::Lost),
        code::LOSS_TRANSIENT => Some(Change::LostForNow),
        code::LOSS_TRANSIENT_CAN_DUCK => Some(Change::Ducked),
        _ => None,
    }
}

/// What the machine does about a change, and what it remembers afterwards.
///
/// `owed` carries in whether a song is waiting on the sound coming back, and carries out whether one
/// still is. **That flag is the whole of why this does not reopen a settled question.** Coming back
/// to the screen owes nothing, so nothing resumes there, and
/// `Leaving the screen stops the music` stands. A call that ends owes a song, because the machine
/// stopped it rather than the person.
///
/// `Ducked` answers nothing on purpose. The request declares that the machine will not pause when
/// ducked, so the system lowers the volume for the length of the chime and puts it back, and a
/// singer sings through it.
pub fn action(change: Change, owed: bool) -> (Option<TransportCommand>, bool) {
    match change {
        Change::Lost => (Some(TransportCommand::Pause), false),
        Change::LostForNow => (Some(TransportCommand::Pause), true),
        Change::Ducked => (None, owed),
        Change::Regained if owed => (Some(TransportCommand::Play), false),
        Change::Regained => (None, false),
    }
}

/// Everywhere that is not Android, where nothing arbitrates the sound.
///
/// **A desktop, a television appliance and an iPhone all reach this.** iOS has an audio session
/// rather than focus, and cpal's own backend answers the interruption there; a desktop lets every
/// application make a sound at once by design. So the seam exists on every platform and does
/// nothing on most of them, which keeps the machine's own code free of `cfg`.
#[cfg(not(target_os = "android"))]
mod platform {
    use super::Change;

    /// Asks for the sound. Answers that it has it, there being nothing to ask.
    pub fn request() -> bool {
        true
    }

    /// Gives the sound back.
    pub fn abandon() {}

    /// Takes the change the system reported, if there is one.
    pub fn take() -> Option<Change> {
        None
    }
}

pub use platform::{abandon, request, take};

/// Android, where the three platform calls live.
#[cfg(target_os = "android")]
mod platform {
    use std::sync::atomic::{AtomicI32, Ordering};

    use super::{Change, from_code};

    /// The last change Android reported, waiting for the poll to read it.
    ///
    /// **Written from Android's main thread and read from the watchdog**, which is why it is an
    /// atomic and not a field of anything locked. The listener runs on the thread that must never
    /// wait — five seconds on it is an ANR — so it stores one integer and returns, and
    /// `Machine::settle_audio_focus` does the work within fifty milliseconds. This is the same
    /// arrangement `Machine::foreground` documents, for the same reason.
    ///
    /// `NONE` rather than an `Option` because an atomic holds a plain integer, and zero is not one
    /// of Android's codes.
    static REPORTED: AtomicI32 = AtomicI32::new(NONE);

    /// The value meaning nothing has been reported since the last read.
    const NONE: i32 = 0;

    /// Takes the change Android reported, leaving nothing behind.
    ///
    /// A change that arrives twice between two polls is read once, at its latest value, which is the
    /// one that describes the sound now.
    pub fn take() -> Option<Change> {
        match REPORTED.swap(NONE, Ordering::AcqRel) {
            NONE => None,
            value => from_code(value),
        }
    }

    /// Asks the activity for the sound, and says whether it was given.
    pub fn request() -> bool {
        match call(jni::jni_str!("requestAudioFocus")) {
            Ok(granted) => granted,
            Err(error) => {
                tracing::warn!(%error, "could not ask for audio focus");
                // **Reported rather than fatal, and it answers yes.** A machine that refused to play
                // because it could not reach a Java method would be worse than one that plays over
                // somebody, and playing is what it did before any of this existed.
                true
            }
        }
    }

    /// Gives the sound back.
    pub fn abandon() {
        if let Err(error) = call(jni::jni_str!("abandonAudioFocus")) {
            tracing::warn!(%error, "could not give audio focus back");
        }
    }

    /// Calls a no-argument boolean method on the activity.
    ///
    /// **The activity rather than a class, and that is what makes this work from this thread.** JNI's
    /// `FindClass` on a thread the JVM did not start searches the system class loader, which knows
    /// nothing of an application's own classes. Calling a method on an object the process already
    /// holds asks its class directly, so there is no lookup to get wrong.
    ///
    /// Both handles come back out of `ndk-context`, which [`crate::androidctx`] filled in for cpal.
    #[expect(
        unsafe_code,
        reason = "rebuilding jni handles from the raw pointers androidctx published; both are \
                  null-checked before use"
    )]
    fn call(method: &jni::strings::JNIStr) -> Result<bool, String> {
        let context = ndk_context::android_context();
        let raw_vm = context.vm();
        let raw_activity = context.context();
        if raw_vm.is_null() || raw_activity.is_null() {
            return Err("the Android context was never published".to_owned());
        }
        // SAFETY: `androidctx` published the loader's VM pointer, which outlives the process's use
        // of it.
        let vm = unsafe { jni::JavaVM::from_raw(raw_vm.cast()) };
        vm.attach_current_thread(|env: &mut jni::Env<'_>| {
            // SAFETY: a global reference `androidctx` made and deliberately never releases.
            let activity = unsafe { jni::objects::JObject::from_raw(env, raw_activity.cast()) };
            env.call_method(&activity, method, jni::jni_sig!("()Z"), &[])?
                .z()
        })
        .map_err(|error: jni::errors::Error| error.to_string())
    }

    /// Android's `OnAudioFocusChangeListener`, arriving on the main thread.
    ///
    /// **One store and nothing else.** See [`REPORTED`]. A panic here would cross the JNI boundary
    /// into a listener the platform called, so the body is written to have nothing in it that can
    /// panic rather than being wrapped in something that catches one.
    #[expect(
        unsafe_code,
        reason = "exporting the C symbol the JVM looks up by name; the body stores one integer"
    )]
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_rrgmc_karaokemachine_AudioFocus_onAudioFocusChange(
        _env: jni::EnvUnowned<'_>,
        _class: jni::objects::JClass<'_>,
        change: jni::sys::jint,
    ) {
        REPORTED.store(change, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every code Android documents maps to a case, and nothing else does.
    #[test]
    fn the_four_codes_are_the_four_cases() {
        assert_eq!(from_code(1), Some(Change::Regained));
        assert_eq!(from_code(-1), Some(Change::Lost));
        assert_eq!(from_code(-2), Some(Change::LostForNow));
        assert_eq!(from_code(-3), Some(Change::Ducked));
        for stranger in [0, 2, -4, i32::MIN, i32::MAX] {
            assert_eq!(
                from_code(stranger),
                None,
                "{stranger} is not a code we know"
            );
        }
    }

    /// The whole policy, as a table.
    #[test]
    fn what_each_change_asks_for() {
        let pause = Some(TransportCommand::Pause);
        let play = Some(TransportCommand::Play);
        let cases = [
            // change, owed coming in, what to do, owed going out
            (Change::Lost, false, pause, false),
            (Change::Lost, true, pause, false),
            (Change::LostForNow, false, pause, true),
            (Change::LostForNow, true, pause, true),
            (Change::Ducked, false, None, false),
            (Change::Ducked, true, None, true),
            (Change::Regained, false, None, false),
            (Change::Regained, true, play, false),
        ];
        for (change, owed, want_command, want_owed) in cases {
            let (command, now_owed) = action(change, owed);
            assert_eq!(command, want_command, "{change:?} with owed={owed}");
            assert_eq!(now_owed, want_owed, "{change:?} with owed={owed}");
        }
    }

    /// A song stopped for a call comes back; one stopped for good does not.
    #[test]
    fn only_a_loan_is_repaid() {
        let (command, owed) = action(Change::LostForNow, false);
        assert_eq!(command, Some(TransportCommand::Pause));
        assert!(owed, "a transient loss owes the song");
        let (command, owed) = action(Change::Regained, owed);
        assert_eq!(command, Some(TransportCommand::Play), "and pays it back");
        assert!(!owed, "once");

        let (command, owed) = action(Change::Lost, false);
        assert_eq!(command, Some(TransportCommand::Pause));
        assert!(!owed, "a permanent loss owes nothing");
        let (command, _) = action(Change::Regained, owed);
        assert_eq!(command, None, "so getting it back starts nothing");
    }

    /// Coming back to the screen is not a focus change, so nothing here resumes a song.
    ///
    /// The rule `Leaving the screen stops the music` rejects auto-resume, and this is the guard on
    /// it: the only state that makes [`action`] answer `Play` is a transient loss it caused itself.
    #[test]
    fn nothing_resumes_a_song_the_screen_paused() {
        let (command, _) = action(Change::Regained, false);
        assert_eq!(command, None);
    }
}
