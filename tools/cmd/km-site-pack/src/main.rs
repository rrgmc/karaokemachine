//! The command line over the `km_site_pack` library.

use std::collections::BTreeMap;
use std::io::IsTerminal as _;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser;
use km_pack::Rejection;
use km_site_pack::{CrawlOptions, Event, Extensions, Options, PackageOptions, Report};
use url::Url;

/// Fetch the song files a site links, and build a package from them.
///
/// Reads the pages of one site, downloads every song file and archive they link into a folder,
/// opens the archives, and builds a package from the songs that have words.
///
/// The site's robots.txt is honoured. A file already in the folder is not downloaded again, so a
/// run that was stopped carries on where it was.
#[derive(Parser)]
#[command(name = "km-site-pack", version, about, long_about)]
struct Cli {
    /// The address of the site, or of one folder of it. Pages outside that folder are not read.
    #[arg(
        required_unless_present = "from_folder",
        conflicts_with = "from_folder"
    )]
    url: Option<String>,

    /// The folder the files are downloaded into.
    #[arg(
        required_unless_present = "from_folder",
        conflicts_with = "from_folder"
    )]
    folder: Option<PathBuf>,

    /// Use a folder as it stands: open its archives and build the package, with no download.
    #[arg(long, value_name = "FOLDER")]
    from_folder: Option<PathBuf>,

    /// How many links deep a page may be from the address given.
    #[arg(long, default_value_t = 2)]
    depth: u32,

    /// The most pages one run reads.
    #[arg(long, default_value_t = 2000)]
    max_pages: usize,

    /// The song-file extensions to take. A .zip archive is always opened for them.
    #[arg(long, default_value = "kar,mid,midi", value_name = "LIST")]
    ext: String,

    /// Milliseconds to leave between two requests. A longer wait the site asks for is honoured.
    #[arg(long, default_value_t = 500, value_name = "MS")]
    delay_ms: u64,

    /// The package's name. The folder's name when not given.
    #[arg(long)]
    name: Option<String>,

    /// Who publishes the package.
    #[arg(long)]
    publisher: Option<String>,

    /// The package's version.
    #[arg(long, default_value = "1.0.0", value_name = "VERSION")]
    package_version: String,

    /// The language of a song whose file does not give one, as an ISO 639-1 code.
    #[arg(long, default_value = "und", value_name = "CODE")]
    default_language: String,

    /// Leave out a song whose suitability is below this number, from 0 to 10.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=10))]
    min_suitability: Option<u8>,

    /// Put a file with no words into the package as well.
    #[arg(long)]
    keep_wordless: bool,

    /// Where the package is written. The folder's parent when not given.
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// Download and unpack, and build no package.
    #[arg(long)]
    no_package: bool,

    /// List what a run would download, and write nothing.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("km-site-pack: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the command. `Ok(false)` is a run that finished and has nothing to show for it.
fn run(cli: &Cli) -> Result<bool> {
    let extensions = Extensions::parse(&cli.ext);
    let site = match &cli.url {
        Some(raw) => {
            let start = parse_address(raw)?;
            Some(CrawlOptions {
                depth: cli.depth,
                max_pages: cli.max_pages,
                extensions: extensions.clone(),
                ..CrawlOptions::new(start)
            })
        }
        None => None,
    };
    let folder = cli
        .from_folder
        .clone()
        .or_else(|| cli.folder.clone())
        .context("no folder was given")?;
    let package = (!cli.no_package).then(|| PackageOptions {
        name: cli.name.clone(),
        version: cli.package_version.clone(),
        publisher: cli.publisher.clone(),
        default_language: cli.default_language.clone(),
        min_suitability: cli.min_suitability,
        keep_wordless: cli.keep_wordless,
        out_dir: cli.out_dir.clone(),
    });
    let options = Options {
        site,
        folder,
        extensions,
        delay: Duration::from_millis(cli.delay_ms),
        dry_run: cli.dry_run,
        package,
    };

    let report = km_site_pack::run(&options, progress())?;
    Ok(summarize(&options, &report))
}

/// Reads the address a person typed, which often has no scheme in front of it.
fn parse_address(raw: &str) -> Result<Url> {
    let raw = raw.trim();
    let spelled = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&spelled).with_context(|| format!("{raw:?} is not an address"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        anyhow::bail!("{raw:?} is not an http or https address");
    }
    Ok(url)
}

