//! One favorites folder as a string of decimal digits, so it can be carried to another phone as a
//! QR code held up between two screens.
//!
//! Two phones each keep their own collection and, by design, cannot reach each other: every shell
//! here binds loopback, so nothing else on the WiFi can drive the karaoke machine, and that is
//! deliberate rather than incidental. A code held up between two screens needs no network, no
//! account and no pairing — which also suits the pair of devices this was written for, an Android
//! phone and an iPad, between which neither AirDrop nor Nearby Share exists.
//!
//! **In this crate rather than in `km-remote-core`, and the layering decides it**: the core
//! implements the traits this crate defines, so it depends on this one and never the other way
//! about. The handlers are the only caller, and a fourth crate under `crates/remote/` to hold one
//! file would be a crate nobody could place.
//!
//! # The format
//!
//! ```text
//! format(1) width(1) count(4) namelen(3) name(3 digits/byte) code×count(width each) check(2)
//! └─ "2"    └─ 1-9   └─ 0142  └─ 004     └─ 082111099107     └─ 0102300477…      └─ mod-97
//! ```
//!
//! The width of a code field is measured from the largest code and written into the header rather
//! than fixed, so nothing here assumes how a given device numbers its catalog.
//!
//! # Why digits
//!
//! The sibling project picked an all-digit payload because `rsc.io/qr` selects one encoding mode
//! for an entire string, so a single letter anywhere would push a payload of hundreds of characters
//! out of QR's numeric mode — 3⅓ bits a character — into byte mode's 8, and cost roughly two and a
//! half times the size. **That premise does not hold here.** [`qrcode::QrCode::new`] calls
//! `push_optimal_data`, which segments the payload and picks a mode per segment, so a name in byte
//! mode beside its codes in numeric mode is what the library would produce on its own.
//!
//! Three other things keep the format:
//!
//! * **The validation is defined over a digit string.** "Every character is a digit" is one total,
//!   cheap check that turns away a photograph of a URL, a Wi-Fi code or a ticket barcode before any
//!   structure is read at all, and the mod-97 check is the whole payload read as one number. Both
//!   would have to be redesigned for a mixed payload, and neither would come out better.
//! * **The saving and the cost are both confined to the name.** Three digits per UTF-8 byte is ten
//!   bits where byte mode spends eight, paid on a label of a few characters against a body of
//!   hundreds — a folder called `Rock` costs twelve digits, and the codes keep the dense encoding.
//!   Leaving the name out was tried first in that project and is exactly why the receiving device
//!   had to ask which folder it had just been handed.
//! * **It is a format shared with a sibling project**, which is the point rather than a curiosity:
//!   a folder can pass between a phone running that program and a phone running this one only if
//!   both write the identical string, and the format digit `2` is the same claim in both.
//!   `a_code_this_build_writes_is_the_code_the_sibling_project_writes` pins one byte for byte.
//!
//! # Why it is checked
//!
//! A camera that reads three quarters of a code, or a paste that clips the last line, must fail
//! rather than quietly import a truncated folder. The length is therefore **fully determined by the
//! header** and verified against it, and a mod-97 check closes the remaining case: a
//! plausible-looking string with the right shape and the wrong contents.

use km_songcode::SongCode;

/// The current format digit.
pub const FORMAT: u8 = b'2';

/// What the four-digit count field can express.
pub const MAX_SONGS: usize = 9_999;

/// What the three-digit name length can express, in bytes.
///
/// Far beyond the sixty characters a folder name is allowed, so trimming to it is a guard against
/// a caller rather than something a person meets.
pub const MAX_NAME: usize = 999;

/// What the one-digit width field can express.
///
/// Nine digits reaches 999,999,999, which is beyond any karaoke catalog and comfortably beyond
/// [`km_songcode::MAX_NUMBER`]'s seven. The guard is still real, because [`SongCode::new`]
/// deliberately does not range-check and a code read off another device's screen is not this
/// device's to vouch for.
const MAX_WIDTH: usize = 9;

/// Format, width, a four-digit count, a three-digit name length.
const HEADER_LEN: usize = 9;

/// The mod-97 check.
const CHECK_LEN: usize = 2;

/// What one code carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folder {
    /// What the folder is called on the sending device.
    ///
    /// **A label, not an instruction.** The receiving phone shows it so that a code from the wrong
    /// folder is visible before anything is written, and never uses it to decide where the songs
    /// go — being inside a folder already decided that.
    pub name: String,
    /// The song codes filed in it.
    pub songs: Vec<SongCode>,
}

