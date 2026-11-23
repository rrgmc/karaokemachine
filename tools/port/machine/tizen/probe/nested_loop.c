/* M1 of the Tizen port: the nested loop.
 *
 * A probe the prototype host loads over its port and calls on its UI thread:
 *
 *     int km_probe_run(const char *args, void *ecore_win, int w, int h, void (*log)(const char *));
 *
 * It does not return until the run ends. Each frame it lets Ecore do its work by calling Ecore's
 * iterate, which under the default `pace=animator` blocks until an Ecore animator says a frame is
 * due; then it drains SDL's queue, draws and presents. Under `pace=deadline` it does one
 * non-blocking iterate and waits out the rest of the frame budget itself. That is the shape `run_with` in the machine's `display.rs` would take on the set
 * (docs/plans/tizen-port.md, "The design problem: whose loop runs").
 *
 * The window is the measured one from docs/research/tizen.md section 3: SDL on the host's
 * `wl_display`, a custom-role surface made a desynchronised subsurface above the host's surface,
 * created transparent and given an opaque region after its first frame, vsync off, never destroyed
 * and SDL never quit. The remote's keys arrive at Ecore on the host's window, and on the set they
 * also reach SDL through its own wl_keyboard on the shared display. Ecore's are logged, and pushed
 * into SDL's queue only with `push=1`, tagged with a keyboard id of their own.
 *
 * Keyboard focus: on the set, the 123 button (`XF86More`) opens the on-screen number pad, and once
 * it closed no key reached the application again. Activating the host's window brought the keys
 * back. So the probe watches Ecore_Wl2's focus, activate and visibility events and, by `refocus=`,
 * tries to win focus back while SDL keeps its window.
 *
 * Args, `key=value` separated by commas, every one optional:
 *
 *   secs=120       safety cap in seconds. The run ends on its own after this, whatever happens.
 *   push=0         0: Ecore keys are logged but not pushed; the loop relies on SDL's own keyboard.
 *                  1: Ecore keys are also pushed into SDL's queue (the first build's behaviour; on
 *                  the set every key then arrived twice).
 *   refocus=1      what happens when the host loses keyboard focus. Only ECORE_WL2_EVENT_FOCUS_OUT
 *                  and _FOCUS_IN move the focus state; WINDOW_ACTIVATE, _DEACTIVATE and
 *                  VISIBILITY_CHANGE are logged only (on the set, ACTIVATE arrived 20 ms after the
 *                  number pad took focus, and focus never came back):
 *                    1     SDL's opaque region is cleared (on the next present) and drawing goes
 *                          on; ecore_wl2_window_activate(host) after `refocus_delay` frames, then
 *                          every 60 frames, uncapped, until FOCUS_IN. On focus in, the full opaque
 *                          region is set again one frame later
 *                    stop  presenting stops while focus is out; activate is called at once;
 *                          presenting resumes on focus in, or after 2 s whatever happens, and then
 *                          the same 60-frame retries run
 *                    0     nothing; the events are still logged
 *                  With 1 or stop, VISIBILITY_CHANGE fully_obscured=0 (back from the set's menu)
 *                  while focus is out also calls activate on the next frame.
 *   refocus_delay=30  frames between losing focus and the first activate.
 *   kb=0           1: also bind a wl_keyboard of this probe's own through a registry on a private
 *                  queue, dispatched each frame, and log every enter and leave with the surface it
 *                  names (the host's or SDL's); its keys are only counted.
 *   back=done      what Back does at Ecore. Back read out of SDL's queue ends the loop whichever
 *                  route it took: pushed, or SDL's own keyboard (keycode AC_BACK or Escape,
 *                  scancode AC_BACK or Escape).
 *                    done  the Ecore handler consumes Back (ECORE_CALLBACK_DONE); DALi never sees
 *                          it; the loop ends when Back is read out of SDL's queue
 *                    pass  the handler passes Back on (ECORE_CALLBACK_PASS_ON), so NUI's Exit()
 *                          also runs; the loop still ends on Back from SDL's queue, the same frame
 *                    host  passes Back on and does NOT end the loop on it: the loop runs until it
 *                          sees the host quitting inside the pump, or the cap
 *   order=prepend  where the key handlers go in Ecore's list: prepend (ahead of DALi's, so `done`
 *                  can withhold Back) or append (after DALi's, as the earlier probes did). Prepend
 *                  falls back to append when `ecore_event_handler_prepend` is absent, and says so.
 *   backkey=acback what Back becomes in SDL: acback (SDLK_AC_BACK, SDL_SCANCODE_AC_BACK) or escape.
 *   opaque=1       set the opaque region after this many presented frames; 0 never sets it.
 *   pace=animator  what paces the frames:
 *                    animator  an Ecore animator whose callback only sets a tick flag; the loop
 *                              calls ecore_main_loop_iterate_may_block(1) until the tick arrives,
 *                              then drains SDL, draws and presents. The machine owns the loop and
 *                              Ecore paces it, as the animator-driven probe that measured smooth
 *                              was paced. The animator's frametime is set to 1/fps. Without
 *                              ..._may_block it falls back to iterate plus a 1 ms sleep, and says so
 *                    deadline  one non-blocking ecore_main_loop_iterate() a frame, then
 *                              SDL_DelayPrecise to fixed 1/fps deadlines (the first run on the set:
 *                              about a third of presents were more than 20 ms apart)
 *   fps=60         the frame budget, 1/fps seconds.
 *   eoquit=1       also listen for the Efl loop's "quit" event through libeo; 0 skips that.
 *
 * What ends a run, logged as `ending:` and returned:
 *    0  Back, read out of SDL's queue (pushed or from SDL's own keyboard)
 *    1  the safety cap
 *    2  the host quitting inside the pump (ECORE_EVENT_SIGNAL_EXIT, or the Efl loop's quit event)
 *    3  a Wayland protocol error, or a present that failed
 *   -1  km_probe_run was re-entered while a run was active; the inner call returns at once
 *   10+ setup failed before the loop started
 *
 * Everything goes to the host's `log`, and to `nested-loop.log` beside this library with an fsync
 * per line, so it survives the process ending inside the pump. Every 5 s: frames presented,
 * presents more than 20 ms after the one before, the slowest present interval, the time inside
 * Ecore's iterate calls per frame (mean and max, which under `pace=animator` is mostly time
 * blocked waiting for the tick), iterate calls per frame, the time drawing and inside
 * SDL_RenderPresent (mean and max), under `pace=deadline` the worst overshoot of the delay, and how
 * many 100 ms Ecore timer ticks and animator ticks arrived. Every
 * key is logged as Ecore delivered it and again as SDL's queue returned it, and every focus,
 * activate and visibility event with its time. Any of this probe's Ecore callbacks running outside
 * the iterate call is logged as re-entrancy.
 *
 * Ecore is reached through dlopen/dlsym only, as the earlier probes did, so nothing links against
 * EFL. `Ecore_Event_Key` below is transcribed from EFL's public header
 * src/lib/ecore_input/Ecore_Input.h (`struct _Ecore_Event_Key`, EFL 1.19 and later: `keycode` and
 * `data` since 1.10, `dev` since 1.19). Only the leading fields up to `keycode` are read, and those
 * have been stable since 1.10. Ecore_Window is `uintptr_t` on Wayland builds. */
#define _GNU_SOURCE
#include <SDL3/SDL.h>
#include <wayland-client.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <math.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef void (*log_fn)(const char *);

/* ---- Ecore and Eo, by hand ---- */

typedef unsigned char ebool;   /* Eina_Bool */
#define ECORE_CALLBACK_DONE 0     /* EINA_FALSE: no later handler sees the event */
#define ECORE_CALLBACK_PASS_ON 1  /* EINA_TRUE */
#define ECORE_CALLBACK_RENEW 1

typedef struct {
    const char *keyname;     /* the key name, e.g. "XF86Back" */
    const char *key;         /* the key symbol */
    const char *string;
    const char *compose;
    uintptr_t window;        /* Ecore_Window */
    uintptr_t root_window;
    uintptr_t event_window;
    unsigned int timestamp;
    unsigned int modifiers;
    int same_screen;
    unsigned int keycode;    /* @since 1.10 */
    void *data;              /* @since 1.10 */
    void *dev;               /* Eo *, @since 1.19 */
} Ecore_Event_Key_;

