//! The providers against a recorded server.
//!
//! `wiremock` rather than the real APIs: these tests have to run in CI, with no key and no network,
//! and the paths worth testing are the ones a real run only hits occasionally — a quota refusal, a
//! gateway error, a body that is not JSON. Those are exactly the paths that are wrong when nobody has
//! exercised them.

use km_wallpaper_pack::config::ProviderKind;
use km_wallpaper_pack::providers::openverse::Openverse;
use km_wallpaper_pack::providers::pexels::Pexels;
use km_wallpaper_pack::providers::pixabay::Pixabay;
use km_wallpaper_pack::providers::{Provider, client};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PIXABAY_BODY: &str = r#"{
  "total": 1,
  "hits": [
    {
      "id": 1234567,
      "pageURL": "https://pixabay.com/photos/lake-1234567/",
      "tags": "mountain, lake",
      "largeImageURL": "https://pixabay.com/get/lake_1280.jpg",
      "imageWidth": 5184,
      "imageHeight": 3456,
      "user": "APhotographer"
    }
  ]
}"#;

#[tokio::test]
async fn a_search_reaches_the_provider_with_the_filters_the_config_asked_for() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/"))
        .and(query_param("key", "test-key"))
        .and(query_param("image_type", "photo"))
        .and(query_param("orientation", "horizontal"))
        .and(query_param("min_width", "2400"))
        .and(query_param("safesearch", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PIXABAY_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let provider = Pixabay::with_key(
        client().expect("client"),
        "test-key".to_owned(),
        format!("{}/api/", server.uri()),
        2400,
    );
    let images = provider
        .search("mountain lake dusk", 1)
        .await
        .expect("search");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].provider, ProviderKind::Pixabay);
    assert_eq!(images[0].author, "APhotographer");
}

#[tokio::test]
async fn a_quota_refusal_is_waited_out_rather_than_failing_the_run() {
    let server = MockServer::start().await;
    // 429 with a `Retry-After` the client must honor, then success. Anything else — giving up, or
    // hammering the endpoint — costs a key its quota for an hour.
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "1")
                .set_body_string("slow down"),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PIXABAY_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let provider = Pixabay::with_key(
        client().expect("client"),
        "k".to_owned(),
        format!("{}/api/", server.uri()),
        100,
    );
    let images = provider
        .search("lake", 1)
        .await
        .expect("the retry succeeds");
    assert_eq!(images.len(), 1);
}

#[tokio::test]
async fn a_server_error_is_retried_and_then_reported_with_the_provider_named() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;

    let provider = Pixabay::with_key(
        client().expect("client"),
        "k".to_owned(),
        format!("{}/api/", server.uri()),
        100,
    );
    let error = provider.search("lake", 1).await.expect_err("must fail");
    let message = error.to_string();
    assert!(message.starts_with("pixabay:"), "{message}");
    assert!(message.contains("gave up after"), "{message}");
    assert!(
        message.contains("502"),
        "the last status is named: {message}"
    );
    // Five attempts, not one and not forever.
    assert_eq!(server.received_requests().await.map(|r| r.len()), Some(5));
}

#[tokio::test]
async fn a_body_that_is_not_json_is_an_error_and_not_a_panic() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>maintenance</html>"))
        .mount(&server)
        .await;

    let provider = Pixabay::with_key(
        client().expect("client"),
        "k".to_owned(),
        format!("{}/api/", server.uri()),
        100,
    );
    let error = provider.search("lake", 1).await.expect_err("must fail");
    assert!(error.to_string().contains("unreadable JSON"), "{error}");
}

#[tokio::test]
async fn a_refusal_that_is_not_a_quota_is_not_retried() {
    let server = MockServer::start().await;
    // 400 means the request was wrong, and asking again five times will not make it right. It costs
    // this term, not the run.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400))
        .expect(1)
        .mount(&server)
        .await;

    let provider = Pixabay::with_key(
        client().expect("client"),
        "k".to_owned(),
        format!("{}/api/", server.uri()),
        100,
    );
    let error = provider.search("lake", 1).await.expect_err("must fail");
    assert!(error.to_string().contains("400"), "{error}");
    assert_eq!(server.received_requests().await.map(|r| r.len()), Some(1));
}

#[tokio::test]
async fn pexels_sends_its_key_as_a_header_and_asks_for_landscape() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/search"))
        .and(header("authorization", "pexels-key"))
        .and(query_param("orientation", "landscape"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"photos":[{"id":42,"width":5472,"height":3648,
                "url":"https://www.pexels.com/photo/ocean-42/",
                "photographer":"A Photographer",
                "photographer_url":"https://www.pexels.com/@a",
                "src":{"original":"https://images.pexels.com/photos/42/x.jpeg"}}]}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let provider = Pexels::with_key(
        client().expect("client"),
        "pexels-key".to_owned(),
        format!("{}/v1/search", server.uri()),
    );
    let images = provider
        .search("calm ocean horizon", 1)
        .await
        .expect("search");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].provider, ProviderKind::Pexels);
    assert_eq!(
        images[0].download_url,
        "https://images.pexels.com/photos/42/x.jpeg"
    );
}

/// Openverse sends a bearer token when it has one, and asks only for licenses it may redistribute.
///
/// Note what is *not* asserted present: `min_width`. That parameter is ours — the cache key needs it
/// because a different width gate is a different question — and the API would answer 400 if it
/// arrived, so `url()` strips it. A `query_param` matcher would pass either way, which is why
/// `the_width_gate_is_in_the_cache_key_and_not_in_the_url` in the provider's own tests carries that
/// half.
#[tokio::test]
async fn openverse_sends_a_bearer_token_and_asks_only_for_licenses_it_may_redistribute() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/images/"))
        .and(header("Authorization", "Bearer openverse-token"))
        .and(query_param("license", "cc0,pdm,by"))
        .and(query_param("category", "photograph"))
        .and(query_param("aspect_ratio", "wide"))
        .and(query_param("page_size", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"result_count":1,"results":[{
                "id":"abc","foreign_landing_url":"https://commons.wikimedia.org/wiki/File:A.jpg",
                "url":"https://upload.wikimedia.org/a.jpg","creator":"Ada",
                "license":"cc0","license_version":"1.0",
                "license_url":"https://creativecommons.org/publicdomain/zero/1.0/",
                "width":4000,"height":3000,"tags":[]}]}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let provider = Openverse::with_token(
        client().expect("client"),
        Some("openverse-token".to_owned()),
        format!("{}/v1/images/", server.uri()),
        2400,
    );
    let images = provider.search("alpine valley", 1).await.expect("search");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].provider, ProviderKind::Openverse);
    assert_eq!(images[0].license().code, "cc0");
    assert!(images[0].license().redistribution().is_granted());
}

/// Without a token the provider still works — it runs inside three caps instead of failing.
///
/// Asserting the header is *absent* is the half that matters: an empty `Bearer ` would be a 401, and
/// the failure would look like a bad token rather than a missing one.
#[tokio::test]
async fn openverse_without_a_token_sends_no_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/images/"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"result_count":0,"results":[]}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let provider = Openverse::with_token(
        client().expect("client"),
        None,
        format!("{}/v1/images/", server.uri()),
        2400,
    );
    let images = provider.search("alpine valley", 1).await.expect("search");
    assert!(images.is_empty());

    let sent = server.received_requests().await.expect("requests");
    assert!(
        sent[0].headers.get("authorization").is_none(),
        "no token means no header at all, not an empty one"
    );
}
