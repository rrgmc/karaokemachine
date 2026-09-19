//! Timing what a page and a scan batch cost, on a database too large to reason about.
//!
//! **Three `#[ignore]`d tests, and they are the only ones in this repository.** They are ignored
//! because none of them runs anywhere but on a machine holding a real corpus, and because each takes
//! minutes to hours — so `cargo km-test` skips them and nothing in CI reaches them. `KM_CORPUS` says
//! which folder, `KM_MMAP` says which of the two settings this run is measuring, and one run measures
//! one setting: the figures mean nothing unless the operating system's cache has been emptied first,
//! and that cannot be done from inside this process.
//!
//! ```sh
//! KM_CORPUS=<a folder holding one .kmbuild> KM_MMAP=on cargo km-test --release -- \
//!     --ignored --exact db::measure::a_cold_page_load_over_a_real_corpus --nocapture
//! KM_CORPUS=<...> KM_MMAP=off KM_SAMPLE=4000 cargo km-test --release -- \
//!     --ignored --exact db::measure::a_bounded_forced_pass_over_a_real_corpus --nocapture
//! KM_CORPUS=<...> KM_MMAP=on cargo km-test --release -- \
//!     --ignored --exact db::measure::where_the_same_words_threshold_sits --nocapture
//! ```
//!
//! **A child of `db` rather than an example**, because this has to call the program's own queries
//! and not a second spelling of them. A measurement carrying its own `SELECT COUNT(*)` reports on a
//! string that happens to match today and stops matching the day anybody adds a term — and it goes
//! on printing a number, which is the same failure the `counts` cache's own note warns about. Being
//! a child module reaches `Db`, its connection and the five aggregates with no widening of anything.
//!
//! **Its own file rather than a section of `tests.rs`**, whose header argues for one module on the
//! grounds that the tests share a fixture and a `db()` helper. These share neither: they need a real
//! corpus, they never run in CI, and they take hours.
//!
//! **Every figure here is about somebody's own corpus.** What it prints under `local only` goes in a
//! file git does not carry; the commit-safe block holds durations, ratios and the sample size, and
//! never a file count, a song count, a row count or a database size. See `What a committed file may
//! say about the machine it was written on` in `docs/decisions/repository.md`.

use super::*;

use std::time::{Duration, Instant};

/// The folder to measure. No default: a measurement that quietly invented a corpus would be worse
/// than one that refused to start.
fn corpus() -> PathBuf {
    let Ok(root) = std::env::var("KM_CORPUS") else {
        panic!(
            "KM_CORPUS must name a folder holding one .kmbuild -- see the commands in BUILDING.md"
        );
    };
    PathBuf::from(root)
}

/// Which mapping this run is measuring, and the figure it asks `tune` for.
fn mapping() -> (&'static str, i64) {
    match std::env::var("KM_MMAP").as_deref() {
        Ok("on") => ("mapped", -1),
        Ok("off") => ("unmapped", 0),
        other => {
            panic!("KM_MMAP must be on or off, so a run says which it measured (got {other:?})")
        }
    }
}

/// A duration in the unit that suits its size, the way the architecture note writes them.
fn took(at: Duration) -> String {
    let millis = at.as_secs_f64() * 1000.0;
    if millis < 1.0 {
        format!("{millis:.3} ms")
    } else if millis < 1000.0 {
        format!("{millis:.1} ms")
    } else if at.as_secs() < 120 {
        format!("{:.2} s", at.as_secs_f64())
    } else {
        format!("{} m {:02} s", at.as_secs() / 60, at.as_secs() % 60)
    }
}

/// Times one call, and hands back both the duration and what it answered.
fn timed<T>(work: impl FnOnce() -> T) -> (Duration, T) {
    let started = Instant::now();
    let out = work();
    (started.elapsed(), out)
}

/// What the connection actually got, which is the only honest way to know a pragma took.
fn says(db: &Db, pragma: &str) -> String {
    db.pragma_for_test(pragma)
        .unwrap_or_else(|_| "unreadable".to_owned())
}

