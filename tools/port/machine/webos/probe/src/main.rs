//! The webOS probe: asks a webOS television, from inside an application's jail, the questions in
//! `docs/research/webos.md` §2 that can each end a port. See `docs/architecture/webos.md`.
//!
//! The launcher starts it with the launch parameters as a JSON string in `argv[1]`. It writes the
//! report and serves it on `0.0.0.0:8177`, or 8180 when the machine runs, `GET /report` and
//! `GET /log`. The parameters choose the rest: `"draw": 1` opens a window with a moving bar and logs frame gaps and keys (P2, P4),
//! `"sdl": 1` runs `km-sdl-test`, the same through SDL3, and copies its output into the log,
//! `"machine": 1` runs the machine itself the same way,
//! `"sound": "alsa"` or `"pulse"` plays a sine (P3), and `"secs"` (default 600) bounds them.
//! Standard library only, so the first question, whether Rust's prebuilt std loads here at all, is
//! not confused with any crate's.

mod draw;
mod sound;
mod wayland;

use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::raw::{c_char, c_double, c_int};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

unsafe extern "C" {
    fn gnu_get_libc_version() -> *const c_char;
    fn cos(x: c_double) -> c_double;
    fn getuid() -> u32;
    fn getgid() -> u32;
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
}

static LOG: Mutex<String> = Mutex::new(String::new());
static START: Mutex<Option<Instant>> = Mutex::new(None);

fn log(line: &str) {
    let t = START
        .lock()
        .unwrap()
        .map(|s| s.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    let line = format!("[{t:8.3}] {line}\n");
    eprint!("{line}");
    LOG.lock().unwrap().push_str(&line);
}

fn main() {
    *START.lock().unwrap() = Some(Instant::now());
    let args: Vec<String> = std::env::args().collect();
    log(&format!("started, argv = {args:?}"));
    let params = args.get(1).cloned().unwrap_or_default();
    let secs = param_u64(&params, "secs").unwrap_or(600);

    let report = report(&args);
    log("report written");
    for dir in writable_dirs() {
        let _ = std::fs::write(dir.join("km-probe-report.txt"), &report);
    }

    // The machine takes 8177 when it runs, so the probe moves aside.
    let machine = param_u64(&params, "machine") == Some(1);
    let port = if machine { 8180 } else { 8177 };
    std::thread::spawn(move || serve(report, port));

    let sound = param_str(&params, "sound");
    let sound_thread = sound.map(|backend| std::thread::spawn(move || sound::run(&backend, secs)));
    if param_u64(&params, "draw") == Some(1) {
        draw::run(secs);
    }
    if param_u64(&params, "sdl") == Some(1) {
        run_child("km-sdl-test", &[secs.to_string()], "sdl");
    }
    if machine {
        // `"package": "<file>"` installs a package carried beside the executable, as a
        // double-clicked .kmpkg arrives on a desktop.
        let mut args: Vec<String> = param_str(&params, "package")
            .map(|name| {
                let exe = std::fs::read_link("/proc/self/exe").unwrap_or_default();
                exe.with_file_name(name).display().to_string()
            })
            .into_iter()
            .collect();
        // `"frame_stats": 1` has the machine log its frame timing, which is how a video song's
        // decode is measured on a set nobody is watching.
        if param_u64(&params, "frame_stats") == Some(1) {
            args.push("--frame-stats".into());
        }
        run_child("karaokemachine", &args, "km");
    }
    if let Some(t) = sound_thread {
        let _ = t.join();
    }
    // Keep serving the log for a while after the work is done, so it can be read.
    let linger = if ["draw", "sdl", "machine"]
        .iter()
        .any(|k| param_u64(&params, k).is_some())
        || param_str(&params, "sound").is_some()
    {
        120
    } else {
        secs
    };
    log(&format!("work done; serving for {linger} s more"));
    std::thread::sleep(Duration::from_secs(linger));
    log("time is up, exiting");
}

/// Runs `name` from beside this executable and copies its output into the log, each line tagged.
fn run_child(name: &str, args: &[String], tag: &'static str) {
    use std::io::BufRead;
    let exe = std::fs::read_link("/proc/self/exe").unwrap_or_default();
    let test = exe.with_file_name(name);
    log(&format!("starting {}", test.display()));
    // Libraries carried in the application's own `lib/`, ahead of the set's: its ffmpeg 4.0
    // cannot decode H.264, so the machine's video build brings its own.
    let lib = exe.with_file_name("lib");
    let child = std::process::Command::new(&test)
        .args(args)
        .env("LD_LIBRARY_PATH", &lib)
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return log(&format!("{name} did not start: {e}")),
    };
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                log(&format!("{tag}: {line}"));
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
            log(&format!("{tag}: {line}"));
        }
    }
    log(&format!("{name} exited: {:?}", child.wait()));
}

