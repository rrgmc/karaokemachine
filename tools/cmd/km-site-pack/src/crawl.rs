//! Reads a site's pages and lists the files they link.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::ControlFlow;
use std::time::Duration;

use anyhow::{Result, bail};
use url::Url;

use crate::Event;
use crate::client::{AGENT, Client};
use crate::links::{extract_links, sitemap_locations};
use crate::names::{Extensions, extension, last_segment};
use crate::robots::Robots;

/// The most a page, a sitemap or a `robots.txt` may weigh.
const MAX_PAGE: u64 = 4 * 1024 * 1024;

/// How many sitemaps one run reads. A sitemap may list further sitemaps.
const MAX_SITEMAPS: usize = 20;

/// Extensions that are never a page, so a link to one is not asked for.
const NOT_A_PAGE: [&str; 36] = [
    "jpg", "jpeg", "png", "gif", "bmp", "ico", "svg", "webp", "css", "js", "json", "xml", "rss",
    "pdf", "doc", "docx", "txt", "mp3", "wav", "ogg", "wma", "mp4", "avi", "mpg", "wmv", "mov",
    "flv", "swf", "exe", "msi", "rar", "7z", "gz", "tar", "sf2", "woff",
];

/// What to crawl and how far.
#[derive(Debug, Clone)]
pub struct CrawlOptions {
    /// The address a person gave.
    pub start: Url,
    /// How many links deep a page may be from the start. The start itself is depth 0.
    pub depth: u32,
    /// The most pages one run reads.
    pub max_pages: usize,
    /// The song-file extensions to list.
    pub extensions: Extensions,
}

impl CrawlOptions {
    /// The defaults for one address: two links deep and two thousand pages.
    #[must_use]
    pub fn new(start: Url) -> Self {
        Self {
            start,
            depth: 2,
            max_pages: 2000,
            extensions: Extensions::default(),
        }
    }
}

/// A file a page links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLink {
    /// Where the file is.
    pub url: Url,
    /// The first page found linking it.
    pub page: Url,
}

/// What a crawl found.
#[derive(Debug, Default)]
pub struct Found {
    /// The files to download, ordered by address.
    pub files: Vec<FileLink>,
    /// How many pages were read.
    pub pages_read: usize,
    /// The pages that could not be read, each with the reason.
    pub pages_failed: Vec<(Url, String)>,
    /// How many addresses `robots.txt` asked this program to leave alone.
    pub disallowed: usize,
    /// Whether the page limit stopped the crawl before the site ran out.
    pub capped: bool,
    /// Whether the caller stopped the crawl.
    pub canceled: bool,
    /// The wait left between two requests: the one asked for, or the site's own when it is longer.
    pub delay: Duration,
}

/// The part of a site a crawl stays inside: one host, and the folder the start address is in.
///
/// One host often holds many unrelated sites, each in a folder of its own.
struct Scope {
    host: String,
    port: Option<u16>,
    folder: String,
}

impl Scope {
    fn of(start: &Url) -> Self {
        let path = start.path();
        let folder = match path.rfind('/') {
            Some(at) => path[..=at].to_owned(),
            None => "/".to_owned(),
        };
        Self {
            host: start.host_str().unwrap_or_default().to_ascii_lowercase(),
            port: start.port_or_known_default(),
            folder,
        }
    }

    fn holds(&self, url: &Url) -> bool {
        url.host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(&self.host))
            && url.port_or_known_default() == self.port
            && url.path().starts_with(&self.folder)
    }
}

