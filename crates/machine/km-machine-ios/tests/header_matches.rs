//! The committed C header and `src/ffi.rs` describe the same two functions.
//!
//! **This test is the reason the header can be hand-written.** cbindgen would generate it, at the
//! cost of a tool to install and a step to run; the alternative is to write two declarations by
//! hand and prove they still match, which is what this does. `km-remote-ios` carries the same test
//! over six.
//!
//! What it is protecting against is specific and nasty: C has no name mangling, so a Rust function
//! whose *parameters* changed still links against a stale declaration. The result is not a build
//! error but a call with the wrong stack layout, on a device, with nothing pointing at the cause.
//!
//! It reads both files as text rather than compiling anything, which is what lets it run on the
//! machine doing the building — `src/ffi.rs` is behind `cfg(target_os = "ios")` and is not compiled
//! here at all, so a test that needed it compiled would be a test that never ran.

/// The two, each as (name, C return type, C parameter list).
///
/// Written out rather than parsed out of the Rust, because a list that derived itself from one side
/// could not disagree with that side — and disagreeing is the entire job.
const SURFACE: &[(&str, &str, &str)] = &[
    (
        "km_machine_configure",
        "void",
        "const char *support, const char *documents",
    ),
    ("SDL_main", "int", "int argc, char *argv[]"),
];

const HEADER: &str = include_str!("../include/km_machine.h");
const FFI: &str = include_str!("../src/ffi.rs");

/// Every declared function is exported, and every exported function is declared.
#[test]
fn the_header_and_the_rust_agree_about_which_functions_exist() {
    for (name, ret, params) in SURFACE {
        let declaration = format!("{ret} {name}({params});");
        assert!(
            HEADER.contains(&declaration),
            "include/km_machine.h does not declare `{declaration}`",
        );
        assert!(
            FFI.contains(&format!("fn {name}(")),
            "src/ffi.rs does not export `{name}`",
        );
    }

    // The counts have to match too, or a third function added to one side and not the other would
    // pass everything above.
    let exported = FFI.matches("pub extern \"C\" fn ").count()
        + FFI.matches("pub unsafe extern \"C\" fn ").count();
    assert_eq!(
        exported,
        SURFACE.len(),
        "src/ffi.rs exports {exported} functions; this test knows about {}",
        SURFACE.len(),
    );
}

/// Each export is wrapped, and each is `no_mangle`.
///
/// An `extern "C"` function is `-unwind` in this edition, so a panic crossing one aborts the
/// process — which somebody sees as the application vanishing with nothing written down. Every body
/// goes through `guard`, and this is what says so out loud rather than trusting a review of the
/// same three lines twice.
#[test]
fn every_export_is_no_mangle_and_catches_its_own_panics() {
    let exports = FFI.matches("#[unsafe(no_mangle)]").count();
    assert_eq!(
        exports,
        SURFACE.len(),
        "expected {} `#[unsafe(no_mangle)]` attributes, found {exports}",
        SURFACE.len(),
    );

    let guarded = FFI.matches("guard(").count();
    assert!(
        guarded >= SURFACE.len(),
        "only {guarded} uses of `guard`, which cannot cover {} exports",
        SURFACE.len(),
    );
}

/// The header says the caller creates the two directories.
///
/// Not style: this library creates neither, and a shell that assumed otherwise would hand down a
/// path that does not exist. The machine then writes nothing and says so only in a log nobody on a
/// phone is reading, so the rule has to be where the person writing the Swift reads it.
#[test]
fn the_header_says_who_creates_the_directories() {
    assert!(
        HEADER.contains("this library creates neither"),
        "include/km_machine.h must say that the caller creates both directories",
    );
}
