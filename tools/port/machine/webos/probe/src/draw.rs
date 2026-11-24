//! P2 and P4: a window of the probe's own, drawn with EGL and GLES2, with the remote's keys logged.
//!
//! The set's compositor advertises `wl_shell` and `wl_webos_shell` and no `xdg_wm_base`, so this does
//! what webosbrew's SDL2 fork does: a `wl_shell` toplevel, a `wl_webos_shell_surface` on the same
//! surface carrying the application id, and the fullscreen state asked of the webOS one. It is the
//! surface setup an SDL3 patch would carry, tried without SDL first. Every library is opened with
//! `dlopen`, so the image needs no Wayland, EGL or GLES development files.

use std::ffi::{CStr, CString, c_void};
use std::os::raw::{c_char, c_int};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::{Duration, Instant};

use crate::log;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
unsafe extern "C" {
    fn close(fd: c_int) -> c_int;
}

#[repr(C)]
pub struct WlMessage {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const WlInterface,
}

#[repr(C)]
pub struct WlInterface {
    name: *const c_char,
    version: c_int,
    method_count: c_int,
    methods: *const WlMessage,
    event_count: c_int,
    events: *const WlMessage,
}

static QUIT: AtomicBool = AtomicBool::new(false);
static KEYBOARD_IFACE: AtomicPtr<WlInterface> = AtomicPtr::new(std::ptr::null_mut());
static POINTER_IFACE: AtomicPtr<WlInterface> = AtomicPtr::new(std::ptr::null_mut());
static MARSHAL_CTOR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static MARSHAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ADD_LISTENER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

type MarshalCtor = unsafe extern "C" fn(*mut c_void, u32, *const WlInterface, ...) -> *mut c_void;
type MarshalCtorV =
    unsafe extern "C" fn(*mut c_void, u32, *const WlInterface, u32, ...) -> *mut c_void;
type Marshal = unsafe extern "C" fn(*mut c_void, u32, ...);
type AddListener = unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> c_int;

fn marshal_ctor() -> MarshalCtor {
    unsafe { std::mem::transmute(MARSHAL_CTOR.load(Ordering::Relaxed)) }
}
fn marshal() -> Marshal {
    unsafe { std::mem::transmute(MARSHAL.load(Ordering::Relaxed)) }
}
fn add_listener(proxy: *mut c_void, listener: *const c_void) {
    let f: AddListener = unsafe { std::mem::transmute(ADD_LISTENER.load(Ordering::Relaxed)) };
    unsafe { f(proxy, listener, std::ptr::null_mut()) };
}

