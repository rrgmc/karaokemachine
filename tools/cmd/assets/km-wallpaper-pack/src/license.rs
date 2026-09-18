//! What one image may be done with, as its source stated it.
//!
//! This crate spent a month with the license as a constant per *provider* —
//! `ProviderKind::license()` returning one `&'static str` per website — and that is the shape which
//! made the question invisible. A type that cannot represent a per-image license never prompts
//! anybody to look one up, and nobody did until an aggregator made the constant impossible to keep.
//! See `Where a wallpaper pack's photographs may come from` in `docs/decisions/repository.md`.
//!
//! **The distinction this module exists to draw is not "free" versus "paid".** Every source here is
//! free to *use*. What differs is whether the pack may be **passed on**, which Pixabay and Pexels
//! grant for neither, and which has nothing to do with money.

use serde::{Deserialize, Serialize};

/// The license one image carries.
///
/// `Eq` rather than just `PartialEq`, because [`crate::providers::RemoteImage`] derives `Eq` and
/// this becomes a field of it. Three string fields, so that costs nothing — but it is the reason no
/// ratio or score may ever join either struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct License {
    /// Machine-readable, and **the only thing [`License::redistribution`] decides from**: `cc0`,
    /// `pdm`, `by`, `by-sa`, `by-nc`, `by-nd` as Creative Commons spells them, or `pixabay` /
    /// `pexels` for a provider's own terms.
    pub code: String,
    /// The name a person reads: `CC0 1.0`, `CC BY 4.0`, `Pixabay Content License`.
    pub name: String,
    /// Where the license text is. `None` for a provider's own terms, which `ATTRIBUTION.md` then
    /// names without linking rather than linking to something invented.
    pub url: Option<String>,
}

/// Whether a pack holding this image may be handed on — and, when it may not, the sentence saying so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redistribution {
    /// The license grants it outright.
    Granted,
    /// Use it on the machine that built it. The pack may not be passed on.
    Personal {
        /// Whose terms these are, for the message.
        source: &'static str,
        /// What they cover, in one sentence.
        terms: &'static str,
    },
}

impl Redistribution {
    /// Whether a pack carrying this image may be handed on.
    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted)
    }
}

/// The license codes whose images may be redistributed.
///
/// **A constant and never a config key**, so that widening it is a change somebody reviews rather
/// than a setting somebody types.
///
/// The exclusions are choices, and each is written down here:
///
/// * **`by-nd` is out because this crate is a derivative factory.** [`crate::process::render`] crops,
///   resizes, blurs, vignettes and re-encodes every image with no path that skips it, so what comes
///   out is an adapted work and No-Derivatives does not cover one.
/// * **`by-sa` is out** because explaining which of two licenses governs which bytes of one download
///   is a cost with no matching benefit — the pack is a separate zip and would not reach the code
///   either way. Worth knowing what it excludes: Wiki Loves Monuments, the largest body of good
///   freely-licensed landmark photography, is CC BY-SA. If a pack ever wants famous landmarks rather
///   than landscapes, this is the exclusion to revisit deliberately.
/// * **`by-nc` is out** because it would attach to everyone downstream of a permissively-licensed
///   application.
const REDISTRIBUTABLE: [&str; 3] = ["cc0", "pdm", "by"];

impl License {
    /// A Creative Commons license, from the code and version an aggregator reports.
    ///
    /// `cc0` and `pdm` are not versioned in the way `by` is, but Openverse reports a version for all
    /// of them, so the name is built the same way throughout rather than special-cased.
    pub fn creative_commons(code: &str, version: &str, url: Option<String>) -> Self {
        Self {
            code: code.to_ascii_lowercase(),
            name: cc_name(code, version),
            url,
        }
    }

    /// A provider's own terms, which are one license for the whole site and have no per-image URL.
    pub fn site_terms(code: &'static str, name: &'static str) -> Self {
        Self {
            code: code.to_owned(),
            name: name.to_owned(),
            url: None,
        }
    }

    /// A license nothing recorded.
    ///
    /// Not redistributable, because [`License::redistribution`] fails closed — which is the right
    /// answer: an image whose license nobody wrote down is exactly the thing that must not ship.
    pub fn unstated() -> Self {
        Self {
            code: "unstated".to_owned(),
            name: "not stated".to_owned(),
            url: None,
        }
    }

    /// Whether a pack carrying this image may be handed on.
    ///
    /// **The fallback arm is `Personal`, and that is the design.** A code this tool has never heard
    /// of is not redistributable, so a source added later cannot widen the position by accident —
    /// somebody has to come here and say so.
    pub fn redistribution(&self) -> Redistribution {
        if REDISTRIBUTABLE.contains(&self.code.as_str()) {
            return Redistribution::Granted;
        }
        match self.code.as_str() {
            "pixabay" => Redistribution::Personal {
                source: "Pixabay",
                terms: "Its license covers using the photographs and not passing a pack of them on.",
            },
            "pexels" => Redistribution::Personal {
                source: "Pexels",
                terms: "Its API guidelines cover using the photographs and not passing a pack of \
                        them on.",
            },
            _ => Redistribution::Personal {
                source: "this license",
                terms: "It is not one this tool knows to grant redistribution, so it is treated as \
                        permitting use and not passing on.",
            },
        }
    }

