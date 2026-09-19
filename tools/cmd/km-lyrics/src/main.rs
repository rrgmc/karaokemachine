//! Inspect what was actually parsed out of a karaoke MIDI file.
//!
//! Lyric-format variance is the project's top risk, so it gets a dedicated tool from the first
//! milestone. Two modes:
//!
//! * `dump` — everything one file yielded, as text or JSON.
//! * `scan` — parse a whole directory tree and report the distribution of formats, encodings and
//!   failures. This is how the parser is held against reality rather than against its own fixtures.
//!
//! See `docs/ARCHITECTURE.md`.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use km_song::{LineInference, LyricGranularity, ParseOptions, Song, WordEnds};
use km_suitability::{Abstention, Analysis, MelodyOutcome};

#[derive(Parser)]
#[command(
    name = "km-lyrics",
    about = "Inspect karaoke MIDI lyric parsing",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what one file parsed to.
    Dump(DumpArgs),
    /// Parse every MIDI file under a directory and summarize the results.
    Scan(ScanArgs),
    /// Report what a package's two-line lyric preview would say, across a whole corpus.
    Preview(PreviewArgs),
}

#[derive(Args)]
struct DumpArgs {
    /// The `.mid` or `.kar` file to inspect.
    file: PathBuf,
    /// Emit JSON instead of readable text.
    #[arg(long)]
    json: bool,
    /// Force a lyric encoding, as a package manifest would (for example `windows-1252`).
    #[arg(long)]
    encoding: Option<String>,
    /// Include the channel event list, which is usually very long.
    #[arg(long)]
    events: bool,
    /// List every text meta event as it appears in the file, without interpreting it.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
struct ScanArgs {
    /// Directory to walk recursively.
    dir: PathBuf,
    /// Stop after this many files.
    #[arg(long)]
    limit: Option<usize>,
    /// Comma-separated extensions to consider.
    #[arg(long, default_value = "mid,midi,kar")]
    ext: String,
    /// Worker threads.
    #[arg(long, default_value_t = 8)]
    jobs: usize,
    /// Write the path of every failing file here.
    #[arg(long)]
    failures: Option<PathBuf>,
    /// Show this many example paths per failure reason.
    #[arg(long, default_value_t = 3)]
    examples: usize,
    /// Leave every line where the file put it, rather than re-breaking a run the file left unmarked.
    ///
    /// This is how the bound on such a run is measured, and it reports a corpus the bound has not
    /// been applied to. Without it the report says what the machine will draw.
    #[arg(long)]
    as_written: bool,
}

/// The corpus run behind the lyric-preview heuristic.
///
/// The same shape as [`ScanArgs`] on purpose: the banner rules in `km_song::looks_like_a_banner` are
/// a claim about what real files contain, and the only way to know whether they are right is to run
/// them over hundreds of thousands of real files and read what comes out. This is the counterpart of
/// `km-cdg`'s `scan` example, which set the MP3+G packaging checks the same way.
#[derive(Args)]
struct PreviewArgs {
    /// Directory to walk recursively.
    dir: PathBuf,
    /// Stop after this many files.
    #[arg(long)]
    limit: Option<usize>,
    /// Comma-separated extensions to consider.
    #[arg(long, default_value = "mid,midi,kar")]
    ext: String,
    /// Worker threads.
    #[arg(long, default_value_t = 8)]
    jobs: usize,
    /// Show this many sampled previews, with the lines that were skipped above each.
    #[arg(long, default_value_t = 20)]
    examples: usize,
    /// Print a tally of every line that was skipped.
    #[arg(long)]
    skipped: bool,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Dump(args) => dump(&args),
        Command::Scan(args) => scan(&args),
        Command::Preview(args) => preview(&args),
    }
}

// --- dump ------------------------------------------------------------------------------------

fn dump(args: &DumpArgs) -> Result<()> {
    let bytes =
        std::fs::read(&args.file).with_context(|| format!("reading {}", args.file.display()))?;

    if args.raw {
        let events = km_song::text_events(&bytes, args.encoding.as_deref())
            .with_context(|| format!("parsing {}", args.file.display()))?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&events)?);
        } else {
            println!(
                "{} text meta event(s) in {}",
                events.len(),
                args.file.display()
            );
            for event in &events {
                println!(
                    "  track {:>2}  tick {:>8}  {:<15}  {:?}",
                    event.track, event.tick, event.kind, event.text
                );
            }
        }
        return Ok(());
    }

    let options = ParseOptions {
        declared_encoding: args.encoding.clone(),
        inference: None,
    };
    let song = Song::parse(&bytes, &options)
        .with_context(|| format!("parsing {}", args.file.display()))?;

    if args.json {
        print_json(&song, args.events)?;
    } else {
        print_text(&song, &args.file, args.events);
    }
    Ok(())
}

fn print_json(song: &Song, include_events: bool) -> Result<()> {
    let mut root = serde_json::Map::new();
    root.insert("flavor".into(), serde_json::to_value(song.flavor)?);
    root.insert("track_count".into(), song.track_count.into());
    root.insert("ticks_per_quarter".into(), song.ticks_per_quarter.into());
    root.insert("duration_ticks".into(), song.duration_ticks.into());
    root.insert("duration_ms".into(), song.duration_ms().into());
    root.insert("tempo_changes".into(), song.tempo_map.change_count().into());
    root.insert("note_count".into(), song.note_count().into());
    root.insert(
        "sounding_channels".into(),
        serde_json::to_value(song.sounding_channels())?,
    );
    root.insert("encoding".into(), song.decoder.name().into());
    root.insert(
        "encoding_source".into(),
        serde_json::to_value(song.decoder.source())?,
    );
    root.insert(
        "granularity".into(),
        serde_json::to_value(song.lyrics.granularity())?,
    );
    root.insert("meta".into(), serde_json::to_value(&song.meta)?);
    root.insert("analysis".into(), serde_json::to_value(Analysis::of(song))?);
    root.insert("lyrics".into(), serde_json::to_value(&song.lyrics)?);
    if include_events {
        root.insert("events".into(), serde_json::to_value(&song.events)?);
    }
    println!("{}", serde_json::to_string_pretty(&root)?);
    Ok(())
}