/// Why a code could not be written, or read.
///
/// **Five distinguishable cases, because the remedy differs for each** — and a variant rather than a
/// sentence, for the reason [`crate::machine::codes`] exists at all: the page that prints this is
/// drawn in the viewer's language, and a codec has no viewer to ask. The `Display` text is a log
/// line and is never rendered; [`ShareError::message_key`] is what a page words it from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    /// Not one of ours, or damaged past recognizing.
    #[error("this does not look like a favorites code")]
    Format,
    /// The right shape, and the check digits disagree.
    #[error("this code is damaged")]
    Checksum,
    /// A folder with nothing in it, which is not worth a code.
    #[error("there are no songs to share")]
    Empty,
    /// More songs, or larger codes, than the format can carry.
    #[error("this folder is too large to share as one code")]
    TooLarge,
}

impl ShareError {
    /// The message id a page words this with.
    ///
    /// **The mapping lives beside the enum**, so the two cannot drift the way a `match` in another
    /// module could. It is the same bargain [`crate::words::refusal_key`] makes across a process
    /// boundary; there is no boundary here, so there is no wire code either — the variant is the
    /// stable name.
    #[must_use]
    pub fn message_key(self) -> &'static str {
        match self {
            Self::Format => "share-error-format",
            Self::Checksum => "share-error-damaged",
            Self::Empty => "share-error-empty",
            Self::TooLarge => "share-error-too-large",
        }
    }

    /// Every variant, for the test that checks each reaches a message that exists.
    ///
    /// What the compiler cannot check: a variant added above and not given an arm in
    /// [`message_key`](Self::message_key) *is* caught, because that match is exhaustive — but a
    /// variant whose message id is absent from a catalog is not, and that is a page drawing
    /// `⟦share-error-…⟧` at a party.
    pub const ALL: &'static [Self] = &[Self::Format, Self::Checksum, Self::Empty, Self::TooLarge];
}

/// Renders a folder as a code.
///
/// The codes are sorted and de-duplicated first, so two phones holding the same songs produce the
/// same string whatever order they happen to store them in. That is what makes a code comparable at
/// all, and it is why `the_order_and_the_duplicates_do_not_change_the_code` is worth having.
pub fn encode(folder: &Folder) -> Result<String, ShareError> {
    let songs = normalise(&folder.songs)?;

    // The largest code decides how wide every field is. Sorting has already put it last.
    let largest = songs[songs.len() - 1].number();
    let width = decimal_width(largest);
    if width > MAX_WIDTH {
        return Err(ShareError::TooLarge);
    }

    let name = trim_name(&folder.name);

    let mut out =
        String::with_capacity(HEADER_LEN + 3 * name.len() + songs.len() * width + CHECK_LEN);
    out.push(char::from(FORMAT));
    out.push(char::from(b'0' + width as u8));
    out.push_str(&format!("{:04}", songs.len()));
    out.push_str(&format!("{:03}", name.len()));
    for byte in name.as_bytes() {
        out.push_str(&format!("{byte:03}"));
    }
    for song in &songs {
        out.push_str(&format!("{:0width$}", song.number(), width = width));
    }

    let check = checksum(&out);
    out.push_str(&format!("{check:02}"));
    Ok(out)
}

/// Reads a code back, refusing anything it cannot fully account for.
///
/// Whitespace is ignored, because a code that has been through a messaging app or a text field
/// arrives wrapped across lines.
pub fn decode(code: &str) -> Result<Folder, ShareError> {
    let digits: String = code
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r'))
        .collect();
    let bytes = digits.as_bytes();

    if bytes.len() < HEADER_LEN + CHECK_LEN {
        return Err(ShareError::Format);
    }
    // The total, cheap rejection: anything that is not a digit is not one of ours, whatever else it
    // may be. This is what turns away a photograph of a URL or a ticket barcode before any of the
    // structure below is trusted.
    if !bytes.iter().all(u8::is_ascii_digit) {
        return Err(ShareError::Format);
    }
    match bytes[0] {
        FORMAT => {}
        _ => return Err(ShareError::Format),
    }

    let width = usize::from(bytes[1] - b'0');
    if !(1..=MAX_WIDTH).contains(&width) {
        return Err(ShareError::Format);
    }
    let count = field(&digits[2..6])?;
    if count == 0 {
        return Err(ShareError::Format);
    }
    let name_len = field(&digits[6..HEADER_LEN])?;

    // **The header fully determines the length**, which is what turns a truncated scan into an error
    // instead of a short folder. Checked before the checksum, because a length that cannot be right
    // makes every slice below a potential panic.
    if bytes.len() != HEADER_LEN + 3 * name_len + count * width + CHECK_LEN {
        return Err(ShareError::Format);
    }

    let split = bytes.len() - CHECK_LEN;
    let (body, check) = (&digits[..split], &digits[split..]);
    if check != format!("{:02}", checksum(body)) {
        return Err(ShareError::Checksum);
    }

    let mut name = Vec::with_capacity(name_len);
    for index in 0..name_len {
        let at = HEADER_LEN + index * 3;
        let byte = field(&digits[at..at + 3])?;
        if byte > 255 {
            return Err(ShareError::Format);
        }
        name.push(byte as u8);
    }

    let mut songs = Vec::with_capacity(count);
    for index in 0..count {
        let at = HEADER_LEN + 3 * name_len + index * width;
        // Nine digits at most, so this cannot overflow a `u32`; and deliberately **not**
        // range-checked against `km_songcode::MAX_NUMBER`. Refusing a larger number would break the
        // format's own promise that nothing here assumes how another device numbers its catalog,
        // and a code this catalog cannot hold falls out at the known-songs filter and is reported as
        // left out — which is the treatment `favdb::read_code` already argues for.
        songs.push(SongCode::new(field(&digits[at..at + width])? as u32));
    }

    // A name that does not survive the round trip is a label, not data, so it is dropped rather
    // than failing the whole code — the songs are the point.
    Ok(Folder {
        name: String::from_utf8(name).unwrap_or_default(),
        songs,
    })
}

