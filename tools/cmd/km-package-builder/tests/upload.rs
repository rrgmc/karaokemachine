//! Sending a song — and a package — to a machine that cannot see this box's disk.
//!
//! **Against a real listening server, because the interesting failure is on the wire.** Every other
//! test of this feature is on one side of it or the other: `src/app.rs` checks which route an address
//! picks, and `km-api`'s surface tests check what the machine does with a multipart body assembled by
//! hand. Neither would notice the thing most likely to be wrong — that the body `reqwest` writes is
//! not the body `axum` reads.
//!
//! The server binds an **ephemeral loopback** port, which is not a detail: on Windows a test binary's
//! path carries a build hash, so a test that bound a non-loopback address would raise a fresh
//! firewall prompt on every rebuild and leave a dead rule behind. See `No test binds a non-loopback
//! address` in the repository's notes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use km_api::testing::{Decided, Faults, Recorded, TestMachine};
use km_api::{ApiConfig, ApiState, Listening, bind};
use km_package_builder::app::Client;

/// The password the test machine is set up with.
const MACHINE_PASSWORD: &str = "curate1975";

/// A machine listening on a loopback port, stopped when this is dropped.
struct Machine {
    base: String,
    machine: Arc<TestMachine>,
    task: tokio::task::JoinHandle<()>,
}

impl Machine {
    async fn start(faults: Faults) -> Self {
        Self::start_with_debugging(faults, true).await
    }

    async fn start_with_debugging(faults: Faults, debugging: bool) -> Self {
        let machine = TestMachine::new().shared();
        machine.set_faults(faults);
        let state = ApiState::from_machine(
            machine.clone(),
            // A password and debugging on, which is what these tests need to reach anything: the
            // package routes are admin now, and the two play routes are not mounted at all with
            // debugging off. Both are properties of a real machine an owner has set up for
            // curation, which is the situation this file is about.
            {
                let mut config = ApiConfig::default()
                    .without_mdns()
                    .on_ephemeral_port()
                    .with_password(MACHINE_PASSWORD)
                    .expect("argon2 hashes a password");
                config.debug_enabled = debugging;
                config
            },
        );
        let listening: Listening = bind(state).await.expect("bind");
        let addr = listening.local_addr;
        let task = tokio::spawn(async move {
            let _ = listening.serve().await;
        });
        Self {
            // `is_loopback` would say true of this and send a path instead, so every test here calls
            // `play_upload` directly rather than going through the branch. Which branch an address
            // picks is `app.rs`'s own business and is tested there against eleven spellings.
            base: format!("http://{addr}"),
            machine,
            task,
        }
    }

    /// A client that has already logged in, because the package routes are admin.
    ///
    /// `play_upload` does not need this — the debug routes are public whenever they are mounted —
    /// but installing does, and every test here is about a curator who knows the machine's password.
    async fn client(&self) -> Client {
        let client = Client::new(&self.base);
        client
            .log_in(MACHINE_PASSWORD)
            .await
            .expect("the test machine's password is right");
        client
    }

    /// The same client with no token, for the refusal a curator who does not know it gets.
    fn stranger(&self) -> Client {
        Client::new(&self.base)
    }

