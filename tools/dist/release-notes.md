A karaoke machine that plays MIDI files, video files, MP3+G pairs and UltraStar songs. It
highlights the words in time with the music, takes song requests from a phone, and has an HTTP API
for search, queueing and control.

## What changed

- **The machine is "Karaoke Machine" under its icon**, and its streaming launcher is "KM Stream", so
  no launcher cuts the name mid-word. Upgrading removes the old shortcuts and applications.
- **The package builder can number a package's only volume.** A package that will pass 999 songs
  takes the name `vol1` from its first build. Its file then keeps that name when a second volume
  starts.

## Which file to download

| File | For |
|---|---|
| `karaokemachine-setup-@VERSION@-windows-x86_64.exe` | Windows 10 or 11, 64-bit. One installer for every program, with a checkbox per part. It installs for your account only and does not ask for an administrator password. |
| `karaokemachine-setup-@VERSION@-macos-aarch64.pkg` | macOS on Apple Silicon. The same seven programs behind six checkboxes. Applications go to `/Applications` and the command-line tools to `/usr/local/bin`. It asks for your administrator password once and downloads nothing. |
| `km-remote-setup-@VERSION@-windows-x86_64.exe` | Windows 10 or 11, 64-bit — **the remote on its own**, for a computer that is not the karaoke machine. About 5 MB. The installer above already contains it; this one is for a computer that wants nothing else. |
| `km-remote-setup-@VERSION@-macos-aarch64.pkg` | macOS on Apple Silicon — **the remote on its own**. KM Remote goes to `/Applications` and nothing is put on your `PATH`. |
| `karaokemachine_@VERSION@-1_amd64.deb` | Debian 13 or later, amd64. Install it with `sudo apt install ./karaokemachine_@VERSION@-1_amd64.deb`. |
| `karaokemachine-tools_@VERSION@-1_amd64.deb` | The package builder, the offline remote and the picture-and-bank tool, for the same Debian. Download it beside the one above and install both at once: `sudo apt install ./karaokemachine_@VERSION@-1_amd64.deb ./karaokemachine-tools_@VERSION@-1_amd64.deb`. A box under a television needs only the machine. |
| `karaokemachine-@VERSION@-x86_64-unknown-linux-gnu.tar.gz` | Any other 64-bit Linux. Unpack it anywhere and run it. It includes the libraries it needs. |
| `karaokemachine-@VERSION@-android.apk` | The machine on Android and Google TV. One file covers 32-bit and 64-bit ARM. |
| `km-remote-@VERSION@-android.apk` | The remote, for a phone. It works when the machine is not reachable. One file covers both ARM architectures. |
| `karaokemachine-@VERSION@-ios-unsigned.ipa` | The machine on an iPhone or iPad. Sign it before installing it; see **Signing on iOS** below. |
| `km-remote-@VERSION@-ios-unsigned.ipa` | The remote on an iPhone or iPad. Sign it the same way. |

`@CAROLS@` is a song package and a separate download. It holds sixteen public-domain Christmas
carols from the Open Hymnal Project. Put it in the folder that `karaokemachine --show-paths` calls
`packages` and press `Ctrl+F10`, or send it to the machine from the curation tool. A new install has
an empty catalog.

## Signatures

Each file answers for itself, and what a platform says about a download differs.

<!-- platform: macos -->
- **The macOS packages** are signed and notarized by Apple. Double-click one to open Installer.
  There is no warning to click past.
<!-- /platform -->
<!-- platform: windows -->
- **Windows** shows *"Windows protected your PC"*. Click **More info**, then **Run anyway**.
<!-- /platform -->
<!-- platform: android -->
- **The Android APKs** are signed with the project's own key, so Android asks you to allow
  installation from this source and nothing more, and each version installs over the one before it.
  **A device holding an APK signed with a different key has to uninstall it first**, which Android
  reports as a refusal rather than a question. Export the remote's favorites from its share page
  beforehand and import them afterwards.
<!-- /platform -->
<!-- platform: linux -->
- **The two `.deb` files and the tarball** are unsigned, and apt reports this when you install a file
  by path.
<!-- /platform -->

<!-- platform: ios -->
## Signing on iOS

iOS does not install unsigned applications, and Apple does not issue a signature that can be
distributed with the file. Both `.ipa` files are marked `unsigned`, and you sign them yourself. It
takes about five minutes per app and needs a computer.

You need an Apple ID, and the one already on the phone is fine. You also need a free signing tool:
[Sideloadly](https://sideloadly.io) for macOS or Windows, or [AltStore](https://altstore.io), which
installs from the phone.

1. Download the `.ipa`.
2. Connect the iPhone or iPad to the computer and unlock it.
3. Open Sideloadly, drag the `.ipa` onto it, enter your Apple ID and click **Start**.
4. On the device, open **Settings → General → VPN & Device Management**, tap your Apple ID under
   *Developer App*, and tap **Trust**. The application will not open until you do.
5. Launch it once with a network connection. The certificate is verified on that first run.

Two limits come from the Apple ID:

- A free Apple ID signs an app for seven days. After that the app stops opening and you repeat
  step 3. Your songs, settings and catalog stay on the device. A paid Apple Developer account signs
  for a year, and AltStore renews the signature in the background.
- A free Apple ID allows three applications at a time. The machine and the remote are two of them.

Two things work differently on iOS:

- The machine does not announce itself on the network. The iOS remote finds it by scanning; the
  Android remote and a web browser need the address shown on the idle screen.
- The machine stops answering when it goes to the background and nothing is playing, because iOS
  suspends an application that is not playing audio. A song that is playing continues with the
  screen off.
<!-- /platform -->

Everything here is built from the sources in this repository, at tag `v@VERSION@`.
