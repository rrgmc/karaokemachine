# The offline remote, as an iOS application

The same program the desktop runs, on an iPhone or an iPad: this device's own copy of a machine's
catalog, favorites, and queueing when the machine is switched on. A `WKWebView` on
`http://127.0.0.1:<port>/`, where the server is a Rust static library linked into the app binary.

The Rust is `crates/remote/km-remote-ios` (the six C functions) over `crates/remote/km-remote-host` (the state
machine, shared with the Android application) over `crates/remote/km-remote-core` (the server). Nothing in
this directory is Rust; it is an XcodeGen spec, four Swift files and an asset catalog.

## Building

```sh
tools/port/remote/ios/build.sh              # device + simulator, debug
tools/port/remote/ios/build.sh --release
tools/port/remote/ios/build.sh --device-only
# ...or `task build:ios:remote`, with RELEASE=1 and DEVICE=1.
```

Then open `KaraokeRemote.xcodeproj` and press Run. Re-run the script whenever the Rust changes; it
regenerates the project, so **never edit the project in Xcode expecting it to stick** — edit
`project.yml`.

Once per machine:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
brew install xcodegen
```

The script checks both, and checks for Xcode and the iOS platform, before it builds anything.

**`xcode-select` does not have to be pointing at Xcode.** It very often points at the Command Line
Tools, which have no `xcodebuild` and no iOS SDKs; the script exports `DEVELOPER_DIR` for its own
process rather than asking anybody to change a machine-wide setting with `sudo`. Set `XCODE` if it
is somewhere other than `/Applications/Xcode.app`.

## What is different from the Android application, and why

| | `ports/remote/android/` | here |
|---|---|---|
| The library | a `cdylib`, loaded with `System.loadLibrary` | a `staticlib`, linked into the app binary — iOS will not load an arbitrary dynamic library |
| Handed to the build | staged into `jniLibs/` by a second script | assembled into an `.xcframework`, which is what Xcode consumes; there is no staging step |
| The project | Gradle, committed | XcodeGen, generated from `project.yml`; the `.xcodeproj` and the `Info.plist` are git-ignored |
| Slices | `arm64-v8a` and `armeabi-v7a` | `aarch64-apple-ios` and `aarch64-apple-ios-sim`, both arm64. No Intel simulator |
| Discovery | mDNS, with Java holding a `MulticastLock` | a unicast sweep — see below |
| Cleartext to loopback | `network_security_config.xml` | `NSAllowsLocalNetworking` |
| Safe areas | measured in Java, injected as a CSS variable, re-applied on every navigation | nothing: `WKWebView` reports them and the stylesheet already reads them |
| Backup | off wholesale (`allowBackup="false"`) | the mirror excluded, the favorites kept — see below |

## The three things that are iOS's rather than ours

**Cleartext to loopback.** `NSAppTransportSecurity` → `NSAllowsLocalNetworking`, in `project.yml`.
Without it the web view renders nothing and says very little about why. Only the web view needs it:
the Rust side's HTTP to the machine and its `ws://` event stream are native sockets that App
Transport Security does not police at all, so there is no reason to widen this and a real loss in
doing so — the same note the Android project's XML carries about its own exception.

**Discovery cannot use multicast.** iOS has required
`com.apple.developer.networking.multicast` for multicast and broadcast since version 14, and Apple
grants it only after a manually reviewed request. Without it an mDNS browse does not fail — it finds
nothing and reports success, forever, which is indistinguishable from a house with the machine
switched off.

So this application passes `km_remote_core::find::Sweep` instead of `find::Mdns`: ordinary unicast
to each address on the local subnet, asking `GET /api/v1/discover` and taking the first that answers
as a karaoke machine. Entitlement-free. It needs `NSLocalNetworkUsageDescription`, which is
mandatory and whose absence is also silent — every packet is dropped and the app is not told.

Apple's own Bonjour APIs (`NWBrowser` with `NSBonjourServices`) are the other way through and need no
entitlement either. They were not used, and the reason is worth knowing before somebody switches:
they put discovery in Swift, which means a seventh function to hand the answer back down, and they
make the one genuinely tricky part of this application the one part `cargo test` cannot reach.
`find::Sweep` is `km-remote-core`'s code with `km-remote-core`'s tests.

**What the sweep costs, said plainly:** it assumes port 8177, where an SRV record would have carried
one. A machine moved off that port is invisible to it, and typing the address is the answer — which
is why the "no machine found" screen has a box.

### Saving a backup, and reading a code

**The camera needs four things and none is redundant with the others.** This is the one part of this
port where a missing line is fatal rather than merely broken.

