//! What the tests in this crate need and the remote does not.
//!
//! Every test here holds a SQLite file in a folder of its own. [`Scratch`] is that folder, and
//! `km-testkit` holds it because other crates need the same one. [`StubLocator`] stands in for the
//! network, for every test about what the remote does with what it finds there.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use km_api::discover::Sighting;

use crate::find::Locator;

pub use km_testkit::Scratch;

/// A locator that answers whatever it was told to, and counts how often it was asked.
///
/// The count is the interesting half: what `find::locate` promises is not only that it finds a
/// machine but that it does **not** go looking when it already has an answer, and only a counter
/// can tell the two apart.
pub struct StubLocator {
    answer: Vec<Sighting>,
    asked: AtomicUsize,
    wanted: Mutex<Option<String>>,
}

impl StubLocator {
    /// One machine with an address and no identity, or nothing at all.
    pub fn new(answer: Option<&str>) -> Self {
        Self::seeing(answer.into_iter().map(|url| sighting(None, url)).collect())
    }

    /// Whatever is on the network, identities and all.
    pub fn seeing(answer: Vec<Sighting>) -> Self {
        Self {
            answer,
            asked: AtomicUsize::new(0),
            wanted: Mutex::new(None),
        }
    }

    /// How many times something looked.
    pub fn asked(&self) -> usize {
        self.asked.load(Ordering::Relaxed)
    }

    /// The machine the last hunt asked for, if any.
    pub fn hunting(&self) -> Option<String> {
        self.wanted.lock().expect("not poisoned").clone()
    }
}

impl Locator for StubLocator {
    fn look(&self) -> Vec<Sighting> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.answer.clone()
    }

    fn hunt(&self, id: Option<&str>) {
        *self.wanted.lock().expect("not poisoned") = id.map(str::to_owned);
    }
}

/// A machine seen on the network, called what every test here calls it.
pub fn sighting(id: Option<&str>, url: &str) -> Sighting {
    Sighting {
        name: "Living Room".to_owned(),
        id: id.map(str::to_owned),
        url: url.to_owned(),
    }
}
