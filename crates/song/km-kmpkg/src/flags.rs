//! The flags word a package's header carries.
//!
//! **A flag is a fact about the file as a whole, readable before the manifest is.** It sits in the
//! container header at bytes 10..14, little-endian, so it can be read without parsing the manifest.
//!
//! **An unknown bit is kept, not refused.** The container version already refuses a layout this
//! build cannot read. A flag is information about a package this build can read, so refusing one it
//! does not recognize would make every later flag break every earlier machine. Every store therefore
//! keeps the whole word, and [`PackageFlags::names`] reports the bits this build knows.
//!
//! See `A package's header carries flags, and an unknown one is kept` in
//! `docs/decisions/packaging.md`.

/// The flags word in a package's header.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PackageFlags(u32);

impl PackageFlags {
    /// No flag set.
    pub const NONE: Self = Self(0);

    /// Built straight from a folder, with nobody reviewing its titles, duplicates or numbers.
    ///
    /// Every surface that lists packages shows it, except the television. See `An uncurated package
    /// says so everywhere but the television` in `docs/decisions/packaging.md`.
    pub const UNCURATED: Self = Self(1 << 0);

    /// Every bit this build has a name for, with that name.
    ///
    /// **The one place a bit is mapped to a word.** The API, the admin pages, the remotes and
    /// `km-pack` all read it, so a new flag is a new line here and a new label in each catalog.
    const KNOWN: [(Self, &'static str); 1] = [(Self::UNCURATED, "uncurated")];

    /// The word exactly as the header holds it, unknown bits included.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The word, unknown bits included.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every bit of `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// This word with every bit of `other` also set.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether no bit is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the package was built straight from a folder.
    #[must_use]
    pub const fn is_uncurated(self) -> bool {
        self.contains(Self::UNCURATED)
    }

    /// The names of the set bits this build knows, in bit order.
    ///
    /// A bit with no name here is left out rather than invented, and [`Self::bits`] still carries it.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::KNOWN
            .into_iter()
            .filter(move |(flag, _)| self.contains(*flag))
            .map(|(_, name)| name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_with_an_unknown_bit_keeps_it_and_names_only_what_it_knows() {
        let flags = PackageFlags::from_bits(0b101);
        assert_eq!(flags.bits(), 0b101);
        assert!(flags.is_uncurated());
        assert_eq!(flags.names().collect::<Vec<_>>(), ["uncurated"]);
    }

    #[test]
    fn no_flag_names_nothing() {
        assert!(PackageFlags::NONE.is_empty());
        assert!(!PackageFlags::NONE.is_uncurated());
        assert_eq!(PackageFlags::NONE.names().count(), 0);
    }
}
