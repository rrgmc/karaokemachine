//! Whether anything this process prints will be read, and what to do when it will not.
//!
//! Two ways to end up with nowhere to print, and this crate exists because the second one arrived
//! after the first was solved.
//!
//! **A console-subsystem executable that was double-clicked is *given* a console** — a black window
//! that appears beside the application and stays for the session, which is precisely the thing this
//! was written to remove. So the process asks whether anybody was there to read it, and lets go of
//! the console when nobody was. That is a Windows build with no window in it — a `--no-desktop`
//! `km-package-builder.exe` or `km-remote.exe` — and every build of either on a platform with no
//! subsystem to choose.
//!
//! **It is emphatically *not* the console twins.** Letting go of the console is right for an
//! executable that was handed one it never wanted and wrong for the one whose entire reason to
//! exist is to be read: free it there and double-clicking `km-package-builder-console.exe` opens a
//! console and closes it again a moment later, leaving a server running with nothing on screen at
//! all. Which of the two an executable is, is a thing only that executable knows, so
//! [`decide_where_to_talk`] is told — see [`Console`].
//!
//! **A GUI-subsystem executable never has one at all.** That is `km-package-builder` and
//! `km-remote` on Windows once each has a window, and `karaokemachine` on Windows always
//! (every build of the machine has one): no console is created, so none flashes
//! past, and `GetConsoleProcessList` answers zero rather than one. Answering zero is *not* the same
//! as having nowhere to print, though — standard handles are inherited whatever the subsystem, so
//! `karaokemachine --version | cat` writes down a perfectly good pipe, and `dist_version` in
//! `tools/dist/common.sh` stages releases by relying on exactly that. Hence the handle check: the
//! count says whether a console is involved, the handle says whether anything can be written.
//!
//! **Getting that wrong is not a lost line, it is a crash.** `std::io::_print` *panics* on a write
//! failure — the message is `failed printing to stdout` — and a null standard output handle fails
//! every write. Left alone, the double-click path would abort the process every single time, and
//! would never once fail when run from a shell, which is where it was being tested.
//!
//! Hence [`say`]. Everything that would otherwise be a `println!` goes through it: it prints when
//! there is
//! somewhere to print, and becomes a `tracing` event when there is not. `tracing_subscriber` drops
//! its writer's errors rather than panicking, which is the difference that matters here.
//!
//! # A crate rather than a module, and why it is shared
//!
//! This began as `tools/km-package-builder/src/console.rs`, the curation tool's own answer to its own
//! double-click. It became a crate when the machine needed the same answer: `karaokemachine.exe` is
//! GUI-subsystem on Windows for the same reason and prints from the same kind of early-exit flag
//! path, so the alternative was a second copy of two `unsafe` calls. The workspace denies
//! `unsafe_code` and this module holds its only whole-module allowance (see the `Unsafe code, once`
//! decision in `docs/decisions/`) — so a copy would not have been a second caller, it would have been a
//! second exception. Sharing keeps the count at one file and one pair of calls.
//!
//! Nothing here knows which program it is in, and it is told only one thing about it: whether a
//! console is something this executable wanted. That is the narrowest fact that cannot be worked out
//! from the outside. Everything else — *is anybody reading?* — remains a property of how the process
//! was launched rather than of what it does, and remains this crate's own to answer.

pub mod meter;

pub use crate::meter::Meter;

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether anything printed would be thrown away, or would take the process with it.
static NOWHERE: AtomicBool = AtomicBool::new(false);

/// Says one line to whoever is listening.
///
/// A person at a terminal, if there is one; the log, if there is not. Deliberately not a macro
/// wrapping `println!` — the whole point is that the two are *different sinks*, and a macro that
/// expanded to `println!` in one arm would keep the panic in the other.
pub fn say(line: impl AsRef<str>) {
    let line = line.as_ref();
    if NOWHERE.load(Ordering::Relaxed) {
        // Trimmed because the banner is written with blank lines and leading spaces for a terminal,
        // and a log line wants neither.
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            tracing::info!("{trimmed}");
        }
    } else {
        println!("{line}");
    }
}

/// Whether nothing written will be seen.
///
/// Used to decide things that only make sense when somebody is reading — chiefly whether to open a
/// browser, since a URL printed to nowhere has told nobody anything. Not for deciding *where* to
/// print: that is [`say`]'s job and the two should not be confused.
pub fn nowhere_to_talk() -> bool {
    NOWHERE.load(Ordering::Relaxed)
}

