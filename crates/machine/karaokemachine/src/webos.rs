//! The machine on an LG webOS television, as a native executable installed through Developer Mode.
//! Built by `tools/port/machine/webos/build.sh`; measured in `docs/research/webos.md` §13.
//!
//! Two things differ from desktop Linux, and both are here.
//!
//! **The window.** The set's compositor offers `wl_shell` and `wl_webos_shell`, and no
//! `xdg_wm_base`, so SDL3 cannot give a window a shell of its own. The window is made with a custom
//! surface role, and [`create_window`] gives SDL's `wl_surface` the roles webosbrew's SDL2 fork
//! gives it: a `wl_shell` toplevel, and a `wl_webos_shell_surface` carrying the application id and
//! asked to go fullscreen. The two shell interfaces come from `libwayland-client` through `dlopen`,
//! as SDL itself loads it, and the webOS ones are written out below from `webos-shell.xml`.
//!
//! **Back.** The remote's Back arrives as evdev `KEY_PREVIOUS`, 412, which SDL maps to no key at
//! all. [`remap_event`] turns it into `AC_BACK`, which the display already binds.

#![expect(
    unsafe_code,
    reason = "Wayland calls through dlopen, and SDL's window properties"
)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};

use anyhow::Context as _;
use sdl3::VideoSubsystem;
use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::sys;
use sdl3::video::Window;

/// What the set's 1080p UI plane is, and what the window is made at.
const WIDTH: i64 = 1920;
const HEIGHT: i64 = 1080;

/// The evdev code the remote's Back key arrives as.
const EVDEV_BACK: u16 = 412;

