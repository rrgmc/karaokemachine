//! The whole flow through the router: browse to a folder, start the editor, read how it ended.
//!
//! No test here starts a process. The launcher is a stand-in that records what it was asked to
//! run, and writes the synced copy where a test wants the editor to have saved.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{HeaderValue, StatusCode};
use km_song_sync::app::{App, Config, Editor, Outcome};
use km_song_sync::launch::{Ended, Launch, Launcher, Words};
use km_testkit::{Scratch, http};

/// A launcher that starts nothing.
#[derive(Debug, Default)]
struct Recorded {
    /// Every launch asked for.
    launches: Mutex<Vec<Launch>>,
    /// A file to write, as an editor that saved.
    saves: Option<PathBuf>,
    /// What the editor says for refusing to start.
    refusal: Option<String>,
}

impl Launcher for Recorded {
    fn run(&self, launch: &Launch) -> Ended {
        self.launches.lock().expect("the lock").push(launch.clone());
        if let Some(file) = &self.saves {
            std::fs::write(file, b"synced").expect("writing the synced copy");
        }
        Ended {
            refusal: self.refusal.clone(),
        }
    }
}

/// A tool over `folder`, with a stand-in machine beside it and `launcher` in place of a process.
fn tool(folder: &Scratch, launcher: Arc<Recorded>) -> Arc<App> {
    let machine = folder.join("the-machine");
    std::fs::write(&machine, b"").expect("writing the stand-in machine");
    App::new(Config {
        settings_path: None,
        url: "http://127.0.0.1:0/".to_owned(),
        folder: Some(folder.to_path_buf()),
        current_dir: None,
        exe_dir: None,
        machine_exe: Some(machine),
        machine_data_dir: None,
        scratch: folder.join("scratch"),
        launcher,
    })
}

/// A form post from this tool's own page, which carries `sec-fetch-site: same-origin`.
async fn post(app: &Arc<App>, uri: &str, body: &str) -> (StatusCode, String) {
    let mut request = http::form(uri, body.to_owned());
    request
        .headers_mut()
        .insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    let answer = http::send(km_song_sync::server::router(Arc::clone(app)), request).await;
    (answer.status, answer.text())
}

async fn get(app: &Arc<App>, uri: &str) -> String {
    let router = km_song_sync::server::router(Arc::clone(app));
    http::send(router, http::get(uri)).await.text()
}

