# The offline remote, as an Android application

A WebView on the same server the desktop remote runs, with the server inside the phone. The pages are
`km-remote-pages`'s, unchanged — there is no native interface here, because the interface already exists.

```
app-debug.apk
 ├─ lib/arm64-v8a/libkm_remote_android.so     the server, loaded not executed
 ├─ lib/armeabi-v7a/libkm_remote_android.so
 └─ MainActivity                               a WebView on http://127.0.0.1:<port>
```

**There is no `externalNativeBuild` block anywhere in this project, on purpose** — the same rule
`ports/machine/android/README.md` states for the machine. cargo builds the native side and Gradle only
packages it.

## Building

```sh
tools/port/remote/android/build.sh          # cargo -> libkm_remote_android.so, both ABIs
tools/port/remote/android/stage.sh          # copy them into app/src/main/jniLibs/<abi>/
cd ports/remote/android && ./gradlew assembleDebug
```

`task build:android:remote` is those three steps under one name, and `task build:android:remote:native`
is the two before Gradle — the cargo half, which needs no JDK. `RELEASE=1` reaches all three;
`ARM64=1` is `--arm64-only`, and reaches `build.sh` **and** `stage.sh`. The Taskfile wraps these
commands and is not a substitute for them.

**Give `--arm64-only` to both, when running them by hand.** Skipping an ABI in `build.sh` does not
delete the library an earlier both-ABI build left in the target directory, so a staging step that
copied whatever it found would put a stale one in the APK — arm64 built this minute beside an armv7
from weeks ago, in a package whose whole point was to leave armv7 out. `stage.sh` removes any ABI
directory the run did not stage, which is what keeps `jniLibs` a picture of the build rather than of
every build. The machine's version of this note matters more, because a television runs the leftover.

Out comes `app/build/outputs/apk/debug/app-debug.apk` — **25 MB debug, 12 MB release**, and the two
libraries are essentially all of it either way (16.1 + 9.2 MB debug, 6.8 + 5.3 MB release; Gradle
strips them, so these are far below what cargo leaves in `target/`). There is no asset step and
nothing to fetch first: `km-remote-pages` compiles its templates, its stylesheet, its script and its icon
in and serves them from routes, so unlike the machine there is no `MANIFEST`, nothing is unpacked on
the device, and no SoundFont or wallpaper pack has to be downloaded before a build will work.

**`JAVA_HOME` must name a JDK 17 or 21**, exactly as for the machine — Gradle 8.12 fails on JDK 25
with `Unsupported class file major version 69`, and Android Studio's bundled JBR *is* 25.
`ANDROID_HOME` must point at the SDK, or `local.properties` must set `sdk.dir`.

`app/build.gradle` fails the build with a readable message if the libraries have not been staged. The
alternative is an APK that installs and then dies at launch with `UnsatisfiedLinkError`, which says
nothing about the cause.

Worth knowing when editing any XML here: **a `--` cannot appear inside an XML comment**, and the
resource compiler reports it as a bare parse error. It has now cost time in this project twice.

## What is different from the machine's app, and why

Neither project shares source with the other. What they share is conventions, and the differences are
all deliberate:

| | `ports/machine/android/` (the machine) | here |
|---|---|---|
| What loads | SDL's `SDLActivity`, which calls `SDL_main` | a plain `Activity`, which calls `Native.start` |
| Dependencies | SDL's Java, copied verbatim | **none at all** — an Activity and a WebView are framework classes |
| ABIs | both, **compulsory**: every Google TV device runs a 32-bit OS | both, but armv7 is for old phones; no television runs this |
| Orientation | landscape-locked, so a rotation cannot rebuild the GL surface mid-song | free; the page is a phone's single fluid column |
| Launcher | `LEANBACK_LAUNCHER` too, and a banner | the ordinary launcher only |
| Assets | unpacked on first run from a `MANIFEST` | none |
| `assembleRelease` | works; signed with the release key | the same, and the same key |

## The three things that are Android's rather than ours

**Cleartext to loopback.** Android has blocked plain HTTP since API 28, and that includes
`127.0.0.1`. `res/xml/network_security_config.xml` is the exception, and without it the WebView
renders nothing and says almost nothing about why. It permits loopback and nothing else — the Rust
side's own HTTP to the machine is a native socket and is not policed by that file at all, so there is
no reason to widen it.

