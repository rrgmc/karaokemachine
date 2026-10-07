//! Starting the lyric sync editor, and reading how it ended.
//!
//! **[`plan`] decides and [`Launcher::run`] does.** The plan is a program, its arguments and where
//! the words go, so every platform's command line is tested on every platform. The words always
//! reach the editor as UTF-8, which is the only encoding it reads.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::machine::Machine;

/// What the page asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The song to put words on.
    pub song: PathBuf,
    /// The words to tap. `None` opens the song's own words.
    pub words: Option<String>,
    /// Keep the timing the song's own words have, where the given words start with them.
    pub resume: bool,
    /// Replace the synced copy that exists.
    pub force: bool,
    /// The data folder the machine reads its settings from, when not its own.
    pub data_dir: Option<PathBuf>,
}

/// How the words reach the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Words {
    /// There are none to hand over.
    None,
    /// Written to the editor's standard input.
    Piped(String),
    /// Written to a file the editor is told to read, and removed when the editor ends.
    File {
        /// The file.
        path: PathBuf,
        /// What goes in it.
        text: String,
    },
}

/// One start of the editor, decided and not yet done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The program to run.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// How the words reach it.
    pub words: Words,
    /// The file the editor's refusal is read from, where it cannot be read from a pipe.
    pub said_in: Option<PathBuf>,
}

/// How the editor ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ended {
    /// What the editor said was wrong, in its own words. `None` is a clean end.
    pub refusal: Option<String>,
}

/// Decides how to start the editor on `machine`.
///
/// **A bundle is started through the system**, which is the macOS rule for one program starting
/// another: the editor gets its own activation and its own place in the Dock. The system hides the
/// editor's standard input, so the words go through a file under `scratch`. It hides the exit
/// status too, so the refusal is read from a second file.
#[must_use]
pub fn plan(machine: &Machine, request: &Request, scratch: &Path) -> Launch {
    let mut editor: Vec<OsString> = Vec::new();
    if let Some(data_dir) = &request.data_dir {
        editor.push("--data-dir".into());
        editor.push(data_dir.into());
    }
    editor.push("--sync".into());
    editor.push(request.song.clone().into());

    match machine {
        Machine::Binary(program) => {
            let words = match &request.words {
                Some(text) => {
                    editor.push("--sync-words".into());
                    editor.push("-".into());
                    Words::Piped(text.clone())
                }
                None => Words::None,
            };
            push_choices(&mut editor, request);
            Launch {
                program: program.clone(),
                args: editor,
                words,
                said_in: None,
            }
        }
        Machine::Bundle(bundle) => {
            let words = match &request.words {
                Some(text) => {
                    let path = scratch.join("words.txt");
                    editor.push("--sync-words".into());
                    editor.push(path.clone().into());
                    Words::File {
                        path,
                        text: text.clone(),
                    }
                }
                None => Words::None,
            };
            push_choices(&mut editor, request);
            let said_in = scratch.join("editor.err");
            // `-n` starts a second copy where the machine is already running, which would otherwise
            // be brought forward with the arguments dropped. `-W` waits for the editor to close.
            let mut args: Vec<OsString> = vec![
                "-n".into(),
                "-W".into(),
                "-a".into(),
                bundle.clone().into(),
                "--stderr".into(),
                said_in.clone().into(),
                "--args".into(),
            ];
            args.extend(editor);
            Launch {
                program: PathBuf::from("/usr/bin/open"),
                args,
                words,
                said_in: Some(said_in),
            }
        }
    }
}

/// The two flags that follow the words.
fn push_choices(args: &mut Vec<OsString>, request: &Request) {
    // The editor accepts `--sync-continue` only with words to continue with.
    if request.resume && request.words.is_some() {
        args.push("--sync-continue".into());
    }
    if request.force {
        args.push("--sync-force".into());
    }
}

/// Something that can run a [`Launch`] and wait for the editor to close.
///
/// A trait so a test of the pages starts no process.
pub trait Launcher: Send + Sync + std::fmt::Debug {
    /// Runs the editor and blocks until it has closed.
    fn run(&self, launch: &Launch) -> Ended;
}

/// The launcher that starts a real process.
#[derive(Debug, Default, Clone, Copy)]
pub struct Process;

impl Launcher for Process {
    fn run(&self, launch: &Launch) -> Ended {
        match run_process(launch) {
            Ok(ended) => ended,
            Err(error) => Ended {
                refusal: Some(format!("{}: {error}", launch.program.display())),
            },
        }
    }
}

