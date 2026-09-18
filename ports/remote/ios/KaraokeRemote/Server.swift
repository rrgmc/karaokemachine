import Foundation
import UIKit

/// The six C functions, wearing Swift.
///
/// Nothing here decides anything: the state machine is `km-remote-host`, on the other side of the
/// boundary. This is the conversion layer, and it is deliberately the only place in the app that
/// mentions a `km_remote_` symbol.
enum Server {
    /// Where the two databases live.
    ///
    /// **Application Support rather than Documents**, and rather than Caches. `catalog.sqlite` is
    /// this device's copy of what the machine already holds, not something the user made, and
    /// `Documents` is backed up to iCloud and can be exposed in the Files app. `Caches` is worse
    /// still: the system may empty it between launches, and it would take the favorites with it.
    ///
    /// Swift creates the directory and hands down its path; nothing on the Rust side derives one.
    /// That is the first of `km-remote-core`'s four seams, and the reason it exists is that the
    /// crate a desktop uses for this ships modules for Linux, macOS, Windows and the web and would
    /// answer here with a path outside the container.
    static var dataDirectory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let dir = base.appendingPathComponent("km-remote-pages", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    /// Starts the server, or does nothing if one is already going.
    static func start() {
        let dir = dataDirectory.path
        let machine = Settings.machineAddress ?? ""
        excludeMirrorFromBackup()
        dir.withCString { d in
            machine.withCString { m in
                // **Both buffers die when these closures return**, so the library copies them on
                // entry — see the module header in `crates/remote/km-remote-ios/src/ffi.rs`. Reading one
                // later, from a task, is a use-after-free that presents as a garbled address rather
                // than as a crash, and the simulator hides it.
                km_remote_start(d, machine.isEmpty ? nil : m)
            }
        }
    }

    /// The port the remote is answering on, or nil until it is.
    static var port: Int? {
        let p = Int(km_remote_port())
        return p == 0 ? nil : p
    }

    /// Where the pages are.
    static var url: URL? {
        guard let port else { return nil }
        return URL(string: "http://127.0.0.1:\(port)/")
    }

    /// How many songs this device's copy holds, or nil if that is not known yet.
    ///
    /// Zero is a real answer and is not nil: it is a first run that found no machine and has
    /// nothing to show, which is the case `RootViewController` has to tell apart from a broken one.
    static var songs: Int? {
        let n = Int(km_remote_songs())
        return n < 0 ? nil : n
    }

    /// The machine this run is talking to, or nil — which is ordinary, not a failure.
    ///
    /// The string belongs to the library and must not be freed here. `String(cString:)` copies.
    static var machine: String? {
        guard let c = km_remote_machine() else { return nil }
        return String(cString: c)
    }

    /// Why the server stopped, or nil while it is healthy.
    static var failure: String? {
        guard let c = km_remote_failure() else { return nil }
        return String(cString: c)
    }

    /// Asks the server to stop. Returns at once.
    static func stop() {
        km_remote_stop()
    }

    /// Waits for the server to answer, the way a host does.
    ///
    /// **No deadline, and that is deliberate.** The only failure signal that means anything is
    /// `failure` being set. Both the Android shell and the owner's Go remote learned this the same
    /// way: a fixed deadline long enough for a first catalog import is indistinguishable from no
    /// deadline at all, and a shorter one reports a perfectly healthy server as dead.
    static func waitUntilReady(ready: @escaping (URL) -> Void, failed: @escaping (String) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async {
            while true {
                if let url {
                    DispatchQueue.main.async { ready(url) }
                    return
                }
                if let failure {
                    DispatchQueue.main.async { failed(failure) }
                    return
                }
                Thread.sleep(forTimeInterval: 0.1)
            }
        }
    }

    /// Keeps the catalog mirror out of iCloud and iTunes backups, and the favorites in.
    ///
    /// **The one thing iOS can do here that Android cannot.** The Android application turns backup
    /// off wholesale, because Auto Backup has a 25 MB per-app quota that a real `catalog.sqlite`
    /// is well past, and because it copies WAL databases as they lie. iOS lets the two files be
    /// treated differently, so the derived one is excluded and the collection somebody built up
    /// over a year is not — which is the mitigation the `How the Android remote is signed` decision
    /// says does not exist yet.
    ///
    /// Applied on every start rather than once: the files do not exist before the first run, and
    /// the flag is per-file rather than per-directory.
    private static func excludeMirrorFromBackup() {
        let dir = dataDirectory
        for name in ["catalog.sqlite", "catalog.sqlite-wal", "catalog.sqlite-shm"] {
            var file = dir.appendingPathComponent(name)
            guard FileManager.default.fileExists(atPath: file.path) else { continue }
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? file.setResourceValues(values)
        }
    }
}

/// The one thing this app remembers for itself.
///
/// Everything else — the catalog, the favorites, which machine was last found — is the server's,
/// in its own two databases. This is only the address somebody typed on the "no machine found"
/// screen, which has to survive the restart that applying it needs.
enum Settings {
    private static let machineKey = "machine_address"

    static var machineAddress: String? {
        get {
            let raw = UserDefaults.standard.string(forKey: machineKey) ?? ""
            let trimmed = raw.trimmingCharacters(in: .whitespaces)
            return trimmed.isEmpty ? nil : trimmed
        }
        set {
            let trimmed = newValue?.trimmingCharacters(in: .whitespaces) ?? ""
            if trimmed.isEmpty {
                // **Clearing it is the point.** Naming a machine pins it: the server will not
                // wander to another one afterwards, however long this one stays silent. So the
                // screen that sets this must be able to unset it, or a typo is permanent.
                UserDefaults.standard.removeObject(forKey: machineKey)
            } else {
                UserDefaults.standard.set(trimmed, forKey: machineKey)
            }
        }
    }
}
