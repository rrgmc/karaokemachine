//! A pack built end to end, from a cache of synthetic photographs.
//!
//! This is the test that stands in for the CI job the plan asked for: a miniature pack is built and
//! then handed to `verify`, so a threshold change or a hand-added image cannot silently ship a
//! background nobody can read the lyrics over. It needs no network and no committed binaries — the
//! "photographs" are generated, which also makes the determinism check exact.

use std::path::Path;

use image::{Rgb, RgbImage};
use km_wallpaper_pack::cache::Cache;
use km_wallpaper_pack::commands;
use km_wallpaper_pack::config::{Config, ProviderKind};
use km_wallpaper_pack::manifest::Manifest;
use km_wallpaper_pack::providers::RemoteImage;

/// A config with one query group and a small pack.
const CONFIG: &str = r#"
[output]
sizes = ["320x180"]
target_count = 10
zip = true

[filters]
min_source_width = 100
# Dialled down for the same reason as the line above: the fixtures are 400x300, and the shipped
# default is a floor for real downloads rather than for generated ones.
min_decoded_width = 100
min_entropy = 0.5

[[queries]]
provider = "pixabay"
terms = ["mountain lake dusk"]
pages = 1
"#;

/// A photograph-ish image: a dark vertical gradient with some texture, so it has entropy without
/// being busy where the lyrics go.
fn photograph(width: u32, height: u32, brightness: u8) -> Vec<u8> {
    let mut image = RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let sky = (y as f32 / height as f32 * f32::from(brightness)) as u8;
        let texture = ((x / 16 + y / 16) % 3) as u8 * 4;
        *pixel = Rgb([
            sky.saturating_add(texture),
            sky.saturating_add(texture / 2),
            sky.saturating_add(texture).saturating_add(8),
        ]);
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode the fixture");
    bytes
}

