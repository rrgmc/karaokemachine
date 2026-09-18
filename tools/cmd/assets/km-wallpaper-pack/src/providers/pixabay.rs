//! Pixabay.
//!
//! Quota is around 100 requests a minute per key. The search is asked for horizontal photographs
//! above the configured width, with safesearch on and 200 hits a page — the largest the API allows,
//! which is what keeps a forty-term config inside the quota.
//!
//! Pixabay's terms require that images be **hosted rather than hotlinked**, which is what this tool
//! does by construction: the pack ships the bytes.
//!
//! **The same terms document forbids passing that pack on**, and the juxtaposition is the point —
//! this comment used to stop at the reassuring half. The Content License bars distributing content
//! "on a Standalone basis", and the Terms of Service name the form: *"as a print, **wallpaper**,
//! poster or on merchandise"*. So a pack built from here is for the machine that built it. Nothing
//! about fetching changes; what changes is that `manifest.json` records the license per image, so a
//! pack can say which kind it is.

use serde::Deserialize;

use crate::config::ProviderKind;
use crate::error::{Error, Result};
use crate::providers::{Keys, Provider, RemoteImage, Throttle, get_with_retry, urlencode};

/// The API's own maximum, and the reason a run stays inside the quota.
const PER_PAGE: u32 = 200;

/// A Pixabay client.
pub struct Pixabay {
    client: reqwest::Client,
    key: String,
    base: String,
    min_width: u32,
    throttle: Throttle,
}

impl Pixabay {
    /// A client against the caller's keys.
    ///
    /// **The key arrives in [`Keys`] rather than out of `std::env::var` here**, which is right for
    /// a command line and unreachable from a program whose user types the key into a form.
    /// `main.rs` passes `Keys::from_env()`, so the environment contract is unchanged.
    pub fn new(client: reqwest::Client, keys: &Keys, min_width: u32) -> Result<Self> {
        let key = keys.require(ProviderKind::Pixabay)?;
        Ok(Self::with_key(
            client,
            key,
            "https://pixabay.com/api/".to_owned(),
            min_width,
        ))
    }

    /// A client against an explicit key and base URL, which is what the tests drive.
    pub fn with_key(client: reqwest::Client, key: String, base: String, min_width: u32) -> Self {
        Self {
            client,
            key,
            base,
            min_width,
            throttle: Throttle::new(100, std::time::Duration::from_secs(60)),
        }
    }

    /// The query string for one page, which is also the cache key.
    ///
    /// The key is deliberately **not** part of it: it is a secret, it would land in a cache path, and
    /// two people with different keys are asking the same question.
    pub fn request(&self, term: &str, page: u32) -> String {
        format!(
            "q={}&image_type=photo&orientation=horizontal&min_width={}&safesearch=true&per_page={PER_PAGE}&page={page}",
            urlencode(term),
            self.min_width,
        )
    }

    fn url(&self, term: &str, page: u32) -> String {
        format!(
            "{}?key={}&{}",
            self.base,
            self.key,
            self.request(term, page)
        )
    }

    /// Turns a response body into images.
    pub fn parse(body: &str, term: &str) -> Result<Vec<RemoteImage>> {
        let response: Response = serde_json::from_str(body)
            .map_err(|error| Error::provider("pixabay", format!("unreadable JSON: {error}")))?;
        Ok(response
            .hits
            .into_iter()
            .map(|hit| RemoteImage {
                provider: ProviderKind::Pixabay,
                id: hit.id.to_string(),
                download_url: hit.large_image_url,
                page_url: hit.page_url,
                author_url: Some(format!("https://pixabay.com/users/{}/", hit.user)),
                author: hit.user,
                width: hit.image_width,
                height: hit.image_height,
                tags: hit
                    .tags
                    .split(',')
                    .map(|tag| tag.trim().to_owned())
                    .filter(|tag| !tag.is_empty())
                    .collect(),
                query: term.to_owned(),
                // Left `None` rather than stamped with Pixabay's terms, deliberately: `None` means
                // "this source has one license, ask it", which is true here and keeps every record
                // this provider has ever written -- cached or new -- reading the same way.
                license: None,
                title: None,
                attribution: None,
            })
            .collect())
    }
}

