//! Finding a song under another spelling of its name.
//!
//! **Loose on purpose, and a ranking rather than a filter.** The corpus files one song as
//! `Dancing In The Dark` by `Springsteen, Bruce`, as `Dancin' in the Dark`, and as a file name holding
//! both, and the Songs search box, which asks for every word, reaches one of those at a time. This
//! answers *is there another file of this song?* with the likeliest names first and the coincidences
//! last, where somebody reading down the list stops.
//!
//! Two halves. [`match_query`] is the FTS5 expression that gathers candidates cheaply out of
//! `songs_fts`, wide enough to catch a misspelt word; [`likeness`] is the score that orders them, done
//! outside SQLite because no tokenizer there compares one word with another. Nothing here reads or
//! writes a database. See `Finding a song under another spelling of its name` in
//! `docs/decisions/curation.md`.

/// The lowest likeness a candidate is shown at.
///
/// Low, because a false match costs a glance and a missed one is the file somebody was looking for.
pub const THRESHOLD: f32 = 0.4;

/// How many matches a search shows at most.
pub const SHOWN: usize = 100;

/// How many candidates the index is asked for before they are scored.
pub const CANDIDATES: u32 = 400;

/// Words that say nothing about which song a file is.
///
/// Articles and joiners, which one spelling of a name has and the next leaves out, and the words a
/// corpus adds to a name to describe the file rather than the song.
const NOISE: &[&str] = &[
    "the",
    "a",
    "an",
    "and",
    // Short joining words, which two unrelated titles share often enough to lift one toward the
    // other: *Born in the USA* is not a spelling of *Dancing in the Dark*.
    "in",
    "of",
    "on",
    "to",
    "at",
    "by",
    "for",
    "with",
    "is",
    "it",
    "my",
    "me",
    "you",
    "de",
    "da",
    "do",
    "la",
    "el",
    "en",
    "e",
    "y",
    "feat",
    "ft",
    "featuring",
    "karaoke",
    "version",
    "kar",
    "mid",
    "midi",
    "instrumental",
    "vocal",
    "remix",
    "live",
];

/// A word shorter than this is not searched for in the index, where a prefix of one or two letters
/// matches a large share of every word there is.
const SHORTEST_INDEXED: usize = 3;

/// A word is searched for by its first this-many letters, so a misspelling further in still matches.
const PREFIX: usize = 5;

/// Two words less alike than this count as different words.
const WORD_FLOOR: f32 = 0.75;

/// The words of a name that say which song it is: folded, without what brackets hold, and without
/// [`NOISE`].
///
/// Brackets hold what a publisher adds to a name, `[SF Karaoke]` or `(Live)`, and one name carries
/// them where the next does not. A name that is nothing but brackets keeps what they hold, and a name
/// made only of noise words keeps those, because *The The* is a band and an empty list of words
/// matches nothing.
pub fn words(name: &str) -> Vec<String> {
    let outside = unbracketed(name);
    let from_outside = folded_words(&outside);
    if from_outside.is_empty() {
        folded_words(name)
    } else {
        from_outside
    }
}

/// `name` with every bracketed run taken out, and everything after a bracket that never closes, which
/// is what a name cut short by a field's length leaves.
fn unbracketed(name: &str) -> String {
    let mut kept = String::with_capacity(name.len());
    let mut depth = 0usize;
    for c in name.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                kept.push(' ');
            }
            _ if depth == 0 => kept.push(c),
            _ => {}
        }
    }
    kept
}

/// [`words`] without the brackets rule.
fn folded_words(name: &str) -> Vec<String> {
    let folded = km_song::text::fold(name);
    let all: Vec<&str> = folded
        .split(' ')
        .filter(|word| word.chars().count() >= 2)
        .collect();
    let kept: Vec<String> = all
        .iter()
        .filter(|word| !NOISE.contains(word))
        .map(|word| (*word).to_owned())
        .collect();
    if kept.is_empty() {
        all.into_iter().map(str::to_owned).collect()
    } else {
        kept
    }
}