fn print_text(song: &Song, path: &Path, include_events: bool) {
    println!("{}", path.display());
    println!("  format          {:?}", song.flavor);
    println!("  tracks          {}", song.track_count);
    println!(
        "  timebase        {} tpqn, {} tempo change(s)",
        song.ticks_per_quarter,
        song.tempo_map.change_count()
    );
    println!(
        "  duration        {} ticks / {}",
        song.duration_ticks,
        format_ms(song.duration_ms())
    );
    println!("  notes           {}", song.note_count());
    println!("  channels        {:?}", song.sounding_channels());
    println!(
        "  encoding        {} ({:?})",
        song.decoder.name(),
        song.decoder.source()
    );
    // Only when there is one, so an ordinary file's dump keeps its shape.
    let mut habits: Vec<&str> = Vec::new();
    if song.dialect.angle_starts_lines {
        habits.push("lines opened with `<`");
    }
    if song.dialect.annotations_are_marked {
        habits.push("chord symbols dropped");
    }
    if song.dialect.harmonica_tabs {
        habits.push("harmonica tabs dropped");
    }
    if !habits.is_empty() {
        println!("  notation        {}", habits.join(", "));
    }

    println!(
        "  title           {}",
        song.meta.title.as_deref().unwrap_or("-")
    );
    println!(
        "  artist          {}",
        song.meta.artist.as_deref().unwrap_or("-")
    );
    if let Some(copyright) = &song.meta.copyright {
        println!("  copyright       {copyright}");
    }
    if let Some(language) = &song.meta.language {
        println!("  language        {language}");
    }
    for info in &song.meta.info {
        println!("  info            {info}");
    }

    // The divider prints invisibly in the lines below, so without saying so here a file whose word
    // ends are lost would read exactly like one whose are not. Where the lines came from is the
    // same kind of fact and is the first thing to know when a line below looks wrong: a file that
    // places none of its own has every line here inferred, and one that places some of them may
    // still have been cut where it went quiet.
    let mut notes: Vec<&str> = Vec::new();
    match song.lyrics.word_ends {
        WordEnds::AsWritten => {}
        WordEnds::EverySyllableSpaced => notes.push("every syllable spaced, no word ends marked"),
        WordEnds::NoneSpaced => notes.push("nothing spaced, no word ends marked"),
    }
    notes.push(if song.lyrics.lines_are_marked {
        "lines placed by the file"
    } else {
        "lines inferred"
    });
    println!(
        "  lyrics          {} line(s), {} syllable(s), {} page(s), {:?}, {}",
        song.lyrics.line_count(),
        song.lyrics.syllable_count(),
        song.lyrics.page_count(),
        song.lyrics.granularity(),
        notes.join(", ")
    );

    let analysis = Analysis::of(song);
    match &analysis.melody {
        MelodyOutcome::Found(melody) => println!(
            "  melody          channel {} (confidence {:.2}, alignment {:.0}%, monophony {:.0}%) via {:?}",
            melody.channel,
            melody.confidence,
            melody.lyric_alignment * 100.0,
            melody.monophony * 100.0,
            melody.signals
        ),
        MelodyOutcome::Abstained { abstained } => {
            println!("  melody          none claimed ({abstained:?})");
        }
    }
    let s = &analysis.suitability;
    println!(
        "  suitability     {}/10  (lyrics {}, sync {}, channels {}, arrangement {})",
        s.value,
        s.breakdown.lyrics,
        s.breakdown.sync,
        s.breakdown.channels,
        s.breakdown.arrangement
    );
    for warning in &s.warnings {
        println!("    ! {:?}: {}", warning.code, warning.message);
    }
    println!();

    let mut page = None;
    for line in &song.lyrics.lines {
        if page != Some(line.page) {
            if page.is_some() {
                println!("  ---- page {} ----", line.page);
            }
            page = Some(line.page);
        }
        println!(
            "  [{:>8} {:>9}] {}",
            line.start_tick,
            format_ms(song.tempo_map.tick_to_ms(line.start_tick)),
            line.text()
        );
    }

    if include_events {
        println!("\n  events ({}):", song.events.len());
        for event in &song.events {
            println!("    {:>8}  {:?}", event.tick, event.kind);
        }
    }
}

fn format_ms(ms: u32) -> String {
    format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1_000) % 60, ms % 1_000)
}

// --- scan ------------------------------------------------------------------------------------

