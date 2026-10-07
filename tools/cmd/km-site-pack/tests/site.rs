//! A whole run against an invented site, served from the loopback address.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use km_pack::Rejection;
use km_site_pack::{CrawlOptions, Event, Extensions, Options, PackageOptions};
use km_song::testing;
use km_testkit::Scratch;
use url::Url;

/// One thing the invented site serves.
struct Served {
    status: u16,
    kind: &'static str,
    body: Vec<u8>,
}

fn page(body: &str) -> Served {
    Served {
        status: 200,
        kind: "text/html; charset=utf-8",
        body: body.as_bytes().to_vec(),
    }
}

fn file(body: Vec<u8>) -> Served {
    Served {
        status: 200,
        kind: "application/octet-stream",
        body,
    }
}

/// What the server was asked for: each path, and the name the asker gave.
type Asked = Arc<Mutex<Vec<(String, String)>>>;

/// Serves `routes` from a loopback port until the test ends, and answers 404 for anything else.
fn serve(routes: impl FnOnce(&str) -> BTreeMap<String, Served>) -> (String, Asked) {
    let listener =
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).expect("bind loopback");
    let origin = format!(
        "http://{}",
        listener.local_addr().expect("read back the port")
    );
    let routes = routes(&origin);
    let asked: Asked = Arc::default();
    let heard = Arc::clone(&asked);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            let mut agent = String::new();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("user-agent")
                {
                    value.trim().clone_into(&mut agent);
                }
            }
            let raw = request.split_whitespace().nth(1).unwrap_or("/");
            let path = percent_encoding::percent_decode_str(raw)
                .decode_utf8_lossy()
                .into_owned();
            heard
                .lock()
                .expect("the request log")
                .push((path.clone(), agent));

            let missing = Served {
                status: 404,
                kind: "text/html",
                body: b"<html>not here</html>".to_vec(),
            };
            let served = routes.get(&path).unwrap_or(&missing);
            let head = format!(
                "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                served.status,
                served.kind,
                served.body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&served.body);
        }
    });
    (origin, asked)
}

/// An archive of two songs and a text file. One song claims a path outside the archive's folder.
fn archive() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in [
        ("songs/in.kar", testing::melody_and_accompaniment()),
        ("../../escape.kar", testing::drums_and_lyrics()),
        ("readme.txt", b"hello".to_vec()),
    ] {
        zip.start_file(name, stored).expect("start an entry");
        zip.write_all(&bytes).expect("write an entry");
    }
    zip.finish().expect("finish the archive").into_inner()
}

