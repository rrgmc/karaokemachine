//! The shapes the templates render.
//!
//! A thin layer over `km-api`'s DTOs rather than a replacement for them: a row is the API's
//! [`SongDto`] plus the one thing only the remote knows — whether this phone has starred it.
//! Copying the song's fields into a struct of our own would be a third description of a song in a
//! workspace that already argues against the second.
//!
//! Formatting lives here too, and not in the templates. askama can call a method, so a template asks
//! for `row.duration()` and gets `3:42`; the alternative is arithmetic in markup, which is where it
//! stops being testable.

use std::fmt;

use km_api::dto::{NowPlayingDto, QueueEntryDto, SettingsDto, SongDto, StateDto, TransportDto};
use km_songcode::SongCode;
use serde::{Deserialize, Serialize};

/// Which list the Songs tab is showing.
///
/// One page with a mode rather than three pages, because the search box, the language filter and the
/// A–Z strip mean the same thing in all of them and a tap between modes should not lose what is
/// typed in the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Every song.
    #[default]
    Songs,
    /// Artists, then that artist's songs.
    Artists,
    /// Folders, then that folder's songs.
    Favorites,
}

impl Mode {
    /// The value that goes in a URL.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Songs => "songs",
            Mode::Artists => "artists",
            Mode::Favorites => "favorites",
        }
    }

    /// The message id for what the tab says.
    ///
    /// A key rather than the word, because the word is the viewer's and this type has no locale.
    /// The markup renders it with `|t`, which is the rule the catalog states: markup carries a key
    /// and nothing else.
    pub fn label_key(self) -> &'static str {
        match self {
            Mode::Songs => "mode-songs",
            Mode::Artists => "mode-artists",
            Mode::Favorites => "mode-favorites",
        }
    }

    /// The message id for what the search box asks for in this mode.
    pub fn placeholder_key(self) -> &'static str {
        match self {
            Mode::Songs => "search-placeholder-songs",
            Mode::Artists => "search-placeholder-artists",
            Mode::Favorites => "search-placeholder-favorites",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One song in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongRow {
    /// What the catalog says.
    pub song: SongDto,
    /// Whether this phone has filed it anywhere.
    pub starred: bool,
}

impl SongRow {
    /// Its number.
    pub fn number(&self) -> SongCode {
        self.song.number
    }

    /// Its title.
    pub fn title(&self) -> &str {
        &self.song.title
    }

    /// Its artist, or an em dash. Never invented, and never blank — a blank cell in a list of
    /// hundreds reads as a rendering fault rather than as an absent artist.
    pub fn artist(&self) -> &str {
        self.song.artist.as_deref().unwrap_or("—")
    }

    /// Whether an artist is actually known, for deciding whether to show one at all.
    pub fn has_artist(&self) -> bool {
        self.song.artist.is_some()
    }

    /// `3:42`.
    pub fn duration(&self) -> String {
        clock(self.song.duration_ms)
    }

    /// Whether there is anything worth searching YouTube for.
    ///
    /// The row draws the link only where this is true, and *absent* is the right answer rather than
    /// a link that lands on a page of nothing — see [`youtube_query`].
    pub fn searchable(&self) -> bool {
        youtube_query(self.title(), self.song.artist.as_deref()).is_some()
    }

    /// The YouTube search URL, or an empty string when there is nothing worth searching for.
    ///
    /// The template guards on [`searchable`](Self::searchable) first; the empty answer here is
    /// belt-and-braces rather than a case that should ever be rendered.
    pub fn youtube_url(&self) -> String {
        youtube_query(self.title(), self.song.artist.as_deref())
            .map(|query| {
                format!(
                    "https://www.youtube.com/results?search_query={}",
                    crate::prefs::encode(&query)
                )
            })
            .unwrap_or_default()
    }
}

/// What to search YouTube for, or `None` when the row does not say enough to bother.
///
/// **Nothing stores a link.** No catalog, no package and no `SongDto` carries a URL, and this is
/// the same conclusion `km-package-builder` reached with its own `youtube_query`: what a song has is
/// a title and perhaps an artist, and a search is what those two make. Deliberately the *same
/// judgment* as that one, down to the query string it builds, so the tool that curates a corpus and
/// the remote that browses it do not send somebody to two different pages for one song. It differs
/// only in what it can see — the builder has the file's path and can compare a title against it,
/// while a phone browsing a catalog has no path at all — so this is the two-argument form of the
/// same test.
///
/// The guard matters because of what the corpus actually contains. A great many files are titled
/// `CORCOVAD` or `AMD0123`, which is the file's own truncated name rather than a song's, and a
/// search for one of those finds nothing at all. With an artist the title no longer has to stand on
/// its own, which is why the shape test only applies when there is not one.
fn youtube_query(title: &str, artist: Option<&str>) -> Option<String> {
    let title = title.trim();
    let artist = artist.map(str::trim).filter(|value| !value.is_empty());

    if let Some(artist) = artist {
        return Some(if title.is_empty() {
            artist.to_owned()
        } else {
            format!("{artist} {title}")
        });
    }

    if title.is_empty() || looks_like_a_filename(title) {
        return None;
    }
    Some(title.to_owned())
}

/// Whether a title is a file's own name dressed up as one.
///
/// No spaces and no lower-case letters is the shape of a truncated 8.3 name rather than of a title.
/// `CORCOVAD` and `AMD0123` both fail this; `Corcovado` and `Águas de Março` both pass.
fn looks_like_a_filename(title: &str) -> bool {
    !title.contains(' ') && !title.chars().any(char::is_lowercase)
}

/// An entry in the queue, as the page draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRow {
    /// The API's entry.
    pub entry: QueueEntryDto,
    /// Whether it is the first one, which cannot move up.
    pub first: bool,
    /// Whether it is the last one, which cannot move down.
    pub last: bool,
}