/// Counters accumulated over a corpus.
#[derive(Default)]
struct Stats {
    files: usize,
    parsed: usize,
    failed: usize,
    panicked: usize,
    flavor: BTreeMap<String, usize>,
    encoding: BTreeMap<String, usize>,
    encoding_source: BTreeMap<String, usize>,
    granularity: BTreeMap<String, usize>,
    failure_reason: BTreeMap<String, usize>,
    failure_examples: BTreeMap<String, Vec<String>>,
    with_title: usize,
    with_artist: usize,
    total_lines: u64,
    total_syllables: u64,
    /// Lyrics exist but every timing point sits at tick 0 — unusable for highlighting.
    all_at_zero: usize,
    /// Lyrics exist but the file has no notes.
    no_notes: usize,
    /// Lyrics exist but every syllable ends with a space, so no word end is marked.
    ///
    /// **This is the instrument the detection's thresholds are set from.** A sweep says how many
    /// files the rule claims, which is the only way to see a false positive before a singer does.
    every_syllable_spaced: usize,
    /// Lyrics exist and carry no space at all, which marks no word end either.
    ///
    /// Counted apart from the rule above because the two are separate claims on a corpus, and a
    /// sweep that summed them could not say which rule had reached a file.
    none_spaced: usize,
    /// Files whose lines are opened with a bracket, so the bracket is read as a mark.
    ///
    /// The same instrument as `every_syllable_spaced` one rule over: a sweep is the only way to see a rule
    /// claiming a file it should have left alone before somebody watching a television does.
    angle_lines: usize,
    /// Files written in chords, whose chord events are dropped rather than sung.
    chord_annotations: usize,
    /// Files with harmonica tabs among their words, whose tabs are dropped.
    harmonica_tabs: usize,
    /// Files where a space mark reached a syllable, which is the count that must be nothing.
    space_mark_files: usize,
    /// Every space mark that reached a syllable, by the shape it was written in.
    space_mark_shape: BTreeMap<String, usize>,
    /// Melody channel detected, keyed by channel number.
    melody_channel: BTreeMap<String, usize>,
    /// Reason detection abstained, keyed by reason.
    melody_abstained: BTreeMap<String, usize>,
    /// Suitability distribution.
    suitability: BTreeMap<String, usize>,
    /// How long a file is sung for, first counted syllable to last, in buckets of five seconds.
    ///
    /// **This is the instrument `min_sung_ms` is set from**, and it is asked only of files that
    /// score for their lyrics: a file already condemned for a business card in its lyric track says
    /// nothing about where the line between a song and a fragment falls.
    sung_seconds: BTreeMap<String, usize>,
    /// Of the same files, how many are sung for less than each candidate minimum.
    ///
    /// The blast radius of every candidate at once: choosing one takes six points off this many
    /// files.
    sung_under: BTreeMap<String, usize>,
    /// Warning codes raised, across all files.
    warning: BTreeMap<String, usize>,
    /// Sum of suitabilities, for the mean.
    suitability_total: u64,
    /// Files that say where their own lines go and carry words.
    ///
    /// The denominator for the three tallies below, which are about a file that marks its lines and
    /// then stops. Such a file is trusted line by line, so a stretch it marked nothing across
    /// reaches the screen whole, and how wide that gets is a question only a corpus answers.
    marked_files: usize,
    /// Line length in characters, over files that mark their own lines, in buckets of ten.
    marked_line_chars: BTreeMap<String, usize>,
    /// How long a line is held, over the same lines, in buckets of one second.
    marked_line_seconds: BTreeMap<String, usize>,
    /// Files that mark their lines and still hold one past each candidate bound.
    ///
    /// The blast radius of every bound at once: re-breaking at one of these touches this many files.
    marked_over: BTreeMap<String, usize>,
    /// How much wider a line break the file wrote is than that file's ordinary syllable step.
    ///
    /// **A file that says where its lines go has said what a pause is worth in it**, and this is that
    /// answer over a corpus rather than over one song. It is what sets `PHRASE_GAP_MULTIPLE`: the
    /// multiple has to sit below where a file's own breaks land and above its within-line steps.
    marked_break_multiple: BTreeMap<String, usize>,
    failures: Vec<String>,
}

/// Parse options for a sweep, honoring the parser by default and the file alone on request.
///
/// **`RUNAWAY_LINE_CHARS` and `PHRASE_GAP_MULTIPLE` are set from a sweep, so setting them needs one
/// that neither has touched** — a run past the bound is otherwise re-broken before the tally sees
/// it, and the tail measured is the one the bound already cut. That is what `as_written` asks for,
/// and it is the only question it answers: every other reading wants the lines the machine will
/// draw, so it is off unless asked for.
///
/// `default_hold_ticks` is the one threshold that comes from a file's timebase rather than from what
/// reads well, and naming the options means guessing it. It bounds the last syllable of a song and
/// nothing else, so the guess costs one line's held time per file and touches no width at all.
fn sweep_options(as_written: bool) -> ParseOptions {
    ParseOptions {
        declared_encoding: None,
        inference: as_written.then(|| LineInference {
            runaway_line_chars: usize::MAX,
            ..LineInference::for_ticks_per_quarter(480)
        }),
    }
}

/// Line widths a bound might be drawn at, in characters.
///
/// Wide enough apart to show the shape of the tail rather than to sample it evenly: the question is
/// where a marked file stops being a file with long lines and starts being a file that stopped
/// speaking, and that shoulder is what these have to bracket.
const CANDIDATE_BOUNDS: [usize; 8] = [55, 70, 85, 100, 120, 150, 200, 300];

/// Lengths a minimum for the sung span might be drawn at, in seconds.
///
/// Spread around three quarters of a minute for the same reason [`CANDIDATE_BOUNDS`] is spread
/// around a line width: the question is where a file stops being a short song and starts being a
/// fragment, and these have to bracket that shoulder rather than sample the range evenly.
const CANDIDATE_SUNG_SECONDS: [usize; 6] = [20, 30, 40, 45, 60, 75];

impl Stats {
    fn merge(&mut self, other: Stats) {
        self.files += other.files;
        self.parsed += other.parsed;
        self.failed += other.failed;
        self.panicked += other.panicked;
        self.with_title += other.with_title;
        self.with_artist += other.with_artist;
        self.total_lines += other.total_lines;
        self.total_syllables += other.total_syllables;
        self.all_at_zero += other.all_at_zero;
        self.no_notes += other.no_notes;
        self.every_syllable_spaced += other.every_syllable_spaced;
        self.none_spaced += other.none_spaced;
        self.angle_lines += other.angle_lines;
        self.chord_annotations += other.chord_annotations;
        self.harmonica_tabs += other.harmonica_tabs;
        self.space_mark_files += other.space_mark_files;
        self.marked_files += other.marked_files;
        merge_counts(&mut self.marked_line_chars, other.marked_line_chars);
        merge_counts(&mut self.marked_line_seconds, other.marked_line_seconds);
        merge_counts(&mut self.marked_over, other.marked_over);
        merge_counts(&mut self.marked_break_multiple, other.marked_break_multiple);
        merge_counts(&mut self.space_mark_shape, other.space_mark_shape);
        self.suitability_total += other.suitability_total;
        merge_counts(&mut self.melody_channel, other.melody_channel);
        merge_counts(&mut self.melody_abstained, other.melody_abstained);
        merge_counts(&mut self.suitability, other.suitability);
        merge_counts(&mut self.sung_seconds, other.sung_seconds);
        merge_counts(&mut self.sung_under, other.sung_under);
        merge_counts(&mut self.warning, other.warning);
        merge_counts(&mut self.flavor, other.flavor);
        merge_counts(&mut self.encoding, other.encoding);
        merge_counts(&mut self.encoding_source, other.encoding_source);
        merge_counts(&mut self.granularity, other.granularity);
        merge_counts(&mut self.failure_reason, other.failure_reason);
        for (reason, examples) in other.failure_examples {
            self.failure_examples
                .entry(reason)
                .or_default()
                .extend(examples);
        }
        self.failures.extend(other.failures);
    }

