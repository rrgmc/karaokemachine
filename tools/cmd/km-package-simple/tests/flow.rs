//! The whole flow through the router: read a folder, rename and leave out, build, open the file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use km_package_simple::app::{App, Phase};
use tower::ServiceExt as _;

/// A scratch folder of this test's own, removed however the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("km-package-simple-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn post(app: &Arc<App>, uri: &str, body: &str) -> (StatusCode, String) {
    let response = km_package_simple::server::router(Arc::clone(app))
        .oneshot(
            Request::post(uri)
                .header("content-type", "application/x-www-form-urlencoded")
                .header("sec-fetch-site", "same-origin")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn get(app: &Arc<App>, uri: &str) -> String {
    let response = km_package_simple::server::router(Arc::clone(app))
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
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
    std::fs::write(songs.0.join("a.kar"), km_song::testing::soft_karaoke()).expect("write");
    std::fs::write(songs.0.join("b.mid"), km_song::testing::lyric_events()).expect("write");
    std::fs::write(songs.0.join("copy.kar"), km_song::testing::soft_karaoke()).expect("write");
    let out = Scratch::new("flow-out");

    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let (status, _) = post(&app, "/folder", &format!("path={}", form_value(&songs.0))).await;
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
        form_value(&out.0)
    );
    let (status, _) = post(&app, "/build", &body).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    settled(&app).await;
    let built = match &app.lock().phase {
        Phase::Built(built) => built.clone(),
        other => panic!("not built: {other:?} {:?}", app.lock().error),
    };
    assert_eq!(built.files.len(), 1);

    let file = out.0.join("party-1.0.0.kmpkg");
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
        out.0.join("party-1.0.0.kmpkg.txt").is_file(),
        "a listing beside it"
    );

    let page = get(&app, "/").await;
    assert!(page.contains("party-1.0.0.kmpkg"), "{page}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_post_from_another_site_is_refused() {
    let app = App::new(None, "http://127.0.0.1:0/".to_owned());
    let response = km_package_simple::server::router(Arc::clone(&app))
        .oneshot(
            Request::post("/quit")
                .header("sec-fetch-site", "cross-site")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
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