impl QueueRow {
    /// Its opaque id. **Every mutation goes by this and never by position** — a page that has been
    /// open for a minute is looking at positions that have since moved.
    pub fn id(&self) -> u64 {
        self.entry.id
    }

    /// Its place in the queue, counting from one, because nobody waiting to sing is zeroth.
    pub fn position(&self) -> usize {
        self.entry.position + 1
    }

    /// Its title.
    pub fn title(&self) -> &str {
        &self.entry.title
    }

    /// Its artist, or an em dash.
    pub fn artist(&self) -> &str {
        self.entry.artist.as_deref().unwrap_or("—")
    }

    /// Who asked for it, when anybody said.
    pub fn singer(&self) -> Option<&str> {
        self.entry.singer.as_deref()
    }
}

/// Everything the player card draws.
///
/// Built from `StateDto` so that a page and a fragment cannot disagree about what "playing" means.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerView {
    /// The whole state, as reported.
    pub state: StateDto,
    /// Whether the machine is reachable at all.
    pub online: bool,
}

impl PlayerView {
    /// What to draw when the machine has not answered.
    ///
    /// Spelled out rather than taken from a `Default` on the API's own types, and the reason is that
    /// there is no honest default for a *setting*. A `SettingsDto::default()` would have to claim
    /// some tempo and some guide-melody state, and a card drawn from it would be asserting things
    /// about a machine nobody has heard from. Here the same values are visibly a placeholder: the
    /// transport is idle, nothing is loaded, and every control on the card is disabled anyway,
    /// because `has_song()` is false.
    pub fn unreachable() -> Self {
        Self {
            state: StateDto {
                transport: TransportDto::Idle,
                now_playing: None,
                position_ms: 0,
                queue_len: 0,
                settings: SettingsDto {
                    transpose: 0,
                    tempo_ratio: 1.0,
                    melody_enabled: false,
                    music_volume: 1.0,
                    lyric_offset_ms: 0,
                },
            },
            online: false,
        }
    }

    /// The song, when there is one.
    pub fn now(&self) -> Option<&NowPlayingDto> {
        self.state.now_playing.as_ref()
    }

    /// Whether anything is loaded.
    pub fn has_song(&self) -> bool {
        self.state.now_playing.is_some()
    }

    /// The loaded song's title, when there is one.
    ///
    /// **Split from [`Self::title_key`] because the two are different kinds of thing.** A song's
    /// title is data out of a corpus and is the same in every language; what to say when there is no
    /// song is a sentence, and belongs in the catalog. One method returning both meant the sentence
    /// could only ever be English.
    pub fn song_title(&self) -> Option<&str> {
        self.state
            .now_playing
            .as_ref()
            .map(|now| now.title.as_str())
    }

