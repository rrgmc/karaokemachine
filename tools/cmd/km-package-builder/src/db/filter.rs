//! What the browse bar can narrow the corpus by, and how each control becomes SQL.
//!
//! `Sort`, `ScoreFilter`, `SuitabilityFilter`, `CopiesFilter`, `AddedFilter`, `LanguageFilter`,
//! `Initial`, and the `Filter` that carries them all — plus `fts_match_query`, which turns what somebody typed into an
//! FTS5 expression that cannot mean anything but literal words.
//!
//! **One vocabulary, and it is the page's as much as the database's.** Each of these is parsed from
//! a query parameter, rendered back into a chip, and turned into a `WHERE` clause, so they are the
//! one place those three descriptions of the same idea are kept in step. That is why they belong
//! together in a file and not scattered through the queries that consume them.
//!
//! Everything is re-exported from `crate::db`, so no path outside this module changed.

use super::*;

/// How the browse table is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Highest automatic suitability first.
    Suitability,
    /// Highest personal rating first, unrated last.
    UserScore,
    /// Alphabetical by effective title.
    ///
    /// **The default**, and it was suitability once. Two things made that the wrong first impression of a
    /// corpus: 86% of real files rate 8 or above, so it sorts *broken from working* far
    /// better than it sorts anything from anything else, and the top of the list was therefore an
    /// arbitrary slice of near-identical numbers. Alphabetical is the order somebody browsing a
    /// collection expects, and it is stable — the same folder opens on the same page tomorrow. Suitability
    /// is still one click away, and is the right order for hunting defects rather than for browsing.
    /// A video makes this sharper still, having no automatic suitability to be sorted by at all.
    #[default]
    Title,
    /// Alphabetical by effective performer.
    Artist,
    /// Longest first.
    Duration,
    /// Most copies on disk first.
    Copies,
    /// Alphabetical by language code, unclassified last.
    Language,
    /// Most recently edited first, never edited last.
    Updated,
    /// Most recently added to the corpus first.
    Added,
}

impl Sort {
    /// Reads the `sort` query parameter.
    pub fn parse(value: &str) -> Self {
        match value {
            "suitability" => Self::Suitability,
            "user_score" => Self::UserScore,
            "title" => Self::Title,
            "artist" => Self::Artist,
            "duration" => Self::Duration,
            "copies" => Self::Copies,
            "language" => Self::Language,
            "updated" => Self::Updated,
            "added" => Self::Added,
            // Including the empty string, which is what an unset query parameter is — so this, not
            // the `#[default]` above, is what actually decides the order a corpus first opens in.
            _ => Self::default(),
        }
    }

    /// The spelling used in URLs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Suitability => "suitability",
            Self::UserScore => "user_score",
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Duration => "duration",
            Self::Copies => "copies",
            Self::Language => "language",
            Self::Updated => "updated",
            Self::Added => "added",
        }
    }
}

/// What a hand-set score has to be for a song to show.
///
/// One control per score rather than the pair of them a "rated / unrated" select and a "≥ n" select
/// would need. The four cases are not independent — asking for `≥ 7` already means rated — so
/// offering them as one list is both smaller on the page and impossible to set to a contradiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScoreFilter {
    /// No constraint.
    #[default]
    Any,
    /// Only songs nobody has scored.
    Unset,
    /// Only songs somebody has scored, whatever they said.
    Set,
    /// Only songs scored at least this well. Implies set.
    AtLeast(u8),
}

impl ScoreFilter {
    /// Reads the query parameter: `""` · `unset` · `set` · a number.
    pub fn parse(value: &str) -> Self {
        match value {
            "unset" => Self::Unset,
            "set" => Self::Set,
            other => match other.parse::<u8>() {
                Ok(score) if score <= 10 => Self::AtLeast(score),
                _ => Self::Any,
            },
        }
    }

    /// The spelling used in URLs and in the `<option>` values, so a round trip keeps the control set.
    pub fn as_str(self) -> String {
        match self {
            Self::Any => String::new(),
            Self::Unset => "unset".to_owned(),
            Self::Set => "set".to_owned(),
            Self::AtLeast(score) => score.to_string(),
        }
    }

    /// The `WHERE` fragment for a column, or `None` when there is nothing to add.
    pub(super) fn clause(self, column: &str) -> Option<String> {
        match self {
            Self::Any => None,
            Self::Unset => Some(format!("{column} IS NULL")),
            Self::Set => Some(format!("{column} IS NOT NULL")),
            Self::AtLeast(score) => Some(format!("{column} >= {score}")),
        }
    }
}

/// Which bucket of the alphabet bar a song has to file under.
///
/// The same shape as [`ScoreFilter`] and [`LanguageFilter`] — `parse` / `as_str` / `describe` /
/// `clause` — and for the same reason: the bar's value has to round-trip through a query string, and
/// a value nothing recognizes has to fall back to *any* rather than to a predicate nothing matches.
///
/// **The bucketing in the data is finer than the bucketing on the bar, deliberately.**
/// [`title_initial`] still files each digit under itself, because its text is the key of the
/// `songs_browse_letter_artist` expression index and every index in [`Db::create_browse_indexes`] is
/// `IF NOT EXISTS` — so changing that expression would leave every existing database holding an index
/// the new query no longer matches, and the planner would silently stop using it. The collapse
/// happens here, in the predicate, where it costs nothing and no database has to be rebuilt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Initial {
    /// No constraint.
    #[default]
    Any,
    /// Titles filing under one letter, accents folded.
    Letter(char),
    /// Titles starting with any digit. One button and not ten: nobody browses a corpus for the songs
    /// beginning with 7.
    Digits,
    /// Everything that is neither a letter nor a digit — punctuation, and every script this fold does
    /// not reach.
    Symbol,
}