    /// The code for a license named the way `ATTRIBUTION.md` and `credits.toml` write it.
    ///
    /// For [`crate::commands::local`], whose sidecar is typed by a person and carries a name rather
    /// than a code. **Exact matches only, and `unstated` otherwise** — guessing from a string that
    /// merely contains "CC BY" would read `CC BY-NC-ND 4.0` as `by`, which is the one direction this
    /// must never be wrong in. A curated pack that wants a code this table lacks says so explicitly
    /// with `license_code` in its sidecar.
    pub fn code_for_name(name: &str) -> String {
        let known = [
            ("cc0 1.0", "cc0"),
            ("cc0", "cc0"),
            ("public domain mark 1.0", "pdm"),
            ("public domain mark", "pdm"),
            ("cc by 4.0", "by"),
            ("cc by 3.0", "by"),
            ("cc by 2.0", "by"),
            ("cc by-sa 4.0", "by-sa"),
            ("cc by-nc 4.0", "by-nc"),
            ("cc by-nd 4.0", "by-nd"),
            ("pixabay content license", "pixabay"),
            ("pexels license", "pexels"),
        ];
        let lowered = name.trim().to_ascii_lowercase();
        known
            .iter()
            .find(|(candidate, _)| *candidate == lowered)
            .map(|(_, code)| (*code).to_owned())
            .unwrap_or_else(|| "unstated".to_owned())
    }
}

/// `CC BY 4.0` from `by` and `4.0`.
fn cc_name(code: &str, version: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    let version = version.trim();
    let spelled = match code.as_str() {
        "cc0" => "CC0".to_owned(),
        "pdm" => "Public Domain Mark".to_owned(),
        other => format!("CC {}", other.to_ascii_uppercase()),
    };
    if version.is_empty() {
        spelled
    } else {
        format!("{spelled} {version}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc0_and_public_domain_and_attribution_may_be_passed_on() {
        for code in ["cc0", "pdm", "by"] {
            let license = License::creative_commons(code, "4.0", None);
            assert!(
                license.redistribution().is_granted(),
                "{code} should be redistributable"
            );
        }
    }

    /// No-Derivatives forbids sharing an adapted work, and `process::render` adapts every image
    /// there is. This is the exclusion that is not a judgment call.
    #[test]
    fn by_nd_is_refused_because_this_crate_derives_every_image() {
        let license = License::creative_commons("by-nd", "4.0", None);
        assert!(!license.redistribution().is_granted());
    }

    /// "We do not charge today" is not a license position: NC binds everyone downstream of a
    /// permissively-licensed application.
    #[test]
    fn by_nc_is_refused_because_the_application_is_permissively_licensed() {
        let license = License::creative_commons("by-nc", "4.0", None);
        assert!(!license.redistribution().is_granted());
    }

    /// A share-alike obligation on images inside an MIT/Apache release is the confusion this whole
    /// exercise ended. Excluded deliberately rather than by omission.
    #[test]
    fn by_sa_is_refused_because_a_second_license_in_one_download_is_the_confusion_to_avoid() {
        let license = License::creative_commons("by-sa", "4.0", None);
        assert!(!license.redistribution().is_granted());
    }

    /// The fallback arm. A source added later cannot widen the position by accident.
    #[test]
    fn a_code_this_tool_has_never_heard_of_is_not_redistributable() {
        let license = License {
            code: "some-new-license".to_owned(),
            name: "Some New License".to_owned(),
            url: None,
        };
        assert!(!license.redistribution().is_granted());
    }

    /// The refusal names whose terms it is and says what they cover, so a build warning can too.
    ///
    /// It asserted a word out of Pixabay's quoted clause until the refusals stopped quoting and
    /// started summarising, which left it failing on every platform for a day: `cargo km-test` does
    /// not reach this workspace, and `tools/platform/linux/check.sh` — which does — is not what
    /// anybody runs by habit. What it checks now is what the summary is *for*: the source is
    /// attributable and the position travels with it.
    #[test]
    fn a_providers_own_terms_are_refused_and_the_reason_travels_with_the_refusal() {
        let license = License::site_terms("pixabay", "Pixabay Content License");
        match license.redistribution() {
            Redistribution::Granted => panic!("a Pixabay pack may not be passed on"),
            Redistribution::Personal { source, terms } => {
                assert_eq!(source, "Pixabay");
                assert!(terms.contains("not passing a pack of them on"), "{terms}");
            }
        }
    }

    #[test]
    fn a_license_is_named_the_way_a_person_writes_it() {
        assert_eq!(
            License::creative_commons("by", "4.0", None).name,
            "CC BY 4.0"
        );
        assert_eq!(
            License::creative_commons("cc0", "1.0", None).name,
            "CC0 1.0"
        );
        assert_eq!(
            License::creative_commons("by-sa", "3.0", None).name,
            "CC BY-SA 3.0"
        );
        assert_eq!(License::creative_commons("by", "", None).name, "CC BY");
    }

    #[test]
    fn a_name_a_person_typed_maps_back_to_its_code() {
        assert_eq!(License::code_for_name("CC0 1.0"), "cc0");
        assert_eq!(License::code_for_name("  cc by 4.0 "), "by");
        assert_eq!(License::code_for_name("Pixabay Content License"), "pixabay");
    }

    /// The direction this must never be wrong in. A substring rule would read the most restrictive
    /// Creative Commons license there is as the most permissive one.
    #[test]
    fn a_name_that_merely_contains_cc_by_does_not_become_by() {
        assert_eq!(License::code_for_name("CC BY-NC-ND 4.0"), "unstated");
        assert!(
            !License {
                code: License::code_for_name("CC BY-NC-ND 4.0"),
                name: "CC BY-NC-ND 4.0".to_owned(),
                url: None,
            }
            .redistribution()
            .is_granted()
        );
    }
}
