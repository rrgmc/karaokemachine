//! Full-screen wallpapers cycling behind everything, as on a real machine.
//!
//! Still images only. Three separable pieces, so the timing logic is testable without decoding
//! anything:
//!
//! * [`Playlist`] — which images, in what order, and rescanning so more can be added while running.
//! * [`Schedule`] — when to change and how far through a crossfade we are.
//! * [`Loader`] — a background thread that decodes and downscales.
//!
//! An image comes out of a zip file in the folder, or from a file named individually; see
//! [`ImageSource`]. The loader exists because decoding is far too slow for a render thread. A 4K
//! JPEG takes tens of milliseconds to decode and would drop frames every time the wallpaper
//! changed, and uploading one at full size wastes GPU memory for pixels that are immediately scaled
//! down. It is decoded and resized to the display's own size off-thread, then handed over as
//! ready-to-upload pixels.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Image extensions that are loaded.
const EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "bmp", "gif"];

/// Extensions treated as archives of images.
const ARCHIVE_EXTENSIONS: [&str; 1] = ["zip"];

/// The most of one archive entry that is read into memory.
///
/// **The declared size is somebody else's claim about a file we did not make**, so it bounds the
/// capacity guess and nothing else — the bytes that arrive are what is counted. Deflate reaches
/// about a thousand to one, so a wallpaper pack inside the upload limit can otherwise expand to tens
/// of gigabytes while a picture is being drawn.
///
/// A photograph is single-digit megabytes and the pack this is read from is refused above 64 MiB on
/// the way in, so nothing real reaches this.
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// The most any one wallpaper may decode to, and the largest dimensions it may declare.
///
/// **`image`'s own default caps a single allocation at 512 MiB and bounds no dimension**, which
/// refuses the absurd and admits the merely ruinous: a picture just inside that ceiling is half a
/// gigabyte on a television with a gigabyte in it, and it is decoded on the way to being scaled down
/// to the screen. Both halves are named here because a dimension cap is what stops the work as well
/// as the memory — `resize_to_fill` walks every source pixel.
///
/// 16,384 on a side is four times a 4K screen's width, and every photograph anybody puts behind a
/// song is far inside it.
const MAX_PICTURE_PIXELS: u32 = 16_384;

/// The most memory one decode may take, in bytes.
const MAX_PICTURE_ALLOC: u64 = 96 * 1024 * 1024;

/// How a wallpaper fills the screen.
///
/// Carries its own serde derives because this is *configuration* — it is what `settings.json` holds.
/// Mirroring it in `km-app` would be a second type to keep in step for no gain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Fill the screen, cropping the overflow. What a wallpaper normally wants.
    #[default]
    Cover,
    /// Fit entirely on screen, leaving bars.
    Contain,
}

/// Wallpaper settings.
#[derive(Debug, Clone, PartialEq)]
pub struct WallpaperConfig {
    /// Folder to read wallpaper packs from. A zip in it is read as a folder of images; a picture
    /// sitting there on its own is passed over.
    ///
    /// Empty by default, and deliberately so: this crate draws what it is given and has no way to
    /// know where the embedding application installed its assets. An empty or missing folder is a
    /// supported state — the display falls back to a generated gradient — so a wrong guess here
    /// would be worse than none, because it would look like a configured folder that is empty.
    pub dir: PathBuf,
    /// Extra image or zip files to show **as well as** whatever [`Self::dir`] holds.
    ///
    /// Additive and never a replacement — the folder is scanned regardless and these go in beside
    /// what it finds. Empty on an ordinary machine; the embedding application fills it from a
    /// `debug.` setting, which is where naming an individual file is legitimate.
    ///
    /// **Not a second directory**, and that is the shape rather than a limitation: what was wanted
    /// was naming *files*, and a list of folders cannot accept one image. The three-candidate rule
    /// that picks [`Self::dir`] between an owner's folder, an overlay and the bundled set still
    /// picks exactly one, and still refuses to merge them.
    pub extra: Vec<PathBuf>,
    /// How long each image is shown, excluding the crossfade.
    pub interval: Duration,
    /// How long to cross-fade between images.
    pub crossfade: Duration,
    /// Shuffle rather than showing them in name order.
    pub shuffle: bool,
    /// How the image fills the screen.
    pub fit: Fit,
    /// How much to darken the image, 0.0 to 1.0, so lyrics stay readable over it.
    pub dim: f32,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::new(),
            extra: Vec::new(),
            interval: Duration::from_secs(30),
            crossfade: Duration::from_millis(1_200),
            shuffle: true,
            // Lyrics have to stay readable over an arbitrary photograph, so the default leans dark.
            fit: Fit::Cover,
            dim: 0.45,
        }
    }
}

/// One wallpaper image, wherever it lives.
///
/// **A zip's images are images in the folder**: an archive of twenty photographs behaves exactly as
/// twenty files would, in the cycle, in the count and in the shuffle. That is what lets a collection
/// arrive as one download and stay one file, and it is why a playlist entry is this rather than a
/// [`PathBuf`].
///
/// [`Self::File`] is what a file named in `debug.wallpapers` becomes. The folder itself yields only
/// [`Self::Zipped`] — see `A wallpaper is a pack` in `docs/decisions/interface.md`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageSource {
    /// An image file in the folder.
    File(PathBuf),
    /// An image inside a zip file in the folder.
    Zipped {
        /// The zip file.
        archive: PathBuf,
        /// The entry's path within it, spelled exactly as the archive spells it — which is what
        /// finds it again at decode time.
        entry: String,
    },
}

impl ImageSource {
    /// The file on disk this comes from: the image itself, or the archive holding it.
    pub fn container(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Zipped { archive, .. } => archive,
        }
    }

    /// The name to show for it.
    ///
    /// A zipped image is named `archive.zip/entry.jpg`, because `01.jpg` is a name two archives can
    /// both contain and the whole point of the folder is that anything may be dropped into it.
    pub fn name(&self) -> String {
        match self {
            Self::File(path) => file_name(path).to_owned(),
            Self::Zipped { archive, entry } => format!("{}/{entry}", file_name(archive)),
        }
    }

    /// What entries are sorted by, so an archive's images sit where the archive sits in name order
    /// rather than in a block of their own after every file named individually.
    fn sort_key(&self) -> String {
        match self {
            Self::File(path) => path.to_string_lossy().into_owned(),
            Self::Zipped { archive, entry } => format!("{}/{entry}", archive.to_string_lossy()),
        }
    }
}

impl From<PathBuf> for ImageSource {
    fn from(path: PathBuf) -> Self {
        Self::File(path)
    }
}

impl From<&Path> for ImageSource {
    fn from(path: &Path) -> Self {
        Self::File(path.to_path_buf())
    }
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("wallpaper")
}

