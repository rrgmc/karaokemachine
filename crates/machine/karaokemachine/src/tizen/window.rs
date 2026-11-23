//! SDL's window inside the host's, and the per-frame Ecore pump.
//!
//! This is the M1 probe, `tools/port/machine/tizen/probe/nested_loop.c`, in Rust, and every choice
//! in it was measured on the set in `docs/research/tizen.md` §3:
//!
//! - **SDL shares the host's `wl_display`**, because a window of its own sticks on an old frame.
//! - **Its surface is a custom-role, desynchronised subsurface above the host's**, because the
//!   compositor has no `xdg_wm_base` and `SDL_CreateWindow` would wait for it for ever.
//! - **It is created transparent and declared opaque after its first frame.** An opaque region on
//!   the first frame takes keyboard focus from the host; none at all makes a playing song stutter.
//! - **The opaque region is cleared while a system overlay has focus**, and set again a frame after
//!   focus returns, or focus never returns.
//! - **Each frame waits in Ecore** until an animator ticks, so the host's UI thread keeps turning.
//! - **The window is never destroyed and SDL never quit.** Either kills the host. The ending is the
//!   subsurface destroyed and a null buffer committed, with SDL's window kept.
//!
//! EFL is reached with `dlopen`, as is `libwayland-client`: the set has them and the build image
//! does not, and SDL already loads Wayland the same way.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI8, Ordering};

use anyhow::Context as _;
use sdl3::VideoSubsystem;
use sdl3::render::Canvas;
use sdl3::sys;
use sdl3::video::Window;

// ---- dlopen ----------------------------------------------------------------------------------

/// `RTLD_NOW`, which is 2 on every Linux.
const RTLD_NOW: c_int = 2;

#[expect(
    unsafe_code,
    reason = "declaring dlopen and dlsym, which glibc exports"
)]
unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[expect(unsafe_code, reason = "dlopen of a system library by its soname")]
fn open(name: &CStr) -> anyhow::Result<*mut c_void> {
    // SAFETY: a NUL-terminated soname; the handle is never closed, so nothing resolved from it
    // outlives it.
    let lib = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
    anyhow::ensure!(!lib.is_null(), "{} would not open", name.to_string_lossy());
    Ok(lib)
}

/// A symbol's address, or `None`.
#[expect(unsafe_code, reason = "dlsym on a handle from `open`")]
fn address(lib: *mut c_void, name: &CStr) -> Option<*mut c_void> {
    // SAFETY: `lib` came from `dlopen` and is never closed; `name` is NUL-terminated.
    let p = unsafe { dlsym(lib, name.as_ptr()) };
    (!p.is_null()).then_some(p)
}

/// A function, typed by the caller as the C prototype it is documented with.
#[expect(
    unsafe_code,
    reason = "a dlsym result turned into the function pointer type its C header declares"
)]
fn function<F: Copy>(lib: *mut c_void, name: &CStr) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: every `F` used below is an `extern "C" fn` type transcribed from the library's header,
    // and a non-null code address is a valid value of it.
    address(lib, name).map(|p| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

fn required<F: Copy>(lib: *mut c_void, name: &CStr) -> anyhow::Result<F> {
    function(lib, name).with_context(|| format!("{} is missing", name.to_string_lossy()))
}

/// An Ecore event type, which the library exports as an `int` it fills in at init.
#[expect(
    unsafe_code,
    reason = "reading an exported int whose address dlsym gave"
)]
fn event_type(lib: *mut c_void, name: &CStr) -> Option<c_int> {
    // SAFETY: the symbol is an `int` the library owns for its lifetime, and it is read on the UI
    // thread after the host has initialised Ecore.
    address(lib, name)
        .map(|p| unsafe { *p.cast::<c_int>() })
        .filter(|&t| t != 0)
}

// ---- the host's window -------------------------------------------------------------------------

