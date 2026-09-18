//! What the remote says, and how a refusal becomes a sentence.
//!
//! **The catalog is this crate's**, beside the markup it is rendered into, for the reason
//! `km-admin-pages` already copies its own asset hash rather than sharing one: what these words
//! belong to is *this* set of pages, and a rename that orphans a key should be caught in one crate.
//!
//! # Rendering a refusal
//!
//! [`refusal_key`] is the whole of the cross-process design. The machine sends a stable code; this
//! turns it into a message id; the page looks it up in the viewer's own language. **The machine's
//! sentence is never rendered** — it is English, written by a process that has no idea who is
//! reading, and putting it on the page is how an English sentence ends up inside a Portuguese one.
//! It stays in the log, where whoever is diagnosing wants it.
//!
//! A code this build does not know is not an error. It falls back to `error-unavailable`, the
//! sentence a refusal with no name of its own gets — so an offline remote talking to a newer
//! machine degrades to a true sentence rather than to a blank or a shrug.

use std::sync::OnceLock;

use km_locale::{Catalog, Locale};

/// The remote's own words, one catalog per locale.
///
/// `include_str!`, per `Bundling assets` — this crate ends up inside the machine, inside `km-remote`,
/// inside an APK and inside an iOS app, and a catalog that failed to travel would leave a page of
/// `⟦error-queue-full⟧`.
const CATALOGS: &[(Locale, &str)] = &[
    (Locale::English, include_str!("../i18n/en.ftl")),
    (
        Locale::BrazilianPortuguese,
        include_str!("../i18n/pt-BR.ftl"),
    ),
];

/// The remote's messages for one locale, parsed once.
///
/// `&'static` because that is what askama's values store needs to hold one — see
/// [`km_locale::filters`].
#[must_use]
pub fn messages(locale: Locale) -> &'static Catalog {
    static PARSED: OnceLock<Vec<(Locale, Catalog)>> = OnceLock::new();
    let parsed = PARSED.get_or_init(|| {
        CATALOGS
            .iter()
            .map(|(locale, source)| {
                let catalog = Catalog::new(*locale, source).unwrap_or_else(|errors| {
                    panic!("{locale} remote catalog: {}", errors.join("; "))
                });
                (*locale, catalog)
            })
            .collect()
    });
    parsed
        .iter()
        .find(|(candidate, _)| *candidate == locale)
        .map(|(_, catalog)| catalog)
        .expect("every locale has a remote catalog")
}

/// The generic refusal, and the fallback for a code this build does not know.
pub const ERROR_UNAVAILABLE: &str = "error-unavailable";
/// The queue is at its limit.
pub const ERROR_QUEUE_FULL: &str = "error-queue-full";
/// A password is needed.
pub const ERROR_UNAUTHORIZED: &str = "error-unauthorized";
/// The song, entry or folder is gone.
pub const ERROR_NOT_FOUND: &str = "error-not-found";
/// The command went out and was never confirmed. Not a fault; see `RemoteError::NotAcknowledged`.
pub const ERROR_NOT_ACKNOWLEDGED: &str = "error-not-acknowledged";
/// Anything else.
pub const ERROR_FAILED: &str = "error-failed";

/// The message id for one of the machine's stable refusal codes.
///
/// **The only place the two vocabularies meet.** Everything to the left of an arm is `km-api`'s or
/// `karaokemachine`'s; everything to the right is this crate's catalog. Keeping them apart is what
/// lets either be renamed without the other noticing — and what stops a page rendering a code it
/// half-recognizes.
#[must_use]
pub fn refusal_key(code: &str) -> &'static str {
    match family(code) {
        km_app_codes::NO_KEY => "error-no-key",
        km_app_codes::NO_TEMPO => "error-no-tempo",
        km_app_codes::NO_MELODY => "error-no-melody",
        km_app_codes::NO_MELODY_CHANNEL => "error-no-melody-channel",
        km_app_codes::NOTHING_PLAYING => "error-nothing-playing",
        km_app_codes::NOTHING_LOADED => "error-nothing-loaded",
        km_app_codes::NOTHING_QUEUED => "error-nothing-queued",
        km_app_codes::NO_SOUND => "error-no-sound",
        crate::machine::NO_FAVORITES => "error-no-favorites",
        crate::machine::NO_DEMO => "error-no-demo",
        // Including `unavailable` itself, which is what a machine sends for a refusal that has no
        // finer name — and what an older or newer one sends for a name this build cannot place.
        _ => ERROR_UNAVAILABLE,
    }
}

