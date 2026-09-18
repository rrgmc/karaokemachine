//! The config file, and the rules about what a valid one is.
//!
//! Two decisions worth stating, because both are refusals:
//!
//! * **API keys never come from the file.** They are read from the environment, and a key-shaped
//!   entry in the TOML is a hard error rather than a warning — a config with a key in it is a config
//!   somebody will commit, and the error is the only thing that stops that happening quietly.
//! * **Validation happens once, at load.** Every field is checked here so that the rest of the tool
//!   can treat the config as already sane. A band that is upside down or a quality of 400 should
//!   fail before a single request is made, not three thousand downloads in.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::license::License;

/// The whole configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// What to write.
    #[serde(default)]
    pub output: Output,
    /// The legibility gate.
    #[serde(default)]
    pub legibility: Legibility,
    /// What to reject before legibility is even considered.
    #[serde(default)]
    pub filters: Filters,
    /// What to search for, per provider.
    #[serde(default)]
    pub queries: Vec<QueryGroup>,
}

/// Every setting at its default, and **no queries**.
///
/// For `commands::local`, which curates a folder somebody already has rather than searching for
/// one — so it needs the output and legibility settings and has nothing to ask a provider. Those
/// defaults are not arbitrary: [`Legibility`]'s are read off the app's own `Theme` and
/// `WallpaperConfig`, which is exactly what a hand-picked shipped set has to be measured against.
///
/// Deliberately not reachable through [`Config::parse`], whose [`Config::validate`] refuses an empty
/// `queries` — a config *file* with nothing to search for is a mistake, and this is not a file.
impl Default for Config {
    fn default() -> Self {
        Self {
            output: Output::default(),
            legibility: Legibility::default(),
            filters: Filters::default(),
            queries: Vec::new(),
        }
    }
}

/// Output settings.
//
// `default` at the container level, not only on the field that holds this: a config that sets one
// key of a table must get the app-shaped defaults for the rest, and without this serde asks for
// every field of any table that is mentioned at all.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Output {
    /// Sizes to write, each `WIDTHxHEIGHT`.
    pub sizes: Vec<Size>,
    /// Encoded format.
    pub format: Format,
    /// JPEG quality, 1..=100. Ignored for WebP.
    pub jpeg_quality: u8,
    /// Stop selecting once the pack holds this many images.
    pub target_count: usize,
    /// Whether to zip the pack when it is built.
    pub zip: bool,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            // One size by default. The app resizes every wallpaper to the display before uploading
            // it, so a 4K copy buys nothing on a 1080p panel and costs four times the bytes; on a 4K
            // panel it buys real sharpness. Hence opt-in rather than default.
            sizes: vec![Size {
                width: 1920,
                height: 1080,
            }],
            format: Format::Jpeg,
            jpeg_quality: 82,
            target_count: 120,
            zip: true,
        }
    }
}

/// An output size.
///
/// Ordered so that a map keyed by size has a stable iteration order, which is what keeps a build's
/// per-size reporting in the same order every run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Size {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Size {
    /// The aspect ratio, width over height.
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

impl std::str::FromStr for Size {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let (w, h) = value.split_once(['x', 'X']).ok_or_else(|| {
            Error::config(format!("`{value}` is not a size; write it as 1920x1080"))
        })?;
        let width = w
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("`{w}` is not a width")))?;
        let height = h
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("`{h}` is not a height")))?;
        if width == 0 || height == 0 {
            return Err(Error::config(format!("`{value}` has a zero side")));
        }
        Ok(Self { width, height })
    }
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl Serialize for Size {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Encoded output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// JPEG at the configured quality.
    Jpeg,
    /// Lossless-ish WebP.
    WebP,
}

impl Format {
    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
        }
    }
}

/// An sRGB color with channels in 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    /// Red, 0..1.
    pub r: f32,
    /// Green, 0..1.
    pub g: f32,
    /// Blue, 0..1.
    pub b: f32,
}

