//! The evening on disk: `evening.json` in the data directory.
//!
//! **The file exists only while `singers.keep_evening` is on.** It holds the queue, the seats with
//! their tokens, the requests, the record of what was sung and the evening's word. So a machine
//! that starts again finds the night where it stopped, and every phone keeps its seat.
//!
//! **A file this build cannot read is set aside, and the machine starts empty.** A newer version,
//! an older one, a file with no version and a file that does not parse all take that path. The
//! file becomes `evening.json.bad`, because a lost queue costs less than a machine that will not
//! start.
//!
//! **A second file holds the evening from an earlier write.** A power cut can damage the file
//! being written, and a venue's machine loses power often. So at most once in
//! [`KEEP_PREVIOUS_EVERY`] the file becomes `evening.previous.json` before the next write. A start
//! that cannot use `evening.json` reads that copy, and loses a minute of the night where it would
//! lose all of it.
//!
//! **A seat's token is written as it is.** It proves one singer's seat for one evening and nothing
//! else, so it gets the care the rest of the data directory gets.
//!
//! This module reads and writes. [`crate::machine`] decides when.

use std::path::Path;

use km_queue::EveningSnapshot;
use serde::{Deserialize, Serialize};

/// The version this build writes.
///
/// It rises when a field changes its meaning under a file that already exists. A new field with a
/// default needs no new version.
pub const CURRENT_EVENING_VERSION: u32 = 1;

/// The oldest version this build reads.
pub const OLDEST_EVENING_VERSION: u32 = 1;

/// How long the earlier copy stands before the file replaces it.
///
/// A minute, so the copy was written well before the power cut that damaged the file. A shorter
/// time puts both writes inside one fault, and a longer one loses more of the night.
pub const KEEP_PREVIOUS_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// What the file holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EveningFile {
    /// The version the file was written in.
    pub version: u32,
    /// The evening's word, which a phone keeps beside its seats.
    pub word: String,
    /// Everything the evening holds.
    pub evening: EveningSnapshot,
}

/// An evening and its word, which is what one write carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The evening's word.
    pub word: String,
    /// Everything the evening holds.
    pub evening: EveningSnapshot,
}

/// The bytes of a file holding `kept`, in the current version.
pub fn encode(kept: &Kept) -> Vec<u8> {
    let file = EveningFile {
        version: CURRENT_EVENING_VERSION,
        word: kept.word.clone(),
        evening: kept.evening.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&file).expect("an evening serializes");
    bytes.push(b'\n');
    bytes
}

/// Reads the file, or sets it aside when this build cannot read it.
///
/// `None` means the machine starts an empty evening: no file, or one that is now
/// `evening.json.bad`.
pub fn load(path: &Path) -> Option<Kept> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::error!(%error, "evening.json could not be opened; starting an empty evening");
            return None;
        }
    };
    match serde_json::from_str::<EveningFile>(&text) {
        Ok(file) if (OLDEST_EVENING_VERSION..=CURRENT_EVENING_VERSION).contains(&file.version) => {
            Some(Kept {
                word: file.word,
                evening: file.evening,
            })
        }
        Ok(file) => {
            set_aside(
                path,
                &format!(
                    "it is in version {}, and this build reads {OLDEST_EVENING_VERSION} to \
                     {CURRENT_EVENING_VERSION}",
                    file.version
                ),
            );
            None
        }
        Err(error) => {
            set_aside(path, &error.to_string());
            None
        }
    }
}

/// Renames a file this build cannot use to `evening.json.bad`, and says why.
pub fn set_aside(path: &Path, reason: &str) {
    let refused = path.with_extension("json.bad");
    tracing::warn!(
        reason,
        kept = %refused.display(),
        "a kept evening could not be used, and it was set aside"
    );
    let _ = std::fs::rename(path, &refused);
}

/// Writes the file whole, so a power cut leaves the evening before or the evening after.
pub fn write(path: &Path, kept: &Kept) -> std::io::Result<()> {
    km_api::files::replace(path, &encode(kept))
}

