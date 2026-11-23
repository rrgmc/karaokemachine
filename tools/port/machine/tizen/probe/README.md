# The nested-loop probe

M1 of [`docs/plans/tizen-port.md`](../../../../../docs/plans/tizen-port.md): a C probe that runs an
SDL frame loop on the host's UI thread and lets Ecore iterate every frame. It is the gate for the
nested-loop design. `pace=animator`, the default, blocks in `ecore_main_loop_iterate_may_block(1)`
until an Ecore animator marks a frame due. `pace=deadline` does one non-blocking
`ecore_main_loop_iterate()` and sleeps to fixed deadlines. On the set, `deadline` presented about a
third of its frames more than 20 ms apart.

Two more findings from the set shape the args. SDL's own keyboard on the shared display already
delivers the remote's keys, so `push=0`, the default, leaves Ecore's keys logged but not pushed.
Second, once the set's on-screen number pad closed, no key reached the application. So the probe
watches the host window's focus events. `refocus=1`, the default, clears SDL's opaque region and
calls `ecore_wl2_window_activate` every second until `FOCUS_IN` arrives. `refocus=stop` stops
presenting instead. `kb=1` binds a `wl_keyboard` of the probe's own and logs which surface each
keyboard enter and leave names. The window setup is the one measured in
[`docs/research/tizen.md`](../../../../../docs/research/tizen.md) §3.

| File | What it is |
|---|---|
| `nested_loop.c` | the probe. Its header comment lists the args, the endings and what is logged |
| `build.sh` | builds `nested-loop.so` in the Tizen image, linking the patched static SDL3 whole-archive |

```sh
tools/port/machine/tizen/probe/build.sh
```

The image defaults to `km-tizen` with SDL3 at `/opt/sdl3`. `KM_TIZEN_IMAGE` names another image, and
`SDL3_PREFIX` names a host folder holding `include/` and `lib/libSDL3.a`, which the script mounts. The
library goes to `local/tizen/probe/`, which git ignores, or to `KM_PROBE_OUT`.

The library exports `int km_probe_run(const char *args, void *ecore_win, int w, int h,
void (*log)(const char *))`. The prototype host loads it and calls it on its UI thread. The call
blocks until Back, the host quitting, or the `secs=` cap. It needs only `libwayland-client.so.0`,
`libm.so.6` and `libc.so.6`. Ecore, Eo and the Ecore Wayland library are opened with `dlopen`.
