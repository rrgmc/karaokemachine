# Assets — icons, wallpapers, pictures

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## The asset directory

The three bundled assets are the SoundFont, the display font and the wallpaper folder. They were once
working-directory-relative string literals in three crates. That is wrong off a developer's shell in
two ways. An installed build launched from a menu or a service manager has whatever working directory
its launcher chose. On Android the working directory is `/`. The machine would come up with a test
tone, a system font and a gradient, all silently, on a device where the assets were present.

`Paths` carries an `asset_dir` beside `config_dir` and `data_dir`, and `Paths::asset()` resolves below
it. It is **absolute in every case**, so `--show-paths` names somewhere an operator can go and look:

| Case | `asset_dir` |
|---|---|
| Executable has an `assets` sibling | that directory |
| Desktop otherwise | `<working directory>/assets` — which is what keeps `cargo run` working, since `target/debug/` has no `assets` sibling |
| macOS bundle | `../Resources/assets` from `Contents/MacOS`, which is for executables only |
| Android | `<app-private>/assets` |
| `--data-dir DIR` | `DIR/assets`, plus the checkout overlay, which the command line asks for by name |

**Neither `km-display` nor its callers guess.** `find_font` takes the bundled path as an argument,
and `WallpaperConfig::dir` defaults to *empty* rather than to `assets/wallpapers`. A library that
draws what it is given cannot know where the application installed its files. An empty folder is
already a supported state. So a wrong guess was strictly worse than none, since it looked like a
configured folder that happened to be empty.

**`wallpaper.dir` in `settings.json` is `null` by default**, meaning "resolved by the rules". Writing
the default out as a path was the subtler half of the same bug. It was correct on the machine that
wrote it and wrong anywhere the asset directory moved.

**And the resolution is re-made at every rescan rather than at load time.** The rule chooses among
the three candidates *by contents*, and contents change. On a machine before its first picture, the
owner's folder is created empty on purpose. So a choice frozen at startup left the contents live and
the choice stale. Dropping an image in then scanned a folder that had already lost.

`Settings::wallpaper_config` and the display loop's rescan seam both ask `WallpaperSettings::folder`.
So there is one definition of the rule with three readers rather than a restatement at each.

## The local overlay

`assets/` is the tree every carrier ships. So it is the wrong place for anything a developer wants
only on their own machine. It was also the only place that worked, because `discover_asset_dir` has
no other answer for a `cargo run` from the repository root. Two things therefore went into releases
that nobody meant to put there: a built wallpaper pack, and any override SoundFont.

`Paths` carries `overlay_asset_dir: Option<PathBuf>` and `Paths::asset` prefers it. **The predicate is
four things, all of which must hold:**

1. Not Android. Two things enforce this, the structure and an explicit `cfg!` guard, so the property
   survives somebody rearranging `discover`.
2. `asset_dir` came from the working-directory branch, **or** the caller is the CLI's `--data-dir` path,
   which opts in by name.
3. `<cwd>/assets` is a directory.
4. `<cwd>/local/assets` is a directory, and is the overlay.

Condition 3 keeps this an *overlay* rather than a second asset source: it may only supplement a base
that exists. That keeps the "does not exist — run `fetch-assets.sh`" note of `--show-paths` truthful.
There is deliberately **no `.git` check**. In a worktree `.git` is a *file*, so an `is_dir` test would
disable the overlay in exactly the place `tools/dev/worktree.sh` seeds it.

**An installed build cannot reach it, and that is structural rather than a check.** Every carrier
puts `assets/` beside the executable: in `/opt`, in `Contents/Resources` or app-private. So the only
way in is an executable with no `assets` sibling, run from a directory holding both `assets/` and
`local/assets/`. In a checkout that is `target/debug/karaokemachine`. One crossover is deliberate and
has a test: a `Contents/MacOS` executable with *no* `Resources/assets` falls through to the working
directory. That is the "developer running the binary straight out of a bundle" case.

**`Paths::rooted_at` never enables it, and this is the part to preserve.** Of its call sites most are
tests, and tests run with the workspace root as their working directory. On a developer's machine
that root has both directories. So an overlay would make asset-resolution tests **fail on a
developer's box and pass on CI**. In the passing direction it would mean the suite opens a 206 MiB
bank. `Paths::with_checkout_overlay` is the opt-in and `cli.rs` its only caller.

**Resolution is per path, not per tree**, so an overlay holding only a SoundFont leaves the bundled
font and wallpapers alone. The test is `exists` rather than `is_file`. The code asks for
`wallpapers` as a directory and `soundfont/gm.sf2` as a file. One rule covering both is easier to
hold than two that nearly agree. The accepted cost: **a `local/assets/wallpapers/` replaces the
bundled folder.**

`asset()` touches the filesystem, which it did not before. Its callers resolve once at startup, so it
is single-digit `stat` calls per run. (The wallpaper *folder* is the exception and does not go
through `asset()` at all. As above, it is re-chosen at every rescan, because only contents can answer
the question it asks.) `androidassets::unpack` uses it as a *write* target. That is safe only because
the overlay is always `None` on Android.

**Nothing deletes the folder.** `local/` holds a large download, every worktree's data directory and
somebody's notes. So a `task clean` reaching into it would be a worse bug than the one this fixes.
What replaces a deleter is visibility: `--show-paths` prints an `overlay` line, and `run` logs it
once. `rm -rf local/assets` is the whole undo.

**The SoundFont half has been withdrawn.** The rule is untouched, and a hand-placed `gm.sf2` still
wins. But the overlay is a *wallpaper* mechanism now, because a bank put here is inaudible to every
build that is not a `cargo run`. Branches 1 and 2 return no overlay, and between them they are every
carrier. That was written down as a property to be proud of, and it was also the defect.

## The application icon

Generated by `cargo run -p km-display --example icon`, which is the **only** place the artwork is
defined. Byte-identical on every run, so regenerating is not a diff.