    fn played(&self) -> Vec<String> {
        self.machine
            .recorded()
            .iter()
            .filter_map(|entry| match entry {
                Recorded::PlayAudition(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// Everything the machine was told a curator had already settled, one entry per audition.
    fn played_with(&self) -> Vec<Decided> {
        self.machine
            .recorded()
            .iter()
            .filter_map(|entry| match entry {
                Recorded::Decided(decided) => Some(decided.clone()),
                _ => None,
            })
            .collect()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A scratch folder holding one or two files, removed when the test ends.
struct Songs(PathBuf);

impl Songs {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("km-builder-upload-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        Self(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("write");
        path
    }
}

impl Drop for Songs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("song")
        .to_owned()
}

/// The whole feature, over a socket: bytes in, a staged song playing.
#[tokio::test]
async fn a_song_sent_over_the_wire_is_played_under_the_stem_it_was_sent_with() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("one");
    let song = songs.write("whatever-the-file-is-called.kar", b"MThd not really a midi");

    machine
        .client()
        .await
        .play_upload(
            &song,
            None,
            "Sultans of Swing",
            &km_api::Audition::default(),
        )
        .await
        .expect("the machine takes it");

    // Named from the stem and the part's *extension* — never from the file's own name.
    assert_eq!(machine.played(), vec!["Sultans of Swing.kar".to_owned()]);
}

/// The pair is why an upload is staged in a folder rather than streamed at a decoder.
#[tokio::test]
async fn both_halves_of_an_mp3_g_song_cross_in_one_request() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("pair");
    // Deliberately the shapes a real corpus has: a mixed-case extension, and a stem with a trailing
    // space. Neither may reach the machine's disk, because both halves are named from the stem.
    let audio = songs.write("Perfidia .MP3", b"ID3 audio");
    let graphics = songs.write("Perfidia .Cdg", b"CDG graphics");

    machine
        .client()
        .await
        .play_upload(
            &audio,
            Some(&graphics),
            "Perfidia",
            &km_api::Audition::default(),
        )
        .await
        .expect("the machine takes both");

    // One call, naming the half sent first. The machine finds the other beside it.
    assert_eq!(machine.played(), vec!["Perfidia.mp3".to_owned()]);
}

/// A stem taken from a real filename, which is where accents actually come from.
#[tokio::test]
async fn a_stem_with_accents_in_it_survives_the_wire() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("accents");
    let song = songs.write("O Poeta Está Vivo.kar", b"MThd");
    let stem = stem_of(&song);

    machine
        .client()
        .await
        .play_upload(&song, None, &stem, &km_api::Audition::default())
        .await
        .expect("the machine takes it");

    assert_eq!(machine.played(), vec!["O Poeta Está Vivo.kar".to_owned()]);
}

/// The shipped default, and the message the tool answers it with.
///
/// **A machine with debugging off, rather than one whose controller refuses.** That distinction used
/// to be invisible — `accepts_uploads` came from the controller, so a fault could stand in for the
/// setting. It cannot now: what `/discover` reports is the mode, and the mode is what decides whether
/// the route is mounted at all. Modelling it with a fault would test a disagreement a real machine
/// cannot have.
#[tokio::test]
async fn a_machine_not_in_debugging_mode_is_answered_with_the_setting_to_change() {
    let machine = Machine::start_with_debugging(Faults::default(), false).await;
    let songs = Songs::new("refused");
    let song = songs.write("song.kar", b"MThd");

    let error = machine
        .client()
        .await
        .play_upload(&song, None, "song", &km_api::Audition::default())
        .await
        .expect_err("refused");

    let said = error.to_string();
    // The machine's own words, plus the JSON to paste — in the nested shape the file really has.
    assert!(said.contains("not in debugging mode"), "{said}");
    assert!(said.contains("\"enabled\": true"), "{said}");
    assert!(said.contains("--show-paths"), "{said}");
    assert!(machine.played().is_empty());
    // **Nothing was sent**, which is the assertion and not a bonus: the route was never reached,
    // so no folder was opened. A flake here would be the bug — a server that refuses mid-request
    // and closes leaves the sending half looking like a dropped connection, so the message above
    // arrives as "the machine is not answering" whenever the body is big enough to still be going.
    // A four-byte fixture usually wins that race; a video never would.
    assert!(
        !machine
            .machine
            .recorded()
            .iter()
            .any(|entry| matches!(entry, Recorded::OpenAudition)),
        "the upload route should not have been reached at all"
    );
}

/// The bug this pair exists for: both install routes came back as a `serde_json::Value`, whose
/// `Display` is the raw JSON, so the message beside the Install button read
/// `Installed into http://…:8177. {"report":"installed \"favtest1\" · 155 songs"}`.
///
/// This is the upload half — the machine words the sentence, because only the machine knows what it
/// did with the bytes, and the tool's job is to read the one field and nothing else.
#[tokio::test]
async fn an_uploaded_package_comes_back_as_a_sentence_and_not_as_a_body() {
    let machine = Machine::start(Faults::default()).await;
    let packages = Songs::new("package-upload");
    let package = packages.write("favtest1.kmpkg", b"PK\x03\x04 not really a package");

    let said = machine
        .client()
        .await
        .upload_package(&package)
        .await
        .expect("the machine takes it");

    // The double's own wording rather than a real install's, which is the point: whatever the
    // machine says is what arrives, with nothing wrapped around it.
    assert_eq!(said, "took favtest1.kmpkg (25 bytes)");
    assert!(!said.contains('{'), "a JSON body reached the page: {said}");
}

/// A curator who has not signed in is refused, and both send routes refuse the same way.
///
/// **The cost of `packages.install` becoming an admin action, over a socket.** That route was public
/// for as long as this tool had nowhere to keep a password; it has one now, and this is what happens
/// before somebody uses it. Worth asserting on both halves because they take different code paths —
/// one a JSON body, one a multipart stream — and a 401 that arrived as a dropped connection on the
/// streaming half would be the same failure the pre-flight in `play_upload` exists to avoid.
#[tokio::test]
async fn a_curator_who_has_not_signed_in_is_refused_by_both_send_routes() {
    let machine = Machine::start(Faults::default()).await;
    let packages = Songs::new("no-password");
    let package = packages.write("favtest1.kmpkg", b"PK\x03\x04 not really a package");

    let error = machine
        .stranger()
        .upload_package(&package)
        .await
        .expect_err("an unauthenticated upload must be refused");
    assert!(
        matches!(error, km_package_builder::app::AppError::Refused(_)),
        "a refusal, not a transport failure: {error}"
    );

    let by_upload = error.to_string();

    let error = machine
        .stranger()
        .install_package(&package)
        .await
        .expect_err("an unauthenticated install must be refused");
    assert!(
        matches!(error, km_package_builder::app::AppError::Refused(_)),
        "a refusal, not a transport failure: {error}"
    );
    let by_path = error.to_string();

    // **Both roads say the same sentence**, which is the invariant `Both roads to an install say the
    // same sentence, and it is written once` makes about the *report* and which had never been true
    // of the refusal. For a while only the upload half explained itself, so one missing password
    // read as three paragraphs of instructions on one branch and as km-api's
    // `'…' needs the admin password; send an admin token` on the other.
    for said in [&by_upload, &by_path] {
        assert!(
            said.contains("Password box on the machine panel"),
            "the refusal must name the control that answers it: {said}"
        );
        assert!(
            said.contains("/admin/") && said.contains("--show-paths"),
            "and the two ways through for somebody who will not use it: {said}"
        );
        // It lands in a `.message`, which is escaped and `pre-wrap`. See `app.rs`'s own test.
        assert!(
            !said.contains('*') && !said.contains('`'),
            "markdown: {said}"
        );
    }

    // ...and signing in opens both, which is what makes the refusal a step rather than a wall.
    machine
        .client()
        .await
        .upload_package(&package)
        .await
        .expect("the password opens it");
}

/// The path half, which answers a set of fields rather than a sentence — and is the branch nobody
/// had complained about yet because its JSON is less obviously JSON.
#[tokio::test]
async fn a_package_installed_by_path_is_read_as_fields_and_worded_here() {
    let machine = Machine::start(Faults::default()).await;

    let report = machine
        .client()
        .await
        .install_package(Path::new("packages/favtest1.kmpkg"))
        .await
        .expect("the machine installs it");

    assert_eq!(report.package_id, "favtest1");
    assert_eq!(report.songs_added, 2);
    // One sentence for both roads to an install. It lives in `km-api`'s DTO, so the machine's own
    // drop and upload paths say it too.
    assert_eq!(report.sentence(), "installed \"favtest1\" \u{b7} 2 songs");
    // The one thing this branch knows and the upload branch cannot: the same recording already in
    // the catalog under another number.
    assert_eq!(report.duplicate_content.len(), 1);
}

/// A file that went away between being listed and being played is this tool's fault, not a network
/// failure, and should not be reported as one.
#[tokio::test]
async fn a_song_that_is_no_longer_on_this_disk_fails_before_anything_is_sent() {
    let machine = Machine::start(Faults::default()).await;
    let error = machine
        .client()
        .await
        .play_upload(
            Path::new("no-such-folder/no-such-song.kar"),
            None,
            "gone",
            &km_api::Audition::default(),
        )
        .await
        .expect_err("there is nothing to send");

    assert!(error.to_string().contains("no-such-song.kar"), "{error}");
    assert!(machine.played().is_empty());
}

/// A curator's corrections cross with the song, so the preview is worth listening to.
///
/// **Without this the Play button answers a question nobody asked.** The corrections being
/// auditioned are not in the file and are in no package yet, so a machine left to detect for itself
/// plays the song exactly as it was found — and a curator who has just silenced a channel hears no
/// difference and concludes the control does nothing.
#[tokio::test]
async fn a_songs_corrections_cross_with_it() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("fixes");
    let song = songs.write("something.kar", b"MThd not really a midi");
    // The forced instrument is here for its argument: a correction whose name crossed without its
    // program would preview a song nobody chose.
    let decided = vec![
        km_fixes::Fix::IgnoreBankSelect { channel: 4 },
        km_fixes::Fix::MuteChannel { channel: 2 },
        km_fixes::Fix::ForceProgram {
            channel: 5,
            program: 52,
        },
    ];

    machine
        .client()
        .await
        .play_upload(
            &song,
            None,
            "Something",
            &km_api::Audition {
                fixes: Some(&decided),
                ..Default::default()
            },
        )
        .await
        .expect("the machine takes it");

    assert_eq!(
        machine.played_with(),
        vec![Decided {
            fixes: Some(decided),
            ..Default::default()
        }]
    );
}

/// An UltraStar song crosses as its MP3 with its words beside it, because the machine never reads
/// the `.txt`.
#[tokio::test]
async fn an_ultrastar_songs_words_cross_with_its_mp3() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("ultrastar");
    let audio = songs.write("Someone - Song.mp3", b"ID3 not really audio");
    let words = km_song::ultrastar::parse(
        b"#TITLE:Song\n#MP3:Someone - Song.mp3\n#BPM:300\n: 0 4 0 Hel\n: 4 2 0 lo\nE\n",
    )
    .expect("an UltraStar song")
    .timeline;