impl Initial {
    /// Reads the query parameter: `""` · `A`–`Z` · `0-9` · `#`.
    ///
    /// Anything else is [`Initial::Any`], by the rule [`LanguageFilter::parse`] and the `kind`
    /// parameter already follow: a hand-edited query string must not be able to produce an empty page
    /// with no explanation.
    pub fn parse(value: &str) -> Self {
        match value {
            "" => Self::Any,
            "0-9" => Self::Digits,
            "#" => Self::Symbol,
            other => {
                let mut chars = other.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_ascii_alphabetic() => {
                        Self::Letter(c.to_ascii_uppercase())
                    }
                    _ => Self::Any,
                }
            }
        }
    }

    /// The spelling used in URLs and in the radio values, so a round trip keeps the bar set.
    pub fn as_str(self) -> String {
        match self {
            Self::Any => String::new(),
            Self::Letter(letter) => letter.to_string(),
            Self::Digits => "0-9".to_owned(),
            Self::Symbol => "#".to_owned(),
        }
    }

    /// How the button reads. `#` is a glyph that says nothing about what it holds, so it is spelled.
    pub fn label(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Any => words.msg("initial-any").into_owned(),
            Self::Letter(letter) => letter.to_string(),
            // A range of digits reads the same in every language; *symbol* is a word and does not.
            Self::Digits => "0-9".to_owned(),
            Self::Symbol => words.msg("initial-symbol").into_owned(),
        }
    }

    /// How the chip in the filter bar reads. Prose, so a sentence rather than a glyph.
    ///
    /// **Worded here rather than handed back as a key**, unlike a scan status: a filter is described
    /// while a page is being drawn, so the language is in reach, and one of the four carries a
    /// letter.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Any => String::new(),
            Self::Letter(letter) => words
                .msg_with("chip-starts-with", &[("letter", letter.to_string().into())])
                .into_owned(),
            Self::Digits => words.msg("chip-starts-with-a-number").into_owned(),
            Self::Symbol => words.msg("chip-starts-with-a-symbol").into_owned(),
        }
    }

    /// The `WHERE` fragment for an expression, or `None` when there is nothing to add.
    ///
    /// Takes the expression [`title_initial`] builds rather than a column name, which is what lets the
    /// predicate match the `songs_browse_letter_artist` index tree for tree.
    ///
    /// **`IN` and not `GLOB '[0-9]'` for the digits**, and that is the whole reason the bar stays
    /// fast: an `IN` list against an indexed expression is ten index seeks, while a `GLOB` is not
    /// sargable at all — it would put the page back to scanning the corpus, with nothing failing to
    /// say so. A planner test asserts the seek, which is what would catch the swap.
    ///
    /// Every value here is produced by this code from a closed set and never typed by a person, so it
    /// goes into the fragment rather than costing a binding — the argument [`ScoreFilter::clause`]
    /// already makes.
    pub(super) fn clause(self, expression: &str) -> Option<String> {
        match self {
            Self::Any => None,
            Self::Letter(letter) => Some(format!("{expression} = '{letter}'")),
            Self::Digits => Some(format!(
                "{expression} IN ('0','1','2','3','4','5','6','7','8','9')"
            )),
            Self::Symbol => Some(format!("{expression} = '#'")),
        }
    }
}
/// Which band of the automatic 0–10 suitability score a browse query will accept.
///
/// **Three bands rather than the `≥ N` ladder this replaced**, and the difference is what the
/// control is for. A curator sorting a corpus is not asking "how good is good enough?" one number at
/// a time — they are asking one of three questions: *what can I package as it stands* (8–10), *what
/// is worth listening to before deciding* (5–7), and *what is broken and should be looked at or
/// thrown away* (under 5). A ladder answers the first two badly and cannot ask the third at all:
/// `≥ N` has no upper edge, so there was no way to see the low end without also seeing every
/// good song above it.
///
/// The bands therefore **partition** 0–10 rather than overlapping, which is what the retired ladder
/// did — `≥ 5` and `≥ 8` are nested, so two adjacent options showed mostly the same songs. Nothing
/// is unreachable: every suitability falls in exactly one band.
///
/// The one thing lost is asking for exactly `≥ 9`, and it is worth naming rather than glossing: sort
/// by suitability is what answers that now, and it answers it better, because it shows you where the
/// cliff actually is instead of making you guess a threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SuitabilityFilter {
    /// No constraint.
    #[default]
    Any,
    /// 8 to 10 — packageable as it stands.
    High,
    /// 5 to 7 — worth an audition first.
    Middle,
    /// Under 5 — something is wrong with the file.
    Low,
}

impl SuitabilityFilter {
    /// Reads the query parameter: `""` · `8-10` · `5-7` · `0-4`.
    ///
    /// Anything else is [`SuitabilityFilter::Any`], by the same rule the rest of the bar follows: a
    /// hand-edited query string must not be able to produce an empty page with no explanation.
    ///
    /// **The low band's value is `0-4` and its label is `<5`.** A `<` in a URL is legal but is
    /// escaped by everything that touches one, so `suitability=%3C5` would be what a copied link
    /// actually read. The two ends of the range say the same thing and survive a round trip through
    /// an address bar unchanged.
    pub fn parse(value: &str) -> Self {
        match value {
            "8-10" => Self::High,
            "5-7" => Self::Middle,
            "0-4" => Self::Low,
            _ => Self::Any,
        }
    }

    /// The spelling used in URLs and in the `<option>` values, so a round trip keeps the control set.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "",
            Self::High => "8-10",
            Self::Middle => "5-7",
            Self::Low => "0-4",
        }
    }

    /// How the option reads in the dropdown.
    ///
    /// Not the same string as [`Self::as_str`], which is the low band's whole point: the value is a
    /// range because a URL has to survive being copied, and the label is `<5` because that is how
    /// somebody reading a filter bar thinks about it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::High => "8-10",
            Self::Middle => "5-7",
            Self::Low => "<5",
        }
    }

    /// How the chip in the filter bar reads.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Any => String::new(),
            Self::High => words.msg("chip-suitability-high").into_owned(),
            Self::Middle => words.msg("chip-suitability-middle").into_owned(),
            Self::Low => words.msg("chip-suitability-low").into_owned(),
        }
    }

    /// The `WHERE` fragment for a column, or `None` when there is nothing to add.
    ///
    /// The bounds are constants this code owns rather than text from a person, so they go into the
    /// fragment rather than costing a binding — the same judgment [`CopiesFilter::clause`] makes.
    pub(super) fn clause(self, column: &str) -> Option<String> {
        match self {
            Self::Any => None,
            Self::High => Some(format!("{column} BETWEEN 8 AND 10")),
            Self::Middle => Some(format!("{column} BETWEEN 5 AND 7")),
            Self::Low => Some(format!("{column} < 5")),
        }
    }
}

