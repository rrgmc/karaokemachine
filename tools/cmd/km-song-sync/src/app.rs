//! The tool's state, and the one piece of work that runs off the request path.
//!
//! **One editor at a time.** Two would play through one audio device. Starting one puts the wait
//! for it on a blocking thread, and the page polls [`Editor`] until it has closed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use km_locale::Locale;

use crate::launch::{Launcher, Request};
use crate::machine::Machine;
use crate::settings::Settings;

/// What the editor is doing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Editor {
    /// Nothing has been started.
    #[default]
    Idle,
    /// The editor is open on a song.
    Running {
        /// The song it is open on.
        song: PathBuf,
    },
    /// The editor has closed.
    Ended(Outcome),
}

/// How a run of the editor ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It wrote the synced copy.
    Saved {
        /// The file it wrote.
        out: PathBuf,
    },
    /// It closed without writing anything.
    Nothing {
        /// The song it was open on.
        song: PathBuf,
    },
    /// It refused to start, or stopped on a fault.
    Failed {
        /// The song it was started on.
        song: PathBuf,
        /// Why, in the machine's own words.
        said: String,
    },
}

/// Everything behind the lock.
#[derive(Debug, Default)]
pub struct Inner {
    /// What the editor is doing.
    pub editor: Editor,
    /// What is remembered between runs.
    pub settings: Settings,
}

/// What a run is given from outside: its command line and its surroundings.
#[derive(Debug)]
pub struct Config {
    /// Where the settings file is. `None` remembers nothing.
    pub settings_path: Option<PathBuf>,
    /// Where the pages are served.
    pub url: String,
    /// The folder named on the command line.
    pub folder: Option<PathBuf>,
    /// The folder the program was started in.
    pub current_dir: Option<PathBuf>,
    /// The folder the program's own executable is in.
    pub exe_dir: Option<PathBuf>,
    /// The machine named on the command line or in the environment.
    pub machine_exe: Option<PathBuf>,
    /// The data folder the machine is told to use, when not its own.
    pub machine_data_dir: Option<PathBuf>,
    /// A folder for files that last as long as one editor run.
    pub scratch: PathBuf,
    /// What starts the editor.
    pub launcher: Arc<dyn Launcher>,
}

/// The whole tool's state, shared by every request.
#[derive(Debug)]
pub struct App {
    inner: Mutex<Inner>,
    config: Config,
    /// Asked when somebody presses Quit.
    pub stop: Stop,
    /// Whether the page is inside this tool's own window, where closing it is the quit.
    pub windowed: std::sync::atomic::AtomicBool,
    /// Where the pages are served, for the banner and the window.
    pub url: String,
}

/// What the page asks a start for.
#[derive(Debug, Clone, Default)]
pub struct Start {
    /// The song.
    pub song: PathBuf,
    /// What the words box holds, when its box says to use it.
    pub pasted: Option<String>,
    /// Keep the timing the song's own words have.
    pub resume: bool,
    /// Replace the synced copy that exists.
    pub force: bool,
    /// The title somebody typed.
    pub title: Option<String>,
    /// The artist somebody typed.
    pub artist: Option<String>,
    /// The language somebody chose, as its code.
    pub language: Option<String>,
}

/// What somebody typed for a name, where it is a name and not the one the song states.
///
/// A blank field and an unchanged one both pass nothing, so the song's own name stands.
fn given(stated: Option<&str>, typed: Option<String>) -> Option<String> {
    let typed = typed?;
    let typed = typed.trim();
    (!typed.is_empty() && stated.map(str::trim) != Some(typed)).then(|| typed.to_owned())
}

