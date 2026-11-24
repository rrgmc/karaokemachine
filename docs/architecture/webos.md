# The machine on an LG webOS television

**An unofficial carrier.** The machine runs on a webOS 5 set as a native application, Karaoke
Machine (`com.rrgmc.karaokemachine`), installed through Developer Mode, which removes it when the
session lapses. The release carries it as an unofficial download: the decision is
[`An LG television carrier is an unofficial download`](../decisions/distribution.md#an-lg-television-carrier-is-an-unofficial-download),
and the findings are in [`docs/research/webos.md`](../research/webos.md) §13.

## What is different from desktop Linux

Everything is behind the `webos` cargo feature of `karaokemachine`, in `src/webos.rs`, and the
television defaults the Tizen build already has.

| | Desktop Linux | webOS |
|---|---|---|
| The window | SDL3's own, through `xdg_wm_base` | the compositor has no `xdg_wm_base`. SDL3 makes a custom-role window, and `webos::create_window` gives its `wl_surface` a `wl_shell` toplevel and a `wl_webos_shell_surface` with the application id, fullscreen asked. Both interfaces are reached through `dlopen`, as SDL reaches libwayland |
| SDL3 | built from source by `sdl3-sys` | prebuilt and patched in the `km-webos` image. `armv7-unknown-linux-gnueabi` selects the prebuilt in `km-display` and `karaokemachine`, the same split the Tizen build uses. `sdl3-webos.patch` makes two libwayland 1.18 symbols optional, since the set's libwayland is older |
| No screen | logged, and the machine goes on without one | logged, and also shown as a system notification by `webos::explain_no_screen`, sent through the set's own `libhelpers` to `com.webos.notification/createToast`. On the `k3lp` and `k5lp` boards with `/dev/rtkmem` unreadable, the sign of the Developer Mode setup a webOS 24 update can leave unfinished, it names the fix: Developer Mode off, a restart, and on again |
| The machine's name | `KaraokeMachine` until the owner renames it | on a fresh install, the name the owner gave the television, read by `webos::television_name` from `com.webos.settingsservice` through `libhelpers`; `KaraokeMachine` where the set will not say. A rename afterwards stands |
| Back | `Escape` or `AC_BACK` | evdev 412 with no SDL keycode. `webos::remap_event` turns it into `AC_BACK` before the display sees it |
| Fullscreen, window stack, pointer, key hints, file manager, browser | as configured | fixed or absent, through the constants `display.rs` already gates on `tizen` |
| Number pad, re-pitching recordings | off, on | on, off, as on the other televisions |
| Sound | cpal on the system default | the same, which on the set is ALSA's `default` |
| Video songs | with the `video` feature, against the system's ffmpeg or the pinned one | with the `video` feature, against ffmpeg at the pin, which the `.ipk` carries in `lib/`. The set's own is 4.0, without threads, and its H.264 decoder does not work. The probe puts `lib/` on `LD_LIBRARY_PATH`, and the sonames, `.61` against the set's `.58`, never meet |
| Assets | beside the executable or the checkout | `assets/` beside the executable in the `.ipk`: the SoundFont, the wallpapers, and DejaVu as `fonts/karaoke.ttf` |

## The build

`tools/port/machine/webos/build.sh` builds the `km-webos` image, then everything in it, into
`local/webos/`, and packs one `.ipk` of type `native`.

- **The image is Debian buster's armel cross-compiler.** Buster's glibc is the set's, 2.28. Buster is
  past its support, so apt reads `archive.debian.org`, security suite included.
- **SDL3 is built against wayland 1.18's headers and scanner**, built in the image for the build
  machine. 1.18's scanner still generates `wl_proxy_marshal_constructor`, which the set has.
- **ffmpeg is built at the pin in `tools/setup/ffmpeg-pin.sh`**, with its cross configure line less
  zlib, as the Tizen image does. Unlike Tizen's it is shipped: `build.sh` copies the libraries the
  machine names in `DT_NEEDED` into the `.ipk`'s `lib/`.
- **ALSA for cpal is unpacked, not installed.** Multiarch would pull an armel libc that conflicts with
  the cross one, so the two packages go to `/opt/alsa-armel` with a `.pc` of their own.
- **The `.ipk` is packed in a container by webosbrew's `ares-package`**, the Rust one from
  `ares-cli-rs`, fetched by checksum into `local/webos/`. It writes the plain `ar` headers the set's
  installer takes, every directory 0777 and group 5000, and each file with its own mode, which the
  copy into the container makes real. Its binaries want glibc 2.34, so the container is
  `ubuntu:24.04` rather than the km-webos image. LG's npm packer on Windows wrote every file 0666,
  and the archive used to be rewritten by hand.
- **The launcher's icons are `icon/icon-80.png` and `icon/icon-130.png`**, as `icon` and
  `largeIcon`, on `iconColor` `#2a0e42`, the theme's `icon_ground`.
- **The release checks it against firmware it has never run on.** The `webos` job in `release.yml`
  runs `webosbrew-ipk-verify` from `webosbrew/dev-toolbox-cli`, pinned to a release, and writes which
  webOS versions have every library and symbol the `.ipk` needs into the job summary. A firmware
  missing one is reported and passes; a package no set would install fails the job.

## Running it

**`build.sh` packs Karaoke Machine**, `com.rrgmc.karaokemachine_<version>_arm.ipk`, whose main
executable is the machine. Four things make that work:

- **The process is settled first.** `webos::prepare_process`, the first call in `cli::main`, turns
  core dumps off, since one would take a large share of the partition Developer Mode applications
  share, and fills in `XDG_RUNTIME_DIR=/tmp/xdg` and `EGL_PLATFORM=wayland` if the launcher has not.
- **The launcher's JSON is dropped.** webOS passes the launch parameters as one JSON object in
  `argv[1]`, which `webos::launch_args` removes before clap sees the command line.
- **ffmpeg is found through an rpath.** Under `webos`, `build.rs` links the executable with
  `$ORIGIN/lib`, since the launcher sets no `LD_LIBRARY_PATH`.
- **The application folder is mode 777 in the package.** It is `HOME`, where the settings and the
  catalog go, and the application runs as a user of its own that the installer does not make its
  owner. At 755 the machine cannot create its data folder and exits at once.

**`build.sh --probe` packs KM Probe instead**, `com.rrgmc.kmprobe`, whose main executable is the
probe, `km-webos-probe`. It starts the machine for `"machine": 1` and serves its log on port 8180.
`run.sh <device> '{"machine":1}'` installs and launches it. The two are separate applications with
separate data. `KM_WEBOS_PACKAGE` carries a package into the `.ipk`, and `"package": "<file>"` hands it
to the machine to install; an absolute path works too. A package of real songs stays in `local/`.
`"frame_stats": 1` passes `--frame-stats`, so a song's frame timing is in the log.

A package too large to carry in the `.ipk` reaches the set by `wget` from the developer shell,
`ares-novacom --run`, into `/media/developer/temp`: `ares-push` is refused under the TV profile.

The probe's other modes measure the set: `"draw"`, `"sdl"`, `"sound"`, and the report on its own.
`docs/research/webos.md` §13 has what each found.