/// The FTS5 expression that gathers candidates, or `None` when the name has no words.
///
/// Every word is ORed, so a file missing one of them or misspelling it is still found, and each is
/// matched by a prefix of at most [`PREFIX`] letters: `"sprin"*` reaches `Springstein`, and `"danci"*`
/// reaches `Dancin`. The expression names no column, so a title word finds a file whose name is in its
/// artist field, and the other way round.
pub fn match_query(title: &str, artist: &str) -> Option<String> {
    let mut all = words(title);
    all.extend(words(artist));
    let long: Vec<&String> = all
        .iter()
        .filter(|word| word.chars().count() >= SHORTEST_INDEXED)
        .collect();
    let chosen = if long.is_empty() {
        all.iter().collect()
    } else {
        long
    };

    let mut terms: Vec<String> = Vec::new();
    for word in chosen {
        // Folding leaves letters, digits and spaces, so no `"` can be in a word; this is what keeps it
        // that way if folding changes.
        let prefix: String = word.chars().filter(|c| *c != '"').take(PREFIX).collect();
        let term = format!("\"{prefix}\"*");
        if !prefix.is_empty() && !terms.contains(&term) {
            terms.push(term);
        }
    }
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

/// How alike one song's name is to another's, from 0 to 1.
///
/// Where both sides name an artist, the title weighs three times the artist, because a cover by
/// somebody else is still the song. That fielded score is set against one that pools each side's
/// title and artist into one set of words, which is what finds swapped fields and a file name holding
/// both, and the higher of the two is kept.
///
/// **A pooled score is scaled by how much of the title it found.** Pooled, a file by the same artist
/// shares half its words with the name searched for whatever song it is, and every other song by
/// that artist would rank beside the one being looked for. Where both sides name an artist it is also
/// taken at nine tenths, so a clean match on both fields ranks above a jumble of the same words.
///
/// **An artist only one side names does not count against the match.** Pooled, that artist's words
/// are extra words, and a one-word title searched with no artist would stay under [`THRESHOLD`]
/// against every file that names a two-word artist. So a search with no artist also takes the title
/// against the title alone, and a file with no artist takes it at the weight a cover gets, because
/// the artist can be neither confirmed nor ruled out.
pub fn likeness(title: &str, artist: &str, other_title: &str, other_artist: &str) -> f32 {
    let title = words(title);
    let artist = words(artist);
    let other_title = words(other_title);
    let other_artist = words(other_artist);

    let join = |a: &[String], b: &[String]| -> Vec<String> { a.iter().chain(b).cloned().collect() };
    let all = join(&title, &artist);
    let other_all = join(&other_title, &other_artist);

    // Only an artist's words can lift a pooled score without the title, so a name searched with no
    // artist is not scaled twice for the same missing words.
    let title_found = if title.is_empty() || artist.is_empty() {
        1.0
    } else {
        coverage(&title, &other_all)
    };
    let pooled = name_likeness(&all, &other_all) * title_found;

    if artist.is_empty() {
        return pooled.max(name_likeness(&title, &other_title));
    }
    if other_artist.is_empty() {
        return pooled.max(0.75 * name_likeness(&title, &other_title));
    }
    let fielded =
        0.75 * name_likeness(&title, &other_title) + 0.25 * name_likeness(&artist, &other_artist);
    fielded.max(0.9 * pooled)
}

/// The share of `words` that has a match among `other`, weighed by how good each match is.
fn coverage(words: &[String], other: &[String]) -> f32 {
    if words.is_empty() {
        return 0.0;
    }
    let found: f32 = words
        .iter()
        .map(|word| {
            other
                .iter()
                .map(|candidate| word_likeness(word, candidate))
                .fold(0.0, f32::max)
        })
        .sum();
    found / words.len() as f32
}

/// How alike two lists of words are: each word's best match on the other side, over the longer list.
///
/// Dividing by the longer list is what makes an extra word cost something, so *Dancing in the Street*
/// is half a match for *Dancing in the Dark* rather than most of one.
fn name_likeness(words: &[String], other: &[String]) -> f32 {
    let longer = words.len().max(other.len());
    if words.is_empty() || other.is_empty() {
        return 0.0;
    }
    let shorter_side = if words.len() <= other.len() {
        words
    } else {
        other
    };
    let longer_side = if words.len() <= other.len() {
        other
    } else {
        words
    };
    let matched: f32 = shorter_side
        .iter()
        .map(|word| {
            longer_side
                .iter()
                .map(|candidate| word_likeness(word, candidate))
                .fold(0.0, f32::max)
        })
        .sum();
    matched / longer as f32
}

/// How alike two folded words are, from 0 to 1.
///
/// The same word is 1. A word that begins the other, at four letters or more, is 0.9, which is
/// `dancin` against `dancing`. Otherwise it is the share of letter pairs the two have in common, the
/// Dice coefficient, which is `springstein` against `springsteen`; under [`WORD_FLOOR`] it is 0.
fn word_likeness(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if short.chars().count() >= 4 && long.starts_with(short) {
        return 0.9;
    }
    let pairs = |word: &str| -> Vec<(char, char)> {
        let chars: Vec<char> = word.chars().collect();
        chars.windows(2).map(|pair| (pair[0], pair[1])).collect()
    };
    let a_pairs = pairs(a);
    let mut b_pairs = pairs(b);
    let total = a_pairs.len() + b_pairs.len();
    if total == 0 {
        return 0.0;
    }
    let mut shared = 0;
    for pair in &a_pairs {
        if let Some(at) = b_pairs.iter().position(|other| other == pair) {
            b_pairs.swap_remove(at);
            shared += 1;
        }
    }
    let dice = (2 * shared) as f32 / total as f32;
    if dice >= WORD_FLOOR { dice } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: &str = "Dancing in the Dark";
    const ARTIST: &str = "Bruce Springsteen";

    fn against(title: &str, artist: &str) -> f32 {
        likeness(TITLE, ARTIST, title, artist)
    }

    #[test]
    fn the_same_name_is_a_whole_match() {
        assert_eq!(against(TITLE, ARTIST), 1.0);
    }

    #[test]
    fn spellings_of_one_song_clear_the_threshold() {
        for (title, artist) in [
            ("DANCING IN THE DARK", "Springsteen, Bruce"),
            ("Dancin' in the Dark", "Bruce Springstein"),
            ("Dancing in the Dark (Karaoke Version)", "Bruce Springsteen"),
            ("Dancing in the Dark", ""),
            ("Bruce Springsteen - Dancing in the Dark", ""),
            ("Bruce Springsteen", "Dancing in the Dark"),
            ("Dancing in the Dark", "Hot Chip"),
        ] {
            let score = against(title, artist);
            assert!(score >= THRESHOLD, "{title} / {artist}: {score}");
        }
    }

    #[test]
    fn a_different_song_ranks_below_every_spelling() {
        let street = against("Dancing in the Street", "Martha and the Vandellas");
        let misspelt = against("Dancin in the Dark", "Bruce Springstein");
        let cover = against("Dancing in the Dark", "Hot Chip");
        assert!(street < misspelt, "{street} against {misspelt}");
        assert!(street < cover, "{street} against {cover}");
    }

    #[test]
    fn nothing_in_common_is_nothing() {
        assert_eq!(against("Corcovado", "Tom Jobim"), 0.0);
        assert_eq!(likeness("", "", TITLE, ARTIST), 0.0);
    }

    #[test]
    fn a_search_with_no_artist_finds_the_song_under_any_artist() {
        for artist in [
            "Tom Jobim",
            "Astrud Gilberto",
            "Frank Sinatra and Antonio Carlos Jobim",
        ] {
            let score = likeness("Corcovado", "", "Corcovado", artist);
            assert_eq!(score, 1.0, "{artist}");
        }
        assert_eq!(likeness(TITLE, "", TITLE, ARTIST), 1.0);
    }

    #[test]
    fn a_file_with_no_artist_is_weighed_like_a_cover() {
        let unnamed = likeness("Corcovado", "Tom Jobim", "Corcovado", "");
        let cover = likeness("Corcovado", "Tom Jobim", "Corcovado", "Astrud Gilberto");
        assert!(unnamed >= THRESHOLD, "{unnamed}");
        assert_eq!(unnamed, cover);
        assert!(unnamed < likeness("Corcovado", "Tom Jobim", "Corcovado", "Tom Jobim"));
    }

    #[test]
    fn a_search_with_no_artist_still_ranks_another_song_below_the_title() {
        let street = likeness(
            TITLE,
            "",
            "Dancing in the Street",
            "Martha and the Vandellas",
        );
        let exact = likeness(TITLE, "", TITLE, ARTIST);
        assert!(street < exact, "{street} against {exact}");
        assert_eq!(likeness("Corcovado", "", "Born in the USA", ARTIST), 0.0);
    }

    #[test]
    fn words_fold_and_leave_the_noise_out() {
        assert_eq!(
            words("The Café, feat. Zé — Karaoke"),
            vec!["cafe".to_owned(), "ze".to_owned()]
        );
        assert_eq!(words("The The"), vec!["the".to_owned(), "the".to_owned()]);
    }

    #[test]
    fn what_brackets_hold_is_left_out() {
        assert_eq!(words("Dancing In The Dark [SF Karaoke]"), words(TITLE));
        assert_eq!(words("Dancing In The Dark (Live) [DMG Karao"), words(TITLE));
        assert_eq!(words("(Untitled)"), vec!["untitled".to_owned()]);
    }

    #[test]
    fn another_song_by_the_same_artist_is_not_a_match() {
        for (title, artist) in [
            ("Born in the USA", "Bruce Springsteen"),
            ("Glory Days by Bruce Springsteen", ""),
        ] {
            let score = against(title, artist);
            assert!(score < THRESHOLD, "{title} / {artist}: {score}");
        }
    }

    #[test]
    fn the_expression_ors_prefixes_of_every_word() {
        assert_eq!(
            match_query(TITLE, ARTIST).as_deref(),
            Some(r#""danci"* OR "dark"* OR "bruce"* OR "sprin"*"#)
        );
    }

    #[test]
    fn short_words_are_searched_only_when_there_is_nothing_else() {
        assert_eq!(match_query("U2", "").as_deref(), Some(r#""u2"*"#));
        assert_eq!(match_query("Me & U", "").as_deref(), Some(r#""me"*"#));
        assert_eq!(match_query("", "").as_deref(), None);
        assert_eq!(match_query("!!", "").as_deref(), None);
    }

    #[test]
    fn a_word_is_searched_once() {
        assert_eq!(
            match_query("Love Love Love", "").as_deref(),
            Some(r#""love"*"#)
        );
    }

    #[test]
    fn word_likeness_takes_a_prefix_and_a_misspelling() {
        assert_eq!(word_likeness("dancin", "dancing"), 0.9);
        assert!(word_likeness("springstein", "springsteen") >= WORD_FLOOR);
        assert_eq!(word_likeness("dark", "street"), 0.0);
        assert_eq!(word_likeness("in", "it"), 0.0);
    }
}