/// What a page load costs on a corpus-sized database, and whether the mapping changes it.
///
/// **The five aggregates are timed in sequence on one connection**, because that is what
/// `count_everything` does on a page load: each carries the benefit of whatever pages the ones
/// before it faulted in, and so does the page that follows them. That makes each row a share of one
/// page load rather than a standalone cost, which the note says beside the table.
///
/// The cold pass is the upper bound and the warm pass the lower. A page drawn while a scan writes
/// sits between them, and on a spinning disk nearer cold, because the writer is evicting.
#[test]
#[ignore = "needs a real corpus and an emptied page cache; see the module header"]
fn a_cold_page_load_over_a_real_corpus() {
    let root = corpus();
    let (name, bytes) = mapping();
    map_this_much_for_test(bytes);

    let db = Db::open_reading(&root).expect("open the corpus for reading");
    println!("## {name}\n");
    println!("- `mmap_size` in force: {}", says(&db, "mmap_size"));
    println!("- `cache_size` in force: {}\n", says(&db, "cache_size"));

    for pass in ["cold", "warm"] {
        println!("### the status bar, {pass}\n");
        println!("| aggregate | took |");
        println!("|---|---|");
        let mut total = Duration::ZERO;
        for (what, at) in [
            ("songs", timed(|| db.count_songs()).0),
            ("files", timed(|| db.count_files()).0),
            ("failed", timed(|| db.count_failed()).0),
            ("favorites", timed(|| db.count_favorites()).0),
            ("packages", timed(|| db.count_packages()).0),
        ] {
            total += at;
            println!("| {what} | {} |", took(at));
        }
        println!("| **the five together** | **{}** |\n", took(total));

        println!("### a browse page, {pass}\n");
        let (rows, page) = timed(|| db.songs_page(&Filter::default()));
        let shown = page.expect("a first page of rows").0.len();
        let (counting, _) = timed(|| db.song_count(&Filter::default()));
        println!("| what | took |");
        println!("|---|---|");
        println!("| a first page of rows | {} |", took(rows));
        println!("| the count that labels it | {} |\n", took(counting));
        // Kept out of the commit-safe block: how many rows a page holds is fine, but it is printed
        // here only so a reader can see the page was not empty.
        println!("local only: the page showed {shown} rows\n");
    }

    // **The cache, proved both ways.** The first of these must be a hit, because nothing has written
    // and both halves of the key stand still on a read-only connection; the second must cost like a
    // miss once the cache is cleared. Together they are the evidence for the whole question: during
    // a scan the writer moves `data_version`, so every page load pays the miss.
    let (first, _) = timed(|| db.counts());
    let (again, _) = timed(|| db.counts());
    db.counts.set(None);
    let (forced, _) = timed(|| db.counts());
    println!("### the cache\n");
    println!("| what | took |");
    println!("|---|---|");
    println!("| `counts()` | {} |", took(first));
    println!("| again, unchanged | {} |", took(again));
    println!("| again, cache cleared | {} |\n", took(forced));

    // **Shown rather than asserted**, because which of these an index can serve is the whole
    // explanation of the figures above and is not guessable from the SQL. The third is the one that
    // matters: it is the same `WHERE` the rows beside it are drawn through, and `Db::song_count`
    // builds it from the filter rather than spelling it out, so it is rebuilt the same way here.
    let (where_sql, _) = Filter::default().to_sql();
    let filtered = format!("SELECT COUNT(*) FROM songs s WHERE {where_sql}");
    // Forced through the index as well as left to the planner, because the two answer different
    // questions: whether an index *can* serve the count, and whether the planner thinks it should.
    let forced =
        format!("SELECT COUNT(*) FROM songs s INDEXED BY songs_countable WHERE {where_sql}");
    for sql in [
        "SELECT COUNT(*) FROM songs WHERE merged_into IS NULL",
        "SELECT COUNT(*) FROM files",
        &filtered,
        &forced,
    ] {
        match db.plan_for_test(sql, &[]) {
            Ok(plan) => println!("```\n{sql}\n{plan}\n```\n"),
            Err(error) => println!("```\n{sql}\nrefused: {error}\n```\n"),
        }
    }

    // What the planner has to go on. An index with no row here is one it will not choose, which is
    // the first thing to check when it declines one.
    if let Ok(stats) = db.stats_for_test("songs") {
        println!("`sqlite_stat1` for `songs`:\n\n```\n{stats}\n```\n");
    }

    println!("local only: {}", root.display());
    map_this_much_for_test(-1);
}