impl Rgb {
    /// Pure black.
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };
    /// Pure white.
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };

    /// Parses `#RRGGBB`.
    pub fn from_hex(text: &str) -> Result<Self> {
        let hex = text.trim().trim_start_matches('#');
        if hex.len() != 6 {
            return Err(Error::config(format!(
                "`{text}` is not a color; write it as #ECEFF4"
            )));
        }
        let channel = |at: usize| -> Result<f32> {
            u8::from_str_radix(&hex[at..at + 2], 16)
                .map(|value| f32::from(value) / 255.0)
                .map_err(|_| Error::config(format!("`{text}` is not a color")))
        };
        Ok(Self {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
        })
    }

    /// Back to `#RRGGBB`.
    pub fn to_hex(self) -> String {
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!(
            "#{:02X}{:02X}{:02X}",
            byte(self.r),
            byte(self.g),
            byte(self.b)
        )
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_hex(&text).map_err(serde::de::Error::custom)
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// The legibility gate.
///
/// The defaults are read off the app rather than chosen here, and the pairing is deliberate: see
/// `Theme::lyric_band` and `WallpaperConfig::default().dim` in `km-display`, and the test there named
/// `the_lyric_band_is_what_the_wallpaper_tool_measures`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Legibility {
    /// Top of the lyric band, as a fraction of image height.
    pub band_top: f32,
    /// Bottom of the lyric band, as a fraction of image height.
    pub band_bottom: f32,
    /// The lyric color contrast is measured against.
    pub text_color: Rgb,
    /// The darkening the app itself applies, which is what an image is judged at.
    pub assumed_dim: f32,
    /// The WCAG contrast ratio an image must reach.
    pub target_contrast: f32,
    /// Gaussian blur applied before measuring and before writing.
    pub blur_sigma: f32,
    /// Whether to darken the corners.
    pub vignette: bool,
}

impl Default for Legibility {
    fn default() -> Self {
        Self {
            // km-display draws its two lyric rows at 0.42 and 0.55 of screen height, each about
            // 0.085 tall. 0.40..0.68 is that span with a little margin either side.
            band_top: 0.40,
            band_bottom: 0.68,
            // Theme::lyric_pending. Near-white on purpose: pure white glares over a bright photo.
            text_color: Rgb {
                r: 0.925,
                g: 0.937,
                b: 0.957,
            },
            // WallpaperConfig::default().dim.
            assumed_dim: 0.45,
            // AAA rather than AA. Lyrics are read at a distance, in motion, by somebody who is
            // singing rather than studying, and the cost of aiming high is a smaller pack.
            target_contrast: 7.0,
            blur_sigma: 2.5,
            vignette: true,
        }
    }
}

impl Legibility {
    /// The band as pixel rows of an image `height` tall.
    pub fn band_rows(&self, height: u32) -> (u32, u32) {
        let top = (self.band_top * height as f32).round().max(0.0) as u32;
        let bottom = (self.band_bottom * height as f32)
            .round()
            .min(height as f32) as u32;
        (top.min(height), bottom.min(height))
    }
}