/// The host's `wl_display` and the `wl_surface` of its window.
#[expect(unsafe_code, reason = "three Ecore_Wl2 getters on the host's window")]
fn host_surfaces() -> anyhow::Result<(*mut c_void, *mut c_void)> {
    type Get = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    let host = crate::tizen::host().context("the host published no window")?;
    let ewl = open(c"libecore_wl2.so.1")?;
    let window_display: Get = required(ewl, c"ecore_wl2_window_display_get")?;
    let display_get: Get = required(ewl, c"ecore_wl2_display_get")?;
    let surface_get: Get = required(ewl, c"ecore_wl2_window_surface_get")?;
    let window = host.ecore_window as *mut c_void;
    // SAFETY: `window` is the host's live `Ecore_Wl2_Window *`, and this runs on its UI thread.
    let (display, surface) = unsafe { (display_get(window_display(window)), surface_get(window)) };
    anyhow::ensure!(
        !display.is_null() && !surface.is_null(),
        "the host's window has no wl_display or wl_surface"
    );
    Ok((display, surface))
}

/// Points SDL at the host's `wl_display`. Before SDL's video subsystem starts.
#[expect(
    unsafe_code,
    reason = "one SDL global property, which sdl3-rs does not wrap"
)]
pub(crate) fn prepare() -> anyhow::Result<()> {
    let (display, _) = host_surfaces()?;
    // SAFETY: the global property set is SDL's own; the pointer is stored, not dereferenced, until
    // the video subsystem opens it on this thread.
    let set = unsafe {
        sys::properties::SDL_SetPointerProperty(
            sys::properties::SDL_GetGlobalProperties(),
            sys::video::SDL_PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER,
            display,
        )
    };
    anyhow::ensure!(
        set,
        "SDL would not take the host's wl_display: {}",
        sdl3::get_error()
    );
    sdl3::hint::set("SDL_VIDEO_DRIVER", "wayland");
    sdl3::hint::set("SDL_RENDER_DRIVER", "opengles2");
    // The animator paces the frames. SDL's vsync waits on frame callbacks the set rarely sends.
    sdl3::hint::set("SDL_RENDER_VSYNC", "0");
    Ok(())
}

static CREATED: AtomicBool = AtomicBool::new(false);

/// SDL's window: a custom-role surface the size of the host's, transparent, with an EGL window.
///
/// sdl3-rs builds windows from a title, a size and flags only, so the window is created from
/// properties here and wrapped. **It is created once per process and never destroyed**: a clone of
/// its context is leaked, which also holds SDL and its video subsystem up, so no drop anywhere
/// destroys it or quits SDL.
#[expect(
    unsafe_code,
    reason = "SDL_CreateWindowWithProperties, and wrapping the result"
)]
pub(crate) fn create_window(video: &VideoSubsystem, title: &str) -> anyhow::Result<Window> {
    use sys::properties::{
        SDL_CreateProperties, SDL_DestroyProperties, SDL_SetBooleanProperty, SDL_SetNumberProperty,
        SDL_SetStringProperty,
    };
    use sys::video as v;
    anyhow::ensure!(
        !CREATED.swap(true, Ordering::AcqRel),
        "SDL's window inside the host's exists already; it is made once per process"
    );
    let host = crate::tizen::host().context("the host published no window")?;
    let title = std::ffi::CString::new(title)?;
    // SAFETY: a fresh property set, filled and freed here; the window SDL returns is wrapped at once.
    let raw = unsafe {
        let props = SDL_CreateProperties();
        SDL_SetStringProperty(
            props,
            v::SDL_PROP_WINDOW_CREATE_TITLE_STRING,
            title.as_ptr(),
        );
        SDL_SetNumberProperty(
            props,
            v::SDL_PROP_WINDOW_CREATE_WIDTH_NUMBER,
            host.width.into(),
        );
        SDL_SetNumberProperty(
            props,
            v::SDL_PROP_WINDOW_CREATE_HEIGHT_NUMBER,
            host.height.into(),
        );
        SDL_SetBooleanProperty(props, v::SDL_PROP_WINDOW_CREATE_OPENGL_BOOLEAN, true);
        SDL_SetBooleanProperty(props, v::SDL_PROP_WINDOW_CREATE_BORDERLESS_BOOLEAN, true);
        SDL_SetBooleanProperty(
            props,
            v::SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN,
            true,
        );
        SDL_SetBooleanProperty(
            props,
            v::SDL_PROP_WINDOW_CREATE_WAYLAND_CREATE_EGL_WINDOW_BOOLEAN,
            true,
        );
        // No opaque region on the first frame, or the compositor takes the host's keyboard focus.
        SDL_SetBooleanProperty(props, v::SDL_PROP_WINDOW_CREATE_TRANSPARENT_BOOLEAN, true);
        let raw = v::SDL_CreateWindowWithProperties(props);
        SDL_DestroyProperties(props);
        raw
    };
    anyhow::ensure!(
        !raw.is_null(),
        "SDL would not create its window: {}",
        sdl3::get_error()
    );
    // SAFETY: `raw` is a live window SDL just made, owned by nothing else, with no Metal view.
    let window = unsafe { Window::from_ll(video.clone(), raw, null_mut()) };
    std::mem::forget(window.context());
    tracing::info!(
        driver = video.current_video_driver(),
        width = host.width,
        height = host.height,
        "SDL's window, on the host's wl_display"
    );
    Ok(window)
}