/// What it costs to open a corpus-sized database on a build that added an index.
///
/// **The read test cannot do this and must not.** `Db::open_reading` runs no migration and no schema
/// batch — that is the whole point of it — so an index this build declares does not exist on a
/// database until something opens it for writing. A measurement that forgot this reports on the
/// index it meant to test while the index is absent, which is exactly what happened once.
///
/// It is also the figure a person pays: opening a curated corpus on a build that added an index
/// builds it and gathers statistics again, because an index with no `sqlite_stat1` row is one the
/// planner will not choose.
#[test]
#[ignore = "needs a real corpus; see the module header"]
fn bringing_a_corpus_up_to_this_build() {
    let root = corpus();
    let (name, bytes) = mapping();
    map_this_much_for_test(bytes);

    let (at, opened) = timed(|| Db::open(&root));
    let db = opened.expect("open the corpus for writing");
    println!("## {name}, brought up to this build\n");
    println!("| what | took |");
    println!("|---|---|");
    println!("| the whole open | {} |\n", took(at));

    if let Ok(stats) = db.stats_for_test("songs") {
        println!("`sqlite_stat1` for `songs` after it:\n\n```\n{stats}\n```\n");
    }

    let pages = db.close();
    println!(
        "local only: {} — {pages} page(s) folded back",
        root.display()
    );
    map_this_much_for_test(-1);
}

/// What a scan batch's write costs, and whether the mapping changes it.
///
/// **A bounded forced pass, never a whole one.** `KM_SAMPLE` is required and the options are built
/// with `only`, so the pass reads a sample rather than the corpus: a scoped run cannot reach
/// `forget_missing`, does not stamp `last_scan` and does not touch the duplicate verdicts, which is
/// what makes running it against a real corpus an ordinary re-analysis of those songs rather than
/// something to be undone afterwards.
///
/// **A poller reads alongside it**, because that is the regime the status bar's cost actually
/// matters in: while the writer commits, `data_version` moves on every batch and every page load
/// pays a full recount. It competes with the writer for the disk, identically in both runs — so the
/// ratio stands and the absolute figures are a little pessimistic.
///
/// The phase timings the scan already keeps are printed at the end, which is a second reading of the
/// tail's two whole-corpus passes under this setting.
#[test]
#[ignore = "needs a real corpus and an emptied page cache; see the module header"]
fn a_bounded_forced_pass_over_a_real_corpus() {
    let root = corpus();
    let (name, bytes) = mapping();
    let wanted: usize = std::env::var("KM_SAMPLE")
        .ok()
        .and_then(|count| count.parse().ok())
        .expect("KM_SAMPLE must say how many files to re-read; a whole pass is not reachable here");

    map_this_much_for_test(bytes);

    let mut paths = Vec::new();
    km_pack::collect_songs(&root, &mut paths);
    paths.sort();
    assert!(!paths.is_empty(), "no songs under {}", root.display());
    // Every nth, so the sample is spread across the corpus and the rows it writes are scattered by
    // construction — a song's id is the hash of its bytes. Deterministic, so two runs write the same
    // rows and the comparison is of the mapping rather than of the sample.
    let stride = (paths.len() / wanted).max(1);
    let sample: std::collections::HashSet<String> = paths
        .iter()
        .step_by(stride)
        .take(wanted)
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();

    let db = Db::open(&root).expect("open the corpus for writing");
    println!("## {name}, a bounded forced pass\n");
    println!("- `mmap_size` in force: {}", says(&db, "mmap_size"));
    println!("- files sampled: {}\n", sample.len());

    let shared = std::sync::Arc::new(Shared::new(db));
    let progress = std::sync::Arc::new(crate::scan::Progress::default());

    // The poller, which is the status bar measured while a scan writes.
    let watching = std::sync::Arc::clone(&progress);
    let reading_root = root.clone();
    let poller = std::thread::spawn(move || {
        let mut samples: Vec<Duration> = Vec::new();
        while !watching.snapshot().finished {
            std::thread::sleep(Duration::from_secs(5));
            if let Ok(db) = Db::open_reading(&reading_root) {
                let (at, _) = timed(|| db.counts());
                samples.push(at);
            }
        }
        samples
    });

    let options = crate::scan::ScanOptions::only(sample);
    assert!(
        options.only.is_some(),
        "a whole forced pass must be unreachable from here"
    );
    let (whole, outcome) = timed(|| crate::scan::run(&shared, options, &progress));
    outcome.expect("the scan finished");

    let mid_scan = poller.join().expect("the poller finished");
    println!("### while it wrote\n");
    println!("| what | took |");
    println!("|---|---|");
    println!("| the whole pass | {} |", took(whole));
    if !mid_scan.is_empty() {
        let worst = mid_scan.iter().max().copied().unwrap_or_default();
        let total: Duration = mid_scan.iter().sum();
        println!(
            "| the status bar, mid-scan, worst of {} | {} |",
            mid_scan.len(),
            took(worst)
        );
        println!(
            "| the status bar, mid-scan, mean | {} |",
            took(total / mid_scan.len() as u32)
        );
    }
    println!();

    println!("### the phases the scan kept\n");
    println!("| phase | took |");
    println!("|---|---|");
    for (phase, at) in progress.timings() {
        println!("| {phase} | {} |", took(at));
    }
    println!();

    // Checkpointed and the mapping dropped, so the corpus is left as an ordinary close leaves it.
    let pages = shared.lock().close();
    println!(
        "local only: {} — {pages} page(s) of journal folded back",
        root.display()
    );
    map_this_much_for_test(-1);
}