/// The order a shuffled playlist walks its images in.
///
/// **Seeded by the caller, and the algorithm is here.** The seam this crate has always kept is that
/// randomness comes from outside — a display crate reaching for entropy is a display crate no test
/// can pin. A seed keeps that seam while letting the ordering live beside the playlist it reorders,
/// which is what a caller-supplied closure could not do: the order has to survive the rescan that
/// happens on every change, so something has to hold it.
///
/// SplitMix64, hand-rolled for the reason the timestamps in this workspace are: picking which
/// photograph comes next is not a job with a threat model, and a generator crate for it would be a
/// dependency bought with nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shuffle(u64);

impl Shuffle {
    /// A shuffle that will always produce the same order from the same seed.
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `bound`, or zero when there is nothing to choose from.
    ///
    /// Multiply-and-shift rather than a remainder: the bias is smaller and it costs one
    /// multiplication, where the modulo it replaces costs a division.
    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        let drawn = u128::from(self.next()) * bound as u128;
        (drawn >> 64) as usize
    }

    /// Fisher-Yates over a slice.
    fn apply<T>(&mut self, slice: &mut [T]) {
        for index in (1..slice.len()).rev() {
            let other = self.below(index + 1);
            slice.swap(index, other);
        }
    }
}

/// The images to show, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playlist {
    entries: Vec<ImageSource>,
    position: usize,
    /// The order, when the order is not the folder's.
    ///
    /// `None` is name order, which is what a caller listing *files* wants and what
    /// [`Playlist::scan`] hands back. `Some` is a pass over every picture in an order nobody can
    /// predict, reshuffled when the pass ends.
    shuffle: Option<Shuffle>,
}

impl Playlist {
    /// Builds a playlist from an explicit list of sources.
    ///
    /// Order is by name, which makes the sequence predictable and repeatable. Shuffling is applied
    /// by [`Playlist::shuffled_with`], which takes its randomness from the caller rather than
    /// reaching for a global generator.
    pub fn from_sources(mut entries: Vec<ImageSource>) -> Self {
        entries.sort_by_key(ImageSource::sort_key);
        Self {
            entries,
            position: 0,
            shuffle: None,
        }
    }

    /// Builds a playlist from plain paths, for tests and for a caller that already has files.
    pub fn from_paths(entries: Vec<PathBuf>) -> Self {
        Self::from_sources(entries.into_iter().map(ImageSource::File).collect())
    }

    /// Scans a folder for wallpaper packs, plus any extra files named individually.
    ///
    /// **The folder holds zips and nothing else** — the images inside them, in one flat list. A
    /// loose picture there is passed over, and `A wallpaper is a pack` in
    /// `docs/decisions/interface.md` is why: a zip is the only thing in that folder with an identity
    /// of its own, so it is the only thing a second copy of can be recognized as the same thing. A
    /// missing or unreadable folder yields an empty playlist rather than an error: the display falls
    /// back to a plain background, which is a far better outcome than refusing to start.
    ///
    /// `extra` names individual files to add **on top of** the folder, and **it still takes a loose
    /// image**. That is not an inconsistency but what `debug.` is for: naming one file is exactly
    /// the fragility that list exists to hold, the same way `debug.packages` names a `.kmpkg` the
    /// folder rule would not have found. It is one parameter rather than a second function so that a
    /// caller which must not pass extras has to write `&[]` and be seen doing it — see
    /// `holds_wallpapers` in `km-app`'s settings, where counting them would turn an additive setting
    /// into a replacing one and blank the screen.
    ///
    /// Deduplicated, because a file may be both named here and sitting in the folder.
    pub fn scan(dir: impl AsRef<Path>, extra: &[PathBuf]) -> Self {
        let mut entries = Vec::new();
        if let Ok(read) = std::fs::read_dir(dir.as_ref()) {
            for entry in read.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if is_archive(&path) {
                    entries.extend(archive_entries(&path));
                }
            }
        }
        for path in extra {
            if !path.is_file() {
                // Silently, as an unreadable folder is: this is a `debug.` list, and a typo shows up
                // as a picture that never appears rather than as anything a singer should read.
                continue;
            }
            if is_image(path) {
                entries.push(ImageSource::File(path.clone()));
            } else if is_archive(path) {
                entries.extend(archive_entries(path));
            }
        }
        // Deduplicated through `from_sources`' own ordering key rather than by adding an `Ord` to
        // `ImageSource`: sorting there is already how the list gets its stable order, so this asks
        // the same question the same way.
        entries.sort_by_key(ImageSource::sort_key);
        entries.dedup_by_key(|entry| entry.sort_key());
        Self::from_sources(entries)
    }

    /// Walks the images in an order nobody can predict, instead of by name.
    ///
    /// **A pass, not a draw.** Every picture is shown once before any is shown twice, and the order
    /// is made again when the pass ends — which is what somebody means by *random* about a folder
    /// of photographs, where picking independently each time would show one picture twice in a row
    /// and leave another out of a whole evening.
    ///
    /// The shuffle is kept rather than applied and forgotten, because the folder is rescanned on
    /// every change: an order applied once would be sorted back into name order the first time a
    /// picture appeared. See [`Playlist::refresh`].
    #[must_use]
    pub fn shuffled(mut self, mut shuffle: Shuffle) -> Self {
        shuffle.apply(&mut self.entries);
        self.shuffle = Some(shuffle);
        self.position = 0;
        self
    }

    /// How many images are in the playlist.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no images.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every image, in cycle order.
    ///
    /// For a caller that wants the *files* rather than the pictures — the owner's page lists one row
    /// per file and an archive is one row — which means grouping these by
    /// [`ImageSource::container`]. Exposed rather than adding a `containers()` here because the
    /// grouping is a question about what a page shows, and this type is about what is on screen next.
    pub fn entries(&self) -> &[ImageSource] {
        &self.entries
    }

    /// The image that should be showing.
    pub fn current(&self) -> Option<&ImageSource> {
        self.entries.get(self.position)
    }

    /// The image after the current one, which is what the loader should prepare next.
    pub fn peek_next(&self) -> Option<&ImageSource> {
        if self.entries.is_empty() {
            return None;
        }
        let next = (self.position + 1) % self.entries.len();
        self.entries.get(next)
    }

    /// Advances to the next image, wrapping at the end.
    ///
    /// **A shuffled playlist is reordered where it wraps**, so the second pass over a folder is not
    /// the first one again. The picture that has just been showing is kept out of the front of the
    /// new order: it is the one repeat a fresh shuffle can produce that a viewer would notice,
    /// because the two showings would be adjacent.
    pub fn advance(&mut self) -> Option<&ImageSource> {
        if self.entries.is_empty() {
            return None;
        }
        self.position = (self.position + 1) % self.entries.len();
        if self.position == 0 {
            self.reshuffle();
        }
        self.current()
    }

    /// Makes the order again, keeping the picture that was showing away from the front.
    fn reshuffle(&mut self) {
        let Some(shuffle) = self.shuffle.as_mut() else {
            return;
        };
        let showing = self.entries.last().cloned();
        shuffle.apply(&mut self.entries);
        if self.entries.len() > 1 && showing.is_some_and(|was| self.entries.first() == Some(&was)) {
            let elsewhere = 1 + shuffle.below(self.entries.len() - 1);
            self.entries.swap(0, elsewhere);
        }
    }

    /// Replaces the entries with a fresh scan, keeping the current image showing if it survived.
    ///
    /// Rescanning is what lets images — or a zip file of them — be dropped into the folder while the
    /// machine is running, without restarting it.
    ///
    /// **A shuffled playlist keeps its order across this, and that is the whole reason the shuffle
    /// is a field rather than something done once at startup.** The scan hands back name order, so
    /// taking it wholesale would sort the pass back into alphabetical the first time anybody added
    /// a picture — which is every thirty seconds on a folder nobody is touching, since this runs on
    /// every change. What survives keeps the place it had; what has gone is dropped; what is new is
    /// put into the part of the pass that has not happened yet, so a picture dropped in is seen
    /// during this pass rather than after every other picture in the folder.
    pub fn refresh(&mut self, dir: impl AsRef<Path>, extra: &[PathBuf]) {
        let showing = self.current().cloned();
        let fresh = Self::scan(dir, extra);

        match self.shuffle.as_mut() {
            // Name order is remade from the folder every time, so there is nothing to preserve.
            None => self.entries = fresh.entries,
            Some(shuffle) => {
                let present: std::collections::HashSet<&ImageSource> =
                    fresh.entries.iter().collect();
                let mut kept = std::mem::take(&mut self.entries);
                kept.retain(|entry| present.contains(entry));

                let known: std::collections::HashSet<ImageSource> = kept.iter().cloned().collect();
                // Anywhere at or after the picture on screen, never before it: an insertion in
                // front would move what is showing and the pass would repeat it.
                let after = kept
                    .iter()
                    .position(|entry| Some(entry) == showing.as_ref())
                    .map_or(0, |index| index + 1);
                for entry in fresh.entries {
                    if known.contains(&entry) {
                        continue;
                    }
                    let at = after + shuffle.below(kept.len() + 1 - after);
                    kept.insert(at, entry);
                }
                self.entries = kept;
            }
        }

        self.position = showing
            .and_then(|source| self.entries.iter().position(|entry| *entry == source))
            .unwrap_or(0);
    }
}