    /// The message id for what to say when nothing is loaded.
    ///
    /// Two sentences and not one: a queue with somebody in it is about to start, and saying nothing
    /// is playing to a person who has just queued a song reads as the machine having lost it.
    pub fn title_key(&self) -> &'static str {
        if self.state.queue_len > 0 {
            "now-up-next"
        } else {
            "now-nothing-playing"
        }
    }

    /// Its artist, when there is one.
    pub fn artist(&self) -> Option<&str> {
        self.state.now_playing.as_ref()?.artist.as_deref()
    }

    /// Who asked for it.
    pub fn singer(&self) -> Option<&str> {
        self.state.now_playing.as_ref()?.singer.as_deref()
    }

    /// Whether the machine picked this song for itself because nobody was singing.
    ///
    /// The phone is where this matters most, because it is the screen the queueing happens on. A
    /// demo song has no queue entry to run out — the next one starts the moment it ends — so
    /// somebody *waiting* for it to finish waits for ever. What it needs saying is that queueing is
    /// the way through and takes the deck straight away, which is not what waiting behind a person
    /// does and is the reason this predicate exists at all.
    pub fn is_demo(&self) -> bool {
        matches!(
            self.state.now_playing.as_ref().map(|now| &now.origin),
            Some(km_api::dto::OriginDto::Demo { .. })
        )
    }

    /// Whether asking the machine to play something of its own choosing would do anything.
    ///
    /// The machine's own three conditions minus the one no client can see — whether it has sound at
    /// all — which is why this grays the button out rather than standing in for the refusal: the
    /// machine still has the last word, and it gives it as a sentence.
    ///
    /// **Not the same question as "is demo mode on".** This is a one-shot: with the mode off it is
    /// exactly one song, because chaining is what the mode buys.
    pub fn can_start_demo(&self) -> bool {
        self.online && !self.has_song() && self.state.queue_len == 0
    }

    /// Whether the machine is advancing through a song right now.
    pub fn is_playing(&self) -> bool {
        self.state.transport == TransportDto::Playing
    }

    /// `1:07`.
    pub fn elapsed(&self) -> String {
        clock(self.state.position_ms)
    }

    /// `3:42`, or `--:--` with nothing loaded.
    pub fn total(&self) -> String {
        match &self.state.now_playing {
            Some(now) => clock(now.duration_ms),
            None => "--:--".to_owned(),
        }
    }

    /// How far through, 0–100, for the bar's width.
    pub fn percent(&self) -> u32 {
        let Some(now) = &self.state.now_playing else {
            return 0;
        };
        if now.duration_ms == 0 {
            return 0;
        }
        ((u64::from(self.state.position_ms) * 100) / u64::from(now.duration_ms)).min(100) as u32
    }

    /// Whether the key can be shifted. `false` for every video song, and the button is drawn
    /// disabled rather than hidden — the machine states this per song precisely so a client can gray
    /// a control out instead of collecting a 409.
    pub fn can_transpose(&self) -> bool {
        self.state
            .now_playing
            .as_ref()
            .is_some_and(|now| now.transpose_available)
    }

    /// Whether the tempo can be changed.
    pub fn can_tempo(&self) -> bool {
        self.state
            .now_playing
            .as_ref()
            .is_some_and(|now| now.tempo_available)
    }

    /// Whether a guide melody was found in this song.
    pub fn can_melody(&self) -> bool {
        self.state
            .now_playing
            .as_ref()
            .is_some_and(|now| now.melody_available)
    }

    /// The key shift, as `+2` or `-1` or `0`.
    pub fn transpose(&self) -> String {
        let semitones = self.state.settings.transpose;
        if semitones > 0 {
            format!("+{semitones}")
        } else {
            semitones.to_string()
        }
    }

    /// The tempo, as a percentage of the written one.
    pub fn tempo_percent(&self) -> i32 {
        (self.state.settings.tempo_ratio * 100.0).round() as i32
    }

    /// The music volume, 0–100.
    pub fn volume_percent(&self) -> i32 {
        (self.state.settings.music_volume * 100.0).round() as i32
    }

    /// Whether the guide melody is switched on.
    pub fn melody_on(&self) -> bool {
        self.state.settings.melody_enabled
    }
}

