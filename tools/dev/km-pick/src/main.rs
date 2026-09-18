//! A checkbox list on a terminal, for a shell script that has rows and wants a choice.
//!
//! ```text
//! printf 'a\ton\tAlpha\nb\t\tBeta\n' > rows.txt
//! km-pick --rows rows.txt --max 1 --out chosen.txt
//! ```
//!
//! # What this is, and what it is not
//!
//! **The list is [`inquire::MultiSelect`] and nothing here draws it.** The `[x]` markers, the
//! scrolling, the type-to-filter and the refusal to confirm too many are all that prompt's, and this
//! crate is roughly a hundred lines of adapter: read rows, hand them over, write down what came back.
//! Anything that looks like a feature of this program is a feature of that one.
//!
//! It exists because the caller is a shell script. `tools/dev/soundfont-debug.sh` is what knows the
//! asset cache, `KM_SF2_DIRS` and how to fetch a bank — all shell — and the only part of choosing
//! eight of sixty-three banks that shell cannot do is the checkbox list itself. So that part, and
//! only that part, is Rust.
//!
//! **It knows nothing about SoundFonts**, which is the property that keeps the bank table
//! single-sourced: `crates/machine/km-banks/src/lib.rs` and `tools/setup/soundfont-banks.sh`
//! stay its only two readers, and a third one here would be exactly the drift that arrangement
//! exists to prevent.
//!
//! # The protocol
//!
//! One row per line in the `--rows` file, tab-separated, in the order they should appear:
//!
//! ```text
//! <key> \t <flags> \t <label>
//! ```
//!
//! * `key` — written to `--out` when the row is chosen. It never appears on screen, so it can be
//!   whatever the caller finds easiest to switch on afterwards.
//! * `flags` — comma-separated. `on` starts the row ticked; `no` means it may be seen but not
//!   chosen, and confirming with one ticked is refused with `--refusal` while the list stays up.
//! * `label` — drawn as it stands. The caller has already aligned its columns, because the caller is
//!   the one that knows how wide its values are.
//!
//! **A file rather than stdin, and that is a bug fix rather than a preference.** The rows were read
//! from stdin until a Windows run ate them: crossterm reads `STD_INPUT_HANDLE` there, not `CONIN$`,
//! so the row text arrived as keystrokes — spaces in the labels ticked whatever rows the cursor
//! happened to be on and a newline confirmed the lot, in about a tenth of a second and with no
//! error anywhere. Unix opens `/dev/tty` and would have been fine, which is exactly why it was
//! worth getting wrong. **stdin belongs to the prompt**, on every platform, and the guard in
//! [`main`] now asks whether it is a terminal.
//!
//! # Why the result goes to a file
//!
//! `--out`, never stdout. The prompt owns the terminal for as long as it runs, and a redraw that
//! ended up in a caller's `$(...)` would be a corrupt answer rather than a visible fault. A file has
//! no such question in it: it is written once, after the prompt has given the terminal back.
//!
//! The chosen keys are written **in input order**, whatever order they were ticked in. That is what
//! lets the caller decide what the order means — for the SoundFont slots it is the bank table's own
//! order, which is the ranking.
//!
//! # Exit codes
//!
//! | | |
//! |---|---|
//! | 0 | a choice was made, and `--out` holds it — possibly empty, which is a choice |
//! | 1 | canceled with Esc or Ctrl-C; `--out` is not written |
//! | 2 | the arguments or the rows were unusable |
//! | 3 | there is no terminal to draw on |

use std::fmt;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use inquire::list_option::ListOption;
use inquire::validator::{ErrorMessage, Validation};
use inquire::{InquireError, MultiSelect};

/// Nothing was chosen because the person said so. Not a failure.
const EXIT_CANCELED: u8 = 1;
/// The arguments or the rows were unusable.
const EXIT_USAGE: u8 = 2;
/// There is nowhere to draw a list.
const EXIT_NO_TERMINAL: u8 = 3;

#[derive(Parser, Debug)]
#[command(
    name = "km-pick",
    about = "A checkbox list on a terminal, for a shell script that has rows and wants a choice",
    long_about = "Reads `<key>\\t<flags>\\t<label>` rows from --rows, draws them as a checkbox list, \
                  and writes the chosen keys to --out, one per line, in input order.\n\n\
                  `flags` is comma-separated: `on` starts a row ticked, `no` means it may be seen \
                  but not chosen. The label is drawn as it stands.\n\n\
                  Exit 1 means canceled and nothing was written; 3 means there is no terminal."
)]
struct Cli {
    /// The rows to draw: `<key>\t<flags>\t<label>`, one per line, in the order they appear.
    ///
    /// A file and not stdin, because stdin is where the prompt reads its keys on Windows.
    #[arg(long, value_name = "FILE")]
    rows: PathBuf,