// ---- Ecore and Wayland, by hand ----------------------------------------------------------------

/// `Eina_Bool`.
type EinaBool = u8;
const CALLBACK_DONE: EinaBool = 0;
const CALLBACK_PASS_ON: EinaBool = 1;
const CALLBACK_RENEW: EinaBool = 1;

type EventCb = extern "C" fn(*mut c_void, c_int, *mut c_void) -> EinaBool;
type TaskCb = extern "C" fn(*mut c_void) -> EinaBool;
type HandlerAdd = unsafe extern "C" fn(c_int, EventCb, *const c_void) -> *mut c_void;
type Del = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

struct Ecore {
    iterate_may_block: unsafe extern "C" fn(c_int) -> c_int,
    animator_del: Del,
    handler_del: Del,
}

/// `libwayland-client`'s entry points, as `wayland-client-protocol.h` uses them. The marshalling
/// calls are the pre-1.20 ones, which every libwayland still exports.
struct Wayland {
    display: *mut c_void,
    marshal: unsafe extern "C" fn(*mut c_void, u32, ...),
    marshal_constructor: unsafe extern "C" fn(*mut c_void, u32, *const c_void, ...) -> *mut c_void,
    marshal_constructor_versioned:
        unsafe extern "C" fn(*mut c_void, u32, *const c_void, u32, ...) -> *mut c_void,
    proxy_destroy: unsafe extern "C" fn(*mut c_void),
    flush: unsafe extern "C" fn(*mut c_void) -> c_int,
    get_error: unsafe extern "C" fn(*mut c_void) -> c_int,
    region_interface: *const c_void,
}

// Request opcodes, from wayland.xml.
const DISPLAY_GET_REGISTRY: u32 = 1;
const REGISTRY_BIND: u32 = 0;
const COMPOSITOR_CREATE_REGION: u32 = 1;
const REGION_DESTROY: u32 = 0;
const REGION_ADD: u32 = 1;
const SUBCOMPOSITOR_GET_SUBSURFACE: u32 = 1;
const SUBSURFACE_DESTROY: u32 = 0;
const SUBSURFACE_SET_POSITION: u32 = 1;
const SUBSURFACE_PLACE_ABOVE: u32 = 2;
const SUBSURFACE_SET_DESYNC: u32 = 5;
const SURFACE_ATTACH: u32 = 1;
const SURFACE_SET_OPAQUE_REGION: u32 = 4;
const SURFACE_COMMIT: u32 = 6;

/// Set by the animator, taken by [`Frames::wait`].
static TICK: AtomicBool = AtomicBool::new(false);
/// The newest focus change of the host's window: 1 in, -1 out, 0 none since the last frame.
static FOCUS: AtomicI8 = AtomicI8::new(0);
/// Ecore saw the process asked to exit.
static QUIT: AtomicBool = AtomicBool::new(false);

extern "C" fn on_animator(_: *mut c_void) -> EinaBool {
    TICK.store(true, Ordering::Release);
    CALLBACK_RENEW
}

