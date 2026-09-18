//! The song queue.
//!
//! Pure state, held on the control thread. It knows song numbers and titles but nothing about how a
//! song is loaded or played, so the awkward parts — reordering, removing the entry that is currently
//! playing, adding the same song twice — are testable without an audio device or a catalog.
//!
//! Entries are identified by an opaque id rather than by position. A remote that deletes "entry 3"
//! after somebody else has already removed entry 1 would otherwise delete the wrong song, and on a
//! machine with several people queueing at once that happens constantly.

use std::collections::VecDeque;

use km_songcode::SongCode;

/// A queued song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    /// Opaque, stable for the entry's lifetime. Never reused.
    pub id: u64,
    /// The catalog code: the package's bank and the song's slot in it, as one number.
    pub number: SongCode,
    /// Title, so the display need not consult the catalog to show the queue.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// Who asked for it, when a remote says.
    pub singer: Option<String>,
}

impl QueueEntry {
    /// A one-line description for the display.
    pub fn label(&self) -> String {
        let mut label = format!("{}  {}", self.number, self.title);
        if let Some(artist) = &self.artist {
            label.push_str(" — ");
            label.push_str(artist);
        }
        if let Some(singer) = &self.singer {
            label.push_str(" (");
            label.push_str(singer);
            label.push(')');
        }
        label
    }
}

/// What a song should be queued as.
///
/// No `Default`: a song code has no sensible zero, since zero is not a song number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRequest {
    /// The catalog code: the package's bank and the song's slot in it, as one number.
    pub number: SongCode,
    /// Title.
    pub title: String,
    /// Performer.
    pub artist: Option<String>,
    /// Who asked.
    pub singer: Option<String>,
}

/// How many songs may wait.
///
/// A cap exists so a stuck remote or a bored guest cannot queue tens of thousands of songs and make
/// the display useless. High enough that a real party never reaches it.
pub const MAX_QUEUED: usize = 200;

/// The waiting songs, in order.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    entries: VecDeque<QueueEntry>,
    next_id: u64,
}

/// Why a song could not be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueueFull {
    /// The queue is at its limit.
    #[error("the queue is full ({MAX_QUEUED} songs)")]
    Full,
}

impl Queue {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many songs are waiting.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The waiting songs, in play order.
    pub fn entries(&self) -> impl Iterator<Item = &QueueEntry> {
        self.entries.iter()
    }

    /// The song that will play next.
    pub fn peek(&self) -> Option<&QueueEntry> {
        self.entries.front()
    }

    /// Adds a song to the back, returning its id.
    pub fn add(&mut self, request: QueueRequest) -> Result<u64, QueueFull> {
        if self.entries.len() >= MAX_QUEUED {
            return Err(QueueFull::Full);
        }
        let id = self.next_id;
        // Ids are never reused, so a stale reference from a remote fails to match rather than
        // hitting whatever took the old id's place.
        self.next_id += 1;
        self.entries.push_back(QueueEntry {
            id,
            number: request.number,
            title: request.title,
            artist: request.artist,
            singer: request.singer,
        });
        Ok(id)
    }