/// A cache holding `count` distinct photographs, plus one that is far too bright to pass.
fn seeded_cache(root: &Path, count: usize) -> Cache {
    let cache = Cache::open(root).expect("cache");
    let mut index = Vec::new();
    for n in 0..count {
        let id = format!("{}", 1000 + n);
        // Distinct brightnesses so the perceptual hashes differ and nothing is deduplicated away.
        cache
            .put_original(
                ProviderKind::Pixabay,
                &id,
                "png",
                &photograph(400, 300, 60 + (n as u8) * 9),
            )
            .expect("store");
        index.push(RemoteImage {
            provider: ProviderKind::Pixabay,
            id: id.clone(),
            download_url: format!("https://example.test/{id}.png"),
            page_url: format!("https://pixabay.com/photos/{id}/"),
            author: format!("Photographer {n}"),
            author_url: Some(format!("https://pixabay.com/users/photographer{n}/")),
            width: 400,
            height: 300,
            tags: vec!["mountain".to_owned()],
            query: "mountain lake dusk".to_owned(),
            license: None,
            title: None,
            attribution: None,
        });
    }

    // A white frame: nothing within a 45% scrim makes white lyrics readable over this, and the pack
    // has to say so rather than ship it.
    let mut white = RgbImage::new(400, 300);
    for (x, y, pixel) in white.enumerate_pixels_mut() {
        let noise = ((x / 8 + y / 8) % 2) as u8 * 6;
        *pixel = Rgb([250 - noise, 250 - noise, 252 - noise]);
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(white)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode");
    cache
        .put_original(ProviderKind::Pixabay, "9999", "png", &bytes)
        .expect("store");
    index.push(RemoteImage {
        provider: ProviderKind::Pixabay,
        id: "9999".to_owned(),
        download_url: "https://example.test/9999.png".to_owned(),
        page_url: "https://pixabay.com/photos/9999/".to_owned(),
        author: "Too Bright".to_owned(),
        author_url: None,
        width: 400,
        height: 300,
        tags: Vec::new(),
        query: "mountain lake dusk".to_owned(),
        license: None,
        title: None,
        attribution: None,
    });

    cache.extend_index(&index).expect("index");
    cache
}

#[test]
fn a_pack_is_built_from_the_cache_and_passes_its_own_gate() {
    let dir = tempfile::tempdir().expect("temp");
    let config = Config::parse(CONFIG).expect("config");
    let cache = seeded_cache(&dir.path().join("cache"), 4);
    let out = dir.path().join("out");

    let analysis = commands::analyze(&config, &cache, &out, false, false, None).expect("analyze");
    assert!(!analysis.chosen.is_empty(), "the dark frames are usable");
    assert!(
        analysis
            .rejected
            .iter()
            .any(|entry| entry.key.ends_with(":9999") && entry.reason == "contrast_unreachable"),
        "the white frame is refused, by name: {:?}",
        analysis.rejected
    );

    let manifest = commands::build(
        &config,
        &cache,
        &analysis,
        &out,
        false,
        false,
        "2026-08-24T00:00:00Z".to_owned(),
    )
    .expect("build");
    assert_eq!(manifest.images.len(), analysis.chosen.len());

    // Every promise the manifest makes about an image is a promise about a file that exists.
    for entry in &manifest.images {
        let path = out.join(&entry.file);
        assert!(path.is_file(), "{} is missing", entry.file);
        assert!(
            entry.measured_contrast >= config.legibility.target_contrast,
            "{} claims {}",
            entry.file,
            entry.measured_contrast
        );
        assert!(!entry.author.is_empty(), "attribution is not optional");
    }

    // And `verify` agrees, which is the property the whole gate rests on: the measurement `build`
    // made and the measurement a later check makes have to be the same measurement.
    let checked = commands::verify(&out).expect("verify");
    assert_eq!(checked, manifest.images.len());

    // The deliverable: one zip to drop into the app's wallpaper folder.
    let zip = std::fs::read_dir(&out)
        .expect("read out")
        .flatten()
        .find(|entry| entry.path().extension().is_some_and(|e| e == "zip"))
        .expect("a zip was written");
    let archive =
        zip::ZipArchive::new(std::fs::File::open(zip.path()).expect("open")).expect("zip");
    let names: Vec<_> = archive.file_names().collect();
    assert!(names.contains(&"manifest.json"), "{names:?}");
    assert!(names.contains(&"ATTRIBUTION.md"), "{names:?}");
    assert!(
        names.iter().any(|name| name.ends_with(".jpg")),
        "and the pictures: {names:?}"
    );
    assert!(
        !names.contains(&"analysis.json"),
        "the working file that produced the pack is not part of it: {names:?}"
    );
}

#[test]
fn verify_fails_with_its_own_exit_code_when_an_image_is_replaced() {
    let dir = tempfile::tempdir().expect("temp");
    let config = Config::parse(CONFIG).expect("config");
    let cache = seeded_cache(&dir.path().join("cache"), 2);
    let out = dir.path().join("out");
    let analysis = commands::analyze(&config, &cache, &out, false, false, None).expect("analyze");
    let manifest = commands::build(
        &config,
        &cache,
        &analysis,
        &out,
        false,
        false,
        "2026-08-24T00:00:00Z".to_owned(),
    )
    .expect("build");

    // Somebody drops a bright picture into the pack under a name the manifest already promises.
    let file = out.join(&manifest.images[0].file);
    let white = RgbImage::from_pixel(320, 180, Rgb([255, 255, 255]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(white)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .expect("encode");
    std::fs::write(&file, &bytes).expect("write");

    let error = commands::verify(&out).expect_err("must fail");
    assert_eq!(
        error.exit_code(),
        2,
        "a contrast failure has its own code: {error}"
    );

    // And a *missing* file is a different failure, because it means something else is wrong.
    std::fs::remove_file(&file).expect("remove");
    let error = commands::verify(&out).expect_err("must fail");
    assert_eq!(error.exit_code(), 3, "{error}");
}

#[test]
fn two_runs_over_the_same_cache_produce_the_same_pack() {
    let dir = tempfile::tempdir().expect("temp");
    let config = Config::parse(CONFIG).expect("config");
    let cache = seeded_cache(&dir.path().join("cache"), 3);

    let build_into = |out: &Path| -> Manifest {
        let analysis =
            commands::analyze(&config, &cache, out, false, false, None).expect("analyze");
        commands::build(
            &config,
            &cache,
            &analysis,
            out,
            false,
            false,
            "stamped later".to_owned(),
        )
        .expect("build")
    };

    let first_dir = dir.path().join("first");
    let second_dir = dir.path().join("second");
    let first = build_into(&first_dir);
    let second = build_into(&second_dir);

    assert_eq!(
        first.images, second.images,
        "same cache, same config, same pack"
    );
    for entry in &first.images {
        assert_eq!(
            std::fs::read(first_dir.join(&entry.file)).expect("read"),
            std::fs::read(second_dir.join(&entry.file)).expect("read"),
            "{} differs between runs",
            entry.file
        );
    }

    // The two runs shared a cache, so the second one measured nothing: it read back what the first
    // wrote. That makes this test the measurement cache's correctness proof as well as the pack's --
    // a memo that returned even slightly different numbers would show up here as a different pack.
    let jsonl = std::fs::read_to_string(cache.root().join("metrics.jsonl")).expect("metrics.jsonl");
    assert_eq!(
        jsonl.lines().count(),
        4,
        "one record per image -- the three photographs and the white frame -- written by the first \
         run and not again by the second"
    );
}

#[test]
fn remeasuring_reaches_the_same_answer_as_the_cache_it_ignores() {
    // The other direction of the test above. There, a warm cache had to agree with a cold one; here,
    // a run told to ignore a warm cache has to agree with having used it. Both matter: the first says
    // the memo is not lossy, the second says `--remeasure` is a genuine escape hatch and not a
    // different code path that quietly produces a different pack.
    let dir = tempfile::tempdir().expect("temp");
    let config = Config::parse(CONFIG).expect("config");
    let cache = seeded_cache(&dir.path().join("cache"), 3);

    let warm = commands::analyze(&config, &cache, &dir.path().join("a"), false, false, None)
        .expect("analyze, populating the cache");
    let cold = commands::analyze(&config, &cache, &dir.path().join("b"), true, false, None)
        .expect("analyze, ignoring the cache");

    let keys = |analysis: &commands::Analysis| -> Vec<String> {
        analysis.chosen.iter().map(|c| c.key.clone()).collect()
    };
    assert_eq!(keys(&warm), keys(&cold), "same pack, memo or no memo");
    for (warm, cold) in warm.chosen.iter().zip(&cold.chosen) {
        assert_eq!(warm.phash, cold.phash, "{}: perceptual hash", warm.key);
        assert_eq!(warm.score, cold.score, "{}: score", warm.key);
        assert_eq!(
            warm.measured_contrast, cold.measured_contrast,
            "{}: contrast",
            warm.key
        );
    }
}

#[test]
fn a_measurement_taken_under_other_settings_is_not_reused() {
    // The cache key is `Config::measurement_hash`, not `Config::hash`, and this pins both halves of
    // that choice: a legibility change must invalidate a measurement, and a change to the queries --
    // which cannot affect how any one photograph measures -- must not.
    let dir = tempfile::tempdir().expect("temp");
    let cache = seeded_cache(&dir.path().join("cache"), 2);
    let out = dir.path().join("out");

    // Three records for two photographs: `seeded_cache` adds a white frame that no scrim can rescue,
    // and it is measured like any other. Rejection happens after measurement, so a hopeless image is
    // measured exactly once ever rather than on every run — which is most of the point.
    let records = || {
        std::fs::read_to_string(cache.root().join("metrics.jsonl"))
            .expect("read")
            .lines()
            .count()
    };

    let config = Config::parse(CONFIG).expect("config");
    commands::analyze(&config, &cache, &out, false, false, None).expect("analyze");
    assert_eq!(records(), 3);

    // Another search term: the same photographs, measured the same way, so nothing is re-measured.
    let more_queries = Config::parse(&CONFIG.replace(
        r#"terms = ["mountain lake dusk"]"#,
        r#"terms = ["mountain lake dusk", "alpine valley"]"#,
    ))
    .expect("config");
    assert_ne!(more_queries.hash(), config.hash(), "a different pack");
    assert_eq!(
        more_queries.measurement_hash(),
        config.measurement_hash(),
        "but not a different measurement"
    );
    commands::analyze(&more_queries, &cache, &out, false, false, None).expect("analyze");
    assert_eq!(
        records(),
        3,
        "a new search term must not re-measure anything"
    );

    // A different scrim: every contrast answer changes, so every measurement has to be taken again.
    let dimmer = Config::parse(&format!("{CONFIG}\n[legibility]\nassumed_dim = 0.6\n"))
        .expect("config with a deeper scrim");
    assert_ne!(
        dimmer.measurement_hash(),
        config.measurement_hash(),
        "legibility is part of a measurement"
    );
    commands::analyze(&dimmer, &cache, &out, false, false, None).expect("analyze");
    assert_eq!(
        records(),
        6,
        "three more records, under the new key, beside the three already there"
    );
}

#[test]
fn a_second_size_is_judged_on_its_own_merits() {
    // A 4K crop of a photograph is a different picture from a 1080p crop of it — different pixels
    // under the lyrics — so the gate runs per size rather than once per photograph.
    let dir = tempfile::tempdir().expect("temp");
    let config = Config::parse(&CONFIG.replace(
        "sizes = [\"320x180\"]",
        "sizes = [\"320x180\", \"640x360\"]",
    ))
    .expect("config");
    let cache = seeded_cache(&dir.path().join("cache"), 2);
    let out = dir.path().join("out");

    let analysis = commands::analyze(&config, &cache, &out, false, false, None).expect("analyze");
    let manifest = commands::build(
        &config,
        &cache,
        &analysis,
        &out,
        false,
        false,
        "2026-08-24T00:00:00Z".to_owned(),
    )
    .expect("build");

    let small = manifest
        .images
        .iter()
        .filter(|entry| entry.file.starts_with("320x180/"))
        .count();
    let large = manifest
        .images
        .iter()
        .filter(|entry| entry.file.starts_with("640x360/"))
        .count();
    assert!(small > 0 && large > 0, "both sizes were written");
    assert!(commands::verify(&out).is_ok(), "and both pass the gate");
}

#[test]
fn building_into_a_used_directory_needs_force() {
    let dir = tempfile::tempdir().expect("temp");
    let config = Config::parse(CONFIG).expect("config");
    let cache = seeded_cache(&dir.path().join("cache"), 2);
    let out = dir.path().join("out");
    let analysis = commands::analyze(&config, &cache, &out, false, false, None).expect("analyze");

    let build = |force| {
        commands::build(
            &config,
            &cache,
            &analysis,
            &out,
            force,
            false,
            "2026-08-24T00:00:00Z".to_owned(),
        )
    };
    build(false).expect("the first build");
    let error = build(false).expect_err("the second must refuse");
    assert!(error.to_string().contains("--force"), "{error}");

    // What a rebuild has to get rid of: an image from an earlier selection, which output names
    // guarantee will not be overwritten, and an earlier zip under an earlier count.
    let stale = out.join("320x180/scenery-099-deadbeef-320x180.jpg");
    std::fs::write(&stale, b"an image from a build before this one").expect("write");
    let old_zip = out.join("wallpapers-00000000.zip");
    std::fs::write(&old_zip, b"a pack from a build before this one").expect("write");

    let manifest = build(true).expect("and force replaces");
    assert!(
        !stale.exists(),
        "the stale image is gone from the directory"
    );
    assert!(!old_zip.exists(), "and so is the pack it was in");
    assert!(
        out.join("analysis.json").is_file(),
        "the analysis is what build was given; it is not build's to delete"
    );

    let zips: Vec<_> = std::fs::read_dir(&out)
        .expect("read out")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "zip"))
        .collect();
    assert_eq!(zips.len(), 1, "one deliverable, not one per run: {zips:?}");

    // And the deliverable holds exactly what the manifest promises: no stale image, no nested zip,
    // and not the analysis either.
    let archive = zip::ZipArchive::new(std::fs::File::open(&zips[0]).expect("open")).expect("zip");
    let mut names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    names.sort();
    let mut expected: Vec<String> = manifest
        .images
        .iter()
        .map(|entry| entry.file.clone())
        .chain(["ATTRIBUTION.md".to_owned(), "manifest.json".to_owned()])
        .collect();
    expected.sort();
    assert_eq!(names, expected);

    assert!(
        commands::verify(&out).is_ok(),
        "and the pack still checks out"
    );
}

#[test]
fn a_dry_run_never_removes_a_pack() {
    let dir = tempfile::tempdir().expect("temp");
    let config = Config::parse(CONFIG).expect("config");
    let cache = seeded_cache(&dir.path().join("cache"), 2);
    let out = dir.path().join("out");
    let analysis = commands::analyze(&config, &cache, &out, false, false, None).expect("analyze");

    let build = |force, dry_run| {
        commands::build(
            &config,
            &cache,
            &analysis,
            &out,
            force,
            dry_run,
            "2026-08-24T00:00:00Z".to_owned(),
        )
    };
    let manifest = build(false, false).expect("the first build");
    build(true, true).expect("a dry run says what would happen");

    for entry in &manifest.images {
        assert!(
            out.join(&entry.file).is_file(),
            "{} was removed by a run that writes nothing",
            entry.file
        );
    }
    assert!(commands::verify(&out).is_ok(), "the pack is untouched");
}
