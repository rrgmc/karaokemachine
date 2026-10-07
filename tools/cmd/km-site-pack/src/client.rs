//! The one place a request leaves from.

use std::cell::Cell;
use std::io::Read as _;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use url::Url;

/// What this program calls itself to a server and to a `robots.txt`.
pub const AGENT: &str = concat!("km-site-pack/", env!("CARGO_PKG_VERSION"));

/// How long one request may take from its first byte to its last.
const TIMEOUT: Duration = Duration::from_secs(60);

/// How many redirects one request follows.
const REDIRECTS: u32 = 5;

/// One answer from a server.
#[derive(Debug)]
pub struct Answer {
    /// The HTTP status.
    pub status: u16,
    /// The `Content-Type` header in lowercase, when the server sent one.
    pub content_type: Option<String>,
    /// The body, up to the limit asked for.
    pub body: Vec<u8>,
    /// Whether the body was longer than the limit, in which case `body` is cut short.
    pub too_large: bool,
    /// Whether the server asked for a browser to prove itself before it would answer.
    pub challenged: bool,
}

impl Answer {
    /// Whether the server sent what was asked for.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as text, with any byte that is not UTF-8 replaced.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A client that waits between requests and says who it is.
pub struct Client {
    agent: ureq::Agent,
    delay: Cell<Duration>,
    last: Cell<Option<Instant>>,
}

/// The longest wait a site's `robots.txt` can ask of this program between two requests.
///
/// A longer one would turn a few hundred files into a day's run that looks stopped.
pub const MAX_ASKED_DELAY: Duration = Duration::from_secs(30);

impl Client {
    /// A client that leaves `delay` between the end of one request and the start of the next.
    #[must_use]
    pub fn new(delay: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .user_agent(AGENT)
            .timeout_global(Some(TIMEOUT))
            .max_redirects(REDIRECTS)
            // A refusal is an answer to report, with its status, and is not a fault of the request.
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            agent,
            delay: Cell::new(delay),
            last: Cell::new(None),
        }
    }

    /// The wait between two requests as it stands.
    #[must_use]
    pub fn delay(&self) -> Duration {
        self.delay.get()
    }

    /// Lengthens the wait to what a site asks for, up to [`MAX_ASKED_DELAY`]. It never shortens it.
    pub fn slow_to(&self, asked: Duration) {
        self.delay
            .set(self.delay.get().max(asked.min(MAX_ASKED_DELAY)));
    }

    /// Asks for `url` and reads at most `limit` bytes of the answer.
    ///
    /// # Errors
    ///
    /// When the address is not `http` or `https`, the server cannot be reached, or the answer
    /// stops part way.
    pub fn get(&self, url: &Url, limit: u64) -> Result<Answer> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow!("{url} is not an http or https address"));
        }
        if let Some(last) = self.last.get() {
            let waited = last.elapsed();
            let delay = self.delay.get();
            if waited < delay {
                std::thread::sleep(delay - waited);
            }
        }
        let outcome = self.ask(url, limit);
        self.last.set(Some(Instant::now()));
        outcome
    }

    fn ask(&self, url: &Url, limit: u64) -> Result<Answer> {
        let response = self
            .agent
            .get(url.as_str())
            .call()
            .map_err(|error| anyhow!("could not reach {url}: {error}"))?;

        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_ascii_lowercase)
        };
        let content_type = header("content-type");
        let mitigated = header("cf-mitigated").is_some();

        let mut body = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(limit.saturating_add(1))
            .read_to_end(&mut body)
            .with_context(|| format!("the answer from {url} stopped part way"))?;
        let too_large = body.len() as u64 > limit;
        body.truncate(usize::try_from(limit).unwrap_or(usize::MAX));

        let challenged = matches!(status, 403 | 503)
            && (mitigated || contains(&body, b"challenges.cloudflare.com"));
        Ok(Answer {
            status,
            content_type,
            body,
            too_large,
            challenged,
        })
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
