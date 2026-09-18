//! What a fetched bank is checked against, and the rules that come with the table's own fields.
//!
//! **These are properties of a row, not of anybody's HTTP client**, which is why they are here and
//! not beside a download loop. Two programs fetch a bank now — the machine's `fetch.rs` over
//! `ureq`, on a named thread beside the audio thread, and `km-admin` over `reqwest` on tokio — and
//! the transfer loops are honestly two implementations, because they differ in the two places that
//! matter: cancellation, and where the progress counter lives.
//!
//! What must **not** be two implementations is any of this. A second copy of the rule that decides
//! *which* hash a row is pinned with is a download that verifies against the wrong number, and
//! silently installs a file nobody checked — which is the hazard the table's own header opens with.

use std::path::{Path, PathBuf};

/// How far past the table's exact `bytes` a server may claim before the download is refused unread.
///
/// **A sanity check and not the real one.** The digest is what actually decides; this only stops an
/// hour being spent on something that cannot possibly be the file — a login page, an error document
/// or a redirect to something else entirely. Twice the expected size is deliberately generous: a
/// row's `bytes` is exact, so anything within a factor of two earns a download and a hash.
pub const SIZE_SLACK: u64 = 2;

/// Whether a size a server claims is close enough to the table's to be worth downloading.
///
/// `expected == 0` means the table did not say, and then nothing can be concluded — answer yes and
/// let the digest decide.
pub fn size_is_plausible(claimed: u64, expected: u64) -> bool {
    expected == 0 || claimed <= expected.saturating_mul(SIZE_SLACK)
}

/// Where a part-finished download lives.
///
/// Hidden and named after the bank, so an interrupted fetch leaves something obviously incomplete
/// rather than a short `.sf2` that would load and sound wrong.
pub fn part_name(name: &str) -> PathBuf {
    PathBuf::from(format!(".{name}.part"))
}

/// Whichever digest the table pinned this row with.
///
/// Two, because six rows are pinned by the sha1 archive.org publishes in its own item metadata
/// rather than by a sha256 this project computed — see the bank table's header for why that is a
/// deliberate choice and not a compromise.
pub enum Hasher {
    /// The ordinary case: a sha256 this repository computed.
    Sha256(Box<sha2::Sha256>),
    /// A `sha1:`-prefixed digest, taken from the publisher's own metadata.
    Sha1(Box<sha1::Sha1>),
    /// No digest to check. Only reachable for an archive whose row pins the member instead.
    None,
}

impl Hasher {
    /// Picks the hash the digest string asks for.
    ///
    /// **The prefix is the whole of the rule**: `sha1:` means sha1, an empty string means nothing to
    /// check, and anything else is a sha256. Reading it any other way — by length, say — would
    /// accept a truncated digest as a different algorithm.
    pub fn for_digest(digest: &str) -> Self {
        use sha1::Digest as _;
        if digest.is_empty() {
            Hasher::None
        } else if digest.starts_with("sha1:") {
            Hasher::Sha1(Box::new(sha1::Sha1::new()))
        } else {
            Hasher::Sha256(Box::new(sha2::Sha256::new()))
        }
    }

    /// Feeds the next chunk in.
    pub fn update(&mut self, bytes: &[u8]) {
        use sha1::Digest as _;
        match self {
            Hasher::Sha256(hasher) => hasher.update(bytes),
            Hasher::Sha1(hasher) => hasher.update(bytes),
            Hasher::None => {}
        }
    }

    /// The digest as hex, or an empty string where there was nothing to check.
    pub fn finish(self) -> String {
        use sha1::Digest as _;
        match self {
            Hasher::Sha256(hasher) => hex(&hasher.finalize()),
            Hasher::Sha1(hasher) => hex(&hasher.finalize()),
            Hasher::None => String::new(),
        }
    }
}

/// What a row pinned, with the `sha1:` prefix taken off — the form a computed digest is compared to.
pub fn wanted(pinned: &str) -> &str {
    pinned.strip_prefix("sha1:").unwrap_or(pinned)
}

