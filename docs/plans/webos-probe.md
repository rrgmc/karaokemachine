# Plan: the webOS probe

**What will be built:** a probe application for an LG webOS television. It is LG's Flutter template
with one C++ plugin, and the plugin loads a library of ours. Run on a set, it answers the questions in
[`docs/research/webos.md`](../research/webos.md) §2 that can each end a port. It is not the port:
nothing of the machine runs in it until P5, and then only loaded over the probe's own port, as Tizen's
M2 to M4 were. This file is deleted in the change that records what the probe found. That change
moves the measurements into the research note and, if the gates pass, opens the port plan and the
distribution decision.

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
4. **On the computer:** Node.js 14.15.1 to 16.20.2, then `npm install -g @webos-tools/cli`, version
   3.2.4 or later for Flutter. Then `ares-setup-device` and **add**: the set's address, port `9922`,
   user `prisoner`, no password. Then `ares-novacom --device <name> --getkey` with the passphrase, and
   `ares-device --system-info --device <name>`.
5. **Read the version** from that output. Flutter for webOS needs webOS TV 26 Re:New or later. An
   older set cannot run P1 to P4 as planned, and the fallback below applies.

Developer Mode ends when the session time runs out, or after ten reboots without a network. Either
way it removes everything installed under it. **EXTEND** in the application adds time while the set
is online.

## Where it lands

| Piece | What it is | Where |
|---|---|---|
| Image | Ubuntu 24.04, Node 16, `@webos-tools/cli`, `flutter-webos` and LG's NDK, `webos-ndk-flutter-starfish-x86_64-ca9v1-11.2.0.sh`. LG supports Ubuntu hosts only, and this repository is developed on Windows | `tools/port/machine/webos/Dockerfile` |
| Host | `flutter-webos create --platforms webos`, cut down to one screen of text and a key log | `tools/port/machine/webos/probe/app/` |
| Plugin | C++, a few dozen lines. It opens our library with `dlopen`, calls its entry point, and passes the report back over a method channel | `tools/port/machine/webos/probe/plugin/` |
| Library | `libkm_probe.so`, a Rust `cdylib` in a crate of its own outside the workspace. It is Rust because whether Rust's prebuilt std loads on the set is one of the questions | `tools/port/machine/webos/probe/native/` |
| Scripts | `build.sh` builds everything in the image. `run.sh` installs and starts it with `flutter-webos run`. `load.sh` sends a new library to a running probe | `tools/port/machine/webos/` |

**The probe serves HTTP on `0.0.0.0:8177`**, like Tizen's KM Probe. `GET /report` returns what P1
measured, `GET /log` returns the log, and `POST /load` takes a new library, loads it and calls it. P2
to P5 then iterate in about a minute each, without a reinstall.

**The target triple is the open question P1 settles.** The plan starts from Tizen's,
`armv7-unknown-linux-gnueabi` with `-C target-feature=-soft-float,+neon,+vfp4`. LG's NDK names
`gnueabi`, and its plugins build `softfp`. The NDK's own compiler is the linker.

## Milestones

Each milestone is its own pull request, recorded in the research note as it passes. P1 is the gate.

| Milestone | Deliverable | Verified by | Size |
|---|---|---|---|
| P0 Toolchain | the image, LG's hello-world built and run on the set | `flutter-webos doctor` all ticks; the hello-world on screen; the version from `ares-device` recorded | S |
| P1 Native code loads | the plugin loads `libkm_probe.so`, and the screen shows its report. The report covers the ELF loader and glibc version, `uname`, which directories are writable and which survive a reboot, `/tmp/usb`, `/dev/dri`, the environment (`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `APPID`), free memory, and which of `libwayland-client`, `libEGL`, `libGLESv2`, `libasound`, `libpulse`, `libavcodec` and SDL are in `/usr/lib`. The HTTP listener starts | the report on screen, and `curl http://<set>:8177/report` from another machine | S |
| P2 The surface | a `wl_registry` dump with versions. How the plugin reaches the embedder's `wl_display` and `wl_surface`, found by reading LG's `FlutterWindow` headers. Then SDL3, with the Tizen patches if they are needed, drawing a moving bar into a subsurface of that window | 60 fps for a minute, watched on the set; frame gaps logged | M |
| P3 Sound | a 440 Hz sine through cpal on ALSA's `default`. If that fails, SDL3's PulseAudio backend, then the platform's `ndlmedia` | heard clean for a minute; underruns logged | S |
| P4 The remote | which keys reach Flutter, logged in Dart, and which reach SDL3 over the subsurface. If they reach only Flutter, keys are forwarded into SDL's queue over the method channel | arrows, OK, Back and digits each seen once | S |
| P5 The machine | the machine as a `cdylib` with a `webos` feature and a `km_probe_run` entry point, loaded over `POST /load`: the idle screen, the pad, the API, then a MIDI song | the idle screen on the set, the owner's page open on a phone, a song heard | M–L |

**Fallback, if the set is older than webOS 26 Re:New, or P1 fails:** the homebrew route in the
research note. It is the same library, started as a bare executable from an `.ipk` of type `native`,
on the webosbrew NDK. P2 to P4 still run, minus the host's window. That answers whether SDL3 can draw
on LG at all, but not whether the official route can be shipped.

## Risks

| Risk | Why it matters | Answer |
|---|---|---|
| LG's embedder exposes no `wl_surface` to a plugin | the Samsung design has nowhere to draw | a window of SDL's own (research note §3, rungs 1–3). On Samsung that failed |
| The float ABI is wrong | a library that loads and then computes wrongly | P1 reports the loader, and a float test in the report checks a known result |
| Developer Mode expires mid-work | the probe is removed | EXTEND while the set is online, and `run.sh` reinstalls |
| The jail blocks `dlopen` from the app folder, as Tizen 4 to 5.5 did | P1 fails | an in-memory loader, as OmegaTube's on Tizen |
| A firmware update moves the ground | the measurements belong to one version | the version is recorded with every result |

**Open on the way:** whether `flutter-webos run` needs the passphrase key that `ares` already holds, or
its own `custom-devices get-key`; and whether a release build, ahead-of-time compiled, behaves
differently from the debug build P0 runs.
