//! The evening's singers.
//!
//! A singer is a *seat* that the machine issues, and the typed name is its label. A seat holds an
//! id, an optional name and a token. The token is what proves that a caller holds the seat, because
//! a name is typed and anybody can type one.
//!
//! Pure state, held beside the queue on the control thread. It draws no randomness and folds no
//! text: the caller hands in the new token and the name's comparison key. So every rule here is
//! testable with fixed values, and this crate keeps the two dependencies it has.
//!
//! **A seat is no account.** It holds no password and nothing that outlives the process, and the
//! machine keeps no list of people. It does let the machine tell two phones apart.

/// How many seats one evening may hold.
///
/// A cap exists so a phone at the room level cannot fill memory by registering singers. High
/// enough that a real night never reaches it.
pub const MAX_SEATS: usize = 1_000;

/// The longest name a seat may take, in characters.
///
/// The machine's screen draws the name in a column beside a title, so a paragraph is refused.
pub const MAX_SEAT_NAME_CHARS: usize = 40;

/// The longest device id a remote may send, in characters.
pub const MAX_DEVICE_ID_CHARS: usize = 64;

/// A singer's name, as typed and as compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatName {
    /// What people see: the name as typed, trimmed.
    pub shown: String,
    /// What two names are compared by: no case, no surrounding spaces and no accents.
    ///
    /// The caller folds it, so `Ana`, ` ana ` and `Aná` all arrive with one key.
    pub key: String,
}

/// One singer of the evening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    /// Opaque, stable for the evening. Never reused.
    pub id: u64,
    /// The name people see, when the singer gave one.
    pub name: Option<String>,
    /// The number of an unnamed seat, as in `Guest 4`.
    ///
    /// A label and no name: it tells unnamed seats apart on an operator's page and on the guest's
    /// own phone. A seat that takes a name later keeps the number and shows the name.
    pub guest: Option<u32>,
}

/// What the owner's settings ask of a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SeatRules {
    /// Whether every singer gives a name.
    pub name_required: bool,
    /// How many singers one device may hold, or `None` for no limit.
    pub max_per_device: Option<u32>,
}

/// A seat, with the token that now proves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    /// The seat.
    pub seat: Seat,
    /// The token the caller keeps. Any token the seat held before has ended.
    pub token: String,
}

/// Why a seat action was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SeatError {
    /// Another singer holds the name, and holds a song under it.
    #[error("another singer holds that name")]
    NameTaken,
    /// The owner requires a name and the caller gave none.
    #[error("this machine needs a name for every singer")]
    NameRequired,
    /// The name has nothing to compare by, or is too long.
    #[error("that name cannot be used")]
    NameUnusable,
    /// The device id is empty or too long.
    #[error("that device id cannot be used")]
    DeviceUnusable,
    /// The device already holds its one unnamed singer.
    #[error("this device already has a singer with no name")]
    UnnamedTaken,
    /// The device holds as many singers as the owner allows.
    #[error("this device has as many singers as the machine allows")]
    DeviceFull,
    /// The evening holds as many singers as it may.
    #[error("the machine has as many singers as it holds ({MAX_SEATS})")]
    Full,
    /// No seat has that id.
    #[error("no such singer")]
    NotFound,
    /// The token does not prove the seat.
    #[error("that token does not belong to this singer")]
    WrongToken,
}

/// A seat as the registry holds it.
#[derive(Debug, Clone)]
struct Held {
    seat: Seat,
    /// The folded name, for the unique-name rule.
    key: Option<String>,
    /// The device that registered it.
    device: String,
    token: String,
}

/// The evening's seats.
#[derive(Debug, Clone, Default)]
pub struct Seats {
    held: Vec<Held>,
    next_id: u64,
    next_guest: u32,
}

impl Seats {
    /// No seats.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many singers the evening holds.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Whether nobody has registered.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Every seat, in the order each registered.
    pub fn seats(&self) -> impl Iterator<Item = &Seat> {
        self.held.iter().map(|held| &held.seat)
    }

    /// One seat by id.
    pub fn get(&self, id: u64) -> Option<&Seat> {
        self.held
            .iter()
            .find(|held| held.seat.id == id)
            .map(|held| &held.seat)
    }

    /// Whether a token proves a seat.
    pub fn holds(&self, id: u64, token: &str) -> bool {
        self.held
            .iter()
            .any(|held| held.seat.id == id && same_token(&held.token, token))
    }