/// How many copies of a song on disk a browse query will accept.
///
/// The same shape as [`ScoreFilter`], and the options overlap for the same reason that one offers
/// `set` beside `≥ 5`: the buckets answer different questions a curator actually asks. *1* is what a
/// folder looks like once a dedupe pass has finished with it; *2–10* is an ordinary duplicate; *more
/// than 10* is where to start when the pass has not run.
///
/// **The bar's three buckets partition `file_count`, and `AtLeastTwo` is deliberately not among
/// them.** It is the union of the two above it rather than a fourth bucket, so picking it and
/// picking either of them return overlapping lists and the dropdown has no shape a reader could
/// hold. It is a fourth arm nothing on the bar reaches, for the Duplicates page's *byte-identical
/// copies* panel, which is the one place *more than one copy* is the whole question.
///
/// Every arm reads `songs.file_count`, which is a real column maintained by triggers on `files` — not
/// the correlated `COUNT(*)` this filter's predecessor used. See the note on [`Sort::Copies`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopiesFilter {
    /// No constraint.
    #[default]
    Any,
    /// Exactly one file on disk.
    One,
    /// Two to ten inclusive.
    TwoToTen,
    /// More than ten.
    OverTen,
    /// Two or more, which no dropdown on the bar offers — see the note above.
    AtLeastTwo,
}

impl CopiesFilter {
    /// Reads the query parameter: `""` · `1` · `2-10` · `10+`.
    ///
    /// Anything else is [`CopiesFilter::Any`], by the same rule the rest of the bar follows — `2+`
    /// included, which no dropdown here can display. A link carrying one shows the whole corpus
    /// rather than a bucket nothing can select, which is the lesser of the two wrongs: the
    /// alternative is a select with nothing selected sitting above a narrowed list.
    pub fn parse(value: &str) -> Self {
        match value {
            "1" => Self::One,
            "2-10" => Self::TwoToTen,
            "10+" => Self::OverTen,
            // The one bucket the bar cannot show, and it parses because the Duplicates page links
            // to it: *more than one copy* is that page's whole question, and a link whose value
            // fell through to `Any` would quietly answer it with the entire corpus.
            "2+" => Self::AtLeastTwo,
            _ => Self::Any,
        }
    }

    /// The spelling used in URLs and in the `<option>` values, so a round trip keeps the control set.
    ///
    /// [`Self::Any`] carries the empty string, which is what the bar sends for *no constraint* and
    /// what keeps `parse(x.as_str()) == x` true of everything the bar can produce.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "",
            Self::One => "1",
            Self::TwoToTen => "2-10",
            Self::OverTen => "10+",
            Self::AtLeastTwo => "2+",
        }
    }

    /// How the chip in the filter bar reads.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Any => String::new(),
            Self::One => words.msg("chip-one-copy").into_owned(),
            Self::TwoToTen => words.msg("chip-two-to-ten-copies").into_owned(),
            Self::OverTen => words.msg("chip-over-ten-copies").into_owned(),
            Self::AtLeastTwo => words.msg("chip-more-than-one-copy").into_owned(),
        }
    }

    /// The `WHERE` fragment for a column, or `None` when there is nothing to add.
    pub(super) fn clause(self, column: &str) -> Option<String> {
        match self {
            Self::Any => None,
            Self::One => Some(format!("{column} = 1")),
            Self::TwoToTen => Some(format!("{column} BETWEEN 2 AND 10")),
            Self::OverTen => Some(format!("{column} > 10")),
            Self::AtLeastTwo => Some(format!("{column} > 1")),
        }
    }
}

/// How long ago a song has to have been added to the corpus.
///
/// Added means `songs.first_seen`: the scan that first found the song stamps it, and no later scan
/// moves it. The bounds are measured back from the moment the query runs, so a saved link keeps
/// meaning *the last seven days* rather than a fixed week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddedFilter {
    /// No constraint.
    #[default]
    Any,
    /// In the last 24 hours.
    Day,
    /// In the last 7 days.
    Week,
    /// In the last 30 days.
    Month,
    /// More than 30 days ago.
    OverMonth,
}

impl AddedFilter {
    /// Reads the query parameter: `""` · `1d` · `7d` · `30d` · `30d+`. Anything else is
    /// [`AddedFilter::Any`].
    pub fn parse(value: &str) -> Self {
        match value {
            "1d" => Self::Day,
            "7d" => Self::Week,
            "30d" => Self::Month,
            "30d+" => Self::OverMonth,
            _ => Self::Any,
        }
    }

    /// The spelling used in URLs and in the `<option>` values.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "",
            Self::Day => "1d",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::OverMonth => "30d+",
        }
    }

    /// How the chip in the filter bar reads.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Any => String::new(),
            Self::Day => words.msg("chip-added-day").into_owned(),
            Self::Week => words.msg("chip-added-week").into_owned(),
            Self::Month => words.msg("chip-added-month").into_owned(),
            Self::OverMonth => words.msg("chip-added-over-month").into_owned(),
        }
    }

    /// The `WHERE` fragment for a column, or `None` when there is nothing to add.
    ///
    /// The bound is SQLite's own clock in the stamp's fixed-width `YYYY-MM-DDTHH:MM:SSZ` shape, so a
    /// text comparison is a comparison in time.
    pub(super) fn clause(self, column: &str) -> Option<String> {
        let since = |modifier: &str| format!("strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '{modifier}')");
        match self {
            Self::Any => None,
            Self::Day => Some(format!("{column} >= {}", since("-1 day"))),
            Self::Week => Some(format!("{column} >= {}", since("-7 days"))),
            Self::Month => Some(format!("{column} >= {}", since("-30 days"))),
            Self::OverMonth => Some(format!("{column} < {}", since("-30 days"))),
        }
    }
}

/// Whether a browse list shows one row per recording or one per file.
///
/// **Collapsed is the default, and it is what stops duplicate curation.** On the corpus this was
/// measured against, 30.6% of everything favorited was a second copy of a song already in the same
/// list — 2,169 entries of 7,089. Nobody chose that twice; they were shown six rows of one song and
/// starred more than one. Hiding the five is the fix, and it has to be the default or it fixes
/// nothing.
///
/// **`All` is not a debugging switch.** Two versions of a recording are genuinely different files,
/// and choosing between them by ear is the one judgment here the machine cannot make. This is where
/// somebody goes to make it.
///
/// **A filter naming one favorite is never collapsed**, whatever this says: see [`Filter::to_sql`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VersionsFilter {
    /// One row per recording: a cluster shows its representative and hides the rest.
    #[default]
    Collapsed,
    /// One row per file, cluster or no cluster.
    All,
}