    machine
        .client()
        .await
        .play_upload(
            &audio,
            None,
            "notes",
            &km_api::Audition {
                lyrics: Some(&words),
                ..Default::default()
            },
        )
        .await
        .expect("the machine takes it");

    assert_eq!(machine.played(), vec!["notes.mp3".to_owned()]);
    assert_eq!(
        machine.played_with(),
        vec![Decided {
            lyrics: Some(words),
            ..Default::default()
        }]
    );
}

/// Deciding there should be no corrections is a decision, and is not the same as deciding nothing.
///
/// An empty list has to reach the machine as an empty list. Sending nothing instead would make a
/// song whose corrections were turned off sound exactly like a song nobody had touched, which is
/// the one distinction somebody unticking a box is listening for.
#[tokio::test]
async fn an_empty_list_crosses_as_an_empty_list() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("fixes-none");
    let song = songs.write("something.kar", b"MThd not really a midi");

    machine
        .client()
        .await
        .play_upload(
            &song,
            None,
            "Something",
            &km_api::Audition {
                fixes: Some(&[]),
                ..Default::default()
            },
        )
        .await
        .expect("the machine takes it");

    assert_eq!(
        machine.played_with(),
        vec![Decided {
            fixes: Some(Vec::new()),
            ..Default::default()
        }]
    );
}