impl Provider for Pixabay {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Pixabay
    }

    async fn search(&self, term: &str, page: u32) -> Result<Vec<RemoteImage>> {
        self.throttle.claim().await;
        let body = get_with_retry(
            &self.client,
            ProviderKind::Pixabay,
            &self.url(term, page),
            &[],
        )
        .await?;
        Self::parse(&body, term)
    }
}

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    hits: Vec<Hit>,
}

/// One hit, as Pixabay spells it.
///
/// `camelCase` covers most of the fields, but not the two that end in an acronym: Pixabay sends
/// `pageURL` and `largeImageURL`, which camel-casing turns into `pageUrl` and `largeImageUrl` — and
/// serde then reports a missing field for a key that is right there in the response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hit {
    id: u64,
    #[serde(rename = "pageURL")]
    page_url: String,
    tags: String,
    #[serde(rename = "largeImageURL")]
    large_image_url: String,
    image_width: u32,
    image_height: u32,
    user: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{
      "total": 2,
      "hits": [
        {
          "id": 1234567,
          "pageURL": "https://pixabay.com/photos/lake-dusk-1234567/",
          "tags": "mountain, lake, dusk",
          "largeImageURL": "https://pixabay.com/get/lake_1280.jpg",
          "imageWidth": 5184,
          "imageHeight": 3456,
          "user": "APhotographer"
        }
      ]
    }"#;

    #[test]
    fn a_hit_becomes_a_remote_image_with_its_attribution_intact() {
        let images = Pixabay::parse(BODY, "mountain lake dusk").expect("parse");
        assert_eq!(images.len(), 1);
        let image = &images[0];
        assert_eq!(image.id, "1234567");
        assert_eq!(image.author, "APhotographer");
        assert_eq!(
            image.page_url, "https://pixabay.com/photos/lake-dusk-1234567/",
            "the source page is what ATTRIBUTION.md links to"
        );
        assert_eq!(image.tags, vec!["mountain", "lake", "dusk"]);
        assert_eq!(image.query, "mountain lake dusk", "which term found it");
        assert_eq!(image.width, 5184);
    }

    #[test]
    fn an_empty_result_is_not_an_error() {
        let images = Pixabay::parse(r#"{"total":0,"hits":[]}"#, "nothing at all").expect("parse");
        assert!(images.is_empty());
    }

    #[test]
    fn nonsense_is_an_error_that_names_the_provider() {
        let error = Pixabay::parse("<html>rate limited</html>", "lake").expect_err("must fail");
        assert!(error.to_string().starts_with("pixabay:"), "{error}");
    }

    #[test]
    fn the_request_carries_the_filters_and_never_the_key() {
        let provider = Pixabay::with_key(
            crate::providers::client().expect("client"),
            "SECRET".to_owned(),
            "https://pixabay.test/api/".to_owned(),
            2400,
        );
        let request = provider.request("coastal cliffs", 2);
        assert!(request.contains("q=coastal+cliffs"), "{request}");
        assert!(request.contains("min_width=2400"), "{request}");
        assert!(request.contains("orientation=horizontal"), "{request}");
        assert!(request.contains("safesearch=true"), "{request}");
        assert!(request.contains("page=2"), "{request}");
        assert!(
            !request.contains("SECRET"),
            "the cache key must not carry a secret: {request}"
        );
        // The URL that is actually fetched does carry it.
        assert!(provider.url("coastal cliffs", 2).contains("key=SECRET"));
    }

    #[test]
    fn a_term_with_punctuation_cannot_forge_a_parameter() {
        let provider = Pixabay::with_key(
            crate::providers::client().expect("client"),
            "k".to_owned(),
            "https://x.test/".to_owned(),
            100,
        );
        let request = provider.request("lake&per_page=1", 1);
        assert!(request.contains("q=lake%26per_page%3D1"), "{request}");
    }
}
