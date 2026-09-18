//! The General MIDI banks this project knows how to fetch, compiled in from the table.
//!
//! The Rust half of `crates/machine/km-banks/data/soundfont-banks.conf`; the shell half is
//! `tools/setup/soundfont-banks.sh`. **One definition, three readers** — the same bargain
//! `tools/setup/features.sh` makes, and load-bearing here in a way it is not there: a second copy of
//! a digest is a download that verifies against the wrong number.
//!
//! **The third reader is why this is a crate of its own.** `tools/cmd/assets/km-admin` fetches a
//! bank as well, on a desktop, for a machine that may have no internet — and it cannot depend on
//! `karaokemachine`, which links SDL3, nor learn a URL from the API, because
//! `km_api::dto::SoundFontOfferDto` carries a bank's size and license and deliberately not its
//! download address. So the table lives where both readers can reach it, and the machine's own
//! `banks` module is a re-export.
//!
//! `include_str!` rather than a file read at run time, deliberately. This is a catalog of what
//! *could* be fetched, not a description of the machine it is running on: it has to be the same on a
//! `.deb`, inside a signed macOS bundle and unpacked from an APK, none of which have a writable
//! place to keep it and two of which are read-only. It is about 42 KB — it was 9 KB while the table
//! held eleven banks, and the second survey took it to sixty-three.
//!
//! **Nine of those are offered and the rest are here to be tested against**, which is what
//! [`CatalogBank::rank`] marks. A catalog this size is not a thing to show somebody on a phone;
//! it is a thing to stop the next person re-downloading four gigabytes to answer a question this one
//! already answered.
//!
//! **A row here is not a bank on the machine.** `karaokemachine`'s `soundfont::installed` answers
//! that, from a folder. These two lists are joined for the `/dev/` picker, and confusing them is the
//! way to offer somebody a bank that is not there.

pub mod digest;

use std::sync::OnceLock;

/// The table itself, verbatim.
const TABLE: &str = include_str!("../data/soundfont-banks.conf");

/// How a bank can be got hold of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The one bank installed into `assets/` and shipped with the machine.
    Bundled,
    /// A direct download pinned by digest.
    Pinned,
    /// Published through a page rather than a direct URL, so there is nothing a program can fetch
    /// and the row carries a [`page`](CatalogBank::page) instead.
    Manual,
}

