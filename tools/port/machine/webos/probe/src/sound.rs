//! P3: a 440 Hz sine, through ALSA's `default` or through PulseAudio's simple API, with underruns
//! logged. Both libraries are opened with `dlopen`.

use std::ffi::{CStr, CString, c_void};
use std::os::raw::{c_char, c_int, c_long};
use std::time::{Duration, Instant};

use crate::log;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

const RATE: u32 = 48_000;
const CHUNK: usize = 960; // 20 ms of stereo frames

fn open(name: &str) -> Result<*mut c_void, String> {
    let c = CString::new(name).unwrap();
    let h = unsafe { dlopen(c.as_ptr(), 2) };
    if h.is_null() {
        return Err(format!("dlopen {name}: {}", unsafe { CStr::from_ptr(dlerror()) }.to_string_lossy()));
    }
    Ok(h)
}

fn sym<T: Copy>(lib: *mut c_void, name: &str) -> Result<T, String> {
    let c = CString::new(name).unwrap();
    let p = unsafe { dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        return Err(format!("{name} not found"));
    }
    Ok(unsafe { std::mem::transmute_copy(&p) })
}

/// The next 20 ms of a quiet stereo sine.
fn fill(buf: &mut [i16], phase: &mut f32) {
    for frame in buf.chunks_mut(2) {
        let s = (phase.sin() * 0.15 * i16::MAX as f32) as i16;
        frame[0] = s;
        frame[1] = s;
        *phase += 2.0 * std::f32::consts::PI * 440.0 / RATE as f32;
        if *phase > 2.0 * std::f32::consts::PI {
            *phase -= 2.0 * std::f32::consts::PI;
        }
    }
}

/// Plays for `secs` through `backend`, "alsa" or "pulse".
pub fn run(backend: &str, secs: u64) {
    let r = match backend {
        "pulse" => pulse(secs),
        _ => alsa(secs),
    };
    if let Err(e) = r {
        log(&format!("sound ({backend}) FAILED: {e}"));
    }
}

fn alsa(secs: u64) -> Result<(), String> {
    let lib = open("libasound.so.2")?;
    let pcm_open: unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int = sym(lib, "snd_pcm_open")?;
    let set_params: unsafe extern "C" fn(*mut c_void, c_int, c_int, u32, u32, c_int, u32) -> c_int = sym(lib, "snd_pcm_set_params")?;
    let writei: unsafe extern "C" fn(*mut c_void, *const c_void, c_long) -> c_long = sym(lib, "snd_pcm_writei")?;
    let recover: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int = sym(lib, "snd_pcm_recover")?;
    let strerror: unsafe extern "C" fn(c_int) -> *const c_char = sym(lib, "snd_strerror")?;
    let err = |e: c_int| unsafe { CStr::from_ptr(strerror(e)) }.to_string_lossy().into_owned();

    let mut pcm = std::ptr::null_mut();
    let e = unsafe { pcm_open(&mut pcm, c"default".as_ptr(), 0, 0) };
    if e < 0 {
        return Err(format!("snd_pcm_open default: {}", err(e)));
    }
    // S16_LE, interleaved read/write, two channels, resampling allowed, 100 ms of latency.
    let e = unsafe { set_params(pcm, 2, 3, 2, RATE, 1, 100_000) };
    if e < 0 {
        return Err(format!("snd_pcm_set_params: {}", err(e)));
    }
    log("sound: ALSA default open, 48 kHz stereo S16, playing 440 Hz");
    let mut buf = vec![0i16; CHUNK * 2];
    let mut phase = 0.0;
    let (mut underruns, mut chunks) = (0u32, 0u64);
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        fill(&mut buf, &mut phase);
        let n = unsafe { writei(pcm, buf.as_ptr() as *const c_void, CHUNK as c_long) };
        if n < 0 {
            underruns += 1;
            log(&format!("sound: writei {} (underrun {underruns})", err(n as c_int)));
            let e = unsafe { recover(pcm, n as c_int, 1) };
            if e < 0 {
                return Err(format!("snd_pcm_recover: {}", err(e)));
            }
        }
        chunks += 1;
    }
    log(&format!("sound done: {chunks} chunks of 20 ms, {underruns} underruns"));
    Ok(())
}

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

fn pulse(secs: u64) -> Result<(), String> {
    let lib = open("libpulse-simple.so.0")?;
    type New = unsafe extern "C" fn(
        *const c_char, *const c_char, c_int, *const c_char, *const c_char,
        *const SampleSpec, *const c_void, *const c_void, *mut c_int,
    ) -> *mut c_void;
    let new: New = sym(lib, "pa_simple_new")?;
    let write: unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int = sym(lib, "pa_simple_write")?;
    let pulse = open("libpulse.so.0")?;
    let strerror: unsafe extern "C" fn(c_int) -> *const c_char = sym(pulse, "pa_strerror")?;
    let err = |e: c_int| unsafe { CStr::from_ptr(strerror(e)) }.to_string_lossy().into_owned();

    let spec = SampleSpec { format: 3, rate: RATE, channels: 2 }; // PA_SAMPLE_S16LE
    let mut e = 0;
    let s = unsafe {
        new(std::ptr::null(), c"km-probe".as_ptr(), 1, std::ptr::null(), c"sine".as_ptr(), &spec,
            std::ptr::null(), std::ptr::null(), &mut e)
    };
    if s.is_null() {
        return Err(format!("pa_simple_new: {}", err(e)));
    }
    log("sound: PulseAudio stream open, 48 kHz stereo S16, playing 440 Hz");
    let mut buf = vec![0i16; CHUNK * 2];
    let mut phase = 0.0;
    let (mut failures, mut chunks) = (0u32, 0u64);
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        fill(&mut buf, &mut phase);
        if unsafe { write(s, buf.as_ptr() as *const c_void, buf.len() * 2, &mut e) } < 0 {
            failures += 1;
            log(&format!("sound: pa_simple_write {}", err(e)));
        }
        chunks += 1;
    }
    log(&format!("sound done: {chunks} chunks of 20 ms, {failures} write failures"));
    Ok(())
}
