//! Splitting the pinned hymnal into tunes, and giving each verse its own pass of the music.
//!
//! **The verse expansion is the whole of the difficulty here**, and this is why. The Open Hymnal
//! writes a tune's music once and stacks its
//! verses underneath as consecutive `w:` lines:
//!
//! ```text
//! [V: S1V1] [Q:1/4=60] F3/4G// F/ D3/2 | F3/4G// F/ D3/2 |
//! w: 1.~Si- * lent night, ho- * ly night, All is calm, all is bright
//! w: 2.~Si- * lent night, ho- * ly night, Shep- herds quake at the sight;
//! [V: S1V2]  D3/4E// D/ B,3/2 | D3/4E// D/ B,3/2 |
//! ```
//!
//! That is right for a printed score, where the verses sit one under another on the page, and wrong
//! for a karaoke file, where the music has to be played once per verse. `abc2midi` takes one set of
//! lyrics, so converting this as it stands yields **verse one and nothing else** — about thirty
//! seconds of music, which is below the minimum duration `km-suitability` will score and carries
//! far too few syllables to sing.
//!
//! So the body is regrouped into *systems* — a run of voice lines plus the lyric lines under
//! them — and emitted once per verse, each pass carrying only that verse's words. A four-verse
//! Silent Night becomes 3 systems × 4 = 12 systems of music and 172 syllables, which is the song.
//!
//! Two normalizations are applied on the way through, and both are notations `abc2midi` 5.03
//! **rejects outright** while `abcm2ps` (which is what the hymnal is typeset with) accepts. Left
//! alone they do not warn — they drop music, with `Malformed note` on stderr and a shorter file:
//!
//! - `A.-` → `A-`, a staccato dot before a tie. In *The First Noel*.
//! - `.(.(G F) G2)` → `.((G F) G2)`, a nested staccato slur. In *In The Bleak MidWinter* and
//!   *O Come, All Ye Faithful*.
//!
//! Neither changes a pitch or a duration; both drop an articulation mark that this pipeline has no
//! use for, since the output is a backing track rather than a score.

use anyhow::{Result, bail};

/// One tune, as it stands in the hymnal.
#[derive(Debug, Clone)]
pub struct Tune {
    /// The `X:` reference number. Stable within a pinned edition, which is why the pack selects on
    /// it rather than on the title.
    pub number: u32,
    /// The `T:` title.
    pub title: String,
    /// The `C:` lines verbatim — attribution prose for the words, the music and the setting, and
    /// the one line stating the copyright status. `license::assess` reads these; `CREDITS.md`
    /// prints them.
    pub credits: Vec<String>,
    /// Every line of the tune, `X:` first.
    lines: Vec<String>,
}

/// A tune with one pass of the music per verse.
#[derive(Debug, Clone)]
pub struct Expanded {
    /// ABC ready for `abc2midi`.
    pub abc: String,
    /// How many verses were found.
    pub verses: usize,
    /// How many systems the music was written in, before expansion.
    pub systems: usize,
}

/// Splits a hymnal into its tunes.
///
/// A tune runs from its `X:` line to the next one. Everything the hymnal puts between tunes —
/// page-layout directives, the license banner repeated every few pages — is comment or `%%`
/// directive and travels harmlessly with the tune above it, so it is cut rather than parsed.
#[must_use]
pub fn split(source: &str) -> Vec<Tune> {
    let mut tunes = Vec::new();
    let mut current: Option<Vec<String>> = None;

    for line in source.lines() {
        if line.starts_with("X:") {
            if let Some(lines) = current.take()
                && let Some(tune) = Tune::new(lines)
            {
                tunes.push(tune);
            }
            current = Some(vec![line.to_owned()]);
        } else if let Some(lines) = current.as_mut() {
            lines.push(line.to_owned());
        }
    }
    if let Some(lines) = current
        && let Some(tune) = Tune::new(lines)
    {
        tunes.push(tune);
    }
    tunes
}

impl Tune {
    /// Builds a tune from its lines, or `None` if the `X:` line carries no number.
    fn new(mut lines: Vec<String>) -> Option<Self> {
        // Anything after the tune's last music line belongs to the page, not the song.
        if let Some(at) = lines.iter().position(|l| l.starts_with("%%newpage")) {
            lines.truncate(at);
        }

        let number = field(&lines, "X:")?.trim().parse().ok()?;
        let title = field(&lines, "T:")?.trim().to_owned();
        let credits = lines
            .iter()
            .filter_map(|l| l.strip_prefix("C:"))
            .map(|v| v.trim().to_owned())
            .collect();

        Some(Self {
            number,
            title,
            credits,
            lines,
        })
    }

