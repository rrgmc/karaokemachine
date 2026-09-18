//! Handing a file or a URL to whatever the operating system uses for it.
//!
//! **This was two copies of one file until there were three callers**, and the rule that produced
//! the crate was written down in both of them: *a third caller earns the crate*. The alternative
//! considered and rejected each time was folding it into `km-console`, which would widen the one
//! file allowed to write `unsafe` from "is anybody reading?" to "and also start processes" — a
//! second exception rather than a second caller. The bargain the duplication was always going to
//! cost had already been paid once: the Windows quoting bug in [`command_for`] was found in
//! `km-package-builder` and had to be fixed in `km-remote` too, from a report about a different
//! button. That cannot happen again from here.
//!
//! Three callers, and each wants a different one of the two functions for a different reason.
//! `km-package-builder` opens a `.kar` in whatever media player is installed, which is the quickest
//! way to hear a candidate without involving the karaoke machine at all, and opens its own URL for
//! `--open`. `km-remote` opens its URL for `--open` and when a webview will not build. The machine
//! opens the packages folder, which is `F12`.
//!
//! **Every one of them runs it on the machine the program is on**, which for the two web servers
//! means it only does anything useful when the browser and the tool are on the same box — the normal
//! way to use those, and their pages say so beside the button.
//!
//! Best-effort by nature. A machine with no default browser, no file manager, or no `xdg-open` at
//! all is a normal thing and not a reason for anything here to refuse to start, so this returns the
//! failure and never decides what to do about it: two callers log it and carry on, and the machine
//! puts it on the screen.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

/// Start a child process without giving it a console window of its own.
///
/// Spelled here rather than taken from `windows-sys`, which this crate does not depend on at all —
/// it depends on nothing. Taking a whole crate and a process-creation feature for one `u32` would be
/// a larger change than the number it defines, and would end this crate's only real property.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Hands a path to the platform's opener.
///
/// A file lands in whatever is registered for its extension; a **directory** opens in the file
/// manager, which all three platforms do without being asked differently.
pub fn open(path: &Path) -> std::io::Result<()> {
    run(command_for(path.as_os_str())?)
}

/// Hands a URL to the platform's browser.
pub fn open_url(url: &str) -> std::io::Result<()> {
    run(command_for(OsStr::new(url))?)
}

fn run(mut command: Command) -> std::io::Result<()> {
    // **The opener's own name, because "No such file or directory" names nothing.** This crate is
    // the only place that knows which program was going to run, and its absence is the ordinary
    // failure on Linux: `xdg-open` comes from `xdg-utils`, which a box with no desktop environment
    // has no reason to carry. Every caller prints this error beside the address it could not open,
    // so what the reader gets is the package to install rather than an errno they must place.
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command.status().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{program} is not installed"),
            )
        } else {
            error
        }
    })?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "the system opener exited with {status}"
        )));
    }
    Ok(())
}

/// The command that would be run. Split out so it can be asserted without launching anything.
///
/// On macOS and Linux the target is one argument and there is nothing else to say: `open` and
/// `xdg-open` are ordinary programs, and an argument reaches them as it was written.
///
/// **Windows is not that, and this file used to claim it was.** The comment here read *"the target
/// is passed as a single argument rather than interpolated into a command line, so a filename
/// containing a space, a quote or an ampersand cannot become part of the command"* — and every
/// clause of it was true of Rust's `Command` and false of what actually ran. There is no `argv` on
/// Windows: `Command` flattens its arguments into one command-line string, quoting an argument only
/// when it is empty or holds a space or a tab, and `cmd.exe` then re-parses that string with rules
/// of its own in which `&` separates commands. A percent-encoded URL holds no space — `model::encode`
/// guarantees that — so it went across bare and `cmd` cut it at the first `&`. *Open in browser* on
/// a filtered browse list therefore opened `…/songs?min_score=8` — the filter's spelling at the
/// time — threw away the letter and the page, and reported `exit code: 1` from the fragments it
/// then tried to run as commands. The same line
/// broke *Open in OS* for any song whose file name holds an `&` with no space beside it, which a
/// corpus of hundreds of thousands of files has plenty of; a space in the name was accidentally
/// protecting the rest.
///
/// So the target is quoted **for `cmd`'s parser** by [`cmd_quoted`] and written verbatim with
/// `raw_arg`, which is the only way to decide what that parser sees. The empty `""` before it is
/// still the window title `start` insists on, and is what stops `start` reading the quoted target as
/// one.
fn command_for(target: &OsStr) -> std::io::Result<Command> {
    if cfg!(target_os = "windows") {
        let quoted = cmd_quoted(target)?;
        let mut command = Command::new("cmd");
        command.args(["/c", "start", ""]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Verbatim, because `arg` would quote this again by MSVCRT's rules — backslashes
            // doubled, quotes escaped as `\"` — and `cmd` understands neither.
            command.raw_arg(&quoted);
            // **`cmd` is a console program, and every caller here has a GUI-subsystem executable
            // with no console at all**, so without this Windows makes one — a black window that
            // flashes up on every Play click, every browser open and every `F12`, from applications
            // whose whole point is that they do not do that. Nothing is lost by hiding it: `start`
            // hands the target to the shell and says nothing.
            command.creation_flags(CREATE_NO_WINDOW);
        }
        // Never reached, this arm being `cfg!(windows)`. It is what lets the shape of the Windows
        // command be asserted from a Linux or macOS test run, which is where this crate's CI is.
        #[cfg(not(windows))]
        command.arg(&quoted);
        Ok(command)
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(target);
        Ok(command)
    } else {
        // **Android matches this arm and has no `xdg-open`**, and so does a Debian box with no
        // desktop on it, which is what the machine's appliance deployment is. Neither is special-
        // cased here: a caller that should not offer the route decides that for itself — the
        // machine's `FILE_MANAGER` compiles the key out on Android — and a caller that offers it
        // anyway gets the honest `NotFound` back rather than a guess made in this file about a
        // platform it cannot see.
        let mut command = Command::new("xdg-open");
        command.arg(target);
        Ok(command)
    }
}