/// The command line without the launch parameters the launcher hands every application.
///
/// webOS starts a native application with its parameters as one JSON object in `argv[1]`, which no
/// argument of the machine's could accept; dropped, the rest parse as anywhere else.
pub(crate) fn launch_args(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Vec<std::ffi::OsString> {
    args.into_iter()
        .enumerate()
        .filter(|(i, a)| *i != 1 || !a.to_string_lossy().starts_with('{'))
        .map(|(_, a)| a)
        .collect()
}

/// Gives the remote's Back the `AC_BACK` keycode; every other event is passed through.
pub(crate) fn remap_event(event: Event) -> Event {
    match event {
        Event::KeyDown {
            timestamp,
            window_id,
            scancode,
            keymod,
            repeat,
            which,
            raw: EVDEV_BACK,
            ..
        } => Event::KeyDown {
            timestamp,
            window_id,
            keycode: Some(Keycode::AcBack),
            scancode,
            keymod,
            repeat,
            which,
            raw: EVDEV_BACK,
        },
        Event::KeyUp {
            timestamp,
            window_id,
            scancode,
            keymod,
            repeat,
            which,
            raw: EVDEV_BACK,
            ..
        } => Event::KeyUp {
            timestamp,
            window_id,
            keycode: Some(Keycode::AcBack),
            scancode,
            keymod,
            repeat,
            which,
            raw: EVDEV_BACK,
        },
        other => other,
    }
}

#[repr(C)]
struct WlMessage {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const WlInterface,
}

#[repr(C)]
struct WlInterface {
    name: *const c_char,
    version: c_int,
    method_count: c_int,
    methods: *const WlMessage,
    event_count: c_int,
    events: *const WlMessage,
}

type MarshalCtor = unsafe extern "C" fn(*mut c_void, u32, *const WlInterface, ...) -> *mut c_void;
type MarshalCtorV =
    unsafe extern "C" fn(*mut c_void, u32, *const WlInterface, u32, ...) -> *mut c_void;
type Marshal = unsafe extern "C" fn(*mut c_void, u32, ...);
type AddListener = unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> c_int;
type Roundtrip = unsafe extern "C" fn(*mut c_void) -> c_int;

static MARSHAL: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

fn symbol(lib: *mut c_void, name: &CStr) -> anyhow::Result<*mut c_void> {
    // SAFETY: `lib` is a handle dlopen returned and `name` is a C string.
    let p = unsafe { dlsym(lib, name.as_ptr()) };
    anyhow::ensure!(!p.is_null(), "libwayland-client has no {name:?}");
    Ok(p)
}

/// Leaks a C string for an interface table, which lives as long as the process.
fn leak(s: &str) -> *const c_char {
    CString::new(s).expect("no NUL").into_raw()
}

fn message(name: &str, signature: &str, types: Vec<*const WlInterface>) -> WlMessage {
    WlMessage {
        name: leak(name),
        signature: leak(signature),
        types: Box::leak(types.into_boxed_slice()).as_ptr(),
    }
}

fn interface(name: &str, methods: Vec<WlMessage>, events: Vec<WlMessage>) -> *const WlInterface {
    let (method_count, event_count) = (methods.len() as c_int, events.len() as c_int);
    Box::leak(Box::new(WlInterface {
        name: leak(name),
        version: 1,
        method_count,
        methods: Box::leak(methods.into_boxed_slice()).as_ptr(),
        event_count,
        events: Box::leak(events.into_boxed_slice()).as_ptr(),
    }))
}

/// `wl_webos_shell` and `wl_webos_shell_surface`, from `webos-shell.xml` in webosbrew/SDL-webOS.
fn webos_interfaces(wl_surface: *const WlInterface) -> (*const WlInterface, *const WlInterface) {
    let n = null::<WlInterface>();
    let surface = interface(
        "wl_webos_shell_surface",
        vec![
            message("set_location_hint", "u", vec![n]),
            message("set_state", "u", vec![n]),
            message("set_property", "ss", vec![n, n]),
            message("set_key_mask", "u", vec![n]),
        ],
        vec![
            message("state_changed", "u", vec![n]),
            message("position_changed", "ii", vec![n, n]),
            message("close", "", vec![]),
            message("exposed", "a", vec![n]),
            message("state_about_to_change", "u", vec![n]),
        ],
    );
    let shell = interface(
        "wl_webos_shell",
        vec![
            message("get_system_pip", "", vec![]),
            message("get_shell_surface", "no", vec![surface, wl_surface]),
        ],
        vec![],
    );
    (shell, surface)
}

#[derive(Default)]
struct Globals {
    shell: u32,
    webos_shell: u32,
}

extern "C" fn on_global(
    data: *mut c_void,
    _: *mut c_void,
    name: u32,
    iface: *const c_char,
    _: u32,
) {
    // SAFETY: `data` is the `Globals` this module passed, alive across the roundtrip, and `iface`
    // is a C string libwayland owns.
    let (globals, iface) = unsafe { (&mut *data.cast::<Globals>(), CStr::from_ptr(iface)) };
    match iface.to_bytes() {
        b"wl_shell" => globals.shell = name,
        b"wl_webos_shell" => globals.webos_shell = name,
        _ => {}
    }
}
extern "C" fn on_global_remove(_: *mut c_void, _: *mut c_void, _: u32) {}
static REGISTRY_LISTENER: [extern "C" fn(); 2] = [
    // SAFETY: listener tables are arrays of function pointers that libwayland calls with the
    // event's own signature, which is how C declares them too.
    unsafe {
        std::mem::transmute::<
            extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32),
            extern "C" fn(),
        >(on_global)
    },
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, u32), extern "C" fn()>(
            on_global_remove,
        )
    },
];

extern "C" fn on_ping(_: *mut c_void, shell_surface: *mut c_void, serial: u32) {
    // SAFETY: set before any listener is added; `pong` is opcode 0 of `wl_shell_surface`.
    let marshal =
        unsafe { std::mem::transmute::<*mut c_void, Marshal>(MARSHAL.load(Ordering::Acquire)) };
    unsafe { marshal(shell_surface, 0, serial) };
}
extern "C" fn on_configure(_: *mut c_void, _: *mut c_void, _: u32, w: i32, h: i32) {
    tracing::debug!(w, h, "wl_shell_surface configure");
}
extern "C" fn on_popup_done(_: *mut c_void, _: *mut c_void) {}
static SHELL_SURFACE_LISTENER: [extern "C" fn(); 3] = [
    // SAFETY: as for the registry listener.
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, u32), extern "C" fn()>(
            on_ping,
        )
    },
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, u32, i32, i32), extern "C" fn()>(
            on_configure,
        )
    },
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void), extern "C" fn()>(
            on_popup_done,
        )
    },
];

