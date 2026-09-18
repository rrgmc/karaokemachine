# The application icons

Everything here is generated. Do not edit these files:

```sh
cargo run -p km-display --example icon
```

That writes this directory, **two** sets of Android launcher resources — the machine's under
`ports/machine/android/app/src/main/res/mipmap-*/` and the offline remote's under
`ports/remote/android/app/src/main/res/mipmap-*/` — and **one** iOS app icon, the remote's, at
`ports/remote/ios/KaraokeRemote/Assets.xcassets/AppIcon.appiconset/icon-1024.png`. The renderer is
[`crates/playback/km-display/examples/icon.rs`](../crates/playback/km-display/examples/icon.rs), and it is where the
design lives — everything is signed distance functions over a unit square, so one drawing serves a
16-pixel favicon and a 1024-pixel macOS icon rather than two that can drift apart.
Colors come from `km_display::theme::Theme`, so the icon tracks the app — **seven of them, and not
one is new**: the four leads are `lyric_sung`, `accent`, `accent_alt` and `icon_glow`, the two
supporting bands are mixed between `icon_ground` and `icon_glow`, the plate is `background` and the
`K` is `lyric_pending`. **Still seven with a fourth program**, which is the whole of what the fourth
lead cost: it had to reuse one, and the section below says which and what that costs. `icon_ground` and `icon_glow` are the icons' own and deliberately **not**
`background`; the fields say why, and the temptation to fold them back together is the thing to
resist. `background` doing duty as the *plate* is not that — a television ground and a plate are
both dark **objects**, where a ground is the thing they could not share.

Rendering is deterministic: regenerating produces byte-identical files, so re-running the example is
not a diff.

## Three layers, four palettes, one drawing

Every icon here is the same three layers: **angular bands of color** filling the tile, a
**near-black plate** over them, and **`KM`** on the plate — the K in the theme's near-white, the M in
the hue that names the program.

The letters are set at **85% of their own drawing**, about the middle of the plate — `monogram::SCALE`
in the example. At full size they left a tenth of the plate clear on each side, which read as a plate
the mark had outgrown at every size and on every platform; the vertical margin was never the tight
one, so the change is felt on the sides. It is one number scaling the *input* to `monogram_distance`
rather than a nudge per constant, because the letters are one coordinate system and moving the cap
height without the arms, the waist and the vertex would draw a different pair of letters rather than
smaller ones.

The **M is colored because the wordmark colors it.** `site/index.html` sets the name as
`Karaoke<span>Machine</span>` and `site/style.css` gives that span the amber, so the machine's icon
is its own wordmark with everything but the initials taken away. The other two are the same idea in
their own hue.

