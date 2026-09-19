//! Finding songs that are probably the same recording.
//!
//! Exact duplicates need nothing here: a song's id *is* the hash of its bytes, so byte-identical
//! files land on one row without any pass to run. What this module does is the harder half — the
//! corpus has the same song re-saved by a different sequencer, re-encoded, or trimmed by a few
//! milliseconds, and those are different bytes and therefore different songs.
//!
//! **Nothing here merges anything.** It produces suggestions with a reason attached, which a person
//! confirms or dismisses. That is deliberate: a wrong merge silently hides a song somebody wanted,
//! and unlike a missed duplicate it leaves no trace to notice later.

use std::collections::HashMap;

use km_song::Song;

use crate::db::Fingerprint;

/// Below this, a pair is not worth a person's time.
const THRESHOLD: f32 = 0.72;

/// Durations closer than this are treated as the same length.
const DURATION_TOLERANCE: f32 = 0.04;

/// How many pairs a single song may be suggested in.
///
/// A generic title like "Instrumental" would otherwise pair with hundreds of others and bury
/// everything else in the review queue.
const MAX_PAIRS_PER_SONG: usize = 8;

/// A song's structural signature: what it is made of, independent of how it was saved.
///
/// Deliberately coarse. Two files of the same recording differ in their byte layout, their track
/// names, their embedded credits and often their tempo map, but they have the same number of notes
/// in roughly the same channels for roughly the same length. Anything finer would stop matching the
/// re-saved copies this exists to catch.
pub fn fingerprint(song: &Song) -> String {
    let channels: Vec<String> = song.sounding_channels().iter().map(u8::to_string).collect();
    // Note count in buckets of 16 so a copy with a couple of stray notes still matches.
    let notes = song.note_count() / 16;
    // Duration to the nearest two seconds, for the same reason.
    let seconds = song.duration_ms() / 2000;
    format!("{notes}:{seconds}:{}", channels.join(","))
}

/// Below this many words, a lyric says too little to identify a song by.
///
/// **Measured, not chosen.** With the credit lines already gone, the false groups left on the real
/// corpus were legal boilerplate — *all rights reserved, not for broadcast* — of 17 and 20 words,
/// while every true group had 90 or more. The gap is wide and the floor sits in it.
pub const MIN_LYRIC_WORDS: usize = 25;

/// The words a song is identified by: what it sings, folded, or `None` when there are too few to
/// matter.
///
/// **The one signal here that needs no name**, which is the whole reason it exists beside
/// [`fingerprint`]. A shape match has to be confirmed by a title or an artist — see [`compare`] —
/// and a great many files in a real corpus are called `EARTHW~2` or `TRACK01`. Two files that sing
/// the same words are the same song whatever they are called.
///
/// **Dropping what is not sung is load-bearing rather than tidy.** A great many files carry the
/// sequencer's business card in the lyric track and nothing else; keyed as sung words, one such card
/// made a single group of 126 unrelated songs on the real corpus, and the commercial discs' *ALL
/// rights reserved. NOT FOR RENTAL.* made another of 55.
///
/// [`km_song::looks_like_a_banner`] is what already knows both, and is borrowed rather than restated.
/// It decides whether a leading line is fit to be a package's preview, which is the same question
/// asked one line at a time: an address, a credit in either language, the legal boilerplate — with
/// the continuation for where that sentence wraps — and a section label nobody sings.
///
/// **A false positive costs a key nothing**, which is what makes the wider rule safe here where a
/// narrower one is wanted elsewhere: both files are folded by the same rule, so a line dropped from
/// one is dropped from the other and the two still meet. [`MIN_LYRIC_WORDS`] is what a false positive
/// can reach, and a floor is the safe direction to be wrong in.
///
/// Folded with [`km_song::text::fold`], the alphabet the whole product orders and searches by, so
/// `Coração` and `CORACAO` come out alike — and the same folding the `unicode61 remove_diacritics 2`
/// tokenizer does, so a word here is a word `lyrics_fts` holds.
///
/// **One rule, two readings of it.** [`lyric_key`] hashes these words and asks whether two songs sing
/// exactly the same ones; [`crate::lyric_likeness`] asks how much of them two songs share. Restating
/// the rule for the second would let the loose reading drift from the strict one, and a page saying a
/// pair is alike while the pass says it is not is worse than either answer.
pub fn sung_words(lyrics: &str) -> Option<Vec<String>> {
    let sung: Vec<&str> = lyrics
        .lines()
        .filter(|line| !km_song::looks_like_a_banner(line))
        .collect();
    let folded = km_song::text::fold(&sung.join(" "));
    // `fold` collapses runs and trims, so splitting on the gaps is splitting into words.
    let words: Vec<String> = folded
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    (words.len() >= MIN_LYRIC_WORDS).then_some(words)
}