    /// Registers a singer for a device, and hands back the seat with its token.
    ///
    /// **A held name comes back to whoever asks, while its seat holds no song.** That is the way
    /// back for a phone that lost its token, and it needs nobody. The seat gets `token`, its old
    /// token ends, and the seat moves to this device. While the seat holds a song the name is
    /// refused, because a reclaim by name alone would hand one singer's songs to another.
    ///
    /// `holds_song` says whether a seat has a song queued or waiting.
    pub fn register(
        &mut self,
        device: &str,
        name: Option<SeatName>,
        token: String,
        rules: SeatRules,
        holds_song: impl Fn(u64) -> bool,
    ) -> Result<Issued, SeatError> {
        check_device(device)?;
        let Some(name) = name else {
            if rules.name_required {
                return Err(SeatError::NameRequired);
            }
            // One unnamed singer for each device, so people who share a tablet can tell their
            // songs apart.
            if self
                .held
                .iter()
                .any(|held| held.device == device && held.key.is_none())
            {
                return Err(SeatError::UnnamedTaken);
            }
            self.check_room(device, rules)?;
            self.next_guest += 1;
            let guest = Some(self.next_guest);
            return Ok(self.push(device, None, guest, token));
        };
        check_name(&name)?;

        if let Some(index) = self.index_of_key(&name.key) {
            let held = &self.held[index];
            if holds_song(held.seat.id) {
                return Err(SeatError::NameTaken);
            }
            if held.device != device {
                self.check_device_room(device, rules)?;
            }
            let held = &mut self.held[index];
            held.device = device.to_owned();
            held.token.clone_from(&token);
            held.seat.name = Some(name.shown);
            return Ok(Issued {
                seat: held.seat.clone(),
                token,
            });
        }

        self.check_room(device, rules)?;
        Ok(self.push(device, Some(name), None, token))
    }

    /// Gives a seat a name, or another name. The token is the proof.
    ///
    /// The seat keeps its id, its token and whatever it holds. The name must be free, and a rename
    /// never takes a name from another seat.
    pub fn rename(&mut self, id: u64, token: &str, name: SeatName) -> Result<Seat, SeatError> {
        check_name(&name)?;
        let index = self.index_of(id).ok_or(SeatError::NotFound)?;
        if !same_token(&self.held[index].token, token) {
            return Err(SeatError::WrongToken);
        }
        if self
            .index_of_key(&name.key)
            .is_some_and(|other| other != index)
        {
            return Err(SeatError::NameTaken);
        }
        let held = &mut self.held[index];
        held.key = Some(name.key);
        held.seat.name = Some(name.shown);
        Ok(held.seat.clone())
    }

    /// Gives a seat a new token, whatever it holds. The old token ends.
    ///
    /// No proof is asked here, so the caller's level is the check. This is the way back for a
    /// singer who lost a token while holding a song.
    pub fn reissue(&mut self, id: u64, token: String) -> Result<Issued, SeatError> {
        let index = self.index_of(id).ok_or(SeatError::NotFound)?;
        let held = &mut self.held[index];
        held.token.clone_from(&token);
        Ok(Issued {
            seat: held.seat.clone(),
            token,
        })
    }

    fn push(
        &mut self,
        device: &str,
        name: Option<SeatName>,
        guest: Option<u32>,
        token: String,
    ) -> Issued {
        let id = self.next_id;
        // Ids are never reused, for the reason a queue entry's are not.
        self.next_id += 1;
        let (shown, key) = match name {
            Some(name) => (Some(name.shown), Some(name.key)),
            None => (None, None),
        };
        let seat = Seat {
            id,
            name: shown,
            guest,
        };
        self.held.push(Held {
            seat: seat.clone(),
            key,
            device: device.to_owned(),
            token: token.clone(),
        });
        Issued { seat, token }
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.held.iter().position(|held| held.seat.id == id)
    }

    fn index_of_key(&self, key: &str) -> Option<usize> {
        self.held
            .iter()
            .position(|held| held.key.as_deref() == Some(key))
    }

    /// Whether one more seat fits, for the evening and for this device.
    fn check_room(&self, device: &str, rules: SeatRules) -> Result<(), SeatError> {
        if self.held.len() >= MAX_SEATS {
            return Err(SeatError::Full);
        }
        self.check_device_room(device, rules)
    }

    fn check_device_room(&self, device: &str, rules: SeatRules) -> Result<(), SeatError> {
        let Some(max) = rules.max_per_device else {
            return Ok(());
        };
        let on_device = self
            .held
            .iter()
            .filter(|held| held.device == device)
            .count();
        if on_device >= max as usize {
            return Err(SeatError::DeviceFull);
        }
        Ok(())
    }
}

fn check_device(device: &str) -> Result<(), SeatError> {
    if device.is_empty() || device.chars().count() > MAX_DEVICE_ID_CHARS {
        return Err(SeatError::DeviceUnusable);
    }
    Ok(())
}

