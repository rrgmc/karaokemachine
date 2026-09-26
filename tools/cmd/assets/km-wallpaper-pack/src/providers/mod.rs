//! Talking to the stock APIs, and the one shape their answers are turned into.
//!
//! Each provider has its own quota, its own field names and its own idea of what "large" means, and
//! none of that is allowed past [`RemoteImage`]. Everything downstream — cache, metrics, selection,
//! manifest — works on that struct alone.
//!
//! **Adding a provider is not one file, and the license is why.** A third source is an
//! *aggregator*, whose images arrive under many licenses, and a `ProviderKind::license()` returning
//! one `&'static str` per site cannot describe one. A type that cannot represent the truth is how a
//! question stops being asked — nobody looks up what a pack may be used for while the type says
//! there is nothing to look up. So the license travels per image, in `manifest::Entry`.
//!
//! **Throttling is hand-rolled** rather than taken from a rate-limiting crate. What the quotas need
//! is one sentence — no two requests to the same provider closer together than *interval* — and a
//! crate for that would be a dependency, a vocabulary and a set of behaviors to learn for something
//! [`Throttle`] says in fifteen lines. The same reasoning the API tests already use for writing HTTP
//! by hand.
//!
//! **Retries honor `Retry-After` and give up.** A 429 or a 5xx is retried with exponential backoff
//! and jitter, five attempts, because a quota that has just been exhausted comes back. Any other 4xx
//! is fatal *for that request only*: a single search term that a provider dislikes must not cost the
//! run the other forty.

pub mod openverse;
pub mod pexels;
pub mod pixabay;

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::config::ProviderKind;
use crate::error::{Error, Result};
use crate::license::License;

/// One image as every later stage sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteImage {
    /// Which provider it came from.
    pub provider: ProviderKind,
    /// The provider's own id.
    pub id: String,
    /// The largest variant offered.
    pub download_url: String,
    /// The human-facing page, for attribution.
    pub page_url: String,
    /// Who took it.
    pub author: String,
    /// Their page, when the provider gives one.
    pub author_url: Option<String>,
    /// Width as the provider reports it.
    pub width: u32,
    /// Height as the provider reports it.
    pub height: u32,
    /// The provider's tags, kept for debugging a query that returns the wrong sort of picture.
    pub tags: Vec<String>,
    /// The search term that surfaced it. Selection uses this to stop one term dominating the pack.
    pub query: String,
    /// The license this image carries, when its source stated one per image.
    ///
    /// `None` means "ask the provider", not "unknown" — see [`RemoteImage::license`], which is the
    /// only thing that should read this field.
    ///
    /// **`#[serde(default)]` here is load-bearing rather than tidy.** This struct is serialized in
    /// three places — `index.jsonl`, the response cache (which stores these normalized records rather
    /// than a provider's own body), and `analysis.json` — and [`crate::cache::Cache`] *warns and
    /// skips* a line it cannot parse. A required field would turn a corpus of tens of thousands of
    /// cached records into an empty index, with nothing but a warning per line to say so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    /// The source's own title, when it has one. Aggregators do; the two stock APIs do not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A ready-made attribution sentence, when the source composes one.
    ///
    /// Recorded, **not rendered**. `ATTRIBUTION.md` is grouped, sorted and byte-stable by design, and
    /// dropping a foreign sentence into each line would break that consistency across providers. The
    /// credits file is *generated*; the manifest is a *record*, and this is where the record keeps
    /// the source's exact wording for anybody who needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

impl RemoteImage {
    /// The license this image carries.
    ///
    /// **The back-fill is exact rather than a guess.** A record cached before licenses were per image
    /// is, by construction, Pixabay's or Pexels' — there was no third provider when it was written —
    /// so the provider's own terms are both the right answer and the only one available.
    ///
    /// An aggregator that somehow returned no license falls to [`License::unstated`], which is not
    /// redistributable. Failing closed is the point: an image whose license nothing recorded is
    /// exactly the thing that must not ship.
    pub fn license(&self) -> License {
        self.license
            .clone()
            .or_else(|| self.provider.whole_site_license())
            .unwrap_or_else(License::unstated)
    }
    /// Megapixels, for the size component of the score.
    pub fn megapixels(&self) -> f32 {
        (self.width as f32 * self.height as f32) / 1_000_000.0
    }

