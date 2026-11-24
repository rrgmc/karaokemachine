# The machine on an LG webOS television

**A development carrier, not a product.** The machine runs on a webOS 5 set as a native executable
installed through Developer Mode, which removes it when the session lapses. Whether LG becomes a
platform is a product decision that has not been taken; the findings are in
[`docs/research/webos.md`](../research/webos.md) §13.

## What is different from desktop Linux

Everything is behind the `webos` cargo feature of `karaokemachine`, in `src/webos.rs`, and the
television defaults the Tizen build already has.

| | Desktop Linux | webOS |
|---|---|---|
| The window | SDL3's own, through `xdg_wm_base` | the compositor has no `xdg_wm_base`. SDL3 makes a custom-role window, and `webos::create_window` gives its `wl_surface` a `wl_shell` toplevel and a `wl_webos_shell_surface` with the application id, fullscreen asked. Both interfaces are reached through `dlopen`, as SDL reaches libwayland |
| SDL3 | built from source by `sdl3-sys` | prebuilt and patched in the `km-webos` image. `armv7-unknown-linux-gnueabi` selects the prebuilt in `km-display` and `karaokemachine`, the same split the Tizen build uses. `sdl3-webos.patch` makes two libwayland 1.18 symbols optional, since the set's libwayland is older |
| Back | `Escape` or `AC_BACK` | evdev 412 with no SDL keycode. `webos::remap_event` turns it into `AC_BACK` before the display sees it |
| Fullscreen, window stack, pointer, key hints, file manager, browser | as configured | fixed or absent, through the constants `display.rs` already gates on `tizen` |
| Number pad, re-pitching recordings | off, on | on, off, as on the other televisions |
| Sound | cpal on the system default | the same, which on the set is ALSA's `default` |
| Video songs | with the `video` feature | not built. The set carries ffmpeg 4 and the machine is built against 7.1 |
| Assets | beside the executable or the checkout | `assets/` beside the executable in the `.ipk`: the SoundFont, the wallpapers, and DejaVu as `fonts/karaoke.ttf` |

## The build

`tools/port/machine/webos/build.sh` builds the `km-webos` image, then everything in it, into
`local/webos/`, and packs one `.ipk` of type `native`.

- **The image is Debian buster's armel cross-compiler.** Buster's glibc is the set's, 2.28. Buster is
  past its support, so apt reads `archive.debian.org`, security suite included.
- **SDL3 is built against wayland 1.18's headers and scanner**, built in the image for the build
  machine. 1.18's scanner still generates `wl_proxy_marshal_constructor`, which the set has.
- **ALSA for cpal is unpacked, not installed.** Multiarch would pull an armel libc that conflicts with
  the cross one, so the two packages go to `/opt/alsa-armel` with a `.pc` of their own.
- **The `.ipk` is repacked in the image.** On Windows `ares-package` writes every file 0666, and the
  set's installer refuses GNU ar's `name/` headers, so the archive is written by hand.

## Running it

The `.ipk`'s main executable is the probe, `km-webos-probe`, which starts the machine for
`"machine": 1` and serves its log on port 8180. `run.sh <device> '{"machine":1}'` installs and
launches it. `KM_WEBOS_PACKAGE` carries a package into the `.ipk`, and `"package": "<file>"` hands it
to the machine to install. A package of real songs stays in `local/`.

The probe's other modes measure the set: `"draw"`, `"sdl"`, `"sound"`, and the report on its own.
`docs/research/webos.md` §13 has what each found.