typedef ebool (*ecore_event_cb)(void *data, int type, void *event);
typedef ebool (*ecore_task_cb)(void *data);
typedef struct { void *object; const void *desc; void *info; } Efl_Event_;
typedef void (*efl_event_cb)(void *data, const Efl_Event_ *ev);

static void (*ecore_iterate)(void);
static int (*ecore_iterate_may_block)(int);
static void *(*ecore_animator_add_)(ecore_task_cb, const void *);
static void *(*ecore_animator_del_)(void *);
static void (*ecore_animator_frametime_set_)(double);
static int (*ecore_animator_source_get_)(void);
static int (*ecore_nested_get)(void);
static void *(*ecore_handler_add)(int, ecore_event_cb, const void *);
static void *(*ecore_handler_prepend)(int, ecore_event_cb, const void *);
static void *(*ecore_handler_del)(void *);
static void *(*ecore_timer_add_)(double, ecore_task_cb, const void *);
static void *(*ecore_timer_del_)(void *);
static ebool (*eo_cb_add)(void *, const void *, short, efl_event_cb, const void *);
static ebool (*eo_cb_del)(void *, const void *, efl_event_cb, const void *);

/* ---- logging ---- */

static log_fn LOG;
static int log_file = -1;

static void logf_(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void logf_(const char *fmt, ...)
{
    char buf[720], line[780];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (log_file >= 0) {
        struct timespec t;
        clock_gettime(CLOCK_REALTIME, &t);
        struct tm tm;
        localtime_r(&t.tv_sec, &tm);
        int n = snprintf(line, sizeof line, "%02d:%02d:%02d.%03ld %s\n", tm.tm_hour, tm.tm_min, tm.tm_sec,
                         t.tv_nsec / 1000000, buf);
        if (n > (int)sizeof line - 1) n = (int)sizeof line - 1;
        if (write(log_file, line, (size_t)n) < 0) { /* nothing to be done */ }
        fsync(log_file);
    }
    if (LOG) LOG(buf);
}

static void open_log_file(void)
{
    if (log_file >= 0) return;
    Dl_info info;
    if (!dladdr((void *)open_log_file, &info) || !info.dli_fname) return;
    char path[512];
    snprintf(path, sizeof path, "%s", info.dli_fname);
    char *slash = strrchr(path, '/');
    if (!slash) return;
    snprintf(slash + 1, sizeof path - (size_t)(slash + 1 - path), "nested-loop.log");
    log_file = open(path, O_WRONLY | O_CREAT | O_APPEND, 0644);
    if (log_file >= 0 && lseek(log_file, 0, SEEK_END) > 256 * 1024) {
        if (ftruncate(log_file, 0) < 0) { /* keep appending */ }
    }
}

static void sdl_log(void *u, int cat, SDL_LogPriority p, const char *msg)
{
    (void)u;
    if (p >= SDL_LOG_PRIORITY_WARN) logf_("SDL[%d/%d] %s", cat, (int)p, msg);
}

/* ---- the run's state ---- */

enum back_mode { BACK_DONE, BACK_PASS, BACK_HOST };
enum ending { END_NONE = -2, END_REENTERED = -1, END_BACK = 0, END_CAP = 1, END_HOST_QUIT = 2, END_WAYLAND = 3 };
static const char *ending_name(int e)
{
    switch (e) {
    case END_BACK: return "Back, read out of SDL's queue";
    case END_CAP: return "the safety cap";
    case END_HOST_QUIT: return "the host quitting inside the pump";
    case END_WAYLAND: return "a Wayland error or a failed present";
    default: return "?";
    }
}

static struct {
    int secs, fps, opaque_after, eoquit, prepend;
    enum back_mode back;
    int back_escape;
    int pace_deadline;
    int push;
    int refocus;          /* REFOCUS_* */
    int refocus_delay;    /* frames */
    int kb;               /* bind a wl_keyboard of our own */
} opt;

enum { REFOCUS_OFF, REFOCUS_OPAQUE, REFOCUS_STOP };

/* Ecore_Wl2 window events, resolved by name as probe_back.c's watch() does. */
enum { FE_FOCUS_IN, FE_FOCUS_OUT, FE_ACTIVATE, FE_DEACTIVATE, FE_VISIBILITY, FE_COUNT };
static const char *const focus_event_names[FE_COUNT] = {
    "ECORE_WL2_EVENT_FOCUS_IN", "ECORE_WL2_EVENT_FOCUS_OUT", "ECORE_WL2_EVENT_WINDOW_ACTIVATE",
    "ECORE_WL2_EVENT_WINDOW_DEACTIVATE", "ECORE_WL2_EVENT_WINDOW_VISIBILITY_CHANGE",
};
/* Upstream EFL's Ecore_Wl2.h: struct _Ecore_Wl2_Event_Window_Visibility_Change { unsigned int win;
 * int fully_obscured; }. Only read for the log. */
typedef struct { unsigned int win; int fully_obscured; } Ecore_Wl2_Visibility_;

static void (*wl2_window_activate)(void *);
static int (*wl2_window_activated_get)(void *);
static void *host_win;
static int focus_in = 1;          /* assumed in when the run starts */
static int focus_pending;         /* +1 in, -1 out, set by the handlers, read by the frame loop */
static int visible_again;         /* VISIBILITY_CHANGE fully_obscured=0 seen, read by the frame loop */
static Uint64 last_activate_ns;
static unsigned refocus_tries, refocus_total, focus_events;

static int running;            /* inside km_probe_run, this copy of the library */
static int in_iterate;         /* inside ecore_main_loop_iterate() */
static int in_callback;        /* depth of this probe's own Ecore callbacks */
static int ending = END_NONE;
static const char *quit_source;
static unsigned long long frame;
static Uint64 run_start_ns;

static SDL_Window *win;
static SDL_Renderer *ren;
static struct wl_display *dpy;
static struct wl_surface *parent, *sdl_surf;
static struct wl_subsurface *sub;
static struct wl_compositor *g_comp;
static struct wl_subcompositor *g_subcomp;

#define PUSHED_KEYBOARD_ID 0x4B4D0001u  /* `which` on the key events this probe pushes */

/* key state, for repeat */
static unsigned char key_down[SDL_SCANCODE_COUNT];
static char last_key[64] = "none yet";
static unsigned keys_from_ecore, keys_pushed, keys_from_sdl, keys_from_sdl_own;
static unsigned reentrant_callbacks;
static unsigned timer_ticks;
static unsigned anim_ticks;       /* every animator callback */
static unsigned anim_coalesced;   /* ticks that arrived while one was still pending */
static int anim_tick;             /* set by the animator, cleared by the frame loop */

static double secs_since_start(void) { return (SDL_GetTicksNS() - run_start_ns) / 1e9; }

/* Every callback of this probe's that Ecore calls goes through these two. */
static void callback_enter(const char *what)
{
    in_callback++;
    if (running && !in_iterate) {
        reentrant_callbacks++;
        logf_("RE-ENTRANCY: %s ran while km_probe_run was active but outside ecore_main_loop_iterate "
              "(frame %llu)", what, frame);
    }
    if (in_callback > 1)
        logf_("RE-ENTRANCY: %s ran while another of this probe's callbacks was on the stack (depth %d)",
              what, in_callback);
}
static void callback_leave(void) { in_callback--; }

/* ---- keys: Ecore to SDL ---- */

static int map_key(const char *name, SDL_Scancode *sc, SDL_Keycode *kc, int *is_back)
{
    static const struct { const char *name; SDL_Scancode sc; SDL_Keycode kc; } table[] = {
        { "Up", SDL_SCANCODE_UP, SDLK_UP },          { "Down", SDL_SCANCODE_DOWN, SDLK_DOWN },
        { "Left", SDL_SCANCODE_LEFT, SDLK_LEFT },    { "Right", SDL_SCANCODE_RIGHT, SDLK_RIGHT },
        { "Return", SDL_SCANCODE_RETURN, SDLK_RETURN }, { "KP_Enter", SDL_SCANCODE_RETURN, SDLK_RETURN },
        { "0", SDL_SCANCODE_0, SDLK_0 }, { "1", SDL_SCANCODE_1, SDLK_1 }, { "2", SDL_SCANCODE_2, SDLK_2 },
        { "3", SDL_SCANCODE_3, SDLK_3 }, { "4", SDL_SCANCODE_4, SDLK_4 }, { "5", SDL_SCANCODE_5, SDLK_5 },
        { "6", SDL_SCANCODE_6, SDLK_6 }, { "7", SDL_SCANCODE_7, SDLK_7 }, { "8", SDL_SCANCODE_8, SDLK_8 },
        { "9", SDL_SCANCODE_9, SDLK_9 },
        { "KP_0", SDL_SCANCODE_0, SDLK_0 }, { "KP_1", SDL_SCANCODE_1, SDLK_1 }, { "KP_2", SDL_SCANCODE_2, SDLK_2 },
        { "KP_3", SDL_SCANCODE_3, SDLK_3 }, { "KP_4", SDL_SCANCODE_4, SDLK_4 }, { "KP_5", SDL_SCANCODE_5, SDLK_5 },
        { "KP_6", SDL_SCANCODE_6, SDLK_6 }, { "KP_7", SDL_SCANCODE_7, SDLK_7 }, { "KP_8", SDL_SCANCODE_8, SDLK_8 },
        { "KP_9", SDL_SCANCODE_9, SDLK_9 },
    };
    *is_back = 0;
    if (!name) return 0;
    if (strcmp(name, "XF86Back") == 0 || strcmp(name, "Escape") == 0) {
        *is_back = 1;
        if (opt.back_escape) { *sc = SDL_SCANCODE_ESCAPE; *kc = SDLK_ESCAPE; }
        else { *sc = SDL_SCANCODE_AC_BACK; *kc = SDLK_AC_BACK; }
        return 1;
    }
    for (unsigned i = 0; i < sizeof table / sizeof table[0]; i++)
        if (strcmp(name, table[i].name) == 0) { *sc = table[i].sc; *kc = table[i].kc; return 1; }
    return 0;
}

static ebool on_key(void *data, int type, void *event)
{
    int down = data != NULL;   /* the down handler is registered with a non-null data pointer */
    (void)type;
    callback_enter(down ? "the key-down handler" : "the key-up handler");
    const Ecore_Event_Key_ *k = event;
    const char *name = k && k->keyname ? k->keyname : "(null)";
    keys_from_ecore++;
    SDL_Scancode sc;
    SDL_Keycode kc;
    int is_back;
    ebool ret = ECORE_CALLBACK_PASS_ON;
    if (!map_key(k ? k->keyname : NULL, &sc, &kc, &is_back)) {
        logf_("%.2f s Ecore key %s %s: unmapped (key=%s keycode=%u), not pushed, passed on", secs_since_start(),
              down ? "down" : "up", name, k && k->key ? k->key : "-", k ? k->keycode : 0);
        callback_leave();
        return ECORE_CALLBACK_PASS_ON;
    }
    int repeat = down && key_down[sc];
    key_down[sc] = (unsigned char)down;
    SDL_Event ev;
    SDL_zero(ev);
    ev.type = down ? SDL_EVENT_KEY_DOWN : SDL_EVENT_KEY_UP;
    ev.key.timestamp = SDL_GetTicksNS();
    ev.key.windowID = SDL_GetWindowID(win);
    ev.key.which = PUSHED_KEYBOARD_ID;
    ev.key.scancode = sc;
    ev.key.key = kc;
    ev.key.mod = SDL_KMOD_NONE;
    ev.key.raw = (Uint16)(k ? k->keycode : 0);
    ev.key.down = down;
    ev.key.repeat = repeat;
    bool pushed = opt.push && SDL_PushEvent(&ev);
    if (pushed) keys_pushed++;
    if (is_back) ret = opt.back == BACK_DONE ? ECORE_CALLBACK_DONE : ECORE_CALLBACK_PASS_ON;
    logf_("%.2f s Ecore key %s %s (keycode %u)%s -> SDL %s/%s %s; handler returns %s", secs_since_start(),
          down ? "down" : "up", name, k ? k->keycode : 0, repeat ? " repeat" : "",
          SDL_GetScancodeName(sc), SDL_GetKeyName(kc),
          !opt.push ? "not pushed (push=0)" : pushed ? "pushed" : "PUSH FAILED",
          ret == ECORE_CALLBACK_DONE ? "DONE (DALi does not see it)" : "PASS_ON");
    callback_leave();
    return ret;
}

/* ---- the host window's focus ---- */

/* Logs every event and records the newest focus direction; the frame loop acts on it, outside Ecore. */
static ebool on_focus_event(void *data, int type, void *event)
{
    int which = (int)(intptr_t)data;
    (void)type;
    callback_enter("a focus event handler");
    focus_events++;
    double since_activate = last_activate_ns ? (SDL_GetTicksNS() - last_activate_ns) / 1e6 : -1;
    const char *name = which >= 0 && which < FE_COUNT ? focus_event_names[which] : "?";
    int activated = wl2_window_activated_get && host_win ? wl2_window_activated_get(host_win) : -1;
    /* Only FOCUS_IN and FOCUS_OUT move the focus state. On the set, WINDOW_ACTIVATE arrived 20 ms
     * after the number pad took focus, and focus never came back, so ACTIVATE and DEACTIVATE are
     * logged and nothing more. */
    switch (which) {
    case FE_FOCUS_IN:
        focus_pending = 1;
        logf_("%.2f s %s (host activated %d; %.0f ms after this probe's last activate)", secs_since_start(),
              name, activated, since_activate);
        break;
    case FE_FOCUS_OUT:
        focus_pending = -1;
        logf_("%.2f s %s (host activated %d)", secs_since_start(), name, activated);
        break;
    case FE_ACTIVATE:
    case FE_DEACTIVATE:
        logf_("%.2f s %s (host activated %d; %.0f ms after this probe's last activate): logged only",
              secs_since_start(), name, activated, since_activate);
        break;
    case FE_VISIBILITY: {
        const Ecore_Wl2_Visibility_ *v = event;
        logf_("%.2f s %s fully_obscured=%d (host activated %d)", secs_since_start(), name,
              v ? v->fully_obscured : -1, activated);
        if (v && v->fully_obscured == 0) visible_again = 1;
        break;
    }
    default:
        logf_("%.2f s focus event %d", secs_since_start(), which);
    }
    callback_leave();
    return ECORE_CALLBACK_PASS_ON;
}

static void activate_host(const char *why)
{
    if (!wl2_window_activate) { logf_("%.2f s activate wanted (%s) but ecore_wl2_window_activate is absent", secs_since_start(), why); return; }
    int before = wl2_window_activated_get ? wl2_window_activated_get(host_win) : -1;
    last_activate_ns = SDL_GetTicksNS();
    wl2_window_activate(host_win);
    refocus_total++;
    int after = wl2_window_activated_get ? wl2_window_activated_get(host_win) : -1;
    logf_("%.2f s ecore_wl2_window_activate(host): %s; activated_get %d before, %d after; frame %llu",
          secs_since_start(), why, before, after, frame);
}

/* ---- kb=1: a wl_keyboard of this probe's own, on a private queue, as probe_back.c's own_keyboard() ---- */

static struct wl_event_queue *kb_queue;
static struct wl_display *kb_wrapper;
static struct wl_registry *kb_registry;
static struct wl_seat *kb_seat;
static struct wl_keyboard *kb_keyboard;
static unsigned kb_keys, kb_enters, kb_leaves;

static const char *surface_name(struct wl_surface *s)
{
    static char b[48];
    if (!s) return "null";
    if (s == parent) return "the host's surface";
    if (s == sdl_surf) return "SDL's surface";
    snprintf(b, sizeof b, "surface id %u", wl_proxy_get_id((struct wl_proxy *)s));
    return b;
}
static void kb_keymap(void *d, struct wl_keyboard *k, uint32_t f, int32_t fd, uint32_t sz)
{ (void)d; (void)k; (void)f; (void)sz; close(fd); }
static void kb_enter(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s, struct wl_array *keys)
{
    (void)d; (void)k; (void)serial; (void)keys;
    kb_enters++;
    logf_("%.2f s own wl_keyboard ENTER: %s", secs_since_start(), surface_name(s));
}
static void kb_leave(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s)
{
    (void)d; (void)k; (void)serial;
    kb_leaves++;
    logf_("%.2f s own wl_keyboard LEAVE: %s", secs_since_start(), surface_name(s));
}
static void kb_key(void *d, struct wl_keyboard *k, uint32_t serial, uint32_t time, uint32_t key, uint32_t state)
{ (void)d; (void)k; (void)serial; (void)time; (void)key; (void)state; kb_keys++; }
static void kb_mods(void *d, struct wl_keyboard *k, uint32_t s, uint32_t a, uint32_t b, uint32_t c, uint32_t g)
{ (void)d; (void)k; (void)s; (void)a; (void)b; (void)c; (void)g; }
static void kb_repeat(void *d, struct wl_keyboard *k, int32_t r, int32_t dl) { (void)d; (void)k; (void)r; (void)dl; }
static const struct wl_keyboard_listener kb_listener = { kb_keymap, kb_enter, kb_leave, kb_key, kb_mods, kb_repeat };

static void kb_reg_global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t v)
{
    (void)d;
    if (strcmp(iface, "wl_seat") == 0 && !kb_seat) kb_seat = wl_registry_bind(r, name, &wl_seat_interface, v < 5 ? v : 5);
}
static void kb_reg_remove(void *d, struct wl_registry *r, uint32_t name) { (void)d; (void)r; (void)name; }
static const struct wl_registry_listener kb_reg_listener = { kb_reg_global, kb_reg_remove };

