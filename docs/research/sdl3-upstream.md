# Research: SDL3 fixes that could go upstream

**Research only.** The SDL3 changes the television ports carry or work around, and for each, what an
upstream fix would be. Kept current as ports find more. The ports' own notes are
[`tizen.md`](tizen.md) and [`webos.md`](webos.md) §13.

**SDL does not take AI-written contributions** (recorded in `sdl3-tizen.patch`). So nothing here is
sent as a patch from this repository. Each entry is written so a person can file it as an issue with
the evidence attached, or write the change themselves.

All entries are against SDL 3.4.14, the version `Cargo.lock` pins.

| # | Platform | What breaks | Carried here as | Upstream fix | Likely reception |
|---|---|---|---|---|---|
| 1 | Tizen | `wl_output.scale` of 0 is announced, and SDL divides by it | `sdl3-tizen.patch`: treat a scale below 1 as 1 | the same guard in `handle_wl_output_scale`. The protocol says the factor is a positive integer, so a guard costs nothing on a conforming compositor | good: a crash-class guard against a broken compositor, one line |
| 2 | Tizen | `wl_output.mode` reports the refresh rate in hertz, where the protocol says millihertz, so SDL reports 0.06 Hz | `sdl3-tizen.patch`: a refresh below 1000 is taken as hertz | the same heuristic, or ignoring an implausible rate | uncertain: it guesses at a compositor bug, and no real display runs below 1 Hz, so the guess is safe |
| 3 | webOS 5 | the Wayland driver fails to load when libwayland is older than 1.18, because `wl_proxy_set_tag` and `wl_proxy_get_tag` are required symbols | `sdl3-webos.patch`: both optional; without them every surface and output is taken as SDL's | the same, or tracking SDL's own surfaces another way, for example by their user data | uncertain: 1.18 dates from 2020 and may be a deliberate floor. The build-time `wayland-client>=1.18` check can stay; the issue is only the run-time requirement |
| 4 | webOS 5 | the remote's Back key arrives as evdev `KEY_PREVIOUS`, 412, which SDL's Linux table maps to `SDL_SCANCODE_UNKNOWN` | the machine maps `raw` 412 itself | none proposed. 412 means "previous" generically, and mapping it to `SDL_SCANCODE_AC_BACK` is right only for LG remotes | poor; keep it local |
| 5 | webOS 5 | xkbcommon logs "couldn't find a Compose file for locale C" at error level, once per seat | nothing; harmless | SDL could skip compose setup for the `C` locale, or log quietly | minor; probably not worth an issue |

## Not proposed

- **A `wl_shell` path for compositors without `xdg_wm_base`.** SDL2 had one; SDL3 removed it. A
  custom-role window (`SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN`) lets the program
  assign `wl_shell` and `wl_webos_shell` itself, which is what both television ports do. Asking SDL to
  carry a deprecated shell for that would be asking for something the API already makes unnecessary.
- **A patch hook in the Rust `sdl3-sys` crate.** The ports link a prebuilt SDL3 through
  `use-pkg-config` and `link-static` instead, chosen by target triple in `km-display` and
  `karaokemachine`.

## Evidence to attach

- #1 and #2: `tizen.md` §3. Unpatched, the process dies with `SIGFPE` in `handle_wl_output_done`,
  and the refresh reads 0.06 Hz.
- #3: the set's `libwayland-client.so.0` exports `wl_proxy_create_wrapper` and
  `wl_display_prepare_read_queue` but not `wl_proxy_set_tag`, `wl_proxy_get_tag` or
  `wl_proxy_marshal_flags`. With the patch SDL3 draws at 60 fps, `webos.md` §13. Unpatched, the same
  test fails at once with `SDL_Init failed: wayland not available`: both are `SDL_WAYLAND_SYM`
  rather than `SDL_WAYLAND_SYM_OPT` in `SDL_waylandsym.h`, so the loader refuses the whole driver.
- #4: the key log in `webos.md` §13.
