# Research: the machine and the tools in a Linux desktop session

**Research only. Nothing implemented, nothing decided.** What the product *is* on Linux is a
product decision. Anything here that changed it would need
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`A second Linux carrier`](../decisions/distribution.md#a-second-linux-carrier) to say so first. A
window for the three tools would need
[`The package builder's window`](../decisions/curation.md#the-package-builders-window),
[`The remote's window, and its portable core`](../decisions/remotes.md#the-remotes-window-and-its-portable-core)
and [`A running server has an icon in the bar`](../decisions/interface.md#a-running-server-has-an-icon-in-the-bar)
reopened.

Investigated 2026-09-13. **No claim here was checked in a live X11 or Wayland session.** The
repository's Linux experience comes from a `kmsdrm` appliance with no display server on it. That is
the one Linux configuration that answers none of the questions below.

**Summary.** The machine is an ordinary SDL3 desktop application on Linux, and the packaging already
carries what that needs. The finding that reorders the rest is SDL's own. On a Wayland desktop SDL3
**deliberately prefers XWayland** unless the compositor advertises the `fifo-v1` protocol. So X11 is
the backend a desktop session actually takes today. The `.deb` makes X11 a hard dependency and only
recommends Wayland, and that matches SDL's behavior rather than trailing it.

Three consequences follow. The portable tarball's dependency list names no Wayland library in any of
its three distribution families. That list describes a working machine rather than a broken one. The
known Wayland icon gap closes with one line through the safe API. `SDL_APP_ID` is a hint, and the
machine already sets a hint before `SDL_Init`. And nothing verifies any of it.

**No automated check in this repository has ever opened a window on Linux.** Every container the
verifiers run in has no display server and exercises the headless fallback.

The three web-backed tools have no Linux window by decision. They hand the same page to the default
browser, and nothing above affects it. A framebuffer executable separate from an X11 one is not earned. One Linux binary
already holds every backend, and the environment picks between them. The tools are the opposite:
their window is linked rather than opened, and the build-time split is the one they already have.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository. High confidence. |
| **[built]** | Read out of the Linux build this repository produces, in the container that produces it. |
| **[ran]** | Observed by running the machine on Linux — a WSLg session, and a Debian appliance driving a television. |
| **[source]** | Read from SDL3's own source. High confidence. |
| **[web]** | Public documentation. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. Four ways a Linux box puts this on a screen

**[repo]** Only the fourth is decided. `What the machine *is*, on Linux` names a Debian box with no
display server. SDL3's `kmsdrm` backend drives it from a bare virtual terminal.
[`appliance.md`](../architecture/appliance.md) is the whole account of making that work. The other
three are what a `.deb` or a tarball meets on a computer somebody already owns.

| | What the machine gets |
|---|---|
| An X11 session | SDL3's `x11` backend, talking to the X server directly |
| A Wayland session with XWayland | SDL3's `x11` backend, talking to XWayland — **the ordinary case**, per §2 |
| A Wayland session, natively | SDL3's `wayland` backend |
| No session at all | SDL3's `kmsdrm` backend, which is the appliance |

**[repo]** Both desktop deliverables exist already. The package installs a desktop entry and the
`hicolor` icons, and ships its systemd unit **disabled**. So installing it gives a menu entry and
seizes nothing. `A second Linux carrier` adds a portable folder whose `install.sh` runs the
machine's own `--register`. That writes the same entry into `XDG_DATA_HOME`.

## 2. SDL prefers XWayland on a Wayland desktop, and says why

**[repo]** Nothing in Rust chooses a backend. `display.rs:1454` calls `sdl3::init()` with no
subsystem flags and `:1460` calls `sdl.video()`, taking SDL's probe order as it comes. The single
place a driver is named is the appliance's unit,
`crates/machine/karaokemachine/linux/karaokemachine.service:144`, with `SDL_VIDEODRIVER=kmsdrm`. Its
own comment gives the reason. A box that happens to carry X libraries must not quietly pick a
backend needing a server nobody runs. The commented `cage` fallback beneath it names `wayland` as
the unit's second driver.

**[source]** The `bootstrap[]` array in SDL's `src/video/SDL_video.c` puts a **`Wayland_preferred`**
entry ahead of `X11`, and the plain `Wayland` entry **after** it. So the order on a Linux desktop
build is: preferred Wayland, then X11, then fallback Wayland, then `kmsdrm`, then offscreen and
dummy.

**[source]** What `Wayland_preferred` demands is the finding. Beyond `WAYLAND_DISPLAY`,
`XDG_SESSION_TYPE=wayland` and a successful `wl_display_connect`, `Wayland_IsPreferred` queries the
registry for `wp_fifo_manager_v1`. It returns false without it, and logs that the compositor lacks
fifo-v1 and that it is *"falling back to XWayland for GPU performance reasons"*. The check does not
detect XWayland. It assumes XWayland has no fifo-v1 either, and that makes X11 the next entry rather
than a competing one.

**[inferred]** A Wayland desktop whose compositor does not implement fifo-v1 therefore runs this
machine through XWayland **by SDL's choice**. The native Wayland backend is reachable only if X11
fails to initialize at all. `SDL_VIDEODRIVER=wayland` in the environment is what overrides that, and
nothing in the machine sets it.

**[ran]** It was observed on a WSLg session with `WAYLAND_DISPLAY` set and a Wayland compositor
answering. The machine drew a window, and its process had mapped `libX11`, `libX11-xcb`, seven
`libxcb-*` and `libGLX_mesa`. It had mapped **no `libwayland-client` at all**. So SDL took the `x11`
driver through XWayland with a Wayland session available to it. That is §2 happening rather than
being reasoned about. One compositor is one data point, and WSLg's is not a desktop's, but it points
where the source says.

**[inferred]** It remains the claim most worth testing against a desktop compositor, because the
protocol's spread moves under it. A compositor that gains fifo-v1 moves this machine from XWayland to
native Wayland on somebody's computer, with no change here at all.

**[built]** All of that is live in one binary. `build-from-source-static` passes cmake no driver
flags, so SDL enables what the dev packages offer. `apt-deps.sh` offers X11, Wayland and DRM/GBM
alike. The generated `SDL_build_config.h` in the Linux build volume defines `SDL_VIDEO_DRIVER_X11`,
`SDL_VIDEO_DRIVER_WAYLAND`, `SDL_VIDEO_DRIVER_KMSDRM`, `SDL_VIDEO_DRIVER_OFFSCREEN` and
`SDL_VIDEO_DRIVER_DUMMY`. It leaves `WINDOWS`, `VIVANTE`, `ROCKCHIP` and `OPENVR` undefined. The
debug build and both release builds agree.

**[built]** SDL loads every one of them by soname at run time, and that makes §3 a list rather than
a link line:

| Driver | What it opens |
|---|---|
| `x11` | `libX11.so.6`, and `libXcursor.so.1`, `libXext.so.6`, `libXfixes.so.3`, `libXi.so.6`, `libXrandr.so.2`, `libXss.so.1`, `libXtst.so.6` |
| `wayland` | `libwayland-client.so.0`, `libwayland-cursor.so.0`, `libwayland-egl.so.1`, `libxkbcommon.so.0`, `libdecor-0.so.0` |
| `kmsdrm` | `libdrm.so.2`, `libgbm.so.1` |

**[repo]** The packaging maps onto that one for one, which is the check `$auto` cannot perform. The
`.deb`'s hard dependencies are the `x11` row plus the `kmsdrm` row, and its Recommends are the
`wayland` row.

## 3. Every backend is a Recommends, and apt chooses the deployment

**[repo]** `crates/machine/karaokemachine/Cargo.toml` puts in `depends` only what every build needs,
whatever it draws on. That is `libasound2t64`, `libc6`, the `kmsdrm` row, `libudev1` and a font.
Every X11 and Wayland library goes in `recommends`. SDL reaches all of them through `dlopen`, so
apt's choice *is* the deployment. `apt-get install` pulls Recommends and gives a desktop its
backend. `--no-install-recommends`, which `deploy.sh` passes, gives a box under a television a
deployment that cannot run the `x11` driver at all.

**[built]** `libX11` itself survives that, and it is not the packaging's to decline. Mesa's EGL and
gallium packages hard-depend on `libx11-xcb1`, and `kmsdrm` draws through Mesa. So installing the
`depends` list alone brings `libx11-6`, `libx11-data`, `libx11-xcb1` and six `libxcb-*`, 22 shared
objects in all. It does not bring the eight libraries the `x11` row of §2 names, and those decide
whether the driver can start. The verifier asserts those rather than counting `libX11`.

**[repo]** That could not be expressed while the package linked Debian's ffmpeg, and the reason is
§4. `libegl1` and `libgles2` stay hard because `kmsdrm` needs them; `libgl1` stays a Recommends,
being desktop GL that only the X11 path uses.

**[repo]** `$auto` is spelled out rather than asked for. The ffmpeg libraries sit inside the
package, so `dpkg-shlibdeps` resolves them against the build image's `-dev` packages. That puts back
the dependency the bundling removes. So `depends` names what it derived. A `DT_NEEDED` allowlist
over the shipped binary refuses an X, Wayland, VA-API, VDPAU or OpenCL library in its place.

**[inferred]** Read against §2 this is not a loosening. A desktop reaches its screen through
XWayland on the X11 row, which is the path SDL would have chosen with every Wayland library present.

**[source]** `libdecor` is optional inside SDL as well. The Wayland backend falls back to
server-side decorations through `zxdg_decoration_manager_v1` when libdecor is absent or disabled. So
`libdecor-0-0` sitting in Recommends costs a title bar's appearance at worst.

## 4. Debian's ffmpeg is what made X unavoidable

**[built]** `libavutil.so.59` in trixie carries `libX11.so.6` as a direct `NEEDED`, beside
`libva-x11.so.2`, `libva-drm.so.2`, `libva.so.2`, `libvdpau.so.1`, `libvpl.so.2` and
`libOpenCL.so.1`. So a package linking it did not merely have X on disk. An appliance loaded an X
client library, two video-acceleration stacks and an OpenCL client into its process. That box had
no X server and no hardware decode in use.

**[built]** Installing the four ffmpeg runtime packages into a clean `debian:13-slim` with
`--no-install-recommends` and nothing else brings **14 `libX*` files** with them. Measured: that
chain is 4.1 MB, and Debian's four ffmpeg libraries are 21 MB.

**[ran]** And it is not a disk cost. A Debian appliance with no display manager drew through
`kmsdrm` on a bare virtual terminal. It had `libva`, `libva-x11`, `libva-drm`, `libvdpau`, `libvpl`
and `libOpenCL` **mapped into its process** beside `libX11` and the `libxcb` family. Those are six
acceleration libraries loaded on a box that calls none of them. With the package carrying its own
ffmpeg, the same box maps none of the six. What remains is `libX11`, `libxcb` and
`libwayland-client`, which arrive through Mesa.

**[repo]** The package therefore carries the LGPL ffmpeg this repository already builds for the
tarball. `tools/platform/linux/ffmpeg-lgpl.sh` configures it `--disable-autodetect`. It then refuses
its own build unless every library it produced links nothing but `libc`, `libm`, `libgcc_s`,
`libpthread`, `libdl`, `librt` and its siblings. So the libraries a package ships cannot acquire an
X dependency without that script saying so first.

**[repo]** `libopenh264` travels with them. `libavcodec` links it for the one thing ffmpeg cannot do
itself: encode H.264, which is what a streaming run uses. It is C++ where the rest of that directory
is C. So `libstdc++6` is the one `depends` entry no mechanism could have derived, because nothing in
the binary's own dynamic section mentions it.

**[repo]** The tarball's list in `tools/platform/linux/runtime-deps.sh` still names the X11 set for
all three distribution families and no Wayland library in any of them. Under §2 that produces a
working machine either way. So the two carriers disagree about what they *offer* rather than about
whether they work. A tarball cannot take the native Wayland path on a compositor that advertises
fifo-v1. There the preferred entry declines on a missing library rather than on the protocol.

## 5. No check here opens a window on Linux

**[ran]** A person can, and §2 is what came of doing it. What follows is about the automated checks,
which is where the gap is: every one of them runs somewhere with no screen.

**[repo]** `verify-tarball.sh` installs each family's list into a clean container of that
distribution and starts the application. `verify-deb.sh` does the equivalent for the package.
`docs/architecture/distribution.md` already states the limit for audio: a container has no sound
device, and the run says so honestly.

**[inferred]** The same limit covers the screen and is written down nowhere. A container has no
display server and no DRM device, so both verifiers exercise the headless fallback. They prove the
libraries resolve, the binary loads, the layout is right and asset discovery works. They cannot
prove SDL picks a backend, creates a window, or draws a frame.

**[repo]** CI adds nothing here. `.github/workflows/ci.yml` installs the build dependencies through
`apt-deps.sh` and runs the test suite, with no Xvfb and no Weston. `--headless` in this repository
means no window rather than a virtual X server, and the only headless-browser work in the tree
drives Chrome for screenshots.

**[repo]** The reason nothing else covers it is that `sdl.video()` has exactly one call site in the
tree, `display.rs:1460`. Everything else that draws goes through `km_display::offscreen`, which says
it outright:

> *"It needs no video subsystem: `SDL_Init` is never asked for one, and the software renderer draws
> into a plain surface. That is why it runs where CI does, and over SSH on a box with no monitor."*

Its callers are `km-display`'s examples and `--stream`.

**[repo]** So a Linux box can already draw the **complete** screen with no display server. A
streaming run renders every frame through `Offscreen` and hands it to an encoder. That exercises
fonts, the CJK fallback, layout, the wallpaper and every branch of `draw`. What it settles is
narrower than it looks: it says nothing about which backend SDL picks, because it asks for none. But
it means the question *"does this draw at all on Linux"* is separable from §2, and answerable in a
container.

**[inferred]** So every claim in §2, §3 and §4 about what a desktop session does rests on reading
SDL's source. One container each for X11 and Wayland would turn all of them into observations. The
probe is small, because the answer is a string. SDL names the driver it settled on, and the machine
already logs `canvas.renderer_name` beside it at `display.rs:1566`.

## 6. Five things a Wayland session answers differently

| | |
|---|---|
| Standing in front | **[repo]** `display.rs:434-437` names Wayland as the compositor that declines, a client there not placing itself in the stack. `apply_always_on_top` at `:447` is an FFI call whose result is **asked of SDL** at `:466` rather than tracked, so `T` and `WINDOW_STACKS` already survive a refusal |
| The window's icon | **[repo]** The desktop entry's `StartupWMClass=karaokemachine` matches the X11 `WM_CLASS` SDL takes from the executable's name; Wayland matches the surface's `app_id`, which SDL takes from its application metadata. §7 is the one-line answer |
| Leaving fullscreen | **[repo]** `apply_fullscreen` at `:406` calls `SDL_SyncWindow` before setting size and position, because the change is asynchronous wherever a compositor has to agree. Written for the appliance's mode bug and correct here for a different reason |
| Where the window opens | **[inferred]** `position_centered()` at `display.rs:1546` is authoritative on X11 and advisory on Wayland, where a client does not place its own surface |
| Fractional scaling | **[inferred]** `high_pixel_density()` at `:1551` is asked for on every platform, and a 125% or 150% desktop scale is a question the geometry tests' `SCREENS` matrix does not reach — that matrix spans aspect ratios, not scale factors |

**[repo]** The fullscreen path has one more Linux-shaped property. When a run starts fullscreen, the
window is built at the primary display's mode (`display.rs:1512`) rather than at the configured
size. It falls back to the configured size when the mode cannot be read, and that keeps an odd or
headless backend working.

## 7. The icon gap closes with a hint, not with FFI

**[repo]** `docs/architecture/assets.md` and the desktop entry both record the gap, and both give
the same reason. `sdl3-rs` 0.18.4 wraps no application metadata, so nothing sets the Wayland
`app_id`.

**[source]** SDL carries `SDL_HINT_APP_ID`, spelled `SDL_APP_ID`. Its documentation calls it the
app ID string desktop compositors use to identify and group windows and to match applications with
their icons. It overrides the metadata property. It asks to be set before SDL is initialized.

**[repo]** A hint is reachable through the safe API. `sdl3::hint::names::APP_ID` is generated into
the same module `display.rs:1448` names `ORIENTATIONS` from, and `sdl3::hint::set` takes it. So the
shape is a second call beside the orientation one at `:1447`. It sits above `sdl3::init()`, where
the hint's documentation asks for it, and carries `karaokemachine` to match the desktop entry's base
name.

**[inferred]** This costs no `unsafe`, no new dependency and no `cfg`. The hint is inert wherever it
means nothing, exactly as the orientation hint above it is. What it cannot do is prove itself from
Windows: confirming the icon needs a Wayland session, which is the §5 gap again.

## 8. The three tools have no Linux window, by decision

**[repo]** `km-package-builder`, `km-remote` and `km-admin` each declare the same feature,
`desktop = ["dep:wry", "dep:tao", "dep:km-tray", "dep:km-webshell"]`. Cargo leaves it off, and
staging turns it on for Windows and macOS alone. `wry` links libwebkit2gtk and `tray-icon`'s Linux
backend links `libayatana-appindicator`, both at load time. So a Linux build carrying either does
not reach `main`, and `--browser` cannot rescue it, because the failure is in the dynamic loader.
`tao`'s only Linux backend is gtk3.

**[repo]** The enforcement is two absences rather than a rule. `tools/setup/features.sh` omits both
`desktop` features from `KM_FEATURES` and `KM_FEATURES_VIDEO`. That is why `--all-features` is
refused on this workspace. `apt-deps.sh` installs neither `-dev` package, so a build that asked for
one would stop in `javascriptcore-rs-sys` rather than at load time.

**[repo]** What a Linux desktop gets instead is the same page. All three serve the real interface
over loopback, and the webview is a viewer rather than a second front end. So the browser opened
through `km-osopen`'s `xdg-open` shows exactly what the window would.

**[inferred]** Nothing in §2 through §7 touches the tools. Their Linux story is a browser tab on
every session type, so X11 against Wayland is invisible to them. That leaves `km-webshell`'s window
arithmetic compiled out on Linux, and the tray absent for the same reason.

**[ran]** All three build and run on Linux, and the claims above hold where it can be seen. Each has
one executable with no `-console` twin. `km-package-builder`'s entire `NEEDED` is `libgcc_s`,
`libm`, `libc` and the loader. Nothing links `gtk`, `webkit` or `ayatana`, and each serves its own
page over loopback.

**[ran]** The browser is the link that was never tested, and it has two ends. Where `xdg-open` is
present, the tool hands it the exact address it printed. Where it is absent, nothing opens. That is
an ordinary state for a Linux box with no desktop environment, since `xdg-open` comes from
`xdg-utils`. The tools then say which program was missing rather than printing an errno or nothing
at all.

**[repo]** The machine's own `F11` takes that route too. It carries the one place where a desktop
session is charged for the appliance. `BROWSER_KEY_HINT` at `display.rs:340` is `None` for Linux, so
the connect panel never offers the key there. The constant's own reasoning says why. It cannot tell
a bare TTY from a desktop, and a promise half of Linux would break is worse than no promise. The
press still works: `WEB_BROWSER` answers yes on Linux, and a box without `xdg-open` comes back with
the reason on the screen.

**[repo]** `FILE_MANAGER` and `WEB_BROWSER` are `true` on Linux, and the appliance is deliberately
not excluded from either. Both doc comments say so in the same words. A box with no desktop has no
`xdg-open`, and the press comes back with the reason on the screen. That beats a constant that
cannot tell a bare TTY from a desktop pretending otherwise. The inability is the argument for
attempting, not a restriction.

**[inferred]** So a desktop session loses one line of advertisement and no capability. What would
close even that is a predicate the machine could evaluate rather than compile. One is
`VideoSubsystem::current_video_driver`, which `sdl3-rs` 0.18.4 wraps safely and which §2 makes
meaningful. The other is an `xdg-open` lookup on `PATH`, which is the thing actually being
predicted. Either is available at the moment the connect panel is drawn, both being after
`sdl.video()`.

## 9. A second executable is earned when the binary cannot choose at run time

**[repo]** This project splits one program into two executables exactly once.
[`Two executables on Windows`](../decisions/distribution.md#two-executables-on-windows) states the
criterion in one line: *"Only Windows gets two, because only Windows has a subsystem to choose."*
`#![windows_subsystem]` is a property of a binary crate root, fixed in the image before `main` runs
and unreachable from inside it. That is what earns the console twin. It also makes the twin nearly
free, being three lines hanging off a library the pair shares.

**[built]** A framebuffer-versus-X11 split fails that test at the first clause. One Linux binary
already carries `x11`, `wayland` and `kmsdrm` together, per §2. So there is nothing for a second
executable to contain that the first does not.

**[repo]** Nor is there a decision for it to make earlier. The choice happens at run time through
one environment variable. `SDL_VIDEODRIVER=kmsdrm` in the unit is the whole appliance-versus-desktop
switch, and the commented `cage` fallback flips it to `wayland` by editing that same line. A second
executable would put a second build where a one-line edit stands.

**[repo]** And the machine can ask which it got. `sdl3-rs` 0.18.4 wraps
`VideoSubsystem::current_video_driver` safely. So anything that wants to behave differently on a
bare TTY has the answer a line after `sdl.video()`. That is later than a compile-time carrier, and
correct on a box that is both.

**[repo]** What a split would cost is concrete. The `.deb` ships **one** binary in both roles:
`Exec=karaokemachine %f` in the desktop entry, `ExecStart=/usr/bin/karaokemachine` in the unit. That
is what lets `What the machine *is*, on Linux` install as an ordinary application and become an
appliance when somebody enables the unit. Two executables force a choice that design avoids: two
desktop entries, two packages, or one package whose menu entry and whose unit start different
programs.

**[repo]** Cargo makes it worse rather than easier. A `desktop`/`appliance` pair is mutually
exclusive, and cargo features are additive. That is the trap `tools/setup/features.sh` exists to
document, and this workspace has already paid for it once.

**[inferred]** What a split would buy is smaller than it looks. Five of the six platform constants
divide Android and iOS from everything else and already give the appliance and a desktop the same
answer. Only `BROWSER_KEY_HINT` singles Linux out, and §8 is what it withholds.

Narrower dependencies are the one real saving: an appliance binary would need neither the `x11` row
nor `libdecor-0-0`. That buys a smaller install on a box that is already a dedicated one. Against it
stands a second build, stage, verify and package path. And per §5, nothing on that platform opens a
window in any check at all.

**[inferred]** The case that would change this is a carrier where a backend cannot be present. That
is the way `webos.md` reaches for a feature flag, because webOS **is** `target_os = "linux"` and
wants different answers from the appliance. Nothing on Debian is that: the drivers coexist in one
file and the environment picks.

### The three tools answer the same test the other way

**[repo]** `km-package-builder`, `km-remote` and `km-admin` meet the criterion the machine fails,
and the split they earn is the one they already have. A window there is `wry`, and `wry` links
libwebkit2gtk at load time. A build carrying it does not reach `main` on a box without the library.
So *whether this binary can have a window* is settled before any code runs, exactly as
`#![windows_subsystem]` is. That is why §8's `desktop` feature is a build-time gate and not a flag,
and why `--browser` cannot stand in for it.

**[repo]** So each of the three is a different file on Linux from the one Windows and macOS ship.
Cargo says so structurally: every one declares a `<name>-console` twin under
`required-features = ["desktop"]`. So a Linux build produces **exactly one executable** per tool,
and it is the windowless one. The dependency asymmetry is the whole of it. SDL `dlopen`s its
backends by soname, per §2, so one machine binary holds all three. `wry` is linked, so a tool binary
holds its window or cannot start.

**[repo]** The machine's own twin is not this split and is worth telling apart from it.
`karaokemachine-console` carries **no** `required-features`. It is built on every platform and staged
only on Windows, and its own header gives the reason. There is no feature meaning *this build has a
window*, because this one always does. Where there is no subsystem to choose, the two executables
are the same program under two names, and the Linux carriers ship one.

**[inferred]** Nothing about X11 against framebuffer reaches the tools at all. They serve the same
page to a browser on every Linux session type, per §8. So a display-server split would divide them
along an axis they do not have. That leaves the machine as the only program in the tree the
question can even be asked about.

## 10. What each gap would cost

| Gap | Then |
|---|---|
| Nothing opens a window on Linux in any check | A `--stream` container first, which needs no display server and proves the drawing half; then Xvfb and headless Weston, which convert §2, §3, §4 and §6 from reading to observation |
| The Wayland `app_id` | One `sdl3::hint::set` call beside the orientation hint, per §7, plus a Wayland session to see it work |
| The tarball offers no native Wayland | One list in `runtime-deps.sh`, gated on the verifier above being able to fail on it |
| Fractional scaling is unmeasured | Measurement first, then a decision about whether `SCREENS` grows a scale axis |
| The connect panel never offers `F11` on Linux | A runtime predicate in place of one `const`, per §8, and one decision about what the panel may promise |
| Native windows for the three tools | Not a cost but a reversal: it trades *one executable, copy it anywhere* for a window, on the platform where a browser is universal |

## 11. Out of scope, whatever the answer

- **The appliance's `kmsdrm` path.** [`appliance.md`](../architecture/appliance.md) owns it, and
  the unit names the driver so none of §2 applies there.
- **`cage`.** Already the documented fallback, written into the unit as two commented lines.
- **X11-only desktops as a separate target.** Under §2 they take the same backend a Wayland desktop
  takes, so one X11 test covers both.
- **A display manager on the appliance.** `deploy.sh` already warns about one holding DRM master.
- **Anything about the three tools' windows.** §8 is a decision, and reopening it is a decision's
  work rather than a finding's.

## 12. Recommendation

**Say nothing new and test what is already claimed.** The machine is a desktop application on Linux
today, and the packaging is right for the backend SDL actually picks. What is missing is any
evidence for that, and the evidence is cheap.

**Start with `--stream`, which needs no display server at all.** A container starts a streaming run,
fetches `/stream/live.m3u8` and decodes one frame. That proves the drawing half on Linux: fonts,
fallback, layout, wallpaper, every branch of `draw`. It costs one `docker run` against a package
that already exists. It is the cheapest rung because it asks SDL for no video subsystem, and that is
also its limit: it says nothing about §2.

**Then the two containers**, one Xvfb and one headless Weston, each starting the machine and
reporting `canvas.renderer_name` and the driver SDL settled on. Those are what settle §2 and give §3
and §4 a verifier that can fail. After that, the tarball's Wayland libraries become an ordinary
question with an answer.

The `SDL_APP_ID` hint in §7 is the one change worth making without waiting for any of that. It is
one call and needs no `unsafe`. The gap it closes sits in two files as something that cannot be
reached from the safe API.

## Sources

- SDL3's own source, read for the probe order and the preference check: `src/video/SDL_video.c`
  (`bootstrap[]`) and `src/video/wayland/SDL_waylandvideo.c` (`Wayland_IsPreferred`, the fifo-v1
  query, and libdecor's optionality).
- `sdl3-sys`'s generated hint table, for `SDL_HINT_APP_ID` and what a compositor does with it.
- The `SDL_build_config.h` cmake writes inside the Linux build container, read out of the shared
  `karaokemachine-deb-build` volume. It was the source for which video drivers this build carries
  and which sonames each opens. Both release builds and the debug build agree.
- `docs/architecture/appliance.md`, `docs/architecture/display.md`,
  `docs/architecture/assets.md`, `docs/architecture/distribution.md`,
  `docs/architecture/desktop-shell.md`, `docs/decisions/distribution.md`,
  `docs/decisions/curation.md` and `docs/decisions/interface.md` in this repository.