fn has_extension(path: &Path, allowed: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            let lower = ext.to_lowercase();
            allowed.contains(&lower.as_str())
        })
}

fn is_image(path: &Path) -> bool {
    has_extension(path, &EXTENSIONS)
}

fn is_archive(path: &Path) -> bool {
    has_extension(path, &ARCHIVE_EXTENSIONS)
}

/// The images inside one zip file.
///
/// Only the archive's directory is read — nothing is decoded and nothing is unpacked to disk, so
/// this is cheap enough to run on every rescan. A corrupt, encrypted or simply not-a-zip file
/// contributes no images rather than failing the scan: the folder is somewhere people drop things,
/// and one bad file must not cost them the rest.
fn archive_entries(archive: &Path) -> Vec<ImageSource> {
    let Ok(file) = std::fs::File::open(archive) else {
        tracing::warn!(path = %archive.display(), "could not open a wallpaper archive");
        return Vec::new();
    };
    let zip = match zip::ZipArchive::new(std::io::BufReader::new(file)) {
        Ok(zip) => zip,
        Err(error) => {
            tracing::warn!(path = %archive.display(), %error, "not a readable zip file");
            return Vec::new();
        }
    };
    let entries: Vec<_> = zip
        .file_names()
        .filter(|name| !name.ends_with('/') && is_image(Path::new(name)) && !is_macos_junk(name))
        .map(|name| ImageSource::Zipped {
            archive: archive.to_path_buf(),
            entry: name.to_owned(),
        })
        .collect();
    tracing::debug!(
        path = %archive.display(),
        images = entries.len(),
        "wallpaper archive"
    );
    entries
}

/// Whether a zip entry is macOS packaging rather than a picture.
///
/// Compressing a folder on macOS adds a `__MACOSX` tree of `._name` AppleDouble files that carry the
/// same extensions as the originals and are not images. Left in, every such archive would put a
/// failed decode between each real wallpaper.
fn is_macos_junk(name: &str) -> bool {
    name.split('/')
        .any(|part| part == "__MACOSX" || part.starts_with("._"))
}

/// Tracks when to change wallpaper and how far through a crossfade we are.
///
/// Driven by elapsed time handed in by the caller rather than reading a clock, so the schedule can
/// be tested exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    interval: Duration,
    crossfade: Duration,
    /// Time since the current image became fully visible.
    elapsed: Duration,
    /// How far through a crossfade, if one is running.
    fading: Option<Duration>,
    /// The next image has been asked for and has not arrived yet.
    ///
    /// A crossfade needs two images and there is only one, so nothing fades here — the fade's clock
    /// starts at [`Schedule::start_crossfade`], when the loader delivers. Starting it when the
    /// interval elapsed instead ramped the *outgoing* image up from zero with nothing behind it,
    /// which is a black screen for as long as a decode takes.
    awaiting: bool,
}

/// What the schedule wants the renderer to do this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Presentation {
    /// Opacity of the incoming image, 0.0 to 1.0. Below 1.0 means a crossfade is in progress.
    pub fade_in: f32,
    /// Whether a crossfade is running, so the outgoing image is still needed.
    pub crossfading: bool,
}

impl Schedule {
    /// A schedule for the given timings.
    pub fn new(interval: Duration, crossfade: Duration) -> Self {
        Self {
            interval,
            crossfade,
            elapsed: Duration::ZERO,
            fading: None,
            awaiting: false,
        }
    }

    /// Advances by a frame's worth of time.
    ///
    /// Returns `true` when it is time to move to the next image — once per change, not once per
    /// frame until it arrives, because the caller advances its playlist on every `true`.
    pub fn tick(&mut self, delta: Duration) -> bool {
        if let Some(fading) = &mut self.fading {
            *fading += delta;
            if *fading >= self.crossfade {
                self.fading = None;
                self.elapsed = Duration::ZERO;
            }
            return false;
        }

        // Asked for, not arrived. There is nothing to time: the interval is spent and the fade has
        // not begun.
        if self.awaiting {
            return false;
        }

        self.elapsed += delta;
        if self.elapsed >= self.interval {
            self.awaiting = true;
            return true;
        }
        false
    }

    /// Starts the crossfade, which is what arriving with a decoded image looks like.
    ///
    /// The only thing that begins a fade — and it is called once the incoming image exists, so the
    /// ramp always has something to fade *from* as well as *to*.
    pub fn start_crossfade(&mut self) {
        self.awaiting = false;
        self.fading = Some(Duration::ZERO);
    }

