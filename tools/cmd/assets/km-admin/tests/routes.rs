//! Every path this program's own pages name is a route it mounts.
//!
//! **The mirror of `every_call_this_program_makes_is_a_route_the_machine_mounts`.** That one sweeps
//! what this program asks of a *machine*, against the machine's own route table. This one sweeps
//! what it asks of *itself* — and the two directions fail the same way, silently, because a page
//! that asks for a path nothing answers looks like a page with a missing picture rather than like a
//! broken link.
//!
//! **What makes the sweep necessary is the prefix.** Everything is served under `/admin` while both
//! routers declare their paths without it, so every `src`, `action`, `hx-get` and `Location` spells
//! `/admin` by hand — the arrangement
//! [`docs/architecture/admin.md`](../../../../../docs/architecture/admin.md) argues for, whose cost
//! is exactly this.
//!
//! **`PATCH` is how a path is asked about without being used.** axum answers `405` for a path it has
//! a route for under another method and `404` for one it has no route for at all, so *not 404* is
//! the whole assertion and no handler ever runs: no cache to populate, no machine to stand up, no
//! job started, nothing written.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use km_admin::server::{State, router};
use tower::ServiceExt as _;

/// A sample for each placeholder a path in the markup can carry.
///
/// **Samples rather than patterns**, for the reason the machine-side sweep gives: this compares a
/// whole built path against a whole mounted one, so a route whose *shape* changed — a segment added,
/// `fetch` renamed — fails here. The values need only be well-formed; `PATCH` never reaches the
/// handler that would look them up.
const SAMPLES: &[(&str, &str)] = &[
    ("section", "pictures"),
    ("bank.id", "generaluser"),
    ("pack.id", "2026-09-09-120"),
    ("picture.provider", "pixabay"),
    ("picture.id", "7"),
];

/// Every root-relative path written into this program's own templates.
///
/// **Read from the directory rather than a list**, so a template added later cannot escape the
/// sweep by not being named here.
fn paths_in_templates() -> BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut found = BTreeSet::new();
    let entries = std::fs::read_dir(&dir).expect("this program has templates");
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|kind| kind != "html") {
            continue;
        }
        let markup = std::fs::read_to_string(&path).expect("a template of text");
        for raw in root_relative_values(&markup) {
            found.insert(with_samples(&raw, path.display()));
        }
    }
    assert!(
        !found.is_empty(),
        "the scan found no paths at all, so it is asserting nothing"
    );
    found
}

/// Every `="/…"` value in one template, whole, newlines included.
///
/// Attribute-agnostic on purpose: `src`, `action`, `href` and every `hx-` verb are the same mistake
/// waiting to happen, and naming them would be a list to keep.
fn root_relative_values(markup: &str) -> Vec<String> {
    let mut values = Vec::new();
    let bytes = markup.as_bytes();
    let mut at = 0;
    while let Some(found) = markup[at..].find("=\"/") {
        let opens = at + found + 2;
        let closes = bytes[opens..]
            .iter()
            .position(|byte| *byte == b'"')
            .map(|offset| opens + offset);
        let Some(closes) = closes else { break };
        values.push(markup[opens..closes].to_owned());
        at = closes;
    }
    values
}

/// One markup path with its placeholders filled in.
///
/// **A placeholder with no sample fails rather than being skipped**, so a new one has to be thought
/// about instead of quietly leaving a path unswept.
fn with_samples(raw: &str, whose: impl std::fmt::Display) -> String {
    let mut path = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(opens) = rest.find("{{") {
        path.push_str(&rest[..opens]);
        let after = &rest[opens + 2..];
        let closes = after
            .find("}}")
            .unwrap_or_else(|| panic!("{whose} has an unclosed placeholder in {raw}"));
        let name = after[..closes].trim();
        let sample = SAMPLES
            .iter()
            .find(|(placeholder, _)| *placeholder == name)
            .map(|(_, sample)| *sample)
            .unwrap_or_else(|| {
                panic!("{whose} names {{{{ {name} }}}} in {raw}, which SAMPLES has no sample for")
            });
        path.push_str(sample);
        rest = &after[closes + 2..];
    }
    path.push_str(rest);
    path
}

/// Whether this program has a route for a path, whatever the method.
async fn is_mounted(state: &State, path: &str) -> bool {
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    response.status() != StatusCode::NOT_FOUND
}

/// **The failure this holds down is a quiet one.** A `src` that drops the prefix draws the review
/// grid as a hundred and twenty lines of alt text, and a `Location` that drops it sends a browser
/// which has just pressed a button to a page that is not there — neither says *broken link*
/// anywhere, and neither is a shape a reader of the markup notices.
#[tokio::test]
async fn own_paths_are_routes_this_program_mounts() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    // No machine: a path is mounted or it is not, and nothing here asks one anything.
    let state = State::new(dir.path().to_path_buf(), None);

    let mut swept: Vec<String> = paths_in_templates().into_iter().collect();
    // The consts a text scan cannot see: one reaches markup through a struct field —
    // `_job.html`'s `hx-get="{{ list_route }}"` — and two through a `Location` header.
    swept.extend(
        km_admin::views::OWN_PATHS
            .iter()
            .map(|path| (*path).to_owned()),
    );

    let mut missing = Vec::new();
    for path in &swept {
        if !is_mounted(&state, path).await {
            missing.push(path.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "this program's own pages name {} path(s) it does not mount: {}",
        missing.len(),
        missing.join(", ")
    );
}

/// A path that really is absent is reported as absent.
///
/// **Without this the sweep above could be vacuously green**: `is_mounted` answers on a status, and
/// a harness that answered something other than `404` for everything would pass every path ever
/// written. The pair asserted here is one path with and without the prefix, which is the whole
/// distinction the sweep rests on.
#[tokio::test]
async fn a_path_with_no_route_is_seen_as_missing() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let state = State::new(dir.path().to_path_buf(), None);

    assert!(
        !is_mounted(&state, "/pictures/thumb/pixabay/7").await,
        "the sweep cannot tell a missing route from a mounted one"
    );
    assert!(
        is_mounted(&state, "/admin/pictures/thumb/pixabay/7").await,
        "the prefixed path is the one that is mounted"
    );
}
