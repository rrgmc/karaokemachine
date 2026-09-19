//! The `.kmpkg` manifest: what a package says about its songs.
//!
//! This is a **wire format**, so it deliberately does not reuse the analysis types from
//! `km-suitability`. Pinning the manifest to internal representations would turn every refactor of the
//! scoring code into a format change, and would break older packages. Signals and warning codes are
//! stored as strings for the same reason: a reader from before a new warning existed should skip it,
//! not fail to open the package.

use km_songcode::MAX_BANK;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Newest manifest format version this build understands.
///
/// Note that it is **not** what a package is written as: see [`FORMAT_VERSION_MIDI_ONLY`]. What a
/// reader opens is [`FORMAT_VERSIONS_READ`].
pub const FORMAT_VERSION: u32 = 5;

/// The version written for a package that holds only MIDI songs.
///
/// **The version is chosen by content**, and a MIDI-only package's content is the same whatever the
/// format has learned about other kinds: same schema, same entries inside the archive, no `kind`
/// field at all. Moving it would refuse, on every machine, packages those machines read perfectly.
pub const FORMAT_VERSION_MIDI_ONLY: u32 = 1;

/// The version written for a package that holds any media song — video or MP3+G.
///
/// **One constant for both media kinds**, because they differ in *what* a song is and never in *where*
/// it lives: both keep their media inside the archive.
pub const FORMAT_VERSION_MEDIA: u32 = 4;

/// The version written for a package that holds any UltraStar song.
///
/// **A kind whose second entry no format 4 reader looks for.** An UltraStar song's lyric timeline
/// sits beside its audio, and `SongKind::Unknown` already refuses the kind on an older build; the
/// version is what makes that refusal name the build that reads it. A package without one keeps the
/// version it had.
pub const FORMAT_VERSION_ULTRASTAR: u32 = 5;

/// The manifest versions this build opens: exactly the ones it writes.
///
/// **Any other number is refused, older or newer**, by the rule in `A store opens at its current
/// version or is refused`. A version this build does not write is a shape it does not read, and a
/// package read as a shape it does not have fails song by song rather than at the door.
pub const FORMAT_VERSIONS_READ: [u32; 3] = [
    FORMAT_VERSION_MIDI_ONLY,
    FORMAT_VERSION_MEDIA,
    FORMAT_VERSION_ULTRASTAR,
];

/// What kind of file a song is.
///
/// These are not variations on a theme. A MIDI song is a file *inside* the package that the machine
/// parses, transposes and draws lyrics from; a video song is a file *beside* it that is decoded and
/// shown; an MP3+G song is a **pair** of files beside it, one decoded and one drawn by this
/// application from data in the file. Everything that differs between them is reachable from this
/// one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SongKind {
    /// A MIDI or karaoke file, stored inside the package.
    #[default]
    Midi,
    /// A video file, stored beside the package in its media folder.
    Video,
    /// An MP3 and a CD+G graphics file, both stored beside the package in its media folder.
    Cdg,
    /// An MP3 and the lyric timeline read from the UltraStar file beside it, both inside the
    /// package.
    UltraStar,
    /// A kind written by a build newer than this one. Never constructed here.
    ///
    /// **This catch-all is the load-bearing part, and it was added a kind too late to help.**
    /// Without it, `kind: "cdg"` does not merely fail a version check on an older reader — it fails
    /// to deserialize the enum, which fails the whole manifest, so `Package::open` returns a JSON
    /// error *before* `Manifest::problems` can say `UnsupportedFormat`. Raising the format version
    /// cannot fix that retroactively for a binary already in service; nothing written into a file
    /// can. What it does fix is the *next* kind, which will now degrade to a clean refusal naming
    /// the version it needs.
    #[serde(other)]
    Unknown,
}

impl SongKind {
    /// Whether this is a MIDI song.
    ///
    /// Used to keep the field out of a manifest that has no video in it, so packages built before
    /// video existed and packages built after it are byte-identical when they hold the same songs.
    #[must_use]
    pub fn is_midi(&self) -> bool {
        matches!(self, Self::Midi)
    }

    /// Whether this is a video song, whose media lives outside the archive.
    #[must_use]
    pub fn is_video(&self) -> bool {
        matches!(self, Self::Video)
    }

    /// Whether this is an MP3+G song: an MP3 and a `.cdg`, both outside the archive.
    #[must_use]
    pub fn is_cdg(&self) -> bool {
        matches!(self, Self::Cdg)
    }

    /// Whether this is an UltraStar song: an MP3 and a lyric timeline, both inside the archive.
    #[must_use]
    pub fn is_ultrastar(&self) -> bool {
        matches!(self, Self::UltraStar)
    }

    /// Whether the machine draws this song's words, rather than the song bringing its own picture.
    #[must_use]
    pub fn draws_words(&self) -> bool {
        matches!(self, Self::Midi | Self::UltraStar)
    }

    /// The wire name, as it appears in a manifest and in the API.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Midi => "midi",
            Self::Video => "video",
            Self::Cdg => "cdg",
            Self::UltraStar => "ultrastar",
            Self::Unknown => "unknown",
        }
    }

    /// Reads back what [`Self::as_str`] wrote.
    ///
    /// **Anything unfamiliar is [`SongKind::Unknown`]**, which is the same answer `#[serde(other)]`
    /// gives the wire and is what makes this safe at a storage boundary: the offline remote's mirror
    /// keeps this column as `TEXT`, and a row written by a machine that knows a fourth kind must
    /// read back as *a kind I do not know* rather than being quietly called MIDI.
    ///
    /// Named for [`Self::as_str`] rather than `from_str`, which clippy rightly reads as a shadow of
    /// `FromStr::from_str` — and this is infallible, so implementing that trait would mean an
    /// `Err = Infallible` nobody would ever match on.
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "midi" => Self::Midi,
            "video" => Self::Video,
            "cdg" => Self::Cdg,
            "ultrastar" => Self::UltraStar,
            _ => Self::Unknown,
        }
    }

    /// How to name this kind of song in a sentence, for the API's refusal messages.
    #[must_use]
    pub fn article_name(&self) -> &'static str {
        match self {
            Self::Midi => "a MIDI song",
            Self::Video => "a video song",
            Self::Cdg => "an MP3+G song",
            Self::UltraStar => "an UltraStar song",
            Self::Unknown => "this song",
        }
    }
}

/// The file inside the archive that holds the manifest.
pub const MANIFEST_PATH: &str = "manifest.json";

/// How many characters a generated package id has.
///
/// Sixty-four bits written as hex. Named rather than spelled twice because
/// [`PackageMeta::is_generated_id`] refuses what [`PackageMeta::new_id`] writes if the two disagree,
/// and a package nothing can build is the failure that arrangement would produce.
pub const GENERATED_ID_CHARS: usize = 16;

/// A generated id, fixed, for tests and for documentation.
///
/// Public because an id is now refused unless it has the generated shape, so every test in this
/// workspace that builds a package needs one — and sixteen hexadecimal characters is exactly the
/// sort of thing that would otherwise be typed sixteen different ways, one of them wrong. A test
/// that is *about* the shape writes its own value rather than using this.
///
/// The same value the decisions and the doc comments here already use in their examples.
pub const EXAMPLE_ID: &str = "1f4a9c8e2b7d0356";

/// A whole package's manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version. A reader refuses anything newer than it understands.
    pub format: u32,
    /// About the package itself.
    pub package: PackageMeta,
    /// The songs it contains.
    pub songs: Vec<SongEntry>,
}

/// Identification for a package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageMeta {
    /// Stable identifier, used to recognize an upgrade of the same package.
    ///
    /// **Two packages sharing one is the worst failure in the catalog**, and it is why
    /// [`PackageMeta::new_id`] exists. The machine keys an install on this, so a second package
    /// arriving under an id the first already used is not a clash it can report — it is an *upgrade*,
    /// and installing it replaces a volume that has nothing to do with it. `karaoke-vol1` is a name
    /// two packagers would each pick unprompted, so a typed id makes that a matter of luck.
    ///
    /// It is also what [`PackageMeta::suggested_bank`] hashes, so it decides which thousand the
    /// package's songs dial in.
    ///
    /// **Still a free `String`, and the format is unchanged.** A hand-written `km-pack` spec may name
    /// whatever id its author wants — there is no shape enforced here and `Manifest::problems` does
    /// not check one. What changed is only that `km-package-builder` stops *asking* for one.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Package version, free-form.
    pub version: String,
    /// Who made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// When it was built, as an ISO-8601 string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// Which curated set this file is a volume of, when a curation tool divided one.
    ///
    /// **Read by the tool that wrote it and by nothing else.** A machine installs, banks and removes
    /// each volume as the package it is; this only lets a built volume be imported back into the set
    /// it came from. So [`Manifest::problems`] never checks it, and a reader that predates it skips
    /// the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<VolumeOf>,
}

/// The set a package file is one volume of. See [`PackageMeta::volume`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeOf {
    /// The set's own id. The first volume's package id is the same value.
    pub of: String,
    /// The set's name, without the volume number [`PackageMeta::name`] carries.
    pub name: String,
    /// Which volume this is, from 1.
    pub number: u32,
}

