//! Deciding which survivors go in the pack, and saying out loud why the rest did not.
//!
//! The scoring is four weighted terms, and the weights encode what a karaoke wallpaper is for rather
//! than what makes a good photograph:
//!
//! * **Entropy** (0.35) — something to look at for three minutes, but not chaos.
//! * **A calm band** (0.30) — the strongest signal that lyrics will be readable, and the one that no
//!   amount of darkening can rescue.
//! * **Little darkening needed** (0.20) — the less scrim an image needs to clear the gate, the more
//!   of the photograph survives on screen.
//! * **Size** (0.15) — a 24-megapixel original crops and downscales better than a 2-megapixel one.
//!
//! **Every rejection is named.** A pack that comes back smaller than expected is the normal case, and
//! without a reason per image the only way to tune a threshold is guessing.

use std::collections::BTreeMap;

use crate::config::{Filters, Legibility};
use crate::metrics::ImageMetrics;

/// Why an image is not in the pack.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rejection {
    /// The photograph the provider described is smaller than `min_source_width`.
    TooSmall,
    /// The file actually downloaded is narrower than `min_decoded_width`.
    ///
    /// Distinct from [`Rejection::TooSmall`], and the distinction is the point. A provider can
    /// advertise a 5151-pixel original and serve a 1280-pixel copy of it; only this one is about the
    /// bytes that would go into the pack. Sharing a label would have hidden that, which is exactly
    /// what happened when a single threshold was read off the metadata for both questions.
    TooSmallOnDisk,
    /// Below `min_entropy`: a flat or blown-out frame.
    LowEntropy,
    /// Above `max_band_busyness`: detail where the words go.
    TooBusy,
    /// No darkening within the app's own scrim reaches the target contrast.
    ContrastUnreachable,
    /// The cached original is missing or could not be decoded, so nothing about it is known.
    ///
    /// Not a judgment on the photograph — a gap in the cache. It exists so that `chosen + rejected`
    /// equals the number of images `analyze` says it knows about. Twelve images once disappeared
    /// between those two numbers with no reason recorded anywhere, which is exactly the failure the
    /// rest of this enum was written to prevent.
    Unmeasurable(&'static str),
    /// The same photograph as another, which is kept instead.
    DuplicateOf(String),
    /// The per-term cap is already full.
    QueryFull(String),
    /// The pack reached `target_count` first.
    PackFull,
    /// Its license does not grant redistribution, and `[filters] redistributable_only` is on.
    ///
    /// Carries the license code, because that is what the decision was made from:
    /// `not_redistributable:pixabay` in the histogram says what was dropped *and* why, where a bare
    /// count would leave somebody guessing which of their query groups it came from.
    NotRedistributable(String),
}

impl Rejection {
    /// The short label used in the histogram and in logs.
    pub fn label(&self) -> String {
        match self {
            Self::TooSmall => "too_small".to_owned(),
            Self::TooSmallOnDisk => "too_small_on_disk".to_owned(),
            Self::LowEntropy => "low_entropy".to_owned(),
            Self::TooBusy => "too_busy".to_owned(),
            Self::ContrastUnreachable => "contrast_unreachable".to_owned(),
            Self::Unmeasurable(cause) => format!("unmeasurable:{cause}"),
            Self::DuplicateOf(key) => format!("duplicate_of:{key}"),
            Self::QueryFull(term) => format!("query_full:{term}"),
            Self::PackFull => "pack_full".to_owned(),
            Self::NotRedistributable(code) => format!("not_redistributable:{code}"),
        }
    }

