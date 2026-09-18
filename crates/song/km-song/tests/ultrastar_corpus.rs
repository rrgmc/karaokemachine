//! Reads every `.txt` under a folder of UltraStar songs and says what became of each.
//!
//! **Ignored, and pointed at a folder by `KM_ULTRASTAR_CORPUS`**, because the files are not
//! redistributable and live outside the repository. Synthetic cases are in `src/ultrastar/tests.rs`;
//! this is what says whether those cases are the ones that exist.
//!
//! ```sh
//! KM_ULTRASTAR_CORPUS=<your UltraStar folder> cargo test -p km-song --test ultrastar_corpus -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use km_song::ultrastar;

fn texts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            texts(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
        {
            out.push(path);
        }
    }
}

#[test]
#[ignore = "reads a folder outside the repository; set KM_ULTRASTAR_CORPUS"]
fn every_ultrastar_file_in_a_folder_reads() {
    let root = std::env::var_os("KM_ULTRASTAR_CORPUS").expect("KM_ULTRASTAR_CORPUS names a folder");
    let mut paths = Vec::new();
    texts(Path::new(&root), &mut paths);

    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    let mut empty_lines = 0usize;
    for path in &paths {
        let bytes = std::fs::read(path).expect("readable");
        let outcome = match ultrastar::parse(&bytes) {
            Ok(song) => {
                if song
                    .timeline
                    .lines
                    .iter()
                    .any(|line| line.text().trim().is_empty())
                {
                    empty_lines += 1;
                    println!("a line draws nothing: {}", path.display());
                }
                let audio = path.parent().expect("parent").join(&song.audio);
                if audio.is_file() {
                    format!("read, {}", song.decoder.name())
                } else {
                    println!(
                        "audio {} ({}) missing: {}",
                        song.audio,
                        song.decoder.name(),
                        path.display()
                    );
                    "read, audio missing".to_owned()
                }
            }
            Err(error) => {
                println!("{error}: {}", path.display());
                format!("refused: {error}")
            }
        };
        *outcomes.entry(outcome).or_default() += 1;
    }
    for (outcome, count) in &outcomes {
        println!("{count:>6}  {outcome}");
    }
    println!("{empty_lines:>6}  read with a line that draws nothing");
}