struct Lib(*mut c_void, &'static str);

impl Lib {
    fn open(name: &'static str) -> Result<Self, String> {
        let c = CString::new(name).unwrap();
        let h = unsafe { dlopen(c.as_ptr(), 2) };
        if h.is_null() {
            let e = unsafe { CStr::from_ptr(dlerror()) }
                .to_string_lossy()
                .into_owned();
            return Err(format!("dlopen {name}: {e}"));
        }
        Ok(Lib(h, name))
    }
    fn sym<T: Copy>(&self, name: &str) -> Result<T, String> {
        let c = CString::new(name).unwrap();
        let p = unsafe { dlsym(self.0, c.as_ptr()) };
        if p.is_null() {
            return Err(format!("{name} not in {}", self.1));
        }
        Ok(unsafe { std::mem::transmute_copy(&p) })
    }
    fn ptr(&self, name: &str) -> Result<*mut c_void, String> {
        self.sym::<*mut c_void>(name)
    }
}

fn cs(s: &str) -> *const c_char {
    CString::new(s).unwrap().into_raw()
}

fn msg(name: &str, sig: &str, types: Vec<*const WlInterface>) -> WlMessage {
    WlMessage {
        name: cs(name),
        signature: cs(sig),
        types: Box::leak(types.into_boxed_slice()).as_ptr(),
    }
}

fn iface(name: &str, methods: Vec<WlMessage>, events: Vec<WlMessage>) -> *mut WlInterface {
    let (mc, ec) = (methods.len() as c_int, events.len() as c_int);
    Box::leak(Box::new(WlInterface {
        name: cs(name),
        version: 1,
        method_count: mc,
        methods: Box::leak(methods.into_boxed_slice()).as_ptr(),
        event_count: ec,
        events: Box::leak(events.into_boxed_slice()).as_ptr(),
    }))
}

/// `wl_webos_shell` and `wl_webos_shell_surface`, from webos-shell.xml in webosbrew/SDL-webOS.
fn webos_interfaces(wl_surface: *const WlInterface) -> (*mut WlInterface, *mut WlInterface) {
    let n = std::ptr::null::<WlInterface>();
    let surface = iface(
        "wl_webos_shell_surface",
        vec![
            msg("set_location_hint", "u", vec![n]),
            msg("set_state", "u", vec![n]),
            msg("set_property", "ss", vec![n, n]),
            msg("set_key_mask", "u", vec![n]),
        ],
        vec![
            msg("state_changed", "u", vec![n]),
            msg("position_changed", "ii", vec![n, n]),
            msg("close", "", vec![]),
            msg("exposed", "a", vec![n]),
            msg("state_about_to_change", "u", vec![n]),
        ],
    );
    let shell = iface(
        "wl_webos_shell",
        vec![
            msg("get_system_pip", "", vec![]),
            msg("get_shell_surface", "no", vec![surface, wl_surface]),
        ],
        vec![],
    );
    (shell, surface)
}

// -- listeners -------------------------------------------------------------------------------------

#[derive(Default)]
struct Globals {
    list: Vec<(String, u32, u32)>,
}

unsafe extern "C" fn reg_global(
    data: *mut c_void,
    _: *mut c_void,
    name: u32,
    iface: *const c_char,
    version: u32,
) {
    let g = unsafe { &mut *(data as *mut Globals) };
    g.list.push((
        unsafe { CStr::from_ptr(iface) }
            .to_string_lossy()
            .into_owned(),
        version,
        name,
    ));
}
unsafe extern "C" fn reg_remove(_: *mut c_void, _: *mut c_void, _: u32) {}
#[repr(C)]
struct RegistryListener(
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
);
static REGISTRY_LISTENER: RegistryListener = RegistryListener(reg_global, reg_remove);

unsafe extern "C" fn ss_ping(_: *mut c_void, ss: *mut c_void, serial: u32) {
    unsafe { marshal()(ss, 0, serial) };
}
unsafe extern "C" fn ss_configure(_: *mut c_void, _: *mut c_void, edges: u32, w: i32, h: i32) {
    log(&format!("wl_shell_surface configure edges={edges} {w}x{h}"));
}
unsafe extern "C" fn ss_popup_done(_: *mut c_void, _: *mut c_void) {}
#[repr(C)]
struct ShellSurfaceListener(
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, i32, i32),
    unsafe extern "C" fn(*mut c_void, *mut c_void),
);
static SHELL_SURFACE_LISTENER: ShellSurfaceListener =
    ShellSurfaceListener(ss_ping, ss_configure, ss_popup_done);

unsafe extern "C" fn ws_state(_: *mut c_void, _: *mut c_void, state: u32) {
    log(&format!(
        "webos state_changed {state} (3 = fullscreen, 1 = minimized)"
    ));
}
unsafe extern "C" fn ws_position(_: *mut c_void, _: *mut c_void, x: i32, y: i32) {
    log(&format!("webos position_changed {x},{y}"));
}
unsafe extern "C" fn ws_close(_: *mut c_void, _: *mut c_void) {
    log("webos close");
    QUIT.store(true, Ordering::Relaxed);
}
unsafe extern "C" fn ws_exposed(_: *mut c_void, _: *mut c_void, _: *mut c_void) {
    log("webos exposed");
}
unsafe extern "C" fn ws_about(_: *mut c_void, _: *mut c_void, state: u32) {
    log(&format!("webos state_about_to_change {state}"));
}
#[repr(C)]
struct WebosSurfaceListener(
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, i32, i32),
    unsafe extern "C" fn(*mut c_void, *mut c_void),
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
);
static WEBOS_SURFACE_LISTENER: WebosSurfaceListener =
    WebosSurfaceListener(ws_state, ws_position, ws_close, ws_exposed, ws_about);