/// Rejections that have nothing to do with contrast.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Filters {
    /// Smallest acceptable source width, as the *provider* describes the original photograph.
    ///
    /// A search-time knob: it is handed to the provider as its own `min_width`
    /// (see `crate::commands::fetch`), so it decides which photographs are ever asked for. It says
    /// nothing about the file that comes back — see [`Filters::min_decoded_width`].
    pub min_source_width: u32,
    /// Smallest acceptable width of the file actually downloaded.
    ///
    /// Separate from [`Filters::min_source_width`] because the two are different quantities, and
    /// conflating them hid a real defect for a month: Pixabay's `largeImageURL` is capped at 1280
    /// pixels, so a 2400 threshold read off the provider's metadata passed 4,682 files that were all
    /// 1280 wide, and the pack was silently upscaled 3x to 4K.
    ///
    /// Measured on the decoded image, which is the only number that describes the bytes going into
    /// the pack. Defaults low enough not to reject a corpus that is already downloaded: it is a floor
    /// against genuinely tiny files, not a sharpness gate.
    pub min_decoded_width: u32,
    /// Least acceptable entropy, in bits.
    pub min_entropy: f32,
    /// Most acceptable gradient magnitude inside the lyric band.
    pub max_band_busyness: f32,
    /// Perceptual-hash distance at or below which two images are the same photograph.
    pub phash_distance: u32,
    /// Most images any single search term may contribute.
    pub max_per_query: usize,
    /// Keep only images whose license grants redistribution.
    ///
    /// **Default `false`, and the reason is where packs land.** `--zip-dest` writes to
    /// `local/assets/wallpapers`, which is gitignored and which no staging script can see, so an
    /// ordinary pack goes nowhere but the machine that built it — which is what the tool is for, and
    /// what the wider filter would get in the way of.
    ///
    /// Turn it on when the pack is for somebody else. What it changes is that a non-redistributable
    /// image is *rejected and counted* rather than silently included — it appears in the histogram
    /// as `not_redistributable`, like every other reason an image did not make it.
    pub redistributable_only: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            min_source_width: 2400,
            min_decoded_width: 1280,
            min_entropy: 4.0,
            max_band_busyness: 0.16,
            phash_distance: 10,
            // Sixteen rather than twelve: the eight terms in the example config would otherwise cap
            // a pack at 96 and `target_count` could never bind.
            max_per_query: 16,
            redistributable_only: false,
        }
    }
}

/// Which provider to ask, and for what.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryGroup {
    /// The provider.
    pub provider: ProviderKind,
    /// Search terms.
    pub terms: Vec<String>,
    /// Pages of results to ask for per term.
    pub pages: u32,
}

/// A stock provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// pixabay.com
    Pixabay,
    /// pexels.com
    Pexels,
    /// openverse.org — an **aggregator** rather than a stock library. Its images come from Wikimedia
    /// Commons, Flickr, museum open collections and others, each under its own license, which is why
    /// [`ProviderKind::whole_site_license`] answers `None` for it.
    Openverse,
}

impl ProviderKind {
    /// The name used in paths, manifests and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pixabay => "pixabay",
            Self::Pexels => "pexels",
            Self::Openverse => "openverse",
        }
    }

    /// The environment variable its key comes from.
    pub fn key_var(self) -> &'static str {
        match self {
            Self::Pixabay => "PIXABAY_API_KEY",
            Self::Pexels => "PEXELS_API_KEY",
            // **Optional, alone among the three**: Openverse answers without one, inside three
            // separate caps. See `Openverse::new`.
            Self::Openverse => "OPENVERSE_API_TOKEN",
        }
    }

    /// Where somebody goes to get a key.
    ///
    /// **Here rather than inline in each constructor, which is where it was.** Two of the three were
    /// written into an `Error::MissingKey` and the third was nowhere at all, so the answer to "where
    /// do I get one?" existed only in the message you saw after failing. A program with a form has to
    /// ask the question *before* that, so it needs the same fact in front.
    pub fn key_page(self) -> &'static str {
        match self {
            Self::Pixabay => "pixabay.com/api/docs/",
            Self::Pexels => "pexels.com/api/",
            Self::Openverse => "api.openverse.org/v1/#tag/auth",
        }
    }

    /// Whether a search needs a key at all.
    ///
    /// **False for exactly one, and the asymmetry is a product fact.** Openverse is the source whose
    /// packs may be redistributed, and it answers anonymously, so it is the one a person can use
    /// having installed nothing and signed up to nothing — which is why it is the default. Anonymous is capped three ways; see `Openverse::new`.
    pub fn needs_key(self) -> bool {
        !matches!(self, Self::Openverse)
    }

    /// The license **every** image from this source carries, when there is one.
    ///
    /// `None` for a source that aggregates many, which is not a gap to be filled in later: it is the
    /// whole reason this replaced `license() -> &'static str`. That signature could not describe an
    /// aggregator, and a type that cannot represent the truth is how a question stops being asked —
    /// nobody looked up what a pack may be *used for* until the constant became impossible to keep.
    /// Returning a `"varies"` string instead would have kept the broken model alive behind a value.
    ///
    /// Both providers here answer `Some`, because both really do have one license each. The return
    /// type is `Option` for the source that does not, and the match is exhaustive, so the next
    /// provider is **forced** to state a position here at compile time. One caller:
    /// [`crate::providers::RemoteImage::license`], which back-fills records cached before licenses
    /// were per image.
    pub fn whole_site_license(self) -> Option<License> {
        match self {
            Self::Pixabay => Some(License::site_terms("pixabay", "Pixabay Content License")),
            Self::Pexels => Some(License::site_terms("pexels", "Pexels License")),
            // The aggregator this return type exists for: its images arrive under many licenses, so
            // there is no answer here and each record carries its own.
            Self::Openverse => None,
        }
    }
}

