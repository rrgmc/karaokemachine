//! The hook itself, rather than the writer under it: a real panic, and the file it leaves.
//!
//! **An integration test rather than a unit one, because `set_hook` is process-wide.** Installing it
//! inside the library's own test binary would change how every other test in that binary reports a
//! failure. Here the binary holds one test, and the hook is the thing being tested.
//!
//! **A panic on a spawned thread is what makes this runnable at all.** The hook runs exactly as it
//! would for the display thread, the report is written, and only that thread dies -- so the harness
//! is still alive to look at what was written. Panicking on the main thread would prove the same
//! thing and take the harness with it.

use std::path::PathBuf;

#[test]
fn a_panic_leaves_a_report_without_anybody_having_asked_for_one() {
    let dir = std::env::temp_dir().join("km-logfile-tests").join("hooked");
    let _ = std::fs::remove_dir_all(&dir);

    // No log file, no flag, no environment variable: the point is that this needs none of them.
    km_logfile::report_panics(&dir, "karaokemachine", km_logfile::KEEP_ALL);

    let died = std::thread::Builder::new()
        .name("km-audio".to_owned())
        .spawn(|| panic!("a NUL got into the words"))
        .expect("the thread starts")
        .join();
    assert!(died.is_err(), "the thread is supposed to have panicked");

    let reports: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the hook makes the folder")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|found| found == "crash"))
        .collect();
    assert_eq!(reports.len(), 1, "one panic, one report: {reports:?}");

    let written = std::fs::read_to_string(&reports[0]).expect("read");
    // The four things somebody woken up by this needs: what, where, which thread, and which build.
    assert!(written.contains("a NUL got into the words"), "{written}");
    assert!(written.contains("panicked at"), "{written}");
    assert!(written.contains("km-audio"), "{written}");
    assert!(written.contains(env!("CARGO_PKG_VERSION")), "{written}");
}