static void own_keyboard_start(void)
{
    kb_keys = kb_enters = kb_leaves = 0;
    kb_queue = wl_display_create_queue(dpy);
    kb_wrapper = wl_proxy_create_wrapper(dpy);
    wl_proxy_set_queue((struct wl_proxy *)kb_wrapper, kb_queue);
    kb_registry = wl_display_get_registry(kb_wrapper);
    wl_registry_add_listener(kb_registry, &kb_reg_listener, NULL);
    wl_display_roundtrip_queue(dpy, kb_queue);
    if (!kb_seat) { logf_("kb=1: no wl_seat"); return; }
    kb_keyboard = wl_seat_get_keyboard(kb_seat);
    wl_keyboard_add_listener(kb_keyboard, &kb_listener, NULL);
    wl_display_roundtrip_queue(dpy, kb_queue);
    logf_("kb=1: own wl_keyboard bound on a private queue (seat v%u); host surface id %u, SDL surface id %u",
          wl_proxy_get_version((struct wl_proxy *)kb_seat), wl_proxy_get_id((struct wl_proxy *)parent),
          wl_proxy_get_id((struct wl_proxy *)sdl_surf));
}

static void own_keyboard_stop(void)
{
    if (!kb_queue) return;
    wl_display_dispatch_queue_pending(dpy, kb_queue);
    logf_("kb=1: own wl_keyboard saw %u enter, %u leave, %u key events", kb_enters, kb_leaves, kb_keys);
    if (kb_keyboard) {
        if (wl_proxy_get_version((struct wl_proxy *)kb_keyboard) >= 3) wl_keyboard_release(kb_keyboard);
        else wl_keyboard_destroy(kb_keyboard);
    }
    if (kb_seat) wl_seat_destroy(kb_seat);
    wl_registry_destroy(kb_registry);
    wl_proxy_wrapper_destroy(kb_wrapper);
    wl_display_flush(dpy);
    wl_event_queue_destroy(kb_queue);
    kb_keyboard = NULL; kb_seat = NULL; kb_registry = NULL; kb_wrapper = NULL; kb_queue = NULL;
}

