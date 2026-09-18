//! Openverse.
//!
//! **The one source here whose packs may be handed on**, and the reason the license had to become a
//! property of the image. Openverse is an *aggregator*, not a stock library: it indexes Wikimedia
//! Commons, Flickr, StockSnap, rawpixel and museum open collections, and its results arrive under
//! whatever license each of those carries. `ProviderKind::license()` returning one string per
//! website could not describe that, which is why it is gone.
//!
//! Filtered to `cc0,pdm,by`, which is the allow-list in [`crate::license`] and not a preference
//! expressed here.
//!
//! # `category=photograph` is not a size optimization
//!
//! It is the parameter that excludes `digitized_artwork`, and that class is exactly what wasted a
//! round of hand-picking when the shipped set was chosen: scanned album pages with the mount still
//! in frame, and archival prints with the negative number written across the corner. Every one of
//! them cleared the contrast gate — a sepia photograph of a fjord is beautifully calm behind
//! lyrics — and every one was obviously wrong on sight. One query parameter is a better filter than
//! a person looking at thirty pictures.
//!
//! # Anonymous is not merely slower — it is capped three ways
//!
//! Read off the live API and off Openverse's own `DEFAULT_THROTTLE_RATES`:
//!
//! | | Anonymous | With a token |
//! |---|---|---|
//! | Burst | 20/min | 100/min |
//! | Sustained | 200/day | 10,000/day |
//! | `page_size` | **20** | higher |
//! | Pagination depth | **240** | higher |
//!
//! The last two are the ones that bite, and neither is a speed limit. A 240-result ceiling means a
//! search cannot *see* past its first 240 hits however long you wait — so a `result_count` of 240 is
//! the cap reporting itself, not an answer about the corpus. Register at
//! `POST /v1/auth_tokens/register/`, exchange the credentials at `POST /v1/auth_tokens/token/`, and
//! put the token in `OPENVERSE_API_TOKEN`.
//!
//! # The bytes do not come from the API you searched
//!
//! `url` points at Wikimedia, Flickr or a museum, each with its own policy. Two things measured the
//! hard way while assembling the shipped set: Wikimedia refuses a bare tool name as a User-Agent,
//! and it throttles a sustained sequential run hard enough to have failed **82 of 103 downloads**
//! until they backed off and retried. `crate::providers::client` sends a real User-Agent and
//! `crate::commands::download` retries; this is the note for whoever meets it next.

use serde::Deserialize;

use crate::config::ProviderKind;
use crate::error::{Error, Result};
use crate::license::License;
use crate::providers::{Keys, Provider, RemoteImage, Throttle, get_with_retry, urlencode};

/// The most a page may hold without a token. Asking for more is a 400, not a truncation.
const PER_PAGE: u32 = 20;

/// The licenses asked for, and then checked again on the way back.
///
/// **A constant rather than a config key**, for the reason [`crate::license`] gives: this is a legal
/// position, and a knob would let it widen in a file nobody reviews. It is the same list
/// `License::redistribution` grants, spelled the way the API wants it.
const LICENSES: &str = "cc0,pdm,by";

/// An Openverse client.
pub struct Openverse {
    client: reqwest::Client,
    token: Option<String>,
    base: String,
    min_width: u32,
    throttle: Throttle,
}

impl Openverse {
    /// A client, with a token if one is in the environment.
    ///
    /// **Infallible, alone among the three providers.** Pixabay and Pexels return
    /// [`Error::MissingKey`] because they answer nothing without a key; Openverse answers perfectly
    /// well without one, just inside three caps. Refusing to run would be wrong, and running
    /// silently would be worse — so it warns once, naming what the caps are.
    ///
    /// **The key arrives in [`Keys`] rather than out of `std::env::var` here**, which is right for
    /// a command line and unreachable from a program whose user types the token into a form.
    /// `main.rs` passes `Keys::from_env()`, so the environment contract is unchanged.
    pub fn new(client: reqwest::Client, keys: &Keys, min_width: u32) -> Self {
        let token = keys.get(ProviderKind::Openverse).map(str::to_owned);
        if token.is_none() {
            tracing::warn!(
                var = ProviderKind::Openverse.key_var(),
                "no Openverse token: 20 requests a minute and 200 a day, at most 20 results a page, \
                 and no search can see past its first 240 hits. The last is a ceiling rather than a \
                 delay -- a `result_count` of 240 is the cap, not the corpus."
            );
        }
        Self::with_token(
            client,
            token,
            "https://api.openverse.org/v1/images/".to_owned(),
            min_width,
        )
    }

