//! Pexels.
//!
//! Quota is 200 requests an hour and 20,000 a month, so slots are eighteen seconds apart — an order
//! of magnitude tighter than Pixabay, and the reason a Pexels group in the config should carry a few
//! good terms rather than forty speculative ones.
//!
//! The key goes in an `Authorization` header rather than the query string. Pexels' API guidelines ask
//! for **visible photographer attribution**, which is why `ATTRIBUTION.md` is generated for every
//! pack rather than only when somebody remembers to ask.
//!
//! **Those same guidelines forbid this pack being handed on**: *"You may not copy or replicate core
//! functionality of Pexels (including making Pexels content available as a wallpaper app)"*, and the
//! license adds *"don't redistribute … on other stock photo or wallpaper platforms"*. Their
//! exception — a platform serving a different purpose may let a user pick a background *through the
//! API* — is why the machine drawing a photograph behind lyrics was never in question, and does not
//! reach a frozen bundle of bytes shipped with the product.

use serde::Deserialize;

use crate::config::ProviderKind;
use crate::error::{Error, Result};
use crate::providers::{Keys, Provider, RemoteImage, Throttle, get_with_retry, urlencode};

/// The API's maximum page size.
const PER_PAGE: u32 = 80;

/// A Pexels client.
pub struct Pexels {
    client: reqwest::Client,
    key: String,
    base: String,
    throttle: Throttle,
}

impl Pexels {
    /// A client against the caller's keys.
    ///
    /// **The key arrives in [`Keys`] rather than out of `std::env::var` here**, which is right for
    /// a command line and unreachable from a program whose user types the key into a form.
    /// `main.rs` passes `Keys::from_env()`, so the environment contract is unchanged.
    pub fn new(client: reqwest::Client, keys: &Keys) -> Result<Self> {
        let key = keys.require(ProviderKind::Pexels)?;
        Ok(Self::with_key(
            client,
            key,
            "https://api.pexels.com/v1/search".to_owned(),
        ))
    }

    /// A client against an explicit key and base URL, which is what the tests drive.
    pub fn with_key(client: reqwest::Client, key: String, base: String) -> Self {
        Self {
            client,
            key,
            base,
            throttle: Throttle::new(200, std::time::Duration::from_secs(3600)),
        }
    }

    /// The query string for one page, which is also the cache key.
    pub fn request(&self, term: &str, page: u32) -> String {
        format!(
            "query={}&orientation=landscape&per_page={PER_PAGE}&page={page}",
            urlencode(term)
        )
    }

    fn url(&self, term: &str, page: u32) -> String {
        format!("{}?{}", self.base, self.request(term, page))
    }

    /// Turns a response body into images.
    pub fn parse(body: &str, term: &str) -> Result<Vec<RemoteImage>> {
        let response: Response = serde_json::from_str(body)
            .map_err(|error| Error::provider("pexels", format!("unreadable JSON: {error}")))?;
        Ok(response
            .photos
            .into_iter()
            .map(|photo| RemoteImage {
                provider: ProviderKind::Pexels,
                id: photo.id.to_string(),
                // `original` is the untouched upload. The sized variants are cropped to Pexels'
                // own aspect ratios, which would crop twice.
                download_url: photo.src.original,
                page_url: photo.url,
                author: photo.photographer,
                author_url: Some(photo.photographer_url),
                width: photo.width,
                height: photo.height,
                // Pexels does not return tags on search, and inventing them from the term would put
                // a guess in the manifest.
                tags: Vec::new(),
                query: term.to_owned(),
                // `None` means "this source has one license, ask it", which is true here. See the
                // same note in `pixabay.rs`.
                license: None,
                title: None,
                attribution: None,
            })
            .collect())
    }
}

impl Provider for Pexels {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Pexels
    }

    async fn search(&self, term: &str, page: u32) -> Result<Vec<RemoteImage>> {
        self.throttle.claim().await;
        let headers = [("Authorization", self.key.clone())];
        let body = get_with_retry(
            &self.client,
            ProviderKind::Pexels,
            &self.url(term, page),
            &headers,
        )
        .await?;
        Self::parse(&body, term)
    }
}

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    photos: Vec<Photo>,
}

#[derive(Debug, Deserialize)]
struct Photo {
    id: u64,
    width: u32,
    height: u32,
    url: String,
    photographer: String,
    photographer_url: String,
    src: Src,
}

#[derive(Debug, Deserialize)]
struct Src {
    original: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{
      "page": 1,
      "photos": [
        {
          "id": 3573351,
          "width": 5472,
          "height": 3648,
          "url": "https://www.pexels.com/photo/calm-ocean-3573351/",
          "photographer": "A Photographer",
          "photographer_url": "https://www.pexels.com/@aphotographer",
          "src": {
            "original": "https://images.pexels.com/photos/3573351/pexels-photo.jpeg",
            "large2x": "https://images.pexels.com/photos/3573351/pexels-photo.jpeg?w=1880"
          }
        }
      ]
    }"#;

    #[test]
    fn a_photo_becomes_a_remote_image_and_keeps_its_photographer() {
        let images = Pexels::parse(BODY, "calm ocean horizon").expect("parse");
        assert_eq!(images.len(), 1);
        let image = &images[0];
        assert_eq!(image.id, "3573351");
        assert_eq!(image.author, "A Photographer");
        assert_eq!(
            image.author_url.as_deref(),
            Some("https://www.pexels.com/@aphotographer"),
            "Pexels asks for visible attribution, so the link has to survive"
        );
        assert_eq!(
            image.download_url, "https://images.pexels.com/photos/3573351/pexels-photo.jpeg",
            "the original, not a variant Pexels has already cropped"
        );
        assert!(
            image.tags.is_empty(),
            "search returns none, so none are invented"
        );
    }

    #[test]
    fn the_request_asks_for_landscape_and_carries_no_key() {
        let provider = Pexels::with_key(
            crate::providers::client().expect("client"),
            "SECRET".to_owned(),
            "https://pexels.test/v1/search".to_owned(),
        );
        let request = provider.request("alpine valley clouds", 3);
        assert!(request.contains("query=alpine+valley+clouds"), "{request}");
        assert!(request.contains("orientation=landscape"), "{request}");
        assert!(request.contains("page=3"), "{request}");
        assert!(
            !request.contains("SECRET"),
            "the key rides in a header: {request}"
        );
        assert!(!provider.url("x", 1).contains("SECRET"));
    }

    #[test]
    fn nonsense_is_an_error_that_names_the_provider() {
        let error = Pexels::parse("nope", "ocean").expect_err("must fail");
        assert!(error.to_string().starts_with("pexels:"), "{error}");
    }
}