    /// Removes an entry by id. Returns it if it was there.
    pub fn remove(&mut self, id: u64) -> Option<QueueEntry> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        self.entries.remove(index)
    }

    /// Takes the next song off the front.
    pub fn pop(&mut self) -> Option<QueueEntry> {
        self.entries.pop_front()
    }

    /// Moves an entry to a position, clamped to the queue's bounds.
    ///
    /// Returns `false` when the id is unknown.
    pub fn move_to(&mut self, id: u64, index: usize) -> bool {
        let Some(from) = self.entries.iter().position(|entry| entry.id == id) else {
            return false;
        };
        let Some(entry) = self.entries.remove(from) else {
            return false;
        };
        // After removal the queue is one shorter, so the target has to be clamped against the new
        // length or moving to the end panics.
        let to = index.min(self.entries.len());
        self.entries.insert(to, entry);
        true
    }

    /// Empties the queue, returning what was removed.
    pub fn clear(&mut self) -> Vec<QueueEntry> {
        self.entries.drain(..).collect()
    }

    /// Position of an entry, for a remote showing "3rd in line".
    pub fn position_of(&self, id: u64) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == id)
    }

    /// Whether a song number is already waiting.
    ///
    /// Duplicates are allowed — two people may genuinely want the same song — but a remote may want
    /// to warn first.
    pub fn contains_number(&self, number: SongCode) -> bool {
        self.entries.iter().any(|entry| entry.number == number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(number: u32) -> QueueRequest {
        QueueRequest {
            number: SongCode::new(number),
            title: format!("Song {number}"),
            artist: Some("Someone".to_owned()),
            singer: None,
        }
    }

    #[test]
    fn songs_play_in_the_order_they_were_added() {
        let mut queue = Queue::new();
        queue.add(request(1)).expect("add");
        queue.add(request(2)).expect("add");
        queue.add(request(3)).expect("add");

        assert_eq!(queue.len(), 3);
        assert_eq!(queue.pop().expect("first").number, SongCode::new(1));
        assert_eq!(queue.pop().expect("second").number, SongCode::new(2));
        assert_eq!(queue.pop().expect("third").number, SongCode::new(3));
        assert!(queue.pop().is_none());
        assert!(queue.is_empty());
    }

    #[test]
    fn ids_are_never_reused() {
        // A remote holding a stale id must fail to match rather than deleting whatever took its
        // place. This is the whole reason entries are addressed by id.
        let mut queue = Queue::new();
        let first = queue.add(request(1)).expect("add");
        queue.remove(first);
        let second = queue.add(request(2)).expect("add");

        assert_ne!(first, second);
        assert!(queue.remove(first).is_none(), "the old id must not match");
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn removing_by_id_takes_the_right_entry_even_after_earlier_removals() {
        let mut queue = Queue::new();
        let a = queue.add(request(10)).expect("add");
        let b = queue.add(request(20)).expect("add");
        let c = queue.add(request(30)).expect("add");

        queue.remove(a);
        // `b` is now at position 0, but its id still identifies it.
        assert_eq!(queue.remove(b).expect("b").number, SongCode::new(20));
        assert_eq!(queue.peek().expect("remaining").id, c);
    }

    #[test]
    fn removing_an_unknown_id_does_nothing() {
        let mut queue = Queue::new();
        queue.add(request(1)).expect("add");
        assert!(queue.remove(9_999).is_none());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn an_entry_can_be_moved_to_the_front_to_play_next() {
        let mut queue = Queue::new();
        queue.add(request(1)).expect("add");
        queue.add(request(2)).expect("add");
        let third = queue.add(request(3)).expect("add");

        // `move_to(id, 0)` rather than a `promote` wrapper over it, which existed and was called by
        // nothing but this line.
        assert!(queue.move_to(third, 0));
        assert_eq!(queue.peek().expect("front").number, SongCode::new(3));
        // And nothing was lost.
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn moving_past_the_end_lands_at_the_end_rather_than_panicking() {
        let mut queue = Queue::new();
        let first = queue.add(request(1)).expect("add");
        queue.add(request(2)).expect("add");
        queue.add(request(3)).expect("add");

        assert!(queue.move_to(first, 999));
        let numbers: Vec<SongCode> = queue.entries().map(|entry| entry.number).collect();
        assert_eq!(
            numbers,
            vec![SongCode::new(2), SongCode::new(3), SongCode::new(1)]
        );
    }

    #[test]
    fn moving_to_its_own_position_changes_nothing() {
        let mut queue = Queue::new();
        queue.add(request(1)).expect("add");
        let second = queue.add(request(2)).expect("add");
        queue.add(request(3)).expect("add");

        assert!(queue.move_to(second, 1));
        let numbers: Vec<SongCode> = queue.entries().map(|entry| entry.number).collect();
        assert_eq!(
            numbers,
            vec![SongCode::new(1), SongCode::new(2), SongCode::new(3)]
        );
    }

    #[test]
    fn moving_an_unknown_id_reports_failure() {
        let mut queue = Queue::new();
        queue.add(request(1)).expect("add");
        assert!(!queue.move_to(1_234, 0));
    }

    #[test]
    fn the_queue_is_capped() {
        let mut queue = Queue::new();
        for number in 0..MAX_QUEUED {
            queue
                .add(request(number as u32))
                .expect("should fit under the cap");
        }
        assert_eq!(queue.add(request(9_999)), Err(QueueFull::Full));
        assert_eq!(queue.len(), MAX_QUEUED);
    }

    #[test]
    fn room_frees_up_after_a_song_plays() {
        let mut queue = Queue::new();
        for number in 0..MAX_QUEUED {
            queue.add(request(number as u32)).expect("add");
        }
        queue.pop();
        assert!(queue.add(request(9_999)).is_ok());
    }

    #[test]
    fn the_same_song_may_be_queued_twice() {
        // Two people wanting the same song is normal, not an error.
        let mut queue = Queue::new();
        queue.add(request(42)).expect("add");
        queue.add(request(42)).expect("add");
        assert_eq!(queue.len(), 2);
        assert!(queue.contains_number(SongCode::new(42)));
        assert!(!queue.contains_number(SongCode::new(43)));
    }

    #[test]
    fn positions_are_reported_for_a_remote_to_show() {
        let mut queue = Queue::new();
        let a = queue.add(request(1)).expect("add");
        let b = queue.add(request(2)).expect("add");
        assert_eq!(queue.position_of(a), Some(0));
        assert_eq!(queue.position_of(b), Some(1));
        queue.pop();
        assert_eq!(
            queue.position_of(b),
            Some(0),
            "positions shift as songs play"
        );
        assert_eq!(queue.position_of(a), None);
    }

    #[test]
    fn clearing_returns_what_was_removed() {
        let mut queue = Queue::new();
        queue.add(request(1)).expect("add");
        queue.add(request(2)).expect("add");
        let removed = queue.clear();
        assert_eq!(removed.len(), 2);
        assert!(queue.is_empty());
    }

    #[test]
    fn labels_include_the_details_that_are_present() {
        let mut queue = Queue::new();
        let id = queue
            .add(QueueRequest {
                number: SongCode::new(10_234),
                title: "Exagerado".to_owned(),
                artist: Some("Cazuza".to_owned()),
                singer: Some("Ana".to_owned()),
            })
            .expect("add");
        let entry = queue.entries().find(|e| e.id == id).expect("entry");
        let label = entry.label();
        assert!(label.contains("10234"));
        assert!(label.contains("Exagerado"));
        assert!(label.contains("Cazuza"));
        assert!(label.contains("Ana"));

        // And omit what is absent, rather than showing empty separators.
        let bare = QueueEntry {
            id: 0,
            number: SongCode::new(1),
            title: "Just A Title".to_owned(),
            artist: None,
            singer: None,
        };
        assert_eq!(bare.label(), "1  Just A Title");
    }

    #[test]
    fn an_empty_queue_reports_nothing_up_next() {
        let queue = Queue::new();
        assert!(queue.peek().is_none());
        assert_eq!(queue.entries().count(), 0);
    }
}