    /// The extension to cache the download under, from the URL's own tail.
    pub fn extension(&self) -> &str {
        let tail = self
            .download_url
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or("jpg");
        // Query strings ride along on some CDN URLs, and a file called `.jpg?auto=compress` is not
        // a file anybody wants in a cache.
        let tail = tail.split(['?', '&', '#']).next().unwrap_or("jpg");
        match tail.to_ascii_lowercase().as_str() {
            "png" => "png",
            "webp" => "webp",
            "jpeg" => "jpeg",
            _ => "jpg",
        }
    }
}

/// Spaces requests so a provider's quota is respected.
///
/// One instant, guarded, moved forward by `interval` on each claim. Bursting is deliberately not
/// allowed: a quota of a hundred a minute spent in the first two seconds is how a key gets a 429 for
/// the following fifty-eight, and this tool has all day.
#[derive(Debug)]
pub struct Throttle {
    interval: Duration,
    next: Mutex<Option<Instant>>,
}

impl Throttle {
    /// A throttle allowing `requests` per `window`.
    pub fn new(requests: u32, window: Duration) -> Self {
        let requests = requests.max(1);
        Self {
            interval: window / requests,
            next: Mutex::new(None),
        }
    }

    /// Waits until the next request may be made, and claims that slot.
    pub async fn claim(&self) {
        let wait_until = {
            let mut next = self.next.lock().await;
            let now = Instant::now();
            let at = match *next {
                Some(at) if at > now => at,
                _ => now,
            };
            *next = Some(at + self.interval);
            at
        };
        tokio::time::sleep_until(wait_until).await;
    }
}

/// How a provider is asked for pictures.
///
/// `async fn` in a trait rather than `async_trait`: there is no `dyn Provider` anywhere here — the
/// callers hold a concrete provider or an enum over them — so the dependency would buy nothing.
pub trait Provider {
    /// Which provider this is.
    fn kind(&self) -> ProviderKind;

    /// One page of results for one term.
    fn search(
        &self,
        term: &str,
        page: u32,
    ) -> impl std::future::Future<Output = Result<Vec<RemoteImage>>> + Send;
}

/// The credentials a run searches with, however the caller came by them.
///
/// **This exists because a key cannot always come from the environment, and until it did the
/// providers assumed it always could.** Each `Provider::new` read `std::env::var` itself, which is
/// exactly right for a command line and unreachable from a program whose user types the key into a
/// form: `std::env::set_var` is `unsafe` in edition 2024, and this workspace denies `unsafe`. So
/// there was no way to supply one at all — not a hard way, none — and the `with_key` and
/// `with_token` constructors the tests already used were the shape of the answer.
///
/// The environment contract is unchanged and is now one function: [`Keys::from_env`], which
/// `main.rs` calls. What was the only path is the default path.
///
/// **Openverse is optional and the other two are not**, which is the asymmetry every part of this
/// crate has to keep. A missing Pixabay or Pexels key is [`Error::MissingKey`]; a missing Openverse
/// token means anonymous access, inside three caps its own constructor explains.
#[derive(Debug, Clone, Default)]
pub struct Keys {
    /// `PIXABAY_API_KEY`.
    pub pixabay: Option<String>,
    /// `PEXELS_API_KEY`.
    pub pexels: Option<String>,
    /// `OPENVERSE_API_TOKEN`, and the only one a run can do without.
    pub openverse: Option<String>,
}

