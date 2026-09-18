//! What the Pictures section is currently set to look for.
//!
//! **A `km_wallpaper_pack::Config` built from a form rather than parsed from a file.** The tool's own
//! `config.toml` is a document a packager edits; this is a handful of fields somebody sets on a page,
//! and the two want different things. What is shared is the *type*: everything below produces a real
//! `Config`, so the pipeline sees exactly what it would see from a file and nothing here is a second
//! interpretation of a threshold.
//!
//! **Defaults come from `Config::default()`**, which is where the numbers argued for in
//! `docs/architecture/assets.md` already live — the 0.40–0.68 band the lyrics occupy, the `#ECEFF4`
//! they are drawn in, the 0.45 scrim the display lays over every wallpaper. Re-typing any of them
//! here would be a second copy of a number that was measured once.

use std::path::{Path, PathBuf};

use km_wallpaper_pack::Config;
use km_wallpaper_pack::config::{ProviderKind, QueryGroup};
use serde::{Deserialize, Serialize};

/// What this program remembers about a search, between runs.
///
/// **Never a key.** Those are `crate::keys`' business and live in a different file, so that
/// remembering a search term and remembering a credential can never become one decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// What to call the pack this search builds, as a slug, or `None` for the plain name.
    ///
    /// **A label on the output, and deliberately not part of the search.** It is not in
    /// [`Settings::to_config`] and therefore not in `Config::hash`, which is what keeps a rename from
    /// invalidating `analysis.json` and asking somebody to measure four thousand photographs again
    /// because they changed a word. The name is applied where a built pack is *kept* — see
    /// `crate::run::kept_name` — which is the one place a pack acquires a lasting identity.
    ///
    /// Stored already slugged, by [`slug`], because this becomes a folder name, a file name and a
    /// path inside a route. What somebody typed is not kept: the slug is shown back to them in the
    /// field, so the rule is visible rather than described.
    ///
    /// `serde(default)` is not optional — every `settings.json` written before this existed has no
    /// such key, and without it this program would start with its defaults and quietly forget a
    /// packager's search terms.
    #[serde(default)]
    pub name: Option<String>,
    /// Which provider to search.
    pub provider: ProviderKind,
    /// What to search for, one term per line as the page shows it.
    pub terms: Vec<String>,
    /// Pages of results per term.
    pub pages: u32,
    /// How many pictures the finished pack should hold.
    pub target_count: usize,
    /// The contrast an image must reach to be kept.
    pub target_contrast: f32,
    /// Smallest width to *ask a provider* for, as it describes the photograph.
    #[serde(default = "default_min_source_width")]
    pub min_source_width: u32,
    /// Where this program's log goes, how much detail is in it, and how many runs are kept.
    ///
    /// **The one section here that is about a run rather than about a search**, and it is here
    /// because a program with a window is one people start from an icon, where there is no command
    /// line to be told on.
    ///
    /// Read by [`peek_logging`] before the subscriber exists, and carried on the struct as well so
    /// that saving a search cannot drop it — this file is written whole.
    #[serde(default)]
    pub logging: km_logsettings::LoggingSettings,
    /// Smallest width the *downloaded file* may actually be.
    ///
    /// **On the page, and it has to be**, because these two are different quantities and the second
    /// is the one that surprises people. A provider's reported width is not a promise about the file
    /// it serves: Pixabay's `largeImageURL` is capped at 1280 whatever the photograph really is, and
    /// a first real run against Openverse here downloaded 39 pictures it had asked for at 2400 wide
    /// and rejected every one of them as `too_small_on_disk`. That is the gate working — the pack is
    /// 1920 wide and upscaling is not a fix — but a threshold nobody can see is a search that
    /// silently returns nothing.
    #[serde(default = "default_min_decoded_width")]
    pub min_decoded_width: u32,
}

/// The provider-side width gate's default, from the pipeline's own.
fn default_min_source_width() -> u32 {
    km_wallpaper_pack::Config::default()
        .filters
        .min_source_width
}