    fn record(&mut self, path: &Path, bytes: &[u8], examples: usize, as_written: bool) {
        self.files += 1;

        // A corpus this size will contain files that break assumptions; one bad file must not end
        // the scan, so a panic is caught and counted like any other failure.
        let options = sweep_options(as_written);
        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Song::parse(bytes, &options)
        }));

        let song = match parsed {
            Ok(Ok(song)) => song,
            Ok(Err(error)) => {
                self.failed += 1;
                self.note_failure(path, &error.to_string(), examples);
                return;
            }
            Err(_) => {
                self.panicked += 1;
                let reason = last_panic().unwrap_or_else(|| "panic".to_owned());
                self.note_failure(path, &format!("PANIC: {reason}"), examples);
                return;
            }
        };

        self.parsed += 1;
        *self.flavor.entry(format!("{:?}", song.flavor)).or_default() += 1;
        *self
            .encoding
            .entry(song.decoder.name().to_owned())
            .or_default() += 1;
        *self
            .encoding_source
            .entry(format!("{:?}", song.decoder.source()))
            .or_default() += 1;
        *self
            .granularity
            .entry(format!("{:?}", song.lyrics.granularity()))
            .or_default() += 1;

        if song.meta.title.is_some() {
            self.with_title += 1;
        }
        if song.meta.artist.is_some() {
            self.with_artist += 1;
        }
        self.total_lines += song.lyrics.line_count() as u64;
        self.total_syllables += song.lyrics.syllable_count() as u64;

        let analysis = Analysis::of(&song);
        match &analysis.melody {
            MelodyOutcome::Found(melody) => {
                *self
                    .melody_channel
                    .entry(format!("channel {:>2}", melody.channel))
                    .or_default() += 1;
            }
            MelodyOutcome::Abstained { abstained } => {
                *self
                    .melody_abstained
                    .entry(abstention_name(*abstained).to_owned())
                    .or_default() += 1;
            }
        }
        *self
            .suitability
            .entry(format!("{:>2}/10", analysis.suitability.value))
            .or_default() += 1;
        self.suitability_total += u64::from(analysis.suitability.value);
        for warning in &analysis.suitability.warnings {
            *self
                .warning
                .entry(format!("{:?}", warning.code))
                .or_default() += 1;
        }

        // Asked only of a file whose lyrics score, so the histogram describes songs rather than the
        // business cards and chord charts the quantity test has already settled.
        if analysis.suitability.breakdown.lyrics > 0 {
            let seconds = (km_suitability::sung_span_ms(&song) / 1_000) as usize;
            *self.sung_seconds.entry(bucket(seconds, 5)).or_default() += 1;
            for least in CANDIDATE_SUNG_SECONDS {
                if seconds < least {
                    *self
                        .sung_under
                        .entry(format!("under {least:>3}s"))
                        .or_default() += 1;
                }
            }
        }

        if song.lyrics.granularity() != LyricGranularity::None {
            if song.lyrics.syllable_ticks().iter().all(|&t| t == 0) {
                self.all_at_zero += 1;
            }
            if song.note_count() == 0 {
                self.no_notes += 1;
            }
            match song.lyrics.word_ends {
                WordEnds::AsWritten => {}
                WordEnds::EverySyllableSpaced => self.every_syllable_spaced += 1,
                WordEnds::NoneSpaced => self.none_spaced += 1,
            }
            if song.dialect.angle_starts_lines {
                self.angle_lines += 1;
            }
            if song.dialect.annotations_are_marked {
                self.chord_annotations += 1;
            }
            if song.dialect.harmonica_tabs {
                self.harmonica_tabs += 1;
            }
            if song.lyrics.lines_are_marked {
                self.record_marked_lines(&song);
            }
        }

        // Nothing should be counted here: `km_song::spacing::resolve` runs where a syllable is
        // written. The tally is what makes that a claim about a corpus rather than about fixtures,
        // and it is how the shapes the rule answers for were found in the first place.
        let mut marked = false;
        for line in &song.lyrics.lines {
            for syllable in &line.syllables {
                for mark in km_song::spacing::marks(&syllable.text) {
                    *self
                        .space_mark_shape
                        .entry(mark.name().to_owned())
                        .or_default() += 1;
                    marked = true;
                }
            }
        }
        if marked {
            self.space_mark_files += 1;
        }
    }

    /// Tallies the lines of one file that says where its lines go.
    ///
    /// Width in characters and how long the line is held are recorded separately because they
    /// disagree: a slow line of few words is long in seconds and short on the screen, and only the
    /// second of those is what runs off the edge.
    fn record_marked_lines(&mut self, song: &Song) {
        self.marked_files += 1;
        let mut widest = 0usize;
        for line in &song.lyrics.lines {
            let chars = line.text().chars().count();
            widest = widest.max(chars);
            *self.marked_line_chars.entry(bucket(chars, 5)).or_default() += 1;
            let held_ms = song
                .tempo_map
                .tick_to_ms(line.end_tick)
                .saturating_sub(song.tempo_map.tick_to_ms(line.start_tick));
            *self
                .marked_line_seconds
                .entry(bucket(held_ms as usize / 1_000, 1))
                .or_default() += 1;
        }
        for bound in CANDIDATE_BOUNDS {
            if widest > bound {
                *self.marked_over.entry(format!("{bound:>4}")).or_default() += 1;
            }
        }
        self.record_break_multiples(song);
    }

    /// Tallies how much wider this file's own line breaks are than its ordinary syllable step.
    ///
    /// The step is the median gap *within* lines, so a break the file wrote cannot raise the figure
    /// it is measured against. A file whose syllables all sit at one tick has no step to speak of
    /// and is skipped rather than counted as an infinite multiple.
    fn record_break_multiples(&mut self, song: &Song) {
        let ms = |tick| song.tempo_map.tick_to_ms(tick);
        let mut within: Vec<u32> = Vec::new();
        for line in &song.lyrics.lines {
            for pair in line.syllables.windows(2) {
                within.push(ms(pair[1].start_tick).saturating_sub(ms(pair[0].start_tick)));
            }
        }
        within.sort_unstable();
        let step = within.get(within.len() / 2).copied().unwrap_or(0);
        if step == 0 {
            return;
        }
        for pair in song.lyrics.lines.windows(2) {
            let Some(last) = pair[0].syllables.last() else {
                continue;
            };
            let gap = ms(pair[1].start_tick).saturating_sub(ms(last.start_tick));
            // In tenths, so the shoulder between a within-line step and a break is readable.
            let tenths = u64::from(gap) * 10 / u64::from(step);
            *self
                .marked_break_multiple
                .entry(bucket(usize::try_from(tenths).unwrap_or(usize::MAX), 5))
                .or_default() += 1;
        }
    }

    fn note_failure(&mut self, path: &Path, reason: &str, examples: usize) {
        let key = normalize_reason(reason);
        *self.failure_reason.entry(key.clone()).or_default() += 1;
        let bucket = self.failure_examples.entry(key).or_default();
        if bucket.len() < examples {
            bucket.push(path.display().to_string());
        }
        self.failures
            .push(format!("{}\t{}", path.display(), reason));
    }
}

