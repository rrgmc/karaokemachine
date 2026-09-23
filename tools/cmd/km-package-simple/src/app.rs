//! The tool's state, and the two pieces of work that run off the request path.
//!
//! **One folder at a time, and one job at a time.** Reading a folder and building its packages are
//! both minutes of work on a large folder, so each runs on a blocking thread and reports into
//! [`Phase`]. A page polls that phase, and every other request answers from the lock in
//! microseconds.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use km_locale::Locale;
use km_pack::{BuildEvent, BuildOptions, DescribeOptions, Skipped};

use crate::session::{PackageForm, Planned, Session};
use crate::settings::Settings;

/// What the tool is doing.
#[derive(Debug, Clone, Default)]
pub enum Phase {
    /// No folder yet.
    #[default]
    Empty,
    /// Reading a folder: MIDI files read so far, and how many there are.
    Reading {
        /// The folder being read.
        folder: PathBuf,
        /// MIDI files read.
        done: usize,
        /// MIDI files in all.
        total: usize,
    },
    /// A folder is read and its songs are listed.
    Ready,
    /// Writing packages.
    Building {
        /// Which volume, from 1.
        volume: usize,
        /// How many volumes.
        volumes: usize,
        /// Songs of this volume read so far.
        done: usize,
        /// Songs in this volume.
        total: usize,
    },
    /// The packages are written.
    Built(Built),
}

/// What a build wrote.
#[derive(Debug, Clone, Default)]
pub struct Built {
    /// Every package file, with how many songs it holds.
    pub files: Vec<(PathBuf, usize)>,
    /// Songs a build could not put in, and why, in `km-pack`'s words.
    pub skipped: Vec<Skipped>,
}

/// Everything behind the lock.
#[derive(Debug, Default)]
pub struct Inner {
    /// What is happening.
    pub phase: Phase,
    /// The folder's songs, once read.
    pub session: Option<Session>,
    /// What the package form holds.
    pub form: Option<PackageForm>,
    /// The last thing that went wrong, shown once and then cleared.
    pub error: Option<String>,
    /// What is remembered between runs.
    pub settings: Settings,
}

/// The whole tool's state, shared by every request.
#[derive(Debug)]
pub struct App {
    inner: Mutex<Inner>,
    settings_path: Option<PathBuf>,
    /// Asked when somebody presses Quit.
    pub stop: Stop,
    /// Whether the page is inside this tool's own window, where closing it is the quit.
    pub windowed: std::sync::atomic::AtomicBool,
    /// Where the pages are served, for the banner and the window.
    pub url: String,
}