impl PackageMeta {
    /// A fresh package id: sixteen hexadecimal characters from the operating system's entropy.
    ///
    /// **Generated rather than typed, because an id collision replaces somebody's package.** See the
    /// note on [`PackageMeta::id`] for what that costs. Sixty-four bits is far past enough — two
    /// packagers would have to build on the order of five billion packages between them before a
    /// shared id became likely — and it is short enough to appear in a URL and an error message.
    ///
    /// **The human-readable half already exists**, so nothing is lost by making this opaque:
    /// `PackageMeta::name` is what a person types and what every surface shows.
    ///
    /// **Not derived from the name, the songs or the time.** A name is exactly the thing two
    /// packagers collide on; a content hash would change the id every time a song was added, which
    /// is the one property an id must not have; and a timestamp collides between two people working
    /// the same afternoon.
    ///
    /// The same shape as `km_api::discover::new_instance_id`, deliberately — that is the other place
    /// in this workspace that needs an identifier nobody chose, and one shape is easier to trust than
    /// two.
    ///
    /// # Panics
    ///
    /// If the operating system will not produce entropy. A package that cannot be identified cannot
    /// be built, so there is no degraded answer to return here.
    #[must_use]
    pub fn new_id() -> String {
        let mut bytes = [0_u8; GENERATED_ID_CHARS / 2];
        getrandom::fill(&mut bytes)
            .expect("the OS must be able to produce entropy for a package id");
        let mut hex = String::with_capacity(GENERATED_ID_CHARS);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    /// Whether an id is one [`Self::new_id`] would produce: sixteen characters, all lowercase hex.
    ///
    /// **The shape is what is checkable, and it is enough.** Randomness cannot be verified from the
    /// value — `0000000000000000` passes — and the point is not to prove entropy. It is that nothing
    /// a person types *as a label* has this shape, so an id can no longer carry the name of the
    /// folder somebody scanned, the client a volume was cut for, or the party it was cut for. See
    /// `A package says nothing about the machine that built it` in `docs/decisions/packaging.md`.
    ///
    /// Refused at [`Manifest::problems`], which is the one gate every route in shares, so a package
    /// that fails this cannot be opened at all.
    #[must_use]
    pub fn is_generated_id(id: &str) -> bool {
        id.len() == GENERATED_ID_CHARS
            && id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    /// The bank a package with this id asks for when it names none.
    ///
    /// Spreading packages over the banks by their own ids is what makes a package land in the *same*
    /// thousand on every machine, whatever order things were installed in — so a book a packager
    /// prints before any machine has seen the files is right on the machines that install them. It
    /// is not a guarantee and does not need to be: a machine that finds the bank taken puts the
    /// package in the next free one and nothing is refused.
    ///
    /// **1 to [`km_songcode::MAX_BANK`], never 0**, and that range is load-bearing rather than tidy.
    /// Bank 0 is the machine's own — see `Bank 0 is the machine's own` — so no package may be given
    /// it. Keeping it out of the hash is what makes the derivation agree with that rather than
    /// merely obey it: a package hashing to 0 would print bank-0 numbers in a book and install
    /// somewhere else, which is the book-against-machine disagreement this whole arrangement exists
    /// to prevent, for one package in ten thousand.
    ///
    /// **SHA-256 rather than `DefaultHasher`, and that is not a stylistic choice.** `DefaultHasher`
    /// is explicitly not stable across Rust releases; `km-catalog` uses one only because both sides
    /// of its comparison are made in a single process. This answer has to be the same number next
    /// year and on somebody else's machine, or the one thing it exists for fails silently.
    ///
    /// **Four digest bytes rather than two, because the modulo bias stopped being ignorable.** Two
    /// bytes span 65,536 values, and `65536 % 9999` is 3,940 — so banks 1 to 3,940 would come up
    /// seven times in 65,536 and the rest six, a 17% edge for the low third of the range. At the old
    /// `MAX_BANK` of 999 the same arithmetic gave a 1.5% edge, which is why it went unremarked.
    /// `u32` spans 4.29 billion, where the leftover is a rounding error.
    pub fn suggested_bank(id: &str) -> u16 {
        let digest = Sha256::digest(id.as_bytes());
        let spread = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
        1 + (spread % u32::from(MAX_BANK)) as u16
    }

    /// The bank this package asks to be dialled in, which is the one its id implies.
    ///
    /// **The one place that question is answered**, and it has to be one place: the machine asks it
    /// to decide where a package installs, and `km_pack::book` asks it to decide what numbers to
    /// print for files no machine has seen. Two copies of the question is how they come to have two
    /// answers — a package printing one number in a book and installing under another. The same
    /// argument that makes `tools/cmd/km-pack` a library as well as a command.
    ///
    /// **A manifest cannot name a bank, and that is what keeps the answer the same everywhere.** A
    /// number a curator typed is true on the machine they typed it for and a guess about every other
    /// one, so a package carrying its own would put its songs in one thousand at home and — after
    /// the collision the machine resolves silently — another thousand at a friend's house, with a
    /// printed book that is right in exactly one of those places. The id is the only thing about a
    /// package that every machine reads the same way. A `bank:` key in an older manifest is read
    /// past rather than refused, because nothing here sets `deny_unknown_fields`.
    ///
    /// Moving an *installed* package is a separate act with a separate route,
    /// `PUT /api/v1/packages/{id}/bank` — the machine's own state, changed by the owner who can see
    /// what else is on it. It reaches 1 to [`km_songcode::MAX_BANK`] and no further: bank 0 is the
    /// machine's own by every road.
    pub fn wanted_bank(&self) -> u16 {
        Self::suggested_bank(&self.id)
    }

    /// What a file holding this package is called, without its extension.
    ///
    /// **The name for a person and the id for the machine**, so a folder of packages can be read at
    /// a glance and two files of one package still collide on purpose. The id is what a machine keys
    /// an install on, so putting it in the name is what makes a rebuilt package land on top of the
    /// one it replaces however the sender happened to name the file.
    ///
    /// A name that slugs away to nothing leaves the id alone, which is a poor label and a perfectly
    /// good file name — and the packages whose names are strangest are exactly the ones somebody
    /// most needs to find in a folder.
    #[must_use]
    pub fn file_stem(&self) -> String {
        // [`Manifest::problems`] refuses a package whose id is not a name, so every route that
        // installs one has already been through that gate. This is the guard for the route that
        // deliberately skips it — `read_manifest_unchecked` reads a manifest in order to *diagnose*
        // it — and it changes nothing for a package that opens at all.
        let id = nameable_id(&self.id);
        let Some(mut slug) = name_slug(&self.name) else {
            return id;
        };
        // Cut before the trim, so a cut landing mid-run cannot leave a stem ending in a dash. The
        // name gets whatever the bound has left after the id and the dash between them. Every
        // character `name_slug` keeps is ASCII, so no index here can fall inside one.
        slug.truncate(MAX_STEM_CHARS.saturating_sub(id.chars().count() + 1));
        let slug = slug.trim_matches('-');
        if slug.is_empty() {
            return id;
        }
        format!("{slug}-{id}")
    }
}

/// The id, as something that can be part of a file name.
///
/// An id that is already a name is returned exactly as it stands, which is what keeps a package
/// installing under the name it has always installed under. Only one that is *not* a name is folded,
/// and it is folded rather than refused because this is reached from the diagnostic read alone —
/// [`Manifest::problems`] has already refused the package everywhere else, and a diagnosis that
/// panicked or escaped a folder would be worse than a diagnosis under an odd name.
///
/// Bounded here as well as in [`PackageMeta::file_stem`]'s other arm, because an id used on its own
/// is the one path to a name that nothing else trims. The cut is by characters, so it cannot land
/// inside one.
fn nameable_id(id: &str) -> String {
    let usable = if is_safe_name(id) {
        id.to_owned()
    } else {
        name_slug(id).unwrap_or_else(|| UNNAMED_ID.to_owned())
    };
    usable.chars().take(MAX_STEM_CHARS).collect()
}

/// What an id folds to when it holds nothing that could be part of a name.
///
/// Only ever reached through the diagnostic read, so it labels a package that will not install
/// rather than one a person will go looking for in a folder.
const UNNAMED_ID: &str = "package";

/// The longest stem [`PackageMeta::file_stem`] will build, in characters.
///
/// Windows keeps `MAX_PATH` at 260 for a path that is not `\\?\`-prefixed, and a data directory has
/// already spent some of it. The same bound `km_api`'s upload staging applies, for the same reason.
///
/// **The bound is here and not in [`name_slug`]**, because it is a fact about a *file name* and not
/// about the fold. An id derived from a name is the same characters and has no `MAX_PATH` problem,
/// so a length rule pushed down into the fold would shorten one for the other's reason.
const MAX_STEM_CHARS: usize = 80;

/// A name, folded into something that survives as a file name on three platforms.
///
/// Fold case, one dash for any run of spaces and dashes, and keep letters, digits, dots, dashes and
/// underscores — a rule somebody can predict from looking at the answer. `None` when what is left
/// holds no letter or digit, which is the caller's cue that this name cannot label anything.
///
/// Only ASCII letters are kept, deliberately: `Músicas Brasileiras` becomes `msicas-brasileiras`,
/// which is ugly and unambiguous, where transliterating would be a table of somebody's opinions
/// about other people's alphabets — the same judgment `One alphabet, everywhere` makes about a fold
/// table.
///
/// **This is where the rule lives for everything in the root workspace**, asked by
/// [`PackageMeta::file_stem`] and by `km-package-builder`. `km-admin` keeps a copy because
/// `tools/cmd/assets/` is a second cargo workspace, and `karaokemachine`'s `bank_id` is a different
/// rule that also folds dots. See `A wallpaper pack may be named` in `docs/decisions/interface.md`.
#[must_use]
pub fn name_slug(name: &str) -> Option<String> {
    let mut slug = String::new();
    for character in name.trim().to_lowercase().chars() {
        match character {
            'a'..='z' | '0'..='9' | '.' | '_' => slug.push(character),
            ' ' | '-' if !slug.ends_with('-') => slug.push('-'),
            _ => {}
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    slug.chars()
        .any(|character| character.is_ascii_alphanumeric())
        .then_some(slug)
}

/// One song in a package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SongEntry {
    /// The song's number **inside this package** — its slot, 1 to [`km_songcode::MAX_SLOT`].
    ///
    /// Not the number a singer dials: the machine adds the bank it assigned this package, and
    /// `bank * 1000 + slot` is what reaches the keypad and the book. Inside a package the number is
    /// already scoped by the package, so it stays a bare `u32` and never a `SongCode`.
    pub number: u32,
    /// Song title.
    pub title: String,
    /// Performer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    /// What language the song is sung in, as an ISO 639-1 code — see [`crate::Language`].
    ///
    /// A `String` rather than a `Language` **on purpose**, and it is the same argument the module
    /// doc makes about signals and warning codes: the type is closed, the wire is not. A build from
    /// the future may carry a code this one has never heard of, and the package must *open*,
    /// showing the value as it stands, rather than failing. Everything that **writes** one goes through
    /// [`crate::Language`], so a package built by this version only ever holds a real code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// What kind of song this is.
    ///
    /// Absent means [`SongKind::Midi`]: the writer leaves it out for every MIDI song, so a MIDI-only
    /// manifest carries no `kind` field at all.
    #[serde(default, skip_serializing_if = "SongKind::is_midi")]
    pub kind: SongKind,
    /// Where the song's file is.
    ///
    /// For a MIDI song, a path *inside* the archive. For a video song, a file name inside the
    /// package's media folder — the sibling directory named after the package — because a video is
    /// tens of megabytes and reading an archive entry means reading the whole thing into memory.
    pub file: String,
    /// Length in milliseconds at the written tempo.
    pub duration_ms: u32,
    /// Encoding to decode the lyrics with. Overrides detection, which is the whole point: a packager
    /// fixes it once here instead of every listener seeing mojibake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyric_encoding: Option<String>,
    /// Transposition to apply by default, for a file written in an awkward key.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub default_transpose: i8,
    /// Whether the machine plays this song and draws none of its words.
    ///
    /// What it is for: a file can be a good arrangement and a bad karaoke song, because its lyric
    /// track is mistimed, is the arranger's business card, or is a chord chart. An empty screen is
    /// better than text nobody can follow, and the television says `no lyrics` in the corner so the
    /// emptiness is explained rather than mysterious.
    ///
    /// **Both detected and edited**, which is [`Self::fixes`]'s shape rather than
    /// [`Self::default_transpose`]'s. `km_suitability::Suitability::words_cannot_be_followed` names
    /// the three faults that answer this without a person, so a rebuild re-derives it; a person who
    /// disagrees is carried over by [`EditedField::LyricsHidden`] — **including a person who says
    /// the words are to be drawn**, which is this field false with the marker set, the one state
    /// the field alone cannot express.
    ///
    /// **Only a song whose words the machine draws can carry it.** A video or MP3+G song's words are
    /// pixels in a picture, so there is nothing here to suppress — see [`SongKind::draws_words`].
    ///
    /// **This does not move the format version, deliberately**, for the reason [`Self::tags`] and
    /// [`Self::loudness`] did not: the version is chosen by content (see
    /// [`FORMAT_VERSION_MIDI_ONLY`]) and nothing here sets `deny_unknown_fields`, so a build that
    /// predates this field ignores it and a package whose songs all draw their words is
    /// byte-identical to what an earlier build wrote.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lyrics_hidden: bool,
    /// Corrections for defects in the file's own MIDI events.
    ///
    /// **The one field that is both detected and edited**, where [`Self::default_transpose`] is only
    /// edited and [`Self::melody`] is only detected. A rebuild from source re-derives the list, so a
    /// defect found by a newer detector reaches a package that was built before it existed — unless
    /// somebody has touched it, in which case [`EditedField::Fixes`] carries their list over whole.
    /// A person's answer to *should this channel be silent* is not a fact about the bytes and cannot
    /// be re-derived; the detector's answer is.
    ///
    /// Empty for a video or MP3+G song, which have no MIDI events to correct.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixes: Vec<km_fixes::Fix>,
    /// The melody channel, when detection was confident. `null` when it was not — the machine must
    /// not guess at playback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub melody: Option<MelodyRecord>,
    /// Why no melody channel was claimed, kept so a low suitability is explainable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub melody_abstained: Option<String>,
    /// How well the file works as a karaoke song.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suitability: Option<SuitabilityRecord>,
    /// The first couple of lines a singer would actually see, when the file has words in it.
    ///
    /// What it is for: a package could say a song's number, title, artist, suitability and language, and
    /// carried nothing anybody could *read* to recognize it. Two lines is enough to tell `Tempo
    /// Perdido` from another song of the same name, and short enough that a four-thousand-song
    /// manifest does not become a lyric database.
    ///
    /// **Detected, never edited.** It is a fact about the bytes, like the duration and the suitability, so
    /// it is absent from [`fields`], from [`Self::inherit_edits_from`] and from `km_pack`'s
    /// `apply_edits`: a rebuild re-derives it and there is nothing for a person to correct.
    ///
    /// **Empty for a video song and for an MP3+G song**, and that is what they are rather than a gap
    /// — the words in both are pixels in a picture. The same reasoning as the `Searching a video's
    /// words` decision in `docs/decisions/`.
    ///
    /// **This did not move the format version, deliberately.** The version is chosen by content (see
    /// [`FORMAT_VERSION_MIDI_ONLY`]) and nothing here sets `deny_unknown_fields`, so a build that
    /// predates this field ignores it and opens the package exactly as before. A package *gains* a
    /// preview only by being rebuilt; installing an old one again cannot conjure words that are not
    /// in its manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lyric_preview: Vec<String>,
    /// What somebody filed this song under — `rock`, `anime`, `brasil`. See [`crate::Tag`].
    ///
    /// `Vec<String>` rather than `Vec<Tag>` for the reason [`Self::language`] is a `String`: the type
    /// is closed, the wire is not. A hand-edited manifest may hold anything, and it must *open*
    /// rather than fail. Everything that **writes** one goes through [`crate::Tag`], so a package
    /// built by this version only ever holds folded slugs.
    ///
    /// **Hand-set, never detected**, which is the whole difference from every other field here.
    /// Nothing reads a file and concludes it is rock, so a tag exists only because a person said so
    /// — and that is why it is in [`fields`] and in [`Self::inherit_edits_from`] while
    /// [`Self::lyric_preview`] is in neither. A rebuild would lose every tag otherwise.
    ///
    /// **Optional, unlike a language.** A build refuses a song with no language; nothing refuses a
    /// song for having no tag, and there is no package-level default to fall back on, because a
    /// default would be a value nobody checked.
    ///
    /// **This does not move the format version**, for the same reason [`Self::lyric_preview`] did
    /// not: the version is chosen by content and nothing here sets `deny_unknown_fields`, so a build
    /// that predates this field ignores it and an untagged package is byte-identical to what an
    /// earlier build wrote.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// How loud the song's audio is, measured when the package was built.
    ///
    /// What it is for: a video or an MP3+G song is mastered by whoever published it, loud, and every
    /// publisher differently — so on one machine at one setting of one amplifier they play 8 to 13 dB
    /// above a MIDI song and move within that range from one file to the next. The machine attenuates
    /// each to the level its SoundFont bank already plays at, and this is the number it needs to do
    /// that. See the `Video and MP3+G play at the MIDI reference level` decision in
    /// `docs/decisions/audio.md`.
    ///
    /// **Absent for a MIDI song, and that is what it is rather than a gap.** A MIDI song is the
    /// reference: it has no level of its own until a bank renders it, and which bank that is belongs
    /// to the machine playing it rather than to the package.
    ///
    /// **Detected, never edited.** It is a fact about the bytes, like the duration and the
    /// [`Self::lyric_preview`], so it is absent from [`fields`], from [`Self::inherit_edits_from`]
    /// and from `km_pack`'s `apply_edits`: a rebuild re-derives it and there is nothing for a person
    /// to correct. Being outside those lists means a rebuild *replaces* it rather than inheriting
    /// it, which reads like the defect `docs/architecture/packaging.md` warns about — a field left
    /// out of that list quietly vanishing — and here is the wanted behaviour, because the only thing
    /// that should change a measurement is measuring again.
    ///
    /// **This did not move the format version, deliberately**, for the reason
    /// [`Self::lyric_preview`] and [`Self::tags`] did not: the version is chosen by content (see
    /// [`FORMAT_VERSION_MIDI_ONLY`]) and nothing here sets `deny_unknown_fields`, so a build that
    /// predates this field ignores it and opens the package exactly as before. A package *gains* a
    /// measurement only by being built or re-analysed by a build that has this field; installing an
    /// older one again cannot conjure a level that is not in its manifest, and a song without one
    /// plays at gain 1.0, exactly as it does today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<LoudnessRecord>,
    /// Hash of the MIDI bytes, for spotting the same recording filed under two numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Names of fields that were set by hand rather than read from the file.
    ///
    /// Detection gets titles and artists wrong — files name a producer where the title belongs, or
    /// give no metadata at all — so a person has to be able to correct them. Recording *which*
    /// fields were corrected is what makes those corrections durable: re-analysis and rebuilds
    /// preserve them instead of overwriting them with the same wrong guess.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edited: Vec<EditedField>,
}

/// A field that can be recorded in [`SongEntry::edited`].
///
/// **A closed type where this was seven `&str` constants**, and the difference is what the mechanism
/// is for. `mark_edited` and `is_edited` took a `&str`, so a typo at any of the ~20 call sites in
/// `km-pack` and `km-package-builder` compiled, marked a field nothing recognised, and was then
/// dropped by `inherit_edits_from`'s `_ => continue` — silently discarding somebody's hand
/// correction on the next rebuild from source, which is the exact failure the `edited` list exists
/// to prevent. The `fields::` constants were a real mitigation and were still a convention rather
/// than a type.
///
/// **The wire stays open.** `#[serde(other)] Unknown` means a manifest written by a later build,
/// naming a field this one has never heard of, still deserializes — and `inherit_edits_from` leaves
/// it alone exactly as the `_` arm did. What changed is that *this* build can no longer invent one
/// by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditedField {
    /// The song title.
    Title,
    /// The performer.
    Artist,
    /// The language code.
    Language,
    /// The song's tags.
    ///
    /// Present even though a tag has no detected half to be overwritten by, because
    /// [`SongEntry::inherit_edits_from`] walks this list and nothing else: a tag left out of it
    /// would survive in the manifest and vanish on the next rebuild from source.
    Tags,
    /// The lyric encoding.
    LyricEncoding,
    /// The default transposition.
    DefaultTranspose,
    /// The song number.
    Number,
    /// The list of corrections applied to the song's own events.
    ///
    /// Present although the list has a detected half, and the two are not in conflict: the marker
    /// says a person has decided, and a rebuild then keeps their answer instead of the detector's.
    Fixes,
    /// The melody channel.
    ///
    /// Marked for [`Self::Fixes`]'s reason and not [`Self::Title`]'s: the field has a detected half,
    /// and the marker is what makes a rebuild keep a person's answer rather than the detector's.
    /// **A song somebody said has no melody carries the marker with [`SongEntry::melody`] absent**,
    /// which is the one state the field alone cannot express — absent otherwise means detection
    /// abstained, and the two must not be confused by a rebuild.
    Melody,
    /// Whether the song's words are drawn.
    ///
    /// Marked for [`Self::Fixes`]'s reason: the field has a detected half, and the marker is what
    /// makes a rebuild keep a person's answer rather than the detector's. **A song somebody said is
    /// to draw its words carries the marker with [`SongEntry::lyrics_hidden`] false**, which is the
    /// one state the field alone cannot express — false otherwise means detection found nothing
    /// wrong, and a rebuild must not confuse the two.
    LyricsHidden,
    /// A field named by a build newer than this one. Never constructed here.
    ///
    /// Carried through a round trip untouched, so a rebuild by an older build does not silently drop
    /// a marker it did not understand — the same reason [`SongKind::Unknown`] exists.
    #[serde(other)]
    Unknown,
}

impl EditedField {
    /// The name as it appears in a manifest, and in `km-pack export`'s CSV.
    ///
    /// `serde(rename_all = "snake_case")` above produces exactly these, which is what keeps the two
    /// from drifting.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Language => "language",
            Self::Tags => "tags",
            Self::LyricEncoding => "lyric_encoding",
            Self::DefaultTranspose => "default_transpose",
            Self::Number => "number",
            Self::Fixes => "fixes",
            Self::Melody => "melody",
            Self::LyricsHidden => "lyrics_hidden",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for EditedField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl SongEntry {
    /// Records that a field was set by hand.
    pub fn mark_edited(&mut self, field: EditedField) {
        if !self.edited.contains(&field) {
            self.edited.push(field);
        }
    }

    /// Whether a field was set by hand, and so must not be overwritten by detection.
    #[must_use]
    pub fn is_edited(&self, field: EditedField) -> bool {
        self.edited.contains(&field)
    }

    /// Copies every hand-edited field from an earlier version of the same song.
    ///
    /// Used when a package is rebuilt from its source folder: the files are re-read and re-analyzed,
    /// but corrections a person made survive. Songs are matched by content hash, which is stable
    /// across rebuilds in a way file paths and numbers are not.
    pub fn inherit_edits_from(&mut self, previous: &SongEntry) {
        for field in &previous.edited {
            match field {
                EditedField::Title => self.title = previous.title.clone(),
                EditedField::Artist => self.artist = previous.artist.clone(),
                EditedField::Language => self.language = previous.language.clone(),
                EditedField::Tags => self.tags = previous.tags.clone(),
                EditedField::LyricEncoding => self.lyric_encoding = previous.lyric_encoding.clone(),
                EditedField::DefaultTranspose => {
                    self.default_transpose = previous.default_transpose
                }
                EditedField::Number => self.number = previous.number,
                EditedField::Fixes => self.fixes = previous.fixes.clone(),
                // Both fields, because *no melody* is `melody` absent and `melody_abstained` saying
                // why detection gave up. Carrying the first alone would leave a rebuilt entry
                // claiming a person's silence was the detector's.
                EditedField::Melody => {
                    self.melody = previous.melody.clone();
                    self.melody_abstained = previous.melody_abstained.clone();
                }
                // The preview travels with it, because a song whose words are not drawn carries
                // none. It is otherwise detected, so a rebuild fills it again from the file, and
                // carrying the flag without it would put the words back into the song book.
                EditedField::LyricsHidden => {
                    self.lyrics_hidden = previous.lyrics_hidden;
                    if self.lyrics_hidden {
                        self.lyric_preview.clear();
                    }
                }
                // A field name from a later build: left alone rather than guessed at. Exhaustive
                // now, so adding a variant above is a compile error here rather than a silent drop.
                EditedField::Unknown => continue,
            }
            self.mark_edited(*field);
        }
    }
}

fn is_zero(value: &i8) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A confidently detected melody channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MelodyRecord {
    /// The channel, 0-based.
    pub channel: u8,
    /// How strongly the evidence favored it.
    pub confidence: f32,
    /// Which signals fired. Strings, so an unfamiliar one is readable rather than fatal.
    pub signals: Vec<String>,
}

/// How loud a song's audio was measured to be, in the units EBU R128 reports.
///
/// **A wire format, like everything else in this module** — deliberately not `km_loudness`'s own
/// `Loudness`, on the rule the module documentation states: reusing an analysis type would turn a
/// change in measuring code into a change of package format.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnessRecord {
    /// Integrated loudness over the whole song, in LUFS. Negative for anything but a divergence.
    pub lufs: f32,
    /// The loudest true peak in either channel, in dBTP, where `0.0` is full scale.
    ///
    /// **Carried although the machine does not read it**, which is worth stating so it is not
    /// mistaken for something load-bearing that stopped being read. Levelling only ever attenuates,
    /// so nothing needs a peak to know a gain is safe; a peak is what would be needed to decide
    /// whether a *quiet* song could safely be raised, and having measured it means that question can
    /// be answered from the packages people already have rather than reopened with a guess.
    #[serde(default, skip_serializing_if = "is_no_peak")]
    pub peak_dbtp: f32,
}

/// Whether a peak is the absent-value stand-in, so it is left out of the JSON.
///
/// `serde` needs a function here and `f32` has no `is_empty`. Zero is the right stand-in rather than
/// a sentinel worth avoiding: 0.0 dBTP is exactly full scale, a measurement no real file lands on to
/// the bit, and a manifest that omits the field reads back as a song whose peak nobody recorded.
fn is_no_peak(value: &f32) -> bool {
    *value == 0.0
}

/// The 0-10 suitability and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuitabilityRecord {
    /// 0 to 10. Rates the file, never a singer.
    ///
    /// **`value`, and nothing else.** The word `score` is refused because this project does not score
    /// singers and the two readings are one keystroke apart. Accepting it on the way in — on the
    /// reasoning that a package is a file somebody already has — is the right argument about a
    /// released product and is not one about this one.
    ///
    /// **Of the refused spellings this is the one that costs something**, and what it costs is
    /// this: a `.kmpkg` whose manifest says `score` fails to open, because this field has no serde
    /// default and so has no `value`. That is loud rather than silent — the alternative, a default of 0, would sort a good package to the bottom of
    /// every list without a word — and the remedy is one command, `km-pack build`, over the
    /// description the package was built from. See `No compatibility aliases` in
    /// `docs/decisions/songs.md`.
    pub value: u8,
    /// Where it came from.
    pub breakdown: BreakdownRecord,
    /// What is wrong with the file.
    #[serde(default)]
    pub warnings: Vec<WarningRecord>,
}

impl SuitabilityRecord {
    /// Full marks, for a file that was made to be sung to and is sung for long enough to be worth it.
    ///
    /// **A video song and an MP3+G song rate 10, by what they are rather than by measurement.** The
    /// 0–10 asks how good a file is as a karaoke source, and these were manufactured as
    /// karaoke: a commercial disc or a karaoke video has the words, has them timed by whoever
    /// authored it, and is a real backing track rather than somebody's sketch. An absent number
    /// sorts such a file below a mediocre MIDI file, which is the opposite of the truth. See the
    /// `Suitability, for a song that was made to be sung to` decision in `docs/decisions/`.
    ///
    /// **One thing about such a file is in doubt, and it is how much of it is sung.** A thirty-second
    /// video is a clip or a fragment of a rip rather than a karaoke track, so full marks are for a
    /// file with enough singing in it and [`Self::too_brief_to_choose`] is for one without.
    /// `km_pack::purpose_made_suitability` is the one place that decides between them, because it is
    /// where both the threshold and the name of the warning are in scope.
    ///
    /// The breakdown is filled to match rather than left at zero, so it cannot contradict the number
    /// it is supposed to explain. It is a derivation that does not apply here, not a measurement
    /// that happened to come out full.
    #[must_use]
    pub fn purpose_made() -> Self {
        Self {
            value: 10,
            breakdown: BreakdownRecord {
                lyrics: 3,
                sync: 3,
                channels: 2,
                arrangement: 2,
            },
            warnings: Vec::new(),
        }
    }