/// Whether a computed digest matches what a row pinned.
///
/// **Case-insensitive on purpose.** Hex is hex, and a table that spelled one row's digest in capitals
/// would otherwise reject a download that was perfectly correct.
pub fn matches(computed: &str, pinned: &str) -> bool {
    computed.eq_ignore_ascii_case(wanted(pinned))
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Takes one member out of a zip and checks its digest.
///
/// **Which digest is checked is the subtle part, and it is the table's decision rather than this
/// function's.** A row that publishes a bank inside an archive carries up to two: `archive_digest`
/// for the download, and `digest` for the member. Where the row gives both, the archive is verified
/// first and this checks the member as well; where it gives only the member's, that is the only
/// thing checked and the archive is trusted no further than the file it yields.
pub fn extract_member(
    archive: &Path,
    member: &str,
    out: &Path,
    digest: &str,
    chunk: usize,
) -> Result<(), String> {
    use std::io::{Read as _, Write as _};

    let file = std::fs::File::open(archive)
        .map_err(|error| format!("{} cannot be read: {error}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|error| format!("{} is not a zip: {error}", archive.display()))?;
    let mut entry = zip
        .by_name(member)
        .map_err(|error| format!("{member} is not in the archive: {error}"))?;

    let written = std::fs::File::create(out)
        .map_err(|error| format!("{} cannot be written: {error}", out.display()))?;
    let mut writer = std::io::BufWriter::new(written);
    let mut hasher = Hasher::for_digest(digest);
    let mut buffer = vec![0u8; chunk];
    loop {
        let read = entry
            .read(&mut buffer)
            .map_err(|error| format!("could not read {member}: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    }
    writer
        .flush()
        .map_err(|error| format!("could not write {}: {error}", out.display()))?;

    if !digest.is_empty() && !matches(&hasher.finish(), digest) {
        let _ = std::fs::remove_file(out);
        return Err(format!(
            "{member} does not match the digest this bank is pinned to. Nothing was installed."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_is_read_as_the_kind_the_table_pinned_it_with() {
        // sha256 of the empty input, which is the one value both algorithms can be told apart by
        // without a fixture.
        let mut sha256 =
            Hasher::for_digest("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        sha256.update(b"");
        assert_eq!(
            sha256.finish(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let mut sha1 = Hasher::for_digest("sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709");
        sha1.update(b"");
        assert_eq!(sha1.finish(), "da39a3ee5e6b4b0d3255bfef95601890afd80709");

        // An empty pin hashes nothing, which is the archive case where the member carries the check.
        let mut none = Hasher::for_digest("");
        none.update(b"anything");
        assert_eq!(none.finish(), "");
    }

    #[test]
    fn the_sha1_prefix_comes_off_before_a_comparison() {
        assert!(matches("abc", "sha1:abc"));
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "sha1:abd"));
        assert_eq!(wanted("sha1:abc"), "abc");
        assert_eq!(wanted("abc"), "abc");
    }

    #[test]
    fn hex_case_does_not_decide_whether_a_download_was_correct() {
        assert!(matches("ABCDEF", "abcdef"));
        assert!(matches("abcdef", "sha1:ABCDEF"));
    }

    #[test]
    fn a_part_file_is_hidden_and_named_after_the_bank() {
        // It sits in the same folder the picker reads, so it must not look like a bank: the listing
        // takes `.sf2` only, and this ends in `.part`.
        let part = part_name("Roland SC-55 v3.7.sf2");
        assert_eq!(part.to_str(), Some(".Roland SC-55 v3.7.sf2.part"));
        assert!(!part.to_string_lossy().ends_with(".sf2"));
    }

    #[test]
    fn a_wildly_bigger_body_than_the_table_says_is_refused_but_a_close_one_is_not() {
        assert!(size_is_plausible(1_000, 1_000));
        assert!(size_is_plausible(1_999, 1_000));
        assert!(!size_is_plausible(2_001, 1_000));
        // Nothing can be concluded when the table did not say, so the digest is left to decide.
        assert!(size_is_plausible(u64::MAX, 0));
    }
}
