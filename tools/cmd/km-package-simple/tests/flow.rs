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

    // Rename the first song and leave the second out.
    let (status, _) = post(&app, "/songs/0", "title=Renamed&artist=Somebody").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, list) = post(&app, "/songs/keep?page=0", "from=1&to=1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains("left-out"), "{list}");

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