    /// A client with an explicit token and endpoint, for tests.
    pub fn with_token(
        client: reqwest::Client,
        token: Option<String>,
        base: String,
        min_width: u32,
    ) -> Self {
        // Both rates come from Openverse's own throttle settings, halved into an interval by
        // `Throttle`. The gap is 0.6 s with a token and 3 s without, which is the difference between
        // a config that finishes and one that does not.
        let throttle = match token {
            Some(_) => Throttle::new(100, std::time::Duration::from_secs(60)),
            None => Throttle::new(20, std::time::Duration::from_secs(60)),
        };
        Self {
            client,
            token,
            base,
            min_width,
            throttle,
        }
    }

    /// The query string for one page, which is also the cache key.
    ///
    /// **`min_width` is in here even though Openverse has no such parameter**, and that is
    /// deliberate rather than sloppy. The API offers only `size` buckets, so the width gate is
    /// applied in [`Openverse::parse`] — but `crate::cache` keys a stored response on this string,
    /// and a different `min_width` is a different question that must not be served an answer
    /// filtered under the old one.
    ///
    /// The token is not in it, for the reason `pixabay.rs` gives about its key: it is a secret, it
    /// would land in a cache path, and two people with different tokens are asking the same thing.
    pub fn request(&self, term: &str, page: u32) -> String {
        format!(
            "q={}&license={LICENSES}&license_type=commercial,modification&category=photograph\
             &aspect_ratio=wide&size=large&extension=jpg,png\
             &unstable__include_sensitive_results=false\
             &min_width={}&page_size={PER_PAGE}&page={page}",
            urlencode(term),
            self.min_width,
        )
    }

    /// The URL actually fetched: the request, minus the parameter the API does not know.
    ///
    /// `min_width` is ours and would be a 400 if sent, so it is stripped here rather than left out
    /// of [`Openverse::request`] — the cache key needs it and the API must not see it.
    fn url(&self, term: &str, page: u32) -> String {
        let request = self.request(term, page);
        let sent: Vec<&str> = request
            .split('&')
            .filter(|parameter| !parameter.starts_with("min_width="))
            .collect();
        format!("{}?{}", self.base, sent.join("&"))
    }

    /// Turns one response into records.
    ///
    /// **Two filters, and the second is the interesting one.** Results below `min_width` go, because
    /// the API cannot be asked for a width. And a result whose license is off the allow-list goes
    /// *even though the request asked for exactly that list* — a provider's answer is not trusted to
    /// match its own filter, the same reasoning that makes `commands::parse_for` overwrite `query`
    /// rather than believe a cached record about which term found it.
    ///
    /// A result with no `url` is skipped rather than erroring: Openverse indexes some records whose
    /// file is unreachable, and one of them should not cost the page.
    pub fn parse(body: &str, term: &str, min_width: u32) -> Result<Vec<RemoteImage>> {
        let response: Response = serde_json::from_str(body)
            .map_err(|error| Error::provider("openverse", format!("unreadable JSON: {error}")))?;
        Ok(response
            .results
            .into_iter()
            .filter_map(|result| {
                let download_url = result.url?;
                if result.width < min_width {
                    return None;
                }
                let license = License::creative_commons(
                    &result.license,
                    result.license_version.as_deref().unwrap_or_default(),
                    result.license_url,
                );
                if !license.redistribution().is_granted() {
                    return None;
                }
                Some(RemoteImage {
                    provider: ProviderKind::Openverse,
                    id: result.id,
                    download_url,
                    // The page on the *source*, not on Openverse: it is where the license can be
                    // checked, and it is what `ATTRIBUTION.md` links to.
                    page_url: result.foreign_landing_url,
                    author: result.creator.unwrap_or_else(|| "Unknown".to_owned()),
                    author_url: result.creator_url,
                    width: result.width,
                    height: result.height,
                    tags: result
                        .tags
                        .into_iter()
                        .filter_map(|tag| tag.name)
                        .filter(|tag| !tag.is_empty())
                        .collect(),
                    query: term.to_owned(),
                    license: Some(license),
                    title: result.title,
                    attribution: result.attribution,
                })
            })
            .collect())
    }
}

