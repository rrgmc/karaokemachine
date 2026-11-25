# Research: the machine on an LG webOS television

**Research only. Nothing implemented, nothing decided.** A platform is a product decision, so a
webOS carrier needs an entry in `docs/decisions/distribution.md` beside
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios) first.

Investigated 2026-09-13 and revised 2026-11-24. **§13 is what a 2020 set on webOS 5 measured**, with
the probe in `tools/port/machine/webos/`, and the machine itself runs there as
[`docs/architecture/webos.md`](../architecture/webos.md) describes. Nothing else here was verified
on a television, and the marker on each claim says how far it can be trusted. **§14 is what an
established native webOS application, moonlight-tv, does that this port does not.**

**Summary.** The whole machine runs on an LG television as a native 32-bit ARM Linux program. The
engine is ready: it already plays all three song kinds on a 32-bit Cortex-A55 television box at
59.5 fps.

**The 2026-11-24 revision found the route the first version said did not exist.** That version knew
one way in, a homebrew executable sideloaded through Developer Mode, and called it a homebrew carrier
rather than a shippable one. Since 2026-06 LG has published **Flutter for webOS TV**, an SDK LG
maintains itself. Its applications are `"type": "native"`, compiled ahead of time to ARM. Its plugins
are C++ shared libraries built with CMake, and one of LG's own links ALSA. That is the shape the
[Samsung port](tizen.md) is built in: a thin host on the vendor's supported framework, loading the
machine as a library. Flutter takes the place .NET took there. §1 has the detail. It needs webOS
TV 26 "Re:New" or later, which television years that covers is not yet known, and installing still
goes through Developer Mode.

**On a webOS 5 set both questions that could end it are answered, and the machine runs** (§13). SDL3
draws at 60 fps in a custom-role window given the webOS shell roles, after a two-symbol patch for the
set's older libwayland. cpal plays through ALSA's `default` with no underruns. The machine plays a
MIDI song with its lyrics in about 56 MB and 0.6 of a core, and the singer's page works from a phone.
Two findings still shape the product rather than block it. An installed application is removed when
the Developer Mode session runs out, unless the Content Store carries it. And the set's ffmpeg
cannot decode H.264, so the `.ipk` carries its own.

