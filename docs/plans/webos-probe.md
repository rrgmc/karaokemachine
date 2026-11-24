# Plan: the webOS probe

**What will be built:** a probe for an LG webOS television that answers the questions in
[`docs/research/webos.md`](../research/webos.md) §2 that can each end a port. It is not the port:
nothing of the machine runs in it until P5. This file is deleted in the change that records what the
probe found. That change moves the measurements into the research note and, if the gates pass, opens
the port plan and the distribution decision.

**The set it runs on first is a 2020 one, webOS 5**, too old for LG's Flutter route (webOS TV 26
Re:New or later). So this plan is the research note's homebrew route: the probe is a bare native
executable in an `.ipk` of type `native`, installed with `ares-install` through Developer Mode. It
answers whether native code, SDL3, sound and the remote work on LG at all. It does not answer whether
the official route can be shipped; a webOS 26 set runs the Flutter probe in the last section for that.

**Out of scope:** the machine as an installed application, the Content Store, video songs, USB
sticks, a CI job. A platform is a product decision, so none of this commits to a port.

## What the television needs, once

The steps are LG's, from the Developer Mode application guide and the CLI installation guide.

1. **An LG developer account**, made with SIGN IN on `webostv.developer.lge.com`.
2. **On the set:** connect it to the network. Install **Developer Mode** from the LG Content Store and
   start it. Sign in with that account and turn **Dev Mode Status** on. The set reboots. Start the
   application again and turn **Key Server** on. It shows a six-character passphrase and the
   **Remain Session** time.
3. **The set's address**, from its network settings. The Developer Mode application does not show it.
4. **On the computer:** Node.js, then `npm install -g @webos-tools/cli`. Then `ares-setup-device` and
   **add**: the set's address, port `9922`, user `prisoner`, no password. Then
   `ares-novacom --device <name> --getkey` with the passphrase, and
   `ares-device --system-info --device <name>`.
5. **Read the version** from that output. `sdkVersion` 5.x is webOS 5.

Developer Mode ends when the session time runs out, or after ten reboots without a network. Either
way it removes everything installed under it. **EXTEND** in the application adds time while the set
is online.

## What the developer shell already said

`ares-novacom --run` gives a shell as `prisoner`, outside any application's jail. On the webOS 5 set:

- `/lib/ld-linux.so.3` and glibc 2.28. **The userland is softfp, `gnueabi`**, not the `gnueabihf` the
  research note inferred for the homebrew toolchain.
- Four Cortex-A55 cores running a 32-bit ARMv7 kernel, with VFPv4 and NEON. About 1.1 GB of memory,
  about 300 MB of it available with the television's own UI running.
- `/usr/lib` holds `libwayland-client`, `libEGL`, `libGLESv2`, `libasound`, `libpulse`,
  `libavcodec.so.58` and LG's own SDL2 2.0.10.

P1 asks the same questions again from inside the application, where the jail applies.

## Where it lands

| Piece | What it is | Where |
|---|---|---|
| Image | Debian buster with its armel cross compiler and Rust. Buster's glibc is the set's, 2.28, so nothing links against a symbol the set lacks. It is the Tizen image's arrangement, one Debian release older | `tools/port/machine/webos/Dockerfile` |
| Probe | `km-webos-probe`, a Rust executable in a crate of its own outside the workspace. It is Rust because whether Rust's prebuilt std loads on the set is one of the questions | `tools/port/machine/webos/probe/` |
| Scripts | `build.sh` builds the probe in the image and packs the `.ipk`. `run.sh` installs and launches it with `ares-install` and `ares-launch` | `tools/port/machine/webos/` |

**The target is `armv7-unknown-linux-gnueabi`** with
`-C target-feature=-soft-float,+neon,+vfp4`, the Tizen build's: hardware floating point inside, the
softfp calling convention at the boundary. The report carries a float check with a known result.

**The probe serves HTTP on `0.0.0.0:8177`.** `GET /report` returns what P1 measured, and `GET /log`
returns the log. Each later milestone is a mode of the same executable, chosen by the launch
parameters `ares-launch --params` passes in `argv[1]`.

## Milestones

Each milestone is recorded in the research note as it passes. P1 is the gate.

**Where it stands, 2026-11-24:** P0, P1, P3 and P4 passed. P2 passed through SDL3, at 60 fps in a
custom-role window, with `sdl3-webos.patch` for the set's older libwayland; SDL3's keys are being
checked. P5 is next. Research note §13.

| Milestone | Deliverable | Verified by | Size |
|---|---|---|---|
| P0 Toolchain | the image; a probe that only writes a line, packed, installed and launched | the line read back from the set | S |
| P1 Native code runs | the report: the ELF loader and glibc version, `uname`, which directories are writable and which survive a reboot, `/tmp/usb`, `/dev/dri`, the environment (`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `APPID`), free memory, the float check, and which of the libraries above are present. The HTTP listener starts | `curl http://<set>:8177/report` from another machine | S |
| P2 The surface | a `wl_registry` dump with versions. Then SDL3 drawing a moving bar, climbing the research note's §3 ladder: kmsdrm, stock Wayland, the webOS shell patched in, a driver of our own. There is no host window to borrow | 60 fps for a minute, watched on the set; frame gaps logged | M |
| P3 Sound | a 440 Hz sine through cpal on ALSA's `default`. If that fails, PulseAudio | heard clean for a minute; underruns logged | S |
| P4 The remote | which keys reach SDL3, and under which keycodes | arrows, OK, Back and digits each seen once | S |
| P5 The machine | the machine as an executable with a `webos` feature, launched the same way: the idle screen, the pad, the API, then a MIDI song | the idle screen on the set, the owner's page open on a phone, a song heard | M–L |

## Risks

| Risk | Why it matters | Answer |
|---|---|---|
| The jail mounts the application folder `noexec` | P0 fails and the homebrew route ends | none on this route; the Flutter route below loads a library instead |
| LSM advertises no shell SDL3 accepts | SDL3 has no window | the §3 ladder; it ends at "not on this hardware" |
| The memory left beside the television's UI | about 300 MB available | measured in P1 and watched in P5 |
| Developer Mode expires mid-work | the probe is removed | EXTEND while the set is online, and `run.sh` reinstalls |
| A firmware update moves the ground | the measurements belong to one version | the version is recorded with every result |

## For a webOS 26 set: the Flutter probe

The official route, for when a set new enough is at hand. LG's Flutter template with one C++ plugin
that opens a library of ours with `dlopen` and passes its report back over a method channel. The image
adds Node 16, `@webos-tools/cli` 3.2.4 or later, `flutter-webos` and LG's NDK,
`webos-ndk-flutter-starfish-x86_64-ca9v1-11.2.0.sh`, on Ubuntu 24.04, which is the only host LG
supports. The probe executable above becomes a `cdylib`, unchanged inside. P2 then tries a
`wl_subsurface` of the host's window first, Samsung's arrangement, which needs the plugin to reach the
embedder's `wl_surface`. Open on that route: whether `flutter-webos run` needs the passphrase key that
`ares` already holds, and whether an ahead-of-time release build behaves as the debug build does.