/// Waits until the editor has closed.
async fn closed(app: &Arc<App>) -> Outcome {
    for _ in 0..600 {
        if let Editor::Ended(outcome) = &app.lock().editor {
            return outcome.clone();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the editor did not close");
}

/// A start form's body.
fn start_form(song: &Path, rest: &[(&str, &str)]) -> String {
    let song = song.display().to_string();
    let mut fields = vec![("song", song.as_str())];
    fields.extend_from_slice(rest);
    serde_urlencoded::to_string(fields).expect("encode")
}

fn args(launch: &Launch) -> Vec<String> {
    launch
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_lists_folders_and_midi_files_and_nothing_else() {
    let folder = Scratch::new("sync-flow-list");
    std::fs::create_dir_all(folder.join("more")).expect("making a test folder");
    std::fs::write(folder.join("sung.kar"), km_song::testing::soft_karaoke()).expect("write");
    std::fs::write(folder.join("tune.mid"), km_song::testing::instrumental()).expect("write");
    std::fs::write(folder.join("tune.txt"), "la la\n").expect("write");
    std::fs::write(folder.join("cover.jpg"), b"").expect("write");
    let app = tool(&folder, Arc::default());

    let page = get(&app, "/").await;
    assert!(page.contains(">more<"), "a folder is walked to: {page}");
    assert!(page.contains(">sung.kar<"), "{page}");
    assert!(page.contains(">tune.mid<"), "{page}");
    assert!(!page.contains(">tune.txt<"), "a text file is not a song");
    assert!(!page.contains("cover.jpg"), "{page}");
    assert!(
        page.contains(r#"<textarea id="words-box" name="words""#),
        "{page}"
    );
    assert!(
        page.contains("Uses the words in tune.txt,"),
        "the row names the text file beside the song: {page}"
    );
    assert!(page.contains("Has words already"), "{page}");

    let narrowed = get(
        &app,
        &format!(
            "/browse?rows=1&filter=SUNG&at={}",
            start_form(&folder, &[]).trim_start_matches("song=")
        ),
    )
    .await;
    assert!(narrowed.contains(">sung.kar<"), "{narrowed}");
    assert!(!narrowed.contains(">tune.mid<"), "{narrowed}");
    assert!(narrowed.trim_start().starts_with(r#"<div id="rows""#));
}

#[tokio::test(flavor = "multi_thread")]
async fn pasted_words_reach_the_editor_and_a_saved_copy_is_reported() {
    let folder = Scratch::new("sync-flow-pasted");
    let song = folder.join("tune.mid");
    std::fs::write(&song, km_song::testing::instrumental()).expect("write");
    std::fs::write(folder.join("tune.txt"), "from the file\n").expect("write");
    let launcher = Arc::new(Recorded {
        saves: Some(folder.join("tune.kar")),
        ..Recorded::default()
    });
    let app = tool(&folder, Arc::clone(&launcher));

    let (status, panel) = post(
        &app,
        "/start",
        &start_form(
            &song,
            &[
                ("words", "pas-ted words\r\n"),
                ("use_words", "on"),
                ("resume", "on"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(panel.contains(r#"id="editor""#), "{panel}");

    assert_eq!(
        closed(&app).await,
        Outcome::Saved {
            out: folder.join("tune.kar")
        }
    );
    let launches = launcher.launches.lock().expect("the lock").clone();
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0].words,
        Words::Piped("pas-ted words\n".to_owned()),
        "pasted words win over the text file"
    );
    assert!(args(&launches[0]).contains(&"--sync-continue".to_owned()));

    let router = km_song_sync::server::router(Arc::clone(&app));
    let polled = http::send(router, http::get("/editor")).await;
    assert_eq!(
        polled.header("HX-Trigger"),
        Some("editor-ended"),
        "the browser draws again once the editor has closed"
    );
    assert!(
        polled.text().contains("Saved tune.kar"),
        "{}",
        polled.text()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_box_uses_the_text_file_and_then_the_songs_own_words() {
    let folder = Scratch::new("sync-flow-sidecar");
    let tune = folder.join("tune.mid");
    let sung = folder.join("sung.kar");
    std::fs::write(&tune, km_song::testing::instrumental()).expect("write");
    std::fs::write(folder.join("tune.txt"), "from the file\n").expect("write");
    std::fs::write(&sung, km_song::testing::soft_karaoke()).expect("write");
    let launcher = Arc::new(Recorded::default());
    let app = tool(&folder, Arc::clone(&launcher));

    post(&app, "/start", &start_form(&tune, &[("resume", "on")])).await;
    closed(&app).await;
    post(&app, "/start", &start_form(&sung, &[("words", "  \n")])).await;
    closed(&app).await;

    let launches = launcher.launches.lock().expect("the lock");
    assert_eq!(
        launches[0].words,
        Words::Piped("from the file\n".to_owned())
    );
    assert_eq!(launches[1].words, Words::None);
    assert!(
        !args(&launches[1]).contains(&"--sync-words".to_owned()),
        "the song's own words are opened with nothing given"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_makes_no_sense_is_refused_and_starts_nothing() {
    let folder = Scratch::new("sync-flow-refused");
    let tune = folder.join("tune.mid");
    std::fs::write(&tune, km_song::testing::instrumental()).expect("write");
    let synced = folder.join("synced.mid");
    std::fs::write(&synced, km_song::testing::instrumental()).expect("write");
    std::fs::write(folder.join("synced.kar"), b"").expect("write");
    std::fs::write(folder.join("notes.txt"), "la\n").expect("write");
    let launcher = Arc::new(Recorded::default());
    let app = tool(&folder, Arc::clone(&launcher));

    let (_, no_words) = post(&app, "/start", &start_form(&tune, &[])).await;
    assert!(no_words.contains("has no words"), "{no_words}");

    // The box is ticked and holds nothing: somebody meant words to be there.
    let (_, empty) = post(
        &app,
        "/start",
        &start_form(&tune, &[("words", " \n"), ("use_words", "on")]),
    )
    .await;
    assert!(empty.contains("box is empty"), "{empty}");

    let (_, exists) = post(
        &app,
        "/start",
        &start_form(&synced, &[("words", "la"), ("use_words", "on")]),
    )
    .await;
    assert!(exists.contains("already there"), "{exists}");

    let (_, not_a_song) = post(
        &app,
        "/start",
        &start_form(
            &folder.join("notes.txt"),
            &[("words", "la"), ("use_words", "on")],
        ),
    )
    .await;
    assert!(not_a_song.contains("Select a MIDI file"), "{not_a_song}");

    assert!(launcher.launches.lock().expect("the lock").is_empty());
    assert_eq!(app.lock().editor, Editor::Idle);

    // The same row with its box ticked starts, and the editor is told to replace the copy.
    post(
        &app,
        "/start",
        &start_form(
            &synced,
            &[("words", "la"), ("use_words", "on"), ("force", "on")],
        ),
    )
    .await;
    closed(&app).await;
    let launches = launcher.launches.lock().expect("the lock");
    assert!(args(&launches[0]).contains(&"--sync-force".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_typed_name_reaches_the_editor_only_where_the_song_states_none() {
    let folder = Scratch::new("sync-flow-names");
    let tune = folder.join("tune.mid");
    let sung = folder.join("sung.kar");
    std::fs::write(&tune, km_song::testing::instrumental()).expect("write");
    std::fs::write(&sung, km_song::testing::soft_karaoke()).expect("write");
    let launcher = Arc::new(Recorded::default());
    let app = tool(&folder, Arc::clone(&launcher));

    let page = get(&app, "/").await;
    assert!(page.contains(r#"<input type="text" id="title" name="title""#));
    assert!(page.contains(r#"<option value="pt">Portuguese</option>"#));
    assert!(page.contains(r#"data-artist="""#), "{page}");

    let typed = [
        ("words", "la la"),
        ("use_words", "on"),
        ("artist", "  The Singers "),
        ("language", "PT"),
        ("title", " "),
    ];
    post(&app, "/start", &start_form(&tune, &typed)).await;
    closed(&app).await;
    let over_a_stated_title = [("title", "Another Name"), ("language", "no such code")];
    post(&app, "/start", &start_form(&sung, &over_a_stated_title)).await;
    closed(&app).await;

    let launches = launcher.launches.lock().expect("the lock");
    let named = args(&launches[0]);
    assert!(named.contains(&"--sync-artist=The Singers".to_owned()));
    assert!(named.contains(&"--sync-language=pt".to_owned()));
    assert!(
        !named.iter().any(|arg| arg.starts_with("--sync-title")),
        "a blank field passes nothing: {named:?}"
    );
    let stated = args(&launches[1]);
    assert!(
        !stated.iter().any(|arg| arg.starts_with("--sync-title")),
        "the song's own title stands: {stated:?}"
    );
    assert!(
        !stated.iter().any(|arg| arg.starts_with("--sync-language")),
        "{stated:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_editors_own_refusal_reaches_the_page() {
    let folder = Scratch::new("sync-flow-failed");
    let sung = folder.join("sung.kar");
    std::fs::write(&sung, km_song::testing::soft_karaoke()).expect("write");
    let launcher = Arc::new(Recorded {
        refusal: Some("the text has 3 words".to_owned()),
        ..Recorded::default()
    });
    let app = tool(&folder, launcher);

    post(
        &app,
        "/start",
        &start_form(&sung, &[("words", "la la la"), ("use_words", "on")]),
    )
    .await;
    assert!(matches!(closed(&app).await, Outcome::Failed { .. }));

    let panel = get(&app, "/editor").await;
    assert!(panel.contains("the text has 3 words"), "{panel}");
    assert!(
        !panel.contains("hx-trigger"),
        "a closed editor is not polled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_machine_the_page_says_so_and_no_start_is_offered() {
    let folder = Scratch::new("sync-flow-no-machine");
    let sung = folder.join("sung.kar");
    std::fs::write(&sung, km_song::testing::soft_karaoke()).expect("write");
    let app = App::new(Config {
        settings_path: None,
        url: "http://127.0.0.1:0/".to_owned(),
        folder: Some(folder.to_path_buf()),
        current_dir: None,
        exe_dir: None,
        machine_exe: Some(folder.join("not-there")),
        machine_data_dir: None,
        scratch: folder.join("scratch"),
        launcher: Arc::new(Recorded::default()),
    });

    let page = get(&app, "/").await;
    assert!(page.contains("was not found beside this program"), "{page}");
    assert!(
        page.contains(r#"hx-swap="outerHTML" data-no-machine>"#),
        "{page}"
    );

    let (_, refused) = post(&app, "/start", &start_form(&sung, &[])).await;
    assert!(refused.contains("was not found"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_cannot_be_read_says_why_and_another_site_cannot_press_start() {
    let folder = Scratch::new("sync-flow-gate");
    let sung = folder.join("sung.kar");
    std::fs::write(&sung, km_song::testing::soft_karaoke()).expect("write");
    let launcher = Arc::new(Recorded::default());
    let app = tool(&folder, Arc::clone(&launcher));

    let unreadable = get(&app, "/browse?at=%2Fno%2Fsuch%2Ffolder").await;
    assert!(unreadable.contains("could not be read"), "{unreadable}");

    let mut request = http::form("/start", start_form(&sung, &[]));
    request
        .headers_mut()
        .insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
    let answer = http::send(km_song_sync::server::router(Arc::clone(&app)), request).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert!(launcher.launches.lock().expect("the lock").is_empty());
}