    /// What a file made to be sung to scores when there is too little of it to sing.
    ///
    /// The same four points a MIDI file of the same length keeps, the backing being spread across
    /// channels and a real arrangement, neither of which a short file stops being. Nothing for the
    /// words or their timing, which is what a file over in forty seconds has to offer. The caller
    /// supplies the warning, because the name of it belongs to the analysis rather than to the
    /// container.
    #[must_use]
    pub fn too_brief_to_choose(warning: WarningRecord) -> Self {
        Self {
            value: 4,
            breakdown: BreakdownRecord {
                lyrics: 0,
                sync: 0,
                channels: 2,
                arrangement: 2,
            },
            warnings: vec![warning],
        }
    }
}

/// The suitability's components.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakdownRecord {
    /// Lyrics present and how finely timed, 0 to 3.
    pub lyrics: u8,
    /// Lyric timings against the music, 0 to 3.
    pub sync: u8,
    /// The backing spread across channels rather than piled onto one, 0 or 2.
    pub channels: u8,
    /// A real arrangement rather than a sketch, 0 to 2.
    pub arrangement: u8,
}

/// One problem found in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarningRecord {
    /// Machine-readable code.
    pub code: String,
    /// What is wrong, in words a packager can act on.
    pub message: String,
}

/// Something wrong with a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestProblem {
    /// The format version is not one this build reads — see [`FORMAT_VERSIONS_READ`].
    UnsupportedFormat(u32),
    /// Two songs share a number.
    DuplicateNumber(u32),
    /// Song number 0, which no remote can dial.
    ZeroNumber,
    /// A song number above [`km_songcode::MAX_NUMBER`], which no keypad can dial either.
    NumberTooLarge(u32),
    /// A song has no title.
    MissingTitle(u32),
    /// A song points at no file.
    MissingFile(u32),
    /// A file path escapes the archive.
    UnsafePath {
        /// The song number.
        number: u32,
        /// The offending path.
        path: String,
    },
    /// A field the installed file's name is built from holds something that is not a name.
    ///
    /// **The one problem here that is about the package rather than a song.** The machine names an
    /// installed package from the manifest, so a separator or a drive letter in the id puts the
    /// file somewhere other than the packages folder — and a package handed to a machine by the
    /// operating system reaches that before anybody has typed a password.
    UnsafeName {
        /// Which field: `id` or `version`.
        field: &'static str,
        /// The offending value.
        value: String,
    },
    /// The package's id is one somebody typed rather than one a build generated.
    ///
    /// **A package is handed to a stranger**, and an id a person chose says what they chose it
    /// after — the folder they scanned, the client, the party. A generated id says nothing, and it
    /// is also the only kind that cannot collide, which matters because the machine keys an install
    /// on the id and a second package arriving under a used one replaces a volume that has nothing
    /// to do with it.
    TypedId {
        /// The offending id.
        id: String,
    },
    /// The package has no songs.
    Empty,
    /// The package holds more songs than a bank has room for.
    ///
    /// A package's songs are numbered 1 to [`km_songcode::MAX_SLOT`] and the machine adds the bank, so
    /// a package larger than that has songs that would dial into the **next** package's block.
    TooManySongs {
        /// How many songs it holds.
        count: usize,
    },
    /// Two songs have identical content under different numbers.
    DuplicateContent {
        /// The number kept.
        first: u32,
        /// The number that duplicates it.
        second: u32,
    },
    /// An MP3+G or UltraStar song's `file` does not name an audio file, so the entry beside its
    /// audio cannot be found.
    FileNotAudio {
        /// The song number.
        number: u32,
        /// What kind of song it is.
        kind: SongKind,
        /// The path it named.
        path: String,
    },
    /// A song declares a `lyric_encoding` that names no encoding anybody can decode with.
    ///
    /// **Reported here because playback deliberately will not report it.** `TextDecoder::resolve`
    /// falls through to ordinary detection on an unrecognised label — refusing to play a song over
    /// a manifest typo would be the worse failure — so a packager who wrote `cp-1252` got
    /// detection's guess and no word from anywhere. The symptom was mojibake on exactly the songs
    /// the field had been set to fix, which is the one thing this field exists to prevent.
    ///
    /// The vocabulary is the WHATWG Encoding Standard's; `km_song::encoding::is_known_label` is the
    /// same test the decoder itself applies.
    UnknownEncoding {
        /// The song number.
        number: u32,
        /// The label it declared.
        label: String,
    },
}