fn abstention_name(abstention: Abstention) -> &'static str {
    match abstention {
        Abstention::NoCandidates => "no candidates (no non-drum notes)",
        Abstention::NothingMonophonic => "nothing monophonic",
        Abstention::OutsideVocalRange => "monophonic but outside singing range",
        Abstention::SilentUnderTheWords => "silent while the words are sung",
        Abstention::NoSupportingEvidence => "no supporting evidence",
        Abstention::Ambiguous => "ambiguous (two plausible channels)",
    }
}

/// The label of the bucket `value` falls in, padded so the labels sort in value order.
fn bucket(value: usize, size: usize) -> String {
    format!("{:>6}", value / size * size)
}

/// The bucket floor below which `fraction` of a histogram's entries fall.
///
/// The labels are read back as numbers, which is what the padding in [`bucket`] is for: a histogram
/// merged across worker threads is a map of labels, and a percentile needs them in value order.
fn percentile(histogram: &BTreeMap<String, usize>, fraction: f64) -> usize {
    let total: usize = histogram.values().sum();
    if total == 0 {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "a rank within a corpus-sized count, and the clamp keeps it in range"
    )]
    let target = ((total as f64 * fraction).ceil() as usize).clamp(1, total);
    let mut seen = 0usize;
    for (label, count) in histogram {
        seen += count;
        if seen >= target {
            return label.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn merge_counts(into: &mut BTreeMap<String, usize>, from: BTreeMap<String, usize>) {
    for (key, count) in from {
        *into.entry(key).or_default() += count;
    }
}

/// Collapses error messages that differ only in their details, so the tally stays readable.
fn normalize_reason(reason: &str) -> String {
    let cut = reason.find(" at ").or_else(|| reason.find(": "));
    match cut {
        Some(i) if i > 12 => reason[..i].to_owned(),
        _ => reason.to_owned(),
    }
}

static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

fn last_panic() -> Option<String> {
    LAST_PANIC.lock().ok().and_then(|mut slot| slot.take())
}

fn scan(args: &ScanArgs) -> Result<()> {
    let extensions: Vec<String> = args
        .ext
        .split(',')
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();

    eprint!("collecting files under {} ... ", args.dir.display());
    let mut paths = Vec::new();
    collect(&args.dir, &extensions, args.limit, &mut paths)?;
    eprintln!("{} file(s)", paths.len());

    // A corpus scan would otherwise print a panic message per bad file; record it instead.
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut slot) = LAST_PANIC.lock() {
            *slot = Some(info.to_string());
        }
    }));

    let jobs = args.jobs.max(1);
    let chunk_size = paths.len().div_ceil(jobs).max(1);
    let stats = std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut stats = Stats::default();
                    for path in chunk {
                        match std::fs::read(path) {
                            Ok(bytes) => stats.record(path, &bytes, args.examples, args.as_written),
                            Err(error) => {
                                stats.files += 1;
                                stats.failed += 1;
                                stats.note_failure(
                                    path,
                                    &format!("read error: {error}"),
                                    args.examples,
                                );
                            }
                        }
                    }
                    stats
                })
            })
            .collect();

        let mut total = Stats::default();
        for handle in handles {
            match handle.join() {
                Ok(stats) => total.merge(stats),
                Err(_) => eprintln!("warning: a worker thread died; results are incomplete"),
            }
        }
        total
    });

    let _ = std::panic::take_hook();
    report(&stats, args.as_written);

    if let Some(path) = &args.failures {
        let mut file =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        for line in &stats.failures {
            writeln!(file, "{line}")?;
        }
        eprintln!(
            "\nwrote {} failure line(s) to {}",
            stats.failures.len(),
            path.display()
        );
    }
    Ok(())
}

// --- preview ---------------------------------------------------------------------------------

/// How many lines a package's preview holds. Mirrors `km_pack`'s own constant; this command exists to
/// measure exactly what that will store.
const PREVIEW_LINES: usize = 2;