    /// Where to write the chosen keys, one per line, in input order.
    ///
    /// Not stdout: the prompt owns the terminal while it runs, and a redraw in a caller's `$(...)`
    /// would be a corrupt answer rather than a visible fault.
    #[arg(long, value_name = "FILE")]
    out: PathBuf,

    /// What to say above the list. May contain newlines; the last line is a good place for the
    /// caller's own column header, since the labels below it are already aligned.
    #[arg(long, default_value = "Choose")]
    title: String,

    /// Refuse to confirm more than this many. 0 is no limit.
    #[arg(long, default_value_t = 0, value_name = "N")]
    max: usize,

    /// How many rows to show at once.
    #[arg(long, default_value_t = 15, value_name = "N")]
    page_size: usize,

    /// What to say when a `no` row is ticked. `{}` is replaced by that row's key.
    ///
    /// Said at the prompt, with the list still up and every other tick intact — refusing a whole
    /// selection because one row in it was unavailable is how somebody loses eight decisions to one
    /// mistake.
    #[arg(long, default_value = "{} cannot be chosen", value_name = "TEXT")]
    refusal: String,
}

/// One row, as it was read and as the list will show it.
#[derive(Debug)]
struct Row {
    /// What the caller gets back. Never drawn.
    key: String,
    /// What is drawn.
    label: String,
    /// Whether it starts ticked.
    on: bool,
    /// Whether it may be chosen at all. A `no` row is drawn and refused, not hidden — the caller
    /// listing it at all is saying it is worth knowing about.
    choosable: bool,
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// Reads the rows, and says which line was wrong rather than that a line was.
fn parse_rows(input: &str) -> anyhow::Result<Vec<Row>> {
    let mut rows = Vec::new();
    for (number, line) in input.lines().enumerate() {
        // A trailing CR, for the same reason `km_bank` in tools/setup/soundfont-banks.sh strips one:
        // this is fed by a shell script, and on Windows a shell script is one CRLF away from a key
        // that matches nothing.
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let number = number + 1;

        // `splitn(3, …)`, so a label may hold tabs — it is drawn verbatim and this program has no
        // opinion about what is in it.
        let mut fields = line.splitn(3, '\t');
        let key = fields.next().unwrap_or_default();
        let Some(flags) = fields.next() else {
            anyhow::bail!("line {number} has no flags field: rows are <key>\\t<flags>\\t<label>");
        };
        let Some(label) = fields.next() else {
            anyhow::bail!("line {number} has no label field: rows are <key>\\t<flags>\\t<label>");
        };
        if key.is_empty() {
            anyhow::bail!("line {number} has an empty key, and a key is what the caller gets back");
        }
        if label.is_empty() {
            anyhow::bail!("line {number} has an empty label, so it would draw as a blank row");
        }

        let flag = |wanted: &str| flags.split(',').any(|flag| flag.trim() == wanted);
        rows.push(Row {
            key: key.to_owned(),
            label: label.to_owned(),
            on: flag("on"),
            choosable: !flag("no"),
        });
    }
    Ok(rows)
}

fn run(cli: &Cli) -> anyhow::Result<ExitCode> {
    let input = std::fs::read_to_string(&cli.rows)
        .map_err(|error| anyhow::anyhow!("{}: {error}", cli.rows.display()))?;
    let rows = parse_rows(&input)?;
    if rows.is_empty() {
        anyhow::bail!(
            "{} holds no rows, so there is nothing to choose from",
            cli.rows.display()
        );
    }

    // Which rows start ticked, as the indices `with_default` wants. Held in a binding because it is
    // borrowed for as long as the prompt lives.
    let ticked: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.on)
        .map(|(index, _)| index)
        .collect();

    let max = cli.max;
    let refusal = cli.refusal.clone();
    // **Everything is refused at the prompt, never after it.** The list stays up and every other
    // tick survives, so a mistake costs the mistake and not the other seven decisions — which is
    // exactly what an exit after the fact costs.
    let prompt = MultiSelect::new(&cli.title, rows)
        .with_default(&ticked)
        .with_page_size(cli.page_size)
        .with_validator(move |chosen: &[ListOption<&Row>]| {
            // The specific complaint before the general one: "arachno cannot be fetched" says what
            // to do about it where "9 chosen" does not.
            if let Some(refused) = chosen.iter().find(|option| !option.value.choosable) {
                return Ok(Validation::Invalid(ErrorMessage::Custom(
                    refusal.replace("{}", &refused.value.key),
                )));
            }
            if max > 0 && chosen.len() > max {
                return Ok(Validation::Invalid(ErrorMessage::Custom(format!(
                    "{} chosen, and there is room for {max}",
                    chosen.len()
                ))));
            }
            Ok(Validation::Valid)
        });