/// Whether a console is something this executable wanted.
///
/// **Not a question about the run, which is why it is an argument and not something measured.** Both
/// halves of a pair of Windows executables can be double-clicked and both are handed the same
/// console; what differs is that one of them is the program somebody double-clicks and the other is
/// the program somebody types. Nothing observable at run time separates those two — the subsystem is
/// a flag in a header this process would have to read about itself — so the executable says which it
/// is, in the same spirit as `Shell` in `km-package-builder` and `km-remote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Console {
    /// A console this executable never asked for.
    ///
    /// Windows hands one to any console-subsystem binary that was double-clicked, and beside an
    /// application it is a black window that sits there for the session. If nobody is in it, it is
    /// let go of. This is the primary executable of each pair: `km-package-builder.exe` and
    /// `km-remote.exe` in a build with no window, and either on a platform with no subsystem to
    /// choose.
    NotWanted,
    /// A console this executable exists for, and keeps.
    ///
    /// The console twins — `km-package-builder-console.exe`, `km-remote-console.exe`,
    /// `karaokemachine-console.exe` — are the ones to type at, and are staged precisely so that a
    /// person has something that answers. Freeing their console makes a double-click open a window
    /// and close it again, which is the bug this distinction exists to fix. A twin started from a
    /// shell is unaffected either way: the shell is in the console too, so it was never a candidate
    /// for freeing.
    Wanted,
}

/// Whether a console Windows attached to this process should be let go of.
///
/// `attached` is `GetConsoleProcessList`'s count: zero when no console was ever created, one when
/// this process is alone in one — which is what a double-click arranges — and more when a shell is
/// in it too.
///
/// **Outside the `cfg(windows)` module on purpose**, and it is the only thing that is. The rule is
/// what was wrong, and a rule that can only be tested on the platform that has consoles is a rule
/// two of the three CI platforms cannot check. Everything genuinely Windows-only — the syscalls, the
/// handle check — stays where it was.
/// Dead on every platform but Windows, which is the price of hoisting it out and worth paying: the
/// point of it being here is that the *test* runs everywhere, and a test in a `cfg(windows)` module
/// is one two of the three CI platforms cannot run.
#[cfg_attr(not(windows), allow(dead_code))]
fn frees_the_console(attached: u32, console: Console) -> bool {
    attached == 1 && console == Console::NotWanted
}

/// Works out which sink this process has, giving up an unwanted console on the way.
///
/// Must be called before anything is printed, which is why it is the first thing `run` does after
/// installing the log subscriber.
pub fn decide_where_to_talk(console: Console) {
    if windows::nothing_is_listening(console) {
        NOWHERE.store(true, Ordering::Relaxed);
    }
}

/// The Windows half. Everywhere else there is nothing to decide.
///
/// macOS and Linux do not attach a console to a process launched from the Finder or a `.desktop`
/// entry: standard output goes to the system log or to nowhere, and writing to it is harmless. Nor
/// does either have a subsystem to choose, which is the other half of the problem here. What this
/// module solves is specific to Windows.
#[cfg(not(windows))]
mod windows {
    /// Somebody may well be reading, and if they are not it costs nothing.
    pub fn nothing_is_listening(_console: super::Console) -> bool {
        false
    }
}

#[cfg(windows)]
mod windows {
    // **The workspace denies `unsafe_code`, and this is its only whole-module allowance.**
    //
    // The rule is worth keeping and worth breaking exactly here. There is no safe wrapper for either
    // of these two calls — every function in `windows-sys` is an `unsafe fn` — and the allowance has
    // stayed at two: the handle check below is `AsRawHandle`, which is safe.
    //
    // Both calls are trivially sound. Neither takes a pointer we did not allocate, neither can
    // observe uninitialized memory, and both are documented as safe to call at any time from any
    // thread. The buffer handed to `GetConsoleProcessList` is a stack array whose length is passed
    // alongside it.
    //
    // Nothing else in this crate may use `unsafe`, and that this file is shared by three programs
    // rather than owned by one is what keeps the allowance at one module. Elsewhere in the workspace
    // an exception is a `#[expect(unsafe_code, reason = "…")]` on a single item — thirty-eight of
    // them, in the machine, its shells, `km-display` and `km-audio`, each naming the FFI call, the
    // SDL ordering rule or the mixer it exists for. See the `Unsafe code, once` decision in
    // `docs/decisions/`.
    #![allow(unsafe_code)]

    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

    /// Whether anything printed from here would go nowhere, freeing an unwanted console on the way.
    ///
    /// **The process count is the whole of the first test, and it is exact rather than a heuristic.**
    /// Launched from `cmd`, PowerShell or Git Bash, the shell is attached to the same console and the
    /// count is at least two. Launched by double-clicking a file, Windows creates a console for this
    /// process alone and the count is one. So "am I the only one here?" answers "did a person type
    /// this?" without guessing at parent process names or window titles.
    ///
    /// **Zero is the GUI-subsystem case and needs the second test.** No console was ever created, so
    /// there is nothing to free — but standard handles are inherited regardless of subsystem, so a
    /// run with a pipe or a redirect on it can still be read. Answer "not detached" here and the
    /// first `println!` of every double-click of the windowed build panics; answer "nowhere"
    /// unconditionally and `--version` prints nothing down a pipe, which is how releases are
    /// staged.
    ///
    /// **One is where the count stops being the whole answer.** It says a console exists and
    /// nobody typed into it — but "nobody typed this" is not the same as
    /// "nobody will read it", and for a console twin it is exactly backwards: somebody
    /// double-clicked the executable that exists to print, and freeing its console makes the window
    /// appear and vanish. So the count says what was arranged and [`super::Console`] says what this
    /// executable wanted, and only the two together decide.
    pub fn nothing_is_listening(console: super::Console) -> bool {
        let mut owners = [0u32; 4];
        // Returns how many processes are attached, filling as much of the buffer as fits.
        let attached = unsafe { GetConsoleProcessList(owners.as_mut_ptr(), owners.len() as u32) };
        if attached == 0 {
            return stdout_goes_nowhere();
        }
        // Ignoring nothing here deliberately: a console we were told we own but cannot free is one
        // we can still print to, so the answer follows whether the call worked.
        super::frees_the_console(attached, console) && unsafe { FreeConsole() != 0 }
    }