**A fifth mark carries a badge instead of a hue.** The machine is one program with two launchers —
one that draws a television and one given `--stream` — and a hue would say they were two programs.
`karaokemachine-stream-*` is the machine's own amber mark with a source and two waves added in the
strip of plate the letters leave empty, and outside that corner it is byte-identical to
`icon-*`, which a test holds. It goes to the launchers that pass `--stream` and to the Windows
notification area of a run that was, and nowhere else: the window icon, the favicons, the Plymouth
logo and the phone launchers have nothing beside them to be told apart from. The reasoning is
[`A badge says how the machine was started`](../docs/decisions/interface.md#a-badge-says-how-the-machine-was-started).

**A sixth mark is a silhouette, and it is the only file here with no palette in it.**
`karaokemachine-bar.png` is what the macOS menu bar is handed for a streaming run. That bar draws
*template images*: the system reads a picture's alpha and paints the shape itself, so one file is
right on a light bar, on a dark one, and inverted while its menu is open. None of the five marks
above can be one — each is a full-bleed rounded tile, so its silhouette is the tile — and this one
can only because the plate and the bands are left out.

**Leaving them out is what makes it belong there.** A plate is a dark object, and a dark object in a
dark menu bar is a hole; the letters are the mark's content, and content is what the glyphs beside
it are. So it is `monogram_distance` over the same coordinates the tile uses, cropped to the letters'
own box by `bar_box` — which is computed from `monogram`'s constants rather than typed out, so moving
a letter moves the crop with it.

**No badge, although it is only ever in the bar for a streaming run.** A badge separates two things
standing next to each other, and nothing else this program has ever puts an icon up there — the same
argument that keeps one off the window icon and the favicons. What it buys is the letters at a
legible size instead of two thirds of one.

Two numbers decide how it is drawn, and they are deliberately separate: `BAR_ICON_HEIGHT` is the
resolution, 128 pixels, because `km-tray` resizes down to the bar's own size and a filter for making
things smaller wants to be given something larger; `BAR_ICON_INK` is how much of that is letter, 74%,
because a menu bar item is 22 points tall and nothing in one is 22 points of ink. The file is wider
than it is tall, and `km-tray` fixes its height and lets the width follow — forcing both would scale
the two axes by different amounts and draw the letters stretched.

Windows takes no such file: the notification area draws the colored mark out of the executable's own
resources and has no template convention to follow.

There are **four** icons and they are the same artwork under four palettes: the machine leads with
the theme's sung-lyric amber, `km-package-builder` with the theme's blue accent, `km-remote` with
the theme's second accent green, and `km-admin` with the theme's magenta. That hue reaches exactly
two places — the widest band, and the M — and everything else is the same code producing the same
pixels, so the four cannot drift into four designs. `render` takes the color and nothing else in
the example knows which program it is drawing for.

**The fourth one is where "and not one is new" started to bite**, and what it cost is worth writing
down. By the time a fourth program wanted a lead there was no unclaimed hue left in `Theme`: `alert`
means *warning*, `icon_ground` is a ground and too dark to lead, and the `lyric_*` pair are the words
on a screen. What was left was `icon_glow` — the magenta that is already the bright end of the two
*supporting* bands in every mark.

Two consequences, both taken deliberately rather than designed away:

- **This icon's lead band and its middle band are the same hue.** They differ in value — the lead
  band starts darker than the middle band ends — so the tile still reads as three steps, one of
  which is a fold rather than a hue change. Tinting the supporting bands to avoid it was the
  alternative, and it is the thing not to do: `the_hue_reaches_two_places_and_nowhere_else` holds
  that the deep purple, the magenta, the plate and the K are byte-for-byte identical in every mark,
  which is what stops four programs becoming four designs.
- **The magenta is lifted 12% toward white to lead.** Straight, it measured **4.00:1** for the M
  against the plate, under the 4.5:1 floor the letters have to clear — which is not bad luck:
  `Theme::icon_glow`'s own note records that it lost its contrast ceiling *when the mark stopped
  standing on the ground*, and making it a mark color again re-imposes exactly what it was released
  from. Lifting is the established answer here rather than a new one — "the blue was lifted once to
  keep a contrast floor and the green followed it to stay a set" — and it is done where the palette
  is read, not in `Theme`, because lifting it there would change the bands of all four marks to fix
  one letter.

**The ground is tinted per mark rather than shared**, which the `Application icon` decision argues.
A shared ground is right while the mark is a colored glyph carrying the difference on its own, and
not once the mark is two letters, one of which is the same near-white in all of them.

The reason there is more than one icon at all is that these are the programs in this product that get
run *next to each other*, on a desktop, where an identical icon on two windows and two taskbar
buttons is an icon doing no work. "It is the same product" would give the offline remote
`karaokemachine.ico` byte for byte — and so is the package builder; what settles it is not products
but taskbars, and the offline remote is the one program here somebody runs *while the machine is
playing*.

**16 pixels is where this design is weakest**, and the trade is taken deliberately. Two letters in
a twelve-pixel plate are a smear; what identifies an icon at that size is the palette rather than the
letters, which is why the palette carries the identity and not the mark. Everything from 32 up reads
`KM` cleanly. The plate *grows* below 32 rather than going away, because the plate is where all of
the letters' contrast comes from.

The four get different numbers of loose sizes, and the rule is that every file here has a reader.
The machine gets **eight**: seven because its Debian package fills `hicolor` up to 512, and a 1024
because that same package carries a Plymouth theme, where the mark is drawn on a television during
boot rather than in a taskbar. It is the only size here whose reader is neither a desktop, an
executable nor a web page — and `hicolor` still stops at 512, because the package's asset list names
every destination one by one and a size added to the generator does not silently acquire a
directory. The builder gets six:
no Debian package, so no 512, but `--register` writes 16 through 256 into the user's `hicolor`. The
remote and `km-admin` get **two** each — the 32 each one's own page serves, and the 256 the icon
tests sample and the menu bar is handed — because neither registers anything into `hicolor`.
**All four have a macOS bundle and therefore an `.icns`**, which moves no loose-size count, because
an `.icns` carries its own sizes and is written from `ICNS_MEMBERS` rather than from those lists.

**The remote has an Android launcher entry too.** It is an application on a phone, so it gets the
same three files per density the machine does, in the green — and the loose-size count still does not
move, for the same reason the `.icns` does not move it: those two files are read by a *page* and by a
*test*, and a launcher reads
neither. It gets no `drawable-xhdpi/banner.png`, because a banner is the tile a television's home row
draws and a remote is the thing you hold instead.

**And an iOS one, which is one file where Android is fifteen.** That is the platform being simpler
rather than the icon being unfinished: iOS applies its own superellipse mask and resamples a single
1024-pixel image everywhere it appears, so there is no foreground/background split and no density
ladder. Two things about drawing it are different from everything else here, and both are in
`icon.rs`. It uses a **full-bleed** ground, because a rounded square under Apple's mask is rounded
twice — the identical argument `Ground::Bleed` already makes about Android's launcher. And it is
written by `opaque_png` rather than `png`, because **iOS rejects an app icon with an alpha channel**:
the mask is the system's, and a transparent pixel is a hole in it. The loose-size count did not move
for this either.

## What is here, and who reads it

| File | Read by |
|---|---|
| `icon-16.png` … `icon-512.png` | `hicolor` in the Debian package; `icon-32.png` is the favicon compiled into `km-api` **and the one `km-remote-pages` serves for the remote the machine itself hosts**; `icon-64.png` is the mark beside the title in the repository's own `README.md`, the one reader here that is neither compiled in nor installed — GitHub renders it from the relative path, so nothing stages a second copy; `icon-256.png` is the window icon compiled into `km-display` |
| `icon-1024.png` | The Debian package again, as `usr/share/plymouth/themes/karaokemachine/logo.png` — the boot splash on the appliance, drawn on a television between the bootloader and the machine's first frame. Under the theme rather than under `hicolor`, which stops at 512; and the theme scales it to a fraction of the panel, so 1024 is the ceiling rather than the size anybody sees |
| `karaokemachine.ico` | `crates/machine/karaokemachine/build.rs`, which puts it in the Windows executable |
| `karaokemachine.icns` | `tools/platform/macos/app-bundle.sh` |
| `km-package-builder-16.png` … `-256.png` | `tools/cmd/km-package-builder/src/register.rs`, which compiles them in and writes them into the user's `hicolor` on `--register`; `-32.png` is also that tool's favicon, compiled into `src/server.rs`; and `-256.png` is what that tool's `src/desktop.rs` hands `km-tray` for the **macOS menu bar**, decoded and resized to 44 — 256 rather than 32 because the bar wants 44 physical pixels on a Retina display, and downscaling beats upscaling |
| `km-package-builder.ico` | `tools/cmd/km-package-builder/build.rs`, which puts it in the Windows executable — and `km_webshell::with_icons` then reads it back out of the running process for the title bar and the taskbar, as `km-tray` does for the notification area. Windows therefore decodes no PNG for either |
| `km-package-builder.icns` | `tools/dist/cmd.sh`, which puts it in `KM Package Builder.app/Contents/Resources`. Its name without the extension is what `tools/platform/macos/Info.package-builder.plist` holds in `CFBundleIconFile` — and in the document type's `CFBundleTypeIconFile` and `UTTypeIconFile`, so a `.kmbuild` in the Finder wears it too |
| `km-remote-32.png` | `crates/remote/km-remote-pages`, as `ICON_REMOTE_PNG` — the favicon the *offline* remote serves at `/static/icon.png`. That crate holds both marks and `km-remote-core` picks this one, because the same pages are served by the machine, where the amber is right |
| `km-remote-256.png` | `crates/playback/km-display/src/icon.rs`'s test, which samples it to check each program's hue is still its own — and `crates/remote/km-remote/src/desktop.rs`, which hands it to `km-tray` for the **macOS menu bar**, on the same terms as the package builder's |
| `km-remote.ico` | `crates/remote/km-remote/build.rs`, which puts it in the Windows executable — and `km_webshell::with_icons` then reads it back out of the running process for the title bar and the taskbar, as `km-tray` does for the notification area |
| `km-remote.icns` | `tools/dist/cmd.sh`, which puts it in `KM Remote.app/Contents/Resources`. Its name without the extension is what `tools/platform/macos/Info.remote.plist` holds in `CFBundleIconFile` — once and not three times, unlike the builder's, because the remote declares no document type |
| `km-admin-32.png` | `tools/cmd/assets/km-admin/src/server.rs`, as `ICON_PNG` — the favicon that tool's own page serves |
| `km-admin-256.png` | `crates/playback/km-display/src/icon.rs`'s tests, which sample it with the other three and check its lead is the magenta and nobody else's hue — and `tools/cmd/assets/km-admin/src/desktop.rs`, which hands it to `km-tray` for the **macOS menu bar**, on the same terms as the package builder's |
| `km-admin.ico` | `tools/cmd/assets/km-admin/build.rs`, which puts it in the Windows executable — and `km_webshell::with_icons` then reads it back out of the running process, at 16 for the title bar and at the large metric for the taskbar, as `km-tray` does for the notification area. It was the last of the three to ask for one, and looked right in the taskbar the whole time it did not — then stopped looking right the moment it got one, which is how the missing second slot was found |
| `km-admin.icns` | `tools/dist/cmd.sh`, which puts it in `KM Admin.app/Contents/Resources`. Its name without the extension is what `tools/platform/macos/Info.admin.plist` holds in `CFBundleIconFile` — once, like the remote's, because it declares no document type either |
| `karaokemachine-stream-16.png` … `-256.png` | `hicolor` again, under the name the desktop entry's stream action gives: the Debian package installs them and `crates/machine/karaokemachine/src/register.rs` compiles them in for `--register`. `-32.png` is also what `src/tray.rs` hands `km-tray` for the **macOS menu bar**, and `-256.png` is what `km_display::icon`'s test samples |
| `karaokemachine-stream.ico` | `crates/machine/karaokemachine/build.rs`, which puts it in the Windows executable **beside** `karaokemachine.ico` at the next ordinal — so the notification area of a `--stream` run and the Start Menu entry that passes `--stream` both find it without a second file being installed anywhere |
| `karaokemachine-stream.icns` | `tools/platform/macos/app-bundle.sh`, which puts it in `KaraokeMachine Stream.app/Contents/Resources`. Its name without the extension is what `tools/platform/macos/Info.stream.plist` holds in `CFBundleIconFile` |

Committed rather than built, because these are assets: Gradle needs Android's copies present in
`res/`, `build.rs` needs the `.ico` before the executable can carry it, and nobody should need a Rust
toolchain to build an APK.

## Why they are not under `assets/`

`assets/` is *shipped* — `tools/platform/windows/dist.sh`, `tools/port/machine/android/assets.sh` and the Debian package
each copy all of it beside the binary. Nothing here is needed at run time: the two icons a running
process wants are compiled into it, and the rest are read by installers and build scripts. Putting
them in `assets/` would ship three quarters of a megabyte of dead weight to every user.
