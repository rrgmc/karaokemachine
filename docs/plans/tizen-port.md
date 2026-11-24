# Plan: the machine on a Samsung television

**What will be built:** the whole machine as an app on a Samsung Tizen television, installed as a
`.tpk`, started from the set's app list, playing every song kind with sound, words and the remote,
and reachable from phones. It carries out
[`What the machine *is*, on a Samsung television`](../decisions/distribution.md#what-the-machine-is-on-a-samsung-television);
where the two disagree, the decision is right. The measurements behind it are in
[`docs/research/tizen.md`](../research/tizen.md). This file is deleted in the change that finishes
the port.

**Out of scope:** the Samsung store, 4K, HEVC and Vorbis/Opus sources, finding a USB stick, and a
release CI job. Each can follow once the app runs.

## The design problem: whose loop runs

**The machine's frame loop runs on the host's UI thread and, once per frame, lets Ecore do one pass
of its work.** That is the smallest change that satisfies both sides. **M1 proved it on the set**
with the probe in `tools/port/machine/tizen/probe/`: smooth frames, live keys and a clean exit,
measured in [`docs/research/tizen.md`](../research/tizen.md) §3.

- **The machine** draws from one long blocking loop, `run_with` in
  `crates/machine/karaokemachine/src/display.rs`. On Android, `SDL_main` gives it a thread of its own.
- **Tizen** needs SDL on the host's UI thread, because SDL and DALi read one Wayland queue. That
  thread must keep the Ecore main loop turning, or Tizen closes the app after about twenty seconds,
  and no key, lifecycle or system event arrives.

| Approach | What changes | Verdict |
|---|---|---|
| **Nested loop:** the host calls the machine on its UI thread; each frame the machine waits in `ecore_main_loop_iterate_may_block(1)` until an Ecore animator ticks | a per-frame hook and a Tizen window path in `display.rs`; pacing by the animator, not the frame budget | **chosen**: M1 passed |
| Inverted loop: `run_with` re-cut into a struct whose `frame()` the host's animator calls | the largest scope in the workspace, re-cut into fields | not needed |
| SDL on a thread of its own, on its own Wayland connection | nothing in the loop | ruled out: a window of its own sticks on an old frame, and a subsurface must share the host's connection |

**Keys need no bridge.** SDL's own `wl_keyboard` shares the host's connection and receives the
remote's keys while the host's window has focus. Back arrives as `AC Back`, which the machine already
treats as back. Pushing Ecore keys into SDL as well delivered every key twice, so nothing is pushed.
The media keys are the exception: the set keeps them unless a window grabs them, so the machine
grabs them in TOPMOST mode, as flutter-tizen and OmegaTube do. On the set all nine grabs were
taken and the remote's Play/Pause paused a song, on 2026-11-24. Each key Ecore sees is logged at
debug.

**Focus is the one thing to manage.** A system overlay, such as the set's number pad or its menu,
takes keyboard focus. If SDL's window still declares a full opaque region, focus never comes back.
Clearing the opaque region on `ECORE_WL2_EVENT_FOCUS_OUT`, and setting it again a frame after
`FOCUS_IN`, lets the compositor return focus by itself, with no `ecore_wl2_window_activate`.

## Work breakdown

Everything is behind a `tizen` cargo feature rather than `target_os`, because Tizen reports itself
as ordinary Linux. **The one exception is how SDL is linked**: a feature cannot remove the
workspace's `build-from-source-static`, so the softfp triple `armv7-unknown-linux-gnueabi`, which no
other carrier uses, selects the patched SDL3 prebuilt in the image at `/opt/sdl3`.

| Piece | What it is | Where it lands | Size |
|---|---|---|---|
| Toolchain | a Docker image with the armel cross-compiler and Rust for `armv7-unknown-linux-gnueabi`, hardware float on; the machine built as a `cdylib` | `tools/port/machine/tizen/` | S |
| SDL patches | the two one-line guards, output scale 0 and refresh in hertz, applied to the source `sdl3` builds from; SDL3 and SDL3_ttf built static in the image | `sdl3-tizen.patch`, applied by the Dockerfile | S |
| Display path | the window as a custom-role subsurface of the host's window on its `wl_display`; transparent first frame, then opaque, and not opaque while focus is out; the per-frame Ecore pump, paced by an animator; never destroyed | `display.rs`, behind `tizen` | M–L |
| Keys | SDL's own keyboard, unchanged; Ecore's focus events watched for the opaque region; the media keys grabbed | the same, and a small FFI module | S |
| Audio output | a `km-audio` output over `audio_out_write` in place of cpal's stream, opened with `dlopen`; cpal and the ALSA mixer stay linked, so `libasound` stays needed | `km-audio`, behind `tizen` | M |
| Video | `ffmpeg-next` linked against ffmpeg 7.1 built for armel at build time only; at run time the set's own `libavcodec.so.61` answers | the Docker image | M |
| Paths and assets | the data folder and the package's `res/assets` handed down from C#, as on iOS; the bank, font and wallpapers read in place there, with nothing written out | `settings/paths.rs`, `tizen/entry.rs`, the package | S |
| .NET host | a NUI app that hands the native window and the folders to `km_tizen_run`; Back ends it | `ports/machine/tizen/` | M |
| Packaging and install | the `.tpk` built with the Tizen .NET workload and signed with its test author certificate; Apps2Samsung signs it again for the set it installs on | `tools/port/machine/tizen/package.sh` | M |

## Milestones

Each milestone is its own pull request. M0 and M1 run side by side, and M1 is the gate. M2 to M4 run
the real machine inside the probe app already on the set, loaded over its port, so nothing is
installed before M5.

| Milestone | Deliverable | Verified by | Size |
|---|---|---|---|
| M0 Toolchain | the machine builds as a `cdylib` for `armv7-unknown-linux-gnueabi`, video included | `readelf`: hardware float; every needed soname present on the set | S |
| M1 Nested loop | a C probe: an SDL frame loop on the UI thread pumping Ecore each frame, Back ending it | on the set: smooth for a minute, arrows and Back arrive, a clean exit. **Passed** | S |
| M2 The machine starts | `km_tizen_run`: the machine with its idle screen, pad, catalog and API | on the set: idle screen drawn, pad driven by the remote, the owner's page open on a phone. **Passed**: `tools/port/machine/tizen/load.sh` runs it in the probe app | M–L |
| M3 Sound | the `audio_io` output in `km-audio`; the bundled bank carried in the `.so`; `songs.sh` installs a synthetic test package | a MIDI song dialled on the pad and heard clean. **Passed**: the words in time, with no latency correction | M |
| M4 Every song kind | video through the set's ffmpeg, MP3+G, words in time; no new playback code, so the work is `songs.sh` adding a synthetic 1080p video song and an MP3+G song whose words light on a beep each second, the `.cdg` from km-cdg's `sync_song` example | a video song and an MP3+G song played, each flash and each word on its beep. **Passed** on 2026-11-24, in the probe app; the remote's Play/Pause paused a song too | S–M |
| M5 A real app | the .NET host, assets in `res/`, a signed `.tpk`: `package.sh` builds `KmMachine1.KaraokeMachine`, the API on 8177 as settings say | installed with Apps2Samsung and started from the app list, and the remote's Play/Pause pausing a song. **Passed** on 2026-11-24: installed with Apps2Samsung, started from the app list, every song kind played from the remote, Back closed it. The host draws its name before starting the machine, or the set keeps its launch spinner up | M |
| M6 Documentation | `docs/architecture/tizen.md`, `BUILDING.md`, `DEPLOYING.md`, the decision amended; this file deleted | review | S |

M1 passed, so the inverted loop is not built.

## Risks

| Risk | Why it matters | Fallback or answer |
|---|---|---|
| Nested Ecore iteration misbehaves: re-entrant callbacks, uneven frames, an exit inside the pump | it is the whole display design | measured at M1: no re-entrant callback, and Back's `Exit()` inside the pump ended the app cleanly |
| The full machine is heavier than the probes | catalog, API threads and fonts, in a 32-bit process | measured at M2; the 275 MB bank already fit beside a song |
| SDL patch authorship | SDL does not accept AI-written contributions | the guards stay a local patch here |
| A firmware update changes the set | the focus quirk, the ffmpeg version and the compositor are the set's | the probes in `local/` re-measure it |
| How long an install lasts | a weekly reinstall would make this a hobbyist carrier | measured later; installing at M5 restarts that clock |

**Open on the way:** whether Apps2Samsung installs an app this size, or `sdb` with a Tizen Studio
certificate is needed.

**Answered at M2:** Back on the idle screen ends the machine, and the machine then ends the app with
`ui_app_exit`. The set lets an app advertise itself over mDNS. A phone reaches the API on
`0.0.0.0`. The first start, writing the carried assets and an empty catalog, takes about two
seconds.