/// The same words, in runs of lines the banner rule kept next to each other.
///
/// **A run ends where a line was dropped, and that boundary is load-bearing for anything asking the
/// search index a question.** `lyrics_fts` indexes the whole `lyrics` column, banners included, so the
/// last word before a dropped line and the first word after it are *not* next to each other in the
/// index however adjacent they are here. A phrase built across that seam asks for something no
/// document holds and quietly matches nothing.
///
/// **[`sung_words`] folds the whole lyric at once and this folds a line at a time**, which is the
/// same words either way — the only thing a line boundary contributes is a space, and folding
/// collapses it on both sides. They are written separately because one of them is read once per page
/// and the other once per song in a pass over the whole corpus, and the cheaper shape belongs to the
/// one that is read a corpus at a time.
pub fn sung_runs(lyrics: &str) -> Vec<Vec<String>> {
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut run: Vec<String> = Vec::new();
    for line in lyrics.lines() {
        if km_song::looks_like_a_banner(line) {
            if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
            continue;
        }
        // `fold` collapses runs and trims, so splitting on the gaps is splitting into words.
        run.extend(
            km_song::text::fold(line)
                .split(' ')
                .filter(|word| !word.is_empty())
                .map(str::to_owned),
        );
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

/// A song's words, as a key two files can be compared by, or `None` when it has too few to matter.
///
/// Hashed with [`km_kmpkg::content_hash`], whose own note says it identifies duplicates and is not a
/// security boundary — one hasher in this workspace, not two. What counts as a word is
/// [`sung_words`], which carries the reasoning.
pub fn lyric_key(lyrics: &str) -> Option<String> {
    let words = sung_words(lyrics)?;
    Some(km_kmpkg::content_hash(words.join(" ").as_bytes()))
}

/// Suggests near-duplicate pairs from every song's fingerprint and names, and from its words.
///
/// Returns `(a_id, b_id, similarity, reason)` with `a_id < b_id`, so a pair is only ever stored once
/// and the survivor of a merge is stable rather than depending on iteration order.
pub fn suggest(songs: &[Fingerprint]) -> Vec<(String, String, f32, String)> {
    // Candidates are only ever compared inside a bucket. Comparing all pairs in a whole corpus is
    // 180 billion comparisons; bucketing by structure makes it linear in practice, and two files of
    // the same recording share a bucket by construction.
    let mut buckets: HashMap<&str, Vec<&Fingerprint>> = HashMap::new();
    for song in songs {
        if song.fingerprint.is_empty() {
            continue;
        }
        buckets
            .entry(song.fingerprint.as_str())
            .or_default()
            .push(song);
    }

    let mut pairs = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();

    for bucket in buckets.values() {
        // A bucket that large is a fingerprint that says nothing — an empty file, or a very short
        // one — and pairing everything in it would be noise, not a finding.
        if bucket.len() < 2 || bucket.len() > 64 {
            continue;
        }
        for (i, a) in bucket.iter().enumerate() {
            for b in bucket.iter().skip(i + 1) {
                let (first, second) = if a.id <= b.id { (*a, *b) } else { (*b, *a) };
                if counts.get(first.id.as_str()).copied().unwrap_or(0) >= MAX_PAIRS_PER_SONG
                    || counts.get(second.id.as_str()).copied().unwrap_or(0) >= MAX_PAIRS_PER_SONG
                {
                    continue;
                }
                let Some((similarity, reason)) = compare(first, second) else {
                    continue;
                };
                *counts.entry(first.id.as_str()).or_default() += 1;
                *counts.entry(second.id.as_str()).or_default() += 1;
                pairs.push((first.id.clone(), second.id.clone(), similarity, reason));
            }
        }
    }

    pairs.extend(same_words(songs));
    pairs.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    pairs
}

/// Pairs every set of songs that sing the same words.
///
/// **A name is not asked for, and that is the entire point.** [`compare`] refuses a shape match that
/// no title or artist confirms, because a corpus built from one studio's template would otherwise
/// pair with itself — and the cost of that rule is the files whose names say nothing, which is a
/// great many of them. Two files with the same words need no name: `EARTHW~2` and `FANTAZY` are one
/// song, and only their lyrics can say so.
///
/// **A star and not a clique.** The first song of a set is paired with each of the others, so a set
/// of fifteen is fourteen pairs rather than a hundred and five. Grouping joins them either way — see
/// `Db::cluster` — and the smaller shape is what lets somebody dismiss one file out of a set: a
/// clique reconnects the two it separated through any third member, so a dismissal there does not
/// survive the next pass.
///
/// [`MAX_PAIRS_PER_SONG`] is deliberately not applied. It keeps a review queue readable and there is
/// no review queue; the star is already bounded by the set.
fn same_words(songs: &[Fingerprint]) -> Vec<(String, String, f32, String)> {
    let mut buckets: HashMap<&str, Vec<&Fingerprint>> = HashMap::new();
    for song in songs {
        let Some(key) = song.lyric_key.as_deref() else {
            continue;
        };
        buckets.entry(key).or_default().push(song);
    }

    let mut pairs = Vec::new();
    for bucket in buckets.values_mut() {
        // The same guard the shape pass states: a key shared by that many files says nothing,
        // however it was arrived at. Nothing on the real corpus comes close once the credit lines
        // are gone, which is what makes this a limit rather than a filter.
        if bucket.len() < 2 || bucket.len() > 64 {
            continue;
        }
        // Sorted, so the centre of the star is the same song on every run and the pairs it writes
        // are stable — the rule the canonical `a_id < b_id` ordering below follows for its reason.
        bucket.sort_by(|a, b| a.id.cmp(&b.id));
        let (centre, rest) = bucket.split_first().expect("a bucket of two or more");
        for other in rest {
            // 1.0 rather than a score: this is an exact match on a folded value, so there is no
            // similarity to report and nothing a threshold could usefully do to it.
            pairs.push((
                centre.id.clone(),
                other.id.clone(),
                1.0,
                "same words".to_owned(),
            ));
        }
    }
    pairs
}

/// Scores one pair, or rejects it.
fn compare(a: &Fingerprint, b: &Fingerprint) -> Option<(f32, String)> {
    if !similar_duration(a.duration_ms, b.duration_ms) {
        return None;
    }

    let a_title = normalize(&a.title);
    let b_title = normalize(&b.title);
    let a_artist = normalize(&a.artist);
    let b_artist = normalize(&b.artist);

    // The structural fingerprint already matched -- that is what put them in the same bucket -- so
    // it is worth a real share of the score on its own.
    let mut score = 0.45f32;
    let mut reasons = vec!["same shape".to_owned()];

    if !a_title.is_empty() && a_title == b_title {
        score += 0.40;
        reasons.push("same title".to_owned());
    } else if !a_title.is_empty() && !b_title.is_empty() {
        let overlap = token_overlap(&a_title, &b_title);
        if overlap >= 0.6 {
            score += 0.30 * overlap;
            reasons.push("similar title".to_owned());
        }
    }

    if !a_artist.is_empty() && a_artist == b_artist {
        score += 0.15;
        reasons.push("same artist".to_owned());
    }

    // A shape match with nothing to confirm it is not enough. The corpus is full of files built from
    // the same template, and they are not the same song.
    if reasons.len() == 1 {
        return None;
    }

    let score = score.min(1.0);
    if score < THRESHOLD {
        return None;
    }
    Some((score, reasons.join(", ")))
}

/// Whether two durations are close enough to be the same recording.
fn similar_duration(a: u32, b: u32) -> bool {
    if a == 0 || b == 0 {
        return a == b;
    }
    let longer = a.max(b) as f32;
    let shorter = a.min(b) as f32;
    (longer - shorter) / longer <= DURATION_TOLERANCE
}

/// Case-, accent- and punctuation-folded text, for comparing titles.
///
/// The same folding the FTS tokenizer implies, done outside SQLite because these comparisons happen
/// outside it. It lives in `km-song` rather than here because several crates need it now — this one
/// to group duplicate files, `km-remote-pages` to filter a favorites folder in memory, `km-remote-core` to
/// build the folded sort key its mirror orders by, and the song book to file a song under a letter.
/// Two copies of an accent table is two things to disagree, and the disagreement would look like bad
/// data rather than like a bug.
pub fn normalize(value: &str) -> String {
    km_song::text::fold(value)
}

/// The share of the shorter title's words that also appear in the longer one.
fn token_overlap(a: &str, b: &str) -> f32 {
    let a_tokens: Vec<&str> = a.split(' ').filter(|t| !t.is_empty()).collect();
    let b_tokens: Vec<&str> = b.split(' ').filter(|t| !t.is_empty()).collect();
    if a_tokens.is_empty() || b_tokens.is_empty() {
        return 0.0;
    }
    let shared = a_tokens.iter().filter(|t| b_tokens.contains(t)).count();
    shared as f32 / a_tokens.len().min(b_tokens.len()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print(id: &str, title: &str, artist: &str, duration_ms: u32, shape: &str) -> Fingerprint {
        Fingerprint {
            id: id.to_owned(),
            fingerprint: shape.to_owned(),
            title: title.to_owned(),
            artist: artist.to_owned(),
            duration_ms,
            lyric_key: None,
        }
    }

    /// [`print`], singing the given words. The shape is deliberately unique so nothing here can
    /// pair on it by accident and pass for a lyric match.
    fn sings(id: &str, title: &str, lyrics: &str) -> Fingerprint {
        Fingerprint {
            lyric_key: lyric_key(lyrics),
            ..print(id, title, "", 200_000, id)
        }
    }

    /// Twenty-six words, which is one clear of [`MIN_LYRIC_WORDS`].
    const ENOUGH: &str = "one two three four five six seven eight nine ten eleven twelve thirteen                           fourteen fifteen sixteen seventeen eighteen nineteen twenty twentyone                           twentytwo twentythree twentyfour twentyfive twentysix";

    #[test]
    fn a_business_card_in_the_lyric_track_is_not_a_song() {
        // The real shape, redacted: an address, a site and a telephone number, repeated, and no
        // words at all. Keyed as sung words, one of these made a single group of 126 unrelated
        // songs on the corpus this was measured against.
        let card = "someone@example.com http://example.com Tel:(00)00000000\n\
                    someone@example.com http://example.com Tel:(00)00000000\n\
                    someone@example.com http://example.com Tel:(00)00000000";
        assert_eq!(lyric_key(card), None);
    }

    #[test]
    fn a_credit_line_does_not_count_towards_the_floor() {
        // Twenty-four words of song with a credit line in the middle: the line is dropped and what
        // is left is one word short, so it keys to nothing.
        let short = "one two three four five six seven eight nine ten eleven twelve\n\
                     sequenced by someone@example.com\n\
                     thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty \
                     twentyone twentytwo twentythree twentyfour";
        assert_eq!(lyric_key(short), None);

        let long = format!("{short} twentyfive");
        assert!(lyric_key(&long).is_some(), "one more word clears the floor");
    }

    #[test]
    fn the_legal_notice_on_a_commercial_disc_is_not_a_song() {
        // The exact text, wrapped across events the way the files carry it. Nothing in it is an
        // address or a number, so a rule that looks only for contact details lets it through — and
        // 55 songs on the real corpus had this and nothing else as their words.
        let notice = "ALL rights reserved. Not for broadcast or\n\
                      transmission of any kind.\n\
                      DO NOT DUPLICATE. NOT FOR RENTAL.";
        assert_eq!(lyric_key(notice), None);

        // And unwrapped, since not every writer breaks it in the same place.
        let one_line = "ALL rights reserved. Not for broadcast or transmission of any kind. \
                        DO NOT DUPLICATE. NOT FOR RENTAL. International rights secured.";
        assert_eq!(lyric_key(one_line), None);
    }

    #[test]
    fn a_copyright_line_does_not_count_towards_the_floor() {
        let with_notice = format!("Copyright 1994 Some Publisher, all rights reserved\n{ENOUGH}");
        // The notice is dropped and the song beneath it still keys — to the same value it would
        // have without the notice, which is what lets a file that carries one meet a file that
        // does not.
        assert_eq!(lyric_key(&with_notice), lyric_key(ENOUGH));
        assert!(lyric_key(&with_notice).is_some());
    }

    #[test]
    fn a_section_label_is_not_a_word_of_the_song() {
        // `(Intro)` and `Vocals` are a sequencer labelling its own track, and a file that labels
        // its sections must key the same as one that does not.
        let labelled = format!("(Intro)\nVocals\n{ENOUGH}\n[ CHORUS ]");
        assert_eq!(lyric_key(&labelled), lyric_key(ENOUGH));

        // Twenty-four words of song plus three labels is still twenty-four words.
        let padded = "(Intro)\nVocals\nWords\none two three four five six seven eight nine ten \
                      eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen \
                      twenty twentyone twentytwo twentythree twentyfour";
        assert_eq!(lyric_key(padded), None);
    }

    #[test]
    fn a_sequencer_credit_in_either_language_is_dropped() {
        for credit in [
            "Sequenced by Somebody",
            "Realizado por Alguem",
            "Editado por GLOMAR",
            "midi by Someone Else",
        ] {
            let with_credit = format!("{credit}\n{ENOUGH}");
            assert_eq!(
                lyric_key(&with_credit),
                lyric_key(ENOUGH),
                "{credit:?} should not reach the key"
            );
        }
    }

    #[test]
    fn the_floor_is_words_and_not_lines() {
        assert_eq!(lyric_key("a b c"), None);
        assert!(lyric_key(ENOUGH).is_some());
    }

    #[test]
    fn accents_case_and_punctuation_key_alike() {
        // The same lyric as two sequencers wrote it: one accented and capitalised, one not, one
        // broken into different lines. `fold` is what makes those one key.
        let accented = format!("Coração, {ENOUGH}!");
        let plain = format!("CORACAO\n{ENOUGH}");
        assert_eq!(lyric_key(&accented), lyric_key(&plain));
        assert!(lyric_key(&accented).is_some());
    }

    #[test]
    fn different_words_key_differently() {
        let other = format!("{ENOUGH} and one more line of something else entirely");
        assert_ne!(lyric_key(ENOUGH), lyric_key(&other));
    }

    #[test]
    fn the_same_words_pair_two_songs_no_name_could_join() {
        // Neither the shapes nor the names match — this is the pair the fingerprint pass refuses,
        // and the one a real corpus is full of.
        let songs = vec![
            sings("aaa", "EARTHW~2", ENOUGH),
            sings("bbb", "FANTAZY", ENOUGH),
        ];
        let pairs = suggest(&songs);
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].0.as_str(), pairs[0].1.as_str()), ("aaa", "bbb"));
        assert_eq!(pairs[0].3, "same words");
    }

    #[test]
    fn a_song_with_no_words_never_pairs_on_them() {
        let songs = vec![
            sings("aaa", "One", ""),
            sings("bbb", "Two", ""),
            sings("ccc", "Three", "a b c"),
        ];
        assert!(suggest(&songs).is_empty());
    }

    #[test]
    fn a_set_of_words_is_a_star_and_not_a_clique() {
        let songs: Vec<Fingerprint> = ["aaa", "bbb", "ccc", "ddd"]
            .iter()
            .map(|id| sings(id, id, ENOUGH))
            .collect();
        let pairs = suggest(&songs);
        // Three pairs, not six. A clique would reconnect through a third song whatever anybody
        // dismissed, so the star is what lets a dismissal survive the next pass.
        assert_eq!(pairs.len(), 3);
        assert!(
            pairs.iter().all(|(a, _, _, _)| a == "aaa"),
            "the lowest id is the centre on every run: {pairs:?}"
        );
    }

    #[test]
    fn a_lyric_shared_by_too_many_files_says_nothing() {
        let songs: Vec<Fingerprint> = (0..70)
            .map(|i| sings(&format!("id{i:03}"), "whatever", ENOUGH))
            .collect();
        assert!(suggest(&songs).is_empty(), "the bucket guard holds");
    }

    #[test]
    fn accents_and_case_fold_together() {
        assert_eq!(normalize("Coração"), "coracao");
        assert_eq!(normalize("CORAÇÃO"), "coracao");
        assert_eq!(normalize("Águas de Março!"), "aguas de marco");
        assert_eq!(normalize("  a - b  "), "a b");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn the_same_song_saved_twice_is_suggested() {
        let songs = vec![
            print("aaa", "Coração", "Roberto Carlos", 200_000, "12:100:0,1,3"),
            print("bbb", "CORACAO", "Roberto Carlos", 202_000, "12:100:0,1,3"),
        ];
        let pairs = suggest(&songs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "aaa");
        assert_eq!(pairs[0].1, "bbb");
        assert!(pairs[0].2 >= THRESHOLD, "{}", pairs[0].2);
        assert!(pairs[0].3.contains("same title"), "{}", pairs[0].3);
    }

    #[test]
    fn a_matching_shape_alone_is_not_a_suggestion() {
        // Same structure, entirely different songs. This is the common case in a corpus built from
        // one studio's template, and suggesting it would make the review queue useless.
        let songs = vec![
            print("aaa", "Alpha", "One", 200_000, "12:100:0,1,3"),
            print("bbb", "Beta", "Two", 200_000, "12:100:0,1,3"),
        ];
        assert!(suggest(&songs).is_empty());
    }

    #[test]
    fn different_lengths_are_never_paired() {
        let songs = vec![
            print("aaa", "Coração", "Roberto Carlos", 120_000, "12:100:0,1,3"),
            print("bbb", "Coração", "Roberto Carlos", 240_000, "12:100:0,1,3"),
        ];
        assert!(suggest(&songs).is_empty());
    }

    #[test]
    fn different_shapes_are_never_compared() {
        let songs = vec![
            print("aaa", "Coração", "Roberto Carlos", 200_000, "12:100:0,1,3"),
            print("bbb", "Coração", "Roberto Carlos", 200_000, "40:100:0,2,9"),
        ];
        assert!(suggest(&songs).is_empty());
    }

    #[test]
    fn a_partly_matching_title_can_still_qualify() {
        let songs = vec![
            print(
                "aaa",
                "Garota de Ipanema",
                "Tom Jobim",
                200_000,
                "12:100:0,1",
            ),
            print(
                "bbb",
                "Garota de Ipanema (ao vivo)",
                "Tom Jobim",
                201_000,
                "12:100:0,1",
            ),
        ];
        let pairs = suggest(&songs);
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].3.contains("similar title"), "{}", pairs[0].3);
    }

    #[test]
    fn a_pair_is_stored_once_with_the_smaller_id_first() {
        let songs = vec![
            print("zzz", "Coração", "Roberto Carlos", 200_000, "12:100:0,1"),
            print("aaa", "Coração", "Roberto Carlos", 200_000, "12:100:0,1"),
        ];
        let pairs = suggest(&songs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "aaa");
        assert_eq!(pairs[0].1, "zzz");
    }

    #[test]
    fn one_song_cannot_flood_the_review_queue() {
        let mut songs = Vec::new();
        for i in 0..30 {
            songs.push(print(
                &format!("id{i:03}"),
                "Instrumental",
                "",
                200_000,
                "12:100:0,1",
            ));
        }
        let pairs = suggest(&songs);
        // Without the cap this would be 435 pairs, all of them the same finding restated.
        assert!(pairs.len() < 200, "{} pairs", pairs.len());
        for (a, b, _, _) in &pairs {
            assert!(a < b);
        }
    }

    #[test]
    fn an_empty_fingerprint_is_never_compared() {
        let songs = vec![
            print("aaa", "Coração", "Roberto Carlos", 200_000, ""),
            print("bbb", "Coração", "Roberto Carlos", 200_000, ""),
        ];
        assert!(suggest(&songs).is_empty());
    }
}