/// The message id for one of this device's own stable codes.
///
/// **The third place two vocabularies meet, and the nearest one.** [`refusal_key`] maps what the
/// *machine* said and [`how_key`] maps what the *locator* decided; this maps what `km-remote-core`
/// decided — a machine that has stopped answering, a folder name already taken, an address box
/// submitted empty. That crate is in this process, which is what made sending finished sentences
/// look harmless, and it has no catalog and no viewer to ask, which is what made it wrong.
///
/// `None` for a code this build has no message for. Every caller has its own right answer to that:
/// a toast falls back to [`ERROR_FAILED`], and the banner draws nothing rather than a wrong reason.
#[must_use]
pub fn code_key(code: &str) -> Option<&'static str> {
    match code {
        crate::machine::codes::NOT_ANSWERING => Some("offline-not-answering"),
        crate::machine::codes::NONE_FOUND => Some("offline-none-found"),
        crate::machine::codes::STREAM_CLOSED => Some("offline-stream-closed"),
        crate::machine::codes::LOOKING => Some("connection-looking"),
        crate::machine::codes::CONNECTING => Some("connection-connecting"),
        crate::machine::codes::ADDRESS_NEEDED => Some("machine-type-address"),
        crate::machine::codes::FOLDER_NEEDS_NAME => Some("folder-needs-name"),
        crate::machine::codes::FOLDER_NAME_TAKEN => Some("folder-name-taken"),
        crate::machine::codes::ONLY_FOLDER => Some("folder-only-one"),
        _ => None,
    }
}

/// The message id for how a machine's address was arrived at.
///
/// **The second place the two vocabularies meet**, and it is the [`refusal_key`] bargain one screen
/// over: `km-remote-core` sends a stable code, this turns it into a message id, and the page looks
/// it up in the viewer's own language. It used to send the English words and the card printed them,
/// so a Portuguese reader was told their machine was `remembered`.
///
/// A code this build does not know renders nothing rather than a wrong sentence — unlike a refusal,
/// which has a true generic to fall back to. There is no true generic for *how* an address was
/// arrived at, and the line it sits on reads perfectly well without it.
#[must_use]
pub fn how_key(code: &str) -> Option<&'static str> {
    match code {
        "asked-for" => Some("machine-how-asked-for"),
        "remembered" => Some("machine-how-remembered"),
        "machine-moved" => Some("machine-how-moved"),
        "adopted" => Some("machine-how-adopted"),
        "chosen" => Some("machine-how-chosen"),
        _ => None,
    }
}

/// Which kind of song a refusal is about, for the three messages that select on it.
///
/// The kind rides in the code — `no_key_video` — because it has to reach the page and the code is
/// the only stable thing on this wire; see the machine's `no_key_code`. A code with no kind, and a
/// code from a build that spells its kinds differently, both come back `other`, whose arm is a true
/// sentence about any song.
#[must_use]
pub fn refusal_kind(code: &str) -> &'static str {
    match code.rsplit_once('_').map(|(_, kind)| kind) {
        Some("midi") => "midi",
        Some("video") => "video",
        Some("cdg") => "cdg",
        Some("ultrastar") => "ultrastar",
        _ => "other",
    }
}

/// A code with its kind suffix removed, so one arm serves every kind of one refusal.
fn family(code: &str) -> &str {
    match code.rsplit_once('_') {
        Some((head, "midi" | "video" | "cdg" | "ultrastar")) => head,
        _ => code,
    }
}