    /// Emits one pass of the music per verse.
    ///
    /// The header is everything through the last `K:` or `%%MIDI program` line; the rest is body.
    /// A new system begins when a voice the current system has already written appears again,
    /// which is what makes this independent of how many voices a tune has — the pack's sixteen
    /// range from two (*Away In A Manger*) to four (everything else).
    pub fn expand(&self, melody_voice_name: &str) -> Result<Expanded> {
        let split_at = self
            .lines
            .iter()
            .rposition(|l| l.starts_with("K:") || l.starts_with("%%MIDI program"))
            .unwrap_or(0);
        let (header, body) = self.lines.split_at(split_at + 1);

        let systems = systems(body)?;
        let verses = systems.iter().map(|s| s.words.len()).max().unwrap_or(0);
        if verses == 0 {
            bail!("{} (X:{}) has no aligned lyrics", self.title, self.number);
        }

        let mut out: Vec<String> = header
            .iter()
            // The prose credits are dropped from the ABC and kept in `credits`. abc2midi copies
            // `C:` and `S:` fields into the karaoke text stream, where they arrive as a wall of
            // metadata in place of the song's first line — and, being Latin-1, they are also what
            // pushes the file off UTF-8 and makes km-song guess an encoding.
            .filter(|l| !l.starts_with("C:") && !l.starts_with("S:"))
            .map(|l| name_melody_voice(l, melody_voice_name))
            .collect();

        for verse in 0..verses {
            for system in &systems {
                for (index, music) in system.music.iter().enumerate() {
                    // Only the last pass ends the tune; the earlier ones run on into the next verse.
                    let music = if verse + 1 < verses {
                        open_final_bar(music)
                    } else {
                        music.clone()
                    };
                    out.push(normalize(&music));
                    if index == 0
                        && let Some(words) = system.words.get(verse % system.words.len().max(1))
                    {
                        out.push(strip_verse_number(words));
                    }
                }
            }
        }

        out.push(String::new());
        Ok(Expanded {
            abc: out.join("\n"),
            verses,
            systems: systems.len(),
        })
    }
}

/// A run of voice lines with the lyric lines that belong under them.
struct System {
    music: Vec<String>,
    words: Vec<String>,
}

/// Groups a tune's body into systems.
fn systems(body: &[String]) -> Result<Vec<System>> {
    let mut systems: Vec<System> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for line in body {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // `% 5` bar counters go; `%%directive` stays.
        if trimmed.starts_with('%') && !trimmed.starts_with("%%") {
            continue;
        }

        if let Some(voice) = voice_of(trimmed) {
            if systems.is_empty() || seen.contains(&voice) {
                systems.push(System {
                    music: Vec::new(),
                    words: Vec::new(),
                });
                seen.clear();
            }
            seen.push(voice);
            push_music(&mut systems, line);
        } else if let Some(words) = trimmed.strip_prefix("w:") {
            let Some(system) = systems.last_mut() else {
                bail!("a lyric line appears before any voice");
            };
            system.words.push(words.trim().to_owned());
        } else if trimmed.starts_with("W:") {
            // Unaligned extra verses. There is no way to know which note each syllable belongs to,
            // so they are not singable and are left out rather than guessed at.
        } else {
            push_music(&mut systems, line);
        }
    }

    if systems.is_empty() {
        bail!("no music found");
    }
    Ok(systems)
}

/// Appends a music line to the system in hand, ignoring anything before the first one.
fn push_music(systems: &mut [System], line: &str) {
    if let Some(system) = systems.last_mut() {
        system.music.push(line.to_owned());
    }
}

/// The voice a `[V: name]` line names.
fn voice_of(line: &str) -> Option<String> {
    let rest = line.strip_prefix("[V:")?;
    Some(rest.trim_start().split([']', ' ']).next()?.to_owned())
}

/// Turns a closing `|]` into a plain bar, so the music runs on into the next verse.
fn open_final_bar(line: &str) -> String {
    match line.trim_end().strip_suffix("|]") {
        Some(head) => format!("{head}|"),
        None => line.to_owned(),
    }
}

/// Drops the printed verse number from a lyric line.
///
/// The hymnal writes `1.~Si- * lent night` — the number, then ABC's `~` (a hard space that joins
/// the syllables either side). Sung, the number is not part of the words.
fn strip_verse_number(words: &str) -> String {
    let body = words
        .split_once('~')
        .filter(|(head, _)| {
            !head.is_empty()
                && head
                    .trim_end_matches('.')
                    .chars()
                    .all(|c| c.is_ascii_digit())
        })
        .map_or(words, |(_, tail)| tail);
    format!("w: {}", body.trim_start())
}