impl Provider for Openverse {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Openverse
    }

    async fn search(&self, term: &str, page: u32) -> Result<Vec<RemoteImage>> {
        self.throttle.claim().await;
        let headers = match &self.token {
            Some(token) => vec![("Authorization", format!("Bearer {token}"))],
            None => Vec::new(),
        };
        let body = get_with_retry(
            &self.client,
            ProviderKind::Openverse,
            &self.url(term, page),
            &headers,
        )
        .await?;
        Self::parse(&body, term, self.min_width)
    }
}

/// One page of results.
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    results: Vec<SearchResult>,
}

/// One image, as Openverse describes it.
///
/// Almost everything is optional, because an aggregator's records are only as complete as the source
/// it indexed. `width`/`height` are not: a result without them cannot be judged at all.
#[derive(Debug, Deserialize)]
struct SearchResult {
    id: String,
    #[serde(default)]
    title: Option<String>,
    foreign_landing_url: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    creator: Option<String>,
    #[serde(default)]
    creator_url: Option<String>,
    license: String,
    #[serde(default)]
    license_version: Option<String>,
    #[serde(default)]
    license_url: Option<String>,
    #[serde(default)]
    attribution: Option<String>,
    width: u32,
    height: u32,
    #[serde(default)]
    tags: Vec<Tag>,
}