/// A song nobody has decided about sends nothing, and the machine detects as it always has.
#[tokio::test]
async fn a_song_nobody_has_decided_about_sends_nothing() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("fixes-undecided");
    let song = songs.write("something.kar", b"MThd not really a midi");

    machine
        .client()
        .await
        .play_upload(&song, None, "Something", &km_api::Audition::default())
        .await
        .expect("the machine takes it");

    assert_eq!(machine.played_with(), vec![Decided::default()]);
}

/// The title and performer a curator typed cross with the song.
///
/// A corpus file's own title is frequently the arranger's, an abbreviation, or absent, which is why
/// somebody retypes it. A preview showing the file's version says one thing on the curation page and
/// another on the television, and the person checking is looking at both.
#[tokio::test]
async fn a_curators_title_and_performer_cross_with_the_song() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("names");
    let song = songs.write("iwtkwli.kar", b"MThd not really a midi");

    machine
        .client()
        .await
        .play_upload(
            &song,
            None,
            "iwtkwli",
            &km_api::Audition {
                title: Some("I Want to Know What Love Is"),
                artist: Some("Foreigner"),
                ..Default::default()
            },
        )
        .await
        .expect("the machine takes it");

    assert_eq!(
        machine.played_with(),
        vec![Decided {
            title: Some("I Want to Know What Love Is".to_owned()),
            artist: Some("Foreigner".to_owned()),
            ..Default::default()
        }]
    );
}

