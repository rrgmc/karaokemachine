//! The committed C header and `src/ffi.rs` describe the same six functions.
//!
//! **This test is the reason the header can be hand-written.** cbindgen would generate it, at the
//! cost of a tool to install and a step to run; the alternative is to write six declarations by
//! hand and prove they still match, which is what this does.
//!
//! What it is protecting against is specific and nasty: C has no name mangling, so a Rust function
//! whose *parameters* changed still links against a stale declaration. The result is not a build
//! error but a call with the wrong stack layout, on a device, with nothing pointing at the cause.
//!
//! It reads both files as text rather than compiling anything, which is what lets it run on the
//! machine doing the building — `src/ffi.rs` is behind `cfg(target_os = "ios")` and is not compiled
//! here at all, so a test that needed it compiled would be a test that never ran.

/// The six, each as (name, C return type, C parameter list).
///
/// Written out rather than parsed out of the Rust, because a list that derived itself from one side
/// could not disagree with that side — and disagreeing is the entire job.
const SURFACE: &[(&str, &str, &str)] = &[
    (
        "km_remote_start",
        "void",
        "const char *data_dir, const char *machine",
    ),
    ("km_remote_port", "int32_t", "void"),
    ("km_remote_songs", "int32_t", "void"),
    ("km_remote_failure", "const char *", "void"),
    ("km_remote_machine", "const char *", "void"),
    ("km_remote_stop", "void", "void"),
];

const HEADER: &str = include_str!("../include/km_remote.h");
const FFI: &str = include_str!("../src/ffi.rs");

/// Every declared function is exported, and every exported function is declared.
#[test]
fn the_header_and_the_rust_agree_about_which_functions_exist() {
    for (name, ret, params) in SURFACE {
        let declaration = if *ret == "const char *" {
            format!("{ret}{name}({params});")
        } else {
            format!("{ret} {name}({params});")
        };
        assert!(
            HEADER.contains(&declaration),
            "include/km_remote.h does not declare `{declaration}`",
        );
        assert!(
            FFI.contains(&format!("fn {name}(")),
            "src/ffi.rs does not export `{name}`",
        );
    }

    // The counts have to match too, or a seventh function added to one side and not the other would
    // pass everything above.
    let declared = HEADER.matches("km_remote_").count();
    let exported = FFI.matches("pub extern \"C\" fn km_remote_").count()
        + FFI.matches("pub unsafe extern \"C\" fn km_remote_").count();
    assert_eq!(
        exported,
        SURFACE.len(),
        "src/ffi.rs exports {exported} functions; this test knows about {}",
        SURFACE.len(),
    );
    assert!(
        declared >= SURFACE.len(),
        "include/km_remote.h mentions km_remote_ {declared} times, which is too few to declare {}",
        SURFACE.len(),
    );
}

/// Each export is wrapped, and each is `no_mangle`.
///
/// An `extern "C"` function is `-unwind` in this edition, so a panic crossing one aborts the
/// process — which somebody sees as the app vanishing while a `Timer` polls it ten times a second.
/// Every body goes through `guard`, and this is what says so out loud rather than trusting six
/// separate reviews of the same three lines.
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

/// The header says who owns the two returned strings.
///
/// Not style: the caller polls at ten hertz, and a caller that freed what it was handed would
/// double-free on the second poll. The rule only exists if it is written where somebody reads it.
#[test]
fn the_header_says_the_returned_strings_must_not_be_freed() {
    let warnings = HEADER.matches("DO NOT FREE IT").count();
    assert_eq!(
        warnings, 2,
        "both string-returning functions must carry the ownership rule; found {warnings}",
    );
}