fn check_name(name: &SeatName) -> Result<(), SeatError> {
    if name.key.is_empty() || name.shown.chars().count() > MAX_SEAT_NAME_CHARS {
        return Err(SeatError::NameUnusable);
    }
    Ok(())
}

/// Compares two tokens in time that depends on their lengths alone.
///
/// A token is random and long, so a timing difference gives a caller on the network little. The
/// comparison costs nothing to make even.
fn same_token(held: &str, offered: &str) -> bool {
    if held.len() != offered.len() {
        return false;
    }
    held.bytes()
        .zip(offered.bytes())
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: &str = "phone-a";
    const TABLET: &str = "tablet-b";

    fn name(shown: &str) -> Option<SeatName> {
        Some(SeatName {
            shown: shown.to_owned(),
            key: shown.to_lowercase(),
        })
    }

    fn free(_: u64) -> bool {
        false
    }

    fn register(seats: &mut Seats, device: &str, shown: Option<&str>, token: &str) -> Issued {
        seats
            .register(
                device,
                shown.and_then(name),
                token.to_owned(),
                SeatRules::default(),
                free,
            )
            .expect("the seat registers")
    }

    #[test]
    fn a_registered_singer_gets_a_seat_and_a_token_that_proves_it() {
        let mut seats = Seats::new();
        let ana = register(&mut seats, PHONE, Some("Ana"), "t1");

        assert_eq!(ana.seat.name.as_deref(), Some("Ana"));
        assert_eq!(ana.seat.guest, None);
        assert!(seats.holds(ana.seat.id, "t1"));
        assert!(!seats.holds(ana.seat.id, "t2"));
        assert!(!seats.holds(ana.seat.id, ""));
        assert_eq!(seats.len(), 1);
    }

    #[test]
    fn a_singer_needs_no_name_and_gets_a_number_instead() {
        let mut seats = Seats::new();
        let first = register(&mut seats, PHONE, None, "t1");
        let second = register(&mut seats, TABLET, None, "t2");

        assert_eq!((first.seat.name, first.seat.guest), (None, Some(1)));
        assert_eq!(second.seat.guest, Some(2));
        assert_ne!(first.seat.id, second.seat.id);
    }

    #[test]
    fn one_device_holds_one_unnamed_singer_at_most() {
        let mut seats = Seats::new();
        register(&mut seats, PHONE, None, "t1");

        let again = seats.register(PHONE, None, "t2".to_owned(), SeatRules::default(), free);
        assert_eq!(again, Err(SeatError::UnnamedTaken));
        // Every further singer on it gives a name.
        register(&mut seats, PHONE, Some("Beto"), "t3");
        assert_eq!(seats.len(), 2);
    }

    #[test]
    fn a_name_is_held_by_one_seat_however_it_is_typed() {
        let mut seats = Seats::new();
        let ana = register(&mut seats, PHONE, Some("Ana"), "t1");

        // The seat holds a song, so the name is refused to a second device.
        let taken = seats.register(
            TABLET,
            Some(SeatName {
                shown: " ANA ".trim().to_owned(),
                key: "ana".to_owned(),
            }),
            "t2".to_owned(),
            SeatRules::default(),
            |id| id == ana.seat.id,
        );
        assert_eq!(taken, Err(SeatError::NameTaken));
        assert!(
            seats.holds(ana.seat.id, "t1"),
            "the first token still holds"
        );
    }

    #[test]
    fn a_free_seat_comes_back_by_its_name_with_a_new_token() {
        let mut seats = Seats::new();
        let ana = register(&mut seats, PHONE, Some("Ana"), "t1");

        let back = register(&mut seats, TABLET, Some("ana"), "t2");

        assert_eq!(back.seat.id, ana.seat.id, "the same seat");
        assert_eq!(back.seat.name.as_deref(), Some("ana"));
        assert!(seats.holds(ana.seat.id, "t2"));
        assert!(!seats.holds(ana.seat.id, "t1"), "the old token ends");
        assert_eq!(seats.len(), 1);
    }

    #[test]
    fn a_rename_keeps_the_seat_and_needs_its_token() {
        let mut seats = Seats::new();
        let guest = register(&mut seats, PHONE, None, "t1");
        register(&mut seats, TABLET, Some("Ana"), "t2");
        let id = guest.seat.id;

        let wrong = seats.rename(id, "t2", name("Beto").expect("a name"));
        assert_eq!(wrong, Err(SeatError::WrongToken));

        let taken = seats.rename(id, "t1", name("Ana").expect("a name"));
        assert_eq!(taken, Err(SeatError::NameTaken), "even a free seat's name");

        let named = seats
            .rename(id, "t1", name("Beto").expect("a name"))
            .expect("a free name");
        assert_eq!(named.id, id);
        assert_eq!(named.name.as_deref(), Some("Beto"));
        assert!(seats.holds(id, "t1"), "the token stays");

        // The same name again, typed another way, is no clash with itself.
        let again = seats.rename(id, "t1", name("BETO").expect("a name"));
        assert_eq!(again.expect("its own name").name.as_deref(), Some("BETO"));

        // The device's unnamed place is free again.
        register(&mut seats, PHONE, None, "t3");
    }

    #[test]
    fn a_reissue_ends_the_old_token_whatever_the_seat_holds() {
        let mut seats = Seats::new();
        let ana = register(&mut seats, PHONE, Some("Ana"), "t1");

        let issued = seats
            .reissue(ana.seat.id, "t2".to_owned())
            .expect("the seat exists");
        assert_eq!(issued.seat, ana.seat);
        assert!(seats.holds(ana.seat.id, "t2"));
        assert!(!seats.holds(ana.seat.id, "t1"));

        assert_eq!(
            seats.reissue(9_999, "t3".to_owned()),
            Err(SeatError::NotFound)
        );
    }

    #[test]
    fn the_owner_may_require_a_name() {
        let mut seats = Seats::new();
        let rules = SeatRules {
            name_required: true,
            max_per_device: None,
        };
        let unnamed = seats.register(PHONE, None, "t1".to_owned(), rules, free);
        assert_eq!(unnamed, Err(SeatError::NameRequired));
        assert!(
            seats
                .register(PHONE, name("Ana"), "t1".to_owned(), rules, free)
                .is_ok()
        );
    }

    #[test]
    fn the_owner_may_cap_the_singers_on_one_device() {
        let mut seats = Seats::new();
        let rules = SeatRules {
            name_required: false,
            max_per_device: Some(2),
        };
        for (shown, token) in [("Ana", "t1"), ("Beto", "t2")] {
            seats
                .register(PHONE, name(shown), token.to_owned(), rules, free)
                .expect("under the cap");
        }
        let third = seats.register(PHONE, name("Caio"), "t3".to_owned(), rules, free);
        assert_eq!(third, Err(SeatError::DeviceFull));
        // Another device has its own count.
        let other = seats.register(TABLET, name("Caio"), "t3".to_owned(), rules, free);
        assert!(other.is_ok());
        // A full device cannot take a free seat's name from another device either.
        let moved = seats.register(PHONE, name("Caio"), "t4".to_owned(), rules, free);
        assert_eq!(moved, Err(SeatError::DeviceFull));
        // It can ask again for a name it holds.
        let own = seats.register(PHONE, name("Ana"), "t5".to_owned(), rules, free);
        assert!(own.is_ok());
    }

    #[test]
    fn the_evening_is_capped() {
        let mut seats = Seats::new();
        for number in 0..MAX_SEATS {
            let device = format!("device-{number}");
            register(&mut seats, &device, None, "t");
        }
        let one_more = seats.register("one-more", None, "t".to_owned(), SeatRules::default(), free);
        assert_eq!(one_more, Err(SeatError::Full));
    }

    #[test]
    fn a_name_or_a_device_that_cannot_be_used_is_refused() {
        let mut seats = Seats::new();
        let rules = SeatRules::default();
        let symbols = SeatName {
            shown: "!!!".to_owned(),
            key: String::new(),
        };
        assert_eq!(
            seats.register(PHONE, Some(symbols), "t".to_owned(), rules, free),
            Err(SeatError::NameUnusable)
        );
        let long = "a".repeat(MAX_SEAT_NAME_CHARS + 1);
        assert_eq!(
            seats.register(PHONE, name(&long), "t".to_owned(), rules, free),
            Err(SeatError::NameUnusable)
        );
        assert_eq!(
            seats.register("", name("Ana"), "t".to_owned(), rules, free),
            Err(SeatError::DeviceUnusable)
        );
        let device = "d".repeat(MAX_DEVICE_ID_CHARS + 1);
        assert_eq!(
            seats.register(&device, name("Ana"), "t".to_owned(), rules, free),
            Err(SeatError::DeviceUnusable)
        );
        assert!(seats.is_empty());
    }

    #[test]
    fn ids_are_never_reused_and_seats_list_in_order() {
        let mut seats = Seats::new();
        let ana = register(&mut seats, PHONE, Some("Ana"), "t1");
        let beto = register(&mut seats, TABLET, Some("Beto"), "t2");
        assert!(beto.seat.id > ana.seat.id);
        let names: Vec<_> = seats.seats().map(|seat| seat.name.clone()).collect();
        assert_eq!(names, vec![Some("Ana".to_owned()), Some("Beto".to_owned())]);
        assert_eq!(seats.get(beto.seat.id), Some(&beto.seat));
        assert_eq!(seats.get(9_999), None);
    }
}