/// Where the same-words threshold sits, and whether the phrases reach the files it is set for.
///
/// **The corpus supplies its own ground truth, which is what makes this a measurement.**
///
/// - The **true pairs** are the ones the duplicate pass already proposes from a matching structure
///   *and* a matching name, whose two lyric keys differ. Shape and name agreeing is an independent
///   statement that the two files are one recording; the keys differing means the transcriptions are
///   not identical, which is the case the exact key cannot reach and this page exists for.
/// - The **false pairs** are songs the pass proposed nothing for, taken two at a time. What they
///   score is what a coincidence can reach.
///
/// [`crate::lyric_likeness::THRESHOLD`] belongs in the gap between the true pairs' low tail and the
/// false pairs' high tail, and the table prints both so it can be read off them.
///
/// **Recall is the figure that decides whether this works, and it is not the threshold.** A true pair
/// the phrases never return as a candidate is invisible whatever the threshold says, and the answer
/// to a low number is more [`crate::lyric_likeness::PROBES`] or a shorter
/// [`crate::lyric_likeness::SHINGLE`] — never a lower threshold.
///
/// ```sh
/// KM_CORPUS=<a folder holding one .kmbuild> KM_MMAP=on cargo km-test --release -- \
///     --ignored --exact db::measure::where_the_same_words_threshold_sits --nocapture
/// ```
#[test]
#[ignore = "needs a real corpus; see the module header"]
fn where_the_same_words_threshold_sits() {
    use crate::lyric_likeness::{THRESHOLD, likeness};

    let root = corpus();
    let (mapping, pages) = mapping();
    map_this_much_for_test(pages);
    let db = Db::open(&root).expect("open the corpus");
    println!("## Where the same-words threshold sits\n");
    println!("`{mapping}`, `mmap_size` = {}\n", says(&db, "mmap_size"));

    // The pass's own reading of the corpus, which is where both sets of pairs come from.
    let (reading, songs) = timed(|| db.fingerprints().expect("fingerprints"));
    println!(
        "reading every song's shape and words took {}\n",
        took(reading)
    );

    let lyrics_of = |id: &str| -> Option<String> {
        db.conn
            .query_row("SELECT lyrics FROM songs WHERE id = ?1", [id], |row| {
                row.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
    };
    let scored = |a: &str, b: &str| -> Option<f32> {
        let mine = crate::lyric_likeness::Shingles::of(&lyrics_of(a)?)?;
        Some(likeness(&mine, &lyrics_of(b)?))
    };

    let proposed = crate::dupes::suggest(&songs);
    let keyed: std::collections::HashMap<&str, &Option<String>> = songs
        .iter()
        .map(|song| (song.id.as_str(), &song.lyric_key))
        .collect();

    // True pairs: proposed by shape and name, and not already caught by the exact key.
    //
    // **Scoring every one of them and asking the index about a sample.** A score is two row lookups
    // and two sets; a recall check is a whole search, and the pass proposes tens of thousands of
    // pairs on a real corpus. So the distribution is the whole population and recall is
    // [`RECALL_SAMPLE`] of it, which is what makes this a run somebody waits for rather than leaves
    // overnight.
    let mut agreed: Vec<f32> = Vec::new();
    let mut asked = 0usize;
    let mut reached = 0usize;
    for (a, b, _, reason) in &proposed {
        if reason == "same words" {
            continue;
        }
        let (Some(left), Some(right)) = (keyed.get(a.as_str()), keyed.get(b.as_str())) else {
            continue;
        };
        if left.is_none() || left == right {
            continue;
        }
        let Some(likeness) = scored(a, b) else {
            continue;
        };
        agreed.push(likeness);
        // Whether the phrases would have returned the other file at all, which no threshold can fix.
        //
        // **Every version, and the measurement is worthless without it.** These pairs are the ones
        // the duplicate pass grouped, so one of each is hidden behind the other as a version — and
        // the default filter collapses those in SQL, before a phrase is asked for anything. Asking
        // with it on measures the filter and calls it recall.
        if asked < RECALL_SAMPLE {
            asked += 1;
            let unfiltered = Filter {
                versions: VersionsFilter::All,
                ..Filter::default()
            };
            if db
                .similar_words(a, &unfiltered)
                .is_ok_and(|(hits, _)| hits.iter().any(|song| &song.id == b))
            {
                reached += 1;
            }
        }
    }

    // False pairs: songs the pass proposed nothing for at all.
    let paired: std::collections::HashSet<&str> = proposed
        .iter()
        .flat_map(|(a, b, _, _)| [a.as_str(), b.as_str()])
        .collect();
    let alone: Vec<&Fingerprint> = songs
        .iter()
        .filter(|song| song.lyric_key.is_some() && !paired.contains(song.id.as_str()))
        .collect();
    let mut coincidence: Vec<f32> = Vec::new();
    for pair in alone.chunks(2).take(COINCIDENCE_SAMPLE) {
        let [a, b] = pair else { continue };
        if let Some(likeness) = scored(&a.id, &b.id) {
            coincidence.push(likeness);
        }
    }

    agreed.sort_by(f32::total_cmp);
    coincidence.sort_by(f32::total_cmp);
    let at = |sorted: &[f32], share: f64| -> String {
        if sorted.is_empty() {
            return "--".to_owned();
        }
        let nth = ((sorted.len() - 1) as f64 * share).round() as usize;
        format!("{:.3}", sorted[nth])
    };

    println!("| pairs | sampled | 5th | 50th | 95th |");
    println!("|---|---|---|---|---|");
    println!(
        "| the same recording, keyed differently | {} | {} | {} | {} |",
        agreed.len(),
        at(&agreed, 0.05),
        at(&agreed, 0.50),
        at(&agreed, 0.95)
    );
    println!(
        "| paired with nothing | {} | {} | {} | {} |",
        coincidence.len(),
        at(&coincidence, 0.05),
        at(&coincidence, 0.50),
        at(&coincidence, 0.95)
    );
    println!();

    let kept = |sorted: &[f32], floor: f32| -> String {
        if sorted.is_empty() {
            return "--".to_owned();
        }
        let over = sorted.iter().filter(|value| **value >= floor).count();
        format!("{:.1}%", 100.0 * over as f64 / sorted.len() as f64)
    };
    println!("| threshold | true pairs kept | false pairs let through |");
    println!("|---|---|---|");
    for floor in [0.4_f32, 0.5, 0.6, 0.7, 0.8, 0.9] {
        println!(
            "| {floor:.2} | {} | {} |",
            kept(&agreed, floor),
            kept(&coincidence, floor)
        );
    }
    println!();

    if asked > 0 {
        println!(
            "the phrases reached the other file in {:.1}% of {asked} true pairs asked\n",
            100.0 * reached as f64 / asked as f64
        );
    }

    // What one page costs, over songs that have words to compare.
    let mut timings: Vec<Duration> = songs
        .iter()
        .filter(|song| song.lyric_key.is_some())
        .take(SEARCHES_TIMED)
        .map(|song| timed(|| db.similar_words(&song.id, &Filter::default())).0)
        .collect();
    timings.sort();
    if let Some(worst) = timings.last() {
        println!("| one search, over {} songs | took |", timings.len());
        println!("|---|---|");
        println!("| median | {} |", took(timings[timings.len() / 2]));
        println!(
            "| 95th | {} |",
            took(timings[(timings.len() * 95 / 100).min(timings.len() - 1)])
        );
        println!("| worst | {} |", took(*worst));
        println!();
    }

    // **The assertion that holds whatever the corpus is.** The loose reading has to agree with the
    // strict one wherever the strict one fires, which is what *the loose counterpart* means.
    for (a, b, _, reason) in &proposed {
        if reason == "same words" {
            assert_eq!(
                scored(a, b),
                Some(1.0),
                "two songs the exact key joins must score 1.0"
            );
        }
    }

    println!("`THRESHOLD` is {THRESHOLD}, and belongs between the two tails above.");
    println!("local only: {}", root.display());
    map_this_much_for_test(-1);
}

/// How many searches the measurement times. Enough for a median to mean something, few enough that
/// the run is not itself the slow part.
const SEARCHES_TIMED: usize = 50;

/// How many true pairs the index is actually asked about.
///
/// A recall check is a whole search where a score is two row lookups, and the pass proposes tens of
/// thousands of pairs on a real corpus. A share read off two hundred of them is worth what a share
/// read off all of them is, and it is the difference between a run somebody waits for and one left
/// overnight.
const RECALL_SAMPLE: usize = 200;

/// How many pairs of unpaired songs are scored for the coincidence tail.
///
/// The tail is what a threshold has to clear, and it settles long before the corpus runs out.
const COINCIDENCE_SAMPLE: usize = 20_000;