/// The machine's refusal codes, spelled here rather than depended on.
///
/// **This crate must not take `karaokemachine`.** The dependency runs the other way — the machine
/// links these pages — so the codes are written out here, and the machine's
/// `every_refusal_this_machine_sends_is_a_sentence_the_remote_has` is what stops the two copies
/// drifting: it is the one place both spellings can be seen at once. Same bargain `LOG_TARGET`'s
/// four filter strings already make, with a test instead of a comment.
mod km_app_codes {
    /// A song kind with no key to change.
    pub const NO_KEY: &str = "no_key";
    /// A song kind with no tempo to change.
    pub const NO_TEMPO: &str = "no_tempo";
    /// A song kind with no guide melody at all.
    pub const NO_MELODY: &str = "no_melody";
    /// A MIDI song whose melody channel could not be identified.
    pub const NO_MELODY_CHANNEL: &str = "no_melody_channel";
    /// A transport command sent with nothing playing.
    pub const NOTHING_PLAYING: &str = "nothing_playing";
    /// A transport command sent with no song loaded.
    pub const NOTHING_LOADED: &str = "nothing_loaded";
    /// Play, with an empty machine and an empty queue.
    pub const NOTHING_QUEUED: &str = "nothing_queued";
    /// The machine cannot make sound at all.
    pub const NO_SOUND: &str = "no_sound";

