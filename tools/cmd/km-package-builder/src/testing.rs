//! What the tests in this crate need and the tool does not.
//!
//! **A test that touches a disk takes a [`Scratch`], never a fixed name in the temp folder.**
//! Several checkouts run this suite at once, and cargo runs its tests in parallel. A shared name
//! lets one run delete the database another run is reading. [`Scratch`] names its folder per
//! process and per call, and removes it when it drops.

pub use km_testkit::Scratch;