/* ---- the host quitting inside the pump ---- */

static void host_quit(const char *source)
{
    if (!quit_source) quit_source = source;
    logf_("%.2f s host quit seen: %s (inside iterate: %s, frame %llu)", secs_since_start(), source,
          in_iterate ? "yes" : "no", frame);
}

static ebool on_signal_exit(void *data, int type, void *event)
{
    (void)data; (void)type; (void)event;
    callback_enter("the ECORE_EVENT_SIGNAL_EXIT handler");
    host_quit("ECORE_EVENT_SIGNAL_EXIT");
    callback_leave();
    return ECORE_CALLBACK_PASS_ON;
}

static void on_loop_quit(void *data, const Efl_Event_ *ev)
{
    (void)data; (void)ev;
    callback_enter("the Efl loop quit callback");
    host_quit("the Efl loop's quit event");
    callback_leave();
}

static ebool on_timer(void *data)
{
    (void)data;
    callback_enter("the 100 ms Ecore timer");
    timer_ticks++;
    callback_leave();
    return ECORE_CALLBACK_RENEW;
}

/* The animator only says a frame is due. Drawing happens in the frame loop, outside Ecore. */
static ebool on_animator(void *data)
{
    (void)data;
    callback_enter("the animator");
    anim_ticks++;
    if (anim_tick) anim_coalesced++;
    anim_tick = 1;
    callback_leave();
    return ECORE_CALLBACK_RENEW;
}

static void on_exit_(void)
{
    if (log_file < 0) return;
    char line[200];
    int n = snprintf(line, sizeof line, "exit() is running: inside km_probe_run %s, inside iterate %s, frame %llu\n",
                     running ? "YES" : "no", in_iterate ? "YES" : "no", frame);
    if (write(log_file, line, (size_t)n) < 0) { /* nothing to be done */ }
    fsync(log_file);
}

/* ---- Wayland: wl_compositor and wl_subcompositor through a registry on a private queue ---- */

static void reg_global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t v)
{
    (void)d;
    if (strcmp(iface, "wl_subcompositor") == 0) g_subcomp = wl_registry_bind(r, name, &wl_subcompositor_interface, 1);
    if (strcmp(iface, "wl_compositor") == 0) g_comp = wl_registry_bind(r, name, &wl_compositor_interface, v < 4 ? v : 4);
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t name) { (void)d; (void)r; (void)name; }
static const struct wl_registry_listener reg_listener = { reg_global, reg_remove };

/* ---- args ---- */

static const char *arg_value(const char *args, const char *key)
{
    size_t kl = strlen(key);
    for (const char *p = args; p && *p;) {
        if (strncmp(p, key, kl) == 0 && p[kl] == '=') return p + kl + 1;
        p = strchr(p, ',');
        if (p) p++;
    }
    return NULL;
}
static int arg_is(const char *args, const char *key, const char *val)
{
    const char *v = arg_value(args, key);
    size_t n = strlen(val);
    return v && strncmp(v, val, n) == 0 && (v[n] == 0 || v[n] == ',');
}
static int arg_int(const char *args, const char *key, int dflt)
{
    const char *v = arg_value(args, key);
    return v ? atoi(v) : dflt;
}

/* ---- drawing ---- */

