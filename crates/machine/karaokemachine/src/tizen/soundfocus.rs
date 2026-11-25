//! Sound focus on a Samsung television: `audiofocus`'s three platform calls over Tizen's sound
//! manager.
//!
//! **The policy is `crate::audiofocus`'s, shared with Android**; this asks for the sound, gives it
//! back and stores what the set says. `libcapi-media-sound-manager.so.0` is opened with `dlopen`, as
//! `window.rs` opens Ecore, so nothing links against it.
//!
//! **One stream information, made on the first request and kept for the life of the process.** It
//! is not attached to the `audio_io` handle: focus is advice, and the machine pausing itself is the
//! whole of the effect. Attaching it would let the sound server refuse the output, which the machine
//! has no answer for. The set re-grants focus itself when the taker lets go (reacquisition is on by
//! default), and that `ACQUIRED` is what resumes a song paused for a while.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::null;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::audiofocus::{Change, from_tizen, tizen_code::FOR_PLAYBACK};

use super::window::{open, required};

const LIBRARY: &CStr = c"libcapi-media-sound-manager.so.0";

/// `SOUND_STREAM_TYPE_MEDIA` and `SOUND_BEHAVIOR_NONE`.
const STREAM_TYPE_MEDIA: c_int = 0;
const BEHAVIOR_NONE: c_int = 0;

type Info = *mut c_void;
type FocusCallback = extern "C" fn(Info, c_int, c_int, c_int, c_int, *const c_char, *mut c_void);
type CreateFn = unsafe extern "C" fn(c_int, FocusCallback, *mut c_void, *mut Info) -> c_int;
type FocusFn = unsafe extern "C" fn(Info, c_int, c_int, *const c_char) -> c_int;

/// The stream information and the two calls made on it.
struct Focus {
    info: Info,
    acquire: FocusFn,
    release: FocusFn,
}

// SAFETY: `info` is an opaque handle the sound manager documents as usable from any thread; it is
// never freed, and only passed back to the library.
#[expect(
    unsafe_code,
    reason = "a C handle shared between the threads that ask for focus"
)]
unsafe impl Send for Focus {}
// SAFETY: as above; nothing here mutates through a shared reference.
#[expect(
    unsafe_code,
    reason = "a C handle shared between the threads that ask for focus"
)]
unsafe impl Sync for Focus {}

/// Made once; `None` if the library or the stream information could not be had, said once.
static FOCUS: OnceLock<Option<Focus>> = OnceLock::new();

/// The last change the set reported, encoded by [`encode`], waiting for the poll.
static REPORTED: AtomicI32 = AtomicI32::new(NONE);
const NONE: i32 = 0;

fn encode(change: Change) -> i32 {
    match change {
        Change::Lost => 1,
        Change::LostForNow => 2,
        Change::Ducked => 3,
        Change::Regained => 4,
    }
}

fn decode(value: i32) -> Option<Change> {
    match value {
        1 => Some(Change::Lost),
        2 => Some(Change::LostForNow),
        3 => Some(Change::Ducked),
        4 => Some(Change::Regained),
        _ => None,
    }
}

/// Takes the change the set reported, leaving nothing behind.
pub fn take() -> Option<Change> {
    decode(REPORTED.swap(NONE, Ordering::AcqRel))
}

/// Asks for the sound. **Answers yes even when refused**, as Android's half does when it cannot
/// ask: the machine plays anyway, as it did before it asked, and asks again on the next song.
pub fn request() -> bool {
    let Some(focus) = focus() else {
        return true;
    };
    // SAFETY: `info` is the live stream information; the prototype is `sound_manager.h`'s.
    #[expect(
        unsafe_code,
        reason = "sound_manager_acquire_focus on the handle made in `focus`"
    )]
    let rc = unsafe { (focus.acquire)(focus.info, FOR_PLAYBACK, BEHAVIOR_NONE, null()) };
    if rc == 0 {
        tracing::info!("the set gave this machine the sound");
    } else {
        tracing::warn!(
            code = format_args!("{rc:#x}"),
            "the set would not give this machine the sound; playing anyway"
        );
    }
    true
}

/// Gives the sound back. A refusal means it was not held, which is what was wanted.
pub fn abandon() {
    let Some(focus) = focus() else {
        return;
    };
    // SAFETY: as in `request`.
    #[expect(
        unsafe_code,
        reason = "sound_manager_release_focus on the handle made in `focus`"
    )]
    let rc = unsafe { (focus.release)(focus.info, FOR_PLAYBACK, BEHAVIOR_NONE, null()) };
    if rc == 0 {
        tracing::info!("gave the sound back to the set");
    } else {
        tracing::debug!(code = format_args!("{rc:#x}"), "the sound was not held");
    }
}

fn focus() -> Option<&'static Focus> {
    FOCUS
        .get_or_init(|| match make() {
            Ok(focus) => Some(focus),
            Err(error) => {
                tracing::warn!(%error, "no sound focus on this set; the machine will not pause for other sound");
                None
            }
        })
        .as_ref()
}

#[expect(
    unsafe_code,
    reason = "sound_manager_create_stream_information, whose handle is kept for good"
)]
fn make() -> anyhow::Result<Focus> {
    let lib = open(LIBRARY)?;
    let create: CreateFn = required(lib, c"sound_manager_create_stream_information")?;
    let acquire: FocusFn = required(lib, c"sound_manager_acquire_focus")?;
    let release: FocusFn = required(lib, c"sound_manager_release_focus")?;
    let mut info: Info = std::ptr::null_mut();
    // SAFETY: the prototype is `sound_manager.h`'s; `info` is written by the call; the callback is a
    // `'static` function and the user data is unused.
    let rc = unsafe { create(STREAM_TYPE_MEDIA, on_focus, std::ptr::null_mut(), &mut info) };
    anyhow::ensure!(
        rc == 0 && !info.is_null(),
        "sound_manager_create_stream_information answered {rc:#x}"
    );
    Ok(Focus {
        info,
        acquire,
        release,
    })
}

/// The set's focus callback, on whatever thread the sound manager calls it from.
///
/// It stores one integer and logs, inside `catch_unwind`: a panic must not cross back into C, and
/// which reason and thread the set reports is still unmeasured, so the log line is the measurement.
extern "C" fn on_focus(
    _info: Info,
    mask: c_int,
    state: c_int,
    reason: c_int,
    behavior: c_int,
    _extra: *const c_char,
    _user: *mut c_void,
) {
    let _ = std::panic::catch_unwind(|| {
        let change = from_tizen(mask, state, reason, behavior);
        tracing::info!(
            mask,
            state,
            reason,
            behavior,
            ?change,
            thread = std::thread::current().name().unwrap_or("unnamed"),
            "the set moved the sound"
        );
        if let Some(change) = change {
            REPORTED.store(encode(change), Ordering::Release);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_change_survives_the_atomic() {
        for change in [
            Change::Lost,
            Change::LostForNow,
            Change::Ducked,
            Change::Regained,
        ] {
            assert_eq!(decode(encode(change)), Some(change));
        }
        assert_eq!(decode(NONE), None);
    }
}
