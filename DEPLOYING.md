# Deploying

**The last mile: getting a build onto a device somebody sings in front of.**
[`BUILDING.md`](BUILDING.md) is what produced the artifact — this is what puts it on the phone, the
television or the box under it.

Seven targets, and they are not equally finished. One has a script that does the whole job. Four stop
at a file you install by hand, and two are a project you open and press Run in.

| Target | Build | Onto the device |
|---|---|---|
| [The machine → Android](#the-machine--android) | `task build:android` | `adb install -r` |
| [The machine → Meta Quest](#the-machine--meta-quest) | `task build:android:quest` | `adb install -r` |
| [The machine → iOS](#the-machine--ios) | `task build:ios` | Xcode, press Run |
| [The remote → Android](#the-remote--android) | `task build:android:remote` | `adb install -r` |
| [The remote → iOS](#the-remote--ios) | `task build:ios:remote` | Xcode, press Run |
| [The machine → Linux, over SSH](#the-machine--a-linux-box-over-ssh) | folded into the task | `task deploy:linux HOST=user@box` |
| [The remote → Linux, over SSH](#the-remote--a-linux-box-over-ssh) | by hand, in a container | by hand — **and usually unnecessary** |

`task` is optional here as everywhere: every line below works with it absent, and the script each
task wraps is named beside it.

---

## Finding an Android device first

**Nothing installs until `adb` can see the device**, and this is the step that most often goes wrong.
Three ways in, for three different devices.

**A phone or tablet, over Wi-Fi** — Android 11 and newer. Settings → Developer options → **Wireless
debugging** → *Pair device with pairing code*:

```sh
adb pair 192.168.1.x:37xxx        # the PAIRING port, and the code from the dialog
adb connect 192.168.1.x:42xxx     # the port on the Wireless debugging screen itself
```

**The two ports are different**, and using the pairing port for `connect` is the usual first mistake.
The dialog shows one, and the screen behind it shows the other. Pairing is once per machine, and
`connect` is once per boot.

**A television** — Google TV and Android TV have no pairing dialog. Enable Developer options by
opening Settings and tapping **Android TV OS build** seven times, turn on **USB debugging**, then:

```sh
adb connect 192.168.1.x:5555
```

The television shows an *Allow USB debugging?* prompt the first time; tick *Always allow*.

**Over USB** — plug it in and accept the RSA fingerprint prompt on the device.

### Finding the port without walking over to the device

**A phone's `connect` port changes at every boot.** The number written down last time is wrong, and
the screen holding the new one is across the room. `adb` browses for devices over mDNS and will tell
you instead:

```sh
adb mdns check                    # prints the discovery daemon's version, or says it is unavailable
adb mdns services
```

```
adb-SERIAL1-AB2CDE   _adb-tls-connect._tcp   192.168.1.x:37xxx
adb-SERIAL2          _adb._tcp               192.168.1.x:5555
```

**The service type says which port you are looking at.** `_adb-tls-connect._tcp` is a phone's
wireless-debugging port — the one that moves; `_adb._tcp` is the fixed 5555 a television listens on.
A device in the middle of pairing publishes a third, `_adb-tls-pairing._tcp`, which is the *other*
port of the two the section above warns about. A service name works wherever an address does:

```sh
adb connect adb-SERIAL1-AB2CDE._adb-tls-connect._tcp
```

**A listed service is not a reachable device.** The record outlives the phone that published it. A
phone that has since slept, left the Wi-Fi or closed its Wireless debugging screen is still listed,
at a port nothing answers on. `adb connect` then fails with a refused connection or a timeout,
neither of which mentions mDNS or says the device is simply away. `ping` the address to
tell a stale record from a live one before reading anything more into the failure.

Then, whichever route:

```sh
adb devices -l                    # `device` is ready; `unauthorized` means the prompt is unanswered
adb -s <serial> install -r ...    # -s picks one when more than one is attached
```

---

## The machine → Android

### Once per machine

```sh
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi
task assets                       # the GM SoundFont, which is not committed
task ffmpeg:android               # the LGPL ffmpeg, per ABI (not needed with NO_VIDEO=1)
```

Also needed, and each fails in a way that does not name itself:

- **The Android NDK**, from Android Studio's SDK Manager, or `ANDROID_NDK_HOME`.
- **`ANDROID_HOME`** pointing at the SDK, or `sdk.dir` in `ports/machine/android/local.properties`
  (not committed, being machine-specific).
- **`JAVA_HOME` naming a JDK 17 or 21.** Gradle 8.12 rejects JDK 25 with `Unsupported class file
  major version 69`, and 25 is exactly what Android Studio bundles. *The JDK already on the machine
  is the wrong one.* A Gradle toolchain does not help: what fails is the JVM compiling the build
  script, not the one compiling the app.
- **`ninja` on `PATH`**, for SDL.

### Build and install

```sh
task build:android
adb install -r ports/machine/android/app/build/outputs/apk/flat/debug/app-flat-debug.apk
adb logcat -s karaokemachine
```

About 78 MB, most of it the SoundFont and the wallpaper pack. `task build:android:native` is the
cargo half alone, which needs no JDK.

### Songs

A plain push into the app's external files folder — no permission on any API level, and no `run-as`:

```sh
adb push mysongs.kmpkg \
  /storage/emulated/0/Android/data/com.rrgmc.karaokemachine/files/packages/
```

**`adb push` is the only route that writes into that folder directly.** Android 11's scoped-storage
enforcement closed `/Android/data/` to third-party file managers and hides it over MTP, so a USB copy
will not do it. The app's *private* directory is scanned first and is reachable only through
`run-as`; `karaokemachine --show-paths` names both.

**Without a cable, open the package instead.** A `.kmpkg` anywhere the device can see it offers
KaraokeMachine in *Open with*: Downloads, the Files app, a browser's download notification. The
machine copies it into the packages folder and installs it. That works with the machine running and
with it closed, and is what a device that never meets a computer uses.

### Three things that bite

- **`RELEASE=1` assembles a release APK**, at
  `ports/machine/android/app/build/outputs/apk/flat/release/app-flat-release.apk`. It takes the key
  `KM_ANDROID_KEYSTORE` names, and the debug key where it names none; the build prints which. It is
  the one to hand somebody. A debug APK is debuggable, and anything holding ADB access can attach to
  a debuggable application and run code as it. Without `RELEASE=1` the debug APK is what comes out,
  at the `flat/debug/app-flat-debug.apk` path above.
- **Never ship `ARM64=1`.** Every Google TV device runs a 32-bit OS and loads `armeabi-v7a` alone, so
  an ARM64-only APK installs on a phone and fails on a television. It is a quick-iteration flag.
- **`NO_VIDEO=1` drops the ffmpeg prerequisite** and produces a machine that cannot play video songs.
  The build refuses to start without ffmpeg rather than do that silently.

---

## The machine → Meta Quest

A headset is an Android device, so everything above holds. What differs is the APK. It puts the
machine on a screen hanging in the room rather than in a system panel.

### Once per machine

The same prerequisites as Android, plus developer mode on the headset. The Meta Horizon phone
application turns that on, rather than anything in the headset.

### Build and install

```sh
task build:android:quest
adb install -r ports/machine/android/app/build/outputs/apk/headset/debug/app-headset-debug.apk
```

It installs **beside** the ordinary Android APK rather than over it, because the application id ends
in `.quest`. A headset can hold both, and each keeps its own songs.

### Songs

The same two routes as Android, and the folder is the one belonging to `com.rrgmc.karaokemachine.quest`.
Opening a `.kmpkg` from the headset's Files application is the route that needs no cable.

### Three things that bite

- **Wireless debugging needs a cable once per boot.** `adb tcpip 5555` over USB, then
  `adb connect <address>:5555`. Horizon OS shows no pairing-code screen, so the `adb pair` flow above
  does not reach it.
- **The headset sleeps the moment it leaves your face**, which stops the song and ends a measurement.
  `adb shell am broadcast -a com.oculus.vrpowermanager.prox_close` stops that, and
  `com.oculus.vrpowermanager.automation_disable` puts it back. Neither survives a reboot, and a
  headset left awake with passthrough running warms up and turns its fan on.
- **The library shows the icon and no name.** A name comes from Meta's store, and a sideloaded
  application has no entry there. See `What the machine *is*, on a headset` in
  [`docs/decisions/distribution.md`](docs/decisions/distribution.md).
- **A launch that does nothing is usually the controllers.** Horizon OS refuses to start an immersive
  application with no controller awake, and says so in a dialog the application never sees. The log
  line is `app_launch_blocked_controller_required`. Wake a controller, or use hands, which this
  application declares.

## The remote → Android

A much smaller build than the machine's: no ffmpeg, no ninja, no libclang, no CMake and no assets
step. The prerequisites are the machine's first four and nothing else — NDK, `cargo-ndk`, the two
rustup targets, `ANDROID_HOME`, and a JDK 17 or 21.

```sh
task build:android:remote
adb install -r ports/remote/android/app/build/outputs/apk/debug/app-debug.apk
adb logcat -s km-remote
```

25 MB debug, 12 MB release.

- **`RELEASE=1` produces an installable APK** —
  `ports/remote/android/app/build/outputs/apk/release/app-release.apk`, signed with the key
  `KM_ANDROID_KEYSTORE` names, the same arrangement the machine's has.
- **There is no leanback launcher on this APK, by design**, so a television shows no tile for it.
  `adb shell am start -n com.rrgmc.karaokemachine.remote/.MainActivity` is the way in. The
  offline remote belongs on a phone; running it on a television proves the ABI rather than proposing
  a use.
- The two databases are readable without root:
  `adb shell run-as com.rrgmc.karaokemachine.remote ls -l files/remote`.

### Installing over a debug-signed build: save a backup first

Android refuses to install an APK signed with a different key over an existing one. A device holding
a build from before the release key therefore uninstalls to take one. An uninstall deletes
`favorites.sqlite`, the one file in this product that nothing can rebuild. `adb install -r` reports
this as `INSTALL_FAILED_UPDATE_INCOMPATIBLE`.

**Save a backup first**: the folder list's *Backup* → *Save a backup*, which writes
`km-favorites-<date>.json` wherever the document picker is pointed. Restore it after installing the
new build. The sequence is cheap and the omission is not recoverable.

```sh
# with the backup saved
adb uninstall com.rrgmc.karaokemachine.remote
adb install ports/remote/android/app/build/outputs/apk/release/app-release.apk
```

The machine's is the same sequence against `com.rrgmc.karaokemachine`, and it costs a copy back and a
rescan rather than a loss. Its uninstall takes the private packages folder and `library.sqlite`, and
a package is a file somebody still has.

**A signed APK carries a v3 signature, so this is paid once.** Android 9 and above accept a later key
that presents a signing lineage. That is what keeps a future rotation from asking for the uninstall
again.

### The camera, the first time

The share page's scanner asks twice on the first run. Android's own `CAMERA` prompt comes first, and
then the page's request, which the shell grants once somebody answers the first. Both happen with
nothing to do but tap *Allow*.

**Denied twice, Android stops asking**, and there is then no way back from inside the app. The toast
names the way: Settings → Apps → KaraokeMachine Remote → Permissions → Camera. The launcher's icon
says `KM Remote`, while Settings reads the application label, which is the whole name. The page's
*Can't scan it?* box takes a pasted code and needs no camera, so nothing is unreachable either way.

---

## The machine → iOS

**macOS only**, and it needs the full Xcode rather than the Command Line Tools. The machine itself
on an iPhone and an iPad: the same synthesizer, catalog, display and API as every other build.
[`ports/machine/ios/README.md`](ports/machine/ios/README.md) is the port's own page and has the
table of what differs from the Android application.

### Once per machine

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
brew install xcodegen
xcodebuild -downloadPlatform iOS        # if the iOS platform is not installed
tools/setup/fetch-assets.sh             # the SoundFont
```

`xcode-select` does **not** have to be pointing at Xcode — the build exports `DEVELOPER_DIR` for its
own process rather than asking for a machine-wide `sudo`. Set `XCODE=` if Xcode is somewhere other
than `/Applications/Xcode.app`. All of these are checked before anything is built.

The build fetches the lyric font and ffmpeg itself, into the shared asset cache. The first run
therefore costs an ffmpeg cross-compile of a few minutes, once.

### Build and run

```sh
task build:ios                                      # RELEASE=1, DEVICE=1, NOAPP=1, NO_VIDEO=1
open ports/machine/ios/KaraokeMachine.xcodeproj     # pick the device, press Run
```

**To hand it to somebody else instead**, `IPA=1` packages the build as
`dist/karaokemachine/ios/karaokemachine-<version>-ios-unsigned.ipa`, which they sign with their own
Apple ID. It needs `RELEASE=1` and it keeps the video decoder.
[`README.md`](README.md#installing) has the procedure they follow.

`DEVICE=1` skips the simulator slice, which is a whole second SDL and a whole second ffmpeg.
`NO_VIDEO=1` leaves out the decoder and the four frameworks with it.

### What to know before it goes wrong

- **Never edit the project in Xcode.** It is generated from `ports/machine/ios/project.yml` by every
  build, and the next one discards the change. Edit `project.yml`.
- **Signing is Automatic**, from the development team named in `project.yml`. That is an Apple
  Development certificate for putting a build on your own devices. It is a different thing from the
  Developer ID used for macOS releases.
- **A freshly installed development build needs the network once.** It verifies its certificate and
  lets the profile be trusted on the device, exactly as the remote's does.
- **A package arrives through Files**, not `adb push`. Copy a `.kmpkg` into the app's `Documents`
  folder over USB or from the Files app, and the machine catalogues it at the next start. *Open in*
  reaches the machine too. `km-package-builder`'s *Send* posts to the running machine over the network and
  needs none of that.
- **The machine does not announce itself over mDNS here.** Multicast needs an entitlement Apple
  grants only after a manual review, and without it the packets are dropped in silence. An iPhone
  running the offline remote finds it anyway, by sweeping the subnet; an Android remote has to be
  given the address.
- **A backgrounded machine stops answering.** iOS suspends an application that is not playing
  audio, so a remote loses a machine nobody is singing on. A *playing* song survives the screen
  going off.

---

## The remote → iOS

**macOS only**, and it needs the full Xcode rather than the Command Line Tools.

### Once per machine

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
brew install xcodegen
xcodebuild -downloadPlatform iOS        # if the iOS platform is not installed
```

`xcode-select` does **not** have to be pointing at Xcode — the build exports `DEVELOPER_DIR` for its
own process rather than asking for a machine-wide `sudo`. Set `XCODE=` if Xcode is somewhere other
than `/Applications/Xcode.app`. All four of these are checked before anything is built.

### Build and run

```sh
task build:ios:remote                              # RELEASE=1, DEVICE=1
open ports/remote/ios/KaraokeRemote.xcodeproj      # pick the device, press Run
```

`DEVICE=1` skips the simulator slice — a minute quicker, and the result cannot then run in a
simulator.

**To hand it to somebody else instead**, `IPA=1` packages the build as
`dist/km-remote/ios/km-remote-<version>-ios-unsigned.ipa`, which they sign with their own Apple ID.
It needs `RELEASE=1`. [`README.md`](README.md#installing) has the procedure they follow.

### What to know before it goes wrong

- **Never edit the project in Xcode.** It is generated from `ports/remote/ios/project.yml` by every
  build, and the next one discards the change. Edit `project.yml`.
- **Signing is Automatic**, from the development team named in `project.yml`. That is an Apple
  Development certificate for putting a build on your own devices. It is a different thing from the
  Developer ID used for macOS releases.
- **A freshly installed development build needs the network once**, to verify its certificate and let
  the profile be trusted on the device. Without it the app fails to launch with
  `FBSOpenApplicationServiceErrorDomain error 1`. **Reinstalling over the top clears it; uninstalling
  first does not.**
- **Installing is not launching.** `xcrun devicectl device install app` succeeds even in the state
  above, so a successful install says nothing about whether the app starts.
- **The camera prompt is iOS's, and it is asked once.** The app's `WKUIDelegate` answers WebKit's own
  prompt, which a web view never remembers and would otherwise raise on every scan. The system prompt
  is then the only one anybody sees, remembered and revocable in Settings → KaraokeMachine Remote.

  **If the app vanishes the instant the receive page opens rather than prompting**, the built bundle
  has no `NSCameraUsageDescription`. iOS kills an app that asks for the camera without one, and that
  is the whole of the symptom.
- **A saved backup goes through the share sheet**, into Files, Dropbox, Drive or whatever else is
  installed. Nothing writes it into the app's own Documents, which would put a copy of the collection
  into iCloud. Restoring reads a file back through the same picker.

---

## The machine → a Linux box over SSH

The one target with a real deployment script. It builds the package, copies it, installs it, enables
the service and restarts it. That turns an ordinary Debian box into an appliance which comes back by
itself after a power cut.

**`HOST` is the box and has no default**, here or in the Taskfile. The value is an address in
somebody's own house, and no tracked file may carry one. `task dist:deb` builds the package without
sending it.

```sh
task deploy:linux HOST=user@box
task deploy:linux HOST=user@box NO_BUILD=1              # send what is already staged
task deploy:linux HOST=user@box NO_VIDEO=1              # the plain build
task deploy:linux HOST=user@box SONGS=./packages        # also send .kmpkg files, and restart
task deploy:linux HOST=user@box PORT=2222 IDENTITY=~/.ssh/karaoke
```

The script underneath takes the host as its first argument, and each variable as the flag it is named
after. Every line above is therefore a line you can type with `task` absent:

```sh
tools/platform/linux/deploy.sh user@box --no-build --songs ./packages
```

`--help` prints the same list, and `task --list` prints the variables each task takes.

### Before the first one

```sh
ssh-copy-id user@box
```

Without a key it asks for a password **twice per deploy** — once for the copy, once for everything
else. The script notices and prints this line itself.

### Requirements

**Here:** Docker (for the build, so not needed with `--no-build`), `ssh` and `scp`. `rsync` is used
for `--songs` when present, and falls back to `scp`.

**There:** Debian 13 or newer, systemd, a graphics device with a kernel mode-setting driver, and
network access to the Debian archive. `apt-get` installs the package, so it resolves the dependencies
on the box.

**amd64 only.** The package is `karaokemachine_<version>-1_amd64.deb`; a Raspberry Pi or other arm64
box is not a target this tooling reaches.

### What it reports, and the four things that bite

The script prints the installed binary's checksum and the version. It also prints whether the service
is active and enabled, where its files are, and the last 30 journal lines. Then:

- **`user@box` is the *sudo* account, never `karaoke`.** The package creates the service account. It
  has no password, and nothing ever logs in as it.
- **An enabled display manager is the usual cause of a black screen.** gdm, lightdm and sddm hold DRM
  master for themselves, and the machine cannot then take the screen. The script warns when it finds
  one; `sudo systemctl disable --now <dm>` is the cure.
- **`journalctl -u karaokemachine` shows almost nothing**, which reads exactly like an application
  that started and then went quiet. The service runs inside a login session, so its output is filed
  under the session scope. Use **`journalctl -t karaokemachine`**.
- **The last test is the television.** Nothing the script prints substitutes for looking at it: the
  service reports `active (running)` whether or not a picture ever appeared.

**Turning it off.** The deploy installs a logind drop-in. **A single press of the box's power button
then shuts the machine down cleanly**, with settings saved and the audio device released. Pressing it
again brings the box back. It takes effect at the next boot, and the script prints what logind
believes in the meantime.

`/admin/`'s *This machine* tab carries the same two acts behind the admin password. **Shut down**,
and **Restart**, which ends the application and lets systemd start it again. Restart is the thing to
reach for after changing a setting the machine reads only at startup. `Ctrl+Q` on a keyboard plugged
into the box does what the power button does.

```sh
ssh user@box 'sudo journalctl -t karaokemachine -f'
ssh user@box 'sudo systemctl restart karaokemachine'
# If a restart loop ever trips systemd's start limit, the unit stays failed until this.
ssh user@box 'sudo systemctl reset-failed karaokemachine'
```

The phones in the room want `http://<box>:8177/`.

### A box with no television on it

**A streaming run needs no screen, no display server, no X and no Wayland**, so a box with none of
them still has a machine. `--stream` draws the screen for an encoder and serves it. The television is
whatever plays `http://<box>:8177/stream/live.m3u8` in the room the singing happens in. The deploy
above is unchanged: the same package, the same command, the same box.

**Start it as the account the package made**, or what comes up is a second machine rather than the
same one. The songs, the settings and the catalog live under the `karaoke` account's home, and
`user@box` is the sudo account. A run started from your own login resolves its own empty folder, and
then finds port 8177 held by the service:

```sh
ssh user@box 'sudo systemctl stop karaokemachine'
ssh user@box 'sudo -u karaoke -H karaokemachine --stream'
```

**`-H` is load-bearing**, and its absence is the failure that looks like success. Sudo leaves `HOME`
pointing at the calling account, so the machine answers a different question and comes up with
nothing in it. `sudo -u karaoke -H karaokemachine --show-paths` says which folders a run will use
before one starts. On a box whose sudoers lists the specific commands a deploy runs, `sudo -u
karaoke` is not among them. `deploy.sh` asks `getent` for the home instead, for that reason.

**The service the package ships is the television one**, and it stays that. Its `TTYPath=/dev/tty1`,
its `Conflicts=getty@tty1.service`, its `SDL_VIDEODRIVER=kmsdrm` and the `ExecStartPre` that waits
for DRM are all there to take a screen this run does not want. Enabling it on a box with no screen
gets a service that waits for a television to appear. A machine that comes back by itself after a
power cut *and* streams is a unit of its own. The decision that has to come first is whether an
appliance can be a streaming appliance.

**A `NO_VIDEO=1` deploy cannot stream.** That build links no encoder, so the machine stops at startup
saying so and naming the feature that supplies one. The plain deploy is the one to send to a box that
streams.

**A run started over SSH ends with the connection**, so `systemd-run --user --scope` or a terminal
multiplexer is what keeps it up for an evening.

### Making the boot look like an appliance too — once per box

A deploy gets the machine onto the box. The seconds *before* it are still the distribution's: a
five-second bootloader menu, then kernel text, then black. One more command covers them.

```sh
task deploy:linux:boot HOST=user@box             # hide the menu, use our splash
task deploy:linux:boot HOST=user@box REVERT=1    # put the box back
task deploy:linux:boot HOST=user@box FORCE=1     # anyway, on a dual-boot box
task deploy:linux:boot HOST=user@box SLIM=1      # and make the boot faster
tools/platform/linux/appliance-boot.sh user@box [--revert] [--force] [--slim-initramfs]
```

**What it assumes about the box.** Debian, and little else. It probes for the bootloader, whatever
regenerates its config, the Plymouth theme tool and the initramfs lister, rather than assuming their
names. It skips the bootloader half cleanly on a box that does not boot through GRUB.
The one distribution-specific step is installing Plymouth, which is `apt-get`. Anywhere else it says
so and stops, rather than half-configuring somebody's boot.

**Run it after the first deploy, and then not again.** The Plymouth theme ships inside the package,
so the package has to arrive before there is a theme to select, and the script refuses otherwise.
Nothing about it belongs on the every-commit path: it rewrites `/etc/default/grub` and rebuilds the
initramfs.

Four things worth knowing before running it:

- **It asks for a password, and a deploy does not.** `update-grub`, `update-initramfs` and
  `plymouth-set-default-theme` are not on the NOPASSWD list of a properly locked-down box, and they
  should not be. There is an operator in front of this one.
- **Nothing is drawn for the first second, on purpose, and a key held down as the box starts brings
  the menu up.** On a UEFI box that window is the *only* way into the menu. GRUB's held-SHIFT test is
  a BIOS facility and does not exist there. Try it once, before you need it. It is one second rather
  than three. The panel already spends about two seconds re-syncing HDMI when i915 takes the
  connector over, and the window stacks on top of that.
- **It refuses on a dual-boot box** unless given `FORCE=1`. Hiding the menu takes the other operating
  system away from anybody who does not know about the window.
- **`REVERT=1` restores what was there**, from copies taken exactly once, so a second run cannot
  overwrite the record of how the box started out.
- **`SLIM=1` is the one thing here you cannot undo over ssh.** It builds the initramfs for the
  hardware present rather than for all of it. The bootloader reading a large one dominates the boot,
  so this is the largest saving available — the appliance's was 72 MB, at 12 MB/s. The cost is
  that such an initramfs will not boot hardware that changes. Swap a storage controller or move the
  disk, and you need a rescue USB. `REVERT=1` puts `MODULES` back and rebuilds, from a box that still
  boots.

```sh
ssh user@box 'plymouth-set-default-theme'                          # karaokemachine
ssh user@box 'cat /proc/cmdline'                                   # says splash, after a reboot
ssh user@box 'systemd-analyze critical-chain karaokemachine.service'
```

That last one is the check worth making if the television stays black afterwards. Plymouth holds DRM
master while the splash is up, so `plymouth-quit-wait.service` has to come before the machine.

---

## The remote → a Linux box over SSH

**There is no script for this, and usually there is no need for one.** The machine already serves the
remote page at `http://<box>:8177/`, so a phone on the LAN needs nothing installed. `km-remote` is
its own daemon only where a box should hold its own copy of the catalog, and work while the machine
is switched off.

### Building a Linux binary

`task dist:tools -- km-remote` builds for the *host* platform, so it is the wrong tool from a Windows
or macOS box. Build it in the same Debian image the `.deb` is built in:

```sh
task shell:linux                    # an interactive shell in the trixie build image
```

Then, inside the container — `/src` is the checkout, and `CARGO_TARGET_DIR` is `/build/target`:

```sh
cargo build --release -p km-remote
cp /build/target/release/km-remote /src/dist/
exit
```

**No `--features`, deliberately.** The `desktop` feature links `wry`, which loads libwebkit2gtk. Such
a build would not *start* on a headless box: the failure is in the dynamic loader, before `main`, and
`--browser` cannot rescue it. A Linux `km-remote` serves its page and never opens a
window.

**The copy into `/src` is what gets the binary out.** `/build` is a named Docker volume and is
invisible from the host; `/src` is the checkout, mounted read-write.

### Sending and running it

```sh
scp dist/km-remote user@box:/tmp/
ssh user@box 'sudo install -m 755 /tmp/km-remote /usr/local/bin/'
ssh user@box 'km-remote --machine 192.168.1.x --lan --data-dir /var/lib/km-remote'
```

- **`--machine`** takes a host or a full URL, and keeps a port when given one — a bare host gets
  `:8177` appended. Omit it and the remote finds the machine by itself.
- **`--lan`** is required for anything but that box to reach it; without it the server is loopback
  only.
- **`--data-dir`** is where the catalog copy and the favorites live. The favorites are a
  collection somebody builds up over a year, so give it somewhere permanent.

### A unit to start from

**This repository does not ship one** — unlike `karaokemachine.service`, which the `.deb` installs.
The remote serves HTTP and never touches a screen, so it needs none of the machine's session and TTY
machinery:

```ini
[Unit]
Description=KaraokeMachine remote
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
DynamicUser=yes
StateDirectory=km-remote
ExecStart=/usr/local/bin/km-remote --lan --data-dir /var/lib/km-remote
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Add `--machine` if discovery does not find the machine on its own. Then the ordinary
`sudo systemctl enable --now km-remote`. `journalctl -u km-remote` works normally here: this unit has
no login session, so none of the machine's `-t` caveat applies.

---

## Where each thing lands

| Target | Command | Artifact | Installed as |
|---|---|---|---|
| Machine → Android | `task build:android` | `ports/machine/android/app/build/outputs/apk/flat/debug/app-flat-debug.apk` | `com.rrgmc.karaokemachine` |
| Machine → Meta Quest | `task build:android:quest` | `ports/machine/android/app/build/outputs/apk/headset/debug/app-headset-debug.apk` | `com.rrgmc.karaokemachine.quest` |
| Machine → iOS | `task build:ios` | `ports/machine/ios/KaraokeMachine.xcodeproj` | `com.rrgmc.karaokemachine` |
| Remote → Android | `task build:android:remote` | `ports/remote/android/app/build/outputs/apk/debug/app-debug.apk` | `com.rrgmc.karaokemachine.remote` |
| Remote → iOS | `task build:ios:remote` | `ports/remote/ios/KaraokeRemote.xcodeproj` | `com.rrgmc.karaokemachine.remote` |
| Machine → Linux | `task deploy:linux HOST=user@box` | `dist/karaokemachine/linux/*.deb` | `/opt/karaokemachine`, `karaokemachine.service` |
| …and its boot, once | `task deploy:linux:boot HOST=user@box` | nothing built | `/etc/default/grub`, the `karaokemachine` Plymouth theme |
| Remote → Linux | `task shell:linux`, then by hand | `dist/km-remote` | wherever you put it |