**The multicast lock.** An mDNS *browse* sees nothing unless Java holds a
`WifiManager.MulticastLock` for its duration; sending multicast needs no such thing, which is why the
machine advertises happily without one. `MainActivity` takes it across `onStart`/`onStop` and the
Rust side is unchanged. Not holding it while backgrounded is correct rather than a compromise: a
browse then finds nothing and the server's own recovery logic answers "stay where you are".

**The camera, asked for twice over.** The share page's scanner needs `CAMERA`, and Android's own
prompt has to be answered before the *page's* request can be granted — which is why
`onPermissionRequest` holds the request and `onRequestPermissionsResult` answers it. A camera denied
twice is never asked for again, so the shell says where Settings is; the page always has its paste
box.

**A declared permission implies required features, and there are three of them here.**
`CHANGE_WIFI_MULTICAST_STATE` implies `android.hardware.wifi`, and `CAMERA` implies
`android.hardware.camera` **and** `android.hardware.camera.autofocus` — any of which would filter the
app off a device that lacks it, and this is a remote whose whole job works with no camera and no
Wi-Fi hardware named. The manifest declares all three only to un-require them. Check it on the built
APK, not in the source:

```sh
aapt2 dump badging app-debug.apk | grep feature
# want three lines, all `uses-feature-not-required`:
#   android.hardware.camera, android.hardware.camera.autofocus, android.hardware.wifi
```

## What this is not

**No foreground service, and nothing to add one for.** The Go remote this application follows keeps
one, because a dropped session there costs one of the karaoke unit's five client slots. Nothing here
is holding anything: the server is in this process, so backgrounding does not move its port, and the
client reconnects on its own. That absence takes the `connectedDevice` permission trap and the
`START_STICKY` crash loop with it — neither is fixed here, both are unreachable.

**No wake lock, and the screen sleeps on its own timeout.** There is no `WAKE_LOCK` permission, no
`PowerManager`, no `getWindow().addFlags(FLAG_KEEP_SCREEN_ON)` and no `setKeepScreenOn` anywhere in
this port; the two `getWindow()` calls in `MainActivity` are the inset controller and the status-bar
color. **Note the contrast with the machine's app**, which does hold the screen on and gets it from
SDL rather than from anything in its manifest — see *The screen staying on is SDL's* in
`ports/machine/android/README.md`. That is a box under a television with a song on the screen; this
is a phone somebody put down, and a remote that stopped a phone sleeping would be a battery fault
wearing the shape of a feature.

**What that costs is the paragraph above it**, and the two belong together: no service holding
anything and no wake lock means Android is free to freeze this process the moment the screen goes
off. Everything in `Coming back to a page is a reason to try the machine now` (in
`docs/decisions/remotes.md`) is the bill for these two lines, paid on the recovery side — where it
belongs, because the alternative is answering "the remote lost its connection while I put the phone
down" by never letting the phone rest. `keepScreenOn` is also a `View` attribute rather than an
`<activity>` one, so a manifest line would silently do nothing here exactly as it did there.

**No `useLegacyPackaging`, no `extractNativeLibs`.** Those exist so a Go *executable* shipped under a
`lib*.so` name can be extracted and `exec`ed. This is a real shared library; `System.loadLibrary`
maps it straight out of the APK, and setting either would keep two copies on the device.

**Download, file-chooser and camera plumbing, all three of which the page needs and cannot have.**
The share pages read a folder's code with `getUserMedia`, the backup route serves an attachment, and
restoring one uses an `<input type="file">` — so `MainActivity`'s chrome client answers a
`PermissionRequest` and an `onShowFileChooser`, and the WebView carries a `DownloadListener`.

**Two of those fail in the same shape, and it is the shape to remember.** An unanswered
`PermissionRequest` leaves `getUserMedia` pending for ever rather than failing, so the receive page
sits on "Starting the camera…" with nothing to read; an unanswered `ValueCallback` leaves the file
input *permanently* dead, which only shows up on the second attempt at restoring. Every branch of
both answers, including `results.length == 0` — how Android reports its own prompt being dismissed by
a tap outside rather than answered either way.