impl VersionsFilter {
    /// Reads the query parameter: `""` is collapsed, `all` is every version.
    ///
    /// The default carries the empty string rather than a word, so a browse URL says nothing at all
    /// until somebody asks for the unusual thing — the rule the rest of the bar follows.
    pub fn parse(value: &str) -> Self {
        match value {
            "all" => Self::All,
            _ => Self::Collapsed,
        }
    }

    /// The spelling used in URLs and in the option values, so a round trip keeps the control set.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collapsed => "",
            Self::All => "all",
        }
    }

    /// How the chip in the filter bar reads.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        match self {
            Self::Collapsed => String::new(),
            Self::All => crate::words::messages(locale)
                .msg("songs-every-version")
                .into_owned(),
        }
    }

    /// The `WHERE` fragment, or `None` when there is nothing to add.
    pub(super) fn clause(self) -> Option<String> {
        match self {
            Self::Collapsed => Some("s.duplicate_of IS NULL".to_owned()),
            Self::All => None,
        }
    }
}

/// Whether a browse query is about the corpus or about what has been thrown away.
///
/// **Two arms and no *both*, which is what separates this from every other control on the bar.**
/// The rest narrow a list of songs somebody is curating; this one chooses which of two lists is
/// being looked at. A deleted song carries no rating, no filing and no package that means anything
/// any more, so mixing the two would put rows into every count and every page that none of the
/// other controls can say anything useful about.
///
/// The default is the corpus, and it is the empty string, so a browse URL says nothing at all until
/// somebody asks for the unusual thing — the rule the rest of the bar follows.
///
/// Its clause is the one place [`Filter::to_sql`] does not take [`browsable`] whole: *only deleted*
/// has to invert the half of that predicate this type owns while keeping the other half, since a
/// song both merged and deleted is still a merge and has no row of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeletedFilter {
    /// The corpus: everything nobody has thrown away.
    #[default]
    Live,
    /// Only what has been thrown away.
    Only,
}

impl DeletedFilter {
    /// Reads the query parameter: `""` is the corpus, `only` is what was thrown away.
    ///
    /// Anything else reads as the corpus, by the rule every `parse` here follows: a hand-edited
    /// query string must not be able to produce a page nothing explains.
    pub fn parse(value: &str) -> Self {
        match value {
            "only" => Self::Only,
            _ => Self::Live,
        }
    }

    /// The spelling used in URLs, so a round trip keeps the control set.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "",
            Self::Only => "only",
        }
    }

    /// How the chip in the filter bar reads.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        match self {
            Self::Live => String::new(),
            Self::Only => crate::words::messages(locale)
                .msg("songs-only-deleted")
                .into_owned(),
        }
    }

    /// The `WHERE` fragment. `alias` is what the caller prefixes its columns with, and an index can
    /// have none — [`browsable`] asks [`Self::Live`] for its half through this.
    pub(super) fn clause(self, alias: &str) -> String {
        match self {
            Self::Live => format!("{alias}deleted_at IS NULL"),
            Self::Only => format!("{alias}deleted_at IS NOT NULL"),
        }
    }
}

/// What a browse query asks about a song's filing.
///
/// **Five arms rather than a checkbox, and the negative ones are what earn the control.**
/// A checkbox can only ask *which of these have I filed?*; the question a curation pass is actually
/// made of is the other one — *which of these have I not looked at yet?* — and on a corpus where
/// almost nothing is filed it is the arm that narrows hundreds of thousands of rows to the work
/// remaining. The two are not each other's absence: no filter at all shows both.
///
/// [`Self::Filed`] is the same question asked of a corpus part-way through a pass. A working list
/// holds songs somebody set aside to decide about later, so *in any favorite* counts the undecided
/// with the decided and cannot answer *what have I settled?*. The row already draws that line — a
/// star fills for any list and goes gold only for a filing — and this is that line as a filter. See
/// `A favorite can be a working list` in `docs/decisions/curation.md`.
///
/// [`Self::NotFiled`] is its other half: *what is left to settle?*. [`Self::NotIn`] cannot answer
/// it, because a song set aside in a working list is in a favorite and so drops out of the work that
/// remains.
///
/// A residual predicate over whichever sort index the query is already walking, like `kind` and
/// `granularity`, so [`super::Db::create_browse_indexes`] has nothing to add for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FavoritedFilter {
    /// No constraint.
    #[default]
    Any,
    /// Only songs in at least one favorite, whichever it is.
    In,
    /// Only songs in none.
    NotIn,
    /// Only songs in at least one favorite that is a filing rather than a working list.
    Filed,
    /// Only songs in no filing: in no favorite at all, or only in working lists.
    NotFiled,
}

impl FavoritedFilter {
    /// Reads the query parameter: `""` · `in` · `out` · `filed` · `unfiled`.
    ///
    /// `1` is read as [`Self::In`], which is the spelling a checkbox sent; anything else is
    /// [`Self::Any`], by the rule the rest of the bar follows. [`Self::as_str`] gives `1` no
    /// spelling, so a link carrying it normalizes on its first page turn — one filter, one spelling.
    /// See `No compatibility aliases` in `docs/decisions/songs.md`.
    pub fn parse(value: &str) -> Self {
        match value {
            "in" | "1" => Self::In,
            "out" => Self::NotIn,
            "filed" => Self::Filed,
            "unfiled" => Self::NotFiled,
            _ => Self::Any,
        }
    }

