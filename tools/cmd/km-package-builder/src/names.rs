//! Taking an artist out of the title it was filed inside.
//!
//! A corpus files one song's two fields in one string. `Bob Dylan-A Hard Rain's A-Gonna Fall` and
//! `Bob Marley - Natural Mystic 1982` are a title meta event and a file name from the same world, and
//! both leave the artist column empty while the artist is sitting in front of somebody in the title.
//! `km-pack` writes an MP3+G stem whole for the same reason: the stem is reliably `Artist - Title`
//! and nothing before this could say which half was which.
//!
//! **The first `-` is the seam, and only the first.** A title holds dashes of its own —
//! `A Hard Rain's A-Gonna Fall`, `Re-Recording` — so a rule taking the last dash, or every dash,
//! would cut inside the song's name. The artist comes first in every spelling of this shape the
//! corpus has, which is what makes the first one the seam rather than a guess.
//!
//! **Only the ASCII hyphen.** An en dash and an em dash are typography, and a corpus written by
//! sequencers on code pages does not have them where it has this defect; a rule reaching for all
//! three would cut a name that used one deliberately.
//!
//! **Both halves have to be there.** `-Foo` and `Foo-` hold no artist to take out, so they are left
//! exactly as they are rather than given an empty artist — which is a decision, and not one this
//! rule is entitled to make.

/// The artist and the title, in that order, or `None` for a name with nothing to take out of it.
///
/// `None` covers the three ways there is nothing here to decide: no hyphen, nothing before the first
/// one, or nothing after it. Each half is trimmed, so the spaced and the unspaced spelling of the
/// same defect answer the same way.
pub fn artist_and_title(value: &str) -> Option<(String, String)> {
    let (artist, title) = value.split_once('-')?;
    let artist = artist.trim();
    let title = title.trim();
    (!artist.is_empty() && !title.is_empty()).then(|| (artist.to_owned(), title.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two spellings the corpus writes, which differ only in space around the seam.
    #[test]
    fn the_first_dash_is_the_seam_spaced_or_not() {
        assert_eq!(
            artist_and_title("Bob Seger-Night moves"),
            Some(("Bob Seger".to_owned(), "Night moves".to_owned()))
        );
        assert_eq!(
            artist_and_title("Bob Marley - Natural Mystic 1982"),
            Some(("Bob Marley".to_owned(), "Natural Mystic 1982".to_owned()))
        );
    }

    /// The rule this one exists to protect: a song's own dashes are inside its name.
    #[test]
    fn a_later_dash_stays_in_the_title() {
        assert_eq!(
            artist_and_title("Bob Dylan-A Hard Rain's A-Gonna Fall"),
            Some((
                "Bob Dylan".to_owned(),
                "A Hard Rain's A-Gonna Fall".to_owned()
            ))
        );
    }

    /// A file name is a name like any other, and its underscores are not this rule's business.
    #[test]
    fn a_file_name_splits_and_keeps_its_underscores() {
        assert_eq!(
            artist_and_title("bob_marley-zimbabwe"),
            Some(("bob_marley".to_owned(), "zimbabwe".to_owned()))
        );
    }

    #[test]
    fn a_name_with_no_dash_holds_no_artist() {
        for name in ["Corcovado", "Garota de Ipanema", "", "   "] {
            assert_eq!(artist_and_title(name), None, "{name} has no seam");
        }
    }

    /// Half a split is not a split. An empty artist is a decision somebody else's button makes.
    #[test]
    fn a_name_missing_a_half_is_left_alone() {
        for name in ["-Zimbabwe", "Zimbabwe-", " - ", "-"] {
            assert_eq!(artist_and_title(name), None, "{name} is half a name");
        }
    }

    /// An en dash and an em dash are typography, and neither is the seam.
    #[test]
    fn only_the_ascii_hyphen_cuts() {
        assert_eq!(artist_and_title("Bob Marley – Zimbabwe"), None);
        assert_eq!(artist_and_title("Bob Marley — Zimbabwe"), None);
    }
}