/// Progress on standard error, on one line where that is a terminal.
fn progress() -> impl FnMut(Event<'_>) -> ControlFlow<()> {
    let live = std::io::stderr().is_terminal();
    let mut stage = "";
    move |event| {
        let (name, line) = match event {
            Event::Page { url, done } => ("pages", format!("reading page {}: {url}", done + 1)),
            Event::Download { url, index, total } => {
                ("files", format!("file {} of {total}: {url}", index + 1))
            }
            Event::Unpack { archive } => ("archives", format!("opening {}", archive.display())),
            Event::Read { done, total } => ("songs", format!("reading song {done} of {total}")),
            Event::Build {
                volume,
                volumes,
                done,
                total,
            } => (
                "package",
                format!(
                    "packaging song {} of {total}, volume {} of {volumes}",
                    done + 1,
                    volume + 1
                ),
            ),
            Event::Write { out } => ("write", format!("writing {}", out.display())),
        };
        if live {
            if name != stage && !stage.is_empty() {
                eprintln!();
            }
            // The line is cut to a width every terminal has, so it never wraps and scrolls.
            let shown: String = line.chars().take(78).collect();
            eprint!("\r{shown:<78}");
        } else if name != stage || matches!(event, Event::Unpack { .. } | Event::Write { .. }) {
            eprintln!("{line}");
        }
        stage = name;
        ControlFlow::Continue(())
    }
}

/// Prints what the run did, and says whether it has something to show.
fn summarize(options: &Options, report: &Report) -> bool {
    if std::io::stderr().is_terminal() {
        eprintln!();
    }
    let mut good = true;

    if let Some(found) = &report.found {
        println!("pages read       {}", found.pages_read);
        println!("wait per request {} ms", found.delay.as_millis());
        if !found.pages_failed.is_empty() {
            println!("pages not read   {}", found.pages_failed.len());
            for (url, why) in found.pages_failed.iter().take(5) {
                println!("    {url}: {why}");
            }
        }
        if found.disallowed > 0 {
            println!("left alone       {} (robots.txt)", found.disallowed);
        }
        if found.capped {
            println!("stopped at the page limit; --max-pages raises it");
        }
        println!("files found      {}", found.files.len());
        if options.dry_run {
            for file in &found.files {
                println!("    {}", file.url);
            }
            println!("\ndry run: nothing written");
            return true;
        }
        good &= !found.files.is_empty();
    }

    if let Some(downloaded) = &report.downloaded {
        println!("downloaded       {}", downloaded.written.len());
        if downloaded.kept > 0 {
            println!("already there    {}", downloaded.kept);
        }
        if !downloaded.refused.is_empty() {
            println!("not downloaded   {}", downloaded.refused.len());
            for (url, why) in &downloaded.refused {
                println!("    {url}: {why}");
            }
        }
    }

    if let Some(unpacked) = &report.unpacked
        && (unpacked.archives > 0 || unpacked.kept > 0 || !unpacked.skipped.is_empty())
    {
        println!(
            "archives opened  {} ({} song file(s))",
            unpacked.archives,
            unpacked.written.len()
        );
        if unpacked.kept > 0 {
            println!("already opened   {}", unpacked.kept);
        }
        for (what, why) in &unpacked.skipped {
            println!("    {what}: {why}");
        }
    }

    if let Some(packaged) = &report.packaged {
        print_rejected(&packaged.rejected);
        if !packaged.skipped.is_empty() {
            println!("skipped          {}", packaged.skipped.len());
            for skipped in packaged.skipped.iter().take(10) {
                println!("    {}: {}", skipped.source.display(), skipped.why);
            }
        }
        if packaged.packages.is_empty() && !report.canceled {
            println!("\nno song with words was found, so no package was written");
            good = false;
        }
        for built in &packaged.packages {
            println!(
                "\nwrote {} with {} song(s)",
                built.path.display(),
                built.songs
            );
            println!("      {}", built.description.display());
            if let Some(listing) = &built.listing {
                println!("      {}", listing.display());
            }
        }
    }

    if report.canceled {
        println!("\nstopped before the end");
        good = false;
    }
    good
}

/// The files left out of the package, counted by reason and named where a person will ask which.
fn print_rejected(rejected: &[(PathBuf, Rejection)]) {
    if rejected.is_empty() {
        return;
    }
    println!("left out         {}", rejected.len());
    let mut by_reason: BTreeMap<String, Vec<&PathBuf>> = BTreeMap::new();
    for (path, reason) in rejected {
        let key = match reason {
            Rejection::LowSuitability(_) => "rated below the minimum".to_owned(),
            Rejection::DuplicateOf(_) => "duplicate of another file".to_owned(),
            other => other.to_string(),
        };
        by_reason.entry(key).or_default().push(path);
    }
    for (reason, paths) in by_reason {
        println!("  {:>6}  {reason}", paths.len());
        // A file with no words is named in full: it is the one a person goes to look at.
        let shown = if reason == Rejection::NoLyrics.to_string() {
            paths.len()
        } else {
            3
        };
        for path in paths.iter().take(shown) {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            println!("          {name}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_typed_without_a_scheme_is_read_as_https() {
        let url = parse_address("example.com/songs/").expect("an address");
        assert_eq!(url.as_str(), "https://example.com/songs/");
        assert!(parse_address("ftp://example.com/").is_err());
        assert!(parse_address("not an address").is_err());
    }

    #[test]
    fn the_command_line_is_consistent() {
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
    }
}