Three things here differ from the Go counterpart deliberately. The file-chooser intent is built by
hand as `*/*` rather than from `params.createIntent()`, so the page's `accept` list does not become a
MIME filter — Drive and Dropbox report their own types, and a filter makes a backup that is plainly
visible impossible to pick. `setType` for a save comes from the listener's own `mimetype` argument
rather than being hardcoded. And the copy loop is written out because `InputStream.transferTo` is
API 33 against `minSdk 26` with no desugaring here, which would be a `NoSuchMethodError` on exactly
the old hardware `armeabi-v7a` exists for.

**Do not turn the export into a `blob:` URL.** `staysInside` returns true for any non-http scheme, so
a `blob:` stays in the WebView, which cannot render one — and nothing at all happens.

## Signing, and the one thing it costs

The release build type is signed with the key `KM_ANDROID_KEYSTORE` names, and with the debug key
where it names none, so a fresh clone builds with nothing set up. Check what a build actually used
rather than assuming:

```sh
apksigner verify --print-certs app/build/outputs/apk/release/app-release.apk
```

`CN=Android Debug` there means the variable was unset. `tools/port/apk-signer.sh <apk>` says the same
thing in one line, and the build prints it after `assembleRelease`.

**The cost is worth knowing before it is paid.** Android refuses to install an APK signed with a
different key over an existing one, so a device holding a build from before the release key uninstalls
to take one — and an uninstall deletes app-private storage, which is where `favorites.sqlite` lives.
That file is the one thing here nothing can rebuild: the catalog mirror comes back from the machine
in seconds, a collection of favorites does not come back at all.

**The export turns that from a loss into a sequence to remember**: save a backup from the folder list,
uninstall, install the signed build, restore. Worth doing *before* the install that refuses rather
than after, which is the whole reason it is written here as well as in `DEPLOYING.md`. The APKs carry
a v3 signature, so a later rotation presents a lineage instead of asking for this again. See the
signing decision in `docs/decisions/`.

## What has actually been run, and what has not

On a **Galaxy S23, Android 16, arm64**, against a machine advertising on the LAN: discovery on the
first browse, the catalog mirrored, the event stream connected and the page's dot green; the page
rendering clear of the status bar with the tab bar reaching the bottom edge; backgrounding and
returning keeping the same process, the same port and a live page with **no reload** — which is the
thing a forked-child design cannot do; and back at the root shutting the server down cleanly.

Two bugs were found that way and neither was reachable from a desktop: the multicast lock was taken
in `onStart` while the server starts in `onCreate`, so the first browse ran without it; and the
"No karaoke machine found" screen did not notice the background browse succeeding twenty seconds
later, leaving a false sentence in front of a working remote.

**`armeabi-v7a` has now been run, and it was the last 32-bit gap in the project.** A **Google TV
Streamer** installed the release APK, reported `primaryCpuAbi=armeabi-v7a`, and did the whole job on
the first launch: the server came up, the machine was found, and the catalog mirrored —
`the remote is ready`, `connected to the karaoke machine`, `copied the song list from the machine
songs=88`. The page rendered its song list, favorites and queue controls, with the connected dot
green.

**The symbol check was necessary and was not sufficient**, which is the point worth keeping.
`llvm-readelf --dyn-syms` confirming all six `Java_..._Native_*` exports survive the version script
says they are *present*; only an Activity starting says the runtime **resolves** them. The failure
this was guarding against — a JNI export left on an old symbol after the namespace rename — compiles,
links, stages and packages, and shows up nowhere until `System.loadLibrary` runs. Both halves passed
here.

There is no `LEANBACK_LAUNCHER` on this APK by design, so a television has no tile for it; `adb
shell am start -n com.rrgmc.karaokemachine.remote/.MainActivity` is the way in. That is a test
convenience and not a gap — the offline remote belongs on a phone, and running it on the television
proves the ABI rather than proposing a use.

## Reading what it is doing

```sh
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb logcat -s km-remote
```

`km-remote-pages` is the tag, which is what makes it filterable beside the machine's `karaokemachine`. A
release build says `info` and a debug build adds this application's three crates; there is no
`RUST_LOG` to set for an Activity and no command line to put a `-v` on, so the build type is what
asks.

The two databases land in app-private storage and can be listed without root:

```sh
adb shell run-as com.rrgmc.karaokemachine.remote ls -l files/remote
```