impl App {
    /// A fresh tool, remembering what `settings_path` holds.
    #[must_use]
    pub fn new(settings_path: Option<PathBuf>, url: String) -> Arc<Self> {
        let settings = Settings::load(settings_path.as_deref());
        Arc::new(Self {
            inner: Mutex::new(Inner {
                settings,
                ..Inner::default()
            }),
            settings_path,
            stop: Stop::default(),
            windowed: std::sync::atomic::AtomicBool::new(false),
            url,
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
        inner.settings.save(self.settings_path.as_deref());
    }

    /// Starts reading a folder, unless a job is already running.
    pub fn read_folder(self: &Arc<Self>, folder: PathBuf) -> Result<(), &'static str> {
        {
            let mut inner = self.lock();
            if matches!(inner.phase, Phase::Reading { .. } | Phase::Building { .. }) {
                return Err("said-busy");
            }
            if !folder.is_dir() {
                return Err("said-no-folder");
            }
            inner.phase = Phase::Reading {
                folder: folder.clone(),
                done: 0,
                total: 0,
            };
            inner.session = None;
            inner.error = None;
            inner.settings.last_folder = Some(folder.clone());
            inner.settings.save(self.settings_path.as_deref());
        }

        let app = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let options = DescribeOptions {
                unbounded: true,
                ..DescribeOptions::default()
            };
            let described = km_pack::describe(&folder, &options, |done, total| {
                if let Phase::Reading {
                    done: at,
                    total: all,
                    ..
                } = &mut app.lock().phase
                {
                    *at = done;
                    *all = total;
                }
            });
            let mut inner = app.lock();
            match described {
                Ok(description) => {
                    inner.form = Some(PackageForm::for_folder(&folder));
                    inner.session = Some(Session::from_description(folder, description));
                    inner.phase = Phase::Ready;
                }
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "reading the folder failed");
                    inner.error = Some(format!("{error:#}"));
                    inner.phase = Phase::Empty;
                }
            }
        });
        Ok(())
    }

    /// Starts writing the packages the session plans, unless a job is already running.
    pub fn build(self: &Arc<Self>, form: PackageForm) -> Result<(), &'static str> {
        let (plan, folder) = {
            let mut inner = self.lock();
            if !matches!(inner.phase, Phase::Ready) {
                return Err("said-busy");
            }
            let Some(session) = &inner.session else {
                return Err("said-no-folder");
            };
            if session.kept() == 0 {
                return Err("said-nothing-kept");
            }
            if form.name.trim().is_empty() {
                return Err("said-no-name");
            }
            let plan = session.plan(&form, km_kmpkg::PackageMeta::new_id());
            let folder = session.folder.clone();
            inner.phase = Phase::Building {
                volume: 1,
                volumes: plan.len(),
                done: 0,
                total: 0,
            };
            inner.form = Some(form);
            inner.error = None;
            (plan, folder)
        };

        let app = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let written = build_all(&app, &folder, &plan);
            let mut inner = app.lock();
            match written {
                Ok(built) => inner.phase = Phase::Built(built),
                Err(error) => {
                    tracing::warn!(%error, "the build failed");
                    inner.error = Some(error);
                    inner.phase = Phase::Ready;
                }
            }
        });
        Ok(())
    }

    /// Back to the song list after a build, to change something and build again.
    pub fn back_to_songs(&self) {
        let mut inner = self.lock();
        if matches!(inner.phase, Phase::Built(_)) {
            inner.phase = Phase::Ready;
        }
    }

    /// Forgets the folder, back to the first page.
    pub fn close(&self) {
        let mut inner = self.lock();
        if matches!(inner.phase, Phase::Ready | Phase::Built(_)) {
            inner.phase = Phase::Empty;
            inner.session = None;
            inner.form = None;
            inner.error = None;
        }
    }
}

/// Writes every planned volume, in order, reporting into the phase as it goes.
fn build_all(app: &Arc<App>, folder: &Path, plan: &[Planned]) -> Result<Built, String> {
    let mut built = Built::default();
    for (index, planned) in plan.iter().enumerate() {
        {
            let mut inner = app.lock();
            inner.phase = Phase::Building {
                volume: index + 1,
                volumes: plan.len(),
                done: 0,
                total: planned.spec.songs.len(),
            };
        }
        let outcome = km_pack::build::build(
            &planned.spec,
            &BuildOptions {
                base: folder,
                out: Some(&planned.out),
                dry_run: false,
                measure_loudness: true,
                write_listing: true,
                flags: km_kmpkg::PackageFlags::NONE,
            },
            |event| {
                if let BuildEvent::Song { index, .. } = event
                    && let Phase::Building { done, .. } = &mut app.lock().phase
                {
                    *done = index;
                }
                ControlFlow::Continue(())
            },
        )
        .map_err(|error| format!("{error:#}"))?;

        if !outcome.problems.is_empty() {
            return Err(outcome.problems.join("; "));
        }
        if !outcome.wrote() {
            return Err(format!(
                "{} was not written: no song went in",
                planned.out.display()
            ));
        }
        built
            .files
            .push((outcome.out_path.clone(), outcome.written));
        built.skipped.extend(outcome.skipped);
    }
    Ok(built)
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
