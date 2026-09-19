//! Finding a song by the words it sings rather than by what it is called.
//!
//! **The loose reading of the signal [`crate::dupes::lyric_key`] reads strictly.** That key hashes a
//! song's words and joins the files whose words are the same, which is exact: one verse missing, one
//! sequencer's transcription against another's, one line of spelling, and the hash differs and the
//! pass sees nothing. The names cannot rescue those pairs either — the files this reaches are the
//! ones called `EARTHW~2`. So the same words, compared by how much of them two songs share.
//!
//! **Tight, and that is the difference from [`crate::similar`].** A similar name is a reason to look
//! at a file; a shared lyric body is nearly a statement that two files are one song, so the threshold
//! sits where the same recording is and not where a resemblance starts.
//!
//! Two halves, as in [`crate::similar`]: [`match_query`] is the FTS5 expression that gathers
//! candidates cheaply out of `lyrics_fts`, and [`Shingles::likeness`] is the score that orders them,
//! done outside SQLite because no tokenizer there compares one body of words with another. Nothing
//! here reads or writes a database. See `Finding a song by the words it sings` in
//! `docs/decisions/curation.md`.
//!
//! **A folded word is a word the index holds**, which is what lets a phrase be built here and
//! answered there. [`km_song::text::fold`] is deliberately the same folding as the
//! `unicode61 remove_diacritics 2` tokenizer both FTS tables use, and `km-catalog` has a test that
//! fails if the two ever drift apart.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// How many words in a row make one unit of comparison.
///
/// **Three, because the two neighbours are what carry the order.** One word is a bag that any song in
/// the same language fills — half this corpus sings *love* — and the longer the run the more of it a
/// single stray syllable destroys, since one changed word spoils [`SHINGLE`] of them on each side.
pub const SHINGLE: usize = 3;

/// The lowest likeness a match is shown at.
///
/// **Measured, not chosen**, by `db::measure::where_the_same_words_threshold_sits` over a real
/// corpus. Two populations it can take from that corpus: 2,594 pairs the duplicate pass joined by a
/// matching shape *and* a matching name whose lyric keys differ, which are one recording typed twice,
/// against 10,934 pairs of songs it joined to nothing.
///
/// | pairs | 5th | 50th | 95th |
/// |---|---|---|---|
/// | one recording, typed twice | 0.000 | 0.788 | 0.978 |
/// | joined to nothing | 0.000 | 0.000 | 0.008 |
///
/// **The gap is the whole story, and it is a cliff rather than a slope.** A coincidence's 95th
/// percentile is 0.008 — seventy-five times below this — so where the line falls between the two
/// populations barely moves what gets through: 0.7% of the unrelated pairs clear this and 0.9% clear
/// 0.40. What moves is how much of a true pair is kept, which is 71.2% here and 48.6% at 0.80.
///
/// So it sits low in the gap rather than high in it, and the arithmetic says why a true pair needs
/// the room. A file missing one verse of five shares four and holds five, which is 0.8; missing two
/// is 0.6; a transcription differing in a twentieth of its words breaks [`SHINGLE`] runs for each
/// word it changes, which takes a pair that would have been 1.0 to about 0.74. The two compound,
/// because the file with a verse gone is usually also the file somebody else typed.
///
/// **What it cannot reach is not below the line, it is at zero.** A fifth of the true pairs share no
/// run at all, which is where the two files were typed by people who broke the words differently —
/// no threshold recovers those, and lowering this one to 0.40 buys 7.6 points of them and no more.
///
/// A cover with reworked verses keeps its chorus and little else, which is well under this. A medley
/// holding a whole short song pays for every other song in it, because the union is the whole medley.
pub const THRESHOLD: f32 = 0.6;

/// How many matches a search shows at most.
///
/// [`crate::similar::SHOWN`]'s number, for its reason: past a hundred, a list ordered by likeness is
/// coincidence. It is reached even less often here, because a song is filed under a handful of names
/// and not a hundred.
pub const SHOWN: usize = 100;

/// How many candidates the index is asked for before they are scored.
pub const CANDIDATES: u32 = 400;

/// How many phrases a search asks the index for.
///
/// **Enough that a file missing any one part of the song still matches several.** They are spread
/// across the whole body rather than taken from its opening, because the first thing a shortened
/// file loses is its first verse and the last thing it loses is its chorus.
pub const PROBES: usize = 12;