static void draw(int w, int h, double t, double it_mean, double it_max, unsigned late)
{
    SDL_SetRenderDrawColor(ren, 20, 60, 110, 255);
    SDL_RenderClear(ren);

    /* A bar crossing the screen in two seconds: 16 px a frame at 1080p, so a held or doubled frame is
     * a visible jump. */
    float x = (float)fmod(t * (w / 2.0), (double)w);
    SDL_SetRenderDrawColor(ren, 240, 200, 40, 255);
    SDL_FRect bar = { x, h * 0.55f, 120, h * 0.3f };
    SDL_RenderFillRect(ren, &bar);

    /* A row of 60 cells, one lit per frame: a dropped frame on camera is a skipped cell. */
    float cw = w / 60.0f;
    SDL_SetRenderDrawColor(ren, 80, 80, 80, 255);
    SDL_FRect row = { 0, h * 0.9f, (float)w, 30 };
    SDL_RenderFillRect(ren, &row);
    SDL_SetRenderDrawColor(ren, 255, 255, 255, 255);
    SDL_FRect cell = { (float)(frame % 60) * cw, h * 0.9f, cw - 2, 30 };
    SDL_RenderFillRect(ren, &cell);

    char line[96];
    SDL_SetRenderScale(ren, 5, 5);
    SDL_RenderDebugText(ren, 16, 12, "M1 nested loop");
    snprintf(line, sizeof line, "frame %llu  %.1f s", frame, t);
    SDL_RenderDebugText(ren, 16, 28, line);
    snprintf(line, sizeof line, "last key: %s", last_key);
    SDL_RenderDebugText(ren, 16, 44, line);
    snprintf(line, sizeof line, "focus %s  refocus tries %u (%u)", focus_in ? "in" : "OUT", refocus_tries, refocus_total);
    SDL_RenderDebugText(ren, 16, 108, line);
    snprintf(line, sizeof line, "ecore %.2f / %.2f ms", it_mean, it_max);
    SDL_RenderDebugText(ren, 16, 60, line);
    snprintf(line, sizeof line, "late presents (5 s): %u", late);
    SDL_RenderDebugText(ren, 16, 76, line);
    SDL_RenderDebugText(ren, 16, 92, opt.back == BACK_HOST ? "Back: the host's" : "Back ends it");
    SDL_SetRenderScale(ren, 1, 1);
}

/* ---- the probe ---- */

static void *sym(void *lib, const char *name) { return lib ? dlsym(lib, name) : NULL; }

