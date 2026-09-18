//! Sending a process's logs somewhere a person can read them, on Android.
//!
//! On a desktop a `main` points `tracing` at stdout and that is the end of it. On Android there is
//! no stdout: the process is started by the zygote with its standard streams pointing at
//! `/dev/null`, so a subscriber writing there discards every line. For a while `km-app` did not
//! install a subscriber under `SDL_main` at all, which had the same effect for a different reason —
//! and the result was a device that failed silently, with `adb logcat` showing SDL starting the
//! machine and then nothing whatsoever. Debugging anything in that state is guesswork.
//!
//! So on Android the events go to the Android log, which is what `adb logcat` reads.
//!
//! `__android_log_write` rather than a crate: `liblog` is already linked on Android and this is its
//! stable C entry point, three arguments wide and unchanged since Android 1.0. A logging crate for
//! this would be a dependency, a version to track and a bridge to configure, in exchange for the
//! thirty lines below.
//!
//! # The tag is the caller's
//!
//! **A crate rather than a module because there are two Android applications here**, and they are
//! separate applications rather than two views of one: the machine, and the offline remote. The only
//! thing that differs between them is the logcat tag, so that is what [`Logcat::new`] takes —
//! `adb logcat -s karaokemachine` and `adb logcat -s km-remote` then show one app each, which is the
//! whole point of a tag. It was a `const` in `km-app` while there was one caller, and hardcoding it
//! is the single thing that had to change to let the second one exist.
//!
//! ```no_run
//! tracing_subscriber::fmt()
//!     .with_writer(km_androidlog::Logcat::new(c"km-remote"))
//!     .with_ansi(false)
//!     .without_time()
//!     .init();
//! ```
//!
//! # It compiles everywhere
//!
//! The crate compiles on every platform, not only Android: off Android the sink is a stub, so
//! [`drain_lines`] — the only part with logic worth getting wrong — is exercised by an ordinary
//! `cargo test` on the machine doing the building. Gated solely on Android it would carry tests that
//! never ran anywhere.

use std::ffi::CStr;
use std::io::{self, Write};

/// Splits `buf` into complete lines, hands each to `sink`, and leaves any unterminated remainder.
///
/// Separated out and platform-independent because it is the part that can be wrong: the formatting
/// layer arrives in several `write` calls per event, `__android_log_write` emits one logcat entry per
/// call, and a partial line has to wait for its newline rather than go out twice.
fn drain_lines(buf: &mut Vec<u8>, mut sink: impl FnMut(&[u8])) {
    let Some(last_newline) = buf.iter().rposition(|b| *b == b'\n') else {
        return;
    };
    let remainder = buf.split_off(last_newline + 1);
    let complete = std::mem::replace(buf, remainder);
    // `complete` ends with the newline at `last_newline`, so splitting on '\n' would yield a final
    // empty element and emit a blank logcat entry after every event. Drop the terminator instead of
    // filtering empties, which would also swallow genuine blank lines between events.
    for line in complete[..last_newline].split(|b| *b == b'\n') {
        sink(line);
    }
}

/// One event's worth of bytes, on its way to the Android log.
///
/// Carries the tag rather than reading a global, so two applications in one workspace cannot end up
/// sharing one — which is exactly what a `const` here would have forced.
pub struct LogcatWriter {
    tag: &'static CStr,
    buf: Vec<u8>,
}

impl LogcatWriter {
    /// A writer that tags everything it emits with `tag`.
    #[must_use]
    pub const fn new(tag: &'static CStr) -> Self {
        Self {
            tag,
            buf: Vec::new(),
        }
    }
}

impl Write for LogcatWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let tag = self.tag;
        self.buf.extend_from_slice(buf);
        drain_lines(&mut self.buf, |line| write_line(tag, line));
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let tag = self.tag;
        drain_lines(&mut self.buf, |line| write_line(tag, line));
        Ok(())
    }
}

impl Drop for LogcatWriter {
    fn drop(&mut self) {
        // An event that ended without a trailing newline would otherwise be lost, and the one that
        // matters most — a panic message, a fatal error — is exactly the one likely to be truncated.
        let remainder = std::mem::take(&mut self.buf);
        write_line(self.tag, &remainder);
    }
}

#[cfg(target_os = "android")]
mod sink {
    use std::ffi::{CStr, CString, c_char, c_int};

    /// `ANDROID_LOG_INFO` from `<android/log.h>`.
    ///
    /// One priority for everything rather than a mapping from `tracing`'s levels: the formatted line
    /// already begins with `INFO`/`WARN`/`ERROR`, so encoding it twice would only let the two
    /// disagree. `INFO` is chosen because it survives logcat's default filtering.
    const ANDROID_LOG_INFO: c_int = 4;

    #[expect(
        unsafe_code,
        reason = "declaring liblog's C entry point; there is no safe way to name a foreign function"
    )]
    unsafe extern "C" {
        /// From `liblog`. Returns the bytes written or a negative errno; the machine has nowhere
        /// useful to report a logging failure to, so the result is ignored.
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    /// Writes one line to the Android log under `tag`, dropping it if it cannot be represented.
    #[expect(
        unsafe_code,
        reason = "one FFI call into liblog with two NUL-terminated strings this function owns"
    )]
    pub fn write_line(tag: &CStr, line: &[u8]) {
        // A trailing `\r` shows up as a stray glyph in logcat, and blank lines are noise.
        let Some(end) = line.iter().rposition(|b| *b != b'\r') else {
            return;
        };
        // An interior NUL cannot go through a C string. Rather than drop the line, cut it there: the
        // beginning of a message is worth more than nothing, and `tracing` output should not contain
        // one anyway.
        let text = match CString::new(&line[..=end]) {
            Ok(text) => text,
            Err(error) => {
                let upto = error.nul_position();
                match CString::new(&error.into_vec()[..upto]) {
                    Ok(text) => text,
                    Err(_) => return,
                }
            }
        };
        // SAFETY: both pointers are NUL-terminated — `tag` is a `CStr` and `text` a `CString`, and
        // both outlive the call. liblog copies what it needs and retains neither.
        unsafe {
            __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), text.as_ptr());
        }
    }
}