impl std::fmt::Display for ManifestProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(version) => write!(
                f,
                "manifest format {version} is not one this build reads (it reads \
                 {FORMAT_VERSION_MIDI_ONLY}, {FORMAT_VERSION_MEDIA} and {FORMAT_VERSION_ULTRASTAR})"
            ),
            Self::DuplicateNumber(number) => write!(
                f,
                "song number {number} is used more than once; a number is how a singer asks for a \
                 song, so it has to be unique"
            ),
            Self::ZeroNumber => write!(f, "song number 0 cannot be dialled on a remote"),
            Self::NumberTooLarge(number) => write!(
                f,
                "song number {number} is above {}; a package numbers its songs 1 to {} and the \
                 machine adds the bank, so this song would dial into the next package's numbers",
                km_songcode::MAX_SLOT,
                km_songcode::MAX_SLOT
            ),
            Self::MissingTitle(number) => write!(f, "song {number} has no title"),
            Self::MissingFile(number) => write!(f, "song {number} names no file"),
            Self::UnsafePath { number, path } => {
                write!(f, "song {number} points outside the archive: {path:?}")
            }
            Self::UnsafeName { field, value } => write!(
                f,
                "the package's {field} is {value:?}, which cannot be part of a file name; the \
                 machine names an installed package from the manifest, so this one would be \
                 written outside the packages folder"
            ),
            Self::TypedId { id } => write!(
                f,
                "the package's id is {id:?}, which somebody typed; an id is {GENERATED_ID_CHARS} \
                 hexadecimal characters a build generates, so that a package says nothing about \
                 where it was made and no two packages can claim to be each other. There is no \
                 converter: rebuild it from its description with `km-pack build \
                 <description>.kmspec.yaml`"
            ),
            Self::Empty => write!(f, "the package contains no songs"),
            Self::TooManySongs { count } => write!(
                f,
                "the package holds {count} songs and a package holds at most {}; packages here are \
                 curated by hand, so split it rather than growing it",
                km_songcode::MAX_SLOT
            ),
            Self::DuplicateContent { first, second } => write!(
                f,
                "songs {first} and {second} are byte-identical; the same recording under two \
                 numbers is a catalog defect"
            ),
            Self::FileNotAudio { number, kind, path } => write!(
                f,
                "song {number} is {} but names {path:?}, which is not audio; the entry beside its \
                 audio is found from the audio's name and so cannot be found at all",
                kind.article_name()
            ),
            Self::UnknownEncoding { number, label } => write!(
                f,
                "song {number} declares the lyric encoding {label:?}, which names no encoding; the \
                 machine will fall back to detecting one, which is what this field was set to stop"
            ),
        }
    }
}

/// Whether a manifest `file` names one of the audio extensions an MP3+G song may use.
///
/// Spelled out here rather than calling `crate::is_audio_file` because a manifest entry is a plain
/// string with forward slashes, not a host path, and going through `Path` to ask about it invites
/// the platform's own opinions about what a file name is.
fn names_audio(file: &str) -> bool {
    file.rsplit_once('.').is_some_and(|(_, extension)| {
        crate::AUDIO_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    })
}

impl Manifest {
    /// A manifest for a new package with no songs yet.
    pub fn new(package: PackageMeta) -> Self {
        Self {
            // A package with no songs in it certainly has no video songs, and gains nothing by
            // declaring a version older readers would refuse. `PackageBuilder::write` raises this
            // if a video song is actually added.
            format: FORMAT_VERSION_MIDI_ONLY,
            package,
            songs: Vec::new(),
        }
    }

    /// Whether any song in this package keeps its media outside the archive.
    #[must_use]
    pub fn has_video(&self) -> bool {
        self.songs.iter().any(|song| song.kind.is_video())
    }

    /// Whether any song in this package is an MP3+G pair.
    #[must_use]
    pub fn has_cdg(&self) -> bool {
        self.songs.iter().any(|song| song.kind.is_cdg())
    }

    /// Whether any song in this package is an UltraStar song.
    #[must_use]
    pub fn has_ultrastar(&self) -> bool {
        self.songs.iter().any(|song| song.kind.is_ultrastar())
    }

    /// The format version this manifest should be written as.
    ///
    /// Chosen by content rather than fixed, so adopting video costs nothing to the packages that
    /// do not use it. See [`FORMAT_VERSION_MIDI_ONLY`].
    #[must_use]
    pub fn required_format(&self) -> u32 {
        if self.has_ultrastar() {
            FORMAT_VERSION_ULTRASTAR
        } else if self.has_cdg() || self.has_video() {
            FORMAT_VERSION_MEDIA
        } else {
            FORMAT_VERSION_MIDI_ONLY
        }
    }

    /// Finds a song by its number.
    pub fn song(&self, number: u32) -> Option<&SongEntry> {
        self.songs.iter().find(|song| song.number == number)
    }

    /// Everything wrong with this manifest.
    ///
    /// Returns all problems rather than the first, so a packager fixes a batch in one pass instead of
    /// rebuilding once per mistake.
    ///
    /// # What must never be added here
    ///
    /// **A rule about [`SongEntry::language`].** This function is called by `Package::open` as well
    /// as by `PackageBuilder::write`, so anything it refuses is a package that cannot be *opened* —
    /// and a package from a newer build may carry a code this one does not know. A
    /// `MissingLanguage` or `InvalidLanguage` variant would take every such package off a machine's
    /// catalog.
    ///
    /// Requiring a language is a *curation* rule and lives in the packagers: `km-pack build` and
    /// `km-package-builder`'s build both refuse a song without one, which is the same guarantee
    /// applied where it costs nothing. `a_package_with_a_language_this_build_does_not_know_still_opens`
    /// in `lib.rs` is the regression test.
    ///
    /// # The four considered exceptions
    ///
    /// [`ManifestProblem::CdgFileNotAudio`] was safe because no package in existence declared
    /// `kind: "cdg"` when it was added, so it could refuse nothing already in service.
    ///
    /// [`ManifestProblem::TooManySongs`] is the fourth, and it has the sharpest argument of the
    /// four. A package's songs are numbered 1 to [`km_songcode::MAX_SLOT`] and the machine adds the
    /// bank, so a song above that slot does not merely fail to dial — it **is** a number belonging
    /// to the next package's block. Such a package therefore names songs that are not its own
    /// wherever it is read, which is the definition of broken rather than untidy. It rests on the
    /// same two grounds as the one below: no `.kmpkg` has ever been released, and it states what the
    /// machine can do rather than expressing a preference about curation. That the cap also
    /// discourages importing a whole corpus is a product decision, argued in
    /// `docs/decisions/packaging.md`, and deliberately not the reason it is enforced *here*.
    ///
    /// [`ManifestProblem::NumberTooLarge`] is the third, and it is the one closest to the rule above
    /// — it refuses a package outright on the value of a field, which is exactly what a language
    /// rule would have done. What makes it allowed is that the reasoning behind the rule is about
    /// *packages in service* rather than about fields, and there are none: no `.kmpkg` has ever been
    /// released. It is also not a curation preference but a statement about what the machine can do,
    /// which is the other half of why it belongs here rather than in the packagers: a song numbered
    /// above [`km_songcode::MAX_SLOT`] would be dialled as another package's song, so a package carrying
    /// one is broken wherever it is read from and not merely untidy. The packagers refuse it too, and
    /// earlier, which is where a person can still do something about it.
    pub fn problems(&self) -> Vec<ManifestProblem> {
        let mut problems = Vec::new();

        // Here rather than in `Package::open`, which gets it in four places for one line: the
        // machine, the builder (which can never trip it, since `write` sets the format from the
        // content first), the curation tool's import, and **`km-pack check`** — which reads a
        // manifest unchecked precisely so it can diagnose rather than refuse, and is therefore the
        // most useful place for the message to appear.
        if !FORMAT_VERSIONS_READ.contains(&self.format) {
            problems.push(ManifestProblem::UnsupportedFormat(self.format));
        }
        if self.songs.is_empty() {
            problems.push(ManifestProblem::Empty);
        }
        // The two fields an installed file's name is built from. Checked here rather than where the
        // name is built, because a package whose id would land the file outside the packages folder
        // is a package to refuse rather than one to install under a repaired name — and `problems`
        // is the one gate every route in shares.
        for (field, value) in [("id", &self.package.id), ("version", &self.package.version)] {
            if !is_safe_name(value) {
                problems.push(ManifestProblem::UnsafeName {
                    field,
                    value: value.clone(),
                });
            }
        }
        // After the check above rather than instead of it. A generated id is a safe name by
        // construction, so this makes that one redundant for the id — and the redundant one is the
        // check that stops a package being *written outside the packages folder*, which a manifest
        // reaches before anybody has typed a password. Keeping both costs a line and means a future
        // change to either shape cannot quietly open that.
        if !PackageMeta::is_generated_id(&self.package.id) {
            problems.push(ManifestProblem::TypedId {
                id: self.package.id.clone(),
            });
        }
        // Two problems and not one, on purpose: this says the package is too big, and the
        // `NumberTooLarge` below names the song that will not fit. A package of a thousand songs
        // numbered 1 to 1000 trips both, and each half is separately actionable.
        if self.songs.len() > usize::from(km_songcode::MAX_SLOT) {
            problems.push(ManifestProblem::TooManySongs {
                count: self.songs.len(),
            });
        }

        let mut seen_numbers: Vec<u32> = Vec::with_capacity(self.songs.len());
        let mut seen_hashes: Vec<(&str, u32)> = Vec::new();

        for song in &self.songs {
            if song.number == 0 {
                problems.push(ManifestProblem::ZeroNumber);
            }
            if song.number > u32::from(km_songcode::MAX_SLOT) {
                problems.push(ManifestProblem::NumberTooLarge(song.number));
            }
            if seen_numbers.contains(&song.number) {
                problems.push(ManifestProblem::DuplicateNumber(song.number));
            } else {
                seen_numbers.push(song.number);
            }
            if song.title.trim().is_empty() {
                problems.push(ManifestProblem::MissingTitle(song.number));
            }
            if song.file.trim().is_empty() {
                problems.push(ManifestProblem::MissingFile(song.number));
            } else if !is_safe_path(&song.file) {
                problems.push(ManifestProblem::UnsafePath {
                    number: song.number,
                    path: song.file.clone(),
                });
            } else if (song.kind.is_cdg() || song.kind.is_ultrastar()) && !names_audio(&song.file) {
                // An MP3+G song's graphics are found by rule from its audio's name, so a `file`
                // that is not audio makes `graphics_path` return nothing at all. Caught here
                // because the alternative surfaces at singing time as "song 421 names no usable
                // media", which is the hardest kind of fault to place.
                //
                // Adding a rule here was safe in a way that a *language* rule would not have been
                // (see the note on this function): no package in existence declares `kind: "cdg"`,
                // so this cannot refuse to open anything already in service.
                problems.push(ManifestProblem::FileNotAudio {
                    number: song.number,
                    kind: song.kind,
                    path: song.file.clone(),
                });
            }
            // Checked here for the reason the whole of `problems` exists: playback is lenient about
            // this on purpose and therefore cannot be the thing that tells anybody. See
            // `ManifestProblem::UnknownEncoding`.
            //
            // Safe to add for `CdgFileNotAudio`'s reason read the other way: a package whose label
            // *is* known is unaffected, and one whose label is not was already being decoded by
            // detection — so this reports a fault that has always been there rather than refusing
            // anything that used to work.
            if let Some(label) = &song.lyric_encoding
                && !km_song::encoding::is_known_label(label)
            {
                problems.push(ManifestProblem::UnknownEncoding {
                    number: song.number,
                    label: label.clone(),
                });
            }
            if let Some(hash) = &song.content_hash {
                match seen_hashes.iter().find(|(seen, _)| *seen == hash.as_str()) {
                    Some((_, first)) => problems.push(ManifestProblem::DuplicateContent {
                        first: *first,
                        second: song.number,
                    }),
                    None => seen_hashes.push((hash, song.number)),
                }
            }
        }
        problems
    }