/// One row of the table.
///
/// Borrowed from [`TABLE`] rather than owned: every field is a slice of a `&'static str` the binary
/// already carries, so the whole catalog costs one `Vec` of pointers.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogBank {
    /// The name a person types — `sc55-v37`.
    pub id: &'static str,
    /// How this one can be got hold of: shipped, pinned to a URL, or by hand only.
    pub status: Status,
    /// What the file is called in the cache, and on disk once fetched.
    pub name: &'static str,
    /// Where to fetch it, for a `pinned` row. `None` for `manual`.
    pub url: Option<&'static str>,
    /// `<sha256 hex>`, or `sha1:<hex>` for a digest archive.org published rather than one this
    /// repository computed. See the table's own header for why both are trusted.
    pub digest: Option<&'static str>,
    /// The exact size, which is how a row is confirmed to name the file the research note measured
    /// rather than a same-named different one.
    pub bytes: u64,
    /// That size as the note writes it — `103.4 MiB`.
    pub size: &'static str,
    /// The archive a bank is published inside, where it is not served loose.
    pub archive: Option<&'static str>,
    /// The archive's own digest.
    pub archive_digest: Option<&'static str>,
    /// Which member of the archive is the bank.
    pub member: Option<&'static str>,
    /// Where a person goes for a `manual` bank.
    pub page: Option<&'static str>,
    /// The `music_volume` this bank wants, where it clips at `1.0`.
    pub volume: Option<f32>,
    /// Song-to-song loudness spread in LU. Lower is better; the bundled bank is 7.8.
    ///
    /// **Every value in the table is a fork measurement**, re-run for §13 at one revision. It read
    /// 6.7 here until then, which was `rustysynth` 1.3.6's number for the same file: implementing
    /// the SF2 modulators 1.3.6 discarded cost the bundled bank 1.1 LU of the one metric it was
    /// chosen on. Do not compare one of these against a figure from §4 of the note.
    pub spread: Option<f32>,
    /// Mean integrated loudness across the seven songs, in LUFS, at `music_volume: 1.0`.
    ///
    /// **The reference a video or MP3+G song is levelled to**, which makes this the one column here
    /// the machine *acts* on rather than merely shows — see `Video and MP3+G play at the MIDI
    /// reference level` in `docs/decisions/audio.md`. `None` falls back to
    /// `km_loudness::DEFAULT_REFERENCE_LUFS`, which is also what somebody's own unlisted `.sf2`
    /// gets.
    ///
    /// **At `music_volume: 1.0`, so [`Self::volume`] does not enter into it.** The owner's level
    /// multiplies a MIDI song and a media song alike, so it cancels out of the difference between
    /// them; the reference has to be the level before it, or a hot bank's reduction would be
    /// counted twice.
    pub lufs: Option<f32>,
    /// What §8 of the research note found about its terms.
    pub license: &'static str,
    /// One line of what the note concluded.
    pub note: &'static str,
    /// A license file that travels with the bank, where one exists.
    ///
    /// Only two rows in the table ship one. It is cached under `<bank>-<lic_name>` rather than under
    /// its own name, because several banks call theirs `LICENSE.txt`.
    pub lic_name: Option<&'static str>,
    /// Where that license file is fetched from.
    pub lic_url: Option<&'static str>,
    /// Its digest, pinned the same way and read the same way as the bank's own.
    pub lic_digest: Option<&'static str>,
    /// Where this bank stands in the shortlist the machine offers, `1` to [`OFFERED`].
    ///
    /// **This is what separates a bank the product offers from one that exists to be tested
    /// against.** The table is the whole survey — every bank measured through this machine's own
    /// engine that has a source worth pinning to — and that is far more than
    /// anybody should be shown on a phone. A ranked row is offered; an unranked one is reachable
    /// only from a shell, through `task soundfont BANK=<id>` and the `Ctrl+1`…`Ctrl+9` switcher.
    ///
    /// Rank 1 is the bundled bank, so the Setup tab shows [`OFFERED`] − 1 buttons: a row inviting
    /// somebody to download the bank they are already listening to would be nonsense, and
    /// `karaokemachine`'s `Machine::soundfont_offers` drops it for that reason rather than this one.
    pub rank: Option<u8>,
    /// The one bank the machine suggests, marked on its row in the Setup tab.
    ///
    /// **Exactly one row carries it**, and a test holds that. A second recommendation is not a
    /// stronger one — it is a list, which is what the other eight rows already are.
    ///
    /// It is deliberately separate from [`CatalogBank::rank`], which is an ordering. A rank of 2
    /// says "shown first among the offers"; this says "we think you want this one", and the two
    /// would not always coincide — the recommended bank is a 261.9 MiB download, and a future list
    /// might reasonably put something smaller at the top while still recommending it.
    ///
    /// **The terms stay printed beside it**, as they are on every other row; see `Where a bank may
    /// be fetched from` in `docs/decisions/repository.md`.
    pub recommended: bool,
    /// Whether this is the bank installed into `assets/` and shipped.
    pub bundled: bool,
}

/// How many banks the machine offers, and therefore how many rows carry a [`CatalogBank::rank`].
///
/// Nine because the switcher has nine keys and a phone has one screen. It is not a limit the
/// measurements imply — sixty banks have a spread wide enough to matter — it is the point at which
/// a list stops being a choice and becomes a catalog to scroll.
pub const OFFERED: u8 = 9;

/// Every bank, in the order the table lists them: the nine ranked ones in rank order, then the rest
/// by ascending loudness spread.
pub fn catalog() -> &'static [CatalogBank] {
    static PARSED: OnceLock<Vec<CatalogBank>> = OnceLock::new();
    PARSED.get_or_init(|| parse(TABLE))
}