/// A song nobody has retyped sends nothing, and the machine reads the file as it always has.
#[tokio::test]
async fn a_song_nobody_has_renamed_sends_no_names() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("names-none");
    let song = songs.write("something.kar", b"MThd not really a midi");

    machine
        .client()
        .await
        .play_upload(&song, None, "Something", &km_api::Audition::default())
        .await
        .expect("the machine takes it");

    assert_eq!(machine.played_with(), vec![Decided::default()]);
}

/// The key a curator chose crosses with the song.
///
/// The one field of a preview whose absence is audible without knowing the song: somebody deciding
/// whether two semitones down is enough is listening for the key, and a preview in the file's own
/// key answers a question they are not asking.
#[tokio::test]
async fn a_curators_key_crosses_with_the_song() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("key");
    let song = songs.write("something.kar", b"MThd not really a midi");

    machine
        .client()
        .await
        .play_upload(
            &song,
            None,
            "Something",
            &km_api::Audition {
                transpose: Some(-2),
                ..Default::default()
            },
        )
        .await
        .expect("the machine takes it");

    assert_eq!(
        machine.played_with(),
        vec![Decided {
            transpose: Some(-2),
            ..Default::default()
        }]
    );
}

/// The melody channel a curator chose crosses with the song, and so does their saying there is none.
///
/// The machine offers its guide-melody toggle only on a song with a channel, so a preview that
/// detected for itself would hide the toggle on exactly the song whose detector abstained.
#[tokio::test]
async fn a_curators_melody_channel_crosses_with_the_song() {
    for (stem, melody) in [("Named", Some(2)), ("None", None)] {
        let machine = Machine::start(Faults::default()).await;
        let songs = Songs::new(&format!("melody-{stem}"));
        let song = songs.write("something.kar", b"MThd not really a midi");

        machine
            .client()
            .await
            .play_upload(
                &song,
                None,
                stem,
                &km_api::Audition {
                    melody: Some(melody),
                    ..Default::default()
                },
            )
            .await
            .expect("the machine takes it");

        assert_eq!(
            machine.played_with(),
            vec![Decided {
                melody: Some(melody),
                ..Default::default()
            }]
        );
    }
}

/// A part the machine cannot read as a number is ignored rather than refusing the whole upload.
///
/// The song still plays, in its own key. A transposition that failed to arrive is a song in the
/// wrong key, which anybody listening for a key hears at once — where refusing the upload would
/// leave a Play button that does nothing.
#[tokio::test]
async fn a_key_that_is_not_a_number_leaves_the_song_in_its_own() {
    let machine = Machine::start(Faults::default()).await;
    let songs = Songs::new("key-bad");
    let song = songs.write("something.kar", b"MThd not really a midi");

    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/debug/play-upload", machine.base))
        .multipart(
            reqwest::multipart::Form::new()
                .text("stem", "Something")
                .text("transpose", "two semitones down")
                .part(
                    "primary",
                    reqwest::multipart::Part::bytes(std::fs::read(&song).expect("read"))
                        .file_name("something.kar"),
                ),
        )
        .send()
        .await
        .expect("the machine answers");

    assert!(response.status().is_success());
    assert_eq!(machine.played_with(), vec![Decided::default()]);
}