/// Milliseconds as `m:ss`, or `h:mm:ss` past an hour.
///
/// Saturating rather than wrapping: a duration this cannot represent is a broken package, and the
/// remote showing an implausible number beats it panicking in front of a room.
pub fn clock(ms: u32) -> String {
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_reads_the_way_a_track_length_is_written() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(9_000), "0:09");
        assert_eq!(clock(222_000), "3:42");
        assert_eq!(clock(3_600_000), "1:00:00");
        assert_eq!(clock(3_661_000), "1:01:01");
    }

    /// Rounding down, so a song is never reported as finished a fraction before it is.
    #[test]
    fn a_part_second_does_not_round_up() {
        assert_eq!(clock(1_999), "0:01");
    }

    /// An artist is enough on its own: the title no longer has to stand up by itself, so the shape
    /// test does not apply and `CORCOVAD` becomes a perfectly good search beside `Tom Jobim`.
    #[test]
    fn an_artist_makes_any_title_worth_searching_for() {
        assert_eq!(
            youtube_query("CORCOVAD", Some("Tom Jobim")).as_deref(),
            Some("Tom Jobim CORCOVAD")
        );
        assert_eq!(
            youtube_query("", Some("Tom Jobim")).as_deref(),
            Some("Tom Jobim")
        );
    }

    /// The corpus is full of these, and a search for one lands on a page of nothing. Absent beats a
    /// link that goes nowhere.
    #[test]
    fn a_title_that_is_only_a_filename_is_not_worth_searching_for() {
        assert!(youtube_query("CORCOVAD", None).is_none());
        assert!(youtube_query("AMD0123", None).is_none());
        assert!(youtube_query("", None).is_none());
        // A blank artist does not rescue one.
        assert!(youtube_query("CORCOVAD", Some("   ")).is_none());
    }

    /// One word is not the test — capitalisation and spaces are. `Corcovado` is a title and
    /// `CORCOVAD` is what a filesystem did to one.
    #[test]
    fn a_real_title_stands_on_its_own() {
        assert_eq!(
            youtube_query("Águas de Março", None).as_deref(),
            Some("Águas de Março")
        );
        assert_eq!(
            youtube_query("Corcovado", None).as_deref(),
            Some("Corcovado")
        );
    }

    #[test]
    fn the_search_url_is_encoded_rather_than_interpolated() {
        let row = SongRow {
            song: SongDto {
                title: "Highway to Hell".to_owned(),
                ..song(1001, Some("AC/DC"))
            },
            starred: false,
        };
        assert!(row.searchable());
        assert_eq!(
            row.youtube_url(),
            "https://www.youtube.com/results?search_query=AC%2FDC%20Highway%20to%20Hell"
        );
    }

    fn song(number: u32, artist: Option<&str>) -> SongDto {
        SongDto {
            number: SongCode::new(number),
            title: "Tempo Perdido".to_owned(),
            artist: artist.map(str::to_owned),
            language: None,
            kind: km_kmpkg::SongKind::Midi,
            duration_ms: 222_000,
            suitability: Some(9),
            melody_available: true,
            default_transpose: 0,
            package_id: "vol1".to_owned(),
            content_hash: Some(format!("{number:032x}")),
            lyric_preview: Vec::new(),
            tags: Vec::new(),
        }
    }

    /// A blank cell in a list of hundreds reads as a rendering fault. An em dash reads as "nobody
    /// wrote one down", which is what it means.
    #[test]
    fn a_song_with_no_artist_shows_a_dash_rather_than_nothing() {
        let row = SongRow {
            song: song(1, None),
            starred: false,
        };
        assert_eq!(row.artist(), "—");
        assert!(!row.has_artist());
    }

    #[test]
    fn a_row_reports_its_own_length() {
        let row = SongRow {
            song: song(1, Some("Legião Urbana")),
            starred: false,
        };
        assert_eq!(row.duration(), "3:42");
    }
}