/// One row by the name a person types.
///
/// **This was a test helper, and its own doc comment said what would promote it**: *"nothing outside
/// these tests looks a bank up by id yet — the machine matches by filename, since that is what a
/// file on disk and a row in the table share. The downloader will want this, and can move it out
/// when it does."* `km-admin` is that downloader: a page offering sixty-three rows sends back the
/// id of the one somebody picked, because a filename is not what a link carries.
pub fn bank(id: &str) -> Option<&'static CatalogBank> {
    catalog().iter().find(|bank| bank.id == id)
}

/// Whether a row describes something a program could fetch by itself.
///
/// `manual` rows cannot: the publisher serves the file through a page rather than a direct address,
/// so there is no URL to pin a digest against. Such a row carries a [`page`](CatalogBank::page) to
/// send somebody to instead.
///
/// **Stated once because it was already stated twice** — as a test helper here and inline in the
/// machine's `soundfont_offers`, which fills `SoundFontOfferDto::fetchable`. Two readings of "can
/// this be fetched?" that could drift is exactly the shape this crate exists to stop.
pub fn fetchable(bank: &CatalogBank) -> bool {
    bank.status != Status::Manual && bank.url.is_some() && bank.digest.is_some()
}

/// Reads the table.
///
/// **Panics on a malformed row rather than skipping it**, and that is right here where it would be
/// wrong for a folder of files: this input is compiled into the binary, so a fault in it is a fault
/// in the build and every test below runs against the real thing. A row silently dropped would be a
/// bank that quietly stopped being offered.
fn parse(text: &'static str) -> Vec<CatalogBank> {
    let mut banks: Vec<CatalogBank> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(id) = trimmed.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            banks.push(CatalogBank {
                id,
                status: Status::Pinned,
                name: "",
                url: None,
                digest: None,
                bytes: 0,
                size: "",
                archive: None,
                archive_digest: None,
                member: None,
                page: None,
                volume: None,
                spread: None,
                lufs: None,
                license: "",
                note: "",
                lic_name: None,
                lic_url: None,
                lic_digest: None,
                rank: None,
                recommended: false,
                bundled: false,
            });
            continue;
        }

        let bank = banks.last_mut().unwrap_or_else(|| {
            panic!("soundfont-banks.conf:{}: a key before any [id]", number + 1)
        });
        let (key, value) = trimmed.split_once(char::is_whitespace).unwrap_or_else(|| {
            panic!(
                "soundfont-banks.conf:{}: `{trimmed}` has no value",
                number + 1
            )
        });
        let value = value.trim_start();

        match key {
            "status" => {
                bank.status = match value {
                    "bundled" => Status::Bundled,
                    "pinned" => Status::Pinned,
                    "manual" => Status::Manual,
                    other => panic!(
                        "soundfont-banks.conf:{}: unknown status `{other}`",
                        number + 1
                    ),
                }
            }
            "name" => bank.name = value,
            "url" => bank.url = Some(value),
            "digest" => bank.digest = Some(value),
            "bytes" => {
                bank.bytes = value.parse().unwrap_or_else(|_| {
                    panic!(
                        "soundfont-banks.conf:{}: `{value}` is not a size",
                        number + 1
                    )
                })
            }
            "size" => bank.size = value,
            "archive" => bank.archive = Some(value),
            "archive_digest" => bank.archive_digest = Some(value),
            "member" => bank.member = Some(value),
            "page" => bank.page = Some(value),
            "volume" => {
                bank.volume = Some(value.parse().unwrap_or_else(|_| {
                    panic!(
                        "soundfont-banks.conf:{}: `{value}` is not a level",
                        number + 1
                    )
                }))
            }
            "spread" => {
                bank.spread = Some(value.parse().unwrap_or_else(|_| {
                    panic!(
                        "soundfont-banks.conf:{}: `{value}` is not a spread",
                        number + 1
                    )
                }))
            }
            "lufs" => {
                bank.lufs = Some(value.parse().unwrap_or_else(|_| {
                    panic!(
                        "soundfont-banks.conf:{}: `{value}` is not a loudness",
                        number + 1
                    )
                }))
            }
            "license" => bank.license = value,
            "note" => bank.note = value,
            "lic_name" => bank.lic_name = Some(value),
            "lic_url" => bank.lic_url = Some(value),
            "lic_digest" => bank.lic_digest = Some(value),
            "rank" => {
                let rank: u8 = value.parse().unwrap_or_else(|_| {
                    panic!(
                        "soundfont-banks.conf:{}: `{value}` is not a rank",
                        number + 1
                    )
                });
                if rank == 0 || rank > OFFERED {
                    panic!(
                        "soundfont-banks.conf:{}: rank {rank} is outside 1..={OFFERED}",
                        number + 1
                    );
                }
                bank.rank = Some(rank);
            }
            "recommended" => bank.recommended = value == "1",
            "bundled" => bank.bundled = value == "1",
            other => panic!("soundfont-banks.conf:{}: unknown key `{other}`", number + 1),
        }
    }
    banks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shortlist the machine offers: nine, ranked, first in the file, the bundled bank at 1.
    ///
    /// **The properties, not the table.** Sixty-odd ids as a literal list is re-typed rather than
    /// read, and it says nothing about *why* an order is right; a rank repeated or skipped passes
    /// such a list and breaks the Setup tab.
    #[test]
    fn nine_banks_are_ranked_and_they_come_first() {
        let ranked: Vec<(&str, u8)> = catalog()
            .iter()
            .filter_map(|bank| bank.rank.map(|rank| (bank.id, rank)))
            .collect();
        assert_eq!(ranked.len(), usize::from(OFFERED), "{ranked:?}");

        // Ranks are 1..=OFFERED with nothing repeated and nothing missed, and file order is rank
        // order — `soundfont-debug.sh` fills its eight switcher slots from the top of the file down,
        // so the two cannot be allowed to disagree.
        let mut seen: Vec<u8> = ranked.iter().map(|(_, rank)| *rank).collect();
        assert_eq!(seen, (1..=OFFERED).collect::<Vec<_>>(), "{ranked:?}");
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), usize::from(OFFERED));

        assert_eq!(ranked[0].0, "generaluser", "rank 1 is the bundled bank");

        // The order somebody arrived at by listening to all eight on a real machine. It is not the
        // spread column's order and is not meant to be: that column puts Colombo seventh and the
        // SC-55 eighth, and a person put them first and second.
        let ids: Vec<&str> = ranked.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            ids,
            [
                "generaluser",
                "colombogmgs2",
                "sc55-v37",
                "sgm-guits",
                "musicatheoria",
                "chorium",
                "aspirin",
                "fluidr3",
                "musescore",
            ]
        );
        let first_unranked = catalog()
            .iter()
            .position(|bank| bank.rank.is_none())
            .expect("some rows are unranked");
        assert_eq!(
            first_unranked,
            usize::from(OFFERED),
            "ranked rows come first"
        );
    }

    /// Exactly one bank is recommended, and it is one the machine can actually fetch.
    ///
    /// **A second recommendation is not a stronger one, it is a list** — and the other eight rows
    /// are already that. Recommending a `manual` row would be worse still: a suggestion somebody
    /// cannot act on without leaving the page.
    #[test]
    fn one_bank_is_recommended_and_it_can_be_fetched() {
        let picked: Vec<&str> = catalog()
            .iter()
            .filter(|bank| bank.recommended)
            .map(|bank| bank.id)
            .collect();
        assert_eq!(picked, ["colombogmgs2"]);

        let bank = bank("colombogmgs2").expect("colombogmgs2");
        assert!(fetchable(bank), "a recommendation must be actionable");
        assert!(bank.rank.is_some(), "an unranked row is never shown");
        // And it is not the bundled bank, which is never an offer and so could never carry a badge.
        assert!(!bank.bundled);
    }

    /// The unranked rows are worth reaching, which is what `GET /audio/soundfonts?all=true` assumes.
    ///
    /// **The premise of the wider list, asserted rather than assumed.** That route exists so the rest
    /// of the survey can be fetched from the `/dev/` page instead of from a shell, and it would be a
    /// page of rows nobody could act on if the table's unranked half were all `manual` or all
    /// unpinned. It is the same check [`one_bank_is_recommended_and_it_can_be_fetched`] makes of the
    /// recommendation, applied to the part of the table a singer never sees.
    #[test]
    fn most_of_the_catalog_is_unranked_and_can_still_be_fetched() {
        let unranked: Vec<&CatalogBank> = catalog()
            .iter()
            .filter(|bank| bank.rank.is_none())
            .collect();
        assert!(
            unranked.len() > usize::from(OFFERED),
            "the shortlist is most of the table, so widening it buys nothing: {} rows",
            unranked.len()
        );
        let actionable = unranked.iter().filter(|bank| fetchable(bank)).count();
        assert!(
            actionable > unranked.len() / 2,
            "only {actionable} of {} unranked rows can be fetched",
            unranked.len()
        );
    }

    /// Below the shortlist the file is ordered by the metric, which is what `--list` prints.
    #[test]
    fn the_rest_are_listed_by_ascending_spread() {
        let mut previous = 0.0_f32;
        for bank in catalog().iter().filter(|bank| bank.rank.is_none()) {
            let spread = bank
                .spread
                .unwrap_or_else(|| panic!("{} has no spread", bank.id));
            assert!(
                spread >= previous,
                "{} breaks the ordering at {spread} after {previous}",
                bank.id
            );
            previous = spread;
        }
    }

    /// **A filename is an identity here, not a label.** `Machine::measured_level` joins a chosen bank
    /// to its row by filename to find the level it wants, so two rows sharing a name would give one
    /// of them somebody else's `music_volume` — silently, and differently depending on table order.
    /// The survey supplies the case: archive.org's `MuseScore_General.sf2` is v0.1.1 and osuosl's is
    /// v0.2, and the older one has no row for exactly this reason.
    #[test]
    fn no_two_rows_name_the_same_file() {
        let mut names: Vec<&str> = catalog().iter().map(|bank| bank.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two rows share a filename");
    }

    /// Every row needs the fields a listing prints, whatever else it has.
    #[test]
    fn every_row_carries_what_a_listing_needs() {
        for bank in catalog() {
            assert!(!bank.name.is_empty(), "{} has no file name", bank.id);
            assert!(!bank.size.is_empty(), "{} has no size", bank.id);
            assert!(!bank.license.is_empty(), "{} has no license", bank.id);
            assert!(!bank.note.is_empty(), "{} has no note", bank.id);
            assert!(bank.bytes > 0, "{} has no byte count", bank.id);
            // **Case-insensitively**, which the second survey forced: real banks are published as
            // `ChoriumRevA.SF2`, `CREATIVE_8MBGM.SF2` and `tgk3.SF2` at least as often as in lower
            // case. `soundfont::is_bank_file` has always compared this way, so the machine handled
            // them and only this assertion did not.
            assert!(
                std::path::Path::new(bank.name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("sf2")),
                "{} is not an .sf2: {}",
                bank.id,
                bank.name
            );
        }
    }

    /// **A value in this file is printed verbatim, and the comments around it are Markdown.**
    ///
    /// `note` and `license` reach a person through `km-admin`'s Sound tab and the machine's own
    /// listing, both of which escape what they are given rather than render it. The `#` lines in
    /// `soundfont-banks.conf` are prose *about* the table and use `**bold**` freely, so the two
    /// styles sit one line apart and the wrong one leaks: `roland-sc55`'s note read
    /// `**diverges** on two of the seven songs` from the day it was written, and printed its own
    /// asterisks in every window that showed it. No status code can see that, and neither can a
    /// person reading the file, where the markup looks like every comment above it.
    ///
    /// Asterisks and backticks only. Underscores are not checked because a file name is allowed to
    /// contain them and several do.
    #[test]
    fn no_row_authors_markdown_into_a_value() {
        for bank in catalog() {
            for (field, text) in [("note", bank.note), ("license", bank.license)] {
                for markup in ["**", "`"] {
                    assert!(
                        !text.contains(markup),
                        "{}'s {field} carries {markup}, which is printed rather than rendered: {text}",
                        bank.id
                    );
                }
            }
        }
    }

    /// The property the downloader rests on: anything it will fetch is pinned by a digest.
    #[test]
    fn anything_fetchable_is_pinned_by_a_digest() {
        for bank in catalog() {
            if bank.status == Status::Manual {
                assert!(
                    !fetchable(bank),
                    "{} is manual and must not be fetchable",
                    bank.id
                );
                assert!(
                    bank.url.is_none(),
                    "{} is manual and must have no url",
                    bank.id
                );
                assert!(
                    bank.page.is_some(),
                    "{} is manual and must name a page",
                    bank.id
                );
                continue;
            }
            assert!(fetchable(bank), "{} should be fetchable", bank.id);
            assert!(bank.url.is_some(), "{} has no url", bank.id);
            let digest = bank.digest.expect("a digest");
            // Either 64 hex characters of sha256, or archive.org's own published sha1.
            let ok = match digest.strip_prefix("sha1:") {
                Some(sha1) => sha1.len() == 40 && sha1.chars().all(|c| c.is_ascii_hexdigit()),
                None => digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()),
            };
            assert!(ok, "{} has a malformed digest: {digest}", bank.id);
        }
    }

    /// Exactly one bank ships, and it is the one `assets/` holds.
    #[test]
    fn one_bank_is_the_bundled_one() {
        let bundled: Vec<&str> = catalog()
            .iter()
            .filter(|bank| bank.bundled)
            .map(|bank| bank.id)
            .collect();
        assert_eq!(bundled, ["generaluser"]);
        let bank = bank("generaluser").expect("the bundled bank");
        assert_eq!(bank.status, Status::Bundled);
        // The one license file that ships, pinned by digest — see the table's own note on why.
        assert_eq!(bank.lic_name, Some("LICENSE.txt"));
        assert!(bank.lic_digest.is_some());
    }

    /// The one row published inside a zip, which is why the archive fields exist at all.
    #[test]
    fn the_archived_bank_names_its_member() {
        let bank = bank("fluidr3").expect("fluidr3");
        assert_eq!(bank.archive, Some("fluid-soundfont.zip"));
        assert_eq!(bank.member, Some("FluidR3 GM2-2.SF2"));
        assert!(bank.archive_digest.is_some());
        // The member's own digest is the bank's, not the archive's.
        assert_ne!(bank.digest, bank.archive_digest);
    }

    /// The levels the note derived, which the machine applies when a bank is chosen.
    ///
    /// **Two of these are corrections rather than entries**, and both are the same mistake: a level
    /// that was estimated instead of derived came out short. Arachno was `0.7` on nothing but
    /// inference until a copy was fetched by hand for §13 and measured a pre-clamp peak of 1.589 —
    /// so 0.6. Musyng Kite carried none at all because 1,020 MiB was a day's download at the rate
    /// §12 recorded; it is 0.5.
    #[test]
    fn the_measured_levels_are_the_ones_the_note_derived() {
        for (id, volume) in [
            ("musescore", 0.75),
            ("fluidr3", 0.61),
            ("arachno", 0.62),
            ("sc55-v37", 0.61),
            ("sgm", 0.83),
            ("musyng", 0.53),
            ("aspirin", 0.50),
            ("jurgen", 0.29),
            ("chorium", 0.69),
            ("toh34", 0.20),
        ] {
            assert_eq!(bank(id).expect(id).volume, Some(volume), "{id}");
        }
        // And the banks that clip nothing carry none, which is a finding and not a gap:
        // 0 of 7 clipped means the largest pre-clamp peak is already under 1.0, so there is nothing
        // for a reduction to buy.
        for id in [
            "generaluser",
            "timgm6mb",
            "uhd3",
            "sc55",
            "fatboy",
            "musicatheoria",
            "colombogmgs2",
            "symphonyhall",
        ] {
            assert_eq!(bank(id).expect(id).volume, None, "{id}");
        }
    }

    /// Every level is a hundredth between 0.01 and 1.0, because that is what the method produces:
    /// the largest step that clears the bank's own peak. A level outside it is a typo rather than a
    /// measurement.
    ///
    /// **It was tenths until somebody listened.** Chorium's peak is 1.431, so it clears at 0.69 and
    /// a tenth handed it 0.6 — 1.2 dB quieter than it needed to be, which is enough to lose a
    /// comparison against a bank whose peak happened to round more kindly. The rule is unchanged;
    /// only the step is finer.
    #[test]
    fn every_level_is_a_step_the_method_could_have_produced() {
        for bank in catalog() {
            let Some(volume) = bank.volume else { continue };
            assert!(
                (0.01..=1.0).contains(&volume),
                "{} has an impossible level {volume}",
                bank.id
            );
            let steps = volume * 100.0;
            assert!(
                (steps - steps.round()).abs() < 1e-3,
                "{} has a level that is not a hundredth: {volume}",
                bank.id
            );
        }
    }

    /// The reference levels, and the bundled bank's above all — it is what levelling calibrates to.
    ///
    /// **Re-measured at rustysynth `3e5ef8bd`** with `tools/dev/soundfont-measure.sh` over the asset
    /// cache. The same run reproduced each of these rows' `spread` to the decimal — 7.8, 7.4, 10.6,
    /// 8.7, 6.2, 6.6 — which is what says the loudness beside them was taken with the synthesizer
    /// the machine renders with rather than with the one the note used.
    #[test]
    fn the_reference_levels_are_the_ones_that_were_measured() {
        for (id, lufs) in [
            ("generaluser", -21.9),
            ("colombogmgs2", -21.0),
            ("sc55-v37", -18.2),
            ("chorium", -19.7),
            ("somsak", -12.4),
            ("sc55", -26.6),
        ] {
            assert_eq!(bank(id).expect(id).lufs, Some(lufs), "{id}");
        }
    }

    /// Every reference level is a real loudness: negative, and not absurdly so.
    ///
    /// **A sign error is the failure this catches**, and it is the one worth catching: LUFS is
    /// negative for everything but a divergence, and a positive value here would make `gain_for`
    /// attenuate every media song to the floor. The measured field runs from −12.4 to −26.6, so the
    /// bounds are wide enough to admit a bank nobody has measured yet and tight enough that a typo
    /// of a digit fails.
    #[test]
    fn every_reference_level_is_a_plausible_loudness() {
        for bank in catalog() {
            let Some(lufs) = bank.lufs else { continue };
            assert!(
                (-40.0..=-5.0).contains(&lufs),
                "{} has an implausible loudness {lufs}",
                bank.id
            );
        }
    }

    /// A value runs to the end of the line — a `#` in one would be text and not a comment, and the
    /// spaces and quotes a real row does carry are pinned here.
    #[test]
    fn a_value_runs_to_the_end_of_the_line() {
        let uhd3 = bank("uhd3").expect("uhd3");
        assert_eq!(uhd3.license, "\"Not for commercial distribution\"");
        assert_eq!(uhd3.name, "UHD3.sf2");
        // Spaces in a filename, which is the case the id slug exists for.
        assert_eq!(
            bank("sc55-v37").expect("sc55-v37").name,
            "Roland SC-55 v3.7.sf2"
        );
    }

    #[test]
    fn an_unknown_name_is_none_rather_than_a_panic() {
        assert!(bank("nonesuch").is_none());
        assert!(bank("").is_none());
    }
}