impl Keys {
    /// The keys as the environment has them — the contract `config.example.toml` documents.
    ///
    /// An unset or empty variable is `None`. **Empty counts as absent** deliberately: `PEXELS_API_KEY=`
    /// in a shell profile is somebody clearing it, and treating that as a key would send an empty
    /// credential and get a 401 that names nothing.
    pub fn from_env() -> Self {
        let read = |kind: ProviderKind| {
            std::env::var(kind.key_var())
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        Self {
            pixabay: read(ProviderKind::Pixabay),
            pexels: read(ProviderKind::Pexels),
            openverse: read(ProviderKind::Openverse),
        }
    }

    /// The key for one provider, if there is one.
    pub fn get(&self, kind: ProviderKind) -> Option<&str> {
        match kind {
            ProviderKind::Pixabay => self.pixabay.as_deref(),
            ProviderKind::Pexels => self.pexels.as_deref(),
            ProviderKind::Openverse => self.openverse.as_deref(),
        }
    }

    /// The key for a provider that must have one, or the error naming where to get it.
    ///
    /// The message is the same one an unset variable produced before this type existed, because it
    /// is still the right thing to say at a command line — and a caller with a form can show the
    /// variable name or not, as it likes.
    pub fn require(&self, kind: ProviderKind) -> Result<String> {
        self.get(kind).map(str::to_owned).ok_or(Error::MissingKey {
            var: kind.key_var(),
            provider: kind.key_page(),
        })
    }
}

/// Installs rustls' crypto provider, once.
///
/// reqwest is built with `rustls-no-provider`, so until something installs one,
/// `Client::builder().build()` fails — and `Client::new()` *panics*. [`client`] calls this, which is
/// why no caller inside this crate can forget.
///
/// **It is public for the caller that builds its own client**: `km-admin` shares this crate's
/// `reqwest` and therefore its missing provider, but wants a different user-agent and a timeout long
/// enough for a gigabyte SoundFont, so it cannot simply use [`client`]. Exposing the install is what
/// stops the second program either duplicating the `Once` or discovering the panic at run time.
pub fn install_crypto_provider() {
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        // Fails only if a provider is already installed, which is the state this is reaching for.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// The HTTP client every provider shares.
///
/// It installs rustls' crypto provider first, and that ordering is the whole reason this function
/// exists — see [`install_crypto_provider`]. Doing it here means no caller can forget, including a
/// test that only wanted to check a query string.
pub fn client() -> Result<reqwest::Client> {
    install_crypto_provider();

    reqwest::Client::builder()
        .user_agent(format!("km-wallpaper-pack/{}", crate::VERSION))
        .build()
        .map_err(|error| Error::provider("http", error.to_string()))
}

/// Percent-encoding for a search term.
///
/// Terms are English words and spaces; this exists so that "coastal cliffs" is one query rather than
/// two, and so that a stray `&` in a config cannot forge a parameter.
pub fn urlencode(text: &str) -> String {
    form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// A GET that retries the failures worth retrying.
///
/// Returns the body as text. `Retry-After` replaces the backoff schedule for the next attempt when
/// the server sends one: it is the server saying exactly how long it needs, and guessing shorter is
/// how a temporary 429 becomes a permanent one. Both of its forms count, a number of seconds and an
/// HTTP date.
pub async fn get_with_retry(
    client: &reqwest::Client,
    provider: ProviderKind,
    url: &str,
    headers: &[(&str, String)],
) -> Result<String> {
    /// Attempts, including the first.
    const ATTEMPTS: u32 = 5;
    /// First backoff, doubled each time.
    const BASE: Duration = Duration::from_millis(500);

    let name = provider.as_str();
    let mut last = String::from("no attempt was made");
    let mut asked: Option<Duration> = None;

    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            // Jitter from the attempt number and the URL rather than a random source, so a failing
            // run is reproducible. Two providers backing off in lockstep is not a problem worth
            // randomness here: the throttle already spaces them.
            let jitter = Duration::from_millis(u64::from(url.len() as u32 % 250));
            let backoff = BASE * 2u32.pow(attempt - 1) + jitter;
            tokio::time::sleep(asked.take().unwrap_or(backoff)).await;
        }

        let mut request = client.get(url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                // Worth logging at every request: it is the only warning before a quota runs out.
                if let Some(remaining) = response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .and_then(|value| value.to_str().ok())
                {
                    tracing::debug!(provider = name, remaining, "quota");
                }

                if status.is_success() {
                    return response.text().await.map_err(|error| {
                        Error::provider(name, format!("unreadable body: {error}"))
                    });
                }

                last = format!("HTTP {status}");
                let worth_retrying = status.as_u16() == 429 || status.is_server_error();
                if !worth_retrying {
                    return Err(Error::provider(name, last));
                }
                asked = response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| retry_after(value, SystemTime::now()));
                if let Some(wait) = asked {
                    tracing::warn!(provider = name, seconds = wait.as_secs(), "asked to wait");
                }
            }
            Err(error) => last = error.to_string(),
        }
    }

    Err(Error::provider(
        name,
        format!("gave up after {ATTEMPTS} attempts: {last}"),
    ))
}