/// What a corpus run learned about previews.
#[derive(Default)]
struct PreviewStats {
    /// Files walked.
    files: usize,
    /// Files that parsed.
    parsed: usize,
    /// Files with a lyric timeline at all.
    with_lyrics: usize,
    /// How many preview lines each song ended up with, `0..=PREVIEW_LINES`.
    lines: BTreeMap<usize, usize>,
    /// How many leading lines were skipped, tallied by count.
    skips: BTreeMap<usize, usize>,
    /// Songs where the skip budget ran out, so the classifier gave up and took what was there.
    budget_exhausted: usize,
    /// Every skipped line, tallied. Only collected with `--skipped`, because a full corpus produces
    /// hundreds of thousands of distinct ones.
    skipped_lines: BTreeMap<String, usize>,
    /// The first line each preview *kept*, tallied.
    ///
    /// **This is the measurement that finds what the rules are missing**, and it works because real
    /// lyrics differ per song: a first line shared by hundreds of files is boilerplate that survived,
    /// not a coincidence. It is how the multi-line legal notices were found — only their opening line
    /// matched a rule, and the continuation came through.
    kept_first: BTreeMap<String, usize>,
    /// Sampled previews to read: the skipped lines, then what was kept.
    examples: Vec<String>,
}

impl PreviewStats {
    fn record(&mut self, path: &Path, bytes: &[u8], args: &PreviewArgs) {
        self.files += 1;
        let Ok(song) = std::panic::catch_unwind(|| Song::parse(bytes, &ParseOptions::default()))
            .unwrap_or_else(|_| {
                let _ = last_panic();
                Err(km_song::SongError::NoTracks)
            })
        else {
            return;
        };
        self.parsed += 1;
        if song.lyrics.is_empty() {
            return;
        }
        self.with_lyrics += 1;

        let preview = song.lyrics.preview(PREVIEW_LINES);

        // What was skipped, worked out by comparing against the raw first lines. Recomputed rather
        // than returned by `preview`, so the measured code path is exactly the shipped one.
        let raw: Vec<String> = song
            .lyrics
            .lines
            .iter()
            .map(|line| line.text().trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect();
        let kept_first = preview.first();
        let skipped = kept_first
            .and_then(|first| {
                raw.iter()
                    .position(|line| line.starts_with(first.trim_end_matches('…')))
            })
            .unwrap_or(0);

        *self.lines.entry(preview.len()).or_default() += 1;
        *self.skips.entry(skipped).or_default() += 1;
        if skipped >= km_song::timeline::PREVIEW_MAX_SKIP {
            self.budget_exhausted += 1;
        }
        if let Some(first) = kept_first {
            *self.kept_first.entry(first.clone()).or_default() += 1;
        }
        if args.skipped {
            for line in raw.iter().take(skipped) {
                *self.skipped_lines.entry(line.clone()).or_default() += 1;
            }
        }
        // Sampled from the songs that actually exercised the rules, which are the telling ones.
        if skipped > 0 && self.examples.len() < args.examples {
            let mut block = format!("  {}\n", path.display());
            for line in raw.iter().take(skipped) {
                block.push_str(&format!("    skipped: {line}\n"));
            }
            for line in &preview {
                block.push_str(&format!("    kept:    {line}\n"));
            }
            self.examples.push(block);
        }
    }

    fn merge(&mut self, other: Self) {
        self.files += other.files;
        self.parsed += other.parsed;
        self.with_lyrics += other.with_lyrics;
        self.budget_exhausted += other.budget_exhausted;
        for (key, count) in other.lines {
            *self.lines.entry(key).or_default() += count;
        }
        for (key, count) in other.skips {
            *self.skips.entry(key).or_default() += count;
        }
        for (key, count) in other.skipped_lines {
            *self.skipped_lines.entry(key).or_default() += count;
        }
        for (key, count) in other.kept_first {
            *self.kept_first.entry(key).or_default() += count;
        }
        self.examples.extend(other.examples);
    }
}

fn preview(args: &PreviewArgs) -> Result<()> {
    let extensions: Vec<String> = args
        .ext
        .split(',')
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();

    eprint!("collecting files under {} ... ", args.dir.display());
    let mut paths = Vec::new();
    collect(&args.dir, &extensions, args.limit, &mut paths)?;
    eprintln!("{} file(s)", paths.len());

    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut slot) = LAST_PANIC.lock() {
            *slot = Some(info.to_string());
        }
    }));

    let jobs = args.jobs.max(1);
    let chunk_size = paths.len().div_ceil(jobs).max(1);
    let stats = std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut stats = PreviewStats::default();
                    for path in chunk {
                        match std::fs::read(path) {
                            Ok(bytes) => stats.record(path, &bytes, args),
                            Err(_) => stats.files += 1,
                        }
                    }
                    stats
                })
            })
            .collect();

        let mut total = PreviewStats::default();
        for handle in handles {
            match handle.join() {
                Ok(stats) => total.merge(stats),
                Err(_) => eprintln!("warning: a worker thread died; results are incomplete"),
            }
        }
        total
    });
    let _ = std::panic::take_hook();

    println!("files            {}", stats.files);
    println!("parsed           {}", stats.parsed);
    println!("with lyrics      {}", stats.with_lyrics);
    println!(
        "budget exhausted {}   (the classifier gave up and took what was there)",
        stats.budget_exhausted
    );

    let named = |map: &BTreeMap<usize, usize>, label: &str| {
        map.iter()
            .map(|(key, count)| (format!("{label} {key}"), *count))
            .collect::<BTreeMap<String, usize>>()
    };
    print_table(
        "preview length",
        &named(&stats.lines, "lines:"),
        stats.with_lyrics,
    );
    print_table(
        "leading lines skipped",
        &named(&stats.skips, "skipped:"),
        stats.with_lyrics,
    );
    if args.skipped {
        print_table_top(
            "what was skipped",
            &stats.skipped_lines,
            stats.with_lyrics,
            60,
        );
    }
    // A first line shared by many files is boilerplate that got through, because real lyrics differ
    // per song. This is the list to read when deciding whether a rule is missing.
    let shared: BTreeMap<String, usize> = stats
        .kept_first
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(line, count)| (line.clone(), *count))
        .collect();
    print_table_top(
        "first kept line, where more than one file shares it",
        &shared,
        stats.with_lyrics,
        40,
    );
    if !stats.examples.is_empty() {
        println!("\nsampled previews, skipped lines first");
        for block in &stats.examples {
            print!("{block}");
        }
    }
    Ok(())
}

