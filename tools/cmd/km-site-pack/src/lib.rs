//! Fetches the song files a site links, and builds a package from them.
//!
//! Four stages, each a function a caller may run on its own: [`crawl`] lists the files, [`download`]
//! brings them into a folder, [`unpack`] opens the archives among them, and [`package`] turns the
//! folder into packages through `km-pack`. [`run`] is the four in order.
//!
//! # Nothing here prints
//!
//! Each stage reports progress as an [`Event`] passed to a callback, and its result as a struct. A
//! command line and a page want different things from the same run. The callback returns
//! [`ControlFlow::Break`] to stop the work.
//!
//! # Nothing here is asynchronous
//!
//! A request blocks. A caller on an asynchronous runtime runs a stage on a blocking thread, as it
//! runs `km-pack`'s.
//!
//! The terms this program fetches on are `A person names the site a song file is fetched from` in
//! `docs/decisions/repository.md`.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use url::Url;

pub mod client;
pub mod crawl;
pub mod download;
pub mod links;
pub mod names;
pub mod package;
pub mod robots;
pub mod unpack;

pub use client::Client;
pub use crawl::{CrawlOptions, FileLink, Found, crawl};
pub use download::{Downloaded, download};
pub use names::Extensions;
pub use package::{Built, PackageOptions, Packaged, package};
pub use unpack::{Unpacked, unpack};

/// One step of a run, as it happens.
#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    /// A page is about to be read.
    Page {
        /// The page.
        url: &'a Url,
        /// How many pages have been read so far.
        done: usize,
    },
    /// A file is about to be downloaded, or found in the folder already.
    Download {
        /// The file.
        url: &'a Url,
        /// Its place in the list, from zero.
        index: usize,
        /// How long the list is.
        total: usize,
    },
    /// An archive is about to be opened.
    Unpack {
        /// The archive.
        archive: &'a Path,
    },
    /// The folder's song files are being read.
    Read {
        /// How many have been read.
        done: usize,
        /// How many there are.
        total: usize,
    },
    /// A song is going into a package.
    Build {
        /// Which volume, from zero.
        volume: usize,
        /// How many volumes there are.
        volumes: usize,
        /// The song's place in this volume, from zero.
        done: usize,
        /// How many songs this volume holds.
        total: usize,
    },
    /// A package is being written.
    Write {
        /// The package.
        out: &'a Path,
    },
}

/// Everything one run is told.
#[derive(Debug, Clone)]
pub struct Options {
    /// The site to read. With none, the folder is used as it stands and no request is made.
    pub site: Option<CrawlOptions>,
    /// The folder the files go into, and the one the packages are built from.
    pub folder: PathBuf,
    /// The song-file extensions taken out of an archive.
    pub extensions: Extensions,
    /// How long to leave between two requests.
    pub delay: Duration,
    /// List what a run would download and write nothing.
    pub dry_run: bool,
    /// What to build. With none, the run stops once the folder is filled.
    pub package: Option<PackageOptions>,
}

/// What one run did, stage by stage. A stage that did not run is absent.
#[derive(Debug, Default)]
pub struct Report {
    /// What the crawl found.
    pub found: Option<Found>,
    /// What was downloaded.
    pub downloaded: Option<Downloaded>,
    /// What was unpacked.
    pub unpacked: Option<Unpacked>,
    /// What was packaged.
    pub packaged: Option<Packaged>,
    /// Whether the caller stopped the run.
    pub canceled: bool,
}

/// Runs the stages in order: crawl, download, unpack, package.
///
/// # Errors
///
/// When a stage cannot go on at all: the site cannot be read or answers only a browser, the folder
/// cannot be written, or a package cannot be built. One file that fails is in the report instead.
pub fn run(options: &Options, mut on: impl FnMut(Event<'_>) -> ControlFlow<()>) -> Result<Report> {
    let mut report = Report::default();

    if let Some(site) = &options.site {
        let client = Client::new(options.delay);
        let found = crawl(&client, site, &mut on)?;
        let canceled = found.canceled;
        let files = found.files.clone();
        report.found = Some(found);
        if canceled || options.dry_run {
            report.canceled = canceled;
            return Ok(report);
        }

        let downloaded = download(&client, &files, &options.folder, &mut on)?;
        let canceled = downloaded.canceled;
        report.downloaded = Some(downloaded);
        if canceled {
            report.canceled = true;
            return Ok(report);
        }
    } else if options.dry_run {
        return Ok(report);
    }

    let unpacked = unpack(&options.folder, &options.extensions, &mut on)?;
    let canceled = unpacked.canceled;
    report.unpacked = Some(unpacked);
    if canceled {
        report.canceled = true;
        return Ok(report);
    }

    if let Some(package_options) = &options.package {
        let packaged = package(&options.folder, package_options, &mut on)?;
        report.canceled = packaged.canceled;
        report.packaged = Some(packaged);
    }
    Ok(report)
}
