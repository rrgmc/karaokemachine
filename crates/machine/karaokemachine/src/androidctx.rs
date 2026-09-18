//! Publishing the Android context that cpal expects somebody else to have published.
//!
//! cpal's Android backend reaches the Java side through `ndk_context::android_context()`, which reads
//! a JavaVM pointer and an Activity object out of a global in the `ndk-context` crate. Something has
//! to put them there. In the usual Android-Rust arrangement that something is `ndk-glue` or
//! `android-activity`, which owns the activity and sets it during `ANativeActivity_onCreate`.
//!
//! We use neither: the activity is SDL's, and SDL keeps its JNI handles to itself. So the global
//! stayed empty, and the first thing to ask for it — the audio thread, opening the output device —
//! panicked with `android context was not initialized` and took the audio path down with it. On the
//! device that surfaced only as "no audio output", because a panic unwinds the thread and drops the
//! channel it would otherwise have reported through.
//!
//! cpal needs **both** halves: it takes the VM to attach a thread, and then calls methods *on* the
//! Activity to reach `AudioManager`. So both have to be genuine.
//!
//! **The VM comes from our own `JNI_OnLoad`.** Android calls that on every library `loadLibrary`
//! brings in, handing over the `JavaVM*` directly — no JNI call, no environment to borrow, and it
//! happens before `SDL_main`, which is exactly the ordering needed. The alternative was to ask SDL for
//! a `JNIEnv*` and call `GetJavaVM` through the function table, which in `jni-sys` 0.4 is a union
//! keyed by JNI version and in `jni` 0.22 means manufacturing an owned `Env` from a pointer somebody
//! else created — more unsafe code, and less certain, than letting the loader tell us.
//!
//! **The Activity comes from SDL**, which documents `SDL_GetAndroidActivity` as returning a *local*
//! reference. `ndk-context` keeps the pointer for the life of the process, so it is promoted to a
//! global reference here.

use std::ffi::c_void;
use std::sync::Once;
use std::sync::atomic::{AtomicPtr, Ordering};

use jni::sys::{JavaVM as RawJavaVM, jint};

/// The JavaVM, as handed to [`JNI_OnLoad`].
static JAVA_VM: AtomicPtr<RawJavaVM> = AtomicPtr::new(std::ptr::null_mut());

/// Guards the one-shot initialization.
///
/// `ndk_context::initialize_android_context` asserts that nothing was set before, so a second call
/// aborts the process. `SDL_main` runs once today, but a crash would be a poor way to find out that
/// has changed.
static ONCE: Once = Once::new();

/// Android's entry point for a freshly loaded native library.
///
/// The only reason this exists is to capture the `JavaVM*`. It deliberately does nothing else: the
/// loader calls it with the Java side only half set up, so registering natives or touching the
/// activity here would be a different and worse kind of bug.
#[expect(
    unsafe_code,
    reason = "exporting the C symbol Android's loader looks for; the body only stores a pointer"
)]
#[unsafe(no_mangle)]
pub extern "C" fn JNI_OnLoad(vm: *mut RawJavaVM, _reserved: *mut c_void) -> jint {
    JAVA_VM.store(vm, Ordering::Release);
    // The JNI version this library needs, not the one the device offers. 1.6 predates every Android
    // release we support and nothing here uses anything newer.
    jni::sys::JNI_VERSION_1_6
}

/// Publishes the JavaVM and Activity for cpal to find.
///
/// Reports rather than fails: without it there is no audio, which the machine already knows how to
/// say out loud, and refusing to start would trade a silent karaoke machine for no karaoke machine.
pub fn publish() {
    ONCE.call_once(|| match collect() {
        Ok(()) => tracing::debug!("published the JavaVM and Activity for cpal"),
        Err(error) => {
            tracing::error!(%error, "could not publish the Android context; there will be no audio")
        }
    });
}

/// The fallible part, separated so the reporting above stays readable.
#[expect(
    unsafe_code,
    reason = "reconstructing jni handles from raw pointers the loader and SDL provided, and handing \
              them to ndk-context; both are null-checked first"
)]
fn collect() -> Result<(), String> {
    let raw_vm = JAVA_VM.load(Ordering::Acquire);
    if raw_vm.is_null() {
        return Err("JNI_OnLoad was never called, so there is no JavaVM".to_owned());
    }
    // SAFETY: the loader gave us this pointer and the JVM outlives the process's use of it.
    let vm = unsafe { jni::JavaVM::from_raw(raw_vm) };

    // SAFETY: SDL is initialized by the time this runs, and SDL documents this call as safe from any
    // thread. The returned reference is local, which is dealt with below.
    let activity = unsafe { sdl3::sys::system::SDL_GetAndroidActivity() };
    if activity.is_null() {
        return Err("SDL reported no Activity".to_owned());
    }

    let context = vm
        .attach_current_thread(|env: &mut jni::Env<'_>| {
            // SAFETY: a non-null local reference SDL owns and keeps alive for this frame.
            let local = unsafe { jni::objects::JObject::from_raw(env, activity.cast()) };
            // `into_raw` hands out the pointer *without* deleting the reference, which is the whole
            // point: dropping the `Global` would call `DeleteGlobalRef` and invalidate the very
            // pointer `ndk-context` is about to keep. Never released, and that is the intended
            // lifetime — cpal dereferences it on the audio thread for as long as the machine runs.
            env.new_global_ref(&local).map(jni::refs::Global::into_raw)
        })
        .map_err(|error: jni::errors::Error| error.to_string())?;

    // SAFETY: the VM pointer is the loader's and outlives us; `context` is a global reference that is
    // never deleted. `ONCE` ensures the assert inside cannot trip.
    unsafe {
        ndk_context::initialize_android_context(raw_vm.cast(), context.cast());
    }
    Ok(())
}