- `NSCameraUsageDescription` in `project.yml`. Without it the app is **killed** rather than refused,
  so the first symptom on a device is it vanishing the instant the receive page starts its camera.
  **No entitlement, unlike multicast above** — a usage description is granted by the person at run
  time and reviewed by nobody.
- `config.allowsInlineMediaPlayback = true`, or WebKit takes the preview fullscreen over the page it
  belongs to.
- `config.mediaTypesRequiringUserActionForPlayback = []`, or `play()` is blocked: `scan.js` calls it
  after `getUserMedia` resolves, and by then the tap that started it no longer counts as a gesture.
  The symptom is a frozen first frame rather than an error.
- `requestMediaCapturePermissionFor` on the `WKUIDelegate`. WebKit's own prompt is **never
  remembered** in a `WKWebView`, so it is raised on every `getUserMedia` call and there is no way to
  answer once. Implementing this replaces it, leaving iOS's app-level prompt as the only one anybody
  sees — and that is what makes the receive page's auto-start viable rather than a prompt per visit.

**A saved backup goes to `temporaryDirectory`, never to `Documents`.** `Documents` is iCloud-backed
and exposed in the Files app, so a copy of the collection written there would defeat the point of
excluding the mirror — and would make `excludeMirrorFromBackup`'s literal array a three-file array. A
fresh directory per save, because WebKit refuses a destination that already exists and two backups
taken on one day carry the same dated name. The file is then handed to a share sheet, whose popover
**must** be anchored: unanchored, it raises rather than presents on an iPad, and an iPad is the
device this has been run on.

**Restoring needs no Swift at all** — `WKWebView` handles `<input type="file">` itself.

**`isHarmless` covers a third case and needs no code for it.** Turning a navigation into a download
cancels that navigation, so a saved backup arrives as `WebKitErrorDomain` 102 routinely, alongside
the rarer refused foreign link. Do not narrow that filter to foreign hosts: it would put a failure
screen up every time somebody saved their favorites.

## What this is not

- **No `fork` and no child process.** iOS forbids both, but so does good sense here: the server runs
  on a `tokio` runtime inside this process, and the port is read straight across the boundary. The
  owner's Go remote forks on Android and links a `c-archive` here because a cgo-free Go binary
  cannot be loaded as a library; that is a Go constraint and Rust has it on neither platform.
- **No background mode, no `beginBackgroundTask`, no suspend and no resume.** The Go remote needs
  all of those because its karaoke unit serves five clients and an abandoned session holds one of
  those slots. Nothing here is being held. A suspension freezes the server along with the app and
  thaws it with the port unchanged, and the page's `EventSource` reconnects on its own.
- **No `NSBonjourServices`**, per the section above.
- **No song-book download.** `Capabilities::offline()` has `song_book: false`, so the printed book —
  which the *machine* serves — is absent from these pages entirely and there is nothing here to
  fetch it with. That is a different download from the favorites backup, which does exist: see
  *Saving a backup, and reading a code* below.
- **No time-zone plumbing.** Go hardcodes `time.Local` to UTC on iOS and ships no platform zoneinfo,
  so that project passes a zone down through the environment and reads it with `C.getenv` because
  the Go runtime caches its own copy at process start. Rust has neither problem.

## Signing

Automatic, with the team named in `project.yml` — **on the target and not on the project**, because
XcodeGen writes an empty `DEVELOPMENT_TEAM` into every app target's own configurations and that
overrides a project-level one, leaving Xcode reporting that signing "requires a development team"
while the value sits unused one level up.

This is an Apple Development certificate for putting a build on the owner's own devices, which is a
different thing from the Developer ID `tools/dist/common.sh` reads out of `KM_SIGN_IDENTITY` for
macOS releases. Same team, two signing worlds; see the decision entry in `docs/decisions/`.

The two databases are in the app container, at `Library/Application Support/km-remote-pages/` —
which is what `Server.dataDirectory` appends, and what the decision entry agrees with. **The
catalog mirror is excluded from backup and the favorites are not**, which is the one thing iOS
can do here that Android cannot: the mirror is a copy of what the machine holds and comes back in
seconds, and a collection somebody built up over a year does not come back at all. The Android
application turns backup off wholesale, because Auto Backup has a 25 MB per-app quota a real
catalog is well past and copies WAL databases as they lie — **and has the pages' own export instead**,
which this platform now has as well.

## What has actually been run, and what has not

**Run on an iPad (6th generation, A1893)**, against a machine on the LAN. Discovery, the catalog
mirror and the event stream all work; the song list is served and the page loads.