/// A tag, which Openverse returns as an object rather than a string.
#[derive(Debug, Deserialize)]
struct Tag {
    #[serde(default)]
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real response: a CC BY photograph from Wikimedia via Openverse.
    const BODY: &str = r#"{
      "result_count": 240,
      "results": [
        {
          "id": "4e410a41-53a6-40a3-b622-ae2db53c2269",
          "title": "NZ Southern Alps at sunset",
          "foreign_landing_url": "https://commons.wikimedia.org/w/index.php?curid=135809767",
          "url": "https://upload.wikimedia.org/wikipedia/commons/a/ab/Alps.jpg",
          "creator": "Marek Slusarczyk",
          "creator_url": "https://commons.wikimedia.org/wiki/User:Tupungato",
          "license": "by",
          "license_version": "3.0",
          "license_url": "https://creativecommons.org/licenses/by/3.0/",
          "attribution": "\"NZ Southern Alps\" by Marek Slusarczyk is licensed under CC BY 3.0.",
          "provider": "wikimedia",
          "source": "wikimedia",
          "width": 2430,
          "height": 1620,
          "tags": [{"name": "mountain"}, {"name": "sunset"}]
        }
      ]
    }"#;

    fn provider(min_width: u32) -> Openverse {
        Openverse::with_token(
            crate::providers::client().expect("client"),
            Some("test-token".to_owned()),
            "https://example.test/v1/images/".to_owned(),
            min_width,
        )
    }

    #[test]
    fn a_result_becomes_a_remote_image_carrying_its_own_license() {
        let images = Openverse::parse(BODY, "alpine valley", 2400).expect("parse");
        assert_eq!(images.len(), 1);
        let image = &images[0];
        assert_eq!(image.provider, ProviderKind::Openverse);
        assert_eq!(image.id, "4e410a41-53a6-40a3-b622-ae2db53c2269");
        assert_eq!(image.author, "Marek Slusarczyk");
        assert_eq!(
            image.page_url, "https://commons.wikimedia.org/w/index.php?curid=135809767",
            "the source's page is what ATTRIBUTION.md links to, not Openverse's"
        );
        assert_eq!(image.title.as_deref(), Some("NZ Southern Alps at sunset"));
        assert!(image.attribution.is_some(), "the source's own sentence");
        assert_eq!(image.tags, vec!["mountain", "sunset"]);

        let license = image.license();
        assert_eq!(license.code, "by");
        assert_eq!(license.name, "CC BY 3.0");
        assert_eq!(
            license.url.as_deref(),
            Some("https://creativecommons.org/licenses/by/3.0/")
        );
        assert!(license.redistribution().is_granted());
    }

    /// The API has no width parameter, so this is the only gate there is.
    #[test]
    fn a_result_narrower_than_the_filter_is_dropped() {
        let images = Openverse::parse(BODY, "alpine valley", 4000).expect("parse");
        assert!(images.is_empty(), "2430 is under a 4000 floor");
    }

    /// **The filter that matters.** The request asks for `cc0,pdm,by`; this checks the answer anyway.
    /// A provider is not trusted to honor its own filter, and a hand-edited cache must not be able
    /// to introduce a license the pack may not carry.
    #[test]
    fn a_license_off_the_allow_list_is_dropped_even_though_the_request_asked_for_the_filter() {
        let body = BODY.replace(r#""license": "by""#, r#""license": "by-nc-nd""#);
        let images = Openverse::parse(&body, "alpine valley", 2400).expect("parse");
        assert!(
            images.is_empty(),
            "by-nc-nd may not be redistributed, whatever the request asked for"
        );
    }

    #[test]
    fn a_result_with_no_file_costs_only_itself() {
        let body = BODY.replace(
            r#""url": "https://upload.wikimedia.org/wikipedia/commons/a/ab/Alps.jpg","#,
            "",
        );
        let images = Openverse::parse(&body, "alpine valley", 2400).expect("parse");
        assert!(images.is_empty());
    }

    #[test]
    fn an_empty_result_is_not_an_error() {
        let images = Openverse::parse(r#"{"result_count":0,"results":[]}"#, "x", 0).expect("parse");
        assert!(images.is_empty());
    }

    #[test]
    fn nonsense_is_an_error_that_names_the_provider() {
        let error = Openverse::parse("<html>rate limited</html>", "x", 0).expect_err("error");
        assert!(error.to_string().starts_with("openverse:"), "{error}");
    }

    #[test]
    fn the_request_carries_the_filters_and_never_the_token() {
        let request = provider(2400).request("alpine valley", 2);
        assert!(request.contains("q=alpine+valley"), "{request}");
        assert!(request.contains("license=cc0,pdm,by"), "{request}");
        assert!(request.contains("category=photograph"), "{request}");
        assert!(request.contains("aspect_ratio=wide"), "{request}");
        assert!(request.contains("page=2"), "{request}");
        assert!(
            !request.contains("test-token"),
            "the cache key must not carry a secret: {request}"
        );
    }

    /// `min_width` is ours: the cache key needs it and the API would refuse it.
    #[test]
    fn the_width_gate_is_in_the_cache_key_and_not_in_the_url() {
        let provider = provider(2400);
        assert!(provider.request("lake", 1).contains("min_width=2400"));
        assert!(!provider.url("lake", 1).contains("min_width"));
    }

    /// Two different width gates are two different questions and must not share a cached answer.
    #[test]
    fn a_different_width_gate_is_a_different_cache_key() {
        assert_ne!(
            provider(2400).request("lake", 1),
            provider(3000).request("lake", 1)
        );
    }

    #[test]
    fn a_term_with_punctuation_cannot_forge_a_parameter() {
        let request = provider(2400).request("lake&page_size=1", 1);
        assert!(request.contains("q=lake%26page_size%3D1"), "{request}");
    }
}