extern "C" fn on_focus_in(_: *mut c_void, _: c_int, _: *mut c_void) -> EinaBool {
    FOCUS.store(1, Ordering::Release);
    CALLBACK_PASS_ON
}

extern "C" fn on_focus_out(_: *mut c_void, _: c_int, _: *mut c_void) -> EinaBool {
    FOCUS.store(-1, Ordering::Release);
    CALLBACK_PASS_ON
}

extern "C" fn on_signal_exit(_: *mut c_void, _: c_int, _: *mut c_void) -> EinaBool {
    QUIT.store(true, Ordering::Release);
    CALLBACK_PASS_ON
}

/// Withholds Back from the host. SDL's own keyboard delivers it to the machine, which decides what
/// Back means; the host seeing it too would exit the application on every press.
#[expect(
    unsafe_code,
    reason = "reading the key name from Ecore's Ecore_Event_Key"
)]
extern "C" fn on_key(_: *mut c_void, _: c_int, event: *mut c_void) -> EinaBool {
    // SAFETY: Ecore passes an `Ecore_Event_Key *`, whose first field has been `const char *keyname`
    // since EFL 1.0; it is only read, during the callback.
    let name = unsafe {
        let name = event
            .cast::<*const c_char>()
            .as_ref()
            .copied()
            .unwrap_or(std::ptr::null());
        (!name.is_null()).then(|| CStr::from_ptr(name))
    };
    if name == Some(c"XF86Back") {
        CALLBACK_DONE
    } else {
        CALLBACK_PASS_ON
    }
}

/// What a registry round trip binds, filled in by [`on_global`].
struct Globals<'a> {
    wl: &'a Wayland,
    compositor_interface: *const c_void,
    subcompositor_interface: *const c_void,
    compositor: *mut c_void,
    subcompositor: *mut c_void,
}

#[repr(C)]
struct RegistryListener {
    global: extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32),
    global_remove: extern "C" fn(*mut c_void, *mut c_void, u32),
}

static REGISTRY_LISTENER: RegistryListener = RegistryListener {
    global: on_global,
    global_remove: on_global_remove,
};

#[expect(
    unsafe_code,
    reason = "wl_registry.bind, from the registry's own listener"
)]
extern "C" fn on_global(
    data: *mut c_void,
    registry: *mut c_void,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    // SAFETY: `data` is the `Globals` on `Frames::attach`'s stack, alive for the round trip that
    // dispatches this; `interface` is a NUL-terminated string libwayland owns for the call.
    let (globals, interface) = unsafe { (&mut *data.cast::<Globals>(), CStr::from_ptr(interface)) };
    let (target, wanted, version) = if interface == c"wl_compositor" {
        (
            &mut globals.compositor,
            globals.compositor_interface,
            version.min(4),
        )
    } else if interface == c"wl_subcompositor" {
        (
            &mut globals.subcompositor,
            globals.subcompositor_interface,
            1,
        )
    } else {
        return;
    };
    // SAFETY: `wl_registry_bind` as the protocol header writes it. A `wl_interface` begins with its
    // `const char *name`.
    *target = unsafe {
        let iface_name = *wanted.cast::<*const c_char>();
        (globals.wl.marshal_constructor_versioned)(
            registry,
            REGISTRY_BIND,
            wanted,
            version,
            name,
            iface_name,
            version,
            null_mut::<c_void>(),
        )
    };
}

extern "C" fn on_global_remove(_: *mut c_void, _: *mut c_void, _: u32) {}

// ---- the frames ----------------------------------------------------------------------------------

/// SDL's window made a subsurface of the host's, and the per-frame pump. Lives on the host's UI
/// thread for as long as the frame loop does; dropping it is the measured teardown.
pub(crate) struct Frames {
    ecore: Ecore,
    wl: Wayland,
    surface: *mut c_void,
    parent: *mut c_void,
    compositor: *mut c_void,
    subsurface: *mut c_void,
    width: i32,
    height: i32,
    animator: *mut c_void,
    handlers: Vec<*mut c_void>,
    focus_in: bool,
    /// The presented-frame count at which the full opaque region is declared. `None` while it is
    /// declared, or while focus is out.
    opaque_at: Option<u64>,
    presented: u64,
    wayland_error: bool,
}