/// The two notations abc2midi 5.03 rejects. See the module header.
fn normalize(line: &str) -> String {
    line.replace(".(.(", ".((").replace(".-", "-")
}

/// Names the melody voice, so the post-processing pass can find it by name.
///
/// The name never reaches the MIDI — abc2midi parses `name=` for the typesetter and does not write
/// voice names as track names, which is exactly why `midi::rewrite` exists. It is set here so the
/// generated ABC is self-describing to anybody reading it.
fn name_melody_voice(line: &str, melody: &str) -> String {
    match line.strip_prefix("V: ") {
        Some(rest) if rest.starts_with(melody) && !line.contains("name=") => {
            format!("{} name=\"Melody\"", line.trim_end())
        }
        _ => line.to_owned(),
    }
}

/// The value of the first line with this prefix.
fn field(lines: &[String], prefix: &str) -> Option<String> {
    lines
        .iter()
        .find_map(|l| l.strip_prefix(prefix))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_VERSES: &str = "\
X: 7
T: A Tune
C: Words: somebody, 1870.
C: copyright: public domain.
S: Music source: a book.
M: 4/4
V: S1V1 clef=treble
V: S1V2
K: G
%%MIDI program 1 0
% 1
[V: S1V1] G A B c |
w: 1.~one two three four
w: 2.~five six sev- en
[V: S1V2] D E F G |
% 5
[V: S1V1] c B A G |]
w: five six sev- en eight
w: nine ten e- le- ven
[V: S1V2] G F E D |]
W: 3.An unaligned verse
";

    fn tune() -> Tune {
        let tunes = split(TWO_VERSES);
        assert_eq!(tunes.len(), 1);
        tunes.into_iter().next().unwrap()
    }

    #[test]
    fn a_tune_carries_its_number_title_and_credits() {
        let t = tune();
        assert_eq!(t.number, 7);
        assert_eq!(t.title, "A Tune");
        assert_eq!(t.credits.len(), 2);
        assert!(t.credits[1].contains("public domain"));
    }

    #[test]
    fn the_music_is_emitted_once_per_verse() {
        let e = tune().expand("S1V1").unwrap();
        assert_eq!(e.verses, 2);
        assert_eq!(e.systems, 2);
        // Two systems, two voices each, twice over.
        assert_eq!(e.abc.matches("[V: S1V1]").count(), 4);
        assert_eq!(e.abc.matches("[V: S1V2]").count(), 4);
        // ...and each verse's own words, once.
        assert_eq!(e.abc.matches("w: one two three four").count(), 1);
        assert_eq!(e.abc.matches("w: five six sev- en\n").count(), 1);
    }

    #[test]
    fn the_verse_number_is_not_sung() {
        let e = tune().expand("S1V1").unwrap();
        assert!(!e.abc.contains("1.~"));
        assert!(!e.abc.contains("w: 1."));
    }

    #[test]
    fn only_the_last_pass_closes_the_tune() {
        let e = tune().expand("S1V1").unwrap();
        // One `|]` per voice, in the final verse only.
        assert_eq!(e.abc.matches("|]").count(), 2);
    }

    #[test]
    fn the_prose_fields_are_left_out_of_the_abc() {
        let e = tune().expand("S1V1").unwrap();
        assert!(!e.abc.contains("C: Words"));
        assert!(!e.abc.contains("S: Music source"));
        assert!(e.abc.contains("T: A Tune"));
    }

    #[test]
    fn an_unaligned_verse_is_not_guessed_at() {
        let e = tune().expand("S1V1").unwrap();
        assert!(!e.abc.contains("unaligned"));
    }

    #[test]
    fn the_melody_voice_is_named() {
        let e = tune().expand("S1V1").unwrap();
        assert!(e.abc.contains("V: S1V1 clef=treble name=\"Melody\""));
        assert!(!e.abc.contains("V: S1V2 name="));
    }

    /// Both are in the pinned edition and both drop music if they reach abc2midi.
    #[test]
    fn the_two_rejected_notations_are_normalized() {
        assert_eq!(normalize("[V: X] A.- A G |"), "[V: X] A- A G |");
        assert_eq!(normalize("[V: X] .(.(G F) G2) |"), "[V: X] .((G F) G2) |");
    }

    #[test]
    fn a_tune_with_no_lyrics_is_refused() {
        let source = "X: 1\nT: Instrumental\nK: C\n[V: A] C D E F |]\n";
        let err = split(source)[0].expand("A").unwrap_err().to_string();
        assert!(err.contains("no aligned lyrics"), "{err}");
    }
}