extern "C" fn on_state(_: *mut c_void, _: *mut c_void, state: u32) {
    tracing::info!(state, "webOS window state (3 fullscreen, 1 minimized)");
}
extern "C" fn on_position(_: *mut c_void, _: *mut c_void, _: i32, _: i32) {}
extern "C" fn on_close(_: *mut c_void, _: *mut c_void) {
    tracing::info!("the television asked the window to close");
    // SDL has no window-level request for a custom-role surface, so the request is SDL's own quit.
    // SAFETY: SDL_PushEvent copies the event.
    unsafe {
        let mut event: sys::events::SDL_Event = std::mem::zeroed();
        event.r#type = sys::events::SDL_EVENT_QUIT.into();
        sys::events::SDL_PushEvent(&mut event);
    }
}
extern "C" fn on_exposed(_: *mut c_void, _: *mut c_void, _: *mut c_void) {}
extern "C" fn on_about_to_change(_: *mut c_void, _: *mut c_void, _: u32) {}
static WEBOS_SURFACE_LISTENER: [extern "C" fn(); 5] = [
    // SAFETY: as for the registry listener.
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, u32), extern "C" fn()>(
            on_state,
        )
    },
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, i32, i32), extern "C" fn()>(
            on_position,
        )
    },
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void), extern "C" fn()>(on_close)
    },
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, *mut c_void), extern "C" fn()>(
            on_exposed,
        )
    },
    unsafe {
        std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, u32), extern "C" fn()>(
            on_about_to_change,
        )
    },
];