**The release offers it as an unofficial download**, the decision
[`An LG television carrier is an unofficial download`](../decisions/distribution.md#an-lg-television-carrier-is-an-unofficial-download).
The route to an install that does not expire is LG's Flutter one, on a webOS 26 set, which has not
been tried.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository. High confidence. |
| **[source]** | Read from webOS or SDL source, or from a file listing. High confidence. |
| **[web]** | Public documentation and community reports. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |
| **[set]** | Measured on a television, §13. |

## 1. What a webOS television permits

**[web]** Native applications install and run on a stock, unrooted television through Developer Mode.
Kodi ships that way for webOS 5 and later. It is a larger native program than this one, so it
settles whether the path works at all. Until 2026 that was the only native route, and an unofficial
one. The subsection at the end of this section has the official route LG has added since.

**[web]** The toolchain is `webosbrew/meta-lg-webos-ndk`, a Yocto SDK producing **armv7a glibc**
binaries. **[set]** On the webOS 5 set the userland is softfp, not hard-float: the loader is
`ld-linux.so.3`, and the Rust target is `armv7-unknown-linux-gnueabi`, as on Tizen (§13). Installation is an `.ipk` pushed with `ares-install`, and the
launcher starts the program as an ordinary ELF executable with a JSON string in `argv[1]`.

**[repo]** That last point removes the layer both mobile carriers need. iOS has
`crates/machine/km-machine-ios` because the application binary is Swift. Rust has to be a
`staticlib` underneath it. Android has `MainActivity` because SDL's Java calls into the library.

A webOS program starts the way the Debian package starts, so `src/main.rs` and `km_app::run` are the
entry point. No shim crate exists, and `run_on_phone`, `ioscfg` and the `cdylib` crate type all stay
untouched. **This is the largest single saving the platform offers.**

**[web]** Applications draw through LSM, the webOS Wayland compositor, using `wl_webos_shell` rather
than `xdg_wm_base`. Audio is ALSA with PulseAudio above it. Hardware video decode is reachable only
through the television's media pipeline, which draws on a plane behind the GL surface.
**[source]** That pipeline is not only a file-or-URL player: NDL DirectMedia takes compressed frames
from the application, §5.

### Since 2026-06: an official native application type, through Flutter

**[web]** LG announced Flutter for webOS TV as "now officially available for developers" in its
developer news of 2026-06-30. It supports "webOS TV 26 Re:New, and later", and a television that
shipped with webOS 26 needs the Re:New update first. LG says nothing about which model years receive
it. Development is on Ubuntu only, with WSL2 or a container elsewhere, and a device needs Developer
Mode to deploy to.

**[source]** The SDK is `lg-flutter-webos/flutter-webos`, BSD-3-Clause, with its plugins in
`lg-flutter-webos/plugins`. What its files show:

| | |
|---|---|
| **Application type** | the template's `appinfo.json` says `"type": "native"`, with `main` naming the executable. The type the first version of this note called unofficial is the one LG's own SDK emits |
| **Code** | release builds are AOT. The runner is a C++ `main.cc`, an `App` deriving from LG's `FlutterWindow`, which registers the plugins |
| **Native plugins** | a plugin's `webos/` folder is a CMake project building a `SHARED` library, registered through an ordinary `pluginClass` in `pubspec.yaml`. Its CMake has a list of further libraries to bundle with it |
| **Toolchain** | LG's webOS NDK, `environment-setup-ca9v1-webosmllib32-linux-gnueabi`, and output under `build/webos/arm/`. The plugins build with `-mfpu=neon -mfloat-abi=softfp` |
| **Sound** | `audioplayers_soloud` builds SoLoud's ALSA backend and requires `alsa` from pkg-config. Its sibling `audioplayers_webos` goes through the platform's media pipeline |
| **Nothing found** | `dart:ffi`, an FFI plugin template, how the embedder gets its surface (Wayland or EGL), and any plugin for the remote's keys |

**[inferred]** The port this implies is Tizen's, part for part. A Flutter application draws a label
and nothing more. A plugin of perhaps a hundred lines of C++ opens `libkm_app.so` with `dlopen` and
calls one exported entry point with the window, the data folder and the assets folder. The machine is
the `cdylib`, as it is on Android and Tizen, not the executable the paragraph above described. That
gives up the saving it called the largest the platform offers. It buys a route LG supports.

**[inferred]** The two toolchains disagree about floating point. The homebrew NDK says hard-float and
`gnueabihf`. LG's says `gnueabi`, and its plugins build `softfp`, which is the Samsung set's ABI and
the triple the Tizen build already uses. A library must match the process that loads it, so LG's
answer is the one that binds a plugin. The probe in §2 reads the loader's name and settles it.

**[web]** The other routes, for completeness:

- **A JavaScript service** cannot carry native code. LG's documentation says flatly that modules with
  C/C++ add-ons cannot be used, and services run inside a jail. Whether one may start a bundled
  executable is not documented either way.
- **A web application in WebAssembly** is possible in principle. The engine is Chromium 120 on
  webOS 25 and Chromium 132 on webOS 26, but LG documents nothing about WebAssembly, AudioWorklet or
  WebGL2 support. It would also replace `km-display` wholesale, which is the divergence §3 refuses
  for SDL2.
- **Rooting** is not a product route. The faultmanager, DejaVuln and crashd chains are patched in
  current firmware. Their maintainers report the 2025 televisions rootable only on the webOS 10.0 they
  shipped with.

## 2. What one afternoon answers

Five of the fatal questions cost one `.ipk` between them, and none of them involves this
repository. Two programs, deployed together.

**A forty-line Rust binary**, cross-compiled for `armv7-unknown-linux-gnueabihf` against the NDK
sysroot, that writes a file and exits:

- Whether the jail permits `exec` from the application's own directory at all. A `noexec` mount
  there ends the port.
- Whether rustup's prebuilt std links and loads against the television's glibc, and whether the
  userland is genuinely hard-float. The wrong triple produces a binary that loads and then computes
  wrongly.
- **Where the process can write.** Probe `$XDG_DATA_HOME`, `$HOME`, the application's own
  directory, `/tmp` and `/var/lib/webosbrew`, and print what each answered. Without one that
  survives a reboot there is nowhere for `settings.json` or the catalog.
- Whether `readdir("/tmp/usb")` sees a stick from inside the jail, before and after a restart.
- Whether it can bind a TCP listener and be reached from another machine on the network.

**A sixty-line `wl_registry` dump**, printing every Wayland global with its interface **and its
version**. This is the question the whole port turns on. Asking it directly separates four failure
modes that "try SDL3 and see" would collapse into one. The versions matter as much as the names. The
SDL2 fork carries an ABI fix because the television ships an older libwayland. A version mismatch
fails inside libwayland rather than anywhere readable.

**[inferred]** Add `ls -l /dev/dri` to the same run. If a DRM card is visible and openable from the
jail, that is the best outcome available. SDL3's `kmsdrm` backend is what the Debian appliance
already ships, documented in [`appliance.md`](../architecture/appliance.md). The graphics problem
below then disappears entirely. LSM almost certainly holds DRM master, which is the appliance's own
documented failure against a display manager, and the question costs ten seconds.

**[inferred]** **On the Flutter route of §1 the same questions are asked from inside LG's template**,
and that deploy comes first. It is LG's hello-world plus a plugin of a few dozen lines of C++. The
plugin opens a test library with `dlopen`, and the library writes a file, probes the same folders,
binds a listener and dumps the registry. Two questions are added. **Whether the plugin can reach the
host's Wayland surface**, which is what SDL3 has to draw into. And the ELF loader's name, which settles
hard-float against `softfp`. A library loaded from the application's own folder answers the `exec`
question in the form this route asks it.

## 3. The blocker: there is no SDL3 for webOS

**[repo]** The display is SDL3 throughout. `docs/architecture/display.md` states it flatly: "There is
**no `TextRenderer` trait** and nothing here is behind a seam." `crates/playback/km-display` draws
against `sdl3::render::Canvas` and `sdl3::ttf::Font` across every source file, and the 4,060-line
frame loop in `crates/machine/karaokemachine/src/display.rs` names `sdl3::video`, `sdl3::ttf` and raw
calls into `sdl3::sys`. SDL is vendored and built from source for every target through
`build-from-source-static`.

**[source]** The only webOS SDL is `webosbrew/SDL-webOS`, a fork of SDL2 2.30. It carries no separate
video driver. It patches the Wayland one with six files: `SDL_waylandwebos.c`,
`SDL_waylandwebos_abifix.c` (the older libwayland ABI), `SDL_waylandwebos_cursor.c`,
`SDL_waylandwebos_foreign.c` (video-plane punch-through) and `SDL_waylandwebos_osk.c` (the on-screen
keyboard). Surface setup binds `wl_webos_shell_surface`, `wl_webos_foreign`,
`wl_webos_input_manager`, `wl_starfish_pointer` and `text_model_factory`. It takes the application
id from the `APPID` environment variable. It treats `WL_WEBOS_SHELL_SURFACE_STATE_FULLSCREEN` as a
state to react to rather than a mode to request.

**[web]** SDL3 rebuilt Wayland shell handling around xdg-shell and libdecor, and dropped `wl_shell`.
So the patch does not transplant line for line: surface creation and configure handling need a third
shell path.

**[inferred]** LSM probably advertises no `wl_shell` substitute SDL3 accepts. Kodi has its own
Wayland windowing backend requiring xdg-shell, and its webOS port uses SDL2 instead. The registry
dump in §2 settles it. **[set]** On webOS 5 it did: the registry has `wl_shell` and
`wl_webos_shell` and no `xdg_wm_base`, so rung 1 is out (§13).

**The ladder, stopping at the first rung that works:**

0. **kmsdrm**, if the jail can open a DRM card. No new SDL code at all.
1. **Stock SDL3 Wayland**, if `xdg_wm_base` is advertised. A hint or two, and `wl_webos_shell` for
   the application id and fullscreen on top.
2. **Patch SDL3's Wayland driver** with the webOS extensions. SDL3's driver descends from the one
   the fork patched, so this is carrying a patch forward rather than writing one.
3. **A new SDL3 `webos` video driver** over EGL and the webOS shell. Weeks, and an SDL video driver
   maintained here for as long as the port lives.

**[inferred]** With a Flutter host, the rung to try first after kmsdrm is Samsung's. SDL3's stock
Wayland driver draws into a `wl_subsurface` of the host's own window. On Tizen 10 a window of SDL's
own stuck on an old frame, while a subsurface ran at 60 fps
([`tizen.md`](tizen.md) §3). It is rung 1 with the toplevel lent by the host, so it needs neither
`xdg_wm_base` nor `wl_webos_shell`. It does need the plugin to hand the machine a `wl_surface`, or
something it can be reached from, and that is the unknown §1 lists.

**SDL2 is not a fourth rung.** `km-display` takes `sdl3-ttf-sys` directly, and every one of its
source files draws through SDL3 types. The workspace depends on the `unsafe_textures` feature. That
feature drops the `Texture` lifetime, which is load-bearing for the text cache. SDL2 means a second
`km-display` and a permanent divergence in the file that decides what the machine looks like. **If
the ladder runs out at rung 3 the answer is "not on this hardware".**

**[repo]** A patched SDL has nowhere to go in the build as it stands. `sdl3-sys` with
`build-from-source-static` fetches and builds SDL itself and offers no patch hook. So rungs 2 and 3
also mean building SDL out of band and switching that target to a prebuilt SDL. That is a **third**
`sdl3` dependency block.

Both `crates/playback/km-display/Cargo.toml` and
`crates/machine/karaokemachine/Cargo.toml` warn that cargo features are additive. The split has to be
declared in both. Getting it wrong in one silently undid the other on Android, and the library came
out with no `libSDL3.so` dependency at all.

## 4. Audio, which is as likely to end the port as graphics

**[repo]** `crates/playback/km-audio` is the only crate naming cpal, and cpal's Linux backend is
ALSA. `km_audio::device::decide` takes `linux: bool` as a parameter rather than reading a `cfg`, so
nothing about device selection is bound to a target.

**[web]** A television's sound is ALSA with PulseAudio above it, and PulseAudio owns the hardware
devices.

**[source]** LG's own `audioplayers_soloud` plugin builds SoLoud's ALSA backend and nothing else
besides a null one. A plugin in LG's SDK, for applications LG supports, opens ALSA. That makes the
first outcome below the likeliest. It is still a build file rather than a measurement, and on Samsung
ALSA was refused where the platform's own service worked.

**[inferred]** Three outcomes, and the spike is a thirty-line sine tone:

- cpal opens the default device because `libasound_module_pcm_pulse.so` is present in the image and
  `default` routes through it. Nothing changes.
- Only PulseAudio works. `km-audio` needs a second output path, and the cheapest is SDL3's own
  audio: it is linked already and has a PulseAudio backend. That is a new output in the one crate
  that has exactly one, and a decision of its own.
- The platform pipeline owns the PCM, and an ordinary application cannot open it. The media
  pipeline is not a sink a synthesizer can push samples into, so that is the end of the port.

**A separate question with no answer here:** microphone audio is mixed in hardware. So the
machine is usable in a room only if the television's own output reaches the amplifier over ARC or
optical.

## 5. Video

**[repo]** `crates/playback/km-video` is one file with no `cfg` and no platform assumption in it, over
ffmpeg 7.1.5 pinned in `tools/setup/ffmpeg-pin.sh`. A webOS build is a fifth caller of that pin. It
follows `tools/port/machine/android/ffmpeg.sh` in building **shared** LGPL libraries rather than
static ones, for the licence reason that script gives at length.

**[repo]** Software decode is the only option and it may be enough. `docs/architecture/android.md`
measured a 1920x1080 H.264 song at 926 kbps on four Cortex-A55s. Single-threaded, it starved on three
runs of three. With `threads=5 kind=Frame` it played all 273 seconds at 59.5 fps and reported nothing
at all.

**[repo]** One trap transfers with the build rather than with the platform. Android narrows its
decoder set because libavcodec's link line overruns Windows' 32,767-character command line. This
repository is developed on Windows, so a webOS ffmpeg cross-compiled here meets the same wall.

**[inferred]** A television's SoC is in that class and is already running the television, so
headroom is the question rather than raw speed. If it fails, video songs are declined and the
`.ipk` is built without them. The machine still catalogs, searches and queues them, and says at
startup that it cannot play them. That is a supported shape of every carrier today. **[set]** It did
not fail: §13 measured 1080p30 at about 2.1 cores with an ffmpeg of the application's own.

**[source]** Hardware decode is open to an application after all. moonlight-tv streams games on these
sets through `mariotaku/ss4s`, whose webOS modules push **compressed frames** to the television:
`NDL_DirectVideoPlay(data, size, pts)` takes H.264, H.265, VP9 and AV1, and NDL's audio side takes
raw S16LE PCM, or Opus. There is one module for webOS 4, one for webOS 5 and later, and LGNC and
StarfishMediaPipeline ones for other generations. So libavformat could demux a video song here and
the television decode it, at a fraction of a core and up to the panel's own 4K.

**[inferred]** Two costs keep it a candidate rather than a plan:

- **The picture is on a plane behind the GL surface.** The lyrics, the queue and the idle screen have
  to be drawn over it with the surface transparent where the video shows, which is the punch-through
  the SDL2 fork carries in `SDL_waylandwebos_foreign.c`. `km-display` draws an opaque frame today.
- **Sound and picture need one clock.** A video song's lyrics are burned into the picture. Handing NDL
  both the video and the decoded audio lets the television keep them together, at the price of a
  second sound output beside cpal; keeping the sound on ALSA leaves two clocks to reconcile.

It is worth taking up only if video songs on an LG set are wanted beyond what software decode gives.

## 6. Memory is the second constraint

**[repo]** `docs/architecture/audio.md` measured the bank picker on a television box with 3.87 GB of
RAM. Resident memory is the bank's file size plus about 125 MB. So the bundled GeneralUser GS costs
158.7 MB, and the largest bank the machine offers costs 427.9 MB. A swap between the two largest peaks
at **674.2 MB**, because `Bank::load` runs while the old bank is still live in the audio callback.
The same session measured 552 MB total RSS while playing 1080p with a 274 MB override bank.

**[web]** A television has between 1.1 GB and 1.3 GB of RAM for webOS 5 and 6. It shares that with
the operating system and every background service. And webOS restarts an application when memory runs
short.

**[inferred]** The bundled bank fits and a large one does not. On this carrier the offered bank list
is a constraint rather than a preference. The two-banks-at-once transient during a swap is the number
that decides which entries can be offered at all. It is measurable the day a television runs
the machine, and it decides a product question rather than a build one.

## 7. Where the songs live

**[web]** An application installs under `/media/developer/apps/usr/palm/applications/<appid>`. `/tmp`
is writable and cleared on boot. `/var/lib/webosbrew/` is writable and survives a reboot. USB drives
mount at `/tmp/usb/<disk>/<partition>`, walked as `/tmp/usb/*/*` because the letters move with what
is plugged in and in what order. The jailer decides what a Developer Mode application sees, and USB
has been reported to need a television restart before it appears inside one.

**[repo]** The machine's own data has an existing home for this. `Paths::data_rooted_at` roots config
and data at one directory while leaving assets to ordinary discovery, which is what `--data-dir`
uses. Assets need nothing at all. `asset_dirs_from` already takes the directory beside the executable
when it holds an `assets` folder. That is the Debian layout, and it is also the `.ipk` layout.

**[inferred]** Songs on a stick are the one genuinely new product mechanism. The shape that keeps
[the removable-media non-goal](../decisions/foundations.md) intact is a walk rather than a mount
watcher:

- `settings.package_dirs` cannot serve here on its own. The mount path is not stable across sticks or
  reboots, and there is nowhere for an owner to type it.
- A walk of `/tmp/usb/*/*` contributes each partition's `packages` folder. It is appended **last**
  inside `Paths::packages_dirs`, after the machine's own folder and the owner's configured ones. So
  nothing on a stick displaces what the machine already installed.
- The walk runs inside `packages_dirs` rather than being frozen at startup. So the next look picks up
  a stick that arrives later. Every look is something somebody asked for: the rescan action, the
  admin endpoint, or a restart. No timer and no watcher, so nothing reasons about
  volumes.
- A stick that is pulled is a folder that is not there, which the reconcile already handles.

## 8. The carrier uninstalls itself

**[web]** Developer Mode sessions run 1000 hours. Pressing EXTEND while the television is online
extends a session. When a session expires **the applications installed under it are removed and
Developer Mode is disabled**, and ten reboots with no network disable it too.

**[web]** Rooting lifts this. But firmware released since the middle of 2022 patches the RootMyTV
exploit chain, so it cannot root a television bought today. The later chains are patched too (§1).

**[web]** Keeping the session alive from outside has been tried. Community scripts call LG's session
reset with the developer token from a cron job, or have Home Assistant open the Developer Mode
application and press EXTEND. The reports are mixed, and some say the scripted reset no longer
works. LG's own forum answer is that 1000 hours rarely needs extending.

**[inferred]** The Flutter route of §1 changes where this can end rather than how it starts. LG's
guide deploys through Developer Mode as well. But an application built with LG's SDK, of the type LG's
template emits, is something the LG Content Store can at least be asked to carry. Individual sellers
can register, and LG puts every submission through QA. Nothing found says whether an individual's
native or Flutter application is accepted, or what QA makes of a karaoke machine that serves an HTTP
API on the local network. A store install has none of the expiry above.

**[inferred]** This is the finding that most affects whether the port is worth having. A karaoke
machine is an appliance somebody switches on in a room. This one asks its owner to keep a developer
session alive or lose it. The machine's data therefore belongs where an uninstall does not reach. Then
pushing the `.ipk` again restores a working machine rather than an empty one.

## 9. How it would fit

**[repo]** The Android port is the measure. Its note says: "Six of the seven needed **no changes at
all**, which is the pure-Rust bet paying off exactly as planned." The same holds here, and the
television work is already done. `km_display::input::action_for` maps the arrows to focus movement,
the digit rows to number entry, and `Return` to submit. It maps both `Escape` and `AcBack` to back,
all of it written for a television remote.
[`Nothing is drawn where a television will not show it`](../decisions/interface.md#nothing-is-drawn-where-a-television-will-not-show-it)
settles the 5% overscan inset.
`km-api`'s held listener and its relisten already answer an application that is sent away and comes
back, which is what pressing HOME does.

What a port adds:

| | |
|---|---|
| `ports/machine/webos/` | `appinfo.json`, icons, and the per-port `README.md`. No project file, because webOS has none |
| `tools/port/machine/webos/` | `build.sh`, `stage.sh`, `assets.sh`, `ffmpeg.sh`, and `sdl.sh` if a patched SDL is needed |
| `tools/port/webos-ndk.sh` | the twin of `tools/port/ndk.sh`: the sysroot, the cross compiler, the CMake toolchain file, the minimum webOS version named once |
| `.cargo/config.toml` | a `[target.armv7-unknown-linux-gnueabihf]` entry naming the SDK's linker |
| a `webos` cargo feature | on `karaokemachine` alone, beside `video`, plus its line in `tools/setup/features.sh` |
| `src/settings/paths.rs` | the data directory ladder, and the USB walk in `packages_dirs` |
| `src/display.rs` | webOS values for `FULLSCREEN_IS_FIXED`, `KEY_HINTS`, `DRAG_AND_DROP`, `FILE_MANAGER`, `WEB_BROWSER` and `WINDOW_STACKS`, all answering the television way |

**[inferred]** The feature flag is what a `cfg` cannot do here. webOS **is** `target_os = "linux"`,
and so is the Debian appliance, which wants different answers about paths, fullscreen and those six
constants. `video` is the precedent for a feature naming a build rather than a target. A bare `--cfg`
through `RUSTFLAGS` is undeclared where clippy runs with `-D warnings`.

**[repo]** SDL links static here, as everywhere except Android, because no Java side calls into it.
Assets are ordinary files, so iOS's treatment applies and nothing like `androidassets.rs` is needed.

**[inferred]** On the Flutter route of §1 the table changes in three rows. `ports/machine/webos/`
becomes a Flutter project: the Dart host, `webos/meta/appinfo.json`, and the C++ plugin that loads
the machine. `tools/port/machine/webos/` drives LG's `flutter-webos` and NDK, inside a container
because LG supports Ubuntu hosts only. And the machine becomes a `cdylib` exporting one entry point
behind the `webos` feature, which is what `tizen` already is. The Samsung host is
`ports/machine/tizen/Main.cs`, and it is the model for the Dart one. It draws its first frame before
starting the machine, and it shows the reason when the machine cannot load.

## 10. Effort and risks

The estimate is keyed on which rung of §3 the graphics land on, and nothing below rung 1 is knowable
until the registry dump exists.

| Graphics lands on | Then |
|---|---|
| rung 0, kmsdrm | about a week of build plumbing and days of Rust |
| rung 1, stock SDL3 Wayland | about a week and a half, plus hardware iteration |
| rung 2, a carried SDL3 patch | three to four weeks, and a vendored patch for as long as the port lives |
| rung 3, a new SDL3 driver | six to nine weeks, and an SDL video driver maintained here |
| nothing works | days, which is the point of asking first |

| Risk | Where it lands |
|---|---|
| No SDL3 video backend can be made to work | the port stops |
| cpal opens no device and only the media pipeline exists | the port stops |
| The jail refuses `exec` or offers no persistent writable directory | the port stops |
| LG's Flutter embedder lends a plugin no Wayland surface | the Flutter route packages the machine but cannot show it, and the ladder of §3 starts over from a window of its own |
| The Content Store declines it | the Flutter route is a sideload too, and §8 applies unchanged |
| A patched SDL is needed | a third `sdl3` dependency block, and the additive-features trap that has already been paid for once |
| The bank does not fit in a television's RAM | the offered bank list shrinks to the small entries |
| The jailer hides USB from the application | songs have no source; a restart is the reported workaround |
| The Developer Mode session expires | the machine is removed and pushed again, and its data has to outlive it |
| Remote keys arrive through a Luna service rather than the compositor | a new input source, where today there is none |

## 11. Out of scope, whatever the answer

- **Any carrier with an indefinite life**, short of the Content Store. Developer Mode expires and the
  Homebrew Channel needs a rooted television.
- **The on-screen keyboard.** The machine's own number pad is how a song number is entered.
- **Writing to the stick.** It is a source of songs and not a place to put them.
- **Noticing a stick without being asked.** The rescan is somebody's act.

## 12. Recommendation

**Not now, but no longer "not shippable".** The Android port on a Google TV device already serves a
television. The display question is answered for webOS 5: the machine draws and plays there
(§13). What has changed is the carrier. The first version of this note had only a homebrew one, which asks its owner to keep a
developer session alive or lose the machine. LG's Flutter SDK is an official native application type,
and a host plus a loaded library has already been built and measured on Samsung. On LG it is the route to
try first, and the Content Store is the only way found to an install that does not expire.

If it is ever taken up as a product, the first act is the afternoon in §2 on a television with
webOS 26 Re:New, deployed as LG's Flutter template with the probe as a plugin. What the webOS 5
set settled carries over as a guess, not a measurement: whether the newer compositor still lacks
`xdg_wm_base`, and how old its libwayland is, decide whether the window code here is needed there.
Memory is the other measurement, because it decides which SoundFont banks a carrier can offer; on
webOS 5 the machine left about 236 MB free with the bundled bank, and a 275 MB bank played only by
leaning on swap.

The sibling note is [`tizen.md`](tizen.md), which asks the same question of a Samsung television. It
finds a port possible there too. SDL3 draws through its own Wayland driver into the host
application's window, since a window of its own does not stay on screen, and sound goes through the
platform's own service rather than SDL's.

## 13. Measured: a 2020 set on webOS 5

**[set]** Measured 2026-11-24 on a 2020 LG set, `sdkVersion` 5.6.2, with the homebrew route of the
plan: a std-only Rust executable in an `.ipk` of type `native`, installed with `ares-install` through
Developer Mode. LG's Flutter route needs webOS 26 Re:New, so it was not tried. Built in
`tools/port/machine/webos/`.

| Question | Answer |
|---|---|
| Does a bundled executable run from the application folder? | **Yes.** The folder is not `noexec`. The launcher starts it with the parameters as JSON in `argv[1]` |
| Loader, float ABI, glibc | `/lib/ld-linux.so.3`, softfp, glibc 2.28. `armv7-unknown-linux-gnueabi` with `-soft-float,+neon,+vfp4`, built on Debian buster's armel compiler. Rust's prebuilt std loads; a float through libm's `cos` and a local one both come out right |
| The machine | four Cortex-A55 cores under a 32-bit ARMv7 kernel, 4.4. About 1.1 GB of memory, 230 to 300 MB of it available with the television's own UI running |
| Who it runs as | an unprivileged user in groups `audio`, `video` and `compositor`, no capabilities. `HOME` is the application folder |
| Where it can write | the application folder, `/tmp`, `/var/tmp` and `/media/developer/temp`. The application folder is removed with the Developer Mode session. It and `/media/developer/temp` survive a cold boot, and the application folder survives a reinstall of the same `.ipk`; `/tmp` is cleared on boot |
| USB | `/tmp/usb` is visible and was empty, with no stick in |
| `/dev/dri` | `card0` is there and opening it fails with `ENODEV`. **Rung 0, kmsdrm, is out** |
| Wayland | `XDG_RUNTIME_DIR=/tmp/xdg`, which holds `wayland-0`. `WAYLAND_DISPLAY` is unset and `wl_display_connect(NULL)` finds the socket anyway. `SDL_VIDEODRIVER=wayland` and `APPID` are set by the launcher |
| The registry | `wl_compositor` v3, `wl_shell` v1, `wl_webos_shell` v1, `wl_seat` v3 three times, `wl_shm`, `wl_output` v2, `wl_webos_foreign`, `wl_webos_input_manager`, `wl_starfish_pointer`, `text_model_factory`, `mali_buffer_sharing`. **No `xdg_wm_base`**, so stock SDL3 has no shell to use and rung 1 is out |
| Drawing | a `wl_shell` toplevel with a `wl_webos_shell_surface` on the same surface, `appId` set and fullscreen asked: the SDL2 fork's setup. The compositor answers fullscreen and exposed at once, and the keyboard enters the surface. EGL 1.4 and GLES 3.2 on a Mali. A moving bar drew at **60.0 fps for a minute**: 5 frames of 3,584 over 20 ms, four of them in the first second |
| Sound | ALSA's `default` opens at 48 kHz stereo S16. A 440 Hz sine played for a minute with **no underruns**. `/tmp/xdg/pulse` exists; PulseAudio was not needed |
| The remote | a standard remote, not a Magic Remote. Every key reaches the surface's `wl_keyboard` as an evdev code on one of the three seats, with `_WEBOS_ACCESS_POLICY_KEYS_BACK` set: arrows 103, 105, 106, 108; OK 28 (`KEY_ENTER`); Back 412 (`KEY_PREVIOUS`); digits as `KEY_1` onwards. The pointer sent nothing. A bar on screen and a clean tone were confirmed by eye and ear |
| Libraries in `/usr/lib` | `libwayland-client`, `libwayland-egl`, `libEGL`, `libGLESv2`, `libasound`, `libpulse`, `libavcodec.so.58` (ffmpeg 4), `libxkbcommon`, `libfreetype` and LG's SDL2 2.0.10. No `libdrm`, no `libgbm` |
| The network | a listener on `0.0.0.0:8177` answers from another machine on the LAN |
| libwayland | older than 1.18: no `wl_proxy_set_tag`/`get_tag` (1.18), `wl_proxy_marshal_flags` (1.20) or `wl_proxy_get_display` (1.23). `wl_proxy_create_wrapper` and `wl_display_prepare_read_queue` are there |
| SDL3 | **draws.** SDL3 3.4.14 built against wayland 1.18's headers and scanner, whose generated code calls only `wl_proxy_marshal_constructor`, with `tools/port/machine/webos/sdl3-webos.patch` making the two tag functions optional. The window is made with `SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN`, and the program gives SDL's `wl_surface` the `wl_shell` and `wl_webos_shell` roles itself. SDL's GLES2 renderer reports 1920x1080 and a moving bar ran at **59.9 fps for a minute**, 9 frames of 3,565 over 20 ms, after a 378 ms stall in the first second. xkbcommon finds no Compose file for locale `C`, which costs only dead keys |
| SDL3's keys | every key arrives as an SDL key event. Arrows, OK as `Return` and the digits as `1` onwards map to their ordinary scancodes and keycodes. **Back does not**: scancode 400 with no name, keycode `0x200003`, `raw` 412. The machine has to map evdev 412 to Back itself, as the SDL2 fork's `SDL_scancode_webos.c` does |
| The machine | **runs, and plays.** Built with the `webos` feature and no `video`, launched from the `.ipk`: the idle screen fullscreen at 1920x1080 with vsync, the wallpapers, the number pad, and the remote's keys working, Back included through the evdev 412 remap. The singer's page opens on a phone, and a song queued from it played a MIDI file with the bundled SoundFont through ALSA's `default`, lyrics in time, no underruns, confirmed by eye and ear. mDNS advertises |
| Its cost | 50 MB resident at idle and 56 MB playing, 79 MB at peak, leaving about 236 MB available. Playing a MIDI song takes about 0.6 of one core of the four |
| Its data | settings, the catalog and the packages folder land under `HOME`, the application folder, so Developer Mode's expiry takes them with it. The developer shell cannot read them. **They survive a cold boot and a reinstall**: launched with no package named, the machine still holds the one installed before |
| A larger bank | ColomboGMGS2 (275 MB) as the bundled bank: the machine starts in about 7 s and sits at 140 MB resident plus 154 MB in swap, of about 900 MB swap the set has. Playing takes about 0.85 of a core. One burst of two xruns about 11 s into the first song, then clean; confirmed by ear |
| MP3+G | **plays**, with no change to the build: the MP3 is decoded by symphonia, not ffmpeg. Four real pairs packed with `km-pack` installed and played, picture and sound confirmed by eye and ear. About 0.23 of a core, under half a MIDI song's, and 57 MB resident, no xruns |
| Storage | about 700 MB free on the 2 GB partition Developer Mode applications share, the machine and its assets taking about 40 MB. MIDI fits tens of thousands of songs; MP3+G, at 6 to 8 MB a song, about a hundred; video a few dozen at best. Songs on a USB stick would change that, and whether the application can read one is not measured |
| The set's ffmpeg | 4.0 (libavcodec 58.18), all four libraries the machine links. **Its H.264 decoder does not work**: Baseline, Main and High alike return a handful of frames and `decode_slice_header error` for the rest. It is built without threads. MPEG-4 Part 2 decodes, 720p at 123 fps on one core. The machine builds against 4.0's headers with `ffmpeg-sys-next` 9.0, needing only `lzo.h` copied in and `..Default::default()` on `threading::Config` |
| Video songs | **play, with an ffmpeg of the application's own**, now what `build.sh` builds: ffmpeg at the pin, 7.1, whose four libraries add 14.6 MB to the application, 12 MB of it libavcodec with every native codec. On a one-minute test pattern at 30 fps, harder to encode than a karaoke video, with the bundled bank: 720p takes about 1.1 cores, and 1080p about 2.1 cores and 150 MB resident and swapped together, against 59 MB idle. No picture dropped, a few late, no xruns. A first spike against a decoder-only ffmpeg 4.0 came to 2.6 MB and decoded 124 fps at 720p and 60 fps at 1080p on five threads. Not yet seen by eye |
| What memory a library costs | the bank, not the songs. Idle with the bundled bank and a video package installed, 58 MB. With ColomboGMGS2 and about 1,100 MIDI songs, about 300 MB resident and swapped together |
| Copying a file onto the set | `ares-push` is refused under the TV profile, and the developer shell takes no stdin. `wget` on the set from an HTTP server on the LAN works; `/media/developer/temp` is world-writable |
| Restarting the set | holding the remote's power button is standby, not a restart. With Quick Start+ off, power off and on is a cold boot. After one, the network comes up late, and the developer port answers only once it has |
| A surprise | once, between two launches, the set was found switched off. The cause is unknown |

**What this means for §3.** Rungs 0 and 1 are out, and rung 2 turned out smaller than it looked.
SDL3's Wayland driver needs no third shell path: its custom-role window hands the surface to the
program, and the webOS roles are about forty lines beside the frame loop rather than inside SDL. The
patch carried is the libwayland one, two symbols made optional. **[repo]** And the build already
has the prebuilt SDL the paragraph after the §3 ladder asks for: the Tizen port switches
`armv7-unknown-linux-gnueabi` to `use-pkg-config` and `link-static` in both `km-display` and
`karaokemachine`, and webOS uses the same triple. So no third dependency block is needed; the webOS
image only has to put its patched SDL3 where pkg-config finds it, as the Tizen image does.
`docs/research/sdl3-upstream.md` lists both platforms' SDL3 fixes as candidates for upstream.

### What is known, by webOS version

What works where, kept up to date as sets are tried. Only webOS 5 has been run.

| webOS | Sets | Known | Source |
|---|---|---|---|
| 5 (`sdkVersion` 5.x) | 2020 | Everything in the table above: native `.ipk` through Developer Mode, softfp and glibc 2.28, `wl_shell` + `wl_webos_shell` with no `xdg_wm_base`, libwayland older than 1.18, SDL3 drawing at 60 fps through a custom-role window, ALSA `default`, remote keys reaching SDL3 with Back mapped from evdev 412, **the machine running and playing MIDI and MP3+G songs**, video songs at up to 1080p30 with an ffmpeg of its own since the set's cannot decode H.264, its data surviving a cold boot and a reinstall, and a 275 MB bank playing through swap. Too old for LG's Flutter route | **[set]** |
| 5 and later | | Native applications install through Developer Mode on a stock set; Kodi ships so. The machine's `.ipk` finds every library and symbol it needs on each firmware `webosbrew-ipk-verify` knows from 5.3.1 to 11.2.0, and on none before 5 | **[web]**, §1; **[source]**, §14 |
| 24 and 25 | 2024, 2025 | Chromium 120 is the web engine on webOS 25. The root chains reach 2025 sets only on the webOS 10.0 they shipped with. On webOS 24 (9.x), sets on the `k3lp` and `k5lp` boards can ship a broken jail configuration in which a native application's graphics fail to start; moonlight-tv detects it by `/dev/rtkmem` being unreadable | **[web]**, §1; **[source]**, §14 |
| 26 Re:New and later | | LG's Flutter for webOS TV: an official `native` application type with C++ plugins, LG's NDK for `ca9v1`, Chromium 132. Whether its compositor offers `xdg_wm_base`, and how new its libwayland is, are not known | **[web]**, §1 |

## 14. What moonlight-tv shows

**[source]** `mariotaku/moonlight-tv` is a game-streaming client that has shipped as a native SDL
`.ipk` on webOS for years, on more firmware than has been tried here. Read 2026-11-24. It uses the
SDL2 fork, bundled under `lib/backports` with an rpath, which §3 rules out here. What else it does
that this port does not:

| | What it does | What it means here |
|---|---|---|
| **Hardware decode** | `mariotaku/ss4s` feeds compressed frames and PCM to NDL DirectMedia | §5: video songs could be decoded by the television |
| **Checking firmware it has never run on** | `webosbrew-ipk-verify`, from `webosbrew/dev-toolbox-cli`, checks the `.ipk`'s libraries and symbols against each webOS release's, and its CI writes the report into the job summary | the only way found to say which webOS versions the `.ipk` loads on without owning those sets. Only webOS 5 has been run (§13). **Taken**: the release's `webos` job runs it. **Run 2026-11-24** against tool release `v20261017-2d5a7b6`: the `.ipk` loads on every firmware it knows from webOS 5.3.1 to 11.2.0, and on none from 1.2.0 to 4.10.0, where glibc lacks `memfd_create`, `powf`, `expf`, `logf` and `exp2f` at 2.27 and `fcntl`/`fcntl64` at 2.28. That is the build's glibc 2.28 and nothing else, so a build against glibc 2.24 would likely reach webOS 4 |
| **The broken webOS 24 jail** | when the window cannot be made on webOS 9 or later, it opens a known-issues page with `SDL_OpenURL` | **Taken**, as a system notification rather than a web page: `webos::explain_no_screen` says why there is no screen, and on a board with the broken jail names the fix. moonlight-tv's wiki gives it as Developer Mode off, a restart and on again, which removes the applications Developer Mode installed. A failure that crashes the graphics driver rather than returning an error is not caught |
| **The Homebrew Channel** | each release carries a `.manifest.json` made by `webosbrew-gen-manifest`, which is what a listing in `webosbrew/apps-repo` points at | rooted sets would install and update the machine from the Channel, with no Developer Mode expiry. Listing it is an outward act, and a decision |
| **Installing in Developer Mode** | its README points owners to `webosbrew/dev-manager-desktop`, a desktop program that takes the key and installs the `.ipk` | **Taken**: the manual's route, with LG's command-line tools as the alternative. It also shows Developer Mode's time left and renews the session, on a schedule if asked |
| **Packing** | `webosbrew/ares-cli-rs`, a Rust `ares-package` shipped as a Linux `.deb` | **Taken**, v0.7.0 in an `ubuntu:24.04` container, since it wants glibc 2.34. It replaced the npm packer and the hand-written archive: the same files and file modes, every directory 0777 and group 5000, and `webosbrew-ipk-verify` finds nothing no set would install. Installed on the webOS 5 set over an earlier install, whose settings and library survived: they live in the application's own folder, which a reinstall leaves alone |
| **The remote** | Back as Escape, EXIT opening its overlay, Channel up and down as page up and down, the four colour keys as actions, HOME captured only when the owner asks, through the fork's access-policy hints | the machine maps the arrows, the digits, OK and Back. It has no list on the television to page, so the Channel keys have nothing to do |
| **Luna from a Developer Mode application** | reads the television's own name from `com.webos.settingsservice` (`network`, `deviceName`), falling back to `modelName` from `com.webos.service.tv.systemproperty`; returns to the home bar by launching `com.webos.app.home` with `launchType` `homeKey` | **Tried and removed.** On the webOS 5 set the settings service answered through `libhelpers` and a fresh install took the name, but a set never renamed answers `[LG] webOS TV <model>`, and no other platform names the machine after its device. The decision is in `docs/decisions/api-and-network.md` |
| **Startup** | sets `EGL_PLATFORM=wayland` and `XDG_RUNTIME_DIR=/tmp/xdg` when the launcher has not, and sets `RLIMIT_CORE` to zero in release builds | **[inferred]** a core of a process this size would take a large share of the 2 GB partition Developer Mode applications share (§13). **Taken**, in `webos::prepare_process` |
| **An unwritable home** | settings fall back to `/tmp` and the application runs without persistence | the `.ipk` makes its folder 777 instead; the fallback would cover a firmware whose installer differs |
| **`appinfo.json`** | `largeIcon`, `iconColor`, `splashBackground`, and `dialAppName` | **Taken**, all four. The icons at LG's 80 and 130 pixels; the splash is `icon/splash-1920x1080.png`. `dialAppName` `KaraokeMachine` registers the application with the set's DIAL service as soon as it is installed: on the webOS 5 set `GET http://<set>:36866/apps/KaraokeMachine` answered `stopped`, a `POST` started the machine (201, its API answering), and a `DELETE` of the `run` link stopped it. The Application-URL comes from the UPnP description on port 1394. No client of ours speaks DIAL yet |

## Sources

- webOS Homebrew: the native development, filesystem and environment setup guides.
- `webosbrew/meta-lg-webos-ndk` and `webosbrew/SDL-webOS`, read for the toolchain and for the six
  webOS files inside the Wayland driver.
- LG's webOS TV developer news of 2026-06-30, "Flutter for webOS TV is now available for developers",
  and the Flutter for webOS guide: the supported versions, the Ubuntu host, Developer Mode.
- `lg-flutter-webos/flutter-webos`: `doc/getting-started.md` for the NDK, and
  `templates/app/meta/appinfo.json.tmpl` and `templates/app/runner/main.cc` for the application
  type and the runner. `lg-flutter-webos/plugins`: `doc/plugin-guide.md`, `doc/plugin-list.md`, and
  `packages/audioplayers_soloud` (`pubspec.yaml`, `webos/CMakeLists.txt`, `webos/Configure.cmake`,
  `webos/webos_player.cc`).
- LG's JavaScript service guides, for the C/C++ add-on rule and the jail. LG's web engine
  specification, for the Chromium version of each release. LG's app ecosystem and approval process
  pages, for individual sellers and QA.
- `throwaway96/faultmanager-autoroot` and `throwaway96/dejavuln-autoroot`, for which firmware the
  later root chains reach. Community reports of renewing the Developer Mode session from cron and
  from Home Assistant.
- LG's webOS TV developer documentation: the Developer Mode application.
- Kodi's webOS installation instructions, for native applications on a stock television.
- RootMyTV, for which firmware the exploit chain reaches.
- `mariotaku/moonlight-tv`: `src/app/platform/webos/`, `src/main.c`, `src/app/app.c`,
  `src/app/ui/root.c`, `CMakeLists.txt`, `cmake/PackageWebOS.cmake`, `cmake/AresPackage.cmake`,
  `deploy/webos/appinfo.json` and its webOS build and release workflows. `mariotaku/ss4s`:
  `modules/webos/ndl/webos5/` and `modules/webos/utils/jail_check.c`.
- `docs/architecture/android.md`, `docs/architecture/audio.md`, `docs/architecture/display.md`,
  `docs/architecture/appliance.md` and `docs/decisions/distribution.md` in this repository.