    /// Whether this process's standard output is a handle that cannot be written to.
    ///
    /// `GetStdHandle` answers null when nothing was inherited and `INVALID_HANDLE_VALUE` when it
    /// failed; both are checked, because the two mean the same thing to a caller and only one of them
    /// is the common case. Reached through `AsRawHandle` rather than a third `windows-sys` call, so
    /// this costs no `unsafe`.
    pub(super) fn stdout_goes_nowhere() -> bool {
        let handle = std::io::stdout().as_raw_handle();
        handle.is_null() || handle == INVALID_HANDLE_VALUE
    }

    /// What `GetStdHandle` returns on failure, spelled here rather than pulled in as a second
    /// `windows-sys` feature for one constant.
    const INVALID_HANDLE_VALUE: std::os::windows::io::RawHandle = -1isize as _;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both sinks, in one test, because they share one global.
    ///
    /// **Deliberately not two tests.** The flag is process-wide and the harness runs tests on
    /// threads, so a second test that flipped it would race whichever other test happened to read
    /// it — which is exactly what happened when these were written apart, and the failure moves
    /// around between runs.
    ///
    /// What is being asserted is the whole reason the module exists: `println!` panics on a stdout
    /// that cannot be written, so the silent branch must never reach it. The test cannot free a real
    /// console — that would take the harness's own output with it — so it drives the flag directly,
    /// which is precisely the state `decide_where_to_talk` leaves behind.
    #[test]
    fn a_silenced_process_logs_where_a_talking_one_prints() {
        assert!(
            !nowhere_to_talk(),
            "the flag starts false, so this is the terminal path"
        );
        say("a line");

        NOWHERE.store(true, Ordering::Relaxed);
        assert!(nowhere_to_talk());
        say("this must not reach println");
        // A line that is nothing but the banner's own padding: it has something to print and nothing
        // to log, and must not produce an empty log event.
        say("   ");

        NOWHERE.store(false, Ordering::Relaxed);
        assert!(!nowhere_to_talk());
    }

    /// A console twin keeps its console; the executable that never asked for one gives it back.
    ///
    /// **This is the bug, written down.** The count alone used to decide, so a double-clicked
    /// `km-package-builder-console.exe` — one process, one console, and the very executable that
    /// exists to be read — had its console freed: a window that opened and closed again, leaving a
    /// server running with nothing on screen. The count says what was arranged, and only the pair
    /// decides.
    ///
    /// Runs on every platform, which is the point of hoisting the rule out of the `cfg(windows)`
    /// module: two of the three CI platforms could not otherwise check it.
    #[test]
    fn only_an_executable_that_never_wanted_a_console_gives_one_back() {
        // A double-click: this process alone in a console Windows made for it.
        assert!(frees_the_console(1, Console::NotWanted), "nobody is in it");
        assert!(
            !frees_the_console(1, Console::Wanted),
            "the twin exists to be read; freeing this is the bug"
        );

        // A shell is in the console too, so somebody typed this and is waiting for an answer.
        for console in [Console::NotWanted, Console::Wanted] {
            assert!(!frees_the_console(2, console), "a shell is attached");
            // No console was ever created: the GUI-subsystem case, where there is nothing to free
            // and the handle check answers instead.
            assert!(!frees_the_console(0, console), "there is no console");
        }
    }

    /// The handle check answers rather than trapping, and says "somewhere" for a normal test run.
    ///
    /// **`nothing_is_listening` itself is deliberately not called here**: on a console holding this
    /// process alone it would call `FreeConsole`, and a test that can take the harness's own output
    /// away is worse than no test. This is the half that is safe to ask and the half that is new —
    /// the process count has been right since it was written, and the handle is what the
    /// GUI-subsystem build added.
    ///
    /// It asserts an answer rather than merely not panicking, because `cargo test` always has
    /// somewhere to put stdout: a console, or the pipe the harness captures through.
    #[cfg(windows)]
    #[test]
    fn a_test_run_always_has_somewhere_to_print() {
        assert!(!windows::stdout_goes_nowhere());
    }
}