**Three layers: shards, plate, monogram.** Three straight-edged bands of color fill the tile, and a
near-black plate sits over them. `KM` stands on the plate: the K near-white, and the M in the hue that
names the program. The M is colored because `site/style.css` colors it: the icon is the site's own
wordmark with everything but the initials taken away.

**Signed distance fields, not a bitmap that gets scaled down.** Every stroke is a capsule and every
band a half-plane, evaluated per pixel. Coverage comes from the distance itself, which is exact
antialiasing from one sample. Two things fall out that a resampled bitmap cannot give.

- **Margins can change as the icon shrinks.** The plate is 62% of the tile normally and **grows** to
  80% at 32 pixels and below. A margin is a *fraction* of the canvas, so at 16 pixels that proportion
  is most of the icon.
- Terminals can be squared off by intersecting the letters with a cap-height band. That is why there
  is no rotation in the renderer at all.

**Three dead ends, because the obvious fix is wrong every time.**

1. *A hand-tuned small-size microphone with the cradle dropped* came out looking like a **trophy**. A
   rounded head over a thin stem and a wide foot is a goblet, and the cradle was the only thing
   saying microphone. Widening every stroke to a 2-pixel minimum then merged the cradle into the base
   at 16 pixels. What worked was the simplest of the three: **one drawing at every size, at nominal
   weight**, and let a 1-pixel cradle be soft. The rule outlived the microphone.
2. *A drawn singer* was a profile with the head tilted back and a microphone at a lifted chin. It read
   well at 48 pixels and up and was a smudge below that. Two things about building it are worth
   keeping even though it went. The hole between the forearm and the chest was the only thing saying
   "arm" at small sizes. The seam separating the microphone from the face had to come out of the
   *face alone*. Carving it out of the whole figure cut the hand off the microphone it was holding.
3. *Dropping the plate at small sizes* is the obvious way to simplify. It **inverts the icon**: the
   plate is where all of the letters' contrast comes from. It grows instead.

**Three palettes, one drawing.** The machine leads with the theme's amber, the builder with its blue
accent, the remote with its green. `render` takes that one color, and nothing else in the example
knows which program it is drawing for. So the three cannot drift into three designs. The hue reaches
exactly two places, the widest band and the M, and a test pins the rest as byte-identical across all
three.

**And one badge, on a second axis.** `render`'s other parameter is a `Badge`, which says how the
program was *started* rather than which program it is. `Badge::Stream` draws a source and two waves
in the strip of plate below the letters' baseline, in the lead it was already given.

The quadrant
those waves are cut to is an intersection with two half-planes. That is the same trick as the
monogram's cap-height band, and it is why the renderer still has no rotation in it. The badged mark
goes to the launchers that pass `--stream`, and to the icon bar of a run that was. Everything else
keeps the plain one.