    /// The spelling used in URLs and in the `<option>` values, so a round trip keeps the control set.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "",
            Self::In => "in",
            Self::NotIn => "out",
            Self::Filed => "filed",
            Self::NotFiled => "unfiled",
        }
    }

    /// How the chip in the filter bar reads.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Any => String::new(),
            Self::In => words.msg("songs-in-any-favorite").into_owned(),
            Self::NotIn => words.msg("songs-in-no-favorite").into_owned(),
            Self::Filed => words.msg("songs-in-filed-favorite").into_owned(),
            Self::NotFiled => words.msg("songs-in-no-filed-favorite").into_owned(),
        }
    }

    /// The `WHERE` fragment, or `None` when there is nothing to add.
    ///
    /// `NOT IN` is safe over `song_favorites.song_id` because the column is `NOT NULL`; the same
    /// clause over a nullable one returns no rows at all and says nothing about why.
    ///
    /// [`Self::Filed`] and [`Self::NotFiled`] join where the other two do not, because *is this list
    /// a filing* is a fact about the favorite and not about the membership — the same join
    /// `permanent_count` in [`super::sql::browse_columns`] counts through, so the filter and the
    /// star's colour cannot come to disagree about what a filing is. The two are one subquery behind
    /// `IN` and `NOT IN`, so they cannot come to disagree with each other either.
    pub(super) fn clause(self) -> Option<&'static str> {
        macro_rules! filed_songs {
            () => {
                "(SELECT sf.song_id FROM song_favorites sf
                  JOIN favorites f ON f.id = sf.favorite_id WHERE f.temporary = 0)"
            };
        }
        match self {
            Self::Any => None,
            Self::In => Some("s.id IN (SELECT song_id FROM song_favorites)"),
            Self::NotIn => Some("s.id NOT IN (SELECT song_id FROM song_favorites)"),
            Self::Filed => Some(concat!("s.id IN ", filed_songs!())),
            Self::NotFiled => Some(concat!("s.id NOT IN ", filed_songs!())),
        }
    }
}

/// What a browse query asks about a song's language.
///
/// The same four-way shape as [`ScoreFilter`], and for the same reason: *nobody has said* is a
/// distinct and interesting answer, not the absence of a filter. On a real corpus it is also the
/// commonest — most files declare no language and are written in an encoding that implies none — so
/// `unset` is the filter a curator reaches for most, and it is what pairs with the bulk set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LanguageFilter {
    /// No constraint.
    #[default]
    Any,
    /// Only songs whose language nobody has said and nothing implied.
    Unset,
    /// Only songs that have one, whatever it is.
    Set,
    /// Only songs in this language.
    Is(Language),
}

impl LanguageFilter {
    /// Reads the query parameter: `""` · `unset` · `set` · a code.
    ///
    /// Anything else falls back to [`LanguageFilter::Any`] rather than matching nothing. A
    /// hand-edited query string must not be able to produce an empty page with no explanation —
    /// which is the rule the `kind` parameter already follows.
    pub fn parse(value: &str) -> Self {
        match value {
            "unset" => Self::Unset,
            "set" => Self::Set,
            other => Language::parse(other).map_or(Self::Any, Self::Is),
        }
    }

    /// The spelling used in URLs and `<option>` values, so a round trip keeps the control set.
    pub fn as_str(self) -> String {
        match self {
            Self::Any => String::new(),
            Self::Unset => "unset".to_owned(),
            Self::Set => "set".to_owned(),
            Self::Is(language) => language.code().to_owned(),
        }
    }

    /// How the chip in the filter bar reads. Prose, so the *name* rather than the code.
    ///
    /// **A language's own name stays English**, which `km_kmpkg::Language` documents: it carries an
    /// English name for the pickers that have to show it, and that is data about the corpus rather
    /// than a word this tool says.
    pub fn describe(self, locale: km_locale::Locale) -> String {
        let words = crate::words::messages(locale);
        match self {
            Self::Any => String::new(),
            Self::Unset => words.msg("chip-language-unset").into_owned(),
            Self::Set => words.msg("chip-language-any").into_owned(),
            Self::Is(language) => language.name().to_owned(),
        }
    }

    /// The `WHERE` fragment for an expression, or `None` when there is nothing to add.
    ///
    /// Takes the *expression* [`eff_language`] builds rather than a column name, which is what makes
    /// the filter agree with the column the list is showing.
    pub(super) fn clause(self, expression: &str) -> Option<String> {
        match self {
            Self::Any => None,
            Self::Unset => Some(format!("{expression} IS NULL")),
            Self::Set => Some(format!("{expression} IS NOT NULL")),
            // A `&'static str` this code produced from a closed table, never text from a person, so
            // it goes into the fragment rather than costing a binding -- the argument `to_sql`
            // already makes for the numeric bounds.
            Self::Is(language) => Some(format!("{expression} = '{}'", language.code())),
        }
    }
}

/// A browse query.
///
/// **`PartialEq` is here for [`Self::only_this_favorite`]**, which asks whether a filter narrows by
/// one favorite and by nothing else. Comparing against a built expectation is what makes that
/// question survive a new field: one added here defaults to *not narrowing*, so it joins the
/// comparison without anybody remembering to name it.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    /// Free text, matched against title and artist.
    pub query: Option<String>,
    /// Which band of the automatic suitability is acceptable.
    pub suitability: SuitabilityFilter,
    /// What the person's own rating has to be.
    pub user_score: ScoreFilter,
    /// Which bucket of the alphabet bar the effective title has to file under.
    pub initial: Initial,
    /// Only songs by exactly this artist, as typed or as clicked in a row.
    ///
    /// **Exact where [`Self::query`] is a substring search, and that is the whole reason it exists.**
    /// The `title or artist` box answers *which song is this?*; `Queen` there also brings back
    /// Queensrÿche, every Queen tribute and any song with the word in its title. This answers *what
    /// else did they do?*, which is a different question and the one a curator asks while filling a
    /// package.
    ///
    /// Matched on the folded key rather than on the artist itself — see [`Self::to_sql`].
    pub artist: Option<String>,
    /// Only songs with a file under this folder, given as a path prefix ending in `/`.
    pub folder: Option<String>,
    /// Whether a song has to be in a favorite, in none, or either.
    pub favorited: FavoritedFilter,
    /// Only songs in this favorite.
    pub favorite: Option<i64>,
    /// `Some(true)` for songs with a melody channel, `Some(false)` for those without.
    pub melody: Option<bool>,
    /// Only songs whose encoding was decided this way.
    pub encoding_source: Option<String>,
    /// How many copies on disk a song has to have.
    pub copies: CopiesFilter,
    /// How long ago the song was added to the corpus.
    pub added: AddedFilter,
    /// Whether the list collapses a cluster of near-identical files to one row.
    pub versions: VersionsFilter,
    /// Whether this is the corpus or what has been thrown away.
    pub deleted: DeletedFilter,
    /// Only songs not yet in any package.
    pub unpackaged: bool,
    /// Only songs in this package.
    pub in_package: Option<String>,
    /// Lyric granularity to require.
    pub granularity: Option<String>,
    /// Only songs of this kind. `None` shows both, which is the default: a mixed corpus is the
    /// point, and a filter that had to be cleared to see everything would be a worse default.
    pub kind: Option<SongKind>,
    /// What the song is sung in.
    pub language: LanguageFilter,
    /// Languages to leave out, whatever else matches.
    ///
    /// **A field of its own rather than a case of [`LanguageFilter`]**, which is `Copy` and whose
    /// every method takes `self` — a variant holding a list would take that away from all four.
    /// Keeping them apart is also what lets *not these* compose with *this one*, and what leaves
    /// every saved filter spelling `language=<code>` reading exactly as it did.
    ///
    /// A song nothing has placed is not in any of these languages, so it stays: the clause has to
    /// say so outright, since `NOT IN` over a NULL is NULL and would drop every unclassified song
    /// the moment one language was excluded.
    pub language_not: Vec<Language>,
    /// Only songs carrying **any** one of these tags, as slugs.
    ///
    /// OR rather than AND, matching every other surface: the vocabulary is open, so one kind of
    /// song is filed under several words by different hands, and a person picking both means
    /// *either of these*.
    pub tags: Vec<String>,
    /// How to order the results.
    pub sort: Sort,
    /// Rows per page.
    pub limit: u32,
    /// Rows to skip.
    pub offset: u32,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            query: None,
            suitability: SuitabilityFilter::Any,
            user_score: ScoreFilter::Any,
            initial: Initial::Any,
            artist: None,
            folder: None,
            favorited: FavoritedFilter::Any,
            favorite: None,
            melody: None,
            encoding_source: None,
            copies: CopiesFilter::Any,
            added: AddedFilter::Any,
            versions: VersionsFilter::default(),
            deleted: DeletedFilter::default(),
            unpackaged: false,
            in_package: None,
            granularity: None,
            kind: None,
            language: LanguageFilter::Any,
            language_not: Vec::new(),
            tags: Vec::new(),
            sort: Sort::default(),
            limit: 100,
            offset: 0,
        }
    }
}

