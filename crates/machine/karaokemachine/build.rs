//! Puts the application icon inside the Windows executable.
//!
//! Windows reads an executable's icon from a resource compiled into the binary, not from a file
//! beside it, so this is the only way Explorer, the taskbar and the alt-tab switcher get one before
//! the process starts. `km_display::icon` covers the window itself, which is a different thing and
//! needs both: the resource is what a shortcut and a folder listing show, and the window icon is
//! what a running app shows.
//!
//! The version is picked up from `Cargo.toml` unaided. The two string fields are **not**, which is
//! only visible by looking: `winresource` defaults both `ProductName` and `FileDescription` to the
//! *crate name*, so unset the properties dialog reads "karaokemachine" twice over and says nothing
//! about what the program is.
//!
//! `ProductName` is the product, and is deliberately the same string in
//! `tools/cmd/km-package-builder/build.rs` — the machine and the curation tool are two programs in one
//! product, which is exactly the distinction Windows has these two fields for. `FileDescription`
//! is what tells them apart.
//!
//! **`FileDescription` is the program's name, spelled out here, and not the crate's `description`.**
//! Windows Firewall's "allow this app to communicate" dialog names an executable by that field, and
//! this is the one program in the workspace that raises it by default — so what it holds is read by
//! somebody deciding whether to let the machine onto their network. A Cargo `description` is a
//! sentence written for `cargo` and for docs.rs, and a sentence there reads as a fault in the
//! dialog rather than as a name. The string is the same one `km_display` puts on the idle screen
//! and the window title; a build script runs before its crate compiles and cannot read a constant
//! out of it, so this is one of the copies listed in
//! `What the tool calls itself` in `docs/decisions/foundations.md`.
//!
//! **`karaokemachine-console` shares it**, because a resource is attached per crate and not per
//! binary. Both executables are the machine, so one name for the two is the honest answer rather
//! than a limitation worked around.
//!
//! `LegalCopyright` is left empty. There is no `authors` field in the workspace to derive one from,
//! and a copyright line is a claim rather than a build detail — not something to invent here.
//!
//! `tools/cmd/km-package-builder/build.rs` and `crates/remote/km-remote/build.rs` are the same script for
//! the curation tool and the offline remote. Each attaches a **different** icon — the same
//! microphone in the theme's accent blue and its second accent green rather than this one's amber,
//! because all three are run side by side on one desktop and an identical icon there is an icon
//! doing no work. Their own headers have the reasoning. What still says they are one product is
//! `ProductName` above, which is the same string in all three and is exactly the field Windows keeps
//! for that. Change one script and look at the others: they are twenty lines each and they diverge
//! in one line.

fn main() {
    // Two different questions, and both guards are needed. **Whether `winresource` exists** is a
    // question about the host: cargo resolves `[target.'cfg(windows)'.build-dependencies]` against
    // the machine doing the building, so on Linux and macOS the crate is not in the build at all and
    // naming it is a compile error — which is exactly how CI went red while every Windows build
    // stayed green. **Whether to attach a resource** is a question about the target, because a
    // Windows host cross-compiling for Android has the crate available and must not run a resource
    // compiler over an ELF shared object.
    #[cfg(windows)]
    attach_icon();

    if std::env::var_os("CARGO_FEATURE_TIZEN_EMBED_ASSETS").is_some() {
        embed_tizen_assets();
    }
}

/// Writes `$OUT_DIR/tizen_assets.rs`: `CARRIED`, every file under `KM_TIZEN_ASSETS` as a path
/// relative to it and its bytes, for `src/tizen/entry.rs` to write out on the set.
///
/// Generated rather than one archive so that nothing has to be unpacked but files, and so that
/// cargo rebuilds when any one of them changes.
fn embed_tizen_assets() {
    use std::path::{Path, PathBuf};

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }

    println!("cargo:rerun-if-env-changed=KM_TIZEN_ASSETS");
    let root = PathBuf::from(std::env::var_os("KM_TIZEN_ASSETS").expect(
        "tizen-embed-assets needs KM_TIZEN_ASSETS, the staged asset folder; \
         tools/port/machine/tizen/build.sh sets it",
    ));
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();

    let mut code = String::from("static CARRIED: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let relative = file
            .strip_prefix(&root)
            .expect("under the root")
            .to_string_lossy()
            .replace('\\', "/");
        let absolute = file.canonicalize().expect("a staged file");
        code.push_str(&format!(
            "    ({relative:?}, include_bytes!({:?}) as &[u8]),\n",
            absolute.to_string_lossy()
        ));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("tizen_assets.rs"), code).expect("write tizen_assets.rs");
}

#[cfg(windows)]
fn attach_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=../../../icon/karaokemachine.ico");
    println!("cargo:rerun-if-changed=../../../icon/karaokemachine-stream.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../../icon/karaokemachine.ico");
    // **The second mark, for the notification area of a streaming run**, which `src/tray.rs` asks
    // for by this ordinal — change one and change the other. It is above the default rather than
    // below it because the Windows shell draws the *lowest* icon resource for the executable, and
    // what an Explorer window, a taskbar button and a Start Menu tile should show is the machine
    // rather than one way of starting it.
    resource.set_icon_with_id("../../../icon/karaokemachine-stream.ico", "2");
    resource.set("ProductName", "KaraokeMachine");
    // The same string as `ProductName`, and correct: this program *is* the product, where the three
    // beside it are named for the part of it they are.
    resource.set("FileDescription", "KaraokeMachine");
    if let Err(error) = resource.compile() {
        // A warning rather than a failure. Compiling a resource needs `rc.exe` from the Windows SDK,
        // and a machine can have the MSVC *linker* without it — this one nearly did. Refusing to
        // build the whole application because its icon could not be attached would be the wrong
        // trade by a wide margin; the build still produces a working executable, with the default
        // icon and a line saying why.
        println!("cargo:warning=could not attach the application icon: {error}");
        println!(
            "cargo:warning=the executable will build and run with Windows' default icon. This \
             needs rc.exe from the Windows SDK."
        );
    }
}
