# Installing

**The downloads are on the [release page](https://github.com/rrgmc/karaokemachine/releases)**, one
file per platform. The carol package is a separate download beside them. To build from source, see
[`BUILDING.md`](https://github.com/rrgmc/karaokemachine/blob/master/BUILDING.md).

| Platform | What you get |
|---|---|
| **Windows** | A setup program, `karaokemachine-setup-<version>-windows-x86_64.exe`, with all nine products behind component checkboxes. It installs **per-user** into `%LOCALAPPDATA%\Programs` with no UAC prompt, and offers to add itself to `PATH` and to open `.kmbuild` files. Or a **portable folder**: unzip and run. |
| **macOS** | An installer package, `karaokemachine-setup-<version>-macos-<arch>.pkg`, with the same nine products. Applications go to `/Applications`, and command-line tools to `/usr/local/karaokemachine` with symlinks in `/usr/local/bin`. It asks for your administrator password once. Or `Karaoke Machine.app` on its own. |
| **Windows or macOS, the remote alone** | `km-remote-setup-<version>-windows-x86_64.exe` or `km-remote-setup-<version>-macos-<arch>.pkg`. It installs KM Remote only, for a computer that never plays a song. It can stay beside a full install. |
| **Debian, Ubuntu** | A `.deb` that names its ffmpeg and font dependencies. It installs a menu entry, an icon and the `karaokemachine` command, plus the appliance service, switched off. A second `.deb`, `karaokemachine-tools`, holds the two package builders, the offline remote and the picture-and-bank tool. Name both files in one `apt install` to get both. |
| **Any Linux** | A `.tar.gz`. Unpack it anywhere and run it, with no root and no package manager. It carries its own ffmpeg. A second one, `karaokemachine-portable-<version>-linux-x86_64.tar.gz`, is a **portable copy**: the machine and every tool, each keeping its settings and songs in the `data` folder inside. |
| **Android, Google TV** | An APK carrying both ABIs, for a phone and for a television. |
| **Meta Quest** | An APK of its own, which puts the machine on a screen hanging in the room with the room still behind it. The screen starts on your wall, and you move it and resize it by hand. The singer's queue hangs beside it. A button switches to an ordinary system window and back, once the queue is empty. |
| **iPhone, iPad** | An `.ipa` for the machine and one for the remote, both **unsigned**. You sign them yourself with your own Apple ID. |

**Every install also has a second launcher, which starts the machine streaming.** See
[Watching it in another room](streaming.md).

**Windows shows *"Windows protected your PC"* when you open a setup program**, because the Windows
downloads are not signed. Click **More info**, then **Run anyway**.
