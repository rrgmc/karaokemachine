//! Who may do what: five levels, and what decides a caller's.
//!
//! **Five levels, in a fixed order, and each one includes the ones below it.** A viewer sees the
//! catalog, the queue and the screen. A requester also registers a singer and asks for a song,
//! which the machine queues in that singer's turn. A queuer also adds a song with no singer, which
//! takes no turn, and turns a singer's knobs: key,
//! tempo, volume, melody and lyric offset. A controller also interrupts: skip, play now, the other
//! transport buttons, and moving or removing anybody's queue entry. The admin also reconfigures the
//! machine.
//!
//! **A caller's level is the higher of two things.** The room level is what everybody on the network
//! gets with no code at all, and it starts at [`Access::Request`]. A token is what a code or the admin
//! password buys, and it carries its own level. See [`crate::routes::required_access`] for what each
//! route needs.

use serde::{Deserialize, Serialize};

/// One of the five levels, lowest first.
///
/// `Ord` follows the declaration order, so `caller >= needed` is the whole check.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Sees the catalog, the queue and what is playing. Changes nothing.
    View,
    /// Registers a singer and names one, and asks for a song and takes one back.
    ///
    /// **The default room level.** Anybody in the room asks for a song, and the machine queues it
    /// in that singer's turn. So a new machine gives its singers turns with no setup. No code buys
    /// this level, because a room below it is a room where only somebody with a code adds songs.
    #[default]
    Request,
    /// Adds a song with no singer, which takes no turn, and turns a singer's knobs.
    Queue,
    /// Interrupts: skip, play now, the transport, and anybody's queue entry.
    Control,
    /// Reconfigures the machine. Only the admin password grants it, and a room level never does.
    Admin,
}

impl Access {
    /// Every level, lowest first.
    pub const ALL: [Self; 5] = [
        Self::View,
        Self::Request,
        Self::Queue,
        Self::Control,
        Self::Admin,
    ];

    /// The levels a room may be given. The admin level is never one of them.
    pub const ROOM: [Self; 4] = [Self::View, Self::Request, Self::Queue, Self::Control];

    /// The word for this level, as it appears in JSON, in a token and in `settings.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Request => "request",
            Self::Queue => "queue",
            Self::Control => "control",
            Self::Admin => "admin",
        }
    }

    /// The level a word names, if it names one.
    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == word)
    }

    /// Whether a room may be given this level.
    pub fn is_room_level(self) -> bool {
        Self::ROOM.contains(&self)
    }

    /// The room level a stored level is read as.
    ///
    /// A settings file is typed by hand, so it can name the admin level for the room. That is read
    /// as [`Self::Control`], the highest a room may hold.
    #[must_use]
    pub fn as_room(self) -> Self {
        self.min(Self::Control)
    }

    /// Whether a code grants this level. Only the queue and control levels have one.
    pub fn has_code(self) -> bool {
        matches!(self, Self::Queue | Self::Control)
    }
}

impl std::fmt::Display for Access {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_levels_are_ordered_lowest_first() {
        assert!(Access::View < Access::Request);
        assert!(Access::Request < Access::Queue);
        assert!(Access::Queue < Access::Control);
        assert!(Access::Control < Access::Admin);
    }

    /// A new machine gives its singers turns with no setup.
    #[test]
    fn a_fresh_room_can_ask_for_a_song_and_not_queue_one() {
        assert_eq!(Access::default(), Access::Request);
        assert!(Access::default() < Access::Queue);
    }

    #[test]
    fn every_level_round_trips_through_its_word() {
        for level in Access::ALL {
            assert_eq!(Access::from_word(level.as_str()), Some(level));
            let json = serde_json::to_string(&level).expect("a level serializes");
            assert_eq!(json, format!("\"{}\"", level.as_str()));
        }
        assert_eq!(Access::from_word("owner"), None);
    }

    #[test]
    fn a_room_is_never_given_the_admin_level() {
        assert!(!Access::ROOM.contains(&Access::Admin));
        assert!(!Access::Admin.is_room_level());
    }

    #[test]
    fn a_stored_level_is_read_as_one_a_room_may_hold() {
        assert_eq!(Access::Admin.as_room(), Access::Control);
        for level in Access::ALL {
            assert!(level.as_room().is_room_level());
        }
        for level in Access::ROOM {
            assert_eq!(level.as_room(), level);
        }
    }

    /// A room may be given the request level, and no code buys it.
    #[test]
    fn the_request_level_is_a_rooms_and_never_a_codes() {
        assert!(Access::Request.is_room_level());
        assert!(!Access::Request.has_code());
    }
}
