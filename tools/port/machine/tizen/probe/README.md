# The nested-loop probe

M1 of [`docs/plans/tizen-port.md`](../../../../../docs/plans/tizen-port.md): a C probe that runs an
SDL frame loop on the host's UI thread and calls `ecore_main_loop_iterate()` once per frame. It is
the gate for the nested-loop design. The window setup is the one measured in
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
