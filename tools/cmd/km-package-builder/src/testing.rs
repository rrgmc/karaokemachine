//! What the tests in this crate need and the tool does not.
//!
//! One type so far, and it earns a module by having had **five** copies: four identical
//! `struct Scratch(PathBuf)` definitions in `backup`, `build`, `scan` and `server`, and a fifth
//! shape in `db` that was not a type at all — four tests there built a temp directory by hand,
//! under a **fixed name**, and removed it only on the way *in*.
//!
//! That last shape is worth a sentence, because it is the one that was actually wrong rather than
//! merely repeated. A fixed name is shared by every process that runs the suite, and this
//! repository routinely has three checkouts building at once, so two runs would land on the same
//! `km-package-builder-kind-migration` and one would delete the other's database mid-test. Removing
//! on the way in also means the directory is *always* left behind, which is how twenty of them came
//! to be sitting in the temp folder.

use std::path::PathBuf;

/// A scratch directory that removes itself.
///
/// The name carries the process id, the test's own name and the thread the test is running on —
/// enough that two suites in two checkouts cannot collide, and that a directory which does survive
/// says which test left it.
pub struct Scratch(pub PathBuf);

impl Scratch {
    /// Makes an empty directory for a test called `name`.
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "km-package-builder-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    /// Writes a fixture into it, making any folders on the way.
    pub fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("make the parent");
        }
        std::fs::write(path, bytes).expect("write the fixture");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
