//! Fetching a bank, against a server this test controls.
//!
//! **Nothing here touches the network.** `wiremock` binds an ephemeral loopback port and answers
//! what each case needs — which is also the rule this repository has a scar about: a test that binds
//! a non-loopback address raises a fresh Windows firewall prompt for every rebuild, because a test
//! binary's path carries a build hash. Sixty dead rules accumulated once from a single `0.0.0.0:0`.
//!
//! What is worth testing here is not the happy path — a real download proved that — but the ways it
//! goes wrong, because those are the ones nobody sees until they happen to somebody.

use km_admin::bank;
use km_admin::job::Job;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// An HTTP client, with rustls' crypto provider installed first.
///
/// **That call is not a formality and this test proved it.** `reqwest` is built here with
/// `rustls-no-provider`, so `Client::new()` *panics* until something installs one — which is exactly
/// what these three cases did on their first run. It is why `install_crypto_provider` was made
/// public in the first place: `km-admin` shares `km-wallpaper-pack`'s `reqwest` and therefore its
/// missing provider, and every caller that builds its own client has to say so.
fn client() -> reqwest::Client {
    km_wallpaper_pack::providers::install_crypto_provider();
    reqwest::Client::new()
}

/// The smallest row in the table, so a test can use a real `CatalogBank` rather than invent one.
///
/// Real rather than synthetic on purpose: the digest, the exact byte count and the `sha1:` prefix
/// are the things being exercised, and a hand-written row would let all three drift from the table
/// the program actually reads.
fn gxscc() -> &'static km_banks::CatalogBank {
    km_banks::bank("gxscc").expect("gxscc is in the table")
}

/// The bytes that satisfy that row's digest — which nothing here has, so the cases below are all
/// failures. That is the point: the successful path needs the real file and is proved by running
/// the program, and every case here is one that leaves something behind if it is got wrong.
#[tokio::test]
async fn a_download_that_breaks_off_leaves_nothing_behind() {
    let server = MockServer::start().await;
    // A body far shorter than the `Content-Length` claims: the client reads what it can and the
    // stream then ends early, which is what a connection breaking mid-transfer looks like from
    // inside `bytes_stream`. This is the case that was found for real against archive.org, at
    // 81,669 bytes of 128,788.
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0_u8; 4096])
                .insert_header("content-length", "128788"),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("temp dir");
    let http = client();
    let job = Job::new("starting");

    let outcome = fetch_from(&http, &server.uri(), dir.path(), &job).await;
    assert!(outcome.is_err(), "a truncated body must not be accepted");

    assert_eq!(
        leftovers(dir.path()),
        Vec::<String>::new(),
        "a failed download must leave no .part file behind"
    );
}

#[tokio::test]
async fn a_body_that_does_not_match_the_digest_is_not_kept() {
    let server = MockServer::start().await;
    // The right length, the wrong bytes — so the size check passes and only the digest catches it.
    // This is the case the whole `digest` module exists for.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0_u8; 128_788]))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("temp dir");
    let http = client();
    let job = Job::new("starting");

    let refusal = fetch_from(&http, &server.uri(), dir.path(), &job)
        .await
        .expect_err("the digest must refuse it");
    assert!(
        refusal.contains("digest"),
        "it should say why, not just fail: {refusal}"
    );
    assert_eq!(
        leftovers(dir.path()),
        Vec::<String>::new(),
        "bytes of unknown provenance must not be left under the bank's name"
    );
}

#[tokio::test]
async fn a_body_wildly_larger_than_the_table_says_is_refused_before_it_is_read() {
    let server = MockServer::start().await;
    // The digest is the real check; this one exists so that a URL which has quietly become a login
    // page fails in a second rather than in an hour.
    //
    // **A genuinely larger body, not a lying header.** The first attempt set `content-length` to
    // 999999999 over 64 bytes, and hyper refused to *serve* it — a response whose declared length
    // contradicts its body is not something a server can send, so the test was checking a case that
    // cannot arise. `SIZE_SLACK` is 2 and the row states 128,788 bytes, so anything past 257,576
    // trips the gate; 300,000 is over the line and still nothing to allocate.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0_u8; 300_000]))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().expect("temp dir");
    let http = client();
    let job = Job::new("starting");

    let refusal = fetch_from(&http, &server.uri(), dir.path(), &job)
        .await
        .expect_err("a wildly bigger body must be refused");
    assert!(
        refusal.contains("refusing before"),
        "it should say it stopped early: {refusal}"
    );
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
}

/// Runs `bank::fetch` against a row rewritten to point at the test's own server.
///
/// `CatalogBank` is `&'static` because the table is compiled in, so this leaks one row rather than
/// changing that signature for a test's sake — a handful of bytes, once per case.
async fn fetch_from(
    http: &reqwest::Client,
    base: &str,
    data_dir: &std::path::Path,
    job: &Job,
) -> Result<std::path::PathBuf, String> {
    let mut row = gxscc().clone();
    row.url = Some(Box::leak(format!("{base}/bank.sf2").into_boxed_str()));
    bank::fetch(http, Box::leak(Box::new(row)), data_dir, job).await
}

/// Whatever is in the banks folder, hidden files included.
fn leftovers(data_dir: &std::path::Path) -> Vec<String> {
    let dir = bank::banks_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