    /// Whether the manifest is usable.
    pub fn is_valid(&self) -> bool {
        self.problems().is_empty()
    }
}

/// Whether a relative path read out of a document is safe to follow.
///
/// A package and a curation database are both files somebody else may have written, so a path in
/// either is a claim rather than this project's own writing. `../../etc/passwd` must never be
/// followed, and `C:\Windows\System32\…` must not either — joining an absolute path onto a folder
/// discards the folder.
///
/// **It refuses rather than repairs.** A caller wanting the safe part of a bad path is a caller
/// that will open something other than what it was asked for, and a name reaching this that is not
/// a plain relative path is a document saying something untrue about itself. Refusing is also what
/// lets the same string stay an exact archive key: normalizing `midi/../song.kar` to `song.kar`
/// would produce a name matching no entry.
///
/// **A colon is refused wherever it falls**, which covers a drive letter, a UNC prefix and an NTFS
/// alternate data stream in one rule. A file name containing one cannot exist on Windows, so a
/// corpus meant to be carried between machines has none to lose.
///
/// What this does *not* cover is a name that becomes a file on disk: Windows device names such as
/// `CON` and `NUL`, and components whose trailing dots and spaces Win32 strips. Every caller uses
/// the result to *look something up* — an entry in an archive, a file below a corpus root — so
/// anything that starts creating files under a name a document chose needs a stricter rule than
/// this one.
pub fn is_safe_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    // Absolute paths, drive letters and UNC prefixes.
    if path.starts_with('/') || path.starts_with('\\') || path.contains(':') {
        return false;
    }
    // Any parent-directory component, in either separator style.
    path.split(['/', '\\'])
        .all(|part| !part.is_empty() && part != ".." && part != ".")
}

