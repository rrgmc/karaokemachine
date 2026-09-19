//! Build, inspect and validate `.kmpkg` song packages.
//!
//! This is where analysis happens. Melody detection and suitability scoring run once, here, and are
//! written into the package — so the machine reads a recorded fact instead of re-deriving it on every
//! play, and a packager can see and correct what was decided.
//!
//! Building from a real corpus means most of the work is refusing things: files that are not MIDI,
//! files with no lyrics, and above all the same recording appearing under several names. A song
//! number is how a singer asks for a song, so a duplicate in the catalog is a defect that has to be
//! caught before the package ships.
//!
//! The pipeline itself lives in this crate's library half (`lib.rs`), because `tools/cmd/km-package-builder`
//! builds the same manifests from the same files and the two must not diverge. What is left here is
//! the command line: argument shapes, and reporting a run in a form a person reads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use km_kmpkg::{
    EditedField, Language, MediaEntry, Package, PackageBuilder, SongEntry, content_hash,
    is_seekable_entry, read_manifest_unchecked,
};
use km_pack::{
    Edits, Rejection, apply_edits, melody_record, read_edits, read_index, suitability_record,
    write_package,
};
use km_song::{ParseOptions, Song};
use km_suitability::Analysis;

#[derive(Parser)]
#[command(
    name = "km-pack",
    about = "Build and validate karaoke song packages",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Describe a folder of songs, writing a package description to edit and then build.
    Spec(SpecArgs),
    /// Build a package from a description.
    Build(BuildArgs),
    /// List what a package contains.
    Inspect(InspectArgs),
    /// Validate a package and report anything wrong with it.
    Check(CheckArgs),
    /// Re-run analysis on an existing package, writing a new one.
    Reanalyze(ReanalyzeArgs),
    /// Write a package's song metadata to a CSV for editing.
    Export(ExportArgs),
    /// Apply an edited CSV back to a package.
    Apply(ApplyArgs),
    /// Correct one song's details.
    Edit(EditArgs),
    /// Print one or more packages as a song book, a printable PDF.
    Book(BookArgs),
}

#[derive(Args)]
struct BookArgs {
    /// The packages to print, in any order.
    ///
    /// Songs from all of them are pooled and sorted by artist within a section per language, so
    /// five volumes reads as one alphabet.
    #[arg(required = true)]
    packages: Vec<PathBuf>,
    /// Where to write the PDF. Defaults to the package name with a `.pdf` extension, or
    /// `songbook.pdf` when there is more than one.
    #[arg(long)]
    out: Option<PathBuf>,
    // `--book-name` rather than `--name`: `--name` on `spec` is the package's display name, so one
    // spelling would mean two things inside one program, and the machine already calls this
    // `--book-name` on `--song-book`. `?name=` on `GET /api/v1/songs/book.pdf` is a third spelling
    // and stays, sitting under a route whose whole subject is the book.
    /// Heading printed at the top left of every page. Defaults to `KaraokeMachine`.
    ///
    /// Names the machine the book is for, as opposed to what the document is called, which is
    /// `--title`.
    #[arg(long)]
    book_name: Option<String>,
    /// What the top of every page says. Defaults to `SONG LIST`.
    #[arg(long)]
    title: Option<String>,
    /// Print a song's language as this when the song does not say.
    ///
    /// Affects the book only. This is not the `default_language` a build writes into a package; use
    /// it when printing a package that has no language recorded.
    #[arg(long, value_name = "CODE")]
    default_language: Option<String>,
    /// Print songs under this bank instead of the one the package id implies: `--bank <PACKAGE>=<N>`.
    ///
    /// Repeatable. `--bank 3` on its own works when only one package was named. Most books need
    /// this: a package's bank comes from its id. Use it for a package whose bank was changed on the
    /// machine, so the book matches what a singer will dial.
    #[arg(long, value_name = "SPEC")]
    bank: Vec<String>,
    // A filter, never a heading: the book is sectioned by language, an open vocabulary has no order
    // to head sections by, and a song with three tags would appear three times or arbitrarily once.
    /// Include only songs carrying any one of these tags. Comma-separated: `--tags rock,brasil`.
    #[arg(long, value_name = "TAGS")]
    tags: Option<String>,
}

#[derive(Args)]
struct ExportArgs {
    /// The package to export.
    package: PathBuf,
    /// Where to write the CSV. Defaults to the package name with a `.csv` extension.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Args)]
struct ApplyArgs {
    /// The package to update.
    package: PathBuf,
    /// The edited CSV.
    #[arg(long)]
    index: PathBuf,
    /// Write here instead of updating the package in place.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Report what would change without writing anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct EditArgs {
    /// The package to update.
    package: PathBuf,
    /// Which song to correct.
    #[arg(long)]
    number: u32,
    /// New title.
    #[arg(long)]
    title: Option<String>,
    /// New performer. Pass an empty string to clear it.
    #[arg(long)]
    artist: Option<String>,
    /// New language, as an ISO 639-1 code (`pt`, `ja`, `und`). Pass an empty string to clear it.
    #[arg(long)]
    language: Option<String>,
    // Add and remove rather than a `--tags` that sets, matching the curation tool's bulk control:
    // a song carries several, so setting would silently drop the ones not named.
    /// Add a tag to this song. Repeatable.
    ///
    /// Tags are converted to slugs: `"Rock & Roll"` becomes `rock-roll`.
    #[arg(long = "add-tag", value_name = "TAG")]
    add_tag: Vec<String>,
    /// Take a tag off this song. Repeatable.
    #[arg(long = "remove-tag", value_name = "TAG")]
    remove_tag: Vec<String>,
    /// New lyric encoding, for text that came out wrong.
    #[arg(long)]
    encoding: Option<String>,
    /// New default transposition, in semitones.
    #[arg(long)]
    transpose: Option<i8>,
    /// Play this song and draw none of its words, or draw them on one detection silenced.
    ///
    /// `--lyrics-hidden true` for a file whose words are mistimed or are not the song's;
    /// `--lyrics-hidden false` to overrule detection the other way.
    #[arg(long = "lyrics-hidden", value_name = "BOOL")]
    lyrics_hidden: Option<bool>,
    /// Write here instead of updating the package in place.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Report what would change without writing anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct SpecArgs {
    /// Folder to scan, recursively.
    dir: PathBuf,
    /// Where to write the description. Defaults to `<dir>/<name>.kmspec.yaml`.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Package identifier. Reinstalling the same identifier replaces it, so keep it stable.
    ///
    /// Generated, and sixteen hexadecimal characters is the only shape a package may carry. Give one
    /// only to rebuild a package that already exists under it.
    #[arg(long)]
    id: Option<String>,
    /// Display name. Defaults to the folder's own name.
    #[arg(long)]
    name: Option<String>,
    /// Package version.
    #[arg(long, default_value = "1.0.0")]
    package_version: String,
    /// Who made it.
    #[arg(long)]
    publisher: Option<String>,
    /// First song number to assign. A package numbers its songs 1 to 999; the machine adds the bank.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=i64::from(km_songcode::MAX_SLOT)))]
    start_number: u32,
    // Mostly superseded by the description itself, which says all of this and can be edited in
    // place. It stays because an existing index is a spreadsheet somebody already filled in.
    /// A CSV that overrides what is read from the files.
    ///
    /// Needs a `file` column, plus any of `number`, `title`, `artist`, `language`, `encoding`.
    #[arg(long)]
    index: Option<PathBuf>,
    /// Language to use for any song whose language could not be detected, as an ISO 639-1 code.
    ///
    /// A build refuses a song with no language, so this is required for most folders. Use `und` for
    /// undetermined, or a real code such as `pt` when the folder is all one language.
    ///
    /// Fills gaps only: a song that already has a language keeps it, and the field is not marked as
    /// hand-edited.
    #[arg(long)]
    default_language: Option<String>,
    /// Leave out songs whose suitability is below this, out of 10.
    #[arg(long)]
    min_suitability: Option<u8>,
    /// Leave out files with no lyrics at all, which cannot be sung from.
    #[arg(long)]
    require_lyrics: bool,
    /// Force the lyric encoding for every song, rather than detecting it.
    #[arg(long)]
    encoding: Option<String>,
    /// Stop after this many songs.
    #[arg(long)]
    limit: Option<usize>,
    // Content hash rather than number or path: numbers get re-flowed and folders reorganized while
    // the bytes of a recording do not.
    /// Copy titles, artists and numbers from an existing package.
    ///
    /// Only the fields that package records as hand-edited, matched to songs by content hash.
    #[arg(long)]
    from: Option<PathBuf>,
    /// What the description should name as the package to build. Defaults to `<id>.kmpkg`.
    #[arg(long)]
    package_out: Option<String>,
    // Re-encoding is on by default so the machine only ever meets one profile. It is rarely
    // expensive: a download asked for AVC and AAC is already in profile and is copied either way.
    /// Store videos as they are instead of re-encoding ones outside the packaging profile.
    ///
    /// A video the machine cannot play at all is still refused rather than stored.
    #[arg(long)]
    no_transcode: bool,
    /// Walk the folder and report what would be described, without writing anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct BuildArgs {
    /// The package description to build.
    spec: PathBuf,
    /// Where to write the package, overriding the description's own `out:`.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Report what would be built without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Skip measuring how loud each video and MP3+G song is.
    ///
    /// Measuring decodes every media song's audio in full, so it is the slowest part of building a
    /// package of videos. Songs built without it play at whatever level they were published at,
    /// which is louder than a MIDI song and differs from one publisher to the next.
    #[arg(long)]
    no_loudness: bool,
    /// Write a plain-text listing of the package beside it, as `<package>.kmpkg.txt`.
    ///
    /// The package's name, version and publisher, then a line per song: number, title, artist,
    /// length and language. It is for whoever is handed the package and has no tool to open it.
    #[arg(long)]
    listing: bool,
}

#[derive(Args)]
struct InspectArgs {
    /// The package to look at.
    package: PathBuf,
    /// Show every song rather than a summary.
    #[arg(long)]
    songs: bool,
    /// Emit JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct CheckArgs {
    /// The package to validate.
    package: PathBuf,
    /// Warn about songs whose suitability is below this.
    #[arg(long, default_value_t = 5)]
    min_suitability: u8,
    /// Exit non-zero if any song falls below `--min-suitability`, for use in a build script.
    #[arg(long)]
    strict: bool,
    /// Read every entry and check its bytes against the checksum the package records for it.
    ///
    /// Off by default because it reads the whole package, which for a video library is most of a
    /// minute. It is what answers *did this arrive intact?* — the rest of `check` answers *does this
    /// describe itself properly?*
    #[arg(long)]
    verify: bool,
}

#[derive(Args)]
struct ReanalyzeArgs {
    /// The package to re-analyze.
    package: PathBuf,
    /// Where to write the result.
    ///
    /// Required unless `--dry-run` is given.
    #[arg(long, required_unless_present = "dry_run")]
    out: Option<PathBuf>,
    /// Re-analyze and report what would change, without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Skip re-measuring how loud each video and MP3+G song is.
    ///
    /// Measuring decodes every media song's audio in full, from inside the package. Without it the
    /// levels already in the manifest are carried across untouched, and a package that had none
    /// still has none.
    #[arg(long)]
    no_loudness: bool,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Spec(args) => describe(&args),
        Command::Build(args) => build(&args),
        Command::Inspect(args) => inspect(&args),
        Command::Check(args) => check(&args),
        Command::Reanalyze(args) => reanalyze(&args),
        Command::Export(args) => export(&args),
        Command::Apply(args) => apply(&args),
        Command::Edit(args) => edit(&args),
        Command::Book(args) => book(&args),
    }
}