/// Reads a fixed-width run of digits, which the caller has already proved are digits.
fn field(digits: &str) -> Result<usize, ShareError> {
    digits.parse().map_err(|_| ShareError::Format)
}

/// How many decimal digits a number takes.
fn decimal_width(number: u32) -> usize {
    if number == 0 {
        return 1;
    }
    number.ilog10() as usize + 1
}

/// Bounds the name to what the length field can express, cutting on a character boundary so a
/// multi-byte character is never split in half.
fn trim_name(name: &str) -> &str {
    let name = name.trim();
    if name.len() <= MAX_NAME {
        return name;
    }
    let mut cut = MAX_NAME;
    while cut > 0 && !name.is_char_boundary(cut) {
        cut -= 1;
    }
    &name[..cut]
}

/// Sorts, de-duplicates and range-checks the codes.
fn normalise(songs: &[SongCode]) -> Result<Vec<SongCode>, ShareError> {
    let mut out = songs.to_vec();
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(ShareError::Empty);
    }
    if out.len() > MAX_SONGS {
        return Err(ShareError::TooLarge);
    }
    Ok(out)
}

/// The digit string read as one long number, modulo 97.
///
/// Taken a digit at a time because the number is far too large for any integer type — a
/// three-hundred-song code is over fifteen hundred digits.
fn checksum(digits: &str) -> u32 {
    digits.bytes().fold(0u32, |carried, byte| {
        (carried * 10 + u32::from(byte - b'0')) % 97
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, songs: &[u32]) -> Folder {
        Folder {
            name: name.to_owned(),
            songs: songs.iter().copied().map(SongCode::new).collect(),
        }
    }

    #[test]
    fn a_folder_survives_the_round_trip() {
        let sent = folder("Rock", &[1001, 2005, 10_234]);
        let read = decode(&encode(&sent).expect("encode")).expect("decode");
        assert_eq!(read, sent);
    }

    /// The name is three digits a UTF-8 byte, so a multi-byte character is the case to check.
    #[test]
    fn a_name_with_accents_and_spaces_comes_back_as_it_went() {
        let sent = folder("Sertanejo Raiz — Anos 80", &[1001]);
        let read = decode(&encode(&sent).expect("encode")).expect("decode");
        assert_eq!(read.name, "Sertanejo Raiz — Anos 80");
    }

    /// The property the whole format exists for.
    #[test]
    fn every_character_of_a_code_is_a_digit() {
        let code = encode(&folder("Festa da Ana", &[1001, 2005])).expect("encode");
        assert!(
            code.bytes().all(|byte| byte.is_ascii_digit()),
            "a single non-digit would cost the numeric mode: {code}"
        );
    }

    /// Two phones holding the same songs must produce the same string, or a code is not comparable.
    #[test]
    fn the_order_and_the_duplicates_do_not_change_the_code() {
        let one = encode(&folder("Rock", &[2005, 1001, 1001])).expect("encode");
        let other = encode(&folder("Rock", &[1001, 2005])).expect("encode");
        assert_eq!(one, other);
    }

    #[test]
    fn the_width_field_follows_the_largest_code() {
        let narrow = encode(&folder("", &[1001])).expect("encode");
        let wide = encode(&folder("", &[1_234_567])).expect("encode");
        assert_eq!(&narrow[1..2], "4");
        assert_eq!(&wide[1..2], "7");
    }

    /// A camera that read three quarters of a code must fail rather than import a short folder.
    #[test]
    fn a_truncated_code_is_refused_rather_than_read_short() {
        let code = encode(&folder("Rock", &[1001, 2005, 3007])).expect("encode");
        for cut in 1..code.len() {
            assert!(
                decode(&code[..cut]).is_err(),
                "{cut} characters of a {} character code decoded",
                code.len()
            );
        }
    }

    /// The case the length check cannot see: the right shape, the wrong contents.
    #[test]
    fn one_wrong_digit_is_caught_by_the_check() {
        let code = encode(&folder("Rock", &[1001, 2005])).expect("encode");
        // A digit in the middle of the codes, so the length still agrees with the header.
        let at = code.len() - CHECK_LEN - 3;
        let mut damaged = code.clone();
        let replacement = if &code[at..at + 1] == "9" { "8" } else { "9" };
        damaged.replace_range(at..at + 1, replacement);
        assert_eq!(decode(&damaged), Err(ShareError::Checksum));
    }

    #[test]
    fn whitespace_a_messaging_app_added_is_ignored() {
        let code = encode(&folder("Rock", &[1001, 2005])).expect("encode");
        let wrapped = format!("  {}\r\n{}\t\n", &code[..10], &code[10..]);
        assert_eq!(decode(&wrapped).expect("decode").name, "Rock");
    }

    #[test]
    fn junk_is_refused_as_not_ours() {
        for not_ours in [
            "",
            "hello",
            "https://example.invalid/songs",
            "9400020040821110991071001200562",
            "000000000000",
        ] {
            assert!(
                matches!(
                    decode(not_ours),
                    Err(ShareError::Format | ShareError::Checksum)
                ),
                "{not_ours:?} was accepted"
            );
        }

        // A code whose format digit is not this build's is not one of ours either.
        let code = encode(&folder("Rock", &[1001])).expect("encode");
        assert_eq!(decode(&format!("1{}", &code[1..])), Err(ShareError::Format));
    }

    #[test]
    fn an_empty_folder_has_no_code() {
        assert_eq!(encode(&folder("Rock", &[])), Err(ShareError::Empty));
    }

    /// A folder with no name still travels; only the label is missing.
    #[test]
    fn a_folder_with_no_name_still_travels() {
        let read = decode(&encode(&folder("   ", &[1001, 2005])).expect("encode")).expect("decode");
        assert_eq!(read.name, "");
        assert_eq!(read.songs.len(), 2);
    }

    #[test]
    fn a_folder_past_the_count_field_is_refused() {
        let many: Vec<u32> = (1_000..1_000 + MAX_SONGS as u32 + 1).collect();
        assert_eq!(encode(&folder("Rock", &many)), Err(ShareError::TooLarge));
    }

    /// A code wider than the one-digit width field can express.
    #[test]
    fn a_code_too_large_for_the_width_field_is_refused() {
        assert_eq!(
            encode(&folder("Rock", &[1_000_000_000])),
            Err(ShareError::TooLarge)
        );
    }

    /// Cut on a character boundary, or the name is half a character and no longer UTF-8.
    #[test]
    fn an_overlong_name_is_cut_on_a_character_boundary() {
        // 'ç' is two bytes, so a 999-byte cut lands mid-character for some repeat count.
        let long = "ç".repeat(600);
        let read = decode(&encode(&folder(&long, &[1001])).expect("encode")).expect("decode");
        assert!(read.name.len() <= MAX_NAME);
        assert!(
            read.name.chars().all(|c| c == 'ç'),
            "the cut split a character: {:?}",
            read.name
        );
    }

    /// The songs are the point; a name that will not round-trip is dropped, not fatal.
    #[test]
    fn a_name_that_is_not_utf8_is_dropped_and_the_songs_are_not() {
        // Built by hand, because `encode` cannot produce one: a lone 0xFF byte where a name goes.
        let body = format!("2{}{:04}{:03}{:03}{:04}", 4, 1, 1, 0xFF, 1001);
        let code = format!("{body}{:02}", checksum(&body));
        let read = decode(&code).expect("the songs still arrive");
        assert_eq!(read.name, "", "an unreadable label is dropped");
        assert_eq!(read.songs, vec![SongCode::new(1001)]);
    }

    /// **The interop pin.** Verified against the sibling project's own `favsync.Encode` rather than
    /// re-derived from its documentation — a folder can only pass between the two programs if both
    /// write this exact string.
    #[test]
    fn a_code_this_build_writes_is_the_code_the_sibling_project_writes() {
        assert_eq!(
            encode(&folder("Rock", &[1001, 2005])).expect("encode"),
            "2400020040821110991071001200562"
        );
    }

    #[test]
    fn every_error_this_codec_raises_reaches_a_message_that_exists() {
        for error in ShareError::ALL {
            let key = error.message_key();
            for locale in km_locale::Locale::ALL {
                assert!(
                    crate::words::messages(*locale).keys().contains(key),
                    "no `{key}` in the {locale} catalog"
                );
            }
        }
    }
}