    /// The label without its detail, for counting.
    ///
    /// **Every arm is its own label up to the first `:`**, which is what lets
    /// [`Rejection::kind_of_label`] answer the same question from the string alone.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TooSmall => "too_small",
            Self::TooSmallOnDisk => "too_small_on_disk",
            Self::LowEntropy => "low_entropy",
            Self::TooBusy => "too_busy",
            Self::ContrastUnreachable => "contrast_unreachable",
            Self::Unmeasurable(_) => "unmeasurable",
            Self::DuplicateOf(_) => "duplicate_of",
            Self::QueryFull(_) => "query_full",
            Self::PackFull => "pack_full",
            Self::NotRedistributable(_) => "not_redistributable",
        }
    }

    /// The kind a label belongs to, for a caller holding only the recorded string.
    ///
    /// **`analysis.json` records [`Rejection::label`] and not the enum**, so a reader of that file
    /// has the detail welded to the kind and no way back to the variant. Counting the whole label
    /// then puts every duplicate in a bucket of its own — a page of `duplicate_of:pixabay:1633185
    /// ×1` where the question being asked is which threshold did the rejecting.
    ///
    /// `kind_and_label_are_one_vocabulary` is what keeps this answering exactly what [`Rejection::kind`]
    /// would.
    #[must_use]
    pub fn kind_of_label(label: &str) -> &str {
        label.split_once(':').map_or(label, |(kind, _)| kind)
    }
}

/// How big an image is, from the two sources that do not agree about it.
///
/// One struct rather than three parameters because the first two are a pair that has already been
/// confused once, with consequences: a threshold meant for the file was applied to the metadata, and
/// a pack of 1280-pixel downloads was upscaled to 4K for a month without a single rejection to show
/// for it. Sitting next to each other, they are hard to mix up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dimensions {
    /// Width of the photograph as the *provider describes* it.
    pub source_width: u32,
    /// Width of the file actually downloaded and decoded.
    pub decoded_width: u32,
    /// Megapixels of the photograph as the provider describes it, for the size term of the score.
    pub megapixels: f32,
}

/// One image with everything needed to judge it.
#[derive(Debug, Clone)]
pub struct Scored {
    /// `provider:id`.
    pub key: String,
    /// The search term that surfaced it.
    pub query: String,
    /// How big it is, both as advertised and as downloaded.
    pub dimensions: Dimensions,
    /// What was measured.
    pub metrics: ImageMetrics,
    /// The score, 0..1.
    pub score: f32,
}

/// Scores one image. Returns `None` when a hard filter rejects it.
pub fn score(
    key: String,
    query: String,
    dimensions: Dimensions,
    metrics: ImageMetrics,
    filters: &Filters,
    legibility: &Legibility,
) -> std::result::Result<Scored, (String, Rejection)> {
    if dimensions.source_width < filters.min_source_width {
        return Err((key, Rejection::TooSmall));
    }
    if dimensions.decoded_width < filters.min_decoded_width {
        return Err((key, Rejection::TooSmallOnDisk));
    }
    if metrics.entropy < filters.min_entropy {
        return Err((key, Rejection::LowEntropy));
    }
    if metrics.band_busyness > filters.max_band_busyness {
        return Err((key, Rejection::TooBusy));
    }
    let Some(required) = metrics.required_alpha else {
        return Err((key, Rejection::ContrastUnreachable));
    };

    let entropy_term = normalize(metrics.entropy, filters.min_entropy, 7.5);
    let calm_term = 1.0 - normalize(metrics.band_busyness, 0.0, filters.max_band_busyness);
    // Relative to the scrim it is judged against: needing 0.2 of a 0.45 scrim leaves more of the
    // photograph on screen than needing 0.44 of it, and `assumed_dim` is the only scale that makes
    // that comparable across configs.
    let headroom = if legibility.assumed_dim > 0.0 {
        1.0 - (required / legibility.assumed_dim).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let size_term = normalize(dimensions.megapixels, 2.0, 24.0);

    let score = 0.35 * entropy_term + 0.30 * calm_term + 0.20 * headroom + 0.15 * size_term;
    Ok(Scored {
        key,
        query,
        dimensions,
        metrics,
        score,
    })
}

/// What a selection run produced.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// The pack, best first.
    pub chosen: Vec<Scored>,
    /// Everything else, and why.
    pub rejected: Vec<(String, Rejection)>,
}

impl Selection {
    /// How many were rejected for each kind of reason.
    pub fn histogram(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for (_, rejection) in &self.rejected {
            *counts.entry(rejection.kind()).or_insert(0) += 1;
        }
        counts
    }
}