/// One lyric's word-runs, made once and set against many.
///
/// **Hashed rather than held as text.** Nothing ever reads a run back, a long lyric is thousands of
/// them, and a search scores [`CANDIDATES`] lyrics against one. Nothing is written down either, so
/// which hasher this is does not have to hold still between builds — unlike
/// [`crate::dupes::lyric_key`], whose value is compared with one another run wrote.
#[derive(Debug, Clone)]
pub struct Shingles(HashSet<u64>);

impl Shingles {
    /// Every run of [`SHINGLE`] words the song sings, or `None` when it has too few words to compare.
    ///
    /// **A set and not a tally.** A chorus sung four times is one run, so a file that repeats it three
    /// times and a file that repeats it five are not held apart by the repetition — which is a
    /// difference in how a sequencer wrote the file out and never a difference in the song.
    ///
    /// Built from the flat word list and not from [`crate::dupes::sung_runs`], because one file wraps
    /// its lines where another does not and a comparison that noticed would punish the same song for
    /// being typed differently.
    pub fn of(lyrics: &str) -> Option<Self> {
        let words = crate::dupes::sung_words(lyrics)?;
        Some(Self(words.windows(SHINGLE).map(hash_of).collect()))
    }

    /// How much of two lyric bodies is word for word the same, from 0 to 1.
    ///
    /// The share of all the runs either song has that both of them have, which is what makes an extra
    /// verse cost something — the reason `similar::name_likeness` divides by the longer of two names.
    ///
    /// **Not the share of the shorter side.** That reading scores a medley as holding a whole song at
    /// 100%, and a file carrying only a first verse as being the whole of it. Both answer *is this the
    /// same recording?* with *yes* where the honest answer is *part of it is*, and both are what this
    /// page exists to refuse.
    pub fn likeness(&self, other: &Self) -> f32 {
        if self.0.is_empty() || other.0.is_empty() {
            return 0.0;
        }
        let shared = self.0.intersection(&other.0).count();
        // Inclusion-exclusion rather than building the union, which would allocate to count what two
        // lengths and the intersection already say.
        let union = self.0.len() + other.0.len() - shared;
        shared as f32 / union as f32
    }
}

/// How alike two lyric bodies are, from 0 to 1, when one of them is only ever read once.
///
/// Zero when the other song has too few words to say anything, which is not the same as two songs
/// having nothing in common and is not worth showing either.
pub fn likeness(mine: &Shingles, other: &str) -> f32 {
    Shingles::of(other).map_or(0.0, |other| mine.likeness(&other))
}

/// The phrases this lyric could be looked for by, in the order it sings them.
///
/// Each is [`SHINGLE`] folded words that lie inside one run of kept lines — see
/// [`crate::dupes::sung_runs`] for why a phrase may not cross a line the banner rule dropped.
///
/// **A phrase the song repeats is offered once.** A chorus comes round three times and says no more
/// the third time than the first, and keeping the repeats would spend the whole sample on the one
/// part of the song a *different* song is most likely to share.
pub fn probes(lyrics: &str) -> Vec<Vec<String>> {
    let runs = crate::dupes::sung_runs(lyrics);
    // The same floor [`Shingles::of`] answers `None` at, asked here too so this is honest read on its
    // own: a song with too few words to be compared has no phrase worth asking the index for either.
    if runs.iter().map(Vec::len).sum::<usize>() < crate::dupes::MIN_LYRIC_WORDS {
        return Vec::new();
    }
    let mut seen: HashSet<u64> = HashSet::new();
    runs.iter()
        .flat_map(|run| run.windows(SHINGLE))
        .filter(|run| seen.insert(hash_of(run)))
        .map(|run| run.to_vec())
        .collect()
}

/// Every distinct word the probes are built from, which is what a caller asks the index about.
pub fn probe_words(probes: &[Vec<String>]) -> HashSet<String> {
    probes.iter().flatten().cloned().collect()
}

