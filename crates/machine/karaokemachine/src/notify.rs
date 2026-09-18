//! Telling systemd the machine has loaded.
//!
//! **This exists so that a boot splash can stay on the television until there is something to
//! replace it.** Plymouth holds DRM master while it draws, so it has to be gone before the display
//! can start — and Debian quits it at `multi-user.target`, which on the appliance is about four
//! seconds before the machine has a first frame. Four seconds of black, measured, on a set that had
//! just been showing the mark.
//!
//! The fix is ordering rather than timing: `plymouth-quit.service` is ordered *after* this service,
//! and this service is `Type=notify`, so "after" means after the machine says it is ready rather
//! than after the process has been forked. The splash then covers the SoundFont, the catalog and the
//! API, and goes at the last possible moment.
//!
//! **`READY=1` is sent before the display starts, not after**, and that is the whole subtlety. It
//! cannot be sent after: the display cannot start until Plymouth releases DRM master, and Plymouth
//! will not release it until this message arrives. Sending it here means the machine is announcing
//! *everything except the screen* — which is exactly the contract the appliance needs, because a
//! television that never comes on is a state the machine is designed to sit in and keep retrying.
//!
//! Written by hand rather than by taking `libsystemd`: the protocol is one datagram to one socket,
//! and the dependency would be a C library on the build of every platform that has no systemd.
//!
//! Everything here is a no-op off Linux and a no-op under any manager that sets no
//! `NOTIFY_SOCKET` — running the binary from a shell, from a desktop session, or under the tarball
//! carrier's own unit, which is `Type=simple`.

/// Tell the service manager the machine has finished loading.
///
/// Best effort and deliberately silent about not being under systemd, which is the ordinary case
/// everywhere except the appliance. A failure to *send* is logged at debug, because the consequence
/// is real but is not the machine's problem to solve: the unit's `TimeoutStartSec` bounds it, and
/// the appliance ends up doing what it did before this existed.
pub fn ready() {
    #[cfg(target_os = "linux")]
    {
        // Read here rather than inside `send`, so that `send` takes the socket as an argument and a
        // test can hand it one. The workspace denies `unsafe`, and the only other way to test this
        // would be `std::env::set_var`, which is unsafe in this edition and process-wide besides.
        let Some(socket) = std::env::var_os("NOTIFY_SOCKET") else {
            // Not under a service manager that wants telling. The overwhelmingly common case.
            return;
        };
        linux::send(std::path::Path::new(&socket), "READY=1");
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::unix::net::UnixDatagram;
    use std::path::Path;

    /// Whether `NOTIFY_SOCKET` names a socket in the abstract namespace rather than a file.
    ///
    /// **A named predicate rather than an `if` inside `send`, so that it can be tested.** systemd
    /// uses the abstract namespace when told to, and Rust cannot open one by path: `send_to` on a
    /// name beginning `@` looks for a *file* so called and fails. That failure is indistinguishable
    /// from any other — no file appears either way — so a test of `send`'s behaviour cannot tell
    /// whether this branch exists. Testing the decision is the only way to hold it.
    ///
    /// Skipped rather than supported because the filesystem socket is what systemd uses by default
    /// and what the appliance gets. Reported rather than ignored, because the consequence is a boot
    /// splash held until the unit's start timeout, with nothing anywhere saying why.
    pub fn is_abstract(socket: &Path) -> bool {
        socket.to_string_lossy().starts_with('@')
    }

    pub fn send(socket: &Path, message: &str) {
        if is_abstract(socket) {
            tracing::debug!(
                socket = %socket.display(),
                "NOTIFY_SOCKET is an abstract socket, which this cannot open; not notifying"
            );
            return;
        }

        // Bound to nothing: a datagram socket needs no address of its own to send, and binding one
        // would leave a file behind for systemd to have opinions about.
        match UnixDatagram::unbound().and_then(|sock| sock.send_to(message.as_bytes(), socket)) {
            Ok(_) => tracing::debug!(%message, "told the service manager"),
            Err(error) => tracing::debug!(
                %error,
                socket = %socket.display(),
                "could not tell the service manager; it will fall back to its own timeout"
            ),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::os::unix::net::UnixDatagram;
    use std::path::{Path, PathBuf};

    /// A scratch directory that cleans up after itself, the same shape as `Scratch` in `settings.rs`.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "km-notify-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("make the scratch directory");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A real datagram on a real socket.
    ///
    /// **A hand-written protocol with no test is a protocol nobody has checked**, and this one is
    /// load-bearing in a way its size hides: if the datagram never arrives, systemd waits out
    /// `TimeoutStartSec` with a boot splash on the television and then fails the unit.
    #[test]
    fn what_systemd_is_waiting_for_arrives() {
        let scratch = Scratch::new("arrives");
        let path = scratch.0.join("notify.sock");

        let listener = UnixDatagram::bind(&path).expect("bind the stand-in service manager");
        listener
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("set a timeout so a failure is a failure and not a hang");

        super::linux::send(&path, "READY=1");

        let mut buf = [0u8; 64];
        let read = listener.recv(&mut buf).expect("systemd would have had it");
        assert_eq!(&buf[..read], b"READY=1");
    }

    /// Nobody listening is not an error.
    ///
    /// The path exists in the environment and the socket does not — a manager that died, or a stale
    /// variable inherited from somewhere. `send` has to return rather than panic, because the caller
    /// is one line above the display loop and taking the machine down over an unheard hint would be
    /// a far worse failure than the one it is reporting.
    #[test]
    fn a_socket_nobody_is_listening_on_is_survivable() {
        let scratch = Scratch::new("deaf");
        super::linux::send(&scratch.0.join("nothing-here.sock"), "READY=1");
    }

    /// Which names are the abstract namespace and which are files.
    ///
    /// **This tests the decision and not the behaviour, deliberately.** Two earlier versions of this
    /// test were worthless and each looked fine: one joined the `@` name onto a scratch directory,
    /// so the path began with `/tmp` and never took the branch at all; the other asserted that no
    /// file appeared afterwards, which is true whether the guard is there or not, because sending to
    /// a path that does not exist creates nothing either. `send` has no observable difference to
    /// catch, so the predicate is what has to be held.
    #[test]
    fn the_abstract_namespace_is_told_apart_from_a_file() {
        assert!(super::linux::is_abstract(Path::new("@km-abstract")));
        // The form systemd uses by default, and the one the appliance gets.
        assert!(!super::linux::is_abstract(Path::new("/run/systemd/notify")));
        // An `@` that is not the first character is an ordinary file name and a legal one.
        assert!(!super::linux::is_abstract(Path::new("/run/km@notify")));
    }
}