/// Makes the file the earlier copy, so the next write leaves two evenings on the disk.
///
/// A rename, so the copy is whole at every instant. With no file there is nothing to keep, and
/// the copy that exists stays.
pub fn rotate(path: &Path, previous: &Path) {
    match std::fs::rename(path, previous) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            tracing::warn!(%error, "could not keep the earlier copy of evening.json");
        }
        _ => {}
    }
}

/// Removes the file. A file that is already absent is no failure.
pub fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use km_queue::{Evening, QueueRequest};
    use km_songcode::SongCode;
    use km_testkit::Scratch;

    use super::*;

    fn an_evening() -> Kept {
        let mut evening = Evening::new();
        evening
            .add(QueueRequest {
                number: SongCode::new(1_500),
                title: "A song".to_owned(),
                artist: None,
                singer: Some("Ana".to_owned()),
                duration_ms: 180_000,
                seat: None,
            })
            .expect("add");
        Kept {
            word: "0123abcd".to_owned(),
            evening: evening.snapshot(None, Instant::now()),
        }
    }

    fn bad_beside(path: &Path) -> bool {
        path.with_extension("json.bad").exists()
    }

    #[test]
    fn a_written_evening_reads_back_as_it_was() {
        let scratch = Scratch::new("evening-store-round-trip");
        let path = scratch.path().join("evening.json");
        let kept = an_evening();

        write(&path, &kept).expect("write");

        assert_eq!(load(&path), Some(kept));
        assert!(!bad_beside(&path));
    }

    #[test]
    fn no_file_is_an_empty_evening_and_nothing_is_set_aside() {
        let scratch = Scratch::new("evening-store-missing");
        let path = scratch.path().join("evening.json");
        assert_eq!(load(&path), None);
        assert!(!bad_beside(&path));
        remove(&path).expect("an absent file is no failure");
    }

    /// Every file this build cannot read becomes `evening.json.bad`, and the machine starts empty.
    #[test]
    fn a_file_this_build_cannot_read_is_set_aside() {
        let sound = String::from_utf8(encode(&an_evening())).expect("utf-8");
        let newer = sound.replacen(
            &format!("\"version\": {CURRENT_EVENING_VERSION}"),
            &format!("\"version\": {}", CURRENT_EVENING_VERSION + 1),
            1,
        );
        let older = sound.replacen(
            &format!("\"version\": {CURRENT_EVENING_VERSION}"),
            "\"version\": 0",
            1,
        );
        let unnumbered = sound.replacen(&format!("\"version\": {CURRENT_EVENING_VERSION},"), "", 1);
        assert_ne!(newer, sound);
        assert_ne!(unnumbered, sound);

        for (name, text) in [
            ("newer", newer.as_str()),
            ("older", older.as_str()),
            ("unnumbered", unnumbered.as_str()),
            ("broken", "{ not json"),
        ] {
            let scratch = Scratch::new(&format!("evening-store-{name}"));
            let path = scratch.path().join("evening.json");
            std::fs::write(&path, text).expect("write");

            assert_eq!(load(&path), None, "{name}");
            assert!(bad_beside(&path), "{name} is kept beside");
            assert!(!path.exists(), "{name} is out of the way");
        }
    }

    #[test]
    fn a_rotated_file_is_the_earlier_copy_and_the_next_write_leaves_two() {
        let scratch = Scratch::new("evening-store-rotate");
        let path = scratch.path().join("evening.json");
        let previous = scratch.path().join("evening.previous.json");
        let first = an_evening();
        let mut second = an_evening();
        second.word = "89abcdef".to_owned();

        rotate(&path, &previous);
        assert!(!previous.exists(), "nothing to keep yet");

        write(&path, &first).expect("write");
        rotate(&path, &previous);
        write(&path, &second).expect("write");
        assert_eq!(load(&previous), Some(first.clone()));
        assert_eq!(load(&path), Some(second));

        // With no file, the copy that exists stays.
        remove(&path).expect("remove");
        rotate(&path, &previous);
        assert_eq!(load(&previous), Some(first));
    }

    #[test]
    fn a_removed_file_is_gone() {
        let scratch = Scratch::new("evening-store-remove");
        let path = scratch.path().join("evening.json");
        write(&path, &an_evening()).expect("write");
        remove(&path).expect("remove");
        assert!(!path.exists());
    }
}