/// The file-side width gate's default, from the pipeline's own.
fn default_min_decoded_width() -> u32 {
    km_wallpaper_pack::Config::default()
        .filters
        .min_decoded_width
}

impl Default for Settings {
    fn default() -> Self {
        let config = Config::default();
        Self {
            // **Nothing saved yet, rather than a name declined.** A search will not run without
            // one — see `ready` — and nothing here suggests what it should be: a suggested name is
            // the name every pack on every machine ends up with.
            name: None,
            // Openverse, because it needs no account and its packs may be passed on.
            provider: ProviderKind::Openverse,
            terms: DEFAULT_TERMS
                .iter()
                .map(|term| (*term).to_owned())
                .collect(),
            pages: 2,
            target_count: config.output.target_count,
            target_contrast: config.legibility.target_contrast,
            min_source_width: config.filters.min_source_width,
            min_decoded_width: config.filters.min_decoded_width,
            logging: km_logsettings::LoggingSettings::default(),
        }
    }
}

/// What the page opens with in the search box.
///
/// **Calm, dark, and empty in the middle** — which is not a taste but the brief: two lines of lyrics
/// are drawn across the center of these for three minutes at a time, and the gate rejects anything
/// busy enough to compete with letter shapes. Somebody who wants cities and crowds can type them and
/// watch most of them be rejected, which is a more useful lesson than a shorter list would be.
const DEFAULT_TERMS: &[&str] = &[
    "night sky stars",
    "calm lake dusk",
    "misty forest dawn",
    "desert dunes evening",
    "ocean horizon twilight",
    "snow field blue hour",
];

impl Settings {
    /// The terms as the page shows them: one per line.
    pub fn terms_text(&self) -> String {
        self.terms.join("\n")
    }

    /// Reads a textarea back into terms, dropping the blank lines somebody leaves behind.
    pub fn set_terms(&mut self, text: &str) {
        self.terms = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
    }

    /// The `Config` the pipeline is driven with.
    ///
    /// **Everything not on the page comes from `Config::default()`**, so the band, the text color
    /// and the assumed scrim are the app's own measured numbers rather than a copy of them.
    pub fn to_config(&self) -> Config {
        let mut config = Config::default();
        config.output.target_count = self.target_count;
        config.legibility.target_contrast = self.target_contrast;
        config.filters.min_source_width = self.min_source_width;
        config.filters.min_decoded_width = self.min_decoded_width;
        config.queries = vec![QueryGroup {
            provider: self.provider,
            terms: self.terms.clone(),
            pages: self.pages,
        }];
        config
    }

    /// Whether this can be run at all, and why not.
    ///
    /// **A reason rather than a sentence.** No locale is in reach here and the handler that answers
    /// has one, so what comes back names the fault and the page words it.
    pub fn ready(&self, has_key: bool) -> Result<(), NotReady> {
        // **A pack is named before it is built, not after.** What a pack is *for* lives in its name
        // and nowhere else: two searches an hour apart leave two files differing by eight hex
        // characters, and nothing on this computer can then say which was the one for the beach
        // party. The name is asked for where somebody still knows the answer.
        if self.name.as_deref().unwrap_or_default().is_empty() {
            return Err(NotReady::NoName);
        }
        if self.terms.is_empty() {
            return Err(NotReady::NoTerms);
        }
        if self.provider.needs_key() && !has_key {
            return Err(NotReady::NeedsKey(self.provider.as_str()));
        }
        Ok(())
    }
}

/// Why a search cannot start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotReady {
    /// The pack has not been given a name.
    NoName,
    /// Nothing has been typed to search for.
    NoTerms,
    /// The chosen provider answers nobody without a key, and none has been given.
    NeedsKey(&'static str),
}