/// The FTS5 expression that gathers candidates, or `None` when the lyric offers no phrase.
///
/// [`PROBES`] phrases ORed as FTS5 phrase terms: `"quiet nights of" OR "stars and guitars" OR …`.
///
/// **Phrases and not words.** A song's words ORed one at a time match most of the corpus, for the
/// reason the Lyrics page's own search gives: half of it sings *love*. A three-word phrase is
/// specific enough that the files it matches are nearly all worth scoring.
///
/// **Spread first, then rarest.** The probes are cut into [`PROBES`] equal stretches and one is taken
/// from each, so a file missing the opening verse still matches on the ones after it. Within a stretch
/// the phrase kept is the one whose rarest word the fewest songs hold, because a phrase is only as
/// selective as its rarest word — and `ORDER BY bm25` ranks every row a query matches before any
/// `LIMIT` can cut one, so a dozen phrases of common words make SQLite score a large part of the
/// corpus to answer one page. `held_by` is what the caller read out of `lyrics_vocab`; a word it does
/// not name is held by nothing, which is as selective as a word can be.
pub fn match_query(probes: &[Vec<String>], held_by: &HashMap<String, u64>) -> Option<String> {
    if probes.is_empty() {
        return None;
    }
    let rarest = |phrase: &Vec<String>| -> u64 {
        phrase
            .iter()
            .map(|word| held_by.get(word).copied().unwrap_or(0))
            .min()
            .unwrap_or(0)
    };

    let wanted = PROBES.min(probes.len());
    let mut terms: Vec<String> = Vec::new();
    for nth in 0..wanted {
        // Integer arithmetic, so the stretches cover the song exactly and none is ever empty.
        let start = nth * probes.len() / wanted;
        let end = (nth + 1) * probes.len() / wanted;
        let Some(phrase) = probes[start..end]
            .iter()
            .min_by_key(|phrase| rarest(phrase))
        else {
            continue;
        };
        // Folding leaves letters, digits and spaces, so no `"` can be in a word; this is what keeps
        // it that way if folding changes.
        let term = format!("\"{}\"", phrase.join(" ").replace('"', ""));
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

fn hash_of(run: &[String]) -> u64 {
    let mut hasher = DefaultHasher::new();
    run.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verses enough to clear [`crate::dupes::MIN_LYRIC_WORDS`], in lines the banner rule keeps.
    fn verses(count: usize) -> String {
        (0..count)
            .map(|nth| {
                format!(
                    "she walked in from the rain number {nth}\n\
                     and nobody knew her name at all {nth}\n\
                     the band played on until the morning {nth}"
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The whole query for a lyric, with nothing known about how rare any word is.
    fn query_for(lyrics: &str) -> Option<String> {
        match_query(&probes(lyrics), &HashMap::new())
    }

    #[test]
    fn a_song_is_alike_to_itself() {
        let song = verses(5);
        let mine = Shingles::of(&song).expect("enough words");
        assert_eq!(likeness(&mine, &song), 1.0);
    }

    #[test]
    fn two_songs_sharing_nothing_are_not_alike() {
        let mine = Shingles::of(&verses(5)).expect("enough words");
        let other: String = (0..5)
            .map(|nth| format!("a completely different set of words entirely here {nth}\n"))
            .collect();
        assert!(
            likeness(&mine, &other) < 0.05,
            "unrelated songs sit near zero"
        );
    }

    #[test]
    fn a_missing_verse_stays_above_the_threshold() {
        let mine = Shingles::of(&verses(5)).expect("enough words");
        let likeness = likeness(&mine, &verses(4));
        assert!(
            likeness >= THRESHOLD,
            "one verse of five gone is still the same song, got {likeness}"
        );
    }

    #[test]
    fn a_shared_chorus_is_not_the_same_song() {
        // Two songs that sing one chorus and nothing else in common, which is a cover's shape and the
        // thing the threshold is set to refuse.
        let chorus = "and we danced until the morning came around again tonight";
        let mine = Shingles::of(&format!("{}\n{chorus}", verses(4))).expect("enough words");
        let other: String = (0..4)
            .map(|nth| format!("a wholly unrelated line of words goes here {nth}\n"))
            .collect();
        let likeness = likeness(&mine, &format!("{other}\n{chorus}"));
        assert!(
            likeness < THRESHOLD,
            "one shared chorus is not one song, got {likeness}"
        );
    }

    #[test]
    fn a_medley_does_not_contain_the_songs_in_it() {
        // The case that rules out dividing by the shorter side: a medley holds the whole of a short
        // song, so containment would call it a perfect match.
        let song = verses(2);
        let medley = format!("{song}\n{}", verses(9));
        let mine = Shingles::of(&song).expect("enough words");
        let likeness = likeness(&mine, &medley);
        assert!(
            likeness < THRESHOLD,
            "a medley is not the song it holds, got {likeness}"
        );
    }

    #[test]
    fn a_song_with_too_few_words_is_not_compared() {
        assert!(Shingles::of("la la la").is_none());
        assert!(query_for("la la la").is_none());
    }

    #[test]
    fn a_banner_is_not_compared_by() {
        // The same song, one file carrying the sequencer's card above it. Both sides drop it, so the
        // two still meet — the property `sung_words` exists for.
        let plain = verses(5);
        let carded = format!("Sequenced by Somebody, 123 Any Street, Anytown\n{plain}");
        let mine = Shingles::of(&plain).expect("enough words");
        assert_eq!(likeness(&mine, &carded), 1.0);
    }

    #[test]
    fn a_line_break_costs_nothing() {
        // One file wraps where another does not. The words are the same, so the score must be.
        let wrapped = verses(5);
        let unwrapped = wrapped.replace('\n', " ");
        let mine = Shingles::of(&wrapped).expect("enough words");
        assert_eq!(likeness(&mine, &unwrapped), 1.0);
    }

    #[test]
    fn no_phrase_crosses_a_dropped_line() {
        // The seam that matches nothing: `lyrics_fts` holds the banner's words, so the word before it
        // and the word after it are not next to each other there.
        let lyrics = format!(
            "she walked in from the rain\n\
             Sequenced by Somebody, 123 Any Street, Anytown\n\
             {}",
            verses(4)
        );
        let phrases = probes(&lyrics);
        assert!(!phrases.is_empty());
        for phrase in &phrases {
            let joined = phrase.join(" ");
            assert!(
                !joined.contains("rain she"),
                "a phrase spanned the dropped line: {joined}"
            );
        }
    }

    #[test]
    fn the_query_is_phrases_the_index_can_answer() {
        let query = query_for(&verses(5)).expect("enough words");
        assert!(query.starts_with('"'), "{query}");
        assert_eq!(
            query.matches(" OR ").count(),
            PROBES - 1,
            "a song this long has a phrase to spare for each stretch: {query}"
        );
        for term in query.split(" OR ") {
            assert_eq!(term.trim_matches('"').split(' ').count(), SHINGLE, "{term}");
        }
    }

    #[test]
    fn the_phrases_are_spread_across_the_song() {
        // A file that lost its first verse still has to match several of them, which is only true if
        // they are not all taken from the opening.
        let query = query_for(&verses(5)).expect("enough words");
        let tail = verses(5);
        let tail: String = tail.lines().skip(3).collect::<Vec<_>>().join(" ");
        let still_there = query
            .split(" OR ")
            .filter(|term| tail.contains(term.trim_matches('"')))
            .count();
        assert!(
            still_there >= PROBES / 2,
            "a file missing its first verse matched only {still_there} phrases"
        );
    }

    #[test]
    fn a_repeated_chorus_is_asked_for_once() {
        let chorus = "and the band played on and on forever more tonight";
        let with_repeats = format!("{}\n{chorus}\n{chorus}\n{chorus}", verses(4));
        let query = query_for(&with_repeats).expect("enough words");
        let terms: Vec<&str> = query.split(" OR ").collect();
        let unique: HashSet<&str> = terms.iter().copied().collect();
        assert_eq!(terms.len(), unique.len(), "{query}");
    }

    #[test]
    fn the_rarest_phrase_of_a_stretch_is_the_one_asked_for() {
        // Every word distinct, so each phrase can be reasoned about on its own.
        let words: Vec<String> = (0..30).map(|nth| format!("word{nth}")).collect();
        let phrases = probes(&words.join(" "));
        // The first stretch holds exactly these two, and the second one is the rarer.
        assert_eq!(phrases[0].join(" "), "word0 word1 word2");
        assert_eq!(phrases[1].join(" "), "word1 word2 word3");

        let held_by: HashMap<String, u64> = probe_words(&phrases)
            .into_iter()
            .map(|word| {
                let rare = word == "word3";
                (word, if rare { 1 } else { 9_000 })
            })
            .collect();
        let query = match_query(&phrases, &held_by).expect("enough words");
        assert!(
            query.starts_with("\"word1 word2 word3\""),
            "the stretch took its commoner phrase: {query}"
        );
    }

    #[test]
    fn a_stretch_of_equally_common_phrases_takes_its_first() {
        // Nothing known about any word, which is what an index with no answer for them looks like.
        // The spread is then the only thing choosing, and it has to stay put.
        let words: Vec<String> = (0..30).map(|nth| format!("word{nth}")).collect();
        let query = query_for(&words.join(" ")).expect("enough words");
        assert!(query.starts_with("\"word0 word1 word2\""), "{query}");
    }

    #[test]
    fn a_short_song_asks_for_every_stretch_it_has() {
        // Just over the floor, so it has fewer phrases than `PROBES` and must not ask for one twice.
        let words: Vec<String> = (0..crate::dupes::MIN_LYRIC_WORDS)
            .map(|nth| format!("word{nth}"))
            .collect();
        let query = query_for(&words.join(" ")).expect("just over the floor");
        let terms: Vec<&str> = query.split(" OR ").collect();
        let unique: HashSet<&&str> = terms.iter().collect();
        assert_eq!(terms.len(), unique.len(), "{query}");
        assert!(terms.len() <= PROBES);
    }
}