/// Whether a manifest field may be built into a file name.
///
/// **A stricter question than [`is_safe_path`] asks**, because the answer is used to *make* a file
/// rather than to look one up: [`PackageMeta::file_stem`] puts the id into the name a package is
/// installed under, and the curation tool puts the id and the version into the name it writes a
/// build to. A separator in either escapes the folder, and an absolute value replaces it outright.
///
/// **A name, not a shape.** `PackageMeta::id` stays the free string
/// [`PackageMeta::new_id`]'s own documentation promises — `karaoke-vol1` and `Músicas` are as
/// welcome as sixteen hex characters, so a hand-written spec and a re-imported package both keep
/// whatever they came with. What is refused is a value that is not a name at all.
///
/// Trailing dots and spaces are why the last test is not simply "non-empty": Win32 strips them from
/// a component, so `..` , `. ` and `...` all reach the filesystem as something other than what they
/// say, and the first of them is a climb.
///
/// Length is deliberately **not** a reason to refuse. A long id makes a long name and nothing worse,
/// and `file_stem` bounds the name it builds; refusing one would make a package that has always
/// installed stop opening.
#[must_use]
pub fn is_safe_name(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['/', '\\', ':'])
        && !value.chars().any(char::is_control)
        && value
            .trim_matches(|c: char| c == '.' || c.is_whitespace())
            .chars()
            .next()
            .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    fn meta() -> PackageMeta {
        PackageMeta {
            id: EXAMPLE_ID.to_owned(),
            name: "Test Package".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: None,
            created: None,
            volume: None,
        }
    }

    /// Bank 0 is the machine's own, so nothing derived may ever land on it.
    ///
    /// Walked over thousands of ids rather than asserted for one, because the property is about the
    /// *range* — `% (MAX_BANK + 1)` and `1 + % MAX_BANK` differ by exactly this, and one character
    /// is all it would take to put the reservation back inside the space it is meant to be outside.
    #[test]
    fn a_derived_bank_is_never_bank_zero() {
        for n in 0..5_000 {
            let bank = PackageMeta::suggested_bank(&format!("package-{n}"));
            assert!(
                (1..=km_songcode::MAX_BANK).contains(&bank),
                "id package-{n} derived bank {bank}"
            );
        }
    }

    /// Every bank is about equally likely, which is what the four-byte spread bought.
    ///
    /// The two-byte spread this replaced took 65,536 values modulo 9,999, so the first 3,940 banks
    /// came up seven times where the rest came up six — a 17% edge, and a bias towards the low banks
    /// is a bias towards *collisions*, since it crowds packages into a third of the range. The
    /// tolerance here is loose on purpose: 50,000 ids over 9,999 banks average five apiece, so
    /// ordinary randomness swings a single bucket wildly and only a systematic tilt shows up in
    /// thirds.
    #[test]
    fn derived_banks_are_spread_evenly_over_the_range() {
        let mut thirds = [0_u32; 3];
        let ids = 50_000;
        for n in 0..ids {
            let bank = PackageMeta::suggested_bank(&format!("spread-{n}"));
            let third = usize::from(bank - 1) * 3 / usize::from(km_songcode::MAX_BANK);
            thirds[third.min(2)] += 1;
        }
        let expected = f64::from(ids) / 3.0;
        for (index, count) in thirds.iter().enumerate() {
            let drift = (f64::from(*count) - expected).abs() / expected;
            assert!(
                drift < 0.05,
                "third {index} holds {count} of {ids} ids, {:.1}% off an even share",
                drift * 100.0
            );
        }
    }

    /// A generated id is unique in practice and is not a bank in disguise.
    ///
    /// Sixty-four bits of entropy, so a thousand of them colliding here would mean the generator is
    /// broken rather than that the test was unlucky — the real odds are about one in 340 billion.
    #[test]
    fn a_generated_id_is_sixteen_hex_characters_and_does_not_repeat() {
        let ids: std::collections::BTreeSet<String> =
            (0..1_000).map(|_| PackageMeta::new_id()).collect();
        assert_eq!(ids.len(), 1_000, "generated ids collided");
        for id in &ids {
            assert_eq!(id.len(), 16, "{id} is not sixteen characters");
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "{id} is not lowercase hex"
            );
        }
    }

    /// The same id gives the same bank, which is the whole point: a package's numbers must not
    /// depend on which machine installed it or in what order.
    #[test]
    fn a_derived_bank_depends_only_on_the_id() {
        assert_eq!(
            PackageMeta::suggested_bank("brasil-vol2"),
            PackageMeta::suggested_bank("brasil-vol2")
        );
        assert_ne!(
            PackageMeta::suggested_bank("brasil-vol1"),
            PackageMeta::suggested_bank("brasil-vol2"),
            "two ids this close should still land apart"
        );
    }

    #[test]
    fn a_packages_bank_is_the_one_its_id_implies_and_there_is_no_other_source() {
        let silent = meta();
        assert_eq!(
            silent.wanted_bank(),
            PackageMeta::suggested_bank(EXAMPLE_ID)
        );

        // The whole of what can change the answer is the id, so two packages alike in every other
        // way are banked apart and one renamed is banked the same.
        let mut renamed = meta();
        renamed.name = "Something else entirely".to_owned();
        renamed.version = "9.9".to_owned();
        assert_eq!(renamed.wanted_bank(), silent.wanted_bank());

        let mut other = meta();
        other.id = PackageMeta::new_id();
        assert_ne!(other.wanted_bank(), silent.wanted_bank());
    }

    /// A `bank:` key from an older manifest is read past, not refused.
    ///
    /// Nothing here sets `deny_unknown_fields`, and this is the case that matters: a package built
    /// against a format that carried the key must still open, and must be banked by its id like
    /// every other one rather than by whatever number is sitting in the file.
    #[test]
    fn a_manifest_carrying_a_bank_key_opens_and_is_still_banked_by_its_id() {
        let json = r#"{
            "format": 1,
            "package": { "id": "1f4a9c8e2b7d0356", "name": "Test", "version": "1.0", "bank": 3 },
            "songs": []
        }"#;
        let manifest: Manifest = serde_json::from_str(json).expect("the key is read past");
        assert_eq!(
            manifest.package.wanted_bank(),
            PackageMeta::suggested_bank(EXAMPLE_ID)
        );
    }

    /// The same examples `km-admin`'s copy of this rule names, so the two stay comparable by
    /// reading.
    #[test]
    fn a_name_folds_to_something_a_file_system_takes() {
        let slug = |name: &str| name_slug(name);

        assert_eq!(slug("Brasil Volume 1").as_deref(), Some("brasil-volume-1"));
        assert_eq!(slug("Praias do Sul").as_deref(), Some("praias-do-sul"));
        // Accents are dropped rather than transliterated: ugly and unambiguous beats a table of
        // somebody's opinions about other people's alphabets.
        assert_eq!(
            slug("Músicas Brasileiras").as_deref(),
            Some("msicas-brasileiras")
        );
        // One dash for any run of spaces and dashes.
        assert_eq!(slug("Rock & Roll").as_deref(), Some("rock-roll"));
        assert_eq!(slug("  Sertanejo  ").as_deref(), Some("sertanejo"));
        // Dots and underscores are legal in a file name and are kept.
        assert_eq!(slug("vol_2.1").as_deref(), Some("vol_2.1"));

        // A name holding no letter or digit cannot label anything.
        assert_eq!(slug("日本の歌"), None);
        assert_eq!(slug("---"), None);
        assert_eq!(slug(""), None);
    }

    /// A stem is the name for a person and the id for the machine.
    ///
    /// The id is the half that matters: it is what an install is keyed on, so two files of one
    /// package have to agree on a name however they were delivered.
    #[test]
    fn a_file_stem_carries_the_name_and_the_id() {
        let package = meta();
        assert_eq!(package.file_stem(), "test-package-1f4a9c8e2b7d0356");

        // A rebuild is the same package, so the same file — the version is deliberately absent.
        let mut rebuilt = meta();
        rebuilt.version = "1.0.9".to_owned();
        assert_eq!(rebuilt.file_stem(), package.file_stem());

        // Two packages that share a name do not share a file.
        let mut other = meta();
        other.id = PackageMeta::new_id();
        assert_ne!(other.file_stem(), package.file_stem());
    }

    /// A name that folds away to nothing still names a file, and a long one is cut to fit.
    ///
    /// The cut lands before the trim, so it cannot leave a stem ending in a dash — and the whole
    /// stem stays inside the bound `MAX_PATH` sets for a data directory that has already spent some
    /// of it.
    #[test]
    fn a_stem_is_bounded_and_a_nameless_package_is_still_named() {
        let mut nameless = meta();
        nameless.name = "日本の歌".to_owned();
        assert_eq!(nameless.file_stem(), EXAMPLE_ID);

        let mut long = meta();
        long.name = "Uma Coletânea Muito Longa de Canções ".repeat(10);
        long.id = PackageMeta::new_id();
        let stem = long.file_stem();
        assert!(
            stem.len() <= MAX_STEM_CHARS,
            "{stem} is {} long",
            stem.len()
        );
        assert!(stem.ends_with(&long.id), "{stem} must still carry the id");
        assert!(!stem.contains("--"), "{stem} holds an empty run");
    }

    fn song(number: u32) -> SongEntry {
        SongEntry {
            number,
            kind: SongKind::Midi,
            title: format!("Song {number}"),
            artist: Some("Someone".to_owned()),
            language: None,
            file: format!("midi/{number}.kar"),
            duration_ms: 200_000,
            lyric_encoding: None,
            default_transpose: 0,
            lyrics_hidden: false,
            fixes: Vec::new(),
            melody: None,
            melody_abstained: None,
            suitability: None,
            lyric_preview: Vec::new(),
            tags: Vec::new(),
            loudness: None,
            content_hash: None,
            edited: Vec::new(),
        }
    }

    #[test]
    fn a_well_formed_manifest_has_no_problems() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        manifest.songs.push(song(2));
        assert_eq!(manifest.problems(), vec![]);
        assert!(manifest.is_valid());
    }

    #[test]
    fn an_empty_package_is_a_problem() {
        let manifest = Manifest::new(meta());
        assert!(manifest.problems().contains(&ManifestProblem::Empty));
    }

    #[test]
    fn a_duplicate_number_is_reported() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(7));
        manifest.songs.push(song(7));
        assert!(
            manifest
                .problems()
                .contains(&ManifestProblem::DuplicateNumber(7))
        );
    }

    #[test]
    fn song_number_zero_is_rejected() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(0));
        assert!(manifest.problems().contains(&ManifestProblem::ZeroNumber));
    }

    #[test]
    fn a_number_above_the_last_slot_is_rejected() {
        // A slot of 1000 is not merely undialable — banked, it *is* the next package's first song,
        // so a package carrying one would name a song that is not its own.
        let mut manifest = Manifest::new(meta());
        manifest
            .songs
            .push(song(u32::from(km_songcode::MAX_SLOT) + 1));
        assert!(
            manifest
                .problems()
                .contains(&ManifestProblem::NumberTooLarge(1_000))
        );
    }

    #[test]
    fn the_last_slot_itself_is_fine() {
        // The boundary in the direction that matters: a bank is exactly this wide, so its last slot
        // must be a number a package may carry.
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(u32::from(km_songcode::MAX_SLOT)));
        assert_eq!(manifest.problems(), vec![]);
    }

    #[test]
    fn a_package_larger_than_a_bank_is_rejected() {
        // Two problems and not one: the package is too big, *and* the songs past the end are named.
        // A curated package is meant to be a volume somebody chose, not a folder somebody pointed at.
        let mut manifest = Manifest::new(meta());
        for number in 1..=u32::from(km_songcode::MAX_SLOT) + 1 {
            manifest.songs.push(song(number));
        }
        let problems = manifest.problems();
        assert!(problems.contains(&ManifestProblem::TooManySongs { count: 1_000 }));
        assert!(problems.contains(&ManifestProblem::NumberTooLarge(1_000)));
    }

    #[test]
    fn a_package_filling_a_bank_exactly_is_fine() {
        let mut manifest = Manifest::new(meta());
        for number in 1..=u32::from(km_songcode::MAX_SLOT) {
            manifest.songs.push(song(number));
        }
        assert_eq!(manifest.problems(), vec![]);
    }

    #[test]
    fn a_missing_title_is_reported() {
        let mut manifest = Manifest::new(meta());
        let mut entry = song(5);
        entry.title = "   ".to_owned();
        manifest.songs.push(entry);
        assert!(
            manifest
                .problems()
                .contains(&ManifestProblem::MissingTitle(5))
        );
    }

    #[test]
    fn identical_content_under_two_numbers_is_reported() {
        // The corpus this is built for is full of duplicates, so a package with the same recording
        // under two numbers is a mistake worth catching before it ships.
        let mut manifest = Manifest::new(meta());
        let mut first = song(10);
        first.content_hash = Some("abc123".to_owned());
        let mut second = song(20);
        second.content_hash = Some("abc123".to_owned());
        manifest.songs.push(first);
        manifest.songs.push(second);

        assert!(
            manifest
                .problems()
                .contains(&ManifestProblem::DuplicateContent {
                    first: 10,
                    second: 20
                })
        );
    }

    #[test]
    fn different_content_is_not_flagged_as_duplicate() {
        let mut manifest = Manifest::new(meta());
        let mut first = song(10);
        first.content_hash = Some("aaa".to_owned());
        let mut second = song(20);
        second.content_hash = Some("bbb".to_owned());
        manifest.songs.push(first);
        manifest.songs.push(second);
        assert_eq!(manifest.problems(), vec![]);
    }

    #[test]
    fn all_problems_are_reported_at_once_not_just_the_first() {
        let mut manifest = Manifest::new(meta());
        let mut bad = song(0);
        bad.title = String::new();
        bad.file = String::new();
        manifest.songs.push(bad);
        let problems = manifest.problems();
        assert!(
            problems.len() >= 3,
            "expected several problems, got {problems:?}"
        );
    }

    /// A declared encoding that names nothing is reported, because playback will not report it.
    ///
    /// **The asymmetry is the whole point.** `TextDecoder::resolve` deliberately falls through to
    /// detection on an unrecognised label — refusing to play a song over a manifest typo is the
    /// worse failure, and `km-song` has a test pinning that. So the only place this can be caught is
    /// here, before the package ships, and until now it was caught nowhere: `cp-1252` got
    /// detection's guess with no problem reported, no warning, and nothing in the log. The symptom
    /// was mojibake on exactly the songs somebody had set the field to fix.
    #[test]
    fn an_encoding_label_that_names_no_encoding_is_a_problem() {
        let mut manifest = Manifest::new(meta());
        let mut wrong = song(10);
        // A real typo, and the reason it is this one: the standard's label is `cp1252`, so a hyphen
        // that reads perfectly well to a person names nothing at all.
        wrong.lyric_encoding = Some("cp-1252".to_owned());
        manifest.songs.push(wrong);

        assert_eq!(
            manifest.problems(),
            vec![ManifestProblem::UnknownEncoding {
                number: 10,
                label: "cp-1252".to_owned(),
            }]
        );
    }

    /// ...and every spelling the standard does accept passes, including the ones that look wrong.
    #[test]
    fn the_encoding_labels_the_standard_accepts_are_not_problems() {
        for label in [
            "windows-1252",
            "cp1252",
            "Windows-1252",
            "Shift_JIS",
            "windows-949",
            "euc-kr",
        ] {
            let mut manifest = Manifest::new(meta());
            let mut entry = song(10);
            entry.lyric_encoding = Some(label.to_owned());
            manifest.songs.push(entry);
            assert_eq!(manifest.problems(), vec![], "{label} should be accepted");
        }
    }

    #[test]
    fn a_newer_format_is_refused_rather_than_misread() {
        let mut manifest = Manifest::new(meta());
        manifest.format = FORMAT_VERSION + 1;
        manifest.songs.push(song(1));
        assert!(
            manifest
                .problems()
                .contains(&ManifestProblem::UnsupportedFormat(FORMAT_VERSION + 1))
        );
    }

    #[test]
    fn path_traversal_is_rejected() {
        assert!(!is_safe_path("../outside.kar"));
        assert!(!is_safe_path("midi/../../etc/passwd"));
        assert!(!is_safe_path("/absolute/path.kar"));
        assert!(!is_safe_path("\\windows\\path.kar"));
        assert!(!is_safe_path("C:/Windows/System32/x.kar"));
        assert!(!is_safe_path(""));
        assert!(!is_safe_path("midi//double.kar"));
        assert!(!is_safe_path("./here.kar"));
    }

    #[test]
    fn ordinary_relative_paths_are_accepted() {
        assert!(is_safe_path("song.kar"));
        assert!(is_safe_path("midi/10234.kar"));
        assert!(is_safe_path("midi/nested/deep/song.mid"));
    }

    /// The id and the version become part of the file an installed package is written to.
    #[test]
    fn a_field_built_into_a_file_name_refuses_anything_that_is_not_one() {
        for value in [
            "../../../../evil",
            "..\\..\\evil",
            "/etc/cron.d/evil",
            "D:\\tunes\\evil",
            "C:/Windows/Temp/evil",
            "vol1/../../evil",
            "stream:name",
            "..",
            ".",
            "...",
            ". ",
            "",
            "bad\nname",
        ] {
            assert!(!is_safe_name(value), "{value:?} is not a file name");
        }
    }

    /// The other half: an id stays the free string it was promised to be.
    #[test]
    fn an_id_a_person_would_choose_is_still_a_name() {
        for value in [
            "1f4a9c8e2b7d0356",
            "karaoke-vol1",
            "brasil1",
            "Músicas",
            "vol 1",
            "1.0.0",
            "2.0.0-rc1",
            "name.with.dots",
        ] {
            assert!(is_safe_name(value), "{value:?} is a perfectly good name");
        }
    }

    #[test]
    fn an_id_that_would_leave_the_packages_folder_is_reported() {
        let mut manifest = Manifest::new(PackageMeta {
            id: "../../../../evil".to_owned(),
            ..meta()
        });
        manifest.songs.push(song(3));

        let problems = manifest.problems();
        assert!(
            problems
                .iter()
                .any(|problem| matches!(problem, ManifestProblem::UnsafeName { field: "id", .. })),
            "an id that is not a name must be a problem: {problems:?}"
        );
        assert!(!manifest.is_valid());
    }

    /// An id says what a person chose it after, and a generated one says nothing.
    ///
    /// The rejected values are the shapes a curator actually produces: a folder's own name, a volume
    /// number, a name with the right characters but the wrong length, and the right length in the
    /// wrong alphabet.
    #[test]
    fn a_typed_id_is_refused_and_a_generated_one_is_not() {
        for typed in [
            "festa-ana-2024",
            "karaoke-vol1",
            "1f4a9c8e2b7d035",
            "1f4a9c8e2b7d03567",
            "1F4A9C8E2B7D0356",
            "1f4a9c8e2b7d035g",
            "",
        ] {
            assert!(
                !PackageMeta::is_generated_id(typed),
                "{typed:?} is not an id a build would generate"
            );
        }
        assert!(PackageMeta::is_generated_id(EXAMPLE_ID));
        assert!(
            PackageMeta::is_generated_id(&PackageMeta::new_id()),
            "what `new_id` writes has to be what `is_generated_id` accepts, or nothing can be built"
        );

        let mut manifest = Manifest::new(PackageMeta {
            id: "festa-ana-2024".to_owned(),
            ..meta()
        });
        manifest.songs.push(song(3));

        let problems = manifest.problems();
        assert!(
            problems
                .iter()
                .any(|problem| matches!(problem, ManifestProblem::TypedId { .. })),
            "a typed id must be a problem: {problems:?}"
        );
        assert!(!manifest.is_valid());
        // The message is the only thing a person gets, and a package this refuses vanishes from the
        // catalog, so it has to name both the id and the remedy.
        let said = problems
            .iter()
            .find(|problem| matches!(problem, ManifestProblem::TypedId { .. }))
            .expect("the problem")
            .to_string();
        assert!(said.contains("festa-ana-2024"), "got {said}");
        assert!(said.contains("km-pack build"), "got {said}");
    }

    #[test]
    fn a_version_that_would_leave_the_packages_folder_is_reported() {
        let mut manifest = Manifest::new(PackageMeta {
            version: "../../evil".to_owned(),
            ..meta()
        });
        manifest.songs.push(song(3));

        assert!(
            manifest.problems().iter().any(|problem| matches!(
                problem,
                ManifestProblem::UnsafeName {
                    field: "version",
                    ..
                }
            )),
            "the curation tool builds a file name from the version too"
        );
    }

    /// The invariant rather than the diagnosis: whatever the id says, the stem is one component.
    ///
    /// Asserted for the diagnostic read's sake, which reaches `file_stem` on a manifest
    /// `Manifest::problems` would have refused.
    #[test]
    fn a_file_stem_is_always_a_single_component() {
        for id in [
            "../../../../evil",
            "..\\..\\evil",
            "D:\\tunes\\evil",
            "/etc/cron.d/evil",
            "..",
            "",
        ] {
            for name in ["A Real Name", "", "🎤"] {
                let subject = PackageMeta {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    ..meta()
                };
                let stem = subject.file_stem();
                assert!(
                    !stem.contains(['/', '\\', ':']),
                    "id {id:?} with name {name:?} produced a stem holding a separator: {stem:?}"
                );
                assert!(
                    Path::new(&stem).file_name() == Some(stem.as_ref()),
                    "id {id:?} with name {name:?} produced a stem that is not one component: \
                     {stem:?}"
                );
            }
        }
    }

    /// A package whose id is already a name keeps the name it has always installed under.
    #[test]
    fn a_safe_id_reaches_the_stem_exactly_as_it_stands() {
        let named = PackageMeta {
            id: "1f4a9c8e2b7d0356".to_owned(),
            name: "Brasil Volume 1".to_owned(),
            ..meta()
        };
        assert_eq!(named.file_stem(), "brasil-volume-1-1f4a9c8e2b7d0356");

        let unnamed = PackageMeta {
            id: "1f4a9c8e2b7d0356".to_owned(),
            name: "🎤".to_owned(),
            ..meta()
        };
        assert_eq!(unnamed.file_stem(), "1f4a9c8e2b7d0356");
    }

    #[test]
    fn an_unsafe_path_in_a_manifest_is_reported() {
        let mut manifest = Manifest::new(meta());
        let mut entry = song(3);
        entry.file = "../escape.kar".to_owned();
        manifest.songs.push(entry);
        assert!(matches!(
            manifest.problems().as_slice(),
            [ManifestProblem::UnsafePath { number: 3, .. }]
        ));
    }

    #[test]
    fn a_manifest_round_trips_through_json() {
        let mut manifest = Manifest::new(meta());
        let mut entry = song(10_234);
        entry.lyric_encoding = Some("windows-1252".to_owned());
        entry.melody = Some(MelodyRecord {
            channel: 5,
            confidence: 0.91,
            signals: vec!["track_name".to_owned(), "lyric_alignment".to_owned()],
        });
        entry.suitability = Some(SuitabilityRecord {
            value: 8,
            breakdown: BreakdownRecord {
                lyrics: 3,
                sync: 3,
                channels: 2,
                arrangement: 0,
            },
            warnings: vec![WarningRecord {
                code: "few_channels".to_owned(),
                message: "only 3 instrument channels".to_owned(),
            }],
        });
        manifest.songs.push(entry);

        let json = serde_json::to_string_pretty(&manifest).expect("serializes");
        let parsed: Manifest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn absent_optional_fields_are_omitted_from_the_json() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        let json = serde_json::to_string(&manifest).expect("serializes");
        // A manifest full of nulls is harder to read and larger for no benefit.
        assert!(!json.contains("\"melody\""), "got {json}");
        assert!(!json.contains("\"suitability\""));
        assert!(!json.contains("\"default_transpose\""));
        // The same argument as the tags below: a package whose songs all draw their words is
        // byte-identical to what a build predating this field wrote, which is why the format
        // version did not move for it.
        assert!(!json.contains("\"lyrics_hidden\""));
    }

    /// The three answers about a song's words, across a rebuild from source.
    ///
    /// **The `show` answer is the one worth the test.** A person saying the words *are* to be drawn
    /// leaves the field false — exactly what a song nobody has touched carries — so the marker is
    /// the only thing that tells a rebuild not to hand the song back to measurement.
    #[test]
    fn a_persons_answer_about_the_words_outlives_a_rebuild_in_both_directions() {
        // Somebody silenced a song measurement was content with.
        let mut hidden = song(1);
        hidden.lyrics_hidden = true;
        hidden.mark_edited(EditedField::LyricsHidden);

        let mut rebuilt = song(1);
        rebuilt.inherit_edits_from(&hidden);
        assert!(rebuilt.lyrics_hidden);
        assert!(rebuilt.is_edited(EditedField::LyricsHidden));

        // And somebody overruled measurement the other way, on a song a rebuild silences again.
        let mut shown = song(1);
        shown.lyrics_hidden = false;
        shown.mark_edited(EditedField::LyricsHidden);

        let mut remeasured = song(1);
        remeasured.lyrics_hidden = true;
        remeasured.inherit_edits_from(&shown);
        assert!(
            !remeasured.lyrics_hidden,
            "a rebuild must not put the words back out of sight over somebody's answer"
        );
        assert!(remeasured.is_edited(EditedField::LyricsHidden));

        // A song nobody has answered for takes whatever the rebuild measured.
        let untouched = song(1);
        let mut measured = song(1);
        measured.lyrics_hidden = true;
        measured.inherit_edits_from(&untouched);
        assert!(measured.lyrics_hidden);
        assert!(!measured.is_edited(EditedField::LyricsHidden));
    }

    /// The preview is the words on every surface the television is not, so it goes with them.
    #[test]
    fn inheriting_a_silenced_song_takes_its_preview_with_it() {
        let mut hidden = song(1);
        hidden.lyrics_hidden = true;
        hidden.mark_edited(EditedField::LyricsHidden);

        let mut rebuilt = song(1);
        rebuilt.lyric_preview = vec!["first line".to_owned(), "second line".to_owned()];
        rebuilt.inherit_edits_from(&hidden);
        assert!(rebuilt.lyric_preview.is_empty());
    }

    /// An untagged package is byte-identical to what a build predating tags wrote.
    ///
    /// This is the whole argument for not moving [`FORMAT_VERSION`] when tags arrived, so it is
    /// worth a test rather than a comment: `skip_serializing_if` means the key is absent, not empty,
    /// and a corpus of packages does not churn because a field was added that none of them uses.
    #[test]
    fn an_untagged_song_writes_no_tags_key_at_all() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        let json = serde_json::to_string(&manifest).expect("serializes");
        assert!(!json.contains("\"tags\""), "got {json}");

        // And a manifest that predates the field still opens, with no tags rather than an error.
        let parsed: Manifest = serde_json::from_str(&json).expect("deserializes");
        assert!(parsed.songs[0].tags.is_empty());
    }

    /// A volume names its set, a package that is not one writes no key, and neither is a problem.
    #[test]
    fn a_volume_round_trips_and_a_plain_package_writes_no_key() {
        let mut plain = Manifest::new(meta());
        plain.songs.push(song(1));
        let json = serde_json::to_string(&plain).expect("serializes");
        assert!(!json.contains("\"volume\""), "got {json}");

        let mut volume = plain.clone();
        volume.package.volume = Some(VolumeOf {
            of: EXAMPLE_ID.to_owned(),
            name: "Brasil".to_owned(),
            number: 2,
        });
        let json = serde_json::to_string(&volume).expect("serializes");
        let parsed: Manifest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed, volume);
        // Never a problem, whatever it says: a machine that refused a package over this key would
        // refuse a file it can install perfectly well.
        assert_eq!(parsed.problems(), plain.problems());
    }

    /// Tags survive a rebuild from source, which is the only reason they are in [`fields`].
    #[test]
    fn a_rebuild_inherits_the_tags_a_person_set() {
        let mut previous = song(1);
        previous.tags = vec!["rock".to_owned(), "brasil".to_owned()];
        previous.mark_edited(EditedField::Tags);

        // What re-analysis produces: the same song, read again, with nothing a person said.
        let mut rebuilt = song(1);
        rebuilt.inherit_edits_from(&previous);

        assert_eq!(rebuilt.tags, vec!["rock".to_owned(), "brasil".to_owned()]);
        assert!(rebuilt.is_edited(EditedField::Tags));
    }

    /// A manifest written before the singing suitability was withdrawn still opens: nothing here
    /// sets `deny_unknown_fields`, so the key is read past rather than refused.
    #[test]
    fn a_withdrawn_singing_suitability_is_ignored_rather_than_refused() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        let json = serde_json::to_string(&manifest).expect("serializes");
        let older = json.replace("\"number\":1", "\"number\":1,\"singing_suitability\":9");
        let parsed: Manifest = serde_json::from_str(&older).expect("deserializes");
        assert_eq!(parsed.songs[0].number, 1);
    }

    #[test]
    fn an_unknown_warning_code_still_parses() {
        // Forward compatibility: a package written by a later build must still open.
        let json = r#"{
            "format": 1,
            "package": {"id": "1f4a9c8e2b7d0356", "name": "X", "version": "1"},
            "songs": [{
                "number": 1, "title": "T", "file": "a.kar", "duration_ms": 1000,
                "suitability": {
                    "value": 5,
                    "breakdown": {"lyrics": 3, "sync": 2, "channels": 0, "arrangement": 0},
                    "warnings": [{"code": "something_invented_later", "message": "hello"}]
                }
            }]
        }"#;
        let manifest: Manifest = serde_json::from_str(json).expect("should parse");
        let warnings = &manifest.songs[0]
            .suitability
            .as_ref()
            .expect("suitability")
            .warnings;
        assert_eq!(warnings[0].code, "something_invented_later");
        // The unknown *warning code* is what this test is about, and the suitability beside it is
        // read normally — an unfamiliar code must not cost the number it sits next to.
        assert_eq!(
            manifest.songs[0]
                .suitability
                .as_ref()
                .expect("suitability")
                .value,
            5
        );
    }

    /// The suitability is `value`, and a package saying `score` is refused rather than defaulted.
    ///
    /// The word is refused because this project does not score *singers* and the two readings are
    /// one keystroke apart — see the field's own note for what that costs and why it is affordable
    /// here.
    ///
    /// **The refusal is the assertion, not an afterthought.** This field has no `serde(default)`
    /// precisely so that a pre-rename package fails to open: a default of 0 would let one through
    /// reading as the worst possible file, which sorts a good package to the bottom of every list
    /// and says nothing about why. Loud beats quiet, and the remedy is one `km-pack build`.
    #[test]
    fn a_package_saying_score_is_refused_rather_than_read_as_zero() {
        let old = r#"{"score": 7, "breakdown": {"lyrics": 3, "sync": 2, "channels": 2,
                      "arrangement": 0}}"#;
        let new = r#"{"value": 7, "breakdown": {"lyrics": 3, "sync": 2, "channels": 2,
                      "arrangement": 0}}"#;

        let refused = serde_json::from_str::<SuitabilityRecord>(old);
        assert!(refused.is_err(), "the old spelling must not be read");
        let message = refused.expect_err("refused").to_string();
        assert!(
            message.contains("value"),
            "the error must name the field that is missing: {message}"
        );

        let from_new: SuitabilityRecord = serde_json::from_str(new).expect("the new spelling");
        assert_eq!(from_new.value, 7);

        // And what this build writes is the new spelling only.
        let written = serde_json::to_string(&from_new).expect("serialize");
        assert!(written.contains("\"value\":7"), "got {written}");
        assert!(
            !written.contains("\"score\""),
            "the old spelling must not be written back out: {written}"
        );
    }

    /// The other direction from `a_build_that_never_heard_of_previews_still_reads_the_manifest`, and
    /// the one an owner actually meets: a `.kmpkg` on a shelf, written before this field existed,
    /// opening on a build that has it. `serde(default)` is the whole mechanism, and this is a real
    /// manifest with the key simply not present rather than present and empty.
    #[test]
    fn a_package_written_before_previews_existed_still_opens() {
        let json = r#"{
            "format": 1,
            "package": {"id": "1f4a9c8e2b7d0356", "name": "Volume 1", "version": "1.0.0"},
            "songs": [{
                "number": 500, "title": "Águas de Março", "artist": "Tom Jobim",
                "file": "midi/500.kar", "duration_ms": 200000
            }]
        }"#;
        let manifest: Manifest = serde_json::from_str(json).expect("an older package must open");
        assert_eq!(manifest.songs[0].title, "Águas de Março");
        assert!(
            manifest.songs[0].lyric_preview.is_empty(),
            "no preview, and no error either — only a rebuild puts words in it"
        );
    }

    #[test]
    fn a_hand_edited_field_is_recorded_once() {
        let mut entry = song(1);
        entry.mark_edited(EditedField::Title);
        entry.mark_edited(EditedField::Title);
        assert_eq!(entry.edited, vec![EditedField::Title]);
        assert!(entry.is_edited(EditedField::Title));
        assert!(!entry.is_edited(EditedField::Artist));
    }

    #[test]
    fn a_rebuild_inherits_hand_edited_fields_and_nothing_else() {
        // The corrected version from a previous build.
        let mut previous = song(10);
        previous.title = "Corcovado".to_owned();
        previous.artist = Some("Tom Jobim".to_owned());
        previous.language = Some("por".to_owned());
        previous.mark_edited(EditedField::Title);
        previous.mark_edited(EditedField::Artist);

        // What detection produced this time: the same bad guess as before.
        let mut fresh = song(10);
        fresh.title = "CORCOVAD".to_owned();
        fresh.artist = None;
        fresh.language = Some("eng".to_owned());
        fresh.duration_ms = 999_000;

        fresh.inherit_edits_from(&previous);

        assert_eq!(fresh.title, "Corcovado", "the correction must win");
        assert_eq!(fresh.artist.as_deref(), Some("Tom Jobim"));
        assert_eq!(
            fresh.language.as_deref(),
            Some("eng"),
            "language was not hand-edited, so the freshly detected value stands"
        );
        assert_eq!(
            fresh.duration_ms, 999_000,
            "re-measured facts are never inherited"
        );
        assert!(fresh.is_edited(EditedField::Title));
        assert!(fresh.is_edited(EditedField::Artist));
    }

    #[test]
    fn a_hand_assigned_number_is_inherited_so_a_rebuild_does_not_renumber_it() {
        // A number is how a singer asks for a song. If a person pinned one, a rebuild must not move
        // it just because the folder was reordered.
        let mut previous = song(1);
        previous.number = 4_242;
        previous.mark_edited(EditedField::Number);

        let mut fresh = song(1);
        fresh.number = 7;
        fresh.inherit_edits_from(&previous);
        assert_eq!(fresh.number, 4_242);
    }

    /// A field name from a later build reads as [`EditedField::Unknown`] rather than failing.
    ///
    /// **Through serde rather than by pushing the variant**, because the round trip is the claim:
    /// the enum is closed to this build's own call sites — which is the whole point of it — and open
    /// to the wire, and only deserialization exercises the second half. Pushing `Unknown` directly
    /// would test the `match` arm while assuming the thing that makes it reachable.
    #[test]
    fn an_unfamiliar_edited_field_name_reads_as_unknown() {
        let field: EditedField =
            serde_json::from_str("\"something_invented_later\"").expect("a later build's name");
        assert_eq!(field, EditedField::Unknown);

        // ...and the familiar ones still read as themselves, which is what says `serde(other)` did
        // not swallow the lot.
        for (wire, expected) in [
            ("\"title\"", EditedField::Title),
            ("\"lyric_encoding\"", EditedField::LyricEncoding),
            ("\"default_transpose\"", EditedField::DefaultTranspose),
            ("\"lyrics_hidden\"", EditedField::LyricsHidden),
            ("\"fixes\"", EditedField::Fixes),
        ] {
            let field: EditedField = serde_json::from_str(wire).expect("a known name");
            assert_eq!(field, expected, "{wire}");
            assert_eq!(
                serde_json::to_string(&field).expect("round trip"),
                wire,
                "the spelling has to survive a round trip, or a rebuild loses the marker"
            );
        }
    }

    #[test]
    fn an_unfamiliar_edited_field_name_is_skipped_rather_than_guessed_at() {
        // A package written by a later build may record a field this one knows nothing about.
        let mut previous = song(1);
        previous.edited.push(EditedField::Unknown);
        previous.title = "Should Not Move".to_owned();

        let mut fresh = song(1);
        let before = fresh.title.clone();
        fresh.inherit_edits_from(&previous);
        assert_eq!(fresh.title, before);
    }

    #[test]
    fn inheriting_from_an_unedited_entry_changes_nothing() {
        let previous = song(1);
        let mut fresh = song(1);
        fresh.title = "Detected".to_owned();
        fresh.inherit_edits_from(&previous);
        assert_eq!(fresh.title, "Detected");
        assert!(fresh.edited.is_empty());
    }

    #[test]
    fn the_edited_list_is_omitted_from_json_when_empty() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        let json = serde_json::to_string(&manifest).expect("serializes");
        assert!(!json.contains("\"edited\""), "got {json}");
    }

    #[test]
    fn edits_round_trip_through_json() {
        let mut manifest = Manifest::new(meta());
        let mut entry = song(1);
        entry.title = "Fixed By Hand".to_owned();
        entry.mark_edited(EditedField::Title);
        manifest.songs.push(entry);

        let json = serde_json::to_string(&manifest).expect("serializes");
        let parsed: Manifest = serde_json::from_str(&json).expect("deserializes");
        assert!(parsed.songs[0].is_edited(EditedField::Title));
    }

    #[test]
    fn a_song_with_no_words_writes_no_preview_key_at_all() {
        // The point of `skip_serializing_if`: a video song, an MP3+G song and a wordless MIDI file
        // must produce the same bytes they always did.
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        let json = serde_json::to_string(&manifest).expect("serializes");
        assert!(!json.contains("lyric_preview"), "{json}");
    }

    #[test]
    fn a_preview_survives_a_round_trip() {
        let mut manifest = Manifest::new(meta());
        let mut entry = song(1);
        entry.lyric_preview = vec!["Tempo perdido".to_owned(), "E que tudo mais".to_owned()];
        manifest.songs.push(entry);

        let json = serde_json::to_string(&manifest).expect("serializes");
        let parsed: Manifest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed.songs[0].lyric_preview.len(), 2);
        assert_eq!(parsed.songs[0].lyric_preview[0], "Tempo perdido");
    }

    /// The reason the format version did not move. A build that predates this field sees an unknown
    /// key and ignores it, because nothing here sets `deny_unknown_fields` — so a package written by
    /// this build still opens on every machine in service. Modeled by a struct with the field
    /// removed, which is exactly what an older `SongEntry` is.
    #[test]
    fn a_build_that_never_heard_of_previews_still_reads_the_manifest() {
        #[derive(serde::Deserialize)]
        struct OldEntry {
            number: u32,
            title: String,
        }
        #[derive(serde::Deserialize)]
        struct OldManifest {
            format: u32,
            songs: Vec<OldEntry>,
        }

        let mut manifest = Manifest::new(meta());
        let mut entry = song(7);
        entry.title = "Tempo Perdido".to_owned();
        entry.lyric_preview = vec!["Tempo perdido".to_owned()];
        manifest.songs.push(entry);

        let json = serde_json::to_string(&manifest).expect("serializes");
        let old: OldManifest =
            serde_json::from_str(&json).expect("an older build must still read it");
        assert_eq!(old.format, FORMAT_VERSION_MIDI_ONLY);
        assert_eq!(old.songs[0].number, 7);
        assert_eq!(old.songs[0].title, "Tempo Perdido");
    }

    /// The same three properties for `loudness`, which is the field levelling added.
    ///
    /// Grouped rather than split across three tests because they are one claim in three sentences:
    /// the key is absent when nothing measured, it survives a round trip when something did, and a
    /// build that never heard of it reads the manifest either way.
    #[test]
    fn a_loudness_is_absent_round_trips_and_does_not_break_an_older_build() {
        // Absent. A MIDI song is the reference and carries none, so its bytes are what they were.
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        let json = serde_json::to_string(&manifest).expect("serializes");
        assert!(!json.contains("loudness"), "{json}");
        // ...and the version is still the MIDI-only one, so an old machine opens it.
        assert_eq!(manifest.required_format(), FORMAT_VERSION_MIDI_ONLY);

        // Present, and back again unchanged.
        let mut manifest = Manifest::new(meta());
        let mut entry = song(2);
        entry.loudness = Some(LoudnessRecord {
            lufs: -5.5,
            peak_dbtp: 2.5,
        });
        manifest.songs.push(entry);
        let json = serde_json::to_string(&manifest).expect("serializes");
        let parsed: Manifest = serde_json::from_str(&json).expect("deserializes");
        let record = parsed.songs[0].loudness.expect("a measurement");
        assert!((record.lufs - (-5.5)).abs() < f32::EPSILON);
        assert!((record.peak_dbtp - 2.5).abs() < f32::EPSILON);

        // And an older build, modeled by a struct without the field, still reads the manifest.
        #[derive(serde::Deserialize)]
        struct OldEntry {
            number: u32,
        }
        #[derive(serde::Deserialize)]
        struct OldManifest {
            songs: Vec<OldEntry>,
        }
        let old: OldManifest =
            serde_json::from_str(&json).expect("an older build must still read it");
        assert_eq!(old.songs[0].number, 2);
    }

    /// A peak of exactly zero is left out, and reads back as zero rather than as a fault.
    ///
    /// `is_no_peak` is the one piece of this field with a stand-in value in it, so it is worth
    /// pinning: 0.0 dBTP is full scale, which no real file lands on to the bit, and a manifest that
    /// omits the key means nobody recorded a peak.
    #[test]
    fn a_peak_of_zero_is_not_written() {
        let mut manifest = Manifest::new(meta());
        let mut entry = song(1);
        entry.loudness = Some(LoudnessRecord {
            lufs: -14.0,
            peak_dbtp: 0.0,
        });
        manifest.songs.push(entry);

        let json = serde_json::to_string(&manifest).expect("serializes");
        assert!(json.contains("lufs"), "{json}");
        assert!(!json.contains("peak_dbtp"), "{json}");

        let parsed: Manifest = serde_json::from_str(&json).expect("deserializes");
        let record = parsed.songs[0].loudness.expect("a measurement");
        assert!((record.lufs - (-14.0)).abs() < f32::EPSILON);
        assert_eq!(record.peak_dbtp, 0.0);
    }

    /// A measurement is not a hand-edited field, so a rebuild re-derives it rather than inheriting.
    ///
    /// The `lyric_preview` side of the divergence rather than the `tags` side — see the field's own
    /// documentation for why that is wanted here and is a defect there.
    #[test]
    fn a_loudness_is_not_a_hand_edited_field() {
        let mut previous = song(1);
        previous.loudness = Some(LoudnessRecord {
            lufs: -9.0,
            peak_dbtp: 0.5,
        });
        previous.mark_edited(EditedField::Title);

        let mut rebuilt = song(1);
        rebuilt.inherit_edits_from(&previous);

        // The hand correction came across; the measurement did not, because measuring again is the
        // only thing that should produce one.
        assert!(rebuilt.is_edited(EditedField::Title));
        assert_eq!(rebuilt.loudness, None);
    }

    #[test]
    fn a_preview_is_not_a_hand_edited_field() {
        // It is detected from the bytes, like the duration and the suitability, so a rebuild re-derives it
        // and `inherit_edits_from` has nothing to carry across.
        let mut previous = song(1);
        previous.lyric_preview = vec!["stale".to_owned()];
        previous.mark_edited(EditedField::Title);
        previous.title = "Corrected By Hand".to_owned();

        let mut rebuilt = song(1);
        rebuilt.lyric_preview = vec!["fresh".to_owned()];
        rebuilt.inherit_edits_from(&previous);

        assert_eq!(rebuilt.title, "Corrected By Hand", "the title is inherited");
        assert_eq!(
            rebuilt.lyric_preview,
            vec!["fresh".to_owned()],
            "the preview is re-derived, never inherited"
        );
    }

    #[test]
    fn looking_up_a_song_by_number() {
        let mut manifest = Manifest::new(meta());
        manifest.songs.push(song(1));
        manifest.songs.push(song(99));
        assert_eq!(manifest.song(99).map(|s| s.number), Some(99));
        assert!(manifest.song(1234).is_none());
    }

    #[test]
    fn a_song_with_nothing_to_correct_writes_no_fixes_key() {
        let entry = song(1);
        let json = serde_json::to_string(&entry).expect("serializes");
        assert!(
            !json.contains("\"fixes\""),
            "an empty list is an absence, and a package full of them would carry the word once per song"
        );
    }

    #[test]
    fn a_hand_set_fix_list_survives_a_rebuild_and_a_detected_one_does_not() {
        // The distinction the `EditedField` exists for. A rebuild re-derives what the detector
        // found, so a defect a newer detector recognizes reaches an old package; a person's answer
        // is kept instead, because nothing in the bytes can be consulted to reach it again.
        let mut previous = song(1);
        previous.fixes = vec![km_fixes::Fix::MuteChannel { channel: 5 }];
        previous.mark_edited(EditedField::Fixes);

        let mut rebuilt = song(1);
        rebuilt.fixes = vec![km_fixes::Fix::IgnoreBankSelect { channel: 4 }];
        rebuilt.inherit_edits_from(&previous);
        assert_eq!(rebuilt.fixes, previous.fixes);
        assert!(rebuilt.is_edited(EditedField::Fixes));

        let mut untouched = song(1);
        untouched.fixes = vec![km_fixes::Fix::IgnoreBankSelect { channel: 4 }];
        untouched.inherit_edits_from(&song(1));
        assert_eq!(
            untouched.fixes,
            vec![km_fixes::Fix::IgnoreBankSelect { channel: 4 }]
        );
    }

    #[test]
    fn a_fix_named_by_a_later_build_opens_and_is_written_back_whole() {
        // A manifest is not `deny_unknown_fields`, and the same has to be true one level down: a
        // curator's list rewritten by an older build must not lose the fix it could not read.
        let mut entry = song(1);
        entry.fixes = vec![km_fixes::Fix::MuteChannel { channel: 3 }];
        let with_future = serde_json::to_string(&entry)
            .expect("serializes")
            .replace("mute_channel", "invented_later");

        let reopened: SongEntry = serde_json::from_str(&with_future).unwrap_or_else(|error| {
            panic!("a manifest naming a newer fix has to open: {error}");
        });
        assert_eq!(reopened.fixes.len(), 1);
        assert_eq!(km_fixes::resolve(&reopened.fixes).mute, [false; 16]);

        let rewritten = serde_json::to_string(&reopened).expect("serializes");
        assert!(rewritten.contains("invented_later"));
        assert!(rewritten.contains("\"channel\":3"));
    }
}