int km_probe_run(const char *args, void *ecore_win, int w, int h, log_fn log)
{
    LOG = log;
    open_log_file();
    if (!args) args = "";

    /* Re-entry: this copy's flag, and a process-wide one, because the host writes each load to a new
     * file and a second copy has statics of its own. */
    if (running || getenv("KM_NESTED_LOOP_ACTIVE")) {
        logf_("RE-ENTRANCY: km_probe_run('%s') called while a run is active (%s); returning at once",
              args, running ? "this copy" : "another copy of the probe");
        return END_REENTERED;
    }
    running = 1;
    setenv("KM_NESTED_LOOP_ACTIVE", "1", 1);
    static int atexit_done;
    if (!atexit_done) { atexit(on_exit_); atexit_done = 1; }

    opt.secs = arg_int(args, "secs", 120);
    if (opt.secs <= 0) opt.secs = 120;
    opt.fps = arg_int(args, "fps", 60);
    if (opt.fps <= 0) opt.fps = 60;
    opt.opaque_after = arg_int(args, "opaque", 1);
    opt.eoquit = arg_int(args, "eoquit", 1);
    opt.pace_deadline = arg_is(args, "pace", "deadline");
    opt.prepend = !arg_is(args, "order", "append");
    opt.back = arg_is(args, "back", "pass") ? BACK_PASS : arg_is(args, "back", "host") ? BACK_HOST : BACK_DONE;
    opt.back_escape = arg_is(args, "backkey", "escape");
    opt.push = arg_int(args, "push", 0) != 0;
    opt.refocus = arg_is(args, "refocus", "stop") ? REFOCUS_STOP : arg_is(args, "refocus", "0") ? REFOCUS_OFF : REFOCUS_OPAQUE;
    opt.refocus_delay = arg_int(args, "refocus_delay", 30);
    opt.kb = arg_int(args, "kb", 0) != 0;
    visible_again = 0;
    if (opt.refocus_delay < 0) opt.refocus_delay = 30;
    focus_in = 1;
    focus_pending = 0;
    last_activate_ns = 0;
    refocus_tries = refocus_total = focus_events = 0;
    host_win = ecore_win;
    ending = END_NONE;
    quit_source = NULL;
    frame = 0;
    memset(key_down, 0, sizeof key_down);
    snprintf(last_key, sizeof last_key, "none yet");
    keys_from_ecore = keys_pushed = keys_from_sdl = keys_from_sdl_own = 0;
    reentrant_callbacks = 0;
    run_start_ns = SDL_GetTicksNS();

    logf_("---- nested loop '%s': pace=%s secs=%d fps=%d opaque=%d back=%s order=%s backkey=%s eoquit=%d; %dx%d",
          args, opt.pace_deadline ? "deadline" : "animator", opt.secs, opt.fps, opt.opaque_after,
          opt.back == BACK_DONE ? "done" : opt.back == BACK_PASS ? "pass" : "host",
          opt.prepend ? "prepend" : "append", opt.back_escape ? "escape" : "acback", opt.eoquit, w, h);
    logf_("     push=%d refocus=%s refocus_delay=%d kb=%d", opt.push,
          opt.refocus == REFOCUS_STOP ? "stop" : opt.refocus == REFOCUS_OFF ? "0" : "1", opt.refocus_delay, opt.kb);

    int rc = 0;
    void *ecore = dlopen("libecore.so.1", RTLD_NOW);
    void *input = dlopen("libecore_input.so.1", RTLD_NOW);
    void *ewl = dlopen("libecore_wl2.so.1", RTLD_NOW);
    ecore_iterate = (void (*)(void))sym(ecore, "ecore_main_loop_iterate");
    ecore_iterate_may_block = (int (*)(int))sym(ecore, "ecore_main_loop_iterate_may_block");
    ecore_animator_add_ = (void *(*)(ecore_task_cb, const void *))sym(ecore, "ecore_animator_add");
    ecore_animator_del_ = (void *(*)(void *))sym(ecore, "ecore_animator_del");
    ecore_animator_frametime_set_ = (void (*)(double))sym(ecore, "ecore_animator_frametime_set");
    ecore_animator_source_get_ = (int (*)(void))sym(ecore, "ecore_animator_source_get");
    ecore_nested_get = (int (*)(void))sym(ecore, "ecore_main_loop_nested_get");
    ecore_handler_add = (void *(*)(int, ecore_event_cb, const void *))sym(ecore, "ecore_event_handler_add");
    ecore_handler_prepend = (void *(*)(int, ecore_event_cb, const void *))sym(ecore, "ecore_event_handler_prepend");
    ecore_handler_del = (void *(*)(void *))sym(ecore, "ecore_event_handler_del");
    ecore_timer_add_ = (void *(*)(double, ecore_task_cb, const void *))sym(ecore, "ecore_timer_add");
    ecore_timer_del_ = (void *(*)(void *))sym(ecore, "ecore_timer_del");
    void *(*win_display)(void *) = (void *(*)(void *))sym(ewl, "ecore_wl2_window_display_get");
    void *(*display_get)(void *) = (void *(*)(void *))sym(ewl, "ecore_wl2_display_get");
    void *(*surface_get)(void *) = (void *(*)(void *))sym(ewl, "ecore_wl2_window_surface_get");
    int *key_down_type = (int *)sym(input, "ECORE_EVENT_KEY_DOWN");
    int *key_up_type = (int *)sym(input, "ECORE_EVENT_KEY_UP");
    int *signal_exit_type = (int *)sym(ecore, "ECORE_EVENT_SIGNAL_EXIT");
    wl2_window_activate = (void (*)(void *))sym(ewl, "ecore_wl2_window_activate");
    wl2_window_activated_get = (int (*)(void *))sym(ewl, "ecore_wl2_window_activated_get");
    if (!ecore_iterate || !ecore_handler_add || !ecore_handler_del || !ecore_timer_add_ || !ecore_timer_del_ ||
        !win_display || !display_get || !surface_get || !key_down_type || !key_up_type) {
        logf_("setup: a needed Ecore symbol is missing: iterate %p handler_add %p handler_del %p timer_add %p "
              "timer_del %p wl2 %p/%p/%p key types %p/%p", (void *)ecore_iterate, (void *)ecore_handler_add,
              (void *)ecore_handler_del, (void *)ecore_timer_add_, (void *)ecore_timer_del_, (void *)win_display,
              (void *)display_get, (void *)surface_get, (void *)key_down_type, (void *)key_up_type);
        rc = 10;
        goto out;
    }
    logf_("Ecore: main loop nesting on entry %d; ecore_event_handler_prepend %s; key types %d/%d; signal exit %d",
          ecore_nested_get ? ecore_nested_get() : -1, ecore_handler_prepend ? "present" : "absent",
          *key_down_type, *key_up_type, signal_exit_type ? *signal_exit_type : -1);

    /* ---- SDL in the host's window, exactly as measured ---- */
    dpy = display_get(win_display(ecore_win));
    parent = surface_get(ecore_win);
    if (!dpy || !parent) { logf_("setup: no wl_display (%p) or host surface (%p)", (void *)dpy, (void *)parent); rc = 11; goto out; }

    SDL_SetLogOutputFunction(sdl_log, NULL);
    if (!SDL_WasInit(SDL_INIT_VIDEO)) {
        SDL_SetPointerProperty(SDL_GetGlobalProperties(), SDL_PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER, dpy);
        SDL_SetHint(SDL_HINT_VIDEO_DRIVER, "wayland");
        SDL_SetHint(SDL_HINT_RENDER_DRIVER, "opengles2");
        SDL_SetHint(SDL_HINT_RENDER_VSYNC, "0");
        if (!SDL_Init(SDL_INIT_VIDEO)) { logf_("setup: SDL_Init failed: %s", SDL_GetError()); rc = 12; goto out; }
    }
    logf_("SDL %d.%d.%d, video driver %s, on the host's wl_display", SDL_MAJOR_VERSION, SDL_MINOR_VERSION,
          SDL_MICRO_VERSION, SDL_GetCurrentVideoDriver());

    if (!win) {
        SDL_PropertiesID p = SDL_CreateProperties();
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_WIDTH_NUMBER, w);
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_HEIGHT_NUMBER, h);
        SDL_SetBooleanProperty(p, SDL_PROP_WINDOW_CREATE_OPENGL_BOOLEAN, true);
        SDL_SetBooleanProperty(p, SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN, true);
        SDL_SetBooleanProperty(p, SDL_PROP_WINDOW_CREATE_WAYLAND_CREATE_EGL_WINDOW_BOOLEAN, true);
        /* No opaque region on the first frame, or the compositor takes keyboard focus from the host. */
        SDL_SetBooleanProperty(p, SDL_PROP_WINDOW_CREATE_TRANSPARENT_BOOLEAN, true);
        win = SDL_CreateWindowWithProperties(p);
        SDL_DestroyProperties(p);
        if (!win) { logf_("setup: SDL_CreateWindowWithProperties failed: %s", SDL_GetError()); rc = 13; goto out; }
        sdl_surf = SDL_GetPointerProperty(SDL_GetWindowProperties(win), SDL_PROP_WINDOW_WAYLAND_SURFACE_POINTER, NULL);
        ren = SDL_CreateRenderer(win, NULL);
        if (!ren) { logf_("setup: SDL_CreateRenderer failed: %s", SDL_GetError()); rc = 14; goto out; }
        SDL_SetRenderVSync(ren, 0);
    } else {
        logf_("setup: reusing this copy's SDL window from an earlier run (it is never destroyed)");
    }

    if (!g_subcomp) {
        struct wl_event_queue *q = wl_display_create_queue(dpy);
        struct wl_display *wrapped = wl_proxy_create_wrapper(dpy);
        wl_proxy_set_queue((struct wl_proxy *)wrapped, q);
        struct wl_registry *reg = wl_display_get_registry(wrapped);
        wl_registry_add_listener(reg, &reg_listener, NULL);
        wl_display_roundtrip_queue(dpy, q);
    }
    if (!g_subcomp || !g_comp) { logf_("setup: wl_subcompositor %p, wl_compositor %p", (void *)g_subcomp, (void *)g_comp); rc = 15; goto out; }
    sub = wl_subcompositor_get_subsurface(g_subcomp, sdl_surf, parent);
    wl_subsurface_set_position(sub, 0, 0);
    wl_subsurface_place_above(sub, parent);
    wl_subsurface_set_desync(sub);
    wl_surface_commit(parent);
    wl_display_flush(dpy);
    logf_("SDL window: renderer %s, a desynchronised subsurface above the host's surface, transparent", SDL_GetRendererName(ren));

    /* ---- Ecore handlers, registered just before the loop ---- */
    void *(*add_key)(int, ecore_event_cb, const void *) = opt.prepend && ecore_handler_prepend ? ecore_handler_prepend : ecore_handler_add;
    void *h_down = add_key(*key_down_type, on_key, (void *)1);
    void *h_up = add_key(*key_up_type, on_key, NULL);
    void *h_sig = signal_exit_type && *signal_exit_type ? ecore_handler_add(*signal_exit_type, on_signal_exit, NULL) : NULL;
    void *timer = ecore_timer_add_(0.1, on_timer, NULL);
    logf_("key handlers %s (%p, %p); signal-exit handler %p; 100 ms timer %p",
          add_key == ecore_handler_prepend ? "prepended, ahead of DALi's" :
          opt.prepend ? "APPENDED: prepend is absent, so DALi sees each key first" : "appended, after DALi's",
          h_down, h_up, h_sig, timer);
    void *h_focus[FE_COUNT] = { 0 };
    {
        char found[400] = "";
        for (int i = 0; i < FE_COUNT; i++) {
            int *p = (int *)sym(ewl, focus_event_names[i]);
            if (!p || !*p) continue;
            h_focus[i] = ecore_handler_add(*p, on_focus_event, (void *)(intptr_t)i);
            size_t len = strlen(found);
            snprintf(found + len, sizeof found - len, "%s ", focus_event_names[i] + 16);
        }
        logf_("focus events watched: %s; ecore_wl2_window_activate %s, activated_get %s (host activated %d)", found,
              wl2_window_activate ? "present" : "ABSENT", wl2_window_activated_get ? "present" : "absent",
              wl2_window_activated_get ? wl2_window_activated_get(ecore_win) : -1);
    }

    void *loop_obj = NULL;
    const void *quit_desc = NULL;
    if (opt.eoquit) {
        void *eo = dlopen("libeo.so.1", RTLD_NOW);
        eo_cb_add = (ebool (*)(void *, const void *, short, efl_event_cb, const void *))sym(eo, "efl_event_callback_priority_add");
        eo_cb_del = (ebool (*)(void *, const void *, efl_event_cb, const void *))sym(eo, "efl_event_callback_del");
        quit_desc = sym(ecore, "_EFL_LOOP_EVENT_QUIT");
        static const char *getters[] = { "efl_main_loop_get", "efl_app_main_get", "ecore_main_loop_get" };
        const char *got = "none";
        for (unsigned i = 0; i < sizeof getters / sizeof getters[0] && !loop_obj; i++) {
            void *(*g)(void) = (void *(*)(void))sym(ecore, getters[i]);
            if (g && (loop_obj = g())) got = getters[i];
        }
        ebool ok = 0;
        if (eo_cb_add && quit_desc && loop_obj) ok = eo_cb_add(loop_obj, quit_desc, 0, on_loop_quit, NULL);
        logf_("Efl loop quit event: callback_add %p, _EFL_LOOP_EVENT_QUIT %p, loop %p from %s: %s",
              (void *)eo_cb_add, quit_desc, loop_obj, got, ok ? "listening" : "NOT listening");
        if (!ok) loop_obj = NULL;
    }

    /* ---- what paces the frames ---- */
    void *animator = NULL;
    if (!opt.pace_deadline) {
        if (!ecore_animator_add_ || !ecore_animator_del_) {
            logf_("pace: ecore_animator_add/del absent (%p/%p); falling back to pace=deadline",
                  (void *)ecore_animator_add_, (void *)ecore_animator_del_);
            opt.pace_deadline = 1;
        } else {
            if (ecore_animator_frametime_set_) ecore_animator_frametime_set_(1.0 / opt.fps);
            anim_tick = 0;
            anim_ticks = anim_coalesced = 0;
            animator = ecore_animator_add_(on_animator, NULL);
            logf_("pace: animator %p, source %d (0 timer, 1 custom), frametime %s 1/%d s; waiting with %s",
                  animator, ecore_animator_source_get_ ? ecore_animator_source_get_() : -1,
                  ecore_animator_frametime_set_ ? "set to" : "NOT set (no ecore_animator_frametime_set), wanted",
                  opt.fps,
                  ecore_iterate_may_block ? "ecore_main_loop_iterate_may_block(1)"
                                          : "FALLBACK: ecore_main_loop_iterate_may_block is absent, so "
                                            "ecore_main_loop_iterate() plus a 1 ms sleep");
            if (!animator) { logf_("pace: ecore_animator_add returned NULL; falling back to pace=deadline"); opt.pace_deadline = 1; }
        }
    }
    if (opt.pace_deadline) logf_("pace: deadline, one ecore_main_loop_iterate() a frame, then SDL_DelayPrecise to 1/%d s deadlines", opt.fps);

    if (opt.kb) own_keyboard_start();

    /* ---- the nested loop ---- */
    const Uint64 budget = 1000000000ull / (Uint64)opt.fps;
    const Uint64 cap_ns = (Uint64)opt.secs * 1000000000ull;
    Uint64 start = SDL_GetTicksNS(), deadline = start, prev_present = 0, window_start = start;
    unsigned w_frames = 0, w_late = 0, w_resyncs = 0, w_timer0 = timer_ticks;
    unsigned w_anim0 = anim_ticks, w_coal0 = anim_coalesced;
    Uint64 w_slowest = 0, w_it_sum = 0, w_it_max = 0;
    Uint64 w_draw_sum = 0, w_draw_max = 0, w_pres_sum = 0, w_pres_max = 0, w_over_max = 0;
    unsigned long long w_calls_sum = 0;
    unsigned w_calls_max = 0;
    unsigned long long t_frames = 0, t_late = 0;
    Uint64 t_slowest = 0, t_it_max = 0, t_pres_max = 0;
    double shown_it_mean = 0, shown_it_max = 0;
    unsigned shown_late = 0;
    int made_opaque = 0, back_seen = 0;
    int opaque_cleared = 0, stopped = 0;
    unsigned long long out_frame = 0, opaque_restore_at = 0, stopped_frames = 0;
    Uint64 out_ns = 0;
    run_start_ns = start;

    while (ending == END_NONE) {
        /* 1. Ecore: DALi's events, the host's posted work, timers, and the key handlers. Under
         * pace=deadline, one non-blocking pass. Under pace=animator, passes that may block until the
         * animator says a frame is due -- at least one, so a frame never skips Ecore. */
        Uint64 a = SDL_GetTicksNS();
        unsigned calls = 0;
        if (opt.pace_deadline) {
            in_iterate = 1;
            ecore_iterate();
            in_iterate = 0;
            calls = 1;
        } else {
            do {
                in_iterate = 1;
                if (ecore_iterate_may_block) ecore_iterate_may_block(1);
                else ecore_iterate();
                in_iterate = 0;
                calls++;
                if (!ecore_iterate_may_block && !anim_tick && !quit_source) SDL_DelayNS(1000000);
            } while (!anim_tick && !quit_source && SDL_GetTicksNS() - start < cap_ns);
            anim_tick = 0;
        }
        Uint64 it = SDL_GetTicksNS() - a;
        w_it_sum += it;
        if (it > w_it_max) w_it_max = it;
        w_calls_sum += calls;
        if (calls > w_calls_max) w_calls_max = calls;
        if (quit_source) { ending = END_HOST_QUIT; break; }

        /* 1b. Focus, as the handlers saw it during that pass. */
        if (focus_pending) {
            int in = focus_pending > 0;
            focus_pending = 0;
            if (in != focus_in) {
                focus_in = in;
                Uint64 fnow = SDL_GetTicksNS();
                if (!in) {
                    out_frame = frame;
                    out_ns = fnow;
                    refocus_tries = 0;
                    logf_("%.2f s focus OUT at frame %llu", secs_since_start(), frame);
                    if (opt.refocus == REFOCUS_OPAQUE) {
                        wl_surface_set_opaque_region(sdl_surf, NULL);
                        opaque_cleared = 1;
                        opaque_restore_at = 0;
                        logf_("%.2f s refocus: opaque region cleared, on the next present; drawing goes on; "
                              "activate in %d frames", secs_since_start(), opt.refocus_delay);
                    } else if (opt.refocus == REFOCUS_STOP) {
                        stopped = 1;
                        logf_("%.2f s refocus=stop: presenting stopped", secs_since_start());
                        activate_host("refocus=stop, at once");
                        refocus_tries = 1;
                    }
                } else {
                    logf_("%.2f s focus IN after %.0f ms out, %u activate tries", secs_since_start(),
                          out_ns ? (fnow - out_ns) / 1e6 : 0.0, refocus_tries);
                    if (opaque_cleared) opaque_restore_at = frame + 1;
                    if (stopped) {
                        stopped = 0;
                        prev_present = 0;
                        logf_("%.2f s refocus=stop: presenting resumes on focus in", secs_since_start());
                    }
                    refocus_tries = 0;
                }
            }
        }
        /* While focus is out, whatever else arrives: the first activate after refocus_delay frames,
         * then one every 60 frames, with no cap, until FOCUS_IN. */
        if (opt.refocus != REFOCUS_OFF && !focus_in && !stopped &&
            frame >= out_frame + (unsigned long long)opt.refocus_delay + 60ull * refocus_tries) {
            char why[64];
            snprintf(why, sizeof why, "refocus try %u, focus out since frame %llu", refocus_tries + 1, out_frame);
            activate_host(why);
            refocus_tries++;
        }
        /* Back from the set's menu: VISIBILITY_CHANGE fully_obscured=0. */
        if (visible_again) {
            visible_again = 0;
            if (opt.refocus != REFOCUS_OFF && !focus_in) activate_host("visible again (fully_obscured=0) with focus out");
            else logf_("%.2f s visible again; focus %s, no activate", secs_since_start(), focus_in ? "in" : "out (refocus=0)");
        }
        if (kb_queue) wl_display_dispatch_queue_pending(dpy, kb_queue);
        if (stopped && SDL_GetTicksNS() - out_ns >= 2000000000ull) {
            stopped = 0;
            prev_present = 0;
            logf_("%.2f s refocus=stop: no focus in after 2 s; presenting resumes anyway", secs_since_start());
        }

        /* 2. SDL's queue: SDL's own keyboard, and the pushed keys under push=1. */
        SDL_Event e;
        while (SDL_PollEvent(&e)) {
            if (e.type == SDL_EVENT_KEY_DOWN || e.type == SDL_EVENT_KEY_UP) {
                int ours = e.key.which == PUSHED_KEYBOARD_ID;
                if (ours) keys_from_sdl++; else keys_from_sdl_own++;
                const char *kn = SDL_GetKeyName(e.key.key);
                if (e.type == SDL_EVENT_KEY_DOWN)
                    snprintf(last_key, sizeof last_key, "%s%s", kn && *kn ? kn : "unmapped", ours ? "" : " (SDL's own)");
                logf_("%.2f s SDL key %s %s, scancode %s%s, %s", secs_since_start(),
                      e.type == SDL_EVENT_KEY_DOWN ? "down" : "up", kn && *kn ? kn : "unmapped",
                      SDL_GetScancodeName(e.key.scancode), e.key.repeat ? ", repeat" : "",
                      ours ? "pushed from Ecore" : "from SDL's own keyboard");
                if (e.type == SDL_EVENT_KEY_DOWN &&
                    (e.key.key == SDLK_AC_BACK || e.key.key == SDLK_ESCAPE ||
                     e.key.scancode == SDL_SCANCODE_AC_BACK || e.key.scancode == SDL_SCANCODE_ESCAPE)) {
                    back_seen = 1;
                    logf_("%.2f s Back read out of SDL's queue, %s", secs_since_start(),
                          ours ? "pushed from Ecore" : "from SDL's own keyboard");
                }
            } else if (e.type == SDL_EVENT_QUIT) {
                logf_("%.2f s SDL_EVENT_QUIT", secs_since_start());
            } else if (e.type == SDL_EVENT_WINDOW_FOCUS_GAINED || e.type == SDL_EVENT_WINDOW_FOCUS_LOST) {
                logf_("%.2f s SDL window focus %s", secs_since_start(), e.type == SDL_EVENT_WINDOW_FOCUS_GAINED ? "gained" : "lost");
            }
        }
        if (back_seen && opt.back != BACK_HOST) { ending = END_BACK; break; }

        /* refocus=stop: nothing is presented while focus is out; Ecore keeps turning. */
        if (stopped) {
            stopped_frames++;
            if (SDL_GetTicksNS() - start >= cap_ns) { ending = END_CAP; break; }
            if (opt.pace_deadline) { SDL_DelayNS(budget); deadline = SDL_GetTicksNS(); }
            continue;
        }

        /* 3. Draw and present. */
        Uint64 d0 = SDL_GetTicksNS();
        double t = (d0 - start) / 1e9;
        draw(w, h, t, shown_it_mean, shown_it_max, shown_late);
        Uint64 p0 = SDL_GetTicksNS();
        if (!SDL_RenderPresent(ren)) {
            logf_("%.2f s SDL_RenderPresent failed: %s", t, SDL_GetError());
            ending = END_WAYLAND;
            break;
        }
        Uint64 now = SDL_GetTicksNS();
        w_draw_sum += p0 - d0;
        if (p0 - d0 > w_draw_max) w_draw_max = p0 - d0;
        w_pres_sum += now - p0;
        if (now - p0 > w_pres_max) w_pres_max = now - p0;
        frame++;
        w_frames++;
        if (prev_present) {
            Uint64 iv = now - prev_present;
            if (iv > 20000000ull) w_late++;
            if (iv > w_slowest) w_slowest = iv;
        }
        prev_present = now;

        if (!made_opaque && !opaque_cleared && opt.opaque_after > 0 && frame >= (unsigned long long)opt.opaque_after) {
            struct wl_region *r = wl_compositor_create_region(g_comp);
            wl_region_add(r, 0, 0, w, h);
            wl_surface_set_opaque_region(sdl_surf, r);
            wl_region_destroy(r);
            made_opaque = 1;
            logf_("%.2f s opaque region set after frame %llu; takes effect on the next present", t, frame);
        }
        if (opaque_restore_at && frame >= opaque_restore_at && focus_in) {
            struct wl_region *r = wl_compositor_create_region(g_comp);
            wl_region_add(r, 0, 0, w, h);
            wl_surface_set_opaque_region(sdl_surf, r);
            wl_region_destroy(r);
            opaque_restore_at = 0;
            opaque_cleared = 0;
            logf_("%.2f s refocus: full opaque region set again at frame %llu, a frame after focus in", t, frame);
        }
        int werr = wl_display_get_error(dpy);
        if (werr) { logf_("%.2f s wl_display error %d", t, werr); ending = END_WAYLAND; break; }

        /* 4. Report every 5 s. */
        if (now - window_start >= 5000000000ull) {
            shown_it_mean = w_frames ? w_it_sum / 1e6 / w_frames : 0;
            shown_it_max = w_it_max / 1e6;
            shown_late = w_late;
            double n = w_frames ? (double)w_frames : 1;
            logf_("%.0f s: %u frames presented, %u more than 20 ms after the one before, slowest %.2f ms; "
                  "in Ecore mean %.3f ms max %.2f ms, iterate calls per frame mean %.2f max %u; "
                  "draw mean %.2f max %.2f ms; present mean %.2f max %.2f ms; %s%.2f ms; "
                  "%u Ecore timer ticks of 50; %u animator ticks, %u coalesced; %u resyncs; "
                  "keys ecore %u pushed %u sdl %u sdl-own %u; re-entrant callbacks %u",
                  t, w_frames, w_late, w_slowest / 1e6, shown_it_mean, shown_it_max, w_calls_sum / n, w_calls_max,
                  w_draw_sum / 1e6 / n, w_draw_max / 1e6, w_pres_sum / 1e6 / n, w_pres_max / 1e6,
                  opt.pace_deadline ? "delay overshoot max " : "(no delay under pace=animator) ",
                  w_over_max / 1e6, timer_ticks - w_timer0, anim_ticks - w_anim0, anim_coalesced - w_coal0,
                  w_resyncs, keys_from_ecore, keys_pushed, keys_from_sdl, keys_from_sdl_own, reentrant_callbacks);
            t_frames += w_frames; t_late += w_late;
            if (w_slowest > t_slowest) t_slowest = w_slowest;
            if (w_it_max > t_it_max) t_it_max = w_it_max;
            if (w_pres_max > t_pres_max) t_pres_max = w_pres_max;
            w_frames = w_late = w_resyncs = 0;
            w_slowest = w_it_sum = w_it_max = 0;
            w_draw_sum = w_draw_max = w_pres_sum = w_pres_max = w_over_max = 0;
            w_calls_sum = 0;
            w_calls_max = 0;
            w_timer0 = timer_ticks;
            w_anim0 = anim_ticks;
            w_coal0 = anim_coalesced;
            window_start = now;
        }

        /* 5. The safety cap, then, under pace=deadline, the rest of the frame budget. Under
         * pace=animator the next frame's wait is the Ecore pass at the top of the loop. */
        if (now - start >= cap_ns) { ending = END_CAP; break; }
        if (opt.pace_deadline) {
            deadline += budget;
            now = SDL_GetTicksNS();
            if (now > deadline + budget) { deadline = now; w_resyncs++; }
            else if (now < deadline) {
                SDL_DelayPrecise(deadline - now);
                Uint64 woke = SDL_GetTicksNS();
                if (woke > deadline && woke - deadline > w_over_max) w_over_max = woke - deadline;
            }
        }
    }
    t_frames += w_frames; t_late += w_late;
    if (w_slowest > t_slowest) t_slowest = w_slowest;
    if (w_it_max > t_it_max) t_it_max = w_it_max;
    if (w_pres_max > t_pres_max) t_pres_max = w_pres_max;
    double total = (SDL_GetTicksNS() - start) / 1e9;
    logf_("ending: %s%s%s, after %.1f s", ending_name(ending), quit_source ? " via " : "", quit_source ? quit_source : "", total);
    if (opt.refocus == REFOCUS_STOP)
        logf_("focus: %s at the end; %u focus events, %u activate calls, %llu loop passes not presented while stopped",
              focus_in ? "in" : "OUT", focus_events, refocus_total, stopped_frames);
    else
        logf_("focus: %s at the end; %u focus events, %u activate calls", focus_in ? "in" : "OUT", focus_events,
              refocus_total);
    logf_("totals (pace=%s): %llu frames (%.1f fps), %llu late, slowest %.2f ms, in Ecore max %.2f ms, present max %.2f ms; "
          "animator ticks %u, coalesced %u; keys ecore %u pushed %u sdl %u sdl-own %u; re-entrant callbacks %u",
          opt.pace_deadline ? "deadline" : "animator", t_frames, total > 0 ? t_frames / total : 0, t_late,
          t_slowest / 1e6, t_it_max / 1e6, t_pres_max / 1e6, anim_ticks, anim_coalesced,
          keys_from_ecore, keys_pushed, keys_from_sdl, keys_from_sdl_own, reentrant_callbacks);

    /* ---- the measured teardown: subsurface destroyed and committed, a null buffer, SDL's window kept ---- */
    ecore_handler_del(h_down);
    ecore_handler_del(h_up);
    if (h_sig) ecore_handler_del(h_sig);
    ecore_timer_del_(timer);
    if (animator) ecore_animator_del_(animator);
    for (int i = 0; i < FE_COUNT; i++) if (h_focus[i]) ecore_handler_del(h_focus[i]);
    own_keyboard_stop();
    if (loop_obj && eo_cb_del) eo_cb_del(loop_obj, quit_desc, on_loop_quit, NULL);
    Uint64 td = SDL_GetTicksNS();
    wl_subsurface_destroy(sub);
    sub = NULL;
    wl_surface_attach(sdl_surf, NULL, 0, 0);
    wl_surface_commit(sdl_surf);
    wl_surface_commit(parent);
    wl_display_flush(dpy);
    logf_("teardown: handlers, timer and animator removed, subsurface destroyed, null buffer committed, SDL's window kept; "
          "%.1f ms, wl_display error %d", (SDL_GetTicksNS() - td) / 1e6, wl_display_get_error(dpy));
    rc = ending;

out:
    logf_("km_probe_run returns %d", rc);
    running = 0;
    unsetenv("KM_NESTED_LOOP_ACTIVE");
    return rc;
}