impl App {
    /// A fresh tool, remembering what the settings file holds.
    #[must_use]
    pub fn new(config: Config) -> Arc<Self> {
        let settings = Settings::load(config.settings_path.as_deref());
        Arc::new(Self {
            inner: Mutex::new(Inner {
                settings,
                ..Inner::default()
            }),
            url: config.url.clone(),
            config,
            stop: Stop::default(),
            windowed: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// The state, locked. A poisoned lock is taken back rather than passed on: every write under
    /// it is a whole value, so a panic cannot have left one half written.
    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The language a page is drawn in: the stored one, or the browser's.
    #[must_use]
    pub fn locale(&self, accept_language: Option<&str>) -> Locale {
        let stored = self.lock().settings.locale.clone();
        stored
            .as_deref()
            .and_then(Locale::parse)
            .or_else(|| accept_language.and_then(km_locale::negotiate))
            .unwrap_or(Locale::English)
    }

    /// Stores the language the pages speak.
    pub fn set_locale(&self, locale: Locale) {
        let mut inner = self.lock();
        inner.settings.locale = Some(locale.tag().to_owned());
        inner.settings.save(self.config.settings_path.as_deref());
    }

    /// The machine, looked for again each time, so one installed while the page is open is found.
    #[must_use]
    pub fn machine(&self) -> Option<Machine> {
        crate::machine::find(
            self.config.machine_exe.as_deref(),
            self.config.exe_dir.as_deref(),
            std::env::var_os("PATH").as_deref(),
        )
    }

    /// The folder the browser opens on.
    #[must_use]
    pub fn start_folder(&self) -> Option<PathBuf> {
        let last = self.lock().settings.last_folder.clone();
        start_folder(
            self.config.folder.as_deref(),
            self.config.current_dir.as_deref(),
            self.config.exe_dir.as_deref(),
            last.as_deref(),
            km_folders::start().as_deref(),
        )
    }

    /// Remembers the folder the browser is on, so the next run opens there.
    ///
    /// The folder, never a song: a settings file holds no path to one song. The file is written
    /// only when the folder changed, because a page turn asks for the same folder again.
    pub fn remember_folder(&self, folder: &Path) {
        let mut inner = self.lock();
        if inner.settings.last_folder.as_deref() == Some(folder) {
            return;
        }
        inner.settings.last_folder = Some(folder.to_path_buf());
        inner.settings.save(self.config.settings_path.as_deref());
    }

    /// Starts the editor on a song, unless one is open or the start makes no sense.
    ///
    /// **The words box wins when it is ticked, then the text file beside the song, then the song's
    /// own.** A ticked box that holds nothing is refused, since somebody meant words to be there. Every check
    /// the page makes is made again here, because the folder may have changed under the page.
    ///
    /// **A typed title, artist or language reaches the editor only where it differs from the
    /// song's own.**
    ///
    /// # Errors
    ///
    /// The catalog key of the sentence that says why not.
    pub fn start(self: &Arc<Self>, start: Start) -> Result<(), &'static str> {
        let song = start.song;
        if !crate::rows::is_song(&song) || !song.is_file() {
            return Err("said-no-song");
        }
        let Some(machine) = self.machine() else {
            return Err("said-no-machine");
        };

        let pasted = start.pasted.map(|pasted| pasted.replace("\r\n", "\n"));
        if pasted
            .as_deref()
            .is_some_and(|pasted| pasted.trim().is_empty())
        {
            return Err("said-box-empty");
        }
        let words = pasted.or_else(|| crate::rows::words_beside(&song).map(|(_, words)| words));
        let found = crate::rows::found(&song);
        if words.is_none() {
            match found.holds {
                crate::rows::Holds::Words => {}
                crate::rows::Holds::NoWords => return Err("said-no-words"),
                crate::rows::Holds::NotMidi => return Err("said-not-midi"),
            }
        }

        let out = km_song::kar_write::synced_path(&song);
        if out.exists() && !start.force {
            return Err("said-output-exists");
        }

        let request = Request {
            song: song.clone(),
            words,
            resume: start.resume,
            force: start.force,
            title: given(found.title.as_deref(), start.title),
            artist: given(found.artist.as_deref(), start.artist),
            // Only a code from the table is passed on. The list on the page offers nothing else.
            // The song's own language is compared as a code, because the song may spell it `ENGL`.
            language: given(
                found
                    .language
                    .as_deref()
                    .and_then(km_kmpkg::Language::from_declared)
                    .map(km_kmpkg::Language::code),
                start
                    .language
                    .as_deref()
                    .and_then(km_kmpkg::Language::parse)
                    .map(|language| language.code().to_owned()),
            ),
            data_dir: self.config.machine_data_dir.clone(),
        };
        let launch = crate::launch::plan(&machine, &request, &self.config.scratch);

        {
            let mut inner = self.lock();
            if matches!(inner.editor, Editor::Running { .. }) {
                return Err("said-busy");
            }
            inner.editor = Editor::Running { song: song.clone() };
        }

        let app = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let started = SystemTime::now();
            let ended = app.config.launcher.run(&launch);
            let outcome = match ended.refusal {
                Some(said) => {
                    tracing::warn!(song = %song.display(), said, "the editor did not run");
                    Outcome::Failed { song, said }
                }
                None if written_since(&out, started) => Outcome::Saved { out },
                None => Outcome::Nothing { song },
            };
            app.lock().editor = Editor::Ended(outcome);
        });
        Ok(())
    }
}

