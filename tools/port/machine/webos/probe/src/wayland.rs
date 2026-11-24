//! The `wl_registry` dump: every global the compositor advertises to this process, with its version.
//! libwayland-client is opened with `dlopen`, so the probe links against nothing the image lacks and
//! a set without it still gets a report.

use std::ffi::{CStr, CString, c_void};
use std::fmt::Write as _;
use std::os::raw::{c_char, c_int};

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;
/// `WL_DISPLAY_GET_REGISTRY`, the request's opcode on `wl_display`.
const GET_REGISTRY: u32 = 1;

#[repr(C)]
struct RegistryListener {
    global: unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32),
    global_remove: unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
}

unsafe extern "C" fn on_global(
    data: *mut c_void,
    _: *mut c_void,
    name: u32,
    iface: *const c_char,
    version: u32,
) {
    let out = unsafe { &mut *(data as *mut Vec<(String, u32, u32)>) };
    let iface = unsafe { CStr::from_ptr(iface) }
        .to_string_lossy()
        .into_owned();
    out.push((iface, version, name));
}

unsafe extern "C" fn on_global_remove(_: *mut c_void, _: *mut c_void, _: u32) {}

static LISTENER: RegistryListener = RegistryListener {
    global: on_global,
    global_remove: on_global_remove,
};

fn sym(lib: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    let c = CString::new(name).unwrap();
    let p = unsafe { dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        Err(format!("{name} not found"))
    } else {
        Ok(p)
    }
}

fn last_dl_error() -> String {
    let e = unsafe { dlerror() };
    if e.is_null() {
        "unknown".into()
    } else {
        unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned()
    }
}

/// The registry, one line per global, or why it could not be read.
pub fn registry() -> String {
    match dump() {
        Ok(s) => s,
        Err(e) => format!("FAILED: {e}\n"),
    }
}

fn dump() -> Result<String, String> {
    let lib = unsafe { dlopen(c"libwayland-client.so.0".as_ptr(), RTLD_NOW) };
    if lib.is_null() {
        return Err(format!("dlopen: {}", last_dl_error()));
    }
    type Connect = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type Marshal = unsafe extern "C" fn(*mut c_void, u32, *const c_void, ...) -> *mut c_void;
    type AddListener = unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> c_int;
    type Roundtrip = unsafe extern "C" fn(*mut c_void) -> c_int;
    type Disconnect = unsafe extern "C" fn(*mut c_void);
    let connect: Connect = unsafe { std::mem::transmute(sym(lib, "wl_display_connect")?) };
    let marshal: Marshal =
        unsafe { std::mem::transmute(sym(lib, "wl_proxy_marshal_constructor")?) };
    let add_listener: AddListener =
        unsafe { std::mem::transmute(sym(lib, "wl_proxy_add_listener")?) };
    let roundtrip: Roundtrip = unsafe { std::mem::transmute(sym(lib, "wl_display_roundtrip")?) };
    let disconnect: Disconnect = unsafe { std::mem::transmute(sym(lib, "wl_display_disconnect")?) };
    let registry_interface = sym(lib, "wl_registry_interface")?;

    let display = unsafe { connect(std::ptr::null()) };
    if display.is_null() {
        return Err(format!(
            "wl_display_connect(NULL) failed: {} (WAYLAND_DISPLAY={:?})",
            std::io::Error::last_os_error(),
            std::env::var("WAYLAND_DISPLAY").ok()
        ));
    }
    let registry = unsafe {
        marshal(
            display,
            GET_REGISTRY,
            registry_interface,
            std::ptr::null_mut::<c_void>(),
        )
    };
    if registry.is_null() {
        unsafe { disconnect(display) };
        return Err("wl_display_get_registry returned NULL".into());
    }
    let mut globals: Vec<(String, u32, u32)> = Vec::new();
    unsafe {
        add_listener(
            registry,
            &LISTENER as *const _ as *const c_void,
            &mut globals as *mut _ as *mut c_void,
        );
        roundtrip(display);
    }
    globals.sort();
    let mut out = String::new();
    for (iface, version, name) in &globals {
        let _ = writeln!(out, "{iface} v{version} (name {name})");
    }
    let _ = writeln!(out, "{} globals", globals.len());
    unsafe { disconnect(display) };
    Ok(out)
}