impl ProviderKind {
    /// Every provider, for the messages that must not silently omit one.
    ///
    /// Adding a variant breaks [`ProviderKind::as_str`], [`ProviderKind::key_var`],
    /// [`ProviderKind::whole_site_license`] and the one runtime match in `commands::fetch` — the
    /// compiler finds all four. It does **not** break the secret-key error message below, which
    /// named two providers by hand and would have quietly left a third out. That message is the only
    /// thing standing between an API key and a committed config file, so it is built from this.
    pub const ALL: [ProviderKind; 3] = [Self::Pixabay, Self::Pexels, Self::Openverse];
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Words that must never appear as keys in the config file.
///
/// Checked against the raw text rather than the parsed structure, because `deny_unknown_fields`
/// already rejects an unknown key with a message about the *schema* — and "unknown field `api_key`"
/// is the wrong thing to say to somebody who has just pasted a secret into a file they are about to
/// commit.
const SECRET_KEYS: [&str; 5] = ["api_key", "apikey", "key", "token", "secret"];

impl Config {
    /// Loads and validates a config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&text)
    }

    /// Parses and validates config text.
    pub fn parse(text: &str) -> Result<Self> {
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                continue;
            }
            if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim().trim_matches('"').to_ascii_lowercase();
                if SECRET_KEYS.contains(&key.as_str()) {
                    // **Built from `ProviderKind::ALL` rather than naming providers by hand.** This
                    // message is the only thing standing between an API key and a committed config
                    // file, and it is the one place a new provider does *not* break the build --
                    // adding a variant made three matches and one `fetch` arm fail to compile, and
                    // left this silently naming two of three.
                    let vars: Vec<&str> = ProviderKind::ALL
                        .iter()
                        .map(|provider| provider.key_var())
                        .collect();
                    return Err(Error::config(format!(
                        "`{key}` in the config file: API keys are read from {} instead, so that a \
                         config file is safe to commit. Remove the line.",
                        vars.join(", "),
                    )));
                }
            }
        }

        let config: Self =
            toml::from_str(text).map_err(|source| Error::config(source.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Every rule the rest of the tool is allowed to assume.
    pub fn validate(&self) -> Result<()> {
        let legibility = &self.legibility;
        if !(0.0..1.0).contains(&legibility.band_top)
            || !(0.0..=1.0).contains(&legibility.band_bottom)
            || legibility.band_bottom <= legibility.band_top
        {
            return Err(Error::config(format!(
                "the lyric band must run down the image: band_top {} band_bottom {}",
                legibility.band_top, legibility.band_bottom
            )));
        }
        if !(0.0..=1.0).contains(&legibility.assumed_dim) {
            return Err(Error::config(
                "assumed_dim is an alpha, so it lies between 0 and 1".to_owned(),
            ));
        }
        if !(1.0..=21.0).contains(&legibility.target_contrast) {
            return Err(Error::config(
                "target_contrast is a WCAG ratio, so it lies between 1 and 21".to_owned(),
            ));
        }
        if legibility.blur_sigma < 0.0 {
            return Err(Error::config("blur_sigma cannot be negative".to_owned()));
        }
        if self.output.sizes.is_empty() {
            return Err(Error::config("no output sizes".to_owned()));
        }
        if !(1..=100).contains(&self.output.jpeg_quality) {
            return Err(Error::config("jpeg_quality runs from 1 to 100".to_owned()));
        }
        if self.output.target_count == 0 {
            return Err(Error::config(
                "target_count of zero builds nothing".to_owned(),
            ));
        }
        if self.filters.max_per_query == 0 {
            return Err(Error::config(
                "max_per_query of zero admits nothing".to_owned(),
            ));
        }
        // A Hamming threshold is only meaningful as a fraction of the hash. Two unrelated hashes sit
        // PHASH_BITS/2 bits apart on average, so a distance approaching a third of the width starts
        // matching photographs that have nothing to do with each other — and does it silently, by
        // shipping a small pack rather than by failing. A 22-bit hash against a threshold of 10 once
        // collapsed 1,608 candidates into 6.
        let widest = crate::metrics::PHASH_BITS / 3;
        if self.filters.phash_distance as usize >= widest {
            return Err(Error::config(format!(
                "phash_distance {} is too loose for a {}-bit hash: at {} bits and beyond, unrelated \
                 photographs match. Use something near 10.",
                self.filters.phash_distance,
                crate::metrics::PHASH_BITS,
                widest
            )));
        }
        if self.queries.is_empty() {
            return Err(Error::config(
                "no queries: nothing to search for".to_owned(),
            ));
        }
        for group in &self.queries {
            if group.terms.is_empty() {
                return Err(Error::config(format!(
                    "{} has no search terms",
                    group.provider
                )));
            }
            if group.pages == 0 {
                return Err(Error::config(format!(
                    "{} asks for zero pages",
                    group.provider
                )));
            }
        }
        Ok(())
    }

    /// The providers this config actually needs keys for, in a stable order.
    pub fn providers(&self) -> Vec<ProviderKind> {
        let mut providers: Vec<_> = self.queries.iter().map(|group| group.provider).collect();
        providers.sort_unstable();
        providers.dedup();
        providers
    }

    /// A hash of the config, so a pack can say which settings produced it.
    ///
    /// Over the *parsed and re-serialized* config rather than the file's bytes: a comment or a
    /// reordered table is not a different pack, and a hash that says otherwise would make the
    /// determinism test meaningless.
    pub fn hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let canonical = toml::to_string(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// A hash of only what one image's *measurement* depends on: the first output size and the whole
    /// `[legibility]` table.
    ///
    /// Deliberately narrower than [`Config::hash`], and that is the entire point of it. Adding a
    /// search term changes the pack, so it must change `hash`; it changes nothing whatsoever about
    /// how any one photograph is measured, and a measurement cache keyed on the whole config would
    /// throw away four thousand answers that are still correct. The two are read by different things
    /// for different reasons — `hash` guards `analysis.json` against a stale config, this guards
    /// `metrics.jsonl` against a stale measurement — and they are not interchangeable.
    ///
    /// What goes in is exactly what `crate::commands::measure_original` reads, and nothing else:
    /// `output.sizes[0]`, because every measurement is taken on the picture rendered at that size,
    /// and `legibility`, because it decides the crop band, the blur, the vignette and the contrast
    /// solver. `filters` is deliberately absent — it is applied by `crate::select::score` afterwards,
    /// on numbers already measured, so a threshold can be re-tuned without re-decoding anything.
    /// That is the case the cache exists for.
    ///
    /// Built the same way as [`Config::hash`] — over re-serialized values rather than the file's
    /// bytes — so a comment or a reordered table is not a cache miss.
    pub fn measurement_hash(&self) -> String {
        use sha2::{Digest, Sha256};

        #[derive(Serialize)]
        struct Measured<'a> {
            size: Option<Size>,
            legibility: &'a Legibility,
        }

        // `Size` serializes as a string and `Legibility` as a table, and serde emits fields in
        // declaration order, so this is valid TOML: values before tables.
        let canonical = toml::to_string(&Measured {
            size: self.output.sizes.first().copied(),
            legibility: &self.legibility,
        })
        .unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
[[queries]]
provider = "pixabay"
terms = ["mountain lake dusk"]
pages = 1
"#;

    #[test]
    fn a_minimal_config_gets_the_apps_own_defaults() {
        let config = Config::parse(MINIMAL).expect("parse");
        assert_eq!(config.legibility.assumed_dim, 0.45, "the app's scrim");
        assert_eq!(config.legibility.target_contrast, 7.0);
        assert_eq!(config.legibility.text_color.to_hex(), "#ECEFF4");
        assert_eq!(config.output.sizes.len(), 1, "4K is opt-in");
        assert_eq!(config.providers(), vec![ProviderKind::Pixabay]);
    }

    #[test]
    fn a_key_in_the_file_is_refused_by_name() {
        let text = format!("api_key = \"deadbeef\"\n{MINIMAL}");
        let error = Config::parse(&text).expect_err("must refuse");
        let message = error.to_string();
        assert!(message.contains("PIXABAY_API_KEY"), "{message}");
        assert!(
            message.contains("safe to commit"),
            "the message has to say why: {message}"
        );
    }

    #[test]
    fn a_commented_out_key_is_not_a_key() {
        let text = format!("# api_key = \"see the README\"\n{MINIMAL}");
        assert!(Config::parse(&text).is_ok());
    }

    #[test]
    fn an_upside_down_band_is_refused_before_anything_is_downloaded() {
        let text = format!("[legibility]\nband_top = 0.9\nband_bottom = 0.2\n{MINIMAL}");
        let error = Config::parse(&text).expect_err("must refuse");
        assert!(error.to_string().contains("band"), "{error}");
    }

    /// A Hamming threshold is meaningless without the width it is measured against.
    ///
    /// The hash was 22 bits wide against a distance of 10, so unrelated photographs matched 42% of
    /// the time and the failure showed up as a small pack rather than as an error. Nothing about a
    /// distance of 10 looks wrong on its own, which is why the check compares it to the width.
    #[test]
    fn a_phash_distance_too_loose_for_the_hash_is_refused() {
        let text = format!("[filters]\nphash_distance = 24\n{MINIMAL}");
        let error = Config::parse(&text).expect_err("must refuse");
        assert!(error.to_string().contains("phash_distance"), "{error}");

        // And the value the tool actually ships with is comfortably inside the limit.
        let fine = format!("[filters]\nphash_distance = 10\n{MINIMAL}");
        Config::parse(&fine)
            .expect("ten bits of sixty-four is a near-duplicate, not a coincidence");
    }

    #[test]
    fn a_config_with_no_queries_is_refused() {
        let error = Config::parse("[output]\ntarget_count = 10\n").expect_err("must refuse");
        assert!(error.to_string().contains("queries"), "{error}");
    }

    #[test]
    fn sizes_round_trip_through_their_written_form() {
        let size: Size = "3840x2160".parse().expect("parse");
        assert_eq!(size.width, 3840);
        assert_eq!(size.to_string(), "3840x2160");
        assert!((size.aspect() - 16.0 / 9.0).abs() < 0.001);
        assert!("1920".parse::<Size>().is_err(), "a size needs both sides");
        assert!(
            "0x1080".parse::<Size>().is_err(),
            "a zero side is not a size"
        );
    }

    #[test]
    fn the_band_in_pixels_covers_the_rows_the_app_draws_lyrics_on() {
        let legibility = Legibility::default();
        let (top, bottom) = legibility.band_rows(1080);
        // km-display draws its rows at 0.42 and 0.55 of the height.
        assert!(top <= (0.42 * 1080.0) as u32);
        assert!(
            bottom >= (0.55 * 1080.0) as u32 + 92,
            "plus a line of glyphs"
        );
    }

    #[test]
    fn the_hash_ignores_comments_and_notices_settings() {
        let plain = Config::parse(MINIMAL).expect("parse");
        let commented = Config::parse(&format!("# a note\n{MINIMAL}")).expect("parse");
        assert_eq!(
            plain.hash(),
            commented.hash(),
            "a comment is not a different pack"
        );

        let changed = Config::parse(&format!("[legibility]\ntarget_contrast = 4.5\n{MINIMAL}"))
            .expect("parse");
        assert_ne!(plain.hash(), changed.hash(), "a threshold is");
    }
}