impl Filter {
    /// The favorite this filter names, when naming it is the only narrowing it does.
    ///
    /// **What it decides is whether a package made from this filter *is* that list**, which is the
    /// one condition under which the package may be sourced from it. A filter of `Brasil Axé` and
    /// Portuguese makes a package that the list alone would not: the first sync would add every song
    /// in the list that is sung in something else, so a package claiming that pair as its source
    /// claims something false the moment anybody presses the button.
    ///
    /// **Built and compared rather than a run of field tests**, so that a term added to this struct
    /// later cannot be forgotten here: a new field arrives at its own default, the comparison sees
    /// it, and a filter carrying it stops being *only this favorite* without anybody remembering to
    /// say so. Eighteen `is_none()` calls would have been eighteen chances to miss the nineteenth.
    ///
    /// The three that do not narrow travel across: the sort, the page, and how many rows are on it.
    ///
    /// **Collapsing a cluster travels across too, and is the one judgment call here.** It decides
    /// which *rows* a page draws rather than which songs the list holds, and a list holding two
    /// files of one recording is a condition the union already answers for: the sync counts them as
    /// a clash, keeps them, and the sentence points at Tidy on the Favorites page.
    pub fn only_this_favorite(&self) -> Option<i64> {
        let favorite = self.favorite?;
        let bare = Self {
            favorite: Some(favorite),
            versions: self.versions,
            sort: self.sort,
            limit: self.limit,
            offset: self.offset,
            ..Self::default()
        };
        (*self == bare).then_some(favorite)
    }

