//! The `km-wallpaper-pack` command.
//!
//! Dispatch, logging, and a printed summary. Everything that can be tested without a process lives in
//! the library beside this.

use std::process::ExitCode;

use clap::Parser;
use km_wallpaper_pack::cache::Cache;
use km_wallpaper_pack::cli::{Cli, Command};
use km_wallpaper_pack::commands::{self, Analysis};
use km_wallpaper_pack::providers::Keys;
use km_wallpaper_pack::{Config, Result};

fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(cli.log_filter())
        .with_target(false)
        // stdout is for the answer, especially under `--json`; logs go to stderr where a pipe cannot
        // confuse them for output.
        .with_writer(std::io::stderr)
        .init();

    // rustls' crypto provider is installed by `providers::client`, which is the only thing that
    // builds an HTTP client — so no caller can forget it, including a test.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("could not start the async runtime: {error}");
            return ExitCode::from(1);
        }
    };

    match runtime.block_on(run(&cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code() as u8)
        }
    }
}

async fn run(cli: &Cli) -> Result<()> {
    // The CPU pool is sized here rather than per call site, so `--jobs` means one thing.
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(cli.parallelism())
        .build_global();

    match &cli.command {
        Command::Fetch { config, refresh } => {
            let (config, cache) = prepare(cli, config)?;
            let count = commands::fetch(
                &config,
                &cache,
                &Keys::from_env(),
                *refresh,
                cli.dry_run,
                None,
            )
            .await?;
            report(cli, "fetch", &[("downloaded", count.to_string())]);
        }
        Command::Analyze { config, remeasure } => {
            let (config, cache) = prepare(cli, config)?;
            let analysis =
                commands::analyze(&config, &cache, &cli.out, *remeasure, cli.dry_run, None)?;
            summarize_analysis(cli, &analysis);
        }
        Command::Build { config, force } => {
            let (config, cache) = prepare(cli, config)?;
            let analysis = read_analysis(cli)?;
            let manifest = commands::build(
                &config,
                &cache,
                &analysis,
                &cli.out,
                *force,
                cli.dry_run,
                km_wallpaper_pack::manifest::generated_now(),
            )?;
            copy_zip(cli, &config);
            report(
                cli,
                "build",
                &[
                    ("images", manifest.images.len().to_string()),
                    ("out", cli.out.display().to_string()),
                    ("zip_dest", cli.zip_dest.display().to_string()),
                ],
            );
        }
        Command::All {
            config,
            force,
            remeasure,
        } => {
            let (config, cache) = prepare(cli, config)?;
            let downloaded =
                commands::fetch(&config, &cache, &Keys::from_env(), false, cli.dry_run, None)
                    .await?;
            let analysis =
                commands::analyze(&config, &cache, &cli.out, *remeasure, cli.dry_run, None)?;
            let manifest = commands::build(
                &config,
                &cache,
                &analysis,
                &cli.out,
                *force,
                cli.dry_run,
                km_wallpaper_pack::manifest::generated_now(),
            )?;
            copy_zip(cli, &config);
            report(
                cli,
                "all",
                &[
                    ("downloaded", downloaded.to_string()),
                    ("chosen", analysis.chosen.len().to_string()),
                    ("images", manifest.images.len().to_string()),
                    ("zip_dest", cli.zip_dest.display().to_string()),
                ],
            );
        }
        Command::Local {
            dir,
            credits,
            force,
        } => {
            let manifest = commands::local(
                dir,
                credits.as_deref(),
                &cli.out,
                *force,
                cli.dry_run,
                km_wallpaper_pack::manifest::generated_now(),
            )?;
            // The same copy every other build gets, and the same default destination — which for
            // this command is usually the wrong one on purpose. A curated set is normally destined
            // for `assets/wallpapers`, and that is a decision somebody types rather than a default
            // that quietly puts photographs into every carrier.
            copy_zip(cli, &Config::default());
            report(
                cli,
                "local",
                &[
                    ("images", manifest.images.len().to_string()),
                    ("out", cli.out.display().to_string()),
                    ("zip_dest", cli.zip_dest.display().to_string()),
                ],
            );
        }
        Command::Verify { pack } => {
            let checked = commands::verify(pack)?;
            report(
                cli,
                "verify",
                &[("images", checked.to_string()), ("ok", "true".to_owned())],
            );
        }
    }
    Ok(())
}

