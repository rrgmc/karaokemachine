//! The Samsung television's output: Tizen's `audio_io`, in place of cpal.
//!
//! The set's audio server holds the hardware, so the ALSA device cpal opens answers `EBADFD`
//! (`docs/research/tizen.md` §3). `libcapi-media-audio-io.so.0` is opened with `dlopen`, so nothing
//! links against it, and needs no privilege. A thread of its own renders a block, converts it to S16
//! and hands it to the blocking `audio_out_write`, which returns once the device has room: that is
//! the clock, and the position the player publishes is the one the words are drawn from.

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use super::AudioError;

/// The rate and channel count asked for, as measured clean on the set.
pub(super) const SAMPLE_RATE: u32 = 48_000;
pub(super) const CHANNELS: u16 = 2;

/// Frames rendered and written at a time: 21 ms at 48 kHz, the block the probe played with.
pub(super) const BLOCK_FRAMES: usize = 1_024;

const LIBRARY: &CStr = c"libcapi-media-audio-io.so.0";

/// `RTLD_NOW`, which is 2 on every Linux.
const RTLD_NOW: c_int = 2;
/// `AUDIO_CHANNEL_STEREO` and `AUDIO_SAMPLE_TYPE_S16_LE` from `audio_io.h`.
const AUDIO_CHANNEL_STEREO: c_int = 0x81;
const AUDIO_SAMPLE_TYPE_S16_LE: c_int = 0x71;

/// What the player hears about: one name, because the set has one output.
pub(super) const NAME: &str = "Tizen audio_io";

type Handle = *mut c_void;
type CreateFn = unsafe extern "C" fn(c_int, c_int, c_int, *mut Handle) -> c_int;
type HandleFn = unsafe extern "C" fn(Handle) -> c_int;
type WriteFn = unsafe extern "C" fn(Handle, *mut c_void, c_uint) -> c_int;
type BufferSizeFn = unsafe extern "C" fn(Handle, *mut c_int) -> c_int;

#[expect(
    unsafe_code,
    reason = "declaring dlopen and dlsym, which glibc exports"
)]
unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The five `audio_out_*` calls the output makes, and one it logs with.
#[derive(Clone, Copy)]
struct Api {
    create: CreateFn,
    prepare: HandleFn,
    write: WriteFn,
    unprepare: HandleFn,
    destroy: HandleFn,
    buffer_size: Option<BufferSizeFn>,
}

impl Api {
    /// Opens the library, or says there is no device: off the set, or on a firmware without it.
    #[expect(unsafe_code, reason = "dlopen of a system library by its soname")]
    fn load() -> Result<Self, AudioError> {
        // SAFETY: a NUL-terminated soname; the handle is never closed, so nothing resolved from it
        // outlives it.
        let lib = unsafe { dlopen(LIBRARY.as_ptr(), RTLD_NOW) };
        if lib.is_null() {
            return Err(AudioError::NoDevice);
        }
        Ok(Self {
            create: required(lib, c"audio_out_create_new")?,
            prepare: required(lib, c"audio_out_prepare")?,
            write: required(lib, c"audio_out_write")?,
            unprepare: required(lib, c"audio_out_unprepare")?,
            destroy: required(lib, c"audio_out_destroy")?,
            buffer_size: function(lib, c"audio_out_get_buffer_size"),
        })
    }
}

fn required<F: Copy>(lib: *mut c_void, name: &CStr) -> Result<F, AudioError> {
    function(lib, name).ok_or_else(|| {
        AudioError::Stream(format!(
            "{} has no {}",
            LIBRARY.to_string_lossy(),
            name.to_string_lossy()
        ))
    })
}

/// A function, typed by the caller as the C prototype `audio_io.h` declares.
#[expect(
    unsafe_code,
    reason = "a dlsym result turned into the function pointer type its C header declares"
)]
fn function<F: Copy>(lib: *mut c_void, name: &CStr) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: `lib` came from `dlopen` and is never closed; `name` is NUL-terminated.
    let p = unsafe { dlsym(lib, name.as_ptr()) };
    // SAFETY: every `F` here is an `extern "C" fn` type transcribed from `audio_io.h`, and a
    // non-null code address is a valid value of it.
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

/// Whether the output exists, without opening it.
pub(super) fn available() -> Result<(), AudioError> {
    Api::load().map(|_| ())
}