    /// A change is under way that the interval did not ask for.
    ///
    /// Everything that is not the timer — a song starting, `POST /wallpapers/next`, an upload, a
    /// delete — reaches the loader without passing through [`Self::tick`], so nothing has set
    /// `awaiting` and the interval goes on timing under a decode that is already in flight. A
    /// request made a moment before the interval runs out would then fire the timer as well: two
    /// images asked for, the playlist advanced twice, and neither picture on the screen long
    /// enough to be seen. [`Self::start_crossfade`] and [`Self::abandon_change`] clear it, exactly
    /// as they do for a change the timer began.
    pub fn change_now(&mut self) {
        self.awaiting = true;
        self.elapsed = Duration::ZERO;
    }

    /// The image that was asked for is never coming — a decode or an upload failed.
    ///
    /// Puts the schedule back to showing what is already on screen, so the next interval asks
    /// again. Without it one bad file stops the cycle for the rest of the evening.
    pub fn abandon_change(&mut self) {
        self.awaiting = false;
        self.elapsed = Duration::ZERO;
    }

    /// What to draw this frame.
    pub fn presentation(&self) -> Presentation {
        match self.fading {
            Some(fading) if !self.crossfade.is_zero() => {
                let progress = fading.as_secs_f32() / self.crossfade.as_secs_f32();
                Presentation {
                    fade_in: progress.clamp(0.0, 1.0),
                    crossfading: true,
                }
            }
            // A zero-length crossfade is a hard cut, which is a legitimate setting.
            Some(_) => Presentation {
                fade_in: 1.0,
                crossfading: false,
            },
            None => Presentation {
                fade_in: 1.0,
                crossfading: false,
            },
        }
    }
}

/// A decoded, downscaled image ready to upload.
#[derive(Debug, Clone)]
pub struct LoadedImage {
    /// Where it came from.
    pub source: ImageSource,
    /// Width in pixels, after downscaling.
    pub width: u32,
    /// Height in pixels, after downscaling.
    pub height: u32,
    /// Tightly packed RGBA8 pixels.
    pub rgba: Vec<u8>,
}

/// A request to the loader thread.
struct LoadRequest {
    source: ImageSource,
    target_width: u32,
    target_height: u32,
    fit: Fit,
}

/// Decodes wallpapers off the render thread.
pub struct Loader {
    requests: mpsc::Sender<LoadRequest>,
    results: mpsc::Receiver<Result<LoadedImage, String>>,
}

impl Loader {
    /// Starts the loader thread.
    pub fn start() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<LoadRequest>();
        let (result_tx, result_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("wallpaper-loader".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let outcome = decode(&request);
                    // A closed receiver means the display is shutting down.
                    if result_tx.send(outcome).is_err() {
                        break;
                    }
                }
            })
            // Without the loader the display would decode on the render thread and stutter, so
            // failing to spawn is worth knowing about rather than silently degrading.
            .expect("the wallpaper loader thread should start");

        Self {
            requests: request_tx,
            results: result_rx,
        }
    }

    /// Asks for an image, sized for the display.
    ///
    /// Returns `false` if the loader has stopped.
    pub fn request(
        &self,
        source: impl Into<ImageSource>,
        width: u32,
        height: u32,
        fit: Fit,
    ) -> bool {
        self.requests
            .send(LoadRequest {
                source: source.into(),
                target_width: width.max(1),
                target_height: height.max(1),
                fit,
            })
            .is_ok()
    }

    /// Collects a finished image, if one is ready. Never blocks.
    pub fn poll(&self) -> Option<Result<LoadedImage, String>> {
        self.results.try_recv().ok()
    }
}