/// A string out of the launch parameters, `{"sound": "alsa"}`.
fn param_str(params: &str, key: &str) -> Option<String> {
    let at = params.find(&format!("\"{key}\""))?;
    let rest = params[at + key.len() + 2..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

fn serve(report: String, port: u16) {
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("bind 0.0.0.0:{port} failed: {e}"));
            return;
        }
    };
    log(&format!("listening on 0.0.0.0:{port}"));
    loop {
        match listener.accept() {
            Ok((mut s, peer)) => {
                let _ = s.set_nonblocking(false);
                let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                log(&format!("GET {path} from {peer}"));
                let body = match path.as_str() {
                    "/log" => LOG.lock().unwrap().clone(),
                    "/report" | "/" => report.clone(),
                    _ => "not found\n".to_string(),
                };
                let _ = write!(
                    s,
                    "HTTP/1.0 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// A number out of the launch parameters, `{"secs": 30}`, without a JSON parser.
fn param_u64(params: &str, key: &str) -> Option<u64> {
    let at = params.find(&format!("\"{key}\""))?;
    let rest = &params[at + key.len() + 2..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn report(args: &[String]) -> String {
    let mut r = String::new();
    let exe = std::fs::read_link("/proc/self/exe").unwrap_or_default();
    let _ = writeln!(r, "# km-webos-probe report\n");
    let _ = writeln!(r, "exe: {}", exe.display());
    let _ = writeln!(r, "argv: {args:?}");
    let _ = writeln!(r, "cwd: {:?}", std::env::current_dir().ok());
    let _ = writeln!(r, "uid/gid: {}/{}", unsafe { getuid() }, unsafe {
        getgid()
    });
    let _ = writeln!(
        r,
        "time: {:?}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
    );

    let _ = writeln!(r, "\n## Loader and ABI");
    let _ = writeln!(r, "{}", elf_facts(&exe));
    let libc = unsafe { CStr::from_ptr(gnu_get_libc_version()) }
        .to_string_lossy()
        .into_owned();
    let _ = writeln!(r, "glibc: {libc}");
    // The float check crosses the C boundary: cos() is libm's, so a wrong calling convention
    // returns garbage rather than failing to load.
    let c = unsafe { cos(std::hint::black_box(1.0)) };
    let local = std::hint::black_box(2.0f64).sqrt() * std::f64::consts::PI;
    let _ = writeln!(
        r,
        "float: libm cos(1) = {c:.15} ({}), sqrt(2)*pi = {local:.15} ({})",
        ok((c - 0.540_302_305_868_14).abs() < 1e-12),
        ok((local - 4.442_882_938_158_366).abs() < 1e-12)
    );

    let _ = writeln!(r, "\n## Kernel");
    for f in ["ostype", "osrelease", "version"] {
        let _ = writeln!(r, "{f}: {}", read(&format!("/proc/sys/kernel/{f}")).trim());
    }
    let cpu = read("/proc/cpuinfo");
    for key in ["model name", "Features", "CPU part"] {
        if let Some(l) = cpu.lines().find(|l| l.starts_with(key)) {
            let _ = writeln!(r, "{l}");
        }
    }
    let _ = writeln!(
        r,
        "cores: {}",
        cpu.lines().filter(|l| l.starts_with("processor")).count()
    );

    let _ = writeln!(r, "\n## Memory");
    for l in read("/proc/meminfo").lines().take(5) {
        let _ = writeln!(r, "{l}");
    }
    for l in read("/proc/self/status").lines() {
        if ["VmRSS", "Groups", "Cap", "Seccomp", "NoNewPrivs"]
            .iter()
            .any(|k| l.starts_with(k))
        {
            let _ = writeln!(r, "{l}");
        }
    }

    let _ = writeln!(r, "\n## Environment");
    let mut env: Vec<_> = std::env::vars().collect();
    env.sort();
    for (k, v) in env {
        let _ = writeln!(r, "{k}={v}");
    }

    let _ = writeln!(r, "\n## Where it can write");
    let home = std::env::var("HOME").unwrap_or_default();
    let xdg = std::env::var("XDG_DATA_HOME").unwrap_or_default();
    let app = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut dirs: Vec<PathBuf> = vec![app.clone(), PathBuf::from(&home), PathBuf::from(&xdg)];
    for d in [
        "/tmp",
        "/var/lib/webosbrew",
        "/media/developer",
        "/media/internal",
        "/media/developer/temp",
        "/var/tmp",
        "/home/root",
    ] {
        dirs.push(PathBuf::from(d));
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    for d in dirs.iter().filter(|d| !d.as_os_str().is_empty()) {
        let marker = d.join("km-probe-marker");
        let before = std::fs::read_to_string(&marker).ok();
        let stamp = format!(
            "{:?}\n",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
        );
        let wrote = std::fs::write(&marker, stamp);
        let _ = writeln!(
            r,
            "{}: write {}; earlier marker {}",
            d.display(),
            wrote
                .map(|_| "ok".to_string())
                .unwrap_or_else(|e| format!("FAILED ({e})")),
            before
                .map(|s| format!("found, written at {}", s.trim()))
                .unwrap_or_else(|| "none".into())
        );
    }

    let _ = writeln!(r, "\n## Mounts for the application folder, /tmp and /media");
    for l in read("/proc/self/mountinfo").lines() {
        let mp = l.split(' ').nth(4).unwrap_or("");
        if mp == "/"
            || mp.starts_with("/tmp")
            || mp.starts_with("/media")
            || (!mp.is_empty() && app.starts_with(mp) && mp != "/")
        {
            let _ = writeln!(r, "{l}");
        }
    }

    let _ = writeln!(r, "\n## Devices");
    let _ = writeln!(r, "/tmp/usb: {}", list("/tmp/usb"));
    let _ = writeln!(r, "/dev/dri: {}", list("/dev/dri"));
    for e in std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = e.path();
        let _ = writeln!(r, "  open {} rw: {}", p.display(), try_open(&p));
    }
    let _ = writeln!(r, "/dev/snd: {}", list("/dev/snd"));
    let rt = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp/xdg".into());
    let _ = writeln!(r, "{rt}: {}", list(&rt));

    let _ = writeln!(r, "\n## Wayland globals");
    r.push_str(&wayland::registry());

    let _ = writeln!(r, "\n## Libraries");
    for lib in [
        "libwayland-client.so.0",
        "libwayland-egl.so.1",
        "libEGL.so.1",
        "libGLESv2.so.2",
        "libasound.so.2",
        "libpulse.so.0",
        "libpulse-simple.so.0",
        "libavcodec.so.58",
        "libavformat.so.58",
        "libSDL2-2.0.so.0",
        "libdrm.so.2",
        "libgbm.so.1",
        "libxkbcommon.so.0",
        "libfreetype.so.6",
        "libudev.so.1",
        "libstdc++.so.6",
    ] {
        let found = ["/usr/lib", "/lib"]
            .iter()
            .map(|d| format!("{d}/{lib}"))
            .find(|p| Path::new(p).exists());
        let _ = writeln!(r, "{lib}: {}", found.unwrap_or_else(|| "absent".into()));
    }
    r
}

fn ok(b: bool) -> &'static str {
    if b { "right" } else { "WRONG" }
}

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| format!("({e})"))
}

fn list(p: &str) -> String {
    match std::fs::read_dir(p) {
        Ok(it) => {
            let mut v: Vec<String> = it
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            format!("{v:?}")
        }
        Err(e) => format!("({e})"),
    }
}

fn try_open(p: &Path) -> String {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();
    let fd = unsafe {
        open(c.as_ptr(), 2 /* O_RDWR */)
    };
    if fd < 0 {
        format!("FAILED ({})", std::io::Error::last_os_error())
    } else {
        unsafe { close(fd) };
        "ok".into()
    }
}

/// The ELF interpreter and the ARM float-ABI flag of the running executable.
fn elf_facts(exe: &Path) -> String {
    let Ok(b) = std::fs::read(exe) else {
        return "could not read own executable".into();
    };
    if b.len() < 52 || &b[..4] != b"\x7fELF" || b[4] != 1 {
        return "not a 32-bit ELF".into();
    }
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
    let flags = u32_at(36);
    let abi = match flags & 0x600 {
        0x400 => "hard-float",
        0x200 => "soft/softfp",
        _ => "unmarked",
    };
    let (phoff, phentsize, phnum) = (u32_at(28), u16_at(42), u16_at(44));
    let mut interp = "none".to_string();
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if u32_at(ph) == 3 {
            let (off, size) = (u32_at(ph + 4), u32_at(ph + 16));
            interp = String::from_utf8_lossy(&b[off..off + size])
                .trim_end_matches('\0')
                .to_string();
        }
    }
    let present = Path::new(&interp).exists();
    format!("interpreter: {interp} (present: {present})\ne_flags: {flags:#x}, float ABI {abi}")
}

/// Every directory the report found writable, for a copy of the report on disk.
fn writable_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(app) = std::fs::read_link("/proc/self/exe")
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        v.push(app);
    }
    v.push(PathBuf::from("/tmp"));
    v.into_iter()
        .filter(|d| std::fs::metadata(d.join("km-probe-marker")).is_ok())
        .collect()
}