/// Off Android there is no Android log to write to. The type still compiles so that `drain_lines`
/// can be tested on the machine doing the building.
#[cfg(not(target_os = "android"))]
mod sink {
    use std::ffi::CStr;

    pub fn write_line(_tag: &CStr, _line: &[u8]) {}
}

use sink::write_line;

/// Routes panics through `tracing`, so they reach the Android log too.
///
/// A panic's default report goes to stderr, which on Android is `/dev/null` — so a thread dying takes
/// its explanation with it. The audio thread did exactly that: it panicked while opening the device,
/// which the machine saw only as a dropped channel, with no message anywhere.
///
/// The backtrace is force-captured rather than left to `RUST_BACKTRACE`, because there is no
/// convenient way to set an environment variable for an activity. Frames come out as addresses, since
/// the packaged library is stripped; symbolize them against the unstripped copy in `target/`:
///
/// ```text
/// llvm-symbolizer --obj=target/aarch64-linux-android/debug/lib<name>.so 0x…
/// ```
///
/// Not gated on Android — it is ordinary `std` and `tracing`, and gating a function whose body has
/// no platform in it only stops an off-Android build from typechecking its caller.
pub fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Both of the payload types a `panic!` produces: `&'static str` for a literal message and
        // `String` for a formatted one. `payload_as_str` covers exactly those two.
        //
        // The one-call form needs a recent compiler, and this workspace has one: the pin names
        // the toolchain exactly, so there is no reason to hand-roll a
        // `downcast_ref::<&str>().or_else(…downcast_ref::<String>…)` pair against an older
        // `rust-version`. See `The Rust toolchain is pinned exactly` in docs/decisions/repository.md.
        let message = info
            .payload_as_str()
            .unwrap_or("<non-string panic payload>");
        let location = match info.location() {
            Some(location) => location.to_string(),
            None => "<unknown location>".to_owned(),
        };
        let thread = std::thread::current();
        tracing::error!(
            thread = thread.name().unwrap_or("<unnamed>"),
            location = %location,
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "panic: {message}"
        );
        // Chained rather than replaced: it costs nothing here and keeps the standard behavior for
        // any future target where stderr does go somewhere.
        previous(info);
    }));
}

/// A [`tracing_subscriber::fmt::MakeWriter`] that turns formatted events into Android log entries.
///
/// Not gated on Android, unlike the `unsafe` sink underneath it: a caller writing
/// `.with_writer(Logcat::new(c"km-remote"))` inside a `#[cfg(target_os = "android")]` block is doing
/// the gating already, and a type that exists everywhere is one an off-Android `cargo check` can
/// still typecheck.
pub struct Logcat(&'static CStr);

impl Logcat {
    /// A `MakeWriter` tagging every line with `tag`, as `adb logcat -s <tag>` filters on.
    #[must_use]
    pub const fn new(tag: &'static CStr) -> Self {
        Self(tag)
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logcat {
    type Writer = LogcatWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogcatWriter::new(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects what would have been emitted, so the assertions can be about lines rather than about
    /// the leftover buffer alone.
    fn drain(buf: &mut Vec<u8>) -> Vec<String> {
        let mut out = Vec::new();
        drain_lines(buf, |line| {
            out.push(String::from_utf8_lossy(line).into_owned())
        });
        out
    }

    #[test]
    fn a_partial_line_stays_buffered_until_its_newline_arrives() {
        let mut buf = b"one two".to_vec();
        assert!(
            drain(&mut buf).is_empty(),
            "an unterminated line must be held, not emitted"
        );
        assert_eq!(buf, b"one two");

        buf.extend_from_slice(b" three\n");
        assert_eq!(drain(&mut buf), ["one two three"]);
        assert!(buf.is_empty(), "nothing should be left over");
    }

    #[test]
    fn several_lines_in_one_write_are_split_and_the_remainder_kept() {
        let mut buf = b"a\nb\nc".to_vec();
        assert_eq!(drain(&mut buf), ["a", "b"]);
        assert_eq!(buf, b"c", "only the unterminated tail should remain");
    }

    #[test]
    fn a_line_is_emitted_once_and_not_again() {
        // The bug this guards: splitting the whole buffer and then not clearing it, which repeats
        // every line on each subsequent write.
        let mut buf = b"only\n".to_vec();
        assert_eq!(drain(&mut buf), ["only"]);
        assert!(
            drain(&mut buf).is_empty(),
            "a second drain must emit nothing"
        );
    }

    #[test]
    fn a_blank_line_between_two_events_does_not_swallow_the_second() {
        let mut buf = b"first\n\nsecond\n".to_vec();
        assert_eq!(drain(&mut buf), ["first", "", "second"]);
    }

    #[test]
    fn the_writer_reports_every_byte_consumed() {
        // tracing's fmt layer treats a short write as an error, so this has to hold even though the
        // bytes go to a buffer rather than a file.
        let mut writer = LogcatWriter::new(c"km-androidlog-test");
        assert_eq!(writer.write(b"a line\n").expect("infallible"), 7);
        writer.flush().expect("infallible");
    }
}
