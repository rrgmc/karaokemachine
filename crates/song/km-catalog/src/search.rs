//! Searching the catalog.
//!
//! The interesting part is turning what somebody typed into an FTS5 query. Raw user text cannot go
//! into a `MATCH` clause: FTS5 has its own syntax where `AND`, `OR`, `NOT`, `NEAR`, `*`, `^`, `:`,
//! `-` and quotes are all operators, so a search for `rock 'n' roll` or `AC/DC` is a syntax error at
//! best. Every token is therefore quoted, which makes it a literal, and a trailing `*` is added so
//! typing part of a word finds it.

use std::fmt::Write as _;

/// How results are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    /// Best text match first. Falls back to number order when there is no search text.
    #[default]
    Relevance,
    /// By title.
    Title,
    /// By artist, then title.
    Artist,
    /// By song number.
    Number,
    /// Highest suitability first — the good files before the rough ones.
    Suitability,
    /// Shuffled: a different answer every time, for picking a song nobody asked for.
    ///
    /// **Not on the wire.** `SortDto` — the enum the `sort=` query parameter parses into — has no
    /// counterpart, deliberately: this exists for the machine's own demo mode, which needs one
    /// arbitrary song, and a search route that reorders the whole catalog per request is a full
    /// scan somebody else can ask for repeatedly.
    ///
    /// SQLite does the shuffling, which is what keeps `rand` out of the machine's dependencies. It
    /// is a scan, so pair it with a small `limit`; the alternative — a random `OFFSET` over
    /// [`crate::Library::song_count`] — is rejected for the reason `export_after` gives about
    /// offsets, and because the count can race an install.
    Random,
}

/// Largest page a single search will return.
///
/// The catalog can hold hundreds of thousands of songs, and a remote asking for all of them would
/// build a response nobody can use.
pub const MAX_LIMIT: usize = 500;

/// What to search for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free text, matched against title and artist.
    pub text: Option<String>,
    /// Restrict to one artist, matched as a substring, case-insensitively.
    pub artist: Option<String>,
    /// Restrict to one language, as an ISO 639-1 code.
    ///
    /// Matched **exactly**, where `artist` above is a substring — and the asymmetry is the point. An
    /// artist is typed by a person and half-remembered; a language is a code out of a closed table,
    /// so a substring match would only ever be a way to get the wrong answer. `zh` therefore does
    /// not match anything but `zh`.
    pub language: Option<String>,
    /// Restrict to songs carrying **any** one of these tags.
    ///
    /// OR rather than AND, and that is the whole design of the filter rather than a default. The
    /// vocabulary is open, so one kind of song is filed under several words by different hands:
    /// `rock` on some rows and `rock-nacional` on others, `xmas` beside `natal`. A person picking
    /// both means *either of these*, and an AND over words nobody reconciled answers the second
    /// pick with an empty list. `rock` plus `brasil` is the songs that are one or the other.
    ///
    /// A `Vec` rather than an `Option<Vec>` because empty already means *no tag filter*; there is no
    /// second kind of absence to distinguish, unlike a language, which a song can lack.
    pub tags: Vec<String>,
    /// Leave out songs from these packages, by package id.
    ///
    /// A person hiding packages they never sing from on their own remote. Empty hides nothing.
    pub exclude_packages: Vec<String>,
    /// Only songs whose suitability is at least this.
    pub min_suitability: Option<u8>,
    /// Only songs with a confidently detected melody channel.
    pub melody_only: bool,
    /// How to order results.
    pub sort: SortOrder,
    /// Maximum results, capped at [`MAX_LIMIT`].
    pub limit: usize,
    /// How many to skip.
    pub offset: usize,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: None,
            artist: None,
            language: None,
            tags: Vec::new(),
            exclude_packages: Vec::new(),
            min_suitability: None,
            melody_only: false,
            sort: SortOrder::default(),
            limit: 50,
            offset: 0,
        }
    }
}