/// SDL's window, made with a custom surface role and given the webOS roles.
///
/// sdl3-rs builds windows from a title, a size and flags only, so the window is created from
/// properties here and wrapped, as the Tizen build does.
pub(crate) fn create_window(video: &VideoSubsystem, title: &str) -> anyhow::Result<Window> {
    use sys::properties::{
        SDL_CreateProperties, SDL_DestroyProperties, SDL_GetGlobalProperties,
        SDL_GetPointerProperty, SDL_SetBooleanProperty, SDL_SetNumberProperty,
        SDL_SetStringProperty,
    };
    use sys::video as v;
    let title = CString::new(title)?;
    // SAFETY: a fresh property set, filled and freed here; the window SDL returns is wrapped at once.
    let raw = unsafe {
        let props = SDL_CreateProperties();
        SDL_SetStringProperty(
            props,
            v::SDL_PROP_WINDOW_CREATE_TITLE_STRING,
            title.as_ptr(),
        );
        SDL_SetNumberProperty(props, v::SDL_PROP_WINDOW_CREATE_WIDTH_NUMBER, WIDTH);
        SDL_SetNumberProperty(props, v::SDL_PROP_WINDOW_CREATE_HEIGHT_NUMBER, HEIGHT);
        SDL_SetBooleanProperty(props, v::SDL_PROP_WINDOW_CREATE_OPENGL_BOOLEAN, true);
        SDL_SetBooleanProperty(props, v::SDL_PROP_WINDOW_CREATE_BORDERLESS_BOOLEAN, true);
        SDL_SetBooleanProperty(
            props,
            v::SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN,
            true,
        );
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
    // SAFETY: both are pointers SDL publishes and keeps alive with the video subsystem and window.
    let (display, surface) = unsafe {
        (
            SDL_GetPointerProperty(
                SDL_GetGlobalProperties(),
                v::SDL_PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER,
                null_mut(),
            ),
            SDL_GetPointerProperty(
                v::SDL_GetWindowProperties(raw),
                v::SDL_PROP_WINDOW_WAYLAND_SURFACE_POINTER,
                null_mut(),
            ),
        )
    };
    anyhow::ensure!(
        !display.is_null() && !surface.is_null(),
        "SDL is not on Wayland: no wl_display or wl_surface"
    );
    give_roles(display, surface).context("the webOS window roles")?;
    tracing::info!(
        driver = video.current_video_driver(),
        width = WIDTH,
        height = HEIGHT,
        "SDL's window, a wl_shell toplevel with a wl_webos_shell_surface"
    );
    Ok(window)
}

/// The `wl_shell` and `wl_webos_shell` roles on SDL's surface, fullscreen asked.
fn give_roles(display: *mut c_void, surface: *mut c_void) -> anyhow::Result<()> {
    // SAFETY: dlopen of a library SDL has already opened; the handle is never closed.
    let lib = unsafe { dlopen(c"libwayland-client.so.0".as_ptr(), 2) };
    if lib.is_null() {
        // SAFETY: dlerror returns a C string after a failed dlopen.
        let error = unsafe { CStr::from_ptr(dlerror()) }
            .to_string_lossy()
            .into_owned();
        anyhow::bail!("dlopen libwayland-client: {error}");
    }
    // SAFETY: each symbol is transmuted to the signature libwayland declares for it.
    let (marshal_ctor, marshal_ctor_v, marshal, add_listener, roundtrip): (
        MarshalCtor,
        MarshalCtorV,
        Marshal,
        AddListener,
        Roundtrip,
    ) = unsafe {
        (
            std::mem::transmute::<*mut c_void, MarshalCtor>(symbol(
                lib,
                c"wl_proxy_marshal_constructor",
            )?),
            std::mem::transmute::<*mut c_void, MarshalCtorV>(symbol(
                lib,
                c"wl_proxy_marshal_constructor_versioned",
            )?),
            std::mem::transmute::<*mut c_void, Marshal>(symbol(lib, c"wl_proxy_marshal")?),
            std::mem::transmute::<*mut c_void, AddListener>(symbol(lib, c"wl_proxy_add_listener")?),
            std::mem::transmute::<*mut c_void, Roundtrip>(symbol(lib, c"wl_display_roundtrip")?),
        )
    };
    MARSHAL.store(marshal as *mut c_void, Ordering::Release);
    let registry_iface = symbol(lib, c"wl_registry_interface")?.cast::<WlInterface>();
    let surface_iface = symbol(lib, c"wl_surface_interface")?.cast::<WlInterface>();
    let shell_iface = symbol(lib, c"wl_shell_interface")?.cast::<WlInterface>();
    let shell_surface_iface = symbol(lib, c"wl_shell_surface_interface")?.cast::<WlInterface>();
    let (webos_shell_iface, webos_surface_iface) = webos_interfaces(surface_iface);

    let mut globals = Globals::default();
    // SAFETY: Wayland requests by opcode, each with the arguments its signature names. `globals`
    // outlives the roundtrip that fills it, and the registry is never used after this function.
    unsafe {
        let registry = marshal_ctor(display, 1, registry_iface, null_mut::<c_void>());
        add_listener(
            registry,
            REGISTRY_LISTENER.as_ptr().cast(),
            (&raw mut globals).cast(),
        );
        roundtrip(display);
        anyhow::ensure!(
            globals.shell != 0 && globals.webos_shell != 0,
            "the compositor offers no wl_shell or no wl_webos_shell"
        );
        let shell = marshal_ctor_v(
            registry,
            0,
            shell_iface,
            1,
            globals.shell,
            (*shell_iface).name,
            1u32,
            null_mut::<c_void>(),
        );
        let webos_shell = marshal_ctor_v(
            registry,
            0,
            webos_shell_iface,
            1,
            globals.webos_shell,
            (*webos_shell_iface).name,
            1u32,
            null_mut::<c_void>(),
        );
        let shell_surface =
            marshal_ctor(shell, 0, shell_surface_iface, null_mut::<c_void>(), surface);
        add_listener(
            shell_surface,
            SHELL_SURFACE_LISTENER.as_ptr().cast(),
            null_mut(),
        );
        marshal(shell_surface, 9, c"karaokemachine".as_ptr()); // set_class
        marshal(shell_surface, 3); // set_toplevel
        let webos_surface = marshal_ctor(
            webos_shell,
            1,
            webos_surface_iface,
            null_mut::<c_void>(),
            surface,
        );
        add_listener(
            webos_surface,
            WEBOS_SURFACE_LISTENER.as_ptr().cast(),
            null_mut(),
        );
        let app_id = CString::new(std::env::var("APPID").unwrap_or_default())?;
        marshal(webos_surface, 2, c"appId".as_ptr(), app_id.as_ptr());
        // The remote's Back comes to the application rather than closing it.
        marshal(
            webos_surface,
            2,
            c"_WEBOS_ACCESS_POLICY_KEYS_BACK".as_ptr(),
            c"true".as_ptr(),
        );
        marshal(webos_surface, 1, 3u32); // set_state fullscreen
        roundtrip(display);
    }
    Ok(())
}