The decision is
[`A badge says how the machine was started`](../decisions/interface.md#a-badge-says-how-the-machine-was-started).
`accent_alt`, `icon_ground` and `icon_glow` are the colors in `Theme` no screen draws. They live there
rather than as literals because of the rule against drift. That rule keeps the icons from drifting
from the app: *every* color in the drawing is a theme color.

**The ground is `icon_ground`/`icon_glow` and not `background`.** It was `background` until a
measurement found the tile rendering black. A dim indigo glow peaked at `rgb(28, 33, 73)`. The
vignette's 0.30 coefficient multiplies a squared distance reaching 1.4 at a corner, and so takes the
corners to `rgb(5, 6, 10)`. The two grounds answer different questions. A television's sits behind a
photograph with lyrics over it and must not spend luminance, and a launcher tile's has nothing over
it.

So they are two fields, and `the_icons_ground_is_not_the_televisions_background` in `theme.rs` is
what stops them being folded back into one. `background` *is* the plate, and that is not the
distinction collapsing. A television ground and a plate are both dark **objects**, and a ground is
what they could not share.

**The contrast constraint flipped direction with the layers, and that is the thing to know.** While
the mark stood on the ground, the ground had a **ceiling**. The palest mark capped it, which is what
forced `accent` up from `#4FC3F7`. The mark stands on a plate now and clears it by 15:1, so the
ceiling is gone. What replaced it is a **floor**, and it caught a real defect. The darkest band as
`icon_ground` neat measured **1.2:1** against the plate, so the plate's top-left edge was simply not
there.

`src/icon.rs` holds both ends now: 4.5:1 for a letter against the plate, and 1.8:1 for the plate
against every band. Two jobs need two numbers: a letter has to be read, and a plate edge only has to
be seen.

Where each ends up:

| Platform | Artifact | Wired up by |
|---|---|---|
| Windows | `.ico` compiled *into* each executable | `build.rs`, via `winresource` |
| macOS | `.icns` in `Contents/Resources` | the bundle staging helper |
| Linux | PNGs into `hicolor`, plus a `.desktop` entry | `[package.metadata.deb]`; `--register` for the tools |
| Android | adaptive layers + legacy mipmaps | `res/mipmap-anydpi-v26/ic_launcher.xml` |
| Every SDL window | `icon-256.png`, compiled in | `km_display::set_window_icon` |
| Each web page | a 32px PNG, compiled in | `GET /icon.png` |
| The three tool windows | that executable's *own* resource, by ordinal `1`, at **two** sizes | `km_webshell::with_icons` |
| A streaming run's icon bar | the badged mark — ordinal `2` on Windows, a 32px PNG on macOS | `km_tray::Spec::icon_ordinal` |
| The launchers that pass `--stream` | the same badged mark, by icon index, `CFBundleIconFile` and `Icon=` | `installer.iss`; `Info.stream.plist`; the desktop action |

**The window icon and the executable's icon are not the same thing and both are needed.** A shell
reads the resource in the binary and the `.desktop` file before the process exists. Neither helps a
portable folder copied somewhere and run. The package builder demonstrated it the hard way by having
one and not the other. The shell drew the executable's mark in the taskbar and Alt-Tab, while the
title bar showed Windows' default. The cause is that `tao` registers its class with both icon slots
null and then sends `WM_SETICON` twice with zero, actively clearing them.

Reading the icon back out of the running executable by ordinal is what makes the fix free: no
decoder, no dependency, no second copy.

**A Windows *window* has two icon slots, not one, and filling half of them is worse than filling
none.** `ICON_SMALL` is the title bar; `ICON_BIG` is the taskbar button and Alt-Tab. The shell
resolves a taskbar icon by walking `ICON_BIG` → the class icons → `ICON_SMALL` → the executable's
resource. While `tao` had zeroed both window slots, that walk ran to the end, and Windows picked the
right frame out of the `.ico`. That is why the paragraph above could say the switcher looked correct.
It is also why setting `ICON_BIG` looked like it would change nothing.

It was load-bearing. Filling `ICON_SMALL` with an exact 16 stopped the walk one step early. All three
tool windows then began stretching a 16-pixel drawing into a 24-pixel taskbar button.

So both are set, at sizes that differ on purpose. The small metric goes to the title bar.
`LR_DEFAULTSIZE`, the large metric, DPI-scaled, goes to everything else. This is the one piece of the
shells that lives in `km-webshell` for a reason other than duplication. The same wrong sentence lands
in all three `desktop.rs` files, and a mechanism nobody gets right twice belongs in one place.

**The shell chooses the remote's favicon, and had to.** `km-remote-pages` is linked into both the
machine and the offline remote, so a constant there is one favicon for two programs. Both PNGs live
in that crate, and `Remote::with_icon` is the offline shell's way of saying otherwise. It is
deliberately **not** a `Capabilities` field. That struct is what a mode can *do*, and which picture a
tab shows is not one of those.

**`icon/` is not under `assets/`.** `assets/` is shipped, and nothing in `icon/` is read at run
time: the two icons a running process wants are compiled in. So putting them there would ship 730 KB
of dead weight to every user.

**Two containers are hand-written** rather than taken from a crate. A `.ico` is a 6-byte header, a
16-byte directory entry per image and PNG payloads. An `.icns` is a magic word, a length and
type-tagged chunks. Each is less code than the dependency would be.

**A theme name with no file behind it is an error nowhere.** So a `.desktop` entry can say
`Icon=km-package-builder` with nothing ever writing that file, and draw a generic icon indefinitely.
`gtk-update-icon-cache` needs `-qtf` where the other refresh commands need no flags. A per-user theme
has no `index.theme`, and the command refuses the directory outright without `-f`.

**One known gap: Wayland may show a generic icon.** X11 matches a `.desktop` file to a window by its
`WM_CLASS`, which `StartupWMClass=` covers. Wayland matches on the surface's `app_id`, which SDL sets
from `SDL_SetAppMetadata`, and `sdl3-rs` 0.18 exposes no wrapper for it.

## Default wallpapers

**Seven CC0 photographs** in the committed `assets/wallpapers/default-wallpapers.zip`, with
`CREDITS.md` beside them. 1.04 MB — *smaller* than the gradients they replaced.

**A karaoke wallpaper has an unusual brief.** It must be interesting enough to look at for three
minutes, and plain enough that outlined white lyrics stay readable. That rules out most photographs.

Both halves of that are answerable rather than binding. CC0 and public-domain photographs may be
redistributed outright, which is exactly what a committed asset needs. Measurement decides
legibility, not taste. Of 103 CC0 candidates, **33 cleared the gate**.

**The gate is necessary and not sufficient**, which is the part worth writing down. Every one of the
33 was then looked at, and the rejections were things no ratio can see. One was an archival scan with
the album mount in frame. Others were an aurora full of TV antennas and a crop that is a close-up of
wet rock. One more was a "waterfall" that is really a portrait of a man on a bridge.

Seven ship rather than eight because the eighth was never good enough; a short set beats a padded
one.

**The gradient example stays and is not vestigial.** It is the one set that cannot fail the gate,
because it is drawn against the gate rather than measured after the fact. So it is what to reach for
if a photograph ever has to be withdrawn.

**One counter-intuitive finding.** Those gradients are shallow enough to band on an 8-bit panel, so
they want a dither. But a per-pixel dither **cost seven times the file size**, taking the set from
1.0 MB to 7.5 MB. It destroys the horizontal runs PNG's filters rely on. Dithering by *row*
keeps each row internally smooth, and costs 18%. It still breaks up contours anywhere they are not
perfectly horizontal.

## The README's pictures

Eight committed PNGs, regenerated by `tools/dev/screenshots.sh`, and one animated WebP,
`screen-singing.webp`, regenerated by `tools/dev/screen-animation.sh`.

**This is a committed generated binary that is *not* reproducible**, unlike the wallpapers. The
pictures depend on the discovered system font, on the corpus and on Chrome's version. So regenerating
on a different machine rewrites 2.9 MB for no product change. **Refresh them deliberately — at a
release, or when a screen actually changes — never routinely, and never from a container.**

**The television pictures are taken over the shipped wallpaper pack**, not over a generated
gradient. They use the first image `Playlist` yields from `default-wallpapers.zip`, which is what a
fresh install shows. That is also where the 2.9 MB came from. It was 1.3 MB while the background was
a smooth gradient, and a starfield does not deflate. The per-file and total budgets in
`tools/dev/screenshots.sh` were raised to match, deliberately. See the
`Which wallpaper the published pictures are taken over` decision.

**A machine without the corpus must not be able to publish.** The pictures show a real catalog
because a remote listing `Song 1001 / Even Artist` advertises a demo. That material is machine-local.
So with no corpus the script renders into `target/screenshots` and leaves `docs/images` untouched.

**The animated picture needs no corpus, and still depends on the font.** `screen-animation.sh` reads
a carol out of the released pack, so it runs on any machine with the network. The display draws in
the font it discovers, though. A Linux run draws DejaVu Sans and a Windows run draws Segoe UI. So
regenerate it on the machine that takes the stills, and the two agree.

**The catalog is curated, and the curation is the whole mechanism**: a hand-picked set with a
hand-authored `index.csv`. There is no automatic filter, and there should not be one. "Worth
photographing" is a judgment. A rule that inferred it from some file in the tree would make editing
that file silently repaint the README.

Three couplings, each invisible until it bites:

- **Language comes from `index.csv` and may never be inferred.** `@LENGL` is the Soft Karaoke editor's
  *default* rather than an assertion, so most non-English `.kar` files declare English. The
  hand-authored column is the only thing in the pipeline that knows.
- **The language picker is hidden below two languages.** So an English-only catalog would delete a
  control from two published pictures and falsify the README's alt text. A handful of other-language
  songs stay in, and the list captures pass `?language=en`. That is deterministic, where relying on
  alphabetical order is luck.
- **The script chooses the staged songs folder, not the operator.** The builder prints the absolute
  path it is scanning across its header, and that header is inside a published picture. So for as
  long as the operator chose it, every committed copy named somebody's own folder.

**The scratch tree is state, and a run clears every part of it that a picture can see.** The catalog,
the offline remote's two databases and the packages folder are each emptied before they are filled.
The reason is that what survives a run is a build of some earlier format. The machine reports a
package the current manifest cannot read as a red banner across the top of the remote's own page.
That page is one of the published pictures.

**The package id is generated, so the bank is pinned to the id this run produced.** A package's bank
comes from its id, and the id is sixteen random characters. So a bank left to follow it is a
different thousand every run, while the queue the display draws is written down. The script reads the
id back out of the description it just wrote and names it in `package_banks`.

Four things about driving headless Chrome, none of them facts anybody remembers:

- **Chrome resolves a relative `--screenshot` path against something other than the calling shell's
  working directory.** Chrome then writes nothing and exits 0.
- **The remote holds an `EventSource` open forever**, so headless Chrome never decides the page has
  finished loading. `--virtual-time-budget` does not help: it is a real socket, not virtual time. The
  script fetches the pages with `curl` and captures them from a copy with that one script tag removed.
  The server renders the whole page and SSE only patches it afterwards. So what is photographed is
  what a phone is sent.
- **Headless Chrome will not give a viewport narrower than 500 CSS px.** Asking for a 390 px phone
  silently lays out at 500 and crops, which looks like a broken stylesheet. Hence the assertion on
  each output's pixel dimensions.
- **A color scheme is each page's business, not the capture script's.** Forcing
  `preferredColorScheme` reached past the page it was aimed at. There is now no single value the flag
  could take: the remote is pinned light and the builder dark. Each page declares its own
  `color-scheme` for the form controls that follow one.

## `km-wallpaper-pack` — photographs that lyrics survive

The generated gradients solve legibility by construction. This tool solves it by **measurement**: a
WCAG contrast ratio computed in the band where the lyrics actually are.

**Its original framing ran two halves in parallel**, and the symmetry was the bug. The two halves
were *stock APIs settle the license, and a contrast ratio settles legibility*. One half was measured
and the other assumed. Reading as a matched pair is what made the assumed half look like a finding.
See *The license was not settled by the API* below.

There are three phases, and two of them never touch the network. `fetch` fills a content-addressed
cache, and `analyze` and `build` are pure functions of it. That split is the whole ergonomics. A
threshold can be re-tuned twenty times in an afternoon on a corpus that cost an hour of quota once.

**The gate is the app's own scrim, and the pack darkens nothing.** The display already lays a 45% dim
over every wallpaper, so the solver runs as a *filter*. It finds the smallest alpha reaching the
target contrast. An image ships when that alpha is no more than the app will apply anyway. Baking a
second scrim into the JPEG would darken twice and override a setting the singer may change. The
manifest keeps `required_alpha` regardless, because re-deriving it means re-downloading the original.

**Three numbers come from the app, not from taste:**

- **The band is 0.40–0.68 of image height**, because the display draws its two lyric rows at 0.42 and
  0.55. The obvious guess, 0.55–0.95, is the bottom of the screen. This app draws nothing there but
  `next:` and a progress bar, so a pack built to it measures the wrong stripe of every photograph. A
  test in `km-display` fails if the rows move out from under that band. The tool does **not** depend
  on `km-display`: that would drag SDL3-from-source into a build-time asset tool.
- **The text is `#ECEFF4`, not white**, with a relative luminance about 0.84. So measuring against
  1.0 overstates every ratio by a seventh. The outline is deliberately not modeled. It only ever
  helps, so leaving it out keeps the gate conservative.
- **Measuring the 16:9 frame is the worst case for every screen**, since `Fit::Cover` crops to fill.
  So wider and narrower displays both see subsets of the band measured here.

**The compositing model and the luminance model have to agree.** Darkening happens in sRGB; relative
luminance is defined on linearised channels. Getting that backwards makes every solved alpha wrong in
the direction of too light. There is no closed form, so the alpha is bisected. The early return
checks the target at `max_alpha` rather than assuming the interval brackets a root. A background
*brighter* than the text inverts the monotonicity the search relies on.

**Busyness is rejected separately from contrast**, because they fail differently. A calm bright sky
can be darkened into readability, and a detailed hedge cannot at any alpha. One fixture proved the
point twice: a *one-pixel* checkerboard has a Sobel response of **zero**. The columns either side of
any pixel are identical.

**It is not a member of the main workspace, and TLS is why.** Cargo unifies features per crate
version across a workspace build. So making it one handed `km-package-builder` half a TLS stack. Its
`Client::new()` panicked with "no rustls crypto provider is configured", and only when the workspace
was built together. The same source produced two different binaries depending on how it was built,
and that made this a decision rather than a fix.

`tools/cmd/assets` has `exclude` in the root manifest, its own lockfile, and **its own CI step**. Excluded from the workspace means excluded from
every check the workspace runs.

**It lives in a second workspace rather than being one.** The distinction started mattering when
`km-admin` arrived wanting the same `reqwest` with the same features. `tools/cmd/assets/Cargo.toml`
is a virtual root holding both. `km-wallpaper-pack` inherits its lints, its metadata, its dependency
versions and its `version` from there.

**Unification is the thing being avoided across the exclusion boundary and the thing being relied on
inside it.** Two excluded workspaces would have meant two lockfiles that must agree about `image` and
`zip`, with nothing to make them. That is the drift this crate's own dependency comment already
exists to prevent. One root also means one CI job, one `rust-cache` key and one `cargo fmt` line. It
means one `rust-version` for the toolchain-bump checklist and one `version` for the release one.

**Both programs carry the machine's version.** A staged folder is named from the version the binary
reports, and looked for using the version a manifest reports. So a program on a number of its own
needs an arm in every script that walks `dist/`. `tools/dist/clean.sh` was given both and
`tools/dist/bin.sh` one. km-admin was staged under its own number and then reported missing under the
machine's. The version is written down exactly twice, once per workspace root, because nothing is
inherited across an `exclude`.

`tools/dev/check-version-pin.sh` holds the two equal, the same way `check-toolchain-pin.sh` holds
`rust-version` one line below it.

Its `rustls` is `rustls-no-provider` plus an explicitly installed **ring** provider, because
aws-lc-rs does not build against this machine's MSVC. The provider is installed inside the one
function that builds an HTTP client. So no caller can forget it and get a handshake failure instead.

### What it takes to drive from something other than a command line

Three seams arrived when `km-admin` began driving the same phases behind a browser. Each replaces an
assumption that was invisible while a terminal was the only caller.

**A key is the caller's, not the environment's.** Every `Provider::new` read `std::env::var` itself.
That is exactly right for a command line, and **unreachable** from a program whose user types the key
into a form. `std::env::set_var` is `unsafe` in edition 2024, and this workspace denies `unsafe`. So
there was no way to supply one at all. `providers::Keys` is the type, and `Keys::from_env()` is the
one place the old contract now lives.

`main.rs` calls it, so `PIXABAY_API_KEY` and the other two behave exactly as documented. The
`with_key`/`with_token` constructors the wiremock tests already drove were the shape of the
answer. This makes them the ordinary path rather than the test-only one. `ProviderKind` gained
`key_page()` and `needs_key()` at the same time. A program with a form has to say *where to get a
key* before the failure, rather than inside the error it produces.

**A phase can be watched as well as logged.** `progress::Sink` is `Option<&dyn Fn(Tick) + Sync>`. It
is `None` at a command line, where `tracing` every two hundred images is the right answer and is
untouched. `Sync` is not decoration: the measuring loop is `rayon`'s `par_iter`, so ticks arrive from
every worker at once.

`total: 0` means *unknown*, which a bar draws as indeterminate. `fetch` cannot know how many
originals it will pull until every search has answered. A total it has not established would make
the bar jump backwards. The download counter counts what was **attempted** rather than what succeeded. So a 404 from a CDN
does not read as a stall.

**A reader needs a smaller picture than the pipeline does.** `Cache::thumb`/`put_thumb`/`thumb_path`
put a JPEG preview under the same `--cache-dir` as `originals/` and `gray/`. Nothing in the three
phases reads one; the review grid does. Serving originals to a hundred and twenty tiles would push
several hundred megabytes through a webview, to draw thumbnails a few hundred pixels wide.

It is deliberately **not** keyed on `Config::measurement_hash`. A thumbnail is a picture of the original,
so no `[legibility]` or `[filters]` value can change what it should look like. Adding it costs no
re-measure.

### Four failures worth keeping the lesson from

- **A Hamming distance is only meaningful against the width of the hash.** A run over 4,682 images
  chose **6** against a target of 120. `perceptual_hash` produced 22 bits while its doc comment said
  64. `DoubleGradient` resizes to `(W/2+1, H/2+1)`, so `hash_size(8, 4)` emitted 22 comparisons. They
  went into a `u64` through a `take(8)` that silently accepted three bytes.

  `distance ≤ 10` is the standard near-duplicate threshold *for 64 bits*. Over 22 bits it declares
  two-fifths of unrelated pairs identical. **Why the tests missed it is the more useful half.** Every
  dedupe test hand-wrote 64-bit literals, so they exercised a bit space the real hasher never
  produced, and *no test anywhere called `perceptual_hash`*. The new ones go through the real hasher
  and assert the statistical property the threshold depends on. `Config::validate` now refuses a
  distance at or beyond a third of the hash width.
- **Identity must be measured on the photograph, not the treated picture.** The hash was taken after
  the blur and vignette. So a shared treatment became shared bits on a pool that was already all dark,
  all blurred and all 16:9. `render` is split into `canonical` (crop and resize) and `treat`, with the
  hash taken between them at no extra cost.
- **A dev profile in the wrong file.** An `analyze` run takes 78 minutes at opt-level 0. A crate
  declaring a bare `[workspace]` is **its own workspace root**. A workspace root inherits no profiles
  from the directory above it, so `[profile.dev.package.image]` has to be written twice.

  **The stanzas live in `tools/cmd/assets/Cargo.toml`**, which is the same lesson one level up. Cargo
  ignores a *member's* profile stanzas outright. So a crate moving under a shared root takes them with
  it rather than leaving a copy behind. `Finished dev profile [optimized + debuginfo]` is the line
  that says they took effect.

  It is 118 s cold and 24 ms warm now. The rest comes from memoising the measurement against
  **`Config::measurement_hash`, deliberately not `Config::hash`**. That key covers the output size
  and the legibility table and nothing else. So re-tuning a `[filters]` threshold re-decodes nothing.
- **A gate can read the wrong number and still look like it is working.** `min_source_width = 2400`
  was checked against the *provider's metadata* width. Pixabay's `largeImageURL` is capped at 1280,
  whatever the photograph's real size. So 4,682 files, every one of them 1280 wide, pass, and a 4K
  config upscales them 3×. What answers it is a *second* gate, `min_decoded_width`, with a rejection
  label of its own. The original key is genuinely right at search time, and sharing a label would
  hide the same confusion again.

**`--force` replaces rather than overlays.** Walking the output directory is only correct if it holds
exactly one pack. It never does, since output names carry the selection index and a hash. The walk
swept every image of every earlier build into the new zip. Nothing downstream could catch it, because
`verify` iterates the manifest and never the directory. A stale wallpaper shipped, was displayed, and
had passed no gate anybody could re-check.

The zip is assembled from the manifest's entries now. `manifest::pack_artifacts` is the single
definition of what a pack consists of, and both the guard and the sweep read it. **One definition,
because two would drift, and the drift would surface as a stale image in a deliverable rather than as
an error.**

### The license was not settled by the API

**What the terms actually say.** Pixabay's Content License bars distributing content "on a Standalone
basis", and its ToS names the form: *"as a print, **wallpaper**, poster or on merchandise"*. Pexels'
API guidelines bar *"making Pexels content available as a wallpaper app"*. Neither turns on money.

**The design smell, which is the transferable part.** `ProviderKind::license() -> &'static str`
returned one license per site. That is not merely a place where the wrong answer was stored. It is a
type that **cannot represent** a per-image license, and a type that cannot represent the truth is how
a question stops being asked. Nobody looked up what a pack may be used for until an aggregator made
the constant impossible to keep.

The fix has three parts.

- `manifest::Entry` carries `license` and `license_url` per image, with `provider` demoted to
  optional metadata.
- `ATTRIBUTION.md` groups by license rather than by provider. That is also the better file, since
  what a reader needs first is what they may *do*.
- It opens with the statement that every image was modified, because CC BY requires that and
  `render` has no path that skips it. A test pins that notice. So a future fast path has to revisit
  the claim rather than leave it quietly untrue.

**Both providers stay.** A pack somebody builds with their own keys for their own machine is what the
tool is for, and no distribution occurs.

### `local` — the set that ships

`commands::local` builds a pack from a folder somebody chose and a `credits.toml` beside it. The
three-phase pipeline exists to get a hundred usable photographs out of thousands nobody will look at.
That is the wrong shape for seven images chosen by eye, whose provenance is typed rather than parsed.

- **The gate is the same gate**: the same measurement and the same manifest. That is what lets
  `verify` re-check a shipped set exactly as it re-checks a pack.
- **No config file.** The default legibility values are read off the app's
  own theme, so they already *are* what a set destined for `assets/` must be measured against.
- **A rejected image stops a real run and only warns under `--dry-run`.** Choosing eight of a hundred
  needs every verdict at once; building the set somebody settled on must not silently drop one.
- **The sidecar is checked three ways, and all three are errors.** They are an image the sidecar does
  not mention, an entry naming a file that is not there, and one file described twice. Each ends with
  a shipped image whose license nothing records, which is the defect this command exists to prevent.

**Two sourcing traps worth knowing before writing another provider.** An aggregator's
`width`/`height` may describe the **work** while its `url` is a scaled rendition. That is why a
client-side decoded-width gate is mandatory rather than belt-and-braces. And **the bytes do not come
from the API you searched**: Wikimedia, Flickr and museum servers do, each with its own rate limit. A
sustained sequential run failed 82 of 103 downloads until they backed off and retried.

## `km-admin` — the same pipeline, behind a page

**A caller, not a second pipeline.** `fetch`, `analyze` and `build` are `km-wallpaper-pack`'s own and do
exactly what they do from a command line. What is here is the plumbing a browser needs and a
terminal does not.

**That is all it is, plus a shell.** This program *hosts* the machine's own page rather than
carrying a second copy of it. It implements `km_admin_pages::machine`'s traits over its HTTP client,
and mounts the machine's own markup at `/admin`. So `layout.html`, `machine.html`, the output picker,
the send form and the stylesheet exist once.

[`admin.md`](admin.md) is how that seam is built. What
stays below is this program's own half, which is the searching. That is the part
`A fourth program, rather than a fourth tab on the owner's page` keeps off the machine, and the
reason this program exists at all.

**What is this program's in `static/` is htmx and 63 lines of `ui.js`.** They are declared to the
shared layout through `Admin::with_scripts` rather than written into markup. They are the one thing
the searching needs that the machine's page does not. A job that runs for minutes has to report
itself, and `_job.html` replaces itself once a second to do it. A tag written into a host's own
layout goes wherever that layout goes. That is why these are a field on the seam rather than a line
in a template.

**Three phases, one job, one button.** They are one errand: somebody asked for pictures. `analyze`
and `build` are cheap enough on a warm cache that splitting them into three buttons would be three
ways to be half-finished. The phase name is what says which part is running.

**And the button stops there.** Sending is a second button on a second row, because that is where
the errand really ends. See
[`Everything it makes is kept, listed, and sent as a second act`](../decisions/distribution.md#everything-it-makes-is-kept-listed-and-sent-as-a-second-act).
The handler captures no client at all, which is the mechanical half of it. Taking `state.client()`
when the request arrives hands the pack to whatever that was, minutes or an hour later.

**Two directories, and which one is cleared is the whole point.** `build` runs with `force`, and
`manifest::clear_pack` empties what it writes into. So the working directory and the shelf cannot be
the same place:

```
<data_dir>/
  banks/<CatalogBank::name>          a fetched bank, under the table's own filename
  pictures/cache/                      originals and thumbnails
  pictures/build/                      analyze + build write here; cleared by the next run
  pictures/packs/<zip stem>/           the zip, manifest.json, ATTRIBUTION.md
```

`run::keep` moves the three files after a successful build. The size-named directories of loose
images stay behind. The zip is the deliverable and already holds them. A copy beside it would double
what a pack costs on disk, for something nothing here reads. `analysis.json` also stays, in
`pictures/build`, because it is the *review* rather than part of a pack. `manifest::pack_artifacts`
has always said so, and `last_review` reads it from there.

**A folder is a pack because it contains a zip.** That is what makes a run killed between
`create_dir_all` and the move invisible, rather than a row offering a Send that could only fail.

**A pack id comes off a URL and a bank id does not**, which is the asymmetry the two remove routes
turn on. `km_banks::bank(&id)` maps an id to a row, and the *row* states the filename. So nothing a
caller types reaches the filesystem. A pack takes whatever name `zip_name` produced, and a route
hands that name back. So `pictures::pack_dir` demands a single `Component::Normal` equal to the id it
was given.

**A hand-written check refuses the backslash on top of that.** `Path::components` splits on what the build
target calls a separator. `a\b` is two components on Windows and one legal filename on Linux. So
without the check, the guard's answer would depend on which machine compiled it, and the test would
agree with the bug.

**The lists redraw themselves once, on the last poll.** `_job.html`'s `hx-get` lives inside the
`running` branch. So the response that reports `running: false` is the final one the browser ever
receives. It is exactly one, at exactly the moment what is on this computer has changed. It carries a
`hx-get` on `hx-trigger="load"` that replaces `#local`, which wraps both `_banks.html` and
`_packs.html`.

Without it a finished download leaves its own row still offering *Get*: the poll swaps `#job` and
nothing else. That was survivable while every button redirected. It is not survivable when the point
of finishing a download is a Send button appearing. Songs passes `None` and redraws nothing, having
nothing of its own to list.

**A form either has no `hx-post` or has a handler that answers `HX-Redirect`, and the login box is
the second kind.** Every control that changes a machine redirects to `/machine` afterwards. A browser
follows a 303 transparently. An htmx request follows it and then swaps the whole document into
whatever it was aiming at. That is why `_address_form.html` and `_discovered.html` are plain posts
and say so in their own comments.

The password form keeps its swap, because a wrong password is a non-2xx sentence and only an htmx
request turns that into a toast. So `handlers::reload` answers a success with `HX-Redirect` and 204
instead. Without it, signing in drew a second nav bar and a second machine panel inside the password
box. It is the same answer `km-package-builder`'s `require_workspace` gives the same collision.

**`analyze` and `build` go on `spawn_blocking`.** Both are `rayon`-parallel and CPU-bound for their
whole duration. Holding a tokio worker for minutes means the request polling for progress competes
for threads with the measuring it is asking about. The job crosses that boundary as an `Arc`.
Sending a borrowed reference by pointer is the obvious wrong turn, and this workspace denies
`unsafe` outright.

**A total of zero means *not known yet*.** `fetch` cannot know how many originals it will pull until
every search has answered. So the bar is drawn indeterminate, rather than at a number that would jump
backwards. The download counter counts what was **attempted**, so a CDN's 404 does not read as a
stall.

**Thumbnails, because the review grid is a hundred and twenty tiles.** They are 320px JPEGs kept in
the same cache as the originals: 12 KB against 480 KB, measured on a real run. They are deliberately
not keyed on `Config::measurement_hash`. A thumbnail is a picture of an immutable original. So
nothing in `[legibility]` or `[filters]` can change what it should look like, and adding it costs no
re-measure.

**Two size gates are on the page, and a first real run is why.** Against Openverse it searched 39
pictures, downloaded every one and rejected every one as `too_small_on_disk`. That is the gate
working, as the two-gate trap above explains. But a threshold nobody can see is a search that
silently returns nothing. So `min_source_width` and `min_decoded_width` are both fields, with the
difference between them spelled out beside the second.

**The bank half shares its rules and not its loop.** `km_banks::digest` holds which hash a row is
pinned with, the size slack, the `.part` name and the archive-member rule. The transfer is
`reqwest`-on-tokio here against `ureq`-on-a-named-thread in the machine. The two differ in the places
that decide a loop's shape: cancellation, and where the counter lives.

**One bank cannot be sent at all**: Orpheus is 1,288,303,498 bytes against `MAX_SOUNDFONT_BYTES`, a
gibibyte. The machine can fetch it itself, because its own downloader writes straight to disk with
no such ceiling. The page says so on the row rather than after a 1.2 GiB download. **That check stays
on the fetch even now that sending is its own step.** A bank this program could download and never
hand over is a gigabyte spent on a row whose only remaining button is Remove. The row saying so
beforehand is the answer that was already right.

**A bank already on the disk is not fetched again.** There was no such check while Get was also the
only Send. Pressing it twice was how somebody sent a bank twice, at the cost of the whole download
each time. With a Send button there is no reading of a second Get that is worth a gigabyte. Get is
not drawn at all once the file is here. Re-downloading is Remove and then Get, which needs no third
button and cannot be pressed by accident.

**`here` and `on_the_machine` are two facts and were one `else if`.** The badge showed *downloaded*
only when the bank was not on the machine. That was harmless while both were the outcome of a single
button. It is wrong now twice over. *Here but not sent* is precisely the state the Send button exists
for. *Here and sent* is the row that says the next machine in the house costs no download.

`landed_bank` is the one definition of *here*, so the badge, the send route and the remove route
cannot come to disagree.

**CORS never enters it.** The browser talks only to `km-admin` on loopback; `km-admin` talks to the
machine server-to-server. `api.cors_origins` ships empty and stays irrelevant. That is worth stating,
because it is the first thing anybody asks about a program that reads another program's API.

**Finding a machine is now `km_api::discover::watch::Watcher`, started once the socket is bound.**
The Look button answers from a snapshot and pokes the registry for a fresh query. There is no
`spawn_blocking` and no three-second wait. The wait existed because a house with two machines must
see both. A registry that has been listening since the program opened satisfies that. It is started
after the bind rather than in `State::new`, because a great many tests build one of those and
`Watcher::start` opens a multicast socket.

**It goes through `km_api::discover`, which is already a dependency.**
`_karaokemachine._tcp.local.` is written down once, in the crate that also advertises it.
`km-package-builder` and `km-remote-core` make the same arrangement. It is the reason none of the
three carries its own copy of the service name or of how to read an advertisement. It costs no new
crate here: `mdns-sd` and `if-addrs` already resolve into this workspace's lockfile through `km-api`.
That crate carries no HTTP client, so it cannot re-open the `reqwest` feature question the second
workspace exists to settle.

**It is a `recv_timeout` loop, so it goes on `spawn_blocking`.** Three seconds on a runtime worker
is three seconds of this program's own page not answering. That is the same mistake `analyze` and
`build` avoid above. The address in each row is the machine's own `url` TXT record, rather than a
guess from the addresses that arrived. `display_name` is what turns a blank advertised name into
something a row can show.

**No test calls `browse`**: opening an `mdns_sd` daemon joins a multicast group. Nothing here may
bind a non-loopback address in a test. A test binary's path carries a build hash, so every rebuild
would raise a fresh Windows firewall prompt and leave a dead rule behind. The tests render the
fragment instead, which is where the decision lives.

**The chosen machine is a `km_api::discover::known::Known` in `machine.json` under the data
directory**: id, address, name and when it last answered. The id comes from the `/discover` the
Machine page already makes, never from a choice. That keeps the rule below intact while letting the
program follow that machine to a new address. And the ladder in `State::new` is asked-for, then
remembered, then nothing.

The third rung of `km_remote_core::find::locate`, adopting what the network answers, is deliberately
absent. See `Finding the machine, and remembering which one it was` in
[`docs/decisions/distribution.md`](../decisions/distribution.md). `--machine` does not come through
`set_machine`, and so it is never written down. That is not an oversight: it is the reason the
constructor builds its client directly.

**Two spellings that have to be reconciled, both found by running it.** A bank is joined to the
machine's installed list by *stem*. The table calls a file `GXSCC_gm_033.sf2`, and
`SoundFontBankDto.name` is `GXSCC_gm_033`. So comparing them directly makes every row read "not on
the machine", however many times one was sent. And `Discovery::factory_password` must not come from
a `ConnectInfo` snapshot. Otherwise `GET /discover` reports the wrong thing about a machine whose
password was set at run time, and a client that trusts that answer stops asking.

## Sending a file that is already on this computer

**Not the pipeline above.** Nothing is searched, measured or built; the browser hands over bytes and
they go to the machine. There are three routes: `/songs/upload`, `/pictures/upload` and
`/sound/upload`. They are the same spellings the machine's own `/admin/` uses, so the two surfaces
read alike.

**Staged, then streamed, and the staging is a lifetime rather than a preference.** axum's multipart
`Field<'a>` borrows the `Multipart`, which borrows the request body. So anything reading it must run
inside the handler. Forwarding it straight into `reqwest` would therefore hold the *browser's*
request open for the whole machine-side transfer. There would be no progress bar, and the one-hour
upload timeout would apply to a browser rather than to a server-to-server call. A failure would
arrive after a gigabyte, with nowhere good to put it.

Staging to a file and sending that with `Part::stream_with_length` also restores the request's
`Content-Length`. `Form::compute_length` gives one only when every part states its length. That is
what lets the machine refuse an oversized upload before reading it. It fixed the two existing senders
on the way past: `Client::upload` read the whole file into memory, which is a gibibyte for a bank.

**Two names for one file, and the failure that makes it matter.** A counter this program owns names
the staged file, because *nothing is ever written under a name the network chose*. That is the rule
`bank.rs` states. It holds here even though the "network" is a local browser, since `--lan` exists.
What goes on the wire is the browser's own name, because the machine reads its **stem**.
`receive_upload` stages as `{safe_stem}.{extension}`, and a bank's stem becomes the name the Sound
page joins on.

Sending the staging name would install a SoundFont called `0`. `safe_stem` is `pub(crate)` in
`km-api` and is not reimplemented here. The machine sanitises what it is given, and a second copy of
that function would be a second thing to keep correct.

**The body limits are per route and come from `km_api::uploads::limit_for`.** Every other route here
takes a small form, for which axum's 2 MB default is the right guard. So a router-wide
`DefaultBodyLimit` would be a hole with no author. No slack is added for the multipart envelope. A
file of exactly the limit is refused here *and* by the machine. Slack would make this program accept
what the machine is about to refuse.

**A trip is read from `MultipartError::status()` and `body_text()`, never `Display`.** That is the
whole point of the paragraph on `km_admin_pages::router`. axum words a limit trip as a 413 and
*"Request payload is too large"*. Its `Display` is the useless *"Error parsing multipart/form-data
request"*. That is a parser complaint naming neither a size nor a limit, for a file that was
perfectly well formed.

**The machine reads it the same way now.** `multipart_failure` in `km-api` is the one place that
maps a `MultipartError`, and it answers 413 where axum's status says 413. So a file too big for the
machine reads the same whether it went to `/wallpapers` directly or through this program. Before,
it did not.

**And the limit is quoted from one function.** `km_api::uploads::limit_in_words` produces both the
"Up to 64 MB." above the chooser here and the "the limit is 64 MB" the machine refuses with. A page
and a refusal describing one number two different ways is exactly the drift the `uploads` module
exists to prevent.

**Nothing survives, and that needs two halves.** A guard removes the staged file when the sending
task ends: on success, refusal or panic. `staging::sweep` empties the folder at startup, which is the
only moment nothing is passing through. `Drop` cannot cover a kill, and `bank.rs`' `download` carries
the scar of exactly that. A broken connection left a partial file in a folder for ever, because one
failure path out of three forgot to clean up.

**A send reports through the section's own job.** So the answer to "did it land?" is the same bar as
everything else here: the handler returns `_job.html`, and the browser polls itself to the end. The
job is **unstoppable**. A send is one `reqwest` call with no loop of ours inside it, so a Stop button
would set a flag nothing reads. A control that never does anything teaches somebody to disbelieve the
ones that do.
