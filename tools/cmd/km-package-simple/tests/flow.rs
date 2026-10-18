//! The whole flow through the router: read a folder, rename and leave out, build, open the file.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode};
use km_package_simple::app::{App, Phase};
use km_testkit::{Scratch, http};

/// A form post from this tool's own page, which carries `sec-fetch-site: same-origin`.
async fn post(app: &Arc<App>, uri: &str, body: &str) -> (StatusCode, String) {
    let mut request = http::form(uri, body.to_owned());
    request
        .headers_mut()
        .insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    let answer = http::send(km_package_simple::server::router(Arc::clone(app)), request).await;
    (answer.status, answer.text())
}

async fn get(app: &Arc<App>, uri: &str) -> String {
    let router = km_package_simple::server::router(Arc::clone(app));
    http::send(router, http::get(uri)).await.text()
}

/// Waits until the job running in the background has ended.
async fn settled(app: &Arc<App>) {
    for _ in 0..600 {
        if !matches!(
            app.lock().phase,
            Phase::Reading { .. } | Phase::Building { .. }
        ) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the job did not end");
}

fn form_value(path: &Path) -> String {
    serde_urlencoded::to_string([("x", path.display().to_string())])
        .expect("encode")
        .trim_start_matches("x=")
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_becomes_an_uncurated_package_with_the_changes_made_on_the_page() {
    let songs = Scratch::new("flow-songs");
    std::fs::write(songs.join("a.kar"), km_song::testing::soft_karaoke()).expect("write");
    std::fs::write(songs.join("b.mid"), km_song::testing::lyric_events()).expect("write");
    std::fs::write(songs.join("copy.kar"), km_song::testing::soft_karaoke()).expect("write");
    let out = Scratch::new("flow-out");

    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let (status, _) = post(&app, "/folder", &format!("path={}", form_value(&songs))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;
    assert!(
        matches!(app.lock().phase, Phase::Ready),
        "{:?}",
        app.lock().error
    );

    let page = get(&app, "/").await;
    assert!(page.contains("a.kar"), "{page}");
    assert!(
        page.contains("copy.kar"),
        "the copy is listed as not a song: {page}"
    );
    // A MIDI song has a suitability, so it is coloured by its band and never drawn as `none`.
    assert!(
        ["low", "mid", "high"]
            .iter()
            .any(|band| page.contains(&format!(r#"<span class="suitability {band}">"#))),
        "{page}"
    );
    assert!(!page.contains(r#"class="suitability none""#), "{page}");
    assert_eq!(
        page.matches(r#"<nav class="pager">"#).count(),
        2,
        "the pager is above the list and below it: {page}"
    );
    assert!(
        page.contains(r##"hx-target="#browse-out_dir" hx-swap="innerHTML""##),
        "the output folder offers a picker of its own: {page}"
    );

    // Rename the first song and leave the second out.
    let (status, _) = post(&app, "/songs/0", "title=Renamed&artist=Somebody").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, list) = post(&app, "/songs/keep?page=0", "from=1&to=1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains("left-out"), "{list}");

    // The button that leaves out the low songs answers with the list, and changes no song that
    // is 8 or more.
    let before: Vec<bool> = app
        .lock()
        .session
        .as_ref()
        .expect("a session")
        .rows
        .iter()
        .map(|row| row.kept)
        .collect();
    let (status, list) = post(&app, "/songs/leave-out-low?page=0", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains(r#"id="songs""#), "{list}");
    assert!(
        !list.contains("/songs/leave-out-low"),
        "no kept song is low any more: {list}"
    );
    let session = app.lock().session.clone().expect("a session");
    for (row, was) in session.rows.iter().zip(before) {
        assert_eq!(row.kept, was && !row.is_low(), "{}", row.song.file);
    }
    // Put the first song back, so the build below sees only the song left out by hand.
    let (status, _) = post(&app, "/songs/keep?page=0", "from=0&to=0&keep=on").await;
    assert_eq!(status, StatusCode::OK);

    let body = format!(
        "name=Party&version=1.0.0&publisher=&language=und&out_dir={}",
        form_value(&out)
    );
    let (status, _) = post(&app, "/build", &body).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;
    let built = match &app.lock().phase {
        Phase::Built(built) => built.clone(),
        other => panic!("not built: {other:?} {:?}", app.lock().error),
    };
    assert_eq!(built.files.len(), 1);

    let file = out.join("party-1.0.0.kmpkg");
    let package = km_kmpkg::Package::open(&file).expect("open the package");
    assert!(
        package.is_uncurated(),
        "a package from a folder is uncurated"
    );
    assert_eq!(package.len(), 1, "the song left out stayed out");
    let song = &package.manifest().songs[0];
    assert_eq!(song.number, 1);
    assert_eq!(song.title, "Renamed");
    assert_eq!(song.artist.as_deref(), Some("Somebody"));
    assert!(
        out.join("party-1.0.0.kmpkg.txt").is_file(),
        "a listing beside it"
    );

    let page = get(&app, "/").await;
    assert!(page.contains("party-1.0.0.kmpkg"), "{page}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticked_box_builds_a_linked_package_over_the_folder_that_was_read() {
    let songs = Scratch::new("flow-linked-songs");
    let bytes = km_song::testing::soft_karaoke();
    std::fs::create_dir_all(songs.join("rock")).expect("a folder");
    std::fs::write(songs.join("rock").join("a.kar"), &bytes).expect("write");
    let out = Scratch::new("flow-linked-out");

    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let (status, _) = post(&app, "/folder", &format!("path={}", form_value(&songs))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;

    let page = get(&app, "/").await;
    assert!(
        page.contains(r#"<input type="checkbox" name="linked" value="yes">"#),
        "the box is offered, and off: {page}"
    );

    let body = format!(
        "name=Party&version=1.0.0&publisher=&language=und&linked=yes&out_dir={}",
        form_value(&out)
    );
    let (status, _) = post(&app, "/build", &body).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;
    assert!(
        matches!(app.lock().phase, Phase::Built(_)),
        "{:?}",
        app.lock().error
    );

    let file = out.join("party-1.0.0.kmpkg");
    let package = km_kmpkg::Package::open(&file).expect("open the package");
    assert!(package.is_linked());
    assert!(package.is_uncurated(), "and still uncurated");
    assert_eq!(package.linked_root(), Some("rock"));
    assert_eq!(package.manifest().songs[0].file, "a.kar");
    assert_eq!(
        package
            .with_linked_dir(songs.path())
            .read_song(1)
            .expect("read through the folder"),
        bytes
    );

    let page = get(&app, "/").await;
    assert!(page.contains("Each one is linked"), "{page}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_paths_of_a_linked_package_can_start_above_the_folder_that_was_read() {
    let library = Scratch::new("flow-linked-base");
    let bytes = km_song::testing::soft_karaoke();
    let folder = library.join("collection");
    std::fs::create_dir_all(folder.join("rock")).expect("a folder");
    std::fs::write(folder.join("rock").join("a.kar"), &bytes).expect("write");
    let out = Scratch::new("flow-linked-base-out");

    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let (status, _) = post(&app, "/folder", &format!("path={}", form_value(&folder))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;

    let page = get(&app, "/").await;
    assert!(
        page.contains("rock/a.kar"),
        "the folder itself to start: {page}"
    );
    let preview = get(
        &app,
        &format!("/linked-path?linked_base={}", form_value(&library)),
    )
    .await;
    assert!(preview.contains("collection/rock/a.kar"), "{preview}");
    let refused = get(
        &app,
        &format!(
            "/linked-path?linked_base={}",
            form_value(&folder.join("rock"))
        ),
    )
    .await;
    assert!(refused.contains(r#"class="error""#), "{refused}");

    let body = format!(
        "name=Party&version=1.0.0&publisher=&language=und&linked=yes&linked_base={}&out_dir={}",
        form_value(&library),
        form_value(&out)
    );
    let (status, _) = post(&app, "/build", &body).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;
    assert!(
        matches!(app.lock().phase, Phase::Built(_)),
        "{:?}",
        app.lock().error
    );

    let package = km_kmpkg::Package::open(out.join("party-1.0.0.kmpkg")).expect("open the package");
    assert_eq!(package.linked_root(), Some("collection/rock"));
    assert_eq!(package.manifest().songs[0].file, "a.kar");
    assert_eq!(
        package
            .with_linked_dir(library.path())
            .read_song(1)
            .expect("read through the folder above"),
        bytes
    );
}

/// A real MP3, the one the CD+G player's own tests play.
const TONE: &[u8] = include_bytes!("../../../../crates/playback/km-cdg/fixtures/tone.mp3");

/// A CD+G stream that draws one tile, which is what makes it a song's words.
fn graphics(colour: u8) -> Vec<u8> {
    let mut packet = [0u8; 24];
    packet[0] = 9;
    packet[1] = 6;
    packet[4..8].copy_from_slice(&[colour, 1, 2, 3]);
    packet[8..20].fill(0x3F);
    let mut stream = packet.to_vec();
    stream.resize(24 * 300, 0);
    stream
}

/// Reads `folder`, builds it linked over `base` with the description box ticked, and returns
/// what was built.
async fn build_linked_with_descriptions(
    folder: &Path,
    base: &Path,
    out: &Path,
) -> km_package_simple::app::Built {
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let (status, _) = post(&app, "/folder", &format!("path={}", form_value(folder))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;

    let page = get(&app, "/").await;
    assert!(
        page.contains(r#"<input type="checkbox" name="write_spec" value="yes">"#),
        "the description box is offered, and off: {page}"
    );

    let body = format!(
        "name=Party&version=1.0.0&publisher=&language=und&linked=yes&write_spec=yes\
         &linked_base={}&out_dir={}",
        form_value(base),
        form_value(out)
    );
    let (status, _) = post(&app, "/build", &body).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;
    let built = match &app.lock().phase {
        Phase::Built(built) => built.clone(),
        other => panic!("not built: {other:?} {:?}", app.lock().error),
    };
    let page = get(&app, "/").await;
    assert!(page.contains("party-1.0.0.kmspec.yaml"), "{page}");
    built
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copied_pair_is_skipped_and_the_description_rebuilds_the_package() {
    let library = Scratch::new("flow-copies");
    let folder = library.join("cdg").join("collection");
    std::fs::create_dir_all(&folder).expect("a folder");
    for (stem, colour) in [("a", 0), ("a2", 0), ("b", 1)] {
        std::fs::write(folder.join(format!("{stem}.mp3")), TONE).expect("write");
        std::fs::write(folder.join(format!("{stem}.cdg")), graphics(colour)).expect("write");
    }
    let out = Scratch::new("flow-copies-out");

    let built = build_linked_with_descriptions(&folder, library.path(), &out).await;
    assert_eq!(built.files.len(), 1);
    assert_eq!(built.files[0].1, 2, "the copy stayed out");
    assert_eq!(built.skipped.len(), 1, "{:?}", built.skipped);
    assert!(built.skipped[0].source.ends_with("a2.mp3"));
    assert_eq!(built.skipped[0].why, "the same recording as song 1");

    let spec_path = out.join("party-1.0.0.kmspec.yaml");
    assert_eq!(built.specs, std::slice::from_ref(&spec_path));
    let spec = km_pack::Spec::read(&spec_path).expect("the description reads");
    let files: Vec<&str> = spec.songs.iter().map(|song| song.file.as_str()).collect();
    assert_eq!(files, ["cdg/collection/a.mp3", "cdg/collection/b.mp3"]);

    // The description makes the same package with no flag.
    let again = out.join("again.kmpkg");
    let outcome = km_pack::build::build(
        &spec,
        &km_pack::BuildOptions {
            base: &spec.base(&spec_path),
            out: Some(&again),
            dry_run: false,
            measure_loudness: false,
            write_listing: false,
            flags: km_kmpkg::PackageFlags::NONE,
            skip_duplicates: false,
        },
        |_| std::ops::ControlFlow::Continue(()),
    )
    .expect("build");
    assert!(
        outcome.wrote(),
        "{:?} {:?}",
        outcome.problems,
        outcome.skipped
    );
    let first = km_kmpkg::Package::open(out.join("party-1.0.0.kmpkg")).expect("open");
    let second = km_kmpkg::Package::open(&again).expect("open");
    let hashes = |package: &km_kmpkg::Package| -> Vec<Option<String>> {
        package
            .manifest()
            .songs
            .iter()
            .map(|song| song.content_hash.clone())
            .collect()
    };
    assert_eq!(hashes(&first), hashes(&second));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_post_from_another_site_is_refused() {
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let request = Request::post("/quit")
        .header("sec-fetch-site", "cross-site")
        .body(Body::empty())
        .expect("request");
    let answer = http::send(km_package_simple::server::router(Arc::clone(&app)), request).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_is_not_there_is_said_and_nothing_starts() {
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let (status, _) = post(&app, "/folder", "path=%2Fno%2Fsuch%2Ffolder").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(matches!(app.lock().phase, Phase::Empty));
    let page = get(&app, "/").await;
    assert!(page.contains("There is no folder at that path"), "{page}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folder_picker_is_offered_and_lists_nothing_until_pressed() {
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let page = get(&app, "/").await;
    assert!(
        page.contains(r##"hx-get="/folders" hx-target="#browse-path" hx-swap="innerHTML""##),
        "the form's hx-swap=none must not be inherited: {page}"
    );
    assert!(page.contains(r#"<div id="browse-path"></div>"#), "{page}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folder_picker_lists_folders_sorted_and_narrows_by_name() {
    let root = Scratch::new("flow-picker");
    for name in ["beta", "Alpha", ".hidden"] {
        std::fs::create_dir_all(root.join(name)).expect("making a test folder");
    }
    std::fs::write(root.join("song.kar"), km_song::testing::soft_karaoke()).expect("write");
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());

    let picker = get(&app, &format!("/folders?for=path&at={}", form_value(&root))).await;
    let alpha = picker.find(">Alpha<").expect("Alpha is listed");
    let beta = picker.find(">beta<").expect("beta is listed");
    assert!(alpha < beta, "the order folds case: {picker}");
    assert!(
        !picker.contains(">.hidden<"),
        "a dotted folder is not walked to"
    );
    assert!(!picker.contains("song.kar"), "a file is not a folder");
    assert!(picker.contains(r#"data-use="path""#), "{picker}");

    let narrowed = get(
        &app,
        &format!(
            "/folders?for=path&rows=1&filter=ALP&at={}",
            form_value(&root)
        ),
    )
    .await;
    assert!(narrowed.contains(">Alpha<"), "{narrowed}");
    assert!(!narrowed.contains(">beta<"), "{narrowed}");
    assert!(
        narrowed
            .trim_start()
            .starts_with(r#"<div id="folders-path">"#),
        "{narrowed}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folder_picker_says_why_a_folder_cannot_be_read() {
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let router = km_package_simple::server::router(Arc::clone(&app));
    let answer = http::send(
        router,
        http::get("/folders?for=out_dir&at=%2Fno%2Fsuch%2Ffolder"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(
        answer.text().contains("could not be read"),
        "{}",
        answer.text()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folder_picker_lists_the_top_and_refuses_an_unknown_box() {
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let router = km_package_simple::server::router(Arc::clone(&app));
    let top = http::send(router, http::get("/folders?for=path&drives=1")).await;
    assert_eq!(top.status, StatusCode::OK);

    let router = km_package_simple::server::router(Arc::clone(&app));
    let unknown = http::send(router, http::get("/folders?for=name")).await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
}