    /// The `WHERE` clause and the values it binds.
    ///
    /// A merged song never appears: it is the same recording as the one it points at, and showing
    /// both is precisely what merging was for. A song set aside as a probable duplicate is hidden
    /// the same way and by default, which [`VersionsFilter`] turns off.
    ///
    /// **Both predicates belong here rather than in the query**, so that `Db::song_count` narrows
    /// with the page it counts. A count taken through a different `WHERE` than the rows is a pager
    /// that offers a page which comes back empty.
    pub fn to_sql(&self) -> (String, Vec<Binding>) {
        // The two halves of `browsable`, and only one of them is constant here: a merge has no row
        // of its own whatever else is asked, where *only deleted* is a list somebody asked for.
        // `browsable` composes the same two for the indexes, taking `DeletedFilter::Live`'s half
        // from the line below — so the ordinary query and the partial indexes cannot drift.
        let mut clauses = vec![
            "s.merged_into IS NULL".to_owned(),
            self.deleted.clause("s."),
        ];
        // **A list shows every song filed in it, versions included.** Collapsing decides which rows of
        // the corpus are offered for filing; inside one favorite it would hide exactly what was filed,
        // and a list of nothing but second versions reads as empty beside a count saying ten. Two
        // versions in one list are what Tidy on the Favorites page is for.
        if self.favorite.is_none() {
            clauses.extend(self.versions.clause());
        }
        let mut values = Vec::new();

        if let Some(query) = &self.query
            && !query.trim().is_empty()
        {
            clauses.push(format!(
                "s.rowid IN (SELECT rowid FROM songs_fts WHERE songs_fts MATCH ?{})",
                values.len() + 1
            ));
            values.push(Binding::Text(fts_match_query(query)));
        }
        // The bound is a `u8` this code produced, never text from a person, so it goes into the
        // fragment rather than costing a binding.
        clauses.extend(self.suitability.clause("s.suitability"));
        clauses.extend(self.user_score.clause("s.user_score"));
        clauses.extend(self.initial.clause(&title_initial("s.")));
        if let Some(folder) = &self.folder {
            let (low, high) = prefix_range(folder);
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM files f WHERE f.song_id = s.id
                         AND f.path >= ?{} AND f.path < ?{})",
                values.len() + 1,
                values.len() + 2
            ));
            values.push(Binding::Text(low));
            values.push(Binding::Text(high));
        }
        if let Some(clause) = self.favorited.clause() {
            clauses.push(clause.to_owned());
        }
        if let Some(favorite) = self.favorite {
            clauses.push(format!(
                "s.id IN (SELECT song_id FROM song_favorites WHERE favorite_id = ?{})",
                values.len() + 1
            ));
            values.push(Binding::Integer(favorite));
        }
        match self.melody {
            Some(true) => clauses.push("s.melody_channel IS NOT NULL".to_owned()),
            Some(false) => clauses.push("s.melody_channel IS NULL".to_owned()),
            None => {}
        }
        if let Some(source) = &self.encoding_source {
            clauses.push(format!("s.det_encoding_source = ?{}", values.len() + 1));
            values.push(Binding::Text(source.clone()));
        }
        if let Some(granularity) = &self.granularity {
            clauses.push(format!("s.granularity = ?{}", values.len() + 1));
            values.push(Binding::Text(granularity.clone()));
        }
        // **Matched on `sort_artist`, which is the fold of the effective artist, and not on
        // `eff_artist` itself.** The fold is `km_song::text::fold` -- the alphabet the A-Z strip
        // files by, the song book prints and both catalogs sort by -- so `DIRE STRAITS`,
        // `Dire Straits` and an accented spelling are one artist here. On a real corpus that is the
        // difference between a filter and a curiosity: the same performer arrives under four
        // spellings across four folders, and an exact match on the raw column would answer "what
        // else did they do?" with a quarter of the answer. It also makes the box forgiving of what
        // somebody types, which an exact filter otherwise is not.
        //
        // `sort_artist` is NULL for a song with no artist at all, so those never match -- right, and
        // the reason there is no "unset" option here: a song with no artist is not *by* anybody, and
        // the question this filter asks does not apply to it.
        //
        // **No index of its own**, like `kind`, `granularity` and `encoding_source` above: this is a
        // residual predicate over whichever sort index the query is already walking.
        // `create_browse_indexes` serves the ten *sorts*, and nothing here changes that.
        if let Some(artist) = &self.artist {
            clauses.push(format!("s.sort_artist = ?{}", values.len() + 1));
            values.push(Binding::Text(km_song::text::fold(artist)));
        }
        if let Some(kind) = self.kind {
            clauses.push(format!("s.kind = ?{}", values.len() + 1));
            values.push(Binding::Text(kind.as_str().to_owned()));
        }
        clauses.extend(self.language.clause(&eff_language("s.")));
        if !self.language_not.is_empty() {
            // **The `IS NULL` leg is the clause, not a nicety.** `NOT IN` over a NULL is NULL rather
            // than true, so without it excluding one language would take every song nothing has
            // placed with it -- which on a corpus mid-classification is most of them, disappearing
            // for a reason the bar does not show. A song with no language is not in any language.
            //
            // The codes are `&'static str` from a closed table rather than text anybody typed, so
            // they go into the fragment, as `LanguageFilter::Is` already does one line up.
            let codes = self
                .language_not
                .iter()
                .map(|language| format!("'{}'", language.code()))
                .collect::<Vec<_>>()
                .join(", ");
            let language = eff_language("s.");
            clauses.push(format!(
                "({language} IS NULL OR {language} NOT IN ({codes}))"
            ));
        }
        clauses.extend(self.copies.clause("s.file_count"));
        clauses.extend(self.added.clause("s.first_seen"));
        if self.unpackaged {
            clauses.push("s.id NOT IN (SELECT song_id FROM package_songs)".to_owned());
        }
        if let Some(package) = &self.in_package {
            clauses.push(format!(
                "s.id IN (SELECT song_id FROM package_songs WHERE package_id = ?{})",
                values.len() + 1
            ));
            values.push(Binding::Text(package.clone()));
        }
        // One `EXISTS` holding an `IN`, which *is* the OR: the subquery stops at the first tag the
        // song carries, seeking `song_tags`' own `(song_id, tag)` key. The same spelling
        // `km_catalog::SearchQuery::to_sql` uses, so the builder and the machine narrow identically,
        // and the emptiness is checked there for the same reason: `IN ()` is a syntax error.
        //
        // **No `songs_browse_*` companion is needed for this**, unlike every filter above it: this
        // is an `EXISTS` against an indexed join table rather than an expression on `songs`, so
        // `create_browse_indexes` has nothing to add. Worth stating, because the obvious assumption
        // from the rest of this function is the opposite.
        if !self.tags.is_empty() {
            let mut placeholders = Vec::with_capacity(self.tags.len());
            for tag in &self.tags {
                values.push(Binding::Text(tag.clone()));
                placeholders.push(format!("?{}", values.len()));
            }
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM song_tags t WHERE t.song_id = s.id AND t.tag IN ({}))",
                placeholders.join(", ")
            ));
        }

        (clauses.join(" AND "), values)
    }

    /// The `ORDER BY` clause. Never built from anything a person typed.
    /// Returns a `String` rather than a `&'static str` because the language ordering is built from
    /// [`eff_language`]. Writing that expression out literally here would be a second copy of it,
    /// free to drift from the one the column and the filter use — which is the thing
    /// `create_browse_indexes` already warns about.
    ///
    /// **Every arm has an index whose key is these terms in this order**, built by
    /// [`Db::create_browse_indexes`]. Changing an arm here without changing its index there does not
    /// fail — it silently goes back to sorting the corpus — so the two are asserted against each
    /// other by the planner tests rather than left to be remembered.
    ///
    /// **Six arms end with [`WITHIN_TITLE`]**, which is where the performer tie-break lives.
    /// [`Sort::Artist`] leads with the performer instead, so it spells its terms out; [`Sort::Duration`]
    /// and [`Sort::Copies`] never reach a title at all.
    ///
    /// **[`Sort::Copies`] is not an exception, and the reason it looks like one is worth knowing.**
    /// Ordering by a correlated `COUNT(*)` over `files` cannot use an index, because an index cannot
    /// carry a value computed from another table — which prices "denormalize the count onto `songs`"
    /// as a schema change and a scan change for one sort option. What settles it the other way is
    /// that a *filter* over copies — [`CopiesFilter`] — pays the same full pass on every page and on
    /// every count, so it is two features rather than one. `songs.file_count` is a real column,
    /// maintained by triggers on `files` rather than by the scan (see `schema.sql`), and this arm
    /// reads it like any other.
    pub fn order_by(&self) -> String {
        match self.sort {
            Sort::Suitability => format!("s.suitability DESC, {WITHIN_TITLE}"),
            // NULLs last: a song nobody has rated is not a song rated zero.
            Sort::UserScore => format!("s.user_score IS NULL, s.user_score DESC, {WITHIN_TITLE}"),
            Sort::Title => WITHIN_TITLE.to_owned(),
            // The performer first, then the title within it — the same two artist terms
            // [`WITHIN_TITLE`] carries, in the other order and for the same reason.
            Sort::Artist => "s.sort_artist IS NULL, s.sort_artist, s.sort_title, s.id".to_owned(),
            Sort::Duration => "s.duration_ms DESC, s.id".to_owned(),
            Sort::Copies => "s.file_count DESC, s.suitability DESC, s.id".to_owned(),
            // Unclassified last, by the same rule the suitability and rating sorts follow: a song nobody has
            // classified is not a song classified as anything.
            Sort::Language => {
                let language = eff_language("s.");
                format!("{language} IS NULL, {language}, {WITHIN_TITLE}")
            }
            // NULLs last again, and here the term carries the column's whole meaning: NULL is
            // "nobody has touched this song", and a song nobody has touched is not a song edited at
            // the epoch. The stamp is fixed-width text to the second, so ordering it as text orders
            // it in time.
            Sort::Updated => format!("s.updated_at IS NULL, s.updated_at DESC, {WITHIN_TITLE}"),
            // No `IS NULL` term: every song has a `first_seen`.
            Sort::Added => format!("s.first_seen DESC, {WITHIN_TITLE}"),
        }
    }
}