/// The target as `cmd.exe` has to be given it: wrapped in double quotes.
///
/// Quotes are the whole mechanism, and they are enough for the metacharacters that matter —
/// `&`, `|`, `<`, `>`, `^`, `(` and `)` are all inert between them.
///
/// **A target holding a `"` is refused rather than mangled**, because a `cmd` command line has no
/// escape for one: `\"` is MSVCRT's convention and `cmd` does not read it, and `""` inside quotes
/// means different things in different builds of the shell. It is unreachable today — a browser
/// percent-encodes `"` as `%22` before it ever reaches the address bar, and Windows forbids the
/// character in a path — but "unreachable" is exactly the reasoning that produced the bug above, so
/// it is a returned error and not a comment.
///
/// The one thing quoting does **not** stop is `%VAR%` expansion, and it is left alone deliberately:
/// there is no escape for `%` on a `cmd` command line either, `cmd` leaves a `%` alone unless it
/// brackets a variable that exists, and the `%` in a URL is percent-encoding (`%20`, `%2F`) which
/// never does.
fn cmd_quoted(target: &OsStr) -> std::io::Result<OsString> {
    if target.as_encoded_bytes().contains(&b'"') {
        return Err(std::io::Error::other(format!(
            "cannot open {}: a double quote cannot be passed through the Windows shell",
            target.to_string_lossy()
        )));
    }
    let mut quoted = OsString::with_capacity(target.len() + 2);
    quoted.push("\"");
    quoted.push(target);
    quoted.push("\"");
    Ok(quoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The browse list's own address bar, as `HX-Push-Url` writes it and *Open in browser* reads it
    /// back: several parameters, no space anywhere, because every value is percent-encoded. This is
    /// the exact shape that was being cut at the first `&`.
    ///
    /// Kept in step with what the browse page actually emits, which is the only thing that makes it
    /// worth writing a real URL out rather than `a=1&b=2`.
    const FILTERED: &str = "http://127.0.0.1:8178/songs?suitability=8-10&initial=Q&offset=200";

    /// **An opener that is not installed says which one**, because the caller prints this error and
    /// `No such file or directory (os error 2)` tells a reader nothing they can act on. The Linux
    /// case is the one that happens: `xdg-open` ships in `xdg-utils`, which a box with no desktop
    /// environment has no reason to carry, so the tools' browser-open fails there as a matter of
    /// course. Driven through a name nothing could provide rather than through the real opener,
    /// which on a developer's machine exists and would launch something.
    #[test]
    fn a_missing_opener_names_itself() {
        let error = run(Command::new("km-no-such-opener")).expect_err("no such program");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(
            error.to_string().contains("km-no-such-opener"),
            "the message should name the program, got: {error}"
        );
    }

    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_target_is_quoted_for_the_windows_shell() {
        assert_eq!(
            cmd_quoted(OsStr::new(FILTERED)).unwrap().to_string_lossy(),
            format!("\"{FILTERED}\"")
        );
    }

    /// The song files this is pointed at come from somebody else's corpus, and `Simon&Garfunkel`
    /// with no space is a name that really occurs, and an unquoted one opens as far as the `&`.
    #[test]
    fn a_file_name_with_an_ampersand_and_no_space_is_quoted_too() {
        assert_eq!(
            cmd_quoted(OsStr::new("C:/x/Simon&Garfunkel-Sound_of_Silence.kar"))
                .unwrap()
                .to_string_lossy(),
            "\"C:/x/Simon&Garfunkel-Sound_of_Silence.kar\""
        );
    }

    #[test]
    fn a_target_holding_a_quote_is_refused_rather_than_mangled() {
        let error = cmd_quoted(OsStr::new("C:/x/a\"b.kar")).unwrap_err();
        assert!(error.to_string().contains("double quote"), "{error}");
    }

    #[test]
    fn the_target_is_a_separate_argument_not_part_of_a_command_line() {
        let args = args_of(&command_for(OsStr::new("/x/a song & more.kar")).unwrap());
        assert!(
            args.iter().any(|arg| arg.contains("/x/a song & more.kar")),
            "{args:?}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_url_is_handed_over_whole() {
        let args = args_of(&command_for(OsStr::new("http://127.0.0.1:8178/")).unwrap());
        assert!(
            args.iter().any(|arg| arg == "http://127.0.0.1:8178/"),
            "{args:?}"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_gets_the_empty_title_argument_start_needs() {
        let args = args_of(&command_for(OsStr::new("C:/x/song.kar")).unwrap());
        assert_eq!(args[0], "/c");
        assert_eq!(args[1], "start");
        // Without this, `start` reads the target as the window title and opens nothing.
        assert_eq!(args[2], "");
    }

    /// The regression, asserted where it actually happens: on the *command line* `cmd` will parse,
    /// not on the `argv` Rust never gets to hand over. An assertion that the URL is merely *present*
    /// among the arguments passes against the broken code.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_quotes_the_target_so_cmd_cannot_split_it_on_an_ampersand() {
        let args = args_of(&command_for(OsStr::new(FILTERED)).unwrap());
        assert_eq!(args[3], format!("\"{FILTERED}\""));
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains('&') && !arg.starts_with('"')),
            "an ampersand outside quotes is a command separator to cmd: {args:?}"
        );
    }
}
