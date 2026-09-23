//! The console executable: `km-package-simple-console`.
//!
//! The same library with no window, so `--help`, the version and a refusal to start print where
//! somebody can read them. Built only with the `desktop` feature, because without it the other
//! executable already is this one.

fn main() -> anyhow::Result<()> {
    km_package_simple::run(km_package_simple::Shell::Console)
}