Two numbers worth keeping. The sweep walked **1,021 addresses** on a /22 and found the machine in
**117 ms**, which is the same order as the Go remote's 21–86 ms on the same shape of network — that
one is a UDP ping and this is a TCP connect plus an HTTP round trip, so a small constant slower is
the expected shape. And the first sweep of a fresh install found nothing at all in 550 ms, which is
every packet being dropped while the local-network prompt is on screen; the second one, twenty
seconds later, found the machine. That self-correction is the design working rather than a
workaround, and it is why the recovery loop matters more here than on Android.

**The device found two bugs on the first run, and neither was reachable from a Mac.** Both are fixed;
see "Two things a device found" in `docs/ARCHITECTURE.md`. One was in code shared with the Android
application, where it was invisible.

**Still not run:** a queue. The machine used for this had four fixture songs and nothing was played
through it, so the controls, the now-playing card and the transport are unexercised on this platform.

**Sharing and backup shipped from a Windows box unbuilt.** There is no Xcode there and
`task build:ios:remote` carries `platforms: [darwin]` — a task whose platform does not match is
**skipped rather than failed**, so a green run of it on that machine was evidence of nothing.

**It shipped not compiling, and the build did not say so.** Foundation's `NSURLErrorCancelled` had
been anglicised to `NSURLErrorCanceled` by a rename sweep, and this target had not built since —
because `build.sh` generated the project without compiling it, so the one command anybody runs for
"the iOS build" exited 0 over a Swift file no compiler would accept. **`build.sh` compiles the app
now**, unsigned, as its last step; `--no-app` opts out.

**Run on the iPad on 2026-09-07**, a Release build installed with `devicectl` against a machine
holding 155 songs. Five of the seven open questions are answered:

- **The target compiles** — checked by every build now, rather than by remembering to open Xcode.
- **`NSCameraUsageDescription` is in the built bundle**, read out of the `Info.plist` of a built
  `KaraokeRemote.app`. Its absence *kills* the app, so the first symptom would have been the app
  vanishing rather than an error anybody can read.
- **`window.isSecureContext` is true at `http://127.0.0.1:<port>/`** — observed indirectly and
  conclusively: `scan.js` unhides the scan block only where `navigator.mediaDevices.getUserMedia`
  exists, and that is defined only on a potentially-trustworthy origin. The Scan button was there,
  so the origin qualifies.
- **Saving a backup does not put a failure screen up.** The 102 path is swallowed as intended: the
  server logged `wrote a favorites backup folders=1 songs=1`, the page stayed, and nothing was
  logged at warning or error for the life of the session.
- **The share sheet presents rather than raising.** This is the iPad-only failure — a
  `UIActivityViewController` with no popover anchor raises there and takes the app with it — and the
  sheet appeared on the iPad itself.

**Two remain, and both need the camera pointed at a code:**

- that the delegate replaces WebKit's prompt: the app-level camera prompt asked exactly **once**,
  and a second press of Scan asking nothing;
- that the preview stays inline and plays with no tap.

The Rust and the pages behind all of it are exercised by `cargo km-test` on every platform, and the
Android shell's half of the same work has been built and run — so what is untested here is the Swift
and the plist, not the feature.

**Two things about deploying that are Apple's rather than ours**, and both cost time on the first
evening:

- A freshly *installed* development build needs the network once to verify its certificate, and the
  profile trusted on the device — the app refuses to launch until then, with
  `FBSOpenApplicationServiceErrorDomain error 1` and a message about an invalid signature that says
  nothing about the real cause. Reinstalling **over the top** of an existing install avoids it;
  uninstalling first does not.
- `xcrun devicectl device install app` succeeds in that state, so a successful install is not
  evidence that a launch will work.

## Reading what it is doing

Xcode's console. The server writes `tracing` events to stderr, at `info` in a release build and at
this application's own three crates in a debug one. **There is no `os_log` bridge**, deliberately:
`km-androidlog` exists because Android has no stdout at all, and iOS does. What `os_log` would buy
is Console.app and `devicectl --console`, for a build running with Xcode *not* attached; that is the
day to write the crate.

Two things worth knowing when a symbol looks missing:

- **A debug build's `KaraokeRemote` is a ~58 KB launcher stub**, and the real binary beside it is
  `KaraokeRemote.debug.dylib`. Checking the wrong one for `_km_remote_start` finds nothing and
  suggests the archive never linked. `nm -a KaraokeRemote.app/KaraokeRemote.debug.dylib | grep
  km_remote_` is the check that means something.
- The databases are in app-private storage, so there is no `adb push` equivalent. The container is
  reachable from Xcode's Devices and Simulators window.