unsafe extern "C" fn seat_caps(_: *mut c_void, seat: *mut c_void, caps: u32) {
    log(&format!(
        "seat {seat:p} capabilities {caps:#x} (1 pointer, 2 keyboard, 4 touch)"
    ));
    if caps & 1 != 0 {
        let p = unsafe {
            marshal_ctor()(
                seat,
                0,
                POINTER_IFACE.load(Ordering::Relaxed),
                std::ptr::null_mut::<c_void>(),
            )
        };
        add_listener(p, &POINTER_LISTENER as *const _ as *const c_void);
    }
    if caps & 2 != 0 {
        let kb = unsafe {
            marshal_ctor()(
                seat,
                1,
                KEYBOARD_IFACE.load(Ordering::Relaxed),
                std::ptr::null_mut::<c_void>(),
            )
        };
        add_listener(kb, &KEYBOARD_LISTENER as *const _ as *const c_void);
    }
}
unsafe extern "C" fn seat_name(_: *mut c_void, seat: *mut c_void, name: *const c_char) {
    log(&format!(
        "seat {seat:p} name {}",
        unsafe { CStr::from_ptr(name) }.to_string_lossy()
    ));
}
#[repr(C)]
struct SeatListener(
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char),
);
static SEAT_LISTENER: SeatListener = SeatListener(seat_caps, seat_name);

unsafe extern "C" fn p_enter(
    _: *mut c_void,
    p: *mut c_void,
    _: u32,
    _: *mut c_void,
    x: i32,
    y: i32,
) {
    log(&format!("pointer {p:p} enter at {},{}", x / 256, y / 256));
}
unsafe extern "C" fn p_leave(_: *mut c_void, p: *mut c_void, _: u32, _: *mut c_void) {
    log(&format!("pointer {p:p} leave"));
}
unsafe extern "C" fn p_motion(_: *mut c_void, _: *mut c_void, _: u32, _: i32, _: i32) {}
unsafe extern "C" fn p_button(
    _: *mut c_void,
    p: *mut c_void,
    _: u32,
    _: u32,
    button: u32,
    state: u32,
) {
    log(&format!(
        "BUTTON {button:#x} {} on pointer {p:p}",
        if state == 1 { "down" } else { "up" }
    ));
}
unsafe extern "C" fn p_axis(_: *mut c_void, _: *mut c_void, _: u32, axis: u32, value: i32) {
    log(&format!("AXIS {axis} {}", value / 256));
}
#[repr(C)]
struct PointerListener(
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void, i32, i32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, i32, i32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, u32, u32, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, u32, i32),
);
static POINTER_LISTENER: PointerListener =
    PointerListener(p_enter, p_leave, p_motion, p_button, p_axis);

unsafe extern "C" fn kb_keymap(_: *mut c_void, _: *mut c_void, format: u32, fd: i32, size: u32) {
    log(&format!("keyboard keymap format {format}, {size} bytes"));
    unsafe { close(fd) };
}
unsafe extern "C" fn kb_enter(
    _: *mut c_void,
    kb: *mut c_void,
    _: u32,
    surface: *mut c_void,
    _: *mut c_void,
) {
    log(&format!("keyboard {kb:p} enter surface {surface:p}"));
}
unsafe extern "C" fn kb_leave(_: *mut c_void, kb: *mut c_void, _: u32, surface: *mut c_void) {
    log(&format!("keyboard {kb:p} leave surface {surface:p}"));
}
unsafe extern "C" fn kb_key(_: *mut c_void, kb: *mut c_void, _: u32, _: u32, key: u32, state: u32) {
    log(&format!(
        "KEY {key} (evdev; xkb {}) {} on {kb:p}",
        key + 8,
        if state == 1 { "down" } else { "up" }
    ));
}
unsafe extern "C" fn kb_mods(
    _: *mut c_void,
    _: *mut c_void,
    _: u32,
    _: u32,
    _: u32,
    _: u32,
    _: u32,
) {
}
unsafe extern "C" fn kb_repeat(_: *mut c_void, _: *mut c_void, _: i32, _: i32) {}
#[repr(C)]
struct KeyboardListener(
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, i32, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *mut c_void),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, u32, u32, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, u32, u32, u32, u32),
    unsafe extern "C" fn(*mut c_void, *mut c_void, i32, i32),
);
static KEYBOARD_LISTENER: KeyboardListener =
    KeyboardListener(kb_keymap, kb_enter, kb_leave, kb_key, kb_mods, kb_repeat);