    match prompt.raw_prompt() {
        Ok(mut chosen) => {
            // By index rather than in the order they were ticked: the caller decides what the order
            // means, and it can only do that if the order is the one it sent.
            chosen.sort_by_key(|option| option.index);
            // An empty choice writes an empty file, which is the answer "none of them". A caller
            // tells that from "changed their mind" by the exit code, not by the file.
            let text: String = chosen
                .iter()
                .map(|option| format!("{}\n", option.value.key))
                .collect();
            std::fs::write(&cli.out, text)?;
            Ok(ExitCode::SUCCESS)
        }
        // Esc and Ctrl-C. Nothing is written, so a caller that finds no file knows the difference
        // between "chose nothing" and "changed their mind".
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            Ok(ExitCode::from(EXIT_CANCELED))
        }
        Err(InquireError::NotTTY) => {
            eprintln!("km-pick: there is no terminal to draw a list on.");
            Ok(ExitCode::from(EXIT_NO_TERMINAL))
        }
        Err(error) => Err(error.into()),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // **stdin as well as stderr, and stdin is the one that matters.** crossterm reads
    // `STD_INPUT_HANDLE` on Windows, so a piped stdin is not merely unhelpful there — its bytes
    // arrive as keystrokes and answer the prompt. That is how the rows themselves once ticked seven
    // banks nobody asked for. Asked before anything is drawn, so the failure is a sentence rather
    // than a selection.
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        eprintln!(
            "km-pick: there is no terminal to draw a list on.\n\
             \x20     The rows come from --rows; stdin and stderr must both be a terminal."
        );
        return ExitCode::from(EXIT_NO_TERMINAL);
    }

    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("km-pick: {error}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_a_key_flags_and_a_label() {
        let rows = parse_rows("colombogmgs2\t\tColombo  261.9 MiB  cached\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "colombogmgs2");
        assert_eq!(rows[0].label, "Colombo  261.9 MiB  cached");
        assert!(!rows[0].on);
    }

    #[test]
    fn the_on_flag_ticks_a_row() {
        let rows = parse_rows("a\ton\tAlpha\nb\t\tBeta\n").unwrap();
        assert!(rows[0].on);
        assert!(!rows[1].on);
    }

    #[test]
    fn the_no_flag_says_a_row_may_be_seen_and_not_chosen() {
        let rows = parse_rows("a\tno\tAlpha\nb\t\tBeta\n").unwrap();
        assert!(!rows[0].choosable);
        assert!(rows[1].choosable);
    }

    /// Both at once, which is not nonsense: a row already in the answer whose bank has since
    /// stopped being fetchable is ticked *and* refused, and the person is told rather than having
    /// it silently dropped.
    #[test]
    fn the_flags_are_a_comma_separated_set_rather_than_one_word() {
        let rows = parse_rows("a\ton,no\tAlpha\n").unwrap();
        assert!(rows[0].on);
        assert!(!rows[0].choosable);
    }

    #[test]
    fn a_label_may_hold_spaces_equals_signs_and_tabs() {
        // The SoundFont caller sends aligned columns and a `<path>=<name>=<volume>` spec is the shape
        // of what it does with the answer, so neither may be special here.
        let rows = parse_rows("k\t\tname = value\tand a tab\n").unwrap();
        assert_eq!(rows[0].label, "name = value\tand a tab");
    }

    #[test]
    fn a_blank_line_is_skipped_rather_than_refused() {
        let rows = parse_rows("a\t\tAlpha\n\nb\t\tBeta\n").unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn a_trailing_cr_is_not_part_of_the_label() {
        let rows = parse_rows("a\t\tAlpha\r\n").unwrap();
        assert_eq!(rows[0].label, "Alpha");
    }

    #[test]
    fn a_row_with_too_few_fields_names_its_line() {
        let error = parse_rows("a\t\tAlpha\nb\ton\n").unwrap_err().to_string();
        assert!(error.contains("line 2"), "{error}");
        assert!(error.contains("label"), "{error}");
    }

    #[test]
    fn an_empty_key_is_refused_because_the_caller_gets_it_back() {
        let error = parse_rows("\t\tAlpha\n").unwrap_err().to_string();
        assert!(error.contains("empty key"), "{error}");
    }

    #[test]
    fn an_empty_label_is_refused_because_it_would_draw_as_nothing() {
        let error = parse_rows("a\ton\t\n").unwrap_err().to_string();
        assert!(error.contains("empty label"), "{error}");
    }
}