/// Put the finished zip in the app's wallpaper folder, and say where it went.
///
/// Best effort by design, which is why the failure is reported here rather than returned. The pack is
/// already written and correct at this point, so a destination that cannot be created — a read-only
/// checkout, a `--zip-dest` on a disk that is not there — is worth a line on stderr and is not worth
/// failing a fifteen-minute build over. `--out` still holds the zip either way; this copy is a
/// convenience, not the deliverable.
///
/// Nothing to copy when the config asked for no zip, and nothing to copy under `--dry-run`, which
/// promises to write nothing anywhere.
fn copy_zip(cli: &Cli, config: &Config) {
    if !config.output.zip || cli.dry_run {
        return;
    }
    let name = km_wallpaper_pack::manifest::zip_name(&config.hash());
    match commands::copy_zip(&cli.out, &cli.zip_dest, &name) {
        Ok(Some(to)) => println!("copied {}", to.display()),
        Ok(None) => {}
        Err(error) => eprintln!(
            "could not copy {name} to {}: {error}",
            cli.zip_dest.display()
        ),
    }
}

fn prepare(cli: &Cli, config: &std::path::Path) -> Result<(Config, Cache)> {
    let config = Config::load(config)?;
    let cache = Cache::open(&cli.cache_dir)?;
    Ok((config, cache))
}

fn read_analysis(cli: &Cli) -> Result<Analysis> {
    let path = cli.out.join("analysis.json");
    let text = std::fs::read_to_string(&path).map_err(|source| km_wallpaper_pack::Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|error| {
        km_wallpaper_pack::Error::PackMismatch(format!("{}: {error}", path.display()))
    })
}

/// A line of key=value on stdout, or a JSON object under `--json`.
fn report(cli: &Cli, command: &str, fields: &[(&str, String)]) {
    if cli.json {
        let object: serde_json::Map<String, serde_json::Value> = fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), serde_json::Value::String(value.clone())))
            .chain(std::iter::once((
                "command".to_owned(),
                serde_json::Value::String(command.to_owned()),
            )))
            .collect();
        println!("{}", serde_json::Value::Object(object));
        return;
    }
    let rendered: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("{key} {value}"))
        .collect();
    println!("{command}: {}", rendered.join(", "));
}

fn summarize_analysis(cli: &Cli, analysis: &Analysis) {
    if cli.json {
        report(
            cli,
            "analyze",
            &[
                ("chosen", analysis.chosen.len().to_string()),
                ("rejected", analysis.rejected.len().to_string()),
            ],
        );
        return;
    }

    println!("analyze: chosen {}", analysis.chosen.len());
    let mut histogram: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for entry in &analysis.rejected {
        *histogram
            .entry(entry.reason.split(':').next().unwrap_or("other"))
            .or_insert(0) += 1;
    }
    // The histogram is the whole point of the phase: it is what a threshold is tuned against.
    for (reason, count) in histogram {
        println!("  {count:>6}  {reason}");
    }
    // **`--dry-run` promises to write nothing, and this line used to claim otherwise.** `analyze`
    // itself is guarded correctly — the file is not written — so the only thing wrong was the
    // sentence, which is the worst place for it to be wrong: a dry run is a thing somebody does
    // *because* they want to know what would happen, and it was answering with a file that was not
    // there. Found by running one to check a config after a move, and looking at the timestamp of
    // the file it named.
    if cli.dry_run {
        println!(
            "\nwrote nothing: --dry-run. {} is untouched.",
            cli.out.join("analysis.json").display()
        );
    } else {
        println!("\nwrote {}", cli.out.join("analysis.json").display());
    }
}

// The calendar tests went to `manifest.rs` with the two functions they cover.
