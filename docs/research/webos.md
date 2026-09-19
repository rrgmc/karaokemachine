# Research: the machine on an LG webOS television

**Research only. Nothing implemented, nothing decided.** A platform is a product decision, so a
webOS carrier needs an entry in `docs/decisions/distribution.md` beside
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios) first.

Investigated 2026-09-13. **No claim here was verified on a television**, and the marker on each says
how far it can be trusted.

**Summary.** The whole machine runs on an LG television as a native armv7 Linux program installed by
sideload. It is a homebrew carrier rather than a shippable one. The engine is ready: it already plays
all three song kinds on a 32-bit Cortex-A55 television box at 59.5 fps. And webOS is glibc Linux, so
the machine is an ordinary executable there and needs none of the shell code both mobile ports carry.

Two questions decide the answer, and nobody has asked either of hardware. **There is no SDL3 for
webOS**, and cpal reaches ALSA where PulseAudio owns a television's sound. Two further findings shape
the product rather than block it. An installed application is removed when the Developer Mode session
runs out. And a television has about a third of the memory the Android measurements had.

**Nothing is
being built from this.** If it is ever taken up, one afternoon with a television answers five
questions that can each end it.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository. High confidence. |
| **[source]** | Read from webOS or SDL source, or from a file listing. High confidence. |
| **[web]** | Public documentation and community reports. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. What a webOS television permits

**[web]** Native applications are not officially supported on a retail television, and they install
and run on a stock, unrooted one through Developer Mode. Kodi ships that way for webOS 5 and later.
It is a larger native program than this one, so it settles whether the path works at all.

**[web]** The toolchain is `webosbrew/meta-lg-webos-ndk`, a Yocto SDK producing **armv7a glibc**
binaries. The kernel is arm64 and the userland is not, so the Rust target is
`armv7-unknown-linux-gnueabihf`. Installation is an `.ipk` pushed with `ares-install`, and the
launcher starts the program as an ordinary ELF executable with a JSON string in `argv[1]`.

**[repo]** That last point removes the layer both mobile carriers need. iOS has
`crates/machine/km-machine-ios` because the application binary is Swift. Rust has to be a
`staticlib` underneath it. Android has `MainActivity` because SDL's Java calls into the library.

A webOS program starts the way the Debian package starts, so `src/main.rs` and `km_app::run` are the
entry point. No shim crate exists, and `run_on_phone`, `ioscfg` and the `cdylib` crate type all stay
untouched. **This is the largest single saving the platform offers.**

**[web]** Applications draw through LSM, the webOS Wayland compositor, using `wl_webos_shell` rather
than `xdg_wm_base`. Audio is ALSA with PulseAudio above it. Hardware video decode is reachable only
through the television's media pipeline, `com.webos.media` and NDL DirectMedia. That pipeline takes a
file or a URL and draws on a plane behind the GL surface.

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
dump in §2 settles it.

**The ladder, stopping at the first rung that works:**

0. **kmsdrm**, if the jail can open a DRM card. No new SDL code at all.
1. **Stock SDL3 Wayland**, if `xdg_wm_base` is advertised. A hint or two, and `wl_webos_shell` for
   the application id and fullscreen on top.
2. **Patch SDL3's Wayland driver** with the webOS extensions. SDL3's driver descends from the one
   the fork patched, so this is carrying a patch forward rather than writing one.
3. **A new SDL3 `webos` video driver** over EGL and the webOS shell. Weeks, and an SDL video driver
   maintained here for as long as the port lives.

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
startup that it cannot play them. That is a supported shape of every carrier today.

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
exploit chain, so it cannot root a television bought today.

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
| A patched SDL is needed | a third `sdl3` dependency block, and the additive-features trap that has already been paid for once |
| The bank does not fit in a television's RAM | the offered bank list shrinks to the small entries |
| The jailer hides USB from the application | songs have no source; a restart is the reported workaround |
| The Developer Mode session expires | the machine is removed and pushed again, and its data has to outlive it |
| Remote keys arrive through a Luna service rather than the compositor | a new input source, where today there is none |

## 11. Out of scope, whatever the answer

- **Hardware video decode.** The media pipeline is handed a file or a URL and draws behind the GL
  surface. It cannot carry a song whose picture and whose sound are produced separately.
- **The LG Content Store.** The answer to how this is installed is a sideload, and a decision entry
  says so plainly rather than hedging.
- **Any carrier with an indefinite life.** Developer Mode expires and the Homebrew Channel needs a
  rooted television.
- **The on-screen keyboard.** The machine's own number pad is how a song number is entered.
- **Video-plane punch-through.** That patch serves hardware video behind the GL surface, which this
  machine does not use.
- **Writing to the stick.** It is a source of songs and not a place to put them.
- **Noticing a stick without being asked.** The rescan is somebody's act.

## 12. Recommendation

**Not now.** The Android port on a Google TV device already serves a television. This carrier asks
its owner to keep a developer session alive or lose the machine. And on this platform the display may
turn out not to work at all.

If it is ever taken up, the first act is the afternoon in §2 and nothing else. The bare binary, the
registry dump and the sine tone of §4 belong in one deploy. Between them they answer five questions
that can each end the port, and they cost a day. They turn every estimate here from a guess into a
number. Memory is the second measurement, because it decides which SoundFont banks this carrier can
offer. That is a product question rather than a build one.

The sibling note is [`tizen.md`](tizen.md), which asks the same question of a Samsung television and
gets a harder answer.

## Sources

- webOS Homebrew: the native development, filesystem and environment setup guides.
- `webosbrew/meta-lg-webos-ndk` and `webosbrew/SDL-webOS`, read for the toolchain and for the six
  webOS files inside the Wayland driver.
- LG's webOS TV developer documentation: the Developer Mode application, and Flutter for webOS for
  what an officially supported native path looks like.
- Kodi's webOS installation instructions, for native applications on a stock television.
- RootMyTV, for which firmware the exploit chain reaches.
- `docs/architecture/android.md`, `docs/architecture/audio.md`, `docs/architecture/display.md`,
  `docs/architecture/appliance.md` and `docs/decisions/distribution.md` in this repository.