/// How long a `Retry-After` value asks for, capped at two minutes.
///
/// The header is either a number of seconds or an HTTP date. A date already past asks for no wait.
/// The cap keeps one server's answer from stalling a whole run.
fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    const CAP: Duration = Duration::from_secs(120);
    let value = value.trim();
    let wait = match value.parse::<u64>() {
        Ok(seconds) => Duration::from_secs(seconds),
        Err(_) => httpdate::parse_http_date(value)
            .ok()?
            .duration_since(now)
            .unwrap_or(Duration::ZERO),
    };
    Some(wait.min(CAP))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_reads_seconds_and_dates_and_caps_both() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        assert_eq!(retry_after("7", now), Some(Duration::from_secs(7)));
        assert_eq!(retry_after("9999", now), Some(Duration::from_secs(120)));
        let later = httpdate::fmt_http_date(now + Duration::from_secs(30));
        assert_eq!(retry_after(&later, now), Some(Duration::from_secs(30)));
        let earlier = httpdate::fmt_http_date(now - Duration::from_secs(30));
        assert_eq!(retry_after(&earlier, now), Some(Duration::ZERO));
        assert_eq!(retry_after("soon", now), None);
    }

    #[test]
    fn a_missing_key_names_the_variable_and_where_to_get_one() {
        // The message an empty `Keys` produces is the one an unset variable produced before this
        // type existed, because it is still the right thing to say at a command line.
        let keys = Keys::default();
        let error = keys
            .require(ProviderKind::Pixabay)
            .expect_err("pixabay needs a key");
        let said = error.to_string();
        assert!(said.contains("PIXABAY_API_KEY"), "{said}");
        assert!(said.contains("pixabay.com"), "{said}");
    }

    #[test]
    fn openverse_is_the_one_that_runs_without_a_key() {
        // The asymmetry every part of this crate has to keep, and the reason Openverse is the
        // default a person can use having signed up to nothing.
        assert!(ProviderKind::Pixabay.needs_key());
        assert!(ProviderKind::Pexels.needs_key());
        assert!(!ProviderKind::Openverse.needs_key());

        let keys = Keys::default();
        assert!(keys.get(ProviderKind::Openverse).is_none());
        assert!(keys.require(ProviderKind::Pexels).is_err());
    }

    #[test]
    fn each_provider_reads_its_own_slot() {
        let keys = Keys {
            pixabay: Some("pix".to_owned()),
            pexels: None,
            openverse: Some("open".to_owned()),
        };
        assert_eq!(keys.get(ProviderKind::Pixabay), Some("pix"));
        assert_eq!(keys.get(ProviderKind::Openverse), Some("open"));
        assert_eq!(keys.get(ProviderKind::Pexels), None);
        assert_eq!(
            keys.require(ProviderKind::Pixabay)
                .expect("pixabay has one"),
            "pix"
        );
    }

    #[test]
    fn every_provider_names_a_distinct_variable_and_page() {
        // A copy-paste in either table would silently point two providers at one key, which is the
        // kind of thing a match arm makes easy and a test makes impossible.
        let all = [
            ProviderKind::Pixabay,
            ProviderKind::Pexels,
            ProviderKind::Openverse,
        ];
        let mut vars: Vec<&str> = all.iter().map(|kind| kind.key_var()).collect();
        vars.sort_unstable();
        vars.dedup();
        assert_eq!(vars.len(), all.len(), "two providers share a variable");

        let mut pages: Vec<&str> = all.iter().map(|kind| kind.key_page()).collect();
        pages.sort_unstable();
        pages.dedup();
        assert_eq!(pages.len(), all.len(), "two providers share a key page");
    }

    fn image(url: &str) -> RemoteImage {
        RemoteImage {
            provider: ProviderKind::Pixabay,
            id: "1".to_owned(),
            download_url: url.to_owned(),
            page_url: String::new(),
            author: String::new(),
            author_url: None,
            width: 4000,
            height: 3000,
            tags: Vec::new(),
            query: String::new(),
            license: None,
            title: None,
            attribution: None,
        }
    }

    /// **The migration test, and the one that matters most in this file.**
    ///
    /// `RemoteImage` is serialized in three places — `index.jsonl`, the response cache, and
    /// `analysis.json` — and `Cache::read_jsonl` warns and *skips* a line it cannot parse. Had the
    /// license been a required field, a corpus of tens of thousands of cached records would have
    /// come back as an empty index with nothing but a warning per line, and `analyze` would have
    /// reported a corpus of nothing without anything looking broken.
    ///
    /// The line below is a real one, trimmed: no `license` key, because none existed when it was
    /// written.
    #[test]
    fn a_record_cached_before_licenses_were_per_image_reads_back_under_its_providers_terms() {
        let line = r#"{"provider":"pixabay","id":"8363426",
            "download_url":"https://pixabay.com/get/g41f.jpg",
            "page_url":"https://pixabay.com/photos/lake-8363426/",
            "author":"Somebody","author_url":null,"width":5151,"height":3434,
            "tags":["lake"],"query":"mountain lake dusk"}"#;

        let image: RemoteImage = serde_json::from_str(line).expect("an old record still reads");
        assert_eq!(image.license, None, "the field is absent, not invented");

        let license = image.license();
        assert_eq!(license.code, "pixabay");
        assert_eq!(license.name, "Pixabay Content License");
        assert!(
            !license.redistribution().is_granted(),
            "a Pixabay pack may not be passed on, whenever it was cached"
        );
    }

    /// A record that states its own license keeps it, rather than being overwritten by the site's.
    #[test]
    fn a_record_that_states_its_license_keeps_it() {
        let mut image = image("https://x.test/a.jpg");
        image.provider = ProviderKind::Pixabay;
        image.license = Some(crate::license::License::creative_commons(
            "cc0",
            "1.0",
            Some("https://creativecommons.org/publicdomain/zero/1.0/".to_owned()),
        ));
        let license = image.license();
        assert_eq!(license.code, "cc0");
        assert!(license.redistribution().is_granted());
    }

    #[test]
    fn the_cache_extension_comes_from_the_url_and_ignores_its_query_string() {
        assert_eq!(image("https://x.test/a/b.jpg").extension(), "jpg");
        assert_eq!(image("https://x.test/a/b.PNG").extension(), "png");
        assert_eq!(
            image("https://x.test/a/b.webp?auto=compress").extension(),
            "webp"
        );
        // Pexels serves URLs with no extension at all.
        assert_eq!(image("https://x.test/photo/12345").extension(), "jpg");
    }

    #[test]
    fn megapixels_are_what_the_score_ranks_on() {
        assert!((image("x.jpg").megapixels() - 12.0).abs() < 0.001);
    }

    #[tokio::test(start_paused = true)]
    async fn the_throttle_spaces_requests_rather_than_letting_them_burst() {
        // 100 a minute is Pixabay's quota, so slots are 600ms apart.
        let throttle = Throttle::new(100, Duration::from_secs(60));
        let start = Instant::now();
        for _ in 0..4 {
            throttle.claim().await;
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1_800),
            "four claims cost three intervals, got {elapsed:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_claim_does_not_wait() {
        let throttle = Throttle::new(1, Duration::from_secs(3600));
        let start = Instant::now();
        throttle.claim().await;
        assert!(
            start.elapsed() < Duration::from_millis(10),
            "nothing to wait for yet"
        );
    }
}
