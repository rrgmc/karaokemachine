/*
 * km-sdl-test: P2 and P4 of docs/plans/webos-probe.md through SDL3, on a webOS 5 set.
 *
 *   km-sdl-test [secs]
 *
 * SDL3's Wayland driver needs xdg_wm_base for a window of its own, and the set's compositor offers
 * wl_shell and wl_webos_shell instead. So the window is made with a custom surface role, and this
 * program gives SDL's wl_surface the role the probe's draw mode proved: a wl_shell toplevel and a
 * wl_webos_shell_surface carrying the application id, asked to go fullscreen. SDL then draws into
 * it with its GLES2 renderer, and its keyboard reports the remote's keys.
 *
 * Everything is written to stderr, which the probe copies into its log. libwayland-client is opened
 * with dlopen, as SDL does: the image has its headers but no armel library to link.
 */

#include <SDL3/SDL.h>
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct wl_message {
    const char *name;
    const char *signature;
    const struct wl_interface **types;
};

struct wl_interface {
    const char *name;
    int version;
    int method_count;
    const struct wl_message *methods;
    int event_count;
    const struct wl_message *events;
};

typedef void *(*marshal_ctor_fn)(void *, uint32_t, const struct wl_interface *, ...);
typedef void *(*marshal_ctor_v_fn)(void *, uint32_t, const struct wl_interface *, uint32_t, ...);
typedef void (*marshal_fn)(void *, uint32_t, ...);
typedef int (*add_listener_fn)(void *, const void *, void *);
typedef int (*roundtrip_fn)(void *);

static marshal_ctor_fn marshal_ctor;
static marshal_ctor_v_fn marshal_ctor_v;
static marshal_fn marshal;
static add_listener_fn add_listener;
static roundtrip_fn roundtrip;

/* wl_webos_shell and wl_webos_shell_surface, from webos-shell.xml in webosbrew/SDL-webOS. */
static const struct wl_interface *nulls[2];
static const struct wl_interface webos_surface_interface;
static const struct wl_interface *get_shell_surface_types[2];
static const struct wl_message webos_shell_requests[] = {
    { "get_system_pip", "", NULL },
    { "get_shell_surface", "no", get_shell_surface_types },
};
static const struct wl_interface webos_shell_interface = {
    "wl_webos_shell", 1, 2, webos_shell_requests, 0, NULL,
};
static const struct wl_message webos_surface_requests[] = {
    { "set_location_hint", "u", nulls },
    { "set_state", "u", nulls },
    { "set_property", "ss", nulls },
    { "set_key_mask", "u", nulls },
};
static const struct wl_message webos_surface_events[] = {
    { "state_changed", "u", nulls },
    { "position_changed", "ii", nulls },
    { "close", "", nulls },
    { "exposed", "a", nulls },
    { "state_about_to_change", "u", nulls },
};
static const struct wl_interface webos_surface_interface = {
    "wl_webos_shell_surface", 1, 4, webos_surface_requests, 5, webos_surface_events,
};

static uint32_t shell_name, webos_shell_name;
static bool quit;

static void on_global(void *data, void *registry, uint32_t name, const char *iface, uint32_t version)
{
    if (strcmp(iface, "wl_shell") == 0) {
        shell_name = name;
    } else if (strcmp(iface, "wl_webos_shell") == 0) {
        webos_shell_name = name;
    }
}
static void on_global_remove(void *data, void *registry, uint32_t name) {}
static const void *registry_listener[] = { (void *)on_global, (void *)on_global_remove };

static void ss_ping(void *data, void *ss, uint32_t serial) { marshal(ss, 0, serial); }
static void ss_configure(void *data, void *ss, uint32_t edges, int32_t w, int32_t h)
{
    SDL_Log("wl_shell_surface configure %dx%d", w, h);
}
static void ss_popup_done(void *data, void *ss) {}
static const void *shell_surface_listener[] = { (void *)ss_ping, (void *)ss_configure, (void *)ss_popup_done };