/// `km-pack book` — one or more packages, printed.
///
/// **No `video` feature and no ffmpeg**, unlike every other command here that meets a video song: a
/// book is made of metadata and never opens a song's bytes, so this works in a plain
/// `cargo run -p km-pack`.
fn book(args: &BookArgs) -> Result<()> {
    let mut packages = Vec::with_capacity(args.packages.len());
    for path in &args.packages {
        packages.push(
            km_pack::book::Loaded::open(path)
                .with_context(|| format!("opening {}", path.display()))?,
        );
    }
    let overrides = km_pack::book::parse_overrides(&args.bank, &packages)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    // Refused by name rather than dropped, the way `--index` refuses one: a word that is not a tag
    // contributes no songs, so dropping it prints a book missing everything that word was meant to
    // bring, and says nothing. A book goes to a printer, where the browsing surfaces redraw.
    let tags: Vec<String> = match args.tags.as_deref() {
        Some(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(|word| {
                km_kmpkg::Tag::parse(word)
                    .map(km_kmpkg::Tag::into_string)
                    .with_context(|| format!("--tags {word:?} is not a tag"))
            })
            .collect::<Result<Vec<String>>>()?,
        None => Vec::new(),
    };

    let (book, warnings) = km_pack::book::build(
        &packages,
        &overrides,
        args.default_language.as_deref(),
        &tags,
        km_pack::book::Naming {
            name: args.book_name.as_deref(),
            title: args.title.as_deref(),
        },
    );

    // **Before the file is written**, so a book whose numbers are wrong is never a surprise
    // discovered by reading the printout.
    //
    // Capped, the way `build` caps its own list. Two packages sharing a bank makes *every* song
    // collide, so an uncapped list is one useful line followed by four thousand restatements of it
    // — and a wall of warnings is one nobody reads to the end of, which is the same as no warning.
    const SHOWN: usize = 10;
    for warning in warnings.iter().take(SHOWN) {
        eprintln!("warning: {warning}");
    }
    if warnings.len() > SHOWN {
        eprintln!("warning: ...and {} more", warnings.len() - SHOWN);
    }
    if !warnings.is_empty() {
        eprintln!();
    }

    let out = args
        .out
        .clone()
        .unwrap_or_else(|| km_pack::book::default_out(&args.packages));
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).with_context(|| format!("making {}", parent.display()))?;
    }
    std::fs::write(&out, book.render()).with_context(|| format!("writing {}", out.display()))?;

    println!(
        "wrote {} with {} song(s) from {} package(s) over {} page(s)",
        out.display(),
        book.row_count(),
        packages.len(),
        book.page_count()
    );
    let replaced = book.replaced();
    if replaced.count > 0 {
        let sample: String = replaced.sample.iter().collect();
        println!(
            "\n{} character(s) could not be drawn and became '?': {sample}\n\
             The book uses the PDF's built-in Helvetica, which covers Western European text only.",
            replaced.count
        );
    }
    Ok(())
}

/// Columns of the editable CSV.
///
/// `number` and `file` identify the row; `hash` is informational and lets a person spot duplicates
/// in a spreadsheet. The rest are editable, and `edited` records which were changed by hand.
const EXPORT_HEADERS: &[&str] = &[
    "number",
    "title",
    "artist",
    "language",
    // Comma-joined inside the cell -- one column for a field a song has several of, unambiguous
    // because a slug cannot contain a comma. `km_pack::read_edits` reads it back the same way.
    "tags",
    "encoding",
    "transpose",
    // Read back, unlike the three below it: `true` and `false` are the whole of a boolean, so this
    // column carries a value rather than a summary of one.
    "lyrics_hidden",
    "suitability",
    "melody",
    // Shown and not read back, as `suitability` and `melody` beside it are. A fix carries arguments
    // and a list of them is not a thing anybody edits in a spreadsheet cell; the curation tool has
    // a control for it. What this column is for is seeing, in one place, which songs of a package
    // are being corrected at all.
    "fixes",
    "file",
    "hash",
    "edited",
];

/// Writes a rebuilt package and says where it went.
///
/// The write itself is [`km_pack::write_package`]; this only reports it, because the library half
/// is shared with a web tool that has no stdout to print to.
fn write_and_report(builder: PackageBuilder, original: &Path, out: Option<&Path>) -> Result<()> {
    let written = write_package(builder, original, out)?;
    match out {
        Some(_) => println!("wrote {}", written.display()),
        None => println!("updated {}", written.display()),
    }
    Ok(())
}