/// A URL's path and query, which is what a `robots.txt` rule is matched against.
fn robots_path(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

/// Reads the pages of a site and lists the song files and archives they link.
///
/// # Errors
///
/// When the start address cannot be read, or the site answers only a browser.
pub fn crawl(
    client: &Client,
    options: &CrawlOptions,
    mut on: impl FnMut(Event<'_>) -> ControlFlow<()>,
) -> Result<Found> {
    let scope = Scope::of(&options.start);
    let robots = read_robots(client, &options.start);
    let mut found = Found::default();
    if let Some(asked) = robots.crawl_delay {
        client.slow_to(asked);
    }
    found.delay = client.delay();

    // A person naming an address does not change what the site asked of a program.
    if !robots.allows(&robots_path(&options.start)) {
        bail!(
            "{} asks programs not to read it, in its robots.txt",
            options.start
        );
    }

    let mut queue: VecDeque<(Url, u32)> = VecDeque::new();
    let mut queued: BTreeSet<String> = BTreeSet::new();
    let mut files: BTreeMap<String, FileLink> = BTreeMap::new();

    queued.insert(options.start.as_str().to_owned());
    queue.push_back((options.start.clone(), 0));
    if options.depth >= 1 {
        for page in sitemap_pages(client, &options.start, &robots) {
            if scope.holds(&page) {
                consider(
                    &page,
                    &options.start,
                    1,
                    options,
                    &scope,
                    &robots,
                    &mut queue,
                    &mut queued,
                    &mut files,
                    &mut found,
                );
            }
        }
    }

    while let Some((page, depth)) = queue.pop_front() {
        if found.pages_read + found.pages_failed.len() >= options.max_pages {
            found.capped = true;
            break;
        }
        if on(Event::Page {
            url: &page,
            done: found.pages_read,
        })
        .is_break()
        {
            found.canceled = true;
            break;
        }
        let is_start = depth == 0 && page == options.start;
        let answer = match client.get(&page, MAX_PAGE) {
            Ok(answer) => answer,
            Err(error) if is_start => return Err(error),
            Err(error) => {
                found.pages_failed.push((page, format!("{error:#}")));
                continue;
            }
        };
        if answer.challenged {
            bail!(
                "{page} answers only a browser: the site asks each visitor to pass a check this \
                 program does not take. Save the files with a browser and use --from-folder"
            );
        }
        if !answer.is_ok() {
            if is_start {
                bail!("{page} answered with status {}", answer.status);
            }
            found
                .pages_failed
                .push((page, format!("status {}", answer.status)));
            continue;
        }
        // A link with no extension can turn out to be a picture or a download. Only markup is read.
        let is_markup = answer
            .content_type
            .as_deref()
            .is_none_or(|kind| kind.contains("html") || kind.contains("text/plain"));
        if !is_markup {
            continue;
        }
        found.pages_read += 1;
        for link in extract_links(&answer.text(), &page) {
            consider(
                &link,
                &page,
                depth + 1,
                options,
                &scope,
                &robots,
                &mut queue,
                &mut queued,
                &mut files,
                &mut found,
            );
        }
    }

    found.files = files.into_values().collect();
    Ok(found)
}

/// Files one link as a file to download, a page to read, or neither.
#[expect(
    clippy::too_many_arguments,
    reason = "the crawl's whole state, passed to its one helper"
)]
fn consider(
    link: &Url,
    page: &Url,
    depth: u32,
    options: &CrawlOptions,
    scope: &Scope,
    robots: &Robots,
    queue: &mut VecDeque<(Url, u32)>,
    queued: &mut BTreeSet<String>,
    files: &mut BTreeMap<String, FileLink>,
    found: &mut Found,
) {
    let name = last_segment(link).unwrap_or_default();
    let is_file = options.extensions.is_wanted(&name);
    let is_page = !is_file
        && scope.holds(link)
        && depth <= options.depth
        && !extension(&name).is_some_and(|found| NOT_A_PAGE.contains(&found.as_str()));
    if !is_file && !is_page {
        return;
    }
    let key = link.as_str().to_owned();
    if files.contains_key(&key) || queued.contains(&key) {
        return;
    }
    // A file on a second host answers to that host's own rules, which this run has not read. The
    // rules read here are the given host's, and they are applied where they hold.
    if scope.holds(link) && !robots.allows(&robots_path(link)) {
        found.disallowed += 1;
        queued.insert(key);
        return;
    }
    if is_file {
        files.insert(
            key,
            FileLink {
                url: link.clone(),
                page: page.clone(),
            },
        );
    } else {
        queued.insert(key);
        queue.push_back((link.clone(), depth));
    }
}

/// The site's `robots.txt`, or rules that forbid nothing when it has none.
fn read_robots(client: &Client, start: &Url) -> Robots {
    let Ok(address) = start.join("/robots.txt") else {
        return Robots::default();
    };
    match client.get(&address, MAX_PAGE) {
        Ok(answer) if answer.is_ok() => Robots::parse(&answer.text(), AGENT),
        _ => Robots::default(),
    }
}

/// The pages the site's sitemaps list. A site with no sitemap gives none.
fn sitemap_pages(client: &Client, start: &Url, robots: &Robots) -> Vec<Url> {
    let mut waiting: VecDeque<Url> = robots
        .sitemaps
        .iter()
        .filter_map(|listed| Url::parse(listed).ok())
        .collect();
    if waiting.is_empty()
        && let Ok(usual) = start.join("/sitemap.xml")
    {
        waiting.push_back(usual);
    }
    let host = start.host_str().map(str::to_ascii_lowercase);
    let mut read = BTreeSet::new();
    let mut pages = Vec::new();
    while let Some(sitemap) = waiting.pop_front() {
        // A sitemap is read from the given host only, whatever a `robots.txt` line names.
        if read.len() >= MAX_SITEMAPS
            || sitemap.host_str().map(str::to_ascii_lowercase) != host
            || !read.insert(sitemap.as_str().to_owned())
        {
            continue;
        }
        let Ok(answer) = client.get(&sitemap, MAX_PAGE) else {
            continue;
        };
        if !answer.is_ok() {
            continue;
        }
        for listed in sitemap_locations(&answer.text()) {
            if extension(listed.path()).as_deref() == Some("xml") {
                waiting.push_back(listed);
            } else {
                pages.push(listed);
            }
        }
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crawl_stays_in_the_folder_it_was_given() {
        let start = Url::parse("http://127.0.0.1:8000/site/index.html").expect("a url");
        let scope = Scope::of(&start);
        let holds = |address: &str| scope.holds(&Url::parse(address).expect("a url"));
        assert!(holds("http://127.0.0.1:8000/site/a/b.html"));
        assert!(!holds("http://127.0.0.1:8000/other/b.html"));
        assert!(!holds("http://127.0.0.1:9000/site/b.html"));
        assert!(!holds("http://localhost:8000/site/b.html"));
    }

    #[test]
    fn a_site_at_the_root_of_its_host_is_the_whole_host() {
        let start = Url::parse("http://127.0.0.1/").expect("a url");
        let scope = Scope::of(&start);
        assert!(scope.holds(&Url::parse("http://127.0.0.1/any/page.html").expect("a url")));
    }
}