fn collect(
    dir: &Path,
    extensions: &[String],
    limit: Option<usize>,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    if limit.is_some_and(|l| out.len() >= l) {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // An unreadable directory in a huge corpus is not a reason to abandon the scan.
        Err(error) => {
            eprintln!("warning: skipping {}: {error}", dir.display());
            return Ok(());
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, extensions, limit, out)?;
            if limit.is_some_and(|l| out.len() >= l) {
                return Ok(());
            }
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && extensions.iter().any(|want| want.eq_ignore_ascii_case(ext))
        {
            out.push(path);
            if limit.is_some_and(|l| out.len() >= l) {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn report(stats: &Stats, as_written: bool) {
    let pct = |n: usize| {
        if stats.files == 0 {
            0.0
        } else {
            n as f64 * 100.0 / stats.files as f64
        }
    };

    println!("files scanned      {}", stats.files);
    println!(
        "  parsed           {} ({:.2}%)",
        stats.parsed,
        pct(stats.parsed)
    );
    println!(
        "  failed           {} ({:.2}%)",
        stats.failed,
        pct(stats.failed)
    );
    println!(
        "  panicked         {} ({:.2}%)",
        stats.panicked,
        pct(stats.panicked)
    );

    print_table("karaoke format", &stats.flavor, stats.parsed);
    print_table("lyric granularity", &stats.granularity, stats.parsed);
    print_table("encoding decided by", &stats.encoding_source, stats.parsed);
    print_table_top("encoding", &stats.encoding, stats.parsed, 12);

    println!("\nmetadata");
    println!(
        "  with title       {} ({:.2}%)",
        stats.with_title,
        pct(stats.with_title)
    );
    println!(
        "  with artist      {} ({:.2}%)",
        stats.with_artist,
        pct(stats.with_artist)
    );

    if stats.parsed > 0 {
        println!("\nlyrics");
        println!(
            "  mean lines       {:.1}",
            stats.total_lines as f64 / stats.parsed as f64
        );
        println!(
            "  mean syllables   {:.1}",
            stats.total_syllables as f64 / stats.parsed as f64
        );
        println!(
            "  all at tick 0    {} (unusable for highlighting)",
            stats.all_at_zero
        );
        println!("  lyrics, no notes {}", stats.no_notes);
        println!(
            "  every word spaced {} (a space after every syllable, so the words are drawn divided)",
            stats.every_syllable_spaced
        );
        println!(
            "  nothing spaced   {} (no space at all, so the words are drawn divided)",
            stats.none_spaced
        );
        println!(
            "  bracket lines    {} ({:.2}%) (a file whose lines are opened with `<`)",
            stats.angle_lines,
            pct(stats.angle_lines)
        );
        println!(
            "  chords dropped   {} ({:.2}%) (a file written in chord symbols)",
            stats.chord_annotations,
            pct(stats.chord_annotations)
        );
        println!(
            "  harmonica tabs   {} ({:.2}%) (a file with harmonica tabs among its words)",
            stats.harmonica_tabs,
            pct(stats.harmonica_tabs)
        );

        println!(
            "  space marks left {} file(s) ({:.2}%)",
            stats.space_mark_files,
            pct(stats.space_mark_files)
        );
        print_table_top("  by shape", &stats.space_mark_shape, stats.parsed, 8);

        if stats.marked_files > 0 {
            let lines: usize = stats.marked_line_chars.values().sum();
            println!(
                "\nlines a file placed itself, {} ({} file(s), {lines} line(s))",
                if as_written {
                    "as written"
                } else {
                    "as the machine will draw them"
                },
                stats.marked_files
            );
            println!("  width, characters, at the floor of a five-wide bucket");
            for fraction in [0.50, 0.75, 0.90, 0.95, 0.99, 0.999] {
                println!(
                    "    p{:<5} {:>6}",
                    format!("{:.1}", fraction * 100.0),
                    percentile(&stats.marked_line_chars, fraction)
                );
            }
            println!("  held, whole seconds");
            for fraction in [0.50, 0.90, 0.99, 0.999] {
                println!(
                    "    p{:<5} {:>6}",
                    format!("{:.1}", fraction * 100.0),
                    percentile(&stats.marked_line_seconds, fraction)
                );
            }
            println!("  a break the file wrote, as a multiple of its own syllable step");
            for fraction in [0.01, 0.05, 0.10, 0.25, 0.50, 0.90] {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a bucket floor in tenths, well inside f64"
                )]
                let multiple = percentile(&stats.marked_break_multiple, fraction) as f64 / 10.0;
                println!(
                    "    p{:<5} {multiple:>6.1}x",
                    format!("{:.0}", fraction * 100.0)
                );
            }
            println!("  files holding a line wider than");
            for bound in CANDIDATE_BOUNDS {
                let over = stats
                    .marked_over
                    .get(&format!("{bound:>4}"))
                    .copied()
                    .unwrap_or(0);
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a percentage of a corpus-sized count"
                )]
                let share = over as f64 * 100.0 / stats.marked_files as f64;
                println!("    {bound:>4} chars  {over:>7} ({share:.2}%)");
            }
        }
    }

    let detected: usize = stats.melody_channel.values().sum();
    println!(
        "
melody detection"
    );
    println!(
        "  detected         {} ({:.2}% of parsed)",
        detected,
        if stats.parsed == 0 {
            0.0
        } else {
            detected as f64 * 100.0 / stats.parsed as f64
        }
    );
    print_table_top("  by channel", &stats.melody_channel, stats.parsed, 8);
    print_table_top(
        "  abstained because",
        &stats.melody_abstained,
        stats.parsed,
        8,
    );

    if stats.parsed > 0 {
        println!(
            "
suitability (mean {:.2}/10)",
            stats.suitability_total as f64 / stats.parsed as f64
        );
        let mut rows: Vec<_> = stats.suitability.iter().collect();
        rows.sort_by_key(|(k, _)| k.as_str());
        for (suitability, count) in rows {
            println!(
                "  {suitability}  {count:>7}  {:>6.2}%",
                *count as f64 * 100.0 / stats.parsed as f64
            );
        }
    }
    print_table_top("warnings raised", &stats.warning, stats.parsed, 20);

    let sung: usize = stats.sung_seconds.values().sum();
    if sung > 0 {
        println!("\nhow long a file is sung for ({sung} file(s) scoring for their lyrics)");
        println!("  seconds, at the floor of a five-wide bucket");
        for fraction in [0.001, 0.01, 0.05, 0.10, 0.25, 0.50] {
            println!(
                "    p{:<5} {:>6}",
                format!("{:.1}", fraction * 100.0),
                percentile(&stats.sung_seconds, fraction)
            );
        }
        // Only the low end is printed. The shoulder a minimum has to sit on is down here, and the
        // long tail of ordinary songs says nothing about where it falls.
        println!("  the low end, bucket by bucket");
        for (seconds, count) in &stats.sung_seconds {
            if seconds.trim().parse::<usize>().is_ok_and(|s| s < 120) {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a percentage of a corpus-sized count"
                )]
                let share = *count as f64 * 100.0 / sung as f64;
                println!("    {seconds}s  {count:>7} ({share:.2}%)");
            }
        }
        println!("  files sung for less than");
        for least in CANDIDATE_SUNG_SECONDS {
            let under = stats
                .sung_under
                .get(&format!("under {least:>3}s"))
                .copied()
                .unwrap_or(0);
            #[expect(
                clippy::cast_precision_loss,
                reason = "a percentage of a corpus-sized count"
            )]
            let share = under as f64 * 100.0 / sung as f64;
            println!("    {least:>3}s  {under:>7} ({share:.2}%)");
        }
    }

    if !stats.failure_reason.is_empty() {
        println!("\nfailure reasons");
        let mut reasons: Vec<_> = stats.failure_reason.iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(a.1));
        for (reason, count) in reasons {
            println!("  {count:>7}  {reason}");
            for example in stats.failure_examples.get(reason).into_iter().flatten() {
                println!("           e.g. {example}");
            }
        }
    }
}