/// The invented site. Its pages reach three links deep, and it has a page it asks programs to
/// leave alone, a page its sitemap alone names, and a page outside the folder the run is given.
fn site(origin: &str) -> BTreeMap<String, Served> {
    let mut routes = BTreeMap::new();
    let mut put = |path: &str, served: Served| {
        routes.insert(path.to_owned(), served);
    };
    put(
        "/robots.txt",
        Served {
            status: 200,
            kind: "text/plain",
            body: format!(
                "User-agent: *\nDisallow: /site/private/\nSitemap: {origin}/sitemap.xml\n"
            )
            .into_bytes(),
        },
    );
    put(
        "/sitemap.xml",
        Served {
            status: 200,
            kind: "application/xml",
            body: format!(
                "<urlset><url><loc>{origin}/site/deep/hidden.html</loc></url>\
                 <url><loc>{origin}/outside/o.html</loc></url></urlset>"
            )
            .into_bytes(),
        },
    );
    put(
        "/site/index.html",
        page(
            r#"<html><body>
            <a href="a.html">A</a> <a href=private/p.html>P</a> <a href="../outside/o.html">O</a>
            <a href="josé.kar">one</a> <a href="broken.kar">broken</a> <a href="missing.kar">gone</a>
            <A HREF='words.mid'>words</A> <a href="plain.mid">plain</a> <a href="pack.zip">pack</a>
            <img src="logo.jpg"> <a href="mailto:a@example.com">mail</a>
            </body></html>"#,
        ),
    );
    put(
        "/site/a.html",
        page(r#"<a href="b.html">B</a> <a href="one.kar">1</a>"#),
    );
    put(
        "/site/b.html",
        page(r#"<a href="c.html">C</a> <a href="two.kar">2</a>"#),
    );
    put("/site/c.html", page(r#"<a href="never.kar">never</a>"#));
    put(
        "/site/private/p.html",
        page(r#"<a href="secret.kar">s</a>"#),
    );
    put("/outside/o.html", page(r#"<a href="out.kar">out</a>"#));
    put(
        "/site/deep/hidden.html",
        page(r#"<a href="hidden.kar">h</a>"#),
    );

    put("/site/josé.kar", file(testing::soft_karaoke()));
    put("/site/one.kar", file(testing::lyric_events()));
    put("/site/two.kar", file(testing::soft_karaoke_real_layout()));
    put("/site/deep/hidden.kar", file(testing::high_quality_song()));
    put("/site/words.mid", file(testing::named_text_track()));
    put("/site/plain.mid", file(testing::instrumental()));
    put("/site/pack.zip", file(archive()));
    // A server that has lost a file and says so with a page and status 200.
    put(
        "/site/broken.kar",
        page("<html>Sorry, that file has moved</html>"),
    );
    put("/site/never.kar", file(testing::soft_karaoke()));
    put("/site/private/secret.kar", file(testing::soft_karaoke()));
    put("/outside/out.kar", file(testing::soft_karaoke()));
    routes
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read the folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn options(origin: &str, folder: &Path, out: &Path) -> Options {
    let start = Url::parse(&format!("{origin}/site/index.html")).expect("the start address");
    Options {
        site: Some(CrawlOptions::new(start)),
        folder: folder.to_path_buf(),
        extensions: Extensions::default(),
        delay: Duration::ZERO,
        dry_run: false,
        package: Some(PackageOptions {
            name: Some("Invented Site".to_owned()),
            out_dir: Some(out.to_path_buf()),
            ..PackageOptions::default()
        }),
    }
}

#[test]
fn a_site_becomes_a_folder_and_a_package() {
    let (origin, asked) = serve(site);
    let scratch = Scratch::new("site-pack-run");
    let folder = scratch.join("songs");
    let out = scratch.join("out");

    let report = km_site_pack::run(&options(&origin, &folder, &out), |_: Event<'_>| {
        ControlFlow::Continue(())
    })
    .expect("the run");

    // The crawl: four pages within two links, one from the sitemap, and nothing past its limits.
    let found = report.found.as_ref().expect("a crawl");
    assert_eq!(found.pages_read, 4, "index, a, b and the sitemap's page");
    assert_eq!(found.disallowed, 1, "the private page");
    assert!(!found.capped);
    let asked = asked.lock().expect("the request log").clone();
    let paths: Vec<&str> = asked.iter().map(|(path, _)| path.as_str()).collect();
    for never in [
        "/site/c.html",
        "/site/never.kar",
        "/site/private/p.html",
        "/site/private/secret.kar",
        "/outside/o.html",
        "/outside/out.kar",
        "/site/logo.jpg",
    ] {
        assert!(!paths.contains(&never), "{never} was asked for");
    }
    assert!(
        asked
            .iter()
            .all(|(_, agent)| agent.starts_with("km-site-pack/")),
        "every request names the program"
    );

    // The download: the two files the server did not really have are reported and not kept.
    let downloaded = report.downloaded.as_ref().expect("a download");
    assert_eq!(downloaded.written.len(), 7);
    assert_eq!(downloaded.refused.len(), 2, "{:?}", downloaded.refused);
    let unpacked = report.unpacked.as_ref().expect("an unpacking");
    assert_eq!(unpacked.archives, 1);
    assert_eq!(unpacked.written.len(), 2);
    assert_eq!(
        names_in(&folder),
        [
            "hidden.kar",
            "josé.kar",
            "one.kar",
            "pack",
            "pack.zip",
            "plain.mid",
            "two.kar",
            "words.mid",
        ]
    );
    // The entry that claimed `../../escape.kar` is inside the archive's own folder.
    assert_eq!(names_in(&folder.join("pack")), ["escape.kar", "in.kar"]);
    assert_eq!(names_in(&scratch), ["out", "songs"]);

    // The package: every song with words, and the wordless file still in the folder.
    let packaged = report.packaged.as_ref().expect("a package");
    let wordless: Vec<&Path> = packaged
        .rejected
        .iter()
        .filter(|(_, why)| *why == Rejection::NoLyrics)
        .map(|(path, _)| path.as_path())
        .collect();
    assert_eq!(wordless.len(), 1, "{:?}", packaged.rejected);
    assert!(wordless[0].ends_with("plain.mid"));
    assert!(folder.join("plain.mid").is_file());

    assert_eq!(packaged.packages.len(), 1);
    let built = &packaged.packages[0];
    // Six song files came down and two came out of the archive.
    assert_eq!(built.songs, 8 - packaged.rejected.len());
    assert_eq!(built.path, out.join("invented-site-1.0.0.kmpkg"));
    let package = km_kmpkg::Package::open(&built.path).expect("the package opens");
    assert_eq!(package.manifest().songs.len(), built.songs);
    let flags = km_kmpkg::read_flags(&built.path).expect("the flags");
    assert!(flags.contains(km_kmpkg::PackageFlags::UNCURATED));

    // No source address is written down anywhere.
    let host = origin.trim_start_matches("http://");
    let description = std::fs::read_to_string(&built.description).expect("the description");
    let listing =
        std::fs::read_to_string(built.listing.as_ref().expect("a listing")).expect("the listing");
    assert!(!description.contains(host) && !listing.contains(host));
    assert!(description.contains("josé.kar"));

    // A second run asks for no file it has, and builds the same package.
    let again = km_site_pack::run(&options(&origin, &folder, &out), |_: Event<'_>| {
        ControlFlow::Continue(())
    })
    .expect("the second run");
    let downloaded = again.downloaded.expect("a download");
    assert_eq!(downloaded.written.len(), 0);
    assert_eq!(downloaded.kept, 7);
    assert_eq!(again.unpacked.expect("an unpacking").kept, 1);
    assert_eq!(
        again.packaged.expect("a package").packages[0].songs,
        built.songs
    );
}

#[test]
fn a_dry_run_lists_the_files_and_writes_nothing() {
    let (origin, _) = serve(site);
    let scratch = Scratch::new("site-pack-dry");
    let folder = scratch.join("songs");
    let mut options = options(&origin, &folder, &scratch.join("out"));
    options.dry_run = true;

    let report =
        km_site_pack::run(&options, |_: Event<'_>| ControlFlow::Continue(())).expect("the run");

    assert_eq!(report.found.expect("a crawl").files.len(), 9);
    assert!(report.downloaded.is_none() && report.packaged.is_none());
    assert!(names_in(&scratch).is_empty());
}

#[test]
fn a_depth_of_zero_reads_the_one_page() {
    let (origin, asked) = serve(site);
    let scratch = Scratch::new("site-pack-depth");
    let mut options = options(&origin, &scratch.join("songs"), &scratch.join("out"));
    options.dry_run = true;
    if let Some(site) = &mut options.site {
        site.depth = 0;
    }

    let report =
        km_site_pack::run(&options, |_: Event<'_>| ControlFlow::Continue(())).expect("the run");

    let found = report.found.expect("a crawl");
    assert_eq!(found.pages_read, 1);
    assert_eq!(found.files.len(), 6);
    let asked = asked.lock().expect("the request log");
    assert!(!asked.iter().any(|(path, _)| path == "/site/a.html"));
}

#[test]
fn a_site_that_answers_only_a_browser_is_reported_as_one() {
    let (origin, _) = serve(|_| {
        BTreeMap::from([(
            "/site/index.html".to_owned(),
            Served {
                status: 403,
                kind: "text/html",
                body: b"<html><script src=\"https://challenges.cloudflare.com/x.js\"></script>"
                    .to_vec(),
            },
        )])
    });
    let scratch = Scratch::new("site-pack-challenge");
    let folder = scratch.join("songs");

    let error = km_site_pack::run(
        &options(&origin, &folder, &scratch.join("out")),
        |_: Event<'_>| ControlFlow::Continue(()),
    )
    .expect_err("the run stops");

    assert!(format!("{error:#}").contains("answers only a browser"));
    assert!(!folder.exists());
}

#[test]
fn a_folder_filled_by_hand_is_unpacked_and_packaged_with_no_request() {
    let scratch = Scratch::new("site-pack-folder");
    scratch.write("songs/pack.zip", archive());
    scratch.write("songs/one.kar", testing::soft_karaoke());
    let folder = scratch.join("songs");

    let report = km_site_pack::run(
        &Options {
            site: None,
            folder: folder.clone(),
            extensions: Extensions::default(),
            delay: Duration::ZERO,
            dry_run: false,
            package: Some(PackageOptions::default()),
        },
        |_: Event<'_>| ControlFlow::Continue(()),
    )
    .expect("the run");

    assert!(report.found.is_none() && report.downloaded.is_none());
    let packaged = report.packaged.expect("a package");
    assert_eq!(packaged.packages.len(), 1);
    assert_eq!(packaged.packages[0].songs, 3);
    // With no place named, the package goes beside the folder and never into it.
    assert_eq!(packaged.packages[0].path, scratch.join("songs-1.0.0.kmpkg"));
}

#[test]
fn a_minimum_suitability_leaves_the_lower_songs_out() {
    let scratch = Scratch::new("site-pack-minimum");
    scratch.write("songs/good.kar", testing::high_quality_song());
    scratch.write("songs/thin.kar", testing::drums_and_lyrics());
    let folder = scratch.join("songs");
    let run = |min_suitability| {
        km_site_pack::package(
            &folder,
            &PackageOptions {
                min_suitability,
                out_dir: Some(scratch.join(format!("out-{min_suitability:?}"))),
                ..PackageOptions::default()
            },
            |_: Event<'_>| ControlFlow::Continue(()),
        )
        .expect("the packaging")
    };

    let all = run(None);
    assert_eq!(all.packages[0].songs, 2);

    let best = run(Some(10));
    let low: Vec<_> = best
        .rejected
        .iter()
        .filter(|(_, why)| matches!(why, Rejection::LowSuitability(_)))
        .collect();
    assert!(!low.is_empty(), "the thin song is rated below ten");
    assert_eq!(
        best.packages.first().map_or(0, |built| built.songs),
        2 - low.len()
    );
}

#[test]
fn a_site_that_asks_for_a_longer_wait_gets_it() {
    let (origin, _) = serve(|_| {
        BTreeMap::from([
            (
                "/robots.txt".to_owned(),
                Served {
                    status: 200,
                    kind: "text/plain",
                    body: b"User-agent: *\nCrawl-delay: 0.05\n".to_vec(),
                },
            ),
            (
                "/site/index.html".to_owned(),
                page(r#"<a href="one.kar">1</a>"#),
            ),
        ])
    });
    let scratch = Scratch::new("site-pack-delay");
    let mut options = options(&origin, &scratch.join("songs"), &scratch.join("out"));
    options.dry_run = true;

    let report =
        km_site_pack::run(&options, |_: Event<'_>| ControlFlow::Continue(())).expect("the run");

    // The run asked for no wait at all, and the site's own is the longer of the two.
    assert_eq!(
        report.found.expect("a crawl").delay,
        Duration::from_millis(50)
    );
}