fn export(args: &ExportArgs) -> Result<()> {
    let package = Package::open(&args.package)
        .with_context(|| format!("opening {}", args.package.display()))?;
    let out = args.out.clone().unwrap_or_else(|| {
        let mut path = args.package.clone();
        path.set_extension("csv");
        path
    });

    let mut writer =
        csv::Writer::from_path(&out).with_context(|| format!("writing {}", out.display()))?;
    writer.write_record(EXPORT_HEADERS)?;

    for song in &package.manifest().songs {
        writer.write_record([
            song.number.to_string(),
            song.title.clone(),
            song.artist.clone().unwrap_or_default(),
            song.language.clone().unwrap_or_default(),
            song.tags.join(","),
            song.lyric_encoding.clone().unwrap_or_default(),
            song.default_transpose.to_string(),
            song.lyrics_hidden.to_string(),
            song.suitability
                .as_ref()
                .map(|s| s.value.to_string())
                .unwrap_or_default(),
            song.melody
                .as_ref()
                .map(|m| m.channel.to_string())
                .unwrap_or_default(),
            km_pack::describe_fixes(&song.fixes),
            song.file.clone(),
            song.content_hash.clone().unwrap_or_default(),
            song.edited
                .iter()
                .map(|field| field.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ])?;
    }
    writer.flush()?;

    println!(
        "wrote {} row(s) to {}\n\nEdit the title, artist, language, tags, encoding, transpose and \
         lyrics_hidden columns, \
         then apply them with:\n  km-pack apply {} --index {}",
        package.len(),
        out.display(),
        args.package.display(),
        out.display()
    );
    Ok(())
}

fn apply(args: &ApplyArgs) -> Result<()> {
    let package = Package::open(&args.package)
        .with_context(|| format!("opening {}", args.package.display()))?;
    let file =
        read_edits(&args.index).with_context(|| format!("reading {}", args.index.display()))?;
    let edits = file.edits;

    if !file.bad_language.is_empty() {
        // Reported rather than fatal, and the rest of each row still applies. Same argument as the
        // unmatched-numbers warning below: a spreadsheet is edited at scale, and refusing the whole
        // file over one cell means editing it all again.
        eprintln!(
            "warning: {} row(s) name a language that is not an ISO 639-1 code, and were left \
             unchanged: {}",
            file.bad_language.len(),
            file.bad_language
                .iter()
                .take(10)
                .map(|(number, raw)| format!("{number} ({raw})"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let manifest = package.manifest().clone();
    let mut builder = PackageBuilder::new(manifest.package.clone());
    let mut changes = 0usize;
    let mut unmatched: Vec<u32> = edits.keys().copied().collect();

    for song in &manifest.songs {
        let mut entry = song.clone();
        if let Some(edit) = edits.get(&song.number) {
            unmatched.retain(|number| *number != song.number);
            changes += apply_edits(&mut entry, edit, song);
        }
        // Media is copied across entry for entry -- `raw_copy_file`, so nothing is decoded, nothing
        // re-encoded and the CRC is carried over. This is what `add_out_of_archive` used to achieve
        // by not copying anything at all, and it is where the cost of a package being one file lands:
        // correcting one title now moves the whole archive. Going through `read_song` here is what
        // used to fail outright on any package holding a video song.
        if song.kind.is_midi() {
            let bytes = package.read_song(song.number)?;
            entry.content_hash = None;
            builder.add(entry, bytes)?;
        } else {
            builder.add_media_copied(entry, &package)?;
        }
    }

    if !unmatched.is_empty() {
        // Numbers in the CSV that no song matches are almost always a typo, and silently ignoring
        // them means somebody's correction never lands and nobody finds out.
        eprintln!(
            "warning: {} row(s) name a song number this package does not contain: {}",
            unmatched.len(),
            unmatched
                .iter()
                .take(10)
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    println!(
        "{changes} field(s) changed across {} song(s)",
        builder.len()
    );
    if args.dry_run {
        println!("dry run: nothing written");
        return Ok(());
    }
    write_and_report(builder, &args.package, args.out.as_deref())
}

fn edit(args: &EditArgs) -> Result<()> {
    // Checked before the package is even opened, so a mistyped code fails instantly and names the
    // flag rather than reporting itself somewhere in the middle of a rewrite.
    let language = match args.language.as_deref().map(str::trim) {
        Some("") | None => args.language.clone(),
        Some(raw) => Some(
            Language::parse(raw)
                .with_context(|| {
                    format!(
                        "--language {raw:?} is not an ISO 639-1 code (try `pt`, `ja`, or `und`)"
                    )
                })?
                .code()
                .to_owned(),
        ),
    };

    let package = Package::open(&args.package)
        .with_context(|| format!("opening {}", args.package.display()))?;
    let manifest = package.manifest().clone();
    if manifest.song(args.number).is_none() {
        bail!("this package has no song numbered {}", args.number);
    }

    // An empty `--artist ""` clears the field; omitting the flag leaves it alone. Those are
    // different intentions and the CLI has to be able to express both.
    let optional = |value: &Option<String>| {
        value
            .as_ref()
            .map(|text| (!text.trim().is_empty()).then(|| text.trim().to_owned()))
    };
    // The song's own tags plus the additions minus the removals, so `--add-tag` is additive against
    // what is already there rather than against nothing. `None` when neither flag was given, which
    // is what leaves the field alone — see `Edits::tags`.
    let tags = if args.add_tag.is_empty() && args.remove_tag.is_empty() {
        None
    } else {
        let parse = |flag: &str, raw: &[String]| -> Result<Vec<km_kmpkg::Tag>> {
            raw.iter()
                .map(|value| {
                    km_kmpkg::Tag::parse(value)
                        .with_context(|| format!("--{flag} {value:?} is not a tag"))
                })
                .collect()
        };
        let adding = parse("add-tag", &args.add_tag)?;
        let removing = parse("remove-tag", &args.remove_tag)?;
        let current = manifest
            .song(args.number)
            .map(|song| song.tags.clone())
            .unwrap_or_default();
        let mut held: Vec<km_kmpkg::Tag> = km_kmpkg::tag::parse_list(&current.join(","));
        held.extend(adding);
        held.retain(|tag| !removing.contains(tag));
        held.sort();
        held.dedup();
        Some(held.into_iter().map(km_kmpkg::Tag::into_string).collect())
    };

    let edits = Edits {
        title: args.title.clone(),
        artist: optional(&args.artist),
        language: optional(&language),
        tags,
        encoding: optional(&args.encoding),
        transpose: args.transpose,
        lyrics_hidden: args.lyrics_hidden,
        // No flag sets a fix list. It is a list of structures rather than a value, and the place
        // that offers one is the curation tool.
        fixes: None,
        // Nor the melody channel: naming one is a judgement made while looking at a table of every
        // channel's evidence, which is a page rather than a flag.
        melody: None,
    };

    let mut builder = PackageBuilder::new(manifest.package.clone());
    let mut changed = 0usize;
    for song in &manifest.songs {
        let mut entry = song.clone();
        if song.number == args.number {
            changed = apply_edits(&mut entry, &edits, song);
            println!("  {:>8}  {}", entry.number, entry.title);
            if let Some(artist) = &entry.artist {
                println!("            {artist}");
            }
        }
        // Media is copied across entry for entry -- `raw_copy_file`, so nothing is decoded, nothing
        // re-encoded and the CRC is carried over. This is what `add_out_of_archive` used to achieve
        // by not copying anything at all, and it is where the cost of a package being one file lands:
        // correcting one title now moves the whole archive. Going through `read_song` here is what
        // used to fail outright on any package holding a video song.
        if song.kind.is_midi() {
            let bytes = package.read_song(song.number)?;
            entry.content_hash = None;
            builder.add(entry, bytes)?;
        } else {
            builder.add_media_copied(entry, &package)?;
        }
    }

    if changed == 0 {
        println!("nothing changed");
        return Ok(());
    }
    println!("{changed} field(s) changed");
    if args.dry_run {
        println!("dry run: nothing written");
        return Ok(());
    }
    write_and_report(builder, &args.package, args.out.as_deref())
}

/// Walks a folder and writes down what is in it.
///
/// The step that makes a build reviewable. `km-pack build <folder>` on its own decides which files
/// are songs, what they are called and what number each gets, and packages the answer without
/// showing it to anybody — so the only way to correct a title is to build a package and edit it.
/// This writes the same decisions to a file first.
fn describe(args: &SpecArgs) -> Result<()> {
    // Generated rather than taken from the folder. A folder's name is where people put a client, a
    // party or their own, and a package is handed to a stranger — see `A package says nothing about
    // the machine that built it` in `docs/decisions/packaging.md`.
    //
    // Refused here rather than left to `Manifest::problems`, which would refuse it too: there, the
    // answer arrives after a scan of a corpus and a build, and it is the same mistake either way.
    let id = match args.id.clone() {
        Some(given) if !km_kmpkg::PackageMeta::is_generated_id(&given) => bail!(
            "--id {given:?} is not an id a build generates, and a package carrying one cannot be \
             opened. Leave it out and one is generated; give one only to rebuild a package that \
             already exists under it."
        ),
        Some(given) => given,
        None => km_kmpkg::PackageMeta::new_id(),
    };
    if !args.dir.is_dir() {
        bail!("{} is not a folder", args.dir.display());
    }
    // Validated before the folder is walked, so a typo costs nothing rather than surfacing after a
    // scan of several thousand files.
    let default_language = match args.default_language.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => Some(
            Language::parse(raw)
                .with_context(|| {
                    format!("--default-language {raw:?} is not an ISO 639-1 code (try `und`)")
                })?
                .code()
                .to_owned(),
        ),
        _ => None,
    };
    let index = match &args.index {
        Some(path) => {
            read_index(path).with_context(|| format!("reading the index {}", path.display()))?
        }
        None => BTreeMap::new(),
    };

    // The name still seeds from the folder, and the written description is where somebody sees it
    // and changes it. The file is named from *that* rather than from the id, which is sixteen
    // hexadecimal characters and nothing to pick a description out of a folder by.
    let name = args.name.clone().unwrap_or_else(|| folder_name(&args.dir));
    let stem = km_kmpkg::name_slug(&name).unwrap_or_else(|| id.clone());
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| args.dir.join(format!("{stem}.kmspec.yaml")));

    eprintln!("scanning {}", args.dir.display());
    let described = km_pack::describe(
        &args.dir,
        &km_pack::DescribeOptions {
            id: id.clone(),
            name,
            version: args.package_version.clone(),
            publisher: args.publisher.clone(),
            start_number: args.start_number,
            default_language,
            encoding: args.encoding.clone(),
            transcode: !args.no_transcode,
            out: Some(
                args.package_out
                    .clone()
                    .unwrap_or_else(|| format!("{stem}.kmpkg")),
            ),
            min_suitability: args.min_suitability,
            require_lyrics: args.require_lyrics,
            limit: args.limit,
            index,
        },
        walk_progress(),
    )?;

    let mut spec = described.spec;
    if let Some(package) = &args.from {
        seed_from_package(&mut spec, &args.dir, package)?;
    }
    // Absent when the description sits in the folder it describes, which is the usual case and what
    // makes such a description movable with its songs. Otherwise the folder is named absolutely --
    // a relative walk back up would be shorter to read and wrong the moment either end moves.
    spec.root = root_for(&out, &args.dir);

    report_description(&described.rejected, &described.orphans);

    if args.dry_run {
        println!(
            "\ndry run: nothing written; {} would describe {} song(s)",
            out.display(),
            spec.songs.len()
        );
        return Ok(());
    }

    spec.write(&out)
        .with_context(|| format!("writing {}", out.display()))?;
    println!(
        "\nwrote {} describing {} song(s)",
        out.display(),
        spec.songs.len()
    );
    println!("edit it, then: km-pack build {}", out.display());
    Ok(())
}

/// Builds what a description says.
///
/// The whole of the command: read the file, hand it to the shared builder, report what came back.
/// Every decision about what goes in the package was made when the description was written, which is
/// the point of having one.
fn build(args: &BuildArgs) -> Result<()> {
    let spec = km_pack::Spec::read(&args.spec)?;
    let base = spec.base(&args.spec);

    let outcome = km_pack::build::build(
        &spec,
        &km_pack::BuildOptions {
            base: &base,
            out: args.out.as_deref(),
            dry_run: args.dry_run,
            measure_loudness: !args.no_loudness,
            write_listing: args.listing,
        },
        build_progress(),
    )?;

    report_outcome(&outcome);

    if !outcome.unlanguaged.is_empty() {
        bail!("{}", unlanguaged_message(&outcome));
    }
    if outcome.is_empty() {
        bail!("no songs were accepted, so there is nothing to write");
    }
    if !outcome.problems.is_empty() {
        bail!(
            "the manifest has problems, so nothing was written:\n  {}",
            outcome.problems.join("\n  ")
        );
    }
    if args.dry_run {
        eprintln!("\ndry run: nothing written");
        return Ok(());
    }
    println!(
        "\nwrote {} with {} song(s)",
        outcome.out_path.display(),
        outcome.written
    );
    if let Some(listing) = &outcome.listing_path {
        println!("      {}", listing.display());
    }
    Ok(())
}

/// The folder's own name, as a package's display name.
///
/// **Its case is kept**, because a name is read rather than used to build a file name:
/// `Brasil Volume 1` folded to `brasil volume 1` is a name somebody has to repair by hand. What the
/// description and the package are *called* is `km_kmpkg::name_slug` of this, which folds.
fn folder_name(dir: &Path) -> String {
    dir.canonicalize()
        .ok()
        .as_deref()
        .and_then(Path::file_name)
        .or_else(|| dir.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("package")
        .to_owned()
}

/// What the description's `root:` should say, given where it is going and what it describes.
///
/// `None` when the two are the same folder, which is both the usual case and the one worth keeping
/// tidy: such a description needs no `root:` at all and travels with the songs it names.
fn root_for(out: &Path, dir: &Path) -> Option<String> {
    let here = out.parent().unwrap_or(Path::new("."));
    let same = match (here.canonicalize(), dir.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => here == dir,
    };
    if same {
        return None;
    }
    Some(tidy(dir))
}

/// An absolute path in the spelling a person would type.
///
/// `canonicalize` on Windows returns a `\\?\C:\...` verbatim path, which is correct, accepted by
/// every API here, and alarming in a file somebody is meant to read and edit. The same tidying
/// `km-app` and the curation tool already do at their own edges.
fn tidy(path: &Path) -> String {
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = absolute.display().to_string();
    km_pack::spec::slashed(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// Seeds hand-edited details from an existing package into a fresh description.
///
/// Songs are matched by **content hash**, which is what survives a rebuild: numbers get re-flowed
/// and folders get reorganized, and the bytes of a recording do not change. Only the fields that
/// package recorded as `edited` are carried, so a value that was merely detected last time is
/// detected again this time rather than frozen into the file.
fn seed_from_package(spec: &mut km_pack::Spec, dir: &Path, package: &Path) -> Result<()> {
    let existing = Package::open(package)
        .with_context(|| format!("opening {} to seed from", package.display()))?;
    let edits: BTreeMap<String, SongEntry> = existing
        .manifest()
        .songs
        .iter()
        .filter(|song| !song.edited.is_empty())
        .filter_map(|song| song.content_hash.clone().map(|hash| (hash, song.clone())))
        .collect();
    eprintln!(
        "seeding from {} hand-edited song(s) in {}",
        edits.len(),
        package.display()
    );

    let mut carried = 0;
    for song in &mut spec.songs {
        let Ok(bytes) = std::fs::read(dir.join(&song.file)) else {
            continue;
        };
        let Some(prior) = edits.get(&content_hash(&bytes)) else {
            continue;
        };
        if prior.is_edited(EditedField::Title) {
            song.title = Some(prior.title.clone());
        }
        if prior.is_edited(EditedField::Artist) {
            song.artist = prior.artist.clone();
        }
        if prior.is_edited(EditedField::Language) {
            song.language = prior.language.clone();
        }
        if prior.is_edited(EditedField::LyricEncoding) {
            song.encoding = prior.lyric_encoding.clone();
        }
        if prior.is_edited(EditedField::DefaultTranspose) {
            song.transpose = Some(prior.default_transpose);
        }
        if prior.is_edited(EditedField::LyricsHidden) {
            song.lyrics_hidden = Some(prior.lyrics_hidden);
        }
        if prior.is_edited(EditedField::Fixes) {
            song.fixes = Some(prior.fixes.clone());
        }
        if prior.is_edited(EditedField::Number) {
            song.number = Some(prior.number);
        }
        carried += 1;
    }
    eprintln!("  carried into {carried} song(s)");
    Ok(())
}

/// A progress line for the folder walk, on a terminal only.
///
/// Rewriting itself with a carriage return is only meaningful on a terminal; piped into a log the
/// returns are noise around thousands of copies of one line. The same test [`build_progress`] makes.
fn walk_progress() -> impl FnMut(usize, usize) {
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let mut last = usize::MAX;
    move |done, total| {
        if !interactive || total == 0 {
            return;
        }
        // Every 200 files rather than every file: the work per file is a parse and an analysis, so
        // a print per file is a measurable fraction of the run on a fast disk.
        let step = done / 200;
        if step != last {
            last = step;
            eprint!("  reading {done} of {total}\r");
        }
    }
}

/// Reports a build as it happens, in the shape the command line has always used.
fn build_progress() -> impl FnMut(km_pack::BuildEvent<'_>) -> std::ops::ControlFlow<()> {
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let mut total = 0usize;
    // Both exist only to manage the re-encoding line, so both are gone with the feature that raises
    // it — otherwise a build without `video` carries two variables nothing ever reads.
    #[cfg(feature = "video")]
    let mut last_percent = u8::MAX;
    #[cfg(feature = "video")]
    let mut encoding = false;

    move |event| {
        match event {
            km_pack::BuildEvent::Starting { songs } => {
                total = songs;
                eprintln!("\nbuilding {songs} song(s)");
            }
            km_pack::BuildEvent::Song { index, .. } => {
                if interactive && (index % 25 == 0 || index + 1 == total) {
                    eprint!("  packaging {} of {total}\r", index + 1);
                }
            }
            #[cfg(feature = "video")]
            km_pack::BuildEvent::Encoding { progress, .. } => {
                // Printed only when the number changes, so a five-minute encode is a line that
                // grows rather than thousands of them.
                let percent = progress.percent();
                if interactive && percent != last_percent && percent % 10 == 0 {
                    last_percent = percent;
                    encoding = true;
                    eprint!("    re-encoding {percent}%\r");
                }
            }
            km_pack::BuildEvent::Added { number, note } => {
                #[cfg(feature = "video")]
                if encoding {
                    // Wipes whatever the progress line left behind, so the result starts at a
                    // column rather than after `re-encoding 100%`.
                    eprint!("\r{:width$}\r", "", width = 24);
                    encoding = false;
                    last_percent = u8::MAX;
                }
                if let Some(note) = note {
                    eprintln!("  {number}  {note}");
                }
            }
            km_pack::BuildEvent::Skipped { source, why } => {
                eprintln!("  skipped {}: {why}", source.display());
            }
            km_pack::BuildEvent::Writing { out } => {
                // Said out loud because `PackageBuilder::write` has no progress of its own, so a
                // large package is a silent minute at the end of a run that has been talking all
                // along. Without this it looks like a hang at 100%.
                eprintln!("\nwriting {}...", out.display());
            }
        }
        std::ops::ControlFlow::Continue(())
    }
}

/// Reports what the folder walk decided not to describe.
///
/// Grouped by reason, because a per-file list over a real corpus is thousands of lines. Counts are
/// what a packager acts on; examples are what they investigate.
fn report_description(
    rejected: &[(PathBuf, Rejection)],
    orphans: &[(PathBuf, km_pack::CdgOrphan)],
) {
    // Half a pair is not a song, but a folder that quietly loses files is a folder nobody can
    // reconcile against what they put in it.
    for (path, why) in orphans {
        eprintln!("  skipped {}: {}", path.display(), why.describe());
    }

    if rejected.is_empty() {
        return;
    }
    println!("rejected   {}", rejected.len());

    let mut by_reason: BTreeMap<String, (usize, Vec<&PathBuf>)> = BTreeMap::new();
    for (path, reason) in rejected {
        let key = match reason {
            Rejection::LowSuitability(_) => "rated below the minimum".to_owned(),
            Rejection::DuplicateOf(_) => "duplicate of another file".to_owned(),
            other => other.to_string(),
        };
        let slot = by_reason.entry(key).or_insert((0, Vec::new()));
        slot.0 += 1;
        if slot.1.len() < 3 {
            slot.1.push(path);
        }
    }
    for (reason, (count, examples)) in by_reason {
        println!("  {count:>6}  {reason}");
        for example in examples {
            println!("          e.g. {}", example.display());
        }
    }
}

/// Reports what went into the package.
fn report_outcome(outcome: &km_pack::BuildOutcome) {
    let Some(manifest) = &outcome.manifest else {
        return;
    };
    println!("accepted   {}", outcome.written);

    // Both figures below are over MIDI songs only, and the denominator is the point. Neither a video
    // nor an MP3+G song has an automatic suitability or a melody channel — not a zero of either — so
    // counting them below the line would drag the average down and make detection look worse the
    // more of them a package holds.
    let midi = manifest
        .songs
        .iter()
        .filter(|song| song.kind.is_midi())
        .count();
    let (videos, cdg, ultrastar) = media_counts(&manifest.songs);
    if videos > 0 || cdg > 0 || ultrastar > 0 {
        println!("of which   {midi} midi, {videos} video, {cdg} mp3+g, {ultrastar} ultrastar");
    }
    if outcome.videos_transcoded > 0 {
        println!(
            "re-encoded {} of {videos} video(s)",
            outcome.videos_transcoded
        );
    }

    // The mean is over **every** song, because every song has a real suitability: a video or an
    // MP3+G pair is a flat 10 by what it is.
    let values: Vec<u8> = manifest
        .songs
        .iter()
        .filter_map(|song| song.suitability.as_ref().map(|record| record.value))
        .collect();
    if !values.is_empty() {
        let total: u32 = values.iter().map(|value| u32::from(*value)).sum();
        println!(
            "mean suitability {:.2}/10",
            f64::from(total) / values.len() as f64
        );
    }
    if midi > 0 {
        let with_melody = manifest
            .songs
            .iter()
            .filter(|song| song.melody.is_some())
            .count();
        println!(
            "melody     {with_melody} of {midi} ({:.1}%)",
            with_melody as f64 * 100.0 / midi as f64
        );
    }

    if !outcome.skipped.is_empty() {
        println!("skipped    {}", outcome.skipped.len());
        for skipped in outcome.skipped.iter().take(10) {
            println!("          {}: {}", skipped.source.display(), skipped.why);
        }
        if outcome.skipped.len() > 10 {
            println!("          ...and {} more", outcome.skipped.len() - 10);
        }
    }
}

/// What to say about songs that named no language, which is a list of songs to go and classify.
///
/// **The remedies have to be ones `build` can actually take.** This message named three that it
/// could not: "the curation tool", which is `km-package-builder` and has been since M17; the
/// `--index` CSV, which `build` stopped accepting when M19 made it take a description and nothing
/// else (it lives on `apply` now); and `--default-language`, which is a flag on `spec`. So the one
/// command that prints this refused the work and then offered three ways out, none of which applied
/// to it. The description is the thing to point at: it is already open in front of whoever is
/// reading this, and both fixes are edits to it.
fn unlanguaged_message(outcome: &km_pack::BuildOutcome) -> String {
    let listed = outcome
        .unlanguaged
        .iter()
        .take(10)
        .map(|(number, title)| format!("  {number}  {title}"))
        .collect::<Vec<_>>()
        .join("\n");
    let more = match outcome.unlanguaged.len().saturating_sub(10) {
        0 => String::new(),
        rest => format!("\n  ...and {rest} more"),
    };
    format!(
        "{} song(s) have no language, and a package cannot ship without one:\n{listed}{more}\n\n\
         Edit the description: give each song a `language`, or set `default_language` in its \
         `package` block to file the rest under one code -- `und` is the standard's own \
         \"undetermined\" and is the honest answer. Describing the folder again with \
         `km-pack spec --default-language und` writes that key for you, and km-package-builder can \
         set them song by song.",
        outcome.unlanguaged.len()
    )
}

fn inspect(args: &InspectArgs) -> Result<()> {
    let package = Package::open(&args.package)
        .with_context(|| format!("opening {}", args.package.display()))?;
    let manifest = package.manifest();

    if args.json {
        println!("{}", serde_json::to_string_pretty(manifest)?);
        return Ok(());
    }

    println!("{}", args.package.display());
    println!("  id         {}", manifest.package.id);
    println!("  name       {}", manifest.package.name);
    println!("  version    {}", manifest.package.version);
    println!("  songs      {}", manifest.songs.len());

    let values: Vec<u8> = manifest
        .songs
        .iter()
        .filter_map(|song| song.suitability.as_ref().map(|s| s.value))
        .collect();
    if !values.is_empty() {
        let total: u32 = values.iter().map(|s| u32::from(*s)).sum();
        println!(
            "  mean suitability {:.2}/10",
            f64::from(total) / values.len() as f64
        );
    }
    // Over MIDI songs only: neither a video nor an MP3+G song has a melody channel to detect, so
    // counting them in the denominator would make detection look worse the more of them the package
    // holds.
    let midi = manifest
        .songs
        .iter()
        .filter(|song| song.kind.is_midi())
        .count();
    let (videos, cdg, ultrastar) = media_counts(&manifest.songs);
    if videos > 0 || cdg > 0 || ultrastar > 0 {
        println!("  of which   {midi} midi, {videos} video, {cdg} mp3+g, {ultrastar} ultrastar");
    }
    let with_melody = manifest
        .songs
        .iter()
        .filter(|song| song.melody.is_some())
        .count();
    println!("  melody     {with_melody} of {midi}");

    // Over the media songs only, and the denominator is the point: a MIDI song is the reference the
    // others are levelled to and carries no measurement, so counting one would make a package of
    // MIDI songs look unmeasured when it is complete.
    let media = manifest.songs.len() - midi;
    if media > 0 {
        let levels: Vec<f32> = manifest
            .songs
            .iter()
            .filter_map(|song| song.loudness.as_ref().map(|record| record.lufs))
            .collect();
        if levels.is_empty() {
            println!(
                "  loudness   0 of {media} measured -- these will play unlevelled; \
                 `km-pack reanalyze` measures them without rebuilding"
            );
        } else {
            let total: f32 = levels.iter().sum();
            let mean = total / levels.len() as f32;
            let quietest = levels.iter().copied().fold(f32::MAX, f32::min);
            let loudest = levels.iter().copied().fold(f32::MIN, f32::max);
            println!(
                "  loudness   {} of {media} measured, mean {mean:.1} LUFS, \
                 spread {:.1} LU ({quietest:.1} to {loudest:.1})",
                levels.len(),
                loudest - quietest
            );
        }
    }

    if args.songs {
        println!();
        for song in &manifest.songs {
            let suitability = song
                .suitability
                .as_ref()
                .map_or_else(|| "  -".to_owned(), |s| format!("{:>3}", s.value));
            let melody = song
                .melody
                .as_ref()
                .map_or_else(|| " -".to_owned(), |m| format!("{:>2}", m.channel));
            // Padded to the width of the longest thing that can appear here, so the titles line up
            // whether or not a song has a language.
            let language = song.language.as_deref().unwrap_or("-");
            // The measurement and what it will roughly cost the song, in one column. The gain is
            // the useful half — a LUFS figure alone leaves the reader doing logarithms in their
            // head — and a blank is a MIDI song, which is the reference rather than a gap.
            //
            // **Against the default reference, not against a bank.** A packaging tool has no
            // business knowing which SoundFont some machine will play this with, and the machine
            // works the real gain out from the bank that is sounding. The default is within a tenth
            // of a decibel of the bundled bank, so what is printed here is what all but a
            // reconfigured machine will apply.
            let level = song.loudness.as_ref().map_or_else(
                || "           ".to_owned(),
                |record| {
                    format!(
                        "{:>6.1}Lu x{:.2}",
                        record.lufs,
                        km_loudness::gain_for(km_loudness::DEFAULT_REFERENCE_LUFS, record.lufs)
                    )
                },
            );
            println!(
                "  {:>8}  {suitability}/10  ch{melody}  {language:>3}  {level}  {}{}",
                song.number,
                song.title,
                song.artist
                    .as_deref()
                    .map(|a| format!("  --  {a}"))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn check(args: &CheckArgs) -> Result<()> {
    // Read unchecked first, so a package that will not open still gets diagnosed rather than just
    // refused. Telling somebody "invalid" without saying why is useless.
    let manifest = read_manifest_unchecked(&args.package)
        .with_context(|| format!("reading {}", args.package.display()))?;

    println!("{}", args.package.display());
    println!("  songs {}", manifest.songs.len());

    let problems = manifest.problems();
    if problems.is_empty() {
        println!("  manifest ok");
    } else {
        println!("  {} manifest problem(s):", problems.len());
        for problem in &problems {
            println!("    ! {problem}");
        }
    }

    // Only worth checking the archive contents if the manifest itself made sense.
    let mut missing = Vec::new();
    let mut compressed = Vec::new();
    if problems.is_empty() {
        let package = Package::open(&args.package)?;
        missing = package.missing_entries()?;
        if missing.is_empty() {
            println!("  every song file is present");
        } else {
            println!(
                "  {} song(s) name a missing file: {missing:?}",
                missing.len()
            );
        }

        // The line that makes "an entry a decoder seeks into is stored" checkable rather than merely
        // intended. Such an entry compressed has no byte range to seek into, so the machine would
        // refuse it at play time; better to say so to whoever still has the sources. Costs one walk
        // of the central directory and no reads — the sizes and methods are already there.
        //
        // The `.cdg` is deliberately not in that set — it is read whole and is deflated on purpose —
        // so it is reported with what it saves rather than flagged. `is_seekable_entry` is the one
        // place that distinction is made, shared with the writer so the two cannot drift.
        let media = package.media_entries()?;
        if !media.is_empty() {
            let bytes: u64 = media.iter().map(|entry| entry.size).sum();
            compressed = media
                .iter()
                .filter(|entry| is_seekable_entry(&entry.name) && !entry.stored)
                .map(|entry| entry.name.clone())
                .collect();
            let deflated: Vec<&MediaEntry> = media
                .iter()
                .filter(|entry| !is_seekable_entry(&entry.name) && !entry.stored)
                .collect();
            println!(
                "  media  {} entries, {}{}",
                media.len(),
                human(bytes),
                if compressed.is_empty() {
                    ", all seekable entries stored uncompressed"
                } else {
                    ""
                }
            );
            if !deflated.is_empty() {
                let raw: u64 = deflated.iter().map(|entry| entry.size).sum();
                println!(
                    "         {} of those deflated ({} read whole, not seeked into)",
                    deflated.len(),
                    human(raw)
                );
            }
            for name in &compressed {
                println!(
                    "    ! {name} is compressed; an entry the decoder seeks into must be stored \
                     uncompressed so it can be played by seeking into it"
                );
            }
        }
    }

    // Outside the `problems.is_empty()` block above, deliberately: the package most worth asking
    // this about is one whose manifest has just been refused, and everything in that block goes
    // through `Package::open`, which would never reach it.
    if args.verify {
        let damaged = km_kmpkg::damaged_entries(&args.package)?;
        if damaged.is_empty() {
            println!("  every entry matches the checksum the package records for it");
        } else {
            let (noun, verb) = if damaged.len() == 1 {
                ("entry", "does")
            } else {
                ("entries", "do")
            };
            println!(
                "  {} {noun} {verb} not match the checksum the package records",
                damaged.len()
            );
            for name in damaged.iter().take(5) {
                println!("    ! {name}");
            }
            if damaged.len() > 5 {
                println!("    ... and {} more", damaged.len() - 5);
            }
            println!("    the package was damaged after it was built; fetch or build it again");
        }
    }

    let mut low = Vec::new();
    let mut warning_counts: BTreeMap<String, usize> = BTreeMap::new();
    for song in &manifest.songs {
        if let Some(suitability) = &song.suitability {
            if suitability.value < args.min_suitability {
                low.push((song.number, suitability.value, song.title.clone()));
            }
            for warning in &suitability.warnings {
                *warning_counts.entry(warning.code.clone()).or_default() += 1;
            }
        }
    }

    if !warning_counts.is_empty() {
        println!("\n  warnings across the package:");
        let mut rows: Vec<_> = warning_counts.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1));
        for (code, count) in rows {
            println!("    {count:>6}  {code}");
        }
    }

    if low.is_empty() {
        println!(
            "\n  no song's suitability is below {}",
            args.min_suitability
        );
    } else {
        println!("\n  {} song(s) below {}:", low.len(), args.min_suitability);
        for (number, suitability, title) in low.iter().take(20) {
            println!("    {number:>8}  {suitability}/10  {title}");
        }
        if low.len() > 20 {
            println!("    ... and {} more", low.len() - 20);
        }
    }

    // Reported, never fatal outside `--strict`. This runs against packages built long before
    // language was a code, and those are perfectly good packages -- they play, they search, they
    // install. What they cannot do is answer "what Portuguese have we got?", and saying so is the
    // useful thing. Listing the distinct raw values with counts is what makes it actionable: it is
    // exactly the input `km-pack apply --index` needs.
    let mut unlanguaged = 0usize;
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    for song in &manifest.songs {
        match song.language.as_deref() {
            None => unlanguaged += 1,
            Some(raw) if Language::parse(raw).is_none() => {
                *unknown.entry(raw.to_owned()).or_default() += 1;
            }
            Some(_) => {}
        }
    }
    if unlanguaged > 0 || !unknown.is_empty() {
        println!("\n  language:");
        if unlanguaged > 0 {
            println!("    {unlanguaged:>6}  no language at all");
        }
        if !unknown.is_empty() {
            let mut rows: Vec<_> = unknown.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let total: usize = unknown.values().sum();
            let listed = rows
                .iter()
                .take(6)
                .map(|(raw, count)| format!("{raw} ({count})"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("    {total:>6}  not an ISO 639-1 code: {listed}");
        }
    } else if !manifest.songs.is_empty() {
        println!("\n  every song names a language");
    }

    // Reported the way the language block is, and **never a failure**: a MIDI song with no words is
    // a real thing (it still plays and still scores), and a video or MP3+G song has none by what it
    // is. What this answers is "did the build actually record any?", which is the question after
    // rebuilding a package made before previews existed. The kinds are split out because a count
    // mixing them would look alarming and mean nothing.
    if !manifest.songs.is_empty() {
        let (mut wordless_midi, mut by_kind) = (0usize, 0usize);
        for song in &manifest.songs {
            if !song.lyric_preview.is_empty() {
                continue;
            }
            if song.kind.draws_words() {
                wordless_midi += 1;
            } else {
                by_kind += 1;
            }
        }
        let with = manifest.songs.len() - wordless_midi - by_kind;
        println!("\n  lyric preview:");
        println!("    {with:>6}  carry the first line(s) of the song");
        if wordless_midi > 0 {
            println!("    {wordless_midi:>6}  MIDI or UltraStar song(s) with no words to show");
        }
        if by_kind > 0 {
            println!("    {by_kind:>6}  video or MP3+G song(s), whose words are pixels");
        }
    }

    let unlanguaged_fails = args.strict && (unlanguaged > 0 || !unknown.is_empty());
    // A compressed media entry fails whatever `--strict` says, and belongs with `missing` rather
    // than with the suitability warnings: the machine cannot play it either way, so it is a broken package
    // and not a scruffy one.
    let failed = !problems.is_empty()
        || !missing.is_empty()
        || !compressed.is_empty()
        || (args.strict && !low.is_empty())
        || unlanguaged_fails;
    if failed {
        bail!("the package did not pass");
    }
    Ok(())
}

/// A byte count in the largest unit that leaves a number a person can read.
///
/// Here because a package is measured in gigabytes rather than in manifest entries, and
/// `1204 entries, 18300000000` is a number nobody reads.
fn human(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GB", 1_000_000_000), ("MB", 1_000_000), ("kB", 1_000)];
    for (unit, scale) in UNITS {
        if bytes >= scale {
            return format!("{:.1} {unit}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes} B")
}

/// How many video, MP3+G and UltraStar songs a package holds, each counted by its own kind.
fn media_counts(songs: &[km_kmpkg::SongEntry]) -> (usize, usize, usize) {
    let count =
        |wanted: km_kmpkg::SongKind| songs.iter().filter(|song| song.kind == wanted).count();
    (
        count(km_kmpkg::SongKind::Video),
        count(km_kmpkg::SongKind::Cdg),
        count(km_kmpkg::SongKind::UltraStar),
    )
}

/// Measures one media song's level from inside the package it is already in.
///
/// **The bytes are read through a window into the archive**, exactly as playback reads them, so a
/// package re-analyzed here and one rebuilt from source measure the same thing — which is the
/// property that makes this a shortcut rather than a second answer. `km-cdg` and `km-video` each
/// hold a test that their reader and path forms agree.
///
/// `None` on any failure, and it is reported rather than fatal for the reason the build path gives:
/// a level is worth having and is not worth refusing a package over. A video in a build with no
/// `video` feature lands here too and comes back `None`, which is why it says so.
fn measure_in_package(
    package: &Package,
    song: &km_kmpkg::SongEntry,
) -> Option<km_kmpkg::LoudnessRecord> {
    let name = song.file.clone();
    let reader = match package.media_reader(song.number) {
        Ok(reader) => reader,
        Err(error) => {
            eprintln!(
                "  {:>8}  could not be opened to measure: {error}",
                song.number
            );
            return None;
        }
    };

    let measured = if song.kind.is_cdg() || song.kind.is_ultrastar() {
        km_cdg::measure_loudness_from(reader, &name).map_err(|error| error.to_string())
    } else {
        measure_packaged_video(reader, &name)
    };

    match measured {
        Ok(Some(loudness)) => Some(km_kmpkg::LoudnessRecord {
            lufs: loudness.lufs,
            peak_dbtp: loudness.peak_dbtp,
        }),
        Ok(None) => None,
        Err(error) => {
            eprintln!("  {:>8}  could not be measured: {error}", song.number);
            None
        }
    }
}

/// A packaged video's level, where this build can decode one.
#[cfg(feature = "video")]
fn measure_packaged_video(
    reader: km_kmpkg::EntryWindow,
    name: &str,
) -> std::result::Result<Option<km_loudness::Loudness>, String> {
    km_video::measure_loudness_from(reader, name).map_err(|error| error.to_string())
}

/// ...and the refusal where it cannot, which names the feature so it is actionable.
///
/// The same shape `km_pack::build`'s own `add_video` stub takes: the rest of the package is still
/// written, and the one song that could not be measured says why rather than silently keeping
/// whatever it had.
#[cfg(not(feature = "video"))]
fn measure_packaged_video(
    _reader: km_kmpkg::EntryWindow,
    _name: &str,
) -> std::result::Result<Option<km_loudness::Loudness>, String> {
    Err("this build has no `video` feature, so it cannot measure a video".to_owned())
}

/// How long a media song in a package is sung for.
///
/// An UltraStar song carries its timeline, so the span is read from it. A video's words are pixels
/// and an MP3+G pair's are one-bit tiles, so those two are answered by their own length, and so is
/// an UltraStar song whose timeline will not open, which is the honest fallback rather than a
/// refusal: a package that opens far enough to be re-analyzed should be re-analyzed.
fn media_sung_ms(package: &Package, song: &km_kmpkg::SongEntry) -> u32 {
    if song.kind.is_ultrastar()
        && let Ok(timeline) = package.lyric_timeline(song.number)
    {
        return km_suitability::sung_span_ms(&km_song::ultrastar::song_from_timeline(timeline));
    }
    song.duration_ms
}

fn reanalyze(args: &ReanalyzeArgs) -> Result<()> {
    let package = Package::open(&args.package)
        .with_context(|| format!("opening {}", args.package.display()))?;
    let manifest = package.manifest().clone();

    let mut builder = PackageBuilder::new(manifest.package.clone());
    let mut changed = 0usize;

    for song in &manifest.songs {
        // Three of the four components cannot be re-analyzed for a media song: separate channels,
        // lyrics and how well they sync are MIDI facts, and a video or an MP3+G pair has none of
        // them. **How much of it is sung can be, and so can how loud it is**, both from what is
        // already in the package, so this is the one path that corrects either without a rebuild.
        //
        // Carried across with `add_media_copied` either way, so `reanalyze` still writes a complete
        // package and the bytes are copied rather than re-encoded.
        if !song.kind.is_midi() {
            let mut entry = song.clone();
            let sung_ms = media_sung_ms(&package, song);
            let before = entry.suitability.as_ref().map(|record| record.value);
            let record = km_pack::purpose_made_suitability(sung_ms);
            let after = record.value;
            if before != Some(after) {
                changed += 1;
                println!(
                    "  {:>8}  {} -> {after}/10  {}",
                    song.number,
                    before.map_or_else(|| "-".to_owned(), |value| value.to_string()),
                    song.title
                );
            }
            entry.suitability = Some(record);
            if !args.no_loudness {
                let before = entry.loudness.map(|record| record.lufs);
                entry.loudness = measure_in_package(&package, song).or(entry.loudness);
                let after = entry.loudness.map(|record| record.lufs);
                // Reported on the same line shape as a suitability change, and only when it moved:
                // re-analyzing a package that already carries its levels should be quiet.
                if before.map(f32::to_bits) != after.map(f32::to_bits) {
                    changed += 1;
                    println!(
                        "  {:>8}  {} -> {}  {}",
                        song.number,
                        before.map_or_else(|| "-".to_owned(), |lufs| format!("{lufs:.1} LUFS")),
                        after.map_or_else(|| "-".to_owned(), |lufs| format!("{lufs:.1} LUFS")),
                        song.title
                    );
                }
            }
            builder.add_media_copied(entry, &package)?;
            continue;
        }
        let bytes = package.read_song(song.number)?;
        let options = ParseOptions {
            declared_encoding: song.lyric_encoding.clone(),
            inference: None,
        };
        let mut entry = song.clone();

        match Song::parse(&bytes, &options) {
            Ok(parsed) => {
                let analysis = Analysis::of(&parsed);
                let before = song.suitability.as_ref().map(|s| s.value);
                let after = analysis.suitability.value;
                if before != Some(after) {
                    changed += 1;
                    println!(
                        "  {:>8}  {} -> {after}/10  {}",
                        song.number,
                        before.map_or_else(|| "-".to_owned(), |s| s.to_string()),
                        song.title
                    );
                }
                entry.melody = melody_record(&analysis);
                // Rewritten alongside the melody, not left as it was. A song that abstained last
                // time and has a channel now would otherwise keep a stale reason next to it,
                // claiming both that a melody was found and why none was.
                entry.melody_abstained = km_pack::melody_abstained(&analysis);
                entry.suitability = Some(suitability_record(&analysis));
            }
            // A song already in a package that no longer parses is worth reporting loudly, but not
            // worth dropping: keeping it preserves the number a singer already knows.
            Err(error) => eprintln!("  {:>8}  could not re-parse: {error}", song.number),
        }
        entry.content_hash = None;
        builder.add(entry, bytes)?;
    }

    if args.dry_run {
        println!(
            "
dry run: nothing written; {changed} value(s) would change across {} song(s)",
            manifest.songs.len()
        );
        return Ok(());
    }

    // `required_unless_present` has already refused the run that reaches here without one, so the
    // `expect` is clap's guarantee restated rather than a check of our own.
    let out = args
        .out
        .as_deref()
        .expect("--out is required without --dry-run");
    let written = builder.write(out)?;
    println!(
        "
re-analyzed {} song(s), {changed} value(s) changed; wrote {}",
        written.songs.len(),
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two lines `tools/dist/cmd.sh` prints beside a staged km-pack, and the two the staged
    /// README opens with.
    ///
    /// It exists because they were wrong. `build` took a folder until M19 made a package a
    /// description you can read, and the report went on printing
    /// `km-pack build <folder> --out vol1.kmpkg` for as long as nothing compared the two. The
    /// README beside it had been rewritten; only the line nobody tests had not.
    ///
    /// **Note what this can and cannot catch.** The old line still *parses* — `<SPEC>` is a path
    /// and `--out` is a real option — so it failed at run time, with `reading <folder>: Is a
    /// directory`, on the first command a person typed out of a fresh release. A parse test would
    /// not have caught it and does not catch it now; what it pins is that the two lines the report
    /// prints stay lines this binary accepts, which is the part that silently rotted.
    #[test]
    fn the_commands_the_report_and_the_readme_tell_people_to_run_parse() {
        Cli::try_parse_from([
            "km-pack",
            "spec",
            "/songs/Brasil",
            "--out",
            "vol1.kmspec.yaml",
        ])
        .expect("the first line a staged km-pack tells you to type must parse");

        Cli::try_parse_from(["km-pack", "build", "vol1.kmspec.yaml"])
            .expect("the second line must parse");
    }

    /// A scanned folder names the package for a person, and never identifies it.
    ///
    /// **The folder's name is the leak.** A corpus is filed by client, by party and by the person
    /// whose collection it was, and `package.id` is what the machine builds the installed file's
    /// name out of. The name seeds from the folder, because that is a label somebody reads in the
    /// written description and changes before they build; the id is the half that travels whatever
    /// they do.
    #[test]
    fn a_scanned_folder_does_not_name_the_package() {
        let dir = std::env::temp_dir().join(format!(
            "km-pack-spec-id-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let songs = dir.join("Festa-Ana-2024");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&songs).expect("a scratch corpus");
        std::fs::write(songs.join("a.kar"), km_song::testing::soft_karaoke()).expect("a song");

        let out = dir.join("written.kmspec.yaml");
        let Command::Spec(args) = Cli::try_parse_from([
            "km-pack",
            "spec",
            &songs.display().to_string(),
            "--out",
            &out.display().to_string(),
        ])
        .expect("parse")
        .command
        else {
            panic!("that is the spec command");
        };
        describe(&args).expect("describe the folder");

        let written = std::fs::read_to_string(&out).expect("the description");
        let spec = km_pack::Spec::read(&out).expect("parse the description");
        assert!(
            km_kmpkg::PackageMeta::is_generated_id(&spec.package.id),
            "the id must be generated, got {:?}",
            spec.package.id
        );
        assert_eq!(
            spec.package.name, "Festa-Ana-2024",
            "the name is the half a person reads and edits"
        );
        // Not merely different from the folder: nothing built from this description may carry it.
        assert!(!written.contains("Festa-Ana-2024\n  id"), "got {written}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An id somebody types is refused where they typed it.
    ///
    /// `Manifest::problems` would refuse it too, but only after a scan of a corpus and a build — so
    /// the answer would arrive an hour after the mistake.
    #[test]
    fn a_typed_id_is_refused_before_the_folder_is_scanned() {
        let Command::Spec(args) =
            Cli::try_parse_from(["km-pack", "spec", "/songs/Brasil", "--id", "festa-ana-2024"])
                .expect("parse")
                .command
        else {
            panic!("that is the spec command");
        };
        let refused = describe(&args).expect_err("a typed id is not an id");
        let said = refused.to_string();
        assert!(said.contains("festa-ana-2024"), "got {said}");
    }
}