/// The running output: a thread writing to `audio_io` until this is dropped.
pub(crate) struct Stream {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Stream {
    /// Opens the device and starts writing what `render` fills, one [`BLOCK_FRAMES`] block at a
    /// time. `failed` is called once if a write is refused, and the thread then ends.
    pub(super) fn start(
        render: impl FnMut(&mut [f32]) + Send + 'static,
        failed: impl FnOnce() + Send + 'static,
    ) -> Result<Self, AudioError> {
        let api = Api::load()?;
        let stop = Arc::new(AtomicBool::new(false));
        let (opened, outcome) = mpsc::channel();
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("km-audio-io".to_owned())
            .spawn(move || run(api, &thread_stop, &opened, render, failed))
            .map_err(|e| AudioError::Stream(e.to_string()))?;
        // The handle is made on the thread that writes to it, and the answer comes back here so a
        // refusal is an error from `open`, as cpal's would be.
        match outcome.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(AudioError::Stream(error))
            }
            Err(_) => {
                let _ = thread.join();
                Err(AudioError::Stream("the audio_io thread ended".to_owned()))
            }
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // A write returns within one block, so this waits at most about 21 ms.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[expect(
    unsafe_code,
    reason = "the audio_io calls, on a handle this function creates and destroys"
)]
fn run(
    api: Api,
    stop: &AtomicBool,
    opened: &mpsc::Sender<Result<(), String>>,
    mut render: impl FnMut(&mut [f32]),
    failed: impl FnOnce(),
) {
    let mut handle: Handle = null_mut();
    // SAFETY: the prototype is `audio_io.h`'s; `handle` is written by the call.
    let rc = unsafe {
        (api.create)(
            SAMPLE_RATE as c_int,
            AUDIO_CHANNEL_STEREO,
            AUDIO_SAMPLE_TYPE_S16_LE,
            &mut handle,
        )
    };
    if rc != 0 {
        let _ = opened.send(Err(format!("audio_out_create_new answered {rc:#x}")));
        return;
    }
    // SAFETY: `handle` is the one just created.
    let rc = unsafe { (api.prepare)(handle) };
    if rc != 0 {
        // SAFETY: as above; it is not used again.
        unsafe { (api.destroy)(handle) };
        let _ = opened.send(Err(format!("audio_out_prepare answered {rc:#x}")));
        return;
    }
    let buffer_bytes = api.buffer_size.and_then(|get| {
        let mut size: c_int = 0;
        // SAFETY: `handle` is prepared; `size` is written by the call.
        (unsafe { get(handle, &mut size) } == 0).then_some(size)
    });
    let bytes_per_ms = SAMPLE_RATE / 1000 * u32::from(CHANNELS) * 2;
    tracing::info!(
        device = NAME,
        sample_rate = SAMPLE_RATE,
        channels = CHANNELS,
        format = "S16_LE",
        buffer_bytes,
        buffer_ms = buffer_bytes.map(|b| b.unsigned_abs() / bytes_per_ms),
        block_frames = BLOCK_FRAMES,
        "opened the output device"
    );
    let _ = opened.send(Ok(()));

    let samples = BLOCK_FRAMES * usize::from(CHANNELS);
    let mut float = vec![0.0f32; samples];
    let mut pcm = vec![0i16; samples];
    let bytes = c_uint::try_from(samples * size_of::<i16>()).unwrap_or(c_uint::MAX);
    let mut failed = Some(failed);
    while !stop.load(Ordering::Acquire) {
        render(&mut float);
        // The block is already limited to full scale; the cast saturates anyway.
        for (out, sample) in pcm.iter_mut().zip(&float) {
            *out = (sample * f32::from(i16::MAX)) as i16;
        }
        // SAFETY: `handle` is prepared and `pcm` holds `bytes` bytes for the length of the call.
        let rc = unsafe { (api.write)(handle, pcm.as_mut_ptr().cast(), bytes) };
        if rc < 0 {
            tracing::error!(
                code = format_args!("{rc:#x}"),
                "audio_out_write refused a block; dropping the stream"
            );
            if let Some(failed) = failed.take() {
                failed();
            }
            break;
        }
    }
    // SAFETY: `handle` is prepared, and nothing uses it after these two calls.
    unsafe {
        (api.unprepare)(handle);
        (api.destroy)(handle);
    }
    tracing::info!(device = NAME, "closed the output device");
}