/// Starts the process, hands it the words, and waits.
fn run_process(launch: &Launch) -> std::io::Result<Ended> {
    if let Words::File { path, text } = &launch.words {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)?;
    }
    if let Some(said_in) = &launch.said_in {
        let _ = std::fs::remove_file(said_in);
    }

    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        // The machine writes its log to standard output. Nobody reads it here, and a pipe nobody
        // reads fills and stops the editor.
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .stdin(match launch.words {
            Words::Piped(_) => Stdio::piped(),
            _ => Stdio::null(),
        });
    let mut child = command.spawn()?;
    if let (Words::Piped(text), Some(mut stdin)) = (&launch.words, child.stdin.take()) {
        // An editor that refused before reading has closed the pipe, and its refusal is on
        // standard error. The write's own failure says nothing more.
        let _ = stdin.write_all(text.as_bytes());
    }
    let output = child.wait_with_output()?;

    let mut said = String::from_utf8_lossy(&output.stderr).into_owned();
    if let Some(said_in) = &launch.said_in {
        said.push_str(&std::fs::read_to_string(said_in).unwrap_or_default());
        let _ = std::fs::remove_file(said_in);
    }
    if let Words::File { path, .. } = &launch.words {
        let _ = std::fs::remove_file(path);
    }

    let refusal = refusal_in(&said).or_else(|| {
        (!output.status.success()).then(|| match said.trim().lines().last() {
            Some(last) => last.trim().to_owned(),
            None => output.status.to_string(),
        })
    });
    Ok(Ended { refusal })
}

/// The sentence the machine gives for refusing to start, where its output holds one.
///
/// The machine ends a refused start with `Error:` and the reason on one line.
#[must_use]
pub fn refusal_in(said: &str) -> Option<String> {
    said.lines()
        .find_map(|line| line.trim().strip_prefix("Error: "))
        .map(|reason| reason.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            song: PathBuf::from("/tunes/karaoke/song.mid"),
            words: Some("la la\n".to_owned()),
            resume: false,
            force: false,
            data_dir: None,
        }
    }

    fn args(launch: &Launch) -> Vec<String> {
        launch
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn an_executable_takes_the_words_on_standard_input() {
        let machine = Machine::Binary(PathBuf::from("/opt/karaokemachine/karaokemachine"));
        let launch = plan(&machine, &request(), Path::new("/scratch"));

        assert_eq!(
            launch.program,
            Path::new("/opt/karaokemachine/karaokemachine")
        );
        assert_eq!(
            args(&launch),
            ["--sync", "/tunes/karaoke/song.mid", "--sync-words", "-"]
        );
        assert_eq!(launch.words, Words::Piped("la la\n".to_owned()));
        assert_eq!(launch.said_in, None);
    }

    #[test]
    fn no_words_opens_the_songs_own_and_never_asks_to_continue() {
        let machine = Machine::Binary(PathBuf::from("karaokemachine"));
        let launch = plan(
            &machine,
            &Request {
                words: None,
                resume: true,
                force: true,
                data_dir: Some(PathBuf::from("/scratch/data")),
                ..request()
            },
            Path::new("/scratch"),
        );

        assert_eq!(
            args(&launch),
            [
                "--data-dir",
                "/scratch/data",
                "--sync",
                "/tunes/karaoke/song.mid",
                "--sync-force"
            ]
        );
        assert_eq!(launch.words, Words::None);
    }

    #[test]
    fn a_bundle_is_started_through_the_system_with_the_words_in_a_file() {
        let machine = Machine::Bundle(PathBuf::from("/Applications/Karaoke Machine.app"));
        let launch = plan(
            &machine,
            &Request {
                resume: true,
                ..request()
            },
            Path::new("/scratch"),
        );

        assert_eq!(launch.program, Path::new("/usr/bin/open"));
        let words = Path::new("/scratch").join("words.txt");
        let said = Path::new("/scratch").join("editor.err");
        assert_eq!(
            args(&launch),
            [
                "-n",
                "-W",
                "-a",
                "/Applications/Karaoke Machine.app",
                "--stderr",
                &said.to_string_lossy(),
                "--args",
                "--sync",
                "/tunes/karaoke/song.mid",
                "--sync-words",
                &words.to_string_lossy(),
                "--sync-continue",
            ]
        );
        assert_eq!(
            launch.words,
            Words::File {
                path: words,
                text: "la la\n".to_owned()
            }
        );
        assert_eq!(launch.said_in, Some(said));
    }

    #[test]
    fn a_refusal_is_the_machines_own_sentence() {
        assert_eq!(
            refusal_in("Error: song.kar exists; pass --sync-force to replace it\n"),
            Some("song.kar exists; pass --sync-force to replace it".to_owned())
        );
        assert_eq!(refusal_in("a warning\n"), None);
    }

    #[test]
    fn a_program_that_is_not_there_is_a_refusal_and_not_a_failure() {
        let launch = Launch {
            program: PathBuf::from("/definitely/not/a/real/program"),
            args: Vec::new(),
            words: Words::None,
            said_in: None,
        };
        assert!(Process.run(&launch).refusal.is_some());
    }
}