fn decode(request: &LoadRequest) -> Result<LoadedImage, String> {
    let image = match &request.source {
        ImageSource::File(path) => {
            let reader = image::ImageReader::open(path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .with_guessed_format()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            bounded(reader)
                .decode()
                .map_err(|e| format!("{}: {e}", path.display()))?
        }
        ImageSource::Zipped { archive, entry } => {
            let bytes = read_archive_entry(archive, entry)?;
            // Decoded by content rather than by extension: an entry's name inside an archive is
            // even less of a promise than a file's on disk.
            let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| format!("{}/{entry}: {e}", archive.display()))?;
            bounded(reader)
                .decode()
                .map_err(|e| format!("{}/{entry}: {e}", archive.display()))?
        }
    };

    // Downscaled before it ever reaches the GPU. Uploading a 4K image to fill a 1080p screen wastes
    // three quarters of the memory and all of the bandwidth.
    let scaled = match request.fit {
        Fit::Cover => image.resize_to_fill(
            request.target_width,
            request.target_height,
            image::imageops::FilterType::Triangle,
        ),
        Fit::Contain => image.resize(
            request.target_width,
            request.target_height,
            image::imageops::FilterType::Triangle,
        ),
    };

    let rgba = scaled.to_rgba8();
    Ok(LoadedImage {
        source: request.source.clone(),
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// Reads one image out of an archive, whole, on the loader thread.
///
/// Whole rather than streamed because the decoders want to seek, and a wallpaper is a few megabytes
/// — the same bytes the file variant hands to `image::open`. The archive is reopened per image
/// instead of held: a `ZipArchive` is not shareable across the rescan that may replace the file
/// underneath it, and opening one is a directory read, not a decompression.
/// Puts this machine's ceilings on a reader before it decodes anything.
///
/// **A picture is a file from a stranger as much as a package is**, and the wallpaper folder takes
/// one over the network. `image`'s defaults bound a single allocation and no dimension, so both are
/// named here — see [`MAX_PICTURE_PIXELS`].
///
/// The limits are set on the reader rather than checked after the decode, which is the whole point:
/// a decoder that has already allocated the memory has already done the harm.
fn bounded<R: std::io::BufRead + std::io::Seek>(
    mut reader: image::ImageReader<R>,
) -> image::ImageReader<R> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PICTURE_PIXELS);
    limits.max_image_height = Some(MAX_PICTURE_PIXELS);
    limits.max_alloc = Some(MAX_PICTURE_ALLOC);
    reader.limits(limits);
    reader
}

fn read_archive_entry(archive: &Path, entry: &str) -> Result<Vec<u8>, String> {
    let describe =
        |error: &dyn std::fmt::Display| format!("{}/{entry}: {error}", archive.display());

    let file = std::fs::File::open(archive).map_err(|e| describe(&e))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| describe(&e))?;
    let found = zip.by_name(entry).map_err(|e| describe(&e))?;

    // The uncompressed size comes from the archive's own directory, so this is normally one
    // allocation. Capped, because that number is somebody else's claim about a file we did not make.
    let mut bytes = Vec::with_capacity(found.size().min(MAX_ENTRY_BYTES) as usize);
    // One byte past the ceiling, which is what tells an entry that exactly fills it from one that
    // runs past it without holding the overrun.
    found
        .take(MAX_ENTRY_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| describe(&e))?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(describe(
            &format_args!("is larger than {MAX_ENTRY_BYTES} bytes and was not read").to_string(),
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn file(name: &str) -> ImageSource {
        ImageSource::File(PathBuf::from(name))
    }

    /// A scratch folder of its own per test, so tests that write files do not collide.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("km-display-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A tiny real PNG, as bytes, so archives in tests hold images that actually decode.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 200])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("encode png");
        bytes.into_inner()
    }

    /// Puts each named picture into a pack of its own, and says what the folder now holds.
    ///
    /// **The folder takes packs and not pictures**, so a test about *what is in the folder* has to
    /// put its pictures in one. One pack per picture rather than one holding all of them, because
    /// these tests are about a playlist's order and membership: a pack apiece keeps the two lists
    /// the same length and keeps `a.png` sorting before `b.png`, which sharing an archive would not.
    fn packs_of(dir: &Path, names: &[&str]) -> Vec<ImageSource> {
        names
            .iter()
            .map(|name| {
                let stem = Path::new(name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .expect("a stem");
                let archive = dir.join(format!("{stem}.zip"));
                write_zip(&archive, &[(name, png_bytes(4, 4))]);
                ImageSource::Zipped {
                    archive,
                    entry: (*name).to_owned(),
                }
            })
            .collect()
    }

    /// Writes a zip file whose entries are `(name, bytes)`.
    fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
        let file = std::fs::File::create(path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, options).expect("start entry");
            std::io::Write::write_all(&mut zip, bytes).expect("write entry");
        }
        zip.finish().expect("finish zip");
    }

    fn wait_for(loader: &Loader) -> Result<LoadedImage, String> {
        for _ in 0..400 {
            if let Some(result) = loader.poll() {
                return result;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the loader never answered");
    }

    #[test]
    fn an_empty_playlist_yields_nothing_rather_than_panicking() {
        let mut playlist = Playlist::default();
        assert!(playlist.is_empty());
        assert_eq!(playlist.current(), None);
        assert_eq!(playlist.peek_next(), None);
        assert_eq!(playlist.advance(), None);
    }

    #[test]
    fn a_missing_folder_gives_an_empty_playlist_not_an_error() {
        let playlist = Playlist::scan("definitely/not/a/real/folder", &[]);
        assert!(playlist.is_empty());
    }

    #[test]
    fn entries_are_ordered_by_name_so_the_sequence_is_repeatable() {
        let playlist = Playlist::from_paths(paths(&["c.jpg", "a.jpg", "b.jpg"]));
        assert_eq!(playlist.current(), Some(&file("a.jpg")));
        assert_eq!(playlist.peek_next(), Some(&file("b.jpg")));
    }

    #[test]
    fn advancing_wraps_at_the_end() {
        let mut playlist = Playlist::from_paths(paths(&["a.jpg", "b.jpg"]));
        assert_eq!(playlist.advance(), Some(&file("b.jpg")));
        assert_eq!(playlist.advance(), Some(&file("a.jpg")));
    }

    #[test]
    fn a_single_image_still_advances_to_itself() {
        let mut playlist = Playlist::from_paths(paths(&["only.jpg"]));
        assert_eq!(playlist.peek_next(), Some(&file("only.jpg")));
        assert_eq!(playlist.advance(), Some(&file("only.jpg")));
    }

    /// The order comes from the seed, so a test can pin one and the machine can draw one.
    #[test]
    fn shuffling_is_seeded_by_the_caller_so_it_can_be_deterministic() {
        let names = ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg"];
        let order = |seed| {
            let mut playlist = Playlist::from_paths(paths(&names)).shuffled(Shuffle::seeded(seed));
            let mut seen = vec![playlist.current().cloned().expect("a first picture")];
            for _ in 1..names.len() {
                seen.push(playlist.advance().cloned().expect("a next picture"));
            }
            seen
        };

        assert_eq!(order(7), order(7), "one seed is one order");
        assert_ne!(order(7), order(8), "and two seeds are not");
        assert_ne!(
            order(7),
            Playlist::from_paths(paths(&names)).entries().to_vec(),
            "a shuffled playlist does not walk the folder by name"
        );
    }

    /// **Every picture once before any picture twice**, which is what shuffle means about a folder.
    #[test]
    fn a_pass_shows_every_picture_exactly_once() {
        let names = ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg", "f.jpg"];
        let mut playlist = Playlist::from_paths(paths(&names)).shuffled(Shuffle::seeded(3));

        let mut seen = vec![playlist.current().cloned().expect("a first picture")];
        for _ in 1..names.len() {
            seen.push(playlist.advance().cloned().expect("a next picture"));
        }

        let mut sorted = seen.clone();
        sorted.sort_by_key(ImageSource::sort_key);
        sorted.dedup_by_key(|entry| entry.sort_key());
        assert_eq!(
            sorted.len(),
            names.len(),
            "a pass repeated a picture: {seen:?}"
        );
    }

    /// The second pass is not the first one again, and does not open on what just closed the first.
    #[test]
    fn the_order_is_made_again_at_the_end_of_a_pass() {
        let names = ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg", "f.jpg"];
        let mut playlist = Playlist::from_paths(paths(&names)).shuffled(Shuffle::seeded(11));

        let first: Vec<_> = playlist.entries().to_vec();
        for _ in 1..names.len() {
            playlist.advance();
        }
        let last_of_the_pass = playlist.current().cloned().expect("a last picture");

        // The wrap, which is where the order is made again.
        let opens_the_second = playlist.advance().cloned().expect("a first picture");
        assert_ne!(
            playlist.entries().to_vec(),
            first,
            "the second pass repeated the first"
        );
        assert_ne!(
            opens_the_second, last_of_the_pass,
            "the one repeat a viewer would notice is two showings in a row"
        );
    }

    /// A shuffled pass survives the rescan that happens on every change.
    #[test]
    fn refreshing_does_not_sort_a_shuffled_pass_back_into_name_order() {
        let dir = scratch("wallpaper-shuffle-refresh");
        packs_of(&dir, &["a.png", "b.png", "c.png", "d.png", "e.png"]);

        let mut playlist = Playlist::scan(&dir, &[]).shuffled(Shuffle::seeded(5));
        let before = playlist.entries().to_vec();
        let showing = playlist.current().cloned().expect("a picture");

        playlist.refresh(&dir, &[]);

        assert_eq!(playlist.entries().to_vec(), before, "the order was rebuilt");
        assert_eq!(playlist.current(), Some(&showing), "and the screen changed");
    }

    /// A picture dropped in is reached during this pass rather than after every other one.
    #[test]
    fn a_picture_added_mid_pass_lands_in_what_is_left_of_it() {
        let dir = scratch("wallpaper-shuffle-newcomer");
        packs_of(
            &dir,
            &["a.png", "b.png", "c.png", "d.png", "e.png", "f.png"],
        );

        let mut playlist = Playlist::scan(&dir, &[]).shuffled(Shuffle::seeded(9));
        playlist.advance();
        playlist.advance();
        let showing = playlist.current().cloned().expect("a picture");

        let newcomer = packs_of(&dir, &["new.png"]).remove(0);
        playlist.refresh(&dir, &[]);

        let at = playlist
            .entries()
            .iter()
            .position(|entry| *entry == newcomer)
            .expect("the new picture is in the playlist");
        let showing_at = playlist
            .entries()
            .iter()
            .position(|entry| *entry == showing)
            .expect("the picture on screen is still in the playlist");
        assert!(
            at > showing_at,
            "a newcomer placed before the screen would move what is showing"
        );
        assert_eq!(playlist.current(), Some(&showing), "and the screen changed");
    }

    /// What has gone is dropped, and everything else keeps the place it had.
    #[test]
    fn a_picture_removed_costs_only_its_own_place_in_the_pass() {
        let dir = scratch("wallpaper-shuffle-removed");
        packs_of(&dir, &["a.png", "b.png", "c.png", "d.png", "e.png"]);

        let mut playlist = Playlist::scan(&dir, &[]).shuffled(Shuffle::seeded(13));
        let gone = playlist.entries()[3].clone();
        let expected: Vec<_> = playlist
            .entries()
            .iter()
            .filter(|entry| **entry != gone)
            .cloned()
            .collect();

        std::fs::remove_file(gone.container()).expect("remove");
        playlist.refresh(&dir, &[]);

        assert_eq!(playlist.entries().to_vec(), expected);
    }

    /// An unshuffled playlist is still the folder in name order, every time it is asked.
    #[test]
    fn an_unshuffled_playlist_is_still_the_folder_by_name() {
        let dir = scratch("wallpaper-unshuffled");
        packs_of(&dir, &["c.png", "a.png", "b.png"]);

        let mut playlist = Playlist::scan(&dir, &[]);
        let by_name: Vec<_> = playlist.entries().to_vec();
        playlist.refresh(&dir, &[]);
        assert_eq!(playlist.entries().to_vec(), by_name);
        assert_eq!(
            playlist.current().map(ImageSource::name).as_deref(),
            Some("a.zip/a.png"),
            "the folder is walked in name order whatever order the packs were written in"
        );
    }

    #[test]
    fn only_image_extensions_are_recognized() {
        assert!(is_image(Path::new("photo.JPG")));
        assert!(is_image(Path::new("photo.png")));
        assert!(is_image(Path::new("photo.webp")));
        assert!(!is_image(Path::new("song.kar")));
        assert!(!is_image(Path::new("notes.txt")));
        assert!(!is_image(Path::new("no-extension")));
        // A zip is not itself an image; it is a folder's worth of them.
        assert!(!is_image(Path::new("pack.zip")));
        assert!(is_archive(Path::new("pack.ZIP")));
        assert!(!is_archive(Path::new("photo.png")));
    }

    #[test]
    fn a_zipped_image_is_named_after_its_archive_as_well_as_itself() {
        let source = ImageSource::Zipped {
            archive: PathBuf::from("/assets/wallpapers/pack.zip"),
            entry: "beach/01.jpg".to_owned(),
        };
        assert_eq!(source.name(), "pack.zip/beach/01.jpg");
        assert_eq!(
            source.container(),
            Path::new("/assets/wallpapers/pack.zip"),
            "the file on disk is the archive"
        );
        assert_eq!(file("photo.png").name(), "photo.png");
    }

    #[test]
    fn a_zips_images_count_as_images_in_the_folder() {
        let dir = scratch("wallpaper-zip-scan");
        // A loose picture in the folder, which is passed over: the folder takes packs.
        std::fs::write(dir.join("loose.png"), png_bytes(4, 4)).expect("write");
        write_zip(
            &dir.join("pack.zip"),
            &[
                ("one.png", png_bytes(4, 4)),
                ("nested/two.jpg", png_bytes(4, 4)),
                // Neither of these is a wallpaper: one is not an image, the other is macOS
                // packaging that carries an image extension and is not an image.
                ("readme.txt", b"not a picture".to_vec()),
                ("__MACOSX/._one.png", b"resource fork".to_vec()),
            ],
        );

        let playlist = Playlist::scan(&dir, &[]);
        assert_eq!(
            playlist.len(),
            2,
            "the archive's two images, and not the loose file beside it, got {:?}",
            playlist.entries
        );
        let names: Vec<_> = playlist.entries.iter().map(ImageSource::name).collect();
        assert!(
            !names.contains(&"loose.png".to_owned()),
            "a picture loose in the folder is not a wallpaper: {names:?}"
        );
        assert!(names.contains(&"pack.zip/one.png".to_owned()), "{names:?}");
        assert!(
            names.contains(&"pack.zip/nested/two.jpg".to_owned()),
            "{names:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn any_number_of_zips_contribute_together() {
        let dir = scratch("wallpaper-many-zips");
        for archive in ["a.zip", "b.zip", "c.zip"] {
            write_zip(
                &dir.join(archive),
                &[("1.png", png_bytes(4, 4)), ("2.png", png_bytes(4, 4))],
            );
        }

        let playlist = Playlist::scan(&dir, &[]);
        assert_eq!(playlist.len(), 6, "three archives of two images each");

        // Ordered as though the archives' contents were laid out in the folder: an archive's images
        // sit together, at the archive's own place in name order.
        let names: Vec<_> = playlist.entries.iter().map(ImageSource::name).collect();
        assert_eq!(
            names,
            vec![
                "a.zip/1.png",
                "a.zip/2.png",
                "b.zip/1.png",
                "b.zip/2.png",
                "c.zip/1.png",
                "c.zip/2.png",
            ]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_that_cannot_be_read_costs_only_itself() {
        let dir = scratch("wallpaper-broken-zip");
        std::fs::write(dir.join("truncated.zip"), b"PK\x03\x04 and then nothing").expect("write");
        std::fs::write(dir.join("empty.zip"), b"").expect("write");
        packs_of(&dir, &["real.png"]);

        let playlist = Playlist::scan(&dir, &[]);
        assert_eq!(
            playlist.len(),
            1,
            "the good pack survives two unreadable archives"
        );
        assert_eq!(
            playlist.current().map(ImageSource::name).as_deref(),
            Some("real.zip/real.png")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_of_nothing_but_junk_adds_no_images() {
        let dir = scratch("wallpaper-junk-zip");
        write_zip(
            &dir.join("junk.zip"),
            &[("notes.txt", b"x".to_vec()), ("song.kar", b"x".to_vec())],
        );

        assert!(Playlist::scan(&dir, &[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_schedule_changes_image_after_the_interval() {
        let mut schedule = Schedule::new(Duration::from_secs(2), Duration::from_millis(500));
        assert!(!schedule.tick(Duration::from_secs(1)));
        assert!(
            schedule.tick(Duration::from_secs(1)),
            "the interval has elapsed, so it is time to change"
        );
    }

    #[test]
    fn a_crossfade_reports_progress_and_then_finishes() {
        let mut schedule = Schedule::new(Duration::from_secs(1), Duration::from_millis(1_000));
        schedule.tick(Duration::from_secs(1));
        // The loader delivering, which is the only thing that starts a fade.
        schedule.start_crossfade();

        let start = schedule.presentation();
        assert!(start.crossfading);
        assert!(start.fade_in < 0.1, "the incoming image starts invisible");

        schedule.tick(Duration::from_millis(500));
        let middle = schedule.presentation();
        assert!(
            (0.4..=0.6).contains(&middle.fade_in),
            "halfway should be about 0.5, got {}",
            middle.fade_in
        );

        schedule.tick(Duration::from_millis(600));
        let after = schedule.presentation();
        assert!(!after.crossfading, "the crossfade should be over");
        assert_eq!(after.fade_in, 1.0);
    }

    #[test]
    fn no_second_change_is_requested_while_a_crossfade_runs() {
        let mut schedule = Schedule::new(Duration::from_millis(100), Duration::from_secs(5));
        assert!(schedule.tick(Duration::from_millis(100)));
        schedule.start_crossfade();
        // Even long past the interval, a fade in progress must not trigger another change.
        assert!(!schedule.tick(Duration::from_secs(1)));
        assert!(!schedule.tick(Duration::from_secs(1)));
    }

    /// The bug this schedule was shaped to fix: between the interval elapsing and the loader
    /// delivering, the only image in existence is the one already on screen, and it must stay at
    /// full opacity. Fading it up from zero over the theme background is a black screen.
    #[test]
    fn the_current_image_stays_fully_visible_until_the_next_one_arrives() {
        let mut schedule = Schedule::new(Duration::from_millis(100), Duration::from_secs(2));
        assert!(schedule.tick(Duration::from_millis(100)));

        for frame in 0..60 {
            let presentation = schedule.presentation();
            assert_eq!(
                presentation.fade_in, 1.0,
                "frame {frame} of the wait dimmed the image that is on screen"
            );
            assert!(
                !presentation.crossfading,
                "frame {frame} claimed a crossfade with only one image to show"
            );
            schedule.tick(Duration::from_millis(16));
        }

        // And the fade is normal once there is something to fade to.
        schedule.start_crossfade();
        assert!(schedule.presentation().crossfading);

        // Delivery ends the wait as well as starting the fade. Those are two separate fields, so a
        // `start_crossfade` that cleared only one would run this fade and then never ask again.
        schedule.tick(Duration::from_secs(2));
        assert!(!schedule.presentation().crossfading);
        assert!(
            schedule.tick(Duration::from_millis(100)),
            "the interval runs again once the change is finished"
        );
    }

    /// `true` means "advance the playlist and ask for that image", so a `true` on every frame of a
    /// slow decode would race through the folder and show none of it.
    #[test]
    fn only_one_change_is_asked_for_while_the_next_image_decodes() {
        let mut schedule = Schedule::new(Duration::from_millis(100), Duration::from_secs(2));
        assert!(schedule.tick(Duration::from_millis(100)));
        for _ in 0..60 {
            assert!(!schedule.tick(Duration::from_millis(16)));
        }
    }

    /// Three things reach here and none of them will ever call `start_crossfade`: a decode that
    /// fails, an upload that fails, and a change that could not even be asked for — an empty folder,
    /// or a loader that has stopped. Without this the cycle would sit waiting for an image nobody is
    /// going to deliver.
    #[test]
    fn an_image_that_never_arrives_does_not_stop_the_cycle() {
        let mut schedule = Schedule::new(Duration::from_millis(100), Duration::from_secs(2));
        assert!(schedule.tick(Duration::from_millis(100)));
        schedule.abandon_change();

        assert!(!schedule.tick(Duration::from_millis(50)));
        assert!(
            schedule.tick(Duration::from_millis(50)),
            "a fresh interval should ask again after a failed load"
        );
    }

    #[test]
    fn the_interval_restarts_only_after_the_crossfade_completes() {
        let mut schedule = Schedule::new(Duration::from_millis(500), Duration::from_millis(200));
        assert!(schedule.tick(Duration::from_millis(500)));
        schedule.start_crossfade();
        schedule.tick(Duration::from_millis(200));
        // Fresh interval, so a short tick must not immediately ask for another change.
        assert!(!schedule.tick(Duration::from_millis(100)));
        assert!(schedule.tick(Duration::from_millis(400)));
    }

    #[test]
    fn a_zero_length_crossfade_is_a_hard_cut_rather_than_a_division_by_zero() {
        let mut schedule = Schedule::new(Duration::from_millis(10), Duration::ZERO);
        assert!(schedule.tick(Duration::from_millis(10)));
        schedule.start_crossfade();
        let presentation = schedule.presentation();
        assert_eq!(presentation.fade_in, 1.0);
        assert!(!presentation.crossfading);
    }

    #[test]
    fn a_manual_change_starts_a_crossfade() {
        let mut schedule = Schedule::new(Duration::from_secs(30), Duration::from_millis(500));
        schedule.start_crossfade();
        assert!(schedule.presentation().crossfading);
    }

    #[test]
    fn a_change_the_interval_did_not_ask_for_stops_the_interval_timing() {
        let mut schedule = Schedule::new(Duration::from_secs(30), Duration::from_millis(500));
        // A whisker from the end of the interval, which is where the fault lived.
        assert!(!schedule.tick(Duration::from_millis(29_900)));
        schedule.change_now();
        assert!(
            !schedule.tick(Duration::from_secs(5)),
            "the timer must not ask for a second image while a decode is in flight"
        );
        // The loader delivering, and then a fresh interval from there.
        schedule.start_crossfade();
        schedule.tick(Duration::from_millis(500));
        assert!(!schedule.tick(Duration::from_secs(29)));
        assert!(schedule.tick(Duration::from_secs(1)));
    }

    #[test]
    fn a_change_that_never_arrives_leaves_the_interval_running_again() {
        let mut schedule = Schedule::new(Duration::from_secs(2), Duration::from_millis(500));
        schedule.change_now();
        // A decode or an upload that failed: nothing is coming, so the timer has to take over
        // again or one bad file stops the cycle for the rest of the evening.
        schedule.abandon_change();
        assert!(schedule.tick(Duration::from_secs(2)));
    }

    #[test]
    fn refreshing_keeps_the_current_image_showing_when_it_survives() {
        let dir = scratch("wallpaper-tests");
        packs_of(&dir, &["a.png", "b.png"]);

        let mut playlist = Playlist::scan(&dir, &[]);
        assert_eq!(playlist.len(), 2);
        playlist.advance();
        let showing = playlist.current().cloned();

        // A third image appears while the machine is running, in a pack of its own.
        packs_of(&dir, &["c.png"]);
        playlist.refresh(&dir, &[]);

        assert_eq!(playlist.len(), 3);
        assert_eq!(
            playlist.current().cloned(),
            showing,
            "the image on screen should not jump when the folder is rescanned"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refreshing_falls_back_to_the_start_when_the_current_image_is_gone() {
        let dir = scratch("wallpaper-removed");
        packs_of(&dir, &["a.png", "b.png"]);

        let mut playlist = Playlist::scan(&dir, &[]);
        playlist.advance();
        std::fs::remove_file(dir.join("b.zip")).expect("remove");
        playlist.refresh(&dir, &[]);

        assert_eq!(
            playlist.current().map(ImageSource::name).as_deref(),
            Some("a.zip/a.png")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refreshing_holds_a_zipped_image_on_screen_and_notices_a_new_archive() {
        let dir = scratch("wallpaper-zip-refresh");
        write_zip(
            &dir.join("b.zip"),
            &[("1.png", png_bytes(4, 4)), ("2.png", png_bytes(4, 4))],
        );

        let mut playlist = Playlist::scan(&dir, &[]);
        playlist.advance();
        let showing = playlist.current().cloned();
        assert_eq!(
            showing.as_ref().map(ImageSource::name).as_deref(),
            Some("b.zip/2.png")
        );

        // A whole archive is dropped in, sorting *before* the one on screen — the position has to
        // follow the image rather than the index.
        write_zip(&dir.join("a.zip"), &[("1.png", png_bytes(4, 4))]);
        playlist.refresh(&dir, &[]);

        assert_eq!(playlist.len(), 3);
        assert_eq!(
            playlist.current().cloned(),
            showing,
            "the image on screen should not jump when a new archive appears"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_loader_reports_an_unreadable_file_rather_than_dying() {
        let loader = Loader::start();
        assert!(loader.request(
            Path::new("definitely/not/an/image.png"),
            100,
            100,
            Fit::Cover
        ));

        // The loader replies rather than panicking, and stays alive for the next request.
        let outcome = wait_for(&loader);
        assert!(outcome.is_err(), "expected an error, got {outcome:?}");
    }

    #[test]
    fn the_loader_reports_a_missing_archive_entry_rather_than_dying() {
        let dir = scratch("wallpaper-zip-missing-entry");
        let archive = dir.join("pack.zip");
        write_zip(&archive, &[("1.png", png_bytes(4, 4))]);

        let loader = Loader::start();
        assert!(loader.request(
            ImageSource::Zipped {
                archive,
                entry: "gone.png".to_owned(),
            },
            100,
            100,
            Fit::Cover
        ));
        let outcome = wait_for(&loader);
        assert!(outcome.is_err(), "expected an error, got {outcome:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_loader_decodes_and_downscales() {
        let dir = scratch("loader-tests");
        let path = dir.join("big.png");

        // A deliberately oversized image, to prove it comes back at the requested size.
        std::fs::write(&path, png_bytes(800, 600)).expect("write");

        let loader = Loader::start();
        assert!(loader.request(path.as_path(), 200, 100, Fit::Cover));

        let loaded = wait_for(&loader).expect("should decode");
        assert_eq!(
            (loaded.width, loaded.height),
            (200, 100),
            "cover fills exactly"
        );
        assert_eq!(loaded.rgba.len(), 200 * 100 * 4, "tightly packed RGBA");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_loader_decodes_an_image_out_of_a_zip_exactly_as_it_does_a_file() {
        let dir = scratch("loader-zip-tests");
        let archive = dir.join("pack.zip");
        write_zip(&archive, &[("nested/big.png", png_bytes(800, 600))]);

        let source = ImageSource::Zipped {
            archive,
            entry: "nested/big.png".to_owned(),
        };
        let loader = Loader::start();
        assert!(loader.request(source.clone(), 200, 100, Fit::Cover));

        let loaded = wait_for(&loader).expect("should decode");
        assert_eq!((loaded.width, loaded.height), (200, 100));
        assert_eq!(loaded.rgba.len(), 200 * 100 * 4);
        assert_eq!(
            loaded.source, source,
            "the image says which archive entry it is, so the display can name it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every image the playlist holds, in order, by the name the display would show.
    fn names(playlist: &mut Playlist) -> Vec<String> {
        let mut seen = Vec::new();
        for _ in 0..playlist.len() {
            if let Some(current) = playlist.current() {
                seen.push(current.name());
            }
            playlist.advance();
        }
        seen
    }

    /// An extra file is shown **as well as** the folder's contents, never instead of them.
    #[test]
    fn extra_files_are_added_to_the_folder_rather_than_replacing_it() {
        let dir = scratch("wallpaper-extra");
        packs_of(&dir, &["in-folder.png"]);
        let outside = dir.join("elsewhere");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let named = outside.join("named.png");
        std::fs::write(&named, b"x").expect("write");

        let mut playlist = Playlist::scan(&dir, std::slice::from_ref(&named));
        assert_eq!(playlist.len(), 2, "the folder's own image is still there");
        let shown = names(&mut playlist);
        assert!(
            shown.iter().any(|name| name.contains("in-folder")),
            "{shown:?}"
        );
        assert!(shown.iter().any(|name| name.contains("named")), "{shown:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file both named and sitting in the folder is one image, not two.
    ///
    /// The case an owner reaches by naming something and then tidying it into the folder, and the
    /// one a plain concatenation shows twice — once every cycle, for ever.
    #[test]
    fn a_file_both_named_and_in_the_folder_appears_once() {
        let dir = scratch("wallpaper-extra-dup");
        let both = dir.join("both.png");
        std::fs::write(&both, b"x").expect("write");

        let playlist = Playlist::scan(&dir, std::slice::from_ref(&both));
        assert_eq!(
            playlist.len(),
            1,
            "one file is one image however it is reached"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An extra that is not there contributes nothing, exactly as an unreadable folder does.
    ///
    /// Silently, because this comes from a `debug.` list: a typo shows up as a picture that never
    /// appears rather than as anything a singer should have to read.
    #[test]
    fn an_extra_that_is_not_there_is_ignored() {
        let dir = scratch("wallpaper-extra-missing");
        packs_of(&dir, &["real.png"]);

        let playlist = Playlist::scan(&dir, &[dir.join("nope.png")]);
        assert_eq!(playlist.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A zip named as an extra is a folder of wallpapers, exactly as one in the folder is.
    #[test]
    fn a_zip_named_as_an_extra_contributes_its_images() {
        let dir = scratch("wallpaper-extra-zip");
        let outside = dir.join("elsewhere");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let archive = outside.join("pack.zip");
        write_zip(
            &archive,
            &[("one.png", b"x".to_vec()), ("two.png", b"x".to_vec())],
        );

        let playlist = Playlist::scan(&dir, std::slice::from_ref(&archive));
        assert_eq!(
            playlist.len(),
            2,
            "a zip is a folder of wallpapers wherever it is named"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
