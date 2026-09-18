//! Which of several files of one song to play first.
//!
//! A corpus holds four to eight files of one song and the suitability ties across most of them —
//! 89% of the groups [`crate::dupes`] finds have their top suitability tied. What separates two
//! copies is finer than the 0–10: how the words are marked, whether they land on the music, how
//! much arrangement there is behind them, and whether the text was decoded or guessed at. All of
//! that is already written down by a scan, in the four components the suitability was summed from
//! and in three columns beside them, so ordering a handful of ticked songs reads no files.
//!
//! **The number this produces is a position, not a rating.** Nothing here is stored, nothing is a
//! second quantity beside the suitability, and nothing is called a score. See
//! `A quality hint is a position on the row, and it is rubbed out rather than kept` in
//! `docs/decisions/curation.md`.

/// What decides one song's place in a quality hint.
///
/// Every field is a column a scan wrote. `None` means *no scan has said*, which every key below
/// treats as the worst value it could hold — the same answer `NULLS LAST` gives a descending sort,
/// and the honest one: a row whose analysis is missing is not a row to play first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// Content hash, which is also the last tie-break.
    pub id: String,
    /// The automatic suitability, 0 to 10.
    pub suitability: Option<u8>,
    /// The lyrics component of it, 0 to 3.
    pub lyrics: Option<u8>,
    /// The sync component of it, 0 to 3.
    pub sync: Option<u8>,
    /// The arrangement component of it, 0 to 2.
    pub arrangement: Option<u8>,
    /// How many channels sound, drums included.
    pub channel_count: Option<u32>,
    /// `declared`, `utf8`, `detected` or `fallback`, as the scan wrote it.
    pub encoding_source: Option<String>,
    /// How many files on disk are byte-identical copies of this.
    pub file_count: u32,
}

/// How much the lyric text can be trusted, best first.
///
/// A declared encoding is somebody's word and valid UTF-8 is a fact about the bytes, so the two are
/// equal and both certain. Detection is a measurement that can be wrong, and the CP1252 fallback is
/// what is left when nothing else applied — which on this corpus is around a third of the files.
/// Of two copies of one song, the one whose words were decoded rather than guessed at is the better
/// copy, whatever the two of them sum to.
fn encoding_rank(source: Option<&str>) -> u8 {
    match source {
        Some("declared" | "utf8") => 3,
        Some("detected") => 2,
        Some("fallback") => 1,
        _ => 0,
    }
}

impl Key {
    /// Everything that decides an order, largest first, as one comparable tuple.
    fn rank(&self) -> (u8, u8, u8, u8, u32, u8, u32) {
        (
            self.suitability.unwrap_or(0),
            self.lyrics.unwrap_or(0),
            self.sync.unwrap_or(0),
            self.arrangement.unwrap_or(0),
            self.channel_count.unwrap_or(0),
            encoding_rank(self.encoding_source.as_deref()),
            self.file_count,
        )
    }
}

/// Orders the keys, best first, and returns their ids.
///
/// **Every key is a column a scan measured, and the rating somebody typed is not one of them.** A
/// rating says how much this song is wanted in a package rather than which copy of it is the better
/// file — `User score` in `docs/decisions/songs.md` is where that is argued — so a copy carrying one
/// would lead its group on an answer to a different question.
///
/// **The suitability first, then the three components that separate two copies of one song.** The
/// lyrics component leads those, because the words as drawn are what a singer is looking at: 3
/// marks where words end, 2 is thin or drawn divided, 1 arrives a line at a time. Sync says whether
/// they land on the music, and the arrangement and its channel count say what is behind them.
///
/// **The melody channel decides nothing here**, as it deducts nothing from the suitability: whether
/// one could be picked out measures the detector and how a file was named, not how the song sings.
/// The row's own column says whether the machine will be able to offer the toggle.
///
/// **The id is last, and it is a promise rather than a preference.** The same ticks give the same
/// order however they arrived, so pressing the button twice does not reshuffle the badges.
pub fn order(mut keys: Vec<Key>) -> Vec<String> {
    keys.sort_by(|a, b| b.rank().cmp(&a.rank()).then_with(|| a.id.cmp(&b.id)));
    keys.into_iter().map(|key| key.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A middling key, so a test can set the one field it is about.
    ///
    /// Every value here leaves room above it, which is what lets one test raise each key in turn
    /// and watch it decide.
    fn key(id: &str) -> Key {
        Key {
            id: id.to_owned(),
            suitability: Some(8),
            lyrics: Some(2),
            sync: Some(2),
            arrangement: Some(1),
            channel_count: Some(8),
            encoding_source: Some("detected".to_owned()),
            file_count: 1,
        }
    }

    /// One key raised above the middling value [`key`] gives it, named so a failure says which.
    type Decider = (&'static str, fn(&mut Key));

    /// The keys in the order they are asked, each one on its own.
    fn deciders() -> Vec<Decider> {
        vec![
            ("suitability", |k| k.suitability = Some(10)),
            ("lyrics", |k| k.lyrics = Some(3)),
            ("sync", |k| k.sync = Some(3)),
            ("arrangement", |k| k.arrangement = Some(2)),
            ("channels", |k| k.channel_count = Some(12)),
            ("encoding", |k| {
                k.encoding_source = Some("declared".to_owned())
            }),
            ("copies", |k| k.file_count = 4),
        ]
    }

    #[test]
    fn each_key_in_turn_decides_when_everything_before_it_is_equal() {
        for (name, better) in deciders() {
            let mut winner = key("b-loses-on-id");
            better(&mut winner);
            let ordered = order(vec![key("a-wins-on-id"), winner]);
            assert_eq!(ordered[0], "b-loses-on-id", "{name} did not decide");
        }
    }

    #[test]
    fn an_unmeasured_row_sorts_where_the_worst_measured_one_does_and_not_first() {
        let mut unmeasured = key("unmeasured");
        unmeasured.suitability = None;
        unmeasured.lyrics = None;
        unmeasured.sync = None;
        unmeasured.arrangement = None;
        unmeasured.channel_count = None;
        unmeasured.encoding_source = None;
        let ordered = order(vec![unmeasured, key("measured")]);
        assert_eq!(ordered, vec!["measured", "unmeasured"]);
    }

    #[test]
    fn the_same_songs_give_the_same_order_whichever_way_they_arrive() {
        let one = key("aaa");
        let two = key("bbb");
        let three = key("ccc");
        let forwards = order(vec![one.clone(), two.clone(), three.clone()]);
        let backwards = order(vec![three, two, one]);
        assert_eq!(forwards, backwards);
        assert_eq!(forwards, vec!["aaa", "bbb", "ccc"]);
    }

    #[test]
    fn a_guessed_encoding_loses_to_a_declared_one_and_to_valid_utf8() {
        let mut declared = key("declared");
        declared.encoding_source = Some("declared".to_owned());
        let mut utf8 = key("utf8");
        utf8.encoding_source = Some("utf8".to_owned());
        let mut fallback = key("fallback");
        fallback.encoding_source = Some("fallback".to_owned());
        let ordered = order(vec![fallback, utf8, declared]);
        assert_eq!(ordered, vec!["declared", "utf8", "fallback"]);
    }
}