/// Turns what somebody typed into something that can be a file name, a folder name and a URL segment.
///
/// Fold case, one dash for any run of spaces and dashes, and drop everything that is not an ASCII
/// letter, a digit, a dot or an underscore. `None` when nothing survives, which is how an empty box
/// and a box holding only punctuation both mean *no name*.
///
/// **"Nothing survives" means no letter and no digit, not an empty string, and a test found that the
/// hard way.** A dot is a legal character in a file name, so keeping it is right — but `..` is then
/// made of nothing but legal characters and is still the parent directory. It reaches
/// [`pack_dir`], which refuses it, and a name that is silently refused later is worse than one
/// refused here. Requiring one alphanumeric answers `.`, `..`, `...` and `___` with the same rule
/// rather than with a list of the shapes somebody thought of.
///
/// Only ASCII letters are kept, deliberately: `Praias do Sul` becomes `praias-do-sul` and
/// `Músicas Brasileiras` becomes `msicas-brasileiras`, which is ugly and unambiguous where
/// transliterating would be a table of somebody's opinions about other people's alphabets. The slug
/// is written back into the field, so the rule is *shown* rather than described.
///
/// **This is the third copy of this rule in the repository and the first two cannot be reached from
/// here.** `km-package-builder`'s `slug` is across the `tools/cmd/assets/` workspace exclusion
/// boundary, and `karaokemachine`'s `bank_id` is `pub(crate)` in a crate that links SDL3. It is
/// spelled character-for-character like the package builder's so the two stay comparable by reading;
/// see `A picture pack may be named` in docs/decisions/packaging.md, which says why a fourth crate to
/// share nine lines was not the answer.
pub fn slug(name: &str) -> Option<String> {
    let mut slug = String::new();
    for character in name.trim().to_lowercase().chars() {
        match character {
            'a'..='z' | '0'..='9' | '.' | '_' => slug.push(character),
            ' ' | '-' if !slug.ends_with('-') => slug.push('-'),
            _ => {}
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    slug.chars()
        .any(|character| character.is_ascii_alphanumeric())
        .then_some(slug)
}

/// Where the settings are written.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

/// What the settings file says about logging, without reading the rest of it.
///
/// **Before [`load`] rather than through it**, because the subscriber has to exist before anything
/// can be said and the whole struct drags every other default into a question about one section.
#[must_use]
pub fn peek_logging(data_dir: &Path) -> km_logsettings::LoggingSettings {
    km_logsettings::peek(settings_path(data_dir))
}

/// The remembered settings, or the defaults.
pub fn load(data_dir: &Path) -> Settings {
    std::fs::read_to_string(settings_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes the settings down.
///
/// A failure is a warning rather than a refusal: not remembering a search term is a smaller problem
/// than not being able to search.
pub fn save(data_dir: &Path, settings: &Settings) {
    let write = || -> Result<(), String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|error| format!("{} cannot be written to: {error}", data_dir.display()))?;
        let text = serde_json::to_string_pretty(settings)
            .map_err(|error| format!("could not write the settings: {error}"))?;
        std::fs::write(settings_path(data_dir), text)
            .map_err(|error| format!("could not write the settings: {error}"))
    };
    if let Err(error) = write() {
        tracing::warn!(%error, "the search settings were not remembered");
    }
}

/// Where the picture cache and the built packs go.
///
/// **Under `--data-dir`, never the repository's `.wpcache`** — that one belongs to whoever is
/// working on `km-wallpaper-pack` — and never `~/.cache/karaokemachine/assets`, which is a development
/// box's shared bank cache with gigabytes of somebody's survey in it.
pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("pictures/cache")
}

/// Where a pack is *built*, and overwritten by the next build.
///
/// **The scratch half, and it is scratch on purpose.** `analyze` writes `analysis.json` here and
/// `build` runs with `force`, clearing what the last run left — which is right for a working
/// directory and was wrong for the only copy of a pack. What survives a second search is what was
/// moved into [`packs_dir`] when the first one finished.
pub fn build_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("pictures/build")
}

/// Where finished packs are kept, one folder each.
///
/// **A folder per pack rather than a folder of packs**, because a pack is a zip *and* the manifest
/// and attribution that belong to it, and three files loose in one directory can only describe the
/// most recent of them.
pub fn packs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("pictures/packs")
}