    /// Every code above, for the test that compares them with the machine's own.
    ///
    /// Test-only: the machine's side of this pairing is asserted from `karaokemachine`, which is
    /// where both spellings can be seen at once.
    #[cfg(test)]
    pub const ALL: &[&str] = &[
        NO_KEY,
        NO_TEMPO,
        NO_MELODY,
        NO_MELODY_CHANNEL,
        NOTHING_PLAYING,
        NOTHING_LOADED,
        NOTHING_QUEUED,
        NO_SOUND,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::NO_CHOICE;

    #[test]
    fn every_catalog_parses() {
        for locale in Locale::ALL {
            assert!(!messages(*locale).keys().is_empty(), "{locale}");
        }
    }

    #[test]
    fn every_message_is_translated() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let missing = messages(*locale).missing_from(english);
            assert!(
                missing.is_empty(),
                "{locale} has not caught up: {missing:?}"
            );
        }
    }

    #[test]
    fn no_locale_invents_a_message_english_does_not_have() {
        let english = messages(Locale::English);
        for locale in Locale::ALL {
            let extra = english.missing_from(messages(*locale));
            assert!(
                extra.is_empty(),
                "{locale} has keys nothing asks for: {extra:?}"
            );
        }
    }

    #[test]
    fn every_refusal_code_reaches_a_message_that_exists() {
        // The gap a constant cannot close: the arm compiles and the catalog may still not have what
        // it names. Without this, pressing KEY+ on a video song shows `⟦error-no-key⟧`.
        let english = messages(Locale::English);
        for code in km_app_codes::ALL {
            let key = refusal_key(code);
            assert_ne!(
                key, ERROR_UNAVAILABLE,
                "`{code}` fell through to the generic"
            );
            assert!(english.keys().contains(key), "no message called `{key}`");
        }
        // And the same three with a kind on the end, which is the form the machine actually sends.
        for code in ["no_key_video", "no_tempo_cdg", "no_melody_midi"] {
            let key = refusal_key(code);
            assert_ne!(
                key, ERROR_UNAVAILABLE,
                "`{code}` fell through to the generic"
            );
            assert!(english.keys().contains(key), "no message called `{key}`");
        }
        for code in [crate::machine::NO_FAVORITES, crate::machine::NO_DEMO] {
            let key = refusal_key(code);
            assert_ne!(
                key, ERROR_UNAVAILABLE,
                "`{code}` fell through to the generic"
            );
            assert!(english.keys().contains(key), "no message called `{key}`");
        }
    }

    /// Every `|t` key in the markup is in the catalog.
    ///
    /// **The half a constant cannot reach.** A key in a template is a string literal askama passes
    /// through untouched, so a rename that misses one compiles and renders `⟦tab-songs⟧` on a
    /// singer's phone. Scanning the markup is crude and is the only thing that can see inside it —
    /// the same bargain `tools/dev/check-no-local-refs.sh` already makes.
    /// Every code this device raises about itself reaches a message that exists.
    ///
    /// The third of these, and the one that closes the last hole: `km-remote-core` decides a
    /// machine has stopped answering or a folder name is taken, and for as long as it sent the
    /// sentence rather than the code its US English landed in translated pages. `codes::ALL` is
    /// what the compiler cannot check — a `const` added there and not here would draw the generic
    /// failure with nothing to say it had happened.
    #[test]
    fn every_code_this_device_raises_reaches_a_message_that_exists() {
        for code in crate::machine::codes::ALL {
            let key = code_key(code).unwrap_or_else(|| panic!("`{code}` reaches no message id"));
            for locale in Locale::ALL {
                assert!(
                    messages(*locale).keys().contains(key),
                    "no `{key}` in the {locale} catalog"
                );
            }
        }
        assert_eq!(code_key("invented-by-a-later-version"), None);
    }

    /// Every way a machine can have been arrived at reaches a message that exists.
    ///
    /// [`every_refusal_code_reaches_a_message_that_exists`] one wire over. `Why::code` is an
    /// exhaustive match, so the compiler covers the producing side; what it cannot see is whether
    /// this crate has a message for each — and the card used to print `km-remote-core`'s English
    /// straight onto a Portuguese page rather than fall through to anything.
    #[test]
    fn every_way_a_machine_is_arrived_at_reaches_a_message_that_exists() {
        use km_api::discover::known::Why;
        for why in [
            Why::AskedFor,
            Why::Remembered,
            Why::MachineMoved,
            Why::Adopted,
            Why::Chosen,
        ] {
            let code = why.code();
            let key = how_key(code).unwrap_or_else(|| panic!("`{code}` reaches no message id"));
            for locale in Locale::ALL {
                assert!(
                    messages(*locale).keys().contains(key),
                    "no `{key}` in the {locale} catalog"
                );
            }
        }
        assert_eq!(how_key("invented-by-a-later-version"), None);
    }

    #[test]
    fn every_key_in_the_markup_is_in_the_catalog() {
        let english = messages(Locale::English);
        let mut seen = 0usize;
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates"))
            .expect("the templates directory")
        {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_none_or(|ext| ext != "html") {
                continue;
            }
            let markup = std::fs::read_to_string(&path).expect("read the template");
            for key in template_keys(&markup) {
                seen += 1;
                assert!(
                    english.keys().contains(&key),
                    "{} asks for `{key}`, which no catalog has",
                    path.display()
                );
            }
        }
        assert!(
            seen > 40,
            "the scanner found only {seen} keys; it is broken"
        );
    }

    #[test]
    fn no_message_is_left_unused() {
        // Read backwards: a key nothing looks up is a leftover from a rename, sitting there looking
        // like work for whoever translates next.
        let mut used: Vec<String> = ALL_MESSAGE_IDS.iter().map(|id| (*id).to_owned()).collect();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/templates"))
            .expect("the templates directory")
        {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_none_or(|ext| ext != "html") {
                continue;
            }
            let markup = std::fs::read_to_string(&path).expect("read the template");
            used.extend(template_keys(&markup));
        }
        for key in messages(Locale::English).keys() {
            assert!(
                used.contains(key),
                "`{key}` is in the catalog and nothing asks for it"
            );
        }
    }

    /// Every `"key"|t` in a template, however much whitespace is around the pipe.
    fn template_keys(markup: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for (index, _) in markup.match_indices('"') {
            let rest = &markup[index + 1..];
            let Some(end) = rest.find('"') else { continue };
            let candidate = &rest[..end];
            if candidate.is_empty()
                || !candidate
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                continue;
            }
            let after = rest[end + 1..].trim_start();
            if after.starts_with("|t }}") || after.starts_with("| t }}") {
                keys.push(candidate.to_owned());
            }
        }
        keys
    }

    /// Every id this crate's Rust looks up, for the orphan test.
    ///
    /// The markup's are scanned; these are not scannable, because they are constants.
    const ALL_MESSAGE_IDS: &[&str] = &[
        ERROR_UNAVAILABLE,
        ERROR_QUEUE_FULL,
        ERROR_UNAUTHORIZED,
        ERROR_NOT_FOUND,
        ERROR_NOT_ACKNOWLEDGED,
        ERROR_FAILED,
        "error-no-key",
        "error-no-tempo",
        "error-no-melody",
        "error-no-melody-channel",
        "error-nothing-playing",
        "error-nothing-loaded",
        "error-nothing-queued",
        "error-no-sound",
        "error-no-favorites",
        "error-no-demo",
        "language-picker",
        // Returned by a method and rendered with `|t`, so the markup carries no literal for the
        // scanner above to find — see `Mode::label_key` and `PlayerView::title_key`.
        "mode-songs",
        "mode-artists",
        "mode-favorites",
        "search-placeholder-songs",
        "search-placeholder-artists",
        "search-placeholder-favorites",
        "now-nothing-playing",
        "now-up-next",
        // Composed in `handlers.rs`, because each puts a value or a count into a sentence.
        "list-count-shown",
        "list-count-of",
        "empty-songs",
        "empty-search",
        "empty-initial",
        "empty-initial-search",
        "empty-folder",
        "empty-folder-initial",
        "empty-folder-search",
        "empty-artists",
        "empty-artists-search",
        "empty-artists-initial",
        "empty-artists-initial-search",
        "empty-folders",
        "empty-folders-search",
        "empty-hidden-packages",
        "count-songs",
        "song-unfavorite-confirm",
        "machine-songs-copied",
        // Keyed by `km-remote-core`'s stable code — see `how_key`.
        "machine-how-asked-for",
        "machine-how-remembered",
        "machine-how-moved",
        "machine-how-adopted",
        "machine-how-chosen",
        // Composed in `handlers.rs` — a toast, a badge, or a login page's one line.
        "queued-song",
        "playing-next-song",
        "playing-now-song",
        "queued-not-moved",
        "next-not-started",
        "demo-starting",
        "badge-queued",
        "badge-next-up",
        "badge-playing",
        "badge-sent",
        NO_CHOICE,
        "machine-type-address",
        "machine-offer-gone",
        "machine-found-named",
        "machine-found",
        "machine-scan-nothing",
        "machine-scan-already",
        "machine-scan-kept",
        "machine-more-answered",
        "singer-set",
        "singer-cleared",
        "packages-hidden-saved",
        "packages-all-shown",
        "folder-needs-name",
        "folder-made",
        "folder-renamed",
        "folder-deleted",
        "favorite-added",
        "favorite-removed",
        "favorite-removed-short",
        // Keyed by this device's own stable codes — see `code_key`.
        "offline-not-answering",
        "offline-none-found",
        "offline-stream-closed",
        "connection-looking",
        "connection-connecting",
        "folder-name-taken",
        "folder-only-one",
        "machine-now-using",
        "machine-copy-current",
        "machine-copy-imported",
        "machine-copy-not-answering",
        // Keyed by a codec variant rather than by a code off a wire — see `share::ShareError` and
        // `backup::BackupError`, which each carry the mapping beside the enum because there is no
        // process boundary here for a code to cross.
        "share-error-format",
        "share-error-damaged",
        "share-error-empty",
        "share-error-too-large",
        "backup-error-not-ours",
        "backup-error-damaged",
        "backup-error-empty",
        "backup-error-too-large",
        "backup-error-no-file",
        // Composed in `handlers.rs`, because each puts a folder's name or a count into a sentence.
        "share-title",
        "share-receive-sub",
        "share-one-way",
        "share-code-alt",
        "share-point-at",
        "share-confirm-add",
        "share-confirm-mismatch",
        "share-open-folder",
        "favorites-added",
        "favorites-already-here",
        "favorites-left-out",
        "favorites-not-here",
        "favorites-missing-package",
        "favorites-missing-recording",
        "count-folders",
        "backup-title",
        "backup-save-sub",
        "backup-restore-title",
        "backup-read-from",
        "backup-folders-created",
        "backup-unreadable",
        "backup-format-newer",
    ];

    #[test]
    fn a_code_this_build_does_not_know_falls_back_rather_than_failing() {
        // An offline remote may be talking to a machine of another version. What it must not do is
        // show the machine's English sentence inside a translated page.
        assert_eq!(refusal_key("something_invented_later"), ERROR_UNAVAILABLE);
        assert_eq!(refusal_key("unavailable"), ERROR_UNAVAILABLE);
        assert!(
            messages(Locale::BrazilianPortuguese)
                .keys()
                .contains(ERROR_UNAVAILABLE)
        );
    }
}