static void ws_state(void *data, void *ws, uint32_t state) { SDL_Log("webos state_changed %u", state); }
static void ws_position(void *data, void *ws, int32_t x, int32_t y) { SDL_Log("webos position_changed %d,%d", x, y); }
static void ws_close(void *data, void *ws)
{
    SDL_Log("webos close");
    quit = true;
}
static void ws_exposed(void *data, void *ws, void *rects) { SDL_Log("webos exposed"); }
static void ws_about(void *data, void *ws, uint32_t state) { SDL_Log("webos state_about_to_change %u", state); }
static const void *webos_surface_listener[] = {
    (void *)ws_state, (void *)ws_position, (void *)ws_close, (void *)ws_exposed, (void *)ws_about,
};

/* Gives SDL's surface the webOS roles. */
static bool give_role(void *display, void *surface)
{
    void *lib = dlopen("libwayland-client.so.0", RTLD_NOW);
    if (!lib) {
        SDL_Log("dlopen libwayland-client: %s", dlerror());
        return false;
    }
    marshal_ctor = (marshal_ctor_fn)dlsym(lib, "wl_proxy_marshal_constructor");
    marshal_ctor_v = (marshal_ctor_v_fn)dlsym(lib, "wl_proxy_marshal_constructor_versioned");
    marshal = (marshal_fn)dlsym(lib, "wl_proxy_marshal");
    add_listener = (add_listener_fn)dlsym(lib, "wl_proxy_add_listener");
    roundtrip = (roundtrip_fn)dlsym(lib, "wl_display_roundtrip");
    const struct wl_interface *registry_iface = dlsym(lib, "wl_registry_interface");
    const struct wl_interface *surface_iface = dlsym(lib, "wl_surface_interface");
    const struct wl_interface *shell_iface = dlsym(lib, "wl_shell_interface");
    const struct wl_interface *shell_surface_iface = dlsym(lib, "wl_shell_surface_interface");
    if (!marshal_ctor || !marshal_ctor_v || !marshal || !add_listener || !roundtrip || !registry_iface ||
        !surface_iface || !shell_iface || !shell_surface_iface) {
        SDL_Log("a libwayland-client symbol is missing");
        return false;
    }
    get_shell_surface_types[0] = &webos_surface_interface;
    get_shell_surface_types[1] = surface_iface;

    void *registry = marshal_ctor(display, 1, registry_iface, NULL);
    add_listener(registry, registry_listener, NULL);
    roundtrip(display);
    if (!shell_name || !webos_shell_name) {
        SDL_Log("no wl_shell (%u) or wl_webos_shell (%u)", shell_name, webos_shell_name);
        return false;
    }
    void *shell = marshal_ctor_v(registry, 0, shell_iface, 1, shell_name, shell_iface->name, 1, NULL);
    void *webos = marshal_ctor_v(registry, 0, &webos_shell_interface, 1, webos_shell_name,
                                 webos_shell_interface.name, 1, NULL);

    void *ss = marshal_ctor(shell, 0, shell_surface_iface, NULL, surface);
    add_listener(ss, shell_surface_listener, NULL);
    marshal(ss, 9, "km-sdl-test"); /* set_class */
    marshal(ss, 3);                /* set_toplevel */

    void *ws = marshal_ctor(webos, 1, &webos_surface_interface, NULL, surface);
    add_listener(ws, webos_surface_listener, NULL);
    const char *appid = SDL_getenv("APPID");
    marshal(ws, 2, "appId", appid ? appid : "");
    marshal(ws, 2, "_WEBOS_ACCESS_POLICY_KEYS_BACK", "true");
    marshal(ws, 1, 3u); /* set_state fullscreen */
    roundtrip(display);
    return true;
}