/// How an order that has reached one title decides what comes next: the performer, then the id.
///
/// **Two songs sharing a title are two different recordings**, and a content hash is not an order
/// anybody can read — so a page holding five songs called *Goodbye* draws each performer's together
/// rather than scattering them. The id stays as the last term, so the order is total and a page
/// boundary falls in the same place every time.
///
/// **The `IS NULL` term is not decoration and must not be simplified away.** [`eff_artist`] has no
/// `nullif`, so `sort_artist` is `''` for an artist recorded as blank and NULL for one nobody
/// recorded at all, and this term is what keeps the two apart — empty first, absent last. A song
/// nobody named a performer for is not somebody to look up, so it goes after the named ones rather
/// than among the blank ones, which is where the artist sort and the printed book both put it.
///
/// **One constant rather than a copy per arm**, because these terms are also the tail of six index
/// keys in [`Db::create_browse_indexes`]: a term added to an arm and forgotten there does not fail,
/// it silently goes back to sorting the corpus.
pub(super) const WITHIN_TITLE: &str = "s.sort_title, s.sort_artist IS NULL, s.sort_artist, s.id";

/// The same terms as an index key, which names columns of `songs` with no alias to give them.
///
/// **Two spellings of one rule, and the test is what keeps them one.** An index key that stops
/// matching the order it serves does not fail; it goes back to sorting the corpus, which no test of
/// what a page *shows* can see. So the pair is asserted against each other rather than remembered.
pub(super) const WITHIN_TITLE_KEY: &str = "sort_title, sort_artist IS NULL, sort_artist, id";

/// Turns typed text into an FTS5 `MATCH` expression that cannot be syntax.
///
/// Every token is double-quoted, so an apostrophe, a `*`, or the word `OR` is matched literally
/// rather than parsed. The last token gets a prefix `*` because the person is probably still typing
/// it. Copied in spirit from `km_catalog::search::fts_match_query` — the same reasoning, over a
/// different table.
///
/// **A `"` somebody typed asks for the words in that order and next to each other.** Two words in
/// the box match a song holding both anywhere, which is right for a half-remembered fragment and
/// wrong for a line somebody can quote: over a corpus this size *quiet nights* on its own is
/// thousands of songs holding the two words apart. So a quoted run becomes one FTS5 phrase, and a
/// phrase is the one construction the quoting below can safely let through, because its arguments
/// are tokens this function produced rather than text somebody typed.
///
/// **An unclosed `"` closes at the end of the input**, which is the state of the box while somebody
/// is still typing the phrase. The alternative is a query that means something else entirely until
/// the closing mark lands, so the results jump as it is typed and then jump back.
///
/// **A phrase somebody closed takes no prefix `*`, and one still open does.** The star is there
/// because the last word is probably half-typed, which is true of an unclosed phrase and false of a
/// closed one: a phrase with the second mark on it is a phrase meant exactly, and widening its last
/// word would quietly undo the thing the marks were typed to ask for. FTS5 spells the open case
/// `"a b"*`, a phrase prefix, so the two differ by that character alone.
///
/// **In spirit, and not in code — do not merge the two.** They were read as duplicates in a review
/// and are not, in four ways that each belong to their own caller:
///
/// * *Tokenizing.* This splits on every non-alphanumeric character, so `don't` becomes `don` and
///   `t`. The catalog's splits on whitespace and keeps the apostrophe inside the token. A curation
///   tool searches file stems and half-typed fragments; a singer's remote searches song titles,
///   where `don't` is a word.
/// * *Nothing typed.* This answers `""`, a quoted empty string that matches nothing, because a
///   punctuation-only query here should show an empty table. The catalog answers an empty `String`,
///   which its caller reads as *no search at all* and shows everything.
/// * *Escaping.* The catalog doubles an embedded `"`; this one reads it as the mark that opens or
///   closes a phrase, and no `"` reaches the expression except the ones written here.
/// * *Phrases.* A singer typing on a phone is not quoting a line, and a remote with no way to type
///   a `"` easily would be offering a construction nobody could reach.
///
/// Sharing one function would mean picking one set of answers and silently changing the other
/// search box.
pub fn fts_match_query(input: &str) -> String {
    // Odd segments are what sat between one `"` and the next, which makes a trailing unclosed
    // phrase the last segment and needs no separate case.
    let segments: Vec<&str> = input.split('"').collect();
    let closing_mark_is_missing = segments.len().is_multiple_of(2);

    let mut groups: Vec<String> = Vec::new();
    // Whether the last group written is a phrase somebody finished quoting, which is the one case
    // that takes no prefix star.
    let mut ends_closed = false;

    for (index, segment) in segments.iter().enumerate() {
        let words: Vec<&str> = segment
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        if words.is_empty() {
            continue;
        }
        if index % 2 == 1 {
            // One phrase: the whole quoted run inside a single pair of marks, which is FTS5's
            // spelling for *these tokens, in this order, adjacent*.
            groups.push(format!("\"{}\"", words.join(" ")));
            ends_closed = !(closing_mark_is_missing && index == segments.len() - 1);
        } else {
            groups.extend(words.iter().map(|word| format!("\"{word}\"")));
            ends_closed = false;
        }
    }

    if groups.is_empty() {
        // Matches nothing, which is the right answer for a query of only punctuation. An empty
        // string would be an FTS syntax error.
        return "\"\"".to_owned();
    }
    let last = groups.len() - 1;
    groups
        .iter()
        .enumerate()
        .map(|(index, group)| match index == last && !ends_closed {
            true => format!("{group}*"),
            false => group.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