// -- the run ---------------------------------------------------------------------------------------

const W: i32 = 1920;
const H: i32 = 1080;

/// Draws a moving bar for `secs`, logging frame gaps once every five seconds and every key.
pub fn run(secs: u64) {
    if let Err(e) = try_run(secs) {
        log(&format!("draw FAILED: {e}"));
    }
}

fn try_run(secs: u64) -> Result<(), String> {
    let wl = Lib::open("libwayland-client.so.0")?;
    let wegl = Lib::open("libwayland-egl.so.1")?;
    let egl = Lib::open("libEGL.so.1")?;
    let gles = Lib::open("libGLESv2.so.2")?;

    MARSHAL_CTOR.store(wl.ptr("wl_proxy_marshal_constructor")?, Ordering::Relaxed);
    MARSHAL.store(wl.ptr("wl_proxy_marshal")?, Ordering::Relaxed);
    ADD_LISTENER.store(wl.ptr("wl_proxy_add_listener")?, Ordering::Relaxed);
    KEYBOARD_IFACE.store(
        wl.ptr("wl_keyboard_interface")? as *mut WlInterface,
        Ordering::Relaxed,
    );
    POINTER_IFACE.store(
        wl.ptr("wl_pointer_interface")? as *mut WlInterface,
        Ordering::Relaxed,
    );
    let ctor_v: MarshalCtorV = wl.sym("wl_proxy_marshal_constructor_versioned")?;
    let connect: unsafe extern "C" fn(*const c_char) -> *mut c_void =
        wl.sym("wl_display_connect")?;
    let roundtrip: unsafe extern "C" fn(*mut c_void) -> c_int = wl.sym("wl_display_roundtrip")?;
    let dispatch_pending: unsafe extern "C" fn(*mut c_void) -> c_int =
        wl.sym("wl_display_dispatch_pending")?;
    let flush: unsafe extern "C" fn(*mut c_void) -> c_int = wl.sym("wl_display_flush")?;
    let i_registry = wl.ptr("wl_registry_interface")? as *const WlInterface;
    let i_compositor = wl.ptr("wl_compositor_interface")? as *const WlInterface;
    let i_surface = wl.ptr("wl_surface_interface")? as *const WlInterface;
    let i_shell = wl.ptr("wl_shell_interface")? as *const WlInterface;
    let i_shell_surface = wl.ptr("wl_shell_surface_interface")? as *const WlInterface;
    let i_seat = wl.ptr("wl_seat_interface")? as *const WlInterface;
    let (i_webos_shell, i_webos_surface) = webos_interfaces(i_surface);

    let display = unsafe { connect(std::ptr::null()) };
    if display.is_null() {
        return Err("wl_display_connect failed".into());
    }
    let registry =
        unsafe { marshal_ctor()(display, 1, i_registry, std::ptr::null_mut::<c_void>()) };
    let mut globals = Globals::default();
    let f: AddListener = unsafe { std::mem::transmute(ADD_LISTENER.load(Ordering::Relaxed)) };
    unsafe {
        f(
            registry,
            &REGISTRY_LISTENER as *const _ as *const c_void,
            &mut globals as *mut _ as *mut c_void,
        )
    };
    unsafe { roundtrip(display) };

    let bind = |want: &str, iface: *const WlInterface, max: u32| -> Vec<*mut c_void> {
        globals
            .list
            .iter()
            .filter(|(n, _, _)| n == want)
            .map(|(_, v, name)| {
                let v = (*v).min(max);
                let iname = unsafe { (*iface).name };
                unsafe {
                    ctor_v(
                        registry,
                        0,
                        iface,
                        v,
                        *name,
                        iname,
                        v,
                        std::ptr::null_mut::<c_void>(),
                    )
                }
            })
            .collect()
    };
    let compositor = *bind("wl_compositor", i_compositor, 3)
        .first()
        .ok_or("no wl_compositor")?;
    let shell = *bind("wl_shell", i_shell, 1).first().ok_or("no wl_shell")?;
    let webos_shell = *bind("wl_webos_shell", i_webos_shell, 1)
        .first()
        .ok_or("no wl_webos_shell")?;
    for seat in bind("wl_seat", i_seat, 3) {
        add_listener(seat, &SEAT_LISTENER as *const _ as *const c_void);
    }

    let surface =
        unsafe { marshal_ctor()(compositor, 0, i_surface, std::ptr::null_mut::<c_void>()) };
    let ss = unsafe {
        marshal_ctor()(
            shell,
            0,
            i_shell_surface,
            std::ptr::null_mut::<c_void>(),
            surface,
        )
    };
    add_listener(ss, &SHELL_SURFACE_LISTENER as *const _ as *const c_void);
    unsafe { marshal()(ss, 9, c"km-probe".as_ptr()) }; // set_class
    unsafe { marshal()(ss, 3) }; // set_toplevel
    let ws = unsafe {
        marshal_ctor()(
            webos_shell,
            1,
            i_webos_surface,
            std::ptr::null_mut::<c_void>(),
            surface,
        )
    };
    add_listener(ws, &WEBOS_SURFACE_LISTENER as *const _ as *const c_void);
    let appid = CString::new(std::env::var("APPID").unwrap_or_default()).unwrap();
    unsafe { marshal()(ws, 2, c"appId".as_ptr(), appid.as_ptr()) };
    unsafe {
        marshal()(
            ws,
            2,
            c"_WEBOS_ACCESS_POLICY_KEYS_BACK".as_ptr(),
            c"true".as_ptr(),
        )
    };
    unsafe { marshal()(ws, 1, 3u32) }; // set_state fullscreen
    unsafe { roundtrip(display) };
    log("surface created: wl_shell toplevel + wl_webos_shell_surface, fullscreen asked");

    // EGL
    let egl_window_create: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> *mut c_void =
        wegl.sym("wl_egl_window_create")?;
    let get_display: unsafe extern "C" fn(*mut c_void) -> *mut c_void = egl.sym("eglGetDisplay")?;
    let initialize: unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int) -> c_int =
        egl.sym("eglInitialize")?;
    let bind_api: unsafe extern "C" fn(u32) -> c_int = egl.sym("eglBindAPI")?;
    let choose: unsafe extern "C" fn(
        *mut c_void,
        *const c_int,
        *mut *mut c_void,
        c_int,
        *mut c_int,
    ) -> c_int = egl.sym("eglChooseConfig")?;
    let create_ctx: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const c_int,
    ) -> *mut c_void = egl.sym("eglCreateContext")?;
    let create_surf: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const c_int,
    ) -> *mut c_void = egl.sym("eglCreateWindowSurface")?;
    let make_current: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> c_int = egl.sym("eglMakeCurrent")?;
    let swap_interval: unsafe extern "C" fn(*mut c_void, c_int) -> c_int =
        egl.sym("eglSwapInterval")?;
    let swap: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int =
        egl.sym("eglSwapBuffers")?;
    let get_error: unsafe extern "C" fn() -> c_int = egl.sym("eglGetError")?;
    let query_string: unsafe extern "C" fn(*mut c_void, c_int) -> *const c_char =
        egl.sym("eglQueryString")?;
    let clear_color: unsafe extern "C" fn(f32, f32, f32, f32) = gles.sym("glClearColor")?;
    let clear: unsafe extern "C" fn(u32) = gles.sym("glClear")?;
    let enable: unsafe extern "C" fn(u32) = gles.sym("glEnable")?;
    let disable: unsafe extern "C" fn(u32) = gles.sym("glDisable")?;
    let scissor: unsafe extern "C" fn(c_int, c_int, c_int, c_int) = gles.sym("glScissor")?;
    let viewport: unsafe extern "C" fn(c_int, c_int, c_int, c_int) = gles.sym("glViewport")?;
    let get_string: unsafe extern "C" fn(u32) -> *const c_char = gles.sym("glGetString")?;

    let ew = unsafe { egl_window_create(surface, W, H) };
    let dpy = unsafe { get_display(display) };
    let (mut major, mut minor) = (0, 0);
    if unsafe { initialize(dpy, &mut major, &mut minor) } == 0 {
        return Err(format!("eglInitialize failed: {:#x}", unsafe {
            get_error()
        }));
    }
    let vendor = unsafe { CStr::from_ptr(query_string(dpy, 0x3053)) }
        .to_string_lossy()
        .into_owned();
    log(&format!("EGL {major}.{minor}, vendor {vendor}"));
    unsafe { bind_api(0x30A0) };
    let attribs = [
        0x3033, 4, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3040, 4, 0x3038,
    ];
    let mut cfg = std::ptr::null_mut();
    let mut n = 0;
    if unsafe { choose(dpy, attribs.as_ptr(), &mut cfg, 1, &mut n) } == 0 || n == 0 {
        return Err(format!("eglChooseConfig found nothing: {:#x}", unsafe {
            get_error()
        }));
    }
    let ctx_attribs = [0x3098, 2, 0x3038];
    let ctx = unsafe { create_ctx(dpy, cfg, std::ptr::null_mut(), ctx_attribs.as_ptr()) };
    let esurf = unsafe { create_surf(dpy, cfg, ew, std::ptr::null()) };
    if ctx.is_null() || esurf.is_null() {
        return Err(format!("context or surface failed: {:#x}", unsafe {
            get_error()
        }));
    }
    if unsafe { make_current(dpy, esurf, esurf, ctx) } == 0 {
        return Err(format!("eglMakeCurrent failed: {:#x}", unsafe {
            get_error()
        }));
    }
    unsafe { swap_interval(dpy, 1) };
    let renderer = unsafe { CStr::from_ptr(get_string(0x1F01)) }
        .to_string_lossy()
        .into_owned();
    let version = unsafe { CStr::from_ptr(get_string(0x1F02)) }
        .to_string_lossy()
        .into_owned();
    log(&format!("GL_RENDERER {renderer}, GL_VERSION {version}"));
    unsafe { viewport(0, 0, W, H) };

    let start = Instant::now();
    let until = start + Duration::from_secs(secs);
    let mut window_start = Instant::now();
    let mut last = Instant::now();
    let (mut frames, mut gaps, mut worst) = (0u32, 0u32, Duration::ZERO);
    let (mut total_frames, mut total_gaps) = (0u64, 0u64);
    while Instant::now() < until && !QUIT.load(Ordering::Relaxed) {
        let t = start.elapsed().as_secs_f32();
        let x = ((t * 480.0) as i32).rem_euclid(W - 64);
        unsafe {
            clear_color(0.05, 0.08, 0.20, 1.0);
            clear(0x4000);
            enable(0x0C11);
            scissor(x, 0, 64, H);
            clear_color(1.0, 0.85, 0.2, 1.0);
            clear(0x4000);
            disable(0x0C11);
            if swap(dpy, esurf) == 0 {
                return Err(format!("eglSwapBuffers failed: {:#x}", get_error()));
            }
            dispatch_pending(display);
            flush(display);
        }
        let now = Instant::now();
        let gap = now - last;
        last = now;
        frames += 1;
        total_frames += 1;
        if gap > Duration::from_millis(20) {
            gaps += 1;
            total_gaps += 1;
        }
        worst = worst.max(gap);
        if window_start.elapsed() >= Duration::from_secs(5) {
            let fps = frames as f32 / window_start.elapsed().as_secs_f32();
            log(&format!(
                "frames: {fps:.1} fps, {gaps} gaps over 20 ms, worst {:.1} ms",
                worst.as_secs_f32() * 1000.0
            ));
            window_start = Instant::now();
            frames = 0;
            gaps = 0;
            worst = Duration::ZERO;
            unsafe { roundtrip(display) };
        }
    }
    log(&format!(
        "draw done: {total_frames} frames in {:.1} s, {total_gaps} over 20 ms",
        start.elapsed().as_secs_f32()
    ));
    Ok(())
}