/// Whether `file` was written at or after `since`.
///
/// Two seconds of slack, because some file systems keep a file's time no finer than that.
fn written_since(file: &Path, since: SystemTime) -> bool {
    let since = since
        .checked_sub(std::time::Duration::from_secs(2))
        .unwrap_or(since);
    std::fs::metadata(file)
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| modified >= since)
}

/// The folder the browser opens on: the first of these that is a folder.
///
/// * the folder named on the command line;
/// * the folder the browser was on last;
/// * the folder the program was started in, when somebody chose it;
/// * the person's home folder.
///
/// **A double-click chooses no folder.** It starts the program in its own folder or at the top of
/// the disk, and neither is where somebody keeps songs. So the current folder counts only when it
/// is neither of those.
#[must_use]
pub fn start_folder(
    named: Option<&Path>,
    current: Option<&Path>,
    exe_dir: Option<&Path>,
    last: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let chosen = current.filter(|dir| dir.parent().is_some() && Some(*dir) != exe_dir);
    [named, last, chosen, home]
        .into_iter()
        .flatten()
        .find(|dir| dir.is_dir())
        .map(Path::to_path_buf)
}

/// How a Quit asks the server to stop. Cloned into every place that can ask.
#[derive(Debug, Clone)]
pub struct Stop(tokio::sync::watch::Sender<bool>);

impl Default for Stop {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }
}

impl Stop {
    /// Asks for the stop. Asking twice is the same as asking once.
    pub fn ask(&self) {
        self.0.send_replace(true);
    }

    /// Waits until somebody asks.
    pub async fn asked(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|asked| *asked).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use km_testkit::Scratch;

    #[test]
    fn the_browser_opens_where_the_person_stands_unless_nobody_chose_it() {
        let scratch = Scratch::new("sync-start-folder");
        let dir = |name: &str| {
            let dir = scratch.join(name);
            std::fs::create_dir_all(&dir).expect("making a test folder");
            dir
        };
        let (named, current, program, last, home) = (
            dir("named"),
            dir("current"),
            dir("program"),
            dir("last"),
            dir("home"),
        );
        let gone = scratch.join("gone");
        let root = current.ancestors().last().expect("a root");

        let from = |named: Option<&Path>, current: Option<&Path>, last: Option<&Path>| {
            start_folder(named, current, Some(&program), last, Some(&home))
        };
        assert_eq!(from(Some(&named), Some(&current), Some(&last)), Some(named));
        assert_eq!(from(None, Some(&current), None), Some(current.clone()));
        // Where the browser was last wins over where the program was started.
        assert_eq!(from(None, Some(&current), Some(&last)), Some(last.clone()));
        // Started by a double-click: in the program's own folder, or at the top of the disk.
        assert_eq!(from(None, Some(&program), None), Some(home.clone()));
        assert_eq!(from(None, Some(root), Some(&last)), Some(last));
        assert_eq!(from(None, Some(&program), Some(&gone)), Some(home.clone()));
        assert_eq!(from(Some(&gone), None, None), Some(home));
    }
}