int main(int argc, char **argv)
{
    int secs = argc > 1 ? atoi(argv[1]) : 60;
    SDL_SetHint(SDL_HINT_VIDEO_DRIVER, "wayland");
    if (!SDL_Init(SDL_INIT_VIDEO | SDL_INIT_EVENTS)) {
        SDL_Log("SDL_Init failed: %s", SDL_GetError());
        return 1;
    }
    SDL_Log("SDL %d.%d.%d, video driver %s", SDL_MAJOR_VERSION, SDL_MINOR_VERSION, SDL_MICRO_VERSION,
            SDL_GetCurrentVideoDriver());

    SDL_PropertiesID props = SDL_CreateProperties();
    SDL_SetStringProperty(props, SDL_PROP_WINDOW_CREATE_TITLE_STRING, "km-sdl-test");
    SDL_SetNumberProperty(props, SDL_PROP_WINDOW_CREATE_WIDTH_NUMBER, 1920);
    SDL_SetNumberProperty(props, SDL_PROP_WINDOW_CREATE_HEIGHT_NUMBER, 1080);
    SDL_SetBooleanProperty(props, SDL_PROP_WINDOW_CREATE_OPENGL_BOOLEAN, true);
    SDL_SetBooleanProperty(props, SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN, true);
    SDL_Window *win = SDL_CreateWindowWithProperties(props);
    if (!win) {
        SDL_Log("SDL_CreateWindow failed: %s", SDL_GetError());
        return 1;
    }
    void *display = SDL_GetPointerProperty(SDL_GetGlobalProperties(), SDL_PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER, NULL);
    void *surface = SDL_GetPointerProperty(SDL_GetWindowProperties(win), SDL_PROP_WINDOW_WAYLAND_SURFACE_POINTER, NULL);
    SDL_Log("window made: wl_display %p, wl_surface %p", display, surface);
    if (!display || !surface || !give_role(display, surface)) {
        return 1;
    }
    SDL_Log("surface roles given: wl_shell toplevel + wl_webos_shell_surface, fullscreen asked");

    SDL_Renderer *r = SDL_CreateRenderer(win, "opengles2");
    if (!r) {
        SDL_Log("SDL_CreateRenderer failed: %s", SDL_GetError());
        return 1;
    }
    SDL_SetRenderVSync(r, 1);
    int w = 0, h = 0;
    SDL_GetCurrentRenderOutputSize(r, &w, &h);
    SDL_Log("renderer %s, output %dx%d", SDL_GetRendererName(r), w, h);

    Uint64 start = SDL_GetTicksNS(), last = start, window_start = start, worst = 0;
    unsigned frames = 0, gaps = 0, total = 0, total_gaps = 0;
    while (!quit && SDL_GetTicksNS() - start < (Uint64)secs * SDL_NS_PER_SECOND) {
        SDL_Event e;
        while (SDL_PollEvent(&e)) {
            if (e.type == SDL_EVENT_KEY_DOWN || e.type == SDL_EVENT_KEY_UP) {
                SDL_Log("KEY %s scancode %d (%s) keycode %#x (%s) raw %u", e.type == SDL_EVENT_KEY_DOWN ? "down" : "up",
                        e.key.scancode, SDL_GetScancodeName(e.key.scancode), e.key.key, SDL_GetKeyName(e.key.key),
                        e.key.raw);
            } else if (e.type == SDL_EVENT_QUIT || e.type == SDL_EVENT_WINDOW_CLOSE_REQUESTED) {
                SDL_Log("quit event %#x", e.type);
                quit = true;
            } else if (e.type >= SDL_EVENT_WINDOW_FIRST && e.type <= SDL_EVENT_WINDOW_LAST) {
                SDL_Log("window event %#x", e.type);
            }
        }
        float t = (float)(SDL_GetTicksNS() - start) / SDL_NS_PER_SECOND;
        SDL_SetRenderDrawColor(r, 20, 40, 80, 255);
        SDL_RenderClear(r);
        SDL_SetRenderDrawColor(r, 60, 220, 120, 255);
        SDL_FRect bar = { (float)((int)(t * 480.0f) % (w - 64)), 0, 64, (float)h };
        SDL_RenderFillRect(r, &bar);
        SDL_RenderPresent(r);

        Uint64 now = SDL_GetTicksNS(), gap = now - last;
        last = now;
        frames++, total++;
        if (gap > 20 * SDL_NS_PER_MS) {
            gaps++, total_gaps++;
        }
        if (gap > worst) {
            worst = gap;
        }
        if (now - window_start >= 5 * SDL_NS_PER_SECOND) {
            SDL_Log("frames: %.1f fps, %u gaps over 20 ms, worst %.1f ms",
                    frames / ((double)(now - window_start) / SDL_NS_PER_SECOND), gaps, worst / 1e6);
            window_start = now, frames = 0, gaps = 0, worst = 0;
        }
    }
    SDL_Log("sdl done: %u frames in %.1f s, %u over 20 ms", total,
            (double)(SDL_GetTicksNS() - start) / SDL_NS_PER_SECOND, total_gaps);
    SDL_DestroyRenderer(r);
    SDL_DestroyWindow(win);
    SDL_Quit();
    return 0;
}