/// Picks the pack: best first, no term over its cap, stopping at `target_count`.
///
/// Sorted by score descending with the digest as tie-break, so two runs over the same cache produce
/// the same pack in the same order — which is what makes the manifest comparable between runs.
pub fn select(mut scored: Vec<Scored>, filters: &Filters, target_count: usize) -> Selection {
    scored.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.key.cmp(&b.key)));

    let mut per_query: BTreeMap<String, usize> = BTreeMap::new();
    let mut selection = Selection::default();
    for image in scored {
        if selection.chosen.len() >= target_count {
            selection.rejected.push((image.key, Rejection::PackFull));
            continue;
        }
        let used = per_query.entry(image.query.clone()).or_insert(0);
        if *used >= filters.max_per_query {
            let term = image.query.clone();
            selection
                .rejected
                .push((image.key, Rejection::QueryFull(term)));
            continue;
        }
        *used += 1;
        selection.chosen.push(image);
    }
    selection
}

/// Maps a value onto 0..1 between `low` and `high`, clamped.
pub fn normalize(value: f32, low: f32, high: f32) -> f32 {
    if high <= low {
        return 0.0;
    }
    ((value - low) / (high - low)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(entropy: f32, busyness: f32, required: Option<f32>) -> ImageMetrics {
        ImageMetrics {
            mean_luma: 0.2,
            entropy,
            band_busyness: busyness,
            band_max_cell_luma: 0.1,
            required_alpha: required,
            measured_contrast: required.map(|_| 8.0),
            contrast_at_assumed_dim: 8.0,
            required_alpha_uncapped: required.or(Some(0.9)),
        }
    }

    fn dimensions(source_width: u32, decoded_width: u32) -> Dimensions {
        Dimensions {
            source_width,
            decoded_width,
            megapixels: 12.0,
        }
    }

    fn scored(key: &str, query: &str, score_value: f32) -> Scored {
        Scored {
            key: key.to_owned(),
            query: query.to_owned(),
            dimensions: dimensions(3000, 3000),
            metrics: metrics(6.0, 0.05, Some(0.1)),
            score: score_value,
        }
    }

    #[test]
    fn every_hard_filter_names_itself() {
        let filters = Filters::default();
        let legibility = Legibility::default();
        let judge = |width, decoded_width, metrics| {
            score(
                "k".to_owned(),
                "q".to_owned(),
                dimensions(width, decoded_width),
                metrics,
                &filters,
                &legibility,
            )
            .expect_err("should be rejected")
            .1
        };

        assert_eq!(
            judge(800, 4000, metrics(6.0, 0.05, Some(0.1))),
            Rejection::TooSmall
        );
        // The case the two gates exist to tell apart: a provider advertising a 5151-pixel original
        // and serving a 1280-pixel copy of it. The metadata clears `min_source_width`; the file does
        // not clear `min_decoded_width`, and the rejection has to say which of the two it was.
        assert_eq!(
            judge(5151, 800, metrics(6.0, 0.05, Some(0.1))),
            Rejection::TooSmallOnDisk
        );
        assert_eq!(
            judge(3000, 4000, metrics(1.0, 0.05, Some(0.1))),
            Rejection::LowEntropy
        );
        assert_eq!(
            judge(3000, 4000, metrics(6.0, 0.9, Some(0.1))),
            Rejection::TooBusy
        );
        assert_eq!(
            judge(3000, 4000, metrics(6.0, 0.05, None)),
            Rejection::ContrastUnreachable
        );
    }

    #[test]
    fn a_calmer_image_that_needs_less_darkening_scores_higher() {
        let filters = Filters::default();
        let legibility = Legibility::default();
        let judge = |busyness, required| {
            score(
                "k".to_owned(),
                "q".to_owned(),
                dimensions(4000, 4000),
                metrics(6.0, busyness, Some(required)),
                &filters,
                &legibility,
            )
            .expect("passes")
            .score
        };
        assert!(judge(0.02, 0.05) > judge(0.15, 0.05), "calm beats busy");
        assert!(
            judge(0.05, 0.05) > judge(0.05, 0.44),
            "less scrim beats more"
        );
        // And the weights are a partition of one, so a score is a fraction.
        assert!((0.0..=1.0).contains(&judge(0.02, 0.0)));
    }

    #[test]
    fn one_search_term_cannot_fill_the_pack() {
        let filters = Filters {
            max_per_query: 2,
            ..Filters::default()
        };
        let candidates = vec![
            scored("a", "sunset", 0.9),
            scored("b", "sunset", 0.8),
            scored("c", "sunset", 0.7),
            scored("d", "forest", 0.6),
        ];
        let selection = select(candidates, &filters, 10);
        assert_eq!(
            selection
                .chosen
                .iter()
                .map(|s| s.key.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "d"]
        );
        assert_eq!(selection.rejected.len(), 1);
        assert_eq!(
            selection.rejected[0].1,
            Rejection::QueryFull("sunset".to_owned())
        );
    }

    #[test]
    fn the_pack_stops_at_the_target_and_says_so() {
        let candidates = vec![
            scored("a", "one", 0.9),
            scored("b", "two", 0.8),
            scored("c", "three", 0.7),
        ];
        let selection = select(candidates, &Filters::default(), 2);
        assert_eq!(selection.chosen.len(), 2);
        assert_eq!(selection.rejected[0].1, Rejection::PackFull);
        assert_eq!(selection.histogram().get("pack_full"), Some(&1));
    }

    #[test]
    fn a_tie_is_broken_by_key_so_two_runs_agree() {
        let one = select(
            vec![scored("b", "q", 0.5), scored("a", "q", 0.5)],
            &Filters::default(),
            10,
        );
        let other = select(
            vec![scored("a", "q", 0.5), scored("b", "q", 0.5)],
            &Filters::default(),
            10,
        );
        assert_eq!(
            one.chosen.iter().map(|s| s.key.clone()).collect::<Vec<_>>(),
            other
                .chosen
                .iter()
                .map(|s| s.key.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_histogram_counts_kinds_rather_than_details() {
        let selection = Selection {
            chosen: Vec::new(),
            rejected: vec![
                ("a".to_owned(), Rejection::DuplicateOf("x".to_owned())),
                ("b".to_owned(), Rejection::DuplicateOf("y".to_owned())),
                ("c".to_owned(), Rejection::TooBusy),
            ],
        };
        let histogram = selection.histogram();
        assert_eq!(histogram.get("duplicate_of"), Some(&2));
        assert_eq!(histogram.get("too_busy"), Some(&1));
        // The detail survives where it is useful: in the per-image label.
        assert_eq!(
            Rejection::DuplicateOf("pixabay:7".to_owned()).label(),
            "duplicate_of:pixabay:7"
        );
    }

    /// A label reduces to the kind the enum would have named, for every variant.
    ///
    /// **This is what lets the two live apart.** `kind` is for a caller holding the variant and
    /// `kind_of_label` for one holding only what `analysis.json` recorded, and they answer about the
    /// same rejection — so a variant whose label stopped starting with its own kind would have the
    /// two counting different things in different programs.
    #[test]
    fn kind_and_label_are_one_vocabulary() {
        let detail = "pixabay:7".to_owned();
        let every = [
            Rejection::TooSmall,
            Rejection::TooSmallOnDisk,
            Rejection::LowEntropy,
            Rejection::TooBusy,
            Rejection::ContrastUnreachable,
            Rejection::Unmeasurable("undecodable"),
            Rejection::DuplicateOf(detail.clone()),
            Rejection::QueryFull("mountains at dusk".to_owned()),
            Rejection::PackFull,
            Rejection::NotRedistributable("pixabay".to_owned()),
        ];
        for rejection in &every {
            let label = rejection.label();
            assert_eq!(
                Rejection::kind_of_label(&label),
                rejection.kind(),
                "{label} does not reduce to its own kind"
            );
        }
    }
}