fn print_table(title: &str, counts: &BTreeMap<String, usize>, total: usize) {
    print_table_top(title, counts, total, usize::MAX);
}

fn print_table_top(title: &str, counts: &BTreeMap<String, usize>, total: usize, limit: usize) {
    if counts.is_empty() {
        return;
    }
    println!("\n{title}");
    let mut rows: Vec<_> = counts.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in rows.into_iter().take(limit) {
        let share = if total == 0 {
            0.0
        } else {
            *count as f64 * 100.0 / total as f64
        };
        println!("  {count:>7}  {share:>6.2}%  {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // **This module exists because this crate had none**, which is worth stating plainly: the header
    // at the top of this file says lyric-format variance is the project's top risk and that this
    // tool is how the parser is held against reality. The tool that measures the top risk was the
    // one thing nothing measured.
    //
    // No library split was needed to make it possible, contrary to how this looked from outside: a
    // binary crate reaches its own private items from a `mod tests` beside them. The tests had
    // simply never been written.

    /// A failure tally is only readable if messages differing in their details collapse together.
    ///
    /// The `> 12` guard is the part worth pinning: it stops a short message being cut down to a
    /// stub, which would file unrelated failures under one row.
    #[test]
    fn a_reason_is_cut_at_its_details_but_never_down_to_a_stub() {
        assert_eq!(
            normalize_reason("track header missing at byte 4213"),
            "track header missing"
        );
        assert_eq!(
            normalize_reason("unsupported division: SMPTE 29.97"),
            "unsupported division"
        );
        // Short enough that cutting would leave a stub, so it is kept whole.
        assert_eq!(normalize_reason("bad at 4"), "bad at 4");
        assert_eq!(normalize_reason("io: nope"), "io: nope");
        // Nothing to cut at.
        assert_eq!(normalize_reason("truncated"), "truncated");
    }

    /// Positions read as `m:ss.mmm`, with both fields padded.
    ///
    /// An unpadded field is the failure that matters: `1:5.30` reads as five seconds where it means
    /// five and a bit, and a dump is read by eye against a stopwatch.
    #[test]
    fn a_position_is_padded_so_it_reads_as_a_time() {
        assert_eq!(format_ms(0), "0:00.000");
        assert_eq!(format_ms(5_300), "0:05.300");
        assert_eq!(format_ms(65_030), "1:05.030");
        assert_eq!(format_ms(3_600_000), "60:00.000");
        // Past an hour it goes on counting in minutes rather than growing an hours field, which is
        // right for a song and is what the format string says.
        assert_eq!(format_ms(3_661_001), "61:01.001");
    }

    /// Merging a worker's tally into the total adds rather than replaces.
    ///
    /// `scan`'s parallelism rests on this: each thread counts its own chunk and the counts are
    /// merged, so a `merge_counts` that overwrote would silently report one chunk as the corpus.
    #[test]
    fn merging_counts_adds_them_rather_than_replacing() {
        let mut total = BTreeMap::new();
        merge_counts(
            &mut total,
            BTreeMap::from([("soft".to_owned(), 3), ("lyric".to_owned(), 1)]),
        );
        merge_counts(
            &mut total,
            BTreeMap::from([("soft".to_owned(), 4), ("none".to_owned(), 2)]),
        );
        assert_eq!(total.get("soft"), Some(&7));
        assert_eq!(total.get("lyric"), Some(&1));
        assert_eq!(total.get("none"), Some(&2));
    }

    /// Every abstention has a sentence, and no two share one.
    ///
    /// The tally is keyed by these strings, so two abstentions naming themselves the same way would
    /// merge into one row and the report would understate both.
    #[test]
    fn every_abstention_has_its_own_sentence() {
        let all = [
            Abstention::NoCandidates,
            Abstention::NothingMonophonic,
            Abstention::OutsideVocalRange,
            Abstention::SilentUnderTheWords,
            Abstention::NoSupportingEvidence,
            Abstention::Ambiguous,
        ];
        let mut names: Vec<&str> = all.iter().copied().map(abstention_name).collect();
        assert!(
            names.iter().all(|name| !name.is_empty()),
            "an empty name would be an unlabelled row"
        );
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two abstentions share a sentence");
    }
}