/// A value bound into a query.
#[derive(Debug, Clone, PartialEq)]
pub enum Binding {
    /// A string.
    Text(String),
    /// An integer.
    Integer(i64),
}

impl rusqlite::ToSql for Binding {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            Self::Text(value) => value.to_sql(),
            Self::Integer(value) => value.to_sql(),
        }
    }
}

impl SearchQuery {
    /// A text search with default paging.
    pub fn text(query: impl Into<String>) -> Self {
        Self {
            text: Some(query.into()),
            ..Default::default()
        }
    }

    /// The effective limit, after capping.
    pub fn effective_limit(&self) -> usize {
        self.limit.clamp(1, MAX_LIMIT)
    }

    /// Builds the SQL and its bindings.
    pub fn to_sql(&self) -> (String, Vec<Binding>) {
        let mut bindings: Vec<Binding> = Vec::new();
        let mut sql = String::new();
        let columns = crate::song_columns();

        let match_query = self
            .text
            .as_deref()
            .map(fts_match_query)
            .filter(|q| !q.is_empty());

        match &match_query {
            Some(query) => {
                // Joined against the FTS table so `rank` is available for relevance ordering.
                let _ = write!(
                    sql,
                    "SELECT {} FROM songs s \
                     JOIN songs_fts f ON f.rowid = s.id \
                     WHERE songs_fts MATCH ?{}",
                    prefix_columns(columns, "s"),
                    bindings.len() + 1
                );
                bindings.push(Binding::Text(query.clone()));
            }
            None => {
                let _ = write!(
                    sql,
                    "SELECT {} FROM songs s WHERE 1 = 1",
                    prefix_columns(columns, "s")
                );
            }
        }

        if let Some(artist) = &self.artist
            && !artist.trim().is_empty()
        {
            // Folded on both sides, so this narrowing and the artist *list* it is reached from
            // agree about who `Legião Urbana` is. Before, one folded and the other did not, and a
            // singer who typed the name without its tilde got a list row and then no songs.
            bindings.push(Binding::Text(format!(
                "%{}%",
                crate::escape_like(&km_song::text::fold(artist.trim()))
            )));
            let _ = write!(
                sql,
                " AND s.sort_artist LIKE ?{} ESCAPE '\\'",
                bindings.len()
            );
        }
        if let Some(language) = &self.language
            && !language.trim().is_empty()
        {
            // Exact, and lowercased on the way in so a remote passing `PT` still finds the songs a
            // packager stored as `pt`. Every code the packagers write is already lowercase.
            bindings.push(Binding::Text(language.trim().to_lowercase()));
            let _ = write!(sql, " AND s.language = ?{}", bindings.len());
        }
        // One `EXISTS` holding an `IN`, which *is* the OR: the subquery stops at the first tag the
        // song carries. `song_tags` is keyed on `(song_id, tag)`, so the correlation seeks that key
        // and the `IN` reads the handful of rows one song has. `s.tags LIKE '%rock%'` is the
        // spelling to avoid at the size this catalog reaches: it cannot use an index, and it
        // matches `punk-rock`.
        //
        // **The emptiness is checked rather than left to the clause**: an empty list is no filter at
        // all, and `IN ()` is a syntax error.
        //
        // Read through `Tag` so that whatever arrived on the wire is compared in the alphabet the
        // column was written in: `?tags=Forró` finds the songs stored under `forro`. A word that
        // folds to nothing is dropped, not an error — see `km_kmpkg::tag::parse_list`.
        let tags: Vec<String> = self
            .tags
            .iter()
            .filter_map(|tag| km_kmpkg::Tag::parse(tag))
            .map(km_kmpkg::Tag::into_string)
            .collect();
        if !tags.is_empty() {
            let mut placeholders = Vec::with_capacity(tags.len());
            for tag in tags {
                bindings.push(Binding::Text(tag));
                placeholders.push(format!("?{}", bindings.len()));
            }
            let _ = write!(
                sql,
                " AND EXISTS (SELECT 1 FROM song_tags t WHERE t.song_id = s.id AND t.tag IN ({}))",
                placeholders.join(", ")
            );
        }
        push_package_exclusion(
            &mut sql,
            &mut bindings,
            "s.package_id",
            &self.exclude_packages,
        );
        if let Some(min_suitability) = self.min_suitability {
            bindings.push(Binding::Integer(i64::from(min_suitability)));
            let _ = write!(
                sql,
                " AND s.suitability IS NOT NULL AND s.suitability >= ?{}",
                bindings.len()
            );
        }
        if self.melody_only {
            sql.push_str(" AND s.melody_channel IS NOT NULL");
        }

        sql.push_str(match (self.sort, match_query.is_some()) {
            // Relevance is only meaningful when there is something to match against.
            (SortOrder::Relevance, true) => " ORDER BY f.rank, s.number",
            (SortOrder::Relevance, false) => " ORDER BY s.number",
            // The folded key, not `title COLLATE NOCASE`: `NOCASE` is ASCII-only, so it filed every
            // accented title *after* `Z` and `É o amor` came last in a catalog of eleven thousand
            // songs. The same order the offline mirror has been in all along.
            //
            // **Two songs sharing a title are two recordings, so the performer decides between
            // them** — a song number is not an order anybody can read. `sort_artist` is
            // `NOT NULL DEFAULT ''` here, so `= ''` is what sends a song nobody named a performer
            // for to the end of its title rather than to the front of it.
            (SortOrder::Title, _) => {
                " ORDER BY s.sort_key, s.sort_artist = '', s.sort_artist, s.number"
            }
            // A song with no artist keeps its place at the top. `artist COLLATE NOCASE` put NULLs
            // first because SQLite sorts NULL first; `fold` of no artist is the empty string, which
            // sorts first too.
            (SortOrder::Artist, _) => " ORDER BY s.sort_artist, s.sort_key, s.number",
            (SortOrder::Number, _) => " ORDER BY s.number",
            // NULLs last, so unrated songs do not sit above rated ones.
            (SortOrder::Suitability, _) => {
                " ORDER BY s.suitability IS NULL, s.suitability DESC, s.number"
            }
            // No tie-break on `number`: a stable second key would make the same song win every
            // draw among equals, which is the one thing this order exists to avoid.
            (SortOrder::Random, _) => " ORDER BY RANDOM()",
        });

        bindings.push(Binding::Integer(self.effective_limit() as i64));
        let _ = write!(sql, " LIMIT ?{}", bindings.len());
        bindings.push(Binding::Integer(self.offset as i64));
        let _ = write!(sql, " OFFSET ?{}", bindings.len());

        (sql, bindings)
    }
}

