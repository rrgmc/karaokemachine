//! Driving `km-wallpaper-pack`'s three phases from a page.
//!
//! **This is a caller, not a second pipeline.** `fetch`, `analyze` and `build` are the tool's own,
//! doing exactly what they do from a command line; what is here is the plumbing a browser needs and
//! a terminal does not — a progress sink, a blocking phase moved off the runtime, and a thumbnail.
//!
//! **`analyze` and `build` go on `spawn_blocking`.** Both are `rayon`-parallel and CPU-bound for
//! their whole duration; running either on a tokio worker would occupy it for minutes, and the page
//! that is polling for progress would stop being answered — the request that reports "measuring, 40%"
//! is competing for the same threads as the measuring.

use std::path::{Path, PathBuf};

use km_wallpaper_pack::cache::Cache;
use km_wallpaper_pack::commands;
use km_wallpaper_pack::config::ProviderKind;
use km_wallpaper_pack::providers::Keys;

use crate::job::Job;
use crate::pictures::Settings;

/// How wide a review thumbnail is.
///
/// Big enough to judge whether a photograph is calm behind two lines of text, small enough that a
/// hundred and twenty of them are a page rather than a download.
const THUMB_WIDTH: u32 = 320;

/// Search, measure, select and build. Answers with the pack that was written.
///
/// **The job arrives as an `Arc` rather than a reference**, because two of the three phases are
/// moved onto a blocking thread and a borrow cannot go with them. Reaching for `unsafe` to send a
/// pointer instead is the obvious wrong turn here, and this workspace denies `unsafe` outright.
pub async fn pictures(
    settings: Settings,
    keys: Keys,
    data_dir: &Path,
    job: std::sync::Arc<Job>,
) -> Result<PathBuf, String> {
    let config = settings.to_config();
    let cache_dir = crate::pictures::cache_dir(data_dir);
    let out = crate::pictures::build_dir(data_dir);
    let cache = Cache::open(&cache_dir).map_err(|error| error.to_string())?;

    // The sink the tool ticks into. It has to be `Sync` because `analyze` ticks from every `rayon`
    // worker at once — see `km_wallpaper_pack::progress`.
    let sink = |tick: km_wallpaper_pack::progress::Tick| {
        job.progress(
            crate::job::phase_key(tick.phase),
            tick.done as u64,
            tick.total as u64,
        );
    };

    commands::fetch(&config, &cache, &keys, false, false, Some(&sink))
        .await
        .map_err(|error| error.to_string())?;
    if job.stopping() {
        return Err("stopped".to_owned());
    }

    // **Off the runtime for the whole of the next two.** Both are `rayon`-parallel and CPU-bound,
    // and a tokio worker held for minutes is a page that stops answering its own progress poll.
    let analyzed = {
        let config = config.clone();
        let cache = cache.clone();
        let out = out.clone();
        let job = std::sync::Arc::clone(&job);
        tokio::task::spawn_blocking(move || {
            let sink = |tick: km_wallpaper_pack::progress::Tick| {
                job.progress(
                    crate::job::phase_key(tick.phase),
                    tick.done as u64,
                    tick.total as u64,
                );
            };
            commands::analyze(&config, &cache, &out, false, false, Some(&sink))
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("measuring died: {error}"))??
    };

    if analyzed.chosen.is_empty() {
        return Err(format!(
            "nothing came through the gate: {} pictures were looked at and every one was rejected. \
             Try other search terms, or a lower contrast.",
            analyzed.rejected.len()
        ));
    }
    if job.stopping() {
        return Err("stopped".to_owned());
    }

    job.progress(crate::job::phase::BUILDING, 0, analyzed.chosen.len() as u64);
    let _manifest = {
        let config = config.clone();
        let cache = cache.clone();
        let out = out.clone();
        tokio::task::spawn_blocking(move || {
            // `force`, because this output directory is this program's own and holds the last pack
            // it built. A refusal to overwrite is right for a packager's `--out`; here it would mean
            // a second search could never finish.
            commands::build(
                &config,
                &cache,
                &analyzed,
                &out,
                true,
                false,
                km_wallpaper_pack::manifest::generated_now(),
            )
            .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("building died: {error}"))??
    };

    // The zip is the deliverable: `km-display` reads one in the wallpaper folder as a folder of
    // images, and `Controller::accept_upload` moves it in whole rather than unpacking it.
    let built = km_wallpaper_pack::manifest::zip_name(&config.hash());
    let pack = out.join(&built);
    if !pack.is_file() {
        return Err(format!("{built} was not written"));
    }
    keep(
        data_dir,
        &pack,
        &kept_name(&built, settings.name.as_deref()),
    )
}

/// What a built pack is called once it is kept.
///
/// **The name is applied here and not in `km-wallpaper-pack`**, and which side of that line it falls
/// on is the whole decision. The pipeline writes into a scratch directory it clears on every run, and
/// it derives that file's name from `Config::hash` — so putting the name into the config would make a
/// rename change the hash, which invalidates `analysis.json` and asks somebody to re-measure four
/// thousand photographs because they changed a word. Keeping is where a pack stops being scratch and
/// acquires a lasting identity, so keeping is where it acquires a name.
///
/// The prefix stays first. Nothing depends on it here — `pack_artifacts`' prefix test only ever
/// sees the build directory's unrenamed copy — but every pack this program writes begins that way,
/// and one that did not would read as something else that happened to be in the folder.
///
/// The slug has already been through [`crate::pictures::slug`], so it is a legal single path
/// component and [`crate::pictures::pack_dir`] will accept the folder id derived from it.
fn kept_name(built: &str, slug: Option<&str>) -> String {
    let Some(slug) = slug.filter(|slug| !slug.is_empty()) else {
        return built.to_owned();
    };
    match built.strip_prefix(km_wallpaper_pack::manifest::ZIP_PREFIX) {
        Some(rest) => format!("{}{slug}-{rest}", km_wallpaper_pack::manifest::ZIP_PREFIX),
        // Unreachable while `zip_name` builds the name it does, and not worth an `expect`: a pack
        // named after the search is better than a failed build.
        None => format!("{slug}-{built}"),
    }
}

/// Moves a finished pack out of the working directory into a folder of its own.
///
/// **The move is what makes a second search safe.** `build` runs with `force` and clears the
/// directory it writes into, so a pack left where it was built survives only until the next one —
/// which is exactly what it must not do now that a pack is something you install on more than one
/// machine.
///
/// **Three files, and not the loose images.** The zip is the deliverable and already contains them;
/// keeping `1920x1080/` beside it would double what a pack costs on disk for a copy nothing here
/// reads. `ATTRIBUTION.md` travels because it is a license obligation rather than a convenience, and
/// `manifest.json` because it is what a row in the list is drawn from.
fn keep(data_dir: &Path, pack: &Path, name: &str) -> Result<PathBuf, String> {
    let id = name.strip_suffix(".zip").unwrap_or(name);
    let dest = crate::pictures::pack_dir(data_dir, id)
        .ok_or_else(|| format!("{name} is not a name this program can keep a pack under"))?;

    // A rebuild of the same search produces the same name, and half of the old one left beside the
    // new one would be a pack that describes pictures it does not contain.
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .map_err(|error| format!("{} could not be replaced: {error}", dest.display()))?;
    }
    std::fs::create_dir_all(&dest)
        .map_err(|error| format!("{} cannot be written to: {error}", dest.display()))?;

    let kept = dest.join(name);
    std::fs::rename(pack, &kept)
        .map_err(|error| format!("{name} could not be moved into its own folder: {error}"))?;

    // The sidecars are worth having and are not worth failing over: a pack whose attribution did not
    // move is still a pack, and the zip has already been paid for.
    for sidecar in [
        km_wallpaper_pack::manifest::MANIFEST_FILE,
        km_wallpaper_pack::manifest::ATTRIBUTION_FILE,
    ] {
        let from = pack.with_file_name(sidecar);
        if from.is_file()
            && let Err(error) = std::fs::rename(&from, dest.join(sidecar))
        {
            tracing::warn!(%error, sidecar, "the pack was kept without one of its sidecars");
        }
    }

    Ok(kept)
}

/// A small JPEG of one candidate, made once and kept.
pub fn thumbnail(cache: &Cache, provider: ProviderKind, id: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = cache.thumb(provider, id) {
        return std::fs::read(&path).map_err(|error| error.to_string());
    }

    let original = cache
        .original(provider, id)
        .ok_or_else(|| format!("no picture cached for {provider}:{id}"))?;
    let image = image::open(&original).map_err(|error| format!("could not read it: {error}"))?;
    let small = image.thumbnail(THUMB_WIDTH, THUMB_WIDTH * 2);

    let mut bytes = Vec::new();
    small
        .to_rgb8()
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .map_err(|error| format!("could not encode a thumbnail: {error}"))?;
    // Written back so the next hundred and nineteen tiles are a file read.
    let _ = cache.put_thumb(provider, id, &bytes);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lays out a working directory as `analyze` and `build` leave one.
    fn built(home: &Path, name: &str) -> PathBuf {
        let build = crate::pictures::build_dir(home);
        std::fs::create_dir_all(build.join("1920x1080")).expect("the working folder");
        std::fs::write(build.join(name), b"PK").expect("the zip");
        std::fs::write(build.join("1920x1080/a.jpg"), b"jpeg").expect("an image");
        std::fs::write(
            build.join(km_wallpaper_pack::manifest::MANIFEST_FILE),
            b"{}",
        )
        .expect("the manifest");
        std::fs::write(
            build.join(km_wallpaper_pack::manifest::ATTRIBUTION_FILE),
            b"# credits",
        )
        .expect("the attribution");
        std::fs::write(build.join("analysis.json"), b"[]").expect("the review");
        build.join(name)
    }

    #[test]
    fn a_finished_pack_leaves_the_directory_the_next_build_clears() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let name = "wallpapers-e36b9929.zip";
        let pack = built(home.path(), name);

        let kept = keep(home.path(), &pack, name).expect("it is kept");

        assert_eq!(
            kept,
            crate::pictures::packs_dir(home.path())
                .join("wallpapers-e36b9929")
                .join(name)
        );
        assert!(kept.is_file());
        assert!(!pack.exists(), "it moved rather than being copied");

        let folder = kept.parent().expect("a folder of its own");
        assert!(
            folder
                .join(km_wallpaper_pack::manifest::MANIFEST_FILE)
                .is_file()
        );
        assert!(
            folder
                .join(km_wallpaper_pack::manifest::ATTRIBUTION_FILE)
                .is_file(),
            "the license file goes with the pack it credits"
        );
        // The loose images are already inside the zip; a second copy would double what a pack costs
        // on disk for something nothing here reads.
        assert!(!folder.join("1920x1080").exists());
        // And the review stays where the page reads it from.
        assert!(
            crate::pictures::build_dir(home.path())
                .join("analysis.json")
                .is_file()
        );
    }

    #[test]
    fn rebuilding_the_same_search_replaces_that_pack_rather_than_mixing_with_it() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let name = "wallpapers-e36b9929.zip";

        let first = built(home.path(), name);
        let kept = keep(home.path(), &first, name).expect("it is kept");
        // Something only the first build wrote. A pack that kept it would describe pictures it does
        // not contain.
        std::fs::write(kept.with_file_name("stale.json"), b"{}").expect("a leftover");

        let second = built(home.path(), name);
        let kept = keep(home.path(), &second, name).expect("it is kept again");

        assert!(kept.is_file());
        assert!(!kept.with_file_name("stale.json").exists());
    }

    #[test]
    fn a_named_pack_carries_its_name_after_the_prefix() {
        assert_eq!(
            kept_name("wallpapers-e36b9929.zip", Some("beaches")),
            "wallpapers-beaches-e36b9929.zip",
            "the prefix stays first, so every pack this program writes still begins alike"
        );
    }

    #[test]
    fn an_unnamed_pack_is_called_what_every_pack_used_to_be_called() {
        let plain = "wallpapers-e36b9929.zip";
        assert_eq!(kept_name(plain, None), plain);
        // A slug that came back empty is the same as none. `pictures::slug` answers `None` for that,
        // but a stored settings file could hold `Some("")` from a build that did not.
        assert_eq!(kept_name(plain, Some("")), plain);
    }

    /// The name has to survive being a folder, and `keep` is what turns it into one.
    #[test]
    fn a_named_pack_gets_a_folder_named_after_it() {
        let home = tempfile::tempdir().expect("a temporary folder");
        let built_as = "wallpapers-e36b9929.zip";
        let pack = built(home.path(), built_as);
        let name = kept_name(built_as, Some("praias-do-sul"));

        let kept = keep(home.path(), &pack, &name).expect("it is kept");

        assert_eq!(
            kept,
            crate::pictures::packs_dir(home.path())
                .join("wallpapers-praias-do-sul-e36b9929")
                .join("wallpapers-praias-do-sul-e36b9929.zip")
        );
        assert!(kept.is_file());
    }
}