/// One kept pack's folder, or `None` where the name is not one this program could have written.
///
/// **The id comes off a URL and the bank ids do not**, which is the whole reason this exists: a
/// bank's path is built from `km_banks`' own table, so nothing a caller types reaches the
/// filesystem, while a pack is named by whatever `zip_name` produced and is then handed back in a
/// route. So the name must be a single component and nothing else — no separator, no `.`, no `..`,
/// no prefix or root of its own — and `Path::components` deciding that is stricter than looking for
/// the characters by hand.
///
/// **The backslash is checked separately, and it has to be.** `components` splits on what the
/// *build target* calls a separator, so `a\b` is two components on Windows and one perfectly legal
/// filename on Linux — which would make this function's answer depend on which machine compiled it,
/// and the test that caught it agree with the bug.
pub fn pack_dir(data_dir: &Path, id: &str) -> Option<PathBuf> {
    if id.contains('\\') {
        return None;
    }
    let mut parts = Path::new(id).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(one)), None) if one == id => {
            Some(packs_dir(data_dir).join(one))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_apps_own_measured_numbers() {
        // Not re-typed here: the band the lyrics occupy and the scrim the display lays over every
        // wallpaper were measured once and live in `Config::default()`.
        let config = Settings::default().to_config();
        let stock = Config::default();
        assert_eq!(config.legibility.band_top, stock.legibility.band_top);
        assert_eq!(config.legibility.band_bottom, stock.legibility.band_bottom);
        assert_eq!(config.legibility.assumed_dim, stock.legibility.assumed_dim);
        assert_eq!(config.legibility.text_color, stock.legibility.text_color);
    }

    /// A named pack, for the tests about everything except the name.
    fn named() -> Settings {
        Settings {
            name: Some("beaches".to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn the_page_opens_on_the_provider_that_needs_no_account() {
        let settings = named();
        assert_eq!(settings.provider, ProviderKind::Openverse);
        assert!(
            settings.ready(false).is_ok(),
            "and it can be run as soon as the pack is named"
        );
    }

    /// **A pack is named before it is built**, which is the one answer this page cannot supply
    /// itself: a suggested name is the name every pack would carry.
    #[test]
    fn a_pack_with_no_name_is_refused_before_anything_is_searched() {
        let settings = Settings::default();
        assert_eq!(settings.name, None, "nothing is suggested");
        assert_eq!(
            settings.ready(true).expect_err("a pack needs a name"),
            NotReady::NoName
        );
        // ...and a name that slugs away to nothing is no name, the same as an empty box.
        let punctuation = Settings {
            name: slug("!!!"),
            ..Default::default()
        };
        assert_eq!(
            punctuation.ready(true).expect_err("that is not a name"),
            NotReady::NoName
        );
    }

    #[test]
    fn a_provider_that_needs_a_key_says_so_before_anything_is_spent() {
        let settings = Settings {
            provider: ProviderKind::Pixabay,
            ..named()
        };
        let refusal = settings.ready(false).expect_err("pixabay needs a key");
        assert_eq!(refusal, NotReady::NeedsKey("pixabay"), "{refusal:?}");
        assert!(settings.ready(true).is_ok());
    }

    #[test]
    fn nothing_to_search_for_is_refused_rather_than_run() {
        // An empty query set searches nothing, builds nothing, and would report success.
        let mut settings = named();
        settings.set_terms("   \n\n  ");
        assert!(settings.terms.is_empty());
        assert_eq!(
            settings.ready(true).expect_err("nothing to search for"),
            NotReady::NoTerms
        );
    }

    #[test]
    fn terms_survive_a_round_trip_through_the_textarea() {
        let mut settings = Settings::default();
        settings.set_terms("night sky\n\n  calm lake  \nmisty forest\n");
        assert_eq!(settings.terms, ["night sky", "calm lake", "misty forest"]);
        assert_eq!(settings.terms_text(), "night sky\ncalm lake\nmisty forest");
    }

    #[test]
    fn one_query_group_is_built_for_the_chosen_provider() {
        let settings = Settings {
            provider: ProviderKind::Pexels,
            pages: 3,
            ..Default::default()
        };
        let config = settings.to_config();
        assert_eq!(config.queries.len(), 1);
        assert_eq!(config.queries[0].provider, ProviderKind::Pexels);
        assert_eq!(config.queries[0].pages, 3);
    }

    #[test]
    fn the_cache_is_this_programs_and_not_a_checkouts() {
        // `.wpcache` in a repository belongs to whoever is working on the tool, and the shared asset
        // cache on a development box has gigabytes of somebody's SoundFont survey in it.
        let dir = cache_dir(Path::new("/data"));
        assert!(dir.starts_with("/data"), "{dir:?}");
        assert!(!dir.to_string_lossy().contains(".wpcache"), "{dir:?}");
    }

    #[test]
    fn building_and_keeping_are_two_directories() {
        // The one that gets cleared must not be the one holding everything that was ever built.
        let build = build_dir(Path::new("/data"));
        let packs = packs_dir(Path::new("/data"));
        assert_ne!(build, packs);
        assert!(!packs.starts_with(&build) && !build.starts_with(&packs));
    }

    #[test]
    fn a_pack_id_is_one_name_and_never_a_path() {
        let root = Path::new("/data");
        assert_eq!(
            pack_dir(root, "wallpapers-e36b9929"),
            Some(packs_dir(root).join("wallpapers-e36b9929"))
        );

        // Everything a route could be handed that is not a name in that folder.
        for bad in [
            "..",
            ".",
            "",
            "../banks",
            "a/b",
            "a\\b",
            "/etc",
            "C:/Windows",
            "./a",
        ] {
            assert_eq!(pack_dir(root, bad), None, "{bad:?} is not a pack");
        }
    }

    #[test]
    fn a_name_becomes_something_a_file_system_and_a_url_can_both_carry() {
        assert_eq!(slug("Beaches"), Some("beaches".to_owned()));
        assert_eq!(slug("  Praias do Sul  "), Some("praias-do-sul".to_owned()));
        // One dash for any run of spaces and dashes, so this is not `rock--roll`.
        assert_eq!(slug("Rock & Roll"), Some("rock-roll".to_owned()));
        // Dots and underscores survive, because a file name may hold them.
        assert_eq!(slug("v1.2_final"), Some("v1.2_final".to_owned()));
    }

    #[test]
    fn a_name_with_no_ascii_letters_left_in_it_is_no_name() {
        // Empty, and everything that empties out. Both mean "the plain name", which is the state the
        // page opens in.
        assert_eq!(slug(""), None);
        assert_eq!(slug("   "), None);
        assert_eq!(slug("!!!"), None);
        assert_eq!(slug("---"), None);
        // Dots are legal in a file name and are kept — but a name made of nothing else is `..`,
        // which is the parent directory wearing a slug's clothes. See the rule in `slug`.
        assert_eq!(slug("."), None);
        assert_eq!(slug(".."), None);
        assert_eq!(slug("___"), None);
    }

    /// Accented letters are dropped rather than transliterated, and that is the decision.
    ///
    /// Ugly and unambiguous beats a table of somebody's opinions about other people's alphabets. What
    /// makes it defensible is that the slug is written back into the field, so nobody is surprised by
    /// it twice.
    #[test]
    fn accents_are_dropped_rather_than_guessed_at() {
        assert_eq!(
            slug("Músicas Brasileiras"),
            Some("msicas-brasileiras".to_owned())
        );
    }

    /// Whatever comes out has to be something `pack_dir` will accept, because it becomes a folder.
    #[test]
    fn a_slug_is_always_a_single_path_component() {
        let root = Path::new("/data");
        for typed in [
            "a/b",
            "a\\b",
            "..",
            ".",
            "C:/Windows",
            "../../etc/passwd",
            "night sky / stars",
        ] {
            let Some(slug) = slug(typed) else {
                continue;
            };
            assert!(
                pack_dir(root, &slug).is_some(),
                "{typed:?} slugged to {slug:?}, which is not a folder this program may write"
            );
        }
    }
}