impl Frames {
    /// Makes SDL's surface a desynchronised subsurface above the host's, turns the renderer's vsync
    /// off, and starts the animator and the event handlers.
    #[expect(
        unsafe_code,
        reason = "Ecore and Wayland calls through dlsym'd entry points"
    )]
    pub(crate) fn attach(canvas: &Canvas<Window>) -> anyhow::Result<Self> {
        let host = crate::tizen::host().context("the host published no window")?;
        let (display, parent) = host_surfaces()?;

        // The renderer is leaked as the window is, so no drop destroys it either.
        std::mem::forget(canvas.texture_creator());
        // SAFETY: the canvas's live renderer and window.
        let surface = unsafe {
            sys::render::SDL_SetRenderVSync(canvas.raw(), 0);
            sys::properties::SDL_GetPointerProperty(
                sys::video::SDL_GetWindowProperties(canvas.window().raw()),
                sys::video::SDL_PROP_WINDOW_WAYLAND_SURFACE_POINTER,
                null_mut(),
            )
        };
        anyhow::ensure!(!surface.is_null(), "SDL's window has no wl_surface");

        let lib = open(c"libwayland-client.so.0")?;
        let interface = |name: &CStr| {
            address(lib, name)
                .map(|p| p.cast_const())
                .with_context(|| format!("{} is missing", name.to_string_lossy()))
        };
        let wl = Wayland {
            display,
            marshal: required(lib, c"wl_proxy_marshal")?,
            marshal_constructor: required(lib, c"wl_proxy_marshal_constructor")?,
            marshal_constructor_versioned: required(
                lib,
                c"wl_proxy_marshal_constructor_versioned",
            )?,
            proxy_destroy: required(lib, c"wl_proxy_destroy")?,
            flush: required(lib, c"wl_display_flush")?,
            get_error: required(lib, c"wl_display_get_error")?,
            region_interface: interface(c"wl_region_interface")?,
        };
        let create_queue: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
            required(lib, c"wl_display_create_queue")?;
        let create_wrapper: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
            required(lib, c"wl_proxy_create_wrapper")?;
        let set_queue: unsafe extern "C" fn(*mut c_void, *mut c_void) =
            required(lib, c"wl_proxy_set_queue")?;
        let add_listener: unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> c_int =
            required(lib, c"wl_proxy_add_listener")?;
        let roundtrip_queue: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
            required(lib, c"wl_display_roundtrip_queue")?;
        let registry_interface = interface(c"wl_registry_interface")?;
        let subsurface_interface = interface(c"wl_subsurface_interface")?;

        // wl_compositor and wl_subcompositor through a registry on a private queue, so nothing of
        // the host's is dispatched here. The queue, the wrapper and the registry are kept for the
        // life of the process, as the two globals bound on them are.
        let mut globals = Globals {
            wl: &wl,
            compositor_interface: interface(c"wl_compositor_interface")?,
            subcompositor_interface: interface(c"wl_subcompositor_interface")?,
            compositor: null_mut(),
            subcompositor: null_mut(),
        };
        // SAFETY: libwayland's documented calls on the host's display, on its UI thread; the
        // listener is static and `globals` outlives the round trip that fills it.
        unsafe {
            let queue = create_queue(display);
            let wrapper = create_wrapper(display);
            set_queue(wrapper, queue);
            let registry = (wl.marshal_constructor)(
                wrapper,
                DISPLAY_GET_REGISTRY,
                registry_interface,
                null_mut::<c_void>(),
            );
            anyhow::ensure!(!registry.is_null(), "no wl_registry");
            add_listener(
                registry,
                (&raw const REGISTRY_LISTENER).cast(),
                (&raw mut globals).cast(),
            );
            roundtrip_queue(display, queue);
        }
        let (compositor, subcompositor) = (globals.compositor, globals.subcompositor);
        anyhow::ensure!(
            !compositor.is_null() && !subcompositor.is_null(),
            "the compositor offers no wl_compositor or wl_subcompositor"
        );

        // SAFETY: the protocol header's inline requests, on proxies made above.
        let subsurface = unsafe {
            let sub = (wl.marshal_constructor)(
                subcompositor,
                SUBCOMPOSITOR_GET_SUBSURFACE,
                subsurface_interface,
                null_mut::<c_void>(),
                surface,
                parent,
            );
            anyhow::ensure!(!sub.is_null(), "no wl_subsurface");
            (wl.marshal)(sub, SUBSURFACE_SET_POSITION, 0_i32, 0_i32);
            (wl.marshal)(sub, SUBSURFACE_PLACE_ABOVE, parent);
            (wl.marshal)(sub, SUBSURFACE_SET_DESYNC);
            (wl.marshal)(parent, SURFACE_COMMIT);
            (wl.flush)(display);
            sub
        };

        // Ecore: the animator that paces the frames, Back withheld from the host, focus, and the
        // process being asked to exit.
        let core = open(c"libecore.so.1")?;
        let input = open(c"libecore_input.so.1")?;
        let ewl = open(c"libecore_wl2.so.1")?;
        let ecore = Ecore {
            iterate_may_block: required(core, c"ecore_main_loop_iterate_may_block")?,
            animator_del: required(core, c"ecore_animator_del")?,
            handler_del: required(core, c"ecore_event_handler_del")?,
        };
        let frametime_set: unsafe extern "C" fn(f64) =
            required(core, c"ecore_animator_frametime_set")?;
        let animator_add: unsafe extern "C" fn(TaskCb, *const c_void) -> *mut c_void =
            required(core, c"ecore_animator_add")?;
        let handler_add: HandlerAdd = required(core, c"ecore_event_handler_add")?;
        // Ahead of the host's handlers, so Back can be withheld from them.
        let prepend: Option<HandlerAdd> = function(core, c"ecore_event_handler_prepend");
        if prepend.is_none() {
            tracing::warn!("ecore_event_handler_prepend is absent; the host will see Back too");
        }
        let key_down = event_type(input, c"ECORE_EVENT_KEY_DOWN").context("no key-down event")?;
        let key_up = event_type(input, c"ECORE_EVENT_KEY_UP").context("no key-up event")?;
        let focus_in = event_type(ewl, c"ECORE_WL2_EVENT_FOCUS_IN");
        let focus_out = event_type(ewl, c"ECORE_WL2_EVENT_FOCUS_OUT");
        if focus_in.is_none() || focus_out.is_none() {
            tracing::warn!("Ecore_Wl2's focus events are missing; an overlay may keep the keys");
        }
        let signal_exit = event_type(core, c"ECORE_EVENT_SIGNAL_EXIT");

        TICK.store(false, Ordering::Release);
        FOCUS.store(0, Ordering::Release);
        QUIT.store(false, Ordering::Release);
        let mut handlers = Vec::new();
        // SAFETY: Ecore's documented calls, on its own thread, with static callbacks.
        let animator = unsafe {
            let add_key = prepend.unwrap_or(handler_add);
            handlers.push(add_key(key_down, on_key, std::ptr::null()));
            handlers.push(add_key(key_up, on_key, std::ptr::null()));
            if let Some(t) = focus_in {
                handlers.push(handler_add(t, on_focus_in, std::ptr::null()));
            }
            if let Some(t) = focus_out {
                handlers.push(handler_add(t, on_focus_out, std::ptr::null()));
            }
            if let Some(t) = signal_exit {
                handlers.push(handler_add(t, on_signal_exit, std::ptr::null()));
            }
            frametime_set(1.0 / 60.0);
            animator_add(on_animator, std::ptr::null())
        };
        handlers.retain(|h| !h.is_null());
        anyhow::ensure!(!animator.is_null(), "ecore_animator_add returned nothing");
        tracing::info!(
            "SDL's window is a subsurface of the host's; frames paced by an Ecore animator"
        );

        Ok(Self {
            ecore,
            wl,
            surface,
            parent,
            compositor,
            subsurface,
            width: host.width,
            height: host.height,
            animator,
            handlers,
            focus_in: true,
            opaque_at: Some(1),
            presented: 0,
            wayland_error: false,
        })
    }

    /// Lets Ecore run until the animator says the next frame is due. Returns `false` when the
    /// process was asked to exit, so the loop ends.
    #[expect(unsafe_code, reason = "Ecore's iterate, through a dlsym'd entry point")]
    pub(crate) fn wait(&mut self) -> bool {
        loop {
            // SAFETY: on the host's UI thread, which is the thread Ecore's main loop belongs to.
            unsafe { (self.ecore.iterate_may_block)(1) };
            if QUIT.load(Ordering::Acquire) {
                tracing::info!("Ecore says the process is exiting");
                return false;
            }
            if TICK.swap(false, Ordering::AcqRel) {
                break;
            }
        }
        match FOCUS.swap(0, Ordering::AcqRel) {
            -1 if self.focus_in => {
                // An overlay has focus. With the region cleared, the compositor gives it back.
                self.focus_in = false;
                self.opaque_at = None;
                self.set_opaque(false);
                tracing::debug!("focus out: the opaque region is cleared");
            }
            1 if !self.focus_in => {
                self.focus_in = true;
                self.opaque_at = Some(self.presented + 1);
                tracing::debug!("focus in: the opaque region returns a frame from now");
            }
            _ => {}
        }
        true
    }

    /// After each present: declares the window opaque once its first frame is up, and again a frame
    /// after focus returns. It takes effect on the next present.
    #[expect(
        unsafe_code,
        reason = "wl_display_get_error, through a dlsym'd entry point"
    )]
    pub(crate) fn presented(&mut self) {
        self.presented += 1;
        if self.focus_in && self.opaque_at.is_some_and(|at| self.presented >= at) {
            self.opaque_at = None;
            self.set_opaque(true);
        }
        if !self.wayland_error {
            // SAFETY: the host's live display.
            let error = unsafe { (self.wl.get_error)(self.wl.display) };
            if error != 0 {
                self.wayland_error = true;
                tracing::error!(error, "the Wayland connection has failed");
            }
        }
    }

    #[expect(
        unsafe_code,
        reason = "wl_surface.set_opaque_region, from the protocol header"
    )]
    fn set_opaque(&self, opaque: bool) {
        let wl = &self.wl;
        // SAFETY: the protocol header's inline requests on SDL's live surface and the compositor
        // bound in `attach`.
        unsafe {
            if opaque {
                let region = (wl.marshal_constructor)(
                    self.compositor,
                    COMPOSITOR_CREATE_REGION,
                    wl.region_interface,
                    null_mut::<c_void>(),
                );
                if region.is_null() {
                    return;
                }
                (wl.marshal)(region, REGION_ADD, 0_i32, 0_i32, self.width, self.height);
                (wl.marshal)(self.surface, SURFACE_SET_OPAQUE_REGION, region);
                (wl.marshal)(region, REGION_DESTROY);
                (wl.proxy_destroy)(region);
            } else {
                (wl.marshal)(
                    self.surface,
                    SURFACE_SET_OPAQUE_REGION,
                    null_mut::<c_void>(),
                );
            }
        }
    }
}

impl Drop for Frames {
    /// The measured ending: the handlers and the animator removed, the subsurface destroyed, and a
    /// null buffer committed, with SDL's window kept. The host's own surface shows again.
    #[expect(
        unsafe_code,
        reason = "Ecore and Wayland calls through dlsym'd entry points"
    )]
    fn drop(&mut self) {
        let wl = &self.wl;
        // SAFETY: everything here was made in `attach` and is released once, on the UI thread.
        unsafe {
            for &handler in &self.handlers {
                (self.ecore.handler_del)(handler);
            }
            (self.ecore.animator_del)(self.animator);
            (wl.marshal)(self.subsurface, SUBSURFACE_DESTROY);
            (wl.proxy_destroy)(self.subsurface);
            (wl.marshal)(
                self.surface,
                SURFACE_ATTACH,
                null_mut::<c_void>(),
                0_i32,
                0_i32,
            );
            (wl.marshal)(self.surface, SURFACE_COMMIT);
            (wl.marshal)(self.parent, SURFACE_COMMIT);
            (wl.flush)(wl.display);
        }
        tracing::info!("SDL's window is off the host's; it is kept, as SDL is");
    }
}