/// Appends `AND <column> NOT IN (…)` for the packages a person hid, binding one value per id.
///
/// An empty list appends nothing.
pub(crate) fn push_package_exclusion(
    sql: &mut String,
    bindings: &mut Vec<Binding>,
    column: &str,
    packages: &[String],
) {
    if packages.is_empty() {
        return;
    }
    let mut placeholders = Vec::with_capacity(packages.len());
    for package in packages {
        bindings.push(Binding::Text(package.clone()));
        placeholders.push(format!("?{}", bindings.len()));
    }
    let _ = write!(sql, " AND {column} NOT IN ({})", placeholders.join(", "));
}

/// Qualifies a comma-separated column list with a table alias.
fn prefix_columns(columns: &str, alias: &str) -> String {
    columns
        .split(',')
        .map(|column| format!("{alias}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Turns typed text into a safe FTS5 `MATCH` expression.
///
/// Each token is wrapped in double quotes, which makes FTS5 treat it as a literal string rather than
/// syntax, and internal quotes are doubled. A `*` is appended to the final token so a partial word
/// still matches — somebody typing "beatl" expects to find the Beatles.
pub fn fts_match_query(text: &str) -> String {
    let tokens: Vec<String> = text
        .split_whitespace()
        // Punctuation-only tokens contribute nothing and can leave an empty quoted string, which
        // FTS5 rejects.
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .map(|token| {
            let cleaned: String = token
                .chars()
                .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '\'')
                .collect();
            format!("\"{}\"", cleaned.replace('"', "\"\""))
        })
        .collect();

    if tokens.is_empty() {
        return String::new();
    }
    let last = tokens.len() - 1;
    tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            if index == last {
                format!("{token}*")
            } else {
                token.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_become_quoted_tokens_with_a_prefix_wildcard() {
        assert_eq!(fts_match_query("hello world"), "\"hello\" \"world\"*");
        assert_eq!(fts_match_query("beatl"), "\"beatl\"*");
    }

    #[test]
    fn fts_operators_in_user_input_are_neutralised() {
        // Every one of these would be a syntax error or a silently different query if passed raw.
        for input in [
            "AND",
            "OR",
            "NOT",
            "NEAR(a b)",
            "a*b",
            "^start",
            "col:value",
            "-minus",
        ] {
            let query = fts_match_query(input);
            assert!(
                query.starts_with('"'),
                "{input:?} should be quoted, got {query}"
            );
            // No bare operator characters survive outside the quotes.
            let inner = query.trim_end_matches('*');
            assert!(
                inner.starts_with('"') && inner.ends_with('"'),
                "got {query}"
            );
        }
    }

    #[test]
    fn a_quote_in_the_input_cannot_terminate_a_token_early() {
        // The character filter strips the quotes before the escape step ever sees them, so the
        // result is three clean literals. What matters is the invariant: quotes appear only as the
        // delimiters this function added, never from user input.
        let query = fts_match_query("rock \"n\" roll");
        assert_eq!(query, "\"rock\" \"n\" \"roll\"*");

        // Stated as a property rather than a specific string: every quote must be a delimiter, so
        // they come in pairs around each token.
        for input in [
            "rock \"n\" roll",
            "a\"b",
            "\"\"\"",
            "unbalanced \" quote",
            "trailing\"",
        ] {
            let query = fts_match_query(input);
            assert_eq!(
                query.matches('"').count() % 2,
                0,
                "{input:?} produced unbalanced quotes: {query}"
            );
        }
    }

    #[test]
    fn punctuation_only_input_yields_no_query_rather_than_an_invalid_one() {
        assert_eq!(fts_match_query("!!! ??? ---"), "");
        assert_eq!(fts_match_query(""), "");
        assert_eq!(fts_match_query("   "), "");
    }

    #[test]
    fn a_slash_in_a_band_name_does_not_break_the_query() {
        // AC/DC is the classic case that breaks a naive MATCH.
        let query = fts_match_query("AC/DC");
        assert_eq!(query, "\"ACDC\"*");
    }

    #[test]
    fn an_apostrophe_is_kept_because_it_is_part_of_words() {
        let query = fts_match_query("don't");
        assert!(query.contains("don't"), "got {query}");
    }

    #[test]
    fn the_limit_is_capped_so_a_remote_cannot_ask_for_everything() {
        let query = SearchQuery {
            limit: 100_000,
            ..Default::default()
        };
        assert_eq!(query.effective_limit(), MAX_LIMIT);
    }

    #[test]
    fn a_zero_limit_still_returns_something() {
        let query = SearchQuery {
            limit: 0,
            ..Default::default()
        };
        assert_eq!(query.effective_limit(), 1);
    }

    #[test]
    fn a_text_search_joins_the_index_and_orders_by_rank() {
        let (sql, bindings) = SearchQuery::text("beatles").to_sql();
        assert!(sql.contains("songs_fts MATCH"), "got {sql}");
        assert!(sql.contains("ORDER BY f.rank"), "got {sql}");
        assert_eq!(bindings[0], Binding::Text("\"beatles\"*".to_owned()));
    }

    #[test]
    fn a_search_without_text_does_not_join_the_index() {
        let (sql, _) = SearchQuery::default().to_sql();
        assert!(!sql.contains("songs_fts"), "got {sql}");
        assert!(sql.contains("ORDER BY s.number"), "got {sql}");
    }

    #[test]
    fn relevance_ordering_falls_back_sensibly_with_no_text() {
        let (sql, _) = SearchQuery {
            sort: SortOrder::Relevance,
            ..Default::default()
        }
        .to_sql();
        // `rank` does not exist without a MATCH, so ordering must not reference it.
        assert!(!sql.contains("rank"), "got {sql}");
    }

    #[test]
    fn filters_are_added_as_bound_parameters_never_interpolated() {
        let query = SearchQuery {
            artist: Some("Bobby'; DROP TABLE songs; --".to_owned()),
            min_suitability: Some(7),
            melody_only: true,
            ..Default::default()
        };
        let (sql, bindings) = query.to_sql();
        assert!(
            !sql.contains("DROP TABLE"),
            "user input must not reach the SQL: {sql}"
        );
        assert!(sql.contains("s.sort_artist LIKE ?"), "got {sql}");
        assert!(sql.contains("s.suitability >= ?"), "got {sql}");
        assert!(sql.contains("s.melody_channel IS NOT NULL"), "got {sql}");
        // Lower case, because the artist narrowing folds its needle now — which also turns the
        // semicolons and the `--` into spaces. The assertion is still the one that matters: the
        // words the attacker typed are in a *binding*, not in the statement.
        assert!(
            bindings
                .iter()
                .any(|b| matches!(b, Binding::Text(t) if t.contains("drop table")))
        );
    }

    #[test]
    fn suitability_ordering_puts_unrated_songs_last() {
        let (sql, _) = SearchQuery {
            sort: SortOrder::Suitability,
            ..Default::default()
        }
        .to_sql();
        assert!(
            sql.contains("s.suitability IS NULL, s.suitability DESC"),
            "unrated songs must not outrank rated ones: {sql}"
        );
    }

    /// Within one title the performer decides, and a song nobody named one for comes last.
    ///
    /// `sort_artist` is `NOT NULL DEFAULT ''` here, so the `= ''` term is what makes the unnamed
    /// ones the end of a title rather than the front of it.
    #[test]
    fn the_title_order_breaks_a_tie_on_the_performer() {
        let (sql, _) = SearchQuery {
            sort: SortOrder::Title,
            ..Default::default()
        }
        .to_sql();
        assert!(
            sql.contains("ORDER BY s.sort_key, s.sort_artist = '', s.sort_artist, s.number"),
            "two songs sharing a title are ordered by performer: {sql}"
        );
    }

    #[test]
    fn every_sort_order_produces_an_order_by_clause() {
        for sort in [
            SortOrder::Relevance,
            SortOrder::Title,
            SortOrder::Artist,
            SortOrder::Number,
            SortOrder::Suitability,
            SortOrder::Random,
        ] {
            let (sql, _) = SearchQuery {
                sort,
                ..Default::default()
            }
            .to_sql();
            assert!(sql.contains("ORDER BY"), "{sort:?} produced {sql}");
        }
    }

    /// A stable tie-break would defeat the whole order — see [`SortOrder::Random`].
    #[test]
    fn the_random_order_has_no_second_key_to_make_it_repeat() {
        let (sql, _) = SearchQuery {
            sort: SortOrder::Random,
            ..Default::default()
        }
        .to_sql();
        assert!(sql.contains("ORDER BY RANDOM()"), "got {sql}");
        assert!(
            !sql.contains("RANDOM(), s."),
            "a tie-break would make the same song win every draw: {sql}"
        );
    }

    #[test]
    fn columns_are_qualified_with_the_table_alias() {
        let prefixed = prefix_columns("number, title, artist", "s");
        assert_eq!(prefixed, "s.number, s.title, s.artist");
    }
}
